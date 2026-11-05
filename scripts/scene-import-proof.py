#!/usr/bin/env python3
"""F043/BL086/BL100 — attach a directed scene through the *dialog* of a private native window.

    python3 scripts/directed-scene-proof/with-gpu-lock.py \
        python3 scripts/scene-import-proof.py [--binary target/debug/tessel-editor] [--out report.json]

Starts the DIRECT binary with its own short HOME (own socket, own settings: the user's window and
project are never touched) and drives only semantic requests over its socket, no pointer, no
coordinates: `invoke flow.attach_scene`, `set dialog.field.*`, `invoke dialog.submit`, `get`. Files
are the synthetic corpus of `scripts/directed-scene-proof/corpus` (drawn from scratch, no private
media), copied into a throwaway folder so the proof can break them.

Proves, in the live window: the dialog opens and hides the WebView (`Workflow::has_dialog`); the
bundle is read and validated by a background job (BL100), so a bundle that is not valid (or not
there) closes the dialog and fails the `scene_import` node on the `attach` step with its typed code,
with no scene, history entry or folder created; a request the host refuses before any job (a
malformed ID) keeps the dialog open; a valid bundle + folder attaches (one undo entry, when the job
is applied), adopts and loads through the background jobs and the window applies them; a missing and an altered resource fail with the host's own code
and leave the workspace clean; the scene is selected in the inspector and the monitor draws it.

It does NOT prove the native file picker (a macOS panel needs a human) nor aesthetics.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
CORPUS = ROOT / 'scripts/directed-scene-proof/corpus'
spec = importlib.util.spec_from_file_location('video_proof', ROOT / 'scripts/video-live-proof.py')
video = importlib.util.module_from_spec(spec)
spec.loader.exec_module(video)
MIN_FREE_GIB = 10


def authored(base, name):
    folder = Path(base) / name
    (folder / 'assets').mkdir(parents=True)
    shutil.copy(CORPUS / 'bundle.json', folder / 'bundle.json')
    for asset in ('sky.png', 'mid.png', 'hero.png'):
        shutil.copy(CORPUS / 'assets' / asset, folder / 'assets' / asset)
    return folder


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', default=str(ROOT / 'target/debug/tessel-editor'))
    parser.add_argument('--out')
    args = parser.parse_args()
    free_gib = shutil.disk_usage('/tmp').free / 1024 ** 3
    if free_gib < MIN_FREE_GIB:
        print(json.dumps({'ok': False, 'error': f'only {free_gib:.1f} GiB free, need {MIN_FREE_GIB}'}))
        return 1
    report = {'checks': [], 'input': 'semantic IPC (socket), no pointer, no coordinates',
              'not_proven': ['native file picker panel', 'visual appearance']}
    home = tempfile.mkdtemp(prefix='te-r2-', dir='/tmp')
    # Without this the monitor does not read its picture back, so `last_frame.hash` stays empty and
    # nothing here could compare what it drew with an explicit render.
    os.environ['TESSEL_VIEWPORT_INSPECT'] = '1'
    lab = video.Editor(str(Path(args.binary).resolve()), home)

    def send(request):
        return lab.send({'version': 1, **request})

    def ok(request):
        response = send(request)
        if not response.get('ok'):
            raise AssertionError(f'{request} -> {response}')
        return response['data']

    def op(request):
        return ok({'command': 'op', 'request': request})

    def get(target):
        return send({'command': 'get', 'target': target})

    def state(target):
        return ok({'command': 'get', 'target': target})['state']

    def check(name, condition, detail=''):
        report['checks'].append({'name': name, 'ok': bool(condition), 'detail': '' if condition else str(detail)[:400]})
        print(('PASS ' if condition else 'FAIL ') + name + ('' if condition else f'  [{str(detail)[:300]}]'), flush=True)
        if not condition:
            raise AssertionError(name)

    def fill(**fields):
        for name, value in fields.items():
            ok({'command': 'set', 'target': f'dialog.field.{name}', 'field': 'value', 'value': value})

    def wait_chain(wanted, seconds=30):
        deadline = time.time() + seconds
        while time.time() < deadline:
            current = state('scene_import')
            if current['state'] in wanted:
                return current
            time.sleep(0.1)
        raise AssertionError(f'the chain never reached {wanted}: {state("scene_import")}')

    def undo_depth():
        return op({'op': 'history.status'})['history']['undo_depth']

    try:
        lab.wait_ready(90)
        work = Path(home) / 'w'
        work.mkdir()
        good = authored(work, 'good')
        project_dir = work / 'ws'
        op({'op': 'project.create', 'name': 'Scenes', 'dir': str(project_dir)})
        check('the scene flows are nodes of the live tree',
              all(get(node).get('ok') for node in ('flow.attach_scene', 'flow.scene_resources', 'scene_import')))
        check('before any chain the node is idle', state('scene_import') == {'state': 'idle'})

        # --- dialog opens and hides the WebView -------------------------------------------------
        visible_before = get('monitor.viewport')['data']['visible']
        opened = ok({'command': 'invoke', 'target': 'flow.attach_scene', 'action': 'open'})
        check('invoking flow.attach_scene opens the dialog', opened.get('flow') == 'attach_scene', opened)
        dialog = state('dialog')
        check('the dialog reports the scene flow', dialog['flow'] == 'attach_scene', dialog)
        fields = {name: get(f'dialog.field.{name}') for name in ('bundle', 'id', 'name', 'source', 'show')}
        check('every field is a semantic node', all(entry.get('ok') for entry in fields.values()), fields)
        check('"show" offers yes/no and defaults to yes',
              fields['show']['data']['fields'][0]['options'] == ['yes', 'no'] and fields['show']['data']['value'] == 'yes',
              fields['show'])
        # The WebView paints above GPUI, so `has_dialog` must take it off screen. The tree's
        # `visible` follows the last paint, so give the window a frame.
        deadline = time.time() + 5
        while time.time() < deadline and get('monitor.viewport')['data']['visible']:
            time.sleep(0.1)
        check('an open dialog takes the monitor WebView off screen',
              visible_before is not False and get('monitor.viewport')['data']['visible'] is False,
              (visible_before, get('monitor.viewport')))

        # --- invalid bundle: the attach job fails, nothing created ------------------------------
        broken = work / 'broken.json'
        broken.write_text('{"bundleVersion": 99}')
        undo_before = undo_depth()
        fill(bundle=str(broken), id='street', source=str(good))
        malformed = send({'command': 'set', 'target': 'dialog.field.id', 'field': 'value', 'value': 'bad id!'})
        check('a field accepts the typed ID', malformed['ok'], malformed)
        refused = send({'command': 'invoke', 'target': 'dialog.submit'})
        check('an ID the host refuses before any job stays in the dialog with its code',
              not refused['ok'] and refused['error']['code'] == 'invalid_id' and state('dialog')['flow'] == 'attach_scene', refused)
        fill(id='street')
        ok({'command': 'invoke', 'target': 'dialog.submit'})
        check('the dialog closes: the bundle is read by a job', not get('dialog').get('ok'), get('dialog'))
        failed_read = wait_chain({'failed'})
        check('an invalid bundle fails the attach step with its typed code',
              failed_read['error']['step'] == 'attach' and failed_read['error']['code'] == 'invalid_bundle'
              and failed_read['scene'] is None, failed_read)
        check('no scene, no history entry, no resource folder',
              op({'op': 'scene.list'})['scenes'] == [] and undo_depth() == undo_before
              and not (project_dir / 'scenes').exists(), op({'op': 'scene.list'}))
        ok({'command': 'invoke', 'target': 'flow.attach_scene', 'action': 'open'})
        fill(bundle=str(work / 'nowhere.json'))
        ok({'command': 'invoke', 'target': 'dialog.submit'})
        missing = wait_chain({'failed'})
        check('a path that is not there is bundle_unreadable',
              missing['error']['step'] == 'attach' and missing['error']['code'] == 'bundle_unreadable', missing)

        # --- valid file: attach, adopt, load, select, show --------------------------------------
        ok({'command': 'invoke', 'target': 'flow.attach_scene', 'action': 'open'})
        fill(bundle=str(good / 'bundle.json'), id='street', source=str(good), show='yes')
        submitted = ok({'command': 'invoke', 'target': 'dialog.submit'})
        check('a valid bundle starts the chain at the attach job',
              submitted['action'] == 'scene.import' and submitted['import']['step'] in ('attach', 'adopt', 'resolve'), submitted)
        check('the dialog closed', not get('dialog').get('ok'), get('dialog'))
        done = wait_chain({'loaded', 'failed'})
        check('attach is one undo entry', undo_depth() == undo_before + 1, undo_depth())
        check('the background jobs adopted and loaded the scene', done['state'] == 'loaded' and done['error'] is None, done)
        check('steps ran in order', done['done'][:4] == ['attach', 'adopt', 'resolve', 'select'], done['done'])
        for asset in ('sky.png', 'mid.png', 'hero.png'):
            check(f'{asset} is a verified copy inside the project', (project_dir / 'scenes/street/assets' / asset).is_file())
        status = op({'op': 'scene.resources.status'})
        street = next(entry for entry in status['attached'] if entry['scene'] == 'street')
        check('the host says the scene is loaded and current', street['loaded'] and street['current'], street)
        jobs = state('jobs')['recent']
        actions = [event['result']['action'] for event in reversed(jobs) if event.get('result')]
        check('attach, adopt then resolve are in the shared job history',
              actions[:3] == ['scene.attach.start', 'scene.resources.adopt', 'scene.resources.resolve'], actions)
        check('the inspector is in scene mode on the new scene',
              get('inspector.mode')['data']['value'] == 'scene' and state('scene.panel')['scene'] == 'street',
              (get('inspector.mode'), get('scene.panel')))
        # The monitor draws on its own schedule: wait for it to settle on the new scene, then
        # compare what it drew with an explicit render of the same frame (an independent path).
        deadline = time.time() + 30
        view = {}
        while time.time() < deadline:
            view = state('monitor.viewport')
            directed = view.get('directed') or {}
            if (view.get('phase') == 'ready' and directed.get('scene') == 'street'
                    and directed.get('active') is None and directed.get('queued') is None
                    and (view.get('last_frame') or {}).get('hash')):
                break
            time.sleep(0.1)
        directed = view.get('directed') or {}
        screen = view.get('last_frame') or {}
        check('the monitor shows the attached scene and has drawn a frame',
              directed.get('scene') == 'street' and bool(screen.get('hash')), view)
        shown = (directed.get('shown') or {}).get('edit_frame', 0)
        reference = op({'op': 'scene.render', 'scene': 'street', 'frame': shown,
                        'width': screen['width'], 'height': screen['height']})['frame']
        check('what the monitor drew is the explicit render of the same frame (same hash)',
              screen['hash'] == reference['hash'], (screen.get('hash'), reference.get('hash')))
        frame = {'request': screen.get('request'), 'hash': screen.get('hash')}
        check('the WebView is back on screen after the dialog closed', get('monitor.viewport')['data']['visible'] is True, get('monitor.viewport'))
        report['monitor_frame'] = {'request': frame.get('request'), 'hash': frame.get('hash')}

        # --- missing resource: attached, not loaded, workspace clean ----------------------------
        lost = authored(work, 'lost')
        (lost / 'assets/hero.png').unlink()
        ok({'command': 'invoke', 'target': 'flow.attach_scene', 'action': 'open'})
        fill(bundle=str(lost / 'bundle.json'), id='alley', source=str(lost), show='no')
        ok({'command': 'invoke', 'target': 'dialog.submit'})
        failed = wait_chain({'loaded', 'failed'})
        check('a missing resource fails the adopt with the host code',
              failed['state'] == 'failed' and failed['error']['step'] == 'adopt' and failed['error']['code'] == 'resources_invalid', failed)
        check('the host names the missing file', 'hero' in failed['error']['message'], failed)
        listed = [scene['id'] for scene in op({'op': 'scene.list'})['scenes']]
        check('the valid bundle stays attached', 'alley' in listed, listed)
        check('nothing was copied and nothing is loaded for it',
              not (project_dir / 'scenes/alley').exists()
              and not next(e for e in op({'op': 'scene.resources.status'})['attached'] if e['scene'] == 'alley')['loaded'])
        check('the scene_import node keeps the failure for a client that missed the dialog',
              state('scene_import')['scene'] == 'alley' and state('scene_import')['error']['code'] == 'resources_invalid',
              state('scene_import'))

        # --- altered source: refused by hash ----------------------------------------------------
        warped = authored(work, 'warped')
        sky = warped / 'assets/sky.png'
        data = bytearray(sky.read_bytes())
        data[-1] ^= 0xFF
        sky.write_bytes(bytes(data))
        ok({'command': 'invoke', 'target': 'flow.attach_scene', 'action': 'open'})
        fill(bundle=str(warped / 'bundle.json'), id='lane', source=str(warped), show='no')
        ok({'command': 'invoke', 'target': 'dialog.submit'})
        altered = wait_chain({'loaded', 'failed'})
        check('an altered file is refused by hash and never copied',
              altered['state'] == 'failed' and altered['error']['code'] == 'resources_invalid'
              and not (project_dir / 'scenes/lane').exists(), altered)

        # --- repair through the second dialog ---------------------------------------------------
        shutil.copy(CORPUS / 'assets/hero.png', lost / 'assets/hero.png')
        opened = ok({'command': 'invoke', 'target': 'flow.scene_resources', 'action': 'open'})
        check('the resources dialog opens', opened['flow'] == 'scene_resources', opened)
        fill(scene='alley', source=str(lost), show='no')
        ok({'command': 'invoke', 'target': 'dialog.submit'})
        repaired = wait_chain({'loaded', 'failed'})
        check('the same scene loads once its folder is repaired, with no second attach',
              repaired['state'] == 'loaded' and [scene['id'] for scene in op({'op': 'scene.list'})['scenes']].count('alley') == 1, repaired)
        report['ok'] = True
    except Exception as error:
        report['ok'] = False
        report['error'] = repr(error)
    finally:
        try:
            lab.stop()
        finally:
            lab.log.close()
            shutil.rmtree(home, ignore_errors=True)
            if args.out:
                Path(args.out).write_text(json.dumps(report, indent=2) + '\n')
            print(json.dumps({'ok': report.get('ok'), 'checks': len(report['checks']), 'error': report.get('error')}))
    return 0 if report.get('ok') else 1


if __name__ == '__main__':
    raise SystemExit(main())
