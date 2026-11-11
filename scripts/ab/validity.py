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


def failed_assets(events):
    """The `asset_state` events in which the page says an asset failed to load: a play that lost a picture's source is not the play that was asked for."""
    return [e for e in events if e["kind"] == "asset_state" and (e.get("fields") or {}).get("state") == "failed"]


def core_validity(record, mode, unfinished):
    """The ONE functional validity of every experiment: the session ran to its end (`plays_ok`; `unfinished` says what that means for the experiment),
    no asset failed to load, no request answered with an error status, nothing was lost, the host ran the mode that was asked and the editor was alive.
    Returns the reasons it is not valid; an experiment adds only what is its own (the factor ran, the warm ran, the page account)."""
    reasons = []
    status = record.get("status") or {}
    asset_io = status.get("asset_io") or {}
    if record.get("error"):
        reasons.append(f"error: {redact(record['error'])}")
    gone = record.get("editor_exit") or {}
    if gone and not gone.get("alive", True):
        reasons.append(f"the editor process ended by itself (return code {gone.get('returncode_before_stop')})")
    if asset_io.get("mode") != mode:
        reasons.append(f"the host reports mode {asset_io.get('mode')!r}, the arm asked for {mode!r}")
    pool = asset_io.get("pool") or {}
    if (pool.get("saturated") or 0) > 0:
        reasons.append(f"the read queue saturated {pool['saturated']} time(s)")
    if (pool.get("panicked") or 0) > 0:
        reasons.append("a read panicked")
    if (pool.get("stale") or 0) > 0:
        reasons.append(f"{pool['stale']} read(s) were answered stale during a play")
    if record.get("events_lost"):
        reasons.append(f"{record['events_lost']} trace event(s) were lost")
    if not record.get("plays_ok", False):
        reasons.append(unfinished)
    events = record.get("events", [])
    bad = [e for e in events if e["kind"] == "asset_io" and e["fields"].get("status") not in ACCEPTABLE_STATUS]
    if bad:
        reasons.append(f"{len(bad)} asset request(s) answered with another status ({sorted({e['fields'].get('status') for e in bad})})")
    failed = failed_assets(events)
    if failed:
        reasons.append(f"{len(failed)} asset load(s) failed in the page")
    return reasons


def session_validity(session):
    """(valid, [reasons]) of a whole-project session: the core, and the scene warm (on in both arms) has to have run."""
    reasons = core_validity(session, session["mode"], "a play did not finish normally")
    warmed = ((session.get("status") or {}).get("stats") or {}).get("warmed")
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
    """(valid, [reasons]) of a window play: the core, and every frame has to carry the page's account (the page timing was on)."""
    reasons = core_validity(record, record["io"], "the window did not play to its end")
    if not any(e["kind"] == "frame_page" for e in record.get("events", [])):
        reasons.append("no frame carried the page account (the page timing was not on)")
    return (not reasons), reasons
