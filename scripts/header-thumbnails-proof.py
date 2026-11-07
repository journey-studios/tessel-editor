#!/usr/bin/env python3
"""BL144 — the header follows the open project and the library draws real splat/scene thumbnails,
proved on two separate surfaces with the same scenario.

    python3 scripts/directed-scene-proof/with-gpu-lock.py \
        python3 scripts/header-thumbnails-proof.py [--editor target/debug/tessel-editor] \
        [--render target/debug/tessel-render] [--out DIR] [--report FILE]

* headless (no window): `tessel-render`, JSON Lines over stdio, the real Node + Chrome renderer. A window's
  socket is not headless evidence.
* native window: the DIRECT editor binary with its own short `HOME` (own socket, settings, recents: the
  user's window and project are never touched), driven only by its `ui` CLI (`op`, `get`). No pointer, no
  coordinates. The window schedules the thumbnail jobs itself; the script only reads the tree.

The files are the synthetic fixtures `./scripts/fixtures` writes (a splat cloud and a scene with an orange box,
a blue sphere and that cloud). Nothing is generated, no model is run, no private media is used.

Oracle, computed here from the default scene camera (`[0,1,5]` looking at the origin, 50 degrees) with a plain
pinhole and independently of the Rust and JS code under test: the thumbnail PNG of the scene must be orange at
the projected box centre, blue at the projected sphere centre and background in the corners; the thumbnail of
the bare splat must show the cloud at the centre and neither the box nor the sphere.

Proved on both: `project.header` and `status.header` name `Project / Sequence` after create, open and a second
open; a thumbnail job draws one PNG per splat and scene at the sequence aspect and size; the pixels match the
oracle; a picture is never applied to another project (the same asset IDs and files are in both, on purpose); an
unknown asset is refused and the project is untouched (revision, undo depth). Native only: the semantic
`node:13` label equals the header, the tree's `thumbnail` state per `media:<id>`, a thumbnail job that was
running when the project was replaced is cancelled, and the OS capture of the window.

Not proven here: a physical pointer, a real GPU (SwiftShader), a real SHARP file (BL143, another PR).
"""
import argparse
import json
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MIN_FREE_GIB = 12
FRAME = (320, 180)
WIDTH = 240
# `tessel_editor_core::thumbnails::size_for(320, 180)`: 240 across, an even height.
HEIGHT = 136
BACKGROUND = (10, 13, 26)


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


class Headless:
    name = 'headless'

    def __init__(self, binary, _home=None):
        self.process = subprocess.Popen([binary], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=subprocess.DEVNULL, text=True, bufsize=1)

    def op(self, request):
        self.process.stdin.write(json.dumps(request) + '\n')
        self.process.stdin.flush()
        return normalize(json.loads(self.process.stdout.readline()))

    def get(self, target):
        return False, {'code': 'desktop_required', 'message': 'no tree without a window'}

    def prepare(self):
        pass

    def stop(self):
        self.process.stdin.close()
        self.process.wait(60)


class Native:
    name = 'native'

    def __init__(self, binary, home):
        self.binary = binary
        self.env = {**os.environ, 'HOME': home}
        self.socket = os.path.join(home, '.cache/tessel-editor/ui.sock')
        self.log_path = os.path.join(home, 'editor.log')
        self.log = open(self.log_path, 'w')
        self.process = subprocess.Popen([binary], env=self.env, stdout=self.log, stderr=self.log,
                                        start_new_session=True)

    def ui(self, *args, timeout=90):
        done = subprocess.run([self.binary, 'ui', *args], env=self.env, capture_output=True, text=True,
                              timeout=timeout)
        try:
            return json.loads(done.stdout)
        except ValueError:
            raise RuntimeError(f'ui {args}: no JSON ({done.returncode}): {done.stdout[:200]} {done.stderr[:200]}')

    def op(self, request):
        return normalize(self.ui('op', json.dumps(request)))

    def get(self, target):
        return normalize(self.ui('get', target))

    def invoke(self, target):
        return normalize(self.ui('invoke', target))

    def prepare(self, seconds=90):
        deadline = time.time() + seconds
        while time.time() < deadline:
            if self.process.poll() is not None:
                raise RuntimeError(f'the editor exited ({self.process.returncode}): '
                                   f'{Path(self.log_path).read_text()[-500:]}')
            if os.path.exists(self.socket):
                try:
                    if self.ui('status', timeout=10).get('ok'):
                        return
                except (RuntimeError, subprocess.TimeoutExpired):
                    pass
            time.sleep(0.25)
        raise RuntimeError('the window never answered on its socket')

    def stop(self):
        self.process.terminate()
        try:
            self.process.wait(15)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
        self.log.close()

    def capture(self, path):
        self.invoke('window.activate')
        time.sleep(1.0)
        wid = window_id(self.process.pid)
        if not wid:
            return None, 'no on-screen window for this PID (screen locked, covered or capture denied)'
        run = subprocess.run(['screencapture', '-x', '-o', '-l', wid, str(path)], capture_output=True, text=True)
        if run.returncode != 0 or not os.path.exists(path):
            return None, f'screencapture failed ({run.returncode}): {run.stderr.strip()[:120]}'
        return str(path), ''


def window_id(pid):
    script = (
        'import CoreGraphics\n'
        'let l = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as! [[String: Any]]\n'
        'var best = 0; var area = 0.0\n'
        'for w in l {\n'
        f'  if (w[kCGWindowOwnerPID as String] as? Int) == {pid} && (w[kCGWindowLayer as String] as? Int) == 0 {{\n'
        '    let b = w[kCGWindowBounds as String] as! [String: Double]\n'
        '    let a = (b["Width"] ?? 0) * (b["Height"] ?? 0)\n'
        '    if a > area { area = a; best = w[kCGWindowNumber as String] as! Int }\n'
        '  }\n'
        '}\n'
        'print(best)\n'
    )
    out = subprocess.run(['swift', '-e', script], capture_output=True, text=True).stdout.strip()
    return out if out and out != '0' else None


# ---- the oracle: a PNG reader and a pinhole, both independent of the code under test ----

def read_png(path):
    """`(width, height, rows)` of an 8-bit non-interlaced RGB/RGBA PNG; `rows[y][x]` is `(r, g, b)`."""
    data = Path(path).read_bytes()
    assert data[:8] == b'\x89PNG\r\n\x1a\n', f'{path} is not a PNG'
    at, idat, header = 8, b'', None
    while at < len(data):
        length, kind = struct.unpack('>I4s', data[at:at + 8])
        chunk = data[at + 8:at + 8 + length]
        if kind == b'IHDR':
            header = struct.unpack('>IIBBBBB', chunk)
        elif kind == b'IDAT':
            idat += chunk
        at += 12 + length
    width, height, depth, color, _, _, interlace = header
    assert depth == 8 and color in (2, 6) and interlace == 0, header
    bpp = 3 if color == 2 else 4
    raw = zlib.decompress(idat)
    stride = width * bpp
    rows, previous = [], bytearray(stride)
    for y in range(height):
        line = bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        filter_type = raw[y * (stride + 1)]
        for i in range(stride):
            left = line[i - bpp] if i >= bpp else 0
            up = previous[i]
            corner = previous[i - bpp] if i >= bpp else 0
            if filter_type == 1:
                line[i] = (line[i] + left) & 255
            elif filter_type == 2:
                line[i] = (line[i] + up) & 255
            elif filter_type == 3:
                line[i] = (line[i] + (left + up) // 2) & 255
            elif filter_type == 4:
                guess = left + up - corner
                pa, pb, pc = abs(guess - left), abs(guess - up), abs(guess - corner)
                pick = left if pa <= pb and pa <= pc else up if pb <= pc else corner
                line[i] = (line[i] + pick) & 255
        previous = line
        rows.append([tuple(line[x * bpp:x * bpp + 3]) for x in range(width)])
    return width, height, rows


def project_point(point, width, height):
    """Pixel of a scene-space point from the default scene camera: `[0,1,5]` at the origin, 50 degrees."""
    import math
    eye = (0.0, 1.0, 5.0)
    to = (-eye[0], -eye[1], -eye[2])
    norm = math.sqrt(sum(c * c for c in to))
    forward = tuple(c / norm for c in to)
    right = (-forward[2], 0.0, forward[0])
    norm = math.hypot(right[0], right[2])
    right = tuple(c / norm for c in right)
    up = (right[1] * forward[2] - right[2] * forward[1],
          right[2] * forward[0] - right[0] * forward[2],
          right[0] * forward[1] - right[1] * forward[0])
    offset = tuple(point[i] - eye[i] for i in range(3))
    dot = lambda a, b: sum(x * y for x, y in zip(a, b))  # noqa: E731
    depth = dot(offset, forward)
    focal = height / 2 / math.tan(math.radians(25))
    return (round(width / 2 + focal * dot(offset, right) / depth),
            round(height / 2 - focal * dot(offset, up) / depth))


def far_from_background(pixel, by):
    return max(abs(pixel[i] - BACKGROUND[i]) for i in range(3)) > by


def near_background(pixel):
    return max(abs(pixel[i] - BACKGROUND[i]) for i in range(3)) <= 12


def check_pictures(check, label, scene_png, splat_png):
    """The pixel oracle on the two PNG files of one project."""
    for name, png in (('scene', scene_png), ('splat', splat_png)):
        width, height, _ = read_png(png)
        check(f'{label}: the {name} thumbnail is {WIDTH}x{HEIGHT}', (width, height) == (WIDTH, HEIGHT), (width, height))
    width, height, scene = read_png(scene_png)
    _, _, splat = read_png(splat_png)
    box = project_point((-2.2, 0.0, 0.0), width, height)
    sphere = project_point((2.2, 0.0, 0.0), width, height)
    middle = project_point((0.0, 0.0, 0.0), width, height)
    at = lambda rows, p: rows[p[1]][p[0]]  # noqa: E731
    b, s = at(scene, box), at(scene, sphere)
    check(f'{label}: the scene thumbnail is orange where the box projects {box}', b[0] > b[2] + 60, b)
    check(f'{label}: the scene thumbnail is blue where the sphere projects {sphere}', s[2] > s[0] + 60, s)
    check(f'{label}: the scene thumbnail shows the cloud at the center {middle}',
          far_from_background(at(scene, middle), 40), at(scene, middle))
    check(f'{label}: the splat thumbnail shows the cloud at the center {middle}',
          far_from_background(at(splat, middle), 40), at(splat, middle))
    check(f'{label}: the splat thumbnail has no box and no sphere',
          near_background(at(splat, box)) and near_background(at(splat, sphere)), (at(splat, box), at(splat, sphere)))
    corners = [(2, 2), (width - 3, height - 3)]
    check(f'{label}: both corners are the background',
          all(near_background(at(rows, c)) for rows in (scene, splat) for c in corners))


def scenario(surface, check, scratch, fixtures, screenshots):
    def ok(request, what=None):
        good, payload = surface.op(request)
        if not good:
            raise ProofFailure(f'{request} -> {payload}')
        return payload

    def wait_for(predicate, what, seconds=180):
        deadline = time.time() + seconds
        last = None
        while time.time() < deadline:
            last = predicate()
            if last:
                return last
            time.sleep(0.4)
        raise ProofFailure(f'timed out waiting for {what}: {last}')

    def create(name, sequence, workspace):
        return ok({'op': 'project.create', 'name': name, 'sequence': sequence, 'rate': 24, 'width': FRAME[0],
                   'height': FRAME[1], 'length': 48, 'dir': str(workspace)})

    def import_media():
        ok({'op': 'media.import', 'paths': [str(fixtures / 'cloud.splat'), str(fixtures / 'scene.tessel-scene.json')]})
        if surface.name == 'native':
            wait_for(lambda: len(ok({'op': 'status'})['assets']) == 2, 'the import')

    def library():
        """`{id: thumbnail report}` of the splat and the scene, from `media.status` (the same op on both surfaces)."""
        assets = ok({'op': 'media.status'})['assets']
        return {a['id']: a for a in assets}

    def header_matches(project, sequence, step):
        label = f'{project} / {sequence}'
        h = ok({'op': 'project.header'})
        check(f'{step}: project.header says {label!r}',
              h['label'] == label and h['project'] == project and h['sequence'] == sequence, h)
        status = ok({'op': 'status'})
        check(f'{step}: status.header is the same answer', status['header'] == h, status['header'])
        if surface.name == 'native':
            good, node = surface.get('node:13')
            check(f'{step}: the semantic title node (node:13) is {label!r}', good and node['label'] == label, node)
        return h

    def pictures(workspace, want_paths=None):
        got = library()
        splat_id, scene_id = 'cloud', 'scene-tessel-scene'
        reports = {k: got[k]['thumbnail'] for k in (splat_id, scene_id)}
        check('both thumbnails are ready', all(r['state'] == 'ready' for r in reports.values()), reports)
        for id_, report in reports.items():
            check(f'{id_}: the picture is inside this project\'s workspace cache',
                  report['path'].startswith(str(workspace)) and '/.tessel/cache/' in report['path'], report['path'])
            check(f'{id_}: the picture file exists and is {report["width"]}x{report["height"]}',
                  Path(report['path']).is_file() and (report['width'], report['height']) == (WIDTH, HEIGHT), report)
        return reports[scene_id]['path'], reports[splat_id]['path'], reports

    ws_a, ws_b = scratch / 'ws-a', scratch / 'ws-b'
    # A document's workspace is its own folder: each project is saved inside its workspace.
    doc_a, doc_b = ws_a / 'a.tessel.json', ws_b / 'b.tessel.json'

    # ---- the header follows the project, and the first pictures ----
    created = create('Aurora', 'cut-a', ws_a)
    check('project A is created in its own workspace', created['workspace'] == str(ws_a), created)
    header_matches('Aurora', 'cut-a', 'after project.create')
    import_media()
    edits_before = ok({'op': 'status'})['persistence'], ok({'op': 'history.status'})
    before = library()
    check('the imported splat and scene have a pending picture before the job runs'
          if surface.name == 'headless' else 'the imported splat and scene are in the library',
          set(before) == {'cloud', 'scene-tessel-scene'}, set(before))
    if surface.name == 'headless':
        check('before the job both pictures are pending',
              all(a['thumbnail']['state'] == 'pending' for a in before.values()), before)
        report = ok({'op': 'media.thumbnails'})
        check('media.thumbnails draws both (splat and scene)',
              report['ready'] == 2 and report['applied']['failed'] == [], report)
    else:
        wait_for(lambda: all(a['thumbnail']['state'] == 'ready' for a in library().values()),
                 'the window\'s own thumbnail jobs')
    scene_a, splat_a, reports_a = pictures(ws_a)
    check_pictures(check, 'project A', scene_a, splat_a)
    edits_after = ok({'op': 'status'})['persistence'], ok({'op': 'history.status'})
    check('drawing the pictures is not an edit (revision, dirty flag and undo history unchanged)',
          edits_before == edits_after, (edits_before, edits_after))
    if surface.name == 'native':
        shot, reason = surface.capture(screenshots / 'bl144-header-thumbnails-window.png')
        check('the OS capture of the window was taken' if shot else f'capture skipped: {reason}', True)
    ok({'op': 'project.save', 'path': str(doc_a)})

    # ---- a second project with the same asset IDs and files (a trap on purpose) ----
    create('Basalt', 'cut-b', ws_b)
    header_matches('Basalt', 'cut-b', 'after a second project.create')
    import_media()
    b_before = library()
    check('B starts with no picture of A (a new project, an empty book)',
          all(a['thumbnail']['state'] in ('pending', 'ready') and
              a['thumbnail'].get('path', str(ws_b)).startswith(str(ws_b)) for a in b_before.values()), b_before)
    ok({'op': 'project.save', 'path': str(doc_b)})

    if surface.name == 'headless':
        queued = ok({'op': 'media.thumbnails', 'wait': False})
        job = queued['job']
        ok({'op': 'job.wait', 'job': job})
        opened = ok({'op': 'project.open', 'path': str(doc_a)})
        header_matches('Aurora', 'cut-a', 'after project.open of A')
        committed = ok({'op': 'job.commit', 'job': job})
        check('B\'s finished result is refused when A is open (stale, nothing applied)',
              committed['applied'].get('stale') is True, committed)
        a_now = library()
        check('A did not receive any picture of B',
              all(a['thumbnail']['state'] == 'pending' for a in a_now.values()), a_now)
        report = ok({'op': 'media.thumbnails'})
        check('A\'s pictures come back from its cache', report['ready'] == 2, report)
        scene_again, splat_again, _ = pictures(ws_a)
        check('the cache serves the very same files', (scene_again, splat_again) == (scene_a, splat_a),
              (scene_again, splat_again))
        ok({'op': 'project.open', 'path': str(doc_b)})
        header_matches('Basalt', 'cut-b', 'after project.open of B')
        ok({'op': 'media.thumbnails'})
        scene_b, splat_b, _ = pictures(ws_b)
        check('B\'s pictures are not A\'s files', scene_b != scene_a and splat_b != splat_a, (scene_b, scene_a))
        check_pictures(check, 'project B', scene_b, splat_b)
    else:
        # B's window job is drawing now (a cold renderer takes seconds): replace the project under it.
        jobs = surface.get('jobs')[1]['state']
        running = [j for j in jobs['all'] if j['kind'] == 'thumbnail' and j['state'] not in
                   ('succeeded', 'failed', 'cancelled')]
        ok({'op': 'project.open', 'path': str(doc_a)})
        header_matches('Aurora', 'cut-a', 'after project.open of A')
        samples = []

        def settled():
            now = library()
            samples.append({k: v['thumbnail'].get('path') for k, v in now.items()})
            jobs = surface.get('jobs')[1]['state']['all']
            return all(j['state'] in ('succeeded', 'failed', 'cancelled') for j in jobs if j['kind'] == 'thumbnail') \
                and all(a['thumbnail']['state'] == 'ready' for a in now.values())

        wait_for(settled, 'every thumbnail job to end with A\'s pictures ready')
        scene_again, splat_again, _ = pictures(ws_a)
        check('A shows its own pictures (the same files as before), never B\'s',
              (scene_again, splat_again) == (scene_a, splat_a), (scene_again, splat_again))
        check('no sample taken while the jobs ran held a path outside A\'s workspace',
              all(p is None or p.startswith(str(ws_a)) for s in samples for p in s.values()), samples[-3:])
        thumbs = [j for j in surface.get('jobs')[1]['state']['all'] if j['kind'] == 'thumbnail']
        states = sorted(j['state'] for j in thumbs)
        check(f'the thumbnail jobs ended: {states} (B\'s was running when A replaced it: {len(running)})',
              all(s in ('succeeded', 'cancelled') for s in states), states)
        ok({'op': 'project.open', 'path': str(doc_b)})
        header_matches('Basalt', 'cut-b', 'after project.open of B')
        wait_for(lambda: all(a['thumbnail']['state'] == 'ready' for a in library().values()),
                 'B\'s own thumbnail jobs')
        scene_b, splat_b, _ = pictures(ws_b)
        check('B\'s pictures are not A\'s files', scene_b != scene_a and splat_b != splat_a, (scene_b, scene_a))
        check_pictures(check, 'project B', scene_b, splat_b)
        shot, reason = surface.capture(screenshots / 'bl144-header-thumbnails-window-b.png')
        check('the OS capture of the second project was taken' if shot else f'capture skipped: {reason}', True)

    # ---- an error leaves the project intact ----
    before_status = ok({'op': 'status'})
    undo_before = ok({'op': 'history.status'})
    good, error = surface.op({'op': 'media.thumbnails', 'assets': ['ghost']})
    check('an unknown asset is refused with unknown_element', not good and error['code'] == 'unknown_element', error)
    after_status = ok({'op': 'status'})
    check('the project is untouched by the refusal (revision, undo depth, header)',
          after_status['persistence'] == before_status['persistence']
          and after_status['header'] == before_status['header'], (before_status['persistence'],
                                                                   after_status['persistence']))
    check('the undo history is untouched by the refusal', ok({'op': 'history.status'}) == undo_before, undo_before)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--editor', default=str(ROOT / 'target/debug/tessel-editor'))
    parser.add_argument('--render', default=str(ROOT / 'target/debug/tessel-render'))
    parser.add_argument('--out')
    parser.add_argument('--report')
    parser.add_argument('--only', choices=['headless', 'native'])
    args = parser.parse_args()
    free = shutil.disk_usage('/').free / 2 ** 30
    if free < MIN_FREE_GIB:
        sys.exit(f'only {free:.1f} GiB free; the floor is {MIN_FREE_GIB} GiB')
    out = Path(args.out or tempfile.mkdtemp(prefix='bl144-proof-'))
    out.mkdir(parents=True, exist_ok=True)
    screenshots = ROOT / 'docs/features/assets'
    screenshots.mkdir(parents=True, exist_ok=True)
    fixtures = out / 'fixtures'
    subprocess.run([str(ROOT / 'scripts/fixtures'), str(fixtures)], check=True, capture_output=True)
    report, failed = {}, False
    for name in ('headless', 'native'):
        if args.only and args.only != name:
            continue
        scratch = out / name
        scratch.mkdir(exist_ok=True)
        check = Recorder(name)
        # A short HOME: the socket path of a long one does not fit a Unix socket.
        home = tempfile.mkdtemp(prefix='b4-', dir='/tmp')
        surface = (Headless if name == 'headless' else Native)(args.render if name == 'headless' else args.editor, home)
        try:
            surface.prepare()
            scenario(surface, check, scratch, fixtures, screenshots)
        except ProofFailure as failure:
            failed = True
            print(f'FAILED: {failure}', flush=True)
        finally:
            surface.stop()
            shutil.rmtree(home, ignore_errors=True)
        report[name] = check.checks
        print(f'{name}: {sum(c["ok"] for c in check.checks)}/{len(check.checks)} checks', flush=True)
    if args.report:
        Path(args.report).write_text(json.dumps(report, indent=2))
    sys.exit(1 if failed else 0)


if __name__ == '__main__':
    main()
