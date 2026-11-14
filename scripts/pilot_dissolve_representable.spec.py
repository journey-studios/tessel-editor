"""Tests of the representable analytic reference X (BL082.4.17): `scripts/pilot_dissolve_representable.py` and
`docs/migration/fixtures/pilot-fx/dissolve-representable-{cases,manifest}.json`. No editor, GPU, runtime, engine, Cargo,
Node or origin execution is needed (the origin blobs are never read here).

Five layers, none of which compares the generator only with itself:

1. HAND VECTORS written here as literals (the plan's section 9: seeds, flicker levels at the real, instance, camera and
   clamped clocks, the weights, the projected marker centre, one lit pixel worked with Decimal, clocks counted on fingers).
2. An INDEPENDENT REFERENCE written in this file from the same formulas but with other means: unsigned masked integers for
   the 32-bit hash, `fractions.Fraction` for seconds, noise, easing and weights, `decimal.Decimal` (40 digits) for the
   sRGB curves, the projection and the light, and a closed form for an axis-aligned camera that holds or trucks along +x.
   It recomputes EVERY recorded state, probe, deck code and `rejects` list of the cases file, hypothesis by hypothesis.
3. A CONSUMER GATE (1/255 per channel, |mean signed delta| <= 0.25 with at least 24 probes) run against fake products that
   implement each wrong rule: it must FAIL for the hypotheses a scenario must reject, PASS for those that must agree and
   PASS for the truth.
4. MUTANTS of the generator itself (a scenario with a short instance or camera, a claim without power, a control that
   disagrees, a wrong held pattern, a hypothesis that leaks into the primary deck, a broken flag, a broken flicker): each
   must refuse (typed code) or change the data, and the formula mutants must also fail the hand vectors.
5. STRUCTURE: scenario coverage the plan asks for, determinism, byte identity with the committed files, valid PNGs and
   hashes, UTF-8/LF, no bare `assert`, imports.

The spec REFUSES a Microsoft Store stub as the interpreter (a refusal is never a skip), prints `sys.version`, and uses
`unittest` assertions, never a bare `assert`, so it still tests under `python -O`.
"""
import ast
import copy
import hashlib
import importlib.util
import json
import struct
import sys
import unittest
import zlib
from decimal import Decimal, getcontext
from fractions import Fraction
from functools import lru_cache
from pathlib import Path
from unittest.mock import patch

HERE = Path(__file__).parent
GENERATOR_PATH = HERE / "pilot_dissolve_representable.py"


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, HERE / filename)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


m = load("pilot_dissolve_analytic", "pilot_dissolve_analytic.py")
d = load("pilot_dissolve_corpus", "pilot-dissolve-corpus.py")
g = load("pilot_dissolve_representable", "pilot_dissolve_representable.py")

_problem = d.interpreter_problem()
if _problem:
    sys.stderr.write("INTERPRETER_NOT_REAL: %s\n" % _problem)
    raise SystemExit(d.EXIT_BAD_INTERPRETER)
sys.stderr.write("python: %s\n" % sys.version.replace("\n", " "))

getcontext().prec = 40
CASES_BYTES = (g.FIXTURES / g.CASES_NAME).read_bytes()
CASES = json.loads(CASES_BYTES.decode("utf-8"))
SCENARIOS = {item["id"]: item for item in CASES["scenarios"]}
SHORTS = list(g.SHORTS)
W, H = 640, 360
TAN = None


# ---- layer 2: the independent reference ----------------------------------------------------------------------


def d_sin_cos(x):
    term, total, n = x, x, 1
    while abs(term) > Decimal(10) ** -38:
        term = -term * x * x / ((2 * n) * (2 * n + 1))
        total += term
        n += 1
    term, cos, n = Decimal(1), Decimal(1), 1
    while abs(term) > Decimal(10) ** -38:
        term = -term * x * x / ((2 * n - 1) * (2 * n))
        cos += term
        n += 1
    return total, cos


PI = Decimal("3.14159265358979323846264338327950288419716939937510582097494")
_sin, _cos = d_sin_cos(PI * 25 / 180)
TAN = _sin / _cos
ASPECT = Decimal(W) / Decimal(H)
MASK = 0xFFFFFFFF


def u_hash(n, salt):
    h = (((n & MASK) * 0x9E3779B1) & MASK) ^ (((salt & MASK) * 0x85EBCA6B) & MASK)
    h ^= h >> 15
    h = (h * 0x2C1B3C6D) & MASK
    h ^= h >> 12
    h = (h * 0x297A2D39) & MASK
    h ^= h >> 15
    return Fraction(h, 1 << 32)


def u_seed(light_id):
    seed = 7
    for char in light_id:
        seed = (seed * 31 + ord(char)) & MASK
    return seed


def u_noise(seconds, seed):
    whole = seconds.numerator // seconds.denominator
    fraction = seconds - whole
    low, high = u_hash(whole, seed), u_hash(whole + 1, seed)
    return low + (high - low) * fraction * fraction * (3 - 2 * fraction)


def u_level(light_id, flicker, seconds):
    flicker = Fraction(flicker).limit_denominator(1000)
    if flicker <= 0:
        return Fraction(1)
    seed = u_seed(light_id)
    return 1 - flicker * Fraction(45, 100) * (Fraction(6, 10) * u_noise(seconds * 3, seed) + Fraction(4, 10) * u_noise(seconds * 13, seed + 101))


def dec(fraction):
    return Decimal(fraction.numerator) / Decimal(fraction.denominator)


def u_decode(code):
    v = Decimal(code) / 255
    return v / Decimal("12.92") if v <= Decimal("0.04045") else ((v + Decimal("0.055")) / Decimal("1.055")) ** Decimal("2.4")


def u_encode(linear):
    v = min(Decimal(1), max(Decimal(0), linear))
    return v * Decimal("12.92") if v <= Decimal("0.0031308") else Decimal("1.055") * v ** (1 / Decimal("2.4")) - Decimal("0.055")


def u_round(unrounded):
    return int((unrounded + Decimal("0.5")).to_integral_value(rounding="ROUND_FLOOR"))


def u_event(instance, start, duration):
    if instance < start:
        return Fraction(0)
    if instance >= start + duration:
        return Fraction(1)
    return Fraction(instance - start, duration)


def u_clamp(wanted, frames):
    return max(0, min(frames - 1, wanted))


def u_state(sc, side, f, hypothesis=None, primary=None):
    """Instance, camera and light seconds of the deck of `side` at edit frame `f` (truth when `hypothesis` is None)."""
    shots = sc["shots"]
    shot = shots[side]
    num, den = sc["fps"]
    numbers = shots[primary] if hypothesis == "PRI" else shot
    delta = f - numbers["start"]
    inst_wanted, cam_wanted = numbers["inst_start"] + delta, numbers["cam_in"] + delta
    inst = u_clamp(inst_wanted, shot["inst_frames"])
    cam = u_clamp(cam_wanted, shot["cam_frames"])
    if hypothesis == "SWP":
        inst = u_clamp(numbers["cam_in"] + delta, shot["cam_frames"])
        cam = u_clamp(numbers["inst_start"] + delta, shot["inst_frames"])
    elif hypothesis == "PIN":
        low = max(-numbers["inst_start"], -numbers["cam_in"])
        high = min(shot["inst_frames"] - 1 - numbers["inst_start"], shot["cam_frames"] - 1 - numbers["cam_in"])
        joint = max(low, min(high, delta))
        inst, cam = u_clamp(numbers["inst_start"] + joint, shot["inst_frames"]), u_clamp(numbers["cam_in"] + joint, shot["cam_frames"])
    elif hypothesis == "UNH":
        inst = max(0, inst_wanted)
    seconds = Fraction(f * den, num)
    if hypothesis == "LIN":
        seconds = Fraction(inst * den, num)
    elif hypothesis == "LSR":
        seconds = Fraction((f - shot["start"]) * den, num)
    elif hypothesis == "LCC":
        seconds = Fraction(cam * den, num)
    elif hypothesis == "LCE":
        seconds = Fraction(max(shot["start"], min(shot["end"] - 1, f)) * den, num)
    elif hypothesis == "LNF":
        seconds = Fraction(f, round(Fraction(num, den)))
    return {"instance": inst, "camera": cam, "seconds": seconds, "instance_wanted": inst_wanted, "camera_wanted": cam_wanted}


def u_pixel(deck, state, x, y):
    clip = deck["camera"]
    segment = clip["segments"][0]
    frames = segment["frames"]
    if clip["target"][0] != clip["position"][0] or clip["target"][1] != clip["position"][1] or clip["target"][2] != 0 or clip["position"][2] != 6:
        raise AssertionError("the closed form needs a camera looking down -z from z = 6")
    shift = Decimal(0) if segment["recipe"] == "hold" else Decimal(repr(segment["distance"])) * min(state["camera"], frames - 1) / max(1, frames - 1)
    cx, cy, cz = Decimal(repr(clip["position"][0])) + shift, Decimal(repr(clip["position"][1])), Decimal(6)
    ndc_x = Decimal(2 * x + 1) / W - 1
    ndc_y = 1 - Decimal(2 * y + 1) / H
    chosen = None
    for plane in sorted(deck["planes"], key=lambda item: item["render_order"]):
        position = [Decimal(repr(c)) for c in plane["position"]]
        if "event" in plane:
            event = plane["event"]
            progress = dec(u_event(state["instance"], event["start"], event["duration"]))
            position = [position[i] + Decimal(repr(event["delta"][i])) * progress for i in range(3)]
        depth = cz - position[2]
        hx, hy = cx + ndc_x * TAN * ASPECT * depth, cy + ndc_y * TAN * depth
        if abs(hx - position[0]) <= Decimal(repr(plane["width"])) / 2 and abs(hy - position[1]) <= Decimal(repr(plane["height"])) / 2:
            chosen = (plane, (hx, hy, position[2]))
    if chosen is None:
        return None, [0, 0, 0]
    plane, hit = chosen
    base = plane["colour8"]
    if not deck["lights"]:
        return plane["id"], list(base)
    total = [Decimal(0)] * 3
    for lamp in deck["lights"]:
        level = dec(u_level(lamp["id"], lamp["flicker"], state["seconds"]))
        to_light = [Decimal(repr(lamp["position"][i])) - hit[i] for i in range(3)]
        distance = sum(c * c for c in to_light).sqrt()
        facing = max(Decimal(0), to_light[2] / distance)
        falloff = 1 / (1 + (distance / Decimal(repr(lamp["radius"]))) ** 2)
        amount = facing * falloff * falloff * Decimal(repr(lamp["intensity"])) * level
        for channel in range(3):
            total[channel] += Decimal(repr(lamp["colour"][channel])) * amount
    codes = []
    for channel in range(3):
        lit = u_decode(base[channel]) * (1 + total[channel])
        if lit > 1:
            raise AssertionError("clipped")
        codes.append(u_round(u_encode(lit) * 255))
    return plane["id"], codes


def u_progress(k, duration, shifted):
    t = Fraction(k, duration) if shifted else Fraction(2 * k + 1, 2 * duration)
    return 4 * t * t * t if t < Fraction(1, 2) else 1 - (-2 * t + 2) ** 3 / 2


def u_mix(a8, b8, k, duration, shifted=False):
    p = u_progress(k, duration, shifted)
    s = p * p * (3 - 2 * p)
    out = []
    for a_code, b_code in zip(a8, b8):
        a, b = u_decode(a_code), u_decode(b_code)
        lin = (a * (1 - dec(p)) + b * dec(p)) * 0 + (a * (1 - dec(s)) + b * dec(s))  # amount 1: the shaped branch
        out.append(u_round(u_encode(lin) * 255))
    return out


@lru_cache(maxsize=None)
def u_deck_cached(scenario_id, side, instance, camera, seconds, x, y):
    sc = SCENARIOS[scenario_id]
    return u_pixel(sc["decks"][side], {"instance": instance, "camera": camera, "seconds": seconds}, x, y)


def u_out(sc, states, x, y, k, mix="plain"):
    decks = {}
    for side in ("A", "B"):
        s = states[side]
        decks[side] = u_deck_cached(sc["id"], side, s["instance"], s["camera"], s["seconds"], x, y)
    a8, b8 = decks["A"][1], decks["B"][1]
    if mix == "DSW":
        return u_mix(b8, a8, k, 12), decks
    return u_mix(a8, b8, k, 12, shifted=(mix == "NOH")), decks


def u_worlds(sc, f):
    cut = sc["shots"]["A"]["end"]
    primary = "A" if f < cut else "B"
    neighbour = "B" if primary == "A" else "A"
    truth = {side: u_state(sc, side, f) for side in ("A", "B")}
    worlds = {}
    for short in SHORTS:
        if short in ("FLG", "DSW", "NOH"):
            continue
        world = dict(truth)
        world[neighbour] = u_state(sc, neighbour, f, short, primary)
        worlds[short] = world
    return truth, worlds, primary, neighbour


def u_flagged(sc, truth):
    out = {}
    for side, state in truth.items():
        shot = sc["shots"][side]
        out[side] = (state["instance_wanted"] >= shot["inst_frames"] - 1 or state["instance_wanted"] <= 0,
                     state["camera_wanted"] >= shot["cam_frames"] - 1 or state["camera_wanted"] <= 0)
    return out


def hex_of(code):
    return "#%02x%02x%02x" % tuple(code)


def separation(a, b):
    return max(abs(a[i] - b[i]) for i in range(3))


# ---- layer 3: the consumer gate ------------------------------------------------------------------------------


def gate(expected, product):
    """1/255 per channel on every probe and |mean signed delta| <= 0.25 per channel (with at least 24 probes)."""
    if any(abs(p[i] - e[i]) > 1 for e, p in zip(expected, product) for i in range(3)):
        return False
    if len(expected) >= 24:
        for i in range(3):
            if abs(sum(p[i] - e[i] for e, p in zip(expected, product)) / len(expected)) > 0.25:
                return False
    return True


@lru_cache(maxsize=None)
def scenario_products(scenario_id):
    """Per scenario: the expected codes of every probe and, per hypothesis, the fake product's codes (independent)."""
    sc = SCENARIOS[scenario_id]
    expected, products = [], {short: [] for short in SHORTS if short != "FLG"}
    probes = 0
    for frame in sc["frames"]:
        if not frame["in_window"]:
            continue
        f, k = frame["edit_frame"], frame["k"]
        truth, worlds, primary, neighbour = u_worlds(sc, f)
        for probe in frame["probes"]:
            x, y = [int(v) for v in probe["pixel"].split(",")]
            code = [int(probe["expected"][1 + 2 * i:3 + 2 * i], 16) for i in range(3)]
            expected.append(code)
            probes += 1
            for short in products:
                if short in ("DSW", "NOH"):
                    out, _ = u_out(sc, truth, x, y, k, short)
                else:
                    out, _ = u_out(sc, worlds[short], x, y, k)
                products[short].append(out)
    return expected, products


# ---- the tests -------------------------------------------------------------------------------------------------


class HandVectors(unittest.TestCase):
    """Layer 1: literals worked outside the generator (plan section 9)."""

    def test_seeds_of_the_origin(self):
        self.assertEqual((u_seed("lamp-a"), u_seed("lamp-b")), (807477363, 807477364))
        self.assertEqual((m.seed_of("lamp-a"), m.seed_of("lamp-b")), (807477363, 807477364))

    def test_flicker_levels_at_each_clock(self):
        def level(num, den, lamp="lamp-a"):
            return float(u_level(lamp, 1, Fraction(num, den)))
        vectors = [
            (level(284, 24), 0.811566769), (level(493, 24), 0.970547388), (level(402, 24), 0.962061105), (level(283, 24), 0.895748195),
            (level(463, 24, "lamp-b"), 0.832962015), (level(0, 1, "lamp-b"), 0.739396138), (level(475, 24, "lamp-b"), 0.752307966),
            (level(1802 * 1001, 30000), 0.725952789), (level(1802, 30), 0.622278394), (level(1799 * 1001, 30000), 0.590289496),
        ]
        for got, want in vectors:
            self.assertAlmostEqual(got, want, places=8)

    def test_the_generator_kernel_agrees_with_the_hand_levels(self):
        self.assertAlmostEqual(m.flicker_level("lamp-a", 1.0, 284 / 24), 0.811566769, places=8)
        self.assertAlmostEqual(m.flicker_level("lamp-a", 1.0, 493 / 24), 0.970547388, places=8)
        self.assertAlmostEqual(m.flicker_level("lamp-b", 1.0, 463 / 24), 0.832962015, places=8)
        self.assertAlmostEqual(m.flicker_level("lamp-a", 1.0, 1802 * 1001 / 30000), 0.725952789, places=8)
        self.assertAlmostEqual(m.flicker_level("lamp-a", 1.0, 1802 / 30), 0.622278394, places=8)

    def test_weights(self):
        want = [0, 0.000182, 0.003830, 0.027595, 0.114713, 0.330723, 0.669277, 0.885287, 0.972405, 0.996170, 0.999818, 1]
        got = [float(u_progress(k, 12, False) ** 2 * (3 - 2 * u_progress(k, 12, False))) for k in range(12)]
        for a, b in zip(got, want):
            self.assertAlmostEqual(a, b, places=6)
        recorded = [frame["transition"]["weight"] for frame in SCENARIOS["rep-a-tail-both-held-light"]["frames"] if frame["in_window"]]
        for a, b in zip(recorded, want):
            self.assertAlmostEqual(a, b, places=6)

    def test_projection_and_the_lit_pixel(self):
        self.assertTrue(str(TAN).startswith("0.46630765815499859283"))
        centre_x = (Decimal("1.5") / (6 * TAN * ASPECT) + 1) / 2 * W
        centre_y = (1 - Decimal("-0.5") / (6 * TAN)) / 2 * H
        self.assertEqual((str(centre_x)[:13], str(centre_y)[:13]), ("416.502811422", "212.167603807"))
        frame = next(item for item in SCENARIOS["rep-a-tail-both-held-light"]["frames"] if item["edit_frame"] == 284)
        self.assertEqual(frame["decks"]["A"]["marker"]["centre_px"], "416.502811423,212.167603808")
        # deck A, pixel (160, 90): hit (-3.3056, 1.8549, -2), d 4.4711, n.l 0.8946, falloff^2 n.l 0.81149, code (221.475, 223.096, 224.523)
        probe = next(item for item in frame["probes"] if item["pixel"] == "160,90")
        self.assertEqual(probe["deck_a"], "backdrop #dddfe1")
        sc = SCENARIOS["rep-a-tail-both-held-light"]
        state = {"instance": 493, "camera": 402, "seconds": Fraction(284, 24)}
        cover, codes = u_pixel(sc["decks"]["A"], state, 160, 90)
        self.assertEqual((cover, codes), ("backdrop", [221, 223, 225]))
        level = u_level("lamp-a", 1, Fraction(284, 24))
        amount = Decimal("0.81148914273") * 20 * dec(level)
        for base, colour, want in ((64, Decimal(1), "221.4750"), (76, Decimal("0.7"), "223.0955"), (97, Decimal("0.4"), "224.5230")):
            self.assertEqual(str(u_encode(u_decode(base) * (1 + colour * amount)) * 255)[:8], want)

    def test_clocks_counted_on_fingers(self):
        def deck(scenario, f, side):
            return next(item for item in SCENARIOS[scenario]["frames"] if item["edit_frame"] == f)["decks"][side]
        a = "rep-a-tail-both-held-light"
        self.assertEqual([(deck(a, 283, "A")[key]) for key in ("instance_frame", "camera_clip_frame", "held_instance", "held_camera")], [493, 402, False, False])
        self.assertEqual([(deck(a, 284, "A")[key]) for key in ("instance_wanted", "instance_frame", "camera_wanted", "camera_clip_frame", "held_instance", "held_camera")], [494, 493, 403, 402, True, True])
        b = "rep-b-tail-instance-held-camera-runs"
        self.assertEqual([(deck(b, 29, "A")[key]) for key in ("instance_frame", "held_instance")], [33, False])
        self.assertEqual([(deck(b, 30, "A")[key]) for key in ("instance_wanted", "instance_frame", "held_instance", "camera_clip_frame", "held_camera")], [34, 33, True, 30, False])
        self.assertEqual([(deck(b, 41, "A")[key]) for key in ("instance_frame", "camera_clip_frame", "held_camera")], [33, 41, False])
        b2 = "rep-b2-tail-instance-edge-camera-runs"
        self.assertEqual([(deck(b2, 34, "A")[key]) for key in ("instance_wanted", "instance_frame", "held_instance")], [38, 38, False])
        self.assertEqual([(deck(b2, 35, "A")[key]) for key in ("instance_wanted", "instance_frame", "held_instance")], [39, 38, True])
        c = "rep-c-head-both-held-light"
        self.assertEqual([(deck(c, 463, "B")[key]) for key in ("instance_wanted", "instance_frame", "camera_clip_frame", "held_instance", "held_camera")], [-12, 0, 0, True, True])
        self.assertEqual([(deck(c, 474, "B")[key]) for key in ("instance_wanted", "instance_frame", "held_instance")], [-1, 0, True])
        self.assertEqual([(deck(c, 475, "B")[key]) for key in ("instance_frame", "camera_clip_frame", "held_instance", "held_camera")], [0, 0, False, False])
        n = "rep-d-ntsc-tail-held-light"
        self.assertEqual(next(item for item in SCENARIOS[n]["frames"] if item["edit_frame"] == 1800)["edit_seconds"]["rational"], "3003/50")
        self.assertEqual(next(item for item in SCENARIOS[n]["frames"] if item["edit_frame"] == 1802)["edit_seconds"]["rational"], "901901/15000")
        self.assertEqual([(deck(n, 1800, "A")[key]) for key in ("instance_frame", "camera_clip_frame", "held_instance")], [1799, 1799, True])
        self.assertEqual(next(item for item in SCENARIOS[a]["frames"] if item["edit_frame"] == 284)["edit_seconds"]["rational"], "71/6")

    def test_windows(self):
        self.assertEqual([SCENARIOS[i]["transition"]["window"] for i in ("rep-a-tail-both-held-light", "rep-c-head-both-held-light", "rep-x-center-both-mixed")],
                         [{"start": 284, "end_exclusive": 296}, {"start": 463, "end_exclusive": 475}, {"start": 24, "end_exclusive": 36}])

    def test_the_two_clock_hypotheses_are_not_the_same_by_construction(self):
        """The marker of rep-b: held instance 33 gives p = 0.5, y = 0; the joint clamp moves the camera by one frame per k."""
        deck = SCENARIOS["rep-b-tail-instance-held-camera-runs"]["decks"]["A"]
        marker = next(plane for plane in deck["planes"] if plane["id"] == "marker")
        self.assertEqual((marker["position"], marker["event"]["start"]), ([10.8, -2.4, 0.0], 27))
        self.assertEqual(u_event(33, 27, 12), Fraction(1, 2))
        self.assertAlmostEqual(deck["camera"]["segments"][0]["distance"], 16.92, places=9)


class IndependentReference(unittest.TestCase):
    """Layer 2: every recorded state, probe, deck code and `rejects` list, recomputed with other means."""

    def test_states_seconds_and_weights_of_every_frame(self):
        frames = 0
        for sc in CASES["scenarios"]:
            for frame in sc["frames"]:
                f = frame["edit_frame"]
                num, den = sc["fps"]
                seconds = Fraction(f * den, num)
                self.assertEqual(frame["edit_seconds"]["rational"], "%d/%d" % (seconds.numerator, seconds.denominator))
                self.assertAlmostEqual(frame["edit_seconds"]["decimal"], float(seconds), places=9)
                truth, _, primary, neighbour = u_worlds(sc, f)
                self.assertEqual(frame["primary"], primary)
                for side in ("A", "B"):
                    state, deck = truth[side], frame["decks"][side]
                    shot = sc["shots"][side]
                    self.assertEqual((deck["instance_wanted"], deck["instance_frame"], deck["camera_wanted"], deck["camera_clip_frame"]),
                                     (state["instance_wanted"], state["instance"], state["camera_wanted"], state["camera"]), (sc["id"], side, f))
                    self.assertEqual(deck["held_instance"], not (0 <= state["instance_wanted"] < shot["inst_frames"]))
                    self.assertEqual(deck["held_camera"], not (0 <= state["camera_wanted"] < shot["cam_frames"]))
                    self.assertEqual(deck["fx_edge"], not (shot["start"] <= f < shot["end"]))
                    self.assertAlmostEqual(deck["scene_seconds"], float(seconds), places=9)
                    for row, lamp in zip(deck["lights"], sc["decks"][side]["lights"]):
                        level = float(u_level(lamp["id"], lamp["flicker"], seconds))
                        self.assertAlmostEqual(row["flicker_level"], level, places=8)
                        self.assertAlmostEqual(row["effective_intensity"], level * lamp["intensity"], places=7)
                if frame["in_window"]:
                    self.assertEqual(frame["neighbour"], neighbour)
                    p = u_progress(frame["k"], 12, False)
                    s = p * p * (3 - 2 * p)
                    self.assertAlmostEqual(frame["transition"]["progress"], float(p), places=9)
                    self.assertAlmostEqual(frame["transition"]["shaped"], float(s), places=9)
                    self.assertAlmostEqual(frame["transition"]["weight"], float(s), places=9)
                frames += 1
        self.assertEqual(frames, 7 * 14)

    def test_every_probe_deck_code_expected_code_and_rejects_list(self):
        probes = 0
        for sc in CASES["scenarios"]:
            for frame in sc["frames"]:
                if not frame["in_window"]:
                    self.assertNotIn("probes", frame)
                    continue
                f, k = frame["edit_frame"], frame["k"]
                truth, worlds, primary, neighbour = u_worlds(sc, f)
                pixels = [probe["pixel"] for probe in frame["probes"]]
                self.assertEqual(len(pixels), len(set(pixels)), "a probe is a distinct pixel per frame")
                for probe in frame["probes"]:
                    x, y = [int(v) for v in probe["pixel"].split(",")]
                    out, decks = u_out(sc, truth, x, y, k)
                    self.assertEqual(probe["deck_a"], "%s %s" % (decks["A"][0], hex_of(decks["A"][1])), (sc["id"], f, probe["id"]))
                    self.assertEqual(probe["deck_b"], "%s %s" % (decks["B"][0], hex_of(decks["B"][1])), (sc["id"], f, probe["id"]))
                    self.assertEqual(probe["expected"], hex_of(out), (sc["id"], f, probe["id"]))
                    rejected = set()
                    for short in SHORTS:
                        if short == "FLG":
                            continue
                        wrong = u_out(sc, truth, x, y, k, short)[0] if short in ("DSW", "NOH") else u_out(sc, worlds[short], x, y, k)[0]
                        if separation(wrong, out) >= 4:
                            rejected.add(short)
                    self.assertEqual(set(probe.get("rejects", "").split(",")) - {""}, rejected, (sc["id"], f, probe["id"]))
                    probes += 1
        self.assertGreater(probes, 300)

    def test_the_matrix_is_the_count_of_the_recorded_probes(self):
        for sc in CASES["scenarios"]:
            for short in SHORTS:
                row = sc["matrix"][short]
                if row["kind"] == "state":
                    flagged = []
                    for frame in sc["frames"]:
                        truth = u_worlds(sc, frame["edit_frame"])[0]
                        flags = u_flagged(sc, truth)
                        if any(flags[side] != (frame["decks"][side]["held_instance"], frame["decks"][side]["held_camera"]) for side in ("A", "B")):
                            flagged.append(frame["edit_frame"])
                    self.assertEqual(row["state_frames"], flagged, (sc["id"], short))
                    self.assertEqual(row["status"], "rejects" if flagged else "agrees")
                    continue
                hit = [frame["edit_frame"] for frame in sc["frames"] for probe in frame.get("probes", []) if short in probe.get("rejects", "").split(",")]
                total = sum(len(frame.get("probes", [])) for frame in sc["frames"])
                self.assertEqual((row["rejecting_probes"], row["rejecting_frames"], row["probes"]), (len(hit), len(set(hit)), total), (sc["id"], short))
                if len(hit) >= 3 and len(set(hit)) >= 2:
                    self.assertEqual(row["status"], "rejects")
                elif row["max_separation"] <= 1:
                    self.assertEqual(row["status"], "agrees")
                else:
                    self.assertEqual(row["status"], "partial")

    def test_neighbour_only_isolation_in_the_recomputation(self):
        """A clock hypothesis changes the neighbour's state and never the primary's (the primary is the truth both sides)."""
        for sc in CASES["scenarios"]:
            for frame in sc["frames"]:
                if not frame["in_window"]:
                    continue
                truth, worlds, primary, neighbour = u_worlds(sc, frame["edit_frame"])
                for short, world in worlds.items():
                    self.assertEqual(world[primary], truth[primary], (sc["id"], short, frame["edit_frame"]))


class ConsumerGate(unittest.TestCase):
    """Layer 3: fake products that implement each wrong rule."""

    def test_the_truth_passes_and_each_claim_is_decided_as_pre_registered(self):
        for sc in CASES["scenarios"]:
            expected, products = scenario_products(sc["id"])
            self.assertTrue(gate(expected, [list(code) for code in expected]), "the truth passes")
            for short in sc["must_reject"]:
                if short == "FLG":
                    continue
                self.assertFalse(gate(expected, products[short]), (sc["id"], short, "must fail the gate"))
            for short in sc["must_agree"]:
                self.assertTrue(gate(expected, products[short]), (sc["id"], short, "must pass the gate"))

    def test_flg_fails_on_states(self):
        for sc in CASES["scenarios"]:
            row = sc["matrix"]["FLG"]
            if "FLG" in sc["must_reject"]:
                self.assertTrue(row["state_frames"], sc["id"])
            self.assertEqual(row["status"], "rejects" if row["state_frames"] else "agrees")

    def test_the_non_discriminating_variants_are_reported(self):
        reported = {sc["id"]: sc["reported_only"] for sc in CASES["scenarios"]}
        self.assertIn("LIN", reported["rep-ctl-real-both"])
        self.assertIn("LCC", reported["rep-x-center-both-mixed"])
        for sc in CASES["scenarios"]:
            self.assertEqual(set(sc["reported_only"]) | set(sc["must_reject"]) | set(sc["must_agree"]), set(SHORTS), sc["id"])
            for short in sc["reported_only"]:
                self.assertIn(sc["matrix"][short]["status"], ("rejects", "agrees", "partial"))

    def test_the_gate_itself(self):
        base = [[100, 100, 100]] * 30
        self.assertTrue(gate(base, [[100, 100, 100]] * 30))
        self.assertFalse(gate(base, [[102, 100, 100]] * 30))
        biased = [[101, 100, 100]] * 30
        self.assertFalse(gate(base, biased), "a constant +1 breaks the bias limit with at least 24 probes")
        self.assertTrue(gate(base[:3], [[101, 100, 100]] * 3), "the bias limit needs at least 24 probes")


class GeneratorMutants(unittest.TestCase):
    """Layer 4: the generator refuses what the plan forbids and cannot be fooled by a broken kernel."""

    def scenario(self, scenario_id):
        return copy.deepcopy(next(item for item in g.SCENARIOS if item["id"] == scenario_id))

    def refusal(self, sc):
        with self.assertRaises(g.Refusal) as caught:
            g.build_scenario(sc)
        return caught.exception.code

    def test_the_old_12_over_30_shape_is_refused(self):
        sc = self.scenario("rep-b-tail-instance-held-camera-runs")
        sc["shots"]["A"]["inst_frames"] = 12
        self.assertEqual(self.refusal(sc), "REP_SHOT_EXCEEDS_INSTANCE")

    def test_a_short_camera_is_refused(self):
        sc = self.scenario("rep-ctl-real-both")
        sc["shots"]["B"]["cam_frames"] = 40
        self.assertEqual(self.refusal(sc), "REP_CAMERA_SHORT")

    def test_non_integers_and_bad_tiling(self):
        sc = self.scenario("rep-a-tail-both-held-light")
        sc["shots"]["A"]["cam_in"] = 119.0
        self.assertEqual(self.refusal(sc), "REP_NOT_AN_INTEGER")
        sc = self.scenario("rep-a-tail-both-held-light")
        sc["shots"]["A"]["cam_in"] = -1
        self.assertEqual(self.refusal(sc), "REP_NOT_AN_INTEGER")
        sc = self.scenario("rep-a-tail-both-held-light")
        sc["edit"] = 400
        self.assertEqual(self.refusal(sc), "REP_SHOTS_DO_NOT_TILE")

    def test_a_claim_without_power_and_a_control_that_disagrees(self):
        sc = self.scenario("rep-a-tail-both-held-light")
        sc["rejects"] = sc["rejects"] + ["LSR"]  # A starts at 0: the shot-relative clock IS the edit clock
        sc["agrees"] = [short for short in sc["agrees"] if short != "LSR"]
        self.assertEqual(self.refusal(sc), "REP_NO_REFUSAL_POWER")
        sc = self.scenario("rep-a-tail-both-held-light")
        sc["agrees"] = sc["agrees"] + ["LIN"]
        sc["rejects"] = [short for short in sc["rejects"] if short != "LIN"]
        self.assertEqual(self.refusal(sc), "REP_CONTROL_DISAGREES")

    def test_nominal_fps_has_no_power_at_24_over_1(self):
        sc = self.scenario("rep-a-tail-both-held-light")
        self.assertIn("LNF", sc["agrees"])
        sc["rejects"] = sc["rejects"] + ["LNF"]
        sc["agrees"] = [short for short in sc["agrees"] if short != "LNF"]
        self.assertEqual(self.refusal(sc), "REP_NO_REFUSAL_POWER")

    def test_a_wrong_held_pattern_and_a_wrong_handles_mode(self):
        sc = self.scenario("rep-b-tail-instance-held-camera-runs")
        sc["held"] = {"instance": "all", "camera": "all"}
        self.assertEqual(self.refusal(sc), "REP_HELD_PATTERN")
        sc = self.scenario("rep-ctl-real-both")
        sc["handles"] = "hold"
        self.assertEqual(self.refusal(sc), "REP_HANDLES_HOLD_WITHOUT_A_HOLD")
        sc = self.scenario("rep-a-tail-both-held-light")
        sc["handles"] = "use"
        self.assertEqual(self.refusal(sc), "REP_HANDLES_USE_NEEDS_REAL_FRAMES")

    def test_a_hypothesis_that_leaks_into_the_primary_deck_is_caught(self):
        original = g.hypothesis_states

        def leaky(edit, f, short, truth, primary, neighbour):
            states = original(edit, f, short, truth, primary, neighbour)
            if short == "LIN":
                states[primary] = g.with_wanted(edit, primary, m.clocks_of(edit, primary, f, m.HYPOTHESES["LIGHT_ON_THE_INSTANCE_CLOCK"]["clock"]))
            return states
        with patch.object(g, "hypothesis_states", leaky):
            self.assertEqual(self.refusal(self.scenario("rep-a-tail-both-held-light")), "REP_ISOLATION_BROKEN")

    def test_a_broken_held_flag_is_refused(self):
        def limit(wanted, frames):
            frame = max(0, min(frames - 1, wanted))
            return frame, frame != wanted or wanted == frames - 1
        with patch.object(m, "limit_frame", limit):
            self.assertEqual(self.refusal(self.scenario("rep-a-tail-both-held-light")), "REP_HELD_FLAG")

    def test_a_primary_clamp_is_refused(self):
        """BL082.4.23: a deck inside its own shot must never be clamped."""
        original = m.limit_frame

        def clamp_early(wanted, frames):
            frame = max(0, min(frames - 2, wanted))
            return frame, frame != wanted
        with patch.object(m, "limit_frame", clamp_early):
            self.assertIn(self.refusal(self.scenario("rep-c-head-both-held-light")), ("REP_HELD_FLAG", "REP_PRIMARY_CLAMPED"))
        self.assertIsNotNone(original)

    def test_a_kernel_mutant_changes_the_data_and_fails_the_hand_vectors(self):
        before = g.canonical({"x": g.build_scenario(next(item for item in g.SCENARIOS if item["id"] == "rep-a-tail-both-held-light"))})

        original = m.flicker_level

        def whole_second(light_id, flicker, seconds):
            return original(light_id, flicker, float(int(seconds)))
        with patch.object(m, "flicker_level", whole_second):
            try:
                after = g.canonical({"x": g.build_scenario(next(item for item in g.SCENARIOS if item["id"] == "rep-a-tail-both-held-light"))})
            except g.Refusal:
                after = None
            self.assertNotEqual(before, after)
            self.assertNotAlmostEqual(m.flicker_level("lamp-a", 1.0, 284 / 24), 0.811566769, places=6)

    def test_shorts_agree_with_the_s0_oracle_and_with_the_plan(self):
        for short, long_name, kind in g.HYPOTHESES:
            if long_name:
                self.assertEqual(m.HYPOTHESES[long_name]["short"], short)
        self.assertEqual(SHORTS, ["PIN", "SWP", "PRI", "UNH", "FLG", "LIN", "LSR", "LCC", "LCE", "LNF", "DSW", "NOH"])

    def test_pre_registered_thresholds_are_the_s0_ones(self):
        t = CASES["thresholds"]
        self.assertEqual((t["min_separation_codes"], t["min_rejecting_probes"], t["min_rejecting_frames"], t["agree_max_codes"], t["flat_margin_px"]), (4, 3, 2, 1, 6))
        self.assertEqual(t["tolerance_for_the_implementation"]["max_abs_delta_8bit"], 1)
        self.assertEqual(t["tolerance_for_the_implementation"]["max_abs_mean_signed_delta_8bit"], 0.25)
        s0 = json.loads((g.FIXTURES / g.S0_CASES_NAME).read_text(encoding="utf-8"))["analytic_probes"]["thresholds"]
        for key in ("min_separation_codes", "min_rejecting_probes", "min_rejecting_frames", "agree_max_codes", "flat_margin_px", "tie_margin"):
            self.assertEqual(t[key], s0[key], key)


class Structure(unittest.TestCase):
    """Layer 5."""

    def test_coverage_the_plan_asks_for(self):
        self.assertEqual(sorted(SCENARIOS), sorted(["rep-a-tail-both-held-light", "rep-b-tail-instance-held-camera-runs", "rep-b2-tail-instance-edge-camera-runs",
                                                    "rep-c-head-both-held-light", "rep-d-ntsc-tail-held-light", "rep-x-center-both-mixed", "rep-ctl-real-both"]))
        for sc in CASES["scenarios"]:
            self.assertEqual(len(sc["frames"]), 14)
            window = sc["transition"]["window"]
            self.assertEqual([frame["edit_frame"] for frame in sc["frames"]], list(range(window["start"] - 1, window["end_exclusive"] + 1)))
            for side in ("A", "B"):
                shot = sc["shots"][side]
                length = shot["end"] - shot["start"]
                self.assertGreaterEqual(shot["inst_frames"], shot["inst_start"] + length, (sc["id"], side, "instance covers the shot"))
                self.assertGreaterEqual(shot["cam_frames"], shot["cam_in"] + length, (sc["id"], side, "camera covers the shot"))
                self.assertGreaterEqual(shot["cam_in"], 0)
            self.assertEqual(sorted(sc["decks"]), ["A", "B"])
        self.assertEqual(SCENARIOS["rep-a-tail-both-held-light"]["transition"]["alignment"], "start")
        self.assertEqual(SCENARIOS["rep-c-head-both-held-light"]["transition"]["alignment"], "end")
        self.assertEqual(SCENARIOS["rep-d-ntsc-tail-held-light"]["fps"], [30000, 1001])
        self.assertEqual(SCENARIOS["rep-a-tail-both-held-light"]["neighbour_held"], {"instance": "all", "camera": "all"})
        self.assertEqual(SCENARIOS["rep-b-tail-instance-held-camera-runs"]["neighbour_held"], {"instance": "all", "camera": "none"})
        self.assertEqual(SCENARIOS["rep-x-center-both-mixed"]["neighbour_held"], {"instance": "some", "camera": "some"})
        self.assertEqual(SCENARIOS["rep-ctl-real-both"]["neighbour_held"], {"instance": "none", "camera": "none"})
        for sc in CASES["scenarios"]:
            self.assertEqual(sc["transition"]["handles"], "use" if sc["id"] == "rep-ctl-real-both" else "hold")

    def test_the_bundles_are_complete_and_valid_in_shape(self):
        self.assertEqual(sorted(CASES["bundles"]), sorted(SCENARIOS))
        for sc in CASES["scenarios"]:
            bundle = CASES["bundles"][sc["id"]]
            self.assertEqual(bundle["bundleId"], sc["id"])
            self.assertEqual((bundle["editDurationFrames"], bundle["fps"]["numerator"], bundle["fps"]["denominator"]), (sc["edit_duration_frames"], *sc["fps"]))
            self.assertEqual(bundle["renderProfile"]["alphaMode"], "opaque")
            self.assertEqual(len(bundle["fx"]["transitions"]), 1)
            transition = bundle["fx"]["transitions"][0]
            self.assertEqual((transition["alignment"], transition["handles"], transition["durationFrames"]), (sc["transition"]["alignment"], sc["transition"]["handles"], 12))
            instances = {item["id"]: item["durationFrames"] for item in bundle["sceneInstances"]}
            clips = {clip["id"]: sum(seg["durationFrames"] for seg in clip["segments"]) for clip in bundle["cameraClips"]}
            for shot in bundle["shots"]:
                length = shot["editRange"]["endFrameExclusive"] - shot["editRange"]["startFrame"]
                self.assertGreaterEqual(instances[shot["sceneInstanceId"]], shot["instanceStartFrame"] + length)
                self.assertGreaterEqual(clips[shot["cameraClipId"]], shot["cameraInFrame"] + length)
                self.assertGreaterEqual(shot["cameraInFrame"], 0)
            for definition in bundle["sceneDefinitions"]:
                self.assertEqual([node["id"] for node in definition["nodes"]], ["root", "backdrop", "marker"], "colliding ids on purpose")
                self.assertLessEqual(len(definition.get("lighting", {}).get("lights", [])), 4)
            lit = g.A.KITS[sc["kit"]]["lit"]
            self.assertEqual(all("lighting" in definition for definition in bundle["sceneDefinitions"]), lit)
            self.assertEqual(bool(bundle["performanceTracks"]), g.A.KITS[sc["kit"]]["moving"])

    def test_assets_are_valid_pngs_with_the_recorded_hash(self):
        manifest = json.loads((g.FIXTURES / g.MANIFEST_NAME).read_text(encoding="utf-8"))
        files = {item["file"]: item for item in manifest["assets"]}
        self.assertEqual(len(files), 12)
        by_name = {Path(name).stem: item for name, item in files.items()}
        for name, item in files.items():
            data = (g.FIXTURES / name).read_bytes()
            self.assertEqual(hashlib.sha256(data).hexdigest(), item["sha256"])
            self.assertEqual(len(data), item["bytes"])
            self.assertEqual(data[:8], b"\x89PNG\r\n\x1a\n")
            offset, chunks = 8, {}
            while offset < len(data):
                length, tag = struct.unpack(">I4s", data[offset:offset + 8])
                body = data[offset + 8:offset + 8 + length]
                crc = struct.unpack(">I", data[offset + 8 + length:offset + 12 + length])[0]
                self.assertEqual(crc, zlib.crc32(tag + body) & 0xFFFFFFFF)
                chunks[tag] = body
                offset += 12 + length
            self.assertEqual(struct.unpack(">IIBBBBB", chunks[b"IHDR"]), (1, 1, 8, 2, 0, 0, 0))
            raw = zlib.decompress(chunks[b"IDAT"])
            stem = Path(name).stem
            self.assertEqual(raw, bytes([0, int(stem[5:7], 16), int(stem[7:9], 16), int(stem[9:11], 16)]))
        for bundle in CASES["bundles"].values():
            for asset in bundle["assets"]:
                self.assertEqual(asset["contentHash"], by_name[asset["assetId"]]["sha256"])
                self.assertEqual((asset["widthPx"], asset["heightPx"], asset["alphaMode"]), (1, 1, "opaque"))

    def test_the_committed_files_are_what_the_generator_produces(self):
        wanted = g.expected_files()
        self.assertEqual(wanted, g.expected_files(), "deterministic")
        self.assertEqual(g.compare(g.FIXTURES, wanted), [])
        self.assertEqual(wanted[g.CASES_NAME], CASES_BYTES)

    def test_manifest_provenance(self):
        manifest = json.loads((g.FIXTURES / g.MANIFEST_NAME).read_text(encoding="utf-8"))
        self.assertEqual(manifest["corpus"]["sha256"], hashlib.sha256(CASES_BYTES).hexdigest())
        s0 = manifest["source_pins"]["s0_manifest"]
        self.assertEqual(s0["manifest_sha256"], hashlib.sha256((g.FIXTURES / g.S0_MANIFEST_NAME).read_bytes()).hexdigest())
        self.assertEqual(s0["sources"], 33)
        self.assertEqual(s0["tip"], d.TIP)
        blockers = {item["id"] for item in manifest["blockers"]}
        self.assertTrue({"ORIGIN_NOT_EXECUTED", "ORIGIN_GOLDEN_NOT_COMPARABLE", "ORACLE_NOT_S2_VALIDATED", "RUST_PROVES_IMPORTABILITY_ONLY"} <= blockers)
        self.assertIn("pin_status", manifest["source_pins"])
        self.assertEqual(manifest["corpus"]["counts"]["scenarios"], 7)
        self.assertTrue(all(row["present"] for row in CASES["destination_reads"]))

    def test_bytes_are_utf8_lf_and_free_of_host_paths(self):
        paths = [g.FIXTURES / g.CASES_NAME, g.FIXTURES / g.MANIFEST_NAME, GENERATOR_PATH, Path(__file__)]
        for path in paths:
            data = path.read_bytes()
            self.assertNotIn(b"\r", data, path.name)
            self.assertFalse(data.startswith(b"\xef\xbb\xbf"), path.name)
            text = data.decode("utf-8")
            if path == Path(__file__):
                continue  # the spec names a Store stub path and the forbidden words on purpose
            for forbidden in ("C:\\Users", "/Users/", "wally", "E:\\www"):
                self.assertNotIn(forbidden, text, (path.name, forbidden))
            if path.suffix == ".json":
                self.assertNotIn("origin-output", text, path.name)
            self.assertTrue(text.endswith("\n"))

    def test_no_bare_assert_and_stdlib_only_imports(self):
        for path in (GENERATOR_PATH, Path(__file__)):
            tree = ast.parse(path.read_text(encoding="utf-8"))
            self.assertEqual([node for node in ast.walk(tree) if isinstance(node, ast.Assert)], [], path.name)
        tree = ast.parse(GENERATOR_PATH.read_text(encoding="utf-8"))
        imported = set()
        for node in ast.walk(tree):
            if isinstance(node, ast.Import):
                imported |= {alias.name.split(".")[0] for alias in node.names}
            elif isinstance(node, ast.ImportFrom):
                imported.add((node.module or "").split(".")[0])
        self.assertTrue(imported <= set(sys.stdlib_module_names), imported - set(sys.stdlib_module_names))

    def test_the_generator_refuses_a_store_stub_and_has_a_cli(self):
        stub = r"C:\Users\x\AppData\Local\Microsoft\WindowsApps\python.exe"
        self.assertIn("Store stub", d.interpreter_problem(stub, (3, 13), 0))
        self.assertIsNone(d.interpreter_problem(sys.executable))
        self.assertEqual(g.main(["generate", "--check"]), g.EXIT_OK)
        self.assertEqual(g.main(["generate"]), g.EXIT_ORIGIN_UNAVAILABLE, "generate without --check needs the origin")
        self.assertEqual(g.main(["verify", "--require-pin"]), g.EXIT_PIN_UNVERIFIED, "no origin, no pin claim")
        self.assertEqual(g.main(["verify"]), g.EXIT_OK)

    def test_the_hypothesis_matrix_text_lists_every_scenario(self):
        text = g.matrix_text(CASES)
        for scenario_id in SCENARIOS:
            self.assertIn(scenario_id, text)


if __name__ == "__main__":
    unittest.main(verbosity=1)
