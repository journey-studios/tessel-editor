"""Spec of the pure parts of the BL064 runner (`python3 scripts/scene-plan-performance/run.spec.py`)."""
import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("run", Path(__file__).with_name("run.py"))
run = importlib.util.module_from_spec(spec)
spec.loader.exec_module(run)

SOURCE = """use a;
// @timeline-begin
use tessel::PlanTimeline;
// @timeline-end
fn keep() {}
    let x = json!({
        "a": 1,
        // @timeline-begin
        "timelineWarmNs": 2,
        // @timeline-end
        "b": 3,
    });
"""


class StripTimeline(unittest.TestCase):
    def test_removes_only_the_marked_regions(self):
        stripped = run.strip_timeline(SOURCE)
        self.assertNotIn("PlanTimeline", stripped)
        self.assertNotIn("timelineWarmNs", stripped)
        self.assertNotIn("@timeline", stripped)
        self.assertIn("fn keep() {}", stripped)
        self.assertIn('"a": 1,', stripped)
        self.assertIn('"b": 3,', stripped)

    def test_the_real_benchmark_strips_clean(self):
        bench = (run.REPO / run.BENCH).read_text()
        self.assertIn("@timeline-begin", bench)
        stripped = run.strip_timeline(bench)
        # Code only: the module docs still describe the metrics the pristine tree cannot produce.
        for code in ("PlanTimeline::new", "scene_plan::PlanTimeline", "timeline.build", "timelineWarmNs"):
            self.assertNotIn(code, stripped)
        self.assertEqual(stripped.count("@timeline"), 0)


def sample(build, warm, scale=1):
    return {
        "scenario": "s", "shape": {"nodes": 3}, "sampledFrames": 4, "reps": 2,
        "buildNs": build, "timelineWarmNs": warm, "timelineColdNs": warm * 2,
        "buildAndJsonNs": build * 5, "timelineAndJsonNs": build * 4,
        "nodeTimelineNewNs": 10 * scale, "cameraTimelineNewNs": 10 * scale, "toJsonStringNs": 7,
        "jsonBytesPerFrame": 100,
    }


class Aggregate(unittest.TestCase):
    def test_median_across_processes_and_ratios_from_the_medians(self):
        out = run.aggregate([sample(100, 10), sample(300, 30), sample(200, 20)])
        self.assertEqual(out["buildNs"], 200)
        self.assertEqual(out["buildNsRange"], [100, 300])
        self.assertEqual(out["timelineSpeedupWarm"], 10)
        self.assertEqual(out["timelineSpeedupCold"], 5)
        self.assertEqual(out["exportLoopSpeedup"], 1.25)
        self.assertEqual(out["timelineSetupShareOfBuild"], 20 / 200)
        self.assertEqual(out["repeats"], 3)

    def test_a_tree_without_timeline_has_no_timeline_ratios(self):
        row = {k: v for k, v in sample(100, 10).items() if not k.startswith("timeline") and k != "buildAndJsonNs"
               and k != "timelineAndJsonNs"}
        row["buildAndJsonNs"] = 500
        out = run.aggregate([row])
        self.assertNotIn("timelineSpeedupWarm", out)
        self.assertNotIn("exportLoopSpeedup", out)


if __name__ == "__main__":
    unittest.main()
