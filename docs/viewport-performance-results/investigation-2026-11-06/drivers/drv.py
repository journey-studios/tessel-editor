#!/usr/bin/env python3
"""Driver of the performance investigation: isolated editor, semantic requests only.

Every run starts its own editor (isolated HOME, own socket) and never touches another instance.
"""
import json
import os
import shutil
import signal
import socket
import statistics
import subprocess
import sys
import threading
import time

ROOT = "/Users/wallyssonnunes/.t3/worktrees/tessel-editor/t3-129bf52a"
WORK = "/tmp/perf-inv"
RELEASE = f"{WORK}/tessel-editor-instr"
DEBUG = f"{WORK}/tessel-editor-instr-debug"
class _RT(dict):
    def __missing__(self, key):
        return f"{WORK}/rt-{key}"
RT = _RT({"drain": f"{WORK}/rt-instr-drain"})


def pct(values, share):
    if not values:
        return None
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(round(share * (len(ordered) - 1))))]


def summary(values, digits=2):
    values = [v for v in values if v is not None]
    if not values:
        return {"n": 0}
    return {
        "n": len(values),
        "mean": round(statistics.fmean(values), digits),
        "p50": round(pct(values, 0.5), digits),
        "p95": round(pct(values, 0.95), digits),
        "p99": round(pct(values, 0.99), digits),
        "max": round(max(values), digits),
    }


def webkit_pids():
    out = subprocess.run(["ps", "-axo", "pid=,rss=,comm="], capture_output=True, text=True).stdout
    found = {}
    for line in out.splitlines():
        parts = line.split(None, 2)
        if len(parts) == 3 and ("com.apple.WebKit" in parts[2]):
            found[int(parts[0])] = (int(parts[1]), parts[2].rsplit("/", 1)[-1])
    return found


def footprint_mb(pid):
    """Physical footprint (includes GPU-resident memory the process owns) from `footprint`."""
    try:
        out = subprocess.run(["footprint", "-p", str(pid), "--noCategories"], capture_output=True, text=True, timeout=20).stdout
    except Exception:
        return None
    for line in out.splitlines():
        if "Footprint:" in line and "phys_footprint" not in line:
            tok = line.split("Footprint:")[1].strip().split()
            try:
                value = float(tok[0])
            except ValueError:
                return None
            unit = tok[1] if len(tok) > 1 else "MB"
            return value * {"KB": 1 / 1024, "MB": 1, "GB": 1024}.get(unit, 1)
    return None


class Editor:
    def __init__(self, label, binary=RELEASE, runtime="instr", env=None, project=None, home=None, log=True):
        self.label = label
        self.home = home or f"{WORK}/home-{label}"
        shutil.rmtree(self.home, ignore_errors=True)
        os.makedirs(self.home)
        self.socket_path = os.path.join(self.home, ".cache/tessel-editor/ui.sock")
        self.perf_log = f"{WORK}/results/{label}.perflog.jsonl"
        os.makedirs(f"{WORK}/results", exist_ok=True)
        if os.path.exists(self.perf_log):
            os.remove(self.perf_log)
        environment = {
            **os.environ,
            "HOME": self.home,
            "TESSEL_RUNTIME_DIR": RT[runtime],
            "TESSEL_VIEWPORT_TRACE": "1",
            "TESSEL_PERF_LOG": self.perf_log,
            **(env or {}),
        }
        self.before_webkit = set(webkit_pids())
        self.log_path = os.path.join(self.home, "editor.log")
        self.logf = open(self.log_path, "w")
        self.process = subprocess.Popen([binary], env=environment, stdout=self.logf, stderr=self.logf, start_new_session=True)
        self.pid = self.process.pid
        self.started = time.time()

    # --- transport -------------------------------------------------------
    def send(self, request, timeout=120):
        started = time.perf_counter()
        with socket.socket(socket.AF_UNIX) as conn:
            conn.settimeout(timeout)
            conn.connect(self.socket_path)
            conn.sendall((json.dumps(request) + "\n").encode())
            data = b""
            while not data.endswith(b"\n"):
                chunk = conn.recv(1 << 20)
                if not chunk:
                    break
                data += chunk
        response = json.loads(data)
        response["_ms"] = (time.perf_counter() - started) * 1000
        return response

    def op(self, request, timeout=120):
        return self.send({"version": 1, "command": "op", "request": request}, timeout)

    def get(self, target):
        return self.send({"version": 1, "command": "get", "target": target})

    def invoke(self, target, action=None):
        request = {"version": 1, "command": "invoke", "target": target}
        if action:
            request["action"] = action
        return self.send(request)

    def viewport(self):
        return self.get("monitor.viewport")["data"]["state"]

    def render_status(self):
        data = self.op({"op": "render.status"})["data"]
        return data.get("report", data)

    def wait_socket(self, seconds=60):
        deadline = time.time() + seconds
        while time.time() < deadline:
            if self.process.poll() is not None:
                raise RuntimeError("editor exited: " + open(self.log_path).read()[-500:])
            if os.path.exists(self.socket_path):
                try:
                    if self.viewport()["phase"] == "ready":
                        return time.time() - self.started
                except Exception:
                    pass
            time.sleep(0.2)
        raise RuntimeError("viewport never ready")

    def activate(self):
        self.invoke("window.activate")

    def open_project(self, path, timeout=300):
        t = time.time()
        reply = self.op({"op": "project.open", "path": path}, timeout)
        return reply, time.time() - t

    def first_frame(self, frame=0, timeout=600):
        """render.frame until media decode/loads are done; returns seconds."""
        t = time.time()
        while time.time() - t < timeout:
            reply = self.op({"op": "render.frame", "frame": frame}, 300)
            if reply.get("ok"):
                return time.time() - t, reply
            code = (reply.get("error") or {}).get("code")
            if code not in ("media_preparing", "cancelled"):
                return time.time() - t, reply
            time.sleep(0.3)
        return None, None

    def settle(self, timeout=60):
        """Wait until the monitor shows the picture of the playhead (nothing pending)."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            state = self.viewport()
            session = state["session"]
            last = state.get("last_frame")
            if state["phase"] == "ready" and session["status"] == "ready" and not session.get("pending") and last and last["revision"] == session["revision"] and not session.get("refine_pending"):
                return True
            time.sleep(0.05)
        return False

    def seek(self, frame):
        return self.send({"version": 1, "command": "set", "target": "timeline.playhead", "field": "frame", "value": str(frame)})

    def stop(self):
        try:
            self.process.terminate()
            self.process.wait(10)
        except Exception:
            self.process.kill()
        self.logf.close()

    # --- memory ----------------------------------------------------------
    def mine_webkit(self):
        now = webkit_pids()
        return {pid: info for pid, info in now.items() if pid not in self.before_webkit}

    def memory(self):
        out = {}
        rss = subprocess.run(["ps", "-o", "rss=", "-p", str(self.pid)], capture_output=True, text=True).stdout.strip()
        out["editor_rss_mb"] = round(int(rss) / 1024, 1) if rss else None
        out["editor_footprint_mb"] = footprint_mb(self.pid)
        procs = {}
        for pid, (rss_kb, name) in self.mine_webkit().items():
            procs[name + ":" + str(pid)] = {"rss_mb": round(rss_kb / 1024, 1), "footprint_mb": footprint_mb(pid)}
        out["webkit"] = procs
        return out


def read_perf_log(path):
    rows = []
    if not os.path.exists(path):
        return rows
    with open(path) as handle:
        for line in handle:
            try:
                rows.append(json.loads(line))
            except ValueError:
                pass
    return rows


def page_frames(rows, t_from=None, t_to=None):
    frames = []
    for row in rows:
        if row.get("k") == "page" and row["msg"].get("perf") == "frame":
            if (t_from is None or row["t"] >= t_from) and (t_to is None or row["t"] <= t_to):
                frames.append({**row["msg"], "host_t": row["t"]})
    return frames


def kind_rows(rows, kind, t_from=None, t_to=None):
    return [r for r in rows if r.get("k") == kind and (t_from is None or r["t"] >= t_from) and (t_to is None or r["t"] <= t_to)]


def probe_now(rows):
    return rows[-1]["t"] if rows else 0.0


def wait_assets(editor, project_assets=None, timeout=600):
    """Poll until every asset reports ready in the viewport session."""
    t = time.time()
    while time.time() - t < timeout:
        state = editor.viewport()
        if state["phase"] == "ready":
            return time.time() - t
        time.sleep(0.5)
    return None


def dump(path, obj):
    with open(path, "w") as handle:
        json.dump(obj, handle, indent=1, default=str)


def gpu_util():
    out = subprocess.run(["ioreg", "-r", "-d", "1", "-w0", "-c", "IOAccelerator"], capture_output=True, text=True).stdout
    import re
    m = re.search(r'"Device Utilization %"=(\d+)', out)
    return int(m.group(1)) if m else None


class GpuSampler:
    def __init__(self, period=0.4):
        self.period, self.values, self.stop_ev = period, [], threading.Event()
        self.thread = threading.Thread(target=self.loop, daemon=True)

    def loop(self):
        while not self.stop_ev.is_set():
            v = gpu_util()
            if v is not None:
                self.values.append(v)
            self.stop_ev.wait(self.period)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *a):
        self.stop_ev.set()
        self.thread.join(2)

    def report(self):
        return {"n": len(self.values), "mean": round(statistics.fmean(self.values), 1) if self.values else None, "max": max(self.values) if self.values else None}


def wait_quiet_gpu(limit=22, timeout=90):
    """Waits until two consecutive GPU-utilisation samples are <= limit (other agents also use the GPU)."""
    t = time.time()
    ok = 0
    while time.time() - t < timeout:
        v = gpu_util()
        ok = ok + 1 if v is not None and v <= limit else 0
        if ok >= 3:
            return True
        time.sleep(0.6)
    return False
