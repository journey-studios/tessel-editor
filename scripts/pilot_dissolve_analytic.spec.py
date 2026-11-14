"""Tests of the analytic reference X (P4b-S0, review finding P2-2): `scripts/pilot_dissolve_analytic.py` and the
`analytic_probes` section of `docs/migration/fixtures/pilot-fx/dissolve-cases.json`. No editor, GPU, runtime page,
engine, Cargo, Node or origin execution is needed (the origin blobs are never read here).

Five layers, none of which compares the module only with itself:

1. HAND VECTORS written here as literals (the origin's own flicker numbers as the repository's Rust test records them,
   projections worked at 50 digits, clocks counted on fingers, a lit colour worked with Decimal).
2. An INDEPENDENT REFERENCE written in this file from the same origin text but with other means: `ctypes` for the 32-bit
   integer mix, a perspective MATRIX (Three.js `makePerspective`) instead of a ray basis, a closed form for a camera that
   only trucks, `fractions.Fraction` for the weights and `decimal.Decimal` for the sRGB curves. It recomputes EVERY probe,
   state, light and marker of the corpus section, and the hypotheses each probe rejects.
3. A CONSUMER GATE (1/255 per channel and |mean signed delta| <= 0.25, the corpus' own tolerances) run against fake
   products that implement each wrong rule: it must FAIL for the hypotheses a scenario must reject, PASS for the ones that
   must agree and PASS for the truth.
4. MUTANTS OF THE ORACLE ITSELF (patching the module's kernels): each must change the corpus section (or refuse to build),
   and the formula mutants must also fail the hand vectors of layer 1.
5. STRUCTURE: coverage of the cases the plan asks for, determinism, independence of the imports, UTF-8/LF, no bare `assert`.

The spec REFUSES a Microsoft Store stub as the interpreter (a refusal is never a skip), prints `sys.version`, and is written
with `unittest` assertions, never a bare `assert`, so it still tests under `python -O`.
"""
import ast
import importlib.util
import json
import math
import sys
import unittest
from ctypes import c_int32, c_uint32
from decimal import ROUND_FLOOR, Decimal, localcontext
from fractions import Fraction
from functools import lru_cache
from pathlib import Path
from unittest.mock import patch

HERE = Path(__file__).parent


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, HERE / filename)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


m = load("pilot_dissolve_analytic", "pilot_dissolve_analytic.py")
d = load("pilot_dissolve_corpus", "pilot-dissolve-corpus.py")

_problem = d.interpreter_problem()
if _problem:
    sys.stderr.write("INTERPRETER_NOT_REAL: %s\n" % _problem)
    raise SystemExit(d.EXIT_BAD_INTERPRETER)
sys.stderr.write("python: %s\n" % sys.version.replace("\n", " "))

CORPUS_BYTES = (d.CORPUS / d.CASES_NAME).read_bytes()
CORPUS = json.loads(CORPUS_BYTES.decode("utf-8"))
SECTION = CORPUS["analytic_probes"]
SCENARIOS = {item["id"]: item for item in SECTION["scenarios"]}
SHORT = {name: item["short"] for name, item in SECTION["hypotheses"].items()}
BY_SHORT = {short: name for name, short in SHORT.items()}
W, H = 640, 360
FOV_HALF_TAN = math.tan(math.radians(50.0) / 2.0)
ASPECT = W / H


# ---- layer 2: the independent reference ----------------------------------------------------------------------


def r_imul(a, b):
    return c_int32(c_int32(a).value * c_int32(b).value).value


def r_hash01(n, salt):
    h = r_imul(n, 0x9E3779B1) ^ r_imul(salt, 0x85EBCA6B)
    h = c_int32(h ^ (c_uint32(h).value >> 15)).value
    h = r_imul(h, 0x2C1B3C6D)
    h = c_int32(h ^ (c_uint32(h).value >> 12)).value
    h = r_imul(h, 0x297A2D39)
    h = c_int32(h ^ (c_uint32(h).value >> 15)).value
    return c_uint32(h).value / 4294967296


def r_seed(light_id):
    seed = 7
    for character in light_id:
        seed = r_imul(seed, 31) + ord(character)  # the sum is not wrapped: the next imul or `| 0` does it
    return seed


def r_noise(x, seed):
    i = math.floor(x)
    frac = x - i
    smooth = frac * frac * (3 - 2 * frac)
    low = r_hash01(i, seed)
    return low + (r_hash01(i + 1, seed) - low) * smooth


def r_flicker(light_id, flicker, seconds):
    if flicker <= 0:
        return 1.0
    seed = r_seed(light_id)
    return 1 - flicker * 0.45 * (0.6 * r_noise(seconds * 3, seed) + 0.4 * r_noise(seconds * 13, seed + 101))


def r_seconds(sc, frame):
    num, den = sc["fps"]
    return float(Fraction(frame * den, num))


def r_clamp(wanted, frames):
    return 0 if frames <= 0 else max(0, min(frames - 1, wanted))


def r_clocks(sc, key, f, rule, primary):
    """One deck's state at the REAL edit frame f; `rule` is None (the truth) or the short id of a hypothesis."""
    shot = sc["shots"][key]
    instance_n = sc["instance_frames"][shot["instance"]]
    camera_n = sc["camera_clip_frames"][shot["camera"]]
    basis = sc["shots"][primary] if (rule == "PRI" and key != primary) else shot
    delta = f - basis["start"]
    want_i, want_c = basis["instance_start"] + delta, basis["camera_in"] + delta
    if rule == "SWP":
        instance, camera = r_clamp(basis["camera_in"] + delta, camera_n), r_clamp(basis["instance_start"] + delta, instance_n)
    elif rule == "PIN":
        low = max(-basis["instance_start"], -basis["camera_in"])
        high = min(instance_n - 1 - basis["instance_start"], camera_n - 1 - basis["camera_in"])
        joint = min(max(delta, low), high)
        instance, camera = r_clamp(basis["instance_start"] + joint, instance_n), r_clamp(basis["camera_in"] + joint, camera_n)
    else:
        instance, camera = r_clamp(want_i, instance_n), r_clamp(want_c, camera_n)
    seconds = r_seconds(sc, f)
    if rule == "LIN":
        seconds = r_seconds(sc, instance)
    elif rule == "LSR":
        seconds = r_seconds(sc, f - shot["start"])
    elif rule == "LCC":
        seconds = r_seconds(sc, camera)
    elif rule == "LCE":
        seconds = r_seconds(sc, max(shot["start"], min(shot["end"] - 1, f)))
    elif rule == "LNF":
        seconds = f / round(sc["fps"][0] / sc["fps"][1])
    return {"instance_frame": instance, "camera_clip_frame": camera, "held_instance": instance != want_i, "held_camera": camera != want_c,
            "scene_seconds": seconds, "delta": delta}


def r_primary(sc, f):
    return "A" if f < sc["cut"] else "B"


def r_states(sc, f, rule):
    primary = r_primary(sc, f)
    return {key: r_clocks(sc, key, f, rule, primary) for key in ("A", "B")}


def r_camera_x(sc, key, state):
    clip = SECTION["kits"][sc["kit"]]["decks"][key]["camera_clip_of_layout_mixed"]
    segment = clip["segments"][0]
    frames = sc["camera_clip_frames"][sc["shots"][key]["camera"]]
    speed = segment["distance"] / (segment["frames"] - 1) if segment["recipe"] == "truckRight" else 0.0
    return clip["position"][0] + speed * min(state["camera_clip_frame"], frames - 1)


def r_plane_centre(plane, state):
    x, y, z = plane["position"]
    event = plane.get("event")
    if event is not None:
        p = min(1.0, max(0.0, (state["instance_frame"] - event["start"]) / event["duration"]))
        x, y, z = x + event["delta"][0] * p, y + event["delta"][1] * p, z + event["delta"][2] * p
    return x, y, z


def r_project(point, cam):
    """Three.js: view = translation (the camera looks down -z), projection = makePerspective(vertical fov, aspect)."""
    vx, vy, vz = point[0] - cam[0], point[1] - cam[1], point[2] - cam[2]
    f = 1.0 / FOV_HALF_TAN
    clip_x, clip_y, clip_w = (f / ASPECT) * vx, f * vy, -vz
    return ((clip_x / clip_w + 1) / 2 * W, (1 - clip_y / clip_w) / 2 * H)


def r_cover(sc, key, state, x, y):
    """The plane that covers the CENTRE of pixel (x, y): the last of the painter's queue whose projected rectangle holds it."""
    deck = SECTION["kits"][sc["kit"]]["decks"][key]
    cam = (r_camera_x(sc, key, state), 0.0, 6.0)
    centre_x, centre_y = x + 0.5, y + 0.5
    chosen = None
    for plane in sorted(deck["planes"], key=lambda item: item["render_order"]):
        position = r_plane_centre(plane, state)
        half_w, half_h = plane["width"] / 2, plane["height"] / 2
        left, top = r_project((position[0] - half_w, position[1] + half_h, position[2]), cam)
        right, bottom = r_project((position[0] + half_w, position[1] - half_h, position[2]), cam)
        if left <= centre_x <= right and top <= centre_y <= bottom:
            chosen = (plane, position, cam)
    return chosen


def r_light_amount(light, hit, cam):
    to_light = [light["position"][i] - hit[i] for i in range(3)]
    distance = math.sqrt(sum(c * c for c in to_light))
    facing = max(0.0, (to_light[2] * (1.0 if cam[2] >= hit[2] else -1.0)) / distance)  # n = (0, 0, +1) facing the camera
    falloff = 1.0 / (1.0 + (distance / light["radius"]) ** 2)
    return facing * falloff * falloff


class FloatBackend:
    """Colour maths in floats (fast: the hypotheses)."""

    @staticmethod
    def decode(code):
        v = code / 255.0
        return v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4

    @staticmethod
    def encode255(linear):
        x = min(max(linear, 0.0), 1.0)
        return (x * 12.92 if x <= 0.0031308 else 1.055 * x ** (1 / 2.4) - 0.055) * 255

    @staticmethod
    def number(value):
        return value

    @staticmethod
    def code(value):
        return int(min(255, max(0, math.floor(value + 0.5)))), abs(value - math.floor(value) - 0.5)


class DecimalBackend:
    """Colour maths in 60-digit decimals (the truth)."""

    @staticmethod
    @lru_cache(maxsize=None)
    def _decode(code):
        with localcontext() as ctx:
            ctx.prec = 60
            v = Decimal(code) / Decimal(255)
            return v / Decimal("12.92") if v <= Decimal("0.04045") else ((v + Decimal("0.055")) / Decimal("1.055")) ** Decimal("2.4")

    decode = staticmethod(lambda code: DecimalBackend._decode(code))

    @staticmethod
    def encode255(linear):
        with localcontext() as ctx:
            ctx.prec = 60
            x = min(max(linear, Decimal(0)), Decimal(1))
            if x <= Decimal("0.0031308"):
                return x * Decimal("12.92") * 255
            return (Decimal("1.055") * x ** (Decimal(5) / Decimal(12)) - Decimal("0.055")) * 255

    @staticmethod
    def number(value):
        return Decimal(repr(float(value))) if not isinstance(value, Decimal) else value

    @staticmethod
    def code(value):
        floor = value.to_integral_value(rounding=ROUND_FLOOR)
        return int(min(255, max(0, (value + Decimal("0.5")).to_integral_value(rounding=ROUND_FLOOR)))), abs(value - floor - Decimal("0.5"))


def r_deck_code(sc, key, state, x, y, backend):
    """(plane id, 8-bit code, distance of the closest channel to a rounding tie) of one deck at pixel (x, y)."""
    chosen = r_cover(sc, key, state, x, y)
    if chosen is None:
        return None, [0, 0, 0], 1.0
    plane, position, cam = chosen
    base = plane["colour8"]
    deck = SECTION["kits"][sc["kit"]]["decks"][key]
    if not deck["lights"]:
        return plane["id"], list(base), 1.0
    depth = cam[2] - position[2]
    hit = (cam[0] + ((x + 0.5) / W * 2 - 1) * ASPECT * FOV_HALF_TAN * depth, cam[1] + (1 - (y + 0.5) / H * 2) * FOV_HALF_TAN * depth, position[2])
    total = [0.0, 0.0, 0.0]
    for light in deck["lights"]:
        level = light["intensity"] * r_flicker(light["id"], light["flicker"], state["scene_seconds"])
        amount = r_light_amount(light, hit, cam) * level
        for channel in range(3):
            total[channel] += light["colour"][channel] * amount
    codes, margin = [], 1.0
    for channel in range(3):
        value = backend.encode255(backend.decode(base[channel]) * (1 + backend.number(total[channel])))
        code, distance = backend.code(value)
        codes.append(code)
        margin = min(margin, float(distance))
    return plane["id"], codes, margin


def r_weight(sc, k):
    transition = sc["transition"]
    t = Fraction(2 * k + 1, 2 * transition["duration"])
    if transition["easing"] == "linear":
        progress = t
    elif transition["easing"] == "easeInOutCubic":
        progress = 4 * t ** 3 if t < Fraction(1, 2) else 1 - (-2 * t + 2) ** 3 / 2
    else:
        progress = 1 - (1 - t) ** 3
    shaped = progress * progress * (3 - 2 * progress) if transition["curve"] == "smooth" else progress
    amount = Fraction(transition["intensity"])
    return progress * (1 - amount) + shaped * amount


def r_mix(a8, b8, weight, backend):
    """out = a * (1 - w) + b * w in linear light, the weight an exact rational (the corpus' module mixes twice and adds)."""
    codes, margin = [], 1.0
    w = backend.number(float(weight)) if backend is FloatBackend else Decimal(weight.numerator) / Decimal(weight.denominator)
    for a, b in zip(a8, b8):
        value = backend.encode255(backend.decode(a) * (1 - w) + backend.decode(b) * w)
        code, distance = backend.code(value)
        codes.append(code)
        margin = min(margin, float(distance))
    return codes, margin


def r_out(sc, f, x, y, rule, backend):
    """The final code at pixel (x, y) of window frame f under `rule` (None = the truth). Returns (code, plane A, plane B, a8, b8, tie margin)."""
    states = r_states(sc, f, rule)
    cover_a, a8, margin_a = r_deck_code(sc, "A", states["A"], x, y, backend)
    cover_b, b8, margin_b = r_deck_code(sc, "B", states["B"], x, y, backend)
    start = sc["transition"]["window"]["start"]
    weight = r_weight(sc, f - start)
    if rule == "DSW":
        out, margin_m = r_mix(b8, a8, weight, backend)
    else:
        out, margin_m = r_mix(a8, b8, weight, backend)
    return out, cover_a, cover_b, a8, b8, min(margin_a, margin_b, margin_m)


def hex8(code):
    return "#%02x%02x%02x" % tuple(code)


def parse_pixel(text):
    x, y = text.split(",")
    return int(x), int(y)


def parse_deck(text):
    plane, colour = text.split(" ")
    return (None if plane == "None" else plane), colour


# ---- layer 3: the consumer gate ------------------------------------------------------------------------------


def gate(scenario, produce):
    """The corpus' tolerance applied to a product: `produce(frame_row, probe)` gives its 8-bit code. Returns the problems."""
    deltas = [[], [], []]
    problems = []
    for frame in scenario["frames"]:
        for probe in frame.get("probes", []):
            expected = [int(probe["expected"][1 + 2 * i:3 + 2 * i], 16) for i in range(3)]
            got = produce(frame, probe)
            for channel in range(3):
                deltas[channel].append(got[channel] - expected[channel])
    for channel in range(3):
        if deltas[channel] and max(abs(item) for item in deltas[channel]) > 1:
            problems.append("channel %d: a probe is off by more than 1 code (%d)" % (channel, max(abs(item) for item in deltas[channel])))
        if len(deltas[channel]) >= 24 and abs(sum(deltas[channel]) / len(deltas[channel])) > 0.25:
            problems.append("channel %d: the mean signed delta is %.3f" % (channel, sum(deltas[channel]) / len(deltas[channel])))
    return problems


def product_with(scenario, rule):
    """A fake product that follows the truth except for `rule` (a wrong rule): float colour maths, the module not involved."""
    def produce(frame, probe):
        x, y = parse_pixel(probe["pixel"])
        return r_out(scenario, frame["edit_frame"], x, y, rule, FloatBackend)[0]
    return produce


# ---- layer 1: hand vectors -----------------------------------------------------------------------------------

# Values of the origin's `flickerLevel` (scene-lighting.ts), as the repository's Rust test records them ("computed by running
# its code (`Math.imul` mixing) in Node"): crates/core/src/directed_scene/lighting.rs. The spec checks that the literals are
# still in that file, so the vectors cannot drift away from it.
HAND_FLICKER = [("lamp-1", 1.0, 0.0, 0.6885484384116716, "0.688_548_438_411_671_6"), ("lamp-1", 1.0, 1.25, 0.8343367595735618, "0.834_336_759_573_561_8"),
                ("torch", 0.5, 2.5, 0.9335570273292251, "0.933_557_027_329_225_1"),
                ("a-very-long-light-identifier-that-overflows-the-seed", 1.0, 7.3, 0.8122692488485295, "0.812_269_248_848_529_5")]
# camera (-2, 0, 6) looking at (-2, 0, 0), vertical fov 50, 640 x 360, world point -> continuous pixel; tan 25 deg = 0.4663076581549986,
# worked at 50 digits (Decimal series, independent of the module): u = ((x + 2) / (6 * tan * 16 / 9) + 1) / 2 * 640, v = (1 - y / (6 * tan)) / 2 * 360
HAND_PROJECTION = [((0.0, 0.0, 0.0), 448.670415230574, 180.0), ((0.0, 1.0, 0.0), 448.670415230574, 115.664792384713), ((2.0, -1.0, 0.0), 577.340830461147, 244.335207615287)]
# a lit colour: base 100, lamp one scene unit... L = 1 exactly (a lamp straight ahead at distance 2 with radius 1e9 and intensity 1, white): 100 -> 138; 50 with L = 3 -> 100
HAND_LIT = [(100, 1.0, 138), (50, 3.0, 100), (200, 0.5, 239)]


def hand_problems(mod):
    """What the module gets wrong against the literals of this file (empty when it is right)."""
    problems = []
    for light_id, flicker, seconds, expected, _ in HAND_FLICKER:
        got = mod.flicker_level(light_id, flicker, seconds)
        if abs(got - expected) > 1e-13:
            problems.append("flicker %s %s %s: %r vs %r" % (light_id, flicker, seconds, got, expected))
    if mod.flicker_level("lamp-1", 0.0, 3.7) != 1.0:
        problems.append("flicker 0 is not exactly 1")
    pose = ((-2.0, 0.0, 6.0), (-2.0, 0.0, 0.0))
    for point, u, v in HAND_PROJECTION:
        got = mod.project(point, pose)
        if abs(got[0] - u) > 1e-6 or abs(got[1] - v) > 1e-6:
            problems.append("projection %r: %r vs (%s, %s)" % (point, got, u, v))
    x, y = mod.project((0.0, 0.0, 0.0), pose)
    if int(math.floor(x)) != 448:
        problems.append("the marker centre pixel is not 448")
    clip = {"position": [-2.0, 0.0, 6.0], "target": [-2.0, 0.0, 0.0],
            "segments": [{"recipe": "truckRight", "frames": 40, "from": "initial", "distance": 0.36 * 39, "easing": "linear"}]}
    for frame, x_expected in ((0, -2.0), (12, 2.32), (39, 12.04), (55, 12.04)):
        got = mod.camera_pose(clip, frame)[0][0]
        if abs(got - x_expected) > 1e-9:
            problems.append("truck x at clip frame %d: %r vs %r" % (frame, got, x_expected))
    hold = {"position": [0.0, 0.0, 6.0], "target": [0.0, 0.0, 0.0], "segments": [{"recipe": "hold", "frames": 12, "from": "initial"}]}
    if mod.camera_pose(hold, 30)[0] != (0.0, 0.0, 6.0):
        problems.append("a hold moved")
    for frame, p in ((5, 0.0), (30, 0.0), (36, 0.5), (42, 1.0), (50, 1.0)):
        if abs(mod.event_progress(frame, 30, 12) - p) > 1e-12:
            problems.append("event progress at %d" % frame)
    for base, light_sum, code in HAND_LIT:
        deck = {"id": "T", "planes": [{"id": "marker", "colour8": [base, base, base], "width": 2.0, "height": 2.0, "position": [0.0, 0.0, 0.0], "render_order": 0}],
                "lights": [{"id": "t", "position": [0.0, 0.0, 2.0], "colour": [1.0, 1.0, 1.0], "intensity": light_sum, "radius": 1e9, "flicker": 0.0}],
                "camera": {"position": [0.0, 0.0, 6.0], "target": [0.0, 0.0, 0.0], "segments": [{"recipe": "hold", "frames": 1, "from": "initial"}]}}
        state = {"instance_frame": 0, "camera_clip_frame": 0, "scene_seconds": 0.0}
        try:
            got = mod.deck_pixel(deck, state, 320, 180)[2]
        except mod.Refusal as refusal:
            got = refusal.code
        if got != [code, code, code]:
            problems.append("lit colour of %d with L = %s: %r vs %d" % (base, light_sum, got, code))
    unlit = {"id": "T", "planes": [{"id": "marker", "colour8": [7, 8, 9], "width": 2.0, "height": 2.0, "position": [0.0, 0.0, 0.0], "render_order": 0}], "lights": [],
             "camera": hold}
    if mod.deck_pixel(unlit, {"instance_frame": 0, "camera_clip_frame": 0, "scene_seconds": 0.0}, 320, 180)[2] != [7, 8, 9]:
        problems.append("an unlit plane is not its own 8-bit colour")
    if abs(mod.light_amount_at({"position": [0.0, 0.0, 2.0], "radius": 6.0}, (0.0, 0.0, 0.0), (0.0, 0.0, 1.0), (0.0, 0.0, 6.0)) - 0.81) > 1e-12:
        problems.append("light amount straight ahead is not 0.81")
    amount = mod.light_amount_at({"position": [0.0, 0.0, 2.0], "radius": 6.0}, (3.0, 0.0, 0.0), (0.0, 0.0, 1.0), (0.0, 0.0, 6.0))
    if abs(amount - (36 / 49) ** 2 * 2 / math.sqrt(13)) > 1e-12:
        problems.append("light amount off axis")
    edit = {"fps": [30000, 1001], "shots": {"A": {"start": 0, "end": 1800, "instance_start": 0, "camera_in": 0, "instance": "i", "camera": "c"}},
            "instances": {"i": 1800}, "cameras": {"c": 1800}}
    if abs(mod.clocks_of(edit, "A", 1794)["scene_seconds"] - 59.8598) > 1e-12:
        problems.append("1794 / (30000 / 1001) is not 59.8598 s")
    return problems


# clocks counted by hand: (layout, shot, edit frame) -> (instance frame, camera frame, held instance, held camera)
HAND_CLOCKS = [
    ("mixed", "B", 24, (0, 6, True, False)), ("mixed", "B", 29, (0, 11, True, False)), ("mixed", "B", 30, (0, 12, False, False)),
    ("mixed", "A", 29, (33, 29, False, False)), ("mixed", "A", 30, (34, 29, False, True)), ("mixed", "A", 35, (39, 29, False, True)),
    ("edge", "A", 31, (35, 31, False, False)), ("edge", "A", 32, (36, 31, False, True)), ("edge", "A", 33, (36, 31, True, True)),
    ("real", "B", 26, (6, 8, False, False)), ("held", "B", 24, (0, 0, True, True)), ("held", "A", 35, (29, 29, True, True)),
    ("held-light", "A", 35, (11, 11, True, True)),
]


class HandVectorTests(unittest.TestCase):
    def test_the_module_matches_every_hand_vector(self):
        self.assertEqual(hand_problems(m), [])

    def test_the_flicker_literals_are_still_the_ones_the_rust_test_records(self):
        text = (HERE.parent / "crates/core/src/directed_scene/lighting.rs").read_text(encoding="utf-8")
        for _, _, _, _, rust in HAND_FLICKER:
            self.assertIn(rust, text)

    def test_the_clocks_counted_by_hand(self):
        for layout, shot, frame, want in HAND_CLOCKS:
            edit = m.build_edit(layout)
            got = m.clocks_of(edit, shot, frame)
            self.assertEqual((got["instance_frame"], got["camera_clip_frame"], got["held_instance"], got["held_camera"]), want, (layout, shot, frame))

    def test_the_window_and_progress_of_the_defaults_by_hand(self):
        self.assertEqual(m.window_of(30, 12, "center"), (24, 36))
        self.assertEqual(m.window_of(30, 12, "start"), (30, 42))
        self.assertEqual(m.window_of(30, 12, "end"), (18, 30))
        progress, shaped, amount, weight = m.weight_for(5, dict(m.TRANSITION))
        self.assertAlmostEqual(progress, 0.385127, places=6)  # plan 6.2
        self.assertAlmostEqual(weight, 0.330723, places=6)
        self.assertEqual(amount, 1.0)
        self.assertEqual(m.mix_codes([200, 0, 0], [40, 0, 0], dict(m.TRANSITION), 5)[0], 168)  # plan 6.2: 200 -> 40 at k = 5

    def test_the_seed_of_a_light_id_equals_the_reference(self):
        self.assertEqual(m.seed_of("lamp-1"), r_seed("lamp-1"))
        self.assertEqual(m.seed_of("a-very-long-light-identifier-that-overflows-the-seed"), r_seed("a-very-long-light-identifier-that-overflows-the-seed"))

    def test_the_integer_mix_agrees_with_ctypes_on_a_grid(self):
        for n in (-3, -1, 0, 1, 2, 7, 90, 2 ** 31 - 1, -(2 ** 31)):
            for salt in (0, 1, 101, 123456789, -77, 2 ** 40 + 5):
                self.assertEqual(m.hash01(n, salt), r_hash01(n, salt), (n, salt))
        for seconds in (-1.5, 0.0, 0.3, 1.0, 2.7, 59.8598 * 13):
            self.assertAlmostEqual(m.value_noise(seconds, 12345), r_noise(seconds, 12345), places=13)


# ---- layer 2 tests: the reference recomputes the whole section ---------------------------------------------------------


class ReferenceTests(unittest.TestCase):
    def test_every_probe_equals_the_independent_decimal_reference(self):
        checked = 0
        for scenario in SECTION["scenarios"]:
            for frame in scenario["frames"]:
                for probe in frame.get("probes", []):
                    x, y = parse_pixel(probe["pixel"])
                    out, cover_a, cover_b, a8, b8, margin = r_out(scenario, frame["edit_frame"], x, y, None, DecimalBackend)
                    where = (scenario["id"], frame["edit_frame"], probe["id"])
                    self.assertGreater(margin, 1e-6, where)
                    self.assertEqual(probe["expected"], hex8(out), where)
                    self.assertEqual(probe["deck_a"], "%s %s" % (cover_a, hex8(a8)), where)
                    self.assertEqual(probe["deck_b"], "%s %s" % (cover_b, hex8(b8)), where)
                    checked += 1
        self.assertGreaterEqual(checked, 350)

    def test_every_probe_lies_well_inside_a_flat_area(self):
        for scenario in SECTION["scenarios"]:
            for frame in scenario["frames"]:
                states = r_states(scenario, frame["edit_frame"], None)
                for probe in frame.get("probes", []):
                    x, y = parse_pixel(probe["pixel"])
                    for key in ("A", "B"):
                        seen = set()
                        for dx in (-6, 0, 6):
                            for dy in (-6, 0, 6):
                                self.assertTrue(0 <= x + dx < W and 0 <= y + dy < H, (scenario["id"], probe["id"]))
                                chosen = r_cover(scenario, key, states[key], x + dx, y + dy)
                                seen.add(None if chosen is None else chosen[0]["id"])
                        self.assertEqual(len(seen), 1, (scenario["id"], frame["edit_frame"], probe["id"], key, seen))

    def test_every_state_light_and_marker_equals_the_reference(self):
        for scenario in SECTION["scenarios"]:
            lit = SECTION["kits"][scenario["kit"]]["decks"]["A"]["lights"]
            for frame in scenario["frames"]:
                states = r_states(scenario, frame["edit_frame"], None)
                where = (scenario["id"], frame["edit_frame"])
                self.assertEqual(frame["primary"], r_primary(scenario, frame["edit_frame"]), where)
                for key in ("A", "B"):
                    row, ref = frame["states"][key], states[key]
                    for field in ("instance_frame", "camera_clip_frame", "held_instance", "held_camera", "delta"):
                        self.assertEqual(row[field], ref[field], where + (key, field))
                    self.assertAlmostEqual(row["scene_seconds"], ref["scene_seconds"], places=9, msg=where + (key,))
                    if lit:
                        deck = SECTION["kits"][scenario["kit"]]["decks"][key]
                        for light, got in zip(deck["lights"], frame["lights"][key]):
                            level = r_flicker(light["id"], light["flicker"], ref["scene_seconds"])
                            self.assertAlmostEqual(got["flicker_level"], level, places=9)
                            self.assertAlmostEqual(got["effective_intensity"], light["intensity"] * level, places=7)
                    if "markers" in frame:
                        plane = next(item for item in SECTION["kits"][scenario["kit"]]["decks"][key]["planes"] if item["id"] == "marker")
                        centre = r_plane_centre(plane, ref)
                        cam = (r_camera_x(scenario, key, ref), 0.0, 6.0)
                        u, v = r_project(centre, cam)
                        got_u, got_v = [float(item) for item in frame["markers"][key]["centre_px"].split(",")]
                        self.assertAlmostEqual(got_u, u, places=6, msg=where + (key,))
                        self.assertAlmostEqual(got_v, v, places=6, msg=where + (key,))
                        half = plane["width"] / 2
                        left, top = r_project((centre[0] - half, centre[1] + half, centre[2]), cam)
                        right, bottom = r_project((centre[0] + half, centre[1] - half, centre[2]), cam)
                        got_rect = [float(item) for item in frame["markers"][key]["rect_px"].split(",")]
                        for got, want in zip(got_rect, (left, top, right, bottom)):
                            self.assertAlmostEqual(got, want, places=6, msg=where + (key,))

    def test_the_hypotheses_each_probe_rejects_are_exactly_the_ones_the_reference_separates(self):
        for scenario in SECTION["scenarios"]:
            names = scenario["must_reject"] + scenario["must_agree"]
            for frame in scenario["frames"]:
                for probe in frame.get("probes", []):
                    x, y = parse_pixel(probe["pixel"])
                    truth = r_out(scenario, frame["edit_frame"], x, y, None, FloatBackend)[0]
                    separated = set()
                    for name in names:
                        if name in scenario["must_agree"]:
                            continue
                        wrong = r_out(scenario, frame["edit_frame"], x, y, SHORT[name], FloatBackend)[0]
                        if max(abs(wrong[i] - truth[i]) for i in range(3)) >= 4:
                            separated.add(SHORT[name])
                    recorded = set(probe.get("rejects", "").split(",")) - {""}
                    self.assertEqual(recorded, separated, (scenario["id"], frame["edit_frame"], probe["id"]))
                    for name in scenario["must_agree"]:
                        wrong = r_out(scenario, frame["edit_frame"], x, y, SHORT[name], FloatBackend)[0]
                        self.assertLessEqual(max(abs(wrong[i] - truth[i]) for i in range(3)), 1, (scenario["id"], frame["edit_frame"], probe["id"], name))

    def test_the_window_progress_and_weight_of_every_frame_equal_the_exact_rational(self):
        for scenario in SECTION["scenarios"]:
            window = scenario["transition"]["window"]
            for frame in scenario["frames"]:
                inside = window["start"] <= frame["edit_frame"] < window["end_exclusive"]
                self.assertEqual(frame["in_window"], inside)
                if inside:
                    k = frame["edit_frame"] - window["start"]
                    self.assertEqual(frame["k"], k)
                    self.assertAlmostEqual(frame["transition"]["weight"], float(r_weight(scenario, k)), places=9)
                else:
                    self.assertIsNone(frame["transition"])
                    self.assertNotIn("probes", frame)


# ---- layer 3 tests: the consumer gate ------------------------------------------------------------------------------------


class GateTests(unittest.TestCase):
    def test_the_truth_passes_the_gate_in_every_scenario(self):
        for scenario in SECTION["scenarios"]:
            self.assertEqual(gate(scenario, product_with(scenario, None)), [], scenario["id"])

    def test_each_wrong_rule_a_scenario_must_reject_fails_the_gate(self):
        for scenario in SECTION["scenarios"]:
            for name in scenario["must_reject"]:
                problems = gate(scenario, product_with(scenario, SHORT[name]))
                self.assertTrue(problems, "%s: a product implementing %s passes the gate" % (scenario["id"], name))

    def test_the_wrong_rules_that_must_agree_still_pass_the_gate(self):
        saw_a_control = False
        for scenario in SECTION["scenarios"]:
            for name in scenario["must_agree"]:
                saw_a_control = True
                self.assertEqual(gate(scenario, product_with(scenario, SHORT[name])), [], "%s: %s is a control" % (scenario["id"], name))
        self.assertTrue(saw_a_control)

    def test_the_gate_catches_a_truncating_product_by_its_bias_in_every_scenario(self):
        for scenario in SECTION["scenarios"]:
            def truncating(frame, probe, scenario=scenario):
                x, y = parse_pixel(probe["pixel"])
                states = r_states(scenario, frame["edit_frame"], None)
                a8 = r_deck_code(scenario, "A", states["A"], x, y, DecimalBackend)[1]
                b8 = r_deck_code(scenario, "B", states["B"], x, y, DecimalBackend)[1]
                weight = r_weight(scenario, frame["k"])
                w = Decimal(weight.numerator) / Decimal(weight.denominator)
                floors = []
                for a, b in zip(a8, b8):
                    value = DecimalBackend.encode255(DecimalBackend.decode(a) * (1 - w) + DecimalBackend.decode(b) * w)
                    floors.append(int(value.to_integral_value(rounding=ROUND_FLOOR)))
                return floors

            # within 1 code per probe (a max-delta-1 gate passes it): only the bias limit refuses it
            problems = gate(scenario, truncating)
            self.assertTrue(problems, scenario["id"])
            self.assertTrue(all("mean signed delta" in item for item in problems), (scenario["id"], problems))


# ---- layer 4: mutants of the oracle itself -----------------------------------------------------------------------------------


def section_without_destination_reads(section):
    clone = json.loads(json.dumps(section))
    clone.pop("destination_reads", None)
    return clone


EXPECTED_SECTION = section_without_destination_reads(SECTION)


def detected(mutation):
    """True when the oracle under `mutation` refuses to build or produces a section that is not the corpus'."""
    try:
        built = m.build_section()
    except m.Refusal:
        return True
    return json.loads(json.dumps(built)) != EXPECTED_SECTION


ORIGINAL = {name: getattr(m, name) for name in ("clocks_of", "mix_codes", "weight_for", "flicker_level", "ushr", "camera_pose", "event_progress",
                                                "pixel_ray", "deck_pixel", "decode", "encode", "edit_seconds", "light_amount_at", "project")}


def clocks_with(name):
    clock = m.HYPOTHESES[name]["clock"]

    def wrapper(edit, shot_id, f, hypothesis=None, primary_id=None):
        return ORIGINAL["clocks_of"](edit, shot_id, f, clock, primary_id)
    return wrapper


def reversed_painter(deck, state, x, y):
    flipped = dict(deck, planes=[dict(plane, render_order=-plane["render_order"]) for plane in deck["planes"]])
    return ORIGINAL["deck_pixel"](flipped, state, x, y)


def event_end_inclusive(frame, start, duration):
    if frame < start:
        return 0.0
    return min(1.0, (frame - start) / (duration - 1))


MUTANTS = [
    # the wrong rules of the hypotheses, put INTO the oracle as if they were its own truth
    ("both clocks pinned together", {"clocks_of": clocks_with("BOTH_CLOCKS_PINNED_TOGETHER")}, False),
    ("instance and camera clocks swapped", {"clocks_of": clocks_with("INSTANCE_AND_CAMERA_CLOCKS_SWAPPED")}, False),
    ("the neighbour on the primary's clocks", {"clocks_of": clocks_with("NEIGHBOUR_ON_PRIMARY_CLOCKS")}, False),
    ("light frozen with the instance", {"clocks_of": clocks_with("LIGHT_ON_THE_INSTANCE_CLOCK")}, False),
    ("light on the shot-relative clock", {"clocks_of": clocks_with("LIGHT_ON_THE_SHOT_RELATIVE_CLOCK")}, False),
    ("light on the camera clip clock", {"clocks_of": clocks_with("LIGHT_ON_THE_CAMERA_CLIP_CLOCK")}, False),
    ("light on the clamped edit frame", {"clocks_of": clocks_with("LIGHT_ON_THE_CLAMPED_EDIT_FRAME")}, False),
    ("light on a nominal frame rate", {"clocks_of": clocks_with("LIGHT_ON_A_NOMINAL_FRAME_RATE")}, False),
    ("decks swapped in the mix", {"mix_codes": lambda a8, b8, transition, k: ORIGINAL["mix_codes"](b8, a8, transition, k)}, False),
    # formula mutants: they must also fail the hand vectors
    ("progress without the half frame", {"weight_for": lambda k, transition: ORIGINAL["weight_for"](k - 0.5, transition)}, False),
    ("flicker depth 0.40 instead of 0.45", {"flicker_level": lambda light_id, flicker, seconds: 1 - (1 - ORIGINAL["flicker_level"](light_id, flicker, seconds)) * (0.40 / 0.45)}, True),
    ("hash01 with an arithmetic shift", {"ushr": lambda value, bits: m.to_int32(value) >> bits}, True),
    ("camera one frame ahead", {"camera_pose": lambda clip, frame: ORIGINAL["camera_pose"](clip, frame + 1)}, True),
    ("event progress ends one frame early", {"event_progress": event_end_inclusive}, True),
    ("rays through the pixel corner, not its centre", {"pixel_ray": lambda pose, x, y: ORIGINAL["pixel_ray"](pose, x - 0.5, y - 0.5)}, False),
    ("painter's queue reversed", {"deck_pixel": reversed_painter}, False),
    ("light applied in sRGB instead of linear light", {"decode": lambda code8: code8 / 255.0, "encode": lambda linear: max(0.0, min(1.0, linear))}, True),
    ("seconds from the nominal rate", {"edit_seconds": lambda frame, fps: frame / round(fps[0] / fps[1])}, True),
    ("lights as a point with no falloff", {"light_amount_at": lambda light, point, normal, eye: 1.0}, True),
]


class MutantTests(unittest.TestCase):
    def test_the_pristine_module_rebuilds_the_corpus_section(self):
        self.assertEqual(json.loads(json.dumps(m.build_section())), EXPECTED_SECTION)

    def test_every_mutant_of_the_oracle_changes_or_refuses_the_section(self):
        for name, patches, _ in MUTANTS:
            with patch.multiple(m, **patches):
                self.assertTrue(detected(name), "the corpus does not notice: %s" % name)

    def test_a_different_field_of_view_is_noticed_too(self):
        with patch.object(m, "FOV_DEG", 40.0):
            self.assertTrue(detected("fov 40"))
        with patch.object(m, "VIEWPORT", (640, 480)):
            self.assertTrue(detected("viewport 640x480"))

    def test_the_formula_mutants_also_fail_the_hand_vectors(self):
        for name, patches, formula in MUTANTS:
            if not formula:
                continue
            with patch.multiple(m, **patches):
                self.assertTrue(hand_problems(m), "the hand vectors do not notice: %s" % name)

    def test_the_hand_vectors_notice_a_clock_mutant_too(self):
        # the clocks counted by hand (HAND_CLOCKS) refuse every clock mutant that changes a frame
        for name in ("BOTH_CLOCKS_PINNED_TOGETHER", "INSTANCE_AND_CAMERA_CLOCKS_SWAPPED", "NEIGHBOUR_ON_PRIMARY_CLOCKS"):
            wrong = 0
            for layout, shot, frame, want in HAND_CLOCKS:
                got = ORIGINAL["clocks_of"](m.build_edit(layout), shot, frame, m.HYPOTHESES[name]["clock"],
                                            primary_id=("A" if frame < 30 else "B") if name == "NEIGHBOUR_ON_PRIMARY_CLOCKS" else None)
                if (got["instance_frame"], got["camera_clip_frame"], got["held_instance"], got["held_camera"]) != want:
                    wrong += 1
            self.assertGreaterEqual(wrong, 2, name)


# ---- layer 5: structure -------------------------------------------------------------------------------------------------------


class StructureTests(unittest.TestCase):
    def test_the_corpus_section_is_what_the_generator_produces_today(self):
        self.assertEqual(json.loads(json.dumps(m.build_section())), EXPECTED_SECTION)

    def test_the_build_is_deterministic_and_the_json_is_utf8_lf(self):
        first = d.canonical(m.build_section())
        self.assertEqual(first, d.canonical(m.build_section()))
        self.assertNotIn(b"\r", first)
        text = CORPUS_BYTES.decode("utf-8")
        self.assertNotIn("\r", text)
        for forbidden in ("wally", "/Users/", "C:\\", "origin-output"):
            self.assertNotIn(forbidden, text)

    def test_the_section_keeps_the_origin_formula_kind_and_its_limits(self):
        self.assertEqual(SECTION["kind"], "origin-formula")
        self.assertIn("A validar", SECTION["status"])
        for phrase in ("no runtime, Rust, JSONL, UI or engine surface", "FX_TRANSITION_NOT_YET", "S2", "nothing rendered"):
            self.assertIn(phrase, SECTION["status"])
        self.assertIn("no product transition evaluator", SECTION["independence"])
        self.assertIn("no pure runtime frame fabricated for a mixed hold", SECTION["independence"])
        for probe_text in ("S2", "7.3", "D17"):
            self.assertIn(probe_text, json.dumps(SECTION["plan_sections"]) + SECTION["status"])

    def test_the_destination_text_was_read_not_run_and_is_still_what_it_was(self):
        reads = SECTION["destination_reads"]
        self.assertIn("READ ONLY", reads["status"])
        self.assertGreaterEqual(len(reads["lines"]), 10)
        for row in reads["lines"]:
            self.assertTrue(row["present"], row["line"])
            self.assertIn(row["line"], (HERE.parent / row["file"]).read_text(encoding="utf-8").replace("\r\n", "\n"))

    def test_the_thresholds_are_the_pre_registered_ones_and_the_tolerances_are_the_corpus_ones(self):
        thresholds = SECTION["thresholds"]
        self.assertEqual((thresholds["min_separation_codes"], thresholds["min_rejecting_probes"], thresholds["min_rejecting_frames"]), (4, 3, 2))
        self.assertEqual((thresholds["flat_margin_px"], thresholds["flat_max_code_spread"], thresholds["agree_max_codes"]), (6, 6, 1))
        tolerance = CORPUS["tolerances"]["monitor_and_render"]
        self.assertEqual(thresholds["tolerance_for_the_implementation"]["max_abs_delta_8bit"], tolerance["max_abs_delta_8bit"])
        self.assertEqual(thresholds["tolerance_for_the_implementation"]["max_abs_mean_signed_delta_8bit"], tolerance["bias_limit"]["max_abs_mean_signed_delta_8bit"])
        self.assertEqual(CORPUS["tolerances"]["export_h264"]["max_abs_delta_8bit"], 16)  # imported from F016, unchanged
        self.assertIn("never retuned", thresholds["status"])

    def test_every_scenario_refuses_what_it_must_with_enough_probes_on_enough_frames(self):
        for scenario in SECTION["scenarios"]:
            counts = {name: 0 for name in scenario["must_reject"]}
            frames = {name: set() for name in scenario["must_reject"]}
            for frame in scenario["frames"]:
                for probe in frame.get("probes", []):
                    for short in probe.get("rejects", "").split(","):
                        if short and BY_SHORT[short] in counts:
                            counts[BY_SHORT[short]] += 1
                            frames[BY_SHORT[short]].add(frame["edit_frame"])
            for name, count in counts.items():
                self.assertGreaterEqual(count, 3, (scenario["id"], name))
                self.assertGreaterEqual(len(frames[name]), 2, (scenario["id"], name))
                self.assertEqual(scenario["rejecting_probes"][name], count)

    def test_the_cases_the_plan_asks_for_are_all_there(self):
        held_instance_running_camera, held_camera_running_instance, both_held, none_held = [], [], [], []
        for scenario in SECTION["scenarios"]:
            for frame in scenario["frames"]:
                if not frame["in_window"]:
                    continue
                state = frame["states"][frame["neighbour"]]
                pair = (state["held_instance"], state["held_camera"])
                where = (scenario["id"], frame["edit_frame"])
                if pair == (True, False):
                    held_instance_running_camera.append(where)
                elif pair == (False, True):
                    held_camera_running_instance.append(where)
                elif pair == (True, True):
                    both_held.append(where)
                else:
                    none_held.append(where)
        self.assertGreaterEqual(len(held_instance_running_camera), 6)  # instance held + camera advancing
        self.assertGreaterEqual(len(held_camera_running_instance), 6)  # camera held + instance advancing
        self.assertGreaterEqual(len(both_held), 6)
        self.assertGreaterEqual(len(none_held), 6)
        ids = set(SCENARIOS)
        for needed in ("pos-center-mixed", "pos-end-mixed", "pos-start-mixed", "pos-edge-start", "pos-control-real", "pos-control-held", "lit-center-held",
                       "lit-end-held", "lit-start-held", "lit-center-instance-running", "lit-ntsc-center", "both-center-mixed"):
            self.assertIn(needed, ids)

    def test_both_sides_of_the_cut_the_window_limits_and_the_frames_outside_are_covered(self):
        for scenario in SECTION["scenarios"]:
            frames = {frame["edit_frame"]: frame for frame in scenario["frames"]}
            window = scenario["transition"]["window"]
            self.assertIn(window["start"] - 1, frames, scenario["id"])
            self.assertIn(window["end_exclusive"], frames, scenario["id"])
            self.assertFalse(frames[window["start"] - 1]["in_window"])
            self.assertFalse(frames[window["end_exclusive"]]["in_window"])
            self.assertIn(window["start"], frames)  # k = 0
            if scenario["id"] != "pos-edge-start":  # the edge case looks at the first window frames only
                self.assertIn(window["end_exclusive"] - 1, frames)  # k = D - 1
        center = SCENARIOS["pos-center-mixed"]
        primaries = {frame["primary"] for frame in center["frames"] if frame["in_window"]}
        self.assertEqual(primaries, {"A", "B"})  # both sides of the cut

    def test_the_light_follows_the_real_edit_frame_even_when_both_clocks_are_held(self):
        scenario = SCENARIOS["lit-center-held"]
        saw = 0
        for frame in scenario["frames"]:
            if not frame["in_window"]:
                continue
            state = frame["states"][frame["neighbour"]]
            if state["held_instance"] and state["held_camera"]:
                saw += 1
                self.assertAlmostEqual(state["scene_seconds"], frame["edit_frame"] / 24, places=9)  # f / fps, not the held frame
                self.assertNotAlmostEqual(state["scene_seconds"], state["instance_frame"] / 24, places=3)
        self.assertGreaterEqual(saw, 6)

    def test_a_scene_start_or_offset_tells_the_edit_clock_from_the_clip_clock(self):
        scenario = SCENARIOS["lit-center-instance-running"]
        shot = scenario["shots"]["B"]
        self.assertNotEqual(shot["instance_start"], shot["start"])
        self.assertNotEqual(shot["camera_in"], shot["start"])
        for frame in scenario["frames"]:
            if frame["in_window"]:
                state = frame["states"]["B"]
                self.assertFalse(state["held_instance"])  # the instance RUNS here, and still the light is not on its clock
                self.assertNotEqual(round(state["scene_seconds"] * 24), state["instance_frame"])
                self.assertEqual(round(state["scene_seconds"] * 24), frame["edit_frame"])

    def test_the_rational_rate_is_30000_over_1001_and_the_window_sits_at_a_late_frame(self):
        scenario = SCENARIOS["lit-ntsc-center"]
        self.assertEqual(scenario["fps"], [30000, 1001])
        self.assertEqual(scenario["cut"], 1800)
        frame = next(item for item in scenario["frames"] if item["edit_frame"] == 1794)
        self.assertAlmostEqual(frame["states"]["A"]["scene_seconds"], 1794 * 1001 / 30000, places=9)
        self.assertNotAlmostEqual(frame["states"]["A"]["scene_seconds"], 1794 / 30, places=3)

    def test_handles_agree_with_what_the_neighbour_really_shows(self):
        for scenario in SECTION["scenarios"]:
            held = scenario["neighbour_held_frames"]
            if scenario["transition"]["handles"] == "use":
                self.assertEqual(held, [], scenario["id"])
            else:
                self.assertTrue(held, scenario["id"])

    def test_the_module_stays_independent_of_the_product_and_of_the_corpus_script(self):
        source = (HERE / "pilot_dissolve_analytic.py").read_text(encoding="utf-8")
        tree = ast.parse(source)
        imports = set()
        for node in ast.walk(tree):
            if isinstance(node, ast.Import):
                imports |= {alias.name.split(".")[0] for alias in node.names}
            elif isinstance(node, ast.ImportFrom):
                imports.add((node.module or "").split(".")[0])
        self.assertEqual(imports, {"math", "fractions"})
        for forbidden in ("subprocess", "importlib", "open(", "ctypes"):
            self.assertNotIn(forbidden, source, forbidden)
        self.assertEqual(m.MIN_PYTHON, (3, 9))

    def test_neither_file_uses_a_bare_assert_and_both_are_ascii_safe_text(self):
        for name in ("pilot_dissolve_analytic.py", "pilot_dissolve_analytic.spec.py"):
            text = (HERE / name).read_text(encoding="utf-8")
            self.assertNotIn("\r", text)
            tree = ast.parse(text)
            self.assertEqual([node.lineno for node in ast.walk(tree) if isinstance(node, ast.Assert)], [], name)

    def test_the_main_script_reports_a_disagreement_instead_of_building(self):
        with patch.object(m, "window_of", lambda cut, duration, alignment: (cut, cut + duration)):
            with self.assertRaises(d.Refusal) as caught:
                d.analytic_section(d.repo_facts())
        self.assertEqual(caught.exception.code, "ORACLE_ANALYTIC_DISAGREES")
        with patch.object(m, "build_section", side_effect=m.Refusal("ANALYTIC_NO_REFUSAL_POWER", "x")):
            with self.assertRaises(d.Refusal) as caught:
                d.analytic_section(d.repo_facts())
        self.assertEqual(caught.exception.code, "ORACLE_ANALYTIC_NO_REFUSAL_POWER")

    def test_the_manifest_counts_and_declares_the_analytic_section(self):
        manifest = json.loads((d.CORPUS / d.MANIFEST_NAME).read_text(encoding="utf-8"))
        counts = manifest["corpus"]["counts"]
        self.assertEqual(counts["analytic_scenarios"], len(SECTION["scenarios"]))
        self.assertEqual(counts["analytic_hypotheses"], 9)
        self.assertEqual(counts["analytic_probes"], sum(len(frame.get("probes", [])) for scenario in SECTION["scenarios"] for frame in scenario["frames"]))
        ids = {item["id"] for item in manifest["assumptions"]}
        self.assertTrue({"ANALYTIC_SCENES_NOT_RENDERED", "ANALYTIC_FLAT_PLANES_ONLY"} <= ids)
        self.assertIn("TEXTURED_FRAMES_NOT_REAL_BEFORE_S2", {item["id"] for item in manifest["blockers"]})


if __name__ == "__main__":
    unittest.main(verbosity=1)
