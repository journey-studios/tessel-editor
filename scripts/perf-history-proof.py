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
    working directory nor the project's folder, and whose errors are typed. The file is written
    off the window's thread: the command answers `state: writing` and the end (`done`, or the
    typed refusal of the disk) is read from `get perf.history` -> `export`;
  * clear forgets the history; stop keeps it readable;
  * the cost: playback of the same span with the history off, recording with the panel closed
    ("hidden") and recording with the panel open ("visible"), alternating the order each round,
    comparing what the app already counts (pictures accepted, skipped frames, largest gap, UI
    repaint requests) and the editor process's CPU time;
  * with --buffer-minutes N (a positive, finite number): one continuous playback of N minutes while
    recording; the ring stays 240 buckets, the events stay within their capacity, playback is
    still running at the end, and the editor's RSS growth after warm-up is judged only when at
    least `MIN_BUFFER_SAMPLES` readings exist (an unknown reading is unknown, never zero).

Every report names the commit and whether the binary is older than the sources (`source`): a
number from a binary that is not the head under review is not evidence about that head, and a
stale binary fails a check unless `--allow-stale-binary` says the run is about that binary.

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


MIN_BUFFER_SAMPLES = 6


def parse_buffer_minutes(text, max_seconds=600):
    """`--buffer-minutes`: a finite number of minutes (0 means off) that the fixture can cover
    (`max_seconds` is the fixture helper's limit, 10 minutes). Anything else is refused up front,
    before an editor is started, with a message that names this option, not `--seconds`."""
    try:
        minutes = float(text)
    except (TypeError, ValueError):
        raise ValueError(f"--buffer-minutes needs a number, got {text!r}") from None
    if not math.isfinite(minutes) or minutes < 0:
        raise ValueError(f"--buffer-minutes must be a finite number >= 0, got {text!r}")
    if minutes * 60.0 > max_seconds:
        raise ValueError(f"--buffer-minutes must be at most {max_seconds / 60:g} (the fixture covers {max_seconds} s), got {text!r}")
    return minutes


def buffer_sample_every(minutes):
    """Seconds between readings: about `MIN_BUFFER_SAMPLES` per run, but never slower than 20 s
    and never a busy loop; a short run is not stretched to a whole 20 s step."""
    return min(20.0, max(1.0, minutes * 60.0 / MIN_BUFFER_SAMPLES))


def buffer_wait(every, deadline, now):
    """Bound the next wait by the remaining run time, including earlier sampling overhead."""
    return min(every, max(0.0, deadline - now))


def buffer_growth(samples):
    """RSS growth over the run and after the first third (warm-up), in MiB. A reading that is
    missing is missing (`None`), not zero, and too few readings judge nothing: each result is
    `None` rather than a growth the data cannot support."""
    readings = [sample.get("rss_mib") for sample in samples]
    if len(readings) < MIN_BUFFER_SAMPLES or any(not isinstance(value, (int, float)) for value in readings):
        return {"growth_mib": None, "growth_after_warmup_mib": None, "judged": False}
    warm = readings[len(readings) // 3:]
    return {
        "growth_mib": round(readings[-1] - readings[0], 1),
        "growth_after_warmup_mib": round(warm[-1] - warm[0], 1),
        "judged": True,
    }


def cpu_rate(cpu_before, cpu_after, wall_seconds):
    """CPU seconds per second of **measured** wall time between the two readings. `None` when a
    reading is unknown, the wall time is not positive or the counter went backwards (a restarted
    process): an unknown rate is never a zero or a guess."""
    if cpu_before is None or cpu_after is None or wall_seconds is None:
        return None
    if wall_seconds <= 0 or cpu_after < cpu_before:
        return None
    return round((cpu_after - cpu_before) / wall_seconds, 3)


def binary_is_stale(binary_mtime, newest_source_mtime):
    """Whether the binary predates the newest source it is supposed to prove (`None`: unknown)."""
    if binary_mtime is None or newest_source_mtime is None:
        return None
    return binary_mtime < newest_source_mtime


def source_state(binary):
    """Which commit this run speaks for, whether the tree has uncommitted changes and whether the
    binary is older than the sources: what a reader needs to know before trusting a number."""
    def git(*args):
        done = subprocess.run(["git", "-C", str(ROOT), *args], capture_output=True, text=True)
        return done.stdout.strip() if done.returncode == 0 else None

    head = git("rev-parse", "HEAD")
    dirty = git("status", "--porcelain", "--", "crates", "Cargo.toml", "Cargo.lock")
    listed = git("ls-files", "crates", "Cargo.toml", "Cargo.lock")
    newest = None
    for name in (listed or "").splitlines():
        try:
            mtime = (ROOT / name).stat().st_mtime
        except OSError:
            continue
        newest = mtime if newest is None else max(newest, mtime)
    try:
        built = os.stat(binary).st_mtime
    except OSError:
        built = None
    return {
        "git_head": head,
        "uncommitted_sources": None if dirty is None else bool(dirty),
        "binary_older_than_sources": binary_is_stale(built, newest),
    }


def export_outcome(response, state):
    """How an export ended, from the command's answer and `get perf.history` -> `export`:
    `("refused", code)` when the command itself refused (nothing needed the disk), `("done", None)`,
    `("failed", code)` for the typed end of the background write, or `("pending", None)` while it
    still writes. A response with neither error nor a known state is `("unknown", None)`."""
    error = (response or {}).get("error")
    if error:
        return ("refused", error.get("code"))
    state = state or {}
    if state.get("state") == "done":
        return ("done", None)
    if state.get("state") == "failed":
        return ("failed", (state.get("error") or {}).get("code"))
    if state.get("state") == "writing":
        return ("pending", None)
    return ("unknown", None)


def export_code(outcome):
    """The typed error code of an outcome that did not produce a file, else `None`."""
    kind, code = outcome
    return code if kind in ("refused", "failed") else None


def settle_export(editor, response, timeout=15.0):
    """Waits for the background write the command started (a refusal needs no wait) and returns
    its outcome. A write that never ends within `timeout` is `("pending", None)`: a failure the
    caller's check reports, not an assumption that it worked."""
    deadline = time.monotonic() + timeout
    while True:
        outcome = export_outcome(response, history(editor).get("export"))
        if outcome[0] != "pending" or time.monotonic() >= deadline:
            return outcome
        time.sleep(0.1)


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
    try:
        buffer_minutes = parse_buffer_minutes(arg("--buffer-minutes", "0"), pmp.MAX_SECONDS)
    except ValueError as error:
        print(error)
        return 2
    if not os.path.exists(binary):
        print("usage: perf-history-proof.py [--binary <editor>] [--shots DIR] [--seconds N] [--rounds N] [--buffer-minutes N] [--allow-stale-binary]")
        return 2
    try:
        frames = pmp.fixture_frames(max(seconds, buffer_minutes * 60.0))
    except ValueError as error:
        print(error)
        return 2
    report.clear()
    report.update({"checks": [], "cost": [], "feature": "F049 history (BL145.2)", "binary": binary, "source": source_state(binary)})
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
        stale = report["source"]["binary_older_than_sources"]
        check(
            "the binary is not older than the sources it speaks for (or the run says it is about that binary)",
            stale is False or "--allow-stale-binary" in sys.argv,
            report["source"],
        )

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
        view = state.get("view")
        check("the recording produced a view to read events from", isinstance(view, dict), state)
        events = [event["kind"] for event in ((view or {}).get("events") or {}).get("newest", [])]
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
        ended = settle_export(editor, exported)
        check("export_to answers that the write started, off the window's thread", exported.get("ok") and (exported.get("data") or {}).get("state") == "writing", exported)
        check("export_to ends as a local file", ended[0] == "done" and os.path.isfile(target), [ended, exported])
        if os.path.isfile(target):
            text = Path(target).read_text()
            doc = json.loads(text)
            folder = os.path.dirname(document)
            check("the export is schema tessel.perf-history v1", doc.get("schema") == "tessel.perf-history" and doc.get("schema_version") == 1, [doc.get("schema"), doc.get("schema_version")])
            check("the export never carries a local path", work not in text and folder not in text and home not in text, "a path leaked")
            check("the export keeps absent as null and carries its limits", any(v is None for v in doc["series"]["texture_bytes"]["count"]) and len(doc["limits"]) >= 5, doc["limits"][:1])
            report["export_bytes"] = len(text)
        bad = editor.set(NODE, "export_to", work)
        code = export_code(settle_export(editor, bad))
        check("exporting to a directory is a typed error", code == "perf_history_export_is_directory", [code, bad])
        bad = editor.set(NODE, "export_to", os.path.join(work, "no", "such", "x.json"))
        code = export_code(settle_export(editor, bad))
        check("exporting under a missing folder is a typed error", code == "perf_history_export_parent_missing", [code, bad])

        again = editor.set(NODE, "export_to", target)
        code = export_code(settle_export(editor, again))
        check("exporting over an existing file is refused and the file is untouched", code == "perf_history_export_exists" and Path(target).read_text() == text, [code, again])
        bad = editor.set(NODE, "export_to", "relative-name.json")
        check("a relative destination is a typed error", export_code(settle_export(editor, bad)) == "perf_history_export_relative", bad)

        # The panel's own destination field: `set export_destination` is typing/picking a path,
        # `invoke export` is the button. Nothing is written to a place nobody chose.
        check("no destination until one is given", history(editor).get("export_destination") == "", history(editor).get("export_destination"))
        empty = editor.invoke(NODE, "export")
        check("the export button without a destination is a typed error", export_code(settle_export(editor, empty)) == "perf_history_export_path_missing", empty)
        chosen = os.path.join(work, "from-the-field.json")
        typed = editor.set(NODE, "export_destination", chosen)
        check("typing a destination writes nothing by itself", typed.get("ok") and not os.path.exists(chosen) and history(editor).get("export_destination") == chosen, typed)
        pmp.settle(editor, 0.8)
        report["shot_destination"] = editor.shot(shots, "history-export-destination")
        pressed = editor.invoke(NODE, "export")
        ended = settle_export(editor, pressed)
        check("the export button writes to the destination in the field", ended[0] == "done" and os.path.isfile(chosen), [ended, pressed])
        pressed_again = editor.invoke(NODE, "export")
        code = export_code(settle_export(editor, pressed_again))
        check("the same destination again is refused, never replaced", code == "perf_history_export_exists", [code, pressed_again])
        editor.set(NODE, "export_destination", "")

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
                wall_before = time.monotonic()
                run = pmp.playback_run(editor, seconds, False)
                cpu_after = cpu_seconds(editor.process.pid)
                wall = time.monotonic() - wall_before
                run.update({"arm": arm, "round": index, "wall_s": round(wall, 2)})
                # Divided by the wall time actually elapsed between the two readings, not by a
                # guessed constant on top of `--seconds`; unknown stays absent.
                rate = cpu_rate(cpu_before, cpu_after, wall)
                if rate is not None:
                    run["cpu_s_per_s"] = rate
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
            started_at = time.monotonic()
            samples = []
            every = buffer_sample_every(buffer_minutes)
            deadline = started_at + buffer_minutes * 60
            while time.monotonic() < deadline:
                time.sleep(buffer_wait(every, deadline, time.monotonic()))
                state = history(editor)
                samples.append({
                    "t_s": round(time.monotonic() - started_at),
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
            check("playback was still running at the end (the buffer was measured under load)", (final["view"].get("state") or {}).get("expected_motion") is True, final["view"].get("state"))
            growth = buffer_growth(samples)
            report["buffer_rss"] = growth
            check(
                f"enough RSS readings to judge growth ({MIN_BUFFER_SAMPLES} known ones needed; use a longer --buffer-minutes)",
                growth["judged"],
                [sample.get("rss_mib") for sample in samples],
            )
            if growth["judged"]:
                check("the editor's RSS does not keep growing with the recording (after warm-up)", growth["growth_after_warmup_mib"] < 64, growth)
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
