#!/usr/bin/env python3
"""F055 / BL015.1 (G1.2) — the scale tool of the gizmo, proved on two separate surfaces with one scenario:

    python3 scripts/scene-gizmo-scale-proof.py [--editor target/debug/tessel-editor] \
        [--headless target/debug/tessel-headless] [--out report.json] [--allow-dirty]

Functional proof only (no timing is measured), so it does not take the GPU lock: that lock is reserved for
performance validations (AGENTS.md).

The surfaces and their mechanics are the ones of `scene-gizmo-proof.py` (S8), loaded from it, not copied:

* **headless (no window)**: `tessel-headless --empty`, JSON Lines over stdio. No GPUI, no WebView.
* **native window**: the DIRECT editor binary with its own short `HOME`, driven only by its `ui` socket
  (`op`, `get`, `invoke`). The user's window and project are never touched.

The scenario, in semantic requests on the corpus bundle (animated: `stage` slides, `hero` sways, `echo` has a
scale channel), at a frame where the stage has slid. Every expectation is computed HERE (a pinhole, matrix
products, `W·diag(d)` and `(I + (f-1)·a·aᵀ)·W`), never by the core:

* the tool is session state (`scene.gizmo.tool`): not an edit, refused to change under an open drag;
* the handles sit where the pinhole puts them: the tip of an axis handle is the node's origin plus the handle's
  length along the axis (the node's own axis in `local`, the scene's in `world`); the uniform box sits at a fixed
  picture offset up and to the right;
* a handle drag ends with the matrix the oracle computes (`W' = W·diag(d)` for a local axis, `G·W` for a world
  axis, `f·W` for the uniform handle; the node's origin does not move), as ONE undo entry; the candidate is not the
  document until the release (read back through `project.save`);
* `scene.scale` with the drag's axis, factor and space leaves the same saved document; undo restores it exactly;
* a rotated parent, an unevenly scaled parent and a mirroring parent change which direction a local axis points
  at, never whether it can be scaled; a world axis that no node axis runs along is disabled as `world_axis_shear`;
* factor 1, a factor of zero, a negative one, one past the limits, the screen handle, a scale channel on the
  node (`echo`), every cancel reason and every typed refusal leave document, history and revision as they were.

The native run adds what only a window has: the page negotiates `overlay.gizmo`, paints the boxes (pixel probes
read back from the canvas), the monitor's picture follows the candidate and is the committed picture after the
release and the initial one after undo (hashes), and **an explicit render and an FFmpeg export made while the
gizmo is on screen are bit-identical to the same ones made with nothing selected** (the export is decoded by an
independent tool, `ffmpeg`).

Report promises (as S8): the `--out` file is `ok: false` from the start; only a complete clean run turns it into
`ok: true`; a surface that did not run every required check fails; `final_evidence` only for both surfaces on a clean
tree without `--allow-dirty`.

Not proven here, on purpose: a PHYSICAL pointer or a native drag on the WKWebView (BL015.2, BL043/BL044), a splat
(the corpus has none: its refusals and its uniform scale are in `crates/core/tests/scene_gizmo_scale.rs`) and
performance (BL015.1 keeps it pending). The `scene.pointer.*` requests carry picture coordinates; a synthetic
request is not a mouse. The page's own mouse handling of the boxes is proved with real mouse events in headless
Chrome by `runtime/test/gizmo-overlay.test.mjs`.
"""
import argparse
import hashlib
import importlib.util
import json
import math
from pathlib import Path
import shutil
import subprocess
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
ECHO = 'scene/scene-a/node/echo'
ROOT_NODE = 'scene/scene-a/node/root'
SURFACES = S8.SURFACES
TOLERANCE = 1e-9
FACTOR_TOLERANCE = 1e-6
HANDLE_LENGTH = 0.14  # picture heights, the contract's number (read back from `metrics.handle_length`)
UNIFORM_OFFSET = 0.09

_COMMON = (
    'the corpus bundle loads',
    'the scale tool is chosen and is not an edit',
    'a selected node has three scale handles and the uniform box',
    'every axis handle ends where the pinhole puts it and the box is at the contract\'s picture offset',
    'a local axis drag is one undo entry and leaves W\' = W · diag(d)',
    'scene.scale with the drag\'s axis, factor and space leaves the same saved document',
    'the uniform handle leaves W\' = f · W and the origin where it was',
    'a world axis drag is one undo entry and leaves W\' = G · W',
    'a world axis no node axis runs along is disabled as `world_axis_shear` and refused typed',
    'a rotated, unevenly scaled and mirroring parent keep every local axis scalable, with the oracle\'s matrix',
    'a factor of one is a no-op: same bytes, no history, no revision',
    'a position past the origin keeps the last legal candidate',
    'a scale channel on the node is refused as `animated_scale` and nothing is keyed',
    'the tool cannot change under an open drag',
    'the translate tool is what it was',
)
REQUIRED = {
    'headless': _COMMON,
    'native': _COMMON + (
        'the page announced `overlay.gizmo` and drew the scale handles',
        'the page painted each handle in its colour',
        'the monitor\'s picture follows the candidate and returns to the initial one after undo',
        'an explicit render with the gizmo on screen is the render with nothing selected',
        'an FFmpeg export with the gizmo on screen decodes to the same pixels as with nothing selected',
    ),
}


# ---- the independent oracle: nothing here is the core's code -------------------------------------------------

def matmul(a, b):
    return [[sum(a[r][k] * b[k][c] for k in range(3)) for c in range(3)] for r in range(3)]


def diag(d):
    return [[d[0], 0, 0], [0, d[1], 0], [0, 0, d[2]]]


def linear_of(matrix):
    return [[matrix[0], matrix[4], matrix[8]], [matrix[1], matrix[5], matrix[9]], [matrix[2], matrix[6], matrix[10]]]


def column(m, c):
    return [m[0][c], m[1][c], m[2][c]]


def close(a, b, tolerance=TOLERANCE):
    return all(abs(x - y) <= tolerance for row_a, row_b in zip(a, b) for x, y in zip(row_a, row_b))


def scaled(m, f):
    return [[value * f for value in row] for row in m]


def world_projector(camera, output, origin):
    """`(handle length in world units, the projection)` at the depth of `origin`."""
    position = [camera['position'][a] for a in 'xyz']
    target = [camera['target'][a] for a in 'xyz']
    forward = S8.unit(S8.sub(target, position))
    depth = S8.dot(S8.sub(origin, position), forward)
    half = math.tan(math.radians(camera['fovDeg']) / 2)
    return HANDLE_LENGTH * 2 * half * depth


def along(origin, direction, length):
    return [origin[i] + direction[i] * length for i in range(3)]


def uv(point):
    return {'u': point[0], 'v': point[1]}


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

    def scale(entity, axis, factor, space):
        return ok({'op': 'scene.scale', 'scene': SCENE, 'entity': entity, 'frame': FRAME, 'axis': axis,
                   'factor': factor, 'space': space})

    def handle(shown, id_):
        return next(h for h in shown['handles'] if h['id'] == id_)

    def pointer(camera, output, shown, origin, id_, direction, factor):
        """Where the pointer must be for a drag that grabbed `id_` at its tip to ask for `factor`."""
        if id_ == 'uniform':
            o, b = shown['origin'], handle(shown, 'uniform')['to']
            return {'u': o['u'] + (b['u'] - o['u']) * factor, 'v': o['v'] + (b['v'] - o['v']) * factor}
        length = world_projector(camera, output, origin)
        return uv(S8.project(camera, output, along(origin, direction, length * factor)))

    def drag(entity, id_, direction, factor, space, label):
        """A whole drag of `id_` to `factor`; returns (up, matrix before, matrix after, origin)."""
        ok({'op': 'scene.select', 'scene': SCENE, 'entities': [entity]})
        tool('scale', space)
        shown = overlay()
        plan = plan_at()
        camera, output = plan['camera'], plan['output']
        node = entity.rsplit('/', 1)[1]
        before, origin = world(node)
        grabbed = handle(shown, id_)['to']
        down = ok({'op': 'scene.pointer.down', 'scene': SCENE, 'frame': FRAME, 'point': grabbed})
        if not (down['dragging'] is True and down['hit']['handle'] == id_ and down['tool'] == 'scale'):
            raise S8.ProofFailure(f'{label}: the press on `{id_}` did not open its drag: {down}')
        target = pointer(camera, output, shown, origin, id_, direction, factor)
        moved = ok({'op': 'scene.pointer.move', 'point': target})
        if not (moved['applied'] is True and moved['document_changed'] is False):
            raise S8.ProofFailure(f'{label}: the move was not a candidate: {moved}')
        up = ok({'op': 'scene.pointer.up', 'point': target})
        after, after_origin = world(node)
        return up, before, after, origin, after_origin

    if surface.name == 'native':
        ok({'op': 'project.create', 'name': 'G1.2 proof', 'sequence': 'main', 'rate': 24, 'width': 320, 'height': 180,
            'length': 96, 'dir': str(scratch / 'workspace')})
    attached = ok({'op': 'scene.attach', 'id': SCENE, 'path': str(BUNDLE)})
    check('the corpus bundle loads', attached.get('scene') == SCENE, attached)
    if surface.name == 'native':
        loaded = ok({'op': 'scene.resources.resolve', 'scene': SCENE, 'base': str(CORPUS)})
        check('the resources are resolved and applied', surface.lifecycle.commit_job(surface.lab, loaded, 'g1.2'), loaded)
        ok({'op': 'monitor.show_scene', 'scene': SCENE})
        surface.lab.seek(FRAME)
        check('the monitor converged on the animating frame', surface.lifecycle.settle(surface.lab, FRAME, 60) is not None,
              surface.viewport().get('directed'))
        record['initial_hash'] = S8.picture_hash(surface)
        check('the monitor has a picture at the animating frame', bool(record['initial_hash']), surface.viewport())

    base_bundle, base_history = bundle(), history()
    ok({'op': 'scene.select', 'scene': SCENE, 'entities': [MID]})
    before_tool = (bundle(), history())
    chosen = tool('scale', 'local')
    check('the scale tool is chosen and is not an edit',
          chosen['tool'] == 'scale' and chosen['space'] == 'local' and chosen['changed'] is True
          and chosen['tools'] == ['translate', 'rotate', 'scale'] and (bundle(), history()) == before_tool, chosen)

    # ---- the handles ----------------------------------------------------------------------------------------------
    shown = overlay()
    plan = plan_at()
    camera, output = plan['camera'], plan['output']
    linear_before, origin = world('mid')
    check('a selected node has three scale handles and the uniform box',
          shown['tool'] == 'scale' and shown['visible'] is True and shown['disabled_handles'] == []
          and [(h['id'], h['kind']) for h in shown['handles']]
          == [('x', 'scale_axis'), ('y', 'scale_axis'), ('z', 'scale_axis'), ('uniform', 'scale_uniform')], shown)
    length = world_projector(camera, output, origin)
    worst = 0.0
    for index, id_ in enumerate('xyz'):
        direction = S8.unit(column(linear_before, index))
        expected = S8.project(camera, output, along(origin, direction, length))
        got = handle(shown, id_)['to']
        worst = max(worst, abs(expected[0] - got['u']), abs(expected[1] - got['v']))
    aspect = output['width'] / output['height']
    box = handle(shown, 'uniform')['to']
    diagonal = UNIFORM_OFFSET * math.sqrt(0.5)
    worst = max(worst, abs(box['u'] - (shown['origin']['u'] + diagonal / aspect)), abs(box['v'] - (shown['origin']['v'] - diagonal)))
    check('every axis handle ends where the pinhole puts it and the box is at the contract\'s picture offset',
          worst < 1e-9 and shown['metrics']['handle_length'] == HANDLE_LENGTH
          and shown['metrics']['uniform_offset'] == UNIFORM_OFFSET, worst)

    if surface.name == 'native':
        native_selected(surface, check, record)

    # ---- a drag of the local x handle -------------------------------------------------------------------------------
    factor = 1.7
    x_direction = S8.unit(column(linear_before, 0))
    base_history = history()
    ok({'op': 'scene.select', 'scene': SCENE, 'entities': [MID]})
    tool('scale', 'local')
    grabbed = handle(overlay(), 'x')['to']
    down = ok({'op': 'scene.pointer.down', 'scene': SCENE, 'frame': FRAME, 'point': grabbed})
    check('a press on a handle opens a drag and writes nothing',
          down['dragging'] is True and down['hit']['handle'] == 'x' and down['tool'] == 'scale'
          and bundle() == base_bundle and history() == base_history, down)
    target = pointer(camera, output, overlay(), origin, 'x', x_direction, factor)
    moved = ok({'op': 'scene.pointer.move', 'point': target})
    check('a move is a candidate: the saved document and the history are untouched',
          moved['applied'] is True and moved['document_changed'] is False and bundle() == base_bundle
          and history() == base_history, moved)
    if surface.name == 'native':
        native_candidate(surface, check, record)
    refused_tool = refused({'op': 'scene.gizmo.tool', 'tool': 'translate'})
    check('the tool cannot change under an open drag',
          refused_tool['code'] == 'gesture_active' and surface.op({'op': 'scene.gizmo.tool'})[1]['tool'] == 'scale',
          refused_tool)
    up = ok({'op': 'scene.pointer.up', 'point': target})
    swept = up['factor']
    record['scale'] = up['scale']
    record['factor'] = swept
    linear_after, after_origin = world('mid')
    check('a local axis drag is one undo entry and leaves W\' = W · diag(d)',
          up['document_changed'] is True and history()[0] == base_history[0] + 1 and abs(swept - factor) < FACTOR_TOLERANCE
          and up['axis'] == 'x' and up['space'] == 'local'
          and close(linear_after, matmul(linear_before, diag([swept, 1, 1])))
          and all(abs(a - b) < TOLERANCE for a, b in zip(after_origin, origin)), (up, swept))
    dragged_bundle = bundle()
    if surface.name == 'native':
        native_committed(surface, check, record)
    ok({'op': 'history.undo'})
    check('one undo restores the saved document exactly', bundle() == base_bundle)
    if surface.name == 'native':
        native_undone(surface, check, record)
    base_history = history()

    scaled_ = scale(MID, up['axis'], swept, up['space'])
    check('scene.scale with the drag\'s axis, factor and space leaves the same saved document',
          scaled_['changed'] is True and bundle() == dragged_bundle and scaled_['scale'] == up['scale'], scaled_)
    ok({'op': 'history.undo'})
    base_history = history()

    # ---- the uniform handle ----------------------------------------------------------------------------------------
    up, before, after, start, end = drag(MID, 'uniform', None, 1.4, 'world', 'uniform')
    check('the uniform handle leaves W\' = f · W and the origin where it was',
          up['document_changed'] is True and abs(up['factor'] - 1.4) < FACTOR_TOLERANCE
          and close(after, scaled(before, up['factor'])) and all(abs(a - b) < TOLERANCE for a, b in zip(start, end)), up)
    ok({'op': 'history.undo'})
    check('and one undo takes it back', bundle() == base_bundle)

    # ---- a world axis ------------------------------------------------------------------------------------------------
    ok({'op': 'scene.rotate', 'scene': SCENE, 'entity': MID, 'frame': FRAME, 'axis': 'z', 'degrees': 30, 'space': 'world'})
    turned_bundle = bundle()
    ok({'op': 'scene.select', 'scene': SCENE, 'entities': [MID]})
    tool('scale', 'world')
    shown = overlay()
    check('a world axis no node axis runs along is disabled as `world_axis_shear` and refused typed',
          [h['id'] for h in shown['handles']] == ['z', 'uniform']
          and shown['disabled_handles'] == [{'id': 'x', 'reason': 'world_axis_shear'}, {'id': 'y', 'reason': 'world_axis_shear'}]
          and refused({'op': 'scene.scale', 'scene': SCENE, 'entity': MID, 'frame': FRAME, 'axis': 'x', 'factor': 2,
                       'space': 'world'})['details']['reason'] == 'world_axis_shear'
          and bundle() == turned_bundle, shown)
    up, before, after, start, end = drag(MID, 'z', [0, 0, 1], 2.0, 'world', 'world z')
    g = [[1, 0, 0], [0, 1, 0], [0, 0, up['factor']]]
    check('a world axis drag is one undo entry and leaves W\' = G · W',
          up['document_changed'] is True and abs(up['factor'] - 2.0) < FACTOR_TOLERANCE
          and close(after, matmul(g, before)) and all(abs(a - b) < TOLERANCE for a, b in zip(start, end)), up)
    ok({'op': 'history.undo'})
    ok({'op': 'history.undo'})
    check('and the turn and the scale are both taken back', bundle() == base_bundle)
    base_history = history()

    # ---- parents ----------------------------------------------------------------------------------------------------
    parent_cases = (
        ('rotated', {'x': 1, 'y': 1, 'z': 1}, ('z', 50)),
        ('unevenly scaled', {'x': 2, 'y': 1, 'z': 3}, None),
        ('mirroring', {'x': -1, 'y': 1, 'z': 1}, None),
    )
    parents_ok = True
    detail = []
    for name, root_scale, root_turn in parent_cases:
        ok({'op': 'scene.edit', 'scene': SCENE, 'ops': [{'set': f'{ROOT_NODE}/scale', 'value': root_scale}]})
        if root_turn:
            ok({'op': 'scene.rotate', 'scene': SCENE, 'entity': ROOT_NODE, 'frame': FRAME, 'axis': root_turn[0],
                'degrees': root_turn[1], 'space': 'world'})
        mid_before, mid_origin = world('mid')
        for index, id_ in enumerate('xyz'):
            answer = scale(MID, id_, 1.6, 'local')
            mid_after, mid_after_origin = world('mid')
            d = [1, 1, 1]
            d[index] = 1.6
            good = (answer['changed'] is True and close(mid_after, matmul(mid_before, diag(d)))
                    and all(abs(a - b) < TOLERANCE for a, b in zip(mid_origin, mid_after_origin)))
            parents_ok = parents_ok and good
            detail.append((name, id_, good))
            ok({'op': 'history.undo'})
        shown_here = overlay()
        parents_ok = parents_ok and [h['id'] for h in shown_here['handles']][-1] == 'uniform'
        while history()[0] > base_history[0]:
            ok({'op': 'history.undo'})
    check('a rotated, unevenly scaled and mirroring parent keep every local axis scalable, with the oracle\'s matrix',
          parents_ok and bundle() == base_bundle, detail)
    base_history = history()

    # ---- the animating nodes ----------------------------------------------------------------------------------------
    hero_before, hero_origin = world('hero')
    answer = scale(HERO, 'y', 1.3, 'local')
    hero_after, hero_after_origin = world('hero')
    check('a node under a sliding parent that sways is scaled about its own origin',
          answer['changed'] is True and close(hero_after, matmul(hero_before, diag([1, 1.3, 1])))
          and all(abs(a - b) < TOLERANCE for a, b in zip(hero_origin, hero_after_origin)))
    ok({'op': 'history.undo'})

    # ---- no-ops, cancels -------------------------------------------------------------------------------------------
    base_bundle, base_history = bundle(), history()
    shown_noops = []
    for axis in ('x', 'y', 'z', 'uniform'):
        answer = scale(MID, axis, 1, 'local')
        shown_noops.append(answer['changed'] is False and bundle() == base_bundle and history() == base_history)
    check('a factor of one is a no-op: same bytes, no history, no revision', all(shown_noops), shown_noops)
    for factor_, reason in ((0, 'factor_not_positive'), (-1, 'factor_not_positive'), (5000, 'above_maximum_scale'),
                            (0.0001, 'below_minimum_scale')):
        error = refused({'op': 'scene.scale', 'scene': SCENE, 'entity': MID, 'frame': FRAME, 'axis': 'x',
                         'factor': factor_, 'space': 'local'})
        check(f'factor {factor_} is refused as `{reason}`', error['code'] == 'not_scalable' and error['details']['reason'] == reason,
              error)
        untouched(f'factor {factor_}', base_bundle, base_history)
    shown = overlay()
    ok({'op': 'scene.select', 'scene': SCENE, 'entities': [MID]})
    tool('scale', 'local')
    shown = overlay()
    grabbed = handle(shown, 'uniform')['to']
    for reason in ('escape', 'blur', 'pointer_cancel', 'hidden', 'unavailable', 'api'):
        ok({'op': 'scene.pointer.down', 'scene': SCENE, 'frame': FRAME, 'point': grabbed})
        ok({'op': 'scene.pointer.move', 'point': pointer(camera, output, shown, origin, 'uniform', None, 2.5)})
        cancelled = ok({'op': 'scene.pointer.cancel', 'reason': reason})
        check(f'cancel `{reason}` ends the scale drag', cancelled['reason'] == reason and cancelled['dragging'] is False, cancelled)
        untouched(f'cancel `{reason}`', base_bundle, base_history)
    # a drag whose pointer goes past the node keeps the last legal candidate
    ok({'op': 'scene.pointer.down', 'scene': SCENE, 'frame': FRAME, 'point': grabbed})
    good_target = pointer(camera, output, shown, origin, 'uniform', None, 1.8)
    ok({'op': 'scene.pointer.move', 'point': good_target})
    past = ok({'op': 'scene.pointer.move', 'point': shown['origin']})
    final = ok({'op': 'scene.pointer.up', 'point': shown['origin']})
    check('a position past the origin keeps the last legal candidate',
          past['applied'] is False and past['reason'] == 'factor_not_positive' and abs(final['factor'] - 1.8) < FACTOR_TOLERANCE
          and history()[0] == base_history[0] + 1, (past, final))
    ok({'op': 'history.undo'})
    base_history = history()

    # ---- refusals ----------------------------------------------------------------------------------------------------
    for name, request, code in (
        ('the screen handle', {'op': 'scene.scale', 'scene': SCENE, 'entity': MID, 'frame': FRAME, 'axis': 'screen',
                               'factor': 2}, 'invalid_value'),
        ('a missing factor', {'op': 'scene.scale', 'scene': SCENE, 'entity': MID, 'frame': FRAME, 'axis': 'x'},
         'invalid_request'),
        ('an unknown node', {'op': 'scene.scale', 'scene': SCENE, 'entity': 'scene/scene-a/node/ghost', 'frame': FRAME,
                             'axis': 'x', 'factor': 2}, 'unknown_entity'),
        ('a camera clip', {'op': 'scene.scale', 'scene': SCENE, 'entity': 'camera/cam-main', 'frame': FRAME, 'axis': 'x',
                           'factor': 2}, 'not_scalable'),
        ('an unknown tool', {'op': 'scene.gizmo.tool', 'tool': 'resize'}, 'invalid_value'),
    ):
        error = refused(request)
        check(f'refused: {name} ({code})', error['code'] == code, error)
        untouched(name, base_bundle, base_history)

    tracks_before = bundle().get('performanceTracks')
    error = refused({'op': 'scene.scale', 'scene': SCENE, 'entity': ECHO, 'frame': FRAME, 'axis': 'x', 'factor': 2})
    ok({'op': 'scene.select', 'scene': SCENE, 'entities': [ECHO]})
    hidden = overlay()
    check('a scale channel on the node is refused as `animated_scale` and nothing is keyed',
          error['code'] == 'not_scalable' and error['details']['reason'] == 'animated_scale'
          and hidden['visible'] is False and hidden['reason'] == 'animated_scale'
          and bundle().get('performanceTracks') == tracks_before, (error, hidden))
    untouched('the animated node', base_bundle, base_history)
    ok({'op': 'scene.select', 'scene': SCENE, 'entities': [MID]})

    # ---- the old tools -----------------------------------------------------------------------------------------------
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
    tool('rotate', 'world')
    rings = overlay()
    check('and so is the rotate tool', [h['kind'] for h in rings['handles']] == ['ring', 'ring', 'ring']
          and ok({'op': 'scene.rotate', 'scene': SCENE, 'entity': MID, 'frame': FRAME, 'axis': 'z', 'degrees': 0,
                  'space': 'world'})['changed'] is False)
    if surface.name == 'native':
        native_export(surface, check, scratch, record)


# ---- what only a window has --------------------------------------------------------------------------------------

def native_selected(surface, check, record):
    state = surface.viewport()
    overlay = state.get('overlay') or {}
    drawn = S8.wait_for(lambda: ((surface.viewport().get('overlay') or {}).get('drawn') or {}).get('tool') == 'scale')
    check('the page announced `overlay.gizmo` and drew the scale handles',
          overlay.get('supported') is True and overlay.get('enabled') is True and bool(drawn),
          surface.viewport().get('overlay'))
    drawn_state = (surface.viewport().get('overlay') or {}).get('drawn') or {}
    probes = {probe['id']: probe['rgba'] for probe in drawn_state.get('probes', [])}
    x, y, z, uniform = probes.get('x'), probes.get('y'), probes.get('z'), probes.get('uniform')
    check('the page painted each handle in its colour',
          all((x, y, z, uniform)) and x[3] > 200 and x[0] > x[1] and x[0] > x[2] and y[3] > 200 and y[1] > y[0]
          and z[3] > 200 and z[2] > z[0] and z[2] > z[1] and uniform[3] > 200 and min(uniform[:3]) > 200, probes)
    record['drawn_probes'] = probes


def native_candidate(surface, check, record):
    drawn = S8.wait_for(lambda: ((surface.viewport().get('overlay') or {}).get('drawn') or {}).get('active') == 'x')
    record['active_drawn'] = bool(drawn)
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
          record.get('picture_followed') is True and record.get('active_drawn') is True
          and record['committed_hash'] == record['candidate_hash'] and S8.picture_hash(surface) == record['initial_hash'],
          (record, S8.picture_hash(surface)))


def decode_export(destination, width, height):
    """The first frame of an MP4, decoded by FFmpeg (an independent tool) to raw RGB: `(bytes, digest)`."""
    raw = subprocess.run(['ffmpeg', '-v', 'error', '-i', str(destination), '-frames:v', '1', '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-'],
                         capture_output=True, check=True).stdout
    if len(raw) != width * height * 3:
        raise S8.ProofFailure(f'the decoded export is {len(raw)} bytes, expected {width * height * 3}')
    return raw, hashlib.sha256(raw).hexdigest()


def surface_overlay(surface):
    good, payload = surface.op({'op': 'scene.gizmo.state', 'scene': SCENE, 'frame': S8.FRAME})
    if not good:
        raise S8.ProofFailure(f'scene.gizmo.state -> {payload}')
    return payload['overlay']


def native_export(surface, check, scratch, record):
    """An explicit render and an export made while the gizmo is on screen are the ones made with nothing selected."""
    lab = surface.lab
    width, height = 320, 180

    shown = surface_overlay(surface)
    tips = [shown['origin']] + [h['to'] for h in shown['handles']]
    pixels = [[min(width - 1, max(0, round(t['u'] * width))), min(height - 1, max(0, round(t['v'] * height)))] for t in tips]

    def render():
        good, body = lab.op({'op': 'scene.render', 'scene': SCENE, 'frame': S8.FRAME, 'width': width, 'height': height,
                             'probes': pixels})
        if not good:
            raise S8.ProofFailure(f'scene.render -> {body}')
        return body['frame']

    def export(name):
        destination = scratch / f'{name}.mp4'
        good, body = lab.op({'op': 'scene.export', 'scene': SCENE, 'preset': 'fullhd', 'destination': str(destination), 'from': S8.FRAME,
                             'frames': 1, 'audio': False, 'resources_root': str(CORPUS)}, 120)
        if not good:
            raise S8.ProofFailure(f'scene.export -> {body}')
        if body.get('state') == 'queued' or 'job' in body:
            job = body.get('job')
            status = {}

            def finished():
                nonlocal status
                got, status = lab.op({'op': 'job.get', 'job': job})
                return got and status['state'] in ('succeeded', 'failed', 'cancelled')
            if not S8.wait_for(finished, 120, 0.1):
                raise S8.ProofFailure(f'the export job did not finish: {status}')
            if status['state'] != 'succeeded':
                raise S8.ProofFailure(f'the export job {status["state"]}: {status}')
        return destination

    ok = lab.op
    ok({'op': 'scene.select', 'scene': SCENE, 'entities': [MID]})
    ok({'op': 'scene.gizmo.tool', 'tool': 'scale', 'space': 'local'})
    drawn = S8.wait_for(lambda: ((surface.viewport().get('overlay') or {}).get('drawn') or {}).get('tool') == 'scale'
                        and ((surface.viewport().get('overlay') or {}).get('drawn') or {}).get('visible') is True)
    if not drawn:
        raise S8.ProofFailure(f'the gizmo is not on screen: {surface.viewport().get("overlay")}')
    S8.settle_converged(surface)
    with_gizmo = render()
    exported_with = export('with-gizmo')
    ok({'op': 'scene.select', 'scene': SCENE, 'entities': []})
    S8.wait_for(lambda: ((surface.viewport().get('overlay') or {}).get('drawn') or {}).get('visible') is False)
    S8.settle_converged(surface)
    without = render()
    exported_without = export('without-gizmo')
    check('an explicit render with the gizmo on screen is the render with nothing selected',
          with_gizmo['hash'] == without['hash'] and with_gizmo.get('probes') == without.get('probes')
          and with_gizmo.get('inspected') is True and len(with_gizmo.get('probes', [])) == len(pixels), (with_gizmo, without))
    record['render_probes'] = with_gizmo.get('probes')
    _, digest_with = decode_export(exported_with, 1920, 1080)
    _, digest_without = decode_export(exported_without, 1920, 1080)
    # The control: the comparison can tell. A scale that really changes the picture changes the decoded export.
    ok({'op': 'scene.select', 'scene': SCENE, 'entities': [MID]})
    ok({'op': 'scene.scale', 'scene': SCENE, 'entity': MID, 'frame': S8.FRAME, 'axis': 'uniform', 'factor': 1.5, 'space': 'local'})
    S8.settle_converged(surface)
    _, digest_scaled = decode_export(export('scaled'), 1920, 1080)
    ok({'op': 'history.undo'})
    check('an FFmpeg export with the gizmo on screen decodes to the same pixels as with nothing selected',
          digest_with == digest_without and digest_scaled != digest_without, (digest_with, digest_without, digest_scaled))
    record['render_hash'] = with_gizmo['hash']
    record['export_digest'] = digest_with


def run_surface(name, binary, report):
    work = tempfile.mkdtemp(prefix='te-g12-', dir='/tmp')
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
    # a surface that did not stop is a failed proof, never `ok`.
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
                       'the page\'s own mouse handling (runtime/test/gizmo-overlay.test.mjs)',
                       'a splat (no splat in the corpus; crates/core/tests/scene_gizmo_scale.rs)',
                       'performance (BL015.1)'],
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
            same = h['scale'] == n['scale'] and abs(h['factor'] - n['factor']) < 1e-9
            report['surfaces_agree_on_the_drag'] = same
            if not same:
                raise S8.ProofFailure('headless and native computed different scales for the same drag')
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
