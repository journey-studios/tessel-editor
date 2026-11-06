#!/usr/bin/env python3
"""BL129 proof: MSAA by motion. Playback and scrub draw scene layers with fewer samples; the picture
is refined to the final count by itself when the playhead stops; explicit frames never change.

    python3 scripts/preview-quality-proof.py --binary target/release/tessel-editor \
        --project /tmp/copy/project.tessel.json [--runtime-dir DIR] [--env K=V ...] [--label L]
        [--playback 40:60,181:30] [--scrub take:20:100:60,xfade:150:100:60] [--reps 3]
        [--hashes 5,60,190] [--inspect] [--png-dir DIR --png-frames 60,193] [--json FILE]

Starts the DIRECT binary with an isolated HOME (own socket, own settings; another editor is never
touched) and drives it with semantic requests only (`op`, `get`, `invoke`; no pointer, no coordinates).
`--project` should be a COPY of the project: the editor may write beside it. `--runtime-dir` is a
directory with `dist/` (default: the repository's `runtime/`), so a base build and a new one can be
compared with their own runtimes.

What it measures (the editor's own clock: `TESSEL_VIEWPORT_TRACE=1` stamps every accepted frame):

* `playback`: for each `START:FRAMES` the clock frames the playhead crossed against the distinct
  pictures the monitor accepted, per region (`single` / `xfade`), the spacing of the pictures (p50/p95/p99)
  and the runtime's draw time. `dropped_pct` is the share of clock frames that never got a picture.
* `converge`: after `playback.stop`, the time until the picture on screen is final quality again
  (`session.refine_pending` false, `last_frame.source.reduced_scene_layers` 0, no pending request).
* `scrub`: `seek-scrub-proof.py` per `NAME:ORIGIN:SPAN:HZ` (pictures per 2 s, gaps, lag, release to final).
* `--hashes`: `render.frame` hashes of explicit frames (needs `--inspect`, i.e. TESSEL_VIEWPORT_INSPECT=1),
  to compare a base build with the new one; with `--inspect` the refined picture after a stop is also
  compared with the explicit render of the same frame.

Prints one JSON report on the last line. GPU utilisation (ioreg) and the load average are recorded per
run: the GPU may be shared, so repeat the runs and read the spread, not one number.
"""
import argparse
import json
import os
import re
import shutil
import socket
import statistics
import subprocess
import sys
import tempfile
import threading
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FPS = 24.0


def pct(values, share):
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(round(share * (len(ordered) - 1))))]


def summary(values, digits=2):
    values = [v for v in values if v is not None]
    if not values:
        return {"n": 0}
    out = {"n": len(values), "mean": round(statistics.fmean(values), digits), "p50": round(pct(values, 0.5), digits),
           "p95": round(pct(values, 0.95), digits), "max": round(max(values), digits)}
    if len(values) >= 20:
        out["p99"] = round(pct(values, 0.99), digits)
    return out


def gpu_util():
    out = subprocess.run(["ioreg", "-r", "-d", "1", "-w0", "-c", "IOAccelerator"], capture_output=True, text=True).stdout
    match = re.search(r'"Device Utilization %"=(\d+)', out)
    return int(match.group(1)) if match else None


class GpuSampler:
    def __init__(self, period=0.4):
        self.period, self.values, self.stop_event = period, [], threading.Event()
        self.thread = threading.Thread(target=self.loop, daemon=True)

    def loop(self):
        while not self.stop_event.is_set():
            value = gpu_util()
            if value is not None:
                self.values.append(value)
            self.stop_event.wait(self.period)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_):
        self.stop_event.set()
        self.thread.join(2)

    def report(self):
        return {"n": len(self.values), "mean": round(statistics.fmean(self.values), 1) if self.values else None,
                "max": max(self.values) if self.values else None}


def wait_quiet_gpu(limit=22, timeout=60):
    """The GPU may be shared with other work: start a run when two samples in a row are quiet."""
    deadline = time.time() + timeout
    quiet = 0
    while time.time() < deadline:
        value = gpu_util()
        quiet = quiet + 1 if value is not None and value <= limit else 0
        if quiet >= 2:
            return True
        time.sleep(0.6)
    return False


class Editor:
    def __init__(self, binary, runtime_dir, env):
        self.home = tempfile.mkdtemp(prefix="tpq")
        self.socket_path = os.path.join(self.home, ".cache/tessel-editor/ui.sock")
        environment = {**os.environ, "HOME": self.home, "TESSEL_VIEWPORT_TRACE": "1", **env}
        if runtime_dir:
            environment["TESSEL_RUNTIME_DIR"] = runtime_dir
        self.log_path = os.path.join(self.home, "editor.log")
        self.log = open(self.log_path, "w")
        self.process = subprocess.Popen([binary], env=environment, stdout=self.log, stderr=self.log, start_new_session=True)

    def send(self, request, timeout=300):
        with socket.socket(socket.AF_UNIX) as conn:
            conn.settimeout(timeout)
            conn.connect(self.socket_path)
            conn.sendall((json.dumps({"version": 1, **request}) + "\n").encode())
            data = b""
            while not data.endswith(b"\n"):
                chunk = conn.recv(1 << 20)
                if not chunk:
                    break
                data += chunk
        return json.loads(data)

    def op(self, request, timeout=300):
        return self.send({"command": "op", "request": request}, timeout)

    def get(self, target):
        return self.send({"command": "get", "target": target})

    def viewport(self):
        return self.get("monitor.viewport")["data"]["state"]

    def render_status(self):
        data = self.op({"op": "render.status"})["data"]
        return data.get("report", data)

    def wait_ready(self, seconds=90):
        deadline = time.time() + seconds
        while time.time() < deadline:
            if self.process.poll() is not None:
                with open(self.log_path) as handle:
                    raise RuntimeError(f"the editor exited ({self.process.returncode}): {handle.read()[-600:]}")
            if os.path.exists(self.socket_path):
                try:
                    if self.viewport()["phase"] == "ready":
                        return
                except Exception:
                    pass
            time.sleep(0.2)
        raise RuntimeError("the viewport never became ready")

    def explicit(self, frame, timeout=600):
        """`render.frame` until the media is decoded and loaded; returns the reply."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            reply = self.op({"op": "render.frame", "frame": frame})
            if reply.get("ok"):
                return reply
            if (reply.get("error") or {}).get("code") not in ("media_preparing", "cancelled"):
                return reply
            time.sleep(0.3)
        raise RuntimeError(f"render.frame {frame} never completed")

    def explicit_png(self, frame, path):
        """An explicit `render.frame` written as a PNG; returns the pixel hash it reports."""
        deadline = time.time() + 600
        while time.time() < deadline:
            reply = self.op({"op": "render.frame", "frame": frame, "out": path})
            if reply.get("ok"):
                return ((reply.get("data") or {}).get("frame") or {}).get("hash")
            if (reply.get("error") or {}).get("code") not in ("media_preparing", "cancelled"):
                raise RuntimeError(f"render.frame {frame} -> {path}: {reply}")
            time.sleep(0.3)
        raise RuntimeError(f"render.frame {frame} never completed")

    def settled(self):
        state = self.viewport()
        session, last = state["session"], state.get("last_frame")
        return (state["phase"] == "ready" and session["status"] == "ready" and not session.get("pending")
                and last is not None and last["revision"] == session["revision"])

    def wait_settled(self, timeout=60):
        deadline = time.time() + timeout
        while time.time() < deadline:
            if self.settled():
                return True
            time.sleep(0.05)
        return False

    def stop(self):
        try:
            os.killpg(self.process.pid, 15)
            self.process.wait(10)
        except Exception:
            try:
                os.killpg(self.process.pid, 9)
            except Exception:
                pass
        self.log.close()
        shutil.rmtree(self.home, ignore_errors=True)


def final_quality(state):
    """The picture on screen is the final one: nothing to refine and no scene below the final samples."""
    session, last = state["session"], state.get("last_frame") or {}
    source = last.get("source") or {}
    return (state["phase"] == "ready" and session["status"] == "ready" and not session.get("pending")
            and not session.get("refine_pending") and last.get("revision") == session["revision"]
            and not source.get("reduced_scene_layers") and not source.get("proxy_layers"))


def wait_final_quality(editor, timeout=30):
    started = time.perf_counter()
    while time.perf_counter() - started < timeout:
        state = editor.viewport()
        if final_quality(state):
            return round((time.perf_counter() - started) * 1000), state
        time.sleep(0.02)
    return None, editor.viewport()


def regions_of(xfades):
    def region(frame):
        return "xfade" if any(low <= frame < high for low, high in xfades) else "single"
    return region


def playback_run(editor, start, frames, xfades, inspect):
    region = regions_of(xfades)
    editor.op({"op": "seek", "frame": start})
    editor.wait_settled()
    wait_final_quality(editor)
    time.sleep(0.3)
    quiet = wait_quiet_gpu()
    base = editor.render_status()
    base_requests = base["session"]["stats"]["requests"]
    with GpuSampler() as gpu:
        started = editor.op({"op": "playback.start", "from": start})
        if not started.get("ok"):
            return {"error": started}
        deadline = time.time() + 20
        while time.time() < deadline:
            if editor.op({"op": "playback.status"})["data"].get("state") == "playing":
                break
            time.sleep(0.1)
        time.sleep(frames / FPS + 0.3)
        stopped = editor.op({"op": "playback.stop"})
        stop_at = time.perf_counter()
    report = (stopped.get("data") or {}).get("report") or {}
    end = report.get("frame", start)
    converge_ms, final_state = wait_final_quality(editor)
    status = editor.render_status()
    traced = [f for f in status["session"]["trace"]["recent"] if f["id"] > base_requests]
    ordered = sorted((f for f in traced if f.get("interactive") and f.get("shown_frame") is not None), key=lambda f: f["done_ms"])
    # The picture of each position once: the refine after the stop redraws the last one, and is not playback.
    interactive, seen = [], set()
    for f in ordered:
        if f["shown_frame"] not in seen:
            seen.add(f["shown_frame"])
            interactive.append(f)
    result = {"start": start, "frames_asked": frames, "end_frame": end, "gpu": gpu.report(), "gpu_quiet_before": quiet,
              "load_avg": round(os.getloadavg()[0], 1), "converge_ms": converge_ms, "regions": {},
              "refined_frames": status["session"]["stats"]["refined"] - base["session"]["stats"]["refined"],
              "playback": {k: report.get(k) for k in ("elapsed_ms", "frame", "lag_frames", "skipped_frames")}}
    result["final_source"] = (final_state.get("last_frame") or {}).get("source")
    result["pictures"] = [[f["shown_frame"], f["done_ms"], f["runtime_render_ms"]] for f in interactive]
    for name in ("single", "xfade"):
        items = [f for f in interactive if start < f["shown_frame"] <= end and region(f["shown_frame"]) == name]
        clock = sum(1 for frame in range(start + 1, end + 1) if region(frame) == name)
        if not items or not clock:
            continue
        shown = len({f["shown_frame"] for f in items})
        # A gap only counts inside one region: the jump over a crossfade is not a stall.
        gaps = [b["done_ms"] - a["done_ms"] for a, b in zip(items, items[1:])
                if all(region(frame) == name for frame in range(a["shown_frame"], b["shown_frame"] + 1))]
        result["regions"][name] = {"clock_frames": clock, "shown_frames": shown,
                                   "dropped_pct": round(100 * (1 - shown / clock), 1),
                                   "interval_ms": summary(gaps), "runtime_render_ms": summary([f["runtime_render_ms"] for f in items])}
    if inspect and converge_ms is not None:
        last = final_state.get("last_frame") or {}
        presented = (final_state["session"].get("presented") or {}).get("frame")
        if last.get("hash") and presented is not None:
            size = final_state.get("output") or {}
            explicit = editor.op({"op": "render.frame", "frame": presented, "width": size.get("width"),
                                  "height": size.get("height"), "probes": []})
            frame = (explicit.get("data") or {}).get("frame") or {}
            result["refined_equals_explicit"] = {"frame": presented, "on_screen": last["hash"], "explicit": frame.get("hash"),
                                                 "equal": last["hash"] == frame.get("hash")}
    return result


def scrub_run(editor, name, origin, span, hz, label):
    editor.op({"op": "seek", "frame": int(origin)})
    editor.wait_settled()
    wait_final_quality(editor)
    time.sleep(0.3)
    quiet = wait_quiet_gpu()
    out = os.path.join(editor.home, f"scrub-{name}.json")
    with GpuSampler() as gpu:
        proc = subprocess.run([sys.executable, os.path.join(ROOT, "scripts/seek-scrub-proof.py"), "--socket", editor.socket_path,
                               "--seconds", "2", "--hz", str(hz), "--span", str(span), "--json", out, "--settle-timeout", "30"],
                              capture_output=True, text=True)
    try:
        with open(out) as handle:
            report = json.load(handle)
    except Exception:
        return {"name": name, "error": (proc.stdout + proc.stderr)[-400:]}
    return {"name": name, "hz": hz, "span": span, "gpu": gpu.report(), "gpu_quiet_before": quiet,
            "load_avg": round(os.getloadavg()[0], 1),
            "presentations_in_window": report.get("presentations_in_window"),
            "pictures_per_s": round(report["presentations_in_window"] / 2.0, 1) if report.get("presentations_in_window") is not None else None,
            "present_gap_ms": report.get("present_gap_ms"), "lag_frames": report.get("lag_frames"),
            "release_to_final_ms": report.get("release_to_final_ms"), "first_present_ms": report.get("first_present_ms"),
            "passed": proc.returncode == 0}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--binary", default=os.path.join(ROOT, "target/release/tessel-editor"))
    parser.add_argument("--runtime-dir")
    parser.add_argument("--project", required=True)
    parser.add_argument("--label", default="run")
    parser.add_argument("--env", action="append", default=[], help="KEY=VALUE for the editor (repeatable)")
    parser.add_argument("--playback", default="40:60,181:30")
    parser.add_argument("--xfades", default="186:200,268:282,410:424", help="frame ranges that draw two scenes")
    parser.add_argument("--scrub", default="take:20:100:60,xfade:150:100:60")
    parser.add_argument("--reps", type=int, default=3)
    parser.add_argument("--warm", default="5,190,300,450", help="frames drawn first so every asset is loaded")
    parser.add_argument("--hashes", default="", help="explicit frames whose hash is reported (needs --inspect)")
    parser.add_argument("--png-dir", help="write explicit render.frame PNGs of --png-frames here (no playback/scrub when --playback/--scrub are empty)")
    parser.add_argument("--png-frames", default="")
    parser.add_argument("--inspect", action="store_true", help="TESSEL_VIEWPORT_INSPECT=1 (changes timing: not for measurements)")
    parser.add_argument("--json", help="also write the report here")
    args = parser.parse_args()

    env = dict(item.split("=", 1) for item in args.env)
    if args.inspect:
        env["TESSEL_VIEWPORT_INSPECT"] = "1"
    xfades = [tuple(int(v) for v in item.split(":")) for item in args.xfades.split(",") if item]
    editor = Editor(args.binary, args.runtime_dir, env)
    report = {"label": args.label, "binary": args.binary, "env": env, "playback": [], "scrub": [], "hashes": {}}
    try:
        editor.wait_ready()
        editor.send({"command": "invoke", "target": "window.activate"})
        opened = editor.op({"op": "project.open", "path": args.project})
        if not opened.get("ok"):
            raise RuntimeError(f"project.open: {opened}")
        time.sleep(0.8)
        for frame in [int(v) for v in args.warm.split(",") if v]:
            reply = editor.explicit(frame)
            if not reply.get("ok"):
                raise RuntimeError(f"warm-up render.frame {frame}: {reply}")
            editor.wait_settled()
        report["renderer"] = (editor.viewport().get("capabilities") or {}).get("renderer")
        for frame in [int(v) for v in args.hashes.split(",") if v]:
            frame_report = ((editor.explicit(frame).get("data") or {}).get("frame")) or {}
            report["hashes"][str(frame)] = frame_report.get("hash")
        if args.png_dir:
            os.makedirs(args.png_dir, exist_ok=True)
            for frame in [int(v) for v in args.png_frames.split(",") if v]:
                path = os.path.join(args.png_dir, f"{args.label}-f{frame:04d}.png")
                reply = editor.explicit_png(frame, path)
                report.setdefault("png", {})[str(frame)] = {"path": path, "hash": reply}
        for spec in [s for s in args.playback.split(",") if s]:
            start, frames = (int(v) for v in spec.split(":"))
            for rep in range(args.reps):
                run = playback_run(editor, start, frames, xfades, args.inspect)
                run.update({"spec": spec, "rep": rep})
                report["playback"].append(run)
                line = " | ".join(f"{name}: shown {r['shown_frames']}/{r['clock_frames']} dropped {r['dropped_pct']}% "
                                  f"gap p50/p95 {r['interval_ms'].get('p50')}/{r['interval_ms'].get('p95')} draw p50 {r['runtime_render_ms'].get('p50')}"
                                  for name, r in run.get("regions", {}).items())
                print(f"{args.label} playback {spec}#{rep} gpu {run.get('gpu', {}).get('mean')} converge {run.get('converge_ms')} ms | {line}", flush=True)
        for spec in [s for s in args.scrub.split(",") if s]:
            name, origin, span, hz = spec.split(":")
            for rep in range(args.reps):
                run = scrub_run(editor, name, origin, span, hz, args.label)
                run["rep"] = rep
                report["scrub"].append(run)
                print(f"{args.label} scrub {name}#{rep} pictures/s {run.get('pictures_per_s')} gap {run.get('present_gap_ms')} lag {run.get('lag_frames')} "
                      f"final {run.get('release_to_final_ms')} gpu {run.get('gpu', {}).get('mean')}", flush=True)
    finally:
        editor.stop()
    if args.json:
        with open(args.json, "w") as handle:
            json.dump(report, handle, indent=1)
    print(json.dumps(report))


if __name__ == "__main__":
    main()
