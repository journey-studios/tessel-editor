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


def camera_controls(expected, bundle, portrait_bundle=None):
    """Controls on 640x360@8 and, when the portrait bundle is given, on 360x640@24 (three judged markers, the truck of cam-b half done): a verifier that
    only ever failed on landscape would say nothing about the portrait profile."""
    palette = expected["palette"]
    results = []
    # the camera clip index is the clip the frame's shot plays (shot-a -> cam-a at frame 8; shot-b -> cam-b at frame 24, where the truck is half done): a mutation of
    # another shot's clip would change nothing in the picture and the control could never fail
    targets = [(bundle, 640, 360, 8, 0, "")]
    if portrait_bundle is not None:
        targets.append((portrait_bundle, 360, 640, 24, 1, " (portrait 360x640@24)"))
    for bundle, width, height, frame, clip, tag in targets:
        case = expected["camera"]["cases"][f"{width}x{height}@{frame}"]
        exact = verify.exact_masks_for(bundle, case)

        def judge(rgb, case=case, exact=exact, width=width, height=height):
            return lambda p: verify.judge_camera_frame(p, rgb, width, height, exact, case, palette)

        def picture(b, f=frame, state=None, width=width, height=height):
            return camera_picture(b, f, width, height, palette, state)

        def mutated(change, bundle=bundle):
            copy_bundle = copy.deepcopy(bundle)
            change(copy_bundle)
            return copy_bundle

        def fov_plus_five(b, clip=clip):
            b["cameraClips"][clip]["initialFovDeg"] += 5

        def dolly_reversed(b, clip=clip):
            b["cameraClips"][clip]["segments"][0]["distanceSceneUnits"] *= -1

        results.append(run_control(f"positive: the model's own rasterised picture{tag}", True, judge(picture(bundle))))
        results.append(run_control(f"negative: fov +5 deg{tag}", False, judge(picture(mutated(fov_plus_five)))))
        results.append(run_control(f"negative: dolly direction reversed{tag}", False, judge(picture(mutated(dolly_reversed)))))
        results.append(run_control(f"negative: mirrored x{tag}", False, judge(flip_horizontal(picture(bundle), width, height))))
        results.append(run_control(f"negative: camera one frame late{tag}", False, judge(picture(bundle, frame - 1))))

        def horizontal_fov(p, bundle=bundle, frame=frame, case=case, exact=exact, width=width, height=height):
            state = oracle.frame_state(bundle, frame)
            state["camera"] = dict(state["camera"])
            state["camera"]["fovDeg"] = math.degrees(2 * math.atan(math.tan(math.radians(state["camera"]["fovDeg"]) / 2) * height / width))
            return verify.judge_camera_frame(p, camera_picture(bundle, frame, width, height, palette, state), width, height, exact, case, palette)

        results.append(run_control(f"negative: vertical fov used as horizontal{tag}", False, horizontal_fov))
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
    """Every control runs on EVERY frame the corpus judges (`expected["splat"]["frames"]`): the overlap of the two order splats
    changes with the camera, and a positive control on one frame cannot show that another frame is not rejected."""
    palette = expected["palette"]
    width, height = expected["splat"]["renderSize"]
    results = []

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
        out = []
        for e in entries:
            e = dict(e)
            if e["node"] == "s-purple":
                e["sigmaPx"] /= 1.5
            if e["node"] == "s-pair":
                e["x"] += 40 if e["index"] == 0 else -40
            out.append(e)
        return out

    for frame in expected["splat"]["frames"]:
        case = expected["splat"]["cases"][str(frame)]
        tag = f" @f{frame}"

        def judge(rgb, case=case):
            return lambda p: verify.judge_splat_frame(p, rgb, width, height, case, palette)

        results.append(run_control(f"positive: the model's own gaussians composited over the flat background{tag}", True,
                                   judge(splat_picture(case, palette, width, height))))
        for name, change in (("centroid shifted 4 px", shift), ("colour channels swapped", swap_channels), ("front and back order swapped", swap_order),
                             ("size x1.5", bigger), ("splat missing", missing), ("node transform ignored", transform_ignored)):
            results.append(run_control(f"negative: {name}{tag}", False, judge(splat_picture(case, palette, width, height, change))))
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


def correct_export(expected, samples, channels=2):
    """What a correct export writes for `samples` (source-level mono): the origin's channel law applied, on every output channel."""
    law = 10 ** (oracle.channel_gain_db(expected["audio"]["mix"]["channels"], channels) / 20)
    scaled = [x * law for x in samples]
    return [list(scaled) for _ in range(channels)]


def audio_controls(expected, bundles, work):
    results = []
    work = pathlib.Path(work)

    def judge_file(case_id, name, channels):
        case = expected["audio"]["cases"][case_id]
        bundle = bundles[case["bundle"]]
        path = work / f"{case_id}-{name}.mp4"
        verify.write_av_file(path, channels, case["frames"], case["fps"])
        return lambda p: verify.judge_audio(p, path, case, bundle, label="", video_seconds=case["durationSecondsFloat"])

    for case_id in ("audio24_window_a", "audio24_window_b", "audio_ntsc_window"):
        exact = window(expected, bundles, case_id)
        results.append(run_control(f"positive: analytic window of {case_id}, stereo with the origin's mono->stereo law, AAC", True,
                                   judge_file(case_id, "exact", correct_export(expected, exact))))
    case_id, case, bundle = "audio24_window_a", expected["audio"]["cases"]["audio24_window_a"], bundles["audio24"]
    exact = window(expected, bundles, case_id)
    n = len(exact)

    def stereo(samples, extra=1.0):
        return [[extra * x for x in channel] for channel in correct_export(expected, samples)]

    def shift(k):
        return ([0.0] * k + exact)[:n]

    results.append(run_control("positive: mono output, no channel conversion (0 dB)", True, judge_file(case_id, "mono", [exact])))
    results.append(run_control("negative: mono source written at 0 dB (channel law ignored)", False, judge_file(case_id, "unlawful", [exact, exact])))
    results.append(run_control("negative: shifted 5 ms", False, judge_file(case_id, "shift5ms", stereo(shift(240)))))
    results.append(run_control("negative: shifted one frame", False, judge_file(case_id, "shift1frame", stereo(shift(2000)))))
    table = table_of(expected)
    start = Fraction(bundle["audio"]["editStartSeconds"]).limit_denominator(1000)
    origin = float(oracle.source_time_of(oracle.frames_to_seconds(case["from"], Fraction(24)), start))
    mix_44k = [table.value(origin + k / 44100) for k in range(int(case["durationSecondsFloat"] * 44100))]  # 44.1 kHz read as 48 kHz
    results.append(run_control("negative: rate bug (44.1 kHz read as 48 kHz)", False, judge_file(case_id, "ratebug", stereo(mix_44k))))
    results.append(run_control("negative: silent stream", False, judge_file(case_id, "silent", [[0.0] * n, [0.0] * n])))
    results.append(run_control("negative: gain -6 dB", False, judge_file(case_id, "gain", stereo(exact, 0.5))))
    results.append(run_control("negative: mix offset ignored", False,
                               judge_file(case_id, "offset", stereo(window(expected, bundles, case_id, edit_start=Fraction(0))))))
    results.append(run_control("negative: retired A1 reading (mix[0] plays at editStartSeconds)", False,
                               judge_file(case_id, "retiredA1", stereo(window(expected, bundles, case_id, edit_start=-start)))))
    results.append(run_control("negative: export range ignored", False, judge_file(case_id, "range", stereo(window(expected, bundles, case_id, first=0)))))
    swapped = oracle.ToneTable([(a, b, {440: 880, 880: 440}.get(hz, hz), amp) for a, b, hz, amp in table.segments], table.duration)
    results.append(run_control("negative: tone swapped", False, judge_file(case_id, "swapped", stereo(window(expected, bundles, case_id, table=swapped)))))
    results.append(run_control("negative: truncated audio", False, judge_file(case_id, "truncated", stereo(exact[: int(n * 0.6)]))))

    def no_audio(p):
        path = work / f"{case_id}-noaudio.mp4"
        verify.write_video_only(path, case["frames"], case["fps"])
        return verify.judge_audio(p, path, case, bundle, label="", video_seconds=case["durationSecondsFloat"])

    results.append(run_control("negative: no audio stream", False, no_audio))
    ntsc = expected["audio"]["cases"]["audio_ntsc_window"]
    wrong = window(expected, bundles, "audio_ntsc_window", fps=Fraction(30))
    results.append(run_control("negative: NTSC treated as 30 fps", False, judge_file("audio_ntsc_window", "ntsc-as-30", stereo(wrong))))
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
    results = [run_control("positive: a bundle that declares the corpus hash of every asset", True,
                           lambda p: verify.judge_declared_hashes(p, json.loads((corpus_dir / "bundle.camera.json").read_text()), assets)),
               run_control("positive: a package with every asset", True, lambda p: verify.judge_package(p, archive, assets))]
    victim = "assets/mk-green.png"

    flipped = work / "flipped.zip"
    verify.tamper_zip(archive, flipped, lambda info, data: bytes([data[0] ^ 1]) + data[1:] if info.filename.endswith(victim) else data)
    results.append(run_control("negative: asset byte flipped inside the package", False, lambda p: verify.judge_package(p, flipped, assets)))

    removed = work / "removed.zip"
    verify.tamper_zip(archive, removed, lambda info, data: None if info.filename.endswith(victim) else data)
    results.append(run_control("negative: asset removed from the package", False, lambda p: verify.judge_package(p, removed, assets)))

    def wrong_declared_hash(p):
        bundle = json.loads((corpus_dir / "bundle.camera.json").read_text())
        bundle["assets"][1]["contentHash"] = "0" * 64
        return verify.judge_declared_hashes(p, bundle, assets)

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
