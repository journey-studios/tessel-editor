#!/usr/bin/env python3
"""The plan's media flow with no window: library + CLI (`tessel-render`), nothing else.

    ./scripts/cargo build -p tessel-editor-render --locked
    python3 scripts/headless-flow-proof.py [--work DIR] [--report FILE]

Process A creates a project, imports a video that has its own sound plus a tone, builds a
timeline and saves it. Process B (a fresh process, so nothing decoded is in memory) opens the
saved project and exports 1080p and 4K **without any manual `media.prepare`**. The exported
files are read back with ffprobe and measured spectrally: the video clip's own 330 Hz sound
(frames 24-48) and the audio track's 440 Hz tone (whole sequence) must both be there, each where
it belongs. Spectral content proves the sound reached the mix; it does not prove it was heard.
"""
import json
import os
import shutil
import subprocess
import sys
import time

import tonecheck

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RENDER = os.path.join(ROOT, "target/debug/tessel-render")
FAILED = []


def check(name, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else f"  -> {detail}"), flush=True)
    if not ok:
        FAILED.append(name)
    return ok


class Session:
    def __init__(self):
        self.process = subprocess.Popen([RENDER], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1)

    def op(self, request):
        self.process.stdin.write(json.dumps(request) + "\n")
        self.process.stdin.flush()
        return json.loads(self.process.stdout.readline())

    def close(self):
        self.process.stdin.close()
        self.process.wait(timeout=60)


def ffprobe(path):
    out = subprocess.run(["ffprobe", "-v", "error", "-show_streams", "-show_format", "-of", "json", path], capture_output=True, check=True)
    return json.loads(out.stdout)


def main():
    args = sys.argv[1:]
    work = args[args.index("--work") + 1] if "--work" in args else "/tmp/tessel-headless-flow"
    report_path = args[args.index("--report") + 1] if "--report" in args else None
    shutil.rmtree(work, ignore_errors=True)
    os.makedirs(work)
    originals = os.path.join(work, "originals")
    os.makedirs(originals)
    video = os.path.join(originals, "clip.mp4")
    tone = os.path.join(originals, "tone-440.wav")
    subprocess.run(["ffmpeg", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=duration=2:size=320x180:rate=24",
                    "-f", "lavfi", "-i", "sine=frequency=330:duration=2", "-c:v", "libx264", "-pix_fmt", "yuv420p",
                    "-c:a", "aac", "-shortest", video], check=True)
    subprocess.run(["ffmpeg", "-loglevel", "error", "-f", "lavfi", "-i", "sine=frequency=440:duration=2", "-ar", "48000", tone], check=True)
    workspace = os.path.join(work, "project")
    document = os.path.join(workspace, "flow.tessel.json")
    metrics = {}

    a = Session()
    try:
        steps = [
            {"op": "project.create", "name": "Headless flow", "sequence": "main", "rate": 24, "width": 1920, "height": 1080, "length": 48, "dir": workspace},
            {"op": "project.save", "path": document},
        ]
        for step in steps:
            reply = a.op(step)
            check(step["op"], reply.get("ok"), json.dumps(reply)[:300])
        imported = a.op({"op": "media.import", "paths": [video, tone]})
        items = imported.get("result", {}).get("applied", {}).get("items", [])
        check("import: the video and the tone land in the library", imported.get("ok") and len(items) == 2, json.dumps(imported)[:400])
        ids = {item["name"]: item["id"] for item in items}
        for step in [
            {"op": "track.add", "id": "v1", "name": "Video", "kind": "video"},
            {"op": "track.add", "id": "music", "name": "Music", "kind": "audio"},
            {"op": "clip.add", "id": "c-vid", "track": "v1", "asset": ids["clip.mp4"], "start": 24, "duration": 24},
            {"op": "clip.add", "id": "c-tone", "track": "music", "asset": ids["tone-440.wav"], "start": 0, "duration": 48},
            {"op": "project.save", "path": document},
        ]:
            reply = a.op(step)
            check(f"timeline: {step['op']} {step.get('id', '')}", reply.get("ok"), json.dumps(reply)[:300])
    finally:
        a.close()

    b = Session()
    try:
        opened = b.op({"op": "project.open", "path": document})
        check("a fresh process opens the saved project", opened.get("ok"), json.dumps(opened)[:300])
        for preset, width, height in [("fullhd", 1920, 1080), ("uhd4k", 3840, 2160)]:
            destination = os.path.join(work, f"{preset}.mp4")
            started = time.time()
            reply = b.op({"op": "export.start", "preset": preset, "destination": destination, "wait": True})
            metrics[f"export_{preset}_seconds"] = round(time.time() - started, 1)
            check(f"{preset}: export succeeds with no manual media.prepare", reply.get("ok"), json.dumps(reply)[:500])
            if not reply.get("ok"):
                continue
            probe = ffprobe(destination)
            stream = next(s for s in probe["streams"] if s["codec_type"] == "video")
            audio = [s for s in probe["streams"] if s["codec_type"] == "audio"]
            check(f"{preset}: {width}x{height}, 48 frames at 24 fps, AAC", (stream["width"], stream["height"]) == (width, height)
                  and stream["nb_frames"] == "48" and stream["r_frame_rate"] == "24/1" and audio and audio[0]["codec_name"] == "aac", json.dumps(probe)[:300])
            levels = {"330_first": tonecheck.tone_level(destination, 0.1, 0.8, 330), "330_second": tonecheck.tone_level(destination, 1.1, 0.8, 330),
                      "440_first": tonecheck.tone_level(destination, 0.1, 0.8, 440), "440_second": tonecheck.tone_level(destination, 1.1, 0.8, 440)}
            metrics[f"levels_{preset}"] = {key: round(value, 4) for key, value in levels.items()}
            check(f"{preset}: the video's own 330 Hz sound is in the export, only while the clip plays",
                  levels["330_second"] > 0.02 and levels["330_first"] < levels["330_second"] / 20, str(levels))
            check(f"{preset}: the 440 Hz track tone covers the whole sequence", levels["440_first"] > 0.02 and levels["440_second"] > 0.02, str(levels))
            muted = b.op({"op": "clip.property", "clip": "c-vid", "property": "audio_muted", "value": True}) if preset == "fullhd" else None
            if muted is not None:
                check("muting the clip's sound is one edit", muted.get("ok"), json.dumps(muted)[:300])
                silent = os.path.join(work, "fullhd-muted.mp4")
                again = b.op({"op": "export.start", "preset": "fullhd", "destination": silent, "wait": True})
                check("fullhd muted: export succeeds", again.get("ok"), json.dumps(again)[:300])
                if again.get("ok"):
                    level = tonecheck.tone_level(silent, 1.1, 0.8, 330)
                    metrics["muted_330_second"] = round(level, 4)
                    check("fullhd muted: the video's sound is gone and the track tone stays",
                          level < 0.005 and tonecheck.tone_level(silent, 1.1, 0.8, 440) > 0.02, f"{level}")
                b.op({"op": "clip.property", "clip": "c-vid", "property": "audio_muted", "value": False})
    finally:
        b.close()

    print(json.dumps(metrics, indent=2))
    if report_path:
        with open(report_path, "w") as handle:
            json.dump({"metrics": metrics, "failed": FAILED}, handle, indent=2)
    print(f"\n{'FAILED: ' + str(FAILED) if FAILED else 'all checks passed'}")
    return 1 if FAILED else 0


if __name__ == "__main__":
    sys.exit(main())
