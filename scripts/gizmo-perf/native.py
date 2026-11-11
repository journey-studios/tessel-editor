#!/usr/bin/env python3
"""The shared gizmo journey in the REAL window (WKWebView), with its own editor instance (F055 / BL015.21).

    python3 scripts/gizmo-perf/native.py --binary target/release/tessel-editor [--out DIR] [--corpora small dense deep]

What it does. For each corpus one FRESH editor process (own HOME and socket, the HEAD release binary, the repository's
runtime page) is started and the SAME `docs/migration/fixtures/gizmo-perf/journey.json` the core and the GPUI host run
is replayed through the window's socket with semantic requests only (`op`, `get`, `set`, `invoke`): no coordinate
clicks, no key events, the user's own editor is never addressed. It then checks, per step, that the stored transform and
the undo/redo depths equal the golden trace the core produced (`journey.expected.json`), and, in the picture the
window itself drew, that a committed gesture changed it, that its undo restored the exact picture hash and that a
cancelled gesture left it unchanged (state, history AND pixels, not just time).

What it measures (all raw samples are kept, nothing is trimmed):
  * `client_ms`: wall time of each request as the client saw it (socket round trip: core + host + IPC; NOT the host cost
    alone, and not a presentation);
  * the editor's own report of each picture the runtime drew and READ BACK (`monitor.viewport` `last_frame`:
    `render_ms`, `total_ms`, `prepare_ms`, `draw_ms`, `flush_ms` and the pixel hash, one entry per request) and the
    session trace spans of the host (`TESSEL_VIEWPORT_TRACE=1`: plan, plan_key, evaluate, ipc, serve, dispatch, with
    their percentiles). Directed-scene pictures are NOT stamped by the session trace (`frames_total` stays 0), so
    `presented_after_done_ms`/`submit_to_presented_ms` do not exist for them: said in `limits`, not invented;
  * the cold open: from `monitor.show_scene` to the first converged picture of the journey's frame, per corpus, in a
    process that has never drawn anything (the first frame is NOT excluded);
  * the overlay's own acknowledgement from the page (`TESSEL_VIEWPORT_OVERLAY_ACK=1`): drawn handles after a state.

What it does NOT prove, said again in the report (`limits`): the compositor/GPU scan-out is not observable from here;
the OS capture (`screencapture -l <windowid>`, observational, one per corpus) is only a witness that a window existed
with something painted; no physical pointer or keyboard is used (BL015.2/BL043/BL044 stay pending); a socket reply or
the page's `frame.done` is NOT counted as proof of painting. Nothing here is budgeted: the numbers are observations of
one machine and one backend, with the load and thermal state recorded; a budget needs the same A/B discipline as the
core suite and a baseline native binary, which is the open part of BL015.21.

Performance lock: taken ONCE, without waiting, for the whole run; busy or a failed guard (load, thermal, disk below
the floor) is BLOCKED (exit 3). Exit 0 only when every functional check passed; 1 on a failure; 3 BLOCKED.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import socket
import statistics
import subprocess
import sys
import tempfile
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent))
import harness  # noqa: E402
import proof_surface  # noqa: E402

ROOT = harness.ROOT
FIXTURES = ROOT / 'docs/migration/fixtures/gizmo-perf'
TOLERANCE = 1e-9
LIMITS = [
    "the compositor / GPU scan-out is not observable; directed-scene pictures carry no presented-frame stamp (the session trace's frames_total is 0), so core->host->runtime is measured, the last hop is not",
    "a socket reply and the page's frame.done are not proof of painting; the picture hash is the runtime read-back",
    "the OS capture is observational (one window witness per corpus), never an assertion",
    "no physical pointer, keyboard or popover focus is used: BL015.2, BL043, BL044 remain pending",
    "client_ms includes the socket round trip and the client process: it is not the host cost",
    "observations of one machine/backend; nothing is budgeted and no A/B against a baseline native binary exists yet",
]


def near(a, b):
    if isinstance(a, bool) or isinstance(b, bool):
        return type(a) is type(b) and a == b
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        return abs(a - b) <= TOLERANCE
    if isinstance(a, list) and isinstance(b, list):
        return len(a) == len(b) and all(near(x, y) for x, y in zip(a, b))
    if isinstance(a, dict) and isinstance(b, dict):
        return a.keys() == b.keys() and all(near(a[k], b[k]) for k in a)
    return a == b


def fill(value, variables):
    if isinstance(value, str) and value.startswith('$'):
        return variables[value[1:]]
    if isinstance(value, list):
        return [fill(item, variables) for item in value]
    if isinstance(value, dict):
        return {key: fill(item, variables) for key, item in value.items()}
    return value


def pct(values, q):
    ordered = sorted(values)
    return ordered[max(0, min(len(ordered) - 1, int(-(-q * len(ordered) // 1)) - 1))] if ordered else None


def summary(values):
    return {'n': len(values), 'min': min(values), 'p50': pct(values, 0.5), 'p95': pct(values, 0.95),
            'p99': pct(values, 0.99), 'max': max(values)} if values else {'n': 0}


class Window:
    """One editor process, driven through its socket the way `./scripts/ui` does, with a fast client."""

    def __init__(self, binary, home):
        os.environ.update({'TESSEL_VIEWPORT_TRACE': '1', 'TESSEL_VIEWPORT_INSPECT': '1',
                           'TESSEL_VIEWPORT_OVERLAY_ACK': '1', 'TESSEL_RUNTIME_DIR': str(ROOT / 'runtime')})
        self.native = proof_surface.Native(binary, home)
        self.socket = self.native.socket
        self.frames = {}

    def send(self, request, timeout=90):
        with socket.socket(socket.AF_UNIX) as conn:
            conn.settimeout(timeout)
            conn.connect(self.socket)
            conn.sendall((json.dumps({'version': 1, **request}) + '\n').encode())
            data = b''
            while not data.endswith(b'\n'):
                chunk = conn.recv(1 << 20)
                if not chunk:
                    break
                data += chunk
        return json.loads(data)

    def op(self, request, timeout=90):
        answer = self.send({'command': 'op', 'request': request}, timeout)
        return answer['ok'], (answer.get('data') if answer['ok'] else answer.get('error'))

    def viewport(self):
        return self.send({'command': 'get', 'target': 'monitor.viewport'})['data']['state']

    def trace(self):
        """Polls what the editor itself reports: the picture the runtime last drew and read back (`last_frame`: its
        own render/total/prepare/draw/flush milliseconds and the pixel hash, one entry per request, never trimmed)
        and the session trace (host spans plan/evaluate/ipc/serve with their percentiles; `frames_total` counts only
        the pictures the session stamps, which directed-scene pictures are not)."""
        state = self.viewport()
        last = state.get('last_frame') or {}
        if last.get('request') is not None:
            self.frames[last['request']] = {key: last.get(key) for key in (
                'request', 'revision', 'hash', 'inspected', 'render_ms', 'total_ms', 'prepare_ms', 'draw_ms',
                'flush_ms', 'width', 'height')}
        ok, data = self.op({'op': 'render.status'})
        return (data or {}).get('report', data) or {}

    def converged(self, scene, frame):
        state = self.viewport()
        directed = state.get('directed') or {}
        shown = directed.get('shown') or {}
        return (state.get('phase') == 'ready' and directed.get('scene') == scene and directed.get('active') is None
                and directed.get('queued') is None and not directed.get('suspended')
                and (frame is None or shown.get('edit_frame') == frame) and bool(state.get('last_frame'))
                and not str(state.get('headline', '')).startswith('Falha')), state

    def settle(self, scene, frame, seconds=30):
        """Seconds (client clock) until the monitor drew the picture of `frame` and nothing is in flight; None on timeout."""
        started = time.perf_counter()
        deadline = started + seconds
        state = None
        while time.perf_counter() < deadline:
            done, state = self.converged(scene, frame)
            if done:
                return (time.perf_counter() - started) * 1000, state
            time.sleep(0.01)
        return None, state

    def stop(self):
        return self.native.stop()


def transform(window, scene, node):
    out = []
    for prop in ('position', 'rotation', 'scale'):
        ok, data = window.op({'op': 'scene.get', 'scene': scene, 'property': f'{node}/{prop}'})
        assert ok, (prop, data)
        out.append(data['value'])
    return out


def depths(window):
    ok, data = window.op({'op': 'history.status'})
    assert ok, data
    pick = lambda key: data.get(key, (data.get('history') or {}).get(key))  # noqa: E731
    return pick('undo_depth'), pick('redo_depth')


def picture_hash(state):
    return (state.get('last_frame') or {}).get('hash')


def run_corpus(binary, journey, golden, name, out, notes):
    corpus = journey['corpora'][name]
    base = str(ROOT / journey['base'])
    variables = {'scene': journey['scene'], 'scene_b': journey['scene_b'], 'frame': journey['frame'],
                 'bundle': str(ROOT / corpus['bundle']), 'bundle_b': str(ROOT / corpus['bundle_b']), 'base': base,
                 'node': corpus['node'], 'node_b': corpus['node_b'], 'other': corpus['other']}
    home = tempfile.mkdtemp(prefix='gp-', dir='/tmp')
    window = None
    result = {'corpus': name, 'checks': [], 'samples': [], 'steps': {}, 'pictures': [], 'overlay_drawn': [],
              'captures': []}

    def check(label, condition, detail=''):
        result['checks'].append({'check': label, 'ok': bool(condition), 'detail': str(detail)[:300]})
        return bool(condition)

    try:
        window = Window(binary, home)
        window.native.prepare()
        result['pid'] = window.native.process.pid
        scene = journey['scene']
        node = corpus['node']
        committed, undone = [], []
        hashes = []  # picture hashes before each committed gesture, for the exact-undo pixel check

        def timed(step, request, timeout=120):
            started = time.perf_counter()
            ok, data = window.op(request, timeout)
            result['samples'].append({'step': step, 'op': request['op'], 'client_ms': (time.perf_counter() - started) * 1000,
                                      'ok': ok, 'code': None if ok else (data or {}).get('code')})
            return ok, data

        def commit_job(step, data):
            if not isinstance(data, dict) or data.get('state') != 'queued':
                return True
            deadline = time.time() + 60
            while time.time() < deadline:
                ok, status = window.op({'op': 'job.get', 'job': data['job']})
                if ok and status['state'] in ('succeeded', 'failed', 'cancelled'):
                    break
                time.sleep(0.03)
            else:
                return check(f'{step}: the job finished', False)
            if status['state'] != 'succeeded':
                return check(f'{step}: the job succeeded', False, status)
            ok, commit = window.op({'op': 'job.commit', 'job': data['job']})
            return check(f'{step}: the job was applied', ok or (commit or {}).get('code') == 'job_finished', commit)

        for raw in journey['steps']:
            step = fill(raw, variables)
            sid = step['id']
            if 'surfaces' in step and 'native' not in step['surfaces']:
                result['steps'][sid] = {'skipped': f"listed for {step['surfaces']}"}
                continue
            if sid == 'switch.attach':
                scene = journey['scene_b']
                node = corpus['node_b']
            record = result['steps'].setdefault(sid, {})
            if step.get('kind') == 'drag':
                drag(window, timed, check, step, scene, variables, node, committed, hashes, result)
            elif step.get('ui'):
                words = step['ui']
                if words[:3] == ['set', 'window', 'size']:
                    answer = window.send({'command': 'set', 'target': 'window', 'field': 'size', 'value': words[3]})
                else:
                    answer = window.send({'command': 'invoke', 'target': words[1]})
                check(f'{sid}: the window accepted `ui {" ".join(words)}`', answer.get('ok'), answer)
                if sid.startswith('present.resize'):
                    ms, state = window.settle(scene, journey['frame'])
                    record['settle_ms'] = ms
                    check(f'{sid}: the monitor converged after the resize', ms is not None, json.dumps(state)[:260])
                if sid == 'present.dialog.open':
                    time.sleep(0.3)
                    record['page_hidden_by_dialog'] = (window.viewport().get('on_screen') is False
                                                       or window.viewport().get('hidden'))
            elif step.get('wait') == 'presented':
                ms, state = window.settle(scene, journey['frame'])
                record['client_ms_to_first_picture'] = ms
                check(f'{sid}: the first picture of frame {journey["frame"]} was drawn', ms is not None)
                if ms is not None:
                    window.trace()
                    result['first_picture_hash'] = picture_hash(state)
                    overlay = state.get('overlay') or {}
                    check(f'{sid}: the page announced the overlay', overlay.get('supported') is True, overlay)
                    path = Path(out) / f'{name}.open.png'
                    shot, why = window.native.capture(path)
                    result['captures'].append({'at': sid, 'path': shot, 'why': why or '',
                                               'sha256': harness.sha256(shot) if shot else None,
                                               'observational_only': True})
            else:
                if step.get('tool'):
                    timed(sid, {'op': 'scene.gizmo.tool', **step['tool']})
                request = step['op']
                before_hash = picture_hash(window.viewport()) if request['op'] in ('history.undo', 'history.redo') else None
                ok, data = timed(sid, request)
                check(f'{sid}: {request["op"]} answered', ok, data)
                if request['op'] == 'scene.resources.resolve' and ok:
                    commit_job(sid, data)
                if request['op'] == 'monitor.show_scene' and ok:
                    ms, state = window.settle(scene, None if sid == 'open.show' else journey['frame'], 20)
                    record['client_ms_to_converged_picture'] = ms
                    check(f'{sid}: the shown scene drew its picture', ms is not None, json.dumps(state)[:260])
                if request['op'] == 'scene.gizmo.state' and ok:
                    overlay = data['overlay']
                    drawn = None
                    if overlay.get('visible'):
                        deadline = time.time() + 5
                        while time.time() < deadline and not drawn:
                            drawn = (window.viewport().get('overlay') or {}).get('drawn')
                            time.sleep(0.02)
                        result['overlay_drawn'].append({'step': sid, 'drawn': drawn})
                if request['op'] in ('history.undo', 'history.redo') and ok:
                    ms, state = window.settle(scene, journey['frame'])
                    record['client_ms_to_converged_picture'] = ms
                    if request['op'] == 'history.undo' and committed:
                        if before_hash is not None and hashes:
                            check(f'{sid}: the undo restored the exact picture of before the gesture',
                                  picture_hash(state) == hashes[-1], (picture_hash(state), hashes[-1]))
                            undone.append(hashes.pop())
                    elif request['op'] == 'history.redo' and undone:
                        hashes.append(undone.pop())
            if sid == 'open.attach' or sid == 'switch.attach':
                result['attached'] = True
            if result.get('attached') and not step.get('ui') and step.get('wait') is None and sid != 'open.project':
                got = {'transform': transform(window, scene, node), 'undo': depths(window)[0],
                       'redo': depths(window)[1]}
                record['state'] = got
                want = golden[name].get(sid)
                if want is not None:
                    check(f'{sid}: transform/undo/redo equal the core golden trace',
                          near(got['transform'], want['transform']) and got['undo'] == want['undo']
                          and got['redo'] == want['redo'], (got, want))
            window.trace()
        report = window.trace()
        result['frames_seen'] = len(window.frames)
        result['pictures'] = list(window.frames.values())
        trace = (report.get('session') or {}).get('trace') or {}
        result['trace'] = {'enabled': trace.get('enabled'), 'frames_total': trace.get('frames_total'),
                           'dropped': trace.get('dropped'), 'spans_ms': trace.get('spans')}
        result['overlay_report'] = (window.viewport().get('overlay') or {})
        result['session_error'] = ((report.get('session') or {}).get('error'))
        check('the session reported no error', not result['session_error'], result['session_error'])
    except (proof_surface.SurfaceError, OSError, AssertionError, KeyError, ValueError) as error:
        check('the journey ran to the end', False, repr(error))
    finally:
        if window is not None:
            result['cleanup_errors'] = window.stop()
        shutil.rmtree(home, ignore_errors=True)
    return result


def drag(window, timed, check, step, scene, variables, node, committed, hashes, result):
    sid = step['id']
    frame = variables['frame']
    timed(sid, {'op': 'scene.gizmo.tool', 'tool': step['tool'], 'space': step['space']})
    before = transform(window, scene, node)
    undo_before = depths(window)[0]
    ms, state = window.settle(scene, frame)
    start_hash = picture_hash(state) if state else None
    ok, shown = timed(sid, {'op': 'scene.gizmo.state', 'scene': scene, 'frame': frame})
    if not check(f'{sid}: gizmo state', ok, shown):
        return
    handle = next(h for h in shown['overlay']['handles'] if h['id'] == step['handle'])
    ring = []
    if handle.get('points'):
        points = handle['points']
        ring = [p for p in points[len(points) // 8:] if p]
        start = ring[0]
    else:
        start = handle['to']
    ok, down = timed(sid, {'op': 'scene.pointer.down', 'scene': scene, 'frame': frame, 'point': start})
    if not (check(f'{sid}: the press landed on the handle', ok and down.get('dragging') is True, down)
            and check(f'{sid}: the hit handle is `{step["handle"]}`', down['hit']['handle'] == step['handle'], down.get('hit'))):
        return
    moves = step['moves']
    delta = step.get('delta') or [0, 0]
    applied = 0
    for index in range(1, moves + 1):
        if ring:
            point = ring[min(index, len(ring) - 1)]
        else:
            point = {'u': start['u'] + delta[0] * index / moves, 'v': start['v'] + delta[1] * index / moves}
        ok, moved = timed(sid, {'op': 'scene.pointer.move', 'point': point})
        check(f'{sid}: move {index} answered and stayed a candidate', ok and moved.get('document_changed') is False, moved)
        applied += 1 if ok and moved.get('applied') is True else 0
    check(f'{sid}: a move was applied', applied > 0)
    window.trace()
    check(f'{sid}: the candidate is not in the document before the release', near(transform(window, scene, node), before))
    if step['end'] == 'up':
        ok, up = timed(sid, {'op': 'scene.pointer.up'})
        check(f'{sid}: the release committed', ok and up.get('document_changed') is True, up)
        ms, state = window.settle(scene, frame)
        result['steps'][sid]['client_ms_up_to_converged_picture'] = ms
        check(f'{sid}: the committed picture was drawn', ms is not None)
        after_hash = picture_hash(state) if state else None
        check(f'{sid}: the committed picture differs from the one before the gesture',
              after_hash is not None and after_hash != start_hash, (start_hash, after_hash))
        check(f'{sid}: one undo entry per gesture', depths(window)[0] == undo_before + 1)
        committed.append((before, transform(window, scene, node)))
        hashes.append(start_hash)
    else:
        ok, cancel = timed(sid, {'op': 'scene.pointer.cancel', 'reason': 'api'})
        check(f'{sid}: the cancel answered', ok, cancel)
        ms, state = window.settle(scene, frame)
        check(f'{sid}: after the cancel the picture is the one before the gesture',
              state is not None and picture_hash(state) == start_hash, (start_hash, picture_hash(state) if state else None))
        check(f'{sid}: a cancel changes nothing and adds no undo entry',
              near(transform(window, scene, node), before) and depths(window)[0] == undo_before)


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', default=str(ROOT / 'target/release/tessel-editor'))
    parser.add_argument('--out', default='/tmp/gizmo-perf-native')
    parser.add_argument('--corpora', nargs='*', default=['small', 'dense', 'deep'])
    parser.add_argument('--functional-only', action='store_true',
                        help='run the journey and its functional checks WITHOUT the performance lock and the load '
                             'guard; the timings of such a run are not a measurement and the status says so')
    args = parser.parse_args(argv)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    manifest_path = out / 'manifest.json'
    manifest = {'suite': 'gizmo-perf-native', 'utc_start': harness.utc(), 'cwd': str(ROOT), 'limits': LIMITS,
                'identity': harness.git_identity(), 'toolchain': harness.toolchain(), 'binary': args.binary,
                'status': 'ABORTED', 'command': sys.argv}
    if not Path(args.binary).exists() or not (ROOT / 'runtime/dist/runtime.js').exists():
        manifest.update(status='PREREQUISITE_MISSING',
                        reason='needs the release editor and `./scripts/runtime setup` (runtime/dist)')
        return harness.die(1, manifest, manifest_path)
    manifest['binary_sha256'] = harness.sha256(args.binary)
    manifest['runtime_sha256'] = harness.sha256(ROOT / 'runtime/dist/runtime.js')
    journey = json.loads((FIXTURES / 'journey.json').read_text())
    golden = json.loads((FIXTURES / 'journey.expected.json').read_text())
    manifest['corpus'] = json.loads((FIXTURES / 'corpus.manifest.json').read_text())
    manifest['measurement'] = not args.functional_only
    state, reason = harness.guards()
    manifest['machine_before'] = state
    if state['free_gib'] < harness.MIN_FREE_GIB_FOR_HEAVY:
        reason = reason or f"only {state['free_gib']} GiB free (a heavy start needs {harness.MIN_FREE_GIB_FOR_HEAVY})"
    if args.functional_only:
        if state['free_gib'] < harness.FLOOR_FREE_GIB:
            manifest.update(status='BLOCKED', reason=f"only {state['free_gib']} GiB free (floor {harness.FLOOR_FREE_GIB})")
            return harness.die(harness.BLOCKED_EXIT, manifest, manifest_path)
        results = [run_corpus(args.binary, journey, golden, name, out, manifest) for name in args.corpora]
        manifest['machine_after'] = harness.machine()
    else:
        if reason:
            manifest.update(status='BLOCKED', reason=reason)
            return harness.die(harness.BLOCKED_EXIT, manifest, manifest_path)
        with harness.Lock() as lock:
            if not lock.held:
                manifest.update(status='BLOCKED', reason='performance lock busy (not waited for)')
                return harness.die(harness.BLOCKED_EXIT, manifest, manifest_path)
            results = [run_corpus(args.binary, journey, golden, name, out, manifest) for name in args.corpora]
            manifest['machine_after'] = harness.machine()
    report = {'journey_version': journey['version'], 'results': results}
    for result in results:
        pics = [p for p in result['pictures'] if p.get('total_ms') is not None]
        result['picture_summary'] = {
            'runtime_total_ms': summary([p['total_ms'] for p in pics]),
            'runtime_render_ms': summary([p['render_ms'] for p in pics if p.get('render_ms') is not None]),
            'pictures_seen': len(pics), 'all_read_back': all(p.get('inspected') for p in pics)}
        result['client_ms_by_op'] = {}
        for sample in result['samples']:
            result['client_ms_by_op'].setdefault(sample['op'], []).append(sample['client_ms'])
        result['client_ms_summary'] = {op: summary(v) for op, v in result['client_ms_by_op'].items()}
    harness.write(out / 'report.json', report)
    failed = [(r['corpus'], c['check']) for r in results for c in r['checks'] if not c['ok']]
    manifest['report_sha256'] = harness.sha256(out / 'report.json')
    manifest['checks'] = {'total': sum(len(r['checks']) for r in results), 'failed': len(failed)}
    manifest['failed_checks'] = failed
    ok_status = 'PASS_FUNCTIONAL_OBSERVATION_ONLY' if manifest['measurement'] else 'FUNCTIONAL_ONLY_NOT_A_MEASUREMENT'
    manifest.update(status='FAIL' if failed else ok_status,
                    reason='; '.join(f'{c}: {k}' for c, k in failed[:6]))
    manifest['utc_end'] = harness.utc()
    manifest['exit'] = 1 if failed else 0
    harness.write(manifest_path, manifest)
    print(json.dumps({k: manifest[k] for k in ('status', 'reason', 'checks', 'exit')}))
    return manifest['exit']


if __name__ == '__main__':
    raise SystemExit(main())
