"""Whether a recorded session measured what it claims, and the analysis of one session (per play). Functional validity only, never a performance judgement."""
import re
import statistics

from ab.play import entries_of, play_pictures, play_segments, summarize_play, window_report
from ab.stats import memory_summary, percentile, quiet_class

ACCEPTABLE_STATUS = {200, 206}


# What a report may say of a failure: `{"code", "kind"}`, nothing of the message. `kind` is one of ERROR_KINDS; `code` is a host answer's error code
# (`[a-z_]{1,32}`) or an exception's class name (`<Name>Error|Exception`, at most 40 letters), and a text that offers neither is `other`. The message itself (it can carry a path, a
# file name, a client's name) stays in the session's own raw record, which is local and never copied into a report.
ERROR_KINDS = ("timeout", "ended_early", "host_refused", "exception", "other")
_HOST_CODE = re.compile(r"'code': '([a-z_]{1,32})'")
_EXCEPTION = re.compile(r"\b([A-Z][A-Za-z]{0,38}(?:Error|Exception))\(")


def published_error(text):
    """The `{"code", "kind"}` of a failure's raw text. Only a token that fits a fixed pattern is ever taken from it, never a piece of the text."""
    text = str(text)
    if "did not end within" in text:
        return {"code": "play_did_not_end", "kind": "timeout"}
    if "before its end" in text:
        return {"code": "window_stopped", "kind": "ended_early"}
    host = _HOST_CODE.search(text)
    if host:
        return {"code": host.group(1), "kind": "host_refused"}
    exception = _EXCEPTION.search(text)
    if exception:
        return {"code": exception.group(1), "kind": "exception"}
    return {"code": "other", "kind": "other"}


def error_label(error):
    """A published error as one short word for a table or a reason."""
    return f"{error['kind']}:{error['code']}"


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
        reasons.append(f"error: {error_label(published_error(record['error']))}")
    # What the session recorded while it ran, including the poll of its `finally` (after `plays_ok` was decided): a trace that could not be
    # read is an incomplete trace. Published as codes and kinds, once each.
    recorded = sorted({error_label(published_error(text)) for text in record.get("errors") or []})
    if recorded:
        reasons.append(f"the session recorded error(s): {', '.join(recorded)}")
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


def failed_session(record, complete):
    """Whether the session did not run as claimed, whatever its plays showed: it died (`error`), recorded errors (a play that timed out or was refused,
    a trace that could not be read), its plays did not run to their end (`plays_ok`), the editor was gone before the harness stopped it, or the plays that
    were asked for are not all there (`complete`, the runner's own reading). The ONE definition, for every runner that tells a failed run from a hung one.

    A HANG is something else: what a play showed, a finding about the arm that `hung_play` judges. A session can be both, and a hang never excuses a
    failure: the run is then reported as failed (exit 1) and its hang is still counted."""
    gone = record.get("editor_exit") or {}
    return bool(record.get("error") or record.get("errors") or not record.get("plays_ok", True) or (gone and not gone.get("alive", True)) or not complete)


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
    """(valid, [reasons]) of a window play: the core, and every picture of its plays has to carry the page's account (the page timing was on).

    WHY every picture: the host asks the page for its account for all frames or for none (`telemetry.timing` of the handshake), so a picture
    without one is a lost report, and `decompose_play` leaves the page parts of exactly that picture without a value."""
    reasons = core_validity(record, record["io"], "the window did not play to its end")
    events = record.get("events", [])
    accounted = {e["fields"].get("id") for e in events if e["kind"] == "frame_page"}
    pictures = [f for segment in play_segments(events) for f in play_pictures(record.get("frames", []), segment)]
    missing = [f for f in pictures if f["id"] not in accounted]
    if not accounted:
        reasons.append("no frame carried the page account (the page timing was not on)")
    elif missing:
        reasons.append(f"{len(missing)} of {len(pictures)} picture(s) of the plays carried no page account (a lost report)")
    return (not reasons), reasons
