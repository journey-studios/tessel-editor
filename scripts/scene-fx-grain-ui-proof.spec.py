"""Pure tests of the grain window proof (`scene-fx-grain-ui-proof.py`): no editor, GPU or FFmpeg.

What is pinned is that the proof cannot end green when it did not look (a missing job, a failed job,
absent frame hashes are each a recorded FAIL and compare nothing), and that its expectation is the
independent Python corpus (the same hash vectors), not a copy of the product.
"""
import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("scene_fx_grain_ui_proof", Path(__file__).with_name("scene-fx-grain-ui-proof.py"))
proof = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proof)

NOWHERE = Path("/nonexistent")


class JudgeExport(unittest.TestCase):
    def setUp(self):
        proof.P.checks.clear()
        proof.P.failures.clear()

    def test_a_lost_job_compares_nothing_and_fails(self):
        compared = proof.judge_export(None, {"state": "succeeded"}, NOWHERE, "ffmpeg", 44, 8, 6)
        self.assertEqual(compared, 0)
        self.assertTrue(proof.P.failures)

    def test_a_failed_job_compares_nothing_and_fails(self):
        failed = {"state": "failed", "error": "boom"}
        self.assertEqual(proof.judge_export({"state": "succeeded"}, failed, NOWHERE, "ffmpeg", 44, 8, 6), 0)
        self.assertTrue(proof.P.failures)

    def test_missing_frame_hashes_compare_nothing_and_fail(self):
        info = {"state": "succeeded", "result": {"frame_hashes": {"list": []}}}
        self.assertEqual(proof.judge_export(info, info, NOWHERE, "ffmpeg", 44, 8, 6), 0)
        self.assertTrue(proof.P.failures)


class Expectation(unittest.TestCase):
    def test_the_expected_pixel_is_the_corpus_grain_and_amount_zero_is_the_identity(self):
        points = [(3, 4), (10, 20)]
        probes = [[120, 80, 40, 255], [10, 200, 90, 255]]
        params = dict(proof.DEFAULT_PARAMS)
        grained = proof.expected(probes, points, 180, params, 77)
        for got, probe, (x, y) in zip(grained, probes, points):
            want, _ = proof.G.grain_pixel(probe[:3], x, y, 180, params, 1.0, 77)
            self.assertEqual(got, want)
        self.assertNotEqual(grained[0], [120.0, 80.0, 40.0])
        flat = proof.expected(probes, points, 180, {**params, "amount": 0.0}, 77)
        self.assertEqual(flat, [[120.0, 80.0, 40.0], [10.0, 200.0, 90.0]])

    def test_the_seed_is_a_pure_function_of_the_frame(self):
        seeds = [proof.G.seed_for("per-frame", 77, frame) for frame in range(4)]
        self.assertEqual(len(set(seeds)), 4)
        self.assertEqual(seeds, [proof.G.seed_for("per-frame", 77, frame) for frame in range(4)])


if __name__ == "__main__":
    unittest.main()
