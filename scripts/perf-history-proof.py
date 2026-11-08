#!/usr/bin/env python3
"""Proof of the rolling performance history (F049 / BL145.2) in the *real window*.

    python3 scripts/directed-scene-proof/with-gpu-lock.py python3 scripts/perf-history-proof.py \
        [--binary target/release/tessel-editor] [--shots DIR] [--seconds 8] [--rounds 3] \
        [--buffer-minutes 0] [--keep]

Starts the DIRECT binary with an isolated HOME (own socket; the user's own editor is never
addressed), builds a synthetic H.264 project with FFmpeg (so the monitor really presents
pictures) and drives everything through semantic requests (`get`, `set`, `invoke`, `op`); no
pointer, no coordinates. It reuses the helpers of `perf-meter-proof.py` (editor process, fixture,
playback run, cleanup). It checks, one by one:

  * off by default: no recording, no data, `perf.history` carries no view;
  * recording while the panel is closed: UI frames, monitor pictures, `render_ms`, request
    latency, RSS and the texture estimate are all observed, and `visible` stays false (recording
    is not visibility);
  * the panel (`set perf.history visible true`) hides the native view like any dialog
    (`monitor.viewport` says `on_screen:false`), keeps recording, and closing it neither stops nor
    starts anything;
  * freeze keeps one immutable snapshot while the recording continues underneath, a selection
    outside the window is a typed error, one inside returns the inspection (numbers and correlated
    events), resume goes back to the moving window;
  * the export is a local JSON of `tessel.perf-history` v1 whose text contains neither the
    working directory nor the project's folder, and whose errors are typed;
  * clear forgets the history; stop keeps it readable;
  * the cost: playback of the same span with the history off, recording with the panel closed
    ("hidden") and recording with the panel open ("visible"), alternating the order each round,
    comparing what the app already counts (pictures accepted, skipped frames, largest gap, UI
    repaint requests) and the editor process's CPU time;
  * with --buffer-minutes N: one continuous playback of N minutes while recording; the ring stays
    240 buckets, the events stay within their capacity and the editor's RSS growth is reported.

With --shots DIR it captures the window of THIS process with `screencapture -l` at the live,
frozen and inspected states. Prints one JSON report on the last line, whatever happens (`ok`,
`failures`); exit status 1 on a failed check or an exception.
"""
import importlib.util
import json
import math
import os
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
NODE = "perf.history"
spec = importlib.util.spec_from_file_location("perf_meter_proof", HERE / "perf-meter-proof.py")
pmp = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pmp)
check = pmp.check
report = pmp.report

_wait_job = pmp.wait_job


def _wait_job_applied(editor, job, timeout=120.0, **kw):
    """`succeeded` can be read a moment before the project commits the result (`result_state`
    still `pending`), which a loaded machine makes visible; wait for the commit as well."""
    deadline = time.monotonic() + timeout
    while True:
        info = _wait_job(editor, job, timeout=timeout, **kw)
        if info.get("state") != "succeeded" or info.get("result_state") != "pending" or time.monotonic() >= deadline:
            return info
        time.sleep(0.25)


pmp.wait_job = _wait_job_applied


def arg(name, default=None):
    return sys.argv[sys.argv.index(name) + 1] if name in sys.argv else default


def history(editor):
    return editor.get(NODE)["data"]["state"]


def peaks(state):
    return ((state.get("view") or {}).get("peaks")) or {}


def viewport_on_screen(editor):
    return editor.viewport().get("on_screen")


def cpu_seconds(pid):
    """Cumulative CPU time of one process (`ps -o time=`: `[[HH:]MM:]SS.cc`), or None."""
    out = subprocess.run(["ps", "-o", "time=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
    return parse_cpu_time(out)


def parse_cpu_time(text):
    if not re.fullmatch(r"(\d+-)?(\d+:){0,2}\d+(\.\d+)?", text or ""):
        return None
    days = 0
    if "-" in text:
        day, text = text.split("-")
        days = int(day)
    seconds = 0.0
    for part in text.split(":"):
        seconds = seconds * 60 + float(part)
    return days * 86400 + seconds


def rss_mib(pid):
    out = subprocess.run(["ps", "-o", "rss=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
    return round(int(out) / 1024, 1) if out.isdigit() else None


def arm_order(round_index):
    """Alternates so a drifting machine hits every arm in every position."""
    arms = ["off", "hidden", "visible"]
    return arms if round_index % 2 == 0 else list(reversed(arms))


def arm_setup(editor, arm):
    """Puts the editor in one arm: nothing, recording with the panel closed, or recording with it open."""
    editor.set(NODE, "visible", False)
    editor.set(NODE, "recording", False)
    editor.invoke(NODE, "clear")
    if arm in ("hidden", "visible"):
        editor.set(NODE, "recording", True)
    if arm == "visible":
        editor.set(NODE, "visible", True)
    pmp.settle(editor, 1.2)


def mean(values):
    values = [v for v in values if isinstance(v, (int, float))]
    return round(sum(values) / len(values), 3) if values else None


def summarize(runs):
    out = {}
    for arm in ("off", "hidden", "visible"):
        mine = [run for run in runs if run["arm"] == arm and run.get("ok")]
        out[arm] = {
            "runs": len(mine),
            **{key: mean([run.get(key) for run in mine]) for key in ("ticks", "skipped_frames", "max_gap_frames", "pictures_accepted", "ui_repaints", "cpu_s_per_s")},
        }
    return out


def main():
    binary = os.path.abspath(arg("--binary", str(ROOT / "target/release/tessel-editor")))
    shots = arg("--shots")
    seconds = float(arg("--seconds", "8"))
    rounds = int(arg("--rounds", "3"))
    buffer_minutes = float(arg("--buffer-minutes", "0"))
    if not os.path.exists(binary):
        print("usage: perf-history-proof.py [--binary <editor>] [--shots DIR] [--seconds N] [--rounds N] [--buffer-minutes N]")
        return 2
    try:
        frames = pmp.fixture_frames(max(seconds, buffer_minutes * 60.0))
    except ValueError as error:
        print(error)
        return 2
    report.clear()
    report.update({"checks": [], "cost": [], "feature": "F049 history (BL145.2)", "binary": binary})
    pmp.failures.clear()
    work = tempfile.mkdtemp(prefix="php-", dir="/tmp")
    home = os.path.join(work, "home")
    os.makedirs(home)
    editor = None
    interrupted = None
    try:
        editor = pmp.Editor(binary, home)
        editor.wait_ready()
        pmp.settle(editor)
        report["load_average"] = os.getloadavg()

        # --- off by default
        state = history(editor)
        check("off by default: not recording, no data, no view", state["recording"] is False and state["has_data"] is False and "view" not in state, state)
        check("off by default: the panel is closed", state["visible"] is False, state)

        ok, why, document = pmp.synthetic_project(editor, work, frames)
        check("a project whose pictures the runtime draws", ok, why)
        if not ok:
            raise RuntimeError(f"no fixture: {why}")
        frame = pmp.frame_when_prepared(editor)
        check("a real frame renders from the fixture", frame.get("ok"), str(frame)[:300])
        report["fixture_frames"] = frames
        pmp.settle(editor, 1.5)
        check("still off after the project loaded: nothing was recorded on the side", history(editor)["has_data"] is False)

        # --- recording with the panel closed
        started = editor.set(NODE, "recording", True)
        check("set recording true starts it", started.get("ok") and history(editor)["recording"] is True, started)
        check("recording does not open the panel", history(editor)["visible"] is False)
        check("the monitor stays on screen while only recording", viewport_on_screen(editor) is True, editor.viewport())
        editor.op({"op": "seek", "frame": 0})
        time.sleep(0.5)
        editor.op({"op": "playback.start", "from": 0})
        time.sleep(seconds)
        state = history(editor)
        seen = peaks(state)
        for metric in ("ui_frames", "monitor_frames", "render_ms", "latency_ms", "rss_bytes"):
            check(f"recorded while hidden: {metric} has samples", bool(seen.get(metric)) and seen[metric]["count"] > 0, seen.get(metric))
        report["peaks_hidden"] = seen
        latency = seen.get("latency_ms") or {}
        render = seen.get("render_ms") or {}
        check("latency (admission to acceptance) is kept apart from render_ms (CPU)", latency.get("max") is not None and render.get("max") is not None and latency["max"] >= render["min"], [latency, render])
        events = [event["kind"] for event in (state["view"]["events"]["newest"])]
        check("playback and recording are events of the timeline", "recording_started" in events and "playback_started" in events, events)

        # --- the panel
        shown = editor.set(NODE, "visible", True)
        check("set visible true opens the panel", shown.get("ok") and history(editor)["visible"] is True, shown)
        pmp.settle(editor, 1.5)
        check("the open panel hides the native view like any dialog", viewport_on_screen(editor) is False, editor.viewport())
        check("opening the panel keeps recording", history(editor)["recording"] is True)
        report["shot_live"] = editor.shot(shots, "history-live")

        # --- freeze, inspect, resume
        frozen = editor.invoke(NODE, "freeze")
        check("freeze fixes one snapshot", frozen.get("ok") and history(editor)["frozen"] is True, frozen)
        window = history(editor)["view"]["window"]
        count_then = peaks(history(editor))["ui_frames"]["count"]
        time.sleep(1.5)
        check("the frozen view does not move while the recording continues", peaks(history(editor))["ui_frames"]["count"] == count_then and history(editor)["recording"] is True)
        again = editor.invoke(NODE, "freeze")
        check("freezing twice keeps the first snapshot and says so", again.get("ok") and again["data"]["changed"] is False, again)
        outside = editor.set(NODE, "selected_ms", window["end_ms"] + 5000)
        check("a selection outside the window is a typed error", (outside.get("error") or {}).get("code") == "perf_history_outside_window", outside)
        inside = editor.set(NODE, "selected_ms", window["end_ms"] - 4000)
        inspection = (inside.get("data") or {}).get("inspection") or {}
        check("a selection inside the window returns numbers and events", inside.get("ok") and "metrics" in inspection and "events" in inspection, str(inside)[:300])
        report["inspection"] = inspection
        report["shot_inspect"] = editor.shot(shots, "history-frozen-inspect")
        # The same panel in the light theme: bars, lane and markers come from the Kit's tokens.
        editor.invoke("theme.light")
        pmp.settle(editor, 1.2)
        report["shot_light"] = editor.shot(shots, "history-frozen-inspect-light")
        editor.invoke("theme.dark")
        pmp.settle(editor, 0.8)
        editor.invoke(NODE, "clear_selection")
        check("clear_selection clears it", history(editor)["selected_ms"] is None)
        resumed = editor.invoke(NODE, "resume")
        check("resume goes back to the moving window", resumed.get("ok") and history(editor)["frozen"] is False, resumed)

        # --- export
        target = os.path.join(work, "exported-history.json")
        exported = editor.set(NODE, "export_to", target)
        check("export_to writes a local file", exported.get("ok") and os.path.isfile(target), exported)
        if os.path.isfile(target):
            text = Path(target).read_text()
            doc = json.loads(text)
            folder = os.path.dirname(document)
            check("the export is schema tessel.perf-history v1", doc.get("schema") == "tessel.perf-history" and doc.get("schema_version") == 1, [doc.get("schema"), doc.get("schema_version")])
            check("the export never carries a local path", work not in text and folder not in text and home not in text, "a path leaked")
            check("the export keeps absent as null and carries its limits", any(v is None for v in doc["series"]["texture_bytes"]["count"]) and len(doc["limits"]) >= 5, doc["limits"][:1])
            report["export_bytes"] = len(text)
        bad = editor.set(NODE, "export_to", work)
        check("exporting to a directory is a typed error", (bad.get("error") or {}).get("code") == "perf_history_export_is_directory", bad)
        bad = editor.set(NODE, "export_to", os.path.join(work, "no", "such", "x.json"))
        check("exporting under a missing folder is a typed error", (bad.get("error") or {}).get("code") == "perf_history_export_parent_missing", bad)

        # --- closing the panel: not stopping
        hidden = editor.set(NODE, "visible", False)
        pmp.settle(editor, 1.2)
        check("closing the panel brings the monitor back", hidden.get("ok") and viewport_on_screen(editor) is True, editor.viewport())
        check("closing the panel does not stop the recording", history(editor)["recording"] is True)
        editor.op({"op": "playback.stop"})
        editor.set(NODE, "recording", False)
        state = history(editor)
        check("stopping keeps the data readable", state["recording"] is False and state["has_data"] is True and bool(peaks(state)), state.get("has_data"))
        editor.invoke(NODE, "clear")
        check("clear forgets the history", history(editor)["has_data"] is False)

        # --- the cost
        for index in range(rounds):
            for arm in arm_order(index):
                arm_setup(editor, arm)
                cpu_before = cpu_seconds(editor.process.pid)
                run = pmp.playback_run(editor, seconds, False)
                cpu_after = cpu_seconds(editor.process.pid)
                run.update({"arm": arm, "round": index})
                if cpu_before is not None and cpu_after is not None:
                    run["cpu_s_per_s"] = round((cpu_after - cpu_before) / (seconds + 2.5), 3)
                report["cost"].append(run)
                print(json.dumps(run), flush=True)
                check(f"cost round {index} arm {arm}: playback covered the whole {seconds:g} s window", run.get("ok"), run.get("error"))
        arm_setup(editor, "off")
        report["cost_mean"] = summarize(report["cost"])
        print(json.dumps(report["cost_mean"]), flush=True)

        # --- the buffer over minutes
        if buffer_minutes > 0:
            arm_setup(editor, "hidden")
            editor.op({"op": "seek", "frame": 0})
            time.sleep(1.0)
            editor.op({"op": "playback.start", "from": 0})
            started_at = time.time()
            samples = []
            while time.time() - started_at < buffer_minutes * 60:
                time.sleep(20)
                state = history(editor)
                samples.append({
                    "t_s": round(time.time() - started_at),
                    "rss_mib": rss_mib(editor.process.pid),
                    "buckets": state["view"]["window"]["buckets"],
                    "events_held": state["view"]["events"]["held"],
                    "events_dropped": state["view"]["events"]["dropped"],
                })
            report["buffer_samples"] = samples
            final = history(editor)
            check("the ring stays 240 buckets after minutes", final["view"]["window"]["buckets"] == 240 and final["view"]["window"]["duration_ms"] == 60000, final["view"]["window"])
            check("the events stay within their capacity", final["view"]["events"]["held"] <= final["view"]["events"]["capacity"], final["view"]["events"])
            check("the window really is 60 s: the oldest bucket is not older than the window", final["view"]["window"]["end_ms"] - final["view"]["window"]["start_ms"] <= 60250, final["view"]["window"])
            growth = (samples[-1]["rss_mib"] or 0) - (samples[0]["rss_mib"] or 0) if samples else None
            report["buffer_rss_growth_mib"] = growth
            warmup = samples[len(samples) // 3:] if len(samples) > 3 else samples
            late_growth = (warmup[-1]["rss_mib"] or 0) - (warmup[0]["rss_mib"] or 0) if warmup else None
            report["buffer_rss_growth_after_warmup_mib"] = late_growth
            check("the editor's RSS does not keep growing with the recording (after warm-up)", late_growth is not None and late_growth < 64, [growth, late_growth])
            editor.op({"op": "playback.stop"})
            arm_setup(editor, "off")
    except BaseException as error:  # the report is still printed, then re-raised
        pmp.record_abort(error)
        if not isinstance(error, Exception):
            interrupted = error
    finally:
        report.update(pmp.conclude(editor, work, "--keep" in sys.argv))
        print(json.dumps(report))
    if interrupted is not None:
        raise interrupted
    return 1 if pmp.failures else 0


if __name__ == "__main__":
    sys.exit(main())
