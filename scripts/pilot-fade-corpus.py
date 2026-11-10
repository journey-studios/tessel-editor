#!/usr/bin/env python3
"""Pilot fade corpus: small pinned cases with the provenance of every expectation (P2).

BL029 / BL040 / BL082.1, `docs/migration/pilot-parity-execution-plan.md` (P2). The vignette's twin is
`scripts/pilot-fx-corpus.py`; this one is standalone and shares no routine with it or with the Rust
`directed_scene::fx::reference`.

Writes (`generate`) and checks (`verify`), in `docs/migration/fixtures/pilot-fx/`:

  fade-cases.json     the cases: effect layers laid over the minimal bundle, the passes expected at
                      chosen frames, the pixels expected of a tiny picture (a fade alone, and a fade
                      stacked with a vignette in both orders). Consumed by
                      `crates/core/tests/pilot_fade_corpus.rs`.
  fade-manifest.json  the provenance: origin snapshot, blob id + sha256 of every origin source an
                      expectation was read from, the sha256 of the corpus, the blockers.

HOW THE EXPECTATIONS WERE MADE: by transcribing the formulas of the pilot's `fade` sources
(`git show <snapshot>:<path>`, read only) into this script and evaluating them in float64. THE ORIGIN
IS NEVER INSTALLED, BUILT OR EXECUTED. The expectations are `origin-formula`, not the outputs of the
origin running. The constants a transcription could get wrong (ranges, defaults, parameter order) and
the shader lines are checked against the origin text at generation time, and `verify --origin`
re-checks every blob. The origin's golden `fade.png` needs its harness (an engine build and a
browser): a declared blocker, and no golden is invented.

Documented Rust difference: the pilot's pipeline round-trips the frame through linear light and
dithers when the fade is 0; the Rust reference and the runtime return the input bytes exactly when
`intensity x weight x amount` is 0 (the pilot comment says "exactly the identity at 0" as the intent).

    scripts/pilot-fade-corpus.py generate [--origin PATH] [--snapshot SHA]
    scripts/pilot-fade-corpus.py verify   [--origin PATH]       # offline when --origin is absent

Python standard library only. Exit 0 on success, 1 on a finding.
"""

import argparse
import hashlib
import json
import math
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CORPUS = ROOT / "docs/migration/fixtures/pilot-fx"
BASE_BUNDLE = "docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json"
DEFAULT_ORIGIN = Path.home() / "www/journeystudios/tesselapp"
DEFAULT_SNAPSHOT = "d40ac3f7c0"
SCHEMA = 1
CASES_NAME = "fade-cases.json"
MANIFEST_NAME = "fade-manifest.json"

FADE = "packages/scene-engine/src/fx/catalog/fade.ts"
SHADER = "packages/scene-engine/src/runtime/fx/shaders/fade.ts"
META = "packages/scene-engine/src/fx/catalog/fx-effect-meta.ts"
PARAMS = "packages/scene-engine/src/fx/catalog/param-defs.ts"
VIGNETTE_CATALOG = "packages/scene-engine/src/fx/catalog/vignette.ts"
VIGNETTE_SHADER = "packages/scene-engine/src/runtime/fx/shaders/vignette.ts"
SOURCES = [
    (FADE, "catalog: params, defaults, label and summary"),
    (SHADER, "shader: mix(source, colour, amount x uAmount)"),
    (META, "intensity is appended last, default 1, range 0..1"),
    (PARAMS, "ratioParam: 0..1, step 0.01"),
    (VIGNETTE_SHADER, "the vignette the stacking cases compose with"),
    (VIGNETTE_CATALOG, "vignette defaults used by the stacking cases"),
    ("packages/scene-engine/src/runtime/fx/shaders/chunks/color.ts", "sRGB <-> linear curves"),
    ("packages/scene-engine/src/fx/fx-clip-sample.ts", "envelopeWeight: fadeIn/fadeOut at mid-frame"),
    ("packages/scene-engine/src/fx/fx-frame-decks.ts", "order of tracks, hidden clips and tracks"),
    ("packages/scene-engine/src/runtime/fx/fx-compositor.ts", "amount = clamp01(intensity * weight)"),
    ("packages/scene-engine/src/fx/catalog/__golden__/fade.png", "GOLDEN (blocked: not comparable)"),
    ("packages/scene-engine/scripts/fx-golden.mjs", "golden harness and tolerance (blocked: not run)"),
]

# The fade's parameters as the origin declares them: (min, max, default); colour is black by default.
FADE_AMOUNT = (0.0, 1.0, 1.0)
FADE_INTENSITY = (0.0, 1.0, 1.0)
FADE_ORDER = ["color", "amount", "intensity"]

# The vignette's, for the stacking cases only (the vignette corpus pins them separately).
VIGNETTE = {"radius": 0.75, "softness": 0.45, "intensity": 0.7, "roundness": 0.6}


def check_against(texts):
    """The numbers and lines transcribed here must be the origin's. Returns problems."""
    problems = []
    fade = texts.get(FADE, "")
    if 'id: "fade"' not in fade or 'family: "look"' not in fade:
        problems.append("catalog: the fade is not a `look` with id `fade` in the origin text")
    if 'params: [colorDef("color", { r: 0, g: 0, b: 0 }), ratioParam("amount", 1)],' not in fade:
        problems.append("catalog: the fade params are not (colour black, amount ratio default 1)")
    if "typicalDurationFrames: 24," not in fade:
        problems.append("catalog: typicalDurationFrames is not 24")
    shader = texts.get(SHADER, "")
    for line in (
        "gl_FragColor = vec4(mix(sourceAt(vUv), uColor, uFade * uAmount), 1.0);",
        'uFade: numberParam(params, "amount"),',
        'uColor: colorParam(params, "color"),',
        "Exactly the identity at 0.",
    ):
        if line not in shader:
            problems.append("shader: line not found in the origin: %s" % line)
    meta = texts.get(META, "")
    intensity = re.search(
        r'id:\s*"intensity",\s*kind:\s*"number",\s*min:\s*0,\s*max:\s*1,\s*default:\s*1,\s*step:\s*0\.01', meta
    )
    if not intensity or "[...meta.params, INTENSITY_PARAM]" not in meta:
        problems.append("meta: intensity 0..1 default 1, appended last, is not in the origin text")
    if "min = 0, max = 1" not in texts.get(PARAMS, "") or "step: 0.01," not in texts.get(PARAMS, ""):
        problems.append("params: ratioParam is not 0..1 step 0.01 in the origin text")
    sample = texts.get("packages/scene-engine/src/fx/fx-clip-sample.ts", "")
    for line in (
        "weight = Math.min(weight, (frameInClip + 0.5) / fadeIn)",
        "(clip.durationFrames - frameInClip - 0.5) / fadeOut,",
    ):
        if line not in sample:
            problems.append("envelope: line not found in the origin: %s" % line)
    if "return Math.max(0, Math.min(1, weight))" not in sample:
        problems.append("envelope: the 0..1 clamp is not in the origin text")
    decks = texts.get("packages/scene-engine/src/fx/fx-frame-decks.ts", "")
    for line in (
        "for (const track of bundle.fx?.tracks ?? [])",
        "track.hidden ||",
        "const ordered = [...track.clips].sort((a, b) => a.startFrame - b.startFrame)",
        "for (const clip of ordered)",
        "if (!clip.hidden)",
        "clips.push(clip)",
    ):
        if line not in decks:
            problems.append("deck order or visibility: line not found: " + line)
    if "amount: clampUnit(intensity * pass.weight)," not in texts.get(
        "packages/scene-engine/src/runtime/fx/fx-compositor.ts", ""
    ):
        problems.append("compositor: amount = clampUnit(intensity * pass.weight) not in the origin text")
    if "gl_FragColor = vec4(mix(c, uColor, shade * uAmount), 1.0);" not in texts.get(VIGNETTE_SHADER, ""):
        problems.append("vignette shader: the mix line is not in the origin text")
    vignette_shader = texts.get(VIGNETTE_SHADER, "")
    for line in (
        "float stretch = mix(1.0, uSize.x / uSize.y, uRoundness);",
        "vec2 shape = vec2(stretch, 1.0);",
        "float r = length((vUv - uCenter) * 2.0 * shape) / max(length(shape), 1.0);",
        "float shade = smoothstep(uRadius, uRadius + uSoftness, r);",
    ):
        if line not in vignette_shader:
            problems.append("vignette shader: shape line not found: " + line)
    catalog = texts.get(VIGNETTE_CATALOG, "")
    for param, default in VIGNETTE.items():
        block = re.search(r'id:\s*"' + re.escape(param) + r'"[^}]*?default:\s*([0-9.]+)', catalog)
        if block is None or float(block.group(1)) != default:
            problems.append("vignette catalog: default mismatch: " + param)
    if 'default: { x: 0.5, y: 0.5 }' not in catalog or 'default: { r: 0, g: 0, b: 0 }' not in catalog:
        problems.append("vignette catalog: centre or colour default mismatch")
    return problems


# ---- the formulas, evaluated ------------------------------------------------------------------


def envelope_weight(clip, frame_in_clip):
    weight = 1.0
    fade_in = clip.get("fadeInFrames", 0)
    fade_out = clip.get("fadeOutFrames", 0)
    if fade_in > 0:
        weight = min(weight, (frame_in_clip + 0.5) / fade_in)
    if fade_out > 0:
        weight = min(weight, (clip["durationFrames"] - frame_in_clip - 0.5) / fade_out)
    return max(0.0, min(1.0, weight))


def params_of(clip):
    given = clip.get("params", {})
    if clip["effect"] == "fade":
        return {
            "color": given.get("color", {"r": 0.0, "g": 0.0, "b": 0.0}),
            "amount": given.get("amount", FADE_AMOUNT[2]),
            "intensity": given.get("intensity", FADE_INTENSITY[2]),
        }
    out = {name: given.get(name, default) for name, default in VIGNETTE.items()}
    out["color"] = given.get("color", {"r": 0.0, "g": 0.0, "b": 0.0})
    out["center"] = given.get("center", {"x": 0.5, "y": 0.5})
    return out


def passes_at(fx, frame):
    """The passes of the output layer at an edit frame, in the origin's order."""
    passes = []
    for track in fx["tracks"]:
        if track.get("hidden"):
            continue
        for clip in sorted(track["clips"], key=lambda c: c["startFrame"]):
            if clip.get("hidden"):
                continue
            start = clip["startFrame"]
            if not (start <= frame < start + clip["durationFrames"]):
                continue
            in_clip = frame - start
            weight = envelope_weight(clip, in_clip)
            params = params_of(clip)
            passes.append(
                {
                    "track": track["id"],
                    "clip": clip["id"],
                    "effect": clip["effect"],
                    "frame_in_clip": in_clip,
                    "weight": weight,
                    "amount": max(0.0, min(1.0, params["intensity"] * weight)),
                    "fade_amount": params["amount"] if clip["effect"] == "fade" else None,
                }
            )
    return passes


def srgb_to_linear(c):
    return c / 12.92 if c <= 0.04045 else ((max(c, 0.0) + 0.055) / 1.055) ** 2.4


def linear_to_srgb(c):
    c = max(0.0, min(1.0, c))
    return c * 12.92 if c < 0.0031308 else 1.055 * c ** (1.0 / 2.4) - 0.055


def smoothstep(edge0, edge1, x):
    t = max(0.0, min(1.0, (x - edge0) / (edge1 - edge0)))
    return t * t * (3.0 - 2.0 * t)


def shade_pixels(rgba, width, height, params, amount, strength_at):
    """One pass: mix(source, colour, strength x amount) in linear light, alpha untouched."""
    if amount <= 0.0:
        return list(rgba)
    out = list(rgba)
    colour = (params["color"]["r"], params["color"]["g"], params["color"]["b"])
    for y in range(height):
        for x in range(width):
            weight = strength_at(x, y) * amount
            if weight <= 0.0:
                continue
            at = (y * width + x) * 4
            for channel in range(3):
                linear = srgb_to_linear(rgba[at + channel] / 255.0)
                mixed = linear + (colour[channel] - linear) * weight
                out[at + channel] = int(max(0, min(255, math.floor(linear_to_srgb(mixed) * 255.0 + 0.5))))
    return out


def fade_pixels(rgba, width, height, params, amount):
    # `uFade` is the amount parameter, uniform over the frame.
    return shade_pixels(rgba, width, height, params, amount, lambda x, y: params["amount"])


def vignette_pixels(rgba, width, height, params, amount):
    centre = (params["center"]["x"], 1.0 - params["center"]["y"])  # the point's y is flipped for the shader
    stretch = 1.0 + (width / height - 1.0) * params["roundness"]
    shape = (stretch, 1.0)
    norm = max(math.hypot(*shape), 1.0)

    def shade(x, y):
        u = (x + 0.5) / width
        v = 1.0 - (y + 0.5) / height
        r = math.hypot((u - centre[0]) * 2.0 * shape[0], (v - centre[1]) * 2.0 * shape[1]) / norm
        return smoothstep(params["radius"], params["radius"] + params["softness"], r)

    return shade_pixels(rgba, width, height, params, amount, shade)


def picture(width, height):
    rgba = []
    for y in range(height):
        for x in range(width):
            rgba += [
                60 + (x * 130) // max(width - 1, 1),
                90 + (y * 100) // max(height - 1, 1),
                200 - (x * 40) // max(width - 1, 1),
                255,
            ]
    return rgba


# ---- the cases ----------------------------------------------------------------------------------


def layer(*tracks):
    return {"catalogVersion": 1, "transitions": [], "tracks": list(tracks)}


def track(track_id, *clips):
    return {"id": track_id, "label": "FX " + track_id, "clips": list(clips)}


def clip(clip_id, effect, start, duration, **extra):
    value = {"id": clip_id, "effect": effect, "startFrame": start, "durationFrames": duration}
    value.update(extra)
    return value


def frames_case(case_id, title, fx, frames, basis):
    return {
        "id": case_id,
        "title": title,
        "kind": "frames",
        "origin_basis": basis,
        "fx": fx,
        "frames": [{"frame": f, "passes": passes_at(fx, f)} for f in frames],
    }


def pixels_case(case_id, title, fx, frame, width, height, basis):
    base = picture(width, height)
    expected = list(base)
    for item in passes_at(fx, frame):
        owner = next(c for t in fx["tracks"] for c in t["clips"] if c["id"] == item["clip"])
        draw = fade_pixels if owner["effect"] == "fade" else vignette_pixels
        expected = draw(expected, width, height, params_of(owner), item["amount"])
    return {
        "id": case_id,
        "title": title,
        "kind": "pixels",
        "origin_basis": basis,
        "fx": fx,
        "frame": frame,
        "width": width,
        "height": height,
        "input_rgba_hex": bytes(base).hex(),
        "expected_rgba_hex": bytes(expected).hex(),
        "tolerance_per_channel": 1,
        "identity": expected == base,
    }


def build_cases():
    white = {"r": 1.0, "g": 1.0, "b": 1.0}
    red = {"r": 1.0, "g": 0.0, "b": 0.0}
    fade_to_white = layer(track("t1", clip("c1", "fade", 10, 30, params={"color": white, "amount": 0.5})))
    stack_fade_first = layer(
        track("t1", clip("f", "fade", 0, 40, params={"color": white, "amount": 0.7})),
        track("t2", clip("v", "vignette", 0, 40, params={"intensity": 1.0})),
    )
    stack_vignette_first = layer(
        track("t1", clip("v", "vignette", 0, 40, params={"intensity": 1.0})),
        track("t2", clip("f", "fade", 0, 40, params={"color": white, "amount": 0.7})),
    )
    return [
        frames_case(
            "fade-defaults-window",
            "A default fade draws on [start, start+duration) and nowhere else; intensity 1 gives amount 1",
            layer(track("t1", clip("c1", "fade", 10, 30))),
            [9, 10, 25, 39, 40],
            "origin-formula: fade.ts defaults, fx-compositor amount = intensity x weight",
        ),
        frames_case(
            "fade-envelope-multiplies-with-the-amount-parameter",
            "fadeIn/fadeOut shape the clip weight at mid-frame; the amount parameter is a separate factor",
            layer(
                track(
                    "t1",
                    clip("c1", "fade", 0, 20, fadeInFrames=4, fadeOutFrames=4, params={"amount": 0.6, "intensity": 0.8}),
                )
            ),
            [0, 1, 2, 3, 4, 10, 15, 16, 17, 18, 19],
            "origin-formula: envelopeWeight at (frame + 0.5), amount = clamp01(intensity x weight); the fade shader multiplies by the amount param",
        ),
        frames_case(
            "fade-hidden-and-order",
            "A hidden clip draws nothing; the passes come in track order, a fade before a vignette",
            layer(
                track("t1", clip("c1", "fade", 0, 30)),
                track("t2", clip("c2", "vignette", 0, 30)),
                track("t3", clip("c3", "fade", 0, 30, hidden=True)),
            ),
            [5],
            "origin-formula: fx-frame-decks (track order, hidden clips)",
        ),
        pixels_case(
            "fade-amount-zero-is-the-baseline",
            "amount 0 mixes nothing: the exact input picture",
            layer(track("t1", clip("c1", "fade", 0, 40, params={"color": white, "amount": 0.0}))),
            10, 32, 18,
            "origin-formula: mix(source, colour, 0 x uAmount) is the source",
        ),
        pixels_case(
            "fade-intensity-zero-is-the-baseline",
            "intensity 0 makes the pass amount 0: the exact input picture",
            layer(track("t1", clip("c1", "fade", 0, 40, params={"color": white, "intensity": 0.0}))),
            10, 32, 18,
            "origin-formula: amount = clamp01(intensity x weight) = 0",
        ),
        pixels_case(
            "fade-half-way-to-white-in-linear-light",
            "amount 0.5 towards white: the mean of the linear values, then encoded",
            fade_to_white,
            20, 32, 18,
            "origin-formula: shaders/fade.ts + chunks/color.ts",
        ),
        pixels_case(
            "fade-red-at-quarter-intensity",
            "intensity 0.25 towards red",
            layer(track("t1", clip("c1", "fade", 0, 40, params={"color": red, "intensity": 0.25}))),
            10, 32, 18,
            "origin-formula: pass amount 0.25, colour passed through unchanged",
        ),
        pixels_case(
            "fade-fully-red-is-the-colour",
            "intensity 1 and amount 1 towards red: every pixel is the colour",
            layer(track("t1", clip("c1", "fade", 0, 40, params={"color": red}))),
            10, 32, 18,
            "origin-formula: mix(source, colour, 1) is the colour",
        ),
        pixels_case(
            "fade-then-vignette",
            "a fade drawn before a vignette: the vignette darkens the lifted picture",
            stack_fade_first,
            10, 32, 18,
            "origin-formula: passes are drawn in track order, each over the previous output",
        ),
        pixels_case(
            "vignette-then-fade",
            "the same two clips in the other track order give other pixels",
            stack_vignette_first,
            10, 32, 18,
            "origin-formula: passes are drawn in track order, each over the previous output",
        ),
    ]


def build_corpus():
    return {
        "schema": SCHEMA,
        "base_bundle": BASE_BUNDLE,
        "edit_duration_frames": 96,
        "note": "Generated by scripts/pilot-fade-corpus.py; edit nothing by hand (verify fails).",
        "cases": build_cases(),
        "refusals": [
            {
                "id": "amount-above-range",
                "basis": "origin catalog: amount is ratioParam 0..1 (fade.ts, param-defs.ts). The code is the Rust core's own",
                "op": {"op": "set_param", "clip": "c1", "param": "amount", "value": 1.5},
                "fx": layer(track("t1", clip("c1", "fade", 10, 30))),
                "code": "param_out_of_range",
            },
            {
                "id": "intensity-above-range",
                "basis": "origin meta: intensity is 0..1 for every effect (fx-effect-meta.ts). The code is the Rust core's own",
                "op": {"op": "set_param", "clip": "c1", "param": "intensity", "value": 2.0},
                "fx": layer(track("t1", clip("c1", "fade", 10, 30))),
                "code": "param_out_of_range",
            },
        ],
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
        "purpose": "Provenance of the pilot fade corpus (fade-cases.json): where each expectation comes from.",
        "origin": {
            "repository": "journeystudios/tesselapp (read only; never installed, built or executed)",
            "snapshot": snapshot,
            "snapshot_commit": full_snapshot,
            "access": "git show / cat-file of the blobs below",
        },
        "corpus": {
            "file": CASES_NAME,
            "sha256": sha256_of(corpus_bytes),
            "base_bundle": BASE_BUNDLE,
            "generator": "scripts/pilot-fade-corpus.py",
            "regenerate": "scripts/pilot-fade-corpus.py generate --origin <path to the origin repo>",
        },
        "expectation_kinds": {
            "origin-formula": "read from the origin source and evaluated in float64 by the generator; the origin was not run",
            "rust-contract": "a code or behaviour of the Rust core, flagged as such (not claimed of the origin)",
        },
        "pixel_tolerance": {
            "per_channel": 1,
            "why": "the reference is float64 with the origin's curves; the Rust one may differ by the last rounding of pow",
        },
        "documented_differences": [
            {
                "id": "ZERO_WEIGHT_IS_BYTE_IDENTITY",
                "detail": "The pilot pipeline is linear throughout and encodes at the end (with dither); the Rust reference and the runtime return the input bytes exactly when intensity x weight x amount is 0. The identity cases pin the exact bytes.",
            }
        ],
        "sources": sources,
        "blockers": [
            {
                "id": "ORIGIN_GOLDEN_NOT_COMPARABLE",
                "detail": "The origin golden (catalog/__golden__/fade.png) needs its harness: a build of the engine and a browser, i.e. executing the origin. Forbidden in this delivery. No golden was invented and none is compared.",
                "consequence": "Pixel equivalence with the origin is `origin-formula`, never golden equivalence. Visual parity stays partial.",
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
    cases_path, manifest_path = paths()
    CORPUS.mkdir(parents=True, exist_ok=True)
    cases_path.write_bytes(corpus_bytes)
    manifest_path.write_bytes(canonical(build_manifest(corpus_bytes, sources, snapshot, full)))
    print("wrote %s (%d cases) and %s" % (cases_path.relative_to(ROOT), len(build_cases()), manifest_path.relative_to(ROOT)))
    return 0


def verify(origin):
    cases_path, manifest_path = paths()
    findings = []
    if not cases_path.exists() or not manifest_path.exists():
        print("missing corpus or manifest: run `generate`", file=sys.stderr)
        return 1
    corpus_bytes = cases_path.read_bytes()
    manifest = json.loads(manifest_path.read_text())
    if sha256_of(corpus_bytes) != manifest["corpus"]["sha256"]:
        findings.append("fade-cases.json was edited: its sha256 is not the manifest's")
    if corpus_bytes != canonical(build_corpus()):
        findings.append("fade-cases.json is not what the generator produces today (formulas or cases changed)")
    if not (ROOT / manifest["corpus"]["base_bundle"]).exists():
        findings.append("the base bundle is missing: " + manifest["corpus"]["base_bundle"])
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
    print("pilot-fade corpus verified %s: %d cases, %d sources" % (mode, len(build_cases()), len(manifest["sources"])))
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
