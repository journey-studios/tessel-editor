#!/usr/bin/env python3
"""BL064 benchmark runner: cost of building a ScenePlan for many frames, and the size of its JSON.

It builds `crates/core/tests/scene_plan_performance_bench.rs` in release, runs the resulting test
binary directly (so Cargo is not held while measuring), repeats the process, and reports medians with
their spread plus the peak resident size of each process. With `--base TREE` it also measures a
pristine checkout of main (the benchmark file is overlaid on it with the PlanTimeline parts removed),
so "before" is a different build, not the same binary asked a different question.

    python3 scripts/scene-plan-performance/run.py --repeats 5 --out /tmp/te-r6-result.json
    python3 scripts/scene-plan-performance/run.py --base /path/to/pristine-main --repeats 5

Cargo is invoked through the serial wrapper when it exists (``~/.cache/tessel-editor/cargo-serial.py``,
or ``$TESSEL_CARGO``), otherwise through ``./scripts/cargo``, in the tree being measured. Nothing is
cleaned, no HOME is changed, no dependency is touched. Synthetic scenes only: no private media.
"""
import argparse
import json
import os
import platform
import re
import shlex
import shutil
import statistics
import subprocess
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
BENCH = "crates/core/tests/scene_plan_performance_bench.rs"
SUPPORT = "crates/core/tests/scene_plan_performance_support"
SCENARIOS = ["small", "medium", "large", "camera-heavy", "channel-heavy", "wide-flat"]

# Metrics reported as a ratio of two others are recomputed from the medians, never medianed again.
DERIVED = {
    "timelineSetupShareOfBuild": lambda m: (m["nodeTimelineNewNs"] + m["cameraTimelineNewNs"]) / m["buildNs"],
}
TIMELINE_RATIOS = {
    "timelineSpeedupWarm": ("buildNs", "timelineWarmNs"),
    "timelineSpeedupCold": ("buildNs", "timelineColdNs"),
    "exportLoopSpeedup": ("buildAndJsonNs", "timelineAndJsonNs"),
}


def strip_timeline(source: str) -> str:
    """The benchmark without the parts that name `PlanTimeline` (for a tree that does not have it)."""
    return re.sub(r"[ \t]*// @timeline-begin\n.*?[ \t]*// @timeline-end\n", "", source, flags=re.S)


def median(values):
    return statistics.median(values)


def aggregate(runs):
    """Median of every numeric metric across process repeats, plus min/max of the headline ones."""
    first = runs[0]
    out = {key: first[key] for key in ("scenario", "shape", "sampledFrames", "reps")}
    numeric = [k for k, v in first.items() if isinstance(v, (int, float)) and k not in out]
    for key in numeric:
        out[key] = median([run[key] for run in runs if key in run])
    for key in ("buildNs", "timelineWarmNs", "toJsonStringNs"):
        if key in first:
            values = [run[key] for run in runs]
            out[key + "Range"] = [min(values), max(values)]
    for key, (num, den) in TIMELINE_RATIOS.items():
        if num in out and den in out:
            out[key] = out[num] / out[den]
    if "nodeTimelineNewNs" in out:
        out["timelineSetupShareOfBuild"] = DERIVED["timelineSetupShareOfBuild"](out)
    sections = first.get("jsonBytesBySection")
    if sections:
        out["jsonBytesBySection"] = sections
    out["repeats"] = len(runs)
    return out


class Tree:
    def __init__(self, path: Path, cargo: list):
        self.path, self.cargo = path, cargo

    def cargo_run(self, *args, capture=False):
        command = [*self.cargo, *args]
        return subprocess.run(command, cwd=self.path, text=True, capture_output=capture, check=False)

    def build_bench(self) -> Path:
        result = self.cargo_run(
            "test", "--release", "-p", "tessel-editor-core", "--test", "scene_plan_performance_bench",
            "--no-run", "--message-format=json", capture=True,
        )
        if result.returncode != 0:
            sys.stderr.write(result.stdout[-4000:] + result.stderr[-4000:])
            raise SystemExit(f"build failed in {self.path}")
        executable = None
        for line in result.stdout.splitlines():
            if not line.startswith("{"):
                continue
            message = json.loads(line)
            if message.get("reason") == "compiler-artifact" and message.get("executable") \
                    and message["target"]["name"] == "scene_plan_performance_bench":
                executable = message["executable"]
        if not executable:
            raise SystemExit("could not find the benchmark executable in Cargo's output")
        return Path(executable)


def peak_rss_mib(command, env):
    """Peak RSS of a child, from `wait4`. macOS reports bytes, Linux KiB."""
    process = subprocess.Popen(command, env={**os.environ, **env}, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, text=True)
    stdout = process.stdout.read()
    stderr = process.stderr.read()
    _, status, usage = os.wait4(process.pid, 0)
    process.returncode = os.waitstatus_to_exitcode(status)
    if process.returncode != 0:
        sys.stderr.write(stdout[-3000:] + stderr[-3000:])
        raise SystemExit(f"{command[-1]} failed")
    divisor = 1024 * 1024 if sys.platform == "darwin" else 1024
    return stdout.splitlines(), usage.ru_maxrss / divisor


def measure_tree(tree: Tree, args, with_timeline: bool):
    executable = tree.build_bench()
    scenarios = args.scenarios
    cost_runs = {name: [] for name in scenarios}
    peaks = {name: [] for name in scenarios}
    for repeat in range(args.repeats):
        for name in scenarios:
            env = {"SCENE_PLAN_PERF_SCENARIO": name, "SCENE_PLAN_PERF_REPS": str(args.reps),
                   "SCENE_PLAN_PERF_FRAMES": str(args.frames)}
            lines, peak = peak_rss_mib(
                [str(executable), "report_plan_cost_and_size", "--ignored", "--exact", "--nocapture",
                 "--test-threads=1"], env)
            rows = [json.loads(l.split("SCENE_PLAN_PERF ", 1)[1]) for l in lines if "SCENE_PLAN_PERF {" in l]
            cost_runs[name].append(rows[0])
            peaks[name].append(peak)
            print(f"  repeat {repeat + 1}/{args.repeats} {name}: build {rows[0]['buildNs'] / 1e3:.1f} us/frame",
                  flush=True)
    memory = []
    for name in args.memory_scenarios:
        for mode in (["direct", "timeline"] if with_timeline else ["direct"]):
            env = {"SCENE_PLAN_PERF_SCENARIO": name, "SCENE_PLAN_PERF_MODE": mode,
                   "SCENE_PLAN_PERF_LONG": str(args.long)}
            lines, peak = peak_rss_mib(
                [str(executable), "report_memory_over_a_long_run", "--ignored", "--exact", "--nocapture",
                 "--test-threads=1"], env)
            row = [json.loads(l.split("SCENE_PLAN_PERF_MEMORY ", 1)[1]) for l in lines
                   if "SCENE_PLAN_PERF_MEMORY {" in l][0]
            row["peakRssMiB"] = peak
            memory.append(row)
            print(f"  memory {name}/{mode}: rss {row['rssKiBBefore']}->{row['rssKiBAfter']} KiB", flush=True)
    cost = []
    for name in scenarios:
        row = aggregate(cost_runs[name])
        row["peakRssMiBPerProcess"] = median(peaks[name])
        cost.append(row)
    return {"cost": cost, "memory": memory}


def git(path: Path, *args):
    result = subprocess.run(["git", *args], cwd=path, text=True, capture_output=True, check=False)
    return result.stdout.strip() if result.returncode == 0 else None


def provenance(path: Path, label: str):
    channel = re.search(r'channel\s*=\s*"([^"]+)"', (path / "rust-toolchain.toml").read_text())
    return {
        "label": label, "tree": str(path),
        "gitHead": git(path, "rev-parse", "HEAD"),
        "gitDirty": bool(git(path, "status", "--short")) if git(path, "rev-parse", "HEAD") else None,
        "rustToolchain": channel.group(1) if channel else None,
        "profile": "release (cargo test --release)", "host": platform.platform(),
        "cpus": os.cpu_count(), "python": platform.python_version(),
        "utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    }


def default_cargo():
    if os.environ.get("TESSEL_CARGO"):
        return shlex.split(os.environ["TESSEL_CARGO"])
    serial = Path.home() / ".cache/tessel-editor/cargo-serial.py"
    return ["python3", str(serial)] if serial.exists() else ["./scripts/cargo"]


def overlay_base(base: Path):
    """Put the benchmark (without PlanTimeline) and its fixtures into the pristine tree."""
    (base / BENCH).write_text(strip_timeline((REPO / BENCH).read_text()))
    target = base / SUPPORT
    if target.exists():
        shutil.rmtree(target)
    shutil.copytree(REPO / SUPPORT, target)


def render(label, result):
    lines = [f"### {label}", "",
             "| scenario | nodes | frames | build us | evaluate us | timeline-setup % | validate us | to_json us | "
             "json KiB/frame | json MiB/edit | warm us | cold us | warm x | export-loop x |",
             "|---|---|---|---|---|---|---|---|---|---|---|---|---|---|"]
    for row in result["cost"]:
        us = lambda key: f"{row[key] / 1e3:.1f}" if key in row else "-"
        ratio = lambda key: f"{row[key]:.2f}" if key in row else "-"
        lines.append(
            f"| {row['scenario']} | {row['shape']['nodes']} | {row['shape']['editFrames']} | {us('buildNs')} | "
            f"{us('evaluateFrameNs')} | {row['timelineSetupShareOfBuild'] * 100:.1f} | {us('validateNs')} | "
            f"{us('toJsonStringNs')} | {row['jsonBytesPerFrame'] / 1024:.1f} | "
            f"{row['jsonBytesAllEditFrames'] / 1048576:.1f} | {us('timelineWarmNs')} | {us('timelineColdNs')} | "
            f"{ratio('timelineSpeedupWarm')} | {ratio('exportLoopSpeedup')} |")
    lines += ["", "| memory scenario | mode | frames | rss before KiB | rss after KiB | highest KiB | peak MiB |",
              "|---|---|---|---|---|---|---|"]
    for row in result["memory"]:
        lines.append(f"| {row['scenario']} | {row['mode']} | {row['frames']} | {row['rssKiBBefore']} | "
                     f"{row['rssKiBAfter']} | {row['rssKiBHighest']} | {row['peakRssMiB']:.1f} |")
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--tree", type=Path, default=REPO, help="tree to measure (default: this checkout)")
    parser.add_argument("--base", type=Path, help="pristine checkout of main to measure as the baseline")
    parser.add_argument("--repeats", type=int, default=5, help="process repetitions per scenario (median reported)")
    parser.add_argument("--reps", type=int, default=7, help="in-process repetitions per metric")
    parser.add_argument("--frames", type=int, default=480, help="frames sampled across each edit")
    parser.add_argument("--long", type=int, default=20000, help="frames of the memory run")
    parser.add_argument("--scenarios", type=lambda v: v.split(","), default=SCENARIOS)
    parser.add_argument("--memory-scenarios", type=lambda v: [name for name in v.split(",") if name], default=["medium", "camera-heavy"])
    parser.add_argument("--cargo", type=shlex.split, default=default_cargo())
    parser.add_argument("--out", type=Path, help="write the full result as JSON")
    args = parser.parse_args()

    report = {"args": {k: str(v) for k, v in vars(args).items()}, "runs": {}}
    if args.base:
        print(f"baseline: {args.base}", flush=True)
        overlay_base(args.base)
        report["runs"]["baseline"] = {
            "provenance": provenance(args.base, "baseline (pristine main)"),
            **measure_tree(Tree(args.base, args.cargo), args, with_timeline=False)}
    print(f"tree: {args.tree}", flush=True)
    report["runs"]["after"] = {
        "provenance": provenance(args.tree, "after (PlanTimeline)"),
        **measure_tree(Tree(args.tree, args.cargo), args, with_timeline=True)}
    if args.out:
        args.out.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    for key, run in report["runs"].items():
        print("\n" + render(run["provenance"]["label"], run))


if __name__ == "__main__":
    main()
