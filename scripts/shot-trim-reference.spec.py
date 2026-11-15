"""Tests of the independent reference of the P5 shot durations and cuts (S0, BL082.5.1, F065).

    python scripts/shot-trim-reference.spec.py        # also under `python -O`

No editor, GPU, runtime page, engine or origin execution is needed. Layers, none of which compares the reference
only with itself:

1. HAND VECTORS written here as literals (the origin decisions #3, #6, #8, #9, #10, #11, #12, #13, #15, #18, #21,
   #28 and the D09 key cases, worked by hand on documents built in this file).
2. An INDEPENDENT VALIDITY CHECK of a document (`independent_problems`, written here from the origin rules, not
   the reference's `validate_document`) and the PROPERTY of T21: seeded random sequences of ops on seeded random
   documents; an accepted result is valid by that check, a refusal leaves the document as it was, a batch equals
   its steps, and an op placed AT the `limitFrame` of a refusal does not refuse with the same code (#33).
3. NEGATIVE CONTROLS: mistakes a port could make (a ripple that does not move the shots after it, a tail that does
   not prune keys, ...) are applied to a fresh copy of the reference; the battery must report them. Some run with
   the reference's own gate blinded, to prove the independent check does the catching.
4. The stored corpus and manifest, the audit against the `DEFERRED_FIELDS` list, `--check` and its tamper
   controls, the Python 3.9 syntax floor, the absence of a bare `assert` and of machine data.

The spec REFUSES a Microsoft Store stub as the interpreter (a refusal is never a skip), prints `sys.version`, and
is written with `unittest` assertions, never a bare `assert`, so it still tests under `python -O`.
"""
import ast
import copy
import hashlib
import importlib.util
import io
import json
import random
import re
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
REFERENCE_PATH = HERE / "shot-trim-reference.py"
FIXTURES = ROOT / "docs/migration/fixtures/shot-trim"
MAX_EDIT = 216000


def load_reference():
    """A FRESH module each call, so a negative control can patch its functions without touching the others."""
    loader = importlib.util.spec_from_file_location("shot_trim_reference_copy", REFERENCE_PATH)
    module = importlib.util.module_from_spec(loader)
    loader.loader.exec_module(module)
    return module


R = load_reference()
_problem = R.interpreter_problem()
if _problem:
    sys.stderr.write("INTERPRETER_NOT_REAL: %s\n" % _problem)
    raise SystemExit(R.EXIT_BAD_INTERPRETER)
sys.stderr.write("python: %s\n" % sys.version.replace("\n", " "))

STORED_CASES = json.loads((FIXTURES / "cases.json").read_text(encoding="utf-8"))
STORED_MANIFEST = json.loads((FIXTURES / "manifest.json").read_text(encoding="utf-8"))


# ---------------------------------------------------------------------------------------------------------
# Documents written for this spec (not imported from the reference)
# ---------------------------------------------------------------------------------------------------------

CUT = {"type": "cut", "reason": "scene change"}
CONT = {"type": "continuous"}


def make_doc(shots, instances, tracks=None, transitions=None, defs=None, extra=None):
    """`shots`: `(id, start, end, instance, instance_start, camera_in[, join])`; `instances`: `(id, frames, locked)`."""
    out = []
    for index, item in enumerate(shots):
        join = item[6] if len(item) > 6 else (None if index == 0 else CUT)
        entry = {
            "id": item[0], "sceneInstanceId": item[3], "editRange": {"startFrame": item[1], "endFrameExclusive": item[2]},
            "instanceStartFrame": item[4], "cameraClipId": "cam", "cameraInFrame": item[5], "intent": "i " + item[0],
        }
        if join is not None:
            entry["transitionInto"] = dict(join)
        out.append(entry)
    doc = {
        "editDurationFrames": max(item[2] for item in shots),
        "fps": {"numerator": 24, "denominator": 1},
        "shots": out,
        "sceneInstances": [dict(id=i[0], sceneDefinitionId="d-" + i[0], durationFrames=i[1], **({"editLocked": True} if i[2] else {})) for i in instances],
        "cameraClips": [{"id": "cam", "durationFrames": 300}],
        "sceneDefinitions": defs or [],
        "fx": {"tracks": tracks or [], "transitions": transitions or []},
    }
    doc.update(extra or {})
    return doc


def three(inst_b=(0, 0), durations=(60, 60, 60), **kw):
    """a [0,30) b [30,60) c [60,90), one instance each; b starts `inst_b` frames into its scene and camera clip."""
    return make_doc(
        [("a", 0, 30, "ia", 0, 0), ("b", 30, 60, "ib", inst_b[0], inst_b[1]), ("c", 60, 90, "ic", 0, 0)],
        [("ia", durations[0], False), ("ib", durations[1], False), ("ic", durations[2], False)], **kw)


def clip(clip_id, start, frames, shot=None, keys=None, fade_in=0, fade_out=0):
    out = {"id": clip_id, "startFrame": start, "durationFrames": frames}
    if shot:
        out["scope"] = {"kind": "shot", "shotId": shot}
    if keys:
        out["keys"] = {"p": [{"frame": f, "value": 1, "easing": "linear"} for f in keys]}
    if fade_in:
        out["fadeInFrames"] = fade_in
    if fade_out:
        out["fadeOutFrames"] = fade_out
    return out


def keys_at(frames):
    return [{"frame": f, "value": 0.5, "easing": "linear"} for f in frames]


def summary(ref, document, ops):
    """What a vector compares: the shots, the edit, the instance lengths, the codes of the notes, or the refusal."""
    result = ref.apply_ops(document, ops)
    if not result["ok"]:
        body = result["refusal"]
        return {"refused": body["code"], "limit": body.get("limitFrame")}
    doc = result["document"]
    return {
        "shots": [[s["id"], s["editRange"]["startFrame"], s["editRange"]["endFrameExclusive"], s["instanceStartFrame"], s["cameraInFrame"]]
                  for s in sorted(doc["shots"], key=lambda s: s["editRange"]["startFrame"])],
        "edit": doc["editDurationFrames"],
        "instances": dict((i["id"], i["durationFrames"]) for i in doc["sceneInstances"]),
        "notes": [n["code"] for n in result["notes"]],
        "document": doc,
    }


def clip_frames(summary_doc, clip_id):
    for track in summary_doc["document"]["fx"]["tracks"]:
        for entry in track["clips"]:
            if entry["id"] == clip_id:
                return entry
    return None


def key_frames(entry, param="p"):
    return [k["frame"] for k in entry.get("keys", {}).get(param, [])]


# ---------------------------------------------------------------------------------------------------------
# Layer 1: hand vectors. Each returns a list of findings (empty = the reference agrees with the origin text).
# ---------------------------------------------------------------------------------------------------------


def hand_vectors(ref):
    out = []

    def expect(name, compute, want):
        try:
            got = compute()
        except (KeyError, TypeError, IndexError, ValueError) as error:
            out.append("%s: could not be computed (%s %s)" % (name, type(error).__name__, error))
            return
        if got != want:
            out.append("%s: got %r, wanted %r" % (name, got, want))

    def op_set(shot, frames, **kw):
        return {"set_shot_duration": dict(shot=shot, frames=frames, **kw)}

    def op_trim(shot, edge, frame, **kw):
        return {"trim_shot": dict(shot=shot, edge=edge, frame=frame, **kw)}

    def op_cut(a, b, frame, **kw):
        return {"move_cut": {"from": a, "to": b, "frame": frame, **kw}}

    def op_edit(frames, **kw):
        return {"set_edit_duration": dict(frames=frames, **kw)}

    # #3 ripple: the end moves, everything after moves with it, the edit changes by the same delta
    s = summary(ref, three(), [op_set("b", 40)])
    expect("#3 ripple grow", lambda: ((s["shots"], s["edit"])), ([["a", 0, 30, 0, 0], ["b", 30, 70, 0, 0], ["c", 70, 100, 0, 0]], 100))
    s = summary(ref, three(), [op_set("b", 10)])
    expect("#3 ripple shrink", lambda: ((s["shots"], s["edit"], s["instances"]["ib"])), ([["a", 0, 30, 0, 0], ["b", 30, 40, 0, 0], ["c", 40, 70, 0, 0]], 70, 60))
    # #6 the instance grows to fit, never shrinks, with a note
    s = summary(ref, three(durations=(30, 30, 30)), [op_set("b", 45)])
    expect("#6 grows", lambda: ((s["instances"], s["notes"])), ({"ia": 30, "ib": 45, "ic": 30}, ["INSTANCE_GREW"]))
    # #21 the edit duration moves only the last shot; below one frame is SHOT_TOO_SHORT with the legal end
    s = summary(ref, three(), [op_edit(100)])
    expect("#21 edit grows the last", lambda: (s["shots"]), [["a", 0, 30, 0, 0], ["b", 30, 60, 0, 0], ["c", 60, 100, 0, 0]])
    expect("#21 edit too short", lambda: (summary(ref, three(), [op_edit(60)])), {"refused": "SHOT_TOO_SHORT", "limit": 61})
    expect("#33 edit at the limit", lambda: (summary(ref, three(), [op_edit(61)])["edit"]), 61)
    # #11 ripple of the head: the shot stays, its scene and camera advance, the shots after move back
    s = summary(ref, three(), [op_trim("b", "start", 40)])
    expect("#11 head cut", lambda: ((s["shots"], s["edit"])), ([["a", 0, 30, 0, 0], ["b", 30, 50, 10, 10], ["c", 50, 80, 0, 0]], 80))
    # #3 roll later: the next shot loses its head, the edit keeps its length
    s = summary(ref, three(), [op_cut("a", "b", 40)])
    expect("#3 roll later", lambda: ((s["shots"], s["edit"])), ([["a", 0, 40, 0, 0], ["b", 40, 60, 10, 10], ["c", 60, 90, 0, 0]], 90))
    # #28 content: anchor keeps the picture on its edit frames, follow starts the scene at the cut, auto = anchor with head
    headed = lambda: three(inst_b=(20, 20), durations=(60, 120, 60))
    expect("#28 anchor", lambda: (summary(ref, headed(), [op_cut("a", "b", 20, content="anchor")])["shots"][1]), ["b", 20, 60, 10, 10])
    expect("#28 auto anchors what exists", lambda: (summary(ref, headed(), [op_cut("a", "b", 20)])["shots"][1]), ["b", 20, 60, 10, 10])
    expect("#28 follow does not recede", lambda: (summary(ref, headed(), [op_cut("a", "b", 20, content="follow")])["shots"][1]), ["b", 20, 60, 20, 20])
    mixed = summary(ref, three(inst_b=(6, 9), durations=(60, 120, 60)), [op_cut("a", "b", 20)])
    expect("#28 auto mixes anchor and follow", lambda: ((mixed["shots"][1], mixed["notes"])), (["b", 20, 60, 0, 3], ["AUTO_FELL_TO_FOLLOW"]))
    # #9 anchor without head frames: NO_HEAD_FRAMES, limit = the earliest legal cut; at the limit it passes
    short = lambda: three(inst_b=(5, 20), durations=(60, 120, 60))
    expect("#9 no head", lambda: (summary(ref, short(), [op_cut("a", "b", 20, content="anchor")])), {"refused": "NO_HEAD_FRAMES", "limit": 25})
    expect("#33 no head at limit", lambda: (summary(ref, short(), [op_cut("a", "b", 25, content="anchor")])["shots"][1]), ["b", 25, 60, 0, 15])
    # #18 one hour at 60 fps
    expect("#18 too long", lambda: (summary(ref, three(), [op_edit(MAX_EDIT + 1)])), {"refused": "EDIT_TOO_LONG", "limit": MAX_EDIT})
    expect("#18 at the limit", lambda: (summary(ref, three(), [op_edit(MAX_EDIT)])["edit"]), MAX_EDIT)
    # #8 the continuation of a take slides in the scene and the camera; the instance grows for it
    take = make_doc([("a", 0, 30, "ia", 0, 0), ("b", 30, 60, "ia", 30, 30, CONT)], [("ia", 60, False)])
    s = summary(ref, take, [op_set("a", 40)])
    expect("#8 continuation slides", lambda: ((s["shots"], s["instances"])), ([["a", 0, 40, 0, 0], ["b", 40, 70, 40, 40]], {"ia": 70}))
    expect("#8 head of a continuation", lambda: (summary(ref, take, [op_trim("b", "start", 36)])), {"refused": "CONTINUOUS_JOIN", "limit": None})
    expect("#30 follow into a continuous join", lambda: (summary(ref, take, [op_cut("a", "b", 24, content="follow")])), {"refused": "CONTINUOUS_JOIN", "limit": None})
    expect("#30 auto anchors into a continuous join", lambda: (summary(ref, take, [op_cut("a", "b", 24)])["shots"]), [["a", 0, 24, 0, 0], ["b", 24, 60, 24, 24]])
    # #10 LOCKED: shots of one locked instance that were in step must stay in step
    locked = make_doc([("a", 0, 30, "il", 0, 0), ("b", 30, 60, "ib", 0, 0), ("c", 60, 90, "il", 60, 60)], [("il", 120, True), ("ib", 60, False)])
    expect("#10 locked", lambda: (summary(ref, locked, [op_set("b", 40)])), {"refused": "LOCKED", "limit": None})
    lone = make_doc([("a", 0, 30, "ia", 0, 0), ("b", 30, 60, "ib", 0, 0)], [("ia", 60, True), ("ib", 60, True)])
    expect("#10 one shot per instance is free", lambda: (summary(ref, lone, [op_set("a", 40)])["edit"]), 70)
    # #13 transitions: TRANSITION_TOO_LONG with the legal end, shrink to the longest the shots hold
    def trans(alignment, frames):
        return [{"id": "t", "fromShotId": "b", "toShotId": "c", "durationFrames": frames, "alignment": alignment, "handles": "hold"}]
    expect("#13 center too long", lambda: (summary(ref, three(transitions=trans("center", 10)), [op_set("b", 3)])), {"refused": "TRANSITION_TOO_LONG", "limit": 35})
    expect("#33 center at limit", lambda: (summary(ref, three(transitions=trans("center", 10)), [op_set("b", 5)])["edit"]), 65)
    s = summary(ref, three(transitions=trans("center", 10)), [op_set("b", 3, shrink_transitions=True)])
    expect("#13 center shrink", lambda: ((s["document"]["fx"]["transitions"][0]["durationFrames"], s["notes"])), (7, ["TRANSITION_SHRUNK"]))
    s = summary(ref, three(transitions=trans("start", 20)), [op_cut("b", "c", 75, shrink_transitions=True)])
    expect("#13 start shrink = the incoming shot", lambda: (s["document"]["fx"]["transitions"][0]["durationFrames"]), 15)
    s = summary(ref, three(transitions=trans("end", 20)), [op_set("b", 8, shrink_transitions=True)])
    expect("#13 end shrink = the outgoing shot", lambda: (s["document"]["fx"]["transitions"][0]["durationFrames"]), 8)
    # #16 narration is refused here and nothing moves
    expect("#16 with_narration", lambda: (summary(ref, three(), [op_set("b", 40, with_narration=True)])["refused"]), "SHOTS_OPTION_NOT_YET")
    # #12/#31 output clips: inside the removed stretch the clip goes, a crossing clip is cut, later clips move back
    out_tracks = [{"id": "o", "clips": [clip("o1", 5, 10)]}, {"id": "p", "clips": [clip("o2", 40, 10)]},
                  {"id": "q", "clips": [clip("o3", 25, 20)]}, {"id": "r", "clips": [clip("o5", 70, 15)]}]
    s = summary(ref, three(tracks=out_tracks), [op_set("b", 10)])
    expect("#12 removed and cut", lambda: ([(c, clip_frames(s, c) and (clip_frames(s, c)["startFrame"], clip_frames(s, c)["durationFrames"])) for c in ("o1", "o2", "o3", "o5")]),
            [("o1", (5, 10)), ("o2", None), ("o3", (25, 15)), ("o5", (50, 15))])
    s = summary(ref, three(tracks=out_tracks), [op_set("b", 40)])
    expect("#12 grow stretches the crossing clip", lambda: ([(clip_frames(s, c)["startFrame"], clip_frames(s, c)["durationFrames"]) for c in ("o2", "o3", "o5")]), [(40, 10), (25, 20), (80, 15)])
    s = summary(ref, three(tracks=out_tracks), [op_set("b", 40, keep_absolute=True)])
    expect("#12 keep_absolute", lambda: ([(clip_frames(s, c)["startFrame"], clip_frames(s, c)["durationFrames"]) for c in ("o2", "o3", "o5")]), [(40, 10), (25, 20), (70, 15)])
    # D09 keys by case: whole = intact; head cut = shifted and pruned below 0; tail cut = pruned at the end or after
    s = summary(ref, three(tracks=[{"id": "t", "clips": [clip("w", 62, 20, keys=[0, 19])]}]), [op_set("a", 50)])
    expect("D09 moves whole: keys intact", lambda: ((clip_frames(s, "w")["startFrame"], key_frames(clip_frames(s, "w")))), (82, [0, 19]))
    s = summary(ref, three(tracks=[{"id": "t", "clips": [clip("h", 50, 25, keys=[0, 9, 10, 20])]}]), [op_set("b", 10)])
    expect("D09 head cut", lambda: ((clip_frames(s, "h")["startFrame"], clip_frames(s, "h")["durationFrames"], key_frames(clip_frames(s, "h")))), (40, 15, [0, 10]))
    s = summary(ref, three(tracks=[{"id": "t", "clips": [clip("e", 60, 25, keys=[0, 10, 24])]}]), [op_edit(75)])
    expect("D09 tail cut", lambda: ((clip_frames(s, "e")["durationFrames"], key_frames(clip_frames(s, "e")))), (15, [0, 10]))
    s = summary(ref, three(tracks=[{"id": "t", "clips": [clip("f", 40, 30, fade_in=14, fade_out=14)]}]), [op_set("b", 20)])
    expect("D09 fades fit, the fade in first", lambda: ((clip_frames(s, "f")["durationFrames"], clip_frames(s, "f").get("fadeInFrames"), clip_frames(s, "f").get("fadeOutFrames"))), (20, 6, 14))
    # #12 a shot clip moves with the picture (contentShift) and is cut at the shot
    s = summary(ref, three(tracks=[{"id": "t", "clips": [clip("sc", 32, 6, shot="b"), clip("sd", 40, 15, shot="b")]}]), [op_trim("b", "start", 40)])
    expect("#12 shot clips", lambda: ((clip_frames(s, "sc"), (clip_frames(s, "sd")["startFrame"], clip_frames(s, "sd")["durationFrames"]))), (None, (30, 15)))
    # #15 keys of a light and of the weather: after the point they move, inside the removed stretch they go
    lit = [{"id": "d-ia", "lighting": {"ambient": 1, "lights": [{"id": "lamp", "keys": {"intensity": keys_at([5, 45, 70])}}]}, "weatherKeys": keys_at([10, 50])}]
    s = summary(ref, three(defs=lit), [op_set("b", 10)])
    lamp = lambda: s["document"]["sceneDefinitions"][0]["lighting"]["lights"][0]["keys"]["intensity"]
    expect("#15 light keys", lambda: ([k["frame"] for k in lamp()]), [5, 50])
    expect("#15 weather keys", lambda: ([k["frame"] for k in s["document"]["sceneDefinitions"][0]["weatherKeys"]]), [10])
    only = [{"id": "d-ia", "lighting": {"ambient": 1, "lights": [{"id": "lamp", "keys": {"color": keys_at([40, 50])}}]}, "weatherKeys": keys_at([45])}]
    s = summary(ref, three(defs=only), [op_set("b", 10)])
    definition = lambda: s["document"]["sceneDefinitions"][0]
    expect("#15 empty list is absent", lambda: (("keys" in definition()["lighting"]["lights"][0], "weatherKeys" in definition())), (False, False))
    return out


# ---------------------------------------------------------------------------------------------------------
# Layer 2: an independent validity check and the property
# ---------------------------------------------------------------------------------------------------------


def window_of(cut, duration, alignment):
    start = {"start": cut, "center": cut - duration // 2, "end": cut - duration}[alignment]
    before = max(0, min(start + duration, cut) - start)
    return start, start + duration, before, duration - before


def independent_problems(doc):
    """The rules of the origin that a document of this model must keep, written here for the check of the property
    (it shares no code with the reference's `validate_document`). `[]` means valid."""
    out = []
    shots = sorted(doc["shots"], key=lambda s: s["editRange"]["startFrame"])
    by_id = dict((s["id"], s) for s in shots)
    instances = dict((i["id"], i) for i in doc["sceneInstances"])
    edit = doc["editDurationFrames"]
    if edit > MAX_EDIT or edit < 1:
        out.append("edit length %s" % edit)
    cursor = 0
    for shot in shots:
        start, end = shot["editRange"]["startFrame"], shot["editRange"]["endFrameExclusive"]
        if start != cursor:
            out.append("gap or overlap before %s" % shot["id"])
        if end <= start:
            out.append("empty shot %s" % shot["id"])
        cursor = end
        if shot["instanceStartFrame"] < 0 or shot["cameraInFrame"] < 0:
            out.append("negative start in %s" % shot["id"])
        if shot["instanceStartFrame"] + (end - start) > instances[shot["sceneInstanceId"]]["durationFrames"]:
            out.append("%s outlasts its instance" % shot["id"])
    if cursor != edit:
        out.append("the shots cover %d, the edit is %d" % (cursor, edit))
    for previous, shot in zip(shots, shots[1:]):
        length = previous["editRange"]["endFrameExclusive"] - previous["editRange"]["startFrame"]
        join = shot.get("transitionInto") or {}
        if join.get("type") == "continuous":
            if shot["sceneInstanceId"] != previous["sceneInstanceId"] or shot["instanceStartFrame"] != previous["instanceStartFrame"] + length:
                out.append("continuity of the scene broken at %s" % shot["id"])
            if shot["cameraInFrame"] != previous["cameraInFrame"] + length:
                out.append("continuity of the camera broken at %s" % shot["id"])
        elif join.get("type") != "cut" or not join.get("reason"):
            out.append("bad join into %s" % shot["id"])
    for track in doc["fx"]["tracks"]:
        spans = []
        for entry in track["clips"]:
            first, frames = entry["startFrame"], entry["durationFrames"]
            if frames < 1 or first < 0 or first + frames > edit:
                out.append("clip %s outside the edit" % entry["id"])
            if any(first < b and a < first + frames for a, b in spans):
                out.append("clip %s overlaps in its track" % entry["id"])
            spans.append((first, first + frames))
            scope = entry.get("scope")
            if scope and scope["kind"] == "shot":
                owner = by_id[scope["shotId"]]["editRange"]
                if first < owner["startFrame"] or first + frames > owner["endFrameExclusive"]:
                    out.append("clip %s leaves its shot" % entry["id"])
            if entry.get("fadeInFrames", 0) + entry.get("fadeOutFrames", 0) > frames:
                out.append("fades of %s do not fit" % entry["id"])
            for param, keys in (entry.get("keys") or {}).items():
                if not keys:
                    out.append("empty key list in %s" % entry["id"])
                previous = -1
                for key in keys:
                    if not 0 <= key["frame"] < frames or key["frame"] <= previous:
                        out.append("key of %s/%s out of the clip or out of order" % (entry["id"], param))
                    previous = key["frame"]
    windows = []
    for tr in doc["fx"]["transitions"]:
        source, target = by_id[tr["fromShotId"]], by_id[tr["toShotId"]]
        if source["editRange"]["endFrameExclusive"] != target["editRange"]["startFrame"]:
            out.append("transition %s is not at a cut" % tr["id"])
            continue
        from_len = source["editRange"]["endFrameExclusive"] - source["editRange"]["startFrame"]
        to_len = target["editRange"]["endFrameExclusive"] - target["editRange"]["startFrame"]
        start, end, before, after = window_of(target["editRange"]["startFrame"], tr["durationFrames"], tr["alignment"])
        windows.append((start, end))
        if tr["durationFrames"] < 1 or start < 0 or end > edit or from_len < before or to_len < after:
            out.append("transition %s does not fit" % tr["id"])
        if tr.get("handles", "hold") != "hold":
            tail = min(instances[source["sceneInstanceId"]]["durationFrames"] - (source["instanceStartFrame"] + from_len), 300 - (source["cameraInFrame"] + from_len))
            head = min(target["instanceStartFrame"], target["cameraInFrame"])
            if after > max(0, tail) or before > head:
                out.append("transition %s lacks handles" % tr["id"])
    windows.sort()
    for (_, first_end), (second_start, _) in zip(windows, windows[1:]):
        if second_start < first_end:
            out.append("transition windows overlap")
    for definition in doc["sceneDefinitions"]:
        lists = [definition["weatherKeys"]] if "weatherKeys" in definition else []
        for light in (definition.get("lighting") or {}).get("lights", []):
            if "keys" in light and not light["keys"]:
                out.append("empty keys object of %s" % light["id"])
            lists.extend((light.get("keys") or {}).values())
        for keys in lists:
            previous = -1
            if not keys:
                out.append("empty key list in %s" % definition["id"])
            for key in keys:
                if key["frame"] < 0 or key["frame"] <= previous:
                    out.append("keys of %s out of order" % definition["id"])
                previous = key["frame"]
    return out


def random_document(rng):
    """A valid document of the model: 2..5 shots, joins `cut` or `continuous`, shared and locked instances, clips of
    both scopes with keys and fades, transitions with `hold` or `use` handles, light and weather keys."""
    count = rng.randint(2, 5)
    plan = []
    instances = {}
    cursor = 0
    for index in range(count):
        length = rng.randint(4, 40)
        if index and rng.random() < 0.35:
            previous = plan[-1]
            record = dict(id="s%d" % index, instance=previous["instance"], inst_start=previous["inst_start"] + previous["length"],
                          cam_in=previous["cam_in"] + previous["length"], join=CONT)
        else:
            if index and instances and rng.random() < 0.25:
                instance = rng.choice(sorted(instances))
            else:
                instance = "i%d" % index
                instances[instance] = {"locked": rng.random() < 0.25, "frames": 0}
            record = dict(id="s%d" % index, instance=instance, inst_start=rng.randint(0, 25), cam_in=rng.randint(0, 25), join=None if index == 0 else CUT)
        record["length"] = length
        record["start"] = cursor
        cursor += length
        plan.append(record)
    for record in plan:
        instances[record["instance"]]["frames"] = max(instances[record["instance"]]["frames"], record["inst_start"] + record["length"] + rng.randint(0, 15))
    # a chain keeps one instance, so the head frames of its first shot also bound the others: nothing else to do
    edit = cursor
    shots = [(r["id"], r["start"], r["start"] + r["length"], r["instance"], r["inst_start"], r["cam_in"], r["join"]) for r in plan]
    tracks, taken = [], []
    for track_index in range(2):
        placed = []
        for _ in range(rng.randint(0, 3)):
            if rng.random() < 0.5:
                owner = rng.choice(plan)
                first = rng.randint(owner["start"], owner["start"] + owner["length"] - 1)
                frames = rng.randint(1, owner["start"] + owner["length"] - first)
                scope = owner["id"]
            else:
                first = rng.randint(0, edit - 1)
                frames = rng.randint(1, min(30, edit - first))
                scope = None
            if any(first < b and a < first + frames for a, b in placed):
                continue
            placed.append((first, first + frames))
            keys = sorted(rng.sample(range(frames), min(frames, rng.randint(0, 3)))) if frames > 1 else []
            fade_in = rng.randint(0, frames // 2) if rng.random() < 0.4 else 0
            fade_out = rng.randint(0, frames - fade_in) if rng.random() < 0.4 else 0
            taken.append(clip("c%d_%d" % (track_index, len(taken)), first, frames, shot=scope, keys=keys or None, fade_in=fade_in, fade_out=fade_out))
            tracks.append((track_index, taken[-1]))
    track_docs = [{"id": "t%d" % t, "clips": [c for tt, c in tracks if tt == t]} for t in range(2)]
    transitions, windows = [], []
    for position in range(1, count):
        if plan[position]["join"] is not CUT or rng.random() > 0.4:
            continue
        alignment = rng.choice(["start", "center", "end"])
        before_len, after_len = plan[position - 1]["length"], plan[position]["length"]
        longest = {"start": after_len, "end": before_len, "center": min(2 * before_len + 1, 2 * after_len)}[alignment]
        duration = rng.randint(1, longest)
        start, end, before, after = window_of(plan[position]["start"], duration, alignment)
        if start < 0 or end > edit or any(start < b and a < end for a, b in windows):
            continue
        handles = "hold"
        head = min(plan[position]["inst_start"], plan[position]["cam_in"])
        previous = plan[position - 1]
        tail = min(instances[previous["instance"]]["frames"] - (previous["inst_start"] + previous["length"]), 300 - (previous["cam_in"] + previous["length"]))
        if rng.random() < 0.3 and before <= head and after <= max(0, tail):
            handles = "use"
        windows.append((start, end))
        transitions.append({"id": "tr%d" % position, "fromShotId": plan[position - 1]["id"], "toShotId": plan[position]["id"],
                            "durationFrames": duration, "alignment": alignment, "handles": handles})
    definitions = []
    for name in sorted(instances):
        definition = {"id": "d-" + name}
        if rng.random() < 0.6:
            lights = []
            for light_index in range(rng.randint(1, 2)):
                light = {"id": "l%d" % light_index}
                keys = {}
                for channel in ("intensity", "color", "position"):
                    if rng.random() < 0.5:
                        keys[channel] = keys_at(sorted(rng.sample(range(0, edit + 10), rng.randint(1, 4))))
                if keys:
                    light["keys"] = keys
                lights.append(light)
            definition["lighting"] = {"ambient": 1, "lights": lights}
        if rng.random() < 0.4:
            definition["weatherKeys"] = keys_at(sorted(rng.sample(range(0, edit + 10), rng.randint(1, 4))))
        definitions.append(definition)
    document = make_doc(shots, [(name, instances[name]["frames"], instances[name]["locked"]) for name in sorted(instances)],
                        tracks=track_docs, transitions=transitions, defs=definitions)
    for item in document["sceneInstances"]:
        item["sceneDefinitionId"] = "d-" + item["id"]
    return document


def random_op(rng, doc):
    shots = sorted(doc["shots"], key=lambda s: s["editRange"]["startFrame"])
    opts = {}
    if rng.random() < 0.15:
        opts["keep_absolute"] = True
    if rng.random() < 0.25:
        opts["shrink_transitions"] = True
    content = rng.choice(["auto", "auto", "anchor", "follow"])
    if content != "auto":
        opts["content"] = content
    edit = doc["editDurationFrames"]
    kind = rng.choice(["trim", "trim", "cut", "set", "set", "edit"])
    if kind == "cut" and len(shots) > 1:
        i = rng.randrange(len(shots) - 1)
        cut = shots[i]["editRange"]["endFrameExclusive"]
        return {"move_cut": {"from": shots[i]["id"], "to": shots[i + 1]["id"], "frame": cut + rng.randint(-20, 20), **opts}}
    shot = rng.choice(shots)
    if rng.random() < 0.3:
        opts["mode"] = "roll"
    if kind == "trim" or kind == "cut":
        edge = rng.choice(["start", "end"])
        start, end = shot["editRange"]["startFrame"], shot["editRange"]["endFrameExclusive"]
        frame = (start if edge == "start" else end) + rng.randint(-20, 20)
        return {"trim_shot": {"shot": shot["id"], "edge": edge, "frame": frame, **opts}}
    if kind == "set":
        return {"set_shot_duration": {"shot": shot["id"], "frames": rng.randint(0, 60), **opts}}
    opts.pop("mode", None)
    return {"set_edit_duration": {"frames": edit + rng.randint(-25, 40), **opts}}


def op_at_limit(doc, wrapped, limit):
    """Written here: the same op with its frame at `limit` (the end of the shot in the duration ops, #33)."""
    name = next(iter(wrapped))
    body = dict(wrapped[name])
    if name in ("trim_shot", "move_cut"):
        body["frame"] = limit
    elif name == "set_shot_duration":
        shot = [s for s in doc["shots"] if s["id"] == body["shot"]][0]
        body["frames"] = limit - shot["editRange"]["startFrame"]
    else:
        body["frames"] = limit
    return {name: body}


def legit_rejection(doc, op):
    """The gate may refuse a candidate for reasons the op is allowed to cause: keep_absolute leaves layers where they
    are, and `use` handles may be taken by a roll or a head. Anything else is a defect of the layers."""
    name = next(iter(op))
    if op[name].get("keep_absolute"):
        return True
    return any(tr.get("handles", "hold") != "hold" for tr in doc["fx"]["transitions"])


def property_findings(ref, seed, sequences, blind_gate=False):
    """T21: seeded random sequences. Returns the findings (empty = every property held)."""
    findings = []
    rng = random.Random(seed)
    counts = {"ok": 0, "refused": 0, "limit_checked": 0, "rejected": 0}
    for number in range(sequences):
        doc = random_document(rng)
        problems = independent_problems(doc)
        if problems:
            return ["the generator made an invalid document (seq %d): %s" % (number, problems)]
        steps, current = [], doc
        all_ok = True
        for _ in range(rng.randint(1, 4)):
            op = random_op(rng, current)
            before = copy.deepcopy(current)
            result = ref.apply_ops(current, [op])
            if current != before:
                findings.append("seq %d: apply_ops changed its input document" % number)
            if result["ok"]:
                counts["ok"] += 1
                bad = independent_problems(result["document"])
                if bad:
                    findings.append("seq %d: %s left an invalid document: %s" % (number, json.dumps(op), bad[:3]))
                steps.append(op)
                current = result["document"]
                continue
            all_ok = False
            counts["refused"] += 1
            body = result["refusal"]
            if result["document"] != before:
                findings.append("seq %d: the refusal %s changed the document" % (number, body["code"]))
            if body["code"] == "CANDIDATE_REJECTED":
                counts["rejected"] += 1
                if not legit_rejection(before, op):
                    findings.append("seq %d: %s was rejected by the gate with no legitimate cause: %s" % (number, json.dumps(op), body.get("findings")))
            if body.get("limitFrame") is not None:
                twin = ref.apply_ops(before, [op_at_limit(before, op, body["limitFrame"])])
                counts["limit_checked"] += 1
                if not twin["ok"] and twin["refusal"]["code"] == body["code"]:
                    findings.append("seq %d: %s refuses again with %s at its limitFrame %s" % (number, json.dumps(op), body["code"], body["limitFrame"]))
            break
        if all_ok and len(steps) > 1:
            batch = ref.apply_ops(doc, steps)
            if not batch["ok"] or batch["document"] != current:
                findings.append("seq %d: the batch of %d ops differs from its steps" % (number, len(steps)))
    if not blind_gate and (counts["ok"] < sequences or counts["refused"] < sequences // 10 or counts["limit_checked"] < 5):
        findings.append("the sequences did not exercise enough: %r" % counts)
    return findings


# ---------------------------------------------------------------------------------------------------------
# Layer 3: the battery and the negative controls
# ---------------------------------------------------------------------------------------------------------


def corpus_findings(ref):
    out = []
    for case in STORED_CASES["cases"]:
        result = ref.run_case({"kind": case["kind"], "document": case["document"], "ops": case["ops"]})
        if json.loads(json.dumps(result)) != case["expect"]:
            out.append("corpus case %s differs" % case["id"])
    return out


def battery(ref, sequences=60, blind_gate=False):
    return {
        "hand": hand_vectors(ref),
        "corpus": corpus_findings(ref),
        "property": property_findings(ref, 20261115, sequences, blind_gate),
    }


def mutate_no_ripple(ref):
    ref.shift_later_shots = lambda new, seq, index, delta: None


def mutate_tail_not_pruned(ref):
    def prune(keys, shift, duration):
        if not keys:
            return keys, {}
        kept = {}
        for param in keys:
            inside = [dict(key, frame=key["frame"] - shift) for key in keys[param] if key["frame"] - shift >= 0]
            if inside:
                kept[param] = inside
        return (kept or None), {}
    ref.prune_keys = prune


def mutate_fades_not_fit(ref):
    ref.fit_envelope = lambda entry: []


def mutate_removed_keys_kept(ref):
    def keep(frame, chain):
        for splice in chain:
            if splice[0] == "ins":
                if frame >= splice[1]:
                    frame += splice[2]
            elif frame >= splice[2]:
                frame -= splice[2] - splice[1]
        return frame
    ref.map_point = keep


def mutate_refusal_changes_document(ref):
    original = ref.apply_ops

    def apply_ops(document, ops):
        result = original(document, ops)
        if not result["ok"]:
            result["document"]["editDurationFrames"] += 1
        return result
    ref.apply_ops = apply_ops


def mutate_limit_minus_one(ref):
    original = ref.apply_ops

    def apply_ops(document, ops):
        result = original(document, ops)
        if not result["ok"] and result["refusal"].get("limitFrame") is not None:
            result["refusal"]["limitFrame"] -= 1
        return result
    ref.apply_ops = apply_ops


def mutate_limit_plus_one(ref):
    original = ref.apply_ops

    def apply_ops(document, ops):
        result = original(document, ops)
        if not result["ok"] and result["refusal"].get("limitFrame") is not None:
            result["refusal"]["limitFrame"] += 1
        return result
    ref.apply_ops = apply_ops


def mutate_narration_accepted(ref):
    original = ref.normalize_op

    def normalize(wrapped):
        name, fields = original(wrapped)
        fields["with_narration"] = False
        return name, fields
    ref.normalize_op = normalize


def mutate_lock_ignored(ref):
    ref._lock_problem = lambda doc, before, after: None


def mutate_transitions_not_judged(ref):
    ref._judge_transitions = lambda doc, new, changed_ids, shrink, limit_for: []


def mutate_continuation_stays(ref):
    ref.slide_continuation = lambda new, seq, index, frames: None


def mutate_output_clips_stay(ref):
    ref.map_clip = lambda start, end, chain: (start, end, 0)


def mutate_center_rounds_up(ref):
    ref.max_transition_frames = lambda alignment, f, t: t if alignment == "start" else (f if alignment == "end" else min(2 * f, 2 * t + 1))


def mutate_instance_never_grows(ref):
    original = ref._finish

    def finish(doc, old, new, chain, edit_duration, opts, changed_ids, limit_for, extra_notes):
        saved = dict((i["id"], i["durationFrames"]) for i in doc["sceneInstances"])
        notes = original(doc, old, new, chain, edit_duration, opts, changed_ids, limit_for, extra_notes)
        for instance in doc["sceneInstances"]:
            instance["durationFrames"] = saved[instance["id"]]
        return [n for n in notes if n["code"] != "INSTANCE_GREW"]
    ref._finish = finish


def mutate_blind_gate(ref):
    ref.validate_document = lambda doc: []


# name -> (the mutation, also run with the reference's own gate blinded, the layers that must report it)
MUTANTS = [
    ("a ripple that does not move the shots after it", mutate_no_ripple, True, ("hand",)),
    ("a tail cut that does not prune keys", mutate_tail_not_pruned, True, ("hand",)),
    ("fades that are never fitted to the clip", mutate_fades_not_fit, True, ("hand",)),
    ("keys inside a removed stretch that survive", mutate_removed_keys_kept, False, ("hand",)),
    ("a refusal that changes the document", mutate_refusal_changes_document, False, ("property",)),
    ("a limitFrame one frame too low", mutate_limit_minus_one, False, ("hand", "property")),
    ("a limitFrame one frame too high", mutate_limit_plus_one, False, ("hand", "property")),
    ("with_narration accepted", mutate_narration_accepted, False, ("hand",)),
    ("LOCKED never raised", mutate_lock_ignored, False, ("hand",)),
    ("transitions never judged", mutate_transitions_not_judged, True, ("hand",)),
    ("the continuation of a take does not slide", mutate_continuation_stays, True, ("hand",)),
    ("output clips that never move", mutate_output_clips_stay, False, ("hand",)),
    ("a centred transition window with the odd frame on the wrong side", mutate_center_rounds_up, False, ("hand",)),
    ("an instance that never grows", mutate_instance_never_grows, True, ("hand",)),
]


class HandVectorTests(unittest.TestCase):
    def test_the_reference_agrees_with_the_origin_text(self):
        self.assertEqual(hand_vectors(R), [])

    def test_the_hand_vectors_are_enough_of_them(self):
        source = Path(__file__).read_text(encoding="utf-8")
        self.assertGreaterEqual(source.count("expect("), 40)


class PropertyTests(unittest.TestCase):
    def test_t21_random_sequences_keep_the_document_valid_and_refusals_change_nothing(self):
        self.assertEqual(property_findings(R, 20261115, 220), [])

    def test_a_second_seed_agrees(self):
        self.assertEqual(property_findings(R, 7, 120), [])

    def test_the_generator_makes_valid_documents_of_every_shape(self):
        rng = random.Random(1)
        shapes = set()
        for _ in range(300):
            doc = random_document(rng)
            self.assertEqual(independent_problems(doc), [])
            if any((s.get("transitionInto") or {}).get("type") == "continuous" for s in doc["shots"]):
                shapes.add("continuous")
            if any(i.get("editLocked") for i in doc["sceneInstances"]):
                shapes.add("locked")
            if doc["fx"]["transitions"]:
                shapes.add("transitions")
            if any(t.get("handles") == "use" for t in doc["fx"]["transitions"]):
                shapes.add("use")
            if any(c.get("scope") for t in doc["fx"]["tracks"] for c in t["clips"]):
                shapes.add("shot clips")
            if any("weatherKeys" in d for d in doc["sceneDefinitions"]):
                shapes.add("weather keys")
        self.assertEqual(shapes, {"continuous", "locked", "transitions", "use", "shot clips", "weather keys"})

    def test_the_independent_check_does_reject_invalid_documents(self):
        base = three()
        broken = copy.deepcopy(base)
        broken["shots"][1]["editRange"]["startFrame"] = 31
        self.assertTrue(independent_problems(broken))
        broken = copy.deepcopy(base)
        broken["fx"]["tracks"] = [{"id": "t", "clips": [clip("k", 0, 10, keys=[10])]}]
        self.assertTrue(independent_problems(broken))
        broken = copy.deepcopy(base)
        broken["sceneInstances"][0]["durationFrames"] = 5
        self.assertTrue(independent_problems(broken))
        self.assertEqual(independent_problems(base), [])

    def test_a_batch_is_one_gate_at_the_end_and_a_refusal_names_the_op(self):
        result = R.apply_ops(three(), [{"set_shot_duration": {"shot": "b", "frames": 40}}, {"set_shot_duration": {"shot": "zz", "frames": 10}}])
        self.assertFalse(result["ok"])
        self.assertEqual(result["refusal"]["index"], 1)
        self.assertEqual(result["refusal"]["code"], "SHOT_NOT_FOUND")
        self.assertEqual(result["document"], three())

    def test_malformed_ops_are_not_domain_refusals(self):
        for bad in ({}, {"nope": {}}, {"trim_shot": {"shot": "a"}}, {"trim_shot": {"shot": "a", "edge": "middle", "frame": 3}},
                    {"set_shot_duration": {"shot": "a", "frames": True}}, {"set_shot_duration": {"shot": "a", "frames": 3, "mode": "slip"}},
                    {"set_edit_duration": {"frames": 3, "extra": 1}}):
            with self.assertRaises(ValueError):
                R.apply_ops(three(), [bad])

    def test_a_refused_layer_field_refuses_with_shots_layer_not_yet(self):
        # The audit leaves no field to refuse; a synthetic entry proves the branch does what the plan says (D12).
        ref = load_reference()
        ref.DEFERRED_AUDIT = ref.DEFERRED_AUDIT + [{"field": "sceneDefinitions[].future", "destination": "refuse"}]
        doc = three(defs=[{"id": "d-ia", "future": [{"frame": 1}]}])
        result = ref.apply_ops(doc, [{"set_shot_duration": {"shot": "b", "frames": 40}}])
        self.assertEqual((result["ok"], result["refusal"]["code"]), (False, "SHOTS_LAYER_NOT_YET"))
        self.assertEqual(result["document"], doc)


class NegativeControlTests(unittest.TestCase):
    def test_the_real_reference_passes_the_whole_battery(self):
        found = battery(R)
        self.assertEqual(dict((layer, items) for layer, items in found.items() if items), {})

    def test_every_mutant_is_reported(self):
        for name, mutate, with_blind_gate, layers in MUTANTS:
            ref = load_reference()
            mutate(ref)
            found = battery(ref)
            reported = [layer for layer, items in found.items() if items]
            self.assertTrue(reported, "the spec does not notice: %s" % name)
            for layer in layers:
                self.assertIn(layer, reported, "%s: the %s layer misses it (reported by %s)" % (name, layer, reported))

    def test_with_the_gate_blinded_the_independent_check_still_catches_it(self):
        for name, mutate, with_blind_gate, layers in MUTANTS:
            if not with_blind_gate:
                continue
            ref = load_reference()
            mutate(ref)
            mutate_blind_gate(ref)
            findings = property_findings(ref, 20261115, 120, blind_gate=True)
            self.assertTrue(findings, "with the gate blinded, the property layer does not notice: %s" % name)

    def test_a_blinded_gate_is_noticed_because_keep_absolute_then_leaves_invalid_documents(self):
        ref = load_reference()
        mutate_blind_gate(ref)
        self.assertTrue(property_findings(ref, 20261115, 120, blind_gate=True))

    def test_there_are_enough_negative_controls(self):
        self.assertGreaterEqual(len(MUTANTS), 12)
        self.assertGreaterEqual(len([m for m in MUTANTS if m[2]]), 5)


class CorpusTests(unittest.TestCase):
    def test_the_stored_corpus_is_what_the_reference_computes(self):
        self.assertEqual(corpus_findings(R), [])

    def test_case_ids_are_unique_and_every_input_document_is_valid(self):
        ids = [case["id"] for case in STORED_CASES["cases"]]
        self.assertEqual(len(ids), len(set(ids)))
        for case in STORED_CASES["cases"]:
            self.assertEqual(independent_problems(case["document"]), [], case["id"])

    def test_every_matrix_row_is_covered_with_success_and_with_refusal_where_the_rule_can_refuse(self):
        cells = {}
        for case in STORED_CASES["cases"]:
            expect = case["expect"]
            kind = "explain" if "explain" in expect else ("ok" if expect["ok"] else "refusal")
            for row in case["rows"]:
                cells.setdefault(row, set()).add(kind)
        for row in ("T01", "T02", "T03", "T04", "T05", "T07", "T08", "T09", "T10", "T11", "T12", "T13", "T14", "T14b", "T15", "T16", "T17", "T21"):
            self.assertIn("ok", cells.get(row, set()), row)
        for row in ("T01", "T02", "T03", "T04", "T05", "T07", "T09", "T10", "T12", "T14", "T14b", "T17"):
            self.assertIn("refusal", cells.get(row, set()), row)
        self.assertEqual(cells["T19"], {"explain"})

    def test_every_refusal_with_a_limit_frame_has_a_twin_at_that_frame(self):
        cases = dict((case["id"], case) for case in STORED_CASES["cases"])
        twins = 0
        for case in STORED_CASES["cases"]:
            expect = case["expect"]
            if case["kind"] != "ops" or expect["ok"] or len(case["ops"]) != 1 or expect["refusal"].get("limitFrame") is None:
                continue
            twin = cases.get(case["id"] + "@limit")
            self.assertIsNotNone(twin, case["id"])
            twins += 1
            code = expect["refusal"]["code"]
            self.assertEqual(twin["ops"], [op_at_limit(case["document"], case["ops"][0], expect["refusal"]["limitFrame"])])
            self.assertTrue(twin["expect"]["ok"] or twin["expect"]["refusal"]["code"] != code, case["id"])
        self.assertGreaterEqual(twins, 15)

    def test_every_refusal_code_of_the_plan_that_can_happen_has_a_case(self):
        seen = set(case["expect"]["refusal"]["code"] for case in STORED_CASES["cases"] if "refusal" in case["expect"])
        self.assertEqual(seen, {"SHOT_TOO_SHORT", "SHOT_NOT_FOUND", "SHOTS_NOT_ADJACENT", "EDIT_TOO_LONG", "LOCKED", "CONTINUOUS_JOIN",
                                "NO_HEAD_FRAMES", "SHOTS_OPTION_NOT_YET", "TRANSITION_TOO_LONG", "CANDIDATE_REJECTED"})

    def test_the_three_contents_the_options_and_the_batches_are_in_the_corpus(self):
        contents, options, batches = set(), set(), 0
        for case in STORED_CASES["cases"]:
            if len(case["ops"]) > 1:
                batches += 1
            for wrapped in case["ops"]:
                for body in wrapped.values():
                    contents.add(body.get("content", "auto"))
                    options.update(k for k in ("keep_absolute", "shrink_transitions", "with_narration", "mode") if k in body)
        self.assertEqual(contents, {"auto", "anchor", "follow"})
        self.assertEqual(options, {"keep_absolute", "shrink_transitions", "with_narration", "mode"})
        self.assertGreaterEqual(batches, 5)

    def test_the_tagged_ambiguities_are_in_the_manifest_choices(self):
        tags = set(tag for case in STORED_CASES["cases"] for tag in case["tags"])
        self.assertTrue({"AUTO_RESERVE", "FOLLOW", "AT_LIMIT"} <= tags)
        self.assertIn("AUTO_RESERVE", STORED_MANIFEST["oracle_choices"])

    def test_the_explain_vectors(self):
        by_id = dict((case["id"], case["expect"]["explain"]) for case in STORED_CASES["cases"] if case["kind"] == "explain")
        handles = by_id["t19-explain-handles"]
        self.assertEqual(handles["shots"][0]["handles"], {"scene_before": 5, "scene_after": 65, "camera_before": 7, "camera_after": 23})
        self.assertEqual(handles["warnings"], [])
        self.assertEqual([w["code"] for w in by_id["t19-audio-longer-than-edit"]["warnings"]], ["EDIT_SHORTER_THAN_AUDIO"])
        self.assertEqual([w["code"] for w in by_id["t19-edit-longer-than-audio"]["warnings"]], ["EDIT_LONGER_THAN_AUDIO"])
        self.assertEqual(by_id["t19-audio-within-one-frame"]["warnings"], [])
        self.assertIsNone(by_id["t19-no-audio"]["audio"])
        self.assertTrue(by_id["t19-explain-locked-and-continuous"]["shots"][0]["locked"])
        self.assertEqual(by_id["t19-explain-locked-and-continuous"]["shots"][1]["transition_into"], "continuous")


class AuditTests(unittest.TestCase):
    @staticmethod
    def deferred_fields():
        text = (ROOT / "crates/core/src/directed_scene/mod.rs").read_text(encoding="utf-8")
        block = text[text.index("pub const DEFERRED_FIELDS"):]
        block = block[:block.index("];")]
        return re.findall(r'^\s*"(.*)",\s*$', block, flags=re.M)

    def test_the_audit_covers_every_deferred_field_except_lighting(self):
        listed = set(self.deferred_fields())
        self.assertGreater(len(listed), 15)
        self.assertIn("sceneDefinitions[].lighting", listed)
        audited = [entry["field"] for entry in R.DEFERRED_AUDIT]
        self.assertEqual(sorted(audited), sorted(listed - {"sceneDefinitions[].lighting"}))
        self.assertEqual(len(audited), len(set(audited)))

    def test_every_entry_has_a_clock_a_shape_a_destination_and_evidence(self):
        clocks = {"edit-frames", "edit-seconds", "instance-frames", "clip-frames", "none"}
        for entry in R.DEFERRED_AUDIT:
            self.assertIn(entry["clock"], clocks, entry["field"])
            self.assertIn(entry["destination"], {"remap", "refuse", "stays"}, entry["field"])
            self.assertTrue(entry["shape"] and entry["why"] and entry["evidence"], entry["field"])

    def test_the_destination_follows_the_clock(self):
        for entry in R.DEFERRED_AUDIT:
            if entry["destination"] == "remap":
                self.assertEqual(entry["clock"], "edit-frames", entry["field"])
            if entry["clock"] in ("instance-frames", "clip-frames", "none"):
                self.assertEqual(entry["destination"], "stays", entry["field"])
        self.assertEqual([e["field"] for e in R.DEFERRED_AUDIT if e["destination"] == "remap"], ["sceneDefinitions[].weatherKeys"])

    def test_every_citation_names_a_source_of_the_manifest(self):
        sources = set((which, path) for which, path, _, _ in R.ORIGIN_SOURCES)
        for which, path, first, last in R.evidence_refs():
            self.assertIn((which, path), sources)
            self.assertTrue(1 <= first <= last, (path, first, last))

    def test_d07_concludes_real_and_does_not_trigger_q2(self):
        self.assertEqual(R.D07_AUDIT["conclusion"], "real")
        self.assertFalse(R.D07_AUDIT["q2_triggered"])
        self.assertGreaterEqual(len(R.D07_AUDIT["findings"]), 3)


class ManifestTests(unittest.TestCase):
    def test_the_manifest_pins_the_corpus_the_documents_and_the_sources(self):
        manifest = STORED_MANIFEST
        self.assertIn("ORIGIN_NOT_EXECUTED", manifest["flags"])
        self.assertEqual(manifest["generator"], R.GENERATOR_VERSION)
        cases_bytes = (FIXTURES / "cases.json").read_bytes()
        self.assertEqual(manifest["corpus"]["sha256"], hashlib.sha256(cases_bytes).hexdigest())
        self.assertEqual(manifest["corpus"]["cases"], len(STORED_CASES["cases"]))
        for document in manifest["rule_documents"]:
            data = (ROOT / document["path"]).read_bytes().replace(b"\r\n", b"\n")
            self.assertEqual(document["sha256"], hashlib.sha256(data).hexdigest(), document["path"])
        self.assertEqual(sorted(d["path"] for d in manifest["rule_documents"]), sorted(R.ORIGIN_DOCS))
        self.assertGreaterEqual(len(manifest["rule_sources"]), 30)
        for source in manifest["rule_sources"]:
            self.assertRegex(source["sha256"], r"^[0-9a-f]{64}$")
            self.assertRegex(source["blob"], r"^[0-9a-f]{40}$")
            self.assertIn(source["which"], ("pin", "tip"))
        self.assertTrue(any(s["path"].endswith("schema/render-bundle.schema.ts") and s["commit"].startswith("d6adac15") for s in manifest["rule_sources"]))

    def test_the_fixture_has_no_machine_data(self):
        forbidden = ["/" + "Users/", "C:" + "\\\\Users", "/" + "Volumes/", ".t3/" + "worktrees", "wal" + "ly", "E:" + "/www", "E:" + "\\\\www"]
        for name in ("cases.json", "manifest.json"):
            text = (FIXTURES / name).read_text(encoding="utf-8")
            for pattern in forbidden:
                self.assertNotIn(pattern, text, name)


def run_main(*argv):
    out, err = io.StringIO(), io.StringIO()
    with redirect_stdout(out), redirect_stderr(err):
        code = R.main(list(argv))
    return code, out.getvalue(), err.getvalue()


class CheckCommandTests(unittest.TestCase):
    def test_check_on_the_repository_fixtures_reproduces_them_and_writes_nothing(self):
        before = dict((p.name, p.read_bytes()) for p in FIXTURES.iterdir())
        code, out, err = run_main("--check")
        self.assertEqual((code, err), (0, ""), out + err)
        self.assertIn("check ok", out)
        self.assertEqual(dict((p.name, p.read_bytes()) for p in FIXTURES.iterdir()), before)

    def test_check_through_a_real_process(self):
        done = subprocess.run([sys.executable, str(REFERENCE_PATH), "--check"], capture_output=True, text=True, cwd=str(ROOT))
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)

    def test_check_fails_on_a_tampered_missing_or_stale_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(run_main("--out", tmp)[0], 0)
            self.assertEqual(run_main("--check", "--out", tmp)[0], 0)
            target = Path(tmp) / "cases.json"
            original = target.read_bytes()
            target.write_bytes(original.replace(b'"ok": true', b'"ok": false', 1))
            code, _, err = run_main("--check", "--out", tmp)
            self.assertEqual(code, 1)
            self.assertIn("cases.json differs", err)
            target.write_bytes(original)
            (Path(tmp) / "manifest.json").write_bytes((Path(tmp) / "manifest.json").read_bytes() + b" ")
            self.assertEqual(run_main("--check", "--out", tmp)[0], 1)
            (Path(tmp) / "manifest.json").unlink()
            code, _, err = run_main("--check", "--out", tmp)
            self.assertEqual(code, 1)
            self.assertIn("manifest.json is missing", err)

    def test_writing_twice_gives_the_same_bytes(self):
        with tempfile.TemporaryDirectory() as one, tempfile.TemporaryDirectory() as two:
            run_main("--out", one)
            run_main("--out", two)
            for name in ("cases.json", "manifest.json"):
                self.assertEqual((Path(one) / name).read_bytes(), (Path(two) / name).read_bytes())

    def test_a_case_that_does_not_do_what_its_author_meant_stops_the_generator(self):
        ref = load_reference()
        original = ref.build_cases
        ref.build_cases = lambda: [dict(case, want="LOCKED") if case["id"] == "t01-grow-middle" else case for case in original()]
        with self.assertRaises(SystemExit) as stop:
            ref.build_corpus()
        self.assertIn("ORACLE_CASE_MISMATCH", str(stop.exception))

    def test_a_store_stub_is_not_an_interpreter(self):
        self.assertIn("Store", R.interpreter_problem("C:/" + "Users/x/AppData/Local/Microsoft/WindowsApps/python.exe", (3, 13), 0))
        self.assertIsNotNone(R.interpreter_problem("python", (3, 8), 10))
        self.assertIsNone(R.interpreter_problem("python", (3, 9), 10))


class SourceHygieneTests(unittest.TestCase):
    FILES = (REFERENCE_PATH, Path(__file__).resolve())

    def test_python_39_syntax_floor(self):
        for path in self.FILES:
            source = path.read_text(encoding="utf-8")
            tree = ast.parse(source, filename=str(path), feature_version=(3, 9))
            for node in ast.walk(tree):
                self.assertNotIsInstance(node, getattr(ast, "Match", ()), path.name)
                self.assertNotIsInstance(node, getattr(ast, "TryStar", ()), path.name)
                annotations = []
                if isinstance(node, ast.AnnAssign):
                    annotations.append(node.annotation)
                if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                    annotations.extend(a.annotation for a in node.args.args + node.args.kwonlyargs if a.annotation)
                    if node.returns:
                        annotations.append(node.returns)
                for annotation in annotations:
                    for part in ast.walk(annotation):
                        self.assertFalse(isinstance(part, ast.BinOp) and isinstance(part.op, ast.BitOr), "X | Y annotation in %s" % path.name)
                if isinstance(node, ast.Call) and getattr(node.func, "id", getattr(node.func, "attr", "")) == "dataclass":
                    self.assertNotIn("slots", [k.arg for k in node.keywords])
            self.assertNotIn("import " + "dataclasses", source)

    def test_no_bare_assert_so_python_dash_o_behaves_the_same(self):
        for path in self.FILES:
            tree = ast.parse(path.read_text(encoding="utf-8"))
            self.assertEqual([n.lineno for n in ast.walk(tree) if isinstance(n, ast.Assert)], [], path.name)

    def test_only_the_standard_library_is_imported(self):
        stdlib = set(sys.stdlib_module_names) if hasattr(sys, "stdlib_module_names") else None
        for path in self.FILES:
            for node in ast.walk(ast.parse(path.read_text(encoding="utf-8"))):
                names = []
                if isinstance(node, ast.Import):
                    names = [a.name.split(".")[0] for a in node.names]
                if isinstance(node, ast.ImportFrom) and node.module and not node.level:
                    names = [node.module.split(".")[0]]
                for name in names:
                    if stdlib is not None:
                        self.assertIn(name, stdlib, "%s imports %s" % (path.name, name))

    def test_the_reference_reads_no_rust_but_the_deferred_list(self):
        source = REFERENCE_PATH.read_text(encoding="utf-8")
        self.assertEqual(sorted(set(re.findall(r"crates/[A-Za-z0-9_/.\-]+\.rs", source))), ["crates/core/src/directed_scene/mod.rs"])
        for path in (REFERENCE_PATH,):
            self.assertNotIn("subprocess.run([\"cargo\"", path.read_text(encoding="utf-8"))

    def test_no_machine_data_in_the_files_of_this_slice(self):
        forbidden = ["/" + "Users/", "C:" + "\\\\Users", "C:" + "/Users", "/" + "Volumes/", ".t3/" + "worktrees", "wal" + "ly", "E:" + "/www", "E:" + "\\\\www", "top_cpu" + "_processes"]
        files = list(self.FILES) + [ROOT / "docs/features/shot-duration.md", ROOT / "docs/migration/p5-s0-report.md"]
        for path in files:
            if not path.exists():
                continue
            text = path.read_text(encoding="utf-8")
            for number, line in enumerate(text.split("\n"), 1):
                for pattern in forbidden:
                    self.assertNotIn(pattern, line, "%s:%d" % (path.name, number))


if __name__ == "__main__":
    unittest.main(verbosity=1)
