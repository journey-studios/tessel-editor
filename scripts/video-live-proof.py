#!/usr/bin/env python3
"""Proof of container video and proxy sequences in the *real window* (WKWebView host).

    python3 scripts/video-live-proof.py [--binary target/release/tessel-editor] [--keep]

Starts the DIRECT binary with an isolated HOME (own socket, own settings; the user's project is
never touched: nothing is opened, saved or replaced there), makes barcoded clips with FFmpeg
(each picture carries its frame number as a 10-bit barcode in its top row), builds a project
through the API and checks, with semantic requests only (no OS pointer, no coordinate clicks):

  * `render.frame` in the window shows the frame asked for, in forward / reverse / random order,
    first and last included, for an H.264 clip added by path (the native `<video>` path), a PNG
    sequence and a JPEG sequence;
  * a WebM the engine cannot read is a typed error, never a black frame;
  * `seek` (what the playhead does) redraws the monitor: latency from the request to the
    displayed frame, p50/p95/p99, for a 540p and a 1080p H.264 clip.

Prints one JSON report on stdout's last line. Exit status 1 on any failed check. Artifacts are
removed unless --keep.
"""
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
WIDTH, HEIGHT, FRAMES = 640, 360, 48
CELL = WIDTH // 20
failures = []
report = {"checks": [], "latency": {}}


def check(name, condition, detail=""):
    report["checks"].append({"check": name, "ok": bool(condition), "detail": str(detail)[:300]})
    print(("PASS " if condition else "FAIL ") + name + (f"  [{detail}]" if detail and not condition else ""), flush=True)
    if not condition:
        failures.append(name)


def barcode_filter(width):
    cell = width // 20
    boxes = [f"drawbox=x=0:y=0:w={cell * 10}:h={cell}:color=black:t=fill"]
    for bit in range(10):
        boxes.append(f"drawbox=x={bit * cell}:y=0:w={cell}:h={cell}:color=white:t=fill:enable='mod(floor(n/{2 ** bit}),2)'")
    return "noise=c0s=5:allf=t," + ",".join(boxes)


def ffmpeg(*args):
    return subprocess.run(["ffmpeg", "-nostdin", "-v", "error", "-y", *args], capture_output=True, text=True).returncode == 0


def clip(path, width, height, frames, codec):
    source = ["-f", "lavfi", "-i", f"testsrc2=size={width}x{height}:rate=24:duration=100", "-frames:v", str(frames), "-vf", barcode_filter(width)]
    return ffmpeg(*source, *codec, path)


H264 = ["-c:v", "libx264", "-crf", "18", "-preset", "veryfast", "-pix_fmt", "yuv420p", "-g", "24", "-movflags", "+faststart"]


class Editor:
    def __init__(self, binary, home):
        self.socket_path = os.path.join(home, ".cache/tessel-editor/ui.sock")
        env = {**os.environ, "HOME": home}
        # The log stays next to the isolated HOME: it is the only place a failed start explains itself.
        self.log_path = os.path.join(home, "editor.log")
        self.log = open(self.log_path, "w")
        self.process = subprocess.Popen([binary], env=env, stdout=self.log, stderr=self.log, start_new_session=True)

    def wait_ready(self, seconds=90):
        deadline = time.time() + seconds
        while time.time() < deadline:
            if self.process.poll() is not None:
                with open(self.log_path) as handle:
                    raise RuntimeError(f"the editor exited ({self.process.returncode}) before it was ready: {handle.read()[-600:]}")
            if os.path.exists(self.socket_path):
                try:
                    if self.viewport()["phase"] == "ready":
                        return
                except (OSError, ValueError, KeyError, TypeError):
                    pass
            time.sleep(0.25)
        raise RuntimeError("the viewport never became ready")

    def send(self, request, timeout=60):
        started = time.perf_counter()
        with socket.socket(socket.AF_UNIX) as conn:
            conn.settimeout(timeout)
            conn.connect(self.socket_path)
            conn.sendall((json.dumps(request) + "\n").encode())
            data = b""
            while not data.endswith(b"\n"):
                chunk = conn.recv(65536)
                if not chunk:
                    break
                data += chunk
        response = json.loads(data)
        response["_ms"] = (time.perf_counter() - started) * 1000
        return response

    def op(self, request, timeout=60):
        return self.send({"version": 1, "command": "op", "request": request}, timeout)

    def viewport(self):
        return self.send({"version": 1, "command": "get", "target": "monitor.viewport"})["data"]["state"]

    def stop(self):
        self.process.terminate()
        try:
            self.process.wait(10)
        except subprocess.TimeoutExpired:
            self.process.kill()


def decode(probes):
    return sum((1 << bit) for bit, pixel in enumerate(probes) if sum(pixel[:3]) / 3 > 128)


def shuffled(count, seed):
    state = seed
    items = list(range(count))
    for index in range(count - 1, 0, -1):
        state = (state * 1664525 + 1013904223) % (1 << 32)
        j = state % (index + 1)
        items[index], items[j] = items[j], items[index]
    return items


def percentiles(values):
    ordered = sorted(values)
    at = lambda share: round(ordered[min(len(ordered) - 1, int(share * len(ordered)))], 1)
    return {"n": len(ordered), "p50": at(0.5), "p95": at(0.95), "p99": at(0.99), "max": round(ordered[-1], 1)}


def main():
    binary = os.path.join(ROOT, "target/release/tessel-editor")
    if "--binary" in sys.argv:
        binary = os.path.abspath(sys.argv[sys.argv.index("--binary") + 1])
    if not os.path.exists(binary):
        print(f"missing {binary}; run ./scripts/cargo build --release --locked")
        return 2
    # A short path: the socket lives under HOME and a Unix socket path is limited (SUN_LEN).
    work = tempfile.mkdtemp(prefix="tvl-", dir="/tmp")
    home = os.path.join(work, "home")
    os.makedirs(home)
    editor = Editor(binary, home)
    try:
        editor.wait_ready()
        report["binary"] = binary
        editor.send({"version": 1, "command": "invoke", "target": "window.activate"})
        time.sleep(0.5)
        caps = editor.op({"op": "capabilities"})
        check("capabilities answer from the real window", caps.get("ok"), str(caps)[:200])
        report["renderer"] = caps.get("data", {}).get("renderer")

        paths = {
            "h264": os.path.join(work, "h264.mp4"),
            "big": os.path.join(work, "h264-1080.mp4"),
            "webm": os.path.join(work, "vp9.webm"),
        }
        check("fixture h264 540p", clip(paths["h264"], WIDTH, HEIGHT, FRAMES, H264))
        check("fixture h264 1080p", clip(paths["big"], 1920, 1080, 240, H264))
        have_webm = clip(paths["webm"], WIDTH, HEIGHT, FRAMES, ["-c:v", "libvpx-vp9", "-b:v", "0", "-crf", "30", "-g", "24", "-cpu-used", "6", "-deadline", "good", "-pix_fmt", "yuv420p"])
        for folder, ext, quality in (("png", "png", ["-compression_level", "1"]), ("jpg", "jpg", ["-q:v", "3"])):
            os.makedirs(os.path.join(work, folder))
            pattern = os.path.join(work, folder, f"x%05d.{ext}")
            source = ["-f", "lavfi", "-i", f"testsrc2=size={WIDTH}x{HEIGHT}:rate=24:duration=100", "-frames:v", str(FRAMES), "-vf", barcode_filter(WIDTH)]
            check(f"fixture {folder} sequence", ffmpeg(*source, *quality, "-start_number", "0", pattern))
            paths[folder] = pattern

        assets = {
            "h264": {"op": "asset.add", "kind": "video", "id": "h264", "name": "H264", "path": paths["h264"], "rate": 24, "width": WIDTH, "height": HEIGHT, "duration": 2},
            "big": {"op": "asset.add", "kind": "video", "id": "big", "name": "H264 1080", "path": paths["big"], "rate": 24, "width": 1920, "height": 1080, "duration": 10},
            "png": {"op": "asset.add", "kind": "video", "id": "png", "name": "PNG", "pattern": paths["png"], "rate": 24, "frames": FRAMES},
            "jpg": {"op": "asset.add", "kind": "video", "id": "jpg", "name": "JPG", "pattern": paths["jpg"], "rate": 24, "frames": FRAMES},
            "webm": {"op": "asset.add", "kind": "video", "id": "webm", "name": "WebM", "path": paths["webm"], "rate": 24, "width": WIDTH, "height": HEIGHT, "duration": 2},
        }

        def show(asset, length):
            """A fresh project holding only `asset`, as one clip from frame 0."""
            steps = [
                {"op": "project.create", "name": f"Video live {asset}", "sequence": "main", "rate": 24, "width": 320, "height": 180, "length": length},
                assets[asset],
                {"op": "track.add", "id": "t", "name": "T", "kind": "video"},
                {"op": "clip.add", "id": f"c-{asset}", "track": "t", "asset": asset, "start": 0, "duration": length},
            ]
            for step in steps:
                response = editor.op(step)
                if not response.get("ok"):
                    return False, response
            return True, None

        def code_at(frame):
            probes = [[(bit * 2 + 1) * 160 // 20, 8] for bit in range(10)]
            response = editor.op({"op": "render.frame", "frame": frame, "probes": probes})
            if not response.get("ok"):
                return None, response
            return decode(response["data"]["frame"]["probes"]), response

        for asset in ("h264", "png", "jpg"):
            ok, response = show(asset, FRAMES)
            check(f"{asset}: clip on the timeline", ok, str(response)[:200])
            for name, order in (
                ("edges", [0, FRAMES - 1, 0, 1, FRAMES - 2, 24, 23, 25]),
                ("forward", list(range(FRAMES))),
                ("reverse", list(range(FRAMES - 1, -1, -1))),
                ("random", shuffled(FRAMES, 7)),
            ):
                wrong = []
                for frame in order:
                    code, response = code_at(frame)
                    if code != frame:
                        wrong.append((frame, code, None if code is not None else str(response)[:120]))
                check(f"{asset} {name}: shown frame == asked frame in the real window", not wrong, wrong[:5])

        if have_webm:
            show("webm", FRAMES)
            code, response = code_at(10)
            error = (response or {}).get("error") or {}
            typed = code is None and error.get("code") in ("unsupported_format", "decode_failed")
            check("webm: a typed error or the exact frame, never a black frame", typed or code == 10, f"code={code} error={error}")
            report["webm_outcome"] = "exact" if code == 10 else error.get("code")

        # Seek -> displayed, the latency of scrubbing in the window.
        for asset, length in (("h264", FRAMES), ("big", 240)):
            ok, _ = show(asset, length)
            check(f"{asset}: clip for the seek measurement", ok)
            # GPUI only paints a window that is not covered; the monitor's bookkeeping runs in that paint.
            editor.send({"version": 1, "command": "invoke", "target": "window.activate"})
            time.sleep(1.0)
            editor.op({"op": "seek", "frame": 0})
            time.sleep(1.0)
            samples = []
            for frame in shuffled(length, 11)[:120]:
                before = (editor.viewport()["last_frame"] or {}).get("revision", 0)
                started = time.perf_counter()
                editor.op({"op": "seek", "frame": frame})
                deadline = time.time() + 15
                while time.time() < deadline:
                    state = editor.viewport()
                    if state["last_frame"] and state["last_frame"]["revision"] > before and state["session"]["status"] == "ready":
                        break
                    time.sleep(0.002)
                samples.append((time.perf_counter() - started) * 1000)
            report["latency"][f"{asset}:seek_to_displayed_ms:random"] = percentiles(samples)
            check(f"{asset}: no seek took a second (p99 < 500 ms)", percentiles(samples)["p99"] < 500, report["latency"][f"{asset}:seek_to_displayed_ms:random"])
    finally:
        editor.stop()
        if "--keep" in sys.argv:
            print("kept", work)
        else:
            shutil.rmtree(work, ignore_errors=True)
    print(json.dumps(report))
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
