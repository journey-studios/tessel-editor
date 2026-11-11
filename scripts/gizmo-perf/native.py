#!/usr/bin/env python3
"""The shared gizmo journey in the REAL window (WKWebView), from a process the runner starts itself (F055 / BL015.21).

    python3 scripts/gizmo-perf/native.py --phase build [--out DIR]
    python3 scripts/gizmo-perf/native.py --phase run   [--out DIR] [--corpora small dense deep] [--functional-only]

PROVENANCE. `build` builds the HEAD release editor through the repository's cargo wrapper (default target, finite
timeout, clean tree only) and writes `build.json`: HEAD, tree, the editor's sha256, the runtime page's sha256 and
source tree id, the corpus manifest and journey hashes. `run` refuses (`PROVENANCE_REFUSED`, exit 1) unless the editor it
is about to start IS that binary (sha256), the tree is HEAD's and clean, `runtime/dist` is the recorded one and newer than
the runtime sources, and every file the corpus manifest hashes is what it says. A number from another head, another build
or another corpus cannot come out of this runner.

THE FLOW. For each corpus: (1) a throwaway process of the SAME binary authors the corpus project (`project.create`,
`scene.attach`, `project.save` into its own HOME) and exits; (2) a FRESH process (own HOME and socket, the repository's
runtime page) is started and the journey's first step opens that SAVED project with the real `project.open`: not
`project.create` posing as an opening. The same `docs/migration/fixtures/gizmo-perf/journey.json` the core and the GPUI
host run is then replayed through the window's socket with semantic requests only (`op`, `get`, `set`, `invoke`): no
coordinate clicks, no key events, the user's own editor is never addressed. A reopened project starts with a clean history
(core contract), so the golden trace's undo depths are compared with that one offset, recorded.

A NEW PICTURE. A picture counts only when the monitor reports (`monitor.viewport`) that: the phase is `ready`, nothing is
in flight, the directed scene is the requested one, the picture it shows (`directed.shown.request_id`) is the very picture
it read back (`last_frame.request`), it is for the requested edit frame, and its request id is STRICTLY newer than the
one observed right before the command. A reply on the socket, the page's `frame.done`, a cached value or the previous
scene's picture can therefore never satisfy a wait. The checks are on the picture's pixel hash (read back by the runtime):
a commit must change it, an undo must restore the exact earlier hash, a switch must show the new scene.

CANCEL. Before the release the candidate is proven different from the base (the move reply's own candidate value and
`would_change`, and, when the host drew it, a new picture whose hash differs); after the cancel the stored transform,
the undo/redo depths and the picture hash are the base's, with a new picture when a candidate had been drawn.

WHAT IT MEASURES (all raw samples are kept, nothing is trimmed): `client_ms` per request (socket round trip: core + host +
IPC, NOT the host alone); the editor's own report of each picture (`render_ms`, `total_ms`, ... and the hash); the host's
session trace spans; the cold open as THREE separate numbers (`launch_to_socket_ms`, the `project.open` request, and
`monitor.show_scene` to its first new picture) kept apart from the hot gestures; the overlay's own acknowledgement.

WHAT IT DOES NOT PROVE (`limits`): the compositor/GPU scan-out is not observable (directed-scene pictures carry no
presented-frame stamp: `presented` is reported as not observable, never as a pass); the OS capture is a witness, not an
assertion; no physical pointer or keyboard (BL015.2/BL043/BL044 stay pending); no splat case exists; nothing is budgeted.

The performance lock is taken ONCE, without waiting, for the whole run; busy or a failed guard (load 1/5 min, thermal,
disk below the heavy-start reserve) is BLOCKED (exit 3); the guards are read again after the run and a dirty reading
makes it INVALID (exit 3). Every wait and the whole corpus are finite. Exit 0 only when every functional check passed;
1 on a failure or a refusal; 3 BLOCKED/INVALID.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import socket
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
CORPUS_TIMEOUT_S = 1200
LIMITS = [
    "the compositor / GPU scan-out is not observable; directed-scene pictures carry no presented-frame stamp (the session trace's frames_total is 0): `presented` is NOT OBSERVABLE here, so core->host->runtime is measured, the last hop is not",
    "a socket reply and the page's frame.done are not proof of painting; the picture hash is the runtime read-back and a picture only counts when its request id is newer than the one before the command",
    "the OS capture is observational (one window witness per corpus), never an assertion",
    "no physical pointer, keyboard or popover focus is used: BL015.2, BL043, BL044 remain pending",
    "client_ms includes the socket round trip and the client process: it is not the host cost",
    "no splat case: the synthetic corpus has no splat asset (BL015.23)",
    "observations of one machine/backend; nothing is budgeted and no A/B against a baseline native binary exists yet",
]


class Timeout(Exception):
    """A finite wait ran out (a check, never a hang)."""


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


# --- what the monitor says about a picture (pure: testable without a window) --------------------------------------


def picture(state):
    """The facts that identify the picture the monitor shows."""
    directed = state.get('directed') or {}
    shown = directed.get('shown') or {}
    last = state.get('last_frame') or {}
    return {'scene': directed.get('scene'), 'request': shown.get('request_id'), 'edit_frame': shown.get('edit_frame'),
            'frame_request': last.get('request'), 'revision': last.get('revision'), 'hash': last.get('hash'),
            'presented': 'not_observable'}


def mark(state):
    """The id of the newest picture shown now: what a later picture must be strictly newer than."""
    request = picture(state)['request']
    return request if isinstance(request, int) and not isinstance(request, bool) else -1


def new_picture(state, scene, frame, after):
    """`(done, info)`: is the picture the monitor shows now the NEW picture of `scene` at `frame`?"""
    directed = state.get('directed') or {}
    info = picture(state)
    request = info['request']
    done = (state.get('phase') == 'ready' and directed.get('scene') == scene and directed.get('active') is None
            and directed.get('queued') is None and not directed.get('suspended')
            and not str(state.get('headline', '')).startswith('Falha')
            and isinstance(request, int) and not isinstance(request, bool)
            and info['frame_request'] == request and info['hash'] is not None
            and (frame is None or info['edit_frame'] == frame)
            and (after is None or request > after))
    return done, info


def candidate_differs(reply, base):
    """Is the candidate a drag reply reports different from the stored transform `base` (`[position, rotation,
    scale]`)? Uses the reply's own candidate value and `would_change`, never the picture."""
    if not isinstance(reply, dict) or reply.get('applied') is not True:
        return False
    if reply.get('would_change') is not True:
        return False
    for key, at in (('position', 0), ('rotation', 1), ('scale', 2)):
        if key in reply:
            return not near(reply[key], base[at])
    return bool(reply.get('changes'))


def history_offset(want_undo, got_undo):
    """A reopened project starts with a clean history: the golden's undo depths are compared with this offset."""
    return want_undo - got_undo


# --- provenance ------------------------------------------------------------------------------------------------


def runtime_source_tree():
    return harness.out(['git', 'rev-parse', 'HEAD:runtime'])


def runtime_stale_reason(root=ROOT):
    dist = root / 'runtime/dist/runtime.js'
    if not dist.exists():
        return 'runtime/dist/runtime.js is missing: run `./scripts/runtime setup`'
    tracked = subprocess.run(['git', 'ls-files', 'runtime/src', 'runtime/host', 'runtime/worker', 'runtime/package.json',
                              'runtime/pnpm-lock.yaml'], cwd=root, stdout=subprocess.PIPE, text=True).stdout.split()
    newest = max((root / p).stat().st_mtime for p in tracked if (root / p).exists()) if tracked else 0
    if newest > dist.stat().st_mtime:
        return 'runtime/dist is older than a runtime source file: run `./scripts/runtime build`'
    return None


def verify_corpus(manifest=None, fixtures=FIXTURES, root=ROOT):
    """`None` when every file the corpus manifest hashes (and the journey) is what it says, else why not."""
    manifest = manifest or json.loads((fixtures / 'corpus.manifest.json').read_text())
    for name, entry in manifest['files'].items():
        path = fixtures / name
        if not path.exists() or harness.sha256(path) != entry['sha256']:
            return f'{name} is not the generated bundle the corpus manifest hashes'
    if harness.sha256(fixtures / 'journey.json') != manifest['journey']['sha256']:
        return 'journey.json changed after the corpus manifest was written'
    return None


def verify_provenance(record, binary, identity, allow_dirty=False, root=ROOT, fixtures=FIXTURES):
    """`None` when `binary` is the build `record` describes for the current HEAD, else why not."""
    if record is None:
        return 'no build record: run `--phase build` for this HEAD first'
    if identity['dirty'] and not allow_dirty:
        return 'the working tree is dirty: the binary cannot belong to HEAD'
    if record['head'] != identity['head'] or record['tree'] != identity['tree']:
        return f"the binary was built for {record['head'][:9]}, HEAD is {identity['head'][:9]}"
    if harness.sha256(binary) != record['binary_sha256']:
        return 'the editor binary is not the one the build phase recorded (sha256 differs)'
    dist = root / 'runtime/dist/runtime.js'
    if not dist.exists() or harness.sha256(dist) != record['runtime_sha256']:
        return 'runtime/dist/runtime.js is not the one recorded at build time'
    if record.get('runtime_source_tree') != runtime_source_tree():
        return 'the runtime sources differ from the ones the build recorded'
    stale = runtime_stale_reason(root)
    if stale:
        return stale
    if record['corpus_manifest_sha256'] != harness.sha256(fixtures / 'corpus.manifest.json'):
        return 'the corpus manifest is not the one recorded at build time'
    return verify_corpus(fixtures=fixtures, root=root)


def build_phase(args, manifest, manifest_path, out):
    if manifest['identity']['dirty'] and not args.allow_dirty:
        manifest.update(status='REFUSED', reason='the working tree is dirty: the binary would not belong to HEAD')
        return harness.die(1, manifest, manifest_path)
    reason = runtime_stale_reason() or verify_corpus()
    if reason:
        manifest.update(status='PREREQUISITE_MISSING', reason=reason)
        return harness.die(1, manifest, manifest_path)
    command = ['./scripts/cargo', 'build', '--release', '--locked', '-p', 'tessel-editor', '--message-format=json']
    done = harness.run_bounded(command, timeout=harness.BUILD_TIMEOUT_S)
    exe = None
    for line in done.stdout.splitlines():
        try:
            message = json.loads(line)
        except ValueError:
            continue
        target = message.get('target') or {}
        if message.get('executable') and target.get('name') == 'tessel-editor' and 'bin' in target.get('kind', []):
            exe = message['executable']
    manifest['build'] = {'command': command, **done.record(), 'executable': exe, 'stderr_tail': done.stderr[-1500:]}
    if done.returncode != 0 or not exe:
        manifest.update(status='BUILD_FAILED', reason='the release build failed or timed out')
        return harness.die(1, manifest, manifest_path)
    exes = out / 'exes'
    exes.mkdir(exist_ok=True)
    copy = exes / 'tessel-editor'
    shutil.copy(exe, copy)
    too_big = harness.artifact_reason(exes)
    if too_big:
        manifest.update(status='REFUSED', reason=too_big)
        return harness.die(1, manifest, manifest_path)
    record = {'head': manifest['identity']['head'], 'tree': manifest['identity']['tree'],
              'binary': str(copy), 'binary_sha256': harness.sha256(copy),
              'runtime_sha256': harness.sha256(ROOT / 'runtime/dist/runtime.js'),
              'runtime_source_tree': runtime_source_tree(),
              'corpus_manifest_sha256': harness.sha256(FIXTURES / 'corpus.manifest.json'),
              'journey_sha256': harness.sha256(FIXTURES / 'journey.json'), 'built_utc': harness.utc(),
              'toolchain': manifest['toolchain']}
    harness.write(out / 'build.json', record)
    manifest.update(status='BUILT', reason='no run was made', exit=0, record=record)
    harness.write(manifest_path, manifest)
    print(json.dumps({'status': 'BUILT', 'binary_sha256': record['binary_sha256']}))
    return 0


# --- the window ------------------------------------------------------------------------------------------------


class Window:
    """One editor process, driven through its socket the way `./scripts/ui` does, with a fast client."""

    def __init__(self, binary, home):
        os.environ.update({'TESSEL_VIEWPORT_TRACE': '1', 'TESSEL_VIEWPORT_INSPECT': '1',
                           'TESSEL_VIEWPORT_OVERLAY_ACK': '1', 'TESSEL_RUNTIME_DIR': str(ROOT / 'runtime')})
        self.launched = time.perf_counter()
        self.native = proof_surface.Native(binary, home)
        self.socket = self.native.socket
        self.frames = {}
        self.deadline = None

    def check_deadline(self):
        if self.deadline is not None and time.monotonic() > self.deadline:
            raise Timeout(f'the corpus did not finish within {CORPUS_TIMEOUT_S}s')

    def ready(self):
        self.native.prepare()
        return (time.perf_counter() - self.launched) * 1000

    def send(self, request, timeout=90):
        self.check_deadline()
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

    def mark(self):
        return mark(self.viewport())

    def settle(self, scene, frame, after=None, seconds=30):
        """`(ms, state, info)`: seconds (client clock) until the monitor showed the NEW picture of `scene` at `frame`
        (strictly newer than request `after`) with nothing in flight; `ms` is None on timeout."""
        started = time.perf_counter()
        deadline = started + seconds
        state, info = None, None
        while time.perf_counter() < deadline:
            self.check_deadline()
            state = self.viewport()
            done, info = new_picture(state, scene, frame, after)
            if done:
                return (time.perf_counter() - started) * 1000, state, info
            time.sleep(0.01)
        return None, state, info

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


def author_saved_project(binary, variables, corpus_name, scratch):
    """A throwaway process of the SAME binary writes the corpus project (create, attach, save) and exits. Returns
    `(path, record)`; the record is kept in the report (hash and size of the saved document)."""
    home = tempfile.mkdtemp(prefix='gp-author-', dir='/tmp')
    window = None
    try:
        window = Window(binary, home)
        window.ready()
        path = Path(scratch) / f'{corpus_name}.tessel.json'
        for request in ({'op': 'project.create', 'name': f'gizmo-perf-{corpus_name}'},
                        {'op': 'scene.attach', 'id': variables['scene'], 'path': variables['bundle']},
                        {'op': 'project.save', 'path': str(path)}):
            ok, data = window.op(request, 120)
            if not ok:
                raise RuntimeError(f"authoring `{request['op']}` failed: {data}")
        if not path.exists():
            raise RuntimeError('`project.save` answered ok but wrote no document')
        return path, {'path': str(path), 'sha256': harness.sha256(path), 'bytes': path.stat().st_size}
    finally:
        if window is not None:
            window.stop()
        shutil.rmtree(home, ignore_errors=True)


def run_corpus(binary, journey, golden, name, out, notes):
    corpus = journey['corpora'][name]
    base = str(ROOT / journey['base'])
    variables = {'scene': journey['scene'], 'scene_b': journey['scene_b'], 'frame': journey['frame'],
                 'bundle': str(ROOT / corpus['bundle']), 'bundle_b': str(ROOT / corpus['bundle_b']), 'base': base,
                 'node': corpus['node'], 'node_b': corpus['node_b'], 'other': corpus['other']}
    scratch = tempfile.mkdtemp(prefix='gp-scratch-', dir='/tmp')
    home = tempfile.mkdtemp(prefix='gp-', dir='/tmp')
    window = None
    result = {'corpus': name, 'checks': [], 'samples': [], 'steps': {}, 'pictures': [], 'overlay_drawn': [],
              'captures': [], 'cleanup_errors': []}

    def check(label, condition, detail=''):
        result['checks'].append({'check': label, 'ok': bool(condition), 'detail': str(detail)[:300]})
        return bool(condition)

    try:
        project, authored = author_saved_project(binary, variables, name, scratch)
        result['authored_project'] = authored
        window = Window(binary, home)
        window.deadline = time.monotonic() + CORPUS_TIMEOUT_S
        result['launch_to_socket_ms'] = window.ready()  # process start -> its socket answers: the cold launch
        result['pid'] = window.native.process.pid
        scene = journey['scene']
        node = corpus['node']
        shown = {'scene': None}
        committed, undone = [], []
        hashes = []  # picture hashes before each committed gesture, for the exact-undo pixel check
        offset = {'undo': 0}
        marks = {}

        def timed(step, request, timeout=120, phase=None):
            started = time.perf_counter()
            ok, data = window.op(request, timeout)
            result['samples'].append({'step': step, 'phase': phase, 'op': request['op'],
                                      'client_ms': (time.perf_counter() - started) * 1000,
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
            phase = step.get('phase')
            if 'surfaces' in step and 'native' not in step['surfaces']:
                result['steps'][sid] = {'skipped': f"listed for {step['surfaces']}"}
                continue
            if sid == 'switch.attach':
                scene = journey['scene_b']
                node = corpus['node_b']
            record = result['steps'].setdefault(sid, {'phase': phase})
            if step.get('kind') == 'drag':
                drag(window, timed, check, step, scene, variables, node, committed, hashes, result, shown)
            elif sid == 'open.project':
                # The real opening of a SAVED project, in a process that has never drawn anything.
                ok, data = timed(sid, {'op': 'project.open', 'path': str(project)}, phase=phase)
                check(f'{sid}: project.open of the saved project answered', ok, data)
                check(f'{sid}: it was the opening of a document (action project.open)',
                      ok and isinstance(data, dict) and data.get('action') == 'project.open', data)
                record['kind'] = 'project.open of a saved project'
                result['opened_saved_project'] = True
            elif sid == 'open.attach':
                # The scene came with the saved project: there is no attach to run, only to verify.
                record['covered_by'] = 'project.open of the saved project'
                result['attached'] = True
                got_undo = depths(window)[0]
                check(f'{sid}: a reopened project starts with a clean history', got_undo == 0, got_undo)
                want = golden[name].get(sid)
                if want is not None:
                    offset['undo'] = history_offset(want['undo'], got_undo)
                record['history_offset'] = offset['undo']
            elif step.get('ui'):
                words = step['ui']
                if words[:3] == ['set', 'window', 'size']:
                    answer = window.send({'command': 'set', 'target': 'window', 'field': 'size', 'value': words[3]})
                else:
                    answer = window.send({'command': 'invoke', 'target': words[1]})
                check(f'{sid}: the window accepted `ui {" ".join(words)}`', answer.get('ok'), answer)
                if sid.startswith('present.resize'):
                    ms, state, info = window.settle(shown['scene'], journey['frame'])
                    record['settle_ms'] = ms
                    check(f'{sid}: the monitor converged after the resize', ms is not None, json.dumps(info))
                if sid == 'present.dialog.open':
                    time.sleep(0.3)
                    record['page_hidden_by_dialog'] = (window.viewport().get('on_screen') is False
                                                       or window.viewport().get('hidden'))
            elif step.get('wait') == 'presented':
                ms, state, info = window.settle(shown['scene'], journey['frame'], after=marks.get('seek'))
                record['client_ms_to_first_picture'] = ms
                record['picture'] = info
                check(f'{sid}: a NEW picture of frame {journey["frame"]} was drawn after the seek', ms is not None,
                      json.dumps(info))
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
                    timed(sid, {'op': 'scene.gizmo.tool', **step['tool']}, phase=phase)
                request = step['op']
                before_hash = picture_hash(window.viewport()) if request['op'] in ('history.undo', 'history.redo') else None
                before_mark = window.mark() if request['op'] in ('monitor.show_scene', 'seek', 'history.undo',
                                                                  'history.redo') else None
                ok, data = timed(sid, request, phase=phase)
                check(f'{sid}: {request["op"]} answered', ok, data)
                if request['op'] == 'seek':
                    marks['seek'] = before_mark
                if request['op'] == 'scene.resources.resolve' and ok:
                    commit_job(sid, data)
                if request['op'] == 'monitor.show_scene' and ok:
                    target = request['scene']
                    ms, state, info = window.settle(target, None if sid == 'open.show' else journey['frame'],
                                                    after=before_mark, seconds=20)
                    record['client_ms_to_converged_picture'] = ms
                    record['picture'] = info
                    wanted = (step.get('expect') or {}).get('shown_scene', target)
                    check(f'{sid}: `{wanted}` drew a NEW picture (newer than request {before_mark})',
                          ms is not None and info['scene'] == wanted, json.dumps(info))
                    if ms is not None:
                        shown['scene'] = target
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
                    ms, state, info = window.settle(shown['scene'], journey['frame'], after=before_mark)
                    record['client_ms_to_converged_picture'] = ms
                    record['picture'] = info
                    check(f'{sid}: the {request["op"]} drew a NEW picture', ms is not None, json.dumps(info))
                    if request['op'] == 'history.undo' and committed:
                        if before_hash is not None and hashes and ms is not None:
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
                    check(f'{sid}: transform/undo/redo equal the core golden trace (undo offset {offset["undo"]})',
                          near(got['transform'], want['transform']) and got['undo'] + offset['undo'] == want['undo']
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
    except BaseException as error:  # noqa: BLE001 - any cause is a recorded failed check; the cleanup below still runs
        check('the journey ran to the end', False, f'{type(error).__name__}: {error}')
        if isinstance(error, (KeyboardInterrupt, SystemExit)):
            raise
    finally:
        if window is not None:
            result['cleanup_errors'] += window.stop()
        shutil.rmtree(home, ignore_errors=True)
        shutil.rmtree(scratch, ignore_errors=True)
        check('the editor process ended without cleanup problems', not result['cleanup_errors'],
              result['cleanup_errors'])
    return result


def drag(window, timed, check, step, scene, variables, node, committed, hashes, result, shown):
    sid = step['id']
    phase = step.get('phase')
    frame = variables['frame']
    record = result['steps'][sid]
    timed(sid, {'op': 'scene.gizmo.tool', 'tool': step['tool'], 'space': step['space']}, phase=phase)
    before = transform(window, scene, node)
    undo_before, redo_before = depths(window)
    ms, state, info = window.settle(scene, frame)
    # The picture before the gesture is the base of every pixel comparison below. `settle` hands back the last state it
    # saw when it times out (`ms is None`): that picture never converged and is not a base, so nothing is pressed.
    if not check(f'{sid}: the picture before the gesture converged (it is the base of the pixel checks)',
                 ms is not None, json.dumps(info)[:260]):
        return
    start_hash = picture_hash(state)
    start_mark = mark(state)
    ok, listed = timed(sid, {'op': 'scene.gizmo.state', 'scene': scene, 'frame': frame}, phase=phase)
    if not check(f'{sid}: gizmo state', ok, listed):
        return
    handle = next((h for h in listed['overlay']['handles'] if h['id'] == step['handle']), None)
    if not check(f'{sid}: the overlay has the `{step["handle"]}` handle', handle is not None, listed['overlay']):
        return
    ring = []
    if handle.get('points'):
        points = handle['points']
        ring = [p for p in points[len(points) // 8:] if p]
        start = ring[0]
    else:
        start = handle['to']
    ok, down = timed(sid, {'op': 'scene.pointer.down', 'scene': scene, 'frame': frame, 'point': start}, phase=phase)
    if not (check(f'{sid}: the press landed on the handle', ok and down.get('dragging') is True, down)
            and check(f'{sid}: the hit handle is `{step["handle"]}`', down['hit']['handle'] == step['handle'], down.get('hit'))):
        return
    moves = step['moves']
    delta = step.get('delta') or [0, 0]
    applied = 0
    last_move = None
    for index in range(1, moves + 1):
        if ring:
            point = ring[min(index, len(ring) - 1)]
        else:
            point = {'u': start['u'] + delta[0] * index / moves, 'v': start['v'] + delta[1] * index / moves}
        ok, moved = timed(sid, {'op': 'scene.pointer.move', 'point': point}, phase=phase)
        check(f'{sid}: move {index} answered and stayed a candidate', ok and moved.get('document_changed') is False, moved)
        if ok and moved.get('applied') is True:
            applied += 1
            last_move = moved
    check(f'{sid}: a move was applied', applied > 0)
    candidate_real = candidate_differs(last_move, before)
    check(f'{sid}: the candidate differs from the base (the reply\'s own candidate value)', candidate_real, last_move)
    # Was the candidate DRAWN? A picture newer than the press, for this scene, shown while the gesture is open.
    ms_c, state_c, info_c = window.settle(scene, frame, after=start_mark, seconds=2)
    candidate_drawn = ms_c is not None
    record['candidate_picture_drawn'] = candidate_drawn
    if candidate_drawn:
        check(f'{sid}: the drawn candidate picture differs from the base picture',
              picture_hash(state_c) != start_hash, (start_hash, picture_hash(state_c)))
        record['candidate_picture'] = info_c
    window.trace()
    check(f'{sid}: the candidate is not in the document before the release', near(transform(window, scene, node), before))
    gesture_mark = mark(window.viewport())
    if step['end'] == 'up':
        ok, up = timed(sid, {'op': 'scene.pointer.up'}, phase=phase)
        check(f'{sid}: the release committed', ok and up.get('document_changed') is True, up)
        ms, state, info = window.settle(scene, frame, after=gesture_mark if candidate_drawn else start_mark)
        record['client_ms_up_to_converged_picture'] = ms
        record['picture'] = info
        check(f'{sid}: the committed picture is a NEW picture', ms is not None, json.dumps(info))
        after_hash = picture_hash(state) if state else None
        check(f'{sid}: the committed picture differs from the one before the gesture',
              after_hash is not None and after_hash != start_hash, (start_hash, after_hash))
        check(f'{sid}: one undo entry per gesture', depths(window)[0] == undo_before + 1)
        committed.append((before, transform(window, scene, node)))
        hashes.append(start_hash)
    else:
        ok, cancel = timed(sid, {'op': 'scene.pointer.cancel', 'reason': 'api'}, phase=phase)
        check(f'{sid}: the cancel answered', ok, cancel)
        if candidate_drawn:
            ms, state, info = window.settle(scene, frame, after=gesture_mark)
            check(f'{sid}: the cancel drew a NEW picture (the candidate had been drawn)', ms is not None, json.dumps(info))
        else:
            ms, state, info = window.settle(scene, frame, after=None)
            check(f'{sid}: the monitor converged after the cancel', ms is not None, json.dumps(info))
        record['picture'] = info
        # A timeout (`ms is None`) leaves the last state seen, which can still hold the base picture's hash: only a
        # picture the monitor converged on after the cancel may be compared with the base.
        check(f'{sid}: after the cancel the picture is the base picture',
              ms is not None and state is not None and picture_hash(state) == start_hash,
              (ms, start_hash, picture_hash(state) if state else None))
        check(f'{sid}: the cancel left the base document (transform equals the one before the press)',
              near(transform(window, scene, node), before))
        check(f'{sid}: the cancel added no undo entry and no redo entry',
              depths(window) == (undo_before, redo_before), (depths(window), (undo_before, redo_before)))


def main(argv=None):
    harness.install_termination_handlers()
    parser = argparse.ArgumentParser()
    parser.add_argument('--phase', choices=['build', 'run'], default='run')
    parser.add_argument('--binary', help='the editor binary (default: the one `build.json` recorded)')
    parser.add_argument('--build-record', help='`build.json` of the build phase (default: --out/build.json)')
    parser.add_argument('--out', default='/tmp/gizmo-perf-native')
    parser.add_argument('--corpora', nargs='*', default=['small', 'dense', 'deep'])
    parser.add_argument('--allow-dirty', action='store_true',
                        help='run with a dirty tree (recorded; the binary cannot be claimed for HEAD)')
    parser.add_argument('--functional-only', action='store_true',
                        help='run the journey and its functional checks WITHOUT the performance lock and the load '
                             'guard; the timings of such a run are not a measurement and the status says so')
    args = parser.parse_args(argv)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    manifest_path = out / ('manifest.build.json' if args.phase == 'build' else 'manifest.json')
    manifest = {'suite': 'gizmo-perf-native', 'phase': args.phase, 'utc_start': harness.utc(), 'cwd': str(ROOT),
                'limits': list(LIMITS), 'identity': harness.git_identity(), 'toolchain': harness.toolchain(),
                'status': 'ABORTED', 'command': sys.argv}
    if args.phase == 'build':
        return build_phase(args, manifest, manifest_path, out)
    record_path = Path(args.build_record) if args.build_record else out / 'build.json'
    record = json.loads(record_path.read_text()) if record_path.exists() else None
    binary = args.binary or (record or {}).get('binary') or str(ROOT / 'target/release/tessel-editor')
    manifest['binary'] = binary
    if not Path(binary).exists() or not (ROOT / 'runtime/dist/runtime.js').exists():
        manifest.update(status='PREREQUISITE_MISSING',
                        reason='needs the release editor (`--phase build`) and `./scripts/runtime setup` (runtime/dist)')
        return harness.die(1, manifest, manifest_path)
    manifest['binary_sha256'] = harness.sha256(binary)
    manifest['runtime_sha256'] = harness.sha256(ROOT / 'runtime/dist/runtime.js')
    manifest['build_record'] = record
    manifest['dirty_run'] = bool(manifest['identity']['dirty'])
    refusal = verify_provenance(record, binary, manifest['identity'], args.allow_dirty)
    if refusal:
        manifest.update(status='PROVENANCE_REFUSED', reason=refusal)
        return harness.die(1, manifest, manifest_path)
    journey = json.loads((FIXTURES / 'journey.json').read_text())
    golden = json.loads((FIXTURES / 'journey.expected.json').read_text())
    manifest['corpus'] = json.loads((FIXTURES / 'corpus.manifest.json').read_text())
    manifest['measurement'] = not args.functional_only
    state, reason = harness.guards()
    manifest['machine_before'] = state
    manifest['limits'] += harness.claims(state)
    if state['free_gib'] < harness.MIN_FREE_GIB_FOR_HEAVY:
        reason = reason or f"only {state['free_gib']} GiB free (a heavy start needs {harness.MIN_FREE_GIB_FOR_HEAVY})"
    after_reason = None

    def collect():
        """Every corpus, in order. A termination (SIGTERM/Ctrl-C) leaves a `TERMINATED` manifest, never nothing."""
        try:
            return [run_corpus(binary, journey, golden, name, out, manifest) for name in args.corpora]
        except BaseException as error:
            manifest.update(status='TERMINATED', reason=f'{type(error).__name__}: {error}', utc_end=harness.utc())
            harness.write(manifest_path, manifest)
            raise

    if args.functional_only:
        if state['free_gib'] < harness.FLOOR_FREE_GIB:
            manifest.update(status='BLOCKED', reason=f"only {state['free_gib']} GiB free (floor {harness.FLOOR_FREE_GIB})")
            return harness.die(harness.BLOCKED_EXIT, manifest, manifest_path)
        results = collect()
        manifest['machine_after'] = harness.machine()
    else:
        if reason:
            manifest.update(status='BLOCKED', reason=reason)
            return harness.die(harness.BLOCKED_EXIT, manifest, manifest_path)
        with harness.Lock() as lock:
            if not lock.held:
                manifest.update(status='BLOCKED', reason='performance lock busy (not waited for)')
                return harness.die(harness.BLOCKED_EXIT, manifest, manifest_path)
            results = collect()
            after_reason = harness.judge_after(manifest)
    report = {'journey_version': journey['version'], 'results': results}
    for result in results:
        pics = [p for p in result['pictures'] if p.get('total_ms') is not None]
        result['picture_summary'] = {
            'runtime_total_ms': summary([p['total_ms'] for p in pics]),
            'runtime_render_ms': summary([p['render_ms'] for p in pics if p.get('render_ms') is not None]),
            'pictures_seen': len(pics), 'all_read_back': all(p.get('inspected') for p in pics)}
        result['client_ms_by_op'] = {}
        result['client_ms_by_phase'] = {}
        for sample in result['samples']:
            result['client_ms_by_op'].setdefault(sample['op'], []).append(sample['client_ms'])
            result['client_ms_by_phase'].setdefault(sample.get('phase') or '?', []).append(sample['client_ms'])
        result['client_ms_summary'] = {op: summary(v) for op, v in result['client_ms_by_op'].items()}
        result['client_ms_phase_summary'] = {p: summary(v) for p, v in result['client_ms_by_phase'].items()}
    harness.write(out / 'report.json', report)
    failed = [(r['corpus'], c['check']) for r in results for c in r['checks'] if not c['ok']]
    manifest['report_sha256'] = harness.sha256(out / 'report.json')
    manifest['checks'] = {'total': sum(len(r['checks']) for r in results), 'failed': len(failed)}
    manifest['failed_checks'] = failed
    ok_status = 'PASS_FUNCTIONAL_OBSERVATION_ONLY' if manifest['measurement'] else 'FUNCTIONAL_ONLY_NOT_A_MEASUREMENT'
    if failed:
        status, code = 'FAIL', 1
    elif after_reason:
        status, code = 'INVALID_GUARD_AFTER_RUN', harness.BLOCKED_EXIT
        manifest['reason'] = after_reason
    elif manifest['dirty_run']:
        status, code = 'DIRTY_RUN_NOT_FOR_HEAD', 1
    else:
        status, code = ok_status, 0
    manifest['status'] = status
    manifest['reason'] = manifest.get('reason') if status == 'INVALID_GUARD_AFTER_RUN' else '; '.join(
        f'{c}: {k}' for c, k in failed[:6])
    manifest['utc_end'] = harness.utc()
    manifest['exit'] = code
    harness.write(manifest_path, manifest)
    print(json.dumps({k: manifest[k] for k in ('status', 'reason', 'checks', 'exit')}))
    return code


if __name__ == '__main__':
    raise SystemExit(main())
