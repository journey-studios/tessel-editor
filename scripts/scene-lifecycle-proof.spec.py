"""Pure-logic tests of the native lifecycle proof; no editor, GPU or runtime page is started."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("scene_lifecycle", Path(__file__).with_name("scene-lifecycle-proof.py"))
proof = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proof)


def state(**overrides):
    value = {
        "phase": "ready",
        "headline": "Three.js · 320×180 · 4 ms",
        "last_frame": {"hash": "abc"},
        "directed": {"scene": "proof", "active": None, "queued": None, "suspended": False, "shown": {"edit_frame": 20}},
    }
    value.update(overrides)
    return value


class ConvergenceTests(unittest.TestCase):
    def test_converged_needs_the_requested_frame_and_nothing_in_flight(self):
        self.assertTrue(proof.converged(state(), 20))
        self.assertFalse(proof.converged(state(), 21), "another frame is on screen")

    def test_a_parked_in_flight_suspended_or_failed_monitor_has_not_converged(self):
        for key, value in (("active", {"id": 1}), ("queued", {"explicit": False}), ("suspended", True)):
            directed = dict(state()["directed"], **{key: value})
            self.assertFalse(proof.converged(state(directed=directed), 20), key)
        self.assertFalse(proof.converged(state(headline="Falha: context_lost"), 20))
        self.assertFalse(proof.converged(state(phase="booting"), 20))
        self.assertFalse(proof.converged(state(last_frame=None), 20))

    def test_error_code_reads_typed_errors_only(self):
        self.assertEqual(proof.error_code((False, {"code": "cancelled"})), "cancelled")
        self.assertIsNone(proof.error_code((True, {"frame": {}})))
        self.assertIsNone(proof.error_code(None), "a hang is not an error code")


class ChaosPageTests(unittest.TestCase):
    def test_the_page_installs_the_triggers_before_the_unmodified_runtime(self):
        with tempfile.TemporaryDirectory() as directory:
            runtime = Path(proof.chaos_runtime(directory))
            html = (runtime / "dist" / "index.html").read_text()
            self.assertLess(html.index("loseContext"), html.index('<script src="/runtime.js"></script>'))
            config = json.loads(html.split("const config = ", 1)[1].split(";\n", 1)[0])
            self.assertEqual({item["frame"] for item in config["triggers"]}, {item["frame"] for item in proof.TRIGGERS})
            self.assertEqual((runtime / "dist" / "runtime.js").read_bytes(), (proof.ROOT / "runtime" / "dist" / "runtime.js").read_bytes())

    def test_trigger_frames_are_unique_so_scenarios_never_fire_each_others_loss(self):
        frames = [item["frame"] for item in proof.TRIGGERS]
        self.assertEqual(len(frames), len(set(frames)))

    def test_slow_copies_keep_the_bytes_but_not_the_expectations(self):
        with tempfile.TemporaryDirectory() as directory:
            copy = proof.slow_corpus(directory, "slow-a")
            self.assertEqual((copy / "assets" / "hero.png").read_bytes(), (proof.CORPUS / "assets" / "hero.png").read_bytes())
            self.assertFalse((copy / "expected.json").exists())
            self.assertTrue(any(rule["match"] in f"{copy}/" for rule in proof.SLOW))


class OutcomeTests(unittest.TestCase):
    def test_a_failed_check_is_recorded_and_fails_the_run(self):
        proof.failures.clear()
        proof.report.update(checks=[], scenarios={})
        proof.current["name"] = "unit"
        with patch("builtins.print"):
            self.assertTrue(proof.check("passes", True))
            self.assertFalse(proof.check("fails", False, "why"))
        self.assertEqual(proof.failures, ["unit: fails"])
        self.assertEqual([item["ok"] for item in proof.report["checks"]], [True, False])
        proof.current["name"] = None


if __name__ == "__main__":
    unittest.main()
