"""Pure-logic tests of the preview quality proof (BL130): what a run records when the final picture never comes.

No editor, GPU or runtime page is started: the editor is a fake, the scrub sub-script is a stub that writes its
JSON, and the waits are replaced.
"""
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import types
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("preview_quality", Path(__file__).with_name("preview-quality-proof.py"))
proof = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proof)


def screen(hash_="aaaa", frame=20):
    return {
        "phase": "ready",
        "session": {"presented": {"frame": frame}, "status": "ready", "revision": 3},
        "last_frame": {"hash": hash_, "revision": 3, "source": {"quality": "original", "stale_sort_layers": 0}},
        "output": {"width": 320, "height": 180},
    }


class FakeEditor:
    def __init__(self, explicit_hash="aaaa"):
        self.home = tempfile.mkdtemp(prefix="tpq-spec")
        self.socket_path = os.path.join(self.home, "ui.sock")
        self.explicit_hash = explicit_hash
        self.explicit_calls = 0

    def op(self, request, timeout=300):
        return {"ok": True, "data": {}}

    def wait_settled(self):
        pass

    def viewport(self):
        return screen()

    def explicit(self, frame, width=None, height=None, probes=None):
        self.explicit_calls += 1
        return {"ok": True, "data": {"frame": {"hash": self.explicit_hash}}}


def scrub(editor, waits, inspect=True):
    """`scrub_run` with the waits it makes (before the scrub, after it) answered by `waits`."""

    def run_subscript(command, **_):
        assert "--no-restore" in command, "compare the release frame, not a restored origin"
        out = command[command.index("--json") + 1]
        with open(out, "w") as handle:
            json.dump({"presentations_in_window": 100, "release_to_final_ms": 200}, handle)
        return types.SimpleNamespace(returncode=0, stdout="", stderr="")

    with patch.object(proof, "wait_final_quality", side_effect=waits), patch.object(proof, "wait_quiet_gpu", return_value=True), \
            patch.object(proof, "gpu_util", return_value=None), patch.object(proof.subprocess, "run", side_effect=run_subscript), \
            patch.object(proof.time, "sleep"):
        return proof.scrub_run(editor, "take", "20", "100", "60", "spec", inspect)


class FinalQualityTests(unittest.TestCase):
    def test_a_scrub_whose_final_picture_never_comes_says_so_even_with_inspect(self):
        result = scrub(FakeEditor(), [(5, screen()), (None, screen())])
        self.assertIs(result["passed"], False, "the sub-script passed, but the refined picture was never there to compare")
        self.assertIs(result["final_quality_timeout"], True)
        self.assertIn("30 s after the scrub was released", result["error"])
        self.assertEqual(result["refined_equals_explicit"], {"error": result["error"]})
        self.assertIsNone(result["final_quality_ms"])
        self.assertEqual(result["final_source"]["quality"], "original", "what was on screen instead is kept")

    def test_a_scrub_whose_final_picture_matches_the_explicit_one_passes(self):
        editor = FakeEditor("aaaa")
        result = scrub(editor, [(5, screen()), (180, screen())])
        self.assertIs(result["passed"], True)
        self.assertNotIn("final_quality_timeout", result)
        self.assertNotIn("error", result)
        self.assertEqual(result["refined_equals_explicit"]["equal"], True)
        self.assertEqual(editor.explicit_calls, 1)

    def test_a_refined_picture_that_differs_from_the_explicit_one_fails_the_run(self):
        result = scrub(FakeEditor("bbbb"), [(5, screen()), (180, screen())])
        self.assertIs(result["passed"], False)
        self.assertEqual(result["refined_equals_explicit"]["equal"], False)
        self.assertNotIn("final_quality_timeout", result)

    def test_a_comparison_that_could_not_be_made_is_not_a_pass(self):
        broken = screen()
        broken["last_frame"]["hash"] = None
        result = scrub(FakeEditor(), [(5, screen()), (180, broken)])
        self.assertIs(result["passed"], False)
        self.assertIn("error", result["refined_equals_explicit"])

    def test_without_inspect_a_scrub_waits_but_does_not_compare_hashes(self):
        waits = [(5, screen()), (180, screen())]
        result = scrub(FakeEditor(), waits, inspect=False)
        self.assertIs(result["passed"], True)
        self.assertNotIn("refined_equals_explicit", result)

    def test_scrub_timeout_without_inspect_fails_the_run(self):
        result = scrub(FakeEditor(), [(5, screen()), (None, screen())], inspect=False)
        self.assertIs(result["passed"], False)
        self.assertIs(result["final_quality_timeout"], True)
        self.assertNotIn("refined_equals_explicit", result)

    def test_the_outcome_of_a_timeout_does_not_depend_on_inspect(self):
        for inspect in (False, True):
            outcome = proof.final_quality_outcome(FakeEditor(), None, screen(), inspect, "the playback stopped")
            self.assertIs(outcome["final_quality_timeout"], True, inspect)
            self.assertIs(outcome["passed"], False, inspect)
            self.assertIn("30 s after the playback stopped", outcome["error"])
            self.assertEqual("refined_equals_explicit" in outcome, inspect)

    def test_a_final_picture_without_inspect_compares_nothing_and_is_not_a_failure(self):
        editor = FakeEditor()
        outcome = proof.final_quality_outcome(editor, 120, screen(), False, "the playback stopped")
        self.assertNotIn("passed", outcome)
        self.assertEqual(editor.explicit_calls, 0)


class FailuresTests(unittest.TestCase):
    def test_every_run_that_failed_or_errored_is_listed_and_clean_ones_are_not(self):
        report = {
            "playback": [
                {"spec": "40:60", "rep": 0, "passed": True},
                {"spec": "40:60", "rep": 1, "passed": False, "final_quality_timeout": True, "error": "never final"},
                {"spec": "181:30", "rep": 0, "error": "trace window truncated"},
            ],
            "scrub": [{"name": "take", "rep": 0, "passed": True}, {"name": "xfade", "rep": 0, "passed": False}],
        }
        failures = proof.failures_of(report)
        self.assertEqual([(f["kind"], f["run"], f["rep"]) for f in failures], [("playback", "40:60", 1), ("playback", "181:30", 0), ("scrub", "xfade", 0)])
        self.assertIs(failures[0]["final_quality_timeout"], True)
        self.assertEqual(failures[2]["error"], "the run did not pass")
        self.assertEqual(proof.failures_of({"playback": [{"passed": True}], "scrub": [{"name": "t", "passed": True}]}), [])


if __name__ == "__main__":
    unittest.main()
