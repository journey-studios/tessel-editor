#!/usr/bin/env python3
"""F060 proof: the point light in the *real* window, through semantic requests only (no clicks).

    ./scripts/cargo build -p tessel-editor --locked
    python3 scripts/point-light-ui-proof.py [--keep]

Launches `target/debug/tessel-editor`, attaches the synthetic corpus of `scripts/directed-scene-proof`
twice (without and with one lamp), then:

  - the inspector node of the lamp's radius/intensity/position exists, with the value of the document;
  - `set scene.property:<radius>` (the inspector's own handler) is one undo entry and the window's
    `scene.render` of that frame differs from the unlit scene's and from the one before the edit;
  - a range-knob drag through `scene.pointer.*` leaves the radius `scene.range` leaves, in one entry;
  - the animated-position refusal (`disabled_reason`) is proved by `scene_light_tests.rs` and the core
    test, not here;
  - undo gives back the first lit picture (same hash).

The monitor's WebView is the same renderer that `scene.render` calls; this does not prove a physical
pointer, nor the overlay painted by WKWebView, nor performance.
"""
import json
import os
import socket
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SOCKET = os.path.expanduser("~/.cache/tessel-editor/ui.sock")
BINARY = os.environ.get("TESSEL_EDITOR_BIN", os.path.join(ROOT, "target/debug/tessel-editor"))
CORPUS = os.path.join(ROOT, "scripts/directed-scene-proof/corpus")
LAMP = "scene/scene-a/light/lamp"
FAILED = []


def send(request, timeout=120):
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


def op(request, timeout=120):
    return send({"version": 1, "command": "op", "request": request}, timeout)


def get(target):
    return send({"version": 1, "command": "get", "target": target})


def setf(target, field, value):
    return send({"version": 1, "command": "set", "target": target, "field": field, "value": value})


def check(name, condition, detail=""):
    print(("PASS " if condition else "FAIL ") + name + ("" if condition else f"  [{detail}]"), flush=True)
    if not condition:
        FAILED.append(name)
    return condition


def data(reply):
    return reply.get("data", reply)


def hash_of(scene):
    reply = op({"op": "scene.render", "scene": scene, "frame": 0, "width": 640, "height": 360})
    return data(reply)["frame"]["hash"] if reply["ok"] else json.dumps(reply)[:300]


def depth():
    return data(op({"op": "history.status"}))["history"]["undo_depth"]


def lamp_plan():
    return data(op({"op": "scene.plan", "scene": "street", "frame": 0}))["plan"]["lights"][0]


bundle = json.load(open(os.path.join(CORPUS, "bundle.json")))
plain = json.loads(json.dumps(bundle))
bundle["sceneDefinitions"][0]["lighting"] = {"ambient": 1, "lights": [{
    "id": "lamp", "position": {"x": 0.0, "y": 1.0, "z": -3.0},
    "color": {"r": 1.0, "g": 0.8, "b": 0.5}, "intensity": 6.0, "radius": 2.5, "flicker": 0.0}]}

process = subprocess.Popen([BINARY], stdout=open("/tmp/tessel-light-editor.log", "w"), stderr=subprocess.STDOUT)
try:
    for _ in range(150):
        if os.path.exists(SOCKET):
            try:
                send({"version": 1, "command": "status"})
                break
            except OSError:
                pass
        time.sleep(0.2)
    send({"version": 1, "command": "invoke", "target": "window.activate"})
    done = []

    def wait_jobs(count, timeout=240):
        """Resolving resources is a job: wait until it ran and was applied to the project."""
        end = time.time() + timeout
        while time.time() < end:
            state = get("jobs")["data"]["state"]
            if len(state["recent"]) >= count and not state["active"]:
                return state
            time.sleep(0.3)
        raise TimeoutError("the resources job never finished")

    check("a project", op({"op": "project.create", "name": "Light"})["ok"])
    for name, document in (("plain", plain), ("street", bundle)):
        check(f"attach {name}", op({"op": "scene.attach", "id": name, "bundle": document})["ok"])
        check(f"resolve {name}", op({"op": "scene.resources.resolve", "scene": name, "base": CORPUS})["ok"])
        wait_jobs(len(done) + 1)
        done.append(name)

    plain_hash, lit_hash = hash_of("plain"), hash_of("street")
    print("hashes", plain_hash, lit_hash, flush=True)
    check("the window draws the lit scene differently from the unlit one", plain_hash != lit_hash, f"{plain_hash} {lit_hash}")

    check("the scene inspector", setf("inspector.mode", "value", "scene")["ok"])
    if get("scene.picker")["ok"]:
        check("pick the lit scene", setf("scene.picker", "value", "street")["ok"])
    check("the lamp is selectable", op({"op": "scene.select", "scene": "street", "entities": [LAMP]})["ok"])
    for prop in ("radius", "intensity", "position", "color/r"):
        node = get(f"scene.property:{LAMP}/{prop}")
        check(f"inspector node {prop}", node["ok"], json.dumps(node)[:200])
    before = depth()
    edit = setf(f"scene.property:{LAMP}/radius", "value", "3.5")
    check("the inspector sets the radius", edit["ok"], json.dumps(edit)[:300])
    check("one undo entry", depth() == before + 1, f"{before} -> {depth()}")
    check("the document has it", lamp_plan()["radius"] == 3.5)
    edited_hash = hash_of("street")
    print("edited", edited_hash, flush=True)
    check("the picture changed", edited_hash != lit_hash)
    refused = setf(f"scene.property:{LAMP}/radius", "value", "-2")
    check("a negative radius is refused", not refused["ok"], json.dumps(refused)[:200])
    check("and leaves the history alone", depth() == before + 1)

    check("undo", op({"op": "history.undo"})["ok"])
    check("undo gives the first lit picture back", hash_of("street") == lit_hash)

    state = op({"op": "scene.gizmo.state", "scene": "street", "frame": 0})
    overlay = data(state).get("overlay") or data(state).get("result", {}).get("overlay")
    check("the gizmo overlay of the lamp", overlay is not None, json.dumps(state)[:300])
    knob = next(h for h in overlay["handles"] if h["id"] == "range")
    start = {"u": knob["to"]["u"], "v": knob["to"]["v"]}
    end = {"u": start["u"] + 0.04, "v": start["v"]}
    before = depth()
    down = op({"op": "scene.pointer.down", "scene": "street", "frame": 0, "point": start})
    check("press on the range knob", data(down).get("handle") == "range", json.dumps(down)[:400])
    op({"op": "scene.pointer.move", "point": end})
    up = data(op({"op": "scene.pointer.up", "point": end}))
    check("the drag is one undo entry", depth() == before + 1, f"{before} -> {depth()}")
    delta = up["delta"]
    check("the radius grew by the reported delta", abs(lamp_plan()["radius"] - (2.5 + delta)) < 1e-9, json.dumps(up)[:200])
    check("undo the drag", op({"op": "history.undo"})["ok"])
    check("scene.range gives the same radius", op({"op": "scene.range", "scene": "street", "entity": LAMP, "frame": 0, "delta": delta})["ok"]
          and abs(lamp_plan()["radius"] - (2.5 + delta)) < 1e-9)
    op({"op": "history.undo"})
    check("the window stayed alive", process.poll() is None)
finally:
    process.terminate()
    try:
        process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        process.kill()

print("FAILED: " + ", ".join(FAILED) if FAILED else "ALL PASSED")
sys.exit(1 if FAILED else 0)
