#!/usr/bin/env python3
"""BL157.3: does paying a first-use cost AHEAD of the cut shorten what the cut costs? One factor at a time, same binary, same runtime.

    python3 scripts/first-use-ab.py narrow --factor preupload|warm_draw --binary B --project P.tessel.json --out DIR [--from 114 --to 210]
    python3 scripts/first-use-ab.py full   --factor preupload|warm_draw --binary B --project P.tessel.json --out DIR [--pairs 4]
    python3 scripts/first-use-ab.py analyze --factor ... --mode narrow|full --project P.tessel.json --out DIR [--report-dir DIR2]

The two factors are SEPARATE experiments (the arms of one run never switch both on):

    preupload   TESSEL_VIEWPORT_PREUPLOAD=1   the first texture of each still image is uploaded to the GPU between frames, ahead of its clip
    warm_draw   TESSEL_VIEWPORT_WARM_DRAW=1   a splat scene the warm sorted is drawn once into a 16 px scratch target, ahead of the cut

Every session is a FRESH editor on the SAME binary and runtime, with the asset I/O mode fixed at `async`, the scene warm on (both arms), the
default resolution, MSAA, audio and look-ahead, retention on and `gpu_finish` off. The control arm sets the factor's switch to `0`
explicitly (and the other factor's too), so nothing inherited from the shell can turn one on. `narrow` plays one window of the project once
per arm (the default [114, 210] holds the cut to scene 2 at 186/188 and the picture clip at 196: it separates the picture's own cost from
the cut's); `full` plays the whole project twice per session (a FIRST play of a cold editor and a REPEAT in the same process, never pooled)
over 4 pairs in the order control/on, on/control, control/on, on/control.

DECISION RULES, fixed here before any measurement (diagnostic references, not acceptance limits; 41.7 / 60 / 62.5 ms are references):

    comparable pair   both sessions valid, both `quiet` (median external CPU <= 3 cores, p95 <= 6) and the two medians within 2 cores.
                      Everything else is published in the `loaded` group, never dropped.
    local benefit     for the factor's own target (the picture clip's entry for `preupload`, the cut's entry for `warm_draw`), the arm's
                      entry gap is smaller than the control's by at least 10 ms in at least 3 of the 4 pairs' FIRST plays, and the pre-cut
                      windows' total excess (ms above the frame budget in the 2 s before each cut) is not larger by more than 20 ms.
    shifted           the entry gap is smaller but the pre-cut excess grew by more than 20 ms: reported as "moved before the cut", not a gain.
    closed            a cost is closed only if the arm's gap at the entry is under 62.5 ms in every valid first play. One entry (the picture's) says
                      nothing about the larger 188 -> 189 pause: it is judged on its own row.
    never claimed     acceptance of BL157. No default is changed here.

Exit: 0 all sessions valid; 1 a session invalid (still published); 2 a prerequisite is missing (BLOCKED).
"""
import argparse
import importlib.util
import json
import os
import pathlib
import shutil
import statistics
import sys
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent
BUDGET_DELTA_MS = 10.0
PRE_CUT_EXCESS_MS = 20.0
# `span` is the page's `runtime_spans` kind for the factor and `worked` the ONE outcome that means the manipulation happened (every other outcome
# says why it did not). The list of outcomes the page can report is `runtime/test/fixtures/activation-spans.json`: the runtime suite makes the real
# page report each, the host suite sends each to the trace, and `first-use-ab.spec.py` checks these two fields against it.
FACTORS = {
    "preupload": {"env": "TESSEL_VIEWPORT_PREUPLOAD", "other": "TESSEL_VIEWPORT_WARM_DRAW", "report": "preupload", "feature": "texture.preupload",
                  "span": "preupload", "worked": "uploaded", "target": "the picture clip's entry"},
    "warm_draw": {"env": "TESSEL_VIEWPORT_WARM_DRAW", "other": "TESSEL_VIEWPORT_PREUPLOAD", "report": "warm_draw", "feature": "scene.warm_draw",
                  "span": "warm_draw", "worked": "drawn", "target": "the cut's entry"},
}
ARMS = ("control", "on")
# A play that shows fewer than this share of the project's frames, or waits this long between two pictures, is a HANG: reported in its own column
# and counted per arm. It does not make the run invalid (it is what the editor did), and it is never dropped.
HANG_PICTURES_SHARE = 0.9
HANG_GAP_MS = 2000.0


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


RL = load_module("real_latency_ab", ROOT / "scripts" / "real-latency-ab.py")
TP = load_module("viewport_transit_probe", ROOT / "scripts" / "viewport-transit-probe.py")


# ------------------------------------------------------------------------------------------------ pure logic
def arm_env(factor, arm):
    """The environment that makes an arm: the factor's switch explicitly 0 or 1, the other factor's explicitly 0, the probe's GPU wait off."""
    spec = FACTORS[factor]
    return {spec["env"]: "1" if arm == "on" else "0", spec["other"]: "0", "TESSEL_VIEWPORT_PROBE_GPU_FINISH": "0"}


def arm_order(pairs):
    """control/on, on/control, control/on, on/control, ..."""
    order = []
    for pair in range(pairs):
        order += ["control", "on"] if pair % 2 == 0 else ["on", "control"]
    return order


def factor_validity(record, factor):
    """The factor's own claims (beyond the harness's): the host reports the arm it was asked for, and `on` really did the work."""
    spec = FACTORS[factor]
    arm = record.get("arm")
    reasons = []
    status = record.get("status") or {}
    report = status.get(spec["report"]) or {}
    other = status.get(FACTORS[[f for f in FACTORS if f != factor][0]]["report"]) or {}
    if report.get("enabled") is not (arm == "on"):
        reasons.append(f"the host reports {spec['report']}.enabled={report.get('enabled')!r} for the {arm} arm")
    if other.get("enabled"):
        reasons.append("the other factor is on: the arms of one experiment switch one factor only")
    spans = [e for e in record.get("events", []) if e["kind"] == "runtime_span" and e["fields"].get("k") == spec["span"]]
    if arm == "on":
        if report.get("supported") is not True:
            reasons.append(f"the runtime did not announce `{spec['feature']}`")
        if factor == "preupload" and not (report.get("sent") or 0) > 0:
            reasons.append("the host sent no pre-upload (nothing was manipulated)")
        if not any(s["fields"].get("outcome") == spec["worked"] for s in spans):
            reasons.append(f"the page reports no `{spec['span']}` that worked (the manipulation did not happen)")
    elif spans:
        reasons.append(f"the control arm shows `{spec['span']}` spans")
    return reasons


def span_summary(events, kind):
    spans = [e["fields"] for e in events if e["kind"] == "runtime_span" and e["fields"].get("k") == kind]
    outcomes = {}
    for span in spans:
        outcomes[span.get("outcome")] = outcomes.get(span.get("outcome"), 0) + 1
    worked_outcome = next((spec["worked"] for spec in FACTORS.values() if spec["span"] == kind), None)
    worked = [s["ms"] for s in spans if s.get("outcome") == worked_outcome]
    return {"spans": len(spans), "outcomes": outcomes, "ms": RL.stats(worked), "total_ms": round(sum(worked), 1)}


def gl_calls(events):
    """The page's slow GL calls (`texSubImage2D`, ...) the glProbe reports, by name: count and worst ms."""
    out = {}
    for e in events:
        if e["kind"] == "runtime_span" and e["fields"].get("k") == "gl_call":
            name = e["fields"].get("name") or e["fields"].get("call") or "?"
            item = out.setdefault(name, {"n": 0, "max_ms": 0.0})
            item["n"] += 1
            item["max_ms"] = max(item["max_ms"], e["fields"].get("ms") or 0.0)
    return out


def entries(frames, events, project, segment, starts, width):
    """For each entry (a clip start or a cut): the LONGEST gap that ends on one of its first `width`+1 frames, with the picture that ended it and
    where that picture's time went. The long wait of a cut can land a frame or two after the cut (`187 -> 188` for the cut at 186), so the
    first picture at the entry is not the one to read."""
    shots = RL.play_pictures(frames, segment)
    decomposed = {d["id"]: d for d in TP.decompose_play(frames, events, segment)}
    out = []
    for start in starts:
        best = None
        for index, shot in enumerate(shots):
            if not (index and start <= shot["shown_frame"] <= start + width and shots[index - 1]["shown_frame"] != shot["shown_frame"]):
                continue
            gap = round(shot["done_ms"] - shots[index - 1]["done_ms"], 1)
            if best is None or gap > best[0]:
                best = (gap, shot, shots[index - 1])
        if best is None:
            out.append({"entry": start, "missing": True})
            continue
        gap, shot, before = best
        parts = (decomposed.get(shot["id"]) or {}).get("parts") or {}
        out.append({"entry": start, "from_frame": before["shown_frame"], "shown_frame": shot["shown_frame"], "gap_ms": gap,
                    "request_to_accept_ms": shot["submit_to_done_ms"], "runtime_total_ms": shot["runtime_total_ms"],
                    "page_queue_ms": parts.get("page_queue"), "page_draw_ms": parts.get("page_draw"),
                    "dominant": (decomposed.get(shot["id"]) or {}).get("dominant")})
    return out


def analyze_run(record, project, factor, window=None):
    """One run (a full session or a window play) into the per-play facts the report prints. Never a judgement."""
    spec = FACTORS[factor]
    events = record.get("events", [])
    frames = record.get("frames", [])
    if window:
        # The window's play is stopped by polling and the refine of the paused playhead follows; the stop frame and anything after it are not part of it.
        frames = [f for f in frames if f.get("shown_frame") is None or f["shown_frame"] < window[1]]
    segments = RL.play_segments(events)
    names = ["first"] if window else ["first", "repeat"]
    budget = 1000.0 / project["fps"]
    in_window = (lambda f: window[0] - 2 <= f <= window[1] + 2) if window else (lambda f: True)
    plays = {}
    for name, segment in zip(names, segments):
        summary = RL.summarize_play(frames, events, segment, project, budget)
        windows = RL.window_report(frames, project, segment)
        mine = [f for f in frames if segment["clock_ms"] and segment["clock_ms"] <= f["done_ms"] < segment["end_ms"]]
        plays[name] = {
            "summary": {k: summary[k] for k in ("pictures", "interval_ms", "slow_gt_62_5", "slow_gt_100", "over_budget_gt_41_7", "request_to_accept_ms",
                                                  "transit_ms", "plans_ahead_ms", "prefetch_total_ms", "sync_ms", "ui_callback_total_ms",
                                                  "reads_on_ui_thread", "by_location", "play_command_to_first_picture_ms",
                                                  "play_command_to_clock_ms")},
            "slow_detail": [{"from_frame": g["from_frame"], "to_frame": g["to_frame"], "gap_ms": g["gap_ms"], "where": g["where"],
                             "transit_ms": g["picture"]["transit_ms"], "runtime_total_ms": g["picture"]["runtime_total_ms"],
                             "ui_callback_ms": g["context"]["ui_callback_ms"], "page_work_ms": round(
                                 (g["context"].get("page_decode_ms") or 0) + (g["context"].get("page_fetch_ms") or 0) + (g["context"].get("page_warm_ms") or 0), 1),
                             "reads": g["context"]["reads"]} for g in summary["slow_detail"] if in_window(g["to_frame"])],
            "png_entries": [e for e in entries(frames, events, project, segment, project["video_starts"], RL.VIDEO_ENTRY_WINDOW) if not window or window[0] <= e["entry"] <= window[1]],
            "cut_entries": [e for e in entries(frames, events, project, segment, project["cuts"], RL.ENTRY_WINDOW) if not window or window[0] <= e["entry"] <= window[1]],
            "pre_cut_excess_ms": round(sum(w["prefetch_window"]["excess_ms"] for w in windows if not window or window[0] <= w["cut"] <= window[1] + RL.ENTRY_WINDOW), 1),
            "pre_cut_slow": sum(w["prefetch_window"]["slow"] for w in windows if not window or window[0] <= w["cut"] <= window[1] + RL.ENTRY_WINDOW),
            "windows": windows,
            "frames_in_play": len(mine),
            "hang": bool(window is None and (summary["pictures"] < HANG_PICTURES_SHARE * project["length"] or (summary["interval_ms"]["max"] or 0) > HANG_GAP_MS)),
        }
    valid, reasons = RL.session_validity(record | {"mode": "async"}) if not window else TP.validity(record)
    reasons = list(reasons) + factor_validity(record, factor)
    warmed = ((record.get("status") or {}).get("stats") or {}).get("warmed")
    if window and not (isinstance(warmed, int) and warmed > 0):
        reasons.append("the scene warm never ran (it must be on in both arms)")
    external = record.get("external_cores") or []
    return {
        "arm": record.get("arm"), "pair": record.get("pair"), "order_index": record.get("order_index"),
        "valid": not reasons, "invalid_reasons": reasons,
        "quiet_class": RL.quiet_class(external), "external_median_cores": round(statistics.median(external), 2) if external else None,
        "external_p95_cores": RL.percentile(external, 0.95),
        "loadavg_start": (record.get("host") or {}).get("loadavg_start"),
        "loadavg_peak_1m": max((row[0] for row in record.get("loadavg", [])), default=None),
        "memory": RL.memory_summary(record.get("memory_series", [])),
        "factor_report": (record.get("status") or {}).get(spec["report"]),
        "factor_spans": span_summary(events, spec["span"]),
        "gl_calls": gl_calls(events),
        "asset_io": (record.get("status") or {}).get("asset_io"),
        "warmed": ((record.get("status") or {}).get("stats") or {}).get("warmed"),
        "plays": plays,
        "errors": [RL.redact(e) for e in record.get("errors", [])],
    }


def delta(on, control, getter):
    a, b = getter(control), getter(on)
    return None if a is None or b is None else round(b - a, 3)


def target_gap(play, factor, entry):
    """The gap at one entry of a play (the picture clip's at 196, the cut's at 186/188), or None when it was not shown."""
    pool = play["png_entries"] if factor == "preupload" else play["cut_entries"]
    item = next((e for e in pool if e["entry"] == entry and not e.get("missing")), None)
    return item["gap_ms"] if item else None


def pair_rows(runs, factor, entry):
    by_pair = {}
    for run in runs:
        by_pair.setdefault(run["pair"], {})[run["arm"]] = run
    rows = []
    for pair, sides in sorted(by_pair.items()):
        if set(sides) != set(ARMS):
            continue
        control, on = sides["control"], sides["on"]
        comparable = control["valid"] and on["valid"] and control["quiet_class"] == "quiet" and on["quiet_class"] == "quiet" \
            and abs((control["external_median_cores"] or 0) - (on["external_median_cores"] or 0)) <= RL.PAIR_MAX_DELTA_CORES
        for play in sorted(set(control["plays"]) & set(on["plays"])):
            pc, po = control["plays"][play], on["plays"][play]
            rows.append({
                "pair": pair, "play": play, "comparable": comparable, "order": [control["order_index"], on["order_index"]],
                "entry_gap_ms": {"control": target_gap(pc, factor, entry), "on": target_gap(po, factor, entry)},
                "png_entry_gap_median_ms": {arm: _median_gap(p["png_entries"]) for arm, p in (("control", pc), ("on", po))},
                "cut_entry_gap_median_ms": {arm: _median_gap(p["cut_entries"]) for arm, p in (("control", pc), ("on", po))},
                "slow_gt_62_5": {"control": pc["summary"]["slow_gt_62_5"], "on": po["summary"]["slow_gt_62_5"]},
                "slow_gt_100": {"control": pc["summary"]["slow_gt_100"], "on": po["summary"]["slow_gt_100"]},
                "max_gap_ms": {"control": pc["summary"]["interval_ms"]["max"], "on": po["summary"]["interval_ms"]["max"]},
                "p95_gap_ms": {"control": pc["summary"]["interval_ms"]["p95"], "on": po["summary"]["interval_ms"]["p95"]},
                "pre_cut_excess_ms": {"control": pc["pre_cut_excess_ms"], "on": po["pre_cut_excess_ms"]},
                "play_to_first_picture_ms": {"control": pc["summary"]["play_command_to_first_picture_ms"], "on": po["summary"]["play_command_to_first_picture_ms"]},
                "memory_peak_mib": {"control": control["memory"]["rss_sum_mib"]["peak"], "on": on["memory"]["rss_sum_mib"]["peak"]},
                "memory_median_mib": {"control": control["memory"]["rss_sum_mib"]["median"], "on": on["memory"]["rss_sum_mib"]["median"]},
            })
    return rows


def _median_gap(items):
    values = [e["gap_ms"] for e in items if not e.get("missing")]
    return round(statistics.median(values), 1) if values else None


def verdict(rows, factor, entry):
    """Mechanical, with the rules of the docstring. It says what the numbers do; it never says accepted."""
    first = [r for r in rows if r["play"] == "first"]
    spec = FACTORS[factor]
    if len(first) < 4:
        return f"INCONCLUSIVE: {len(first)} first-play pair(s); the rule needs 4."
    usable = [r for r in first if r["entry_gap_ms"]["control"] is not None and r["entry_gap_ms"]["on"] is not None]
    if len(usable) < 4:
        return f"INCONCLUSIVE: {spec['target']} was shown in both arms in only {len(usable)} of the first plays."
    better = [r for r in usable if r["entry_gap_ms"]["control"] - r["entry_gap_ms"]["on"] >= BUDGET_DELTA_MS]
    grew = [r for r in usable if r["pre_cut_excess_ms"]["on"] - r["pre_cut_excess_ms"]["control"] > PRE_CUT_EXCESS_MS]
    comparable = sum(1 for r in usable if r["comparable"])
    parts = [f"entry {entry} ({spec['target']}): the arm's gap was at least {BUDGET_DELTA_MS:.0f} ms under the control's in {len(better)} of {len(usable)} first plays "
             f"(per pair control/on: {[(r['entry_gap_ms']['control'], r['entry_gap_ms']['on']) for r in usable]}); pre-cut excess grew by more than "
             f"{PRE_CUT_EXCESS_MS:.0f} ms in {len(grew)} (per pair control/on: {[(r['pre_cut_excess_ms']['control'], r['pre_cut_excess_ms']['on']) for r in usable]}); "
             f"{comparable} of {len(usable)} pairs comparable (quiet)."]
    if len(better) >= 3 and len(grew) == 0:
        parts.append("LOCAL BENEFIT at this entry by the pre-fixed rule (this is not the larger pause and not an acceptance)")
    elif len(better) >= 3:
        parts.append("SHIFTED: the entry is shorter but time was added before the cut")
    else:
        parts.append("NO LOCAL BENEFIT by the pre-fixed rule")
    closed = all((r["entry_gap_ms"]["on"] or 0) < 62.5 for r in usable)
    parts.append("the entry is under 62.5 ms in every first play" if closed else "the entry is still over 62.5 ms in some first play: not closed")
    return " ".join(parts)


# ---------------------------------------------------------------------------------------------------- report
def markdown(report):
    m = report["manifest"]
    spec = FACTORS[m["factor"]]
    lines = [f"# First-use A/B (BL157.3): `{m['factor']}` on and off, {m['mode']}", "",
             f"- code `{m['git']['sha'][:12]}` dirty files {m['git']['dirty_files']}; binary sha256 `{m['binary_sha256']}`; runtime sha256 `{m['runtime_sha256']}`; "
             f"document sha256 `{m['document_sha256']}`",
             f"- the SAME binary and runtime for both arms; asset I/O `async`; scene warm on; gpu_finish off; arms: control = `{spec['env']}=0`, on = `{spec['env']}=1`; "
             f"the other factor `{spec['other']}=0` in every arm",
             f"- window {m.get('window')}; order {m['order']}; every session a fresh editor"
             + ("; each full session plays the project twice (FIRST and REPEAT, never pooled)" if m["mode"] == "full" else ""), "",
             f"Pre-fixed rules: see the script. Entry judged: {report['entry']} ({spec['target']}). References 41.7 / 60 / 62.5 ms are diagnostic only.", "",
             "## Runs", "",
             "| # | pair | arm | valid | class | ext median/p95 cores | loadavg start / peak 1m | pictures | gaps >62.5 / >100 | max gap ms | hang | pre-cut excess ms | play→first picture ms | memory median/peak MiB (editor + helpers, RSS sum) |",
             "|---|---|---|---|---|---|---|---|---|---|---|---|---|---|"]
    for run in report["runs"]:
        for name, play in run["plays"].items():
            s = play["summary"]
            mem = run["memory"]["rss_sum_mib"]
            lines.append(f"| {run['order_index']} | {run['pair']} | {run['arm']} ({name}) | {'yes' if run['valid'] else 'NO: ' + '; '.join(run['invalid_reasons'])} | "
                         f"{run['quiet_class']} | {run['external_median_cores']}/{run['external_p95_cores']} | {run['loadavg_start']} / {run['loadavg_peak_1m']} | "
                         f"{s['pictures']} | {s['slow_gt_62_5']} / {s['slow_gt_100']} | {s['interval_ms']['max']} | {'HANG' if play['hang'] else 'no'} | {play['pre_cut_excess_ms']} | "
                         f"{s['play_command_to_first_picture_ms']} | {mem['median']} / {mem['peak']} |")
    lines += ["", "## Entries (the longest gap ending on the entry's first frames, ms, with the frames it spans; request→accept in parentheses; page draw after the slash)", ""]
    lines += ["| run | play | " + " | ".join(f"png {e}" for e in report["project"]["video_starts"] if _in(e, m)) + " | " +
              " | ".join(f"cut {e}" for e in report["project"]["cuts"] if _in(e, m)) + " |"]
    columns = 2 + len([e for e in report["project"]["video_starts"] if _in(e, m)]) + len([e for e in report["project"]["cuts"] if _in(e, m)])
    lines += ["|" + "---|" * columns]
    for run in report["runs"]:
        for name, play in run["plays"].items():
            def cell(item):
                if item.get("missing"):
                    return "missing"
                return f"{item['gap_ms']} {item['from_frame']}→{item['shown_frame']} ({item['request_to_accept_ms']}) / {item['page_draw_ms']}"
            png = [cell(e) for e in play["png_entries"]]
            cuts = [cell(e) for e in play["cut_entries"]]
            lines.append(f"| {run['order_index']} {run['arm']} | {name} | " + " | ".join(png + cuts) + " |")
    lines += ["", "## Every gap above 62.5 ms (where, what the host and page were doing)", "",
              "| run | play | frames | gap ms | where | transit ms | runtime ms | UI callback ms | page work ms | reads |", "|---|---|---|---|---|---|---|---|---|---|"]
    for run in report["runs"]:
        for name, play in run["plays"].items():
            for g in play["slow_detail"]:
                lines.append(f"| {run['order_index']} {run['arm']} | {name} | {g['from_frame']}→{g['to_frame']} | {g['gap_ms']} | {g['where']} | {g['transit_ms']} | "
                             f"{g['runtime_total_ms']} | {g['ui_callback_ms']} | {g['page_work_ms']} | {g['reads']} |")
    lines += ["", "## Prefetch, locations and the factor's own work", "",
              "| run | play | prefetch total ms p50/p95/max | plans ahead p95/max | by location (n, p95, slow) | factor spans (outcomes, total ms, max ms) | slow GL calls |", "|---|---|---|---|---|---|---|"]
    for run in report["runs"]:
        for name, play in run["plays"].items():
            s = play["summary"]
            loc = "; ".join(f"{k}: {v['n']}, {v['p95']}, {v['slow']}" for k, v in s["by_location"].items())
            fs = run["factor_spans"]
            lines.append(f"| {run['order_index']} {run['arm']} | {name} | {s['prefetch_total_ms']['p50']}/{s['prefetch_total_ms']['p95']}/{s['prefetch_total_ms']['max']} | "
                         f"{s['plans_ahead_ms']['p95']}/{s['plans_ahead_ms']['max']} | {loc} | {fs['outcomes']}, {fs['total_ms']}, {fs['ms']['max']} | "
                         f"{json.dumps(run['gl_calls'], sort_keys=True)} |")
    lines += ["", "## Per arm and play kind (valid runs only; per-run values are listed, never averaged away)", "",
              "| arm/play | runs (quiet; hangs) | gaps >62.5 per run | >100 per run | max gap per run ms | p50 / p95 of the runs' gaps (median) | PNG entry gaps n p50/p95/max | cut entry gaps n p50/p95/max | pre-cut excess per run ms | memory: median of medians / peaks per run MiB |",
              "|---|---|---|---|---|---|---|---|---|---|"]
    for key, a in report["arms"].items():
        png, cut = a["png_entry_gaps"], a["cut_entry_gaps"]
        lines.append(f"| {key} | {a['runs']} ({a['quiet']}; {a['hangs']}) | {a['gaps_gt_62_5_per_run']} | {a['gaps_gt_100_per_run']} | {a['max_gap_per_run']} | {a['p50_gap_median_ms']} / {a['p95_gap_median_ms']} | "
                     f"{png['n']} {png['p50']}/{png['p95']}/{png['max']} | {cut['n']} {cut['p50']}/{cut['p95']}/{cut['max']} | {a['pre_cut_excess_per_run']} | "
                     f"{a['memory_median_of_medians_mib']} / {a['memory_peak_per_run_mib']} |")
    lines += ["", "## Pairs (control vs on; the same play of the same pair)", "",
              "| pair | play | comparable | entry gap ms control→on | PNG entries median ms | cut entries median ms | gaps >62.5 | >100 | max gap ms | pre-cut excess ms | peak MiB |",
              "|---|---|---|---|---|---|---|---|---|---|---|"]
    for r in report["pairs"]:
        pair = lambda key: f"{r[key]['control']}→{r[key]['on']}"
        lines.append(f"| {r['pair']} | {r['play']} | {'yes' if r['comparable'] else 'no (loaded or invalid)'} | {pair('entry_gap_ms')} | {pair('png_entry_gap_median_ms')} | "
                     f"{pair('cut_entry_gap_median_ms')} | {pair('slow_gt_62_5')} | {pair('slow_gt_100')} | {pair('max_gap_ms')} | {pair('pre_cut_excess_ms')} | {pair('memory_peak_mib')} |")
    lines += ["", f"Verdict (mechanical, pre-fixed rules): {report['verdict']}", ""]
    return "\n".join(lines)


def _in(entry, manifest):
    window = manifest.get("window")
    return not window or window[0] <= entry <= window[1]


def pooled(runs, arm, play, key):
    """Every entry gap of one arm's plays (all runs, one play kind) in one list, for p50/p95/max over the entries themselves."""
    return [e["gap_ms"] for run in runs if run["arm"] == arm and play in run["plays"] for e in run["plays"][play][key] if not e.get("missing")]


def arm_summary(runs):
    """Per arm and play kind over the VALID runs (invalid ones stay listed in the runs table, never pooled): medians across runs of the per-run
    figures, the pooled entry gaps, and the memory as the median of the sessions' medians and the median and maximum of their peaks."""
    out = {}
    for arm in ARMS:
        for play in ("first", "repeat"):
            mine = [r for r in runs if r["arm"] == arm and r["valid"] and play in r["plays"]]
            if not mine:
                continue
            plays = [r["plays"][play] for r in mine]
            med = lambda values: round(statistics.median(values), 1) if values else None
            valid_runs = [r for r in runs if r["valid"]]
            out[f"{arm}/{play}"] = {
                "runs": len(mine), "quiet": sum(1 for r in mine if r["quiet_class"] == "quiet"), "hangs": sum(1 for p in plays if p["hang"]),
                "gaps_gt_62_5_per_run": [p["summary"]["slow_gt_62_5"] for p in plays], "gaps_gt_100_per_run": [p["summary"]["slow_gt_100"] for p in plays],
                "max_gap_per_run": [p["summary"]["interval_ms"]["max"] for p in plays],
                "p50_gap_median_ms": med([p["summary"]["interval_ms"]["p50"] for p in plays]), "p95_gap_median_ms": med([p["summary"]["interval_ms"]["p95"] for p in plays]),
                "pre_cut_excess_per_run": [p["pre_cut_excess_ms"] for p in plays],
                "png_entry_gaps": RL.stats(pooled(valid_runs, arm, play, "png_entries")), "cut_entry_gaps": RL.stats(pooled(valid_runs, arm, play, "cut_entries")),
                "play_to_first_picture_per_run": [p["summary"]["play_command_to_first_picture_ms"] for p in plays],
                "memory_median_of_medians_mib": med([r["memory"]["rss_sum_mib"]["median"] for r in mine if r["memory"]["rss_sum_mib"]["median"] is not None]),
                "memory_peak_per_run_mib": [r["memory"]["rss_sum_mib"]["peak"] for r in mine],
            }
    return out


def build_report(records, project, manifest, factor):
    window = manifest.get("window") if manifest["mode"] == "narrow" else None
    runs = [analyze_run(r, project, factor, window) for r in records]
    entry = manifest["entry"]
    rows = pair_rows(runs, factor, entry)
    return {"manifest": manifest, "entry": entry, "project": {k: v for k, v in project.items() if k != "sources"}, "runs": runs, "pairs": rows, "arms": arm_summary(runs),
            "verdict": verdict(rows, factor, entry) if manifest["mode"] == "full" else "narrow: read the entry rows; one pair is a diagnostic, not a decision",
            "all_valid": all(r["valid"] for r in runs)}


def write_outputs(out, report, report_dir):
    (out / "analysis.json").write_text(json.dumps(report, indent=2))
    (out / "analysis.md").write_text(markdown(report))
    if report_dir:
        target = pathlib.Path(report_dir)
        target.mkdir(parents=True, exist_ok=True)
        (target / "analysis.json").write_text(json.dumps(report, indent=2))
        (target / "analysis.md").write_text(markdown(report))
    shas = "".join(f"{RL.sha256_file(p)}  {p.name}\n" for p in sorted(out.glob("run-*.json")))
    (out / "runs.sha256").write_text(shas)
    if report_dir:
        (pathlib.Path(report_dir) / "runs.sha256").write_text(shas)


# --------------------------------------------------------------------------------------------------- commands
def prerequisites(args, W):
    binary = pathlib.Path(args.binary).resolve()
    document = pathlib.Path(args.project).resolve()
    for needed in (binary, ROOT / "runtime" / "dist" / "runtime.js", document):
        if not needed.exists():
            print(f"BLOCKED: missing {needed}")
            return None
    project = RL.parse_project(json.loads(document.read_text()))
    if [s for s in RL.check_sources(project["sources"]) if not s["present"]]:
        print("BLOCKED: project source(s) missing")
        return None
    if W.free_gib() < RL.MIN_FREE_GIB_START:
        print(f"BLOCKED: {W.free_gib():.1f} GiB free")
        return None
    locks = W.acquire_locks(args.lock_wait)
    if locks is None:
        print(f"BLOCKED: the GPU bench locks were not free within {args.lock_wait:.0f} s")
        return None
    return binary, document, project, locks


def manifest_of(args, binary, document, mode, window, project):
    return {"git": RL.git_state(), "binary_sha256": RL.sha256_file(binary), "runtime_sha256": RL.sha256_file(ROOT / "runtime" / "dist" / "runtime.js"),
            "document_sha256": RL.sha256_file(document), "factor": args.factor, "mode": mode, "window": list(window) if window else None,
            "order": arm_order(args.pairs if mode == "full" else 1), "io": "async", "label": args.label, "started_unix": time.time(),
            "entry": args.entry if args.entry is not None else (196 if args.factor == "preupload" else project["cuts"][0]),
            "env_control": arm_env(args.factor, "control"), "env_on": arm_env(args.factor, "on")}


def cmd_run(args, mode):
    W = load_module("scene_warm_proof", ROOT / "scripts" / "scene-warm-proof.py")
    got = prerequisites(args, W)
    if got is None:
        return 2
    binary, document, project, locks = got
    window = (args.start, args.end) if mode == "narrow" else None
    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    manifest = manifest_of(args, binary, document, mode, window, project)
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2))
    print(f"build {manifest['git']['sha'][:9]} dirty={manifest['git']['dirty_files']} binary {manifest['binary_sha256'][:12]} order {manifest['order']}", flush=True)
    records = []
    try:
        for index, arm in enumerate(manifest["order"]):
            env = arm_env(args.factor, arm)
            if mode == "narrow":
                record = TP.run_iteration(W, binary, document, project, "async", window, index, arm_env=env)
                record.setdefault("host", {"loadavg_start": [round(v, 2) for v in os.getloadavg()]})
            else:
                record = RL.run_session(W, binary, document, project, "async", out, index, index, 0, env_extra=env, arm=arm)
            record["arm"], record["pair"], record["order_index"] = arm, index // 2, index
            (out / f"run-{index:02d}-{arm}.json").write_text(json.dumps(record))
            records.append(record)
            run = analyze_run(record, project, args.factor, window)
            print(f"  run {index} {arm} valid={run['valid']} {run['invalid_reasons']} class={run['quiet_class']}", flush=True)
    finally:
        for handle in locks:
            handle.close()
    manifest["finished_unix"] = time.time()
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2))
    return finish(out, records, project, manifest, args.factor, args.report_dir)


def finish(out, records, project, manifest, factor, report_dir):
    report = build_report(records, project, manifest, factor)
    write_outputs(out, report, report_dir)
    print(markdown(report))
    return 0 if report["all_valid"] else 1


def cmd_analyze(args):
    out = pathlib.Path(args.out)
    manifest = json.loads((out / "manifest.json").read_text())
    project = RL.parse_project(json.loads(pathlib.Path(args.project).read_text()))
    records = [json.loads(p.read_text()) for p in sorted(out.glob("run-*.json"))]
    return finish(out, records, project, manifest, manifest["factor"], args.report_dir)


def activation_verdicts(sessions, factor):
    """What makes the pixel proof mean something: the manipulation HAPPENED. The `on` session that played into the cut reports the factor's worked
    outcome in the page's own spans, no control session reports a span of the factor, and the host reports each arm's switch as it was asked.
    Missing data never passes (an empty list of sessions is not "all of them agree")."""
    spec = FACTORS[factor]
    on_hot = [s for s in sessions if s.get("factor_arm") == "on" and s.get("order") == "hot"]
    worked = [((s.get("spans") or {}).get(spec["span"]) or {}).get(spec["worked"], 0) for s in on_hot]
    control = [s for s in sessions if s.get("factor_arm") == "control"]
    shown = [(s.get("spans") or {}).get(spec["span"]) for s in control]
    switches = [((s.get("factors") or {}).get(spec["report"]) or {}).get("enabled") for s in sessions]
    return [
        (f"activation: the `on` session that played into the cut reports `{spec['span']}`/`{spec['worked']}` from the page",
         bool(on_hot) and all(n > 0 for n in worked), worked),
        (f"activation: no control session reports a `{spec['span']}` span", bool(control) and not any(shown), shown),
        (f"activation: the host reports `{spec['report']}.enabled` as each arm was asked", bool(sessions) and switches == [s.get("factor_arm") == "on" for s in sessions], switches),
    ]


def cmd_pixels(args):
    """The explicit frames of the SYNTHETIC project (no private media) are byte-identical with the factor off and on, hot and cold, in the real window.
    Only `warm_draw` has something to draw there (the project has splat scenes and no stills): the picture clip's pixels are proven in the
    real-Chrome runtime tests (`runtime/test/preupload-browser.test.mjs`), not here."""
    if args.factor != "warm_draw":
        print("N/A: the synthetic project has no stills; see runtime/test/preupload-browser.test.mjs for the pre-upload's pixel proof")
        return 0
    W = load_module("scene_warm_proof", ROOT / "scripts" / "scene-warm-proof.py")
    binary = pathlib.Path(args.binary).resolve()
    if W.free_gib() < args.min_free_gib:
        print(f"BLOCKED: {W.free_gib():.1f} GiB free")
        return 2
    locks = W.acquire_locks(args.lock_wait)
    if locks is None:
        print("BLOCKED: the GPU bench locks were not free")
        return 2
    original_env, original_retry = W.env_for, W.retry_while_preparing
    refused = []
    box = {"arm": "control"}

    def retrying(call, *rest, **named):
        answer = original_retry(call, *rest, **named)
        for _ in range(4):
            if not (isinstance(answer, dict) and not answer.get("ok")):
                break
            refused.append({"arm": box["arm"], "answer": str(answer)[:300]})
            if ((answer.get("error") or {}).get("code")) != "cancelled":
                break
            time.sleep(1.0)
            answer = original_retry(call, *rest, **named)
        return answer

    W.retry_while_preparing = retrying
    W.env_for = lambda arm, inspect: {**original_env("warm", inspect), "TESSEL_VIEWPORT_ASSET_IO": "async", "TESSEL_GL": "gpu", **arm_env(args.factor, box["arm"])}
    work = tempfile.mkdtemp(prefix="fuab-px-", dir="/tmp")
    results = []
    try:
        cuts, cut_frames = 5, 72
        document = W.build_project(str(binary), os.path.join(work, "build"), 200000, cuts, cut_frames)
        if not document:
            return 1
        frames = sorted({cut_frames + 5, 2 * cut_frames + 5, (cuts - 1) * cut_frames + 5, cut_frames + 40})
        number = 0
        for arm in ARMS:
            for order in ("hot", "cold"):
                box["arm"] = arm
                session = W.pixel_session(str(binary), document, "warm", order, frames, cut_frames, os.path.join(work, f"px{number}"))
                session["factor_arm"] = arm
                results.append(session)
                number += 1
    finally:
        W.env_for, W.retry_while_preparing = original_env, original_retry
        for handle in locks:
            handle.close()
        shutil.rmtree(work, ignore_errors=True)
    verdicts = W.pixel_verdicts(results, frames) + activation_verdicts(results, args.factor)
    failures = [name for name, ok, _ in verdicts if not ok]
    for name, ok, detail in verdicts:
        print(("PASS " if ok else "FAIL ") + name + ("" if ok else f"  [{str(detail)[:200]}]"))
    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    (out / "pixels.json").write_text(json.dumps({"factor": args.factor, "frames": frames, "sessions": results, "refused_requests": refused,
                                                  "verdicts": [{"check": n, "ok": ok} for n, ok, _ in verdicts]}, indent=2))
    return 1 if failures or any(s.get("error") for s in results) else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    for name in ("narrow", "full"):
        p = sub.add_parser(name)
        p.add_argument("--factor", choices=sorted(FACTORS), required=True)
        p.add_argument("--binary", default=str(ROOT / "target" / "release" / "tessel-editor"))
        p.add_argument("--project", required=True)
        p.add_argument("--out", required=True)
        p.add_argument("--report-dir")
        p.add_argument("--label", default="")
        p.add_argument("--entry", type=int, help="the entry judged by the verdict (default: 196 for preupload, the first cut for warm_draw)")
        p.add_argument("--lock-wait", type=float, default=15.0)
        if name == "narrow":
            p.add_argument("--from", dest="start", type=int, default=114)
            p.add_argument("--to", dest="end", type=int, default=210)
        else:
            p.add_argument("--pairs", type=int, default=4)
    px = sub.add_parser("pixels")
    px.add_argument("--factor", choices=sorted(FACTORS), required=True)
    px.add_argument("--binary", default=str(ROOT / "target" / "release" / "tessel-editor"))
    px.add_argument("--out", required=True)
    px.add_argument("--lock-wait", type=float, default=15.0)
    px.add_argument("--min-free-gib", type=float, default=RL.MIN_FREE_GIB_START,
                    help="the pixel proof writes under 1 GiB; a lower start floor is allowed only above the 12 GiB hard floor")
    ana = sub.add_parser("analyze")
    ana.add_argument("--project", required=True)
    ana.add_argument("--out", required=True)
    ana.add_argument("--report-dir")
    args = parser.parse_args()
    if args.command == "analyze":
        return cmd_analyze(args)
    if args.command == "pixels":
        return cmd_pixels(args)
    return cmd_run(args, args.command)


if __name__ == "__main__":
    raise SystemExit(main())
