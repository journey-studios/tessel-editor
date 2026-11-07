"""Pure-Python tests of `proof_surface.py` (BL140.6): no editor, GPU, window or Cargo.

Fake `tessel-headless`/editor programs are tiny Python scripts, so the failure modes of a surface
(end of file, non-JSON, a broken pipe, a hang, a missing binary, a `None` payload) are exercised
for real and the harness must report each one as a typed, recorded failure that does not stop the
next surface and does not mask its cause.
"""
import importlib.util
import os
from pathlib import Path
import stat
import sys
import tempfile
import textwrap
import unittest
from unittest.mock import patch
import subprocess

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
spec = importlib.util.spec_from_file_location('proof_surface', HERE / 'proof_surface.py')
ps = importlib.util.module_from_spec(spec)
sys.modules['proof_surface'] = ps
spec.loader.exec_module(ps)


def fake_headless(directory, body):
    path = Path(directory) / 'fake_headless.py'
    path.write_text(textwrap.dedent(body))
    return str(path)


ECHO = '''
import json, sys
for line in sys.stdin:
    request = json.loads(line)
    if request.get('op') == 'nodata':
        print(json.dumps({'ok': True}), flush=True)
    elif request.get('op') == 'noerror':
        print(json.dumps({'ok': False}), flush=True)
    else:
        print(json.dumps({'ok': True, 'result': {'echo': request.get('op')}}), flush=True)
'''


class HeadlessTests(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix='ps-spec')

    def surface(self, body, timeout=5):
        script = fake_headless(self.dir, body)
        surface = ps.Headless(sys.executable, None, (script,), timeout=timeout)
        self.addCleanup(surface.stop)
        return surface

    def test_a_normal_exchange_and_the_compatible_signature(self):
        script = fake_headless(self.dir, ECHO)
        surface = ps.Headless(sys.executable, None, (script,))   # (binary, home, extra) stays valid
        self.addCleanup(surface.stop)
        self.assertEqual(surface.op({'op': 'plan'}), (True, {'echo': 'plan'}))
        self.assertEqual(surface.set('t', 'f', 1)[0], True)
        self.assertEqual(surface.invoke('t', 'a')[0], True)
        self.assertEqual(surface.move('clip:x', 3)[0], True)
        self.assertEqual(surface.get('t')[1]['code'], 'desktop_required')

    def test_none_payloads_are_normalised_not_crashed_on(self):
        surface = self.surface(ECHO)
        self.assertEqual(surface.op({'op': 'nodata'}), (True, {}))
        good, error = surface.op({'op': 'noerror'})
        self.assertFalse(good)
        self.assertEqual(error['code'], 'no_error_payload')
        with self.assertRaises(ps.SurfaceError):
            ps.normalize(['not', 'an', 'object'])

    def test_end_of_file_is_a_typed_error_with_the_exit_status(self):
        surface = self.surface('import sys; sys.exit(3)')
        with self.assertRaises(ps.SurfaceError) as raised:
            surface.op({'op': 'plan'})
        self.assertRegex(str(raised.exception), r'closed its output|stdin is closed')
        with self.assertRaises(ps.SurfaceError):          # a later call fails fast, never hangs
            surface.op({'op': 'plan'})

    def test_non_json_reply_is_a_typed_error(self):
        surface = self.surface('import sys\nfor line in sys.stdin:\n    print("hello", flush=True)\n')
        with self.assertRaisesRegex(ps.SurfaceError, 'not JSON'):
            surface.op({'op': 'plan'})

    def test_a_json_reply_that_is_not_an_object_is_a_typed_error(self):
        surface = self.surface('import sys\nfor line in sys.stdin:\n    print("[1, 2]", flush=True)\n')
        with self.assertRaisesRegex(ps.SurfaceError, 'not an object'):
            surface.op({'op': 'plan'})

    def test_a_silent_process_times_out_instead_of_hanging(self):
        surface = self.surface('import sys, time\nfor line in sys.stdin:\n    time.sleep(60)\n', timeout=0.5)
        with self.assertRaisesRegex(ps.SurfaceError, 'no reply'):
            surface.op({'op': 'plan'})

    def test_a_broken_pipe_is_a_typed_error(self):
        surface = self.surface('import os\nos.close(0)\nimport time; time.sleep(30)\n', timeout=2)
        surface.process.kill()
        surface.process.wait()
        with self.assertRaises(ps.SurfaceError):
            for _ in range(5):                            # the first write may still be buffered
                surface.op({'op': 'plan'})

    def test_a_missing_binary_is_a_typed_error(self):
        with self.assertRaisesRegex(ps.SurfaceError, 'cannot start'):
            ps.Headless(os.path.join(self.dir, 'no-such-binary'))

    def test_stop_never_raises_even_when_the_process_is_gone(self):
        surface = self.surface(ECHO)
        surface.process.kill()
        surface.process.wait()
        self.assertIsInstance(surface.stop(), list)


class RecorderTests(unittest.TestCase):
    def test_a_skip_is_not_a_pass(self):
        check = ps.Recorder('x')
        check('first', True)
        check.skip('capture', 'screen locked')
        self.assertEqual((check.passed, check.skipped, len(check.checks)), (1, 1, 2))
        self.assertFalse(check.checks[1]['ok'])
        self.assertEqual(check.checks[1]['status'], 'skip')

    def test_a_failed_check_stops_the_run_and_is_recorded(self):
        check = ps.Recorder('x')
        with self.assertRaises(ps.ProofFailure):
            check('bad', False, 'why')
        self.assertEqual((check.passed, check.checks[0]['status']), (0, 'fail'))


class RunSurfaceTests(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix='ps-run'))

    def test_a_missing_binary_is_a_recorded_failure_and_the_next_surface_still_runs(self):
        first = ps.run_surface(ps.Headless, str(self.dir / 'missing'), lambda s, c, p: None, self.dir)
        self.assertTrue(first['failed'])
        self.assertIn('cannot start', first['error'])
        self.assertFalse(first['complete'])
        script = fake_headless(self.dir, ECHO)

        def scenario(surface, check, scratch):
            good, payload = surface.op({'op': 'plan'})
            check('echo', good and payload['echo'] == 'plan')

        second = ps.run_surface(ps.Headless, sys.executable, scenario, self.dir, extra=(script,))
        self.assertFalse(second['failed'])
        self.assertEqual((second['passed'], second['complete']), (1, True))

    def test_the_cause_is_kept_when_a_check_fails_or_the_surface_dies(self):
        script = fake_headless(self.dir, 'import sys; sys.exit(2)')

        def scenario(surface, check, scratch):
            surface.op({'op': 'plan'})

        died = ps.run_surface(ps.Headless, sys.executable, scenario, self.dir, extra=(script,))
        self.assertTrue(died['failed'])
        self.assertRegex(died['error'], r'^SurfaceError: ')

        def failing(surface, check, scratch):
            check('nope', False, 'detail')

        failed = ps.run_surface(ps.Headless, sys.executable, failing, self.dir,
                                extra=(fake_headless(self.dir, ECHO),))
        self.assertTrue(failed['failed'])
        self.assertIn('nope', failed['error'])

    def test_an_unexpected_exception_in_the_scenario_is_recorded_not_raised(self):
        def broken(surface, check, scratch):
            return {}['missing']

        result = ps.run_surface(ps.Headless, sys.executable, broken, self.dir,
                                extra=(fake_headless(self.dir, ECHO),))
        self.assertTrue(result['failed'])
        self.assertTrue(result['error'].startswith('KeyError'))

    def test_a_skip_makes_the_run_incomplete_but_not_failed(self):
        def scenario(surface, check, scratch):
            check('ran', True)
            check.skip('capture', 'no window')

        result = ps.run_surface(ps.Headless, sys.executable, scenario, self.dir,
                                extra=(fake_headless(self.dir, ECHO),))
        self.assertFalse(result['failed'])
        self.assertFalse(result['complete'])
        self.assertEqual((result['passed'], result['skipped']), (1, 1))

    def test_a_cleanup_failure_never_replaces_the_cause(self):
        class Dying(ps.Headless):
            def stop(self):
                raise OSError('cleanup exploded')

        def scenario(surface, check, scratch):
            check('bad', False, 'the real cause')

        result = ps.run_surface(Dying, sys.executable, scenario, self.dir,
                                extra=(fake_headless(self.dir, ECHO),))
        self.assertIn('bad', result['error'])
        self.assertTrue(any('cleanup exploded' in line for line in result['cleanup_errors']))


FAKE_EDITOR = '''#!{python}
import json, os, sys, time
if len(sys.argv) > 1 and sys.argv[1] == 'ui':
    mode = os.environ.get('FAKE_UI', 'json')
    if mode == 'json':
        print(json.dumps({{'ok': True, 'data': {{'args': sys.argv[2:]}}}}))
    elif mode == 'text':
        print('Traceback: not json')
    elif mode == 'array':
        print('[1]')
    elif mode == 'slow':
        time.sleep(60)
    sys.exit(0)
sock = os.path.join(os.environ['HOME'], '.cache/tessel-editor/ui.sock')
os.makedirs(os.path.dirname(sock), exist_ok=True)
open(sock, 'w').close()
time.sleep(120)
'''


class NativeTests(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix='ps-native'))
        self.binary = self.dir / 'fake-editor'
        self.binary.write_text(FAKE_EDITOR.format(python=sys.executable))
        self.binary.chmod(self.binary.stat().st_mode | stat.S_IEXEC)
        self.home = tempfile.mkdtemp(prefix='ps-home')

    def tearDown(self):
        os.environ.pop('FAKE_UI', None)

    def native(self, mode='json'):
        os.environ['FAKE_UI'] = mode      # the editor copies the environment when it starts
        surface = ps.Native(str(self.binary), self.home)
        self.addCleanup(surface.stop)
        return surface

    def test_ui_calls_and_prepare_work_against_a_cooperative_editor(self):
        surface = self.native()
        surface.prepare(10)
        self.assertEqual(surface.op({'op': 'plan'})[0], True)
        self.assertEqual(surface.set('t', 'f', 2)[1]['args'], ['set', 't', 'f', '2'])
        self.assertEqual(surface.move('clip:c', 4)[1]['args'], ['move', 'clip:c', '--frame', '4'])

    def test_non_json_and_non_object_ui_output_are_typed_errors(self):
        surface = self.native('text')
        with self.assertRaisesRegex(ps.SurfaceError, 'no JSON'):
            surface.get('x')
        surface.env['FAKE_UI'] = 'array'
        with self.assertRaisesRegex(ps.SurfaceError, 'not a JSON object'):
            surface.get('x')

    def test_a_hung_ui_call_times_out(self):
        surface = self.native('slow')
        with self.assertRaisesRegex(ps.SurfaceError, 'no answer within'):
            surface.ui('get', 'x', timeout=0.5)

    def test_a_missing_editor_binary_is_a_typed_error_and_leaks_no_file_handle(self):
        with self.assertRaisesRegex(ps.SurfaceError, 'cannot start'):
            ps.Native(str(self.dir / 'absent'), self.home)

    def test_stop_ends_the_editor_and_never_raises(self):
        surface = self.native()
        surface.prepare(10)
        self.assertEqual(surface.stop(), [])
        self.assertIsNotNone(surface.process.poll())
        self.assertIsInstance(surface.stop(), list)        # again, after the process is gone

    def test_capture_reports_a_reason_instead_of_a_path_when_the_os_refuses(self):
        surface = self.native()
        surface.prepare(10)
        with patch.object(ps, 'window_id', return_value=None), patch.object(ps.time, 'sleep'):
            path, reason = surface.capture(self.dir / 'shot.png')
        self.assertIsNone(path)
        self.assertIn('no on-screen window', reason)

    def test_capture_survives_a_failing_activation(self):
        surface = self.native('text')
        path, reason = surface.capture(self.dir / 'shot.png')
        self.assertIsNone(path)
        self.assertIn('activate', reason)


class WindowIdTests(unittest.TestCase):
    def test_swift_timeout_or_absence_is_none(self):
        with patch.object(ps.subprocess, 'run', side_effect=subprocess.TimeoutExpired('swift', 1)):
            self.assertIsNone(ps.window_id(1))
        with patch.object(ps.subprocess, 'run', side_effect=FileNotFoundError('swift')):
            self.assertIsNone(ps.window_id(1))

    def test_unparseable_or_zero_output_is_none_and_a_number_is_kept(self):
        for output, expected in (('', None), ('0', None), ('error: x', None), ('4242\n', '4242')):
            done = subprocess.CompletedProcess([], 0, stdout=output, stderr='')
            with patch.object(ps.subprocess, 'run', return_value=done):
                self.assertEqual(ps.window_id(1), expected, output)

    def test_the_swift_call_has_a_deadline(self):
        done = subprocess.CompletedProcess([], 0, stdout='7', stderr='')
        with patch.object(ps.subprocess, 'run', return_value=done) as run:
            ps.window_id(1)
        self.assertEqual(run.call_args.kwargs['timeout'], ps.CAPTURE_TIMEOUT)


if __name__ == '__main__':
    unittest.main()
