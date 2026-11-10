"""Pure-logic tests of the BL157.1 real-project A/B harness; no editor, GPU, runtime page or private media is started."""
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
import xml.etree.ElementTree as ET

spec = importlib.util.spec_from_file_location("real_latency_ab", Path(__file__).with_name("real-latency-ab.py"))
ab = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ab)

PROJECT = {"length": 600, "fps": 24.0, "cuts": [186, 268, 410]}


def shot(id, frame, done, latency=10.0, interactive=True, submitted=None):
    return {"id": id, "shown_frame": frame, "done_ms": done, "submitted_ms": done - latency if submitted is None else submitted,
            "submit_to_done_ms": latency, "transit_ms": 1.0, "runtime_total_ms": 5.0, "prepare_ms": 0.2, "interactive": interactive}


def event(kind, t, **fields):
    return {"kind": kind, "t_ms": t, "seq": int(t), "fields": fields}


class SchedulingTests(unittest.TestCase):
    def test_the_arms_alternate_and_stay_balanced(self):
        self.assertEqual(ab.pair_order(4), ["sync", "async", "async", "sync", "sync", "async", "async", "sync"])
        self.assertEqual(ab.pair_order(4).count("sync"), 4)

    def test_nearest_rank_percentiles_never_invent_a_value(self):
        values = [10.0, 20.0, 30.0, 400.0]
        self.assertEqual(ab.percentile(values, 0.5), 20.0)
        self.assertEqual(ab.percentile(values, 0.95), 400.0)
        self.assertEqual(ab.stats([])["max"], None)
        self.assertEqual(ab.stats([None, 5.0])["n"], 1)


class ProjectTests(unittest.TestCase):
    def document(self):
        return {"active_sequence": "main", "sequences": [{"id": "main", "length": 600, "rate": {"num": 24, "den": 1}, "clips": [
            {"kind": "scene", "start": 0}, {"kind": "scene", "start": 186}, {"kind": "scene", "start": 268}, {"kind": "video", "start": 18},
            {"kind": "scene", "start": 410}]}],
            "library": {"assets": [{"kind": "splat", "asset": {"source": {"type": "file", "path": "/nonexistent/private-name.ply"}}}]}}

    def test_cuts_are_the_scene_clip_starts_after_zero_read_from_the_timeline(self):
        project = ab.parse_project(self.document())
        self.assertEqual(project["cuts"], [186, 268, 410])
        self.assertEqual(project["fps"], 24.0)
        self.assertEqual(project["clip_kinds"], {"scene": 4, "video": 1})

    def test_the_source_report_carries_a_label_and_no_name(self):
        checked = ab.check_sources(ab.parse_project(self.document())["sources"])
        text = json.dumps(checked)
        self.assertNotIn("private-name", text)
        self.assertNotIn("/nonexistent", text)
        self.assertEqual(checked[0]["present"], False)

    def test_a_present_source_has_a_size_and_a_head_tail_hash(self):
        with tempfile.TemporaryDirectory() as folder:
            path = os.path.join(folder, "a.ply")
            Path(path).write_bytes(b"x" * 10)
            checked = ab.check_sources([{"kind": "splat", "path": path}])
            self.assertEqual(checked[0]["size"], 10)
            self.assertEqual(len(checked[0]["head_tail_sha256"]), 64)


class GapTests(unittest.TestCase):
    def test_where_a_gap_is_is_decided_by_the_cuts_the_timeline_gave(self):
        self.assertEqual(ab.locate(140, PROJECT["cuts"], 48), "prefetch:186")
        self.assertEqual(ab.locate(186, PROJECT["cuts"], 48), "cut:186")
        self.assertEqual(ab.locate(192, PROJECT["cuts"], 48), "cut:186")
        self.assertEqual(ab.locate(200, PROJECT["cuts"], 48), "other")
        self.assertEqual(ab.locate(137, PROJECT["cuts"], 48), "other")

    def test_a_play_is_its_interactive_pictures_between_its_clock_and_the_next_play(self):
        events = [event("play_command", 100), event("play_clock_started", 150), event("play_command", 5000), event("play_clock_started", 5050)]
        segments = ab.play_segments(events)
        self.assertEqual([s["clock_ms"] for s in segments], [150, 5050])
        frames = [shot(1, 0, 90, interactive=False), shot(2, 1, 200), shot(3, 2, 240), shot(4, 600, 4800, interactive=False),
                  shot(5, 1, 5100), shot(6, 2, 5140)]
        first = [f["id"] for f in ab.play_pictures(frames, segments[0])]
        second = [f["id"] for f in ab.play_pictures(frames, segments[1])]
        self.assertEqual(first, [2, 3], "the picture before the clock and the refine after the end are not part of the play")
        self.assertEqual(second, [5, 6])

    def test_the_gap_context_counts_only_what_overlaps_the_gap(self):
        events = [
            event("tick", 1010, late_ms=3.0, body_ms=1.0),
            event("tick", 1090, late_ms=40.0, body_ms=2.0),
            event("tick", 9000, late_ms=99.0, body_ms=2.0),
            event("asset_cb", 1040, recv_ms=1030.0, callback_ms=25.0, mode="sync", outcome="answered"),
            event("asset_cb", 5000, recv_ms=4990.0, callback_ms=500.0, mode="sync", outcome="answered"),
            event("asset_io", 1050, read_ms=20.0, respond_ms=2.0, bytes=1000, queue_wait_ms=0.0, mode="sync"),
            event("prefetch", 1080, total_ms=7.0, plans_ahead_ms=6.0),
        ]
        context = ab.gap_context(events, 1000, 1100)
        self.assertEqual(context["ticks"], 2)
        self.assertEqual(context["max_tick_late_ms"], 40.0)
        self.assertEqual(context["ui_callback_ms"], 25.0)
        self.assertEqual(context["reads"], 1)
        self.assertEqual(context["read_bytes"], 1000)
        self.assertEqual(context["plans_ahead_ms_max"], 6.0)
        self.assertEqual(context["max_tick_late_ms"], 40.0, "the tick at 9000 ms is outside the gap")

    def test_a_play_summary_counts_slow_gaps_by_location_without_calling_them_causes(self):
        events = [event("play_command", 0), event("play_clock_started", 10)]
        frames, done = [], 10
        for index, frame in enumerate(range(130, 150)):
            done += 120.0 if frame == 140 else 41.7
            frames.append(shot(index, frame, done))
        segment = ab.play_segments(events)[0]
        summary = ab.summarize_play(frames, events, segment, {**PROJECT, "fps": 24.0}, 41.667)
        self.assertEqual(summary["slow_gt_62_5"], 1)
        self.assertEqual(summary["slow_gt_100"], 1)
        self.assertEqual(summary["slow_detail"][0]["where"], "prefetch:186")
        self.assertEqual(summary["by_location"]["prefetch:186"]["slow"], 1)
        self.assertEqual(summary["interval_ms"]["max"], 120.0)


def session(mode, **overrides):
    base = {"number": 0, "mode": mode, "order_index": 0, "events": [], "events_lost": 0, "plays_ok": True,
            "status": {"asset_io": {"mode": mode, "pool": {"saturated": 0, "panicked": 0, "stale": 0} if mode == "async" else None},
                       "stats": {"warmed": 12}}}
    base.update(overrides)
    return base


class ValidityTests(unittest.TestCase):
    def test_a_clean_session_is_valid(self):
        for mode in ab.MODES:
            self.assertEqual(ab.session_validity(session(mode)), (True, []))

    def test_saturation_is_an_invalid_run_not_a_win(self):
        bad = session("async")
        bad["status"]["asset_io"]["pool"]["saturated"] = 1
        valid, reasons = ab.session_validity(bad)
        self.assertFalse(valid)
        self.assertIn("saturated", reasons[0])

    def test_the_arm_must_be_the_mode_in_effect(self):
        wrong = session("async")
        wrong["status"]["asset_io"]["mode"] = "sync"
        self.assertFalse(ab.session_validity(wrong)[0])

    def test_lost_events_a_failed_play_an_error_status_and_a_missing_warm_each_invalidate(self):
        self.assertFalse(ab.session_validity(session("sync", events_lost=3))[0])
        self.assertFalse(ab.session_validity(session("sync", plays_ok=False))[0])
        self.assertFalse(ab.session_validity(session("sync", events=[event("asset_io", 1, status=503)]))[0])
        self.assertTrue(ab.session_validity(session("sync", events=[event("asset_io", 1, status=206)]))[0])
        cold = session("sync")
        cold["status"]["stats"]["warmed"] = 0
        self.assertFalse(ab.session_validity(cold)[0])


class LoadClassTests(unittest.TestCase):
    def test_quiet_is_decided_by_thresholds_fixed_before_the_measurement(self):
        self.assertEqual(ab.quiet_class([0.5, 1.0, 2.0, 1.5]), "quiet")
        self.assertEqual(ab.quiet_class([5.0, 6.0, 7.0]), "loaded")
        self.assertEqual(ab.quiet_class([0.5] * 18 + [20.0, 20.0]), "loaded", "a heavy tail is not a quiet machine")
        self.assertEqual(ab.quiet_class([]), "unknown")


def summary(mode, **kw):
    play = {"slow_gt_62_5": kw.get("slow", 2), "slow_gt_100": 0, "interval_ms": {"p50": 41.7, "p95": 50.0, "max": kw.get("max", 90.0)},
            "request_to_accept_ms": {"p95": 20.0}}
    return {"mode": mode, "order_index": kw.get("order", 0), "valid": kw.get("valid", True), "quiet_class": kw.get("cls", "quiet"),
            "external_median_cores": kw.get("ext", 1.0), "plays": {"first": play, "repeat": play}}


class PairTests(unittest.TestCase):
    def test_a_pair_is_comparable_only_when_both_are_valid_quiet_and_alike(self):
        rows = ab.pair_rows({0: {"sync": summary("sync"), "async": summary("async", slow=1, max=70.0)}})
        self.assertTrue(all(r["comparable"] for r in rows))
        self.assertEqual(rows[0]["delta_async_minus_sync"]["slow_gt_62_5"], -1)
        self.assertEqual(rows[0]["delta_async_minus_sync"]["max_gap_ms"], -20.0)
        loaded = ab.pair_rows({0: {"sync": summary("sync"), "async": summary("async", cls="loaded", ext=9.0)}})
        self.assertFalse(any(r["comparable"] for r in loaded))
        drift = ab.pair_rows({0: {"sync": summary("sync", ext=0.5), "async": summary("async", ext=3.0)}})
        self.assertFalse(any(r["comparable"] for r in drift), "both quiet, but the machine changed between them")
        invalid = ab.pair_rows({0: {"sync": summary("sync", valid=False), "async": summary("async")}})
        self.assertFalse(any(r["comparable"] for r in invalid))


class ChartTests(unittest.TestCase):
    def test_the_charts_are_well_formed_svg(self):
        gaps = [(i, i + 1, i * 40.0, (i + 1) * 40.0, 40.0 + (80 if i == 140 else 0)) for i in range(130, 160)]
        ET.fromstring(ab.svg_gaps("t", PROJECT, {"sync": gaps, "async": gaps}))
        ET.fromstring(ab.svg_gaps("t", PROJECT, {}))
        events = [event("asset_cb", 100, recv_ms=90.0, callback_ms=8.0, mode="sync", outcome="answered"),
                  event("asset_io", 110, read_ms=6.0, respond_ms=1.0, bytes=5, mode="async"),
                  event("tick", 120, late_ms=30.0, due_ms=90.0, ran_ms=120.0),
                  event("prefetch", 130, total_ms=5.0, plans_ahead_ms=4.0),
                  event("sync", 140, total_ms=3.0)]
        ET.fromstring(ab.svg_spans("s", events, [shot(1, 1, 80), shot(2, 2, 180)], 0, 200))


if __name__ == "__main__":
    unittest.main()
