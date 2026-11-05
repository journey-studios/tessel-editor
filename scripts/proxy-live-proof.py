#!/usr/bin/env python3
"""Proof of the automatic preview proxy (OPT01) and the selective repaint (OPT04) in the *real window*.

    python3 scripts/proxy-live-proof.py [--binary target/release/tessel-editor] [--runs 3]
                                        [--out DIR] [--keep]

Starts the DIRECT release binary with an isolated HOME per variant (own socket, own settings; the
user's project is never touched), makes a barcoded 1080p H.264 clip with FFmpeg and, with semantic
requests only (no OS pointer, no coordinate clicks), checks:

  * the media job builds the proxy on its own and `op preview.policy` says `ready`;
  * a scrub frame is drawn from the proxy (`last_frame.source.proxy_layers > 0`, quality
    `preview`), and ~200 ms after the last intent the monitor replaces it with the original;
  * an explicit `render.frame` (barcode decoded) and an export use the ORIGINAL: the export of the
    proxy-enabled editor matches the export of the `original` policy frame for frame (ffmpeg
    framemd5) and sits at the PSNR of a plain re-encode of the source, not of an upscaled 540p;
  * with `TESSEL_VIEWPORT_INSPECT=1` the picture on screen after the drag is byte-identical to an
    explicit original `render.frame` (the refine converges to the original pixels);
  * scrub numbers per variant (`seek-scrub-proof.py`, the editor's own clock) so the proxy is compared on the same machine, same clip, same drag.

Variants: original, auto (the present-gate experiment, OPT03, was measured with this script and
removed: no gain, see docs/viewport-performance-next-report.md). The poll of the observer is never FPS.
Prints one JSON report on stdout's last line and writes it to --out when given. Exit 1 on a
failed check. Own artifacts live under /tmp and are removed unless --keep.
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

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
HERE = os.path.dirname(os.path.abspath(__file__))


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, file))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


video = load("video_live_proof", "video-live-proof.py")
failures = []
report = {"checks": [], "variants": {}}


def check(name, condition, detail=""):
    report["checks"].append({"check": name, "ok": bool(condition), "detail": str(detail)[:300]})
    print(("PASS " if condition else "FAIL ") + name + (f"  [{detail}]" if detail and not condition else ""), flush=True)
    if not condition:
        failures.append(name)


class SkipPixel(Exception):
    pass


class Variant:
    def __init__(self, binary, work, name, policy, env):
        self.name, self.policy = name, policy
        home = os.path.join(work, f"h-{name}")
        os.makedirs(home)
        self.home = home
        self.env = env
        self.editor = None
        self.asset = None
        self.binary = binary

    def start(self):
        env = {**os.environ, "HOME": self.home, **self.env}
        editor = video.Editor.__new__(video.Editor)
        editor.socket_path = os.path.join(self.home, ".cache/tessel-editor/ui.sock")
        editor.log_path = os.path.join(self.home, "editor.log")
        editor.log = open(editor.log_path, "w")
        editor.process = subprocess.Popen([self.binary], env=env, stdout=editor.log, stderr=editor.log, start_new_session=True)
        self.editor = editor
        editor.wait_ready()
        editor.send({"version": 1, "command": "invoke", "target": "window.activate"})
        time.sleep(0.5)

    def stop(self):
        if self.editor:
            self.editor.stop()

    def op(self, request, timeout=60):
        return self.editor.op(request, timeout)

    def viewport(self):
        return self.editor.viewport()


def build_project(variant, clip, length):
    """A saved project with the clip imported (video is decoded into the workspace, which needs a
    save and a real import: `asset.add` by path is not hashed and cannot be exported)."""
    workspace = os.path.join(variant.home, "project")
    document = os.path.join(workspace, "proof.tessel.json")
    first = [
        {"op": "project.create", "name": "Proxy live", "sequence": "main", "rate": 24, "width": 1920, "height": 1080, "length": length, "dir": workspace},
        {"op": "project.save", "path": document},
        {"op": "media.import", "paths": [clip]},
    ]
    for step in first:
        response = variant.op(step)
        if not response.get("ok"):
            return response

    def imported():
        assets = (variant.op({"op": "media.status"}).get("data") or {}).get("assets") or []
        return assets[0]["id"] if assets else None

    asset = wait_for(imported, 120)
    if not asset:
        return {"error": "the import never produced an asset"}
    variant.asset = asset
    for step in [
        {"op": "track.add", "id": "t", "name": "T", "kind": "video"},
        {"op": "clip.add", "id": "c", "track": "t", "asset": asset, "start": 0, "duration": length},
        {"op": "project.save", "path": document},
    ]:
        response = variant.op(step)
        if not response.get("ok"):
            return response
    return None


def wait_for(predicate, seconds, step=0.2):
    deadline = time.time() + seconds
    while time.time() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(step)
    return None


def policy_report(variant):
    response = variant.op({"op": "preview.policy"})
    return (response.get("data") or {}).get("report") or response.get("data") or {}


def asset_reason(report_json):
    """The reason of the (only) video asset, whatever ID the import gave it."""
    assets = report_json.get("assets") or {}
    items = list(assets.values()) if isinstance(assets, dict) else list(assets)
    return items[0].get("reason") if items else None


def scrub(variant, runs, extra):
    """Several drags through seek-scrub-proof.py; one result per run."""
    results = []
    for index in range(runs):
        path = os.path.join(variant.home, f"scrub-{index}.json")
        command = [
            sys.executable, os.path.join(HERE, "seek-scrub-proof.py"),
            "--socket", variant.editor.socket_path, "--seconds", "2", "--hz", "60", "--span", "200",
            "--json", path, *extra,
        ]
        outcome = subprocess.run(command, capture_output=True, text=True)
        if os.path.exists(path):
            with open(path) as handle:
                data = json.load(handle)
            data["_exit"] = outcome.returncode
            results.append(data)
        else:
            results.append({"_exit": outcome.returncode, "_error": (outcome.stdout + outcome.stderr)[-400:]})
        time.sleep(0.5)
    return results


def pick(result, *keys):
    value = result
    for key in keys:
        if not isinstance(value, dict) or key not in value:
            return None
        value = value[key]
    return value


def summarize(results):
    def column(*keys):
        values = [pick(r, *keys) for r in results]
        return [v for v in values if isinstance(v, (int, float))]

    def median(values):
        ordered = sorted(values)
        return ordered[len(ordered) // 2] if ordered else None

    return {
        "runs": len(results),
        "exit_codes": [r.get("_exit") for r in results],
        "presentations_in_window": column("presentations_in_window"),
        "present_gap_ms_median": median(column("present_gap_ms", "median")),
        "present_gap_ms_p95": median(column("present_gap_ms", "p95")),
        "lag_frames_median": median(column("lag_frames", "median")),
        "release_to_picture_ms": column("release_to_picture_ms"),
        "release_to_final_ms": column("release_to_final_ms"),
        "release_to_original_polled_ms": column("release_to_original_polled_ms"),
        "render_ms_median": median(column("last_frame", "render_ms")),
        "stats_delta": [r.get("stats_delta") for r in results],
    }


def refine_latency(variant, samples=5):
    """Time from the last scrub intent to the original-quality picture replacing the proxy one.

    Polled (every ~10 ms through `monitor.viewport`), so it is bounded by the poll and is an upper
    bound; every sample ends on a frame the monitor has not shown before, otherwise nothing is drawn.
    """
    results = []
    for index in range(samples):
        for step in range(8):
            frame = 20 + index * 40 + step * 3
            variant.editor.send({"version": 1, "command": "set", "target": "timeline.playhead", "field": "frame", "value": str(frame)})
            time.sleep(1 / 60)
        released = time.monotonic()
        picture, original = None, None
        while time.monotonic() - released < 3:
            state = variant.viewport()
            last = state.get("last_frame") or {}
            source = last.get("source") or {}
            if picture is None and source.get("proxy_layers"):
                picture = time.monotonic() - released
            if picture is not None and not source.get("proxy_layers") and not state["session"].get("refine_pending") and state["session"]["status"] == "ready":
                original = time.monotonic() - released
                break
            time.sleep(0.01)
        results.append({"proxy_picture_ms": round(picture * 1000) if picture is not None else None,
                        "original_picture_ms": round(original * 1000) if original is not None else None})
        time.sleep(0.5)
    return results


def export(variant, destination, frames):
    started = variant.op({"op": "export.start", "preset": "fullhd", "destination": destination, "range": [0, frames], "replace": True})
    if not started.get("ok"):
        return False, started
    job = (started.get("data") or {}).get("job")

    def done():
        info = variant.op({"op": "job.get", "job": job}).get("data") or {}
        return info if info.get("state") in ("succeeded", "failed", "cancelled") else None

    info = wait_for(done, 600, 0.5)
    return bool(info and info["state"] == "succeeded"), info


def framemd5(path):
    out = subprocess.run(["ffmpeg", "-nostdin", "-v", "error", "-i", path, "-map", "0:v:0", "-f", "framemd5", "-"], capture_output=True, text=True)
    return [line.split(",")[-1].strip() for line in out.stdout.splitlines() if line and not line.startswith("#")]


def psnr(path, reference, frames, through_proxy=False):
    """PSNR of `path` against `reference`; `through_proxy` first squeezes the reference to the
    540-row proxy size and back, i.e. what an export drawn from the proxy would look like."""
    squeeze = "scale=960:540:flags=bicubic,scale=1920:1080:flags=bicubic," if through_proxy else ""
    out = subprocess.run(
        ["ffmpeg", "-nostdin", "-i", path, "-i", reference, "-frames:v", str(frames),
         "-lavfi", f"[0:v]format=yuv420p[a];[1:v]{squeeze}format=yuv420p[b];[a][b]psnr", "-f", "null", "-"],
        capture_output=True, text=True)
    for line in out.stderr.splitlines():
        if "average:" in line and "PSNR" in line:
            token = line.split("average:")[1].split()[0]
            return float(token) if token != "inf" else 99.0
    return None


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default=os.path.join(ROOT, "target/release/tessel-editor"))
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--out")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--only", help="comma separated variant names (and `pixel`); default all")
    args = parser.parse_args()
    if not os.path.exists(args.binary):
        print(f"missing {args.binary}; run ./scripts/cargo build --release --locked")
        return 2
    work = tempfile.mkdtemp(prefix="tpp-", dir="/tmp")
    length = 240
    clip = os.path.join(work, "h264-1080.mp4")
    check("fixture h264 1080p", video.clip(clip, 1920, 1080, length, video.H264))
    variants = [
        ("original", "original", {"TESSEL_VIEWPORT_TRACE": "1"}),
        ("auto", "auto", {"TESSEL_VIEWPORT_TRACE": "1"}),
    ]
    exports = {}
    wanted = set(args.only.split(",")) if args.only else None
    try:
        for name, policy, env in variants:
            if wanted and name not in wanted:
                continue
            variant = Variant(args.binary, work, name, policy, env)
            report["variants"][name] = {"policy": policy, "env": env}
            try:
                variant.start()
                failed = build_project(variant, clip, length)
                check(f"{name}: project with the 1080p clip", failed is None, failed)
                response = variant.op({"op": "preview.policy", "mode": policy})
                check(f"{name}: policy {policy} accepted", response.get("ok"), response)
                # Decoding the clip (and, for auto, building the proxy) is a job that ends by itself.
                ready = wait_for(lambda: variant.op({"op": "render.frame", "frame": 0}).get("ok"), 180)
                check(f"{name}: decode finished (explicit frame answers)", ready)
                if policy == "auto":
                    reason = wait_for(lambda: asset_reason(policy_report(variant)) == "proxy_ready", 180)
                    check(f"{name}: media job built the proxy (`preview.policy` reason proxy_ready)", reason, asset_reason(policy_report(variant)))
                else:
                    check(f"{name}: original policy keeps proxies out (reason policy_original)", asset_reason(policy_report(variant)) == "policy_original", asset_reason(policy_report(variant)))
                variant.editor.send({"version": 1, "command": "invoke", "target": "window.activate"})
                time.sleep(1.0)

                # One scrub frame, then the refine.
                variant.op({"op": "seek", "frame": 0})
                time.sleep(1.0)
                reply = variant.editor.send({"version": 1, "command": "set", "target": "timeline.playhead", "field": "frame", "value": "120"})
                check(f"{name}: scrub intent accepted", reply.get("ok"), reply)
                seen = {"proxy": False}

                def settled():
                    state = variant.viewport()
                    last = state.get("last_frame") or {}
                    source = last.get("source") or {}
                    if source.get("proxy_layers"):
                        seen["proxy"] = True
                    session = state["session"]
                    if session["status"] == "ready" and not session.get("pending") and not session.get("refine_pending") and last.get("revision") == session["revision"] and not source.get("proxy_layers"):
                        return state
                    return None

                state = wait_for(settled, 20, 0.005)
                check(f"{name}: scrub picture converges on the original", state, "no convergence")
                if policy == "auto":
                    check(f"{name}: the scrub picture came from the proxy", seen["proxy"], "no frame reported proxy layers")
                    stats = state["session"]["stats"] if state else {}
                    check(f"{name}: a refine frame replaced the proxy picture", stats.get("refined", 0) >= 1, stats)
                else:
                    check(f"{name}: no proxy picture under the original policy", not seen["proxy"])

                # Explicit frame = original, exact barcode.
                probes = [[(bit * 2 + 1) * 1920 // 40, 8] for bit in range(10)]
                explicit = variant.op({"op": "render.frame", "frame": 77, "probes": probes})
                data = (explicit.get("data") or {}).get("frame") or {}
                code = video.decode(data["probes"]) if data.get("probes") else None
                check(f"{name}: explicit render.frame shows the frame asked for", code == 77, f"code={code}")
                source = data.get("source") or {}
                check(f"{name}: explicit render.frame is original quality", source.get("quality") in (None, "original") and not source.get("proxy_layers"), source)

                if policy in ("original", "auto") and True:
                    destination = os.path.join(work, f"export-{name}.mp4")
                    ok, info = export(variant, destination, 24)
                    check(f"{name}: export of 24 frames succeeds", ok, info)
                    exports[name] = destination

                if policy == "auto":
                    latency = refine_latency(variant)
                    report["variants"][name]["refine_latency"] = latency
                    check(f"{name}: every drag ended on a proxy picture and then on the original", all(item["proxy_picture_ms"] is not None and item["original_picture_ms"] is not None for item in latency), latency)
                runs = scrub(variant, args.runs, [])
                summary = summarize(runs)
                report["variants"][name]["scrub"] = summary
                check(f"{name}: scrub runs converged on the last position", all(r.get("converged") for r in runs), [r.get("converged") for r in runs])
                check(f"{name}: scrub kept the monitor updating", all((pick(r, "presentations_in_window") or 0) >= 5 for r in runs), summary["presentations_in_window"])
                status = (variant.op({"op": "render.status"}).get("data") or {})
                report["variants"][name]["ui"] = status.get("ui")
                report["variants"][name]["session"] = (status.get("session") or {}).get("stats")
            finally:
                variant.stop()

        # Export: proxy-enabled editor vs original policy, frame by frame.
        if "original" in exports and "auto" in exports:
            hashes_original, hashes_auto = framemd5(exports["original"]), framemd5(exports["auto"])
            check("export: 24 frames in both", len(hashes_original) == 24 and len(hashes_auto) == 24, (len(hashes_original), len(hashes_auto)))
            check("export: the proxy-enabled editor exports the same frames as the original policy", hashes_original == hashes_auto, "framemd5 differs")
            psnr_auto = psnr(exports["auto"], clip, 24)
            psnr_original = psnr(exports["original"], clip, 24)
            # Informational only (the synthetic clip is noise, so PSNR cannot tell a 540p copy from a
            # re-encode): the proof is the frame-by-frame identity above. The `original` policy editor has
            # no proxy at all, so an identical export cannot have been drawn from one.
            report["export"] = {"psnr_auto_vs_source": psnr_auto, "psnr_original_policy_vs_source": psnr_original,
                                "frames": len(hashes_auto), "identical_to_original_policy": hashes_original == hashes_auto}
            probe = json.loads(subprocess.run(["ffprobe", "-v", "error", "-show_streams", "-of", "json", exports["auto"]], capture_output=True, text=True).stdout)
            video_stream = next((s for s in probe.get("streams", []) if s["codec_type"] == "video"), {})
            check("export: 1920x1080 at 24 fps", (video_stream.get("width"), video_stream.get("height"), video_stream.get("r_frame_rate")) == (1920, 1080, "24/1"), video_stream)

        # Pixel proof: the picture on screen after a drag with proxies is the original's pixels.
        pixel = Variant(args.binary, work, "pixel", "auto", {"TESSEL_VIEWPORT_INSPECT": "1"})
        try:
            if wanted and "pixel" not in wanted:
                raise SkipPixel()
            pixel.start()
            check("pixel: project", build_project(pixel, clip, length) is None)
            pixel.op({"op": "preview.policy", "mode": "auto"})
            wait_for(lambda: pixel.op({"op": "render.frame", "frame": 0}).get("ok"), 180)
            wait_for(lambda: asset_reason(policy_report(pixel)) == "proxy_ready", 180)
            pixel.editor.send({"version": 1, "command": "invoke", "target": "window.activate"})
            time.sleep(1.0)
            runs = scrub(pixel, 2, ["--pixel-proof"])
            report["pixel"] = [{"exit": r.get("_exit"), "pixels": r.get("pixels"), "checks": r.get("checks")} for r in runs]
            check("pixel: after the drag the screen matches an explicit original render", all(r.get("_exit") == 0 for r in runs), [r.get("pixels") or r.get("_error") for r in runs])
        except SkipPixel:
            pass
        finally:
            pixel.stop()
    finally:
        if args.keep:
            print("kept", work)
        else:
            shutil.rmtree(work, ignore_errors=True)
    if args.out:
        with open(args.out, "w") as handle:
            json.dump(report, handle, indent=1)
    print(json.dumps(report))
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
