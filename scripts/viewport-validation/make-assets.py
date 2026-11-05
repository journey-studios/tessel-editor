#!/usr/bin/env python3
"""Synthetic assets only: python3 make-assets.py <out> [--quick]. No user media."""
import json, os, random, struct, subprocess, sys, math, shutil
out = sys.argv[1]; os.makedirs(out, exist_ok=True)
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
def ff(*a): subprocess.run(["ffmpeg", "-nostdin", "-v", "error", "-y", *a], check=True)

# ---- stills: the same demo picture as the earlier measurements (assets/demo), 1.6 MP and 4K
demo = next((os.path.join(ROOT, "assets/demo", f) for f in sorted(os.listdir(os.path.join(ROOT, "assets/demo"))) if f.endswith((".png", ".jpg", ".jpeg"))), None)
if not os.path.exists(f"{out}/still.png"):
    ff("-i", demo, "-vf", "scale=1672:941", f"{out}/still.png")
    ff("-i", demo, "-vf", "scale=3840:2160", f"{out}/still4k.png")
    ff("-f", "lavfi", "-i", "testsrc2=size=1672x941:rate=1", "-frames:v", "1", f"{out}/still-alt.png")

# ---- splats. `gauss`: compact cloud seen from outside. `room`: splats on the walls of a box with the
# camera inside, sized so coverage stays similar when the count grows (scan-like, many layers on screen).
# `dense`: constant splat size, overdraw grows with the count (a worst case, not a typical capture).
def splats(name, n, kind, seed=12345):
    path = f"{out}/{name}.splat"
    if os.path.exists(path): return
    r = random.Random(seed); buf = bytearray(n * 32)
    base = 0.6 * math.sqrt(1 / n) * 10 if kind == "room" else {"gauss": 0.012, "dense": 0.02}[kind]
    for i in range(n):
        if kind == "room":
            face = r.randrange(6); u, v = r.uniform(-3, 3), r.uniform(-3, 3); d = r.uniform(-0.05, 0.05)
            p = [(3 + d, u, v), (-3 + d, u, v), (u, 3 + d, v), (u, -3 + d, v), (u, v, 3 + d), (u, v, -3 + d)][face]
            c = [int(127 + 120 * math.sin(u * 2 + face)), int(127 + 120 * math.sin(v * 2)), int(127 + 120 * math.sin((u + v) * 1.5))]
            a = 230
        else:
            p = (r.gauss(0, .8), r.gauss(0, .8), r.gauss(0, .8))
            c = [r.randrange(40, 255) for _ in range(3)]; a = 200
        struct.pack_into("<6f4B4B", buf, i * 32, p[0], p[1], p[2], base, base, base, c[0], c[1], c[2], a, 255, 128, 128, 128)
    open(path, "wb").write(buf)
    json.dump({"format": "tessel-scene", "version": 1, "background": [0, 0, 0, 1], "objects": [{"type": "splat", "asset": "splat", "position": [0, 0, 0], "scale": 1}]}, open(f"{out}/{name}.json", "w"))

for label, n in [("100k", 100_000), ("1m", 1_000_000), ("3m", 3_000_000)]:
    splats(f"gauss{label}", n, "gauss")
    splats(f"room{label}", n, "room")
splats("dense1m", 1_000_000, "dense")
splats("gauss100k-alt", 100_000, "gauss", seed=999)
json.dump({"format": "tessel-scene", "version": 1, "background": [0.05, 0.05, 0.1, 1], "objects": [
    {"type": "box", "size": [1, 1, 1], "color": "#c04040", "position": [-1, 0, 0], "spin": [0, 40, 0]},
    {"type": "sphere", "radius": 0.6, "color": "#40a0c0", "position": [1, 0, 0], "spin": [20, 0, 0]}]}, open(f"{out}/box.json", "w"))
json.dump({"format": "tessel-scene", "version": 1, "background": [0.1, 0.05, 0.05, 1], "objects": [
    {"type": "box", "size": [1, 1, 1], "color": "#40c040", "position": [0, 0, 0]}]}, open(f"{out}/static.json", "w"))
json.dump({"format": "tessel-scene", "version": 1, "background": [0, 0, 0.1, 1], "objects": [
    {"type": "box", "size": [1, 1, 1], "color": "#c0c040", "position": [0, 0, 0], "spin": [0, 90, 0]},
    {"type": "splat", "asset": "splat", "position": [0, 0, 0], "scale": 1}]}, open(f"{out}/animsplat.json", "w"))

# ---- video: moderate noise over testsrc2, with the frame number as a 10-bit code in the first row.
def vf(w, h):
    cell = w // 20; boxes = [f"drawbox=x=0:y=0:w={cell*10}:h={cell}:color=black:t=fill"]
    for k in range(10):
        boxes.append(f"drawbox=x={k*cell}:y=0:w={cell}:h={cell}:color=white:t=fill:enable='mod(floor(n/{2**k}),2)'")
    return f"noise=c0s=5:allf=t," + ",".join(boxes)
V = f"{out}/video"; os.makedirs(V, exist_ok=True)
def master(w, h, n):
    p = f"{V}/master-{h}.mkv"
    if not os.path.exists(p):
        ff("-f", "lavfi", "-i", f"testsrc2=size={w}x{h}:rate=24:duration={n/24}", "-vf", vf(w, h), "-c:v", "libx264", "-qp", "4", "-preset", "ultrafast", "-g", "1", "-pix_fmt", "yuv444p", p)
    return p
def seq(h, m, ext, n, *q):
    d = f"{V}/seq-{h}-{ext}"
    if os.path.isdir(d): return
    os.makedirs(d)
    ff("-i", m, "-frames:v", str(n), *q, f"{d}/f%05d.{ext}")
    # the pattern is 0-based in the plan, ffmpeg writes from 1
    for i in range(n): os.rename(f"{d}/f{i+1:05d}.{ext}", f"{d}/x{i:05d}.{ext}")
def enc(name, m, *args):
    p = f"{V}/{name}"
    if not os.path.exists(p): ff("-i", m, *args, p)
for w, h, n in [(1280, 720, 240), (1920, 1080, 240), (3840, 2160, 72)]:
    m = master(w, h, n)
    if h == 720: seq(h, m, "png", 120)
    if h <= 1080: seq(h, m, "jpg", 120, "-q:v", "3")
    if h == 1080: seq(h, m, "png", 72)
    if h == 2160: seq(h, m, "jpg", 48, "-q:v", "3")
    x264 = ["-c:v", "libx264", "-crf", "20", "-preset", "veryfast", "-pix_fmt", "yuv420p"]
    enc(f"h264-g24-{h}.mp4", m, *x264, "-g", "24", "-movflags", "+faststart")
    if h <= 1080:
        enc(f"h264-intra-{h}.mp4", m, *x264, "-g", "1", "-movflags", "+faststart")
        enc(f"vp9-g24-{h}.webm", m, "-c:v", "libvpx-vp9", "-b:v", "0", "-crf", "32", "-g", "24", "-cpu-used", "6", "-deadline", "good", "-row-mt", "1", "-pix_fmt", "yuv420p")
        enc(f"vp8-g24-{h}.webm", m, "-c:v", "libvpx", "-b:v", "6M", "-g", "24", "-cpu-used", "6", "-deadline", "good", "-pix_fmt", "yuv420p")
    if h == 1080:
        enc("vp9-intra-1080.webm", m, "-c:v", "libvpx-vp9", "-b:v", "0", "-crf", "36", "-g", "1", "-cpu-used", "6", "-deadline", "good", "-row-mt", "1", "-pix_fmt", "yuv420p")
        enc("hevc-g24-1080.mp4", m, "-c:v", "hevc_videotoolbox", "-q:v", "60", "-g", "24", "-tag:v", "hvc1", "-pix_fmt", "yuv420p", "-movflags", "+faststart")
        enc("av1-g24-1080.mp4", m, "-c:v", "libsvtav1", "-crf", "35", "-preset", "10", "-g", "24", "-pix_fmt", "yuv420p", "-movflags", "+faststart")
# proxy for scrub: 540p intra-only, derived from the 1080p master
pm = f"{V}/master-1080.mkv"
enc("proxy-h264-intra-540.mp4", pm, "-vf", "scale=960:540", "-c:v", "libx264", "-crf", "26", "-preset", "veryfast", "-g", "1", "-pix_fmt", "yuv420p", "-movflags", "+faststart")
if not os.path.isdir(f"{V}/seq-540-proxy"):
    os.makedirs(f"{V}/seq-540-proxy")
    ff("-i", pm, "-frames:v", "120", "-vf", "scale=960:540", "-q:v", "5", f"{V}/seq-540-proxy/f%05d.jpg")
    for i in range(120): os.rename(f"{V}/seq-540-proxy/f{i+1:05d}.jpg", f"{V}/seq-540-proxy/x{i:05d}.jpg")
for m in ("master-720", "master-1080", "master-2160"):
    if "--keep-masters" not in sys.argv and os.path.exists(f"{V}/{m}.mkv"): os.remove(f"{V}/{m}.mkv")
print(subprocess.run(["du", "-sh", out, V], capture_output=True, text=True).stdout)
