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
import collections
import contextlib
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import shutil
import signal
import socket
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
# How long `Probe.end` waits for its thread; a thread still alive after that is a finding, never hidden.
PROBE_JOIN_S = 30
# How long a session's child gets to take pending files back and exit once its stdin is closed.
CHILD_EXIT_S = 120


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


# One probe answer. `sent` and `received` are `time.monotonic()` instants taken around the request;
# `state` is the `jobs` payload the window computed when it finally answered.
Sample = collections.namedtuple('Sample', 'sent received latency_ms state')
# One request that got NO answer: `sent`/`failed` are `time.monotonic()` instants (`failed` is None for a
# request still in flight when the probe stopped), `cause` is 'timeout' | 'network' | 'protocol' |
# 'unexpected' | 'unfinished', `error` the exception text. It is never classified by an answer (there is none).
Failure = collections.namedtuple('Failure', 'sent failed latency_ms cause error')


def result_state_of(sample, job):
    """What the window's own answer said about `job`; None when the job was not listed (yet)."""
    if not sample.state:
        return None
    return next((entry['result_state'] for entry in sample.state['all'] if entry['id'] == job), None)


def discard_window(samples, job, failures=()):
    """The monotonic span `(start, end)` of the discard, taken from instants, never from the state an
    answer carries (BL112.12).

    A request that is sent while the window is removing files and answered late is labelled with the
    state of the moment the window finally replied: 'discarded'. Classifying by that label would drop
    exactly the answers a stalled UI delays. So the span is bounded by when requests were SENT and
    RECEIVED, wide on purpose:

    * start: the send instant of the last request that was still answered before the discard began
      (the first answer that says 'discarding' or 'discarded' comes after it). Everything sent from
      then on may have run during the discard. If the first answer already shows the discard,
      include earlier failed sends too: no preceding answer established an idle boundary;
    * end: the instant the first 'discarded' answer was received; the discard ended before the
      window could produce it. With no such answer the end of the discard is unknown, so the span ends at
      the last instant the probe recorded, an answer or a failure (a request that failed after the last
      answer must not fall outside the window just because nothing answered it).

    Samples come in the order the (single) probe thread completed them, which is the order the
    window handled them. None when no answer ever showed the discard began."""
    states = [result_state_of(sample, job) for sample in samples]
    began = next((i for i, state in enumerate(states) if state in ('discarding', 'discarded')), None)
    if began is None:
        return None
    start = samples[began - 1].sent if began else samples[0].sent
    if began == 0:
        start = min([start] + [failure.sent for failure in failures])
    done = next((i for i, state in enumerate(states) if state == 'discarded'), None)
    if done is not None:
        return start, samples[done].received
    return start, max([samples[-1].received] + [f.failed if f.failed is not None else f.sent for f in failures])


def latencies_sent_within(samples, start, end):
    """Every answer to a request SENT inside `[start, end]`, whatever state it came back labelled with."""
    return [sample.latency_ms for sample in samples if start <= sample.sent <= end]


def failures_sent_within(failures, start, end):
    """Every request SENT inside `[start, end]` that got no answer, whatever happened to it afterwards."""
    return [failure for failure in failures if start <= failure.sent <= end]


def finite(value, digits=4):
    """A JSON-safe number: None for None, NaN and infinity (the report is written with `allow_nan=False` in spirit)."""
    return round(value, digits) if isinstance(value, (int, float)) and math.isfinite(value) else None


def failure_record(failure, origin):
    """One failed request as report data; instants are seconds from `origin`, never NaN/inf."""
    return {'sent_s': finite(failure.sent - origin), 'failed_s': finite(None if failure.failed is None else failure.failed - origin),
            'latency_ms': finite(failure.latency_ms, 1), 'cause': failure.cause, 'error': str(failure.error)[:300]}


def responsiveness_verdict(samples, failures, start, end, thread_alive=False):
    """The full 'responsive while removing' judgement over the discard window `[start, end]`: the three
    latency conditions of `ceiling_verdict` plus (BL112.12, CodeRabbit 4499357511) that NO request sent in
    the window failed and that the probe thread did not outlive its stop. A failure is judged by when it was
    SENT, like a late answer, and never by an answer's state. Failures outside the window are counted and
    reported, not judged."""
    verdict = ceiling_verdict(latencies_sent_within(samples, start, end))
    inside = failures_sent_within(failures, start, end)
    verdict.update({
        'no_failed_requests': not inside,
        'probe_stopped': not thread_alive,
        'failed_in_window': [failure_record(failure, start) for failure in inside],
        'failed_outside_window': len(failures) - len(inside),
    })
    verdict['ok'] = (verdict['enough_samples'] and verdict['p99_ok'] and verdict['max_ok']
                     and verdict['no_failed_requests'] and verdict['probe_stopped'])
    return verdict


def probe_report(probe, origin):
    """Everything the probe saw that is not a latency: every failed request in the order it happened,
    the counts, and whether its thread is still alive. JSON-safe by construction."""
    failures = probe.failures
    causes = collections.Counter(failure.cause for failure in failures)
    # an answer with `ok: false` is both a Sample and a 'rejected' Failure: one request, counted once
    return {'requests': len(probe.requests) - causes.get('rejected', 0), 'answered': len(probe.samples), 'failed': len(failures),
            'by_cause': dict(causes), 'thread_alive_after_stop': bool(probe.thread_alive),
            'failures': [failure_record(failure, origin) for failure in failures]}


def ceiling_verdict(latencies):
    """The three conditions of the 'responsive while removing' ceiling, separately, so a control can show
    that the latency ones fail on a stalled window without needing the sample count to be reached."""
    return {
        'enough_samples': len(latencies) >= MIN_SAMPLES_DURING_DISCARD,
        'p99_ok': bool(latencies) and percentile(latencies, 0.99) <= DISCARD_P99_MS,
        'max_ok': bool(latencies) and max(latencies) <= DISCARD_MAX_MS,
        'stats': stats(latencies),
    }


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

    def __enter__(self):
        return self

    def __exit__(self, *failure):
        """Whatever ended the session (a check, an assert, a request that raised), THIS child does not
        outlive it: stdin is closed so it discards what is pending and exits, and only a child that
        does not exit in time is killed. Nothing else is touched, and it all happens before the caller
        removes the workspace the child owns (BL112.12)."""
        if self.process.poll() is None:
            with contextlib.suppress(OSError):
                self.process.stdin.close()
            try:
                self.process.wait(timeout=CHILD_EXIT_S)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        for stream in (self.process.stdin, self.process.stdout):
            with contextlib.suppress(OSError):
                stream.close()


def scene_files(workspace):
    root = Path(workspace) / 'scenes/street/assets'
    return [root / name for name in ('sky.png', 'mid.png', 'hero.png')]


def jsonl_phase(binary, work, report, pad_mib):
    """Every session is an entry of one ExitStack: however this function ends, each child it started
    has been given EOF and has exited (or been killed) before the caller removes `work`."""
    with contextlib.ExitStack() as sessions:
        return jsonl_checks(binary, work, report, pad_mib, sessions)


def jsonl_checks(binary, work, report, pad_mib, sessions):
    check = report.check
    results = {}

    def session(name):
        workspace = Path(work) / f'jsonl-{name}'
        proc = sessions.enter_context(Jsonl(binary))
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
    """A second client that keeps asking the window for its state and times every request.

    EVERY request is recorded, in the order it completed, with the instants it was sent and ended: an
    answer as a `Sample`, anything else as a `Failure` with its cause (BL112.12). Nothing is swallowed:
    a timeout, a network error, a reply that is not JSON, or any other exception becomes a `Failure`, and
    the probe keeps asking afterwards (so one failure cannot silence the rest of the measurement). An
    answer that says `ok: false` is both: a `Sample` (it was timed) and a `Failure` of cause 'rejected'."""

    def __init__(self, lab):
        self.lab = lab
        self.requests = []  # Sample | Failure, in completion order (one thread: the order the window handled them)
        self.in_flight = None  # `sent` of the request being waited for
        self.stop = threading.Event()
        self.thread = None
        self.ended = False
        self.thread_alive = False

    @property
    def samples(self):
        return [request for request in self.requests if isinstance(request, Sample)]

    @property
    def failures(self):
        return [request for request in self.requests if isinstance(request, Failure)]

    def start(self):
        self.stop.clear()
        self.ended = False
        self.thread_alive = False
        self.thread = threading.Thread(target=self._run, daemon=True)
        self.thread.start()

    @staticmethod
    def cause_of(error):
        if isinstance(error, (TimeoutError, socket.timeout)):
            return 'timeout'
        if isinstance(error, OSError):
            return 'network'
        if isinstance(error, ValueError):  # JSONDecodeError: the window closed the connection without a line
            return 'protocol'
        return 'unexpected'

    def _record(self, request):
        if not self.ended:  # a straggler that finishes after the verdict was drawn is not part of it
            self.requests.append(request)

    def _run(self):
        while not self.stop.is_set():
            sent = time.monotonic()
            started = time.perf_counter()
            self.in_flight = sent
            try:
                answer = self.lab.send({'version': 1, 'command': 'get', 'target': 'jobs'}, timeout=120)
                latency = (time.perf_counter() - started) * 1000
                received = time.monotonic()
                state = answer.get('data', {}).get('state') if answer.get('ok') else None
                self._record(Sample(sent, received, latency, state))
                if not answer.get('ok'):
                    self._record(Failure(sent, received, latency, 'rejected', answer.get('error')))
            except Exception as error:  # every one is data: a thread that dies silently hides the failure
                self._record(Failure(sent, time.monotonic(), (time.perf_counter() - started) * 1000,
                                     self.cause_of(error), repr(error)))
            finally:
                self.in_flight = None
            self.stop.wait(PROBE_GAP_S)

    def end(self):
        """Stops the thread and returns the answers. A thread that is still alive afterwards is NOT hidden: it
        is reported (`thread_alive`) and the request it is stuck on is recorded as an 'unfinished' failure."""
        if self.ended:
            return self.samples
        self.stop.set()
        if self.thread is not None:
            self.thread.join(PROBE_JOIN_S)
            self.thread_alive = self.thread.is_alive()
        stuck = self.in_flight
        self.ended = True
        if self.thread_alive and stuck is not None:
            self.requests.append(Failure(stuck, None, None, 'unfinished',
                                         f'no answer {time.monotonic() - stuck:.1f} s after the request was sent'))
        return self.samples


def judge_discard(report, results, probe, job):
    """Judges the probe of a discard, shared by the window phase and by the spec: every check of the
    'responsive while removing' verdict, with every request the probe made (answers AND failures) recorded
    in `results` BEFORE any check can raise. A request is 'during the discard' by when it was SENT (see
    `discard_window`), not by the state its answer carries."""
    check = report.check
    samples = probe.end()
    failures = probe.failures
    origin = samples[0].sent if samples else (failures[0].sent if failures else 0.0)
    window = discard_window(samples, job, failures)
    results['probe'] = probe_report(probe, origin)
    check('the probe thread stopped (a thread still waiting is a finding, not a pass)', not probe.thread_alive,
          results['probe'])
    check('the probe saw the discard begin (a window of instants can be drawn)', window is not None,
          [(round(sample.latency_ms, 1), result_state_of(sample, job)) for sample in samples[-10:]])
    start, end = window
    verdict = responsiveness_verdict(samples, failures, start, end, probe.thread_alive)
    during = latencies_sent_within(samples, start, end)
    by_label = [sample.latency_ms for sample in samples if result_state_of(sample, job) == 'discarding']
    results['during_discard'] = stats(during)
    # The previous measurement (answers labelled 'discarding'), kept next to the new one so the
    # difference between them is data, not a claim.
    results['during_discard_by_answer_label'] = stats(by_label)
    results['during_discard_answered_after_the_end'] = sum(
        1 for sample in samples if start <= sample.sent <= end and result_state_of(sample, job) == 'discarded')
    results['discard_window_seconds'] = round(end - start, 2)
    results['discard_window_basis'] = 'send/receive instants (time.monotonic), not the answer label'
    results['probe'] = probe_report(probe, start)
    results['discard_verdict'] = {key: value for key, value in verdict.items() if key != 'stats'}
    check(f'the probe sampled the window while it removed (>= {MIN_SAMPLES_DURING_DISCARD} answers)',
          verdict['enough_samples'], results['during_discard'])
    check(f'responsive while removing: p99 <= {DISCARD_P99_MS} ms and max <= {DISCARD_MAX_MS} ms',
          verdict['p99_ok'] and verdict['max_ok'], results['during_discard'])
    check('no request sent while it removed failed (timeout, network, protocol, rejected or unfinished)',
          verdict['no_failed_requests'], verdict['failed_in_window'])
    return verdict


def window_phase(binary, work, report, pad_mib):
    check = report.check
    results = {}
    home = tempfile.mkdtemp(prefix='te-s1-', dir='/tmp')
    os.environ['TESSEL_VIEWPORT_INSPECT'] = '1'
    lab = video.Editor(str(Path(binary).resolve()), home)
    probe = None

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

        # The same suspension again, now seen by the probe and judged by the SAME functions that judge
        # the discard (`latencies_sent_within` and `ceiling_verdict`, with the same ceilings), over the
        # span we know exactly: from the SIGSTOP to the SIGCONT. A probe or a classifier that could
        # not see a stalled window would let this pass; it must fail the latency ceilings.
        control_probe = Probe(lab)
        control_probe.start()
        try:
            wait_for(lambda: len(control_probe.samples) >= 3, 30, 'the control probe samples a healthy window')
            # A request already in flight when the signal lands is stalled too: the span starts a
            # little before the SIGSTOP so it is counted (a fast request in that margin is harmless).
            stopped = time.monotonic() - 0.05
            os.kill(lab.process.pid, signal.SIGSTOP)
            time.sleep(1.5)
            os.kill(lab.process.pid, signal.SIGCONT)
            resumed = time.monotonic()
            wait_for(lambda: any(sample.latency_ms >= CONTROL_MIN_STALL_MS for sample in control_probe.samples),
                     30, 'the stalled request is answered')
        finally:
            control_samples = control_probe.end()
        verdict = ceiling_verdict(latencies_sent_within(control_samples, stopped, resumed))
        results['control_classifier'] = verdict
        check('control: the classifier that judges the discard fails a suspended window on p99 and on max',
              verdict['stats']['n'] >= 1 and not verdict['p99_ok'] and not verdict['max_ok'], verdict)

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
        probe.end()
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

        judge_discard(report, results, probe, job)

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
            try:
                if probe is not None:
                    probe.end()  # a failed check must not leave it asking a window that is going away
                    # ...and what it saw is reported even when an earlier check raised before the verdict.
                    first = probe.requests[0].sent if probe.requests else 0.0
                    results.setdefault('probe', probe_report(probe, first))
            finally:
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
