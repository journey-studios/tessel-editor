"""One late picture's time split into the parts one clock can tell apart (host send, page queue, page draw, IPC wait, delivery)."""
from ab.play import SLOW_DIAGNOSTIC_MS as SLOW_MS, play_pictures

# A component is "dominant" only when it is at least this much of the picture's time and at least this long.
DOMINANT_MIN_MS = 25.0


# ------------------------------------------------------------------------------------------------ pure logic
def eval_events(events):
    """The WebView's completion of each `frame.render` script: (send time on the host clock, ms from send to completion)."""
    out = []
    for e in events:
        if e["kind"] == "eval_done" and e["fields"].get("kind") == "frame.render":
            out.append({"sent_ms": e["t_ms"] - e["fields"]["ms"], "done_ms": e["t_ms"], "eval_ms": e["fields"]["ms"], "used": False})
    return sorted(out, key=lambda x: x["sent_ms"])


def match_eval(evals, submitted_ms):
    """The first unused completion whose script was sent at or after the submission (scripts leave in order)."""
    for item in evals:
        if not item["used"] and item["sent_ms"] >= submitted_ms - 0.05:
            item["used"] = True
            return item
    return None


def decompose(frame, page_event, evaluation):
    """One accepted picture's time, split into the parts one clock can tell apart, and the dominant one named."""
    total = frame["submit_to_done_ms"]
    page = (page_event or {}).get("page") or {}
    held = page.get("held_ms")
    queue = page.get("queue_ms")
    ipc_wait = (page_event or {}).get("ipc_wait_ms") or 0.0
    pre_send = max(0.0, evaluation["sent_ms"] - frame["submitted_ms"]) if evaluation else None
    parts = {"host_pre_send": pre_send, "page_queue": queue, "page_draw": (held - queue) if held is not None and queue is not None else None,
             "ipc_wait": ipc_wait if page_event else None}
    known = sum(v for v in parts.values() if v is not None)
    # Delivery both ways: everything the host waited for that no part above accounts for. Only meaningful with the page account.
    delivery = max(0.0, total - known) if held is not None else None
    parts["delivery"] = delivery
    named = {k: v for k, v in parts.items() if v is not None}
    dominant = max(named, key=named.get) if named else None
    if dominant is not None and (named[dominant] < DOMINANT_MIN_MS or named[dominant] < 0.4 * total):
        dominant = "spread" if total >= SLOW_MS / 2 else "none"
    detail = None
    if dominant == "page_queue":
        loop = (page.get("loop") or {}).get("max_ms") or 0.0
        ahead = page.get("ahead") or {}
        if loop >= 0.7 * (queue or 0):
            detail = "event_loop_stalled"
        elif (ahead.get("frames") or 0) + (ahead.get("warms") or 0) > 0:
            detail = "behind_queued_work"
        else:
            detail = "unexplained_by_loop_or_queue"
    if dominant == "delivery" and evaluation:
        detail = "in_dominant" if evaluation["eval_ms"] >= 0.7 * delivery else "out_or_later_dominant"
    return {"id": frame["id"], "shown_frame": frame.get("shown_frame"), "total_ms": round(total, 3), "transit_ms": round(frame["transit_ms"], 3),
            "runtime_total_ms": frame["runtime_total_ms"], "eval_ms": round(evaluation["eval_ms"], 3) if evaluation else None,
            "page_loop_max_ms": (page.get("loop") or {}).get("max_ms"), "page_ahead": page.get("ahead"),
            "parts": {k: (round(v, 3) if v is not None else None) for k, v in parts.items()},
            "dominant": dominant, "detail": detail}


def decompose_play(frames, events, segment):
    pictures = play_pictures(frames, segment)
    by_id = {e["fields"]["id"]: e["fields"] for e in events if e["kind"] == "frame_page"}
    # The host's own `frame_page` event carries the page's account; keyed by the frame id.
    evals = eval_events(events)
    out = []
    for picture in sorted(pictures, key=lambda f: f["submitted_ms"]):
        out.append(decompose(picture, by_id.get(picture["id"]), match_eval(evals, picture["submitted_ms"])))
    return out
