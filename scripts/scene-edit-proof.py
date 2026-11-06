#!/usr/bin/env python3
"""F034/BL004/BL005 (S6) — typed component properties and the origin-equivalent camera start, proved on
two separate surfaces with the same scenario:

    python3 scripts/directed-scene-proof/with-gpu-lock.py \
        python3 scripts/scene-edit-proof.py [--editor target/debug/tessel-editor] \
        [--headless target/debug/tessel-headless] [--out report.json] [--allow-dirty]

* **native window**: the DIRECT editor binary started with its own short `HOME` (own socket and settings:
  the user's window and project are never touched), driven only by the `ui` CLI of that binary
  (`ui op`, `ui get`, `ui set`), the same requests as `./scripts/ui`. No pointer, no coordinates.
* **headless (no window)**: `tessel-headless`, JSON Lines over stdio. A window's socket is NOT headless
  evidence; the two runs are reported separately and the library itself is covered by `cargo test`.

The scenario loads the corpus bundle (camera `cam-main`: dolly-in, hold, truck-right; moving camera), then:
`start/position/x` converts the first movement into keys (and says so in `conversions`), keeps the rest of
the trajectory, and ONE undo restores the document; `start/target` and `renderOrder` write in place with no
conversion; invalid and refused edits change nothing (document, revision, undo depth); a no-op leaves no
history entry. The document is read back through `project.save` (the real `.tessel.json`), not through the
same answer that reported the edit. The native run also drives the inspector's semantic `set` of an axis.

Not proven here: pixels, a physical pointer or slider drag (covered by GPUI-headless unit tests, which are
not a native pointer), a native file picker, and the equivalence with the origin (the fixed oracle does that).
"""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
BUNDLE = ROOT / 'scripts/directed-scene-proof/corpus/bundle.json'
MIN_FREE_GIB = 12
TOLERANCE = 1e-9
CLIP = 'camera/cam-main'
START = 'camera/cam-main/start'
HERO = 'scene/scene-a/node/hero'
SAMPLE_FRAMES = [0, 16, 31, 32, 40, 47, 60, 90]
# Frames after the first movement ends (32): the conversion must not move them.
UNMOVED_FRAMES = [32, 40, 47, 60, 90]


class ProofFailure(AssertionError):
    pass


class Recorder:
    """Collects named checks; a failed one stops the run with its detail."""

    def __init__(self, surface):
        self.surface = surface
        self.checks = []

    def __call__(self, name, condition, detail=''):
        self.checks.append({'name': name, 'ok': bool(condition), 'detail': '' if condition else str(detail)[:500]})
        print(('PASS ' if condition else 'FAIL ') + f'[{self.surface}] ' + name
              + ('' if condition else f'  [{str(detail)[:300]}]'), flush=True)
        if not condition:
            raise ProofFailure(f'[{self.surface}] {name}')


def pose_close(left, right, tolerance=TOLERANCE):
    """Two camera answers (`position`, `target`, `fovDeg`) agree component by component."""
    def flat(camera):
        return [camera['position'][axis] for axis in 'xyz'] + [camera['target'][axis] for axis in 'xyz'] + [camera['fovDeg']]
    return all(abs(a - b) <= tolerance + 1e-12 * max(abs(a), abs(b)) for a, b in zip(flat(left), flat(right)))


def normalize(body):
    """`(ok, payload)` from either surface's envelope (`data`/`result`/`error`)."""
    if body.get('ok'):
        return True, body.get('data', body.get('result'))
    return False, body.get('error')


def bundle_of(project):
    return project['scenes']['items'][0]['bundle']


def first_movement(bundle):
    return bundle['cameraClips'][0]['segments'][0]


def hero_plane(bundle):
    definition = bundle['sceneDefinitions'][0]
    return next(node for node in definition['nodes'] if node['id'] == 'hero')


def clean_tracked_tree(root=ROOT):
    """True when no tracked file differs from HEAD (untracked build output is irrelevant)."""
    status = subprocess.run(['git', 'status', '--porcelain', '--untracked-files=no'], cwd=root,
                            capture_output=True, text=True, check=True).stdout
    return status.strip() == '', status


def sha256(path):
    digest = hashlib.sha256()
    with open(path, 'rb') as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b''):
            digest.update(chunk)
    return digest.hexdigest()


class Headless:
    """`tessel-headless --empty`: one JSON request per line, no window, no socket."""
    name = 'headless'

    def __init__(self, binary, _home):
        self.process = subprocess.Popen([binary, '--empty'], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=subprocess.DEVNULL, text=True, bufsize=1)

    def raw(self, request):
        self.process.stdin.write(json.dumps(request) + '\n')
        self.process.stdin.flush()
        return json.loads(self.process.stdout.readline())

    def op(self, request):
        return normalize(self.raw(request))

    def prepare(self):
        pass

    def stop(self):
        self.process.stdin.close()
        self.process.wait(30)


class Native:
    """The DIRECT editor binary with its own short HOME, driven by that binary's `ui` CLI."""
    name = 'native'

    def __init__(self, binary, home):
        self.binary = binary
        self.env = {**os.environ, 'HOME': home}
        self.socket = os.path.join(home, '.cache/tessel-editor/ui.sock')
        self.home = home
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

    def set(self, target, field, value):
        return normalize(self.ui('set', target, field, str(value)))

    def prepare(self, seconds=90):
        deadline = time.time() + seconds
        while time.time() < deadline:
            if self.process.poll() is not None:
                with open(self.log_path) as handle:
                    raise RuntimeError(f'the editor exited ({self.process.returncode}): {handle.read()[-500:]}')
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


def scenario(surface, check, scratch):
    """The S6 paths, identical on both surfaces. Reads the document back through `project.save`."""

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

    def edit(*ops, dry_run=False):
        request = {'op': 'scene.edit', 'scene': 'street', 'ops': list(ops)}
        if dry_run:
            request['dry_run'] = True
        return ok(request)

    saves = [0]

    def project():
        saves[0] += 1
        path = scratch / f'doc-{saves[0]}.tessel.json'
        ok({'op': 'project.save', 'path': str(path)})
        return json.loads(path.read_text())

    def history():
        data = ok({'op': 'history.status'})
        return data['history']['undo_depth'], data['project']['revision']

    def poses(frames=SAMPLE_FRAMES):
        return {frame: ok({'op': 'scene.camera', 'scene': 'street', 'frame': frame})['camera'] for frame in frames}

    def same_poses(before, after, frames):
        return all(pose_close(before[frame], after[frame]) for frame in frames)

    def set_value(path, value):
        return {'set': path, 'value': value}

    if surface.name == 'native':
        ok({'op': 'project.create', 'name': 'S6 proof', 'dir': str(scratch / 'workspace')})
    attached = ok({'op': 'scene.attach', 'id': 'street', 'path': str(BUNDLE)})
    check('the corpus bundle loads', attached.get('scene') == 'street', attached)

    # --- a camera with movement ------------------------------------------------------------------
    base = project()
    base_bundle = copy.deepcopy(bundle_of(base))
    base_poses = poses()
    depth0, revision0 = history()
    z = [base_poses[frame]['position']['z'] for frame in (0, 16, 31)]
    check('the camera moves (dolly-in: z decreases over frames 0, 16, 31)', z[0] > z[1] > z[2], z)
    check('the saved document holds the first movement as a recipe',
          first_movement(base_bundle)['recipe'] == 'dollyIn' and 'keys' not in first_movement(base_bundle),
          first_movement(base_bundle))

    # --- start/position/x converts the first movement, keeps the rest, one undo restores -----------
    dry = edit(set_value(f'{START}/position/x', 1.5), dry_run=True)
    check('dry-run reports the conversion and changes nothing',
          dry.get('conversions') and dry['dry_run'] is True and bundle_of(project()) == base_bundle
          and history() == (depth0, revision0), dry)
    result = edit(set_value(f'{START}/position/x', 1.5))
    expected = [{'clip': 'cam-main', 'index': 0, 'keys': 1, 'recipe': 'dollyIn', 'segment': 0}]
    check('the axis write reports the conversion (clip, op index, segment, recipe, keys)',
          result['changed'] is True and result.get('conversions') == expected, result)
    converted = bundle_of(project())
    movement = first_movement(converted)
    check('the first movement became one key and lost the dolly recipe fields',
          movement['recipe'] == 'keyframes' and len(movement['keys']) == 1
          and 'distanceSceneUnits' not in movement, movement)
    check('the start moved on the written axis only',
          converted['cameraClips'][0]['initialPosition'] == {'x': 1.5, 'y': 1, 'z': 8}
          and converted['cameraClips'][0]['initialTarget'] == base_bundle['cameraClips'][0]['initialTarget'],
          converted['cameraClips'][0])
    after = poses()
    check('frame 0 shows the new start', abs(after[0]['position']['x'] - 1.5) <= TOLERANCE, after[0])
    check('control: the same comparator sees the moved start and the converted movement (frames 0 and 16 differ)',
          not same_poses(base_poses, after, [0]) and not same_poses(base_poses, after, [16]), after[16])
    check('the trajectory after the first movement did not move (frames 32, 40, 47, 60, 90)',
          same_poses(base_poses, after, UNMOVED_FRAMES), {f: after[f] for f in UNMOVED_FRAMES})
    check('the batch is one undo entry', history()[0] == depth0 + 1, history())
    undone = ok({'op': 'history.undo'})
    check('one undo restores the whole document, recipe included',
          undone['changed'] is True and bundle_of(project()) == base_bundle and history()[0] == depth0, undone)
    check('and the poses at every sampled frame', same_poses(base_poses, poses(), SAMPLE_FRAMES))

    # --- start/target and renderOrder write in place: no conversion --------------------------------
    result = edit(set_value(f'{START}/target', {'x': 0, 'y': 0.6, 'z': 0}), set_value(f'{HERO}/renderOrder', 9))
    check('start/target and renderOrder: no conversion in the answer',
          result['changed'] is True and 'conversions' not in result, result)
    written = bundle_of(project())
    check('the first movement is untouched, the target and the renderOrder were written',
          first_movement(written) == first_movement(base_bundle)
          and written['cameraClips'][0]['initialTarget'] == {'x': 0, 'y': 0.6, 'z': 0}
          and hero_plane(written)['renderOrder'] == 9, written['cameraClips'][0])
    check('one entry again', history()[0] == depth0 + 1, history())
    ok({'op': 'history.undo'})
    check('one undo restores the document', bundle_of(project()) == base_bundle and history()[0] == depth0)
    # An undo is itself a revision of the project; from here on the baseline is the settled one.
    depth0, revision0 = history()

    # --- no-op: nothing recorded -------------------------------------------------------------------
    result = edit(set_value(f'{START}/position/x', 0))
    check('writing the current value is a no-op: no conversion, no entry, document identical',
          result['noop'] is True and result['changed'] is False and 'conversions' not in result
          and history() == (depth0, revision0) and bundle_of(project()) == base_bundle, result)

    # --- invalid edits are refused with a typed code and mutate nothing ----------------------------
    refusals = [
        ('start/fov out of range', [set_value(f'{START}/fov', 200)], 'invalid_value', 0),
        ('renderOrder is a whole number', [set_value(f'{HERO}/renderOrder', 1.5)], 'invalid_type', 0),
        ('renderOrder beyond 2^53-1', [set_value(f'{HERO}/renderOrder', 1e16)], 'invalid_value', 0),
        ('an axis takes a number, not a vector',
         [set_value(f'{START}/position/x', {'x': 1, 'y': 1, 'z': 1})], 'invalid_type', 0),
        ('a size has no default to reset', [{'reset': f'{HERO}/width'}], 'no_default', 0),
        ('an unknown clip', [set_value('camera/missing/start/fov', 40)], 'unknown_clip', 0),
        ('a valid first op does not survive a refused second one',
         [set_value(f'{START}/position/x', 3), set_value(f'{START}/fov', 0)], 'invalid_value', 1),
    ]
    for name, ops, code, index in refusals:
        error = refused({'op': 'scene.edit', 'scene': 'street', 'ops': ops})
        check(f'refused: {name} ({code})', error['code'] == code and error['details'].get('index') == index, error)
        check(f'  and nothing changed after: {name}',
              bundle_of(project()) == base_bundle and history() == (depth0, revision0)
              and same_poses(base_poses, poses(), SAMPLE_FRAMES))

    return {'depth': depth0, 'revision': revision0, 'bundle': base_bundle, 'poses': base_poses,
            'project': project, 'history': history, 'ok': ok, 'refused': refused, 'poses_of': poses,
            'same_poses': same_poses}


def native_inspector(surface, check, scratch, context):
    """The inspector's semantic `set` (the slider's begin/update/commit) of an axis, on the live window."""
    ok, refused, project, history = context['ok'], context['refused'], context['project'], context['history']
    base_bundle, depth0, revision0 = context['bundle'], context['depth'], context['revision']
    good, _ = surface.set('inspector.mode', 'value', 'scene')
    check('the scene inspector opens by a semantic set', good)
    selected = ok({'op': 'scene.select', 'scene': 'street', 'entities': [CLIP]})
    check('the camera clip is selected', selected['changed'] is True, selected)
    good, panel = surface.get('scene.panel')
    check('the panel offers the clip start properties and no open gesture',
          f'scene.property:{START}/position' in panel['children'] and not panel['state'].get('gesture'), panel)

    good, result = surface.set(f'scene.property:{START}/position', 'x', 2)
    check('set start/position x converts like the batch and says so',
          good and result['action'] == 'scene.gesture.commit' and result['changed'] is True
          and result['conversions'] == [{'clip': 'cam-main', 'index': 0, 'keys': 1, 'recipe': 'dollyIn',
                                         'segment': 0}], result)
    movement = first_movement(bundle_of(project()))
    check('the saved document holds the keys', movement['recipe'] == 'keyframes' and len(movement['keys']) == 1, movement)
    check('one entry; one undo restores the document',
          history()[0] == depth0 + 1 and ok({'op': 'history.undo'})['changed'] is True
          and bundle_of(project()) == base_bundle and history()[0] == depth0)
    depth0, revision0 = history()

    good, error = surface.set(f'scene.property:{START}/fov', 'value', 200)
    check('set start/fov 200 is refused with the typed code',
          (not good) and error['code'] == 'invalid_value', error)
    good, panel = surface.get('scene.panel')
    check('the refusal left no gesture open and no document or history change',
          not panel['state'].get('gesture') and bundle_of(project()) == base_bundle
          and history() == (depth0, revision0), panel['state'])
    check('the panel shows the same typed error as its notice', panel['state']['notice']['code'] == 'invalid_value',
          panel['state'].get('notice'))

    ok({'op': 'scene.select', 'scene': 'street', 'entities': [HERO]})
    good, result = surface.set(f'scene.property:{HERO}/renderOrder', 'value', 4.4)
    check('the inspector rounds a typed renderOrder to a whole number (4.4 -> 4) in one entry',
          good and result['changes'][0]['after'] == 4 and 'conversions' not in result
          and history()[0] == depth0 + 1, result)
    check('the saved document holds 4', hero_plane(bundle_of(project()))['renderOrder'] == 4)
    ok({'op': 'history.undo'})
    check('one undo restores the document', bundle_of(project()) == base_bundle and history()[0] == depth0)


def run_surface(factory, binary, report, free_gib):
    home = tempfile.mkdtemp(prefix='te-s6-', dir='/tmp')
    scratch = Path(home) / 'scratch'
    scratch.mkdir()
    surface = factory(binary, home)
    check = Recorder(surface.name)
    report[surface.name] = {'checks': check.checks, 'home': home}
    try:
        surface.prepare()
        context = scenario(surface, check, scratch)
        if surface.name == 'native':
            if free_gib() < MIN_FREE_GIB:
                raise ProofFailure('free space fell below the floor')
            native_inspector(surface, check, scratch, context)
    except ProofFailure as failure:
        report[surface.name]['failure'] = str(failure)
        raise
    finally:
        surface.stop()
        shutil.rmtree(home, ignore_errors=True)
    report[surface.name]['passed'] = len(check.checks)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--editor', default=str(ROOT / 'target/debug/tessel-editor'))
    parser.add_argument('--headless', default=str(ROOT / 'target/debug/tessel-headless'))
    parser.add_argument('--out')
    parser.add_argument('--allow-dirty', action='store_true',
                        help='skip the clean-tracked-tree requirement (development only; never final evidence)')
    args = parser.parse_args()

    def free_gib():
        return shutil.disk_usage('/tmp').free / 1024 ** 3

    clean, status = clean_tracked_tree()
    head = subprocess.run(['git', 'rev-parse', 'HEAD'], cwd=ROOT, capture_output=True, text=True, check=True).stdout.strip()
    report = {
        'head': head, 'tracked_tree_clean': clean,
        'binaries': {name: {'path': str(Path(path).resolve()), 'sha256': sha256(path)}
                     for name, path in (('tessel-editor', args.editor), ('tessel-headless', args.headless))},
        'input': 'semantic requests only (ui op/get/set of the direct binary; JSONL for headless), no pointer, no coordinates',
        'not_proven': ['a physical pointer or slider drag (GPUI-headless unit tests only)', 'pixels',
                       'native file picker', 'equivalence with the origin (fixed oracle: oracle.mjs verify)'],
    }
    code = 0
    try:
        if free_gib() < MIN_FREE_GIB:
            raise RuntimeError(f'only {free_gib():.1f} GiB free, need {MIN_FREE_GIB}')
        if not clean and not args.allow_dirty:
            raise RuntimeError(f'tracked files differ from HEAD; commit first:\n{status}')
        report['min_free_gib_seen'] = round(free_gib(), 1)
        run_surface(Headless, args.headless, report, free_gib)
        run_surface(Native, args.editor, report, free_gib)
        report['min_free_gib_seen'] = round(min(report['min_free_gib_seen'], free_gib()), 1)
        report['ok'] = True
    except (ProofFailure, RuntimeError) as failure:
        report['ok'] = False
        report['error'] = str(failure)
        code = 1
        print(f'FAILED: {failure}', flush=True)
    text = json.dumps(report, indent=2)
    if args.out:
        Path(args.out).write_text(text + '\n')
    summary = {key: value for key, value in report.items() if key not in ('headless', 'native')}
    for surface in ('headless', 'native'):
        if surface in report:
            summary[surface] = {'checks': len(report[surface]['checks']), 'failure': report[surface].get('failure')}
    print(json.dumps(summary, indent=2))
    return code


if __name__ == '__main__':
    raise SystemExit(main())
