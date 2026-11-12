#!/usr/bin/env python3
"""F060 proof: the point light end to end through the headless JSONL session (no window, no GPU).

Drives `./scripts/headless` with semantic requests only: attach a scene with one lamp, read the plan,
drag the range knob by pointer events, check the same radius `scene.range` writes, edit the numeric
properties, refuse a keyed channel and undo. Prints a short JSON summary; exits non-zero on a
failed expectation. The pixels are proved elsewhere (`crates/render/tests/scene_light_*.rs`).
"""
import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
BUNDLE = ROOT / "docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json"
LAMP = "scene/scene-a/light/lamp"

bundle = json.loads(BUNDLE.read_text())
bundle["sceneDefinitions"][0]["lighting"] = {
    "ambient": 1,
    "lights": [{
        "id": "lamp", "position": {"x": -0.5, "y": 0.3, "z": 0.5},
        "color": {"r": 1.0, "g": 0.5, "b": 0.25}, "intensity": 2.0, "radius": 1.5, "flicker": 0.0,
    }],
}


def session(requests):
    stdin = "\n".join(json.dumps(r) for r in requests) + "\n"
    out = subprocess.run([str(ROOT / "scripts/headless")], input=stdin, capture_output=True,
                         text=True, cwd=ROOT, check=False).stdout
    return [json.loads(line) for line in out.splitlines() if line.strip()]


def body(reply):
    return reply.get("result", reply.get("error", reply))


def check(condition, what):
    if not condition:
        print(f"FAILED: {what}", file=sys.stderr)
        sys.exit(1)


setup = [
    {"op": "project.create", "name": "Light"},
    {"op": "scene.attach", "id": "street", "bundle": bundle},
    {"op": "scene.select", "scene": "street", "entities": [LAMP]},
]
overlay_request = {"op": "scene.gizmo.state", "scene": "street", "frame": 0}
first = session(setup + [overlay_request, {"op": "scene.plan", "scene": "street", "frame": 0}])
overlay = body(first[-2])["overlay"]
plan = body(first[-1])
plan = plan.get("plan", plan)
check(len(plan["lights"]) == 1 and "pointLights" in plan["requires"], "the plan carries the lamp")
knob = next(h for h in overlay["handles"] if h["id"] == "range")
point = {"u": knob["to"]["u"], "v": knob["to"]["v"]}
moved = {"u": point["u"] + 0.05, "v": point["v"]}

first_depth = session(setup + [{"op": "history.status"}])
drag = session(setup + [
    {"op": "scene.pointer.down", "scene": "street", "frame": 0, "point": point},
    {"op": "scene.pointer.move", "point": moved},
    {"op": "scene.pointer.up", "point": moved},
    {"op": "scene.plan", "scene": "street", "frame": 0},
    {"op": "history.status"},
])
up = body(drag[len(setup) + 2])
delta = up["delta"]
dragged_radius = up["radius"]
check(delta > 0 and abs(dragged_radius - (1.5 + delta)) < 1e-12, "the knob drag widens the range")
depth = body(first_depth[-1])["history"]["undo_depth"]
check(body(drag[-1])["history"]["undo_depth"] == depth + 1, "one undo entry for the gesture")
check(body(drag[-2])["plan"]["lights"][0]["radius"] == dragged_radius, "the plan has the new radius")

numeric = session(setup + [
    {"op": "scene.range", "scene": "street", "entity": LAMP, "frame": 0, "delta": delta},
    {"op": "scene.plan", "scene": "street", "frame": 0},
])
check(body(numeric[-1])["plan"]["lights"][0]["radius"] == dragged_radius, "scene.range gives the radius the drag gave")

edits = session(setup + [
    {"op": "scene.edit", "scene": "street", "ops": [
        {"set": f"{LAMP}/intensity", "value": 5.0},
        {"set": f"{LAMP}/position/z", "value": 1.25},
        {"set": f"{LAMP}/color/g", "value": 0.9},
    ]},
    {"op": "scene.plan", "scene": "street", "frame": 0},
    {"op": "history.undo"},
    {"op": "scene.plan", "scene": "street", "frame": 0},
    {"op": "scene.edit", "scene": "street", "ops": [{"set": f"{LAMP}/radius", "value": -1}]},
    {"op": "scene.edit", "scene": "street", "ops": [{"set": f"{LAMP}/intensity", "value": 21}]},
])
after = body(edits[len(setup) + 1])
after = after.get("plan", after)["lights"][0]
check(after["intensity"] == 5.0 and after["position"]["z"] == 1.25 and after["color"]["g"] == 0.9,
      "the numeric edit reaches the plan")
undone = body(edits[len(setup) + 3])
undone = undone.get("plan", undone)["lights"][0]
check(undone["intensity"] == 2.0 and undone["radius"] == 1.5, "undo restores the lamp")
check("error" in edits[-2] and "error" in edits[-1], "an invalid radius and intensity are refused")

print(json.dumps({"ok": True, "drag_delta": delta, "radius_after_drag": dragged_radius,
                  "refusals": [body(edits[-2]).get("code"), body(edits[-1]).get("code")]}))
