"""Pure-logic tests of the job.discard proof (BL094/BL112, BL112.12); no editor, GPU, Chrome or Cargo build is started.

They drive the real classifier and the real JSONL phase of `job-discard-proof.py`:

* a request sent while the window removes files but answered after the discard ended comes back
  labelled 'discarded'; the discard measurement must still count it (BL112.12), and the previous
  filter (label == 'discarding') must be shown to pass the very same samples, so these tests
  discriminate it;
* the SIGSTOP control goes through the same functions and the same ceilings and must fail them;
* a JSONL session that is interrupted by a failed check leaves no child alive and its workspace
  untouched afterwards (BL112.12);
* the REAL `Probe` is driven against a scripted lab whose `send` raises: a request SENT inside the
  discard window that fails (OSError, a reply that is not JSON, ok:false, a request that never ends) must
  FAIL `judge_discard`, the function the window phase calls; one outside the window is reported but not
  judged. The same scenarios are run against source-level mutants (record removed, guard removed, thread
  death on ValueError, window end ignoring failures, thread-alive ignored): each must stop failing.
"""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import socket
import stat
import tempfile
import threading
import time
import unittest

spec = importlib.util.spec_from_file_location('job_discard', Path(__file__).with_name('job-discard-proof.py'))
proof = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proof)

JOB = 'job-1'


def jobs_payload(result_state):
    return {'all': [{'id': JOB, 'result_state': result_state}]}


def sample(sent, latency_ms, label):
    """One answer: the request left at `sent` (s) and the window took `latency_ms` to answer, labelling the
    job with `label` as of the moment it replied."""
    return proof.Sample(sent, sent + latency_ms / 1000, latency_ms, jobs_payload(label) if label else None)


def healthy_discard(slow_after_the_end=None):
    """10 idle answers, 30 answers during a discard that lasts 0.6 s, then answers after it. With
    `slow_after_the_end` set, one request sent in the middle of the discard is delayed that many ms
    (a stalled UI thread), so it is answered after the discard ended: labelled 'discarded'."""
    samples = [sample(t * 0.02, 2.0, 'pending') for t in range(10)]
    stalled_sent = None
    for index in range(30):
        sent = 0.2 + index * 0.02
        if slow_after_the_end and index == 15:
            stalled_sent = sent
            continue  # the probe thread is busy waiting for this answer
        samples.append(sample(sent, 3.0, 'discarding'))
    if slow_after_the_end:
        samples.append(sample(stalled_sent, slow_after_the_end, 'discarded'))
        # Requests the probe could only send after the slow one came back.
        resume = stalled_sent + slow_after_the_end / 1000
    else:
        resume = 0.2 + 30 * 0.02
    samples += [sample(resume + 0.02 * n, 2.0, 'discarded') for n in range(1, 6)]
    return samples


def old_filter_during(samples):
    """What the proof measured before this fix: the answers labelled 'discarding'."""
    return [s.latency_ms for s in samples if proof.result_state_of(s, JOB) == 'discarding']


def new_during(samples):
    start, end = proof.discard_window(samples, JOB)
    return proof.latencies_sent_within(samples, start, end)


class DiscardWindowTests(unittest.TestCase):
    def test_a_healthy_discard_is_judged_over_every_request_sent_during_it(self):
        samples = healthy_discard()
        during = new_during(samples)
        self.assertGreaterEqual(len(during), proof.MIN_SAMPLES_DURING_DISCARD)
        verdict = proof.ceiling_verdict(during)
        self.assertTrue(verdict['enough_samples'] and verdict['p99_ok'] and verdict['max_ok'], verdict)

    def test_a_slow_answer_that_crosses_the_end_of_the_discard_fails_the_ceiling(self):
        samples = healthy_discard(slow_after_the_end=800)
        slow = [s for s in samples if s.latency_ms == 800][0]
        self.assertEqual(proof.result_state_of(slow, JOB), 'discarded', 'labelled after the end')
        during = new_during(samples)
        self.assertIn(800, during, 'counted although its answer says discarded')
        verdict = proof.ceiling_verdict(during)
        self.assertFalse(verdict['max_ok'], verdict)
        self.assertFalse(verdict['p99_ok'] and verdict['max_ok'])

    def test_the_old_label_filter_passes_the_same_stalled_samples_so_the_test_above_discriminates_it(self):
        samples = healthy_discard(slow_after_the_end=800)
        old = proof.ceiling_verdict(old_filter_during(samples))
        self.assertTrue(old['enough_samples'] and old['p99_ok'] and old['max_ok'],
                        f'the mutation (label filter) is a false PASS on this input: {old}')
        self.assertNotIn(800, old_filter_during(samples))

    def test_a_stall_that_covers_the_whole_discard_is_still_found(self):
        # Pending, then nothing for 1.4 s, then the first answer already says 'discarded'.
        samples = [sample(t * 0.02, 2.0, 'pending') for t in range(25)]
        samples.append(sample(0.5, 1400.0, 'discarded'))
        samples += [sample(2.0 + 0.02 * n, 2.0, 'discarded') for n in range(5)]
        self.assertEqual(old_filter_during(samples), [], 'the label filter sees nothing at all')
        self.assertIn(1400.0, new_during(samples))
        self.assertFalse(proof.ceiling_verdict(new_during(samples))['max_ok'])

    def test_no_answer_that_shows_the_discard_means_no_window(self):
        self.assertIsNone(proof.discard_window([sample(0.0, 2.0, 'pending')] * 5, JOB))
        self.assertIsNone(proof.discard_window([], JOB))

    def test_requests_before_the_discard_and_after_it_are_not_counted(self):
        samples = healthy_discard()
        start, end = proof.discard_window(samples, JOB)
        during = proof.latencies_sent_within(samples, start, end)
        self.assertLess(len(during), len(samples))
        self.assertLessEqual(len(during), 32, 'idle and later answers stay out')

    def test_the_suspended_window_control_fails_through_the_same_classifier_and_ceilings(self):
        # What the control records: a few healthy answers, then one request stalled by the SIGSTOP.
        samples = [sample(t * 0.02, 2.0, None) for t in range(5)]
        stopped, resumed = 0.15, 1.65
        samples.append(sample(0.16, 1480.0, None))
        verdict = proof.ceiling_verdict(proof.latencies_sent_within(samples, stopped, resumed))
        self.assertGreaterEqual(verdict['stats']['n'], 1)
        self.assertFalse(verdict['p99_ok'])
        self.assertFalse(verdict['max_ok'])

    def test_a_probe_blind_to_the_stall_would_not_fail_the_control(self):
        # The same suspension, but the stalled request was never recorded: an empty window must not
        # pass for a verdict. `ceiling_verdict([])` fails both latency conditions and the proof also
        # requires at least one sample in the control span.
        verdict = proof.ceiling_verdict([])
        self.assertFalse(verdict['p99_ok'] or verdict['max_ok'] or verdict['enough_samples'])

    def test_ceilings_are_the_ones_fixed_before_the_first_measurement(self):
        self.assertEqual((proof.IDLE_P99_MS, proof.DISCARD_P99_MS, proof.DISCARD_MAX_MS), (100.0, 100.0, 250.0))
        self.assertEqual(proof.MIN_SAMPLES_DURING_DISCARD, 20)


FAKE_MEDIA = '''#!/usr/bin/env python3
import json, os, sys
pid_file, eof_file = os.environ['FAKE_PID_FILE'], os.environ['FAKE_EOF_FILE']
open(pid_file, 'a').write(str(os.getpid()) + '\\n')
for line in sys.stdin:
    request = json.loads(line)
    print(json.dumps({'ok': True, 'result': {'job': 'job-1', 'state': 'succeeded', 'result_state': 'pending'}}), flush=True)
open(eof_file, 'w').write('eof')  # reached only when stdin ended: the cleanup of a session that is over
'''


def alive(pid):
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    return True


class JsonlSessionCleanupTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.mkdtemp(prefix='te-discard-spec-')
        self.binary = Path(self.directory) / 'fake-media'
        self.binary.write_text(FAKE_MEDIA)
        self.binary.chmod(self.binary.stat().st_mode | stat.S_IEXEC)
        self.pids = Path(self.directory) / 'pids'
        self.eof = Path(self.directory) / 'eof'
        os.environ['FAKE_PID_FILE'], os.environ['FAKE_EOF_FILE'] = str(self.pids), str(self.eof)
        self.work = Path(self.directory) / 'work'
        self.work.mkdir()

    def tearDown(self):
        for pid in self.pids.read_text().split() if self.pids.exists() else []:
            if alive(int(pid)):
                os.kill(int(pid), 9)
        shutil.rmtree(self.directory, ignore_errors=True)

    def children(self):
        return [int(pid) for pid in self.pids.read_text().split()] if self.pids.exists() else []

    def test_a_check_that_fails_mid_session_leaves_no_child_and_the_workspace_untouched(self):
        report = proof.Report()
        # The fake never copies files, so the first check of the phase fails with the real
        # `Report.check` (an AssertionError) while the session is open: the case of the finding.
        with self.assertRaises(AssertionError):
            proof.jsonl_phase(str(self.binary), str(self.work), report, 1)
        pids = self.children()
        self.assertEqual(len(pids), 1, 'one session was open when the check failed')
        # Settled when jsonl_phase returned: no child is alive, with no waiting for it afterwards.
        self.assertFalse(alive(pids[0]), 'the child outlived the phase that owned it')
        self.assertTrue(self.eof.exists(), 'it ended by EOF on stdin (its own cleanup), not by a kill')
        before = sorted(str(path) for path in self.work.rglob('*'))
        time.sleep(0.3)
        self.assertEqual(sorted(str(path) for path in self.work.rglob('*')), before,
                         'nothing writes into the workspace after the phase ended')
        shutil.rmtree(self.work)  # what main() does next: nothing races it
        self.assertFalse(self.work.exists())

    def test_a_request_that_raises_closes_its_session_too(self):
        with self.assertRaises(AssertionError):
            with proof.Jsonl(str(self.binary)) as session:
                session.ok({'op': 'job.get', 'job': 'x'})
                raise AssertionError('forced failure in the middle of a session')
        self.assertFalse(alive(self.children()[0]))
        self.assertTrue(self.eof.exists())

    def test_without_the_context_the_child_outlives_the_failure(self):
        # The control: this is what the old phase did, so the tests above can see a leak.
        session = proof.Jsonl(str(self.binary))
        try:
            session.ok({'op': 'job.get', 'job': 'x'})
            self.assertTrue(alive(self.children()[0]), 'a bare session stays alive on its own')
        finally:
            session.__exit__(None, None, None)
        self.assertFalse(alive(self.children()[0]))

    def test_a_child_that_ignores_eof_is_killed_not_waited_for_forever(self):
        stubborn = Path(self.directory) / 'stubborn'
        stubborn.write_text('#!/usr/bin/env python3\nimport os, time\n'
                            "open(os.environ['FAKE_PID_FILE'], 'a').write(str(os.getpid()) + '\\n')\n"
                            'time.sleep(600)\n')
        stubborn.chmod(stubborn.stat().st_mode | stat.S_IEXEC)
        saved = proof.CHILD_EXIT_S
        proof.CHILD_EXIT_S = 0.5
        try:
            with proof.Jsonl(str(stubborn)):
                # Deterministic readiness: leave the session only once the child has started (it wrote its
                # PID), whatever the load; a fixed sleep raced the interpreter start-up (BL112.12, N2).
                deadline = time.monotonic() + 60
                while not self.pids.exists() and time.monotonic() < deadline:
                    time.sleep(0.01)
                self.assertTrue(self.pids.exists(), 'the child never started within 60 s')
        finally:
            proof.CHILD_EXIT_S = saved
        self.assertFalse(alive(self.children()[0]))



SOURCE = Path(__file__).with_name('job-discard-proof.py')


def load_proof(mutation=None):
    """A fresh instance of the proof module, optionally from mutated source. The mutation must really
    change the text: a mutant that equals the original would prove nothing."""
    text = SOURCE.read_text()
    if mutation is not None:
        old, new = mutation
        if text.count(old) != 1:
            raise AssertionError(f'the mutation target is not unique/present: {old[:60]!r}')
        text = text.replace(old, new)
    module = type(proof)('job_discard_mutant')
    module.__file__ = str(SOURCE)
    exec(compile(text, str(SOURCE), 'exec'), module.__dict__)
    # The latency ceilings are about the window, not this machine: scheduler noise under load must not decide
    # a scenario about failed requests. The fixed ceilings themselves are asserted on a pristine module.
    module.DISCARD_P99_MS = module.DISCARD_MAX_MS = 1e9
    module.PROBE_GAP_S = 0.001
    module.PROBE_JOIN_S = 0.3
    return module


@contextlib.contextmanager
def mute_thread_errors():
    """A mutant whose probe thread dies prints a traceback from the thread; that is the point of the mutant."""
    saved = threading.excepthook
    threading.excepthook = lambda args: None
    try:
        yield
    finally:
        threading.excepthook = saved


class ScriptedLab:
    """`send` plays a script. A label answers with that `result_state`; an exception instance is raised; `'ok:false'`
    answers a refusal; an Event blocks until it is set. After the script `tail` repeats and `done` is set."""

    def __init__(self, script, tail='discarded'):
        self.script, self.tail, self.calls, self.done = list(script), tail, 0, threading.Event()

    def send(self, request, timeout):
        index, self.calls = self.calls, self.calls + 1
        if index >= len(self.script):
            self.done.set()
        step = self.script[index] if index < len(self.script) else self.tail
        if isinstance(step, BaseException):
            raise step
        if isinstance(step, threading.Event):
            self.done.set()  # the probe is now stuck on this request: the scenario is complete
            step.wait(30)
            raise OSError('released')
        if step == 'ok:false':
            return {'ok': False, 'error': {'code': 'refused'}}
        return {'ok': True, 'data': {'state': jobs_payload(step)}}


def run_scenario(module, script, tail='discarded', wait=5):
    """Runs the REAL `Probe` against the script and judges it with the REAL `judge_discard`, as the window phase
    does. Returns (raised check name or None, results, report, probe)."""
    lab = ScriptedLab(script, tail)
    probe = module.Probe(lab)
    report, results = module.Report(), {}
    probe.start()
    lab.done.wait(wait)
    with contextlib.redirect_stdout(io.StringIO()):
        try:
            module.judge_discard(report, results, probe, JOB)
            failed = None
        except AssertionError as error:
            failed = str(error)
    return failed, results, report, probe


IN_WINDOW_FAILURE = ['pending'] * 3 + ['discarding'] * 12 + [OSError('connection reset by peer')] + ['discarding'] * 13 + ['discarded'] * 2
NO_FAILURE = ['pending'] * 3 + ['discarding'] * 25 + ['discarded'] * 2
FAILED_CHECK = 'no request sent while it removed failed'
TAIL_FAILURES = ['pending'] * 3 + ['discarding'] * 25 + [OSError('reset')]


class ProbeFailureTests(unittest.TestCase):
    def assert_fails_on_failed_request(self, module, script, **kwargs):
        failed, results, report, probe = run_scenario(module, script, **kwargs)
        self.assertIsNotNone(failed, 'the verdict passed although a request sent in the window failed')
        self.assertTrue(failed.startswith(FAILED_CHECK), f'failed on another check: {failed}')
        return results, report, probe

    def test_a_healthy_run_passes(self):
        failed, results, report, probe = run_scenario(load_proof(), NO_FAILURE)
        self.assertIsNone(failed, report.data['checks'][-3:])
        self.assertGreaterEqual(results['during_discard']['n'], 20)
        self.assertEqual(results['probe']['failed'], 0)
        self.assertTrue(results['discard_verdict']['ok'])

    def test_twenty_fast_answers_and_one_oserror_in_the_window_fail(self):
        results, report, probe = self.assert_fails_on_failed_request(load_proof(), IN_WINDOW_FAILURE)
        self.assertGreaterEqual(results['during_discard']['n'], 20, 'the 20 fast answers are there: they must not be enough')
        verdict = results['discard_verdict']
        self.assertTrue(verdict['enough_samples'] and verdict['p99_ok'] and verdict['max_ok'])
        self.assertFalse(verdict['no_failed_requests'])
        (record,) = results['probe']['failures']
        self.assertEqual(record['cause'], 'network')
        self.assertIn('connection reset', record['error'])
        self.assertLessEqual(record['sent_s'], record['failed_s'], 'send and failure instants are recorded')
        self.assertEqual(results['probe']['by_cause'], {'network': 1})
        self.assertEqual(results['probe']['failed'], 1)
        self.assertEqual(results['discard_verdict']['failed_in_window'], results['probe']['failures'])
        json.dumps(results, allow_nan=False)  # finite JSON: no NaN, no infinity

    def test_the_probe_keeps_asking_after_a_failure(self):
        results, report, probe = self.assert_fails_on_failed_request(load_proof(), IN_WINDOW_FAILURE)
        failure = probe.failures[0]
        self.assertTrue(any(sample.sent > failure.failed for sample in probe.samples))

    def test_a_reply_that_is_not_json_is_recorded_and_does_not_kill_the_probe(self):
        script = IN_WINDOW_FAILURE.copy()
        script[15] = json.JSONDecodeError('Expecting value', '', 0)
        results, _, probe = self.assert_fails_on_failed_request(load_proof(), script)
        self.assertEqual(results['probe']['by_cause'], {'protocol': 1})
        self.assertGreater(results['probe']['answered'], 15, 'the thread survived the ValueError')

    def test_a_timeout_and_other_errors_have_their_own_cause(self):
        proof_module = load_proof()
        self.assertEqual(proof_module.Probe.cause_of(socket.timeout('timed out')), 'timeout')
        self.assertEqual(proof_module.Probe.cause_of(TimeoutError()), 'timeout')
        self.assertEqual(proof_module.Probe.cause_of(ConnectionResetError()), 'network')
        self.assertEqual(proof_module.Probe.cause_of(json.JSONDecodeError('x', '', 0)), 'protocol')
        self.assertEqual(proof_module.Probe.cause_of(KeyError('x')), 'unexpected')
        script = IN_WINDOW_FAILURE.copy()
        script[15] = socket.timeout('timed out')
        results, _, _ = self.assert_fails_on_failed_request(load_proof(), script)
        self.assertEqual(results['probe']['by_cause'], {'timeout': 1})

    def test_an_answer_that_says_not_ok_is_a_failed_request(self):
        script = IN_WINDOW_FAILURE.copy()
        script[15] = 'ok:false'
        results, _, _ = self.assert_fails_on_failed_request(load_proof(), script)
        self.assertEqual(results['probe']['by_cause'], {'rejected': 1})
        self.assertEqual(results['probe']['failed'], 1)

    def test_failures_outside_the_window_are_reported_not_judged(self):
        before = ['pending', 'pending', OSError('before'), 'pending'] + ['discarding'] * 25
        for script in (before + ['discarded'] * 2,
                       NO_FAILURE + [OSError('after the end')] + ['discarded'] * 2):
            failed, results, report, probe = run_scenario(load_proof(), script)
            self.assertIsNone(failed, report.data['checks'][-3:])
            self.assertEqual(results['discard_verdict']['failed_outside_window'], 1)
            self.assertEqual(results['discard_verdict']['failed_in_window'], [])
            self.assertEqual(results['probe']['failed'], 1, 'it is still recorded')
            json.dumps(results, allow_nan=False)

    def test_a_failure_after_the_last_answer_with_no_discarded_answer_is_in_the_window(self):
        # The end of the discard is unknown (nothing ever said 'discarded'): the span reaches the last instant.
        self.assert_fails_on_failed_request(load_proof(), TAIL_FAILURES, tail=OSError('reset'))

    def test_a_request_that_never_ends_fails_and_the_thread_is_reported_alive(self):
        release = threading.Event()
        try:
            failed, results, report, probe = run_scenario(load_proof(), ['pending'] * 3 + ['discarding'] * 25 + [release])
            self.assertIsNotNone(failed)
            self.assertTrue(failed.startswith('the probe thread stopped'), failed)
            self.assertTrue(results['probe']['thread_alive_after_stop'])
            self.assertEqual(results['probe']['by_cause'], {'unfinished': 1})
            self.assertIsNone(results['probe']['failures'][0]['failed_s'], 'no failure instant for a request that never ended')
            json.dumps(results, allow_nan=False)
        finally:
            release.set()

    def test_a_straggler_after_the_verdict_changes_nothing(self):
        release = threading.Event()
        try:
            _, results, _, probe = run_scenario(load_proof(), ['pending'] * 3 + ['discarding'] * 25 + [release])
            count = len(probe.requests)
            release.set()
            time.sleep(0.2)
            self.assertEqual(len(probe.requests), count)
        finally:
            release.set()

    def test_the_fixed_ceilings_are_untouched_by_the_scenarios(self):
        fresh = type(proof)('pristine')
        fresh.__file__ = str(SOURCE)
        exec(compile(SOURCE.read_text(), str(SOURCE), 'exec'), fresh.__dict__)
        self.assertEqual((fresh.IDLE_P99_MS, fresh.DISCARD_P99_MS, fresh.DISCARD_MAX_MS), (100.0, 100.0, 250.0))
        self.assertEqual(fresh.MIN_SAMPLES_DURING_DISCARD, 20)


MUTANTS = {
    # the failure is no longer recorded (the old `except OSError: continue`, as far as the verdict can see)
    'record_removed': ("""                self._record(Failure(sent, time.monotonic(), (time.perf_counter() - started) * 1000,
                                     self.cause_of(error), repr(error)))""", '                pass'),
    # the verdict no longer fails on a failed request
    'guard_removed': ("""          verdict['no_failed_requests'], verdict['failed_in_window'])""", "          True, None)"),
    # a ValueError ends the probe thread without a word
    'thread_dies_on_valueerror': ('            except Exception as error:', '            except OSError as error:'),
}


class ProbeMutationTests(unittest.TestCase):
    """Each mutant must stop failing the scenario it is meant to fail: the spec sees the removal."""

    def test_removing_the_record_or_the_guard_is_detected(self):
        for name, mutation in MUTANTS.items():
            script = IN_WINDOW_FAILURE.copy()
            if name == 'thread_dies_on_valueerror':
                script[15] = ValueError('not json')
            with self.subTest(mutant=name), mute_thread_errors():
                failed, *_ = run_scenario(load_proof(mutation), script, wait=1.0)
                self.assertFalse(failed is not None and failed.startswith(FAILED_CHECK),
                                 f'the mutant {name} is still caught by the failed-request check: the scenario cannot see it')
                # ...while the same script fails on that check with the real code.
                real, *_ = run_scenario(load_proof(), script)
                self.assertTrue(real is not None and real.startswith(FAILED_CHECK), real)

    def test_a_window_end_that_ignores_failures_is_detected(self):
        mutation = ("    return start, max([samples[-1].received] + [f.failed if f.failed is not None else f.sent for f in failures])",
                    "    return start, samples[-1].received")
        failed, *_ = run_scenario(load_proof(mutation), TAIL_FAILURES, tail=OSError('reset'))
        self.assertFalse(failed is not None and failed.startswith(FAILED_CHECK))

    def test_ignoring_a_thread_that_is_still_alive_is_detected(self):
        release = threading.Event()
        mutation = ('            self.thread_alive = self.thread.is_alive()', '            self.thread_alive = False')
        try:
            failed, *_ = run_scenario(load_proof(mutation), ['pending'] * 3 + ['discarding'] * 25 + [release])
            self.assertIsNone(failed, 'the mutant passes: a probe stuck on a request is invisible to it')
            real, *_ = run_scenario(load_proof(), ['pending'] * 3 + ['discarding'] * 25 + [release])
            self.assertIsNotNone(real)
        finally:
            release.set()


if __name__ == '__main__':
    unittest.main()
