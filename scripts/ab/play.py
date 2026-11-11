"""The pictures of a play, the gaps between them, where a gap sits and what the host was doing around it (one clock: the host's)."""
import math

from ab.stats import stats

SLOW_DIAGNOSTIC_MS = 62.5
SLOWER_DIAGNOSTIC_MS = 100.0
ENTRY_WINDOW = 6
VIDEO_ENTRY_WINDOW = 2
# A play that shows fewer than this share of the project's frames, or waits this long between two pictures, is a HANG: reported on its own and counted
# per arm. It does not make a run invalid (it is what the editor did), and it is never dropped.
HANG_PICTURES_SHARE = 0.9
HANG_GAP_MS = 2000.0


def pictures_of(frames):
    """Accepted pictures in the order the host accepted them."""
    return sorted((f for f in frames if f.get("done_ms") is not None and f.get("shown_frame") is not None),
                  key=lambda f: (f["done_ms"], f["id"]))


def play_segments(events):
    """[(command_ms, clock_started_ms, end_ms)] of every play, from the host's own events (one clock)."""
    commands = sorted((e["t_ms"] for e in events if e["kind"] == "play_command"))
    clocks = sorted((e["t_ms"] for e in events if e["kind"] == "play_clock_started"))
    segments = []
    for index, command in enumerate(commands):
        clock = next((c for c in clocks if c >= command), None)
        end = commands[index + 1] if index + 1 < len(commands) else math.inf
        segments.append({"command_ms": command, "clock_ms": clock, "end_ms": end})
    return segments


def play_pictures(frames, segment):
    """The pictures the clock produced during one play: interactive ones, after its clock started, before the next play.

    The picture that follows the end (the refine of the paused playhead) is not interactive and is not part of the play."""
    if segment["clock_ms"] is None:
        return []
    mine = [f for f in pictures_of(frames) if segment["clock_ms"] <= f["done_ms"] < segment["end_ms"]]
    interactive = [f for f in mine if f.get("interactive")]
    return interactive or mine


def gaps_of(pictures):
    """[(frame_before, frame_after, done_before_ms, done_after_ms, gap_ms)] between consecutive accepted pictures."""
    return [(a["shown_frame"], b["shown_frame"], a["done_ms"], b["done_ms"], round(b["done_ms"] - a["done_ms"], 3))
            for a, b in zip(pictures, pictures[1:])]


def locate(frame_after, cuts, prefetch_frames, entry_window=ENTRY_WINDOW, video_starts=(), video_window=VIDEO_ENTRY_WINDOW):
    """Where a gap that ends on `frame_after` sits: at a cut, at the first frames of a video/still clip, before a cut
    (inside the prefetch window), or elsewhere. The cuts and clip starts come from the timeline."""
    for cut in cuts:
        if cut <= frame_after <= cut + entry_window:
            return f"cut:{cut}"
    for start in video_starts:
        if start <= frame_after <= start + video_window:
            return f"video:{start}"
    for cut in cuts:
        if cut - prefetch_frames <= frame_after < cut:
            return f"prefetch:{cut}"
    return "other"


def overlap_ms(a0, a1, b0, b1):
    return max(0.0, min(a1, b1) - max(a0, b0))


def span_interval(event):
    """Where a span the page reported lay on the host clock: it lasted `ms` and had ended `ago_ms` before the host got the report (the message that
    carries it can leave the page long after), so it is [arrival - ago - ms, arrival - ago]. A span without `ago_ms` (timing off) is placed at its arrival."""
    fields = event["fields"]
    end = event["t_ms"] - (fields.get("ago_ms") or 0)
    return end - (fields.get("ms") or 0), end


def events_in(events, kind, t0, t1):
    return [e for e in events if e["kind"] == kind and t0 <= e["t_ms"] <= t1]


PAGE_WORK_KEYS = ("page_decode_ms", "page_fetch_ms", "page_load_other_ms", "page_warm_ms")


def page_work_ms(context):
    """The page work that overlapped a gap, from a `gap_context`: decode, fetch, load work the loader did not split, and warm. One sum for every report."""
    return sum(context.get(key) or 0 for key in PAGE_WORK_KEYS)


def gap_context(events, t0, t1):
    """What the host was doing around one gap [t0, t1] (host clock): a CORRELATION, not a cause."""
    ticks = events_in(events, "tick", t0, t1)
    callbacks = [e for e in events if e["kind"] == "asset_cb"
                 and overlap_ms(e["fields"]["recv_ms"], e["fields"]["recv_ms"] + e["fields"]["callback_ms"], t0, t1) > 0]
    reads = [e for e in events if e["kind"] == "asset_io"
             and overlap_ms(e["t_ms"] - e["fields"].get("read_ms", 0) - e["fields"].get("respond_ms", 0), e["t_ms"], t0, t1) > 0]
    prefetch = events_in(events, "prefetch", t0, t1)
    sync = events_in(events, "sync", t0, t1)
    # The page's own work, anchored at the host arrival of its report (a page duration is never subtracted from a host
    # stamp; the interval [arrival - duration, arrival] is the page work that had ended by then).
    # A load's page-clock timings: `load_ms` is always there; `fetch_ms` / `decode_ms` only for loaders that split it (a frame sequence, a scene parse and
    # others report `load_ms` alone). What a loader did not split is never called fetch: it is `page_load_other_ms`, load work of an unknown kind.
    decodes, fetches, unsplit = [], [], []
    for e in events:
        timings = (e["fields"].get("timings") or {}) if e["kind"] == "asset_state" else {}
        if not timings.get("load_ms"):
            continue
        end, decode, fetch = e["t_ms"], timings.get("decode_ms") or 0, timings.get("fetch_ms")
        decodes.append(overlap_ms(end - decode, end, t0, t1))
        if fetch is None:
            unsplit.append(overlap_ms(end - timings["load_ms"], end - decode, t0, t1))
        else:
            fetches.append(overlap_ms(end - decode - fetch, end - decode, t0, t1))
            unsplit.append(overlap_ms(end - timings["load_ms"], end - decode - fetch, t0, t1))
    warms = [overlap_ms(*span_interval(e), t0, t1) for e in events if e["kind"] == "runtime_span" and e["fields"].get("k") == "warm"]
    return {
        "page_decode_ms": round(sum(decodes), 3),
        "page_fetch_ms": round(sum(fetches), 3),
        "page_load_other_ms": round(sum(unsplit), 3),
        "page_warm_ms": round(sum(warms), 3),
        "ticks": len(ticks),
        "max_tick_late_ms": max((e["fields"]["late_ms"] for e in ticks), default=None),
        "max_tick_body_ms": max((e["fields"].get("body_ms") or 0 for e in ticks), default=None),
        "ui_callback_ms": round(sum(overlap_ms(e["fields"]["recv_ms"], e["fields"]["recv_ms"] + e["fields"]["callback_ms"], t0, t1)
                                    for e in callbacks), 3),
        "ui_callbacks": len(callbacks),
        "reads": len(reads),
        "read_bytes": sum(e["fields"].get("bytes", 0) for e in reads),
        "read_ms_max": max((e["fields"].get("read_ms", 0) for e in reads), default=None),
        "queue_wait_ms_max": max((e["fields"].get("queue_wait_ms", 0) for e in reads), default=None),
        "prefetch_ms_max": max((e["fields"].get("total_ms") or 0 for e in prefetch), default=None),
        "plans_ahead_ms_max": max((e["fields"].get("plans_ahead_ms") or 0 for e in prefetch), default=None),
        "sync_ms_max": max((e["fields"].get("total_ms") or 0 for e in sync), default=None),
    }


def summarize_play(frames, events, segment, project, fps_budget_ms):
    """Everything one play says, with no judgement."""
    shots = play_pictures(frames, segment)
    gaps = gaps_of(shots)
    values = [g[4] for g in gaps]
    prefetch_frames = int(round(project["fps"] * 2))
    located = {}
    slow = []
    for (before, after, t0, t1, gap), picture in zip(gaps, shots[1:]):
        where = locate(after, project["cuts"], prefetch_frames, video_starts=project.get("video_starts", ()))
        located.setdefault(where, []).append(gap)
        if gap > SLOW_DIAGNOSTIC_MS:
            slow.append({"from_frame": before, "to_frame": after, "gap_ms": gap, "where": where, "context": gap_context(events, t0, t1),
                         "picture": {"submit_to_done_ms": picture["submit_to_done_ms"], "transit_ms": picture["transit_ms"],
                                     "runtime_total_ms": picture["runtime_total_ms"]}})
    ticks = events_in(events, "tick", segment["clock_ms"] or 0, shots[-1]["done_ms"] if shots else 0)
    prefetches = events_in(events, "prefetch", segment["clock_ms"] or 0, shots[-1]["done_ms"] if shots else 0)
    syncs = events_in(events, "sync", segment["clock_ms"] or 0, shots[-1]["done_ms"] if shots else 0)
    callbacks = [e for e in events if e["kind"] == "asset_cb" and (segment["clock_ms"] or 0) <= e["t_ms"] <= (shots[-1]["done_ms"] if shots else 0)]
    reads = [e for e in events if e["kind"] == "asset_io" and (segment["clock_ms"] or 0) <= e["t_ms"] <= (shots[-1]["done_ms"] if shots else 0)]
    first = next((s for s in pictures_of(frames) if s["submitted_ms"] >= segment["command_ms"]), None)
    return {
        "pictures": len(shots),
        "unique_frames": len({s["shown_frame"] for s in shots}),
        "expected_frames": project["length"],
        "interval_ms": stats(values),
        "slow_gt_62_5": sum(1 for v in values if v > SLOW_DIAGNOSTIC_MS),
        "slow_gt_100": sum(1 for v in values if v > SLOWER_DIAGNOSTIC_MS),
        "over_budget_gt_41_7": sum(1 for v in values if v > fps_budget_ms),
        "request_to_accept_ms": stats([s["submit_to_done_ms"] for s in shots]),
        "transit_ms": stats([s["transit_ms"] for s in shots]),
        "runtime_total_ms": stats([s["runtime_total_ms"] for s in shots]),
        "runtime_prepare_ms": stats([s.get("runtime_prepare_ms") for s in shots]),
        "host_prepare_ms": stats([s["prepare_ms"] for s in shots]),
        "tick_late_ms": stats([e["fields"]["late_ms"] for e in ticks]),
        "tick_body_ms": stats([e["fields"].get("body_ms") for e in ticks]),
        "plans_ahead_ms": stats([e["fields"].get("plans_ahead_ms") for e in prefetches]),
        "prefetch_total_ms": stats([e["fields"].get("total_ms") for e in prefetches]),
        "sync_ms": stats([e["fields"].get("total_ms") for e in syncs]),
        "ui_callback_ms": stats([e["fields"]["callback_ms"] for e in callbacks]),
        "ui_callback_total_ms": round(sum(e["fields"]["callback_ms"] for e in callbacks), 3),
        "reads": len(reads),
        "read_bytes": sum(e["fields"].get("bytes", 0) for e in reads),
        "read_ms": stats([e["fields"].get("read_ms") for e in reads]),
        "queue_wait_ms": stats([e["fields"].get("queue_wait_ms") for e in reads]),
        "reads_on_ui_thread": sum(1 for e in reads if e["fields"].get("on_ui_thread")),
        "by_location": {where: stats(vals) | {"slow": sum(1 for v in vals if v > SLOW_DIAGNOSTIC_MS)} for where, vals in sorted(located.items())},
        "slow_detail": slow,
        "play_command_to_first_picture_ms": round(first["done_ms"] - segment["command_ms"], 1) if first else None,
        "play_command_to_clock_ms": round(segment["clock_ms"] - segment["command_ms"], 1) if segment["clock_ms"] else None,
    }


def entries_of(frames, project):
    """Per cut: the first picture shown at the cut: its request→accept and the gap since the picture before it."""
    out = []
    shots = pictures_of(frames)
    for cut in project["cuts"]:
        for index, shot in enumerate(shots):
            if cut <= shot["shown_frame"] <= cut + ENTRY_WINDOW and index and shots[index - 1]["shown_frame"] < cut:
                out.append({"cut": cut, "shown_frame": shot["shown_frame"], "request_to_accept_ms": shot["submit_to_done_ms"],
                            "gap_ms": round(shot["done_ms"] - shots[index - 1]["done_ms"], 1)})
                break
        else:
            out.append({"cut": cut, "missing": True})
    return out


def window_report(frames, project, segment):
    """Per prefetch window and per cut window of one play: gap statistics (so a pause moved from the cut to before it shows)."""
    shots = play_pictures(frames, segment)
    gaps = gaps_of(shots)
    prefetch_frames = int(round(project["fps"] * 2))
    out = []
    for cut in project["cuts"]:
        pre = [g[4] for g in gaps if cut - prefetch_frames <= g[1] < cut]
        at = [g[4] for g in gaps if cut <= g[1] <= cut + ENTRY_WINDOW]
        out.append({"cut": cut,
                    "prefetch_window": stats(pre) | {"slow": sum(1 for v in pre if v > SLOW_DIAGNOSTIC_MS), "excess_ms": round(sum(max(0.0, v - 1000.0 / project["fps"]) for v in pre), 1)},
                    "cut_window": stats(at) | {"slow": sum(1 for v in at if v > SLOW_DIAGNOSTIC_MS)}})
    return out


def hung_play(summary, length):
    """The hang rule of every experiment: `summary` is `summarize_play`'s."""
    return bool(summary["pictures"] < HANG_PICTURES_SHARE * length or (summary["interval_ms"]["max"] or 0) > HANG_GAP_MS)
