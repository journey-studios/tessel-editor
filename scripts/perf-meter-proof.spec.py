"""Pure-logic tests of the debug-meter proof (F049); no editor, GPU or runtime page is started."""
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import tempfile
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("perf_meter_proof", Path(__file__).with_name("perf-meter-proof.py"))
proof = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proof)


def reset():
    proof.failures.clear()
    proof.report.clear()
    proof.report.update({"checks": [], "layouts": {}, "cost": []})


class FixtureTests(unittest.TestCase):
    def test_the_fixture_outlasts_the_measurement_window_by_the_margin(self):
        for seconds in (1.0, 8.0, 20.0, 90.5):
            frames = proof.fixture_frames(seconds)
            # At least 10 s beyond the window: the clock's start (up to a couple of seconds), the
            # settle before it and the margin the playback needs to still be running at the end.
            self.assertGreaterEqual(frames / proof.FPS, seconds + 10, seconds)
            self.assertLess(frames / proof.FPS, seconds + 14, "no gratuitous padding")

    def test_the_default_fixture_would_have_been_too_short_for_twenty_seconds(self):
        # The old fixture: 240 frames at 24 fps = 10 s. A 20 s window needs far more.
        self.assertLess(240, proof.fixture_frames(20.0))

    def test_absurd_durations_are_refused_not_silently_clamped(self):
        for seconds in (0.0, -3.0, float("nan"), float("inf"), proof.MAX_SECONDS + 1):
            with self.assertRaises(ValueError, msg=str(seconds)):
                proof.fixture_frames(seconds)


def make_project(root):
    folder = Path(root) / "project"
    (folder / "media").mkdir(parents=True)
    (folder / ".tessel" / "recovery").mkdir(parents=True)
    (folder / "media" / "clip.bin").write_bytes(b"media")
    (folder / ".tessel" / "recovery" / "rev.json").write_text("state")
    document = folder / "p.tessel.json"
    document.write_text('{"assets":[{"path":"media/clip.bin"}]}')
    return folder, document


class PrivateCopyTests(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="pmp-spec-")
        self.addCleanup(shutil.rmtree, self.root, True)
        self.folder, self.document = make_project(self.root)

    def resolves(self, document):
        return os.path.exists(os.path.join(os.path.dirname(document), "media/clip.bin"))

    def test_a_json_only_copy_loses_the_relative_media_the_negative_control(self):
        naive = Path(self.root) / "naive"
        naive.mkdir()
        copied = shutil.copyfile(self.document, naive / "p.tessel.json")
        self.assertFalse(self.resolves(copied), "the old behaviour: the base moved, the path no longer resolves")

    def test_the_private_copy_resolves_the_same_relative_path(self):
        copied = proof.private_copy(str(self.document), str(Path(self.root) / "copy"))
        self.assertTrue(self.resolves(copied))
        self.assertEqual(Path(copied).read_text(), self.document.read_text())
        self.assertFalse(Path(copied).is_symlink(), "the document is a real, private file")
        self.assertEqual((Path(copied).parent / "media" / "clip.bin").read_bytes(), b"media")

    def test_editor_state_stays_in_the_copy_and_the_original_is_not_written(self):
        before = proof.fingerprint(str(self.document))
        copied = proof.private_copy(str(self.document), str(Path(self.root) / "copy"))
        copy_folder = Path(copied).parent
        self.assertFalse((copy_folder / ".tessel").exists(), "recovery/state is not linked")
        # What the editor would write into its own state and document lands in the copy only.
        (copy_folder / ".tessel").mkdir()
        (copy_folder / ".tessel" / "autosave.json").write_text("x")
        Path(copied).write_text("{}")
        self.assertEqual(proof.fingerprint(str(self.document)), before)
        self.assertFalse((self.folder / ".tessel" / "autosave.json").exists())

    def test_the_fingerprint_notices_a_change_in_the_document_or_the_listing(self):
        before = proof.fingerprint(str(self.document))
        (self.folder / "stray.txt").write_text("x")
        self.assertNotEqual(proof.fingerprint(str(self.document)), before, "a new entry")
        (self.folder / "stray.txt").unlink()
        self.assertEqual(proof.fingerprint(str(self.document)), before)
        self.document.write_text("{}")
        self.assertNotEqual(proof.fingerprint(str(self.document)), before, "a changed document")


class FramePreparationTests(unittest.TestCase):
    class Editor:
        def __init__(self, answers):
            self.answers = list(answers)
            self.asked = 0

        def op(self, request):
            self.asked += 1
            return self.answers.pop(0) if len(self.answers) > 1 else self.answers[0]

    PREPARING = {"ok": False, "error": {"code": "media_preparing", "message": "wait"}}

    def clock(self):
        state = {"t": 0.0}
        return (lambda seconds: state.__setitem__("t", state["t"] + seconds)), (lambda: state["t"])

    def test_preparing_is_a_wait_and_the_frame_that_follows_is_the_answer(self):
        editor = self.Editor([self.PREPARING, self.PREPARING, {"ok": True}])
        sleep, now = self.clock()
        self.assertTrue(proof.frame_when_prepared(editor, 10, sleep, now)["ok"])
        self.assertEqual(editor.asked, 3)

    def test_a_real_refusal_is_returned_at_once_not_retried(self):
        missing = {"ok": False, "error": {"code": "media_missing"}}
        editor = self.Editor([missing])
        sleep, now = self.clock()
        self.assertEqual(proof.error_code(proof.frame_when_prepared(editor, 10, sleep, now)), "media_missing")
        self.assertEqual(editor.asked, 1)

    def test_preparing_that_never_ends_stays_preparing_and_cannot_pass_either_check(self):
        editor = self.Editor([self.PREPARING])
        sleep, now = self.clock()
        answer = proof.frame_when_prepared(editor, 3, sleep, now)
        self.assertFalse(answer["ok"])
        self.assertEqual(proof.error_code(answer), "media_preparing")  # the negative control rejects this too
        self.assertGreater(editor.asked, 1)


class FixtureImportTests(unittest.TestCase):
    def test_wait_job_asks_again_until_the_job_ends(self):
        states = iter(["queued", "running", "running", "succeeded"])

        class Editor:
            def op(self, request):
                return {"ok": True, "data": {"state": next(states), "result_state": "applied"}}

        state = {"t": 0.0}
        info = proof.wait_job(Editor(), "job-1", 10, lambda s: state.__setitem__("t", state["t"] + s), lambda: state["t"])
        self.assertEqual(info["state"], "succeeded")

    def test_a_job_that_never_ends_is_returned_still_running_and_fails_the_fixture(self):
        class Editor:
            def op(self, request):
                return {"ok": True, "data": {"state": "running"}}

        state = {"t": 0.0}
        info = proof.wait_job(Editor(), "job-1", 2, lambda s: state.__setitem__("t", state["t"] + s), lambda: state["t"])
        self.assertEqual(info["state"], "running")

    def test_only_identity_origin_may_hold_an_absolute_path(self):
        prefix = "/tmp/w/rel"
        ok = {"assets": [{"path": "media/clip.mp4", "identity": {"origin": prefix + "/media/clip.mp4"}}]}
        self.assertEqual(proof.absolute_strings_outside_origin(ok, prefix), [])
        leaked = {"assets": [{"path": prefix + "/media/clip.mp4", "identity": {"origin": prefix + "/x"}}]}
        self.assertEqual(proof.absolute_strings_outside_origin(leaked, prefix), [prefix + "/media/clip.mp4"])
        nested = {"a": [{"b": {"c": prefix + "/z"}}]}
        self.assertEqual(proof.absolute_strings_outside_origin(nested, prefix), [prefix + "/z"])


class MediaResolutionTests(unittest.TestCase):
    def editor(self, assets, missing):
        class Editor:
            def op(self, request):
                return {"ok": True, "data": {"assets": assets, "missing": missing}}

        return Editor()

    def test_a_resolved_asset_under_the_base_reads_ok_and_nothing_missing(self):
        got = proof.media_resolution(self.editor([{"state": "ok", "path": "/w/copy/media/clip.mp4"}], 0), "/w/copy")
        self.assertEqual(got, {"missing": 0, "states": ["ok"], "under_base": True})

    def test_a_missing_asset_is_reported_missing_the_negative_controls_signal(self):
        got = proof.media_resolution(self.editor([{"state": "missing", "path": "/w/naive/media/clip.mp4"}], 1), "/w/naive")
        self.assertEqual((got["missing"], got["states"], got["under_base"]), (1, ["missing"], True))

    def test_a_path_that_escaped_the_base_is_not_under_it(self):
        got = proof.media_resolution(self.editor([{"state": "ok", "path": "/elsewhere/clip.mp4"}], 0), "/w/copy")
        self.assertFalse(got["under_base"])

    def test_no_assets_is_not_under_the_base(self):
        self.assertFalse(proof.media_resolution(self.editor([], 0), "/w/copy")["under_base"])


class StopReportTests(unittest.TestCase):
    def test_a_stopped_running_playback_answers_the_flat_report(self):
        flat = {"state": "stopped", "ticks": 400, "skipped_frames": 0}
        self.assertEqual(proof.stop_report({"data": {"report": flat}}), (flat, False))

    def test_a_playback_that_had_already_ended_answers_a_nested_report_and_is_flagged(self):
        last = {"state": "stopped", "ticks": 480}
        report, ended = proof.stop_report({"data": {"report": {"state": "stopped", "last": last}}})
        self.assertTrue(ended)
        self.assertEqual(report, last, "the metrics are unwrapped, but the run is not a full window")

    def test_nothing_to_read_is_ended_early_too(self):
        self.assertEqual(proof.stop_report({}), ({}, True))
        self.assertEqual(proof.stop_report({"data": {"report": {"state": "stopped", "last": None}}}), ({}, True))


class MeanTests(unittest.TestCase):
    def test_runs_that_did_not_cover_their_window_are_left_out(self):
        runs = [
            {"meter": "on", "ok": True, "ticks": 100},
            {"meter": "on", "ok": True, "ticks": 200},
            {"meter": "on", "ok": False, "ticks": 5},
            {"meter": "off", "ok": True, "ticks": 7},
        ]
        self.assertEqual(proof.mean(runs, "on", "ticks"), 150.0)
        self.assertIsNone(proof.mean([runs[2]], "on", "ticks"), "nothing measured is not zero")


class ScreenshotTests(unittest.TestCase):
    def setUp(self):
        reset()
        self.addCleanup(reset)
        self.dir = tempfile.mkdtemp(prefix="pmp-shot-")
        self.addCleanup(shutil.rmtree, self.dir, True)
        self.editor = SimpleNamespace(invoke=lambda *args: None, window_id=lambda: "42")

    def shot(self, run):
        with patch.object(proof.time, "sleep"), patch.object(proof.subprocess, "run", side_effect=run):
            return proof.Editor.shot(self.editor, self.dir, "meter")

    def failed(self):
        return [item for item in proof.report["checks"] if not item["ok"]]

    def test_a_capture_that_exits_nonzero_is_a_failed_check_and_no_path(self):
        path = self.shot(lambda *a, **k: SimpleNamespace(returncode=1, stderr="not permitted"))
        self.assertIsNone(path)
        self.assertEqual(len(self.failed()), 1)
        self.assertIn("meter", proof.failures[0])

    def test_a_zero_exit_without_a_file_is_still_a_failure(self):
        path = self.shot(lambda *a, **k: SimpleNamespace(returncode=0, stderr=""))
        self.assertIsNone(path)
        self.assertEqual(len(self.failed()), 1)

    def test_an_empty_file_is_a_failure(self):
        def make_empty(command, **kwargs):
            Path(command[-1]).write_bytes(b"")
            return SimpleNamespace(returncode=0, stderr="")

        self.assertIsNone(self.shot(make_empty))
        self.assertEqual(len(self.failed()), 1)

    def test_a_real_file_is_reported_and_passes(self):
        def make_file(command, **kwargs):
            Path(command[-1]).write_bytes(b"png")
            return SimpleNamespace(returncode=0, stderr="")

        path = self.shot(make_file)
        self.assertEqual(path, os.path.join(self.dir, "meter.png"))
        self.assertEqual(self.failed(), [])
        self.assertEqual(len(proof.report["checks"]), 1)

    def test_a_stale_file_from_an_earlier_run_is_not_taken_for_the_new_capture(self):
        Path(self.dir, "meter.png").write_bytes(b"old")
        path = self.shot(lambda *a, **k: SimpleNamespace(returncode=1, stderr="denied"))
        self.assertIsNone(path)
        self.assertFalse(os.path.exists(os.path.join(self.dir, "meter.png")))

    def test_no_window_is_a_failure_and_no_directory_asks_for_nothing(self):
        self.editor.window_id = lambda: ""
        self.assertIsNone(self.shot(lambda *a, **k: SimpleNamespace(returncode=0, stderr="")))
        self.assertEqual(len(self.failed()), 1)
        reset()
        self.assertIsNone(proof.Editor.shot(self.editor, None, "meter"))
        self.assertEqual(proof.report["checks"], [], "--shots not given: nothing was asked, nothing is checked")


class FakeClockEditor:
    """An editor whose clock is its own: every request takes a known time, so what a run counts
    can be compared between the two arms without a real window."""

    ACCEPTED_PER_SECOND = 24
    ROUND_TRIP = {"meter": 0.2, "stop": 0.3}

    def __init__(self, log_path):
        self.t = 0.0
        self.calls = []
        self.log_path = log_path
        self.log = SimpleNamespace(flush=lambda: None)
        self.meter_visible = False

    def sleep(self, seconds):
        self.t += seconds

    def wall(self):
        return self.t

    def op(self, request):
        name = request["op"]
        self.calls.append(name)
        if name == "playback.stop":
            self.t += self.ROUND_TRIP["stop"]
            return {"data": {"report": {"ticks": int(self.t * 24), "skipped_frames": 0, "max_gap_frames": 1}}}
        if name == "playback.status":
            return {"data": {"state": "playing", "clock": "running"}}
        return {"ok": True, "data": {}}

    def viewport(self):
        self.calls.append("viewport")
        return {"session": {"stats": {"accepted": int(self.t * self.ACCEPTED_PER_SECOND)}}, "ui": {"repaints": int(self.t * 10)}}

    def meter(self):
        self.calls.append("meter")
        self.t += self.ROUND_TRIP["meter"]
        readout = {"ui": {"state": "live"}, "monitor": {"state": "live"}, "playback_loss": {"state": "live"}, "render_ms": 1.0, "frame": {}}
        return {"visible": self.meter_visible, "readout": readout if self.meter_visible else None}


class PlaybackRunTests(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="pmp-run-")
        self.addCleanup(shutil.rmtree, self.root, True)
        self.log = os.path.join(self.root, "editor.log")
        Path(self.log).write_text("no paints logged\n")

    def run_arm(self, on, seconds=8.0):
        editor = FakeClockEditor(self.log)
        editor.meter_visible = on
        result = proof.playback_run(editor, seconds, on, sleep=editor.sleep, wall=editor.wall)
        return editor, result

    def test_both_arms_read_the_end_of_the_window_before_anything_else_they_do(self):
        for on in (False, True):
            editor, result = self.run_arm(on)
            after_window = editor.calls[editor.calls.index("playback.start"):]
            # mark, then (after the window) the end of it first, then the same requests in both arms.
            self.assertEqual(
                [call for call in after_window if call != "playback.status"][:5],
                ["playback.start", "viewport", "viewport", "meter", "playback.stop"],
                on,
            )

    def test_the_two_arms_make_the_same_requests(self):
        off, _ = self.run_arm(False)
        on, _ = self.run_arm(True)
        self.assertEqual(off.calls, on.calls, "the meter-on arm must not make an extra round trip the other arm skips")

    def test_a_slow_meter_read_does_not_lengthen_the_span_of_the_arm_that_makes_it(self):
        # Pictures are counted to the end of the window, whatever the later round trips take.
        for on in (False, True):
            _, result = self.run_arm(on)
            self.assertEqual(result["pictures_accepted"], 8 * FakeClockEditor.ACCEPTED_PER_SECOND, on)
            self.assertEqual(result["ui_repaints"], 80, on)
        off = self.run_arm(False)[1]["pictures_accepted"]
        on = self.run_arm(True)[1]["pictures_accepted"]
        self.assertEqual(off, on, "no built-in bias toward the meter-on arm")

    def test_only_the_meter_on_arm_keeps_what_the_meter_read(self):
        self.assertNotIn("meter_reads", self.run_arm(False)[1])
        self.assertEqual(self.run_arm(True)[1]["meter_reads"]["ui"], {"state": "live"})


class FakeProcessEditor:
    """Stands in for the editor binary in `main`: no window, no socket."""

    instances = []
    meter_answers = None

    def __init__(self, binary, home, extra_env=None):
        self.log_path = os.path.join(home, "editor.log")
        self.log = open(self.log_path, "w")
        self.log.write("editor started\neditor line before the failure\n")
        self.log.flush()
        self.stopped = False
        self.log_closed_at_stop = None
        FakeProcessEditor.instances.append(self)

    def wait_ready(self, seconds=180):
        return None

    def invoke(self, *args, **kwargs):
        return {"ok": True}

    def meter(self):
        return FakeProcessEditor.meter_answers.pop(0) if len(FakeProcessEditor.meter_answers) > 1 else FakeProcessEditor.meter_answers[0]

    def op(self, request):
        return {"ok": True, "data": {}}

    def stop(self):
        self.stopped = True
        self.log.close()


HIDDEN = {"visible": False, "readout": None, "shortcut": "cmd-shift-d", "shortcut_label": "Cmd+Shift+D"}


class AbortedRunTests(unittest.TestCase):
    """An exception in the middle of the run still ends with one JSON report and exit status 1."""

    def setUp(self):
        reset()
        self.addCleanup(reset)
        self.root = tempfile.mkdtemp(prefix="pmp-main-")
        self.addCleanup(shutil.rmtree, self.root, True)
        self.work = os.path.join(self.root, "pmp-work")
        os.makedirs(self.work)
        self.tmp = os.path.join(self.root, "tmp")
        os.makedirs(self.tmp)
        self.binary = os.path.join(self.root, "editor")
        Path(self.binary).write_text("#!/bin/sh\n")
        FakeProcessEditor.instances = []

    def main(self, answers, fixture=(True, None, "unused"), argv=()):
        FakeProcessEditor.meter_answers = list(answers)
        out = io.StringIO()
        with patch.object(sys, "argv", ["perf-meter-proof.py", "--binary", self.binary, *argv]), \
                patch.object(proof, "Editor", FakeProcessEditor), \
                patch.object(proof.tempfile, "mkdtemp", lambda **kwargs: self.work), \
                patch.object(proof.tempfile, "gettempdir", lambda: self.tmp), \
                patch.object(proof, "synthetic_project", lambda *args: fixture), \
                patch.object(proof.time, "sleep"), \
                patch("sys.stdout", out):
            status = proof.main()
        lines = out.getvalue().strip().splitlines()
        return status, json.loads(lines[-1]), lines

    def test_a_missing_fixture_ends_with_the_report_a_failed_check_and_exit_one(self):
        status, report, _ = self.main([HIDDEN], fixture=(False, "boom", None))
        self.assertEqual(status, 1)
        self.assertIs(report["ok"], False)
        self.assertEqual(report["aborted"]["type"], "RuntimeError")
        self.assertIn("no fixture: boom", report["aborted"]["message"])
        names = [(item["check"], item["ok"]) for item in report["checks"]]
        # What was collected before the exception is still there...
        self.assertIn(("hidden: the node says so and carries no readout", True), names)
        # ...and the exception is an explicit failed check, not only a traceback.
        self.assertIn(("the run completed without an exception (RuntimeError)", False), names)
        self.assertIn("the run completed without an exception (RuntimeError)", report["failures"])

    def test_a_key_error_from_an_answer_is_reported_the_same_way(self):
        status, report, _ = self.main([{"visible": False}])  # no "readout": direct indexing raises
        self.assertEqual(status, 1)
        self.assertIs(report["ok"], False)
        self.assertEqual(report["aborted"]["type"], "KeyError")
        self.assertIn("KeyError", report["failures"][0])

    def test_the_editor_is_stopped_and_its_log_survives_for_the_collection(self):
        _, report, _ = self.main([HIDDEN], fixture=(False, "boom", None))
        editor = FakeProcessEditor.instances[0]
        self.assertTrue(editor.stopped, "own cleanup: the editor this run started is stopped")
        self.assertTrue(editor.log.closed, "the log is closed before it is collected")
        self.assertFalse(os.path.exists(self.work), "the work directory is removed")
        saved = report["editor_log"]
        self.assertTrue(saved.startswith(self.tmp))
        self.assertIn("editor line before the failure", Path(saved).read_text())
        self.assertIn("editor line before the failure", report["editor_log_tail"])

    def test_keep_keeps_the_whole_work_directory(self):
        _, report, lines = self.main([HIDDEN], fixture=(False, "boom", None), argv=["--keep"])
        self.assertTrue(os.path.exists(os.path.join(self.work, "home", "editor.log")))
        self.assertEqual(report["kept"], self.work)
        self.assertIn(f"kept {self.work}", lines)

    def test_the_report_is_the_last_line_and_valid_json_whatever_the_failure(self):
        _, _, lines = self.main([HIDDEN], fixture=(False, "boom", None))
        json.loads(lines[-1])
        self.assertTrue(any(line.startswith("FAIL ") for line in lines[:-1]))

    def test_an_interrupt_still_prints_the_report_and_is_then_raised(self):
        FakeProcessEditor.meter_answers = [HIDDEN]
        out = io.StringIO()
        with patch.object(sys, "argv", ["perf-meter-proof.py", "--binary", self.binary]), \
                patch.object(proof, "Editor", FakeProcessEditor), \
                patch.object(proof.tempfile, "mkdtemp", lambda **kwargs: self.work), \
                patch.object(proof.tempfile, "gettempdir", lambda: self.tmp), \
                patch.object(proof, "synthetic_project", side_effect=KeyboardInterrupt), \
                patch.object(proof.time, "sleep"), \
                patch("sys.stdout", out):
            with self.assertRaises(KeyboardInterrupt):
                proof.main()
        report = json.loads(out.getvalue().strip().splitlines()[-1])
        self.assertIs(report["ok"], False)
        self.assertEqual(report["aborted"]["type"], "KeyboardInterrupt")
        self.assertTrue(FakeProcessEditor.instances[0].stopped)


class ConcludeTests(unittest.TestCase):
    def setUp(self):
        reset()
        self.addCleanup(reset)
        self.root = tempfile.mkdtemp(prefix="pmp-conclude-")
        self.addCleanup(shutil.rmtree, self.root, True)
        self.work = os.path.join(self.root, "pmp-w")
        os.makedirs(os.path.join(self.work, "home"))
        self.editor = FakeProcessEditor(None, os.path.join(self.work, "home"))
        self.tmp = os.path.join(self.root, "tmp")
        os.makedirs(self.tmp)

    def conclude(self, keep=False):
        with patch.object(proof.tempfile, "gettempdir", lambda: self.tmp):
            return proof.conclude(self.editor, self.work, keep)

    def test_a_clean_run_leaves_nothing_behind_and_is_ok(self):
        extra = self.conclude()
        self.assertIs(extra["ok"], True)
        self.assertEqual(extra["failures"], [])
        self.assertFalse(os.path.exists(self.work))
        self.assertEqual(os.listdir(self.tmp), [])
        self.assertTrue(self.editor.stopped)

    def test_a_failed_check_keeps_the_log(self):
        proof.check("something", False, "why")
        extra = self.conclude()
        self.assertIs(extra["ok"], False)
        self.assertEqual(extra["failures"], ["something"])
        self.assertIn("editor started", Path(extra["editor_log"]).read_text())

    def test_an_editor_that_will_not_stop_is_a_failed_check_and_not_a_crash(self):
        def stuck():
            raise OSError("process vanished")

        self.editor.stop = stuck
        extra = self.conclude()
        self.assertIs(extra["ok"], False)
        self.assertEqual(extra["failures"], ["the editor stopped cleanly"])

    def test_no_editor_at_all_still_concludes(self):
        with patch.object(proof.tempfile, "gettempdir", lambda: self.tmp):
            extra = proof.conclude(None, self.work, False)
        self.assertIs(extra["ok"], True)
        self.assertFalse(os.path.exists(self.work))


if __name__ == "__main__":
    unittest.main()
