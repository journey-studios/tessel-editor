#!/usr/bin/env python3
"""F060: discriminating mutations of the point light. One mutation at a time; each must make the
named test command FAIL, and the file is restored byte for byte (sha256 checked) before the next.

    python3 scripts/point-light-mutation-proof.py [name ...]

Runtime mutations rebuild `runtime/dist` (offline esbuild) and run the Node light tests;
core mutations run `scene_light` through the cargo wrapper. A mutation that does not make its tests
fail is reported as SURVIVED and the script exits 1. Run under the GPU lock for the runtime ones.
"""
import hashlib
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
JS = "runtime/src/directed-scene.js"
RUNTIME_TESTS = ["node", "--test", "test/directed-scene-light.test.mjs"]
CORE_TESTS = ["./scripts/cargo", "test", "-p", "tessel-editor-core", "--locked", "--test", "scene_light"]

MUTATIONS = {
    "falloff-exponent": (JS, "tesselLit += uTesselLightColor[i] * (facing * once * once);",
                         "tesselLit += uTesselLightColor[i] * (facing * once);", "runtime"),
    "lit-from-behind": (JS, "if (dot(tesselNormal, cameraPosition - vTesselWorld) < 0.0) tesselNormal = -tesselNormal;",
                        "if (dot(tesselNormal, cameraPosition - vTesselWorld) > 0.0) tesselNormal = -tesselNormal;", "runtime"),
    "colour-order": (JS, "tesselLit += uTesselLightColor[i] * (facing", "tesselLit += uTesselLightColor[i].bgr * (facing", "runtime"),
    "srgb-space": (JS, "outgoingLight = tesselEncode(clamp(tesselDecode(outgoingLight) * (vec3(1.0) + tesselLit), 0.0, 1.0));",
                   "outgoingLight = clamp(outgoingLight * (vec3(1.0) + tesselLit), 0.0, 1.0);", "runtime"),
    "hidden-takes-a-slot": (JS, "lights.filter((light) => !light.hidden)", "lights", "runtime"),
    "range-sign": ("crates/core/src/scene_interaction/gizmo/light.rs", "let radius = base + delta;",
                   "let radius = base - delta;", "core"),
    "keyed-channel-guard": ("crates/core/src/scene_edit/batch.rs", "&& declared.is_keyed(channel.as_str())",
                            "&& declared.is_keyed(\"never\")", "core"),
    "hidden-requires-capability": ("crates/core/src/scene_plan/validate.rs", "if lights.iter().any(|light| !light.hidden) {",
                                   "if !lights.is_empty() {", "core"),
}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(command, cwd):
    return subprocess.run(command, cwd=cwd, capture_output=True, text=True).returncode


def rebuild():
    subprocess.run(["node", "scripts/build.mjs"], cwd=ROOT / "runtime", check=True, capture_output=True)


def main(names):
    survivors = []
    for name in names or MUTATIONS:
        relative, old, new, kind = MUTATIONS[name]
        path = ROOT / relative
        original = path.read_bytes()
        before = digest(path)
        text = original.decode()
        assert text.count(old) == 1, f"{name}: the anchor must appear exactly once, found {text.count(old)}"
        try:
            path.write_text(text.replace(old, new))
            if kind == "runtime":
                rebuild()
                status = run(RUNTIME_TESTS, ROOT / "runtime")
            else:
                status = run(CORE_TESTS, ROOT)
        finally:
            path.write_bytes(original)
            if kind == "runtime":
                rebuild()
        assert digest(path) == before, f"{name}: the file was not restored"
        print(f"{'KILLED  ' if status != 0 else 'SURVIVED'} {name}", flush=True)
        if status == 0:
            survivors.append(name)
    sys.exit(1 if survivors else 0)


main(sys.argv[1:])
