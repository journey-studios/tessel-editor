import sys, argparse; sys.path.insert(0,'/tmp/perf-inv')
from scen import *
ap = argparse.ArgumentParser()
ap.add_argument("label"); ap.add_argument("--runtime", default="drain"); ap.add_argument("--binary", default="release")
ap.add_argument("--env", default=""); ap.add_argument("--project", default=H)
ap.add_argument("--runs", default="40:60,181:30"); ap.add_argument("--reps", type=int, default=2)
ap.add_argument("--warm", default="5,190,300,450")
a = ap.parse_args()
env = dict(kv.split("=",1) for kv in a.env.split(",") if kv)
ed = Editor(a.label, binary=DEBUG if a.binary=="debug" else RELEASE, runtime=a.runtime, env=env)
out = {"label": a.label, "args": vars(a), "runs": [], "load_avg": os.getloadavg()}
try:
    ed.wait_socket(); ed.activate(); ed.open_project(a.project); time.sleep(0.8)
    for fr in [int(x) for x in a.warm.split(",") if x]:
        ed.seek(fr); ed.settle(120)
    for spec in a.runs.split(","):
        start, frames = [int(x) for x in spec.split(":")]
        for rep in range(a.reps):
            res = playback_run(ed, start, frames, f"{spec}#{rep}")
            reg = analyze_regions(res); an = analyze_play(res)
            out["runs"].append({"spec": spec, "rep": rep, "playback": {k:(res.get("playback_report") or {}).get(k) for k in ("elapsed_ms","frame","lag_frames","skipped_frames")}, "regions": reg, "gpu_util": res.get("gpu_util"), "gpui": {k: an.get(k) for k in ("gpui_renders_per_s","gpui_render_interval_ms","gpui_tick_ms")}})
            line = []
            for name, o in reg.items():
                line.append(f"{name}: n={o['shown_frames']}/{o['clock_frames']} drop={o['dropped_pct']}% render p50/p95={o['render_ms']['p50']}/{o['render_ms']['p95']} " + (f"r+d p50={o['render+drain']['p50']} " if 'render+drain' in o else '') + f"iv p50/p95={o['interval_ms'].get('p50')}/{o['interval_ms'].get('p95')}")
            print(a.label, spec, rep, "gpu", res["gpu_util"]["mean"], res["gpu_util"]["max"], " | ".join(line), flush=True)
    out["memory"] = ed.memory()
finally:
    ed.stop()
save(a.label, out)
