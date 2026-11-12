#!/usr/bin/env python3
"""F055 / BL015.1 (G1) — the rotate tool of the gizmo, proved on two separate surfaces with one scenario:

    python3 scripts/scene-gizmo-rotation-proof.py [--editor target/debug/tessel-editor] \
        [--headless target/debug/tessel-headless] [--out report.json] [--allow-dirty]

Functional proof only (no timing is measured), so it does not take the GPU lock: that lock is reserved for
performance validations (AGENTS.md).

The surfaces and their mechanics are the ones of `scene-gizmo-proof.py` (S8), loaded from it, not copied:

* **headless (no window)**: `tessel-headless --empty`, JSON Lines over stdio. No GPUI, no WebView.
* **native window**: the DIRECT editor binary with its own short `HOME`, driven only by its `ui` socket
  (`op`, `get`, `invoke`). The user's window and project are never touched.

The scenario, in semantic requests on the corpus bundle (animated: `stage` slides, `hero` sways), at a frame where
the stage has slid. Every expectation is computed HERE (Rodrigues, a pinhole, a ray against a plane), never by the core:

* the tool is session state (`scene.gizmo.tool`): not an edit, refused to change under an open drag;
* the rings are circles around the node's origin: every polygon vertex of the overlay, un-projected onto the plane of
  its ring, is at one distance from the origin and in that plane;
* a ring drag ends with `W' = G · W` (G the rotation about the world axis by the angle the pointer swept; the node's
  origin does not move), as ONE undo entry; the candidate is not the document until the release (read back through
  `project.save`);
* `scene.rotate` with the drag's axis, angle and space leaves the same saved document; undo restores it exactly;
* the local space turns about the node's own axes (a different result than the world one after a first turn);
* zero and whole turns, every cancel reason and every typed refusal (an animated rotation, a parent that scales
  unevenly, a bad axis or angle) leave document, history and revision as they were.

The native run adds what only a window has: the page negotiates `overlay.gizmo`, draws the rings (pixel probes read
back from the canvas), the monitor's picture follows the candidate and is the committed picture after the release
and the initial one after undo (hashes).

Report promises (as S8): the `--out` file is `ok: false` from the start; only a complete clean run turns it into
`ok: true`; a surface that did not run every required check fails; `final_evidence` only for both surfaces on a clean
tree without `--allow-dirty`.

Not proven here, on purpose: a PHYSICAL pointer or a native drag on the WKWebView (BL015.2, BL043/BL044). The
`scene.pointer.*` requests carry picture coordinates; a synthetic request is not a mouse. The page's own mouse
handling of the rings is proved with real mouse events in headless Chrome by `runtime/test/gizmo-overlay.test.mjs`.
"""
import argparse
import importlib.util
import json
import math
import os
from pathlib import Path
import shutil
import sys
import tempfile
import traceback

ROOT = Path(__file__).resolve().parent.parent


def load_s8():
    spec = importlib.util.spec_from_file_location('scene_gizmo_proof', ROOT / 'scripts' / 'scene-gizmo-proof.py')
    module = importlib.util.module_from_spec(spec)
    sys.modules['scene_gizmo_proof'] = module
    spec.loader.exec_module(module)
    return module


S8 = load_s8()
SCENE = S8.SCENE
BUNDLE = S8.BUNDLE
CORPUS = S8.CORPUS
FRAME = S8.FRAME
MID = 'scene/scene-a/node/mid'
HERO = S8.HERO
ROOT_NODE = 'scene/scene-a/node/root'
SURFACES = S8.SURFACES
TOLERANCE = 1e-9
ANGLE_TOLERANCE = 1e-6
RING_RADIUS = 0.10  # picture heights, the contract's number (read back from `metrics.ring_radius` and compared)

_COMMON = (
    'the corpus bundle loads',
    'the rotate tool is chosen and is not an edit',
    'a selected node has three rings of 65 points each',
    'every ring vertex un-projects onto its ring plane at one distance from the origin',
    'a ring drag is one undo entry and leaves W\' = G · W',
    'scene.rotate with the drag\'s axis, angle and space leaves the same saved document',
    'the local space turns about the node\'s own axis, not the world\'s',
    'a rotation channel on the node is refused as `animated_rotation` and nothing is keyed',
    'a parent that scales unevenly is refused as `non_uniform_parent_scale`',
    'the tool cannot change under an open drag',
    'the translate tool is what it was',
)
REQUIRED = {
    'headless': _COMMON,
    'native': _COMMON + (
        'the page announced `overlay.gizmo` and drew the rings',
        'the page painted each ring in its axis colour',
        'the monitor\'s picture follows the candidate and returns to the initial one after undo',
    ),
}


# ---- the independent oracle: nothing here is the core's code -------------------------------------------------

def rodrigues(axis, radians):
    s, c = math.sin(radians), math.cos(radians)
    k = 1 - c
    x, y, z = axis
    return [
        [c + x * x * k, x * y * k - z * s, x * z * k + y * s],
        [y * x * k + z * s, c + y * y * k, y * z * k - x * s],
        [z * x * k - y * s, z * y * k + x * s, c + z * z * k],
    ]


def matmul(a, b):
    return [[sum(a[r][k] * b[k][c] for k in range(3)) for c in range(3)] for r in range(3)]


def apply(m, v):
    return [sum(m[r][k] * v[k] for k in range(3)) for r in range(3)]


def linear_of(matrix):
    return [[matrix[0], matrix[4], matrix[8]], [matrix[1], matrix[5], matrix[9]], [matrix[2], matrix[6], matrix[10]]]


def close(a, b, tolerance=TOLERANCE):
    return all(abs(x - y) <= tolerance for row_a, row_b in zip(a, b) for x, y in zip(row_a, row_b))


def eye_ray(camera, output, u, v):
    """The world ray `(origin, direction)` through a picture point (the inverse of `S8.project`)."""
    position = [camera['position'][axis] for axis in 'xyz']
    target = [camera['target'][axis] for axis in 'xyz']
    forward = S8.unit(S8.sub(target, position))
    right = S8.unit(S8.cross(forward, [0, 1, 0]))
    up = S8.cross(right, forward)
    half = math.tan(math.radians(camera['fovDeg']) / 2)
    aspect = output['width'] / output['height']
    x = (u * 2 - 1) * half * aspect
    y = (1 - v * 2) * half
    return position, [forward[i] + right[i] * x + up[i] * y for i in range(3)]


def on_plane(camera, output, point, origin, normal):
    position, ray = eye_ray(camera, output, point['u'], point['v'])
    along = S8.dot(S8.sub(origin, position), normal) / S8.dot(ray, normal)
    return [position[i] + ray[i] * along for i in range(3)]


def pointer_for(camera, output, grabbed, origin, axis, degrees):
    """Where the pointer must be so that the grabbed point of the ring plane has turned by `degrees` about `axis`."""
    held = on_plane(camera, output, grabbed, origin, axis)
    turned = apply(rodrigues(axis, math.radians(degrees)), S8.sub(held, origin))
    u, v = S8.project(camera, output, [origin[i] + turned[i] for i in range(3)])
    return {'u': u, 'v': v}


def scenario(surface, check, scratch, record):
    def ok(request):
        good, payload = surface.op(request)
        if not good:
            raise S8.ProofFailure(f'{request} -> {payload}')
        return payload

    def refused(request):
        good, payload = surface.op(request)
        if good:
            raise S8.ProofFailure(f'{request} should have been refused, got {payload}')
        return payload

    saves = [0]

    def bundle():
        saves[0] += 1
        path = scratch / f'doc-{saves[0]}.tessel.json'
        ok({'op': 'project.save', 'path': str(path)})
        return json.loads(path.read_text())['scenes']['items'][0]['bundle']

    def history():
        data = ok({'op': 'history.status'})
        return data['history']['undo_depth'], data['project']['revision']

    def plan_at(frame=FRAME):
        return ok({'op': 'scene.plan', 'scene': SCENE, 'frame': frame})['plan']

    def world(node_id, frame=FRAME):
        found = next(n for n in plan_at(frame)['nodes'] if n['id'] == node_id)
        matrix = found['worldMatrix']
        return linear_of(matrix), [matrix[12], matrix[13], matrix[14]]

    def overlay(frame=FRAME):
        return ok({'op': 'scene.gizmo.state', 'scene': SCENE, 'frame': frame})['overlay']

    def untouched(label, base_bundle, base_history):
        check(f'{label}: document, undo depth and revision are as they were',
              bundle() == base_bundle and history() == base_history, (history(), base_history))

    def tool(name, space='world'):
        return ok({'op': 'scene.gizmo.tool', 'tool': name, 'space': space})

    def rotate(entity, axis, degrees, space):
        return ok({'op': 'scene.rotate', 'scene': SCENE, 'entity': entity, 'frame': FRAME, 'axis': axis,
                   'degrees': degrees, 'space': space})

    if surface.name == 'native':
        ok({'op': 'project.create', 'name': 'G1 proof', 'sequence': 'main', 'rate': 24, 'width': 320, 'height': 180,
            'length': 96, 'dir': str(scratch / 'workspace')})
    attached = ok({'op': 'scene.attach', 'id': SCENE, 'path': str(BUNDLE)})
    check('the corpus bundle loads', attached.get('scene') == SCENE, attached)
    if surface.name == 'native':
        loaded = ok({'op': 'scene.resources.resolve', 'scene': SCENE, 'base': str(CORPUS)})
        check('the resources are resolved and applied', surface.lifecycle.commit_job(surface.lab, loaded, 'g1'), loaded)
        ok({'op': 'monitor.show_scene', 'scene': SCENE})
        surface.lab.seek(FRAME)
        check('the monitor converged on the animating frame', surface.lifecycle.settle(surface.lab, FRAME, 60) is not None,
              surface.viewport().get('directed'))
        record['initial_hash'] = S8.picture_hash(surface)
        check('the monitor has a picture at the animating frame', bool(record['initial_hash']), surface.viewport())

    base_bundle, base_history = bundle(), history()
    ok({'op': 'scene.select', 'scene': SCENE, 'entities': [MID]})
    before_tool = (bundle(), history())
    chosen = tool('rotate', 'world')
    check('the rotate tool is chosen and is not an edit',
          chosen['tool'] == 'rotate' and chosen['space'] == 'world' and chosen['changed'] is True
          and (bundle(), history()) == before_tool, chosen)

    # ---- the rings ---------------------------------------------------------------------------------------------
    shown = overlay()
    plan = plan_at()
    camera, output = plan['camera'], plan['output']
    linear_before, origin = world('mid')
    check('a selected node has three rings of 65 points each',
          shown['tool'] == 'rotate' and shown['visible'] is True and [h['id'] for h in shown['handles']] == ['x', 'y', 'z']
          and all(h['kind'] == 'ring' and len(h['points']) == 65 for h in shown['handles']), shown)
    radius = RING_RADIUS * 2 * math.tan(math.radians(camera['fovDeg']) / 2) * S8.dot(
        S8.sub(origin, [camera['position'][a] for a in 'xyz']),
        S8.unit(S8.sub([camera['target'][a] for a in 'xyz'], [camera['position'][a] for a in 'xyz'])))
    check('the contract\'s ring radius is the one the overlay reports', shown['metrics']['ring_radius'] == RING_RADIUS,
          shown['metrics'])
    axes = {'x': [1, 0, 0], 'y': [0, 1, 0], 'z': [0, 0, 1]}
    worst = 0.0
    for ring in shown['handles']:
        normal = axes[ring['id']]
        for point in ring['points']:
            if point is None:
                continue
            at = on_plane(camera, output, point, origin, normal)
            offset = S8.sub(at, origin)
            worst = max(worst, abs(S8.dot(offset, offset) ** 0.5 - radius))
    check('every ring vertex un-projects onto its ring plane at one distance from the origin', worst < 1e-8, worst)

    # ---- a drag of the z ring ------------------------------------------------------------------------------------
    z_ring = next(h for h in shown['handles'] if h['id'] == 'z')
    grabbed = z_ring['points'][8]
    degrees = 40.0
    target = pointer_for(camera, output, grabbed, origin, axes['z'], degrees)
    down = ok({'op': 'scene.pointer.down', 'scene': SCENE, 'frame': FRAME, 'point': grabbed})
    check('a press on a ring opens a drag and writes nothing',
          down['dragging'] is True and down['hit']['handle'] == 'z' and down['tool'] == 'rotate'
          and bundle() == base_bundle and history() == base_history, down)
    moved = ok({'op': 'scene.pointer.move', 'point': target})
    check('a move is a candidate: the saved document and the history are untouched',
          moved['applied'] is True and moved['document_changed'] is False and bundle() == base_bundle
          and history() == base_history, moved)
    if surface.name == 'native':
        native_candidate(surface, check, record)
    refused_tool = refused({'op': 'scene.gizmo.tool', 'tool': 'translate'})
    check('the tool cannot change under an open drag',
          refused_tool['code'] == 'gesture_active' and surface.op({'op': 'scene.gizmo.tool'})[1]['tool'] == 'rotate',
          refused_tool)
    up = ok({'op': 'scene.pointer.up', 'point': target})
    swept = up['degrees']
    linear_after, after_origin = world('mid')
    expected = matmul(rodrigues(axes['z'], math.radians(swept)), linear_before)
    check('a ring drag is one undo entry and leaves W\' = G · W',
          up['document_changed'] is True and history()[0] == base_history[0] + 1 and abs(swept - degrees) < ANGLE_TOLERANCE
          and close(linear_after, expected) and all(abs(a - b) < TOLERANCE for a, b in zip(after_origin, origin)),
          (up, swept))
    dragged_bundle = bundle()
    if surface.name == 'native':
        native_committed(surface, check, record)
    ok({'op': 'history.undo'})
    check('one undo restores the saved document exactly', bundle() == base_bundle)
    if surface.name == 'native':
        native_undone(surface, check, record)
    base_history = history()

    rotated = rotate(MID, 'z', swept, 'world')
    check('scene.rotate with the drag\'s axis, angle and space leaves the same saved document',
          rotated['changed'] is True and bundle() == dragged_bundle and rotated['rotation'] == up['rotation'], rotated)
    ok({'op': 'history.undo'})
    base_history = history()

    # ---- local versus world ----------------------------------------------------------------------------------------
    rotate(MID, 'y', 90, 'world')
    quarter_bundle = bundle()
    linear_quarter, _ = world('mid')
    rotate(MID, 'x', 30, 'local')
    local_after, _ = world('mid')
    ok({'op': 'history.undo'})
    check('undo of the local turn leaves the quarter turn', bundle() == quarter_bundle)
    rotate(MID, 'x', 30, 'world')
    world_after, _ = world('mid')
    own_x = [row[0] for row in linear_quarter]
    own_x_unit = S8.unit(own_x)
    check('the local space turns about the node\'s own axis, not the world\'s',
          close(local_after, matmul(rodrigues(own_x_unit, math.radians(30)), linear_quarter))
          and close(world_after, matmul(rodrigues([1, 0, 0], math.radians(30)), linear_quarter))
          and not close(local_after, world_after, 1e-3), (local_after, world_after))
    ok({'op': 'history.undo'})
    ok({'op': 'history.undo'})
    base_bundle, base_history = bundle(), history()

    # ---- no-ops, cancels -----------------------------------------------------------------------------------------
    for degrees_ in (0, 360, -360, 720):
        answer = rotate(MID, 'z', degrees_, 'world')
        check(f'{degrees_} degrees is a no-op', answer['changed'] is False, answer)
        untouched(f'{degrees_} degrees', base_bundle, base_history)
    shown = overlay()
    grabbed = next(h for h in shown['handles'] if h['id'] == 'z')['points'][8]
    for reason in ('escape', 'blur', 'pointer_cancel', 'hidden', 'unavailable', 'api'):
        ok({'op': 'scene.pointer.down', 'scene': SCENE, 'frame': FRAME, 'point': grabbed})
        ok({'op': 'scene.pointer.move', 'point': pointer_for(camera, output, grabbed, origin, axes['z'], 25)})
        cancelled = ok({'op': 'scene.pointer.cancel', 'reason': reason})
        check(f'cancel `{reason}` ends the ring drag', cancelled['reason'] == reason and cancelled['dragging'] is False, cancelled)
        untouched(f'cancel `{reason}`', base_bundle, base_history)

    # ---- typed refusals ----------------------------------------------------------------------------------------------
    for name, request, code in (
        ('a screen axis', {'op': 'scene.rotate', 'scene': SCENE, 'entity': MID, 'frame': FRAME, 'axis': 'screen',
                           'degrees': 10}, 'invalid_value'),
        ('an angle that is not a number', {'op': 'scene.rotate', 'scene': SCENE, 'entity': MID, 'frame': FRAME,
                                           'axis': 'z', 'degrees': 'ten'}, 'invalid_request'),
        ('an unknown node', {'op': 'scene.rotate', 'scene': SCENE, 'entity': 'scene/scene-a/node/ghost',
                             'frame': FRAME, 'axis': 'z', 'degrees': 10}, 'unknown_entity'),
        ('a camera clip', {'op': 'scene.rotate', 'scene': SCENE, 'entity': 'camera/cam-main', 'frame': FRAME,
                           'axis': 'z', 'degrees': 10}, 'not_rotatable'),
        ('an unknown tool', {'op': 'scene.gizmo.tool', 'tool': 'resize'}, 'invalid_value'),
    ):
        error = refused(request)
        check(f'refused: {name} ({code})', error['code'] == code, error)
        untouched(name, base_bundle, base_history)

    tracks_before = bundle().get('performanceTracks')
    error = refused({'op': 'scene.rotate', 'scene': SCENE, 'entity': HERO, 'frame': FRAME, 'axis': 'z', 'degrees': 10})
    ok({'op': 'scene.select', 'scene': SCENE, 'entities': [HERO]})
    hidden = overlay()
    check('a rotation channel on the node is refused as `animated_rotation` and nothing is keyed',
          error['code'] == 'not_rotatable' and error['details']['reason'] == 'animated_rotation'
          and hidden['visible'] is False and hidden['reason'] == 'animated_rotation'
          and bundle().get('performanceTracks') == tracks_before, (error, hidden))
    untouched('the animated node', base_bundle, base_history)
    ok({'op': 'scene.select', 'scene': SCENE, 'entities': [MID]})

    ok({'op': 'scene.edit', 'scene': SCENE, 'ops': [{'set': f'{ROOT_NODE}/scale', 'value': {'x': 2, 'y': 1, 'z': 1}}]})
    uneven_bundle, uneven_history = bundle(), history()
    error = refused({'op': 'scene.rotate', 'scene': SCENE, 'entity': MID, 'frame': FRAME, 'axis': 'z', 'degrees': 10})
    hidden = overlay()
    check('a parent that scales unevenly is refused as `non_uniform_parent_scale`',
          error['code'] == 'not_rotatable' and error['details']['reason'] == 'non_uniform_parent_scale'
          and hidden['visible'] is False and hidden['reason'] == 'non_uniform_parent_scale'
          and bundle() == uneven_bundle and history() == uneven_history, (error, hidden))
    ok({'op': 'history.undo'})
    check('and one undo takes the parent edit back', bundle() == base_bundle)
    base_history = history()

    # ---- the old tool ----------------------------------------------------------------------------------------------
    tool('translate', 'world')
    arrows = overlay()
    tip = next(h for h in arrows['handles'] if h['id'] == 'x')['to']
    down = ok({'op': 'scene.pointer.down', 'scene': SCENE, 'frame': FRAME, 'point': tip})
    moved = ok({'op': 'scene.pointer.move', 'point': {'u': tip['u'] + 0.05, 'v': tip['v']}})
    ok({'op': 'scene.pointer.cancel', 'reason': 'api'})
    check('the translate tool is what it was',
          [h['kind'] for h in arrows['handles']] == ['axis', 'axis', 'axis', 'center'] and down['tool'] == 'translate'
          and moved['world_delta']['x'] > 0.05 and abs(moved['world_delta']['y']) < 1e-9
          and bundle() == base_bundle, (down, moved))
    record['rotation'] = up['rotation']
    record['degrees'] = swept


# ---- what only a window has --------------------------------------------------------------------------------------

def native_candidate(surface, check, record):
    state = surface.viewport()
    overlay = state.get('overlay') or {}
    drawn = S8.wait_for(lambda: ((surface.viewport().get('overlay') or {}).get('drawn') or {}).get('active') == 'z')
    check('the page announced `overlay.gizmo` and drew the rings',
          overlay.get('supported') is True and overlay.get('enabled') is True and bool(drawn)
          and ((surface.viewport().get('overlay') or {}).get('drawn') or {}).get('tool') == 'rotate',
          surface.viewport().get('overlay'))
    probes = {probe['id']: probe['rgba'] for probe in ((surface.viewport().get('overlay') or {}).get('drawn') or {}).get('probes', [])}
    x, y = probes.get('x'), probes.get('y')
    check('the page painted each ring in its axis colour',
          bool(x) and bool(y) and x[3] > 200 and x[0] > x[1] and y[3] > 200 and y[1] > y[0], probes)
    changed = S8.wait_for(lambda: S8.picture_hash(surface) not in (None, record['initial_hash']))
    record['picture_followed'] = bool(changed)
    S8.settle_converged(surface)
    record['candidate_hash'] = S8.picture_hash(surface)


def native_committed(surface, check, record):
    S8.settle_converged(surface)
    record['committed_hash'] = S8.picture_hash(surface)


def native_undone(surface, check, record):
    S8.settle_converged(surface)
    check('the monitor\'s picture follows the candidate and returns to the initial one after undo',
          record.get('picture_followed') is True and record['committed_hash'] == record['candidate_hash']
          and S8.picture_hash(surface) == record['initial_hash'],
          (record, S8.picture_hash(surface)))


def run_surface(name, binary, report):
    work = tempfile.mkdtemp(prefix='te-g1-', dir='/tmp')
    scratch = Path(work) / 'scratch'
    scratch.mkdir()
    check = S8.Recorder(name)
    record = {}
    entry = report[name] = {'checks': check.checks, 'work': work, 'retained': True, 'record': record}
    surface = None
    succeeded = False
    try:
        surface = S8.Headless(binary, work) if name == 'headless' else S8.Native(binary, work)
        surface.prepare()
        scenario(surface, check, scratch, record)
        missing = S8.missing_checks(name, check.checks, REQUIRED)
        if missing:
            raise S8.ProofFailure(f'[{name}] required checks did not run ({len(check.checks)} ran): {missing}')
        succeeded = True
    except BaseException as failure:
        entry['failure'] = S8.describe(failure)
        if surface is not None and hasattr(surface, 'log_tail'):
            entry['log_tail'] = surface.log_tail()
        raise
    finally:
        stop_error = S8.stop_surface(surface) if hasattr(S8, 'stop_surface') else None
        entry['passed'] = len(check.checks)
        if stop_error:
            entry['cleanup_error'] = stop_error
        if succeeded and not stop_error:
            shutil.rmtree(work, ignore_errors=True)
            entry['retained'] = False
    # Outside the `finally`, so it never hides the failure of the run itself (S8 does the same):
    # a surface that did not stop is a failed proof, never `ok` (BL015.20).
    if stop_error:
        raise RuntimeError(f'[{name}] the surface did not stop cleanly (work kept at {work}): {stop_error}')


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--editor', default=str(ROOT / 'target/debug/tessel-editor'))
    parser.add_argument('--headless', default=str(ROOT / 'target/debug/tessel-headless'))
    parser.add_argument('--out')
    parser.add_argument('--surfaces', default=','.join(SURFACES), help='development only: a subset (never final evidence)')
    parser.add_argument('--allow-dirty', action='store_true', help='development only; never final evidence')
    args = parser.parse_args(argv)
    out = Path(args.out) if args.out else None
    report = {
        'ok': False, 'final_evidence': False, 'error': 'the run did not finish',
        'input': 'semantic requests only (JSONL for headless; the direct binary\'s socket for native); picture '
                 'coordinates (u, v), no screen coordinates, no physical pointer',
        'not_proven': ['a PHYSICAL pointer or a native drag on the WKWebView (BL015.2, BL043/BL044)',
                       'the page\'s own mouse handling (runtime/test/gizmo-overlay.test.mjs)'],
    }
    if out is not None:
        out.unlink(missing_ok=True)
    S8.write_report(out, report)
    code = 1
    try:
        wanted = S8.parse_surfaces(args.surfaces)
        report['surfaces'] = wanted
        clean, status = S8.tree_status()
        report['head'] = S8.git_head()
        report['tree_clean'] = clean
        report['binaries'] = {name: {'path': str(Path(path).resolve()), 'sha256': S8.sha256(path)}
                              for name, path in (('tessel-editor', args.editor), ('tessel-headless', args.headless))
                              if Path(path).exists()}
        free = shutil.disk_usage('/tmp').free / 1024 ** 3
        if free < S8.MIN_FREE_GIB:
            raise RuntimeError(f'only {free:.1f} GiB free, need {S8.MIN_FREE_GIB}')
        if not clean and not args.allow_dirty:
            raise RuntimeError(f'the tree differs from HEAD; commit first:\n{status}')
        report['min_free_gib_seen'] = round(free, 1)
        binaries = {'headless': args.headless, 'native': args.editor}
        for name in wanted:
            run_surface(name, binaries[name], report)
        if wanted == list(SURFACES):
            h, n = report['headless']['record'], report['native']['record']
            same = h['rotation'] == n['rotation'] and abs(h['degrees'] - n['degrees']) < 1e-9
            report['surfaces_agree_on_the_drag'] = same
            if not same:
                raise S8.ProofFailure('headless and native computed different rotations for the same drag')
        blockers = S8.final_blockers(wanted, args.allow_dirty, clean)
        report['final_evidence_blockers'] = blockers
        report['final_evidence'] = not blockers
        report['ok'] = True
        report.pop('error', None)
        code = 0
    except BaseException as failure:
        report['ok'] = False
        report['final_evidence'] = False
        report['error'] = S8.describe(failure)
        if not isinstance(failure, (S8.ProofFailure, RuntimeError)):
            report['aborted'] = True
            report['traceback'] = traceback.format_exc()[-4000:]
        print(f'FAILED: {S8.describe(failure)}', flush=True)
    finally:
        S8.write_report(out, report)
    summary = {key: value for key, value in report.items() if key not in SURFACES and key != 'traceback'}
    for surface in SURFACES:
        if surface in report:
            summary[surface] = {'checks': len(report[surface]['checks']), 'failure': report[surface].get('failure'),
                                'work': report[surface]['work'], 'retained': report[surface]['retained']}
    print(json.dumps(summary, indent=2))
    return code


if __name__ == '__main__':
    raise SystemExit(main())
