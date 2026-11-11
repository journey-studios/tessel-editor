#!/usr/bin/env python3
"""Pilot film-grain and keys corpus: pinned vectors with the provenance of every expectation (P3).

BL082.1 (the third look, `filmGrain`), BL082.2 (keys, easing, seed), BL036/BL018/BL019,
`docs/migration/p3-animated-fx-film-grain-plan.md`. The vignette's and the fade's twins are
`scripts/pilot-fx-corpus.py` and `scripts/pilot-fade-corpus.py`; this one is standalone and shares no
routine with them, with the Rust `directed_scene::fx` module or with the runtime's GLSL: it is the
independent reference the two are compared with.

Writes (`generate`) and checks (`verify`), in `docs/migration/fixtures/pilot-fx/`:

  grain-cases.json     the vectors: the integer hashes (FNV-1a over UTF-8, lowbias32, `seedFor`,
                       `hashPixel`, the triangular draw and its colored channels, as exact uint32 and
                       float32 bit patterns), the keys resolver (holds, a key's own value, the easing
                       of the arrival key, numbers/colours/points/booleans) and the pixels of flat
                       pictures at chosen probes (the cell, its hash and the expected bytes).
                       Consumed by `crates/core/tests/pilot_grain_corpus.rs` and by the browser test
                       `runtime/test/directed-scene-grain-browser.test.mjs`.
  grain-manifest.json  the provenance: origin snapshot, blob id + sha256 of every origin source an
                       expectation was read from, the sha256 of the corpus, the documented
                       differences and the blockers.

HOW THE EXPECTATIONS WERE MADE: by transcribing the pilot's sources (`git show <snapshot>:<path>`, read
only) into this script and evaluating them with the arithmetic the shader uses: `uint32` for the
hashes, IEEE `float32` (emulated through `struct`) for the noise and the cell division, `float64` for
the rest. THE ORIGIN IS NEVER INSTALLED, BUILT OR EXECUTED: the expectations are `origin-formula`, not
the outputs of the origin running. The shader lines and the constants a transcription could get wrong
are checked against the origin text at generation time, and `verify --origin` re-checks every blob.
The origin's golden (`filmGrain.png`) needs its harness (an engine build and a browser): a declared
blocker, and no golden is invented or compared.

    scripts/pilot-grain-corpus.py generate [--origin PATH] [--snapshot SHA]
    scripts/pilot-grain-corpus.py verify   [--origin PATH]       # offline when --origin is absent

Python standard library only. Exit 0 on success, 1 on a finding.
"""

import argparse
import hashlib
import json
import math
import struct
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CORPUS = ROOT / "docs/migration/fixtures/pilot-fx"
DEFAULT_ORIGIN = Path.home() / "www/journeystudios/tesselapp"
DEFAULT_SNAPSHOT = "d40ac3f7c0"
SCHEMA = 1
CASES_NAME = "grain-cases.json"
MANIFEST_NAME = "grain-manifest.json"

SE = "packages/scene-engine/src/"
CATALOG = SE + "fx/catalog/filmGrain.ts"
SHADER = SE + "runtime/fx/shaders/filmGrain.ts"
HASH_CHUNK = SE + "runtime/fx/shaders/chunks/hash.ts"
COLOR_CHUNK = SE + "runtime/fx/shaders/chunks/color.ts"
FX_HASH = SE + "fx/fx-hash.ts"
HASH_SPEC = SE + "fx/resolve-fx-frame-hashes.spec.ts"
CLIP_SAMPLE = SE + "fx/fx-clip-sample.ts"
RESOLVE = SE + "keyframes/resolve-keyframes.ts"
RULES = SE + "keyframes/keyframe-rules.ts"
FX_KEYS = SE + "fx/fx-keys.ts"
EASING = SE + "camera/easing.ts"
COMPOSITOR = SE + "runtime/fx/fx-compositor.ts"
META = SE + "fx/catalog/fx-effect-meta.ts"
SOURCES = [
    (CATALOG, "catalog: params, ranges, defaults, labels, reseed per-frame"),
    (SHADER, "shader: the grain formula, the cell, the colored channels, the luma response"),
    (HASH_CHUNK, "GLSL hashPixel, unitFloat, triangular and the hash constants"),
    (COLOR_CHUNK, "sRGB <-> linear curves and the Rec.709 luma"),
    (FX_HASH, "fnv1a32 over UTF-8 bytes, lowbias32"),
    (HASH_SPEC, "the pilot's own vectors for fnv1a32 and lowbias32"),
    (CLIP_SAMPLE, "seedFor, mixParam, resolveParams, envelopeWeight"),
    (RESOLVE, "resolveKeyframes: holds, a key's own value, arrival easing"),
    (RULES, "keyframeIssues: order, whole frames, easings; withKeyAt keeps an easing"),
    (FX_KEYS, "fxKeyIssues: keys inside the clip"),
    (EASING, "applyEasing and the three easings"),
    (COMPOSITOR, "REFERENCE_HEIGHT 720, pxScale = height / 720, amount = clamp01(intensity x weight)"),
    (META, "intensity is appended last, default 1, range 0..1"),
    (SE + "fx/catalog/__golden__/filmGrain.png", "GOLDEN (blocked: not comparable)"),
]

M32 = 0xFFFFFFFF
GRAIN_DEFAULTS = {"amount": 0.25, "size": 1.4, "luma": 0.7, "colored": False, "intensity": 1.0}
GRAIN_ORDER = ["amount", "size", "luma", "colored", "intensity"]
REFERENCE_HEIGHT = 720


def check_against(texts):
    """The numbers and lines transcribed here must be the origin's. Returns problems."""
    problems = []

    def need(path, lines):
        text = texts.get(path, "")
        for line in lines:
            if line not in text:
                problems.append("%s: not found in the origin text: %s" % (path.rsplit("/", 1)[-1], line))

    need(CATALOG, [
        'id: "filmGrain"',
        "typicalDurationFrames: 96,",
        'id: "amount",\n      kind: "number",\n      min: 0,\n      max: 1,\n      default: 0.25,\n      step: 0.01,',
        'id: "size",\n      kind: "number",\n      min: 0.5,\n      max: 4,\n      default: 1.4,\n      step: 0.1,\n      unit: "px",',
        'id: "luma",\n      kind: "number",\n      min: 0,\n      max: 1,\n      default: 0.7,\n      step: 0.01,',
        '{ id: "colored", kind: "boolean", default: false },',
        'reseed: "per-frame",',
        'needs: ["seed"],',
    ])
    need(SHADER, [
        "uCellPx: Math.max(numberParam(params, \"size\") * pxScale, 1),",
        "uvec2 cell = uvec2(floor(gl_FragCoord.xy / max(uCellPx, 1.0)));",
        "uint h = hashPixel(uSeed, cell);",
        "float mono = triangular(h);",
        "triangular(lowbias32(h + 0x632be5abu)),",
        "triangular(lowbias32(h + 0x85157af5u))",
        "if (uGrain * uAmount <= 0.0) {",
        "vec3 encoded = linearToSrgb(c);",
        "float response = mix(1.0, 1.0 - smoothstep(0.05, 0.95, luma(encoded)), uLuma);",
        "vec3 grained = clamp(encoded + noise * (uGrain * 0.25 * response), 0.0, 1.0);",
        "gl_FragColor = vec4(mix(c, srgbToLinear(grained), uAmount), 1.0);",
        "uColored: flag(booleanParam(params, \"colored\")),",
    ])
    need(HASH_CHUNK, [
        "return lowbias32(lowbias32(seed ^ 0x9e3779b9u) + pixel.x + (pixel.y << 16u));",
        "return float(hash >> 8u) / 16777216.0;",
        "return unitFloat(hash) + unitFloat(lowbias32(hash ^ 0x68bc21ebu)) - 1.0;",
        "x *= 0x7feb352du;",
        "x *= 0x846ca68bu;",
    ])
    need(COLOR_CHUNK, [
        "return dot(c, vec3(0.2126, 0.7152, 0.0722));",
        "vec3 high = pow((max(c, 0.0) + 0.055) / 1.055, vec3(2.4));",
        "vec3 high = 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055;",
    ])
    need(FX_HASH, ["let hash = 0x811c9dc5", "hash = Math.imul(hash, 0x01000193)", "encoder.encode(text)"])
    need(CLIP_SAMPLE, [
        "const base = owner.seed ?? fnv1a32(owner.id)",
        'if (meta.reseed === "per-frame") {',
        "return lowbias32((base ^ frameInClip) >>> 0)",
        "return lowbias32(base)",
        "return from\n}",
    ])
    need(RESOLVE, [
        "if (frame <= first.frame) {",
        "const arriving = keys.findIndex((key) => frame <= key.frame)",
        "if (to && frame === to.frame) {",
        "(frame - from.frame) / (to.frame - from.frame),",
        "export const mixNumber: Mix<number> = (from, to, t) => from + (to - from) * t",
    ])
    need(RULES, [
        "key.frame <= previous",
        '"linear",\n): Keyframe<T>[] => {',
        "return { ...key, value }",
    ])
    need(FX_KEYS, ["if (key.frame >= 0 && key.frame < durationFrames) {", 'code: "KEY_OUT_OF_CLIP"'])
    need(EASING, [
        "return 4 * t * t * t",
        "return 1 - (p * p * p) / 2",
        "return 1 - p * p * p",
        '"linear",\n  "easeInOutCubic",\n  "easeOutCubic",',
    ])
    need(COMPOSITOR, ["const REFERENCE_HEIGHT = 720", "pxScale: this.options.height / REFERENCE_HEIGHT,"])
    return problems


# ---- the arithmetic of the shader: uint32, float32 ----------------------------------------------


def fnv1a32(text):
    value = 0x811C9DC5
    for byte in text.encode("utf-8"):
        value = ((value ^ byte) * 0x01000193) & M32
    return value


def lowbias32(value):
    h = value & M32
    h ^= h >> 16
    h = (h * 0x7FEB352D) & M32
    h ^= h >> 15
    h = (h * 0x846CA68B) & M32
    h ^= h >> 16
    return h


def hash_pixel(seed, x, y):
    return lowbias32((lowbias32((seed ^ 0x9E3779B9) & M32) + x + ((y << 16) & M32)) & M32)


def f32(value):
    """The nearest IEEE float32 (round to nearest even), as a Python float."""
    return struct.unpack("<f", struct.pack("<f", value))[0]


def f32_bits(value):
    return struct.unpack("<I", struct.pack("<f", value))[0]


def unit_float(h):
    return (h >> 8) / 16777216.0  # a 24-bit integer over 2^24: exact in float32 and float64


def triangular(h):
    # u1 + u2 is exact in float64, then rounded once to float32; `- 1.0` is exact after that.
    return f32(unit_float(h) + unit_float(lowbias32(h ^ 0x68BC21EB))) - 1.0


def colored(h):
    return [
        triangular(h),
        triangular(lowbias32((h + 0x632BE5AB) & M32)),
        triangular(lowbias32((h + 0x85157AF5) & M32)),
    ]


def seed_for(reseed, base, frame_in_clip):
    if reseed == "per-frame":
        return lowbias32((base ^ (frame_in_clip & M32)) & M32)
    return lowbias32(base)


# ---- the keys resolver --------------------------------------------------------------------------


def apply_easing(t, name):
    t = 0.0 if t <= 0 else 1.0 if t >= 1 else t
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
    raise ValueError("Unknown easing: " + name)


def mix_number(a, b, t):
    return a + (b - a) * t


def mix_param(a, b, t):
    if isinstance(a, bool) or isinstance(b, bool):
        return a  # a switch changes ON the next key: the previous value holds until then
    if isinstance(a, (int, float)):
        return mix_number(a, b, t)
    if "r" in a:
        return {c: mix_number(a[c], b[c], t) for c in "rgb"}
    return {c: mix_number(a[c], b[c], t) for c in "xy"}


def resolve_keyframes(keys, frame):
    first = keys[0]
    if frame <= first["frame"]:
        return first["value"]
    arriving = next((i for i, key in enumerate(keys) if frame <= key["frame"]), -1)
    to = keys[arriving] if arriving >= 0 else None
    if to is not None and frame == to["frame"]:
        return to["value"]
    if arriving > 0 and to is not None:
        frm = keys[arriving - 1]
        t = apply_easing((frame - frm["frame"]) / (to["frame"] - frm["frame"]), to.get("easing", "linear"))
        return mix_param(frm["value"], to["value"], t)
    return keys[-1]["value"]


# ---- the grain pixel ----------------------------------------------------------------------------


def srgb_to_linear(c):
    return c / 12.92 if c <= 0.04045 else ((max(c, 0.0) + 0.055) / 1.055) ** 2.4


def linear_to_srgb(c):
    c = max(0.0, min(1.0, c))
    return c * 12.92 if c < 0.0031308 else 1.055 * c ** (1.0 / 2.4) - 0.055


def smoothstep(edge0, edge1, x):
    t = max(0.0, min(1.0, (x - edge0) / (edge1 - edge0)))
    return t * t * (3.0 - 2.0 * t)


def cell_px(size, height):
    """`max(size x pxScale, 1)` in double precision, `pxScale = height / 720` (applied once)."""
    return max(size * (height / REFERENCE_HEIGHT), 1.0)


def cell_of(x, y, height, size):
    """The grain cell of the pixel at column `x`, row `y` counted from the TOP: `gl_FragCoord` counts
    from the bottom, so the row is flipped; the division is float32 like the GPU's."""
    cell = f32(cell_px(size, height))
    gl_y = height - 1 - y
    return (
        int(math.floor(f32(f32(x + 0.5) / cell))),
        int(math.floor(f32(f32(gl_y + 0.5) / cell))),
    )


def grain_pixel(rgb8, x, y, height, params, pass_amount, seed):
    """The pilot's shader line on an sRGB-encoded 8-bit pixel; `pass_amount` is intensity x weight."""
    grain = params["amount"]
    if grain * pass_amount <= 0.0:
        return [float(channel) for channel in rgb8], None
    cx, cy = cell_of(x, y, height, params["size"])
    h = hash_pixel(seed, cx, cy)
    noise = colored(h) if params["colored"] else [triangular(h)] * 3
    encoded = [channel / 255.0 for channel in rgb8]
    luma = 0.2126 * encoded[0] + 0.7152 * encoded[1] + 0.0722 * encoded[2]
    response = 1.0 + ((1.0 - smoothstep(0.05, 0.95, luma)) - 1.0) * params["luma"]
    out = []
    for channel in range(3):
        grained = max(0.0, min(1.0, encoded[channel] + noise[channel] * (grain * 0.25 * response)))
        linear = srgb_to_linear(encoded[channel])
        mixed = linear + (srgb_to_linear(grained) - linear) * pass_amount
        out.append(linear_to_srgb(mixed) * 255.0)
    return out, {"cell": [cx, cy], "hash": h, "noise_bits": [f32_bits(f32(v)) for v in noise]}


# ---- the cases ----------------------------------------------------------------------------------


def hash_cases():
    lcg = 0x2545F491
    triples = []
    for n in range(48):
        lcg = (lcg * 1664525 + 1013904223) & M32
        seed = lcg
        lcg = (lcg * 1664525 + 1013904223) & M32
        x = lcg % 4096 if n % 3 else lcg % 70000
        lcg = (lcg * 1664525 + 1013904223) & M32
        y = lcg % 4096 if n % 4 else lcg % 70000
        triples.append((seed, x, y))
    triples += [(0, 0, 0), (M32, M32, M32), (1, 0, 65535), (1, 65535, 0), (7, 1, 65536), (123456789, 3, 5)]
    texts = ["", "a", "foobar", "viñeta", "fx-01", "transition-01", "grão-ñ-日本-🎞", "a-much-longer-clip-id-with-numbers-123456"]
    lows = [0, 1, 2, 42, 12345, 0xDEADBEEF, M32, 0x80000000, 0x7FFFFFFF]
    hashes = sorted({hash_pixel(*t) for t in triples})[:24] + [0, 1, 0xFFFFFFFF, 0x80000000]
    seeds = []
    for reseed in ("per-frame", "static"):
        for base in (0, 7, fnv1a32("grain-1"), 0xDEADBEEF, M32):
            for frame in (0, 1, 7, 95, 2**31, 2**32 + 5, 2**32 - 1):
                seeds.append({"reseed": reseed, "base": base, "frame": frame, "seed": seed_for(reseed, base, frame)})
    return {
        "fnv1a32": [{"text": text, "hash": fnv1a32(text)} for text in texts],
        "lowbias32": [{"in": value, "out": lowbias32(value)} for value in lows],
        "seed_for": seeds,
        "hash_pixel": [{"seed": s, "x": x, "y": y, "hash": hash_pixel(s, x, y)} for s, x, y in triples],
        "triangular": [{"hash": h, "bits": f32_bits(f32(triangular(h))), "value": triangular(h)} for h in hashes],
        "colored": [{"hash": h, "bits": [f32_bits(f32(v)) for v in colored(h)]} for h in hashes[:8]],
    }


def key(frame, value, easing=None):
    out = {"frame": frame, "value": value}
    if easing is not None:
        out["easing"] = easing
    return out


def key_cases():
    white = {"r": 1.0, "g": 1.0, "b": 1.0}
    black = {"r": 0.0, "g": 0.0, "b": 0.0}
    cases = [
        ("number-three-easings", "amount", [key(4, 0.3), key(14, 0.9, "easeOutCubic"), key(30, 0.2, "easeInOutCubic"), key(39, 1.0, "linear")]),
        ("number-first-key-easing-is-unused", "size", [key(2, 1.0, "easeOutCubic"), key(12, 3.0)]),
        ("number-single-key-holds-everywhere", "luma", [key(5, 0.42)]),
        ("number-unrepresentable-key-values", "amount", [key(0, 0.1), key(10, 0.3), key(20, 0.7)]),
        ("boolean-steps-on-the-next-key", "colored", [key(0, False), key(10, True), key(25, False, "easeOutCubic")]),
        ("colour-channels-mix-linearly", "color", [key(0, black), key(10, white, "easeInOutCubic")]),
        ("point-parts-mix-linearly", "center", [key(0, {"x": 0.0, "y": 1.0}), key(8, {"x": 1.0, "y": 0.0}, "easeOutCubic")]),
    ]
    out = []
    for name, param, keys in cases:
        frames = sorted({0, 1, 3, 9, 11, 13, 24, 26, 38, 39, 100, -5} | {k["frame"] for k in keys} | {k["frame"] + 1 for k in keys} | {k["frame"] - 1 for k in keys})
        exact = sorted({k["frame"] for k in keys})
        out.append(
            {
                "id": name,
                "param": param,
                "keys": keys,
                "queries": [{"frame": f, "value": resolve_keyframes(keys, f), "exact": f in exact or f <= keys[0]["frame"] or f >= keys[-1]["frame"]} for f in frames],
            }
        )
    return out


def flat_case(case_id, title, width, height, params, pass_amount, seed, source, probes, alpha=255):
    items = []
    for x, y in probes:
        expected, extra = grain_pixel(source, x, y, height, params, pass_amount, seed)
        item = {"x": x, "y": y, "expected": [round(v, 9) for v in expected]}
        if extra is not None:
            item.update(extra)
        items.append(item)
    return {
        "id": case_id,
        "title": title,
        "width": width,
        "height": height,
        "params": params,
        "pass_amount": pass_amount,
        "seed": seed,
        "source_rgb": source,
        "source_alpha": alpha,
        "tolerance_per_channel": 1.5,
        "identity": all(item["expected"] == [float(c) for c in source] for item in items),
        "probes": items,
    }


def grid(xs, ys):
    return [(x, y) for y in ys for x in xs]


def pixel_cases():
    grain = dict(GRAIN_DEFAULTS, amount=1.0)
    grey = [128, 128, 128]
    return [
        flat_case(
            "grain-mono-fractional-cell", "720 rows, size 1.4: a 1.4 px cell, so cell borders fall between pixels",
            48, 720, grain, 1.0, 123456789, grey,
            grid([0, 1, 2, 3, 4, 5, 33, 47], [0, 1, 2, 3, 358, 359, 360, 716, 717, 718, 719]),
        ),
        flat_case(
            "grain-colored-channels", "the three channels draw three different triangular noises",
            48, 720, dict(grain, colored=True), 1.0, 987654321, grey,
            grid([0, 1, 7, 8, 47], [0, 1, 360, 719]),
        ),
        flat_case(
            "grain-scale-applies-once-on-a-tall-picture", "1440 rows, size 2: the cell is 4 px (2 x 1440 / 720), not 8 and not 2",
            32, 1440, dict(grain, size=2.0), 1.0, 4242, grey,
            grid([0, 1, 3, 4, 5, 7, 8, 9], [0, 1, 3, 4, 5, 1435, 1436, 1439]),
        ),
        flat_case(
            "grain-small-picture-clamps-the-cell-to-one-pixel", "90 rows, size 1.4: 0.175 px is clamped to a whole pixel",
            40, 90, grain, 1.0, 5, grey,
            grid([0, 1, 2, 39], [0, 1, 2, 88, 89]),
        ),
        flat_case(
            "grain-luma-weighs-the-shadows", "luma 1: the noise weighs more in the shadows than in the highlights",
            24, 720, dict(grain, luma=1.0), 1.0, 99, [20, 20, 20],
            grid([0, 1, 2, 3], [0, 1, 2, 3]),
        ),
        flat_case(
            "grain-luma-highlights", "luma 1 on a bright picture: almost no grain",
            24, 720, dict(grain, luma=1.0), 1.0, 99, [245, 240, 235],
            grid([0, 1, 2, 3], [0, 1, 2, 3]),
        ),
        flat_case(
            "grain-luma-zero-is-uniform", "luma 0: the response is 1 whatever the picture",
            24, 720, dict(grain, luma=0.0), 1.0, 99, [20, 20, 20],
            grid([0, 1, 2, 3], [0, 1, 2, 3]),
        ),
        flat_case(
            "grain-half-pass-amount-mixes-in-linear-light", "intensity x weight 0.5: the mix is in linear light, once",
            24, 720, grain, 0.5, 31337, [200, 160, 80],
            grid([0, 1, 2, 3], [0, 1, 2, 3]),
        ),
        flat_case(
            "grain-seed-uint32-edges", "the largest seed is a seed",
            24, 720, grain, 1.0, M32, grey,
            grid([0, 1, 2, 3], [0, 1, 2, 3]),
        ),
        flat_case(
            "grain-seed-zero", "seed 0 is a seed",
            24, 720, grain, 1.0, 0, grey,
            grid([0, 1, 2, 3], [0, 1, 2, 3]),
        ),
        flat_case(
            "grain-amount-zero-is-the-baseline", "amount 0: the exact input bytes",
            24, 720, dict(grain, amount=0.0), 1.0, 7, [200, 160, 80],
            grid([0, 1, 2, 3], [0, 1, 2, 3]),
        ),
        flat_case(
            "grain-pass-amount-zero-is-the-baseline", "intensity x weight 0: the exact input bytes",
            24, 720, grain, 0.0, 7, [200, 160, 80],
            grid([0, 1, 2, 3], [0, 1, 2, 3]),
        ),
        flat_case(
            "grain-alpha-is-preserved", "the editor keeps the alpha of the picture (the pilot writes 1)",
            24, 720, grain, 1.0, 11, grey,
            grid([0, 1], [0, 1]), alpha=128,
        ),
    ]


def build_corpus():
    return {
        "schema": SCHEMA,
        "note": "Generated by scripts/pilot-grain-corpus.py; edit nothing by hand (verify fails).",
        "grain_defaults": GRAIN_DEFAULTS,
        "grain_param_order": GRAIN_ORDER,
        "reference_height": REFERENCE_HEIGHT,
        "hashes": hash_cases(),
        "keys": key_cases(),
        "pixels": pixel_cases(),
    }


# ---- provenance ---------------------------------------------------------------------------------


def sha256_of(data):
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    return (json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n").encode()


def git(origin, *args):
    return subprocess.run(["git", "-C", str(origin), *args], check=True, capture_output=True).stdout


def read_origin(origin, snapshot):
    sources, texts = [], {}
    for path, role in SOURCES:
        blob = git(origin, "rev-parse", "%s:%s" % (snapshot, path)).decode().strip()
        data = git(origin, "cat-file", "blob", blob)
        sources.append({"path": path, "git_blob": blob, "sha256": sha256_of(data), "bytes": len(data), "role": role})
        if not path.endswith(".png"):
            texts[path] = data.decode("utf-8")
    return sources, texts


def build_manifest(corpus_bytes, sources, snapshot, full_snapshot):
    return {
        "schema": SCHEMA,
        "purpose": "Provenance of the pilot film-grain and keys corpus (grain-cases.json): where each expectation comes from.",
        "origin": {
            "repository": "journeystudios/tesselapp (read only; never installed, built or executed)",
            "snapshot": snapshot,
            "snapshot_commit": full_snapshot,
            "access": "git show / cat-file of the blobs below",
        },
        "corpus": {
            "file": CASES_NAME,
            "sha256": sha256_of(corpus_bytes),
            "generator": "scripts/pilot-grain-corpus.py",
            "regenerate": "scripts/pilot-grain-corpus.py generate --origin <path to the origin repo>",
            "counts": {
                "fnv1a32": len(hash_cases()["fnv1a32"]),
                "lowbias32": len(hash_cases()["lowbias32"]),
                "seed_for": len(hash_cases()["seed_for"]),
                "hash_pixel": len(hash_cases()["hash_pixel"]),
                "key_curves": len(key_cases()),
                "pixel_cases": len(pixel_cases()),
            },
        },
        "expectation_kinds": {
            "origin-formula": "read from the origin source and evaluated with its arithmetic (uint32, float32, float64) by the generator; the origin was not run",
            "origin-spec-vector": "an integer the origin's own spec pins (`resolve-fx-frame-hashes.spec.ts`), repeated in `hash_vectors_of_the_origin_spec`",
        },
        "hash_vectors_of_the_origin_spec": {
            "fnv1a32": {"": 0x811C9DC5, "a": 0xE40C292C, "foobar": 0xBF9CF968, "viñeta": 419953240, "fx-01": 3286700573, "transition-01": 1697369554},
            "lowbias32": {"0": 0, "1": 1753845952, "2": 3507691905, "42": 388445122, "12345": 2435775735, "3735928559": 3861431939, "4294967295": 1734902346},
        },
        "pixel_tolerance": {
            "per_channel": 1.5,
            "why": "the expectation is the unrounded float; the runtime reads 8-bit bytes and its pow may differ in the last bit of a half float. The noise itself is compared as exact integers (cell, hash, float32 bits), where the tolerance is zero.",
        },
        "documented_differences": [
            {
                "id": "ENCODED_SOURCE",
                "detail": "The pilot decodes its decks to linear light and re-encodes inside the pass (`linearToSrgb(c)`); the editor's pictures are already sRGB-encoded bytes, so the grain is added to the encoded source directly. The same maths on the same colours.",
            },
            {
                "id": "ALPHA_PRESERVED",
                "detail": "The pilot writes alpha 1.0; the editor keeps the alpha of the picture it draws over. The cases pin it (grain-alpha-is-preserved).",
            },
            {
                "id": "ZERO_IS_BYTE_IDENTITY",
                "detail": "`uGrain x uAmount <= 0` returns the source exactly in the pilot too (its output alpha aside); the identity cases pin the exact bytes.",
            },
            {
                "id": "EASING_UNWRITTEN_DEFAULT",
                "detail": "The pilot's `withKeyAt` writes `easing: linear` on a new key; the editor leaves an easing nobody chose unwritten (it reads as linear) and an upsert that omits one keeps the key's own. The sampled values are the same.",
            },
            {
                "id": "NEGATIVE_KEY_FRAME_CODE",
                "detail": "A key at a negative frame is reported as KEY_OUT_OF_CLIP only (the pilot also reports KEYFRAME_ORDER); a fractional frame is INVALID_VALUE (the importer's own whole-number rule).",
            },
            {
                "id": "RESIZE_REFUSED_NOT_DROPPED",
                "detail": "A clip shortened under one of its keys is refused atomically with KEY_OUT_OF_CLIP; the pilot's Studio drops the key with a note. The editor never discards data by a resize.",
            },
        ],
        "sources": sources,
        "blockers": [
            {
                "id": "ORIGIN_GOLDEN_NOT_COMPARABLE",
                "detail": "The origin golden (catalog/__golden__/filmGrain.png) needs its harness: a build of the engine and a browser, i.e. executing the origin. Forbidden in this delivery. No golden was invented and none is compared.",
                "consequence": "Pixel equivalence with the origin is `origin-formula`, never golden equivalence. Visual parity of the grain stays partial.",
            },
            {
                "id": "ORIGIN_NOT_EXECUTED",
                "detail": "Expectations are transcribed formulas, not the origin's outputs. Mitigation: constants and formula lines are checked against the origin text at generation and by `verify --origin`.",
                "consequence": "A future execution of the origin (in its own, authorised environment) may replace `origin-formula` by `origin-output` per case.",
            },
        ],
    }


def paths():
    return CORPUS / CASES_NAME, CORPUS / MANIFEST_NAME


def spec_vector_findings(manifest):
    """The vectors the origin's own spec pins must be what this script computes."""
    findings = []
    for text, expected in manifest["hash_vectors_of_the_origin_spec"]["fnv1a32"].items():
        if fnv1a32(text) != expected:
            findings.append("fnv1a32(%r) is not the origin spec's %d" % (text, expected))
    for value, expected in manifest["hash_vectors_of_the_origin_spec"]["lowbias32"].items():
        if lowbias32(int(value)) != expected:
            findings.append("lowbias32(%s) is not the origin spec's %d" % (value, expected))
    return findings


def generate(origin, snapshot):
    sources, texts = read_origin(origin, snapshot)
    problems = check_against(texts)
    if problems:
        print("The transcription does not match the origin:", file=sys.stderr)
        for problem in problems:
            print("  - " + problem, file=sys.stderr)
        return 1
    full = git(origin, "rev-parse", snapshot + "^{commit}").decode().strip()
    corpus_bytes = canonical(build_corpus())
    manifest = build_manifest(corpus_bytes, sources, snapshot, full)
    findings = spec_vector_findings(manifest)
    if findings:
        for finding in findings:
            print("FINDING: " + finding, file=sys.stderr)
        return 1
    cases_path, manifest_path = paths()
    CORPUS.mkdir(parents=True, exist_ok=True)
    cases_path.write_bytes(corpus_bytes)
    manifest_path.write_bytes(canonical(manifest))
    print("wrote %s and %s" % (cases_path.relative_to(ROOT), manifest_path.relative_to(ROOT)))
    return 0


def verify(origin):
    cases_path, manifest_path = paths()
    findings = []
    if not cases_path.exists() or not manifest_path.exists():
        print("missing corpus or manifest: run `generate`", file=sys.stderr)
        return 1
    corpus_bytes = cases_path.read_bytes()
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if sha256_of(corpus_bytes) != manifest["corpus"]["sha256"]:
        findings.append("grain-cases.json was edited: its sha256 is not the manifest's")
    if corpus_bytes != canonical(build_corpus()):
        findings.append("grain-cases.json is not what the generator produces today (formulas or cases changed)")
    findings += spec_vector_findings(manifest)
    recorded = {source["path"] for source in manifest["sources"]}
    for path, _ in SOURCES:
        if path not in recorded:
            findings.append("source not in the manifest: " + path)
    for blocker in ("ORIGIN_GOLDEN_NOT_COMPARABLE", "ORIGIN_NOT_EXECUTED"):
        if blocker not in {b["id"] for b in manifest["blockers"]}:
            findings.append("blocker dropped from the manifest: " + blocker)
    if origin is not None:
        try:
            sources, texts = read_origin(origin, manifest["origin"]["snapshot"])
        except subprocess.CalledProcessError as error:
            print("origin objects are missing (equivalence blocker): %s" % error, file=sys.stderr)
            return 1
        before = {s["path"]: s for s in manifest["sources"]}
        for source in sources:
            old = before.get(source["path"])
            if old is None or (old["git_blob"], old["sha256"]) != (source["git_blob"], source["sha256"]):
                findings.append("origin blob changed or unrecorded: " + source["path"])
        findings += check_against(texts)
        mode = "against the origin blobs"
    else:
        mode = "offline (origin blobs not re-read; pass --origin to re-check them)"
    if findings:
        for finding in findings:
            print("FINDING: " + finding, file=sys.stderr)
        return 1
    print("pilot-grain corpus verified %s: %d sources" % (mode, len(manifest["sources"])))
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    for name in ("generate", "verify"):
        command = sub.add_parser(name)
        command.add_argument("--origin", type=Path, default=None)
        if name == "generate":
            command.add_argument("--snapshot", default=DEFAULT_SNAPSHOT)
    args = parser.parse_args()
    if args.command == "generate":
        return generate(args.origin or DEFAULT_ORIGIN, args.snapshot)
    return verify(args.origin)


if __name__ == "__main__":
    sys.exit(main())
