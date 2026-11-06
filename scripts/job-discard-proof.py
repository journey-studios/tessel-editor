#!/usr/bin/env python3
"""F025/F040 (BL094, BL112) — `job.discard` and a window that does not hash, in real processes.

    python3 scripts/directed-scene-proof/with-gpu-lock.py \
        python3 scripts/job-discard-proof.py [--pad-mib 128] [--out report.json]
    # build first: ./scripts/cargo build -p tessel-editor -p tessel-editor-media --locked

Two surfaces, each a DIRECT process of its own (never the user's window, project or socket):

* JSONL: `target/debug/tessel-media --empty`, a real process fed JSON Lines on stdin. No desktop, no
  socket, no GPU. Proves the headless contract: a finished adoption is discarded by `job.discard`
  (the host waits for the removal), refusals are typed and change nothing, a stale commit is
  removed before the answer, and a process that ends with a result nobody applied takes the
  files back before it exits.
* Window: `target/debug/tessel-editor` with its own short HOME (own socket, own settings), driven
  only by semantic requests (`op`, `get`), no pointer, no coordinates. A scene adoption of a
  large file is made stale while its job runs; the window's tick finds it and must hand the
  removal to a `discard` job. While that job removes the files, a second client keeps asking the
  window for its state: every answer is timed. A control first suspends OUR window process for
  1.5 s and the same timed request must report it, so a passing measurement cannot come from a
  probe that is blind. Requests run on the window's main thread, so a stalled UI thread is a late
  answer; frames per second and paint time are not measured.

The ceilings below were fixed before the first measurement and are not tuned to pass.

It does NOT prove: pixels or aesthetics (a jobs-only change draws nothing new), the native file
picker, a second process sharing the workspace (BL111), a hostile writer, Windows/Linux, or what
the window does when the user closes it with results pending (the `Drop` is covered by the
`job_discard` test, not observed here).
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parent.parent
CORPUS = ROOT / 'scripts/directed-scene-proof/corpus'
spec = importlib.util.spec_from_file_location('video_proof', ROOT / 'scripts/video-live-proof.py')
video = importlib.util.module_from_spec(spec)
spec.loader.exec_module(video)

MIN_FREE_GIB = 12
# Ceilings for the window's answers while it is not blocked on purpose (milliseconds).
IDLE_P99_MS = 100.0
DISCARD_P99_MS = 100.0
DISCARD_MAX_MS = 250.0
MIN_SAMPLES_DURING_DISCARD = 20
# The control must stall the probe at least this long, else the probe cannot see a stall.
CONTROL_MIN_STALL_MS = 1000.0
PROBE_GAP_S = 0.02


def sha256(path):
    digest = hashlib.sha256()
    with open(path, 'rb') as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b''):
            digest.update(chunk)
    return digest.hexdigest()


def authored(base, name, pad_mib=0):
    """The synthetic corpus bundle copied under `base/name`, `sky.png` padded after its image data
    (the transport reads only the header) and the bundle restated with that file's real SHA-256."""
    folder = Path(base) / name
    (folder / 'assets').mkdir(parents=True)
    bundle = json.loads((CORPUS / 'bundle.json').read_text())
    for asset in ('sky.png', 'mid.png', 'hero.png'):
        shutil.copy(CORPUS / 'assets' / asset, folder / 'assets' / asset)
    if pad_mib:
        sky = folder / 'assets/sky.png'
        with open(sky, 'ab') as handle:
            handle.write(bytes(pad_mib << 20))
        digest = sha256(sky)
        for entry in bundle['assets']:
            if entry['assetId'] == 'sky':
                entry['contentHash'] = digest
        for definition in bundle['sceneDefinitions']:
            for node in definition['nodes']:
                if (node.get('assetRef') or {}).get('assetId') == 'sky':
                    node['assetRef']['contentHash'] = digest
    (folder / 'bundle.json').write_text(json.dumps(bundle, indent=2))
    return folder


def percentile(values, share):
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(share * len(ordered)))]


def stats(values):
    return {'n': len(values), 'p50_ms': round(percentile(values, 0.5), 1),
            'p99_ms': round(percentile(values, 0.99), 1), 'max_ms': round(max(values), 1)} if values else {'n': 0}


class Report:
    def __init__(self):
        self.data = {'checks': [], 'input': 'JSONL on stdin / semantic IPC (socket); no pointer, no coordinates',
                     'not_proven': ['pixels or aesthetics', 'native file picker', 'two processes (BL111)',
                                    'hostile writer', 'non-macOS', 'window closed with results pending (test only)']}

    def check(self, name, condition, detail=''):
        self.data['checks'].append({'name': name, 'ok': bool(condition), 'detail': '' if condition else str(detail)[:500]})
        print(('PASS ' if condition else 'FAIL ') + name + ('' if condition else f'  [{str(detail)[:400]}]'), flush=True)
        if not condition:
            raise AssertionError(name)


class Jsonl:
    """A real `tessel-media --empty` process: one request line, one answer line."""

    def __init__(self, binary):
        self.process = subprocess.Popen([binary, '--empty'], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        text=True, bufsize=1)

    def op(self, request):
        self.process.stdin.write(json.dumps(request) + '\n')
        self.process.stdin.flush()
        return json.loads(self.process.stdout.readline())

    def ok(self, request):
        answer = self.op(request)
        assert answer.get('ok'), f'{request} -> {answer}'
        return answer['result']

    def err(self, request):
        answer = self.op(request)
        assert not answer.get('ok'), f'{request} should have been refused: {answer}'
        return answer['error']

    def close(self):
        """Ends stdin like a CLI session that is over; returns how long the process took to exit."""
        self.process.stdin.close()
        started = time.perf_counter()
        self.process.wait(timeout=600)
        return time.perf_counter() - started


def scene_files(workspace):
    root = Path(workspace) / 'scenes/street/assets'
    return [root / name for name in ('sky.png', 'mid.png', 'hero.png')]


def jsonl_phase(binary, work, report, pad_mib):
    check = report.check
    results = {}

    def session(name):
        workspace = Path(work) / f'jsonl-{name}'
        proc = Jsonl(binary)
        proc.ok({'op': 'project.create', 'name': name, 'dir': str(workspace)})
        return proc, workspace

    def adopt(proc, source, bundle):
        proc.ok({'op': 'scene.attach', 'id': 'street', 'path': str(bundle)})
        job = proc.ok({'op': 'scene.resources.adopt', 'scene': 'street', 'source': str(source), 'wait': False})['job']
        waited = proc.ok({'op': 'job.wait', 'job': job, 'timeout_ms': 600000})
        assert waited['state'] == 'succeeded', waited
        return job

    def result_state(proc, job):
        return proc.ok({'op': 'job.get', 'job': job})['result_state']

    # --- discard a finished, unapplied result ---------------------------------------------------
    source = authored(work, 'j-src')
    proc, workspace = session('discard')
    job = adopt(proc, source, source / 'bundle.json')
    check('JSONL: a finished adoption is pending and owns its copies',
          result_state(proc, job) == 'pending' and all(path.is_file() for path in scene_files(workspace)))
    answer = proc.ok({'op': 'job.discard', 'job': job})
    check('JSONL: job.discard waits and reports what it removed',
          answer['state'] == 'discarded' and answer['discards'] == job and answer['discarded'] == 'scene-adopt'
          and answer['cleanup']['removedFiles'] == 3, answer)
    check('JSONL: the copies are gone and the folder with them',
          not any(path.exists() for path in scene_files(workspace)) and not (workspace / 'scenes').exists())
    check('JSONL: the original job is discarded', result_state(proc, job) == 'discarded')
    jobs = proc.ok({'op': 'job.list'})['jobs']
    discard_job = next(entry for entry in jobs if entry['kind'] == 'discard')
    check('JSONL: the discard is an observable job that succeeded', discard_job['state'] == 'succeeded', discard_job)
    for request, code in (({'op': 'job.discard', 'job': job}, 'job_discarded'),
                          ({'op': 'job.commit', 'job': job}, 'job_discarded'),
                          ({'op': 'job.discard', 'job': 'job-999'}, 'unknown_element'),
                          ({'op': 'job.cancel', 'job': discard_job['id']}, 'job_finished')):
        refused = proc.err(request)
        check(f'JSONL: {request["op"]} -> {code}', refused['code'] == code, refused)
    check('JSONL: refusals changed nothing', result_state(proc, job) == 'discarded')
    proc.close()

    # --- an applied result is refused and untouched --------------------------------------------
    proc, workspace = session('applied')
    job = adopt(proc, source, source / 'bundle.json')
    hashes = {path.name: sha256(path) for path in scene_files(workspace)}
    proc.ok({'op': 'job.commit', 'job': job})
    check('JSONL: a committed result is applied', result_state(proc, job) == 'applied')
    refused = proc.err({'op': 'job.discard', 'job': job})
    check('JSONL: discarding an applied result is job_applied', refused['code'] == 'job_applied', refused)
    check('JSONL: ...and the project files are byte-identical',
          {path.name: sha256(path) for path in scene_files(workspace)} == hashes
          and result_state(proc, job) == 'applied')
    proc.close()
    check('JSONL: an accepted result keeps its files after the process ends',
          all(path.is_file() for path in scene_files(workspace)))

    # --- nothing to discard: a result that failed -------------------------------------------------
    proc, workspace = session('failed')
    proc.ok({'op': 'scene.attach', 'id': 'street', 'path': str(source / 'bundle.json')})
    missing = Path(work) / 'nowhere'
    started = proc.op({'op': 'scene.resources.adopt', 'scene': 'street', 'source': str(missing), 'wait': False})
    failed = proc.ok({'op': 'job.wait', 'job': started['result']['job'], 'timeout_ms': 60000}) if started.get('ok') else None
    if failed is not None:
        refused = proc.err({'op': 'job.discard', 'job': started['result']['job']})
        check('JSONL: a failed job has nothing to discard', failed['state'] == 'failed'
              and refused['code'] == 'job_nothing_to_discard', (failed, refused))
    else:
        check('JSONL: a failed adopt is refused up front (no job to discard)', not started['ok'], started)
    proc.close()

    # --- a stale commit removes before it answers (a host that may block) --------------------------
    proc, workspace = session('stale')
    job = adopt(proc, source, source / 'bundle.json')
    proc.ok({'op': 'scene.detach', 'scene': 'street'})
    refused = proc.err({'op': 'job.commit', 'job': job})
    check('JSONL: a stale commit is stale_result', refused['code'] == 'stale_result', refused)
    check('JSONL: the files were removed before the answer arrived (no UI to protect)',
          not (workspace / 'scenes').exists(), refused)
    check('JSONL: the stale job ends discarded', result_state(proc, job) == 'discarded')
    proc.close()

    # --- ending the session with a result nobody applied: the files are back before the exit ------
    big = authored(work, 'j-big', pad_mib=max(8, pad_mib // 2))
    proc, workspace = session('exit')
    job = adopt(proc, big, big / 'bundle.json')
    check('JSONL: a big pending result owns its copies before the end',
          all(path.is_file() for path in scene_files(workspace)) and result_state(proc, job) == 'pending')
    took = proc.close()
    check('JSONL: the process exited only after taking the files back',
          not (workspace / 'scenes').exists() and proc.process.returncode == 0,
          (proc.process.returncode, list((workspace).rglob('*'))))
    results['jsonl_exit_with_pending_seconds'] = round(took, 2)
    return results


# --- the window ---------------------------------------------------------------------------------

class Probe:
    """A second client that keeps asking the window for its state and times every answer."""

    def __init__(self, lab):
        self.lab = lab
        self.samples = []  # (monotonic, latency_ms, jobs_state)
        self.stop = threading.Event()
        self.thread = None

    def start(self):
        self.stop.clear()
        self.thread = threading.Thread(target=self._run, daemon=True)
        self.thread.start()

    def _run(self):
        while not self.stop.is_set():
            started = time.perf_counter()
            try:
                answer = self.lab.send({'version': 1, 'command': 'get', 'target': 'jobs'}, timeout=120)
            except OSError:
                continue
            latency = (time.perf_counter() - started) * 1000
            state = answer.get('data', {}).get('state') if answer.get('ok') else None
            self.samples.append((time.monotonic(), latency, state))
            self.stop.wait(PROBE_GAP_S)

    def end(self):
        self.stop.set()
        self.thread.join(30)
        return self.samples


def window_phase(binary, work, report, pad_mib):
    check = report.check
    results = {}
    home = tempfile.mkdtemp(prefix='te-s1-', dir='/tmp')
    os.environ['TESSEL_VIEWPORT_INSPECT'] = '1'
    lab = video.Editor(str(Path(binary).resolve()), home)

    def send(request, timeout=60):
        return lab.send({'version': 1, **request}, timeout)

    def op(request, timeout=60):
        answer = send({'command': 'op', 'request': request}, timeout)
        assert answer.get('ok'), f'{request} -> {answer}'
        return answer['data']

    def refused(request):
        answer = send({'command': 'op', 'request': request})
        assert not answer.get('ok'), f'{request} should be refused: {answer}'
        return answer['error']

    def jobs():
        return send({'command': 'get', 'target': 'jobs'})['data']['state']

    def job_info(job):
        return next((entry for entry in jobs()['all'] if entry['id'] == job), None)

    def wait_for(predicate, seconds, what):
        deadline = time.time() + seconds
        while time.time() < deadline:
            value = predicate()
            if value:
                return value
            time.sleep(0.01)
        raise AssertionError(f'never: {what}')

    try:
        lab.wait_ready(90)
        check('the window answers semantic requests and shows no job', jobs()['all'] == [], jobs())

        # --- idle baseline -----------------------------------------------------------------------
        jobs()  # the first request pays for connection set-up and is not part of the sample
        idle = []
        for _ in range(60):
            started = time.perf_counter()
            jobs()
            idle.append((time.perf_counter() - started) * 1000)
            time.sleep(PROBE_GAP_S)
        results['idle'] = stats(idle)
        check(f'idle answers: p99 <= {IDLE_P99_MS} ms', percentile(idle, 0.99) <= IDLE_P99_MS, results['idle'])

        # --- control: the probe must see a window that does not answer ----------------------------
        # Requests are served on the window's main thread (`window.update`), so a stall of that
        # thread is a late answer. No operation of the editor blocks it on purpose (an adoption
        # started with `wait:true` returns at once in the window), so the control suspends OUR
        # window process for 1.5 s and the same timed request must report that stall.
        resume = threading.Timer(1.5, os.kill, (lab.process.pid, signal.SIGCONT))
        os.kill(lab.process.pid, signal.SIGSTOP)
        resume.start()
        started = time.perf_counter()
        send({'command': 'get', 'target': 'jobs'}, timeout=60)
        stall = (time.perf_counter() - started) * 1000
        resume.join()
        results['control'] = {'suspended_window_ms': 1500, 'probe_saw_ms': round(stall, 1)}
        check('control: a window that stops answering is measured as a stall by the same request',
              stall >= CONTROL_MIN_STALL_MS, results['control'])
        check('control: the window answers normally again afterwards', jobs()['all'] == [], jobs())

        # --- the stale result: the window's tick must not remove, a discard job does ---------------
        src = authored(work, 'w-src', pad_mib=pad_mib)
        workspace = Path(work) / 'w-ws'
        op({'op': 'project.create', 'name': 'Discard', 'dir': str(workspace)})
        op({'op': 'scene.attach', 'id': 'street', 'path': str(src / 'bundle.json')})
        job = op({'op': 'scene.resources.adopt', 'scene': 'street', 'source': str(src), 'wait': False})['job']
        wait_for(lambda: (job_info(job) or {}).get('state') == 'running', 30, 'the adopt job runs')
        # The result is now guaranteed stale: the scene it was started for goes away while it runs.
        op({'op': 'scene.detach', 'scene': 'street'})
        probe = Probe(lab)
        probe.start()
        discard_job = None
        timeline = []

        def watch():
            nonlocal discard_job
            info = job_info(job)
            if info is None:
                return False
            if not timeline or timeline[-1] != info['result_state']:
                timeline.append(info['result_state'])
            if info['result_state'] == 'discarding' and discard_job is None:
                discard_job = next((entry['id'] for entry in jobs()['all']
                                    if entry['kind'] == 'discard' and entry['label'].endswith(job)), None)
            return info['result_state'] == 'discarded'

        files_during = []
        negative = {}
        deadline = time.time() + 600
        while time.time() < deadline:
            if watch():
                break
            if timeline and timeline[-1] == 'discarding' and not negative and discard_job:
                # The files and their owner are still there while the discard job runs.
                files_during = [path.is_file() for path in scene_files(workspace)]
                negative['discard_again'] = refused({'op': 'job.discard', 'job': job})['code']
                negative['commit'] = refused({'op': 'job.commit', 'job': job})['code']
                negative['cancel_discard'] = refused({'op': 'job.cancel', 'job': discard_job})['code']
                negative['files_after_refusals'] = [path.is_file() for path in scene_files(workspace)]
                negative['state_after_refusals'] = job_info(job)['result_state']
                # The Tarefas dialog renders a result that is being discarded (no cancel button for
                # the discard job, "descartando o resultado" in its row); its rows are painted, not
                # semantic nodes (`get dialog` has no node for it), so only that the flow opens and
                # closes without error is observable.
                opened = send({'command': 'invoke', 'target': 'flow.jobs'})
                negative['jobs_dialog_opened'] = bool(opened.get('ok')) and opened['data'].get('changed') is True
                negative['jobs_dialog_closed'] = bool(send({'command': 'invoke', 'target': 'dialog.cancel'}).get('ok'))
            time.sleep(0.005)
        samples = probe.end()
        results['timeline'] = timeline
        check('the window moved the stale result through pending -> discarding -> discarded',
              timeline[-3:] == ['pending', 'discarding', 'discarded'] or timeline[-2:] == ['discarding', 'discarded'],
              timeline)
        check('the tick named the discard job instead of removing', discard_job is not None, timeline)
        check('while the discard job runs, every copied file is still there (the result owns them)',
              files_during == [True, True, True], (files_during, negative))
        check('while it runs: discard again and commit are job_discarding, the discard job is not cancellable',
              negative.get('discard_again') == 'job_discarding' and negative.get('commit') == 'job_discarding'
              and negative.get('cancel_discard') == 'job_not_cancellable', negative)
        check('the Tarefas dialog opens and closes while a result is being discarded',
              negative.get('jobs_dialog_opened') and negative.get('jobs_dialog_closed'), negative)
        check('the refusals changed nothing',
              negative.get('files_after_refusals') == [True, True, True]
              and negative.get('state_after_refusals') == 'discarding', negative)
        check('the files and the folder are gone once the result is discarded',
              not any(path.exists() for path in scene_files(workspace)) and not (workspace / 'scenes').exists())
        finished = job_info(discard_job)
        check('the discard job succeeded and removed the three copies',
              finished['state'] == 'succeeded' and finished['result']['cleanup']['removedFiles'] == 3, finished)
        again = refused({'op': 'job.discard', 'job': job})
        check('a discarded result is job_discarded', again['code'] == 'job_discarded', again)

        # While the files were being removed, the window kept answering.
        during = [latency for moment, latency, state in samples
                  if state and next((e['result_state'] for e in state['all'] if e['id'] == job), None) == 'discarding']
        results['during_discard'] = stats(during)
        moments = [m for m, _, s in samples if s and next(
            (e['result_state'] for e in s['all'] if e['id'] == job), None) == 'discarding']
        results['discard_window_seconds'] = round(moments[-1] - moments[0], 2) if moments else None
        check(f'the probe sampled the window while it removed (>= {MIN_SAMPLES_DURING_DISCARD} answers)',
              len(during) >= MIN_SAMPLES_DURING_DISCARD, results['during_discard'])
        check(f'responsive while removing: p99 <= {DISCARD_P99_MS} ms and max <= {DISCARD_MAX_MS} ms',
              percentile(during, 0.99) <= DISCARD_P99_MS and max(during) <= DISCARD_MAX_MS, results['during_discard'])

        # The tick reports the end of the discard job on its next beat (250 ms), not at once.
        wait_for(lambda: any(event.get('kind') == 'discard' for event in jobs()['recent']), 10,
                 'the tick reports the discard job')
        recent = jobs()['recent']
        events = [event for event in recent if event.get('kind') == 'discard']
        check('the window reported the end of the discard once (status-bar event)',
              len(events) == 1 and events[0]['state'] == 'succeeded' and events[0]['job'] == discard_job, recent)
        stale = [event for event in recent if event.get('job') == job and (event.get('error') or {}).get('code') == 'stale_result']
        check('the stale apply event names the discard job', stale and stale[0]['error'].get('details', {}).get('discard_job') == discard_job, recent)

        # --- an applied result: refused, untouched --------------------------------------------------
        small = authored(work, 'w-small')
        op({'op': 'scene.attach', 'id': 'street', 'path': str(small / 'bundle.json')})
        job = op({'op': 'scene.resources.adopt', 'scene': 'street', 'source': str(small), 'wait': False})['job']
        wait_for(lambda: (job_info(job) or {}).get('result_state') == 'applied', 30, 'the tick applies a current result')
        hashes = {path.name: sha256(path) for path in scene_files(workspace)}
        error = refused({'op': 'job.discard', 'job': job})
        check('the window refuses to discard an applied result by code', error['code'] == 'job_applied', error)
        check('...and the accepted files are byte-identical and the state unchanged',
              {path.name: sha256(path) for path in scene_files(workspace)} == hashes
              and job_info(job)['result_state'] == 'applied')
        report.data['window_results'] = results
        return results
    finally:
        try:
            lab.stop()
        finally:
            lab.log.close()
            report.data['window_results'] = results
            shutil.rmtree(home, ignore_errors=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--editor', default=str(ROOT / 'target/debug/tessel-editor'))
    parser.add_argument('--media', default=str(ROOT / 'target/debug/tessel-media'))
    parser.add_argument('--pad-mib', type=int, default=128)
    parser.add_argument('--out')
    args = parser.parse_args()
    free_gib = shutil.disk_usage('/tmp').free / 1024 ** 3
    if free_gib < MIN_FREE_GIB:
        print(json.dumps({'ok': False, 'error': f'only {free_gib:.1f} GiB free, need {MIN_FREE_GIB}'}))
        return 1
    report = Report()
    report.data['binaries'] = {'editor': args.editor, 'media': args.media}
    report.data['pad_mib'] = args.pad_mib
    report.data['ceilings_ms'] = {'idle_p99': IDLE_P99_MS, 'discard_p99': DISCARD_P99_MS, 'discard_max': DISCARD_MAX_MS,
                                  'control_min_stall': CONTROL_MIN_STALL_MS}
    work = tempfile.mkdtemp(prefix='te-s1w-', dir='/tmp')
    try:
        report.data['jsonl'] = jsonl_phase(args.media, work, report, args.pad_mib)
        window_phase(args.editor, work, report, args.pad_mib)
        report.data['ok'] = True
    except Exception as error:
        report.data['ok'] = False
        report.data['error'] = repr(error)
    finally:
        shutil.rmtree(work, ignore_errors=True)
        if args.out:
            Path(args.out).write_text(json.dumps(report.data, indent=2) + '\n')
        print(json.dumps({'ok': report.data.get('ok'), 'checks': len(report.data['checks']),
                          'error': report.data.get('error')}))
    return 0 if report.data.get('ok') else 1


if __name__ == '__main__':
    raise SystemExit(main())
