"""Positive and negative controls: proof that the verifiers of S10 can fail.

A verifier that has never failed proves nothing. Each control builds an input that is right (positive) or wrong in exactly
one known way (negative) *without the editor*: pictures from the model's own rasteriser, audio from the analytic tone
table encoded by FFmpeg, packages written by Python's zipfile. A negative control passes (`ok`) only when the verifier
reports at least one failed check on it; a positive control only when it reports none.
"""
import copy
import json
import math
import pathlib
import sys
from fractions import Fraction

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import corpus as corpus_module  # noqa: E402
import oracle  # noqa: E402
import verify  # noqa: E402


class Control:
    def __init__(self, name, should_pass):
        self.name, self.should_pass = name, should_pass
        self.collector = verify.Collector()

    def result(self):
        failed = self.collector.failed()
        return {"control": self.name, "should_pass": self.should_pass, "failed_checks": failed[:6],
                "ok": (not failed) if self.should_pass else bool(failed)}


def run_control(name, should_pass, body):
    control = Control(name, should_pass)
    try:
        body(control.collector)
    except Exception as error:  # noqa: BLE001 - a verifier that raises on a defective input has failed to judge it
        control.collector.check(f"verifier raised {type(error).__name__}", False, error)
    return control.result()


# ---------------------------------------------------------------------------------- pictures
def paint(width, height, background, layers):
    """`layers`: list of (pixel set, colour) painted in order. Returns rgb24 bytes."""
    rgb = bytearray(bytes(background) * (width * height))
    for pixels, color in layers:
        for x, y in pixels:
            base = (y * width + x) * 3
            rgb[base:base + 3] = bytes(color)
    return bytes(rgb)


def flip_horizontal(rgb, width, height):
    out = bytearray(len(rgb))
    for y in range(height):
        for x in range(width):
            src, dst = (y * width + x) * 3, (y * width + (width - 1 - x)) * 3
            out[dst:dst + 3] = rgb[src:src + 3]
    return bytes(out)


def camera_picture(bundle, frame, width, height, palette, state=None):
    state = state or oracle.frame_state(bundle, frame)
    nodes = oracle.nodes_of(bundle)
    layers = [(oracle.marker_mask(state, nodes[name], width, height), tuple(color)) for name, color in palette["markers"].items()]
    return paint(width, height, palette["background"], layers)


def camera_controls(expected, bundle):
    palette = expected["palette"]
    width, height, frame = 640, 360, 8
    case = expected["camera"]["cases"][f"{width}x{height}@{frame}"]
    exact = verify.exact_masks_for(bundle, case)
    results = []

    def judge(rgb):
        return lambda p: verify.judge_camera_frame(p, rgb, width, height, exact, case, palette)

    results.append(run_control("positive: the model's own rasterised picture", True, judge(camera_picture(bundle, frame, width, height, palette))))

    def mutated(change):
        copy_bundle = copy.deepcopy(bundle)
        change(copy_bundle)
        return copy_bundle

    def fov_plus_five(b):
        b["cameraClips"][0]["initialFovDeg"] += 5

    def dolly_reversed(b):
        b["cameraClips"][0]["segments"][0]["distanceSceneUnits"] *= -1

    results.append(run_control("negative: fov +5 deg", False, judge(camera_picture(mutated(fov_plus_five), frame, width, height, palette))))
    results.append(run_control("negative: dolly direction reversed", False, judge(camera_picture(mutated(dolly_reversed), frame, width, height, palette))))
    results.append(run_control("negative: mirrored x", False, judge(flip_horizontal(camera_picture(bundle, frame, width, height, palette), width, height))))
    results.append(run_control("negative: camera one frame late", False, judge(camera_picture(bundle, frame - 1, width, height, palette))))

    def horizontal_fov(p):
        state = oracle.frame_state(bundle, frame)
        state["camera"] = dict(state["camera"])
        state["camera"]["fovDeg"] = math.degrees(2 * math.atan(math.tan(math.radians(state["camera"]["fovDeg"]) / 2) * height / width))
        return verify.judge_camera_frame(p, camera_picture(bundle, frame, width, height, palette, state), width, height, exact, case, palette)

    results.append(run_control("negative: vertical fov used as horizontal", False, horizontal_fov))
    return results


# ---------------------------------------------------------------------------------- splats
def splat_picture(expected_case, palette, width, height, mutate=None):
    """Composites the model's gaussians back to front with the 'over' operator on the flat background."""
    entries = [dict(e) for e in expected_case["splats"]]
    if mutate:
        entries = mutate(entries)
    background = [float(c) for c in palette["background"]]
    rgb = bytearray(bytes(palette["background"]) * (width * height))
    for entry in sorted(entries, key=lambda e: -e["depth"]):
        sigma = entry["sigmaPx"]
        radius = int(4 * sigma) + 2
        for y in range(max(0, int(entry["y"]) - radius), min(height, int(entry["y"]) + radius + 1)):
            for x in range(max(0, int(entry["x"]) - radius), min(width, int(entry["x"]) + radius + 1)):
                alpha = math.exp(-((x - entry["x"]) ** 2 + (y - entry["y"]) ** 2) / (2 * sigma * sigma))
                base = (y * width + x) * 3
                for channel in range(3):
                    rgb[base + channel] = int(round(rgb[base + channel] * (1 - alpha) + entry["color"][channel] * alpha))
    return bytes(rgb)


def splat_controls(expected, bundle):
    palette = expected["palette"]
    width, height = expected["splat"]["renderSize"]
    case = expected["splat"]["cases"]["12"]
    results = []

    def judge(rgb):
        return lambda p: verify.judge_splat_frame(p, rgb, width, height, case, palette)

    results.append(run_control("positive: the model's own gaussians composited over the flat background", True,
                               judge(splat_picture(case, palette, width, height))))

    def shift(entries):
        for e in entries:
            e["x"] += 4
        return entries

    def swap_channels(entries):
        for e in entries:
            e["color"] = [e["color"][2], e["color"][1], e["color"][0]]
        return entries

    def bigger(entries):
        for e in entries:
            e["sigmaPx"] *= 1.5
        return entries

    def missing(entries):
        return [e for e in entries if e["node"] != "s-green"]

    def swap_order(entries):
        for e in entries:
            e["depth"] = -e["depth"]  # painted front to back instead of back to front
        return entries

    def transform_ignored(entries):
        nodes = {n["node"]: n for n in expected["splat"]["nodes"]}
        out = []
        for e in entries:
            e = dict(e)
            if e["node"] == "s-purple":
                e["sigmaPx"] /= 1.5
            if e["node"] == "s-pair":
                e["x"] += 40 if e["index"] == 0 else -40
            out.append(e)
        return out

    results.append(run_control("negative: centroid shifted 4 px", False, judge(splat_picture(case, palette, width, height, shift))))
    results.append(run_control("negative: colour channels swapped", False, judge(splat_picture(case, palette, width, height, swap_channels))))
    results.append(run_control("negative: front and back order swapped", False, judge(splat_picture(case, palette, width, height, swap_order))))
    results.append(run_control("negative: size x1.5", False, judge(splat_picture(case, palette, width, height, bigger))))
    results.append(run_control("negative: splat missing", False, judge(splat_picture(case, palette, width, height, missing))))
    results.append(run_control("negative: node transform ignored", False, judge(splat_picture(case, palette, width, height, transform_ignored))))
    return results


# ---------------------------------------------------------------------------------- audio
def table_of(expected):
    return oracle.ToneTable([(Fraction(a), Fraction(b), hz, amp) for a, b, hz, amp in expected["audio"]["mix"]["segments"]],
                            expected["audio"]["mix"]["seconds"])


def window(expected, bundles, case_id, **override):
    """Analytic samples of a case, optionally with one deliberate error (`fps`, `edit_start`, `first`, `table`)."""
    case = expected["audio"]["cases"][case_id]
    bundle = bundles[case["bundle"]]
    fps = override.get("fps", Fraction(bundle["fps"]["numerator"], bundle["fps"]["denominator"]))
    start = override.get("edit_start", Fraction(bundle["audio"]["editStartSeconds"]).limit_denominator(1000))
    values, _, _ = oracle.expected_window(override.get("table", table_of(expected)), fps, start,
                                          override.get("first", case["from"]), case["frames"], 48000)
    return values


def audio_controls(expected, bundles, work):
    results = []
    work = pathlib.Path(work)
    for case_id in ("audio24_window_a", "audio24_window_b", "audio_ntsc_window"):
        case = expected["audio"]["cases"][case_id]
        bundle = bundles[case["bundle"]]

        def judge(samples_by_channel, case=case, bundle=bundle, name=None):
            path = work / f"{case_id}-{name or 'file'}.mp4"
            verify.write_av_file(path, samples_by_channel, case["frames"], case["fps"])
            return lambda p: verify.judge_audio(p, path, case, bundle, label="", video_seconds=case["durationSecondsFloat"])

        results.append(run_control(f"positive: analytic window of {case_id}, AAC", True, judge([window(expected, bundles, case_id)], name="exact")))
    for case_id, bundle_key in (("audio24_window_a", "audio24"),):
        case = expected["audio"]["cases"][case_id]
        bundle = bundles[bundle_key]
        exact = window(expected, bundles, case_id)

        def judge(samples_by_channel, name, case=case, bundle=bundle, case_id=case_id):
            path = work / f"{case_id}-{name}.mp4"
            verify.write_av_file(path, samples_by_channel, case["frames"], case["fps"])
            return lambda p: verify.judge_audio(p, path, case, bundle, label="", video_seconds=case["durationSecondsFloat"])

        n = len(exact)
        shift = lambda k: ([0.0] * k + exact)[:n]  # noqa: E731
        results.append(run_control("positive: stereo with the same signal on both channels", True, judge([exact, exact], "stereo")))
        results.append(run_control("negative: shifted 5 ms", False, judge([shift(240)], "shift5ms")))
        results.append(run_control("negative: shifted one frame", False, judge([shift(2000)], "shift1frame")))
        mix_44k = []  # the 44.1 kHz samples read as if they were 48 kHz: pitch x1.088, duration x0.919
        table = table_of(expected)
        start = Fraction(bundle["audio"]["editStartSeconds"]).limit_denominator(1000)
        origin = float(oracle.frames_to_seconds(case["from"], Fraction(24)) - start)
        mix_44k = [table.value(origin + k / 44100) for k in range(int(case["durationSecondsFloat"] * 44100))]
        results.append(run_control("negative: rate bug (44.1 kHz read as 48 kHz)", False, judge([mix_44k], "ratebug")))
        results.append(run_control("negative: silent stream", False, judge([[0.0] * n], "silent")))
        results.append(run_control("negative: gain -6 dB", False, judge([[0.5 * x for x in exact]], "gain")))
        results.append(run_control("negative: mix offset ignored", False,
                                   judge([window(expected, bundles, case_id, edit_start=Fraction(0))], "offset")))
        results.append(run_control("negative: export range ignored", False, judge([window(expected, bundles, case_id, first=0)], "range")))
        swapped = oracle.ToneTable([(a, b, {440: 880, 880: 440}.get(hz, hz), amp) for a, b, hz, amp in table.segments], table.duration)
        results.append(run_control("negative: tone swapped", False, judge([window(expected, bundles, case_id, table=swapped)], "swapped")))
        results.append(run_control("negative: truncated audio", False, judge([exact[: int(n * 0.6)]], "truncated")))

        def no_audio(p):
            path = work / f"{case_id}-noaudio.mp4"
            verify.write_video_only(path, case["frames"], case["fps"])
            return verify.judge_audio(p, path, case, bundle, label="", video_seconds=case["durationSecondsFloat"])

        results.append(run_control("negative: no audio stream", False, no_audio))
    case = expected["audio"]["cases"]["audio_ntsc_window"]
    bundle = bundles["audio_ntsc"]
    wrong = window(expected, bundles, "audio_ntsc_window", fps=Fraction(30))

    def ntsc_as_30(p):
        path = work / "ntsc-as-30.mp4"
        verify.write_av_file(path, [wrong], case["frames"], case["fps"])
        return verify.judge_audio(p, path, case, bundle, label="", video_seconds=case["durationSecondsFloat"])

    results.append(run_control("negative: NTSC treated as 30 fps", False, ntsc_as_30))
    return results


# ---------------------------------------------------------------------------------- package
def package_controls(expected, corpus_dir, work):
    import zipfile
    corpus_dir, work = pathlib.Path(corpus_dir), pathlib.Path(work)
    assets = expected["assets"]
    archive = work / "good.zip"
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_STORED) as handle:
        for relative in assets:
            handle.writestr(f"project/{relative}", (corpus_dir / relative).read_bytes())
    results = [run_control("positive: a package with every asset", True, lambda p: verify.judge_package(p, archive, assets))]
    victim = "assets/mk-green.png"

    flipped = work / "flipped.zip"
    verify.tamper_zip(archive, flipped, lambda info, data: bytes([data[0] ^ 1]) + data[1:] if info.filename.endswith(victim) else data)
    results.append(run_control("negative: asset byte flipped inside the package", False, lambda p: verify.judge_package(p, flipped, assets)))

    removed = work / "removed.zip"
    verify.tamper_zip(archive, removed, lambda info, data: None if info.filename.endswith(victim) else data)
    results.append(run_control("negative: asset removed from the package", False, lambda p: verify.judge_package(p, removed, assets)))

    def wrong_declared_hash(p):
        bundle = json.loads((corpus_dir / "bundle.camera.json").read_text())
        record = bundle["assets"][1]
        record["contentHash"] = "0" * 64
        declared = {a["relativePath"]: a["contentHash"] for a in bundle["assets"]}
        mismatched = [path for path, digest in declared.items() if assets[path]["sha256"] != digest]
        p.check("the bundle declares the corpus hash of every asset", not mismatched, mismatched)

    results.append(run_control("negative: wrong hash declared in the bundle", False, wrong_declared_hash))
    directory = work / "dir-package"
    for relative in assets:
        target = directory / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes((corpus_dir / relative).read_bytes())
    results.append(run_control("positive: a directory package is read by the same independent reader", True,
                               lambda p: verify.judge_package(p, directory, assets)))
    return results


def load(corpus_dir=None):
    corpus_dir = pathlib.Path(corpus_dir or corpus_module.CORPUS_DIR)
    expected = json.loads((corpus_dir / "expected.json").read_text())
    bundles = {k: json.loads((corpus_dir / v["file"]).read_text()) for k, v in expected["bundles"].items()}
    return expected, bundles, corpus_dir
