"""Pure-logic tests of the debug-meter proof (F049); no editor, GPU or runtime page is started."""
import importlib.util
import os
from pathlib import Path
import shutil
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("perf_meter_proof", Path(__file__).with_name("perf-meter-proof.py"))
proof = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proof)


def reset():
    proof.failures.clear()
    proof.report["checks"].clear()


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


if __name__ == "__main__":
    unittest.main()
