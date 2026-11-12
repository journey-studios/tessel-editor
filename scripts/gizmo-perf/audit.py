#!/usr/bin/env python3
"""Sensitivity audit of the time budgets (F055 / BL015.21). No benchmark runs here: it reads `budgets.json` and the
reports a controlled run wrote (`run.py`'s `report.json`) and says, per corpus and request kind, how much extra work a
budget would let through.

    python3 scripts/gizmo-perf/audit.py --budgets docs/migration/fixtures/gizmo-perf/budgets.json REPORT.json [...] [--out FILE]

Why it exists. A budget is a multiple of the scene's own plan time, so (a) it carries the speed of no machine and (b) its
slack is measurable in "plans": a kind whose p95 budget is 22 plans while the run observed 7 lets 15 extra plans through.
Per kind it reports, from the WORST observation over the reports given (slack, headroom) and per report (the two
detections, which need the ratios AND the plan time of the SAME report):
  * `slack_plans_p50/p95`: budget minus observed, in plans;
  * `extra_plan_seen_by_time`: whether ONE extra plan per call would exceed the p50 or p95 budget in some report;
  * `delay_1ms_seen_by_time`: whether a delay of 1 ms per call would in some report, converting it with the plan time of that
    report (`reference.p50_ns`, one `delay_1ms_in_plans` per report);
  * `extra_plan_seen_in_reports`, `delay_1ms_seen_in_reports`, `reports`: in how many of the reports the detection holds,
    and the summary lists the kinds seen in only some of them: a ratio of one report is never added to the plan time of another;
  * `p95_headroom_x`: budget over observed.
The structural detector (`scene_gizmo_perf_structure.rs`: layouts per request, exact) and the time detector are
ORTHOGONAL: the first counts work and cannot see a delay, the second sees time and cannot always see one plan. This
audit lists exactly where the time detector alone is blind to one extra plan, so that blindness is a recorded
property of the suite (the structural test covers it) and not a surprise. The conversion of a delay into plans uses a
measured plan time, not a clock mock: it says what the budgets would have to see, and `mutate.py` is the real proof.

The thresholds are declared here, before any run reads them: `HEADROOM_WIDE = 2.0` and `HEADROOM_VERY_WIDE = 3.0`.
"""
import argparse
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import harness  # noqa: E402

HEADROOM_WIDE = 2.0
HEADROOM_VERY_WIDE = 3.0


def observed(reports):
    """{(corpus, kind): [{'p50', 'p95', 'plan_ns'}, ...]}: ONE observation per report, in the order the reports were given.
    The ratios and the plan time of an observation come from the same report: the worst ratio of one report and the
    plan time of another is a run that never happened."""
    out = {}
    for report in reports:
        for corpus in report['corpora']:
            plan_ns = corpus['reference']['p50_ns']
            for phase in ('cold', 'hot'):
                for kind, stats in corpus[phase].items():
                    out.setdefault((corpus['corpus'], kind), []).append(
                        {'p50': stats['ratio_p50'], 'p95': stats['ratio_p95'], 'plan_ns': plan_ns})
    return out


def audit(budgets, reports):
    seen = observed(reports)
    kinds = {}
    for (corpus, kind), observations in sorted(seen.items()):
        limit = budgets['ratios'][corpus].get(kind)
        if limit is None:
            kinds[f'{corpus}/{kind}'] = {'status': 'NO_BUDGET'}
            continue
        b50, b95 = limit['p50_ratio'], limit['p95_ratio']
        worst_p50 = max(o['p50'] for o in observations)
        worst_p95 = max(o['p95'] for o in observations)

        def exceeds(o, extra_plans):
            return o['p50'] + extra_plans > b50 or o['p95'] + extra_plans > b95

        one_ms_in_plans = [1e6 / o['plan_ns'] for o in observations]
        extra_plan_seen = [exceeds(o, 1) for o in observations]
        delay_seen = [exceeds(o, plans) for o, plans in zip(observations, one_ms_in_plans)]
        kinds[f'{corpus}/{kind}'] = {
            'budget_p50': b50, 'budget_p95': b95, 'observed_p50': round(worst_p50, 4),
            'observed_p95': round(worst_p95, 4), 'plan_ns': max(o['plan_ns'] for o in observations),
            'slack_plans_p50': round(b50 - worst_p50, 3), 'slack_plans_p95': round(b95 - worst_p95, 3),
            'reports': len(observations),
            'extra_plan_seen_by_time': any(extra_plan_seen), 'extra_plan_seen_in_reports': sum(extra_plan_seen),
            'delay_1ms_in_plans': [round(plans, 3) for plans in one_ms_in_plans],
            'delay_1ms_seen_by_time': any(delay_seen), 'delay_1ms_seen_in_reports': sum(delay_seen),
            'p95_headroom_x': round(b95 / worst_p95, 3) if worst_p95 else None,
        }
    judged = {k: v for k, v in kinds.items() if v.get('status') != 'NO_BUDGET'}

    def partly(seen_key):
        """The kinds some reports see and others do not: neither answer may stand for all of them."""
        return sorted(k for k, v in judged.items() if 0 < v[seen_key] < v['reports'])

    summary = {
        'kinds': len(judged),
        'one_extra_plan_passes_in_time': sorted(k for k, v in judged.items() if not v['extra_plan_seen_by_time']),
        'one_extra_plan_seen_in_some_reports_only': partly('extra_plan_seen_in_reports'),
        'delay_1ms_passes_in_time': sorted(k for k, v in judged.items() if not v['delay_1ms_seen_by_time']),
        'delay_1ms_seen_in_some_reports_only': partly('delay_1ms_seen_in_reports'),
        'p95_headroom_at_least_2x': sorted(k for k, v in judged.items()
                                           if v['p95_headroom_x'] and v['p95_headroom_x'] >= HEADROOM_WIDE),
        'p95_headroom_at_least_3x': sorted(k for k, v in judged.items()
                                           if v['p95_headroom_x'] and v['p95_headroom_x'] >= HEADROOM_VERY_WIDE),
        'no_budget': sorted(k for k, v in kinds.items() if v.get('status') == 'NO_BUDGET'),
    }
    for key in ('one_extra_plan_passes_in_time', 'one_extra_plan_seen_in_some_reports_only', 'delay_1ms_passes_in_time',
                'delay_1ms_seen_in_some_reports_only', 'p95_headroom_at_least_2x', 'p95_headroom_at_least_3x'):
        summary[key + '_count'] = len(summary[key])
    return {'summary': summary, 'kinds': kinds}


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument('--budgets', required=True)
    parser.add_argument('--out')
    parser.add_argument('reports', nargs='+')
    args = parser.parse_args(argv)
    budgets = json.loads(Path(args.budgets).read_text())
    result = audit(budgets, [json.loads(Path(r).read_text()) for r in args.reports])
    result['inputs'] = {'budgets': args.budgets, 'reports': args.reports}
    text = json.dumps(result, indent=1, sort_keys=True) + '\n'
    if args.out:
        harness.write_text(args.out, text)  # `inputs` holds the paths as typed: redacted like every runner file
    print(json.dumps(result['summary'], indent=1, sort_keys=True))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
