"""Pure-logic tests of the BL157.6 playback poll A/B; no editor, GPU, runtime page or private media is started."""
import contextlib
import importlib.util
import inspect
import io
import json
import re
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
spec = importlib.util.spec_from_file_location("playback_poll_ab", HERE / "playback-poll-ab.py")
ab = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ab)
from ab import batch as shared_batch, play as shared_play  # noqa: E402  (the runner put `scripts/` on the path)

# A private path and a private file name that must never reach a report.
PROJECT = {"length": 600, "fps": 24.0, "cuts": [186, 268, 410], "video_starts": [], "sources": [{"path": "/private/not-published.mov", "kind": "video"}]}

POLL_RS = ROOT / "crates" / "desktop" / "src" / "workstation" / "playback_poll.rs"

# What the 72 native sessions looked like, and what the model says the deadline schedule does.
FIXED_PLAY = {"delivered": 400, "under": 52, "slow": 4, "very_slow": 1, "changes": 280, "polls": 1.87}

DEADLINE_PLAY = {"delivered": 400, "under": 0, "slow": 4, "very_slow": 1, "changes": 280, "polls": 2.0}


def play(delivered=400, under=0, slow=0, very_slow=0, changes=280, polls=2.0, lag=2.5, lag_n=None, skipped=0, pictures=None, max_gap=70.0):
    """One play as `session_row` stores it: what `playback-cadence.py` read of the trace and what the harness's own analysis says of the pictures."""
    lag_n = changes if lag_n is None else lag_n
    return {"session": "run.json", "poll": None, "fps": 24.0, "ticks": changes * 2, "frame_changes": changes, "polls_per_frame_change": polls,
            "delivered": delivered, "under_30_ms": under, "over_62_5_ms": slow, "over_100_ms": very_slow, "max_delivered_ms": max_gap,
            "change_lag_ms": {"n": lag_n, "mean": lag if lag_n else None}, "poll_ms": {"n": changes * 2}, "skipped_frames": skipped,
            "pictures": delivered + 1 if pictures is None else pictures, "max_gap_ms": max_gap, "hung": False}


def row(arm, pair, plays=None, *, counted=True, hung=False, failed=False, findings=(), reasons=(), error=None):
    plays = plays if plays is not None else [play(), play()]
    return {"arm": arm, "pair": pair, "order_index": pair * 2 + (0 if arm == ab.FIXED else 1), "valid": counted or bool(findings), "invalid_reasons": list(reasons),
            "arm_findings": list(findings), "plays": dict(zip(ab.PLAYS, plays)), "hung": hung, "failed": failed, "counted": counted, "error": error,
            "quiet_class": "quiet", "external_median_cores": 0.1, "external_p95_cores": 0.2, "loadavg_start": [1.0, 1.0, 1.0], "loadavg_peak_1m": 1.5}


def scenario(pairs=12, fixed=None, deadline=None, lags=None):
    """The rows of a run: `fixed` / `deadline` override the keyword arguments of `play` for each arm; `lags` is [(fixed lag, deadline lag)] by pair."""
    rows = []
    for pair in range(pairs):
        fixed_lag, deadline_lag = lags[pair] if lags else (11.0, 2.5)
        for arm, base, extra, lag in ((ab.FIXED, FIXED_PLAY, fixed, fixed_lag), (ab.DEADLINE, DEADLINE_PLAY, deadline, deadline_lag)):
            kwargs = {**base, **(extra or {}), "lag": lag}
            rows.append(row(arm, pair, [play(**kwargs), play(**kwargs)]))
    return rows


def summary(**changes):
    """An arm's summary as `arm_summary` makes it, for the thresholds that read it."""
    base = {"sessions": 24, "counted": 24, "hung": 0, "failed": 0, "plays": 48, "delivered": 19200, "under_30_ms": 0, "slow_gt_62_5": 170, "slow_gt_100": 10,
            "under_30_share": 0.0, "slow_share": 0.0089, "skipped_per_play": 0.2, "change_lag_mean_ms": 2.5, "polls_per_frame_change": 2.0,
            "under_30_share_per_play": None}
    return {**base, **changes}


def trace_play(start, poll, picture_gaps, fps=24.0, ticks=21):
    """The events and pictures of one play, in the trace's own shape: a frame change every second poll of 22.2 ms, the clock 0.05 of a frame past each boundary."""
    events = [{"seq": int(start), "t_ms": start, "kind": "play_clock_started", "fields": {"from": 0, "fps": fps, "poll": poll}}]
    for index in range(1, ticks + 1):
        at = start + index * 22.2
        before, after = (index - 1) // 2, index // 2
        clock = after + 0.05 if after != before else before + 0.55
        events.append({"seq": int(at), "t_ms": at, "kind": "tick", "fields": {"due_ms": at - 1.3, "ran_ms": at, "late_ms": 1.3, "body_ms": 0.1, "frame_before": before,
                                                                               "frame_after": after, "continues": True, "clock": clock, "next_ms": 22.2}})
    done, frames = start + 10.0, []
    for gap in [0.0] + list(picture_gaps):
        done += gap
        frames.append({"id": len(frames), "done_ms": done, "interactive": True})
    return events, frames


def record(arm, pair=0, polls=None, gaps=(22.0, 44.0, 44.0, 105.0), error=None, plays=2):
    """A session record with `plays` plays, each naming `polls` (default: the arm) as its schedule."""
    events, frames, start = [], [], 1000.0
    for _ in range(plays):
        more_events, more_frames = trace_play(start, polls if polls is not None else arm, gaps)
        events += more_events
        frames += more_frames
        start += 5000.0
    return {"arm": arm, "pair": pair, "order_index": pair * 2, "number": 0, "mode": "sync", "events": events, "frames": frames, "error": error}


def analysis(valid=True, reasons=(), first=(590, 80.0), repeat=(590, 80.0)):
    """What `real-latency-ab.py` says of a session, with only the fields `session_row` reads."""
    def of(found):
        return None if found is None else {"pictures": found[0], "interval_ms": {"max": found[1]}}
    return {"valid": valid, "invalid_reasons": list(reasons), "quiet_class": "quiet", "external_median_cores": 0.1, "external_p95_cores": 0.3,
            "loadavg_start": [1.0, 1.0, 1.0], "loadavg_peak_1m": 2.0, "plays": {name: of(found) for name, found in (("first", first), ("repeat", repeat)) if found is not None}}


def manifest(pairs=2):
    return {"git": {"sha": "a" * 40, "dirty_files": 3}, "binary_sha256": "b" * 64, "document_sha256": "d" * 64, "pairs": pairs, "order": ab.arm_order(ab.ARMS, pairs),
            "io": "sync", "stress_cpu": 0}


class ArmTests(unittest.TestCase):
    def test_the_order_alternates_which_arm_goes_first_by_pair(self):
        self.assertEqual(ab.arm_order(ab.ARMS, 3), ["fixed", "deadline", "deadline", "fixed", "fixed", "deadline"])
        self.assertEqual(ab.arm_order(ab.ARMS, 0), [])

    def test_the_arms_differ_only_in_the_poll_switch(self):
        fixed, deadline = ab.arm_env("fixed"), ab.arm_env("deadline")
        self.assertEqual(set(fixed), set(deadline))
        self.assertEqual({key for key in fixed if fixed[key] != deadline[key]}, {"TESSEL_PLAYBACK_POLL"})
        self.assertEqual((fixed["TESSEL_PLAYBACK_POLL"], deadline["TESSEL_PLAYBACK_POLL"]), ("fixed", "deadline"))

    def test_the_arms_keep_the_control_environment_of_the_first_use_experiment(self):
        control = ab.CONTROL_ENV
        self.assertEqual((control["TESSEL_VIEWPORT_WARM_DRAW"], control["TESSEL_VIEWPORT_PREUPLOAD"]), ("0", "0"))
        for arm in ab.ARMS:
            self.assertEqual({key: value for key, value in ab.arm_env(arm).items() if key != ab.POLL_ENV}, control)

    def test_the_switch_and_its_values_are_the_ones_the_editor_reads(self):
        source = POLL_RS.read_text()
        self.assertIn(f'pub const POLL_ENV: &str = "{ab.POLL_ENV}";', source)
        for arm in ab.ARMS:
            self.assertIn(f'"{arm}"', source)

    def test_the_arms_hold_the_asset_io_mode_at_the_shipping_default(self):
        self.assertEqual(ab.IO_MODE, "sync")
        self.assertEqual(ab.ARMS, ("fixed", "deadline"))
        self.assertEqual(ab.PLAYS, ("first", "repeat"))


class ArmFindingTests(unittest.TestCase):
    def named(self, *polls, error=None, arm="deadline"):
        events = [{"kind": "play_clock_started", "t_ms": 1.0, "fields": {"from": 0, "fps": 24.0, "poll": poll}} for poll in polls]
        return {"arm": arm, "events": events, "error": error}

    def test_a_session_whose_plays_name_its_arm_has_no_finding(self):
        self.assertEqual(ab.arm_findings(self.named("deadline", "deadline")), [])
        self.assertEqual(ab.arm_findings(self.named("fixed", arm="fixed")), [])

    def test_a_play_that_names_another_schedule_is_a_finding_that_names_it(self):
        found = ab.arm_findings(self.named("deadline", "fixed"))
        self.assertEqual(len(found), 1)
        self.assertIn("fixed", found[0])
        self.assertIn("deadline arm", found[0])

    def test_a_play_that_names_nothing_is_not_the_arm(self):
        self.assertEqual(len(ab.arm_findings(self.named(None, "deadline"))), 1)

    def test_a_trace_without_any_play_is_a_finding_unless_the_session_died_first(self):
        self.assertEqual(len(ab.arm_findings(self.named())), 1)
        self.assertEqual(ab.arm_findings(self.named(error="the editor exited")), [])

    def test_a_record_without_events_is_a_finding(self):
        self.assertEqual(len(ab.arm_findings({"arm": "fixed", "error": None})), 1)


class HungTests(unittest.TestCase):
    def test_a_play_is_hung_under_the_pictures_share_or_over_the_gap(self):
        def hung(pictures, gap):
            return ab.hung_play({"pictures": pictures, "interval_ms": {"max": gap}}, 600)
        self.assertFalse(hung(540, 100.0))
        self.assertTrue(hung(539, 100.0))
        self.assertFalse(hung(600, 2000.0))
        self.assertTrue(hung(600, 2000.1))
        self.assertFalse(hung(600, None))

    def test_the_rule_is_the_one_every_experiment_uses(self):
        self.assertIs(ab.hung_play, shared_play.hung_play, "a copy of the rule in the runner could drift from the others")
        self.assertEqual((shared_play.HANG_PICTURES_SHARE, shared_play.HANG_GAP_MS), (0.9, 2000.0))


class SessionRowTests(unittest.TestCase):
    def row_of(self, rec, found=None, project=PROJECT):
        with mock.patch.object(ab, "analyze_session", return_value=found or analysis()):
            return ab.session_row(rec, project)

    def test_a_valid_session_that_played_twice_in_its_arm_is_counted(self):
        found = self.row_of(record("deadline", pair=3))
        self.assertTrue(found["counted"])
        self.assertFalse(found["failed"])
        self.assertFalse(found["hung"])
        self.assertEqual((found["arm"], found["pair"], found["arm_findings"]), ("deadline", 3, []))
        self.assertEqual(list(found["plays"]), ["first", "repeat"])

    def test_every_play_carries_what_the_cadence_reader_and_the_analysis_say_of_it(self):
        found = self.row_of(record("deadline"), analysis(first=(590, 80.0), repeat=(577, 91.5)))
        first, repeat = found["plays"]["first"], found["plays"]["repeat"]
        # The pictures 22, 44, 44 and 105 ms apart: one under 30 ms, one over 62.5 ms, one over 100 ms.
        self.assertEqual((first["delivered"], first["under_30_ms"], first["over_62_5_ms"], first["over_100_ms"]), (4, 1, 1, 1))
        self.assertEqual((first["pictures"], first["max_gap_ms"]), (590, 80.0))
        self.assertEqual((repeat["pictures"], repeat["max_gap_ms"]), (577, 91.5))
        self.assertEqual(first["poll"], "deadline")
        # Ten frame changes: nine intervals between them (the first has nothing before it to count from), ten lags.
        self.assertEqual(first["frame_changes"], 9)
        self.assertEqual(first["polls_per_frame_change"], 2.0)
        self.assertEqual(first["change_lag_ms"]["n"], 10)
        self.assertAlmostEqual(first["change_lag_ms"]["mean"], 0.05 / 24.0 * 1000.0, places=2)

    def test_the_arm_the_host_traced_is_checked_not_the_arm_asked_for(self):
        found = self.row_of(record("deadline", polls="fixed"))
        self.assertFalse(found["counted"])
        self.assertEqual(len(found["arm_findings"]), 1)

    def test_a_session_the_harness_calls_invalid_is_not_counted_and_says_why(self):
        found = self.row_of(record("fixed"), analysis(valid=False, reasons=["the scene warm did not run"]))
        self.assertFalse(found["counted"])
        self.assertFalse(found["failed"])
        self.assertFalse(found["valid"])
        self.assertEqual(found["invalid_reasons"], ["the scene warm did not run"])

    def test_a_hung_play_makes_a_hung_session_that_still_counts(self):
        found = self.row_of(record("fixed"), analysis(first=(300, 80.0)))
        self.assertTrue(found["plays"]["first"]["hung"])
        self.assertFalse(found["plays"]["repeat"]["hung"])
        self.assertTrue(found["hung"])
        self.assertFalse(found["failed"])
        self.assertTrue(found["counted"])
        slow = self.row_of(record("fixed"), analysis(repeat=(590, 2500.0)))
        self.assertTrue(slow["plays"]["repeat"]["hung"] and slow["hung"])

    def test_a_hang_is_a_result_not_a_failure_even_when_the_second_play_never_came(self):
        # The first play showed 300 of 600 pictures and the second never started: the editor hung, the run ran. The hung guard of the rule speaks to
        # it; the session is not counted because it has not its two plays, with or without the harness calling it invalid.
        lost = self.row_of(record("deadline", plays=1), analysis(first=(300, 80.0), repeat=None))
        self.assertTrue(lost["hung"])
        self.assertFalse(lost["failed"])
        self.assertFalse(lost["counted"])
        stopped = self.row_of(record("deadline", plays=1, error="the second play never started"),
                              analysis(valid=False, reasons=["a play did not finish normally"], first=(300, 80.0), repeat=None))
        self.assertTrue(stopped["hung"])
        self.assertFalse(stopped["failed"])
        self.assertFalse(stopped["counted"])
        # The same session without the hang is a failure: that is what the hang is excused from.
        self.assertTrue(self.row_of(record("deadline", plays=1), analysis(repeat=None))["failed"])

    def test_a_session_with_an_error_failed_and_its_message_has_no_path(self):
        found = self.row_of(record("fixed", error="could not open /Users/someone/private name.tessel.json: refused"))
        self.assertTrue(found["failed"])
        self.assertFalse(found["counted"])
        self.assertNotIn("someone", found["error"])
        self.assertNotIn("private name", found["error"])
        self.assertIn("refused", found["error"])

    def test_a_session_that_played_once_failed(self):
        found = self.row_of(record("deadline", plays=1), analysis(repeat=None))
        self.assertTrue(found["failed"])
        self.assertFalse(found["counted"])
        self.assertEqual(found["arm_findings"], [])

    def test_a_session_whose_analysis_has_no_second_play_failed(self):
        found = self.row_of(record("deadline"), analysis(repeat=None))
        self.assertTrue(found["failed"])
        self.assertIsNone(found["plays"].get("repeat", {}).get("pictures"))

    def test_a_session_that_died_before_playing_failed_without_blaming_the_arm(self):
        died = {"arm": "deadline", "pair": 0, "order_index": 1, "events": [], "frames": [], "error": "the window never opened"}
        found = self.row_of(died, analysis(valid=False, first=None, repeat=None))
        self.assertTrue(found["failed"])
        self.assertEqual(found["arm_findings"], [])
        self.assertFalse(found["counted"])
        self.assertEqual(found["plays"], {})

    def test_the_load_of_the_session_is_recorded_not_filtered_on(self):
        found = self.row_of(record("fixed"))
        self.assertEqual((found["quiet_class"], found["external_median_cores"], found["external_p95_cores"]), ("quiet", 0.1, 0.3))
        self.assertEqual((found["loadavg_start"], found["loadavg_peak_1m"]), ([1.0, 1.0, 1.0], 2.0))


class SummaryTests(unittest.TestCase):
    def rows(self):
        plays_a = [play(delivered=400, under=40, slow=4, very_slow=1, changes=200, polls=1.9, lag=10.0, skipped=1, max_gap=90.0),
                   play(delivered=600, under=90, slow=2, very_slow=0, changes=300, polls=1.8, lag=12.0, skipped=0)]
        plays_b = [play(delivered=1000, under=100, slow=10, very_slow=3, changes=500, polls=2.0, lag=11.0, skipped=2)] * 2
        loud = [play(delivered=10000, under=9999, slow=9999, very_slow=9999, changes=10000, polls=9.0, lag=99.0, skipped=99)] * 2
        return [row("fixed", 0, plays_a), row("fixed", 1, plays_b), row("fixed", 2, loud, counted=False, hung=True), row("fixed", 3, loud, counted=False, failed=True)]

    def test_the_sums_and_shares_are_over_the_counted_sessions_only(self):
        found = ab.arm_summary(self.rows())
        self.assertEqual((found["sessions"], found["counted"], found["hung"], found["failed"], found["plays"]), (4, 2, 1, 1, 4))
        self.assertEqual((found["delivered"], found["under_30_ms"], found["slow_gt_62_5"], found["slow_gt_100"]), (3000, 330, 26, 7))
        self.assertEqual(found["under_30_share"], 0.11)
        self.assertEqual(found["slow_share"], round(26 / 3000, 5))
        self.assertEqual(found["skipped_per_play"], 1.25)

    def test_the_lag_and_the_polls_are_weighted_by_what_they_were_measured_over(self):
        found = ab.arm_summary(self.rows())
        self.assertEqual(found["change_lag_mean_ms"], round((200 * 10.0 + 300 * 12.0 + 500 * 11.0 + 500 * 11.0) / 1500, 3))
        self.assertEqual(found["polls_per_frame_change"], round((200 * 1.9 + 300 * 1.8 + 500 * 2.0 + 500 * 2.0) / 1500, 3))

    def test_the_share_of_short_pictures_is_also_given_play_by_play(self):
        self.assertEqual(ab.arm_summary(self.rows())["under_30_share_per_play"], {"min": 0.1, "median": 0.1, "max": 0.15})

    def test_the_per_play_share_is_the_smallest_the_middle_and_the_largest_of_the_plays(self):
        # 5%, 10% and 20% of 400 pictures: the middle one is not the smallest; a fourth play (30%) makes the median the mean of the two in the middle.
        three = [row("fixed", 0, [play(delivered=400, under=20), play(delivered=400, under=40)]), row("fixed", 1, [play(delivered=400, under=80)])]
        self.assertEqual(ab.arm_summary(three)["under_30_share_per_play"], {"min": 0.05, "median": 0.1, "max": 0.2})
        four = [three[0], row("fixed", 1, [play(delivered=400, under=80), play(delivered=400, under=120)])]
        self.assertEqual(ab.arm_summary(four)["under_30_share_per_play"], {"min": 0.05, "median": 0.15, "max": 0.3})

    def test_an_arm_with_nothing_counted_has_no_shares(self):
        found = ab.arm_summary([row("fixed", 0, counted=False)])
        self.assertEqual((found["sessions"], found["counted"], found["plays"], found["delivered"]), (1, 0, 0, 0))
        for key in ("under_30_share", "slow_share", "skipped_per_play", "change_lag_mean_ms", "polls_per_frame_change", "under_30_share_per_play"):
            self.assertIsNone(found[key], key)

    def test_a_play_without_a_traced_lag_weighs_nothing_in_the_lag(self):
        rows = [row("fixed", 0, [play(lag=10.0), play(lag_n=0)])]
        self.assertEqual(ab.arm_summary(rows)["change_lag_mean_ms"], 10.0)
        self.assertIsNone(ab.arm_summary([row("fixed", 0, [play(lag_n=0), play(lag_n=0)])])["change_lag_mean_ms"])

    def test_pooled_mean_weights_by_count_and_is_none_without_any(self):
        self.assertEqual(ab.pooled_mean([(1, 10.0), (3, 20.0)]), 17.5)
        self.assertIsNone(ab.pooled_mean([]))
        self.assertIsNone(ab.pooled_mean([(0, 5.0)]))


class PairTests(unittest.TestCase):
    def test_a_pair_is_counted_when_both_of_its_sessions_are(self):
        rows = scenario(pairs=4, lags=[(11.0, 2.0), (12.0, 3.0), (13.0, 4.0), (14.0, 5.0)])
        rows[3]["counted"] = False  # the deadline session of pair 1
        rows[4]["counted"] = False  # the fixed session of pair 2
        self.assertEqual(ab.pair_lags(rows), [(0, 11.0, 2.0), (3, 14.0, 5.0)])

    def test_pairs_are_in_pair_order_whatever_the_row_order(self):
        rows = list(reversed(scenario(pairs=3, lags=[(11.0, 2.0), (12.0, 3.0), (13.0, 4.0)])))
        self.assertEqual([pair for pair, _, _ in ab.pair_lags(rows)], [0, 1, 2])

    def test_a_pair_without_a_traced_lag_on_either_side_is_not_a_pair_of_lags(self):
        rows = scenario(pairs=2)
        rows[1]["plays"] = {name: play(lag_n=0) for name in ab.PLAYS}
        self.assertEqual([pair for pair, _, _ in ab.pair_lags(rows)], [1])

    def test_a_pair_with_one_session_only_is_not_a_pair(self):
        self.assertEqual(ab.pair_lags([row("fixed", 0)]), [])

    def test_the_lag_of_a_session_pools_its_two_plays(self):
        found = row("fixed", 0, [play(changes=100, lag=10.0), play(changes=300, lag=20.0)])
        self.assertEqual(ab.lag_of_session(found), 17.5)


class RegressionTests(unittest.TestCase):
    def test_nothing_worse_is_not_a_regression(self):
        self.assertEqual(ab.regressions(summary(), summary()), [])

    def test_the_slow_share_may_not_grow_by_more_than_a_quarter_of_a_percentage_point(self):
        fixed = summary(slow_share=0.0090)
        self.assertEqual(ab.regressions(fixed, summary(slow_share=0.0090 + ab.SLOW_SHARE_MARGIN)), [])
        found = ab.regressions(fixed, summary(slow_share=0.0090 + ab.SLOW_SHARE_MARGIN + 0.0001))
        self.assertEqual(len(found), 1)
        self.assertIn("62.5", found[0])

    def test_the_count_of_very_slow_pictures_may_not_pass_one_and_a_half_times_the_fixed_arms_plus_three(self):
        fixed = summary(slow_gt_100=10)
        self.assertEqual(ab.regressions(fixed, summary(slow_gt_100=18)), [])
        found = ab.regressions(fixed, summary(slow_gt_100=19))
        self.assertEqual(len(found), 1)
        self.assertIn("100", found[0])
        self.assertEqual(ab.regressions(summary(slow_gt_100=0), summary(slow_gt_100=3)), [])
        self.assertEqual(len(ab.regressions(summary(slow_gt_100=0), summary(slow_gt_100=4))), 1)

    def test_the_count_of_very_slow_pictures_is_scaled_to_the_plays_each_arm_has(self):
        fixed = summary(slow_gt_100=10, plays=48)
        # Half the plays: the fixed arm's 10 become 5, and the limit 1.5 x 5 + 3 = 10.5.
        self.assertEqual(ab.regressions(fixed, summary(slow_gt_100=10, plays=24)), [])
        self.assertEqual(len(ab.regressions(fixed, summary(slow_gt_100=11, plays=24))), 1)
        # Twice the plays: 10 become 20, and the limit 1.5 x 20 + 3 = 33.
        self.assertEqual(ab.regressions(fixed, summary(slow_gt_100=33, plays=96)), [])
        self.assertEqual(len(ab.regressions(fixed, summary(slow_gt_100=34, plays=96))), 1)
        # No plays counted in the fixed arm: nothing to scale, the plus alone is the limit.
        self.assertEqual(ab.regressions(summary(slow_gt_100=0, plays=0), summary(slow_gt_100=3)), [])
        self.assertEqual(len(ab.regressions(summary(slow_gt_100=0, plays=0), summary(slow_gt_100=4))), 1)

    def test_skipped_frames_per_play_may_not_grow_by_more_than_fifteen_hundredths(self):
        fixed = summary(skipped_per_play=0.20)
        self.assertEqual(ab.regressions(fixed, summary(skipped_per_play=0.20 + ab.SKIPPED_PER_PLAY_MARGIN)), [])
        found = ab.regressions(fixed, summary(skipped_per_play=0.20 + ab.SKIPPED_PER_PLAY_MARGIN + 0.01))
        self.assertEqual(len(found), 1)
        self.assertIn("skipped", found[0])

    def test_the_deadline_arm_may_not_hang_more_sessions(self):
        self.assertEqual(ab.regressions(summary(hung=2), summary(hung=2)), [])
        self.assertEqual(ab.regressions(summary(hung=2), summary(hung=1)), [])
        found = ab.regressions(summary(hung=2), summary(hung=3))
        self.assertEqual(len(found), 1)
        self.assertIn("hung", found[0])

    def test_every_guard_that_fails_is_reported(self):
        worse = summary(slow_share=0.05, slow_gt_100=100, skipped_per_play=5.0, hung=9)
        self.assertEqual(len(ab.regressions(summary(), worse)), 4)

    def test_a_guard_without_numbers_on_either_side_says_nothing(self):
        self.assertEqual(ab.regressions(summary(slow_share=None, skipped_per_play=None), summary(slow_share=0.5, skipped_per_play=5.0, slow_gt_100=10)), [])
        self.assertEqual(ab.regressions(summary(slow_share=0.0089, skipped_per_play=0.2), summary(slow_share=None, skipped_per_play=None)), [])


class ShortfallTests(unittest.TestCase):
    PAIRS = [(index, 11.0, 2.5) for index in range(12)]

    def short(self, fixed=None, deadline=None, pairs=None):
        return ab.shortfalls(summary(**(fixed or {})), summary(**(deadline or {})), self.PAIRS if pairs is None else pairs)

    def test_a_deadline_arm_that_did_everything_has_no_shortfall(self):
        self.assertEqual(self.short(fixed={"change_lag_mean_ms": 11.0}), [])

    def test_at_most_three_percent_of_the_pictures_may_still_come_under_thirty_ms(self):
        base = {"change_lag_mean_ms": 11.0}
        self.assertEqual(self.short(fixed=base, deadline={"under_30_share": 0.03}), [])
        found = self.short(fixed=base, deadline={"under_30_share": 0.0301})
        self.assertEqual(len(found), 1)
        self.assertIn("30 ms", found[0])
        self.assertEqual(len(self.short(fixed=base, deadline={"under_30_share": None})), 1)

    def test_the_deadline_lag_may_not_pass_four_ms_and_must_be_five_ms_under_the_fixed_one(self):
        self.assertEqual(self.short(fixed={"change_lag_mean_ms": 9.0}, deadline={"change_lag_mean_ms": 4.0}), [])
        above = self.short(fixed={"change_lag_mean_ms": 20.0}, deadline={"change_lag_mean_ms": 4.001})
        self.assertEqual(len(above), 1)
        self.assertIn("above", above[0])
        close = self.short(fixed={"change_lag_mean_ms": 8.999}, deadline={"change_lag_mean_ms": 4.0})
        self.assertEqual(len(close), 1)
        self.assertIn("lower", close[0])

    def test_no_traced_lag_on_either_side_is_a_shortfall_not_a_pass(self):
        for fixed, deadline in ((None, 2.5), (11.0, None), (None, None)):
            found = self.short(fixed={"change_lag_mean_ms": fixed}, deadline={"change_lag_mean_ms": deadline})
            self.assertEqual(len(found), 1, (fixed, deadline))
            self.assertIn("no change lag", found[0])

    def test_the_deadline_arm_polls_about_twice_per_frame_change(self):
        base = {"change_lag_mean_ms": 11.0}
        for polls in (1.9, 2.0, 2.15):
            self.assertEqual(self.short(fixed=base, deadline={"polls_per_frame_change": polls}), [], polls)
        for polls in (1.899, 2.151, None):
            found = self.short(fixed=base, deadline={"polls_per_frame_change": polls})
            self.assertEqual(len(found), 1, polls)
            self.assertIn("polls per frame change", found[0])

    def test_the_lag_has_to_be_lower_in_nine_of_ten_pairs(self):
        base = {"change_lag_mean_ms": 11.0}
        for count, needed in ((12, 11), (10, 9), (20, 18), (3, 3), (1, 1)):
            better = [(index, 11.0, 2.5) for index in range(needed)]
            worse = [(index, 2.5, 11.0) for index in range(needed, count)]
            self.assertEqual(self.short(fixed=base, pairs=better + worse), [], (count, needed))
            fewer = [(index, 11.0, 2.5) for index in range(needed - 1)] + [(index, 2.5, 11.0) for index in range(needed - 1, count)]
            found = self.short(fixed=base, pairs=fewer)
            self.assertEqual(len(found), 1, (count, needed))
            self.assertIn("counted pairs", found[0])

    def test_a_tie_in_a_pair_is_not_lower(self):
        pairs = [(index, 11.0, 2.5) for index in range(9)] + [(9, 5.0, 5.0)]
        self.assertEqual(len(self.short(fixed={"change_lag_mean_ms": 11.0}, pairs=pairs)), 0)
        pairs = [(index, 11.0, 2.5) for index in range(8)] + [(8, 5.0, 5.0), (9, 5.0, 5.0)]
        self.assertEqual(len(self.short(fixed={"change_lag_mean_ms": 11.0}, pairs=pairs)), 1)

    def test_no_pairs_is_a_shortfall(self):
        self.assertEqual(len(self.short(fixed={"change_lag_mean_ms": 11.0}, pairs=[])), 1)

    def test_every_shortfall_is_reported(self):
        found = self.short(fixed={"change_lag_mean_ms": 3.0}, deadline={"under_30_share": 0.2, "change_lag_mean_ms": 9.0, "polls_per_frame_change": 3.0},
                           pairs=[(0, 3.0, 9.0)])
        self.assertEqual(len(found), 5)


class VerdictTests(unittest.TestCase):
    def test_a_clean_win_is_adopted_and_the_words_never_claim_acceptance(self):
        said = ab.verdict(scenario(), 12)
        self.assertTrue(said.startswith("ADOPT:"), said)
        self.assertNotIn("accept", said.lower())
        self.assertIn("13.00%", said)  # the fixed arm's 52 of 400 short pictures
        self.assertIn("0.00%", said)

    def test_an_arm_that_is_not_what_it_claims_beats_every_other_outcome(self):
        rows = scenario(pairs=2)
        rows[1]["arm_findings"] = ["the host named the schedule ['fixed'] for a session of the deadline arm"]
        said = ab.verdict(rows, 2)
        self.assertTrue(said.startswith("INVALID ARMS:"), said)
        self.assertIn("named the schedule", said)

    def test_fewer_counted_pairs_than_the_rule_needs_is_inconclusive(self):
        rows = scenario()
        for found in rows:
            if found["arm"] == ab.DEADLINE and found["pair"] < 3:
                found["counted"] = False
        said = ab.verdict(rows, 12)
        self.assertTrue(said.startswith("INCONCLUSIVE:"), said)
        self.assertIn("9 counted pair(s) of 12", said)
        self.assertIn("needs 10", said)
        self.assertNotIn(" of ", ab.verdict(rows).split("the rule")[0])

    def test_ten_counted_pairs_are_enough(self):
        rows = scenario()
        for found in rows:
            if found["arm"] == ab.DEADLINE and found["pair"] < 2:
                found["counted"] = False
        self.assertTrue(ab.verdict(rows, 12).startswith("ADOPT:"))

    def test_the_fixed_arm_has_to_show_the_pattern_under_repair(self):
        said = ab.verdict(scenario(fixed={"under": 31}), 12)  # 7.75% of the pictures
        self.assertTrue(said.startswith("CONTROL NOT REPRODUCED:"), said)
        self.assertIn("0.0775", said)
        self.assertTrue(ab.verdict(scenario(fixed={"under": 32}), 12).startswith("ADOPT:"))  # exactly 8%
        self.assertTrue(ab.verdict(scenario(fixed={"under": 0}), 12).startswith("CONTROL NOT REPRODUCED:"))

    def test_a_fixed_arm_that_delivered_no_picture_is_a_control_not_reproduced_not_a_crash(self):
        rows = scenario()
        for found in rows:
            if found["arm"] == ab.FIXED:
                found["plays"] = {name: play(delivered=0, under=0, slow=0, very_slow=0) for name in ab.PLAYS}
        said = ab.verdict(rows, 12)
        self.assertTrue(said.startswith("CONTROL NOT REPRODUCED:"), said)
        self.assertIn("None of the fixed arm's pictures", said)

    def test_a_control_that_is_not_reproduced_is_judged_before_anything_else_in_the_deadline_arm(self):
        said = ab.verdict(scenario(fixed={"under": 0}, deadline={"slow": 100}), 12)
        self.assertTrue(said.startswith("CONTROL NOT REPRODUCED:"), said)

    def test_a_regression_is_judged_before_the_gain(self):
        said = ab.verdict(scenario(deadline={"slow": 20}, lags=[(11.0, 11.0)] * 12), 12)
        self.assertTrue(said.startswith("REGRESSION:"), said)
        self.assertIn("62.5", said)

    def test_a_lag_that_did_not_improve_is_no_gain(self):
        said = ab.verdict(scenario(lags=[(11.0, 11.0)] * 12), 12)
        self.assertTrue(said.startswith("NO GAIN:"), said)
        self.assertIn("lower", said)

    def test_the_deadline_arm_still_delivering_short_pictures_is_no_gain(self):
        said = ab.verdict(scenario(deadline={"under": 20}), 12)  # 5%
        self.assertTrue(said.startswith("NO GAIN:"), said)
        self.assertIn("30 ms", said)

    def test_a_deadline_arm_that_polls_more_than_twice_per_change_is_no_gain(self):
        self.assertTrue(ab.verdict(scenario(deadline={"polls": 2.3}), 12).startswith("NO GAIN:"))

    def test_one_pair_in_which_the_lag_did_not_drop_is_still_adopted_two_are_not(self):
        lags = [(11.0, 2.5)] * 12
        lags[4] = (2.4, 2.5)
        self.assertTrue(ab.verdict(scenario(lags=lags), 12).startswith("ADOPT:"))
        lags[5] = (2.4, 2.5)
        self.assertTrue(ab.verdict(scenario(lags=lags), 12).startswith("NO GAIN:"))

    def test_the_verdict_of_a_run_without_rows_is_inconclusive_not_a_crash(self):
        self.assertTrue(ab.verdict([], 12).startswith("INCONCLUSIVE:"))


def make_report(rows=None):
    """`build_report` over prepared rows (the analysis of each session is not what is tested here)."""
    rows = rows if rows is not None else scenario(pairs=12)
    records = [{"arm": found["arm"], "i": index} for index, found in enumerate(rows)]
    with mock.patch.object(ab, "session_row", side_effect=lambda rec, project: rows[rec["i"]]):
        return ab.build_report(records, PROJECT, manifest(12))


class ReportTests(unittest.TestCase):
    def report(self, rows=None):
        return make_report(rows)

    def test_the_report_has_both_arms_the_pairs_and_the_verdict(self):
        found = self.report()
        self.assertEqual(set(found["arms"]), {"fixed", "deadline"})
        self.assertEqual(found["arms"]["fixed"]["counted"], 12)
        self.assertEqual(len(found["counted_pairs"]), 12)
        self.assertEqual(found["counted_pairs"][0], {"pair": 0, "fixed_lag_ms": 11.0, "deadline_lag_ms": 2.5})
        self.assertTrue(found["verdict"].startswith("ADOPT:"))
        self.assertTrue(found["all_ran"])
        self.assertEqual(found["manifest"]["pairs"], 12)

    def test_the_verdict_of_a_report_says_how_many_pairs_the_manifest_asked_for(self):
        found = self.report(scenario(pairs=3))
        self.assertTrue(found["verdict"].startswith("INCONCLUSIVE:"), found["verdict"])
        self.assertIn("3 counted pair(s) of 12", found["verdict"])

    def test_the_project_is_described_without_its_sources(self):
        text = json.dumps(self.report())
        self.assertNotIn("not-published", text)
        self.assertNotIn("/private", text)
        self.assertEqual(self.report()["project"], {"length": 600, "fps": 24.0, "cuts": [186, 268, 410], "video_starts": []})

    def test_the_run_did_not_all_run_when_a_session_failed_or_an_arm_is_not_its_own(self):
        rows = scenario(pairs=12)
        rows[0]["failed"] = True
        self.assertFalse(self.report(rows)["all_ran"])
        rows = scenario(pairs=12)
        rows[1]["arm_findings"] = ["not the arm"]
        self.assertFalse(self.report(rows)["all_ran"])

    def test_a_session_that_is_not_counted_still_has_its_row(self):
        rows = scenario(pairs=12)
        rows[0]["counted"] = False
        found = self.report(rows)
        self.assertEqual(len(found["rows"]), 24)
        self.assertEqual(found["arms"]["fixed"]["sessions"], 12)
        self.assertEqual(found["arms"]["fixed"]["counted"], 11)


class MarkdownTests(unittest.TestCase):
    def text(self, rows=None):
        found = make_report(rows)
        return ab.markdown(found), found

    def test_the_text_shows_the_code_the_binary_the_arms_and_the_verdict(self):
        text, found = self.text()
        self.assertIn("a" * 12, text)
        self.assertIn("b" * 64, text)
        self.assertIn("d" * 64, text)
        self.assertIn("TESSEL_PLAYBACK_POLL", text)
        self.assertIn("| fixed |", text)
        self.assertIn("| deadline |", text)
        self.assertIn(f"Verdict (mechanical, pre-fixed rules): {found['verdict']}", text)
        self.assertNotIn("accepted", text.lower())

    def test_every_session_has_a_row_with_its_note(self):
        rows = scenario(pairs=2)
        rows[0]["counted"], rows[0]["invalid_reasons"] = False, ["the scene warm did not run"]
        rows[1]["counted"], rows[1]["failed"], rows[1]["error"] = False, True, "refused"
        rows[2]["plays"]["first"]["hung"] = True
        rows[3]["plays"].pop("repeat")
        text, _ = self.text(rows)
        self.assertIn("the scene warm did not run", text)
        self.assertIn("FAILED: refused", text)
        self.assertIn("HUNG", text)
        self.assertIn("missing", text)
        self.assertEqual(len([line for line in text.splitlines() if re.match(r"\| \d+ \| \d+ \| (fixed|deadline) \|", line)]), 4)

    def test_the_pairs_are_listed_with_both_lags(self):
        text, _ = self.text()
        self.assertEqual(len([line for line in text.splitlines() if re.match(r"\| \d+ \| 11\.0 \| 2\.5 \|", line)]), 12)

    def test_the_text_names_no_private_path(self):
        text, _ = self.text()
        self.assertNotIn("/private", text)
        self.assertNotIn("not-published", text)


class OutcomeTests(unittest.TestCase):
    def test_the_outcomes_the_docstring_names_are_the_ones_the_verdict_can_give(self):
        match = re.search(r"DECISION RULES.*?\n\n(?=Load is recorded)", ab.__doc__, re.S)
        self.assertTrue(match, "the docstring has no DECISION RULES block")
        text = match.group(0)
        for name in ("INVALID ARMS", "INCONCLUSIVE", "CONTROL NOT REPRODUCED", "REGRESSION", "NO GAIN", "ADOPT"):
            self.assertIn(name, text)
        said = {ab.verdict(scenario(), 12).split(":")[0], ab.verdict([], 12).split(":")[0], ab.verdict(scenario(fixed={"under": 0}), 12).split(":")[0],
                ab.verdict(scenario(deadline={"slow": 99}), 12).split(":")[0], ab.verdict(scenario(lags=[(11.0, 11.0)] * 12), 12).split(":")[0]}
        rows = scenario(pairs=2)
        rows[0]["arm_findings"] = ["x"]
        said.add(ab.verdict(rows, 2).split(":")[0])
        self.assertEqual(said, {"ADOPT", "INCONCLUSIVE", "CONTROL NOT REPRODUCED", "REGRESSION", "NO GAIN", "INVALID ARMS"})


class FinishTests(unittest.TestCase):
    def build(self, rows):
        records = [{"arm": found["arm"], "i": index} for index, found in enumerate(rows)]
        with mock.patch.object(ab, "session_row", side_effect=lambda rec, project: rows[rec["i"]]):
            return ab.EXPERIMENT.build(records, PROJECT, manifest(12))

    def test_the_raw_records_of_a_batch_are_named_by_their_place_and_arm(self):
        self.assertEqual(ab.EXPERIMENT.names(ab.arm_order(ab.ARMS, 2)), ["run-00-fixed.json", "run-01-deadline.json", "run-02-deadline.json", "run-03-fixed.json"])

    def test_a_batch_where_every_session_ran_gives_the_verdict_and_exits_zero(self):
        report = self.build(scenario(pairs=12))
        self.assertEqual(report["verdict"][:5], "ADOPT")
        self.assertIn("Verdict (mechanical, pre-fixed rules)", ab.EXPERIMENT.markdown(report))
        self.assertTrue(ab.EXPERIMENT.ok(report))

    def test_a_failed_session_exits_one_and_a_wrong_arm_too(self):
        rows = scenario(pairs=12)
        rows[0]["failed"] = True
        self.assertFalse(ab.EXPERIMENT.ok(self.build(rows)))
        rows = scenario(pairs=12)
        rows[1]["arm_findings"] = ["the host named another schedule"]
        self.assertFalse(ab.EXPERIMENT.ok(self.build(rows)))


class CommandTests(unittest.TestCase):
    class Locks:
        def __init__(self, name):
            self.name, self.closed = name, 0

        def close(self):
            self.closed += 1

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        (self.root / "runtime" / "dist").mkdir(parents=True)
        (self.root / "runtime" / "dist" / "index.html").write_text("")
        (self.root / "runtime" / "dist" / "runtime.js").write_text("runtime")
        self.binary = self.root / "tessel-editor"
        self.binary.write_text("binary")
        self.document = self.root / "p.tessel.json"
        self.document.write_text("{}")
        self.args = mock.Mock(binary=str(self.binary), project=str(self.document), out=str(self.root / "out"), pairs=2, stress_cpu=0, lock_wait=7.0, report_dir=None)
        for patch in (mock.patch.object(ab, "ROOT", self.root), mock.patch.object(ab, "warm_proof", return_value=object()),
                      mock.patch.object(shared_batch, "git_state", return_value={"sha": "c" * 40, "dirty_files": 1, "diff_sha256": None}),
                      mock.patch.object(ab, "rule_identity", return_value={"rule_sha256": "e" * 64, "rule_commit": "f" * 40})):
            patch.start()
            self.addCleanup(patch.stop)

    def run_sessions(self, blocked=False, sessions=None):
        """`cmd_run` with the lab's session and the analysis replaced: returns (exit code, sessions asked, locks, prerequisites asked, what was shown)."""
        calls, asked, locks = [], [], [self.Locks('/tmp/gpu.lock'), self.Locks('/tmp/review.lock')]
        run = shared_batch.Run(self.binary, self.document, PROJECT, locks, 1.5)

        def fake_prerequisites(args, needed=(), gate=None):
            asked.append({"needed": [Path(path).relative_to(self.root).as_posix() for path in needed], "gate": gate})
            return None if blocked else run

        def fake_session(W, binary, document, record, env, body, stress=0, collect=None):
            calls.append({"record": dict(record), "env": env, "stress": stress, "binary": binary})
            if sessions is not None and record["number"] >= sessions:
                raise RuntimeError("the editor never opened")
            return {**record, "events": [], "frames": []}

        def fake_row(rec, project):
            return row(rec["arm"], rec["order_index"] // 2, [play(lag=11.0 if rec["arm"] == ab.FIXED else 2.5)] * 2)

        shown = io.StringIO()
        with mock.patch.object(ab, "prerequisites", side_effect=fake_prerequisites), mock.patch.object(ab, "run_session", side_effect=fake_session), \
                mock.patch.object(ab, "session_row", side_effect=fake_row), contextlib.redirect_stdout(shown):
            try:
                code = ab.cmd_run(self.args)
            except RuntimeError:
                code = "raised"
        return code, calls, locks, asked, shown.getvalue()

    def test_a_blocked_run_exits_two_and_starts_nothing(self):
        code, calls, _, _, _ = self.run_sessions(blocked=True)
        self.assertEqual((code, calls), (2, []))

    def test_the_run_asks_for_the_runtime_page_and_for_the_rule_to_be_committed(self):
        _, _, _, asked, _ = self.run_sessions()
        self.assertEqual(asked[0]["needed"], ["runtime/dist/index.html", "runtime/dist/runtime.js"])
        self.assertIs(asked[0]["gate"], ab.rule_gate)

    def test_the_gate_is_the_reason_the_rule_is_not_fixed_or_nothing(self):
        with mock.patch.object(ab, "rule_state", return_value=(False, "the rule has changes that are not committed")):
            self.assertEqual(ab.rule_gate(), "the decision rule is not fixed before the run: the rule has changes that are not committed")
        with mock.patch.object(ab, "rule_state", return_value=(True, None)):
            self.assertIsNone(ab.rule_gate())

    def test_the_run_plays_two_sessions_per_pair_in_the_alternating_order_each_in_its_own_environment(self):
        code, calls, locks, _, shown = self.run_sessions()
        self.assertEqual([call["env"][ab.POLL_ENV] for call in calls], ["fixed", "deadline", "deadline", "fixed"])
        for call, arm in zip(calls, ["fixed", "deadline", "deadline", "fixed"]):
            self.assertEqual({key: value for key, value in call["env"].items() if key in ab.arm_env(arm)}, ab.arm_env(arm))
            self.assertEqual(call["env"]["TESSEL_VIEWPORT_ASSET_IO"], "sync")
        self.assertEqual([call["record"]["number"] for call in calls], [0, 1, 2, 3])
        self.assertEqual({call["record"]["mode"] for call in calls}, {"sync"})
        self.assertEqual({call["stress"] for call in calls}, {0})
        self.assertEqual([lock.closed for lock in locks], [1, 1])
        self.assertEqual(code, 0)
        self.assertIn("run 0 fixed", shown)
        self.assertIn("run 3 fixed", shown)

    def test_the_stress_option_reaches_every_session(self):
        self.args.stress_cpu = 3
        _, calls, _, _, _ = self.run_sessions()
        self.assertEqual({call["stress"] for call in calls}, {3})

    def test_the_manifest_carries_the_rule_the_control_environment_and_the_options(self):
        self.run_sessions()
        saved = json.loads((self.root / "out" / "manifest.json").read_text())
        self.assertEqual(saved["order"], ["fixed", "deadline", "deadline", "fixed"])
        self.assertEqual((saved["pairs"], saved["io"], saved["stress_cpu"]), (2, "sync", 0))
        self.assertEqual((saved["rule_sha256"], saved["rule_commit"]), ("e" * 64, "f" * 40))
        self.assertEqual(saved["env"], ab.CONTROL_ENV)
        self.assertEqual((saved["locks"], saved["lock_wait_s"]), (["/tmp/gpu.lock", "/tmp/review.lock"], 1.5))
        self.assertTrue((self.root / "out" / "analysis.md").exists())

    def test_the_call_to_the_harness_matches_its_real_signature(self):
        inspect.signature(ab.run_session).bind(object(), self.binary, self.document, {"number": 0}, ab.arm_env("fixed"), lambda *a: True, 0, collect=lambda *a: True)

    def test_the_command_line_defaults_are_the_ones_the_rule_is_written_for(self):
        seen = {}
        with mock.patch.object(ab, "cmd_run", side_effect=lambda args: seen.update(vars(args)) or 0), \
                mock.patch.object(sys, "argv", ["playback-poll-ab.py", "run", "--binary", "b", "--project", "p", "--out", "o"]):
            self.assertEqual(ab.main(), 0)
        self.assertEqual((seen["pairs"], seen["stress_cpu"], seen["lock_wait"], seen["report_dir"]), (12, 0, 600.0, None))
        with mock.patch.object(ab, "analyze", return_value=0) as analyzed, mock.patch.object(sys, "argv", ["playback-poll-ab.py", "analyze", "--out", "o", "--project", "p"]):
            self.assertEqual(ab.main(), 0)
        self.assertIs(analyzed.call_args.args[0], ab.EXPERIMENT)
        self.assertEqual(analyzed.call_args.args[1].out, "o")


if __name__ == "__main__":
    unittest.main()
