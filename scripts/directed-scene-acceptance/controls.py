"""Positive and negative controls: proof that the verifiers of S10 can fail.

A verifier that has never failed proves nothing. Each control builds an input that is right (positive) or wrong in exactly
one known way (negative) *without the editor*: pictures from the model's own rasteriser, audio from the analytic tone
table encoded by FFmpeg, packages written by Python's zipfile. A negative control passes (`ok`) only when the verifier
reports a failed check of the kind it names (`must_fail`) and did not raise; a positive control only when it reports none.
Picture controls run in two regimes: the picture as the renderer writes it (PNG) and the same picture after the export's own
FFmpeg chain (4:2:0 BT.709 limited range, libx264), each judged with its own tolerances.
"""
import copy
import json
import math
import pathlib
import subprocess
import sys
import tempfile
from fractions import Fraction

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import corpus as corpus_module  # noqa: E402
import oracle  # noqa: E402
import verify  # noqa: E402


class Control:
    """One control. A negative control names the check(s) that have to fail (`must_fail`, substrings of check names): any other
    failure would be accepted, including a verifier that crashed or one that failed on something unrelated. An exception of the
    verifier fails the control, positive or negative: a verifier that raised has not judged the input.

    `stance` of a negative control: `detect` (the named check must fail), `blind` (the rule's own arithmetic says its tolerance
    cannot see this error: the named check must NOT fail, and if it does the declaration is stale and the control fails; recorded
    as a blind spot, never as a detection) or `either` (the error is within the guard band of the rule's limit: no claim, only
    `raised` fails; recorded as such)."""

    def __init__(self, name, should_pass, must_fail=(), stance="detect"):
        if not should_pass and not must_fail:
            raise ValueError(f"negative control `{name}` must name the check that has to fail")
        if stance not in ("detect", "blind", "either") or (should_pass and stance != "detect"):
            raise ValueError(f"bad stance {stance!r} for control `{name}`")
        self.name, self.should_pass, self.must_fail, self.stance = name, should_pass, tuple(must_fail), stance
        self.collector = verify.Collector()
        self.raised = None

    def result(self):
        failed = self.collector.failed()
        named = [name for name in failed if any(token in name for token in self.must_fail)]
        if self.raised is not None:
            ok = False
        elif self.should_pass:
            ok = not failed
        elif self.stance == "blind":
            ok = not named
        elif self.stance == "either":
            ok = True
        else:
            ok = bool(named)
        return {"control": self.name, "should_pass": self.should_pass, "must_fail": list(self.must_fail), "failed_checks": failed[:6],
                "named_failed": named[:3], "raised": None if self.raised is None else f"{type(self.raised).__name__}: {self.raised}",
                "stance": self.stance, "detected": bool(named), "ok": ok}


def run_control(name, should_pass, body, must_fail=(), stance="detect"):
    control = Control(name, should_pass, must_fail, stance)
    try:
        body(control.collector)
    except Exception as error:  # noqa: BLE001 - the verifier raised: it has not judged the input, so the control fails whatever its sign
        control.raised = error
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


# ---------------------------------------------------------------------------------- video regime
VIDEO_TAG = " (video regime)"
CHAIN = "format=rgb24,scale=flags=accurate_rnd+full_chroma_int:out_color_matrix=bt709:out_range=tv,format=yuv420p,setsar=1"


def video_roundtrip(rgb, width, height):
    """The picture after the export's own FFmpeg chain (`tolerances.json` `video`): RGB -> BT.709 limited-range 4:2:0 -> libx264
    medium, profile high, crf 18 -> decoded back by the same BT.709 decoder the live phases use. One intra frame: the motion and
    rate control of a real 48-frame export are NOT rehearsed. A real codec on the model's picture, not an export of the editor."""
    video = verify.TOLERANCES["video"]
    with tempfile.TemporaryDirectory(prefix="s10-video-") as tmp:
        path = pathlib.Path(tmp) / "frame.mp4"
        run = subprocess.run([verify.FFMPEG, "-v", "error", "-nostdin", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{width}x{height}", "-r", "24",
                              "-i", "-", "-vf", CHAIN, "-frames:v", "1", "-c:v", "libx264", "-preset", video["preset"], "-profile:v", video["profile"],
                              "-pix_fmt", "yuv420p", "-crf", str(video["crf"]), *verify.COLOR_TAGS, "-movflags", "+faststart", str(path)],
                             input=rgb, capture_output=True)
        if run.returncode != 0:
            raise RuntimeError(run.stderr.decode()[-300:])
        frames = list(verify.ev.decode(path, width, height))
    if len(frames) != 1:
        raise RuntimeError(f"expected one decoded frame, got {len(frames)}")
    return frames[0]


def regimes():
    """`[(tag, video flag, transform)]`: the PNG regime (the picture as the renderer writes it) and the video regime."""
    return [("", False, lambda rgb, w, h: rgb), (VIDEO_TAG, True, video_roundtrip)]


# ---------------------------------------------------------------------------------- cameras
def camera_picture(bundle, frame, width, height, palette, state=None):
    state = state or oracle.frame_state(bundle, frame)
    nodes = oracle.nodes_of(bundle)
    layers = [(oracle.marker_mask(state, nodes[name], width, height), tuple(color)) for name, color in palette["markers"].items()]
    return paint(width, height, palette["background"], layers)


GEOMETRY = ("centroid within", "mask IoU")  # what a camera geometry error has to trip: the position or the shape of a marker


def camera_controls(expected, bundle, portrait_bundle=None):
    """Controls on 640x360@8 and, when the portrait bundle is given, on 360x640@24 (three judged markers, the truck of cam-b half
    done) and 360x640@0 (a marker outside the frame): a verifier that only ever failed on landscape would say nothing about the
    portrait profile. EVERY control runs in the PNG regime and in the video regime (4:2:0 chroma, `video=True` tolerances)."""
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

        def mutated(change, bundle=bundle):
            copy_bundle = copy.deepcopy(bundle)
            change(copy_bundle)
            return copy_bundle

        def fov_plus_five(b, clip=clip):
            b["cameraClips"][clip]["initialFovDeg"] += 5

        def dolly_reversed(b, clip=clip):
            b["cameraClips"][clip]["segments"][0]["distanceSceneUnits"] *= -1

        def horizontal_fov_picture(bundle=bundle, frame=frame, width=width, height=height):
            state = oracle.frame_state(bundle, frame)
            state["camera"] = dict(state["camera"])
            state["camera"]["fovDeg"] = math.degrees(2 * math.atan(math.tan(math.radians(state["camera"]["fovDeg"]) / 2) * height / width))
            return camera_picture(bundle, frame, width, height, palette, state)

        pictures = [
            ("positive: the model's own rasterised picture", True, (), lambda b=bundle, f=frame, w=width, h=height: camera_picture(b, f, w, h, palette)),
            ("negative: fov +5 deg", False, GEOMETRY, lambda m=mutated, b=fov_plus_five, f=frame, w=width, h=height: camera_picture(m(b), f, w, h, palette)),
            ("negative: dolly direction reversed", False, GEOMETRY, lambda m=mutated, b=dolly_reversed, f=frame, w=width, h=height: camera_picture(m(b), f, w, h, palette)),
            ("negative: mirrored x", False, GEOMETRY, lambda b=bundle, f=frame, w=width, h=height: flip_horizontal(camera_picture(b, f, w, h, palette), w, h)),
            ("negative: camera one frame late", False, GEOMETRY, lambda b=bundle, f=frame, w=width, h=height: camera_picture(b, f - 1, w, h, palette)),
            ("negative: vertical fov used as horizontal", False, GEOMETRY, horizontal_fov_picture),
        ]
        for regime_tag, video, transform in regimes():
            for name, should_pass, must_fail, make in pictures:
                def body(p, make=make, transform=transform, video=video, case=case, exact=exact, width=width, height=height):
                    return verify.judge_camera_frame(p, transform(make(), width, height), width, height, exact, case, palette, video=video)

                results.append(run_control(f"{name}{tag}{regime_tag}", should_pass, body, must_fail))
    if portrait_bundle is not None:
        # a marker the model puts outside the frame (360x640@0: mk_yellow): the picture the model draws leaves nothing of it, and a
        # blob of its colour where the model has none must be reported, in 4:2:0 as well, where the edge of a present marker can
        # fall on the blend line of an absent colour (review R2-13)
        case = expected["camera"]["cases"]["360x640@0"]
        absent = [name for name, marker in case["markers"].items() if marker["area"] == 0]
        exact = verify.exact_masks_for(portrait_bundle, case)

        def clean():
            return camera_picture(portrait_bundle, 0, 360, 640, palette)

        def stray_blob():
            picture = bytearray(clean())
            for x in range(40):
                for y in range(40):
                    base = ((600 + y) * 360 + 300 + x) * 3
                    picture[base:base + 3] = bytes(palette["markers"][absent[0]])
            return bytes(picture)

        for regime_tag, video, transform in regimes():
            def judge_stray(p, make, transform=transform, video=video):
                return verify.judge_camera_frame(p, transform(make(), 360, 640), 360, 640, exact, case, palette, video=video)

            results.append(run_control(f"positive: a frame with a marker outside the picture, nothing stray (portrait 360x640@0){regime_tag}", True,
                                       lambda p, jc=judge_stray: jc(p, clean)))
            results.append(run_control(f"negative: marker colour where the model has none (portrait 360x640@0){regime_tag}", False,
                                       lambda p, jc=judge_stray: jc(p, stray_blob), ("absent from the picture",)))
    return results


# ---------------------------------------------------------------------------------- splats
def splat_picture(expected_case, palette, width, height, mutate=None):
    """Composites the model's gaussians back to front with the 'over' operator on the flat background. An entry with `disc: true`
    is drawn as an opaque disc of its half-intensity radius instead of a gaussian (a control: a renderer without the falloff)."""
    entries = [dict(e) for e in expected_case["splats"]]
    if mutate:
        entries = mutate(entries)
    rgb = bytearray(bytes(palette["background"]) * (width * height))
    for entry in sorted(entries, key=lambda e: -e["depth"]):
        sigma = entry["sigmaPx"]
        radius = int(4 * sigma) + 2
        for y in range(max(0, int(entry["y"]) - radius), min(height, int(entry["y"]) + radius + 1)):
            for x in range(max(0, int(entry["x"]) - radius), min(width, int(entry["x"]) + radius + 1)):
                squared = (x - entry["x"]) ** 2 + (y - entry["y"]) ** 2
                alpha = (1.0 if squared <= entry["r50Px"] ** 2 else 0.0) if entry.get("disc") else math.exp(-squared / (2 * sigma * sigma))
                base = (y * width + x) * 3
                for channel in range(3):
                    rgb[base + channel] = int(round(rgb[base + channel] * (1 - alpha) + entry["color"][channel] * alpha))
    return bytes(rgb)


def isolated_splats(case):
    """The splats the verifier judges one by one (every splat but the two of the overlap pair)."""
    return [(e["node"], e["index"]) for e in case["splats"] if e["asset"] != "sp-order"]


GUARD_PX = 0.5  # the radius scan resolves 0.25 px: a size error this close to the rule's limit is neither claimed visible nor blind


def size_error_state(entry, video, factor):
    """What the half-intensity-radius rule's own arithmetic says about a splat drawn `factor` times too big. The error is
    `(factor - 1) * r50` and the rule tolerates `relative * r50 + absolutePx` (the regime's numbers). `visible`: the error exceeds the
    tolerance by more than `GUARD_PX`, the rule must report it; `blind`: it is below it by more than `GUARD_PX`, the rule cannot see it
    (a declared blind spot, never a detection); `borderline`: within the guard, not claimed either way."""
    rule = verify.TOLERANCES["splat"]["videoHalfIntensityRadius" if video else "halfIntensityRadius"]
    margin = (factor - 1) * entry["r50Px"] - (rule["relative"] * entry["r50Px"] + rule["absolutePx"])
    return "visible" if margin > GUARD_PX else "blind" if margin < -GUARD_PX else "borderline"


def splat_controls(expected, bundle, work=None):
    """Every control runs on EVERY frame the corpus judges (`expected["splat"]["frames"]`), in the PNG regime and in the video
    regime. Size errors are also applied to ONE splat at a time (x1.5 and x2): applied to all of them, one detection hides the
    rest, and the rule's tolerance is blind to x1.5 on the smaller ones in the video regime (`size_error_state`)."""
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

    def scaled(factor, only=None):
        def change(entries):
            for e in entries:
                if only is None or (e["node"], e["index"]) == only:
                    e["sigmaPx"] *= factor
            return entries
        return change

    def missing(entries):
        return [e for e in entries if e["node"] != "s-green"]

    def back_missing(entries):
        return [e for e in entries if not (e["node"] == "s-order" and e["index"] == 1)]

    def back_as_disc(entries):
        for e in entries:
            if e["node"] == "s-order" and e["index"] == 1:
                e["disc"] = True  # an opaque disc of the half-intensity radius: no gaussian falloff
        return entries

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

    everywhere = (("centroid shifted 4 px", shift, ("centroid within",)),
                  ("colour channels swapped", swap_channels, ("centre colour within",)),
                  ("front and back order swapped", swap_order, ("s-order",)),
                  ("size x1.5", scaled(1.5), ("half-intensity radius",)),
                  ("splat missing", missing, ("a splat is drawn near",)),
                  ("back splat missing", back_missing, ("back splat is present",)),
                  ("back splat drawn as an opaque disc", back_as_disc, ("back splat is present",)),
                  ("node transform ignored", transform_ignored, ("centroid within", "half-intensity radius")))
    for frame in expected["splat"]["frames"]:
        case = expected["splat"]["cases"][str(frame)]
        entries = {(e["node"], e["index"]): e for e in case["splats"]}
        for regime_tag, video, transform in regimes():
            tag = f" @f{frame}{regime_tag}"

            def judge(make, case=case, transform=transform, video=video):
                return lambda p: verify.judge_splat_frame(p, transform(make(), width, height), width, height, case, palette, video=video)

            results.append(run_control(f"positive: the model's own gaussians composited over the flat background{tag}", True,
                                       judge(lambda case=case: splat_picture(case, palette, width, height))))
            for name, change, must_fail in everywhere:
                results.append(run_control(f"negative: {name}{tag}", False, judge(lambda case=case, change=change: splat_picture(case, palette, width, height, change)), must_fail))
            for node, index in isolated_splats(case):
                for factor, label in ((1.5, "size x1.5 of each splat alone"), (2.0, "size x2 of each splat alone")):
                    state = size_error_state(entries[(node, index)], video, factor)
                    results.append(run_control(f"negative: {label}: {node}[{index}]{tag}", False,
                                               judge(lambda case=case, change=scaled(factor, (node, index)): splat_picture(case, palette, width, height, change)),
                                               (f"{node}[{index}]: half-intensity radius",),
                                               stance={"visible": "detect", "blind": "blind", "borderline": "either"}[state]) | {"size_state": state})
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

    for case_id in ("audio24_window_a", "audio24_window_b", "audio_ntsc_window", "full_av"):
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

    def negative(name, make, must_fail):
        results.append(run_control(f"negative: {name}", False, make, must_fail))

    # the origin always writes stereo (its graph forces `channel_layouts=stereo`): a mono file is a regression, not a variant
    negative("mono output (the origin always writes stereo)", judge_file(case_id, "mono", [exact]), ("the origin always writes",))
    negative("mono source written at 0 dB (channel law ignored)", judge_file(case_id, "unlawful", [exact, exact]), ("gain",))
    negative("shifted 5 ms", judge_file(case_id, "shift5ms", stereo(shift(240))), ("edge at",))
    negative("shifted one frame", judge_file(case_id, "shift1frame", stereo(shift(2000))), ("edge at",))
    table = table_of(expected)
    start = Fraction(bundle["audio"]["editStartSeconds"]).limit_denominator(1000)
    origin = float(oracle.source_time_of(oracle.frames_to_seconds(case["from"], Fraction(24)), start))
    mix_44k = [table.value(origin + k / 44100) for k in range(int(case["durationSecondsFloat"] * 44100))]  # 44.1 kHz read as 48 kHz
    negative("rate bug (44.1 kHz read as 48 kHz)", judge_file(case_id, "ratebug", stereo(mix_44k)), ("sample-level correlation",))
    negative("silent stream", judge_file(case_id, "silent", [[0.0] * n, [0.0] * n]), ("gain", "tone window"))
    negative("gain -6 dB", judge_file(case_id, "gain", stereo(exact, 0.5)), ("gain",))
    negative("mix offset ignored", judge_file(case_id, "offset", stereo(window(expected, bundles, case_id, edit_start=Fraction(0)))), ("sample-level correlation", "edge at"))
    negative("retired A1 reading (mix[0] plays at editStartSeconds)", judge_file(case_id, "retiredA1", stereo(window(expected, bundles, case_id, edit_start=-start))),
             ("sample-level correlation", "edge at"))
    negative("export range ignored", judge_file(case_id, "range", stereo(window(expected, bundles, case_id, first=0))), ("sample-level correlation", "edge at"))
    swapped = oracle.ToneTable([(a, b, {440: 880, 880: 440}.get(hz, hz), amp) for a, b, hz, amp in table.segments], table.duration)
    negative("tone swapped", judge_file(case_id, "swapped", stereo(window(expected, bundles, case_id, table=swapped))), ("tone window",))
    negative("truncated audio", judge_file(case_id, "truncated", stereo(exact[: int(n * 0.6)])), ("duration", "samples"))

    def no_audio(p):
        path = work / f"{case_id}-noaudio.mp4"
        verify.write_video_only(path, case["frames"], case["fps"])
        return verify.judge_audio(p, path, case, bundle, label="", video_seconds=case["durationSecondsFloat"])

    negative("no audio stream", no_audio, ("decodable audio stream",))

    def constant_envelope(p):
        flat = dict(case, transitions=[])  # a case whose analytic window has no edge: a shifted file would pass the lag check
        path = work / f"{case_id}-constant.mp4"
        verify.write_av_file(path, correct_export(expected, exact), case["frames"], case["fps"])
        return verify.judge_audio(p, path, flat, bundle, label="", video_seconds=case["durationSecondsFloat"])

    negative("constant-envelope window (no edge: an offset is not observable)", constant_envelope, ("lag observable",))
    wrong = window(expected, bundles, "audio_ntsc_window", fps=Fraction(30))
    negative("NTSC treated as 30 fps", judge_file("audio_ntsc_window", "ntsc-as-30", stereo(wrong)), ("edge at",))
    return results


# ---------------------------------------------------------------------------------- package
def package_controls(expected, corpus_dir, work):
    import zipfile
    corpus_dir, work = pathlib.Path(corpus_dir), pathlib.Path(work)
    assets = expected["assets"]
    bundle_file = corpus_dir / expected["bundles"]["camera"]["file"]
    declared = {a["relativePath"]: a for a in json.loads(bundle_file.read_text(encoding="utf-8"))["assets"]}
    declared_assets = {path: info for path, info in assets.items() if path in declared}  # what the camera bundle declares (images, no mix)
    archive = work / "good.zip"
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_STORED) as handle:
        for relative in assets:
            handle.writestr(f"project/{relative}", (corpus_dir / relative).read_bytes())

    def bundle():
        return json.loads(bundle_file.read_text(encoding="utf-8"))

    results = [run_control("positive: a bundle that declares the corpus hash of every asset", True,
                           lambda p: verify.judge_declared_hashes(p, bundle(), declared_assets)),
               run_control("positive: a package with every asset", True, lambda p: verify.judge_package(p, archive, assets))]
    victim = "assets/mk-green.png"

    flipped = work / "flipped.zip"
    verify.tamper_zip(archive, flipped, lambda info, data: bytes([data[0] ^ 1]) + data[1:] if info.filename.endswith(victim) else data)
    results.append(run_control("negative: asset byte flipped inside the package", False, lambda p: verify.judge_package(p, flipped, assets), ("byte-identical",)))

    removed = work / "removed.zip"
    verify.tamper_zip(archive, removed, lambda info, data: None if info.filename.endswith(victim) else data)
    results.append(run_control("negative: asset removed from the package", False, lambda p: verify.judge_package(p, removed, assets), ("byte-identical",)))

    def wrong_declared_hash(p):
        changed = bundle()
        changed["assets"][1]["contentHash"] = "0" * 64
        return verify.judge_declared_hashes(p, changed, declared_assets)

    def missing_declaration(p):
        changed = bundle()
        changed["assets"] = [a for a in changed["assets"] if not a["relativePath"].endswith("mk-green.png")]
        return verify.judge_declared_hashes(p, changed, declared_assets)

    def unknown_declaration(p):
        changed = bundle()
        changed["assets"].append(dict(changed["assets"][0], relativePath="assets/not-in-the-corpus.png", assetId="intruder"))
        return verify.judge_declared_hashes(p, changed, declared_assets)

    results.append(run_control("negative: wrong hash declared in the bundle", False, wrong_declared_hash, ("declares exactly",)))
    results.append(run_control("negative: asset name missing from the declaration", False, missing_declaration, ("declares exactly",)))
    results.append(run_control("negative: unknown asset name declared", False, unknown_declaration, ("declares exactly",)))
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
    expected = json.loads((corpus_dir / "expected.json").read_text(encoding="utf-8"))
    bundles = {k: json.loads((corpus_dir / v["file"]).read_text(encoding="utf-8")) for k, v in expected["bundles"].items()}
    return expected, bundles, corpus_dir
