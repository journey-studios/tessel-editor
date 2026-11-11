"""Pure-logic tests of the scene warm A/B harness; no editor, GPU or runtime page is started."""
import fcntl
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("scene_warm_proof", Path(__file__).with_name("scene-warm-proof.py"))
proof = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proof)


def shot(id, shown, done, latency=10.0):
    return {"id": id, "shown_frame": shown, "done_ms": done, "submit_to_done_ms": latency, "runtime_total_ms": latency / 2}


class FixtureTests(unittest.TestCase):
    def test_a_cloud_is_whole_splats_and_is_reproducible_from_its_seed(self):
        first = proof.splat_bytes(50, 7)
        self.assertEqual(len(first), 50 * 32)
        self.assertEqual(first, proof.splat_bytes(50, 7))
        self.assertNotEqual(first, proof.splat_bytes(50, 8), "another cut has another cloud, or the pixel control is vacuous")

    def test_the_arms_alternate_and_stay_balanced(self):
        self.assertEqual(proof.arm_order(1), ["warm", "cold"])
        order = proof.arm_order(4)
        self.assertEqual(order, ["warm", "cold", "cold", "warm", "warm", "cold", "cold", "warm"])
        self.assertEqual(order.count("warm"), order.count("cold"))


OPENING = [shot(90, 86, -3000), shot(91, 0, -1800)]


class TraceTests(unittest.TestCase):
    def test_overlapping_trace_windows_count_each_picture_once(self):
        seen = {}
        proof.merge_frames(seen, [shot(1, 0, 0), shot(2, 1, 40)])
        proof.merge_frames(seen, [shot(2, 1, 40), shot(3, 2, 83)])
        proof.merge_frames(seen, None)
        self.assertEqual(sorted(seen), [1, 2, 3])

    def test_the_entry_is_the_first_picture_of_the_cut_and_its_gap_is_to_the_picture_before(self):
        shots = OPENING + [shot(1, 70, 0), shot(2, 71, 40), shot(3, 72, 140, latency=95.0), shot(4, 73, 180)]
        entry = proof.entry_report(shots, 72)
        self.assertEqual((entry["shown_frame"], entry["latency_ms"], entry["gap_ms"]), (72, 95.0, 100.0))

    def test_a_cut_the_clock_skipped_is_entered_by_the_next_picture_inside_the_window(self):
        shots = OPENING + [shot(1, 71, 0), shot(2, 74, 120, latency=80.0)]
        self.assertEqual(proof.entry_report(shots, 72)["shown_frame"], 74)

    def test_an_entry_with_no_picture_is_missing_and_never_invented(self):
        shots = OPENING + [shot(1, 60, 0), shot(2, 100, 900)]
        self.assertEqual(proof.entry_report(shots, 72), {"cut": 72, "missing": True})

    def test_slow_intervals_are_the_ones_above_a_frame_and_a_half(self):
        shots = OPENING + [shot(10 + i, 1 + i, done) for i, done in enumerate([0, 41, 82, 150, 190, 232])]
        summary = proof.run_summary({s["id"]: s for s in shots}, [0], [(0, 100.0), (1, 140.0), (2, 120.0)], {"skipped_frames": 2, "max_gap_frames": 3})
        self.assertEqual(summary["slow_intervals"], 1, "only the 68 ms gap")
        self.assertEqual(summary["worst_interval_ms"], 68)
        self.assertEqual(summary["memory_mib"], {"samples": 3, "median": 120.0, "peak": 140.0})
        self.assertEqual((summary["skipped_frames"], summary["max_gap_frames"]), (2, 3))

    def test_the_silence_between_the_opening_pictures_and_the_first_tick_is_not_a_stall_of_the_play(self):
        shots = [shot(0, 86, -3000), shot(1, 0, -1700), shot(2, 0, -1500), shot(3, 1, 0), shot(4, 2, 40), shot(5, 3, 80)]
        self.assertEqual(proof.intervals(shots), [40.0, 40.0])
        self.assertEqual(proof.start_ms(shots), 1500.0, "reported on its own: the playback clock starting")
        self.assertEqual(proof.intervals([shot(0, 0, 0)]), [])
        self.assertIsNone(proof.start_ms([shot(0, 0, 0)]))

    def test_a_picture_before_the_first_tick_is_never_an_entry(self):
        # The project opened at frame 86; that picture is not the entry of a cut at 84.
        shots = OPENING + [shot(1, 1, 0), shot(2, 2, 40)]
        self.assertEqual(proof.entry_report(shots, 84), {"cut": 84, "missing": True})

    def test_a_slow_interval_is_located_by_the_frames_around_it(self):
        shots = OPENING + [shot(10, 1, 100), shot(11, 2, 141), shot(12, 3, 400), shot(13, 4, 441)]
        self.assertEqual(proof.slow_detail(shots), [(2, 3, 259.0)])
        self.assertEqual(proof.run_summary({s["id"]: s for s in shots}, [0], [], None)["slow_detail"], [(2, 3, 259.0)])

    def test_a_peak_above_the_median_is_reported_as_both(self):
        series = [(i, value) for i, value in enumerate([100.0, 100.0, 100.0, 400.0])]
        memory = proof.run_summary({}, [0], series, None)["memory_mib"]
        self.assertEqual((memory["median"], memory["peak"]), (100.0, 400.0))

    def test_the_medians_skip_a_run_that_has_no_such_entry(self):
        runs = [{"summary": {"entries": [{"latency_ms": 20.0}, {"missing": True}]}},
                {"summary": {"entries": [{"latency_ms": 60.0}, {"latency_ms": 5.0}]}}]
        out = proof.medians(runs, [0, 72])
        self.assertEqual((out[0]["runs"], out[0]["median_ms"], out[0]["max_ms"]), (2, 40.0, 60.0))
        self.assertEqual((out[1]["runs"], out[1]["median_ms"]), (1, 5.0))


class ValidityTests(unittest.TestCase):
    def test_a_warm_run_that_never_warmed_measured_the_other_arm(self):
        self.assertFalse(proof.run_is_valid("warm", {"warmed": 0})[0])
        self.assertTrue(proof.run_is_valid("warm", {"warmed": 3})[0])

    def test_a_cold_run_that_warmed_is_invalid_and_so_is_one_without_the_counter(self):
        self.assertFalse(proof.run_is_valid("cold", {"warmed": 1})[0])
        self.assertTrue(proof.run_is_valid("cold", {"warmed": 0})[0])
        self.assertFalse(proof.run_is_valid("cold", {})[0])
        self.assertFalse(proof.run_is_valid("warm", None)[0])

    def test_the_analysis_leaves_an_invalid_run_out_and_says_so(self):
        entries = [{"latency_ms": 10.0}]
        memory = {"median": 100.0, "peak": 120.0}
        good = {"arm": "warm", "valid": True, "summary": {"entries": entries, "memory_mib": memory, "slow_intervals": 0, "worst_interval_ms": 50}}
        bad = {"arm": "warm", "valid": False, "summary": {"entries": [{"latency_ms": 999.0}], "memory_mib": memory, "slow_intervals": 9, "worst_interval_ms": 900}}
        report = proof.analysis([good, bad], [0])
        self.assertEqual(report["arms"]["warm"]["valid_runs"], 1)
        self.assertEqual(report["arms"]["warm"]["entries"][0]["max_ms"], 10.0)
        self.assertEqual(report["arms"]["cold"]["valid_runs"], 0)


class PixelTests(unittest.TestCase):
    @staticmethod
    def session(arm, order, hashes, monitor=None):
        return {"arm": arm, "order": order,
                "hashes": {f: {"hash": h, "inspected": True} for f, h in hashes.items()},
                "monitor": {f: {"hash": (monitor or hashes)[f], "converged": True} for f in hashes}}

    def verdicts(self, sessions, frames):
        return {name: ok for name, ok, _ in proof.pixel_verdicts(sessions, frames)}

    def test_identical_hashes_in_all_four_orders_and_distinct_cuts_pass(self):
        sessions = [self.session(a, o, {1: "x", 2: "y"}) for a, o in (("warm", "hot"), ("cold", "hot"), ("warm", "cold"), ("cold", "cold"))]
        self.assertTrue(all(self.verdicts(sessions, [1, 2]).values()))

    def test_one_arm_drawing_a_different_picture_fails_that_frame(self):
        sessions = [self.session("warm", "hot", {1: "x", 2: "y"}), self.session("cold", "hot", {1: "x", 2: "z"})]
        verdicts = self.verdicts(sessions, [1, 2])
        self.assertTrue(verdicts["frame 1: the explicit render is byte-identical in warm/cold x hot/cold-seek"])
        self.assertFalse(verdicts["frame 2: the explicit render is byte-identical in warm/cold x hot/cold-seek"])

    def test_a_stuck_or_blank_picture_shared_by_two_cuts_fails_the_control(self):
        sessions = [self.session("warm", "hot", {1: "blank", 2: "blank"})]
        self.assertFalse(self.verdicts(sessions, [1, 2])["control: the frames of two different cuts do not share a picture"])

    def test_a_missing_hash_never_passes(self):
        sessions = [self.session("warm", "hot", {1: None})]
        self.assertFalse(self.verdicts(sessions, [1])["frame 1: the explicit render is byte-identical in warm/cold x hot/cold-seek"])

    def test_the_monitor_picture_after_the_pause_must_be_the_explicit_one(self):
        sessions = [self.session("warm", "hot", {1: "x"}, monitor={1: "stale-order"})]
        self.assertFalse(self.verdicts(sessions, [1])["after the pause the monitor shows the explicit render's picture"])


class SpanTests(unittest.TestCase):
    """What the page says it did reaches a proof through the host's trace; these drive the collection with a lab that answers from a script."""

    class Lab:
        def __init__(self, *answers):
            self.answers, self.asked = list(answers), []

        def op(self, request, timeout=60):
            self.asked.append(request)
            return self.answers.pop(0)

    @staticmethod
    def trace(*events, next=None):
        return {"ok": True, "data": {"events": list(events), "next": next if next is not None else len(events)}}

    @staticmethod
    def span(**fields):
        return {"kind": "runtime_span", "seq": 1, "t_ms": 1.0, "fields": fields}

    def test_only_the_pages_spans_are_kept_and_the_cursor_moves_past_everything_read(self):
        lab = self.Lab(self.trace({"kind": "frame_page", "fields": {}}, self.span(k="warm_draw", outcome="drawn", ms=4.0), next=7))
        spans = []
        self.assertEqual(proof.drain_spans(lab, 3, spans), 7)
        self.assertEqual(spans, [{"k": "warm_draw", "outcome": "drawn", "ms": 4.0}])
        self.assertEqual(lab.asked, [{"op": "render.trace", "since": 3, "limit": 16384}], "polled with the cursor, never from the start again")

    def test_a_poll_with_nothing_new_or_one_the_host_refused_keeps_the_cursor_and_loses_nothing(self):
        spans = [{"k": "warm_draw", "outcome": "drawn", "ms": 1.0}]
        self.assertEqual(proof.drain_spans(self.Lab(self.trace(next=9)), 5, spans), 5)
        self.assertEqual(proof.drain_spans(self.Lab({"ok": False, "error": {"code": "busy"}}), 5, spans), 5)
        self.assertEqual(proof.drain_spans(self.Lab(None), 5, spans), 5)
        self.assertEqual(len(spans), 1)

    def test_counts_are_by_kind_and_outcome_and_a_span_without_an_outcome_is_not_lost(self):
        counts = proof.span_counts([
            {"k": "warm_draw", "outcome": "drawn", "ms": 4.0}, {"k": "warm_draw", "outcome": "drawn", "ms": 5.0},
            {"k": "warm_draw", "outcome": "taken", "ms": 0.0}, {"k": "warm", "ms": 3.0, "started": 1},
        ])
        self.assertEqual(counts, {"warm_draw": {"drawn": 2, "taken": 1}, "warm": {"-": 1}})
        self.assertEqual(proof.span_counts([]), {})


class JobTests(unittest.TestCase):
    def test_a_succeeded_job_whose_result_the_window_has_not_applied_is_not_settled(self):
        self.assertFalse(proof.job_settled({"state": "running"}))
        self.assertFalse(proof.job_settled({"state": "succeeded", "result_state": "pending"}))
        self.assertFalse(proof.job_settled({"state": "succeeded"}))
        self.assertTrue(proof.job_settled({"state": "succeeded", "result_state": "applied"}))
        self.assertTrue(proof.job_settled({"state": "failed"}), "a failure is an answer")

    def test_the_wait_asks_again_until_the_result_is_applied(self):
        class Lab:
            answers = iter([{"state": "succeeded", "result_state": "pending"}, {"state": "succeeded", "result_state": "applied"}])

            def op(self, request):
                return {"data": next(self.answers)}

        self.assertEqual(proof.wait_job(Lab(), "j", sleep=lambda s: None)["result_state"], "applied")


class LockTests(unittest.TestCase):
    def test_free_locks_are_taken_and_a_held_one_is_never_waited_for_past_the_deadline(self):
        with tempfile.TemporaryDirectory() as directory:
            paths = [str(Path(directory) / "a.lock"), str(Path(directory) / "b.lock")]
            held = proof.acquire_locks(0, paths)
            self.assertEqual(len(held), 2)
            clock = [0.0]
            ticks = []

            def sleep(seconds):
                ticks.append(seconds)
                clock[0] += seconds

            blocked = proof.acquire_locks(3.0, paths, sleep=sleep, now=lambda: clock[0])
            self.assertIsNone(blocked, "the same file locked twice from another handle is busy")
            self.assertEqual(sum(ticks), 3.0)
            for handle in held:
                fcntl.flock(handle, fcntl.LOCK_UN)
                handle.close()
            again = proof.acquire_locks(0, paths)
            self.assertEqual(len(again), 2, "a failed attempt leaves nothing held")
            for handle in again:
                handle.close()

    def test_preparing_is_retried_until_it_answers_and_then_given_up(self):
        answers = iter([{"ok": False, "error": {"code": "media_preparing"}}, {"ok": True}])
        self.assertTrue(proof.retry_while_preparing(lambda: next(answers), sleep=lambda s: None)["ok"])
        clock = [0.0]

        def tick():
            clock[0] += 10
            return clock[0]

        stuck = proof.retry_while_preparing(lambda: {"ok": False, "error": {"code": "media_preparing"}}, timeout=25, sleep=lambda s: None, now=tick)
        self.assertEqual(proof.error_code(stuck), "media_preparing", "a wait that never ends is the caller's failure, not a pass")


if __name__ == "__main__":
    unittest.main()
