#!/usr/bin/env python3
"""Barcoded video fixtures for the native `<video>` proofs (docs/viewport-bugfix-report.md).

Every picture carries its own index as a 10-bit code in the first row (white cell = bit 1), over
moderate temporal noise so a lossy codec cannot reduce it to a flat picture. A frame is therefore
identified by reading ten pixels, whatever the codec and whichever tool decoded it.

    python3 make-fixtures.py <out-dir> [--big]

`--big` adds the 1080p GOP-24 clips (120 and 240 pictures) of the B4 diagnosis. The fixtures are generated, never
committed (`runtime/scripts/video/.gitignore`-style: keep them under /tmp).
"""
import json
import os
import subprocess
import sys

OUT = sys.argv[1]
BIG = "--big" in sys.argv
os.makedirs(OUT, exist_ok=True)


def ff(*args):
    subprocess.run(["ffmpeg", "-nostdin", "-v", "error", "-y", *args], check=True)


def barcode(width):
    """drawbox chain that writes frame number `n` (counted by the filter graph) in the top row."""
    cell = width // 20
    boxes = [f"drawbox=x=0:y=0:w={cell * 10}:h={cell}:color=black:t=fill"]
    for bit in range(10):
        boxes.append(
            f"drawbox=x={bit * cell}:y=0:w={cell}:h={cell}:color=white:t=fill:enable='mod(floor(n/{2 ** bit}),2)'"
        )
    return "noise=c0s=5:allf=t," + ",".join(boxes)


def source(width, height, rate, frames):
    # testsrc2 at `rate`; `frames` pictures exactly
    return ["-f", "lavfi", "-i", f"testsrc2=size={width}x{height}:rate={rate}:duration=100", "-frames:v", str(frames)]


X264 = ["-c:v", "libx264", "-crf", "18", "-preset", "veryfast", "-pix_fmt", "yuv420p"]
CLIPS = {}


def encode(name, width, height, rate, frames, codec_args, extra_filter="", extra_args=()):
    path = os.path.join(OUT, name)
    if not os.path.exists(path):
        vf = barcode(width) + (("," + extra_filter) if extra_filter else "")
        ff(*source(width, height, rate, frames), "-vf", vf, *codec_args, *extra_args, path)
    CLIPS[name] = dict(width=width, height=height, rate=rate, frames=frames)


W, H, N = 640, 360, 48
encode("h264-g24.mp4", W, H, "24", N, [*X264, "-g", "24", "-movflags", "+faststart"])
encode("h264-intra.mp4", W, H, "24", N, [*X264, "-g", "1", "-movflags", "+faststart"])
encode("h264-bframes.mp4", W, H, "24", N, [*X264, "-g", "24", "-bf", "3", "-movflags", "+faststart"])
encode("h264-ntsc.mp4", W, H, "24000/1001", N, [*X264, "-g", "24", "-movflags", "+faststart"])
encode("h264-30.mp4", W, H, "30", N, [*X264, "-g", "30", "-movflags", "+faststart"])
encode("hevc-g24.mp4", W, H, "24", N, ["-c:v", "hevc_videotoolbox", "-q:v", "70", "-g", "24", "-tag:v", "hvc1", "-pix_fmt", "yuv420p", "-movflags", "+faststart"])
encode("vp9-g24.webm", W, H, "24", N, ["-c:v", "libvpx-vp9", "-b:v", "0", "-crf", "30", "-g", "24", "-cpu-used", "6", "-deadline", "good", "-pix_fmt", "yuv420p"])
encode("vp8-g24.webm", W, H, "24", N, ["-c:v", "libvpx", "-b:v", "4M", "-g", "24", "-cpu-used", "6", "-deadline", "good", "-pix_fmt", "yuv420p"])
encode("av1-g24.mp4", W, H, "24", N, ["-c:v", "libsvtav1", "-crf", "30", "-preset", "10", "-g", "24", "-pix_fmt", "yuv420p", "-movflags", "+faststart"])
# The first picture does not sit at container time 0 (edit list / start_time).
encode("h264-offset.mp4", W, H, "24", N, [*X264, "-g", "24", "-movflags", "+faststart"], extra_args=["-output_ts_offset", "1.0"])
# Variable frame rate: frames 5, 17 and 18 are missing, timestamps kept (the code still counts the
# original index, so a frame-index reading and a timestamp reading disagree after the first gap).
encode("h264-vfr.mp4", W, H, "24", N, [*X264, "-g", "24", "-movflags", "+faststart"],
       extra_filter="select='not(eq(n,5)+eq(n,17)+eq(n,18))'", extra_args=["-fps_mode", "passthrough"])
if BIG:
    encode("h264-g24-1080.mp4", 1920, 1080, "24", 120, [*X264, "-g", "24", "-movflags", "+faststart"])
    # 240 pictures: a random order over it stays cold (no frame repeats within 120 inputs).
    encode("h264-g24-1080-240.mp4", 1920, 1080, "24", 240, [*X264, "-g", "24", "-movflags", "+faststart"])

# Numbered image sequences of the same barcoded picture (the preview path of imported media).
for ext, quality in (("png", ["-compression_level", "1"]), ("jpg", ["-q:v", "3"])):
    directory = os.path.join(OUT, f"seq-{ext}")
    if not os.path.isdir(directory):
        os.makedirs(directory)
        ff(*source(960, 540, "24", N), "-vf", barcode(960), *quality, "-start_number", "0", os.path.join(directory, "x%05d." + ext))
    CLIPS[f"seq-{ext}"] = dict(width=960, height=540, rate="24", frames=N, pattern=os.path.join(directory, "x%05d." + ext), ext=ext)

json.dump(CLIPS, open(os.path.join(OUT, "clips.json"), "w"), indent=1)
print(json.dumps(sorted(CLIPS)))
