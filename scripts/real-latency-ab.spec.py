"""Pure-logic tests of the BL157.1 real-project A/B harness; no editor, GPU, runtime page or private media is started."""
import contextlib
import importlib.util
import io
import json
import sys
import tempfile
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location("real_latency_ab", Path(__file__).with_name("real-latency-ab.py"))
ab = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ab)

PROJECT = {"length": 600, "fps": 24.0, "cuts": [186, 268, 410]}


class ReportTextTests(unittest.TestCase):
    def test_every_generated_table_has_as_many_delimiter_cells_as_header_cells(self):
        report = {"manifest": {"git": {"sha": "0" * 40, "dirty_files": 0}, "binary_sha256": "0" * 64, "runtime_sha256": "0" * 64,
                               "document_sha256": "0" * 64, "sources_present": 1, "sources_total": 1},
                  "project": PROJECT, "sessions": [], "all_valid": True, "n_comparable_pairs": 0, "pairs": {"comparable_quiet": []},
                  "arms": {"sync/quiet": {"sessions": 1, "plays": 2, "pictures": 1200, "gaps_gt_62_5": 1, "gaps_gt_100": 0, "with_ui_stall": 1,
                                          "with_read_overlap": 1, "with_page_work": 0, "with_slow_transit": 0,
                                          "with_neither_ui_stall_nor_page_work": 0, "in_prefetch_window": 1, "at_cut": 0,
                                          "at_video_entry": 0, "other": 0, "worst_gap_ms_per_session": [70.0]}}}
        lines = ab.markdown(report).splitlines()
        checked = 0
        for header, delimiter in zip(lines, lines[1:]):
            if delimiter.startswith("|---"):
                self.assertEqual(header.count("|"), delimiter.count("|"), header)
                checked += 1
        self.assertGreaterEqual(checked, 3)


class PreflightTests(unittest.TestCase):
    def test_media_stored_relative_to_the_document_is_found_from_any_working_directory(self):
        with tempfile.TemporaryDirectory() as folder:
            (Path(folder) / "managed-media").mkdir()
            (Path(folder) / "managed-media" / "clip-0.ply").write_bytes(b"x" * 10)
            document = Path(folder) / "project.tessel.json"
            sources = [{"type": "file", "path": "managed-media/clip-0.ply"}, {"type": "file", "path": "managed-media/missing.ply"}]
            document.write_text(json.dumps({"sequences": [{"id": "main", "length": 10, "rate": {"num": 24, "den": 1}, "clips": []}],
                                            "library": {"assets": [{"kind": "splat", "asset": {"source": source}} for source in sources]}}))
            shown = io.StringIO()
            with contextlib.redirect_stdout(shown):
                code = ab.cmd_preflight(SimpleNamespace(project=str(document)))
            summary_ = json.loads(shown.getvalue())
            self.assertEqual((summary_["sources_total"], summary_["sources_present"], code), (2, 1, 2), "the one that is there is found; the missing one is the only reason for 2")
            self.assertEqual([entry["present"] for entry in summary_["sources"]], [True, False])


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


class ArmTests(unittest.TestCase):
    def test_slow_gaps_are_counted_by_what_they_overlapped_and_invalid_sessions_are_left_out(self):
        def gap(where, ui, reads):
            return {"where": where, "context": {"ui_callback_ms": ui, "reads": reads}}

        play = {"pictures": 600, "slow_gt_62_5": 3, "slow_gt_100": 1, "interval_ms": {"max": 110.0},
                "slow_detail": [gap("prefetch:186", 30.0, 2), gap("other", 0, 0), gap("cut:186", 5.0, 1)]}
        good = {"valid": True, "mode": "sync", "quiet_class": "quiet", "plays": {"first": play, "repeat": play}}
        bad = {"valid": False, "mode": "sync", "quiet_class": "quiet", "plays": {"first": play}}
        play["slow_detail"][0]["context"]["page_decode_ms"] = 15.0
        play["slow_detail"][0]["picture"] = {"transit_ms": 80.0}
        group = ab.arm_groups([good, bad])["sync/quiet"]
        self.assertEqual((group["with_page_work"], group["with_slow_transit"]), (2, 2))
        self.assertEqual(group["with_neither_ui_stall_nor_page_work"], 4)
        self.assertEqual((group["sessions"], group["plays"], group["gaps_gt_62_5"]), (1, 2, 6))
        self.assertEqual((group["with_ui_stall"], group["with_read_overlap"]), (2, 4))
        self.assertEqual((group["in_prefetch_window"], group["at_cut"], group["at_video_entry"], group["other"]), (2, 2, 0, 2))
        self.assertEqual(group["worst_gap_ms_per_session"], [110.0])


    def test_a_gap_over_a_load_the_loader_did_not_split_still_counts_as_page_work(self):
        gap = {"where": "other", "context": {"ui_callback_ms": 0, "reads": 0, "page_decode_ms": 0.0, "page_fetch_ms": 0.0, "page_load_other_ms": 25.0, "page_warm_ms": 0.0}}
        play = {"pictures": 600, "slow_gt_62_5": 1, "slow_gt_100": 0, "interval_ms": {"max": 70.0}, "slow_detail": [gap]}
        group = ab.arm_groups([{"valid": True, "mode": "sync", "quiet_class": "quiet", "plays": {"first": play}}])["sync/quiet"]
        self.assertEqual((group["with_page_work"], group["with_neither_ui_stall_nor_page_work"]), (1, 0))


class PixelsCommandTests(unittest.TestCase):
    def parse(self, *argv):
        seen = {}
        with mock.patch.object(ab, "cmd_pixels", side_effect=lambda args: seen.update(vars(args)) or 0), \
                mock.patch.object(sys, "argv", ["real-latency-ab.py", "pixels", *argv]), contextlib.redirect_stderr(io.StringIO()):
            return ab.main(), seen

    def test_the_pixel_proof_takes_neither_a_lock_wait_nor_a_lower_disk_floor(self):
        code, seen = self.parse("--out", "o")
        self.assertEqual(code, 0)
        self.assertEqual(sorted(seen), ["binary", "command", "out"])
        for flag in (("--min-free-gib", "0"), ("--lock-wait", "5")):
            with self.assertRaises(SystemExit) as raised:
                self.parse("--out", "o", *flag)
            self.assertEqual(raised.exception.code, 2, flag)


if __name__ == "__main__":
    unittest.main()
