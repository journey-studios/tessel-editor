#!/usr/bin/env python3
"""Measures the render path: preview-vs-export difference and the memory of Node + Chrome (+ GPU).

    ./scripts/runtime setup                       # once
    ./scripts/cargo build -p tessel-editor-render --bin tessel-render
    python3 scripts/measure-render.py [--gl swiftshader|gpu] [--out DIR] [--report FILE]
                                      [--live]    # also compare the running window's monitor

What it does (all through the same JSON operations a CLI or a window sends, no coordinates):

1. builds the proof project from the standard fixtures;
2. renders frames at known times and with three distinct scene cameras, at the export size,
   with `render.frame` (the renderer an export drives);
3. exports the project to H.264 with `export.start` and decodes the same frames back;
4. compares each decoded frame with the renderer's own frame of that time and camera: PSNR and the
   mean absolute difference, against an explicit tolerance (below);
5. with `--live`, renders the same frames through the *running window's* viewport (the monitor's
   WebView, i.e. the preview) and compares them with the headless render;
6. samples the resident memory of the whole process tree (Node, Chrome, Chrome's GPU and renderer
   helpers) every 100 ms and reports the peak per phase.

Nothing here is invented: every number printed comes from this run. What it cannot see: GPU memory
that is not in a process' resident set (unified memory on Apple silicon is reported through the
Chrome GPU helper's RSS only as far as the OS accounts it), and the Spark sort time per frame,
which needs a heavy cloud (see docs/features/spark.md).
"""
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RENDER = os.path.join(ROOT, "target", "debug", "tessel-render")
SOCKET = os.path.expanduser("~/.cache/tessel-editor/ui.sock")

# Explicit tolerances. H.264 at the export bitrate is lossy, so an exported frame differs from the
# renderer's own frame by codec noise only (same renderer, same size, same moment).
EXPORT_PSNR_MIN_DB = 30.0
EXPORT_MEAN_MAX = 3.0  # mean absolute luma difference, 0..255
# The preview (WKWebView, Metal) and the export renderer (headless Chrome) are different GL
# stacks drawing the same plan; they may differ in antialiasing and filtering, not in content.
PREVIEW_PSNR_MIN_DB = 26.0
PREVIEW_MEAN_MAX = 6.0

FRAMES = [0, 12, 30, 47]
# (orbit degrees, fov) for the scene clip at the frames above: known cameras.
CAMERAS = {0: (0, 50), 12: (45, 50), 30: (90, 35), 47: (180, 70)}
# The `fullhd` preset's output size: the export renders at exactly this size, so the renderer's frame
# and the decoded export frame are the same shape and need no resampling to be compared.
SIZE = (1920, 1080)


def descendants(pid, table):
    found, stack = {pid}, [pid]
    while stack:
        parent = stack.pop()
        for child, (ppid, _, _) in table.items():
            if ppid == parent and child not in found:
                found.add(child)
                stack.append(child)
    return found


def process_table():
    table = {}
    listing = subprocess.run(["ps", "-axo", "pid=,ppid=,rss=,comm="], capture_output=True, text=True).stdout
    for line in listing.splitlines():
        parts = line.strip().split(None, 3)
        if len(parts) == 4:
            table[int(parts[0])] = (int(parts[1]), int(parts[2]), parts[3])
    return table


def role(command):
    name = os.path.basename(command)
    lowered = command.lower()
    if "gpu" in lowered:
        return "chrome_gpu"
    if "renderer" in lowered:
        return "chrome_renderer"
    if "chrome" in lowered or "chromium" in lowered:
        return "chrome_browser_or_helper"
    if name.startswith("node"):
        return "node"
    if name.startswith("tessel-render"):
        return "tessel_render"
    if name in ("ffmpeg", "ffprobe"):
        return "ffmpeg"
    return "other"


class Sampler(threading.Thread):
    """Peak resident memory of the process tree under `pid`, per named phase."""

    def __init__(self, pid):
        super().__init__(daemon=True)
        self.pid, self.phase, self.stop = pid, "start", False
        self.peaks = {}

    def run(self):
        while not self.stop:
            table = process_table()
            tree = descendants(self.pid, table)
            total, roles = 0, {}
            for pid in tree:
                if pid not in table:
                    continue
                _, rss, command = table[pid]
                total += rss
                roles[role(command)] = roles.get(role(command), 0) + rss
            peak = self.peaks.setdefault(self.phase, {"total_mb": 0.0, "roles_mb": {}, "processes": 0})
            if total / 1024 > peak["total_mb"]:
                peak["total_mb"] = round(total / 1024, 1)
                peak["roles_mb"] = {name: round(kb / 1024, 1) for name, kb in sorted(roles.items())}
                peak["processes"] = len(tree)
            time.sleep(0.1)


class Session:
    def __init__(self, gl):
        command = [RENDER] + (["--gl", gl] if gl else [])
        self.process = subprocess.Popen(
            command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1
        )
        self.sampler = Sampler(self.process.pid)
        self.sampler.start()

    def op(self, request):
        self.process.stdin.write(json.dumps(request) + "\n")
        self.process.stdin.flush()
        response = json.loads(self.process.stdout.readline())
        if not response.get("ok"):
            raise SystemExit(f"{request.get('op')} failed: {json.dumps(response)[:400]}")
        return response.get("result", response.get("data", response))

    def phase(self, name):
        self.sampler.phase = name

    def close(self):
        self.sampler.stop = True
        self.process.stdin.close()
        self.process.wait(timeout=30)


def ffmpeg(*args):
    return subprocess.run(["ffmpeg", "-v", "info", "-nostdin", "-y", *args], capture_output=True, text=True)


def compare(first, second):
    """PSNR (all planes) and the mean absolute luma difference between two same-size images."""
    psnr = ffmpeg("-i", first, "-i", second, "-lavfi", "psnr", "-f", "null", "-").stderr
    match = re.search(r"average:([\d.]+|inf)", psnr)
    db = float("inf") if match and match.group(1) == "inf" else float(match.group(1)) if match else None
    diff = ffmpeg(
        "-i", first, "-i", second, "-lavfi",
        "blend=all_mode=difference,signalstats,metadata=print:key=lavfi.signalstats.YAVG:file=-",
        "-f", "null", "-",
    )
    mean = re.search(r"YAVG=([\d.]+)", diff.stdout + diff.stderr)
    return {"psnr_db": db, "mean_abs_luma": float(mean.group(1)) if mean else None}


def live_send(request, timeout=120):
    with socket.socket(socket.AF_UNIX) as conn:
        conn.settimeout(timeout)
        conn.connect(SOCKET)
        conn.sendall((json.dumps({"version": 1, "command": "op", "request": request}) + "\n").encode())
        data = b""
        while not data.endswith(b"\n"):
            chunk = conn.recv(65536)
            if not chunk:
                break
            data += chunk
    return json.loads(data)


def main():
    args = sys.argv[1:]
    option = lambda name, default=None: args[args.index(name) + 1] if name in args else default
    gl = option("--gl")
    out = option("--out") or tempfile.mkdtemp(prefix="tessel-measure-")
    os.makedirs(out, exist_ok=True)
    live = "--live" in args
    if not os.path.exists(RENDER):
        raise SystemExit("build tessel-render first: ./scripts/cargo build -p tessel-editor-render --bin tessel-render")

    fixtures = os.path.join(out, "fixtures")
    subprocess.run([os.path.join(ROOT, "scripts", "fixtures"), fixtures], check=True, capture_output=True)
    with open(os.path.join(fixtures, "proof.jsonl")) as handle:
        setup = [json.loads(line) for line in handle if line.strip()]

    report = {"gl": gl or "default", "size": list(SIZE), "frames": FRAMES, "tolerance": {
        "export": {"psnr_min_db": EXPORT_PSNR_MIN_DB, "mean_abs_luma_max": EXPORT_MEAN_MAX},
        "preview": {"psnr_min_db": PREVIEW_PSNR_MIN_DB, "mean_abs_luma_max": PREVIEW_MEAN_MAX},
    }}
    session = Session(gl)
    try:
        session.phase("idle (process started, no frame yet)")
        time.sleep(0.5)
        session.phase("project built, assets loading")
        for request in setup:
            session.op(request)

        session.phase("first frames at the export size")
        renders = {}
        for frame in FRAMES:
            orbit, fov = CAMERAS[frame]
            session.op({"op": "clip.camera", "clip": "c-scene", "position": [0, 1.2, 5], "target": [0, 0, 0],
                        "fov": fov, "orbit": orbit})
            path = os.path.join(out, f"render-{frame:02d}.png")
            data = session.op({"op": "render.frame", "frame": frame, "width": SIZE[0], "height": SIZE[1], "out": path})
            renders[frame] = {"path": path, "render_ms": data["frame"]["render_ms"]}
        report["render_ms"] = {str(frame): renders[frame]["render_ms"] for frame in FRAMES}

        # The export renders the *current* project, so each frame must be exported with the camera
        # it was rendered with: the camera is a per-clip value, so export one range per frame.
        session.phase("export 1080p, one frame per known time and camera")
        export_rows = []
        for frame in FRAMES:
            orbit, fov = CAMERAS[frame]
            session.op({"op": "clip.camera", "clip": "c-scene", "position": [0, 1.2, 5], "target": [0, 0, 0],
                        "fov": fov, "orbit": orbit})
            target = os.path.join(out, f"export-{frame:02d}.mp4")
            started = time.perf_counter()
            session.op({"op": "export.start", "preset": "fullhd", "destination": target, "range": [frame, frame + 1]})
            seconds = time.perf_counter() - started
            decoded = os.path.join(out, f"decoded-{frame:02d}.png")
            ffmpeg("-i", target, "-frames:v", "1", decoded)
            row = {"frame": frame, "camera": {"orbit": CAMERAS[frame][0], "fov": CAMERAS[frame][1]},
                   "export_s": round(seconds, 2)}
            row.update(compare(renders[frame]["path"], decoded))
            row["within_tolerance"] = (
                row["psnr_db"] is not None and row["psnr_db"] >= EXPORT_PSNR_MIN_DB
                and row["mean_abs_luma"] is not None and row["mean_abs_luma"] <= EXPORT_MEAN_MAX
            )
            export_rows.append(row)
        report["export_vs_render"] = export_rows

        session.phase("full 48-frame export, 1080p")
        started = time.perf_counter()
        session.op({"op": "export.start", "preset": "fullhd", "destination": os.path.join(out, "full-1080p.mp4")})
        report["export_1080p_48_frames_s"] = round(time.perf_counter() - started, 2)
        session.phase("full 48-frame export, 4K")
        started = time.perf_counter()
        session.op({"op": "export.start", "preset": "uhd4k", "destination": os.path.join(out, "full-4k.mp4")})
        report["export_4k_48_frames_s"] = round(time.perf_counter() - started, 2)
        session.phase("after exports")
        time.sleep(0.5)
    finally:
        session.sampler.stop = True
        report["memory_peaks"] = session.sampler.peaks
        try:
            session.close()
        except Exception:
            session.process.kill()

    if live:
        rows = []
        try:
            live_send({"op": "capabilities"})
            for request in setup:
                live_send(request)
            for frame in FRAMES:
                orbit, fov = CAMERAS[frame]
                live_send({"op": "clip.camera", "clip": "c-scene", "position": [0, 1.2, 5], "target": [0, 0, 0],
                           "fov": fov, "orbit": orbit})
                path = os.path.join(out, f"preview-{frame:02d}.png")
                answer = live_send({"op": "render.frame", "frame": frame, "width": SIZE[0], "height": SIZE[1],
                                    "out": path})
                if not answer.get("ok"):
                    rows.append({"frame": frame, "error": answer})
                    continue
                row = {"frame": frame}
                row.update(compare(renders[frame]["path"], path))
                row["within_tolerance"] = (
                    row["psnr_db"] is not None and row["psnr_db"] >= PREVIEW_PSNR_MIN_DB
                    and row["mean_abs_luma"] is not None and row["mean_abs_luma"] <= PREVIEW_MEAN_MAX
                )
                rows.append(row)
        except (OSError, ValueError) as error:
            rows.append({"error": f"the editor is not running with its automation socket: {error}"})
        report["preview_vs_render"] = rows

    text = json.dumps(report, indent=2)
    print(text)
    if option("--report"):
        with open(option("--report"), "w") as handle:
            handle.write(text + "\n")
    failed = [r for r in report.get("export_vs_render", []) if not r["within_tolerance"]]
    failed += [r for r in report.get("preview_vs_render", []) if not r.get("within_tolerance")]
    if "--keep" not in args and not option("--out"):
        shutil.rmtree(out, ignore_errors=True)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
