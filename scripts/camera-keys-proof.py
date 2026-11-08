#!/usr/bin/env python3
"""BL140 — camera keyframes of a scene clip, proved on two separate surfaces with the same scenario.

    python3 scripts/directed-scene-proof/with-gpu-lock.py \
        python3 scripts/camera-keys-proof.py [--editor target/debug/tessel-editor] \
        [--headless target/debug/tessel-headless] [--out DIR] [--report FILE]

* native window: the DIRECT editor binary with its own short `HOME`, driven only by its `ui` CLI
  (`op`, `get`, `set`, `invoke`), then captured with the OS (`screencapture -l`): no pointer, no coordinates.
* headless (no window): `tessel-headless`, JSON Lines over stdio. A window's socket is not headless evidence.

The scenario: push-in keys move the planned camera (position, FOV, easing); trimming the start/end and moving
the clip keep every key on its picture; capture at the playhead; the document read back through
`project.save` holds the keys (schema 2); refused edits change nothing (document, revision, undo depth);
one undo per edit. The native run also reads the semantic nodes of the inspector (`camera_keys.capture`,
`camera_key:<clip>:<n>`), edits a key through `set`, and renders monitor frames at three times.

Not proven here: a physical pointer/click on the buttons (the tree, GPUI input tests and `invoke` cover the
controls), the equality of preview and export pixels (`crates/render/tests/export.rs`, real runtime).
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from proof_surface import MIN_FREE_GIB, Headless, Native, ProofFailure, free_gib, run_surface  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
TOLERANCE = 1e-9


def scenario(surface, check, scratch, fixtures):
    def ok(request):
        good, payload = surface.op(request)
        if not good:
            raise ProofFailure(f'{request} -> {payload}')
        return payload

    def refused(request, code):
        good, payload = surface.op(request)
        if good or payload.get('code') != code:
            raise ProofFailure(f'{request} should be refused with {code}, got {good} {payload}')

    def camera(frame):
        return ok({'op': 'plan', 'frame': frame})['layers'][0]['camera']

    def z(frame):
        return camera(frame)['position'][2]

    saves = [0]

    def document():
        saves[0] += 1
        path = scratch / f'doc-{surface.name}-{saves[0]}.tessel.json'
        ok({'op': 'project.save', 'path': str(path)})
        return json.loads(path.read_text())

    def scene_clip(doc):
        clips = doc['sequences'][0]['clips']
        return next(clip for clip in clips if clip['id'] == 'c-scene')

    def history():
        data = ok({'op': 'history.status'})
        return data['history']['undo_depth'], data['project']['revision']

    def key(time_s, depth, fov=50, easing='linear'):
        return {'op': 'clip.camera_key', 'clip': 'c-scene', 'time': time_s, 'position': [0, 1.2, depth],
                'target': [0, 0, 0], 'fov': fov, 'easing': easing}

    near = lambda a, b: abs(a - b) <= TOLERANCE + 1e-12 * max(abs(a), abs(b))  # noqa: E731

    lines = [json.loads(line) for line in (fixtures / 'proof.jsonl').read_text().splitlines() if line.strip()]
    for request in lines:
        ok(request)
    ok({'op': 'project.save', 'path': str(scratch / f'{surface.name}.tessel.json')})
    ok({'op': 'clip.camera', 'clip': 'c-scene', 'position': [0, 1.2, 5], 'target': [0, 0, 0], 'fov': 50, 'orbit': 0})
    check('the proof project loads', len(ok({'op': 'status'})['timeline']['clips']) == 5)
    base_doc = document()
    check('a project without keys is saved in the oldest schema, with no `camera_keys`',
          base_doc['schema_version'] == 1 and 'camera_keys' not in scene_clip(base_doc), base_doc['schema_version'])
    still = camera(24)

    # --- keys drive the planned camera ---------------------------------------------------------
    depth0, revision0 = history()
    ok(key(0, 10, fov=60))
    ok(key(2, 4, fov=40))
    check('frame 0 is the first key', near(z(0), 10.0), z(0))
    check('1 s is halfway (linear): z 7, fov 50', near(z(24), 7.0) and near(camera(24)['fov'], 50.0), camera(24))
    check('frame 47 (≈ 1.96 s) is still on the way to the last key at 2 s (z between 4 and 4.3)',
          4.0 < z(47) < 4.3, z(47))
    saved = document()
    check('the saved document holds both keys and uses schema 2',
          saved['schema_version'] == 2 and len(scene_clip(saved)['camera_keys']) == 2, scene_clip(saved))
    depth1, revision1 = history()
    check('two keys = two undo entries and the revision moved', depth1 == depth0 + 2 and revision1 > revision0,
          (depth0, depth1, revision0, revision1))

    # --- hold: an instant strictly after the last key --------------------------------------------
    # Two keys that end at 1 s; frames 36 (1.5 s) and 47 (≈ 1.96 s) are after the last one.
    ok({'op': 'clip.camera_keys', 'clip': 'c-scene', 'keys': [
        {'time': 0, 'position': [0, 1.2, 10], 'target': [0, 0, 0], 'fov': 60, 'easing': 'linear'},
        {'time': 1, 'position': [0, 1.2, 4], 'target': [0, 0, 0], 'fov': 40, 'easing': 'linear'}]})
    check('after the last key (1 s) the camera holds it exactly at frames 36 and 47 (z 4, fov 40)',
          near(z(36), 4.0) and near(z(47), 4.0) and near(camera(47)['fov'], 40.0),
          (camera(36), camera(47)))
    ok({'op': 'clip.camera_keys', 'clip': 'c-scene', 'keys': [
        {'time': 0, 'position': [0, 1.2, 10], 'target': [0, 0, 0], 'fov': 60, 'easing': 'linear'},
        {'time': 2, 'position': [0, 1.2, 4], 'target': [0, 0, 0], 'fov': 40, 'easing': 'linear'}]})

    # --- easing ---------------------------------------------------------------------------------
    ok(key(0, 10, fov=60, easing='easeOutCubic'))
    check('easeOutCubic at half = 1 - 0.5^3 of the way (z = 10 - 6 * 0.875)', near(z(24), 10 - 6 * 0.875), z(24))
    # Re-keying the same time WITHOUT an easing keeps the key's easing (omission is not `linear`).
    omitted = key(0, 10, fov=60)
    del omitted['easing']
    ok(omitted)
    check('a key set without an easing keeps the easing of the key it replaces',
          scene_clip(document())['camera_keys'][0]['easing'] == 'easeOutCubic',
          scene_clip(document())['camera_keys'][0])
    ok(key(0, 10, fov=60, easing='linear'))
    check('an explicit easing is respected',
          scene_clip(document())['camera_keys'][0]['easing'] == 'linear')

    # --- trim and move keep the keys on their picture -------------------------------------------
    good, payload = surface.set('trim:c-scene:start', 'frame', 12)
    check('the start trim is accepted', good, payload)
    check('after trimming the start to 0.5 s the first frame shows scene time 0.5 s (z 8.5)',
          near(z(12), 8.5), z(12))
    good, payload = surface.move('clip:c-scene', 0)
    check('the move is accepted', good, payload)
    check('moving the clip changes nothing about its picture (frame 0 still z 8.5, frame 24 z 5.5)',
          near(z(0), 8.5) and near(z(24), 5.5), (z(0), z(24)))
    ok({'op': 'history.undo'})
    ok({'op': 'history.undo'})

    # --- capture at the playhead ----------------------------------------------------------------
    ok({'op': 'seek', 'frame': 24})
    surface.invoke('clip:c-scene', 'select')
    good, payload = surface.invoke('camera_keys.capture', 'invoke')
    check('capture at the playhead adds a key on the current curve', good and payload['count'] == 3
          and near(z(24), 7.0), payload)
    keys = scene_clip(document())['camera_keys']
    check('the captured key sits at 1 s with the evaluated camera',
          near(keys[1]['time']['seconds'], 1.0) and near(keys[1]['position'][2], 7.0), keys[1])

    # --- refused edits change nothing -------------------------------------------------------------
    before_doc, before_history = document(), history()
    refused(key(4, -10), 'invalid_value')        # straight through the target
    refused(key(3, 4, fov=0.2), 'invalid_value')
    refused(key(3, 4, easing='bounce'), 'invalid_value')
    refused({'op': 'clip.camera_key', 'clip': 'c-img', 'time': 1, 'position': [0, 1, 5]}, 'not_applicable')
    refused({'op': 'clip.camera', 'clip': 'c-scene', 'position': [0, 1, 5], 'orbit': 30}, 'camera_orbit_conflict')
    refused({'op': 'clip.camera', 'clip': 'c-scene', 'position': [0, 2, 7], 'orbit': 0}, 'camera_keys_override')
    refused({**key(3, 4), 'orbit': 30}, 'camera_orbit_conflict')
    refused(key(3, 4, fov=170), 'invalid_value')    # the range is 1 up to, excluding, 170
    check('refused edits leave the document, revision and undo depth untouched',
          document() == before_doc and history() == before_history)

    # --- edit and remove through the target vocabulary -------------------------------------------
    good, payload = surface.set('camera_key:c-scene:2', 'fov', 35)
    check('a key field is set by target', good and near(scene_clip(document())['camera_keys'][2]['fov'], 35.0),
          payload)
    good, payload = surface.invoke('camera_key:c-scene:2', 'remove')
    check('a key is removed by target', good and payload['count'] == 2, payload)
    good, payload = surface.invoke('camera_keys.clear', 'invoke')
    check('clear removes every key and the camera is the clip camera again',
          good and payload['count'] == 0 and camera(24) == still, payload)
    ok({'op': 'history.undo'})
    check('one undo brings the keys back', len(scene_clip(document())['camera_keys']) == 2)

    # --- the head trim at another speed keeps the picture (core rule, both surfaces) ---------------
    # Keys 0 s (z 10) and 8 s (z 2): z = 10 - scene time. At 200 % frame 24 (1 s of timeline) reads
    # scene time 2 s (z 8). Trimming the head to frame 12 (0.5 s) must not move that frame: an
    # unscaled shift would make it read 1.5 s (z 8.5), the bug of BL140.2.
    ok({'op': 'clip.camera_keys', 'clip': 'c-scene', 'keys': [
        {'time': 0, 'position': [0, 1.2, 10], 'target': [0, 0, 0], 'fov': 50, 'easing': 'linear'},
        {'time': 8, 'position': [0, 1.2, 2], 'target': [0, 0, 0], 'fov': 50, 'easing': 'linear'}]})
    ok({'op': 'clip.property', 'clip': 'c-scene', 'property': 'speed', 'value': 200})
    check('at 200 % frame 24 reads scene time 2 s (z 8)', near(z(24), 8.0), z(24))
    good, payload = surface.set('trim:c-scene:start', 'frame', 12)
    check('the head trim at 200 % is accepted', good, payload)
    check('the head trim at 200 % keeps frame 24 on scene time 2 s (z 8, not 8.5)', near(z(24), 8.0), z(24))
    good, payload = surface.set('trim:c-scene:start', 'frame', 0)
    check('trimming the head back at the same speed restores the clip', good and near(z(24), 8.0), payload)
    ok({'op': 'clip.property', 'clip': 'c-scene', 'property': 'speed', 'value': 100})

    if surface.name != 'native':
        return

    # --- native window: semantic nodes, monitor frames, OS capture ------------------------------------
    good, section = surface.get('inspector.camera')
    check('the camera section is a visible accordion for the selected scene clip',
          good and section['visible'] and section['kind'] == 'accordion', section)
    good, node = surface.get('camera_key:c-scene:0')
    check('a key is a node with nine editable fields and a remove action',
          good and len(node['fields']) == 9 and node['actions'][0]['action'] == 'remove', node)
    good, capture = surface.get('camera_keys.capture')
    check('the add button is a semantic node', good and capture['enabled'], capture)
    hashes = []
    for frame in (0, 24, 47):
        shot = ok({'op': 'render.frame', 'frame': frame})
        hashes.append(shot['frame'].get('hash'))
    check('the monitor draws three different pictures at three times (hash)',
          len(set(hashes)) == 3 and all(hashes), hashes)
    # Only the scene on the monitor and the camera section open in the inspector, for the capture.
    for track in ('media-track', 'title-track'):
        surface.set(f'track:{track}', 'hidden', 'true')
    for section in ('transform', 'composite', 'timing'):
        surface.set(f'inspector.{section}', 'expanded', 'false')
    ok({'op': 'seek', 'frame': 24})
    path, reason = surface.capture(scratch / 'bl140-camera-keys-window.png')
    if path:
        check(f'the native window was captured ({path})', True)
    else:
        # Not a pass: the report says SKIP and the run is `complete: false`.
        check.skip('the native window OS capture', reason)
    # The add button announces why it cannot work while the camera orbits (and the command agrees).
    ok({'op': 'clip.camera_keys', 'clip': 'c-scene', 'keys': []})
    ok({'op': 'clip.camera', 'clip': 'c-scene', 'position': [0, 1.2, 5], 'target': [0, 0, 0], 'fov': 50, 'orbit': 30})
    good, capture = surface.get('camera_keys.capture')
    check('the add button is disabled with a reason while the camera orbits',
          good and capture['enabled'] is False and 'orbit' in str(capture.get('disabled_reason', '')), capture)
    refused({'op': 'clip.camera_key.capture', 'clip': 'c-scene'}, 'camera_orbit_conflict')
    return path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--editor', default=str(ROOT / 'target/debug/tessel-editor'))
    parser.add_argument('--headless', default=str(ROOT / 'target/debug/tessel-headless'))
    parser.add_argument('--out', default=str(ROOT / 'target/bl140-proof'))
    parser.add_argument('--report')
    parser.add_argument('--require-capture', action='store_true',
                        help='a missing OS window capture is a failure (default: an explicit SKIP)')
    args = parser.parse_args()
    if free_gib() < MIN_FREE_GIB:
        print(f'refusing to start: less than {MIN_FREE_GIB} GiB free', file=sys.stderr)
        return 2
    report = {'surfaces': {}, 'error': None}

    def finish(code):
        if args.report:
            try:
                Path(args.report).write_text(json.dumps(report, indent=2))
            except OSError as error:
                print(f'could not write the report: {error}', file=sys.stderr)
                return code or 1
        return code

    out = Path(args.out).resolve()
    fixtures = out / 'fixtures'
    try:
        out.mkdir(parents=True, exist_ok=True)
        shutil.rmtree(fixtures, ignore_errors=True)
        subprocess.run([str(ROOT / 'scripts/fixtures'), str(fixtures)], check=True, capture_output=True,
                       timeout=600)
    except (OSError, subprocess.SubprocessError) as error:
        report['error'] = f'fixtures: {type(error).__name__}: {error}'
        print(f"FAILED {report['error']}", file=sys.stderr)
        return finish(1)
    failed = False
    complete = True
    # One surface failing never stops the other, and the report is written either way.
    for cls, binary in ((Headless, args.headless), (Native, args.editor)):
        result = run_surface(cls, binary, lambda surface, check, scratch: scenario(surface, check, scratch,
                                                                                  fixtures), out, 'tck')
        report['surfaces'][result['name']] = result
        failed = failed or result['failed']
        complete = complete and result['complete']
    print(json.dumps({name: f"{s['passed']}/{len(s['checks'])} passed, {s['skipped']} skipped"
                      + (f", FAILED: {s['error']}" if s['failed'] else '')
                      for name, s in report['surfaces'].items()}))
    if not failed and not complete:
        print('INCOMPLETE: at least one check was skipped (see the SKIP lines); this is not a full pass',
              file=sys.stderr)
        if args.require_capture:
            failed = True
    report['complete'] = complete and not failed
    return finish(1 if failed else 0)


if __name__ == '__main__':
    sys.exit(main())
