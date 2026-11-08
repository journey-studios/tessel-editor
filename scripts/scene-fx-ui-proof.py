#!/usr/bin/env python3
"""F016: the vignette in the REAL window — edited through the live semantic nodes, drawn by the
monitor's WebView, exported through the dialog. Only `op` / `get` / `set` / `invoke`: no pointer and
no coordinates (the pointer path is covered by the GPUI tests of `scene_fx_tests.rs`, BL043 stays open).

    python3 scripts/directed-scene-proof/with-gpu-lock.py \\
        python3 scripts/scene-fx-ui-proof.py [--binary target/debug/tessel-editor] [--keep] [--json FILE]

Starts the DIRECT binary with a short isolated HOME (the user's editor, project and socket are never
addressed) over the synthetic corpus of `scripts/directed-scene-proof` (drawn from scratch, no private
media). The expectation is an independent Python transcription of the vignette (not the Rust
reference and not the GLSL): the baseline picture's probes go through it and must equal what the
window's monitor and export draw with the effect.

What it checks, in order:
  * the live tree exposes `scene.fx`, its add action and no clip yet; the baseline picture is
    probed (a 9x6 grid) in the monitor and by an explicit render, and they agree (hash);
  * `invoke scene.fx add:vignette` creates `scene.fx.clip:fx-1`; `set ... intensity 1` and the monitor
    redraws: every probe of the grid equals the Python vignette of the baseline within tolerance and
    the picture really changed (discriminating);
  * intensity 0 draws the baseline's very pixels (hash equal); moving the centre moves the dark
    region (probes match the Python model for the new centre and differ from the first picture);
  * a refused value (`intensity 2`) is the typed `param_out_of_range`, shown in `scene.panel` as the
    notice, and changes neither the document, the picture nor the history;
  * undo/redo (`op history.undo|redo`) step through the single entries (one `set` is one entry);
  * `project.save` + `project.open` bring the clip back with its values (persistence in the window);
  * `flow.export_scene` (dialog) writes an MP4 whose decoded frames match the Python vignette over
    the decoded baseline export, inside the clip, and are the baseline's outside it.

Exit 0 only if every check passed; 1 on a failed check; 2 on a missing prerequisite. Artifacts under
/tmp/te-fx-* (< 500 MiB), refuses to start under 12 GiB free, removed unless --keep.
"""
import argparse
import importlib.util
import json
import math
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts" / "directed-scene-proof"))
import harness as h  # noqa: E402

CORPUS = ROOT / "scripts" / "directed-scene-proof" / "corpus"
MIN_FREE_GIB = 12
WIDTH, HEIGHT, EDIT_FRAMES = 320, 180, 96
FRAME = 12
# 8-bit rounding plus the GPU's float path; the pictures are flat blocks, so a probe is never on an edge.
TOLERANCE = 3
CODEC_TOLERANCE = 16

failures, checks = [], []


def check(name, condition, detail=""):
    checks.append({"check": name, "ok": bool(condition), "detail": "" if condition else str(detail)[:400]})
    print(("PASS " if condition else "FAIL ") + name + (f"  [{str(detail)[:300]}]" if detail and not condition else ""), flush=True)
    if not condition:
        failures.append(name)
    return bool(condition)


def load_lifecycle():
    path = ROOT / "scripts" / "scene-lifecycle-proof.py"
    spec = importlib.util.spec_from_file_location("scene_lifecycle_proof", path)
    module = importlib.util.module_from_spec(spec)
    sys.modules["scene_lifecycle_proof"] = module
    spec.loader.exec_module(module)
    return module


# ---- the independent expectation: the vignette in Python -------------------------------------------

def srgb_to_linear(c):
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def linear_to_srgb(v):
    v = min(1.0, max(0.0, v))
    return v * 12.92 if v <= 0.0031308 else 1.055 * v ** (1 / 2.4) - 0.055


def smoothstep(e0, e1, x):
    t = min(1.0, max(0.0, (x - e0) / (e1 - e0)))
    return t * t * (3 - 2 * t)


DEFAULTS = {"radius": 0.75, "softness": 0.45, "roundness": 0.6, "center": (0.5, 0.5), "color": (0.0, 0.0, 0.0)}


def vignette(rgb, x, y, width, height, amount, **params):
    p = {**DEFAULTS, **params}
    if amount <= 0:
        return list(rgb)
    u, v = (x + 0.5) / width, (y + 0.5) / height
    stretch = 1 + (width / height - 1) * p["roundness"]
    dx = (u - p["center"][0]) * 2 * stretch
    dy = (v - p["center"][1]) * 2  # (1-v)-(1-cy) == cy-v; the sign does not matter squared
    norm = max(math.hypot(stretch, 1.0), 1.0)
    shade = smoothstep(p["radius"], p["radius"] + p["softness"], math.hypot(dx, dy) / norm)
    weight = shade * amount
    out = []
    for source, colour in zip(rgb, p["color"]):
        lin = srgb_to_linear(source / 255.0)
        out.append(255.0 * linear_to_srgb(lin + (colour - lin) * weight))
    return out


def grid(width, height):
    return [[col * (width - 1) // 8, row * (height - 1) // 5] for row in range(6) for col in range(9)]


def worst(got, want):
    return max(abs(a - b) for g, w in zip(got, want) for a, b in zip(g[:3], w[:3]))


# ---- window access ----------------------------------------------------------------------------------

class Win:
    def __init__(self, lab, lifecycle):
        self.lab, self.lc = lab, lifecycle

    def op(self, request, timeout=120):
        ok, body = self.lab.op(request, timeout)
        return ok, body

    def get(self, target):
        return self.lab.send({"command": "get", "target": target})

    def node(self, target):
        answer = self.get(target)
        return answer.get("data") if answer.get("ok") else None

    def field(self, target, name):
        node = self.node(target) or {}
        for field in node.get("fields", []):
            if field["field"] == name:
                return field["value"]
        return None

    def set(self, target, field, value):
        return self.lab.send({"command": "set", "target": target, "field": field, "value": value})

    def invoke(self, target, action=None):
        return self.lab.invoke(target, action)

    def viewport(self):
        return self.lab.viewport()

    def undo_depth(self):
        ok, body = self.op({"op": "history.status"})
        return body["history"].get("undo_depth") if ok else None

    def shown_hash(self, frame, seconds=30):
        """The hash of the picture the monitor drew for `frame`, once it converged."""
        state = self.lc.settle(self.lab, frame, seconds)
        return ((state or {}).get("last_frame") or {}).get("hash")

    def repaint(self, frame):
        """Forces the monitor to draw `frame` anew (seek away and back), then returns its hash."""
        other = frame + 1
        self.lab.seek(other)
        self.shown_hash(other, 30)
        self.lab.seek(frame)
        return self.shown_hash(frame, 30)

    def render(self, frame, probes, width=WIDTH, height=HEIGHT):
        ok, body = self.op({"op": "scene.render", "scene": "proof", "frame": frame, "width": width, "height": height, "probes": probes})
        return body["frame"] if ok else None


def window_id(pid):
    """The on-screen window of `pid` (the largest layer-0 one), for `screencapture -l`."""
    script = (
        "import CoreGraphics\n"
        "let l = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as! [[String: Any]]\n"
        "var best = 0; var area = 0.0\n"
        "for w in l {\n"
        f"  if (w[kCGWindowOwnerPID as String] as? Int) == {pid} && (w[kCGWindowLayer as String] as? Int) == 0 {{\n"
        "    let b = w[kCGWindowBounds as String] as! [String: Double]\n"
        "    let a = (b[\"Width\"] ?? 0) * (b[\"Height\"] ?? 0)\n"
        "    if a > area { area = a; best = w[kCGWindowNumber as String] as! Int }\n"
        "  }\n"
        "}\n"
        "print(best)\n"
    )
    out = subprocess.run(["swift", "-e", script], capture_output=True, text=True).stdout.strip()
    return out if out and out != "0" else None


def capture(lab, path):
    """Native observation only (never an input): the window as the OS paints it."""
    lab.invoke("window.activate")
    time.sleep(0.8)
    wid = window_id(lab.process.pid)
    if not wid:
        return False
    run = subprocess.run(["screencapture", "-x", "-o", "-l", wid, path], capture_output=True, text=True)
    return run.returncode == 0 and os.path.exists(path)


def free_gib(path="/tmp"):
    stat = os.statvfs(path)
    return stat.f_bavail * stat.f_frsize / 1024 ** 3


def decode(ffmpeg, path, width, height):
    out = subprocess.run(
        [ffmpeg, "-v", "error", "-nostdin", "-i", str(path), "-vf",
         "scale=in_color_matrix=bt709:in_range=tv:out_range=pc:flags=accurate_rnd+full_chroma_int,format=rgb24", "-f", "rawvideo", "-"],
        capture_output=True, check=True).stdout
    size = width * height * 3
    return [out[i:i + size] for i in range(0, len(out), size)]


def pixel(frame, width, x, y):
    at = (y * width + x) * 3
    return [frame[at], frame[at + 1], frame[at + 2]]


def export(win, name, destination, frames_from, frames):
    before = job_ids(win)
    opened = win.invoke("flow.export_scene", "open")
    if not check(f"{name}: flow.export_scene opens", opened.get("ok"), opened):
        return None
    for field, value in (("scene", "proof"), ("preset", "fullhd"), ("destination", str(destination)), ("from", str(frames_from)), ("frames", str(frames))):
        answer = win.lab.send({"command": "set", "target": f"dialog.field.{field}", "field": "value", "value": value})
        if not answer.get("ok"):
            check(f"{name}: dialog.field.{field}", False, answer)
            return None
    submitted = win.invoke("dialog.submit")
    if not check(f"{name}: dialog.submit starts the export", submitted.get("ok"), submitted):
        return None
    deadline, new = time.time() + 30, set()
    while not new and time.time() < deadline:
        new = job_ids(win) - before
        time.sleep(0.1)
    if not check(f"{name}: one job", len(new) == 1, new):
        return None
    job = next(iter(new))
    deadline = time.time() + 600
    while time.time() < deadline:
        ok, info = win.op({"op": "job.get", "job": job})
        if ok and info["state"] in ("succeeded", "failed", "cancelled"):
            return info
        time.sleep(0.25)
    return None


def job_ids(win):
    found = set()

    def walk(value):
        if isinstance(value, dict):
            if isinstance(value.get("id"), str):
                found.add(value["id"])
            for child in value.values():
                walk(child)
        elif isinstance(value, list):
            for child in value:
                walk(child)

    ok, body = win.op({"op": "job.list"})
    walk(body if ok else {})
    return found


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--binary", default=str(ROOT / "target" / "debug" / "tessel-editor"))
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--json")
    parser.add_argument("--shot", help="also save a native screenshot of the window with the effect applied (observation only)")
    args = parser.parse_args()
    binary = pathlib.Path(args.binary).resolve()
    for path in (binary, ROOT / "runtime" / "dist" / "runtime.js", CORPUS / "bundle.json"):
        if not path.exists():
            print(f"missing prerequisite: {path}")
            return 2
    ffmpeg = shutil.which("ffmpeg")
    if not ffmpeg:
        print("missing prerequisite: ffmpeg")
        return 2
    if free_gib() < MIN_FREE_GIB:
        print(f"only {free_gib():.1f} GiB free; this proof needs {MIN_FREE_GIB}")
        return 2
    lifecycle = load_lifecycle()
    work = tempfile.mkdtemp(prefix="te-fx-", dir="/tmp")
    lab = None
    try:
        lab = lifecycle.Lab(str(binary), work, {"TESSEL_VIEWPORT_INSPECT": "1"})
        lab.wait_ready()
        lab.invoke("window.activate")
        time.sleep(0.8)
        win = Win(lab, lifecycle)
        folder = os.path.join(work, "ws", "fx")
        ok, body = win.op({"op": "project.create", "name": "fx", "sequence": "main", "rate": 24, "width": WIDTH, "height": HEIGHT,
                           "length": EDIT_FRAMES, "dir": folder})
        if not check("project.create", ok, body):
            return 1
        project_info = body
        ok, body = win.op({"op": "scene.attach", "id": "proof", "path": str(CORPUS / "bundle.json")})
        if not check("scene.attach of the corpus bundle", ok, body):
            return 1
        ok, loaded = win.op({"op": "scene.resources.resolve", "scene": "proof", "base": str(CORPUS)})
        if not check("scene.resources.resolve", ok, loaded) or not lifecycle.commit_job(lab, loaded, "corpus"):
            return 1
        check("set inspector.mode scene", win.set("inspector.mode", "value", "scene").get("ok"))
        ok, body = win.op({"op": "monitor.show_scene", "scene": "proof"})
        if not check("monitor.show_scene", ok, body):
            return 1

        # ---- baseline ----
        section = win.node("scene.fx") or {}
        check("the live tree has scene.fx with no clip and the add action",
              section.get("state", {}).get("clip_count") == 0 and any(a["action"] == "add:vignette" for a in section.get("actions", [])), section)
        points = grid(WIDTH, HEIGHT)
        lab.seek(FRAME)
        monitor_baseline = win.shown_hash(FRAME)
        base = win.render(FRAME, points)
        if not check("explicit baseline render read back", base and base.get("inspected"), base):
            return 1
        check("baseline: the monitor's picture is the render's picture", monitor_baseline == base["hash"], f"{monitor_baseline} vs {base['hash']}")
        baseline_probes = base["probes"]
        depth0 = win.undo_depth()

        # ---- create + intensity 1 ----
        added = win.invoke("scene.fx", "add:vignette")
        check("invoke scene.fx add:vignette", added.get("ok") and added["data"].get("changed") is True, added)
        clip = win.node("scene.fx.clip:fx-1") or {}
        check("scene.fx.clip:fx-1 exists with the catalog defaults",
              win.field("scene.fx.clip:fx-1", "intensity") == 0.7 and clip.get("state", {}).get("start_frame") == 0, clip.get("state"))
        check("one gesture, one history entry", win.undo_depth() == depth0 + 1, (depth0, win.undo_depth()))
        check("set intensity 1", win.set("scene.fx.clip:fx-1", "intensity", 1.0).get("ok"))
        check("two edits, two entries", win.undo_depth() == depth0 + 2, win.undo_depth())

        strong_hash = win.repaint(FRAME)
        strong = win.render(FRAME, points)
        want = [vignette(p[:3], x, y, WIDTH, HEIGHT, 1.0) for p, (x, y) in zip(baseline_probes, points)]
        delta = worst(strong["probes"], want)
        check(f"intensity 1: every probe of the grid equals the Python vignette (worst {delta:.1f})", delta <= TOLERANCE, delta)
        change = worst(strong["probes"], baseline_probes)
        check(f"intensity 1: the picture really changed (strongest change {change})", change >= 25, change)
        check("intensity 1: the monitor's picture is the render's picture", strong_hash == strong["hash"], f"{strong_hash} vs {strong['hash']}")
        check("intensity 1: the monitor differs from the baseline", strong_hash != monitor_baseline)
        frame_report = (win.viewport().get("last_frame") or {})
        check("the monitor's own frame counted the fx pass", "directed_scene_fx" in json.dumps(frame_report), sorted(frame_report))

        if args.shot:
            captured = capture(lab, args.shot)
            print(f"screenshot {'saved to ' + args.shot if captured else 'NOT captured (no on-screen window)'}", flush=True)

        # ---- intensity 0 is the baseline ----
        check("set intensity 0", win.set("scene.fx.clip:fx-1", "intensity", 0.0).get("ok"))
        zero_hash = win.repaint(FRAME)
        zero = win.render(FRAME, points)
        check("intensity 0 is the baseline's very pixels (hash, monitor and render)", zero_hash == monitor_baseline and zero["hash"] == base["hash"], f"{zero_hash} / {zero['hash']} vs {base['hash']}")

        # ---- moved centre ----
        check("set intensity 1 again", win.set("scene.fx.clip:fx-1", "intensity", 1.0).get("ok"))
        check("set center_x 1", win.set("scene.fx.clip:fx-1", "center_x", 1.0).get("ok"))
        check("set center_y 1", win.set("scene.fx.clip:fx-1", "center_y", 1.0).get("ok"))
        check("the sibling components kept their values", win.field("scene.fx.clip:fx-1", "center_x") == 1.0 and win.field("scene.fx.clip:fx-1", "intensity") == 1.0)
        moved_hash = win.repaint(FRAME)
        moved = win.render(FRAME, points)
        want_moved = [vignette(p[:3], x, y, WIDTH, HEIGHT, 1.0, center=(1.0, 1.0)) for p, (x, y) in zip(baseline_probes, points)]
        delta = worst(moved["probes"], want_moved)
        check(f"moved centre: every probe equals the Python vignette for the new centre (worst {delta:.1f})", delta <= TOLERANCE, delta)
        check("moved centre: differs from the centred picture", moved["hash"] != strong["hash"] and moved_hash == moved["hash"])

        # ---- refusal ----
        depth = win.undo_depth()
        refused = win.set("scene.fx.clip:fx-1", "intensity", 2.0)
        check("intensity 2 is the typed param_out_of_range", not refused.get("ok") and refused.get("error", {}).get("code") == "param_out_of_range", refused)
        notice = ((win.node("scene.panel") or {}).get("state") or {}).get("notice") or {}
        check("the refusal is on screen as the panel notice", notice.get("code") == "param_out_of_range", notice)
        check("the refusal changed nothing: value, history, picture",
              win.field("scene.fx.clip:fx-1", "intensity") == 1.0 and win.undo_depth() == depth and win.repaint(FRAME) == moved["hash"], win.undo_depth())
        overlap = win.invoke("scene.fx", "add:sparkle")
        check("an effect outside the catalog is refused by name", not overlap.get("ok"), overlap)

        # ---- undo / redo ----
        ok, body = win.op({"op": "history.undo"})
        check("undo steps back one entry (center_y)", ok and win.field("scene.fx.clip:fx-1", "center_y") == 0.5, body)
        ok, body = win.op({"op": "history.redo"})
        check("redo brings it back", ok and win.field("scene.fx.clip:fx-1", "center_y") == 1.0, body)
        while (win.undo_depth() or 0) > depth0:
            win.op({"op": "history.undo"})
        check("undo to the start removes the clip and the picture is the baseline again",
              (win.node("scene.fx") or {}).get("state", {}).get("clip_count") == 0 and win.repaint(FRAME) == monitor_baseline)
        while (win.undo_depth() or 0) < depth0 + 6 and win.op({"op": "history.redo"})[0]:
            pass
        check("redo restores the clip with its last values", win.field("scene.fx.clip:fx-1", "center_x") == 1.0 and win.field("scene.fx.clip:fx-1", "intensity") == 1.0)

        # ---- toggle hidden is the baseline too ----
        check("invoke toggle_hidden", win.invoke("scene.fx.clip:fx-1", "toggle_hidden").get("ok"))
        check("a hidden clip draws the baseline", win.repaint(FRAME) == monitor_baseline)
        check("invoke toggle_hidden again", win.invoke("scene.fx.clip:fx-1", "toggle_hidden").get("ok"))
        check("shown again, the picture is back", win.repaint(FRAME) == moved["hash"])

        # ---- persistence: save and reopen ----
        document = os.path.join(work, "saved", "fx.tessel.json")
        os.makedirs(os.path.dirname(document))
        ok, saved = win.op({"op": "project.save", "path": document})
        check("project.save", ok, saved)
        ok, body = win.op({"op": "scene.fx.edit", "scene": "proof", "ops": [{"op": "remove_clip", "clip": "fx-1"}]})
        check("the live project drops the clip after saving", ok and (win.node("scene.fx") or {}).get("state", {}).get("clip_count") == 0, body)
        reopened_ok, reopened = win.op({"op": "project.open", "path": document})
        check("project.open of the saved project", reopened_ok, reopened)
        section = win.node("scene.fx") or {}
        check("the reopened window shows the clip again", section.get("state", {}).get("clip_count") == 1, section)
        listed_ok, listed = win.op({"op": "scene.fx.list", "scene": "proof"})
        check("scene.fx.list after reopening finds the clip", listed_ok and listed.get("clip_count") == 1, listed)
        if listed_ok and listed.get("clip_count") == 1:
            values = listed["tracks"][0]["clips"][0]
            check("the clip kept its parameters through the file", values.get("params", {}).get("center") == {"x": 1.0, "y": 1.0}
                  and values["params"].get("intensity") == 1.0, values)
        check("the reopened clip node has the same values", win.field("scene.fx.clip:fx-1", "center_x") == 1.0 and win.field("scene.fx.clip:fx-1", "intensity") == 1.0)

        # ---- export through the dialog ----
        out = pathlib.Path(work) / "out"
        out.mkdir()
        from_frame, frames = 44, 8
        ok, _ = win.op({"op": "scene.fx.edit", "scene": "proof", "ops": [{"op": "remove_clip", "clip": "fx-1"}]})
        baseline_info = export(win, "baseline export", out / "base.mp4", from_frame, frames)
        ok, _ = win.op({"op": "scene.fx.edit", "scene": "proof", "ops": [
            {"op": "add_clip", "effect": "vignette", "start_frame": 0, "duration_frames": 48, "id": "fx-1", "params": {"intensity": 1.0}}]})
        fx_info = export(win, "fx export", out / "fx.mp4", from_frame, frames)
        if baseline_info and fx_info:
            check("both exports succeeded", baseline_info["state"] == "succeeded" and fx_info["state"] == "succeeded", (baseline_info.get("error"), fx_info.get("error")))
            base_frames = decode(ffmpeg, out / "base.mp4", 1920, 1080)
            fx_frames = decode(ffmpeg, out / "fx.mp4", 1920, 1080)
            check("eight frames each", len(base_frames) == len(fx_frames) == frames, (len(base_frames), len(fx_frames)))
            hashes_base = (baseline_info.get("result") or {}).get("frame_hashes", {}).get("list") or []
            hashes_fx = (fx_info.get("result") or {}).get("frame_hashes", {}).get("list") or []
            big = grid(1920, 1080)
            strongest = 0
            for index in range(min(len(base_frames), len(fx_frames))):
                edit_frame = from_frame + index
                inside = edit_frame < 48
                if hashes_base and hashes_fx:
                    check(f"export frame {edit_frame}: hash {'differs inside' if inside else 'equals outside'} the clip",
                          (hashes_base[index] != hashes_fx[index]) == inside, (hashes_base[index], hashes_fx[index]))
                worst_frame = 0
                for x, y in big:
                    before = pixel(base_frames[index], 1920, x, y)
                    after = pixel(fx_frames[index], 1920, x, y)
                    expected = vignette(before, x, y, 1920, 1080, 1.0) if inside else before
                    worst_frame = max(worst_frame, max(abs(a - b) for a, b in zip(after, expected)))
                    strongest = max(strongest, max(abs(a - b) for a, b in zip(after, before)))
                check(f"export frame {edit_frame}: decoded pixels equal the Python vignette over the decoded baseline (worst {worst_frame:.1f})",
                      worst_frame <= CODEC_TOLERANCE, worst_frame)
            check(f"the export comparison discriminates (strongest change {strongest})", strongest >= 25, strongest)
    finally:
        if lab is not None:
            try:
                lab.stop()
            except Exception:
                pass
        if args.keep:
            print(f"kept: {work}")
        else:
            shutil.rmtree(work, ignore_errors=True)

    total = len(checks)
    print(f"\n{total - len(failures)}/{total} checks passed")
    report = {"binary": h.binary_info(str(binary)), "git": h.git_state(), "checks": checks, "failed": failures}
    if args.json:
        pathlib.Path(args.json).write_text(json.dumps(report, indent=2))
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
