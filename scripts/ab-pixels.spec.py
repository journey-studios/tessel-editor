"""Tests of the pixel proof of the A/B runners (`scripts/ab/pixels.py`): a FUNCTIONAL proof, so it takes no GPU bench lock, never starts below the disk
floor and is told its arms' environment instead of patching anything. The lab is a fake: no editor, GPU or runtime page is started."""
import contextlib
import inspect
import io
import json
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ab import pixels as PX  # noqa: E402

ARMS = [("sync", {"TESSEL_VIEWPORT_ASSET_IO": "sync"}), ("async", {"TESSEL_VIEWPORT_ASSET_IO": "async"})]


class Lab:
    """What the pixel proof asks of the lab. `acquire_locks` is the thing it must never ask."""

    def __init__(self, free=40.0, error=None, agree=True, raises=None):
        self.free, self.error, self.agree, self.raises = free, error, agree, raises
        self.locks_asked, self.built, self.sessions = 0, 0, []

    def free_gib(self):
        return self.free

    def acquire_locks(self, wait):
        self.locks_asked += 1
        return None  # as if another benchmark held them

    def build_project(self, binary, folder, splats, cuts, cut_frames):
        self.built += 1
        return "document"

    def env_for(self, arm, inspect):
        return {"TESSEL_VIEWPORT_TRACE": "1"}

    def retry_while_preparing(self, call, *rest, **named):
        return call()

    def pixel_session(self, binary, document, arm, env, order, frames, cut_frames, work, retry=None):
        self.sessions.append({"env": env, "order": order, "retry": retry})
        if len(self.sessions) - 1 == self.raises:
            raise RuntimeError("the editor never opened /Users/someone/private/project.tessel.json")
        return {"arm": arm, "order": order, "hashes": {}, "monitor": {}, **({"error": self.error} if self.error else {})}

    def pixel_verdicts(self, results, frames):
        return [("explicit frames agree", self.agree, "detail")]


class PixelProofTests(unittest.TestCase):
    def run_proof(self, lab, more=None):
        shown = io.StringIO()
        with tempfile.TemporaryDirectory() as folder, mock.patch.object(PX, "warm_proof", return_value=lab), contextlib.redirect_stdout(shown):
            code = PX.run_pixels(Path("/bin/true"), folder, ARMS, more, {"factor": "x"})
            saved = json.loads((Path(folder) / "pixels.json").read_text()) if (Path(folder) / "pixels.json").exists() else None
        return code, saved, shown.getvalue()

    def test_it_runs_while_another_benchmark_holds_the_bench_locks_and_never_asks_for_them(self):
        lab = Lab()
        code, saved, _ = self.run_proof(lab)
        self.assertEqual((code, lab.locks_asked), (0, 0))
        self.assertEqual(len(saved["sessions"]), 4)

    def test_every_arm_plays_hot_and_cold_in_its_own_environment_and_keeps_its_label(self):
        lab = Lab()
        _, saved, _ = self.run_proof(lab)
        self.assertEqual([(s["env"]["TESSEL_VIEWPORT_ASSET_IO"], s["order"]) for s in lab.sessions],
                         [("sync", "hot"), ("sync", "cold"), ("async", "hot"), ("async", "cold")])
        self.assertTrue(all(s["env"]["TESSEL_VIEWPORT_TRACE"] == "1" for s in lab.sessions), "the lab's own switches stay under the arm's")
        self.assertEqual([s["label"] for s in saved["sessions"]], ["sync", "sync", "async", "async"])
        self.assertEqual(saved["factor"], "x")

    def test_below_the_start_floor_nothing_is_built_and_it_is_blocked(self):
        lab = Lab(free=PX.MIN_FREE_GIB_START - 0.1)
        code, saved, shown = self.run_proof(lab)
        self.assertEqual((code, lab.built, saved), (2, 0, None))
        self.assertIn("GiB free", shown)
        self.assertEqual(self.run_proof(Lab(free=PX.MIN_FREE_GIB_START))[0], 0)

    def test_the_start_floor_is_above_the_hard_floor_and_nothing_can_lower_it(self):
        from ab import session
        self.assertGreater(PX.MIN_FREE_GIB_START, session.MIN_FREE_GIB_FLOOR)
        self.assertEqual(session.MIN_FREE_GIB_FLOOR, 12.0)
        self.assertNotIn("min_free_gib", inspect.signature(PX.run_pixels).parameters)

    def test_a_failed_verdict_or_a_session_that_raised_is_exit_one_and_still_written(self):
        code, saved, shown = self.run_proof(Lab(agree=False))
        self.assertEqual(code, 1)
        self.assertIn("FAIL explicit frames agree", shown)
        self.assertEqual(saved["verdicts"], [{"check": "explicit frames agree", "ok": False}])
        code, saved, _ = self.run_proof(Lab(error="the editor never opened"))
        self.assertEqual(code, 1)
        self.assertEqual(saved["sessions"][0]["error"], {"code": "other", "kind": "other"})

    def test_an_error_the_session_answered_is_published_as_a_code_and_a_kind_never_as_its_text(self):
        asked = "playback.start: {'ok': False, 'error': {'code': 'media_preparing', 'message': 'wait /Users/someone/private/clip.ply'}}"
        code, saved, shown = self.run_proof(Lab(error=asked))
        self.assertEqual((code, saved["sessions"][0]["error"]), (1, {"code": "media_preparing", "kind": "host_refused"}))
        for private in ("/Users", "someone", "clip.ply"):
            self.assertNotIn(private, json.dumps(saved))
            self.assertNotIn(private, shown)

    def test_a_session_that_raised_is_a_failed_result_and_the_proof_is_still_written_with_what_ran(self):
        lab = Lab(raises=1)
        code, saved, shown = self.run_proof(lab)
        self.assertEqual(code, 1)
        self.assertIsNotNone(saved, "the only record of the failure is not lost with the working folder")
        self.assertEqual([s["label"] for s in saved["sessions"]], ["sync", "sync", "async", "async"])
        self.assertNotIn("error", saved["sessions"][0], "the session before it keeps its result")
        self.assertEqual(saved["sessions"][1]["error"], {"code": "RuntimeError", "kind": "exception"})
        self.assertEqual((saved["sessions"][1]["arm"], saved["sessions"][1]["order"]), ("warm", "cold"))
        self.assertIn("SESSION FAILED sync cold exception RuntimeError", shown)
        self.assertEqual(len(lab.sessions), 4, "each session is its own editor: the others still run")
        for private in ("/Users", "someone", "project.tessel"):
            self.assertNotIn(private, json.dumps(saved))
            self.assertNotIn(private, shown)

    def test_the_runner_can_add_its_own_verdicts(self):
        code, saved, shown = self.run_proof(Lab(), more=lambda results: [("it happened", False, len(results))])
        self.assertEqual(code, 1)
        self.assertIn("FAIL it happened  [4]", shown)


class RetryTests(unittest.TestCase):
    class Lab:
        def retry_while_preparing(self, call, *rest, **named):
            return call()

    def test_a_frame_replaced_by_a_newer_request_is_asked_again_and_every_refusal_is_kept(self):
        refused = []
        answers = iter([{"ok": False, "error": {"code": "cancelled"}}, {"ok": False, "error": {"code": "cancelled"}}, {"ok": True}])
        with mock.patch.object(time, "sleep"):
            answer = PX.retrying_on_cancel(self.Lab(), refused, "async")(lambda: next(answers))
        self.assertTrue(answer["ok"])
        self.assertEqual([item["arm"] for item in refused], ["async", "async"])
        self.assertEqual(refused[0]["error"], {"code": "cancelled", "kind": "host_refused"})

    def test_a_refusal_is_kept_as_a_code_and_a_kind_never_as_the_text_of_the_answer(self):
        refused = []
        PX.retrying_on_cancel(self.Lab(), refused, "sync")(lambda: {"ok": False, "error": {"code": "media_missing", "message": "no /Users/someone/clip.ply"}})
        self.assertEqual(refused, [{"arm": "sync", "error": {"code": "media_missing", "kind": "host_refused"}}])

    def test_any_other_refusal_is_the_answer_and_a_loop_of_cancellations_gives_up(self):
        refused = []
        other = PX.retrying_on_cancel(self.Lab(), refused, "sync")(lambda: {"ok": False, "error": {"code": "media_missing"}})
        self.assertEqual(other["error"]["code"], "media_missing")
        self.assertEqual(len(refused), 1)
        with mock.patch.object(time, "sleep"):
            stuck = PX.retrying_on_cancel(self.Lab(), [], "sync")(lambda: {"ok": False, "error": {"code": "cancelled"}})
        self.assertFalse(stuck["ok"])


if __name__ == "__main__":
    unittest.main()
