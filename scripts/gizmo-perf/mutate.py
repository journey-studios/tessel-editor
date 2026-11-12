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
  memo_off                `overlay_memo_hit` never hits            -> host idle-window test must fail (its layout count
                                                                      of the repeated syncs, or its memo-hit assertions)
  replan_per_tick         `workflow_tick` asks for the overlay     -> host idle-window test must fail

WHAT "KILLED" MEANS. A mutant is killed by a detector only when the run (a) exited non-zero, (b) lists the NAMED test
of that detector as failed, (c) shows that test's own failure message (the layout budget, the memo hit, the time
budget lines...) and (d) for the time detector, the machine guards read AFTER the run were clean and the run was not
INCONCLUSIVE (a run that prints `over budget:` for a corpus it could judge is not: the note of another, unsteady
corpus beside it does not take that failure back, the order `run.py` reads the output in). Everything else is NOT a
kill and is recorded with its own outcome: a compile error, a timeout, a
missing executable or command (exit 127), a full disk, zero tests, no test result, a panic of some other test, a
failure of the right test with another message, a busy lock, a failed guard. A detector that must NOT see a mutant
(`delay_1ms_per_handler` structurally) must run its named tests and see them pass (`SURVIVED`). The accepted messages
stay strict: for `memo_off` they are the three assertions of the host test that a memo that never hits breaks, nothing
looser (a compile error, a timeout and "the journey ran" are still not a kill).

A clean tree is a precondition of BOTH phases, not a courtesy, and it is read from the tree AS IT IS WHEN THE PHASE RUNS.
`prepare` refuses a dirty working tree before it mutates anything (the executables of the mutants would be built from that
tree, not from HEAD, and the verdict would be filed under HEAD). `time` refuses a dirty working tree too: the controlled run
reads `budgets.json` (a path, not an `include_str!`) and the fixtures from the tree at RUN time, and the runners that judge
it are the tree's, so a verdict taken after `budgets.json`, the journey or a runner was edited since `prepare` would not be
HEAD's even though the executables are; `same_head` compares only the commit and the tree objects, and `at_head` only the
files the mutants edit, so neither sees such an edit. And `time` refuses a `prepare` whose manifest records a non-empty
`identity.dirty` or none at all (the tree reading clean now does not make the executables HEAD's). Both checks are made, the
current tree first; neither can be skipped (there is no `--allow-dirty` here). An identity git could not answer
(`unavailable: ...`) and a file git cannot compare with HEAD are refusals as well, never two equal strings.

Time runs use the controlled release run (`scene_gizmo_perf_budget`, judged against `budgets.json`) under the
performance lock, taken once for all of them and never waited for (BLOCKED, exit 3). Every child has a finite timeout.
The mutated source is restored in a `finally` (also on SIGTERM/SIGINT, which become `SystemExit`) and the file hash is
compared with `git`'s after every mutant; a mismatch aborts loudly. The restore, with the three hashes (committed,
mutated, restored), is logged in the manifest. The `time` phase refuses to run for another HEAD/tree than the
`prepare` that built the executables and writes ONE merged manifest (`manifest.final.json`): its refusals too, never
over `prepare`'s `manifest.json`. It also refuses a `prepare` that is not finished and any mutant `prepare` did not run
(it is never silently dropped), and a verdict over no mutant at all is never a pass. It measures an executable only
when `prepare` recorded the hash of that very file (else `MISSING_EXECUTABLE`), and `prepare` begins by emptying
`--out/exes`, so no executable of an earlier `prepare` can stand in for one this run did not build. A `time` run in which
ANY evaluated timed mutant ended with no answer (`NOT_AN_ANSWER`: blocked, no usable executable, a guard that failed after
the run, an infrastructure error, inconclusive...) keeps every executable, so the same `prepare` can be measured again. A
run that answered for everything it evaluated removes the executables of THOSE mutants only (an `--only` leaves the
others for a later run), and a run that evaluated no timed mutant removes nothing.

One run per `--out` at a time, for BOTH phases AND for every runner: right after creating `--out` (before anything in it is
read, written or removed, a refusal's own manifest included) each takes an exclusive, non-blocking `flock` on
`--out/harness.OUT_LOCK` (`.gizmo-perf.lock`, the ONE name `run.py`, `ab.py`, `native.py` and this runner share) and keeps it
until `main` returns. A run that finds it taken, by this runner or by any other, prints one `REFUSED` JSON line, exits 1 and
writes nothing into `--out`: the holder owns `manifest.json`, `manifest.final.json` and `exes/` (and whatever else its runner
keeps there). Without it a `prepare` (a build, which takes no performance lock) could replace or empty `exes/` after `time`
hashed an executable and before it ran it, or while `run.py --phase measure` was running from that directory, and the result
would be filed under another run's manifest; a second hash check cannot close that window. The lock is taken before the
performance lock and neither is waited for, so two runs cannot wait on each other. It is a `flock`, so a run that dies
(SIGKILL included) never leaves `--out` locked; the lock file itself stays, on purpose (see `harness.exclusive`). The lock
excludes SIMULTANEOUS runs, not SEQUENTIAL ones: a different runner started after this one ended still overwrites its
`manifest.json` and `exes/`, so one `--out` per runner stays the rule. `main` takes the lock once and `build()` of `run.py`,
which `prepare` calls, is a plain function that takes none, so the shared name cannot conflict with itself. `prepare` BUILDS
(and edits the checkout's own source files), so before the lock of `--out` it also takes the per-CHECKOUT build lock of
`harness.guarded` (one lock per checkout, in `/tmp`, shared with the builds of `run.py`, `ab.py` and `native.py`): two builds
of one checkout, from different `--out`, would otherwise edit and compile the same tree at once.

Exit 0 only when every mutant was detected by every detector that must detect it and passed every one that must not;
1 when a mutant is not as expected; 3 when anything was BLOCKED or infrastructure failed (never a pass).
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
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
    # The three assertions of the host test that a memo that never hits breaks, in the order the test checks them: with the
    # memo off the FIRST to fail is the layout count of the repeated syncs (a sync that is not a memo hit lays the scene
    # out; `scene_gizmo_perf_tests.rs`, the `end.0 - start.0 <= SYNCS` assertion), which the first real run (M1 of
    # `measure-12bc8da`) hit and the runner did not accept as a kill. "the journey ran" (the fixture itself failed), a
    # compile error and a timeout are not in this list, so they are still not a kill.
    ('memo_off', 'host'): {
        'tests': ['an_idle_window_plans_nothing_per_tick_and_the_overlay_stays_memoised'],
        'messages': [r'a sync of an unchanged state laid the scene out more than once',
                     r'the overlay was asked for again', r'every sync of an unchanged state is a memo hit']},
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
    """The blob id `HEAD` holds for `path`, or `None` when git cannot say. Never a failure text: two failed reads used to
    be two equal strings (`harness.out` printed an empty line or `unavailable: ...` for each), and "the file is at HEAD"
    was then decided by `'' == ''`."""
    ok, text = harness.git_strict(['rev-parse', f'HEAD:{path}'])
    return text if ok and text else None


def file_hash(path):
    """The blob id of the file `path` as it is now, or `None` when git cannot say (see `committed_hash`)."""
    ok, text = harness.git_strict(['hash-object', path])
    return text if ok and text else None


def at_head(path):
    """True only when git answered BOTH questions and the file is byte for byte `HEAD`'s: no answer is never a match."""
    now, committed = file_hash(path), committed_hash(path)
    return now is not None and now == committed


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
        try:
            self.path.write_text(text.replace(self.mutant['old'], self.mutant['new']))
            self.log.update(committed_blob=committed_hash(self.mutant['file']),
                            mutated_blob=file_hash(self.mutant['file']))
        except BaseException:
            # `__exit__` does not run when `__enter__` raises: whatever was written is put back here, before anything
            # propagates (git unreadable, a full disk, SIGTERM between the write and the return).
            self.path.write_bytes(self.original)
            raise
        return self

    def __exit__(self, *exc):
        self.path.write_bytes(self.original)
        restored = file_hash(self.mutant['file'])
        committed = committed_hash(self.mutant['file'])
        # A git that cannot answer proves nothing: the file was put back, but "restored" is only recorded as true when both
        # hashes exist and agree (the run then stops loudly, below, instead of trusting two missing answers).
        self.log.update(restored_blob=restored, restored=restored is not None and restored == committed,
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
        if guard_after is not None:
            # A pass read on a machine the guard refused after the run says nothing, exactly as a kill does (below):
            # it is not an answer, so the executables are kept for a retry instead of an `UNEXPECTED` verdict.
            return 'INVALID_GUARD_AFTER_RUN', guard_after
        return 'SURVIVED', 'the named tests ran and passed'
    if 'INCONCLUSIVE:' in text and 'over budget:' not in text:
        # `over budget:` is printed only for failures of corpora the reference let be judged (an unsteady corpus adds no
        # failure of its own), so a proven failure is classified below by the named test and its own lines.
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
    # `cargo test` builds the mutated tree: no `TESSEL_GIZMO_PERF_*` (an exported `..._BLESS=1` makes the structural test
    # rewrite its golden file and pass without asserting) and no target directory of the caller's reach it.
    done = harness.run_bounded(command, env=harness.child_env(build=True), timeout=harness.TEST_TIMEOUT_S)
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
    if not flat:
        # `all([])` is True: with no verdict at all "every detector answered as expected" would be vacuously true.
        return 'NO_DETECTOR_EVALUATED', 1
    if any(not v['answer'] for v in flat):
        return 'BLOCKED_OR_INFRA', harness.BLOCKED_EXIT
    if all(v['as_expected'] for v in flat):
        return 'ALL_AS_EXPECTED', 0
    return 'UNEXPECTED', 1


def run_time(exe, report, out, name):
    """One controlled run of a mutant's executable. Returns the record (never raises for the child)."""
    Path(report).unlink(missing_ok=True)
    # What THIS runner sets, and only that: no inherited `TESSEL_GIZMO_PERF_*` (a `..._CALIBRATE=1` in the caller's
    # environment would run the time detector without asserting a budget, and it could then never be killed).
    set_here = {'TESSEL_GIZMO_PERF_OUT': str(report)}
    env = harness.child_env(set_here)
    done = harness.run_bounded([str(exe), '--ignored', '--test-threads=1', '--nocapture', TIME_TEST], cwd=ROOT,
                               env=env, timeout=harness.MEASURE_TIMEOUT_S)
    harness.write_text(out / f'{name}.time.stdout.txt', done.stdout)  # redacted (`harness.redact`)
    harness.write_text(out / f'{name}.time.stderr.txt', done.stderr[-20000:])
    scratch = {}
    after = harness.judge_after(scratch)
    outcome, detail = classify(done, EXPECT[(name, 'time')], guard_after=after)
    failures = [l.strip() for l in done.text.splitlines() if any(re.search(m, l) for m in BUDGET_LINES)]
    return {'outcome': outcome, 'detail': detail, 'status': outcome, **done.record(),
            'env': set_here, 'env_dropped': harness.dropped_env(),
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
    # The checkout (`prepare` only: it builds from the checkout with a mutant IN its tree) and then the directory are
    # owned before anything in either is touched, and until this returns (the `with` is the `finally`: also on
    # SIGTERM/SIGINT, which are `SystemExit`). The locks never wait, the checkout's first, so no run can wait on another.
    # The lock of the directory is the one every runner shares (`harness.OUT_LOCK`).
    with harness.guarded(out, 'mutate.py', builds=args.phase == 'prepare') as refusal:
        if refusal:
            return harness.die(1, refusal, None)
        return run_phase(args, out)


def run_phase(args, out):
    """The phase `args.phase` over `out`, which the caller owns (`out/harness.OUT_LOCK` is held). Returns the exit code."""
    names = args.only or list(MUTANTS)
    manifest = {'suite': 'gizmo-perf-mutants', 'phase': args.phase, 'utc_start': harness.utc(), 'command': sys.argv,
                'identity': harness.git_identity(), 'toolchain': harness.toolchain(), 'mutants': {},
                'status': 'ABORTED'}
    # `manifest.json` is the `prepare` phase's (what `time` reads); the `time` phase writes only `manifest.final.json`, so a
    # refusal of a later `time` run can never replace a finished `prepare` with a manifest that has no mutants.
    refusal_path = out / ('manifest.json' if args.phase == 'prepare' else 'manifest.final.json')
    problem = harness.identity_problem(manifest['identity'])
    if problem:  # a checkout git cannot name: nothing is mutated or measured for it (there is no `--allow-dirty` here)
        manifest.update(status='REFUSED', reason=problem)
        return harness.die(1, manifest, refusal_path)
    if manifest['identity']['dirty']:
        # BOTH phases read the tree as it is NOW (the manifest check of the `time` phase, further down, reads what `prepare`
        # recorded: a different question). `dirty` is the first line of `git status --porcelain` over the build inputs and
        # the files a verdict reads (`harness.DIRTY_PATHSPECS`).
        first = manifest['identity']['dirty'].splitlines()[0][:120]
        if args.phase == 'prepare':
            # Before any mutation. The mutant is built from the whole checkout, so a dirty tree elsewhere (a changed runner,
            # a changed crate) would be built into the executables and the verdict filed under a HEAD it does not describe.
            reason = ('the working tree is dirty: the executables of the mutants would be built from it, not from HEAD, and '
                      f'their verdict would be filed under HEAD ({first})')
        else:
            # Before anything is measured or removed. The executables may be HEAD's (the manifest of `prepare` says so),
            # but the controlled run reads `budgets.json` and the fixtures from THIS tree when it runs, and the runners
            # that judge it are this tree's: an edit made since `prepare` changes the verdict, and nothing else notices.
            reason = ('the working tree is dirty: the time verdict reads `budgets.json` and the fixtures from the tree when '
                      'it runs, and the runners are the tree\'s, so a verdict taken now would not be HEAD\'s even if the '
                      f'executables are (clean the tree and run `time` again; {first})')
        manifest.update(status='REFUSED', reason=reason)
        return harness.die(1, manifest, refusal_path)
    for name in names:
        path = MUTANTS[name]['file']
        if not at_head(path):
            manifest.update(status='REFUSED', reason=f'{path} is not at HEAD (or git cannot say): mutants start from a '
                                                     'committed tree')
            return harness.die(1, manifest, refusal_path)
    exes_dir = out / 'exes'
    if args.phase == 'prepare' and exes_dir.exists():
        # A `prepare` answers for its own HEAD. An executable an earlier `prepare` left here (another HEAD, another run)
        # must not be around to stand in for a mutant whose build in THIS run failed, timed out or named no executable:
        # that run records no hash and copies nothing. Not `ignore_errors`: a directory that cannot be emptied stops it.
        shutil.rmtree(exes_dir)
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
                        record['time_build'] = {'command': command, **done.record(), 'executable_sha256': None,
                                                'env_dropped': harness.dropped_env(build=True)}
                        try:
                            if exe:  # the hash is of the file the build named; `time` measures only a copy with that hash
                                record['time_build']['executable_sha256'] = harness.sha256(exe)
                            if exe and done.returncode == 0:
                                shutil.copy(exe, exes_dir / f'{name}.exe')
                        except OSError as error:
                            # The path cargo printed can be gone (a managed target was pruned): no hash, no copy, and
                            # `time` then says MISSING_EXECUTABLE for this mutant, not a traceback here.
                            record['time_build'].update(executable_sha256=None, copy_error=f'{exe}: {error}')
                            (exes_dir / f'{name}.exe').unlink(missing_ok=True)
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
    # `same_head` also refuses a `prepare` manifest whose identity names no usable head/tree (two empty answers are not
    # "the same HEAD").
    if not harness.same_head(previous['identity'], manifest['identity']):
        manifest.update(status='REFUSED', reason='`prepare` ran for another HEAD/tree (or recorded none): its '
                                                 'executables are not these')
        return harness.die(1, manifest, out / 'manifest.final.json')
    manifest['prepare_identity'] = previous['identity']
    if previous['identity'].get('dirty', 'not recorded') != '':
        # An empty string is the only clean record: a `prepare` that ran on a dirty tree, or whose manifest does not say it
        # did not, built executables that are not HEAD's, and the tree reading clean now does not change what they are.
        manifest.update(status='REFUSED', reason='`prepare` ran on a dirty working tree (or its manifest does not say '
                                                 'it was clean): its executables are not HEAD\'s, so their time verdict '
                                                 'cannot be filed under HEAD')
        return harness.die(1, manifest, out / 'manifest.final.json')
    if previous.get('status') != 'PREPARED':
        manifest.update(status='REFUSED', reason=f"the manifest in --out is not a finished `prepare` "
                                                 f"(status {previous.get('status')!r})")
        return harness.die(1, manifest, out / 'manifest.final.json')
    absent = [name for name in names if name not in previous['mutants']]
    if absent:
        # Filtering `previous` by `names` would leave no mutant at all and every detector "as expected" by default.
        manifest.update(status='REFUSED', reason=f'`prepare` did not run {absent}: nothing was evaluated for them '
                                                 '(run `--phase prepare` for those mutants first)')
        return harness.die(1, manifest, out / 'manifest.final.json')
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
        elif not recorded or harness.sha256(exe) != recorded:
            # Only the file `prepare` hashed is measured. No recorded hash (its build gave no executable) is not
            # "nothing to compare": the file there was left by something else, and its verdict would be this HEAD's.
            manifest['mutants'][name]['time'] = {'outcome': 'MISSING_EXECUTABLE', 'status': 'MISSING_EXECUTABLE',
                                                 'detail': 'no hash of this executable was recorded by `prepare`, or '
                                                           'it changed since `prepare` built it: not the one built'}
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
    # The executables are discardable (20 MB each) only once they have said what they had to say. A run in which ANY
    # evaluated mutant ended with no answer (blocked, no executable, a guard that failed after the run, an infrastructure
    # error, inconclusive...) keeps every executable, so the same `prepare` can be measured again. A run that answered for
    # all it evaluated removes the executables of THOSE mutants only: the ones of mutants an `--only` left out are the
    # business of a later run, and a run that evaluated no timed mutant removes nothing.
    evaluated = {name: record.get('time', {}).get('outcome') for name, record in manifest['mutants'].items()
                 if 'time' in MUTANTS[name]['detectors']}
    if not any(outcome is None or outcome in NOT_AN_ANSWER for outcome in evaluated.values()):
        for name in evaluated:
            (exes_dir / f'{name}.exe').unlink(missing_ok=True)
        if exes_dir.is_dir() and not any(exes_dir.iterdir()):
            exes_dir.rmdir()
    status, code = overall(manifest)
    manifest.update(status=status, exit=code, utc_end=harness.utc())
    harness.write(out / 'manifest.final.json', manifest)
    print(json.dumps({'status': status, 'mutants': {n: r['verdict'] for n, r in manifest['mutants'].items()}},
                     indent=1))
    return code


if __name__ == '__main__':
    raise SystemExit(main())
