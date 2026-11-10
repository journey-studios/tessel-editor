#!/usr/bin/env python3
"""F055 / BL015.18 — cost of the scale overlay in the core, measured once and without waiting for the lock.

    python3 scripts/scene-gizmo-scale-measure.py [--headless target/debug/tessel-headless] [--n 1500] [--out m.json]

This is a PERFORMANCE measurement, so it takes the shared GPU/bench lock (`/tmp/tessel-scene-gpu.lock`, the one
`scripts/directed-scene-proof/with-gpu-lock.py` takes) with `LOCK_NB`: if somebody else holds it, the script writes
`{"measured": false, "reason": "lock_busy"}` and exits 3 WITHOUT waiting, deleting the lock or touching the holder.
The functional proofs never take the lock.

What it measures: the round trip of `scene.gizmo.state` on `tessel-headless` (JSON Lines over a pipe, so it includes
the encoding and the pipe, the same on every row) for a node that is hidden (nothing selected), has the translate
arrows, the rotate rings or the scale boxes. The rows are interleaved round-robin so that machine noise hits all of
them; the figures are the median, p95 and minimum of N calls per row, in microseconds, repeated over three rounds.

Limits, on purpose: a DEBUG binary; no window, no WebView and no page rasterization; a shared machine. A rotation
figure is not a scale figure and neither proves the page's cost. The report keeps every round, never "cost zero".
"""
import argparse
import fcntl
import importlib.util
import json
from pathlib import Path
import statistics
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
LOCK = '/tmp/tessel-scene-gpu.lock'


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / file)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def percentile(values, q):
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(q * len(ordered)))]


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument('--headless', default=str(ROOT / 'target/debug/tessel-headless'))
    parser.add_argument('--n', type=int, default=1500)
    parser.add_argument('--out')
    args = parser.parse_args(argv)
    report = {'measured': False, 'binary': args.headless, 'n': args.n}

    def finish(code):
        if args.out:
            Path(args.out).write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps(report, indent=2))
        return code

    lock = open(LOCK, 'a')
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        report['reason'] = 'lock_busy'
        return finish(3)
    s8 = load('scene_gizmo_proof', 'scene-gizmo-proof.py')
    work = tempfile.mkdtemp(prefix='te-g12-perf-', dir='/tmp')
    surface = s8.Headless(args.headless, work)
    try:
        def ok(request):
            good, payload = surface.op(request)
            if not good:
                raise RuntimeError(f'{request} -> {payload}')
            return payload

        ok({'op': 'scene.attach', 'id': s8.SCENE, 'path': str(s8.BUNDLE)})
        node = 'scene/scene-a/node/mid'
        rows = {
            'hidden': (None, None),
            'translate': (node, 'translate'),
            'rotate': (node, 'rotate'),
            'scale': (node, 'scale'),
        }
        rounds = []
        for _ in range(3):
            samples = {name: [] for name in rows}
            for _ in range(args.n):
                for name, (entity, tool) in rows.items():
                    ok({'op': 'scene.select', 'scene': s8.SCENE, 'entities': [entity] if entity else []})
                    ok({'op': 'scene.gizmo.tool', 'tool': tool or 'translate', 'space': 'local'})
                    started = time.perf_counter()
                    ok({'op': 'scene.gizmo.state', 'scene': s8.SCENE, 'frame': s8.FRAME})
                    samples[name].append((time.perf_counter() - started) * 1e6)
            rounds.append({name: {'p50_us': round(statistics.median(v), 1), 'p95_us': round(percentile(v, 0.95), 1),
                                  'min_us': round(min(v), 1), 'n': len(v)} for name, v in samples.items()})
        report.update({'measured': True, 'rounds': rounds, 'unit': 'microseconds per scene.gizmo.state round trip'})
    finally:
        s8.stop_surface(surface) if hasattr(s8, 'stop_surface') else None
        import shutil
        shutil.rmtree(work, ignore_errors=True)
        fcntl.flock(lock, fcntl.LOCK_UN)
    return finish(0)


if __name__ == '__main__':
    raise SystemExit(main())
