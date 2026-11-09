#!/usr/bin/env python3
"""Regressions for complete event coverage and unavailable monitor peaks."""
import importlib.util
import pathlib
import unittest

spec = importlib.util.spec_from_file_location("smoke", pathlib.Path(__file__).with_name("fx-history-smoke-proof.py"))
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)


def document(events):
    return {"schema": "tessel.perf-history", "schema_version": 1, "events": events, "events_dropped": 0, "recording": {"started_at_ms": 10}, "window": {"start_ms": 0}}


class Evidence(unittest.TestCase):
    def test_forbidden_event_older_than_the_ui_tail_is_detected(self):
        events = [{"kind": "recording_started"}, {"kind": "request_failed"}] + [{"kind": "seek"}] * 35
        self.assertNotIn("request_failed", smoke.kinds(events[-30:]))
        complete, kinds = smoke.complete_events(document(events))
        self.assertTrue(complete)
        self.assertIn("request_failed", [kind for kind in kinds if kind in smoke.FORBIDDEN])

    def test_frozen_export_can_hold_more_than_thirty_events(self):
        events = [{"kind": "seek", "at_ms": n} for n in range(45)]
        compact = {"events": {"held": 45, "dropped": 0, "newest": events[-30:]}}
        self.assertTrue(smoke.frozen_events_match(compact, document(events)))
        compact["events"]["held"] = 44
        self.assertFalse(smoke.frozen_events_match(compact, document(events)))
        compact["events"]["held"] = 45
        compact["events"]["newest"] = events[:30]
        self.assertFalse(smoke.frozen_events_match(compact, document(events)))

    def test_incomplete_or_malformed_evidence_is_never_complete(self):
        for change in ({"events_dropped": 1}, {"events": None}, {"events": [None]}, {"window": {"start_ms": 11}}, {"recording": {}}):
            with self.subTest(change=change):
                data = document([])
                data.update(change)
                self.assertFalse(smoke.complete_events(data)[0])

    def test_null_or_missing_monitor_peaks_give_a_failed_count_check(self):
        for peak in (None, {}, {"count": None}, {"count": "1"}, {"count": float("nan")}, {"count": True}):
            with self.subTest(peak=peak):
                self.assertEqual(smoke.monitor_sample_count({"view": {"peaks": {"monitor_frames": peak}}}), 0)
        self.assertEqual(smoke.monitor_sample_count({}), 0)
        self.assertEqual(smoke.monitor_sample_count({"view": {"peaks": {"monitor_frames": {"count": 3}}}}), 3)


if __name__ == "__main__":
    unittest.main()
