#!/usr/bin/env python3
"""Proof of the debug performance meter (F049) in the *real window*.

    python3 scripts/perf-meter-proof.py [--project /path/to/a.tessel.json] [--shots DIR]
        [--binary target/release/tessel-editor] [--seconds 8] [--rounds 3] [--splat-wait 120] [--paint-probe] [--keep]

Starts the DIRECT binary with an isolated HOME (own socket; the user's own editor is never
addressed). With --project, a PRIVATE COPY of that project is opened first and the script reports
whether the runtime could draw it within --splat-wait seconds. The copy keeps the project's
folder layout (the document is copied; every other entry of its folder is a symlink to the
original, except `.tessel`, the editor's own state), so document-relative media paths resolve
exactly as in the original and the original is only read: its document hash and listing are
compared before and after. The layouts and the cost are then measured on a synthetic H.264
project made with FFmpeg, so the monitor really presents pictures; that project is saved with
its clip INSIDE its folder (relative path) and measured from a private copy of itself, which
proves the copy logic natively (a JSON-only copy is the negative control: `media.status` reports its
media missing, while the private copy resolves it and `render.frame` draws). The clip is imported
(`media.import`, hashed), because an asset that was only added can never be prepared.
Everything goes through semantic requests (`get`, `set`, `invoke`, `op`); no pointer, no
coordinates. It checks:

  * hidden: the node has no readout and `monitor.viewport` reports no UI-signature repaint
    requests at rest (`ui.repaints` counts IPC messages that change the UI, NOT window paints;
    window paints are only counted with --paint-probe, see below);
  * the shortcut's handler (`invoke toolbar.perf_meter toggle`) and `set ... visible` share state;
  * placement in three layouts: default (card over the inspector), inspector closed (card moved
    out of the native view) and the monitor maximized (status-bar strip);
  * the cost: playback of the same span with the meter hidden and shown, alternating, comparing
    what the app already counts without the meter (`playback.stop` report: skipped frames and the
    largest gap; `monitor.viewport`: pictures accepted and `ui.repaints`), plus what the meter itself
    reads while shown. The fixture is sized for --seconds plus a margin; a run whose playback ended
    before its window is a failed check and is left out of the means;
  * pause and resume: enabling the meter while paused reads a rest and no running clock, a
    `playback.resume` of a playback that never paused keeps the loss counts, and resuming after
    a real pause does not record the pause as an interval.

--paint-probe needs a local build that logs `PAINT <ms>` per window paint (not part of the
product); without it the window-paint rate is reported as "not measured", never as zero. With
--shots DIR it captures the window with `screencapture -l <window id of THIS process>`; a
capture that fails (no window, `screencapture` exit status, missing or empty file) is a failed
check and its path is not reported. Prints one JSON report on the last line, whatever happens:
it carries `ok` and the list of `failures`. An exception in the middle of the run (a missing key
in an answer, a missing fixture) is recorded as a failed check (`aborted` has its traceback) with
everything collected before it kept in the report; the editor is stopped and its log is closed
first, a failed run keeps that log (`editor_log`, plus its tail) and `--keep` keeps the whole
working directory. Exit status 1 on a failed check or an exception.
"""
import hashlib
import json
import math
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import traceback

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
NODE = "toolbar.perf_meter"
FPS = 24
# Seconds of playback the fixture has beyond --seconds: the wait for the clock to start, the
# settle before it and the pause/resume leg all play from frame 0 of the same fixture.
FIXTURE_MARGIN_SECONDS = 12
MAX_SECONDS = 600
# What the editor owns inside a project folder: never linked into a private copy.
EDITOR_OWNED = {".tessel"}
failures = []
report = {"checks": [], "layouts": {}, "cost": []}


def arg(name, default=None):
    return sys.argv[sys.argv.index(name) + 1] if name in sys.argv else default


def check(name, condition, detail=""):
    report["checks"].append({"check": name, "ok": bool(condition), "detail": str(detail)[:300]})
    print(("PASS " if condition else "FAIL ") + name + (f"  [{detail}]" if detail and not condition else ""), flush=True)
    if not condition:
        failures.append(name)


class Editor:
    def __init__(self, binary, home, extra_env=None):
        self.socket_path = os.path.join(home, ".cache/tessel-editor/ui.sock")
        self.log_path = os.path.join(home, "editor.log")
        self.log = open(self.log_path, "w")
        self.process = subprocess.Popen([binary], env={**os.environ, "HOME": home, **(extra_env or {})}, stdout=self.log, stderr=self.log, start_new_session=True)

    def send(self, request, timeout=120):
        with socket.socket(socket.AF_UNIX) as conn:
            conn.settimeout(timeout)
            conn.connect(self.socket_path)
            conn.sendall((json.dumps(request) + "\n").encode())
            data = b""
            while not data.endswith(b"\n"):
                chunk = conn.recv(65536)
                if not chunk:
                    break
                data += chunk
        return json.loads(data)

    def op(self, request, timeout=120):
        return self.send({"version": 1, "command": "op", "request": request}, timeout)

    def get(self, target):
        return self.send({"version": 1, "command": "get", "target": target})

    def set(self, target, field, value):
        return self.send({"version": 1, "command": "set", "target": target, "field": field, "value": value})

    def invoke(self, target, action=None):
        request = {"version": 1, "command": "invoke", "target": target}
        if action:
            request["action"] = action
        return self.send(request)

    def meter(self):
        return self.get(NODE)["data"]["state"]

    def viewport(self):
        return self.get("monitor.viewport")["data"]["state"]

    def wait_ready(self, seconds=180):
        deadline = time.time() + seconds
        while time.time() < deadline:
            if self.process.poll() is not None:
                with open(self.log_path) as handle:
                    raise RuntimeError(f"the editor exited ({self.process.returncode}): {handle.read()[-600:]}")
            if os.path.exists(self.socket_path):
                try:
                    if self.viewport()["phase"] == "ready":
                        return
                except (OSError, ValueError, KeyError, TypeError):
                    pass
            time.sleep(0.25)
        raise RuntimeError("the viewport never became ready")

    def window_id(self):
        """The on-screen window of THIS process (never the user's own editor)."""
        script = (
            "import CoreGraphics\n"
            "let l = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as! [[String: Any]]\n"
            f"for w in l {{ if (w[kCGWindowOwnerPID as String] as? Int) == {self.process.pid} {{ print(w[kCGWindowNumber as String]!) ; break }} }}"
        )
        return subprocess.run(["swift", "-e", script], capture_output=True, text=True).stdout.strip()

    def shot(self, directory, name):
        """Captures this process's window; the path only when a non-empty file was written.

        A capture that was asked for (--shots) and failed is a failed check, so the report never
        advertises a screenshot that does not exist."""
        if not directory:
            return None
        os.makedirs(directory, exist_ok=True)
        self.invoke("window.activate")
        time.sleep(0.8)
        window = self.window_id()
        if not window:
            check(f"screenshot {name}: the window of this process was found", False, "no on-screen window id")
            return None
        path = os.path.join(directory, f"{name}.png")
        if os.path.exists(path):
            os.remove(path)
        done = subprocess.run(["screencapture", "-x", "-o", "-l", window, path], check=False, capture_output=True, text=True)
        written = os.path.isfile(path) and os.path.getsize(path) > 0
        check(f"screenshot {name}: captured", done.returncode == 0 and written,
              f"screencapture exit {done.returncode}, file {'present' if written else 'missing or empty'}: {(done.stderr or '').strip()[:120]}")
        return path if done.returncode == 0 and written else None

    def stop(self):
        """Ends THIS process (never another editor) and closes its log so it can be collected."""
        try:
            self.process.terminate()
            try:
                self.process.wait(10)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(10)
        finally:
            self.log.close()


def fixture_frames(seconds):
    """Frames the fixture needs so playback outlasts `seconds` plus the margin; refuses the absurd."""
    if not (seconds > 0 and math.isfinite(seconds)) or seconds > MAX_SECONDS:
        raise ValueError(f"--seconds must be in (0, {MAX_SECONDS}], got {seconds}")
    return int(math.ceil((seconds + FIXTURE_MARGIN_SECONDS) * FPS))


def fingerprint(document):
    """What must not change in a project that was only read: its document and its folder listing."""
    folder = os.path.dirname(os.path.abspath(document))
    with open(document, "rb") as handle:
        digest = hashlib.sha256(handle.read()).hexdigest()
    return {"document_sha256": digest, "entries": sorted(os.listdir(folder))}


def private_copy(document, destination):
    """A private copy of a saved project that resolves its relative media like the original.

    Document-relative paths are resolved against the document's folder, so copying only the JSON
    breaks them. The document is copied; every other entry of its folder is a symlink to the
    original (read only for the proof), except what the editor owns (`.tessel`: recovery and
    state), which lives in the copy. Returns the path of the copied document."""
    source = os.path.abspath(document)
    folder = os.path.dirname(source)
    os.makedirs(destination)
    copied = os.path.join(destination, os.path.basename(source))
    shutil.copyfile(source, copied)
    for entry in sorted(os.listdir(folder)):
        if entry == os.path.basename(source) or entry in EDITOR_OWNED:
            continue
        os.symlink(os.path.join(folder, entry), os.path.join(destination, entry))
    return copied


def error_code(answer):
    error = (answer or {}).get("error")
    return error.get("code") if isinstance(error, dict) else None


def frame_when_prepared(editor, timeout=60.0, sleep=time.sleep, now=time.monotonic):
    """`render.frame` 0, asked again while the host answers `media_preparing` (its media is still
    being prepared: a wait, not a verdict). The last answer is returned; one that is still
    `media_preparing` after `timeout` is a failure of the caller's check, not a pass."""
    deadline = now() + timeout
    while True:
        answer = editor.op({"op": "render.frame", "frame": 0})
        if answer.get("ok") or error_code(answer) != "media_preparing" or now() >= deadline:
            return answer
        sleep(0.5)


def media_resolution(editor, base):
    """What `media.status` says about the open project's media: how many are missing, their states and
    whether every resolved path lies under `base` (the folder of the opened document)."""
    data = editor.op({"op": "media.status"}).get("data") or {}
    assets = data.get("assets") or []
    root = os.path.realpath(base)
    return {
        "missing": data.get("missing"),
        "states": [asset.get("state") for asset in assets],
        "under_base": bool(assets) and all(os.path.realpath(str(asset.get("path"))).startswith(root + os.sep) or str(asset.get("path")).startswith(base + os.sep) for asset in assets),
    }


def wait_job(editor, job, timeout=120.0, sleep=time.sleep, now=time.monotonic):
    """`job.wait` answers while the job still runs (a bounded wait), so ask again until it ends."""
    deadline = now() + timeout
    while True:
        info = editor.op({"op": "job.wait", "job": job}).get("data") or {}
        if info.get("state") not in ("queued", "running") or now() >= deadline:
            return info
        sleep(0.25)


def absolute_strings_outside_origin(value, prefix, key=None):
    """Strings under `prefix` anywhere in a saved document except `identity.origin`, which is an
    informational record of where the file came from, not a path the editor resolves."""
    if isinstance(value, dict):
        return [hit for k, v in value.items() for hit in absolute_strings_outside_origin(v, prefix, k)]
    if isinstance(value, list):
        return [hit for v in value for hit in absolute_strings_outside_origin(v, prefix, key)]
    return [value] if isinstance(value, str) and key != "origin" and prefix in value else []


def synthetic_project(editor, work, frames):
    """A fresh 1280x720 24 fps project with one H.264 clip, so every playback tick draws a picture.

    The clip lives inside the project folder (`work/rel/media`), so the saved document stores its
    path relative to the document. Returns (ok, why, document)."""
    folder = os.path.join(work, "rel")
    os.makedirs(os.path.join(folder, "media"))
    clip = os.path.join(folder, "media", "clip.mp4")
    document = os.path.join(folder, "perf.tessel.json")
    made = subprocess.run(
        ["ffmpeg", "-nostdin", "-v", "error", "-y", "-f", "lavfi", "-i", f"testsrc2=size=1280x720:rate={FPS}:duration={frames / FPS + 2:.3f}",
         "-frames:v", str(frames), "-c:v", "libx264", "-crf", "20", "-preset", "veryfast", "-pix_fmt", "yuv420p", "-g", str(FPS),
         "-movflags", "+faststart", clip],
        capture_output=True, text=True,
    )
    if made.returncode != 0:
        return False, made.stderr[-300:], None
    # `media.import` (not `asset.add`): a hashed asset can be decoded when the project is reopened.
    # An asset that was only added has no hash, its private copy can never be prepared and
    # `render.frame` answers `media_preparing` forever (the job fails: "never hashed").
    created = editor.op({"op": "project.create", "name": "Perf meter proof", "sequence": "main", "rate": FPS, "width": 1280, "height": 720, "length": frames, "dir": folder})
    if not created.get("ok"):
        return False, str(created)[:300], None
    imported = editor.op({"op": "media.import", "paths": [clip], "mode": "reference"})
    if not imported.get("ok"):
        return False, str(imported)[:300], None
    finished = wait_job(editor, imported["data"]["job"])
    if finished.get("state") != "succeeded" or finished.get("result_state") != "applied":
        return False, f"import job: {finished.get('state')} {finished.get('result_state')} {finished.get('error')}", None
    for step in [
        {"op": "track.add", "id": "t", "name": "T", "kind": "video"},
        {"op": "clip.add", "id": "c", "track": "t", "asset": "clip", "start": 0, "duration": frames},
        {"op": "project.save", "path": document},
    ]:
        response = editor.op(step)
        if not response.get("ok"):
            return False, str(response)[:300], None
    return True, None, document


def settle(editor, seconds=1.2):
    editor.invoke("window.activate")
    time.sleep(seconds)


def reading(state, source):
    return ((state.get("readout") or {}).get(source) or {})


def painted_between(editor, start_ms, end_ms):
    """Paints the instrumented build logged inside [start, end] (ms since the epoch), or None."""
    editor.log.flush()
    count = None
    with open(editor.log_path) as handle:
        for line in handle:
            if line.startswith("PAINT "):
                count = count or 0
                if start_ms <= float(line.split()[1]) <= end_ms:
                    count += 1
    return count


def stop_report(stopped):
    """(flat report, ended_early) of a `playback.stop` answer.

    Stopping a playback that is running answers the flat report of that run. Stopping one that
    already ended on its own answers `{"state": "stopped", "last": <that report>}`: the metrics
    are nested and the run did NOT cover the whole window, so it must not be read as a full one."""
    report_ = (stopped.get("data") or {}).get("report") or {}
    if "ticks" in report_:
        return report_, False
    if "last" in report_:
        return report_.get("last") or {}, True
    return report_, True


def playback_run(editor, seconds, meter_on, sleep=time.sleep, wall=time.time):
    """Plays `seconds` from frame 0 and returns what the app counts around it.

    The two arms measure the same span: the viewport is read at `window_end` (before anything the
    arms do differently) and pictures and UI changes are `end - mark`, and both arms then make the
    same requests (`meter` node, `playback.status`, `playback.stop`), so the stop report covers the
    same extra round trips too. Only the meter-on arm keeps what its node answered. The run is
    `ok` only when the playback was still running when the window closed; one that ended early
    (fixture shorter than the window) is reported with the reason and no metrics are taken from it."""
    editor.op({"op": "playback.stop"})
    editor.op({"op": "seek", "frame": 0})
    sleep(1.0)
    started = editor.op({"op": "playback.start", "from": 0})
    if not started.get("ok"):
        return {"error": str(started)[:200]}
    deadline = wall() + 30
    while wall() < deadline:
        status = editor.op({"op": "playback.status"})["data"]
        if status.get("state") == "playing" and status.get("clock") != "starting":
            break
        sleep(0.1)
    mark = editor.viewport()
    window_start = wall() * 1000
    sleep(seconds)
    window_end = wall() * 1000
    end = editor.viewport()
    node = editor.meter()
    live = node if meter_on else None
    still_playing = (editor.op({"op": "playback.status"}).get("data") or {}).get("state") == "playing"
    stopped = editor.op({"op": "playback.stop"})
    played, ended_early = stop_report(stopped)
    stats = lambda state: state["session"]["stats"]
    ui = lambda state: (state.get("ui") or {}).get("repaints", 0)
    result = {
        "meter": "on" if meter_on else "off",
        "seconds": seconds,
        "ok": still_playing and not ended_early,
        "ticks": played.get("ticks"),
        "skipped_frames": played.get("skipped_frames"),
        "max_gap_frames": played.get("max_gap_frames"),
        "pictures_accepted": stats(end)["accepted"] - stats(mark)["accepted"],
        "ui_repaints": ui(end) - ui(mark),
    }
    if not result["ok"]:
        result["error"] = "playback ended before the measurement window (fixture too short): not a measurement"
    paints = painted_between(editor, window_start, window_end)
    if paints is not None:
        # Independent of the meter: a build instrumented to log every window paint (see --paint-probe).
        result["window_paints"] = paints
        result["window_paints_per_s"] = round(paints / seconds, 1)
    if live:
        result["meter_reads"] = {
            "ui": reading(live, "ui"),
            "monitor": reading(live, "monitor"),
            "playback_loss": reading(live, "playback_loss"),
            "render_ms": (live.get("readout") or {}).get("render_ms"),
            "frame": (live.get("readout") or {}).get("frame"),
        }
    return result


def mean(runs, arm, key):
    """Mean of `key` over the runs of one arm that really covered their window (failed ones are out)."""
    values = [run[key] for run in runs if run.get("meter") == arm and run.get("ok") and isinstance(run.get(key), (int, float))]
    return round(sum(values) / len(values), 2) if values else None


def record_abort(error):
    """An exception raised mid-run is a failed check with its reason, never a bare traceback.

    The checks and cost rounds collected so far stay in the report."""
    text = "".join(traceback.format_exception_only(type(error), error)).strip()
    report["aborted"] = {"type": type(error).__name__, "message": str(error)[:500], "traceback": "".join(traceback.format_exception(type(error), error, error.__traceback__))[-2500:]}
    check(f"the run completed without an exception ({type(error).__name__})", False, text)


def conclude(editor, work, keep_all, log_tail=2000):
    """Own cleanup, then the verdict the report carries. Never raises.

    The editor is stopped (its log is closed) BEFORE anything is deleted. A run with a failed check
    or an exception keeps the editor's log (copied next to the work directory, which is then
    removed) and puts its tail in the report; `--keep` keeps the whole work directory. A clean run
    leaves nothing behind. Returns the part of the report this adds."""
    extra = {}
    if editor is not None:
        try:
            editor.stop()
        except Exception as error:  # a stuck editor must not hide the verdict
            check("the editor stopped cleanly", False, error)
    failed = bool(failures)
    log_path = editor.log_path if editor is not None else None
    if log_path and os.path.isfile(log_path) and (failed or keep_all):
        with open(log_path, errors="replace") as handle:
            extra["editor_log_tail"] = handle.read()[-log_tail:]
    if keep_all:
        extra["kept"] = work
    else:
        if failed and log_path and os.path.isfile(log_path):
            saved = os.path.join(tempfile.gettempdir(), f"{os.path.basename(work)}.editor.log")
            try:
                shutil.copyfile(log_path, saved)
                extra["editor_log"] = saved
            except OSError as error:
                extra["editor_log_error"] = str(error)
        shutil.rmtree(work, ignore_errors=True)
    extra["ok"] = not failures
    extra["failures"] = list(failures)
    return extra


def main():
    project = arg("--project")
    binary = os.path.abspath(arg("--binary", os.path.join(ROOT, "target/release/tessel-editor")))
    shots = arg("--shots")
    seconds = float(arg("--seconds", "8"))
    rounds = int(arg("--rounds", "3"))
    splat_wait = float(arg("--splat-wait", "120"))
    if (project and not os.path.exists(project)) or not os.path.exists(binary):
        print("usage: perf-meter-proof.py [--project <file.tessel.json>] [--binary <editor>] [--shots DIR] [--seconds N] [--rounds N]")
        return 2
    try:
        frames = fixture_frames(seconds)
    except ValueError as error:
        print(error)
        return 2
    work = tempfile.mkdtemp(prefix="pmp-", dir="/tmp")
    home = os.path.join(work, "home")
    os.makedirs(home)
    # --paint-probe: only for a build that logs `PAINT <ms>` per window paint (a local, uncommitted
    # instrument): it gives the interface rate with the meter hidden, which the meter itself cannot.
    editor = None
    interrupted = None
    try:
        editor = Editor(binary, home, {"TESSEL_PERF_PROBE": "1"} if "--paint-probe" in sys.argv else None)
        editor.wait_ready()
        report["binary"] = binary
        settle(editor)

        # --- hidden costs nothing
        hidden = editor.meter()
        check("hidden: the node says so and carries no readout", hidden["visible"] is False and hidden["readout"] is None, hidden)
        check("the shortcut is published", bool(hidden.get("shortcut")) and bool(hidden.get("shortcut_label")), hidden)
        report["shortcut"] = [hidden.get("shortcut"), hidden.get("shortcut_label")]

        if project:
            # A private copy that keeps the layout, so document-relative media still resolve; the
            # original is only read, and its document and listing are compared afterwards.
            before = fingerprint(project)
            document = private_copy(project, os.path.join(work, "real"))
            opened = editor.op({"op": "project.open", "path": document})
            check("the project copy opens", opened.get("ok"), str(opened)[:300])
            deadline = time.time() + splat_wait
            frame = None
            while time.time() < deadline:
                frame = editor.op({"op": "render.frame", "frame": 0})
                if frame.get("ok"):
                    break
                time.sleep(1.0)
            # Not a pass/fail of the meter: a loaded machine may not draw 1M-splat scenes in time.
            report["real_project_frame"] = "drawn" if frame and frame.get("ok") else str((frame or {}).get("error"))[:200]
            print("real project frame:", report["real_project_frame"], flush=True)
            editor.set(NODE, "visible", True)
            settle(editor, 2.0)
            report["real_project_shot"] = editor.shot(shots, "perf-meter-real-project")
            report["real_project_readout"] = editor.meter()["readout"]
            editor.set(NODE, "visible", False)
            check("the original project was only read (document hash and folder listing unchanged)", fingerprint(project) == before, before)

        ok, why, saved = synthetic_project(editor, work, frames)
        check("a project whose pictures the runtime draws", ok, why)
        if not ok:
            raise RuntimeError(f"no fixture: {why}")
        report["fixture_frames"] = frames
        document_json = json.load(open(saved))
        stored = json.dumps(document_json)
        absolute = absolute_strings_outside_origin(document_json, os.path.join(work, "rel"))
        check("the fixture stores its clip relative to the document", '"media/clip.mp4"' in stored and not absolute, str(absolute)[:200] or stored[:200])
        # Negative control: only the JSON copied elsewhere cannot find its media...
        naive = os.path.join(work, "naive")
        os.makedirs(naive)
        naive_document = shutil.copyfile(saved, os.path.join(naive, os.path.basename(saved)))
        editor.op({"op": "project.open", "path": naive_document})
        broken = media_resolution(editor, naive)
        check("a JSON-only copy cannot find its media: the relative path moved with the base (negative control)",
              broken["missing"] == 1 and broken["under_base"] is True and broken["states"] == ["missing"], str(broken)[:300])
        # ...and the private copy draws, and is what the rest of the proof measures.
        before = fingerprint(saved)
        copy_document = private_copy(saved, os.path.join(work, "copy"))
        opened = editor.op({"op": "project.open", "path": copy_document})
        check("the private copy of the fixture opens", opened.get("ok"), str(opened)[:300])
        resolved = media_resolution(editor, os.path.dirname(copy_document))
        check("the private copy resolves its media through the document's folder (relative path preserved)",
              resolved["missing"] == 0 and resolved["under_base"] is True and resolved["states"] == ["ok"], str(resolved)[:300])
        frame = frame_when_prepared(editor)
        check("a real frame renders from the private copy (relative media preserved)", frame.get("ok"), str(frame)[:300])
        check("the fixture's original folder was only read", fingerprint(saved) == before, before)
        settle(editor, 2.0)

        # `ui.repaints` counts IPC messages that change the UI signature. It is NOT a window paint
        # counter (the app paints a few frames per second at rest by itself), so it can only say
        # that nothing asked the window to change; the paint rate needs --paint-probe.
        at_rest = editor.viewport()["ui"]["repaints"]
        rest_start = time.time() * 1000
        time.sleep(4.0)
        rest_end = time.time() * 1000
        hidden_rest = editor.viewport()["ui"]["repaints"] - at_rest
        check("hidden at rest: no UI-signature repaint requests in 4 s (ui.repaints counts IPC changes, not window paints)", hidden_rest <= 1, editor.viewport()["ui"])
        report["rest_ui_signature_repaints_4s"] = {"hidden": hidden_rest}
        report["rest_window_paints_4s"] = {"hidden": painted_between(editor, rest_start, rest_end) if "--paint-probe" in sys.argv else "not measured (no --paint-probe build)"}

        # --- the gesture is one handler for the shortcut, the button and the CLI
        shown = editor.invoke(NODE, "toggle")
        check("invoke toggle shows the meter", shown.get("ok") and editor.meter()["visible"] is True, shown)
        check("set visible=false hides it again", editor.set(NODE, "visible", False).get("ok") and editor.meter()["visible"] is False)
        check("set visible=true shows it", editor.set(NODE, "visible", True).get("ok") and editor.meter()["visible"] is True)
        settle(editor, 2.5)
        at_rest = editor.viewport()["ui"]["repaints"]
        rest_start = time.time() * 1000
        time.sleep(4.0)
        rest_end = time.time() * 1000
        shown_rest = editor.viewport()["ui"]["repaints"] - at_rest
        report["rest_ui_signature_repaints_4s"]["shown"] = shown_rest
        report["rest_window_paints_4s"]["shown"] = painted_between(editor, rest_start, rest_end) if "--paint-probe" in sys.argv else "not measured (no --paint-probe build)"
        check("shown at rest: no UI-signature repaint requests in 4 s (ui.repaints counts IPC changes, not window paints)", shown_rest <= 1, shown_rest)

        # --- the readout of a still window and of the monitor. Every request above made the window
        # paint (a real frame), so the readout is only "idle" after a quiet second without any.
        time.sleep(1.6)
        state = editor.meter()
        ui, monitor = reading(state, "ui"), reading(state, "monitor")
        # The app itself paints ~4 frames/s at rest with the meter hidden too (measured with the
        # paint probe: 23 paints in 6 s hidden and shown alike), so the honest reading is either
        # "idle" or a small live rate, never a zero and never a high one.
        report["rest_ui_reading"] = ui
        check("shown at rest: the interface reads 'idle' or a small live rate, never a zero", ui.get("state") == "idle" or (ui.get("state") == "live" and 0 < ui.get("fps", 0) < 10), ui)
        check("at rest the monitor is idle or unavailable, and no rate is a zero", monitor.get("state") in ("idle", "unavailable") and ui.get("fps") != 0.0 and monitor.get("fps") != 0.0, monitor)

        # --- three layouts
        def layout(name):
            settle(editor)
            state = editor.meter()
            report["layouts"][name] = state["placement"]
            report["layouts"][name + "_shot"] = editor.shot(shots, f"perf-meter-{name}")
            return state["placement"]

        default = layout("default")
        check("default layout: a floating card", default and default["kind"] == "floating", default)
        editor.set("region:inspector", "visible", False)
        closed = layout("inspector-closed")
        check("inspector closed: the card moved, it did not vanish", closed and closed["kind"] in ("floating", "status_bar_strip"), closed)
        for panel in ("library", "timeline"):
            editor.set(f"region:{panel}", "visible", False)
        maximized = layout("monitor-maximized")
        check("monitor maximized: the status-bar strip", maximized and maximized["kind"] == "status_bar_strip", maximized)
        for panel in ("library", "timeline", "inspector"):
            editor.set(f"region:{panel}", "visible", True)
        settle(editor)
        editor.set(NODE, "visible", False)

        # --- the cost, alternating so a drifting machine hits both arms
        for index in range(rounds):
            for on in (False, True):
                editor.set(NODE, "visible", on)
                settle(editor, 1.5)
                run = playback_run(editor, seconds, on)
                run["round"] = index
                report["cost"].append(run)
                print(json.dumps(run), flush=True)
                check(f"cost round {index} meter {'on' if on else 'off'}: playback covered the whole {seconds:g} s window", run.get("ok"), run.get("error"))
        # --- pause is a rest and resume does not record the pause as a gap
        editor.set(NODE, "visible", True)
        settle(editor, 1.0)
        editor.op({"op": "seek", "frame": 0})
        time.sleep(1.0)
        editor.op({"op": "playback.start", "from": 0})
        time.sleep(2.5)
        # A resume of a playback that never paused is not a new run: the loss counts survive it.
        loss_before = editor.meter()["readout"]["playback_loss"]
        editor.op({"op": "playback.resume"})
        loss_after = editor.meter()["readout"]["playback_loss"]
        check("resume on a running playback keeps the loss counts", loss_before.get("state") == "live" and loss_after.get("state") == "live" and loss_after["span_frames"] >= loss_before["span_frames"], [loss_before, loss_after])
        editor.op({"op": "playback.pause"})
        time.sleep(1.8)
        paused = editor.meter()["readout"]
        check("paused: the monitor reads idle (parado), not a stall", paused["monitor"]["state"] == "idle" and paused["playback_loss"]["state"] == "not_playing", [paused["monitor"], paused["playback_loss"]])
        # Switching the meter on while paused is a rest too: no running clock, not a 0 fps stall.
        editor.set(NODE, "visible", False)
        editor.set(NODE, "visible", True)
        time.sleep(2.2)
        reenabled = editor.meter()["readout"]
        check("meter enabled while paused: idle, not playing, no loss (not a 0 fps stall)", reenabled["monitor"]["state"] == "idle" and reenabled["playing"] is False and reenabled["playback_loss"]["state"] == "not_playing", [reenabled["monitor"], reenabled["playing"], reenabled["playback_loss"]])
        editor.op({"op": "playback.resume"})
        time.sleep(2.0)
        resumed = editor.meter()["readout"]
        worst = resumed["monitor"].get("worst_interval_ms", 0)
        check("resumed: the pause is not the worst interval", resumed["monitor"]["state"] == "live" and worst < 1500, resumed["monitor"])
        report["pause_resume"] = {"paused": paused["monitor"], "enabled_while_paused": reenabled["monitor"], "resumed": resumed["monitor"]}
        editor.op({"op": "playback.stop"})
        editor.set(NODE, "visible", False)

        editor.set(NODE, "visible", False)

        report["cost_mean"] = {
            arm: {key: mean(report["cost"], arm, key) for key in ("ticks", "skipped_frames", "max_gap_frames", "pictures_accepted", "ui_repaints", "window_paints_per_s")}
            for arm in ("off", "on")
        }
        print(json.dumps(report["cost_mean"]), flush=True)
        if shots:
            editor.set(NODE, "visible", True)
            report["playing_shot"] = None
            editor.op({"op": "seek", "frame": 0})
            time.sleep(1.0)
            editor.op({"op": "playback.start", "from": 0})
            time.sleep(3.0)
            report["playing_shot"] = editor.shot(shots, "perf-meter-playing")
            editor.op({"op": "playback.stop"})
            editor.set(NODE, "visible", False)
    except BaseException as error:  # KeyboardInterrupt too: the report is still printed, then it is re-raised
        record_abort(error)
        if not isinstance(error, Exception):
            interrupted = error
    finally:
        report.update(conclude(editor, work, "--keep" in sys.argv))
        if report.get("kept"):
            print("kept", work)
        print(json.dumps(report))
    if interrupted is not None:
        raise interrupted
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
