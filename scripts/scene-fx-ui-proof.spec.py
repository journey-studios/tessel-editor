"""Negative and pure-logic tests of the F016 window proof (`scene-fx-ui-proof.py`, review R1/A2).

No editor, window, GPU, FFmpeg or 600 s export is started: the clock is a fake one, the window is a
scripted stand-in and the decoded frames are made here. What is pinned is that the proof cannot end
green when it did not look: a timeout, a failed job, a missing or empty file, absent hashes, a wrong
frame count, a stuck undo and a run that compared nothing are each a recorded FAIL.
"""
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("scene_fx_ui_proof", Path(__file__).with_name("scene-fx-ui-proof.py"))
proof = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proof)

W = 1920
POINTS = [(0, 0), (5, 5)]
FRAME_BYTES = 3 * (5 * W + 6)


class FakeClock:
    """Time that moves only when the proof sleeps, so a 600 s deadline costs nothing."""

    def __init__(self):
        self.t = 0.0
        self.slept = 0.0

    def now(self):
        return self.t

    def sleep(self, seconds):
        self.t += seconds
        self.slept += seconds

    def clock(self):
        return proof.Clock(self.now, self.sleep)


class Recorder:
    def __init__(self):
        self.items = []

    def __call__(self, name, condition, detail=""):
        self.items.append((name, bool(condition), str(detail)))
        return bool(condition)

    @property
    def failed(self):
        return [name for name, ok, _ in self.items if not ok]

    def passed(self, fragment):
        return any(fragment in name and ok for name, ok, _ in self.items)

    def failed_with(self, fragment):
        return any(fragment in name and not ok for name, ok, _ in self.items)


class FakeLab:
    def __init__(self, win):
        self.win = win
        self.sent = []

    def send(self, request, timeout=60):
        self.sent.append(request)
        return {"ok": True}


class FakeWin:
    """A scripted window: `jobs` is what `job.get` answers, in order (the last answer repeats)."""

    def __init__(self, job_states=("running",), depths=None, dialog_ok=True):
        self.job_states = list(job_states)
        self.depths = list(depths or [])
        self.dialog_ok = dialog_ok
        self.ops = []
        self.lab = FakeLab(self)
        self.created = False

    def op(self, request, timeout=120):
        self.ops.append(request)
        kind = request["op"]
        if kind == "job.list":
            return True, {"jobs": [{"id": "job-1"}] if self.created else []}
        if kind == "job.get":
            state = self.job_states.pop(0) if len(self.job_states) > 1 else self.job_states[0]
            return True, {"state": state, "result": {}}
        if kind == "job.cancel":
            return True, {"info": {"state": "cancelled"}}
        if kind in ("history.undo", "history.redo"):
            if self.depths:
                self.depths.pop(0)
            return True, {}
        return True, {}

    def invoke(self, target, action=None):
        if target == "dialog.submit":
            self.created = True
        return {"ok": self.dialog_ok}

    def undo_depth(self):
        return self.depths[0] if self.depths else None


def frames_of(colors):
    return [bytes(color) * (FRAME_BYTES // 3) + bytes(3) for color in colors]


def decoded(base, fx):
    table = {"base.mp4": base, "fx.mp4": fx}
    return lambda path: table[Path(path).name]


class WaitTests(unittest.TestCase):
    def test_a_job_that_never_finishes_ends_at_the_deadline_without_waiting_for_real(self):
        fake = FakeClock()
        win = FakeWin(("running",))
        started = time.time()
        self.assertIsNone(proof.wait_job(win, "job-1", 600, fake.clock()))
        self.assertGreaterEqual(fake.slept, 600 - 1, "the whole timeout elapsed, on the fake clock")
        self.assertLess(time.time() - started, 5, "and not a real second of it")

    def test_a_job_that_succeeds_is_returned(self):
        fake = FakeClock()
        win = FakeWin(("running", "running", "succeeded"))
        self.assertEqual(proof.wait_job(win, "job-1", 600, fake.clock())["state"], "succeeded")
        self.assertLess(fake.slept, 5)

    def test_a_timed_out_export_is_a_recorded_fail_and_the_job_is_cancelled(self):
        fake, record = FakeClock(), Recorder()
        win = FakeWin(("running",))
        info = proof.export(win, "fx export", "/tmp/never.mp4", 44, 8, check=record, timeout=600, clock=fake.clock())
        self.assertIsNone(info)
        self.assertTrue(record.failed_with("finished within 600 s"), record.items)
        self.assertIn({"op": "job.cancel", "job": "job-1"}, win.ops)

    def test_a_dialog_that_does_not_open_is_a_fail_not_a_silent_none(self):
        record = Recorder()
        win = FakeWin(dialog_ok=False)
        self.assertIsNone(proof.export(win, "base", "/tmp/x.mp4", 44, 8, check=record, clock=FakeClock().clock()))
        self.assertEqual(len(record.failed), 1, record.items)


class JudgeTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.dir = Path(self.directory.name)
        self.base, self.fx = self.dir / "base.mp4", self.dir / "fx.mp4"
        self.base.write_bytes(b"x" * 10)
        self.fx.write_bytes(b"y" * 10)
        self.record = Recorder()
        self.frames = 4
        self.addCleanup(self.directory.cleanup)

    def info(self, state="succeeded", hashes=None, frames=None):
        hashes = hashes if hashes is not None else [f"h{i}" for i in range(frames or self.frames)]
        return {"state": state, "result": {"frame_hashes": {"list": hashes}}}

    def fx_info(self):
        # Frames 44..47 are inside a clip that ends at 48, so every hash differs from the baseline.
        return self.info(hashes=[f"fx{i}" for i in range(self.frames)])

    def good_frames(self):
        grey = (200, 160, 80)
        before = frames_of([grey] * self.frames)
        after = []
        for index in range(self.frames):
            frame = bytearray(before[index])
            for x, y in POINTS:
                at = (y * W + x) * 3
                value = proof.vignette(grey, x, y, W, 1080, 1.0)
                frame[at:at + 3] = bytes(round(channel) for channel in value)
            after.append(bytes(frame))
        return before, after

    def judge(self, baseline, fx, decode, **kwargs):
        return proof.judge_exports(
            self.record, baseline, fx, (self.base, self.fx), frames=self.frames, from_frame=44, clip_end=48,
            decode=decode, big=POINTS, **kwargs,
        )

    def test_the_green_path_compares_every_frame_and_says_so(self):
        before, after = self.good_frames()
        compared = self.judge(self.info(), self.fx_info(), decoded(before, after))
        self.assertEqual(compared, self.frames)
        self.assertEqual(self.record.failed, [], self.record.items)
        self.assertTrue(self.record.passed("coverage: 4 of 4 export frames were compared"))

    def test_an_effect_that_changed_nothing_fails_the_comparison(self):
        before, _ = self.good_frames()
        self.judge(self.info(), self.fx_info(), decoded(before, before))
        self.assertTrue(self.record.failed, "pixels equal to the baseline inside the clip must not pass")

    def test_a_timed_out_or_lost_export_is_a_fail_with_no_pass(self):
        before, after = self.good_frames()
        compared = self.judge(None, self.fx_info(), decoded(before, after))
        self.assertEqual(compared, 0)
        self.assertTrue(self.record.failed_with("baseline export: finished with a final job state"), self.record.items)
        passes = [name for name, ok, _ in self.record.items if ok and "frame" in name and "export frame" in name]
        self.assertEqual(passes, [], "nothing was compared, so nothing passes")

    def test_a_failed_job_is_a_fail(self):
        before, after = self.good_frames()
        self.judge(self.info(), self.info(state="failed"), decoded(before, after))
        self.assertTrue(self.record.failed_with("fx export: succeeded"))

    def test_a_missing_artifact_is_a_fail(self):
        before, after = self.good_frames()
        self.fx.unlink()
        self.assertEqual(self.judge(self.info(), self.fx_info(), decoded(before, after)), 0)
        self.assertTrue(self.record.failed_with("fx export: the file exists and is not empty"))

    def test_an_empty_artifact_is_a_fail(self):
        before, after = self.good_frames()
        self.base.write_bytes(b"")
        self.assertEqual(self.judge(self.info(), self.fx_info(), decoded(before, after)), 0)
        self.assertTrue(self.record.failed_with("baseline export: the file exists and is not empty"))

    def test_empty_missing_or_short_hashes_are_a_fail_not_a_skip(self):
        before, after = self.good_frames()
        for label, info in (
            ("empty", self.info(hashes=[])),
            ("short", self.info(hashes=["a", "b"])),
            ("blank", self.info(hashes=["a", "", "c", "d"])),
            ("absent", {"state": "succeeded", "result": {}}),
        ):
            with self.subTest(label):
                self.record = Recorder()
                self.assertEqual(self.judge(self.info(), info, decoded(before, after)), 0)
                self.assertTrue(self.record.failed_with("fx export: the job reports 4 non-empty frame hashes"), self.record.items)

    def test_a_wrong_decoded_frame_count_is_a_fail(self):
        before, after = self.good_frames()
        self.assertEqual(self.judge(self.info(), self.fx_info(), decoded(before[:3], after)), 0)
        self.assertTrue(self.record.failed_with("decoded frames each"))


class HistoryTests(unittest.TestCase):
    def test_an_undo_walk_reaches_its_depth(self):
        record = Recorder()
        win = FakeWin(depths=[5, 4, 3, 2])
        self.assertTrue(proof.walk_history(win, "history.undo", 2, check=record, clock=FakeClock().clock()))
        self.assertEqual(len([op for op in win.ops if op["op"] == "history.undo"]), 3)

    def test_a_stuck_history_stops_at_the_limit_instead_of_spinning(self):
        record = Recorder()

        class Stuck(FakeWin):
            def op(self, request, timeout=120):
                self.ops.append(request)
                return True, {}

            def undo_depth(self):
                return 7

        win = Stuck()
        self.assertFalse(proof.walk_history(win, "history.undo", 0, check=record, clock=FakeClock().clock()))
        self.assertTrue(record.failed_with("moved the history toward depth 0"), record.items)
        self.assertEqual(len(win.ops), 1, "the first step that moved nothing ends the walk")

    def test_the_wrong_direction_is_a_fail(self):
        record = Recorder()
        win = FakeWin(depths=[5, 6, 7, 8, 9])
        self.assertFalse(proof.walk_history(win, "history.undo", 2, check=record, clock=FakeClock().clock()))
        self.assertLessEqual(len(win.ops), 1 + proof.HISTORY_STEP_MARGIN + 3)

    def test_an_unreadable_history_status_is_a_fail(self):
        record = Recorder()
        self.assertFalse(proof.walk_history(FakeWin(depths=[]), "history.undo", 0, check=record, clock=FakeClock().clock()))

    def test_the_deadline_ends_a_slow_walk(self):
        record, fake = Recorder(), FakeClock()

        class Slow(FakeWin):
            def op(self, request, timeout=120):
                fake.t += 100
                return super().op(request, timeout)

        win = Slow(depths=list(range(50, 0, -1)))
        self.assertFalse(proof.walk_history(win, "history.undo", 1, check=record, deadline_s=30, margin=100, clock=fake.clock()))
        self.assertTrue(record.failed_with("before the 30 s deadline"), record.items)


class CleanupTests(unittest.TestCase):
    def test_stop_lab_ends_its_own_session_and_nothing_else(self):
        record = Recorder()
        own = subprocess.Popen(["sh", "-c", "sleep 60 & sleep 60"], start_new_session=True)
        other = subprocess.Popen(["sleep", "60"], start_new_session=True)
        self.addCleanup(lambda: (other.kill(), other.wait()))

        class Lab:
            stopped = False

            def stop(self):
                Lab.stopped = True

        self.assertTrue(proof.group_alive(own.pid))
        ok = proof.stop_lab(Lab(), own.pid, check=record)
        own.wait(timeout=10)
        self.assertTrue(ok, record.items)
        self.assertTrue(Lab.stopped)
        self.assertFalse(proof.group_alive(own.pid), "the sleeping child of its session went too")
        self.assertIsNone(other.poll(), "a process of another session is never touched")

    def test_a_stop_that_raises_still_cleans_the_group(self):
        record = Recorder()
        own = subprocess.Popen(["sleep", "60"], start_new_session=True)

        class Lab:
            def stop(self):
                raise RuntimeError("boom")

        self.assertTrue(proof.stop_lab(Lab(), own.pid, check=record))
        own.wait(timeout=10)


class ReportTests(unittest.TestCase):
    def setUp(self):
        proof.checks.clear()
        proof.failures.clear()
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.addCleanup(proof.checks.clear)
        self.addCleanup(proof.failures.clear)

    def run_main(self, argv, run):
        report = Path(self.directory.name) / "report.json"
        with patch.object(sys, "argv", ["proof", "--json", str(report), *argv]), patch.object(proof, "run", run):
            code = proof.main()
        return code, json.loads(report.read_text())

    def test_a_crash_is_a_recorded_failed_check_and_a_written_report(self):
        def crashing(args, surfaces):
            surfaces.append({"surface": "window", "status": "exercised"})
            raise RuntimeError("the window went away")

        code, report = self.run_main(["--binary", "/nonexistent"], crashing)
        self.assertEqual(code, 1)
        self.assertEqual(report["failed"], ["the proof ran to its end"])
        self.assertEqual(report["surfaces"][0]["surface"], "window")
        self.assertEqual(report["expected_surfaces"], ["window"])
        self.assertFalse(Path(self.directory.name, "report.json.tmp").exists(), "the report is replaced atomically")

    def test_a_missing_prerequisite_exits_2_and_still_reports_that_nothing_ran(self):
        with patch.object(sys, "argv", ["proof", "--json", str(Path(self.directory.name) / "r.json"), "--binary", "/nonexistent/tessel-editor"]):
            code = proof.main()
        report = json.loads((Path(self.directory.name) / "r.json").read_text())
        self.assertEqual(code, 2)
        self.assertEqual(report["checks"], [], "no check, so no PASS to mistake for coverage")
        self.assertEqual(report["exit_code"], 2)
        self.assertEqual(report["surfaces"], [])

    def test_a_failed_check_makes_the_exit_code_1_even_if_run_returned_0(self):
        def lying(args, surfaces):
            proof.check("something failed", False, "detail")
            return 0

        code, report = self.run_main([], lying)
        self.assertEqual(code, 1)
        self.assertEqual(report["failed"], ["something failed"])


class DefaultsTests(unittest.TestCase):
    def test_the_limits_are_the_documented_ones(self):
        self.assertEqual(proof.EXPORT_TIMEOUT_S, 600)
        self.assertGreater(proof.HISTORY_DEADLINE_S, 0)
        self.assertGreater(proof.HISTORY_STEP_MARGIN, 0)


if __name__ == "__main__":
    unittest.main()
