"""Pure-logic tests of the BL157.3 first-use A/B; no editor, GPU, runtime page or private media is started."""
import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("first_use_ab", Path(__file__).with_name("first-use-ab.py"))
ab = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ab)

PROJECT = {"length": 600, "fps": 24.0, "cuts": [186, 268, 410], "video_starts": [18, 196, 282], "sources": []}


def frame(id, shown, done, total=10.0):
    return {"id": id, "shown_frame": shown, "submitted_ms": done - total, "done_ms": done, "submit_to_done_ms": total, "transit_ms": 2.0,
            "runtime_total_ms": total - 2.0, "interactive": True, "prepare_ms": 1.0}


def event(kind, t, **fields):
    return {"kind": kind, "t_ms": t, "seq": int(t), "fields": fields}


def record(arm, gap_at_196, *, enabled=None, sent=3, spans=("uploaded",), supported=True, other_enabled=False, factor="preupload"):
    """A tiny window play: frames 190..200 every 42 ms, the picture at 196 late by `gap_at_196`."""
    frames, done = [], 1000.0
    for i, shown in enumerate(range(190, 201)):
        done += gap_at_196 if shown == 196 else 42.0
        frames.append(frame(i + 1, shown, done))
    events = [event("play_command", 900), event("play_clock_started", 950),
              event("frame_page", 1000, id=1, ipc_wait_ms=0.2, page={"queue_ms": 0.1, "held_ms": 1.0, "loop": {"max_ms": 1.0}, "ahead": {}})]
    events += [event("runtime_span", 1200 + n, k=ab.FACTORS[factor]["span"], outcome=outcome, ms=4.0) for n, outcome in enumerate(spans)]
    on = arm == "on" if enabled is None else enabled
    report = {"enabled": on, "supported": supported, "sent": sent}
    other = {"enabled": other_enabled}
    other_key = ab.FACTORS["warm_draw" if factor == "preupload" else "preupload"]["report"]
    return {"arm": arm, "io": "async", "window": [190, 200], "events": events, "frames": frames, "plays_ok": True, "events_lost": 0,
            "status": {"asset_io": {"mode": "async", "pool": {}}, "stats": {"warmed": 2}, ab.FACTORS[factor]["report"]: report, other_key: other},
            "external_cores": [1.0, 1.2], "memory_series": [[0, 100.0, 50.0, 150.0], [2, 120.0, 60.0, 180.0]], "host": {"loadavg_start": [1, 1, 1]},
            "loadavg": [[1.0, 1.0, 1.0]], "errors": []}


class ArmTests(unittest.TestCase):
    def test_every_arm_names_both_factors_so_a_stray_environment_cannot_turn_one_on(self):
        for factor, other in (("preupload", "TESSEL_VIEWPORT_WARM_DRAW"), ("warm_draw", "TESSEL_VIEWPORT_PREUPLOAD")):
            control, on = ab.arm_env(factor, "control"), ab.arm_env(factor, "on")
            self.assertEqual(control[ab.FACTORS[factor]["env"]], "0")
            self.assertEqual(on[ab.FACTORS[factor]["env"]], "1")
            self.assertEqual((control[other], on[other]), ("0", "0"), "the two factors are never both on in an arm")
            self.assertEqual(on["TESSEL_VIEWPORT_PROBE_GPU_FINISH"], "0")

    def test_the_order_alternates_which_arm_goes_first_by_pair(self):
        self.assertEqual(ab.arm_order(4), ["control", "on", "on", "control", "control", "on", "on", "control"])


class ValidityTests(unittest.TestCase):
    def reasons(self, **kw):
        return ab.factor_validity(record(**kw), "preupload")

    def test_a_working_on_arm_and_a_clean_control_have_no_reason(self):
        self.assertEqual(self.reasons(arm="on", gap_at_196=50), [])
        self.assertEqual(self.reasons(arm="control", gap_at_196=50, spans=(), sent=0), [])

    def test_the_host_reporting_another_arm_than_asked_is_invalid(self):
        self.assertTrue(any("enabled" in r for r in self.reasons(arm="on", gap_at_196=50, enabled=False)))
        self.assertTrue(any("enabled" in r for r in self.reasons(arm="control", gap_at_196=50, enabled=True, spans=())))

    def test_an_on_arm_that_manipulated_nothing_is_invalid_not_a_null_result(self):
        self.assertTrue(any("announce" in r for r in self.reasons(arm="on", gap_at_196=50, supported=False)))
        self.assertTrue(any("sent no pre-upload" in r for r in self.reasons(arm="on", gap_at_196=50, sent=0)))
        self.assertTrue(any("manipulation did not happen" in r for r in self.reasons(arm="on", gap_at_196=50, spans=("resident", "stale"))))

    def test_a_control_that_shows_the_factors_spans_or_a_second_factor_on_is_invalid(self):
        self.assertTrue(any("control arm shows" in r for r in self.reasons(arm="control", gap_at_196=50, spans=("uploaded",))))
        self.assertTrue(any("other factor" in r for r in self.reasons(arm="on", gap_at_196=50, other_enabled=True)))

    def test_the_warm_draw_factor_is_judged_on_its_own_report_and_span(self):
        good = record("on", 50, factor="warm_draw", spans=("drawn",))
        self.assertEqual(ab.factor_validity(good, "warm_draw"), [])
        none = record("on", 50, factor="warm_draw", spans=("gone",))
        self.assertTrue(any("manipulation did not happen" in r for r in ab.factor_validity(none, "warm_draw")))


def cut_record(gap_at, stop_gap=300.0):
    """Frames 180..199 every 42 ms, one late picture at `gap_at`, and the repeat of the stop frame 200 long after (the refine)."""
    frames, done = [], 1000.0
    for i, shown in enumerate(range(180, 200)):
        done += 130.0 if shown == gap_at else 42.0
        frames.append(frame(i + 1, shown, done))
    done += stop_gap
    frames.append(frame(99, 200, done))
    base = record("control", 50, spans=())
    return base | {"frames": frames}


class AnalysisTests(unittest.TestCase):
    def test_the_entry_gap_at_a_picture_clip_is_read_from_the_picture_that_first_shows_it(self):
        run = ab.analyze_run(record("control", 90), PROJECT, "preupload", (190, 200))
        play = run["plays"]["first"]
        entry = next(e for e in play["png_entries"] if e["entry"] == 196)
        self.assertEqual(entry["gap_ms"], 90.0)
        self.assertEqual(ab.target_gap(play, "preupload", 196), 90.0)
        self.assertEqual([e["entry"] for e in play["png_entries"]], [196], "only the entries inside the window are judged")

    def test_a_run_with_the_factor_working_is_valid_and_lists_its_spans(self):
        run = ab.analyze_run(record("on", 40), PROJECT, "preupload", (190, 200))
        self.assertTrue(run["valid"], run["invalid_reasons"])
        self.assertEqual(run["factor_spans"]["outcomes"], {"uploaded": 1})
        self.assertEqual(run["memory"]["rss_sum_mib"]["peak"], 180.0)

    def test_a_cut_entry_is_the_longest_gap_in_its_first_frames_not_the_first_picture(self):
        run = ab.analyze_run(cut_record(188), PROJECT, "preupload", (180, 200))
        entry = run["plays"]["first"]["cut_entries"][0]
        self.assertEqual((entry["entry"], entry["from_frame"], entry["shown_frame"], entry["gap_ms"]), (186, 187, 188, 130.0))

    def test_the_stop_frame_and_the_refine_after_it_are_not_part_of_a_window_play(self):
        run = ab.analyze_run(cut_record(188), PROJECT, "preupload", (180, 200))
        play = run["plays"]["first"]
        self.assertEqual(play["summary"]["interval_ms"]["max"], 130.0)
        self.assertEqual([g["gap_ms"] for g in play["slow_detail"]], [130.0])

    def test_a_play_that_shows_almost_none_of_the_project_is_a_hang_listed_not_dropped(self):
        run = ab.analyze_run(record("control", 50, spans=(), sent=0), PROJECT, "preupload", None)
        self.assertTrue(run["plays"]["first"]["hang"], "11 pictures of 600 frames")
        windowed = ab.analyze_run(record("control", 50, spans=(), sent=0), PROJECT, "preupload", (190, 200))
        self.assertFalse(windowed["plays"]["first"]["hang"], "a window play is judged by its own end, not by the whole project")

    def test_every_slow_gap_is_listed_with_where_it_ended(self):
        run = ab.analyze_run(record("control", 130), PROJECT, "preupload", (190, 200))
        slow = run["plays"]["first"]["slow_detail"]
        self.assertEqual([(g["to_frame"], g["gap_ms"], g["where"]) for g in slow], [(196, 130.0, "video:196")])


def pairs(controls, ons, excess=(0, 0)):
    runs = []
    for index, (control, on) in enumerate(zip(controls, ons)):
        for arm, gap in (("control", control), ("on", on)):
            run = ab.analyze_run(record(arm, gap, spans=("uploaded",) if arm == "on" else (), sent=3 if arm == "on" else 0), PROJECT, "preupload", (190, 200))
            run["pair"], run["order_index"] = index, index * 2 + (0 if arm == "control" else 1)
            run["plays"]["first"]["pre_cut_excess_ms"] = excess[0] if arm == "control" else excess[1]
            runs.append(run)
    return runs


class VerdictTests(unittest.TestCase):
    def verdict(self, controls, ons, **kw):
        rows = ab.pair_rows(pairs(controls, ons, **kw), "preupload", 196)
        return ab.verdict(rows, "preupload", 196)

    def test_a_gain_of_at_least_the_fixed_margin_in_three_of_four_is_a_local_benefit_and_never_acceptance(self):
        text = self.verdict([90, 95, 88, 92], [60, 70, 80, 75])
        self.assertIn("LOCAL BENEFIT", text)
        self.assertIn("not an acceptance", text)
        self.assertNotIn("ACCEPTED", text)

    def test_a_shorter_entry_that_added_time_before_the_cut_is_shifted_not_a_gain(self):
        self.assertIn("SHIFTED", self.verdict([90, 95, 88, 92], [60, 60, 60, 60], excess=(0, 50)))

    def test_a_gain_below_the_margin_or_in_too_few_pairs_is_no_benefit(self):
        self.assertIn("NO LOCAL BENEFIT", self.verdict([90, 95, 88, 92], [85, 90, 70, 91]))
        self.assertIn("NO LOCAL BENEFIT", self.verdict([90, 95, 88, 92], [60, 60, 88, 92]))

    def test_an_entry_still_over_the_diagnostic_reference_is_not_called_closed(self):
        self.assertIn("not closed", self.verdict([120, 120, 120, 120], [100, 100, 100, 100]))
        self.assertIn("under 62.5 ms in every first play", self.verdict([120, 120, 120, 120], [50, 50, 50, 50]))

    def test_fewer_than_four_pairs_is_inconclusive(self):
        rows = ab.pair_rows(pairs([90, 95], [60, 60]), "preupload", 196)
        self.assertIn("INCONCLUSIVE", ab.verdict(rows, "preupload", 196))

    def test_a_pair_is_comparable_only_when_both_are_valid_and_quiet(self):
        runs = pairs([90], [60])
        self.assertTrue(ab.pair_rows(runs, "preupload", 196)[0]["comparable"])
        runs[1]["quiet_class"] = "loaded"
        self.assertFalse(ab.pair_rows(runs, "preupload", 196)[0]["comparable"])


if __name__ == "__main__":
    unittest.main()
