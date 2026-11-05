#!/usr/bin/env python3
"""Item 1/4/7: a real window (own HOME) with an instrumented copy of the host.
   run-live.py matrix --out raw.jsonl [--tag x] [--reps 3]    gestures: socket and real GPUI dispatch
   run-live.py lifecycle --out raw.jsonl                      context loss/reload/project switch through the real host
Nothing is sampled during a gesture unless the row says so (obs != none). Traces are read once, after it.
"""
import argparse, json, os, shutil, socket, subprocess, sys, threading, time, signal
TEV = os.environ.get("TEV", "/tmp/tev")
BINS = {"debug": f"{TEV}/bin-inst/debug/tessel-editor", "release": f"{TEV}/bin-inst/release/tessel-editor"}

class Ed:
    def __init__(self, path): self.path = path
    def send(self, **f):
        with socket.socket(socket.AF_UNIX) as s:
            s.settimeout(120); s.connect(self.path); s.sendall((json.dumps(dict(version=1, **f)) + "\n").encode()); d = b""
            while not d.endswith(b"\n"):
                c = s.recv(1 << 20)
                if not c: raise RuntimeError("closed")
                d += c
        return json.loads(d)
    def vp(self): return self.send(command="get", target="monitor.viewport")["data"]["state"]
    def op(self, **r): return self.send(command="op", request=r)
    def seek(self, f): return self.send(command="set", target="timeline.playhead", field="frame", value=str(f))
    def dump(self):
        a = self.send(command="vv_dump")["data"]; time.sleep(0.12)
        b = self.send(command="vv_dump")["data"]
        return {"native": a["native"] + b["native"], "js": a["js"] + b["js"]}

class Instance:
    def __init__(self, binary, variant, tag, env=None):
        self.home = f"{TEV}/homes/{tag}"; shutil.rmtree(self.home, ignore_errors=True); os.makedirs(self.home)
        e = dict(os.environ, HOME=self.home, TESSEL_RUNTIME_DIR=f"{TEV}/rt/{variant}", **(env or {}))
        self.p = subprocess.Popen([BINS[binary]], env=e, cwd=f"{TEV}/inst", stdout=subprocess.DEVNULL, stderr=open(f"{TEV}/raw/live-{tag}.log", "w"))
        self.sock = f"{self.home}/.cache/tessel-editor/ui.sock"; self.ed = Ed(self.sock)
        for _ in range(300):
            time.sleep(0.1)
            if os.path.exists(self.sock):
                try:
                    if self.ed.vp()["phase"] == "ready": break
                except Exception: pass
        else: raise RuntimeError("not ready")
        self.wait_idle()
    def wait_idle(self, limit=30):
        end = time.monotonic() + limit
        while time.monotonic() < end:
            v = self.ed.vp(); s = v["session"]; l = v.get("last_frame")
            if s["status"] == "ready" and not s.get("pending") and l and l["revision"] == s["revision"]: return v
            time.sleep(0.03)
        return self.ed.vp()
    def stop(self):
        self.p.terminate()
        try: self.p.wait(6)
        except Exception: self.p.kill()

def analyze(trace, final):
    N = trace["native"]; J = [e for batch in trace["js"] for e in batch]
    byname = {}
    for n, a, t in N: byname.setdefault(n, []).append((t, a))
    def pair(a, b):
        A, B = byname.get(a, []), byname.get(b, []); return [y[0] - x[0] for x, y in zip(A, B)]
    frames = {}
    for e in J:
        frames.setdefault(e[1], {})[e[0]] = e[2]
        if e[0] == "js_raf": frames[e[1]]["raf_ts"] = e[3]
        if e[0] == "js_recv": frames[e[1]]["frame"] = e[3]
    ids = sorted(i for i, f in frames.items() if "js_post" in f and f.get("frame") is not None)
    ev = sorted(byname.get("eval_start", []))
    ipc = sorted(byname.get("ipc_handler", [])); oi = sorted(byname.get("on_ipc_start", [])); oe = sorted(byname.get("on_ipc_end", [])); fe = sorted(byname.get("ipc_flush_end", []))
    states = sorted(byname.get("state_done", []))
    import bisect
    et = [t for t, _ in ev]; it = [t for t, _ in ipc]; ot = [t for t, _ in oi]; oet = [t for t, _ in oe]; st = [t for t, _ in states]
    rows = []
    for i in ids:
        f = frames[i]; r = {"id": i, "frame": f["frame"]}
        k = bisect.bisect_right(et, f["js_recv"] + 2) - 1  # +2: 1 ms JS quantum plus clock alignment
        if k >= 0: r["eval_to_recv"] = f["js_recv"] - et[k]; r["eval_start"] = et[k]
        r["recv_to_start"] = f["js_start"] - f["js_recv"]
        if "js_drawn" in f: r["draw"] = f["js_drawn"] - f["js_start"]; r["drawn_to_post"] = f["js_post"] - f["js_drawn"]
        k = bisect.bisect_left(it, f["js_post"] - 2)
        if k < len(it): r["post_to_ipc"] = it[k] - f["js_post"]; ipc_t = it[k]
        else: ipc_t = None
        if ipc_t is not None:
            k = bisect.bisect_left(ot, ipc_t)
            if k < len(ot):
                r["ipc_to_on_ipc"] = ot[k] - ipc_t; r["on_ipc_t"] = ot[k]
                k2 = bisect.bisect_left(oet, ot[k])
                if k2 < len(oet): r["on_ipc"] = oet[k2] - ot[k]
        if "js_raf" in f: r["post_to_raf"] = f["js_raf"] - f["js_post"]; r["post_to_raf_ts"] = f["raf_ts"] - f["js_post"]
        # newest state the frame answered: the state_done whose playhead is this frame, latest before eval
        cand = [t for t, a in states if a == f["frame"] and t <= f["js_recv"] + 2]
        if cand: r["state_to_recv"] = f["js_recv"] - cand[-1]; r["state_to_post"] = f["js_post"] - cand[-1]; r["state_to_raf"] = (f.get("js_raf", f["js_post"]) - cand[-1])
        # lag: playhead at post vs frame drawn
        k = bisect.bisect_right(st, f["js_post"]) - 1
        if k >= 0 and states[k][1] is not None: r["lag_frames"] = abs(states[k][1] - f["frame"])
        rows.append(r)
    for a, b in zip(rows, rows[1:]):
        if "eval_start" in a and "eval_start" in b: a["cycle"] = b["eval_start"] - a["eval_start"]
        if "on_ipc_t" in a and "eval_start" in b: a["onipc_to_next_eval"] = b["eval_start"] - a["on_ipc_t"]
        a["post_gap"] = frames[b["id"]]["js_post"] - frames[a["id"]]["js_post"]
    def st_(v):
        v = sorted(x for x in v if x is not None)
        if not v: return None
        q = lambda p: v[min(len(v) - 1, int(p * len(v)))]
        return {"n": len(v), "p50": round(q(.5), 3), "p95": round(q(.95), 3), "p99": round(q(.99), 3), "mean": round(sum(v) / len(v), 3), "max": round(v[-1], 3)}
    out = {"frames": len(rows), "raf_distinct": len({round(f["raf_ts"], 1) for f in frames.values() if "raf_ts" in f}), "raf_frames": sum(1 for f in frames.values() if "raf_ts" in f)}
    for k in ["eval_to_recv", "recv_to_start", "draw", "drawn_to_post", "post_to_ipc", "ipc_to_on_ipc", "on_ipc", "onipc_to_next_eval", "post_to_raf", "post_to_raf_ts", "state_to_recv", "state_to_post", "state_to_raf", "cycle", "post_gap", "lag_frames"]:
        out[k] = st_([r.get(k) for r in rows])
    out["eval_call"] = st_(pair("eval_start", "eval_end")); out["plan"] = st_(pair("plan_start", "plan_end")); out["sync"] = st_(pair("sync_start", "sync_end"))
    out["exec"] = st_(pair("exec_start", "exec_end")); out["state_exec"] = st_(pair("exec_start", "state_done")); out["input_dispatch"] = st_(pair("input_tick", "input_done"))
    def after(a, b):
        B = sorted(t for t, _ in byname.get(b, [])); r = []
        for t, _ in byname.get(a, []):
            k = bisect.bisect_left(B, t)
            if k < len(B): r.append(B[k] - t)
        return r
    out["key"] = st_(after("plan_end", "key_end")); out["preview_request"] = st_(after("key_end", "preview_end"))
    out["gpui_draw"] = st_(after("render_start", "paint_end")); out["foreign_moves"] = len(byname.get("foreign_move", []))
    dur = sorted(after("render_start", "paint_end"))
    out["gpui_draw_ms_total"] = round(sum(dur), 2)
    tail = sorted(after("sync_start", "sync_flush_end")); out["sync_flush"] = st_(tail)
    if states: out["gesture_ms"] = round(states[-1][0] - states[0][0], 1)
    rs = [t for t, _ in byname.get("render_start", [])]; out["gpui_renders"] = len(rs); out["render_gap"] = st_([b - a for a, b in zip(rs, rs[1:])])
    ticks = [t for t, _ in byname.get("input_tick", [])]; out["input_period"] = st_([b - a for a, b in zip(ticks, ticks[1:])]); out["input_ticks"] = len(ticks)
    if rows and "eval_start" in rows[0]:
        t0, t1 = rows[0]["eval_start"], rows[-1]["eval_start"]; out["frames_per_s"] = round((len(rows) - 1) / max(1e-3, (t1 - t0) / 1000), 2)
    out["states"] = len(states); out["last_state"] = states[-1][1] if states else None; out["last_frame_drawn"] = rows[-1]["frame"] if rows else None
    if states and rows: out["release_to_final_post"] = round(frames[rows[-1]["id"]]["js_post"] - states[-1][0], 2)
    return out

def gesture_socket(i, ed, origin, span, hz, seconds, obs):
    n = int(hz * seconds); stop = threading.Event(); polls = [0]
    def poll():
        while not stop.is_set():
            ed.vp(); polls[0] += 1; time.sleep(obs / 1000)
    th = None
    if obs: th = threading.Thread(target=poll); th.start()
    t0 = time.monotonic(); rtts = []
    for k in range(n):
        f = origin + int(round(span * (k + 1) / n)); due = t0 + k / hz; time.sleep(max(0, due - time.monotonic()))
        a = time.monotonic(); ed.seek(f); rtts.append((time.monotonic() - a) * 1000)
    elapsed = time.monotonic() - t0; stop.set()
    if th: th.join()
    rtts.sort(); return {"sent": n, "real_hz": round(n / elapsed, 1), "set_rtt_p50": round(rtts[len(rtts)//2], 2), "set_rtt_p95": round(rtts[int(.95*len(rtts))], 2), "polls": polls[0]}, origin + span

def gesture_gpui(i, ed, origin, span, hz, seconds, cfg=None):
    r = ed.send(command="vv_ruler")["data"]
    x0 = r["x"] + origin * r["ppf"] + r["ppf"] / 2; x1 = r["x"] + (origin + span) * r["ppf"] + r["ppf"] / 2; y = r["y"] + 8
    ticks = int(hz * seconds); ed.send(command="vv_drag", x0=x0, x1=x1, y=y, hz=hz, ticks=ticks); time.sleep(seconds + 0.25)
    return {"ppf": r["ppf"], "x0": x0, "x1": x1, "y": y, "ticks": ticks}, origin + span

def matrix(a):
    out = open(a.out, "a")
    for binary in a.bins.split(","):
        for variant in a.variants.split(","):
            tag = f"{a.tag}-{binary}-{variant}"; inst = Instance(binary, variant, tag)
            ed = inst.ed; ph = ed.send(command="get", target="timeline.playhead")["data"]["state"]
            low, high = ph["min"], ph["max"]; span = min(a.span, high - low - 2); first = True
            try:
                if a.noraf: ed.send(command="vv_eval", script="window.__vvNoRaf=true")
                for rep in range(a.reps):
                    for mode, hz, obs in a.grid():
                        ed.seek(low + 1); inst.wait_idle(); ed.dump(); load = os.getloadavg()[0]; t = time.time()
                        if mode == "socket": info, final = gesture_socket(rep, ed, low + 1, span, hz, a.seconds, obs)
                        else: info, final = gesture_gpui(rep, ed, low + 1, span, hz, a.seconds)
                        t_end = time.monotonic(); v = inst.wait_idle(); wait = (time.monotonic() - t_end) * 1000
                        tr = ed.dump(); res = analyze(tr, final)
                        rec = dict(binary=binary, variant=variant, mode=mode, hz=hz, obs=obs, rep=rep, cold=first, load=round(load, 2), info=info, span=span, res=res, noraf=a.noraf,
                                   last=dict(render_ms=(v.get("last_frame") or {}).get("render_ms"), stats=v["session"]["stats"]), t=t)
                        out.write(json.dumps(rec) + "\n"); out.flush(); first = False
                        print(binary, variant, mode, hz, obs, rep, "load", round(load, 1), "frames", res["frames"], "fps", res.get("frames_per_s"), "cycle", (res.get("cycle") or {}).get("p50"), "last", res.get("last_state"), res.get("last_frame_drawn"), flush=True)
                        time.sleep(0.3)
            finally: inst.stop(); time.sleep(1)

def lifecycle(a):
    out = open(a.out, "a"); fx = f"{TEV}/fx"
    setup = [json.loads(l) for l in open(f"{fx}/proof.jsonl") if l.strip()]
    for variant in a.variants.split(","):
        tag = f"{a.tag}-life-{variant}"; inst = Instance("release", variant, tag, env={"TESSEL_VIEWPORT_INSPECT": "1"}); ed = inst.ed; steps = {}
        def hash_at(name, frame=None):
            if frame is not None: ed.seek(frame)
            time.sleep(0.05); v = inst.wait_idle(45); l = v.get("last_frame") or {}
            steps[name] = dict(hash=l.get("hash"), status=v["session"]["status"], error=(v["session"].get("error") or {}).get("code") if isinstance(v["session"].get("error"), dict) else v["session"].get("error"), frame=(v["session"].get("presented") or {}).get("frame"), phase=v["phase"])
        def explicit(name, frame, w=320, h=180):
            r = ed.op(op="render.frame", frame=frame, width=w, height=h)
            steps[name] = dict(hash=(r.get("data") or r.get("result") or {}).get("frame", {}).get("hash"), ok=r.get("ok"), err=None if r.get("ok") else str(r)[:200])
        try:
            for r in setup:
                x = ed.op(**r)
                if not x.get("ok"): steps.setdefault("setup_errors", []).append(str(x)[:200])
            for f in (3, 30, 10): hash_at(f"f{f}", f)
            explicit("explicit_30", 30)
            def lose():
                ed.send(command="vv_eval", script="TesselRuntime.debug.loseContext()"); time.sleep(0.4); v = ed.vp(); steps.setdefault("status_while_lost", []).append(v["session"]["status"])
                ed.send(command="vv_eval", script="TesselRuntime.debug.restoreContext()"); time.sleep(0.5)
            lose(); hash_at("after_loss_auto_f10"); hash_at("after_loss_f3", 3); hash_at("after_loss_f30", 30); explicit("explicit_30_after_loss", 30)
            # the scene is on top of the stack at f30; move the camera, undo, redo-like edit
            ed.op(op="clip.camera", clip="c-scene", position=[0, 1.2, 5], target=[0, 0, 0], fov=35, orbit=90); hash_at("camera_edit_f30", 30)
            ed.op(op="clip.camera", clip="c-scene", position=[0, 1.2, 5], target=[0, 0, 0], fov=50, orbit=45); hash_at("camera_back_f30", 30)
            lose(); hash_at("after_loss2_f30")
            ed.send(command="invoke", target="monitor.reload", action="reload"); time.sleep(0.2)
            for _ in range(200):
                if ed.vp()["phase"] == "ready": break
                time.sleep(0.05)
            hash_at("after_reload_auto"); hash_at("after_reload_f3", 3); hash_at("after_reload_f30", 30)
            # project switch: IDs now mean something else
            ed.op(op="project.create", name="Other", sequence="main", rate=24, width=320, height=180, length=48)
            ed.op(op="asset.add", kind="scene", id="scn", name="Other scene", path=f"{TEV}/assets/static.json"); ed.op(op="track.add", id="t", name="T", kind="video")
            ed.op(op="clip.add", id="c", track="t", asset="scn", start=0, duration=48, camera=dict(position=[0, 1.2, 5], target=[0, 0, 0], fov=50, orbit=45))
            hash_at("other_project_f3", 3)
            ed.op(op="project.create", name="Viewport proof", sequence="main", rate=24, width=320, height=180, length=48)
            for r in setup[1:]: ed.op(**r)
            hash_at("back_to_proof_f30", 30); hash_at("back_to_proof_f3", 3)
        finally:
            inst.stop(); time.sleep(1)
        out.write(json.dumps(dict(variant=variant, steps=steps)) + "\n"); out.flush(); print(variant, json.dumps(steps)[:1500], flush=True)

ap = argparse.ArgumentParser(); ap.add_argument("what"); ap.add_argument("--out", required=True); ap.add_argument("--tag", default="m"); ap.add_argument("--reps", type=int, default=3)
ap.add_argument("--bins", default="release,debug"); ap.add_argument("--variants", default="baseline-trace,retained-trace"); ap.add_argument("--span", type=int, default=100); ap.add_argument("--seconds", type=float, default=2.0)
ap.add_argument("--noraf", action="store_true"); ap.add_argument("--grid", default="main")
a = ap.parse_args()
G = {"main": [("socket", 30, 0), ("socket", 60, 0), ("socket", 120, 0), ("gpui", 30, 0), ("gpui", 60, 0), ("gpui", 120, 0), ("socket", 30, 10), ("socket", 120, 10), ("socket", 30, 250), ("socket", 120, 250)], "smoke": [("gpui", 60, 0), ("socket", 60, 0)], "ceil": [("socket", 240, 0), ("gpui", 240, 0)], "noraf": [("gpui", 60, 0), ("gpui", 120, 0), ("socket", 120, 0)]}
a.grid = lambda: G[a.__dict__["grid_name"]]
a.__dict__["grid_name"] = sys.argv[sys.argv.index("--grid") + 1] if "--grid" in sys.argv else "main"
{"matrix": matrix, "lifecycle": lifecycle}[a.what](a)
