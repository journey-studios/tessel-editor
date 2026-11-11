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
this A/B cannot see those files, so it never turns them into a "no regression". A candidate that differs in the core AND
outside it gets the verdict of the core part only: the files outside stay in `product_files_outside_the_measurement` and
never enter the verdict, so a `NO_REGRESSION_IN_SCENARIO` says nothing about them.

How. One short `git archive` of the baseline into a temp directory, built SEQUENTIALLY through the repository's cargo
wrapper with its DEFAULT target. A build phase REFUSES to start when `CARGO_TARGET_DIR` or `TESSEL_CARGO_LANE` is set (both
builds would share one artifact path), and it copies the candidate executable out RIGHT AFTER its own build, before the
baseline build starts (the cache GC pruned the candidate's managed target during the baseline build of the first real
A/B); after both builds it refuses (`AB_EXECUTABLES_COLLIDE`) when the two are the same file or have the same sha256, and
any OSError on a copy or on a build output is a recorded `INFRA_EXECUTABLE_MISSING` naming the path, never a traceback. The
temp directory is deleted. The performance lock is taken ONCE, without waiting; a busy lock or a failed machine guard is a
recorded BLOCKED (exit 3). Every child has a finite timeout. `--rounds` must be at least 1 (argument error, exit 2); a
comparison with no sample is INCONCLUSIVE, never a pass. Rounds alternate the order (A B | B A | A B | B A ...),
each side runs a declared warmup run first that is NOT kept (and said so in the report), and every kept run's raw
samples are pooled per (corpus, request kind): nothing is trimmed, no tail dropped, no best-of.

Rules, fixed BEFORE any candidate number is read in a run (never edited after looking; a change of a rule is a
new commit, recorded, and applies to the next run):
  * metrics per kind: median, p95, p99, max, lost samples (n below expected), long ticks (>16.6 ms) and the
    plan-reference ratio (the request over the scene's own `ScenePlan::build_with`, which cancels machine speed). p99
    and max are RECORDED, not judged: one scheduler hiccup in a shared machine would decide a verdict;
  * a kind REGRESSES when the pooled candidate median is more than 10% over the baseline's AND the candidate is
    slower in a majority of the round pairs for that median; or when its pooled p95 is more than 25% over the
    baseline's AND the candidate's p95 is higher in a majority of the round pairs for the p95 (each gate counts its OWN
    statistic per round: the p95 gate never borrows the pairs of the medians, so a tail-only slowdown is judged);
    or when it lost samples or long ticks the baseline did not;
  * noise: when the baseline's own median varies more than 15% between its rounds for a kind, or when more than
    `budgets.json: reference.max_unstable_blocks` of the plan-reference blocks of its corpus disagreed with themselves
    (read from every kept report), that kind is INCONCLUSIVE and says so (never a pass);
  * after the last run the guards are read again (load 1/5 min, thermal, disk); a failed guard makes the whole run
    INVALID (exit 3): the numbers are kept but nothing is judged;
  * "no regression" is only ever claimed for this scenario/config/hardware/backend, not for the native window.
Exit 0 = no kind regressed and none inconclusive, 1 = a kind regressed or a refusal, 3 = BLOCKED/INVALID,
4 = INCONCLUSIVE (or A/A).

Ownership (nothing waits; a taken lock is one REFUSED line, exit 1, nothing written): `--out/harness.OUT_LOCK`
(`.gizmo-perf.lock`, shared by every runner, so a `run.py`, `native.py` or `mutate.py` on the same `--out` is refused too)
makes one run per `--out` in both phases, and a build also takes the lock of the CHECKOUT (`harness.checkout_lock_path`).
The lock excludes simultaneous runs, not sequential ones: keep one `--out` per runner. A checkout git cannot name
(`harness.git_identity` unavailable) and a baseline that names no commit are REFUSED, `--allow-dirty` included. Every git
this runner starts (the identity, `ls-tree`, `archive`, `show`, `diff`, `apply`) runs under `harness.git_env`: a `GIT_DIR` or
`GIT_WORK_TREE` exported by the caller would otherwise make git answer for another repository and the baseline archive, the
production manifests and the identity would describe it.
Every child gets `harness.child_env`: no inherited `TESSEL_GIZMO_PERF_*`; the runner sets the report path and
`CALIBRATE=1` itself (`run_env`) and records what it dropped. Before a measure phase writes anything, what an earlier
attempt left in `--out` (manifest, kept reports, verdicts) is moved to `--out/attempts/<utc>-<status>/`
(`archive_previous_attempt`) and its path is recorded in the manifest: a BLOCKED or INVALID attempt is never overwritten.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import statistics
import subprocess
import sys
import time

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


def git_run(args, cwd, **kwargs):
    """`subprocess.run(['git', *args], cwd=cwd, check=True, **kwargs)` under `harness.git_env()`: the one way this runner
    starts git. Without it a `GIT_DIR`/`GIT_WORK_TREE` of the caller makes git ignore `cwd` and answer for another repository
    (a baseline archive of it, its production manifest). A failing git still raises `CalledProcessError`, as before."""
    return subprocess.run(['git', *args], cwd=cwd, env=harness.git_env(), check=True, **kwargs)


def git_prod_manifest(ref, root=ROOT):
    """{path: git blob id} of the production files of `ref`."""
    listing = git_run(['ls-tree', '-r', ref], root, stdout=subprocess.PIPE, text=True).stdout
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
    archive = git_run(['archive', ref], root, stdout=subprocess.PIPE)
    subprocess.run(['tar', '-x', '-C', str(src)], input=archive.stdout, check=True)
    pinned = git_prod_manifest(ref, root)
    before = dir_prod_manifest(src)
    if before != pinned:
        raise Refused(f'the archive of {ref} is not its production tree: {changed_paths(before, pinned)[:5]}')
    # 1. test-only instrument, from the candidate.
    for path in INSTRUMENT_TESTS:
        piece = git_run(['archive', candidate, '--', path], root, stdout=subprocess.PIPE).stdout
        subprocess.run(['tar', '-x', '-C', str(src)], input=piece, check=True)
    # 2. the counter module and its declaration, only when the pinned ref does not have them.
    if PROBE_FILE not in pinned:
        probe = git_run(['show', f'{candidate}:{PROBE_FILE}'], root, stdout=subprocess.PIPE).stdout
        (src / PROBE_FILE).write_bytes(probe)
        numstat = git_run(['diff', '--numstat', ref, candidate, '--', PROBE_DECL_FILE], root, stdout=subprocess.PIPE,
                          text=True).stdout.split()
        patch = git_run(['diff', '--binary', ref, candidate, '--', PROBE_DECL_FILE], root, stdout=subprocess.PIPE).stdout
        added = [l[1:] for l in patch.decode().splitlines() if l.startswith('+') and not l.startswith('+++')]
        removed = [l for l in patch.decode().splitlines() if l.startswith('-') and not l.startswith('---')]
        if numstat[:2] != ['1', '0'] or added != [PROBE_DECL_LINE] or removed:
            raise Refused(f'`{PROBE_DECL_FILE}` differs from {ref} by more than `{PROBE_DECL_LINE}`: a production '
                          'change cannot be overlaid onto the baseline')
        git_run(['apply', '--whitespace=nowarn', '-'], src, input=patch)
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
    # No target directory of the caller's: `main` refuses one before it builds, and the child never inherits one.
    done = harness.run_bounded(command, cwd=cwd, env=harness.child_env(build=True), timeout=harness.BUILD_TIMEOUT_S)
    return command, done, find_exe(done.stdout)


def run_env(out_path):
    """What the runner sets for one measured run, apart from the caller's environment: the report path and
    `CALIBRATE=1` (the A/B numbers are taken in the calibration mode: the executable asserts no budget, `judge` decides).
    Everything else under `TESSEL_GIZMO_PERF_*` that the caller exported is dropped (`harness.child_env`)."""
    return {'TESSEL_GIZMO_PERF_OUT': str(out_path), 'TESSEL_GIZMO_PERF_CALIBRATE': '1'}


def run_once(exe, out_path):
    """One controlled run. The report is removed first, so a file of another run can never stand in for this one."""
    out_path = Path(out_path)
    out_path.unlink(missing_ok=True)
    return harness.run_bounded([str(exe), '--ignored', '--test-threads=1', '--nocapture', ONE_TEST], cwd=ROOT,
                               env=harness.child_env(run_env(out_path)), timeout=harness.MEASURE_TIMEOUT_S)


def copy_executable(source, target):
    """`None` when `source` was copied to `target`, else the sentence for the record: which path, what the OS said. The
    path a build printed can be gone by the time it is read (the first real A/B lost the candidate's executable to a
    pruned managed target while the baseline was building), and that is a recorded refusal, never a traceback."""
    try:
        shutil.copy(source, target)
    except OSError as error:
        return f'cannot copy {source} to {target}: {error}'
    return None


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


ATTEMPT_FILES = ('manifest.json', 'round*.json', 'round*.stdout.txt', 'warmup-*.json', 'verdicts.json')


def archive_previous_attempt(out):
    """Moves what an earlier attempt in `out` left (its manifest, the kept reports and their stdout, the warmups and the
    verdicts) into `out/attempts/<utc of that attempt>-<its status>/` and returns that directory, or None when there was
    nothing. It runs BEFORE the new attempt writes anything, so a BLOCKED or INVALID attempt is never overwritten: the
    first real A/B lost the numbers of its invalid first attempt to its own retry. The executables, the baseline source
    and `manifest.build.json` are the INPUTS of an attempt and stay where they are. A directory name is never reused."""
    out = Path(out)
    files = sorted({path for pattern in ATTEMPT_FILES for path in out.glob(pattern) if path.is_file()})
    if not files:
        return None
    try:
        previous = json.loads((out / 'manifest.json').read_text())
    except (OSError, ValueError):
        previous = {}
    started = previous.get('utc_start') or time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime(
        min(path.stat().st_mtime for path in files)))
    status = previous.get('status') or 'UNKNOWN'
    base = out / 'attempts' / (re.sub(r'[^0-9A-Za-z]+', '', str(started)) + '-' + re.sub(r'[^0-9A-Za-z_]+', '', str(status)))
    target, count = base, 1
    while target.exists():
        count += 1
        target = Path(f'{base}-{count}')
    target.mkdir(parents=True)
    for path in files:
        shutil.move(str(path), str(target / path.name))
    return target


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
    """Per (corpus, kind): every raw sample of every kept run, with the per-run medians and the per-run p95s (each gate
    of `judge` counts the rounds in which the candidate was slower by ITS OWN statistic)."""
    pool = {}
    for report in reports:
        for key, stats in kinds(report).items():
            slot = pool.setdefault(key, {'samples': [], 'medians': [], 'p95s': [], 'ratios': [], 'long_ticks': 0, 'n': []})
            slot['samples'] += stats['samples_ns']
            slot['medians'].append(stats['p50_ns'])
            slot['p95s'].append(pct(stats['samples_ns'], 0.95))
            slot['ratios'].append(stats['ratio_p50'])
            slot['long_ticks'] += stats.get('long_ticks', 0)
            slot['n'].append(stats['n'])
    return pool


def summarize(slot):
    s = slot['samples']
    return {'n': len(s), 'p50_ns': pct(s, 0.5), 'p95_ns': pct(s, 0.95), 'p99_ns': pct(s, 0.99), 'max_ns': max(s),
            'long_ticks': sum(1 for v in s if v > LONG_TICK_NS), 'run_medians_ns': slot['medians'],
            'run_p95_ns': slot['p95s'],
            'run_ratio_p50': slot['ratios'], 'expected_n_per_run': slot['n']}


def judge(base, cand, unstable=None):
    """The pre-declared rule applied to pooled baseline/candidate samples, per-round medians and per-round p95s."""
    unstable = unstable or {}
    verdicts = {}
    for key in sorted(base):
        absent = [side for side, pool in (('baseline', base), ('candidate', cand)) if not pool.get(key, {}).get('samples')]
        if absent:
            # No sample is not "no regression": with nothing measured on a side there is nothing to compare, and the
            # statistics below would either crash (`max([])`) or call an empty comparison a pass.
            verdicts[f'{key[0]}/{key[1]}'] = {
                'status': 'INCONCLUSIVE', 'reasons': [f'no sample on the {" and ".join(absent)} side: nothing to compare'],
                'baseline': {'n': len(base.get(key, {}).get('samples', []))},
                'candidate': {'n': len(cand.get(key, {}).get('samples', []))}}
            continue
        b, c = summarize(base[key]), summarize(cand[key])
        medians = base[key]['medians']
        spread = (max(medians) - min(medians)) / statistics.median(medians)
        # Each gate has its OWN majority: the median gate counts the round pairs whose MEDIAN is slower, the p95 gate the pairs
        # whose p95 is. A tail-only slowdown leaves the medians as they were, so a p95 gate that borrowed the median pairs
        # would read a coin toss (R2).
        pairs50 = list(zip(base[key]['medians'], cand[key]['medians']))
        pairs95 = list(zip(base[key]['p95s'], cand[key]['p95s']))
        slower = sum(1 for x, y in pairs50 if y > x)
        slower95 = sum(1 for x, y in pairs95 if y > x)
        reasons = []
        p50_over = c['p50_ns'] > b['p50_ns'] * MEDIAN_TOLERANCE
        p95_over = c['p95_ns'] > b['p95_ns'] * P95_TOLERANCE
        if p50_over and slower * 2 > len(pairs50):
            reasons.append(f"median {c['p50_ns']} > {b['p50_ns']} x {MEDIAN_TOLERANCE} (slower in {slower}/{len(pairs50)} pairs)")
        if p95_over and slower95 * 2 > len(pairs95):
            reasons.append(f"p95 {c['p95_ns']} > {b['p95_ns']} x {P95_TOLERANCE} (p95 slower in {slower95}/{len(pairs95)} pairs)")
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
                                          'candidate_slower_pairs': f'{slower}/{len(pairs50)}',
                                          'candidate_slower_p95_pairs': f'{slower95}/{len(pairs95)}',
                                          'median_ratio_candidate_over_baseline': round(c['p50_ns'] / b['p50_ns'], 4),
                                          'p95_ratio_candidate_over_baseline': round(c['p95_ns'] / b['p95_ns'], 4)}
    return verdicts


def overall(verdicts, differs, outside=()):
    """(status, reason, exit code) of a set of per-kind verdicts. `differs` are the product files the measured
    executable is built from that differ between the sides; `outside` the product files that differ but are not in it."""
    statuses = {v['status'] for v in verdicts.values()}
    if 'REGRESSED' in statuses:
        return 'REGRESSED', [k for k, v in verdicts.items() if v['status'] == 'REGRESSED'], 1
    if not verdicts:
        # Zero kinds judged (no round ran, or no report had a sample): the loop over "every kind" never executed, which
        # is not "every kind passed".
        return 'INCONCLUSIVE', 'no kind was judged: there is no sample to compare', harness.INCONCLUSIVE_EXIT
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
    if args.rounds < 1:
        # `range(0)` runs nothing, and "every kind was judged" is vacuously true of zero kinds: refused as an argument.
        parser.error(f'--rounds must be at least 1 (got {args.rounds}): a run with no round has no sample to judge')
    out = Path(args.out)
    # Ownership first, before anything in the checkout or in `--out` is read, written or removed (a refusal's own
    # manifest included), and kept until this returns: a build owns the CHECKOUT too (nothing else may build from it or
    # put a mutant in its tree meanwhile), every phase owns `--out/harness.OUT_LOCK`, the lock every runner shares. Neither
    # lock waits: a taken one is ONE `REFUSED` line, exit 1, and nothing written. This runner keeps manifest.json,
    # manifest.build.json, the round, warmup and verdict files, exes/, baseline-src/ and attempts/ in `--out`.
    with harness.guarded(out, 'ab.py', builds=args.phase == 'build') as refusal:
        if refusal:
            return harness.die(1, refusal, None)
        return run_phase(args, out)


def run_phase(args, out):
    """The phase `args.phase` over `out`, which the caller owns (`out/harness.OUT_LOCK` is held). Returns the exit code."""
    manifest = {'suite': 'gizmo-perf-ab', 'phase': args.phase, 'utc_start': harness.utc(), 'cwd': str(ROOT),
                'command': sys.argv, 'baseline': args.baseline, 'candidate': harness.git_identity(),
                'toolchain': harness.toolchain(), 'rounds': args.rounds,
                'rules': {'median_tolerance': MEDIAN_TOLERANCE, 'p95_tolerance': P95_TOLERANCE,
                          'baseline_spread_tolerance': BASELINE_SPREAD_TOLERANCE, 'long_tick_ns': LONG_TICK_NS,
                          'max_unstable_fraction': MAX_UNSTABLE_FRACTION},
                'status': 'ABORTED'}
    manifest_path = out / 'manifest.json'
    # What an earlier attempt left in `--out` is moved aside BEFORE this one writes anything (a refusal's manifest
    # included): the previous manifest, its kept reports and its verdicts are evidence, not scratch.
    archived = archive_previous_attempt(out)
    manifest['previous_attempt_archived_at'] = str(archived) if archived else None
    problem = harness.identity_problem(manifest['candidate'])
    if problem:  # a checkout git cannot name is refused before anything else, `--allow-dirty` included
        manifest.update(status='REFUSED', reason=problem)
        return harness.die(1, manifest, manifest_path)
    if manifest['candidate']['dirty'] and not args.allow_dirty:
        manifest.update(status='REFUSED', reason='the working tree is dirty: numbers would not belong to HEAD')
        return harness.die(1, manifest, manifest_path)
    manifest['dirty_run'] = bool(manifest['candidate']['dirty'])
    # The baseline is named by a STRICT git call: the empty text a failing `git rev-parse` printed used to be recorded as
    # "the baseline commit" and, with `--allow-other-baseline`, accepted, and two empty names are equal.
    resolved, commit = harness.git_strict(['rev-parse', '--verify', '--quiet', f'{args.baseline}^{{commit}}'])
    if not resolved or not commit:
        manifest.update(status='REFUSED', reason=f'the baseline `{args.baseline}` names no commit here ({commit or "no output"})')
        return harness.die(1, manifest, manifest_path)
    manifest['baseline_commit'] = commit
    if manifest['baseline_commit'] != BASELINE and not args.allow_other_baseline:
        manifest.update(status='REFUSED', reason=f'the baseline is pinned to {BASELINE}; got {manifest["baseline_commit"]}')
        return harness.die(1, manifest, manifest_path)
    manifest['baseline_is_pinned'] = manifest['baseline_commit'] == BASELINE
    manifest['child_env'] = {'set_by_the_runner': {'TESSEL_GIZMO_PERF_OUT': '<the report of each run>',
                                                   'TESSEL_GIZMO_PERF_CALIBRATE': '1'},
                             'dropped_from_the_caller': harness.dropped_env()}
    exes_dir = out / 'exes'
    exes = {'baseline': exes_dir / 'baseline.exe', 'candidate': exes_dir / 'candidate.exe'}
    # The baseline's tests read the journey and the corpus from the checkout the executable was COMPILED in
    # (`CARGO_MANIFEST_DIR` is a compile-time path), so that source tree must outlive the build phase: it lives in
    # `--out/baseline-src` (source only, the target is the repository's default) until a judged run has finished.
    kept_src = out / 'baseline-src'
    try:
        if args.phase == 'build':
            # builds (no performance lock: a build is not a measurement), sequential, default target.
            chosen = [name for name in harness.BUILD_ENV if name in os.environ]
            if chosen:
                # With a target directory chosen from outside, both builds would share one artifact path: the second one
                # overwrites, or the cache GC prunes, what the first printed (BL015.23). Nothing is touched.
                manifest.update(status='REFUSED', env_set=chosen, reason=(
                    f'{" and ".join(chosen)} set: the two builds would not each have their own default target, so the '
                    'baseline build could replace or prune the candidate executable; unset it for this phase'))
                return harness.die(1, manifest, manifest_path)

            def fail(status, reason, **extra):
                """A refused or failed build: the baseline source is not kept (nothing can be measured from it)."""
                shutil.rmtree(kept_src, ignore_errors=True)
                manifest.update(status=status, reason=reason, **extra)
                return harness.die(1, manifest, manifest_path)

            exes_dir.mkdir(exist_ok=True)
            shutil.rmtree(kept_src, ignore_errors=True)
            # The products of an earlier build go first: a build that fails must not leave the previous record or an old
            # executable beside new ones, where `--phase measure` could believe the pair.
            (out / 'manifest.build.json').unlink(missing_ok=True)
            for stale in exes.values():
                stale.unlink(missing_ok=True)
            try:
                record = make_baseline(args.baseline, out)
            except (Refused, subprocess.CalledProcessError) as error:
                manifest.update(status='REFUSED_CONTAMINATION', reason=str(error))
                return harness.die(1, manifest, manifest_path)
            src = record.pop('src')
            manifest['prod_trees'] = record
            c_command, c_done, c_exe = build(ROOT)
            manifest['candidate_build'] = {'command': c_command, **c_done.record(), 'executable': c_exe}
            if not (c_exe and c_done.returncode == 0):
                return fail('BUILD_FAILED', 'the candidate release build failed or timed out')
            # The candidate executable is copied out RIGHT NOW, before the baseline build starts: the cache GC of the
            # repository's cargo wrapper pruned the candidate's managed target while the baseline was building, and the
            # path cargo printed was gone when it was read (the first real A/B, `ab-build-037eda5`).
            problem = copy_executable(c_exe, exes['candidate'])
            manifest['candidate_copied_before_baseline_build'] = problem is None
            if problem:
                return fail('INFRA_EXECUTABLE_MISSING', problem, missing=str(c_exe))
            b_command, b_done, b_exe = build(src)
            manifest['baseline_build'] = {'command': b_command, **b_done.record(), 'executable': b_exe,
                                          'cwd': str(src), 'stderr_tail': b_done.stderr[-1500:]}
            if not (b_exe and b_done.returncode == 0):
                return fail('BUILD_FAILED', 'the baseline release build failed or timed out')
            problem = copy_executable(b_exe, exes['baseline'])
            if problem:
                return fail('INFRA_EXECUTABLE_MISSING', problem, missing=str(b_exe))
            try:
                same_file = os.path.samefile(c_exe, b_exe) if os.path.exists(c_exe) and os.path.exists(b_exe) \
                    else str(c_exe) == str(b_exe)
                manifest['executables_sha256'] = {k: harness.sha256(v) for k, v in exes.items()}
            except OSError as error:
                return fail('INFRA_EXECUTABLE_MISSING', f'cannot read the copied executables: {error}')
            if same_file or len(set(manifest['executables_sha256'].values())) < 2:
                # One artifact measured as both sides is the A/A that a "no regression" would be made of.
                return fail('AB_EXECUTABLES_COLLIDE', (
                    'the candidate and the baseline executables are the same file' if same_file else
                    'the candidate and the baseline executables have the same sha256') +
                    ': one artifact would be measured as both sides', same_file=same_file)
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
        # `same_head` also refuses a build record whose candidate names no usable head/tree: two empty answers are not
        # "the same HEAD".
        if not harness.same_head(built['candidate'], manifest['candidate']):
            manifest.update(status='REFUSED', reason='the executables were built from another HEAD/tree (or the build '
                                                     'record names none)')
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
        if len(set(built['executables_sha256'].values())) < len(exes):
            # A build record whose two sides are one artifact (an older build phase did not look): no rounds are run.
            manifest.update(status='AB_EXECUTABLES_COLLIDE', reason='the candidate and the baseline executables have the '
                                                                     'same sha256: one artifact would be both sides')
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
