"""Whether a recorded session measured what it claims, and the analysis of one session (per play). Functional validity only, never a performance judgement."""
import re
import statistics

from ab.play import entries_of, play_segments, summarize_play, window_report
from ab.stats import memory_summary, percentile, quiet_class

ACCEPTABLE_STATUS = {200, 206}


def redact(text, limit=240):
    """Remove absolute paths, including private filenames with spaces, before publishing a diagnostic.
    Over-redact trailing words up to a quote, colon or newline rather than expose an uncertain path boundary."""
    text = str(text)
    # Include a single component and spaces in the file name. Do not replace known roots first:
    # that would turn /home/user/customer.json into <home>/customer.json and expose the file name.
    return re.sub(r"/[^:'\"\n]+", "<path>", text)[:limit]


def session_validity(session):
    """(valid, [reasons]) of one session: functional validity, never a performance judgement."""
    reasons = []
    mode = session["mode"]
    status = session.get("status") or {}
    asset_io = status.get("asset_io") or {}
    if session.get("error"):
        reasons.append(f"error: {redact(session['error'])}")
    gone = session.get("editor_exit") or {}
    if gone and not gone.get("alive", True):
        reasons.append(f"the editor process ended by itself (return code {gone.get('returncode_before_stop')})")
    if asset_io.get("mode") != mode:
        reasons.append(f"the host reports mode {asset_io.get('mode')!r}, the arm asked for {mode!r}")
    pool = asset_io.get("pool") or {}
    if mode == "async":
        if (pool.get("saturated") or 0) > 0:
            reasons.append(f"the read queue saturated {pool['saturated']} time(s)")
        if (pool.get("panicked") or 0) > 0:
            reasons.append("a read panicked")
        if (pool.get("stale") or 0) > 0:
            reasons.append(f"{pool['stale']} read(s) were answered stale during a play")
    if session.get("events_lost"):
        reasons.append(f"{session['events_lost']} trace event(s) were lost")
    if not session.get("plays_ok", False):
        reasons.append("a play did not finish normally")
    bad = [e for e in session.get("events", []) if e["kind"] == "asset_io" and e["fields"].get("status") not in ACCEPTABLE_STATUS]
    if bad:
        reasons.append(f"{len(bad)} asset request(s) answered with another status ({sorted({e['fields'].get('status') for e in bad})})")
    warmed = (status.get("stats") or {}).get("warmed")
    if not isinstance(warmed, int) or warmed <= 0:
        reasons.append("the scene warm never ran (it must be on in both arms)")
    return (not reasons), reasons


def analyze_session(record, project):
    valid, reasons = session_validity(record)
    segments = play_segments(record.get("events", []))
    names = ["first", "repeat"]
    budget = 1000.0 / project["fps"]
    plays = {}
    for name, segment in zip(names, segments):
        plays[name] = summarize_play(record["frames"], record["events"], segment, project, budget)
        plays[name]["windows"] = window_report(record["frames"], project, segment)
        plays[name]["entries"] = entries_of([f for f in record["frames"] if segment["clock_ms"] and segment["clock_ms"] <= f["done_ms"] < segment["end_ms"]], project)
        plays[name]["playback_report"] = (record.get("played") or {}).get(name)
    external = record.get("external_cores") or []
    return {
        "number": record["number"], "mode": record["mode"], "order_index": record["order_index"],
        "valid": valid, "invalid_reasons": reasons,
        "quiet_class": quiet_class(external),
        "external_median_cores": round(statistics.median(external), 2) if external else None,
        "external_p95_cores": percentile(external, 0.95),
        "loadavg_start": record["host"]["loadavg_start"],
        "loadavg_peak_1m": max((row[0] for row in record.get("loadavg", [])), default=None),
        "memory": memory_summary(record.get("memory_series", [])),
        "asset_io": (record.get("status") or {}).get("asset_io"),
        "warmed": ((record.get("status") or {}).get("stats") or {}).get("warmed"),
        "plays": plays,
    }


def window_validity(record):
    reasons = []
    if record.get("error"):
        reasons.append("error: " + redact(record["error"]))
    gone = record.get("editor_exit") or {}
    if gone and not gone.get("alive", True):
        reasons.append(f"the editor process ended by itself (return code {gone.get('returncode_before_stop')})")
    if record.get("events_lost"):
        reasons.append(f"{record['events_lost']} trace event(s) were lost")
    asset_io = (record.get("status") or {}).get("asset_io") or {}
    if asset_io.get("mode") != record["io"]:
        reasons.append(f"the host reports mode {asset_io.get('mode')!r}, asked {record['io']!r}")
    pool = asset_io.get("pool") or {}
    if (pool.get("saturated") or 0) > 0:
        reasons.append("the read queue saturated")
    if not any(e["kind"] == "frame_page" for e in record.get("events", [])):
        reasons.append("no frame carried the page account (the page timing was not on)")
    if not record.get("plays_ok", False):
        reasons.append("the window did not play to its end")
    bad = [e for e in record.get("events", []) if e["kind"] == "asset_io" and e["fields"].get("status") not in ACCEPTABLE_STATUS]
    if bad:
        reasons.append(f"{len(bad)} asset request(s) answered with another status")
    return (not reasons), reasons
