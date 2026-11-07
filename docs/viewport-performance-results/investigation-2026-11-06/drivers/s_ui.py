import sys, argparse, threading, subprocess, os; sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from scen import *


def cpu_s(pid):
    t = subprocess.run(["ps", "-o", "cputime=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
    try:
        parts = t.split(":"); return int(parts[0]) * 60 + float(parts[1])
    except Exception: return None


class Probe(threading.Thread):
    def __init__(self, ed):
        super().__init__(daemon=True); self.ed = ed; self.rtt = []; self.stop_ev = threading.Event()
    def run(self):
        while not self.stop_ev.is_set():
            r = self.ed.get("timeline.playhead"); self.rtt.append(r["_ms"]); self.stop_ev.wait(0.1)


def phase(ed, out, label, name, start, frames, probe=False, samp=False, play=True):
    """One phase. A failed playback records the error and no entry: the phase is simply not measured."""
    p = Probe(ed) if probe else None
    st = None
    cpu0, t0 = cpu_s(ed.pid), time.time()
    if samp:
        sp = f"{WORK}/results/{label}.{name}.sample.txt"
        def do():
            time.sleep(1.5); subprocess.run(["sample", str(ed.pid), "2", "-file", sp], capture_output=True)
        st = threading.Thread(target=do, daemon=True); st.start()
    if p: p.start()
    try:
        if play:
            res = playback_run(ed, start, frames, name)
        else:
            ed.seek(start); ed.settle(); time.sleep(0.3)
            res = {"rows": [], "trace_frames": []}
            time.sleep(4)
    finally:
        if p: p.stop_ev.set(); p.join(2)
        if st: st.join(10)
    cpu1, t1 = cpu_s(ed.pid), time.time()
    an = analyze_play(res) if play else {}
    entry = {"rtt_ms": summary(p.rtt) if p else None, "cpu_pct_avg_incl_setup": round((cpu1 - cpu0) / (t1 - t0) * 100, 1) if cpu0 is not None and cpu1 is not None else None,
        "gpui_renders_per_s": an.get("gpui_renders_per_s"), "gpui_render_interval_ms": an.get("gpui_render_interval_ms"), "gpui_render_fn_ms": an.get("gpui_render_fn_ms"), "gpui_tick_ms": an.get("gpui_tick_ms"),
        "header_render_ms": an.get("render_ms(header)"), "playback": {k: (res.get("playback_report") or {}).get(k) for k in ("elapsed_ms", "frame", "lag_frames", "skipped_frames")}, "gpu": res.get("gpu_util"), "ui_delta": res.get("ui_delta")}
    out["phases"][name] = entry
    print(label, name, json.dumps({k: entry[k] for k in ("rtt_ms", "cpu_pct_avg_incl_setup", "gpui_renders_per_s", "gpui_render_interval_ms", "gpui_tick_ms", "gpui_render_fn_ms", "playback")}), flush=True)


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("label"); ap.add_argument("--runtime", default="instr"); ap.add_argument("--binary", default="release")
    ap.add_argument("--env", default=""); ap.add_argument("--project", default=H)
    ap.add_argument("--warm", default="5,190,300,450")
    a = ap.parse_args(argv)
    env = dict(kv.split("=", 1) for kv in a.env.split(",") if kv)
    out = {"label": a.label, "args": vars(a), "phases": {}}

    def make_editor():
        return Editor(a.label, binary=DEBUG if a.binary == "debug" else RELEASE, runtime=a.runtime, env=env)

    def body(ed):
        ed.wait_socket(); ed.activate(); ed.open_project(a.project); time.sleep(0.8)
        for fr in [int(x) for x in a.warm.split(",") if x]:
            ed.seek(fr); ed.settle(120)
        ed.seek(40); ed.settle(); time.sleep(1)
        for name, start, frames, kw in [("idle-probe", 40, 0, dict(probe=True, play=False)), ("play-sample", 40, 96, dict(samp=True)),
                                        ("play-probe", 40, 96, dict(probe=True)), ("xfade-probe", 170, 60, dict(probe=True))]:
            guarded(out, {"phase": name}, lambda: phase(ed, out, a.label, name, start, frames, **kw))
        out["memory"] = ed.memory()

    return run_session(a.label, out, make_editor, body)


if __name__ == "__main__":
    sys.exit(main())
