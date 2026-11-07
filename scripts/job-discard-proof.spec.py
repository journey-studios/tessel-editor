"""Pure-logic tests of the job.discard proof (BL094/BL112, BL112.12); no editor, GPU, Chrome or Cargo build is started.

They drive the real classifier and the real JSONL phase of `job-discard-proof.py`:

* a request sent while the window removes files but answered after the discard ended comes back
  labelled 'discarded'; the discard measurement must still count it (BL112.12), and the previous
  filter (label == 'discarding') must be shown to pass the very same samples, so these tests
  discriminate it;
* the SIGSTOP control goes through the same functions and the same ceilings and must fail them;
* a JSONL session that is interrupted by a failed check leaves no child alive and its workspace
  untouched afterwards (BL112.12).
"""
import importlib.util
import os
from pathlib import Path
import shutil
import stat
import tempfile
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
                time.sleep(0.2)
        finally:
            proof.CHILD_EXIT_S = saved
        self.assertFalse(alive(self.children()[0]))


if __name__ == '__main__':
    unittest.main()
