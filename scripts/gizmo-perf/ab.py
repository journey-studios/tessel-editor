#!/usr/bin/env python3
"""Baseline-versus-candidate benchmark of the gizmo handlers (F055 / BL015.21).

    python3 scripts/gizmo-perf/ab.py [--baseline REF] [--rounds N] [--out DIR] [--allow-dirty]

What is compared. The SAME executable source (`scene_gizmo_perf_budget`, its corpus, its journey, its release
profile, the same machine, the same in-process backend) built twice: from `--baseline` (default `cbe9e1b`, the
merge of PR #43) and from this checkout's HEAD. Only the product code differs between the two builds; the
measuring instrument (the plan counter hooks, the tests, the fixtures) is overlaid onto the baseline so that
both sides run identical measurement code. The overlay is `git diff BASELINE HEAD` restricted to those files and is
recorded, with the product diff outside them (which must be empty for the claim "same product" and is otherwise
listed so the reader sees what was compared).

How. One short `git archive` of the baseline into a temp directory, built SEQUENTIALLY through the repository's
cargo wrapper with its DEFAULT target (no `CARGO_TARGET_DIR`, no lane); both test executables are copied out, the
temp directory is deleted. The performance lock is taken ONCE, without waiting; a busy lock or a failed machine guard
is a recorded BLOCKED (exit 3). Rounds alternate the order (A B | B A | A B | B A ...), each side runs a declared
warmup run first that is NOT kept (and said so in the report), and every kept run's raw samples are pooled per
(corpus, request kind): nothing is trimmed, no tail dropped.

Rules, fixed BEFORE any candidate number exists (never edited after looking):
  * metrics per kind: median, p95, p99, max, lost samples (n below expected), errors, long ticks (>16.6 ms) and the
    plan-reference ratio (the request over the scene's own `ScenePlan::build_with`, which cancels machine speed);
  * a kind REGRESSES when the pooled candidate median is more than 10% over the baseline's AND the candidate is
    slower in a majority of the round pairs for that median; or when its pooled p95 is more than 25% over the
    baseline's under the same majority rule; or when it lost samples or long ticks the baseline did not;
  * noise: when the baseline's own median varies more than 15% between its rounds for a kind, that kind is
    INCONCLUSIVE and says so (never a pass);
  * "no regression" is only ever claimed for this scenario/config/hardware/backend, not for the native window.
Exit 0 = no kind regressed and none inconclusive, 1 = a kind regressed, 3 = BLOCKED, 4 = INCONCLUSIVE.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parent))
import harness  # noqa: E402

ROOT = harness.ROOT
BASELINE = 'cbe9e1b393861febf073633fbbaa0da9857780b6'
TEST = 'scene_gizmo_perf_budget'
INSTRUMENT = ['crates/core/src/scene_plan', 'crates/core/tests', 'docs/migration/fixtures/gizmo-perf']
MEDIAN_TOLERANCE = 1.10
P95_TOLERANCE = 1.25
BASELINE_SPREAD_TOLERANCE = 0.15
LONG_TICK_NS = 16_600_000


def find_exe(done):
    exe = None
    for line in done.stdout.splitlines():
        try:
            message = json.loads(line)
        except ValueError:
            continue
        if message.get('executable') and message.get('target', {}).get('name') == TEST:
            exe = message['executable']
    return exe


def build(cwd):
    command = ['./scripts/cargo', 'test', '--release', '--locked', '-p', 'tessel-editor-core', '--test', TEST,
               '--no-run', '--message-format=json']
    done = harness.run(command, cwd=cwd)
    return command, done, find_exe(done)


def make_baseline(ref, workdir):
    """A small `git archive` of `ref` plus the instrument overlay. Returns (path, product_diff_files)."""
    src = Path(workdir) / 'baseline-src'
    src.mkdir()
    archive = subprocess.run(['git', 'archive', ref], cwd=ROOT, stdout=subprocess.PIPE, check=True)
    subprocess.run(['tar', '-x', '-C', str(src)], input=archive.stdout, check=True)
    patch = subprocess.run(['git', 'diff', '--binary', ref, 'HEAD', '--'] + INSTRUMENT, cwd=ROOT,
                           stdout=subprocess.PIPE, check=True).stdout
    # Untracked instrument files (a dirty tree) are not in HEAD: the caller refuses a dirty tree unless allowed.
    if patch:
        subprocess.run(['git', 'apply', '--whitespace=nowarn', '-'], cwd=src, input=patch, check=True)
    product = subprocess.run(['git', 'diff', '--name-only', ref, 'HEAD', '--', 'crates', 'Cargo.toml', 'Cargo.lock'],
                             cwd=ROOT, stdout=subprocess.PIPE, text=True, check=True).stdout.split()
    product = [path for path in product if not any(path == p or path.startswith(p + '/') for p in INSTRUMENT)]
    return src, product


def run_once(exe, out_path):
    env = {**os.environ, 'TESSEL_GIZMO_PERF_OUT': str(out_path), 'TESSEL_GIZMO_PERF_CALIBRATE': '1'}
    done = subprocess.run([exe, '--ignored', '--test-threads=1', '--nocapture',
                           'the_gizmo_handlers_stay_within_their_budget_cold_and_hot_on_every_corpus'],
                          cwd=ROOT, env=env, capture_output=True, text=True)
    return done


def kinds(report):
    """{(corpus, kind): stats dict} from one report."""
    out = {}
    for corpus in report['corpora']:
        for phase in ('cold', 'hot'):
            for kind, stats in corpus[phase].items():
                out[(corpus['corpus'], kind)] = stats
    return out


def pct(values, q):
    ordered = sorted(values)
    if not ordered:
        return None
    return ordered[min(len(ordered) - 1, int(q * len(ordered)))]


def pooled(reports):
    """Per (corpus, kind): every raw sample of every kept run, with the per-run medians."""
    pool = {}
    for report in reports:
        for key, stats in kinds(report).items():
            slot = pool.setdefault(key, {'samples': [], 'medians': [], 'ratios': [], 'long_ticks': 0, 'n': []})
            slot['samples'] += stats['samples_ns']
            slot['medians'].append(stats['p50_ns'])
            slot['ratios'].append(stats['ratio_p50'])
            slot['long_ticks'] += stats.get('long_ticks', 0)
            slot['n'].append(stats['n'])
    return pool


def summarize(slot):
    s = slot['samples']
    return {'n': len(s), 'p50_ns': pct(s, 0.5), 'p95_ns': pct(s, 0.95), 'p99_ns': pct(s, 0.99), 'max_ns': max(s),
            'long_ticks': sum(1 for v in s if v > LONG_TICK_NS), 'run_medians_ns': slot['medians'],
            'run_ratio_p50': slot['ratios'], 'expected_n_per_run': slot['n']}


def judge(base, cand):
    """The pre-declared rule applied to pooled baseline/candidate samples and per-round medians."""
    verdicts = {}
    for key in sorted(base):
        b, c = summarize(base[key]), summarize(cand[key])
        medians = base[key]['medians']
        spread = (max(medians) - min(medians)) / statistics.median(medians)
        pairs = list(zip(base[key]['medians'], cand[key]['medians']))
        slower = sum(1 for x, y in pairs if y > x)
        reasons = []
        p50_over = c['p50_ns'] > b['p50_ns'] * MEDIAN_TOLERANCE
        p95_over = c['p95_ns'] > b['p95_ns'] * P95_TOLERANCE
        if p50_over and slower * 2 > len(pairs):
            reasons.append(f"median {c['p50_ns']} > {b['p50_ns']} x {MEDIAN_TOLERANCE} (slower in {slower}/{len(pairs)} pairs)")
        if p95_over and slower * 2 > len(pairs):
            reasons.append(f"p95 {c['p95_ns']} > {b['p95_ns']} x {P95_TOLERANCE} (slower in {slower}/{len(pairs)} pairs)")
        if c['n'] < b['n']:
            reasons.append(f"lost samples: {c['n']} < {b['n']}")
        if c['long_ticks'] > b['long_ticks']:
            reasons.append(f"long ticks {c['long_ticks']} > {b['long_ticks']}")
        if reasons:
            status = 'REGRESSED'
        elif spread > BASELINE_SPREAD_TOLERANCE:
            status = 'INCONCLUSIVE'
            reasons.append(f'baseline median varied {spread:.0%} between its own rounds (> {BASELINE_SPREAD_TOLERANCE:.0%})')
        else:
            status = 'NO_REGRESSION_IN_SCENARIO'
        verdicts[f'{key[0]}/{key[1]}'] = {'status': status, 'reasons': reasons, 'baseline': b, 'candidate': c,
                                          'baseline_run_spread': round(spread, 4),
                                          'candidate_slower_pairs': f'{slower}/{len(pairs)}',
                                          'median_ratio_candidate_over_baseline': round(c['p50_ns'] / b['p50_ns'], 4),
                                          'p95_ratio_candidate_over_baseline': round(c['p95_ns'] / b['p95_ns'], 4)}
    return verdicts


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument('--baseline', default=BASELINE)
    parser.add_argument('--rounds', type=int, default=4)
    parser.add_argument('--out', default='/tmp/gizmo-perf-ab')
    parser.add_argument('--allow-dirty', action='store_true',
                        help='measure a dirty tree (recorded; never counts for HEAD)')
    args = parser.parse_args(argv)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    manifest = {'suite': 'gizmo-perf-ab', 'utc_start': harness.utc(), 'cwd': str(ROOT), 'baseline': args.baseline,
                'candidate': harness.git_identity(), 'toolchain': harness.toolchain(), 'rounds': args.rounds,
                'rules': {'median_tolerance': MEDIAN_TOLERANCE, 'p95_tolerance': P95_TOLERANCE,
                          'baseline_spread_tolerance': BASELINE_SPREAD_TOLERANCE, 'long_tick_ns': LONG_TICK_NS},
                'status': 'ABORTED'}
    manifest_path = out / 'manifest.json'
    if manifest['candidate']['dirty'] and not args.allow_dirty:
        manifest.update(status='REFUSED', reason='the working tree is dirty: numbers would not belong to HEAD')
        return harness.die(1, manifest, manifest_path)
    manifest['dirty_run'] = bool(manifest['candidate']['dirty'])
    manifest['baseline_commit'] = harness.out(['git', 'rev-parse', args.baseline])
    work = tempfile.mkdtemp(prefix='gizmo-perf-ab-')
    try:
        # 1. builds (no lock: a build is not a measurement), sequential, default target.
        src, product_diff = make_baseline(args.baseline, work)
        manifest['product_files_that_differ'] = product_diff
        c_command, c_done, c_exe = build(ROOT)
        manifest['candidate_build'] = {'command': c_command, 'exit': c_done.returncode, 'executable': c_exe}
        b_command, b_done, b_exe = build(src)
        manifest['baseline_build'] = {'command': b_command, 'exit': b_done.returncode, 'executable': b_exe,
                                      'cwd': str(src), 'stderr_tail': b_done.stderr[-1500:]}
        if not (c_exe and b_exe and c_done.returncode == 0 and b_done.returncode == 0):
            manifest.update(status='BUILD_FAILED', reason='a release build failed')
            return harness.die(1, manifest, manifest_path)
        exes = {'baseline': Path(work) / 'baseline.exe', 'candidate': Path(work) / 'candidate.exe'}
        shutil.copy(b_exe, exes['baseline'])
        shutil.copy(c_exe, exes['candidate'])
        manifest['executables_sha256'] = {k: harness.sha256(v) for k, v in exes.items()}
        shutil.rmtree(src)  # the archive (and any target inside it) is discardable
        # 2. guards + the lock, once.
        state, reason = harness.guards()
        manifest['machine_before'] = state
        if reason:
            manifest.update(status='BLOCKED', reason=reason)
            return harness.die(harness.BLOCKED_EXIT, manifest, manifest_path)
        with harness.Lock() as lock:
            if not lock.held:
                manifest.update(status='BLOCKED', reason='performance lock busy (not waited for)')
                return harness.die(harness.BLOCKED_EXIT, manifest, manifest_path)
            order = []
            for r in range(args.rounds):
                order += ['baseline', 'candidate'] if r % 2 == 0 else ['candidate', 'baseline']
            runs = []
            # declared warmup: one discarded run per side, alternating, before the kept rounds.
            for side in ('baseline', 'candidate'):
                done = run_once(exes[side], out / f'warmup-{side}.json')
                runs.append({'side': side, 'kept': False, 'warmup': True, 'exit': done.returncode,
                             'load_after': os.getloadavg()})
            reports = {'baseline': [], 'candidate': []}
            for index, side in enumerate(order):
                path = out / f'round{index:02d}-{side}.json'
                done = run_once(exes[side], path)
                (out / f'round{index:02d}-{side}.stdout.txt').write_text(done.stdout)
                runs.append({'side': side, 'kept': True, 'index': index, 'exit': done.returncode,
                             'report': path.name, 'report_sha256': harness.sha256(path) if path.exists() else None,
                             'load_after': os.getloadavg(), 'thermal': harness.machine()['thermal'][:300]})
                if done.returncode != 0 or not path.exists():
                    manifest['runs'] = runs
                    manifest.update(status='FAIL', reason=f'round {index} ({side}) failed: {done.stdout[-400:]}')
                    return harness.die(1, manifest, manifest_path)
                if 'INCONCLUSIVE' in done.stdout:
                    runs[-1]['reference_inconclusive'] = True
                reports[side].append(json.loads(path.read_text()))
            manifest['machine_after'] = harness.machine()
        manifest['runs'] = runs
        verdicts = judge(pooled(reports['baseline']), pooled(reports['candidate']))
        harness.write(out / 'verdicts.json', verdicts)
        manifest['verdicts_sha256'] = harness.sha256(out / 'verdicts.json')
        statuses = {v['status'] for v in verdicts.values()}
        manifest['verdict_counts'] = {s: sum(1 for v in verdicts.values() if v['status'] == s) for s in sorted(statuses)}
        if 'REGRESSED' in statuses:
            manifest.update(status='REGRESSED', reason=[k for k, v in verdicts.items() if v['status'] == 'REGRESSED'])
            code = 1
        elif 'INCONCLUSIVE' in statuses:
            manifest.update(status='INCONCLUSIVE', reason=[k for k, v in verdicts.items() if v['status'] == 'INCONCLUSIVE'])
            code = harness.INCONCLUSIVE_EXIT
        else:
            manifest.update(status='NO_REGRESSION_IN_SCENARIO', reason='')
            code = 0
        manifest['utc_end'] = harness.utc()
        manifest['exit'] = code
        harness.write(manifest_path, manifest)
        print(json.dumps({k: manifest[k] for k in ('status', 'reason', 'verdict_counts', 'exit')}))
        return code
    finally:
        shutil.rmtree(work, ignore_errors=True)


if __name__ == '__main__':
    raise SystemExit(main())
