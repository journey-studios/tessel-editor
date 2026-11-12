#!/usr/bin/env python3
"""The time half of the gizmo performance suite, controlled (F055 / BL015.21).

    python3 scripts/gizmo-perf/run.py --phase build   [--out DIR]
    python3 scripts/gizmo-perf/run.py --phase measure [--out DIR] [--calibrate] [--exe PATH --label baseline]

Two phases in two invocations, because a release build leaves the machine loaded and the measurement guard (correctly)
refuses a loaded machine: a runner that built and then measured in one process would block itself. There is no
cooldown, no sleep and no polling between them: run `measure` when the machine is quiet, or get BLOCKED.

Ownership (both phases, nothing waits): `--out/harness.OUT_LOCK` (`.gizmo-perf.lock`, the one name every runner shares, so a
`mutate.py`, `ab.py` or `native.py` on the same `--out` is refused too) makes one run per `--out`, and a `build` also takes
the lock of the CHECKOUT (`harness.checkout_lock_path`, a file under /tmp), so no two builds, and no build and a mutant in
the tree, overlap. A taken lock is one REFUSED line, exit 1, and nothing written. The lock excludes simultaneous runs, not
sequential ones: keep one `--out` per runner. Before anything else a checkout git cannot name
(`harness.git_identity` unavailable) is REFUSED, `--allow-dirty` included. The child runs under `harness.child_env`: no
inherited `TESSEL_GIZMO_PERF_*` (so an exported `..._CALIBRATE=1` or `..._BLESS=1` never reaches it) and, for the build,
no `CARGO_TARGET_DIR`/`TESSEL_CARGO_LANE`; `manifest.run.env` is exactly what the runner set, `env_dropped` what it took away.

`build`: refuses a dirty tree (numbers would not belong to HEAD), builds `scene_gizmo_perf_budget` in RELEASE through the
repository's cargo wrapper under a finite timeout (no performance lock: a build is not a measurement), copies the
executable out (a copy that fails is a recorded INFRA_EXECUTABLE_MISSING naming the path, not a traceback) and
records HEAD, tree, whether the tree was dirty (`--allow-dirty`) and the executable's sha256 in `build.json`.

`measure`: refuses a dirty tree and an executable that is not the one `build` recorded for this HEAD/tree, or that
`build` built from a dirty tree (its record says `dirty`, or does not say it was clean); or it takes `--exe`, the
baseline of an A/B: nothing ties that executable to this HEAD (no head, tree, cleanliness or hash to compare), so its
run is never a PASS (`EXE_GIVEN_NOT_A_PASS_FOR_HEAD`, exit 1; a FAIL, INCONCLUSIVE or calibration keeps its status) and
its path is resolved once, because the child starts from the repository root, not from the caller's directory. It
removes any report of an earlier run in `--out`, checks the machine
(`harness.guards`: load 1/5 min, thermal, free disk) and takes the performance lock ONCE, without waiting. A busy lock or a
failed guard is a recorded BLOCKED (exit 3), never a pass and never a retry. The executable runs with
`--ignored --test-threads=1` under a finite timeout (its group is terminated on expiry): the runner fails unless exactly
the expected tests ran and passed (`tests_passed`: a test counts only when ITS result line says `ok`; `tests_failed` is the
final `failures:` list). The guards are read AGAIN after the run; a dirty reading makes an otherwise passing run INVALID
(exit 3): the numbers are kept but are not a PASS. A FAIL or INCONCLUSIVE keeps its status and its own reason when the
guard also failed, and the reason says so. A real failure is never masked by an INCONCLUSIVE note: a budget failure or a
failed test is FAIL (exit 1) whatever else the output says. INCONCLUSIVE (exit 4) is only for a run in which EVERY failed
test (the main one and, as it can happen to them too, the two negative controls) failed by the note of an unsteady plan
reference; a control that did not detect the injected work beside such a note is a FAIL.

`report.json` (every raw sample of every request kind, cold and hot, with the plan reference of each corpus) and
`manifest.json` (SHA, tree, corpus hashes, binary hash, toolchain, hardware, load before/after, thermal state and its
limitation, command, cwd, env, UTC, exit status) are written AFTER the output was flushed.

`--calibrate` reports without judging (no budget asserted) and says so in the manifest: a calibration is how the
budgets of `docs/migration/fixtures/gizmo-perf/budgets.json` were chosen, never a pass.
"""
import argparse
import json
from pathlib import Path
import re
import shutil
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import harness  # noqa: E402

ROOT = harness.ROOT
TEST = 'scene_gizmo_perf_budget'
FIXTURES = ROOT / 'docs/migration/fixtures/gizmo-perf'
EXPECTED_TESTS = [
    'the_gizmo_handlers_stay_within_their_budget_cold_and_hot_on_every_corpus',
    'the_budget_detector_rejects_one_extra_plan_of_work_per_handler_call',
    'the_budget_detector_rejects_a_millisecond_of_delay_per_handler_call',
]


def build():
    command = ['./scripts/cargo', 'test', '--release', '--locked', '-p', 'tessel-editor-core', '--test', TEST,
               '--no-run', '--message-format=json']
    # A build gets no target directory of the caller's and no `TESSEL_GIZMO_PERF_*` (`harness.child_env`).
    done = harness.run_bounded(command, env=harness.child_env(build=True), timeout=harness.BUILD_TIMEOUT_S)
    exe = None
    for line in done.stdout.splitlines():
        try:
            message = json.loads(line)
        except ValueError:
            continue
        if message.get('executable') and message.get('target', {}).get('name') == TEST:
            exe = message['executable']
    return command, done, exe


PANIC_MESSAGE = re.compile(r'panicked at [^\n]*:\n([^\n]*)')
MARKER = re.compile(r'test (\S+) \.\.\. (.*)')
VERDICT_WORDS = ('ok', 'FAILED', 'ignored')


def test_results(stdout):
    """`{name: 'ok' | 'FAILED' | 'ignored' | None}`: what libtest printed as each test's OWN result. The marker
    `test NAME ...` is printed for a failed test too, so it is not a result. With nothing printed by the test the verdict
    is on the marker's line (`test NAME ... ok`); under `--nocapture` the test's own output sits between the marker and its
    verdict (the budget test prints `test NAME ... GIZMO_PERF_SUMMARY ...`, more summary lines, then a line that is only
    `ok` or `FAILED`: see the retained `stdout.txt` of a PASS and of a FAIL), so a marker whose rest is not a verdict waits
    for the next line that is only one. `None`: the run ended before any verdict was printed."""
    results, pending = {}, None
    for line in stdout.splitlines():
        marker = MARKER.fullmatch(line)
        if marker:
            name, rest = marker.groups()
            results[name], pending = (rest, None) if rest in VERDICT_WORDS else (None, name)
        elif pending is not None and line.strip() in VERDICT_WORDS:
            results[pending], pending = line.strip(), None
    return results


def tests_passed(stdout):
    """The EXPECTED tests whose own result line says `ok` (in `EXPECTED_TESTS` order). A test the final `failures:` list
    names is never one of them, whatever else the output says: that list cannot be cut by a test's own output."""
    results, failed = test_results(stdout), set(failed_tests(stdout))
    return [name for name in EXPECTED_TESTS if results.get(name) == 'ok' and name not in failed]


def failed_tests(stdout):
    """The tests the run reports as failed: libtest's final `failures:` list. (The markers `test NAME ... FAILED` can be
    cut by a test's own output under `--nocapture`; the list cannot.)"""
    if 'failures:' not in stdout:
        return []
    names = []
    for line in stdout.rsplit('failures:', 1)[1].splitlines():
        if line.startswith('test result:'):
            break
        if re.fullmatch(r'    \S+', line):
            names.append(line.strip())
    return names


def only_unsteady(done):
    """True only when EVERY failed test failed by the `INCONCLUSIVE:` note of an unsteady plan reference and by nothing
    else: each failed test needs a panic message of its own and no message may be anything but that note. A failure the
    note does not explain (a negative control that did not detect the injected work, a test that died without a message)
    is a real failure, whatever else the output says; the controls can be inconclusive too, and then it is still one."""
    failed = failed_tests(done.stdout)
    messages = PANIC_MESSAGE.findall(done.stderr)
    return bool(failed) and len(messages) >= len(failed) and all(m.startswith('INCONCLUSIVE:') for m in messages)


def with_guard(reason, after_reason):
    """`reason` of a FAIL or an INCONCLUSIVE, plus the failed after-run guard when there was one. The status is never
    changed by the guard (a real failure is not masked by it), but the reason must not hide the context the numbers were
    taken in: the first real measurement was a FAIL read with a load average of 5.88 after the run and its reason said only
    that a budget was exceeded."""
    if not after_reason:
        return reason
    return f'{reason}; the machine guard ALSO failed after the run: {after_reason}'


def verdict(done, calibrate, report_exists, after_reason):
    """`(status, reason, exit)` of one controlled run. Order matters: a real failure is never masked."""
    text = done.text
    if done.timed_out:
        return 'FAIL', with_guard(f'the run did not finish within {harness.MEASURE_TIMEOUT_S}s (its process group was '
                                  'ended)', after_reason), 1
    passed = tests_passed(done.stdout)
    all_ok = f'test result: ok. {len(EXPECTED_TESTS)} passed; 0 failed; 0 ignored' in done.stdout
    if 'over budget:' in text:
        return 'FAIL', with_guard('a budget was exceeded (see stdout.txt)', after_reason), 1
    if 'INCONCLUSIVE:' in text and done.returncode != 0:
        if only_unsteady(done):
            return 'INCONCLUSIVE', with_guard('the plan reference moved during the measurement', after_reason), \
                harness.INCONCLUSIVE_EXIT
        return 'FAIL', with_guard('a test failed for a reason the unsteady plan reference does not explain, beside an '
                                  'INCONCLUSIVE note (see stdout.txt and stderr.txt)', after_reason), 1
    if not (done.returncode == 0 and all_ok and passed == EXPECTED_TESTS and report_exists):
        return 'FAIL', with_guard('tests failed, did not run, or wrote no report', after_reason), 1
    if after_reason:
        return 'INVALID_GUARD_AFTER_RUN', f'the machine guard failed after the run: {after_reason}', \
            harness.BLOCKED_EXIT
    if calibrate:
        return 'CALIBRATION_ONLY', 'no budget was judged', 0
    return 'PASS', '', 0


def main(argv=None):
    harness.install_termination_handlers()
    parser = argparse.ArgumentParser()
    parser.add_argument('--phase', choices=['build', 'measure'], required=True)
    parser.add_argument('--out', default='/tmp/gizmo-perf-run')
    parser.add_argument('--calibrate', action='store_true')
    parser.add_argument('--exe', help='measure an already built executable (the baseline of an A/B)')
    parser.add_argument('--label', default='candidate')
    parser.add_argument('--allow-dirty', action='store_true',
                        help='run on a dirty tree (recorded as such; the status can never be PASS for HEAD)')
    args = parser.parse_args(argv)
    out = harness.absolute(args.out)  # the child starts in the checkout: see `harness.absolute`
    # The checkout (a build only) and then `--out` are owned before anything in either is read, written or removed,
    # a refusal's own manifest included, and until this returns (the `with` is the `finally`: also on SIGTERM/SIGINT).
    # Neither lock waits: a run that finds one taken prints ONE `REFUSED` line, exits 1 and writes nothing. The lock of
    # `--out` is the one every runner shares (`harness.OUT_LOCK`): this runner keeps manifest.json, manifest.build.json,
    # build.json, report.json, stdout.txt, stderr.txt and exes/ there, and no other runner may run over them meanwhile.
    with harness.guarded(out, 'run.py', builds=args.phase == 'build') as refusal:
        if refusal:
            return harness.die(1, refusal, None)
        return run_phase(args, out)


def run_phase(args, out):
    """The phase `args.phase` over `out`, which the caller owns (`out/harness.OUT_LOCK` is held). Returns the exit code."""
    manifest = {'suite': 'gizmo-perf-core', 'phase': args.phase, 'label': args.label, 'utc_start': harness.utc(),
                'cwd': str(ROOT), 'command': sys.argv, 'calibrate': args.calibrate,
                'identity': harness.git_identity(), 'toolchain': harness.toolchain(), 'status': 'ABORTED',
                'exit': None}
    manifest_path = out / ('manifest.build.json' if args.phase == 'build' else 'manifest.json')
    # A checkout git cannot name is refused before anything else, `--allow-dirty` included: it is not a dirty tree.
    problem = harness.identity_problem(manifest['identity'])
    if problem:
        manifest.update(status='REFUSED', reason=problem)
        return harness.die(1, manifest, manifest_path)
    manifest['dirty_run'] = bool(manifest['identity']['dirty'])
    if manifest['dirty_run'] and not args.allow_dirty:
        manifest.update(status='REFUSED', reason='the working tree is dirty: numbers would not belong to HEAD')
        return harness.die(1, manifest, manifest_path)
    if args.phase == 'build':
        command, done, exe = build()
        manifest['build'] = {'command': command, **done.record(), 'executable': exe, 'stderr_tail': done.stderr[-2000:],
                             'env_dropped': harness.dropped_env(build=True)}
        if done.returncode != 0 or not exe:
            manifest.update(status='BUILD_FAILED', reason='the release build failed or timed out')
            return harness.die(1, manifest, manifest_path)
        exes = out / 'exes'
        exes.mkdir(exist_ok=True)
        try:
            shutil.copy(exe, exes / 'candidate.exe')
            manifest['binary_sha256'] = harness.sha256(exes / 'candidate.exe')
        except OSError as error:
            # The path cargo printed can be gone (another managed target was pruned during the build): a recorded
            # refusal that says which path, never a traceback with no manifest.
            manifest.update(status='INFRA_EXECUTABLE_MISSING', missing=str(exe),
                            reason=f'the executable the build named cannot be copied out ({exe}): {error}')
            return harness.die(1, manifest, manifest_path)
        manifest.update(status='BUILT', reason='no measurement was taken', exit=0)
        # `dirty` is part of the record: a binary built with `--allow-dirty` from local changes must not become "the
        # build of HEAD" once the changes are stashed and the tree reads clean again.
        harness.write(out / 'build.json', {'head': manifest['identity']['head'], 'tree': manifest['identity']['tree'],
                                           'dirty': manifest['dirty_run'],
                                           'binary_sha256': manifest['binary_sha256'], 'exe': 'exes/candidate.exe'})
        harness.write(manifest_path, manifest)
        print(json.dumps({k: manifest[k] for k in ('status', 'binary_sha256')}))
        return 0
    # --- measure
    if args.exe:
        # Resolved here, once: the path that is checked, hashed and recorded is the path `run_bounded` starts, and that
        # runs with `cwd=ROOT`, so a relative one would name another file there.
        exe = str(Path(args.exe).resolve())
        manifest['build'] = {'exe_given': exe}
        manifest['exe_unverified'] = True
    else:
        build_json = out / 'build.json'
        if not build_json.exists():
            manifest.update(status='REFUSED', reason='no `build.json` in --out: run `--phase build` first (or give --exe)')
            return harness.die(1, manifest, manifest_path)
        built = json.loads(build_json.read_text())
        exe = str(out / built['exe'])
        # `same_head` also refuses a build record with no usable head/tree: two empty answers are not "the same HEAD".
        if not harness.same_head(built, manifest['identity']):
            manifest.update(status='REFUSED', reason='the executable was built from another HEAD/tree (or `build.json` '
                                                     'names none)')
            return harness.die(1, manifest, manifest_path)
        # A record that says `dirty`, or does not say it was clean (an older `build.json`), is not the build of HEAD.
        if built.get('dirty', True):
            if not args.allow_dirty:
                manifest.update(status='REFUSED', reason='the executable was built from a dirty tree (or `build.json` '
                                                         'does not say it was not): it cannot be measured as HEAD')
                return harness.die(1, manifest, manifest_path)
            manifest['dirty_run'] = True
        if not Path(exe).exists() or harness.sha256(exe) != built['binary_sha256']:
            manifest.update(status='REFUSED', reason='the executable is missing or changed since the build phase')
            return harness.die(1, manifest, manifest_path)
        manifest['build'] = {'from': 'build.json', **built}
    if not Path(exe).exists():
        manifest.update(status='REFUSED', reason=f'{exe} does not exist')
        return harness.die(1, manifest, manifest_path)
    manifest['binary_sha256'] = harness.sha256(exe)
    manifest['corpus'] = json.loads((FIXTURES / 'corpus.manifest.json').read_text())
    budgets = FIXTURES / 'budgets.json'
    manifest['budgets_sha256'] = harness.sha256(budgets) if budgets.exists() else None
    report_path = out / 'report.json'
    for stale in (report_path, out / 'stdout.txt', out / 'stderr.txt'):
        stale.unlink(missing_ok=True)  # an earlier run's report must never stand in for this one
    state, reason = harness.guards()
    manifest['machine_before'] = state
    manifest['limits'] = harness.claims(state)
    if reason:
        manifest.update(status='BLOCKED', reason=reason)
        return harness.die(harness.BLOCKED_EXIT, manifest, manifest_path)
    with harness.Lock() as lock:
        if not lock.held:
            manifest.update(status='BLOCKED', reason='performance lock busy (not waited for)')
            return harness.die(harness.BLOCKED_EXIT, manifest, manifest_path)
        # What THIS runner sets, and only that: every inherited `TESSEL_GIZMO_PERF_*` is dropped, so an exported
        # `..._CALIBRATE=1` cannot turn a measured run into a calibration the verdict would call a measurement. The
        # record shows exactly what the child was given and what was taken away.
        set_here = {'TESSEL_GIZMO_PERF_OUT': str(report_path)}
        if args.calibrate:
            set_here['TESSEL_GIZMO_PERF_CALIBRATE'] = '1'
        env = harness.child_env(set_here)
        command = [exe, '--ignored', '--test-threads=1', '--nocapture']
        manifest['run'] = {'command': command, 'env': set_here, 'env_dropped': harness.dropped_env()}
        done = harness.run_bounded(command, cwd=ROOT, env=env, timeout=harness.MEASURE_TIMEOUT_S)
        after_reason = harness.judge_after(manifest)
    harness.write_text(out / 'stdout.txt', done.stdout)  # redacted: a test names paths of its own (`harness.redact`)
    harness.write_text(out / 'stderr.txt', done.stderr)
    manifest['run'].update(done.record())
    manifest['run']['stdout_sha256'] = harness.sha256(out / 'stdout.txt')
    manifest['tests_expected'] = EXPECTED_TESTS
    # Only the tests whose own result line says `ok`: a failed test prints its marker too (the first real measurement
    # listed three passed tests for a run whose stdout said "2 passed; 1 failed").
    manifest['tests_passed'] = tests_passed(done.stdout)
    manifest['tests_failed'] = failed_tests(done.stdout)
    status, why, code = verdict(done, args.calibrate, report_path.exists(), after_reason)
    if report_path.exists():
        manifest['report_sha256'] = harness.sha256(report_path)
    if manifest.get('exe_unverified') and status == 'PASS':
        # The manifest names this HEAD, but the executable of `--exe` was never tied to it: not a pass for HEAD, and not an
        # exit 0 either (the same reason as the dirty run below).
        status, why, code = ('EXE_GIVEN_NOT_A_PASS_FOR_HEAD',
                             'the executable came from --exe, not from the build phase of this HEAD', 1)
    elif manifest['dirty_run'] and status == 'PASS':
        # Not a success either: exit 0 would read as a pass to whatever checks only the exit code (`native.py` and
        # `ab.py` exit 1 for the same reason).
        status, why, code = 'DIRTY_RUN_NOT_A_PASS_FOR_HEAD', 'the tree was dirty (--allow-dirty)', 1
    manifest.update(status=status, reason=why)
    manifest['utc_end'] = harness.utc()
    manifest['exit'] = code
    harness.write(manifest_path, manifest)
    print(json.dumps({k: manifest[k] for k in ('status', 'reason', 'exit')}))
    for line in done.stdout.splitlines():
        if line.startswith('GIZMO_PERF_SUMMARY'):
            print(line[:1500])
    return code


if __name__ == '__main__':
    raise SystemExit(main())
