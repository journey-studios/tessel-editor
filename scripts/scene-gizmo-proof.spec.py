"""Pure-logic tests of the S8 gizmo proof; no editor, GPU or runtime page is started.

They pin the independent pinhole the proof uses as its oracle: if it were wrong (a flipped axis, a missing
aspect), the proof would "agree" with a wrong core or "disagree" with a right one.
"""
import importlib.util
import math
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("scene_gizmo_proof", Path(__file__).with_name("scene-gizmo-proof.py"))
proof = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proof)

CAMERA = {"position": {"x": 0, "y": 0, "z": 10}, "target": {"x": 0, "y": 0, "z": 0}, "fovDeg": 90}
WIDE = {"width": 320, "height": 180}
SQUARE = {"width": 100, "height": 100}


class ProjectionTests(unittest.TestCase):
    def test_the_look_at_point_is_the_middle_of_the_picture(self):
        u, v = proof.project(CAMERA, WIDE, [0, 0, 0])
        self.assertAlmostEqual(u, 0.5, places=12)
        self.assertAlmostEqual(v, 0.5, places=12)

    def test_right_is_larger_u_and_up_is_smaller_v(self):
        u, v = proof.project(CAMERA, SQUARE, [2, 0, 0])
        self.assertGreater(u, 0.5, "world +x is the right of the picture")
        up_u, up_v = proof.project(CAMERA, SQUARE, [0, 2, 0])
        self.assertLess(up_v, 0.5, "world +y is the top of the picture (v grows downward)")
        self.assertAlmostEqual(up_u, 0.5, places=12)

    def test_a_ninety_degree_vertical_fov_puts_the_edge_of_the_view_on_the_picture_edge(self):
        # at distance 10 the vertical half-extent is 10 (tan 45 = 1): y = 10 is the top edge (v = 0).
        _, v = proof.project(CAMERA, SQUARE, [0, 10, 0])
        self.assertAlmostEqual(v, 0.0, places=12)

    def test_the_aspect_widens_the_horizontal_extent_only(self):
        u_wide, v_wide = proof.project(CAMERA, WIDE, [10, 10, 0])
        u_square, v_square = proof.project(CAMERA, SQUARE, [10, 10, 0])
        self.assertAlmostEqual(v_wide, v_square, places=12)
        self.assertLess(abs(u_wide - 0.5), abs(u_square - 0.5), "the same x is closer to the middle of a wider picture")
        self.assertAlmostEqual(u_wide, 0.5 + 0.5 / (320 / 180), places=12)

    def test_a_moved_camera_moves_the_picture_the_other_way(self):
        moved = {**CAMERA, "position": {"x": 1, "y": 0, "z": 10}, "target": {"x": 1, "y": 0, "z": 0}}
        before = proof.project(CAMERA, SQUARE, [0, 0, 0])[0]
        after = proof.project(moved, SQUARE, [0, 0, 0])[0]
        self.assertLess(after, before)
        self.assertTrue(math.isfinite(after))


class EnvelopeTests(unittest.TestCase):
    def test_both_surfaces_normalize_to_ok_and_payload(self):
        self.assertEqual(proof.normalize({"ok": True, "data": {"a": 1}}), (True, {"a": 1}))
        self.assertEqual(proof.normalize({"ok": True, "result": {"b": 2}}), (True, {"b": 2}))
        self.assertEqual(proof.normalize({"ok": False, "error": {"code": "no_drag"}}), (False, {"code": "no_drag"}))


class RecorderTests(unittest.TestCase):
    def test_a_failed_check_is_recorded_and_stops_the_run(self):
        check = proof.Recorder("unit")
        check("passes", True)
        with self.assertRaises(proof.ProofFailure):
            check("fails", False, "why")
        self.assertEqual([item["ok"] for item in check.checks], [True, False])
        self.assertEqual(check.checks[1]["detail"], "why")


if __name__ == "__main__":
    unittest.main()
