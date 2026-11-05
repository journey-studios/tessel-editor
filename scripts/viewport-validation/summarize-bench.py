#!/usr/bin/env python3
"""summarize-bench.py life|video|paced|spark|hash <raw.jsonl ...> : compact tables from the bench output."""
import json, sys, re
kind = sys.argv[1]; rows = []
for p in sys.argv[2:]:
    try:
        for l in open(p):
            try: rows.append(json.loads(l))
            except Exception: pass
    except FileNotFoundError: pass
f = lambda d, k="p50": "-" if not d else round(d[k], 1)
if kind == "life":
    by = {}
    for r in rows:
        c = r["cfg"]; key = (r["engine"], c["script"], c.get("splat", "")); by.setdefault(key, {})[c["variant"]] = r
    for key, vs in sorted(by.items()):
        print("==", key, "fatal:", {v: x.get("fatal", "")[:160] for v, x in vs.items() if x.get("fatal")}, "errors:", {v: x.get("errors") for v, x in vs.items() if x.get("errors")})
        base = vs.get("baseline-trace")
        if not base or "steps" not in base: continue
        names = [k for k in base["steps"] if not k.endswith("__ms")]
        for v, x in vs.items():
            if v == "baseline-trace" or "steps" not in x: continue
            diff = {n: (base["steps"][n], x["steps"].get(n)) for n in names if base["steps"][n] != x["steps"].get(n)}
            print("  ", v, "steps", len(names), "different:", json.dumps(diff)[:1200], "info", x.get("info"))
        print("   baseline self-checks:", {n: base["steps"][n] for n in names if any(t in n for t in ("released", "cancel", "supersede", "release_during", "double", "abort", "lose_during", "while_lost"))})
        if base.get("growth"):
            for v, x in vs.items():
                print("  ", v); [print("     ", g["label"], g["info"], g["mem"]) for g in x.get("growth", [])]
if kind == "video":
    print("name | n | miss% | wall p50/p95/p99 | hit | miss | fetch | decode | final(upload+draw) | verify mismatches | fail | serve p50 | errs")
    for r in rows:
        c = r["cfg"]
        if c.get("mode") != "video": continue
        if r.get("fatal") or r.get("load", {}).get("state") != "ready": print(r["engine"], c["name"], "FAIL", (r.get("fatal") or json.dumps(r.get("load")))[:220]); continue
        print(r["engine"][:6], c["name"], "|", r.get("seq_len"), "|", None if r.get("miss_rate") is None else round(r["miss_rate"] * 100), "|", f(r.get("wall")), f(r.get("wall"), "p95"), f(r.get("wall"), "p99"), "|", f(r.get("wall_hit")), "|", f(r.get("wall_miss")), f(r.get("wall_miss"), "p95"), "|", f(r.get("fetch")), "|", f(r.get("decode")), f(r.get("decode"), "p95"), "|", f(r.get("final")), f(r.get("final"), "p95"), "|", r.get("mismatches"), r.get("wrong_samples", [])[:5] if r.get("mismatches") else "", "|", r.get("failed"), "|", r.get("native_serve", {}).get("p50"), "|", len(r.get("errors", [])), "mem", r.get("mem_after"))
if kind == "paced":
    print("name | presented/inputs per_s | latency p50/p95/p99 | wall p50/p95 | lag | release→final | fail | mem")
    for r in rows:
        c = r["cfg"]; p = r.get("paced")
        if not p: print(r["engine"], c["name"], "FAIL", (r.get("fatal") or json.dumps(r.get("load")))[:200]); continue
        print(r["engine"][:6], c["name"], "|", p["presented"], "/", p["inputs"], round(p["presented_per_s"], 1), "|", f(p["latency"]), f(p["latency"], "p95"), f(p["latency"], "p99"), "|", f(p["wall"]), f(p["wall"], "p95"), "|", f(p["lag_inputs"]), f(p["lag_inputs"], "p95"), "|", round(p["release_to_final_ms"]), "|", p["failed"], "|", r.get("mem_after"), r.get("prefetch"))
if kind == "spark":
    print("name | n | render p50/mean/p95/p99 | sort | final | gpu read | wall | rafLag | fps | cold | load ms | mem | attempts")
    for r in rows:
        c = r["cfg"]
        if r.get("fatal"): print(r["engine"], c.get("name"), "FATAL", r["fatal"][:200]); continue
        print(r["engine"][:6], c["name"], c.get("width", ""), "|", (r.get("render") or {}).get("n"), "|", f(r.get("render")), f(r.get("render"), "mean"), f(r.get("render"), "p95"), f(r.get("render"), "p99"), "|", f(r.get("sort")), f(r.get("sort"), "p95"), "|", f(r.get("final")), "|", f(r.get("gpu")), f(r.get("gpu"), "p95"), "|", f(r.get("wall"), "mean"), "|", f(r.get("raf_lag")), "|", round(r.get("fps", 0), 1), "|", round((r.get("cold_first") or {}).get("render", 0)), "|", round(r.get("asset_load_ms", 0)), "|", r.get("mem_after"), "|", (r.get("attempts") or {}).get("mean"), "| info", (r.get("info") or {}).get("rt_hits"))
if kind == "hash":
    by = {}
    for r in rows:
        c = r["cfg"]; key = re.sub(r":(baseline|retained)$", "", c["name"]); by.setdefault((r["engine"], key), {})[c["variant"]] = r
    same = diff = 0
    for (eng, key), vs in sorted(by.items()):
        b, t = vs.get("baseline"), vs.get("retained")
        if not b or not t: print(eng, key, "incomplete", {v: (x.get("fatal") or "ok")[:90] for v, x in vs.items()}); continue
        if b.get("fatal") or t.get("fatal"): print(eng, key, "FATAL", b.get("fatal", "")[:140], t.get("fatal", "")[:140]); continue
        hb, ht = [h["hash"] for h in b["hashes"]], [h["hash"] for h in t["hashes"]]
        rep = {}; [rep.setdefault(h["i"], set()).add(h["hash"]) for x in (b, t) for h in x["hashes"]]
        self_ok = all(len(v) == 1 for v in rep.values())
        ms = sorted(h["ms"] or 0 for h in b["hashes"]); mt = sorted(h["ms"] or 0 for h in t["hashes"])
        ok = hb == ht; same += ok; diff += not ok
        print(eng[:9], key, "frames", len(hb), "identical" if ok else f"DIFFER {sum(x != y for x, y in zip(hb, ht))}", "repeat-stable" if self_ok else "REPEATS UNSTABLE", "ms", round(ms[len(ms)//2]), round(mt[len(mt)//2]), "distinct", len(set(hb)))
    print("identical:", same, "different:", diff)
