#!/usr/bin/env python3
"""Spec of the gizmo performance runners, without a benchmark, a window or the real lock (F055 / BL015.21)."""
import atexit
import collections
import contextlib
import fcntl
import gzip
import hashlib
import importlib.util
import inspect
import io
import itertools
import json
import os
from pathlib import Path
import re
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import textwrap
import time
import types
import unittest
from unittest.mock import MagicMock, patch

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))


def load(name):
    spec = importlib.util.spec_from_file_location(name, HERE / f'{name}.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


harness, ab, mutate, native, audit = (load(n) for n in ('harness', 'ab', 'mutate', 'native', 'audit'))
runner = load('run')

# The per-CHECKOUT build lock of the runners lives under /tmp, one file per checkout. A spec run must neither wait for a
# real build of this checkout nor hold its lock, so every copy of `harness` (the spec's own and the one the runners share
# through `sys.modules`) keeps it in a directory of this run. A runner without that lock simply ignores the attribute.
CHECKOUT_LOCKS = tempfile.mkdtemp(prefix='gizmo-perf-spec-checkout-locks-')
atexit.register(shutil.rmtree, CHECKOUT_LOCKS, ignore_errors=True)
for module in (harness, native.harness):
    module.CHECKOUT_LOCK_DIR = CHECKOUT_LOCKS

# The lock of `--out`: ONE file name for every runner and phase (round 8; rounds 4 to 7 gave each runner a name of its own).
# Written out here, not read from `harness.OUT_LOCK`, so that a runner that renames it fails the spec.
SHARED_LOCK = '.gizmo-perf.lock'


def slot(samples, medians=None, long_ticks=0, p95s=None):
    ordered = sorted(samples)
    medians = medians or [ordered[len(ordered) // 2]]  # the p95 of each round defaults to the pooled one (rounds alike)
    return {'samples': list(samples), 'medians': medians, 'p95s': p95s or [ab.pct(samples, 0.95)] * len(medians),
            'ratios': [1.0], 'long_ticks': long_ticks, 'n': [len(samples)]}


class Lock(unittest.TestCase):
    def test_a_busy_lock_is_blocked_and_never_waited_for(self):
        with tempfile.TemporaryDirectory() as root:
            path = str(Path(root) / 'lock')
            with patch.object(harness, 'LOCK', path):
                first = harness.Lock()
                self.assertTrue(first.held)
                second = harness.Lock()  # returns at once: LOCK_NB
                self.assertFalse(second.held)
                first.release()
                self.assertTrue(harness.Lock().held)

    def test_guards_refuse_a_loaded_machine(self):
        busy = {'cpus': 4, 'loadavg': (3.0, 1, 1), 'thermal': '', 'free_gib': 40}
        with patch.object(harness, 'machine', return_value=busy):
            self.assertIn('load average', harness.guards()[1])
        quiet = {**busy, 'loadavg': (0.5, 1, 1)}
        with patch.object(harness, 'machine', return_value=quiet):
            self.assertIsNone(harness.guards()[1])
        with patch.object(harness, 'machine', return_value={**quiet, 'free_gib': 11.9}):
            self.assertIn('GiB free', harness.guards()[1])
        with patch.object(harness, 'machine', return_value={**quiet, 'thermal': 'CPU_Speed_Limit = 70'}):
            self.assertIn('thermal', harness.guards()[1])


def done(text='', code=0, stderr='', timed_out=False, error=None):
    return harness.Done(['x'], code, text, stderr, timed_out, 1.0, error)


def run_main(module, argv, output=None):
    """A runner's `main` as the command line would call it: its stdout is kept off the test output (in `output`, a
    `StringIO`, when the test wants to read it) and the SIGTERM/SIGINT handlers it installs are put back."""
    saved = {sig: signal.getsignal(sig) for sig in (signal.SIGTERM, signal.SIGINT)}
    try:
        with contextlib.redirect_stdout(output if output is not None else io.StringIO()):
            return module.main(argv)
    finally:
        for sig, handler in saved.items():
            signal.signal(sig, handler)


def stub_git(directory, body):
    """A `git` that is the only command on PATH: what a runner sees when git answers badly or is not there."""
    path = Path(directory) / 'git'
    path.write_text('#!/bin/sh\n' + body + '\n')
    path.chmod(0o755)


MAIN_TEST, PLAN_CONTROL, DELAY_CONTROL = runner.EXPECTED_TESTS
UNSTEADY = ('INCONCLUSIVE: 30% of the plan reference blocks of dense disagreed with themselves by more than 1.5x '
            '(limit 25%); the machine was not steady enough to judge')
NOT_CAUGHT = ('an extra plan per call was not caught for state.translate: [\n'
              '    "dense/down.translate: p50 is 2.00x the plan (budget 3)",\n]')


def libtest(failed, passed=()):
    """What the budget executable prints under `--ignored --test-threads=1 --nocapture`, in libtest's own layout (as in
    the retained runs): the markers and the final `failures:` list on stdout, the panic of each failed test on stderr.
    `failed` maps a failed test to its panic message, or to `None` for a test that died without one."""
    stdout = ['', f'running {len(failed) + len(passed)} tests']
    stdout += [f'test {name} ... ok' for name in passed]
    stdout += [f'test {name} ... FAILED' for name in failed]
    stdout += ['', 'failures:', '', 'failures:'] + [f'    {name}' for name in failed] + ['']
    stdout.append(f'test result: FAILED. {len(passed)} passed; {len(failed)} failed; 0 ignored; 0 measured')
    stderr = ''.join(
        f"\nthread '{name}' (4242) panicked at crates/core/tests/scene_gizmo_perf_budget.rs:313:5:\n{message}\n"
        "note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace\n"
        for name, message in failed.items() if message is not None)
    return done('\n'.join(stdout) + '\n', 101, stderr)


class Harness(unittest.TestCase):
    def test_a_child_that_hangs_is_ended_with_its_group_and_marked_timed_out(self):
        started = time.monotonic()
        result = harness.run_bounded(['sh', '-c', 'sleep 30 & sleep 30'], timeout=1, grace=1)
        self.assertTrue(result.timed_out)
        self.assertNotEqual(result.returncode, 0)
        self.assertLess(time.monotonic() - started, 10)

    def test_a_descendant_that_ignores_sigterm_cannot_hold_the_run_after_its_leader_has_gone(self):
        # The leader obeys SIGTERM; a descendant ignores it and keeps the output pipes open. The group kill used to stop
        # at the leader and `communicate()` then waited for the descendant (here 30 s) with the lock still held.
        script = "sh -c \"trap '' TERM; sleep 30\" & echo up; sleep 30"
        started = time.monotonic()
        result = harness.run_bounded(['sh', '-c', script], timeout=1, grace=1)
        elapsed = time.monotonic() - started
        self.assertTrue(result.timed_out)
        self.assertIn('up', result.stdout)  # what it printed before the timeout is kept
        self.assertLess(elapsed, 10)

    def test_a_descendant_in_its_own_session_cannot_hold_the_read_and_the_partial_output_is_kept(self):
        # `start_new_session` takes the descendant out of the group: no group kill reaches it and it keeps the pipes.
        code = ("import subprocess, time\n"
                "child = subprocess.Popen(['sleep', '30'], start_new_session=True)\n"
                "print('partial output', child.pid, flush=True)\n"
                "time.sleep(30)\n")
        started = time.monotonic()
        result = harness.run_bounded([sys.executable, '-c', code], timeout=1, grace=1)
        elapsed = time.monotonic() - started
        try:
            self.assertTrue(result.timed_out)
            self.assertIn('partial output', result.stdout)
            self.assertLess(elapsed, 10)
        finally:
            for word in result.stdout.split():  # the escaped `sleep` is this test's own child: it does not outlive it
                if word.isdigit():
                    with contextlib.suppress(OSError):
                        os.kill(int(word), signal.SIGKILL)

    def test_a_command_that_cannot_start_is_exit_127_with_its_error_never_an_exception(self):
        result = harness.run_bounded(['/definitely/not/a/binary'], timeout=5)
        self.assertEqual(result.returncode, 127)
        self.assertIn('cannot start', result.error)

    def test_pmset_on_apple_silicon_is_reported_as_not_verified_never_as_quiet(self):
        text = 'Note: No thermal warning level has been recorded\nNote: No performance warning level has been recorded'
        state = harness.thermal_state(text)
        self.assertEqual((state['status'], state['verified']), ('no_limit_reported', False))
        self.assertTrue(harness.claims({'thermal_state': state}))
        self.assertEqual(harness.thermal_state('CPU_Speed_Limit = 70')['status'], 'throttled')
        self.assertEqual(harness.thermal_state('CPU_Speed_Limit = 100')['verified'], True)
        self.assertEqual(harness.thermal_state('unavailable: boom')['status'], 'unavailable')

    def test_the_five_minute_load_is_judged_too(self):
        busy_5 = {'cpus': 4, 'loadavg': (0.1, 3.0, 0.1), 'thermal': '', 'free_gib': 40}
        with patch.object(harness, 'machine', return_value=busy_5):
            self.assertIn('5 min', harness.guards()[1])

    def test_the_guards_are_read_again_after_a_run_and_a_dirty_reading_is_recorded(self):
        record = {}
        loaded = {'cpus': 4, 'loadavg': (3.0, 1, 1), 'thermal': '', 'free_gib': 40}
        with patch.object(harness, 'machine', return_value=loaded):
            self.assertIn('load average', harness.judge_after(record))
        self.assertIn('load average', record['guard_after'])
        self.assertEqual(record['machine_after'], loaded)

    def test_artifacts_over_the_limit_are_refused(self):
        with tempfile.TemporaryDirectory() as root:
            (Path(root) / 'a').write_bytes(b'x' * 1024)
            self.assertIsNone(harness.artifact_reason(root))
            with patch.object(harness, 'ARTIFACT_LIMIT_MB', 0.0001):
                self.assertIn('limit', harness.artifact_reason(root))


class RunVerdict(unittest.TestCase):
    def verdict(self, finished, calibrate=False, report=True, after=None):
        return runner.verdict(finished, calibrate, report, after)

    def passing(self):
        return done('test result: ok. 3 passed; 0 failed; 0 ignored\n' + ''.join(
            f'test {name} ... ok\n' for name in runner.EXPECTED_TESTS))

    def test_a_clean_run_passes_and_a_calibration_never_does(self):
        self.assertEqual(self.verdict(self.passing())[0], 'PASS')
        self.assertEqual(self.verdict(self.passing(), calibrate=True)[0], 'CALIBRATION_ONLY')

    def test_a_budget_failure_is_a_fail_even_when_an_inconclusive_note_is_in_the_output(self):
        text = 'over budget:\ndense/move.translate: p95 is 9x the plan (budget 6)\nINCONCLUSIVE: deep reference moved'
        self.assertEqual(self.verdict(done(text, code=101))[0], 'FAIL')

    def test_inconclusive_alone_is_not_a_pass_and_not_a_fail(self):
        status, _, code = self.verdict(libtest({MAIN_TEST: UNSTEADY}, passed=[PLAN_CONTROL, DELAY_CONTROL]))
        self.assertEqual((status, code), ('INCONCLUSIVE', harness.INCONCLUSIVE_EXIT))

    def test_an_inconclusive_main_test_does_not_hide_a_control_that_failed_for_its_own_reason(self):
        # the main test is unsteady, and the extra-plan control did NOT detect the injected work: that is a real failure
        status, why, code = self.verdict(libtest({MAIN_TEST: UNSTEADY, PLAN_CONTROL: NOT_CAUGHT}, passed=[DELAY_CONTROL]))
        self.assertEqual((status, code), ('FAIL', 1))
        self.assertIn('INCONCLUSIVE', why)  # the reason says the note was there and did not explain it
        status, _, code = self.verdict(libtest({MAIN_TEST: UNSTEADY, DELAY_CONTROL: NOT_CAUGHT}, passed=[PLAN_CONTROL]))
        self.assertEqual((status, code), ('FAIL', 1))

    def test_controls_that_are_inconclusive_too_keep_the_run_inconclusive(self):
        both = libtest({MAIN_TEST: UNSTEADY, PLAN_CONTROL: UNSTEADY}, passed=[DELAY_CONTROL])
        self.assertEqual(self.verdict(both)[::2], ('INCONCLUSIVE', harness.INCONCLUSIVE_EXIT))
        controls_only = libtest({PLAN_CONTROL: UNSTEADY, DELAY_CONTROL: UNSTEADY}, passed=[MAIN_TEST])
        self.assertEqual(self.verdict(controls_only)[::2], ('INCONCLUSIVE', harness.INCONCLUSIVE_EXIT))
        every_test = libtest({MAIN_TEST: UNSTEADY, PLAN_CONTROL: UNSTEADY, DELAY_CONTROL: UNSTEADY})
        self.assertEqual(self.verdict(every_test)[::2], ('INCONCLUSIVE', harness.INCONCLUSIVE_EXIT))

    def test_a_failed_test_without_a_message_beside_an_inconclusive_note_is_a_fail(self):
        # a test that died without a panic message of its own (aborted, killed) is not explained by the note
        status, _, code = self.verdict(libtest({MAIN_TEST: UNSTEADY, DELAY_CONTROL: None}, passed=[PLAN_CONTROL]))
        self.assertEqual((status, code), ('FAIL', 1))
        self.assertEqual(runner.failed_tests(libtest({MAIN_TEST: UNSTEADY}).stdout), [MAIN_TEST])

    def test_a_dirty_guard_after_the_run_is_invalid_never_a_pass(self):
        status, _, code = self.verdict(self.passing(), after='load average (1 min) 9 over 5.5')
        self.assertEqual((status, code), ('INVALID_GUARD_AFTER_RUN', harness.BLOCKED_EXIT))

    def test_a_timeout_a_missing_report_or_a_missing_test_is_a_fail(self):
        self.assertEqual(self.verdict(done('', code=-15, timed_out=True))[0], 'FAIL')
        self.assertEqual(self.verdict(self.passing(), report=False)[0], 'FAIL')
        self.assertEqual(self.verdict(done('test result: ok. 1 passed; 0 failed; 0 ignored'))[0], 'FAIL')

    def test_a_test_whose_own_result_line_says_failed_does_not_count_as_passed(self):
        # (F17) a failed test prints its marker too: the first real measurement listed three passed tests for a run
        # whose own summary said "2 passed; 1 failed", and a marker alone was enough for the verdict to count a test
        contradictory = done('test result: ok. 3 passed; 0 failed; 0 ignored\n' + f'test {MAIN_TEST} ... FAILED\n' +
                             ''.join(f'test {name} ... ok\n' for name in (PLAN_CONTROL, DELAY_CONTROL)))
        self.assertEqual(self.verdict(contradictory)[0], 'FAIL')
        failing = libtest({MAIN_TEST: NOT_CAUGHT}, passed=[PLAN_CONTROL, DELAY_CONTROL])
        self.assertEqual(runner.tests_passed(failing.stdout), [PLAN_CONTROL, DELAY_CONTROL])
        self.assertEqual(runner.tests_passed(self.passing().stdout), runner.EXPECTED_TESTS)

    def test_the_layout_nocapture_really_prints_still_counts_its_tests_as_passed(self):
        # the marker, the test's own summary lines, then a line that is only the verdict (as in the retained stdout.txt)
        text = 'test result: ok. 3 passed; 0 failed; 0 ignored\n' + ''.join(
            f'test {name} ... GIZMO_PERF_SUMMARY {{"corpus": "small"}}\nGIZMO_PERF_SUMMARY more\nok\n'
            for name in runner.EXPECTED_TESTS)
        self.assertEqual(self.verdict(done(text))[0], 'PASS')

    def test_a_fail_carries_the_guard_that_failed_after_the_run_and_stays_a_fail(self):
        # (F18) the first real measurement was a FAIL read with a load average of 5.88 after it: the reason did not say so
        guard = 'load average (1 min) 5.88 over 5.5 (11 cpus)'
        steady = [PLAN_CONTROL, DELAY_CONTROL]
        for label, finished in (('budget', done('over budget:\ndense/move.translate: p95 is 9x the plan (budget 6)', 101)),
                                ('a test failed', libtest({MAIN_TEST: NOT_CAUGHT}, passed=steady)),
                                ('timeout', done('', code=-15, timed_out=True)),
                                ('unsteady beside a real failure', libtest({MAIN_TEST: UNSTEADY, PLAN_CONTROL: NOT_CAUGHT},
                                                                           passed=[DELAY_CONTROL]))):
            status, why, code = self.verdict(finished, after=guard)
            self.assertEqual((status, code), ('FAIL', 1), label)  # the guard never changes a real failure
            self.assertIn(guard, why, label)
        status, why, code = self.verdict(libtest({MAIN_TEST: UNSTEADY}, passed=steady), after=guard)
        self.assertEqual((status, code), ('INCONCLUSIVE', harness.INCONCLUSIVE_EXIT))
        self.assertIn(guard, why)
        self.assertNotIn('machine guard', self.verdict(done('over budget:\nx', 101))[1])  # and no guard, no sentence


class RunRecord(unittest.TestCase):
    """`build.json` says whether the executable came from a dirty tree; `measure` believes only a record that says it
    did not (the tree reads clean again once the changes are stashed, but the binary does not become HEAD's)."""

    CLEAN = {'head': 'h' * 40, 'tree': 't' * 40, 'branch': 'b', 'dirty': ''}
    RECORD = {'head': 'h' * 40, 'tree': 't' * 40, 'binary_sha256': 'x', 'exe': 'exes/candidate.exe'}

    def build(self, out, identity, *flags):
        exe = Path(out) / 'built.exe'
        exe.write_text('binary')
        with patch.object(runner.harness, 'git_identity', return_value=identity), \
                patch.object(runner.harness, 'toolchain', return_value={}), \
                patch.object(runner, 'build', return_value=(['cargo'], done(), str(exe))):
            return run_main(runner, ['--phase', 'build', '--out', out, *flags])

    def measure(self, out, record, *flags):
        (Path(out) / 'build.json').write_text(json.dumps(record))
        with patch.object(runner.harness, 'git_identity', return_value=self.CLEAN), \
                patch.object(runner.harness, 'toolchain', return_value={}):
            code = run_main(runner, ['--phase', 'measure', '--out', out, *flags])
        return code, json.loads((Path(out) / 'manifest.json').read_text())

    def test_the_build_record_says_whether_the_tree_was_dirty(self):
        with tempfile.TemporaryDirectory() as out:
            self.assertEqual(self.build(out, self.CLEAN), 0)
            self.assertIs(json.loads((Path(out) / 'build.json').read_text())['dirty'], False)
        with tempfile.TemporaryDirectory() as out:
            dirty = {**self.CLEAN, 'dirty': ' M crates/core/src/x.rs'}
            self.assertEqual(self.build(out, dirty, '--allow-dirty'), 0)
            self.assertIs(json.loads((Path(out) / 'build.json').read_text())['dirty'], True)

    def test_a_binary_built_from_a_dirty_tree_is_not_measured_as_head_once_the_tree_reads_clean(self):
        for label, record in (('dirty', {**self.RECORD, 'dirty': True}), ('not recorded', self.RECORD)):
            with tempfile.TemporaryDirectory() as out:
                code, manifest = self.measure(out, record)
                self.assertEqual((code, manifest['status']), (1, 'REFUSED'), label)
                self.assertIn('dirty', manifest['reason'], label)
        with tempfile.TemporaryDirectory() as out:  # `--allow-dirty` goes on, and the run is marked dirty for good
            code, manifest = self.measure(out, {**self.RECORD, 'dirty': True}, '--allow-dirty')
            self.assertTrue(manifest['dirty_run'])
            self.assertIn('missing', manifest['reason'])  # refused later, for the executable that is not there
        with tempfile.TemporaryDirectory() as out:  # a record that says it was clean gets as far as the executable
            code, manifest = self.measure(out, {**self.RECORD, 'dirty': False})
            self.assertIn('missing', manifest['reason'])
            self.assertFalse(manifest['dirty_run'])


class Judge(unittest.TestCase):
    key = ('dense', 'move.translate')

    def verdict(self, base, cand, unstable=None):
        return ab.judge({self.key: base}, {self.key: cand}, unstable)['dense/move.translate']

    def steady(self, value, rounds=4):
        samples = [value + i for i in range(40)]
        return slot(samples * rounds, [value + 20] * rounds)

    def test_identical_sides_are_no_regression_inside_the_scenario(self):
        self.assertEqual(self.verdict(self.steady(1000), self.steady(1000))['status'], 'NO_REGRESSION_IN_SCENARIO')

    def test_a_consistent_slowdown_beyond_the_tolerance_regresses(self):
        out = self.verdict(self.steady(1000), self.steady(1300))
        self.assertEqual(out['status'], 'REGRESSED')
        self.assertTrue(any('median' in reason for reason in out['reasons']))

    def test_a_slowdown_inside_the_tolerance_is_not_a_regression(self):
        self.assertEqual(self.verdict(self.steady(1000), self.steady(1060))['status'], 'NO_REGRESSION_IN_SCENARIO')

    def test_a_noisy_baseline_is_inconclusive_never_a_pass(self):
        base = slot([1000] * 40, [900, 1100, 1000, 1200])
        out = self.verdict(base, slot([1000] * 40, [900, 1100, 1000, 1200]))
        self.assertEqual(out['status'], 'INCONCLUSIVE')

    def test_an_unstable_plan_reference_is_inconclusive_and_does_not_hide_a_regression(self):
        same = self.verdict(self.steady(1000), self.steady(1000), {'dense': [0.5]})
        self.assertEqual(same['status'], 'INCONCLUSIVE')
        slower = self.verdict(self.steady(1000), self.steady(1300), {'dense': [0.5]})
        self.assertEqual(slower['status'], 'REGRESSED')

    def test_unstable_corpora_reads_the_reference_of_every_kept_report(self):
        steady = {'corpora': [{'corpus': 'small', 'reference': {'unstable_fraction': 0.0}}]}
        moving = {'corpora': [{'corpus': 'dense', 'reference': {'unstable_fraction': 0.4}}]}
        missing = {'corpora': [{'corpus': 'deep'}]}
        out = ab.unstable_corpora([steady, moving, missing])
        self.assertEqual(sorted(out), ['deep', 'dense'])

    def test_the_noise_limit_of_the_reference_is_the_one_budgets_json_declares(self):
        declared = json.loads(ab.BUDGETS_FILE.read_text())['reference']['max_unstable_blocks']
        self.assertEqual(ab.MAX_UNSTABLE_FRACTION, declared)
        at_limit = {'corpora': [{'corpus': 'small', 'reference': {'unstable_fraction': declared}}]}
        over = {'corpora': [{'corpus': 'dense', 'reference': {'unstable_fraction': declared + 0.01}}]}
        self.assertEqual(sorted(ab.unstable_corpora([at_limit, over])), ['dense'])  # the limit itself is still steady

    def test_the_noise_limit_follows_the_file_and_refuses_anything_that_is_not_a_fraction(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / 'budgets.json'
            path.write_text(json.dumps({'reference': {'max_unstable_blocks': 0.4}}))
            self.assertEqual(ab.max_unstable_fraction(path), 0.4)
            for bad in (None, 'x', True, -0.1, 1.5):
                path.write_text(json.dumps({'reference': {'max_unstable_blocks': bad}}))
                with self.assertRaises(ValueError, msg=repr(bad)):
                    ab.max_unstable_fraction(path)
            path.write_text(json.dumps({'reference': {}}))
            with self.assertRaises(KeyError):
                ab.max_unstable_fraction(path)

    def test_lost_samples_and_new_long_ticks_regress(self):
        base = self.steady(1000)
        fewer = slot(base['samples'][:-1], base['medians'])
        self.assertEqual(self.verdict(base, fewer)['status'], 'REGRESSED')
        longer = slot(base['samples'] + [20_000_000], base['medians'])
        self.assertEqual(self.verdict(base, longer)['status'], 'REGRESSED')

    def test_the_tail_is_kept_in_the_pool(self):
        pooled = ab.pooled([{'corpora': [{'corpus': 'dense', 'cold': {}, 'hot': {'k': {
            'samples_ns': [1, 2, 99], 'p50_ns': 2, 'ratio_p50': 1.0, 'long_ticks': 0, 'n': 3}}}]}])
        self.assertEqual(sorted(pooled[('dense', 'k')]['samples']), [1, 2, 99])

    def test_the_overall_verdict_never_calls_an_identical_product_a_pass(self):
        verdicts = {'k': {'status': 'NO_REGRESSION_IN_SCENARIO'}}
        self.assertEqual(ab.overall(verdicts, {'f': {}})[0], 'NO_REGRESSION_IN_SCENARIO')
        status, _, code = ab.overall(verdicts, {})
        self.assertEqual((status, code), ('A_A_NO_PRODUCT_DIFFERENCE', harness.INCONCLUSIVE_EXIT))
        self.assertEqual(ab.overall({'k': {'status': 'REGRESSED'}}, {})[2], 1)

    def test_a_run_is_clean_only_when_one_test_passed_and_wrote_a_fresh_report(self):
        with tempfile.TemporaryDirectory() as root:
            report = Path(root) / 'r.json'
            ok = done('test result: ok. 1 passed; 0 failed; 0 ignored')
            self.assertIn('no report', ab.run_is_clean(ok, report))
            report.write_text('{}')
            self.assertIsNone(ab.run_is_clean(ok, report))
            self.assertIn('timed out', ab.run_is_clean(done('', -15, timed_out=True), report))
            self.assertIn('1 passed', ab.run_is_clean(done('test result: ok. 0 passed; 0 failed'), report))
            self.assertIn('exit 101', ab.run_is_clean(done('x', 101), report))

    def test_the_reports_of_an_earlier_run_are_removed_before_a_new_one(self):
        with tempfile.TemporaryDirectory() as root:
            for name in ('round00-baseline.json', 'warmup-candidate.json', 'verdicts.json', 'keep.json'):
                (Path(root) / name).write_text('{}')
            ab.clear_previous(root)
            self.assertEqual(sorted(p.name for p in Path(root).iterdir()), ['keep.json'])


def git(root, *args):
    return subprocess.run(['git', '-c', 'user.name=t', '-c', 'user.email=t@t', *args], cwd=root, check=True,
                          stdout=subprocess.PIPE, text=True).stdout


def write(root, relative, text):
    path = Path(root) / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


# What `git rev-parse --local-env-vars` prints for git 2.50.1, written out here on purpose (`harness.GIT_LOCAL_ENV` is the
# runners' copy of it and a test compares both with what the installed git prints).
GIT_NAMES = ('GIT_ALTERNATE_OBJECT_DIRECTORIES', 'GIT_CONFIG', 'GIT_CONFIG_PARAMETERS', 'GIT_CONFIG_COUNT',
             'GIT_OBJECT_DIRECTORY', 'GIT_DIR', 'GIT_WORK_TREE', 'GIT_IMPLICIT_WORK_TREE', 'GIT_GRAFT_FILE', 'GIT_INDEX_FILE',
             'GIT_NO_REPLACE_OBJECTS', 'GIT_REPLACE_REF_BASE', 'GIT_PREFIX', 'GIT_SHALLOW_FILE', 'GIT_COMMON_DIR')


@contextlib.contextmanager
def without_git_env():
    """This process without any variable git calls repository-local, so that a test that wants git's honest answer for a
    directory gets it even if whoever started the spec had exported one (a git hook does)."""
    saved = {name: os.environ.pop(name) for name in GIT_NAMES if name in os.environ}
    try:
        yield
    finally:
        os.environ.update(saved)


def two_repositories(root):
    """`(a, b)`: two throwaway repositories with different commits and the files the identity's `dirty` looks at."""
    a, b = Path(root) / 'a', Path(root) / 'b'
    for repo, body in ((a, 'a'), (b, 'b')):
        repo.mkdir()
        git(repo, 'init', '-q')
        write(repo, 'Cargo.toml', f'[workspace] # {body}\n')
        write(repo, 'crates/core/src/lib.rs', f'// {body}\n')
        git(repo, 'add', '-A')
        git(repo, 'commit', '-q', '-m', body)
    return a, b


class Baseline(unittest.TestCase):
    """The A/B baseline is the pinned product; only an allowlisted, instrumental overlay may touch it."""

    MOD = 'crates/core/src/scene_plan/mod.rs'
    BUILD = 'crates/core/src/scene_plan/build.rs'
    PROBE = 'crates/core/src/scene_plan/probe.rs'

    def repo(self, root, candidate):
        """A throwaway repository: commit A (the pinned ref) and commit B (the candidate, made by `candidate`)."""
        git(root, 'init', '-q')
        write(root, 'Cargo.toml', '[workspace]\n')
        write(root, self.MOD, 'pub mod build;\n')
        write(root, self.BUILD, 'pub fn build() { work(); }\n')
        write(root, 'crates/core/tests/scene_plan.rs', '// existing test\n')
        git(root, 'add', '-A')
        git(root, 'commit', '-q', '-m', 'base')
        candidate(root)
        git(root, 'add', '-A')
        git(root, 'commit', '-q', '-m', 'candidate')
        return git(root, 'rev-parse', 'HEAD~1').strip()

    def instrument(self, root):
        write(root, self.PROBE, 'pub fn layouts_built() -> u64 { 0 }\n')
        write(root, 'crates/core/tests/scene_gizmo_perf_budget.rs', '// budget test\n')
        write(root, 'crates/core/tests/gizmo_perf_support/mod.rs', '// support\n')
        write(root, 'docs/migration/fixtures/gizmo-perf/journey.json', '{}\n')

    def test_extra_work_in_the_plan_hot_path_stays_different_from_the_baseline(self):
        def candidate(root):
            self.instrument(root)
            write(root, self.MOD, 'pub mod build;\npub mod probe;\n')
            write(root, self.BUILD, 'pub fn build() { note(); work(); work(); }\n')  # the candidate's extra work
        with tempfile.TemporaryDirectory() as repo, tempfile.TemporaryDirectory() as work:
            base = self.repo(repo, candidate)
            record = ab.make_baseline(base, work, root=Path(repo))
            src = record['src']
            self.assertEqual((src / self.BUILD).read_text(), 'pub fn build() { work(); }\n')  # pinned, untouched
            self.assertIn('pub mod probe;', (src / self.MOD).read_text())
            self.assertTrue((src / self.PROBE).exists())
            self.assertTrue((src / 'crates/core/tests/scene_gizmo_perf_budget.rs').exists())
            self.assertEqual(record['contaminated'], [])
            self.assertEqual(record['overlay_changed'], sorted([self.MOD, self.PROBE]))
            self.assertIn(self.BUILD, record['product_files_that_differ'])  # NOT erased by the overlay
            self.assertNotEqual(record['prod_tree_before_overlay'], record['prod_tree_after_overlay'])
            self.assertNotEqual(record['prod_tree_after_overlay'], record['prod_tree_candidate'])
            self.assertEqual(record['prod_tree_before_overlay'], record['pinned_ref_tree'])

    def test_a_git_dir_exported_by_the_caller_cannot_make_the_baseline_another_repositorys(self):
        # (R1, round 8) every git of `make_baseline` (ls-tree, archive, show, diff, apply) answers for `root`, never for the
        # repository the caller's `GIT_DIR`/`GIT_WORK_TREE` name: here one that does not even have the pinned commit
        def candidate(root):
            self.instrument(root)
            write(root, self.MOD, 'pub mod build;\npub mod probe;\n')
            write(root, self.BUILD, 'pub fn build() { note(); work(); work(); }\n')
        with tempfile.TemporaryDirectory() as repo, tempfile.TemporaryDirectory() as other, \
                tempfile.TemporaryDirectory() as clean_work, tempfile.TemporaryDirectory() as hostile_work:
            base = self.repo(repo, candidate)
            git(other, 'init', '-q')
            write(other, 'Cargo.toml', '[workspace] # another repository\n')
            git(other, 'add', '-A')
            git(other, 'commit', '-q', '-m', 'other')
            hostile = {'GIT_DIR': str(Path(other) / '.git'), 'GIT_WORK_TREE': other}
            with without_git_env():
                expected = ab.make_baseline(base, clean_work, root=Path(repo))
                manifest = ab.git_prod_manifest('HEAD', Path(repo))
                with patch.dict(os.environ, hostile):
                    got = ab.make_baseline(base, hostile_work, root=Path(repo))
                    hostile_manifest = ab.git_prod_manifest('HEAD', Path(repo))
            self.assertEqual(hostile_manifest, manifest)
            self.assertIn(self.BUILD, hostile_manifest)  # the production files of `repo`, not the `other` one's Cargo.toml
            for key in ('prod_tree_before_overlay', 'prod_tree_after_overlay', 'prod_tree_candidate', 'pinned_ref_tree',
                        'overlay_changed', 'product_files_that_differ', 'contaminated'):
                self.assertEqual(got[key], expected[key], key)

    def test_a_candidate_that_only_adds_work_without_the_probe_differs_in_exactly_that_file(self):
        def candidate(root):
            self.instrument(root)
            write(root, self.MOD, 'pub mod build;\npub mod probe;\n')
            write(root, self.BUILD, 'pub fn build() { work(); work(); }\n')
        with tempfile.TemporaryDirectory() as repo, tempfile.TemporaryDirectory() as work:
            record = ab.make_baseline(self.repo(repo, candidate), work, root=Path(repo))
            self.assertEqual(sorted(record['product_files_that_differ']), [self.BUILD])

    def test_a_change_of_the_declaration_file_beyond_the_probe_line_is_refused(self):
        def candidate(root):
            self.instrument(root)
            write(root, self.MOD, 'pub mod build;\npub mod probe;\npub mod extra;\n')
        with tempfile.TemporaryDirectory() as repo, tempfile.TemporaryDirectory() as work:
            base = self.repo(repo, candidate)
            with self.assertRaises(ab.Refused):
                ab.make_baseline(base, work, root=Path(repo))

    def test_an_overlay_that_reaches_a_production_file_outside_the_allowlist_is_refused(self):
        def candidate(root):
            self.instrument(root)
            write(root, self.MOD, 'pub mod build;\npub mod probe;\n')
            write(root, self.BUILD, 'pub fn build() { work(); work(); }\n')
        with tempfile.TemporaryDirectory() as repo, tempfile.TemporaryDirectory() as work:
            base = self.repo(repo, candidate)
            # a hot path smuggled in through the "instrument" list is contamination, not instrumentation
            with patch.object(ab, 'INSTRUMENT_TESTS', ab.INSTRUMENT_TESTS + [self.BUILD]):
                with self.assertRaises(ab.Refused) as caught:
                    ab.make_baseline(base, work, root=Path(repo))
            self.assertIn(self.BUILD, str(caught.exception))

    def test_a_candidate_with_no_instrumented_test_cannot_be_overlaid_onto_a_baseline(self):
        # This test used to be named `..._the_two_sides_are_the_same_product` and did not show that: it passed through the
        # `CalledProcessError` of the `git archive` of the instrumented test files, which its candidate commit lacks.
        def candidate(root):
            write(root, 'crates/core/tests/other.rs', '// test only\n')
        with tempfile.TemporaryDirectory() as repo, tempfile.TemporaryDirectory() as work:
            base = self.repo(repo, candidate)
            with self.assertRaises(subprocess.CalledProcessError):
                ab.make_baseline(base, work, root=Path(repo))
            self.assertTrue(ab.is_prod('crates/core/src/x.rs'))
            self.assertFalse(ab.is_prod('crates/core/tests/x.rs'))
            self.assertFalse(ab.is_prod('docs/x.md'))
            self.assertTrue(ab.is_prod('Cargo.lock'))

    def test_with_no_production_change_the_two_sides_are_the_same_product(self):
        # The pinned ref already carries the counter module and the instrumented test (as `main` will once this suite is
        # merged) and the candidate changes a test file only: nothing is overlaid, no product file differs, and the
        # verdict is an A/A (exit 4), never a "no regression".
        with tempfile.TemporaryDirectory() as repo, tempfile.TemporaryDirectory() as work:
            git(repo, 'init', '-q')
            write(repo, 'Cargo.toml', '[workspace]\n')
            write(repo, self.MOD, 'pub mod build;\npub mod probe;\n')
            write(repo, self.BUILD, 'pub fn build() { work(); }\n')
            self.instrument(repo)
            git(repo, 'add', '-A')
            git(repo, 'commit', '-q', '-m', 'base')
            write(repo, 'crates/core/tests/other.rs', '// test only\n')
            git(repo, 'add', '-A')
            git(repo, 'commit', '-q', '-m', 'candidate')
            record = ab.make_baseline(git(repo, 'rev-parse', 'HEAD~1').strip(), work, root=Path(repo))
            self.assertEqual(record['product_files_that_differ'], {})
            self.assertEqual(record['overlay_changed'], [])
            self.assertEqual(record['prod_tree_after_overlay'], record['prod_tree_candidate'])
            verdicts = {'k': {'status': 'NO_REGRESSION_IN_SCENARIO'}}
            status, _, code = ab.overall(verdicts, record['product_files_that_differ'])
            self.assertEqual((status, code), ('A_A_NO_PRODUCT_DIFFERENCE', harness.INCONCLUSIVE_EXIT))

    def test_the_measured_executable_is_built_from_the_core_crate_and_the_workspace_build_inputs(self):
        for path in ('crates/core/src/scene_plan/build.rs', 'crates/core/Cargo.toml', 'crates/core/build.rs', 'Cargo.toml',
                     'Cargo.lock', 'rust-toolchain.toml', '.cargo/config.toml', 'vendor/x/src/lib.rs'):
            self.assertTrue(ab.is_measured(path), path)
        for path in ('crates/desktop/src/main.rs', 'crates/render/src/lib.rs', 'crates/media/src/lib.rs',
                     'crates/core/tests/x.rs', 'docs/x.md', 'scripts/gizmo-perf/ab.py'):
            self.assertFalse(ab.is_measured(path), path)

    def test_a_difference_only_outside_the_measured_executable_is_listed_apart_and_is_never_a_no_regression(self):
        # `crates/desktop/src` is product, but the A/B builds and runs `-p tessel-editor-core --test scene_gizmo_perf_budget`:
        # its number cannot see that file, so a candidate that differs only there must not come out as "no regression".
        def candidate(root):
            self.instrument(root)
            write(root, self.MOD, 'pub mod build;\npub mod probe;\n')
            write(root, 'crates/desktop/src/main.rs', 'fn main() { /* only the desktop changed */ }\n')
        with tempfile.TemporaryDirectory() as repo, tempfile.TemporaryDirectory() as work:
            record = ab.make_baseline(self.repo(repo, candidate), work, root=Path(repo))
            self.assertEqual(record['product_files_that_differ'], {})
            self.assertEqual(record['product_files_outside_the_measurement'], ['crates/desktop/src/main.rs'])
            verdicts = {'k': {'status': 'NO_REGRESSION_IN_SCENARIO'}}
            status, reason, code = ab.overall(verdicts, record['product_files_that_differ'],
                                              record['product_files_outside_the_measurement'])
            self.assertEqual((status, code), ('DIFFERENCE_OUTSIDE_THE_MEASUREMENT', harness.INCONCLUSIVE_EXIT))
            self.assertIn('crates/desktop/src/main.rs', reason)

    def test_a_difference_in_the_measured_crate_is_still_judged_beside_one_outside_it(self):
        def candidate(root):
            self.instrument(root)
            write(root, self.MOD, 'pub mod build;\npub mod probe;\n')
            write(root, self.BUILD, 'pub fn build() { work(); work(); }\n')
            write(root, 'crates/desktop/src/main.rs', 'fn main() {}\n')
        with tempfile.TemporaryDirectory() as repo, tempfile.TemporaryDirectory() as work:
            record = ab.make_baseline(self.repo(repo, candidate), work, root=Path(repo))
            self.assertEqual(sorted(record['product_files_that_differ']), [self.BUILD])
            self.assertEqual(record['product_files_outside_the_measurement'], ['crates/desktop/src/main.rs'])
            verdicts = {'k': {'status': 'NO_REGRESSION_IN_SCENARIO'}}
            self.assertEqual(ab.overall(verdicts, record['product_files_that_differ'],
                                        record['product_files_outside_the_measurement'])[0], 'NO_REGRESSION_IN_SCENARIO')

    def test_the_pinned_baseline_is_a_full_commit_id(self):
        self.assertEqual(len(ab.BASELINE), 40)
        self.assertTrue(ab.BASELINE.startswith('cbe9e1b'))


class FakeLock:
    held = True

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False


class MeasurePhase(unittest.TestCase):
    """`ab.py --phase measure` end to end on fakes: the machine, the lock and the benchmark executables are scripted, the
    refusals, the clean-up of what the build phase left and the verdict are the real ones."""

    CLEAN = {'head': 'h' * 40, 'tree': 't' * 40, 'branch': 'b', 'dirty': ''}
    REPORT = {'suite': 'gizmo-perf-core', 'corpora': [{'corpus': 'small', 'reference': {'unstable_fraction': 0.0}, 'cold': {},
              'hot': {'move.translate': {'samples_ns': [10, 11, 12, 13], 'p50_ns': 11, 'ratio_p50': 1.0, 'long_ticks': 0,
                                         'n': 4}}}]}

    def build_dir(self, out, candidate=None):
        """What a finished `--phase build` leaves in --out: its manifest, both executables and the baseline source."""
        out = Path(out)
        (out / 'exes').mkdir()
        (out / 'baseline-src').mkdir()
        (out / 'baseline-src' / 'journey.json').write_text('{}')
        hashes = {}
        for side in ('baseline', 'candidate'):
            (out / 'exes' / f'{side}.exe').write_bytes(side.encode())
            hashes[side] = hashlib.sha256(side.encode()).hexdigest()
        (out / 'manifest.build.json').write_text(json.dumps({
            'candidate': candidate or self.CLEAN, 'baseline_commit': ab.BASELINE, 'candidate_build': {},
            'baseline_build': {}, 'executables_sha256': hashes,
            'prod_trees': {'product_files_that_differ': {'crates/core/src/x.rs': {}},
                           'product_files_outside_the_measurement': []}}))

    def measure(self, out, after_reason=None, *flags):
        def run_once(exe, path):
            Path(path).write_text(json.dumps(self.REPORT))
            return ab.harness.Done(['x'], 0, 'test result: ok. 1 passed; 0 failed; 0 ignored', '', False, 1.0, None)

        def judge_after(manifest):
            manifest['guard_after'] = after_reason or 'clean'
            return after_reason

        fakes = [patch.object(ab.harness, 'git_identity', return_value=self.CLEAN),
                 patch.object(ab.harness, 'toolchain', return_value={}),
                 patch.object(ab.harness, 'out', return_value=ab.BASELINE),
                 patch.object(ab.harness, 'git_strict', create=True, return_value=(True, ab.BASELINE)),
                 patch.object(ab.harness, 'guards', return_value=({}, None)),
                 patch.object(ab.harness, 'machine', return_value={'thermal': ''}),
                 patch.object(ab.harness, 'judge_after', side_effect=judge_after),
                 patch.object(ab.harness, 'Lock', FakeLock),
                 patch.object(ab, 'run_once', side_effect=run_once)]
        with contextlib.ExitStack() as stack:
            for fake in fakes:
                stack.enter_context(fake)
            code = run_main(ab, ['--phase', 'measure', '--out', str(out), '--rounds', '2', *flags])
        return code, json.loads((Path(out) / 'manifest.json').read_text())

    def test_the_executables_of_an_invalid_run_are_kept_for_a_retry_and_those_of_a_judged_run_are_not(self):
        with tempfile.TemporaryDirectory() as out:
            self.build_dir(out)
            code, manifest = self.measure(out, after_reason='load average (1 min) 9 over 5.5')
            self.assertEqual((code, manifest['status']), (harness.BLOCKED_EXIT, 'INVALID_GUARD_AFTER_RUN'))
            # the comment of the run says "a BLOCKED one keeps both for a retry": an INVALID one is not judged either
            self.assertTrue((Path(out) / 'exes' / 'baseline.exe').exists())
            self.assertTrue((Path(out) / 'baseline-src').is_dir())
        with tempfile.TemporaryDirectory() as out:
            self.build_dir(out)
            code, manifest = self.measure(out)
            self.assertEqual((code, manifest['status']), (0, 'NO_REGRESSION_IN_SCENARIO'))
            self.assertFalse((Path(out) / 'exes').exists())
            self.assertFalse((Path(out) / 'baseline-src').exists())

    def test_executables_built_from_a_dirty_tree_are_not_judged_as_head_once_the_tree_reads_clean(self):
        dirty = {**self.CLEAN, 'dirty': ' M crates/core/src/scene_plan/build.rs'}
        with tempfile.TemporaryDirectory() as out:
            self.build_dir(out, candidate=dirty)
            code, manifest = self.measure(out)
            self.assertEqual((code, manifest['status']), (1, 'REFUSED'))
            self.assertIn('dirty', manifest['reason'])
            self.assertTrue((Path(out) / 'exes' / 'candidate.exe').exists())  # refused before anything ran
        with tempfile.TemporaryDirectory() as out:  # `--allow-dirty` measures them and records that they were dirty
            self.build_dir(out, candidate=dirty)
            code, manifest = self.measure(out, None, '--allow-dirty')
            self.assertTrue(manifest['dirty_run'])
        with tempfile.TemporaryDirectory() as out:  # a manifest that does not say whether the build was clean
            self.build_dir(out)
            built = json.loads((Path(out) / 'manifest.build.json').read_text())
            del built['candidate']['dirty']
            (Path(out) / 'manifest.build.json').write_text(json.dumps(built))
            code, manifest = self.measure(out)
            self.assertEqual((code, manifest['status']), (1, 'REFUSED'))

    def test_a_second_attempt_keeps_the_first_one_in_attempts(self):
        # (F16) the first real A/B lost the numbers of its INVALID first attempt to its own retry
        with tempfile.TemporaryDirectory() as out:
            self.build_dir(out)
            code, first = self.measure(out, after_reason='load average (1 min) 9 over 5.5')
            self.assertEqual(first['status'], 'INVALID_GUARD_AFTER_RUN')
            code, second = self.measure(out)
            self.assertEqual(second['status'], 'NO_REGRESSION_IN_SCENARIO')
            attempts = Path(out) / 'attempts'
            kept = sorted(attempts.iterdir()) if attempts.is_dir() else []
            self.assertEqual(len(kept), 1, [p.name for p in kept])
            self.assertTrue(kept[0].name.endswith('-INVALID_GUARD_AFTER_RUN'), kept[0].name)
            self.assertEqual(json.loads((kept[0] / 'manifest.json').read_text())['status'], 'INVALID_GUARD_AFTER_RUN')
            self.assertTrue((kept[0] / 'verdicts.json').is_file())
            self.assertTrue(sorted(kept[0].glob('round*.json')))
            self.assertEqual(second['previous_attempt_archived_at'], harness.redact(str(kept[0])))
            self.assertTrue(sorted(Path(out).glob('round*.json')))  # and the new attempt's own reports are in --out
            # a third attempt, refused for its own reasons (the second was judged: its inputs are gone), keeps the second
            code, third = self.measure(out)
            self.assertEqual((code, third['status']), (1, 'REFUSED'))
            names = sorted(p.name for p in attempts.iterdir())
            self.assertEqual(len(names), 2, names)
            self.assertTrue(any(name.endswith('-NO_REGRESSION_IN_SCENARIO') for name in names), names)

    def test_a_run_whose_reports_hold_no_sample_is_inconclusive_never_a_pass(self):
        with tempfile.TemporaryDirectory() as out:
            self.build_dir(out)
            self.REPORT = {'suite': 'gizmo-perf-core', 'corpora': [
                {'corpus': 'small', 'reference': {'unstable_fraction': 0.0}, 'cold': {}, 'hot': {}}]}
            code, manifest = self.measure(out)
            self.assertEqual((code, manifest['status']), (harness.INCONCLUSIVE_EXIT, 'INCONCLUSIVE'))


class AbBuild(unittest.TestCase):
    """`ab.py --phase build` on fakes: each release build is a function that writes the bytes of an executable where
    cargo would have, and the baseline source is a directory. The order of things, the refusals and the record are the
    real ones."""

    CLEAN = {'head': 'h' * 40, 'tree': 't' * 40, 'branch': 'b', 'dirty': ''}

    def run_build(self, root, candidate='candidate', baseline='baseline', candidate_exit=0, baseline_exit=0,
                  same_file=False, candidate_gone=False, baseline_gone=False, prune_candidate_in_baseline_build=False,
                  env=None, plant=None):
        """Returns `(exit code, manifest, out, calls)`; `calls` lists, for each build, which side it was and which
        executables were already in `--out/exes` when it started."""
        root = Path(root)
        out = root / 'out'
        if plant:
            plant(out)
        target = {'candidate': root / 'target-candidate',
                  'baseline': root / ('target-candidate' if same_file else 'target-baseline')}
        calls = []

        def make_baseline(ref, directory):
            src = Path(directory) / 'baseline-src'
            src.mkdir(parents=True, exist_ok=True)
            (src / 'journey.json').write_text('{}')
            return {'src': src, 'prod_tree_before_overlay': 'x', 'prod_tree_after_overlay': 'x',
                    'prod_tree_candidate': 'x', 'pinned_ref_tree': 'x', 'overlay_changed': [], 'allowlist': [],
                    'contaminated': [], 'product_files_that_differ': {'crates/core/src/x.rs': {}},
                    'product_files_outside_the_measurement': []}

        def build(cwd):
            side = 'candidate' if Path(cwd) == ab.ROOT else 'baseline'
            exes = out / 'exes'
            calls.append((side, sorted(p.name for p in exes.iterdir()) if exes.is_dir() else []))
            if side == 'candidate':
                if not candidate_gone:
                    target['candidate'].write_bytes(candidate.encode())
                return ['cargo'], done('', candidate_exit), str(target['candidate'])
            if prune_candidate_in_baseline_build:
                target['candidate'].unlink(missing_ok=True)  # the cache GC of the cargo wrapper, mid-build
            if not baseline_gone:
                target['baseline'].write_bytes(baseline.encode())
            return ['cargo'], done('', baseline_exit), str(target['baseline'])

        fakes = [patch.object(ab.harness, 'git_identity', return_value=self.CLEAN),
                 patch.object(ab.harness, 'toolchain', return_value={}),
                 patch.object(ab.harness, 'out', return_value=ab.BASELINE),
                 patch.object(ab.harness, 'git_strict', create=True, return_value=(True, ab.BASELINE)),
                 patch.object(ab, 'make_baseline', side_effect=make_baseline), patch.object(ab, 'build', side_effect=build)]
        with contextlib.ExitStack() as stack:
            stack.enter_context(bounded())
            for fake in fakes:
                stack.enter_context(fake)
            stack.enter_context(patch.dict(os.environ, env or {}))
            for name in ('CARGO_TARGET_DIR', 'TESSEL_CARGO_LANE'):
                if name not in (env or {}):
                    os.environ.pop(name, None)  # this spec must not depend on how it was started
            code = run_main(ab, ['--phase', 'build', '--out', str(out)])
        built = out / 'manifest.build.json'
        return code, json.loads((built if built.exists() else out / 'manifest.json').read_text()), out, calls

    def test_the_candidate_executable_is_copied_before_the_baseline_build_starts(self):
        # (F2) the first real A/B: the cache GC pruned the candidate's managed target while the baseline was building
        with tempfile.TemporaryDirectory() as root:
            code, manifest, out, calls = self.run_build(root, prune_candidate_in_baseline_build=True)
            self.assertEqual((code, manifest['status']), (0, 'BUILT'))
            self.assertEqual(calls, [('candidate', []), ('baseline', ['candidate.exe'])])
            self.assertEqual((out / 'exes' / 'candidate.exe').read_bytes(), b'candidate')
            self.assertEqual((out / 'exes' / 'baseline.exe').read_bytes(), b'baseline')

    def test_an_executable_the_build_named_but_that_is_gone_is_a_recorded_refusal_with_its_path(self):
        for label, gone, name in (('candidate', {'candidate_gone': True}, 'target-candidate'),
                                  ('baseline', {'baseline_gone': True}, 'target-baseline')):
            with self.subTest(label), tempfile.TemporaryDirectory() as root:
                code, manifest, out, calls = self.run_build(root, **gone)
                self.assertEqual((code, manifest['status']), (1, 'INFRA_EXECUTABLE_MISSING'))
                self.assertTrue(manifest['missing'].endswith(name), manifest['missing'])
                self.assertIn(name, manifest['reason'])
                self.assertFalse((out / 'manifest.build.json').exists())
                self.assertFalse((out / 'baseline-src').exists())  # nothing measurable came out of it

    def test_one_artifact_measured_as_both_sides_is_refused(self):
        for label, kwargs in (('the same file', {'same_file': True}),
                              ('the same bytes', {'candidate': 'same', 'baseline': 'same'})):
            with self.subTest(label), tempfile.TemporaryDirectory() as root:
                code, manifest, out, calls = self.run_build(root, **kwargs)
                self.assertEqual((code, manifest['status']), (1, 'AB_EXECUTABLES_COLLIDE'), label)
                self.assertFalse((out / 'manifest.build.json').exists())
        with tempfile.TemporaryDirectory() as root:  # the control: two artifacts are built and recorded
            code, manifest, out, calls = self.run_build(root)
            self.assertEqual((code, manifest['status']), (0, 'BUILT'))
            self.assertEqual(len(set(manifest['executables_sha256'].values())), 2)

    def test_a_target_directory_in_the_environment_refuses_the_build_phase_before_any_build(self):
        for name in ('CARGO_TARGET_DIR', 'TESSEL_CARGO_LANE'):
            with self.subTest(name), tempfile.TemporaryDirectory() as root:
                code, manifest, out, calls = self.run_build(root, env={name: '/somewhere'})
                self.assertEqual((code, manifest['status']), (1, 'REFUSED'))
                self.assertEqual(manifest['env_set'], [name])
                self.assertEqual(calls, [])

    def test_a_build_that_fails_leaves_no_product_of_an_earlier_one(self):
        def earlier(out):
            (out / 'exes').mkdir(parents=True)
            (out / 'exes' / 'baseline.exe').write_bytes(b'an earlier baseline')
            (out / 'exes' / 'candidate.exe').write_bytes(b'an earlier candidate')
            (out / 'manifest.build.json').write_text('{"an": "earlier build"}')

        with tempfile.TemporaryDirectory() as root:
            code, manifest, out, calls = self.run_build(root, candidate_exit=1, plant=earlier)
            self.assertEqual((code, manifest['status']), (1, 'BUILD_FAILED'))
            self.assertFalse((out / 'manifest.build.json').exists())  # `--phase measure` could believe the pair
            self.assertEqual(sorted(p.name for p in (out / 'exes').iterdir()), [])


def run_text(failed=(), ok=(), results='FAILED. 0 passed; 1 failed', extra=''):
    lines = ['running 1 test']
    lines += [f'test {name} ... ok' for name in ok]
    lines += [f'test {name} ... FAILED' for name in failed]
    if failed:
        lines += ['', 'failures:', ''] + [f'    {name}' for name in failed] + ['']
    lines += [extra, f'test result: {results}; 0 ignored']
    return '\n'.join(lines)


TIME = mutate.TIME_TEST
BUDGET_LINES = '\n'.join(f'dense/move.{k}: p95 is 9.00x the plan (budget 6.1)' for k in ('translate', 'rotate', 'scale'))


class Verdicts(unittest.TestCase):
    """A mutant is KILLED only by its named test failing with its own message; everything else is not a kill."""

    def classify(self, finished, mutant='delay_1ms_per_handler', detector='time', guard_after=None):
        return mutate.classify(finished, mutate.EXPECT.get((mutant, detector)),
                               mutate.MUST_PASS.get((mutant, detector), ()), guard_after)[0]

    def test_the_named_test_failing_with_its_budget_lines_is_the_only_kill(self):
        text = run_text([TIME], extra=f'over budget:\n{BUDGET_LINES}')
        self.assertEqual(self.classify(done(text, 101)), 'KILLED')

    def test_cargo_not_found_a_missing_command_or_a_start_error_is_infra_never_a_kill(self):
        self.assertEqual(self.classify(done('', 127, error='cannot start cargo')), 'INFRA_ERROR')
        self.assertEqual(self.classify(done('', 127)), 'INFRA_ERROR')

    def test_a_timeout_a_full_disk_and_a_compile_error_are_infra(self):
        self.assertEqual(self.classify(done('', -15, timed_out=True)), 'INFRA_TIMEOUT')
        self.assertEqual(self.classify(done('write: No space left on device', 101)), 'INFRA_DISK')
        self.assertEqual(self.classify(done('error[E0425]: cannot find value\ncould not compile', 101)), 'COMPILE_ERROR')

    def test_zero_tests_and_no_result_are_not_a_kill(self):
        self.assertEqual(self.classify(done('running 0 tests\ntest result: ok. 0 passed; 0 failed', 0)), 'ZERO_TESTS')
        self.assertEqual(self.classify(done('Killed: 9', -9)), 'NO_TEST_RESULT')

    def test_a_failure_of_another_test_or_with_another_message_is_not_a_kill(self):
        other = run_text(['some_other_test'], extra=f'over budget:\n{BUDGET_LINES}')
        self.assertEqual(self.classify(done(other, 101)), 'UNEXPECTED_FAILURE')
        panic = run_text([TIME], extra="thread panicked at 'No such file or directory (fixture)'")
        self.assertEqual(self.classify(done(panic, 101)), 'UNEXPECTED_FAILURE')

    def test_too_few_budget_lines_for_the_delay_mutant_is_not_a_kill(self):
        text = run_text([TIME], extra='dense/move.translate: p95 is 9.00x the plan (budget 6.1)')
        self.assertEqual(self.classify(done(text, 101)), 'UNEXPECTED_FAILURE')  # min_lines is 3 for a delay
        self.assertEqual(self.classify(done(text, 101), 'extra_plan_per_handler'), 'KILLED')  # 1 is enough there

    def test_an_inconclusive_run_and_a_dirty_guard_after_it_are_not_a_kill(self):
        text = run_text([TIME], extra=f'INCONCLUSIVE: the reference moved\n{BUDGET_LINES}')
        self.assertEqual(self.classify(done(text, 101)), 'INCONCLUSIVE')
        killed = run_text([TIME], extra=f'over budget:\n{BUDGET_LINES}')
        self.assertEqual(self.classify(done(killed, 101), guard_after='load average 9'), 'INVALID_GUARD_AFTER_RUN')

    def test_a_structural_detector_that_must_not_see_a_delay_has_to_run_its_tests_and_pass(self):
        names = mutate.MUST_PASS[('delay_1ms_per_handler', 'structural')]
        survived = run_text(ok=names, results='ok. 7 passed; 0 failed')
        self.assertEqual(self.classify(done(survived, 0), detector='structural'), 'SURVIVED')
        absent = run_text(ok=['unrelated'], results='ok. 1 passed; 0 failed')
        self.assertEqual(self.classify(done(absent, 0), detector='structural'), 'ZERO_TESTS')
        failed = run_text([names[0]], extra='laid the scene out 2 times (budget 1)')
        self.assertEqual(self.classify(done(failed, 101), detector='structural'), 'UNEXPECTED_FAILURE')

    def test_the_host_detector_needs_its_own_message(self):
        name = 'workstation::scene_gizmo_perf_tests::an_idle_window_plans_nothing_per_tick_and_the_overlay_stays_memoised'
        text = run_text([name], extra='corpus: 100 idle ticks laid the scene out')
        self.assertEqual(self.classify(done(text, 101), 'replan_per_tick', 'host'), 'KILLED')
        wrong = run_text([name], extra='thread panicked: fixture missing')
        self.assertEqual(self.classify(done(wrong, 101), 'replan_per_tick', 'host'), 'UNEXPECTED_FAILURE')

    HOST_TEST = 'workstation::scene_gizmo_perf_tests::an_idle_window_plans_nothing_per_tick_and_the_overlay_stays_memoised'

    def test_memo_off_is_killed_by_each_of_the_three_assertions_a_memo_that_never_hits_breaks(self):
        # (F15) with the memo off the FIRST assertion to fail is the layout count of the repeated syncs (the first real
        # run, M1 of `measure-12bc8da`): the runner did not accept it, so the mutant that was detected was not "killed"
        for message in ('small: a sync of an unchanged state laid the scene out more than once (4 layouts for 4 syncs)',
                        'small: the overlay was asked for again after a memo hit',
                        'small: every sync of an unchanged state is a memo hit (0 of 4)'):
            text = run_text([self.HOST_TEST], extra=message)
            self.assertEqual(self.classify(done(text, 101), 'memo_off', 'host'), 'KILLED', message)

    def test_memo_off_stays_strict_about_what_else_may_have_failed_the_host_test(self):
        for noise in ('thread panicked: fixture missing', 'the journey ran to the end (3 of 4 steps)',
                      'a sync of the unchanged state laid out'):
            text = run_text([self.HOST_TEST], extra=noise)
            self.assertEqual(self.classify(done(text, 101), 'memo_off', 'host'), 'UNEXPECTED_FAILURE', noise)
        compile_error = done('error[E0425]: cannot find value\ncould not compile', 101)
        self.assertEqual(self.classify(compile_error, 'memo_off', 'host'), 'COMPILE_ERROR')
        self.assertEqual(self.classify(done('', -15, timed_out=True), 'memo_off', 'host'), 'INFRA_TIMEOUT')
        # and the accepted lines are the ones the Rust test really prints
        source = (harness.ROOT / 'crates/desktop/src/workstation/scene_gizmo_perf_tests.rs').read_text()
        for pattern in mutate.EXPECT[('memo_off', 'host')]['messages']:
            self.assertIn(pattern, source)

    def test_failed_names_reads_the_final_list_even_when_output_is_not_captured(self):
        text = f'test {TIME} ... GIZMO_PERF_SUMMARY x\nfailures:\n    {TIME}\n\ntest result: FAILED. 0 passed; 1 failed'
        self.assertEqual(mutate.failed_names(text), {TIME})

    def manifest(self, **records):
        mutants = {}
        for name, outcomes in records.items():
            must = mutate.MUTANTS[name]['detectors']
            mutants[name] = {'must': must, **{d: {'outcome': o} for d, o in outcomes.items()}}
        return {'mutants': mutants}

    def test_the_overall_verdict_separates_a_pass_a_wrong_answer_and_no_answer(self):
        good = self.manifest(delay_1ms_per_handler={'structural': 'SURVIVED', 'time': 'KILLED'})
        self.assertEqual(mutate.overall(good), ('ALL_AS_EXPECTED', 0))
        wrong = self.manifest(delay_1ms_per_handler={'structural': 'SURVIVED', 'time': 'SURVIVED'})
        self.assertEqual(mutate.overall(wrong), ('UNEXPECTED', 1))
        for outcome in ('BLOCKED', 'INFRA_ERROR', 'MISSING_EXECUTABLE', 'INVALID_GUARD_AFTER_RUN'):
            none = self.manifest(delay_1ms_per_handler={'structural': 'SURVIVED', 'time': outcome})
            self.assertEqual(mutate.overall(none), ('BLOCKED_OR_INFRA', harness.BLOCKED_EXIT), outcome)
        absent = self.manifest(delay_1ms_per_handler={'structural': 'SURVIVED'})  # no time record at all
        self.assertEqual(mutate.overall(absent)[0], 'BLOCKED_OR_INFRA')

    def test_a_checkout_that_was_not_restored_fails_loudly(self):
        bad = self.manifest(memo_off={'host': 'KILLED'})
        bad['mutants']['memo_off']['restore'] = {'restored': False}
        self.assertEqual(mutate.overall(bad), ('NOT_RESTORED', 1))

    def test_a_mutated_file_is_restored_when_the_runner_is_terminated_and_the_hashes_are_logged(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / 'f.rs'
            path.write_text('fn a() {}\n')
            mutant = {'file': str(path), 'old': 'fn a() {}', 'new': 'fn a() { loop {} }'}
            log = {}
            previous = harness.install_termination_handlers()
            try:
                with patch.object(mutate, 'ROOT', Path('/')), patch.object(mutate, 'file_hash', return_value='h'), \
                        patch.object(mutate, 'committed_hash', return_value='h'):
                    with self.assertRaises(SystemExit) as stopped:
                        with mutate.Mutated(mutant, log):
                            self.assertIn('loop', path.read_text())
                            os.kill(os.getpid(), signal.SIGTERM)
                            time.sleep(1)
            finally:
                for sig, handler in previous.items():
                    signal.signal(sig, handler)
            self.assertEqual(stopped.exception.code, 128 + signal.SIGTERM)
            self.assertEqual(path.read_text(), 'fn a() {}\n')
            self.assertTrue(log['restored'])
            self.assertEqual({'committed_blob', 'mutated_blob', 'restored_blob'} - set(log), set())

    def test_an_anchor_that_is_not_unique_stops_before_touching_the_file(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / 'f.rs'
            path.write_text('x x\n')
            with patch.object(mutate, 'ROOT', Path('/')):
                with self.assertRaises(mutate.AnchorNotUnique):
                    with mutate.Mutated({'file': str(path), 'old': 'x', 'new': 'y'}):
                        pass
            self.assertEqual(path.read_text(), 'x x\n')


class Audit(unittest.TestCase):
    def report(self, p50, p95, plan_ns=1_000_000):
        stats = {'ratio_p50': p50, 'ratio_p95': p95}
        return {'corpora': [{'corpus': 'dense', 'reference': {'p50_ns': plan_ns}, 'cold': {}, 'hot': {'move': stats}}]}

    def budgets(self, p50, p95):
        return {'ratios': {'dense': {'move': {'p50_ratio': p50, 'p95_ratio': p95}}}}

    def test_a_budget_with_more_than_a_plan_of_slack_lets_one_extra_plan_through(self):
        out = audit.audit(self.budgets(3.0, 5.0), [self.report(1.0, 2.0)])['summary']
        self.assertEqual(out['one_extra_plan_passes_in_time'], ['dense/move'])

    def test_a_tight_budget_sees_one_extra_plan(self):
        out = audit.audit(self.budgets(1.5, 2.5), [self.report(1.0, 2.0)])['summary']
        self.assertEqual(out['one_extra_plan_passes_in_time'], [])

    def test_a_delay_is_converted_with_the_measured_plan_time_not_a_clock(self):
        # plan = 0.5 ms: 1 ms is 2 plans. Slack of 5 plans lets it through; slack of 1 plan does not.
        wide = audit.audit(self.budgets(7.0, 7.0), [self.report(1.0, 1.0, 500_000)])['summary']
        self.assertEqual(wide['delay_1ms_passes_in_time'], ['dense/move'])
        tight = audit.audit(self.budgets(2.0, 2.0), [self.report(1.0, 1.0, 500_000)])['summary']
        self.assertEqual(tight['delay_1ms_passes_in_time'], [])

    def test_the_worst_observation_over_all_reports_is_the_one_audited(self):
        out = audit.audit(self.budgets(3.0, 5.0), [self.report(1.0, 2.0), self.report(1.0, 4.5)])
        self.assertEqual(out['kinds']['dense/move']['observed_p95'], 4.5)
        self.assertEqual(out['summary']['p95_headroom_at_least_2x'], [])


class FakeNative:
    process = types.SimpleNamespace(pid=1)

    def capture(self, path):
        return None, 'fake'

    def stop(self):
        return []


class FakeWindow(native.Window):
    """A window whose monitor reports only what a command really did: a scene shown by `monitor.show_scene` and a new
    picture per request. `stale_shows` are scenes whose `show_scene` answers ok but draws nothing (the BL015.22
    hypothesis, when it is true)."""

    log = []
    stale_shows = set()
    instances = []

    def __init__(self, binary, home):
        self.native = FakeNative()
        self.socket = 'fake'
        self.frames = {}
        self.deadline = None
        self.launched = time.perf_counter()
        self.request = 0
        self.scene = None
        self.frame = None
        FakeWindow.instances.append(self)

    def ready(self):
        return 1.0

    def op(self, request, timeout=90):
        FakeWindow.log.append(request['op'])
        kind = request['op']
        if kind == 'project.open':
            return True, {'action': 'project.open', 'project': 'x'}
        if kind == 'scene.get':
            return True, {'value': {'x': 0, 'y': 0, 'z': 0}}
        if kind == 'history.status':
            return True, {'undo_depth': 0, 'redo_depth': 0}
        if kind == 'scene.resources.resolve':
            return True, {'state': 'applied'}
        if kind == 'monitor.show_scene' and request['scene'] not in FakeWindow.stale_shows:
            self.request += 1
            self.scene = request['scene']
            self.frame = self.frame or 0  # the edit frame survives a scene change
        if kind == 'seek':
            self.request += 1
            self.frame = request['frame']
        if kind == 'render.status':
            return True, {'report': {}}
        return True, {'overlay': {'visible': False}}

    def send(self, request, timeout=90):
        if request.get('target') == 'monitor.viewport':
            return {'ok': True, 'data': {'state': self.state()}}
        return {'ok': True}

    def state(self):
        if self.scene is None:
            return {'phase': 'ready', 'directed': {}, 'overlay': {'supported': True}}
        return {'phase': 'ready', 'headline': '', 'overlay': {'supported': True},
                'directed': {'scene': self.scene, 'active': None, 'queued': None, 'suspended': False,
                             'shown': {'request_id': self.request, 'edit_frame': self.frame}},
                'last_frame': {'request': self.request, 'revision': self.request, 'hash': f'h{self.request}',
                               'inspected': True}}


class NativeFlow(unittest.TestCase):
    JOURNEY = {'version': 1, 'frame': 24, 'scene': 'proof', 'scene_b': 'proof-b', 'base': 'scripts/x',
               'corpora': {'small': {'bundle': 'a.json', 'bundle_b': 'b.json', 'node': 'n', 'node_b': 'm',
                                     'other': 'o'}},
               'steps': [
                   {'id': 'open.project', 'phase': 'cold', 'op': {'op': 'project.create', 'name': 'x'}},
                   {'id': 'open.attach', 'phase': 'cold', 'op': {'op': 'scene.attach', 'id': '$scene', 'path': '$bundle'}},
                   {'id': 'open.show', 'phase': 'cold', 'op': {'op': 'monitor.show_scene', 'scene': '$scene'}},
                   {'id': 'open.seek', 'phase': 'cold', 'op': {'op': 'seek', 'frame': '$frame'}},
                   {'id': 'open.first_frame', 'phase': 'cold', 'wait': 'presented', 'frame': '$frame'},
                   {'id': 'same.show', 'phase': 'same_bundle', 'op': {'op': 'monitor.show_scene', 'scene': 'proof-c'},
                    'expect': {'shown_scene': 'proof-c'}},
               ]}

    def run_it(self, stale=()):
        FakeWindow.log, FakeWindow.stale_shows, FakeWindow.instances = [], set(stale), []
        golden = {'small': {}}
        with tempfile.TemporaryDirectory() as out, \
                patch.object(native, 'Window', FakeWindow), \
                patch.object(native, 'author_saved_project',
                             return_value=(Path('/tmp/saved.tessel.json'), {'path': '/tmp/saved.tessel.json'})):
            return native.run_corpus('bin', self.JOURNEY, golden, 'small', out, {})

    def checks(self, result):
        return {c['check']: c['ok'] for c in result['checks']}

    def test_the_opening_is_a_real_project_open_of_the_saved_project_not_a_create(self):
        result = self.run_it()
        self.assertIn('project.open', FakeWindow.log)
        self.assertNotIn('project.create', FakeWindow.log)
        self.assertNotIn('scene.attach', FakeWindow.log)  # the scene came with the saved document
        self.assertTrue(result['opened_saved_project'])
        self.assertTrue(all(c['ok'] for c in result['checks']), [c for c in result['checks'] if not c['ok']])
        self.assertIn('launch_to_socket_ms', result)
        phases = {s['step']: s['phase'] for s in result['samples']}
        self.assertEqual(phases['open.project'], 'cold')

    def test_a_show_that_answers_ok_but_draws_nothing_new_fails_its_own_check(self):
        result = self.run_it(stale={'proof-c'})
        failed = [c['check'] for c in result['checks'] if not c['ok']]
        self.assertTrue(any(label.startswith('same.show') and 'proof-c' in label for label in failed), failed)

    def test_a_second_scene_of_the_same_bundle_is_in_the_journey_and_expects_to_be_shown(self):
        journey = json.loads((native.FIXTURES / 'journey.json').read_text())
        steps = {step['id']: step for step in journey['steps']}
        self.assertEqual(steps['same.show']['expect'], {'shown_scene': 'proof-c'})
        self.assertEqual(steps['same.attach']['op']['path'], steps['switch.attach']['op']['path'])
        for sid in ('same.attach', 'same.resources', 'same.show'):
            self.assertEqual(steps[sid]['class'], 'presentation')
            self.assertEqual(steps[sid]['surfaces'], ['host', 'native'])

    def test_the_corpus_manifest_hashes_the_current_journey(self):
        self.assertIsNone(native.verify_corpus())


class NewPicture(unittest.TestCase):
    def state(self, request=5, scene='proof', frame=24, **over):
        state = {'phase': 'ready', 'headline': '', 'directed': {
            'scene': scene, 'active': None, 'queued': None, 'suspended': False,
            'shown': {'request_id': request, 'edit_frame': frame}},
            'last_frame': {'request': request, 'revision': request, 'hash': 'h'}}
        state.update(over)
        return state

    def test_a_picture_older_than_the_command_is_not_a_new_picture(self):
        self.assertFalse(native.new_picture(self.state(5), 'proof', 24, after=5)[0])
        self.assertTrue(native.new_picture(self.state(6), 'proof', 24, after=5)[0])
        self.assertTrue(native.new_picture(self.state(1), 'proof', 24, after=-1)[0])

    def test_the_previous_scene_picture_never_satisfies_a_wait_for_another_scene(self):
        self.assertFalse(native.new_picture(self.state(9, scene='proof-b'), 'proof-c', 24, after=5)[0])

    def test_nothing_may_be_in_flight_and_the_read_back_must_be_the_shown_picture(self):
        busy = self.state(6)
        busy['directed']['active'] = {'id': 6}
        self.assertFalse(native.new_picture(busy, 'proof', 24, after=5)[0])
        queued = self.state(6)
        queued['directed']['queued'] = {'explicit': False}
        self.assertFalse(native.new_picture(queued, 'proof', 24, after=5)[0])
        mismatch = self.state(6)
        mismatch['last_frame']['request'] = 5
        self.assertFalse(native.new_picture(mismatch, 'proof', 24, after=5)[0])
        self.assertFalse(native.new_picture(self.state(6, frame=0), 'proof', 24, after=5)[0])
        self.assertFalse(native.new_picture(self.state(6, headline='Falha: x'), 'proof', 24, after=5)[0])
        self.assertFalse(native.new_picture(self.state(6, phase='loading'), 'proof', 24, after=5)[0])

    def test_presented_is_reported_as_not_observable(self):
        self.assertEqual(native.picture(self.state())['presented'], 'not_observable')

    def test_the_mark_of_a_window_that_drew_nothing_is_before_every_request(self):
        self.assertEqual(native.mark({'directed': {}}), -1)
        self.assertEqual(native.mark(self.state(7)), 7)


class Candidate(unittest.TestCase):
    BASE = [{'x': 1.0, 'y': 0.0, 'z': 0.0}, {'w': 1, 'x': 0, 'y': 0, 'z': 0}, {'x': 1, 'y': 1, 'z': 1}]

    def test_a_real_candidate_has_its_own_value_apart_from_the_base_and_would_change(self):
        moved = {'applied': True, 'would_change': True, 'position': {'x': 1.5, 'y': 0.0, 'z': 0.0}}
        self.assertTrue(native.candidate_differs(moved, self.BASE))

    def test_an_empty_candidate_is_not_a_candidate(self):
        same = {'applied': True, 'would_change': True, 'position': {'x': 1.0, 'y': 0.0, 'z': 0.0}}
        self.assertFalse(native.candidate_differs(same, self.BASE))
        self.assertFalse(native.candidate_differs({'applied': True, 'would_change': False,
                                                   'position': {'x': 9, 'y': 0, 'z': 0}}, self.BASE))
        self.assertFalse(native.candidate_differs({'applied': False, 'reason': 'degenerate_pointer'}, self.BASE))
        self.assertFalse(native.candidate_differs(None, self.BASE))

    def test_the_golden_undo_depth_is_compared_with_a_recorded_offset(self):
        self.assertEqual(native.history_offset(1, 0), 1)


class ScriptedWindow(native.Window):
    """A window for `native.drag` whose monitor answers from a script: each `settle` pops the next
    `(ms, state, info)` (`ms=None` is a timeout, which still carries the last state seen, as the real one does)."""

    def __init__(self, settles):
        self.settles = list(settles)
        self.log = []
        self.native = FakeNative()

    ANSWERS = {
        'scene.get': {'value': 0},
        'history.status': {'undo_depth': 0, 'redo_depth': 0},
        'scene.gizmo.state': {'overlay': {'handles': [{'id': 'x', 'to': {'u': 0.0, 'v': 0.0}}]}},
        'scene.pointer.down': {'dragging': True, 'hit': {'handle': 'x'}},
        'scene.pointer.move': {'applied': True, 'document_changed': False, 'would_change': True, 'position': 1.0},
    }

    def op(self, request, timeout=90):
        self.log.append(request['op'])
        return True, self.ANSWERS.get(request['op'], {})

    def settle(self, scene, frame, after=None, seconds=30):
        return self.settles.pop(0)

    def viewport(self):
        return {'directed': {'shown': {'request_id': 9}}}

    def trace(self):
        return {}


def seen(ms, request, picture_hash):
    """One scripted `settle` result: the monitor showing picture `request` whose read-back hash is `picture_hash`."""
    state = {'directed': {'scene': 'proof', 'shown': {'request_id': request, 'edit_frame': 24}},
             'last_frame': {'request': request, 'hash': picture_hash}}
    return ms, state, native.picture(state)


class DragBase(unittest.TestCase):
    """What the pixel checks of a gesture may be compared with: only a picture the monitor converged on."""

    STEP = {'id': 'drag.translate.cancel', 'kind': 'drag', 'phase': 'hot', 'tool': 'translate', 'space': 'world',
            'handle': 'x', 'delta': [0.03, 0.0], 'moves': 2, 'end': 'cancel'}
    BASE_CHECK = 'drag.translate.cancel: the picture before the gesture converged (it is the base of the pixel checks)'
    CANCEL_DREW = 'drag.translate.cancel: the cancel drew a NEW picture (the candidate had been drawn)'
    CANCEL_IS_BASE = 'drag.translate.cancel: after the cancel the picture is the base picture'

    def run_drag(self, settles):
        window = ScriptedWindow(settles)
        checks = {}

        def check(label, condition, detail=''):
            checks[label] = bool(condition)
            return bool(condition)

        def timed(sid, request, timeout=120, phase=None):
            return window.op(request, timeout)

        native.drag(window, timed, check, self.STEP, 'proof', {'frame': 24}, 'n', [], [],
                    {'steps': {self.STEP['id']: {}}}, {})
        return window, checks

    def test_a_base_picture_that_never_converged_stops_the_gesture_before_the_press(self):
        window, checks = self.run_drag([seen(None, 3, 'stale')])
        self.assertEqual(checks, {self.BASE_CHECK: False})  # and nothing else was compared with that picture
        self.assertNotIn('scene.pointer.down', window.log)
        self.assertEqual(window.settles, [])

    def test_a_cancel_that_never_converged_cannot_pass_on_the_stale_hash_of_the_base(self):
        # the last state seen before the timeout still shows the base picture's hash: the old check read it as a pass
        window, checks = self.run_drag([seen(5.0, 1, 'base'), seen(5.0, 2, 'candidate'), seen(None, 2, 'base')])
        self.assertIn('scene.pointer.down', window.log)
        self.assertFalse(checks[self.CANCEL_DREW])
        self.assertFalse(checks[self.CANCEL_IS_BASE])

    def test_a_cancel_that_converged_on_the_base_picture_passes_every_check(self):
        window, checks = self.run_drag([seen(5.0, 1, 'base'), seen(5.0, 2, 'candidate'), seen(5.0, 3, 'base')])
        self.assertEqual([label for label, ok in checks.items() if not ok], [])
        self.assertTrue(checks[self.BASE_CHECK] and checks[self.CANCEL_IS_BASE])
        self.assertIn('scene.pointer.cancel', window.log)

    def test_a_cancel_that_converged_on_another_picture_than_the_base_fails(self):
        window, checks = self.run_drag([seen(5.0, 1, 'base'), seen(5.0, 2, 'candidate'), seen(5.0, 3, 'candidate')])
        self.assertTrue(checks[self.CANCEL_DREW])
        self.assertFalse(checks[self.CANCEL_IS_BASE])

    CANDIDATE_DRAWN = ('drag.translate.cancel: the candidate was drawn while the gesture was open '
                       '(a picture newer than the press, within 2 s)')

    def test_a_candidate_that_differs_but_was_never_drawn_fails_a_check_of_its_own(self):
        # nothing newer than the press showed up: the cancel below would compare the base picture with itself
        window, checks = self.run_drag([seen(5.0, 1, 'base'), seen(None, 1, 'base'), seen(5.0, 1, 'base')])
        self.assertIn('scene.pointer.move', window.log)
        self.assertFalse(checks[self.CANDIDATE_DRAWN])
        self.assertNotIn(self.CANCEL_DREW, checks)  # the check that needs a drawn candidate was not even asked

    def test_a_candidate_that_was_drawn_passes_that_check(self):
        window, checks = self.run_drag([seen(5.0, 1, 'base'), seen(5.0, 2, 'candidate'), seen(5.0, 3, 'base')])
        self.assertTrue(checks[self.CANDIDATE_DRAWN])


class Provenance(unittest.TestCase):
    def setup(self, root):
        root = Path(root)
        (root / 'runtime/dist').mkdir(parents=True)
        (root / 'runtime/dist/runtime.js').write_text('runtime')
        fixtures = root / 'fixtures'
        fixtures.mkdir()
        (fixtures / 'journey.json').write_text('{}')
        manifest = {'files': {}, 'journey': {'sha256': harness.sha256(fixtures / 'journey.json')}}
        (fixtures / 'corpus.manifest.json').write_text(json.dumps(manifest))
        binary = root / 'editor'
        binary.write_text('binary')
        record = {'head': 'a' * 40, 'tree': 'b' * 40, 'dirty': False, 'binary_sha256': harness.sha256(binary),
                  'runtime_sha256': harness.sha256(root / 'runtime/dist/runtime.js'), 'runtime_source_tree': 'rt',
                  'corpus_manifest_sha256': harness.sha256(fixtures / 'corpus.manifest.json')}
        identity = {'head': 'a' * 40, 'tree': 'b' * 40, 'dirty': ''}
        return root, fixtures, binary, record, identity

    def verify(self, root, fixtures, binary, record, identity, **kw):
        # `runtime_stale_reason` asks the git of `root`, a temporary directory here and not a repository: the strict
        # reader refuses that (a git that cannot answer is not "no source is newer"), so this helper stubs it. Its own
        # behaviour is covered by `RuntimeStale`.
        with patch.object(native, 'runtime_source_tree', return_value='rt'), \
                patch.object(native, 'runtime_stale_reason', return_value=None):
            return native.verify_provenance(record, binary, identity, root=root, fixtures=fixtures, **kw)

    def test_the_recorded_build_of_this_head_is_accepted_and_anything_else_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, fixtures, binary, record, identity = self.setup(tmp)
            self.assertIsNone(self.verify(root, fixtures, binary, record, identity))
            self.assertIn('no build record', self.verify(root, fixtures, binary, None, identity))
            self.assertIn('dirty', self.verify(root, fixtures, binary, record, {**identity, 'dirty': ' M x'}))
            self.assertIsNone(self.verify(root, fixtures, binary, record, {**identity, 'dirty': ' M x'},
                                          allow_dirty=True))
            self.assertIn('built for', self.verify(root, fixtures, binary, record, {**identity, 'head': 'c' * 40}))
            self.assertIn('built for', self.verify(root, fixtures, binary, record, {**identity, 'tree': 'd' * 40}))

    def test_another_binary_another_runtime_or_another_corpus_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, fixtures, binary, record, identity = self.setup(tmp)
            binary.write_text('another build')
            self.assertIn('sha256', self.verify(root, fixtures, binary, record, identity))
            binary.write_text('binary')
            (root / 'runtime/dist/runtime.js').write_text('another runtime')
            self.assertIn('runtime/dist', self.verify(root, fixtures, binary, record, identity))
            (root / 'runtime/dist/runtime.js').write_text('runtime')
            (fixtures / 'journey.json').write_text('{"changed": true}')
            self.assertIn('corpus manifest', self.verify(root, fixtures, binary, record, identity))

    def test_two_identities_git_could_not_name_are_not_the_same_head(self):
        # a failing git used to give head '', tree '' and a clean tree, in the record and in the checkout alike: equal
        with tempfile.TemporaryDirectory() as tmp:
            root, fixtures, binary, record, identity = self.setup(tmp)
            nameless = {'head': '', 'tree': '', 'dirty': ''}
            built = {**record, 'head': '', 'tree': ''}
            self.assertIsNotNone(self.verify(root, fixtures, binary, built, nameless))
            self.assertIsNotNone(self.verify(root, fixtures, binary, built, nameless, allow_dirty=True))

    def test_other_runtime_sources_are_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, fixtures, binary, record, identity = self.setup(tmp)
            with patch.object(native, 'runtime_source_tree', return_value='other'):
                self.assertIn('runtime sources', native.verify_provenance(record, binary, identity, root=root,
                                                                           fixtures=fixtures))

    def test_a_record_of_a_dirty_build_or_one_that_does_not_say_is_not_the_build_of_head(self):
        # the tree reads clean (the changes were stashed after `--phase build --allow-dirty`), the build did not
        with tempfile.TemporaryDirectory() as tmp:
            root, fixtures, binary, record, identity = self.setup(tmp)
            silent = {key: value for key, value in record.items() if key != 'dirty'}
            self.assertIn('dirty', self.verify(root, fixtures, binary, {**record, 'dirty': True}, identity))
            self.assertIn('dirty', self.verify(root, fixtures, binary, silent, identity))
            self.assertIsNone(self.verify(root, fixtures, binary, record, identity))
            for allowed in ({**record, 'dirty': True}, silent):  # `--allow-dirty`: accepted (and the run is marked dirty)
                self.assertIsNone(self.verify(root, fixtures, binary, allowed, identity, allow_dirty=True))

    def test_a_runtime_tree_that_git_cannot_name_is_refused_and_never_equal_to_a_record_without_one(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, fixtures, binary, record, identity = self.setup(tmp)
            with patch.object(native, 'runtime_source_tree', return_value=None):
                reason = native.verify_provenance({**record, 'runtime_source_tree': None}, binary, identity, root=root,
                                                  fixtures=fixtures)
            self.assertIn('git cannot tell', reason or '')


class RuntimeTree(unittest.TestCase):
    """`runtime_source_tree` asks the `git` that is first on PATH; here that is a stub script."""

    def tree(self, body):
        with tempfile.TemporaryDirectory() as stubs:
            stub_git(stubs, body)
            with patch.dict(os.environ, {'PATH': f"{stubs}{os.pathsep}{os.environ['PATH']}"}):
                return native.runtime_source_tree()

    def test_the_tree_id_git_prints_is_the_answer(self):
        self.assertEqual(self.tree('echo 0123abcd'), '0123abcd')

    def test_a_git_that_fails_has_no_answer_even_when_it_printed_something(self):
        # `git rev-parse` echoes what it was given when it fails, and `harness.out` turns any failure into a text
        self.assertIsNone(self.tree('echo HEAD:runtime; echo "fatal: bad revision" >&2; exit 128'))

    def test_a_git_that_prints_nothing_or_blank_has_no_answer(self):
        self.assertIsNone(self.tree('exit 0'))
        self.assertIsNone(self.tree('echo "   "; exit 0'))

    def test_a_git_that_cannot_start_has_no_answer(self):
        with tempfile.TemporaryDirectory() as empty, patch.dict(os.environ, {'PATH': empty}):
            self.assertIsNone(native.runtime_source_tree())

    def test_a_git_that_timed_out_has_no_answer(self):
        late = harness.Done(['git'], -9, 'deadbeef\n', '', timed_out=True)
        with patch.object(native.harness, 'run_bounded', return_value=late):
            self.assertIsNone(native.runtime_source_tree())


class BuildPhase(unittest.TestCase):
    """`native.py --phase build`: what it refuses before any cargo, and the record it writes."""

    IDENTITY = {'head': 'a' * 40, 'tree': 'b' * 40, 'branch': 'main', 'dirty': ''}

    def build(self, out, *flags, tree='rt', identity=None, run=None):
        run = run if run is not None else MagicMock(return_value=harness.Done(['cargo'], 1, '', ''))
        with patch.object(native.harness, 'git_identity', return_value=identity or self.IDENTITY), \
                patch.object(native.harness, 'toolchain', return_value={}), \
                patch.object(native.harness, 'run_bounded', run), \
                patch.object(native.harness, 'sha256', return_value='sha'), \
                patch.object(native, 'runtime_stale_reason', return_value=None), \
                patch.object(native, 'verify_corpus', return_value=None), \
                patch.object(native, 'runtime_source_tree', return_value=tree):
            code = run_main(native, ['--phase', 'build', '--out', str(out), *flags])
        return code, json.loads((Path(out) / 'manifest.build.json').read_text()), run

    def test_a_build_that_could_not_record_the_runtime_tree_is_refused_before_building(self):
        with tempfile.TemporaryDirectory() as out:
            code, manifest, run = self.build(out, tree=None)
            self.assertEqual((code, manifest['status']), (1, 'PREREQUISITE_MISSING'))
            self.assertIn('git cannot tell', manifest['reason'])
            run.assert_not_called()

    def test_the_build_gets_no_target_directory_and_no_perf_switch_from_the_caller(self):
        run = MagicMock(return_value=harness.Done(['cargo'], 1, '', ''))
        inherited = {'TESSEL_GIZMO_PERF_CALIBRATE': '1', 'TESSEL_GIZMO_PERF_BLESS': '1', 'CARGO_TARGET_DIR': '/t',
                     'TESSEL_CARGO_LANE': 'lane'}
        with tempfile.TemporaryDirectory() as out, patch.dict(os.environ, inherited):
            self.build(out, run=run)
            env = run.call_args.kwargs.get('env')
            env = dict(os.environ) if env is None else env  # `None` is "inherit everything", the caller's included
            leaked = sorted(name for name in inherited if name in env)
        self.assertEqual(leaked, [])  # names only: `env` itself is never printed

    def test_a_build_whose_executable_is_gone_is_a_recorded_refusal_not_a_traceback(self):
        gone = harness.Done(['cargo'], 0, json.dumps({'executable': '/nowhere/pruned/tessel-editor',
                                                      'target': {'name': 'tessel-editor', 'kind': ['bin']}}) + '\n', '')
        with tempfile.TemporaryDirectory() as out:
            code, manifest, _ = self.build(out, run=MagicMock(return_value=gone))
            self.assertEqual((code, manifest['status']), (1, 'INFRA_EXECUTABLE_MISSING'))
            self.assertEqual(manifest['missing'], '<abs>/tessel-editor')  # the record is redacted (`harness.redact`)
            self.assertFalse((Path(out) / 'build.json').exists())  # no record of a binary that was never copied

    def test_the_build_record_keeps_whether_the_tree_was_dirty(self):
        with tempfile.TemporaryDirectory() as src:
            exe = Path(src) / 'tessel-editor'
            exe.write_text('binary')
            line = json.dumps({'executable': str(exe), 'target': {'name': 'tessel-editor', 'kind': ['bin']}})
            built = harness.Done(['cargo'], 0, line + '\n', '')
            for flags, identity, dirty in (((), self.IDENTITY, False),
                                           (('--allow-dirty',), {**self.IDENTITY, 'dirty': ' M crates/core/src/x.rs'}, True)):
                with tempfile.TemporaryDirectory() as out:
                    code, manifest, _ = self.build(out, *flags, identity=identity, run=MagicMock(return_value=built))
                    self.assertEqual((code, manifest['status']), (0, 'BUILT'))
                    record = json.loads((Path(out) / 'build.json').read_text())
                    self.assertIs(record['dirty'], dirty)
                    self.assertEqual(record['runtime_source_tree'], 'rt')


class MainRun(unittest.TestCase):
    """`native.py --phase run` on fakes: the window is not opened, the corpus run is scripted."""

    IDENTITY = {'head': 'a' * 40, 'tree': 'b' * 40, 'branch': 'main', 'dirty': ''}

    def run_with(self, record, *flags):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            (tmp / 'runtime/dist').mkdir(parents=True)
            (tmp / 'runtime/dist/runtime.js').write_text('runtime')
            binary = tmp / 'editor'
            binary.write_text('binary')
            record_path = tmp / 'build.json'
            record_path.write_text(json.dumps(record))

            def corpus(*_):
                return {'corpus': 'small', 'checks': [{'check': 'a check', 'ok': True}], 'pictures': [], 'samples': []}

            state = {'free_gib': 100, 'loadavg': (0.0, 0.0, 0.0)}
            with patch.object(native, 'ROOT', tmp), patch.object(native, 'verify_provenance', return_value=None), \
                    patch.object(native, 'run_corpus', side_effect=corpus), \
                    patch.object(native.harness, 'git_identity', return_value=self.IDENTITY), \
                    patch.object(native.harness, 'toolchain', return_value={}), \
                    patch.object(native.harness, 'guards', return_value=(state, None)), \
                    patch.object(native.harness, 'claims', return_value=[]), \
                    patch.object(native.harness, 'machine', return_value={}):
                code = run_main(native, ['--phase', 'run', '--out', str(tmp / 'out'), '--binary', str(binary),
                                         '--build-record', str(record_path), '--functional-only', *flags])
            return code, json.loads((tmp / 'out' / 'manifest.json').read_text())

    def test_a_run_of_no_corpus_is_refused_and_never_says_that_no_check_failed(self):
        with tempfile.TemporaryDirectory() as out:
            code = run_main(native, ['--corpora', '--out', out])
            manifest = json.loads((Path(out) / 'manifest.json').read_text())
            self.assertEqual((code, manifest['status']), (1, 'REFUSED'))
            self.assertIn('--corpora', manifest['reason'])

    def test_a_run_of_a_binary_from_a_dirty_build_is_never_a_pass_for_head(self):
        for label, record in (('dirty', {'dirty': True}), ('not said', {})):
            code, manifest = self.run_with(record)
            self.assertEqual((code, manifest['status']), (1, 'DIRTY_RUN_NOT_FOR_HEAD'), label)
            self.assertTrue(manifest['dirty_run'], label)
        code, manifest = self.run_with({'dirty': False})  # the control: the same run with a clean build is a functional pass
        self.assertEqual((code, manifest['status']), (0, 'FUNCTIONAL_ONLY_NOT_A_MEASUREMENT'))


    def test_a_failing_run_carries_the_guard_that_failed_after_it_and_stays_a_fail(self):
        # (F18) the same sentence for the native window: the status stays FAIL, the reason also holds the guard text
        guard = 'load average (1 min) 5.88 over 5.5 (11 cpus)'
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            (tmp / 'runtime/dist').mkdir(parents=True)
            (tmp / 'runtime/dist/runtime.js').write_text('runtime')
            binary = tmp / 'editor'
            binary.write_text('binary')
            record_path = tmp / 'build.json'
            record_path.write_text(json.dumps({'dirty': False}))
            quiet = {'free_gib': 100, 'loadavg': (0.0, 0.0, 0.0)}

            def corpus(*_):
                return {'corpus': 'small', 'checks': [{'check': 'a check that failed', 'ok': False}], 'pictures': [],
                        'samples': []}

            with patch.object(native, 'ROOT', tmp), patch.object(native, 'verify_provenance', return_value=None), \
                    patch.object(native, 'run_corpus', side_effect=corpus), \
                    patch.object(native.harness, 'git_identity', return_value=self.IDENTITY), \
                    patch.object(native.harness, 'toolchain', return_value={}), \
                    patch.object(native.harness, 'guards', return_value=(quiet, None)), \
                    patch.object(native.harness, 'claims', return_value=[]), \
                    patch.object(native.harness, 'machine', return_value={}), \
                    patch.object(native.harness, 'Lock', FakeLock), \
                    patch.object(native.harness, 'judge_after', side_effect=lambda manifest: guard):
                code = run_main(native, ['--phase', 'run', '--out', str(tmp / 'out'), '--binary', str(binary),
                                         '--build-record', str(record_path)])
            manifest = json.loads((tmp / 'out' / 'manifest.json').read_text())
            self.assertEqual((code, manifest['status']), (1, 'FAIL'))
            self.assertIn('a check that failed', manifest['reason'])
            self.assertIn(guard, manifest['reason'])


class Mutants(unittest.TestCase):
    def test_every_anchor_is_unique_in_the_tree_so_a_mutant_cannot_silently_not_apply(self):
        for name, mutant in mutate.MUTANTS.items():
            text = (harness.ROOT / mutant['file']).read_text()
            self.assertEqual(text.count(mutant['old']), 1, name)
            self.assertNotEqual(mutant['old'], mutant['new'], name)

    def test_the_delay_mutant_is_one_the_structural_detector_must_not_see(self):
        self.assertEqual(mutate.MUTANTS['delay_1ms_per_handler']['detectors'], {'structural': False, 'time': True})

    def test_a_mutated_file_is_restored_even_when_the_body_fails(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / 'f.rs'
            path.write_text('fn a() {}\n')
            mutant = {'file': str(path), 'old': 'fn a() {}', 'new': 'fn a() { loop {} }'}
            with patch.object(mutate, 'ROOT', Path('/')), patch.object(mutate, 'file_hash', return_value='x'), \
                    patch.object(mutate, 'committed_hash', return_value='x'):
                with self.assertRaises(RuntimeError):
                    with mutate.Mutated(mutant):
                        self.assertIn('loop', path.read_text())
                        raise RuntimeError('the detector crashed')
            self.assertEqual(path.read_text(), 'fn a() {}\n')

    def test_a_mutant_is_put_back_when_entering_it_fails_because_the_with_statement_never_exits(self):
        # `__exit__` does not run when `__enter__` raises: the mutant was already written when git could not name its blobs
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / 'f.rs'
            path.write_text('fn a() {}\n')
            mutant = {'file': str(path), 'old': 'fn a() {}', 'new': 'fn a() { loop {} }'}
            with patch.object(mutate, 'ROOT', Path('/')), patch.object(mutate, 'file_hash', return_value='x'), \
                    patch.object(mutate, 'committed_hash', side_effect=RuntimeError('git is unavailable')):
                with self.assertRaises(RuntimeError):
                    with mutate.Mutated(mutant):
                        self.fail('the body of the with statement must not run')
            self.assertEqual(path.read_text(), 'fn a() {}\n')


class TimePhase(unittest.TestCase):
    """The `time` phase reads the manifest a finished `prepare` left in --out. Nothing it refuses may replace that
    manifest, and nothing it cannot evaluate may come out as "every detector answered as expected"."""

    IDENTITY = {'head': 'h' * 40, 'tree': 't' * 40, 'branch': 'b', 'dirty': ''}

    def prepared(self, out, status='PREPARED', names=('memo_off',)):
        manifest = {'suite': 'gizmo-perf-mutants', 'phase': 'prepare', 'status': status, 'identity': self.IDENTITY,
                    'mutants': {name: {'file': mutate.MUTANTS[name]['file'], 'must': mutate.MUTANTS[name]['detectors'],
                                       'host': {'outcome': 'KILLED'}} for name in names}}
        (Path(out) / 'manifest.json').write_text(json.dumps(manifest))
        return manifest

    def time_phase(self, out, *only, edited=False):
        """`main --phase time` (files at HEAD, or not: `edited`; a machine nobody asks anything). Returns the exit code
        and `manifest.final.json`, `{}` when it was not written."""
        state = {'loadavg': (0.0, 0.0, 0.0)}
        with patch.object(mutate.harness, 'git_identity', return_value=self.IDENTITY), \
                patch.object(mutate.harness, 'toolchain', return_value={}), \
                patch.object(mutate.harness, 'machine', return_value=state), \
                patch.object(mutate.harness, 'guards', return_value=(state, None)), \
                patch.object(mutate.harness, 'claims', return_value=[]), \
                patch.object(mutate, 'file_hash', return_value='edited' if edited else 'blob'), \
                patch.object(mutate, 'committed_hash', return_value='blob'):
            code = run_main(mutate, ['--phase', 'time', '--out', str(out), *(['--only', *only] if only else [])])
        final = Path(out) / 'manifest.final.json'
        return code, json.loads(final.read_text()) if final.exists() else {}

    def test_a_refusal_of_the_time_phase_never_replaces_the_finished_prepare(self):
        with tempfile.TemporaryDirectory() as out:
            prepared = self.prepared(out)
            code, refusal = self.time_phase(out, 'memo_off', edited=True)  # the file is not at HEAD
            self.assertEqual((code, refusal.get('status')), (1, 'REFUSED'))
            self.assertEqual(json.loads((Path(out) / 'manifest.json').read_text()), prepared)  # still the prepare's
            # the file is put back and `time` runs again: it judges what `prepare` ran, it is not a pass with no mutant
            code, final = self.time_phase(out, 'memo_off')
            self.assertEqual((code, final.get('status')), (0, 'ALL_AS_EXPECTED'))
            self.assertEqual(sorted(final.get('mutants', {})), ['memo_off'])

    def test_a_mutant_that_prepare_did_not_run_is_refused_and_never_filtered_away(self):
        with tempfile.TemporaryDirectory() as out:
            self.prepared(out)  # only `memo_off`
            code, refusal = self.time_phase(out, 'replan_per_tick')  # filtered by name this left no mutant at all
            self.assertEqual((code, refusal.get('status')), (1, 'REFUSED'))
            self.assertIn('replan_per_tick', refusal.get('reason', ''))
            code, refusal = self.time_phase(out, 'memo_off', 'replan_per_tick')  # one of two is not "judge the other"
            self.assertEqual((code, refusal.get('status')), (1, 'REFUSED'))
            self.assertIn('replan_per_tick', refusal.get('reason', ''))

    def test_a_manifest_that_is_not_a_finished_prepare_is_refused(self):
        with tempfile.TemporaryDirectory() as out:
            self.prepared(out, status='REFUSED', names=())  # what a refused `prepare` leaves: no mutants, same identity
            code, refusal = self.time_phase(out, 'memo_off')
            self.assertEqual((code, refusal.get('status')), (1, 'REFUSED'))
            self.assertIn('not a finished', refusal.get('reason', ''))

    def test_a_verdict_over_no_mutant_and_no_detector_is_never_a_pass(self):
        for manifest in ({'mutants': {}}, {'mutants': {'memo_off': {'must': {}}}}):
            status, code = mutate.overall(manifest)
            self.assertNotEqual(status, 'ALL_AS_EXPECTED')
            self.assertNotEqual(code, 0)


class MutantExecutables(unittest.TestCase):
    """The executables of the mutants the time detector judges travel from `prepare` to `time` through --out/exes, and
    the only thing that says a file there is the one `prepare` built is the hash `prepare` recorded for it. Everything
    around that hand-over is faked: git, the machine, the performance lock, the release build (it names the bytes of an
    executable, or none) and the controlled run, which only notes in `self.measured` the bytes it was asked to measure.
    The lock on --out itself (`--out/.gizmo-perf.lock`, which keeps a second run from touching the directory while one owns
    it) is NOT faked: it is a real `flock` in a temporary directory, which the tests also take, through a second
    `open()` of the file, the way another process of any runner would (an `flock` conflicts across two open file
    descriptions even inside one process). The mutated file is one in a temporary directory, never the repository's."""

    NAME = 'time_mutant'
    A = {'head': 'a' * 40, 'tree': '1' * 40, 'branch': 'b', 'dirty': ''}
    B = {'head': 'b' * 40, 'tree': '2' * 40, 'branch': 'b', 'dirty': ''}
    LOADED = 'load average (1 min) 9.00 over 5.5 (11 cpus)'

    def phase(self, root, phase, identity=None, built=None, build_exit=0, blocked=None, edited=False,
              time_outcome='KILLED', only=(), extra=None, git_down=False, vanish=False, on_build=None):
        """`main --phase prepare|time --out ROOT/out` for the one mutant, at HEAD `identity`. `built`: the bytes of the
        executable its release build names, `None` for a build that named none (and failed); `build_exit` is that
        build's exit; `blocked` the reason the machine guard refuses a measurement; `edited`: the mutated file is not
        at HEAD. Returns the exit code and the manifest the phase wrote (`manifest.json` of `prepare`,
        `manifest.final.json` of `time`; `{}` if none). What the runner printed is left in `self.printed`, the number
        of times it asked for the performance lock in `self.performance_locks` and of release builds in `self.builds`.

        `time_outcome`: what the faked controlled run answers; `only`: the `--only` names; `extra`: more mutants for the
        runner to know (name -> mutant); `git_down`: git is NOT faked, the only `git` on PATH is one that fails (the
        identity and the file hashes are then the runner's own reading of a git that cannot answer); `vanish`: the
        executable the release build names is gone when the runner reads it (a pruned managed target); `on_build`: called
        (no argument) each time the release build runs, to look at what the runner holds while it builds."""
        root = Path(root)
        out = root / 'out'
        source = root / 'f.rs'
        source.write_text('fn a() {}\n')
        state = {'loadavg': (0.0, 0.0, 0.0)}
        self.measured = []
        self.printed = io.StringIO()
        self.performance_locks = 0
        self.builds = 0

        def release_build():
            self.builds += 1
            if on_build:
                on_build()
            if built is None:
                return ['cargo', 'test', '--release'], done('error[E0425]: cannot find value', 101), None
            target = root / 'target-exe'
            target.write_bytes(built)
            if vanish:
                target.unlink()
            return ['cargo', 'test', '--release'], done('', build_exit), str(target)

        def controlled_run(exe, report, directory, name):
            self.measured.append((name, Path(exe).read_bytes()))
            return {'outcome': time_outcome, 'status': time_outcome, 'detail': 'faked: nothing was executed'}

        def performance_lock():
            self.performance_locks += 1
            return FakeLock()

        mutants = {self.NAME: {'file': str(source), 'old': 'fn a() {}', 'new': 'fn a() { loop {} }',
                               'detectors': {'time': True}}, **(extra or {})}
        fakes = [patch.object(mutate, 'MUTANTS', mutants), patch.object(mutate, 'ROOT', Path('/')),
                 patch.object(mutate, 'run_time', side_effect=controlled_run),
                 patch.object(mutate.runner, 'build', side_effect=release_build),
                 patch.object(mutate.harness, 'toolchain', return_value={}),
                 patch.object(mutate.harness, 'machine', return_value=state),
                 patch.object(mutate.harness, 'guards', return_value=(state, blocked)),
                 patch.object(mutate.harness, 'claims', return_value=[]),
                 patch.object(mutate.harness, 'Lock', side_effect=performance_lock)]
        if git_down:
            stubs = root / 'git-is-down'
            stubs.mkdir(exist_ok=True)
            stub_git(stubs, 'echo "fatal: not a git repository (or any of the parent directories): .git" >&2; exit 128')
            fakes.append(patch.dict(os.environ, {'PATH': f"{stubs}{os.pathsep}{os.environ['PATH']}"}))
        else:
            fakes += [patch.object(mutate, 'file_hash', return_value='edited' if edited else 'blob'),
                      patch.object(mutate, 'committed_hash', return_value='blob'),
                      patch.object(mutate.harness, 'git_identity', return_value=identity or self.A)]

        def waited(signum, frame):
            raise AssertionError('the run waited instead of finishing or refusing: it must never wait for a lock')

        with contextlib.ExitStack() as stack:
            for fake in fakes:
                stack.enter_context(fake)
            # Everything slow is faked, so a run still going after this long is waiting, and the only thing it could wait
            # for is a lock it must not wait for: a hang becomes a failure.
            stack.callback(signal.signal, signal.SIGALRM, signal.signal(signal.SIGALRM, waited))
            stack.callback(signal.alarm, 0)
            signal.alarm(20)
            code = run_main(mutate, ['--phase', phase, '--out', str(out), *(['--only', *only] if only else [])],
                            self.printed)
        written = out / ('manifest.json' if phase == 'prepare' else 'manifest.final.json')
        return code, json.loads(written.read_text()) if written.exists() else {}

    def finished_prepare(self, out, recorded, identity=None, extra=None):
        """What a finished `prepare` left in --out/manifest.json for the mutant: its build recorded `recorded` as the
        hash of the executable (`None`: the build named none). The executable itself is the caller's. `extra`: the
        records of more mutants `prepare` ran (name -> record)."""
        manifest = {'suite': 'gizmo-perf-mutants', 'phase': 'prepare', 'status': 'PREPARED',
                    'identity': identity or self.A,
                    'mutants': {self.NAME: {'file': 'f.rs', 'must': {'time': True},
                                            'time_build': {'exit': 0, 'executable_sha256': recorded}},
                                **(extra or {})}}
        (Path(out) / 'manifest.json').write_text(json.dumps(manifest))

    def leave(self, out, content):
        """`out/exes/<mutant>.exe` as an earlier run left it; returns the path."""
        exe = Path(out) / 'exes' / f'{self.NAME}.exe'
        exe.parent.mkdir(parents=True, exist_ok=True)
        exe.write_bytes(content)
        return exe

    @contextlib.contextmanager
    def another_run_owns(self, out):
        """`out/.gizmo-perf.lock` held the way a second process on the same --out would hold it (of `mutate.py` or of any
        other runner: the name is shared): the runner's own `open()` of that file conflicts with this one. Leaving the
        block releases it; `BlockingIOError` here means a run that had finished still held the lock."""
        Path(out).mkdir(parents=True, exist_ok=True)
        with open(Path(out) / SHARED_LOCK, 'a') as handle:
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            yield

    @staticmethod
    def contents(out):
        """Every file and directory under `out`, with the bytes of the files (`None` for a directory): what a run that
        was refused must leave exactly as it found it."""
        return {p.relative_to(out).as_posix(): p.read_bytes() if p.is_file() else None
                for p in sorted(Path(out).rglob('*'))}

    def test_a_stale_executable_with_no_hash_recorded_for_it_is_never_measured(self):
        with tempfile.TemporaryDirectory() as root:
            out = Path(root) / 'out'
            out.mkdir()
            self.finished_prepare(out, None)  # this `prepare`'s build named no executable, so it recorded no hash
            self.leave(out, b'built by an earlier prepare, for another head')
            code, final = self.phase(root, 'time')
            answer = final['mutants'][self.NAME]['time']
            self.assertEqual((code, final['status']), (harness.BLOCKED_EXIT, 'BLOCKED_OR_INFRA'))
            self.assertEqual(answer['outcome'], 'MISSING_EXECUTABLE')
            self.assertIn('no hash', answer['detail'])
            self.assertEqual(self.measured, [])  # the file was there, and nothing about this head was run

    def test_only_the_file_whose_hash_prepare_recorded_is_measured(self):
        built = b'what prepare built'
        with tempfile.TemporaryDirectory() as root:
            out = Path(root) / 'out'
            out.mkdir()
            self.leave(out, built)
            self.finished_prepare(out, hashlib.sha256(b'another file').hexdigest())  # a hash, but not this file's
            code, final = self.phase(root, 'time')
            answer = final['mutants'][self.NAME]['time']
            self.assertEqual((code, answer['outcome']), (harness.BLOCKED_EXIT, 'MISSING_EXECUTABLE'))
            self.assertIn('changed since', answer['detail'])
            self.assertEqual(self.measured, [])
            # the control: the same file with ITS hash on record is the one that is measured (a check that refused
            # every executable would pass the half above)
            self.finished_prepare(out, hashlib.sha256(built).hexdigest())
            code, final = self.phase(root, 'time')
            self.assertEqual(self.measured, [(self.NAME, built)])
            self.assertEqual((code, final['status']), (0, 'ALL_AS_EXPECTED'))

    def test_a_prepare_whose_build_gave_no_executable_leaves_none_of_an_earlier_prepare(self):
        with tempfile.TemporaryDirectory() as root:
            out = Path(root) / 'out'
            self.assertEqual(self.phase(root, 'prepare', self.A, built=b'product of A')[0], 0)
            exe = out / 'exes' / f'{self.NAME}.exe'
            self.assertEqual(exe.read_bytes(), b'product of A')
            (exe.parent / 'another_mutant.exe').write_bytes(b'a mutant this prepare does not run')
            # the same --out for the next head: the release build of the mutant named no executable
            code, prepared = self.phase(root, 'prepare', self.B, built=None)
            self.assertEqual(code, 0)  # `prepare` still finishes: the `time` answer is what says it is missing
            self.assertIsNone(prepared['mutants'][self.NAME]['time_build']['executable_sha256'])
            self.assertEqual(sorted(p.name for p in exe.parent.iterdir()), [])
            # a build that named an executable but did not succeed (a timeout after the link) copies nothing either
            self.leave(out, b'product of A')
            code, prepared = self.phase(root, 'prepare', self.B, built=b'product of B', build_exit=-9)
            self.assertEqual(sorted(p.name for p in exe.parent.iterdir()), [])
            # the control: a build that did succeed leaves exactly its own executable, with its hash on record
            code, prepared = self.phase(root, 'prepare', self.B, built=b'product of B')
            self.assertEqual(exe.read_bytes(), b'product of B')
            self.assertEqual(prepared['mutants'][self.NAME]['time_build']['executable_sha256'],
                             hashlib.sha256(b'product of B').hexdigest())

    def test_the_executable_of_one_head_is_never_measured_as_the_product_of_the_next(self):
        # the sequence of the review finding, end to end
        with tempfile.TemporaryDirectory() as root:
            out = Path(root) / 'out'
            exe = out / 'exes' / f'{self.NAME}.exe'
            self.phase(root, 'prepare', self.A, built=b'product of A')  # 1. prepare at head A
            code, final = self.phase(root, 'time', self.A, blocked=self.LOADED)  # 2. a time run that is BLOCKED...
            self.assertEqual(final['mutants'][self.NAME]['time']['outcome'], 'BLOCKED')
            self.assertEqual(exe.read_bytes(), b'product of A')  # ...keeps the executable for a retry
            self.assertEqual(self.measured, [])
            self.phase(root, 'prepare', self.B, built=None)  # 3. prepare at head B, same --out: no executable
            code, final = self.phase(root, 'time', self.B)  # 4. time at head B passes the identity check...
            self.assertEqual(final['mutants'][self.NAME]['time']['outcome'], 'MISSING_EXECUTABLE')
            self.assertEqual(self.measured, [])  # ...and must not measure the product of head A for it
            self.assertEqual(final['prepare_identity'], self.B)

    # --- one run per --out: a `prepare` and a `time` on the same directory at the same time -------------------------

    def test_a_prepare_that_finds_the_out_directory_owned_is_refused_and_changes_nothing(self):
        with tempfile.TemporaryDirectory() as root:
            out = Path(root) / 'out'
            self.leave(out, b'what the owner is building')
            (out / 'exes' / 'stale.exe').write_bytes(b'planted before')  # `prepare` empties exes/: not this one
            with self.another_run_owns(out):
                before = self.contents(out)
                code, written = self.phase(root, 'prepare', self.A, built=b'product of A')
                self.assertEqual((code, written), (1, {}))  # exit 1, and no manifest.json
                self.assertEqual(self.contents(out), before)  # exes/ not emptied, nothing created or changed
                self.assertEqual((out / 'exes' / 'stale.exe').read_bytes(), b'planted before')
                refusal = json.loads(self.printed.getvalue())  # one JSON line
                self.assertEqual(refusal['status'], 'REFUSED')
                self.assertIn(SHARED_LOCK, refusal['reason'])
            # the control: the same directory, once the owner is gone, is prepared as always (a refusal that stuck
            # would pass the half above)
            code, prepared = self.phase(root, 'prepare', self.A, built=b'product of A')
            self.assertEqual((code, prepared['status']), (0, 'PREPARED'))
            self.assertEqual(sorted(p.name for p in (out / 'exes').iterdir()), [f'{self.NAME}.exe'])

    def test_a_time_run_that_finds_the_out_directory_owned_is_refused_measures_nothing_and_changes_nothing(self):
        built = b'what prepare built'
        with tempfile.TemporaryDirectory() as root:
            out = Path(root) / 'out'
            self.leave(out, built)
            self.finished_prepare(out, hashlib.sha256(built).hexdigest())  # everything `time` needs is there
            with self.another_run_owns(out):
                before = self.contents(out)
                code, final = self.phase(root, 'time', self.A)
                self.assertEqual((code, final), (1, {}))  # exit 1, and no manifest.final.json
                self.assertEqual(self.contents(out), before)  # manifest.json and the executable as the owner left them
                self.assertEqual(self.measured, [])  # not measured...
                self.assertEqual(self.performance_locks, 0)  # ...and refused before the performance lock was asked for
                refusal = json.loads(self.printed.getvalue())
                self.assertEqual(refusal['status'], 'REFUSED')
                self.assertIn(SHARED_LOCK, refusal['reason'])
            # the control: the same directory, free, is measured (the executable is the one `prepare` hashed)
            code, final = self.phase(root, 'time', self.A)
            self.assertEqual((code, final['status']), (0, 'ALL_AS_EXPECTED'))
            self.assertEqual(self.measured, [(self.NAME, built)])
            self.assertEqual(self.performance_locks, 1)

    def test_the_out_lock_is_released_when_a_run_ends_and_its_file_stays(self):
        with tempfile.TemporaryDirectory() as root:
            out = Path(root) / 'out'
            lock = out / SHARED_LOCK
            self.assertEqual(self.phase(root, 'prepare', self.A, built=b'product of A')[0], 0)
            # the lock is released by closing the file, not by removing it: with the file gone, two runs would each lock
            # "the" lock file, on a different inode
            self.assertTrue(lock.is_file())
            with self.another_run_owns(out):  # the finished `prepare` no longer holds it...
                pass
            code, final = self.phase(root, 'time', self.A)  # ...so a second run proceeds
            self.assertEqual((code, final['status']), (0, 'ALL_AS_EXPECTED'))
            self.assertEqual(self.measured, [(self.NAME, b'product of A')])
            self.assertTrue(lock.is_file())
            with self.another_run_owns(out):  # nor does the `time` run keep it
                pass

    def test_a_run_refused_for_the_out_directory_does_not_even_write_its_other_refusals(self):
        # The first thing either phase can refuse for is a mutated file that is not at HEAD, and that refusal writes
        # `manifest.json` (prepare) or `manifest.final.json` (time): the lock has to come before it.
        for phase, manifest in (('prepare', 'manifest.json'), ('time', 'manifest.final.json')):
            with self.subTest(phase=phase), tempfile.TemporaryDirectory() as root:
                out = Path(root) / 'out'
                self.leave(out, b'what the owner is building')
                with self.another_run_owns(out):
                    before = self.contents(out)
                    code, written = self.phase(root, phase, self.A, built=b'product of A', edited=True)
                    self.assertEqual((code, written), (1, {}))
                    self.assertFalse((out / manifest).exists())
                    self.assertEqual(self.contents(out), before)
                    self.assertIn(SHARED_LOCK, json.loads(self.printed.getvalue())['reason'])
                # the control: with the directory free the same run refuses for the file, and that refusal IS written
                code, written = self.phase(root, phase, self.A, built=b'product of A', edited=True)
                self.assertEqual((code, written.get('status')), (1, 'REFUSED'))
                self.assertIn('not at HEAD', written['reason'])

    # --- round 6: a clean tree for both phases, an identity git named, the executables that stay ----------------------

    def test_a_prepare_on_a_dirty_tree_is_refused_before_it_mutates_or_builds_anything(self):
        # (F7) the mutants are built from the whole checkout: a dirty tree elsewhere is built into the executables
        dirty = {**self.A, 'dirty': ' M scripts/gizmo-perf/run.py'}
        with tempfile.TemporaryDirectory() as root:
            code, written = self.phase(root, 'prepare', identity=dirty, built=b'product of a dirty tree')
            self.assertEqual((code, written.get('status')), (1, 'REFUSED'))
            self.assertIn('dirty', written['reason'])
            self.assertEqual(self.builds, 0)
            self.assertEqual((Path(root) / 'f.rs').read_text(), 'fn a() {}\n')  # the mutant was never written
            self.assertFalse((Path(root) / 'out' / 'exes').exists())  # and nothing was created or emptied
            # the control: the same prepare on a clean tree builds
            code, written = self.phase(root, 'prepare', identity=self.A, built=b'product')
            self.assertEqual((code, written['status'], self.builds), (0, 'PREPARED', 1))

    def test_a_time_run_refuses_a_prepare_that_ran_on_a_dirty_tree_or_does_not_say_it_did_not(self):
        built = b'what prepare built'
        no_field = {key: value for key, value in self.A.items() if key != 'dirty'}
        for label, identity in (('dirty', {**self.A, 'dirty': ' M crates/core/src/x.rs'}), ('not recorded', no_field)):
            with self.subTest(label), tempfile.TemporaryDirectory() as root:
                out = Path(root) / 'out'
                out.mkdir()
                exe = self.leave(out, built)
                self.finished_prepare(out, hashlib.sha256(built).hexdigest(), identity=identity)
                code, final = self.phase(root, 'time', self.A)  # the tree reads clean now: that does not clean the build
                self.assertEqual((code, final.get('status')), (1, 'REFUSED'))
                self.assertIn('dirty', final['reason'])
                self.assertEqual(self.measured, [])
                self.assertTrue(exe.exists())  # refused before anything was measured or removed

    def test_a_time_run_on_a_dirty_tree_is_refused_before_it_measures_or_removes_anything(self):
        # (R4, round 8) the controlled run reads `budgets.json`, the fixtures and the runners from the tree AT RUN TIME. A tree
        # edited since a clean `prepare` gave a verdict that was not HEAD's, and only `prepare` ever looked at the tree
        built = b'what prepare built'
        for label, line in (('a runner', ' M scripts/gizmo-perf/run.py'),
                            ('budgets.json', ' M docs/migration/fixtures/gizmo-perf/budgets.json'),
                            ('the runtime', ' M runtime/src/host.ts'),
                            ('an untracked build input', '?? crates/core/src/new.rs')):
            with self.subTest(label), tempfile.TemporaryDirectory() as root:
                out = Path(root) / 'out'
                exe = self.leave(out, built)
                self.finished_prepare(out, hashlib.sha256(built).hexdigest())  # a CLEAN prepare: its executables are HEAD's
                before = self.contents(out)
                code, final = self.phase(root, 'time', identity={**self.A, 'dirty': line})
                self.assertEqual((code, final.get('status')), (1, 'REFUSED'))
                self.assertIn('working tree is dirty', final['reason'])
                self.assertIn(line.strip(), final['reason'])  # the first line git printed
                self.assertIn('budgets.json', final['reason'])  # and why a time verdict depends on the tree
                self.assertEqual(self.measured, [])
                self.assertEqual(self.performance_locks, 0)  # refused before the machine was asked for anything
                self.assertEqual({name: data for name, data in self.contents(out).items()
                                  if name not in ('manifest.final.json', SHARED_LOCK)},
                                 before)  # only the refusal record and the lock file are new; the executable and manifest stay
                self.assertTrue(exe.exists())
                # the control: the same `time` run on a clean tree measures (a refusal that stuck would pass the half above)
                code, final = self.phase(root, 'time', self.A)
                self.assertEqual((code, final['status']), (0, 'ALL_AS_EXPECTED'))
                self.assertEqual(self.measured, [(self.NAME, built)])

    def test_each_phase_refuses_a_dirty_tree_in_its_own_words_and_the_current_tree_is_judged_first(self):
        dirty = {**self.A, 'dirty': ' M scripts/gizmo-perf/run.py'}
        with tempfile.TemporaryDirectory() as root:
            code, prepared = self.phase(root, 'prepare', identity=dirty, built=b'x')
            self.assertEqual((code, prepared['status']), (1, 'REFUSED'))
            self.assertIn('executables of the mutants would be built from it', prepared['reason'])
            self.assertNotIn('time verdict', prepared['reason'])
            built = b'what prepare built'
            out = Path(root) / 'out'
            self.leave(out, built)
            self.finished_prepare(out, hashlib.sha256(built).hexdigest(), identity=dirty)  # a dirty prepare AND a dirty tree
            code, final = self.phase(root, 'time', identity=dirty)
            self.assertEqual((code, final['status']), (1, 'REFUSED'))
            self.assertIn('time verdict', final['reason'])  # the tree as it is now comes first...
            self.assertNotIn('`prepare` ran on a dirty', final['reason'])
            code, final = self.phase(root, 'time', identity=self.A)  # ...and the prepare manifest is still checked after it
            self.assertIn('`prepare` ran on a dirty', final['reason'])

    def test_prepare_holds_the_shared_lock_while_it_builds_and_the_build_does_not_refuse_it(self):
        # (R3, round 8) one name for every runner must not make a runner conflict with itself: `mutate.py` takes the lock once,
        # in `main`, and `run.build()` (which it calls) is a plain function. Looked at from inside the build.
        with tempfile.TemporaryDirectory() as root:
            out = Path(root) / 'out'
            held_while_building = []

            def while_building():
                with open(out / SHARED_LOCK, 'a') as other:  # somebody else's open() of the lock file
                    try:
                        fcntl.flock(other, fcntl.LOCK_EX | fcntl.LOCK_NB)
                        held_while_building.append(False)
                    except BlockingIOError:
                        held_while_building.append(True)

            code, prepared = self.phase(root, 'prepare', self.A, built=b'product of A', on_build=while_building)
            self.assertEqual((code, prepared['status']), (0, 'PREPARED'))
            self.assertEqual(held_while_building, [True])  # `main` holds it during the build, once
            self.assertEqual(self.builds, 1)
            self.assertNotIn('guarded', inspect.getsource(runner.build))  # `build` itself asks for no lock

    def test_a_time_run_does_not_match_a_prepare_and_a_checkout_that_both_have_no_head(self):
        nameless = {'head': '', 'tree': '', 'branch': 'b', 'dirty': ''}  # what a failing git used to be read as
        built = b'what prepare built'
        with tempfile.TemporaryDirectory() as root:
            out = Path(root) / 'out'
            out.mkdir()
            self.leave(out, built)
            self.finished_prepare(out, hashlib.sha256(built).hexdigest(), identity=nameless)
            code, final = self.phase(root, 'time', nameless)
            self.assertEqual((code, final.get('status')), (1, 'REFUSED'))
            self.assertEqual(self.measured, [])

    def test_a_git_that_cannot_answer_refuses_both_phases_and_changes_nothing(self):
        # (F1) with git down the file hashes were '' == '' ("at HEAD") and the identity was an empty, clean one
        built = b'what prepare built'
        with tempfile.TemporaryDirectory() as root:
            out = Path(root) / 'out'
            code, written = self.phase(root, 'prepare', built=b'product', git_down=True)
            self.assertEqual((code, written.get('status')), (1, 'REFUSED'))
            self.assertIn('could not name', written['reason'])
            self.assertEqual(self.builds, 0)
            self.assertEqual((Path(root) / 'f.rs').read_text(), 'fn a() {}\n')
            self.leave(out, built)
            self.finished_prepare(out, hashlib.sha256(built).hexdigest())
            code, final = self.phase(root, 'time', git_down=True)
            self.assertEqual((code, final.get('status')), (1, 'REFUSED'))
            self.assertIn('could not name', final['reason'])
            self.assertEqual(self.measured, [])

    def test_the_executables_stay_whenever_an_evaluated_timed_mutant_gave_no_answer(self):
        # (F8) only BLOCKED and MISSING_EXECUTABLE kept them: a guard that failed after the run, a timeout or an
        # inconclusive plan reference deleted the very executables a retry needs
        built = b'what prepare built'
        for outcome in ('INVALID_GUARD_AFTER_RUN', 'INFRA_TIMEOUT', 'INFRA_ERROR', 'INCONCLUSIVE', 'COMPILE_ERROR',
                        'NO_TEST_RESULT', 'ZERO_TESTS', 'BLOCKED', 'MISSING_EXECUTABLE'):
            with self.subTest(outcome=outcome), tempfile.TemporaryDirectory() as root:
                out = Path(root) / 'out'
                out.mkdir()
                exe = self.leave(out, built)
                self.finished_prepare(out, hashlib.sha256(built).hexdigest())
                code, final = self.phase(root, 'time', self.A, time_outcome=outcome)
                self.assertEqual(final['mutants'][self.NAME]['time']['outcome'], outcome)
                self.assertTrue(exe.exists(), outcome)
        with tempfile.TemporaryDirectory() as root:  # the control: an answer is what makes them discardable
            out = Path(root) / 'out'
            out.mkdir()
            exe = self.leave(out, built)
            self.finished_prepare(out, hashlib.sha256(built).hexdigest())
            code, final = self.phase(root, 'time', self.A, time_outcome='KILLED')
            self.assertEqual((code, final['status']), (0, 'ALL_AS_EXPECTED'))
            self.assertFalse(exe.exists())

    def test_an_only_that_evaluates_no_timed_mutant_deletes_no_executable(self):
        # (F9) `--only` of a host-only mutant ran no time detector and still emptied exes/ of the mutants it left out
        built = b'what prepare built'
        host_only = {'file': 'host.rs', 'old': 'a', 'new': 'b', 'detectors': {'host': True}}
        with tempfile.TemporaryDirectory() as root:
            out = Path(root) / 'out'
            out.mkdir()
            exe = self.leave(out, built)
            self.finished_prepare(out, hashlib.sha256(built).hexdigest(), extra={
                'host_only': {'file': 'host.rs', 'must': {'host': True}, 'host': {'outcome': 'KILLED'}}})
            code, final = self.phase(root, 'time', self.A, only=('host_only',), extra={'host_only': host_only})
            self.assertEqual((code, final['status']), (0, 'ALL_AS_EXPECTED'))
            self.assertEqual(self.measured, [])  # no timed mutant was evaluated...
            self.assertTrue(exe.exists())  # ...so the executable of the one that was left out is still there

    def test_a_run_removes_only_the_executables_of_the_mutants_it_evaluated_and_got_an_answer_for(self):
        built, other = b'what prepare built', b'what prepare built for the other mutant'
        another = {'file': 'g.rs', 'old': 'a', 'new': 'b', 'detectors': {'time': True}}
        with tempfile.TemporaryDirectory() as root:
            out = Path(root) / 'out'
            out.mkdir()
            exe = self.leave(out, built)
            (out / 'exes' / 'other_timed.exe').write_bytes(other)
            self.finished_prepare(out, hashlib.sha256(built).hexdigest(), extra={
                'other_timed': {'file': 'g.rs', 'must': {'time': True},
                                'time_build': {'exit': 0, 'executable_sha256': hashlib.sha256(other).hexdigest()}}})
            code, final = self.phase(root, 'time', self.A, only=(self.NAME,), extra={'other_timed': another})
            self.assertEqual(self.measured, [(self.NAME, built)])
            self.assertFalse(exe.exists())  # its own answer came: this one goes
            self.assertEqual(sorted(p.name for p in (out / 'exes').iterdir()), ['other_timed.exe'])  # the other stays

    def test_a_build_whose_executable_is_gone_is_recorded_and_prepare_still_finishes(self):
        with tempfile.TemporaryDirectory() as root:
            code, prepared = self.phase(root, 'prepare', self.A, built=b'product', vanish=True)
            build = prepared['mutants'][self.NAME]['time_build']
            self.assertEqual((code, prepared['status']), (0, 'PREPARED'))
            self.assertIsNone(build['executable_sha256'])
            self.assertIn('target-exe', build['copy_error'])  # which path, what the OS said
            self.assertEqual(sorted(p.name for p in (Path(root) / 'out' / 'exes').iterdir()), [])
            code, final = self.phase(root, 'time', self.A)  # and `time` then says so for that mutant
            self.assertEqual(final['mutants'][self.NAME]['time']['outcome'], 'MISSING_EXECUTABLE')


class Native(unittest.TestCase):
    def test_placeholders_and_tolerance(self):
        self.assertEqual(native.fill({'a': ['$x', 1]}, {'x': 'v'}), {'a': ['v', 1]})
        self.assertTrue(native.near([{'x': 1.0}], [{'x': 1.0 + 1e-12}]))
        self.assertFalse(native.near([{'x': 1.0}], [{'x': 1.0001}]))
        self.assertFalse(native.near(True, 1.0))

    def test_summary_keeps_every_sample_in_its_extremes(self):
        out = native.summary([5, 1, 9, 3])
        self.assertEqual((out['n'], out['min'], out['max']), (4, 1, 9))

    def test_the_golden_trace_holds_only_steps_every_surface_can_run(self):
        journey = json.loads((native.FIXTURES / 'journey.json').read_text())
        golden = json.loads((native.FIXTURES / 'journey.expected.json').read_text())
        classes = {step['id']: step for step in journey['steps']}
        for name, trace in golden.items():
            for sid in trace:
                self.assertIn(sid, classes, sid)
                self.assertEqual(classes[sid]['class'], 'document', f'{name}/{sid}: a presentation step has no golden')
                self.assertNotIn('surfaces', classes[sid], sid)

    def test_a_missing_binary_is_a_recorded_prerequisite_not_a_pass(self):
        with tempfile.TemporaryDirectory() as root:
            code = run_main(native, ['--binary', str(Path(root) / 'nope'), '--out', root])
            self.assertEqual(code, 1)
            self.assertEqual(json.loads((Path(root) / 'manifest.json').read_text())['status'], 'PREREQUISITE_MISSING')


# --- round 6 (PR #44, review 5856147194 and the first real measurement) -------------------------------------------------
# Every test below states a behaviour the runners of `d294bdc` did not have, except four controls that must pass on both
# sides (listed in `spec-new-tests-vs-d294bdc-runners.log`: what must stay true after the fix). They run on fakes: no
# benchmark, no window, no performance lock. What fails on a runner that lacks the behaviour fails on its own assertion
# where the behaviour can be observed through the runner's `main`; a few tests of new helpers (`harness.dropped_env`,
# `guarded`, ...) can only fail there by not finding them.


@contextlib.contextmanager
def bounded(seconds=20):
    """Whatever is still running after `seconds` inside this block is a run that WAITS: it fails instead of hanging."""
    def waited(signum, frame):
        raise AssertionError(f'still running after {seconds}s: a run that must refuse or finish is waiting')

    previous = signal.signal(signal.SIGALRM, waited)
    signal.alarm(seconds)
    try:
        yield
    finally:
        signal.alarm(0)
        signal.signal(signal.SIGALRM, previous)


@contextlib.contextmanager
def git_is_down(directory):
    """The only `git` on PATH is one that fails the way git does outside a repository (or with a broken install)."""
    stubs = Path(directory) / 'git-is-down'
    stubs.mkdir(exist_ok=True)
    stub_git(stubs, 'echo "fatal: not a git repository (or any of the parent directories): .git" >&2; exit 128')
    with patch.dict(os.environ, {'PATH': f"{stubs}{os.pathsep}{os.environ['PATH']}"}):
        yield


@contextlib.contextmanager
def held(path):
    """`path` locked the way another process would lock it: through its own `open()` of the file (an `flock` conflicts
    across open file descriptions, also inside one process). Leaving the block releases it."""
    Path(path).parent.mkdir(parents=True, exist_ok=True)
    with open(path, 'a') as handle:
        fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield


def tree_of(directory):
    """Every file and directory under `directory`, with the bytes of the files (`None` for a directory): what a run that
    was refused must leave exactly as it found it."""
    return {p.relative_to(directory).as_posix(): p.read_bytes() if p.is_file() else None
            for p in sorted(Path(directory).rglob('*'))}


def checkout_lock():
    """Path of the per-checkout build lock of the runners."""
    make = getattr(native.harness, 'checkout_lock_path', None)
    if make is None:
        raise AssertionError('the runners take no per-checkout lock (`harness.checkout_lock_path` does not exist)')
    return make()


class UnavailableGit(unittest.TestCase):
    """git cannot answer (no repository, a broken install, a timeout). That is not "head empty, tree empty, clean": it is
    an unknown, and every runner refuses before it builds, mutates or measures. `--allow-dirty` is for a dirty tree, not
    for a tree nobody could name (F1)."""

    def refused(self, code, manifest):
        self.assertEqual(code, 1)
        self.assertEqual(manifest.get('status'), 'REFUSED', manifest.get('reason'))
        self.assertIn('could not name', manifest.get('reason', ''))

    def test_a_git_that_cannot_answer_is_an_unavailable_identity_never_a_clean_empty_one(self):
        with tempfile.TemporaryDirectory() as root, git_is_down(root):
            identity = native.harness.git_identity()
        self.assertIs(identity.get('unavailable'), True)
        self.assertNotEqual(identity['dirty'], '')  # a failed `status` is never a clean tree
        self.assertTrue(identity['head'].startswith('unavailable'), identity)
        self.assertTrue(identity['tree'].startswith('unavailable'), identity)

    def test_an_identity_git_did_name_is_not_marked_unavailable(self):
        with tempfile.TemporaryDirectory() as root:
            git(root, 'init', '-q')
            Path(root, 'a').write_text('a')
            git(root, 'add', 'a')
            git(root, 'commit', '-q', '-m', 'a')
            identity = native.harness.git_identity(Path(root))
        self.assertIs(identity.get('unavailable'), False)
        self.assertEqual((len(identity['head']), len(identity['tree'])), (40, 40))

    def test_two_identities_that_failed_are_never_the_same_head(self):
        with tempfile.TemporaryDirectory() as root, git_is_down(root):
            first, second = native.harness.git_identity(), native.harness.git_identity()
        self.assertFalse(native.harness.same_head(first, second))
        self.assertFalse(native.harness.same_head({'head': '', 'tree': ''}, {'head': '', 'tree': ''}))
        real = {'head': 'a' * 40, 'tree': 'b' * 40}
        self.assertTrue(native.harness.same_head(real, dict(real)))  # the control: two real, equal answers
        self.assertFalse(native.harness.same_head(real, {**real, 'tree': 'c' * 40}))

    def test_run_py_refuses_to_build_and_to_measure_even_with_allow_dirty(self):
        with tempfile.TemporaryDirectory() as root, git_is_down(root):
            exe = Path(root) / 'built.exe'
            exe.write_text('binary')
            build = MagicMock(return_value=(['cargo'], done(), str(exe)))
            out = Path(root) / 'out-build'
            with patch.object(runner, 'build', build):
                code = run_main(runner, ['--phase', 'build', '--out', str(out), '--allow-dirty'])
            self.refused(code, json.loads((out / 'manifest.build.json').read_text()))
            build.assert_not_called()
            # a build record whose head and tree are empty strings is what a failing git used to write: it must not match
            out = Path(root) / 'out-measure'
            out.mkdir()
            (out / 'build.json').write_text(json.dumps({'head': '', 'tree': '', 'dirty': False, 'binary_sha256': 'x',
                                                        'exe': 'exes/candidate.exe'}))
            code = run_main(runner, ['--phase', 'measure', '--out', str(out), '--allow-dirty'])
            self.refused(code, json.loads((out / 'manifest.json').read_text()))

    def test_ab_py_refuses_both_phases_even_with_allow_dirty_and_another_baseline(self):
        for phase in ('build', 'measure'):
            with self.subTest(phase=phase), tempfile.TemporaryDirectory() as root, git_is_down(root):
                out = Path(root) / 'out'
                make, build = MagicMock(), MagicMock()
                with patch.object(ab, 'make_baseline', make), patch.object(ab, 'build', build):
                    code = run_main(ab, ['--phase', phase, '--out', str(out), '--allow-dirty', '--allow-other-baseline'])
                self.refused(code, json.loads((out / 'manifest.json').read_text()))
                make.assert_not_called()
                build.assert_not_called()

    def test_ab_py_does_not_take_what_a_failing_git_printed_as_the_baseline_commit(self):
        # the candidate is named (patched); only the baseline is asked of a git that fails: an empty `rev-parse` output used
        # to be recorded as "the baseline commit" and, with `--allow-other-baseline`, accepted
        clean = {'head': 'h' * 40, 'tree': 't' * 40, 'branch': 'b', 'dirty': ''}
        with tempfile.TemporaryDirectory() as root, git_is_down(root):
            out = Path(root) / 'out'
            make = MagicMock()
            with patch.object(ab.harness, 'git_identity', return_value=clean), \
                    patch.object(ab.harness, 'toolchain', return_value={}), patch.object(ab, 'make_baseline', make):
                code = run_main(ab, ['--phase', 'build', '--out', str(out), '--allow-other-baseline'])
            manifest = json.loads((out / 'manifest.json').read_text())
            self.assertEqual((code, manifest['status']), (1, 'REFUSED'))
            self.assertIn('names no commit', manifest['reason'])
            make.assert_not_called()

    def test_native_py_refuses_both_phases_even_with_allow_dirty(self):
        for phase in ('build', 'run'):
            with self.subTest(phase=phase), tempfile.TemporaryDirectory() as root, git_is_down(root):
                out = Path(root) / 'out'
                code = run_main(native, ['--phase', phase, '--out', str(out), '--allow-dirty'])
                path = out / ('manifest.build.json' if phase == 'build' else 'manifest.json')
                self.refused(code, json.loads(path.read_text()))

    def test_a_file_git_cannot_compare_with_head_is_never_called_restored(self):
        # `'' == ''` (two failed reads) used to be "the file is at HEAD" and "the mutant was restored"
        with tempfile.TemporaryDirectory() as root, git_is_down(root):
            path = Path(root) / 'f.rs'
            path.write_text('fn a() {}\n')
            mutant = {'file': str(path), 'old': 'fn a() {}', 'new': 'fn a() { loop {} }'}
            with patch.object(mutate, 'ROOT', Path('/')):
                with self.assertRaises(SystemExit):
                    with mutate.Mutated(mutant):
                        pass
            self.assertEqual(path.read_text(), 'fn a() {}\n')  # the file is put back all the same

    def test_git_strict_says_whether_git_answered(self):
        with tempfile.TemporaryDirectory() as root:
            ok, text = native.harness.git_strict(['rev-parse', 'HEAD'], Path(root))  # not a repository
            self.assertFalse(ok)
            self.assertIn('git rev-parse HEAD', text)
            git(root, 'init', '-q')
            ok, text = native.harness.git_strict(['status', '--porcelain'], Path(root))
            self.assertEqual((ok, text), (True, ''))  # an empty answer is a valid one for `status`
        with tempfile.TemporaryDirectory() as root, patch.dict(os.environ, {'PATH': root}):
            ok, text = native.harness.git_strict(['status'], Path(root))  # no git at all
            self.assertFalse(ok)
            self.assertIn('cannot run git', text)


class RuntimeStale(unittest.TestCase):
    """`runtime_stale_reason` asks git which runtime sources exist: a git that cannot say is not "no source is newer"."""

    def reason(self, body, source_newer=False):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'runtime/dist').mkdir(parents=True)
            (root / 'runtime/src').mkdir(parents=True)
            (root / 'runtime/dist/runtime.js').write_text('built')
            (root / 'runtime/src/a.ts').write_text('source')
            now = time.time()
            os.utime(root / 'runtime/dist/runtime.js', (now - 100, now - 100))
            os.utime(root / 'runtime/src/a.ts', (now, now) if source_newer else (now - 200, now - 200))
            stubs = root / 'bin'
            stubs.mkdir()
            stub_git(stubs, body)
            with patch.dict(os.environ, {'PATH': f"{stubs}{os.pathsep}{os.environ['PATH']}"}):
                return native.runtime_stale_reason(root)

    def test_a_git_that_cannot_list_the_sources_is_a_reason_not_a_fresh_runtime(self):
        self.assertIn('git', self.reason('echo "fatal: not a git repository" >&2; exit 128') or '')

    def test_a_listing_with_no_source_that_exists_is_a_reason_too(self):
        self.assertIsNotNone(self.reason('exit 0'))

    def test_a_runtime_newer_than_its_sources_is_fresh_and_an_older_one_is_stale(self):
        self.assertIsNone(self.reason('echo runtime/src/a.ts'))  # the control: this half holds on every runner
        self.assertIn('older', self.reason('echo runtime/src/a.ts', source_newer=True))


class RoundsAndSamples(unittest.TestCase):
    """No round, no sample: "every kind was judged" is vacuously true of zero kinds, and must never be a pass (F3)."""

    def test_zero_or_negative_rounds_is_an_argument_error_that_writes_nothing(self):
        for rounds in ('0', '-1'):
            with self.subTest(rounds=rounds), tempfile.TemporaryDirectory() as root:
                out = Path(root) / 'out'
                errors = io.StringIO()
                with contextlib.redirect_stderr(errors), self.assertRaises(SystemExit) as stopped:
                    run_main(ab, ['--phase', 'measure', '--out', str(out), '--rounds', rounds])
                self.assertEqual(stopped.exception.code, 2)
                self.assertIn('--rounds', errors.getvalue())
                self.assertFalse(out.exists())

    def test_a_side_without_samples_is_inconclusive_never_a_pass(self):
        key = ('dense', 'move.translate')
        full = slot([100 + i for i in range(40)], [120] * 4)
        empty = {'samples': [], 'medians': [], 'ratios': [], 'long_ticks': 0, 'n': []}
        for base, cand in ((empty, empty), (full, empty), (empty, full)):
            verdict = ab.judge({key: base}, {key: cand})['dense/move.translate']
            self.assertEqual(verdict['status'], 'INCONCLUSIVE')
            self.assertIn('no sample', ' '.join(verdict['reasons']))
        self.assertEqual(ab.judge({key: full}, {key: full})['dense/move.translate']['status'], 'NO_REGRESSION_IN_SCENARIO')

    def test_a_verdict_over_no_kind_is_never_a_pass(self):
        status, _, code = ab.overall({}, {'crates/core/src/x.rs': {}})
        self.assertNotEqual(status, 'NO_REGRESSION_IN_SCENARIO')
        self.assertEqual((status, code), ('INCONCLUSIVE', harness.INCONCLUSIVE_EXIT))
        self.assertEqual(ab.overall({}, {})[2], harness.INCONCLUSIVE_EXIT)
        self.assertEqual(ab.overall({'k': {'status': 'REGRESSED'}}, {'f': {}})[0], 'REGRESSED')  # a regression still wins


class ChildEnvironment(unittest.TestCase):
    """A child that measures or builds gets the runner's own environment minus every inherited `TESSEL_GIZMO_PERF_*`
    (builds also lose `CARGO_TARGET_DIR` and `TESSEL_CARGO_LANE`) plus exactly what the runner sets (F14)."""

    INHERITED = {'TESSEL_GIZMO_PERF_CALIBRATE': '1', 'TESSEL_GIZMO_PERF_BLESS': '1',
                 'TESSEL_GIZMO_PERF_OUT': '/somewhere/else.json'}
    PASSING = ('test result: ok. 3 passed; 0 failed; 0 ignored\n' +
               ''.join(f'test {name} ... ok\n' for name in runner.EXPECTED_TESTS))

    def absent(self, name, env, label=''):
        # The message never carries `env`: it is the whole environment of the process, and a log must not hold that.
        self.assertFalse(name in env, f'{name} reached the child {label}'.strip())

    @staticmethod
    def changed(first, second):
        """The names whose value differs between two environments (names only: never the values)."""
        return sorted(name for name in set(first) | set(second) if first.get(name) != second.get(name))

    def test_helpers_drop_what_a_measuring_child_must_not_inherit_and_keep_the_rest(self):
        environ = {**self.INHERITED, 'CARGO_TARGET_DIR': '/t', 'TESSEL_CARGO_LANE': 'l', 'HOME': '/h', 'PATH': '/bin'}
        h = native.harness
        self.assertEqual(h.dropped_env(environ=environ), sorted(self.INHERITED))
        self.assertEqual(h.dropped_env(build=True, environ=environ), sorted([*self.INHERITED, 'CARGO_TARGET_DIR',
                                                                              'TESSEL_CARGO_LANE']))
        self.assertEqual(h.child_env({'TESSEL_GIZMO_PERF_OUT': '/r.json'}, environ=environ),
                         {'CARGO_TARGET_DIR': '/t', 'TESSEL_CARGO_LANE': 'l', 'HOME': '/h', 'PATH': '/bin',
                          'TESSEL_GIZMO_PERF_OUT': '/r.json'})
        self.assertEqual(h.child_env(build=True, environ=environ), {'HOME': '/h', 'PATH': '/bin'})

    def test_scoped_environ_holds_exactly_the_given_environment_and_restores_the_caller_s(self):
        before = dict(os.environ)
        with native.harness.scoped_environ({'ONLY': 'this'}):
            self.assertEqual(sorted(os.environ), ['ONLY'])
            self.assertEqual(os.environ['ONLY'], 'this')
        self.assertEqual(self.changed(os.environ, before), [])
        with self.assertRaises(RuntimeError), native.harness.scoped_environ({'ONLY': 'this'}):
            raise RuntimeError('the body failed')
        self.assertEqual(self.changed(os.environ, before), [])

    def measure(self, stdout, *flags, after=None, code=0, env=None):
        """`run.py --phase measure` end to end: the executable is a file, the benchmark a function that notes the
        environment it was started with and writes the report it was asked for."""
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / 'out'
            (out / 'exes').mkdir(parents=True)
            (out / 'exes' / 'candidate.exe').write_bytes(b'binary')
            (out / 'build.json').write_text(json.dumps({
                'head': 'h' * 40, 'tree': 't' * 40, 'dirty': False,
                'binary_sha256': hashlib.sha256(b'binary').hexdigest(), 'exe': 'exes/candidate.exe'}))
            seen = {}

            def benchmark(command, cwd=None, env=None, timeout=None):
                seen.update(command=command, env=env)
                Path(env['TESSEL_GIZMO_PERF_OUT']).write_text('{}')
                return done(stdout, code)

            def judge_after(manifest):
                manifest['guard_after'] = after or 'clean'
                return after

            quiet = ({'free_gib': 100, 'loadavg': (0.0, 0.0, 0.0)}, None)
            clean = {'head': 'h' * 40, 'tree': 't' * 40, 'branch': 'b', 'dirty': ''}
            with patch.object(runner.harness, 'git_identity', return_value=clean), \
                    patch.object(runner.harness, 'toolchain', return_value={}), \
                    patch.object(runner.harness, 'guards', return_value=quiet), \
                    patch.object(runner.harness, 'claims', return_value=[]), \
                    patch.object(runner.harness, 'Lock', FakeLock), \
                    patch.object(runner.harness, 'judge_after', side_effect=judge_after), \
                    patch.object(runner.harness, 'run_bounded', side_effect=benchmark), \
                    patch.dict(os.environ, env if env is not None else {}):
                result = run_main(runner, ['--phase', 'measure', '--out', str(out), *flags])
            return result, json.loads((out / 'manifest.json').read_text()), seen, out

    def test_an_exported_calibrate_never_reaches_a_measured_run_nor_its_record(self):
        code, manifest, seen, out = self.measure(self.PASSING, env=self.INHERITED)
        self.absent('TESSEL_GIZMO_PERF_CALIBRATE', seen['env'])
        self.absent('TESSEL_GIZMO_PERF_BLESS', seen['env'])
        self.assertEqual(seen['env']['TESSEL_GIZMO_PERF_OUT'], str(out / 'report.json'))  # not the caller's
        self.assertEqual(manifest['run']['env'], {'TESSEL_GIZMO_PERF_OUT': harness.redact(str(out / 'report.json'))})
        self.assertEqual((code, manifest['status']), (0, 'PASS'))

    def test_the_run_record_lists_only_the_tests_whose_own_line_says_ok(self):
        failing = libtest({MAIN_TEST: NOT_CAUGHT}, passed=[PLAN_CONTROL, DELAY_CONTROL])
        code, manifest, seen, out = self.measure(failing.stdout, code=101)
        self.assertEqual((code, manifest['status']), (1, 'FAIL'))
        self.assertEqual(manifest['tests_passed'], [PLAN_CONTROL, DELAY_CONTROL])
        self.assertEqual(manifest['tests_failed'], [MAIN_TEST])

    def test_calibrate_is_in_the_child_only_when_the_runner_was_asked_for_it(self):
        code, manifest, seen, out = self.measure(self.PASSING, '--calibrate', env=self.INHERITED)
        self.assertEqual(seen['env']['TESSEL_GIZMO_PERF_CALIBRATE'], '1')
        self.assertEqual(manifest['run']['env']['TESSEL_GIZMO_PERF_CALIBRATE'], '1')
        self.absent('TESSEL_GIZMO_PERF_BLESS', seen['env'])
        self.assertEqual((code, manifest['status']), (0, 'CALIBRATION_ONLY'))

    def child_env_of(self, module, call, inherited):
        seen = {}

        def fake(command, cwd=None, env=None, timeout=None):
            seen['env'] = dict(os.environ) if env is None else env  # `None` is "inherit everything"
            return done('', 1)

        with patch.object(module.harness, 'run_bounded', side_effect=fake), patch.dict(os.environ, inherited):
            call()
        return seen['env']

    def test_the_builds_of_every_runner_get_no_target_directory_and_no_perf_switch(self):
        inherited = {**self.INHERITED, 'CARGO_TARGET_DIR': '/t', 'TESSEL_CARGO_LANE': 'lane'}
        for label, module, call in (('run.py', runner, runner.build), ('ab.py', ab, lambda: ab.build(HERE))):
            with self.subTest(label):
                env = self.child_env_of(module, call, inherited)
                for name in inherited:
                    self.absent(name, env, f'of {label}')

    def test_the_ab_child_gets_calibrate_explicitly_the_report_path_it_chose_and_nothing_else_inherited(self):
        with tempfile.TemporaryDirectory() as tmp:
            report = Path(tmp) / 'report.json'
            env = self.child_env_of(ab, lambda: ab.run_once('exe', report), self.INHERITED)
        self.absent('TESSEL_GIZMO_PERF_BLESS', env)
        self.assertEqual(env['TESSEL_GIZMO_PERF_CALIBRATE'], '1')  # the A/B takes its numbers in the calibration mode
        self.assertEqual(env['TESSEL_GIZMO_PERF_OUT'], str(report))

    def test_the_mutant_runs_get_no_inherited_perf_switch(self):
        with tempfile.TemporaryDirectory() as tmp:
            report = Path(tmp) / 'report.json'

            def judge_after(scratch):
                scratch.update(guard_after='clean', machine_after={'loadavg': (0.0, 0.0, 0.0)})

            with patch.object(mutate.harness, 'judge_after', side_effect=judge_after):
                env = self.child_env_of(
                    mutate, lambda: mutate.run_time(Path(tmp) / 'x.exe', report, Path(tmp), 'delay_1ms_per_handler'),
                    self.INHERITED)
            self.absent('TESSEL_GIZMO_PERF_CALIBRATE', env)  # a calibrating child asserts no budget: never killed
            self.absent('TESSEL_GIZMO_PERF_BLESS', env)
            self.assertEqual(env['TESSEL_GIZMO_PERF_OUT'], str(report))
        # the structural/host detector would rewrite its golden file and pass under an inherited BLESS
        env = self.child_env_of(mutate, lambda: mutate.detect(['cargo', 'test']), self.INHERITED)
        self.absent('TESSEL_GIZMO_PERF_BLESS', env)

    def test_the_native_editor_gets_no_inherited_perf_switch_and_os_environ_is_left_as_it_was(self):
        seen = {}

        class Process:
            socket = 'socket'

            def __init__(self, binary, home):
                seen.update(os.environ)

        with patch.dict(os.environ, self.INHERITED), patch.object(native.proof_surface, 'Native', Process):
            before = dict(os.environ)
            native.Window('binary', '/tmp/home')
            after = dict(os.environ)
        self.absent('TESSEL_GIZMO_PERF_BLESS', seen)
        self.absent('TESSEL_GIZMO_PERF_CALIBRATE', seen)
        self.assertEqual(seen.get('TESSEL_VIEWPORT_OVERLAY_ACK'), '1')  # what the runner means to set is there
        self.assertEqual(self.changed(after, before), [])  # and nothing it set stays in the process


class Guarded(unittest.TestCase):
    """`harness.guarded`: what every runner owns before it reads or writes anything."""

    def test_it_yields_none_when_free_keeps_the_lock_file_and_releases_the_lock_at_the_end(self):
        guarded = native.harness.guarded
        with tempfile.TemporaryDirectory() as root:
            out = Path(root) / 'out'
            with bounded(), guarded(out, 'x.py') as refusal:
                self.assertIsNone(refusal)
                with open(out / SHARED_LOCK, 'a') as another:  # a second open() of the file is somebody else
                    with self.assertRaises(BlockingIOError):
                        fcntl.flock(another, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with held(out / SHARED_LOCK):  # released: it can be taken again
                pass
            self.assertTrue((out / SHARED_LOCK).is_file())  # a lock file is never removed (two inodes, two owners)

    def test_it_refuses_a_directory_that_is_owned_and_says_which_lock(self):
        guarded = native.harness.guarded
        with tempfile.TemporaryDirectory() as root:
            out = Path(root) / 'out'
            with held(out / SHARED_LOCK), bounded(), guarded(out, 'x.py') as refusal:
                self.assertEqual(refusal['status'], 'REFUSED')
                self.assertIn(str(out / SHARED_LOCK), refusal['reason'])
                self.assertIn('not waited for', refusal['reason'])

    def test_the_checkout_lock_is_one_file_per_checkout_outside_the_repository(self):
        harness_module = native.harness
        with tempfile.TemporaryDirectory() as first, tempfile.TemporaryDirectory() as second:
            path = harness_module.checkout_lock_path(first)
            self.assertEqual(path, harness_module.checkout_lock_path(Path(first)))
            self.assertNotEqual(path, harness_module.checkout_lock_path(second))
            self.assertEqual(path.parent, Path(harness_module.CHECKOUT_LOCK_DIR))
            self.assertFalse(str(path).startswith(str(harness_module.ROOT)))

    def test_a_held_checkout_lock_refuses_a_build_before_the_directory_exists(self):
        guarded = native.harness.guarded
        with tempfile.TemporaryDirectory() as root:
            out = Path(root) / 'out'
            with held(checkout_lock()), bounded():
                with guarded(out, 'x.py', builds=True) as refusal:
                    self.assertEqual(refusal['status'], 'REFUSED')
                    self.assertIn('checkout lock', refusal['reason'])
                self.assertFalse(out.exists())
                with guarded(out, 'x.py') as refusal:  # a phase that builds nothing is not held
                    self.assertIsNone(refusal)


class OwnedDirectories(unittest.TestCase):
    """One run per `--out` for every runner and phase (F4, F5), and one build per CHECKOUT (F6): a run that finds what it
    must own already owned refuses (one REFUSED line, exit 1), never waits, and changes nothing, not even to write its own
    refusal."""

    def planted(self, root):
        out = Path(root) / 'out'
        out.mkdir()
        for name in ('manifest.json', 'report.json', 'round00-baseline.json', 'verdicts.json', 'stdout.txt'):
            (out / name).write_text('written by the run that owns this directory')
        (out / 'exes').mkdir()
        (out / 'exes' / 'candidate.exe').write_bytes(b'what the owner is measuring')
        return out

    def last_line(self, printed):
        return json.loads(printed.getvalue().strip().splitlines()[-1])

    def test_ab_run_and_native_refuse_an_owned_out_and_change_nothing(self):
        cases = (('ab.py', ab, ['--phase', 'measure']), ('run.py', runner, ['--phase', 'measure']),
                 ('native.py', native, ['--phase', 'run']))
        for label, module, argv in cases:
            with self.subTest(label), tempfile.TemporaryDirectory() as root:
                out = self.planted(root)
                printed = io.StringIO()
                with held(out / SHARED_LOCK), bounded():
                    before = tree_of(out)
                    code = run_main(module, [*argv, '--out', str(out)], printed)
                    self.assertEqual(code, 1)
                    self.assertEqual(tree_of(out), before)  # nothing created, changed or removed
                refusal = self.last_line(printed)
                self.assertEqual(refusal['status'], 'REFUSED')
                self.assertIn(SHARED_LOCK, refusal['reason'])
                # the control: with the owner gone the same call is not refused for the directory
                free = io.StringIO()
                with bounded():
                    run_main(module, [*argv, '--out', str(out)], free)
                self.assertNotIn(SHARED_LOCK, free.getvalue())

    BUILD_CASES = ('run.py build', 'ab.py build', 'native.py build', 'mutate.py prepare')

    def build_case(self, label, stack):
        """`(module, argv, build)`: the runner of `label` with whatever would really build (or edit the tree)
        replaced by a mock that fails the test, so that a runner that does not refuse cannot start a cargo build."""
        build = MagicMock(side_effect=AssertionError('a build started although the run did not own what it builds'))
        if label == 'run.py build':
            stack.enter_context(patch.object(runner, 'build', build))
            return runner, ['--phase', 'build'], build
        if label == 'ab.py build':
            stack.enter_context(patch.object(ab, 'build', build))
            stack.enter_context(patch.object(ab, 'make_baseline', build))
            return ab, ['--phase', 'build'], build
        if label == 'native.py build':
            stack.enter_context(patch.object(native.harness, 'run_bounded', build))
            return native, ['--phase', 'build'], build
        stack.enter_context(patch.object(mutate.runner, 'build', build))
        # no mutant: whatever runs here (the runners before the locks did) can touch no source file
        stack.enter_context(patch.object(mutate, 'MUTANTS', {}))
        return mutate, ['--phase', 'prepare'], build

    def test_the_build_phases_refuse_while_the_checkout_is_being_built_and_create_nothing(self):
        for label in self.BUILD_CASES:
            with self.subTest(label), tempfile.TemporaryDirectory() as root, contextlib.ExitStack() as stack:
                out = Path(root) / 'out'
                module, argv, build = self.build_case(label, stack)
                printed = io.StringIO()
                with held(checkout_lock()), bounded():
                    code = run_main(module, [*argv, '--out', str(out)], printed)
                self.assertEqual(code, 1)
                self.assertFalse(out.exists())  # not even the directory
                refusal = self.last_line(printed)
                self.assertEqual(refusal['status'], 'REFUSED')
                self.assertIn('checkout lock', refusal['reason'])
                build.assert_not_called()

    def test_a_build_phase_refuses_an_owned_out_too_and_builds_nothing(self):
        for label in self.BUILD_CASES:
            with self.subTest(label), tempfile.TemporaryDirectory() as root, contextlib.ExitStack() as stack:
                out = self.planted(root)
                module, argv, build = self.build_case(label, stack)
                printed = io.StringIO()
                with held(out / SHARED_LOCK), bounded():
                    before = tree_of(out)
                    code = run_main(module, [*argv, '--out', str(out)], printed)
                    self.assertEqual(code, 1)
                    self.assertEqual(tree_of(out), before)
                refusal = self.last_line(printed)
                self.assertEqual(refusal['status'], 'REFUSED')
                self.assertIn(SHARED_LOCK, refusal['reason'])
                build.assert_not_called()

    def test_the_checkout_lock_is_asked_for_first_and_neither_lock_waits(self):
        with tempfile.TemporaryDirectory() as root:
            out = self.planted(root)
            printed = io.StringIO()
            with held(checkout_lock()), held(out / SHARED_LOCK), bounded():
                before = tree_of(out)
                code = run_main(ab, ['--phase', 'build', '--out', str(out)], printed)
                self.assertEqual(code, 1)
                self.assertEqual(tree_of(out), before)
            reason = self.last_line(printed)['reason']
            self.assertIn('checkout lock', reason)
            self.assertNotIn(SHARED_LOCK, reason)

    def test_a_measure_phase_builds_nothing_and_does_not_need_the_checkout_lock(self):
        with tempfile.TemporaryDirectory() as root:
            out = Path(root) / 'out'
            printed = io.StringIO()
            with held(checkout_lock()), bounded():
                code = run_main(runner, ['--phase', 'measure', '--out', str(out)], printed)
            self.assertNotIn('checkout lock', printed.getvalue())  # it refuses for its own reasons, or measures

    def test_a_real_second_process_is_refused_with_one_line_and_exit_1(self):
        with tempfile.TemporaryDirectory() as root:
            out = self.planted(root)
            with held(out / SHARED_LOCK):
                before = tree_of(out)
                finished = subprocess.run([sys.executable, str(HERE / 'ab.py'), '--phase', 'measure', '--out', str(out)],
                                          capture_output=True, text=True, timeout=60, cwd=str(HERE))
                after = tree_of(out)
            self.assertEqual(finished.returncode, 1, finished.stderr[-300:])
            lines = finished.stdout.strip().splitlines()
            self.assertEqual(len(lines), 1, lines)
            refusal = json.loads(lines[0])
            self.assertEqual(refusal['status'], 'REFUSED')
            self.assertIn(SHARED_LOCK, refusal['reason'])
            self.assertEqual(after, before)


# --- the native window: the overlay acknowledgement, the trace, the wait and the authoring process (F10..F13) --------


def stamp(**changes):
    return {'scene': 'proof', 'tool': 'scale', 'space': 'local', 'revision': 9, 'frame': 24, 'selection': ['n'], **changes}


STATE_ANSWER = {'visible': True, 'tool': 'scale', 'space': 'local', 'entity': 'n', 'stamp': stamp()}
THIS_ACK = {'visible': True, 'tool': 'scale', 'space': 'local', 'entity': 'n', 'stamp': stamp()}
PREVIOUS_ACK = {'visible': True, 'tool': 'rotate', 'space': 'world', 'entity': 'n',
                'stamp': stamp(tool='rotate', space='world', revision=8)}
OPEN_PROJECT = {'id': 'open.project', 'phase': 'cold', 'op': {'op': 'project.create', 'name': 'x'}}
ACK_STEPS = [OPEN_PROJECT, {'id': 'state.scale.local', 'phase': 'hot', 'tool': {'tool': 'scale', 'space': 'local'},
                            'op': {'op': 'scene.gizmo.state', 'scene': '$scene', 'frame': '$frame'}}]


class SavingWindow(FakeWindow):
    """A `FakeWindow` that also writes the document `project.save` was asked for: the authoring process of `run_corpus`
    then runs its real code."""

    def op(self, request, timeout=90):
        if request['op'] == 'project.save':
            FakeWindow.log.append('project.save')
            Path(request['path']).write_text('{}')
            return True, {'saved': request['path']}
        return super().op(request, timeout)


def ack_window(first, then, stale_polls):
    """A window whose page acknowledges `first` for the `stale_polls` reads that follow a `scene.gizmo.state` and `then`
    after them: the host keeps the last acknowledgement it matched until a newer one arrives."""

    class Window(SavingWindow):
        reads = None

        def op(self, request, timeout=90):
            if request['op'] == 'scene.gizmo.state':
                FakeWindow.log.append('scene.gizmo.state')
                Window.reads = 0
                return True, json.loads(json.dumps({'overlay': STATE_ANSWER}))
            return super().op(request, timeout)

        def state(self):
            state = super().state()
            if Window.reads is not None:
                state['overlay'] = {'supported': True, 'drawn': then if Window.reads >= stale_polls else first}
                Window.reads += 1
            return state

    return Window


def run_one_corpus(window_class, steps, author=False, ack_seconds=0.3, clock=None):
    """`native.run_corpus` over `steps` with `window_class` for the editor. The saved project of the opening is a fake
    path unless `author` asks for the real authoring process (which then runs on `window_class` too). `clock`: a
    `VirtualClock` for `native.time` (a wait of 60 s is then read off the clock, not waited for)."""
    FakeWindow.log, FakeWindow.stale_shows, FakeWindow.instances = [], set(), []
    journey = dict(NativeFlow.JOURNEY, steps=steps)
    with contextlib.ExitStack() as stack, tempfile.TemporaryDirectory() as out:
        stack.enter_context(bounded())
        if clock is not None:
            stack.enter_context(patch.object(native, 'time', clock))
        stack.enter_context(patch.object(native, 'Window', window_class))
        stack.enter_context(patch.object(native, 'ACK_SECONDS', ack_seconds, create=True))
        if not author:
            stack.enter_context(patch.object(native, 'author_saved_project', return_value=(
                Path('/tmp/saved.tessel.json'), {'path': '/tmp/saved.tessel.json'})))
        return native.run_corpus('bin', journey, {'small': {}}, 'small', out, {})


def failed_checks(result):
    return [check['check'] for check in result['checks'] if not check['ok']]


class OverlayAcknowledgement(unittest.TestCase):
    """The acknowledgement kept for a step must BELONG to the step (F13): the host holds the last one it matched until a
    newer one arrives, so the first one read after a step is often the previous step's."""

    def ack_check(self, result):
        found = [c for c in result['checks'] if c['check'].startswith('state.scale.local:') and 'acknowledged' in c['check']]
        self.assertEqual(len(found), 1, [c['check'] for c in result['checks']])
        return found[0]

    def test_the_acknowledgement_that_arrives_after_the_previous_one_is_the_one_recorded(self):
        # (a long wait: it ends at the first matching reading; the failing cases below keep the short one)
        result = run_one_corpus(ack_window(PREVIOUS_ACK, THIS_ACK, stale_polls=3), ACK_STEPS, ack_seconds=5)
        check = self.ack_check(result)
        self.assertTrue(check['ok'], check['detail'])
        self.assertEqual(result['overlay_drawn'][0]['drawn']['stamp']['revision'], 9)
        self.assertEqual(failed_checks(result), [])

    def test_a_previous_steps_acknowledgement_that_is_never_replaced_fails_and_shows_both_sides(self):
        # the first real measurement recorded the rotate/world overlay of revision 8 under `state.scale.local`
        result = run_one_corpus(ack_window(PREVIOUS_ACK, PREVIOUS_ACK, stale_polls=0), ACK_STEPS)
        check = self.ack_check(result)
        self.assertFalse(check['ok'])
        self.assertIn("tool: expected 'scale', got 'rotate'", check['detail'])
        self.assertIn("space: expected 'local', got 'world'", check['detail'])
        entry = result['overlay_drawn'][0]
        self.assertEqual(entry['drawn']['tool'], 'rotate')  # the last one seen, kept as evidence
        self.assertEqual(entry['expected']['tool'], 'scale')

    def test_a_step_the_page_never_acknowledged_fails(self):
        result = run_one_corpus(ack_window(None, None, stale_polls=0), ACK_STEPS)
        check = self.ack_check(result)
        self.assertFalse(check['ok'])
        self.assertIn('no acknowledgement', check['detail'])

    def test_an_acknowledgement_of_another_selection_scene_revision_or_frame_fails(self):
        for key, changes in (('stamp.selection', {'selection': ['other']}), ('stamp.scene', {'scene': 'proof-b'}),
                             ('stamp.revision', {'revision': 8}), ('stamp.frame', {'frame': 12})):
            with self.subTest(key):
                ack = {**THIS_ACK, 'stamp': stamp(**changes)}
                check = self.ack_check(run_one_corpus(ack_window(ack, ack, stale_polls=0), ACK_STEPS))
                self.assertFalse(check['ok'])
                self.assertIn(key, check['detail'])

    def test_ack_mismatch_is_empty_only_for_the_acknowledgement_of_the_step(self):
        expected = native.ack_expectation({'scene': 'proof', 'frame': 24}, STATE_ANSWER, {'tool': 'scale', 'space': 'local'})
        self.assertEqual(native.ack_mismatch(expected, THIS_ACK), [])
        self.assertTrue(native.ack_mismatch(expected, PREVIOUS_ACK))
        self.assertIn('no acknowledgement', native.ack_mismatch(expected, None)[0])
        self.assertIn('no acknowledgement', native.ack_mismatch(expected, 'drawn')[0])


class SlowMonitor(native.Window):
    """`settle` over a monitor that answers slowly or never: only `viewport` is scripted. A query is given `timeout`
    seconds like a socket with that timeout; the sleep is cut at `cap` so a runner that passes none cannot hold the spec."""

    CONVERGED = {'phase': 'ready', 'headline': '',
                 'directed': {'scene': 'proof', 'active': None, 'queued': None, 'suspended': False,
                              'shown': {'request_id': 6, 'edit_frame': 24}},
                 'last_frame': {'request': 6, 'revision': 6, 'hash': 'h6'}}

    def __init__(self, delay, honours_timeout=True, cap=2.0):
        self.delay, self.honours_timeout, self.cap = delay, honours_timeout, cap
        self.asked, self.deadline, self.native = [], None, FakeNative()

    def viewport(self, timeout=90):
        self.asked.append(timeout)
        if self.honours_timeout and self.delay > timeout:
            time.sleep(min(timeout, self.cap))
            raise TimeoutError('timed out')
        time.sleep(min(self.delay, self.cap))
        return json.loads(json.dumps(self.CONVERGED))


class NativeSession(unittest.TestCase):
    def test_a_render_status_that_failed_is_no_trace_and_fails_the_session_checks(self):
        # (F10) the error text of a dead query used to be read as the report: no session error in it, a "clean" session
        class DeadTrace(SavingWindow):
            def op(self, request, timeout=90):
                if request['op'] == 'render.status':
                    return False, {'code': 'unavailable', 'message': 'render.status is not available'}
                return super().op(request, timeout)

        window = DeadTrace('bin', 'home')
        self.assertEqual(window.trace(), {})
        self.assertFalse(window.last_trace_ok)
        self.assertTrue(any('render.status failed' in failure for failure in window.trace_failures))
        result = run_one_corpus(DeadTrace, [OPEN_PROJECT])
        checks = {c['check']: c['ok'] for c in result['checks']}
        self.assertFalse(checks['every `render.status` query answered ok (a failed one is no session trace)'])
        self.assertFalse(checks['the session reported no error'])
        self.assertTrue(result['trace_failures'])
        self.assertEqual(failed_checks(run_one_corpus(SavingWindow, [OPEN_PROJECT])), [])  # the control: it answers

    def test_a_monitor_that_never_answers_cannot_hold_a_wait_past_its_seconds(self):
        # (F11) every query is given the time that is LEFT: a monitor that never answers held a 2 s wait for 90 s
        window = SlowMonitor(delay=60)
        started = time.perf_counter()
        with bounded():
            ms, state, info = window.settle('proof', 24, after=-1, seconds=0.6)
        self.assertIsNone(ms)
        self.assertLess(time.perf_counter() - started, 3)
        self.assertTrue(window.asked and all(timeout <= 0.7 for timeout in window.asked), window.asked)

    def test_a_reply_that_arrives_after_the_deadline_is_a_timeout_that_keeps_what_it_showed(self):
        window = SlowMonitor(delay=0.5, honours_timeout=False)
        with bounded():
            ms, state, info = window.settle('proof', 24, after=5, seconds=0.2)
        self.assertIsNone(ms)  # it converged too late to count
        self.assertEqual(info['request'], 6)
        self.assertEqual(state['directed']['shown']['request_id'], 6)

    def test_a_monitor_that_converges_in_time_gives_its_milliseconds(self):
        window = SlowMonitor(delay=0.05)
        with bounded():
            ms, state, info = window.settle('proof', 24, after=5, seconds=5)
        self.assertIsNotNone(ms)
        self.assertEqual(info['request'], 6)

    def test_what_stopping_the_authoring_process_reported_fails_the_cleanup_check(self):
        # (F12) an editor that ignored the terminate and had to be killed used to be thrown away with its process
        class StopsBadly(SavingWindow):
            def stop(self):
                return ['the editor ignored SIGTERM and was killed'] if FakeWindow.instances.index(self) == 0 else []

        result = run_one_corpus(StopsBadly, [OPEN_PROJECT], author=True)
        self.assertEqual(result['cleanup_errors'], ['authoring process: the editor ignored SIGTERM and was killed'])
        checks = {c['check']: c['ok'] for c in result['checks']}
        self.assertFalse(checks['the editor process ended without cleanup problems'])
        self.assertEqual(failed_checks(run_one_corpus(SavingWindow, [OPEN_PROJECT], author=True)), [])  # the control

    def test_a_dialog_that_left_the_native_page_on_screen_fails_its_own_check(self):
        steps = [OPEN_PROJECT, {'id': 'present.dialog.open', 'phase': 'present', 'ui': ['invoke', 'flow.open']}]
        label = 'present.dialog.open: the dialog hid the native page'

        def window(on_screen):
            class Dialog(SavingWindow):
                def state(self):
                    return {**super().state(), 'on_screen': on_screen}

            return Dialog

        self.assertEqual({c['check']: c['ok'] for c in run_one_corpus(window(False), steps)['checks']}[label], True)
        shown = {c['check']: c['ok'] for c in run_one_corpus(window(True), steps)['checks']}
        self.assertFalse(shown[label])


# --- the release of a gesture and the pixels a gesture has to change (BL015.23 items 16 and 17, round 7) -----------------


class VirtualClock:
    """`native.time` for a test: `sleep` moves the clock and nothing waits, so a bound of 30 s is read off the clock and not
    waited for. `HostWindow` reads the same clock to decide what its monitor shows."""

    def __init__(self):
        self.now = 1000.0

    def perf_counter(self):
        return self.now

    monotonic = time = perf_counter  # the runner reads its clock under three names

    def sleep(self, seconds):
        self.now += seconds


def release_overlay(step, revision):
    """The overlay the core answers a release with, and that the page must then acknowledge: the state it left."""
    return {'visible': True, 'tool': step['tool'], 'space': step['space'], 'entity': 'n',
            'stamp': stamp(tool=step['tool'], space=step['space'], revision=revision)}


class HostWindow(native.Window):
    """A window whose monitor is a model of the HOST around ONE gesture. What happens is a function of the clock
    (`VirtualClock`), not of the number of reads, so a read the runner makes for its own reasons moves nothing.

    The moves ask for the candidate's picture (`candidate`, an `(request id, pixel hash)`; it completes `draw_lag` later;
    `draws_candidate=False`: the host never draws it). What the host does with the RELEASE is the behaviour under test:
      dedupe   asks for nothing: the plan equals the candidate's (what the first real run recorded, 12bc8da)
      redraw   asks for `release_picture` at once; it completes `release_lag` later
      late     asks for it `late` seconds after the reply (a host that decides asynchronously)
      older    the monitor goes back to `release_picture`, an older request
      rewrite  the monitor keeps the candidate's request id with the pixels of `release_picture`
      hang     asks for a picture that never completes
    The page acknowledges the committed state `ack_delay` after the reply (`ack` match), never (never) or with the previous
    revision's overlay (stale); until then it holds the acknowledgement of the revision before. A cancel asks for
    `cancel_picture`; `monitor.show_scene`, `seek`, `history.undo` and `history.redo` each ask for the next request (the
    base's pixels; `undo_hash` for the undo; `redo_hash` for the redo, the picture the commit left when not given;
    `undo_draws` / `redo_draws`: False, or the booleans of the undos / redos in order (then True), say that the history
    request is answered ok and the host draws nothing).
    `read_cost`: every `viewport` read takes that long (the monitor is slow to answer)."""

    REVISION = 9  # the project revision before the release; the release leaves REVISION + 1

    def __init__(self, clock, step, base, candidate, release='dedupe', release_picture=None, cancel_picture=None,
                 ack='match', ack_delay=0.02, draw_lag=0.05, release_lag=0.04, late=0.1, draws_candidate=True,
                 overlay_visible=True, adds_undo_entry=True, undo_hash=None, scene='proof', read_cost=0.0,
                 redo_hash=None, undo_draws=True, redo_draws=True):
        self.clock, self.step, self.scene = clock, step, scene
        self.read_cost = read_cost  # seconds of the clock one `viewport` read takes (a slow monitor); the state is then read
        self.base, self.candidate = base, candidate
        self.release, self.release_picture, self.cancel_picture = release, release_picture, cancel_picture
        self.ack, self.ack_delay, self.late = ack, ack_delay, late
        self.draw_lag, self.release_lag = draw_lag, release_lag
        self.draws_candidate, self.overlay_visible, self.adds_undo_entry = draws_candidate, overlay_visible, adds_undo_entry
        self.undo_hash, self.redo_hash = undo_hash, redo_hash
        self.undo_draws = undo_draws if isinstance(undo_draws, bool) else list(undo_draws)
        self.redo_draws = redo_draws if isinstance(redo_draws, bool) else list(redo_draws)
        self.commit_hash = None  # the hash of the picture the commit left: what a redo shows again
        self.shown = base  # (request id, hash) of the picture on the monitor
        self.inflight = None  # {'picture', 'at'}: a picture the host asked for and when it completes
        self.pending = None  # a request the host will only make later
        self.drawn = release_overlay(step, self.REVISION)  # what the page acknowledged last
        self.ack_plan = None
        self.committed = self.undone = False
        self.log = []
        self.native = FakeNative()
        self.deadline = None
        self.frames = {}
        self.trace_failures = []

    def ready(self):
        return 1.0

    def ask(self, picture, delay):
        self.inflight = {'picture': picture, 'at': self.clock.now + delay}

    def draws(self, kind):
        """Whether this history request draws a picture (`undo_draws` / `redo_draws`: a bool, or one boolean per request)."""
        flags = self.undo_draws if kind == 'history.undo' else self.redo_draws
        if isinstance(flags, bool):
            return flags
        return flags.pop(0) if flags else True

    def advance(self):
        now = self.clock.now
        if self.pending and now >= self.pending['at']:
            self.ask(self.pending['picture'], self.release_lag)
            self.pending = None
        if self.inflight and now >= self.inflight['at']:
            self.shown, self.inflight = self.inflight['picture'], None
        if self.ack_plan and now >= self.ack_plan['at']:
            self.drawn, self.ack_plan = self.ack_plan['ack'], None

    def release_reply(self):
        self.committed = True
        self.commit_hash = self.release_picture[1] if self.release in ('redraw', 'late') else self.shown[1]
        overlay = release_overlay(self.step, self.REVISION + 1)
        if self.release == 'redraw':
            self.ask(self.release_picture, self.release_lag)
        elif self.release == 'late':
            self.pending = {'picture': self.release_picture, 'at': self.clock.now + self.late}
        elif self.release == 'older':
            self.shown = self.release_picture
        elif self.release == 'rewrite':
            self.shown = (self.shown[0], self.release_picture[1])
        elif self.release == 'hang':
            self.inflight = {'picture': self.release_picture, 'at': float('inf')}
        if self.ack == 'match':
            self.ack_plan = {'at': self.clock.now + self.ack_delay, 'ack': json.loads(json.dumps(overlay))}
        elif self.ack == 'stale':
            self.ack_plan = {'at': self.clock.now + self.ack_delay, 'ack': release_overlay(self.step, self.REVISION)}
        overlay['visible'] = self.overlay_visible
        return True, {'document_changed': True, 'overlay': overlay}

    def op(self, request, timeout=90):
        kind = request['op']
        self.log.append(kind)
        self.advance()
        if kind == 'scene.get':
            return True, {'value': 5 if self.committed else 0}
        if kind == 'history.status':
            return True, {'undo_depth': 1 if self.committed and self.adds_undo_entry else 0,
                          'redo_depth': 1 if self.undone else 0}
        if kind == 'scene.gizmo.state':
            return True, {'overlay': {'handles': [{'id': self.step['handle'], 'to': {'u': 0.0, 'v': 0.0}}]}}
        if kind == 'scene.pointer.down':
            return True, {'dragging': True, 'hit': {'handle': self.step['handle']}}
        if kind == 'scene.pointer.move':
            if self.draws_candidate:
                self.ask(self.candidate, self.draw_lag)
            return True, {'applied': True, 'document_changed': False, 'would_change': True, 'position': 1.0}
        if kind == 'scene.pointer.up':
            return self.release_reply()
        if kind == 'scene.pointer.cancel':
            if self.cancel_picture:
                self.ask(self.cancel_picture, self.release_lag)
            return True, {}
        if kind == 'project.open':
            return True, {'action': 'project.open', 'project': 'x'}
        if kind in ('monitor.show_scene', 'seek', 'history.undo', 'history.redo'):
            if kind == 'history.undo':
                self.committed, self.undone = False, True
            elif kind == 'history.redo':
                self.committed, self.undone = True, False
            if kind in ('history.undo', 'history.redo') and not self.draws(kind):
                return True, {}  # the document moved and the answer was ok, but the host asked for no picture
            shows = self.base[1]
            if kind == 'history.undo' and self.undo_hash:
                shows = self.undo_hash
            elif kind == 'history.redo':
                shows = self.redo_hash or self.commit_hash
            self.ask((max(self.shown[0], self.inflight['picture'][0] if self.inflight else 0) + 1, shows), self.draw_lag)
            return True, {}
        if kind == 'render.status':
            return True, {'report': {}}
        return True, {}

    def viewport(self, timeout=90):
        self.clock.sleep(self.read_cost)  # the answer reflects the state at the END of the read
        self.advance()
        return {'phase': 'ready', 'headline': '', 'overlay': {'supported': True, 'drawn': self.drawn},
                'directed': {'scene': self.scene, 'queued': None, 'suspended': False,
                             'active': {'id': self.inflight['picture'][0]} if self.inflight else None,
                             'shown': {'request_id': self.shown[0], 'edit_frame': 24}},
                'last_frame': {'request': self.shown[0], 'revision': self.shown[0], 'hash': self.shown[1]}}


class GestureCase(unittest.TestCase):
    """`native.drag` over a `HostWindow` under a virtual clock: what the checks say about what the host did."""

    STEP = {'id': 'drag.translate.x', 'kind': 'drag', 'phase': 'hot', 'tool': 'translate', 'space': 'world',
            'handle': 'x', 'delta': [0.03, 0.0], 'moves': 2, 'end': 'up'}
    SCALE = {'id': 'drag.scale.local.uniform', 'kind': 'drag', 'phase': 'hot', 'tool': 'scale', 'space': 'local',
             'handle': 'x', 'delta': [0.02, -0.02], 'moves': 2, 'end': 'up'}
    SCALE_CANCEL = {'id': 'drag.scale.cancel', 'kind': 'drag', 'phase': 'hot', 'tool': 'scale', 'space': 'local',
                    'handle': 'x', 'delta': [0.02, -0.02], 'moves': 2, 'end': 'cancel'}
    TRANSLATE_CANCEL = dict(SCALE_CANCEL, id='drag.translate.cancel', tool='translate', space='world')
    BASE, CANDIDATE, REDRAWN = (10, 'base'), (13, 'candidate'), (14, 'candidate')
    LEGIT = 'the release left a legitimate picture'
    SAME_PIXELS = 'the redrawn picture shows the same pixels as the candidate'
    DIFFERS = 'the committed picture differs from the one before the gesture'
    CANDIDATE_DIFFERS = 'the drawn candidate picture differs from the base picture'
    CANCEL_IS_BASE = 'after the cancel the picture is the base picture'

    def run_gesture(self, step=None, corpus=None, **model):
        """`(checks, record)` of `native.drag` for `step` (default: the translate release) in `corpus` (None: not declared)."""
        step = step or self.STEP
        clock = VirtualClock()
        window = HostWindow(clock, step, **{'base': self.BASE, 'candidate': self.CANDIDATE, **model})
        checks, details, undoable = {}, {}, []
        result = {'steps': {step['id']: {}}}
        if corpus:
            result['corpus'] = corpus

        def check(label, condition, detail=''):
            checks[label], details[label] = bool(condition), str(detail)
            return bool(condition)

        def timed(sid, request, timeout=120, phase=None):
            return window.op(request, timeout)

        started = clock.now
        with patch.object(native, 'time', clock), bounded(10):
            native.drag(window, timed, check, step, 'proof', {'frame': 24}, 'n', [], undoable, result, {})
        self.elapsed, self.window, self.details, self.undoable = clock.now - started, window, details, undoable
        return checks, result['steps'][step['id']]

    def label(self, checks, fragment):
        found = [label for label in checks if fragment in label]
        self.assertEqual(len(found), 1, (fragment, list(checks)))
        return found[0]

    def verdict(self, checks, fragment):
        return checks[self.label(checks, fragment)]

    def failed(self, checks):
        """The failed checks, in the order the runner asked them, without the step id."""
        return [label.split(': ', 1)[1] for label, ok in checks.items() if not ok]


class Release(GestureCase):
    """What a release may leave on the monitor when the candidate had been drawn (item 16): the host draws nothing for a
    release whose plan equals the candidate's, so a picture newer than the candidate's is one of two legitimate outcomes."""

    def test_a_release_the_host_deduped_is_recorded_as_deduped_and_every_check_passes(self):
        checks, record = self.run_gesture(release='dedupe')
        self.assertEqual(self.failed(checks), [])
        self.assertEqual(record['commit_picture'], 'deduped')
        self.assertIsNotNone(record['client_ms_up_to_converged_picture'])  # the first converged read, not a render latency
        seen = record['commit_observed']
        self.assertEqual((seen['shown_request'], seen['shown_hash']), self.CANDIDATE)
        self.assertEqual(seen['ack_problems'], [])
        self.assertEqual(record['picture']['request'], self.CANDIDATE[0])
        self.assertEqual(record['pixels']['committed_hash'], 'candidate')
        self.assertLess(self.elapsed, 3)  # no wait for a newer picture that was never going to come
        self.assertEqual([entry.base_hash for entry in self.undoable], ['base'])

    def test_a_release_the_host_redrew_is_recorded_as_redrawn_and_every_check_passes(self):
        checks, record = self.run_gesture(release='redraw', release_picture=self.REDRAWN)
        self.assertEqual(self.failed(checks), [])
        self.assertEqual(record['commit_picture'], 'redrawn')
        self.assertEqual(record['picture']['request'], self.REDRAWN[0])
        self.assertGreater(record['client_ms_up_to_converged_picture'], 30)  # it waited for the redraw (40 ms of the model)
        self.assertTrue(self.verdict(checks, self.SAME_PIXELS))

    def test_a_redraw_that_shows_other_pixels_than_the_candidate_fails(self):
        # the preview and the commit disagree: the redraw is not what the candidate showed
        checks, record = self.run_gesture(release='redraw', release_picture=(14, 'other'))
        self.assertEqual(record['commit_picture'], 'redrawn')
        self.assertEqual(self.failed(checks), [self.SAME_PIXELS + ' (same hash)'])

    def test_a_picture_older_than_the_candidates_after_the_release_fails_at_once(self):
        checks, record = self.run_gesture(release='older', release_picture=(12, 'candidate'))
        self.assertFalse(self.verdict(checks, self.LEGIT))
        self.assertIn('OLDER', self.details[self.label(checks, self.LEGIT)])
        self.assertIsNone(record['commit_picture'])
        self.assertFalse(self.verdict(checks, self.DIFFERS))  # nothing legitimate to compare with the base
        self.assertLess(self.elapsed, 3)

    def test_the_candidates_request_with_other_pixels_is_not_a_dedupe(self):
        checks, record = self.run_gesture(release='rewrite', release_picture=(0, 'other'))
        self.assertFalse(self.verdict(checks, self.LEGIT))
        self.assertIsNone(record['commit_picture'])
        self.assertIn('other pixels', self.details[self.label(checks, self.LEGIT)])

    def test_a_redraw_of_the_base_picture_fails_where_pixels_are_required(self):
        checks, record = self.run_gesture(release='redraw', release_picture=(14, 'base'))
        self.assertEqual(record['commit_picture'], 'redrawn')
        self.assertEqual(self.failed(checks), [self.SAME_PIXELS + ' (same hash)', self.DIFFERS])

    def test_a_deduped_release_whose_candidate_already_was_the_base_picture_fails_where_pixels_are_required(self):
        checks, record = self.run_gesture(release='dedupe', candidate=(13, 'base'))
        self.assertEqual(record['commit_picture'], 'deduped')
        self.assertEqual(self.failed(checks), [self.CANDIDATE_DIFFERS, self.DIFFERS])

    def test_a_monitor_that_never_converges_after_the_release_fails_within_the_bound_and_is_not_deduped(self):
        checks, record = self.run_gesture(release='hang', release_picture=(14, 'candidate'))
        self.assertFalse(self.verdict(checks, self.LEGIT))
        self.assertIn('a timeout, not a dedupe', self.details[self.label(checks, self.LEGIT)])
        self.assertIsNone(record['commit_picture'])
        self.assertIsNone(record['client_ms_up_to_converged_picture'])  # None only for a real timeout
        # the hash the last state seen holds (the candidate's) must not be compared with the base: the commit never converged
        self.assertFalse(self.verdict(checks, self.DIFFERS))
        self.assertIsNone(record['pixels']['committed_hash'])
        # read off the clock: the whole gesture takes the 30 s bound and not a second more than a few polls
        self.assertGreaterEqual(self.elapsed, native.COMMIT_SECONDS)
        self.assertLessEqual(self.elapsed, native.COMMIT_SECONDS + 1)

    def test_a_state_the_page_never_acknowledged_is_not_deduped_and_is_given_up_after_the_ack_wait(self):
        checks, record = self.run_gesture(release='dedupe', ack='never')
        self.assertFalse(self.verdict(checks, self.LEGIT))
        self.assertIn('unconfirmed', self.details[self.label(checks, self.LEGIT)])
        self.assertIsNone(record['commit_picture'])
        self.assertEqual(record['commit_observed']['shown_request'], self.CANDIDATE[0])  # it did see the picture
        self.assertGreaterEqual(self.elapsed, native.ACK_SECONDS)  # it waited for the acknowledgement...
        self.assertLess(self.elapsed, native.ACK_SECONDS + 1)  # ...and not for the 30 s of a timeout

    def test_an_acknowledgement_of_the_previous_revision_is_not_the_one_of_the_commit(self):
        checks, record = self.run_gesture(release='dedupe', ack='stale')
        self.assertFalse(self.verdict(checks, self.LEGIT))
        self.assertIn('stamp.revision: expected 10, got 9', self.details[self.label(checks, self.LEGIT)])
        self.assertIsNone(record['commit_picture'])

    def test_an_acknowledgement_that_arrives_late_still_confirms_a_dedupe(self):
        checks, record = self.run_gesture(release='dedupe', ack_delay=1.5)
        self.assertEqual(self.failed(checks), [])
        self.assertEqual(record['commit_picture'], 'deduped')
        self.assertGreaterEqual(self.elapsed, 1.5)
        self.assertLess(record['client_ms_up_to_converged_picture'], 100)  # when the picture was first seen, not the ack

    def test_a_redraw_that_arrives_after_the_first_converged_read_is_not_taken_for_a_dedupe(self):
        # an asynchronous host: the candidate's picture is on the monitor and acknowledged, the redraw comes 100 ms later
        checks, record = self.run_gesture(release='late', release_picture=self.REDRAWN, late=0.1)
        self.assertEqual(self.failed(checks), [])
        self.assertEqual(record['commit_picture'], 'redrawn')
        self.assertEqual(record['picture']['request'], self.REDRAWN[0])

    def test_a_release_that_answered_no_visible_overlay_cannot_be_confirmed(self):
        checks, record = self.run_gesture(release='dedupe', overlay_visible=False)
        self.assertFalse(self.verdict(checks, self.LEGIT))
        self.assertIn('no visible overlay', self.details[self.label(checks, self.LEGIT)])
        self.assertIsNone(record['commit_picture'])
        self.assertLess(self.elapsed, 3)  # nothing to wait for: it is not a wait for the acknowledgement

    def test_a_candidate_that_was_never_drawn_keeps_the_strict_rule(self):
        # no picture of the candidate exists for a release to equal: only a picture newer than the press counts
        checks, record = self.run_gesture(release='dedupe', draws_candidate=False)
        self.assertFalse(self.verdict(checks, 'the candidate was drawn while the gesture was open'))
        self.assertFalse(self.verdict(checks, 'the committed picture is a NEW picture (the candidate was never drawn'))
        self.assertIsNone(record['commit_picture'])
        self.assertNotIn('commit_observed', record)  # `settle_commit` is for a drawn candidate only
        checks, record = self.run_gesture(release='redraw', release_picture=(14, 'x'), draws_candidate=False)
        self.assertTrue(self.verdict(checks, 'the committed picture is a NEW picture (the candidate was never drawn'))
        self.assertEqual(record['commit_picture'], 'redrawn')
        self.assertFalse(self.verdict(checks, 'the candidate was drawn while the gesture was open'))  # still its own failure

    def test_settle_commit_alone_is_bounded_by_its_seconds_and_a_late_reply_is_a_timeout(self):
        clock = VirtualClock()
        window = HostWindow(clock, self.STEP, base=self.BASE, candidate=self.CANDIDATE, release='hang',
                            release_picture=(14, 'candidate'))
        window.shown = self.CANDIDATE
        window.op({'op': 'scene.pointer.up'})
        info = native.picture(window.viewport())
        started = clock.now
        with patch.object(native, 'time', clock):
            settled = window.settle_commit('proof', 24, info, None, None, seconds=3)
        self.assertEqual((settled['outcome'], settled['ms']), (None, None))
        self.assertAlmostEqual(clock.now - started, 3, delta=0.05)


class PixelExpectations(GestureCase):
    """What a gesture has to do to the picture, declared per corpus and per step class (item 17)."""

    def test_the_table_declares_every_corpus_and_class_and_only_dense_scale_may_stay_equal(self):
        for corpus in ('small', 'dense', 'deep'):
            for tool in ('translate', 'rotate', 'scale'):
                rule, why = native.pixel_rule(corpus, tool)
                expected = native.MAY_STAY_EQUAL if (corpus, tool) == ('dense', 'scale') else native.MUST_CHANGE
                self.assertEqual(rule, expected, (corpus, tool))
                self.assertIn(tool, native.PIXEL_EXPECTATIONS[corpus])  # declared, not defaulted
                self.assertGreater(len(why), 40)
        self.assertIn('12bc8da', native.pixel_rule('dense', 'scale')[1])  # the evidence is in the justification
        self.assertEqual(native.pixel_rule('other', 'scale')[0], native.MUST_CHANGE)  # nothing is inherited
        self.assertEqual(native.pixel_rule(None, 'scale')[0], native.MUST_CHANGE)

    def test_a_scale_that_leaves_pixels_equal_fails_in_small_and_in_deep(self):
        for corpus in ('small', 'deep'):
            with self.subTest(corpus):
                checks, record = self.run_gesture(self.SCALE, corpus=corpus, release='dedupe', candidate=(13, 'base'))
                self.assertEqual(self.failed(checks), [self.CANDIDATE_DIFFERS, self.DIFFERS])
                self.assertEqual(record['pixel_expectation']['rule'], native.MUST_CHANGE)

    def test_translate_and_rotate_that_leave_pixels_equal_still_fail_in_dense(self):
        for tool, space in (('translate', 'world'), ('rotate', 'local')):
            with self.subTest(tool):
                step = dict(self.SCALE, id=f'drag.{tool}.x', tool=tool, space=space)
                checks, record = self.run_gesture(step, corpus='dense', release='dedupe', candidate=(13, 'base'))
                self.assertEqual(self.failed(checks), [self.CANDIDATE_DIFFERS, self.DIFFERS])

    def test_an_undeclared_corpus_gets_no_leniency_for_scale(self):
        checks, record = self.run_gesture(self.SCALE, corpus='huge', release='dedupe', candidate=(13, 'base'))
        self.assertEqual(self.failed(checks), [self.CANDIDATE_DIFFERS, self.DIFFERS])
        checks, record = self.run_gesture(self.SCALE, corpus=None, release='dedupe', candidate=(13, 'base'))
        self.assertEqual(self.failed(checks), [self.CANDIDATE_DIFFERS, self.DIFFERS])

    def test_a_dense_scale_that_leaves_pixels_equal_passes_and_records_the_three_hashes(self):
        checks, record = self.run_gesture(self.SCALE, corpus='dense', release='dedupe', candidate=(13, 'base'))
        self.assertEqual(self.failed(checks), [])
        self.assertNotIn(self.CANDIDATE_DIFFERS, ' '.join(checks))  # the check is not asked: the step declares equality
        pixels = record['pixels']
        self.assertEqual((pixels['base_hash'], pixels['candidate_hash'], pixels['committed_hash']), ('base',) * 3)
        self.assertEqual((pixels['candidate_changed'], pixels['committed_changed']), (False, False))
        self.assertEqual(record['pixel_expectation']['rule'], native.MAY_STAY_EQUAL)
        self.assertIn('H17', record['pixel_expectation']['why'])
        self.assertEqual(record['commit_picture'], 'deduped')
        self.assertEqual(self.undoable[0].changed, False)

    def test_a_dense_scale_that_changes_pixels_does_not_fail(self):
        checks, record = self.run_gesture(self.SCALE, corpus='dense', release='dedupe')
        self.assertEqual(self.failed(checks), [])
        self.assertEqual((record['pixels']['candidate_changed'], record['pixels']['committed_changed']), (True, True))
        self.assertTrue(self.undoable[0].changed)

    def test_a_redrawn_dense_scale_that_equals_the_base_records_the_same_pixels_check_as_vacuous(self):
        checks, record = self.run_gesture(self.SCALE, corpus='dense', release='redraw', candidate=(13, 'base'),
                                          release_picture=(14, 'base'))
        self.assertEqual(self.failed(checks), [])
        vacuous = [item['check'] for item in record['vacuous_checks']]
        self.assertEqual(vacuous, [self.label(checks, self.SAME_PIXELS)])
        self.assertTrue(self.details[vacuous[0]].startswith('vacuous ('))
        # still asked: a redraw that is NOT the base does not pass because the step may stay equal
        checks, record = self.run_gesture(self.SCALE, corpus='dense', release='redraw', candidate=(13, 'base'),
                                          release_picture=(14, 'other'))
        self.assertEqual(self.failed(checks), [self.SAME_PIXELS + ' (same hash)'])

    def test_the_cancel_of_a_step_whose_candidate_was_the_base_records_its_restore_check_as_vacuous(self):
        checks, record = self.run_gesture(self.SCALE_CANCEL, corpus='dense', candidate=(13, 'base'), cancel_picture=(14, 'base'))
        self.assertEqual(self.failed(checks), [])
        self.assertEqual([item['check'] for item in record['vacuous_checks']], [self.label(checks, self.CANCEL_IS_BASE)])
        self.assertEqual(record['pixels']['after_cancel_hash'], 'base')
        self.assertTrue(self.verdict(checks, 'the cancel drew a NEW picture'))  # that one is a real proof and stays asked

    def test_the_cancel_of_a_step_whose_candidate_differs_is_a_real_check_not_a_vacuous_one(self):
        checks, record = self.run_gesture(self.TRANSLATE_CANCEL, corpus='dense', cancel_picture=(14, 'base'))
        self.assertEqual(self.failed(checks), [])
        self.assertNotIn('vacuous_checks', record)
        checks, record = self.run_gesture(self.TRANSLATE_CANCEL, corpus='dense', cancel_picture=(14, 'candidate'))
        self.assertEqual(self.failed(checks), [self.CANCEL_IS_BASE])

    def test_a_vacuous_check_is_still_asked_and_still_fails(self):
        # the step may leave pixels equal: that does not excuse a cancel that left something else on the monitor
        checks, record = self.run_gesture(self.SCALE_CANCEL, corpus='dense', candidate=(13, 'base'), cancel_picture=(14, 'odd'))
        self.assertEqual(self.failed(checks), [self.CANCEL_IS_BASE])
        self.assertEqual(len(record['vacuous_checks']), 1)

    def test_a_step_that_may_stay_equal_is_still_judged_on_the_undo_entry(self):
        checks, record = self.run_gesture(self.SCALE, corpus='dense', release='dedupe', candidate=(13, 'base'),
                                          adds_undo_entry=False)
        self.assertEqual(self.failed(checks), ['one undo entry per gesture'])


class GestureInTheCorpus(unittest.TestCase):
    """`native.run_corpus` over a journey with one gesture and its undo: the golden trace, the declared table in the report
    and the undo's own pixel check."""

    UNDO = 'history.undo.scale.uniform'
    DRAG_GOLDEN = {'transform': [5, 5, 5], 'undo': 1, 'redo': 0}
    UNDO_GOLDEN = {'transform': [0, 0, 0], 'undo': 0, 'redo': 1}
    RESTORED = f'{UNDO}: the undo restored the exact picture of before the gesture'
    GOLDEN = '{}: transform/undo/redo equal the core golden trace (undo offset 0)'

    def run_journey(self, corpus, golden=None, **model):
        step = GestureCase.SCALE
        steps = [
            {'id': 'open.project', 'phase': 'cold', 'op': {'op': 'project.create', 'name': 'x'}},
            {'id': 'open.attach', 'phase': 'cold', 'op': {'op': 'scene.attach', 'id': '$scene', 'path': '$bundle'}},
            {'id': 'open.show', 'phase': 'cold', 'op': {'op': 'monitor.show_scene', 'scene': '$scene'}},
            {'id': 'open.seek', 'phase': 'cold', 'op': {'op': 'seek', 'frame': '$frame'}},
            {'id': 'open.first_frame', 'phase': 'cold', 'wait': 'presented', 'frame': '$frame'},
            step, {'id': self.UNDO, 'phase': 'hot', 'op': {'op': 'history.undo'}}]
        journey = dict(NativeFlow.JOURNEY, corpora={corpus: NativeFlow.JOURNEY['corpora']['small']}, steps=steps)
        clock = VirtualClock()
        model = {'base': (10, 'base'), 'candidate': (13, 'candidate'), 'release': 'dedupe', **model}
        golden = {step['id']: self.DRAG_GOLDEN, self.UNDO: self.UNDO_GOLDEN} if golden is None else golden
        with contextlib.ExitStack() as stack, tempfile.TemporaryDirectory() as out:
            stack.enter_context(bounded(20))
            stack.enter_context(patch.object(native, 'time', clock))
            stack.enter_context(patch.object(native, 'Window', lambda binary, home: HostWindow(clock, step, **model)))
            stack.enter_context(patch.object(native, 'author_saved_project', return_value=(
                Path('/tmp/saved.tessel.json'), {'path': '/tmp/saved.tessel.json'})))
            return native.run_corpus('bin', journey, {corpus: golden}, corpus, out, {})

    def test_a_dense_scale_that_left_the_picture_equal_passes_the_corpus_and_records_what_it_could_not_prove(self):
        result = self.run_journey('dense', candidate=(13, 'base'))
        self.assertEqual(failed_checks(result), [])
        self.assertEqual(result['pixel_expectations']['scale']['rule'], native.MAY_STAY_EQUAL)  # the table is in the report
        self.assertEqual(result['pixel_expectations']['translate']['rule'], native.MUST_CHANGE)
        drag = result['steps']['drag.scale.local.uniform']
        self.assertEqual(drag['commit_picture'], 'deduped')
        self.assertEqual(drag['pixels']['committed_changed'], False)
        undo = result['steps'][self.UNDO]
        self.assertEqual([item['check'] for item in undo['vacuous_checks']], [self.RESTORED])  # nothing was there to restore
        detail = [c['detail'] for c in result['checks'] if c['check'] == self.RESTORED][0]
        self.assertTrue(detail.startswith('vacuous ('), detail)

    def test_the_golden_transform_still_judges_a_step_that_may_stay_equal(self):
        wrong = {'drag.scale.local.uniform': dict(self.DRAG_GOLDEN, transform=[7, 7, 7]), self.UNDO: self.UNDO_GOLDEN}
        result = self.run_journey('dense', golden=wrong, candidate=(13, 'base'))
        failing = failed_checks(result)
        self.assertEqual(failing, [self.GOLDEN.format('drag.scale.local.uniform')])

    def test_the_undo_of_a_gesture_that_changed_pixels_is_checked_for_real_and_fails_when_it_restored_something_else(self):
        result = self.run_journey('small')
        self.assertEqual(failed_checks(result), [])
        self.assertNotIn('vacuous_checks', result['steps'][self.UNDO])
        self.assertEqual(result['pixel_expectations']['scale']['rule'], native.MUST_CHANGE)
        result = self.run_journey('small', undo_hash='odd')
        self.assertEqual(failed_checks(result), [self.RESTORED])

    def test_a_scale_that_changed_nothing_in_the_small_corpus_fails_the_corpus(self):
        result = self.run_journey('small', candidate=(13, 'base'))
        self.assertEqual(failed_checks(result), [
            'drag.scale.local.uniform: the drawn candidate picture differs from the base picture',
            'drag.scale.local.uniform: the committed picture differs from the one before the gesture'])


RECORDED_RUN = HERE / 'fixtures/recorded-native-report.json.gz'  # swept of machine paths (`harness.redact`)
Recorded = collections.namedtuple('Recorded', 'corpus step base candidate picture record')


def recorded_gestures():
    """Every drag step of the first real window run (12bc8da: immutable evidence), with what the report kept of it: the
    picture before the gesture (the last picture any earlier step of that corpus recorded), the candidate's and the picture
    the monitor showed after the release or the cancel, each as `(request id, pixel hash)`."""
    report = json.load(gzip.open(RECORDED_RUN))
    journey = json.loads((native.FIXTURES / 'journey.json').read_text())
    found = []
    for result in report['results']:
        last = None
        for step in journey['steps']:
            record = result['steps'].get(step['id']) or {}
            if step.get('kind') == 'drag':
                pick = lambda picture: (picture['request'], picture['hash'])  # noqa: E731
                found.append(Recorded(result['corpus'], step, pick(last), pick(record['candidate_picture']),
                                      pick(record['picture']), record))
            if record.get('picture'):
                last = record['picture']
    return found


class RecordedFirstRun(GestureCase):
    """The first real window run (12bc8da, immutable evidence) replayed through `native.drag` with a monitor that shows the
    request ids and hashes its report recorded: the 15 releases that failed `the committed picture is a NEW picture` and
    the 5 dense checks that found the scale pictures equal to the base. The report does NOT hold what the page acknowledged
    after a release (it kept `overlay_drawn` for the `scene.gizmo.state` steps only), nor when each picture appeared: the
    replay's acknowledgement is assumed to match, and the timing is the model's. Those are the only values here that are
    not the report's."""

    @classmethod
    def setUpClass(cls):
        cls.gestures = recorded_gestures()

    def pick(self, corpus, sid):
        return next(g for g in self.gestures if g.corpus == corpus and g.step['id'] == sid)

    def replay(self, gesture, candidate=None, **model):
        model = ({'release': 'dedupe'} if gesture.step['end'] == 'up' else {'cancel_picture': gesture.picture}) | model
        return self.run_gesture(gesture.step, corpus=gesture.corpus, base=gesture.base,
                                candidate=candidate or gesture.candidate, **model)

    def test_the_report_holds_the_observations_the_replay_is_built_from(self):
        self.assertEqual(len(self.gestures), 21)  # 3 corpora x (5 releases + 2 cancels)
        releases = [g for g in self.gestures if g.step['end'] == 'up']
        self.assertEqual(len(releases), 15)
        for g in releases:  # the observation behind H16: the monitor still showed the candidate's own picture
            self.assertEqual(g.candidate, g.picture, (g.corpus, g.step['id']))
            self.assertGreater(g.candidate[0], g.base[0], (g.corpus, g.step['id']))
            self.assertIsNone(g.record['client_ms_up_to_converged_picture'])  # the 30 s wait that ran out
        for g in self.gestures:  # the observation behind H17: the scale pictures of `dense` are the base picture
            equal = g.candidate[1] == g.base[1]
            self.assertEqual(equal, g.corpus == 'dense' and g.step['tool'] == 'scale', (g.corpus, g.step['id']))
        for g in self.gestures:  # the cancels drew a newer picture, which is the base's
            if g.step['end'] == 'cancel':
                self.assertGreater(g.picture[0], g.candidate[0])
                self.assertEqual(g.picture[1], g.base[1])
        # what it does NOT hold: the acknowledgement of the committed state and the outcome of a release
        for g in self.gestures:
            self.assertNotIn('commit_observed', g.record)
            self.assertNotIn('commit_picture', g.record)

    def test_every_recorded_gesture_passes_the_new_rules(self):
        for g in self.gestures:
            with self.subTest(g.corpus, step=g.step['id']):
                checks, record = self.replay(g)
                self.assertEqual(self.failed(checks), [])
                self.assertEqual(record.get('commit_picture'), 'deduped' if g.step['end'] == 'up' else None)
                self.assertEqual(record['pixel_expectation']['rule'], native.pixel_rule(g.corpus, g.step['tool'])[0])

    def test_the_replay_records_dense_scale_as_unchanged_and_everything_else_as_changed(self):
        for g in self.gestures:
            with self.subTest(g.corpus, step=g.step['id']):
                checks, record = self.replay(g)
                changed = record['pixels']['candidate_changed']
                self.assertEqual(changed, not (g.corpus == 'dense' and g.step['tool'] == 'scale'))

    def test_a_recorded_hash_flipped_to_the_base_hash_fails_where_pixels_are_required(self):
        g = self.pick('small', 'drag.translate.x')
        checks, _ = self.replay(g, candidate=(g.candidate[0], g.base[1]))  # the candidate would have been the base picture
        self.assertEqual(self.failed(checks), [self.CANDIDATE_DIFFERS, self.DIFFERS])
        checks, _ = self.replay(g, release='redraw', release_picture=(g.candidate[0] + 1, g.base[1]))  # a release that
        self.assertIn(self.DIFFERS, self.failed(checks))  # redraws the base picture
        self.assertIn(self.SAME_PIXELS + ' (same hash)', self.failed(checks))

    def test_a_recorded_request_flipped_to_an_older_one_fails(self):
        for corpus in ('small', 'dense', 'deep'):
            g = self.pick(corpus, 'drag.rotate.local.y')
            with self.subTest(corpus):
                checks, record = self.replay(g, release='older', release_picture=(g.candidate[0] - 1, g.candidate[1]))
                self.assertFalse(self.verdict(checks, self.LEGIT))
                self.assertIsNone(record['commit_picture'])

    def test_a_dense_scale_whose_recorded_hash_is_not_the_base_does_not_fail(self):
        for sid in ('drag.scale.local.uniform', 'drag.scale.local.x'):
            g = self.pick('dense', sid)
            checks, record = self.replay(g, candidate=(g.candidate[0], 'ffff0000ffff0000'))
            self.assertEqual(self.failed(checks), [], sid)
            self.assertEqual(record['pixels']['committed_changed'], True)

    def test_the_same_scale_hashes_in_small_or_deep_fail(self):
        for corpus in ('small', 'deep'):
            g = self.pick(corpus, 'drag.scale.local.uniform')
            checks, _ = self.run_gesture(g.step, corpus=corpus, base=g.base, candidate=(g.candidate[0], g.base[1]),
                                         release='dedupe')
            self.assertEqual(self.failed(checks), [self.CANDIDATE_DIFFERS, self.DIFFERS], corpus)


# --- round 8 (PR #44, review 5859397176): git environment, build inputs, one lock per --out, late answers --------------
# New tests of this round are the ones of `GitEnvironment`, `DirtyIdentity`, `OneLockPerOut`, `TwoRunnerProcesses`,
# `BoundedPolls` and `LateAcknowledgement`, the dirty-tree tests of `MutantExecutables` (`test_a_time_run_on_a_dirty_tree...`,
# `test_each_phase_refuses...`, `test_prepare_holds_the_shared_lock...`) and the hostile `GIT_DIR` test of `Baseline`. They are
# run against the runners of the previous head (ca66012) in `spec-new-tests-vs-ca66012-runners.log`. The tests of rounds 4 to 7
# that named a per-runner lock (`.mutate.lock`, `.ab.lock`, `.run.lock`, `.native.lock`) were rewritten on purpose for the
# shared one.

RUNNERS = {'run.py': (runner, ['--phase', 'measure']), 'ab.py': (ab, ['--phase', 'measure']),
           'native.py': (native, ['--phase', 'run']), 'mutate.py': (mutate, ['--phase', 'time'])}


def planted_directory(root):
    """`root/out` with what a run that owns it keeps there, so that a refusal that touches it shows."""
    out = Path(root) / 'out'
    out.mkdir()
    for name in ('manifest.json', 'report.json', 'round00-baseline.json', 'verdicts.json', 'stdout.txt'):
        (out / name).write_text('written by the run that owns this directory')
    (out / 'exes').mkdir()
    (out / 'exes' / 'candidate.exe').write_bytes(b'what the owner is measuring')
    return out


class GitEnvironment(unittest.TestCase):
    """R1: every git a runner starts answers for the checkout it was started in, never for the repository that the caller's
    environment names (`GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`...): git then ignores the directory it was started in."""

    def test_git_env_strips_every_name_the_installed_git_lists_as_repository_local(self):
        listed = subprocess.run(['git', 'rev-parse', '--local-env-vars'], capture_output=True, text=True)
        names = listed.stdout.split()
        if listed.returncode != 0 or not names:
            self.skipTest('this git does not answer `rev-parse --local-env-vars`')
        keep = {'HOME': '/home/someone', 'PATH': '/usr/bin', 'LC_ALL': 'C', 'GIT_AUTHOR_NAME': 'someone',
                'GIT_COMMITTER_EMAIL': 'someone@example.com'}
        cleaned = harness.git_env({**{name: 'hostile' for name in names}, **keep})
        self.assertEqual([name for name in names if name in cleaned], [])
        self.assertEqual(cleaned, keep)  # and nothing else is touched: home, path, locale, author and committer stay
        self.assertLessEqual(set(GIT_NAMES), set(harness.GIT_LOCAL_ENV))  # the written-out list holds the 15 known names

    def test_git_env_is_a_copy_and_leaves_the_environment_of_the_process_alone(self):
        with patch.dict(os.environ, {'GIT_DIR': '/elsewhere', 'GIT_AUTHOR_NAME': 'someone'}):
            cleaned = harness.git_env()
            self.assertNotIn('GIT_DIR', cleaned)
            self.assertEqual(cleaned['GIT_AUTHOR_NAME'], 'someone')
            self.assertEqual(os.environ['GIT_DIR'], '/elsewhere')  # the process keeps what it was given
            cleaned['ONLY_IN_THE_COPY'] = '1'
            self.assertNotIn('ONLY_IN_THE_COPY', os.environ)

    ENV_OF_GIT = 'env | grep "^GIT_" | sort'  # a `git` that prints the GIT_* entries of its own environment

    def names_git_saw(self, start):
        """The names of the `GIT_*` entries in the environment of the git that `start()` runs, with every repository-local
        name exported by the caller (and one that is not: `GIT_AUTHOR_NAME`)."""
        hostile = {name: 'hostile' for name in GIT_NAMES}
        with tempfile.TemporaryDirectory() as stubs:
            stub_git(stubs, self.ENV_OF_GIT)
            path = f"{stubs}{os.pathsep}{os.environ['PATH']}"
            with patch.dict(os.environ, {**hostile, 'GIT_AUTHOR_NAME': 'kept', 'PATH': path}):
                text = start()
        return sorted(line.split('=', 1)[0] for line in text.splitlines())

    def test_every_git_a_runner_starts_runs_without_the_repository_variables_of_the_caller(self):
        def identity_text():
            identity = harness.git_identity()
            return '\n'.join(str(identity[field]) for field in ('head', 'tree', 'branch', 'dirty'))

        starts = {
            'harness.git_strict': lambda: harness.git_strict(['status'])[1],
            'harness.git_identity': identity_text,
            'native.runtime_source_tree': native.runtime_source_tree,
            'ab.git_run (ls-tree, archive, show, diff, apply)':
                lambda: ab.git_run(['status'], '.', stdout=subprocess.PIPE, text=True).stdout,
        }
        for label, start in starts.items():
            with self.subTest(label):
                names = self.names_git_saw(start)
                self.assertIn('GIT_AUTHOR_NAME', names)  # the git did run, and what is not repository-local is kept
                self.assertEqual([name for name in names if name in GIT_NAMES], [])

    def test_no_runner_starts_git_except_through_a_start_that_cleans_the_environment(self):
        # a NEW `['git', ...]` that forgets `git_env` fails here, not in the next review
        starts = 0
        for name in ('harness', 'run', 'ab', 'native', 'mutate', 'audit', 'budgets', 'corpus'):
            for number, line in enumerate((HERE / f'{name}.py').read_text().splitlines(), 1):
                if re.search(r"\[\s*'git'", line):
                    starts += 1
                    self.assertIn('git_env', line,
                                  f"{name}.py:{number} starts git with the caller's environment: {line.strip()}")
        self.assertGreaterEqual(starts, 3)  # harness.git_strict, ab.git_run, native.runtime_source_tree

    def test_a_repository_variable_exported_by_the_caller_cannot_make_the_identity_that_of_another_checkout(self):
        with tempfile.TemporaryDirectory() as root, without_git_env():
            a, b = two_repositories(root)
            write(b, 'crates/core/src/lib.rs', '// b, edited after its commit\n')  # B is DIRTY and A is clean
            expected = harness.git_identity(a)
            self.assertEqual(expected['dirty'], '')
            self.assertEqual(expected['head'], git(a, 'rev-parse', 'HEAD').strip())
            self.assertNotEqual(expected['head'], git(b, 'rev-parse', 'HEAD').strip())
            for label, hostile in (('GIT_DIR', {'GIT_DIR': str(b / '.git')}),
                                   ('GIT_DIR and GIT_WORK_TREE', {'GIT_DIR': str(b / '.git'), 'GIT_WORK_TREE': str(b)}),
                                   ('GIT_WORK_TREE', {'GIT_WORK_TREE': str(b)}),
                                   ('GIT_INDEX_FILE', {'GIT_INDEX_FILE': str(b / '.git' / 'index')})):
                with self.subTest(label), patch.dict(os.environ, hostile):
                    self.assertEqual(harness.git_identity(a), expected)


class DirtyIdentity(unittest.TestCase):
    """R2: the identity's `dirty` is non-empty for every input a build of the checkout, or a verdict of a runner, reads from the
    working tree, and stays empty for what none of them reads. A throwaway repository, real git."""

    FILES = {'Cargo.toml': '[workspace]\n', 'Cargo.lock': '# lock\n', 'rust-toolchain.toml': '[toolchain]\n',
             '.cargo/config.toml': '[build]\n', 'vendor/dep/src/lib.rs': '// vendored\n',
             'crates/core/src/x.rs': '// core\n', 'crates/desktop/src/y.rs': '// desktop\n',
             'crates/media/build.rs': 'fn main() {}\n', 'crates/core/Cargo.toml': '[package]\n',
             'crates/core/tests/t.rs': '// test\n', 'runtime/src/x.ts': '// runtime\n', 'runtime/package.json': '{}\n',
             'scripts/gizmo-perf/run.py': '# runner\n', 'docs/migration/fixtures/gizmo-perf/budgets.json': '{}\n',
             'docs/notes.md': 'notes\n', '.gitignore': 'runtime/dist/\nruntime/node_modules/\ntarget/\n'}
    # what `ab.is_prod` calls a product file: one of each kind it recognises
    BUILD_INPUTS = ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', '.cargo/config.toml', 'vendor/dep/src/lib.rs',
                    'crates/core/src/x.rs', 'crates/desktop/src/y.rs', 'crates/media/build.rs', 'crates/core/Cargo.toml')

    def repo(self, root):
        git(root, 'init', '-q')
        for path, text in self.FILES.items():
            write(root, path, text)
        git(root, 'add', '-A')
        git(root, 'commit', '-q', '-m', 'inputs')
        return Path(root)

    def dirty(self, root):
        with without_git_env():
            return harness.git_identity(root)['dirty']

    def test_a_change_to_any_build_input_ab_names_makes_the_identity_dirty(self):
        with tempfile.TemporaryDirectory() as root:
            repo = self.repo(root)
            self.assertEqual(self.dirty(repo), '')
            for path in self.BUILD_INPUTS:
                with self.subTest(path):
                    self.assertTrue(ab.is_prod(path))  # the list is of what the A/B calls a build input
                    (repo / path).write_text((repo / path).read_text() + '// edited\n')
                    self.assertIn(path, self.dirty(repo))
                    git(repo, 'checkout', '--', path)
                    self.assertEqual(self.dirty(repo), '')
        self.assertEqual([path for path in self.BUILD_INPUTS if ab.is_measured(path)],
                         ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', '.cargo/config.toml', 'vendor/dep/src/lib.rs',
                          'crates/core/src/x.rs', 'crates/core/Cargo.toml'])  # the measured subset is the core crate's

    def test_a_file_that_is_new_in_a_build_input_directory_makes_the_identity_dirty(self):
        with tempfile.TemporaryDirectory() as root:
            repo = self.repo(root)
            for path in ('vendor/another/src/lib.rs', 'crates/core/src/new.rs', '.cargo/extra.toml', 'runtime/src/new.ts',
                         'crates/core/tests/new.rs'):
                with self.subTest(path):
                    write(repo, path, '// new\n')
                    try:
                        self.assertNotEqual(self.dirty(repo), '')  # git may print the new directory instead of the file
                    finally:
                        (repo / path).unlink()
                    self.assertEqual(self.dirty(repo), '')

    def test_a_change_to_the_runtime_sources_makes_it_dirty_and_building_the_runtime_does_not(self):
        with tempfile.TemporaryDirectory() as root:
            repo = self.repo(root)
            write(repo, 'runtime/dist/runtime.js', '// built\n')  # what `./scripts/runtime setup` produces: ignored by git
            write(repo, 'runtime/node_modules/dep/index.js', '// installed\n')
            self.assertEqual(self.dirty(repo), '')
            write(repo, 'runtime/src/x.ts', '// runtime, edited\n')
            self.assertIn('runtime/src/x.ts', self.dirty(repo))
            # the runtime page the native editor serves is not a product file for the A/B, on purpose: its executable is the
            # core test. The identity's `dirty` is the one that covers it, because `native.py` runs the page.
            self.assertFalse(ab.is_prod('runtime/src/x.ts'))
            self.assertFalse(ab.is_measured('runtime/src/x.ts'))

    def test_what_no_build_or_verdict_reads_leaves_the_identity_clean(self):
        with tempfile.TemporaryDirectory() as root:
            repo = self.repo(root)
            write(repo, 'docs/notes.md', 'notes, edited\n')
            write(repo, 'docs/new.md', 'a new page\n')
            write(repo, 'README.md', 'untracked at the top\n')
            write(repo, 'target/release/x', 'built\n')  # ignored
            self.assertEqual(self.dirty(repo), '')

    def test_the_verdict_inputs_keep_making_the_identity_dirty(self):
        with tempfile.TemporaryDirectory() as root:
            repo = self.repo(root)
            for path in ('scripts/gizmo-perf/run.py', 'docs/migration/fixtures/gizmo-perf/budgets.json'):
                with self.subTest(path):
                    (repo / path).write_text('edited\n')
                    self.assertIn(path, self.dirty(repo))
                    git(repo, 'checkout', '--', path)

    def test_the_pathspecs_are_declared_once_and_cover_every_tracked_file_ab_calls_a_build_input(self):
        specs = harness.DIRTY_PATHSPECS
        self.assertEqual(harness.IDENTITY_QUERIES['dirty'], ['status', '--porcelain', '--', *specs])
        for name in ('crates', 'Cargo.toml', 'Cargo.lock', 'scripts', 'docs/migration/fixtures/gizmo-perf', '.cargo',
                     'rust-toolchain.toml', 'vendor', 'runtime'):
            self.assertIn(name, specs)
        ok, listing = harness.git_strict(['ls-files'], harness.ROOT)
        self.assertTrue(ok, listing)
        tracked = listing.splitlines()
        build_inputs = [path for path in tracked if ab.is_prod(path)]
        self.assertGreater(len(build_inputs), 20)  # the real tree has product files: this is not vacuous
        covered = [path for path in build_inputs if any(path == spec or path.startswith(spec + '/') for spec in specs)]
        self.assertEqual(sorted(set(build_inputs) - set(covered)), [])
        self.assertTrue(all(ab.is_prod(path) for path in tracked if ab.is_measured(path)))  # measured is a subset


class OneLockPerOut(unittest.TestCase):
    """R3: ONE lock file for `--out`, whatever the runner and the phase, so that a `mutate.py prepare` (which empties
    `exes/`) cannot run over a `run.py measure` of the same directory."""

    def test_there_is_one_lock_name_and_no_runner_keeps_one_of_its_own(self):
        self.assertEqual(harness.OUT_LOCK, SHARED_LOCK)
        self.assertEqual(native.harness.OUT_LOCK, SHARED_LOCK)
        for name, (module, _) in RUNNERS.items():
            self.assertFalse(hasattr(module, 'OUT_LOCK'), f'{name} still has a lock name of its own')
        self.assertNotIn('lock_name', inspect.signature(harness.guarded).parameters)

    def test_the_refusal_does_not_name_only_the_refused_runner_as_the_holder(self):
        with tempfile.TemporaryDirectory() as root:
            out = Path(root) / 'out'
            with held(out / SHARED_LOCK), bounded(), harness.guarded(out, 'ab.py') as refusal:
                self.assertEqual(refusal['status'], 'REFUSED')
                reason = refusal['reason']
            self.assertIn(str(out / SHARED_LOCK), reason)
            self.assertIn('not waited for', reason)
            for name in RUNNERS:  # any of them may be the holder
                self.assertIn(name, reason)
            self.assertIn('one --out per runner', reason)

    def test_a_runner_of_another_kind_on_an_owned_out_is_refused_for_every_ordered_pair(self):
        for first, second in itertools.permutations(RUNNERS, 2):
            with self.subTest(first=first, second=second), tempfile.TemporaryDirectory() as root:
                out = planted_directory(root)
                (module_a, argv_a), (module_b, argv_b) = RUNNERS[first], RUNNERS[second]
                seen = {}
                tripwire = MagicMock(side_effect=AssertionError(f'{second} started its phase on a --out that {first} owns'))

                def holder(args, directory):
                    """The phase of the FIRST runner: while it owns `--out`, the second one is started."""
                    seen['before'] = tree_of(directory)
                    printed = io.StringIO()
                    seen['code'] = run_main(module_b, [*argv_b, '--out', str(directory)], printed)
                    seen['after'], seen['printed'] = tree_of(directory), printed.getvalue()
                    return 0

                with patch.object(module_a, 'run_phase', side_effect=holder), patch.object(module_b, 'run_phase', tripwire), \
                        bounded():
                    self.assertEqual(run_main(module_a, [*argv_a, '--out', str(out)]), 0)
                tripwire.assert_not_called()
                self.assertEqual(seen['code'], 1)
                self.assertEqual(seen['after'], seen['before'])  # nothing created, changed or removed; the planted files stay
                lines = seen['printed'].strip().splitlines()
                self.assertEqual(len(lines), 1, lines)
                refusal = json.loads(lines[0])
                self.assertEqual(refusal['status'], 'REFUSED')
                self.assertIn(SHARED_LOCK, refusal['reason'])
                self.assertIn(second, refusal['reason'])
                self.assertIn('not waited for', refusal['reason'])
                # the control: once the first has finished the lock is free, and the second is not refused for the directory
                after = MagicMock(return_value=0)
                with patch.object(module_b, 'run_phase', after), bounded():
                    self.assertEqual(run_main(module_b, [*argv_b, '--out', str(out)]), 0)
                after.assert_called_once()


HOLDER = textwrap.dedent("""
    import sys
    sys.path.insert(0, sys.argv[1])
    import harness
    with harness.exclusive(sys.argv[2]) as owned:
        print('held' if owned else 'busy', flush=True)
        sys.stdin.read()  # until the test closes the pipe
""")


class TwoRunnerProcesses(unittest.TestCase):
    """R3, with real processes: a helper holds the lock of `--out` through `harness.exclusive`, and each runner is started as
    its own process (no phase that builds, measures or opens a window: they refuse at the lock, or they refuse for something
    else and the test says so)."""

    def test_every_runner_started_while_another_process_owns_the_out_is_refused_with_one_line_and_exit_1(self):
        with tempfile.TemporaryDirectory() as root:
            out = planted_directory(root)
            holder = subprocess.Popen([sys.executable, '-c', HOLDER, str(HERE), str(out / SHARED_LOCK)],
                                      stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
            try:
                with bounded(30):
                    self.assertEqual(holder.stdout.readline().strip(), 'held')
                before = tree_of(out)
                for name, (_, argv) in RUNNERS.items():
                    with self.subTest(name):
                        finished = subprocess.run([sys.executable, str(HERE / name), *argv, '--out', str(out)],
                                                  capture_output=True, text=True, timeout=60, cwd=str(HERE))
                        self.assertEqual(finished.returncode, 1, finished.stderr[-300:])
                        lines = finished.stdout.strip().splitlines()
                        self.assertEqual(len(lines), 1, lines)
                        refusal = json.loads(lines[0])
                        self.assertEqual(refusal['status'], 'REFUSED')
                        self.assertIn(SHARED_LOCK, refusal['reason'])
                        self.assertIn(name, refusal['reason'])
                        self.assertEqual(tree_of(out), before)  # the planted files, and not a new one
            finally:
                holder.stdin.close()
                try:
                    holder.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    holder.kill()
                    holder.wait()
                holder.stdout.close()


class TimedMonitor:
    """What a bounded wait asks of a window: `viewport(timeout)`, on the virtual clock. The page answers `state` after `delay`
    seconds; a query given less time fails the way a socket with that timeout does (`TimeoutError`, after the time it was
    given), unless `honours_timeout` is False: a page that answers when it likes, however late."""

    def __init__(self, clock, state, delay=0.0, honours_timeout=True):
        self.clock, self.state, self.delay, self.honours = clock, state, delay, honours_timeout
        self.asked = []

    def viewport(self, timeout=90):
        self.asked.append(timeout)
        if self.honours and self.delay > timeout:
            self.clock.sleep(timeout)
            raise TimeoutError('timed out')
        self.clock.sleep(self.delay)
        return json.loads(json.dumps(self.state))


class BoundedPolls(unittest.TestCase):
    """R5 and the same class in `run_corpus`: a wait with a budget gives EVERY query the time that is left, judges the budget
    again after the answer (a late answer is a timeout, never a pass), and records a timeout as a problem, not a traceback."""

    REQUEST = {'scene': 'proof', 'frame': 24}
    TOOL = {'tool': 'scale', 'space': 'local'}

    def ack(self, monitor, clock, seconds=2):
        started = clock.now
        with patch.object(native, 'time', clock), bounded(10):
            result = native.overlay_ack(monitor, self.REQUEST, STATE_ANSWER, self.TOOL, seconds=seconds)
        self.spent = clock.now - started
        return result

    def test_every_query_of_the_acknowledgement_wait_gets_the_time_that_is_left(self):
        clock = VirtualClock()
        # a page that always shows a previous step's acknowledgement
        monitor = TimedMonitor(clock, {'overlay': {'drawn': PREVIOUS_ACK}}, delay=0.5)
        ack, problems, expected = self.ack(monitor, clock)
        self.assertTrue(problems)
        self.assertGreater(len(monitor.asked), 1)
        self.assertTrue(all(0 < timeout <= 2 for timeout in monitor.asked), monitor.asked)  # never the 90 s default
        self.assertEqual(monitor.asked, sorted(monitor.asked, reverse=True))  # what is left only shrinks
        self.assertLessEqual(self.spent, 2 + 0.1)  # the wait ends at its seconds (no query past them)

    def test_a_page_that_never_answers_ends_the_wait_at_its_seconds_with_a_recorded_problem(self):
        clock = VirtualClock()
        monitor = TimedMonitor(clock, {'overlay': {'drawn': THIS_ACK}}, delay=1000)
        ack, problems, expected = self.ack(monitor, clock)  # no exception
        self.assertIsNone(ack)
        self.assertEqual(len(problems), 1)
        self.assertIn('did not answer', problems[0])
        self.assertIn('2 s', problems[0])  # the budget is in the text
        self.assertEqual(monitor.asked, [2.0])
        self.assertAlmostEqual(self.spent, 2, delta=0.05)

    def test_a_matching_acknowledgement_that_arrives_after_the_budget_is_a_timeout_not_an_acknowledgement(self):
        clock = VirtualClock()
        monitor = TimedMonitor(clock, {'overlay': {'drawn': THIS_ACK}}, delay=7, honours_timeout=False)
        ack, problems, expected = self.ack(monitor, clock, seconds=5)
        self.assertEqual(ack, THIS_ACK)  # what it showed stays as the evidence...
        self.assertEqual(len(problems), 1)  # ...but it does not count
        self.assertIn('too late', problems[0])
        self.assertIn('5 s', problems[0])

    def test_a_matching_acknowledgement_in_time_passes(self):
        clock = VirtualClock()
        monitor = TimedMonitor(clock, {'overlay': {'drawn': THIS_ACK}}, delay=0.3)
        ack, problems, expected = self.ack(monitor, clock, seconds=5)
        self.assertEqual((ack, problems), (THIS_ACK, []))
        self.assertEqual(len(monitor.asked), 1)

    def test_a_poll_is_always_made_even_with_no_budget_and_it_is_given_a_little_time(self):
        clock = VirtualClock()
        monitor = TimedMonitor(clock, {'overlay': {'drawn': PREVIOUS_ACK}})
        ack, problems, expected = self.ack(monitor, clock, seconds=0)
        self.assertEqual(len(monitor.asked), 1)
        self.assertTrue(0 < monitor.asked[0] <= 0.1, monitor.asked)
        self.assertTrue(problems)  # a previous step's acknowledgement is still a mismatch, and not a traceback

    def page_hidden(self, monitor, clock, seconds=3):
        with patch.object(native, 'time', clock), bounded(10):
            return native.page_hidden_by_dialog(monitor, seconds)

    def test_the_dialog_wait_gives_every_query_the_time_that_is_left(self):
        clock = VirtualClock()
        shown = TimedMonitor(clock, {'on_screen': True})  # a page that stays on screen, and a monitor that answers at once
        hidden, monitor, problem = self.page_hidden(shown, clock)
        self.assertEqual((hidden, problem), (False, None))
        self.assertGreater(len(shown.asked), 1)
        self.assertTrue(all(0 < timeout <= 3 for timeout in shown.asked), shown.asked)
        gone = TimedMonitor(clock, {'on_screen': False}, delay=0.1)  # the control: a page the dialog hid
        self.assertEqual(self.page_hidden(gone, clock)[0::2], (True, None))
        self.assertEqual(len(gone.asked), 1)

    def test_a_dialog_wait_whose_monitor_never_answers_or_answers_late_is_a_problem_not_a_pass(self):
        clock = VirtualClock()
        silent = TimedMonitor(clock, {'on_screen': False}, delay=1000)
        hidden, monitor, problem = self.page_hidden(silent, clock)
        self.assertFalse(hidden)
        self.assertIn('did not answer', problem)
        self.assertEqual(silent.asked, [3.0])
        late = TimedMonitor(clock, {'on_screen': False}, delay=5, honours_timeout=False)  # hidden, but after the 3 s
        hidden, monitor, problem = self.page_hidden(late, clock)
        self.assertFalse(hidden)
        self.assertIn('too late', problem)
        self.assertEqual(monitor['on_screen'], False)  # what it showed is kept for the record

    def test_a_dialog_step_whose_monitor_does_not_answer_is_a_failed_check_not_a_traceback(self):
        class Silent(SavingWindow):
            asked = []

            def send(self, request, timeout=90):
                if request.get('target') == 'monitor.viewport' and getattr(self, 'invoked', False):
                    self.invoked = False  # only the first read after the dialog opened: the rest of the run answers
                    Silent.asked.append(timeout)
                    raise TimeoutError('timed out')
                if request.get('command') == 'invoke':
                    self.invoked = True
                return super().send(request, timeout)

        Silent.asked = []
        steps = [OPEN_PROJECT, {'id': 'present.dialog.open', 'phase': 'present', 'ui': ['invoke', 'flow.open']}]
        result = run_one_corpus(Silent, steps)
        label = 'present.dialog.open: the dialog hid the native page'
        found = [c for c in result['checks'] if c['check'] == label]
        self.assertEqual(len(found), 1)
        self.assertFalse(found[0]['ok'])
        self.assertIn('did not answer', found[0]['detail'])
        self.assertTrue(Silent.asked and all(0 < timeout <= 3 for timeout in Silent.asked), Silent.asked)

    def test_a_job_get_that_never_answers_is_a_failed_check_bounded_by_the_60_seconds_not_a_traceback(self):
        class Queued(SavingWindow):
            asked = []

            def op(self, request, timeout=90):
                if request['op'] == 'scene.resources.resolve':
                    return True, {'state': 'queued', 'job': 'j1'}
                if request['op'] == 'job.get':
                    Queued.asked.append(timeout)
                    raise TimeoutError('timed out')  # a monitor that never answers: at once here, so nothing waits
                return super().op(request, timeout)

        Queued.asked = []
        steps = [OPEN_PROJECT, {'id': 'resolve', 'phase': 'hot', 'op': {'op': 'scene.resources.resolve', 'scene': '$scene'}}]
        result = run_one_corpus(Queued, steps)
        found = [c for c in result['checks'] if c['check'] == 'resolve: the job finished']
        self.assertEqual(len(found), 1)
        self.assertFalse(found[0]['ok'])
        self.assertIn('did not answer', found[0]['detail'])
        self.assertTrue(Queued.asked and all(0 < timeout <= 60 for timeout in Queued.asked), Queued.asked)


# --- round 9 (PR #44): what `settle_commit` does with the FIRST sighting of an acknowledgement ---------------------------
# Two documented behaviours that the 18 tests of `LateAcknowledgement` and `Release` did not pin: a mutation of each of them
# survived those tests (`spec-round9-ack-tests-vs-mutants.log`). The tests run on fakes: no benchmark, no window, no build.


class ScriptedReads(native.Window):
    """`settle_commit` over a monitor whose reads are scripted one by one: only `viewport` is scripted (like `SlowMonitor`) and a
    read takes the virtual clock `cost` seconds, the answer reflecting the state at the END of the read (like `HostWindow`).
    Each entry of `script` is `(cost, converged, matches)`: the monitor shows the candidate's picture converged (`converged`) or
    a redraw still in flight, and the page's acknowledgement is the one of the committed state (`matches`) or still the one of
    the revision before. A read the script does not have is an error, never a repeat of the last one: the scenario is exactly
    what the script says."""

    def __init__(self, clock, script, step, candidate, scene='proof'):
        self.clock, self.script, self.step, self.candidate, self.scene = clock, list(script), step, candidate, scene
        self.deadline = None

    def shows(self, converged, matches):
        """The state the monitor answers with (the shape `HostWindow.viewport` answers for the candidate's picture)."""
        revision = HostWindow.REVISION + (1 if matches else 0)
        return {'phase': 'ready', 'headline': '', 'overlay': {'supported': True, 'drawn': release_overlay(self.step, revision)},
                'directed': {'scene': self.scene, 'queued': None, 'suspended': False,
                             'active': None if converged else {'id': self.candidate[0] + 1},
                             'shown': {'request_id': self.candidate[0], 'edit_frame': 24}},
                'last_frame': {'request': self.candidate[0], 'revision': self.candidate[0], 'hash': self.candidate[1]}}

    def viewport(self, timeout=90):
        if not self.script:
            raise AssertionError('settle_commit read the monitor once more than the script has')
        cost, converged, matches = self.script.pop(0)
        self.clock.sleep(cost)
        return self.shows(converged, matches)


class LateAcknowledgement(GestureCase):
    """`settle_commit`: a MATCHING acknowledgement counts only when the read that first saw it came within `ACK_SECONDS` of the
    first read of the candidate's picture. A slow read cannot say when the acknowledgement really arrived, and a runner that
    cannot know turns that into `unconfirmed`, never into a dedupe. Two more properties are pinned (round 9): the lateness is
    judged on the read that FIRST saw the acknowledgement, never on the time of the reads that follow it (a slow read after an
    acknowledgement seen in time cannot make it late), and an acknowledgement that stops matching starts over: only its NEXT
    match counts, because a match older than the gap says nothing about what the page shows after it.

    The reset of `acked_at` together with `first_at` when the picture stops converging is NOT covered here, on purpose: it is
    defensive and no read sequence can tell it from `first_at = None` alone (the stale `acked_at` is older than the new
    `first_at`, so it is never late, and a read whose acknowledgement does not match zeroes it before a later match could use
    it), so a test of it would pass on both versions (an equivalent mutant, docs/features/scene-gizmo-performance-timing.md)."""

    def settle_script(self, script):
        """`settle_commit` for the translate release of `STEP` over `ScriptedReads(script)`: the candidate is `CANDIDATE`, the
        release answered the overlay of the revision after `HostWindow.REVISION` and the page must acknowledge it."""
        clock = VirtualClock()
        window = ScriptedReads(clock, script, self.STEP, self.CANDIDATE)
        overlay = release_overlay(self.STEP, HostWindow.REVISION + 1)
        info = native.picture(window.shows(converged=True, matches=True))
        started = clock.now
        with patch.object(native, 'time', clock), bounded(10):
            tool = {'tool': self.STEP['tool'], 'space': self.STEP['space']}
            settled = window.settle_commit('proof', 24, info, overlay, tool)
        self.spent = clock.now - started
        self.unread = window.script  # the reads the scenario had and `settle_commit` did not make
        return settled

    def settle(self, ack_delay, read_cost):
        clock = VirtualClock()
        window = HostWindow(clock, self.STEP, base=self.BASE, candidate=self.CANDIDATE, release='dedupe', ack_delay=ack_delay)
        window.shown = self.CANDIDATE
        ok, up = window.op({'op': 'scene.pointer.up'})  # the release, at T: the acknowledgement is planned at T + ack_delay
        info = native.picture(window.viewport())
        window.read_cost = read_cost  # from now on every read takes that long
        started = clock.now
        with patch.object(native, 'time', clock), bounded(10):
            tool = {'tool': self.STEP['tool'], 'space': self.STEP['space']}
            settled = window.settle_commit('proof', 24, info, up['overlay'], tool)
        self.spent = clock.now - started
        return settled

    def test_a_matching_acknowledgement_first_seen_after_the_ack_budget_is_unconfirmed_and_names_the_budget(self):
        # reads at T+2 (the picture is first seen), +4.01 and +6.02 still show the previous revision; the acknowledgement is
        # there at T+7, and the read that sees it ends at T+8.03: 6 s after the picture, past the 5 s
        settled = self.settle(ack_delay=7.0, read_cost=2.0)
        self.assertIsNone(settled['outcome'])
        self.assertIn('unconfirmed', settled['problem'])
        self.assertIn(f'{native.ACK_SECONDS:g} s budget', settled['problem'])
        self.assertEqual(settled['observed']['ack_problems'], [])  # the last read DID match: it came too late to count
        self.assertEqual(settled['observed']['shown_request'], self.CANDIDATE[0])

    def test_a_matching_acknowledgement_first_seen_within_the_ack_budget_is_a_dedupe_however_slow_the_reads(self):
        settled = self.settle(ack_delay=3.0, read_cost=2.0)  # the read at T+4.01 sees it: 2 s after the picture
        self.assertEqual(settled['outcome'], 'deduped')
        self.assertIsNone(settled['problem'])

    def test_an_acknowledgement_the_page_never_matches_is_still_given_up_after_the_budget(self):
        settled = self.settle(ack_delay=10_000.0, read_cost=0.5)
        self.assertIsNone(settled['outcome'])
        self.assertIn('never acknowledged', settled['problem'])
        self.assertLess(self.spent, native.ACK_SECONDS + 1.5)

    def test_an_acknowledgement_seen_with_the_picture_is_in_time_even_when_the_slow_reads_that_follow_pass_the_ack_budget(self):
        # every read takes 6 s, longer than the 5 s budget. The first read already shows the picture AND the matching
        # acknowledgement, so as far as any read can tell it was there from the start; the second ends 6 s after the first
        # sighting and shows the same thing. Only the read that FIRST saw the acknowledgement is judged against the budget
        # (it was 0 s after the picture): the time the later reads take is not evidence that the acknowledgement was late
        # (the ack_delay=3/read_cost=2 test above cannot tell the two rules apart: the read that first saw the acknowledgement
        # is also the current one there, so both distances are the same 2 s).
        settled = self.settle(ack_delay=0.0, read_cost=6.0)
        self.assertEqual(settled['outcome'], 'deduped')
        self.assertIsNone(settled['problem'])
        self.assertEqual(settled['observed']['ack_problems'], [])
        self.assertEqual(settled['observed']['reads'], 2)  # one that saw the picture and its acknowledgement, one for the quiet period
        self.assertGreater(self.spent, native.ACK_SECONDS)  # the wait itself outlasted the budget: that is not lateness

    def test_an_acknowledgement_that_matched_then_stopped_and_matched_again_after_the_budget_is_unconfirmed_not_deduped(self):
        # reads as (cost, the picture is the candidate's and converged, the acknowledgement matches):
        #   0.01 s: yes, yes   the picture is first seen and the acknowledgement already matches
        #   2.0 s:  yes, NO    the acknowledgement stops matching (still the revision before)
        #   3.5 s:  yes, yes   it matches again, 5.52 s after the picture was first seen: past the 5 s budget
        # What the first read saw is not evidence about the page after the gap: "first seen" starts over at the read that
        # saw it again, which is late. A stale first sighting (0 s) would make this a dedupe, a false pass.
        settled = self.settle_script([(0.01, True, True), (2.0, True, False), (3.5, True, True)])
        self.assertIsNone(settled['outcome'])
        self.assertIn('unconfirmed', settled['problem'])
        self.assertIn('only 5.5 s after the candidate\'s picture was first seen', settled['problem'])
        self.assertIn(f'{native.ACK_SECONDS:g} s budget', settled['problem'])
        self.assertEqual(settled['observed']['ack_problems'], [])  # the last read DID match: it came too late to count
        self.assertEqual(settled['observed']['reads'], 3)
        self.assertEqual(self.unread, [])  # the verdict came at the third read, the one that saw it match again


# --- round 10 (PR #44): CodeRabbit review 5861824134 ----------------------------------------------------------------------
# F2: `commit_job` (the job wait of `run_corpus`) judges its deadline again AFTER an answer, like every other bounded wait.
# F3 (BL015.23 (26)): a redo is compared with the picture its commit left, and the two history stacks follow the document,
# not the pictures. Everything runs on fakes under a virtual clock: no benchmark, no window, no build, and NOTHING WAS
# MEASURED. The replay of recorded hashes reads the first real run's report (12bc8da, immutable evidence), like
# `RecordedFirstRun` does.

OPEN_STEPS = [
    {'id': 'open.project', 'phase': 'cold', 'op': {'op': 'project.create', 'name': 'x'}},
    {'id': 'open.attach', 'phase': 'cold', 'op': {'op': 'scene.attach', 'id': '$scene', 'path': '$bundle'}},
    {'id': 'open.show', 'phase': 'cold', 'op': {'op': 'monitor.show_scene', 'scene': '$scene'}},
    {'id': 'open.seek', 'phase': 'cold', 'op': {'op': 'seek', 'frame': '$frame'}},
    {'id': 'open.first_frame', 'phase': 'cold', 'wait': 'presented', 'frame': '$frame'}]
HISTORY_STEPS = ('history.undo.translate', 'history.redo.translate', 'history.undo.translate.again')
WRONG_HASH = 'ffffffffffffffff'  # a pixel hash no picture of these tests has


def recorded_history():
    """`{corpus: {step id: pixel hash}}` of the three history steps that follow the translate gesture in the first real window
    run (12bc8da: immutable evidence): the hash the monitor showed after the undo, the redo and the undo again."""
    report = json.load(gzip.open(RECORDED_RUN))
    return {result['corpus']: {sid: result['steps'][sid]['picture']['hash'] for sid in HISTORY_STEPS}
            for result in report['results']}


class UndoableCommit(GestureCase):
    """What `native.drag` keeps of a commit for the history steps that follow it (BL015.23 (26a)): the hash of the picture the
    commit left, which a redo has to show again. It is None when the release had no legitimate outcome: there is then no
    picture of the commit to compare a redo with, and none is invented."""

    def kept(self, step=None, **model):
        self.run_gesture(step, **model)
        self.assertEqual(len(self.undoable), 1, self.undoable)
        return self.undoable[0]

    def test_a_dedupe_keeps_the_hash_of_the_candidates_picture_the_monitor_kept(self):
        entry = self.kept(release='dedupe')
        self.assertEqual((entry.base_hash, entry.committed_hash, entry.changed), ('base', 'candidate', True))

    def test_a_redraw_keeps_the_hash_of_the_picture_the_release_left_not_the_candidates(self):
        entry = self.kept(release='redraw', release_picture=(14, 'other'))  # the redraw disagrees with the candidate (a failed check)
        self.assertEqual((entry.committed_hash, entry.changed), ('other', True))

    def test_a_gesture_that_changed_nothing_keeps_the_base_hash_as_the_commit_hash_and_is_marked_unchanged(self):
        entry = self.kept(GestureCase.SCALE, candidate=(13, 'base'))
        self.assertEqual((entry.base_hash, entry.committed_hash, entry.changed), ('base', 'base', False))

    def test_a_release_without_a_legitimate_outcome_keeps_no_commit_hash_and_counts_as_changed(self):
        for model in ({'release': 'hang', 'release_picture': (14, 'candidate')},
                      {'release': 'older', 'release_picture': (12, 'candidate')},
                      {'release': 'rewrite', 'release_picture': (0, 'other')}):
            with self.subTest(model['release']):
                entry = self.kept(**model)
                self.assertIsNone(entry.committed_hash)
                self.assertTrue(entry.changed)  # unknown is not "unchanged": the undo is still asked for real


class HistoryCase(GestureCase):
    """`native.run_corpus` over one gesture and the history steps that follow it, in the order of the real journey (the
    gesture, its undo, its redo, the undo again), with `HostWindow` as the monitor under a virtual clock."""

    DRAGGED = {'transform': [5, 5, 5], 'undo': 1, 'redo': 0}  # what `HostWindow` holds while the gesture is committed
    UNDONE = {'transform': [0, 0, 0], 'undo': 0, 'redo': 1}
    UNDO_RESTORED = 'the undo restored the exact picture of before the gesture'
    REDO_SHOWED = 'the redo showed the exact picture the commit left'
    UNDO_NEW = 'the history.undo drew a NEW picture'
    REDO_NEW = 'the history.redo drew a NEW picture'

    @staticmethod
    def ids(tag):
        return {'undo': f'history.undo.{tag}', 'redo': f'history.redo.{tag}', 'again': f'history.undo.{tag}.again'}

    def run_history(self, corpus='small', step=None, tag='translate', **model):
        step = step or self.STEP
        ids = self.ids(tag)
        steps = [*OPEN_STEPS, step,
                 {'id': ids['undo'], 'phase': 'hot', 'op': {'op': 'history.undo'}},
                 {'id': ids['redo'], 'phase': 'hot', 'op': {'op': 'history.redo'}},
                 {'id': ids['again'], 'phase': 'hot', 'op': {'op': 'history.undo'}}]
        golden = {step['id']: self.DRAGGED, ids['undo']: self.UNDONE, ids['redo']: self.DRAGGED, ids['again']: self.UNDONE}
        journey = dict(NativeFlow.JOURNEY, corpora={corpus: NativeFlow.JOURNEY['corpora']['small']}, steps=steps)
        clock = VirtualClock()
        model = {'base': (10, 'base'), 'candidate': (13, 'candidate'), 'release': 'dedupe', **model}
        with contextlib.ExitStack() as stack, tempfile.TemporaryDirectory() as out:
            stack.enter_context(bounded(20))
            stack.enter_context(patch.object(native, 'time', clock))
            stack.enter_context(patch.object(native, 'Window', lambda binary, home: HostWindow(clock, step, **model)))
            stack.enter_context(patch.object(native, 'author_saved_project', return_value=(
                Path('/tmp/saved.tessel.json'), {'path': '/tmp/saved.tessel.json'})))
            return native.run_corpus('bin', journey, {corpus: golden}, corpus, out, {})

    @staticmethod
    def find(result, sid, fragment):
        """The checks of step `sid` whose label has `fragment`."""
        return [c for c in result['checks'] if c['check'].startswith(f'{sid}: ') and fragment in c['check']]

    def verdicts(self, result, sid, fragment):
        return [c['ok'] for c in self.find(result, sid, fragment)]


class RedoPicture(HistoryCase):
    """The redo of a committed gesture must show the exact picture its commit left, not just a newer one (BL015.23 (26a)): the
    check is real, shows both hashes, is recorded as vacuous where it cannot tell anything, and is never invented."""

    def test_a_redo_that_shows_the_picture_the_commit_left_passes_and_every_history_step_is_judged(self):
        result = self.run_history()
        ids = self.ids('translate')
        self.assertEqual(failed_checks(result), [])
        redo = self.find(result, ids['redo'], self.REDO_SHOWED)
        self.assertEqual([c['ok'] for c in redo], [True])
        self.assertEqual(redo[0]['detail'], str(('candidate', 'candidate')))  # both hashes, the redo's and the commit's
        self.assertEqual(self.verdicts(result, ids['undo'], self.UNDO_RESTORED), [True])
        self.assertEqual(self.verdicts(result, ids['again'], self.UNDO_RESTORED), [True])  # undo, redo, undo: all three judged
        for sid in ids.values():  # a gesture that changed pixels leaves nothing vacuous or unasked
            self.assertNotIn('vacuous_checks', result['steps'][sid])
            self.assertNotIn('unasked_checks', result['steps'][sid])

    def test_a_redo_that_showed_the_base_picture_fails_with_both_hashes_although_it_drew_a_newer_picture(self):
        result = self.run_history(redo_hash='base')  # the redo drew a NEWER picture, but the base one
        ids = self.ids('translate')
        self.assertEqual(failed_checks(result), [f'{ids["redo"]}: {self.REDO_SHOWED}'])
        self.assertEqual(self.verdicts(result, ids['redo'], self.REDO_NEW), [True])  # what the runner asked before: it passes
        self.assertEqual(self.find(result, ids['redo'], self.REDO_SHOWED)[0]['detail'], str(('base', 'candidate')))

    def test_a_redo_that_showed_any_other_picture_fails_too(self):
        result = self.run_history(redo_hash='other')
        ids = self.ids('translate')
        self.assertEqual(failed_checks(result), [f'{ids["redo"]}: {self.REDO_SHOWED}'])
        self.assertEqual(self.find(result, ids['redo'], self.REDO_SHOWED)[0]['detail'], str(('other', 'candidate')))

    def test_a_redrawn_commit_is_judged_the_same_way(self):
        ids = self.ids('translate')
        result = self.run_history(release='redraw', release_picture=self.REDRAWN)
        self.assertEqual(failed_checks(result), [])
        self.assertEqual(self.verdicts(result, ids['redo'], self.REDO_SHOWED), [True])
        result = self.run_history(release='redraw', release_picture=self.REDRAWN, redo_hash='base')
        self.assertEqual(failed_checks(result), [f'{ids["redo"]}: {self.REDO_SHOWED}'])

    def test_a_gesture_that_changed_nothing_records_the_redo_comparison_as_vacuous_and_still_asks_it(self):
        ids = self.ids('scale.uniform')
        scale = dict(corpus='dense', step=GestureCase.SCALE, tag='scale.uniform', candidate=(13, 'base'))
        result = self.run_history(**scale)
        self.assertEqual(failed_checks(result), [])
        vacuous = result['steps'][ids['redo']]['vacuous_checks']
        self.assertEqual([item['check'] for item in vacuous], [f'{ids["redo"]}: {self.REDO_SHOWED}'])
        self.assertIn('left the picture as it was', vacuous[0]['because'])
        detail = self.find(result, ids['redo'], self.REDO_SHOWED)[0]['detail']
        self.assertTrue(detail.startswith('vacuous ('), detail)
        self.assertIn(str(('base', 'base')), detail)
        wrong = self.run_history(**scale, redo_hash='other')  # still asked: vacuous or not, another picture fails
        self.assertEqual(failed_checks(wrong), [f'{ids["redo"]}: {self.REDO_SHOWED}'])

    def test_a_redo_of_a_gesture_without_a_legitimate_release_is_not_compared_and_the_record_says_why(self):
        ids = self.ids('translate')
        result = self.run_history(release='hang', release_picture=(14, 'candidate'))
        self.assertEqual(self.find(result, ids['redo'], self.REDO_SHOWED), [])  # no comparison is invented
        unasked = result['steps'][ids['redo']]['unasked_checks']
        self.assertEqual([item['check'] for item in unasked], [f'{ids["redo"]}: {self.REDO_SHOWED}'])
        self.assertIn('no legitimate picture', unasked[0]['because'])
        self.assertEqual(self.verdicts(result, ids['redo'], self.REDO_NEW), [True])  # "drew a NEW picture" stands
        self.assertEqual(self.verdicts(result, ids['undo'], self.UNDO_RESTORED), [True])  # the undo is still compared with the base

    def test_a_redo_that_drew_no_picture_fails_only_its_own_check_and_is_not_compared_with_a_stale_picture(self):
        ids = self.ids('translate')
        result = self.run_history(redo_draws=False)  # the monitor keeps the base picture the undo left
        self.assertEqual(failed_checks(result), [f'{ids["redo"]}: {self.REDO_NEW}'])
        self.assertEqual(self.find(result, ids['redo'], self.REDO_SHOWED), [])
        self.assertEqual(self.verdicts(result, ids['again'], self.UNDO_RESTORED), [True])  # the redo still moved the stacks

    def test_an_undo_that_drew_no_picture_still_moves_the_stacks_so_the_redo_is_still_compared(self):
        ids = self.ids('translate')
        result = self.run_history(undo_draws=[False])  # only the first undo draws nothing
        self.assertEqual(failed_checks(result), [f'{ids["undo"]}: {self.UNDO_NEW}'])  # only the undo's own failure
        self.assertEqual(self.find(result, ids['undo'], self.UNDO_RESTORED), [])  # nothing to compare: no picture
        self.assertEqual(self.verdicts(result, ids['redo'], self.REDO_SHOWED), [True])  # the gesture is on the redo stack
        self.assertEqual(self.verdicts(result, ids['again'], self.UNDO_RESTORED), [True])
        wrong = self.run_history(undo_draws=[False], redo_hash='other')
        self.assertEqual(failed_checks(wrong), [f'{ids["undo"]}: {self.UNDO_NEW}', f'{ids["redo"]}: {self.REDO_SHOWED}'])


class RecordedRedo(HistoryCase):
    """The first real window run (12bc8da, immutable evidence) holds, in all three corpora, the three history steps that follow
    the translate gesture (undo, redo, undo again) with the hash the monitor showed after each. They are replayed through
    `run_corpus` in that order, on a monitor that shows exactly those hashes (the gesture's base, candidate and commit hashes
    are the report's too). NOT the report's: the request ids and the timing (the model's), and the page's acknowledgement
    after the release (the report never kept it: assumed to match). The replay proves the rule against recorded values, not
    that the product behaves: nothing here ran a window."""

    @classmethod
    def setUpClass(cls):
        cls.gestures = {g.corpus: g for g in recorded_gestures() if g.step['id'] == 'drag.translate.x'}
        cls.history = recorded_history()

    def replay(self, corpus, **model):
        g, shown = self.gestures[corpus], self.history[corpus]
        return self.run_history(corpus=corpus, step=g.step, **{
            'base': (10, g.base[1]), 'candidate': (13, g.candidate[1]), 'undo_hash': shown['history.undo.translate'],
            'redo_hash': shown['history.redo.translate'], **model})

    def test_the_report_holds_what_the_comparison_is_built_from(self):
        self.assertEqual(sorted(self.gestures), ['deep', 'dense', 'small'])
        for corpus, g in self.gestures.items():
            with self.subTest(corpus):
                undo, redo, again = (self.history[corpus][sid] for sid in HISTORY_STEPS)
                self.assertEqual((undo, again), (g.base[1], g.base[1]))  # both undos restored the base picture
                self.assertEqual(redo, g.picture[1])  # the redo showed the picture the gesture's release had left
                self.assertNotEqual(redo, g.base[1])  # a translate changes pixels: the comparison tells something

    def test_the_recorded_order_replayed_with_the_recorded_hashes_passes_and_the_redo_is_really_compared(self):
        ids = self.ids('translate')
        for corpus in self.gestures:
            with self.subTest(corpus):
                result = self.replay(corpus)
                self.assertEqual(failed_checks(result), [])
                redo = self.find(result, ids['redo'], self.REDO_SHOWED)
                shown = self.history[corpus]['history.redo.translate']
                self.assertEqual([c['ok'] for c in redo], [True])
                self.assertEqual(redo[0]['detail'], str((shown, shown)))
                self.assertNotIn('vacuous_checks', result['steps'][ids['redo']])
                self.assertEqual(self.verdicts(result, ids['undo'], self.UNDO_RESTORED), [True])
                self.assertEqual(self.verdicts(result, ids['again'], self.UNDO_RESTORED), [True])

    def test_a_recorded_redo_that_had_shown_the_base_picture_or_any_other_fails_with_both_hashes(self):
        ids = self.ids('translate')
        for corpus, g in self.gestures.items():
            for wrong in (g.base[1], WRONG_HASH):
                with self.subTest(corpus, shows=wrong):
                    result = self.replay(corpus, redo_hash=wrong)
                    self.assertEqual(failed_checks(result), [f'{ids["redo"]}: {self.REDO_SHOWED}'])
                    detail = self.find(result, ids['redo'], self.REDO_SHOWED)[0]['detail']
                    self.assertEqual(detail, str((wrong, self.history[corpus]['history.redo.translate'])))
                    self.assertEqual(self.verdicts(result, ids['redo'], self.REDO_NEW), [True])  # it did draw a newer picture

    def test_the_recorded_dense_scale_that_changed_nothing_records_the_redo_comparison_as_vacuous(self):
        g = next(g for g in recorded_gestures() if (g.corpus, g.step['id']) == ('dense', 'drag.scale.local.uniform'))
        self.assertEqual(g.candidate[1], g.base[1])  # the observation behind H17: the scale pictures are the base picture
        ids = self.ids('scale.uniform')
        scale = dict(corpus='dense', step=g.step, tag='scale.uniform', base=(10, g.base[1]), candidate=(13, g.candidate[1]))
        result = self.run_history(**scale)
        self.assertEqual(failed_checks(result), [])
        vacuous = result['steps'][ids['redo']]['vacuous_checks']
        self.assertEqual([item['check'] for item in vacuous], [f'{ids["redo"]}: {self.REDO_SHOWED}'])
        detail = self.find(result, ids['redo'], self.REDO_SHOWED)[0]['detail']
        self.assertTrue(detail.startswith('vacuous ('), detail)
        self.assertIn(str((g.base[1], g.base[1])), detail)
        wrong = self.run_history(**scale, redo_hash=WRONG_HASH)
        self.assertEqual(failed_checks(wrong), [f'{ids["redo"]}: {self.REDO_SHOWED}'])

    def test_a_recorded_undo_that_drew_no_picture_still_moves_the_stacks_so_the_redo_is_checked(self):
        ids = self.ids('translate')
        for corpus in self.gestures:
            with self.subTest(corpus):
                result = self.replay(corpus, undo_draws=[False])
                self.assertEqual(failed_checks(result), [f'{ids["undo"]}: {self.UNDO_NEW}'])
                self.assertEqual(self.verdicts(result, ids['redo'], self.REDO_SHOWED), [True])
                wrong = self.replay(corpus, undo_draws=[False], redo_hash=WRONG_HASH)
                self.assertEqual(failed_checks(wrong), [f'{ids["undo"]}: {self.UNDO_NEW}',
                                                        f'{ids["redo"]}: {self.REDO_SHOWED}'])


class ScriptedHistory(FakeWindow):
    """A `FakeWindow` whose `history.undo` / `history.redo` are scripted: each pops one `(ok, hash)` of `script`. The request is
    answered `ok`; when `ok` and `hash` is not None the monitor then shows a NEW picture with that hash (None: the host drew
    nothing)."""

    script = []
    picture = None

    def op(self, request, timeout=90):
        if request['op'] in ('history.undo', 'history.redo'):
            FakeWindow.log.append(request['op'])
            ok, shows = ScriptedHistory.script.pop(0)
            if ok and shows is not None:
                self.request += 1
                self.picture = shows
            return ok, {} if ok else {'code': 'nothing_to_undo'}
        return super().op(request, timeout)

    def state(self):
        state = super().state()
        if self.picture is not None and 'last_frame' in state:
            state['last_frame']['hash'] = self.picture
        return state


class HistoryStacks(unittest.TestCase):
    """BL015.23 (26b): the two stacks of `run_corpus` follow the DOCUMENT, not the pictures. `drag` is replaced by a stand-in
    that commits the gestures of a table, so the history steps are judged against known `Undoable`s: `A` and `B` changed
    pixels (base `a0`/`b0`, commit `a1`/`b1`), `Z` changed nothing (`z0` both). The window answers each history request as a
    script says: with the picture it shows afterwards, or with none."""

    GESTURES = {'drag.a': ('a0', 'a1', True), 'drag.b': ('b0', 'b1', True), 'drag.z': ('z0', 'z0', False)}
    UNDO_RESTORED = 'the undo restored the exact picture of before the gesture'
    REDO_SHOWED = 'the redo showed the exact picture the commit left'

    def run_steps(self, gestures, history, script):
        """`run_corpus` over the open steps, one stand-in gesture per id of `gestures` and one history step per id of `history`
        (`history.undo.N` / `history.redo.N`), the window answering the history requests as `script` says."""
        def drag(window, timed, check, step, scene, variables, node, committed, undoable, result, shown):
            base, commit, changed = self.GESTURES[step['id']]
            committed.append(('before', 'after'))
            undoable.append(native.Undoable(base, changed, step['id'], commit))

        steps = [*OPEN_STEPS, *({'id': sid, 'kind': 'drag', 'phase': 'hot'} for sid in gestures),
                 *({'id': sid, 'phase': 'hot', 'op': {'op': 'history.redo' if sid.startswith('history.redo') else 'history.undo'}}
                   for sid in history)]
        ScriptedHistory.script = list(script)
        clock = VirtualClock()
        FakeWindow.log, FakeWindow.stale_shows, FakeWindow.instances = [], set(), []
        with contextlib.ExitStack() as stack, tempfile.TemporaryDirectory() as out:
            stack.enter_context(bounded(20))
            stack.enter_context(patch.object(native, 'time', clock))
            stack.enter_context(patch.object(native, 'Window', ScriptedHistory))
            stack.enter_context(patch.object(native, 'drag', drag))
            stack.enter_context(patch.object(native, 'author_saved_project', return_value=(
                Path('/tmp/saved.tessel.json'), {'path': '/tmp/saved.tessel.json'})))
            result = native.run_corpus('bin', dict(NativeFlow.JOURNEY, steps=steps), {'small': {}}, 'small', out, {})
        self.assertEqual(ScriptedHistory.script, [])  # every scripted answer was used: the scenario is what the script says
        return result

    @staticmethod
    def found(result, sid, fragment):
        return [c for c in result['checks'] if c['check'] == f'{sid}: {fragment}']

    def test_an_undo_without_a_picture_moves_the_stacks_so_the_next_undo_is_judged_against_the_gesture_before_it(self):
        # `B` was undone by the first undo (no picture), so the second undo takes `A` off: it must be compared with `a0`.
        # Compared with `B` (the stacks unmoved) it would fail against `b0` and blame the second undo for the first one's.
        result = self.run_steps(['drag.a', 'drag.b'], ['history.undo.1', 'history.undo.2'], [(True, None), (True, 'a0')])
        self.assertEqual(failed_checks(result), ['history.undo.1: the history.undo drew a NEW picture'])
        second = self.found(result, 'history.undo.2', self.UNDO_RESTORED)
        self.assertEqual([c['ok'] for c in second], [True])
        self.assertEqual(second[0]['detail'], str(('a0', 'a0')))

    def test_a_redo_after_an_undo_without_a_picture_is_compared_with_the_commit_of_the_gesture_that_undo_took_off(self):
        result = self.run_steps(['drag.a', 'drag.b'], ['history.undo.1', 'history.redo.1'], [(True, None), (True, 'b1')])
        self.assertEqual(failed_checks(result), ['history.undo.1: the history.undo drew a NEW picture'])
        self.assertEqual(self.found(result, 'history.redo.1', self.REDO_SHOWED)[0]['detail'], str(('b1', 'b1')))
        wrong = self.run_steps(['drag.a', 'drag.b'], ['history.undo.1', 'history.redo.1'], [(True, None), (True, 'a1')])
        self.assertEqual(failed_checks(wrong), ['history.undo.1: the history.undo drew a NEW picture',
                                                f'history.redo.1: {self.REDO_SHOWED}'])  # `a1` is the other gesture's commit
        self.assertEqual(self.found(wrong, 'history.redo.1', self.REDO_SHOWED)[0]['detail'], str(('a1', 'b1')))

    def test_the_vacuous_record_of_a_redo_names_the_gesture_the_stacks_say_was_redone(self):
        # `Z` left the picture as it was: its redo is recorded as vacuous, and by `Z`'s name, after an undo that drew nothing
        result = self.run_steps(['drag.a', 'drag.z'], ['history.undo.1', 'history.redo.1'], [(True, None), (True, 'z0')])
        self.assertEqual(failed_checks(result), ['history.undo.1: the history.undo drew a NEW picture'])
        vacuous = result['steps']['history.redo.1']['vacuous_checks']
        self.assertEqual([item['check'] for item in vacuous], [f'history.redo.1: {self.REDO_SHOWED}'])
        self.assertIn('drag.z left the picture as it was', vacuous[0]['because'])

    def test_two_undos_and_two_redos_walk_the_gestures_in_order(self):
        script = [(True, 'b0'), (True, 'a0'), (True, 'a1'), (True, 'b1')]
        result = self.run_steps(['drag.a', 'drag.b'], ['history.undo.1', 'history.undo.2', 'history.redo.1', 'history.redo.2'],
                                script)
        self.assertEqual(failed_checks(result), [])
        for sid, detail in (('history.undo.1', ('b0', 'b0')), ('history.undo.2', ('a0', 'a0'))):
            self.assertEqual(self.found(result, sid, self.UNDO_RESTORED)[0]['detail'], str(detail))
        for sid, detail in (('history.redo.1', ('a1', 'a1')), ('history.redo.2', ('b1', 'b1'))):
            self.assertEqual(self.found(result, sid, self.REDO_SHOWED)[0]['detail'], str(detail))

    def test_a_redo_that_drew_no_picture_still_puts_the_gesture_back_on_the_undo_stack(self):
        result = self.run_steps(['drag.a', 'drag.b'], ['history.undo.1', 'history.redo.1', 'history.undo.2'],
                                [(True, 'b0'), (True, None), (True, 'b0')])
        self.assertEqual(failed_checks(result), ['history.redo.1: the history.redo drew a NEW picture'])
        self.assertEqual([c['ok'] for c in self.found(result, 'history.undo.2', self.UNDO_RESTORED)], [True])  # `B` again

    def test_a_history_request_that_was_not_answered_ok_moves_nothing(self):
        result = self.run_steps(['drag.a'], ['history.undo.1', 'history.undo.2'], [(False, None), (True, 'a0')])
        self.assertEqual(failed_checks(result), ['history.undo.1: history.undo answered'])
        self.assertEqual(self.found(result, 'history.undo.1', self.UNDO_RESTORED), [])
        self.assertEqual([c['ok'] for c in self.found(result, 'history.undo.2', self.UNDO_RESTORED)], [True])  # `A` still there


class LateCommitReply(GestureCase):
    """The sweep of F2: `settle_commit` judges its deadline AGAIN after a read. A read that ends after `seconds` and shows a
    legitimate picture (here the release's redraw, which an in-time read calls `redrawn`) came too late to count: the outcome
    is None. The sweep found that no test pinned that branch (a mutant that removed it survived every earlier test)."""

    def settle_late(self, read_cost):
        clock = VirtualClock()
        window = HostWindow(clock, self.STEP, base=self.BASE, candidate=self.CANDIDATE, release='redraw',
                            release_picture=self.REDRAWN)
        window.shown = self.CANDIDATE
        ok, up = window.op({'op': 'scene.pointer.up'})  # the release, at T: the host redraws 40 ms later
        info = native.picture(window.viewport())
        window.read_cost = read_cost  # from now on every read takes that long, whatever timeout it was given
        with patch.object(native, 'time', clock), bounded(10):
            tool = {'tool': self.STEP['tool'], 'space': self.STEP['space']}
            return window.settle_commit('proof', 24, info, up['overlay'], tool, seconds=30)

    def test_a_redraw_that_a_slow_read_saw_after_the_deadline_is_too_late_and_not_redrawn(self):
        settled = self.settle_late(read_cost=31.0)  # the first read ends at T + 31: past the 30 s, and it shows the redraw
        self.assertIsNone(settled['outcome'])
        self.assertIn('after the deadline', settled['problem'])
        self.assertIn('too late to count', settled['problem'])

    def test_the_same_redraw_seen_by_a_read_in_time_is_redrawn(self):
        settled = self.settle_late(read_cost=0.5)
        self.assertEqual(settled['outcome'], 'redrawn')
        self.assertIsNone(settled['problem'])


class LateJobAnswer(unittest.TestCase):
    """F2 of review 5861824134: `commit_job`, the wait of `run_corpus` for a queued job (60 s), judges its deadline AGAIN after
    each answer. The timeout of a socket bounds one call, not how late it answers, so a state that arrives after the deadline
    is too late to count, a final one included, and `job.commit` is never sent after it. Virtual clock: nothing waits."""

    STEPS = [OPEN_PROJECT, {'id': 'resolve', 'phase': 'hot', 'op': {'op': 'scene.resources.resolve', 'scene': '$scene'}}]
    FINISHED, SUCCEEDED, APPLIED = 'resolve: the job finished', 'resolve: the job succeeded', 'resolve: the job was applied'

    def run_job(self, answers):
        """`run_corpus` with a job whose `job.get` answers `answers` one by one (the last one for ever): each is `(seconds,
        state)`, the answer taking that many seconds of the virtual clock to arrive whatever timeout the runner gave it."""
        clock = VirtualClock()

        class Window(SavingWindow):
            script, commits = list(answers), []

            def op(self, request, timeout=90):
                if request['op'] == 'scene.resources.resolve':
                    return True, {'state': 'queued', 'job': 'j1'}
                if request['op'] == 'job.get':
                    seconds, state = Window.script.pop(0) if len(Window.script) > 1 else Window.script[0]
                    clock.sleep(seconds)
                    return True, {'state': state}
                if request['op'] == 'job.commit':
                    Window.commits.append(request['job'])
                    return True, {}
                return super().op(request, timeout)

        result = run_one_corpus(Window, self.STEPS, clock=clock)
        self.commits = Window.commits
        return result

    def only(self, result, label):
        found = [c for c in result['checks'] if c['check'] == label]
        self.assertEqual(len(found), 1, (label, [c['check'] for c in result['checks']]))
        return found[0]

    def labels(self, result):
        return [c['check'] for c in result['checks']]

    def test_a_final_answer_that_arrives_after_the_deadline_is_too_late_and_no_job_commit_is_sent(self):
        result = self.run_job([(61.0, 'succeeded')])
        finished = self.only(result, self.FINISHED)
        self.assertFalse(finished['ok'])
        self.assertIn('after the 60 s deadline', finished['detail'])
        self.assertIn('too late to count', finished['detail'])
        self.assertIn('no `job.commit` was sent', finished['detail'])
        self.assertEqual(self.commits, [])  # the commit after the deadline is what this forbids
        self.assertNotIn(self.APPLIED, self.labels(result))

    def test_every_final_state_that_arrives_late_is_refused_and_none_of_them_is_read(self):
        for state in ('succeeded', 'failed', 'cancelled'):
            with self.subTest(state):
                result = self.run_job([(61.0, state)])
                self.assertFalse(self.only(result, self.FINISHED)['ok'])
                self.assertNotIn(self.SUCCEEDED, self.labels(result))  # a late `failed` is not read either
                self.assertNotIn(self.APPLIED, self.labels(result))
                self.assertEqual(self.commits, [])

    def test_a_late_answer_that_is_not_final_is_refused_with_the_same_detail(self):
        result = self.run_job([(61.0, 'running')])
        finished = self.only(result, self.FINISHED)
        self.assertFalse(finished['ok'])
        self.assertIn('too late to count', finished['detail'])  # the detail used to be empty
        self.assertEqual(self.commits, [])

    def test_the_deadline_belongs_to_the_whole_wait_not_to_a_single_poll(self):
        # the first answer comes 31 s in (in time, not final); the second takes 30 s more: it arrives at 61 s
        result = self.run_job([(31.0, 'running'), (30.0, 'succeeded')])
        self.assertFalse(self.only(result, self.FINISHED)['ok'])
        self.assertIn('too late to count', self.only(result, self.FINISHED)['detail'])
        self.assertEqual(self.commits, [])
        self.assertNotIn(self.APPLIED, self.labels(result))

    def test_an_answer_that_arrives_in_time_behaves_exactly_as_before(self):
        result = self.run_job([(0.5, 'running'), (0.5, 'running'), (0.5, 'succeeded')])
        self.assertEqual(failed_checks(result), [])
        self.assertTrue(self.only(result, self.APPLIED)['ok'])
        self.assertEqual(self.commits, ['j1'])
        self.assertNotIn(self.FINISHED, self.labels(result))  # no check by that name unless the wait failed
        result = self.run_job([(0.5, 'failed')])  # a final state that is not `succeeded` is still its own failed check
        self.assertEqual(failed_checks(result), [self.SUCCEEDED])
        self.assertEqual(self.only(result, self.SUCCEEDED)['detail'], str({'state': 'failed'}))
        self.assertEqual(self.commits, [])

    def test_an_answer_that_arrives_exactly_at_the_deadline_is_in_time_and_one_just_after_it_is_not(self):
        result = self.run_job([(60.0, 'succeeded')])  # the poll starts at T, the answer arrives at T + 60: the deadline itself
        self.assertTrue(self.only(result, self.APPLIED)['ok'])
        self.assertEqual(self.commits, ['j1'])
        result = self.run_job([(60.01, 'succeeded')])
        self.assertFalse(self.only(result, self.FINISHED)['ok'])
        self.assertEqual(self.commits, [])


if __name__ == '__main__':
    unittest.main()
