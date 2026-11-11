"""Tests of the shared core of the A/B runners (`scripts/ab/`): project reading, stats, plays and gaps, validity, charts and the batch loop.
Pure logic; no editor, GPU, runtime page or private media is started."""
import itertools
import json
import os
import pathlib
import sys
import tempfile
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ab import batch as B, charts as C, play as P, project as PR, session as S, stats as ST, transit as TR, validity as V  # noqa: E402

MODES = ("sync", "async")



PROJECT = {"length": 600, "fps": 24.0, "cuts": [186, 268, 410]}


def shot(id, frame, done, latency=10.0, interactive=True, submitted=None):
    return {"id": id, "shown_frame": frame, "done_ms": done, "submitted_ms": done - latency if submitted is None else submitted,
            "submit_to_done_ms": latency, "transit_ms": 1.0, "runtime_total_ms": 5.0, "prepare_ms": 0.2, "interactive": interactive}


def event(name, t, **fields):
    return {"kind": name, "t_ms": t, "seq": int(t), "fields": fields}


def session(mode, **overrides):
    base = {"number": 0, "mode": mode, "order_index": 0, "events": [], "events_lost": 0, "plays_ok": True,
            "status": {"asset_io": {"mode": mode, "pool": {"saturated": 0, "panicked": 0, "stale": 0} if mode == "async" else None},
                       "stats": {"warmed": 12}}}
    base.update(overrides)
    return base


class SchedulingTests(unittest.TestCase):
    def test_the_arms_alternate_and_stay_balanced(self):
        self.assertEqual(B.arm_order(("sync", "async"), 4), ["sync", "async", "async", "sync", "sync", "async", "async", "sync"])
        self.assertEqual(B.arm_order(("sync", "async"), 4).count("sync"), 4)

    def test_nearest_rank_percentiles_never_invent_a_value(self):
        values = [10.0, 20.0, 30.0, 400.0]
        self.assertEqual(ST.percentile(values, 0.5), 20.0)
        self.assertEqual(ST.percentile(values, 0.95), 400.0)
        self.assertEqual(ST.stats([])["max"], None)
        self.assertEqual(ST.stats([None, 5.0])["n"], 1)


class ProjectTests(unittest.TestCase):
    def document(self):
        return {"active_sequence": "main", "sequences": [{"id": "main", "length": 600, "rate": {"num": 24, "den": 1}, "clips": [
            {"kind": "scene", "start": 0}, {"kind": "scene", "start": 186}, {"kind": "scene", "start": 268}, {"kind": "video", "start": 18},
            {"kind": "scene", "start": 410}]}],
            "library": {"assets": [{"kind": "splat", "asset": {"source": {"type": "file", "path": "/nonexistent/private-name.ply"}}}]}}

    def test_cuts_are_the_scene_clip_starts_after_zero_read_from_the_timeline(self):
        project = PR.parse_project(self.document())
        self.assertEqual(project["cuts"], [186, 268, 410])
        self.assertEqual(project["video_starts"], [18])
        self.assertEqual(project["fps"], 24.0)
        self.assertEqual(project["clip_kinds"], {"scene": 4, "video": 1})

    def test_the_source_report_carries_a_label_and_no_name(self):
        checked = PR.check_sources(PR.parse_project(self.document())["sources"])
        text = json.dumps(checked)
        self.assertNotIn("private-name", text)
        self.assertNotIn("/nonexistent", text)
        self.assertEqual(checked[0]["present"], False)

    def test_a_present_source_has_a_size_and_a_head_tail_hash(self):
        with tempfile.TemporaryDirectory() as folder:
            path = os.path.join(folder, "a.ply")
            Path(path).write_bytes(b"x" * 10)
            checked = PR.check_sources([{"kind": "splat", "path": path}])
            self.assertEqual(checked[0]["size"], 10)
            self.assertEqual(len(checked[0]["head_tail_sha256"]), 64)


class GapTests(unittest.TestCase):
    def test_where_a_gap_is_is_decided_by_the_cuts_the_timeline_gave(self):
        self.assertEqual(P.locate(140, PROJECT["cuts"], 48), "prefetch:186")
        self.assertEqual(P.locate(186, PROJECT["cuts"], 48), "cut:186")
        self.assertEqual(P.locate(192, PROJECT["cuts"], 48), "cut:186")
        self.assertEqual(P.locate(200, PROJECT["cuts"], 48), "other")
        self.assertEqual(P.locate(137, PROJECT["cuts"], 48), "other")
        self.assertEqual(P.locate(196, PROJECT["cuts"], 48, video_starts=[196]), "video:196")
        self.assertEqual(P.locate(370, PROJECT["cuts"], 48, video_starts=[370]), "video:370", "a clip entry inside a prefetch window is the clip entry")
        self.assertEqual(P.locate(199, PROJECT["cuts"], 48, video_starts=[196]), "other")

    def test_a_play_is_its_interactive_pictures_between_its_clock_and_the_next_play(self):
        events = [event("play_command", 100), event("play_clock_started", 150), event("play_command", 5000), event("play_clock_started", 5050)]
        segments = P.play_segments(events)
        self.assertEqual([s["clock_ms"] for s in segments], [150, 5050])
        frames = [shot(1, 0, 90, interactive=False), shot(2, 1, 200), shot(3, 2, 240), shot(4, 600, 4800, interactive=False),
                  shot(5, 1, 5100), shot(6, 2, 5140)]
        first = [f["id"] for f in P.play_pictures(frames, segments[0])]
        second = [f["id"] for f in P.play_pictures(frames, segments[1])]
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
        context = P.gap_context(events, 1000, 1100)
        self.assertEqual(context["ticks"], 2)
        self.assertEqual(context["max_tick_late_ms"], 40.0)
        self.assertEqual(context["ui_callback_ms"], 25.0)
        self.assertEqual(context["reads"], 1)
        self.assertEqual(context["read_bytes"], 1000)
        self.assertEqual(context["plans_ahead_ms_max"], 6.0)
        self.assertEqual(context["max_tick_late_ms"], 40.0, "the tick at 9000 ms is outside the gap")

    def test_page_work_counts_only_the_part_that_overlaps_the_gap_and_never_mixes_clocks(self):
        events = [
            # the page reported at host 1500 ms that a load took 400 ms (fetch 100, decode 300): decode ran 1200..1500
            event("asset_state", 1500, state="ready", timings={"load_ms": 400, "fetch_ms": 100, "decode_ms": 300, "bytes": 5}),
            event("runtime_span", 1480, k="warm", queued_ms=0, ms=50, started=1),
            event("runtime_span", 5000, k="warm", queued_ms=0, ms=50, started=1),
        ]
        context = P.gap_context(events, 1400, 1450)
        self.assertEqual(context["page_decode_ms"], 50.0)
        self.assertEqual(context["page_fetch_ms"], 0.0)
        self.assertEqual(context["page_warm_ms"], 20.0)
        self.assertEqual(P.gap_context(events, 1100, 1190)["page_fetch_ms"], 90.0)

    def test_a_play_summary_counts_slow_gaps_by_location_without_calling_them_causes(self):
        events = [event("play_command", 0), event("play_clock_started", 10)]
        frames, done = [], 10
        for index, frame in enumerate(range(130, 150)):
            done += 120.0 if frame == 140 else 41.7
            frames.append(shot(index, frame, done))
        segment = P.play_segments(events)[0]
        summary = P.summarize_play(frames, events, segment, {**PROJECT, "fps": 24.0}, 41.667)
        self.assertEqual(summary["slow_gt_62_5"], 1)
        self.assertEqual(summary["slow_gt_100"], 1)
        self.assertEqual(summary["slow_detail"][0]["where"], "prefetch:186")
        self.assertEqual(summary["by_location"]["prefetch:186"]["slow"], 1)
        self.assertEqual(summary["interval_ms"]["max"], 120.0)


class ValidityTests(unittest.TestCase):
    def test_a_clean_session_is_valid(self):
        for mode in MODES:
            self.assertEqual(V.session_validity(session(mode)), (True, []))

    def test_saturation_is_an_invalid_run_not_a_win(self):
        bad = session("async")
        bad["status"]["asset_io"]["pool"]["saturated"] = 1
        valid, reasons = V.session_validity(bad)
        self.assertFalse(valid)
        self.assertIn("saturated", reasons[0])

    def test_the_arm_must_be_the_mode_in_effect(self):
        wrong = session("async")
        wrong["status"]["asset_io"]["mode"] = "sync"
        self.assertFalse(V.session_validity(wrong)[0])

    def test_lost_events_a_failed_play_an_error_status_and_a_missing_warm_each_invalidate(self):
        self.assertFalse(V.session_validity(session("sync", events_lost=3))[0])
        self.assertFalse(V.session_validity(session("sync", plays_ok=False))[0])
        self.assertFalse(V.session_validity(session("sync", events=[event("asset_io", 1, status=503)]))[0])
        self.assertTrue(V.session_validity(session("sync", events=[event("asset_io", 1, status=206)]))[0])
        cold = session("sync")
        cold["status"]["stats"]["warmed"] = 0
        self.assertFalse(V.session_validity(cold)[0])


class RedactionTests(unittest.TestCase):
    def test_redaction_includes_the_final_component_with_spaces_or_directly_under_a_known_root(self):
        for path in ('/Users/Alice/secret clip.ply', str(pathlib.Path.home()) + '/customer.json',
                     tempfile.gettempdir() + '/secret clip.ply', '/customer.json'):
            with self.subTest(path=path):
                result = V.redact('failed: ' + path + ': boom')
                self.assertNotIn('secret', result)
                self.assertNotIn('clip.ply', result)
                self.assertNotIn('customer.json', result)
                self.assertIn('boom', result)

    def test_a_published_failure_text_names_no_directory(self):
        text = V.redact("RuntimeError('exited: /Users/someone/private/dir/editor.log: boom')")
        self.assertNotIn("/Users/someone", text)
        self.assertNotIn("editor.log", text, "not even the file name")
        spaced = V.redact("failed: /Users/John Smith/private dir/clip.ply: boom")
        self.assertNotIn("John", spaced)
        self.assertNotIn("Smith", spaced)
        self.assertNotIn("clip", spaced)
        self.assertIn("boom", spaced, "the reason survives")
        self.assertNotIn(str(pathlib.Path.home()), V.redact(f"open {pathlib.Path.home()}/a b/c.png failed"))
        self.assertLessEqual(len(V.redact("x" * 1000)), 240)
        valid, reasons = V.session_validity(session("sync", error="failed at /Users/someone/private/clip.ply"))
        self.assertFalse(valid)
        self.assertNotIn("/Users/someone", " ".join(reasons))


class EditorExitTests(unittest.TestCase):
    def test_an_editor_that_ended_by_itself_invalidates_the_session_and_its_log_is_redacted(self):
        gone = session("sync", editor_exit={"alive": False, "returncode_before_stop": -11, "log_lines": []})
        valid, reasons = V.session_validity(gone)
        self.assertFalse(valid)
        self.assertTrue(any("-11" in reason for reason in reasons))
        self.assertTrue(V.session_validity(session("sync", editor_exit={"alive": True, "returncode_before_stop": None, "log_lines": []}))[0])

    def test_the_log_tail_names_files_by_their_last_component_only(self):
        with tempfile.TemporaryDirectory() as folder:
            log = os.path.join(folder, "editor.log")
            Path(log).write_text("fine\nthread panicked at /Users/someone/private/clip-a.ply: boom\n")

            class Process:
                def poll(self):
                    return -6

            class Lab:
                pass

            lab = Lab()
            lab.process = Process()
            lab.log_path = log
            lab.log = open(log, "a")
            seen = S.editor_exit(lab)
            lab.log.close()
            self.assertEqual(seen["returncode_before_stop"], -6)
            self.assertFalse(seen["alive"])
            self.assertNotIn("clip-a", seen["log_lines"][0])
            self.assertNotIn("/Users/someone", seen["log_lines"][0])


class LoadClassTests(unittest.TestCase):
    def test_quiet_is_decided_by_thresholds_fixed_before_the_measurement(self):
        self.assertEqual(ST.quiet_class([0.5, 1.0, 2.0, 1.5]), "quiet")
        self.assertEqual(ST.quiet_class([5.0, 6.0, 7.0]), "loaded")
        self.assertEqual(ST.quiet_class([0.5] * 18 + [20.0, 20.0]), "loaded", "a heavy tail is not a quiet machine")
        self.assertEqual(ST.quiet_class([]), "unknown")


class ChartTests(unittest.TestCase):
    def test_the_charts_are_well_formed_svg(self):
        gaps = [(i, i + 1, i * 40.0, (i + 1) * 40.0, 40.0 + (80 if i == 140 else 0)) for i in range(130, 160)]
        ET.fromstring(C.svg_gaps("t", PROJECT, {"sync": gaps, "async": gaps}))
        ET.fromstring(C.svg_gaps("t", PROJECT, {}))
        events = [event("asset_cb", 100, recv_ms=90.0, callback_ms=8.0, mode="sync", outcome="answered"),
                  event("asset_io", 110, read_ms=6.0, respond_ms=1.0, bytes=5, mode="async"),
                  event("tick", 120, late_ms=30.0, due_ms=90.0, ran_ms=120.0),
                  event("prefetch", 130, total_ms=5.0, plans_ahead_ms=4.0),
                  event("sync", 140, total_ms=3.0)]
        ET.fromstring(C.svg_spans("s", events, [shot(1, 1, 80), shot(2, 2, 180)], 0, 200))


def transit_frame(id, submitted, total, runtime_total=1.0, shown=None):
    return {"id": id, "shown_frame": id if shown is None else shown, "submitted_ms": submitted, "done_ms": submitted + total,
            "submit_to_done_ms": total, "transit_ms": max(0.0, total - runtime_total), "runtime_total_ms": runtime_total, "interactive": True}


def page_event(id, queue=0.0, held=None, loop_max=0.0, ahead=None, ipc_wait=0.2):
    held = queue + 1.0 if held is None else held
    return event("frame_page", 0, id=id, ipc_wait_ms=ipc_wait, page={"queue_ms": queue, "held_ms": held, "ahead": ahead or {"frames": 0, "warms": 0, "loads": 0},
                                                                     "loop": {"max_ms": loop_max, "over": 0, "n": 10}})


def evaluation(sent, ms):
    return {"sent_ms": sent, "done_ms": sent + ms, "eval_ms": ms, "used": False}


class TransitTests(unittest.TestCase):
    def test_a_page_that_held_the_frame_in_its_queue_with_a_stalled_loop_is_named_so(self):
        e = page_event(1, queue=110, held=111, loop_max=105)["fields"]
        d = TR.decompose(transit_frame(1, 1000, 130), e, evaluation(1000.2, 3))
        self.assertEqual((d["dominant"], d["detail"]), ("page_queue", "event_loop_stalled"))

    def test_a_queue_wait_with_a_quiet_loop_and_work_ahead_is_queued_work_not_a_stall(self):
        e = page_event(2, queue=90, held=91, loop_max=5, ahead={"frames": 0, "warms": 1, "loads": 0})["fields"]
        d = TR.decompose(transit_frame(2, 0, 100), e, evaluation(0.1, 2))
        self.assertEqual((d["dominant"], d["detail"]), ("page_queue", "behind_queued_work"))

    def test_time_the_page_and_the_host_do_not_account_for_is_delivery_and_the_eval_says_which_way(self):
        e = page_event(3, queue=0.5, held=2.0, ipc_wait=0.3)["fields"]
        inbound = TR.decompose(transit_frame(3, 0, 120), e, evaluation(0.1, 110))
        self.assertEqual((inbound["dominant"], inbound["detail"]), ("delivery", "in_dominant"))
        outbound = TR.decompose(transit_frame(3, 0, 120), e, evaluation(0.1, 4))
        self.assertEqual((outbound["dominant"], outbound["detail"]), ("delivery", "out_or_later_dominant"))
        self.assertAlmostEqual(inbound["parts"]["delivery"], 120 - 0.1 - 0.5 - 1.5 - 0.3, places=1)

    def test_a_request_that_waited_for_the_window_before_it_was_even_sent_is_the_host_not_the_page(self):
        e = page_event(4, queue=0.2, held=1.2, ipc_wait=0.1)["fields"]
        d = TR.decompose(transit_frame(4, 0, 90), e, evaluation(85.0, 3))
        self.assertEqual(d["dominant"], "host_pre_send")

    def test_the_wait_of_the_message_for_the_windows_task_is_its_own_part(self):
        e = page_event(5, queue=0.2, held=1.2, ipc_wait=70)["fields"]
        d = TR.decompose(transit_frame(5, 0, 80), e, evaluation(0.1, 3))
        self.assertEqual(d["dominant"], "ipc_wait")

    def test_a_picture_without_the_page_account_is_never_given_a_delivery_figure(self):
        d = TR.decompose(transit_frame(6, 0, 120), None, evaluation(0.1, 5))
        self.assertIsNone(d["parts"]["delivery"])
        self.assertIsNone(d["parts"]["page_queue"])

    def test_a_late_picture_with_no_part_that_stands_out_is_spread_not_blamed(self):
        e = page_event(7, queue=20, held=40, ipc_wait=15)["fields"]
        d = TR.decompose(transit_frame(7, 0, 100), e, evaluation(10.0, 20))
        self.assertIn(d["dominant"], ("spread", "delivery"))
        if d["dominant"] == "delivery":
            self.assertGreaterEqual(d["parts"]["delivery"], TR.DOMINANT_MIN_MS)

    def test_completions_are_matched_to_submissions_in_order_and_only_once(self):
        events = [event("eval_done", 20, kind="frame.render", ms=5), event("eval_done", 30, kind="frame.warm", ms=1), event("eval_done", 70, kind="frame.render", ms=6)]
        evals = TR.eval_events(events)
        first = TR.match_eval(evals, 10)
        second = TR.match_eval(evals, 10)
        self.assertEqual((first["eval_ms"], second["eval_ms"]), (5, 6), "the warm script is not a frame's")
        self.assertIsNone(TR.match_eval(evals, 10))


def window_record(**overrides):
    """A window play as `window_validity` reads it (a session plus the mode under `io` and a frame that carries the page's account)."""
    return session("async", io="async", events=[event("frame_page", 5, id=1)], **overrides)


DEFECTS = {
    "an error": lambda r: r.update(error="RuntimeError('boom')"),
    "the editor ended by itself": lambda r: r.update(editor_exit={"alive": False, "returncode_before_stop": -11, "log_lines": []}),
    "another mode than asked": lambda r: r["status"]["asset_io"].update(mode="sync"),
    "a saturated read queue": lambda r: r["status"]["asset_io"]["pool"].update(saturated=1),
    "a panicked read": lambda r: r["status"]["asset_io"]["pool"].update(panicked=1),
    "a stale read": lambda r: r["status"]["asset_io"]["pool"].update(stale=2),
    "lost trace events": lambda r: r.update(events_lost=3),
    "a play that did not run to its end": lambda r: r.update(plays_ok=False),
    "an HTTP error": lambda r: r["events"].append(event("asset_io", 9, status=503)),
    "an asset that failed to load": lambda r: r["events"].append(event("asset_state", 9, state="failed", timings=None)),
}


class OneValidityTests(unittest.TestCase):
    """`valid = ran to its end AND no failed asset AND no HTTP error AND ...`: the same core decides a whole-project session and a window."""

    JUDGES = {"session": lambda r: V.session_validity(r), "window": lambda r: V.window_validity(r)}

    def test_a_clean_record_is_valid_for_both_judges(self):
        for name, judge in self.JUDGES.items():
            record = window_record()
            self.assertEqual(judge(record), (True, []), name)

    def test_every_defect_invalidates_a_session_and_a_window_alike(self):
        for defect, apply in DEFECTS.items():
            for name, judge in self.JUDGES.items():
                with self.subTest(defect=defect, judge=name):
                    record = window_record()
                    record["status"]["asset_io"]["pool"] = {"saturated": 0, "panicked": 0, "stale": 0}
                    apply(record)
                    valid, reasons = judge(record)
                    self.assertFalse(valid)
                    self.assertTrue(reasons)

    def test_an_asset_the_page_could_not_load_is_a_reason_that_names_how_many(self):
        record = window_record()
        record["events"] += [event("asset_state", 9, state="failed"), event("asset_state", 10, state="failed"), event("asset_state", 11, state="ready")]
        valid, reasons = V.session_validity(record)
        self.assertFalse(valid)
        self.assertEqual([r for r in reasons if "failed" in r], ["2 asset load(s) failed in the page"])
        ready = window_record()
        ready["events"] += [event("asset_state", 9, state="loading"), event("asset_state", 10, state="ready")]
        self.assertTrue(V.session_validity(ready)[0])

    def test_what_is_the_experiments_own_stays_with_it(self):
        no_warm = window_record()
        no_warm["status"]["stats"]["warmed"] = 0
        self.assertFalse(V.session_validity(no_warm)[0], "a whole-project session needs the scene warm")
        self.assertTrue(V.window_validity(no_warm)[0], "a window does not judge it (the first-use experiment does)")
        no_page = session("async", io="async", events=[])
        self.assertFalse(V.window_validity(no_page)[0], "a window needs the page's account")
        self.assertTrue(V.session_validity(no_page)[0])


class FakeLab:
    """The editor as `play` asks it: `statuses` are what `playback.status` answers in turn (the last repeats)."""

    def __init__(self, statuses, report=None):
        self.statuses, self.report, self.stopped = list(statuses), report or {"ticks": 10, "skipped_frames": 0, "max_gap_frames": 1}, 0

    def op(self, request):
        if request["op"] == "playback.status":
            return {"ok": True, "data": self.statuses.pop(0) if len(self.statuses) > 1 else self.statuses[0]}
        if request["op"] == "playback.stop":
            self.stopped += 1
            return {"ok": True, "data": {"report": self.report}}
        return {"ok": True}


class FakeWarm:
    @staticmethod
    def retry_while_preparing(call, *rest, **named):
        return call()


def clean_state():
    return {"cursor": 0, "events": [], "frames": {}, "lost": 0, "errors": [], "loadavg": [], "status": {}}


class PlayTests(unittest.TestCase):
    def setUp(self):
        for patch in (mock.patch.object(S.time, "sleep"), mock.patch.object(S.time, "time", side_effect=itertools.count(0.0, 1.0))):
            patch.start()
            self.addCleanup(patch.stop)

    def play(self, lab, finished, state=None, seconds=1.0):
        state = state if state is not None else clean_state()
        return S.play(FakeWarm, lab, state, lambda *a: True, "first", 0, seconds, finished, poll=0), state

    def test_a_play_that_ends_by_itself_gives_the_counters_of_the_host_report(self):
        lab = FakeLab([{"state": "playing"}, {"state": "stopped"}])
        counters, state = self.play(lab, lambda status: status.get("state") != "playing")
        self.assertEqual(counters, {"ticks": 10, "skipped_frames": 0, "max_gap_frames": 1})
        self.assertEqual(state["errors"], [])

    def test_a_play_that_runs_out_of_time_is_not_a_play_it_is_none_with_the_reason(self):
        lab = FakeLab([{"state": "playing"}])
        counters, state = self.play(lab, lambda status: False)
        self.assertIsNone(counters)
        self.assertEqual(lab.stopped, 1, "the editor is stopped all the same")
        self.assertEqual(len(state["errors"]), 1)
        self.assertIn("did not end within", state["errors"][0])

    def test_the_whole_project_twice_stops_at_the_first_play_that_did_not_end_and_says_so(self):
        project = {"length": 48, "fps": 24.0}
        record = {}
        lab = FakeLab([{"state": "playing"}])
        state = clean_state()
        ran = S.whole_project_twice(FakeWarm, project)(lab, state, lambda *a: True, record)
        self.assertFalse(ran)
        self.assertEqual(record["played"], {"first": None})
        self.assertEqual(lab.stopped, 1, "no second play after a first that never ended")

    def test_the_whole_project_twice_ran_when_both_plays_ended(self):
        lab = FakeLab([{"state": "stopped"}])
        record = {}
        self.assertTrue(S.whole_project_twice(FakeWarm, {"length": 48, "fps": 24.0})(lab, clean_state(), lambda *a: True, record))
        self.assertEqual(sorted(record["played"]), ["first", "repeat"])

    def test_a_window_that_stopped_before_its_end_did_not_run_to_its_end(self):
        lab = FakeLab([{"state": "playing", "frame": 120}, {"state": "stopped", "frame": 150}])
        state, record = clean_state(), {}
        self.assertFalse(S.one_window(FakeWarm, {"fps": 24.0}, (100, 200))(lab, state, lambda *a: True, record))
        self.assertEqual(record["window_last_frame"], 150)
        self.assertIn("before its end 200", state["errors"][0])
        self.assertFalse(V.window_validity({**window_record(), "plays_ok": False})[0])

    def test_a_window_whose_playhead_reached_its_end_ran_to_it(self):
        lab = FakeLab([{"state": "playing", "frame": 150}, {"state": "playing", "frame": 200}])
        state, record = clean_state(), {}
        self.assertTrue(S.one_window(FakeWarm, {"fps": 24.0}, (100, 200))(lab, state, lambda *a: True, record))
        self.assertEqual((record["window_last_frame"], state["errors"]), (200, []))


if __name__ == "__main__":
    unittest.main()
