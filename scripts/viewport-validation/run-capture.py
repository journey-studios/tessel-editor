#!/usr/bin/env python3
"""Item 7: real GPUI dispatch -> spans -> rAF -> ScreenCaptureKit, in one epoch clock.
   Capture is the compositor's output as a capture API sees it; it is not light leaving the panel."""
import argparse, json, os, subprocess, sys, time, bisect
from PIL import Image
src = open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "run-live.py")).read().rsplit("ap = argparse", 1)[0]
argv = sys.argv; sys.argv = ["x"]; exec(src); sys.argv = argv
CAP = f"{TEV}/bin-bench/capture-latency"
ap = argparse.ArgumentParser(); ap.add_argument("--out", required=True); ap.add_argument("--variants", default="baseline-trace,retained-trace"); ap.add_argument("--reps", type=int, default=3); a = ap.parse_args()
KB = dict(ken_burns=False)
A = f"{TEV}/assets/video/seq-720-png/x%05d.png"
def locate(inst):
    """Finds the 10-cell code in a screenshot of the window, at a frame whose code alternates."""
    ed = inst.ed; ed.seek(85); inst.wait_idle()  # 85 = 0b0001010101
    w = json.loads(subprocess.run([CAP, "locate", str(inst.p.pid)], capture_output=True, text=True).stdout)
    win = next(x for x in w if x["onscreen"]); png = f"{inst.home}/w.png"
    subprocess.run(["screencapture", "-x", "-o", "-l", str(win["id"]), png], check=True)
    im = Image.open(png).convert("L"); W, H = im.size; sc = W / win["bounds"]["Width"]; px = im.load()
    for y in range(0, H, 2):
        runs = []; x = 0; cur = px[0, y] > 128; start = 0
        for x in range(1, W):
            v = px[x, y] > 128
            if v != cur: runs.append((start, x - start, cur)); start = x; cur = v
        for i in range(len(runs) - 7):
            seg = runs[i:i + 7]
            if seg[0][2] and all(abs(s[1] - seg[0][1]) <= 2 for s in seg) and seg[0][1] >= 40:
                cell = seg[0][1] / sc; xc = seg[0][0] + seg[0][1] // 2; ye = y
                while ye < H and px[xc, ye] > 128: ye += 1
                return dict(x0=seg[0][0] / sc, y0=((y + ye) / 2 / sc) - cell / 2, cell=cell, cells=10, win=win, png=png)
    raise RuntimeError("code not found")
def run_one(inst, loc, hz, span, origin, seconds, hit, out, variant, rep, attempt=0):
    ed = inst.ed
    if hit:  # prime the frame cache: 16 frames, same range
        ed.seek(origin); inst.wait_idle()
        for f in range(origin, origin + span + 1): ed.seek(f); inst.wait_idle()
    ed.seek(origin); inst.wait_idle(); ed.dump()
    cap = f"{inst.home}/cap.json"
    p = subprocess.Popen([CAP, "run", str(inst.p.pid), str(loc["x0"]), str(loc["y0"]), str(loc["cell"]), "10", str(seconds + 1.6), cap], stdout=subprocess.PIPE, text=True)
    p.stdout.readline(); time.sleep(0.5)
    info, _ = gesture_gpui(0, ed, origin, span, hz, seconds); inst.wait_idle(); p.wait(); tr = ed.dump(); c = json.load(open(cap)); I = c["info"]
    to_ms = lambda m: I["epoch_ms"] + (m - I["mach"]) * I["numer"] / I["denom"] / 1e6
    first = {}
    for d, arr, code in c["rows"]:
        first.setdefault(code, (to_ms(d), to_ms(arr)))
    if c["frames_seen"] == 0 and attempt < 5:
        # ScreenCaptureKit sometimes delivers nothing at all, in every variant: a capture failure, counted and retried.
        out.write(json.dumps(dict(variant=variant, rep=rep, hz=hz, hit=hit, capture_failed=True, attempt=attempt, frames_rendered=len(tr["js"]))) + "\n"); out.flush(); time.sleep(2)
        return run_one(inst, loc, hz, span, origin, seconds, hit, out, variant, rep, attempt + 1)
    N = tr["native"]; J = [e for b in tr["js"] for e in b]
    state = {}; ticks = sorted((t, x) for n, x, t in N if n == "input_tick")
    for n, x, t in N:
        if n == "state_done": state.setdefault(x, t)
    frames = {}
    for e in J:
        frames.setdefault(e[1], {})[e[0]] = e[2]
        if e[0] == "js_recv": frames[e[1]]["frame"] = e[3]
    rows = []
    for f in frames.values():
        fr = f.get("frame")
        if fr is None or fr not in state or fr not in first or "js_post" not in f: continue
        ts = state[fr]; dc, ac = first[fr]
        rows.append(dict(frame=fr, state_to_post=f["js_post"] - ts, state_to_raf=f.get("js_raf", f["js_post"]) - ts, state_to_capture=dc - ts, state_to_arrival=ac - ts, post_to_capture=dc - f["js_post"], raf_to_capture=dc - f.get("js_raf", f["js_post"])))
    def st(k):
        v = sorted(r[k] for r in rows)
        if not v: return None
        q = lambda p: v[min(len(v) - 1, int(p * len(v)))]
        return dict(n=len(v), p50=round(q(.5), 2), p95=round(q(.95), 2), p99=round(q(.99), 2), mean=round(sum(v) / len(v), 2), max=round(v[-1], 2))
    rec = dict(variant=variant, rep=rep, hz=hz, span=span, seconds=seconds, hit=hit, load=round(os.getloadavg()[0], 2), frames_seen=c["frames_seen"], codes_seen=len(first), frames_rendered=len(frames), matched=len(rows), capture_scale=I["scale"], **{k: st(k) for k in ["state_to_post", "state_to_raf", "state_to_capture", "state_to_arrival", "post_to_capture", "raf_to_capture"]})
    out.write(json.dumps(rec) + "\n"); out.flush(); print(variant, "hit" if hit else "miss", hz, rec["codes_seen"], rec["matched"], "state→capture", rec["state_to_capture"], "raf→capture", rec["raf_to_capture"], flush=True)
out = open(a.out, "a")
for variant in a.variants.split(","):
    inst = Instance("release", variant, f"cap-{variant}"); ed = inst.ed
    try:
        for r in [dict(op="project.create", name="Code", sequence="main", rate=24, width=1280, height=720, length=120), dict(op="asset.add", kind="video", id="vid", name="Code", pattern=A, rate=24, frames=120), dict(op="track.add", id="t", name="T", kind="video"), dict(op="clip.add", id="c", track="t", asset="vid", start=0, duration=120, properties=KB)]:
            x = ed.op(**r)
            if not x.get("ok"): raise SystemExit(str(x)[:300])
        loc = locate(inst); print("located", {k: loc[k] for k in ("x0", "y0", "cell")}, loc["win"]["bounds"], flush=True)
        for rep in range(a.reps):
            run_one(inst, loc, 30, 14, 40, 1.0, True, out, variant, rep)
            run_one(inst, loc, 60, 14, 40, 0.5, True, out, variant, rep)
            run_one(inst, loc, 60, 100, 2, 2.0, False, out, variant, rep)
    finally: inst.stop(); time.sleep(1)
