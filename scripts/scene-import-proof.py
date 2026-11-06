#!/usr/bin/env python3
"""F043/BL086/BL100/BL101 — attach a directed scene through the *dialog* of a private native window.

    python3 scripts/directed-scene-proof/with-gpu-lock.py \
        python3 scripts/scene-import-proof.py [--binary target/debug/tessel-editor] [--out report.json] \
        [--scenario all|dialog|empty-manifest] [--shots DIR]

Two scenarios, each in its OWN private instance (own HOME, own socket): `dialog` (the R2/S5 flow
below) and `empty-manifest` (BL101 on the real monitor, described at the end).

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

`empty-manifest`: in a project that was never saved (no workspace, no folder anywhere) a bundle
whose nodes name no file attaches through the dialog with no folder, loads (attach, resolve, select;
never adopt), creates nothing on disk, and the monitor reaches `ready` on it and draws a frame whose
hash equals an explicit `scene.render` of the same frame. Expectations are counted from the raw bundle
JSON, not from the core's manifest. Negative controls with the SAME guards: a bundle that names files
stays `attached`, resolving it is `no_workspace` (op and repair dialog), and inside a saved project
without `scenes/<id>` it is `root_unavailable` with nothing created. It also times the window while a
24 MiB bundle is read by the job. `--shots` adds an OS capture of that private window (observation).

It does NOT prove the native file picker (a macOS panel needs a human) nor aesthetics. The
data-only scene has only groups, so its frame is the empty background: the proof is that the
monitor reached ready on it and drew what the explicit render draws, not a picture.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
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



class Live:
    """One private native instance: its own short HOME (own socket and settings, never the user's
    window or project) driven only through semantic requests."""

    def __init__(self, binary, report, tag):
        self.report = report
        self.home = tempfile.mkdtemp(prefix=f'te-{tag}-', dir='/tmp')
        # Without this the monitor does not read its picture back, so `last_frame.hash` stays empty and
        # nothing here could compare what it drew with an explicit render.
        os.environ['TESSEL_VIEWPORT_INSPECT'] = '1'
        self.lab = video.Editor(str(Path(binary).resolve()), self.home)

    def send(self, request):
        return self.lab.send({'version': 1, **request})

    def ok(self, request):
        response = self.send(request)
        if not response.get('ok'):
            raise AssertionError(f'{request} -> {response}')
        return response['data']

    def op(self, request):
        return self.ok({'command': 'op', 'request': request})

    def get(self, target):
        return self.send({'command': 'get', 'target': target})

    def state(self, target):
        return self.ok({'command': 'get', 'target': target})['state']

    def check(self, name, condition, detail=''):
        self.report['checks'].append({'name': name, 'ok': bool(condition), 'detail': '' if condition else str(detail)[:400]})
        print(('PASS ' if condition else 'FAIL ') + name + ('' if condition else f'  [{str(detail)[:300]}]'), flush=True)
        if not condition:
            raise AssertionError(name)

    def fill(self, **fields):
        for name, value in fields.items():
            self.ok({'command': 'set', 'target': f'dialog.field.{name}', 'field': 'value', 'value': value})

    def wait_chain(self, wanted, seconds=30):
        deadline = time.time() + seconds
        while time.time() < deadline:
            current = self.state('scene_import')
            if current['state'] in wanted:
                return current
            time.sleep(0.1)
        raise AssertionError(f'the chain never reached {wanted}: {self.state("scene_import")}')

    def undo_depth(self):
        return self.op({'op': 'history.status'})['history']['undo_depth']

    def tools(self):
        return (self.send, self.ok, self.op, self.get, self.state, self.check, self.fill,
                self.wait_chain, self.undo_depth)

    def close(self):
        try:
            self.lab.stop()
        finally:
            self.lab.log.close()
            shutil.rmtree(self.home, ignore_errors=True)


def dialog_scenario(live):
    """The R2/S5 dialog flow: attach by job, typed failures, adopt, load, monitor, repair."""
    send, ok, op, get, state, check, fill, wait_chain, undo_depth = live.tools()
    lab, home, report = live.lab, live.home, live.report
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

def data_only_bundle(destination):
    """The corpus bundle with every node that names a file removed: groups only. Its three assets
    stay declared (the format requires one) but nothing uses them. Written to `destination`.
    Returns the bundle as a dict."""
    bundle = json.loads((CORPUS / 'bundle.json').read_text())
    kept = set()
    for definition in bundle['sceneDefinitions']:
        definition['nodes'] = [node for node in definition['nodes'] if node['type'] == 'group']
        kept |= {node['id'] for node in definition['nodes']}
    for track in bundle['performanceTracks']:
        track['channels'] = [channel for channel in track['channels'] if channel['nodeId'] in kept]
    destination.write_text(json.dumps(bundle))
    return bundle


def used_files(bundle):
    """How many distinct files a bundle's nodes ask for, counted from the raw JSON: the proof's own
    expectation, independent of the core's manifest."""
    return len({node['assetRef']['relativePath']
                for definition in bundle['sceneDefinitions'] for node in definition['nodes']
                if node.get('assetRef')})


def tree_files(folder):
    return sorted(str(path.relative_to(folder)) for path in Path(folder).rglob('*'))


def empty_manifest_scenario(live, shots=None):
    """BL101 on the real window: a scene whose manifest names no file attaches and loads with no
    project folder at all, the monitor draws it, and a scene that does name files keeps refusing."""
    send, ok, op, get, state, check, fill, wait_chain, undo_depth = live.tools()
    lab, home, report = live.lab, live.home, live.report
    lab.wait_ready(90)
    work = Path(home) / 'w'
    work.mkdir()
    data_path = work / 'data.bundle.json'
    data = data_only_bundle(data_path)
    before_tree = tree_files(Path(home))
    check('expectation: the data-only bundle names no file, the corpus bundle names three',
          used_files(data) == 0 and used_files(json.loads((CORPUS / 'bundle.json').read_text())) == 3,
          (used_files(data),))

    # --- a real project that was never saved: there is no folder anywhere -----------------------
    op({'op': 'project.create', 'name': 'Unsaved', 'sequence': 'main', 'rate': 24, 'width': 320, 'height': 180, 'length': 96})
    # That the project has no folder is proven below, by the refusal a scene with files gets.
    undo_before = undo_depth()

    # --- attach the data-only bundle through the dialog, NO folder ------------------------------
    opened = ok({'command': 'invoke', 'target': 'flow.attach_scene', 'action': 'open'})
    check('the attach dialog opens', opened.get('flow') == 'attach_scene', opened)
    fill(bundle=str(data_path), id='data', show='yes')
    check('the dialog has no folder', get('dialog.field.source')['data']['value'] in ('', None),
          get('dialog.field.source'))
    submitted = ok({'command': 'invoke', 'target': 'dialog.submit'})
    check('submitting starts the attach job, nothing is attached yet',
          submitted['action'] == 'scene.import' and submitted['changed'] is False
          and submitted['import']['step'] == 'attach', submitted)
    done = wait_chain({'loaded', 'failed'})
    check('the chain loads with no folder: attach, resolve, select (no adopt)',
          done['state'] == 'loaded' and done['error'] is None and done['done'][:3] == ['attach', 'resolve', 'select'],
          done)
    check('the host said the scene needs no folder and no files',
          done['attached']['resources'] == {'files': 0, 'needs_folder': False}, done['attached'])
    check('attach is one undo entry', undo_depth() == undo_before + 1, undo_depth())
    check('the scene is attached', [scene['id'] for scene in op({'op': 'scene.list'})['scenes']] == ['data'])
    status = op({'op': 'scene.resources.status'})
    entry = next(item for item in status['attached'] if item['scene'] == 'data')
    check('the host says the scene is loaded and current, against no base', entry['loaded'] and entry['current'], entry)
    check('the resources map carries no file and no base',
          not entry.get('base') and not any(item.get('path') for item in entry.get('resources', [])), entry)
    after_tree = tree_files(Path(home))
    check('no folder or file was created anywhere under the private HOME (no workspace, no scenes/ folder)',
          [path for path in after_tree if path not in before_tree and not path.startswith('.cache/')
           and path != 'editor.log'] == [], (before_tree, after_tree))
    jobs = state('jobs')['recent']
    actions = [event['result']['action'] for event in reversed(jobs) if event.get('result')]
    check('attach then resolve ran as jobs, adopt never ran',
          actions[:2] == ['scene.attach.start', 'scene.resources.resolve'] and 'scene.resources.adopt' not in actions, actions)

    # --- the real monitor: ready, on the scene, a frame drawn, equal to an explicit render -------
    deadline = time.time() + 30
    view = {}
    while time.time() < deadline:
        view = state('monitor.viewport')
        directed = view.get('directed') or {}
        if (view.get('phase') == 'ready' and directed.get('scene') == 'data'
                and directed.get('active') is None and directed.get('queued') is None
                and (view.get('last_frame') or {}).get('hash')):
            break
        time.sleep(0.1)
    directed = view.get('directed') or {}
    screen = view.get('last_frame') or {}
    check('the monitor is ready, on the data-only scene, and has drawn a frame',
          view.get('phase') == 'ready' and directed.get('scene') == 'data' and bool(screen.get('hash')), view)
    check('the WebView is on screen (no dialog open)', get('monitor.viewport')['data']['visible'] is True, get('monitor.viewport'))
    shown = (directed.get('shown') or {}).get('edit_frame', 0)
    reference = op({'op': 'scene.render', 'scene': 'data', 'frame': shown,
                    'width': screen['width'], 'height': screen['height']})['frame']
    check('what the monitor drew is the explicit render of the same frame (same hash)',
          screen['hash'] == reference['hash'], (screen.get('hash'), reference.get('hash')))
    report['monitor_frame_data_only'] = {'request': screen.get('request'), 'hash': screen.get('hash'),
                                         'width': screen.get('width'), 'height': screen.get('height'),
                                         'edit_frame': shown}
    if shots:
        report['screenshot_data_only'] = shots('data-only-monitor')

    # --- the UI stays responsive while a very large bundle is read (BL100) ----------------------
    big_path = work / 'big.bundle.json'
    big = json.loads(data_path.read_text())
    big['x-padding'] = '0' * (24 * 1024 * 1024)
    big_path.write_text(json.dumps(big))
    ok({'command': 'invoke', 'target': 'flow.attach_scene', 'action': 'open'})
    fill(bundle=str(big_path), id='big', show='no')
    started = time.perf_counter()
    submit_ms = None
    ok({'command': 'invoke', 'target': 'dialog.submit'})
    submit_ms = (time.perf_counter() - started) * 1000
    latencies = []
    stalls = []
    deadline = time.time() + 60
    while time.time() < deadline:
        tick = time.perf_counter()
        current = state('scene_import')
        latencies.append((time.perf_counter() - tick) * 1000)
        if latencies[-1] > 100:
            stalls.append({'at_ms': round((tick - started) * 1000), 'ms': round(latencies[-1])})
        if current['state'] in ('loaded', 'failed', 'attached'):
            break
        time.sleep(0.005)
    total_ms = (time.perf_counter() - started) * 1000
    report['large_bundle'] = {'bytes': big_path.stat().st_size, 'submit_ms': round(submit_ms, 1),
                              'polls': len(latencies), 'max_poll_ms': round(max(latencies), 1),
                              'stalls_over_100ms': stalls,
                              'total_ms': round(total_ms, 1), 'final': current['state']}
    check('the large bundle attaches (read, parsed and validated off the UI thread)',
          current['state'] == 'loaded', current)
    check('submitting a 24 MiB bundle returns at once and the window keeps answering while it is read',
          submit_ms < 1000 and max(latencies) < 1000, report['large_bundle'])
    # Reference only (no assertion): the same bundle through the synchronous `scene.attach`, which
    # holds the window for the whole read, parse and validation. What the job leaves on the UI
    # thread is the apply (`stalls_over_100ms`).
    sync_started = time.perf_counter()
    op({'op': 'scene.attach', 'id': 'bigsync', 'path': str(big_path)})
    report['large_bundle']['sync_scene_attach_ms'] = round((time.perf_counter() - sync_started) * 1000)

    # --- negative: a bundle that names files keeps every guard ----------------------------------
    corpus = authored(work, 'files')
    ok({'command': 'invoke', 'target': 'flow.attach_scene', 'action': 'open'})
    fill(bundle=str(corpus / 'bundle.json'), id='street', show='no')
    ok({'command': 'invoke', 'target': 'dialog.submit'})
    attached = wait_chain({'attached', 'loaded', 'failed'})
    check('a bundle with files and no folder stops at attached (it still needs one)',
          attached['state'] == 'attached' and attached['attached']['resources'] == {'files': 3, 'needs_folder': True},
          attached)
    refused = send({'version': 1, 'command': 'op', 'request': {'op': 'scene.resources.resolve', 'scene': 'street'}})
    check('resolving it without any project folder is still no_workspace',
          not refused['ok'] and refused['error']['code'] == 'no_workspace', refused)
    ok({'command': 'invoke', 'target': 'flow.scene_resources', 'action': 'open'})
    fill(scene='street', show='no')
    refused_dialog = send({'command': 'invoke', 'target': 'dialog.submit'})
    check('the repair dialog refuses it the same way and stays open',
          not refused_dialog['ok'] and refused_dialog['error']['code'] == 'no_workspace'
          and state('dialog')['flow'] == 'scene_resources', refused_dialog)
    ok({'command': 'invoke', 'target': 'dialog.cancel'})
    street = next(item for item in op({'op': 'scene.resources.status'})['attached'] if item['scene'] == 'street')
    check('the scene with files is attached and not loaded', not street['loaded'], street)

    # --- negative with a project folder: still root_unavailable ---------------------------------
    project_dir = work / 'ws'
    op({'op': 'project.create', 'name': 'Saved', 'sequence': 'main', 'rate': 24, 'width': 320, 'height': 180,
        'length': 96, 'dir': str(project_dir)})
    ok({'command': 'invoke', 'target': 'flow.attach_scene', 'action': 'open'})
    fill(bundle=str(corpus / 'bundle.json'), id='street', show='no')
    ok({'command': 'invoke', 'target': 'dialog.submit'})
    wait_chain({'attached', 'loaded', 'failed'})
    started_job = op({'op': 'scene.resources.resolve', 'scene': 'street'})
    final = {}
    deadline = time.time() + 30
    while time.time() < deadline:
        final = op({'op': 'job.get', 'job': started_job['job']})
        if final['state'] in ('succeeded', 'failed', 'cancelled'):
            break
        time.sleep(0.05)
    code = (final.get('error') or {}).get('code')
    check('with a project folder but no scenes/street under it, a scene with files is root_unavailable',
          final.get('state') == 'failed' and code == 'root_unavailable', final)
    street = next(item for item in op({'op': 'scene.resources.status'})['attached'] if item['scene'] == 'street')
    check('and nothing is loaded for it, no folder was created for it',
          not street['loaded'] and not (project_dir / 'scenes').exists(), street)
    # The data-only scene in that same project: loads, still creates nothing.
    ok({'command': 'invoke', 'target': 'flow.attach_scene', 'action': 'open'})
    fill(bundle=str(data_path), id='data', show='no')
    ok({'command': 'invoke', 'target': 'dialog.submit'})
    again = wait_chain({'loaded', 'failed'})
    check('the data-only scene also loads inside a saved project without creating scenes/',
          again['state'] == 'loaded' and not (project_dir / 'scenes').exists(), again)

def window_shots(live, folder):
    """Captures the private window with the OS (`screencapture -l <window id of that process>`):
    an observation, never an input. Returns a function name -> {path, bytes} or None."""
    Path(folder).mkdir(parents=True, exist_ok=True)

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

    def take(name):
        time.sleep(0.3)  # let the last frame reach the window
        window = window_id(live.lab.process.pid)
        if not window:
            return None
        path = Path(folder) / f'{name}.png'
        subprocess.run(['screencapture', '-x', '-o', '-l', window, str(path)], check=False)
        return {'path': str(path), 'bytes': path.stat().st_size if path.exists() else 0}

    return take


SCENARIOS = {'dialog': dialog_scenario, 'empty-manifest': empty_manifest_scenario}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', default=str(ROOT / 'target/debug/tessel-editor'))
    parser.add_argument('--out')
    parser.add_argument('--scenario', choices=['all', *SCENARIOS], default='all',
                        help='each scenario starts its own private instance')
    parser.add_argument('--shots', help='folder for OS window captures of the empty-manifest monitor')
    args = parser.parse_args()
    free_gib = shutil.disk_usage('/tmp').free / 1024 ** 3
    if free_gib < MIN_FREE_GIB:
        print(json.dumps({'ok': False, 'error': f'only {free_gib:.1f} GiB free, need {MIN_FREE_GIB}'}))
        return 1
    report = {'checks': [], 'input': 'semantic IPC (socket), no pointer, no coordinates',
              'not_proven': ['native file picker panel', 'visual appearance']}
    chosen = list(SCENARIOS) if args.scenario == 'all' else [args.scenario]
    report['ok'] = True
    for name in chosen:
        live = Live(args.binary, report, name)
        try:
            if name == 'empty-manifest':
                SCENARIOS[name](live, window_shots(live, args.shots) if args.shots else None)
            else:
                SCENARIOS[name](live)
        except Exception as error:
            report['ok'] = False
            report['error'] = f'{name}: {error!r}'
        finally:
            live.close()
        if not report['ok']:
            break
    if args.out:
        Path(args.out).write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'ok': report['ok'], 'checks': len(report['checks']), 'error': report.get('error')}))
    return 0 if report['ok'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
