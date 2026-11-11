"""Pure-logic tests of the BL157.2 transit probe; no editor, GPU, runtime page or private media is started."""
import importlib.util
import json
import os
import unittest
import tempfile
from types import SimpleNamespace
from unittest import mock
from unittest.mock import patch
from pathlib import Path

spec = importlib.util.spec_from_file_location("viewport_transit_probe", Path(__file__).with_name("viewport-transit-probe.py"))
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


def frame(id, submitted, total, runtime_total=1.0, shown=None):
    return {"id": id, "shown_frame": id if shown is None else shown, "submitted_ms": submitted, "done_ms": submitted + total,
            "submit_to_done_ms": total, "transit_ms": max(0.0, total - runtime_total), "runtime_total_ms": runtime_total, "interactive": True}


def event(name, t, **fields):
    return {"kind": name, "t_ms": t, "seq": int(t), "fields": fields}


class BatchRebuildTests(unittest.TestCase):
    def test_the_markdown_exposes_a_partial_archive_warning(self):
        manifest = {'git': {'sha': 'a' * 40, 'dirty_files': 0}, 'binary_sha256': 'b' * 64,
                    'runtime_sha256': 'c' * 64, 'document_sha256': 'd' * 64, 'io': 'async',
                    'window': [1, 2], 'arm_a': 'control', 'arm_b': 'probe', 'pairs': 0,
                    'raw_rebuild': 'raw archive incomplete: summaries as saved'}
        text = probe.ab_markdown({'manifest': manifest, 'runs': [], 'verdict': 'inconclusive'})
        self.assertIn('raw archive incomplete: summaries as saved', text)

    def rebuild(self, raw_names):
        saved = [{'pair': 0, 'arm': arm, 'order': 'AB', 'summary': {'saved': arm}} for arm in ('A', 'B')]
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            (out / 'ab.json').write_text(json.dumps({'manifest': {}, 'runs': saved, 'verdict': 'old'}))
            for name in raw_names:
                (out / name).write_text(json.dumps({'name': name}))
            with patch.object(probe, 'arm_summary', side_effect=lambda record: {'raw': record['name']}), \
                    patch.object(probe, 'ab_verdict', return_value='recomputed'), \
                    patch.object(probe, 'ab_markdown', return_value='report'):
                probe.cmd_ab_report(SimpleNamespace(out=str(out)))
            return saved, json.loads((out / 'ab.json').read_text())

    def test_partial_raw_archive_preserves_every_saved_run(self):
        saved, report = self.rebuild(['pair-0-A.json'])
        self.assertEqual(report['runs'], saved)
        self.assertIn('incomplete', report['manifest']['raw_rebuild'])

    def test_old_files_from_another_batch_never_enter_the_saved_batch(self):
        _, report = self.rebuild(['pair-0-A.json', 'pair-0-B.json', 'pair-9-A.json'])
        self.assertEqual([(r['pair'], r['arm']) for r in report['runs']], [(0, 'A'), (0, 'B')])
        self.assertEqual([r['summary']['raw'] for r in report['runs']], ['pair-0-A.json', 'pair-0-B.json'])


class AbTests(unittest.TestCase):
    def run_of(self, arm, stalls, finish=None, valid=True):
        return {"pair": 0, "order": "AB", "arm": arm, "summary": {"valid": valid, "invalid_reasons": [], "class": "loaded", "pictures": 100, "gap_ms": {"p50": 42, "p95": 50, "max": 90},
                "slow_gaps": 1, "ui_blocks": 0, "page_stalls_ge_40": {"n": stalls, "max_ms": 100 if stalls else None},
                "gpu_finish": {"n": 100, "ge_20": [], "max_ms": finish}, "gaps_over": []}}

    def test_four_valid_runs_per_arm_are_needed_and_a_stall_that_never_reproduced_proves_nothing(self):
        self.assertTrue(probe.ab_verdict([self.run_of("A", 1), self.run_of("B", 0, 90)] * 3).startswith("INCONCLUSIVE: fewer than 4"))
        self.assertTrue(probe.ab_verdict([self.run_of("A", 0), self.run_of("B", 0, 0)] * 4).startswith("INCONCLUSIVE: the manipulation"))

    def test_a_wait_that_reaches_40_ms_where_a_stalls_is_consistent_never_proof_and_a_short_wait_is_inconclusive(self):
        moved = [self.run_of("A", 1), self.run_of("B", 0, 95)] * 4
        self.assertTrue(probe.ab_verdict(moved).startswith("CONSISTENT with GPU work"))
        self.assertIn("not a display or photon latency", probe.ab_verdict(moved))
        short = [self.run_of("A", 1), self.run_of("B", 1, 20)] * 4
        self.assertTrue(probe.ab_verdict(short).startswith("INCONCLUSIVE: the page's wait for the GPU reached 40 ms in fewer"))
        no_stall = [self.run_of("A", 0), self.run_of("B", 0, 95)] * 4
        self.assertTrue(probe.ab_verdict(no_stall).startswith("INCONCLUSIVE: arm A did not reproduce"))

    def test_a_wait_that_never_waited_is_a_failed_manipulation_not_a_refutation(self):
        verdict = probe.ab_verdict([self.run_of("A", 2), self.run_of("B", 2, 0)] * 4)
        self.assertTrue(verdict.startswith("INCONCLUSIVE: the manipulation did not work"), verdict)

    def test_an_invalid_run_is_not_counted_for_its_arm(self):
        runs = [self.run_of("A", 1), self.run_of("B", 0, 95)] * 3 + [self.run_of("A", 1), self.run_of("B", 0, 95, valid=False)]
        self.assertTrue(probe.ab_verdict(runs).startswith("INCONCLUSIVE: fewer than 4"))


class IterationTests(unittest.TestCase):
    def record(self, gap):
        events = [event("play_command", 0, from_=1), event("play_clock_started", 5),
                  event("eval_done", 11, kind="frame.render", ms=2), event("eval_done", 60 + gap, kind="frame.render", ms=2),
                  {"kind": "frame_page", "t_ms": 50, "seq": 3, "fields": {"id": 1, "ipc_wait_ms": 0.1, "page": {"queue_ms": 0.1, "held_ms": 1, "ahead": {}, "loop": {"max_ms": 1}}}},
                  {"kind": "frame_page", "t_ms": 90 + gap, "seq": 4, "fields": {"id": 2, "ipc_wait_ms": 0.1, "page": {"queue_ms": gap - 5, "held_ms": gap - 4, "ahead": {}, "loop": {"max_ms": gap - 8}}}}]
        frames = [frame(1, 10, 40), frame(2, 60, 30 + gap)]
        return {"iteration": 0, "io": "async", "window": [100, 200], "events": events, "frames": frames, "events_lost": 0, "plays_ok": True,
                "status": {"asset_io": {"mode": "async", "pool": {"saturated": 0}}}, "external_cores": [1.0, 1.0], "editor_exit": {"alive": True}}

    def test_a_slow_gap_makes_the_loop_red_and_a_clean_window_does_not(self):
        red = probe.analyze_iteration(self.record(gap=110))
        self.assertEqual(red["plays"][0]["slow_gaps"], 1)
        self.assertEqual(red["plays"][0]["slow_detail"][0]["picture"]["dominant"], "page_queue")
        clean = probe.analyze_iteration(self.record(gap=1))
        self.assertEqual(clean["plays"][0]["slow_gaps"], 0)
        report = probe.build_report([self.record(gap=110)], None, {})
        self.assertEqual(report["red_iterations"], [0])

    def test_a_prefetch_that_holds_the_window_thread_is_red_with_where_its_time_went_and_a_starved_play_is_red_too(self):
        record = self.record(gap=1)
        record["events"] += [event("prefetch_asset", 3100, kind="splat", bytes=900, fs_ms=0.2, send_ms=3099.0, ok=True),
                             event("prefetch", 3101, plans=1, assets=1, loads_started=1, warms_started=0, plans_ahead_ms=0.01, prefetch_ms=3100.0, warm_ms=0.0, total_ms=3100.5)]
        blocks = probe.analyze_iteration(record)["plays"][0]["ui_blocks"]
        self.assertEqual(len(blocks), 1)
        self.assertEqual(blocks[0]["assets"][0]["send_ms"], 3099.0)
        self.assertEqual(probe.build_report([record], None, {})["red_iterations"], [0])
        quiet = self.record(gap=1)
        quiet["window"] = [100, 108]
        self.assertEqual(probe.analyze_iteration(quiet)["plays"][0]["ui_blocks"], [])
        self.assertFalse(probe.analyze_iteration(quiet)["plays"][0]["starved"])
        starved = self.record(gap=1)
        starved["frames"] = starved["frames"][:1]
        starved["window"] = [0, 400]
        self.assertTrue(probe.analyze_iteration(starved)["plays"][0]["starved"])
        self.assertEqual(probe.build_report([starved], None, {})["red_iterations"], [0])

    def test_a_settle_picture_of_the_frame_the_play_stopped_on_is_not_a_playback_gap(self):
        record = self.record(gap=1)
        record["frames"][1]["shown_frame"] = record["frames"][0]["shown_frame"]
        record["frames"][1]["done_ms"] += 400
        self.assertEqual(probe.analyze_iteration(record)["plays"][0]["slow_gaps"], 0)

    def test_an_iteration_without_the_page_account_or_with_a_saturated_pool_is_invalid_not_good(self):
        record = self.record(gap=1)
        record["events"] = [e for e in record["events"] if e["kind"] != "frame_page"]
        record["status"]["asset_io"]["pool"]["saturated"] = 2
        valid, reasons = probe.window_validity(record)
        self.assertFalse(valid)
        self.assertTrue(any("page account" in r for r in reasons) and any("saturated" in r for r in reasons))

    def test_the_default_window_covers_the_prefetch_and_the_first_cut(self):
        self.assertEqual(probe.default_window({"cuts": [186, 268], "fps": 24.0, "length": 600}), (114, 210))
        self.assertEqual(probe.default_window({"cuts": [], "fps": 24.0, "length": 600}), (0, 24))

    def test_the_report_is_json_and_markdown(self):
        report = probe.build_report([self.record(gap=110)], None, {"git": {"sha": "0" * 40, "dirty_files": 0}, "binary_sha256": "a" * 64,
                                                       "runtime_sha256": "b" * 64, "document_sha256": "c" * 64, "io": "async", "window": [1, 2]})
        json.dumps(report)
        self.assertIn("page_queue", probe.markdown(report))


class ArmsAreDistinctTests(unittest.TestCase):
    def test_the_two_arms_differ_even_when_the_shell_exports_the_switch_the_probe_turns_on(self):
        proof = probe.warm_proof()
        with mock.patch.dict(os.environ, {probe.GPU_FINISH: "1"}):
            made = {arm: proof.lab_environment(os.environ, "/lab/h", probe.session_env("async", probe.AB_ENV[arm])) for arm in ("A", "B")}
        self.assertEqual((made["A"][probe.GPU_FINISH], made["B"][probe.GPU_FINISH]), ("0", "1"), "arm A says 0 itself, nothing inherited can turn it on")
        self.assertNotEqual(made["A"], made["B"])


if __name__ == "__main__":
    unittest.main()
