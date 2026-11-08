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


class CpuRateTests(unittest.TestCase):
    def test_the_rate_divides_by_the_measured_wall_time_not_a_constant(self):
        self.assertEqual(proof.cpu_rate(10.0, 13.0, 10.0), 0.3)
        # The same CPU over a longer wall time is a lower rate: nothing is added to the divisor.
        self.assertEqual(proof.cpu_rate(10.0, 13.0, 12.5), 0.24)

    def test_an_unknown_reading_a_bad_wall_time_or_a_restarted_process_is_none_never_zero(self):
        for args in ((None, 1.0, 5.0), (1.0, None, 5.0), (1.0, 2.0, None), (1.0, 2.0, 0), (1.0, 2.0, -3.0), (5.0, 2.0, 5.0)):
            self.assertIsNone(proof.cpu_rate(*args), args)

    def test_no_cpu_used_is_a_real_zero(self):
        self.assertEqual(proof.cpu_rate(4.0, 4.0, 8.0), 0.0)


class BufferMinutesTests(unittest.TestCase):
    def test_a_positive_finite_number_or_zero_is_accepted(self):
        self.assertEqual(proof.parse_buffer_minutes("3"), 3.0)
        self.assertEqual(proof.parse_buffer_minutes("0.5"), 0.5)
        self.assertEqual(proof.parse_buffer_minutes("0"), 0.0)

    def test_anything_else_is_refused_before_an_editor_starts(self):
        for text in ("abc", "", "-1", "nan", "inf", "-inf", None):
            with self.assertRaises(ValueError, msg=repr(text)):
                proof.parse_buffer_minutes(text)

    def test_more_than_the_fixture_covers_is_refused_naming_this_option(self):
        self.assertEqual(proof.parse_buffer_minutes("10", 600), 10.0)
        with self.assertRaises(ValueError) as caught:
            proof.parse_buffer_minutes("10.5", 600)
        self.assertIn("--buffer-minutes", str(caught.exception))
        self.assertNotIn("--seconds", str(caught.exception))

    def test_the_reading_step_is_bounded_and_a_short_run_is_not_stretched(self):
        self.assertEqual(proof.buffer_sample_every(3), 20.0)
        self.assertEqual(proof.buffer_sample_every(1), 10.0)
        self.assertEqual(proof.buffer_sample_every(0.01), 1.0)
        self.assertEqual(proof.buffer_sample_every(60), 20.0)


class BufferGrowthTests(unittest.TestCase):
    def samples(self, readings):
        return [{"rss_mib": value} for value in readings]

    def test_growth_is_judged_with_enough_known_readings(self):
        out = proof.buffer_growth(self.samples([100.0, 130.0, 120.0, 118.0, 119.0, 118.5]))
        self.assertTrue(out["judged"])
        self.assertEqual(out["growth_mib"], 18.5)
        self.assertEqual(out["growth_after_warmup_mib"], -1.5)

    def test_too_few_readings_judge_nothing_instead_of_passing_vacuously(self):
        out = proof.buffer_growth(self.samples([100.0, 101.0]))
        self.assertEqual(out, {"growth_mib": None, "growth_after_warmup_mib": None, "judged": False})

    def test_a_missing_reading_is_unknown_not_zero(self):
        # A zero in place of the unknown reading would read as a huge fake growth (or a fake shrink).
        out = proof.buffer_growth(self.samples([100.0, None, 101.0, 102.0, 103.0, 104.0]))
        self.assertFalse(out["judged"])
        self.assertIsNone(out["growth_mib"])


class SourceStateTests(unittest.TestCase):
    def test_a_binary_older_than_a_source_is_stale_and_unknown_is_unknown(self):
        self.assertTrue(proof.binary_is_stale(100.0, 200.0))
        self.assertFalse(proof.binary_is_stale(200.0, 100.0))
        self.assertFalse(proof.binary_is_stale(200.0, 200.0))
        self.assertIsNone(proof.binary_is_stale(None, 100.0))
        self.assertIsNone(proof.binary_is_stale(100.0, None))


class ExportOutcomeTests(unittest.TestCase):
    def test_the_command_refusing_is_told_apart_from_the_write_failing(self):
        refused = proof.export_outcome({"error": {"code": "perf_history_export_relative"}}, {"state": "idle"})
        self.assertEqual(refused, ("refused", "perf_history_export_relative"))
        failed = proof.export_outcome({"ok": True}, {"state": "failed", "error": {"code": "perf_history_export_exists"}})
        self.assertEqual(failed, ("failed", "perf_history_export_exists"))
        self.assertEqual(proof.export_code(refused), "perf_history_export_relative")
        self.assertEqual(proof.export_code(failed), "perf_history_export_exists")

    def test_done_pending_and_unknown_carry_no_error_code(self):
        self.assertEqual(proof.export_outcome({"ok": True}, {"state": "done", "bytes": 5}), ("done", None))
        self.assertEqual(proof.export_outcome({"ok": True}, {"state": "writing", "id": 2}), ("pending", None))
        self.assertEqual(proof.export_outcome({"ok": True}, None), ("unknown", None))
        for outcome in (("done", None), ("pending", None), ("unknown", None)):
            self.assertIsNone(proof.export_code(outcome))


class BufferDeadlineTests(unittest.TestCase):
    def test_last_wait_and_sampling_overhead_do_not_extend_the_run(self):
        self.assertEqual(proof.buffer_wait(20.0, 123.0, 120.0), 3.0)
        self.assertEqual(proof.buffer_wait(20.0, 123.0, 122.75), 0.25)
        self.assertEqual(proof.buffer_wait(20.0, 123.0, 124.0), 0.0)
        self.assertEqual(proof.buffer_wait(20.0, 123.0, 100.0), 20.0)


if __name__ == "__main__":
    unittest.main()
