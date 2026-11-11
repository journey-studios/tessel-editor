#!/usr/bin/env python3
"""The time half of the gizmo performance suite, controlled (F055 / BL015.21).

    python3 scripts/gizmo-perf/run.py --phase build   [--out DIR]
    python3 scripts/gizmo-perf/run.py --phase measure [--out DIR] [--calibrate] [--exe PATH --label baseline]

Two phases in two invocations, because a release build leaves the machine loaded and the measurement guard (correctly)
refuses a loaded machine: a runner that built and then measured in one process would block itself. There is no
cooldown, no sleep and no polling between them: run `measure` when the machine is quiet, or get BLOCKED.

`build`: refuses a dirty tree (numbers would not belong to HEAD), builds `scene_gizmo_perf_budget` in RELEASE through the
repository's cargo wrapper under a finite timeout (no lock: a build is not a measurement), copies the executable out and
records HEAD, tree, whether the tree was dirty (`--allow-dirty`) and the executable's sha256 in `build.json`.

`measure`: refuses a dirty tree and an executable that is not the one `build` recorded for this HEAD/tree, or that
`build` built from a dirty tree (its record says `dirty`, or does not say it was clean); or it takes `--exe`, the
baseline of an A/B. It removes any report of an earlier run in `--out`, checks the machine
(`harness.guards`: load 1/5 min, thermal, free disk) and takes the performance lock ONCE, without waiting. A busy lock or a
failed guard is a recorded BLOCKED (exit 3), never a pass and never a retry. The executable runs with
`--ignored --test-threads=1` under a finite timeout (its group is terminated on expiry): the runner fails unless exactly
the expected tests ran and passed. The guards are read AGAIN after the run; a dirty reading makes the run INVALID (exit 3):
the numbers are kept but are not a PASS. A real failure is never masked by an INCONCLUSIVE note: a budget failure or a
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
import os
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
    done = harness.run_bounded(command, timeout=harness.BUILD_TIMEOUT_S)
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


def verdict(done, calibrate, report_exists, after_reason):
    """`(status, reason, exit)` of one controlled run. Order matters: a real failure is never masked."""
    text = done.text
    if done.timed_out:
        return 'FAIL', f'the run did not finish within {harness.MEASURE_TIMEOUT_S}s (its process group was ended)', 1
    ran = [name for name in EXPECTED_TESTS if f'test {name} ...' in done.stdout]
    all_ok = f'test result: ok. {len(EXPECTED_TESTS)} passed; 0 failed; 0 ignored' in done.stdout
    if 'over budget:' in text:
        return 'FAIL', 'a budget was exceeded (see stdout.txt)', 1
    if 'INCONCLUSIVE:' in text and done.returncode != 0:
        if only_unsteady(done):
            return 'INCONCLUSIVE', 'the plan reference moved during the measurement', harness.INCONCLUSIVE_EXIT
        return 'FAIL', ('a test failed for a reason the unsteady plan reference does not explain, beside an INCONCLUSIVE '
                        'note (see stdout.txt and stderr.txt)'), 1
    if not (done.returncode == 0 and all_ok and ran == EXPECTED_TESTS and report_exists):
        return 'FAIL', 'tests failed, did not run, or wrote no report', 1
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
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    manifest = {'suite': 'gizmo-perf-core', 'phase': args.phase, 'label': args.label, 'utc_start': harness.utc(),
                'cwd': str(ROOT), 'command': sys.argv, 'calibrate': args.calibrate,
                'identity': harness.git_identity(), 'toolchain': harness.toolchain(), 'status': 'ABORTED',
                'exit': None}
    manifest_path = out / ('manifest.build.json' if args.phase == 'build' else 'manifest.json')
    manifest['dirty_run'] = bool(manifest['identity']['dirty'])
    if manifest['dirty_run'] and not args.allow_dirty:
        manifest.update(status='REFUSED', reason='the working tree is dirty: numbers would not belong to HEAD')
        return harness.die(1, manifest, manifest_path)
    if args.phase == 'build':
        command, done, exe = build()
        manifest['build'] = {'command': command, **done.record(), 'executable': exe, 'stderr_tail': done.stderr[-2000:]}
        if done.returncode != 0 or not exe:
            manifest.update(status='BUILD_FAILED', reason='the release build failed or timed out')
            return harness.die(1, manifest, manifest_path)
        exes = out / 'exes'
        exes.mkdir(exist_ok=True)
        shutil.copy(exe, exes / 'candidate.exe')
        manifest['binary_sha256'] = harness.sha256(exes / 'candidate.exe')
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
        exe = args.exe
        manifest['build'] = {'exe_given': exe}
    else:
        build_json = out / 'build.json'
        if not build_json.exists():
            manifest.update(status='REFUSED', reason='no `build.json` in --out: run `--phase build` first (or give --exe)')
            return harness.die(1, manifest, manifest_path)
        built = json.loads(build_json.read_text())
        exe = str(out / built['exe'])
        if built['head'] != manifest['identity']['head'] or built['tree'] != manifest['identity']['tree']:
            manifest.update(status='REFUSED', reason='the executable was built from another HEAD/tree')
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
        env = {**os.environ, 'TESSEL_GIZMO_PERF_OUT': str(report_path)}
        if args.calibrate:
            env['TESSEL_GIZMO_PERF_CALIBRATE'] = '1'
        command = [exe, '--ignored', '--test-threads=1', '--nocapture']
        manifest['run'] = {'command': command, 'env': {k: v for k, v in env.items() if k.startswith('TESSEL_')}}
        done = harness.run_bounded(command, cwd=ROOT, env=env, timeout=harness.MEASURE_TIMEOUT_S)
        after_reason = harness.judge_after(manifest)
    (out / 'stdout.txt').write_text(done.stdout)
    (out / 'stderr.txt').write_text(done.stderr)
    manifest['run'].update(done.record())
    manifest['run']['stdout_sha256'] = harness.sha256(out / 'stdout.txt')
    manifest['tests_expected'] = EXPECTED_TESTS
    manifest['tests_passed'] = [name for name in EXPECTED_TESTS if f'test {name} ...' in done.stdout]
    status, why, code = verdict(done, args.calibrate, report_path.exists(), after_reason)
    if report_path.exists():
        manifest['report_sha256'] = harness.sha256(report_path)
    if manifest['dirty_run'] and status == 'PASS':
        status, why = 'DIRTY_RUN_NOT_A_PASS_FOR_HEAD', 'the tree was dirty (--allow-dirty)'
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
