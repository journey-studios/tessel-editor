"""Shared pieces of the gizmo performance runners (no suite logic): the performance lock, the
resource guards, the environment record and the manifest writer.

The lock is the repository's performance lock (`/tmp/tessel-scene-gpu.lock`, AGENTS.md). It is taken ONCE,
without waiting (`LOCK_NB`): if somebody holds it the runner records BLOCKED and stops. No loop, no sleep, no new
wait, no kill, no bypass. Builds and functional tests never take it.

Every child process runs through `run_bounded`: a FINITE timeout, its own process group, and on expiry the whole
group is terminated (then killed) and the partial output is kept. A runner that holds the lock therefore cannot be
held forever by a hung test. `install_termination_handlers` turns SIGTERM/SIGINT into `SystemExit`, so the `finally`
blocks and `with` statements (lock release, restoring a mutated checkout) run when a runner is asked to stop.
"""
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
    try:
        done = subprocess.run(command, cwd=cwd, capture_output=True, text=True, timeout=30)
        return done.stdout.strip()
    except (OSError, subprocess.TimeoutExpired) as error:
        return f'unavailable: {error}'


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


def git_identity(root=ROOT):
    return {
        'head': out(['git', 'rev-parse', 'HEAD'], root),
        'tree': out(['git', 'rev-parse', 'HEAD^{tree}'], root),
        'branch': out(['git', 'rev-parse', '--abbrev-ref', 'HEAD'], root),
        'dirty': out(['git', 'status', '--porcelain', '--', 'crates', 'Cargo.toml', 'Cargo.lock', 'scripts', 'docs/migration/fixtures/gizmo-perf'], root),
    }


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
