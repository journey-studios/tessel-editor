#!/usr/bin/env python3
"""Proves that dragging the playhead keeps the monitor updating, and that it ends on the last position.

    python3 scripts/seek-scrub-proof.py [--socket PATH] [--seconds 2] [--hz 30] [--span 120]
                                        [--pixel-proof] [--json FILE] [--min-updates 5]

It sends exactly what the timeline's `on_drag_move` sends (`set timeline.playhead frame N`, one
semantic request per tick; no coordinates, no clicks) to a running editor and reads
`get monitor.viewport`. Nothing in the project changes: the playhead is read first and put back
at the end, and the other state is only read. The editor is never started, restarted or closed.

What it measures (all from the editor's own state, not from this script's clock alone):

* `presentations_in_window`: frames the monitor finished and showed during the drag (`accepted`
  delta). PASS needs >= `--min-updates` in `--seconds` (the 2 s window of the original report).
* `first_present_ms`: first set -> first new picture.
* `present_gap_ms`: time between consecutive pictures during the drag (median / p95 / max),
  from the **editor's own clock**: with `TESSEL_VIEWPORT_TRACE=1` every accepted frame carries a host
  timestamp (`done_ms`), and the gaps are the differences between those. Without the trace the
  script cannot know when a picture appeared, and it reports `sampled_present_gap_ms` instead:
  the spacing of *polls that noticed a new picture*. That figure is bounded by the poll interval
  (`--poll-ms`), merges frames that finished between two polls, and the polling itself loads the UI
  thread, so it is a lower bound of the real rate and never evidence of a gap. It is never used
  for a PASS.
* `lag_frames`: how far the picture on screen was behind the playhead, sampled during the drag.
  The picture's position is `session.presented.frame` (builds with the scheduler) or, for older
  builds, derived from `last_frame.request` and the request counter after every set.
* `release_to_final_ms`: last set -> the picture of the *last* position is on screen. Converged
  means `last_frame.revision == session.revision`, status `ready`, nothing pending, and (when the
  build reports it) `presented.frame == final`. `phase == ready` alone is NOT enough: the phase
  stays `ready` while the request for the final position is still rendering.
* `stats`: session counters delta (requests / accepted / cancelled / dropped_stale / coalesced).
* `render_ms`: the runtime's per-frame draw time (readback and hash included when inspected).

With the trace on (host-clock mode) the script samples nothing during the drag: the only calls
are the `set`s and three `render.status` reads (before the drag, at release, after convergence);
`monitor.viewport` is polled only after release, to detect convergence. The window of the drag is
placed on the host clock with the midpoint of those reads, so stamps carry an uncertainty of about
one `render.status` round trip (reported as `clock_uncertainty_ms`).

`--pixel-proof` needs a monitor that inspects its frames (start the editor with
`TESSEL_VIEWPORT_INSPECT=1`): it checks that the pictures shown during the drag really differ
(distinct hashes, not just a counter moving) and that, after the drag, the picture on screen is
byte-identical (same hash) to an explicit `render.frame` of the final playhead at the same size.
"""
import argparse
import json
import os
import socket
import statistics
import sys
import threading
import time

DEFAULT_SOCKET = os.path.expanduser("~/.cache/tessel-editor/ui.sock")


class Editor:
    """One short-lived connection per request, like the `ui` CLI."""

    def __init__(self, path, timeout=60):
        self.path, self.timeout = path, timeout

    def send(self, command, **fields):
        with socket.socket(socket.AF_UNIX) as sock:
            sock.settimeout(self.timeout)
            sock.connect(self.path)
            sock.sendall((json.dumps(dict(version=1, command=command, **fields)) + "\n").encode())
            data = b""
            while not data.endswith(b"\n"):
                chunk = sock.recv(1 << 20)
                if not chunk:
                    raise RuntimeError("the editor closed the connection")
                data += chunk
        return json.loads(data)

    def viewport(self):
        reply = self.send("get", target="monitor.viewport")
        if not reply.get("ok"):
            raise RuntimeError(f"get monitor.viewport failed: {reply}")
        return reply["data"]["state"]

    def playhead(self):
        return self.send("get", target="timeline.playhead")["data"]["state"]

    def render_status(self):
        """The viewport report with the long trace window, and the local midpoint of the call."""
        sent = time.monotonic()
        reply = self.send("op", request={"op": "render.status"})
        received = time.monotonic()
        if not reply.get("ok"):
            raise RuntimeError(f"render.status failed: {reply}")
        data = reply["data"]
        return data.get("report", data), (sent + received) / 2, (received - sent) * 1000

    def seek(self, frame):
        reply = self.send("set", target="timeline.playhead", field="frame", value=str(frame))
        if not reply.get("ok"):
            raise RuntimeError(f"set playhead failed: {reply}")
        return reply


def proxy_layers(frame):
    """Layers of this frame drawn from a preview proxy (0 when the frame says nothing)."""
    return ((frame or {}).get("source") or {}).get("proxy_layers") or 0


def reduced_layers(frame):
    """Layers below the final quality: a proxy copy, or a scene drawn with fewer MSAA samples (BL129)."""
    source = (frame or {}).get("source") or {}
    return (source.get("proxy_layers") or 0) + (source.get("reduced_scene_layers") or 0)


def stats_of(state):
    return dict(state["session"]["stats"])


def converged(state, final_frame, refined=True):
    """The picture of the last position is on screen and nothing is left to render.

    With `refined` (the default) a reduced picture does not count (a proxy, or a scene with fewer
    MSAA samples): the monitor must have replaced it with the final-quality one (`refine_pending`
    false, no reduced layer left).
    `refined=False` is the moment the *first* picture of the last position is up.
    """
    session = state["session"]
    if refined and (session.get("refine_pending") or reduced_layers(state.get("last_frame"))):
        return False
    last = state.get("last_frame")
    if state["phase"] != "ready" or session["status"] != "ready" or session.get("pending"):
        return False
    if not last or last["revision"] != session["revision"]:
        return False
    presented = session.get("presented")
    return presented is None or presented["frame"] == final_frame


def host_spans(frames):
    """Median / p95 / max of the host-side spans of the frames of this run."""
    names = [
        "prepare_ms", "serialize_ms", "dispatch_ms", "transit_ms",
        "runtime_total_ms", "runtime_render_ms", "accept_ms", "submit_to_done_ms",
    ]
    out = {}
    for name in names:
        values = [f[name] for f in frames]
        if values:
            out[name] = {
                "median": round(statistics.median(values), 3),
                "p95": round(percentile(values, 0.95), 3),
                "max": round(max(values), 3),
            }
    return out


def percentile(values, share):
    if not values:
        return None
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(round(share * (len(ordered) - 1))))]


def run(editor, args):
    start_state = editor.viewport()
    if start_state["phase"] != "ready":
        raise SystemExit(f"the viewport is {start_state['phase']}, not ready")
    original = editor.playhead()
    low, high = original["min"], original["max"]
    span = max(8, min(args.span, high - low))
    # Drag across `span` frames from where the playhead is, away from the end it is nearest to.
    origin = original["frame"] if original["frame"] + span <= high else max(low, high - span)

    deadline = time.monotonic() + 10
    while time.monotonic() < deadline and not converged(start_state, original["frame"]):
        time.sleep(0.05)
        start_state = editor.viewport()
    base = stats_of(start_state)
    inspected = bool(start_state["session"].get("inspect_previews")) or bool(
        (start_state.get("last_frame") or {}).get("hash")
    )

    base_trace_total = None
    traced = bool((start_state["session"].get("trace") or {}).get("enabled"))
    anchor = None
    if traced:
        # Where the drag starts on the editor's clock (midpoint of the call, local time alongside).
        report, local_mid, round_trip = editor.render_status()
        anchor = {"host_ms": report["session"]["trace"]["now_ms"], "local": local_mid, "rt": round_trip}
        base_trace_total = report["session"]["trace"]["frames_total"]

    ticks = int(args.seconds * args.hz)
    positions = [origin + round(i * span / ticks) for i in range(1, ticks + 1)]
    samples = []  # (t, state)
    stop = threading.Event()

    def sampler():
        while not stop.is_set():
            try:
                samples.append((time.monotonic(), editor.viewport()))
            except Exception as error:  # the editor went away: the proof fails below
                samples.append((time.monotonic(), {"error": str(error)}))
                return
            time.sleep(args.poll_ms / 1000)

    # Host-clock mode does not poll during the drag (polling is load on the thread it measures);
    # the pixel proof needs the shown hashes, which only polling can see.
    sampling = args.pixel_proof or not traced
    thread = threading.Thread(target=sampler, daemon=True)
    if sampling:
        thread.start()

    set_times, requests_after = [], []
    t0 = time.monotonic()
    for index, frame in enumerate(positions):
        due = t0 + index / args.hz
        time.sleep(max(0.0, due - time.monotonic()))
        sent = time.monotonic()
        editor.seek(frame)
        set_times.append((sent, time.monotonic(), frame))
    release = time.monotonic()
    drag_seconds = release - t0
    final_frame = positions[-1]

    # Convergence: poll until the picture of the final position is on screen.
    final_state, converged_at, picture_at = None, None, None
    while time.monotonic() - release < args.settle_timeout:
        state = editor.viewport()
        if picture_at is None and converged(state, final_frame, refined=False):
            picture_at = time.monotonic()
        if converged(state, final_frame):
            final_state, converged_at = state, time.monotonic()
            break
        time.sleep(0.01)
    stop.set()
    if sampling:
        thread.join(timeout=5)
    final_state = final_state or editor.viewport()
    final_status = editor.render_status() if traced else None

    live = [(t, s) for t, s in samples if "error" not in s]
    during = [(t, s) for t, s in live if t <= release]
    window_end = t0 + args.seconds
    in_window = [(t, s) for t, s in live if t <= window_end] or during

    def accepted_at(sample):
        return sample["session"]["stats"]["accepted"] - base["accepted"]

    presentations_in_window = max([accepted_at(s) for _, s in in_window] or [0])
    presentations_during_drag = max([accepted_at(s) for _, s in during] or [0])

    # When did each new picture first appear? Sampled: a lower bound on the real rate, bounded by
    # the poll interval. Used only when the editor has no trace (see the module docstring).
    appeared, seen = [], 0
    for t, state in live:
        count = accepted_at(state)
        if count > seen:
            appeared.append(t)
            seen = count
    first_present_ms = round((appeared[0] - t0) * 1000, 1) if appeared else None
    gaps = [(b - a) * 1000 for a, b in zip(appeared, appeared[1:]) if a <= release]

    # Host-clock analysis: every accepted frame of this run with the editor's own timestamps.
    host = None
    if traced:
        trace = final_status[0]["session"]["trace"]
        frames = [f for f in trace["recent"] if f["id"] > base["requests"]]
        if trace["frames_total"] - (base_trace_total or 0) > len(trace["recent"]):
            raise SystemExit("more frames than the trace window keeps: shorten the drag")

        def to_host(local):
            return anchor["host_ms"] + (local - anchor["local"]) * 1000

        def to_local(host_ms):
            return anchor["local"] + (host_ms - anchor["host_ms"]) / 1000

        window_end_host = to_host(t0) + args.seconds * 1000
        release_host = to_host(release)
        in_window_frames = [f for f in frames if f["done_ms"] <= window_end_host]
        during_frames = [f for f in frames if f["done_ms"] <= release_host]
        stamps = [f["done_ms"] for f in during_frames]
        host = {
            "frames": frames,
            "in_window": len(in_window_frames),
            "during": len(during_frames),
            "first_present_ms": round(frames[0]["done_ms"] - to_host(t0), 1) if frames else None,
            "gaps": [b - a for a, b in zip(stamps, stamps[1:])],
            "release_to_final_ms": round(frames[-1]["done_ms"] - release_host) if frames else None,
            "to_local": to_local,
        }
        presentations_in_window = host["in_window"]
        presentations_during_drag = host["during"]
        first_present_ms = host["first_present_ms"]
        gaps = host["gaps"]

    # Lag: frames between the playhead (known from the schedule) and the picture on screen.
    def playhead_at(t):
        value = origin
        for _, done, frame in set_times:
            if done <= t:
                value = frame
        return value

    request_to_frame = {}
    if final_state["session"].get("presented") is None:
        # Older builds: every set is one request, so request n belongs to the n-th change.
        counter = base["requests"]
        for _, _, frame in set_times:
            counter += 1
            request_to_frame[counter] = frame
    lags = []
    if host:
        for frame in host["frames"]:
            if frame["done_ms"] <= release_host and frame["shown_frame"] is not None:
                lags.append(abs(playhead_at(host["to_local"](frame["done_ms"])) - frame["shown_frame"]))
    for t, state in ([] if host else during):
        presented = state["session"].get("presented")
        if presented and presented["frame"] is not None:
            shown = presented["frame"]
        else:
            request = (state.get("last_frame") or {}).get("request")
            shown = request_to_frame.get(request)
        if shown is not None and accepted_at(state) > 0:
            lags.append(abs(playhead_at(t) - shown))

    end = stats_of(final_state)
    delta = {key: end[key] - base.get(key, 0) for key in end}
    last_frame = final_state.get("last_frame") or {}
    result = {
        "inspect": inspected,
        "drag": {"seconds": round(drag_seconds, 3), "ticks": ticks, "from": origin, "to": final_frame},
        "set_ms": {
            "median": round(statistics.median((b - a) * 1000 for a, b, _ in set_times), 2),
            "max": round(max((b - a) * 1000 for a, b, _ in set_times), 2),
        },
        "presentations_in_window": presentations_in_window,
        "presentations_during_drag": presentations_during_drag,
        "first_present_ms": first_present_ms,
        # `clock` says whose clock the figures come from: the editor's (trustworthy) or the
        # script's polling (a lower bound; do not read a gap into it).
        "clock": "host" if host else "observer",
        ("present_gap_ms" if host else "sampled_present_gap_ms"): {
            "median": round(statistics.median(gaps), 1) if gaps else None,
            "p95": round(percentile(gaps, 0.95), 1) if gaps else None,
            "max": round(max(gaps), 1) if gaps else None,
        },
        "lag_frames": {
            "median": statistics.median(lags) if lags else None,
            "p95": percentile(lags, 0.95),
            "max": max(lags) if lags else None,
        },
        "converged": converged_at is not None,
        "release_to_final_ms": (
            host["release_to_final_ms"] if host else round((converged_at - release) * 1000)
        )
        if converged_at
        else None,
        # First picture of the last position (a proxy one counts) vs the original-quality one.
        "release_to_picture_ms": round((picture_at - release) * 1000) if picture_at else None,
        # Polled (so bounded by the poll interval and the UI thread's answer time): when the
        # original-quality picture replaced the proxy one. Equals the picture time without proxies.
        "release_to_original_polled_ms": round((converged_at - release) * 1000) if converged_at else None,
        "stats_delta": delta,
        "presented_frame": (final_state["session"].get("presented") or {}).get("frame"),
        "last_frame": {
            "render_ms": last_frame.get("render_ms"),
            "total_ms": last_frame.get("total_ms"),
            "inspected": bool(last_frame.get("hash")),
        },
        "clock_uncertainty_ms": round(anchor["rt"] / 2, 1) if anchor else None,
        "host_spans_ms": host_spans(host["frames"]) if host else None,
        "renderer": (final_state.get("capabilities") or {}).get("renderer"),
        "output": final_state.get("output"),
    }

    checks = {
        f"at least {args.min_updates} pictures in {args.seconds:g}s": presentations_in_window >= args.min_updates,
        "converged on the last position": converged_at is not None,
        "no render was left cancelled-starved (accepted > 0)": delta.get("accepted", 0) > 0,
    }

    if args.pixel_proof:
        checks.update(pixel_proof(editor, result, live, release, final_state, base, final_frame))

    if not getattr(args, "no_restore", False):
        # Put the playhead back, like the user left it, and wait for the monitor to follow.
        editor.seek(original["frame"])
        settle = time.monotonic() + 10
        while time.monotonic() < settle:
            state = editor.viewport()
            if converged(state, original["frame"]):
                break
            time.sleep(0.02)
        result["restored_playhead"] = editor.playhead()["frame"] == original["frame"]
        checks["the playhead is back where it was"] = result["restored_playhead"]

    result["checks"] = checks
    result["pass"] = all(checks.values())
    return result


def pixel_proof(editor, result, live, release, final_state, base, final_frame):
    """Distinct pictures during the drag, and the final picture equals an explicit render of it."""
    checks = {}
    if not result["inspect"]:
        return {"pixel proof needs TESSEL_VIEWPORT_INSPECT=1 (the monitor did not report hashes)": False}
    shown = {}
    for t, state in live:
        last = state.get("last_frame") or {}
        if t <= release and last.get("hash"):
            shown[last["request"]] = last["hash"]
    distinct = len(set(shown.values()))
    result["pixels"] = {"pictures_seen": len(shown), "distinct_hashes": distinct}
    checks["the pictures shown during the drag differ from each other (not a counter alone)"] = distinct >= max(
        3, min(result["presentations_during_drag"], 8) // 2
    )

    on_screen = final_state.get("last_frame") or {}
    size = final_state.get("output") or {}
    explicit = editor.send(
        "op",
        request={
            "op": "render.frame",
            "frame": final_frame,
            "width": size.get("width"),
            "height": size.get("height"),
            "probes": [],
        },
    )
    frame = (explicit.get("data") or {}).get("frame") or {}
    result["pixels"].update({"on_screen_hash": on_screen.get("hash"), "explicit_hash": frame.get("hash")})
    checks["the picture on screen at release is the explicit render of the last position"] = bool(
        on_screen.get("hash") and on_screen.get("hash") == frame.get("hash")
    )
    return checks


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--socket", default=DEFAULT_SOCKET)
    parser.add_argument("--seconds", type=float, default=2.0)
    parser.add_argument("--hz", type=float, default=30.0)
    parser.add_argument("--span", type=int, default=120, help="frames the drag crosses")
    parser.add_argument("--min-updates", type=int, default=5)
    parser.add_argument("--settle-timeout", type=float, default=10.0)
    parser.add_argument("--poll-ms", type=float, default=10.0, help="gap between `get monitor.viewport` samples")
    parser.add_argument("--pixel-proof", action="store_true")
    parser.add_argument("--no-restore", action="store_true", help="leave the last scrub frame for caller-side inspection (isolated sessions)")
    parser.add_argument("--json", help="also write the result here")
    args = parser.parse_args()

    result = run(Editor(args.socket), args)
    text = json.dumps(result, indent=2)
    print(text)
    if args.json:
        with open(args.json, "w") as handle:
            handle.write(text + "\n")
    failed = [name for name, ok in result["checks"].items() if not ok]
    if failed:
        print("FAIL: " + "; ".join(failed), file=sys.stderr)
        sys.exit(1)
    print("PASS", file=sys.stderr)


if __name__ == "__main__":
    main()
