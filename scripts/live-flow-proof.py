#!/usr/bin/env python3
"""Runs the plan's full flow against the *real* window, through semantic requests only.

    ./scripts/fixtures /tmp/tessel-proof                       # once (image, splats, tone, scene)
    ./scripts/cargo build -p tessel-editor --locked
    python3 scripts/live-flow-proof.py /tmp/tessel-proof [--work DIR] [--report FILE] [--uhd]

It launches `target/debug/tessel-editor` itself (and kills it for the crash step), then:

  create project -> import (dialog) mp4 + png + ply + wav -> timeline with splat scene, text,
  video, image and audio -> undo/redo (header buttons) -> save as (dialog) -> package (dialog)
  -> external zip check -> originals made unavailable -> restore elsewhere -> import a sequence
  from that project into one with colliding IDs (dialog) -> export 1080p, vertical, square
  (dialog, report, job) -> cancel a running export -> autosave -> SIGKILL -> recover (dialog).

Nothing clicks coordinates: `flow.*` opens a dialog, `dialog.field.*` fills it, `dialog.verify`
refreshes its report, `dialog.submit` runs the operation, `jobs` reports the background jobs.
`--uhd` adds a real 3840x2160 export (slow on SwiftShader). Exit status 1 on any failed check.
"""
import json
import os
import shutil
import signal
import socket
import subprocess
import sys
import time
import zipfile

import tonecheck

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SOCKET = os.path.expanduser("~/.cache/tessel-editor/ui.sock")
# TESSEL_EDITOR_BIN lets a run use a copy of the binary: the shared `target/` is garbage-collected by the cargo wrapper when disk is short.
BINARY = os.environ.get("TESSEL_EDITOR_BIN", os.path.join(ROOT, "target/debug/tessel-editor"))
FAILED = []


def send(request, timeout=90):
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
    return json.loads(data)


def op(request, timeout=90):
    return send({"version": 1, "command": "op", "request": request}, timeout)


def get(target):
    return send({"version": 1, "command": "get", "target": target})


def invoke(target, action=None):
    request = {"version": 1, "command": "invoke", "target": target}
    if action:
        request["action"] = action
    return send(request)


def setf(target, field, value):
    return send({"version": 1, "command": "set", "target": target, "field": field, "value": value})


def check(name, condition, detail=""):
    print(("PASS " if condition else "FAIL ") + name + ("" if condition else f"  [{detail}]"))
    if not condition:
        FAILED.append(name)
    return condition


LAUNCHED = []


def launch():
    process = subprocess.Popen([BINARY], stdout=open("/tmp/tessel-flow-editor.log", "w"), stderr=subprocess.STDOUT)
    for _ in range(150):
        if os.path.exists(SOCKET):
            try:
                send({"version": 1, "command": "status"})
                break
            except OSError:
                pass
        time.sleep(0.2)
    invoke("window.activate")
    # The crash step must kill exactly the editor this script started, never another instance that
    # happens to run `target/debug/tessel-editor` on the machine (a pgrep by name did that once).
    LAUNCHED.append(process)
    return process


def jobs():
    return get("jobs")["data"]["state"]


def wait_applied(count, timeout=240):
    """Waits until `count` jobs were applied to the project and none is still running."""
    end = time.time() + timeout
    while time.time() < end:
        state = jobs()
        if len(state["recent"]) >= count and not state["active"]:
            return state
        time.sleep(0.3)
    raise TimeoutError(json.dumps(jobs())[:600])


def fill(fields):
    for name, value in fields.items():
        reply = setf(f"dialog.field.{name}", "value", value)
        if not reply["ok"]:
            return reply
    return {"ok": True}


def ffprobe(path):
    out = subprocess.run(
        ["ffprobe", "-v", "error", "-show_streams", "-show_format", "-of", "json", path],
        capture_output=True, text=True, check=True,
    ).stdout
    return json.loads(out)


def viewport_state():
    # macOS does not paint a covered window, and the monitor's on-screen bookkeeping runs in
    # `render`: bring the window forward before every observation (docs/ui-automation.md).
    invoke("window.activate")
    return get("monitor.viewport")["data"]["state"]


def settle(predicate, timeout=10):
    end = time.time() + timeout
    while time.time() < end:
        value = predicate()
        if value:
            return value
        time.sleep(0.1)
    return None


def alive(pid):
    return subprocess.run(["ps", "-p", str(pid)], capture_output=True).returncode == 0


def verified(timeout=60):
    """`dialog.verify` starts a preflight/verification *job* (nothing blocks the window); the dialog's
    report fills in when the job finishes. Returns `(report, error)` once it has."""
    def finished():
        state = get("dialog")["data"]["state"]
        if state.get("verifying"):
            return None
        return (state.get("report"), state.get("error"))
    return settle(finished, timeout) or (None, "the dialog report never arrived")


def gates_and_playback(work, metrics):
    """WebView overlay/focus/resize gates and real playback, all through semantic requests."""
    shot = os.path.join(work, "gate.png")

    def draws():
        # Right after opening a project its media is decoded in the background; until then the answer
        # is the typed `media_preparing`, which is retried rather than treated as a failure.
        last = []

        def attempt():
            reply = op({"op": "render.frame", "out": shot, "probes": [[320, 180]]}, timeout=60)
            last[:] = [reply]
            if reply["ok"]:
                return reply["data"]["frame"]["hash"]
            # `cancelled`: the monitor asked for a newer frame meanwhile (latest request wins by design).
            return None if reply.get("error", {}).get("code") in ("media_preparing", "cancelled") else "error"
        value = settle(attempt, 60)
        if value == "error":
            print("  render.frame answered:", json.dumps(last)[:300], flush=True)
            return None
        return value

    check("the monitor draws before any dialog", draws())
    check("the viewport is on screen", settle(lambda: viewport_state()["on_screen"]))
    invoke("flow.export")
    check("an open dialog hides the native WebView (it would paint over the dialog)",
          settle(lambda: viewport_state()["on_screen"] is False), str(viewport_state()["on_screen"]))
    invoke("dialog.cancel")
    check("closing the dialog brings the WebView back", settle(lambda: viewport_state()["on_screen"]), str(viewport_state()["on_screen"]))
    check("the monitor still draws after the dialog closed", draws())
    invoke("flow.jobs")
    check("the job list also hides the WebView", settle(lambda: viewport_state()["on_screen"] is False))
    invoke("flow.export")  # replaces the jobs dialog on top
    invoke("dialog.cancel")
    send({"version": 1, "command": "set", "target": "window", "field": "size", "value": "1100x720"})
    time.sleep(0.8)
    check("the monitor draws after a window resize", draws())
    check("the viewport stays ready after the resize", viewport_state()["phase"] == "ready", str(viewport_state()["phase"]))
    send({"version": 1, "command": "set", "target": "window", "field": "size", "value": "1440x900"})

    # ---- playback: the device's sample clock drives picture and sound ----
    op({"op": "seek", "frame": 0})
    check("the play button is a live action", get("monitor.play")["data"]["enabled"] is True)
    stats_before = viewport_state()["session"]["stats"]
    started = op({"op": "playback.start"})
    check("playback.start is accepted", started["ok"], str(started))
    playing = settle(lambda: (lambda r: r if r.get("state") == "playing" and r.get("clock") != "starting" else None)(op({"op": "playback.status"})["data"]), 15)
    check("playback reaches `playing` once the mix is ready and the device is open", playing is not None)
    if not playing:
        return
    check("the play node says Pause", get("monitor.play")["data"]["label"] == "Pause")
    device_clock = playing["clock"] == "audio_device"
    if device_clock:
        audio = playing["audio"]
        check("the output is a real device through the host audio API", bool(audio["output"]["name"]) and audio["output"]["host"], str(audio["output"]))
        check("the device runs at the mix's own rate (no resampling)", audio["output"]["sample_rate"] == 48000, str(audio["output"]))
        check("the host reports a latency estimate", audio["latency_ms"] >= 0, str(audio))
    else:
        check("without a device the failure is explicit, not silent", playing["audio"].get("error", {}).get("code") == "audio_output_unavailable", str(playing["audio"]))
    time.sleep(1.0)
    mid = op({"op": "playback.status"})["data"]
    if device_clock:
        from_device = mid["audio"]["position_frames"] / 48000 * mid["fps"]
        check("the playhead follows the sample clock", abs(mid["frame"] - from_device) <= 2, f"frame {mid['frame']} device {from_device:.1f}")
    else:
        expected = mid["elapsed_ms"] / 1000 * mid["fps"]
        check("the playhead follows the wall clock", abs(mid["frame"] - expected) <= 3, f"frame {mid['frame']} expected {expected:.1f}")
    check("the monitor keeps up (lag within 2 frames)", mid["lag_frames"] <= 2, str(mid))
    stats_mid = viewport_state()["session"]["stats"]
    check("frames were really requested from the runtime while playing", stats_mid["requests"] - stats_before["requests"] >= 10, f"{stats_before} -> {stats_mid}")

    # pause holds, seek moves picture and sound, resume continues from the seek target
    paused = op({"op": "playback.pause"})
    check("pause is accepted", paused["ok"] and paused["data"]["report"]["state"] == "paused", str(paused)[:200])
    time.sleep(0.4)
    held = op({"op": "playback.status"})["data"]["frame"]
    time.sleep(0.6)
    check("a paused playhead does not move", op({"op": "playback.status"})["data"]["frame"] == held)
    sought = op({"op": "playback.seek", "frame": 36})
    sought_head = op({"op": "status"})["data"]["playhead"]["frame"]
    check("seek moves the playhead while paused", sought["ok"] and sought_head == 36, f"playhead {sought_head}; {str(sought)[:200]}")
    check("a seek outside the sequence is refused", not op({"op": "playback.seek", "frame": 4800})["ok"])
    op({"op": "playback.resume"})
    resumed = settle(lambda: (lambda r: r if r["state"] == "playing" and r["frame"] >= 36 else None)(op({"op": "playback.status"})["data"]), 5)
    check("resume continues from the seek target", resumed is not None and resumed["frame"] >= 36, str(resumed))

    finished = settle(lambda: (lambda r: r if r.get("state") == "stopped" else None)(op({"op": "playback.status"})["data"]), 15)
    check("playback stops at the end by itself", finished is not None)
    if finished:
        last = finished["last"]
        audio = last["audio"]
        if device_clock:
            check("the sample clock reported the end", audio.get("finished") is True, str(audio))
            # `device_underruns` is the host's own report of a missed buffer; `late_callbacks` counts callbacks
            # that arrived > 1.75 periods after the previous one (scheduling jitter, which this very run can
            # cause: the machine was compiling in parallel). Both are recorded; only the host's report fails.
            check("the audio host reported no buffer underrun", audio["device_underruns"] == 0, str(audio))
            check("late callbacks stay a small fraction of the run", audio["late_callbacks"] <= max(1, audio["callbacks"] // 50), str(audio))
        metrics["playback"] = {"clock": last["clock"], "skipped_frames": last["skipped_frames"], "max_gap_frames": last["max_gap_frames"], "ticks": last["ticks"],
                               "device": audio.get("output"), "latency_ms": audio.get("latency_ms"), "underruns": audio.get("underruns"),
                               "late_callbacks": audio.get("late_callbacks"), "device_underruns": audio.get("device_underruns"), "callbacks": audio.get("callbacks"),
                               "buffer_frames": audio.get("buffer_frames"), "max_callback_gap_ms": audio.get("max_callback_gap_ms"),
                               "heard": False}
    check("the playhead ended at the sequence end", op({"op": "status"})["data"]["playhead"]["frame"] == 48)
    op({"op": "playback.start", "from": 0})
    settle(lambda: op({"op": "playback.status"})["data"].get("state") == "playing", 15)
    time.sleep(0.5)
    stopped = op({"op": "playback.stop"})
    check("stop reports the final state", stopped["ok"] and stopped["data"]["report"]["state"] == "stopped", str(stopped)[:200])
    check("after stop nothing is playing", op({"op": "playback.status"})["data"]["state"] == "stopped")


def main():
    args = sys.argv[1:]
    if not args:
        print(__doc__)
        return 2
    fixtures = args[0]
    work = args[args.index("--work") + 1] if "--work" in args else "/tmp/tessel-flow-proof"
    report_path = args[args.index("--report") + 1] if "--report" in args else None
    shutil.rmtree(work, ignore_errors=True)
    os.makedirs(work)
    originals = os.path.join(work, "originals")
    os.makedirs(originals)
    for name in ("quadrants.png", "cloud.ply", "cloud.splat", "cloud.spz", "tone-440.wav"):
        shutil.copy(os.path.join(fixtures, name), originals)
    subprocess.run(
        ["ffmpeg", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=duration=2:size=320x180:rate=24",
         "-f", "lavfi", "-i", "sine=frequency=330:duration=2", "-c:v", "libx264", "-pix_fmt", "yuv420p",
         "-c:a", "aac", "-shortest", os.path.join(originals, "clip.mp4")], check=True)
    metrics = {}
    process = launch()
    try:
        flow(work, originals, metrics, fixtures, "--uhd" in args)
    finally:
        process.kill()
        if os.path.exists(SOCKET):
            pass
    if report_path:
        with open(report_path, "w") as handle:
            json.dump(metrics, handle, indent=2)
    print(json.dumps(metrics, indent=2))
    print(f"{'ALL' if not FAILED else 'SOME'} checks done; failed: {FAILED}")
    return 1 if FAILED else 0


def flow(work, originals, metrics, fixtures, uhd):
    project_dir = os.path.join(work, "project")
    created = op({"op": "project.create", "name": "Flow proof", "dir": project_dir, "rate": 24, "seconds": 2,
                  "width": 640, "height": 360})
    check("project created with a workspace", created["ok"], str(created))

    # ---- import through the dialog (header button -> node:14 is the same action as flow.import) ----
    check("header Importar opens the dialog", invoke("node:14")["ok"])
    dialog = get("dialog")["data"]
    check("the import dialog exposes its fields and buttons",
          {"dialog.field.paths", "dialog.field.mode", "dialog.submit", "dialog.cancel"} <= set(dialog["children"]), str(dialog))
    check("a script op still works while a dialog is open", op({"op": "media.status"})["ok"])
    empty = invoke("dialog.submit")
    check("submitting an empty form is refused in words", not empty["ok"] and "Arquivos" in empty["error"]["message"], str(empty))
    check("a bad option is refused", not setf("dialog.field.mode", "value", "bogus")["ok"])
    paths = ";".join(os.path.join(originals, name) for name in ("clip.mp4", "quadrants.png", "cloud.ply", "tone-440.wav"))
    check("fields fill", fill({"paths": paths, "mode": "copy"})["ok"])
    started = time.time()
    submitted = invoke("dialog.submit")
    check("import runs as a background job", submitted["ok"] and submitted["data"].get("job"), str(submitted))
    wait_applied(1)
    metrics["import_seconds"] = round(time.time() - started, 2)
    status = op({"op": "media.status"})["data"]
    kinds = {asset["id"]: asset["kind"] for asset in status["assets"]}
    check("four assets landed in the library", kinds == {"clip": "video", "quadrants": "still", "cloud": "splat", "tone-440": "audio"}, str(kinds))
    check("every imported asset is managed and present", all(a["storage"] == "managed" and a["state"] == "ok" for a in status["assets"]))
    check("the dialog closed after a successful submit", get("dialog")["ok"] is False)

    # ---- timeline: splat scene + media + text + audio ----
    scene = os.path.join(work, "scene.tessel-scene.json")
    with open(scene, "w") as handle:
        json.dump({"format": "tessel-scene", "version": 1, "background": [0.04, 0.05, 0.1, 1.0],
                   "objects": [{"type": "splat", "asset": "cloud", "scale": 1.0, "spin": [0, 20, 0]}]}, handle)
    steps = [
        {"op": "asset.add", "id": "scn", "kind": "scene", "name": "Splat scene", "path": scene},
        {"op": "asset.add", "id": "label", "kind": "text", "name": "Label", "text": {"color": "#ffffff", "content": "TESSEL", "size": 14}},
        {"op": "track.add", "id": "scene-track", "kind": "video", "name": "Scene"},
        {"op": "track.add", "id": "media-track", "kind": "video", "name": "Media"},
        {"op": "track.add", "id": "title-track", "kind": "video", "name": "Titles"},
        {"op": "track.add", "id": "music", "kind": "audio", "name": "Music"},
        {"op": "clip.add", "id": "c-scene", "asset": "scn", "track": "scene-track", "start": 0, "duration": 48,
         "camera": {"fov": 50, "orbit": 45, "position": [0, 1.2, 5], "target": [0, 0, 0]}},
        {"op": "clip.add", "id": "c-img", "asset": "quadrants", "track": "media-track", "start": 0, "duration": 24, "properties": {"zoom": 50}},
        {"op": "clip.add", "id": "c-vid", "asset": "clip", "track": "media-track", "start": 24, "duration": 24, "properties": {"zoom": 50}},
        {"op": "clip.add", "id": "c-title", "asset": "label", "track": "title-track", "start": 0, "duration": 48},
        {"op": "clip.add", "id": "c-tone", "asset": "tone-440", "track": "music", "start": 0, "duration": 48},
    ]
    for step in steps:
        reply = op(step)
        if not check(f"timeline: {step['op']} {step.get('id', '')}", reply["ok"], str(reply)):
            return
    before = op({"op": "plan", "frame": 30})["data"]
    check("the plan at frame 30 has scene, video and title layers", len(before["layers"]) >= 3, str(before)[:300])

    # ---- undo / redo through the header buttons ----
    history = op({"op": "history.status"})["data"]["history"]
    check("history knows the last edit", history.get("undo_label"), str(history))
    check("header Desfazer undoes it", invoke("node:9")["ok"])
    check("undo removed the last clip from the timeline", not get("clip:c-tone")["ok"], str(get("clip:c-tone"))[:200])
    check("header Refazer redoes it", invoke("node:11")["ok"])
    check("redo brought the clip back and restored history", get("clip:c-tone")["ok"] and op({"op": "history.status"})["data"]["history"]["undo_label"] == history["undo_label"])

    # ---- save as (dialog) ----
    document = os.path.join(work, "flow.tessel.json")
    check("Salvar on a never-saved project asks where", invoke("node:19")["ok"] and get("dialog")["data"]["state"]["flow"] == "save_as")
    check("save-as fills", fill({"path": document})["ok"])
    saved = invoke("dialog.submit")
    check("save as writes the document", saved["ok"] and os.path.exists(document), str(saved))
    persistence = op({"op": "status"})["data"]["persistence"]
    check("the project knows its document and is clean", persistence["document"] == document and persistence["dirty"] is False, str(persistence))

    # ---- package (dialog), checked outside the editor ----
    check("header Empacotar opens the dialog", invoke("node:20")["ok"])
    package = os.path.join(work, "flow.tesselpack")
    fill({"path": package, "format": "archive"})
    verify = invoke("dialog.verify")
    package_report, package_error = verified()
    check("package preflight reports without writing", verify["ok"] and package_report and not package_error and not os.path.exists(package), str((verify, package_report, package_error)))
    check("package submit starts a job", invoke("dialog.submit")["ok"])
    wait_applied(2)
    check("the package exists", os.path.exists(package))
    with zipfile.ZipFile(package) as archive:
        bad = archive.testzip()
        names = archive.namelist()
    check("python zipfile reads the package and every CRC passes", bad is None and any(n.endswith("manifest.json") for n in names), f"{bad} {names[:6]}")
    check("an external unzip agrees", subprocess.run(["unzip", "-tq", package], capture_output=True).returncode == 0)
    metrics["package_bytes"] = os.path.getsize(package)

    # ---- originals disappear; restore elsewhere ----
    moved = originals + "-gone"
    os.rename(originals, moved)
    restore_dir = os.path.join(work, "restored")
    # In the window `project.restore` is a job: the reply carries the job id and the project is swapped in
    # when the editor applies the result on its tick.
    jobs_before = len(jobs()["recent"])
    restored = op({"op": "project.restore", "path": package, "folder": restore_dir, "open": True})
    check("restore is accepted as a job and does not block the window", restored["ok"] and restored["data"].get("state") in ("queued", "running"), str(restored)[:300])
    wait_applied(jobs_before + 1)
    check("restore opens the project from the package alone", op({"op": "status"})["data"]["persistence"]["document"] == os.path.join(restore_dir, "project.tessel.json"), str(op({"op": "status"})["data"]["persistence"]))
    status = op({"op": "media.status"})["data"]
    check("every asset resolves inside the restored folder",
          status["missing"] == 0 and all(a["path"].startswith(restore_dir) for a in status["assets"] if a.get("path")), str(status)[:400])
    check("the restored project still plans the same layers", len(op({"op": "plan", "frame": 30})["data"]["layers"]) == len(before["layers"]))
    os.rename(moved, originals)

    # ---- import a sequence into a project with colliding IDs ----
    other_dir = os.path.join(work, "other")
    op({"op": "project.create", "name": "Other", "dir": other_dir, "rate": 24, "seconds": 2, "width": 640, "height": 360})
    op({"op": "asset.add", "id": "clip", "kind": "text", "name": "Colliding", "text": {"color": "#ff0000", "content": "X", "size": 10}})
    op({"op": "track.add", "id": "media-track", "kind": "video", "name": "Mine"})
    check("Importar de outro projeto opens", invoke("flow.import_project")["ok"])
    fill({"source": os.path.join(restore_dir, "project.tessel.json")})
    check("project import submit starts a job", invoke("dialog.submit")["ok"])
    # The job is applied on the editor's tick: wait for its effect, not for a job count (preflight and
    # prepare jobs share the list).
    def merged_assets():
        assets = op({"op": "media.status"})["data"]["assets"]
        return assets if any(a["id"] != "clip" and a["name"] == "clip.mp4" for a in assets) else None
    merged = settle(merged_assets, 60) or op({"op": "media.status"})["data"]["assets"]
    state = settle(lambda: (lambda s: s if not s["active"] else None)(jobs()), 60) or jobs()
    imported_job = next((job for job in state["recent"] if job.get("kind") == "project-import"), {})
    mine = get("media:clip")
    check("the project's own `clip` asset was not overwritten", mine["ok"] and mine["data"]["label"] == "Colliding", str(mine)[:200])
    check("the imported `clip` got a fresh ID next to it", any(a["id"] != "clip" and a["name"] == "clip.mp4" for a in merged), str([(a["id"], a["name"]) for a in merged]))
    check("the project-import job succeeded", imported_job.get("state") == "succeeded", str(imported_job)[:300])

    # ---- go back to the restored project for the exports ----
    check("reopen the saved project", op({"op": "project.open", "path": os.path.join(restore_dir, "project.tessel.json")})["ok"])
    gates_and_playback(work, metrics)
    out = os.path.join(work, "out")
    os.makedirs(out)
    base = len(jobs()["recent"])
    exports = [("fullhd", 1920, 1080), ("vertical", 1080, 1920), ("square", 1080, 1080)]
    if uhd:
        exports.append(("uhd4k", 3840, 2160))
    for index, (preset, width, height) in enumerate(exports, start=1):
        check(f"Exportar opens for {preset}", invoke("node:16")["ok"])
        destination = os.path.join(out, f"{preset}.mp4")
        fill({"preset": preset, "destination": destination})
        verify = invoke("dialog.verify")
        report, report_error = verified()
        check(f"{preset}: preflight shows a ready report", verify["ok"] and report and "Pronto" in report, str((report, report_error)))
        started = time.time()
        check(f"{preset}: export starts", invoke("dialog.submit")["ok"])
        wait_applied(base + index, timeout=900)
        metrics[f"export_{preset}_seconds"] = round(time.time() - started, 1)
        probe = ffprobe(destination)
        video = next(s for s in probe["streams"] if s["codec_type"] == "video")
        audio = [s for s in probe["streams"] if s["codec_type"] == "audio"]
        check(f"{preset}: {width}x{height} at the real size", (video["width"], video["height"]) == (width, height), str((video["width"], video["height"])))
        check(f"{preset}: 48 frames at 24 fps with AAC audio", video["nb_frames"] == "48" and video["r_frame_rate"] == "24/1" and audio and audio[0]["codec_name"] == "aac", str(probe)[:300])
        check(f"{preset}: a file the job reported as succeeded", jobs()["recent"][0]["state"] == "succeeded")
        # The 330 Hz sound lives only in the video clip (frames 24-48 = 1-2 s), the 440 Hz tone is on
        # the audio track for the whole sequence: the export must carry both, each where it belongs.
        first_330 = tonecheck.tone_level(destination, 0.1, 0.8, 330)
        second_330 = tonecheck.tone_level(destination, 1.1, 0.8, 330)
        first_440 = tonecheck.tone_level(destination, 0.1, 0.8, 440)
        second_440 = tonecheck.tone_level(destination, 1.1, 0.8, 440)
        check(f"{preset}: the video clip's own 330 Hz sound is in the export, only while the clip plays",
              second_330 > 0.02 and first_330 < second_330 / 20, f"first {first_330:.4f} second {second_330:.4f}")
        check(f"{preset}: the audio-track 440 Hz tone is still there for the whole sequence", first_440 > 0.02 and second_440 > 0.02, f"{first_440:.4f} {second_440:.4f}")
        metrics.setdefault("video_sound_levels", {})[preset] = {"330Hz_first_second": round(first_330, 4), "330Hz_second_second": round(second_330, 4),
                                                              "440Hz_first_second": round(first_440, 4), "440Hz_second_second": round(second_440, 4)}

    # ---- cancel a running export ----
    invoke("node:16")
    cancel_target = os.path.join(out, "cancel.mp4")
    fill({"preset": "uhd4k", "destination": cancel_target})
    invoke("dialog.submit")
    running = None
    end = time.time() + 120
    while time.time() < end:
        active = jobs()["active"]
        exporting = [job for job in active if "export" in str(job.get("kind", "")) and job["done"] > 0]
        if exporting:
            running = exporting[0]
            break
        time.sleep(0.3)
    check("a 4K export is running with progress", running is not None, str(jobs())[:300])
    if running:
        cancelled = op({"op": "job.cancel", "job": running["id"]})
        check("cancel is acknowledged", cancelled["ok"], str(cancelled))
        # Preflight jobs share the list now, so the cancelled export is found by its own id.
        def finished_cancel():
            state = jobs()
            # `recent` holds the applied-job events (`job`), `active` the live ones (`id`).
            mine = [job for job in state["recent"] if job.get("job", job.get("id")) == running["id"]]
            return mine[0] if mine and not state["active"] else None
        ended = settle(finished_cancel, 120)
        check("the job ends cancelled", ended is not None and ended["state"] == "cancelled", str(ended)[:300])
        check("no partial file is left at the destination", not os.path.exists(cancel_target) and not [f for f in os.listdir(out) if f.startswith(".") or f.endswith(".part")], str(os.listdir(out)))

    # ---- jobs dialog opens ----
    check("Tarefas opens the job list", invoke("node:21")["ok"])

    # ---- autosave, crash, recover ----
    document = os.path.join(restore_dir, "project.tessel.json")
    op({"op": "track.add", "id": "late", "kind": "video", "name": "Late edit"})
    check("the edit made the project dirty", op({"op": "history.status"})["data"]["history"]["undo_label"] is not None)
    time.sleep(24)  # one autosave interval plus a tick
    listing = op({"op": "project.recover", "path": document, "apply": False})
    check("autosave wrote a recovery revision on its own", listing["ok"] and len(listing["data"].get("revisions", listing["data"].get("recoveries", []))) >= 1, str(listing)[:300])
    crashed = LAUNCHED[-1]
    os.kill(crashed.pid, signal.SIGKILL)
    crashed.wait(timeout=10)
    time.sleep(1)
    relaunched = launch()
    try:
        check("the relaunched editor does not have the late track yet", not get("track:late")["ok"])
        invoke("flow.recover")
        fill({"path": document})
        invoke("dialog.verify")
        report, _ = verified()
        check("recover dialog lists the autosave", report and "revis" in report, str(report))
        check("recover applies it", invoke("dialog.submit")["ok"])
        check("the late edit came back after the crash", get("track:late")["ok"])
        persistence = op({"op": "status"})["data"]["persistence"]
        check("a recovered project is dirty: it was never saved", persistence["dirty"] is True, str(persistence))
        check("and still editable", op({"op": "track.add", "id": "after-crash", "kind": "audio", "name": "After"})["ok"])
    finally:
        relaunched.kill()


if __name__ == "__main__":
    sys.exit(main())
