"""Nearest-rank statistics, the load class of a session and its memory: no interpolation invents a value."""
import math
import statistics

# Fixed BEFORE any measurement (BL157.1). External load = CPU of every process that is not the session's own, in cores.
QUIET_MEDIAN_CORES = 3.0
QUIET_P95_CORES = 6.0
# A pair is comparable only if both sessions are quiet AND their median external loads differ by at most this.
PAIR_MAX_DELTA_CORES = 2.0


def percentile(values, share):
    if not values:
        return None
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, math.ceil(share * len(ordered)) - 1))
    return round(ordered[index], 3)


def stats(values):
    """p50 / p95 / max / n of a list (None when empty). Nearest-rank percentiles: no interpolation invents a value."""
    values = [v for v in values if v is not None]
    if not values:
        return {"n": 0, "p50": None, "p95": None, "max": None}
    return {"n": len(values), "p50": percentile(values, 0.5), "p95": percentile(values, 0.95), "max": round(max(values), 3)}


def quiet_class(external_cores):
    """`quiet` / `loaded` for a session from its external CPU samples (cores), by the thresholds fixed in this file."""
    if not external_cores:
        return "unknown"
    median = statistics.median(external_cores)
    if median <= QUIET_MEDIAN_CORES and percentile(external_cores, 0.95) <= QUIET_P95_CORES:
        return "quiet"
    return "loaded"


def merge_by_key(seen, items, key):
    for item in items or []:
        seen.setdefault(item[key], item)
    return seen


def memory_summary(series):
    """Median and PEAK of the editor's RSS and of the new WebKit helpers' RSS, each on its own and their sum (labelled a sum)."""
    def part(index):
        values = [row[index] for row in series]
        return {"median": round(statistics.median(values), 1), "peak": round(max(values), 1)} if values else {"median": None, "peak": None}
    return {"samples": len(series), "editor_mib": part(1), "helpers_mib": part(2),
            "rss_sum_mib": part(3), "note": "RSS, not physical footprint; helpers are the WebKit processes new since the baseline; the sum double counts shared pages"}
