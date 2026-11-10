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


if __name__ == '__main__':
    unittest.main()
