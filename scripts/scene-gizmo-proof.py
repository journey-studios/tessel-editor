#!/usr/bin/env python3
"""F055 / BL015 / BL042 (S8) — the translation gizmo, proved on two separate surfaces with one scenario:

    python3 scripts/directed-scene-proof/with-gpu-lock.py \
        python3 scripts/scene-gizmo-proof.py [--editor target/debug/tessel-editor] \
        [--headless target/debug/tessel-headless] [--out report.json] [--allow-dirty]

* **headless (no window)**: `tessel-headless --empty`, JSON Lines over stdio. No GPUI, no WebView.
* **native window**: the DIRECT editor binary started with its own short `HOME` (own socket, settings and
  runtime page; the user's window and project are never touched), driven only by that binary's `ui` socket
  (`op`, `get`, `invoke`). The directed scene is shown on the monitor, so the overlay is on in the page.

The scenario is the S8 contract in semantic requests. It loads the corpus bundle (animated: the `stage` group
slides and `hero` sways), selects `hero` at an animating frame and then:

* the overlay's origin is where an INDEPENDENT pinhole (written here from the plan's camera and the node's
  world matrix, not from the core) puts the node;
* a drag of the screen handle ends with the node's projected origin under the pointer (also independent), as ONE
  undo entry; the candidate is not the document until the release (read back through `project.save`);
* `scene.translate` with the delta the drag reported leaves the same saved document;
* undo restores the saved document exactly; zero deltas and a press released in place leave no entry;
* every cancel reason, every refusal and an edit under an open drag leave document, history and revision as they
  were (`gesture_invalidated` is typed; cancel settles the dead drag).

The native run adds what only a window has: the page negotiates `overlay.gizmo`, draws the handles (pixel probes
read back from the canvas, opt-in `TESSEL_VIEWPORT_OVERLAY_ACK=1`), the monitor's picture follows the candidate and
is the committed picture after the release and the initial one after undo (hashes), and a dialog that hides the
WebView cancels the open drag.

What the report promises (M3 of the PR #25 review): the `--out` file is replaced at the START by `ok: false` and
only a complete, clean run turns it into `ok: true`; ANY exception (including a missing binary, a dead child or a
bad JSON line) leaves a fresh `ok: false` with the error, and Ctrl-C does too. A surface with zero checks, or
without the checks it is required to have run (`REQUIRED`), fails; so does an empty or unknown `--surfaces`.
`final_evidence` is true only for both surfaces, on a tree with no modified AND no untracked (non-ignored) file,
without `--allow-dirty`. A failed run keeps its work directory (editor log, headless stderr, saved documents) and
says where; a passing one removes it. The child processes are always ended (terminate, then kill).

Not proven here, on purpose: a PHYSICAL pointer or a native drag on the WKWebView (BL043/BL044). The `scene.pointer.*`
requests carry picture coordinates (u, v in 0..1), not screen coordinates; they exercise the same handlers the page's
events do (`overlay.pointer`), but a synthetic request is not a mouse. The page's own mouse handling is proved with
real mouse events in headless Chrome by `runtime/test/gizmo-overlay.test.mjs`.
"""
import argparse
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import traceback

ROOT = Path(__file__).resolve().parent.parent
CORPUS = ROOT / 'scripts/directed-scene-proof/corpus'
BUNDLE = CORPUS / 'bundle.json'
MIN_FREE_GIB = 12
SCENE = 'proof'
HERO = 'scene/scene-a/node/hero'
HERO_POSITION = f'{HERO}/position'
SKY = 'scene/scene-a/node/sky'
FRAME = 24  # the stage has slid and the hero sways: the node is animating here
TOLERANCE = 1e-7
SURFACES = ('headless', 'native')
# Checks that run through hooks or at the end of the scenario: if the hook does not fire, they would vanish
# without a trace and `passed` would only count the rest. A surface that did not run every one of these failed.
_COMMON = (
    'the corpus bundle loads',
    'a selected node has a gizmo with x, y, z and screen handles',
    'the overlay origin is where an independent pinhole puts the node',
    'the release is one undo entry',
    'the node now projects under the pointer (independent pinhole, plan read after the commit)',
    'and it leaves the same saved document as the drag did',
    'Escape with nothing to cancel is not an error',
    'one undo takes the unrelated edit back',
)
REQUIRED = {
    'headless': _COMMON,
    'native': _COMMON + (
        'the page announced `overlay.gizmo` and the overlay is on for the shown scene',
        'the page acknowledged what it drew',
        'the x axis is painted red and the y axis green on the canvas (alpha > 200)',
        'the page drew the active handle of the drag',
        'and shows the initial picture again',
        'the open drag was cancelled by the hide (never committed)',
        'the gizmo is drawn again for the selection',
    ),
}
STOP_GRACE = 30  # seconds a child gets to exit by itself before it is terminated, then killed


class ProofFailure(AssertionError):
    pass


class Recorder:
    def __init__(self, surface):
        self.surface = surface
        self.checks = []

    def __call__(self, name, condition, detail=''):
        self.checks.append({'name': name, 'ok': bool(condition), 'detail': '' if condition else str(detail)[:500]})
        print(('PASS ' if condition else 'FAIL ') + f'[{self.surface}] ' + name
              + ('' if condition else f'  [{str(detail)[:300]}]'), flush=True)
        if not condition:
            raise ProofFailure(f'[{self.surface}] {name}')


def normalize(body):
    if body.get('ok'):
        return True, body.get('data', body.get('result'))
    return False, body.get('error')


def sha256(path):
    digest = hashlib.sha256()
    with open(path, 'rb') as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b''):
            digest.update(chunk)
    return digest.hexdigest()


def tree_status(root=ROOT):
    """`(clean, porcelain)`: clean means NO modified/staged file and NO untracked file that is not ignored (a new
    file the commit does not contain is not the head either)."""
    status = subprocess.run(['git', 'status', '--porcelain', '--untracked-files=all'], cwd=root,
                            capture_output=True, text=True, check=True).stdout
    return status.strip() == '', status


def git_head(root=ROOT):
    return subprocess.run(['git', 'rev-parse', 'HEAD'], cwd=root, capture_output=True, text=True,
                          check=True).stdout.strip()


def parse_surfaces(text):
    """The surfaces to run, in canonical order. Empty, unknown and repeated names are refused: a run of nothing must
    never be a pass."""
    names = [name.strip() for name in text.split(',')]
    if not text.strip() or any(not name for name in names):
        raise RuntimeError(f'--surfaces is empty: nothing would be proved (known: {", ".join(SURFACES)})')
    unknown = [name for name in names if name not in SURFACES]
    if unknown:
        raise RuntimeError(f'unknown surface {unknown} in --surfaces (known: {", ".join(SURFACES)})')
    if len(set(names)) != len(names):
        raise RuntimeError(f'--surfaces repeats a surface: {names}')
    return [name for name in SURFACES if name in names]


def missing_checks(name, checks, required=REQUIRED):
    """What a finished surface was required to have run and did not (everything, when it ran nothing)."""
    ran = {check['name'] for check in checks}
    return [required_name for required_name in required[name] if required_name not in ran]


def final_blockers(wanted, allow_dirty, clean):
    """Why a passing run is still not final evidence (empty: it is)."""
    blockers = []
    if list(wanted) != list(SURFACES):
        blockers.append(f'surfaces {wanted} are a subset of {list(SURFACES)}')
    if allow_dirty:
        blockers.append('--allow-dirty')
    if not clean:
        blockers.append('the tree has modified or untracked files')
    return blockers


def write_report(out, report):
    """Atomically replaces `out`: a reader sees the old file or the new one, never half of one."""
    if out is None:
        return
    out = Path(out)
    out.parent.mkdir(parents=True, exist_ok=True)
    temporary = out.with_name(out.name + '.tmp')
    temporary.write_text(json.dumps(report, indent=2) + '\n')
    os.replace(temporary, out)


def describe(error):
    return f'{type(error).__name__}: {error}'


# ---- the independent pinhole: nothing here is the core's code ---------------------------------------------------

def sub(a, b):
    return [x - y for x, y in zip(a, b)]


def dot(a, b):
    return sum(x * y for x, y in zip(a, b))


def cross(a, b):
    return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]


def unit(a):
    length = math.sqrt(dot(a, a))
    return [x / length for x in a]


def project(camera, output, point):
    """Picture position `(u, v)` (v grows downward) of a world point: vertical field of view, +Y up."""
    position = [camera['position'][axis] for axis in 'xyz']
    target = [camera['target'][axis] for axis in 'xyz']
    forward = unit(sub(target, position))
    right = unit(cross(forward, [0, 1, 0]))
    up = cross(right, forward)
    relative = sub(point, position)
    depth = dot(relative, forward)
    half = math.tan(math.radians(camera['fovDeg']) / 2)
    aspect = output['width'] / output['height']
    x = dot(relative, right) / (depth * half * aspect)
    y = dot(relative, up) / (depth * half)
    return (x + 1) / 2, (1 - y) / 2


class Headless:
    name = 'headless'

    def __init__(self, binary, work):
        self.stderr_path = os.path.join(work, 'headless.stderr.log')
        self.stderr = open(self.stderr_path, 'w')
        try:
            self.process = subprocess.Popen([binary, '--empty'], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                            stderr=self.stderr, text=True, bufsize=1)
        except BaseException:
            self.stderr.close()
            raise

    def log_tail(self, size=600):
        try:
            return Path(self.stderr_path).read_text()[-size:]
        except OSError:
            return ''

    def _gone(self, what):
        code = self.process.poll()
        return RuntimeError(f'tessel-headless {what}'
                            + (f' (exited {code})' if code is not None else '') + f'; stderr: {self.log_tail()!r}')

    def raw(self, request):
        if self.process.poll() is not None:
            raise self._gone(f'is not running for `{request.get("op")}`')
        try:
            self.process.stdin.write(json.dumps(request) + '\n')
            self.process.stdin.flush()
        except OSError as error:
            raise self._gone(f'closed its input ({error})') from error
        line = self.process.stdout.readline()
        if not line:
            raise self._gone(f'closed its output answering `{request.get("op")}`')
        try:
            return json.loads(line)
        except ValueError as error:
            raise self._gone(f'answered a line that is not JSON ({error}): {line[:200]!r}') from error

    def op(self, request):
        return normalize(self.raw(request))

    def prepare(self):
        pass

    def stop(self):
        """Ends the child whatever it does: EOF first, then terminate, then kill. Never returns with it alive."""
        try:
            try:
                self.process.stdin.close()
            except OSError:
                pass
            try:
                self.process.wait(STOP_GRACE)
            except subprocess.TimeoutExpired:
                self.process.terminate()
                try:
                    self.process.wait(5)
                except subprocess.TimeoutExpired:
                    self.kill()
        finally:
            self.kill()
            for stream in (self.process.stdout, self.stderr):
                try:
                    stream.close()
                except OSError:
                    pass

    def kill(self):
        if self.process.poll() is None:
            self.process.kill()
            self.process.wait()


def load_lifecycle():
    """The monitor driver of the lifecycle proof (`Lab`, `settle`, `commit_job`): the isolated-instance mechanics, no copy."""
    path = ROOT / 'scripts' / 'scene-lifecycle-proof.py'
    spec = importlib.util.spec_from_file_location('scene_lifecycle_proof', path)
    module = importlib.util.module_from_spec(spec)
    sys.modules['scene_lifecycle_proof'] = module
    spec.loader.exec_module(module)
    return module


class Native:
    """The DIRECT editor binary with its own short HOME (a `Lab` of the lifecycle proof), driven over its socket."""
    name = 'native'

    def __init__(self, binary, work):
        self.lifecycle = load_lifecycle()
        self.lab = self.lifecycle.Lab(binary, work, {'TESSEL_VIEWPORT_INSPECT': '1', 'TESSEL_VIEWPORT_OVERLAY_ACK': '1'})

    def op(self, request):
        return self.lab.op(request)

    def get(self, target):
        return self.lab.send({'command': 'get', 'target': target})

    def invoke(self, target, action=None):
        return self.lab.invoke(target, action)

    def viewport(self):
        return self.lab.viewport()

    def prepare(self):
        self.lab.wait_ready()
        self.lab.invoke('window.activate')
        time.sleep(0.8)

    def stop(self):
        try:
            self.lab.stop()
        finally:
            self.kill()

    def kill(self):
        process = self.lab.process
        if process.poll() is None:
            process.kill()
            process.wait()

    def log_tail(self, size=600):
        try:
            return Path(self.lab.log_path).read_text()[-size:]
        except OSError:
            return ''


def scenario(surface, check, scratch, record):
    """The contract, identical on both surfaces. The document is read back through `project.save`."""

    def ok(request):
        good, payload = surface.op(request)
        if not good:
            raise ProofFailure(f'{request} -> {payload}')
        return payload

    def refused(request):
        good, payload = surface.op(request)
        if good:
            raise ProofFailure(f'{request} should have been refused, got {payload}')
        return payload

    saves = [0]

    def saved():
        saves[0] += 1
        path = scratch / f'doc-{saves[0]}.tessel.json'
        ok({'op': 'project.save', 'path': str(path)})
        return json.loads(path.read_text())

    def bundle():
        return saved()['scenes']['items'][0]['bundle']

    def node(bundle_doc, node_id):
        definition = bundle_doc['sceneDefinitions'][0]
        return next(candidate for candidate in definition['nodes'] if candidate['id'] == node_id)

    def history():
        data = ok({'op': 'history.status'})
        return data['history']['undo_depth'], data['project']['revision']

    def untouched(label, base_bundle, base_history):
        check(f'{label}: document, undo depth and revision are as they were',
              bundle() == base_bundle and history() == base_history,
              (history(), base_history))

    def plan_at(frame=FRAME):
        return ok({'op': 'scene.plan', 'scene': SCENE, 'frame': frame})['plan']

    def hero_origin(frame=FRAME):
        plan = plan_at(frame)
        matrix = next(n for n in plan['nodes'] if n['id'] == 'hero')['worldMatrix']
        return [matrix[12], matrix[13], matrix[14]]

    def projected_hero(frame=FRAME):
        plan = plan_at(frame)
        return project(plan['camera'], plan['output'], hero_origin(frame))

    def overlay(frame=FRAME):
        return ok({'op': 'scene.gizmo.state', 'scene': SCENE, 'frame': frame})['overlay']

    def handle(shown, handle_id):
        return next(h for h in shown['handles'] if h['id'] == handle_id)

    def press(point, frame=FRAME):
        return ok({'op': 'scene.pointer.down', 'scene': SCENE, 'frame': frame, 'point': point})

    # ---- the scene, the selection and the overlay ----------------------------------------------------------------
    if surface.name == 'native':
        ok({'op': 'project.create', 'name': 'S8 proof', 'sequence': 'main', 'rate': 24, 'width': 320, 'height': 180,
            'length': 96, 'dir': str(scratch / 'workspace')})
    attached = ok({'op': 'scene.attach', 'id': SCENE, 'path': str(BUNDLE)})
    check('the corpus bundle loads', attached.get('scene') == SCENE, attached)
    if surface.name == 'native':
        loaded = ok({'op': 'scene.resources.resolve', 'scene': SCENE, 'base': str(CORPUS)})
        check('the resources are resolved and applied', surface.lifecycle.commit_job(surface.lab, loaded, 'gizmo'), loaded)
        shown = ok({'op': 'monitor.show_scene', 'scene': SCENE})
        check('the monitor shows the directed scene', shown['scene'] == SCENE, shown)
        surface.lab.seek(FRAME)
        check('the monitor converged on the animating frame', surface.lifecycle.settle(surface.lab, FRAME, 60) is not None,
              surface.viewport().get('directed'))

    base_bundle = bundle()
    base_history = history()
    hidden = overlay()
    check('with nothing selected there is no gizmo and the reason is typed',
          hidden['visible'] is False and hidden['reason'] == 'no_selection' and hidden['handles'] == [], hidden)
    ok({'op': 'scene.select', 'scene': SCENE, 'entities': [HERO]})
    shown = overlay()
    plan = plan_at()
    origin = projected_hero()
    check('a selected node has a gizmo with x, y, z and screen handles',
          shown['visible'] is True and [h['id'] for h in shown['handles']] == ['x', 'y', 'z', 'screen'], shown)
    check('the node animates at this frame (its world origin differs from frame 0)',
          max(abs(a - b) for a, b in zip(hero_origin(FRAME), hero_origin(0))) > 0.05, (hero_origin(FRAME), hero_origin(0)))
    check('the overlay origin is where an independent pinhole puts the node',
          abs(shown['origin']['u'] - origin[0]) < 1e-9 and abs(shown['origin']['v'] - origin[1]) < 1e-9,
          (shown['origin'], origin))
    check('the overlay speaks of the picture, not of a pane: its aspect is the output aspect',
          abs(shown['aspect'] - plan['output']['width'] / plan['output']['height']) < 1e-12, shown['aspect'])
    stamp = shown['stamp']
    check('the overlay is stamped with scene, session, revision and frame',
          stamp['scene'] == SCENE and stamp['frame'] == FRAME and stamp['revision'] == history()[1]
          and isinstance(stamp['session'], str) and stamp['session'], stamp)

    # ---- a drag of the screen handle: the node goes under the pointer, as one undo entry -----------------------------
    target = {'u': origin[0] + 0.07, 'v': origin[1] - 0.05}
    check('control: before the drag the node is not under the target point',
          math.hypot(origin[0] - target['u'], origin[1] - target['v']) > 0.05, (origin, target))
    down = press({'u': origin[0], 'v': origin[1]})
    check('a press on the centre handle opens a drag and writes nothing',
          down['dragging'] is True and down['hit']['handle'] == 'screen' and history() == base_history
          and bundle() == base_bundle, down)
    moved = ok({'op': 'scene.pointer.move', 'point': target})
    check('a move is a candidate: the saved document and the history are untouched',
          moved['applied'] is True and moved['document_changed'] is False and bundle() == base_bundle
          and history() == base_history, moved)
    check('the overlay of the move follows the candidate and says which handle is active',
          moved['overlay']['active']['handle'] == 'screen', moved['overlay'])
    if surface.name == 'native':
        native_candidate_view(surface, check, record)
    up = ok({'op': 'scene.pointer.up', 'point': target})
    check('the release is one undo entry', up['document_changed'] is True and history()[0] == base_history[0] + 1, up)
    landed = projected_hero()
    check('the node now projects under the pointer (independent pinhole, plan read after the commit)',
          abs(landed[0] - target['u']) < TOLERANCE and abs(landed[1] - target['v']) < TOLERANCE, (landed, target))
    check('the gizmo stands where the node is: no drag, no gesture left open',
          up['overlay']['active'] is None and ok({'op': 'scene.gesture.status'})['active'] is False, up['overlay'])
    dragged_bundle = bundle()
    dragged_position = node(dragged_bundle, 'hero')['localPosition']
    check('only the hero position changed in the saved document',
          dragged_position != node(base_bundle, 'hero')['localPosition']
          and {k: v for k, v in node(dragged_bundle, 'hero').items() if k != 'localPosition'}
          == {k: v for k, v in node(base_bundle, 'hero').items() if k != 'localPosition'}
          and node(dragged_bundle, 'sky') == node(base_bundle, 'sky'), dragged_position)
    if surface.name == 'native':
        native_committed_view(surface, check, record)
    undone = ok({'op': 'history.undo'})
    check('one undo restores the saved document exactly',
          undone['changed'] is True and bundle() == base_bundle and history()[0] == base_history[0], undone)
    if surface.name == 'native':
        native_undone_view(surface, check, record)
    base_history = history()  # an undo is a revision of its own: the settled baseline

    # ---- the same result without a pointer ------------------------------------------------------------------
    translated = ok({'op': 'scene.translate', 'scene': SCENE, 'entity': HERO, 'frame': FRAME,
                     'delta': up['world_delta']})
    check('scene.translate with the drag\'s world delta is one entry',
          translated['changed'] is True and history()[0] == base_history[0] + 1, translated)
    check('and it leaves the same saved document as the drag did',
          bundle() == dragged_bundle, node(bundle(), 'hero')['localPosition'])
    ok({'op': 'history.undo'})
    check('undo restores it', bundle() == base_bundle)
    base_history = history()
    record['world_delta'] = up['world_delta']
    record['position'] = dragged_position

    # ---- an axis drag does not leak into the other axes ---------------------------------------------------------
    shown = overlay()
    tip = handle(shown, 'x')['to']
    press(tip)
    axis_move = ok({'op': 'scene.pointer.move', 'point': {'u': tip['u'] + 0.05, 'v': tip['v']}})
    delta = axis_move['world_delta']
    check('an x drag moves along world x only', delta['x'] > 0.05 and abs(delta['y']) < 1e-9 and abs(delta['z']) < 1e-9, delta)
    ok({'op': 'scene.pointer.cancel', 'reason': 'api'})
    untouched('the x drag was only a candidate', base_bundle, base_history)

    # ---- zero deltas are no-ops ------------------------------------------------------------------------------
    zero = ok({'op': 'scene.translate', 'scene': SCENE, 'entity': HERO, 'frame': FRAME, 'delta': {'x': 0, 'y': 0, 'z': 0}})
    check('a zero translate is a no-op: no entry, no revision', zero['changed'] is False, zero)
    untouched('zero translate', base_bundle, base_history)
    press({'u': origin[0], 'v': origin[1]})
    released = ok({'op': 'scene.pointer.up'})
    check('a press released in place is not an edit', released['document_changed'] is False, released)
    untouched('press released in place', base_bundle, base_history)

    # ---- every cancel reason leaves no trace --------------------------------------------------------------------
    for reason in ('escape', 'blur', 'pointer_cancel', 'hidden', 'unavailable', 'api'):
        press({'u': origin[0], 'v': origin[1]})
        ok({'op': 'scene.pointer.move', 'point': target})
        cancelled = ok({'op': 'scene.pointer.cancel', 'reason': reason})
        check(f'cancel `{reason}` ends the drag', cancelled['reason'] == reason and cancelled['dragging'] is False, cancelled)
        untouched(f'cancel `{reason}`', base_bundle, base_history)
    quiet = ok({'op': 'scene.pointer.cancel', 'reason': 'escape'})
    check('Escape with nothing to cancel is not an error', quiet.get('noop') is True, quiet)

    # ---- refusals change nothing ---------------------------------------------------------------------------------
    for name, request, code in (
        ('a press outside the picture', {'op': 'scene.pointer.down', 'scene': SCENE, 'frame': FRAME,
                                         'point': {'u': 1.2, 'v': 0.5}}, 'invalid_value'),
        ('a move with no drag', {'op': 'scene.pointer.move', 'point': {'u': 0.5, 'v': 0.5}}, 'no_drag'),
        ('a translate of an unknown node', {'op': 'scene.translate', 'scene': SCENE, 'frame': FRAME,
                                            'entity': 'scene/scene-a/node/ghost', 'delta': {'x': 1, 'y': 0, 'z': 0}},
         'unknown_entity'),
        ('a translate of a camera clip', {'op': 'scene.translate', 'scene': SCENE, 'frame': FRAME,
                                          'entity': 'camera/cam-main', 'delta': {'x': 1, 'y': 0, 'z': 0}},
         'not_translatable'),
        ('a translate whose delta is not a number', {'op': 'scene.translate', 'scene': SCENE, 'frame': FRAME,
                                                      'entity': HERO, 'delta': {'x': 'NaN', 'y': 0, 'z': 0}},
         'invalid_request'),
        ('an unknown cancel reason', {'op': 'scene.pointer.cancel', 'reason': 'because'}, 'invalid_value'),
    ):
        error = refused(request)
        check(f'refused: {name} ({code})', error['code'] == code, error)
        untouched(name, base_bundle, base_history)

    # ---- an edit under an open drag: the old candidate never lands ------------------------------------------------
    press({'u': origin[0], 'v': origin[1]})
    ok({'op': 'scene.pointer.move', 'point': target})
    ok({'op': 'scene.edit', 'scene': SCENE, 'ops': [{'set': f'{SKY}/renderOrder', 'value': 3}]})
    edited_bundle, edited_history = bundle(), history()
    check('the edit under the drag is the only change', node(edited_bundle, 'sky')['renderOrder'] == 3
          and node(edited_bundle, 'hero') == node(base_bundle, 'hero'), edited_history)
    if surface.name == 'native':
        # The window settles a dead drag by itself (the adapter cancels it after the edit), so the drag is
        # gone before the move arrives; the core still refuses the move without writing.
        settled = wait_for(lambda: surface.get('monitor.gizmo')['data']['state']['dragging'] is False, 10)
        check('the window settled the dead drag by itself', bool(settled), surface.get('monitor.gizmo'))
        expected = {'no_drag', 'gesture_invalidated'}
    else:
        expected = {'gesture_invalidated'}
    stale = refused({'op': 'scene.pointer.move', 'point': {'u': target['u'] + 0.01, 'v': target['v']}})
    check('a move after the edit is refused with a typed code', stale['code'] in expected, stale)
    ok({'op': 'scene.pointer.cancel', 'reason': 'api'})
    check('the old candidate never landed', bundle() == edited_bundle, node(bundle(), 'hero')['localPosition'])
    check('and the cancel left history and revision as the edit made them', history() == edited_history,
          (history(), edited_history))
    check('and nothing is left open', ok({'op': 'scene.gesture.status'})['active'] is False)
    ok({'op': 'history.undo'})
    check('one undo takes the unrelated edit back', bundle() == base_bundle)

    return {'base_bundle': base_bundle, 'ok': ok, 'refused': refused, 'saved': saved, 'bundle': bundle,
            'history': history, 'overlay': overlay, 'origin': origin, 'press': press, 'node': node}


# ---- what only a window has -----------------------------------------------------------------------------------

def wait_for(predicate, seconds=30, step=0.05):
    deadline = time.time() + seconds
    while time.time() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(step)
    return None


def picture_hash(surface):
    return (surface.viewport().get('last_frame') or {}).get('hash')


def native_initial(surface, check, record):
    """The page negotiated the overlay, drew the handles (pixels read back) and the monitor has a picture."""
    state = surface.viewport()
    overlay = state.get('overlay') or {}
    check('the page announced `overlay.gizmo` and the overlay is on for the shown scene',
          overlay.get('supported') is True and overlay.get('enabled') is True, overlay)
    drawn = wait_for(lambda: (surface.viewport().get('overlay') or {}).get('drawn'))
    check('the page acknowledged what it drew', drawn is not None, surface.viewport().get('overlay'))
    record['initial_hash'] = picture_hash(surface)
    check('the monitor has a picture to compare with', bool(record['initial_hash']), surface.viewport().get('last_frame'))


def native_selected(surface, check, record):
    """After the selection: the canvas really holds the handles (colours at the middle of each handle)."""
    def drawn_selected():
        drawn = (surface.viewport().get('overlay') or {}).get('drawn') or {}
        return drawn if drawn.get('visible') is True and drawn.get('entity') == HERO else None
    drawn = wait_for(drawn_selected)
    check('the page drew the selected node\'s gizmo', drawn is not None, surface.viewport().get('overlay'))
    probes = {probe['id']: probe['rgba'] for probe in drawn['probes']}
    x, y = probes['x'], probes['y']
    check('the x axis is painted red and the y axis green on the canvas (alpha > 200)',
          x[3] > 200 and x[0] > x[1] and x[0] > x[2] and y[3] > 200 and y[1] > y[0], probes)
    record['drawn_probes'] = probes
    gizmo = surface.get('monitor.gizmo')['data']['state']
    check('the semantic node `monitor.gizmo` reports the selection and no drag',
          gizmo['selection'] == 1 and gizmo['dragging'] is False, gizmo)


def native_candidate_view(surface, check, record):
    """During the drag: the page draws the active handle and the monitor shows the CANDIDATE picture."""
    active = wait_for(lambda: ((surface.viewport().get('overlay') or {}).get('drawn') or {}).get('active') == 'screen')
    check('the page drew the active handle of the drag', bool(active), surface.viewport().get('overlay'))
    gizmo = surface.get('monitor.gizmo')['data']['state']
    check('`monitor.gizmo` reports the drag (handle, gesture)',
          gizmo['dragging'] is True and gizmo['handle'] == 'screen' and gizmo['gesture'].startswith('gesture-'), gizmo)
    changed = wait_for(lambda: picture_hash(surface) not in (None, record['initial_hash']))
    check('the monitor\'s picture follows the candidate (its hash left the initial one)', bool(changed),
          (picture_hash(surface), record['initial_hash']))
    settle_converged(surface)
    record['candidate_hash'] = picture_hash(surface)


def settle_converged(surface):
    return surface.lifecycle.settle(surface.lab, FRAME, 60)


def native_committed_view(surface, check, record):
    settled = settle_converged(surface)
    check('after the release the monitor converged on the committed document', settled is not None, surface.viewport().get('directed'))
    check('the committed picture is the candidate picture the user saw', picture_hash(surface) == record['candidate_hash'],
          (picture_hash(surface), record['candidate_hash']))
    gizmo = surface.get('monitor.gizmo')['data']['state']
    check('`monitor.gizmo` reports no drag after the release', gizmo['dragging'] is False and gizmo['gesture'] == '', gizmo)


def native_undone_view(surface, check, record):
    settled = settle_converged(surface)
    check('after undo the monitor converged', settled is not None, surface.viewport().get('directed'))
    check('and shows the initial picture again', picture_hash(surface) == record['initial_hash'],
          (picture_hash(surface), record['initial_hash']))


def native_dialog(surface, check, record, context):
    """A dialog hides the WebView (it would paint over it); a hidden page gets no release, so the drag ends."""
    ok, bundle, history, press = context['ok'], context['bundle'], context['history'], context['press']
    base_bundle, base_history, origin = context['base_bundle'], history(), context['origin']
    press({'u': origin[0], 'v': origin[1]})
    ok({'op': 'scene.pointer.move', 'point': {'u': origin[0] + 0.05, 'v': origin[1]}})
    check('a drag is open', surface.get('monitor.gizmo')['data']['state']['dragging'] is True)
    opened = surface.invoke('flow.open')
    check('the Open dialog opens (a native view would paint above it)', opened.get('ok') is True, opened)
    hidden = wait_for(lambda: surface.viewport().get('on_screen') is False)
    check('the WebView is hidden while the dialog is open', bool(hidden), surface.viewport())
    gizmo = surface.get('monitor.gizmo')['data']['state']
    check('the open drag was cancelled by the hide (never committed)', gizmo['dragging'] is False, gizmo)
    check('the overlay is switched off in the page', (surface.viewport().get('overlay') or {}).get('enabled') is False,
          surface.viewport().get('overlay'))
    check('the document and the history are as they were', bundle() == base_bundle and history() == base_history)
    closed = surface.invoke('dialog.cancel')
    check('the dialog closes', closed.get('ok') is True, closed)
    shown = wait_for(lambda: surface.viewport().get('on_screen') is True
                     and (surface.viewport().get('overlay') or {}).get('enabled') is True)
    check('the WebView and the overlay are back', bool(shown), surface.viewport())
    check('the gizmo is drawn again for the selection', bool(wait_for(
        lambda: ((surface.viewport().get('overlay') or {}).get('drawn') or {}).get('visible') is True)))
    record['stale_messages'] = (surface.viewport().get('overlay') or {}).get('stale')


class Deps:
    """What `main` talks to. The real ones by default; a test swaps in fakes (no GPU, no editor, no repository)."""

    def __init__(self, **overrides):
        self.factories = {'headless': Headless, 'native': Native}
        self.scenario = scenario
        self.dialog = native_dialog
        self.required = REQUIRED
        self.free_gib = lambda: shutil.disk_usage('/tmp').free / 1024 ** 3
        self.tree_status = tree_status
        self.head = git_head
        self.__dict__.update(overrides)


def stop_surface(surface):
    """Ends the surface's processes; the error (if any) as text. A failed stop never skips the kill."""
    if surface is None:
        return None
    try:
        surface.stop()
        return None
    except BaseException as error:
        try:
            surface.kill()
        except BaseException:
            pass
        return describe(error)


def run_surface(name, binary, report, deps):
    work = tempfile.mkdtemp(prefix='te-s8-', dir='/tmp')
    scratch = Path(work) / 'scratch'
    scratch.mkdir()
    check = Recorder(name)
    record = {}
    # `retained: true` until the run passed: the work directory (editor log, headless stderr, saved documents) is
    # what explains a failure and is never deleted on one.
    entry = report[name] = {'checks': check.checks, 'work': work, 'retained': True, 'record': record}
    surface = None
    succeeded = False
    try:
        surface = deps.factories[name](binary, work)
        surface.prepare()
        if name == 'native':
            # The overlay checks run at the points of the scenario where they belong (see `scenario`).
            original = surface.op

            def wrapped(request):
                result = original(request)
                if request.get('op') == 'monitor.show_scene' and result[0]:
                    native_initial(surface, check, record)
                if request.get('op') == 'scene.select' and result[0]:
                    native_selected(surface, check, record)
                return result
            surface.op = wrapped
        context = deps.scenario(surface, check, scratch, record)
        if name == 'native':
            if deps.free_gib() < MIN_FREE_GIB:
                raise ProofFailure('free space fell below the floor')
            deps.dialog(surface, check, record, context)
        missing = missing_checks(name, check.checks, deps.required)
        if missing:
            raise ProofFailure(f'[{name}] required checks did not run ({len(check.checks)} ran): {missing}')
        succeeded = True
    except BaseException as failure:
        entry['failure'] = describe(failure)
        if surface is not None and hasattr(surface, 'log_tail'):
            entry['log_tail'] = surface.log_tail()
        raise
    finally:
        stop_error = stop_surface(surface)
        entry['passed'] = len(check.checks)
        if stop_error:
            entry['cleanup_error'] = stop_error
        if succeeded and not stop_error:
            shutil.rmtree(work, ignore_errors=True)
            entry['retained'] = False
    if stop_error:
        raise RuntimeError(f'[{name}] the surface did not stop cleanly (work kept at {work}): {stop_error}')


def main(argv=None, deps=None):
    deps = deps or Deps()
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--editor', default=str(ROOT / 'target/debug/tessel-editor'))
    parser.add_argument('--headless', default=str(ROOT / 'target/debug/tessel-headless'))
    parser.add_argument('--out')
    parser.add_argument('--surfaces', default=','.join(SURFACES),
                        help='development only: a subset (never final evidence)')
    parser.add_argument('--allow-dirty', action='store_true',
                        help='skip the clean-tree requirement (development only; never final evidence)')
    args = parser.parse_args(argv)
    out = Path(args.out) if args.out else None
    report = {
        'ok': False, 'final_evidence': False, 'error': 'the run did not finish',
        'input': 'semantic requests only (JSONL for headless; the direct binary\'s socket `op`/`get`/`invoke` for native); '
                 'picture coordinates (u, v), no screen coordinates, no physical pointer',
        'not_proven': ['a PHYSICAL pointer or a native drag on the WKWebView (BL043/BL044)',
                       'a screen capture of the gizmo over the WKWebView',
                       'the page\'s own mouse handling (proved in headless Chrome by runtime/test/gizmo-overlay.test.mjs)'],
    }
    # An old report is gone before anything else runs: a crash can never leave a previous `ok: true` standing.
    if out is not None:
        out.unlink(missing_ok=True)
    write_report(out, report)
    code = 1
    interrupted = None
    try:
        wanted = parse_surfaces(args.surfaces)
        report['surfaces'] = wanted
        clean, status = deps.tree_status()
        report['head'] = deps.head()
        report['tree_clean'] = clean
        report['binaries'] = {name: {'path': str(Path(path).resolve()), 'sha256': sha256(path)}
                              for name, path in (('tessel-editor', args.editor), ('tessel-headless', args.headless))
                              if Path(path).exists()}
        if deps.free_gib() < MIN_FREE_GIB:
            raise RuntimeError(f'only {deps.free_gib():.1f} GiB free, need {MIN_FREE_GIB}')
        if not clean and not args.allow_dirty:
            raise RuntimeError(f'the tree differs from HEAD (modified or untracked); commit first:\n{status}')
        report['min_free_gib_seen'] = round(deps.free_gib(), 1)
        binaries = {'headless': args.headless, 'native': args.editor}
        for name in wanted:
            run_surface(name, binaries[name], report, deps)
        if wanted == list(SURFACES):
            head_record, native_record = report['headless']['record'], report['native']['record']
            same = all(abs(head_record['world_delta'][axis] - native_record['world_delta'][axis]) < 1e-12 for axis in 'xyz')
            report['surfaces_agree_on_the_drag'] = same
            if not same:
                raise ProofFailure('headless and native computed different world deltas for the same drag')
        report['min_free_gib_seen'] = round(min(report['min_free_gib_seen'], deps.free_gib()), 1)
        blockers = final_blockers(wanted, args.allow_dirty, clean)
        report['final_evidence_blockers'] = blockers
        report['final_evidence'] = not blockers
        report['ok'] = True
        report.pop('error', None)
        code = 0
    except BaseException as failure:
        report['ok'] = False
        report['final_evidence'] = False
        report['error'] = describe(failure)
        if not isinstance(failure, (ProofFailure, RuntimeError)):
            report['aborted'] = True
            report['traceback'] = traceback.format_exc()[-4000:]
        if not isinstance(failure, Exception):
            interrupted = failure  # Ctrl-C and SystemExit still end the process, after the report is written
        print(f'FAILED: {describe(failure)}', flush=True)
    finally:
        write_report(out, report)
    summary = {key: value for key, value in report.items() if key not in SURFACES and key != 'traceback'}
    for surface in SURFACES:
        if surface in report:
            summary[surface] = {'checks': len(report[surface]['checks']), 'failure': report[surface].get('failure'),
                                'work': report[surface]['work'], 'retained': report[surface]['retained']}
    print(json.dumps(summary, indent=2))
    if interrupted is not None:
        raise interrupted
    return code


if __name__ == '__main__':
    raise SystemExit(main())
