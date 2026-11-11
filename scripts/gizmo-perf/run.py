#!/usr/bin/env python3
"""The time half of the gizmo performance suite, controlled (F055 / BL015.21).

    python3 scripts/gizmo-perf/run.py [--out DIR] [--calibrate] [--exe PATH]

1. builds `scene_gizmo_perf_budget` in RELEASE through the repository's cargo wrapper (no lock: a build is not a
   measurement) and gets the test executable from cargo's own JSON;
2. checks the machine (`harness.guards`: load, thermal, free disk) and takes the performance lock ONCE, without
   waiting. Busy lock or a failed guard is a recorded BLOCKED (exit 3), never a pass and never a retry;
3. runs the executable with `--ignored --test-threads=1`, which is the only way these tests count: the runner
   fails unless exactly the expected tests ran and passed (an ignored test that did not run is not a result);
4. writes `report.json` (every raw sample of every request kind, cold and hot, with the plan reference of each
   corpus) and `manifest.json` (SHA, tree, corpus hashes, binary hash, toolchain, hardware, load before/after,
   thermal, command, cwd, env, UTC, exit status) AFTER the output was flushed.

`--calibrate` reports without judging (no budget asserted) and says so in the manifest: a calibration is how the
budgets of `docs/migration/fixtures/gizmo-perf/budgets.json` were chosen, never a pass. `--exe` runs an already
built executable (the baseline of `ab.py`).
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import harness  # noqa: E402

ROOT = harness.ROOT
TEST = 'scene_gizmo_perf_budget'
EXPECTED_TESTS = [
    'the_gizmo_handlers_stay_within_their_budget_cold_and_hot_on_every_corpus',
    'the_budget_detector_rejects_one_extra_plan_of_work_per_handler_call',
    'the_budget_detector_rejects_a_millisecond_of_delay_per_handler_call',
]


def build():
    command = ['./scripts/cargo', 'test', '--release', '--locked', '-p', 'tessel-editor-core', '--test', TEST,
               '--no-run', '--message-format=json']
    done = harness.run(command)
    exe = None
    for line in done.stdout.splitlines():
        try:
            message = json.loads(line)
        except ValueError:
            continue
        if message.get('executable') and message.get('target', {}).get('name') == TEST:
            exe = message['executable']
    return command, done, exe


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument('--out', default='/tmp/gizmo-perf-run')
    parser.add_argument('--calibrate', action='store_true')
    parser.add_argument('--exe')
    parser.add_argument('--label', default='candidate')
    args = parser.parse_args(argv)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    manifest = {'suite': 'gizmo-perf-core', 'label': args.label, 'utc_start': harness.utc(), 'cwd': str(ROOT),
                'calibrate': args.calibrate, 'identity': harness.git_identity(), 'toolchain': harness.toolchain(),
                'status': 'ABORTED', 'exit': None}
    manifest_path = out / 'manifest.json'
    if args.exe:
        exe = args.exe
        manifest['build'] = {'exe_given': exe}
    else:
        command, done, exe = build()
        manifest['build'] = {'command': command, 'exit': done.returncode, 'executable': exe,
                             'stderr_tail': done.stderr[-2000:]}
        if done.returncode != 0 or not exe:
            manifest.update(status='BUILD_FAILED', reason='release build failed')
            return harness.die(1, manifest, manifest_path)
    manifest['binary_sha256'] = harness.sha256(exe)
    manifest['corpus'] = json.loads((ROOT / 'docs/migration/fixtures/gizmo-perf/corpus.manifest.json').read_text())
    manifest['budgets_sha256'] = harness.sha256(ROOT / 'docs/migration/fixtures/gizmo-perf/budgets.json') \
        if (ROOT / 'docs/migration/fixtures/gizmo-perf/budgets.json').exists() else None
    state, reason = harness.guards()
    manifest['machine_before'] = state
    if reason:
        manifest.update(status='BLOCKED', reason=reason)
        return harness.die(harness.BLOCKED_EXIT, manifest, manifest_path)
    with harness.Lock() as lock:
        if not lock.held:
            manifest.update(status='BLOCKED', reason='performance lock busy (not waited for)')
            return harness.die(harness.BLOCKED_EXIT, manifest, manifest_path)
        report_path = out / 'report.json'
        env = {**os.environ, 'TESSEL_GIZMO_PERF_OUT': str(report_path)}
        if args.calibrate:
            env['TESSEL_GIZMO_PERF_CALIBRATE'] = '1'
        command = [exe, '--ignored', '--test-threads=1', '--nocapture']
        manifest['run'] = {'command': command, 'env': {k: v for k, v in env.items() if k.startswith('TESSEL_')}}
        done = subprocess.run(command, cwd=ROOT, env=env, capture_output=True, text=True)
        manifest['machine_after'] = harness.machine()
    (out / 'stdout.txt').write_text(done.stdout)
    (out / 'stderr.txt').write_text(done.stderr)
    manifest['run']['exit'] = done.returncode
    manifest['run']['stdout_sha256'] = harness.sha256(out / 'stdout.txt')
    # `--nocapture` puts the test's own output between `...` and `ok`: judge by the summary line.
    ran = [name for name in EXPECTED_TESTS if f'test {name} ...' in done.stdout]
    all_ok = f'test result: ok. {len(EXPECTED_TESTS)} passed; 0 failed; 0 ignored' in done.stdout
    manifest['tests_expected'] = EXPECTED_TESTS
    manifest['tests_passed'] = ran
    if 'INCONCLUSIVE' in done.stdout + done.stderr:
        manifest.update(status='INCONCLUSIVE', reason='the plan reference moved during the measurement')
        code = harness.INCONCLUSIVE_EXIT
    elif done.returncode == 0 and all_ok and ran == EXPECTED_TESTS and report_path.exists():
        manifest['report_sha256'] = harness.sha256(report_path)
        manifest.update(status='CALIBRATION_ONLY' if args.calibrate else 'PASS',
                        reason='no budget was judged' if args.calibrate else '')
        code = 0
    else:
        manifest.update(status='FAIL', reason='tests failed, did not run, or wrote no report')
        code = 1
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
