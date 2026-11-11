"""Pure-logic tests of the BL157.6 playback cadence reader; no editor, GPU, runtime page or private media is started."""
import contextlib
import importlib.util
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ab  # noqa: E402
from ab import stats as shared  # noqa: E402

spec = importlib.util.spec_from_file_location("playback_cadence", Path(__file__).with_name("playback-cadence.py"))
pc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pc)

FRAME_MS = 1000.0 / 24.0
POLL_MS = 22.2


def tick(t, before, after, late=1.3):
    """A `tick` event as the host trace writes it (the field names are the trace's own)."""
    return {"seq": int(t), "t_ms": t, "kind": "tick",
            "fields": {"due_ms": t - late, "ran_ms": t, "late_ms": late, "body_ms": 0.1, "frame_before": before, "frame_after": after, "continues": True}}


def clock_started(t, fps=24.0, poll=None):
    fields = {"from": 0, "fps": fps}
    if poll is not None:
        fields["poll"] = poll
    return {"seq": int(t), "t_ms": t, "kind": "play_clock_started", "fields": fields}


def clocked(t, before, after, clock, next_ms=None, late=1.3):
    """A `tick` event of a host that traces the clock it read (fractional frames) and the wait it armed next (BL157.6's switch)."""
    event = tick(t, before, after, late)
    event["fields"]["clock"] = clock
    event["fields"]["next_ms"] = next_ms
    return event


def picture(done):
    return {"id": int(done), "done_ms": done, "interactive": True}


def polled_play(start, seconds=10.0, poll=POLL_MS, fps=24.0, offset=0.0):
    """The ticks of a play whose clock names frame floor((now - start) * fps) and is polled every `poll` ms (the first one `offset` ms after the start)."""
    events, frame, now = [clock_started(start, fps)], 0, start + offset
    while now - start < seconds * 1000.0:
        now += poll
        named = int((now - start) / (1000.0 / fps))
        events.append(tick(now, frame, named))
        frame = named
    return events


GUARD_MS = 1.0
LATE_MS = 1.46


def scheduled_play(start, schedule, seconds=20.0, fps=24.0):
    """A play whose perfect clock is read when the schedule says: `fixed` waits half a frame after each answer, `deadline` waits for the next
    frame boundary plus a guard (never more than half a frame). Every tick carries the clock it read, as the host traces it."""
    frame_ms = 1000.0 / fps
    cap, events, now, frame = frame_ms / 2, [clock_started(start, fps, schedule)], start, 0
    while now - start < seconds * 1000.0:
        wait = cap
        if schedule == "deadline" and events[-1]["kind"] == "tick":
            into = ((now - start) / frame_ms) % 1.0
            wait = min(cap, (1.0 - into) * frame_ms + GUARD_MS)
        now += wait + LATE_MS
        clock = (now - start) / frame_ms
        named = int(clock)
        events.append(clocked(now, frame, named, clock, next_ms=wait))
        frame = named
    return events


def record(arm, events, frames=(), mode="async"):
    return {"arm": arm, "mode": mode, "events": events, "frames": list(frames)}


class WindowTests(unittest.TestCase):
    def test_each_play_runs_from_its_clock_start_to_the_next(self):
        events = [clock_started(100.0), tick(120.0, 0, 0), clock_started(5000.0), tick(5020.0, 0, 0)]
        self.assertEqual(pc.windows(events), [(100.0, 5000.0), (5000.0, float("inf"))])

    def test_a_trace_without_a_play_has_no_window(self):
        self.assertEqual(pc.windows([tick(10.0, 0, 0)]), [])

    def test_only_ticks_inside_the_play_that_saw_a_frame_are_read(self):
        events = [tick(50.0, 0, 0), clock_started(100.0), tick(120.0, 0, 0), tick(140.0, None, None), tick(160.0, 0, 1), tick(6000.0, 1, 1)]
        found = pc.ticks_of(events, (100.0, 5000.0))
        self.assertEqual([t["ran_ms"] for t in found], [120.0, 160.0])

    def test_a_tick_that_names_no_frame_is_not_a_poll_of_the_clock(self):
        events = [clock_started(0.0), tick(10.0, 3, None), tick(20.0, None, 3)]
        self.assertEqual(pc.ticks_of(events, (0.0, 100.0)), [])

    def test_the_frame_rates_a_trace_reports(self):
        self.assertEqual(pc.frames_per_second([clock_started(0.0, 24.0), clock_started(9.0, 24.0)]), {24.0})
        self.assertEqual(pc.frames_per_second([clock_started(0.0, 24.0), clock_started(9.0, 30.0)]), {24.0, 30.0})
        self.assertEqual(pc.frames_per_second([tick(1.0, 0, 0)]), set())


class PollsPerChangeTests(unittest.TestCase):
    def fields(self, pairs):
        return [t["fields"] for t in [tick(10.0 * (i + 1), a, b) for i, (a, b) in enumerate(pairs)]]

    def test_the_polls_between_two_changes_are_counted(self):
        counts = pc.polls_per_change(self.fields([(0, 0), (0, 1), (1, 1), (1, 2), (2, 2), (2, 3), (3, 4)]))
        self.assertEqual(dict(counts), {2: 2, 1: 1})

    def test_the_first_change_has_no_earlier_one_to_count_from(self):
        self.assertEqual(dict(pc.polls_per_change(self.fields([(0, 0), (0, 0), (0, 0), (0, 1)]))), {})

    def test_polls_after_the_last_change_are_not_a_step(self):
        self.assertEqual(dict(pc.polls_per_change(self.fields([(0, 1), (1, 1), (1, 1)]))), {})

    def test_a_jump_of_two_frames_is_one_change_and_one_skipped_frame(self):
        ticks = self.fields([(0, 1), (1, 1), (1, 3), (3, 3), (3, 4)])
        self.assertEqual(dict(pc.polls_per_change(ticks)), {2: 2})
        self.assertEqual(pc.skipped_frames(ticks), 1)

    def test_no_jump_skips_nothing(self):
        self.assertEqual(pc.skipped_frames(self.fields([(0, 1), (1, 2), (2, 2)])), 0)

    def test_the_gap_between_polls_is_on_the_host_clock(self):
        ticks = [t["fields"] for t in [tick(100.0, 0, 0), tick(122.5, 0, 1), tick(144.0, 1, 1)]]
        self.assertEqual(pc.poll_gaps(ticks), [22.5, 21.5])

    def test_a_poll_without_a_time_is_skipped_not_guessed(self):
        ticks = [{"ran_ms": 10.0}, {"ran_ms": None}, {"ran_ms": 40.0}]
        self.assertEqual(pc.poll_gaps(ticks), [30.0])


class DeliveredTests(unittest.TestCase):
    def test_only_pictures_inside_the_play_count_and_in_time_order(self):
        frames = [picture(5.0), picture(160.0), picture(100.0), picture(140.0), picture(9000.0)]
        self.assertEqual(pc.delivered_gaps(frames, (100.0, 5000.0)), [40.0, 20.0])

    def test_one_picture_has_no_gap(self):
        self.assertEqual(pc.delivered_gaps([picture(100.0)], (0.0, 500.0)), [])

    def test_a_picture_without_a_time_is_skipped(self):
        self.assertEqual(pc.delivered_gaps([{"id": 1}, picture(10.0), picture(30.0)], (0.0, 500.0)), [20.0])


class ModelTests(unittest.TestCase):
    def test_a_frame_of_one_and_seven_eighths_polls_takes_one_or_two(self):
        self.assertEqual(pc.predicted_polls(FRAME_MS, 22.2222), {1: 0.125, 2: 0.875})

    def test_a_poll_that_divides_the_frame_always_takes_the_same_number(self):
        self.assertEqual(pc.predicted_polls(FRAME_MS, FRAME_MS / 2), {2: 1.0})
        self.assertEqual(pc.predicted_polls(FRAME_MS, FRAME_MS), {1: 1.0})

    def test_a_half_poll_is_an_even_split(self):
        self.assertEqual(pc.predicted_polls(50.0, 20.0), {2: 0.5, 3: 0.5})

    def test_the_nominal_poll_alone_is_not_a_pattern_but_its_real_length_is(self):
        self.assertEqual(pc.predicted_polls(FRAME_MS, 1000.0 / 48.0), {2: 1.0})
        self.assertEqual(sorted(pc.predicted_polls(FRAME_MS, 1000.0 / 48.0 + 1.3)), [1, 2])


class StatsTests(unittest.TestCase):
    def test_nearest_rank_without_interpolation(self):
        found = pc.stats([10.0, 20.0, 30.0, 40.0])
        self.assertEqual((found["n"], found["mean"], found["p50"], found["p95"], found["max"]), (4, 25.0, 20.0, 40.0, 40.0))

    # Fixed samples and what this reader's own `stats` returned for them BEFORE it used the harness's (`ab.stats`): the output must not move by a digit or a key.
    SMALL = [3.2, None, 10.0, 7.5, 7.5, 1.0, 22.25, 44.4444, 0.0005, 16.7]
    LARGE = [round(((i * 37) % 101) * 0.731 + (i % 7) * 0.0123, 4) for i in range(1, 201)]

    def test_the_summary_of_a_fixed_sample_is_the_one_this_reader_gave_before_it_shared_the_statistics(self):
        small = {"n": 9, "mean": 12.511, "p50": 7.5, "p95": 44.444, "p99": 44.444, "max": 44.444}
        large = {"n": 200, "mean": 36.718, "p50": 36.562, "p95": 69.507, "p99": 72.418, "max": 73.162}
        nothing = {"n": 0, "mean": None, "p50": None, "p95": None, "p99": None, "max": None}
        one = {"n": 1, "mean": 5.0, "p50": 5.0, "p95": 5.0, "p99": 5.0, "max": 5.0}
        for values, expected in ((self.SMALL, small), (self.LARGE, large), ([], nothing), ([None], nothing), ([5.0], one)):
            with self.subTest(n=len(values)):
                self.assertEqual(pc.stats(values), expected)
                self.assertEqual(json.dumps(pc.stats(values)), json.dumps(expected), "the published JSON keeps its key order too")
                self.assertEqual(list(pc.stats(values)), ["n", "mean", "p50", "p95", "p99", "max"])

    def test_the_statistics_are_the_harnesss_own_not_a_copy_and_load_the_same_way_from_ab(self):
        self.assertIs(pc.stats.func, shared.stats)
        self.assertIs(ab.cadence().stats.func, shared.stats, "`ab.cadence()` (the one `load_module`) gets the same function")

    def test_nothing_is_nothing(self):
        self.assertEqual(pc.stats([None, None])["n"], 0)
        self.assertIsNone(pc.stats([])["p50"])

    def test_the_share_of_a_list(self):
        self.assertEqual(pc.share_of([10.0, 20.0, 40.0, 50.0], lambda gap: gap < 30.0), 0.5)
        self.assertIsNone(pc.share_of([], lambda gap: True))


class SummaryTests(unittest.TestCase):
    def test_the_polled_clock_changes_frame_at_the_predicted_shares(self):
        events = polled_play(1000.0) + polled_play(20000.0)
        found = pc.summarize([("a", record("x", events))])
        self.assertEqual((found["sessions"], found["plays"], found["fps"]), (1, 2, 24.0))
        share = found["polls_per_change_share"]
        self.assertAlmostEqual(share["1"], 0.125, delta=0.01)
        self.assertAlmostEqual(share["2"], 0.875, delta=0.01)
        self.assertEqual(sorted(share), ["1", "2"])
        self.assertAlmostEqual(found["poll_ms"]["mean"], POLL_MS, places=3)
        self.assertEqual(found["skipped_frames"], 0)

    def test_the_model_is_built_from_the_real_poll_not_the_nominal_one(self):
        found = pc.summarize([("a", record("x", polled_play(1000.0)))])
        self.assertAlmostEqual(found["model"]["polls_per_frame"], FRAME_MS / POLL_MS, places=2)
        self.assertAlmostEqual(found["model"]["predicted_share"]["1"], 1 - (FRAME_MS / POLL_MS - 1), delta=0.01)

    def test_a_clock_polled_every_half_frame_never_catches_up(self):
        found = pc.summarize([("a", record("x", polled_play(0.0, poll=FRAME_MS / 2, offset=FRAME_MS / 4)))])
        self.assertEqual(found["polls_per_change_share"], {"2": 1.0})
        self.assertEqual(found["model"]["predicted_share"], {"2": 1.0})

    def test_two_frame_rates_make_no_prediction(self):
        events = polled_play(1000.0, fps=24.0) + polled_play(20000.0, fps=30.0)
        found = pc.summarize([("a", record("x", events))])
        self.assertIsNone(found["fps"])
        self.assertIsNone(found["model"])

    def test_the_monitors_pictures_are_summarised_inside_the_plays(self):
        events = [clock_started(1000.0), tick(1022.0, 0, 0)]
        frames = [picture(1100.0), picture(1122.0), picture(1166.0), picture(1210.0), picture(1300.0), picture(900.0)]
        found = pc.summarize([("a", record("x", events, frames))])
        self.assertEqual(found["delivered_ms"]["n"], 4)
        self.assertEqual(found["delivered_share_under_30_ms"], 0.25)
        self.assertEqual(found["delivered_share_over_62_5_ms"], 0.25)

    def test_a_session_without_ticks_adds_no_play(self):
        found = pc.summarize([("a", record("x", [clock_started(0.0)]))])
        self.assertEqual((found["plays"], found["frame_changes"], found["polls_per_change_share"]), (0, 0, {}))

    def test_jumped_frames_are_added_up(self):
        events = [clock_started(0.0), tick(22.0, 0, 1), tick(44.0, 1, 1), tick(66.0, 1, 4), tick(88.0, 4, 5)]
        self.assertEqual(pc.summarize([("a", record("x", events))])["skipped_frames"], 2)


class ChangeLagTests(unittest.TestCase):
    """How long after its boundary a frame change was seen: the fraction of a frame the clock had already run when the poll read it."""

    def fields(self, *events):
        return [e["fields"] for e in events]

    def test_a_change_is_seen_after_the_part_of_the_frame_the_clock_had_run(self):
        ticks = self.fields(clocked(100.0, 9, 10, 10.25))
        self.assertAlmostEqual(pc.change_lags(ticks, 24.0)[0], 0.25 * FRAME_MS, places=6)

    def test_a_poll_that_did_not_change_the_frame_is_not_a_lag(self):
        self.assertEqual(pc.change_lags(self.fields(clocked(100.0, 10, 10, 10.9)), 24.0), [])

    def test_a_trace_that_does_not_carry_the_clock_is_not_guessed(self):
        self.assertEqual(pc.change_lags(self.fields(tick(100.0, 9, 10)), 24.0), [])

    def test_a_jump_counts_the_age_of_the_frame_that_was_reached(self):
        ticks = self.fields(clocked(100.0, 5, 8, 8.5))
        self.assertAlmostEqual(pc.change_lags(ticks, 24.0)[0], 0.5 * FRAME_MS, places=6)

    def test_a_clock_that_is_not_a_number_is_skipped(self):
        ticks = self.fields(clocked(1.0, 0, 1, None), clocked(2.0, 1, 2, float("nan")), clocked(3.0, 2, 3, "x"), clocked(4.0, 3, 4, float("inf")))
        self.assertEqual(pc.change_lags(ticks, 24.0), [])

    def test_without_a_frame_rate_there_is_no_lag(self):
        ticks = self.fields(clocked(100.0, 9, 10, 10.25))
        self.assertEqual(pc.change_lags(ticks, None), [])
        self.assertEqual(pc.change_lags(ticks, 0), [])

    def test_the_rate_scales_the_lag(self):
        ticks = self.fields(clocked(100.0, 9, 10, 10.5))
        self.assertAlmostEqual(pc.change_lags(ticks, 60.0)[0], 0.5 * 1000.0 / 60.0, places=6)


class PlayWindowTests(unittest.TestCase):
    def test_a_window_carries_the_rate_and_the_poll_mode_of_its_clock_start(self):
        events = [clock_started(100.0, 24.0, "deadline"), clocked(120.0, 0, 0, 0.5), clock_started(5000.0, 30.0, "fixed")]
        self.assertEqual(pc.play_windows(events), [
            {"start": 100.0, "end": 5000.0, "fps": 24.0, "poll": "deadline"},
            {"start": 5000.0, "end": float("inf"), "fps": 30.0, "poll": "fixed"}])

    def test_a_trace_from_before_the_switch_has_no_mode(self):
        self.assertEqual(pc.play_windows([clock_started(1.0)])[0]["poll"], None)

    def test_windows_are_the_same_spans_as_before(self):
        events = [clock_started(100.0), clock_started(5000.0)]
        self.assertEqual(pc.windows(events), [(100.0, 5000.0), (5000.0, float("inf"))])


class ScheduleSummaryTests(unittest.TestCase):
    """The two schedules, on traces made by a model of each, read through the same reader: the pattern of the fixed one, the landing of the
    deadline one, and the lag the reader measures from the clock the host traced."""

    def summary(self, schedule):
        events = scheduled_play(1000.0, schedule) + scheduled_play(60000.0, schedule)
        return pc.summarize([("a", record(schedule, events))])

    def test_the_fixed_schedule_sees_a_change_about_eleven_milliseconds_late(self):
        found = self.summary("fixed")
        self.assertAlmostEqual(found["change_lag_ms"]["mean"], 11.2, delta=1.0)
        self.assertGreater(found["change_lag_ms"]["max"], 20.0)
        self.assertAlmostEqual(found["polls_per_frame_change"], 1.87, delta=0.03)

    def test_the_deadline_schedule_sees_it_just_after_the_boundary(self):
        found = self.summary("deadline")
        landing = GUARD_MS + LATE_MS
        self.assertAlmostEqual(found["change_lag_ms"]["mean"], landing, delta=0.3)
        self.assertLess(found["change_lag_ms"]["max"], landing + 0.5)
        self.assertAlmostEqual(found["polls_per_frame_change"], 2.0, delta=0.05)

    def test_the_deadline_schedule_has_no_short_delivery_step(self):
        found = self.summary("deadline")
        self.assertEqual(found["polls_per_change_share"], {"2": 1.0})

    def test_the_mode_of_the_plays_is_reported(self):
        self.assertEqual(self.summary("deadline")["poll_modes"], ["deadline"])
        self.assertEqual(self.summary("fixed")["poll_modes"], ["fixed"])

    def test_a_trace_without_the_mode_says_so(self):
        found = pc.summarize([("a", record("x", polled_play(1000.0)))])
        self.assertEqual(found["poll_modes"], ["unknown"])
        self.assertEqual(found["change_lag_ms"]["n"], 0)

    def test_a_group_with_two_modes_names_both(self):
        events = scheduled_play(1000.0, "fixed") + scheduled_play(60000.0, "deadline")
        self.assertEqual(pc.summarize([("a", record("x", events))])["poll_modes"], ["deadline", "fixed"])


class EachPlayTests(unittest.TestCase):
    def frames(self, start, gaps):
        done, found = start, []
        for gap in gaps:
            done += gap
            found.append(picture(done))
        return found

    def test_every_play_has_its_own_numbers(self):
        events = scheduled_play(1000.0, "deadline", 5.0) + scheduled_play(60000.0, "fixed", 5.0)
        frames = self.frames(1500.0, [41.7] * 20 + [80.0, 120.0]) + self.frames(60500.0, [22.0, 44.0, 22.0, 44.0, 66.0])
        plays = pc.each_play([("a", record("x", events, frames))])
        self.assertEqual([p["poll"] for p in plays], ["deadline", "fixed"])
        first, second = plays
        # 22 pictures are 21 gaps (the first has nothing before it); 5 pictures are 4 gaps: 44, 22, 44 and 66 ms.
        self.assertEqual((first["delivered"], first["under_30_ms"], first["over_62_5_ms"], first["over_100_ms"]), (21, 0, 2, 1))
        self.assertEqual((second["delivered"], second["under_30_ms"], second["over_62_5_ms"], second["over_100_ms"]), (4, 1, 1, 0))
        self.assertAlmostEqual(first["change_lag_ms"]["mean"], GUARD_MS + LATE_MS, delta=0.3)
        self.assertGreater(second["change_lag_ms"]["mean"], 8.0)
        self.assertEqual(first["session"], "a")

    def test_a_play_without_ticks_is_not_a_play(self):
        self.assertEqual(pc.each_play([("a", record("x", [clock_started(0.0)]))]), [])

    def test_the_plays_add_up_to_the_group(self):
        events = scheduled_play(1000.0, "fixed", 6.0) + scheduled_play(60000.0, "fixed", 6.0)
        frames = self.frames(1200.0, [22.0, 44.0] * 30) + self.frames(60200.0, [22.0, 44.0, 66.0] * 20)
        sessions = [("a", record("x", events, frames))]
        plays, group = pc.each_play(sessions), pc.summarize(sessions)
        self.assertEqual(sum(p["delivered"] for p in plays), group["delivered_ms"]["n"])
        self.assertEqual(sum(p["under_30_ms"] for p in plays), round(group["delivered_share_under_30_ms"] * group["delivered_ms"]["n"]))
        self.assertEqual(sum(p["skipped_frames"] for p in plays), group["skipped_frames"])


class MarksAndEdgesTests(unittest.TestCase):
    """The marks (30, 62.5 and 100 ms) are the rule's own and the spans are half open: a gap exactly on a mark is not past it, a tick exactly at the start of a play
    is in it and one exactly at the start of the next is not."""

    def played(self, gaps):
        """One play with a poll that saw a frame and pictures `gaps` apart (the first picture has nothing before it)."""
        done, frames = 1100.0, [picture(1100.0)]
        for gap in gaps:
            done += gap
            frames.append(picture(done))
        return [("a", record("x", [clock_started(1000.0), tick(1022.0, 0, 0)], frames))]

    def test_a_picture_is_short_under_thirty_ms_and_not_at_thirty(self):
        sessions = self.played([27.0, 30.0, 33.0])
        self.assertEqual(pc.each_play(sessions)[0]["under_30_ms"], 1)
        self.assertEqual(pc.summarize(sessions)["delivered_share_under_30_ms"], round(1 / 3, 4))

    def test_a_picture_is_slow_over_sixty_two_and_a_half_ms_and_not_at_it(self):
        sessions = self.played([61.0, 62.5, 64.0])
        self.assertEqual(pc.each_play(sessions)[0]["over_62_5_ms"], 1)
        self.assertEqual(pc.summarize(sessions)["delivered_share_over_62_5_ms"], round(1 / 3, 4))

    def test_a_picture_is_very_slow_over_a_hundred_ms_and_not_at_it(self):
        sessions = self.played([95.0, 100.0, 105.0])
        self.assertEqual(pc.each_play(sessions)[0]["over_100_ms"], 1)

    def test_the_longest_wait_of_a_play_is_its_largest_gap_and_a_play_without_gaps_has_none(self):
        self.assertEqual(pc.each_play(self.played([30.0, 80.0, 50.0]))[0]["max_delivered_ms"], 80.0)
        self.assertIsNone(pc.each_play(self.played([]))[0]["max_delivered_ms"])

    def test_a_tick_at_the_start_of_a_play_is_in_it_and_one_at_the_start_of_the_next_is_not(self):
        events = [clock_started(100.0), tick(100.0, 0, 1), tick(5000.0, 1, 2), clock_started(5000.0)]
        self.assertEqual([t["ran_ms"] for t in pc.ticks_of(events, (100.0, 5000.0))], [100.0])

    def test_a_picture_at_the_start_of_the_next_play_is_not_in_this_one(self):
        frames = [picture(100.0), picture(200.0), picture(500.0)]
        self.assertEqual(pc.delivered_gaps(frames, (100.0, 500.0)), [100.0])

    def test_a_play_is_placed_by_the_host_clock_not_by_the_sequence_number(self):
        first = {"seq": 7, "t_ms": 1234.5, "kind": "play_clock_started", "fields": {"from": 0, "fps": 24.0}}
        second = {"seq": 99, "t_ms": 9000.5, "kind": "play_clock_started", "fields": {"from": 0, "fps": 24.0}}
        self.assertEqual([(w["start"], w["end"]) for w in pc.play_windows([first, second])], [(1234.5, 9000.5), (9000.5, float("inf"))])


class ReadingEdgesTests(unittest.TestCase):
    def test_a_play_that_names_no_rate_does_not_hide_the_rate_of_the_others(self):
        nameless = {"seq": 1, "t_ms": 0.0, "kind": "play_clock_started", "fields": {"from": 0}}
        self.assertEqual(pc.frames_per_second([nameless, clock_started(9.0, 24.0)]), {24.0})
        self.assertEqual(pc.frames_per_second([nameless]), set())

    def test_two_sessions_at_two_frame_rates_make_no_prediction(self):
        sessions = [("a", record("x", polled_play(1000.0, fps=24.0))), ("b", record("x", polled_play(1000.0, fps=30.0)))]
        found = pc.summarize(sessions)
        self.assertIsNone(found["fps"])
        self.assertIsNone(found["model"])

    def test_a_rate_that_is_not_a_plain_positive_number_gives_no_lag(self):
        ticks = [clocked(100.0, 9, 10, 10.25)["fields"]]
        for rate in (True, float("inf"), float("nan"), -24.0, "24"):
            self.assertEqual(pc.change_lags(ticks, rate), [], rate)

    def test_a_clock_that_is_a_boolean_gives_no_lag(self):
        self.assertEqual(pc.change_lags([clocked(1.0, 0, 1, True)["fields"]], 24.0), [])

    def test_how_late_each_poll_ran_is_summarised(self):
        found = pc.summarize([("a", record("x", polled_play(1000.0, seconds=4.0)))])
        self.assertEqual(found["late_ms"]["n"], found["poll_ms"]["n"] + 1)
        self.assertEqual(found["late_ms"]["mean"], 1.3)

    def test_a_group_without_a_frame_change_says_so(self):
        self.assertEqual(pc.shares({}), "no frame change")
        self.assertEqual(pc.shares({"1": 0.125, "2": 0.875}), "1 poll: 12.5%, 2 polls: 87.5%")


class EachPlayFieldTests(unittest.TestCase):
    def test_a_play_is_summarised_field_by_field(self):
        events = [clock_started(1000.0, 24.0, "fixed"), clocked(1020.0, 0, 0, 0.9), clocked(1040.0, 0, 1, 1.25), clocked(1060.0, 1, 1, 1.7),
                  clocked(1080.0, 1, 3, 3.5), clocked(1100.0, 3, 4, 4.0)]
        frames = [picture(1100.0), picture(1130.0), picture(1200.0), picture(1310.0)]
        found = pc.each_play([("run-00.json", record("x", events, frames))])
        self.assertEqual(len(found), 1)
        play = found[0]
        lag = play.pop("change_lag_ms")
        # Five polls; the changes at the second, fourth and fifth (the first has nothing before it, so two steps: 2 polls and 1 poll); one frame jumped over.
        self.assertEqual(play, {"session": "run-00.json", "poll": "fixed", "fps": 24.0, "ticks": 5, "frame_changes": 2, "polls_per_frame_change": 1.5,
                                "delivered": 3, "under_30_ms": 0, "over_62_5_ms": 2, "over_100_ms": 1, "max_delivered_ms": 110.0,
                                "poll_ms": pc.stats([20.0, 20.0, 20.0, 20.0]), "skipped_frames": 1})
        self.assertEqual(lag["n"], 3)
        self.assertAlmostEqual(lag["mean"], (0.25 + 0.5 + 0.0) / 3 * FRAME_MS, places=3)
        self.assertAlmostEqual(lag["max"], 0.5 * FRAME_MS, places=3)

    def test_the_mean_polls_per_frame_change_weighs_each_step_by_its_count(self):
        self.assertEqual(pc.mean_polls({2: 3, 1: 1}), 1.75)
        self.assertIsNone(pc.mean_polls({}))


class GroupTests(unittest.TestCase):
    def test_a_session_belongs_to_its_arm_then_its_mode(self):
        self.assertEqual(pc.group_of({"arm": "patched", "mode": "async"}, "auto"), "patched")
        self.assertEqual(pc.group_of({"mode": "sync"}, "auto"), "sync")
        self.assertEqual(pc.group_of({}, "auto"), "all")

    def test_all_pools_everything(self):
        self.assertEqual(pc.group_of({"arm": "patched"}, "all"), "all")

    def test_groups_are_reported_in_name_order_with_their_own_numbers(self):
        events = polled_play(1000.0)
        report = pc.build_report([("1", record("unpatched", events)), ("2", record("patched", events)), ("3", record("patched", events))])
        self.assertEqual(list(report["groups"]), ["patched", "unpatched"])
        self.assertEqual([report["groups"][g]["sessions"] for g in report["groups"]], [2, 1])

    def test_all_makes_one_group(self):
        events = polled_play(1000.0)
        report = pc.build_report([("1", record("patched", events)), ("2", record("unpatched", events))], "all")
        self.assertEqual(list(report["groups"]), ["all"])
        self.assertEqual(report["groups"]["all"]["sessions"], 2)


class ReadAndOutputTests(unittest.TestCase):
    def folder(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        root = Path(folder.name)
        for name, arm in (("run-00-patched.json", "patched"), ("run-01-unpatched.json", "unpatched")):
            (root / name).write_text(json.dumps(record(arm, polled_play(1000.0, seconds=4.0))))
        (root / "manifest.json").write_text(json.dumps({"not": "a session"}))
        return root

    def test_a_folder_gives_its_run_files_and_nothing_else(self):
        names = [name for name, _ in pc.read_sessions([str(self.folder())])]
        self.assertEqual(names, ["run-00-patched.json", "run-01-unpatched.json"])

    def test_a_file_is_read_as_it_is(self):
        root = self.folder()
        names = [name for name, _ in pc.read_sessions([str(root / "manifest.json")])]
        self.assertEqual(names, ["manifest.json"])

    def test_the_markdown_shows_the_lag_and_the_mode_when_the_trace_has_them(self):
        events = scheduled_play(1000.0, "deadline", 5.0)
        text = pc.markdown(pc.build_report([("1", record("deadline", events))]))
        self.assertIn("change lag ms mean / p95 / max", text)
        self.assertIn("| deadline | 1 | 1 |", text)
        self.assertIn("poll deadline", text)

    def test_the_markdown_of_an_old_trace_says_the_lag_is_unknown(self):
        text = pc.markdown(pc.build_report([("1", record("x", polled_play(1000.0, seconds=4.0)))]))
        self.assertIn("n/a", text)

    def test_the_markdown_has_each_group_and_its_prediction(self):
        root = self.folder()
        text = pc.markdown(pc.build_report(pc.read_sessions([str(root)])))
        self.assertIn("| patched | 1 | 1 |", text)
        self.assertIn("| unpatched | 1 | 1 |", text)
        self.assertIn("1 poll: ", text)
        self.assertIn("2 polls: ", text)
        self.assertIn("so the model predicts 1 poll: ", text)

    def test_main_prints_and_writes_the_report(self):
        root = self.folder()
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = pc.main([str(root), "--json", str(root / "cadence.json"), "--markdown", str(root / "cadence.md")])
        self.assertEqual(code, 0)
        self.assertIn("Playback clock cadence", out.getvalue())
        self.assertEqual(sorted(json.loads((root / "cadence.json").read_text())["groups"]), ["patched", "unpatched"])
        self.assertEqual((root / "cadence.md").read_text(), out.getvalue())

    def test_main_refuses_a_folder_without_a_play(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        (Path(folder.name) / "run-00-x.json").write_text(json.dumps(record("x", [tick(1.0, 0, 0)])))
        errors = io.StringIO()
        with contextlib.redirect_stderr(errors), contextlib.redirect_stdout(io.StringIO()):
            code = pc.main([folder.name])
        self.assertEqual(code, 2)
        self.assertIn("no session with a play", errors.getvalue())


if __name__ == "__main__":
    unittest.main()
