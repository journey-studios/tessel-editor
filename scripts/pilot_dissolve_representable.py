#!/usr/bin/env python3
"""Representable analytic reference X for the P4b `crossDissolve` (BL082.4.17).

Companion of `scripts/pilot_dissolve_analytic.py` and `scripts/pilot-dissolve-corpus.py` (both imported, neither edited).
Plan, pre-registered before this file existed: `docs/migration/p4b-representable-analytic-plan.md`. Python standard
library only, Python >= 3.9, no bare `assert` (identical under `python -O`).

WHY IT EXISTS. Three S0 `lit-*-held` scenes give shot A a 12-frame instance under a 30-frame shot. The origin tolerates
that (it holds inside the shot); the destination importer refuses it (`SHOT_EXCEEDS_INSTANCE_DURATION`). A consumer that
rebuilds a valid instance skips every pixel of deck A there. This module builds NEW scenarios that the real core
accepts: every instance AND camera covers its own shot, and the hold lives where the origin puts it for a dissolve, in
the NEIGHBOUR deck outside its range (the tail of A, the head of B). Instance and camera are clamped apart, each with
its own `held` flag (false on the last real frame, true only outside), and the light always runs on the real edit frame
`f / (num / den)`, held or not.

WHAT IT ADDS TO S0.
  * the clock mutants (`PIN SWP PRI UNH LIN LSR LCC LCE LNF`) are applied to the NEIGHBOUR deck only; the primary deck is
    the truth on both sides of every comparison, so a primary error can never support a neighbour claim;
  * a probe is a distinct (frame, pixel) pair; the same pixel for two roles is one probe;
  * every scenario records the 12 window frames plus the frame before and after, both decks, with rational seconds;
  * a complete destination bundle per scenario (`bundles`) that the Rust test imports with the real core.

WHAT IT IS NOT. `origin-formula` data on SYNTHETIC scenes, destination-validity-checked by the Rust test
`crates/core/tests/pilot_dissolve_representable_import.rs`. Not an executed golden, not accepted by the origin, no pixel
rendered or measured; no consumer (S1b/S2) validated it. `ORIGIN_NOT_EXECUTED` and `ORIGIN_GOLDEN_NOT_COMPARABLE` hold.

Commands: `generate [--origin P] [--check]`, `verify [--origin P] [--require-pin]`, `matrix`.
"""

import argparse
import binascii
import hashlib
import importlib.util
import json
import struct
import sys
from fractions import Fraction
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
FIXTURES = ROOT / "docs/migration/fixtures/pilot-fx"
ASSET_DIR = "dissolve-representable-assets"
CASES_NAME = "dissolve-representable-cases.json"
MANIFEST_NAME = "dissolve-representable-manifest.json"
S0_CASES_NAME = "dissolve-cases.json"
S0_MANIFEST_NAME = "dissolve-manifest.json"
SCHEMA = 1


def _load(module_name, filename):
    if module_name in sys.modules:
        return sys.modules[module_name]
    spec = importlib.util.spec_from_file_location(module_name, HERE / filename)
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    try:
        spec.loader.exec_module(module)
    except BaseException:
        sys.modules.pop(module_name, None)
        raise
    return module


A = _load("pilot_dissolve_analytic", "pilot_dissolve_analytic.py")
C = _load("pilot_dissolve_corpus", "pilot-dissolve-corpus.py")

EXIT_OK, EXIT_FINDING, EXIT_USAGE, EXIT_ORIGIN_UNAVAILABLE, EXIT_PIN_UNVERIFIED, EXIT_BAD_INTERPRETER = 0, 1, 2, 3, 4, 5


class Refusal(Exception):
    """A typed refusal of this generator (a design error of a scenario, never a quiet pass)."""

    def __init__(self, code, detail=""):
        super().__init__("%s: %s" % (code, detail))
        self.code = code
        self.detail = detail


# ---- pre-registered numbers (plan section 4 and 5); identical to the S0 thresholds, never retuned ------------------

WINDOW = 12
TRANSITION = {"duration": WINDOW, "easing": "easeInOutCubic", "curve": "smooth", "intensity": 1}
LIT_COLOURS = {  # chosen BEFORE any run so that the lit pixel stays near 0.9 linear at flicker level 1 (plan section 4)
    "A": {"backdrop": [64, 76, 97], "marker": [82, 96, 120]},
    "B": {"backdrop": [99, 77, 65], "marker": [105, 83, 68]},
}
MARKER_DEFAULT = {"A": {"x": 7.5, "start": 30}, "B": {"x": 2.2, "start": 0}}
MARKER_BURST = {"duration": 12, "delta": [0.0, 4.8, 0.0], "base_y": -2.4, "size": 0.6}
DELTA_FRAMES_LIMIT = 2**31

# short id -> (analytic long name or None, kind): `clock` hypotheses act on the NEIGHBOUR deck only
HYPOTHESES = [
    ("PIN", "BOTH_CLOCKS_PINNED_TOGETHER", "clock"),
    ("SWP", "INSTANCE_AND_CAMERA_CLOCKS_SWAPPED", "clock"),
    ("PRI", "NEIGHBOUR_ON_PRIMARY_CLOCKS", "clock"),
    ("UNH", None, "clock"),
    ("FLG", None, "state"),
    ("LIN", "LIGHT_ON_THE_INSTANCE_CLOCK", "clock"),
    ("LSR", "LIGHT_ON_THE_SHOT_RELATIVE_CLOCK", "clock"),
    ("LCC", "LIGHT_ON_THE_CAMERA_CLIP_CLOCK", "clock"),
    ("LCE", "LIGHT_ON_THE_CLAMPED_EDIT_FRAME", "clock"),
    ("LNF", "LIGHT_ON_A_NOMINAL_FRAME_RATE", "clock"),
    ("DSW", None, "mix"),
    ("NOH", None, "mix"),
]
SHORTS = [item[0] for item in HYPOTHESES]
RULES = {
    "PIN": "grampo conjunto: um deslocamento limitado ao intervalo comum de instancia e camera",
    "SWP": "instancia calculada com os numeros da camera e camera com os da instancia",
    "PRI": "o vizinho usa inicio, instancia e camera do shot primario",
    "UNH": "a instancia nao e presa no fim (corre alem do ultimo quadro); a camera segue a verdade",
    "FLG": "held marcado tambem no ultimo quadro real (querido >= ultimo) e no primeiro (querido <= 0)",
    "LIN": "luz em instancia / fps (vizinho preso congela o flicker)",
    "LSR": "luz em (f - inicio do shot) / fps",
    "LCC": "luz em camera / fps",
    "LCE": "luz em min(f, fim - 1) / fps (max(f, inicio) na cabeca)",
    "LNF": "luz em f / round(num / den)",
    "DSW": "mix(b, a, ...): decks trocados",
    "NOH": "progresso sem o meio quadro: ease(k / D)",
}
POSITION_SHORTS = ("PIN", "SWP", "PRI", "UNH")


def spec(scenario_id, kit, fps, alignment, handles, shots, edit, rejects, agrees, held, role, markers=None):
    return {"id": scenario_id, "kit": kit, "fps": fps, "alignment": alignment, "handles": handles, "shots": shots,
            "edit": edit, "rejects": rejects, "agrees": agrees, "held": held, "role": role, "markers": markers or MARKER_DEFAULT}


def shot_numbers(start, end, inst_start, inst_frames, cam_in, cam_frames):
    return {"start": start, "end": end, "inst_start": inst_start, "inst_frames": inst_frames, "cam_in": cam_in, "cam_frames": cam_frames}


SCENARIOS = [
    spec("rep-a-tail-both-held-light", "lit", [24, 1], "start", "hold",
         {"A": shot_numbers(0, 284, 210, 494, 119, 403), "B": shot_numbers(284, 344, 0, 60, 0, 60)}, 344,
         ["LIN", "LCC", "LCE", "DSW", "NOH", "FLG"], ["LSR", "LNF", "PIN", "SWP", "PRI", "UNH"],
         {"instance": "all", "camera": "all"},
         "A's tail: instance AND camera held from the first window frame (both end exactly at the cut), B primary, A weighted ~1 at the start: isolates the neighbour-only light error"),
    spec("rep-b-tail-instance-held-camera-runs", "pos", [24, 1], "start", "hold",
         {"A": shot_numbers(0, 30, 4, 34, 0, 48), "B": shot_numbers(30, 60, 0, 60, 0, 40)}, 60,
         ["PIN", "SWP", "PRI", "UNH", "DSW", "NOH", "FLG"], ["LIN", "LSR", "LCC", "LCE", "LNF"],
         {"instance": "all", "camera": "none"},
         "A's tail: the instance is held while the camera clip runs past the shot: a joint clamp is rejected by the camera, an unclamped instance by the marker",
         {"A": {"x": 10.8, "start": 27}, "B": {"x": 0.0, "start": 0}}),
    spec("rep-b2-tail-instance-edge-camera-runs", "pos", [24, 1], "start", "hold",
         {"A": shot_numbers(0, 30, 4, 39, 0, 48), "B": shot_numbers(30, 60, 0, 60, 0, 40)}, 60,
         ["PIN", "SWP", "PRI", "UNH", "DSW", "NOH", "FLG"], ["LIN", "LSR", "LCC", "LCE", "LNF"],
         {"instance": "some", "camera": "none"},
         "like b with 5 real tail frames: the last real instance frame (f = 34) is NOT held, f = 35 is: the held flag boundary and the unclamped instance",
         {"A": {"x": 10.8, "start": 31}, "B": {"x": 0.0, "start": 0}}),
    spec("rep-c-head-both-held-light", "lit", [24, 1], "end", "hold",
         {"A": shot_numbers(0, 475, 0, 475, 0, 475), "B": shot_numbers(475, 535, 0, 60, 0, 60)}, 535,
         ["LIN", "LCC", "LCE", "LSR", "DSW", "NOH", "FLG"], ["LNF", "PIN", "SWP", "PRI", "UNH"],
         {"instance": "all", "camera": "all"},
         "B's head: instance and camera held at 0 for the whole window, A primary, B weighted ~1 at the end"),
    spec("rep-d-ntsc-tail-held-light", "lit", [30000, 1001], "start", "hold",
         {"A": shot_numbers(0, 1800, 0, 1800, 0, 1800), "B": shot_numbers(1800, 3600, 0, 1800, 0, 1800)}, 3600,
         ["LNF", "LIN", "LCC", "LCE", "DSW", "NOH", "FLG"], ["LSR", "PIN", "SWP", "PRI", "UNH"],
         {"instance": "all", "camera": "all"},
         "NTSC 30000/1001 at a large edit frame (about 60 s), static cameras, long instances: only the scene clock moves, and a nominal 30 fps is another second"),
    spec("rep-x-center-both-mixed", "both", [24, 1], "center", "hold",
         {"A": shot_numbers(0, 30, 4, 60, 0, 30), "B": shot_numbers(30, 60, 0, 60, 12, 42)}, 60,
         ["PIN", "SWP", "PRI", "DSW", "NOH"], [],
         {"instance": "some", "camera": "some"},
         "extra: mixed hold on both sides of the cut (B head: instance held, camera runs; A tail: instance runs, camera held); a low centre weight does not isolate a light claim"),
    spec("rep-ctl-real-both", "both", [24, 1], "center", "use",
         {"A": shot_numbers(0, 30, 4, 60, 0, 40), "B": shot_numbers(30, 60, 10, 60, 12, 42)}, 60,
         ["SWP", "PRI", "DSW", "NOH"], ["PIN", "UNH"],
         {"instance": "none", "camera": "none"},
         "control (handles use): nothing is held, so the joint clamp equals the truth; light mutants are only reported (a low centre weight does not isolate them)"),
]


# ---- scenario model -----------------------------------------------------------------------------------------


def strict_int(value, field):
    if isinstance(value, bool) or not isinstance(value, int) or not (0 <= value < DELTA_FRAMES_LIMIT):
        raise Refusal("REP_NOT_AN_INTEGER", "%s = %r" % (field, value))
    return value


def check_representable(sc):
    """The destination validity the plan pre-registers: the old 12/30 shape can never be produced."""
    numbers = sc["shots"]
    for side in ("A", "B"):
        shot = numbers[side]
        for field, value in shot.items():
            strict_int(value, "%s.%s.%s" % (sc["id"], side, field))
        length = shot["end"] - shot["start"]
        if length <= 0:
            raise Refusal("REP_EMPTY_SHOT", "%s %s" % (sc["id"], side))
        if shot["inst_frames"] < shot["inst_start"] + length:
            raise Refusal("REP_SHOT_EXCEEDS_INSTANCE", "%s %s: %d instance frames under a shot that needs %d" % (sc["id"], side, shot["inst_frames"], shot["inst_start"] + length))
        if shot["cam_frames"] < shot["cam_in"] + length:
            raise Refusal("REP_CAMERA_SHORT", "%s %s: the camera does not cover its own shot (BL082.4.23)" % (sc["id"], side))
    if numbers["A"]["start"] != 0 or numbers["A"]["end"] != numbers["B"]["start"] or numbers["B"]["end"] != sc["edit"]:
        raise Refusal("REP_SHOTS_DO_NOT_TILE", sc["id"])
    for value in sc["fps"]:
        strict_int(value, "fps")
    if sc["fps"][0] <= 0 or sc["fps"][1] <= 0:
        raise Refusal("REP_BAD_FPS", sc["id"])


def edit_of(sc):
    shots, instances, cameras = {}, {}, {}
    for side in ("A", "B"):
        item = sc["shots"][side]
        shots[side] = {"start": item["start"], "end": item["end"], "instance_start": item["inst_start"], "camera_in": item["cam_in"],
                       "instance": "inst-" + side.lower(), "camera": "cam-" + side.lower()}
        instances["inst-" + side.lower()] = item["inst_frames"]
        cameras["cam-" + side.lower()] = item["cam_frames"]
    return {"fps": list(sc["fps"]), "edit_duration": sc["edit"], "shots": shots, "instances": instances, "cameras": cameras}


def colours_of(kit, side):
    return LIT_COLOURS[side] if kit == "lit" else A.KIT_COLOURS[kit][side]


def truck_clip(frames):
    clip = A.truck_clip(frames)
    clip["segments"][0]["distance"] = A.r9(A.TRUCK_UNITS_PER_FRAME * (frames - 1))
    return clip


def make_deck(sc, side):
    kit = sc["kit"]
    kit_spec = A.KITS[kit]
    colours = colours_of(kit, side)
    if kit_spec["moving"]:
        burst = sc["markers"][side]
        marker = {"id": "marker", "colour8": colours["marker"], "width": MARKER_BURST["size"], "height": MARKER_BURST["size"],
                  "position": [burst["x"], MARKER_BURST["base_y"], 0.0], "render_order": 1,
                  "event": {"start": burst["start"], "duration": MARKER_BURST["duration"], "delta": list(MARKER_BURST["delta"])}}
    else:
        marker = A.static_marker(colours["marker"])
    frames = sc["shots"][side]["cam_frames"]
    deck = {"id": side, "planes": [A.backdrop(colours["backdrop"]), marker],
            "lights": [dict(A.LAMPS[side])] if kit_spec["lit"] else [],
            "camera": truck_clip(frames) if kit_spec["moving"] else A.hold_clip(frames)}
    if len(deck["lights"]) > 4:
        raise Refusal("REP_LIGHT_OUT_OF_RANGE", "more than four lamps")
    for lamp in deck["lights"]:
        if not (0 <= lamp["intensity"] <= 20 and lamp["radius"] > 0 and 0 <= lamp["flicker"] <= 1 and all(0 <= c <= 1 for c in lamp["colour"])):
            raise Refusal("REP_LIGHT_OUT_OF_RANGE", lamp["id"])
    return deck


def transition_of(sc):
    return dict(TRANSITION, alignment=sc["alignment"], handles=sc["handles"])


def cut_of(edit):
    return edit["shots"]["A"]["end"]


# ---- states ------------------------------------------------------------------------------------------------------


def with_wanted(edit, side, state, numbers_side=None):
    numbers = edit["shots"][numbers_side or side]
    state = dict(state)
    state["instance_wanted"] = numbers["instance_start"] + state["delta"]
    state["camera_wanted"] = numbers["camera_in"] + state["delta"]
    return state


def truth_states(edit, f):
    primary = "A" if f < cut_of(edit) else "B"
    neighbour = "B" if primary == "A" else "A"
    return {side: with_wanted(edit, side, A.clocks_of(edit, side, f)) for side in ("A", "B")}, primary, neighbour


def hypothesis_states(edit, f, short, truth, primary, neighbour):
    """The truth, with the wrong rule applied to the NEIGHBOUR deck only (never to the primary)."""
    states = dict(truth)
    long_name = next(item[1] for item in HYPOTHESES if item[0] == short)
    if long_name is not None:
        hypothesis = A.HYPOTHESES[long_name]["clock"]
        wrong = A.clocks_of(edit, neighbour, f, hypothesis, primary_id=primary)
        states[neighbour] = with_wanted(edit, neighbour, wrong, primary if hypothesis.primary_numbers else None)
    elif short == "UNH":
        state = dict(truth[neighbour])
        state["instance_frame"] = max(0, state["instance_wanted"])
        state["held_instance"] = state["instance_wanted"] < 0
        states[neighbour] = state
    return states


def flagged_states(edit, truth):
    """FLG: the held flag of the clamp function also set on the last / first real frame (it acts on states)."""
    out = {}
    for side, state in truth.items():
        item = edit["shots"][side]
        state = dict(state)
        state["held_instance"] = state["instance_wanted"] >= edit["instances"][item["instance"]] - 1 or state["instance_wanted"] <= 0
        state["held_camera"] = state["camera_wanted"] >= edit["cameras"][item["camera"]] - 1 or state["camera_wanted"] <= 0
        out[side] = state
    return out


def check_flags(sc, edit, f, states):
    for side, state in states.items():
        item = edit["shots"][side]
        for clock, frames_of in (("instance", edit["instances"][item["instance"]]), ("camera", edit["cameras"][item["camera"]])):
            wanted, got, held = state[clock + "_wanted"], state[clock + "_frame" if clock == "instance" else "camera_clip_frame"], state["held_" + clock]
            inside = 0 <= wanted <= frames_of - 1
            if held != (wanted != got) or (inside and (held or got != wanted)) or (not inside and not held):
                raise Refusal("REP_HELD_FLAG", "%s %s %s f=%d: wanted %d got %d held %r" % (sc["id"], side, clock, f, wanted, got, held))
        shot_has_f = item["start"] <= f < item["end"]
        if shot_has_f and (state["instance_wanted"] != state["instance_frame"] or state["camera_wanted"] != state["camera_clip_frame"]):
            raise Refusal("REP_PRIMARY_CLAMPED", "%s %s f=%d: a deck inside its own shot is clamped (BL082.4.23)" % (sc["id"], side, f))


# ---- pixels --------------------------------------------------------------------------------------------------


def mix_noh(a8, b8, transition, k):
    progress = A.ease(k / transition["duration"], transition["easing"])
    shaped = A.mixf(progress, A.smooth_unit(A.clamp01(progress)), 1.0 if transition["curve"] == "smooth" else 0.0)
    amount = A.clamp01(transition["intensity"])
    out = []
    for a_code, b_code in zip(a8, b8):
        a, b = A.decode(a_code), A.decode(b_code)
        out.append(A.code_of(A.encode(A.mixf(A.mixf(a, b, progress), A.mixf(a, b, shaped), amount)) * 255.0))
    return out


def pixel_out(decks, states, x, y, transition, k, mix="plain"):
    _, _, a8 = A.deck_pixel(decks["A"], states["A"], x, y)
    _, _, b8 = A.deck_pixel(decks["B"], states["B"], x, y)
    if mix == "DSW":
        return A.mix_codes(b8, a8, transition, k), a8, b8
    if mix == "NOH":
        return mix_noh(a8, b8, transition, k), a8, b8
    return A.mix_codes(a8, b8, transition, k), a8, b8


def probe_candidates(sc, decks, states, primary, neighbour):
    ordered, seen = [], {}
    raw = []
    for side, role in ((primary, "primary"), (neighbour, "neighbour")):
        x, y, _ = A.marker_pixel(decks[side], states[side])
        raw.append(("marker_%s_%s" % (role, side), (x, y), True))
    raw += [(name, point, False) for name, point in A.clear_points(sc["kit"])]
    for name, point, is_marker in raw:
        if point in seen:
            ordered[seen[point]] = (ordered[seen[point]][0] + "+" + name, point, is_marker and ordered[seen[point]][2])
        else:
            seen[point] = len(ordered)
            ordered.append((name, point, is_marker))
    return ordered


def build_frame(sc, edit, decks, transition, f, window, matrix):
    truth, primary, neighbour = truth_states(edit, f)
    check_flags(sc, edit, f, truth)
    start, end = window
    in_window = start <= f < end
    numerator, denominator = edit["fps"]
    seconds = Fraction(f * denominator, numerator)
    record = {"edit_frame": f, "in_window": in_window, "primary": primary, "neighbour": neighbour if in_window else None,
              "edit_seconds": {"rational": "%d/%d" % (seconds.numerator, seconds.denominator), "decimal": A.r9(float(seconds))}, "decks": {}}
    for side in ("A", "B"):
        state, shot = truth[side], edit["shots"][side]
        item = {"instance_wanted": state["instance_wanted"], "instance_frame": state["instance_frame"], "held_instance": state["held_instance"],
                "camera_wanted": state["camera_wanted"], "camera_clip_frame": state["camera_clip_frame"], "held_camera": state["held_camera"],
                "shot_clip_frame": C.shot_clip_frame(shot["start"], shot["end"] - shot["start"], shot["start"], shot["end"], f),
                "fx_edge": not (shot["start"] <= f < shot["end"]), "scene_seconds": A.r9(state["scene_seconds"]),
                "lights": [{"id": row["id"], "flicker_level": A.r9(row["flicker_level"]), "effective_intensity": A.r9(row["effective_intensity"])}
                           for row in A.deck_lights(decks[side], state)]}
        geometry = A.marker_geometry(decks[side], state)
        item["marker"] = {"world_centre": geometry["world_centre"], "centre_px": geometry["centre_px"]}
        record["decks"][side] = item
    flagged = flagged_states(edit, truth)
    if any(flagged[side][key] != truth[side][key] for side in ("A", "B") for key in ("held_instance", "held_camera")):
        matrix["FLG"]["state_frames"].append(f)
    if not in_window:
        record["transition"] = None
        return record, truth, primary, neighbour
    k = f - start
    progress, shaped, amount, weight = A.weight_for(k, transition)
    record["k"] = k
    record["transition"] = {"progress": A.r9(progress), "shaped": A.r9(shaped), "amount": A.r9(amount), "weight": A.r9(weight)}
    candidates = probe_candidates(sc, decks, truth, primary, neighbour)
    hyp_worlds = {short: hypothesis_states(edit, f, short, truth, primary, neighbour) for short, _, kind in HYPOTHESES if kind == "clock"}
    if A.KITS[sc["kit"]]["moving"]:
        true_x, true_y, _ = A.marker_pixel(decks[neighbour], truth[neighbour])
        taken = [point for _, point, _ in candidates]
        for short in POSITION_SHORTS:
            world = hyp_worlds[short]
            hx, hy, _ = A.marker_pixel(decks[neighbour], world[neighbour])
            far = abs(hx - true_x) > A.MIN_DECOY_SEPARATION_MARGINS * A.FLAT_MARGIN_PX or abs(hy - true_y) > A.MIN_DECOY_SEPARATION_MARGINS * A.FLAT_MARGIN_PX
            if far and (hx, hy) not in taken and A.is_flat(decks, truth, hx, hy) and A.is_flat(decks, world, hx, hy):
                candidates.append(("decoy_of_" + short, (hx, hy), False))
                taken.append((hx, hy))
    probes = []
    for name, (x, y), is_marker in candidates:
        if not A.is_flat(decks, truth, x, y):
            if name.startswith("backdrop"):
                raise Refusal("REP_PROBE_NOT_FLAT", "%s f=%d %s (%d, %d)" % (sc["id"], f, name, x, y))
            matrix["_skipped"].append({"frame": f, "probe": name, "pixel": "%d,%d" % (x, y), "reason": "not flat in both decks"})
            continue
        truth_out, a8, b8 = pixel_out(decks, truth, x, y, transition, k)
        a_cover = A.deck_pixel(decks["A"], truth["A"], x, y)[0]
        b_cover = A.deck_pixel(decks["B"], truth["B"], x, y)[0]
        rejected = []
        for short, long_name, kind in HYPOTHESES:
            if kind == "state":
                continue
            if kind == "mix":
                wrong, _, _ = pixel_out(decks, truth, x, y, transition, k, short)
            else:
                world = hyp_worlds[short]
                wrong, wa8, wb8 = pixel_out(decks, world, x, y, transition, k)
                side_codes = {"A": wa8, "B": wb8}
                truth_codes = {"A": a8, "B": b8}
                if side_codes[primary] != truth_codes[primary]:
                    raise Refusal("REP_ISOLATION_BROKEN", "%s %s f=%d: the hypothesis changed the primary deck" % (sc["id"], short, f))
            separation = max(abs(wrong[i] - truth_out[i]) for i in range(3))
            row = matrix[short]
            row["probes"] += 1
            row["max_separation"] = max(row["max_separation"], separation)
            if separation >= A.MIN_SEPARATION_CODES:
                row["rejecting_probes"] += 1
                row["_frames"].add(f)
                rejected.append(short)
        entry = {"id": name, "pixel": "%d,%d" % (x, y), "deck_a": "%s %s" % (a_cover, A.hex8(a8)), "deck_b": "%s %s" % (b_cover, A.hex8(b8)), "expected": A.hex8(truth_out)}
        if rejected:
            entry["rejects"] = ",".join(rejected)
        probes.append(entry)
    record["probes"] = probes
    return record, truth, primary, neighbour


def held_pattern(values):
    return "all" if all(values) else "none" if not any(values) else "some"


def new_matrix():
    matrix = {short: {"kind": kind, "probes": 0, "rejecting_probes": 0, "max_separation": 0, "_frames": set(), "state_frames": []}
              for short, _, kind in HYPOTHESES}
    matrix["_skipped"] = []
    return matrix


def finish_matrix(matrix):
    out = {}
    for short, _, kind in HYPOTHESES:
        row = matrix[short]
        frames = sorted(row["_frames"])
        if kind == "state":
            status = "rejects" if row["state_frames"] else "agrees"
        elif row["rejecting_probes"] >= A.MIN_REJECTING_PROBES and len(frames) >= A.MIN_REJECTING_FRAMES:
            status = "rejects"
        elif row["max_separation"] <= A.AGREE_MAX_CODES:
            status = "agrees"
        else:
            status = "partial"
        entry = {"kind": kind, "status": status, "probes": row["probes"], "rejecting_probes": row["rejecting_probes"],
                 "rejecting_frames": len(frames), "max_separation": row["max_separation"]}
        if kind == "state":
            entry["state_frames"] = sorted(row["state_frames"])
        out[short] = entry
    return out


def build_scenario(sc):
    check_representable(sc)
    edit = edit_of(sc)
    transition = transition_of(sc)
    decks = {side: make_deck(sc, side) for side in ("A", "B")}
    start, end = A.window_of(cut_of(edit), transition["duration"], transition["alignment"])
    if start < 0 or end > edit["edit_duration"]:
        raise Refusal("REP_WINDOW_OUTSIDE_EDIT", sc["id"])
    matrix = new_matrix()
    frames, neighbour_rows = [], []
    for f in range(start - 1, end + 1):
        record, truth, primary, neighbour = build_frame(sc, edit, decks, transition, f, (start, end), matrix)
        frames.append(record)
        if record["in_window"]:
            neighbour_rows.append(truth[neighbour])
    pattern = {"instance": held_pattern([row["held_instance"] for row in neighbour_rows]), "camera": held_pattern([row["held_camera"] for row in neighbour_rows])}
    if pattern != sc["held"]:
        raise Refusal("REP_HELD_PATTERN", "%s: the neighbour is held %r, the plan says %r" % (sc["id"], pattern, sc["held"]))
    any_held = pattern["instance"] != "none" or pattern["camera"] != "none"
    if sc["handles"] == "use" and any_held:
        raise Refusal("REP_HANDLES_USE_NEEDS_REAL_FRAMES", sc["id"])
    if sc["handles"] == "hold" and not any_held:
        raise Refusal("REP_HANDLES_HOLD_WITHOUT_A_HOLD", sc["id"])
    skipped = matrix.pop("_skipped")
    finished = finish_matrix(matrix)
    for short in sc["rejects"]:
        if finished[short]["status"] != "rejects":
            raise Refusal("REP_NO_REFUSAL_POWER", "%s: %s is %s (%d probes on %d frames, max %d codes; state frames %r)" % (
                sc["id"], short, finished[short]["status"], finished[short]["rejecting_probes"], finished[short]["rejecting_frames"],
                finished[short]["max_separation"], finished[short].get("state_frames")))
    for short in sc["agrees"]:
        if finished[short]["status"] != "agrees":
            raise Refusal("REP_CONTROL_DISAGREES", "%s: %s is %s (max %d codes) but must agree" % (sc["id"], short, finished[short]["status"], finished[short]["max_separation"]))
    reported = [short for short in SHORTS if short not in sc["rejects"] and short not in sc["agrees"]]
    return {
        "id": sc["id"], "kit": sc["kit"], "role": sc["role"], "fps": list(sc["fps"]), "edit_duration_frames": edit["edit_duration"], "cut": cut_of(edit),
        "transition": dict(transition, window={"start": start, "end_exclusive": end}),
        "shots": {side: dict(sc["shots"][side]) for side in ("A", "B")},
        "neighbour_held": pattern, "must_reject": list(sc["rejects"]), "must_agree": list(sc["agrees"]), "reported_only": reported,
        "matrix": finished, "skipped_probes": skipped,
        "decks": {side: {"planes": decks[side]["planes"], "lights": decks[side]["lights"], "camera": decks[side]["camera"]} for side in ("A", "B")},
        "frames": frames,
    }


# ---- the destination bundle --------------------------------------------------------------------------------------


def xyz(x, y, z):
    return {"x": x, "y": y, "z": z}


def flat_name(code):
    return "flat-%02x%02x%02x" % tuple(code)


def png_1x1(code):
    """One opaque RGB pixel, deterministic: a stored (uncompressed) deflate block, no compressor involved."""
    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", binascii.crc32(tag + data) & 0xFFFFFFFF)
    raw = bytes([0, code[0], code[1], code[2]])
    a, b = 1, 0
    for byte in raw:
        a = (a + byte) % 65521
        b = (b + a) % 65521
    stream = b"\x78\x01" + b"\x01" + struct.pack("<HH", len(raw), len(raw) ^ 0xFFFF) + raw + struct.pack(">I", (b << 16) | a)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0)) + chunk(b"IDAT", stream) + chunk(b"IEND", b"")


def scene_nodes(deck, assets):
    nodes = [{"id": "root", "parentId": None, "type": "group", "localPosition": xyz(0, 0, 0), "localRotation": {"x": 0, "y": 0, "z": 0, "w": 1}, "localScale": xyz(1, 1, 1)}]
    for plane in deck["planes"]:
        asset = assets[flat_name(plane["colour8"])]
        nodes.append({
            "id": plane["id"], "parentId": "root", "type": "imagePlane",
            "assetRef": {"assetId": asset["assetId"], "revisionId": asset["revisionId"], "contentHash": asset["contentHash"], "relativePath": asset["relativePath"]},
            "widthSceneUnits": plane["width"], "heightSceneUnits": plane["height"], "renderOrder": plane["render_order"],
            "localPosition": xyz(*plane["position"]), "localRotation": {"x": 0, "y": 0, "z": 0, "w": 1}, "localScale": xyz(1, 1, 1)})
    return nodes


def camera_json(side, sc, deck):
    clip = deck["camera"]
    segment = clip["segments"][0]
    if segment["recipe"] == "hold":
        entry = {"recipe": "hold", "durationFrames": segment["frames"], "from": "initial"}
    else:
        entry = {"recipe": "truckRight", "durationFrames": segment["frames"], "from": "initial", "distanceSceneUnits": segment["distance"], "easing": "linear"}
    return {"id": "cam-" + side.lower(), "initialPosition": xyz(*clip["position"]), "initialTarget": xyz(*clip["target"]), "initialFovDeg": A.FOV_DEG, "segments": [entry]}


def build_bundle(sc, decks, asset_table):
    colours = sorted({tuple(plane["colour8"]) for deck in decks.values() for plane in deck["planes"]})
    assets = {flat_name(list(code)): asset_table[flat_name(list(code))] for code in colours}
    numerator, denominator = sc["fps"]
    fps = {"numerator": numerator, "denominator": denominator}
    bundle = {
        "schemaVersion": "1.0", "bundleId": sc["id"], "revisionId": "rev-1", "fps": dict(fps), "editDurationFrames": sc["edit"],
        "renderProfile": {"width": A.VIEWPORT[0], "height": A.VIEWPORT[1], "fps": dict(fps), "colorSpace": "srgb-display", "alphaMode": "opaque",
                          "premultiplyOnUpload": False, "videoCodec": "h264-yuv420p", "threeRevision": "0.187.0", "browserChannel": "synthetic-oracle"},
        "assets": [assets[name] for name in sorted(assets)],
        "performanceTracks": [], "compositionPasses": [], "sceneDefinitions": [], "sceneInstances": [], "cameraClips": [], "shots": [],
    }
    moving = A.KITS[sc["kit"]]["moving"]
    for side in ("A", "B"):
        lower, deck, shot = side.lower(), decks[side], sc["shots"][side]
        definition = {"id": "scene-" + lower, "schemaVersion": "1.0", "rootNodeId": "root", "nodes": scene_nodes(deck, assets)}
        if deck["lights"]:
            definition["lighting"] = {"ambient": 1, "lights": [
                {"id": lamp["id"], "intensity": lamp["intensity"], "radius": lamp["radius"], "flicker": lamp["flicker"],
                 "position": xyz(*lamp["position"]), "color": {"r": lamp["colour"][0], "g": lamp["colour"][1], "b": lamp["colour"][2]}}
                for lamp in deck["lights"]]}
        bundle["sceneDefinitions"].append(definition)
        tracks = []
        if moving:
            event = next(plane for plane in deck["planes"] if plane["id"] == "marker")["event"]
            bundle["performanceTracks"].append({"id": "perf-" + lower, "instanceId": "inst-" + lower, "channels": [{
                "kind": "event", "channelId": "lift", "nodeId": "marker", "property": "localPosition", "startInstanceFrame": event["start"],
                "durationFrames": event["duration"], "delta": xyz(*event["delta"]), "holdFinal": True}]})
            tracks = ["perf-" + lower]
        bundle["sceneInstances"].append({"id": "inst-" + lower, "sceneDefinitionId": "scene-" + lower, "durationFrames": shot["inst_frames"], "performanceTrackIds": tracks})
        bundle["cameraClips"].append(camera_json(side, sc, deck))
        entry = {"id": "shot-" + lower, "sceneInstanceId": "inst-" + lower, "editRange": {"startFrame": shot["start"], "endFrameExclusive": shot["end"]},
                 "instanceStartFrame": shot["inst_start"], "cameraClipId": "cam-" + lower, "cameraInFrame": shot["cam_in"], "intent": "synthetic shot " + side}
        if side == "B":
            entry["transitionInto"] = {"type": "cut", "reason": "synthetic cut into the representable oracle"}
        bundle["shots"].append(entry)
    bundle["fx"] = {"catalogVersion": 1, "tracks": [], "transitions": [{
        "id": "tr", "effect": "crossDissolve", "fromShotId": "shot-a", "toShotId": "shot-b", "durationFrames": WINDOW, "alignment": sc["alignment"],
        "handles": sc["handles"], "easing": TRANSITION["easing"], "params": {"curve": TRANSITION["curve"], "intensity": TRANSITION["intensity"]}}]}
    return bundle


# ---- the document ------------------------------------------------------------------------------------------------


def sha256_of(data):
    return hashlib.sha256(data).hexdigest()


def _compact(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False, allow_nan=False, separators=(", ", ": "))


def _emit(value, level):
    if isinstance(value, (dict, list)) and value:
        compact = _compact(value)
        if len(compact) + 2 * level <= 100:
            return compact
        pad, close = "  " * (level + 1), "  " * level
        if isinstance(value, dict):
            body = ",\n".join("%s%s: %s" % (pad, json.dumps(key, ensure_ascii=False), _emit(value[key], level + 1)) for key in sorted(value))
            return "{\n%s\n%s}" % (body, close)
        return "[\n%s\n%s]" % (",\n".join(pad + _emit(item, level + 1) for item in value), close)
    return json.dumps(value, ensure_ascii=False, allow_nan=False)


def canonical(value):
    """UTF-8, LF, sorted keys; a container that fits in 100 columns stays on one line (one canonical form)."""
    return (_emit(value, 0) + "\n").encode("utf-8")


def thresholds():
    return {
        "status": "pre-registered in docs/migration/p4b-representable-analytic-plan.md before this generator existed; the S0 numbers, never retuned after a result",
        "flat_margin_px": A.FLAT_MARGIN_PX, "flat_max_code_spread": A.FLAT_MAX_CODE_SPREAD, "min_separation_codes": A.MIN_SEPARATION_CODES,
        "min_rejecting_probes": A.MIN_REJECTING_PROBES, "min_rejecting_frames": A.MIN_REJECTING_FRAMES, "agree_max_codes": A.AGREE_MAX_CODES,
        "tie_margin": A.TIE_MARGIN, "state_hypothesis_rule": "FLG rejects when at least one recorded deck state has a different held flag",
        "tolerance_for_the_implementation": A.TOLERANCE_REFERENCE,
        "probe": "a (frame, pixel) pair; one pixel with two roles is one probe; a marker pixel that is not flat in both decks is listed in `skipped_probes`, never replaced",
    }


def conventions():
    return {
        "scope": "NEW valid scenarios: every instance and camera covers its own shot; the hold lives in the NEIGHBOUR deck outside its range (tail of A, head of B)",
        "clamps": "instance and camera are clamped apart; `held_*` is false for every wanted frame inside [0, frames - 1] (including the last real one) and true only outside it",
        "scene_clock": "the light's seconds are f / (num / den) with the real edit frame, held or not; `edit_seconds.rational` is that fraction in lowest terms",
        "neighbour_only": "PIN SWP PRI UNH LIN LSR LCC LCE LNF replace the neighbour deck's state by the wrong one; the primary deck is the truth on both sides; FLG acts on the held flags of every recorded deck state; DSW and NOH act on the mix",
        "frames": "the 12 window frames plus the frame before and the frame after; both decks always",
        "projection": A.conventions()["projection"], "pixels": A.conventions()["pixels"], "planes": A.conventions()["planes"],
        "event_channel": A.conventions()["event_channel"], "camera": A.conventions()["camera"], "lighting": A.conventions()["lighting"],
        "flicker": A.conventions()["flicker"], "deck_codes": A.conventions()["deck_codes"], "alpha": A.conventions()["alpha"],
        "position_acceptance": A.conventions()["position_acceptance"],
        "bundles": "one complete destination bundle per scenario (assets are 1x1 flat PNGs of the sibling folder); the Rust test imports each with the real core",
        "asset_resolution": {"bundle_prefix": "assets/", "fixture_prefix": ASSET_DIR + "/",
                             "rule": "keep the basename; resolve or copy into the bundle root, checking the manifest hash"},
    }


def s0_facts():
    manifest_bytes = (FIXTURES / S0_MANIFEST_NAME).read_bytes()
    manifest = json.loads(manifest_bytes.decode("utf-8"))
    return {"manifest_sha256": sha256_of(manifest_bytes), "cases_sha256_recorded": manifest["corpus"]["sha256"],
            "cases_sha256_actual": sha256_of((FIXTURES / S0_CASES_NAME).read_bytes()), "sources": len(manifest["sources"]),
            "tip": manifest["origin"]["tip_commit_read"], "pin_named_by_the_plan": manifest["origin"]["pin_named_by_the_plan"]}


DESTINATION_LINES = [
    ("crates/core/src/directed_scene/integrity.rs", '"SHOT_EXCEEDS_INSTANCE_DURATION"', "the importer refuses an instance shorter than its shot: the S0 lit-*-held shape"),
    ("crates/core/src/directed_scene/lighting.rs", "pub const MAX_INTENSITY: f64 = 20.0;", "synthetic lamps fit the core intensity limit"),
    ("crates/core/src/directed_scene/lighting.rs", "pub const MAX_SCENE_LIGHTS: usize = 4;", "at most four lamps"),
    ("crates/core/src/directed_scene/lighting.rs", "const FLICKER_DEPTH: f64 = 0.45;", "flicker depth"),
    ("crates/core/src/directed_scene/lighting.rs", "1.0 - flicker * FLICKER_DEPTH * (0.6 * sway + 0.4 * flutter)", "flicker level"),
    ("crates/core/src/directed_scene/fx/admission.rs", "FX_TRANSITION_NOT_YET", "the public gate that stays closed"),
    ("crates/core/src/directed_scene/temporal/frame.rs", "pub fn resolve_frame_mapping(", "inside-shot mapping the Rust test uses"),
    ("crates/core/src/directed_scene/animation/channels.rs", "fn event_progress(frame: f64, start: f64, duration: f64) -> f64 {", "the event channel the marker follows"),
]


def destination_reads():
    texts, rows = {}, []
    for path, line, why in DESTINATION_LINES:
        if path not in texts:
            texts[path] = (ROOT / path).read_text(encoding="utf-8").replace("\r\n", "\n")
        rows.append({"file": path, "line": line, "why": why, "present": line in texts[path]})
    missing = [row for row in rows if not row["present"]]
    if missing:
        raise Refusal("REP_DESTINATION_FACT", "the destination text the data was reconciled with changed: %s" % missing[0]["line"])
    return rows


def build_assets(scenarios_data):
    colours = sorted({tuple(plane["colour8"]) for sc in scenarios_data for deck in sc["decks"].values() for plane in deck["planes"]})
    files, table = {}, {}
    for code in colours:
        name = flat_name(list(code))
        data = png_1x1(list(code))
        files[name + ".png"] = data
        table[name] = {"assetId": name, "revisionId": "rev-1", "contentHash": sha256_of(data), "relativePath": "assets/%s.png" % name,
                       "widthPx": 1, "heightPx": 1, "alphaMode": "opaque", "colorSpace": "srgb"}
    return files, table


def build_cases():
    problems = []
    for short, _, kind in HYPOTHESES:
        if kind == "clock" and short not in ("UNH",):
            long_name = next(item[1] for item in HYPOTHESES if item[0] == short)
            if A.HYPOTHESES[long_name]["short"] != short:
                problems.append(short)
    if problems:
        raise Refusal("REP_ORACLE_DISAGREES", "short ids differ from the S0 oracle: %r" % problems)
    section = []
    for sc in SCENARIOS:
        try:
            section.append(build_scenario(sc))
        except A.Refusal as refusal:
            raise Refusal("REP_ANALYTIC_" + refusal.code, "%s: %s" % (sc["id"], refusal.detail)) from refusal
    files, table = build_assets(section)
    bundles = {}
    for sc, data in zip(SCENARIOS, section):
        decks = {side: make_deck(sc, side) for side in ("A", "B")}
        bundles[sc["id"]] = build_bundle(sc, decks, table)
    cases = {
        "schema": SCHEMA, "kind": "origin-formula synthetic; destination-validity-checked; NOT an executed golden; not accepted by the origin",
        "status": "oracle + importability, A validar: no consumer (S1b/S2) validated it, nothing rendered or measured; the public gate FX_TRANSITION_NOT_YET stays closed",
        "note": "Generated by scripts/pilot_dissolve_representable.py; edit nothing by hand (verify fails). BL082.4.17.",
        "plan": "docs/migration/p4b-representable-analytic-plan.md", "derived_from": s0_facts(),
        "conventions": conventions(), "thresholds": thresholds(),
        "hypotheses": {short: {"kind": kind, "rule": RULES[short], "long_name": long_name} for short, long_name, kind in HYPOTHESES},
        "scenarios": section, "bundles": bundles, "destination_reads": destination_reads(),
    }
    return cases, files


def expected_files():
    cases, assets = build_cases()
    cases_bytes = canonical(cases)
    manifest = build_manifest(cases_bytes, cases, assets)
    out = {CASES_NAME: cases_bytes, MANIFEST_NAME: canonical(manifest)}
    for name, data in assets.items():
        out[ASSET_DIR + "/" + name] = data
    return out


def build_manifest(cases_bytes, cases, assets):
    probes = sum(len(frame.get("probes", [])) for sc in cases["scenarios"] for frame in sc["frames"])
    return {
        "schema": SCHEMA,
        "purpose": "Provenance of dissolve-representable-cases.json (BL082.4.17): valid synthetic scenarios with the neighbour held outside its range. Oracle + importability; not the feature, not S2-validated.",
        "corpus": {"file": CASES_NAME, "sha256": sha256_of(cases_bytes), "generator": "scripts/pilot_dissolve_representable.py",
                   "spec": "scripts/pilot_dissolve_representable.spec.py", "regenerate": "scripts/pilot_dissolve_representable.py generate --origin <path to the origin repo>",
                   "check": "scripts/pilot_dissolve_representable.py generate --check   (writes nothing; offline)",
                   "counts": {"scenarios": len(cases["scenarios"]), "frames": sum(len(sc["frames"]) for sc in cases["scenarios"]), "probes": probes,
                              "hypotheses": len(cases["hypotheses"]), "bundles": len(cases["bundles"]), "assets": len(assets)}},
        "assets": [{"file": "%s/%s" % (ASSET_DIR, name), "sha256": sha256_of(data), "bytes": len(data)} for name, data in sorted(assets.items())],
        "source_pins": {
            "s0_manifest": {"file": S0_MANIFEST_NAME, **cases["derived_from"]},
            "note": "The kernels are the S0 transcription of the origin text at the pinned tip; the 33 sources and their blob digests are those of the S0 manifest. No new origin source was read for this delivery, and the origin repository was never modified, installed or executed. The destination files in `destination_reads` were READ in this repository.",
            "pin_status": "the pin d40ac3f7c0 may be absent from a clone: `verify --origin` reports PIN_COMMIT_ABSENT instead of claiming it verified; a blob read at the tip is not an origin execution",
        },
        "expectation_kinds": {"origin-formula": "the S0 kernels evaluated here; the origin was not run",
                              "destination-validity-checked": "the bundles import with the real core (Rust test); that checks importability and bounds only"},
        "thresholds": thresholds(),
        "exit_codes": {"0": "verified", "1": "finding", "2": "usage", "3": "ORIGIN_UNAVAILABLE", "4": "PIN_UNVERIFIED (with --require-pin)", "5": "INTERPRETER_NOT_REAL"},
        "blockers": [
            {"id": "ORIGIN_NOT_EXECUTED", "detail": "Expectations are transcribed formulas evaluated here, not the origin's outputs.", "consequence": "`origin-blob verified` is not `origin executed`."},
            {"id": "ORIGIN_GOLDEN_NOT_COMPARABLE", "detail": "No origin golden was read or compared; the origin harness was never run.", "consequence": "Pixel equivalence is origin-formula, never golden equivalence."},
            {"id": "ORACLE_NOT_S2_VALIDATED", "detail": "No consumer evaluated two decks or rendered a pixel against this data.", "consequence": "A ready oracle is not S2-validated; S1b/S2 consume it, and T06/T10/X may claim the A-pixel hold for these scenarios only after that."},
            {"id": "RUST_PROVES_IMPORTABILITY_ONLY", "detail": "The Rust test proves that the bundles import under the hidden admission policy and that bounds, light and inside-shot mapping match; the public gate still refuses.", "consequence": "No held-outside-shot evaluation by the core, no ScenePlan.transition, no pixel."},
            {"id": "LEGACY_PRIMARY_CAMERA_CLAMP_BL082_4_23", "detail": "The legacy primary plan keeps raw cameraIn + delta past a short camera; evaluate_shot_at_offset clamps. The new kits never use a short camera (REP_CAMERA_SHORT).", "consequence": "Both plan states are comparable on these scenarios; the legacy divergence is tracked as BL082.4.23 and untouched here."},
        ],
    }


# ---- commands ------------------------------------------------------------------------------------------------------


def say(stream, text):
    print(text, file=stream)


def report(findings, warnings):
    for warning in warnings:
        say(sys.stderr, "WARNING: " + warning)
    for finding in findings:
        say(sys.stderr, "FINDING: " + finding)


def compare(directory, wanted):
    differences = []
    for name, data in sorted(wanted.items()):
        path = directory / name
        if not path.exists():
            differences.append("missing: " + name)
        elif path.read_bytes() != data:
            differences.append("%s is not what the generator produces today" % name)
    for path in sorted((directory / ASSET_DIR).glob("*")):
        if path.relative_to(directory).as_posix() not in wanted:
            differences.append("unexpected asset: " + path.name)
    return differences


def cmd_generate(origin, directory, check, require_pin):
    if require_pin and origin is None:
        say(sys.stderr, "PIN_UNVERIFIED: --require-pin needs --origin PATH; offline the pin is not checked")
        return EXIT_PIN_UNVERIFIED
    if origin is None and not check:
        say(sys.stderr, "ORIGIN_UNAVAILABLE: `generate` re-checks the origin text the kernels transcribe; pass --origin PATH (or use --check offline)")
        return EXIT_ORIGIN_UNAVAILABLE
    findings, warnings = [], []
    if origin is not None:
        try:
            findings, warnings = C.check_origin(origin, require_pin)
        except C.OriginUnavailable as error:
            say(sys.stderr, "ORIGIN_UNAVAILABLE: %s" % error)
            return EXIT_ORIGIN_UNAVAILABLE
    if findings:
        report(findings, warnings)
        return EXIT_PIN_UNVERIFIED if findings == ["PIN_UNVERIFIED"] else EXIT_FINDING
    try:
        wanted = expected_files()
    except Refusal as refusal:
        say(sys.stderr, "FINDING: the generator refused to build the scenarios: %s" % refusal)
        return EXIT_FINDING
    if check:
        differences = compare(directory, wanted)
        report(differences, warnings)
        if differences:
            return EXIT_FINDING
        say(sys.stdout, "pilot-dissolve-representable --check: the generator reproduces %d files byte for byte (nothing written)" % len(wanted))
        return EXIT_OK
    (directory / ASSET_DIR).mkdir(parents=True, exist_ok=True)
    for name, data in wanted.items():
        (directory / name).write_bytes(data)
    report([], warnings)
    say(sys.stdout, "wrote %d files (%s, %s and %d assets)" % (len(wanted), CASES_NAME, MANIFEST_NAME, len(wanted) - 2))
    return EXIT_OK


def cmd_verify(origin, directory, require_pin):
    if require_pin and origin is None:
        say(sys.stderr, "PIN_UNVERIFIED: --require-pin needs --origin PATH; offline the pin is not checked")
        return EXIT_PIN_UNVERIFIED
    cases_path, manifest_path = directory / CASES_NAME, directory / MANIFEST_NAME
    if not cases_path.exists() or not manifest_path.exists():
        say(sys.stderr, "FINDING: missing cases or manifest: run `generate`")
        return EXIT_FINDING
    findings, warnings = [], []
    cases_bytes = cases_path.read_bytes()
    try:
        manifest = json.loads(manifest_path.read_bytes().decode("utf-8"))
        recorded = manifest["corpus"]["sha256"]
    except (ValueError, KeyError, TypeError) as error:
        say(sys.stderr, "FINDING: the manifest is not readable: %s" % error)
        return EXIT_FINDING
    if sha256_of(cases_bytes) != recorded:
        findings.append("%s was edited: its sha256 is not the manifest's" % CASES_NAME)
    for item in manifest.get("assets", []):
        path = directory / item["file"]
        if not path.exists() or sha256_of(path.read_bytes()) != item["sha256"]:
            findings.append("asset edited or missing: " + item["file"])
    try:
        findings += compare(directory, expected_files())
    except Refusal as refusal:
        findings.append("the generator refused to rebuild: %s" % refusal)
    blockers = {item.get("id") for item in manifest.get("blockers", [])}
    for needed in ("ORIGIN_GOLDEN_NOT_COMPARABLE", "ORIGIN_NOT_EXECUTED", "ORACLE_NOT_S2_VALIDATED"):
        if needed not in blockers:
            findings.append("blocker dropped from the manifest: " + needed)
    if b"origin-output" in cases_bytes:
        findings.append("the cases call an expectation an origin output")
    mode = "offline (origin blobs not re-read; pass --origin to re-check them)"
    if origin is not None:
        try:
            more, warnings = C.check_origin(origin, require_pin)
        except C.OriginUnavailable as error:
            report(findings, [])
            say(sys.stderr, "ORIGIN_UNAVAILABLE: %s" % error)
            return EXIT_ORIGIN_UNAVAILABLE
        findings += more
        absent = any(warning.startswith("PIN_COMMIT_ABSENT:") for warning in warnings)
        mode = "against the origin blobs (origin-blob verified; %s; the origin was NOT executed)" % (
            "pin NOT verified (attestation only)" if absent else "pin commit verified")
    report(findings, warnings)
    if findings:
        return EXIT_PIN_UNVERIFIED if findings == ["PIN_UNVERIFIED"] else EXIT_FINDING
    say(sys.stdout, "pilot-dissolve-representable verified %s: %d scenarios, %d assets" % (mode, manifest["corpus"]["counts"]["scenarios"], len(manifest["assets"])))
    return EXIT_OK


def matrix_text(cases):
    lines = ["%-40s %s" % ("scenario", " ".join("%-8s" % short for short in SHORTS))]
    for sc in cases["scenarios"]:
        cells = []
        for short in SHORTS:
            row = sc["matrix"][short]
            cells.append("%-8s" % ({"rejects": "R", "agrees": "a", "partial": "p"}[row["status"]] + (":%d/%d" % (row["rejecting_probes"], row["rejecting_frames"]) if row["kind"] != "state" else ":%d" % len(row["state_frames"]))))
        lines.append("%-40s %s" % (sc["id"], " ".join(cells)))
    return "\n".join(lines)


def cmd_matrix():
    try:
        cases, _ = build_cases()
    except Refusal as refusal:
        say(sys.stderr, "FINDING: the generator refused to build the scenarios: %s" % refusal)
        return EXIT_FINDING
    say(sys.stdout, matrix_text(cases))
    return EXIT_OK


def main(argv=None):
    problem = C.interpreter_problem()
    if problem:
        say(sys.stderr, "FINDING: INTERPRETER_NOT_REAL: %s" % problem)
        return EXIT_BAD_INTERPRETER
    say(sys.stderr, "python: %s" % sys.version.replace("\n", " "))
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    for name in ("generate", "verify"):
        command = sub.add_parser(name)
        command.add_argument("--origin", type=Path, default=None)
        command.add_argument("--out", type=Path, default=None)
        command.add_argument("--require-pin", action="store_true")
        command.add_argument("--check", action="store_true")
    sub.add_parser("matrix")
    args = parser.parse_args(argv)
    if args.command == "matrix":
        return cmd_matrix()
    directory = args.out or FIXTURES
    if args.command == "generate":
        return cmd_generate(args.origin, directory, args.check, args.require_pin)
    return cmd_verify(args.origin, directory, args.require_pin)


if __name__ == "__main__":
    sys.exit(main())
