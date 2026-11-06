"""Pure-logic tests of the S6 scene-edit proof; no editor, GPU or runtime page is started."""
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("scene_edit_proof", Path(__file__).with_name("scene-edit-proof.py"))
proof = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proof)


def camera(x=0.0, fov=45.0):
    return {"position": {"x": x, "y": 1.0, "z": 8.0}, "target": {"x": 0.0, "y": 0.5, "z": 0.0}, "fovDeg": fov}


class PoseTests(unittest.TestCase):
    def test_equal_poses_agree_and_rounding_noise_is_tolerated(self):
        self.assertTrue(proof.pose_close(camera(), camera()))
        self.assertTrue(proof.pose_close(camera(1.0), camera(1.0 + 1e-12)))

    def test_any_component_beyond_the_tolerance_disagrees(self):
        self.assertFalse(proof.pose_close(camera(), camera(1e-6)), "position")
        self.assertFalse(proof.pose_close(camera(), camera(fov=45.001)), "fov")
        moved = camera()
        moved["target"]["z"] = 0.1
        self.assertFalse(proof.pose_close(camera(), moved), "target")


class EnvelopeTests(unittest.TestCase):
    def test_both_surfaces_normalize_to_ok_and_payload(self):
        self.assertEqual(proof.normalize({"ok": True, "data": {"a": 1}}), (True, {"a": 1}))
        self.assertEqual(proof.normalize({"ok": True, "result": {"b": 2}}), (True, {"b": 2}))
        self.assertEqual(proof.normalize({"ok": False, "error": {"code": "invalid_value"}}),
                         (False, {"code": "invalid_value"}))


class RecorderTests(unittest.TestCase):
    def test_a_failed_check_is_recorded_and_stops_the_run(self):
        check = proof.Recorder("unit")
        check("passes", True)
        with self.assertRaises(proof.ProofFailure):
            check("fails", False, "why")
        self.assertEqual([item["ok"] for item in check.checks], [True, False])
        self.assertEqual(check.checks[1]["detail"], "why")


class BundleTests(unittest.TestCase):
    def test_the_corpus_bundle_has_the_moving_camera_and_the_hero_plane_the_proof_edits(self):
        bundle = proof.json.loads(proof.BUNDLE.read_text())
        project = {"scenes": {"items": [{"bundle": bundle}]}}
        movement = proof.first_movement(proof.bundle_of(project))
        self.assertEqual(movement["recipe"], "dollyIn")
        self.assertEqual(proof.hero_plane(bundle)["renderOrder"], 3)
        self.assertEqual(bundle["cameraClips"][0]["id"], "cam-main")


class CleanTreeTests(unittest.TestCase):
    def git(self, folder, *args):
        subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@t", *args], cwd=folder, check=True,
                       capture_output=True)

    def test_a_modified_tracked_file_is_dirty_and_untracked_output_is_not(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            self.git(root, "init", "-q")
            (root / "a.txt").write_text("one")
            self.git(root, "add", "a.txt")
            self.git(root, "commit", "-qm", "init")
            self.assertTrue(proof.clean_tracked_tree(root)[0])
            (root / "build-output").write_text("x")
            self.assertTrue(proof.clean_tracked_tree(root)[0], "untracked build output does not count")
            (root / "a.txt").write_text("two")
            clean, status = proof.clean_tracked_tree(root)
            self.assertFalse(clean)
            self.assertIn("a.txt", status)


if __name__ == "__main__":
    unittest.main()
