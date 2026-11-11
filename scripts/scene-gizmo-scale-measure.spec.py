#!/usr/bin/env python3
"""Resource cleanup of the measurement driver, without a benchmark or real lock."""
import importlib.util
from pathlib import Path
import tempfile
import types
import unittest
from unittest.mock import Mock, patch

spec = importlib.util.spec_from_file_location('scale_measure', Path(__file__).with_name('scene-gizmo-scale-measure.py'))
measure = importlib.util.module_from_spec(spec)
spec.loader.exec_module(measure)


class Cleanup(unittest.TestCase):
    def test_missing_binary_removes_work_and_releases_lock(self):
        with tempfile.TemporaryDirectory() as root:
            work = Path(root) / 'work'
            work.mkdir()
            fake = types.SimpleNamespace(Headless=Mock(side_effect=FileNotFoundError('missing')), stop_surface=Mock())
            with patch.object(measure, 'LOCK', str(Path(root) / 'lock')), patch.object(measure, 'load', return_value=fake), patch.object(measure.tempfile, 'mkdtemp', return_value=str(work)), patch.object(measure.fcntl, 'flock') as flock, patch.object(measure.sys, 'argv', ['measure']):
                with self.assertRaises(FileNotFoundError):
                    measure.main()
                self.assertFalse(work.exists())
                fake.stop_surface.assert_called_once_with(None)
                self.assertEqual(flock.call_args_list[-1].args[1], measure.fcntl.LOCK_UN)

    def run_main(self, root, fake, mkdtemp=None):
        work = Path(root) / 'work'
        work.mkdir()
        loader = patch.object(measure, 'load', side_effect=fake) if isinstance(fake, BaseException) else patch.object(measure, 'load', return_value=fake)
        made = patch.object(measure.tempfile, 'mkdtemp', side_effect=mkdtemp) if mkdtemp else patch.object(measure.tempfile, 'mkdtemp', return_value=str(work))
        with patch.object(measure, 'LOCK', str(Path(root) / 'lock')), loader as load, made as make, patch.object(measure.fcntl, 'flock') as flock, patch.object(measure.sys, 'argv', ['measure']):
            with self.assertRaises(Exception) as caught:
                measure.main()
        return work, load, make, flock, caught.exception

    def test_failed_load_creates_no_work_and_releases_lock(self):
        with tempfile.TemporaryDirectory() as root:
            work, load, make, flock, error = self.run_main(root, ImportError('no proof module'))
            self.assertIsInstance(error, ImportError)
            make.assert_not_called()
            self.assertTrue(work.exists(), 'nothing of this run existed to remove')
            self.assertEqual(flock.call_args_list[-1].args[1], measure.fcntl.LOCK_UN)

    def test_failed_temp_directory_releases_lock_and_stops_nothing(self):
        with tempfile.TemporaryDirectory() as root:
            fake = types.SimpleNamespace(Headless=Mock(), stop_surface=Mock())
            work, load, make, flock, error = self.run_main(root, fake, mkdtemp=OSError('disk full'))
            self.assertIsInstance(error, OSError)
            fake.Headless.assert_not_called()
            fake.stop_surface.assert_called_once_with(None)
            self.assertEqual(flock.call_args_list[-1].args[1], measure.fcntl.LOCK_UN)

    def test_failing_stop_still_removes_work_and_releases_lock(self):
        with tempfile.TemporaryDirectory() as root:
            surface = Mock()
            surface.op.return_value = (False, 'refused')
            fake = types.SimpleNamespace(Headless=Mock(return_value=surface), stop_surface=Mock(side_effect=RuntimeError('stop failed')), SCENE='s', BUNDLE='b')
            work, load, make, flock, error = self.run_main(root, fake)
            self.assertIsInstance(error, RuntimeError)
            self.assertFalse(work.exists())
            self.assertEqual(flock.call_args_list[-1].args[1], measure.fcntl.LOCK_UN)

    def test_a_busy_lock_measures_nothing_and_touches_no_work(self):
        with tempfile.TemporaryDirectory() as root:
            fake = types.SimpleNamespace(Headless=Mock(), stop_surface=Mock())
            with patch.object(measure, 'LOCK', str(Path(root) / 'lock')), patch.object(measure, 'load', return_value=fake) as load, patch.object(measure.fcntl, 'flock', side_effect=OSError('busy')), patch.object(measure.sys, 'argv', ['measure']), patch('builtins.print') as shown:
                self.assertEqual(measure.main(), 3)
            load.assert_not_called()
            self.assertIn('lock_busy', shown.call_args.args[0])


if __name__ == '__main__':
    unittest.main()
