#!/usr/bin/env python3
import json
import os
import sys
import time

sys.path.insert(0, "/tmp/perf-inv")
from drv import *  # noqa

H = "/tmp/perf-inv/proj/h/tessel-splat-h.tessel.json"
V = "/tmp/perf-inv/proj/v/tessel-splat-v.tessel.json"
FPS = 24.0


def log_len(ed):
    return len(read_perf_log(ed.perf_log))


def start_editor(label, project=H, binary=RELEASE, runtime="instr", env=None, warm=True):
    ed = Editor(label, binary=binary, runtime=runtime, env=env)
    boot = ed.wait_socket()
    ed.activate()
    ed.open_project(project)
    return ed, boot


def pre_visit(ed, frames):
    """Explicit renders (load every asset the frames need); returns per-frame seconds."""
    out = {}
    for frame in frames:
        t, reply = ed.first_frame(frame)
        out[frame] = round(t, 3) if t is not None else None
    ed.settle()
    return out


def playback_run(ed, start, frames, label, extra_wait=0.6, status_poll=True):
    ed.op({"op": "seek", "frame": start})
    ed.settle()
    time.sleep(0.3)
    quiet = wait_quiet_gpu()
    n0 = log_len(ed)
    base = ed.render_status()
    base_stats = dict(base["session"]["stats"])
    base_total = base["session"]["trace"]["frames_total"]
    base_ui = base.get("ui")
    gs = GpuSampler(); gs.__enter__()
    t0 = time.time()
    reply = ed.op({"op": "playback.start", "from": start})
    if not reply.get("ok"):
        return {"error": reply}
    playing_at = None
    deadline = time.time() + 20
    while time.time() < deadline:
        st = ed.op({"op": "playback.status"})["data"]
        if st.get("state") == "playing":
            playing_at = time.time()
            break
        time.sleep(0.1)
    prep = (playing_at or time.time()) - t0
    time.sleep(frames / FPS + extra_wait)
    stop = ed.op({"op": "playback.stop"})
    gs.__exit__()
    t1 = time.time()
    time.sleep(0.4)
    status = ed.render_status()
    rows = read_perf_log(ed.perf_log)[n0:]
    result = {
        "label": label,
        "start": start,
        "frames_asked": frames,
        "gpu_util": gs.report(),
        "gpu_quiet_before": quiet,
        "prepare_s": round(prep, 3),
        "playback_report": (stop.get("data") or {}).get("report"),
        "stats_delta": {k: status["session"]["stats"][k] - base_stats.get(k, 0) for k in status["session"]["stats"]},
        "ui_delta": {k: (status.get("ui") or {}).get(k, 0) - (base_ui or {}).get(k, 0) for k in (status.get("ui") or {})},
        "trace_frames": [f for f in status["session"]["trace"]["recent"] if f["id"] > base["session"]["stats"]["requests"]],
        "trace_dropped_total": status["session"]["trace"]["frames_total"] - base_total,
        "rows": rows,
    }
    return result


def analyze_play(result, lo=None, hi=None):
    """Per-run summary from the perf log + trace. lo/hi bound the shown frame (to bucket by region)."""
    rows = result["rows"]
    frames = [r["msg"] for r in rows if r.get("k") == "page" and r["msg"].get("perf") == "frame"]
    host_t = [r["t"] for r in rows if r.get("k") == "page" and r["msg"].get("perf") == "frame"]
    trace = {f["id"]: f for f in result["trace_frames"]}
    out = {"frames": len(frames)}

    def pick(key):
        return [f.get(key) for f in frames if f.get("outcome") == "done"]

    def spanvals(name):
        return [f["sum"].get(name) for f in frames if f.get("outcome") == "done" and f["sum"].get(name) is not None]

    out["render_ms(header)"] = summary(pick("render_ms"))
    out["page_total_ms"] = summary(pick("total_ms"))
    out["queue_ms"] = summary(pick("queue_ms"))
    for name in ["settle_total", "settle_update", "spark_prepare", "spark_generate_cpu", "spark_drivesort", "spark_readback", "spark_worker_sort", "rig_build", "rig_dispose", "lease_decode", "scene_render_submit", "composite_submit", "settle_sleep"]:
        vals = spanvals(name)
        if vals:
            out[name] = summary(vals)
    attempts = [f["cnt"].get("settle_update") for f in frames if f.get("cnt") and f["cnt"].get("settle_update")]
    if attempts:
        out["settle_attempts_per_frame"] = summary(attempts)
    decs = [f["cnt"].get("lease_decode", 0) for f in frames if f.get("cnt")]
    out["lease_decodes_per_frame"] = summary(decs)
    out["mapping_changes_per_frame"] = summary([f["sum"].get("spark_mapping_changed", 0) for f in frames])
    scene_layers = [sum(1 for l in f["layers"] if l["kind"] == "scene") for f in frames]
    out["scene_layers_per_frame"] = summary(scene_layers, 2)
    if any(f.get("drain_ms") is not None for f in frames):
        out["drain_ms"] = summary([f.get("drain_ms") for f in frames if f.get("drain_ms") is not None])
        out["render+drain_ms"] = summary([f["render_ms"] + f["drain_ms"] for f in frames if f.get("drain_ms") is not None])
    out["post_ms"] = summary([f.get("post_ms") for f in frames if f.get("post_ms") is not None])
    # host spans for the traced frames of this run
    tf = list(trace.values())
    for name in ["prepare_ms", "serialize_ms", "dispatch_ms", "transit_ms", "runtime_total_ms", "accept_ms", "submit_to_done_ms"]:
        vals = [f[name] for f in tf]
        if vals:
            out["host." + name] = summary(vals, 3)
    pres = [f["presented_after_done_ms"] for f in tf if f.get("presented_after_done_ms") is not None]
    if pres:
        out["host.presented_after_done_ms"] = summary(pres)
    stamps = sorted(f["done_ms"] for f in tf)
    gaps = [b - a for a, b in zip(stamps, stamps[1:])]
    if gaps:
        out["frame_interval_ms(host done)"] = summary(gaps)
    shown = [f["shown_frame"] for f in sorted(tf, key=lambda f: f["id"]) if f.get("shown_frame") is not None]
    out["distinct_frames_shown"] = len(set(shown))
    if shown:
        out["shown_range"] = [min(shown), max(shown)]
    ticks = [r["dur"] for r in rows if r.get("k") == "tick"]
    renders = [r for r in rows if r.get("k") == "render"]
    out["gpui_tick_ms"] = summary(ticks, 3)
    out["gpui_render_fn_ms"] = summary([r["dur"] for r in renders], 3)
    rt = [r["t"] for r in renders]
    if len(rt) > 2:
        span = (rt[-1] - rt[0]) / 1000
        out["gpui_renders"] = len(rt)
        out["gpui_renders_per_s"] = round(len(rt) / span, 1) if span > 0 else None
        out["gpui_render_interval_ms"] = summary([b - a for a, b in zip(rt, rt[1:])])
    return out


def save(label, obj):
    path = f"/tmp/perf-inv/results/{label}.json"
    dump(path, obj)
    return path


XFADES = [(186, 200), (268, 282), (410, 424)]


def region_of(frame):
    for lo, hi in XFADES:
        if lo <= frame < hi:
            return "xfade"
    return "single"


def analyze_regions(result):
    rows = result["rows"]
    pf = [(r["t"], r["msg"]) for r in rows if r.get("k") == "page" and r["msg"].get("perf") == "frame"]
    trace = {f["id"]: f for f in result["trace_frames"]}
    buckets = {"single": [], "xfade": []}
    for t, m in pf:
        tr = trace.get(m["id"])
        if not tr or tr.get("shown_frame") is None or m.get("outcome") != "done":
            continue
        buckets[region_of(tr["shown_frame"])].append((tr, m))
    out = {}
    rep = result.get("playback_report") or {}
    start = result["start"]
    end = rep.get("frame", start)
    for name, items in buckets.items():
        if not items:
            continue
        items.sort(key=lambda x: x[0]["done_ms"])
        clock = sum(1 for f in range(start + 1, end + 1) if region_of(f) == name)
        shown = len({tr["shown_frame"] for tr, _ in items})
        gaps = [b[0]["done_ms"] - a[0]["done_ms"] for a, b in zip(items, items[1:])]
        o = {
            "clock_frames": clock,
            "shown_frames": shown,
            "dropped_pct": round(100 * (1 - shown / clock), 1) if clock else None,
            "render_ms": summary([m["render_ms"] for _, m in items]),
            "interval_ms": summary(gaps),
        }
        dr = [m["drain_ms"] for _, m in items if m.get("drain_ms") is not None]
        if dr:
            o["drain_ms"] = summary(dr)
            o["render+drain"] = summary([m["render_ms"] + m["drain_ms"] for _, m in items if m.get("drain_ms") is not None])
        o["readback_ms"] = summary([m["sum"].get("spark_readback") for _, m in items])
        o["worker_sort_ms"] = summary([m["sum"].get("spark_worker_sort") for _, m in items])
        o["lease_decode_ms"] = summary([m["sum"].get("lease_decode", 0) for _, m in items])
        o["scene_layers"] = summary([sum(1 for l in m["layers"] if l["kind"] == "scene") for _, m in items])
        out[name] = o
    return out


XFADES = [(186, 200), (268, 282), (410, 424)]


def region_of(frame):
    for lo, hi in XFADES:
        if lo <= frame < hi:
            return "xfade"
    return "single"


def analyze_regions(result):
    rows = result["rows"]
    pf = [(r["t"], r["msg"]) for r in rows if r.get("k") == "page" and r["msg"].get("perf") == "frame"]
    trace = {f["id"]: f for f in result["trace_frames"]}
    buckets = {"single": [], "xfade": []}
    for t, m in pf:
        tr = trace.get(m["id"])
        if not tr or tr.get("shown_frame") is None or m.get("outcome") != "done":
            continue
        buckets[region_of(tr["shown_frame"])].append((tr, m))
    out = {}
    rep = result.get("playback_report") or {}
    start = result["start"]
    end = rep.get("frame", start)
    for name, items in buckets.items():
        if not items:
            continue
        items.sort(key=lambda x: x[0]["done_ms"])
        clock = sum(1 for f in range(start + 1, end + 1) if region_of(f) == name)
        shown = len({tr["shown_frame"] for tr, _ in items})
        gaps = [b[0]["done_ms"] - a[0]["done_ms"] for a, b in zip(items, items[1:])]
        o = {
            "clock_frames": clock,
            "shown_frames": shown,
            "dropped_pct": round(100 * (1 - shown / clock), 1) if clock else None,
            "render_ms": summary([m["render_ms"] for _, m in items]),
            "interval_ms": summary(gaps),
        }
        dr = [m["drain_ms"] for _, m in items if m.get("drain_ms") is not None]
        if dr:
            o["drain_ms"] = summary(dr)
            o["render+drain"] = summary([m["render_ms"] + m["drain_ms"] for _, m in items if m.get("drain_ms") is not None])
        o["readback_ms"] = summary([m["sum"].get("spark_readback") for _, m in items])
        o["worker_sort_ms"] = summary([m["sum"].get("spark_worker_sort") for _, m in items])
        o["lease_decode_ms"] = summary([m["sum"].get("lease_decode", 0) for _, m in items])
        o["scene_layers"] = summary([sum(1 for l in m["layers"] if l["kind"] == "scene") for _, m in items])
        out[name] = o
    return out
