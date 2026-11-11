"""Shared pieces of the gizmo performance runners (no suite logic): the performance lock, the
resource guards, the environment record and the manifest writer.

The lock is the repository's performance lock (`/tmp/tessel-scene-gpu.lock`, AGENTS.md). It is taken ONCE,
without waiting (`LOCK_NB`): if somebody holds it the runner records BLOCKED and stops. No loop, no sleep, no new
wait, no kill, no bypass. Builds and functional tests never take it.

Every child process runs through `run_bounded`: a FINITE timeout, its own process group, and on expiry the whole
group is terminated (then killed) and the partial output is kept. A runner that holds the lock therefore cannot be
held forever by a hung test. `install_termination_handlers` turns SIGTERM/SIGINT into `SystemExit`, so the `finally`
blocks and `with` statements (lock release, restoring a mutated checkout) run when a runner is asked to stop.

Four more rules live here so that every runner has them (BL015.23 items 23 and following, reviews 5856147194 and 5859397176):
  * a git answer that DECIDES something is `git_strict`/`git_identity` (success is explicit; a failing git is an
    `unavailable` identity every runner refuses, `--allow-dirty` included). `out` stays tolerant, on purpose, for telemetry
    (`pmset`, `sysctl`, the toolchain): there a missing answer is only recorded text;
  * EVERY git a runner starts (here, in `ab.py`, in `native.py`) runs under `git_env`: the caller's environment without git's
    own repository-local variables (`GIT_DIR`, `GIT_WORK_TREE`, ...). With one of them exported git ignores the directory it
    was started in and answers for ANOTHER repository, and the identity of that one would be filed under this checkout;
  * a child that measures or builds gets `child_env`: no inherited `TESSEL_GIZMO_PERF_*` (and no `CARGO_TARGET_DIR` or
    `TESSEL_CARGO_LANE` for a build), exactly what the runner sets, and what was dropped is recorded;
  * `guarded` is the ownership of a run: the lock of the CHECKOUT for a phase that builds from it or edits it, then the
    ONE lock of `--out` that every runner shares (`OUT_LOCK`). Neither waits; a refusal writes nothing.
"""
import contextlib
import fcntl
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import signal
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parent.parent.parent
LOCK = '/tmp/tessel-scene-gpu.lock'
MIN_FREE_GIB_FOR_HEAVY = 16
FLOOR_FREE_GIB = 12
BLOCKED_EXIT = 3
INCONCLUSIVE_EXIT = 4


def utc():
    return time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


BUILD_TIMEOUT_S = 3600      # a release build through the cargo wrapper
TEST_TIMEOUT_S = 1800       # a functional `cargo test` (build included)
MEASURE_TIMEOUT_S = 900     # one controlled run of the benchmark executable
ARTIFACT_LIMIT_MB = 500     # nothing a runner leaves in /tmp may exceed this


class Done:
    """What a bounded child did. `timed_out` is explicit: a timeout is never mistaken for a test result."""

    def __init__(self, command, returncode, stdout, stderr, timed_out=False, seconds=0.0, error=None):
        self.command = command
        self.returncode = returncode
        self.stdout = stdout
        self.stderr = stderr
        self.timed_out = timed_out
        self.seconds = seconds
        self.error = error

    @property
    def text(self):
        return (self.stdout or '') + (self.stderr or '')

    def record(self):
        return {'exit': self.returncode, 'timed_out': self.timed_out, 'seconds': round(self.seconds, 2),
                'error': self.error}


def run_bounded(command, cwd=ROOT, env=None, timeout=BUILD_TIMEOUT_S, grace=10):
    """Runs `command` in its own process group with a finite `timeout` (seconds); never raises for the child.

    A command that cannot start (`FileNotFoundError`, permission) is `returncode=127` with `error` set. On timeout
    the group gets SIGTERM, `grace` seconds, then SIGKILL (always: see `_kill_group`); the output is then read for at
    most `grace` more seconds, whatever was printed is kept and `timed_out` is True. The call therefore returns within
    `timeout + 3 * grace` seconds whatever the child and its descendants do, and so does the lock a runner holds.
    """
    started = time.monotonic()
    if env is None:
        env = child_env()  # never a bare inheritance: no `TESSEL_GIZMO_PERF_*` of the caller reaches a child
    try:
        process = subprocess.Popen(command, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   text=True, start_new_session=True)
    except OSError as error:
        return Done(command, 127, '', '', error=f'cannot start {command[0]}: {error}')
    timed_out = False
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        timed_out = True
        _kill_group(process, grace)
        stdout, stderr = _drain(process, grace)
    except BaseException:
        # SIGTERM/Ctrl-C of the runner itself: never leave the child group behind.
        _kill_group(process, grace)
        raise
    returncode = process.returncode if process.returncode is not None else -signal.SIGKILL
    return Done(command, returncode, stdout or '', stderr or '', timed_out, time.monotonic() - started)


def _kill_group(process, grace):
    """SIGTERM to the whole group, up to `grace` seconds for its leader, then SIGKILL to the group REGARDLESS. The
    leader exiting does not mean the group is gone: a descendant that ignores SIGTERM keeps the output pipes open (so
    the read below would never end) and the performance lock held. The group id outlives its leader as long as one
    member lives; with none left `killpg` finds nothing."""
    for sig in (signal.SIGTERM, signal.SIGKILL):
        try:
            os.killpg(process.pid, sig)
        except (OSError, ProcessLookupError):
            pass
        try:
            process.wait(grace)
        except subprocess.TimeoutExpired:
            pass


def _drain(process, grace):
    """The output of a child whose group was just ended, read for at most `grace` seconds. The pipes close when the
    group's members die; a descendant that left the group (its own session) can still hold them, so the read is
    bounded too: what was read so far is kept, the pipes are closed here and the caller already marked the run timed out."""
    try:
        return process.communicate(timeout=grace)
    except subprocess.TimeoutExpired as expired:
        for pipe in (process.stdout, process.stderr):
            if pipe is not None:
                pipe.close()
        return tuple(part.decode(errors='replace') if isinstance(part, bytes) else (part or '')
                     for part in (expired.stdout, expired.stderr))


def run(command, cwd=ROOT, env=None, timeout=BUILD_TIMEOUT_S):
    return run_bounded(command, cwd=cwd, env=env, timeout=timeout)


def install_termination_handlers():
    """SIGTERM/SIGINT become `SystemExit(143/130)`: `finally` and `with` blocks (lock, restore) run."""
    def stop(signum, _frame):
        raise SystemExit(128 + signum)
    previous = {sig: signal.signal(sig, stop) for sig in (signal.SIGTERM, signal.SIGINT)}
    return previous


def tree_mb(path):
    path = Path(path)
    if path.is_file():
        return path.stat().st_size / 2 ** 20
    return sum(f.stat().st_size for f in path.rglob('*') if f.is_file()) / 2 ** 20


def artifact_reason(*paths):
    """`None` when everything the runner keeps under /tmp stays below the limit, else why not."""
    total = sum(tree_mb(p) for p in paths if Path(p).exists())
    return None if total <= ARTIFACT_LIMIT_MB else f'artifacts take {total:.0f} MB (limit {ARTIFACT_LIMIT_MB})'


def out(command, cwd=ROOT):
    """The text a TELEMETRY command prints (`pmset`, `sysctl`, `ps`, the toolchain versions): a missing answer is only
    recorded text. It ignores the exit code ON PURPOSE and must stay that way; an answer that decides something (the
    identity of the checkout, whether a file is HEAD's) goes through `git_strict`, which says whether it answered."""
    try:
        done = subprocess.run(command, cwd=cwd, capture_output=True, text=True, timeout=30)
        return done.stdout.strip()
    except (OSError, subprocess.TimeoutExpired) as error:
        return f'unavailable: {error}'


# The variables git ITSELF lists as repository-local (`git rev-parse --local-env-vars`, git 2.50.1). Each one tells git which
# repository, object store, index or configuration to use, and git then ignores the directory it was started in. Written out
# on purpose so that the list does not depend on the git that happens to be installed; the spec checks it against what the
# installed git prints. Not in it, and kept: HOME, PATH, the locale, GIT_AUTHOR_*/GIT_COMMITTER_* and the rest.
GIT_LOCAL_ENV = (
    'GIT_ALTERNATE_OBJECT_DIRECTORIES', 'GIT_CONFIG', 'GIT_CONFIG_PARAMETERS', 'GIT_CONFIG_COUNT', 'GIT_OBJECT_DIRECTORY',
    'GIT_DIR', 'GIT_WORK_TREE', 'GIT_IMPLICIT_WORK_TREE', 'GIT_GRAFT_FILE', 'GIT_INDEX_FILE', 'GIT_NO_REPLACE_OBJECTS',
    'GIT_REPLACE_REF_BASE', 'GIT_PREFIX', 'GIT_SHALLOW_FILE', 'GIT_COMMON_DIR')


def git_env(environ=None):
    """A COPY of `environ` (default: this process's) without `GIT_LOCAL_ENV`: the environment of every git a runner starts.

    A runner asks git about the checkout it was started in (`cwd=ROOT`). With `GIT_DIR` (or `GIT_WORK_TREE`, the index, the
    object directory...) exported by whoever launched the runner, git answers for the repository THAT names, whatever `cwd`
    says: `git_identity` then filed the clean HEAD of another repository while the build compiled this one, `git archive`
    packed another repository's baseline, and `git status` judged another tree. `os.environ` itself is left as it was."""
    environ = os.environ if environ is None else environ
    return {name: value for name, value in environ.items() if name not in GIT_LOCAL_ENV}


def git_strict(args, cwd=ROOT, timeout=30):
    """`(ok, text)` of `git ARGS`: `ok` is True only for a command that started, finished in time and exited 0, and then
    `text` is its stripped stdout (an empty one is a valid answer for `status`). Otherwise `ok` is False and `text` says
    why (the exit code and git's own words, a timeout, a command that cannot start): never an empty string that a caller
    could compare with another empty string. `out` above cannot tell the two apart, which is why a failing git used to
    read as `head ''`, `tree ''` and a CLEAN tree. The command runs under `git_env`: it answers for `cwd`, never for a
    repository the caller's environment names."""
    shown = 'git ' + ' '.join(args)
    try:
        done = subprocess.run(['git', *args], cwd=cwd, env=git_env(), capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return False, f'{shown}: no answer within {timeout}s'
    except OSError as error:
        return False, f'{shown}: cannot run git: {error}'
    if done.returncode != 0:
        return False, f'{shown}: exit {done.returncode}: {(done.stderr or done.stdout).strip()[:200]}'
    return True, done.stdout.strip()


def free_gib(path='/'):
    return round(shutil.disk_usage(path).free / 2 ** 30, 2)


def thermal_state(text):
    """What `pmset -g therm` can and cannot say. `verified` is True only when it reported numeric limits.

    On Apple silicon (the M3 Pro here) `pmset` prints only "No ... warning level has been recorded": that is NOT
    a statement that the CPU ran at full speed, so it is reported as `no_limit_reported` with `verified: False`
    and every manifest carries the limitation. Only a numeric limit below 100 blocks a measurement.
    """
    if text.startswith('unavailable'):
        return {'status': 'unavailable', 'verified': False, 'limits': {}, 'detail': text[:120]}
    limits = {name: int(value) for name, value in
              re.findall(r'(CPU_Speed_Limit|CPU_Scheduler_Limit)\s*=\s*(\d+)', text)}
    if any(value < 100 for value in limits.values()):
        return {'status': 'throttled', 'verified': True, 'limits': limits}
    if limits:
        return {'status': 'limits_reported_at_100', 'verified': True, 'limits': limits}
    return {'status': 'no_limit_reported', 'verified': False, 'limits': {},
            'detail': 'pmset reports no CPU_Speed_Limit on this hardware: throttling is not excluded'}


def claims(state):
    """Limitations of a machine state every manifest must carry (so a quiet-looking record is not read as proof)."""
    out = []
    if not state.get('thermal_state', {}).get('verified'):
        out.append('thermal throttling was NOT verified: ' + state.get('thermal_state', {}).get(
            'detail', 'pmset gave no numeric limit'))
    return out


def machine():
    """Hardware, load and thermal state, at this instant. Anything that cannot be read says so."""
    cpus = os.cpu_count() or 1
    therm = out(['pmset', '-g', 'therm'])
    return {
        'utc': utc(),
        'platform': platform.platform(),
        'machine': platform.machine(),
        'cpu': out(['sysctl', '-n', 'machdep.cpu.brand_string']),
        'cpus': cpus,
        'memory_bytes': out(['sysctl', '-n', 'hw.memsize']),
        'loadavg': os.getloadavg(),
        'thermal': therm,
        'thermal_state': thermal_state(therm),
        'power': out(['pmset', '-g', 'batt']).splitlines()[:2],
        'free_gib': free_gib(),
        'top_cpu_processes': out(['ps', '-Ao', 'pid,pcpu,comm', '-r']).splitlines()[:8],
    }


def guards(max_load_per_cpu=0.5):
    """`None` when the machine may be measured, else why not (recorded as BLOCKED, never waited out).

    The 1-minute AND the 5-minute load average must be under the limit (the 5-minute one covers the window of a
    measurement that just ran or is about to run); the 15-minute one is recorded only (it still remembers a build).
    """
    state = machine()
    cpus = state['cpus']
    limit = max_load_per_cpu * cpus
    for label, value in zip(('1 min', '5 min'), state['loadavg'][:2]):
        if value > limit:
            return state, f"load average ({label}) {value:.2f} over {limit:.1f} ({cpus} cpus)"
    thermal = state.get('thermal_state') or thermal_state(state.get('thermal', ''))
    if thermal['status'] == 'throttled':
        return state, f"thermal: {thermal['limits']}"
    if state['free_gib'] < FLOOR_FREE_GIB:
        return state, f"only {state['free_gib']} GiB free (floor {FLOOR_FREE_GIB})"
    return state, None


def judge_after(manifest):
    """The guards re-read AFTER a measurement. Records `machine_after`/`guard_after`; returns the reason it is not
    clean (the numbers are kept, but the run cannot be a PASS), or `None`."""
    state, reason = guards()
    manifest['machine_after'] = state
    manifest['guard_after'] = reason or 'clean'
    return reason


class Lock:
    """One non-blocking acquisition. `held` is False when somebody else has it (BLOCKED)."""

    def __init__(self):
        self.handle = open(LOCK, 'a')
        self.held = False
        try:
            fcntl.flock(self.handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.held = True
        except OSError:
            self.handle.close()

    def release(self):
        if self.held:
            fcntl.flock(self.handle, fcntl.LOCK_UN)
            self.handle.close()
            self.held = False

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.release()


@contextlib.contextmanager
def exclusive(path):
    """Yields True while this process holds an exclusive, non-blocking `flock` on the file `path` (created when absent) and
    False when somebody else has it: another process, or another `open()` of the same file in this one (an `flock` belongs
    to the open file description). It never waits. Leaving the block closes the file, which releases the lock; the file
    stays, because removing a lock file is how two runs end up each locking a different inode. Only a busy lock is
    reported as one: any other error (no `flock` on this file system, a read-only directory) propagates.

    It is what lets a runner own a directory (every runner takes `--out/OUT_LOCK`, through `guarded`); `Lock` above stays the
    performance lock and is not this."""
    with open(path, 'a') as handle:
        try:
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            yield False
        else:
            yield True


# What the identity's `dirty` looks at: every path a build of this checkout, or a verdict of a runner, reads from the working
# tree. Declared ONCE. `ab.is_prod`/`ab.is_measured` name the build inputs (the workspace manifest, the lockfile, the toolchain
# pin, the cargo config, the vendored crates and `crates/*`): all of them are here, and the spec checks that every tracked file
# `is_prod` accepts lies under one of these paths and that a change to each kind of build input makes `dirty` non-empty. The
# rest: `scripts` (the runners and the proof corpus the journey reads), `docs/migration/fixtures/gizmo-perf` (`budgets.json`,
# the journey, the corpus: a controlled run reads them from the tree at RUN time) and `runtime` (the page the native editor
# serves; `ab.is_measured` leaves it out on purpose, its executable being the core test, but `native.py` runs it;
# `runtime/dist` and `runtime/node_modules` are ignored by git, so building them leaves the tree clean). A pathspec that
# matches nothing in a checkout (no `vendor`, no `.cargo`) is not an error for `git status`. NOT covered, and recorded as such
# (BL015.23 item 29): the other compile-time inputs of the editor (`assets/demo/*.png` through `include_bytes!`, the unit
# tests' `include_str!` of `docs/migration/fixtures/bundles/*`).
DIRTY_PATHSPECS = ('crates', 'Cargo.toml', 'Cargo.lock', 'scripts', 'docs/migration/fixtures/gizmo-perf', '.cargo',
                   'rust-toolchain.toml', 'vendor', 'runtime')

IDENTITY_QUERIES = {
    'head': ['rev-parse', 'HEAD'],
    'tree': ['rev-parse', 'HEAD^{tree}'],
    'branch': ['rev-parse', '--abbrev-ref', 'HEAD'],
    'dirty': ['status', '--porcelain', '--', *DIRTY_PATHSPECS],
}


def git_identity(root=ROOT):
    """HEAD, tree, branch and the dirtiness of `root`, each from a STRICT git call (`git_strict`).

    When git answers every question, the dict is `head`, `tree`, `branch`, `dirty` (empty = clean) and
    `unavailable: False`. When ANY question fails, `unavailable` is True, `failed` maps the field to the failure text,
    and each failed field holds `unavailable: <failure text>`: a failed `dirty` query is a non-empty text, so it can never
    read as a clean tree, and two failed identities are not equal unless they failed the same way (callers compare
    through `same_head`, which refuses a failed one anyway). Every runner refuses an unavailable identity before it does
    anything (`identity_problem`), `--allow-dirty` included: it is not a dirty tree, it is a tree nobody could name."""
    identity, failed = {}, {}
    for field, args in IDENTITY_QUERIES.items():
        ok, text = git_strict(args, root)
        if ok and field in ('head', 'tree') and not text:
            ok, text = False, 'git ' + ' '.join(args) + ': printed nothing'
        if ok:
            identity[field] = text
        else:
            failed[field] = text
            identity[field] = f'unavailable: {text}'
    identity['unavailable'] = bool(failed)
    if failed:
        identity['failed'] = failed
    return identity


def identity_problem(identity):
    """`None` when `identity` is a real answer of git, else the sentence of the REFUSED record. A dict that does not
    carry the flag (a record read from disk, a hand-made one) is judged by its `head` and `tree`: empty, not a string, or a
    failure text is no answer either."""
    if identity.get('unavailable'):
        detail = '; '.join(f'{field}: {why}' for field, why in (identity.get('failed') or {}).items()) or 'no detail'
        return (f'git could not name this checkout ({detail}): no number could be filed under a HEAD, so nothing runs '
                '(`--allow-dirty` does not change that: it is not a dirty tree, it is a tree nobody could name)')
    for field in ('head', 'tree'):
        value = identity.get(field)
        if not isinstance(value, str) or not value or value.startswith('unavailable'):
            return f'the identity has no usable {field} ({value!r}): it names no commit'
    return None


def same_head(first, second):
    """True only when BOTH identities are real answers (`identity_problem`) and name the same commit AND tree. Two
    failed answers (both `''`, or both the same failure text) are NOT 'the same HEAD': comparing them with `==` is how an
    unavailable identity used to match another one."""
    return (identity_problem(first) is None and identity_problem(second) is None
            and first['head'] == second['head'] and first['tree'] == second['tree'])


PERF_ENV_PREFIX = 'TESSEL_GIZMO_PERF_'     # the budget executable's own switches (OUT, CALIBRATE, BLESS)
BUILD_ENV = ('CARGO_TARGET_DIR', 'TESSEL_CARGO_LANE')   # where a build puts its artifact: a runner's choice, not the caller's


def dropped_env(build=False, environ=None):
    """The names (sorted) of the inherited variables `child_env` takes out of a child's environment."""
    environ = os.environ if environ is None else environ
    return sorted(name for name in environ if name.startswith(PERF_ENV_PREFIX) or (build and name in BUILD_ENV))


def child_env(extra=None, build=False, environ=None):
    """The environment of a child that MEASURES or BUILDS: the runner's own WITHOUT every inherited `TESSEL_GIZMO_PERF_*`
    variable (and, for `build=True`, without `CARGO_TARGET_DIR` and `TESSEL_CARGO_LANE`), plus exactly `extra`, what the
    runner means to set. An exported `TESSEL_GIZMO_PERF_CALIBRATE=1` used to turn a measured run into a calibration
    (no budget asserted) while `verdict` believed it had judged; an exported `TESSEL_GIZMO_PERF_BLESS=1` makes the
    structural test rewrite its golden file and pass without asserting; a target directory in the environment makes two
    builds share one artifact. The runner records `extra` as the child's `env` and `dropped_env` beside it, so what was
    taken away is on the record too."""
    environ = os.environ if environ is None else environ
    dropped = set(dropped_env(build, environ))
    env = {name: value for name, value in environ.items() if name not in dropped}
    env.update(extra or {})
    return env


@contextlib.contextmanager
def scoped_environ(env):
    """`os.environ` holds exactly `env` inside the block and what it held before outside it. For code that copies
    `os.environ` when it builds a child (`proof_surface.Native`) and cannot take an environment as an argument."""
    saved = dict(os.environ)
    os.environ.clear()
    os.environ.update(env)
    try:
        yield
    finally:
        os.environ.clear()
        os.environ.update(saved)


CHECKOUT_LOCK_DIR = '/tmp'


def checkout_lock_path(root=ROOT):
    """The lock of one CHECKOUT: a file under /tmp (never inside the repository, whose tree must stay clean) named by a
    hash of the checkout's resolved path, so two worktrees never wait on each other and one checkout always agrees."""
    key = hashlib.sha256(str(Path(root).resolve()).encode()).hexdigest()[:16]
    return Path(CHECKOUT_LOCK_DIR) / f'tessel-gizmo-perf-checkout-{key}.lock'


# The lock of `--out`: ONE name for every runner (`run.py`, `ab.py`, `native.py`, `mutate.py`) and every phase. Rounds 4 to 7
# gave each runner a lock file of its own name, so `mutate.py --phase prepare` (which empties `--out/exes`) and a
# `run.py --phase measure` on the same directory did not exclude each other. One name is what makes "one run per `--out`" true.
OUT_LOCK = '.gizmo-perf.lock'


@contextlib.contextmanager
def guarded(out, who, builds=False, root=ROOT):
    """What a runner must own before it reads or writes anything. Yields `None` once it owns it and the REFUSED record
    (print it, exit 1) when it does not: nothing here waits, and a refusal writes nothing but the lock file itself. `who` is
    the runner that asks (the one that would be refused); the holder of a lock it cannot take can be ANY runner.

    1. `builds=True` (a phase that BUILDS from the checkout, or edits it: `mutate.py prepare`): the lock of the CHECKOUT
       (`checkout_lock_path`), taken before `out` exists. A build that runs while another run has a mutant in the tree
       builds the mutant, and two `prepare` runs with different `--out` would each put a mutant in the same files.
    2. `out/OUT_LOCK`: one run per `--out`, for every runner and every phase. Two runs on one directory rebuild or delete what
       the other is measuring (`mutate.py prepare` empties `exes/` that `run.py measure` runs from), and a stale report would
       stand in for a new one. The lock excludes SIMULTANEOUS runs, not SEQUENTIAL ones: a second runner that comes after the
       first has finished still overwrites its `manifest.json` and `exes/`, so one `--out` per runner stays the rule.

    Both are `flock`s released when the block ends (also on SIGTERM/SIGINT, which are `SystemExit`) or the process dies;
    the files stay (see `exclusive`). The checkout lock is taken first, then the one of `out`, in every runner, and neither
    waits, so no run can wait on another. A runner takes this lock ONCE, in its `main`, and calls the others' functions
    (`mutate.py` calls `run.build()`) as plain functions, never through `main`: a shared name cannot conflict with itself."""
    out = Path(out)
    with contextlib.ExitStack() as stack:
        if builds:
            path = checkout_lock_path(root)
            if not stack.enter_context(exclusive(path)):
                yield {'status': 'REFUSED', 'reason': (
                    f'another run holds the checkout lock {path}: it builds from, or edits, this checkout ({root}), and a '
                    'build started now would build whatever that run has in the tree; nothing was read or written '
                    '(not waited for)')}
                return
        out.mkdir(parents=True, exist_ok=True)
        path = out / OUT_LOCK
        if not stack.enter_context(exclusive(path)):
            yield {'status': 'REFUSED', 'reason': (
                f'another gizmo-perf run (any of run.py, ab.py, native.py or mutate.py, in any phase) holds {path}: it owns '
                f'this --out, so `{who}` read and wrote nothing here (not waited for); use one --out per runner')}
            return
        yield None


def toolchain():
    return {'rustc': out(['rustc', '--version']), 'cargo': out(['cargo', '--version'])}


def write(path, payload):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, 'w') as handle:
        json.dump(payload, handle, indent=1, sort_keys=True)
        handle.write('\n')
        handle.flush()
        os.fsync(handle.fileno())
    return path


def die(code, report, path):
    if path:
        write(path, report)
    print(json.dumps({k: report[k] for k in ('status', 'reason') if k in report}))
    return code
