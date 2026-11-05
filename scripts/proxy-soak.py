#!/usr/bin/env python3
"""Budgeted long run (OPT09) of scrubbing a 1080p video with the automatic proxy, in the real window.

    python3 scripts/proxy-soak.py [--seconds 480] [--out FILE] [--binary PATH]

Starts the DIRECT release binary with an isolated HOME (like proxy-live-proof.py), then repeats
2 s drags at 60 Hz (`seek-scrub-proof.py`) and a policy flip (original <-> auto) for --seconds,
sampling the editor's resident memory (`ps`) once per drag. It reports the RSS curve, how many
drags converged on the original, and the session counters. The point is a leak/instability check
for the proxy + refine + selective-repaint paths over minutes; it is a sample of the stated
duration, never a claim about hours. The per-drag poll is never FPS.
"""
import argparse
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)


def load(file):
    spec = importlib.util.spec_from_file_location(file.replace("-", "_"), os.path.join(HERE, file))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


proof = load("proxy-live-proof.py")


def rss_mb(pid):
    out = subprocess.run(["ps", "-o", "rss=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
    return round(int(out) / 1024, 1) if out else None


def children_rss_mb(pid):
    """The WebView's helper processes are children of the editor (WebKit's GPU/WebContent)."""
    out = subprocess.run(["ps", "-A", "-o", "pid=,ppid=,rss="], capture_output=True, text=True).stdout
    total = 0
    for line in out.splitlines():
        parts = line.split()
        if len(parts) == 3 and parts[1] == str(pid):
            total += int(parts[2])
    return round(total / 1024, 1)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--seconds", type=int, default=480)
    parser.add_argument("--out")
    parser.add_argument("--binary", default=os.path.join(ROOT, "target/release/tessel-editor"))
    args = parser.parse_args()
    work = tempfile.mkdtemp(prefix="tsk-", dir="/tmp")
    clip = os.path.join(work, "c.mp4")
    if not proof.video.clip(clip, 1920, 1080, 240, proof.video.H264):
        print("fixture failed")
        return 2
    variant = proof.Variant(args.binary, work, "soak", "auto", {})
    samples, converged, drags = [], 0, 0
    try:
        variant.start()
        failed = proof.build_project(variant, clip, 240)
        if failed:
            print("project failed", failed)
            return 2
        proof.wait_for(lambda: variant.op({"op": "render.frame", "frame": 0}).get("ok"), 180)
        proof.wait_for(lambda: proof.asset_reason(proof.policy_report(variant)) == "proxy_ready", 180)
        variant.editor.send({"version": 1, "command": "invoke", "target": "window.activate"})
        time.sleep(1)
        pid = variant.editor.process.pid
        started = time.time()
        while time.time() - started < args.seconds:
            mode = "original" if drags % 5 == 4 else "auto"  # every fifth drag runs without proxies
            variant.op({"op": "preview.policy", "mode": mode})
            path = os.path.join(variant.home, "drag.json")
            subprocess.run([sys.executable, os.path.join(HERE, "seek-scrub-proof.py"), "--socket", variant.editor.socket_path,
                            "--seconds", "2", "--hz", "60", "--span", "200", "--json", path], capture_output=True, text=True)
            drags += 1
            try:
                with open(path) as handle:
                    result = json.load(handle)
                converged += 1 if result.get("converged") else 0
            except (OSError, ValueError):
                result = {}
            samples.append({"t_s": round(time.time() - started), "mode": mode, "editor_rss_mb": rss_mb(pid),
                            "children_rss_mb": children_rss_mb(pid), "converged": bool(result.get("converged")),
                            "presentations": result.get("presentations_in_window")})
            time.sleep(1.0)
        variant.op({"op": "preview.policy", "mode": "auto"})
        time.sleep(1.0)
        status = variant.op({"op": "render.status"}).get("data") or {}
        summary = {
            "seconds": args.seconds, "drags": drags, "converged": converged,
            "first_rss_mb": samples[0]["editor_rss_mb"] if samples else None,
            "last_rss_mb": samples[-1]["editor_rss_mb"] if samples else None,
            "peak_rss_mb": max((s["editor_rss_mb"] or 0) for s in samples) if samples else None,
            "first_children_rss_mb": samples[0]["children_rss_mb"] if samples else None,
            "last_children_rss_mb": samples[-1]["children_rss_mb"] if samples else None,
            "peak_children_rss_mb": max(s["children_rss_mb"] for s in samples) if samples else None,
            "session_stats": (status.get("session") or {}).get("stats"),
            "ui": status.get("ui"),
            "phase": status.get("phase"),
            "samples": samples,
        }
    finally:
        variant.stop()
        shutil.rmtree(work, ignore_errors=True)
    if args.out:
        with open(args.out, "w") as handle:
            json.dump(summary, handle, indent=1)
    print(json.dumps({k: v for k, v in summary.items() if k != "samples"}))
    return 0 if converged == drags and summary["phase"] == "ready" else 1


if __name__ == "__main__":
    sys.exit(main())
