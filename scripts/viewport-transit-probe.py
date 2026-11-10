#!/usr/bin/env python3
"""BL157.2: where does a picture that arrives late spend its time? A reproduction loop and the decomposition of each late picture.

    python3 scripts/viewport-transit-probe.py loop    --binary target/release/tessel-editor --project P.tessel.json --out DIR
                  [--io async|sync] [--iterations 3] [--from F] [--to T] [--report-dir DIR2]
    python3 scripts/viewport-transit-probe.py ab      --binary ... --project P.tessel.json --out DIR [--pairs 4] [--io async]
    python3 scripts/viewport-transit-probe.py analyze --out DIR [--report-dir DIR2]

`loop` plays a WINDOW of the real project (default: from 3 s before the first cut to a second after it, where the
instrumented runs found the long pauses) in a FRESH editor per iteration (fresh, because the pause follows the first load of a
splat: a second play in the same process finds it resident), with the asset I/O mode fixed (`--io`, async by default so the UI
thread is not also stalled by reads) and the trace and page timing ON. It is RED-CAPABLE: it exits 1 when any iteration shows a
gap above the diagnostic threshold, a UI block (a host step holding the window's thread) or a play that shows almost no
pictures, and prints which. A loop that cannot fail would prove nothing about a change.

Every figure is a duration on ONE clock. Host: send of the request, the WebView's completion of the script, arrival of the
answer at the WebView's IPC handler, start of the window's task. Page: the time the request was held, the wait before it started
(its queue), the lag of its event loop. Nothing here subtracts a page, a host and a Python clock. For one late picture:

    total (host)  =  host_pre_send  +  [ delivery to the page + page_held + delivery back ]  +  ipc_wait  +  accept
    delivery      =  total - host_pre_send - page_held - ipc_wait      (inclusive: both ways, not split by this script)

and the dominant part is named. `eval_ms` (send to the script's completion on the page, seen from the host) says how much of the
delivery was on the way IN. Correlation, not proof of a mechanism; a causal change is a separate experiment (BL157.2).

Exit: 0 no gap above the diagnostic threshold in any iteration; 1 some gap (red); 2 a prerequisite is missing (BLOCKED).
"""
import argparse
import json
import math
import os
import pathlib
import statistics
import sys
import tempfile
import time
import shutil
import importlib.util

ROOT = pathlib.Path(__file__).resolve().parent.parent
SLOW_MS = 62.5
# A component is "dominant" only when it is at least this much of the picture's time and at least this long.
DOMINANT_MIN_MS = 25.0
LATE_TRANSIT_MS = 30.0
# A host step (the prefetch tick) that holds the window's thread this long is a UI block, not a slow picture: nothing is drawn or
# delivered meanwhile. A play that shows fewer pictures than this fraction of its frames was starved.
UI_BLOCK_MS = 50.0
STARVED_FRACTION = 0.25


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


RL = load_module("real_latency_ab", ROOT / "scripts" / "real-latency-ab.py")


# ------------------------------------------------------------------------------------------------ pure logic
def eval_events(events):
    """The WebView's completion of each `frame.render` script: (send time on the host clock, ms from send to completion)."""
    out = []
    for e in events:
        if e["kind"] == "eval_done" and e["fields"].get("kind") == "frame.render":
            out.append({"sent_ms": e["t_ms"] - e["fields"]["ms"], "done_ms": e["t_ms"], "eval_ms": e["fields"]["ms"], "used": False})
    return sorted(out, key=lambda x: x["sent_ms"])


def match_eval(evals, submitted_ms):
    """The first unused completion whose script was sent at or after the submission (scripts leave in order)."""
    for item in evals:
        if not item["used"] and item["sent_ms"] >= submitted_ms - 0.05:
            item["used"] = True
            return item
    return None


def decompose(frame, page_event, evaluation):
    """One accepted picture's time, split into the parts one clock can tell apart, and the dominant one named."""
    total = frame["submit_to_done_ms"]
    page = (page_event or {}).get("page") or {}
    held = page.get("held_ms")
    queue = page.get("queue_ms")
    ipc_wait = (page_event or {}).get("ipc_wait_ms") or 0.0
    pre_send = max(0.0, evaluation["sent_ms"] - frame["submitted_ms"]) if evaluation else None
    parts = {"host_pre_send": pre_send, "page_queue": queue, "page_draw": (held - queue) if held is not None and queue is not None else None,
             "ipc_wait": ipc_wait if page_event else None}
    known = sum(v for v in parts.values() if v is not None)
    # Delivery both ways: everything the host waited for that no part above accounts for. Only meaningful with the page account.
    delivery = max(0.0, total - known) if held is not None else None
    parts["delivery"] = delivery
    named = {k: v for k, v in parts.items() if v is not None}
    dominant = max(named, key=named.get) if named else None
    if dominant is not None and (named[dominant] < DOMINANT_MIN_MS or named[dominant] < 0.4 * total):
        dominant = "spread" if total >= SLOW_MS / 2 else "none"
    detail = None
    if dominant == "page_queue":
        loop = (page.get("loop") or {}).get("max_ms") or 0.0
        ahead = page.get("ahead") or {}
        if loop >= 0.7 * (queue or 0):
            detail = "event_loop_stalled"
        elif (ahead.get("frames") or 0) + (ahead.get("warms") or 0) > 0:
            detail = "behind_queued_work"
        else:
            detail = "unexplained_by_loop_or_queue"
    if dominant == "delivery" and evaluation:
        detail = "in_dominant" if evaluation["eval_ms"] >= 0.7 * delivery else "out_or_later_dominant"
    return {"id": frame["id"], "shown_frame": frame.get("shown_frame"), "total_ms": round(total, 3), "transit_ms": round(frame["transit_ms"], 3),
            "runtime_total_ms": frame["runtime_total_ms"], "eval_ms": round(evaluation["eval_ms"], 3) if evaluation else None,
            "page_loop_max_ms": (page.get("loop") or {}).get("max_ms"), "page_ahead": page.get("ahead"),
            "parts": {k: (round(v, 3) if v is not None else None) for k, v in parts.items()},
            "dominant": dominant, "detail": detail}


def decompose_play(frames, events, segment):
    pictures = RL.play_pictures(frames, segment)
    by_id = {e["fields"]["id"]: e["fields"] for e in events if e["kind"] == "frame_page"}
    # The host's own `frame_page` event carries the page's account; keyed by the frame id.
    evals = eval_events(events)
    out = []
    for picture in sorted(pictures, key=lambda f: f["submitted_ms"]):
        out.append(decompose(picture, by_id.get(picture["id"]), match_eval(evals, picture["submitted_ms"])))
    return out


def late_summary(decomposed, gaps):
    """For the pictures that END a slow gap and for every late picture: counts by dominant part."""
    ids_ending_slow = {gap["to_id"] for gap in gaps if gap["gap_ms"] > SLOW_MS}
    def count(items):
        out = {}
        for item in items:
            key = item["dominant"] if not item["detail"] else f"{item['dominant']}:{item['detail']}"
            out[key] = out.get(key, 0) + 1
        return out
    late = [d for d in decomposed if d["transit_ms"] >= LATE_TRANSIT_MS]
    slow = [d for d in decomposed if d["id"] in ids_ending_slow]
    return {"pictures": len(decomposed), "late_transit_ge_30": len(late), "late_by_part": count(late),
            "slow_gap_pictures": len(slow), "slow_gap_by_part": count(slow)}


def gaps_with_ids(pictures):
    """Gaps between consecutive pictures of a play. A picture of the SAME frame as the one before it is the settle/refine picture
    after the play stopped (the playhead no longer advances), not a playback gap, so the time to it is not counted."""
    shots = sorted(pictures, key=lambda f: (f["done_ms"], f["id"]))
    return [{"from_frame": a["shown_frame"], "to_frame": b["shown_frame"], "to_id": b["id"], "gap_ms": round(b["done_ms"] - a["done_ms"], 3),
             "done_ms": b["done_ms"]} for a, b in zip(shots, shots[1:]) if a["shown_frame"] != b["shown_frame"]]


def ui_blocks(events):
    """Prefetch steps that held the window's thread for UI_BLOCK_MS or more, each with what the per-asset events say about where
    the time went (`fs_ms`: file checks, `send_ms`: handing the load to the backend). Durations on the host's own clock."""
    assets = [e for e in events if e["kind"] == "prefetch_asset"]
    out = []
    for e in events:
        if e["kind"] != "prefetch" or (e["fields"].get("total_ms") or 0) < UI_BLOCK_MS:
            continue
        f = e["fields"]
        began = e["t_ms"] - f["total_ms"]
        inside = [a["fields"] for a in assets if began - 5 <= a["t_ms"] <= e["t_ms"] + 5]
        out.append({"at_ms": round(began, 1), "total_ms": round(f["total_ms"], 1), "prefetch_ms": f.get("prefetch_ms"), "plans_ahead_ms": f.get("plans_ahead_ms"),
                    "warm_ms": f.get("warm_ms"), "loads_started": f.get("loads_started"),
                    "assets": [{k: a.get(k) for k in ("kind", "bytes", "fs_ms", "send_ms", "ok")} for a in inside]})
    return out


def analyze_iteration(record):
    events = record.get("events", [])
    frames = record.get("frames", [])
    segments = RL.play_segments(events)
    plays = []
    for segment in segments:
        pictures = RL.play_pictures(frames, segment)
        gaps = gaps_with_ids(pictures)
        decomposed = decompose_play(frames, events, segment)
        slow = [g for g in gaps if g["gap_ms"] > SLOW_MS]
        by_id = {d["id"]: d for d in decomposed}
        loop_stalls = [e["fields"] for e in events if e["kind"] == "runtime_span" and e["fields"].get("k") == "loop_stall"]
        start, end = record.get("window") or (0, 0)
        blocks = ui_blocks(events)
        plays.append({"pictures": len(pictures), "gap_ms": RL.stats([g["gap_ms"] for g in gaps]), "slow_gaps": len(slow),
                      "ui_blocks": blocks, "starved": len(pictures) < max(1, (end - start) * STARVED_FRACTION),
                      "slow_detail": [g | {"picture": by_id.get(g["to_id"])} for g in slow],
                      "summary": late_summary(decomposed, gaps),
                      "page_loop_stalls": {"n": len(loop_stalls), "max_ms": max((s["ms"] for s in loop_stalls), default=None)},
                      "ipc_wait_ms": RL.stats([e["fields"].get("ipc_wait_ms") for e in events if e["kind"] == "frame_page"]),
                      "eval_ms": RL.stats([x["eval_ms"] for x in eval_events(events)])})
    valid, reasons = validity(record)
    return {"iteration": record["iteration"], "io": record["io"], "valid": valid, "invalid_reasons": reasons,
            "quiet_class": RL.quiet_class(record.get("external_cores") or []),
            "external_median_cores": round(statistics.median(record["external_cores"]), 2) if record.get("external_cores") else None,
            "window": record["window"], "plays": plays}


def validity(record):
    reasons = []
    if record.get("error"):
        reasons.append("error: " + RL.redact(record["error"]))
    gone = record.get("editor_exit") or {}
    if gone and not gone.get("alive", True):
        reasons.append(f"the editor process ended by itself (return code {gone.get('returncode_before_stop')})")
    if record.get("events_lost"):
        reasons.append(f"{record['events_lost']} trace event(s) were lost")
    asset_io = (record.get("status") or {}).get("asset_io") or {}
    if asset_io.get("mode") != record["io"]:
        reasons.append(f"the host reports mode {asset_io.get('mode')!r}, asked {record['io']!r}")
    pool = asset_io.get("pool") or {}
    if (pool.get("saturated") or 0) > 0:
        reasons.append("the read queue saturated")
    if not any(e["kind"] == "frame_page" for e in record.get("events", [])):
        reasons.append("no frame carried the page account (the page timing was not on)")
    if not record.get("plays_ok", False):
        reasons.append("the window did not play to its end")
    bad = [e for e in record.get("events", []) if e["kind"] == "asset_io" and e["fields"].get("status") not in RL.ACCEPTABLE_STATUS]
    if bad:
        reasons.append(f"{len(bad)} asset request(s) answered with another status")
    return (not reasons), reasons


def markdown(report):
    lines = ["# Transit probe (BL157.2)", "",
             "Late pictures decomposed on one clock at a time (see the script's docstring). Diagnostic thresholds only, not approved limits. "
             "`valid` says the harness measured what it claims; a starved play or a UI block is a RED finding about the editor, not an invalid run.", ""]
    manifest = report["manifest"]
    lines += [f"- code `{manifest['git']['sha'][:12]}` dirty files {manifest['git']['dirty_files']}, binary sha256 `{manifest['binary_sha256'][:16]}`, "
              f"runtime sha256 `{manifest['runtime_sha256'][:16]}`, document sha256 `{manifest['document_sha256'][:16]}`",
              f"- asset I/O fixed at `{manifest['io']}`, window frames {manifest['window']} (where the play starts and the frame it is stopped at; the stop is checked by polling, "
              f"so pictures a little past the end are included and a gap ending there counts), {len(report['iterations'])} fresh editor(s)", ""]
    lines += ["| iter | valid | class | gaps >62.5 | worst gap ms | pictures | starved | UI blocks ≥50 (n/max ms) | late transit ≥30 | page loop stalls (n/max ms) | ipc wait p95/max ms | eval p95/max ms |",
              "|---|---|---|---|---|---|---|---|---|---|---|---|"]
    for it in report["iterations"]:
        for p in it["plays"]:
            lines.append(f"| {it['iteration']} | {'yes' if it['valid'] else 'NO: ' + '; '.join(it['invalid_reasons'])} | {it['quiet_class']} | {p['slow_gaps']} | "
                         f"{p['gap_ms']['max']} | {p['pictures']} | {'YES' if p['starved'] else 'no'} | "
                         f"{len(p['ui_blocks'])}/{max((b['total_ms'] for b in p['ui_blocks']), default=None)} | {p['summary']['late_transit_ge_30']} | {p['page_loop_stalls']['n']}/{p['page_loop_stalls']['max_ms']} | "
                         f"{p['ipc_wait_ms']['p95']}/{p['ipc_wait_ms']['max']} | {p['eval_ms']['p95']}/{p['eval_ms']['max']} |")
    lines += ["", "## Slow gaps: which part held the picture that ended each", "",
              "| iter | frames | gap ms | total ms | pre-send | page queue | page draw | ipc wait | delivery (both ways) | eval ms | loop max ms | ahead | dominant |",
              "|---|---|---|---|---|---|---|---|---|---|---|---|---|"]
    for it in report["iterations"]:
        for p in it["plays"]:
            for g in p["slow_detail"]:
                d = g["picture"] or {}
                parts = d.get("parts") or {}
                lines.append(f"| {it['iteration']} | {g['from_frame']}→{g['to_frame']} | {g['gap_ms']} | {d.get('total_ms')} | {parts.get('host_pre_send')} | {parts.get('page_queue')} | "
                             f"{parts.get('page_draw')} | {parts.get('ipc_wait')} | {parts.get('delivery')} | {d.get('eval_ms')} | {d.get('page_loop_max_ms')} | "
                             f"{d.get('page_ahead')} | {d.get('dominant')}{':' + d['detail'] if d.get('detail') else ''} |")
    lines += ["", "## UI blocks: prefetch steps that held the window's thread", "",
              "| iter | began ms | total ms | prefetch ms | plans ahead ms | warm ms | loads | per load: kind bytes fs/send ms |", "|---|---|---|---|---|---|---|---|"]
    for it in report["iterations"]:
        for p in it["plays"]:
            for b in p["ui_blocks"]:
                per = "; ".join(f"{a['kind']} {a['bytes']} B {a['fs_ms']}/{a['send_ms']}" for a in b["assets"]) or "none recorded"
                lines.append(f"| {it['iteration']} | {b['at_ms']} | {b['total_ms']} | {b['prefetch_ms']} | {b['plans_ahead_ms']} | {b['warm_ms']} | {b['loads_started']} | {per} |")
    lines += ["", "## Counts by dominant part", ""]
    total = {}
    for it in report["iterations"]:
        for p in it["plays"]:
            for key, n in p["summary"]["slow_gap_by_part"].items():
                total[key] = total.get(key, 0) + n
    lines += [f"- slow-gap pictures: {json.dumps(total, sort_keys=True)}"]
    return "\n".join(lines) + "\n"


def analyze(records, manifest):
    iterations = [analyze_iteration(r) for r in records]
    red = [it for it in iterations if any(p["slow_gaps"] or p["ui_blocks"] or p["starved"] for p in it["plays"])]
    return {"manifest": manifest, "iterations": iterations, "red_iterations": [it["iteration"] for it in red],
            "all_valid": all(it["valid"] for it in iterations)}


# --------------------------------------------------------------------------------------------------- commands
def default_window(project):
    cut = project["cuts"][0] if project["cuts"] else 0
    fps = int(round(project["fps"]))
    return max(0, cut - 3 * fps), min(project["length"] - 1, cut + fps)


def run_iteration(W, binary, document, project, io, window, number, arm_env=None):
    work = tempfile.mkdtemp(prefix="tp-", dir="/tmp")
    record = {"iteration": number, "io": io, "window": list(window), "started_unix": time.time(),
              "env": {"TESSEL_GL": "gpu", "TESSEL_VIEWPORT_TRACE": "1", "TESSEL_VIEWPORT_ASSET_IO": io, **(arm_env or {})}}
    state = {"cursor": 0, "events": [], "frames": {}, "lost": 0, "errors": [], "loadavg": [], "status": {}}
    baseline = RL.webkit_pids()
    lab = sampler = None
    try:
        if W.free_gib() < RL.MIN_FREE_GIB_FLOOR:
            record["error"] = f"free disk {W.free_gib():.1f} GiB is below the floor"
            return record
        env = dict(record["env"], TESSEL_RUNTIME_DIR=str(ROOT / "runtime"))
        lab = W.Lab(str(binary), work, env)
        lab.wait_ready()
        lab.invoke("window.activate")
        time.sleep(1.0)
        copy = RL.new_workspace_copy(document, os.path.join(work, "copy"))
        opened = lab.op({"op": "project.open", "path": copy})
        if not opened.get("ok"):
            record["error"] = f"project.open: {opened}"
            return record
        sampler = RL.Sampler(lab.process.pid, baseline).start()
        time.sleep(2.0)
        RL.collect(lab, state, 0)
        start, end = window
        started = W.retry_while_preparing(lambda: lab.op({"op": "playback.start", "from": start}))
        if not started.get("ok"):
            state["errors"].append(f"playback.start: {started}")
        else:
            deadline = time.time() + (end - start) / project["fps"] + 40
            while time.time() < deadline:
                time.sleep(0.5)
                RL.collect(lab, state, deadline)
                status = lab.op({"op": "playback.status"}).get("data") or {}
                if status.get("state") != "playing" or (status.get("frame") or 0) >= end:
                    break
            lab.op({"op": "playback.stop"})
            time.sleep(1.0)
            RL.collect(lab, state, deadline)
        record["plays_ok"] = not state["errors"] and started.get("ok", False)
    except Exception as error:  # a loop that dies is reported, not hidden
        record["error"] = repr(error)
        record["plays_ok"] = False
    finally:
        if sampler:
            sampler.stop()
            record["external_cores"] = sampler.external
        if lab:
            try:
                RL.collect(lab, state, 0)
            except Exception:
                pass
            record["editor_exit"] = RL.editor_exit(lab)
            lab.stop()
        shutil.rmtree(work, ignore_errors=True)
    record["events"] = RL.flatten_events(state["events"])
    record["events_lost"] = state["lost"]
    record["frames"] = sorted(state["frames"].values(), key=lambda f: f["id"])
    record["status"] = state["status"]
    record["errors"] = state["errors"]
    record["loadavg"] = state["loadavg"]
    return record


def cmd_loop(args):
    W = load_module("scene_warm_proof", ROOT / "scripts" / "scene-warm-proof.py")
    binary = pathlib.Path(args.binary).resolve()
    document = pathlib.Path(args.project).resolve()
    for needed in (binary, ROOT / "runtime" / "dist" / "runtime.js", document):
        if not needed.exists():
            print(f"BLOCKED: missing {needed}")
            return 2
    project = RL.parse_project(json.loads(document.read_text()))
    missing = [s["label"] for s in RL.check_sources(project["sources"]) if not s["present"]]
    if missing:
        print(f"BLOCKED: {len(missing)} source(s) missing")
        return 2
    if W.free_gib() < RL.MIN_FREE_GIB_START:
        print(f"BLOCKED: {W.free_gib():.1f} GiB free")
        return 2
    window = (args.start, args.end) if args.start is not None and args.end is not None else default_window(project)
    locks = W.acquire_locks(args.lock_wait)
    if locks is None:
        print(f"BLOCKED: the GPU bench locks were not free within {args.lock_wait:.0f} s")
        return 2
    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    manifest = {"git": RL.git_state(), "binary_sha256": RL.sha256_file(binary), "runtime_sha256": RL.sha256_file(ROOT / "runtime" / "dist" / "runtime.js"),
                "document_sha256": RL.sha256_file(document), "io": args.io, "window": list(window), "iterations": args.iterations,
                "label": args.label, "started_unix": time.time()}
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2))
    print(f"build {manifest['git']['sha'][:9]} dirty={manifest['git']['dirty_files']} binary {manifest['binary_sha256'][:12]} window {window}", flush=True)
    records = []
    try:
        for number in range(args.iterations):
            record = run_iteration(W, binary, document, project, args.io, window, number)
            (out / f"iteration-{number:02d}.json").write_text(json.dumps(record))
            records.append(record)
            valid, reasons = validity(record)
            print(f"iteration {number} valid={valid} {reasons}", flush=True)
    finally:
        for handle in locks:
            handle.close()
    return finish(out, records, manifest, args.report_dir)


def finish(out, records, manifest, report_dir):
    report = analyze(records, manifest)
    (out / "analysis.json").write_text(json.dumps(report, indent=2))
    (out / "analysis.md").write_text(markdown(report))
    if report_dir:
        target = pathlib.Path(report_dir)
        target.mkdir(parents=True, exist_ok=True)
        (target / "analysis.json").write_text(json.dumps(report, indent=2))
        (target / "analysis.md").write_text(markdown(report))
    print(markdown(report))
    if report["red_iterations"]:
        print(f"RED: gaps above {SLOW_MS} ms, a UI block of {UI_BLOCK_MS} ms or more, or a starved play in iteration(s) {report['red_iterations']}")
        return 1
    return 0 if report["all_valid"] else 1


# ------------------------------------------------------------------------------------------------------ A/B
def arm_summary(record):
    """What one run says, per run (never averaged away): gaps, slow gaps, UI blocks, the page's big stalls and, for the GPU-finish
    arm, how long the page waited for the GPU after each draw."""
    it = analyze_iteration(record)
    play = it["plays"][0] if it["plays"] else {}
    events = record.get("events", [])
    stalls = [e["fields"]["ms"] for e in events if e["kind"] == "runtime_span" and e["fields"].get("k") == "loop_stall" and e["fields"]["ms"] >= 40]
    finish = [(e["fields"]["id"], e["fields"]["gpu_finish_ms"]) for e in events if e["kind"] == "frame_page" and e["fields"].get("gpu_finish_ms") is not None]
    gaps = gaps_with_ids(RL.play_pictures(record.get("frames", []), RL.play_segments(events)[0])) if RL.play_segments(events) else []
    reasons = list(it["invalid_reasons"])
    if play.get("starved"):
        reasons.append("the page stopped answering: the play shows almost no pictures")
    return {"valid": it["valid"] and not play.get("starved"), "invalid_reasons": reasons, "class": it["quiet_class"], "pictures": play.get("pictures"),
            "gap_ms": play.get("gap_ms"), "slow_gaps": play.get("slow_gaps"), "ui_blocks": len(play.get("ui_blocks", [])),
            "page_stalls_ge_40": {"n": len(stalls), "max_ms": max(stalls, default=None)},
            "gpu_finish": {"n": len(finish), "ge_20": sorted([(i, round(ms, 1)) for i, ms in finish if ms >= 20], key=lambda x: -x[1])[:6],
                           "max_ms": round(max((ms for _, ms in finish), default=0), 1) if finish else None},
            "gaps_over": [(g["from_frame"], g["to_frame"], g["gap_ms"]) for g in gaps if g["gap_ms"] > SLOW_MS]}


def ab_markdown(report):
    m = report["manifest"]
    lines = ["# Transit probe A/B (BL157.2): is the stall after a draw the GPU still working?", "",
             f"- code `{m['git']['sha'][:12]}` dirty files {m['git']['dirty_files']}, binary sha256 `{m['binary_sha256'][:16]}`, runtime sha256 `{m['runtime_sha256'][:16]}`, document sha256 `{m['document_sha256'][:16]}`",
             f"- asset I/O fixed at `{m['io']}`, window {m['window']} (start and target stop; polling can overshoot, and pictures past the end are included), arm A `{m['arm_a']}` / arm B `{m['arm_b']}`, {m['pairs']} pairs, order alternates (pair 0 A first, pair 1 B first, ...)",
             "- every run is a fresh editor: all are FIRST plays of a cold process; a repeated play in one process is not measured here", "",
             "| pair | order | arm | valid | class | pictures | gap p50 | gap p95 | gap max | slow gaps (from→to ms) | UI blocks | page stalls ≥40 (n/max ms) | gpu finish max ms (frames ≥20 ms) |",
             "|---|---|---|---|---|---|---|---|---|---|---|---|---|"]
    for run in report["runs"]:
        s = run["summary"]
        g = s["gap_ms"] or {}
        lines.append(f"| {run['pair']} | {run['order']} | {run['arm']} | {'yes' if s['valid'] else 'NO: ' + '; '.join(s['invalid_reasons'])} | {s['class']} | {s['pictures']} | {g.get('p50')} | {g.get('p95')} | "
                     f"{g.get('max')} | {s['slow_gaps']} ({'; '.join(f'{a}→{b} {c}' for a, b, c in s['gaps_over']) or '-'}) | {s['ui_blocks']} | "
                     f"{s['page_stalls_ge_40']['n']}/{s['page_stalls_ge_40']['max_ms']} | {s['gpu_finish']['max_ms']} ({s['gpu_finish']['ge_20']}) |")
    lines += ["", "## Per arm (runs listed, not averaged)", ""]
    for arm in ("A", "B"):
        runs = [r["summary"] for r in report["runs"] if r["arm"] == arm and r["summary"]["valid"]]
        lines.append(f"- arm {arm}: {len(runs)} valid run(s); worst gap per run {[ (r['gap_ms'] or {}).get('max') for r in runs]}; "
                     f"page stalls ≥40 ms per run {[r['page_stalls_ge_40']['n'] for r in runs]} (max {[r['page_stalls_ge_40']['max_ms'] for r in runs]})")
    lines += ["", f"Verdict: {report['verdict']}", ""]
    return "\n".join(lines)


def ab_verdict(runs):
    """Mechanical and conservative; it never says SUPPORTED/REFUTED, because arm B's own wait is a page stall too (so the stall
    counts of the two arms are not comparable) and the arm is a probe, not a fix. B waits for the GPU inside the frame. What it
    can say: the GPU still had at least 40 ms of work after a draw (measured on the page clock by a readback), in a window
    where arm A shows page stalls of that size, and the delay moved into the frame instead of going away."""
    valid = {arm: [r["summary"] for r in runs if r["arm"] == arm and r["summary"]["valid"]] for arm in ("A", "B")}
    if min(len(valid["A"]), len(valid["B"])) < 4:
        return "INCONCLUSIVE: fewer than 4 valid runs in an arm."
    b_wait = [r["gpu_finish"]["max_ms"] or 0 for r in valid["B"]]
    if sum(1 for ms in b_wait if ms >= 5) < len(b_wait) / 2:
        return f"INCONCLUSIVE: the manipulation did not work, the page's wait for the GPU was under 5 ms in most B runs (max per B run {b_wait})."
    a_stalls = [r["page_stalls_ge_40"]["n"] for r in valid["A"]]
    if sum(1 for n in a_stalls if n > 0) < len(a_stalls) / 2:
        return f"INCONCLUSIVE: arm A did not reproduce a page stall of 40 ms or more in most runs (per run {a_stalls})."
    if sum(1 for ms in b_wait if ms >= 40) < len(b_wait) / 2:
        return f"INCONCLUSIVE: the page's wait for the GPU reached 40 ms in fewer than half of the B runs (max per B run {b_wait}); stalls in A per run {a_stalls}."
    worst = {arm: statistics.median([(r["gap_ms"] or {}).get("max") or 0 for r in valid[arm]]) for arm in ("A", "B")}
    where = "moved into the frame, not removed" if worst["B"] >= 0.75 * worst["A"] else "shorter in B"
    return (f"CONSISTENT with GPU work still pending after a draw: B's readback waited at least 40 ms in {sum(1 for ms in b_wait if ms >= 40)} of {len(b_wait)} runs "
            f"(max per run {b_wait}) where A shows page stalls of 40 ms or more (per run {a_stalls}); median worst gap A {worst['A']} ms, B {worst['B']} ms: the delay was {where}. "
            "Correlation under a causal arm; the arm adds its own wait, so it does not show this is the whole stall, and it is not a display or photon latency.")


def cmd_ab(args):
    W = load_module("scene_warm_proof", ROOT / "scripts" / "scene-warm-proof.py")
    binary = pathlib.Path(args.binary).resolve()
    document = pathlib.Path(args.project).resolve()
    for needed in (binary, ROOT / "runtime" / "dist" / "runtime.js", document):
        if not needed.exists():
            print(f"BLOCKED: missing {needed}")
            return 2
    project = RL.parse_project(json.loads(document.read_text()))
    if [s for s in RL.check_sources(project["sources"]) if not s["present"]]:
        print("BLOCKED: project source(s) missing")
        return 2
    if W.free_gib() < RL.MIN_FREE_GIB_START:
        print(f"BLOCKED: {W.free_gib():.1f} GiB free")
        return 2
    window = (args.start, args.end) if args.start is not None and args.end is not None else default_window(project)
    locks = W.acquire_locks(args.lock_wait)
    if locks is None:
        print(f"BLOCKED: the GPU bench locks were not free within {args.lock_wait:.0f} s")
        return 2
    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    arms = {"A": {}, "B": {"TESSEL_VIEWPORT_PROBE_GPU_FINISH": "1"}}
    manifest = {"git": RL.git_state(), "binary_sha256": RL.sha256_file(binary), "runtime_sha256": RL.sha256_file(ROOT / "runtime" / "dist" / "runtime.js"),
                "document_sha256": RL.sha256_file(document), "io": args.io, "window": list(window), "pairs": args.pairs,
                "arm_a": "trace + page timing (the probe's own cost in both arms)", "arm_b": "arm A + the page waits for the GPU after each draw (TESSEL_VIEWPORT_PROBE_GPU_FINISH=1)",
                "label": args.label, "started_unix": time.time()}
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2))
    runs = []
    try:
        for pair in range(args.pairs):
            order = "AB" if pair % 2 == 0 else "BA"
            for arm in order:
                record = run_iteration(W, binary, document, project, args.io, window, pair, arm_env=arms[arm])
                record["arm"] = arm
                (out / f"pair-{pair}-{arm}.json").write_text(json.dumps(record))
                summary = arm_summary(record)
                runs.append({"pair": pair, "order": order, "arm": arm, "summary": summary})
                print(f"pair {pair} arm {arm} valid={summary['valid']} slow_gaps={summary['slow_gaps']} stalls>=40={summary['page_stalls_ge_40']} finish_max={summary['gpu_finish']['max_ms']}", flush=True)
    finally:
        for handle in locks:
            handle.close()
    report = {"manifest": manifest, "runs": runs, "verdict": ab_verdict(runs)}
    (out / "ab.json").write_text(json.dumps(report, indent=2))
    (out / "ab.md").write_text(ab_markdown(report))
    print(ab_markdown(report))
    return 0 if all(r["summary"]["valid"] for r in runs) else 1


def cmd_ab_report(args):
    """Rebuilds `ab.md`/`ab.json` with the CURRENT rules (validity, starved plays, verdict): from the raw `pair-*.json` runs when they
    are there, otherwise from the saved summaries (then only the verdict is recomputed, and the report says so)."""
    out = pathlib.Path(args.out)
    report = json.loads((out / "ab.json").read_text())
    raw = sorted(out.glob("pair-*-*.json"))
    if raw:
        runs = []
        for path in raw:
            record = json.loads(path.read_text())
            _, pair, arm = path.stem.split("-")
            runs.append({"pair": int(pair), "order": "AB" if int(pair) % 2 == 0 else "BA", "arm": arm, "summary": arm_summary(record)})
        # Run order as executed: pair by pair, in that pair's order.
        runs.sort(key=lambda r: (r["pair"], r["order"].index(r["arm"])))
        report["runs"] = runs
    else:
        report["manifest"]["note"] = "raw runs not available: summaries as saved, verdict recomputed"
    report["verdict"] = ab_verdict(report["runs"])
    (out / "ab.json").write_text(json.dumps(report, indent=2))
    (out / "ab.md").write_text(ab_markdown(report))
    print(ab_markdown(report))
    return 0


def cmd_analyze(args):
    out = pathlib.Path(args.out)
    manifest = json.loads((out / "manifest.json").read_text())
    records = [json.loads(p.read_text()) for p in sorted(out.glob("iteration-*.json"))]
    return finish(out, records, manifest, args.report_dir)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    loop = sub.add_parser("loop")
    loop.add_argument("--binary", default=str(ROOT / "target" / "release" / "tessel-editor"))
    loop.add_argument("--project", required=True)
    loop.add_argument("--out", required=True)
    loop.add_argument("--report-dir")
    loop.add_argument("--io", choices=("sync", "async"), default="async")
    loop.add_argument("--iterations", type=int, default=3)
    loop.add_argument("--from", dest="start", type=int)
    loop.add_argument("--to", dest="end", type=int)
    loop.add_argument("--lock-wait", type=float, default=120.0)
    loop.add_argument("--label", default="")
    ab = sub.add_parser("ab")
    ab.add_argument("--binary", default=str(ROOT / "target" / "release" / "tessel-editor"))
    ab.add_argument("--project", required=True)
    ab.add_argument("--out", required=True)
    ab.add_argument("--io", choices=("sync", "async"), default="async")
    ab.add_argument("--pairs", type=int, default=4)
    ab.add_argument("--from", dest="start", type=int)
    ab.add_argument("--to", dest="end", type=int)
    ab.add_argument("--lock-wait", type=float, default=120.0)
    ab.add_argument("--label", default="")
    rep = sub.add_parser("ab-report")
    rep.add_argument("--out", required=True)
    ana = sub.add_parser("analyze")
    ana.add_argument("--out", required=True)
    ana.add_argument("--report-dir")
    args = parser.parse_args()
    if args.command in ("loop", "ab") and (args.start is None) != (args.end is None):
        parser.error("--from and --to go together: give both or neither (neither measures the default window around the first cut)")
    return {"loop": cmd_loop, "ab": cmd_ab, "ab-report": cmd_ab_report, "analyze": cmd_analyze}[args.command](args)


if __name__ == "__main__":
    raise SystemExit(main())
