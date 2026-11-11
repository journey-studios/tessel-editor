"""One fresh editor and the raw record of what it did.

`run_session` is the only place that starts an editor for an experiment: its own HOME, socket and window, the environment of the arm
(nothing inherited from the shell), the trace polled with a cursor, the memory and the external load sampled, and a record that is complete
whatever happened. What the editor is asked to play is the experiment's `body`; how the trace is polled is its `collect` (a parameter, never a
patched module)."""
import os
import pathlib
import platform
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time

from ab import ROOT
from ab.project import new_workspace_copy
from ab.stats import merge_by_key
from ab.validity import redact

POLL_SECONDS = 2.0
MIN_FREE_GIB_START = 16.0
MIN_FREE_GIB_FLOOR = 12.0
# A unix socket path is limited to about 100 bytes, so a session's folder cannot live under a long TMPDIR.
SOCKET_ROOT = "/tmp"
# The control environment of every experiment: the first-use factors and the probe's GPU wait explicitly off, so nothing else can turn one on.
CONTROL_ENV = {"TESSEL_VIEWPORT_PREUPLOAD": "0", "TESSEL_VIEWPORT_WARM_DRAW": "0", "TESSEL_VIEWPORT_PROBE_GPU_FINISH": "0"}
PLAYS = ("first", "repeat")


def workdir(prefix):
    """A scratch folder with a path short enough for the editor's socket."""
    return tempfile.mkdtemp(prefix=prefix, dir=SOCKET_ROOT)


def session_env(mode, extra=None):
    """What every session starts from: GPU rendering, the trace on, the asset I/O mode; an experiment adds its own switches."""
    return {"TESSEL_GL": "gpu", "TESSEL_VIEWPORT_TRACE": "1", "TESSEL_VIEWPORT_ASSET_IO": mode, **(extra or {})}



def process_table():
    """{pid: (ppid, rss_kib, cpu_percent, command)} of every process."""
    out = subprocess.run(["ps", "-Ao", "pid=,ppid=,rss=,pcpu=,command="], capture_output=True, text=True).stdout
    table = {}
    for line in out.splitlines():
        parts = line.split(None, 4)
        if len(parts) == 5 and parts[0].isdigit() and parts[1].isdigit() and parts[2].isdigit():
            try:
                table[int(parts[0])] = (int(parts[1]), int(parts[2]), float(parts[3]), parts[4])
            except ValueError:
                pass
    return table


class Sampler:
    """Every `every` seconds: editor RSS, RSS of the WebKit helpers that did not exist at the baseline, and the CPU of everything
    else (external load, in cores). The helper set is attributed by "new since the baseline"; see `memory_summary` for the limits."""

    def __init__(self, pid, baseline, every=POLL_SECONDS):
        self.pid, self.baseline, self.every = pid, set(baseline), every
        self.rows, self.external, self._stop = [], [], threading.Event()
        self._thread = threading.Thread(target=self._run, daemon=True)

    def _run(self):
        started = time.monotonic()
        while not self._stop.is_set():
            table = process_table()
            own = {self.pid} | {p for p, row in table.items() if "com.apple.WebKit." in row[3] and p not in self.baseline}
            editor = table.get(self.pid, (0, 0, 0, ""))[1]
            helpers = sum(table[p][1] for p in own if p != self.pid and p in table)
            external = sum(row[2] for p, row in table.items() if p not in own and p != os.getpid())
            elapsed = round(time.monotonic() - started, 1)
            self.rows.append([elapsed, round(editor / 1024, 1), round(helpers / 1024, 1), round((editor + helpers) / 1024, 1)])
            self.external.append(round(external / 100.0, 2))
            self._stop.wait(self.every)

    def start(self):
        self._thread.start()
        return self

    def stop(self):
        self._stop.set()
        self._thread.join(5)
        return self


def webkit_pids():
    return {p for p, row in process_table().items() if "com.apple.WebKit." in row[3]}


def collect(lab, state, deadline):
    """One poll: new trace events (cursor), the frame table, the load average. Returns False when the editor stopped answering."""
    trace = lab.op({"op": "render.trace", "since": state["cursor"], "limit": 16384})
    if not trace.get("ok"):
        state["errors"].append(f"render.trace: {trace}")
        return False
    data = trace["data"]
    state["lost"] += data.get("lost", 0)
    state["events"].extend(data["events"])
    if data["events"]:
        state["cursor"] = data["next"]
    status = lab.report()
    merge_by_key(state["frames"], ((status.get("session") or {}).get("trace") or {}).get("recent"), "id")
    state["status"] = {"asset_io": status.get("asset_io"), "stats": (status.get("session") or {}).get("stats"),
                       "error": (status.get("session") or {}).get("error"),
                       "preupload": (status.get("session") or {}).get("preupload"), "warm_draw": (status.get("session") or {}).get("warm_draw")}
    state["loadavg"].append([round(v, 2) for v in os.getloadavg()])
    return True


def flatten_events(raw):
    return [{"seq": e["seq"], "t_ms": e["t_ms"], "kind": e["kind"], "fields": e["fields"]} for e in raw]


def editor_exit(lab):
    """Did the editor process end before the harness stopped it? Return code (negative: signal) and the log lines that
    say why, with absolute paths removed (the log can name private files)."""
    code = lab.process.poll()
    lines = []
    try:
        lab.log.flush()
        for line in pathlib.Path(lab.log_path).read_text(errors="replace").splitlines():
            if re.search(r"panic|fatal|abort|SIG|error|Error|crash", line):
                lines.append(redact(line))
    except OSError:
        pass
    return {"returncode_before_stop": code, "alive": code is None, "log_lines": lines[-12:]}


def play(W, lab, state, collect, name, start, seconds, finished, poll=POLL_SECONDS):
    """Plays from frame `start` polling the trace every `poll` seconds until `finished(status)`. Returns the stop report's counters, or None when the
    play did not start or did not end by itself within `seconds` + 40 (the reason is in `state["errors"]`): a play that ran out of time is not a play."""
    started = W.retry_while_preparing(lambda: lab.op({"op": "playback.start", "from": start}))
    if not started.get("ok"):
        state["errors"].append(f"playback.start({name}): {started}")
        return None
    deadline = time.time() + seconds + 40
    ended = False
    while time.time() < deadline:
        time.sleep(poll)
        collect(lab, state, deadline)
        if finished(lab.op({"op": "playback.status"}).get("data") or {}):
            ended = True
            break
    report = (lab.op({"op": "playback.stop"}).get("data") or {}).get("report") or {}
    played = report if "ticks" in report else (report.get("last") or {})
    time.sleep(1.0)
    collect(lab, state, deadline)
    if not ended:
        state["errors"].append(f"play({name}) did not end within {seconds + 40:.0f} s")
        return None
    return {key: played.get(key) for key in ("ticks", "skipped_frames", "max_gap_frames")}


def whole_project_twice(W, project):
    """The body of a session of `real-latency-ab.py`: the whole project from frame 0, then again in the same process (resident cache)."""
    def body(lab, state, collect, record):
        plays = {}
        for name in PLAYS:
            plays[name] = play(W, lab, state, collect, name, 0, project["length"] / project["fps"], lambda status: status.get("state") != "playing")
            if plays[name] is None:
                break
            time.sleep(2.0)
        record["played"] = plays
        return all(plays.get(name) is not None for name in PLAYS)
    return body


def one_window(W, project, window):
    """The body of a session of `viewport-transit-probe.py`: a window of the project once, stopped by polling when the playhead reaches its end."""
    start, end = window

    def body(lab, state, collect, record):
        played = play(W, lab, state, collect, "window", start, (end - start) / project["fps"],
                      lambda status: status.get("state") != "playing" or (status.get("frame") or 0) >= end, poll=0.5)
        return played is not None
    return body


def run_session(W, binary, document, record, env, body, stress=0, collect=collect):
    """One fresh editor with `env` (plus the repository's runtime page unless `env` names another): opens a private copy of `document`, runs
    `body(lab, state, collect, record)` (the plays; True when they all ran to their end) and returns `record`, completed.

    `record` is what the experiment calls the session (its number, arm, mode, window ...); a session that dies is reported in `record["error"]`,
    never hidden, and the editor, the sampler and the busy processes are stopped on every path."""
    work = workdir("ab-")
    record.update({"started_unix": time.time(), "env": env,
                   "host": {"ncpu": os.cpu_count(), "system": platform.platform(), "loadavg_start": [round(v, 2) for v in os.getloadavg()],
                            "free_gib_start": round(W.free_gib(), 1)}})
    state = {"cursor": 0, "events": [], "frames": {}, "lost": 0, "errors": [], "loadavg": [], "status": {}}
    baseline = webkit_pids()
    lab = sampler = None
    hogs = []
    try:
        if W.free_gib() < MIN_FREE_GIB_FLOOR:
            raise RuntimeError(f"free disk {W.free_gib():.1f} GiB is below the {MIN_FREE_GIB_FLOOR} GiB floor")
        lab = W.Lab(str(binary), work, {"TESSEL_RUNTIME_DIR": str(ROOT / "runtime"), **env})
        lab.wait_ready()
        lab.invoke("window.activate")
        time.sleep(1.0)
        opened = lab.op({"op": "project.open", "path": new_workspace_copy(document, os.path.join(work, "copy"))})
        if not opened.get("ok"):
            raise RuntimeError(f"project.open: {opened}")
        sampler = Sampler(lab.process.pid, baseline).start()
        for _ in range(stress):
            hogs.append(subprocess.Popen([sys.executable, "-c", "while True: pass"]))
        time.sleep(2.0)
        collect(lab, state, 0)
        record["plays_ok"] = bool(body(lab, state, collect, record)) and not state["errors"]
    except Exception as error:  # a session that dies is reported, not hidden
        record["error"] = repr(error)
        record["plays_ok"] = False
    finally:
        for hog in hogs:
            hog.kill()
        if sampler:
            sampler.stop()
            record["memory_series"] = sampler.rows
            record["external_cores"] = sampler.external
        if lab:
            try:
                collect(lab, state, 0)
            except Exception:
                pass
            record["editor_exit"] = editor_exit(lab)
            lab.stop()
        shutil.rmtree(work, ignore_errors=True)
    record.update({"events": flatten_events(state["events"]), "events_lost": state["lost"],
                   "frames": sorted(state["frames"].values(), key=lambda f: f["id"]), "status": state["status"], "errors": state["errors"],
                   "loadavg": state["loadavg"], "finished_unix": time.time()})
    return record
