#!/usr/bin/env python3
"""Same-build A/B of the scene warm (BL157) in the real window, plus the pixel proof that it changes no pixel.

    python3 scripts/scene-warm-proof.py --binary target/release/tessel-editor [--pairs 4] [--splats 300000]
                                        [--cuts 5] [--cut-frames 72] [--out report.json] [--skip-pixels] [--keep]

What it does (everything semantic through the window's socket: no coordinates, the user's editor is never addressed):

  * Writes a synthetic project in a temporary folder: `--cuts` scenes, each its own cloud of `--splats` seeded random
    splats, played back to back on one track. Nothing private is read or copied; the folder is removed (`--keep`
    leaves it).
  * Takes both GPU bench locks (`/tmp/tessel-scene-gpu.lock`, `/tmp/tessel-video-bl140-bl144-review-gpu.lock`)
    for the whole run, waiting at most `--lock-wait` seconds, then exits 2 (BLOCKED) instead of waiting forever.
  * TIMING: `2 * --pairs` fresh editor processes of the SAME binary, playing the project once each, half with the warm
    on (the default) and half with `TESSEL_VIEWPORT_WARM=0`, in alternating order (warm,cold / cold,warm / ...), so a
    drift of the machine is spread over both sides. Each run records the host's own clock (`TESSEL_VIEWPORT_TRACE=1`:
    `done_ms` of every accepted picture, collected by polling the ring before it wraps) and the memory of the editor
    plus the WebKit processes that did not exist before it started, once a second.
  * PIXELS: for four cut frames of the same project, one editor per (arm, order) = warm/cold x hot/cold-seek, with
    `TESSEL_VIEWPORT_INSPECT=1`. "Hot" plays into the cut so the warm really ran; "cold" seeks straight to the frame.
    The explicit `render.frame` hash of the same frame must be identical in all four, and different between two
    cuts (so a blank picture or a stuck one cannot pass). After the pause the monitor's own picture must be that hash.
  * Validity: a "warm" run that never warmed (`stats.warmed == 0`) or a "cold" one that did is an INVALID run: the
    report says so and the exit code is 1; nothing is averaged over it.

What it reports, per entry (the first picture of each cut) and per run: the latency of the entry picture
(`submit_to_done_ms`, host clock), the gap to the previous picture, the intervals above 1.5 frames, the playback clock's
skipped frames, the load average and free disk at the start, and the memory MEDIAN and PEAK of each run (a peak
that grows while the median falls is reported as it is). It does not average the pairs into a verdict: the report
carries every value and `analysis` states only what the data can support. `flush_ms`-like numbers are CPU, not GPU;
`submit_to_done_ms` is host admission to acceptance, not photon.

Exit 0: every run valid, every pixel check passed; 1: a failed check or an invalid run; 2: a prerequisite is missing or
the locks / the disk reserve could not be had.
"""
import argparse
import fcntl
import importlib.util
import json
import math
import os
import pathlib
import random
import shutil
import socket
import statistics
import struct
import subprocess
import sys
import tempfile
import threading
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent
FPS = 24
PERIOD_MS = 1000.0 / FPS
# An interval above a frame and a half is a visible hitch at 24 fps; it is the threshold of the 60 ms acceptance.
SLOW_MS = 1.5 * PERIOD_MS
LOCK_PATHS = ("/tmp/tessel-scene-gpu.lock", "/tmp/tessel-video-bl140-bl144-review-gpu.lock")
MIN_FREE_GIB = 16
POLL_SECONDS = 4.0
# Frames after a cut in which its first picture may appear (a cut frame the clock skipped is shown a few later).
ENTRY_WINDOW = 6

checks = []
failures = []


def check(name, condition, detail=""):
    checks.append({"check": name, "ok": bool(condition), "detail": str(detail)[:400]})
    print(("PASS " if condition else "FAIL ") + name + (f"  [{str(detail)[:300]}]" if detail and not condition else ""), flush=True)
    if not condition:
        failures.append(name)
    return bool(condition)


# ------------------------------------------------------------------------------------------------ pure logic
def splat_bytes(count, seed):
    """`count` seeded random splats (the 32-byte .splat layout): a ball of small translucent coloured blobs."""
    rng = random.Random(seed)
    out = bytearray()
    for _ in range(count):
        # Uniform in a ball of radius 1.6, so the sort has real depth to order.
        while True:
            x, y, z = (rng.uniform(-1, 1) for _ in range(3))
            if x * x + y * y + z * z <= 1:
                break
        scale = rng.uniform(0.006, 0.02)
        out += struct.pack("<6f4B4B", x * 1.6, y * 1.6, z * 1.6, scale, scale, scale,
                           rng.randrange(40, 255), rng.randrange(40, 255), rng.randrange(40, 255), rng.randrange(110, 230),
                           255, 128, 128, 128)
    return bytes(out)


def arm_order(pairs):
    """warm,cold / cold,warm / ...: the order alternates so a drift of the machine is spread over both arms."""
    order = []
    for index in range(pairs):
        order += ["warm", "cold"] if index % 2 == 0 else ["cold", "warm"]
    return order


def merge_frames(seen, recent):
    """Adds the frames of one trace window to `seen` (by request id) and returns it; a polled window overlaps the last."""
    for frame in recent or []:
        seen.setdefault(frame["id"], frame)
    return seen


def pictures(frames):
    """Accepted pictures of one run in the order they were accepted (host clock)."""
    return sorted((f for f in frames.values() if f.get("done_ms") is not None and f.get("shown_frame") is not None),
                  key=lambda f: (f["done_ms"], f["id"]))


def entry_report(shots, cut):
    """What the first picture at or after `cut` cost: its own latency and the gap since the picture before it.

    The picture is the first with `shown_frame` in [cut, cut + ENTRY_WINDOW]; none means the entry is `missing`
    (the report never invents one)."""
    for index, shot in enumerate(shots):
        if cut <= shot["shown_frame"] <= cut + ENTRY_WINDOW:
            before = shots[index - 1] if index else None
            return {
                "cut": cut,
                "shown_frame": shot["shown_frame"],
                "latency_ms": shot["submit_to_done_ms"],
                "runtime_total_ms": shot.get("runtime_total_ms"),
                "gap_ms": round(shot["done_ms"] - before["done_ms"], 1) if before else None,
            }
    return {"cut": cut, "missing": True}


def intervals(shots):
    """Gaps between consecutive accepted pictures, in milliseconds."""
    return [round(b["done_ms"] - a["done_ms"], 1) for a, b in zip(shots, shots[1:])]


def run_summary(frames, cuts, memory, played):
    """Everything one run says, with no judgement."""
    shots = pictures(frames)
    gaps = intervals(shots)
    mem = [value for _, value in memory]
    return {
        "pictures": len(shots),
        "entries": [entry_report(shots, cut) for cut in cuts],
        "slow_intervals": sum(1 for gap in gaps if gap > SLOW_MS),
        "worst_interval_ms": max(gaps) if gaps else None,
        "excess_ms": round(sum(gap - PERIOD_MS for gap in gaps if gap > SLOW_MS), 1),
        "skipped_frames": (played or {}).get("skipped_frames"),
        "max_gap_frames": (played or {}).get("max_gap_frames"),
        "memory_mib": {
            "samples": len(mem),
            "median": round(statistics.median(mem), 1) if mem else None,
            "peak": round(max(mem), 1) if mem else None,
        },
    }


def medians(runs, cuts):
    """Per entry, the median and the worst latency over the runs that have it, and how many have it."""
    out = []
    for slot, cut in enumerate(cuts):
        values = [run["summary"]["entries"][slot]["latency_ms"] for run in runs if not run["summary"]["entries"][slot].get("missing")]
        out.append({"cut": cut, "runs": len(values),
                    "median_ms": round(statistics.median(values), 1) if values else None,
                    "max_ms": round(max(values), 1) if values else None})
    return out


def run_is_valid(arm, stats):
    """A warm run must have warmed and a cold one must not: otherwise it measured the other arm."""
    warmed = (stats or {}).get("warmed")
    if not isinstance(warmed, int):
        return False, "the session stats carry no `warmed` counter"
    if arm == "warm":
        return (warmed > 0, f"warmed={warmed}" + ("" if warmed else ": the warm never ran"))
    return (warmed == 0, f"warmed={warmed}" + ("" if not warmed else ": the cold arm warmed"))


def distinct(values):
    return len(set(values)) == len(values)


# --------------------------------------------------------------------------------------------- machine state
def acquire_locks(wait_seconds, paths=LOCK_PATHS, sleep=time.sleep, now=time.monotonic):
    """Both bench locks, in a fixed order, without waiting forever. Returns the open handles or None."""
    deadline = now() + wait_seconds
    handles = []
    for path in paths:
        handle = open(path, "a")
        while True:
            try:
                fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
                handles.append(handle)
                break
            except OSError:
                if now() >= deadline:
                    handle.close()
                    for held in handles:
                        held.close()
                    return None
                sleep(1.0)
    return handles


def free_gib(path="/tmp"):
    usage = shutil.disk_usage(path)
    return usage.free / 1024 ** 3


def webkit_rss():
    """{pid: rss KiB} of every WebKit helper process on the machine."""
    out = subprocess.run(["ps", "-axo", "pid=,rss=,command="], capture_output=True, text=True).stdout
    found = {}
    for line in out.splitlines():
        parts = line.split(None, 2)
        if len(parts) == 3 and "com.apple.WebKit." in parts[2] and parts[0].isdigit() and parts[1].isdigit():
            found[int(parts[0])] = int(parts[1])
    return found


def process_rss(pid):
    out = subprocess.run(["ps", "-o", "rss=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
    return int(out) if out.isdigit() else 0


class MemorySampler:
    """The editor plus the WebKit processes that did not exist before it started, in MiB, once a second.

    The WebKit helpers are not children of the editor, so "new since the baseline" is the attribution: another
    WebKit process started by someone else during a run would be counted (the locks keep the bench alone, the
    user's own browser is not controlled). It is RSS, not the physical footprint."""

    def __init__(self, pid, baseline, every=1.0):
        self.pid, self.baseline, self.every = pid, set(baseline), every
        self.samples, self._stop = [], threading.Event()
        self._thread = threading.Thread(target=self._run, daemon=True)

    def _run(self):
        started = time.monotonic()
        while not self._stop.is_set():
            helpers = {pid: rss for pid, rss in webkit_rss().items() if pid not in self.baseline}
            total = process_rss(self.pid) + sum(helpers.values())
            self.samples.append((round(time.monotonic() - started, 1), round(total / 1024, 1)))
            self._stop.wait(self.every)

    def start(self):
        self._thread.start()
        return self

    def stop(self):
        self._stop.set()
        self._thread.join(5)
        return self.samples


# ----------------------------------------------------------------------------------------------- the editor
class Lab:
    """One isolated editor (own HOME, socket and runtime page)."""

    def __init__(self, binary, work, env):
        self.home = os.path.join(work, "h")
        os.makedirs(self.home)
        self.socket_path = os.path.join(self.home, ".cache/tessel-editor/ui.sock")
        if len(self.socket_path) > 100:
            raise SystemExit("socket path too long for a unix socket; use a shorter TMPDIR")
        self.log_path = os.path.join(self.home, "editor.log")
        self.log = open(self.log_path, "w")
        self.process = subprocess.Popen([binary], env={**os.environ, "HOME": self.home, **env}, stdout=self.log, stderr=self.log,
                                        start_new_session=True)

    def send(self, request, timeout=60):
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

    def op(self, request, timeout=60):
        return self.send({"command": "op", "request": request}, timeout)

    def invoke(self, target):
        return self.send({"command": "invoke", "target": target})

    def seek(self, frame):
        return self.send({"command": "set", "target": "timeline.playhead", "field": "frame", "value": str(frame)})

    def playhead(self):
        return self.send({"command": "get", "target": "timeline.playhead"})["data"]["state"]

    def viewport(self):
        return self.send({"command": "get", "target": "monitor.viewport"})["data"]["state"]

    def report(self):
        """`render.status`: the viewport report with the long trace window."""
        answer = self.op({"op": "render.status"})
        data = answer.get("data") or {}
        return data.get("report", data)

    def wait_ready(self, seconds=120):
        deadline = time.time() + seconds
        while time.time() < deadline:
            if self.process.poll() is not None:
                raise RuntimeError(f"the editor exited ({self.process.returncode}): {pathlib.Path(self.log_path).read_text()[-600:]}")
            if os.path.exists(self.socket_path):
                try:
                    if self.viewport()["phase"] == "ready":
                        return
                except (OSError, ValueError, KeyError, TypeError):
                    pass
            time.sleep(0.25)
        raise RuntimeError("the viewport never became ready")

    def stop(self):
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(10)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(10)
        self.log.close()


def error_code(answer):
    error = (answer or {}).get("error")
    return error.get("code") if isinstance(error, dict) else None


def retry_while_preparing(call, timeout=60.0, sleep=time.sleep, now=time.monotonic):
    """`call()` again while the host answers `media_preparing` (a wait, not a verdict)."""
    deadline = now() + timeout
    while True:
        answer = call()
        if answer.get("ok") or error_code(answer) != "media_preparing" or now() >= deadline:
            return answer
        sleep(0.5)


def wait_job(lab, job, timeout=300.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        info = lab.op({"op": "job.wait", "job": job}).get("data") or {}
        if info.get("state") not in ("queued", "running"):
            return info
        time.sleep(0.25)
    return {}


def build_project(binary, work, splats, cuts, cut_frames):
    """The synthetic project, written by a throwaway editor (not part of any measurement). Returns the document path."""
    folder = os.path.join(work, "project")
    clouds = os.path.join(work, "clouds")
    os.makedirs(clouds)
    paths = []
    for index in range(cuts):
        path = os.path.join(clouds, f"cloud{index}.splat")
        pathlib.Path(path).write_bytes(splat_bytes(splats, 1000 + index))
        paths.append(path)
    lab = Lab(binary, os.path.join(work, "setup"), {})
    try:
        lab.wait_ready()
        total = cuts * cut_frames
        created = lab.op({"op": "project.create", "name": "Scene warm proof", "sequence": "main", "rate": FPS, "width": 960,
                          "height": 540, "length": total, "dir": folder})
        if not check("the proof project is created", created.get("ok"), created):
            return None
        imported = lab.op({"op": "media.import", "paths": paths, "mode": "copy"})
        if not check("the clouds import", imported.get("ok"), imported):
            return None
        finished = wait_job(lab, imported["data"]["job"])
        if not check("the import job applied", finished.get("state") == "succeeded" and finished.get("result_state") == "applied", finished):
            return None
        # `media.import` names an asset after the file stem (`cloud0.splat` -> `cloud0`).
        status = lab.op({"op": "media.status"}).get("data") or {}
        kinds = {asset["id"]: asset.get("kind") for asset in status.get("assets", [])}
        names = [f"cloud{index}" for index in range(cuts)]
        if not check("every cloud is a splat asset", all(kinds.get(name) == "splat" for name in names), kinds):
            return None
        steps = [{"op": "track.add", "id": "scenes", "kind": "video", "name": "Scenes"}]
        for index in range(cuts):
            scene = os.path.join(folder, f"scene{index}.tessel-scene.json")
            pathlib.Path(scene).write_text(json.dumps({
                "format": "tessel-scene", "version": 1, "background": [0.02 + 0.02 * index, 0.04, 0.09, 1.0],
                "objects": [{"type": "splat", "asset": f"cloud{index}", "scale": 1.0, "spin": [0, 12 * (index + 1), 0]}]}))
            steps.append({"op": "asset.add", "id": f"scn{index}", "kind": "scene", "name": f"Scene {index}", "path": scene})
        for index in range(cuts):
            steps.append({"op": "clip.add", "id": f"c{index}", "asset": f"scn{index}", "track": "scenes", "start": index * cut_frames,
                          "duration": cut_frames, "camera": {"fov": 50, "orbit": 30 * index, "position": [0, 1.0, 4.2], "target": [0, 0, 0]}})
        document = os.path.join(folder, "warm.tessel.json")
        steps.append({"op": "project.save", "path": document})
        for step in steps:
            reply = lab.op(step)
            if not check(f"setup {step['op']} {step.get('id', '')}", reply.get("ok"), reply):
                return None
        return document
    finally:
        lab.stop()


def private_copy(document, destination):
    """A private copy of the saved project: the document is copied, every other entry is a symlink (read only for
    the run), except what the editor owns (`.tessel`)."""
    source = os.path.abspath(document)
    folder = os.path.dirname(source)
    os.makedirs(destination)
    copied = os.path.join(destination, os.path.basename(source))
    shutil.copyfile(source, copied)
    for entry in sorted(os.listdir(folder)):
        if entry in (os.path.basename(source), ".tessel"):
            continue
        os.symlink(os.path.join(folder, entry), os.path.join(destination, entry))
    return copied


def env_for(arm, inspect):
    env = {"TESSEL_VIEWPORT_TRACE": "1"}
    if arm == "cold":
        env["TESSEL_VIEWPORT_WARM"] = "0"
    if inspect:
        env["TESSEL_VIEWPORT_INSPECT"] = "1"
    return env


def open_project(lab, document, work):
    lab.invoke("window.activate")
    time.sleep(1.0)
    copy = private_copy(document, os.path.join(work, "copy"))
    opened = lab.op({"op": "project.open", "path": copy})
    if not opened.get("ok"):
        raise RuntimeError(f"project.open: {opened}")
    return copy


# ------------------------------------------------------------------------------------------------- timing
def timing_run(binary, document, arm, cuts, work, total_frames):
    """One fresh editor plays the project once. Returns the run record (summary + raw per-run facts)."""
    load = os.getloadavg()
    run = {"arm": arm, "loadavg_start": [round(v, 2) for v in load], "free_gib_start": round(free_gib(), 1)}
    baseline = webkit_rss().keys()
    lab = Lab(binary, work, env_for(arm, False))
    sampler = None
    try:
        lab.wait_ready()
        open_project(lab, document, work)
        sampler = MemorySampler(lab.process.pid, baseline).start()
        time.sleep(1.5)
        started = retry_while_preparing(lambda: lab.op({"op": "playback.start", "from": 0}))
        if not started.get("ok"):
            run["error"] = f"playback.start: {started}"
            return run
        frames = {}
        deadline = time.time() + total_frames / FPS + 30
        while time.time() < deadline:
            time.sleep(POLL_SECONDS)
            merge_frames(frames, ((lab.report().get("session") or {}).get("trace") or {}).get("recent"))
            status = (lab.op({"op": "playback.status"}).get("data") or {})
            if status.get("state") != "playing":
                break
        stopped = lab.op({"op": "playback.stop"})
        report = (stopped.get("data") or {}).get("report") or {}
        played = report if "ticks" in report else (report.get("last") or {})
        final = lab.report()
        merge_frames(frames, ((final.get("session") or {}).get("trace") or {}).get("recent"))
        stats = (final.get("session") or {}).get("stats") or {}
        run["stats"] = {key: stats.get(key) for key in ("requests", "accepted", "warmed", "reloaded", "prefetched", "evicted", "cancelled",
                                                      "dropped_stale")}
        run["error"] = (final.get("session") or {}).get("error")
        run["played"] = {key: played.get(key) for key in ("ticks", "skipped_frames", "max_gap_frames")}
        run["memory_series"] = sampler.stop()
        sampler = None
        run["summary"] = run_summary(frames, cuts, run["memory_series"], played)
        run["valid"], run["valid_why"] = run_is_valid(arm, stats)
        return run
    finally:
        if sampler:
            sampler.stop()
        lab.stop()


# ------------------------------------------------------------------------------------------------- pixels
def pixel_session(binary, document, arm, order, frames, cut_frames, work):
    """One editor, one (arm, order): the explicit hashes of `frames`, and whether the arm really was what it says."""
    seek_proof = load_module("seek_scrub_proof", ROOT / "scripts" / "seek-scrub-proof.py")
    lab = Lab(binary, work, env_for(arm, True))
    result = {"arm": arm, "order": order, "hashes": {}, "monitor": {}}
    try:
        lab.wait_ready()
        open_project(lab, document, work)
        for frame in frames:
            if order == "hot":
                # The scene before the cut plays into it: the prefetch loads, the warm sorts, the clock crosses the cut.
                begin = max(0, frame - cut_frames)
                started = retry_while_preparing(lambda: lab.op({"op": "playback.start", "from": begin}))
                if not started.get("ok"):
                    result["error"] = f"playback.start: {started}"
                    return result
                deadline = time.time() + cut_frames / FPS + 40
                while time.time() < deadline and (lab.playhead().get("frame") or 0) < frame:
                    time.sleep(0.1)
                lab.op({"op": "playback.stop"})
            lab.seek(frame)
            deadline = time.time() + 60
            state = lab.viewport()
            while time.time() < deadline and not seek_proof.converged(state, frame):
                time.sleep(0.2)
                state = lab.viewport()
            result["monitor"][frame] = {"converged": seek_proof.converged(state, frame),
                                        "hash": (state.get("last_frame") or {}).get("hash")}
            size = state.get("output") or {}
            explicit = retry_while_preparing(lambda: lab.op({"op": "render.frame", "frame": frame, "width": size.get("width"),
                                                              "height": size.get("height"), "probes": []}, 120))
            data = ((explicit.get("data") or {}).get("frame")) or {}
            result["hashes"][frame] = {"hash": data.get("hash"), "inspected": data.get("inspected"), "mean": data.get("mean")}
        stats = (lab.report().get("session") or {}).get("stats") or {}
        result["warmed"] = stats.get("warmed")
        return result
    finally:
        lab.stop()


def pixel_verdicts(sessions, frames):
    """The checks over the four sessions: identical per frame across them, different between frames, none blank."""
    out = []
    for frame in frames:
        values = [s["hashes"].get(frame, {}).get("hash") for s in sessions]
        out.append((f"frame {frame}: the explicit render is byte-identical in warm/cold x hot/cold-seek",
                    all(values) and len(set(values)) == 1, values))
    per_frame = [sessions[0]["hashes"].get(frame, {}).get("hash") for frame in frames] if sessions else []
    out.append(("control: the frames of two different cuts do not share a picture", bool(per_frame) and distinct(per_frame), per_frame))
    out.append(("control: every picture was read back (inspected)", all(h.get("inspected") is True for s in sessions for h in s["hashes"].values()), ""))
    out.append(("after the pause the monitor shows the explicit render's picture",
                all(s["monitor"].get(frame, {}).get("hash") == s["hashes"].get(frame, {}).get("hash") and s["monitor"].get(frame, {}).get("converged")
                    for s in sessions for frame in frames), [s["monitor"] for s in sessions]))
    return out


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def analysis(runs, cuts):
    """Only what the data supports: the per-entry medians of both arms, the memory of both arms and the open caveats."""
    arms = {}
    for arm in ("warm", "cold"):
        mine = [run for run in runs if run["arm"] == arm and run.get("valid") and run.get("summary")]
        peaks = [run["summary"]["memory_mib"]["peak"] for run in mine if run["summary"]["memory_mib"]["peak"] is not None]
        meds = [run["summary"]["memory_mib"]["median"] for run in mine if run["summary"]["memory_mib"]["median"] is not None]
        arms[arm] = {
            "valid_runs": len(mine),
            "entries": medians(mine, cuts) if mine else [],
            "memory_median_of_medians_mib": round(statistics.median(meds), 1) if meds else None,
            "memory_median_of_peaks_mib": round(statistics.median(peaks), 1) if peaks else None,
            "memory_max_peak_mib": round(max(peaks), 1) if peaks else None,
            "slow_intervals_per_run": [run["summary"]["slow_intervals"] for run in mine],
            "worst_interval_ms_per_run": [run["summary"]["worst_interval_ms"] for run in mine],
        }
    return {
        "arms": arms,
        "caveats": [
            "RSS of the editor plus the WebKit processes new since the baseline; not the physical footprint, not GPU memory",
            "latency is host admission to acceptance (submit_to_done_ms), CPU-side readback included; it is not photon latency",
            "the pair count bounds what an A/B can say: report the per-run values, do not read a mean as an acceptance",
            "CPU/GPU interference with other work on the machine is recorded (load average) but not controlled",
        ],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--binary", default=str(ROOT / "target" / "release" / "tessel-editor"))
    parser.add_argument("--pairs", type=int, default=4)
    parser.add_argument("--splats", type=int, default=300000)
    parser.add_argument("--cuts", type=int, default=5)
    parser.add_argument("--cut-frames", type=int, default=72)
    parser.add_argument("--lock-wait", type=float, default=600.0)
    parser.add_argument("--out")
    parser.add_argument("--skip-pixels", action="store_true")
    parser.add_argument("--skip-timing", action="store_true")
    parser.add_argument("--keep", action="store_true")
    args = parser.parse_args()
    binary = pathlib.Path(args.binary).resolve()
    for needed in (binary, ROOT / "runtime" / "dist" / "runtime.js"):
        if not needed.exists():
            print(f"missing prerequisite: {needed}")
            return 2
    if args.cuts < 3 or args.pairs < 1 or args.cut_frames < 48:
        print("--cuts >= 3, --pairs >= 1 and --cut-frames >= 48 are needed for the entries to be cuts")
        return 2
    if free_gib() < MIN_FREE_GIB:
        print(f"only {free_gib():.1f} GiB free; this proof needs {MIN_FREE_GIB}")
        return 2
    locks = acquire_locks(args.lock_wait)
    if locks is None:
        print(f"BLOCKED: the GPU bench locks were not free within {args.lock_wait:.0f} s")
        return 2
    work = tempfile.mkdtemp(prefix="tw-", dir="/tmp")
    sha = subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout.strip()
    report = {"git": sha, "binary": str(binary), "args": vars(args), "runs": [], "pixels": [], "checks": checks}
    try:
        digest = subprocess.run(["shasum", "-a", "256", str(binary)], capture_output=True, text=True).stdout.split()[0]
        report["binary_sha256"] = digest
        print(f"build {sha[:9]} binary sha256 {digest[:12]}", flush=True)
        cuts = [index * args.cut_frames for index in range(args.cuts)]
        document = build_project(str(binary), os.path.join(work, "build"), args.splats, args.cuts, args.cut_frames)
        if not document:
            return 1
        if not args.skip_timing:
            for number, arm in enumerate(arm_order(args.pairs)):
                run = timing_run(str(binary), document, arm, cuts, os.path.join(work, f"run{number}"), args.cuts * args.cut_frames)
                run["number"] = number
                report["runs"].append(run)
                check(f"run {number} ({arm}) is valid", run.get("valid") and not run.get("error"),
                      run.get("error") or run.get("valid_why"))
            report["analysis"] = analysis(report["runs"], cuts)
        if not args.skip_pixels:
            frames = [cuts[1] + 5, cuts[2] + 5, cuts[-1] + 5, cuts[1] + 40]
            sessions = []
            for number, (arm, order) in enumerate([("warm", "hot"), ("cold", "hot"), ("warm", "cold"), ("cold", "cold")]):
                session = pixel_session(str(binary), document, arm, order, frames, args.cut_frames, os.path.join(work, f"px{number}"))
                sessions.append(session)
                report["pixels"].append(session)
                check(f"pixel session {arm}/{order} ran", not session.get("error"), session.get("error"))
            hot = {s["arm"]: s for s in sessions if s["order"] == "hot"}
            check("the warm arm warmed in the hot pass", hot.get("warm", {}).get("warmed", 0) > 0, hot.get("warm", {}).get("warmed"))
            check("the cold arm never warmed in the hot pass", hot.get("cold", {}).get("warmed") == 0, hot.get("cold", {}).get("warmed"))
            for name, ok, detail in pixel_verdicts(sessions, frames):
                check(name, ok, detail)
    finally:
        for handle in locks:
            handle.close()
        if args.keep:
            print(f"kept {work}")
        else:
            shutil.rmtree(work, ignore_errors=True)
    if args.out:
        pathlib.Path(args.out).write_text(json.dumps(report, indent=2))
    print(json.dumps(report.get("analysis", {}), indent=2))
    print(f"{'ALL' if not failures else 'SOME'} checks done; failed: {failures}")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
