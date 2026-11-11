#!/usr/bin/env python3
"""Mutation proof of the gizmo performance suite (F055 / BL015.21).

    python3 scripts/gizmo-perf/mutate.py --phase prepare [--out DIR] [--only NAME ...]
    python3 scripts/gizmo-perf/mutate.py --phase time    [--out DIR] [--only NAME ...]

A suite that cannot fail proves nothing. Each mutant below puts one real performance defect into the PRODUCT code
(not into the measuring code: that would be the artificial injection of the negative controls inside
`scene_gizmo_perf_budget`, which stays a control and is never counted here), runs the detectors that must catch it,
restores the file and checks its hash:

  extra_plan_per_handler  every gizmo plan is built twice          -> structural (core) AND time must fail
  delay_1ms_per_handler   `apply_gizmo` sleeps 1 ms               -> structural must PASS (no extra layout: the
                                                                      counter cannot see a delay), time must FAIL
  memo_off                `overlay_memo_hit` never hits            -> host idle-window test must fail
  replan_per_tick         `workflow_tick` asks for the overlay     -> host idle-window test must fail

WHAT "KILLED" MEANS. A mutant is killed by a detector only when the run (a) exited non-zero, (b) lists the NAMED test
of that detector as failed, (c) shows that test's own failure message (the layout budget, the memo hit, the time
budget lines...) and (d) for the time detector, the machine guards read AFTER the run were clean and the run was not
INCONCLUSIVE. Everything else is NOT a kill and is recorded with its own outcome: a compile error, a timeout, a
missing executable or command (exit 127), a full disk, zero tests, no test result, a panic of some other test, a
failure of the right test with another message, a busy lock, a failed guard. A detector that must NOT see a mutant
(`delay_1ms_per_handler` structurally) must run its named tests and see them pass (`SURVIVED`).

Time runs use the controlled release run (`scene_gizmo_perf_budget`, judged against `budgets.json`) under the
performance lock, taken once for all of them and never waited for (BLOCKED, exit 3). Every child has a finite timeout.
The mutated source is restored in a `finally` (also on SIGTERM/SIGINT, which become `SystemExit`) and the file hash is
compared with `git`'s after every mutant; a mismatch aborts loudly. The restore, with the three hashes (committed,
mutated, restored), is logged in the manifest. The `time` phase refuses to run for another HEAD/tree than the
`prepare` that built the executables and writes ONE merged manifest (`manifest.final.json`).

Exit 0 only when every mutant was detected by every detector that must detect it and passed every one that must not;
1 when a mutant is not as expected; 3 when anything was BLOCKED or infrastructure failed (never a pass).
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import harness  # noqa: E402
import run as runner  # noqa: E402

ROOT = harness.ROOT
GIZMO = 'crates/core/src/scene_project/gizmo.rs'
OVERLAY = 'crates/desktop/src/workstation/viewport_overlay.rs'
WORKFLOW = 'crates/desktop/src/workstation/workflow.rs'
STRUCTURE = ['./scripts/cargo', 'test', '--locked', '-p', 'tessel-editor-core', '--test', 'scene_gizmo_perf_structure']
HOST = ['./scripts/cargo', 'test', '--locked', '-p', 'tessel-editor', 'scene_gizmo_perf_tests::an_idle']
TIME_TEST = 'the_gizmo_handlers_stay_within_their_budget_cold_and_hot_on_every_corpus'
BUDGET_LINES = [r'x the plan \(budget', r'samples over a 60 Hz frame']

REPLAN_TICK = '''pub(super) fn workflow_tick(&mut self, cx: &mut Context<Self>) {
        if let Some(scene) = self.viewport.directed_scene().map(str::to_owned) {
            let frame = self.state.timeline.playhead;
            let _ = self.state.execute(crate::state::Command::Scene(
                tessel_editor_core::scene_project::SceneCommand::Gizmo(
                    tessel_editor_core::scene_interaction::GizmoCommand::State {
                        scene,
                        at: tessel_editor_core::scene_interaction::FrameAt::Frame(frame),
                    },
                ),
            ));
        }'''

MUTANTS = {
    'extra_plan_per_handler': {
        'file': GIZMO,
        'old': 'fn plan_of(scene: &DirectedScene, frame: i64) -> Result<ScenePlan, CommandError> {\n',
        'new': 'fn plan_of(scene: &DirectedScene, frame: i64) -> Result<ScenePlan, CommandError> {\n'
               '    let _ = ScenePlan::build_with(scene, frame, PlanOptions { allow_partial: true });\n',
        'detectors': {'structural': True, 'time': True},
    },
    'delay_1ms_per_handler': {
        'file': GIZMO,
        'old': 'pub(super) fn apply_gizmo(&mut self, command: GizmoCommand) -> Result<Value, CommandError> {\n',
        'new': 'pub(super) fn apply_gizmo(&mut self, command: GizmoCommand) -> Result<Value, CommandError> {\n'
               '        std::thread::sleep(std::time::Duration::from_millis(1));\n',
        'detectors': {'structural': False, 'time': True},
    },
    'memo_off': {
        'file': OVERLAY,
        'old': 'let hit = self.overlay.enabled\n',
        'new': 'let hit = false\n            && self.overlay.enabled\n',
        'detectors': {'host': True},
    },
    'replan_per_tick': {
        'file': WORKFLOW,
        'old': 'pub(super) fn workflow_tick(&mut self, cx: &mut Context<Self>) {',
        'new': REPLAN_TICK,
        'detectors': {'host': True},
    },
}

# What each detector must show for a kill (named test + that test's own message) and, for a detector that must
# not see the mutant, the tests that must have run and passed. Declared here, before any mutant is run.
EXPECT = {
    ('extra_plan_per_handler', 'structural'): {
        'tests': ['every_request_plans_the_scene_at_most_as_often_as_its_contract_says_on_every_corpus'],
        'messages': [r'laid the scene out \d+ times \(budget \d+\)']},
    ('extra_plan_per_handler', 'time'): {
        'tests': [TIME_TEST], 'messages': BUDGET_LINES, 'min_lines': 1},
    ('delay_1ms_per_handler', 'time'): {
        'tests': [TIME_TEST], 'messages': BUDGET_LINES, 'min_lines': 3},
    ('memo_off', 'host'): {
        'tests': ['an_idle_window_plans_nothing_per_tick_and_the_overlay_stays_memoised'],
        'messages': [r'the overlay was asked for again', r'every sync of an unchanged state is a memo hit']},
    ('replan_per_tick', 'host'): {
        'tests': ['an_idle_window_plans_nothing_per_tick_and_the_overlay_stays_memoised'],
        'messages': [r'idle ticks laid the scene out']},
}
MUST_PASS = {
    ('delay_1ms_per_handler', 'structural'): [
        'every_request_plans_the_scene_at_most_as_often_as_its_contract_says_on_every_corpus',
        'a_long_drag_costs_one_layout_per_move_and_nothing_else_grows'],
}
# Outcomes that are NOT an answer of the suite: the run did not happen or did not say anything about the mutant.
NOT_AN_ANSWER = {'INFRA_TIMEOUT', 'INFRA_ERROR', 'INFRA_DISK', 'COMPILE_ERROR', 'NO_TEST_RESULT', 'ZERO_TESTS',
                 'BLOCKED', 'INCONCLUSIVE', 'INVALID_GUARD_AFTER_RUN', 'MISSING_EXECUTABLE', 'MISSING'}

FAILED_LINE = re.compile(r'^test (\S+) \.\.\. FAILED', re.M)
OK_LINE = re.compile(r'^test (\S+) \.\.\. ok', re.M)
RESULT_LINE = re.compile(r'test result: (ok|FAILED)\. (\d+) passed; (\d+) failed')


def committed_hash(path):
    return harness.out(['git', 'rev-parse', f'HEAD:{path}'])


def file_hash(path):
    return harness.out(['git', 'hash-object', path])


class AnchorNotUnique(SystemExit):
    pass


class Mutated:
    """Applies a mutant and ALWAYS puts the file back (and proves it did); `log` keeps the three hashes."""

    def __init__(self, mutant, log=None):
        self.mutant = mutant
        self.path = ROOT / mutant['file']
        self.log = log if log is not None else {}

    def __enter__(self):
        self.original = self.path.read_bytes()
        text = self.original.decode()
        if text.count(self.mutant['old']) != 1:
            raise AnchorNotUnique(f"the anchor of the mutant is not unique in {self.mutant['file']}")
        self.path.write_text(text.replace(self.mutant['old'], self.mutant['new']))
        self.log.update(committed_blob=committed_hash(self.mutant['file']), mutated_blob=file_hash(self.mutant['file']))
        return self

    def __exit__(self, *exc):
        self.path.write_bytes(self.original)
        restored = file_hash(self.mutant['file'])
        self.log.update(restored_blob=restored, restored=restored == committed_hash(self.mutant['file']),
                        interrupted_by=repr(exc[1]) if exc[1] is not None else None)
        if not self.log['restored']:
            raise SystemExit(f"{self.mutant['file']} was NOT restored to HEAD: stop and inspect")


def failed_names(text):
    """The tests a cargo test run reports as failed (the `FAILED` lines and the final `failures:` list; with
    `--nocapture` a test's own output can sit between `...` and its verdict)."""
    names = set(FAILED_LINE.findall(text))
    if 'failures:' in text:
        tail = text.rsplit('failures:', 1)[1]
        for line in tail.splitlines():
            if re.fullmatch(r'    \S+', line):
                names.add(line.strip())
            elif line.startswith('test result:'):
                break
    return names


def classify(done, expect=None, must_pass=(), guard_after=None):
    """`(outcome, detail)` of one detector run. `KILLED` only for the named test failing with its own message;
    `SURVIVED` only for a clean pass in which the named tests ran. See the module docstring."""
    text = done.text
    if done.timed_out:
        return 'INFRA_TIMEOUT', f'no answer within the finite timeout ({done.seconds:.0f}s)'
    if done.error or done.returncode == 127:
        return 'INFRA_ERROR', done.error or 'exit 127: the command was not found'
    if 'No space left on device' in text:
        return 'INFRA_DISK', 'the disk is full'
    if 'error[E' in text or 'could not compile' in text or 'error: linking' in text:
        return 'COMPILE_ERROR', 'the mutated tree did not compile: a mutant that does not build was not detected'
    results = RESULT_LINE.findall(text)
    if not results:
        return 'NO_TEST_RESULT', f'no `test result:` line (exit {done.returncode})'
    if sum(int(p) + int(f) for _, p, f in results) == 0:
        return 'ZERO_TESTS', 'the filter matched no test'
    failed = failed_names(text)
    if done.returncode == 0 and not failed:
        oks = set(OK_LINE.findall(text))
        missing = [name for name in must_pass if not any(name in ok for ok in oks)]
        if missing:
            return 'ZERO_TESTS', f'the named tests did not run and pass: {missing}'
        return 'SURVIVED', 'the named tests ran and passed'
    if 'INCONCLUSIVE:' in text:
        return 'INCONCLUSIVE', 'the plan reference moved: neither killed nor survived'
    if guard_after is not None:
        return 'INVALID_GUARD_AFTER_RUN', guard_after
    if expect is None:
        return 'UNEXPECTED_FAILURE', f'a detector that must not see the mutant failed: {sorted(failed)[:4]}'
    missing = [name for name in expect['tests'] if not any(name in f for f in failed)]
    if missing:
        return 'UNEXPECTED_FAILURE', f'the named test(s) {missing} did not fail; failed: {sorted(failed)[:4]}'
    lines = [line.strip() for line in text.splitlines() if any(re.search(m, line) for m in expect['messages'])]
    if len(lines) < expect.get('min_lines', 1):
        return 'UNEXPECTED_FAILURE', (f"the named test failed without its own message "
                                      f"({len(lines)} of {expect.get('min_lines', 1)} expected lines)")
    return 'KILLED', f'{len(lines)} expected failure line(s): {lines[:3]}'


def detect(command, expect=None, must_pass=()):
    done = harness.run_bounded(command, timeout=harness.TEST_TIMEOUT_S)
    outcome, detail = classify(done, expect, must_pass)
    text = done.text
    return {'command': command, **done.record(), 'outcome': outcome, 'detail': detail,
            'detected': outcome == 'KILLED', 'failed_tests': sorted(failed_names(text))[:8],
            'panic': [l for l in text.splitlines() if 'panicked' in l or l.startswith('  ')][:6],
            'log_sha256': hashlib.sha256(text.encode()).hexdigest()}


def verdict_of(name, record):
    """{detector: {must_detect, outcome, as_expected}} for one mutant and the ways it can be not-an-answer."""
    out = {}
    for detector, must in record['must'].items():
        outcome = record.get(detector, {}).get('outcome', 'MISSING')
        wanted = 'KILLED' if must else 'SURVIVED'
        out[detector] = {'must_detect': must, 'outcome': outcome, 'as_expected': outcome == wanted,
                         'answer': outcome not in NOT_AN_ANSWER}
    return out


def overall(manifest):
    """(status, exit code) of the merged manifest."""
    verdicts = {name: verdict_of(name, record) for name, record in manifest['mutants'].items()}
    for name, record in manifest['mutants'].items():
        record['verdict'] = verdicts[name]
        if record.get('restore') and not record['restore'].get('restored', True):
            return 'NOT_RESTORED', 1
    flat = [v for per in verdicts.values() for v in per.values()]
    if any(not v['answer'] for v in flat):
        return 'BLOCKED_OR_INFRA', harness.BLOCKED_EXIT
    if all(v['as_expected'] for v in flat):
        return 'ALL_AS_EXPECTED', 0
    return 'UNEXPECTED', 1


def run_time(exe, report, out, name):
    """One controlled run of a mutant's executable. Returns the record (never raises for the child)."""
    Path(report).unlink(missing_ok=True)
    env = {**os.environ, 'TESSEL_GIZMO_PERF_OUT': str(report)}
    done = harness.run_bounded([str(exe), '--ignored', '--test-threads=1', '--nocapture', TIME_TEST], cwd=ROOT,
                               env=env, timeout=harness.MEASURE_TIMEOUT_S)
    (out / f'{name}.time.stdout.txt').write_text(done.stdout)
    (out / f'{name}.time.stderr.txt').write_text(done.stderr[-20000:])
    scratch = {}
    after = harness.judge_after(scratch)
    outcome, detail = classify(done, EXPECT[(name, 'time')], guard_after=after)
    failures = [l.strip() for l in done.text.splitlines() if any(re.search(m, l) for m in BUDGET_LINES)]
    return {'outcome': outcome, 'detail': detail, 'status': outcome, **done.record(),
            'failures_seen': failures[:24], 'failure_lines': len(failures), 'guard_after': scratch['guard_after'],
            'load_after': scratch['machine_after']['loadavg'],
            'stdout_sha256': harness.sha256(out / f'{name}.time.stdout.txt')}


def main(argv=None):
    harness.install_termination_handlers()
    parser = argparse.ArgumentParser()
    parser.add_argument('--out', default='/tmp/gizmo-perf-mutants')
    parser.add_argument('--only', nargs='*')
    parser.add_argument('--phase', choices=['prepare', 'time'], required=True,
                        help='`prepare`: structural/host detectors and the release builds of the time mutants '
                             '(the builds leave the machine loaded, which the guard refuses); `time`: only the guard, '
                             'the lock and the controlled runs, from the executables `prepare` left in --out/exes '
                             '(no build, no waiting)')
    args = parser.parse_args(argv)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    names = args.only or list(MUTANTS)
    manifest = {'suite': 'gizmo-perf-mutants', 'phase': args.phase, 'utc_start': harness.utc(), 'command': sys.argv,
                'identity': harness.git_identity(), 'toolchain': harness.toolchain(), 'mutants': {},
                'status': 'ABORTED'}
    for name in names:
        path = MUTANTS[name]['file']
        if file_hash(path) != committed_hash(path):
            manifest.update(status='REFUSED', reason=f'{path} is not at HEAD: mutants start from a committed tree')
            return harness.die(1, manifest, out / 'manifest.json')
    exes_dir = out / 'exes'
    exes_dir.mkdir(exist_ok=True)
    if args.phase == 'prepare':
        manifest_path = out / 'manifest.json'
        try:
            for name in names:
                mutant = MUTANTS[name]
                record = manifest['mutants'][name] = {'file': mutant['file'], 'old': mutant['old'],
                                                     'new': mutant['new'], 'must': mutant['detectors'],
                                                     'expect': {d: EXPECT.get((name, d)) or MUST_PASS.get((name, d))
                                                                for d in mutant['detectors']},
                                                     'restore': {}}
                with Mutated(mutant, record['restore']):
                    if 'structural' in mutant['detectors']:
                        record['structural'] = detect(STRUCTURE, EXPECT.get((name, 'structural')),
                                                      MUST_PASS.get((name, 'structural'), ()))
                    if 'host' in mutant['detectors']:
                        record['host'] = detect(HOST, EXPECT.get((name, 'host')), MUST_PASS.get((name, 'host'), ()))
                    if 'time' in mutant['detectors']:
                        command, done, exe = runner.build()
                        record['time_build'] = {'command': command, **done.record(),
                                                'executable_sha256': harness.sha256(exe) if exe else None}
                        if exe and done.returncode == 0:
                            shutil.copy(exe, exes_dir / f'{name}.exe')
            too_big = harness.artifact_reason(exes_dir)
            if too_big:
                manifest.update(status='REFUSED', reason=too_big)
                return harness.die(1, manifest, manifest_path)
        finally:
            manifest['utc_end'] = harness.utc()
        manifest['status'] = 'PREPARED'
        harness.write(manifest_path, manifest)
        print(json.dumps({'status': 'PREPARED', 'structural_and_host': {
            n: {d: r[d]['outcome'] for d in ('structural', 'host') if d in r} for n, r in manifest['mutants'].items()}}))
        return 0
    # --- time phase: no build, one guard, the lock once, the controlled runs.
    previous_path = out / 'manifest.json'
    if not previous_path.exists():
        manifest.update(status='REFUSED', reason='no `prepare` manifest in --out')
        return harness.die(1, manifest, out / 'manifest.final.json')
    previous = json.loads(previous_path.read_text())
    if previous['identity']['head'] != manifest['identity']['head'] or \
            previous['identity']['tree'] != manifest['identity']['tree']:
        manifest.update(status='REFUSED', reason='`prepare` ran for another HEAD/tree: its executables are not these')
        return harness.die(1, manifest, out / 'manifest.final.json')
    manifest['prepare_identity'] = previous['identity']
    manifest['mutants'] = {name: record for name, record in previous['mutants'].items() if name in names}
    state, reason = harness.guards()
    manifest['machine_before'] = state
    manifest['limits'] = harness.claims(state)
    timed = [name for name in manifest['mutants'] if 'time' in MUTANTS[name]['detectors']]
    for name in timed:
        exe = exes_dir / f'{name}.exe'
        recorded = manifest['mutants'][name].get('time_build', {}).get('executable_sha256')
        if not exe.exists():
            manifest['mutants'][name]['time'] = {'outcome': 'MISSING_EXECUTABLE', 'status': 'MISSING_EXECUTABLE',
                                                 'detail': f'{exe} does not exist: run `--phase prepare`'}
        elif recorded and harness.sha256(exe) != recorded:
            manifest['mutants'][name]['time'] = {'outcome': 'MISSING_EXECUTABLE', 'status': 'MISSING_EXECUTABLE',
                                                 'detail': 'the executable changed since `prepare` built it'}
    pending = [n for n in timed if 'time' not in manifest['mutants'][n]]
    if pending and reason:
        for name in pending:
            manifest['mutants'][name]['time'] = {'outcome': 'BLOCKED', 'status': 'BLOCKED', 'detail': reason}
    elif pending:
        with harness.Lock() as lock:
            for name in pending:
                if not lock.held:
                    manifest['mutants'][name]['time'] = {'outcome': 'BLOCKED', 'status': 'BLOCKED',
                                                         'detail': 'performance lock busy (not waited for)'}
                    continue
                manifest['mutants'][name]['time'] = run_time(exes_dir / f'{name}.exe', out / f'{name}.report.json',
                                                             out, name)
    manifest['machine_after'] = harness.machine()
    if not any(r.get('time', {}).get('outcome') in ('BLOCKED', 'MISSING_EXECUTABLE')
               for r in manifest['mutants'].values()):
        shutil.rmtree(exes_dir, ignore_errors=True)  # discardable, 20 MB each; kept when nothing ran
    status, code = overall(manifest)
    manifest.update(status=status, exit=code, utc_end=harness.utc())
    harness.write(out / 'manifest.final.json', manifest)
    print(json.dumps({'status': status, 'mutants': {n: r['verdict'] for n, r in manifest['mutants'].items()}},
                     indent=1))
    return code


if __name__ == '__main__':
    raise SystemExit(main())
