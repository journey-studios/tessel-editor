"""Independent verification of an exported directed-scene video (BL008, front R10).

Nothing here knows how the editor exports. A file goes in; ffprobe, ffmpeg and the independent
reference model (`scripts/directed-scene-proof/model.py`) judge it. The expected values come from
`export_corpus.py`, never from the destination's own report: where the export's result JSON is
compared, it is compared with what was measured in the file, so a report that lies fails.

`check` callers get a `p` with `.check(name, ok, detail)` (`harness.Phase` in the proof, `Collector`
in the self-tests).
"""
import hashlib
import json
import pathlib
import shutil
import subprocess
import sys
from fractions import Fraction

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent))
import export_corpus as corpus  # noqa: E402
import tonecheck  # noqa: E402  (scripts/tonecheck.py: Goertzel tone level, read-only reuse)

REF_W, REF_H = corpus.REF_W, corpus.REF_H
FFMPEG = shutil.which("ffmpeg") or "/opt/homebrew/bin/ffmpeg"
FFPROBE = shutil.which("ffprobe") or "/opt/homebrew/bin/ffprobe"
TOL = corpus.TOLERANCES


class Collector:
    """Silent `.check` sink for the harness self-tests."""

    def __init__(self):
        self.checks = []

    def check(self, name, condition, detail=""):
        self.checks.append((name, bool(condition), "" if condition else str(detail)[:300]))
        return bool(condition)

    def failed(self):
        return [name for name, ok, _ in self.checks if not ok]

    def passed(self):
        return not self.failed()


def tool_versions():
    def first_line(binary):
        out = subprocess.run([binary, "-version"], capture_output=True, text=True)
        return out.stdout.splitlines()[0] if out.stdout else "unavailable"
    return {"ffmpeg": first_line(FFMPEG), "ffprobe": first_line(FFPROBE)}


# ---------------------------------------------------------------------------------- ffprobe / ffmpeg
def _json(cmd):
    run = subprocess.run(cmd, capture_output=True, text=True)
    if run.returncode != 0:
        raise RuntimeError(run.stderr.strip()[-300:] or f"exit {run.returncode}")
    return json.loads(run.stdout)


def ffprobe(path):
    return _json([FFPROBE, "-v", "error", "-show_streams", "-show_format", "-of", "json", str(path)])


def packets(path):
    """Video packet pts (stream ticks, ascending) and the stream time base as a Fraction."""
    data = _json([FFPROBE, "-v", "error", "-select_streams", "v:0", "-show_entries",
                  "packet=pts:stream=time_base", "-of", "json", str(path)])
    num, den = data["streams"][0]["time_base"].split("/")
    return sorted(int(p["pts"]) for p in data["packets"] if "pts" in p), Fraction(int(num), int(den))


def count_frames(path):
    data = _json([FFPROBE, "-v", "error", "-count_frames", "-select_streams", "v:0", "-show_entries",
                  "stream=nb_read_frames", "-of", "json", str(path)])
    return int(data["streams"][0]["nb_read_frames"])


def decode(path, width, height, matrix="bt709"):
    """Yields each decoded frame as `width*height*3` RGB bytes. The YUV->RGB matrix and range are explicit:
    the contract under test is BT.709 limited range, so a decoder default must never decide it."""
    cmd = [FFMPEG, "-v", "error", "-i", str(path), "-map", "0:v:0",
           "-vf", f"scale=in_color_matrix={matrix}:in_range=tv:out_range=pc:flags=accurate_rnd+full_chroma_int,format=rgb24",
           "-f", "rawvideo", "-"]
    process = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    size = width * height * 3
    try:
        while True:
            frame = process.stdout.read(size)
            if len(frame) < size:
                break
            yield frame
    finally:
        process.stdout.close()
        process.kill()
        process.wait()
        process.stderr.close()


def rational(text):
    try:
        num, den = str(text).split("/")
        return Fraction(int(num), int(den)) if int(den) else None
    except (ValueError, ZeroDivisionError):
        return None


# ---------------------------------------------------------------------------------- container / timing
def verify_container(p, path, expect):
    """ffprobe-level contract. Returns (probe_json or None, metrics)."""
    metrics = {}
    try:
        probe = ffprobe(path)
        pts, base = packets(path)
        decoded = count_frames(path)
    except (RuntimeError, KeyError, ValueError, IndexError) as error:
        p.check("the file is readable by ffprobe/ffmpeg", False, error)
        return None, metrics
    p.check("the file is readable by ffprobe/ffmpeg", True)
    videos = [s for s in probe["streams"] if s["codec_type"] == "video"]
    audios = [s for s in probe["streams"] if s["codec_type"] == "audio"]
    if not p.check("exactly one video stream", len(videos) == 1, len(videos)):
        return probe, metrics
    video = videos[0]
    fps = expect["fps"]
    p.check("codec h264, pixel format yuv420p", (video["codec_name"], video.get("pix_fmt")) == ("h264", "yuv420p"),
            (video["codec_name"], video.get("pix_fmt")))
    p.check(f"resolution {expect['width']}x{expect['height']}",
            (video["width"], video["height"]) == (expect["width"], expect["height"]), (video["width"], video["height"]))
    p.check(f"frame count: {expect['frames']} frames decoded (not just declared)", decoded == expect["frames"], decoded)
    if video.get("nb_frames") not in (None, "N/A"):
        p.check("container nb_frames agrees with the decoded count", int(video["nb_frames"]) == decoded, video["nb_frames"])
    p.check(f"r_frame_rate is exactly {fps}", rational(video.get("r_frame_rate")) == fps, video.get("r_frame_rate"))
    p.check(f"avg_frame_rate is exactly {fps}", rational(video.get("avg_frame_rate")) == fps, video.get("avg_frame_rate"))
    p.check("colour declared: BT.709, limited range",
            (video.get("color_space"), video.get("color_range")) == ("bt709", "tv"),
            (video.get("color_space"), video.get("color_range")))
    period = 1 / fps
    if len(pts) == expect["frames"] and pts:
        times = [tick * base for tick in pts]
        tolerance = max(base, Fraction(1, 10 ** 6))
        drift = max(abs(t - (times[0] + index * period)) for index, t in enumerate(times))
        p.check("every pts sits on the exact frame grid i/fps (to one container tick)", drift <= tolerance,
                f"max drift {float(drift):.9f}s, tick {float(base):.9f}s")
        p.check("the first pts is within one frame of zero", abs(times[0]) <= period * TOL["timing"]["startOffsetFrames"], float(times[0]))
        duration = times[-1] - times[0] + period
        want = Fraction(expect["frames"]) / fps
        p.check(f"duration {want} s = frames * den/num (to 1 ms)", abs(duration - want) <= Fraction(1, 1000),
                f"{float(duration):.6f} vs {float(want):.6f}")
        metrics["duration_seconds"] = float(duration)
    else:
        p.check("one video packet pts per frame", False, f"{len(pts)} pts for {expect['frames']} frames")
    metrics["tick"] = f"{base.numerator}/{base.denominator}"
    metrics["bytes"] = pathlib.Path(path).stat().st_size
    metrics["audio_streams"] = len(audios)
    return probe, metrics


# ---------------------------------------------------------------------------------- pixels
def pixel_for(ref_x, ref_y, width, height):
    """Output pixel containing the centre of reference pixel (ref_x, ref_y)."""
    return int((ref_x + 0.5) / REF_W * width), int((ref_y + 0.5) / REF_H * height)


def read_pixel(frame, width, x, y):
    base = (y * width + x) * 3
    return [frame[base], frame[base + 1], frame[base + 2]]


def probe_colors(frame, width, height, probes):
    return [read_pixel(frame, width, *pixel_for(q["refX"], q["refY"], width, height)) for q in probes]


def max_probe_delta(probes, colors):
    return max((max(abs(a - b) for a, b in zip(got, q["color"])) for q, got in zip(probes, colors)), default=0)


def probe_mismatches(probes, colors, tolerance):
    return [f"{q['layer']}@({q['refX']},{q['refY']}): wanted {q['color']}, got {got}"
            for q, got in zip(probes, colors) if max(abs(a - b) for a, b in zip(got, q["color"])) > tolerance]


# Identity works on a dense 160x90 picture of the model, not on the 24 flat probes: flat probes are chosen to be
# insensitive to small motion, so frames a few reference pixels apart share them. The dense picture sees an
# edge move; pixels within one grid step of any layer/colour change are masked (filtering and 4:2:0 live there).
ID_W, ID_H = 160, 90
ID_WINDOW = 4  # compare with scene frames i-4..i+4: far enough to catch swaps and repeats, cheap enough for 48 frames
ID_APART_DELTA = 2 * TOL["videoPixel"]["maxChannelDelta"]  # a cell tells two frames apart only beyond this channel gap
ID_MIN_CELLS = 24  # cells (of 14400) two frames must differ in before their order is asserted
ID_AGREE = 0.75  # share of those cells where the decoded frame must be nearer to its own model frame
ID_MIN_JUDGED_FRAMES = 0.5  # share of frames with at least one judged pair; below it the identity check is vacuous


def _model_image_worker(job):
    bundle, frame = job
    tex = corpus.textures()
    state = corpus.model.frame_state(bundle, frame)
    signature = [corpus.model.visible_layer(state, ID_W, ID_H, x, y, tex) for y in range(ID_H) for x in range(ID_W)]
    rgb = bytearray()
    for hit in signature:
        rgb += bytes(hit["color"]) if hit else b"\x00\x00\x00"
    keys = [None if hit is None else (hit["layer"], tuple(hit["color"])) for hit in signature]
    mask = bytearray(ID_W * ID_H)
    for y in range(ID_H):
        for x in range(ID_W):
            here = keys[y * ID_W + x]
            around = [keys[ny * ID_W + nx] for nx, ny in ((x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1))
                      if 0 <= nx < ID_W and 0 <= ny < ID_H]
            mask[y * ID_W + x] = 1 if all(other == here for other in around) else 0
    return bytes(rgb), bytes(mask)


class Identity:
    """Which model frame does a decoded frame look like? Pictures come from the independent model, cached per run."""

    def __init__(self, bundle, first_frame, frames, cache=None):
        self.bundle, self.first, self.frames = bundle, first_frame, frames
        self.cache = cache if cache is not None else {}
        # Keyed by what the picture depends on, so main/ntsc/audio bundles (same geometry) share their images.
        self.cache_key = hashlib.sha256(json.dumps(corpus.geometry_of(bundle), sort_keys=True).encode()).hexdigest()

    def prepare(self, scene_frames):
        missing = [f for f in scene_frames if (self.cache_key, f) not in self.cache]
        if len(missing) > 3:
            import multiprocessing
            with multiprocessing.get_context("spawn").Pool(max(1, min(len(missing), multiprocessing.cpu_count() - 2))) as pool:
                images = pool.map(_model_image_worker, [(self.bundle, f) for f in missing])
        else:
            images = [_model_image_worker((self.bundle, f)) for f in missing]
        for frame, image in zip(missing, images):
            self.cache[(self.cache_key, frame)] = image

    def image(self, scene_frame):
        return self.cache[(self.cache_key, scene_frame)]

    def window(self, index):
        return [j for j in range(max(0, index - ID_WINDOW), min(self.frames, index + ID_WINDOW + 1))]

    @staticmethod
    def sample(frame, width, height):
        """The decoded frame at the 160x90 grid: the output pixel containing each grid cell's centre."""
        out = bytearray()
        for gy in range(ID_H):
            y = int((gy + 0.5) / ID_H * height)
            row = y * width * 3
            for gx in range(ID_W):
                x = int((gx + 0.5) / ID_W * width)
                out += frame[row + x * 3: row + x * 3 + 3]
        return bytes(out)

    def judge(self, index, decoded_rgb):
        """Pairwise test against each neighbour j in the window. Where the model itself keeps frames `index` and `j`
        apart (>= ID_MIN_CELLS unmasked cells differ by more than 2x the pixel tolerance), the decoded frame must
        agree with the model's frame `index` on at least ID_AGREE of those cells. A mean over the whole picture
        would dilute a moving hero into noise; counting only the cells that differ does not.
        Returns (failures, pairs_judged)."""
        own_rgb, own_mask = self.image(self.first + index)
        failures, judged = [], 0
        for j in self.window(index):
            if j == index:
                continue
            other_rgb, other_mask = self.image(self.first + j)
            apart = nearer_own = 0
            for cell in range(ID_W * ID_H):
                if not (own_mask[cell] and other_mask[cell]):
                    continue
                base = cell * 3
                gap = max(abs(own_rgb[base + c] - other_rgb[base + c]) for c in range(3))
                if gap <= ID_APART_DELTA:
                    continue
                apart += 1
                to_own = max(abs(decoded_rgb[base + c] - own_rgb[base + c]) for c in range(3))
                to_other = max(abs(decoded_rgb[base + c] - other_rgb[base + c]) for c in range(3))
                if to_own < to_other:
                    nearer_own += 1
            if apart < ID_MIN_CELLS:
                continue
            judged += 1
            if nearer_own < ID_AGREE * apart:
                failures.append(f"output frame {index} agrees with scene frame {self.first + j} on {apart - nearer_own} of "
                                f"{apart} cells where it differs from scene frame {self.first + index}")
        return failures, judged


def verify_pixels(p, path, expect, matrix="bt709", identity=True, image_cache=None):
    """Probe every decoded frame against the model; optionally prove frame order/identity."""
    width, height, frames = expect["width"], expect["height"], expect["frames"]
    tolerance = TOL["videoPixel"]["maxChannelDelta"]
    ident = None
    if identity and expect.get("bundle"):
        ident = Identity(expect["bundle"], expect["from"], frames, image_cache)
        ident.prepare(range(expect["from"], expect["from"] + frames))
    failed_probes, total, decoded_frames = [], 0, 0
    wrong_identity, unjudged, worst_delta = [], 0, 0
    for index, frame in enumerate(decode(path, width, height, matrix)):
        decoded_frames += 1
        if index >= frames:
            break
        probes = expect["probes"][str(expect["from"] + index)]
        colors = probe_colors(frame, width, height, probes)
        total += len(probes)
        worst_delta = max(worst_delta, max_probe_delta(probes, colors))
        bad = probe_mismatches(probes, colors, tolerance)
        failed_probes.extend(f"frame {expect['from'] + index} {line}" for line in bad)
        if ident is not None:
            failures, judged = ident.judge(index, ident.sample(frame, width, height))
            wrong_identity.extend(failures)
            unjudged += 0 if judged else 1
    p.check(f"{expect['frames']} frames decoded for the pixel probes", decoded_frames >= frames, decoded_frames)
    allowed = int(TOL["videoPixel"]["maxFailedProbeFraction"] * total)
    p.check(f"all {total} flat-interior probes within {tolerance} of the independent model (BT.709)",
            len(failed_probes) <= allowed, failed_probes[:3])
    if ident is not None:
        p.check("every frame is the model's frame at its index where the model tells neighbours apart (order, no duplicate, no skip)",
                not wrong_identity, wrong_identity[:3])
        p.check(f"identity is not vacuous: at least {int(ID_MIN_JUDGED_FRAMES * 100)}% of the frames were judged against a neighbour",
                unjudged <= frames * (1 - ID_MIN_JUDGED_FRAMES), f"{frames - unjudged} of {frames} judged")
    return {"probes": total, "failed_probes": len(failed_probes), "max_probe_delta": worst_delta,
            "identity_frames_without_a_judgeable_neighbour": unjudged}


# ---------------------------------------------------------------------------------- detail (original PNG, no proxy)
def contrast(frame, width, band, period):
    """(mean Y over columns that should be white - mean Y over columns that should be black) / 255 over the band."""
    rows = [band["y0"] + 20 + 40 * k for k in range(max(1, (band["y1"] - band["y0"] - 40) // 40))]
    white = black = count_white = count_black = 0
    for y in rows:
        base = y * width * 3
        for x in range(100, width - 100):
            value = frame[base + x * 3 + 1]
            if (x // period) % 2 == 0:
                white, count_white = white + value, count_white + 1
            else:
                black, count_black = black + value, count_black + 1
    return (white / count_white - black / count_black) / 255


def detail_frame_metrics(frame, width, height):
    """What one decoded/rendered RGB frame of the detail scene says about the original PNG."""
    spec = corpus.detail_expectation()
    contrasts = {f"{period}px": contrast(frame, width, spec["bands"][f"stripes{period}"], period) for period in (1, 2, 4)}
    colors = spec["bands"]["colors"]
    block_delta = 0
    for block in spec["colorBlocks"]:
        centre = read_pixel(frame, width, (block["x0"] + block["x1"]) // 2, (colors["y0"] + colors["y1"]) // 2)
        block_delta = max(block_delta, max(abs(a - b) for a, b in zip(centre, block["color"])))
    size = spec["markers"]["topLeft"]["size"]
    corners = [(read_pixel(frame, width, size // 2, size // 2), spec["markers"]["topLeft"]["color"]),
               (read_pixel(frame, width, width - size // 2 - 1, height - size // 2 - 1), spec["markers"]["bottomRight"]["color"])]
    marker_delta = max(max(abs(a - b) for a, b in zip(got, want)) for got, want in corners)
    return {"contrast": contrasts, "block_delta": block_delta, "marker_delta": marker_delta}


def verify_detail(p, path, expect, matrix="bt709"):
    width, height = expect["width"], expect["height"]
    rules = TOL["detail"]
    worst = {"1px": 1.0, "2px": 1.0, "4px": 1.0}
    block_delta = marker_delta = decoded_frames = 0
    for frame in decode(path, width, height, matrix):
        decoded_frames += 1
        metrics = detail_frame_metrics(frame, width, height)
        for name, value in metrics["contrast"].items():
            worst[name] = min(worst[name], value)
        block_delta = max(block_delta, metrics["block_delta"])
        marker_delta = max(marker_delta, metrics["marker_delta"])
    p.check("every frame of the detail export was decoded", decoded_frames == expect["frames"], decoded_frames)
    p.check(f"1 px stripes survive: contrast >= {rules['minRetention1px']} of the original (a proxy would not reach 0.2)",
            worst["1px"] >= rules["minRetention1px"], f"{worst['1px']:.3f}")
    p.check(f"2 px stripes survive: contrast >= {rules['minRetention2px']}", worst["2px"] >= rules["minRetention2px"], f"{worst['2px']:.3f}")
    p.check(f"4 px stripes survive: contrast >= {rules['minRetention4px']}", worst["4px"] >= rules["minRetention4px"], f"{worst['4px']:.3f}")
    p.check(f"the 8 colour blocks of the original PNG are within {rules['maxBlockDelta']} (BT.709)",
            block_delta <= rules["maxBlockDelta"], block_delta)
    p.check("orientation/offset markers (red top-left, blue bottom-right) are where the original has them",
            marker_delta <= rules["maxBlockDelta"], marker_delta)
    return {"contrast": {k: round(x, 4) for k, x in worst.items()}, "block_delta": block_delta, "marker_delta": marker_delta}


# ---------------------------------------------------------------------------------- audio policy
def verify_no_audio(p, probe, result=None):
    audios = [s for s in probe["streams"] if s["codec_type"] == "audio"]
    p.check("no audio stream: this directed scene names no audio mix", not audios, [a["codec_name"] for a in audios])
    claimed = (result or {}).get("audio")
    enabled = (claimed.get("enabled") or claimed.get("included")) if isinstance(claimed, dict) else bool(claimed)
    p.check("the result does not claim audio either", not enabled, claimed)


def verify_mixed_audio(p, path, probe, video_seconds):
    """The bundle's mix is two tones (440 Hz then 880 Hz). The export starts at `from`, so the file must
    hold exactly that window of the mix: tone and position, measured by Goertzel on decoded samples."""
    audios = [s for s in probe["streams"] if s["codec_type"] == "audio"]
    if not p.check("one audio stream carries the directed mix", len(audios) == 1, len(audios)):
        return {}
    first = {f: tonecheck.tone_level(str(path), 0.05, 0.4, f) for f in (440, 880)}
    second = {f: tonecheck.tone_level(str(path), 0.55, 0.4, f) for f in (440, 880)}
    p.check("first half of the file holds the 440 Hz part of the mix, not the 880 Hz part",
            first[440] > 0.05 and first[880] < first[440] / 10, first)
    p.check("second half holds the 880 Hz part, not the 440 Hz part",
            second[880] > 0.05 and second[440] < second[880] / 10, second)
    seconds = float(audios[0].get("duration") or 0)
    p.check("audio duration equals the video duration (+-50 ms)", abs(seconds - video_seconds) <= 0.05, f"{seconds} vs {video_seconds}")
    return {"first": {k: round(v, 4) for k, v in first.items()}, "second": {k: round(v, 4) for k, v in second.items()}}


# ---------------------------------------------------------------------------------- one entry point
def expectation(case_id, expected, bundle_json=None, matrix_probes=True):
    """Builds the `expect` dict of a corpus case (from `expected.json`)."""
    case = expected["cases"][case_id]
    out = {"fps": rational(case["fps"]), "frames": case["frames"], "from": case["from"],
           "width": case["output"][0], "height": case["output"][1], "kind": case["kind"], "bundle": bundle_json}
    if case["kind"] == "scene":
        out["probes"] = {str(f): expected["geometry"]["frameProbes"][str(f)]
                         for f in range(case["from"], case["from"] + case["frames"])}
    return out


def verify_export(p, path, expect, matrix="bt709", identity=True, audio="none", result=None, image_cache=None):
    """Runs the container checks, then the pixel or detail checks, then the audio policy. Returns metrics."""
    probe, metrics = verify_container(p, path, expect)
    if probe is None:
        return metrics
    if expect["kind"] == "detail":
        metrics["detail"] = verify_detail(p, path, expect, matrix)
    else:
        metrics["pixels"] = verify_pixels(p, path, expect, matrix, identity, image_cache)
    if audio == "none":
        verify_no_audio(p, probe, result)
    elif audio == "mix":
        metrics["audio"] = verify_mixed_audio(p, path, probe, metrics.get("duration_seconds", 0.0))
    return metrics
