#!/usr/bin/env python3
"""Spec of the A/B verdict of `ab.py`: each gate has its own majority of rounds (R2).

BL015.24: `gizmo-perf.spec.py` (too long) is to be split one spec per module; this file is the one of `ab.judge`/`ab.pooled`.
The reports are built round by round and pooled by `ab.pooled`, as `ab.py` does, so the per-round statistics are the real ones.
`GIZMO_PERF_AB_UNDER_TEST=<file>` points the spec at ANOTHER `ab.py`: that is how a test is shown to fail against the code
it was written against. No benchmark, no lock, no git."""
import importlib.util
import os
from pathlib import Path
import sys
import unittest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import harness  # noqa: E402,F401  (`ab.py` imports it by name: the module object is shared)


def load(path):
    spec = importlib.util.spec_from_file_location('ab_under_test', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


ab = load(os.environ.get('GIZMO_PERF_AB_UNDER_TEST') or HERE / 'ab.py')

BASE = [1000 + i for i in range(40)]  # one round of a kind: p50 1020, p95 1038


def report(samples):
    """What one kept run of the budget executable reports for the kind `move.translate` of the corpus `dense`."""
    ordered = sorted(samples)
    return {'corpora': [{'corpus': 'dense', 'cold': {}, 'hot': {'move.translate': {
        'samples_ns': list(samples), 'p50_ns': ordered[len(ordered) // 2], 'ratio_p50': 1.0, 'long_ticks': 0,
        'n': len(samples)}}}]}


def tail(samples, share=0.2):
    """The slowest `share` of the samples three times slower, the others as they were: the median does not move."""
    ordered = sorted(samples)
    cut = len(ordered) - int(len(ordered) * share)
    return ordered[:cut] + [3 * value for value in ordered[cut:]]


def nudge(samples):
    """The median sample one nanosecond slower: the median of the round moves, its p95 does not."""
    ordered = sorted(samples)
    ordered[len(ordered) // 2] += 1
    return ordered


def verdict(base_rounds, candidate_rounds):
    pool = [ab.pooled([report(samples) for samples in rounds]) for rounds in (base_rounds, candidate_rounds)]
    return ab.judge(*pool)['dense/move.translate']


class EachGateHasItsOwnMajority(unittest.TestCase):
    def test_identical_sides_are_no_regression(self):
        self.assertEqual(verdict([BASE] * 4, [BASE] * 4)['status'], 'NO_REGRESSION_IN_SCENARIO')

    def test_a_tail_only_slowdown_regresses(self):
        out = verdict([BASE] * 4, [tail(BASE)] * 4)  # 20% of the samples x3, the median of every round as it was
        self.assertEqual(out['status'], 'REGRESSED', out['reasons'])
        self.assertTrue(any(reason.startswith('p95') for reason in out['reasons']), out['reasons'])
        self.assertEqual(out['candidate_slower_pairs'], '0/4')  # the medians say nothing: the p95 gate must not ask them

    def test_a_shift_of_the_median_alone_is_judged_by_the_median_gate(self):
        out = verdict([BASE] * 4, [[round(value * 1.15) for value in BASE]] * 4)  # +15%: over 1.10, under the p95's 1.25
        self.assertEqual(out['status'], 'REGRESSED')
        self.assertEqual([reason.split()[0] for reason in out['reasons']], ['median'])

    def test_the_p95_gate_counts_the_p95_of_each_round_and_never_the_medians(self):
        # The medians of ALL four rounds are one nanosecond slower and the pooled p95 is 3x, but only ONE round has a worse
        # p95: a round with a bad tail is a hiccup, the p95 gate wants the majority of the rounds (as the median gate does).
        # A p95 gate that borrowed the pairs of the medians would count 4/4 here and call it a regression.
        out = verdict([BASE] * 4, [nudge(tail(BASE))] + [nudge(BASE)] * 3)
        self.assertEqual(out['status'], 'NO_REGRESSION_IN_SCENARIO', out['reasons'])
        self.assertEqual(out['candidate_slower_pairs'], '4/4')
        self.assertEqual(out['candidate_slower_p95_pairs'], '1/4')

    def test_a_bad_tail_in_most_rounds_regresses(self):
        out = verdict([BASE] * 4, [nudge(tail(BASE))] * 3 + [nudge(BASE)])
        self.assertEqual(out['candidate_slower_p95_pairs'], '3/4')
        self.assertEqual(out['status'], 'REGRESSED')
        self.assertTrue(any(reason.startswith('p95') for reason in out['reasons']), out['reasons'])

    def test_the_pool_keeps_the_p95_of_each_round_beside_its_median(self):
        pool = ab.pooled([report(BASE), report(tail(BASE))])[('dense', 'move.translate')]
        self.assertEqual(pool['medians'], [1020, 1020])
        self.assertEqual(pool['p95s'], [1038, 3 * 1038])


if __name__ == '__main__':
    unittest.main()
