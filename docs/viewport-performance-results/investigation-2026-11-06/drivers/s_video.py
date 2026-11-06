import sys, argparse, shutil; sys.path.insert(0,'/tmp/perf-inv')
from scen import *
ap = argparse.ArgumentParser()
ap.add_argument("label"); ap.add_argument("--runtime", default="instr"); ap.add_argument("--binary", default="release")
ap.add_argument("--env", default=""); ap.add_argument("--counts", default="1,2,4,8"); ap.add_argument("--assets", type=int, default=4)
ap.add_argument("--policies", default="original,auto"); ap.add_argument("--frames", type=int, default=96)
ap.add_argument("--mix", action="store_true")
a = ap.parse_args()
env = dict(kv.split("=",1) for kv in a.env.split(",") if kv)
ed = Editor(a.label, binary=DEBUG if a.binary=="debug" else RELEASE, runtime=a.runtime, env=env)
out = {"label": a.label, "args": vars(a), "runs": []}
def op(o, timeout=300):
    r = ed.op(o, timeout)
    if not r.get("ok"): print("OP FAIL", o, json.dumps(r)[:300], flush=True)
    return r
def wait_for(pred, seconds, step=0.3):
    t=time.time()
    while time.time()-t<seconds:
        v=pred()
        if v: return v
        time.sleep(step)
def media_assets():
    return (ed.op({"op":"media.status"}).get("data") or {}).get("assets") or []
try:
    ed.wait_socket(); ed.activate()
    ws = f"{ed.home}/project"; doc = f"{ws}/v.tessel.json"
    if a.mix:
        import shutil
        os.makedirs(ws, exist_ok=True)
        shutil.copytree("/tmp/perf-inv/proj/h", ws+"/h", dirs_exist_ok=True)
        doc = ws + "/h/tessel-splat-h.tessel.json"
        op({"op":"project.open","path":doc})
        length = 600
    else:
        length = 144
        op({"op":"project.create","name":"Video perf","sequence":"main","rate":24,"width":1280,"height":720,"length":length,"dir":ws})
        op({"op":"project.save","path":doc})
    n0 = len(media_assets())
    paths = [f"/tmp/perf-inv/videos/seg{i}.mp4" for i in range(a.assets)]
    op({"op":"media.import","paths":paths})
    t_imp = time.time()
    ok = wait_for(lambda: len([x for x in media_assets()]) >= n0 + a.assets, 300)
    print("imported", ok is not None, round(time.time()-t_imp,1), "s", flush=True)
    assets = [x["id"] for x in media_assets()][n0:]
    print("assets", assets, json.dumps(media_assets()[n0:n0+1])[:400], flush=True)
    tracks = 0
    counts = [int(x) for x in a.counts.split(",")]
    base_start = 40 if a.mix else 0
    for target in counts:
        while tracks < target:
            i = tracks
            op({"op":"track.add","id":f"vt{i}","name":f"V{i}","kind":"video"})
            op({"op":"clip.add","id":f"vc{i}","track":f"vt{i}","asset":assets[i % len(assets)],"start":base_start,"duration":a.frames+8,"source_start":{"num":((i*10)%30),"den":24}})
            tracks += 1
        op({"op":"project.save","path":doc})
        free = shutil.disk_usage('/tmp').free/2**30
        if free < 0.6: print('DISK LOW', free, flush=True); break
        for policy in a.policies.split(","):
            op({"op":"preview.policy","mode":policy})
            # wait for jobs/readiness
            def ready():
                r = ed.op({"op":"render.frame","frame":base_start+10}, 300)
                return r.get("ok")
            t=time.time(); rd = wait_for(ready, 900, 0.5); prep=time.time()-t
            ed.seek(base_start); ed.settle(120); time.sleep(0.5)
            for rep in range(2):
                res = playback_run(ed, base_start, a.frames, f"{target}-{policy}-{rep}")
                an = analyze_play(res)
                pb = res.get("playback_report") or {}
                shown = an.get("distinct_frames_shown")
                entry = {"videos": target, "policy": policy, "rep": rep, "ready_wait_s": round(prep,1), "playback": {k: pb.get(k) for k in ("elapsed_ms","frame","lag_frames","skipped_frames")},
                         "distinct_shown": shown, "asked": (pb.get("frame") or 0)-base_start, "dropped_pct": round(100*(1-(shown or 0)/max(1,(pb.get("frame") or base_start+a.frames)-base_start)),1),
                         "header_render_ms": an.get("render_ms(header)"), "interval": an.get("frame_interval_ms(host done)"), "page_total": an.get("page_total_ms"),
                         "gpu": res.get("gpu_util"), "gpui_rps": an.get("gpui_renders_per_s"), "layers": an.get("scene_layers_per_frame"), "stats_delta": res.get("stats_delta"),
                         "host": {k: an.get(k) for k in an if k.startswith("host.")}}
                out["runs"].append(entry)
                print(a.label, target, policy, rep, "shown", shown, "/", entry["asked"], "drop", entry["dropped_pct"], "hdr", an.get("render_ms(header)", {}).get("p50"), an.get("render_ms(header)", {}).get("p95"), "iv", (an.get("frame_interval_ms(host done)") or {}).get("p50"), (an.get("frame_interval_ms(host done)") or {}).get("p95"), "gpu", res["gpu_util"]["mean"], "skipped", pb.get("skipped_frames"), flush=True)
    out["memory"] = ed.memory()
    out["disk_home_mb"] = subprocess.run(["du","-sm",ed.home],capture_output=True,text=True).stdout.split()[0]
finally:
    ed.stop()
save(a.label, out)
