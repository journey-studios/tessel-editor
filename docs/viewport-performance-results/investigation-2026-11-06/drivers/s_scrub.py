import sys, argparse, subprocess; sys.path.insert(0,'/tmp/perf-inv')
from scen import *
ap = argparse.ArgumentParser()
ap.add_argument("label"); ap.add_argument("--runtime", default="instr"); ap.add_argument("--binary", default="release")
ap.add_argument("--env", default=""); ap.add_argument("--project", default=H)
ap.add_argument("--drags", default="take:20:100:60,xfade:150:100:60,take30:20:100:30")  # name:origin:span:hz
ap.add_argument("--reps", type=int, default=3); ap.add_argument("--warm", default="5,190,300,450")
a = ap.parse_args()
env = dict(kv.split("=",1) for kv in a.env.split(",") if kv)
ed = Editor(a.label, binary=DEBUG if a.binary=="debug" else RELEASE, runtime=a.runtime, env=env)
out = {"label": a.label, "args": vars(a), "runs": []}
try:
    ed.wait_socket(); ed.activate(); ed.open_project(a.project); time.sleep(0.8)
    for fr in [int(x) for x in a.warm.split(",") if x]:
        ed.seek(fr); ed.settle(120)
    for spec in a.drags.split(","):
        name, origin, span, hz = spec.split(":")
        for rep in range(a.reps):
            ed.seek(int(origin)); ed.settle(60); time.sleep(0.3)
            wait_quiet_gpu()
            n0 = log_len(ed)
            jf = f"/tmp/perf-inv/results/{a.label}.{name}.{rep}.scrub.json"
            gs = GpuSampler(); gs.__enter__()
            p = subprocess.run([sys.executable, f"{ROOT}/scripts/seek-scrub-proof.py", "--socket", ed.socket_path, "--seconds", "2", "--hz", hz, "--span", span, "--json", jf, "--settle-timeout", "30"], capture_output=True, text=True)
            gs.__exit__()
            try:
                r = json.load(open(jf))
            except Exception:
                print("scrub failed", p.stdout[-300:], p.stderr[-300:]); continue
            rows = read_perf_log(ed.perf_log)[n0:]
            pf = [r_["msg"] for r_ in rows if r_.get("k")=="page" and r_["msg"].get("perf")=="frame" and r_["msg"].get("outcome")=="done"]
            entry = {"name": name, "rep": rep, "hz": hz, "span": span, "origin": origin,
                "presentations_in_window": r["presentations_in_window"], "present_gap_ms": r.get("present_gap_ms"), "lag_frames": r["lag_frames"],
                "release_to_final_ms": r["release_to_final_ms"], "first_present_ms": r["first_present_ms"], "stats_delta": r["stats_delta"], "gpu": gs.report(),
                "render_ms": summary([m["render_ms"] for m in pf]), "queue_ms": summary([m["queue_ms"] for m in pf]), "n_page_frames": len(pf), "host_spans": {k: v for k, v in (r.get("host_spans_ms") or {}).items() if k in ("runtime_render_ms","transit_ms","submit_to_done_ms")}}
            out["runs"].append(entry)
            print(a.label, name, rep, "pres", entry["presentations_in_window"], "gap", entry["present_gap_ms"], "lag", entry["lag_frames"], "fin", entry["release_to_final_ms"], "render", entry["render_ms"].get("p50"), entry["render_ms"].get("p95"), "gpu", gs.report(), flush=True)
finally:
    ed.stop()
save(a.label, out)
