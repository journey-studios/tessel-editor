#!/usr/bin/env python3
"""Native proof of a splat in the directed-scene monitor (real window, WKWebView + Spark) — F048 / BL025 subset.

    python3 scripts/directed-scene-proof/with-gpu-lock.py \\
        python3 scripts/scene-splat-native-proof.py --corpus DIR [--binary target/debug/tessel-editor] [--out FILE] [--keep]

`DIR` is a corpus of the independent acceptance harness (S10): `bundle.splat.json`, `splat/*.spz`, `assets/*.png` and
`expected.json`, whose expected splat centres and colours are computed by that harness's own oracle, not by the editor.
This script never writes to it.

Starts the DIRECT binary with a short isolated HOME (own socket and settings: the user's editor and project are never
addressed). Everything is semantic (`op`, `set timeline.playhead`, `get monitor.viewport`): no pointer, no coordinates.
Screen capture is not used; the monitor's picture is judged through `monitor.viewport` (the hash the WebView reported
for the frame it drew) against an explicit `scene.render` of the same frame, whose pixels are probed at the oracle's
splat centres:

  * the monitor converged on the edit frame and nothing is in flight;
  * the monitor's picture hash == the explicit render's hash (same plan, same runtime, same pixels);
  * every isolated splat's centre has the colour the file stores (BL019-F1: no darkening), within the harness
    tolerance, at the oracle's projected centre, at every frame (the camera moves);
  * negative controls: the same colour rule fails at a point shifted 40 px and against the colour the old
    sRGB->linear path painted, so a pass is not vacuous;
  * the monitor's own frame reports the splat layer (`directed_scene_splats`), so it is the Spark pass that drew;
  * the host negotiated `asset.identity` and stamped every load (S4), the splat counts as a loaded resource.

Exit 0 only if every check passed; 1 on a failed check; 2 on a missing prerequisite. Artifacts under /tmp/te-s7-*
(budget 2 GiB, refuses to start under 12 GiB free), removed unless --keep.
"""
import argparse
import importlib.util
import json
import os
import pathlib
import shutil
import sys
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts" / "directed-scene-proof"))
import harness as h  # noqa: E402

MIN_FREE_GIB, BUDGET_GIB = 12, 2
# `splat.centerColorDelta` of the S10 tolerances (historical baseline; not a pre-registration): a centre colour within 24 of the stored one.
COLOUR_TOLERANCE = 24
# Frames of the splat shot the monitor is asked to show: the camera moves between them.
FRAMES = (0, 12, 24, 36, 47)

failures, checks = [], []


def check(name, condition, detail=""):
    checks.append({"check": name, "ok": bool(condition), "detail": str(detail)[:400]})
    print(("PASS " if condition else "FAIL ") + name + (f"  [{str(detail)[:300]}]" if detail and not condition else ""), flush=True)
    if not condition:
        failures.append(name)
    return bool(condition)


def load_lifecycle():
    """The monitor driver of the lifecycle proof (`Lab`, `settle`, ...): same isolated-instance mechanics, no copy."""
    path = ROOT / "scripts" / "scene-lifecycle-proof.py"
    spec = importlib.util.spec_from_file_location("scene_lifecycle_proof", path)
    module = importlib.util.module_from_spec(spec)
    sys.modules["scene_lifecycle_proof"] = module
    spec.loader.exec_module(module)
    return module


def distance(a, b):
    return max(abs(x - y) for x, y in zip(a, b))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--corpus", required=True)
    parser.add_argument("--binary", default=str(ROOT / "target" / "debug" / "tessel-editor"))
    parser.add_argument("--out")
    parser.add_argument("--keep", action="store_true")
    args = parser.parse_args()
    corpus = pathlib.Path(args.corpus).resolve()
    needed = [corpus / "bundle.splat.json", corpus / "expected.json", pathlib.Path(args.binary), ROOT / "runtime" / "dist" / "runtime.js"]
    for path in needed:
        if not path.exists():
            print(f"missing prerequisite: {path}")
            return 2
    if h.free_gib("/tmp") < MIN_FREE_GIB:
        print(f"only {h.free_gib('/tmp'):.1f} GiB free under /tmp; this proof needs {MIN_FREE_GIB}")
        return 2
    expected_doc = json.loads((corpus / "expected.json").read_text(encoding="utf-8"))
    expected = expected_doc["splat"]
    width, height = expected["renderSize"]
    cases = expected["cases"]
    lifecycle = load_lifecycle()

    work = tempfile.mkdtemp(prefix="te-s7-", dir="/tmp")
    report = {"binary": h.binary_info(args.binary), "git": h.git_state(), "corpus": str(corpus), "frames": {}}
    lab = None
    try:
        lab = lifecycle.Lab(args.binary, work, {"TESSEL_VIEWPORT_INSPECT": "1"})
        lab.wait_ready()
        lab.invoke("window.activate")
        time.sleep(0.8)
        caps = lab.viewport().get("capabilities") or {}
        report["renderer"] = caps.get("renderer")

        folder = os.path.join(work, "ws", "s7")
        ok, body = lab.op({"op": "project.create", "name": "s7", "sequence": "main", "rate": 24, "width": width, "height": height,
                           "length": max(FRAMES) + 1, "dir": folder})
        if not check("the project is created at the oracle's render size", ok, body):
            return 1
        ok, body = lab.op({"op": "scene.attach", "id": "proof", "path": str(corpus / "bundle.splat.json")})
        if not check("scene.attach of the splat bundle", ok, body):
            return 1
        ok, loaded = lab.op({"op": "scene.resources.resolve", "scene": "proof", "base": str(corpus)})
        if not check("scene.resources.resolve approves the picture and splat files", ok, loaded):
            return 1
        if not lifecycle.commit_job(lab, loaded, "splat scene"):
            return 1
        ok, body = lab.op({"op": "monitor.show_scene", "scene": "proof"})
        if not check("monitor.show_scene", ok, body):
            return 1

        background = tuple(expected_doc["palette"]["background"])
        for frame in FRAMES:
            case = cases[str(frame)]
            label = f"frame {frame}"
            lab.seek(frame)
            state = lifecycle.settle(lab, frame, 60)
            if not check(f"{label}: the monitor converged on the frame", bool(state), lab.viewport().get("directed")):
                continue
            last = lab.viewport().get("last_frame") or {}
            shown = last.get("hash")
            check(f"{label}: the runtime drew the splat layer in the monitor frame", "directed_scene_splats" in json.dumps(last), sorted(last))
            isolated = [s for s in case["splats"] if s["asset"] != "sp-order"]
            if not check(f"{label}: the corpus has isolated colour probes", bool(isolated), case["splats"]):
                continue
            probes = [[int(round(s["x"])), int(round(s["y"]))] for s in isolated]
            shifted = [[min(width - 1, int(round(s["x"])) + 40), int(round(s["y"]))] for s in isolated]
            far = [list(point) for point in case["backgroundPoints"]]
            ok, body = lab.op({"op": "scene.render", "scene": "proof", "frame": frame, "width": width, "height": height,
                               "probes": probes + shifted + far})
            if not check(f"{label}: an explicit render answers", ok, body):
                continue
            result = body["frame"]
            check(f"{label}: the render was read back", result.get("inspected") is True, result.get("inspected"))
            check(f"{label}: the monitor's picture is the render's picture (hash)", bool(shown) and shown == result["hash"],
                  f"monitor={shown} render={result['hash']}")
            got = result.get("probes")
            if not check(f"{label}: every requested probe was returned",
                         isinstance(got, list) and len(got) == len(probes + shifted + far), got):
                continue
            centres, away, flat = got[: len(isolated)], got[len(isolated): 2 * len(isolated)], got[2 * len(isolated):]
            deltas = {}
            for splat, centre, off in zip(isolated, centres, away):
                stored = splat["color"]
                key = f"{splat['node']}[{splat['index']}]"
                deltas[key] = {"measured": centre, "stored": stored}
                check(f"{label}: {key} centre shows the stored colour within {COLOUR_TOLERANCE}", distance(centre, stored) <= COLOUR_TOLERANCE,
                      f"{centre} vs {stored}")
                # Negative controls: the rule must fail somewhere it should, and for the darkened colour the old path painted.
                check(f"{label}: {key} control — 40 px away is not the splat colour", distance(off, stored) > COLOUR_TOLERANCE, f"{off} vs {stored}")
                darkened = [round(255 * (c / 255) ** 2.2) for c in stored]
                if distance(darkened, stored) > 2 * COLOUR_TOLERANCE:
                    check(f"{label}: {key} control — not the darkened sRGB->linear colour {darkened}", distance(centre, darkened) > COLOUR_TOLERANCE,
                          f"{centre} vs {darkened}")
            for point, value in zip(far, flat):
                check(f"{label}: background at {point} is flat", distance(value, background) <= 6, value)
            report["frames"][frame] = {"hash": result["hash"], "centres": deltas}

        final = lab.viewport()
        d = lifecycle.directed(final)
        slots = d.get("slots") or {}
        check("the host negotiated `asset.identity` with the real runtime", d.get("asset_identity") is True, d)
        check("every load the book tracks was stamped with a request", slots.get("tracked_loads", 0) >= 1 and slots.get("identified_loads") == slots.get("tracked_loads"), slots)
        resources = (lifecycle.cache_of(final) or {}).get("scene_resources")
        check("the runtime holds exactly what the host's book holds (splat included)", resources == slots.get("occupied") and (resources or 0) >= 1, (resources, slots))
        check("the editor is still ready and unfailed", final["phase"] == "ready" and not final["session"].get("error"), final["session"].get("error"))
        used = h.tree_bytes(work) / 1024 ** 3
        report["artifacts_gib"] = round(used, 3)
        check(f"own artifacts stayed under {BUDGET_GIB} GiB ({used:.3f})", used < BUDGET_GIB)
    finally:
        try:
            if lab:
                lab.stop()
        finally:
            if args.keep:
                print("kept", work)
            else:
                shutil.rmtree(work, ignore_errors=True)
    report["checks"] = checks
    report["failures"] = failures
    print(f"\n{len(checks) - len(failures)}/{len(checks)} checks passed")
    if args.out:
        pathlib.Path(args.out).write_text(json.dumps(report, indent=1), encoding="utf-8")
    print(json.dumps({"checks": len(checks), "failures": failures}))
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
