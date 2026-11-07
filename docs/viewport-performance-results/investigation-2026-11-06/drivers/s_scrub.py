import sys, argparse, subprocess, os; sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from scen import *

SCRUB_KEYS = ("presentations_in_window", "lag_frames", "release_to_final_ms", "first_present_ms", "stats_delta")


def run_scrub(script, socket_path, hz, span, jf):
    """Run seek-scrub-proof.py once and return its parsed JSON, only if THIS run wrote it.

    The output file is removed first, so a file from an earlier run (same label/drag/rep) can never be read as this
    run's measurement. A non-zero exit, a missing or older file, unparsable JSON or missing keys all raise."""
    if os.path.lexists(jf):
        os.remove(jf)
    started = time.time()
    p = subprocess.run([sys.executable, script, "--socket", socket_path, "--seconds", "2", "--hz", hz, "--span", span, "--json", jf, "--settle-timeout", "30"], capture_output=True, text=True)
    tail = {"stdout": p.stdout[-300:], "stderr": p.stderr[-300:]}
    if p.returncode != 0:
        raise DriverError(f"scrub proof exited {p.returncode}", tail)
    if not os.path.isfile(jf):
        raise DriverError("scrub proof exited 0 but wrote no JSON", tail)
    if os.path.getmtime(jf) < started - 1:
        raise DriverError("scrub JSON is older than this run", tail)
    try:
        with open(jf) as handle:
            r = json.load(handle)
    except ValueError as exc:
        raise DriverError(f"scrub JSON is not valid: {exc}", tail)
    missing = [k for k in SCRUB_KEYS if not isinstance(r, dict) or k not in r]
    if missing:
        raise DriverError(f"scrub JSON lacks {missing}", tail)
    return r


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("label"); ap.add_argument("--runtime", default="instr"); ap.add_argument("--binary", default="release")
    ap.add_argument("--env", default=""); ap.add_argument("--project", default=H)
    ap.add_argument("--drags", default="take:20:100:60,xfade:150:100:60,take30:20:100:30")  # name:origin:span:hz
    ap.add_argument("--reps", type=int, default=3); ap.add_argument("--warm", default="5,190,300,450")
    a = ap.parse_args(argv)
    env = dict(kv.split("=", 1) for kv in a.env.split(",") if kv)
    out = {"label": a.label, "args": vars(a), "runs": []}

    def make_editor():
        script = scrub_script()  # a wrong repository root fails here, before any editor is started
        out["scrub_script"] = script
        return Editor(a.label, binary=DEBUG if a.binary == "debug" else RELEASE, runtime=a.runtime, env=env)

    def body(ed):
        ed.wait_socket(); ed.activate(); ed.open_project(a.project); time.sleep(0.8)
        for fr in [int(x) for x in a.warm.split(",") if x]:
            ed.seek(fr); ed.settle(120)
        for spec in a.drags.split(","):
            name, origin, span, hz = spec.split(":")
            for rep in range(a.reps):
                def one():
                    ed.seek(int(origin)); ed.settle(60); time.sleep(0.3)
                    wait_quiet_gpu()
                    n0 = log_len(ed)
                    jf = f"{WORK}/results/{a.label}.{name}.{rep}.scrub.json"
                    os.makedirs(os.path.dirname(jf), exist_ok=True)
                    gs = GpuSampler(); gs.__enter__()
                    try:
                        r = run_scrub(out["scrub_script"], ed.socket_path, hz, span, jf)
                    finally:
                        gs.__exit__()
                    rows = read_perf_log(ed.perf_log)[n0:]
                    pf = [r_["msg"] for r_ in rows if r_.get("k") == "page" and r_["msg"].get("perf") == "frame" and r_["msg"].get("outcome") == "done"]
                    entry = {"name": name, "rep": rep, "hz": hz, "span": span, "origin": origin,
                        "presentations_in_window": r["presentations_in_window"], "present_gap_ms": r.get("present_gap_ms"), "lag_frames": r["lag_frames"],
                        "release_to_final_ms": r["release_to_final_ms"], "first_present_ms": r["first_present_ms"], "stats_delta": r["stats_delta"], "gpu": gs.report(),
                        "render_ms": summary([m["render_ms"] for m in pf]), "queue_ms": summary([m["queue_ms"] for m in pf]), "n_page_frames": len(pf), "host_spans": {k: v for k, v in (r.get("host_spans_ms") or {}).items() if k in ("runtime_render_ms", "transit_ms", "submit_to_done_ms")}}
                    out["runs"].append(entry)
                    print(a.label, name, rep, "pres", entry["presentations_in_window"], "gap", entry["present_gap_ms"], "lag", entry["lag_frames"], "fin", entry["release_to_final_ms"], "render", entry["render_ms"].get("p50"), entry["render_ms"].get("p95"), "gpu", gs.report(), flush=True)
                guarded(out, {"drag": name, "rep": rep}, one)

    return run_session(a.label, out, make_editor, body)


if __name__ == "__main__":
    sys.exit(main())
