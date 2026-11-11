"""Pure-logic tests of the BL157.3 first-use A/B; no editor, GPU, runtime page or private media is started."""
import contextlib
import importlib.util
import io
import json
import sys
import unittest
from pathlib import Path
from unittest import mock

spec = importlib.util.spec_from_file_location("first_use_ab", Path(__file__).with_name("first-use-ab.py"))
ab = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ab)

PROJECT = {"length": 600, "fps": 24.0, "cuts": [186, 268, 410], "video_starts": [18, 196, 282], "sources": []}
# What the page can say about the two experiments: the one list the runtime suite, the host suite and this one all read.
CONTRACT = json.loads((Path(__file__).resolve().parent.parent / "runtime/test/fixtures/activation-spans.json").read_text())["spans"]


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
    events = [event("play_command", 900), event("play_clock_started", 950)]
    # the host asks the page for its account of every frame (telemetry.timing), so every picture carries one
    events += [event("frame_page", f["done_ms"], id=f["id"], ipc_wait_ms=0.2, page={"queue_ms": 0.1, "held_ms": 1.0, "loop": {"max_ms": 1.0}, "ahead": {}})
               for f in frames]
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
        self.assertEqual(ab.arm_order(ab.ARMS, 4), ["control", "on", "on", "control", "control", "on", "on", "control"])


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
    def test_the_errors_of_a_session_are_published_as_codes_and_kinds_never_as_their_text(self):
        failing = record("on", 40)
        failing["errors"] = ["play(first): RuntimeError('could not open /Users/someone/private/clip.ply')",
                             "playback.start(first): {'ok': False, 'error': {'code': 'media_preparing', 'message': 'wait /Users/someone/private/clip.ply'}}"]
        run = ab.analyze_run(failing, PROJECT, "preupload", (190, 200))
        self.assertEqual(run["errors"], [{"code": "RuntimeError", "kind": "exception"}, {"code": "media_preparing", "kind": "host_refused"}])
        for private in ("/Users", "someone", "private", "clip.ply"):
            self.assertNotIn(private, json.dumps(run))

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


class PageWorkTests(unittest.TestCase):
    def test_a_slow_gap_over_an_unsplit_load_lists_it_as_page_work(self):
        busy = record("control", 130)
        busy["events"].append(event("asset_state", 1370, state="ready", timings={"load_ms": 100}))  # a frame sequence load: 100 ms, no fetch/decode split
        slow = ab.analyze_run(busy, PROJECT, "preupload", (190, 200))["plays"]["first"]["slow_detail"]
        quiet = ab.analyze_run(record("control", 130), PROJECT, "preupload", (190, 200))["plays"]["first"]["slow_detail"]
        self.assertEqual((slow[0]["page_work_ms"], quiet[0]["page_work_ms"]), (100.0, 0.0))


class MissingStatisticsTests(unittest.TestCase):
    """A valid play with fewer than two accepted pictures has no gap percentiles (None): the aggregation must still publish its report."""

    def lone_picture_run(self):
        lone = record("on", 50)
        lone["frames"] = lone["frames"][:1]
        return ab.analyze_run(lone, PROJECT, "preupload", (190, 200))

    def test_a_play_with_one_picture_has_no_gap_statistics_and_still_counts_as_a_run(self):
        run = self.lone_picture_run()
        self.assertTrue(run["valid"], run["invalid_reasons"])
        self.assertIsNone(run["plays"]["first"]["summary"]["interval_ms"]["p50"])

    def test_the_arm_summary_skips_what_was_not_measured_instead_of_failing(self):
        runs = [self.lone_picture_run(), ab.analyze_run(record("on", 50), PROJECT, "preupload", (190, 200))]
        found = ab.arm_summary(runs)["on/first"]
        self.assertEqual(found["runs"], 2)
        self.assertEqual(found["p50_gap_median_ms"], 42.0, "the median of the run that has one")
        self.assertIsNotNone(found["p95_gap_median_ms"])
        alone = ab.arm_summary([self.lone_picture_run()])["on/first"]
        self.assertEqual((alone["runs"], alone["p50_gap_median_ms"], alone["p95_gap_median_ms"]), (1, None, None))

    def test_the_report_of_a_batch_with_such_a_play_is_published(self):
        manifest = {"factor": "preupload", "mode": "narrow", "entry": 196, "window": [190, 200], "git": {"sha": "0" * 40, "dirty_files": 0}, "order": []}
        report = ab.build_report([record("control", 50, spans=()) | {"frames": record("on", 50)["frames"][:1]}], PROJECT, manifest)
        self.assertEqual(report["arms"]["control/first"]["p50_gap_median_ms"], None)

    def test_the_median_skips_only_what_was_not_measured(self):
        self.assertEqual(ab.median_of([1.0, None, 3.0]), 2.0)
        self.assertEqual(ab.median_of([None, None]), None)
        self.assertEqual(ab.median_of([]), None)
        self.assertEqual(ab.median_of([40.0, 44.4]), 42.2)


class ActivationContractTests(unittest.TestCase):
    """The harness judges an `on` arm by what the page says it did. The runtime suite makes the real page say every outcome of
    `runtime/test/fixtures/activation-spans.json` and the host suite sends each to the trace; these check the last link."""

    def test_the_harness_names_the_same_spans_and_the_same_worked_outcome_as_the_contract(self):
        self.assertEqual({f["span"]: f["worked"] for f in ab.FACTORS.values()}, {k: v["worked"] for k, v in CONTRACT.items()})

    def test_only_the_worked_outcome_is_evidence_that_the_manipulation_happened_whatever_else_the_page_reports(self):
        for factor, factor_spec in ab.FACTORS.items():
            contract = CONTRACT[factor_spec["span"]]
            self.assertEqual(ab.factor_validity(record("on", 50, factor=factor, spans=(contract["worked"],)), factor), [], factor)
            for outcome in contract["not_worked"]:
                reasons = ab.factor_validity(record("on", 50, factor=factor, spans=(outcome,)), factor)
                self.assertTrue(any("manipulation did not happen" in r for r in reasons), f"{factor}/{outcome} must not count as activation")
            mixed = tuple(contract["not_worked"]) + (contract["worked"],)
            self.assertEqual(ab.factor_validity(record("on", 50, factor=factor, spans=mixed), factor), [], "one worked span among the others is enough")

    def test_any_outcome_of_the_contract_in_a_control_arm_is_a_contaminated_control(self):
        for factor, factor_spec in ab.FACTORS.items():
            contract = CONTRACT[factor_spec["span"]]
            for outcome in [contract["worked"], *contract["not_worked"]]:
                reasons = ab.factor_validity(record("control", 50, factor=factor, spans=(outcome,), sent=0), factor)
                self.assertTrue(any("control arm shows" in r for r in reasons), f"{factor}/{outcome}")

    def test_the_summary_adds_up_only_the_time_of_the_worked_outcome(self):
        for factor, factor_spec in ab.FACTORS.items():
            contract = CONTRACT[factor_spec["span"]]
            outcomes = [contract["worked"], *contract["not_worked"]]
            events = [event("runtime_span", 1000 + n, k=factor_spec["span"], outcome=outcome, ms=4.0) for n, outcome in enumerate(outcomes)]
            summary = ab.span_summary(events, factor_spec["span"])
            self.assertEqual(summary["spans"], len(outcomes))
            self.assertEqual(summary["total_ms"], 4.0, f"{factor}: only the span that worked adds its time")
            self.assertEqual(summary["outcomes"], {outcome: 1 for outcome in outcomes})


def pixel_session(arm, order, *, spans=None, enabled=None, factor="warm_draw"):
    """One session of the pixel proof: `spans` as `scene-warm-proof.span_counts` gives them, and the host's report of the factor's switch."""
    report = ab.FACTORS[factor]["report"]
    on = arm == "on"
    return {"label": arm, "order": order, "spans": spans if spans is not None else {}, "factors": {report: {"enabled": on if enabled is None else enabled}}}


class PixelActivationTests(unittest.TestCase):
    """The pixel proof says the manipulation changes no pixel; it means nothing unless the manipulation HAPPENED in the arm that is on."""

    GOOD = [pixel_session("control", "hot"), pixel_session("control", "cold"),
            pixel_session("on", "hot", spans={"warm_draw": {"drawn": 4, "taken": 1}}), pixel_session("on", "cold")]

    def verdicts(self, sessions, factor="warm_draw"):
        return {name.split(": ", 1)[1]: ok for name, ok, _ in ab.activation_verdicts(sessions, factor)}

    def test_a_proof_whose_on_arm_drew_and_whose_control_did_not_is_active(self):
        self.assertTrue(all(self.verdicts(self.GOOD).values()), self.verdicts(self.GOOD))

    def test_an_inert_on_arm_fails_the_proof_however_identical_the_pixels_are(self):
        for spans in ({}, {"warm_draw": {"taken": 3, "gone": 1}}, {"warm": {"-": 2}}):
            sessions = [s if s["label"] != "on" or s["order"] != "hot" else pixel_session("on", "hot", spans=spans) for s in self.GOOD]
            verdicts = self.verdicts(sessions)
            self.assertFalse(verdicts["the `on` session that played into the cut reports `warm_draw`/`drawn` from the page"], spans)

    def test_a_control_arm_that_shows_the_factor_is_contaminated_not_a_proof(self):
        sessions = self.GOOD[:1] + [pixel_session("control", "cold", spans={"warm_draw": {"drawn": 1}})] + self.GOOD[2:]
        self.assertFalse(self.verdicts(sessions)["no control session reports a `warm_draw` span"])

    def test_the_host_must_report_each_arms_switch_as_asked_and_a_missing_report_never_passes(self):
        wrong = [pixel_session("on", "hot", spans={"warm_draw": {"drawn": 1}}, enabled=False)] + self.GOOD[:2]
        self.assertFalse(self.verdicts(wrong)["the host reports `warm_draw.enabled` as each arm was asked"])
        silent = [dict(s, factors={}) for s in self.GOOD]
        self.assertFalse(self.verdicts(silent)["the host reports `warm_draw.enabled` as each arm was asked"])

    def test_no_sessions_is_not_agreement(self):
        self.assertFalse(any(self.verdicts([]).values()))

    def test_the_verdict_reads_the_span_and_the_worked_outcome_of_the_factor_it_is_asked_about(self):
        sessions = [pixel_session("on", "hot", spans={"preupload": {"uploaded": 3}}, factor="preupload"), pixel_session("control", "hot", factor="preupload")]
        self.assertTrue(all(self.verdicts(sessions, "preupload").values()))
        self.assertFalse(self.verdicts(sessions, "warm_draw")["the `on` session that played into the cut reports `warm_draw`/`drawn` from the page"])


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

    def test_a_pair_with_an_invalid_session_is_not_read_even_when_its_numbers_are_a_gain(self):
        runs = pairs([90, 95, 88, 92], [60, 70, 80, 75])
        self.assertIn("LOCAL BENEFIT", ab.verdict(ab.pair_rows(runs, "preupload", 196), "preupload", 196))
        for index in (1, 3):  # the `on` session of the first two pairs did not run to its end: its numbers mean nothing
            runs[index]["valid"] = False
            runs[index]["invalid_reasons"] = ["the window did not play to its end"]
        rows = ab.pair_rows(runs, "preupload", 196)
        self.assertEqual([r["valid"] for r in rows if r["play"] == "first"], [False, False, True, True])
        text = ab.verdict(rows, "preupload", 196)
        self.assertTrue(text.startswith("INCONCLUSIVE"), text)
        self.assertIn("valid sessions in only 2 of 4", text)
        self.assertNotIn("LOCAL BENEFIT", text)

    def test_fewer_than_four_pairs_is_inconclusive(self):
        rows = ab.pair_rows(pairs([90, 95], [60, 60]), "preupload", 196)
        self.assertIn("INCONCLUSIVE", ab.verdict(rows, "preupload", 196))

    def test_a_pair_is_comparable_only_when_both_are_valid_and_quiet(self):
        runs = pairs([90], [60])
        self.assertTrue(ab.pair_rows(runs, "preupload", 196)[0]["comparable"])
        runs[1]["quiet_class"] = "loaded"
        self.assertFalse(ab.pair_rows(runs, "preupload", 196)[0]["comparable"])


class PixelsCommandTests(unittest.TestCase):
    def parse(self, *argv):
        seen = {}
        with mock.patch.object(ab, "cmd_pixels", side_effect=lambda args: seen.update(vars(args)) or 0), \
                mock.patch.object(sys, "argv", ["first-use-ab.py", "pixels", "--factor", "warm_draw", *argv]), contextlib.redirect_stderr(io.StringIO()):
            return ab.main(), seen

    def test_the_pixel_proof_takes_neither_a_lock_wait_nor_a_lower_disk_floor(self):
        code, seen = self.parse("--out", "o")
        self.assertEqual(code, 0)
        self.assertEqual(sorted(seen), ["binary", "command", "factor", "out"])
        for flag in (("--min-free-gib", "0"), ("--lock-wait", "5")):
            with self.assertRaises(SystemExit) as raised:
                self.parse("--out", "o", *flag)
            self.assertEqual(raised.exception.code, 2, flag)


if __name__ == "__main__":
    unittest.main()
