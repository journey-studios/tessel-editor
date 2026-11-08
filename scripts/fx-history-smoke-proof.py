#!/usr/bin/env python3
"""Integration smoke (BL082.20 x BL145.6): an effect gesture while the diagnostic history records,
in the REAL window of ONE build that carries both features.

    python3 scripts/directed-scene-proof/with-gpu-lock.py \\
        python3 scripts/fx-history-smoke-proof.py [--binary target/release/tessel-editor] [--json FILE] [--keep]

It reuses the helpers of `scene-fx-ui-proof.py` (own editor, own socket and HOME, the synthetic
corpus, `op`/`get`/`set`/`invoke` only; no pointer, no coordinates). It does NOT repeat the effect or
the history proofs: it checks that they do not disturb each other, in this order:

  * recording starts with a gesture-capable scene on the monitor;
  * a gesture drawn on the monitor (the candidate picture differs from the document's) neither stops
    nor pauses the recording, and the recording does not move the gesture;
  * freeze under the open gesture is one immutable view while the recording continues; a selection
    inside it is an inspection; the export of the frozen snapshot is a local `tessel.perf-history` v1
    file whose window, events and segments EQUAL the frozen view the window showed, and whose text
    carries no local path (the build's commit is reported, never guessed);
  * cancel, commit (one undo entry), undo and a scene switch under a gesture each leave the document's
    picture and value as the effect proof promises;
  * through all of it the history holds no `playback_started`, `request_failed` or
    `runtime_unavailable` event that the gesture could have invented, and recording is still on.

The report is one JSON object on the last line (`ok`, `failures`, `checks`); the exit status is 1 on a
failed check and 2 on a missing prerequisite. Limits: the history is recorded on a loaded machine, the
frame in flight across a scene switch is covered by the GPUI tests of `scene_lifecycle_tests.rs`, not
provoked here, and nothing in this script measures cost.
"""
import argparse
import importlib.util
import json
import os
import pathlib
import shutil
import signal
import sys
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("scene_fx_ui_proof", HERE / "scene-fx-ui-proof.py")
fx = importlib.util.module_from_spec(spec)
sys.modules["scene_fx_ui_proof"] = fx
spec.loader.exec_module(fx)
check, h = fx.check, fx.h

NODE = "perf.history"
FRAME = fx.FRAME
FORBIDDEN = ("playback_started", "request_failed", "runtime_unavailable")


def state(win):
    return (win.node(NODE) or {}).get("state") or {}


def view(win):
    return state(win).get("view") or {}


def kinds(events):
    return [event.get("kind") for event in events]


def run(args):
    binary = pathlib.Path(args.binary).resolve()
    for path in (binary, ROOT / "runtime" / "dist" / "runtime.js", fx.CORPUS / "bundle.json", fx.CORPUS / "bundle.swap.json"):
        if not path.exists():
            print(f"missing prerequisite: {path}")
            return 2
    if fx.free_gib() < fx.MIN_FREE_GIB:
        print(f"only {fx.free_gib():.1f} GiB free; needs {fx.MIN_FREE_GIB}")
        return 2
    lifecycle = fx.load_lifecycle()
    work = tempfile.mkdtemp(prefix="te-fxh-", dir="/tmp")
    lab, pid = None, None
    report = {"binary": h.binary_info(str(binary)), "git": h.git_state()}
    try:
        lab = lifecycle.Lab(str(binary), work, {"TESSEL_VIEWPORT_INSPECT": "1"})
        pid = lab.process.pid
        for signum in (signal.SIGTERM, signal.SIGINT):
            signal.signal(signum, lambda *_: (_ for _ in ()).throw(SystemExit(130)))
        lab.wait_ready()
        lab.invoke("window.activate")
        time.sleep(0.8)
        win = fx.Win(lab, lifecycle)
        ok, body = win.op({"op": "project.create", "name": "fxh", "sequence": "main", "rate": 24, "width": fx.WIDTH, "height": fx.HEIGHT,
                           "length": fx.EDIT_FRAMES, "dir": os.path.join(work, "ws", "fxh")})
        if not check("project.create", ok, body):
            return 1
        for scene, name in (("proof", "bundle.json"), ("yard", "bundle.swap.json")):
            ok, body = win.op({"op": "scene.attach", "id": scene, "path": str(fx.CORPUS / name)})
            if not check(f"scene.attach {scene}", ok, body):
                return 1
            ok, loaded = win.op({"op": "scene.resources.resolve", "scene": scene, "base": str(fx.CORPUS)})
            if not check(f"scene.resources.resolve {scene}", ok, loaded) or not lifecycle.commit_job(lab, loaded, scene):
                return 1
        check("set inspector.mode scene", win.set("inspector.mode", "value", "scene").get("ok"))
        ok, body = win.op({"op": "monitor.show_scene", "scene": "proof"})
        if not check("monitor.show_scene proof", ok, body):
            return 1
        lab.seek(FRAME)
        added = win.invoke("scene.fx", "add:vignette")
        check("a vignette to drag", added.get("ok") and added["data"].get("changed") is True, added)
        document_hash = win.repaint(FRAME)
        depth0 = win.undo_depth()

        # ---- recording starts; a gesture is drawn under it ----
        started = win.set(NODE, "recording", True)
        check("set perf.history recording true", started.get("ok") and state(win).get("recording") is True, started)
        time.sleep(1.2)
        ok, began = win.op({"op": "scene.fx.gesture.begin", "scene": "proof", "clip": "fx-1"})
        gesture = began.get("gesture") if ok else None
        check("scene.fx.gesture.begin while recording", ok and bool(gesture), began)
        ok, body = win.op({"op": "scene.fx.gesture.update", "gesture": gesture,
                           "ops": [{"op": "set_param", "clip": "fx-1", "param": "intensity", "value": 0.4}]})
        check("scene.fx.gesture.update while recording", ok, body)
        candidate_hash = win.repaint(FRAME)
        check("the monitor draws the candidate under the recording", bool(candidate_hash) and candidate_hash != document_hash, [candidate_hash, document_hash])
        check("the gesture did not stop the recording", state(win).get("recording") is True and state(win).get("has_data") is True, state(win))
        ok, document_list = win.op({"op": "scene.fx.list", "scene": "proof"})
        document_value = document_list["tracks"][0]["clips"][0]["params"]["intensity"] if ok else None
        check("the recording and the open gesture did not touch the document (value and history)", win.undo_depth() == depth0 and document_value == 0.7,
              [win.undo_depth(), depth0, document_value])

        # ---- freeze, select and export under the open gesture ----
        frozen = win.invoke(NODE, "freeze")
        check("freeze under an open gesture", frozen.get("ok") and state(win).get("frozen") is True, frozen)
        frozen_view = view(win)
        window = frozen_view.get("window") or {}
        ok, body = win.op({"op": "scene.fx.gesture.update", "gesture": gesture,
                           "ops": [{"op": "set_param", "clip": "fx-1", "param": "intensity", "value": 0.3}]})
        check("the drag continues while frozen", ok and win.repaint(FRAME) not in (None, document_hash), body)
        time.sleep(1.0)
        check("the frozen view did not move while the recording and the drag went on", view(win) == frozen_view and state(win).get("recording") is True,
              [view(win).get("window"), frozen_view.get("window")])
        end_ms = window.get("end_ms")
        selected = win.set(NODE, "selected_ms", (end_ms - 500.0) if isinstance(end_ms, (int, float)) else 0.0)
        check("a selection inside the frozen window is an inspection", selected.get("ok") and isinstance(state(win).get("selected_ms"), (int, float)), selected)
        target = os.path.join(work, "history-under-gesture.json")
        exported = win.set(NODE, "export_to", target)
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline and (state(win).get("export") or {}).get("state") == "writing":
            time.sleep(0.1)
        outcome = (state(win).get("export") or {}).get("state")
        check("the export of the frozen snapshot ended as a local file", exported.get("ok") and outcome == "done" and os.path.isfile(target), [exported, outcome])
        if os.path.isfile(target):
            text = pathlib.Path(target).read_text()
            document = json.loads(text)
            check("the export is tessel.perf-history v1", document.get("schema") == "tessel.perf-history" and document.get("schema_version") == 1, document.get("schema"))
            check("the export carries no local path", work not in text and str(pathlib.Path.home()) not in text, "a path leaked")
            doc_window = document.get("window") or {}
            check("the export's window equals the frozen view's", doc_window.get("start_ms") == window.get("start_ms") and doc_window.get("end_ms") == window.get("end_ms"),
                  [doc_window, window])
            held = (frozen_view.get("events") or {}).get("newest") or []
            doc_events = document.get("events") or []
            as_set = lambda items: sorted((item.get("at_ms"), item.get("kind")) for item in items)  # noqa: E731
            check("the export's events equal the frozen view's events", as_set(doc_events) == as_set(held) and bool(doc_events), [kinds(doc_events), kinds(held)])
            check("the export's segments equal the frozen view's", document.get("segments") == frozen_view.get("segments"), [document.get("segments"), frozen_view.get("segments")])
            report["export_build"] = document.get("build")
            report["export_events"] = kinds(doc_events)
            report["export_bytes"] = len(text)
        win.invoke(NODE, "clear_selection")
        check("resume returns to the moving window", win.invoke(NODE, "resume").get("ok") and state(win).get("frozen") is False)

        # ---- cancel, commit, undo ----
        ok, body = win.op({"op": "scene.fx.gesture.cancel", "gesture": gesture})
        check("scene.fx.gesture.cancel under the recording", ok, body)
        check("cancel leaves the document's picture, value and history", win.repaint(FRAME) == document_hash and win.undo_depth() == depth0
              and win.field("scene.fx.clip:fx-1", "intensity") == 0.7)
        ok, began = win.op({"op": "scene.fx.gesture.begin", "scene": "proof", "clip": "fx-1"})
        gesture = began.get("gesture") if ok else None
        win.op({"op": "scene.fx.gesture.update", "gesture": gesture, "ops": [{"op": "set_param", "clip": "fx-1", "param": "intensity", "value": 0.4}]})
        ok, body = win.op({"op": "scene.fx.gesture.commit", "gesture": gesture})
        check("commit under the recording is ONE undo entry", ok and win.undo_depth() == depth0 + 1, [body, win.undo_depth()])
        ok, body = win.op({"op": "history.undo"})
        check("undo restores the value and the pixels under the recording", ok and win.field("scene.fx.clip:fx-1", "intensity") == 0.7 and win.repaint(FRAME) == document_hash, body)
        check("the recording survived commit and undo", state(win).get("recording") is True)

        # ---- a scene switch under a gesture ----
        ok, began = win.op({"op": "scene.fx.gesture.begin", "scene": "proof", "clip": "fx-1"})
        gesture = began.get("gesture") if ok else None
        win.op({"op": "scene.fx.gesture.update", "gesture": gesture, "ops": [{"op": "set_param", "clip": "fx-1", "param": "intensity", "value": 0.2}]})
        lab.seek(FRAME + 8)
        shown = win.op({"op": "monitor.show_scene", "scene": "yard"})
        check("the monitor shows another scene under a gesture", shown[0], shown[1])
        check("the panel follows to the other scene", win.set("scene.picker", "value", "yard").get("ok"))
        ok, status = win.op({"op": "scene.fx.gesture.status"})
        check("the scene switch dropped the gesture in the core", ok and not status.get("gesture"), status)
        check("nothing was committed by the switch", win.undo_depth() == depth0, win.undo_depth())
        time.sleep(1.0)

        # ---- the history never invented an event for the gesture ----
        final = state(win)
        events = kinds(((final.get("view") or {}).get("events") or {}).get("newest") or [])
        report["final_events"] = events
        check("the history starts at the recording start", "recording_started" in events, events)
        check("the history invented no playback, failure or runtime event", not [kind for kind in events if kind in FORBIDDEN], events)
        check("a monitor frame of the gesture was counted (the monitor fed the history)", ((final.get("view") or {}).get("peaks") or {}).get("monitor_frames", {}).get("count", 0) > 0, final.get("view", {}).get("peaks"))
        stopped = win.set(NODE, "recording", False)
        check("stopping keeps the data readable", stopped.get("ok") and state(win).get("has_data") is True and state(win).get("recording") is False)
    finally:
        if lab is not None and pid is not None:
            fx.stop_lab(lab, pid)
        if args.keep:
            print(f"kept: {work}")
        else:
            shutil.rmtree(work, ignore_errors=True)
    report["load_average"] = os.getloadavg()
    print(json.dumps({"ok": not fx.failures, "failures": fx.failures, "checks": fx.checks, **report}), flush=True)
    return 1 if fx.failures else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--binary", default=str(ROOT / "target" / "release" / "tessel-editor"))
    parser.add_argument("--json")
    parser.add_argument("--keep", action="store_true")
    args = parser.parse_args()
    code = 1
    try:
        code = run(args)
    except (SystemExit, KeyboardInterrupt) as error:
        code = error.code if isinstance(error, SystemExit) and isinstance(error.code, int) and error.code != 0 else 130
        check("the proof was interrupted", False, f"{type(error).__name__}: {error}")
    except Exception as error:  # noqa: BLE001 - a crash is a recorded FAIL
        check("the proof ran to its end", False, f"{type(error).__name__}: {error}")
    finally:
        if fx.failures and code == 0:
            code = 1
        total = len(fx.checks)
        print(f"\n{total - len(fx.failures)}/{total} checks passed", flush=True)
        if args.json:
            fx.write_report(pathlib.Path(args.json), pathlib.Path(args.binary).resolve(), [], code)
    return code


if __name__ == "__main__":
    raise SystemExit(main())
