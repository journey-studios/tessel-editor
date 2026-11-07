#!/usr/bin/env python3
"""Scenario helpers of the performance investigation (see drv.py).

Failure contract: a step that fails (an op rejected, a wait that times out, a playback that never starts) raises
`DriverError`. That covers the set-up of a session too (`window.activate`, `project.open`, every warm-up/origin `seek`
and the `settle` after it, see `boot` and `seek_to`), not only the measured playback: a run is never recorded on top of
a project that did not open or a monitor that did not settle. Callers record the error in `out["errors"]` through
`guarded` and go on with the next measurement, or stop;
`run_session` then always stops the editor and saves what was measured with `status` complete | partial | failed |
empty, and a non-zero exit code unless it is `complete`. A result that is old, incomplete or empty never reaches the
analysis as if it had been measured.
"""
import json
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from drv import *  # noqa

H = f"{WORK}/proj/h/tessel-splat-h.tessel.json"
V = f"{WORK}/proj/v/tessel-splat-v.tessel.json"
FPS = 24.0


class DriverError(RuntimeError):
    """A measurement step failed; the run it belongs to is invalid and is recorded, never analysed."""

    def __init__(self, message, reply=None):
        super().__init__(message)
        self.reply = reply


class PlaybackError(DriverError):
    """playback.start/stop (or the status around them) failed, or the run drew nothing."""


def require_ok(reply, what, error=DriverError):
    if not isinstance(reply, dict) or not reply.get("ok"):
        raise error(f"{what} rejected", reply)
    return reply


def render_status(ed):
    """`render.status` data (the report), or a DriverError when the editor refuses or answers without a session."""
    reply = require_ok(ed.op({"op": "render.status"}), "render.status", PlaybackError)
    data = reply.get("data") or {}
    data = data.get("report", data)
    if not isinstance(data, dict) or "session" not in data:
        raise PlaybackError("render.status answered without a session", reply)
    return data


def log_len(ed):
    return len(read_perf_log(ed.perf_log))


def seek_to(ed, frame, timeout=60, what="seek"):
    """Move the playhead and wait until the monitor shows that frame; DriverError when the editor refuses the seek or
    the monitor does not settle in `timeout` seconds (a measurement must never start from an unsettled monitor)."""
    reply = ed.seek(frame)
    if not isinstance(reply, dict) or not reply.get("ok"):
        raise DriverError(f"{what}: seek to frame {frame} rejected", reply)
    if not ed.settle(timeout):
        raise DriverError(f"{what}: the monitor did not settle at frame {frame} within {timeout} s", reply)
    return reply


def boot(ed, project, warm=(), what="warm-up"):
    """Common prelude of a session: socket ready, window active, project opened, takes visited (warm-up).

    Every step is checked. Returns the boot time of the viewport in seconds."""
    boot_s = ed.wait_socket()
    require_ok(ed.activate(), "window.activate")
    require_ok(ed.open_project(project)[0], f"project.open {project}")
    time.sleep(0.8)
    for frame in warm:
        seek_to(ed, frame, 120, what)
    return boot_s


def start_editor(label, project=H, binary=RELEASE, runtime="instr", env=None, warm=True):
    ed = Editor(label, binary=binary, runtime=runtime, env=env)
    try:
        boot_s = boot(ed, project)
    except BaseException:
        ed.stop()
        raise
    return ed, boot_s


def pre_visit(ed, frames):
    """Explicit renders (load every asset the frames need); returns per-frame seconds. DriverError when a frame never
    renders (timeout or an error other than 'media is still preparing') or the monitor does not settle."""
    out = {}
    for frame in frames:
        t, reply = ed.first_frame(frame)
        if t is None:
            raise DriverError(f"pre-visit: frame {frame} did not render before the timeout")
        require_ok(reply, f"pre-visit: render.frame {frame}")
        out[frame] = round(t, 3)
    if not ed.settle():
        raise DriverError("pre-visit: the monitor did not settle after the last frame")
    return out


def playback_run(ed, start, frames, label, extra_wait=0.6, status_poll=True):
    """One measured playback. Raises PlaybackError (never returns a half result) when any step fails; the GPU sampler
    is always stopped and a started playback is always stopped again."""
    require_ok(ed.op({"op": "seek", "frame": start}), "seek", PlaybackError)
    if not ed.settle():
        raise PlaybackError(f"monitor did not settle at frame {start} within 60 s before run {label}")
    time.sleep(0.3)
    quiet = wait_quiet_gpu()
    n0 = log_len(ed)
    base = render_status(ed)
    base_stats = dict(base["session"]["stats"])
    base_total = base["session"]["trace"]["frames_total"]
    base_ui = base.get("ui")
    gs = GpuSampler()
    gs.__enter__()
    started = False
    try:
        t0 = time.time()
        reply = ed.op({"op": "playback.start", "from": start})
        if not reply.get("ok"):
            raise PlaybackError(f"playback.start rejected in run {label}", reply)
        started = True
        playing_at = None
        deadline = time.time() + 20
        while time.time() < deadline:
            st = (ed.op({"op": "playback.status"}).get("data") or {})
            if st.get("state") == "playing":
                playing_at = time.time()
                break
            time.sleep(0.1)
        if playing_at is None:
            raise PlaybackError(f"playback never reached state=playing within 20 s in run {label}")
        prep = playing_at - t0
        time.sleep(frames / FPS + extra_wait)
        stop = ed.op({"op": "playback.stop"})
        started = False
        require_ok(stop, f"playback.stop in run {label}", PlaybackError)
    finally:
        if started:
            try:
                ed.op({"op": "playback.stop"})
            except Exception:
                pass
        gs.__exit__()
    report = (stop.get("data") or {}).get("report")
    if not isinstance(report, dict) or report.get("frame") is None:
        raise PlaybackError(f"playback.stop in run {label} returned no report with a frame", stop)
    t1 = time.time()
    time.sleep(0.4)
    status = render_status(ed)
    rows = read_perf_log(ed.perf_log)[n0:]
    if not any(r.get("k") == "page" and (r.get("msg") or {}).get("perf") == "frame" for r in rows):
        raise PlaybackError(f"run {label} drew no frame rows in the perf log (instrumentation missing or nothing drawn)")
    result = {
        "label": label,
        "start": start,
        "frames_asked": frames,
        "gpu_util": gs.report(),
        "gpu_quiet_before": quiet,
        "prepare_s": round(prep, 3),
        "playback_report": report,
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
    path = f"{WORK}/results/{label}.json"
    dump(path, obj)
    return path


def guarded(out, context, fn):
    """Run one measurement. A failure is recorded in out["errors"] with its context (and the editor's reply when
    there is one) and returns None, so the caller skips the analysis of a run that did not happen."""
    try:
        return fn()
    except Exception as exc:  # noqa: BLE001 - every failure must be recorded, not only DriverError
        out.setdefault("errors", []).append({
            **context,
            "type": type(exc).__name__,
            "error": str(exc),
            "reply": getattr(exc, "reply", None),
        })
        print("RUN FAILED", json.dumps(out["errors"][-1], default=str)[:400], flush=True)
        return None


def finalize(label, out, fatal=None):
    """Decide the status, always save, and return the exit code (0 only for a complete, non-empty run)."""
    errors = out.get("errors") or []
    measured = out.get("runs") or out.get("phases")
    if fatal is not None:
        out["fatal"] = {"type": type(fatal).__name__, "error": str(fatal), "reply": getattr(fatal, "reply", None)}
        out["status"] = "failed"
    elif errors and not measured:
        out["status"] = "failed"
    elif errors:
        out["status"] = "partial"
    elif not measured:
        out["status"] = "empty"
    else:
        out["status"] = "complete"
    path = save(label, out)
    print("saved", path, "status", out["status"], "errors", len(errors), flush=True)
    return 0 if out["status"] == "complete" else 1


def run_session(label, out, make_editor, body):
    """Start the editor with make_editor(), run body(ed), stop the editor, save partial results whatever happened.

    Returns the exit code. KeyboardInterrupt/SystemExit are saved as a failed run and re-raised afterwards. The previous
    `results/<label>.json` is removed first, so if the final save fails (disk full) no file of another run is left
    under this label."""
    ed = None
    fatal = None
    stale = f"{WORK}/results/{label}.json"
    if os.path.lexists(stale):  # a result of an earlier run with this label must not outlive a save that fails below
        os.remove(stale)
    try:
        ed = make_editor()
        body(ed)
    except BaseException as exc:  # noqa: BLE001 - partial results are saved first, then the interrupt is re-raised
        fatal = exc
    finally:
        if ed is not None:
            try:
                ed.stop()
            except Exception as exc:  # noqa: BLE001
                out.setdefault("errors", []).append({"type": type(exc).__name__, "error": f"editor stop failed: {exc}"})
    code = finalize(label, out, fatal)
    if fatal is not None and not isinstance(fatal, Exception):
        raise fatal
    return code


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
