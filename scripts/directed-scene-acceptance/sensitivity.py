"""Sensitivity of the S10 rules, computed from the model alone: how large an error each tolerance still lets through.

A tolerance with no sensitivity is a number nobody has argued about. This module derives, from `corpus/expected.json`, the
oracle, the verifiers and `tolerances.json` (never from a picture or a file of the editor), the errors each rule can and cannot see,
so that the numbers are justified before a live run and a reader can tell "detected" from "passes a plausible error":

    python3 scripts/directed-scene-acceptance/sensitivity.py            # JSON on stdout (the audio sweep needs ffmpeg)
    python3 scripts/directed-scene-acceptance/sensitivity.py --fast     # skips the camera and audio sweeps (pure arithmetic)

It reports: (1) the size error the half-intensity-radius rule detects, per splat and frame, in the PNG and in the video regime;
(2) how well the back splat of the overlap pair is separated from "missing" and from "opaque disc" at the probe pixel;
(3) the smallest FOV error of a sweep that the camera rules detect on a model picture; (4) the smallest audio shift the onset rule detects;
(5) how much of each tolerance the model's own correct picture uses (the positive controls, PNG and codec regimes).
It is not a validation of the editor and it approves nothing.
"""
import argparse
import copy
import json
import pathlib
import sys
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import controls  # noqa: E402
import oracle  # noqa: E402
import verify  # noqa: E402

TOL = verify.TOLERANCES


def splat_size(expected):
    """For every isolated splat of every frame: `f*`, the smallest size factor the rule detects in each regime
    (`(f - 1) * r50 > relative * r50 + absolutePx`  <=>  `f > 1 + relative + absolutePx / r50`)."""
    out = {"png": [], "video": []}
    for frame in expected["splat"]["frames"]:
        case = expected["splat"]["cases"][str(frame)]
        for entry in case["splats"]:
            if entry["asset"] == "sp-order":
                continue
            for regime, key in (("png", "halfIntensityRadius"), ("video", "videoHalfIntensityRadius")):
                rule = TOL["splat"][key]
                out[regime].append({"frame": frame, "splat": f"{entry['node']}[{entry['index']}]", "r50Px": entry["r50Px"],
                                    "detectsFactorAbove": round(1 + rule["relative"] + rule["absolutePx"] / entry["r50Px"], 3)})
    summary = {}
    for regime, rows in out.items():
        factors = [r["detectsFactorAbove"] for r in rows]
        summary[regime] = {"measurements": len(rows), "detectsFactorAboveMin": min(factors), "detectsFactorAboveMax": max(factors),
                           "blindToX1_5": sum(1 for f in factors if f >= 1.5), "blindToX2": sum(1 for f in factors if f >= 2.0)}
    return {"summary": summary, "rows": out}


def back_splat(expected):
    """Separation (max-channel distance) of the three back-splat hypotheses at the probe pixel of every frame."""
    rows = []
    for frame in expected["splat"]["frames"]:
        probe = expected["splat"]["cases"][str(frame)]["backProbe"]
        back_centre = None
        entries = expected["splat"]["cases"][str(frame)]["splats"]
        back = next(e for e in entries if e["node"] == "s-order" and e["index"] == 1)
        without = [e for e in entries if e is not back]
        centre = (int(round(back["x"])), int(round(back["y"])))
        background = expected["palette"]["background"]
        back_centre = verify.distance(oracle.composite_at(entries, background, *centre), oracle.composite_at(without, background, *centre))
        rows.append({"frame": frame, "probePixel": [probe["x"], probe["y"]], "separation": probe["separation"],
                     "marginPerHypothesis": probe["separation"] / 2, "missingVsDrawnAtBackCentre": back_centre})
    return {"minSeparation": min(r["separation"] for r in rows), "rows": rows}


def camera_fov(expected, bundles, deltas=(0.1, 0.25, 0.5, 1.0, 5.0)):
    """The markers of 640x360@8 drawn with the FOV of cam-a off by `delta` degrees and judged in the PNG regime: failures per delta."""
    palette = expected["palette"]
    case = expected["camera"]["cases"]["640x360@8"]
    exact = verify.exact_masks_for(bundles["camera"], case)
    rows = []
    for delta in deltas:
        changed = copy.deepcopy(bundles["camera"])
        changed["cameraClips"][0]["initialFovDeg"] += delta
        collector = verify.Collector()
        verify.judge_camera_frame(collector, controls.camera_picture(changed, 8, 640, 360, palette), 640, 360, exact, case, palette)
        rows.append({"fovErrorDeg": delta, "failedChecks": len(collector.failed()), "detected": bool(collector.failed())})
    return rows


def audio_shift(expected, bundles, work, shifts_ms=(1, 2, 3, 4, 5, 6)):
    """The analytic window of audio24_window_a delayed by `ms` and judged: which delays the onset rule reports."""
    case_id = "audio24_window_a"
    case, bundle = expected["audio"]["cases"][case_id], bundles["audio24"]
    exact = controls.window(expected, bundles, case_id)
    rows = []
    for ms in shifts_ms:
        samples = int(round(ms * 48))
        path = pathlib.Path(work) / f"shift-{ms}ms.mp4"
        verify.write_av_file(path, controls.correct_export(expected, ([0.0] * samples + exact)[:len(exact)]), case["frames"], case["fps"])
        collector = verify.Collector()
        verify.judge_audio(collector, path, case, bundle, video_seconds=case["durationSecondsFloat"])
        edges = [name for name in collector.failed() if "edge at" in name]
        rows.append({"shiftMs": ms, "detected": bool(edges), "failedEdgeChecks": len(edges)})
    return rows


def positive_margins(expected, bundles):
    """How much of each tolerance the model's own correct picture uses, in the PNG regime and after the preset's codec chain
    (the positive controls' pictures): the worst centroid offset, the least mask IoU, the worst interior / centre colour distance,
    the worst half-intensity-radius error over its allowance. A margin near 1.0 (or an IoU near its floor) is a tolerance with no room."""
    palette = expected["palette"]
    out = {}
    for tag, video, transform in controls.regimes():
        regime = "video" if video else "png"
        camera = {"maxOffsetPx": 0.0, "minIou": 1.0, "maxInteriorDelta": 0.0}
        for key, case in expected["camera"]["cases"].items():
            width, height = case["size"]
            bundle = bundles[case["bundle"]]
            rgb = transform(controls.camera_picture(bundle, case["frame"], width, height, palette), width, height)
            exact = verify.exact_masks_for(bundle, case)
            collector = verify.Collector()
            metrics = verify.judge_camera_frame(collector, rgb, width, height, exact, case, palette, video=video)
            for name, m in metrics.items():
                if m.get("judged"):
                    camera["maxOffsetPx"] = max(camera["maxOffsetPx"], m["offsetPx"])
                    camera["minIou"] = min(camera["minIou"], m["iou"])
                    cx, cy = int(round(case["markers"][name]["x"])), int(round(case["markers"][name]["y"]))
                    camera["maxInteriorDelta"] = max(camera["maxInteriorDelta"], verify.distance(verify.pixel(rgb, width, cx, cy), palette["markers"][name]))
            camera.setdefault("failedChecks", 0)
            camera["failedChecks"] += len(collector.failed())
        splat = {"maxOffsetPx": 0.0, "maxCentreColourDelta": 0.0, "maxRadiusErrorOverAllowance": 0.0, "failedChecks": 0}
        rule = TOL["splat"]["videoHalfIntensityRadius" if video else "halfIntensityRadius"]
        for frame in expected["splat"]["frames"]:
            case = expected["splat"]["cases"][str(frame)]
            rgb = transform(controls.splat_picture(case, palette, 640, 360), 640, 360)
            collector = verify.Collector()
            metrics = verify.judge_splat_frame(collector, rgb, 640, 360, case, palette, video=video)
            splat["failedChecks"] += len(collector.failed())
            for entry in case["splats"]:
                m = metrics.get(f"{entry['node']}[{entry['index']}]")
                if not m:
                    continue
                splat["maxOffsetPx"] = max(splat["maxOffsetPx"], m["offsetPx"])
                splat["maxCentreColourDelta"] = max(splat["maxCentreColourDelta"], verify.distance(
                    verify.pixel(rgb, 640, int(round(entry["x"])), int(round(entry["y"]))), entry["color"]))
                allowance = rule["relative"] * entry["r50Px"] + rule["absolutePx"]
                splat["maxRadiusErrorOverAllowance"] = max(splat["maxRadiusErrorOverAllowance"], abs(m["r50"] - m["r50Expected"]) / allowance)
        out[regime] = {"camera": camera, "splat": splat}
    return out


def compute(fast=False, work=None):
    expected, bundles, _ = controls.load()
    report = {"splatSize": splat_size(expected)["summary"], "backSplat": back_splat(expected)}
    if not fast:
        report["cameraFov"] = camera_fov(expected, bundles)
        report["positiveMargins"] = positive_margins(expected, bundles)
        with tempfile.TemporaryDirectory(prefix="s10-sensitivity-", dir=work) as tmp:
            report["audioShift"] = audio_shift(expected, bundles, tmp)
    return report


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--fast", action="store_true")
    args = parser.parse_args(argv)
    print(json.dumps(compute(fast=args.fast), indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
