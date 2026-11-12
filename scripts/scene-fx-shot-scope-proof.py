#!/usr/bin/env python3
"""F062/F016 (P4a): effects with the scope `shot` in the REAL window: edited through the live semantic nodes,
drawn by the monitor's WebView, exported through the dialog. Only `op` / `get` / `set` / `invoke`: no pointer
and no coordinates (the pointer over the Kit select is not proved here; BL043 stays open).

    python3 scripts/scene-fx-shot-scope-proof.py [--binary target/debug/tessel-editor] \\
        [--headless target/debug/tessel-headless] [--keep] [--json FILE]

(Functional proof: it does not take the GPU lock, which is for performance measurements only.)

It reuses the plumbing of `scene-fx-ui-proof.py` (a private instance with a short isolated HOME over the
synthetic corpus, no private media, the same cleanup and report) and brings its own expectation: the
independent Python transcription of the vignette and of the fade of that script, applied to the BASELINE
picture's probes. Nothing here is the Rust reference or the GLSL. The corpus has two shots over one
continuous instance: `shot-1` = frames [0, 48) and `shot-2` = frames [48, 96), at 24 fps.

What it checks, in order:
  * the section node lists the scopes the build draws (`output`, `shot`), the shots of the edit and the shot
    the monitor shows, and offers `add_in_shot:<effect>` enabled only on a frame of a shot;
  * `invoke scene.fx add_in_shot:vignette` at monitor frame 12 adds ONE clip on `shot-1` (one history entry),
    cut by the window at the end of the shot (36 of the effect's 96 frames); its node has the `scope` choice
    (value `shot:shot-1`, options with applicable/reason: `shot-2` is `FX_SCOPE_MISMATCH`);
  * with a strong vignette (radius 0) the plan carries one pass with scope `{kind: shot, shotId: shot-1}` in
    frames 12 and 47 and none in 48 and 60, `requires` has `fxShotScope` only where there is a pass, the
    monitor and `scene.render` agree, every probe equals the Python vignette inside, and frames 48 and 60
    are the baseline's very pixels (a frame on each side of the boundary);
  * the same clip laid on the `output` draws the SAME picture inside it (a lone effect is stage-agnostic) and
    its plan has no `fxShotScope`; `set scope shot:shot-1` brings it back; each move is one entry; the same
    scope again is a no-op; undo/redo walk the entries;
  * the refusals are typed and write nothing (value, history, picture): `fx_scope_mismatch`, `unknown_shot`,
    `fx_scope_not_yet`, `invalid_value`, a retime out of the shot (`invalid_bundle`), an unknown clip, an unknown
    effect, and `add_in_shot` where the monitor shows no shot (`disabled_element`); each is the panel notice;
  * the handles of the vignette (F061, BL082.64): a vignette laid on a shot has none: its `handles` action is
    disabled with the core's reason, `invoke handles` and `scene.fx.select` are the typed `scope_not_supported`
    (the clip as target, the scope in `details`), the gizmo node keeps `target: node`, nothing is written; the
    same select through a real `tessel-headless` process answers the same code, and the clip back on the output
    has them again. (The header button greyed with that reason is the GPUI test's: this script has no pointer.)
  * the stages: an OUTPUT fade on the FIRST track and the shot's vignette on a LATER track are drawn
    vignette -> fade (the shot first, whatever the track order): every probe equals the Python composition
    in that order and differs from the flipped order;
  * `project.save` + `project.open` bring the scopes back (same plan);
  * `flow.export_scene` writes an MP4 whose decoded frames (44..51, across the boundary) equal the Python
    composition over the decoded baseline export, frame by frame (vignette only in frames 44..47).

Exit 0 only if every check passed; 1 on a failed check; 2 on a missing prerequisite.
"""
import argparse
import importlib.util
import json
import os
import pathlib
import shutil
import signal
import subprocess
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
check = P.check

WIDTH, HEIGHT, EDIT_FRAMES, FRAME = P.WIDTH, P.HEIGHT, P.EDIT_FRAMES, P.FRAME
TOLERANCE = P.TOLERANCE
CODEC_TOLERANCE = P.CODEC_TOLERANCE
SECTION = "scene.fx"
CLIP = "scene.fx.clip:fx-1"
FIRST, SECOND = "shot-1", "shot-2"
LAST_OF_FIRST, FIRST_OF_SECOND, IN_SECOND, STAGES_FRAME = 47, 48, 60, 20
# Wide enough that every probe of the grid is changed (the defaults leave the centre alone).
STRONG = {"radius": 0.0, "softness": 0.5}
WHITE = (1.0, 1.0, 1.0)


# ---- the independent expectation ------------------------------------------------------------------------

def vignette_of(probes, points, width=WIDTH, height=HEIGHT, **params):
    return [P.vignette(p[:3], x, y, width, height, 1.0, **params) for p, (x, y) in zip(probes, points)]


def staged(probes, points, flipped=False):
    """The shot's vignette, then the output's white fade at 0.5: the composition the stages define (or the
    flipped order, which the stages must NOT draw)."""
    out = []
    for p, (x, y) in zip(probes, points):
        if flipped:
            out.append(P.vignette(P.fade(p[:3], WHITE, 0.5), x, y, WIDTH, HEIGHT, 1.0, **STRONG))
        else:
            out.append(P.fade(P.vignette(p[:3], x, y, WIDTH, HEIGHT, 1.0, **STRONG), WHITE, 0.5))
    return out


def corpus_shots():
    """`(id, start, end_exclusive)` of the corpus bundle, read from the file the window attached."""
    bundle = json.loads((P.CORPUS / "bundle.json").read_text())
    return [(s["id"], s["editRange"]["startFrame"], s["editRange"]["endFrameExclusive"]) for s in bundle["shots"]]


def action_of(node, name):
    return next((a for a in (node or {}).get("actions", []) if a.get("action") == name), None)


def field_of(node, name):
    return next((f for f in (node or {}).get("fields", []) if f.get("field") == name), None)


def span_of(win, target):
    state = (win.node(target) or {}).get("state", {})
    return (state.get("start_frame"), state.get("end_frame_exclusive"))


def notice_code(win):
    return (((win.node("scene.panel") or {}).get("state") or {}).get("notice") or {}).get("code")


def plan_of(win, frame):
    ok, body = win.op({"op": "scene.plan", "scene": "proof", "frame": frame})
    return body["plan"] if ok else {}


def passes_of(win, frame):
    return plan_of(win, frame).get("fx", [])


def requires_of(win, frame):
    return plan_of(win, frame).get("requires", [])


def children_clips(win):
    return [c for c in (win.node(SECTION) or {}).get("children", []) if c.startswith("scene.fx.clip:")]


def new_clip_node(win, before):
    added = [c for c in children_clips(win) if c not in before]
    return added[0] if len(added) == 1 else None


def headless_refusal(headless, bundle):
    """The select of a vignette laid on a shot through a real `tessel-headless` (JSON Lines over a pipe: no window,
    no socket). `(answers, return code)`; the last two requests are the refusals, the first two build the scene."""
    requests = [
        {"op": "scene.attach", "id": "proof", "bundle": bundle},
        {"op": "scene.fx.edit", "scene": "proof", "ops": [
            {"op": "add_clip", "effect": "vignette", "start_frame": FRAME, "duration_frames": 36, "id": "v1", "scope": f"shot:{FIRST}"}]},
        {"op": "scene.fx.select", "scene": "proof", "clip": "v1"},
        {"op": "scene.fx.handles", "scene": "proof", "clip": "v1", "frame": FRAME},
        {"op": "scene.fx.edit", "scene": "proof", "ops": [{"op": "set_scope", "clip": "v1", "scope": "output"}]},
        {"op": "scene.fx.select", "scene": "proof", "clip": "v1"},
    ]
    done = subprocess.run([str(headless), "--empty"], input="".join(json.dumps(r) + "\n" for r in requests),
                          capture_output=True, text=True, timeout=120)
    return [json.loads(line) for line in done.stdout.splitlines() if line.strip()], done.returncode


def run(args, surfaces):
    binary = pathlib.Path(args.binary).resolve()
    headless = pathlib.Path(args.headless).resolve()
    for path in (binary, headless, ROOT / "runtime" / "dist" / "runtime.js", P.CORPUS / "bundle.json"):
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
        ok, body = win.op({"op": "project.create", "name": "shot", "sequence": "main", "rate": 24, "width": WIDTH,
                           "height": HEIGHT, "length": EDIT_FRAMES, "dir": os.path.join(work, "ws", "shot")})
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

        # ---- the baseline of the four frames the proof looks at ----
        points = P.grid(WIDTH, HEIGHT)
        looked = (FRAME, LAST_OF_FIRST, FIRST_OF_SECOND, IN_SECOND)
        base, monitor_base = {}, {}
        for frame in looked:
            lab.seek(frame)
            monitor_base[frame] = win.shown_hash(frame)
            base[frame] = win.render(frame, points)
        if not check("baseline: the four frames rendered and read back", all(base.get(f) and base[f].get("inspected") for f in looked), base):
            return 1
        check("baseline: the monitor's picture is the render's picture at every frame", all(monitor_base[f] == base[f]["hash"] for f in looked),
              {f: (monitor_base[f], base[f]["hash"]) for f in looked})
        check("baseline: no frame asks for the shot capability", all("fxShotScope" not in requires_of(win, f) for f in looked))

        # ---- the section says what a scope is and where the monitor is ----
        lab.seek(FRAME)
        win.shown_hash(FRAME)
        section = win.node(SECTION) or {}
        state = section.get("state", {})
        check("the section draws the scopes output and shot (node stays out)", state.get("scope") == ["output", "shot"], state.get("scope"))
        listed = [(s["id"], s["start_frame"], s["end_frame_exclusive"]) for s in state.get("shots", [])]
        check("the section lists the shots of the corpus edit", listed == corpus_shots(), (listed, corpus_shots()))
        check(f"the monitor shows frame {FRAME}, a frame of {FIRST}", state.get("shot_here") == FIRST and state.get("playhead_frame") == FRAME, state)
        add = action_of(section, "add_in_shot:vignette") or {}
        check("add_in_shot:vignette is enabled on a frame of a shot", add.get("enabled") is True and add.get("disabled_reason") is None, add)
        check("every catalog effect has its add_in_shot action", all(action_of(section, f"add_in_shot:{e}") for e in state.get("effects", [])), state.get("effects"))
        depth0 = win.undo_depth()

        # ---- add on the displayed shot ----
        added = win.invoke(SECTION, "add_in_shot:vignette")
        check("invoke scene.fx add_in_shot:vignette", added.get("ok") and added["data"].get("changed") is True, added)
        clip = win.node(CLIP) or {}
        state = clip.get("state", {})
        check(f"the clip is on {FIRST}, starts at the displayed frame and is cut at the end of the shot (36 of 96 frames)",
              state.get("scope") == f"shot:{FIRST}" and state.get("shot") == FIRST and state.get("start_frame") == FRAME
              and state.get("end_frame_exclusive") == 48 and state.get("duration_frames") == 36, state)
        check("one gesture, one history entry", win.undo_depth() == depth0 + 1, (depth0, win.undo_depth()))
        choice = field_of(clip, "scope") or {}
        check("the scope field is a choice that holds shot:shot-1", choice.get("type") == "choice" and choice.get("value") == f"shot:{FIRST}"
              and choice.get("default") == "output" and choice.get("at_default") is False, choice)
        options = {o["value"]: o for o in choice.get("options", [])}
        check("the options are the output, then each shot of the edit", list(options) == ["output", f"shot:{FIRST}", f"shot:{SECOND}"], list(options))
        check(f"{FIRST} holds the clip; {SECOND} does not (FX_SCOPE_MISMATCH, never enabled)",
              options.get(f"shot:{FIRST}", {}).get("applicable") is True and options.get(f"shot:{SECOND}", {}).get("applicable") is False
              and options.get(f"shot:{SECOND}", {}).get("reason") == "FX_SCOPE_MISMATCH", options)
        for name, value in (("intensity", 1.0), ("radius", STRONG["radius"]), ("softness", STRONG["softness"])):
            check(f"set {name} {value}", win.set(CLIP, name, value).get("ok"))
        check("four gestures, four entries", win.undo_depth() == depth0 + 4, win.undo_depth())

        # ---- the picture: only the shot, a frame on each side of the boundary ----
        hashes = {}
        for frame in (FRAME, LAST_OF_FIRST):
            shown = win.repaint(frame)
            got = win.render(frame, points)
            hashes[frame] = got["hash"]
            delta = P.worst(got["probes"], vignette_of(base[frame]["probes"], points, **STRONG))
            check(f"frame {frame} ({FIRST}): every probe equals the Python vignette of the baseline (worst {delta:.1f})", delta <= TOLERANCE, delta)
            change = P.worst(got["probes"], base[frame]["probes"])
            check(f"frame {frame}: the shot clip really changed the picture (strongest change {change})", change >= 25, change)
            check(f"frame {frame}: the monitor's picture is the render's picture and not the baseline's", shown == got["hash"] and shown != monitor_base[frame], (shown, got["hash"]))
        for frame in (FIRST_OF_SECOND, IN_SECOND):
            shown = win.repaint(frame)
            got = win.render(frame, points)
            check(f"frame {frame} ({SECOND}): the monitor draws the baseline's very picture", shown == monitor_base[frame], (shown, monitor_base[frame]))
            check(f"frame {frame}: the render is the baseline's picture, hash and probes", got["hash"] == base[frame]["hash"] and P.worst(got["probes"], base[frame]["probes"]) == 0, got["hash"])

        # ---- the plan and the reads say the same ----
        inside = passes_of(win, FRAME)
        check("the plan at frame 12 has ONE pass with scope shot/shot-1", len(inside) == 1 and inside[0].get("scope") == {"kind": "shot", "shotId": FIRST}, inside)
        needs = requires_of(win, FRAME)
        check("that plan requires fxShotScope (and outputFx)", "fxShotScope" in needs and "outputFx" in needs, needs)
        check("frame 47 still has the pass", len(passes_of(win, LAST_OF_FIRST)) == 1)
        for frame in (FIRST_OF_SECOND, IN_SECOND):
            check(f"frame {frame} has no pass and does not ask for the shot capability", passes_of(win, frame) == [] and "fxShotScope" not in requires_of(win, frame) and "outputFx" not in requires_of(win, frame), (passes_of(win, frame), requires_of(win, frame)))
        ok, read = win.op({"op": "scene.fx.frame", "scene": "proof", "frame": FRAME})
        check("scene.fx.frame at 12: the shot, one pass in the deck, none in the output",
              ok and (read.get("shot") or {}).get("id") == FIRST and read.get("stages") == {"deck": 1, "output": 0}, read)
        ok, read = win.op({"op": "scene.fx.frame", "scene": "proof", "frame": IN_SECOND})
        check("scene.fx.frame at 60: the other shot, no pass", ok and (read.get("shot") or {}).get("id") == SECOND and read.get("count") == 0, read)
        win.repaint(FRAME)
        frame_report = win.viewport().get("last_frame") or {}
        check("the monitor's own frame at 12 counted the one shot pass", (frame_report.get("source") or {}).get("directed_scene_fx_shot") == 1, frame_report.get("source"))
        win.repaint(IN_SECOND)
        frame_report = win.viewport().get("last_frame") or {}
        check("the monitor's own frame at 60 counted no shot pass", "directed_scene_fx_shot" not in (frame_report.get("source") or {}), frame_report.get("source"))

        # ---- the output scope draws the same picture inside the clip ----
        depth = win.undo_depth()
        moved = win.invoke(CLIP, "set_scope:output")
        check("invoke set_scope:output", moved.get("ok") and moved["data"].get("changed") is True, moved)
        check("one move, one entry", win.undo_depth() == depth + 1, (depth, win.undo_depth()))
        as_output = passes_of(win, FRAME)
        check("the plan pass is now scope output and the plan no longer asks for fxShotScope",
              len(as_output) == 1 and as_output[0].get("scope") == {"kind": "output"} and "fxShotScope" not in requires_of(win, FRAME), (as_output, requires_of(win, FRAME)))
        check("the node says output and the move to the shot is enabled again",
              win.field(CLIP, "scope") == "output" and (action_of(win.node(CLIP), f"set_scope:shot:{FIRST}") or {}).get("enabled") is True)
        output_hash = win.repaint(FRAME)
        check("a lone effect draws the SAME picture on the output as on its shot (hash)", output_hash == hashes[FRAME] and win.render(FRAME, points)["hash"] == hashes[FRAME], (output_hash, hashes[FRAME]))
        back = win.set(CLIP, "scope", f"shot:{FIRST}")
        check("set scope shot:shot-1 (the text of the scope) brings it back", back.get("ok") and back["data"].get("changed") is True and win.undo_depth() == depth + 2, back)
        check("the plan pass is the shot's again", (passes_of(win, FRAME) or [{}])[0].get("scope") == {"kind": "shot", "shotId": FIRST})
        again = win.set(CLIP, "scope", f"shot:{FIRST}")
        check("the scope the clip already holds is a no-op: no entry", again.get("ok") and again["data"].get("changed") is False and win.undo_depth() == depth + 2, again)
        ok, body = win.op({"op": "history.undo"})
        check("undo restores the output scope", ok and win.field(CLIP, "scope") == "output", body)
        ok, body = win.op({"op": "history.undo"})
        check("a second undo restores the shot scope", ok and win.field(CLIP, "scope") == f"shot:{FIRST}" and win.undo_depth() == depth, body)
        ok, body = win.op({"op": "history.redo"})
        check("redo brings the output scope back", ok and win.field(CLIP, "scope") == "output", body)
        ok, body = win.op({"op": "history.redo"})
        check("a second redo brings the shot scope back and the same picture", ok and win.field(CLIP, "scope") == f"shot:{FIRST}" and win.repaint(FRAME) == hashes[FRAME], body)

        # ---- the refusals: typed, on the panel, nothing written ----
        depth = win.undo_depth()
        span = span_of(win, CLIP)
        for text, code in ((f"shot:{SECOND}", "fx_scope_mismatch"), ("shot:nowhere", "unknown_shot"), ("node:anything", "fx_scope_not_yet"),
                           ("banana", "invalid_value"), ("", "invalid_value")):
            refused = win.set(CLIP, "scope", text)
            check(f"set scope `{text}` is the typed {code}", not refused.get("ok") and refused.get("error", {}).get("code") == code, refused)
            check(f"`{text}`: the refusal is the panel's notice", notice_code(win) == code, notice_code(win))
            by_action = win.invoke(CLIP, f"set_scope:{text}")
            check(f"set_scope:`{text}` by invoke is the same {code}", not by_action.get("ok") and by_action.get("error", {}).get("code") == code, by_action)
            check(f"`{text}`: nothing was written (history, span, scope, picture)",
                  win.undo_depth() == depth and span_of(win, CLIP) == span and win.field(CLIP, "scope") == f"shot:{FIRST}" and win.repaint(FRAME) == hashes[FRAME],
                  (win.undo_depth(), span_of(win, CLIP)))
        for label, value in (("a number", 1), ("a switch", True)):
            refused = win.set(CLIP, "scope", value)
            check(f"{label} is not a scope: invalid_value", not refused.get("ok") and refused.get("error", {}).get("code") == "invalid_value", refused)
        retime = win.set(CLIP, "duration_frames", 60)
        check("a retime that leaves the shot is refused whole by the importer (invalid_bundle)", not retime.get("ok") and retime.get("error", {}).get("code") == "invalid_bundle", retime)
        check("the retime wrote nothing and clamped nothing", win.undo_depth() == depth and span_of(win, CLIP) == span, (win.undo_depth(), span_of(win, CLIP)))
        missing = win.invoke("scene.fx.clip:nope", "set_scope:output")
        check("a clip that is not there is unknown_element", not missing.get("ok") and missing.get("error", {}).get("code") == "unknown_element", missing)
        unknown = win.invoke(SECTION, "add_in_shot:sepia")
        check("an effect the catalog does not have is refused by the core (unknown_effect)", not unknown.get("ok") and unknown.get("error", {}).get("code") == "unknown_effect", unknown)

        # ---- the handles of the vignette (F061): none on a shot, the core's typed refusal everywhere ----
        depth = win.undo_depth()
        off = action_of(win.node(CLIP), "handles") or {}
        check("the `handles` action of a vignette on a shot is disabled, with the reason of the core",
              off.get("enabled") is False and "plano" in (off.get("disabled_reason") or ""), off)
        by_action = win.invoke(CLIP, "handles")
        check("invoke handles on it is scope_not_supported (nothing selected)",
              not by_action.get("ok") and by_action.get("error", {}).get("code") == "scope_not_supported", by_action)
        ok, body = win.op({"op": "scene.fx.select", "scene": "proof", "clip": "fx-1"})
        check("scene.fx.select of it is the typed scope_not_supported: the clip is the target, the scope is in `details`",
              not ok and body.get("code") == "scope_not_supported" and body.get("target") == "fx-1"
              and (body.get("details") or {}).get("scope") == f"shot:{FIRST}", body)
        ok, body = win.op({"op": "scene.fx.handles", "scene": "proof", "clip": "fx-1", "frame": FRAME})
        check("scene.fx.handles of it says the same", not ok and body.get("code") == "scope_not_supported", body)
        gizmo = (win.node("monitor.gizmo") or {}).get("state", {})
        check("the monitor's gizmo keeps the node as its target", gizmo.get("target") == "node" and gizmo.get("fx_clip") == "", gizmo)
        check("the refusals wrote nothing", win.undo_depth() == depth, (depth, win.undo_depth()))
        headless_answers, headless_code = headless_refusal(headless, json.loads((P.CORPUS / "bundle.json").read_text()))
        if not check("tessel-headless answered every line",
                     len(headless_answers) == 6, {"answers": len(headless_answers), "exit": headless_code}):
            return 1
        select, read, output_select = headless_answers[2], headless_answers[3], headless_answers[5]
        check("through tessel-headless: scene.fx.select of the same vignette is scope_not_supported",
              select.get("ok") is False and select["error"].get("code") == "scope_not_supported"
              and select["error"].get("target") == "v1" and select["error"]["details"].get("scope") == f"shot:{FIRST}", select)
        check("through tessel-headless: scene.fx.handles says the same", read.get("ok") is False and read["error"].get("code") == "scope_not_supported", read)
        check("through tessel-headless: back on the output the select works again, and the process exits 1 for the two refusals",
              output_select.get("ok") is True and headless_code == 1, (output_select, headless_code))
        moved = win.invoke(CLIP, "set_scope:output")
        check("the vignette laid back on the output has its handles again (action enabled, no reason)",
              moved.get("ok") and (action_of(win.node(CLIP), "handles") or {}).get("enabled") is True
              and (action_of(win.node(CLIP), "handles") or {}).get("disabled_reason") is None, moved)
        shown = win.invoke(CLIP, "handles")
        check("and `handles` shows them: the gizmo is the vignette's", shown.get("ok") and (win.node("monitor.gizmo") or {}).get("state", {}).get("fx_clip") == "fx-1", shown)
        hidden = win.invoke(CLIP, "handles")
        check("`handles` again hides them", hidden.get("ok") and (win.node("monitor.gizmo") or {}).get("state", {}).get("target") == "node", hidden)
        back = win.invoke(CLIP, f"set_scope:shot:{FIRST}")
        check("the vignette is on its shot again for what follows", back.get("ok") and win.field(CLIP, "scope") == f"shot:{FIRST}", back)

        # ---- a shot on the second shot, cut at its end ----
        lab.seek(IN_SECOND)
        win.shown_hash(IN_SECOND)
        check(f"the monitor shows frame {IN_SECOND}, a frame of {SECOND}", (win.node(SECTION) or {}).get("state", {}).get("shot_here") == SECOND)
        before = set(children_clips(win))
        added = win.invoke(SECTION, "add_in_shot:vignette")
        second = new_clip_node(win, before)
        check("add_in_shot on the second shot adds one clip", added.get("ok") and second is not None, (added, second))
        if second is not None:
            st = (win.node(second) or {}).get("state", {})
            check(f"it is on {SECOND}, at frame 60, cut at the end of the shot (36 of 96 frames)",
                  st.get("scope") == f"shot:{SECOND}" and st.get("start_frame") == IN_SECOND and st.get("end_frame_exclusive") == 96, st)
            refused = win.set(second, "scope", f"shot:{FIRST}")
            check("it does not fit the first shot: fx_scope_mismatch", not refused.get("ok") and refused.get("error", {}).get("code") == "fx_scope_mismatch", refused)
            removed = win.invoke(second, "remove")
            check("it is removed again (one entry)", removed.get("ok"), removed)
        lab.seek(FRAME)
        win.shown_hash(FRAME)

        # ---- the stages: the shot first, whatever the track order ----
        gone = win.invoke(CLIP, "remove")
        check("remove the vignette to rebuild the stack in the order that matters", gone.get("ok"), gone)
        before = set(children_clips(win))
        added = win.invoke(SECTION, "add:fade")
        fade_node = new_clip_node(win, before)
        check("add the output fade FIRST (it takes the first track)", added.get("ok") and fade_node is not None, (added, fade_node))
        before = set(children_clips(win))
        added = win.invoke(SECTION, "add_in_shot:vignette")
        shot_node = new_clip_node(win, before)
        check("then the shot's vignette (it overlaps the fade, so it takes a LATER track)", added.get("ok") and shot_node is not None, (added, shot_node))
        if fade_node is None or shot_node is None:
            return 1
        for name, value in (("radius", STRONG["radius"]), ("softness", STRONG["softness"]), ("intensity", 1.0)):
            check(f"vignette: set {name} {value}", win.set(shot_node, name, value).get("ok"))
        for name, value in (("color_r", 1.0), ("color_g", 1.0), ("color_b", 1.0), ("amount", 0.5)):
            check(f"fade: set {name} {value}", win.set(fade_node, name, value).get("ok"))
        ok, layer = win.op({"op": "scene.fx.list", "scene": "proof"})
        order = [(t["id"], [c["effect"] for c in t["clips"]]) for t in layer.get("tracks", [])] if ok else []
        check("the fade is on the first track and the shot's vignette on the second", [effects for _, effects in order] == [["fade"], ["vignette"]], order)
        stacked = passes_of(win, STAGES_FRAME)
        check("the plan draws the vignette (shot) BEFORE the fade (output), against the track order",
              [(p["effect"], p["scope"].get("kind")) for p in stacked] == [("vignette", "shot"), ("fade", "output")], stacked)
        ok, read = win.op({"op": "scene.fx.frame", "scene": "proof", "frame": STAGES_FRAME})
        check("scene.fx.frame counts one pass in each stage", ok and read.get("stages") == {"deck": 1, "output": 1}, read)
        lab.seek(STAGES_FRAME)
        win.shown_hash(STAGES_FRAME)
        stacked_first = win.render(STAGES_FRAME, points)
        # The baseline of this frame comes from the clips being hidden: ask the render for it directly.
        for node in (shot_node, fade_node):
            check(f"hide {node}", win.set(node, "hidden", True).get("ok"))
        clean = win.render(STAGES_FRAME, points)
        for node in (shot_node, fade_node):
            check(f"show {node}", win.set(node, "hidden", False).get("ok"))
        shown = win.repaint(STAGES_FRAME)
        got = win.render(STAGES_FRAME, points)
        want = staged(clean["probes"], points)
        flipped = staged(clean["probes"], points, flipped=True)
        delta = P.worst(got["probes"], want)
        check(f"every probe equals the Python composition vignette -> fade (worst {delta:.1f})", delta <= TOLERANCE * 2, delta)
        gap = P.worst(want, flipped)
        check(f"the flipped order is another picture (the check discriminates: {gap:.0f})", gap >= 25, gap)
        check("the picture is not the flipped order's", P.worst(got["probes"], flipped) > TOLERANCE * 2, P.worst(got["probes"], flipped))
        check("the monitor's picture is the render's picture and not the clean one", shown == got["hash"] and got["hash"] != clean["hash"], (shown, got["hash"], clean["hash"]))
        check("the render after hiding and showing the clips equals the first (the same frame twice)", stacked_first["hash"] == got["hash"], (stacked_first["hash"], got["hash"]))

        # ---- persistence: save and reopen ----
        before_plan = passes_of(win, STAGES_FRAME)
        document = os.path.join(work, "saved", "shot.tessel.json")
        os.makedirs(os.path.dirname(document))
        ok, saved = win.op({"op": "project.save", "path": document})
        check("project.save", ok, saved)
        ok, body = win.op({"op": "scene.fx.edit", "scene": "proof", "ops": [{"op": "remove_clip", "clip": c.split(":", 1)[1]} for c in children_clips(win)]})
        check("the live project drops the clips after saving", ok and (win.node(SECTION) or {}).get("state", {}).get("clip_count") == 0, body)
        ok, body = win.op({"op": "project.open", "path": document})
        check("project.open of the saved project", ok, body)
        check("the reopened plan is the saved plan (scopes, order, params)", passes_of(win, STAGES_FRAME) == before_plan, (before_plan, passes_of(win, STAGES_FRAME)))
        check("the reopened nodes read the scopes back", win.field(shot_node, "scope") == f"shot:{FIRST}" and win.field(fade_node, "scope") == "output",
              (win.field(shot_node, "scope"), win.field(fade_node, "scope")))

        # ---- export through the dialog, a frame on each side of the boundary ----
        out = pathlib.Path(work) / "out"
        out.mkdir()
        from_frame, frames = 44, 8
        ok, body = win.op({"op": "scene.fx.edit", "scene": "proof", "ops": [{"op": "remove_clip", "clip": c.split(":", 1)[1]} for c in children_clips(win)]})
        check("baseline export: the clips are removed before exporting", ok, body)
        base_info = P.export(win, "baseline export", out / "base.mp4", from_frame, frames)
        ok, body = win.op({"op": "scene.fx.edit", "scene": "proof", "ops": [
            {"op": "add_clip", "effect": "fade", "start_frame": 40, "duration_frames": 20, "id": "fx-fade", "params": {"amount": 0.5}},
            {"op": "add_clip", "effect": "vignette", "start_frame": 0, "duration_frames": 48, "id": "fx-shot", "scope": f"shot:{FIRST}", "params": {"intensity": 1.0}}]})
        check("fx export: the output fade (first track) and the shot's vignette (a later track) are added", ok, body)
        for name in ("color_r", "color_g", "color_b"):
            check(f"fx export: fade {name} 1", win.set("scene.fx.clip:fx-fade", name, 1.0).get("ok"))
        fx_info = P.export(win, "fx export", out / "fx.mp4", from_frame, frames)
        compared = judge_export(base_info, fx_info, out, from_frame, frames, lambda path: P.decode(ffmpeg, path, 1920, 1080))
        check(f"coverage: {compared} of {frames} export frames were compared", compared == frames, compared)

        # ---- last, because the monitor is asked for a time the edit does not hold: no shot there ----
        depth = win.undo_depth()
        lab.seek(EDIT_FRAMES)
        past = win.node(SECTION) or {}
        check("the monitor past the end of the edit shows no shot", past.get("state", {}).get("shot_here") is None and past.get("state", {}).get("playhead_frame") == EDIT_FRAMES, past.get("state"))
        off = action_of(past, "add_in_shot:vignette") or {}
        check("add_in_shot is disabled there, with its reason", off.get("enabled") is False and bool(off.get("disabled_reason")), off)
        refused = win.invoke(SECTION, "add_in_shot:vignette")
        check("invoking it anyway is disabled_element, on the section, writing nothing",
              not refused.get("ok") and refused.get("error", {}).get("code") == "disabled_element" and (refused.get("error") or {}).get("target") == SECTION and win.undo_depth() == depth, refused)
        check("that refusal is the panel's notice", notice_code(win) == "disabled_element", notice_code(win))
        surfaces.append({"surface": "window", "how": "live semantic socket of a private instance: op/get/set/invoke, no pointer, no coordinates", "status": "exercised"})
        surfaces.append({"surface": "headless", "how": "a real tessel-headless process, JSON Lines: only the refusal of the handles of a shot clip (scope_not_supported); the rest is N/A here, see crates/core/tests/scene_fx_shot_scope.rs (incl. the process) and crates/render/tests/scene_fx_shot_scope.rs", "status": "exercised (the refusal only)"})
        surfaces.append({"surface": "pointer", "how": "N/A: BL043 stays open; the GPUI tests emit the Kit select's confirmation and click the add buttons", "status": "not proved"})
    finally:
        if lab is not None and lab_pid is not None:
            P.stop_lab(lab, lab_pid)
        if args.keep:
            print(f"kept: {work}")
        else:
            shutil.rmtree(work, ignore_errors=True)
    return 1 if P.failures else 0


def judge_export(base_info, fx_info, out, from_frame, frames, decode, big=None):
    """Counts the export frames compared; every way of not looking is a recorded FAIL. `decode(path)` gives
    the decoded RGB frames (injected, so the spec needs no FFmpeg); `big` the probe points of the frame."""
    for label, info in (("baseline export", base_info), ("fx export", fx_info)):
        if not check(f"{label}: succeeded", info is not None and info.get("state") == "succeeded", info and (info.get("error") or info.get("state"))):
            return 0
    lists = []
    for label, info in (("baseline export", base_info), ("fx export", fx_info)):
        listed = ((info.get("result") or {}).get("frame_hashes") or {}).get("list") or []
        if not check(f"{label}: {frames} frame hashes", len(listed) == frames and all(isinstance(h, str) and h for h in listed), len(listed)):
            return 0
        lists.append(listed)
    for path in (out / "base.mp4", out / "fx.mp4"):
        if not check(f"{path.name}: the file exists and is not empty", path.exists() and path.stat().st_size > 0, str(path)):
            return 0
    plain = decode(out / "base.mp4")
    drawn = decode(out / "fx.mp4")
    if not check(f"{frames} decoded frames each", len(plain) == len(drawn) == frames, (len(plain), len(drawn))):
        return 0
    big = big or P.grid(1920, 1080)
    compared, strongest, order_gap = 0, 0, 0
    for index in range(frames):
        frame = from_frame + index
        in_shot = frame < 48
        check(f"export frame {frame}: the picture differs from the baseline's (the fade covers 40..59)", lists[0][index] != lists[1][index])
        worst_frame = 0
        for x, y in big:
            before = P.pixel(plain[index], 1920, x, y)
            after = P.pixel(drawn[index], 1920, x, y)
            seen = P.vignette(before, x, y, 1920, 1080, 1.0) if in_shot else before
            want = P.fade(seen, WHITE, 0.5)
            worst_frame = max(worst_frame, max(abs(a - b) for a, b in zip(after, want)))
            strongest = max(strongest, max(abs(a - b) for a, b in zip(after, before)))
            if in_shot:
                flipped = P.vignette(P.fade(before, WHITE, 0.5), x, y, 1920, 1080, 1.0)
                order_gap = max(order_gap, max(abs(a - b) for a, b in zip(want, flipped)))
        where = "vignette (shot-1) then fade (output)" if in_shot else "fade only (the shot ended at 48)"
        check(f"export frame {frame}: decoded pixels equal the Python {where} over the decoded baseline (worst {worst_frame:.1f})", worst_frame <= CODEC_TOLERANCE, worst_frame)
        compared += 1
    check(f"the export comparison discriminates (strongest change {strongest})", strongest >= 25, strongest)
    check(f"the export comparison tells the stage order apart (gap {order_gap:.0f})", order_gap >= 15, order_gap)
    return compared


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--binary", default=str(ROOT / "target" / "debug" / "tessel-editor"))
    parser.add_argument("--headless", default=str(ROOT / "target" / "debug" / "tessel-headless"))
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
