"""Pure-logic tests of the S6 scene-edit proof; no editor, GPU or runtime page is started."""
import contextlib
import io
import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

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


class DiskFloorTests(unittest.TestCase):
    def test_a_mid_run_disk_drop_writes_failure_report_and_summary(self):
        surfaces = []

        class FakeSurface:
            def __init__(self, binary, home):
                self.home = Path(home)
                self.stopped = False
                surfaces.append(self)

            def prepare(self):
                pass

            def stop(self):
                self.stopped = True

        class Headless(FakeSurface):
            name = "headless"

        class Native(FakeSurface):
            name = "native"

        with tempfile.TemporaryDirectory() as folder:
            out = Path(folder) / "report.json"
            stdout = io.StringIO()
            free = [20, 20, 11]
            with contextlib.ExitStack() as stack:
                stack.enter_context(patch.object(sys, "argv", ["proof", "--out", str(out)]))
                stack.enter_context(patch.object(proof, "Headless", Headless))
                stack.enter_context(patch.object(proof, "Native", Native))
                stack.enter_context(patch.object(proof, "scenario", return_value=None))
                inspector = stack.enter_context(patch.object(proof, "native_inspector"))
                stack.enter_context(patch.object(proof, "sha256", return_value="test-binary"))
                stack.enter_context(patch.object(proof, "clean_tracked_tree", return_value=(True, "")))
                stack.enter_context(patch.object(proof.subprocess, "run", return_value=SimpleNamespace(stdout="test-head")))
                stack.enter_context(patch.object(proof.shutil, "disk_usage", side_effect=[
                    SimpleNamespace(free=value * 1024 ** 3) for value in free
                ]))
                stack.enter_context(contextlib.redirect_stdout(stdout))
                code = proof.main()
            report = proof.json.loads(out.read_text())
            self.assertEqual(code, 1)
            self.assertFalse(report["ok"])
            self.assertIn("free space fell below the floor", report["error"])
            self.assertEqual(report["native"]["failure"], report["error"])
            self.assertIn("FAILED:", stdout.getvalue())
            self.assertIn('"ok": false', stdout.getvalue())
            inspector.assert_not_called()
            self.assertEqual(len(surfaces), 2)
            self.assertTrue(all(surface.stopped and not surface.home.exists() for surface in surfaces))


if __name__ == "__main__":
    unittest.main()
