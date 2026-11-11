#!/usr/bin/env python3
"""F059/F016: the animated film grain in the REAL window: edited through the live semantic nodes, drawn
by the monitor's WebView, exported through the dialog. Only `op` / `get` / `set` / `invoke`: no pointer
and no coordinates (BL043 stays open).

    python3 scripts/directed-scene-proof/with-gpu-lock.py \\
        python3 scripts/scene-fx-grain-ui-proof.py [--binary target/debug/tessel-editor] [--keep] [--json FILE]

It reuses the plumbing of `scene-fx-ui-proof.py` (a private instance with a short isolated HOME over the
synthetic corpus, no private media, the same cleanup and report) and brings its own expectation: the
independent Python transcription of the origin's grain in `scripts/pilot-grain-corpus.py` (FNV-1a,
lowbias32, the hash, the triangular noise, the cell, the luma response, the linear mix). The expected
pixel is that code applied to the BASELINE picture's probes; nothing here is the Rust reference or
the GLSL.

What it checks, in order:
  * `invoke scene.fx add:filmGrain` creates the clip node with the origin catalog fields, a boolean
    `colored`, a `seed` and the key actions; a number written to the boolean is `invalid_value`;
  * an explicit seed: the plan's pass carries `seed_for(per-frame, seed, frame in clip)` and every probe
    of the monitor/render equals the Python grain of the baseline; the next frame is another grain;
    `colored` draws the three-channel noise;
  * keys by node actions only (`key:amount` captures at the monitor frame, never implicitly):
    linear, easing on the arrival key, move, remove-animation, each one history entry, each drawn
    like the Python grain at the amount the curve gives;
  * a gesture previews what it commits (one entry) and one undo restores it;
  * `project.save` + `project.open` bring the clip, keys and seed back (same plan);
  * `flow.export_scene` writes an MP4 whose decoded cell centres match the Python grain over the
    decoded baseline export (inside the clip), and are the baseline's pictures outside it.

Exit 0 only if every check passed; 1 on a failed check; 2 on a missing prerequisite.
"""
import argparse
import importlib.util
import os
import pathlib
import shutil
import signal
import sys
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / file)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


P = load("scene_fx_ui_proof", "scene-fx-ui-proof.py")
G = load("pilot_grain_corpus", "pilot-grain-corpus.py")
check = P.check

WIDTH, HEIGHT, EDIT_FRAMES, FRAME = P.WIDTH, P.HEIGHT, P.EDIT_FRAMES, P.FRAME
TOLERANCE = 3
CODEC_TOLERANCE = 16
CLIP = "scene.fx.clip:fx-1"
SEED = 77
DEFAULT_PARAMS = {"amount": 1.0, "size": 4.0, "luma": 0.7, "colored": False, "intensity": 1.0}


def expected(base_probes, points, height, params, seed, pass_amount=1.0):
    out = []
    for probe, (x, y) in zip(base_probes, points):
        pixel, _ = G.grain_pixel([int(c) for c in probe[:3]], x, y, height, params, pass_amount, seed)
        out.append(pixel)
    return out


def pass_of(win, frame):
    ok, body = win.op({"op": "scene.plan", "scene": "proof", "frame": frame})
    passes = body["plan"].get("fx", []) if ok else []
    return passes[0] if passes else None


def run(args, surfaces):
    binary = pathlib.Path(args.binary).resolve()
    for path in (binary, ROOT / "runtime" / "dist" / "runtime.js", P.CORPUS / "bundle.json"):
        if not path.exists():
            print(f"missing prerequisite: {path}")
            return 2
    ffmpeg = shutil.which("ffmpeg")
    if not ffmpeg:
        print("missing prerequisite: ffmpeg")
        return 2
    if P.free_gib() < P.MIN_FREE_GIB:
        print(f"only {P.free_gib():.1f} GiB free; this proof needs {P.MIN_FREE_GIB}")
        return 2
    lifecycle = P.load_lifecycle()
    work = tempfile.mkdtemp(prefix="te-fx-", dir="/tmp")
    lab, lab_pid = None, None
    try:
        lab = lifecycle.Lab(str(binary), work, {"TESSEL_VIEWPORT_INSPECT": "1"})
        lab_pid = lab.process.pid
        for signum in (signal.SIGTERM, signal.SIGINT):
            signal.signal(signum, lambda *_: (_ for _ in ()).throw(SystemExit(130)))
        lab.wait_ready()
        lab.invoke("window.activate")
        time.sleep(0.8)
        win = P.Win(lab, lifecycle)
        ok, body = win.op({"op": "project.create", "name": "grain", "sequence": "main", "rate": 24, "width": WIDTH,
                           "height": HEIGHT, "length": EDIT_FRAMES, "dir": os.path.join(work, "ws", "grain")})
        if not check("project.create", ok, body):
            return 1
        ok, body = win.op({"op": "scene.attach", "id": "proof", "path": str(P.CORPUS / "bundle.json")})
        if not check("scene.attach of the corpus bundle", ok, body):
            return 1
        ok, loaded = win.op({"op": "scene.resources.resolve", "scene": "proof", "base": str(P.CORPUS)})
        if not check("scene.resources.resolve", ok, loaded) or not lifecycle.commit_job(lab, loaded, "corpus"):
            return 1
        check("set inspector.mode scene", win.set("inspector.mode", "value", "scene").get("ok"))
        ok, body = win.op({"op": "monitor.show_scene", "scene": "proof"})
        if not check("monitor.show_scene", ok, body):
            return 1

        points = P.grid(WIDTH, HEIGHT)
        base_at = {f: win.render(f, points)["probes"] for f in (FRAME, FRAME + 1, FRAME + 5)}
        lab.seek(FRAME)
        monitor_baseline = win.shown_hash(FRAME)
        depth0 = win.undo_depth()

        # ---- the node ----
        added = win.invoke("scene.fx", "add:filmGrain")
        check("invoke scene.fx add:filmGrain", added.get("ok") and added["data"].get("changed") is True, added)
        node = win.node(CLIP) or {}
        fields = {f["field"]: f for f in node.get("fields", [])}
        check("the grain node lists the origin catalog fields", all(n in fields for n in ("amount", "size", "luma", "colored", "intensity", "seed")), sorted(fields))
        check("colored is a boolean field, amount a number", fields.get("colored", {}).get("type") == "boolean" and fields.get("amount", {}).get("type") != "boolean", fields.get("colored"))
        check("catalog defaults", win.field(CLIP, "amount") == 0.25 and win.field(CLIP, "size") == 1.4 and win.field(CLIP, "luma") == 0.7 and win.field(CLIP, "colored") is False)
        refused = win.set(CLIP, "colored", 1)
        check("a number written to the boolean is invalid_value", not refused.get("ok") and refused.get("error", {}).get("code") == "invalid_value", refused)
        refused = win.set(CLIP, "seed", -1)
        check("a negative seed is refused with a typed error", not refused.get("ok") and bool(refused.get("error", {}).get("code")), refused)
        check("one add, one history entry", win.undo_depth() == depth0 + 1, (depth0, win.undo_depth()))

        # ---- explicit seed, pixels against the Python grain ----
        for name, value in (("amount", 1.0), ("size", 4.0), ("seed", SEED)):
            check(f"set {name} {value}", win.set(CLIP, name, value).get("ok"))
        check("the seed field reads back", win.field(CLIP, "seed") == SEED, win.field(CLIP, "seed"))
        p0 = pass_of(win, FRAME)
        check("the plan pass carries seed_for(per-frame, seed, frame in clip)", p0 and p0.get("seed") == G.seed_for("per-frame", SEED, p0["frameInClip"]), p0)
        grain_hash = win.repaint(FRAME)
        got = win.render(FRAME, points)
        want = expected(base_at[FRAME], points, HEIGHT, DEFAULT_PARAMS, p0["seed"])
        delta = P.worst(got["probes"], want)
        check(f"every probe equals the Python grain of the baseline (worst {delta:.1f})", delta <= TOLERANCE, delta)
        change = P.worst(got["probes"], base_at[FRAME])
        check(f"the grain really changed the picture (strongest change {change})", change >= 15, change)
        check("the monitor's picture is the render's picture", grain_hash == got["hash"] and grain_hash != monitor_baseline)
        lab.seek(FRAME + 1)
        next_hash = win.shown_hash(FRAME + 1)
        p1 = pass_of(win, FRAME + 1)
        got1 = win.render(FRAME + 1, points)
        want1 = expected(base_at[FRAME + 1], points, HEIGHT, DEFAULT_PARAMS, p1["seed"])
        check("the next frame is another seed and another grain", p1["seed"] != p0["seed"] and next_hash == got1["hash"] and next_hash != grain_hash)
        delta = P.worst(got1["probes"], want1)
        check(f"the next frame equals the Python grain for its own seed (worst {delta:.1f})", delta <= TOLERANCE, delta)
        lab.seek(FRAME)

        # ---- colored ----
        check("set colored true", win.set(CLIP, "colored", True).get("ok") and win.field(CLIP, "colored") is True)
        colored = win.render(FRAME, points)
        want_c = expected(base_at[FRAME], points, HEIGHT, {**DEFAULT_PARAMS, "colored": True}, p0["seed"])
        delta = P.worst(colored["probes"], want_c)
        check(f"colored: every probe equals the Python three-channel grain (worst {delta:.1f})", delta <= TOLERANCE, delta)
        check("colored differs from the monochrome grain", colored["hash"] != got["hash"])
        check("set colored false", win.set(CLIP, "colored", False).get("ok"))

        # ---- keys: only by actions, never implicit ----
        depth = win.undo_depth()
        check("set amount 0 (the base only)", win.set(CLIP, "amount", 0.0).get("ok"))
        animation = (win.node(CLIP) or {}).get("state", {}).get("animation") or {}
        amount_entry = next((e for e in animation.get("params", []) if e.get("param") == "amount"), None)
        check("editing the base created no key (never implicit)", amount_entry is not None and amount_entry["key_count"] == 0 and not amount_entry["animated"], amount_entry)
        lab.seek(FRAME)
        win.shown_hash(FRAME)
        keyed = win.invoke(CLIP, "key:amount")
        check("key:amount captures a key at the monitor frame", keyed.get("ok") and keyed["data"].get("changed") is True, keyed)
        # A capture takes what the curve is at the monitor's frame (the base only while nothing is
        # keyed), so the second key starts at the first one's value and its value is then edited.
        lab.seek(FRAME + 10)
        win.shown_hash(FRAME + 10)
        keyed = win.invoke(CLIP, "key:amount")
        check("key:amount at frame 10 of the clip captures the curve's value there", keyed.get("ok") and keyed["data"].get("changed") is True, keyed)
        check("set field key:amount:10 puts the value 1 on that key", win.set(CLIP, "key:amount:10", 1.0).get("ok"))
        animation = (win.node(CLIP) or {}).get("state", {}).get("animation") or {}
        amount_entry = next((e for e in animation.get("params", []) if e.get("param") == "amount"), {})
        key_values = {k["frame"]: k["value"] for k in amount_entry.get("keys", [])}
        check("the amount has exactly the keys 0 -> 0 and 10 -> 1", key_values == {0: 0.0, 10: 1.0}, key_values)
        check("each gesture is one history entry (set + 2 captures + key edit)", win.undo_depth() == depth + 4, (depth, win.undo_depth()))
        lab.seek(FRAME + 5)
        win.shown_hash(FRAME + 5)

        def keyed_picture(label, amount, fic=5):
            plan_pass = pass_of(win, FRAME + 5)
            check(f"{label}: the plan samples amount {amount}", plan_pass and abs(plan_pass["params"]["amount"] - amount) < 1e-9, plan_pass and plan_pass["params"]["amount"])
            drawn = win.render(FRAME + 5, points)
            params = {**DEFAULT_PARAMS, "amount": amount}
            delta = P.worst(drawn["probes"], expected(base_at[FRAME + 5], points, HEIGHT, params, G.seed_for("per-frame", SEED, fic)))
            check(f"{label}: every probe equals the Python grain at that amount (worst {delta:.1f})", delta <= TOLERANCE, delta)
            check(f"{label}: the monitor draws the render", win.repaint(FRAME + 5) == drawn["hash"])
            return drawn["hash"]

        linear_hash = keyed_picture("linear keys", 0.5)
        depth = win.undo_depth()
        eased = win.invoke(CLIP, "easing:amount:10:easeOutCubic")
        check("easing on the arrival key", eased.get("ok") and eased["data"].get("changed") is True and win.undo_depth() == depth + 1, eased)
        eased_hash = keyed_picture("easeOutCubic arrival", 0.875)
        check("the easing changed the picture", eased_hash != linear_hash)
        moved = win.invoke(CLIP, "move_key:amount:10:20")
        check("move_key keeps the easing of the key", moved.get("ok") and moved["data"].get("changed") is True, moved)
        keyed_picture("moved key (t=0.25)", 1.0 - (1.0 - 0.25) ** 3)
        removed = win.invoke(CLIP, "remove_animation:amount")
        check("remove_animation is explicit and one entry", removed.get("ok") and removed["data"].get("changed") is True, removed)
        keyed_picture("animation removed (base 0)", 0.0)
        ok, body = win.op({"op": "history.undo"})
        check("ONE undo brings the animation back", ok, body)
        keyed_picture("undone", 1.0 - (1.0 - 0.25) ** 3)

        # ---- the key editors and the base under keys, by the nodes the panel's inputs share ----
        node = win.node(CLIP) or {}
        listed = {f["field"]: f for f in node.get("fields", [])}
        check("the node lists each key's frame and value and the shift of the curve",
              all(n in listed for n in ("key_frame:amount:0", "key:amount:0", "key_frame:amount:20", "key:amount:20", "shift_keys:amount")),
              sorted(listed))
        base = listed.get("amount", {})
        check("the base of an animated parameter is disabled, labelled and says what edits it",
              base.get("enabled") is False and base.get("animated") is True and base.get("edit_with") == "keys"
              and "(base)" in base.get("label", "") and base.get("disabled_reason"), base)
        depth = win.undo_depth()
        refused = win.set(CLIP, "amount", 0.3)
        check("a write to the animated base is refused with base_under_keys",
              not refused.get("ok") and refused.get("error", {}).get("code") == "base_under_keys" and win.undo_depth() == depth, refused)
        for bad in ("abc", "", "NaN"):
            refused = win.set(CLIP, "key:amount:20", bad)
            check(f"a key value `{bad}` is invalid_value and writes nothing",
                  not refused.get("ok") and refused.get("error", {}).get("code") == "invalid_value" and win.undo_depth() == depth, refused)
        refused = win.set(CLIP, "key_frame:amount:20", 1.5)
        check("a fractional key frame is refused and writes nothing", not refused.get("ok") and win.undo_depth() == depth, refused)
        here = lambda: (win.node("scene.fx") or {}).get("state", {}).get("playhead_frame")  # noqa: E731
        for to, want in (("first", 0), ("last", 20), ("previous", 0)):
            went = win.invoke(CLIP, f"go:amount:{to}")
            check(f"go:amount:{to} moves the monitor to clip frame {want} (edit frame {FRAME + want}) and writes nothing",
                  went.get("ok") and here() == FRAME + want and win.undo_depth() == depth, (went, here()))
        refused = win.invoke(CLIP, "go:amount:previous")
        check("no key before the first: no_previous_key, the monitor stays",
              not refused.get("ok") and refused.get("error", {}).get("code") == "no_previous_key" and here() == FRAME, refused)
        lab.seek(FRAME + 5)
        win.shown_hash(FRAME + 5)

        # ---- a gesture previews what it commits ----
        ok, began = win.op({"op": "scene.fx.gesture.begin", "scene": "proof", "clip": "fx-1"})
        gesture = began.get("gesture") if ok else None
        ok, body = win.op({"op": "scene.fx.gesture.update", "gesture": gesture,
                           "ops": [{"op": "set_param", "clip": "fx-1", "param": "size", "value": 2.0}]})
        check("gesture update", ok, body)
        candidate = win.repaint(FRAME + 5)
        ok, body = win.op({"op": "scene.fx.gesture.commit", "gesture": gesture})
        check("gesture commit", ok, body)
        check("the committed picture is the preview's", win.repaint(FRAME + 5) == candidate)

        # ---- save and reopen ----
        before_plan = pass_of(win, FRAME + 5)
        document = os.path.join(work, "saved", "grain.tessel.json")
        os.makedirs(os.path.dirname(document))
        ok, saved = win.op({"op": "project.save", "path": document})
        check("project.save", ok, saved)
        ok, body = win.op({"op": "scene.fx.edit", "scene": "proof", "ops": [{"op": "remove_clip", "clip": "fx-1"}]})
        check("the live project drops the clip after saving", ok and (win.node("scene.fx") or {}).get("state", {}).get("clip_count") == 0, body)
        ok, body = win.op({"op": "project.open", "path": document})
        check("project.open of the saved project", ok, body)
        check("the reopened plan is the saved plan (keys, easing, seed, params)", pass_of(win, FRAME + 5) == before_plan, (before_plan, pass_of(win, FRAME + 5)))
        check("the reopened node reads the seed back", win.field(CLIP, "seed") == SEED, win.field(CLIP, "seed"))

        # ---- export through the dialog ----
        out = pathlib.Path(work) / "out"
        out.mkdir()
        from_frame, frames, cell = 44, 8, 6
        ok, body = win.op({"op": "scene.fx.edit", "scene": "proof", "ops": [{"op": "remove_clip", "clip": "fx-1"}]})
        check("baseline export: the clip is removed", ok, body)
        base_info = P.export(win, "baseline export", out / "base.mp4", from_frame, frames)
        ok, body = win.op({"op": "scene.fx.edit", "scene": "proof", "ops": [
            {"op": "add_clip", "effect": "filmGrain", "start_frame": 0, "duration_frames": 48, "id": "fx-1",
             "params": {"amount": 1.0, "size": 4.0}},
            {"op": "key_param", "clip": "fx-1", "param": "amount", "frame": 40, "value": 0.5},
            {"op": "key_param", "clip": "fx-1", "param": "amount", "frame": 47, "value": 1.0},
            {"op": "set_seed", "clip": "fx-1", "seed": 2024}]})
        check("fx export: the keyed grain is added", ok, body)
        fx_info = P.export(win, "fx export", out / "fx.mp4", from_frame, frames)
        compared = judge_export(base_info, fx_info, out, ffmpeg, from_frame, frames, cell)
        check(f"coverage: {compared} of {frames} export frames were compared", compared == frames, compared)
        surfaces.append({"surface": "window", "how": "live semantic socket of a private instance: op/get/set/invoke, no pointer, no coordinates", "status": "exercised"})
        surfaces.append({"surface": "headless/lib", "how": "N/A in this script: crates/core/tests/scene_fx_keys.rs and crates/render/tests/scene_fx_export.rs", "status": "not run here"})
        surfaces.append({"surface": "pointer", "how": "N/A: BL043 stays open", "status": "not proved"})
    finally:
        if lab is not None and lab_pid is not None:
            P.stop_lab(lab, lab_pid)
        if args.keep:
            print(f"kept: {work}")
        else:
            shutil.rmtree(work, ignore_errors=True)
    return 1 if P.failures else 0


def judge_export(base_info, fx_info, out, ffmpeg, from_frame, frames, cell):
    """Counts the export frames compared; every way of not looking is a recorded FAIL."""
    for label, info in (("baseline export", base_info), ("fx export", fx_info)):
        if not check(f"{label}: succeeded", info is not None and info.get("state") == "succeeded", info and (info.get("error") or info.get("state"))):
            return 0
    hashes = []
    for label, info in (("baseline export", base_info), ("fx export", fx_info)):
        listed = ((info.get("result") or {}).get("frame_hashes") or {}).get("list") or []
        if not check(f"{label}: {frames} frame hashes", len(listed) == frames and all(isinstance(h, str) and h for h in listed), len(listed)):
            return 0
        hashes.append(listed)
    base = P.decode(ffmpeg, out / "base.mp4", 1920, 1080)
    grained = P.decode(ffmpeg, out / "fx.mp4", 1920, 1080)
    if not check(f"{frames} decoded frames each", len(base) == len(grained) == frames, (len(base), len(grained))):
        return 0
    points = [((x // cell) * cell + cell // 2, (y // cell) * cell + cell // 2) for x, y in P.grid(1920, 1080)]
    compared, strongest = 0, 0
    for index in range(frames):
        frame = from_frame + index
        inside = frame < 48
        check(f"export frame {frame}: hash {'differs inside' if inside else 'equals outside'} the clip", (hashes[0][index] != hashes[1][index]) == inside)
        amount = 0.5 + 0.5 * (min(frame, 47) - 40) / 7
        worst_frame = 0
        for x, y in points:
            before = P.pixel(base[index], 1920, x, y)
            after = P.pixel(grained[index], 1920, x, y)
            if inside:
                want, _ = G.grain_pixel(list(before), x, y, 1080, {**DEFAULT_PARAMS, "amount": amount}, 1.0, G.seed_for("per-frame", 2024, frame))
            else:
                want = before
            worst_frame = max(worst_frame, max(abs(a - b) for a, b in zip(after, want)))
            strongest = max(strongest, max(abs(a - b) for a, b in zip(after, before)))
        check(f"export frame {frame}: decoded pixels equal the Python grain over the decoded baseline (worst {worst_frame:.1f})", worst_frame <= CODEC_TOLERANCE, worst_frame)
        compared += 1
    check(f"the export comparison discriminates (strongest change {strongest})", strongest >= 15, strongest)
    return compared


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--binary", default=str(ROOT / "target" / "debug" / "tessel-editor"))
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--json")
    args = parser.parse_args()
    surfaces, code = [], 1
    try:
        code = run(args, surfaces)
    except (SystemExit, KeyboardInterrupt) as error:
        code = error.code if isinstance(error, SystemExit) and isinstance(error.code, int) and error.code != 0 else 130
        check("the proof was interrupted", False, f"{type(error).__name__}: {error}")
    except Exception as error:  # noqa: BLE001 - a crash is a recorded FAIL
        check("the proof ran to its end", False, f"{type(error).__name__}: {error}")
    finally:
        if P.failures and code == 0:
            code = 1
        print(f"\n{len(P.checks) - len(P.failures)}/{len(P.checks)} checks passed", flush=True)
        if args.json:
            P.write_report(pathlib.Path(args.json), pathlib.Path(args.binary).resolve(), surfaces, code)
    return code


if __name__ == "__main__":
    raise SystemExit(main())
