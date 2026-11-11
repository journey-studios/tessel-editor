#!/usr/bin/env python3
"""BL157.4: does dropping `reader.releaseLock()` from Spark's decoder worker (runtime/scripts/build.mjs) stop the page of the real editor from hanging?

    python3 scripts/runtime-bundle-ab.py run     --binary B --project P.tessel.json --unpatched-bundle RUNTIME.JS --out DIR [--pairs 30]
    python3 scripts/runtime-bundle-ab.py analyze --out DIR --project P.tessel.json [--report-dir DIR2]

WHY. The perf lab (a bare WKWebView host) found a WebKit deadlock: a worker inside `ReadableStreamDefaultReader.releaseLock()` waits for a lock the
garbage collector holds, the decode never answers, and a pending collection can freeze the whole page. BL157.2 recorded native sessions in which the
page stopped answering. The patch is proven in the lab and by unit tests; this asks the same question of the REAL editor, window and project.

WHAT. `run` plays the whole project twice (a first play of a cold editor and a repeat in the same process, as `real-latency-ab.py` does) in
2 * --pairs sessions of the SAME binary, one fresh editor each, in the order unpatched/patched, patched/unpatched, ... The arms differ ONLY in the
`runtime.js` the page loads: `TESSEL_RUNTIME_DIR` of a session is a scratch folder with that arm's bundle and the repository's `index.html`. Everything
else is the control arm of `first-use-ab.py` (async asset I/O, trace on, scene warm on, both first-use factors explicitly 0).

THE BUNDLES. The patch is OPT-IN in the build, so a plain `./scripts/runtime build` makes the UNPATCHED bundle. `--unpatched-bundle` is a `runtime.js` of such a
plain build, copied out of `runtime/dist` (a path argument; it stays out of the repository) before the patched one is built; the PATCHED arm is the
`runtime/dist/runtime.js` that `TESSEL_RUNTIME_SPARK_RELEASE_LOCK_PATCH=1 ./scripts/runtime build` leaves there. A `dist` left by a plain build is therefore
refused as the patched arm (BLOCKED, with the command that builds it).
Each bundle is read before the first session: the unpatched one must contain the decoder's call once, the patched one not at all. After the sessions, each
one is checked against the host's own `asset_io` record of the `runtime.js` it served: within an arm every session got the same number of bytes (the size of the
arm's file) and the unpatched bundle is exactly what the patch removes longer than the patched one; otherwise the run is INVALID ARMS and says nothing.

While a session plays, a watchdog reads the frame table: when the editor is playing and no new picture arrived for STALL_SECONDS, it samples the
page's WebContent process (`sample`, read only, twice at most per session) and keeps the file next to the session. A sample is a DEADLOCK sample only when the
decoder worker is inside `releaseLock` waiting for a lock the collector's marking holds: that is the lab's stack, seen natively. NOTHING IS EVER SIGNALLED:
the WebKit helpers are not children of the editor (they are told apart by "born during the session"), so no process here can be shown to be the session's own.
A frozen page that outlives its editor is reported (pid, role and the sample file, as `leftover_helpers`) and the operator decides what to do with it.

DECISION RULES, fixed here before any measurement (diagnostic; nothing here decides the acceptance of BL157):

    hung play      the play shows fewer than 90% of the project's frames, or waits more than 2000 ms between two pictures (the rule of `first-use-ab.py`).
    hung session   at least one of its plays is hung. Hung and `failed` are independent: a session that errors, records errors, loses a play or whose
                   editor dies is `failed` (exit 1) whether or not a play of it also hung, and a failed session without a hung play is never counted as hung.
    NOT REPRODUCED the unpatched arm has fewer than 3 hung sessions: this run says nothing about the patch.
    CONFIRMED      the unpatched arm has at least 3 hung sessions and the patched arm none.
    REDUCED        the patched arm has some, fewer than the unpatched one: the hang has another cause too; the patch is not called a fix.
    NO EFFECT      the patched arm has as many hung sessions as the unpatched one or more.
    The exact one-sided p-value of "so few hung sessions in the patched arm if both arms had the same chance" is printed as a diagnostic only.

Load is recorded, never filtered on: a hang that needs a busy machine is still a hang. Sessions are never dropped; an invalid one stays in its own row.

Exit: 0 every session ran (the verdict is in the report); 1 a session failed or an arm is not what it claims; 2 a prerequisite is missing or a
lock/disk guard refused (BLOCKED).
"""
import argparse
import math
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from ab import ROOT, warm_proof  # noqa: E402
from ab.batch import Experiment, analyze, arm_order, finish, manifest_of, prerequisites, run_arms  # noqa: E402
from ab.play import hung_play  # noqa: E402
from ab.project import sha256_file  # noqa: E402
from ab.session import CONTROL_ENV, PLAYS, collect as poll_trace, process_table, run_session, session_env, webkit_pids, whole_project_twice  # noqa: E402
from ab.stats import stats  # noqa: E402
from ab.validity import analyze_session, error_label, failed_session, published_error  # noqa: E402

UNPATCHED, PATCHED = "unpatched", "patched"
ARMS = (UNPATCHED, PATCHED)
# What `runtime/scripts/build.mjs` drops (its SPARK_RELEASE_LOCK): present once in an unpatched bundle, nowhere in a patched one.
DECODER_CALL = "reader.releaseLock();"
# What the build puts where the call was: the patched bundle is `len(DECODER_CALL) - len(PATCH_REPLACEMENT)` bytes shorter.
PATCH_REPLACEMENT = "void 0;"
# The patch is opt-in in the build: this is what leaves the PATCHED arm in `runtime/dist/runtime.js` (a plain build leaves the unpatched bundle there).
PATCHED_BUILD = "TESSEL_RUNTIME_SPARK_RELEASE_LOCK_PATCH=1 ./scripts/runtime build"
MIN_UNPATCHED_HANGS = 3
STALL_SECONDS = 10.0
MAX_SAMPLES_PER_SESSION = 2


# ------------------------------------------------------------------------------------------------ pure logic
def bundle_findings(arm, text):
    """Why a bundle is not what its arm says it is (an empty list when it is)."""
    count = text.count(DECODER_CALL)
    if arm == UNPATCHED and count != 1:
        return [f"the unpatched bundle has the decoder's `{DECODER_CALL}` {count} time(s), expected exactly 1 (--unpatched-bundle is the runtime.js of a plain "
                "`./scripts/runtime build`, without the patch variable)"]
    if arm == PATCHED and count != 0:
        return [f"the patched bundle still has the decoder's `{DECODER_CALL}` ({count} time(s)); runtime/dist/runtime.js is the PATCHED arm and the patch is "
                f"opt-in, so build it with `{PATCHED_BUILD}` (a plain build leaves the unpatched bundle)"]
    return []


def served_runtime_bytes(events):
    """Bytes of the `runtime.js` the host served the page of a session: the host's own `asset_io` event (written by the host, not by the page), or None
    when the event is not in the record."""
    for event in events or []:
        fields = event.get("fields") or {}
        if event.get("kind") == "asset_io" and fields.get("asset") == "/runtime.js" and fields.get("status") == 200:
            return fields.get("bytes")
    return None


def served_bundle_check(rows, expected=None):
    """Did every session load the bundle of its arm? Within an arm all sessions were served the same number of bytes (the size of the arm's file, when the
    manifest has it), and the unpatched bundle is exactly what the patch removes longer than the patched one. A session whose record has no such event is
    `unverified` (counted, not a finding)."""
    sizes = {arm: sorted({r["served_runtime_bytes"] for r in rows if r["arm"] == arm and r.get("served_runtime_bytes") is not None}) for arm in ARMS}
    findings = [f"the {arm} sessions were served {len(sizes[arm])} different runtime.js sizes: {sizes[arm]}" for arm in ARMS if len(sizes[arm]) > 1]
    for arm in ARMS:
        if expected and len(sizes[arm]) == 1 and sizes[arm][0] != expected.get(arm):
            findings.append(f"the {arm} sessions were served {sizes[arm][0]} bytes, the arm's bundle has {expected.get(arm)}")
    if len(sizes[UNPATCHED]) == 1 and len(sizes[PATCHED]) == 1:
        difference, removed = sizes[UNPATCHED][0] - sizes[PATCHED][0], len(DECODER_CALL) - len(PATCH_REPLACEMENT)
        if difference != removed:
            findings.append(f"the served bundles differ by {difference} bytes, the patch removes {removed}")
    return {"bytes": sizes, "unverified": sum(1 for r in rows if r.get("served_runtime_bytes") is None), "findings": findings}


def deadlock_signature(text):
    """What a `sample` of the frozen page must show to be the lab's deadlock: a worker inside `releaseLock`, the collector's marking asking for a lock
    (`visitOutputConstraints` or `isReachableFromOpaqueRoots` of the same reader) and a thread parked in a lock's slow path."""
    found = {"release_lock": "ReadableStreamDefaultReader::releaseLock" in text,
             "collector": "visitOutputConstraints" in text or "isReachableFromOpaqueRoots" in text,
             "lock_wait": "lockSlow" in text}
    return {**found, "deadlock": all(found.values())}


def chance_of_so_few(hung_unpatched, n_unpatched, hung_patched, n_patched):
    """Exact one-sided (hypergeometric) probability that the patched arm holds `hung_patched` or fewer of all the hung sessions if both arms had the same
    chance of hanging. A diagnostic number: the verdict is the rule above."""
    total, hung = n_unpatched + n_patched, hung_unpatched + hung_patched
    if n_patched == 0 or n_unpatched == 0 or hung == 0:
        return 1.0
    ways = math.comb(total, n_patched)
    return sum(math.comb(hung, i) * math.comb(total - hung, n_patched - i) for i in range(0, min(hung_patched, n_patched) + 1)) / ways


def counts(rows):
    out = {}
    for arm in ARMS:
        mine = [r for r in rows if r["arm"] == arm]
        out[arm] = {"sessions": len(mine), "hung": sum(1 for r in mine if r["hung"]), "failed": sum(1 for r in mine if r["failed"]),
                    "invalid": sum(1 for r in mine if not r["valid"]), "deadlock_samples": sum(1 for r in mine for s in r["samples"] if s["signature"]["deadlock"])}
    return out


def gap_summary(rows):
    """What the plays of each arm looked like next to the hang rule: the state the editor was in, NOT a comparison this experiment decides (its sessions
    share a machine, not a clock). Per arm: the largest wait between two pictures of each play, the slow gaps (the harness's 62.5 and 100 ms marks), and
    in how many plays each location of the project (a cut, a prefetch, a video start, 'other') had a slow gap."""
    out = {}
    for arm in ARMS:
        plays = [p for r in rows if r["arm"] == arm for p in r["plays"].values()]
        where = {}
        for play in plays:
            for location in play.get("slow_where", []):
                where[location] = where.get(location, 0) + 1
        out[arm] = {"plays": len(plays), "max_gap_ms": stats([p["max_gap_ms"] for p in plays]),
                    "slow_gt_62_5": sum(p.get("slow_gt_62_5", 0) for p in plays), "slow_gt_100": sum(p.get("slow_gt_100", 0) for p in plays),
                    "plays_with_gap_gt_100": sum(1 for p in plays if (p["max_gap_ms"] or 0) > 100),
                    "plays_with_a_slow_gap_at": dict(sorted(where.items(), key=lambda item: (-item[1], item[0])))}
    return out


def verdict(rows):
    """Mechanical, with the rules of the docstring. It says what the numbers do; it never says accepted."""
    c = counts(rows)
    u, p = c[UNPATCHED], c[PATCHED]
    if not u["sessions"] or not p["sessions"]:
        return "INCONCLUSIVE: an arm has no session"
    said = f"unpatched {u['hung']} of {u['sessions']} sessions hung, patched {p['hung']} of {p['sessions']}"
    if u["hung"] < MIN_UNPATCHED_HANGS:
        return f"NOT REPRODUCED: {said}; the hang did not show enough in the unpatched arm for this run to say anything about the patch"
    if p["hung"] == 0:
        return f"CONFIRMED: {said}"
    if p["hung"] < u["hung"]:
        return f"REDUCED: {said}; the hang has another cause as well, the patch is not called a fix"
    return f"NO EFFECT: {said}"


# ------------------------------------------------------------------------------------------------ analysis
def session_row(record, project):
    analysis = analyze_session(record, project)
    plays = {name: {"pictures": p["pictures"], "max_gap_ms": p["interval_ms"]["max"], "hung": hung_play(p, project["length"]),
                    "slow_gt_62_5": p.get("slow_gt_62_5", 0), "slow_gt_100": p.get("slow_gt_100", 0),
                    "slow_where": sorted(where for where, stats in (p.get("by_location") or {}).items() if stats.get("slow"))}
             for name, p in analysis["plays"].items()}
    hung = any(p["hung"] for p in plays.values())
    samples = [{key: s.get(key) for key in ("at_s", "pid", "role", "file")} | {"signature": s["signature"]} for s in record.get("hang_samples") or []]
    return {"arm": record["arm"], "pair": record.get("pair"), "order_index": record.get("order_index"), "valid": analysis["valid"],
            "invalid_reasons": analysis["invalid_reasons"], "plays": plays, "hung": hung,
            "served_runtime_bytes": served_runtime_bytes(record.get("events")),
            "failed": failed_session(record, all(name in plays for name in PLAYS)),
            "error": published_error(record["error"]) if record.get("error") else None,
            "quiet_class": analysis["quiet_class"], "external_median_cores": analysis["external_median_cores"],
            "external_p95_cores": analysis["external_p95_cores"], "loadavg_start": analysis["loadavg_start"],
            "loadavg_peak_1m": analysis["loadavg_peak_1m"], "samples": samples, "leftover_helpers": record.get("leftover_helpers") or []}


def build_report(records, project, manifest):
    rows = [session_row(r, project) for r in records]
    c = counts(rows)
    served = served_bundle_check(rows, manifest.get("bundle_bytes"))
    said = verdict(rows)
    # A session that did not load its arm's bundle makes the arms something else: the numbers below would be about nothing.
    final = f"INVALID ARMS: {'; '.join(served['findings'])} (the rules alone would say: {said})" if served["findings"] else said
    return {"manifest": manifest, "project": {k: v for k, v in project.items() if k != "sources"}, "rows": rows, "counts": c, "verdict": final,
            "served_bundle": served, "play_gaps": gap_summary(rows),
            "chance_of_so_few": round(chance_of_so_few(c[UNPATCHED]["hung"], c[UNPATCHED]["sessions"], c[PATCHED]["hung"], c[PATCHED]["sessions"]), 4),
            "all_ran": not any(r["failed"] for r in rows) and not any(r["leftover_helpers"] for r in rows) and not served["findings"]}


def status_of(row):
    """The `hung` cell of a session: hung and failed are independent, so both are said."""
    found = (["HUNG"] if row["hung"] else []) + ([f"FAILED: {error_label(row['error']) if row['error'] else 'incomplete'}"] if row["failed"] else [])
    return ", ".join(found) or "no"


def markdown(report):
    m = report["manifest"]
    lines = ["# Runtime bundle A/B (BL157.4): the decoder's `releaseLock` kept and dropped, in the real editor", "",
             f"- code `{m['git']['sha'][:12]}` dirty files {m['git']['dirty_files']}; binary sha256 `{m['binary_sha256']}`; document sha256 `{m['document_sha256']}`",
             f"- bundles: unpatched sha256 `{m['bundle_sha256'][UNPATCHED]}`, patched sha256 `{m['bundle_sha256'][PATCHED]}` (the only difference between the arms)",
             f"- the SAME binary for both arms; asset I/O `async`; scene warm on; first-use factors off; every session a fresh editor playing the project twice; "
             f"{m.get('stress_cpu', 0)} busy-loop process(es) alongside each session",
             f"- order {m['order']}"]
    served = report["served_bundle"]
    lines.append(f"- `runtime.js` the host served (its own `asset_io` event): unpatched {served['bytes'][UNPATCHED]} bytes, patched {served['bytes'][PATCHED]} bytes; "
                 f"sessions without that event: {served['unverified']}; findings: {'; '.join(served['findings']) or 'none'}")
    lines += ["", "## Per arm", "", "| arm | sessions | hung | failed | invalid | deadlock samples |", "|---|---|---|---|---|---|"]
    for arm in ARMS:
        c = report["counts"][arm]
        lines.append(f"| {arm} | {c['sessions']} | {c['hung']} | {c['failed']} | {c['invalid']} | {c['deadlock_samples']} |")
    lines += ["", "## What the plays looked like (the state of the editor; not what this experiment decides)", "",
              "| arm | plays | largest wait per play: p50 / p95 / max (ms) | slow gaps > 62.5 ms | > 100 ms | plays with a wait > 100 ms |", "|---|---|---|---|---|---|"]
    for arm in ARMS:
        g = report["play_gaps"][arm]
        lines.append(f"| {arm} | {g['plays']} | {g['max_gap_ms']['p50']} / {g['max_gap_ms']['p95']} / {g['max_gap_ms']['max']} | {g['slow_gt_62_5']} | {g['slow_gt_100']} | "
                     f"{g['plays_with_gap_gt_100']} |")
    for arm in ARMS:
        where = report["play_gaps"][arm]["plays_with_a_slow_gap_at"]
        lines.append(f"- {arm}: plays with a slow gap (> 62.5 ms) at each location: {', '.join(f'{name} {count}' for name, count in where.items()) or 'none'}")
    lines += ["", "## Sessions", "",
              "| # | pair | arm | runtime.js bytes | class | ext median/p95 cores | loadavg start / peak 1m | first play: pictures / max gap ms | repeat play: pictures / max gap ms | hung | valid | deadlock samples |",
              "|---|---|---|---|---|---|---|---|---|---|---|---|"]
    for r in report["rows"]:
        def cell(name):
            play = r["plays"].get(name)
            return "missing" if play is None else f"{play['pictures']} / {play['max_gap_ms']}{' HUNG' if play['hung'] else ''}"
        samples = ", ".join(f"{'DEADLOCK' if s['signature']['deadlock'] else 'other'} at {s['at_s']} s ({s['role']} {s['pid']}: {s['file']})" for s in r["samples"]) or "-"
        lines.append(f"| {r['order_index']} | {r['pair']} | {r['arm']} | {r.get('served_runtime_bytes') or '?'} | {r['quiet_class']} | "
                     f"{r['external_median_cores']}/{r['external_p95_cores']} | "
                     f"{r['loadavg_start']} / {r['loadavg_peak_1m']} | {cell('first')} | {cell('repeat')} | {status_of(r)} | "
                     f"{'yes' if r['valid'] else 'NO: ' + '; '.join(r['invalid_reasons'])} | {samples} |")
    lines += ["", f"Exact one-sided chance of so few hung sessions in the patched arm if both arms were alike (diagnostic only): {report['chance_of_so_few']}", "",
              f"Verdict (mechanical, pre-fixed rules): {report['verdict']}", ""]
    return "\n".join(lines)


# ------------------------------------------------------------------------------------------------ the watchdog
# ------------------------------------------------------------------------------------------------ the watchdog
def process_command(pid):
    return process_table().get(pid, (0, 0, 0.0, ""))[3]


def helper_role(command):
    """The short role of a WebKit helper from its command line (`WebContent`, `Networking`, ...)."""
    return re.sub(r".*com\.apple\.WebKit\.", "", command)[:40]


class Watchdog:
    """Wraps the harness's poll (`collect`): the editor is playing and the frame table has not grown for STALL_SECONDS -> sample the page's process."""

    def __init__(self, out_dir, label, baseline, now=time.monotonic):
        self.out_dir, self.label, self.baseline, self.now = pathlib.Path(out_dir), label, set(baseline), now
        self.started = now()
        self.count, self.changed, self.last_sample = -1, now(), None
        self.samples = []

    def wrap(self, collect):
        def wrapped(lab, state, deadline):
            result = collect(lab, state, deadline)
            try:
                self.observe(lab, len(state["frames"]))
            except Exception as error:  # the watchdog never takes a session down
                self.samples.append({"at_s": round(self.now() - self.started, 1), "error": repr(error), "signature": deadlock_signature("")})
            return result
        return wrapped

    def stalled(self, frames, now):
        """True when the table has not grown for STALL_SECONDS, and at least that long passed since the last sample."""
        if frames != self.count:
            self.count, self.changed = frames, now
            return False
        return now - self.changed >= STALL_SECONDS and (self.last_sample is None or now - self.last_sample >= STALL_SECONDS) \
            and len(self.samples) < MAX_SAMPLES_PER_SESSION

    def observe(self, lab, frames):
        now = self.now()
        if not self.stalled(frames, now):
            return
        if (lab.op({"op": "playback.status"}).get("data") or {}).get("state") != "playing":
            return
        self.last_sample = now
        for pid in sorted(webkit_pids() - self.baseline):
            if "WebContent" not in process_command(pid):
                continue
            target = self.out_dir / f"hang-{self.label}-{pid}-{len(self.samples)}.txt"
            subprocess.run(["/usr/bin/sample", str(pid), "2", "1", "-file", str(target)], capture_output=True, timeout=60)
            text = target.read_text(errors="replace") if target.exists() else ""
            self.samples.append({"at_s": round(now - self.started, 1), "pid": pid, "role": helper_role(process_command(pid)), "file": target.name,
                                 "signature": deadlock_signature(text)})


def leftover_helpers(baseline):
    """WebKit helpers born during the session that are still alive a little after its editor ended: reported (a pid and its role), never killed."""
    time.sleep(3.0)
    return [{"pid": pid, "role": helper_role(process_command(pid))} for pid in sorted(webkit_pids() - set(baseline))]


# ------------------------------------------------------------------------------------------------ commands
def scratch_runtime(parent, arm, bundle, index=None):
    """A folder the host accepts as `TESSEL_RUNTIME_DIR`: `dist/index.html` (the repository's built page unless `index` names another) and this arm's `runtime.js`."""
    dist = pathlib.Path(parent) / arm / "dist"
    dist.mkdir(parents=True)
    shutil.copyfile(index or ROOT / "runtime" / "dist" / "index.html", dist / "index.html")
    shutil.copyfile(bundle, dist / "runtime.js")
    return dist.parent


def bundles_of(args):
    return {UNPATCHED: pathlib.Path(args.unpatched_bundle).resolve(), PATCHED: ROOT / "runtime" / "dist" / "runtime.js"}


def bundle_gate(args):
    """Why the run may not start: a bundle that is not what its arm says (the unpatched one must hold the decoder's call, the patched one not), or None."""
    for arm, bundle in bundles_of(args).items():
        for finding in bundle_findings(arm, bundle.read_text(errors="replace")):
            return finding
    return None


EXPERIMENT = Experiment(record="run-{index:02d}-{arm}.json", build=build_report, markdown=markdown, ok=lambda report: report["all_ran"])


def cmd_run(args):
    bundles = bundles_of(args)
    run = prerequisites(args, EXPERIMENT, needed=(ROOT / "runtime" / "dist" / "index.html", *bundles.values()), gate=lambda: bundle_gate(args))
    if run is None:
        return 2
    W = warm_proof()
    out = pathlib.Path(args.out)
    manifest = manifest_of(run, arm_order(ARMS, args.pairs), runtime_js=None, bundle_sha256={arm: sha256_file(path) for arm, path in bundles.items()},
                           bundle_bytes={arm: path.stat().st_size for arm, path in bundles.items()}, io="async", env=CONTROL_ENV, stress_cpu=args.stress_cpu)
    out.mkdir(parents=True, exist_ok=True)
    scratch = tempfile.TemporaryDirectory(prefix="rb-", dir="/tmp")
    dirs = {arm: scratch_runtime(scratch.name, arm, path) for arm, path in bundles.items()}

    def session(index, arm):
        baseline = webkit_pids()
        watchdog = Watchdog(out, f"{index:02d}-{arm}", baseline)
        record = run_session(W, run.binary, run.document, {"number": index, "mode": "async"},
                             session_env("async", {**CONTROL_ENV, "TESSEL_RUNTIME_DIR": str(dirs[arm])}), whole_project_twice(W, run.project),
                             args.stress_cpu, collect=watchdog.wrap(poll_trace))
        record["hang_samples"] = watchdog.samples
        record["leftover_helpers"] = leftover_helpers(baseline)
        return record

    def summary(record):
        row = session_row(record, run.project)
        return (f"  run {record['order_index']} {record['arm']} hung={row['hung']} failed={row['failed']} valid={row['valid']} class={row['quiet_class']} "
                f"samples={[s['signature']['deadlock'] for s in row['samples']]} leftover={len(row['leftover_helpers'])}")

    try:
        records = run_arms(run, out, manifest, EXPERIMENT, session, summary)
    finally:
        scratch.cleanup()
    return finish(EXPERIMENT, out, records, run.project, manifest, args.report_dir)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    run = sub.add_parser("run")
    run.add_argument("--binary", required=True)
    run.add_argument("--project", required=True)
    run.add_argument("--unpatched-bundle", required=True, help="the runtime.js of a plain `./scripts/runtime build` (no patch variable), copied out of runtime/dist before the patched build")
    run.add_argument("--out", required=True)
    run.add_argument("--pairs", type=int, default=30)
    run.add_argument("--stress-cpu", type=int, default=0,
                     help="busy-loop processes kept alive during each session (the hang needs a loaded machine to show often; both arms get the same)")
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
