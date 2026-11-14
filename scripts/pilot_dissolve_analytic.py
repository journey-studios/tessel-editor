#!/usr/bin/env python3
"""Analytic reference X for mixed-hold decks and the REAL scene clock (P4b-S0, review finding P2-2).

Companion of `scripts/pilot-dissolve-corpus.py` (plan `docs/migration/p4b-cross-dissolve-plan.md`, 7.3, D14/D15/D17,
T06/T10/T16). It is imported by that script; run on its own it only prints a summary. Python standard library
only, Python >= 3.9, no bare `assert` (the module behaves the same under `python -O`).

WHY IT EXISTS. The textured reference of the plan (8-bit bytes of the runtime rendering a PURE frame, then the mix in
Python) cannot serve two situations, because no pure frame equals them:

  * MIXED HOLD: the neighbour deck of a dissolve shows its scene instance HELD at one frame while its camera RUNS
    (or the reverse): the two clamps of `resolveShotAtOffset` are independent (D14/D15);
  * THE LIGHT CLOCK: lights flicker on the REAL edit frame `f / fps`, whatever the instance and camera are doing,
    so a held deck still flickers (D17). The held deck's flicker belongs to no pure frame either.

So S0 fixes, with no runtime and no product code:

  1. SYNTHETIC SCENES: a backdrop and a marker (image planes with flat 8-bit colours), an optional lamp, an explicit
     camera clip, an explicit projection and viewport (kits `pos`, `lit`, `both`);
  2. EXPECTED TEMPORAL STATES per frame: instance frame, camera clip frame, held flags, the scene seconds and the
     lights' effective intensity (instance and camera clamped apart, the light on the real edit frame);
  3. ANALYTIC PROBES: the projected position of the marker (continuous pixels) and the 8-bit colour at points that lie
     well inside flat areas, in each deck and after the mix, with the pre-registered tolerances of the corpus
     (1/255 per channel, |mean signed delta| <= 0.25; H.264 16, imported from F016);
  4. MUTANT HYPOTHESES (`HYPOTHESES`): wrong rules a product could implement (both clocks pinned together, the two clocks
     swapped, the neighbour on the primary shot's clocks, the decks swapped, the light on the instance clock, on the
     shot-relative clock, on the camera clip clock, on the clamped edit frame, on a nominal frame rate). For every
     scenario the generator checks that the hypotheses it must refuse really move at least MIN_REJECTING_PROBES probes by
     MIN_SEPARATION_CODES or more, and that the ones that must agree (a control) move none. A scenario without refusal
     power is a REFUSAL of the generator, never a quiet pass.

WHAT THE KERNELS ARE. Transcriptions of the origin text (tesselapp tip, read with `git show`, never executed):
`weather/weather-model.ts` `hash01`, `lighting/scene-lighting.ts` (`seedOf`, `noise`, `flickerLevel`, `lightAmount`),
`camera/resolve-camera-clip.ts` (truck/dolly/hold, `progressAt`, the final pose held past the clip),
`performance/resolve-performance.ts` (`event` channel), `edit/resolve-shot-at-offset.ts`, `types/fps.ts` and
`runtime/three-scene-runtime.ts` (a Three.js `PerspectiveCamera` with a vertical fov, `lookAt`, pixel centres). The
destination's own runtime text (`runtime/src/directed-scene.js`: decode, multiply by `1 + L`, encode; planes drawn in
plan order) was READ to reconcile the formulas; nothing of it is imported or called, and no Rust is used. The kernels
share nothing with `scripts/pilot-dissolve-corpus.py` (the spec checks that both agree where they overlap).

WHAT IT IS NOT. A contract and an oracle in Python. No textured frame becomes real before S2 (the runtime), no Rust,
JSONL or UI surface exists in S0, and the public gate `FX_TRANSITION_NOT_YET` stays closed until S2A. The expectations
are `origin-formula` (the transcribed origin text evaluated here). Nothing was rendered, measured or validated on pixels.
"""

import math
from fractions import Fraction

MIN_PYTHON = (3, 9)

# ---- pre-registered, before any pixel exists; never retuned after a result -------------------------------------

VIEWPORT = (640, 360)
FOV_DEG = 50.0  # Three.js PerspectiveCamera: VERTICAL fov in degrees, aspect = width / height
UP = (0.0, 1.0, 0.0)
FLAT_MARGIN_PX = 6  # a probe is well inside a flat area: the 3x3 samples at +-6 px see the same plane in each deck
FLAT_MAX_CODE_SPREAD = 6  # and no colour moves more than this inside it: 12 px at <= 0.5 code per px, so a half-pixel misplacement moves <= 0.25 code (the bias limit)
MIN_SEPARATION_CODES = 4  # a hypothesis REJECTS a probe when it differs by this many codes (the tolerance is 1)
MIN_REJECTING_PROBES = 3  # at least this many rejecting probes, on at least MIN_REJECTING_FRAMES frames
MIN_REJECTING_FRAMES = 2
AGREE_MAX_CODES = 1  # a control that must agree: no probe moves by more than the tolerance
TIE_MARGIN = 1e-6  # an unrounded 8-bit value this close to a rounding tie is refused (host independence)
CLIP_MARGIN = 1e-9  # a lit linear value above 1 (clamped by the runtime) would flatten a discrimination: refused
MAX_PROBE_PIXEL_SHIFT = 2  # decoys: the hypothesised marker centre must differ from the true one by > 2 * margin

TOLERANCE_REFERENCE = {
    "max_abs_delta_8bit": 1,
    "max_abs_mean_signed_delta_8bit": 0.25,
    "source": "dissolve-cases.json tolerances.monitor_and_render (plan section 12), imported, not derived from any result",
}


class Refusal(Exception):
    """A typed refusal of the analytic generator (a design error of a kit, never a quiet pass)."""

    def __init__(self, code, detail=""):
        super().__init__("%s: %s" % (code, detail))
        self.code = code
        self.detail = detail


# ---- 32-bit integer semantics of JavaScript (Math.imul, `| 0`, `>>>`) ------------------------------------


def to_int32(value):
    value &= 0xFFFFFFFF
    return value - 0x100000000 if value & 0x80000000 else value


def imul(a, b):
    return to_int32(to_int32(a) * to_int32(b))


def ushr(value, bits):
    return (value & 0xFFFFFFFF) >> bits


def hash01(n, salt):
    """weather/weather-model.ts: [0, 1) from a 32-bit integer mix."""
    h = imul(n, 0x9E3779B1) ^ imul(salt, 0x85EBCA6B)
    h = to_int32(h)
    h = to_int32(h ^ ushr(h, 15))
    h = imul(h, 0x2C1B3C6D)
    h = to_int32(h ^ ushr(h, 12))
    h = imul(h, 0x297A2D39)
    h = to_int32(h ^ ushr(h, 15))
    return (h & 0xFFFFFFFF) / 4294967296.0


def smooth_unit(t):
    return t * t * (3 - 2 * t)


def value_noise(seconds, seed):
    """lighting/scene-lighting.ts `noise`: smooth value noise of time, from the light's own seed."""
    i = math.floor(seconds)
    f = seconds - i
    low = hash01(i, seed)
    return low + (hash01(i + 1, seed) - low) * smooth_unit(f)


def seed_of(light_id):
    """`seedOf`: Math.imul(seed, 31) + code point, the sum NOT wrapped until the next imul or `| 0`."""
    seed = 7
    for char in light_id:
        seed = imul(seed, 31) + ord(char)
    return seed


def flicker_level(light_id, flicker, seconds):
    """`flickerLevel`: 1 at rest; a sway (x3) and a flutter (x13) take up to 45% off at flicker 1."""
    if flicker <= 0:
        return 1.0
    seed = seed_of(light_id)
    sway = value_noise(seconds * 3, seed)
    flutter = value_noise(seconds * 13, seed + 101)
    return 1 - flicker * 0.45 * (0.6 * sway + 0.4 * flutter)


def edit_seconds(frame, fps):
    """`types/fps.ts` editFrameToSeconds: frame / (numerator / denominator), the REAL edit frame."""
    return frame / (fps[0] / fps[1])


# ---- colour: 8-bit codes, IEC 61966-2-1, half-up rounding ----------------------------------------------------


def clamp01(value):
    return 0.0 if value <= 0 else 1.0 if value >= 1 else value


def decode(code8):
    v = code8 / 255.0
    return v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4


def encode(linear):
    v = clamp01(linear)
    return v * 12.92 if v <= 0.0031308 else 1.055 * v ** (1.0 / 2.4) - 0.055


def code_of(unrounded_255):
    """Half-up rounding to 8 bits; a value within TIE_MARGIN of a tie is refused."""
    frac = unrounded_255 - math.floor(unrounded_255)
    if abs(frac - 0.5) < TIE_MARGIN:
        raise Refusal("ANALYTIC_TIE_MARGIN", "%r is too close to a rounding tie to be host independent" % unrounded_255)
    return int(min(255, max(0, math.floor(unrounded_255 + 0.5))))


def r9(value):
    return round(value, 9) + 0.0


# the corpus section writes tuples as strings so that its canonical JSON (one array element per line) stays small:
# colours "#rrggbb", pixels "x,y", continuous points "a,b" and rectangles "left,top,right,bottom" with 9 decimals
def hex8(code):
    return "#%02x%02x%02x" % tuple(code)


def parse_hex8(text):
    return [int(text[1:3], 16), int(text[3:5], 16), int(text[5:7], 16)]


def tuple_text(values):
    return ",".join(("%d" % value) if isinstance(value, int) else ("%.9f" % value).rstrip("0").rstrip(".") or "0" for value in values)


# ---- the transition: window, progress, weight, mix (an own copy: the spec checks it against the corpus script) ----


def window_of(cut, duration, alignment):
    start = cut
    if alignment == "center":
        start = cut - duration // 2
    if alignment == "end":
        start = cut - duration
    return start, start + duration


def ease(t, name):
    t = clamp01(t)
    if name == "linear":
        return t
    if name == "easeInOutCubic":
        if t < 0.5:
            return 4 * t * t * t
        p = -2 * t + 2
        return 1 - (p * p * p) / 2
    if name == "easeOutCubic":
        p = 1 - t
        return 1 - p * p * p
    raise Refusal("ANALYTIC_UNKNOWN_EASING", str(name))


def mixf(x, y, a):
    return x * (1.0 - a) + y * a


def weight_for(k, transition):
    """progress = ease((k + 0.5) / D); shaped = mix(progress, smoothstep(progress), curve == smooth);
    weight = mix(progress, shaped, clamp01(intensity)). The k-th frame of the window is `f - start`."""
    progress = ease((k + 0.5) / transition["duration"], transition["easing"])
    shaped = mixf(progress, smooth_unit(clamp01(progress)), 1.0 if transition["curve"] == "smooth" else 0.0)
    amount = clamp01(transition["intensity"])
    return progress, shaped, amount, mixf(progress, shaped, amount)


def mix_codes(a8, b8, transition, k):
    """The shader on 8-bit sRGB decks, in linear light: mix(mix(a, b, p), mix(a, b, shaped), amount)."""
    progress, shaped, amount, _ = weight_for(k, transition)
    out = []
    for a_code, b_code in zip(a8, b8):
        a, b = decode(a_code), decode(b_code)
        linear = mixf(mixf(a, b, progress), mixf(a, b, shaped), amount)
        out.append(code_of(encode(linear) * 255.0))
    return out


# ---- the edit model: what each deck shows (an own transcription of resolveShotAtOffset + D17) -------------------


def limit_frame(wanted, frames):
    if frames <= 0:
        return 0, True
    frame = max(0, min(frames - 1, wanted))
    return frame, frame != wanted


class HypothesisClock:
    """The wrong rule a mutant applies. `None` of every field is the truth."""

    def __init__(self, pins_together=False, swapped=False, primary_numbers=False, decks_swapped=False, light=None):
        self.pins_together = pins_together
        self.swapped = swapped
        self.primary_numbers = primary_numbers
        self.decks_swapped = decks_swapped
        self.light = light


HYPOTHESES = {
    "BOTH_CLOCKS_PINNED_TOGETHER": {
        "short": "PIN",
        "wrong_rule": "the offset is clamped ONCE, to the range both the instance and the camera have, so a held clock holds the other",
        "plan": "D14/D15: instance and camera are clamped independently",
        "clock": HypothesisClock(pins_together=True),
    },
    "INSTANCE_AND_CAMERA_CLOCKS_SWAPPED": {
        "short": "SWP",
        "wrong_rule": "the instance frame is computed with the camera's start and length, and the camera frame with the instance's",
        "plan": "D14/D15",
        "clock": HypothesisClock(swapped=True),
    },
    "NEIGHBOUR_ON_PRIMARY_CLOCKS": {
        "short": "PRI",
        "wrong_rule": "the neighbour deck uses the primary shot's start, instance start and camera in, instead of its own",
        "plan": "D14: the neighbour deck is the OTHER shot, asked for the offset outside its range",
        "clock": HypothesisClock(primary_numbers=True),
    },
    "DECKS_SWAPPED_IN_THE_MIX": {
        "short": "DSW",
        "wrong_rule": "mix(b, a, ...) instead of mix(a, b, ...): the shot after the cut is taken for the shot before it",
        "plan": "T29 (decks trocados)",
        "clock": HypothesisClock(decks_swapped=True),
    },
    "LIGHT_ON_THE_INSTANCE_CLOCK": {
        "short": "LIN",
        "wrong_rule": "the light's seconds are the (clamped) instance frame / fps, so a held deck freezes its flicker",
        "plan": "D17: a held deck still flickers",
        "clock": HypothesisClock(light="instance"),
    },
    "LIGHT_ON_THE_SHOT_RELATIVE_CLOCK": {
        "short": "LSR",
        "wrong_rule": "the light's seconds are (f - shot start) / fps, the time inside the shot, not the edit's",
        "plan": "D17: the scene clock is the REAL edit frame",
        "clock": HypothesisClock(light="shot_relative"),
    },
    "LIGHT_ON_THE_CAMERA_CLIP_CLOCK": {
        "short": "LCC",
        "wrong_rule": "the light's seconds are the (clamped) camera clip frame / fps, the clip's clock instead of the edit's",
        "plan": "D17",
        "clock": HypothesisClock(light="camera_clip"),
    },
    "LIGHT_ON_THE_CLAMPED_EDIT_FRAME": {
        "short": "LCE",
        "wrong_rule": "the light's seconds are the edit frame clamped into the deck's own shot range / fps (the neighbour is stuck at its first or last frame)",
        "plan": "D17: f is never limited to the shot",
        "clock": HypothesisClock(light="clamped_edit"),
    },
    "LIGHT_ON_A_NOMINAL_FRAME_RATE": {
        "short": "LNF",
        "wrong_rule": "the light's seconds are f / round(numerator / denominator): 30 instead of 30000/1001",
        "plan": "T06: rational fps",
        "clock": HypothesisClock(light="nominal_fps"),
    },
}


def shot_numbers(shot, primary):
    return primary if primary is not None else shot


def clocks_of(edit, shot_id, f, hypothesis=None, primary_id=None):
    """The state of the deck of `shot_id` at the real edit frame `f` (truth when `hypothesis` is None).

    Truth: delta = f - shot.start; instance = clamp(instanceStart + delta), camera = clamp(cameraIn + delta), each
    clamp apart and each with its own `held` flag; the scene seconds of the light = f / (num / den), the REAL
    frame, never limited."""
    shot = edit["shots"][shot_id]
    numbers = shot
    if hypothesis is not None and hypothesis.primary_numbers and primary_id is not None:
        numbers = edit["shots"][primary_id]
    instance_frames = edit["instances"][shot["instance"]]
    camera_frames = edit["cameras"][shot["camera"]]
    delta = f - shot["start"]
    if hypothesis is not None and hypothesis.primary_numbers and primary_id is not None:
        delta = f - numbers["start"]
    wanted_instance = numbers["instance_start"] + delta
    wanted_camera = numbers["camera_in"] + delta
    if hypothesis is not None and hypothesis.swapped:
        # the instance frame computed with the camera's numbers and the camera frame with the instance's
        instance, held_instance = limit_frame(numbers["camera_in"] + delta, camera_frames)
        camera, held_camera = limit_frame(numbers["instance_start"] + delta, instance_frames)
    elif hypothesis is not None and hypothesis.pins_together:
        low = max(-numbers["instance_start"], -numbers["camera_in"])
        high = min(instance_frames - 1 - numbers["instance_start"], camera_frames - 1 - numbers["camera_in"])
        joint = max(low, min(high, delta)) if instance_frames > 0 and camera_frames > 0 else 0
        instance, held_instance = limit_frame(numbers["instance_start"] + joint, instance_frames)
        camera, held_camera = limit_frame(numbers["camera_in"] + joint, camera_frames)
        held_instance = held_camera = joint != delta
    else:
        instance, held_instance = limit_frame(wanted_instance, instance_frames)
        camera, held_camera = limit_frame(wanted_camera, camera_frames)
    seconds = edit_seconds(f, edit["fps"])
    mode = hypothesis.light if hypothesis is not None else None
    if mode == "instance":
        seconds = edit_seconds(instance, edit["fps"])
    elif mode == "shot_relative":
        seconds = edit_seconds(f - shot["start"], edit["fps"])
    elif mode == "camera_clip":
        seconds = edit_seconds(camera, edit["fps"])
    elif mode == "clamped_edit":
        seconds = edit_seconds(max(shot["start"], min(shot["end"] - 1, f)), edit["fps"])
    elif mode == "nominal_fps":
        seconds = f / float(round(edit["fps"][0] / edit["fps"][1]))
    return {
        "shot": shot_id,
        "delta": delta,
        "edit_frame": f,
        "instance_frame": instance,
        "camera_clip_frame": camera,
        "held_instance": held_instance,
        "held_camera": held_camera,
        "scene_seconds": seconds,
    }


# ---- the scenes: planes, an event channel, lamps, a camera clip --------------------------------------------------


def event_progress(instance_frame, start, duration):
    """`performance` event channel: 0 before the start, 1 from start + duration on (holdFinal is ignored)."""
    if instance_frame < start:
        return 0.0
    if instance_frame >= start + duration:
        return 1.0
    return (instance_frame - start) / duration


def cross(a, b):
    return (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0])


def unit(v):
    length = math.sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2])
    return (v[0] / length, v[1] / length, v[2] / length)


def add3(a, b):
    return (a[0] + b[0], a[1] + b[1], a[2] + b[2])


def lerp3(a, b, t):
    return (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t)


def segment_end(pose, segment):
    position, target = pose
    forward = unit((target[0] - position[0], target[1] - position[1], target[2] - position[2]))
    recipe = segment["recipe"]
    if recipe == "hold":
        return pose
    if recipe in ("dollyIn", "dollyOut"):
        sign = 1.0 if recipe == "dollyIn" else -1.0
        delta = tuple(c * sign * segment["distance"] for c in forward)
    elif recipe in ("truckRight", "truckLeft"):
        sign = 1.0 if recipe == "truckRight" else -1.0
        right = cross(forward, UP)
        length = math.sqrt(sum(c * c for c in right))
        if length > 1e-9:
            right = tuple(c / length for c in right)
        delta = tuple(c * sign * segment["distance"] for c in right)
    else:
        raise Refusal("ANALYTIC_CAMERA_RECIPE", "the kits use hold, dolly and truck only: %s" % recipe)
    return (add3(position, delta), add3(target, delta))


def camera_clip_frames(clip):
    return sum(segment["frames"] for segment in clip["segments"])


def camera_pose(clip, frame):
    """`resolveMainTrack`: the movements one after another from the initial pose; past the last one the camera rests
    on the pose it ended with. `progressAt`: ease(local / max(1, frames - 1)); a hold has no easing."""
    initial = (tuple(clip["position"]), tuple(clip["target"]))
    carry = initial
    cursor = 0
    for segment in clip["segments"]:
        start_pose = initial if segment.get("from") == "initial" else carry
        end_pose = cursor + segment["frames"]
        if frame < end_pose:
            local = frame - cursor
            t = local / max(1, segment["frames"] - 1)
            if segment["recipe"] != "hold":
                t = ease(t, segment["easing"])
            end = segment_end(start_pose, segment)
            return lerp3(start_pose[0], end[0], t), lerp3(start_pose[1], end[1], t)
        carry = segment_end(start_pose, segment)
        cursor = end_pose
    return carry


def plane_position(plane, instance_frame):
    """localPosition = base + delta * p (an `event` channel adds to the base)."""
    position = tuple(plane["position"])
    event = plane.get("event")
    if event is None:
        return position
    p = event_progress(instance_frame, event["start"], event["duration"])
    return (position[0] + event["delta"][0] * p, position[1] + event["delta"][1] * p, position[2] + event["delta"][2] * p)


def project(point, pose):
    """World point -> (pixel x, pixel y) continuous, the edge convention: the pixel centre of index i is i + 0.5.
    Three.js: PerspectiveCamera(vertical fov, width / height), lookAt, up (0, 1, 0)."""
    position, target = pose
    forward = unit((target[0] - position[0], target[1] - position[1], target[2] - position[2]))
    right = unit(cross(forward, UP))
    true_up = cross(right, forward)
    rel = (point[0] - position[0], point[1] - position[1], point[2] - position[2])
    depth = rel[0] * forward[0] + rel[1] * forward[1] + rel[2] * forward[2]
    if depth <= 1e-9:
        raise Refusal("ANALYTIC_BEHIND_CAMERA", "a marker behind the camera has no projection")
    width, height = VIEWPORT
    tan_half = math.tan(math.radians(FOV_DEG) / 2.0)
    aspect = width / height
    ndc_x = (rel[0] * right[0] + rel[1] * right[1] + rel[2] * right[2]) / depth / (tan_half * aspect)
    ndc_y = (rel[0] * true_up[0] + rel[1] * true_up[1] + rel[2] * true_up[2]) / depth / tan_half
    return (ndc_x + 1.0) / 2.0 * width, (1.0 - ndc_y) / 2.0 * height


def pixel_ray(pose, x, y):
    """The ray through the CENTRE of pixel (x, y): ndc = (i + 0.5) / size * 2 - 1, y flipped."""
    position, target = pose
    forward = unit((target[0] - position[0], target[1] - position[1], target[2] - position[2]))
    right = unit(cross(forward, UP))
    true_up = cross(right, forward)
    width, height = VIEWPORT
    tan_half = math.tan(math.radians(FOV_DEG) / 2.0)
    ndc_x = (x + 0.5) / width * 2.0 - 1.0
    ndc_y = 1.0 - (y + 0.5) / height * 2.0
    sx, sy = ndc_x * tan_half * (width / height), ndc_y * tan_half
    direction = tuple(forward[i] + sx * right[i] + sy * true_up[i] for i in range(3))
    return position, direction


def light_amount_at(light, point, normal, eye):
    """`lightAmount` with the shader's double-sided rule (the normal faces the camera)."""
    if sum(normal[i] * (eye[i] - point[i]) for i in range(3)) < 0:
        normal = tuple(-c for c in normal)
    to_light = tuple(light["position"][i] - point[i] for i in range(3))
    distance = math.sqrt(sum(c * c for c in to_light))
    if distance < 1e-9 or light["radius"] <= 0:
        return 0.0
    facing = max(0.0, sum(normal[i] * to_light[i] for i in range(3)) / distance)
    falloff = 1.0 / (1.0 + (distance / light["radius"]) ** 2)
    return facing * falloff * falloff


def deck_lights(deck, state):
    """The lights of a deck at its scene seconds: effective intensity = intensity * flickerLevel(seconds)."""
    rows = []
    for light in deck["lights"]:
        level = flicker_level(light["id"], light["flicker"], state["scene_seconds"])
        rows.append({"id": light["id"], "flicker_level": level, "effective_intensity": light["intensity"] * level})
    return rows


def deck_pixel(deck, state, x, y):
    """What a deck draws at pixel (x, y): (covering plane id or None, unrounded 8-bit colour, 8-bit code).
    Planes are drawn in `render_order` (the painter's queue: the last one that covers the pixel wins); a lit plane is
    decoded, multiplied by (1 + L) and encoded, in linear light, clamped to [0, 1]."""
    pose = camera_pose(deck["camera"], state["camera_clip_frame"])
    origin, direction = pixel_ray(pose, x, y)
    chosen = None
    for plane in sorted(deck["planes"], key=lambda item: item["render_order"]):
        centre = plane_position(plane, state["instance_frame"])
        if abs(direction[2]) < 1e-12:
            continue
        t = (centre[2] - origin[2]) / direction[2]
        if t <= 0:
            continue
        hit = tuple(origin[i] + direction[i] * t for i in range(3))
        if abs(hit[0] - centre[0]) <= plane["width"] / 2.0 and abs(hit[1] - centre[1]) <= plane["height"] / 2.0:
            chosen = (plane, hit)
    if chosen is None:
        return None, [0.0, 0.0, 0.0], [0, 0, 0]
    plane, hit = chosen
    base = plane["colour8"]
    if not deck["lights"]:
        return plane["id"], [float(c) for c in base], [int(c) for c in base]
    total = [0.0, 0.0, 0.0]
    for light, row in zip(deck["lights"], deck_lights(deck, state)):
        amount = light_amount_at(light, hit, (0.0, 0.0, 1.0), origin) * row["effective_intensity"]
        for channel in range(3):
            total[channel] += light["colour"][channel] * amount
    unrounded = []
    for channel in range(3):
        linear = decode(base[channel]) * (1.0 + total[channel])
        if linear > 1.0 + CLIP_MARGIN:
            raise Refusal("ANALYTIC_LIT_CLIPPED", "%s %s channel %d lit above 1 (%r): the runtime clamps it" % (deck["id"], plane["id"], channel, linear))
        unrounded.append(encode(linear) * 255.0)
    return plane["id"], unrounded, [code_of(value) for value in unrounded]


def marker_geometry(deck, state):
    """The marker's world centre and its continuous pixel rectangle (the edges projected, corners on the plane)."""
    plane = next(item for item in deck["planes"] if item["id"] == "marker")
    centre = plane_position(plane, state["instance_frame"])
    pose = camera_pose(deck["camera"], state["camera_clip_frame"])
    cx, cy = project(centre, pose)
    left, top = project((centre[0] - plane["width"] / 2.0, centre[1] + plane["height"] / 2.0, centre[2]), pose)
    right, bottom = project((centre[0] + plane["width"] / 2.0, centre[1] - plane["height"] / 2.0, centre[2]), pose)
    return {"world_centre": tuple_text([r9(c) for c in centre]), "centre_px": tuple_text([r9(cx), r9(cy)]),
            "rect_px": tuple_text([r9(left), r9(top), r9(right), r9(bottom)]), "_centre": (cx, cy)}


# ---- the kits: the synthetic scenes --------------------------------------------------------------------------


TRUCK_UNITS_PER_FRAME = 0.36  # about 23 px per frame at z = 0: a one-frame difference of the camera clock moves the marker by 23 px


def truck_clip(frames):
    distance = TRUCK_UNITS_PER_FRAME * (frames - 1)
    return {"position": [-2.0, 0.0, 6.0], "target": [-2.0, 0.0, 0.0],
            "segments": [{"recipe": "truckRight", "frames": frames, "from": "initial", "distance": distance, "easing": "linear"}]}


def hold_clip(frames):
    return {"position": [0.0, 0.0, 6.0], "target": [0.0, 0.0, 0.0],
            "segments": [{"recipe": "hold", "frames": frames, "from": "initial"}]}


# a burst of 0.4 scene units per instance frame upward over 12 frames (about 26 px per frame at z = 0); each deck
# bursts over the instance frames its OWN shot uses around the window, so the instance clock is read at 26 px per frame
MARKER_EVENT = {"A": {"x": 7.5, "start": 30}, "B": {"x": 2.2, "start": 0}}


def moving_marker(colour, shot_id):
    burst = MARKER_EVENT[shot_id]
    return {"id": "marker", "colour8": colour, "width": 0.6, "height": 0.6, "position": [burst["x"], -2.4, 0.0], "render_order": 1,
            "event": {"start": burst["start"], "duration": 12, "delta": [0.0, 4.8, 0.0]}}


def static_marker(colour):
    return {"id": "marker", "colour8": colour, "width": 1.0, "height": 1.0, "position": [1.5, -0.5, 0.0], "render_order": 1}


def backdrop(colour):
    return {"id": "backdrop", "colour8": colour, "width": 60.0, "height": 12.0, "position": [0.0, 0.0, -2.0], "render_order": 0}


# a lamp two scene units in front of the marker's plane, a large radius and an intensity of 4: the light is a
# multiplier of about 2.5 to 4.5 on the unlit colour, and the flicker moves that multiplier by tens of codes
LAMPS = {
    "A": {"id": "lamp-a", "position": [-1.5, 1.0, 2.0], "colour": [1.0, 0.7, 0.4], "intensity": 24.0, "radius": 20.0, "flicker": 1.0},
    "B": {"id": "lamp-b", "position": [1.0, 1.5, 2.0], "colour": [0.4, 0.7, 1.0], "intensity": 24.0, "radius": 20.0, "flicker": 1.0},
}

KIT_COLOURS = {
    "pos": {"A": {"backdrop": [24, 40, 96], "marker": [232, 184, 32]}, "B": {"backdrop": [96, 40, 24], "marker": [32, 200, 160]}},
    "lit": {"A": {"backdrop": [28, 36, 44], "marker": [40, 44, 28]}, "B": {"backdrop": [44, 36, 28], "marker": [28, 44, 52]}},
    "both": {"A": {"backdrop": [20, 28, 44], "marker": [40, 44, 20]}, "B": {"backdrop": [44, 28, 20], "marker": [20, 48, 44]}},
}

KITS = {
    "pos": {"moving": True, "lit": False,
            "doc": "a backdrop and a marker that moves up on its instance clock; the camera trucks right on its own clip; no light"},
    "lit": {"moving": False, "lit": True,
            "doc": "a backdrop and a static marker lit by one flickering lamp per deck; a static camera: only the scene clock moves"},
    "both": {"moving": True, "lit": True,
             "doc": "the moving marker of `pos` with the truck and a flickering lamp per deck: positions and light together"},
}

# layouts: two shots A [0, 30) and B [30, 60) (the cut at 30); numbers chosen so that the clocks of the neighbour deck differ
LAYOUTS = {
    "mixed": {
        "handles": "hold", "edit_duration": 60, "fps": [24, 1],
        "shots": {"A": {"start": 0, "end": 30, "instance_start": 4, "camera_in": 0}, "B": {"start": 30, "end": 60, "instance_start": 0, "camera_in": 12}},
        "instance_frames": {"A": 60, "B": 60}, "camera_frames": {"A": 30, "B": 40},
        "doc": "B head: instance HELD (instanceStart 0) while the camera RUNS (cameraIn 12); A tail: instance RUNS (60 frames) while the camera is HELD (30-frame clip, shot uses all of it)",
    },
    "real": {
        "handles": "use", "edit_duration": 60, "fps": [24, 1],
        "shots": {"A": {"start": 0, "end": 30, "instance_start": 4, "camera_in": 0}, "B": {"start": 30, "end": 60, "instance_start": 10, "camera_in": 12}},
        "instance_frames": {"A": 60, "B": 60}, "camera_frames": {"A": 40, "B": 40},
        "doc": "control with real frames on both clocks of both decks (handles use would be accepted): nothing is held",
    },
    "held": {
        "handles": "hold", "edit_duration": 60, "fps": [24, 1],
        "shots": {"A": {"start": 0, "end": 30, "instance_start": 0, "camera_in": 0}, "B": {"start": 30, "end": 60, "instance_start": 0, "camera_in": 0}},
        "instance_frames": {"A": 30, "B": 60}, "camera_frames": {"A": 30, "B": 40},
        "doc": "both clocks of the neighbour held on both sides: the two clamps coincide (A's instance and camera both end at its frame 29), nothing but the scene clock moves",
    },
    "edge": {
        "handles": "hold", "edit_duration": 60, "fps": [24, 1],
        "shots": {"A": {"start": 0, "end": 30, "instance_start": 4, "camera_in": 0}, "B": {"start": 30, "end": 60, "instance_start": 20, "camera_in": 12}},
        "instance_frames": {"A": 37, "B": 60}, "camera_frames": {"A": 32, "B": 40},
        "doc": "the clamps start INSIDE the window, apart: A's tail has instance frames 34, 35, 36 real then held at 36 (wanted 37), and camera frames 30, 31 real then held at 31 (wanted 32): "
               "k = 0, 1 both real; k = 2 camera held, instance real (the last real instance frame); k >= 3 both held. B's instance starts at 20, so its marker has finished its burst and rests at the top",
    },
    "held-light": {
        "handles": "hold", "edit_duration": 60, "fps": [24, 1],
        "shots": {"A": {"start": 0, "end": 30, "instance_start": 0, "camera_in": 0}, "B": {"start": 30, "end": 60, "instance_start": 0, "camera_in": 0}},
        "instance_frames": {"A": 12, "B": 60}, "camera_frames": {"A": 12, "B": 40},
        "doc": "the neighbour frozen far from the edit clock (A's 12-frame instance and camera freeze at frame 11 while the edit is at 30..41; B freezes at frame 0 while the edit is at 18..29): the light still follows the edit",
    },
    "held-ntsc": {
        "handles": "hold", "edit_duration": 3600, "fps": [30000, 1001],
        "shots": {"A": {"start": 0, "end": 1800, "instance_start": 0, "camera_in": 0}, "B": {"start": 1800, "end": 3600, "instance_start": 0, "camera_in": 0}},
        "instance_frames": {"A": 1800, "B": 1800}, "camera_frames": {"A": 1800, "B": 1800},
        "doc": "30000/1001, cut at frame 1800 (about 60 s): the neighbour held, only the scene clock moves; a nominal 30 fps is a different second",
    },
}


def build_edit(layout):
    spec = LAYOUTS[layout]
    shots = {}
    for shot_id, numbers in spec["shots"].items():
        shots[shot_id] = dict(numbers, instance="inst-" + shot_id.lower(), camera="cam-" + shot_id.lower())
    return {"fps": spec["fps"], "edit_duration": spec["edit_duration"], "shots": shots,
            "instances": {"inst-" + key.lower(): value for key, value in spec["instance_frames"].items()},
            "cameras": {"cam-" + key.lower(): value for key, value in spec["camera_frames"].items()}}


def build_deck(kit, shot_id, layout):
    colours = KIT_COLOURS[kit][shot_id]
    spec = KITS[kit]
    frames = LAYOUTS[layout]["camera_frames"][shot_id]
    return {
        "id": shot_id,
        "planes": [backdrop(colours["backdrop"]), moving_marker(colours["marker"], shot_id) if spec["moving"] else static_marker(colours["marker"])],
        "lights": [dict(LAMPS[shot_id])] if spec["lit"] else [],
        "camera": truck_clip(frames) if spec["moving"] else hold_clip(frames),
    }


# ---- scenarios -------------------------------------------------------------------------------------------------

TRANSITION = {"duration": 12, "alignment": "center", "handles": "hold", "easing": "easeInOutCubic", "curve": "smooth", "intensity": 1}
POSITION_HYPOTHESES = ["BOTH_CLOCKS_PINNED_TOGETHER", "INSTANCE_AND_CAMERA_CLOCKS_SWAPPED", "NEIGHBOUR_ON_PRIMARY_CLOCKS"]
LIGHT_HYPOTHESES = ["LIGHT_ON_THE_INSTANCE_CLOCK", "LIGHT_ON_THE_SHOT_RELATIVE_CLOCK", "LIGHT_ON_THE_CAMERA_CLIP_CLOCK"]

SCENARIOS = [
    {"id": "pos-center-mixed", "kit": "pos", "layout": "mixed", "alignment": "center", "ks": [0, 2, 4, 5, 6, 7, 9, 11],
     "role": "mixed hold, both sides of the cut: B's instance held while its camera runs (before the cut), A's camera held while its instance runs (after it)",
     "rejects": POSITION_HYPOTHESES + ["DECKS_SWAPPED_IN_THE_MIX"], "agrees": []},
    {"id": "pos-end-mixed", "kit": "pos", "layout": "mixed", "alignment": "end", "ks": [0, 4, 7, 8, 9, 10, 11],
     "role": "window [18, 30) ends on the cut: every frame is a head frame of B (instance held, camera running) and the weight reaches the last frame",
     "rejects": ["BOTH_CLOCKS_PINNED_TOGETHER", "INSTANCE_AND_CAMERA_CLOCKS_SWAPPED", "NEIGHBOUR_ON_PRIMARY_CLOCKS", "DECKS_SWAPPED_IN_THE_MIX"], "agrees": []},
    {"id": "pos-start-mixed", "kit": "pos", "layout": "mixed", "alignment": "start", "ks": [0, 1, 2, 3, 4, 7, 11],
     "role": "window [30, 42) starts on the cut: every frame is a tail frame of A (camera held, instance running)",
     "rejects": ["BOTH_CLOCKS_PINNED_TOGETHER", "INSTANCE_AND_CAMERA_CLOCKS_SWAPPED", "NEIGHBOUR_ON_PRIMARY_CLOCKS", "DECKS_SWAPPED_IN_THE_MIX"], "agrees": []},
    {"id": "pos-edge-start", "kit": "pos", "layout": "edge", "alignment": "start", "ks": [0, 1, 2, 3, 4, 5],
     "role": "the clamp limits inside the window: the last real frame, the first held frame, and the two clamps ending on different frames of the tail of A",
     "rejects": ["BOTH_CLOCKS_PINNED_TOGETHER", "INSTANCE_AND_CAMERA_CLOCKS_SWAPPED", "NEIGHBOUR_ON_PRIMARY_CLOCKS", "DECKS_SWAPPED_IN_THE_MIX"], "agrees": []},
    {"id": "pos-control-real", "kit": "pos", "layout": "real", "alignment": "center", "ks": [0, 2, 4, 5, 6, 7, 9, 11],
     "role": "control, nothing held: pinning the clocks together changes nothing, a swap or a wrong deck still does",
     "rejects": ["INSTANCE_AND_CAMERA_CLOCKS_SWAPPED", "NEIGHBOUR_ON_PRIMARY_CLOCKS", "DECKS_SWAPPED_IN_THE_MIX"], "agrees": ["BOTH_CLOCKS_PINNED_TOGETHER"]},
    {"id": "pos-control-held", "kit": "pos", "layout": "held", "alignment": "center", "ks": [0, 2, 4, 5, 6, 7, 9, 11],
     "role": "control, both clocks of the neighbour held together: the pinned-together mutant coincides with the truth",
     "rejects": ["NEIGHBOUR_ON_PRIMARY_CLOCKS", "DECKS_SWAPPED_IN_THE_MIX"], "agrees": ["BOTH_CLOCKS_PINNED_TOGETHER"]},
    {"id": "lit-center-held", "kit": "lit", "layout": "held-light", "alignment": "center", "ks": [0, 2, 4, 5, 6, 7, 9, 11],
     "role": "light on the real clock with the neighbour held on both sides: only the scene clock moves (static camera, static marker)",
     "rejects": LIGHT_HYPOTHESES + ["DECKS_SWAPPED_IN_THE_MIX"], "agrees": POSITION_HYPOTHESES},
    {"id": "lit-end-held", "kit": "lit", "layout": "held-light", "alignment": "end", "ks": [0, 2, 4, 5, 6, 7, 9, 11],
     "role": "window [18, 30): the neighbour is B before its start, held; the light follows the real edit frame",
     "rejects": LIGHT_HYPOTHESES + ["LIGHT_ON_THE_CLAMPED_EDIT_FRAME"], "agrees": POSITION_HYPOTHESES},
    {"id": "lit-start-held", "kit": "lit", "layout": "held-light", "alignment": "start", "ks": [0, 2, 4, 5, 6, 7, 9, 11],
     "role": "window [30, 42): the neighbour is A after its end, held; the light follows the real edit frame",
     "rejects": LIGHT_HYPOTHESES, "agrees": POSITION_HYPOTHESES},
    {"id": "lit-center-instance-running", "kit": "lit", "layout": "real", "alignment": "center", "ks": [0, 2, 4, 5, 6, 7, 9, 11],
     "role": "control: the instance RUNS with a start different from the shot's, yet the light still follows the edit clock, not the instance clock",
     "rejects": ["LIGHT_ON_THE_INSTANCE_CLOCK", "LIGHT_ON_THE_SHOT_RELATIVE_CLOCK"], "agrees": POSITION_HYPOTHESES},
    {"id": "lit-ntsc-center", "kit": "lit", "layout": "held-ntsc", "alignment": "center", "ks": [0, 2, 4, 5, 6, 7, 9, 11],
     "role": "rational 30000/1001 around frame 1800: f * 1001 / 30000, not a nominal 30 fps; the window sits at a late frame",
     "rejects": ["LIGHT_ON_A_NOMINAL_FRAME_RATE", "LIGHT_ON_THE_INSTANCE_CLOCK", "LIGHT_ON_THE_SHOT_RELATIVE_CLOCK"],
     "agrees": POSITION_HYPOTHESES},
    {"id": "both-center-mixed", "kit": "both", "layout": "mixed", "alignment": "center", "ks": [0, 2, 4, 5, 6, 7, 9, 11],
     "role": "everything at once: held instance with running camera (and the reverse) under a lamp that flickers on the real edit frame",
     "rejects": POSITION_HYPOTHESES + LIGHT_HYPOTHESES + ["DECKS_SWAPPED_IN_THE_MIX"], "agrees": []},
]


def scenario_by_id(scenario_id):
    return next(item for item in SCENARIOS if item["id"] == scenario_id)


def transition_for(scenario):
    return dict(TRANSITION, alignment=scenario["alignment"], handles=LAYOUTS[scenario["layout"]]["handles"])


def cut_of(edit):
    return edit["shots"]["A"]["end"]


def deck_states(edit, f, hypothesis=None):
    """Both decks of the window at edit frame `f`: A is the shot before the cut, B the shot after it. The primary one
    is the shot that contains `f`; the other is the neighbour."""
    primary = "A" if f < cut_of(edit) else "B"
    neighbour = "B" if primary == "A" else "A"
    return {
        "A": clocks_of(edit, "A", f, hypothesis, primary_id=primary if neighbour == "A" else None),
        "B": clocks_of(edit, "B", f, hypothesis, primary_id=primary if neighbour == "B" else None),
    }, primary, neighbour


def out_of(decks, states, x, y, transition, k, hypothesis=None):
    """Per-deck 8-bit codes at pixel (x, y) and the final code after the mix."""
    a_cover, a_raw, a8 = deck_pixel(decks["A"], states["A"], x, y)
    b_cover, b_raw, b8 = deck_pixel(decks["B"], states["B"], x, y)
    swapped = hypothesis is not None and hypothesis.decks_swapped
    first, second = (b8, a8) if swapped else (a8, b8)
    return {"a_cover": a_cover, "b_cover": b_cover, "a_raw": a_raw, "b_raw": b_raw, "a8": a8, "b8": b8, "out8": mix_codes(first, second, transition, k)}


def neighbourhood(decks, states, x, y):
    """Covering plane of each deck at the 3x3 samples +-FLAT_MARGIN_PX around (x, y), and the colour spread."""
    covers = {"A": set(), "B": set()}
    codes = {"A": [], "B": []}
    for dx in (-FLAT_MARGIN_PX, 0, FLAT_MARGIN_PX):
        for dy in (-FLAT_MARGIN_PX, 0, FLAT_MARGIN_PX):
            px, py = x + dx, y + dy
            if not (0 <= px < VIEWPORT[0] and 0 <= py < VIEWPORT[1]):
                return None, None  # a neighbourhood that leaves the viewport is not a flat area
            for deck in ("A", "B"):
                cover, _, code = deck_pixel(decks[deck], states[deck], px, py)
                covers[deck].add(cover)
                codes[deck].append(code)
    spread = 0
    for deck in ("A", "B"):
        for channel in range(3):
            values = [code[channel] for code in codes[deck]]
            spread = max(spread, max(values) - min(values))
    return covers, spread


def is_flat(decks, states, x, y):
    covers, spread = neighbourhood(decks, states, x, y)
    if covers is None:
        return False
    return all(len(items) == 1 for items in covers.values()) and spread <= FLAT_MAX_CODE_SPREAD


def probe_record(probe_id, role, point, decks, states, transition, k, hypotheses):
    """One probe: the pixel, what each deck draws there, the final code, and which hypotheses it REJECTS."""
    x, y = point
    truth = out_of(decks, states, x, y, transition, k)
    if not is_flat(decks, states, x, y):
        raise Refusal("ANALYTIC_PROBE_NOT_FLAT", "%s at (%d, %d): the neighbourhood is not one flat area in both decks" % (probe_id, x, y))
    return truth, {
        "id": probe_id, "pixel": "%d,%d" % (x, y),
        "deck_a": "%s %s" % (truth["a_cover"], hex8(truth["a8"])),
        "deck_b": "%s %s" % (truth["b_cover"], hex8(truth["b8"])),
        "expected": hex8(truth["out8"]),
    }


def hypothesis_outputs(edit, decks, f, point, transition, k, names):
    out = {}
    x, y = point
    for name in names:
        hypothesis = HYPOTHESES[name]["clock"]
        states, _, _ = deck_states(edit, f, hypothesis)
        out[name] = out_of(decks, states, x, y, transition, k, hypothesis)["out8"]
    return out


def clear_points(kit):
    """The fixed probes that do not depend on a marker: inside the backdrop, away from every marker position."""
    if kit == "lit":
        return [("backdrop_near_lamp_a", (160, 90)), ("backdrop_near_lamp_b", (500, 70)), ("backdrop_far", (60, 320))]
    return [("backdrop_clear", (48, 24))]


def marker_pixel(deck, state):
    geometry = marker_geometry(deck, state)
    return int(math.floor(geometry["_centre"][0])), int(math.floor(geometry["_centre"][1])), geometry


def build_frame(scenario, edit, decks, transition, f):
    """One frame of a scenario: states, marker geometry and the probes of the window, or the pure primary deck."""
    start, end = window_of(cut_of(edit), transition["duration"], transition["alignment"])
    states, primary, neighbour = deck_states(edit, f, None)
    in_window = start <= f < end
    kit = scenario["kit"]
    record = {
        "edit_frame": f,
        "in_window": in_window,
        "primary": primary,
        "neighbour": neighbour if in_window else None,
        "states": {deck: {key: (r9(value) if isinstance(value, float) else value) for key, value in state.items() if key not in ("shot", "edit_frame")}
                   for deck, state in states.items()},
        "lights": {deck: [{"id": row["id"], "flicker_level": r9(row["flicker_level"]), "effective_intensity": r9(row["effective_intensity"])}
                          for row in deck_lights(decks[deck], states[deck])] for deck in ("A", "B")} if KITS[kit]["lit"] else {"A": [], "B": []},
    }
    if KITS[kit]["moving"]:
        record["markers"] = {deck: {key: value for key, value in marker_geometry(decks[deck], states[deck]).items() if not key.startswith("_")} for deck in ("A", "B")}
    if not in_window:
        record["transition"] = None
        record["expect"] = "no transition: the frame is the primary deck alone (a dry cut), bit for bit"
        return record, []
    k = f - start
    progress, shaped, amount, weight = weight_for(k, transition)
    record["k"] = k
    record["transition"] = {"progress": r9(progress), "shaped": r9(shaped), "amount": r9(amount), "weight": r9(weight)}
    names = scenario["rejects"] + scenario["agrees"]
    points = []
    for deck in (primary, neighbour):
        x, y, _ = marker_pixel(decks[deck], states[deck])
        points.append(("marker_of_%s_%s" % ("primary" if deck == primary else "neighbour", deck), (x, y)))
    points += clear_points(kit)
    # decoys: where each position hypothesis would put the neighbour's marker centre (when it is far from the truth)
    decoys = []
    if KITS[kit]["moving"]:
        true_x, true_y, _ = marker_pixel(decks[neighbour], states[neighbour])
        for name in names:
            hypothesis = HYPOTHESES[name]["clock"]
            if hypothesis.light is not None or hypothesis.decks_swapped:
                continue
            hyp_states, _, _ = deck_states(edit, f, hypothesis)
            hx, hy, _ = marker_pixel(decks[neighbour], hyp_states[neighbour])
            if abs(hx - true_x) > MAX_PROBE_PIXEL_SHIFT * FLAT_MARGIN_PX or abs(hy - true_y) > MAX_PROBE_PIXEL_SHIFT * FLAT_MARGIN_PX:
                if (hx, hy) not in [p for _, p in points] and (hx, hy) not in [p for _, p in decoys]:
                    if is_flat(decks, states, hx, hy) and is_flat(decks, hyp_states, hx, hy):
                        decoys.append(("decoy_of_%s" % HYPOTHESES[name]["short"], (hx, hy)))
    probes, rejecting = [], {}
    for probe_id, point in points + decoys:
        truth, row = probe_record(probe_id, "marker" if probe_id.startswith("marker") else "decoy" if probe_id.startswith("decoy") else "backdrop", point,
                                  decks, states, transition, k, names)
        outputs = hypothesis_outputs(edit, decks, f, point, transition, k, names)
        rejects = []
        for name, code in outputs.items():
            separation = max(abs(code[i] - truth["out8"][i]) for i in range(3))
            if name in scenario["agrees"]:
                if separation > AGREE_MAX_CODES:
                    raise Refusal("ANALYTIC_CONTROL_DISAGREES", "%s: %s moves %s by %d codes but must agree" % (scenario["id"], name, probe_id, separation))
            elif separation >= MIN_SEPARATION_CODES:
                rejects.append(name)
                rejecting.setdefault(name, []).append(f)
        if rejects:
            row["rejects"] = ",".join(HYPOTHESES[name]["short"] for name in sorted(rejects))
        probes.append(row)
    record["probes"] = probes
    return record, [(name, frames) for name, frames in rejecting.items()]


def build_scenario(scenario):
    edit = build_edit(scenario["layout"])
    transition = transition_for(scenario)
    decks = {shot_id: build_deck(scenario["kit"], shot_id, scenario["layout"]) for shot_id in ("A", "B")}
    start, end = window_of(cut_of(edit), transition["duration"], transition["alignment"])
    frames = [start + k for k in scenario["ks"]]
    outside = [start - 1, end]
    reached = {}
    rows = []
    for f in sorted(set(frames + outside)):
        row, rejecting = build_frame(scenario, edit, decks, transition, f)
        rows.append(row)
        for name, hit_frames in rejecting:
            reached.setdefault(name, []).extend(hit_frames)
    for name in scenario["rejects"]:
        hits = reached.get(name, [])
        if len(hits) < MIN_REJECTING_PROBES or len(set(hits)) < MIN_REJECTING_FRAMES:
            raise Refusal("ANALYTIC_NO_REFUSAL_POWER", "%s: %s moves only %d probes on %d frames (needs %d on %d)"
                          % (scenario["id"], name, len(hits), len(set(hits)), MIN_REJECTING_PROBES, MIN_REJECTING_FRAMES))
    window = {"start": start, "end_exclusive": end}
    handles = transition["handles"]
    held_frames = [row["edit_frame"] for row in rows if row["in_window"] and (
        row["states"][row["neighbour"]]["held_instance"] or row["states"][row["neighbour"]]["held_camera"])]
    if handles == "use" and held_frames:
        raise Refusal("ANALYTIC_HANDLES_USE_NEEDS_REAL_FRAMES", "%s: handles use but the neighbour is held at %r" % (scenario["id"], held_frames))
    if handles == "hold" and not held_frames:
        raise Refusal("ANALYTIC_HANDLES_HOLD_WITHOUT_A_HOLD", "%s: handles hold but no neighbour frame is held" % scenario["id"])
    return {
        "id": scenario["id"], "kit": scenario["kit"], "layout": scenario["layout"], "role": scenario["role"],
        "fps": edit["fps"], "edit_duration_frames": edit["edit_duration"], "cut": cut_of(edit),
        "transition": dict(transition, window=window, handles=handles),
        "shots": {key: dict(value) for key, value in edit["shots"].items()},
        "instance_frames": dict(edit["instances"]), "camera_clip_frames": dict(edit["cameras"]),
        "neighbour_held_frames": held_frames,
        "must_reject": list(scenario["rejects"]), "must_agree": list(scenario["agrees"]),
        "rejecting_probes": {name: len(reached.get(name, [])) for name in scenario["rejects"]},
        "frames": rows,
    }


def kit_documents():
    """The scenes themselves, as data S2 can build: planes (flat 8-bit colours), event channel, lamps, camera clips."""
    out = {}
    for kit, spec in KITS.items():
        decks = {}
        for shot_id in ("A", "B"):
            deck = build_deck(kit, shot_id, "mixed")
            decks[shot_id] = {"planes": deck["planes"], "lights": deck["lights"], "camera_clip_of_layout_mixed": deck["camera"]}
        out[kit] = {"doc": spec["doc"], "decks": decks}
    return out


def conventions():
    return {
        "projection": "Three.js PerspectiveCamera(vertical fov %s deg, aspect = %d / %d), lookAt(target), up (0, 1, 0); viewport %dx%d" % (
            FOV_DEG, VIEWPORT[0], VIEWPORT[1], VIEWPORT[0], VIEWPORT[1]),
        "compact": "tuples are strings: colours \"#rrggbb\", pixels \"x,y\", points \"a,b[,c]\" and rectangles \"left,top,right,bottom\" (9 decimals at most), a probe's deck as \"<plane id or None> #rrggbb\"",
        "pixels": "the centre of pixel index i is i + 0.5; ndc_x = (x + 0.5) / W * 2 - 1, ndc_y = 1 - (y + 0.5) / H * 2; `centre_px` and `rect_px` are continuous (edge convention: pixel i covers [i, i + 1))",
        "planes": "image planes facing +z, no rotation or scale, flat opaque 8-bit sRGB colour, drawn in render_order (a painter's queue: the later one wins where both cover; no depth test decides)",
        "event_channel": "performance `event` on localPosition: position = base + delta * p, p = 0 before start, 1 from start + duration on, (f - start) / duration between",
        "camera": "truck/hold recipes from `camera/resolve-camera-clip.ts`: pose = lerp(start, end, ease(local / max(1, frames - 1))) for a recipe, a hold stays; past the last movement the final pose rests",
        "kits": "`pos` and `both`: a marker (0.6 square, z = 0) that bursts upward by 4.8 units over 12 instance frames from `start` (A: 30, B: 0), and a camera that trucks right at a constant %s units per clip frame (a layout changes the clip's LENGTH, never its speed); `lit` and `both`: one lamp per deck; `lit`: a static 1.0 marker and a held camera" % TRUCK_UNITS_PER_FRAME,
        "clocks": "delta = f - shot.start; instance = clamp(instanceStart + delta, 0, instanceFrames - 1); camera = clamp(cameraIn + delta, 0, cameraFrames - 1); each with its own held flag (D14/D15)",
        "scene_clock": "the light's seconds = f / (numerator / denominator) with the REAL edit frame f, never limited to the shot or the instance (D17; origin: `editFrameToSeconds(editFrame, fps)`)",
        "flicker": "level = 1 - flicker * 0.45 * (0.6 * noise(seconds * 3, seed) + 0.4 * noise(seconds * 13, seed + 101)); seed from the light id (`seedOf`); effective intensity = intensity * level",
        "lighting": "unlit plane: the 8-bit colour as it is. lit plane: encode(clamp(decode(base) * (1 + sum_i colour_i * effective_intensity_i * max(0, n . l) * falloff_i^2), 0, 1)), falloff = 1 / (1 + (d / radius)^2) applied twice, n facing the camera, ambient 1",
        "deck_codes": "each deck is quantized to 8-bit sRGB codes (half-up) BEFORE the mix; the mix is the corpus' (decode, mix plain/curved/amount, encode, half-up)",
        "alpha": "output alpha 255",
        "transition": dict(TRANSITION, note="the defaults of a new transition; the alignment varies by scenario"),
    }


def thresholds():
    return {
        "status": "pre-registered before any pixel was produced; never retuned after a result; A validar (design review pending); nothing measured",
        "flat_margin_px": FLAT_MARGIN_PX,
        "flat_max_code_spread": FLAT_MAX_CODE_SPREAD,
        "min_separation_codes": MIN_SEPARATION_CODES,
        "min_rejecting_probes": MIN_REJECTING_PROBES,
        "min_rejecting_frames": MIN_REJECTING_FRAMES,
        "agree_max_codes": AGREE_MAX_CODES,
        "tie_margin": TIE_MARGIN,
        "tolerance_for_the_implementation": TOLERANCE_REFERENCE,
        "rules": [
            "a probe lies where the 3x3 samples at +-flat_margin_px see ONE plane in each deck and the lit colour spreads by at most flat_max_code_spread codes",
            "a hypothesis REJECTS a probe when its expected code differs from the truth by at least min_separation_codes in a channel (the tolerance is 1)",
            "a scenario lists the hypotheses it must reject (>= min_rejecting_probes probes on >= min_rejecting_frames frames) and the ones that must agree (every probe within agree_max_codes): the generator REFUSES a scenario that does not",
            "a lit value above 1 (clamped by the runtime) or a value within tie_margin of a rounding tie is refused, not worked around",
            "a decoy probe is the pixel where a position hypothesis would put the neighbour's marker centre, kept only when it is far from the true one and flat in both worlds; the others are recorded in no file and never replaced by a nearby pixel",
        ],
    }


def hypothesis_table():
    return {name: {"short": item["short"], "wrong_rule": item["wrong_rule"], "plan": item["plan"]} for name, item in HYPOTHESES.items()}


def build_section():
    """The `analytic_probes` section of dissolve-cases.json (deterministic, UTF-8/LF, no host or date)."""
    scenarios = [build_scenario(item) for item in SCENARIOS]
    return {
        "status": "analytic X for mixed holds and the real light clock (P2-2); origin-formula; S0 contract + oracle only: no runtime, Rust, JSONL, UI or engine surface exists; "
                  "textured frames only become real in S2; the public gate FX_TRANSITION_NOT_YET stays closed until S2A; nothing rendered or measured; A validar",
        "kind": "origin-formula",
        "plan_sections": ["7.3 (referência independente X)", "D14", "D15", "D17", "T06", "T10", "T16", "T27", "T29"],
        "independence": "no product transition evaluator, no pure runtime frame fabricated for a mixed hold, no Rust and no GLSL: the kernels are transcribed from the origin text and share nothing with the corpus script",
        "conventions": conventions(),
        "thresholds": thresholds(),
        "hypotheses": hypothesis_table(),
        "kits": kit_documents(),
        "scenarios": scenarios,
    }


def summary(section):
    rows = []
    for scenario in section["scenarios"]:
        probes = sum(len(frame.get("probes", [])) for frame in scenario["frames"])
        rows.append("%-30s %2d frames %3d probes  rejects %s" % (scenario["id"], len(scenario["frames"]), probes, scenario["rejecting_probes"]))
    return "\n".join(rows)


if __name__ == "__main__":
    print(summary(build_section()))
