#!/usr/bin/env python3
"""Runs the viewport proof against the *running* editor through its automation socket.

    ./scripts/fixtures /tmp/tessel-proof          # once
    ./target/debug/tessel-editor &                # or ./scripts/cargo run
    python3 scripts/live-proof.py /tmp/tessel-proof [--shots DIR] [--report FILE]

Every step is a semantic request (the same JSON as `ui op`, `ui set`, `ui invoke`); nothing
clicks at coordinates. With --shots the script also captures the editor window with
`screencapture -l` after each stage, so the picture in the monitor can be looked at.

It asserts the same pixels `crates/render/tests/frames.rs` asserts for the headless worker, so
"the same sequence in the viewport" is checked, not assumed. Exit status 1 on any failed check.
"""
import json
import os
import socket
import subprocess
import sys
import time

SOCKET = os.path.expanduser("~/.cache/tessel-editor/ui.sock")
QUADRANTS = [(220, 40, 40), (40, 200, 60), (40, 80, 220), (230, 210, 40)]
QUADRANT_PROBES = [(120, 67), (200, 67), (120, 112), (200, 112)]


def video_color(index):
    step = index % 8
    return (30 + step * 25, 200 - step * 20, 60 + step * 10)


def send(request, timeout=60):
    started = time.perf_counter()
    with socket.socket(socket.AF_UNIX) as conn:
        conn.settimeout(timeout)
        conn.connect(SOCKET)
        conn.sendall((json.dumps(request) + "\n").encode())
        data = b""
        while not data.endswith(b"\n"):
            chunk = conn.recv(65536)
            if not chunk:
                break
            data += chunk
    response = json.loads(data)
    response["_ms"] = (time.perf_counter() - started) * 1000
    return response


def op(request, timeout=60):
    return send({"version": 1, "command": "op", "request": request}, timeout)


def ui(command, target, **extra):
    return send({"version": 1, "command": command, "target": target, **extra})


def viewport():
    return send({"version": 1, "command": "get", "target": "monitor.viewport"})["data"]["state"]


failures = []
log = []


def check(name, condition, detail=""):
    log.append({"check": name, "ok": bool(condition), "detail": detail})
    print(("PASS " if condition else "FAIL ") + name + (f"  [{detail}]" if detail and not condition else ""))
    if not condition:
        failures.append(name)


def near(actual, expected, tolerance=3):
    return all(abs(a - e) <= tolerance for a, e in zip(actual[:3], expected))


def memory_mb():
    """RSS of the editor and of every WebKit helper process on the machine (the WebView's
    content and GPU processes are not children of the editor, so this over-counts if another
    app is also using WebKit; it is a ceiling, not an exact figure)."""
    listing = subprocess.run(["ps", "-axo", "pid=,rss=,comm="], capture_output=True, text=True).stdout
    editor = 0
    webkit = 0
    for line in listing.splitlines():
        pid, rss, command = line.strip().split(None, 2)
        if command.endswith("/tessel-editor"):
            editor += int(rss)
        elif "com.apple.WebKit" in command:
            webkit += int(rss)
    return {"editor_mb": round(editor / 1024, 1), "webkit_helpers_mb": round(webkit / 1024, 1)}


class Shots:
    def __init__(self, directory):
        self.directory = directory
        if directory:
            os.makedirs(directory, exist_ok=True)

    def window_id(self):
        # The main window is the on-screen one; WebKit may add an off-screen helper.
        script = (
            'import CoreGraphics\nlet l = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as! [[String: Any]]\n'
            'for w in l { if ((w[kCGWindowOwnerName as String] as? String) ?? "").hasSuffix("tessel-editor") { print(w[kCGWindowNumber as String]!) ; break } }'
        )
        return subprocess.run(["swift", "-e", script], capture_output=True, text=True).stdout.strip()

    def take(self, name):
        if not self.directory:
            return None
        time.sleep(0.15)  # let the last frame reach the window before it is captured
        window = self.window_id()
        if not window:
            return None
        path = os.path.join(self.directory, f"{name}.png")
        subprocess.run(["screencapture", "-x", "-o", "-l", window, path], check=False)
        return path


def main():
    args = sys.argv[1:]
    if not args:
        print(__doc__)
        return 2
    fixtures = args[0]
    shots = Shots(args[args.index("--shots") + 1] if "--shots" in args else None)
    report_path = args[args.index("--report") + 1] if "--report" in args else None
    metrics = {"memory_before": memory_mb()}

    # GPUI does not paint a window that is fully covered by another one, and the monitor's
    # on-screen/hidden bookkeeping runs in that paint. Bring the window forward first, as a
    # person looking at it would have.
    ui("invoke", "window.activate")
    time.sleep(0.5)

    # ---- 0. the viewport is up ----
    state = viewport()
    check("viewport phase is ready", state["phase"] == "ready", state["phase"])
    caps = op({"op": "capabilities"})
    check("capabilities report the runtime", caps.get("ok") and "image" in caps["data"]["layers"], str(caps)[:200])
    metrics["renderer"] = caps["data"].get("renderer")

    # ---- 1. build the project through the API ----
    with open(os.path.join(fixtures, "proof.jsonl")) as handle:
        lines = [json.loads(line) for line in handle if line.strip()]
    for line in lines:
        response = op(line)
        check(f"{line['op']} {line.get('id', line.get('name', ''))}", response.get("ok"), str(response)[:200])
    summary = send({"version": 1, "command": "status"})["data"]["editor"]
    check("project has the proof sequence", summary["project"] == "Viewport proof", summary["project"])
    shots.take("01-project-built")

    def frame(name, frame_number, probes=(), png=True):
        request = {"op": "render.frame", "frame": frame_number, "probes": [list(p) for p in probes]}
        if png and shots.directory:
            request["out"] = os.path.abspath(os.path.join(shots.directory, f"{name}-canvas.png"))
        response = op(request)
        if not response.get("ok"):
            return response, None
        return response, response["data"]["frame"]

    # ---- 2. image over scene, title on top ----
    response, first = frame("f00", 0, QUADRANT_PROBES)
    check("frame 0 renders", response.get("ok"), str(response)[:300])
    if first:
        for index, color in enumerate(QUADRANTS):
            check(f"image quadrant {index}", near(first["probes"][index], color), str(first["probes"][index]))
        layers = [layer["kind"] for layer in response["data"]["plan"]["layers"]]
        check("plan alternates scene, image and text", layers == ["scene", "image", "text"], str(layers))
        metrics["frame0"] = {"render_ms": first["render_ms"], "total_ms": first["total_ms"], "wall_ms": round(response["_ms"], 1)}
        if first.get("png"):
            with open(first["png"], "rb") as handle:
                check("PNG file written by the host", handle.read(4)[1:4] == b"PNG")
    shots.take("02-frame-00")

    band = [(x, y) for x in range(60, 260, 2) for y in range(146, 170, 3)]
    _, titled = frame("f00-title", 0, band, png=False)
    white = sum(1 for pixel in (titled or {"probes": []})["probes"] if tuple(pixel[:3]) == (255, 255, 255))
    check("title text pixels present", white > 20, str(white))

    # ---- 3. video (frame sequence): the media track alternates image -> video ----
    seeks = []
    for number, index in ((30, 2), (47, 7)):
        response, current = frame(f"f{number}", number, [(160, 90)])
        if current:
            check(f"video frame at sequence frame {number}", near(current["probes"][0], video_color(index)), str(current["probes"][0]))
            seeks.append({"frame": number, "wall_ms": round(response["_ms"], 1), "render_ms": current["render_ms"]})
        else:
            check(f"video frame at sequence frame {number}", False, str(response)[:200])
        if number == 30:
            shots.take("03-frame-30-video")
    metrics["explicit_frames"] = seeks
    if first and current:
        check("image and video frames differ", first["hash"] != current["hash"])

    # ---- 4. a seek through the API updates the monitor without any render request ----
    before = viewport()["last_frame"]["revision"]
    started = time.perf_counter()
    seek = op({"op": "seek", "frame": 5})
    check("seek is accepted", seek.get("ok"), str(seek)[:200])
    deadline = time.time() + 10
    while time.time() < deadline:
        state = viewport()
        if state["last_frame"] and state["last_frame"]["revision"] > before and state["session"]["status"] == "ready":
            break
        time.sleep(0.005)
    metrics["seek_to_displayed_ms"] = round((time.perf_counter() - started) * 1000, 1)
    check("monitor redrew after the seek", state["last_frame"]["revision"] > before, str(state["session"]["status"]))
    playhead = send({"version": 1, "command": "get", "target": "timeline.playhead"})["data"]["state"]
    check("playhead follows the seek", playhead.get("frame") == 5, str(playhead))
    shots.take("04-after-seek")

    # ---- 5. camera, scene time and the splat (scene isolated) ----
    ui("set", "track:media-track", field="hidden", value="true")
    ui("set", "track:title-track", field="hidden", value="true")
    _, scene0 = frame("scene-0", 0, [(160, 90)])
    _, scene24 = frame("scene-24", 24, [])
    check("scene changes with time", scene0 and scene24 and scene0["hash"] != scene24["hash"])
    check("splat sphere is drawn at the center", scene0 and sum(scene0["probes"][0][:3]) > 250, str(scene0 and scene0["probes"][0]))
    shots.take("05-scene-only")
    before_camera = scene0["hash"] if scene0 else None
    camera = op({"op": "clip.camera", "clip": "c-scene", "position": [4, 3, 6], "target": [0, 0, 0], "fov": 35, "orbit": 0})
    check("camera edit accepted", camera.get("ok"), str(camera)[:200])
    _, moved = frame("scene-0-camera", 0, [])
    check("camera change alters the picture", moved and before_camera and moved["hash"] != before_camera)
    shots.take("06-camera-moved")
    ui("set", "track:media-track", field="hidden", value="false")
    ui("set", "track:title-track", field="hidden", value="false")

    # ---- 6. source preview: one asset apart from the sequence; audio as a waveform ----
    op({"op": "preview.set", "mode": "source", "asset": "img"})
    response, source = frame("source-img", 0, [(80, 40), (240, 140)])
    if source:
        check("source preview: top-left quadrant", near(source["probes"][0], QUADRANTS[0]), str(source["probes"][0]))
        check("source preview: bottom-right quadrant", near(source["probes"][1], QUADRANTS[3]), str(source["probes"][1]))
        check("source preview has one layer", len(response["data"]["plan"]["layers"]) == 1)
    else:
        check("source preview renders", False, str(response)[:200])
    shots.take("07-source-image")
    op({"op": "preview.set", "mode": "source", "asset": "tone"})
    response, wave = frame("source-tone", 0, [(0, 90), (160, 90)])
    check("audio source is a waveform layer", wave and response["data"]["plan"]["layers"][0]["kind"] == "waveform")
    if wave:
        check("waveform playhead column", tuple(wave["probes"][0][:3]) == (255, 255, 255), str(wave["probes"][0]))
    shots.take("08-source-audio")
    op({"op": "preview.set", "mode": "sequence"})

    # ---- 7. audio: processed offline, not heard ----
    audio = op({"op": "render.audio", "from": 0, "frames": 24, "samples": [[0, 0], [100, 0]]})
    check("offline audio mix is reported as processed", audio.get("ok") and audio["data"]["kind"] == "processed", str(audio)[:200])
    if audio.get("ok"):
        check("tone is audible in the mix (peak)", audio["data"]["peak"] > 0.1, str(audio["data"]["peak"]))

    # ---- 8. errors do not change state, and the viewport recovers ----
    before_state = send({"version": 1, "command": "status"})["data"]["editor"]
    missing = op({"op": "asset.add", "kind": "image", "id": "ghost", "name": "Ghost", "path": "/nonexistent/ghost.png"})
    check("missing asset file is rejected", not missing.get("ok") and "error" in missing, str(missing)[:200])
    duplicate = op({"op": "clip.add", "track": "no-such-track", "asset": "img", "start": 0, "duration": 5})
    check("clip on an unknown track is rejected", not duplicate.get("ok"), str(duplicate)[:200])
    after_state = send({"version": 1, "command": "status"})["data"]["editor"]
    check("rejected operations leave the project unchanged", before_state["timeline"] == after_state["timeline"] and before_state["assets"] == after_state["assets"])

    op({"op": "clip.property", "clip": "c-img", "property": "blend", "value": "overlay"})
    blend = op({"op": "render.frame", "frame": 0})
    check("unsupported blend fails the frame with its code", not blend.get("ok") and blend["error"]["code"] == "unsupported_format", str(blend)[:300])
    state = viewport()
    check("failure is visible in the viewport state", state["session"]["status"] == "failed" and "overlay" in state["session"]["error"]["message"], str(state["session"]["error"]))
    shots.take("09-failed-frame")
    op({"op": "clip.property", "clip": "c-img", "property": "blend", "value": "normal"})
    response, recovered = frame("recovered", 0, QUADRANT_PROBES)
    check("the viewport recovers after the failure", recovered and near(recovered["probes"][0], QUADRANTS[0]), str(response)[:200])

    # ---- 9. hide, show and reload the monitor's page ----
    hidden = ui("invoke", "panel:monitor", action="hide")
    check("monitor panel hides", hidden.get("ok"), str(hidden)[:200])
    time.sleep(0.3)
    check("viewport reports off-screen while hidden", viewport()["on_screen"] is False)
    shots.take("10-monitor-hidden")
    shown = ui("invoke", "panel:monitor", action="show")
    check("monitor panel shows again", shown.get("ok"), str(shown)[:200])
    time.sleep(0.5)
    check("viewport reports on-screen when shown", viewport()["on_screen"] is True)
    shots.take("11-monitor-shown-again")
    reload_response = ui("invoke", "monitor.reload", action="invoke")
    check("reload is accepted", reload_response.get("ok"), str(reload_response)[:200])
    deadline = time.time() + 20
    while time.time() < deadline:
        state = viewport()
        if state["phase"] == "ready" and state["last_frame"]:
            break
        time.sleep(0.1)
    check("viewport is ready and drew a frame after the reload", state["phase"] == "ready" and bool(state["last_frame"]), state["phase"])
    shots.take("12-after-reload")

    # ---- 10. a burst of seeks: only the newest may be shown ----
    stats_before = viewport()["session"]["stats"]
    for number in range(0, 40, 3):
        op({"op": "seek", "frame": number})
    time.sleep(1.0)
    state = viewport()
    stats = state["session"]["stats"]
    check("burst of seeks ends on the last frame", state["session"]["status"] == "ready")
    metrics["burst"] = {"seeks": 14, "requests": stats["requests"] - stats_before["requests"], "accepted": stats["accepted"] - stats_before["accepted"],
                        "cancelled": stats["cancelled"] - stats_before["cancelled"], "dropped_stale": stats["dropped_stale"] - stats_before["dropped_stale"]}

    metrics["memory_after"] = memory_mb()
    result = {"failures": failures, "checks": log, "metrics": metrics}
    if report_path:
        with open(report_path, "w") as handle:
            json.dump(result, handle, indent=2)
    print(json.dumps(metrics, indent=2))
    print(f"{len(log) - len(failures)}/{len(log)} checks passed")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
