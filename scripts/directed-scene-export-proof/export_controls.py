"""Positive and negative controls of the export verifier (front R10).

A verifier that cannot fail proves nothing. These controls build REFERENCE videos from frames the
independent model draws (`model.py`, a pinhole ray-caster written without reading the Rust code) and from the
corpus' own PNG, encode them with ffmpeg directly (no editor involved), and require:

  * the references pass every check (so a later failure of the real export is the export's, not the harness'),
  * each deliberate defect (wrong frame count, wrong or integer-rounded fps, duplicated or swapped frame,
    black frame, BT.601 decode, wrong size, missing colour tags, truncated file, extra/silent/shifted audio,
    a proxy-resolution original, a flipped picture) fails the check that exists for it.

Nothing here touches `tessel-render`, `tessel-headless` or the app: it is evidence about the harness
only, never about the export operation (docs/migration/scene-export-proof.md, "What each result proves").
"""
import json
import multiprocessing
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import export_corpus as corpus  # noqa: E402
import export_verify as v  # noqa: E402

SYNTH_W, SYNTH_H = 320, 180
SCENE_FROM, SCENE_FRAMES = 44, 8  # crosses the shot cut at 48
DETAIL_FRAMES = 6


def _render_worker(job):
    bundle_json, frame, width, height = job
    state = corpus.model.frame_state(bundle_json, frame)
    tex = corpus.textures()
    out = bytearray(width * height * 3)
    index = 0
    for py in range(height):
        for px in range(width):
            hit = corpus.model.visible_layer(state, width, height, px, py, tex)
            color = hit["color"] if hit else (0, 0, 0)
            out[index:index + 3] = bytes(color)
            index += 3
    return bytes(out)


def render_reference_frames(bundle_json, first, count, width=SYNTH_W, height=SYNTH_H):
    """The model's picture of frames [first, first+count): pure Python, parallel over frames."""
    jobs = [(bundle_json, first + i, width, height) for i in range(count)]
    with multiprocessing.get_context("spawn").Pool(min(count, max(1, multiprocessing.cpu_count() - 2))) as pool:
        return pool.map(_render_worker, jobs)


COLOR_TAGS = ["-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709", "-color_range", "tv"]
RGB_TO_YUV = "scale=out_color_matrix=bt709:out_range=tv:flags=accurate_rnd+full_chroma_int,format=yuv420p"


UNTAGGED = "setparams=colorspace=unknown:color_primaries=unknown:color_trc=unknown:range=unknown"


def encode_frames(frames, path, width, height, fps, tags=True, audio=None, crf=18):
    """Feeds raw RGB frames to libx264 (the same codec family and CRF the presets ask for)."""
    cmd = [v.FFMPEG, "-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{width}x{height}",
           "-framerate", fps, "-i", "-"]
    if audio:
        cmd += ["-i", str(audio)]
    cmd += ["-vf", RGB_TO_YUV if tags else f"{RGB_TO_YUV},{UNTAGGED}", "-c:v", "libx264", "-crf", str(crf), "-preset", "fast"]
    if tags:
        cmd += COLOR_TAGS
    if audio:
        cmd += ["-c:a", "aac", "-shortest"]
    cmd += ["-r", fps, "-movflags", "+faststart", str(path)]
    run = subprocess.run(cmd, input=b"".join(frames), capture_output=True)
    if run.returncode != 0:
        raise RuntimeError(run.stderr.decode()[-300:])
    return path


def ffmpeg(*args):
    run = subprocess.run([v.FFMPEG, "-v", "error", "-y", *args], capture_output=True)
    if run.returncode != 0:
        raise RuntimeError(run.stderr.decode()[-300:])


def scene_expect(expected, bundle_json, fps_text, width, height, frames=SCENE_FRAMES, first=SCENE_FROM):
    return {"fps": v.rational(fps_text), "frames": frames, "from": first, "width": width, "height": height,
            "kind": "scene", "bundle": bundle_json,
            "probes": {str(f): expected["geometry"]["frameProbes"][str(f)] for f in range(first, first + frames)}}


def outcome(name, should_pass, expect_failed, collector):
    failed = collector.failed()
    if should_pass:
        ok = not failed
    else:
        ok = bool(failed) and any(any(marker in name_ for name_ in failed) for marker in expect_failed)
    return {"control": name, "should_pass": should_pass, "ok": ok, "failed_checks": failed,
            "expected_failure_markers": expect_failed}


def run_scene_controls(work, expected, corpus_dir, log=print):
    """Reference scene videos and their mutations. `work` is an own directory."""
    work = pathlib.Path(work)
    work.mkdir(parents=True, exist_ok=True)
    bundle = json.loads((pathlib.Path(corpus_dir) / "bundle.main.json").read_text())
    log(f"  rendering {SCENE_FRAMES} reference frames with the model ({SYNTH_W}x{SYNTH_H}) ...")
    frames = render_reference_frames(bundle, SCENE_FROM, SCENE_FRAMES)
    results = []
    images = {}  # model pictures are drawn once and shared by every control

    def run(name, path, expect, should_pass, markers=(), **options):
        collector = v.Collector()
        v.verify_export(collector, path, expect, image_cache=images, **options)
        results.append(outcome(name, should_pass, markers, collector))
        log(f"  {'ok  ' if results[-1]['ok'] else 'WRONG'} {name}")

    ref = encode_frames(frames, work / "ref24.mp4", SYNTH_W, SYNTH_H, "24/1")
    expect24 = scene_expect(expected, bundle, "24/1", SYNTH_W, SYNTH_H)
    run("reference 24/1 passes every check", ref, expect24, True)
    ntsc = encode_frames(frames, work / "refntsc.mp4", SYNTH_W, SYNTH_H, "30000/1001")
    run("reference 30000/1001 (NTSC) passes: exact rational fps, duration and pts", ntsc,
        scene_expect(expected, bundle, "30000/1001", SYNTH_W, SYNTH_H), True)

    run("a file with 7 of 8 frames fails the frame count",
        encode_frames(frames[:7], work / "n7.mp4", SYNTH_W, SYNTH_H, "24/1"), expect24, False, ["frame count"])
    run("25 fps against a 24 fps contract fails", encode_frames(frames, work / "n25.mp4", SYNTH_W, SYNTH_H, "25/1"),
        expect24, False, ["r_frame_rate", "duration"])
    run("24 fps against a 30000/1001 contract fails (integer rounding of NTSC is caught)", ref,
        scene_expect(expected, bundle, "30000/1001", SYNTH_W, SYNTH_H), False, ["r_frame_rate"])
    duplicated = list(frames)
    duplicated[4] = frames[3]
    run("a duplicated frame (frame 4 shows frame 3) fails identity",
        encode_frames(duplicated, work / "ndup.mp4", SYNTH_W, SYNTH_H, "24/1"), expect24, False, ["tells neighbours apart"])
    swapped = list(frames)
    swapped[2], swapped[5] = frames[5], frames[2]
    run("two swapped frames fail identity",
        encode_frames(swapped, work / "nswap.mp4", SYNTH_W, SYNTH_H, "24/1"), expect24, False, ["tells neighbours apart"])
    black = list(frames)
    black[3] = bytes(len(frames[3]))
    run("a black frame fails the probes", encode_frames(black, work / "nblack.mp4", SYNTH_W, SYNTH_H, "24/1"),
        expect24, False, ["flat-interior probes"])
    run("decoding the reference as BT.601 fails the probes (the colour contract is discriminated)", ref, expect24, False,
        ["flat-interior probes"], matrix="smpte170m")
    small = work / "n-small.mp4"
    ffmpeg("-i", str(ref), "-vf", "scale=160:90", "-c:v", "libx264", "-crf", "18", *COLOR_TAGS, str(small))
    run("a 160x90 file against a 320x180 contract fails the resolution", small, expect24, False, ["resolution"])
    untagged = encode_frames(frames, work / "nuntagged.mp4", SYNTH_W, SYNTH_H, "24/1", tags=False)
    run("a file without BT.709/limited tags fails the colour declaration", untagged, expect24, False, ["colour declared"])
    cut = work / "ncut.mp4"
    data = ref.read_bytes()
    cut.write_bytes(data[: int(len(data) * 0.55)])
    run("a truncated file does not pass", cut, expect24, False, ["readable", "frame count", "pts", "decoded"])

    tone = work / "tone.wav"
    ffmpeg("-f", "lavfi", "-i", "sine=frequency=440:duration=1", "-ar", "48000", str(tone))
    with_audio = encode_frames(frames, work / "naudio.mp4", SYNTH_W, SYNTH_H, "24/1", audio=tone)
    run("an unrequested audio stream fails the no-audio policy", with_audio, expect24, False, ["no audio stream"], audio="none")
    return results, {"frames_rendered": SCENE_FRAMES}


def run_audio_controls(work, expected, corpus_dir, log=print):
    """`verify_mixed_audio` on videos made from a cheap lavfi picture (the picture is irrelevant here)."""
    work = pathlib.Path(work)
    work.mkdir(parents=True, exist_ok=True)
    mix = pathlib.Path(corpus_dir) / "audio/mix.wav"
    results = []

    def make(name, audio_args, seconds=1):
        path = work / f"{name}.mp4"
        ffmpeg("-f", "lavfi", "-i", f"color=c=gray:s=320x180:r=24:d={seconds}", *audio_args, "-c:v", "libx264", "-pix_fmt", "yuv420p",
               "-c:a", "aac", "-shortest", str(path))
        return path

    def run(name, path, should_pass, markers=()):
        collector = v.Collector()
        probe = v.ffprobe(path)
        v.verify_mixed_audio(collector, path, probe, 1.0)
        results.append(outcome(name, should_pass, markers, collector))
        log(f"  {'ok  ' if results[-1]['ok'] else 'WRONG'} {name}")

    right = make("audio-right", ["-ss", "1.5", "-t", "1", "-i", str(mix)])
    run("the 1.5-2.5 s window of the mix (440 then 880 Hz) passes", right, True)
    early = make("audio-early", ["-ss", "0", "-t", "1", "-i", str(mix)])
    run("the 0-1 s window (440 Hz only) fails: the window offset is checked", early, False, ["second half"])
    late = make("audio-late", ["-ss", "3", "-t", "1", "-i", str(mix)])
    run("the 3-4 s window (880 Hz only) fails the first half", late, False, ["first half"])
    silent = make("audio-silent", ["-f", "lavfi", "-i", "anullsrc=r=48000:cl=mono"])
    run("a silent audio stream fails: promising audio without the mix is caught", silent, False, ["first half", "second half"])
    none = work / "no-audio.mp4"
    ffmpeg("-f", "lavfi", "-i", "color=c=gray:s=320x180:r=24:d=1", "-c:v", "libx264", "-pix_fmt", "yuv420p", str(none))
    run("a file with no audio stream fails the mix check", none, False, ["one audio stream"])
    return results


def run_detail_controls(work, detail_png, expected, log=print):
    """Reference and defective encodes of the 4K detail PNG (an original, not a proxy)."""
    work = pathlib.Path(work)
    work.mkdir(parents=True, exist_ok=True)
    results, metrics = [], {}
    expect = {"fps": v.rational("24/1"), "frames": DETAIL_FRAMES, "from": 0, "width": 3840, "height": 2160, "kind": "detail",
              "bundle": None}

    def encode(name, filters):
        path = work / f"{name}.mp4"
        chain = ",".join([*filters, RGB_TO_YUV])
        ffmpeg("-loop", "1", "-framerate", "24", "-i", str(detail_png), "-frames:v", str(DETAIL_FRAMES), "-vf", chain,
               "-c:v", "libx264", "-crf", "18", "-preset", "fast", *COLOR_TAGS, "-r", "24", "-movflags", "+faststart", str(path))
        return path

    def run(name, path, should_pass, markers=(), **options):
        collector = v.Collector()
        metric = v.verify_export(collector, path, expect, **options)
        results.append(outcome(name, should_pass, markers, collector))
        metrics[name] = metric.get("detail")
        log(f"  {'ok  ' if results[-1]['ok'] else 'WRONG'} {name}  {metric.get('detail')}")

    run("reference encode of the original 3840x2160 PNG passes", encode("detail-ref", ["format=rgb24"]), True)
    run("a proxy (1920x1080 and back up) fails the 1 px stripes",
        encode("detail-proxy", ["scale=1920:1080:flags=bicubic", "scale=3840:2160:flags=bicubic", "format=rgb24"]), False, ["1 px stripes"])
    run("a 3/4-scale proxy fails the 1 px stripes",
        encode("detail-proxy75", ["scale=2880:1620:flags=bicubic", "scale=3840:2160:flags=bicubic", "format=rgb24"]), False, ["1 px stripes"])
    run("a horizontally flipped picture fails the markers", encode("detail-flip", ["hflip", "format=rgb24"]), False, ["markers"])
    run("a picture shifted by 64 px fails the markers",
        encode("detail-shift", ["pad=3904:2160:64:0,crop=3840:2160:0:0", "format=rgb24"]), False, ["markers"])
    run("decoding the reference as BT.601 fails the colour blocks", work / "detail-ref.mp4", False, ["colour blocks"], matrix="smpte170m")
    return results, metrics


def summarize(results):
    return {"controls": len(results), "wrong": [r["control"] for r in results if not r["ok"]]}
