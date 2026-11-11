#!/usr/bin/env python3
"""BL157.1: same-build A/B of the asset protocol (sync vs async reads) on a REAL project, in the real window.

    python3 scripts/real-latency-ab.py preflight --project P.tessel.json
    python3 scripts/real-latency-ab.py run      --binary target/release/tessel-editor --project P.tessel.json --out DIR [--pairs 4] [--charts]
    python3 scripts/real-latency-ab.py analyze  --out DIR --project P.tessel.json [--report-dir DIR2] [--charts]
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

`--charts` writes the SVGs of the gaps next to the analysis (and into `--report-dir`); the analysis never needs them.
`pixels` is a FUNCTIONAL proof (hashes of pixels): it takes no GPU lock. `run`, the performance measurement, takes both locks, and its manifest says
which (`locks`) and how long it waited (`lock_wait_s`).

Exit: 0 every session valid and comparable; 1 some session invalid or the pair set is not comparable (report says which);
2 a prerequisite is missing or a lock/disk guard refused (BLOCKED).
"""
import argparse
import json
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from ab import ROOT, warm_proof  # noqa: E402
from ab.batch import Experiment, analyze, arm_order, finish, manifest_of, prerequisites, run_arms  # noqa: E402
from ab.charts import write_charts  # noqa: E402
from ab.pixels import run_pixels  # noqa: E402
from ab.play import SLOW_DIAGNOSTIC_MS, SLOWER_DIAGNOSTIC_MS  # noqa: E402
from ab.project import check_sources, parse_project, sha256_file  # noqa: E402
from ab.session import POLL_SECONDS, session_env, whole_project_twice, run_session  # noqa: E402
from ab.stats import PAIR_MAX_DELTA_CORES, QUIET_MEDIAN_CORES, QUIET_P95_CORES  # noqa: E402
from ab.validity import analyze_session, session_validity  # noqa: E402

MODES = ("sync", "async")

PAGE_BUSY_MS = 10.0   # page decode/fetch/warm overlapping a gap by at least this long is reported as "with page work"
TRANSIT_SLOW_MS = 30.0  # the picture after the gap spent this long outside the runtime frame (queue, page thread, return)
UI_STALL_MS = 20.0  # a UI-thread protocol callback this long overlapping a gap is reported as "with a UI-thread stall"


# ------------------------------------------------------------------------------------------------ the experiment
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


def arm_groups(analyzed):
    """Valid sessions pooled per (mode, load class): what the slow gaps (> 62.5 ms) overlapped. Counts only; frames of a run
    are correlated, so these are descriptions, not samples of an effect size. Invalid sessions are not pooled (they stay listed)."""
    groups = {}
    for item in analyzed:
        if not item["valid"]:
            continue
        group = groups.setdefault(f"{item['mode']}/{item['quiet_class']}", {
            "sessions": 0, "plays": 0, "pictures": 0, "gaps_gt_62_5": 0, "gaps_gt_100": 0, "with_ui_stall": 0,
            "in_prefetch_window": 0, "at_cut": 0, "at_video_entry": 0, "other": 0, "with_read_overlap": 0, "with_page_work": 0,
            "with_slow_transit": 0, "with_neither_ui_stall_nor_page_work": 0, "worst_gap_ms_per_session": []})
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
                page = (context.get("page_decode_ms") or 0) + (context.get("page_fetch_ms") or 0) + (context.get("page_warm_ms") or 0)
                stall = (context["ui_callback_ms"] or 0) >= UI_STALL_MS
                busy = page >= PAGE_BUSY_MS
                group["with_page_work"] += 1 if busy else 0
                group["with_slow_transit"] += 1 if ((gap.get("picture") or {}).get("transit_ms") or 0) >= TRANSIT_SLOW_MS else 0
                group["with_neither_ui_stall_nor_page_work"] += 1 if not stall and not busy else 0
                where = gap["where"]
                group["in_prefetch_window" if where.startswith("prefetch") else "at_cut" if where.startswith("cut")
                      else "at_video_entry" if where.startswith("video") else "other"] += 1
        group["worst_gap_ms_per_session"].append(round(worst, 1))
    return groups


def build_report(sessions, project, manifest):
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
              f"| group | sessions | plays | pictures | gaps >{SLOW_DIAGNOSTIC_MS} | >100 | with UI-thread callback ≥{UI_STALL_MS:g} ms | overlapping a read | with page decode/fetch/warm ≥{PAGE_BUSY_MS:g} ms | picture transit ≥{TRANSIT_SLOW_MS:g} ms | neither UI stall nor page work | in a prefetch window | at a cut | at a video/still clip entry | elsewhere | worst gap per session ms |",
              "|" + "---|" * 16]
    for name, g in sorted(report["arms"].items()):
        lines.append(f"| {name} | {g['sessions']} | {g['plays']} | {g['pictures']} | {g['gaps_gt_62_5']} | {g['gaps_gt_100']} | {g['with_ui_stall']} | "
                     f"{g['with_read_overlap']} | {g['with_page_work']} | {g['with_slow_transit']} | {g['with_neither_ui_stall_nor_page_work']} | {g['in_prefetch_window']} | {g['at_cut']} | {g['at_video_entry']} | {g['other']} | {g['worst_gap_ms_per_session']} |")
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


# --------------------------------------------------------------------------------------------------- commands
def ran_as_claimed(report):
    return report["all_valid"] and report["n_comparable_pairs"] == len(report["sessions"]) // 2


EXPERIMENT = Experiment(record="session-{index:02d}-{arm}.json", build=build_report, markdown=markdown, ok=ran_as_claimed, charts=write_charts)


def cmd_run(args):
    run = prerequisites(args, EXPERIMENT)
    if run is None:
        return 2
    W = warm_proof()
    sources = check_sources(run.project["sources"])
    manifest = manifest_of(run, arm_order(MODES, args.pairs), pairs=args.pairs, stress_cpu=args.stress_cpu, poll_seconds=POLL_SECONDS, label=args.label,
                           sources=sources, sources_total=len(sources), sources_present=len(sources))

    def session(index, mode):
        return run_session(W, run.binary, run.document, {"number": index, "mode": mode}, session_env(mode), whole_project_twice(W, run.project),
                           args.stress_cpu)

    def summary(record):
        valid, reasons = session_validity(record)
        return f"session {record['order_index']} ({record['mode']}) valid={valid} {reasons}"

    records = run_arms(run, pathlib.Path(args.out), manifest, EXPERIMENT, session, summary)
    return finish(EXPERIMENT, pathlib.Path(args.out), records, run.project, manifest, args.report_dir, args.charts)


def cmd_pixels(args):
    """Explicit-frame pixels of the SYNTHETIC project are identical across the two modes and hot/cold (no private media)."""
    return run_pixels(pathlib.Path(args.binary).resolve(), args.out, [(mode, session_env(mode)) for mode in MODES])


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
    run.add_argument("--charts", action="store_true")
    run.add_argument("--pairs", type=int, default=4)
    run.add_argument("--lock-wait", type=float, default=600.0)
    run.add_argument("--stress-cpu", type=int, default=0, help="busy processes of this script, the same in both arms (adverse batch)")
    run.add_argument("--label", default="")
    ana = sub.add_parser("analyze")
    ana.add_argument("--out", required=True)
    ana.add_argument("--project", required=True)
    ana.add_argument("--report-dir")
    ana.add_argument("--charts", action="store_true")
    pix = sub.add_parser("pixels")
    pix.add_argument("--binary", default=str(ROOT / "target" / "release" / "tessel-editor"))
    pix.add_argument("--out", required=True)
    args = parser.parse_args()
    return {"preflight": cmd_preflight, "run": cmd_run, "analyze": lambda a: analyze(EXPERIMENT, a), "pixels": cmd_pixels}[args.command](args)


if __name__ == "__main__":
    raise SystemExit(main())
