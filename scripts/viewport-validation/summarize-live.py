#!/usr/bin/env python3
"""summarize-live.py raw.jsonl -> table per (binary, variant, mode, hz, obs): medians across reps, cold excluded."""
import json, sys, statistics as S
rows = [json.loads(l) for l in open(sys.argv[1])]
groups = {}
for r in rows:
    groups.setdefault((r["binary"], r["variant"].replace("-trace", ""), r["mode"], r["hz"], r["obs"]), []).append(r)
K = ["frames_per_s", "cycle", "post_gap", "draw", "eval_call", "eval_to_recv", "post_to_ipc", "ipc_to_on_ipc", "on_ipc", "onipc_to_next_eval", "gpui_draw", "render_gap", "plan", "key", "preview_request", "sync", "state_to_post", "state_to_raf", "post_to_raf", "lag_frames", "input_dispatch", "input_period", "release_to_final_post"]
out = []
for key, rs in sorted(groups.items()):
    warm = [r for r in rs if not r["cold"]] or rs
    rec = dict(zip(("binary", "variant", "mode", "hz", "obs"), key)); rec["reps"] = len(warm); rec["load_max"] = max(r["load"] for r in warm); rec["frames_each"] = [r["res"]["frames"] for r in warm]; rec["foreign_moves"] = sum(r["res"].get("foreign_moves", 0) for r in warm)
    rec["gpui_renders_per_s"] = round(S.median(r["res"]["gpui_renders"] / 2.0 for r in warm), 1); rec["gpui_draw_share_pct"] = round(S.median(r["res"].get("gpui_draw_ms_total", 0) / max(1, r["res"].get("gesture_ms", 2000)) * 100 for r in warm), 1)
    for k in K:
        vals = [r["res"].get(k) for r in warm if r["res"].get(k) is not None]
        if not vals: continue
        if isinstance(vals[0], dict): rec[k] = {p: round(S.median(v[p] for v in vals), 2) for p in ("p50", "p95", "p99", "mean")}
        else: rec[k] = round(S.median(vals), 2)
    if rs[0]["mode"] == "socket": rec["real_hz"] = [r["info"].get("real_hz") for r in warm]; rec["set_rtt_p50"] = round(S.median(r["info"]["set_rtt_p50"] for r in warm), 2); rec["polls"] = [r["info"]["polls"] for r in warm]
    out.append(rec)
if len(sys.argv) > 2: json.dump(out, open(sys.argv[2], "w"), indent=0)
f = lambda v, p="p50": "-" if not isinstance(v, dict) else f"{v[p]:.1f}"
print("bin var mode hz obs | fps | cycle p50/p95 | draw | eval_call | eval→recv | post→ipc | ipc→on_ipc | on_ipc→eval | gpuiDraw p50/p95 share% renders/s | plan key prev | state→post p50/p95 | state→raf | post→raf | lag | load")
for r in out:
    print(r["binary"][:3], r["variant"][:4], r["mode"][:4], r["hz"], r["obs"], "|", r.get("frames_per_s"), "|", f(r.get("cycle")), f(r.get("cycle"), "p95"), "|", f(r.get("draw")), "|", (r.get("eval_call") or {}).get("p50"), "|", f(r.get("eval_to_recv")), "|", f(r.get("post_to_ipc")), "|", f(r.get("ipc_to_on_ipc")), "|", f(r.get("onipc_to_next_eval")), "|", f(r.get("gpui_draw")), f(r.get("gpui_draw"), "p95"), r["gpui_draw_share_pct"], r["gpui_renders_per_s"], "|", (r.get("plan") or {}).get("p50"), (r.get("key") or {}).get("p50"), (r.get("preview_request") or {}).get("p50"), "|", f(r.get("state_to_post")), f(r.get("state_to_post"), "p95"), "|", f(r.get("state_to_raf")), "|", f(r.get("post_to_raf")), "|", f(r.get("lag_frames")), "|", r["load_max"], "fm", r["foreign_moves"])
