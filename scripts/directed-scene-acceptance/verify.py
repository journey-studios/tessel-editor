"""Independent verifiers of S10: pictures (camera masks, splats), exported audio and packages.

Nothing here knows how the editor renders, mixes or packages. A picture, a file or an archive goes in; the expected values
come from `corpus/expected.json` (written by `oracle.py`), the tolerances from `tolerances.json` (frozen before the first
result). Reports of the destination are compared with what is measured, never trusted.

`p` is any object with `.check(name, condition, detail="")` (`harness.Phase` in the proof, `Collector` in self-tests).
"""
import array
import hashlib
import json
import math
import pathlib
import shutil
import subprocess
import sys
import zipfile
from fractions import Fraction

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / "directed-scene-export-proof"))
import oracle  # noqa: E402
import export_verify as ev  # noqa: E402  (R10, read-only: container/timing rules and the BT.709 limited-range decoder)

FFMPEG, FFPROBE = ev.FFMPEG, ev.FFPROBE
TOLERANCES = json.loads((HERE / "tolerances.json").read_text())
Collector = ev.Collector


# ---------------------------------------------------------------------------------- pictures
def png_to_rgb(path, width, height):
    run = subprocess.run([FFMPEG, "-v", "error", "-i", str(path), "-f", "rawvideo", "-pix_fmt", "rgb24", "-"], capture_output=True)
    if run.returncode != 0 or len(run.stdout) != width * height * 3:
        raise RuntimeError(f"cannot decode {path} as {width}x{height}: {run.stderr.decode()[-200:]}")
    return run.stdout


def pixel(rgb, width, x, y):
    base = (y * width + x) * 3
    return rgb[base], rgb[base + 1], rgb[base + 2]


def distance(a, b):
    return max(abs(int(a[0]) - int(b[0])), abs(int(a[1]) - int(b[1])), abs(int(a[2]) - int(b[2])))


def _bilinear_distance(rgb, width, height, background, x, y):
    x0, y0 = int(math.floor(x)), int(math.floor(y))
    if x0 < 0 or y0 < 0 or x0 + 1 >= width or y0 + 1 >= height:
        return 0.0
    fx, fy = x - x0, y - y0
    d = [[distance(pixel(rgb, width, x0 + i, y0 + j), background) for i in (0, 1)] for j in (0, 1)]
    return d[0][0] * (1 - fx) * (1 - fy) + d[0][1] * fx * (1 - fy) + d[1][0] * (1 - fx) * fy + d[1][1] * fx * fy


# ---------------------------------------------------------------------------------- camera
def _blend_residual(px, background, color):
    """Distance from `px` to the nearest point of the segment background -> marker colour (an anti-aliased edge of
    a flat marker over the flat background lies on that segment)."""
    d = [c - b for c, b in zip(color, background)]
    norm = sum(x * x for x in d)
    alpha = max(0.0, min(1.0, sum((p - b) * x for p, b, x in zip(px, background, d)) / norm))
    return math.sqrt(sum((p - (b + alpha * x)) ** 2 for p, b, x in zip(px, background, d)))


def marker_masks(rgb, width, height, palette, threshold):
    """Detected mask of every marker: a pixel is a marker pixel when it is `threshold` or more away from the flat
    background, and belongs to the marker whose background-blend line it is nearest to (so the anti-aliased edge of
    one marker is never read as another marker's colour)."""
    background = palette["background"]
    colors = {name: tuple(c) for name, c in palette["markers"].items()}
    masks = {name: set() for name in colors}
    for y in range(height):
        base = y * width * 3
        for x in range(width):
            px = (rgb[base + x * 3], rgb[base + x * 3 + 1], rgb[base + x * 3 + 2])
            if distance(px, background) < threshold:
                continue
            masks[min(colors, key=lambda n: _blend_residual(px, background, colors[n]))].add((x, y))
    return masks


def judge_camera_frame(p, rgb, width, height, exact_masks, case, palette, video=False, label=""):
    """Compares the detected marker masks with the model's exact ray-cast masks of one frame (`exact_masks` from
    `oracle.marker_mask`) and with the model's centroids. Markers with fewer than 300 model pixels are partly out of
    the picture and are recorded as not judged: no frozen tolerance covers them."""
    tol = TOLERANCES["camera"]
    centroid_tol = tol["videoMaskCentroidPx" if video else "maskCentroidPx"]
    iou_min = tol["videoMaskIouMin" if video else "maskIouMin"]
    color_tol = tol["videoInteriorColorDelta" if video else "interiorColorDelta"]
    masks = marker_masks(rgb, width, height, palette, tol["markerMask"]["channelDistanceFromBackgroundMin"])
    metrics = {}
    for name, expected in case["markers"].items():
        if expected["area"] < 300:
            metrics[name] = {"expectedArea": expected["area"], "judged": False}
            continue
        found = oracle.mask_stats(masks[name])
        if not p.check(f"{label}{name}: detected", found is not None, "no pixel of this colour"):
            continue
        offset = math.hypot(found["x"] - expected["x"], found["y"] - expected["y"])
        p.check(f"{label}{name}: centroid within {centroid_tol} px of the model ({offset:.3f})", offset <= centroid_tol,
                f"got ({found['x']:.2f},{found['y']:.2f}) want ({expected['x']},{expected['y']})")
        overlap = oracle.iou(masks[name], exact_masks[name])
        p.check(f"{label}{name}: mask IoU >= {iou_min} against the model's ray-cast mask ({overlap:.3f})", overlap >= iou_min, overlap)
        cx, cy = int(round(expected["x"])), int(round(expected["y"]))
        if (cx, cy) in exact_masks[name]:
            got = pixel(rgb, width, cx, cy)
            p.check(f"{label}{name}: interior colour at the model centroid within {color_tol}",
                    distance(got, palette["markers"][name]) <= color_tol, got)
        metrics[name] = {"offsetPx": round(offset, 3), "iou": round(overlap, 4), "judged": True}
    return metrics


def exact_masks_for(bundle, case):
    """The model's exact masks of every marker of a stored camera case (ray-cast by `oracle`)."""
    state = oracle.frame_state(bundle, case["frame"])
    nodes = oracle.nodes_of(bundle)
    width, height = case["size"]
    return {name: oracle.marker_mask(state, nodes[name], width, height) for name in case["markers"]}


# ---------------------------------------------------------------------------------- splats
def _region(rgb, width, height, background, entry, coverage):
    """Pixels around an expected centre that are `coverage` or more away from the background, with their distance."""
    radius = 3.5 * entry["sigmaPx"] + 3
    x0, x1 = max(0, int(entry["x"] - radius)), min(width, int(entry["x"] + radius) + 1)
    y0, y1 = max(0, int(entry["y"] - radius)), min(height, int(entry["y"] + radius) + 1)
    out = []
    for y in range(y0, y1):
        for x in range(x0, x1):
            if math.hypot(x - entry["x"], y - entry["y"]) > radius:
                continue
            d = distance(pixel(rgb, width, x, y), background)
            if d >= coverage:
                out.append((x, y, d))
    return out


def measure_splat(rgb, width, height, background, entry, coverage):
    region = _region(rgb, width, height, background, entry, coverage)
    if not region:
        return None
    total = sum(d for _, _, d in region)
    cx, cy = sum(x * d for x, _, d in region) / total, sum(y * d for _, y, d in region) / total
    peak = max(distance(pixel(rgb, width, int(round(cx)) + dx, int(round(cy)) + dy), background)
               for dx in (-1, 0, 1) for dy in (-1, 0, 1))
    radii = []
    for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
        r = 0.0
        while r < 4 * entry["sigmaPx"] + 6:
            if _bilinear_distance(rgb, width, height, background, cx + dx * r, cy + dy * r) < 0.5 * peak:
                break
            r += 0.25
        radii.append(r)
    return {"x": cx, "y": cy, "peak": peak, "r50": sum(radii) / 4, "pixels": len(region)}


def judge_splat_frame(p, rgb, width, height, case, palette, video=False, label="", only=None):
    """Judges one frame against the model: background, centroid and size of the isolated splats, centre colours, order."""
    tol = TOLERANCES["splat"]
    background = tuple(palette["background"])
    centroid_tol = tol["videoCentroidPx" if video else "centroidPx"]
    color_tol = tol["videoCenterColorDelta" if video else "centerColorDelta"]
    r50_tol = tol["videoHalfIntensityRadius" if video else "halfIntensityRadius"]
    coverage = tol["coverageChannelDistanceFromBackgroundMin"]
    metrics = {}
    for px, py in case["backgroundPoints"]:
        p.check(f"{label}background at ({px},{py}) is the flat background within {tol['backgroundDelta'] + (6 if video else 0)}",
                distance(pixel(rgb, width, px, py), background) <= tol["backgroundDelta"] + (6 if video else 0),
                pixel(rgb, width, px, py))
    by_key = {(s["node"], s["index"]): s for s in case["splats"]}
    for entry in case["splats"]:
        key = f"{entry['node']}[{entry['index']}]"
        if only and entry["node"] not in only:
            continue
        if entry["asset"] == "sp-order":
            continue  # judged below as a pair
        found = measure_splat(rgb, width, height, background, entry, coverage)
        if not p.check(f"{label}{key}: a splat is drawn near ({entry['x']:.1f},{entry['y']:.1f})", found is not None, "empty region"):
            continue
        offset = math.hypot(found["x"] - entry["x"], found["y"] - entry["y"])
        p.check(f"{label}{key}: centroid within {centroid_tol} px of the projected centre ({offset:.2f})", offset <= centroid_tol,
                f"measured ({found['x']:.2f},{found['y']:.2f}) expected ({entry['x']},{entry['y']})")
        cx, cy = int(round(entry["x"])), int(round(entry["y"]))
        center = pixel(rgb, width, cx, cy)
        p.check(f"{label}{key}: centre colour within {color_tol} of the splat colour", distance(center, entry["color"]) <= color_tol,
                f"{center} vs {entry['color']}")
        allowed = r50_tol["relative"] * entry["r50Px"] + r50_tol["absolutePx"]
        p.check(f"{label}{key}: half-intensity radius {found['r50']:.1f} px vs model {entry['r50Px']:.1f} (+-{allowed:.1f})",
                abs(found["r50"] - entry["r50Px"]) <= allowed, found["r50"])
        metrics[key] = {"offsetPx": round(offset, 3), "r50": round(found["r50"], 3), "r50Expected": entry["r50Px"]}
    front, back = by_key.get(("s-order", 0)), by_key.get(("s-order", 1))
    if front and back and (not only or "s-order" in only):
        for who, at in (("front centre", front), ("back centre", back)):
            got = pixel(rgb, width, int(round(at["x"])), int(round(at["y"])))
            p.check(f"{label}s-order {who}: the front splat's colour wins within {tol['overlapFrontColorDelta']}",
                    distance(got, front["color"]) <= tol["overlapFrontColorDelta"], f"{got} vs front {front['color']}")
            p.check(f"{label}s-order {who}: far (>= {tol['overlapBackColorMinDistance']}) from the back splat's colour",
                    distance(got, back["color"]) >= tol["overlapBackColorMinDistance"], f"{got} vs back {back['color']}")
    return metrics


# ---------------------------------------------------------------------------------- audio
def audio_streams(probe):
    return [s for s in probe["streams"] if s["codec_type"] == "audio"]


def decode_audio(path, rate=48000):
    """Decodes the first audio stream to float samples per channel at `rate` (ffmpeg, no filters but the resampler)."""
    probe = ev.ffprobe(path)
    streams = audio_streams(probe)
    if not streams:
        raise RuntimeError("no audio stream")
    channels = int(streams[0]["channels"])
    run = subprocess.run([FFMPEG, "-v", "error", "-i", str(path), "-map", "0:a:0", "-vn", "-f", "f32le", "-ar", str(rate), "-"],
                         capture_output=True)
    if run.returncode != 0:
        raise RuntimeError(run.stderr.decode()[-200:])
    flat = array.array("f")
    flat.frombytes(run.stdout[: len(run.stdout) // 4 * 4])
    return [flat[c::channels].tolist() for c in range(channels)], probe


def _rms(samples):
    return math.sqrt(sum(x * x for x in samples) / len(samples)) if samples else 0.0


def _envelope(samples, window=96, hop=48):
    """RMS envelope: one value per `hop` samples over `window` samples (2 ms / 1 ms at 48 kHz)."""
    out = []
    for start in range(0, max(0, len(samples) - window), hop):
        chunk = samples[start:start + window]
        out.append(math.sqrt(sum(x * x for x in chunk) / window))
    return out


def correlation(a, b):
    n = min(len(a), len(b))
    dot = sum(a[i] * b[i] for i in range(n))
    na, nb = sum(a[i] * a[i] for i in range(n)), sum(b[i] * b[i] for i in range(n))
    return dot / math.sqrt(na * nb) if na > 0 and nb > 0 else 0.0


def envelope_lag(expected_env, got_env, max_hops):
    """Lag (in hops, positive = the file is late) that best aligns the two envelopes."""
    best, best_lag = -2.0, 0
    for lag in range(-max_hops, max_hops + 1):
        if lag >= 0:
            a, b = expected_env[: len(expected_env) - lag], got_env[lag:]
        else:
            a, b = expected_env[-lag:], got_env[: len(got_env) + lag]
        value = correlation(a, b)
        if value > best:
            best, best_lag = value, lag
    return best_lag, best


def _onset(env, hop_seconds, kind, near, threshold):
    """Time of the first (`on`) or last (`off`) envelope value crossing `threshold` within 80 ms of `near`."""
    lo, hi = max(0, int((near - 0.08) / hop_seconds)), min(len(env), int((near + 0.08) / hop_seconds) + 1)
    crossings = [i for i in range(lo, hi) if env[i] >= threshold]
    if not crossings:
        return None
    # the envelope window starts at index*hop and spans 2 hops: its centre is half a window later
    return (crossings[0] if kind == "on" else crossings[-1] + 1) * hop_seconds + (0.001 if kind == "on" else 0.0)


def expected_samples(case, bundle):
    """Analytic PCM of the case (floats) from the corpus's tone table: never read from the destination."""
    table = oracle.ToneTable(case_mix_segments(), 8)
    fps = Fraction(bundle["fps"]["numerator"], bundle["fps"]["denominator"])
    start = Fraction(bundle["audio"]["editStartSeconds"]).limit_denominator(1000)
    values, count, _ = oracle.expected_window(table, fps, start, case["from"], case["frames"], 48000)
    return values


_MIX = []


def case_mix_segments():
    """The tone table of the corpus's mix, read from expected.json (written by corpus.py from the same constants)."""
    if not _MIX:
        data = json.loads((HERE / "corpus/expected.json").read_text())["audio"]["mix"]["segments"]
        _MIX.extend((Fraction(a), Fraction(b), hz, amp) for a, b, hz, amp in data)
    return list(_MIX)


def judge_audio(p, path, case, bundle, label="", video_seconds=None, channels_expected=None, gain_db=0.0):
    """Measures the exported file's audio against the analytic expectation of `case`. Returns metrics."""
    tol = TOLERANCES["audio"]
    try:
        channels, probe = decode_audio(path, tol["outputSampleRate"])
    except RuntimeError as error:
        p.check(f"{label}audio: the file has a decodable audio stream", False, error)
        return {}
    streams = audio_streams(probe)
    p.check(f"{label}audio: exactly {tol['streamCountExact']} audio stream", len(streams) == tol["streamCountExact"], len(streams))
    p.check(f"{label}audio: {len(channels)} channel(s) are accepted", len(channels) in tol["channelsAccepted"], len(channels))
    p.check(f"{label}audio: sample rate {tol['outputSampleRate']}", int(streams[0]["sample_rate"]) == tol["outputSampleRate"],
            streams[0]["sample_rate"])
    start_time = float(streams[0].get("start_time") or 0)
    p.check(f"{label}audio: start_time within {tol['startTimeAbsSeconds']} s of zero ({start_time:.4f})",
            abs(start_time) <= tol["startTimeAbsSeconds"], start_time)
    audio_seconds = len(channels[0]) / tol["outputSampleRate"]
    reference = video_seconds if video_seconds is not None else case["durationSecondsFloat"]
    p.check(f"{label}audio: duration {audio_seconds:.4f} s within {tol['durationVsVideoSeconds']} s of {reference:.4f} s",
            abs(audio_seconds - reference) <= tol["durationVsVideoSeconds"], audio_seconds)
    expected = expected_samples(case, bundle)
    rate = tol["outputSampleRate"]
    guard = tol["transitionGuardSeconds"]
    metrics = {"channels": []}
    for index, got in enumerate(channels):
        tag = f"{label}audio ch{index}"
        n = min(len(got), len(expected))
        p.check(f"{tag}: at least {len(expected) - int(tol['durationVsVideoSeconds'] * rate)} samples", len(got) >= len(expected) - int(tol["durationVsVideoSeconds"] * rate), len(got))
        c0 = correlation(expected[:n], got[:n])
        p.check(f"{tag}: sample-level correlation with the analytic signal >= {tol['correlationMin']} ({c0:.4f})", c0 >= tol["correlationMin"], c0)
        env_e, env_g = _envelope(expected), _envelope(got)
        hops = int(tol["lagSearchSeconds"] / 0.001)
        lag_hops, lag_corr = envelope_lag(env_e, env_g, hops)
        lag_samples = lag_hops * 48
        p.check(f"{tag}: envelope lag {lag_samples:+d} samples within {tol['lagToleranceSamples']}", abs(lag_samples) <= tol["lagToleranceSamples"], lag_samples)
        # gain and tone content over windows that sit entirely inside one segment (guarded away from the edges)
        windows = int(tol["toneWindowSeconds"] * rate)
        in_tone, weak = [], []
        candidates = sorted({seg["hz"] for seg in case["segments"]} | {440, 880, 1320})
        for seg in case["segments"]:
            first = int((seg["from"] + guard) * rate)
            last = int((seg["to"] - guard) * rate)
            for begin in range(first, last - windows + 1, windows):
                chunk = got[begin:begin + windows]
                if len(chunk) < windows:
                    break
                in_tone.append((begin, seg))
                target = oracle.goertzel_power(chunk, seg["hz"], rate)
                others = max(oracle.goertzel_power(chunk, hz, rate) for hz in candidates if hz != seg["hz"])
                ratio = target / others if others > 0 else float("inf")
                if ratio < tol["toneDominanceRatioMin"]:
                    weak.append(f"{begin / rate:.2f}s expects {seg['hz']} Hz (ratio {ratio:.1f})")
        p.check(f"{tag}: {len(in_tone)} tone window(s) judged, each dominated by its expected frequency (ratio >= {tol['toneDominanceRatioMin']})",
                len(in_tone) > 0 and not weak, weak[:3] or len(in_tone))
        if in_tone:
            got_rms = _rms([x for begin, _ in in_tone for x in got[begin:begin + windows]])
            want_rms = _rms([x for begin, _ in in_tone for x in expected[begin:begin + windows]])
            db = 20 * math.log10(got_rms / want_rms) if got_rms > 0 and want_rms > 0 else -120.0
            p.check(f"{tag}: gain {db:+.2f} dB within +-{tol['gainDbTolerance']} dB of the expected {gain_db:+.2f} dB", abs(db - gain_db) <= tol["gainDbTolerance"], db)
        else:
            db = None
        gaps = _silent_ranges(case, rate, guard, windows)
        worst = max((_rms(got[a:b]) for a, b in gaps), default=0.0)
        p.check(f"{tag}: {len(gaps)} silent range(s) stay below RMS {tol['silenceRmsMax']} (worst {worst:.5f})", worst <= tol["silenceRmsMax"], worst)
        onset_errors = []
        for transition in case["transitions"]:
            amp = _amplitude_near(case, transition)
            found = _onset(env_g, 0.001, transition["kind"], transition["exportSecondsFloat"], 0.35 * amp / math.sqrt(2))
            if found is None:
                p.check(f"{tag}: {transition['kind']} edge near {transition['exportSecondsFloat']:.3f}s exists in the file", False, "no crossing")
                continue
            error = found - transition["exportSecondsFloat"]
            onset_errors.append(error)
            p.check(f"{tag}: {transition['kind']} edge at {found:.4f}s vs {transition['exportSecondsFloat']:.4f}s (error {error * 1000:+.2f} ms, limit {tol['onsetToleranceSeconds'] * 1000:.0f} ms)",
                    abs(error) <= tol["onsetToleranceSeconds"], error)
        metrics["channels"].append({"correlation": round(c0, 5), "lagSamples": lag_samples, "gainDb": None if db is None else round(db, 3),
                                    "silentWorstRms": round(worst, 6), "onsetErrorsMs": [round(e * 1000, 3) for e in onset_errors]})
    return metrics


def _silent_ranges(case, rate, guard, minimum):
    """Sample ranges of the export that the oracle says are silent, shrunk by the guard around every tone."""
    total = case["outputSamples"]
    busy = sorted((seg["from"], seg["to"]) for seg in case["segments"])
    ranges, cursor = [], 0.0
    for a, b in busy + [(case["durationSecondsFloat"], case["durationSecondsFloat"])]:
        if a - cursor > 2 * guard:
            lo, hi = int((cursor + (guard if cursor > 0 else 0)) * rate), int((a - guard) * rate)
            if hi - lo >= minimum // 2:
                ranges.append((lo, min(hi, total)))
        cursor = max(cursor, b)
    return ranges


def _amplitude_near(case, transition):
    near = transition["exportSecondsFloat"]
    return min(case["segments"], key=lambda s: min(abs(s["from"] - near), abs(s["to"] - near)))["amplitude"]


# ---------------------------------------------------------------------------------- controls support
COLOR_TAGS = ["-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709", "-color_range", "tv"]


def write_av_file(path, samples_by_channel, frames, fps_text, size=(64, 36), rate=48000, audio_codec="aac"):
    """A real MP4 with a black picture of `frames` frames at the exact rational `fps_text` and the given PCM as its
    audio. This is how the controls make exports that are wrong in one known way."""
    channels = len(samples_by_channel)
    length = max(len(c) for c in samples_by_channel) if channels else 0
    flat = array.array("f", [0.0]) * (length * channels)
    for index, channel in enumerate(samples_by_channel):
        flat[index::channels] = array.array("f", channel + [0.0] * (length - len(channel)))
    cmd = [FFMPEG, "-v", "error", "-y", "-f", "lavfi", "-i", f"color=c=black:s={size[0]}x{size[1]}:r={fps_text}",
           "-f", "f32le", "-ar", str(rate), "-ac", str(channels), "-i", "pipe:0", "-frames:v", str(frames), "-map", "0:v", "-map", "1:a",
           "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "18", *COLOR_TAGS, "-c:a", audio_codec, "-b:a", "192k", "-movflags", "+faststart", str(path)]
    run = subprocess.run(cmd, input=flat.tobytes(), capture_output=True)
    if run.returncode != 0:
        raise RuntimeError(run.stderr.decode()[-300:])


def write_video_only(path, frames, fps_text, size=(64, 36)):
    cmd = [FFMPEG, "-v", "error", "-y", "-f", "lavfi", "-i", f"color=c=black:s={size[0]}x{size[1]}:r={fps_text}", "-frames:v", str(frames),
           "-c:v", "libx264", "-pix_fmt", "yuv420p", *COLOR_TAGS, str(path)]
    run = subprocess.run(cmd, capture_output=True)
    if run.returncode != 0:
        raise RuntimeError(run.stderr.decode()[-300:])


# ---------------------------------------------------------------------------------- package
def sha256_bytes(data):
    return hashlib.sha256(data).hexdigest()


def read_package(path):
    """`{entry name: sha256}` of a package read by a reader that is not the editor's: a plain directory walk or
    Python's zipfile. A package that neither reader can open yields None."""
    path = pathlib.Path(path)
    if path.is_dir():
        return {str(f.relative_to(path)): sha256_bytes(f.read_bytes()) for f in sorted(path.rglob("*")) if f.is_file()}
    if zipfile.is_zipfile(path):
        with zipfile.ZipFile(path) as archive:
            return {info.filename: sha256_bytes(archive.read(info)) for info in archive.infolist() if not info.is_dir()}
    return None


def judge_package(p, path, expected_assets, label=""):
    """Every corpus asset (by SHA-256) must be present in the package, byte for byte, found by content."""
    entries = read_package(path)
    if not p.check(f"{label}the package opens with an independent reader (zipfile / directory walk)", entries is not None, str(path)):
        return {}
    by_hash = {}
    for name, digest in entries.items():
        by_hash.setdefault(digest, []).append(name)
    missing = [name for name, info in expected_assets.items() if info["sha256"] not in by_hash]
    p.check(f"{label}all {len(expected_assets)} asset(s) are inside, byte-identical (SHA-256)", not missing, missing)
    return {"entries": len(entries), "missing": missing}


def tamper_zip(source, target, mutate):
    """Copies a zip into `target`, applying `mutate(info, data) -> data | None` (None drops the entry)."""
    with zipfile.ZipFile(source) as src, zipfile.ZipFile(target, "w", zipfile.ZIP_STORED) as dst:
        for info in src.infolist():
            data = mutate(info, src.read(info))
            if data is not None:
                dst.writestr(info.filename, data)


def have_ffmpeg():
    return bool(shutil.which("ffmpeg") or pathlib.Path(FFMPEG).exists())
