#!/usr/bin/env python3
"""Proof of the debug performance meter (F049) in the *real window*.

    python3 scripts/perf-meter-proof.py [--project /path/to/a.tessel.json] [--shots DIR]
        [--binary target/release/tessel-editor] [--seconds 8] [--rounds 3] [--splat-wait 120] [--paint-probe] [--keep]

Starts the DIRECT binary with an isolated HOME (own socket; the user's own editor is never
addressed). With --project, a /tmp COPY of that document is opened first (the original is not
touched; the assets it references are only read) and the script reports whether the runtime
could draw it within --splat-wait seconds. The layouts and the cost are then measured on a
synthetic H.264 project made with FFmpeg, so the monitor really presents pictures. Everything
goes through semantic requests (`get`, `set`, `invoke`, `op`); no pointer, no coordinates. It
checks:

  * hidden: the node has no readout and `monitor.viewport` shows no extra UI repaints at rest;
  * the shortcut's handler (`invoke toolbar.perf_meter toggle`) and `set ... visible` share state;
  * placement in three layouts: default (card over the inspector), inspector closed (card moved
    out of the native view) and the monitor maximized (status-bar strip);
  * the cost: playback of the same span with the meter hidden and shown, alternating, comparing
    what the app already counts without the meter (`playback.stop` report: skipped frames and the
    largest gap; `monitor.viewport`: pictures accepted and `ui.repaints`), plus what the meter itself
    reads while shown.

With --shots DIR it captures the window with `screencapture -l <window id of THIS process>`.
Prints one JSON report on the last line. Exit status 1 on a failed check.
"""
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
NODE = "toolbar.perf_meter"
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
        if not directory:
            return None
        os.makedirs(directory, exist_ok=True)
        self.invoke("window.activate")
        time.sleep(0.8)
        window = self.window_id()
        if not window:
            return None
        path = os.path.join(directory, f"{name}.png")
        subprocess.run(["screencapture", "-x", "-o", "-l", window, path], check=False)
        return path

    def stop(self):
        self.process.terminate()
        try:
            self.process.wait(10)
        except subprocess.TimeoutExpired:
            self.process.kill()


def synthetic_project(editor, work, frames=240):
    """A fresh 1280x720 24 fps project with one H.264 clip, so every playback tick draws a picture."""
    clip = os.path.join(work, "clip.mp4")
    made = subprocess.run(
        ["ffmpeg", "-nostdin", "-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=24:duration=100",
         "-frames:v", str(frames), "-c:v", "libx264", "-crf", "20", "-preset", "veryfast", "-pix_fmt", "yuv420p", "-g", "24",
         "-movflags", "+faststart", clip],
        capture_output=True, text=True,
    )
    if made.returncode != 0:
        return False, made.stderr[-300:]
    steps = [
        {"op": "project.create", "name": "Perf meter proof", "sequence": "main", "rate": 24, "width": 1280, "height": 720, "length": frames},
        {"op": "asset.add", "kind": "video", "id": "clip", "name": "Clip", "path": clip, "rate": 24, "width": 1280, "height": 720, "duration": {"num": frames, "den": 24}},
        {"op": "track.add", "id": "t", "name": "T", "kind": "video"},
        {"op": "clip.add", "id": "c", "track": "t", "asset": "clip", "start": 0, "duration": frames},
    ]
    for step in steps:
        response = editor.op(step)
        if not response.get("ok"):
            return False, str(response)[:300]
    return True, None


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


def playback_run(editor, seconds, meter_on):
    """Plays `seconds` from frame 0 and returns what the app counts around it."""
    editor.op({"op": "playback.stop"})
    editor.op({"op": "seek", "frame": 0})
    time.sleep(1.0)
    before = editor.viewport()
    started = editor.op({"op": "playback.start", "from": 0})
    if not started.get("ok"):
        return {"error": str(started)[:200]}
    deadline = time.time() + 30
    while time.time() < deadline:
        status = editor.op({"op": "playback.status"})["data"]
        if status.get("state") == "playing" and status.get("clock") != "starting":
            break
        time.sleep(0.1)
    mark = editor.viewport()
    window_start = time.time() * 1000
    time.sleep(seconds)
    window_end = time.time() * 1000
    live = editor.meter() if meter_on else None
    stopped = editor.op({"op": "playback.stop"})
    after = editor.viewport()
    played = (stopped.get("data") or {}).get("report") or {}
    stats = lambda state: state["session"]["stats"]
    ui = lambda state: (state.get("ui") or {}).get("repaints", 0)
    result = {
        "meter": "on" if meter_on else "off",
        "seconds": seconds,
        "ticks": played.get("ticks"),
        "skipped_frames": played.get("skipped_frames"),
        "max_gap_frames": played.get("max_gap_frames"),
        "pictures_accepted": stats(after)["accepted"] - stats(mark)["accepted"],
        "ui_repaints": ui(after) - ui(mark),
    }
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


def main():
    project = arg("--project")
    binary = os.path.abspath(arg("--binary", os.path.join(ROOT, "target/release/tessel-editor")))
    shots = arg("--shots")
    seconds = float(arg("--seconds", "8"))
    rounds = int(arg("--rounds", "3"))
    splat_wait = float(arg("--splat-wait", "120"))
    if (project and not os.path.exists(project)) or not os.path.exists(binary):
        print("usage: perf-meter-proof.py [--project <file.tessel.json>] [--binary <editor>] [--shots DIR]")
        return 2
    work = tempfile.mkdtemp(prefix="pmp-", dir="/tmp")
    home = os.path.join(work, "home")
    os.makedirs(home)
    # --paint-probe: only for a build that logs `PAINT <ms>` per window paint (a local, uncommitted
    # instrument): it gives the interface rate with the meter hidden, which the meter itself cannot.
    editor = Editor(binary, home, {"TESSEL_PERF_PROBE": "1"} if "--paint-probe" in sys.argv else None)
    try:
        editor.wait_ready()
        report["binary"] = binary
        settle(editor)

        # --- hidden costs nothing
        hidden = editor.meter()
        check("hidden: the node says so and carries no readout", hidden["visible"] is False and hidden["readout"] is None, hidden)
        check("the shortcut is published", bool(hidden.get("shortcut")) and bool(hidden.get("shortcut_label")), hidden)
        report["shortcut"] = [hidden.get("shortcut"), hidden.get("shortcut_label")]

        if project:
            document = os.path.join(work, "project.tessel.json")
            shutil.copyfile(project, document)
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

        ok, why = synthetic_project(editor, work)
        check("a project whose pictures the runtime draws", ok, why)
        frame = editor.op({"op": "render.frame", "frame": 0})
        check("a real frame renders", frame.get("ok"), str(frame)[:300])
        settle(editor, 2.0)

        at_rest = editor.viewport()["ui"]["repaints"]
        time.sleep(4.0)
        hidden_rest = editor.viewport()["ui"]["repaints"] - at_rest
        check("hidden at rest: the window does not repaint by itself", hidden_rest <= 1, editor.viewport()["ui"])
        report["rest_repaints_4s"] = {"hidden": hidden_rest}

        # --- the gesture is one handler for the shortcut, the button and the CLI
        shown = editor.invoke(NODE, "toggle")
        check("invoke toggle shows the meter", shown.get("ok") and editor.meter()["visible"] is True, shown)
        check("set visible=false hides it again", editor.set(NODE, "visible", False).get("ok") and editor.meter()["visible"] is False)
        check("set visible=true shows it", editor.set(NODE, "visible", True).get("ok") and editor.meter()["visible"] is True)
        settle(editor, 2.5)
        at_rest = editor.viewport()["ui"]["repaints"]
        time.sleep(4.0)
        shown_rest = editor.viewport()["ui"]["repaints"] - at_rest
        report["rest_repaints_4s"]["shown"] = shown_rest
        check("shown at rest: the meter keeps no clock of its own (no repaints in 4 s)", shown_rest <= 1, shown_rest)

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
        # --- pause is a rest and resume does not record the pause as a gap
        editor.set(NODE, "visible", True)
        settle(editor, 1.0)
        editor.op({"op": "seek", "frame": 0})
        time.sleep(1.0)
        editor.op({"op": "playback.start", "from": 0})
        time.sleep(2.5)
        editor.op({"op": "playback.pause"})
        time.sleep(1.8)
        paused = editor.meter()["readout"]
        check("paused: the monitor reads idle (parado), not a stall", paused["monitor"]["state"] == "idle" and paused["playback_loss"]["state"] == "not_playing", [paused["monitor"], paused["playback_loss"]])
        editor.op({"op": "playback.resume"})
        time.sleep(2.0)
        resumed = editor.meter()["readout"]
        worst = resumed["monitor"].get("worst_interval_ms", 0)
        check("resumed: the pause is not the worst interval", resumed["monitor"]["state"] == "live" and worst < 1500, resumed["monitor"])
        report["pause_resume"] = {"paused": paused["monitor"], "resumed": resumed["monitor"]}
        editor.op({"op": "playback.stop"})
        editor.set(NODE, "visible", False)

        editor.set(NODE, "visible", False)

        def mean(arm, key):
            values = [run[key] for run in report["cost"] if run.get("meter") == arm and isinstance(run.get(key), (int, float))]
            return round(sum(values) / len(values), 2) if values else None

        report["cost_mean"] = {
            arm: {key: mean(arm, key) for key in ("ticks", "skipped_frames", "max_gap_frames", "pictures_accepted", "ui_repaints", "window_paints_per_s")}
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
    finally:
        editor.stop()
        if "--keep" in sys.argv:
            print("kept", work)
        else:
            shutil.rmtree(work, ignore_errors=True)
    print(json.dumps(report))
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
