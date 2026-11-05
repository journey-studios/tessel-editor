"""Infrastructure of the directed-scene proof: sessions, comparison, locks, reporting.

Nothing here knows a rule of the editor. It sends requests, compares answers with `expected.json`
(written by `corpus.py` from the independent model) and says what was and was not proven.
"""
import contextlib
import fcntl
import hashlib
import json
import os
import pathlib
import shutil
import socket
import subprocess
import sys
import tempfile
import time

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent.parent
CORPUS = HERE / "corpus"
GPU_LOCK_PATH = "/tmp/tessel-scene-gpu.lock"
MIN_FREE_GIB = 6
WORK_BUDGET_BYTES = 2 * 1024 ** 3

PASS, FAIL, BLOCKED = "PASS", "FAIL", "BLOCKED"


# ---------------------------------------------------------------------------------- outcomes
class Blocked(Exception):
    """A required operation or prerequisite does not exist. Never a pass."""

    def __init__(self, reason, needs, backlog):
        super().__init__(reason)
        self.reason, self.needs, self.backlog = reason, needs, backlog


class Abort(Exception):
    """A request that must succeed failed: the phase cannot continue."""


class Phase:
    def __init__(self, phase_id, surface):
        self.id, self.surface = phase_id, surface
        self.status = PASS
        self.checks = []
        self.blocked = None
        self.error = None
        self.metrics = {}
        self.seconds = 0.0

    def check(self, name, condition, detail=""):
        ok = bool(condition)
        self.checks.append({"check": name, "ok": ok, "detail": "" if ok else str(detail)[:400]})
        print(("  PASS " if ok else "  FAIL ") + name + ("" if ok else f"  -> {str(detail)[:300]}"), flush=True)
        if not ok:
            self.status = FAIL
        return ok

    def to_json(self):
        out = {"phase": self.id, "surface": self.surface, "status": self.status, "seconds": round(self.seconds, 2),
               "checks_total": len(self.checks), "checks_failed": sum(1 for c in self.checks if not c["ok"]),
               "checks": self.checks}
        if self.blocked:
            out["blocked"] = self.blocked
        if self.error:
            out["error"] = self.error
        if self.metrics:
            out["metrics"] = self.metrics
        return out


# ---------------------------------------------------------------------------------- numbers
def close(actual, expected, absolute, relative):
    return abs(actual - expected) <= absolute + relative * abs(expected)


def vec_close(got, want, tol):
    return all(close(got[k], want[k], tol["absolute"], tol["relative"]) for k in "xyz")


def matrix_close(got, want, tol):
    return len(got) == 16 and all(close(g, w, tol["absolute"], tol["relative"]) for g, w in zip(got, want))


def pixel_close(got, want, tol):
    return all(abs(int(g) - int(w)) <= tol["maxChannelDelta"] for g, w in zip(got[:3], want[:3]))


def compare_probes(expected, got, tol):
    """Returns the list of mismatches between expected probes (corpus) and read pixels.

    `got[i]` answers `expected[i]` (request order). A missing pixel is a mismatch: "not measured"
    is never "matches"."""
    mismatches = []
    if len(got) != len(expected):
        return [f"expected {len(expected)} probe pixels, got {len(got)}"]
    for probe, pixel in zip(expected, got):
        if pixel is None or len(pixel) < 3:
            mismatches.append(f"({probe['x']},{probe['y']}) {probe['layer']}: no pixel read")
        elif not pixel_close(pixel, probe["color"], tol):
            mismatches.append(f"({probe['x']},{probe['y']}) {probe['layer']}: wanted {probe['color']}, got {list(pixel)[:3]}")
    return mismatches


# ---------------------------------------------------------------------------------- files and environment
def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def newest_source_mtime():
    newest = 0.0
    for crate in (ROOT / "crates").iterdir():
        for base, _, names in os.walk(crate / "src"):
            for name in names:
                if name.endswith(".rs"):
                    newest = max(newest, os.path.getmtime(os.path.join(base, name)))
    return newest


def binary_info(path):
    """Identity of a binary under test, so a result can never be detached from what produced it.
    `older_than_sources` warns that the binary may predate the current checkout; it does not fail."""
    path = pathlib.Path(path)
    if not path.is_file():
        return {"path": str(path), "present": False}
    stat = path.stat()
    return {"path": str(path), "present": True, "sha256": sha256_file(path), "bytes": stat.st_size,
            "mtime_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(stat.st_mtime)),
            "older_than_sources": stat.st_mtime < newest_source_mtime()}


def git_state():
    def run(*args):
        return subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True).stdout.strip()
    return {"head": run("rev-parse", "HEAD"), "branch": run("rev-parse", "--abbrev-ref", "HEAD"),
            "dirty_files": len([line for line in run("status", "--porcelain").splitlines() if line])}


def free_gib(path="/tmp"):
    stat = os.statvfs(path)
    return stat.f_bavail * stat.f_frsize / 1024 ** 3


def tree_bytes(path):
    total = 0
    for base, _, names in os.walk(path):
        for name in names:
            with contextlib.suppress(OSError):
                total += os.path.getsize(os.path.join(base, name))
    return total


class Workspace:
    """Own temporary directory: budget 2 GiB, refuses to start under 6 GiB free, removes only itself."""

    def __init__(self, keep=False, base=None):
        if free_gib() < MIN_FREE_GIB:
            raise SystemExit(f"refusing to start: {free_gib():.1f} GiB free under /tmp, need {MIN_FREE_GIB}")
        self.keep = keep
        self.path = pathlib.Path(base) if base else pathlib.Path(tempfile.mkdtemp(prefix="tsp-", dir="/tmp"))
        self.path.mkdir(parents=True, exist_ok=True)

    def sub(self, name):
        path = self.path / name
        path.mkdir(parents=True, exist_ok=True)
        return path

    def within_budget(self):
        return tree_bytes(self.path) <= WORK_BUDGET_BYTES

    def close(self):
        if not self.keep:
            shutil.rmtree(self.path, ignore_errors=True)


_gpu_lock_handle = None


def acquire_gpu_lock():
    """Exclusive bench lock for Chrome/GPU/native-window work. Taken at the first GPU phase and held to
    the end of this process, so no other proof runs while any child of this harness uses the GPU."""
    global _gpu_lock_handle
    if _gpu_lock_handle is not None:
        return
    handle = open(GPU_LOCK_PATH, "a")
    print(f"waiting for {GPU_LOCK_PATH} ...", flush=True)
    fcntl.flock(handle, fcntl.LOCK_EX)
    print("holding the GPU bench lock", flush=True)
    _gpu_lock_handle = handle


# ---------------------------------------------------------------------------------- sessions
class JsonlSession:
    """One `tessel-headless` / `tessel-render` process; one JSON request per line, one answer per line."""

    def __init__(self, binary, args=(), env=None, cwd=None):
        self.process = subprocess.Popen([str(binary), *args], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=subprocess.PIPE, text=True, bufsize=1, env=env, cwd=cwd)

    def __call__(self, request):
        """Returns (ok, body): body is `result` on success and the `error` object otherwise."""
        self.process.stdin.write(json.dumps(request) + "\n")
        self.process.stdin.flush()
        line = self.process.stdout.readline()
        if not line:
            raise Abort(f"the process closed before answering {request.get('op')}: {self.process.stderr.read()[-300:]}")
        answer = json.loads(line)
        return answer["ok"], answer["result"] if answer["ok"] else answer["error"]

    def close(self):
        with contextlib.suppress(Exception):
            self.process.stdin.close()
        try:
            self.process.wait(timeout=60)
        except subprocess.TimeoutExpired:
            self.process.kill()


class EditorLab:
    """A private native editor: the DIRECT binary, short isolated HOME, its own socket. The user's
    window, project and socket are never addressed. Requests are semantic; no pointer, no coordinates."""

    def __init__(self, binary, home):
        self.home = str(home)
        self.socket_path = os.path.join(self.home, ".cache/tessel-editor/ui.sock")
        if len(self.socket_path) > 100:
            raise SystemExit("socket path too long for a unix socket; use a shorter --work")
        self.log_path = os.path.join(self.home, "editor.log")
        self.log = open(self.log_path, "w")
        env = {**os.environ, "HOME": self.home}
        self.process = subprocess.Popen([str(binary)], env=env, stdout=self.log, stderr=self.log, start_new_session=True)

    def send(self, request, timeout=60):
        with socket.socket(socket.AF_UNIX) as conn:
            conn.settimeout(timeout)
            conn.connect(self.socket_path)
            conn.sendall((json.dumps({"version": 1, **request}) + "\n").encode())
            data = b""
            while not data.endswith(b"\n"):
                chunk = conn.recv(65536)
                if not chunk:
                    break
                data += chunk
        return json.loads(data)

    def op(self, request, timeout=60):
        answer = self.send({"command": "op", "request": request}, timeout)
        return answer["ok"], answer.get("data") if answer["ok"] else answer.get("error")

    def wait_ready(self, seconds=90):
        deadline = time.time() + seconds
        while time.time() < deadline:
            if self.process.poll() is not None:
                raise RuntimeError(f"the editor exited ({self.process.returncode}): {open(self.log_path).read()[-500:]}")
            if os.path.exists(self.socket_path):
                with contextlib.suppress(OSError, ValueError, KeyError, TypeError):
                    if self.send({"command": "get", "target": "monitor.viewport"}, 10)["data"]["state"]["phase"] == "ready":
                        return
            time.sleep(0.25)
        raise RuntimeError("the editor's viewport never became ready")

    def stop(self):
        self.process.terminate()
        try:
            self.process.wait(10)
        except subprocess.TimeoutExpired:
            self.process.kill()
        self.log.close()


# ---------------------------------------------------------------------------------- interfaces
def load_interfaces(path=None):
    return json.loads(pathlib.Path(path or HERE / "interfaces.json").read_text())["logical_ops"]


def fill(template, values):
    """`"$name"` placeholders become the value; keys whose value is None are dropped."""
    out = {}
    for key, raw in template.items():
        value = values.get(raw[1:]) if isinstance(raw, str) and raw.startswith("$") else raw
        if value is not None:
            out[key] = value
    return out


def request_for(interface, **values):
    return {"op": interface["op"], **fill(interface["args"], values)}


def probe_interface(op, interface, **values):
    """Sends the real request for a logical operation. `unknown_operation` means the API does not
    exist: raise Blocked (never a pass, never a stub). Any other outcome is handed back to the phase."""
    ok, body = op(request_for(interface, **values))
    if not ok and body.get("code") == "unknown_operation":
        raise Blocked(f"`{interface['op']}` is not implemented ({body.get('message')})", interface["needs"], interface["backlog"])
    return ok, body


def dig(value, path):
    for key in path.split(".") if path else []:
        value = value[key]
    return value


def write_report(path, report):
    pathlib.Path(path).parent.mkdir(parents=True, exist_ok=True)
    pathlib.Path(path).write_text(json.dumps(report, indent=2) + "\n")


def log(*parts):
    print(*parts, file=sys.stderr, flush=True)
