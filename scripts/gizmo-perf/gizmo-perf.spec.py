#!/usr/bin/env python3
"""Spec of the gizmo performance runners, without a benchmark, a window or the real lock (F055 / BL015.21)."""
import contextlib
import fcntl
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
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


def slot(samples, medians=None, long_ticks=0):
    ordered = sorted(samples)
    return {'samples': list(samples), 'medians': medians or [ordered[len(ordered) // 2]],
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


def run_main(module, argv):
    """A runner's `main` as the command line would call it: its stdout is kept off the test output and the SIGTERM/SIGINT
    handlers it installs are put back."""
    saved = {sig: signal.getsignal(sig) for sig in (signal.SIGTERM, signal.SIGINT)}
    try:
        with contextlib.redirect_stdout(io.StringIO()):
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
        with patch.object(native, 'runtime_source_tree', return_value='rt'):
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
    around that hand-over is faked: git, the machine, the lock, the release build (it names the bytes of an executable,
    or none) and the controlled run, which only notes in `self.measured` the bytes it was asked to measure. The mutated
    file is one in a temporary directory, never the repository's."""

    NAME = 'time_mutant'
    A = {'head': 'a' * 40, 'tree': '1' * 40, 'branch': 'b', 'dirty': ''}
    B = {'head': 'b' * 40, 'tree': '2' * 40, 'branch': 'b', 'dirty': ''}
    LOADED = 'load average (1 min) 9.00 over 5.5 (11 cpus)'

    def phase(self, root, phase, identity=None, built=None, build_exit=0, blocked=None):
        """`main --phase prepare|time --out ROOT/out` for the one mutant, at HEAD `identity`. `built`: the bytes of the
        executable its release build names, `None` for a build that named none (and failed); `build_exit` is that
        build's exit; `blocked` the reason the machine guard refuses a measurement. Returns the exit code and the
        manifest the phase wrote (`manifest.json` of `prepare`, `manifest.final.json` of `time`; `{}` if none)."""
        root = Path(root)
        out = root / 'out'
        source = root / 'f.rs'
        source.write_text('fn a() {}\n')
        state = {'loadavg': (0.0, 0.0, 0.0)}
        self.measured = []

        def release_build():
            if built is None:
                return ['cargo', 'test', '--release'], done('error[E0425]: cannot find value', 101), None
            target = root / 'target-exe'
            target.write_bytes(built)
            return ['cargo', 'test', '--release'], done('', build_exit), str(target)

        def controlled_run(exe, report, directory, name):
            self.measured.append((name, Path(exe).read_bytes()))
            return {'outcome': 'KILLED', 'status': 'KILLED', 'detail': 'faked: nothing was executed'}

        mutants = {self.NAME: {'file': str(source), 'old': 'fn a() {}', 'new': 'fn a() { loop {} }',
                               'detectors': {'time': True}}}
        fakes = [patch.object(mutate, 'MUTANTS', mutants), patch.object(mutate, 'ROOT', Path('/')),
                 patch.object(mutate, 'file_hash', return_value='blob'),
                 patch.object(mutate, 'committed_hash', return_value='blob'),
                 patch.object(mutate, 'run_time', side_effect=controlled_run),
                 patch.object(mutate.runner, 'build', side_effect=release_build),
                 patch.object(mutate.harness, 'git_identity', return_value=identity or self.A),
                 patch.object(mutate.harness, 'toolchain', return_value={}),
                 patch.object(mutate.harness, 'machine', return_value=state),
                 patch.object(mutate.harness, 'guards', return_value=(state, blocked)),
                 patch.object(mutate.harness, 'claims', return_value=[]),
                 patch.object(mutate.harness, 'Lock', FakeLock)]
        with contextlib.ExitStack() as stack:
            for fake in fakes:
                stack.enter_context(fake)
            code = run_main(mutate, ['--phase', phase, '--out', str(out)])
        written = out / ('manifest.json' if phase == 'prepare' else 'manifest.final.json')
        return code, json.loads(written.read_text()) if written.exists() else {}

    def finished_prepare(self, out, recorded, identity=None):
        """What a finished `prepare` left in --out/manifest.json for the mutant: its build recorded `recorded` as the
        hash of the executable (`None`: the build named none). The executable itself is the caller's."""
        manifest = {'suite': 'gizmo-perf-mutants', 'phase': 'prepare', 'status': 'PREPARED',
                    'identity': identity or self.A,
                    'mutants': {self.NAME: {'file': 'f.rs', 'must': {'time': True},
                                            'time_build': {'exit': 0, 'executable_sha256': recorded}}}}
        (Path(out) / 'manifest.json').write_text(json.dumps(manifest))

    def leave(self, out, content):
        """`out/exes/<mutant>.exe` as an earlier run left it; returns the path."""
        exe = Path(out) / 'exes' / f'{self.NAME}.exe'
        exe.parent.mkdir(parents=True, exist_ok=True)
        exe.write_bytes(content)
        return exe

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
            code = native.main(['--binary', str(Path(root) / 'nope'), '--out', root])
            self.assertEqual(code, 1)
            self.assertEqual(json.loads((Path(root) / 'manifest.json').read_text())['status'], 'PREREQUISITE_MISSING')


if __name__ == '__main__':
    unittest.main()
