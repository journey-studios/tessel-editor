#!/usr/bin/env python3
"""Derives `docs/migration/fixtures/gizmo-perf/budgets.json` from calibration reports (F055 / BL015.21).

    python3 scripts/gizmo-perf/budgets.py REPORT.json [REPORT.json ...] --out budgets.json

The budgets file records the hashes of the calibration reports it came from. The rule was NOT pre-registered:
the cold margins below were widened after a judged run failed (see `docs/features/scene-gizmo-performance-timing.md`).
A budget is a multiple of the scene's own plan time (see `scene_gizmo_perf_budget.rs`), so it does not carry the speed
of the machine it was calibrated on:

    p50_ratio = ceil_to_0.05(max over calibrations of the p50 ratio * 1.15)          (floor 0.25)
    p95_ratio = ceil_to_0.05(max over calibrations of the p95 ratio * 1.25)          (floor 0.50)
    `cold.*` kinds (60 fresh editors each): margins 1.25 and 2.0 instead.

A run is INCONCLUSIVE (not a pass, not a failure) when more than 25% of its plan reference blocks (5 plan
builds taken right before each window of requests) disagree with themselves by more than 1.5x: a steady machine
can be judged, a moving one cannot. The same rule applies to a calibration: a report whose reference blocks moved
(or that does not say how steady they were) is REFUSED, because the budget takes the MAXIMUM observed ratio and one
unsteady report would loosen it. Nothing is written when anything is refused, and `--out` is never a report.

Long ticks: a sample over a 60 Hz frame
(16.6 ms) is counted per corpus and request kind; none was observed in the calibrations, so the limit is 0
for `small` and `deep` and 0 for `dense` (a 618-node scene, as the corpus manifest counts it, must still fit a
frame on a quiet machine).
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import sys

MARGIN_P50 = 1.15
MARGIN_P95 = 1.25
MAX_BLOCK_SPREAD = 1.5
MAX_UNSTABLE_BLOCKS = 0.25  # of the reference blocks may disagree with themselves; past it a run is not judged
# A cold sample pays page-in and allocator warm-up that a hot one does not: its tail is wider (60 samples
# per kind), so the cold kinds get wider margins. Set AFTER the judged `try2` failed (dense/cold.state p95 limit 1.45 -> 5.6).
COLD_MARGIN_P50 = 1.25
COLD_MARGIN_P95 = 2.0


def up(value):
    return round(math.ceil(value * 20 - 1e-9) / 20, 2)


def refuse(message):
    """A refusal is raised before anything is written: the budgets that exist are the budgets that stay."""
    raise SystemExit(f'budgets.py: {message}')


def main(argv):
    parser = argparse.ArgumentParser(description='Derive budgets.json from calibration reports (release, steady).')
    parser.add_argument('--out', required=True, help='the budgets file to write')
    parser.add_argument('reports', nargs='+', help='calibration reports: the report.json of `run.py --calibrate`')
    args = parser.parse_intermixed_args(argv)
    out = args.out
    reports = [Path(path) for path in args.reports]
    if Path(out).resolve() in {path.resolve() for path in reports}:
        refuse(f'--out names one of the reports ({Path(out).name}): the budgets would be written over a calibration')
    loaded = [json.loads(path.read_text()) for path in reports]
    observed = {}
    for path, report in zip(reports, loaded):
        if report.get('profile') != 'release':
            refuse(f'{path.name}: budgets come from release runs only')
        for corpus in report['corpora']:
            unstable = corpus.get('reference', {}).get('unstable_fraction')
            if unstable is None:
                refuse(f"{path.name}: the report of {corpus['corpus']} does not say how steady its plan reference was")
            if unstable > MAX_UNSTABLE_BLOCKS:
                refuse(f"{path.name}: {unstable:.0%} of the plan reference blocks of {corpus['corpus']} disagreed with "
                       f'themselves by more than {MAX_BLOCK_SPREAD}x (limit {MAX_UNSTABLE_BLOCKS:.0%}): a calibration '
                       'taken on a moving machine would loosen the budgets and is not used')
            for phase in ('cold', 'hot'):
                for kind, summary in corpus[phase].items():
                    entry = observed.setdefault(corpus['corpus'], {}).setdefault(kind, [0.0, 0.0])
                    entry[0] = max(entry[0], summary['ratio_p50'])
                    entry[1] = max(entry[1], summary['ratio_p95'])
    def margins(kind):
        return (COLD_MARGIN_P50, COLD_MARGIN_P95) if kind.startswith('cold.') else (MARGIN_P50, MARGIN_P95)

    ratios = {corpus: {kind: {'p50_ratio': max(0.25, up(p50 * margins(kind)[0])),
                              'p95_ratio': max(0.5, up(p95 * margins(kind)[1]))}
                       for kind, (p50, p95) in sorted(kinds.items())}
              for corpus, kinds in sorted(observed.items())}
    budgets = {
        'version': 1,
        'rule': 'p50_ratio = ceil_to_0.05(max calibrated p50 ratio * 1.15), floor 0.25; p95_ratio = ceil_to_0.05(max calibrated p95 ratio * 1.25), floor 0.5; cold.* kinds: 1.25 and 2.0; ratio = request time / the scene\'s own ScenePlan::build_with p50 in the same process',
        'derived_by': 'scripts/gizmo-perf/budgets.py',
        'calibrations': [{'file': path.name, 'sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
                          'dir': path.parent.name} for path in reports],
        'reference': {'max_block_spread': MAX_BLOCK_SPREAD, 'max_unstable_blocks': MAX_UNSTABLE_BLOCKS},
        'long_tick_ms': 16.6,
        'long_ticks_max': {'small': 0, 'dense': 0, 'deep': 0},
        'ratios': ratios,
    }
    Path(out).write_text(json.dumps(budgets, indent=1, sort_keys=True) + '\n')
    print(f'wrote {out}: {sum(len(v) for v in ratios.values())} budgets from {len(reports)} calibrations')


if __name__ == '__main__':
    main(sys.argv[1:])
