#!/usr/bin/env python3
"""BL157.6: does asking the playback clock when the next frame is due (instead of every half frame) deliver the frames more evenly, in the real editor?

    python3 scripts/playback-poll-ab.py run     --binary B --project P.tessel.json --out DIR [--pairs 12]
    python3 scripts/playback-poll-ab.py analyze --out DIR --project P.tessel.json [--report-dir DIR2]

WHY. The playback loop asked the audio clock "which frame is it?" half a frame after each answer. With the timer's own lateness (about 1.5 ms) a 24 fps
frame is 1.87 polls long: 13% of the frame changes were seen at the first poll after them and 87% at the second, a regular judder of 22 and 44 ms pictures,
and a change was seen about 11 ms after its boundary on average (`scripts/playback-cadence.py` read it in 72 native sessions). `TESSEL_PLAYBACK_POLL=deadline`
keeps the question at least twice per frame but, when the next boundary is closer than half a frame, arms it for that boundary plus a 1 ms guard
(`crates/desktop/src/workstation/playback_poll.rs`). This asks the real editor, window and project whether that does what the model says.
It is about the evenness of the pictures and how soon a changed frame is requested. It is NOT about the 60-160 ms pauses BL157 is about: nothing here
says anything about them beyond "did not get worse", and nothing here is an acceptance by the user.

WHAT. `run` plays the whole project twice (a first play of a cold editor and a repeat in the same process, as `real-latency-ab.py` does) in 2 * --pairs
sessions of the SAME binary, one fresh editor each, in the order fixed/deadline, deadline/fixed, ... The arms differ ONLY in `TESSEL_PLAYBACK_POLL`
(`fixed` | `deadline`); everything else is the shipping default (asset I/O `sync`) with the control environment of `first-use-ab.py` (trace on, both
first-use factors explicitly 0). Each play is read with `scripts/playback-cadence.py` from the host trace. Each session is checked against the host's own
record: every `play_clock_started` must name the arm's schedule (`poll`); otherwise the run is INVALID ARMS and says nothing. A session that did not
leave exactly two plays that saw frames is `failed` (exit 1): it keeps its own row and is never dropped. `run` refuses to start (BLOCKED, exit 2) until
the rule file below is committed and unchanged, and writes its hash and commit into `manifest.json`: the rule comes before the numbers, in the history.

DECISION RULES, fixed here (and in `docs/viewport-real-latency-ab-results/poll-ab/rule.txt`) before any measurement:

    counted        a session that is valid (the harness's `session_validity`), did not fail and has its two plays. A pair is counted when both its sessions are.
    INVALID ARMS   a play whose `play_clock_started.poll` is not its arm, or a session that played without recording one: no verdict (a session
                   that died before its first play is `failed`, not this).
    INCONCLUSIVE   fewer than 10 counted pairs (of the 12 asked for): no verdict.
    CONTROL NOT REPRODUCED
                   fewer than 8% of the `fixed` arm's pictures came less than 30 ms after the previous one (the 72 native sessions: 11-23%): the pattern
                   under repair is not in this run, which then says nothing about the repair.
    REGRESSION     (control reproduced) in the `deadline` arm, compared with the `fixed` arm, ANY of:
                     - the share of pictures more than 62.5 ms after the previous one is more than 0.25 percentage points higher;
                     - the count of pictures more than 100 ms after the previous one is above 1.5 times the fixed arm's (scaled to the number of plays
                       of the deadline arm) plus 3;
                     - skipped frames per play are more than 0.15 higher;
                     - it has more hung sessions (a play that shows under 90% of the frames, or waits over 2000 ms between pictures) than the fixed arm.
    NO GAIN        (no regression) ANY of, for the `deadline` arm:
                     - more than 3% of the pictures came less than 30 ms after the previous one;
                     - the mean change lag (how long after its boundary a frame change was requested, from the clock the host traced) is above 4 ms,
                       or less than 5 ms below the fixed arm's;
                     - polls per frame change outside 1.9-2.15;
                     - the mean change lag is not lower in at least 90% of the counted pairs.
    ADOPT          none of the above: the evenness gain is real and nothing measured got worse. It permits making `deadline` the default; it is not
                   an acceptance of BL157 and the long pauses are untouched.

Load is recorded (`quiet_class`, external cores, load average), never filtered on. Sessions are never dropped; an uncounted one stays in its own row.

Exit: 0 every session ran (the verdict is in the report); 1 a session failed or an arm is not what it claims; 2 a prerequisite is missing or a lock/disk
guard refused (BLOCKED).
"""
import argparse
import math
import pathlib
import statistics
import subprocess
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from ab import ROOT, cadence, warm_proof  # noqa: E402
from ab.batch import Experiment, analyze, arm_order, finish, manifest_of, prerequisites, run_arms  # noqa: E402
from ab.play import hung_play  # noqa: E402
from ab.project import sha256_file  # noqa: E402
from ab.session import CONTROL_ENV, PLAYS, run_session, session_env, whole_project_twice  # noqa: E402
from ab.validity import analyze_session, error_label, published_error  # noqa: E402

FIXED, DEADLINE = "fixed", "deadline"
ARMS = (FIXED, DEADLINE)
POLL_ENV = "TESSEL_PLAYBACK_POLL"
IO_MODE = "sync"  # the shipping default: the arms differ in the poll schedule only

# The rule above, as numbers. `playback-poll-ab.spec.py` pins each against the docstring and `rule.txt`.
MIN_COUNTED_PAIRS = 10
CONTROL_SHORT_SHARE_MIN = 0.08
DEADLINE_SHORT_SHARE_MAX = 0.03
SLOW_SHARE_MARGIN = 0.0025  # 0.25 percentage points, as a share
VERY_SLOW_FACTOR, VERY_SLOW_PLUS = 1.5, 3
SKIPPED_PER_PLAY_MARGIN = 0.15
LAG_DEADLINE_MAX_MS = 4.0
LAG_GAIN_MIN_MS = 5.0
POLLS_PER_CHANGE_RANGE = (1.9, 2.15)
LAG_PAIR_CONSISTENCY = 0.9


RULE_PATH = pathlib.Path("docs") / "viewport-real-latency-ab-results" / "poll-ab" / "rule.txt"


def rule_state():
    """(True, None) when the rule file is in the git history and unchanged, else (False, why): a rule written after the numbers is not a rule, so `run`
    refuses to start before it is committed."""
    def git(*args):
        return subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True)
    if not (ROOT / RULE_PATH).exists():
        return False, f"{RULE_PATH} does not exist"
    if git("ls-files", "--error-unmatch", "--", str(RULE_PATH)).returncode != 0:
        return False, f"{RULE_PATH} is not tracked by git"
    if git("status", "--porcelain", "--", str(RULE_PATH)).stdout.strip():
        return False, f"{RULE_PATH} has changes that are not committed"
    return True, None


def rule_identity():
    """The rule file the run was started under: its hash and the commit that last changed it (both go into the manifest)."""
    commit = subprocess.run(["git", "log", "-1", "--format=%H", "--", str(RULE_PATH)], cwd=ROOT, capture_output=True, text=True).stdout.strip()
    return {"rule_sha256": sha256_file(ROOT / RULE_PATH), "rule_commit": commit or None}


# ------------------------------------------------------------------------------------------------ pure logic
def arm_findings(record):
    """Why a session is not the arm it says it is (an empty list when it is): the host's own `play_clock_started` names the schedule of each play."""
    arm = record.get("arm")
    named = [(event.get("fields") or {}).get("poll") for event in record.get("events") or [] if event.get("kind") == "play_clock_started"]
    if not named:
        # A session that died before it played has its own row (`failed`, with the error): it does not make the arms something else.
        return [] if record.get("error") else ["the trace has no `play_clock_started` (no play was recorded)"]
    wrong = sorted({str(poll) for poll in named if poll != arm})
    return [f"the host named the schedule {wrong} for a session of the {arm} arm"] if wrong else []


def session_row(record, project):
    """One session as a row: the harness's validity, the arm's own validity, and one entry per play."""
    analysis = analyze_session(record, project)
    read = cadence().each_play([(record.get("arm"), record)])
    plays = {}
    for name, found in zip(PLAYS, read):
        summary = analysis["plays"].get(name)
        plays[name] = {**found, "pictures": summary["pictures"] if summary else None, "max_gap_ms": summary["interval_ms"]["max"] if summary else None,
                       "hung": hung_play(summary, project["length"]) if summary else False}
    hung = any(play["hung"] for play in plays.values())
    findings = arm_findings(record)
    complete = len(read) == len(PLAYS) and all(name in analysis["plays"] for name in PLAYS)
    # A hang is a result of the arm (the `hung` guard of the rule speaks to it), not a failure of the run: a hung session is never `failed`, but it
    # is only counted with its two plays.
    failed = not hung and (bool(record.get("error")) or not complete)
    return {"arm": record.get("arm"), "pair": record.get("pair"), "order_index": record.get("order_index"), "valid": analysis["valid"],
            "invalid_reasons": analysis["invalid_reasons"], "arm_findings": findings, "plays": plays, "hung": hung, "failed": failed,
            "counted": analysis["valid"] and complete and not failed and not findings,
            "error": published_error(record["error"]) if record.get("error") else None,
            "quiet_class": analysis["quiet_class"], "external_median_cores": analysis["external_median_cores"],
            "external_p95_cores": analysis["external_p95_cores"], "loadavg_start": analysis["loadavg_start"],
            "loadavg_peak_1m": analysis["loadavg_peak_1m"]}


def pooled_mean(values):
    """Mean of (n, mean) pairs weighted by n; None when there is nothing."""
    total = sum(n for n, _ in values)
    return round(sum(n * mean for n, mean in values) / total, 3) if total else None


def lag_of(play):
    """(over how many frame changes the lag was read, its mean): a play without a traced lag has no mean and weighs nothing."""
    lag = play["change_lag_ms"]
    return lag["n"], lag["mean"] or 0.0


def arm_summary(rows):
    """The numbers of an arm over its counted sessions (counts, then the shares they make)."""
    mine = [row for row in rows if row["counted"]]
    plays = [play for row in mine for play in row["plays"].values()]
    delivered = sum(play["delivered"] for play in plays)
    under, slow, slower = (sum(play[key] for play in plays) for key in ("under_30_ms", "over_62_5_ms", "over_100_ms"))
    changes = sum(play["frame_changes"] for play in plays)
    per_play = sorted(play["under_30_ms"] / play["delivered"] for play in plays if play["delivered"])
    return {"sessions": len(rows), "counted": len(mine), "hung": sum(1 for row in rows if row["hung"]), "failed": sum(1 for row in rows if row["failed"]),
            "plays": len(plays), "delivered": delivered, "under_30_ms": under, "slow_gt_62_5": slow, "slow_gt_100": slower,
            "under_30_share": round(under / delivered, 4) if delivered else None, "slow_share": round(slow / delivered, 5) if delivered else None,
            "skipped_per_play": round(sum(play["skipped_frames"] for play in plays) / len(plays), 3) if plays else None,
            "change_lag_mean_ms": pooled_mean([lag_of(play) for play in plays]),
            "polls_per_frame_change": pooled_mean([(play["frame_changes"], play["polls_per_frame_change"] or 0.0) for play in plays]) if changes else None,
            "under_30_share_per_play": {"min": round(per_play[0], 4), "median": round(statistics.median(per_play), 4), "max": round(per_play[-1], 4)} if per_play else None}


def lag_of_session(row):
    return pooled_mean([lag_of(play) for play in row["plays"].values()])


def pair_lags(rows):
    """[(pair, fixed mean lag, deadline mean lag)] of the pairs whose two sessions are counted."""
    by_pair = {}
    for row in rows:
        by_pair.setdefault(row["pair"], {})[row["arm"]] = row
    found = []
    for pair, sides in sorted(by_pair.items(), key=lambda item: (item[0] is None, item[0])):
        if all(arm in sides and sides[arm]["counted"] for arm in ARMS):
            fixed, deadline = lag_of_session(sides[FIXED]), lag_of_session(sides[DEADLINE])
            if fixed is not None and deadline is not None:
                found.append((pair, fixed, deadline))
    return found


def regressions(fixed, deadline):
    """What got worse in the deadline arm (the guards of the rule), as sentences; empty when nothing did."""
    found = []
    if deadline["slow_share"] is not None and fixed["slow_share"] is not None and deadline["slow_share"] > fixed["slow_share"] + SLOW_SHARE_MARGIN:
        found.append(f"pictures > 62.5 ms after the previous one: {deadline['slow_share']:.3%} against {fixed['slow_share']:.3%} (more than {SLOW_SHARE_MARGIN:.2%} higher)")
    # Counts, not shares: the fixed arm's count is scaled to the plays the deadline arm has (the arms may have counted a different number of sessions).
    expected = fixed["slow_gt_100"] * deadline["plays"] / fixed["plays"] if fixed["plays"] else 0
    if deadline["slow_gt_100"] > VERY_SLOW_FACTOR * expected + VERY_SLOW_PLUS:
        found.append(f"pictures > 100 ms after the previous one: {deadline['slow_gt_100']} in {deadline['plays']} plays against {fixed['slow_gt_100']} in {fixed['plays']} "
                     f"(above {VERY_SLOW_FACTOR} x {expected:.1f} + {VERY_SLOW_PLUS})")
    if deadline["skipped_per_play"] is not None and fixed["skipped_per_play"] is not None and deadline["skipped_per_play"] > fixed["skipped_per_play"] + SKIPPED_PER_PLAY_MARGIN:
        found.append(f"skipped frames per play: {deadline['skipped_per_play']} against {fixed['skipped_per_play']} (more than {SKIPPED_PER_PLAY_MARGIN} higher)")
    if deadline["hung"] > fixed["hung"]:
        found.append(f"hung sessions: {deadline['hung']} against {fixed['hung']}")
    return found


def shortfalls(fixed, deadline, pairs):
    """What the deadline arm did not achieve (the gain of the rule), as sentences; empty when it achieved all of it."""
    found = []
    if deadline["under_30_share"] is None or deadline["under_30_share"] > DEADLINE_SHORT_SHARE_MAX:
        found.append(f"pictures < 30 ms after the previous one: {deadline['under_30_share']} (more than {DEADLINE_SHORT_SHARE_MAX:.0%})")
    lag, base = deadline["change_lag_mean_ms"], fixed["change_lag_mean_ms"]
    if lag is None or base is None:
        found.append("no change lag was traced (the host did not trace the clock)")
    else:
        if lag > LAG_DEADLINE_MAX_MS:
            found.append(f"mean change lag {lag} ms (above {LAG_DEADLINE_MAX_MS} ms)")
        if base - lag < LAG_GAIN_MIN_MS:
            found.append(f"mean change lag {lag} ms against {base} ms: {base - lag:.3f} ms lower (less than {LAG_GAIN_MIN_MS} ms)")
    polls = deadline["polls_per_frame_change"]
    if polls is None or not POLLS_PER_CHANGE_RANGE[0] <= polls <= POLLS_PER_CHANGE_RANGE[1]:
        found.append(f"polls per frame change {polls} (outside {POLLS_PER_CHANGE_RANGE[0]}-{POLLS_PER_CHANGE_RANGE[1]})")
    better = sum(1 for _, before, after in pairs if after < before)
    if not pairs or better < math.ceil(LAG_PAIR_CONSISTENCY * len(pairs)):
        found.append(f"the mean change lag was lower in {better} of {len(pairs)} counted pairs (needs {LAG_PAIR_CONSISTENCY:.0%})")
    return found


def verdict(rows, requested_pairs=None):
    """Mechanical, with the rules of the docstring. It says what the numbers do; it never says accepted."""
    findings = sorted({finding for row in rows for finding in row["arm_findings"]})
    if findings:
        return f"INVALID ARMS: {'; '.join(findings)}"
    fixed, deadline = (arm_summary([row for row in rows if row["arm"] == arm]) for arm in ARMS)
    pairs = pair_lags(rows)
    if len(pairs) < MIN_COUNTED_PAIRS:
        return f"INCONCLUSIVE: {len(pairs)} counted pair(s){f' of {requested_pairs}' if requested_pairs else ''}, the rule needs {MIN_COUNTED_PAIRS}"
    if fixed["under_30_share"] is None or fixed["under_30_share"] < CONTROL_SHORT_SHARE_MIN:
        return (f"CONTROL NOT REPRODUCED: {fixed['under_30_share']} of the fixed arm's pictures came less than 30 ms after the previous one "
                f"(the rule needs {CONTROL_SHORT_SHARE_MIN:.0%}): the pattern under repair is not in this run")
    bad = regressions(fixed, deadline)
    if bad:
        return f"REGRESSION: {'; '.join(bad)}"
    short = shortfalls(fixed, deadline, pairs)
    if short:
        return f"NO GAIN: {'; '.join(short)}"
    return (f"ADOPT: {deadline['under_30_share']:.2%} of the deadline arm's pictures came less than 30 ms after the previous one (fixed {fixed['under_30_share']:.2%}); "
            f"mean change lag {deadline['change_lag_mean_ms']} ms (fixed {fixed['change_lag_mean_ms']} ms); nothing measured got worse")


# ------------------------------------------------------------------------------------------------ analysis
# ------------------------------------------------------------------------------------------------ analysis
def build_report(records, project, manifest):
    rows = [session_row(record, project) for record in records]
    summaries = {arm: arm_summary([row for row in rows if row["arm"] == arm]) for arm in ARMS}
    pairs = pair_lags(rows)
    said = verdict(rows, (manifest.get("pairs") or None))
    return {"manifest": manifest, "project": {key: value for key, value in project.items() if key != "sources"}, "rows": rows, "arms": summaries,
            "counted_pairs": [{"pair": pair, "fixed_lag_ms": fixed, "deadline_lag_ms": deadline} for pair, fixed, deadline in pairs],
            "verdict": said, "all_ran": not any(row["failed"] for row in rows) and not any(row["arm_findings"] for row in rows)}


def markdown(report):
    m = report["manifest"]
    lines = ["# Playback poll A/B (BL157.6): the clock asked every half frame, and when the next frame is due, in the real editor", "",
             f"- code `{m['git']['sha'][:12]}` dirty files {m['git']['dirty_files']}; binary sha256 `{m['binary_sha256']}`; document sha256 `{m['document_sha256']}`",
             f"- the SAME binary for both arms; the arms differ only in `{POLL_ENV}`; asset I/O `{m.get('io', IO_MODE)}`; every session a fresh editor playing the project twice; "
             f"{m.get('stress_cpu', 0)} busy-loop process(es) alongside each session",
             f"- order {m['order']}", "",
             "## Per arm (counted sessions only)", "",
             "| arm | sessions | counted | hung | failed | plays | pictures (gaps) | < 30 ms | share | per play min / median / max | > 62.5 ms | share | > 100 ms | skipped frames per play | mean change lag ms | polls per frame change |",
             "|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|"]
    for arm in ARMS:
        a = report["arms"][arm]
        per_play = a["under_30_share_per_play"] or {}
        lines.append(f"| {arm} | {a['sessions']} | {a['counted']} | {a['hung']} | {a['failed']} | {a['plays']} | {a['delivered']} | {a['under_30_ms']} | {a['under_30_share']} | "
                     f"{per_play.get('min')} / {per_play.get('median')} / {per_play.get('max')} | {a['slow_gt_62_5']} | {a['slow_share']} | {a['slow_gt_100']} | "
                     f"{a['skipped_per_play']} | {a['change_lag_mean_ms']} | {a['polls_per_frame_change']} |")
    lines += ["", "## Counted pairs: mean change lag (ms)", "", "| pair | fixed | deadline |", "|---|---|---|"]
    for pair in report["counted_pairs"]:
        lines.append(f"| {pair['pair']} | {pair['fixed_lag_ms']} | {pair['deadline_lag_ms']} |")
    lines += ["", "## Sessions", "",
              "| # | pair | arm | class | ext median/p95 cores | loadavg start / peak 1m | first play: > 62.5 / max gap ms / lag ms | repeat play: > 62.5 / max gap ms / lag ms | counted | note |",
              "|---|---|---|---|---|---|---|---|---|---|"]
    for r in report["rows"]:
        def cell(name):
            play = r["plays"].get(name)
            return "missing" if play is None else f"{play['over_62_5_ms']} / {play['max_gap_ms']} / {play['change_lag_ms']['mean']}{' HUNG' if play['hung'] else ''}"
        note = "; ".join(r["invalid_reasons"] + r["arm_findings"] + ([f"FAILED: {error_label(r['error']) if r['error'] else 'incomplete'}"] if r["failed"] else [])) or "-"
        lines.append(f"| {r['order_index']} | {r['pair']} | {r['arm']} | {r['quiet_class']} | {r['external_median_cores']}/{r['external_p95_cores']} | "
                     f"{r['loadavg_start']} / {r['loadavg_peak_1m']} | {cell('first')} | {cell('repeat')} | {'yes' if r['counted'] else 'NO'} | {note} |")
    lines += ["", f"Verdict (mechanical, pre-fixed rules): {report['verdict']}", ""]
    return "\n".join(lines)


# ------------------------------------------------------------------------------------------------ commands
def arm_env(arm):
    """The environment of an arm: the control environment of every experiment (the first-use factors and the probe's GPU wait explicitly 0) and the poll schedule."""
    return {**CONTROL_ENV, POLL_ENV: arm}


def rule_gate():
    """Why the run may not start (the decision rule is not fixed before the numbers), or None."""
    committed, why = rule_state()
    return None if committed else f"the decision rule is not fixed before the run: {why}"


EXPERIMENT = Experiment(record="run-{index:02d}-{arm}.json", build=build_report, markdown=markdown, ok=lambda report: report["all_ran"])


def cmd_run(args):
    runtime = ROOT / "runtime" / "dist" / "runtime.js"
    run = prerequisites(args, EXPERIMENT, needed=(ROOT / "runtime" / "dist" / "index.html", runtime), gate=rule_gate)
    if run is None:
        return 2
    W = warm_proof()
    manifest = manifest_of(run, arm_order(ARMS, args.pairs), runtime_js=runtime, pairs=args.pairs, io=IO_MODE, env=CONTROL_ENV, stress_cpu=args.stress_cpu, **rule_identity())

    def session(index, arm):
        return run_session(W, run.binary, run.document, {"number": index, "mode": IO_MODE}, session_env(IO_MODE, arm_env(arm)),
                           whole_project_twice(W, run.project), args.stress_cpu)

    def summary(record):
        row = session_row(record, run.project)
        return (f"  run {record['order_index']} {record['arm']} counted={row['counted']} hung={row['hung']} failed={row['failed']} valid={row['valid']} "
                f"class={row['quiet_class']} lag={lag_of_session(row)} findings={row['arm_findings']}")

    records = run_arms(run, pathlib.Path(args.out), manifest, EXPERIMENT, session, summary)
    return finish(EXPERIMENT, pathlib.Path(args.out), records, run.project, manifest, args.report_dir)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    run = sub.add_parser("run")
    run.add_argument("--binary", required=True)
    run.add_argument("--project", required=True)
    run.add_argument("--out", required=True)
    run.add_argument("--pairs", type=int, default=12)
    run.add_argument("--stress-cpu", type=int, default=0, help="busy-loop processes kept alive during each session (both arms get the same)")
    run.add_argument("--lock-wait", type=float, default=600.0)
    run.add_argument("--report-dir")
    ana = sub.add_parser("analyze")
    ana.add_argument("--out", required=True)
    ana.add_argument("--project", required=True)
    ana.add_argument("--report-dir")
    args = parser.parse_args()
    return cmd_run(args) if args.command == "run" else analyze(EXPERIMENT, args)


if __name__ == "__main__":
    sys.exit(main())
