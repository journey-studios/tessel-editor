"""Pure-logic tests of the BL157.6 playback cadence reader; no editor, GPU, runtime page or private media is started."""
import contextlib
import importlib.util
import io
import json
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("playback_cadence", Path(__file__).with_name("playback-cadence.py"))
pc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pc)

FRAME_MS = 1000.0 / 24.0
POLL_MS = 22.2


def tick(t, before, after, late=1.3):
    """A `tick` event as the host trace writes it (the field names are the trace's own)."""
    return {"seq": int(t), "t_ms": t, "kind": "tick",
            "fields": {"due_ms": t - late, "ran_ms": t, "late_ms": late, "body_ms": 0.1, "frame_before": before, "frame_after": after, "continues": True}}


def clock_started(t, fps=24.0):
    return {"seq": int(t), "t_ms": t, "kind": "play_clock_started", "fields": {"from": 0, "fps": fps}}


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
