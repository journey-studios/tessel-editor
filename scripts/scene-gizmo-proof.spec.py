"""Pure-logic tests of the S8 gizmo proof; no editor, GPU or runtime page is started.

They pin the independent pinhole the proof uses as its oracle: if it were wrong (a flipped axis, a missing
aspect), the proof would "agree" with a wrong core or "disagree" with a right one.

They also run the REAL `main`/`run_surface` against fake surfaces (no GPU, no editor, no repository) to pin what the
proof promises about its own report (M3 of the PR #25 review): an empty run is no pass, any exception leaves a fresh
`ok: false` and never a previous `ok: true`, a failed run keeps its diagnostics, children are always ended, and
`final_evidence` is only true for the complete, clean run.
"""
import contextlib
import importlib.util
import io
import json
import math
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
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


# ---- the proof's own report ---------------------------------------------------------------------------------

SOURCE = Path(__file__).with_name("scene-gizmo-proof.py").read_text()


class FakeSurface:
    stops = []

    def __init__(self, name, stop_error=None):
        self.name = name
        self.stop_error = stop_error
        self.killed = False
        self.stopped = False

    def prepare(self):
        pass

    def op(self, request):
        return True, {}

    def stop(self):
        self.stopped = True
        if self.stop_error:
            raise self.stop_error

    def kill(self):
        self.killed = True

    def log_tail(self):
        return f"log of {self.name}"


def passing_scenario(surface, check, scratch, record):
    for name in proof.REQUIRED[surface.name]:
        check(name, True)
    record["world_delta"] = {"x": 1.0, "y": 0.0, "z": 0.0}
    return {}


class MainTests(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="gizmo-spec-"))
        self.out = self.tmp / "report.json"
        self.created = []  # the surfaces the fake factory built
        self.works = []

    def tearDown(self):
        for work in self.works:
            shutil.rmtree(work, ignore_errors=True)
        shutil.rmtree(self.tmp, ignore_errors=True)

    def deps(self, **overrides):
        def factory(name, stop_error=None):
            def build(binary, work):
                surface = FakeSurface(name, stop_error)
                self.created.append(surface)
                return surface
            return build

        values = dict(
            factories={"headless": factory("headless"), "native": factory("native")},
            scenario=passing_scenario,
            dialog=lambda *arguments: None,
            tree_status=lambda: (True, ""),
            head=lambda: "0" * 40,
            free_gib=lambda: 100.0,
        )
        values.update(overrides)
        return proof.Deps(**values)

    def run_main(self, *arguments, deps=None):
        buffer = io.StringIO()
        with contextlib.redirect_stdout(buffer):
            code = proof.main(["--out", str(self.out), *arguments], deps or self.deps())
        report = json.loads(self.out.read_text())
        for surface in proof.SURFACES:
            if surface in report:
                self.works.append(report[surface]["work"])
        return code, report

    def test_a_complete_clean_run_is_final_evidence_and_removes_its_work(self):
        code, report = self.run_main()
        self.assertEqual(code, 0)
        self.assertIs(report["ok"], True)
        self.assertIs(report["final_evidence"], True)
        self.assertEqual(report["final_evidence_blockers"], [])
        for surface in proof.SURFACES:
            self.assertEqual(report[surface]["passed"], len(proof.REQUIRED[surface]))
            self.assertIs(report[surface]["retained"], False)
            self.assertFalse(Path(report[surface]["work"]).exists())
        self.assertTrue(all(surface.stopped for surface in self.created))

    def test_an_empty_or_unknown_surface_list_is_not_a_pass(self):
        for text in ("", " ", "none", "headless,", "headles", "headless,headless"):
            with self.subTest(surfaces=text):
                self.created.clear()
                code, report = self.run_main("--surfaces", text)
                self.assertEqual(code, 1)
                self.assertIs(report["ok"], False)
                self.assertIs(report["final_evidence"], False)
                self.assertTrue(report["error"])
                self.assertEqual(self.created, [], "nothing was started for a list that proves nothing")

    def test_a_subset_or_allow_dirty_passes_but_is_never_final_evidence(self):
        code, report = self.run_main("--surfaces", "headless")
        self.assertEqual((code, report["ok"], report["final_evidence"]), (0, True, False))
        self.assertTrue(any("subset" in blocker for blocker in report["final_evidence_blockers"]))
        dirty = self.deps(tree_status=lambda: (False, "?? new-file.py\n"))
        code, report = self.run_main("--allow-dirty", deps=dirty)
        self.assertEqual((code, report["ok"], report["final_evidence"]), (0, True, False))
        self.assertIn("--allow-dirty", report["final_evidence_blockers"])
        code, report = self.run_main(deps=dirty)
        self.assertEqual((code, report["ok"]), (1, False), "a dirty tree is refused without the flag")
        self.assertIn("new-file.py", report["error"])

    def test_an_exception_never_leaves_an_old_ok_true_and_says_it_aborted(self):
        for label, overrides in (
            ("the scenario raises", dict(scenario=lambda *a: {}["missing"])),
            ("the surface cannot be built", dict(factories={"headless": self.refuse_to_build})),
            ("the repository cannot be read", dict(tree_status=self.unreadable)),
        ):
            with self.subTest(label):
                self.out.write_text(json.dumps({"ok": True, "sentinel": "old report"}))
                code, report = self.run_main("--surfaces", "headless", deps=self.deps(**overrides))
                self.assertEqual(code, 1)
                self.assertIs(report["ok"], False)
                self.assertNotIn("sentinel", report, "the old report is gone, not edited")
                self.assertIs(report["aborted"], True)
                self.assertIn("traceback", report)
                self.assertIs(report["final_evidence"], False)

    @staticmethod
    def refuse_to_build(binary, work):
        raise FileNotFoundError(binary)

    @staticmethod
    def unreadable():
        raise KeyError("git said something else")

    def test_a_run_that_dies_before_finishing_leaves_a_failed_report_even_on_interrupt(self):
        self.out.write_text(json.dumps({"ok": True, "sentinel": "old report"}))

        def interrupted(*arguments):
            raise KeyboardInterrupt()

        with self.assertRaises(KeyboardInterrupt):
            self.run_main("--surfaces", "headless", deps=self.deps(scenario=interrupted))
        report = json.loads(self.out.read_text())
        self.assertIs(report["ok"], False)
        self.assertNotIn("sentinel", report)

    def test_the_old_report_is_replaced_before_anything_else_runs(self):
        self.out.write_text(json.dumps({"ok": True, "sentinel": "old report"}))
        seen = []

        def spy(*arguments):
            seen.append(json.loads(self.out.read_text()))
            return passing_scenario(*arguments)

        self.run_main("--surfaces", "headless", deps=self.deps(scenario=spy))
        self.assertIs(seen[0]["ok"], False, "while the run is in progress the report says it is not a pass")
        self.assertNotIn("sentinel", seen[0])

    def test_a_failed_surface_keeps_its_work_and_logs_and_a_passing_one_removes_them(self):
        def failing(surface, check, scratch, record):
            (scratch / "doc-1.tessel.json").write_text("{}")
            check("one check", True)
            check("the check that fails", False, "why")

        code, report = self.run_main("--surfaces", "headless", deps=self.deps(scenario=failing))
        self.assertEqual(code, 1)
        entry = report["headless"]
        self.assertIs(entry["retained"], True)
        self.assertTrue((Path(entry["work"]) / "scratch" / "doc-1.tessel.json").exists(), "the diagnostics survive")
        self.assertEqual(entry["log_tail"], "log of headless")
        self.assertEqual([check["ok"] for check in entry["checks"]], [True, False])
        self.assertTrue(self.created[0].stopped, "and the child was still ended")

    def test_a_surface_that_ran_nothing_or_skipped_a_required_check_fails(self):
        for label, scenario in (
            ("no check at all", lambda surface, check, scratch, record: {}),
            ("a required check that never ran",
             lambda surface, check, scratch, record: [check(name, True) for name in proof.REQUIRED[surface.name][1:]] and {}),
        ):
            with self.subTest(label):
                code, report = self.run_main("--surfaces", "headless", deps=self.deps(scenario=scenario))
                self.assertEqual(code, 1)
                self.assertIs(report["ok"], False)
                self.assertIn("required checks did not run", report["headless"]["failure"])
                self.assertIs(report["headless"]["retained"], True)

    def test_a_stop_that_fails_is_killed_reported_and_keeps_the_work(self):
        def build(binary, work):
            surface = FakeSurface("headless", RuntimeError("would not stop"))
            self.created.append(surface)
            return surface

        code, report = self.run_main("--surfaces", "headless", deps=self.deps(factories={"headless": build}))
        self.assertEqual(code, 1, "a passing scenario with a child that did not stop is not a pass")
        self.assertTrue(self.created[0].killed)
        self.assertIn("would not stop", report["headless"]["cleanup_error"])
        self.assertIs(report["headless"]["retained"], True)
        self.assertTrue(Path(report["headless"]["work"]).exists())

    def test_the_two_surfaces_that_disagree_on_the_drag_fail_the_run(self):
        def scenario(surface, check, scratch, record):
            passing_scenario(surface, check, scratch, record)
            if surface.name == "native":
                record["world_delta"] = {"x": 1.1, "y": 0.0, "z": 0.0}
            return {}

        code, report = self.run_main(deps=self.deps(scenario=scenario))
        self.assertEqual((code, report["ok"], report["surfaces_agree_on_the_drag"]), (1, False, False))

    def test_every_required_check_is_a_check_the_script_really_makes(self):
        for surface, names in proof.REQUIRED.items():
            for name in names:
                with self.subTest(surface=surface, name=name):
                    self.assertIn(f"'{name}'", SOURCE, "a renamed check would make every run fail, or vanish from the manifest")


class SurfaceListTests(unittest.TestCase):
    def test_parse_surfaces(self):
        self.assertEqual(proof.parse_surfaces("native,headless"), ["headless", "native"])
        self.assertEqual(proof.parse_surfaces("native"), ["native"])
        for bad in ("", "none", "a,b", "headless,headless", "headless,"):
            with self.assertRaises(RuntimeError, msg=bad):
                proof.parse_surfaces(bad)

    def test_final_evidence_needs_both_surfaces_a_clean_tree_and_no_flag(self):
        both = list(proof.SURFACES)
        self.assertEqual(proof.final_blockers(both, False, True), [])
        self.assertTrue(proof.final_blockers(["headless"], False, True))
        self.assertTrue(proof.final_blockers(both, True, True))
        self.assertTrue(proof.final_blockers(both, False, False))

    def test_missing_checks_names_what_did_not_run(self):
        ran = [{"name": name} for name in proof.REQUIRED["headless"][:-1]]
        self.assertEqual(proof.missing_checks("headless", ran), [proof.REQUIRED["headless"][-1]])
        self.assertEqual(proof.missing_checks("headless", []), list(proof.REQUIRED["headless"]))


class TreeStatusTests(unittest.TestCase):
    def git(self, root, *arguments):
        subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@example.invalid", *arguments], cwd=root,
                       check=True, capture_output=True)

    def test_untracked_files_make_the_tree_unclean_but_ignored_ones_do_not(self):
        with tempfile.TemporaryDirectory(prefix="gizmo-tree-") as directory:
            root = Path(directory)
            self.git(root, "init", "-q")
            (root / ".gitignore").write_text("ignored.txt\n")
            (root / "tracked.txt").write_text("a")
            self.git(root, "add", ".")
            self.git(root, "commit", "-q", "-m", "init")
            self.assertEqual(proof.tree_status(root), (True, ""))
            (root / "ignored.txt").write_text("x")
            self.assertTrue(proof.tree_status(root)[0], "an ignored file is not part of the head's question")
            (root / "new.txt").write_text("x")
            clean, status = proof.tree_status(root)
            self.assertFalse(clean, "a new file the commit does not contain is not the head")
            self.assertIn("new.txt", status)
            (root / "new.txt").unlink()
            (root / "tracked.txt").write_text("b")
            self.assertFalse(proof.tree_status(root)[0])


class HeadlessChildTests(unittest.TestCase):
    """The real `Headless` against stand-in binaries: a child that will not die, one that dies, one that lies."""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="gizmo-child-"))
        self.grace = proof.STOP_GRACE
        proof.STOP_GRACE = 0.3

    def tearDown(self):
        proof.STOP_GRACE = self.grace
        shutil.rmtree(self.tmp, ignore_errors=True)

    def binary(self, body):
        path = self.tmp / "fake-headless"
        path.write_text(f"#!{sys.executable}\nimport sys, time\n{body}\n")
        path.chmod(path.stat().st_mode | stat.S_IXUSR)
        return str(path)

    def test_a_child_that_ignores_end_of_input_is_terminated_and_never_left_alive(self):
        child = proof.Headless(self.binary("while True:\n    time.sleep(0.05)"), str(self.tmp))
        child.stop()
        self.assertIsNotNone(child.process.poll(), "stop returns only when the child is gone")

    def test_a_child_that_ignores_terminate_too_is_killed(self):
        body = "import signal\nsignal.signal(signal.SIGTERM, signal.SIG_IGN)\nwhile True:\n    time.sleep(0.05)"
        child = proof.Headless(self.binary(body), str(self.tmp))
        child.stop()
        self.assertIsNotNone(child.process.poll())

    def test_a_dead_child_is_an_error_with_its_stderr_not_a_traceback_of_the_proof(self):
        child = proof.Headless(self.binary("sys.stderr.write('panic: no gpu\\n')\nsys.exit(3)"), str(self.tmp))
        child.process.wait(10)
        with self.assertRaises(RuntimeError) as raised:
            child.op({"op": "history.status"})
        self.assertIn("panic: no gpu", str(raised.exception))
        self.assertIn("exited 3", str(raised.exception))
        child.stop()

    def test_an_answer_that_is_not_json_names_the_line_and_the_log(self):
        body = "sys.stdin.readline()\nsys.stderr.write('oops\\n')\nsys.stderr.flush()\nprint('not json at all', flush=True)\ntime.sleep(5)"
        child = proof.Headless(self.binary(body), str(self.tmp))
        with self.assertRaises(RuntimeError) as raised:
            child.op({"op": "history.status"})
        self.assertIn("not JSON", str(raised.exception))
        self.assertIn("not json at all", str(raised.exception))
        child.stop()

    def test_the_headless_stderr_is_a_file_that_survives(self):
        child = proof.Headless(self.binary("sys.stderr.write('kept\\n')\nsys.stderr.flush()"), str(self.tmp))
        child.process.wait(10)
        child.stop()
        self.assertIn("kept", Path(child.stderr_path).read_text())

    def test_main_through_the_real_headless_reports_the_failure_and_keeps_the_log(self):
        binary = self.binary("sys.stderr.write('fatal: cannot start\\n')\nsys.exit(2)")
        out = self.tmp / "report.json"
        deps = proof.Deps(
            factories={"headless": proof.Headless},
            scenario=lambda surface, check, scratch, record: surface.op({"op": "history.status"}),
            tree_status=lambda: (True, ""),
            head=lambda: "1" * 40,
            free_gib=lambda: 100.0,
        )
        with contextlib.redirect_stdout(io.StringIO()):
            code = proof.main(["--surfaces", "headless", "--headless", binary, "--out", str(out)], deps)
        report = json.loads(out.read_text())
        work = Path(report["headless"]["work"])
        try:
            self.assertEqual(code, 1)
            self.assertIs(report["ok"], False)
            self.assertIn("fatal: cannot start", report["headless"]["log_tail"])
            self.assertTrue((work / "headless.stderr.log").exists())
            self.assertIs(report["headless"]["retained"], True)
        finally:
            shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    unittest.main()
