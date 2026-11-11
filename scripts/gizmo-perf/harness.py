"""Shared pieces of the gizmo performance runners (no suite logic): the performance lock, the
resource guards, the environment record and the manifest writer.

The lock is the repository's performance lock (`/tmp/tessel-scene-gpu.lock`, AGENTS.md). It is taken ONCE,
without waiting (`LOCK_NB`): if somebody holds it the runner records BLOCKED and stops. No loop, no sleep, no new
wait, no kill, no bypass. Builds and functional tests never take it.
"""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
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


def run(command, cwd=ROOT, env=None, timeout=None):
    done = subprocess.run(command, cwd=cwd, env=env, capture_output=True, text=True, timeout=timeout)
    return done


def out(command, cwd=ROOT):
    try:
        done = subprocess.run(command, cwd=cwd, capture_output=True, text=True, timeout=30)
        return done.stdout.strip()
    except (OSError, subprocess.TimeoutExpired) as error:
        return f'unavailable: {error}'


def free_gib(path='/'):
    return round(shutil.disk_usage(path).free / 2 ** 30, 2)


def machine():
    """Hardware, load and thermal state, at this instant. Anything that cannot be read says so."""
    cpus = os.cpu_count() or 1
    return {
        'utc': utc(),
        'platform': platform.platform(),
        'machine': platform.machine(),
        'cpu': out(['sysctl', '-n', 'machdep.cpu.brand_string']),
        'cpus': cpus,
        'memory_bytes': out(['sysctl', '-n', 'hw.memsize']),
        'loadavg': os.getloadavg(),
        'thermal': out(['pmset', '-g', 'therm']),
        'power': out(['pmset', '-g', 'batt']).splitlines()[:2],
        'free_gib': free_gib(),
        'top_cpu_processes': out(['ps', '-Ao', 'pid,pcpu,comm', '-r']).splitlines()[:8],
    }


def guards(max_load_per_cpu=0.5):
    """`None` when the machine may be measured, else why not (recorded as BLOCKED, never waited out)."""
    state = machine()
    cpus = state['cpus']
    if state['loadavg'][0] > max_load_per_cpu * cpus:
        return state, f"load average {state['loadavg'][0]:.2f} over {max_load_per_cpu * cpus:.1f} ({cpus} cpus)"
    limit = None
    for line in state['thermal'].splitlines():
        if 'CPU_Speed_Limit' in line:
            limit = int(line.split('=')[-1])
    if limit is not None and limit < 100:
        return state, f'thermal: CPU_Speed_Limit {limit}'
    if state['free_gib'] < FLOOR_FREE_GIB:
        return state, f"only {state['free_gib']} GiB free (floor {FLOOR_FREE_GIB})"
    return state, None


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
