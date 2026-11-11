#!/usr/bin/env python3
"""Spec of the gizmo performance runners, without a benchmark, a window or the real lock (F055 / BL015.21)."""
import fcntl
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))


def load(name):
    spec = importlib.util.spec_from_file_location(name, HERE / f'{name}.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


harness, ab, mutate, native = (load(n) for n in ('harness', 'ab', 'mutate', 'native'))


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


class Judge(unittest.TestCase):
    key = ('dense', 'move.translate')

    def verdict(self, base, cand):
        return ab.judge({self.key: base}, {self.key: cand})['dense/move.translate']

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
