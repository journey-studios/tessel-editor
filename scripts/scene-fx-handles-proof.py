#!/usr/bin/env python3
"""F061 (G4.1, BL082.35) — the 2D handles of the vignette on the monitor, proved on two separate surfaces with
one scenario:

    python3 scripts/scene-fx-handles-proof.py [--editor target/debug/tessel-editor] \
        [--headless target/debug/tessel-headless] [--out report.json] [--allow-dirty]

Functional proof only (no timing is measured), so it does not take the GPU lock: that lock is reserved for
performance validations (AGENTS.md).

The surfaces and their mechanics are the ones of `scene-gizmo-proof.py` (S8), loaded from it, not copied:

* **headless (no window)**: `tessel-headless --empty`, JSON Lines over stdio. No GPUI, no WebView.
* **native window**: the DIRECT editor binary with its own short `HOME`, driven only by its `ui` socket
  (`op`, `get`, `set`, `invoke`). The user's window and project are never touched.

The scenario, in semantic requests on the corpus bundle (320x180) with one vignette `v1`. Every expectation is
computed HERE from the closed formulas of the pixel math (`stretch = 1 + (W/H - 1)·roundness`,
`norm = max(hypot(stretch, 1), 1)`, `a_u(r) = r·norm / (2·stretch)`), never by the core:

* the radius handle sits at `(cx + a_u(radius), cy)` and the softness handle at `(cx + a_u(radius + softness), cy)`;
* a drag of each handle ends with the value the inverse formula gives for the pointer (quantized to the catalog
  step and clamped to its range), as ONE "Edit effects" undo entry, and leaves the very document `scene.fx.edit`
  leaves for that value (compared byte for byte through `project.save`); undo restores it exactly;
* the centre drag is free in 2D;
* a parameter with keys has its handle off (`animated_param`): the press is refused by name, no gesture opens and
  nothing is written;
* a handle off the picture is listed with `handle_off_picture` and a press where it would be opens nothing;
* Escape (`scene.pointer.cancel`) leaves document, history and revision as they were;
* one target at a time: selecting an entity leaves the clip (and a live drag with it), selecting a clip empties
  the selection, and an effect without handles is refused as `fx_has_no_handles`.

The native run adds what only a window has: the panel's `handles` action is the same `scene.fx.select`, the
semantic node `monitor.gizmo` says `target: fx`, the page paints the handles and the ellipses (pixel probes read
back from the canvas), the monitor's picture follows the candidate, is the committed picture after the release and
the initial one after undo (hashes), a dialog that hides the WebView cancels the open drag, and **an explicit
render and an FFmpeg export made with the handles on screen are bit-identical to the ones made with the handles
off** (with a control: a committed drag changes the decoded export).

Report promises (as S8): the `--out` file is `ok: false` from the start; only a complete clean run turns it into
`ok: true`; a surface that did not run every required check fails; `final_evidence` only for both surfaces on a clean
tree without `--allow-dirty`.

Not proven here, on purpose: a PHYSICAL pointer or a native drag on the WKWebView (BL015.2, BL043/BL044), and
performance. The `scene.pointer.*` requests carry picture coordinates; a synthetic request is not a mouse. The
page's own mouse handling of the handles is proved with real mouse events in headless Chrome by
`runtime/test/fx-handles-overlay.test.mjs`.
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


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / file)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


S8 = load('scene_gizmo_proof', 'scene-gizmo-proof.py')
SCENE = S8.SCENE
BUNDLE = S8.BUNDLE
CORPUS = S8.CORPUS
SURFACES = S8.SURFACES
WIDTH, HEIGHT = 320, 180
FRAME = 24  # inside the clip (0..48)
CLIP = 'v1'
CLIP_NODE = f'scene.fx.clip:{CLIP}'
TOLERANCE = 1e-9
CENTER = (0.4, 0.5)
RADIUS, SOFTNESS, ROUNDNESS = 0.4, 0.2, 0.6

_COMMON = (
    'the corpus bundle loads and the vignette is added',
    'an effect without handles is refused as `fx_has_no_handles`',
    'selecting the clip offers the centre, radius and softness handles and the two ellipses',
    'the handles sit where the closed formula puts them',
    'selecting the clip is session state: not an edit',
    'a radius drag ends with the value the inverse formula gives, as one "Edit effects" entry',
    'the radius drag leaves the document `scene.fx.edit` leaves for the same value (byte for byte)',
    'a softness drag ends with the value the inverse formula gives, as one entry',
    'the centre drag is free in 2D and leaves the document `scene.fx.edit` leaves',
    'a press released in place is a no-op: same bytes, no history, no revision',
    'a pointer past the catalog range is clamped to it',
    'Escape leaves document, history and revision as they were',
    'a parameter with keys has its handle off and a press on it is refused by name, writing nothing',
    'a handle off the picture is listed with its reason and a press where it would be opens nothing',
    'selecting an entity leaves the clip and cancels the drag; selecting the clip empties the selection',
)
REQUIRED = {
    'headless': _COMMON,
    'native': _COMMON + (
        'the panel\'s `handles` action is the shared `scene.fx.select` and `monitor.gizmo` says `target: fx`',
        'the page announced `overlay.gizmo` and drew the vignette\'s handles and ellipses',
        'the page painted each handle on the canvas (alpha > 200)',
        'the page drew the active handle of the drag',
        'the monitor\'s picture follows the candidate and returns to the initial one after undo',
        'a dialog that hides the WebView cancels the open drag and writes nothing',
        'an explicit render with the handles on screen is the render with the handles off',
        'an FFmpeg export with the handles on screen decodes to the same pixels as with the handles off',
    ),
}


# ---- the closed formulas of the pixel math: nothing here is the core's code -------------------------------------

def stretch(roundness=ROUNDNESS):
    return 1 + (WIDTH / HEIGHT - 1) * roundness


def norm(roundness=ROUNDNESS):
    return max(math.hypot(stretch(roundness), 1), 1)


def a_u(radius, roundness=ROUNDNESS):
    return radius * norm(roundness) / (2 * stretch(roundness))


def radius_of(u, cx, roundness=ROUNDNESS):
    return (u - cx) * 2 * stretch(roundness) / norm(roundness)


def grid(value, low, high, step):
    snapped = math.floor((value - low) / step + 0.5) * step + low
    return min(max(snapped, low), high)


def near(a, b, tolerance=TOLERANCE):
    return abs(a - b) <= tolerance


def point(u, v):
    return {'u': u, 'v': v}


def handle_of(shown, id_):
    return next((h for h in shown['handles'] if h['id'] == id_), None)


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
        return data['history']['undo_depth'], data['project']['revision'], data['history'].get('undo_label')

    def depth():
        return history()[:2]

    def handles(frame=FRAME):
        return ok({'op': 'scene.fx.handles', 'scene': SCENE, 'clip': CLIP, 'frame': frame})

    def edit(*ops):
        return ok({'op': 'scene.fx.edit', 'scene': SCENE, 'ops': list(ops)})

    def drag(id_, to, label):
        """A whole drag of a handle to the picture point `to`; returns the shown handles before."""
        shown = handles()
        at = handle_of(shown, id_)['at']
        ok({'op': 'scene.pointer.down', 'scene': SCENE, 'frame': FRAME, 'point': at})
        ok({'op': 'scene.pointer.move', 'point': to})
        return shown

    # ---- the scene ------------------------------------------------------------------------------------------------
    if surface.name == 'native':
        ok({'op': 'project.create', 'name': 'F061 proof', 'sequence': 'main', 'rate': 24, 'width': WIDTH, 'height': HEIGHT,
            'length': 96, 'dir': str(scratch / 'workspace')})
    attached = ok({'op': 'scene.attach', 'id': SCENE, 'path': str(BUNDLE)})
    if surface.name == 'native':
        loaded = ok({'op': 'scene.resources.resolve', 'scene': SCENE, 'base': str(CORPUS)})
        if not surface.lifecycle.commit_job(surface.lab, loaded, 'handles'):
            raise S8.ProofFailure(f'the resources were not applied: {loaded}')
        surface.lab.send({'command': 'set', 'target': 'inspector.mode', 'field': 'value', 'value': 'scene'})
    edit({'op': 'add_clip', 'effect': 'vignette', 'start_frame': 0, 'duration_frames': 48, 'id': CLIP,
          'params': {'intensity': 1.0, 'radius': RADIUS, 'softness': SOFTNESS, 'roundness': ROUNDNESS,
                     'center': {'x': CENTER[0], 'y': CENTER[1]}}})
    edit({'op': 'add_clip', 'effect': 'fade', 'start_frame': 0, 'duration_frames': 10, 'id': 'f1'})
    check('the corpus bundle loads and the vignette is added', attached.get('scene') == SCENE, attached)
    if surface.name == 'native':
        shown = ok({'op': 'monitor.show_scene', 'scene': SCENE})
        surface.lab.seek(FRAME)
        if surface.lifecycle.settle(surface.lab, FRAME, 60) is None:
            raise S8.ProofFailure(f'the monitor did not converge: {surface.viewport().get("directed")}')
        record['initial_hash'] = picture_hash(surface)

    base_bundle = bundle()
    base_depth = depth()

    # ---- one effect without handles, then the clip -----------------------------------------------------------------
    error = refused({'op': 'scene.fx.select', 'scene': SCENE, 'clip': 'f1'})
    check('an effect without handles is refused as `fx_has_no_handles`', error.get('code') == 'fx_has_no_handles', error)
    selected = ok({'op': 'scene.fx.select', 'scene': SCENE, 'clip': CLIP})
    shown = handles()
    ids = sorted(h['id'] for h in shown['handles'])
    check('selecting the clip offers the centre, radius and softness handles and the two ellipses',
          ids == ['center', 'radius', 'softness'] and len(shown['ellipses']) == 2 and shown['selected'] is True, shown)
    cx, cy = CENTER
    radius_at = handle_of(shown, 'radius')['at']
    softness_at = handle_of(shown, 'softness')['at']
    check('the handles sit where the closed formula puts them',
          near(handle_of(shown, 'center')['at']['u'], cx) and near(handle_of(shown, 'center')['at']['v'], cy)
          and near(radius_at['u'], cx + a_u(RADIUS)) and near(radius_at['v'], cy)
          and near(softness_at['u'], cx + a_u(RADIUS + SOFTNESS)) and near(softness_at['v'], cy)
          and near(shown['ellipses'][0]['rx'], a_u(RADIUS)) and near(shown['ellipses'][1]['rx'], a_u(RADIUS + SOFTNESS)),
          (radius_at, softness_at, cx + a_u(RADIUS), cx + a_u(RADIUS + SOFTNESS)))
    check('selecting the clip is session state: not an edit',
          selected['document_changed'] is False and bundle() == base_bundle and depth() == base_depth, selected)

    # ---- the radius drag --------------------------------------------------------------------------------------------
    want_radius = grid(radius_of(cx + a_u(0.55) + 0.0013, cx), 0.0, 1.5, 0.01)
    target = point(cx + a_u(0.55) + 0.0013, cy)
    drag('radius', target, 'radius')
    check('the candidate is the one the overlay marks active', handles()['active'] == 'radius', handles())
    check('the candidate is not the document until the release', bundle() == base_bundle and depth() == base_depth,
          (depth(), base_depth))
    ok({'op': 'scene.pointer.up', 'point': target})
    drag_bundle = bundle()
    after = handles()
    count, _, label = history()
    check('a radius drag ends with the value the inverse formula gives, as one "Edit effects" entry',
          near(after['values']['radius'], want_radius) and count == base_depth[0] + 1 and label == 'Edit effects',
          (after['values'], want_radius, count, label))
    ok({'op': 'history.undo'})
    check('undo gives the document back exactly', bundle() == base_bundle and depth()[0] == base_depth[0])
    edit({'op': 'set_param', 'clip': CLIP, 'param': 'radius', 'value': after['values']['radius']})
    check('the radius drag leaves the document `scene.fx.edit` leaves for the same value (byte for byte)',
          bundle() == drag_bundle, 'the two saved documents differ')
    record['radius'] = after['values']['radius']
    ok({'op': 'history.undo'})

    # ---- the softness drag --------------------------------------------------------------------------------------------
    raw_u = cx + a_u(RADIUS + 0.13) + 0.0011
    want_softness = grid(radius_of(raw_u, cx) - RADIUS, 0.05, 1.0, 0.01)
    drag('softness', point(raw_u, cy), 'softness')
    ok({'op': 'scene.pointer.up', 'point': point(raw_u, cy)})
    after = handles()
    check('a softness drag ends with the value the inverse formula gives, as one entry',
          near(after['values']['softness'], want_softness) and depth()[0] == base_depth[0] + 1,
          (after['values'], want_softness, depth()))
    ok({'op': 'history.undo'})

    # ---- the centre drag ------------------------------------------------------------------------------------------------
    new_center = point(0.5512, 0.6171)
    want = {'x': grid(0.5512, 0.0, 1.0, 0.01), 'y': grid(0.6171, 0.0, 1.0, 0.01)}
    drag('center', new_center, 'center')
    ok({'op': 'scene.pointer.up', 'point': new_center})
    after = handles()
    drag_bundle = bundle()
    ok({'op': 'history.undo'})
    edit({'op': 'set_param', 'clip': CLIP, 'param': 'center', 'value': want})
    check('the centre drag is free in 2D and leaves the document `scene.fx.edit` leaves',
          near(after['values']['center']['x'], want['x']) and near(after['values']['center']['y'], want['y'])
          and bundle() == drag_bundle, (after['values'], want))
    ok({'op': 'history.undo'})

    # ---- no-op, clamp, Escape ---------------------------------------------------------------------------------------------
    shown = handles()
    at = handle_of(shown, 'radius')['at']
    resting = (bundle(), depth())
    ok({'op': 'scene.pointer.down', 'scene': SCENE, 'frame': FRAME, 'point': at})
    ok({'op': 'scene.pointer.up', 'point': at})
    check('a press released in place is a no-op: same bytes, no history, no revision',
          (bundle(), depth()) == resting, (depth(), resting[1]))

    # The pointer is put where the closed formula lands OUTSIDE the range and the limit itself is asserted: on the
    # centre the distance is 0, so the softness (distance - radius = -0.4) goes down to its floor, 0.05; far to the
    # right the radius (4.96) goes up to its ceiling, 1.5. (u = 1.0 would give a softness of 0.59, inside the range.)
    if not (radius_of(cx, cx) - RADIUS < 0.05 and radius_of(cx + 3.0, cx) > 1.5):
        raise S8.ProofFailure('the clamp case lost its premise: the points no longer fall outside the range')
    low = point(cx, cy)
    drag('softness', low, 'clamp')
    ok({'op': 'scene.pointer.up', 'point': low})
    floor = handles()['values']['softness']
    ok({'op': 'history.undo'})
    high = point(cx + 3.0, cy)
    drag('radius', high, 'clamp')
    ok({'op': 'scene.pointer.up', 'point': high})
    ceiling = handles()['values']['radius']
    ok({'op': 'history.undo'})
    check('a pointer past the catalog range is clamped to it', near(floor, 0.05) and near(ceiling, 1.5),
          {'softness_floor': floor, 'radius_ceiling': ceiling})

    resting = (bundle(), depth())
    drag('radius', point(cx + 0.1, cy), 'escape')
    ok({'op': 'scene.pointer.cancel', 'reason': 'escape'})
    check('Escape leaves document, history and revision as they were',
          handles()['active'] is None and (bundle(), depth()) == resting, (depth(), resting[1]))

    # ---- keys ------------------------------------------------------------------------------------------------------------------
    edit({'op': 'key_param', 'clip': CLIP, 'param': 'radius', 'frame': 0, 'value': 0.4},
         {'op': 'key_param', 'clip': CLIP, 'param': 'radius', 'frame': 30, 'value': 0.6})
    keyed_bundle, keyed_depth = bundle(), depth()
    shown = handles()
    entry = next((h for h in shown['disabled_handles'] if h['id'] == 'radius'), None)
    spot = point(cx + shown['ellipses'][0]['rx'], cy)
    answer = ok({'op': 'scene.pointer.down', 'scene': SCENE, 'frame': FRAME, 'point': spot})
    check('a parameter with keys has its handle off and a press on it is refused by name, writing nothing',
          entry is not None and entry['reason'] == 'animated_param' and handle_of(shown, 'radius') is None
          and answer.get('refused') == 'animated_param' and answer.get('dragging') is False
          and bundle() == keyed_bundle and depth() == keyed_depth, (entry, answer))
    ok({'op': 'history.undo'})

    # ---- off the picture --------------------------------------------------------------------------------------------------------
    edit({'op': 'set_param', 'clip': CLIP, 'param': 'center', 'value': {'x': 0.9, 'y': 0.5}})
    off_bundle, off_depth = bundle(), depth()
    shown = handles()
    reasons = {h['id']: h['reason'] for h in shown['disabled_handles']}
    # The handle would be at 0.9 + a_u(radius) > 1: a press has to land on the picture, so the nearest
    # thing to it is the picture's edge, and a press past the edge is refused by the pointer's own range.
    edge = refused({'op': 'scene.pointer.down', 'scene': SCENE, 'frame': FRAME,
                    'point': point(0.9 + a_u(RADIUS), 0.5)})
    answer = ok({'op': 'scene.pointer.down', 'scene': SCENE, 'frame': FRAME, 'point': point(0.999, 0.5)})
    check('a handle off the picture is listed with its reason and a press where it would be opens nothing',
          reasons.get('radius') == 'handle_off_picture' and reasons.get('softness') == 'handle_off_picture'
          and handle_of(shown, 'radius') is None and answer.get('dragging') is False
          and edge.get('code') == 'invalid_value'
          and bundle() == off_bundle and depth() == off_depth, (reasons, answer, edge))
    ok({'op': 'history.undo'})
    ok({'op': 'scene.fx.select', 'scene': SCENE, 'clip': CLIP})

    # ---- one target at a time ---------------------------------------------------------------------------------------------------
    at = handle_of(handles(), 'radius')['at']
    resting = (bundle(), depth())
    ok({'op': 'scene.pointer.down', 'scene': SCENE, 'frame': FRAME, 'point': at})
    ok({'op': 'scene.pointer.move', 'point': point(at['u'] + 0.05, at['v'])})
    figure = 'scene/scene-a/node/hero'
    swapped = ok({'op': 'scene.select', 'scene': SCENE, 'entities': [figure]})
    ghost = ok({'op': 'scene.pointer.up', 'point': point(at['u'] + 0.05, at['v'])})
    back = ok({'op': 'scene.fx.select', 'scene': SCENE, 'clip': CLIP})
    check('selecting an entity leaves the clip and cancels the drag; selecting the clip empties the selection',
          swapped.get('fx_target_dropped', {}).get('clip') == CLIP and swapped.get('cancelled_drag')
          and back.get('selection_cleared') is True and (bundle(), depth()) == resting,
          (swapped, back, ghost))
    record['ghost_release'] = ghost

    if surface.name == 'native':
        native_window(surface, check, scratch, record)


# ---- the window ---------------------------------------------------------------------------------------------------------------

def picture_hash(surface):
    return S8.picture_hash(surface)


def drawn_fx(surface):
    drawn = (surface.viewport().get('overlay') or {}).get('drawn') or {}
    return drawn if drawn.get('target') == 'fx' else None


def decode_export(destination):
    raw = subprocess.run(['ffmpeg', '-v', 'error', '-i', str(destination), '-frames:v', '1', '-f', 'rawvideo',
                          '-pix_fmt', 'rgb24', '-'], capture_output=True, check=True).stdout
    if len(raw) != 1920 * 1080 * 3:
        raise S8.ProofFailure(f'the decoded export is {len(raw)} bytes')
    return hashlib.sha256(raw).hexdigest()


def native_window(surface, check, scratch, record):
    lab = surface.lab

    def ok(request, timeout=60):
        good, payload = lab.op(request, timeout)
        if not good:
            raise S8.ProofFailure(f'{request} -> {payload}')
        return payload

    # The panel action is the shared `scene.fx.select`.
    ok({'op': 'scene.fx.select', 'scene': SCENE, 'clip': None})
    node = surface.get(CLIP_NODE)['data']
    off = node['state']['handles'] is False
    toggled = surface.invoke(CLIP_NODE, 'handles')
    gizmo = surface.get('monitor.gizmo')['data']['state']
    on = surface.get(CLIP_NODE)['data']['state']['handles'] is True
    check('the panel\'s `handles` action is the shared `scene.fx.select` and `monitor.gizmo` says `target: fx`',
          off and toggled.get('ok') is True and on and gizmo['target'] == 'fx' and gizmo['fx_clip'] == CLIP,
          (node['state'], toggled, gizmo))
    fade = surface.get('scene.fx.clip:f1')['data']
    action = next(a for a in fade['actions'] if a['action'] == 'handles')
    record['fade_handles_action'] = {'enabled': action['enabled'], 'reason': action.get('disabled_reason')}

    # The page draws the clip's handles.
    drawn = S8.wait_for(lambda: (lambda d: d if d and d['fx']['visible'] else None)(drawn_fx(surface)))
    overlay = surface.viewport().get('overlay') or {}
    check('the page announced `overlay.gizmo` and drew the vignette\'s handles and ellipses',
          overlay.get('supported') is True and drawn is not None
          and sorted(drawn['fx']['handles']) == ['center', 'radius', 'softness'] and drawn['fx']['ellipses'] == 2, overlay)
    probes = {probe['id']: probe['rgba'] for probe in drawn['probes']}
    check('the page painted each handle on the canvas (alpha > 200)',
          set(probes) == {'center', 'radius', 'softness'} and all(p[3] > 200 for p in probes.values()), probes)
    record['drawn_probes'] = probes
    S8.settle_converged(surface)

    # The candidate follows the drag and goes away with the undo.
    shown = ok({'op': 'scene.fx.handles', 'scene': SCENE, 'clip': CLIP, 'frame': FRAME})
    at = handle_of(shown, 'radius')['at']
    initial = picture_hash(surface)
    ok({'op': 'scene.pointer.down', 'scene': SCENE, 'frame': FRAME, 'point': at})
    target = point(at['u'] + 0.08, at['v'])
    ok({'op': 'scene.pointer.move', 'point': target})
    active = S8.wait_for(lambda: (drawn_fx(surface) or {}).get('fx', {}).get('active') == 'radius')
    changed = S8.wait_for(lambda: picture_hash(surface) not in (None, initial))
    S8.settle_converged(surface)
    candidate = picture_hash(surface)
    ok({'op': 'scene.pointer.up', 'point': target})
    S8.settle_converged(surface)
    committed = picture_hash(surface)
    ok({'op': 'history.undo'})
    S8.settle_converged(surface)
    check('the page drew the active handle of the drag', bool(active), surface.viewport().get('overlay'))
    check('the monitor\'s picture follows the candidate and returns to the initial one after undo',
          bool(changed) and candidate == committed and picture_hash(surface) == initial and candidate != initial,
          (initial, candidate, committed, picture_hash(surface)))

    # A dialog hides the WebView: the open drag ends and writes nothing.
    ok({'op': 'scene.fx.select', 'scene': SCENE, 'clip': CLIP})
    before = ok({'op': 'history.status'})['history']['undo_depth']
    ok({'op': 'scene.pointer.down', 'scene': SCENE, 'frame': FRAME, 'point': at})
    ok({'op': 'scene.pointer.move', 'point': target})
    opened = surface.invoke('flow.open')
    hidden = S8.wait_for(lambda: surface.viewport().get('on_screen') is False)
    state = surface.get('monitor.gizmo')['data']['state']
    after = ok({'op': 'history.status'})['history']['undo_depth']
    closed = surface.invoke('dialog.cancel')
    back = S8.wait_for(lambda: surface.viewport().get('on_screen') is True
                       and (drawn_fx(surface) or {}).get('fx', {}).get('visible') is True)
    check('a dialog that hides the WebView cancels the open drag and writes nothing',
          opened.get('ok') is True and bool(hidden) and state['dragging'] is False and after == before
          and closed.get('ok') is True and bool(back), (opened, state, before, after))
    S8.settle_converged(surface)

    # Render and export with the handles on screen are the ones with the handles off.
    pixels = [[min(WIDTH - 1, round(p['at']['u'] * WIDTH)), min(HEIGHT - 1, round(p['at']['v'] * HEIGHT))]
              for p in shown['handles']]

    def render():
        return ok({'op': 'scene.render', 'scene': SCENE, 'frame': FRAME, 'width': WIDTH, 'height': HEIGHT,
                   'probes': pixels})['frame']

    def export(name):
        destination = scratch / f'{name}.mp4'
        body = ok({'op': 'scene.export', 'scene': SCENE, 'preset': 'fullhd', 'destination': str(destination),
                   'from': FRAME, 'frames': 1, 'audio': False, 'resources_root': str(CORPUS)}, 120)
        if body.get('state') == 'queued' or 'job' in body:
            job = body.get('job')
            status = {}

            def finished():
                nonlocal status
                got, status = lab.op({'op': 'job.get', 'job': job})
                return got and status['state'] in ('succeeded', 'failed', 'cancelled')
            if not S8.wait_for(finished, 120, 0.1) or status['state'] != 'succeeded':
                raise S8.ProofFailure(f'the export job did not succeed: {status}')
        return destination

    ok({'op': 'scene.fx.select', 'scene': SCENE, 'clip': CLIP})
    S8.wait_for(lambda: (drawn_fx(surface) or {}).get('fx', {}).get('visible') is True)
    S8.settle_converged(surface)
    with_handles = render()
    exported_with = export('with-handles')
    ok({'op': 'scene.fx.select', 'scene': SCENE, 'clip': None})
    S8.wait_for(lambda: drawn_fx(surface) is None)
    S8.settle_converged(surface)
    without = render()
    exported_without = export('without-handles')
    check('an explicit render with the handles on screen is the render with the handles off',
          with_handles['hash'] == without['hash'] and with_handles.get('probes') == without.get('probes')
          and with_handles.get('inspected') is True and len(with_handles.get('probes', [])) == len(pixels),
          (with_handles, without))
    digest_with, digest_without = decode_export(exported_with), decode_export(exported_without)
    # The control: a committed drag changes the decoded export, so the comparison can tell.
    ok({'op': 'scene.fx.select', 'scene': SCENE, 'clip': CLIP})
    ok({'op': 'scene.pointer.down', 'scene': SCENE, 'frame': FRAME, 'point': at})
    ok({'op': 'scene.pointer.move', 'point': target})
    ok({'op': 'scene.pointer.up', 'point': target})
    S8.settle_converged(surface)
    digest_dragged = decode_export(export('dragged'))
    ok({'op': 'history.undo'})
    check('an FFmpeg export with the handles on screen decodes to the same pixels as with the handles off',
          digest_with == digest_without and digest_dragged != digest_without,
          (digest_with, digest_without, digest_dragged))
    record['render_hash'] = with_handles['hash']
    record['export_digest'] = digest_with


def run_surface(name, binary, report):
    work = tempfile.mkdtemp(prefix='te-f061-', dir='/tmp')
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
                       'the page\'s own mouse handling (runtime/test/fx-handles-overlay.test.mjs)',
                       'performance'],
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
            same = near(h['radius'], n['radius'])
            report['surfaces_agree_on_the_drag'] = same
            if not same:
                raise S8.ProofFailure('headless and native computed different radii for the same drag')
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
