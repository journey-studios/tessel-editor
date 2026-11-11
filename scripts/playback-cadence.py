#!/usr/bin/env python3
"""BL157.6: how the editor's playback clock hands frames to the monitor, read from the host trace of recorded sessions.

    python3 scripts/playback-cadence.py PATH [PATH ...] [--group auto|all] [--json FILE] [--markdown FILE]

PATH is a session record (`run-NN-....json`, as `runtime-bundle-ab.py`, `real-latency-ab.py` and `first-use-ab.py` write them) or a folder of them. The
sessions need the host trace (`TESSEL_VIEWPORT_TRACE=1`): its `tick` and `play_clock_started` events are the input. Nothing is started, nothing is measured
again: this only reads what a run already recorded.

WHY. The playback loop (`crates/desktop/src/workstation/playback.rs`) asks the audio clock "which frame is it?" a fixed number of times per frame
(`TICKS_PER_FRAME`, 2; with `TESSEL_PLAYBACK_POLL=deadline` the question is also armed for the next frame boundary) and moves the playhead when the
answer changed. A `tick` event says how long that poll really was after the previous one and which
frame it saw. If every frame change is seen by the first poll after it, a frame takes either floor(x) or ceil(x) polls, with x = frame period / poll period:
at 24 fps and a 22.2 ms poll x = 1.875, so 87.5% of the frame changes take 2 polls (44 ms) and 12.5% take 1 (22 ms), a fixed pattern of seven or eight 44 ms
steps and one 22 ms step. This reads the trace and prints the shares it really had next to that prediction, plus what the monitor delivered (the gaps
between consecutive pictures) and the frames the clock jumped over. A cadence is not a pause: this says nothing about the long waits BL157 is about, and
it accepts nothing; it is the instrument for deciding whether the poll should be scheduled differently.

CHANGE LAG. A host that traces the clock it read (`tick.clock`, fractional frames; hosts of BL157.6's switch and later) lets this say how long after its
boundary each frame change was seen: the fraction of a frame the clock had already run, over the frame rate. The fixed schedule sees a change anywhere
between 0 and one poll after it (about 11 ms on average at 24 fps), the deadline one just after the boundary (the guard plus the timer's lateness).
`play_clock_started.poll` names the schedule of each play (`unknown` for a trace from before the switch).

Exit: 0 read; 2 no session with a play was found.
"""
import argparse
import collections
import json
import math
import pathlib
import sys

NEARLY_WHOLE = 1e-9
SHORT_DELIVERY_MS = 30.0  # a picture that followed the previous one by less than this came in the catch-up step of the pattern
SLOW_DELIVERY_MS = 62.5  # the harness's own diagnostic mark of a slow gap (one and a half frames at 24 fps)
VERY_SLOW_DELIVERY_MS = 100.0  # the harness's second mark


def read_sessions(paths):
    """(file name, record) of every session record in `paths`: a file as it is, a folder by its `run-*.json`."""
    files = []
    for raw in paths:
        path = pathlib.Path(raw)
        files += sorted(path.glob("run-*.json")) if path.is_dir() else [path]
    return [(file.name, json.loads(file.read_text())) for file in files]


def group_of(record, mode):
    """The A/B arm of a session, else its mode, else everything together."""
    if mode == "all":
        return "all"
    return record.get("arm") or record.get("mode") or "all"


def play_windows(events):
    """Each play: from one `play_clock_started` to the next (or the end of the trace), host time, with the frame rate and the poll schedule that
    event named (`None` when it did not: a trace from before the switch)."""
    starts = [event for event in events if event.get("kind") == "play_clock_started"]
    return [{"start": event["t_ms"], "end": starts[index + 1]["t_ms"] if index + 1 < len(starts) else math.inf,
             "fps": (event.get("fields") or {}).get("fps"), "poll": (event.get("fields") or {}).get("poll")}
            for index, event in enumerate(starts)]


def windows(events):
    """(start, end) host time of each play."""
    return [(window["start"], window["end"]) for window in play_windows(events)]


def frames_per_second(events):
    """The frame rate the clock reported, when every play of the group reported the same."""
    rates = {(event.get("fields") or {}).get("fps") for event in events if event.get("kind") == "play_clock_started"}
    rates.discard(None)
    return rates


def ticks_of(events, window):
    """The `tick` fields inside a play that saw a frame (the first polls, while the device opens, have none)."""
    start, end = window
    found = []
    for event in events:
        fields = event.get("fields") or {}
        if event.get("kind") == "tick" and start <= event["t_ms"] < end and fields.get("frame_before") is not None and fields.get("frame_after") is not None:
            found.append(fields)
    return found


def polls_per_change(ticks):
    """How many polls passed between one frame change and the next (the first change has no earlier one to count from)."""
    counts, since, seen = collections.Counter(), 0, False
    for tick in ticks:
        since += 1
        if tick["frame_after"] != tick["frame_before"]:
            if seen:
                counts[since] += 1
            seen, since = True, 0
    return counts


def poll_gaps(ticks):
    """Milliseconds between one poll and the next, on the host clock."""
    ran = [tick["ran_ms"] for tick in ticks if tick.get("ran_ms") is not None]
    return [after - before for before, after in zip(ran, ran[1:])]


def skipped_frames(ticks):
    """Frames the clock jumped over between two polls (a late poll)."""
    return sum(max(0, tick["frame_after"] - tick["frame_before"] - 1) for tick in ticks)


def change_lags(ticks, fps):
    """Milliseconds after its boundary at which each frame change was seen: the fraction of a frame the clock had already run when the poll read it.
    A poll that jumped several frames counts the age of the frame it reached. A tick that did not trace the clock (an older host), or a rate that
    is not usable, gives nothing: nothing is guessed."""
    if not isinstance(fps, (int, float)) or isinstance(fps, bool) or not math.isfinite(fps) or fps <= 0:
        return []
    lags = []
    for tick in ticks:
        clock = tick.get("clock")
        if tick["frame_after"] == tick["frame_before"] or not isinstance(clock, (int, float)) or isinstance(clock, bool) or not math.isfinite(clock):
            continue
        lags.append((clock - math.floor(clock)) / fps * 1000.0)
    return lags


def delivered_gaps(frames, window):
    """Milliseconds between consecutive pictures the monitor finished inside a play."""
    start, end = window
    done = sorted(frame["done_ms"] for frame in frames if frame.get("done_ms") is not None and start <= frame["done_ms"] < end)
    return [after - before for before, after in zip(done, done[1:])]


def predicted_polls(frame_ms, poll_ms):
    """Polls per frame change if each change is seen at the first poll after it: floor(x) or ceil(x), x = frame period / poll period."""
    steps = round(frame_ms / poll_ms, 3)  # the times are measured to a microsecond: a fraction under a thousandth is that, not a pattern
    low = math.floor(steps)
    share = steps - low
    if share < NEARLY_WHOLE:
        return {low: 1.0}
    return {low: round(1 - share, 4), low + 1: round(share, 4)}


def stats(values):
    """n / mean / p50 / p95 / p99 / max, nearest-rank percentiles (no interpolation invents a value)."""
    values = sorted(value for value in values if value is not None)
    if not values:
        return {"n": 0, "mean": None, "p50": None, "p95": None, "p99": None, "max": None}

    def at(share):
        return round(values[min(len(values) - 1, max(0, math.ceil(share * len(values)) - 1))], 3)

    return {"n": len(values), "mean": round(sum(values) / len(values), 3), "p50": at(0.5), "p95": at(0.95), "p99": at(0.99), "max": round(values[-1], 3)}


def share_of(values, test):
    return round(sum(1 for value in values if test(value)) / len(values), 4) if values else None


def raw_plays(sessions):
    """The raw numbers of every play that saw a frame, as lists (not summaries): a group pools them, `each_play` summarises them one by one."""
    for name, record in sessions:
        events, frames = record.get("events") or [], record.get("frames") or []
        for window in play_windows(events):
            span = (window["start"], window["end"])
            ticks = ticks_of(events, span)
            if not ticks:
                continue
            yield {"session": name, "poll": window["poll"], "fps": window["fps"], "ticks": len(ticks), "spacing": polls_per_change(ticks),
                   "poll_gaps": poll_gaps(ticks), "late": [tick.get("late_ms") for tick in ticks], "skipped": skipped_frames(ticks),
                   "delivered": delivered_gaps(frames, span), "lags": change_lags(ticks, window["fps"])}


def mean_polls(spacing):
    """Polls per frame change, from the counts of `polls_per_change`."""
    changes = sum(spacing.values())
    return round(sum(count * seen for count, seen in spacing.items()) / changes, 3) if changes else None


def summarize(sessions):
    """The cadence of a group of sessions: what the polls did, what the monitor was handed, and the model's prediction."""
    spacing, polls, late, delivered, lags = collections.Counter(), [], [], [], []
    skipped, rates, modes = 0, set(), set()
    plays = list(raw_plays(sessions))
    for _, record in sessions:
        rates |= frames_per_second(record.get("events") or [])
    for play in plays:
        spacing.update(play["spacing"])
        polls += play["poll_gaps"]
        late += play["late"]
        delivered += play["delivered"]
        lags += play["lags"]
        skipped += play["skipped"]
        modes.add(play["poll"] or "unknown")
    changes = sum(spacing.values())
    poll = stats(polls)
    fps = next(iter(rates)) if len(rates) == 1 else None
    model = None
    if fps and poll["mean"]:
        frame_ms = 1000.0 / fps
        model = {"frame_ms": round(frame_ms, 3), "poll_ms": poll["mean"], "polls_per_frame": round(frame_ms / poll["mean"], 3),
                 "predicted_share": {str(count): share for count, share in predicted_polls(frame_ms, poll["mean"]).items()}}
    return {"sessions": len(sessions), "plays": len(plays), "fps": fps, "poll_modes": sorted(modes), "frame_changes": changes,
            "polls_per_change_share": {str(count): round(spacing[count] / changes, 4) for count in sorted(spacing)} if changes else {},
            "polls_per_frame_change": mean_polls(spacing),
            "poll_ms": poll, "late_ms": stats(late), "delivered_ms": stats(delivered), "change_lag_ms": stats(lags),
            "delivered_share_under_30_ms": share_of(delivered, lambda gap: gap < SHORT_DELIVERY_MS),
            "delivered_share_over_62_5_ms": share_of(delivered, lambda gap: gap > SLOW_DELIVERY_MS),
            "skipped_frames": skipped, "model": model}


def each_play(sessions):
    """One summary per play (in the order of the sessions and of the plays inside them): the numbers a paired comparison needs, with counts rather than
    shares so that a group's totals are their sums."""
    return [{"session": play["session"], "poll": play["poll"], "fps": play["fps"], "ticks": play["ticks"],
             "frame_changes": sum(play["spacing"].values()), "polls_per_frame_change": mean_polls(play["spacing"]),
             "delivered": len(play["delivered"]),
             "under_30_ms": sum(1 for gap in play["delivered"] if gap < SHORT_DELIVERY_MS),
             "over_62_5_ms": sum(1 for gap in play["delivered"] if gap > SLOW_DELIVERY_MS),
             "over_100_ms": sum(1 for gap in play["delivered"] if gap > VERY_SLOW_DELIVERY_MS),
             "max_delivered_ms": round(max(play["delivered"]), 3) if play["delivered"] else None,
             "change_lag_ms": stats(play["lags"]), "poll_ms": stats(play["poll_gaps"]), "skipped_frames": play["skipped"]}
            for play in raw_plays(sessions)]


def build_report(sessions, mode="auto"):
    groups = collections.OrderedDict()
    for name, record in sessions:
        groups.setdefault(group_of(record, mode), []).append((name, record))
    return {"groups": {key: summarize(members) for key, members in sorted(groups.items())}}


def shares(found):
    return ", ".join(f"{count} poll{'s' if count != '1' else ''}: {share:.1%}" for count, share in found.items()) or "no frame change"


def lag_cell(lag):
    return f"{lag['mean']} / {lag['p95']} / {lag['max']}" if lag["n"] else "n/a"


def markdown(report):
    lines = ["# Playback clock cadence (BL157.6)", "",
             "What the clock's polls and the monitor's pictures did in the recorded plays. A cadence, not a pause: nothing here is an acceptance.", "",
             "| group | sessions | plays | frame changes | polls per frame change | poll ms mean / p95 / p99 | change lag ms mean / p95 / max | delivered ms p50 / p95 / max | delivered < 30 ms | > 62.5 ms | skipped frames |",
             "|---|---|---|---|---|---|---|---|---|---|---|"]
    for name, g in report["groups"].items():
        poll, delivered = g["poll_ms"], g["delivered_ms"]
        lines.append(f"| {name} | {g['sessions']} | {g['plays']} | {g['frame_changes']} | {shares(g['polls_per_change_share'])} | "
                     f"{poll['mean']} / {poll['p95']} / {poll['p99']} | {lag_cell(g['change_lag_ms'])} | "
                     f"{delivered['p50']} / {delivered['p95']} / {delivered['max']} | "
                     f"{g['delivered_share_under_30_ms']} | {g['delivered_share_over_62_5_ms']} | {g['skipped_frames']} |")
    lines.append("")
    for name, g in report["groups"].items():
        model = g["model"]
        schedule = f"poll {', '.join(g['poll_modes'])}"
        if model:
            lines.append(f"- {name} ({schedule}): a {model['frame_ms']} ms frame over a {model['poll_ms']} ms poll is {model['polls_per_frame']} polls per frame, so the model "
                         f"predicts {shares(model['predicted_share'])}")
        else:
            lines.append(f"- {name} ({schedule}): no single frame rate or no poll time, so no prediction")
    return "\n".join(lines) + "\n"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("paths", nargs="+", help="session records (run-NN-*.json) or folders of them")
    parser.add_argument("--group", choices=("auto", "all"), default="auto", help="auto: by A/B arm, else mode; all: one group")
    parser.add_argument("--json", help="write the report as JSON")
    parser.add_argument("--markdown", help="write the report as markdown")
    args = parser.parse_args(argv)
    sessions = read_sessions(args.paths)
    report = build_report(sessions, args.group)
    if not any(group["plays"] for group in report["groups"].values()):
        print("no session with a play and the host trace was found", file=sys.stderr)
        return 2
    text = markdown(report)
    print(text, end="")
    if args.json:
        pathlib.Path(args.json).write_text(json.dumps(report, indent=2) + "\n")
    if args.markdown:
        pathlib.Path(args.markdown).write_text(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
