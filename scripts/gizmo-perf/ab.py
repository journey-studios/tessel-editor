#!/usr/bin/env python3
"""Baseline-versus-candidate benchmark of the gizmo handlers (F055 / BL015.21).

    python3 scripts/gizmo-perf/ab.py --phase build   [--rounds N] [--out DIR]
    python3 scripts/gizmo-perf/ab.py --phase measure [--rounds N] [--out DIR]

What is compared. The SAME measuring executable source (`scene_gizmo_perf_budget`, its corpus, its journey, its release
profile, the same machine, the same in-process backend) built twice: from the PINNED baseline (`cbe9e1b`, the merge of
PR #43, its production code UNCHANGED) and from this checkout's HEAD.

What the baseline gets besides the pinned production code is an ALLOWLIST, and nothing else (`ALLOWED_OVERLAY`):
  * test-only files: the budget test, the shared journey support and the corpus fixtures (copied from HEAD);
  * the counter module `scene_plan/probe.rs` as a NEW file plus the one `pub mod probe;` line in `scene_plan/mod.rs`
    (the test support names the module). The baseline does NOT get the `note_*` calls: the candidate's counting
    instruments the hot path, the baseline's does not, so the probe overhead is part of what the A/B measures.
Everything else under the production paths (`crates/*/src`, build scripts, manifests, the lockfile, the toolchain pin)
is the pinned ref's, byte for byte. The runner records the production tree digest of the baseline BEFORE and AFTER the
overlay and of the candidate (git blob ids), refuses (`REFUSED_CONTAMINATION`) when the overlay changed anything outside
the allowlist or when the archive is not the ref's tree, and lists every production file that differs between the two
sides. A candidate that adds work in `scene_plan/build.rs` therefore stays DIFFERENT from the baseline; when no
production file differs at all the run is an A/A and says so (exit 4), never a "no regression". "Differs" is judged over
what the MEASURED executable is built from (`is_measured`: the core crate and the workspace build inputs); a candidate
that differs only in other crates (the desktop's) is `DIFFERENCE_OUTSIDE_THE_MEASUREMENT` (exit 4, files listed apart):
this A/B cannot see those files, so it never turns them into a "no regression".

How. One short `git archive` of the baseline into a temp directory, built SEQUENTIALLY through the repository's cargo
wrapper with its DEFAULT target (no `CARGO_TARGET_DIR`, no lane); both test executables are copied out, the temp
directory is deleted. The performance lock is taken ONCE, without waiting; a busy lock or a failed machine guard is a
recorded BLOCKED (exit 3). Every child has a finite timeout. Rounds alternate the order (A B | B A | A B | B A ...),
each side runs a declared warmup run first that is NOT kept (and said so in the report), and every kept run's raw
samples are pooled per (corpus, request kind): nothing is trimmed, no tail dropped, no best-of.

Rules, fixed BEFORE any candidate number is read in a run (never edited after looking; a change of a rule is a
new commit, recorded, and applies to the next run):
  * metrics per kind: median, p95, p99, max, lost samples (n below expected), long ticks (>16.6 ms) and the
    plan-reference ratio (the request over the scene's own `ScenePlan::build_with`, which cancels machine speed). p99
    and max are RECORDED, not judged: one scheduler hiccup in a shared machine would decide a verdict;
  * a kind REGRESSES when the pooled candidate median is more than 10% over the baseline's AND the candidate is
    slower in a majority of the round pairs for that median; or when its pooled p95 is more than 25% over the
    baseline's under the same majority rule; or when it lost samples or long ticks the baseline did not;
  * noise: when the baseline's own median varies more than 15% between its rounds for a kind, or when more than
    `budgets.json: reference.max_unstable_blocks` of the plan-reference blocks of its corpus disagreed with themselves
    (read from every kept report), that kind is INCONCLUSIVE and says so (never a pass);
  * after the last run the guards are read again (load 1/5 min, thermal, disk); a failed guard makes the whole run
    INVALID (exit 3): the numbers are kept but nothing is judged;
  * "no regression" is only ever claimed for this scenario/config/hardware/backend, not for the native window.
Exit 0 = no kind regressed and none inconclusive, 1 = a kind regressed or a refusal, 3 = BLOCKED/INVALID,
4 = INCONCLUSIVE (or A/A).
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import harness  # noqa: E402

ROOT = harness.ROOT
BASELINE = 'cbe9e1b393861febf073633fbbaa0da9857780b6'
TEST = 'scene_gizmo_perf_budget'
ONE_TEST = 'the_gizmo_handlers_stay_within_their_budget_cold_and_hot_on_every_corpus'
PROBE_FILE = 'crates/core/src/scene_plan/probe.rs'
PROBE_DECL_FILE = 'crates/core/src/scene_plan/mod.rs'
PROBE_DECL_LINE = 'pub mod probe;'
# Test-only paths copied from the candidate. Nothing under a production path is in this list.
INSTRUMENT_TESTS = ['crates/core/tests/scene_gizmo_perf_budget.rs', 'crates/core/tests/gizmo_perf_support',
                    'docs/migration/fixtures/gizmo-perf']
# The ONLY production paths the baseline may differ in from the pinned ref after the overlay.
ALLOWED_OVERLAY = {PROBE_FILE: 'new file: the counter module (no call sites)',
                   PROBE_DECL_FILE: f'one added line: `{PROBE_DECL_LINE}`'}
MEDIAN_TOLERANCE = 1.10
P95_TOLERANCE = 1.25
BASELINE_SPREAD_TOLERANCE = 0.15
LONG_TICK_NS = 16_600_000
BUDGETS_FILE = ROOT / 'docs/migration/fixtures/gizmo-perf/budgets.json'


def max_unstable_fraction(path=BUDGETS_FILE):
    """The share of plan-reference blocks that may disagree with themselves before a corpus is INCONCLUSIVE, read from
    `budgets.json: reference.max_unstable_blocks`: the one place it is written, and the field the core's budget test
    reads too, so the A/B and the in-process test cannot drift apart. Anything but a fraction is refused."""
    value = json.loads(Path(path).read_text())['reference']['max_unstable_blocks']
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not 0 <= value <= 1:
        raise ValueError(f'{path}: reference.max_unstable_blocks must be a fraction in [0, 1], got {value!r}')
    return value


MAX_UNSTABLE_FRACTION = max_unstable_fraction()


class Refused(Exception):
    """The comparison would not be what it claims to be."""


# --- production trees ------------------------------------------------------------------------------------------


def is_prod(path):
    """A path whose content is the product (or decides how it is built). Tests and docs are not."""
    parts = path.split('/')
    if path in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml') or parts[0] in ('.cargo', 'vendor'):
        return True
    return len(parts) >= 3 and parts[0] == 'crates' and (parts[2] == 'src' or parts[2] in ('build.rs', 'Cargo.toml'))


def is_measured(path):
    """A product file the MEASURED executable is built from: the A/B runs only `cargo test -p tessel-editor-core --test
    scene_gizmo_perf_budget`, so that is the core crate (it depends on no other workspace crate) and what decides how
    anything is built (workspace manifest, lockfile, toolchain pin, cargo config, vendored crates). A difference in
    `crates/desktop/src` is the product's, but the number this A/B produces cannot see it."""
    parts = path.split('/')
    if path in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml') or parts[0] in ('.cargo', 'vendor'):
        return True
    return len(parts) >= 3 and parts[:2] == ['crates', 'core'] and (parts[2] == 'src' or parts[2] in ('build.rs', 'Cargo.toml'))


def blob_id(data):
    return hashlib.sha1(b'blob %d\0' % len(data) + data).hexdigest()


def git_prod_manifest(ref, root=ROOT):
    """{path: git blob id} of the production files of `ref`."""
    listing = subprocess.run(['git', 'ls-tree', '-r', ref], cwd=root, stdout=subprocess.PIPE, text=True,
                             check=True).stdout
    out = {}
    for line in listing.splitlines():
        meta, path = line.split('\t', 1)
        if is_prod(path):
            out[path] = meta.split()[2]
    return out


def dir_prod_manifest(directory):
    base = Path(directory)
    out = {}
    for path in sorted(base.rglob('*')):
        if path.is_file() and not path.is_symlink():
            relative = path.relative_to(base).as_posix()
            if is_prod(relative):
                out[relative] = blob_id(path.read_bytes())
    return out


def digest(manifest):
    return hashlib.sha256(json.dumps(sorted(manifest.items())).encode()).hexdigest()


def changed_paths(before, after):
    return sorted(p for p in set(before) | set(after) if before.get(p) != after.get(p))


def make_baseline(ref, workdir, root=ROOT, candidate='HEAD'):
    """The pinned `ref` plus the allowlisted overlay. Returns a record with the source path and the evidence.

    Raises `Refused` when the archive is not `ref`'s production tree, when the overlay would touch anything outside
    `ALLOWED_OVERLAY`, or when the candidate's change to the declaration file is not exactly the probe line.
    """
    src = Path(workdir) / 'baseline-src'
    src.mkdir()
    archive = subprocess.run(['git', 'archive', ref], cwd=root, stdout=subprocess.PIPE, check=True)
    subprocess.run(['tar', '-x', '-C', str(src)], input=archive.stdout, check=True)
    pinned = git_prod_manifest(ref, root)
    before = dir_prod_manifest(src)
    if before != pinned:
        raise Refused(f'the archive of {ref} is not its production tree: {changed_paths(before, pinned)[:5]}')
    # 1. test-only instrument, from the candidate.
    for path in INSTRUMENT_TESTS:
        piece = subprocess.run(['git', 'archive', candidate, '--', path], cwd=root, stdout=subprocess.PIPE,
                               check=True).stdout
        subprocess.run(['tar', '-x', '-C', str(src)], input=piece, check=True)
    # 2. the counter module and its declaration, only when the pinned ref does not have them.
    if PROBE_FILE not in pinned:
        probe = subprocess.run(['git', 'show', f'{candidate}:{PROBE_FILE}'], cwd=root, stdout=subprocess.PIPE,
                               check=True).stdout
        (src / PROBE_FILE).write_bytes(probe)
        numstat = subprocess.run(['git', 'diff', '--numstat', ref, candidate, '--', PROBE_DECL_FILE], cwd=root,
                                 stdout=subprocess.PIPE, text=True, check=True).stdout.split()
        patch = subprocess.run(['git', 'diff', '--binary', ref, candidate, '--', PROBE_DECL_FILE], cwd=root,
                               stdout=subprocess.PIPE, check=True).stdout
        added = [l[1:] for l in patch.decode().splitlines() if l.startswith('+') and not l.startswith('+++')]
        removed = [l for l in patch.decode().splitlines() if l.startswith('-') and not l.startswith('---')]
        if numstat[:2] != ['1', '0'] or added != [PROBE_DECL_LINE] or removed:
            raise Refused(f'`{PROBE_DECL_FILE}` differs from {ref} by more than `{PROBE_DECL_LINE}`: a production '
                          'change cannot be overlaid onto the baseline')
        subprocess.run(['git', 'apply', '--whitespace=nowarn', '-'], cwd=src, input=patch, check=True)
    after = dir_prod_manifest(src)
    contaminated = [p for p in changed_paths(before, after) if p not in ALLOWED_OVERLAY]
    if contaminated:
        raise Refused(f'the overlay changed production files outside the allowlist: {contaminated}')
    candidate_prod = git_prod_manifest(candidate, root)
    # `differs` is what the measured executable can see; the rest is listed, never judged (see `is_measured`).
    paths = changed_paths(after, candidate_prod)
    differs = {p: {'baseline': after.get(p), 'candidate': candidate_prod.get(p)} for p in paths if is_measured(p)}
    return {'src': src, 'prod_tree_before_overlay': digest(before), 'prod_tree_after_overlay': digest(after),
            'prod_tree_candidate': digest(candidate_prod), 'pinned_ref_tree': digest(pinned),
            'overlay_changed': changed_paths(before, after), 'allowlist': ALLOWED_OVERLAY,
            'contaminated': contaminated, 'product_files_that_differ': differs,
            'product_files_outside_the_measurement': [p for p in paths if not is_measured(p)]}


# --- build / run -----------------------------------------------------------------------------------------------


def find_exe(stdout):
    exe = None
    for line in stdout.splitlines():
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
    done = harness.run_bounded(command, cwd=cwd, timeout=harness.BUILD_TIMEOUT_S)
    return command, done, find_exe(done.stdout)


def run_once(exe, out_path):
    """One controlled run. The report is removed first, so a file of another run can never stand in for this one."""
    out_path = Path(out_path)
    out_path.unlink(missing_ok=True)
    env = {**os.environ, 'TESSEL_GIZMO_PERF_OUT': str(out_path), 'TESSEL_GIZMO_PERF_CALIBRATE': '1'}
    return harness.run_bounded([str(exe), '--ignored', '--test-threads=1', '--nocapture', ONE_TEST], cwd=ROOT,
                               env=env, timeout=harness.MEASURE_TIMEOUT_S)


def run_is_clean(done, path):
    """None when exactly the one expected test ran and passed and wrote a fresh report, else why not."""
    if done.timed_out:
        return f'timed out after {harness.MEASURE_TIMEOUT_S}s'
    if done.returncode != 0:
        return f'exit {done.returncode}: {done.text[-300:]}'
    if 'test result: ok. 1 passed; 0 failed' not in done.stdout:
        return 'the test summary is not "1 passed; 0 failed"'
    if not Path(path).exists():
        return 'the run wrote no report'
    return None


def clear_previous(out):
    for pattern in ('round*.json', 'round*.stdout.txt', 'warmup-*.json', 'verdicts.json'):
        for stale in Path(out).glob(pattern):
            stale.unlink()


# --- judgement -------------------------------------------------------------------------------------------------


def kinds(report):
    """{(corpus, kind): stats dict} from one report."""
    out = {}
    for corpus in report['corpora']:
        for phase in ('cold', 'hot'):
            for kind, stats in corpus[phase].items():
                out[(corpus['corpus'], kind)] = stats
    return out


def unstable_corpora(reports, limit=MAX_UNSTABLE_FRACTION):
    """{corpus: [fraction, ...]} for the corpora whose plan-reference blocks disagreed with themselves in a kept
    report more than `limit` allows (the machine was not steady enough to judge that corpus)."""
    out = {}
    for report in reports:
        for corpus in report['corpora']:
            fraction = corpus.get('reference', {}).get('unstable_fraction')
            if fraction is None or fraction > limit:
                out.setdefault(corpus['corpus'], []).append(fraction)
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


def judge(base, cand, unstable=None):
    """The pre-declared rule applied to pooled baseline/candidate samples and per-round medians."""
    unstable = unstable or {}
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
        elif key[0] in unstable:
            status = 'INCONCLUSIVE'
            reasons.append(f'the plan reference of `{key[0]}` was unstable in a kept run: {unstable[key[0]]}')
        else:
            status = 'NO_REGRESSION_IN_SCENARIO'
        verdicts[f'{key[0]}/{key[1]}'] = {'status': status, 'reasons': reasons, 'baseline': b, 'candidate': c,
                                          'baseline_run_spread': round(spread, 4),
                                          'candidate_slower_pairs': f'{slower}/{len(pairs)}',
                                          'median_ratio_candidate_over_baseline': round(c['p50_ns'] / b['p50_ns'], 4),
                                          'p95_ratio_candidate_over_baseline': round(c['p95_ns'] / b['p95_ns'], 4)}
    return verdicts


def overall(verdicts, differs, outside=()):
    """(status, reason, exit code) of a set of per-kind verdicts. `differs` are the product files the measured
    executable is built from that differ between the sides; `outside` the product files that differ but are not in it."""
    statuses = {v['status'] for v in verdicts.values()}
    if 'REGRESSED' in statuses:
        return 'REGRESSED', [k for k, v in verdicts.items() if v['status'] == 'REGRESSED'], 1
    if not differs and outside:
        return 'DIFFERENCE_OUTSIDE_THE_MEASUREMENT', (
            f'only files the measured executable is not built from differ ({sorted(outside)[:5]}): the two measured '
            'builds are the same product, and this A/B cannot say anything about those files'), harness.INCONCLUSIVE_EXIT
    if not differs:
        return 'A_A_NO_PRODUCT_DIFFERENCE', 'no production file differs: the two builds are the same product', \
            harness.INCONCLUSIVE_EXIT
    if 'INCONCLUSIVE' in statuses:
        return 'INCONCLUSIVE', [k for k, v in verdicts.items() if v['status'] == 'INCONCLUSIVE'], \
            harness.INCONCLUSIVE_EXIT
    return 'NO_REGRESSION_IN_SCENARIO', '', 0


# --- main ------------------------------------------------------------------------------------------------------


def main(argv=None):
    harness.install_termination_handlers()
    parser = argparse.ArgumentParser()
    parser.add_argument('--baseline', default=BASELINE)
    parser.add_argument('--allow-other-baseline', action='store_true',
                        help='compare against a ref other than the pinned `cbe9e1b` (recorded as such)')
    parser.add_argument('--rounds', type=int, default=4)
    parser.add_argument('--out', default='/tmp/gizmo-perf-ab')
    parser.add_argument('--phase', choices=['build', 'measure'], required=True,
                        help='`build`: both release builds and the executables copied to --out/exes (a build leaves the '
                             'machine loaded, which the guard refuses); `measure`: the guard, the lock and the rounds '
                             'from those executables, in their own invocation (no build, no waiting)')
    parser.add_argument('--allow-dirty', action='store_true',
                        help='measure a dirty tree (recorded; never counts for HEAD)')
    args = parser.parse_args(argv)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    manifest = {'suite': 'gizmo-perf-ab', 'phase': args.phase, 'utc_start': harness.utc(), 'cwd': str(ROOT),
                'command': sys.argv, 'baseline': args.baseline, 'candidate': harness.git_identity(),
                'toolchain': harness.toolchain(), 'rounds': args.rounds,
                'rules': {'median_tolerance': MEDIAN_TOLERANCE, 'p95_tolerance': P95_TOLERANCE,
                          'baseline_spread_tolerance': BASELINE_SPREAD_TOLERANCE, 'long_tick_ns': LONG_TICK_NS,
                          'max_unstable_fraction': MAX_UNSTABLE_FRACTION},
                'status': 'ABORTED'}
    manifest_path = out / 'manifest.json'
    if manifest['candidate']['dirty'] and not args.allow_dirty:
        manifest.update(status='REFUSED', reason='the working tree is dirty: numbers would not belong to HEAD')
        return harness.die(1, manifest, manifest_path)
    manifest['dirty_run'] = bool(manifest['candidate']['dirty'])
    manifest['baseline_commit'] = harness.out(['git', 'rev-parse', args.baseline])
    if manifest['baseline_commit'] != BASELINE and not args.allow_other_baseline:
        manifest.update(status='REFUSED', reason=f'the baseline is pinned to {BASELINE}; got {manifest["baseline_commit"]}')
        return harness.die(1, manifest, manifest_path)
    manifest['baseline_is_pinned'] = manifest['baseline_commit'] == BASELINE
    exes_dir = out / 'exes'
    exes = {'baseline': exes_dir / 'baseline.exe', 'candidate': exes_dir / 'candidate.exe'}
    # The baseline's tests read the journey and the corpus from the checkout the executable was COMPILED in
    # (`CARGO_MANIFEST_DIR` is a compile-time path), so that source tree must outlive the build phase: it lives in
    # `--out/baseline-src` (source only, the target is the repository's default) until a judged run has finished.
    kept_src = out / 'baseline-src'
    try:
        if args.phase == 'build':
            # builds (no lock: a build is not a measurement), sequential, default target.
            exes_dir.mkdir(exist_ok=True)
            shutil.rmtree(kept_src, ignore_errors=True)
            try:
                record = make_baseline(args.baseline, out)
            except (Refused, subprocess.CalledProcessError) as error:
                manifest.update(status='REFUSED_CONTAMINATION', reason=str(error))
                return harness.die(1, manifest, manifest_path)
            src = record.pop('src')
            manifest['prod_trees'] = record
            c_command, c_done, c_exe = build(ROOT)
            manifest['candidate_build'] = {'command': c_command, **c_done.record(), 'executable': c_exe}
            b_command, b_done, b_exe = build(src)
            manifest['baseline_build'] = {'command': b_command, **b_done.record(), 'executable': b_exe,
                                          'cwd': str(src), 'stderr_tail': b_done.stderr[-1500:]}
            if not (c_exe and b_exe and c_done.returncode == 0 and b_done.returncode == 0):
                shutil.rmtree(kept_src, ignore_errors=True)
                manifest.update(status='BUILD_FAILED', reason='a release build failed or timed out')
                return harness.die(1, manifest, manifest_path)
            shutil.copy(b_exe, exes['baseline'])
            shutil.copy(c_exe, exes['candidate'])
            manifest['executables_sha256'] = {k: harness.sha256(v) for k, v in exes.items()}
            manifest['baseline_src_kept_at'] = str(kept_src)
            too_big = harness.artifact_reason(exes_dir) or harness.artifact_reason(kept_src)
            if too_big:
                manifest.update(status='REFUSED', reason=too_big)
                return harness.die(1, manifest, manifest_path)
            manifest.update(status='BUILT', reason='no measurement was taken')
            harness.write(out / 'manifest.build.json', manifest)
            print(json.dumps({'status': 'BUILT', 'executables_sha256': manifest['executables_sha256'],
                              'prod_trees': {k: v for k, v in record.items() if k.startswith('prod_tree')},
                              'product_files_that_differ': sorted(record['product_files_that_differ']),
                              'product_files_outside_the_measurement': record['product_files_outside_the_measurement']}))
            return 0
        build_manifest_path = out / 'manifest.build.json'
        if not build_manifest_path.exists():
            manifest.update(status='REFUSED', reason='no build phase manifest in --out: run `--phase build` first')
            return harness.die(1, manifest, manifest_path)
        built = json.loads(build_manifest_path.read_text())
        for key in ('candidate_build', 'baseline_build', 'executables_sha256', 'prod_trees'):
            manifest[key] = built[key]
        if built['candidate']['head'] != manifest['candidate']['head'] or \
                built['candidate']['tree'] != manifest['candidate']['tree']:
            manifest.update(status='REFUSED', reason='the executables were built from another HEAD/tree')
            return harness.die(1, manifest, manifest_path)
        # The candidate executable of a `--phase build --allow-dirty` holds local changes; once they are stashed the
        # tree reads clean again, but that executable is still not HEAD's. An empty string is the only clean record.
        built_dirty = built['candidate'].get('dirty', 'not recorded') != ''
        if built_dirty and not args.allow_dirty:
            manifest.update(status='REFUSED', reason='the executables were built from a dirty tree (or their manifest '
                                                     'does not say it was clean): they cannot be judged as HEAD')
            return harness.die(1, manifest, manifest_path)
        manifest['dirty_run'] = manifest['dirty_run'] or built_dirty
        if built['baseline_commit'] != manifest['baseline_commit']:
            manifest.update(status='REFUSED', reason='the executables were built against another baseline')
            return harness.die(1, manifest, manifest_path)
        if not kept_src.is_dir():
            manifest.update(status='REFUSED', reason='the baseline source the executable was compiled in is gone '
                                                     '(its tests read the journey from there): run `--phase build` again')
            return harness.die(1, manifest, manifest_path)
        if any(not exes[k].exists() or harness.sha256(exes[k]) != built['executables_sha256'][k] for k in exes):
            manifest.update(status='REFUSED', reason='an executable is missing or changed since the build phase')
            return harness.die(1, manifest, manifest_path)
        clear_previous(out)
        # guards + the lock, once.
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
            order = []
            for r in range(args.rounds):
                order += ['baseline', 'candidate'] if r % 2 == 0 else ['candidate', 'baseline']
            runs = manifest['runs'] = []
            # declared warmup: one discarded run per side, alternating, before the kept rounds.
            for side in ('baseline', 'candidate'):
                done = run_once(exes[side], out / f'warmup-{side}.json')
                runs.append({'side': side, 'kept': False, 'warmup': True, **done.record(),
                             'load_after': os.getloadavg()})
                problem = run_is_clean(done, out / f'warmup-{side}.json')
                if problem:
                    manifest.update(status='FAIL', reason=f'warmup ({side}) failed: {problem}')
                    return harness.die(1, manifest, manifest_path)
            reports = {'baseline': [], 'candidate': []}
            for index, side in enumerate(order):
                path = out / f'round{index:02d}-{side}.json'
                done = run_once(exes[side], path)
                (out / f'round{index:02d}-{side}.stdout.txt').write_text(done.stdout)
                runs.append({'side': side, 'kept': True, 'index': index, **done.record(), 'report': path.name,
                             'report_sha256': harness.sha256(path) if path.exists() else None,
                             'load_after': os.getloadavg(), 'thermal': harness.machine()['thermal'][:300]})
                problem = run_is_clean(done, path)
                if problem:
                    manifest.update(status='FAIL', reason=f'round {index} ({side}) failed: {problem}')
                    return harness.die(1, manifest, manifest_path)
                reports[side].append(json.loads(path.read_text()))
            after_reason = harness.judge_after(manifest)
        unstable = {**unstable_corpora(reports['baseline']), **unstable_corpora(reports['candidate'])}
        verdicts = judge(pooled(reports['baseline']), pooled(reports['candidate']), unstable)
        harness.write(out / 'verdicts.json', verdicts)
        manifest['verdicts_sha256'] = harness.sha256(out / 'verdicts.json')
        manifest['verdict_counts'] = {s: sum(1 for v in verdicts.values() if v['status'] == s)
                                      for s in sorted({v['status'] for v in verdicts.values()})}
        if after_reason:
            manifest.update(status='INVALID_GUARD_AFTER_RUN', reason=after_reason, exit=harness.BLOCKED_EXIT,
                            note='the numbers are kept in verdicts.json as observations; nothing is judged')
            manifest['utc_end'] = harness.utc()
            return harness.die(harness.BLOCKED_EXIT, manifest, manifest_path)
        # Only a judged run is over: the executables and the baseline source go now. A BLOCKED or INVALID one (the return
        # above) keeps both, so the same build can be measured again when the machine is quiet.
        shutil.rmtree(exes_dir, ignore_errors=True)
        shutil.rmtree(kept_src, ignore_errors=True)
        status, reason, code = overall(verdicts, manifest['prod_trees']['product_files_that_differ'],
                                       manifest['prod_trees'].get('product_files_outside_the_measurement', []))
        manifest.update(status=status, reason=reason)
        manifest['utc_end'] = harness.utc()
        manifest['exit'] = code
        harness.write(manifest_path, manifest)
        print(json.dumps({k: manifest[k] for k in ('status', 'reason', 'verdict_counts', 'exit')}))
        return code
    except BaseException:
        if args.phase == 'build':
            shutil.rmtree(kept_src, ignore_errors=True)
        raise


if __name__ == '__main__':
    raise SystemExit(main())
