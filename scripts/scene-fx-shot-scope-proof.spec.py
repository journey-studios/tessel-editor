"""Pure tests of the shot-scope window proof (`scene-fx-shot-scope-proof.py`): no editor, GPU or FFmpeg.

What is pinned is that the proof cannot end green when it did not look (a lost or failed job, absent hashes,
a missing file, a wrong frame count are each a recorded FAIL and compare nothing) and, above all, that its
export judge DISCRIMINATES the three ways the feature can be wrong: a shot effect that is still drawn after
its shot ended, one that is not drawn inside its shot, and the stages drawn in the flipped order. The
expectation is the independent Python of `scene-fx-ui-proof.py`, not a copy of the product.
"""
import contextlib
import importlib.util
import io
import sys
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("scene_fx_shot_scope_proof", Path(__file__).with_name("scene-fx-shot-scope-proof.py"))
proof = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proof)

W, H = 1920, 1080
POINTS = [(0, 0), (5, 5)]
FRAME_BYTES = 3 * (5 * W + 6)
BASE = {(0, 0): [200, 120, 40], (5, 5): [90, 160, 220]}
FROM_FRAME, FRAMES = 44, 8
BASE_INFO = {"state": "succeeded", "result": {"frame_hashes": {"list": [f"base{n}" for n in range(FRAMES)]}}}
FX_INFO = {"state": "succeeded", "result": {"frame_hashes": {"list": [f"fx{n}" for n in range(FRAMES)]}}}


def frame_of(pixels):
    data = bytearray(FRAME_BYTES)
    for (x, y), rgb in pixels.items():
        at = (y * W + x) * 3
        data[at:at + 3] = bytes(int(round(min(255.0, max(0.0, c)))) for c in rgb)
    return bytes(data)


def composed(point, frame, *, shot_until=48, flipped=False, vignette_inside=True, fade=True):
    """What an export of `frame` shows at `point`: the shot's vignette while the frame is in the shot, then the
    white fade at 0.5 (the stages), or the flipped order."""
    x, y = point
    pixel = list(BASE[point])
    in_shot = frame < shot_until and vignette_inside
    if flipped:
        if fade:
            pixel = proof.P.fade(pixel, proof.WHITE, 0.5)
        if in_shot:
            pixel = proof.P.vignette(pixel, x, y, W, H, 1.0)
        return pixel
    if in_shot:
        pixel = proof.P.vignette(pixel, x, y, W, H, 1.0)
    if fade:
        pixel = proof.P.fade(pixel, proof.WHITE, 0.5)
    return pixel


def export_frames(**kind):
    return [frame_of({point: composed(point, FROM_FRAME + index, **kind) for point in POINTS}) for index in range(FRAMES)]


def baseline_frames():
    return [frame_of(dict(BASE)) for _ in range(FRAMES)]


class Judge(unittest.TestCase):
    def setUp(self):
        proof.P.checks.clear()
        proof.P.failures.clear()
        self.dir = tempfile.TemporaryDirectory()
        self.out = Path(self.dir.name)
        for name in ("base.mp4", "fx.mp4"):
            (self.out / name).write_bytes(b"not empty")

    def tearDown(self):
        self.dir.cleanup()

    def judge(self, drawn, base_info=BASE_INFO, fx_info=FX_INFO):
        decoded = {"base.mp4": baseline_frames(), "fx.mp4": drawn}
        return proof.judge_export(base_info, fx_info, self.out, FROM_FRAME, FRAMES, lambda path: decoded[path.name], big=POINTS)

    def failed(self, fragment):
        return [name for name in proof.P.failures if fragment in name]

    def test_the_right_composition_passes_every_check_and_compares_every_frame(self):
        self.assertEqual(self.judge(export_frames()), FRAMES)
        self.assertEqual(proof.P.failures, [])

    def test_a_shot_effect_that_outlives_its_shot_fails_the_frames_after_the_boundary(self):
        self.assertEqual(self.judge(export_frames(shot_until=1000)), FRAMES)
        failed = self.failed("decoded pixels equal")
        self.assertEqual(len(failed), 4, failed)
        self.assertTrue(all(any(f"frame {frame}" in name for name in failed) for frame in range(48, 52)), failed)

    def test_a_shot_effect_that_is_not_drawn_inside_its_shot_fails_the_frames_before_the_boundary(self):
        self.judge(export_frames(vignette_inside=False))
        failed = self.failed("decoded pixels equal")
        self.assertEqual(len(failed), 4, failed)
        self.assertTrue(all(any(f"frame {frame}" in name for name in failed) for frame in range(44, 48)), failed)

    def test_the_flipped_stage_order_is_caught_where_both_effects_overlap(self):
        self.judge(export_frames(flipped=True))
        failed = self.failed("decoded pixels equal")
        self.assertEqual(len(failed), 4, failed)
        self.assertTrue(all(any(f"frame {frame}" in name for name in failed) for frame in range(44, 48)), failed)

    def test_the_same_hashes_for_the_baseline_and_the_effect_are_a_failed_comparison(self):
        self.judge(export_frames(), fx_info=BASE_INFO)
        self.assertEqual(len(self.failed("the picture differs from the baseline")), FRAMES)

    def test_a_lost_job_compares_nothing_and_fails(self):
        self.assertEqual(self.judge(export_frames(), base_info=None), 0)
        self.assertTrue(proof.P.failures)

    def test_a_failed_job_compares_nothing_and_fails(self):
        self.assertEqual(self.judge(export_frames(), fx_info={"state": "failed", "error": "boom"}), 0)
        self.assertTrue(proof.P.failures)

    def test_missing_frame_hashes_compare_nothing_and_fail(self):
        empty = {"state": "succeeded", "result": {"frame_hashes": {"list": []}}}
        self.assertEqual(self.judge(export_frames(), base_info=empty, fx_info=empty), 0)
        self.assertTrue(proof.P.failures)

    def test_an_empty_file_compares_nothing_and_fails(self):
        (self.out / "fx.mp4").write_bytes(b"")
        self.assertEqual(self.judge(export_frames()), 0)
        self.assertTrue(self.failed("the file exists and is not empty"))

    def test_a_wrong_frame_count_compares_nothing_and_fails(self):
        self.assertEqual(self.judge(export_frames()[:5]), 0)
        self.assertTrue(self.failed("decoded frames each"))


class Expectation(unittest.TestCase):
    def test_the_stages_are_the_shot_first_then_the_output_and_the_flipped_order_is_another_picture(self):
        points = [(10, 10), (160, 90), (300, 170)]
        probes = [[120, 80, 40, 255], [10, 200, 90, 255], [255, 255, 255, 255]]
        stages = proof.staged(probes, points)
        flipped = proof.staged(probes, points, flipped=True)
        for got, probe, (x, y) in zip(stages, probes, points):
            vignetted = proof.P.vignette(probe[:3], x, y, proof.WIDTH, proof.HEIGHT, 1.0, **proof.STRONG)
            self.assertEqual(got, proof.P.fade(vignetted, proof.WHITE, 0.5))
        self.assertGreaterEqual(proof.P.worst(stages, flipped), 25)

    def test_a_strong_vignette_changes_every_probe_of_the_grid(self):
        points = proof.P.grid(proof.WIDTH, proof.HEIGHT)
        probes = [[120, 120, 120, 255] for _ in points]
        self.assertGreaterEqual(proof.P.worst(proof.vignette_of(probes, points, **proof.STRONG), probes), 25)

    def test_the_corpus_shots_are_the_two_halves_of_the_edit(self):
        self.assertEqual(proof.corpus_shots(), [("shot-1", 0, 48), ("shot-2", 48, 96)])


class Refusal(unittest.TestCase):
    FULL = {"ok": False, "error": {"code": "scope_not_supported", "target": "v1", "details": {"scope": "shot:shot-1"}}}

    def test_a_complete_refusal_is_the_one_asked_for(self):
        self.assertTrue(proof.refused_as(self.FULL, "scope_not_supported", target="v1", scope="shot:shot-1"))
        self.assertTrue(proof.refused_as(self.FULL, "scope_not_supported"))

    def test_a_refusal_for_another_code_target_or_scope_is_not(self):
        self.assertFalse(proof.refused_as(self.FULL, "fx_has_no_handles"))
        self.assertFalse(proof.refused_as(self.FULL, "scope_not_supported", target="v2"))
        self.assertFalse(proof.refused_as(self.FULL, "scope_not_supported", scope="shot:shot-2"))

    def test_an_answer_without_error_or_details_is_a_failed_check_and_never_a_key_error(self):
        for answer in (None, {}, {"ok": False}, {"ok": False, "error": None}, {"ok": True}, {"ok": False, "error": {"code": "scope_not_supported"}}):
            self.assertFalse(proof.refused_as(answer, "scope_not_supported", target="v1", scope="shot:shot-1"), answer)
        self.assertTrue(proof.refused_as({"ok": False, "error": {"code": "scope_not_supported"}}, "scope_not_supported"))


class Rendered(unittest.TestCase):
    """The judges zip the probes of a render with the points of the grid: a render that answers fewer is a recorded
    FAIL that stops the proof, never a comparison of the probes it has (a PASS) and never a traceback."""

    PROBES = [[120, 80, 40, 255], [10, 200, 90, 255], [255, 255, 255, 255]]
    POINTS = [(10, 10), (160, 90), (300, 170)]

    class Win:
        def __init__(self, answer):
            self.answer = answer

        def render(self, frame, points):
            return self.answer

    def setUp(self):
        proof.P.checks.clear()
        proof.P.failures.clear()

    def test_a_probe_missing_from_the_answer_stops_the_proof_with_a_recorded_fail(self):
        short = {"hash": "h", "probes": self.PROBES[:-1]}
        with self.assertRaises(proof.ShortRender):
            proof.rendered(self.Win(short), 12, self.POINTS, "shot-1")
        self.assertEqual(len(proof.P.failures), 1)
        self.assertIn("answered one probe per grid point", proof.P.failures[0])
        self.assertIn("'probes': 2, 'points': 3", proof.P.checks[-1]["detail"])

    def test_a_render_without_an_answer_or_probes_is_the_same_fail(self):
        for answer in (None, {"hash": "h"}, {"hash": "h", "probes": "abc"}):
            proof.P.failures.clear()
            with self.assertRaises(proof.ShortRender):
                proof.rendered(self.Win(answer), 12, self.POINTS, "baseline")
            self.assertEqual(len(proof.P.failures), 1, answer)

    def test_an_answer_with_one_probe_per_point_is_returned_whole(self):
        answer = {"hash": "h", "probes": self.PROBES}
        self.assertIs(proof.rendered(self.Win(answer), 12, self.POINTS, "shot-1"), answer)
        self.assertEqual(proof.P.failures, [])
        self.assertEqual(len(proof.P.checks), 1)

    def test_the_expectations_refuse_a_probe_list_that_does_not_match_the_points(self):
        for probes in (self.PROBES[:-1], self.PROBES + [[1, 2, 3, 255]]):
            with self.assertRaises(ValueError):
                proof.vignette_of(probes, self.POINTS, **proof.STRONG)
            with self.assertRaises(ValueError):
                proof.staged(probes, self.POINTS)
            with self.assertRaises(ValueError):
                proof.staged(probes, self.POINTS, flipped=True)

    def test_a_short_render_ends_the_run_red_without_a_second_generic_fail(self):
        def stop(args, surfaces):
            proof.rendered(self.Win({"hash": "h", "probes": self.PROBES[:-1]}), 12, self.POINTS, "shot-1")

        real_run, real_argv = proof.run, sys.argv
        proof.run, sys.argv = stop, ["scene-fx-shot-scope-proof.py"]
        try:
            with contextlib.redirect_stdout(io.StringIO()):
                code = proof.main()
        finally:
            proof.run, sys.argv = real_run, real_argv
        self.assertEqual(code, 1)
        self.assertEqual(len(proof.P.failures), 1, proof.P.failures)
        self.assertIn("answered one probe per grid point", proof.P.failures[0])


if __name__ == "__main__":
    unittest.main()
