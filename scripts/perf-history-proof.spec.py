"""Pure-logic tests of the history proof (F049 / BL145.2); no editor, GPU or runtime page is started."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("perf_history_proof", Path(__file__).with_name("perf-history-proof.py"))
proof = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proof)


class CpuTimeTests(unittest.TestCase):
    def test_ps_cputime_forms_are_read(self):
        self.assertEqual(proof.parse_cpu_time("0:01.50"), 1.5)
        self.assertEqual(proof.parse_cpu_time("2:03.25"), 123.25)
        self.assertEqual(proof.parse_cpu_time("1:02:03.00"), 3723.0)
        self.assertEqual(proof.parse_cpu_time("1-00:00:01"), 86401.0)

    def test_anything_else_is_unknown_not_zero(self):
        for text in ("", "abc", "1:2:3:4", None):
            self.assertIsNone(proof.parse_cpu_time(text), text)


class ArmOrderTests(unittest.TestCase):
    def test_every_arm_takes_every_position_over_rounds(self):
        first = proof.arm_order(0)
        second = proof.arm_order(1)
        self.assertEqual(sorted(first), ["hidden", "off", "visible"])
        self.assertEqual(first, list(reversed(second)))
        self.assertNotEqual(first[0], second[0])


class SummaryTests(unittest.TestCase):
    def test_failed_runs_are_out_of_the_means_and_missing_values_stay_missing(self):
        runs = [
            {"arm": "off", "ok": True, "ticks": 100, "cpu_s_per_s": 0.5},
            {"arm": "off", "ok": True, "ticks": 200},
            {"arm": "off", "ok": False, "ticks": 9999},
            {"arm": "hidden", "ok": True, "ticks": 90},
        ]
        out = proof.summarize(runs)
        self.assertEqual(out["off"]["runs"], 2)
        self.assertEqual(out["off"]["ticks"], 150.0)
        self.assertEqual(out["off"]["cpu_s_per_s"], 0.5)
        self.assertIsNone(out["hidden"]["cpu_s_per_s"])
        self.assertEqual(out["visible"]["runs"], 0)
        self.assertIsNone(out["visible"]["ticks"])


if __name__ == "__main__":
    unittest.main()
