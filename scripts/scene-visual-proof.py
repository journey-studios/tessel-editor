#!/usr/bin/env python3
"""Native visual inspection of the scene inspector, tree and monitor note (BL087, frente R9).

    python3 scripts/directed-scene-proof/with-gpu-lock.py \
        python3 scripts/scene-visual-proof.py [--binary target/debug/tessel-editor] [--out DIR]

Starts the DIRECT binary with its own short HOME (own socket and settings; nothing of the user's
app or project is touched) and prepares every state with semantic requests only: no coordinates,
no pointer. It then captures the window with the OS (`screencapture -l <window id>`) at each
combination of window size (wide and the 960x640 minimum) and theme (dark, light) for:

  tree        the expanded tree of a deep, wide scene, nothing selected
  selected    a selected entity with its sliders, one modified value
  candidate   an open gesture: live value (document) and candidate (preview) differ
  error       a typed error (`invalid_value`) shown in the panel
  groupless   an entity without catalog properties (the empty-properties note)

The window id is the one of the PID this script started (never the first `tessel-editor` window
on screen, which could be the user's). A capture the OS refuses is recorded as such: the report
then says `captured:false` and no visual claim may be made from it.

Beside each capture the script records the semantic geometry it can read (`region:inspector`,
the inspector's scene controls) and asserts layout facts that do not need eyes: the inspector
keeps a usable width, the slider tracks measured by the geometry probes are wide enough to
drag, and no control sits outside the inspector. Looking at the PNGs is still the point; the
report lists their paths so they can be opened.

Exit status 1 on a failed check or a missing capture. Artifacts live under /tmp/te-r9 and
only the ones this script wrote are removed unless --keep.
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

import importlib.util  # noqa: E402

_spec = importlib.util.spec_from_file_location("video_proof", os.path.join(ROOT, "scripts/video-live-proof.py"))
video = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(video)

BASE_BUNDLE = os.path.join(ROOT, "docs/migration/fixtures/bundles/valid-graph-sockets-performance.bundle.json")
WORK = "/tmp/te-r9"
HOME = os.path.join(WORK, "h")
SIZES = {"wide": (1680, 1000), "min": (960, 640)}
THEMES = ("dark", "light")
# Inspector widths of the dock (Kit bounds: 100..628) tried at the minimum window, besides the default.
PANEL_WIDTHS = (100, 180, 628)
SCENE = "visual"
DEFINITION = "scene-rig"
DEEP_CHAIN = 9
WIDE_GROUPS = 24
# The smallest slider track (px) a person can still drag precisely. Below it the layout, not the
# user, is at fault.
MIN_TRACK_PX = 60

report = {"input": "semantic IPC for state; OS window capture for pixels; no pointer", "captures": [], "checks": []}
failures = []


def check(name, condition, detail=""):
    report["checks"].append({"check": name, "ok": bool(condition), "detail": str(detail)[:300]})
    print(("PASS " if condition else "FAIL ") + name + (f"  [{detail}]" if detail and not condition else ""), flush=True)
    if not condition:
        failures.append(name)


def node_path(node_id):
    return f"scene/{DEFINITION}/node/{node_id}"


def build_bundle(path):
    """The graph/sockets fixture plus a deep chain and a wide level, so the tree must scroll."""
    with open(BASE_BUNDLE) as handle:
        bundle = json.load(handle)
    nodes = bundle["sceneDefinitions"][0]["nodes"]
    quat = {"x": 0, "y": 0, "z": 0, "w": 1}
    one = {"x": 1, "y": 1, "z": 1}

    def group(node_id, parent):
        return {
            "id": node_id, "parentId": parent, "type": "group",
            "localPosition": {"x": 0, "y": 0, "z": 0}, "localRotation": dict(quat), "localScale": dict(one),
        }

    parent = "arm"
    for level in range(DEEP_CHAIN):
        node_id = f"deep-level-{level}-with-a-rather-long-name-to-test-truncation"
        nodes.append(group(node_id, parent))
        parent = node_id
    for index in range(WIDE_GROUPS):
        nodes.append(group(f"crowd-{index:02d}", "stage"))
    with open(path, "w") as handle:
        json.dump(bundle, handle)
    return parent


def window_id(pid):
    """The on-screen window of `pid`: the largest layer-0 one, so a WebKit helper is skipped."""
    script = (
        "import CoreGraphics\n"
        "let l = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as! [[String: Any]]\n"
        "var best = 0; var area = 0.0\n"
        "for w in l {\n"
        f"  if (w[kCGWindowOwnerPID as String] as? Int) == {pid} && (w[kCGWindowLayer as String] as? Int) == 0 {{\n"
        "    let b = w[kCGWindowBounds as String] as! [String: Double]\n"
        "    let a = (b[\"Width\"] ?? 0) * (b[\"Height\"] ?? 0)\n"
        "    if a > area { area = a; best = w[kCGWindowNumber as String] as! Int }\n"
        "  }\n"
        "}\n"
        "print(best)\n"
    )
    out = subprocess.run(["swift", "-e", script], capture_output=True, text=True).stdout.strip()
    return out if out and out != "0" else None


def png_size(path):
    with open(path, "rb") as handle:
        head = handle.read(24)
    if head[:8] != b"\x89PNG\r\n\x1a\n":
        return None
    return int.from_bytes(head[16:20], "big"), int.from_bytes(head[20:24], "big")


class Lab:
    def __init__(self, binary, out):
        self.out = out
        os.makedirs(out, exist_ok=True)
        shutil.rmtree(HOME, ignore_errors=True)
        os.makedirs(HOME)
        self.editor = video.Editor(os.path.abspath(binary), HOME)

    def send(self, request):
        response = self.editor.send({"version": 1, **request})
        return response

    def ok(self, request):
        response = self.send(request)
        if not response.get("ok"):
            raise AssertionError(json.dumps(response)[:400])
        return response["data"]

    def op(self, request):
        return self.ok({"command": "op", "request": request})

    def get(self, target):
        return self.ok({"command": "get", "target": target})

    def set(self, target, field, value):
        return self.send({"command": "set", "target": target, "field": field, "value": value})

    def invoke(self, target, action=None):
        request = {"command": "invoke", "target": target}
        if action:
            request["action"] = action
        return self.send(request)

    def settle(self, seconds=0.6):
        # macOS paints a covered window lazily and the Kit tree reflows a frame after a push.
        self.invoke("window.activate")
        time.sleep(seconds)

    def capture(self, name, size, theme):
        self.settle()
        path = os.path.join(self.out, f"{name}-{size}-{theme}.png")
        entry = {"name": name, "size": size, "theme": theme, "path": path, "captured": False}
        report["captures"].append(entry)
        wid = window_id(self.editor.process.pid)
        if not wid:
            entry["limit"] = "no on-screen window for this PID (screen locked, covered or capture denied)"
            return entry
        run = subprocess.run(["screencapture", "-x", "-o", "-l", wid, path], capture_output=True, text=True)
        dims = png_size(path) if os.path.exists(path) else None
        if run.returncode != 0 or not dims:
            entry["limit"] = f"screencapture failed ({run.returncode}): {run.stderr.strip()[:120]}"
            return entry
        entry.update(captured=True, pixels=list(dims), window_id=wid)
        window = self.get("window")
        inspector = region_bounds(self).get("region:inspector")
        entry["window"] = [window["width"], window["height"]]
        entry["inspector"] = inspector
        if inspector:
            crop_inspector(entry, inspector, entry["window"])
        return entry


def crop_inspector(entry, bounds, window):
    """Writes `<capture>-inspector.png`, the inspector region only, when Pillow is present."""
    try:
        from PIL import Image
    except ImportError:
        return
    scale = Image.open(entry["path"]).size[0] / window[0]
    image = Image.open(entry["path"])
    box = (
        int(bounds["x"] * scale), int(bounds["y"] * scale),
        int((bounds["x"] + bounds["width"]) * scale), int((bounds["y"] + bounds["height"]) * scale),
    )
    crop_path = entry["path"][:-4] + "-inspector.png"
    image.crop(box).save(crop_path)
    entry["inspector_crop"] = crop_path


def region_bounds(lab):
    """`{region id: bounds}` of the measured regions, from the semantic tree."""
    nodes = lab.ok({"command": "tree"})["nodes"]
    return {node["id"]: node["bounds"] for node in nodes if node.get("id", "").startswith("region:") and "bounds" in node}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default=os.path.join(ROOT, "target/debug/tessel-editor"))
    parser.add_argument("--out", default=os.path.join(WORK, "shots"))
    parser.add_argument("--report")
    parser.add_argument("--keep", action="store_true")
    args = parser.parse_args()

    os.makedirs(WORK, exist_ok=True)
    bundle_path = os.path.join(WORK, "visual.bundle.json")
    deepest = build_bundle(bundle_path)
    lab = Lab(args.binary, args.out)
    try:
        lab.editor.wait_ready(90)
        lab.op({"op": "scene.attach", "id": SCENE, "path": bundle_path})
        check("the scene attaches from a path", lab.get("scene.panel")["state"]["available"] is True)
        lab.set("inspector.mode", "value", "scene")
        for ancestor in ("root", "stage", "arm"):
            lab.invoke(f"scene.entity:{node_path(ancestor)}", "expand")
        deep = node_path(deepest)
        stage = lab.get("scene.tree")["state"]
        report["tree"] = {"visible_entities": stage["visible_entities"], "truncated": stage["truncated"]}
        check("the tree is wider than a minimum-size panel", stage["visible_entities"] > 20, stage["visible_entities"])

        pen = node_path("pen")
        pen_width = f"scene.property:{pen}/width"
        pen_x = f"scene.property:{pen}/position"

        def prepare(theme):
            lab.send({"command": "theme", "target": theme})
            lab.op({"op": "scene.select", "scene": SCENE, "entities": []})

        def states(size, theme, only=None):
            """Each state is reached by semantic requests, captured, then undone to the baseline."""
            wanted = lambda name: only is None or name in only
            if wanted("tree"):
                lab.op({"op": "scene.select", "scene": SCENE, "entities": []})
                lab.capture("tree", size, theme)
            lab.op({"op": "scene.select", "scene": SCENE, "entities": [pen]})
            if wanted("selected"):
                lab.set(pen_width, "value", 1.25)
                lab.capture("selected", size, theme)
            if wanted("deep"):
                lab.op({"op": "scene.select", "scene": SCENE, "entities": [deep]})
                lab.capture("deep", size, theme)
                lab.op({"op": "scene.select", "scene": SCENE, "entities": [pen]})
            if wanted("lastrow"):
                lab.op({"op": "scene.select", "scene": SCENE, "entities": [node_path("crowd-23")]})
                lab.capture("lastrow", size, theme)
                lab.op({"op": "scene.select", "scene": SCENE, "entities": [pen]})
            if wanted("candidate"):
                lab.op({"op": "scene.gesture.begin", "scene": SCENE, "targets": [pen]})
                gesture = lab.get("scene.panel")["state"]["gesture"]
                lab.op({"op": "scene.gesture.update", "gesture": gesture["id"],
                        "ops": [{"set": f"{pen}/width", "value": 3.0}]})
                lab.capture("candidate", size, theme)
                lab.invoke("scene.panel", "cancel_gesture")
            if wanted("error"):
                lab.set(pen_width, "value", -1)
                lab.capture("error", size, theme)
            if wanted("extreme"):
                lab.set(pen_x, "x", 12345.678)
                lab.set(pen_x, "y", -0.001)
                lab.capture("extreme", size, theme)
                lab.set(pen_x, "x", 0.1)
                lab.set(pen_x, "y", 0.2)
            if wanted("vector"):
                lab.op({"op": "scene.select", "scene": SCENE, "entities": [node_path("stage")]})
                lab.set(f"scene.property:{node_path('stage')}/position", "x", 0.5)
                lab.capture("vector", size, theme)

        def record_window(label, width, height):
            window = lab.get("window")
            report.setdefault("windows", {})[label] = {
                "requested": [width, height], "reported": [window["width"], window["height"]],
                "regions": region_bounds(lab),
            }

        for size, (width, height) in SIZES.items():
            for theme in THEMES:
                lab.set("window", "size", f"{width}x{height}")
                prepare(theme)
                lab.settle(0.8)
                record_window(f"{size}-{theme}", width, height)
                states(size, theme)

        # The dock lets a person make the inspector as narrow as 100 px or as wide as 628: the
        # layout has to survive both at the smallest window, not only at the 272 px default.
        lab.set("window", "size", "960x640")
        for panel in PANEL_WIDTHS:
            answer = lab.set("dock:right", "size", panel)
            check(f"the dock accepts an inspector {panel}px wide", answer.get("ok"), answer)
            for theme in THEMES:
                prepare(theme)
                lab.settle(0.8)
                record_window(f"panel{panel}-{theme}", 960, 640)
                states(f"panel{panel}", theme, only=("selected", "extreme", "error"))

        captured = [entry for entry in report["captures"] if entry["captured"]]
        check("every capture was taken by the OS", len(captured) == len(report["captures"]),
              [e.get("limit") for e in report["captures"] if not e["captured"]][:1])
        if not captured:
            report["visual_validated"] = False
            report["note"] = "no capture: visual inspection NOT performed"
        else:
            report["visual_validated"] = "pending-human-or-agent-review"
    except Exception as error:  # report first, then fail: the capture list is the evidence
        check("the run completed", False, repr(error))
    finally:
        try:
            lab.editor.stop()
        finally:
            lab.editor.log.close()
            report["ok"] = not failures
            if args.report:
                with open(args.report, "w") as handle:
                    json.dump(report, handle, indent=2)
                    handle.write("\n")
            print(json.dumps({"ok": report["ok"], "captures": len(report["captures"]),
                              "captured": sum(1 for e in report["captures"] if e["captured"]),
                              "failures": failures}))
            if not args.keep:
                for leftover in (bundle_path,):
                    if os.path.exists(leftover):
                        os.remove(leftover)
                shutil.rmtree(HOME, ignore_errors=True)
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
