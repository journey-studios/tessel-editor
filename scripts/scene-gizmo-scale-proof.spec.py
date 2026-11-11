#!/usr/bin/env python3
"""The scale proof's own promise about shutdown, against a fake surface (no editor, no GPU, no binary)."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

spec = importlib.util.spec_from_file_location('scale_proof', Path(__file__).with_name('scene-gizmo-scale-proof.py'))
proof = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proof)


class Shutdown(unittest.TestCase):
    def run_surface(self, stop_result, scenario_error=None):
        report = {}
        work = tempfile.mkdtemp(prefix='te-g12-spec-', dir='/tmp')
        self.addCleanup(lambda: __import__('shutil').rmtree(work, ignore_errors=True))
        with patch.object(proof.tempfile, 'mkdtemp', return_value=work), \
                patch.object(proof.S8, 'Headless', return_value=Mock()), \
                patch.object(proof, 'scenario', side_effect=scenario_error), \
                patch.object(proof.S8, 'missing_checks', return_value=[]), \
                patch.object(proof.S8, 'stop_surface', return_value=stop_result):
            try:
                proof.run_surface('headless', '/nonexistent', report)
                raised = None
            except BaseException as failure:
                raised = failure
        return report['headless'], raised, work

    def test_a_clean_stop_passes_and_removes_the_work(self):
        entry, raised, work = self.run_surface(None)
        self.assertIsNone(raised)
        self.assertFalse(entry['retained'])
        self.assertFalse(Path(work).exists())

    def test_a_failed_stop_fails_the_proof_and_keeps_the_work(self):
        entry, raised, work = self.run_surface('Interrupted: stop')
        self.assertIsInstance(raised, RuntimeError)
        self.assertIn('did not stop cleanly', str(raised))
        self.assertEqual(entry['cleanup_error'], 'Interrupted: stop')
        self.assertTrue(entry['retained'])
        self.assertTrue(Path(work).exists())

    def test_a_failed_run_keeps_its_own_error_when_the_stop_also_fails(self):
        entry, raised, _ = self.run_surface('Interrupted: stop', scenario_error=ValueError('the scenario failed'))
        self.assertIsInstance(raised, ValueError)
        self.assertIn('the scenario failed', entry['failure'])
        self.assertEqual(entry['cleanup_error'], 'Interrupted: stop')


if __name__ == '__main__':
    unittest.main()
