#!/usr/bin/env python3
"""BL157.1: same-build A/B of the asset protocol (sync vs async reads) on a REAL project, in the real window.

    python3 scripts/real-latency-ab.py preflight --project P.tessel.json
    python3 scripts/real-latency-ab.py run      --binary target/release/tessel-editor --project P.tessel.json --out DIR [--pairs 4]
    python3 scripts/real-latency-ab.py analyze  --out DIR [--report-dir DIR2]
    python3 scripts/real-latency-ab.py pixels   --binary target/release/tessel-editor --out DIR

The project path is an ARGUMENT: no personal path, no media and no asset name is in the repository, and the report written to
`--report-dir` carries only hashes, counts and numbers. The document is COPIED into a private workspace; the assets it names
are only read, by the editor, through absolute paths. The original document and its workspace are never opened or written.

`run`: 2 * --pairs sessions of the SAME binary, one fresh editor each (own HOME, own socket, own window), in the order
sync/async, async/sync, sync/async, async/sync (A = sync, B = async). In each session the whole project plays twice: the
first reproduction and a repetition in the same process (resident cache). They are analysed apart and never pooled.
Both arms run with the trace ON (the same cost), the scene warm ON, `TESSEL_GL=gpu`, and differ ONLY in
`TESSEL_VIEWPORT_ASSET_IO`. The editor is read with `render.trace` (events, host monotonic clock, polled with a cursor and no
frame requested to measure) and `render.status` (the frame table); nothing here subtracts a Python, a page and a Rust clock.

Validity, never averaged over: the report names the mode in effect; an async session that saturated its queue, a session that
lost trace events, a failed play, an asset answered with an error status, or a warm that did not run invalidates THAT session
(it stays in the report, flagged). External load is recorded every poll; a session is `quiet` only under thresholds fixed in
this file before any measurement, and a pair is comparable only when both sessions are quiet. Everything else is published in
its own group, not dropped and not mixed in.

The diagnostic thresholds (62.5 ms, 100 ms) locate pauses; they are NOT limits the user approved. 41.7 ms at 24 fps is only the
frame budget. Nothing here decides acceptance.

Exit: 0 every session valid and comparable; 1 some session invalid or the pair set is not comparable (report says which);
2 a prerequisite is missing or a lock/disk guard refused (BLOCKED).
"""
import argparse
import fcntl
import hashlib
import importlib.util
import json
import math
import os
import pathlib
import platform
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import threading
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent

SLOW_DIAGNOSTIC_MS = 62.5
SLOWER_DIAGNOSTIC_MS = 100.0
ENTRY_WINDOW = 6
POLL_SECONDS = 2.0
MIN_FREE_GIB_START = 16.0
MIN_FREE_GIB_FLOOR = 12.0
# Fixed BEFORE any measurement (BL157.1). External load = CPU of every process that is not the session's own, in cores.
QUIET_MEDIAN_CORES = 3.0
QUIET_P95_CORES = 6.0
# A pair is comparable only if both sessions are quiet AND their median external loads differ by at most this.
PAIR_MAX_DELTA_CORES = 2.0
MODES = ("sync", "async")
ACCEPTABLE_STATUS = {200, 206}


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


# ------------------------------------------------------------------------------------------------ pure logic
def pair_order(pairs):
    """sync/async, async/sync, sync/async, async/sync ...: A = sync first, then B first, alternating."""
    order = []
    for index in range(pairs):
        order += ["sync", "async"] if index % 2 == 0 else ["async", "sync"]
    return order


def percentile(values, share):
    if not values:
        return None
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, math.ceil(share * len(ordered)) - 1))
    return round(ordered[index], 3)


def stats(values):
    """p50 / p95 / max / n of a list (None when empty). Nearest-rank percentiles: no interpolation invents a value."""
    values = [v for v in values if v is not None]
    if not values:
        return {"n": 0, "p50": None, "p95": None, "max": None}
    return {"n": len(values), "p50": percentile(values, 0.5), "p95": percentile(values, 0.95), "max": round(max(values), 3)}


def label_of(text):
    """The short stable label a trace and this report use for a private name (FNV-1a, 32 bits)."""
    value = 0xCBF29CE484222325
    for byte in text.encode():
        value ^= byte
        value = (value * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return f"{value & 0xFFFFFFFF:08x}"


def parse_project(document):
    """What the experiment needs from a project document, without a name or a path: sequence, cuts, source locations."""
    sequences = document.get("sequences") or []
    active = document.get("active_sequence")
    sequence = next((s for s in sequences if s.get("id") == active), sequences[0] if sequences else None)
    if sequence is None:
        raise ValueError("the document has no sequence")
    rate = sequence["rate"]
    fps = rate["num"] / rate["den"]
    clips = sequence.get("clips") or []
    cuts = sorted({c["start"] for c in clips if c.get("kind") == "scene" and c["start"] > 0})
    sources = []
    for entry in (document.get("library") or {}).get("assets", []):
        source = (entry.get("asset") or {}).get("source") or {}
        if source.get("type") == "file" and source.get("path"):
            sources.append({"kind": entry.get("kind"), "path": source["path"]})
        elif source.get("type") == "frame_sequence" and source.get("pattern"):
            sources.append({"kind": entry.get("kind"), "pattern": source["pattern"]})
    return {
        "length": sequence["length"], "fps": fps, "cuts": cuts,
        "clip_kinds": {kind: sum(1 for c in clips if c.get("kind") == kind) for kind in sorted({c.get("kind") for c in clips})},
        "sources": sources,
    }


def head_tail_hash(path, chunk=1 << 20):
    """sha256 of the first and last MiB plus the size: identity enough to notice another file, without reading 66 MiB."""
    digest = hashlib.sha256()
    size = os.path.getsize(path)
    with open(path, "rb") as handle:
        digest.update(handle.read(chunk))
        if size > 2 * chunk:
            handle.seek(size - chunk)
            digest.update(handle.read(chunk))
    return size, digest.hexdigest()


def check_sources(sources):
    """Presence and identity of every source, WITHOUT its name: a label, kind, size and a head/tail hash."""
    out = []
    for source in sources:
        if "path" in source:
            exists = os.path.isfile(source["path"])
            entry = {"label": label_of(source["path"]), "kind": source["kind"], "present": exists}
            if exists:
                entry["size"], entry["head_tail_sha256"] = head_tail_hash(source["path"])
        else:
            import glob
            import re
            pattern = re.sub(r"%0?\d*d", "*", source["pattern"])
            found = glob.glob(pattern)
            entry = {"label": label_of(source["pattern"]), "kind": source["kind"], "present": bool(found), "frames": len(found)}
        out.append(entry)
    return out


def merge_by_key(seen, items, key):
    for item in items or []:
        seen.setdefault(item[key], item)
    return seen


def pictures_of(frames):
    """Accepted pictures in the order the host accepted them."""
    return sorted((f for f in frames if f.get("done_ms") is not None and f.get("shown_frame") is not None),
                  key=lambda f: (f["done_ms"], f["id"]))


def play_segments(events):
    """[(command_ms, clock_started_ms, end_ms)] of every play, from the host's own events (one clock)."""
    commands = sorted((e["t_ms"] for e in events if e["kind"] == "play_command"))
    clocks = sorted((e["t_ms"] for e in events if e["kind"] == "play_clock_started"))
    segments = []
    for index, command in enumerate(commands):
        clock = next((c for c in clocks if c >= command), None)
        end = commands[index + 1] if index + 1 < len(commands) else math.inf
        segments.append({"command_ms": command, "clock_ms": clock, "end_ms": end})
    return segments


def play_pictures(frames, segment):
    """The pictures the clock produced during one play: interactive ones, after its clock started, before the next play.

    The picture that follows the end (the refine of the paused playhead) is not interactive and is not part of the play."""
    if segment["clock_ms"] is None:
        return []
    mine = [f for f in pictures_of(frames) if segment["clock_ms"] <= f["done_ms"] < segment["end_ms"]]
    interactive = [f for f in mine if f.get("interactive")]
    return interactive or mine


def gaps_of(pictures):
    """[(frame_before, frame_after, done_before_ms, done_after_ms, gap_ms)] between consecutive accepted pictures."""
    return [(a["shown_frame"], b["shown_frame"], a["done_ms"], b["done_ms"], round(b["done_ms"] - a["done_ms"], 3))
            for a, b in zip(pictures, pictures[1:])]


def locate(frame_after, cuts, prefetch_frames, entry_window=ENTRY_WINDOW):
    """Where a gap that ends on `frame_after` sits: before a cut (inside the prefetch window), at a cut, or elsewhere."""
    for cut in cuts:
        if cut <= frame_after <= cut + entry_window:
            return f"cut:{cut}"
    for cut in cuts:
        if cut - prefetch_frames <= frame_after < cut:
            return f"prefetch:{cut}"
    return "other"


def overlap_ms(a0, a1, b0, b1):
    return max(0.0, min(a1, b1) - max(a0, b0))


def events_in(events, kind, t0, t1):
    return [e for e in events if e["kind"] == kind and t0 <= e["t_ms"] <= t1]


def gap_context(events, t0, t1):
    """What the host was doing around one gap [t0, t1] (host clock): a CORRELATION, not a cause."""
    ticks = events_in(events, "tick", t0, t1)
    callbacks = [e for e in events if e["kind"] == "asset_cb"
                 and overlap_ms(e["fields"]["recv_ms"], e["fields"]["recv_ms"] + e["fields"]["callback_ms"], t0, t1) > 0]
    reads = [e for e in events if e["kind"] == "asset_io"
             and overlap_ms(e["t_ms"] - e["fields"].get("read_ms", 0) - e["fields"].get("respond_ms", 0), e["t_ms"], t0, t1) > 0]
    prefetch = events_in(events, "prefetch", t0, t1)
    sync = events_in(events, "sync", t0, t1)
    return {
        "ticks": len(ticks),
        "max_tick_late_ms": max((e["fields"]["late_ms"] for e in ticks), default=None),
        "max_tick_body_ms": max((e["fields"].get("body_ms") or 0 for e in ticks), default=None),
        "ui_callback_ms": round(sum(overlap_ms(e["fields"]["recv_ms"], e["fields"]["recv_ms"] + e["fields"]["callback_ms"], t0, t1)
                                    for e in callbacks), 3),
        "ui_callbacks": len(callbacks),
        "reads": len(reads),
        "read_bytes": sum(e["fields"].get("bytes", 0) for e in reads),
        "read_ms_max": max((e["fields"].get("read_ms", 0) for e in reads), default=None),
        "queue_wait_ms_max": max((e["fields"].get("queue_wait_ms", 0) for e in reads), default=None),
        "prefetch_ms_max": max((e["fields"].get("total_ms") or 0 for e in prefetch), default=None),
        "plans_ahead_ms_max": max((e["fields"].get("plans_ahead_ms") or 0 for e in prefetch), default=None),
        "sync_ms_max": max((e["fields"].get("total_ms") or 0 for e in sync), default=None),
    }


def quiet_class(external_cores):
    """`quiet` / `loaded` for a session from its external CPU samples (cores), by the thresholds fixed in this file."""
    if not external_cores:
        return "unknown"
    median = statistics.median(external_cores)
    if median <= QUIET_MEDIAN_CORES and percentile(external_cores, 0.95) <= QUIET_P95_CORES:
        return "quiet"
    return "loaded"


def session_validity(session):
    """(valid, [reasons]) of one session: functional validity, never a performance judgement."""
    reasons = []
    mode = session["mode"]
    status = session.get("status") or {}
    asset_io = status.get("asset_io") or {}
    if session.get("error"):
        reasons.append(f"error: {session['error']}")
    gone = session.get("editor_exit") or {}
    if gone and not gone.get("alive", True):
        reasons.append(f"the editor process ended by itself (return code {gone.get('returncode_before_stop')})")
    if asset_io.get("mode") != mode:
        reasons.append(f"the host reports mode {asset_io.get('mode')!r}, the arm asked for {mode!r}")
    pool = asset_io.get("pool") or {}
    if mode == "async":
        if (pool.get("saturated") or 0) > 0:
            reasons.append(f"the read queue saturated {pool['saturated']} time(s)")
        if (pool.get("panicked") or 0) > 0:
            reasons.append("a read panicked")
        if (pool.get("stale") or 0) > 0:
            reasons.append(f"{pool['stale']} read(s) were answered stale during a play")
    if session.get("events_lost"):
        reasons.append(f"{session['events_lost']} trace event(s) were lost")
    if not session.get("plays_ok", False):
        reasons.append("a play did not finish normally")
    bad = [e for e in session.get("events", []) if e["kind"] == "asset_io" and e["fields"].get("status") not in ACCEPTABLE_STATUS]
    if bad:
        reasons.append(f"{len(bad)} asset request(s) answered with another status ({sorted({e['fields'].get('status') for e in bad})})")
    warmed = (status.get("stats") or {}).get("warmed")
    if not isinstance(warmed, int) or warmed <= 0:
        reasons.append("the scene warm never ran (it must be on in both arms)")
    return (not reasons), reasons


def summarize_play(frames, events, segment, project, fps_budget_ms):
    """Everything one play says, with no judgement."""
    shots = play_pictures(frames, segment)
    gaps = gaps_of(shots)
    values = [g[4] for g in gaps]
    prefetch_frames = int(round(project["fps"] * 2))
    located = {}
    slow = []
    for before, after, t0, t1, gap in gaps:
        where = locate(after, project["cuts"], prefetch_frames)
        located.setdefault(where, []).append(gap)
        if gap > SLOW_DIAGNOSTIC_MS:
            slow.append({"from_frame": before, "to_frame": after, "gap_ms": gap, "where": where, "context": gap_context(events, t0, t1)})
    ticks = events_in(events, "tick", segment["clock_ms"] or 0, shots[-1]["done_ms"] if shots else 0)
    prefetches = events_in(events, "prefetch", segment["clock_ms"] or 0, shots[-1]["done_ms"] if shots else 0)
    syncs = events_in(events, "sync", segment["clock_ms"] or 0, shots[-1]["done_ms"] if shots else 0)
    callbacks = [e for e in events if e["kind"] == "asset_cb" and (segment["clock_ms"] or 0) <= e["t_ms"] <= (shots[-1]["done_ms"] if shots else 0)]
    reads = [e for e in events if e["kind"] == "asset_io" and (segment["clock_ms"] or 0) <= e["t_ms"] <= (shots[-1]["done_ms"] if shots else 0)]
    first = next((s for s in pictures_of(frames) if s["submitted_ms"] >= segment["command_ms"]), None)
    return {
        "pictures": len(shots),
        "unique_frames": len({s["shown_frame"] for s in shots}),
        "expected_frames": project["length"],
        "interval_ms": stats(values),
        "slow_gt_62_5": sum(1 for v in values if v > SLOW_DIAGNOSTIC_MS),
        "slow_gt_100": sum(1 for v in values if v > SLOWER_DIAGNOSTIC_MS),
        "over_budget_gt_41_7": sum(1 for v in values if v > fps_budget_ms),
        "request_to_accept_ms": stats([s["submit_to_done_ms"] for s in shots]),
        "transit_ms": stats([s["transit_ms"] for s in shots]),
        "runtime_total_ms": stats([s["runtime_total_ms"] for s in shots]),
        "runtime_prepare_ms": stats([s.get("runtime_prepare_ms") for s in shots]),
        "host_prepare_ms": stats([s["prepare_ms"] for s in shots]),
        "tick_late_ms": stats([e["fields"]["late_ms"] for e in ticks]),
        "tick_body_ms": stats([e["fields"].get("body_ms") for e in ticks]),
        "plans_ahead_ms": stats([e["fields"].get("plans_ahead_ms") for e in prefetches]),
        "prefetch_total_ms": stats([e["fields"].get("total_ms") for e in prefetches]),
        "sync_ms": stats([e["fields"].get("total_ms") for e in syncs]),
        "ui_callback_ms": stats([e["fields"]["callback_ms"] for e in callbacks]),
        "ui_callback_total_ms": round(sum(e["fields"]["callback_ms"] for e in callbacks), 3),
        "reads": len(reads),
        "read_bytes": sum(e["fields"].get("bytes", 0) for e in reads),
        "read_ms": stats([e["fields"].get("read_ms") for e in reads]),
        "queue_wait_ms": stats([e["fields"].get("queue_wait_ms") for e in reads]),
        "reads_on_ui_thread": sum(1 for e in reads if e["fields"].get("on_ui_thread")),
        "by_location": {where: stats(vals) | {"slow": sum(1 for v in vals if v > SLOW_DIAGNOSTIC_MS)} for where, vals in sorted(located.items())},
        "slow_detail": slow,
        "play_command_to_first_picture_ms": round(first["done_ms"] - segment["command_ms"], 1) if first else None,
        "play_command_to_clock_ms": round(segment["clock_ms"] - segment["command_ms"], 1) if segment["clock_ms"] else None,
    }


def entries_of(frames, project):
    """Per cut: the first picture shown at the cut: its request→accept and the gap since the picture before it."""
    out = []
    shots = pictures_of(frames)
    for cut in project["cuts"]:
        for index, shot in enumerate(shots):
            if cut <= shot["shown_frame"] <= cut + ENTRY_WINDOW and index and shots[index - 1]["shown_frame"] < cut:
                out.append({"cut": cut, "shown_frame": shot["shown_frame"], "request_to_accept_ms": shot["submit_to_done_ms"],
                            "gap_ms": round(shot["done_ms"] - shots[index - 1]["done_ms"], 1)})
                break
        else:
            out.append({"cut": cut, "missing": True})
    return out


def window_report(frames, project, segment):
    """Per prefetch window and per cut window of one play: gap statistics (so a pause moved from the cut to before it shows)."""
    shots = play_pictures(frames, segment)
    gaps = gaps_of(shots)
    prefetch_frames = int(round(project["fps"] * 2))
    out = []
    for cut in project["cuts"]:
        pre = [g[4] for g in gaps if cut - prefetch_frames <= g[1] < cut]
        at = [g[4] for g in gaps if cut <= g[1] <= cut + ENTRY_WINDOW]
        out.append({"cut": cut,
                    "prefetch_window": stats(pre) | {"slow": sum(1 for v in pre if v > SLOW_DIAGNOSTIC_MS), "excess_ms": round(sum(max(0.0, v - 41.667) for v in pre), 1)},
                    "cut_window": stats(at) | {"slow": sum(1 for v in at if v > SLOW_DIAGNOSTIC_MS)}})
    return out


def memory_summary(series):
    """Median and PEAK of the editor's RSS and of the new WebKit helpers' RSS, each on its own and their sum (labelled a sum)."""
    def part(index):
        values = [row[index] for row in series]
        return {"median": round(statistics.median(values), 1), "peak": round(max(values), 1)} if values else {"median": None, "peak": None}
    return {"samples": len(series), "editor_mib": part(1), "helpers_mib": part(2),
            "rss_sum_mib": part(3), "note": "RSS, not physical footprint; helpers are the WebKit processes new since the baseline; the sum double counts shared pages"}


def pair_rows(sessions_by_pair):
    """Paired differences async - sync per pair and play, for the metrics that matter, plus the comparability of the pair."""
    rows = []
    for pair, sides in sorted(sessions_by_pair.items()):
        if "sync" not in sides or "async" not in sides:
            continue
        a, b = sides["sync"], sides["async"]
        comparable = a["quiet_class"] == "quiet" and b["quiet_class"] == "quiet" and a["valid"] and b["valid"] \
            and abs((a["external_median_cores"] or 0) - (b["external_median_cores"] or 0)) <= PAIR_MAX_DELTA_CORES
        for play in ("first", "repeat"):
            pa, pb = a["plays"].get(play), b["plays"].get(play)
            if not pa or not pb:
                continue
            def diff(key, sub=None):
                va = pa[key] if sub is None else pa[key][sub]
                vb = pb[key] if sub is None else pb[key][sub]
                return None if va is None or vb is None else round(vb - va, 3)
            rows.append({
                "pair": pair, "play": play, "comparable": comparable, "order": [a["order_index"], b["order_index"]],
                "sync": {"slow_gt_62_5": pa["slow_gt_62_5"], "slow_gt_100": pa["slow_gt_100"], "max_gap_ms": pa["interval_ms"]["max"], "p95_gap_ms": pa["interval_ms"]["p95"]},
                "async": {"slow_gt_62_5": pb["slow_gt_62_5"], "slow_gt_100": pb["slow_gt_100"], "max_gap_ms": pb["interval_ms"]["max"], "p95_gap_ms": pb["interval_ms"]["p95"]},
                "delta_async_minus_sync": {
                    "slow_gt_62_5": diff("slow_gt_62_5"), "slow_gt_100": diff("slow_gt_100"),
                    "max_gap_ms": diff("interval_ms", "max"), "p95_gap_ms": diff("interval_ms", "p95"),
                    "p50_gap_ms": diff("interval_ms", "p50"),
                    "request_to_accept_p95_ms": diff("request_to_accept_ms", "p95"),
                },
            })
    return rows


UI_STALL_MS = 20.0  # a UI-thread protocol callback this long overlapping a gap is reported as "with a UI-thread stall"


def arm_groups(analyzed):
    """Valid sessions pooled per (mode, load class): what the slow gaps (> 62.5 ms) overlapped. Counts only; frames of a run
    are correlated, so these are descriptions, not samples of an effect size. Invalid sessions are not pooled (they stay listed)."""
    groups = {}
    for item in analyzed:
        if not item["valid"]:
            continue
        group = groups.setdefault(f"{item['mode']}/{item['quiet_class']}", {
            "sessions": 0, "plays": 0, "pictures": 0, "gaps_gt_62_5": 0, "gaps_gt_100": 0, "with_ui_stall": 0,
            "in_prefetch_window": 0, "at_cut": 0, "other": 0, "with_read_overlap": 0, "worst_gap_ms_per_session": []})
        group["sessions"] += 1
        worst = 0.0
        for play in item["plays"].values():
            group["plays"] += 1
            group["pictures"] += play["pictures"]
            group["gaps_gt_62_5"] += play["slow_gt_62_5"]
            group["gaps_gt_100"] += play["slow_gt_100"]
            worst = max(worst, play["interval_ms"]["max"] or 0.0)
            for gap in play["slow_detail"]:
                context = gap["context"]
                group["with_ui_stall"] += 1 if (context["ui_callback_ms"] or 0) >= UI_STALL_MS else 0
                group["with_read_overlap"] += 1 if context["reads"] else 0
                where = gap["where"]
                group["in_prefetch_window" if where.startswith("prefetch") else "at_cut" if where.startswith("cut") else "other"] += 1
        group["worst_gap_ms_per_session"].append(round(worst, 1))
    return groups


# ---------------------------------------------------------------------------------------------------- charts
PALETTE = {"sync": "#c2410c", "async": "#0369a1", "grid": "#d4d4d8", "text": "#27272a", "cut": "#16a34a", "pre": "#fef3c7"}


def svg_gaps(title, project, series, width=900, height=300):
    """Interval between accepted pictures (ms) against the frame shown after the gap; one polyline per mode."""
    left, right, top, bottom = 52, 16, 28, 36
    xs = [g[1] for gaps in series.values() for g in gaps]
    ys = [g[4] for gaps in series.values() for g in gaps]
    if not xs:
        return f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="40"><text x="8" y="24">{title}: no data</text></svg>'
    x0, x1 = 0, max(project["length"], max(xs))
    ymax = max(150.0, max(ys) * 1.05)
    px = lambda x: left + (x - x0) / (x1 - x0) * (width - left - right)
    py = lambda y: height - bottom - y / ymax * (height - top - bottom)
    out = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" font-family="sans-serif" font-size="11">',
           f'<rect width="{width}" height="{height}" fill="white"/>',
           f'<text x="{left}" y="16" fill="{PALETTE["text"]}" font-size="13">{title}</text>']
    prefetch_frames = int(round(project["fps"] * 2))
    for cut in project["cuts"]:
        out.append(f'<rect x="{px(cut - prefetch_frames):.1f}" y="{top}" width="{px(cut) - px(cut - prefetch_frames):.1f}" height="{height - top - bottom}" fill="{PALETTE["pre"]}"/>')
        out.append(f'<line x1="{px(cut):.1f}" x2="{px(cut):.1f}" y1="{top}" y2="{height - bottom}" stroke="{PALETTE["cut"]}" stroke-width="1.5"/>')
    for level, label in ((1000 / project["fps"], "budget 41.7"), (SLOW_DIAGNOSTIC_MS, "diag 62.5"), (SLOWER_DIAGNOSTIC_MS, "diag 100")):
        out.append(f'<line x1="{left}" x2="{width - right}" y1="{py(level):.1f}" y2="{py(level):.1f}" stroke="{PALETTE["grid"]}" stroke-dasharray="4 3"/>')
        out.append(f'<text x="{width - right - 4}" y="{py(level) - 2:.1f}" text-anchor="end" fill="#71717a">{label}</text>')
    out.append(f'<line x1="{left}" x2="{left}" y1="{top}" y2="{height - bottom}" stroke="#71717a"/><line x1="{left}" x2="{width - right}" y1="{height - bottom}" y2="{height - bottom}" stroke="#71717a"/>')
    for mode, gaps in series.items():
        points = " ".join(f"{px(g[1]):.1f},{py(min(g[4], ymax)):.1f}" for g in gaps)
        out.append(f'<polyline points="{points}" fill="none" stroke="{PALETTE[mode]}" stroke-width="1.1" opacity="0.9"/>')
    legend = left + 8
    for mode in series:
        out.append(f'<rect x="{legend}" y="{top + 2}" width="10" height="10" fill="{PALETTE[mode]}"/><text x="{legend + 14}" y="{top + 11}" fill="{PALETTE["text"]}">{mode}</text>')
        legend += 70
    out.append(f'<text x="{(left + width - right) / 2}" y="{height - 8}" text-anchor="middle" fill="#52525b">frame shown after the gap (shaded: 2 s before a cut; green: cut)</text>')
    out.append(f'<text x="14" y="{(top + height - bottom) / 2}" transform="rotate(-90 14 {(top + height - bottom) / 2})" text-anchor="middle" fill="#52525b">interval (ms)</text>')
    out.append('</svg>')
    return "\n".join(out)


def svg_spans(title, events, frames, t0, t1, width=900):
    """Lanes of host spans in [t0, t1] (host clock): UI callbacks, reads, ticks (lateness), prefetch, sync and the gaps of pictures."""
    lanes = ["pictures (gap)", "UI callback", "asset read", "tick late", "prefetch", "sync"]
    left, top, lane_h = 118, 26, 22
    height = top + lane_h * len(lanes) + 34
    px = lambda t: left + (t - t0) / (t1 - t0) * (width - left - 12)
    out = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" font-family="sans-serif" font-size="11">',
           f'<rect width="{width}" height="{height}" fill="white"/>',
           f'<text x="8" y="16" font-size="13" fill="{PALETTE["text"]}">{title} (host clock, ms)</text>']
    for index, lane in enumerate(lanes):
        y = top + index * lane_h
        out.append(f'<text x="6" y="{y + 14}" fill="#52525b">{lane}</text><line x1="{left}" x2="{width - 12}" y1="{y + lane_h}" y2="{y + lane_h}" stroke="{PALETTE["grid"]}"/>')

    def bar(lane, start, end, color, tip):
        if end < t0 or start > t1:
            return
        x = px(max(start, t0))
        w = max(1.0, px(min(end, t1)) - x)
        out.append(f'<rect x="{x:.1f}" y="{top + lane * lane_h + 4}" width="{w:.1f}" height="{lane_h - 8}" fill="{color}" opacity="0.85"><title>{tip}</title></rect>')

    shots = pictures_of(frames)
    for a, b in zip(shots, shots[1:]):
        gap = b["done_ms"] - a["done_ms"]
        bar(0, a["done_ms"], b["done_ms"], "#dc2626" if gap > SLOW_DIAGNOSTIC_MS else "#a1a1aa", f"{gap:.1f} ms")
    for e in events:
        f = e["fields"]
        if e["kind"] == "asset_cb":
            bar(1, f["recv_ms"], f["recv_ms"] + f["callback_ms"], PALETTE[f["mode"]], f"callback {f['callback_ms']} ms ({f['outcome']})")
        elif e["kind"] == "asset_io":
            bar(2, e["t_ms"] - f.get("read_ms", 0) - f.get("respond_ms", 0), e["t_ms"], PALETTE[f["mode"]], f"read {f.get('read_ms')} ms, {f.get('bytes')} bytes")
        elif e["kind"] == "tick" and f["late_ms"] > 5:
            bar(3, f["due_ms"], f["ran_ms"], "#7c3aed", f"late {f['late_ms']:.1f} ms")
        elif e["kind"] == "prefetch" and (f.get("total_ms") or 0) > 1:
            bar(4, e["t_ms"] - f["total_ms"], e["t_ms"], "#0d9488", f"prefetch {f['total_ms']:.1f} ms (plans_ahead {f.get('plans_ahead_ms')})")
        elif e["kind"] == "sync" and (f.get("total_ms") or 0) > 1:
            bar(5, e["t_ms"] - f["total_ms"], e["t_ms"], "#ca8a04", f"sync {f['total_ms']:.1f} ms")
    out.append(f'<text x="{left}" y="{height - 8}" fill="#52525b">{t0:.0f}</text><text x="{width - 12}" y="{height - 8}" text-anchor="end" fill="#52525b">{t1:.0f}</text>')
    out.append('</svg>')
    return "\n".join(out)


# --------------------------------------------------------------------------------------- machine and the editor
def git_state():
    def run(*args):
        return subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True).stdout.strip()
    diff = run("diff", "HEAD")
    return {"sha": run("rev-parse", "HEAD"), "dirty_files": len([l for l in run("status", "--porcelain").splitlines() if l]),
            "diff_sha256": hashlib.sha256(diff.encode()).hexdigest() if diff else None}


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def process_table():
    """{pid: (ppid, rss_kib, cpu_percent, command)} of every process."""
    out = subprocess.run(["ps", "-Ao", "pid=,ppid=,rss=,pcpu=,command="], capture_output=True, text=True).stdout
    table = {}
    for line in out.splitlines():
        parts = line.split(None, 4)
        if len(parts) == 5 and parts[0].isdigit() and parts[1].isdigit() and parts[2].isdigit():
            try:
                table[int(parts[0])] = (int(parts[1]), int(parts[2]), float(parts[3]), parts[4])
            except ValueError:
                pass
    return table


class Sampler:
    """Every `every` seconds: editor RSS, RSS of the WebKit helpers that did not exist at the baseline, and the CPU of everything
    else (external load, in cores). The helper set is attributed by "new since the baseline"; see `memory_summary` for the limits."""

    def __init__(self, pid, baseline, every=POLL_SECONDS):
        self.pid, self.baseline, self.every = pid, set(baseline), every
        self.rows, self.external, self._stop = [], [], threading.Event()
        self._thread = threading.Thread(target=self._run, daemon=True)

    def _run(self):
        started = time.monotonic()
        while not self._stop.is_set():
            table = process_table()
            own = {self.pid} | {p for p, row in table.items() if "com.apple.WebKit." in row[3] and p not in self.baseline}
            editor = table.get(self.pid, (0, 0, 0, ""))[1]
            helpers = sum(table[p][1] for p in own if p != self.pid and p in table)
            external = sum(row[2] for p, row in table.items() if p not in own and p != os.getpid())
            elapsed = round(time.monotonic() - started, 1)
            self.rows.append([elapsed, round(editor / 1024, 1), round(helpers / 1024, 1), round((editor + helpers) / 1024, 1)])
            self.external.append(round(external / 100.0, 2))
            self._stop.wait(self.every)

    def start(self):
        self._thread.start()
        return self

    def stop(self):
        self._stop.set()
        self._thread.join(5)
        return self


def webkit_pids():
    return {p for p, row in process_table().items() if "com.apple.WebKit." in row[3]}


def new_workspace_copy(document, work):
    """Copies only the document into a private folder (assets are absolute paths, read in place)."""
    os.makedirs(work, exist_ok=True)
    copy = os.path.join(work, "project.tessel.json")
    shutil.copyfile(document, copy)
    return copy


def collect(lab, state, deadline):
    """One poll: new trace events (cursor), the frame table, the load average. Returns False when the editor stopped answering."""
    trace = lab.op({"op": "render.trace", "since": state["cursor"], "limit": 16384})
    if not trace.get("ok"):
        state["errors"].append(f"render.trace: {trace}")
        return False
    data = trace["data"]
    state["lost"] += data.get("lost", 0)
    state["events"].extend(data["events"])
    if data["events"]:
        state["cursor"] = data["next"]
    status = lab.report()
    merge_by_key(state["frames"], ((status.get("session") or {}).get("trace") or {}).get("recent"), "id")
    state["status"] = {"asset_io": status.get("asset_io"), "stats": (status.get("session") or {}).get("stats"),
                       "error": (status.get("session") or {}).get("error")}
    state["loadavg"].append([round(v, 2) for v in os.getloadavg()])
    return True


def flatten_events(raw):
    return [{"seq": e["seq"], "t_ms": e["t_ms"], "kind": e["kind"], "fields": e["fields"]} for e in raw]


def play_once(lab, state, name, total_frames, fps, W):
    """Plays the project from 0 to the end, polling the trace every POLL_SECONDS. Returns the playback stop report."""
    started = W.retry_while_preparing(lambda: lab.op({"op": "playback.start", "from": 0}))
    if not started.get("ok"):
        state["errors"].append(f"playback.start({name}): {started}")
        return None
    deadline = time.time() + total_frames / fps + 40
    while time.time() < deadline:
        time.sleep(POLL_SECONDS)
        collect(lab, state, deadline)
        status = lab.op({"op": "playback.status"}).get("data") or {}
        if status.get("state") != "playing":
            break
    stopped = lab.op({"op": "playback.stop"})
    report = (stopped.get("data") or {}).get("report") or {}
    played = report if "ticks" in report else (report.get("last") or {})
    time.sleep(1.0)
    collect(lab, state, deadline)
    return {key: played.get(key) for key in ("ticks", "skipped_frames", "max_gap_frames")}


def editor_exit(lab):
    """Did the editor process end before the harness stopped it? Return code (negative: signal) and the log lines that
    say why, with absolute paths reduced to their last component (the log can name private files)."""
    code = lab.process.poll()
    lines = []
    try:
        lab.log.flush()
        for line in pathlib.Path(lab.log_path).read_text(errors="replace").splitlines():
            if re.search(r"panic|fatal|abort|SIG|error|Error|crash", line):
                lines.append(re.sub(r"(/[^/\s:\"']+)+/([^/\s:\"']+)", r"…/\2", line)[:240])
    except OSError:
        pass
    return {"returncode_before_stop": code, "alive": code is None, "log_lines": lines[-12:]}


def run_session(W, binary, document, project, mode, out_dir, number, order_index, stress):
    """One fresh editor in `mode`: opens a private copy, plays the project twice, returns the raw session record."""
    work = tempfile.mkdtemp(prefix="rl-", dir="/tmp")
    record = {"number": number, "mode": mode, "order_index": order_index, "started_unix": time.time(),
              "env": {"TESSEL_GL": "gpu", "TESSEL_VIEWPORT_TRACE": "1", "TESSEL_VIEWPORT_ASSET_IO": mode},
              "host": {"ncpu": os.cpu_count(), "system": platform.platform(), "loadavg_start": [round(v, 2) for v in os.getloadavg()],
                       "free_gib_start": round(W.free_gib(), 1)}}
    state = {"cursor": 0, "events": [], "frames": {}, "lost": 0, "errors": [], "loadavg": [], "status": {}}
    baseline = webkit_pids()
    lab = None
    sampler = None
    hogs = []
    try:
        if W.free_gib() < MIN_FREE_GIB_FLOOR:
            record["error"] = f"free disk {W.free_gib():.1f} GiB is below the {MIN_FREE_GIB_FLOOR} GiB floor"
            return record
        env = dict(record["env"], TESSEL_RUNTIME_DIR=str(ROOT / "runtime"))
        lab = W.Lab(str(binary), work, env)
        lab.wait_ready()
        lab.invoke("window.activate")
        time.sleep(1.0)
        copy = new_workspace_copy(document, os.path.join(work, "copy"))
        opened = lab.op({"op": "project.open", "path": copy})
        if not opened.get("ok"):
            record["error"] = f"project.open: {opened}"
            return record
        sampler = Sampler(lab.process.pid, baseline).start()
        for _ in range(stress):
            hogs.append(subprocess.Popen([sys.executable, "-c", "while True: pass"]))
        time.sleep(2.0)
        collect(lab, state, 0)
        plays = {}
        for name in ("first", "repeat"):
            played = play_once(lab, state, name, project["length"], project["fps"], W)
            plays[name] = played
            if played is None:
                break
            time.sleep(2.0)
        record["played"] = plays
        record["plays_ok"] = all(plays.get(name) is not None for name in ("first", "repeat")) and not state["errors"]
    except Exception as error:  # a session that dies is reported, not hidden
        record["error"] = repr(error)
        record["plays_ok"] = False
    finally:
        for hog in hogs:
            hog.kill()
        if sampler:
            sampler.stop()
            record["memory_series"] = sampler.rows
            record["external_cores"] = sampler.external
        if lab:
            try:
                collect(lab, state, 0)
            except Exception:
                pass
            record["editor_exit"] = editor_exit(lab)
            lab.stop()
        shutil.rmtree(work, ignore_errors=True)
    record["events"] = flatten_events(state["events"])
    record["events_lost"] = state["lost"]
    record["frames"] = sorted(state["frames"].values(), key=lambda f: f["id"])
    record["status"] = state["status"]
    record["errors"] = state["errors"]
    record["loadavg"] = state["loadavg"]
    record["finished_unix"] = time.time()
    return record


def analyze_session(record, project):
    valid, reasons = session_validity(record)
    segments = play_segments(record.get("events", []))
    names = ["first", "repeat"]
    budget = 1000.0 / project["fps"]
    plays = {}
    for name, segment in zip(names, segments):
        plays[name] = summarize_play(record["frames"], record["events"], segment, project, budget)
        plays[name]["windows"] = window_report(record["frames"], project, segment)
        plays[name]["entries"] = entries_of([f for f in record["frames"] if segment["clock_ms"] and segment["clock_ms"] <= f["done_ms"] < segment["end_ms"]], project)
        plays[name]["playback_report"] = (record.get("played") or {}).get(name)
    external = record.get("external_cores") or []
    return {
        "number": record["number"], "mode": record["mode"], "order_index": record["order_index"],
        "valid": valid, "invalid_reasons": reasons,
        "quiet_class": quiet_class(external),
        "external_median_cores": round(statistics.median(external), 2) if external else None,
        "external_p95_cores": percentile(external, 0.95),
        "loadavg_start": record["host"]["loadavg_start"],
        "loadavg_peak_1m": max((row[0] for row in record.get("loadavg", [])), default=None),
        "memory": memory_summary(record.get("memory_series", [])),
        "asset_io": (record.get("status") or {}).get("asset_io"),
        "warmed": ((record.get("status") or {}).get("stats") or {}).get("warmed"),
        "plays": plays,
    }


def analyze(sessions, project, manifest):
    analyzed = [analyze_session(s, project) for s in sessions]
    by_pair = {}
    for item in analyzed:
        by_pair.setdefault(item["order_index"] // 2, {})[item["mode"]] = item | {"pair": item["order_index"] // 2}
    rows = pair_rows(by_pair)
    groups = {"comparable_quiet": [r for r in rows if r["comparable"]], "not_comparable_or_loaded": [r for r in rows if not r["comparable"]]}
    return {"manifest": manifest, "thresholds": {
                "diagnostic_ms": [SLOW_DIAGNOSTIC_MS, SLOWER_DIAGNOSTIC_MS], "budget_ms": round(1000.0 / project["fps"], 3),
                "quiet_median_cores": QUIET_MEDIAN_CORES, "quiet_p95_cores": QUIET_P95_CORES, "pair_max_delta_cores": PAIR_MAX_DELTA_CORES,
                "async_workers_queue": "2 workers, 16 queued jobs (fixed before measuring)"},
            "project": {k: v for k, v in project.items() if k != "sources"},
            "sessions": analyzed, "pairs": groups, "arms": arm_groups(analyzed),
            "all_valid": all(s["valid"] for s in analyzed),
            "n_comparable_pairs": len({r["pair"] for r in groups["comparable_quiet"]})}


def markdown(report):
    lines = ["# Real-project A/B of the asset protocol (BL157.1)", "",
             "A = sync, B = async, same build. Values are per session and play; deltas are B - A. "
             "Diagnostic thresholds only, not approved limits.", ""]
    manifest = report["manifest"]
    lines += [f"- code `{manifest['git']['sha'][:12]}` dirty files {manifest['git']['dirty_files']}, binary sha256 `{manifest['binary_sha256'][:16]}`, "
              f"runtime sha256 `{manifest['runtime_sha256'][:16]}`, project document sha256 `{manifest['document_sha256'][:16]}`",
              f"- sources: {manifest['sources_present']}/{manifest['sources_total']} present; cuts {report['project']['cuts']}; "
              f"{report['project']['length']} frames at {report['project']['fps']:g} fps",
              f"- comparable pairs: {report['n_comparable_pairs']} of {len(report['sessions']) // 2}; all sessions valid: {report['all_valid']}", ""]
    lines += ["## Sessions", "", "| # | mode | valid | class | ext CPU med/p95 (cores) | load1 start | RSS editor med/peak MiB | RSS helpers med/peak MiB | reads sync-thread |",
              "|---|---|---|---|---|---|---|---|---|"]
    for s in report["sessions"]:
        m = s["memory"]
        on_ui = sum(p["reads_on_ui_thread"] for p in s["plays"].values())
        lines.append(f"| {s['order_index']} | {s['mode']} | {'yes' if s['valid'] else 'NO: ' + '; '.join(s['invalid_reasons'])} | {s['quiet_class']} | "
                     f"{s['external_median_cores']}/{s['external_p95_cores']} | {s['loadavg_start'][0]} | "
                     f"{m['editor_mib']['median']}/{m['editor_mib']['peak']} | {m['helpers_mib']['median']}/{m['helpers_mib']['peak']} | {on_ui} |")
    lines += ["", "## Per session and play", "",
              "| # | mode | play | pictures | gap p50/p95/max ms | >62.5 | >100 | req→accept p95/max ms | tick late p95/max ms | UI callback max ms | skipped | first picture after Play ms |",
              "|---|---|---|---|---|---|---|---|---|---|---|---|"]
    for s in report["sessions"]:
        for name, p in s["plays"].items():
            g, r, t, c = p["interval_ms"], p["request_to_accept_ms"], p["tick_late_ms"], p["ui_callback_ms"]
            lines.append(f"| {s['order_index']} | {s['mode']} | {name} | {p['pictures']} | {g['p50']}/{g['p95']}/{g['max']} | {p['slow_gt_62_5']} | {p['slow_gt_100']} | "
                         f"{r['p95']}/{r['max']} | {t['p95']}/{t['max']} | {c['max']} | {(p['playback_report'] or {}).get('skipped_frames')} | {p['play_command_to_first_picture_ms']} |")
    lines += ["", f"## Slow gaps (> {SLOW_DIAGNOSTIC_MS} ms) by arm and load class (valid sessions only; descriptive counts)", "",
              f"| group | sessions | plays | pictures | gaps >{SLOW_DIAGNOSTIC_MS} | >100 | with UI-thread callback ≥{UI_STALL_MS:g} ms | overlapping a read | in a prefetch window | at a cut | elsewhere | worst gap per session ms |",
              "|---|---|---|---|---|---|---|---|---|---|---|---|"]
    for name, g in sorted(report["arms"].items()):
        lines.append(f"| {name} | {g['sessions']} | {g['plays']} | {g['pictures']} | {g['gaps_gt_62_5']} | {g['gaps_gt_100']} | {g['with_ui_stall']} | "
                     f"{g['with_read_overlap']} | {g['in_prefetch_window']} | {g['at_cut']} | {g['other']} | {g['worst_gap_ms_per_session']} |")
    lines += ["", "## Paired differences (B - A)", ""]
    for group, rows in report["pairs"].items():
        lines += [f"### {group}", "", "| pair | play | >62.5 sync→async | >100 sync→async | max gap sync→async ms | p95 gap sync→async ms | Δ>62.5 | Δmax ms |", "|---|---|---|---|---|---|---|---|"]
        for r in rows:
            d = r["delta_async_minus_sync"]
            lines.append(f"| {r['pair']} | {r['play']} | {r['sync']['slow_gt_62_5']}→{r['async']['slow_gt_62_5']} | {r['sync']['slow_gt_100']}→{r['async']['slow_gt_100']} | "
                         f"{r['sync']['max_gap_ms']}→{r['async']['max_gap_ms']} | {r['sync']['p95_gap_ms']}→{r['async']['p95_gap_ms']} | {d['slow_gt_62_5']} | {d['max_gap_ms']} |")
        lines.append("")
    lines += ["## Gaps above 62.5 ms (where, and what the host was doing)", "",
              "| # | mode | play | frames | gap ms | where | tick late ms | UI callback ms | reads (bytes) | read ms max | prefetch ms max | plans_ahead ms max | sync ms max |",
              "|---|---|---|---|---|---|---|---|---|---|---|---|---|"]
    for s in report["sessions"]:
        for name, p in s["plays"].items():
            for gap in p["slow_detail"]:
                c = gap["context"]
                lines.append(f"| {s['order_index']} | {s['mode']} | {name} | {gap['from_frame']}→{gap['to_frame']} | {gap['gap_ms']} | {gap['where']} | {c['max_tick_late_ms']} | "
                             f"{c['ui_callback_ms']} | {c['reads']} ({c['read_bytes']}) | {c['read_ms_max']} | {c['prefetch_ms_max']} | {c['plans_ahead_ms_max']} | {c['sync_ms_max']} |")
    return "\n".join(lines) + "\n"


def write_charts(sessions, project, report, out):
    out.mkdir(parents=True, exist_ok=True)
    by_pair = {}
    for s in sessions:
        by_pair.setdefault(s["order_index"] // 2, {})[s["mode"]] = s
    names = []
    for pair, sides in sorted(by_pair.items()):
        for play_index, play in enumerate(("first", "repeat")):
            series = {}
            for mode, record in sides.items():
                segments = play_segments(record.get("events", []))
                if play_index < len(segments):
                    series[mode] = gaps_of(play_pictures(record["frames"], segments[play_index]))
            if series:
                name = f"gaps-pair{pair}-{play}.svg"
                (out / name).write_text(svg_gaps(f"pair {pair}, {play} play: interval between pictures", project, series))
                names.append(name)
    for s in sessions:
        segments = play_segments(s.get("events", []))
        if segments and project["cuts"] and segments[0]["clock_ms"] is not None:
            cut = project["cuts"][0]
            shots = [x for x in play_pictures(s["frames"], segments[0]) if x["shown_frame"] >= cut - int(project["fps"] * 2) - 2]
            if not shots:
                continue
            t0 = next((x["done_ms"] for x in shots), 0) - 300
            t1 = next((x["done_ms"] for x in shots if x["shown_frame"] >= cut + 6), shots[-1]["done_ms"]) + 300
            name = f"spans-session{s['order_index']}-{s['mode']}-first-cut{cut}.svg"
            (out / name).write_text(svg_spans(f"session {s['order_index']} ({s['mode']}), first play, window before cut {cut}", s["events"], s["frames"], t0, t1))
            names.append(name)
    return names


# --------------------------------------------------------------------------------------------------- commands
def cmd_preflight(args):
    document = pathlib.Path(args.project)
    data = json.loads(document.read_text())
    project = parse_project(data)
    checked = check_sources(project["sources"])
    summary = {"document_sha256": sha256_file(document), "length": project["length"], "fps": project["fps"], "cuts": project["cuts"],
               "clip_kinds": project["clip_kinds"], "sources_total": len(checked), "sources_present": sum(1 for c in checked if c["present"]),
               "sources": checked}
    print(json.dumps(summary, indent=2))
    return 0 if summary["sources_present"] == summary["sources_total"] else 2


def cmd_run(args):
    W = load_module("scene_warm_proof", ROOT / "scripts" / "scene-warm-proof.py")
    binary = pathlib.Path(args.binary).resolve()
    document = pathlib.Path(args.project).resolve()
    for needed in (binary, ROOT / "runtime" / "dist" / "runtime.js", document):
        if not needed.exists():
            print(f"BLOCKED: missing {needed}")
            return 2
    project = parse_project(json.loads(document.read_text()))
    sources = check_sources(project["sources"])
    missing = [s["label"] for s in sources if not s["present"]]
    if missing:
        print(f"BLOCKED: {len(missing)} source(s) of the project are missing: {missing}")
        return 2
    if W.free_gib() < MIN_FREE_GIB_START:
        print(f"BLOCKED: {W.free_gib():.1f} GiB free, the run needs {MIN_FREE_GIB_START}")
        return 2
    locks = W.acquire_locks(args.lock_wait)
    if locks is None:
        print(f"BLOCKED: the GPU bench locks were not free within {args.lock_wait:.0f} s")
        return 2
    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    manifest = {"git": git_state(), "binary_sha256": sha256_file(binary), "runtime_sha256": sha256_file(ROOT / "runtime" / "dist" / "runtime.js"),
                "document_sha256": sha256_file(document), "sources": sources, "sources_total": len(sources),
                "sources_present": len(sources), "pairs": args.pairs, "order": pair_order(args.pairs), "stress_cpu": args.stress_cpu,
                "poll_seconds": POLL_SECONDS, "started_unix": time.time(), "label": args.label}
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2))
    print(f"build {manifest['git']['sha'][:9]} dirty={manifest['git']['dirty_files']} binary {manifest['binary_sha256'][:12]}", flush=True)
    sessions = []
    try:
        for index, mode in enumerate(manifest["order"]):
            print(f"session {index} ({mode})", flush=True)
            record = run_session(W, binary, document, project, mode, out, index, index, args.stress_cpu)
            (out / f"session-{index:02d}-{mode}.json").write_text(json.dumps(record))
            sessions.append(record)
            valid, reasons = session_validity(record)
            print(f"  valid={valid} {reasons}", flush=True)
    finally:
        for handle in locks:
            handle.close()
    manifest["finished_unix"] = time.time()
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2))
    return finish(out, sessions, project, manifest, args.report_dir)


def finish(out, sessions, project, manifest, report_dir):
    report = analyze(sessions, project, manifest)
    (out / "analysis.json").write_text(json.dumps(report, indent=2))
    (out / "analysis.md").write_text(markdown(report))
    charts = write_charts(sessions, project, report, out / "charts")
    if report_dir:
        target = pathlib.Path(report_dir)
        target.mkdir(parents=True, exist_ok=True)
        (target / "analysis.json").write_text(json.dumps(report, indent=2))
        (target / "analysis.md").write_text(markdown(report))
        for name in charts:
            shutil.copyfile(out / "charts" / name, target / name)
    print(markdown(report))
    return 0 if report["all_valid"] and report["n_comparable_pairs"] == len(sessions) // 2 else 1


def cmd_analyze(args):
    out = pathlib.Path(args.out)
    manifest = json.loads((out / "manifest.json").read_text())
    project = parse_project(json.loads(pathlib.Path(args.project).read_text())) if args.project else None
    if project is None:
        raise SystemExit("analyze needs --project (the document the sessions played)")
    sessions = [json.loads(p.read_text()) for p in sorted(out.glob("session-*.json"))]
    return finish(out, sessions, project, manifest, args.report_dir)


def cmd_pixels(args):
    """Explicit-frame pixels of the SYNTHETIC project are identical across the two modes and hot/cold (no private media)."""
    W = load_module("scene_warm_proof", ROOT / "scripts" / "scene-warm-proof.py")
    binary = pathlib.Path(args.binary).resolve()
    if W.free_gib() < MIN_FREE_GIB_START:
        print(f"BLOCKED: {W.free_gib():.1f} GiB free")
        return 2
    locks = W.acquire_locks(args.lock_wait)
    if locks is None:
        print("BLOCKED: the GPU bench locks were not free")
        return 2
    original_env = W.env_for
    original_retry = W.retry_while_preparing
    refused = []

    def recording_retry(call, *rest, **named):
        answer = original_retry(call, *rest, **named)
        # A frame request replaced by a newer one (the monitor's own refine) is a wait, not a verdict: the explicit
        # render is idempotent, so it is asked again. Every refusal stays in the report (it is a race of the harness,
        # seen in both modes, not a pixel result).
        for _ in range(4):
            if not (isinstance(answer, dict) and not answer.get("ok")):
                break
            refused.append({"mode": mode_box["mode"], "answer": str(answer)[:300]})
            if ((answer.get("error") or {}).get("code")) != "cancelled":
                break
            time.sleep(1.0)
            answer = original_retry(call, *rest, **named)
        return answer

    W.retry_while_preparing = recording_retry
    mode_box = {"mode": "sync"}
    W.env_for = lambda arm, inspect: {**original_env("warm", inspect), "TESSEL_VIEWPORT_ASSET_IO": mode_box["mode"], "TESSEL_GL": "gpu"}
    work = tempfile.mkdtemp(prefix="rlpx-", dir="/tmp")
    results = []
    try:
        cuts, cut_frames = 5, 72
        document = W.build_project(str(binary), os.path.join(work, "build"), 200000, cuts, cut_frames)
        if not document:
            return 1
        frames = sorted({cut_frames + 5, 2 * cut_frames + 5, (cuts - 1) * cut_frames + 5, cut_frames + 40})
        number = 0
        for mode in MODES:
            for order in ("hot", "cold"):
                mode_box["mode"] = mode
                session = W.pixel_session(str(binary), document, "warm", order, frames, cut_frames, os.path.join(work, f"px{number}"))
                session["mode"] = mode
                results.append(session)
                number += 1
    finally:
        W.env_for = original_env
        W.retry_while_preparing = original_retry
        for handle in locks:
            handle.close()
        shutil.rmtree(work, ignore_errors=True)
    verdicts = W.pixel_verdicts(results, frames)
    failures = [name for name, ok, _ in verdicts if not ok]
    for name, ok, detail in verdicts:
        print(("PASS " if ok else "FAIL ") + name + ("" if ok else f"  [{str(detail)[:200]}]"))
    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    (out / "pixels.json").write_text(json.dumps({"frames": frames, "sessions": results,
                                                  "refused_requests": refused,
                                                  "verdicts": [{"check": n, "ok": ok} for n, ok, _ in verdicts]}, indent=2))
    for item in refused:
        print("REFUSED", item["mode"], item["answer"])
    return 1 if failures or any(s.get("error") for s in results) else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    pre = sub.add_parser("preflight")
    pre.add_argument("--project", required=True)
    run = sub.add_parser("run")
    run.add_argument("--binary", default=str(ROOT / "target" / "release" / "tessel-editor"))
    run.add_argument("--project", required=True)
    run.add_argument("--out", required=True)
    run.add_argument("--report-dir")
    run.add_argument("--pairs", type=int, default=4)
    run.add_argument("--lock-wait", type=float, default=600.0)
    run.add_argument("--stress-cpu", type=int, default=0, help="busy processes of this script, the same in both arms (adverse batch)")
    run.add_argument("--label", default="")
    ana = sub.add_parser("analyze")
    ana.add_argument("--out", required=True)
    ana.add_argument("--project")
    ana.add_argument("--report-dir")
    pix = sub.add_parser("pixels")
    pix.add_argument("--binary", default=str(ROOT / "target" / "release" / "tessel-editor"))
    pix.add_argument("--out", required=True)
    pix.add_argument("--lock-wait", type=float, default=600.0)
    args = parser.parse_args()
    return {"preflight": cmd_preflight, "run": cmd_run, "analyze": cmd_analyze, "pixels": cmd_pixels}[args.command](args)


if __name__ == "__main__":
    raise SystemExit(main())
