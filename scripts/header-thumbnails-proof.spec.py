"""Pure-logic tests of the BL144 proof (F053): doubles only, no editor, window, GPU or renderer is started.

    python3 scripts/header-thumbnails-proof.spec.py

What is proved: the oracle (pinhole, PNG reader, ball of splats), the in-flight/cancelled observation rules
(a job that finished before it could be cancelled is a FAIL, not a pass), the OS-capture rule (a refusal is a
SKIP, never a pass), and that one surface that cannot start, dies, hangs or answers garbage is a recorded
failure of that surface only: the other one still runs and the exit code says so.
"""
import importlib.util
import io
import struct
import sys
import tempfile
import unittest
import zlib
from contextlib import redirect_stdout
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
spec = importlib.util.spec_from_file_location('header_thumbnails_proof', HERE / 'header-thumbnails-proof.py')
proof = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proof)
helper = proof  # the helper names the proof imported are the shared ones


def png(width, height, pixel, filter_type=0, noise=False):
    row = bytes([filter_type]) + bytes(pixel) * width
    raw = row * height
    if noise:
        import os
        raw = b''.join(bytes([filter_type]) + os.urandom(3 * width) for _ in range(height))

    def chunk(kind, data):
        body = kind + data
        return struct.pack('>I', len(data)) + body + struct.pack('>I', zlib.crc32(body))

    return (proof.PNG_SIGNATURE + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(raw)) + chunk(b'IEND', b''))


class Recording:
    """A stand-in for `Recorder` with the same surface."""

    def __init__(self):
        self.skips, self.checks = [], []

    def __call__(self, name, condition, detail=''):
        self.checks.append((name, bool(condition)))
        if not condition:
            raise proof.ProofFailure(name)

    def skip(self, name, reason):
        self.skips.append((name, reason))


class RendererOwnershipTests(unittest.TestCase):
    """BL144.10: how the proof reads a busy renderer and retries (no renderer started)."""

    def busy(self, **details):
        base = {'retryable': True, 'queued': False}
        base.update(details)
        return {'code': 'renderer_busy', 'message': 'busy', 'details': base}

    def test_a_typed_retryable_refusal_that_queues_nothing_is_accepted(self):
        check = Recording()
        details = proof.require_busy_refusal(check, 'x', False, self.busy(stopping=True, reserved_for_export=True),
                                             stopping=True, reserved=True)
        self.assertTrue(details['stopping'])

    def test_a_request_that_was_accepted_is_not_a_refusal(self):
        with self.assertRaises(proof.ProofFailure):
            proof.require_busy_refusal(Recording(), 'x', True, {'job': 'job-1'})

    def test_a_refusal_that_promises_a_queue_or_is_not_retryable_fails(self):
        for details in ({'retryable': True, 'queued': True}, {'retryable': False, 'queued': False}, {}):
            with self.assertRaises(proof.ProofFailure, msg=str(details)):
                proof.require_busy_refusal(Recording(), 'x', False,
                                           {'code': 'renderer_busy', 'details': details})

    def test_the_wrong_code_fails(self):
        with self.assertRaises(proof.ProofFailure):
            proof.require_busy_refusal(Recording(), 'x', False, {'code': 'invalid_request', 'details': {}})

    def test_stopping_and_reserved_must_match_what_was_asked(self):
        with self.assertRaises(proof.ProofFailure):
            proof.require_busy_refusal(Recording(), 'x', False, self.busy(), stopping=True)
        with self.assertRaises(proof.ProofFailure):
            proof.require_busy_refusal(Recording(), 'x', False, self.busy(reserved_for_export=True), reserved=False)

    def test_the_retry_asks_again_until_the_renderer_is_free_and_counts_the_asks(self):
        answers = [(False, self.busy()), (False, self.busy()), (True, {'job': 'job-3'})]
        asked = []

        def ask(request):
            asked.append(request)
            return answers.pop(0)

        payload, attempts = proof.retry_until_granted({'op': 'export.start'}, ask, sleep=lambda _: None)
        self.assertEqual((payload, attempts, len(asked)), ({'job': 'job-3'}, 3, 3))

    def test_another_error_is_a_failure_at_once_never_retried(self):
        asked = []

        def ask(request):
            asked.append(request)
            return False, {'code': 'invalid_request', 'message': 'bad'}

        with self.assertRaises(proof.ProofFailure):
            proof.retry_until_granted({'op': 'export.start'}, ask, sleep=lambda _: None)
        self.assertEqual(len(asked), 1)

    def test_a_renderer_that_never_frees_is_a_failure_not_a_pass(self):
        clock = iter(range(0, 1000, 10))
        with self.assertRaises(proof.ProofFailure):
            proof.retry_until_granted({'op': 'export.start'}, lambda _: (False, self.busy()), seconds=30,
                                      sleep=lambda _: None, clock=lambda: next(clock))

    def test_the_export_jobs_are_found_by_kind(self):
        jobs = lambda: [{'kind': 'thumbnail', 'id': 'a'}, {'kind': 'video-export', 'id': 'b'}]
        self.assertEqual([j['id'] for j in proof.jobs_of_kind(jobs, 'video-export')], ['b'])
        self.assertEqual(proof.jobs_of_kind(lambda: [], 'video-export'), [])


class OracleTests(unittest.TestCase):
    def test_the_box_projects_left_of_the_sphere_and_the_origin_near_the_middle(self):
        box = proof.project_point((-2.2, 0.0, 0.0), 240, 136)
        sphere = proof.project_point((2.2, 0.0, 0.0), 240, 136)
        origin = proof.project_point((0.0, 0.0, 0.0), 240, 136)
        self.assertLess(box[0], origin[0])
        self.assertGreater(sphere[0], origin[0])
        self.assertAlmostEqual(box[1], sphere[1], delta=1)
        self.assertTrue(0 <= origin[0] < 240 and 0 <= origin[1] < 136)

    def test_the_png_reader_returns_the_pixels_it_was_given(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'a.png'
            path.write_bytes(png(4, 3, (10, 13, 26)))
            width, height, rows = proof.read_png(path)
            self.assertEqual((width, height), (4, 3))
            self.assertEqual(rows[2][3], (10, 13, 26))

    def test_background_is_near_and_a_cloud_colour_is_far(self):
        self.assertTrue(proof.near_background((10, 13, 26)))
        self.assertTrue(proof.near_background((15, 9, 30)))
        self.assertTrue(proof.far_from_background((230, 180, 90), 40))
        self.assertFalse(proof.far_from_background((12, 14, 27), 40))

    def test_the_ball_is_a_valid_splat_file_around_its_center_and_is_reproducible(self):
        data = proof.ball_splat((1000.0, -200.0, 300.0), 5.0, 500)
        self.assertEqual(len(data), 500 * 32)
        self.assertEqual(data, proof.ball_splat((1000.0, -200.0, 300.0), 5.0, 500))
        for index in range(500):
            x, y, z = struct.unpack_from('<fff', data, index * 32)
            self.assertLessEqual((x - 1000) ** 2 + (y + 200) ** 2 + (z - 300) ** 2, 25.001)

    def test_the_variants_are_distinct_scenes_and_the_far_splat_is_written(self):
        with tempfile.TemporaryDirectory() as folder:
            fixtures = Path(folder)
            (fixtures / 'scene.tessel-scene.json').write_text(
                '{"objects":[{"type":"box","position":[-2.2,0,0]},{"type":"sphere"}]}')
            paths = proof.write_variants(fixtures, 5)
            contents = {p.read_text() for p in paths}
            self.assertEqual(len(contents), 5)
            self.assertTrue((fixtures / 'far.splat').is_file())


class ObservationTests(unittest.TestCase):
    def job(self, state, done, total, kind='thumbnail'):
        return {'id': 'j', 'kind': kind, 'state': state, 'done': done, 'total': total}

    def test_only_a_running_job_with_frames_drawn_and_frames_left_is_in_flight(self):
        self.assertTrue(proof.in_flight(self.job('running', 1, 5)))
        self.assertFalse(proof.in_flight(self.job('running', 0, 5)), 'nothing drawn yet')
        self.assertFalse(proof.in_flight(self.job('running', 5, 5)), 'nothing left')
        self.assertFalse(proof.in_flight(self.job('queued', 2, 5)))
        self.assertFalse(proof.in_flight(self.job('succeeded', 2, 5)))

    def test_the_first_job_seen_in_flight_is_returned(self):
        script = [[self.job('queued', 0, 5)], [self.job('running', 0, 5)], [self.job('running', 2, 5)]]
        seen = proof.await_in_flight(lambda: script.pop(0), 'x', sleep=lambda _: None)
        self.assertEqual(seen['done'], 2)

    def test_a_job_that_ended_before_it_was_ever_seen_in_flight_is_a_failure_not_a_pass(self):
        script = [[self.job('queued', 0, 5)], [self.job('succeeded', 5, 5)]] + [[self.job('succeeded', 5, 5)]] * 5
        with self.assertRaises(proof.ProofFailure) as raised:
            proof.await_in_flight(lambda: script.pop(0), 'B\'s job', sleep=lambda _: None)
        self.assertIn('ended before one was seen running', str(raised.exception))

    def test_other_kinds_of_jobs_are_not_the_thumbnail_job(self):
        script = [[self.job('running', 2, 5, kind='import')]] * 3
        now = iter([0, 1, 2, 1000])
        with self.assertRaises(proof.ProofFailure) as raised:
            proof.await_in_flight(lambda: script[0], 'x', seconds=10, sleep=lambda _: None, clock=lambda: next(now))
        self.assertIn('no thumbnail job was seen running', str(raised.exception))

    def test_cancelled_midway_passes_and_anything_else_fails(self):
        check = Recording()
        proof.require_cancelled_midway(check, 'old', self.job('cancelled', 3, 9))
        for info in (self.job('succeeded', 9, 9), self.job('failed', 3, 9), self.job('cancelled', 9, 9),
                     self.job('running', 3, 9)):
            with self.assertRaises(proof.ProofFailure, msg=str(info)):
                proof.require_cancelled_midway(Recording(), 'old', info)


class FakeWindow:
    def __init__(self, result):
        self.result = result

    def capture(self, path):
        if self.result is None:
            return None, 'no on-screen window for this PID (screen locked)'
        Path(path).write_bytes(self.result)
        return str(path), ''


class CaptureTests(unittest.TestCase):
    def test_a_refused_capture_is_a_skip_and_never_a_passed_check(self):
        check = Recording()
        with tempfile.TemporaryDirectory() as folder:
            shot = proof.capture(FakeWindow(None), check, Path(folder) / 'a.png', 'the window')
        self.assertIsNone(shot)
        self.assertEqual(check.checks, [])
        self.assertEqual(len(check.skips), 1)
        self.assertIn('screen locked', check.skips[0][1])

    def test_a_capture_that_is_not_a_png_fails(self):
        with tempfile.TemporaryDirectory() as folder:
            with self.assertRaises(proof.ProofFailure):
                proof.capture(FakeWindow(b'not a png' * 400), Recording(), Path(folder) / 'a.png', 'the window')

    def test_a_png_capture_passes_and_is_returned(self):
        check = Recording()
        with tempfile.TemporaryDirectory() as folder:
            shot = proof.capture(FakeWindow(png(120, 120, (200, 10, 10), noise=True)), check, Path(folder) / 'a.png', 'w')
            self.assertTrue(shot and Path(shot).is_file())
        self.assertEqual([ok for _, ok in check.checks], [True])


class FakeSurface:
    name = 'native'
    needs_home = False

    def __init__(self, binary, home):
        self.stopped = False

    def prepare(self):
        pass

    def stop(self):
        self.stopped = True
        return []


class Dead(helper.Headless):
    """A headless that exits at once (a crashed binary): the first request meets the end of its output."""

    def __init__(self, binary, home=None, extra=()):
        super().__init__('/usr/bin/false', None, (), timeout=5)


class Hung(helper.Headless):
    def __init__(self, binary, home=None, extra=()):
        super().__init__(sys.executable, None, ('-c', 'import sys; sys.stdin.read()'), timeout=1)


class Missing(helper.Headless):
    def __init__(self, binary, home=None, extra=()):
        super().__init__('/nonexistent/tessel-render', None, ())


class Garbage(helper.Headless):
    def __init__(self, binary, home=None, extra=()):
        super().__init__(sys.executable, None, ('-c', 'print("this is not json", flush=True); input()'), timeout=5)


def probe(surface, check, scratch):
    surface.op({'op': 'status'})


class SurfaceFailureTests(unittest.TestCase):
    def run_all(self, surfaces, scenario=probe):
        with tempfile.TemporaryDirectory() as folder, redirect_stdout(io.StringIO()):
            return proof.run_all(surfaces, scenario, Path(folder))

    def test_each_kind_of_transport_failure_is_recorded_for_its_surface_only(self):
        expected = {
            Dead: 'closed its output|stdin is closed',
            Hung: 'no reply',
            Missing: 'cannot start',
            Garbage: 'not JSON',
        }
        for cls, text in expected.items():
            cls.name = 'headless'
            report = self.run_all([(cls, 'ignored')])
            result = report['headless']
            self.assertTrue(result['failed'], cls.__name__)
            self.assertRegex(result['error'], text, cls.__name__)
            self.assertEqual(proof.exit_code(report), 1)

    def test_a_failing_surface_does_not_stop_the_next_one(self):
        Dead.name = 'headless'
        ran = []

        def scenario(surface, check, scratch):
            ran.append(surface.name)
            check('the native scenario ran', True)
            return []

        report = self.run_all([(Dead, 'x'), (FakeSurface, 'y')], lambda s, c, p: (probe if s.name == 'headless'
                                                                                 else scenario)(s, c, p))
        self.assertTrue(report['headless']['failed'])
        self.assertFalse(report['native']['failed'])
        self.assertEqual(ran, ['native'])
        self.assertEqual(report['native']['passed'], 1)
        self.assertEqual(proof.exit_code(report), 1, 'one failed surface fails the run')

    def test_a_skip_makes_the_run_incomplete_not_failed_and_not_complete(self):
        def scenario(surface, check, scratch):
            check('one', True)
            check.skip('the OS capture', 'screen locked')
            return []

        report = self.run_all([(FakeSurface, 'y')], scenario)
        result = report['native']
        self.assertFalse(result['failed'])
        self.assertFalse(result['complete'])
        self.assertEqual((result['passed'], result['skipped']), (1, 1))
        self.assertEqual(proof.exit_code(report), 2)

    def test_a_clean_run_exits_zero_and_the_surface_is_stopped(self):
        created = []

        class Tracked(FakeSurface):
            def __init__(self, binary, home):
                super().__init__(binary, home)
                created.append(self)

        report = self.run_all([(Tracked, 'y')], lambda s, c, p: c('fine', True) or [])
        self.assertEqual(proof.exit_code(report), 0)
        self.assertTrue(created[0].stopped)

    def test_a_failing_check_stops_the_scenario_and_still_stops_the_surface(self):
        created = []

        class Tracked(FakeSurface):
            def __init__(self, binary, home):
                super().__init__(binary, home)
                created.append(self)

        def scenario(surface, check, scratch):
            check('this one fails', False, 'because')
            raise AssertionError('not reached')

        report = self.run_all([(Tracked, 'y')], scenario)
        self.assertTrue(report['native']['failed'])
        self.assertIn('this one fails', report['native']['error'])
        self.assertTrue(created[0].stopped)


if __name__ == '__main__':
    unittest.main()
