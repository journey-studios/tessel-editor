#!/usr/bin/env python3
"""BL143 — a SHARP PLY imported directly, proved on two separate surfaces with the same scenario.

    python3 scripts/directed-scene-proof/with-gpu-lock.py \
        python3 scripts/sharp-import-proof.py [--editor target/debug/tessel-editor] \
        [--headless target/debug/tessel-headless] [--out DIR] [--report FILE]

* native window: the DIRECT editor binary with its own short `HOME`, driven only by its `ui` CLI
  (`op`, `get`, `invoke`), then captured with the OS (`screencapture -l`): no pointer, no coordinates.
  The monitor (a real WebView + Three.js + Spark) draws the file; five probes read the pixels.
* headless (no window): `tessel-headless`, JSON Lines over stdio. A window's socket is not headless evidence.

The file is synthetic (`./scripts/fixtures` writes `sharp.ply`; no model, no weights, no generated media):
five Gaussians over five known pixels of a 160x120 source image. Axes, up direction, field of view and
camera come from the file's own `intrinsic`/`image_size` elements (Apple `ml-sharp`, `save_ply`), never from
a number typed here. The expected frame pixel of each Gaussian is computed in this script from the
intrinsics with the pinhole model, independently of the Rust and JS code under test.

Proved on both: the capture is read (size, focal length, FOV 50.0°, OpenCV axes -> 180° about X); a clip of
the splat starts on the capture camera; a file with missing or contradictory metadata is refused with its
code and the project is untouched (document, revision, undo depth), then imports with `capture: ignore`;
save/reopen keep the capture (schema 2; a project with no capture keeps schema 1); the capture camera is
restored by `clip.camera_capture`, in one undo step. Native only: the semantic nodes, five pixel probes at the
predicted places, the mirror image of the same file (plain import) not landing there, the OS capture.

Not proven here: a physical pointer on the button (the tree, GPUI input tests and `invoke` cover it),
the pixels of the export (`crates/render/tests/sharp_frames.rs`, real runtime + FFmpeg), a real SHARP output
(no model is run in this repository; the format follows the primary source and the synthetic file).
"""
import argparse
import json
import math
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
from proof_surface import MIN_FREE_GIB, Headless, Native, ProofFailure, Recorder, free_gib  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
FRAME = (320, 180)
SOURCE_PIXELS = [(40, 30), (120, 30), (40, 90), (120, 90), (80, 60)]
# Which channels light up on a black frame for each point of the fixture.
DOMINANT = [(1, 0, 0), (0, 1, 0), (0, 0, 1), (1, 1, 0), (1, 1, 1)]


def write_ply(path, size=True, intrinsic=None):
    """A one-Gaussian SHARP-like PLY (vertex + extrinsic + intrinsic [+ image_size]) written from the spec."""
    intrinsic = intrinsic or [128.0, 0.0, 80.0, 0.0, 128.0, 60.0, 0.0, 0.0, 1.0]
    header = ['ply', 'format binary_little_endian 1.0', 'element vertex 1']
    header += [f'property float {n}' for n in
               ('x', 'y', 'z', 'f_dc_0', 'f_dc_1', 'f_dc_2', 'opacity', 'scale_0', 'scale_1', 'scale_2',
                'rot_0', 'rot_1', 'rot_2', 'rot_3')]
    body = struct.pack('<14f', 0, 0, 4, 1, 1, 1, 2, -3, -3, -3, 1, 0, 0, 0)
    identity = [1.0 if i % 5 == 0 else 0.0 for i in range(16)]
    header += ['element extrinsic 16', 'property float extrinsic']
    body += struct.pack('<16f', *identity)
    header += ['element intrinsic 9', 'property float intrinsic']
    body += struct.pack('<9f', *intrinsic)
    if size:
        header += ['element image_size 2', 'property uint image_size']
        body += struct.pack('<2I', 160, 120)
    path.write_bytes(('\n'.join(header) + '\nend_header\n').encode() + body)


def frame_pixel(capture, source, frame=FRAME):
    """Where the pixel `source` of the source image lands in a `frame`-sized picture through the editor camera.

    A point at depth Z over source pixel (u, v) is X = (u - cx) Z / fx, Y = (v - cy) Z / fy in the file's axes.
    After the half turn about X the editor sees (X, -Y, -Z) from the origin looking down -z with the vertical
    field of view of the intrinsics; Three.js projects that with `tan(fov / 2)` and the frame aspect.
    """
    tan = math.tan(math.radians(capture['vfov']) / 2)
    aspect = frame[0] / frame[1]
    ndc_x = (source[0] - capture['cx']) / (capture['fx'] * tan * aspect)
    ndc_y = -(source[1] - capture['cy']) / (capture['fy'] * tan)
    return round((ndc_x + 1) / 2 * frame[0]), round((1 - ndc_y) / 2 * frame[1])


def lit(color, dominant):
    return all((c > 150) if want else (c < 90) for c, want in zip(color, dominant))


def scenario(surface, check, scratch, fixtures):
    def ok(request):
        good, payload = surface.op(request)
        if not good:
            raise ProofFailure(f'{request} -> {payload}')
        return payload

    def render(request, seconds=90):
        """`render.frame` once the viewport's WebView is up (a cold window answers `backend_unavailable`)."""
        deadline = time.time() + seconds
        while True:
            good, payload = surface.op(request)
            if good or payload.get('code') != 'backend_unavailable' or time.time() > deadline:
                if not good:
                    raise ProofFailure(f'{request} -> {payload}')
                return payload
            time.sleep(1)

    def refused(request, code):
        good, payload = surface.op(request)
        check(f'{request["op"]} is refused with {code}', not good and payload.get('code') == code, payload)
        return payload

    saves = [0]

    def document():
        saves[0] += 1
        path = scratch / f'doc-{surface.name}-{saves[0]}.tessel.json'
        ok({'op': 'project.save', 'path': str(path)})
        return json.loads(path.read_text())

    def history():
        data = ok({'op': 'history.status'})
        return data['history']['undo_depth'], data['project']['revision']

    def layer(frame=0):
        return ok({'op': 'plan', 'frame': frame})['layers'][0]

    sharp = fixtures / 'sharp.ply'
    ok({'op': 'project.create', 'name': 'SHARP', 'sequence': 'main', 'rate': 24, 'width': FRAME[0],
        'height': FRAME[1], 'length': 48})
    ok({'op': 'track.add', 'id': 't-shot', 'name': 'shot', 'kind': 'video'})
    ok({'op': 'project.save', 'path': str(scratch / f'{surface.name}.tessel.json')})
    check('a project with no capture is saved in the oldest schema', document()['schema_version'] == 1)

    # --- the capture is read from the file -------------------------------------------------------
    added = ok({'op': 'asset.add', 'kind': 'splat', 'id': 'shot', 'name': 'shot', 'path': str(sharp)})
    capture = added['asset']['capture']
    check('the file declares 160x120, fx = fy, centred principal point',
          capture['image'] == [160, 120] and abs(capture['fx'] - capture['fy']) < 1e-3
          and (capture['cx'], capture['cy']) == (80.0, 60.0), capture)
    check('the vertical field of view is 2 atan(H / 2 fy) = 50.0 degrees',
          abs(capture['vfov'] - 2 * math.degrees(math.atan(120 / (2 * capture['fy'])))) < 1e-6
          and abs(capture['vfov'] - 50.0) < 1e-3, capture['vfov'])
    check('OpenCV axes (up = -y) become the editor axes by a 180 degree turn about X',
          capture['axes'] == 'opencv' and capture['up'] == [0.0, -1.0, 0.0]
          and capture['orientation'] == [180.0, 0.0, 0.0], capture)
    check('the camera is the origin looking down -z', capture['camera']['position'] == [0.0, 0.0, 0.0]
          and capture['camera']['target'] == [0.0, 0.0, -1.0], capture['camera'])
    check('a 1.x sRGB file imports with no warning', 'warnings' not in added, added.get('warnings'))

    # --- a splat clip starts on the capture camera -------------------------------------------------
    ok({'op': 'clip.add', 'id': 'c-shot', 'track': 't-shot', 'asset': 'shot', 'start': 0, 'duration': 48})
    first = layer(0)
    check('the clip is planned from the capture camera with the axis turn',
          first['camera']['position'] == [0.0, 0.0, 0.0] and abs(first['camera']['fov'] - 50.0) < 1e-3
          and first['camera']['orbit'] == 0 and first['orientation'] == [180.0, 0.0, 0.0], first)
    check('the camera is still: frame 30 equals frame 0', layer(30)['camera'] == first['camera'])

    # --- refused metadata changes nothing --------------------------------------------------------------
    broken = scratch / 'broken.ply'
    odd = scratch / 'odd.ply'
    write_ply(broken, size=False)
    write_ply(odd, intrinsic=[100.0, 0.0, 80.0, 0.0, 130.0, 60.0, 0.0, 0.0, 1.0])
    before_doc, before_history = document(), history()
    incomplete = refused({'op': 'asset.add', 'kind': 'splat', 'id': 'broken', 'name': 'broken',
                          'path': str(broken)}, 'incomplete_capture')
    check('the refusal says how to go on', 'ignore' in incomplete.get('message', ''), incomplete)
    contradictory = refused({'op': 'asset.add', 'kind': 'splat', 'id': 'odd', 'name': 'odd',
                             'path': str(odd)}, 'ambiguous_capture')
    check('the contradictory file says why', 'fx=100' in contradictory.get('message', ''), contradictory)
    refused({'op': 'asset.add', 'kind': 'splat', 'id': 'cut', 'name': 'cut',
             'path': str(fixtures / 'tone-440.wav')}, 'invalid_asset')
    refused({'op': 'asset.add', 'kind': 'splat', 'id': 'x', 'name': 'x', 'path': str(sharp),
             'capture': 'maybe'}, 'invalid_request')
    check('refused imports leave the document, revision and undo depth untouched',
          document() == before_doc and history() == before_history)
    plain = ok({'op': 'asset.add', 'kind': 'splat', 'id': 'plain', 'name': 'plain', 'path': str(broken),
                'capture': 'ignore'})
    check('the same file imports as a plain splat with `capture: ignore`', plain['asset']['capture'] is None)
    ok({'op': 'asset.add', 'kind': 'splat', 'id': 'mirror', 'name': 'mirror', 'path': str(sharp),
        'capture': 'ignore'})
    check('a plain Gaussian PLY (no capture elements) is not a capture either',
          ok({'op': 'asset.add', 'kind': 'splat', 'id': 'cloud', 'name': 'cloud',
              'path': str(fixtures / 'cloud.splat')})['asset']['capture'] is None)

    # --- save, reopen -----------------------------------------------------------------------------
    saved = document()
    stored = next(a for a in saved['library']['assets'] if a['id'] == 'shot')['asset']['meta']['capture']
    check('the document is schema 2 and stores the intrinsics, not the derived field of view',
          saved['schema_version'] == 2 and stored['image'] == [160, 120] and 'vfov' not in stored, stored)
    reopened_path = scratch / f'reopen-{surface.name}.tessel.json'
    ok({'op': 'project.save', 'path': str(reopened_path)})
    ok({'op': 'project.open', 'path': str(reopened_path)})
    check('reopening gives back the same capture camera', layer(0) == first, layer(0))

    # --- restore the capture camera, one undo each -------------------------------------------------
    ok({'op': 'clip.camera', 'clip': 'c-shot', 'position': [3, 1, 6], 'target': [0, 0, 0], 'fov': 30,
        'orbit': 0})
    check('an edited camera is no longer the capture camera', layer(0)['camera']['position'] == [3.0, 1.0, 6.0])
    depth, _ = history()
    restored = ok({'op': 'clip.camera_capture', 'clip': 'c-shot'})
    check('`clip.camera_capture` puts the capture camera back', restored['changed'] and layer(0) == first,
          restored)
    check('it is one undo step', history()[0] == depth + 1)
    ok({'op': 'history.undo'})
    check('undo brings the edited camera back', layer(0)['camera']['position'] == [3.0, 1.0, 6.0])
    refused({'op': 'clip.camera_capture', 'clip': 'c-shot-missing'}, 'unknown_element')
    ok({'op': 'clip.camera_capture', 'clip': 'c-shot'})

    if surface.name != 'native':
        return

    # --- native window: nodes, five pixel probes, the plain import, OS capture ---------------------
    good, shot = surface.get('media:shot')
    check('the library node names the capture',
          good and shot['state']['capture']['source'] == 'sharp' and 'SHARP 160×120 · FOV 50.0°' in
          shot['state']['meta'], shot)
    good, plain_node = surface.get('media:plain')
    check('the plain import has no capture', good and plain_node['state']['capture'] is None, plain_node)
    surface.invoke('clip:c-shot', 'select')
    good, button = surface.get('camera.capture')
    check('the camera section offers the capture camera', good and button['enabled']
          and button['state']['matches'] is True, button)
    probes = [frame_pixel(capture, pixel) for pixel in SOURCE_PIXELS]
    result = render({'op': 'render.frame', 'frame': 12, 'probes': [list(p) for p in probes]})
    colors = [tuple(p) for p in result['frame']['probes']]
    check('each Gaussian is drawn where the intrinsics put it (' + ', '.join(map(str, probes)) + ')',
          all(lit(c[:3], d) for c, d in zip(colors, DOMINANT)), colors)

    # The same file imported as a plain splat, from the default camera: not at those pixels.
    ok({'op': 'track.add', 'id': 't-plain', 'name': 'plain', 'kind': 'video'})
    ok({'op': 'clip.add', 'id': 'c-mirror', 'track': 't-plain', 'asset': 'mirror', 'start': 0,
        'duration': 48})
    surface.set('track:t-shot', 'hidden', 'true')
    wrong = render({'op': 'render.frame', 'frame': 12, 'probes': [list(p) for p in probes]})
    wrong_colors = [tuple(p) for p in wrong['frame']['probes']]
    check('the plain import of the same bytes does not land on the predicted pixels',
          not all(lit(c[:3], d) for c, d in zip(wrong_colors, DOMINANT)), wrong_colors)
    surface.set('track:t-shot', 'hidden', 'false')
    surface.set('track:t-plain', 'hidden', 'true')
    surface.invoke('clip:c-shot', 'select')
    for section in ('transform', 'composite', 'timing'):
        surface.set(f'inspector.{section}', 'expanded', 'false')
    surface.set('inspector.camera', 'expanded', 'true')
    ok({'op': 'seek', 'frame': 12})
    path, reason = surface.capture(scratch / 'bl143-sharp-import-window.png')
    check(f'the native window was captured ({path})' if path else f'native capture unavailable: {reason}', True)
    return path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--editor', default=str(ROOT / 'target/debug/tessel-editor'))
    parser.add_argument('--headless', default=str(ROOT / 'target/debug/tessel-headless'))
    parser.add_argument('--out', default=str(ROOT / 'target/bl143-proof'))
    parser.add_argument('--report')
    args = parser.parse_args()
    if free_gib() < MIN_FREE_GIB:
        print(f'refusing to start: less than {MIN_FREE_GIB} GiB free', file=sys.stderr)
        return 2
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    fixtures = out / 'fixtures'
    shutil.rmtree(fixtures, ignore_errors=True)
    subprocess.run([str(ROOT / 'scripts/fixtures'), str(fixtures)], check=True, capture_output=True)
    report = {'surfaces': {}}
    failed = False
    for cls in (Headless, Native):
        home = tempfile.mkdtemp(prefix='shp') if cls is Native else None
        surface = cls(args.headless if cls is Headless else args.editor, home)
        check = Recorder(surface.name)
        shot = None
        try:
            surface.prepare()
            scratch = out / surface.name
            shutil.rmtree(scratch, ignore_errors=True)
            scratch.mkdir(parents=True)
            shot = scenario(surface, check, scratch, fixtures)
        except (ProofFailure, RuntimeError) as error:
            failed = True
            print(f'FAILED [{surface.name}]: {error}', file=sys.stderr)
        finally:
            surface.stop()
            if home:
                shutil.rmtree(home, ignore_errors=True)
        report['surfaces'][surface.name] = {'checks': check.checks, 'passed': sum(c['ok'] for c in check.checks),
                                            'screenshot': shot}
    print(json.dumps({name: f"{s['passed']}/{len(s['checks'])}" for name, s in report['surfaces'].items()}))
    if args.report:
        Path(args.report).write_text(json.dumps(report, indent=2))
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main())
