"""Pure-logic tests of the BL157.4 runtime bundle A/B; no editor, GPU, runtime page, WebKit process or private media is started."""
import contextlib
import importlib.util
import inspect
import io
import json
import os
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

spec = importlib.util.spec_from_file_location("runtime_bundle_ab", Path(__file__).with_name("runtime-bundle-ab.py"))
ab = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ab)
from ab import play as shared_play  # noqa: E402  (the runner put `scripts/` on the path)

PROJECT = {"length": 100, "fps": 24.0, "cuts": [], "video_starts": [], "sources": []}

# The shape of a real `sample` of the frozen page (names as the lab's two samples show them, trimmed).
DEADLOCK_SAMPLE = """
Thread_1  DispatchQueue_1: com.apple.main-thread
    255 JSC::JITWorklist::suspendAllThreads()  (in JavaScriptCore)
      255 WTF::LockAlgorithm<unsigned char, (unsigned char)1, (unsigned char)2>::lockSlow(WTF::Atomic<unsigned char>&)  (in JavaScriptCore)
Thread_2  DispatchQueue_2: com.apple.WebKit.worker
    255 WebCore::ReadableStreamDefaultReader::releaseLock(WebCore::JSDOMGlobalObject&)  (in WebCore)
      255 JSC::Heap::collectNow()  (in JavaScriptCore)
        255 WebCore::JSReadableStreamDefaultReader::visitOutputConstraints<JSC::SlotVisitor>(JSC::JSCell*, JSC::SlotVisitor&)  (in WebCore)
          255 WTF::LockAlgorithm<unsigned char, (unsigned char)1, (unsigned char)2>::lockSlow(WTF::Atomic<unsigned char>&)  (in JavaScriptCore)
"""
HEALTHY_SAMPLE = """
Thread_1  DispatchQueue_1: com.apple.main-thread
    255 mach_msg2_trap  (in libsystem_kernel.dylib)
Thread_2  Worker
    255 WebCore::ReadableStreamDefaultReader::read(WebCore::JSDOMGlobalObject&)  (in WebCore)
"""


def frame(id, shown, done):
    return {"id": id, "shown_frame": shown, "submitted_ms": done - 10.0, "done_ms": done, "submit_to_done_ms": 10.0, "transit_ms": 2.0,
            "runtime_total_ms": 8.0, "interactive": True, "prepare_ms": 1.0}


def play_events(start):
    return [{"kind": "play_command", "t_ms": start, "seq": int(start), "fields": {}},
            {"kind": "play_clock_started", "t_ms": start + 50.0, "seq": int(start) + 1, "fields": {}}]


def pictures(first_id, start, count, step=41.7, stall_after=None, stall_ms=0.0):
    """`count` pictures, `step` ms apart, with one wait of `stall_ms` after the `stall_after`-th."""
    frames, done = [], start + 100.0
    for n in range(count):
        done += stall_ms if stall_after is not None and n == stall_after else step
        frames.append(frame(first_id + n, n, done))
    return frames


def record(arm, first=None, repeat=None, error=None):
    """A session of two plays; `first` and `repeat` are lists of pictures (None: a healthy play of 100 pictures)."""
    first = pictures(1, 1000.0, 100) if first is None else first
    repeat = pictures(1000, 20000.0, 100) if repeat is None else repeat
    session = {"number": 0, "mode": "async", "order_index": 0, "pair": 0, "arm": arm, "env": {}, "host": {"loadavg_start": [1, 1, 1]},
               "events": play_events(1000.0) + play_events(20000.0), "frames": first + repeat, "events_lost": 0, "plays_ok": error is None,
               "status": {"asset_io": {"mode": "async", "pool": {}}, "stats": {"warmed": 2}}, "external_cores": [1.0, 1.5],
               "memory_series": [[0, 100.0, 50.0, 150.0], [2, 120.0, 60.0, 180.0]], "loadavg": [[1.0, 1.0, 1.0]], "errors": []}
    if error:
        session["error"] = error
    return session


def row(arm, hung=False, failed=False, deadlocks=0):
    return {"arm": arm, "hung": hung, "failed": failed, "valid": True, "samples": [{"signature": {"deadlock": True}}] * deadlocks}


def rows(unpatched_hung, patched_hung, n=10):
    return [row(ab.UNPATCHED, hung=i < unpatched_hung) for i in range(n)] + [row(ab.PATCHED, hung=i < patched_hung) for i in range(n)]


class ArmTests(unittest.TestCase):
    def test_the_order_alternates_which_arm_goes_first_by_pair(self):
        self.assertEqual(ab.arm_order(ab.ARMS, 3), ["unpatched", "patched", "patched", "unpatched", "unpatched", "patched"])
        self.assertEqual(ab.arm_order(ab.ARMS, 0), [])

    def test_an_unpatched_bundle_has_the_decoders_call_exactly_once(self):
        self.assertEqual(ab.bundle_findings(ab.UNPATCHED, f"a();{ab.DECODER_CALL}b();"), [])
        self.assertIn("0 time(s)", ab.bundle_findings(ab.UNPATCHED, "a();void 0;b();")[0])
        self.assertIn("2 time(s)", ab.bundle_findings(ab.UNPATCHED, ab.DECODER_CALL * 2)[0])

    def test_a_patched_bundle_has_none(self):
        self.assertEqual(ab.bundle_findings(ab.PATCHED, "a();void 0;b();"), [])
        self.assertIn("still has", ab.bundle_findings(ab.PATCHED, f"a();{ab.DECODER_CALL}")[0])

    def test_the_other_stream_of_the_library_is_not_the_decoders_call(self):
        self.assertEqual(ab.bundle_findings(ab.PATCHED, "readStream.releaseLock();"), [])


class HangRuleTests(unittest.TestCase):
    def play(self, pictures, max_gap):
        return {"pictures": pictures, "interval_ms": {"max": max_gap}}

    def test_a_play_that_shows_fewer_than_nine_tenths_of_the_frames_is_hung(self):
        self.assertFalse(ab.hung_play(self.play(90, 50.0), 100))
        self.assertTrue(ab.hung_play(self.play(89, 50.0), 100))

    def test_a_wait_over_two_seconds_between_two_pictures_is_hung(self):
        self.assertFalse(ab.hung_play(self.play(100, 2000.0), 100))
        self.assertTrue(ab.hung_play(self.play(100, 2000.1), 100))

    def test_a_play_with_no_picture_at_all_is_hung(self):
        self.assertTrue(ab.hung_play(self.play(0, None), 100))

    def test_it_uses_the_one_hang_rule_of_every_experiment(self):
        self.assertIs(ab.hung_play, shared_play.hung_play, "a copy of the rule in the runner could drift from the others")
        self.assertEqual((shared_play.HANG_PICTURES_SHARE, shared_play.HANG_GAP_MS), (0.9, 2000.0))


class SignatureTests(unittest.TestCase):
    def test_the_lab_stack_is_the_deadlock(self):
        self.assertEqual(ab.deadlock_signature(DEADLOCK_SAMPLE), {"release_lock": True, "collector": True, "lock_wait": True, "deadlock": True})

    def test_a_page_that_is_merely_reading_a_stream_is_not(self):
        signature = ab.deadlock_signature(HEALTHY_SAMPLE)
        self.assertFalse(signature["deadlock"])
        self.assertFalse(signature["release_lock"])

    def test_each_of_the_three_parts_is_needed(self):
        for missing in ("ReadableStreamDefaultReader::releaseLock", "visitOutputConstraints", "lockSlow"):
            self.assertFalse(ab.deadlock_signature(DEADLOCK_SAMPLE.replace(missing, "x"))["deadlock"], missing)

    def test_the_other_name_of_the_collectors_step_counts_as_the_collector(self):
        text = DEADLOCK_SAMPLE.replace("visitOutputConstraints", "isReachableFromOpaqueRoots")
        self.assertTrue(ab.deadlock_signature(text)["deadlock"])

    def test_an_empty_sample_is_not_a_deadlock(self):
        self.assertFalse(ab.deadlock_signature("")["deadlock"])


class ChanceTests(unittest.TestCase):
    def test_all_the_hangs_in_the_unpatched_arm(self):
        # 5 hung of 10 sessions, all in the unpatched arm of 5: 1 way in C(10, 5) = 252.
        self.assertAlmostEqual(ab.chance_of_so_few(5, 5, 0, 5), 1 / 252)

    def test_three_hangs_none_in_the_patched_arm_of_twelve(self):
        self.assertAlmostEqual(ab.chance_of_so_few(3, 12, 0, 12), 293930 / 2704156)

    def test_alike_arms_are_not_surprising(self):
        self.assertGreater(ab.chance_of_so_few(2, 10, 2, 10), 0.5)

    def test_no_hang_at_all_or_an_empty_arm_says_nothing(self):
        self.assertEqual(ab.chance_of_so_few(0, 10, 0, 10), 1.0)
        self.assertEqual(ab.chance_of_so_few(3, 10, 0, 0), 1.0)
        self.assertEqual(ab.chance_of_so_few(3, 0, 0, 10), 1.0)


class VerdictTests(unittest.TestCase):
    def test_fewer_than_three_hangs_in_the_unpatched_arm_says_nothing_about_the_patch(self):
        self.assertTrue(ab.verdict(rows(2, 0)).startswith("NOT REPRODUCED"))
        self.assertTrue(ab.verdict(rows(0, 0)).startswith("NOT REPRODUCED"))

    def test_three_hangs_and_none_with_the_patch_is_confirmed(self):
        self.assertTrue(ab.verdict(rows(3, 0)).startswith("CONFIRMED"))
        self.assertIn("unpatched 3 of 10 sessions hung, patched 0 of 10", ab.verdict(rows(3, 0)))

    def test_some_left_with_the_patch_is_reduced_and_not_called_a_fix(self):
        verdict = ab.verdict(rows(5, 1))
        self.assertTrue(verdict.startswith("REDUCED"))
        self.assertIn("not called a fix", verdict)

    def test_as_many_or_more_with_the_patch_is_no_effect(self):
        self.assertTrue(ab.verdict(rows(4, 4)).startswith("NO EFFECT"))
        self.assertTrue(ab.verdict(rows(4, 6)).startswith("NO EFFECT"))

    def test_an_arm_without_sessions_is_inconclusive(self):
        self.assertTrue(ab.verdict([row(ab.UNPATCHED, hung=True)]).startswith("INCONCLUSIVE"))
        self.assertTrue(ab.verdict([]).startswith("INCONCLUSIVE"))

    def test_a_failed_session_is_not_a_hang(self):
        failed = [row(ab.UNPATCHED, failed=True) for _ in range(5)] + [row(ab.PATCHED) for _ in range(5)]
        self.assertTrue(ab.verdict(failed).startswith("NOT REPRODUCED"))
        self.assertEqual(ab.counts(failed)[ab.UNPATCHED]["failed"], 5)
        self.assertEqual(ab.counts(failed)[ab.UNPATCHED]["hung"], 0)

    def test_deadlock_samples_are_counted_per_arm(self):
        counted = ab.counts([row(ab.UNPATCHED, hung=True, deadlocks=2), row(ab.PATCHED)])
        self.assertEqual((counted[ab.UNPATCHED]["deadlock_samples"], counted[ab.PATCHED]["deadlock_samples"]), (2, 0))


class SessionRowTests(unittest.TestCase):
    def test_a_healthy_session_is_neither_hung_nor_failed(self):
        result = ab.session_row(record(ab.PATCHED), PROJECT)
        self.assertEqual((result["hung"], result["failed"]), (False, False))
        self.assertEqual(sorted(result["plays"]), ["first", "repeat"])
        self.assertEqual(result["plays"]["first"]["pictures"], 100)

    def test_a_play_that_stops_showing_pictures_is_a_hung_session(self):
        result = ab.session_row(record(ab.UNPATCHED, first=pictures(1, 1000.0, 20)), PROJECT)
        self.assertTrue(result["hung"])
        self.assertTrue(result["plays"]["first"]["hung"])
        self.assertFalse(result["plays"]["repeat"]["hung"])
        self.assertFalse(result["failed"])

    def test_a_long_wait_in_the_repeat_is_a_hung_session_too(self):
        result = ab.session_row(record(ab.UNPATCHED, repeat=pictures(1000, 20000.0, 100, stall_after=50, stall_ms=5000.0)), PROJECT)
        self.assertTrue(result["hung"])
        self.assertTrue(result["plays"]["repeat"]["hung"])
        self.assertEqual(result["plays"]["repeat"]["max_gap_ms"], 5000.0)

    def test_a_session_that_errors_without_a_hung_play_is_failed_not_hung(self):
        broken = record(ab.PATCHED, error="RuntimeError('the viewport never became ready')")
        broken["events"], broken["frames"] = [], []
        result = ab.session_row(broken, PROJECT)
        self.assertEqual((result["hung"], result["failed"]), (False, True))
        self.assertEqual(result["plays"], {})

    def test_a_hang_never_excuses_a_failure_a_hung_session_that_errored_is_both(self):
        result = ab.session_row(record(ab.UNPATCHED, first=pictures(1, 1000.0, 20), error="RuntimeError('the second play never started')"), PROJECT)
        self.assertEqual((result["hung"], result["failed"]), (True, True))
        self.assertEqual(ab.status_of(result), "HUNG, FAILED: exception:RuntimeError")

    def test_a_session_whose_repeat_timed_out_or_whose_editor_died_after_both_plays_is_failed_though_no_play_hung(self):
        # Both plays are there and healthy; the harness recorded the timeout in `errors` (and `plays_ok`), not in `error`.
        for name, defect in {"a timeout": {"errors": ["play(repeat) did not end within 50 s"], "plays_ok": False},
                             "an editor that died after the plays": {"editor_exit": {"alive": False, "returncode_before_stop": -11}}}.items():
            with self.subTest(defect=name):
                result = ab.session_row({**record(ab.PATCHED), **defect}, PROJECT)
                self.assertEqual((result["hung"], result["failed"]), (False, True))
                self.assertEqual(sorted(result["plays"]), ["first", "repeat"])
                self.assertEqual(ab.status_of(result), "FAILED: incomplete")
                manifest = {"git": {"sha": "0" * 40, "dirty_files": 0}, "binary_sha256": "b", "document_sha256": "d", "stress_cpu": 0, "order": [],
                            "bundle_sha256": {"unpatched": "u", "patched": "p"}}
                self.assertFalse(ab.build_report([{**record(ab.PATCHED), **defect}], PROJECT, manifest)["all_ran"], "exit 1, not 0")

    def test_the_error_of_a_failed_session_is_published_as_a_code_and_a_kind_without_its_path(self):
        broken = record(ab.PATCHED, error="RuntimeError('could not open /Users/someone/private/clip.ply')")
        broken["events"], broken["frames"] = [], []
        manifest = {"git": {"sha": "0" * 40, "dirty_files": 0}, "binary_sha256": "b", "document_sha256": "d", "stress_cpu": 0, "order": [],
                    "bundle_sha256": {"unpatched": "u", "patched": "p"}}
        report = ab.build_report([broken], PROJECT, manifest)
        text = ab.markdown(report)
        self.assertEqual(report["rows"][0]["error"], {"code": "RuntimeError", "kind": "exception"})
        self.assertIn("FAILED: exception:RuntimeError", text)
        for private in ("/Users", "someone", "private", "clip.ply"):
            self.assertNotIn(private, json.dumps(report))
            self.assertNotIn(private, text)

    def test_the_samples_of_a_session_are_carried_with_their_signature(self):
        session = record(ab.UNPATCHED, first=pictures(1, 1000.0, 20))
        session["hang_samples"] = [{"at_s": 31.0, "pid": 1, "file": "x.txt", "signature": ab.deadlock_signature(DEADLOCK_SAMPLE)}]
        result = ab.session_row(session, PROJECT)
        self.assertEqual([s["signature"]["deadlock"] for s in result["samples"]], [True])


class WatchdogTests(unittest.TestCase):
    def dog(self):
        return ab.Watchdog("/nonexistent", "00-unpatched", set(), now=lambda: 0.0)

    def test_it_waits_for_the_table_to_stop_growing_for_the_stall_time(self):
        dog = self.dog()
        self.assertFalse(dog.stalled(10, 0.0))
        self.assertFalse(dog.stalled(10, ab.STALL_SECONDS - 0.1))
        self.assertTrue(dog.stalled(10, ab.STALL_SECONDS))

    def test_a_table_that_grows_resets_the_wait(self):
        dog = self.dog()
        dog.stalled(10, 0.0)
        self.assertFalse(dog.stalled(11, ab.STALL_SECONDS))
        self.assertFalse(dog.stalled(11, ab.STALL_SECONDS * 1.5))
        self.assertTrue(dog.stalled(11, ab.STALL_SECONDS * 2))

    def test_it_samples_at_most_twice_a_session_and_not_back_to_back(self):
        dog = self.dog()
        dog.stalled(10, 0.0)
        self.assertTrue(dog.stalled(10, 10.0))
        dog.last_sample = 10.0
        self.assertFalse(dog.stalled(10, 15.0), "not again before another stall time passed")
        self.assertTrue(dog.stalled(10, 20.0))
        dog.samples = [{}] * ab.MAX_SAMPLES_PER_SESSION
        self.assertFalse(dog.stalled(10, 40.0), "two samples are the most a session keeps")

    def test_a_page_that_is_not_playing_is_not_sampled(self):
        class Lab:
            def op(self, request):
                return {"ok": True, "data": {"state": "idle"}}

        dog = self.dog()
        dog.count, dog.changed = 5, -100.0
        dog.observe(Lab(), 5)
        self.assertEqual(dog.samples, [])
        self.assertIsNone(dog.last_sample)

    def test_the_wrapped_poll_never_takes_a_session_down(self):
        class Lab:
            def op(self, request):
                raise OSError("socket gone")

        dog = self.dog()
        dog.count, dog.changed = 5, -100.0
        state = {"frames": {i: i for i in range(5)}}
        self.assertEqual(dog.wrap(lambda lab, st, deadline: "polled")(Lab(), state, 0), "polled")
        self.assertIn("socket gone", dog.samples[0]["error"])


class NothingIsSignalledTests(unittest.TestCase):
    """AGENTS.md: no process of anybody else is ever killed. The WebKit helpers are not children of the editor, so a deadlocked page is reported, not reaped."""

    def sampled(self, signalled, pids=(4242,), before=()):
        """A watchdog that sees a stalled, playing editor and new WebContent helpers (`pids`) whose `sample` is the deadlock, with `before` samples already taken."""
        class Lab:
            def op(self, request):
                return {"ok": True, "data": {"state": "playing"}}

        def fake_sample(command, **named):
            Path(command[command.index("-file") + 1]).write_text(DEADLOCK_SAMPLE)

        with tempfile.TemporaryDirectory() as folder:
            dog = ab.Watchdog(folder, "00-unpatched", set(), now=lambda: 0.0)
            dog.count, dog.changed = 5, -100.0
            dog.samples = list(before)
            with mock.patch.object(ab, "webkit_pids", return_value=set(pids)), \
                    mock.patch.object(ab, "process_command", return_value="/x/com.apple.WebKit.WebContent.xpc"), \
                    mock.patch.object(ab.subprocess, "run", side_effect=fake_sample), \
                    mock.patch.object(os, "kill", side_effect=signalled.append), mock.patch.object(os, "killpg", side_effect=signalled.append):
                dog.observe(Lab(), 5)
        return dog

    def test_a_deadlocked_page_is_recorded_with_its_pid_role_and_sample_file_and_never_signalled(self):
        signalled = []
        dog = self.sampled(signalled)
        self.assertEqual(signalled, [])
        self.assertEqual([(s["pid"], s["role"], s["file"]) for s in dog.samples], [(4242, "WebContent.xpc", "hang-00-unpatched-4242-0.txt")])
        self.assertTrue(dog.samples[0]["signature"]["deadlock"])

    def test_one_observation_samples_at_most_the_cap_however_many_helpers_it_finds(self):
        dog = self.sampled([], pids=(4242, 4243, 4244, 4245))
        self.assertEqual([s["pid"] for s in dog.samples], [4242, 4243], "the cap is checked for each helper, not only before the loop")
        self.assertEqual(len(dog.samples), ab.MAX_SAMPLES_PER_SESSION)

    def test_a_session_that_already_has_a_sample_takes_only_what_is_left_of_the_cap(self):
        dog = self.sampled([], pids=(4242, 4243, 4244), before=[{"pid": 1}])
        self.assertEqual([s.get("pid") for s in dog.samples], [1, 4242])
        full = self.sampled([], pids=(4242,), before=[{}] * ab.MAX_SAMPLES_PER_SESSION)
        self.assertEqual(len(full.samples), ab.MAX_SAMPLES_PER_SESSION, "a session at the cap takes no more")

    def test_the_module_has_nothing_that_can_signal_a_process(self):
        source = inspect.getsource(ab)
        for forbidden in ("os.kill", "os.killpg", "SIGKILL", "SIGTERM", "signal.", "killed_deadlocked_helpers"):
            self.assertFalse(forbidden in source, f"the runner mentions `{forbidden}`")
        self.assertFalse(hasattr(ab, "reap"))
        self.assertFalse(hasattr(ab.Watchdog(".", "x", set()), "deadlocked"))

    def test_the_row_of_a_session_carries_the_sample_it_took(self):
        session = record(ab.UNPATCHED, first=pictures(1, 1000.0, 20))
        session["hang_samples"] = [{"at_s": 31.0, "pid": 7, "role": "WebContent.xpc", "file": "x.txt", "signature": ab.deadlock_signature(DEADLOCK_SAMPLE)}]
        self.assertEqual({key: ab.session_row(session, PROJECT)["samples"][0][key] for key in ("pid", "role", "file")},
                         {"pid": 7, "role": "WebContent.xpc", "file": "x.txt"})


class ScratchRuntimeTests(unittest.TestCase):
    def test_it_makes_a_folder_the_host_accepts_with_the_arms_bundle_and_the_page_it_is_given(self):
        with tempfile.TemporaryDirectory() as parent:
            bundle, index = Path(parent) / "a.js", Path(parent) / "index.html"
            bundle.write_text("// the arm's bundle")
            index.write_text("<!-- a page -->")
            directory = ab.scratch_runtime(parent, ab.UNPATCHED, bundle, index)
            self.assertEqual((directory / "dist" / "runtime.js").read_text(), "// the arm's bundle")
            self.assertEqual((directory / "dist" / "index.html").read_text(), "<!-- a page -->")


class BundleGateTests(unittest.TestCase):
    """What the run says before it starts when the two bundles are not the arms they stand for. Nothing here reads the repository's own `runtime/dist`:
    a folder of the test stands in for it, so these hold however that was built."""
    PLAIN = f"a();{ab.DECODER_CALL}b();"
    PATCHED_TEXT = "a();void 0;b();"

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        (self.root / "runtime" / "dist").mkdir(parents=True)
        patcher = mock.patch.object(ab, "ROOT", self.root)
        patcher.start()
        self.addCleanup(patcher.stop)

    def gate(self, dist, unpatched):
        (self.root / "runtime" / "dist" / "runtime.js").write_text(dist)
        given = self.root / "unpatched.js"
        given.write_text(unpatched)
        return ab.bundle_gate(SimpleNamespace(unpatched_bundle=str(given)))

    def test_a_plain_build_in_dist_is_refused_as_the_patched_arm_and_the_message_says_how_to_build_it(self):
        why = self.gate(self.PLAIN, self.PLAIN)
        self.assertIn("still has", why)
        self.assertIn("TESSEL_RUNTIME_SPARK_RELEASE_LOCK_PATCH=1 ./scripts/runtime build", why)
        self.assertIn("opt-in", why)

    def test_a_patched_dist_and_a_plain_unpatched_bundle_are_the_two_arms(self):
        self.assertIsNone(self.gate(self.PATCHED_TEXT, self.PLAIN))

    def test_the_arms_given_the_wrong_way_round_are_refused_whichever_side_is_wrong(self):
        swapped = self.gate(self.PLAIN, self.PATCHED_TEXT)
        self.assertIn("unpatched bundle has the decoder's", swapped)
        self.assertIn("0 time(s)", swapped)
        self.assertIn("plain `./scripts/runtime build`", swapped)
        self.assertIn("unpatched bundle", self.gate(self.PATCHED_TEXT, self.PATCHED_TEXT))

    def test_the_run_starts_only_when_the_gate_lets_it_and_says_blocked_with_the_reason(self):
        (self.root / "runtime" / "dist" / "index.html").write_text("")
        (self.root / "runtime" / "dist" / "runtime.js").write_text(self.PLAIN)
        given = self.root / "unpatched.js"
        given.write_text(self.PLAIN)
        args = SimpleNamespace(unpatched_bundle=str(given), binary=str(given), project=str(given), out=str(self.root / "out"), lock_wait=0.0, pairs=1,
                               stress_cpu=0, report_dir=None, label="")
        shown = io.StringIO()
        proof = mock.Mock()
        proof.free_gib.return_value = 100.0
        with mock.patch("ab.batch.warm_proof", return_value=proof), mock.patch("ab.batch.parse_project", return_value=PROJECT), \
                mock.patch("ab.batch.check_sources", return_value=[]), contextlib.redirect_stdout(shown):
            code = ab.cmd_run(args)
        self.assertEqual(code, 2)
        self.assertIn("BLOCKED: the patched bundle still has", shown.getvalue())
        self.assertIn("TESSEL_RUNTIME_SPARK_RELEASE_LOCK_PATCH=1", shown.getvalue())
        proof.acquire_locks.assert_not_called()


def host_event(asset, status, size):
    return {"kind": "asset_io", "t_ms": 1.0, "seq": 3, "fields": {"asset": asset, "status": status, "bytes": size}}


def served(arm, size):
    return {"arm": arm, "served_runtime_bytes": size}


UNPATCHED_BYTES, PATCHED_BYTES = 4281059, 4281045  # the two real bundles: the patch removes 14 bytes


class ServedBundleTests(unittest.TestCase):
    def test_the_bytes_are_the_hosts_own_record_of_serving_the_page(self):
        events = [host_event("/asset/x", 200, 5), host_event("/runtime.js", 200, UNPATCHED_BYTES)]
        self.assertEqual(ab.served_runtime_bytes(events), UNPATCHED_BYTES)

    def test_a_refused_or_other_response_is_not_the_bundle(self):
        self.assertIsNone(ab.served_runtime_bytes([host_event("/runtime.js", 404, 0), host_event("/index.html", 200, 9)]))
        self.assertIsNone(ab.served_runtime_bytes([]))
        self.assertIsNone(ab.served_runtime_bytes(None))

    def test_a_session_row_carries_what_its_page_was_served(self):
        session = record(ab.PATCHED)
        session["events"] = [host_event("/runtime.js", 200, PATCHED_BYTES)] + session["events"]
        self.assertEqual(ab.session_row(session, PROJECT)["served_runtime_bytes"], PATCHED_BYTES)

    def test_arms_that_were_served_their_own_bundles_pass(self):
        check = ab.served_bundle_check([served(ab.UNPATCHED, UNPATCHED_BYTES)] * 3 + [served(ab.PATCHED, PATCHED_BYTES)] * 3)
        self.assertEqual(check["findings"], [])
        self.assertEqual(check["bytes"], {"unpatched": [UNPATCHED_BYTES], "patched": [PATCHED_BYTES]})

    def test_both_arms_served_the_same_bundle_is_a_finding(self):
        check = ab.served_bundle_check([served(ab.UNPATCHED, PATCHED_BYTES)] * 2 + [served(ab.PATCHED, PATCHED_BYTES)] * 2)
        self.assertIn("differ by 0 bytes, the patch removes 14", check["findings"][0])

    def test_arms_that_differ_by_something_other_than_the_patch_are_a_finding(self):
        check = ab.served_bundle_check([served(ab.UNPATCHED, PATCHED_BYTES + 55), served(ab.PATCHED, PATCHED_BYTES)])
        self.assertIn("differ by 55 bytes", check["findings"][0])

    def test_an_arm_served_two_different_bundles_is_a_finding(self):
        check = ab.served_bundle_check([served(ab.UNPATCHED, UNPATCHED_BYTES), served(ab.UNPATCHED, PATCHED_BYTES), served(ab.PATCHED, PATCHED_BYTES)])
        self.assertIn("unpatched sessions were served 2 different", check["findings"][0])

    def test_the_size_of_the_arms_file_is_checked_when_the_manifest_has_it(self):
        rows = [served(ab.UNPATCHED, UNPATCHED_BYTES), served(ab.PATCHED, PATCHED_BYTES)]
        self.assertEqual(ab.served_bundle_check(rows, {"unpatched": UNPATCHED_BYTES, "patched": PATCHED_BYTES})["findings"], [])
        wrong = ab.served_bundle_check(rows, {"unpatched": UNPATCHED_BYTES, "patched": 4281000})
        self.assertIn(f"patched sessions were served {PATCHED_BYTES} bytes, the arm's bundle has 4281000", wrong["findings"][0])

    def test_a_session_that_ran_without_the_record_is_a_finding_because_its_bundle_is_not_verified(self):
        check = ab.served_bundle_check([served(ab.UNPATCHED, UNPATCHED_BYTES), served(ab.PATCHED, PATCHED_BYTES), served(ab.PATCHED, None)])
        self.assertEqual(check["unverified"], 1)
        self.assertEqual(check["findings"], ["1 session(s) that ran have no record of the runtime.js the host served: the bundle of their arm is not verified"])

    def test_a_failed_session_without_the_record_is_only_unverified_it_may_have_died_before_the_page(self):
        died = {**served(ab.PATCHED, None), "failed": True}
        check = ab.served_bundle_check([served(ab.UNPATCHED, UNPATCHED_BYTES), served(ab.PATCHED, PATCHED_BYTES), died])
        self.assertEqual((check["findings"], check["unverified"]), ([], 1))

    def test_unverified_arms_never_confirm_the_patch_and_the_sessions_stay_listed(self):
        # Three hung unpatched sessions and no hung patched one is CONFIRMED by the rules, unless the arms' bundles were not seen to be served.
        records = [record(ab.UNPATCHED, first=pictures(1, 1000.0, 20)) for _ in range(3)] + [record(ab.PATCHED) for _ in range(3)]
        manifest = {"git": {"sha": "0" * 40, "dirty_files": 0}, "binary_sha256": "b", "document_sha256": "d", "stress_cpu": 0, "order": [],
                    "bundle_sha256": {"unpatched": "u", "patched": "p"}}
        bare = ab.build_report(records, PROJECT, manifest)
        self.assertTrue(bare["verdict"].startswith("INVALID ARMS"), bare["verdict"])
        self.assertIn("the rules alone would say: CONFIRMED", bare["verdict"])
        self.assertFalse(bare["all_ran"])
        self.assertEqual(len(bare["rows"]), 6, "every session keeps its row")
        for session in records:  # the same sessions with the host's own record of what it served
            size = UNPATCHED_BYTES if session["arm"] == ab.UNPATCHED else PATCHED_BYTES
            session["events"] = [host_event("/runtime.js", 200, size)] + session["events"]
        seen = ab.build_report(records, PROJECT, manifest)
        self.assertTrue(seen["verdict"].startswith("CONFIRMED"), seen["verdict"])
        self.assertTrue(seen["all_ran"])

    def test_the_report_calls_the_run_invalid_when_the_arms_are_not_what_they_say(self):
        records = [record(ab.UNPATCHED), record(ab.PATCHED)]
        for session in records:
            session["events"] = [host_event("/runtime.js", 200, PATCHED_BYTES)] + session["events"]  # both pages got the patched bundle
        manifest = {"git": {"sha": "0" * 40, "dirty_files": 0}, "binary_sha256": "b", "document_sha256": "d", "stress_cpu": 0, "order": [],
                    "bundle_sha256": {"unpatched": "u", "patched": "p"}}
        report = ab.build_report(records, PROJECT, manifest)
        self.assertTrue(report["verdict"].startswith("INVALID ARMS"))
        self.assertFalse(report["all_ran"])
        self.assertIn(str(PATCHED_BYTES), ab.markdown(report))


def play(max_gap, slow=0, very_slow=0, where=()):
    return {"pictures": 600, "max_gap_ms": max_gap, "hung": False, "slow_gt_62_5": slow, "slow_gt_100": very_slow, "slow_where": list(where)}


def row_of(arm, **plays):
    return {"arm": arm, "plays": plays}


class PlayGapTests(unittest.TestCase):
    def test_a_session_row_carries_the_slow_gaps_of_each_play_and_where_they_were(self):
        result = ab.session_row(record(ab.PATCHED, repeat=pictures(1000, 20000.0, 100, stall_after=50, stall_ms=5000.0)), PROJECT)
        self.assertEqual((result["plays"]["first"]["slow_gt_62_5"], result["plays"]["first"]["slow_where"]), (0, []))
        self.assertEqual((result["plays"]["repeat"]["slow_gt_62_5"], result["plays"]["repeat"]["slow_gt_100"]), (1, 1))
        self.assertEqual(result["plays"]["repeat"]["slow_where"], ["other"])

    def test_the_largest_wait_of_each_play_is_summarised_per_arm(self):
        rows = [row_of(ab.UNPATCHED, first=play(60.0), repeat=play(70.0)), row_of(ab.PATCHED, first=play(65.0), repeat=play(120.0))]
        summary = ab.gap_summary(rows)
        self.assertEqual((summary["unpatched"]["plays"], summary["unpatched"]["max_gap_ms"]["max"]), (2, 70.0))
        self.assertEqual((summary["patched"]["plays"], summary["patched"]["max_gap_ms"]["max"]), (2, 120.0))

    def test_slow_gaps_are_summed_and_the_plays_with_a_long_wait_counted(self):
        rows = [row_of(ab.UNPATCHED, first=play(70.0, 2, 0), repeat=play(150.0, 3, 1)), row_of(ab.UNPATCHED, first=play(110.0, 1, 1))]
        summary = ab.gap_summary(rows)["unpatched"]
        self.assertEqual((summary["slow_gt_62_5"], summary["slow_gt_100"], summary["plays_with_gap_gt_100"]), (6, 2, 2))

    def test_a_location_counts_the_plays_that_had_a_slow_gap_there_most_often_first(self):
        rows = [row_of(ab.PATCHED, first=play(70.0, 1, 0, ["cut:186", "prefetch:268"]), repeat=play(70.0, 2, 0, ["cut:186", "other", "prefetch:268"])),
                row_of(ab.PATCHED, first=play(70.0, 1, 0, ["prefetch:268"]))]
        found = ab.gap_summary(rows)["patched"]["plays_with_a_slow_gap_at"]
        self.assertEqual(found, {"prefetch:268": 3, "cut:186": 2, "other": 1})
        self.assertEqual(list(found), ["prefetch:268", "cut:186", "other"], "most plays first, not alphabetical")

    def test_an_arm_with_no_play_summarises_to_nothing(self):
        summary = ab.gap_summary([row_of(ab.UNPATCHED, first=play(60.0))])["patched"]
        self.assertEqual((summary["plays"], summary["max_gap_ms"]["max"], summary["plays_with_a_slow_gap_at"]), (0, None, {}))

    def test_the_markdown_shows_the_plays_of_each_arm(self):
        records = [record(ab.UNPATCHED), record(ab.PATCHED, repeat=pictures(1000, 20000.0, 100, stall_after=50, stall_ms=5000.0))]
        manifest = {"git": {"sha": "0" * 40, "dirty_files": 0}, "binary_sha256": "b", "document_sha256": "d", "stress_cpu": 0, "order": [],
                    "bundle_sha256": {"unpatched": "u", "patched": "p"}}
        text = ab.markdown(ab.build_report(records, PROJECT, manifest))
        self.assertIn("What the plays looked like", text)
        self.assertIn("patched: plays with a slow gap (> 62.5 ms) at each location: other 1", text)


if __name__ == "__main__":
    unittest.main()
