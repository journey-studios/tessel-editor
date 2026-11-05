"""Report control-flow regression tests; no editor, GPU or fixtures are started."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import MagicMock, patch

spec = importlib.util.spec_from_file_location("lifecycle", Path(__file__).with_name("viewport-lifecycle-proof.py"))
proof = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proof)


class ReportTests(unittest.TestCase):
    def exercise(self, *, exception=False, stop_error=False):
        proof.failures.clear()
        proof.report.clear()
        proof.report.update(checks=[], scenarios={})
        proof.current["name"] = None
        with tempfile.TemporaryDirectory() as directory:
            output = str(Path(directory) / "report.json")
            lab = MagicMock()
            lab.home = directory
            lab.viewport.return_value = {"page": "scratch/index.html"}
            if stop_error:
                lab.stop.side_effect = RuntimeError("stop failed")
            stream = io.StringIO()
            with contextlib.ExitStack() as stack:
                stack.enter_context(patch("sys.argv", ["proof", "--binary", "fake", "--out", output]))
                stack.enter_context(patch.object(proof.os.path, "exists", return_value=True))
                stack.enter_context(patch.object(proof.shutil, "which", return_value="ffmpeg"))
                stack.enter_context(patch.object(proof, "free_gib", return_value=99))
                stack.enter_context(patch.object(proof, "make_clip", return_value=True))
                stack.enter_context(patch.object(proof, "chaos_runtime", return_value="scratch"))
                stack.enter_context(patch.object(proof, "Lab", return_value=lab))
                stack.enter_context(patch.object(proof.time, "sleep"))
                stack.enter_context(patch.object(proof, "build_project", side_effect=RuntimeError("setup failed") if exception else None, return_value=(None, "rejected")))
                stack.enter_context(contextlib.redirect_stdout(stream))
                if exception or stop_error:
                    with self.assertRaises(RuntimeError):
                        proof.main()
                else:
                    self.assertEqual(proof.main(), 1)
            saved = json.loads(Path(output).read_text())
            self.assertEqual(json.loads(stream.getvalue().splitlines()[-1]), saved)
            self.assertTrue(saved["failures"])
            lab.stop.assert_called_once()

    def test_early_return_emits_report(self):
        self.exercise()

    def test_exception_emits_report_and_propagates(self):
        self.exercise(exception=True)

    def test_cleanup_exception_still_emits_report(self):
        self.exercise(stop_error=True)


if __name__ == "__main__":
    unittest.main()
