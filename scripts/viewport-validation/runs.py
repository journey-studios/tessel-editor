#!/usr/bin/env python3
"""Generates the bench run lists into $TEV/runs/."""
import json, os
TEV = os.environ.get("TEV", "/tmp/tev"); R = f"{TEV}/runs"; os.makedirs(R, exist_ok=True)
def w(name, runs): json.dump(runs, open(f"{R}/{name}.json", "w")); print(name, len(runs))
BT, RT, RC = "baseline-trace", "retained-trace", "retained-rtc"

# ---- item 2: video. Sequences and containers, sequential/random/scrub, timing and exact image.
SEQ = {"png720": dict(dir="seq-720-png", ext="png", count=120), "jpg720": dict(dir="seq-720-jpg", ext="jpg", count=120), "png1080": dict(dir="seq-1080-png", ext="png", count=72), "jpg1080": dict(dir="seq-1080-jpg", ext="jpg", count=120), "jpg4k": dict(dir="seq-2160-jpg", ext="jpg", count=48), "proxy540": dict(dir="seq-540-proxy", ext="jpg", count=120)}
CONT = {"h264g24_720": ("h264-g24-720.mp4", 240), "h264i_720": ("h264-intra-720.mp4", 240), "vp9g24_720": ("vp9-g24-720.webm", 240), "vp8g24_720": ("vp8-g24-720.webm", 240),
        "h264g24_1080": ("h264-g24-1080.mp4", 240), "h264i_1080": ("h264-intra-1080.mp4", 240), "vp9g24_1080": ("vp9-g24-1080.webm", 240), "vp9i_1080": ("vp9-intra-1080.webm", 240), "vp8g24_1080": ("vp8-g24-1080.webm", 240), "hevc_1080": ("hevc-g24-1080.mp4", 240), "av1_1080": ("av1-g24-1080.mp4", 240),
        "h264g24_4k": ("h264-g24-2160.mp4", 72), "proxyh264i_540": ("proxy-h264-intra-540.mp4", 240)}
def video(engine_suffix=""):
    runs = []
    for key, v in SEQ.items():
        for tr in ("forward", "random", "scrub"):
            runs.append(dict(name=f"v:{key}:{tr}:timing", mode="video", variant=BT, video=v, trace=tr, n=100, timeoutSec=120))
        runs.append(dict(name=f"v:{key}:random:verify", mode="video", variant=BT, video=v, trace="random", n=60, verify=True, timeoutSec=120))
    for key, (f, c) in CONT.items():
        v = dict(file=f, count=c)
        for tr in ("forward", "random", "scrub"):
            runs.append(dict(name=f"v:{key}:{tr}:timing", mode="video", variant=BT, video=v, trace=tr, n=100, timeoutSec=200))
        for tr in ("forward", "random", "scrub"):
            runs.append(dict(name=f"v:{key}:{tr}:verify", mode="video", variant=BT, video=v, trace=tr, n=60, verify=True, timeoutSec=200))
        runs.append(dict(name=f"v:{key}:random:verify-mid", mode="video", variant=BT, video=v, trace="random", n=60, verify=True, timeOffset=0.5, timeoutSec=200))
    return runs
w("video-formats", video())
w("video-caps", [dict(name="caps", mode="caps", variant="baseline")])
paced = []
for hz in (30, 60):
    for tr in ("forward", "scrub", "random"):
        for key, v, pf, lim in [("jpg1080", SEQ["jpg1080"], None, None), ("jpg1080+cache64", SEQ["jpg1080"], None, 64), ("jpg1080+prefetch", SEQ["jpg1080"], dict(ahead=12, behind=4, concurrency=2, bytes=512 << 20), None), ("jpg720", SEQ["jpg720"], None, None), ("jpg720+prefetch", SEQ["jpg720"], dict(ahead=12, behind=4, concurrency=2, bytes=256 << 20), None), ("proxy540", SEQ["proxy540"], None, None), ("proxy540+prefetch", SEQ["proxy540"], dict(ahead=12, behind=4, concurrency=2, bytes=128 << 20), None), ("png1080", SEQ["png1080"], None, None), ("png1080+prefetch", SEQ["png1080"], dict(ahead=12, behind=4, concurrency=2, bytes=512 << 20), None),
                                 ("h264g24_1080", dict(file="h264-g24-1080.mp4", count=240), None, None), ("h264i_1080", dict(file="h264-intra-1080.mp4", count=240), None, None), ("vp9g24_1080", dict(file="vp9-g24-1080.webm", count=240), None, None), ("proxyh264i_540", dict(file="proxy-h264-intra-540.mp4", count=240), None, None)]:
            r = dict(name=f"p:{key}:{tr}:{hz}", mode="video", variant=BT, video=v, trace=tr, n=120, paced=hz, timeoutSec=120)
            if pf: r["prefetch"] = pf
            if lim: r["frameLimit"] = lim
            paced.append(r)
w("video-paced", paced)
# prefetch must not change the picture
w("video-prefetch-verify", [dict(name=f"pv:jpg1080:{tr}", mode="video", variant=BT, video=SEQ["jpg1080"], trace=tr, n=100, verify=True, prefetch=dict(ahead=12, behind=4, concurrency=2, bytes=512 << 20), timeoutSec=120) for tr in ("forward", "scrub", "random")])

# ---- item 3: Spark, moving cameras and overdraw, CPU vs drained GPU vs rAF; still with and without the RT cache.
N = {"100k": 120, "1m": 90, "3m": 40}
sp = []
for c, n in N.items():
    for scene, paths in (("gauss", ("orbit", "near", "dolly", "jitter")), ("room", ("room",))):
        for path in paths:
            for v in (BT, RT):
                sp.append(dict(name=f"s:{scene}{c}:{path}:{v}:cpu", scenario=f"{scene}{c}:{path}", variant=v, frames=n, timeoutSec=240))
                sp.append(dict(name=f"s:{scene}{c}:{path}:{v}:read1x1", scenario=f"{scene}{c}:{path}", variant=v, frames=n, gpu="read1x1", timeoutSec=240))
    sp.append(dict(name=f"s:gauss{c}:still:{BT}", scenario=f"gauss{c}:still", variant=BT, frames=n, timeoutSec=240))
    sp.append(dict(name=f"s:gauss{c}:still:{RT}", scenario=f"gauss{c}:still", variant=RT, frames=n, timeoutSec=240))
    sp.append(dict(name=f"s:gauss{c}:still:{RC}", scenario=f"gauss{c}:still", variant=RC, frames=max(n, 120), timeoutSec=240))
    sp.append(dict(name=f"s:gauss{c}:still:{RC}:read1x1", scenario=f"gauss{c}:still", variant=RC, frames=max(n, 120), gpu="read1x1", timeoutSec=240))
sp += [dict(name=f"s:dense1m:near:{v}:{m}", scenario="dense1m:near", variant=v, frames=60, timeoutSec=240, **({"gpu": m} if m != "cpu" else {})) for v in (BT, RT) for m in ("cpu", "read1x1")]
w("spark", sp)
w("spark-resolution", [dict(name=f"sr:gauss1m:orbit:{W}:{BT}", scenario="gauss1m:orbit", variant=BT, width=W, height=H, frames=40, gpu="read1x1", timeoutSec=300) for W, H in ((1920, 1080), (3840, 2160))])

# ---- item 4: lifecycle scripts on baseline and retained (and the frame cache).
life = []
for s, extra in (("image", {}), ("box", {}), ("splat", {}), ("splat", {"splat": "gauss1m"}), ("rtc", {}), ("two", {}), ("cancel", {}), ("limits", {})):
    for v in (BT, RT, RC) + (("retained-singlert",) if s == "two" else ()):
        life.append(dict(name=f"lc:{s}{':' + extra['splat'] if extra else ''}:{v}", mode="script", script=s, variant=v, timeoutSec=420, **extra))
w("lifecycle", life)

# ---- item 5: exact pixels. Out-of-order frames, several cameras, 1M/3M, 960x540/1080p/UHD. Budget: PNG only at the export sizes.
ORDER = [0, 37, 5, 90, 12, 37, 0, 140, 5, 66]
hs = []
for scene in ("gauss1m:orbit", "gauss3m:orbit", "room1m:room", "room3m:room"):
    for W, H, order in ((960, 540, ORDER), (1920, 1080, ORDER), (3840, 2160, ORDER[:7])):
        for v in ("baseline", "retained"):
            hs.append(dict(name=f"h:{scene.replace(':','_')}:{W}:{v}", mode="hash", scenario=scene, variant=v, width=W, height=H, frames_list=order, png=[1] if W != 960 else None, timeoutSec=900))
for sc in ("image", "image4k", "image2", "text", "textchange", "box"):
    for W, H in ((960, 540), (1920, 1080), (3840, 2160)):
        for v in ("baseline", "retained"):
            hs.append(dict(name=f"h:{sc}:{W}:{v}", mode="hash", scenario=sc, variant=v, width=W, height=H, frames_list=[0, 3, 1, 7, 2, 20, 3, 0, 5, 2], png=[1] if W != 960 else None, timeoutSec=300))
hs.append(dict(name="h:twin", mode="hash", scenario="gauss100k:orbit", variant="baseline", frames_list=ORDER, timeoutSec=120))
w("hash", hs)
w("hash-sw", [r for r in hs if not (r["scenario"].startswith(("gauss3m", "room3m")) or (r["width"] if "width" in r else 0) == 3840)])
