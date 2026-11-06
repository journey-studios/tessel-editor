#!/usr/bin/env python3
"""F040/BL111.1 — `scene.resources.claims` and `scene.resources.recover` in a private native window.

    python3 scripts/directed-scene-proof/with-gpu-lock.py \
        python3 scripts/scene-claims-proof.py [--binary target/debug/tessel-editor] \
        [--media-binary target/debug/tessel-media] [--out report.json]

Starts the DIRECT editor binary with its own short HOME (own socket, own settings: the user's
window and project are never touched) and drives it with semantic requests only (`op`, `get`,
`invoke`), no pointer, no coordinates. A second, real `tessel-media` process plays the other
editor: it adopts a scene into the window project's workspace and is then killed with SIGKILL (or
kept alive). Files come from the synthetic corpus of `scripts/directed-scene-proof/corpus`.

Proves, in the live window: both operations answer with a job at once (nothing blocks the window)
and the shared job history carries their result, progress and typed errors; claims says it is not
read-only and lists the orphan entries without removing a file; recover removes only the orphans,
reports the job's progress in files and leaves the workspace as it was; a living process's holder
is not recovered; a ledger that cannot be read makes recover fail with `claims_unavailable` and
removes nothing; a file edited after the crash is kept and the job warns; a ledger whose lock was deleted is
damage (`available:false`, `recover` fails closed, nothing removed); the jobs dialog and the
`jobs` node show the jobs, and a finished job refuses `job.cancel` with a typed error.

It does NOT prove pixels (the monitor runtime is not used), the cancel arriving in the middle of a recovery (deterministic in the lib
tests), a native file picker, Windows or network filesystems.
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


def wait_for_socket(lab, seconds=90):
    deadline = time.time() + seconds
    while time.time() < deadline:
        if lab.process.poll() is not None:
            raise RuntimeError(f'the editor exited ({lab.process.returncode})')
        if os.path.exists(lab.socket_path):
            try:
                if lab.send({'version': 1, 'command': 'get', 'target': 'jobs'}).get('ok'):
                    return
            except (OSError, ValueError):
                pass
        time.sleep(0.25)
    raise RuntimeError('the window never answered on its socket')


class Media:
    """One `tessel-media` process, driven line by line (JSON Lines)."""

    def __init__(self, binary):
        self.process = subprocess.Popen([binary, '--empty'], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=subprocess.DEVNULL, text=True, bufsize=1)

    def ok(self, request):
        self.process.stdin.write(json.dumps(request) + '\n')
        self.process.stdin.flush()
        reply = json.loads(self.process.stdout.readline())
        if not reply.get('ok'):
            raise AssertionError(f'{request} -> {reply}')
        return reply['result']

    def crash(self):
        self.process.kill()
        self.process.wait()

    def finish(self):
        self.process.stdin.close()
        self.process.wait(30)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', default=str(ROOT / 'target/debug/tessel-editor'))
    parser.add_argument('--media-binary', default=str(ROOT / 'target/debug/tessel-media'))
    parser.add_argument('--out')
    args = parser.parse_args()
    free_gib = shutil.disk_usage('/tmp').free / 1024 ** 3
    if free_gib < MIN_FREE_GIB:
        print(json.dumps({'ok': False, 'error': f'only {free_gib:.1f} GiB free, need {MIN_FREE_GIB}'}))
        return 1
    report = {'checks': [], 'input': 'semantic IPC (socket), no pointer, no coordinates',
              'not_proven': ['cancel in the middle of a recovery', 'native file picker', 'Windows', 'network filesystems']}
    home = tempfile.mkdtemp(prefix='te-s2-', dir='/tmp')
    lab = video.Editor(str(Path(args.binary).resolve()), home)
    media_binary = str(Path(args.media_binary).resolve())
    others = []

    def send(request):
        return lab.send({'version': 1, **request})

    def ok(request):
        response = send(request)
        if not response.get('ok'):
            raise AssertionError(f'{request} -> {response}')
        return response['data']

    def op(request):
        return ok({'command': 'op', 'request': request})

    def refused(request):
        response = send({'command': 'op', 'request': request})
        assert not response.get('ok'), response
        return response['error']

    def state(target):
        return ok({'command': 'get', 'target': target})['state']

    def check(name, condition, detail=''):
        report['checks'].append({'name': name, 'ok': bool(condition), 'detail': '' if condition else str(detail)[:400]})
        print(('PASS ' if condition else 'FAIL ') + name + ('' if condition else f'  [{str(detail)[:300]}]'), flush=True)
        if not condition:
            raise AssertionError(name)

    def final_job(started, seconds=30):
        """The window never blocks: the operation answers with a job and the tick finishes it."""
        job = started['job']
        deadline = time.time() + seconds
        while time.time() < deadline:
            info = op({'op': 'job.get', 'job': job})
            info = info.get('info', info)
            if info['state'] in ('succeeded', 'failed', 'cancelled'):
                return info
            time.sleep(0.1)
        raise AssertionError(f'the job never ended: {job}')

    def tree(folder):
        return sorted(str(path.relative_to(folder)) for path in Path(folder).rglob('*'))

    def resources(folder):
        """The tree without the ledger's own bookkeeping (opening it reaps a dead owner's lease file)."""
        return [path for path in tree(folder) if not path.startswith('.tessel-claims')]

    def other_editor(workspace, name):
        """A second process in the window's workspace holding a pending adoption of the scene."""
        bundle_dir = Path(home) / f'authored-{name}'
        (bundle_dir / 'assets').mkdir(parents=True)
        shutil.copy(CORPUS / 'bundle.json', bundle_dir / 'bundle.json')
        for asset in ('sky.png', 'mid.png', 'hero.png'):
            shutil.copy(CORPUS / 'assets' / asset, bundle_dir / 'assets' / asset)
        session = Media(media_binary)
        others.append(session)
        session.ok({'op': 'project.save', 'path': str(workspace / f'{name}.tessel.json')})
        session.ok({'op': 'scene.attach', 'id': 'street', 'path': str(bundle_dir / 'bundle.json')})
        started = session.ok({'op': 'scene.resources.adopt', 'scene': 'street', 'source': str(bundle_dir), 'wait': False})
        done = session.ok({'op': 'job.wait', 'job': started['job']})
        assert done['state'] == 'succeeded', done
        return session

    try:
        # The monitor's WebView runtime is not needed (no pixel is claimed): the window is ready
        # when its socket answers, so the proof also runs where `./scripts/runtime setup` was not.
        wait_for_socket(lab)
        workspace = Path(home) / 'ws'
        op({'op': 'project.create', 'name': 'Claims', 'dir': str(workspace)})
        workspace = Path(os.path.realpath(workspace))
        base = tree(workspace)
        assets = workspace / 'scenes/street/assets'

        # --- a crashed editor leaves an orphan; claims shows it without touching a file ---------
        victim = other_editor(workspace, 'victim')
        check('the other editor copied the scene into the window workspace', (assets / 'hero.png').is_file(), tree(workspace))
        copied = tree(workspace)
        resources_copied = resources(workspace)
        victim.crash()
        started = op({'op': 'scene.resources.claims'})
        check('claims answers with a job at once, the window does not wait', isinstance(started.get('job'), str), started)
        claims = final_job(started)
        check('the claims job succeeded', claims['state'] == 'succeeded', claims)
        result = claims['result']
        check('claims says it is not read-only and why',
              result['readOnly'] is False and 'reaps' in result['effects'], result)
        check('the dead owner was reaped and its entries are orphans',
              len(result['reaped']) == 1 and result['orphans'] >= 3 and result['available'] is True, result)
        check('claims removed no resource file', resources(workspace) == resources_copied, resources(workspace))
        check('the job is in the window state', any(job['id'] == claims['id'] for job in state('jobs')['all']), state('jobs'))

        # --- recover: a job with progress, only the orphans, workspace as it was ----------------
        started = op({'op': 'scene.resources.recover'})
        check('recover answers with a job at once', isinstance(started.get('job'), str), started)
        recovered = final_job(started)
        check('the recover job succeeded', recovered['state'] == 'succeeded', recovered)
        check('it reports progress in orphan files', recovered['done'] == recovered['total'] == 3, recovered)
        cleanup = recovered['result']['recovery']['cleanup']
        check('it removed the three files and the folders and kept nothing',
              cleanup['removedFiles'] == 3 and cleanup['kept'] == [], recovered)
        check('the workspace is as it was before the other editor (plus its own project file)',
              tree(workspace) == sorted(base + ['victim.tessel.json']), tree(workspace))
        check('nothing is claimed any more', op({'op': 'scene.resources.claims'}) is not None and
              final_job(op({'op': 'scene.resources.claims'}))['result']['orphans'] == 0)

        # --- a living editor's holder is not recovered ------------------------------------------
        living = other_editor(workspace, 'living')
        recovered = final_job(op({'op': 'scene.resources.recover'}))
        check('recover removes nothing a living process holds',
              recovered['state'] == 'succeeded' and recovered['result']['recovery']['cleanup']['removedFiles'] == 0
              and (assets / 'hero.png').is_file(), recovered)
        living.finish()
        check('when that editor ends it removes its own pending files', not (workspace / 'scenes').exists(), tree(workspace))

        # --- an edit after the crash is kept, and the job warns ---------------------------------
        victim = other_editor(workspace, 'editor')
        victim.crash()
        data = bytearray((assets / 'sky.png').read_bytes())
        data[-1] ^= 0xFF
        (assets / 'sky.png').write_bytes(bytes(data))
        edited = (assets / 'sky.png').read_bytes()
        recovered = final_job(op({'op': 'scene.resources.recover'}))
        check('the edited file is kept and the others go',
              recovered['state'] == 'succeeded' and (assets / 'sky.png').read_bytes() == edited
              and not (assets / 'hero.png').exists(), recovered)
        check('the job warns about the file it kept',
              any('rewritten in place' in warning for warning in recovered['warnings']), recovered['warnings'])
        shutil.rmtree(workspace / 'scenes')

        # --- an unreadable ledger fails closed ---------------------------------------------------
        victim = other_editor(workspace, 'broken')
        victim.crash()
        (workspace / '.tessel-claims/claims.json').write_text('{ not a ledger')
        copied = tree(workspace)
        claims = final_job(op({'op': 'scene.resources.claims'}))
        check('claims reports the ledger as unavailable instead of failing',
              claims['state'] == 'succeeded' and claims['result']['available'] is False, claims)
        failed = final_job(op({'op': 'scene.resources.recover'}))
        check('recover fails with the typed code and removes nothing',
              failed['state'] == 'failed' and failed['error']['code'] == 'claims_unavailable'
              and tree(workspace) == copied, failed)

        # --- a ledger whose lock was deleted is damage, not an empty ledger (BL111.2) -----------
        shutil.rmtree(workspace / '.tessel-claims')
        shutil.rmtree(workspace / 'scenes')
        victim = other_editor(workspace, 'lockless')
        victim.crash()
        (workspace / '.tessel-claims/ledger.lock').unlink()
        copied = tree(workspace)
        claims = final_job(op({'op': 'scene.resources.claims'}))
        check('claims reports the lockless ledger as damaged, not empty',
              claims['state'] == 'succeeded' and claims['result']['available'] is False
              and 'ledger.lock' in (claims['result']['fault'] or ''), claims)
        lockless = final_job(op({'op': 'scene.resources.recover'}))
        check('recover on a lockless ledger fails closed and removes nothing',
              lockless['state'] == 'failed' and lockless['error']['code'] == 'claims_unavailable'
              and tree(workspace) == copied and (assets / 'hero.png').is_file(), lockless)

        # --- controls: the jobs dialog and the semantic cancel ----------------------------------
        opened = ok({'command': 'invoke', 'target': 'flow.jobs', 'action': 'open'})
        check('the jobs dialog opens', opened.get('flow') == 'jobs' or opened.get('changed') is True, opened)
        error = refused({'op': 'job.cancel', 'job': failed['id']})
        check('cancelling a finished recover job is a typed refusal', error['code'] in ('job_finished', 'invalid_state'), error)
        report['ok'] = True
    except Exception as error:
        report['ok'] = False
        report['error'] = repr(error)
    finally:
        for session in others:
            try:
                session.process.kill()
            except OSError:
                pass
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
