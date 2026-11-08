#!/usr/bin/env python3
"""Pilot vignette corpus: small pinned cases with the provenance of every expectation (P0).

BL036 / BL018 / BL019 / BL082.17, `docs/migration/pilot-parity-execution-plan.md` (P0).

What it writes (`generate`) and checks (`verify`), in `docs/migration/fixtures/pilot-fx/`:

  cases.json    the cases: effect layers (`fx`) laid over the existing minimal bundle, the passes
                expected at chosen frames, the pixels expected of a tiny picture, and the gesture
                behaviours the Rust core must show. Consumed by `crates/core/tests/pilot_fx_corpus.rs`.
  manifest.json the provenance: origin snapshot, blob id + sha256 of every origin source an
                expectation was read from, the sha256 of this corpus, the blockers.

HOW THE EXPECTATIONS WERE MADE: by transcribing, into this script, the formulas of the pilot's
sources (`git show <snapshot>:<path>`, read only) and evaluating them in float64. The ORIGIN IS NEVER
INSTALLED, BUILT OR EXECUTED here. So the expectations are `origin-formula` (read from source), not
outputs of the origin running; the constants that a transcription could get wrong (the catalog's
ranges and defaults) are checked against the origin text at generation time, and `verify --origin`
re-checks every blob. The pilot's golden PNG cannot be compared without running its harness (a
browser and the engine build over its own test plates): it is a declared blocker, and no golden is
invented (manifest `blockers`).

The pixel reference below is written from the origin's GLSL and colour chunks, not from the Rust
`directed_scene::fx::reference`: it is an independent second implementation, so a test that compares
the two compares two derivations of the origin's shader, not one path with itself.

    scripts/pilot-fx-corpus.py generate [--origin PATH] [--snapshot SHA]
    scripts/pilot-fx-corpus.py verify   [--origin PATH]       # offline when --origin is absent

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

# Where each expectation was read from: path inside the origin, and what it was used for.
SOURCES = [
    ("packages/scene-engine/src/fx/catalog/vignette.ts", "catalog: params, ranges, defaults"),
    ("packages/scene-engine/src/runtime/fx/shaders/vignette.ts", "shader: the falloff and the mix"),
    ("packages/scene-engine/src/runtime/fx/shaders/chunks/params.ts", "centre: y flipped for the shader"),
    ("packages/scene-engine/src/runtime/fx/shaders/chunks/color.ts", "sRGB <-> linear curves"),
    ("packages/scene-engine/src/fx/fx-clip-sample.ts", "envelopeWeight: fadeIn/fadeOut at mid-frame"),
    ("packages/scene-engine/src/fx/fx-frame-decks.ts", "order of tracks, hidden clips and tracks"),
    ("packages/scene-engine/src/runtime/fx/fx-compositor.ts", "amount = clamp01(intensity * weight)"),
    ("apps/scene-studio/src/hooks/useFxEdit.ts", "gesture: preview records nothing, one undo, refusal"),
    ("apps/scene-studio/src/engine/fxSlice.ts", "commit that changes nothing records nothing"),
    ("packages/scene-engine/src/fx/catalog/__golden__/vignette.png", "GOLDEN (blocked: not comparable)"),
    ("packages/scene-engine/scripts/fx-golden.mjs", "golden harness and tolerance (blocked: not run)"),
]

# ---- the pilot's vignette, transcribed (constants checked against the origin text) -------------

CATALOG = {
    "radius": (0.0, 1.5, 0.75),
    "softness": (0.05, 1.0, 0.45),
    "intensity": (0.0, 1.0, 0.7),
    "roundness": (0.0, 1.0, 0.6),
}
DEFAULT_COLOR = (0.0, 0.0, 0.0)
DEFAULT_CENTER = (0.5, 0.5)
CATALOG_DEFAULT_ORDER = ["radius", "softness", "intensity", "color", "roundness", "center"]


def check_catalog_against(text):
    """The numbers above must be the origin's. Returns problems (empty when they are)."""
    problems = []
    for name, (low, high, default) in CATALOG.items():
        pattern = (
            r'id:\s*"%s",\s*kind:\s*"number",\s*min:\s*([\d.]+),\s*max:\s*([\d.]+),\s*default:\s*([\d.]+)'
            % name
        )
        found = re.search(pattern, text)
        if not found:
            problems.append("catalog: no number param `%s` in the origin text" % name)
            continue
        got = tuple(float(x) for x in found.groups())
        if got != (low, high, default):
            problems.append("catalog: `%s` is %s in the origin, %s here" % (name, got, (low, high, default)))
    if 'default: { r: 0, g: 0, b: 0 }' not in text:
        problems.append("catalog: the default colour is not black in the origin text")
    if 'default: { x: 0.5, y: 0.5 }' not in text:
        problems.append("catalog: the default centre is not (0.5, 0.5) in the origin text")
    order = re.findall(r'\n\s*(?:\{\s*)?id:\s*"(\w+)",\s*kind:', text)
    if order != CATALOG_DEFAULT_ORDER:
        problems.append("catalog: param order is %s in the origin, %s here" % (order, CATALOG_DEFAULT_ORDER))
    return problems


def check_formulas_against(texts):
    """The shader and clip formulas this script re-implements must still be the origin's lines."""
    problems = []
    shader = texts.get("packages/scene-engine/src/runtime/fx/shaders/vignette.ts", "")
    for line in (
        "float stretch = mix(1.0, uSize.x / uSize.y, uRoundness);",
        "vec2 shape = vec2(stretch, 1.0);",
        "float r = length((vUv - uCenter) * 2.0 * shape) / max(length(shape), 1.0);",
        "float shade = smoothstep(uRadius, uRadius + uSoftness, r);",
        "gl_FragColor = vec4(mix(c, uColor, shade * uAmount), 1.0);",
    ):
        if line not in shader:
            problems.append("shader: line not found in the origin: %s" % line)
    if "return [value.x, 1 - value.y]" not in texts.get(
        "packages/scene-engine/src/runtime/fx/shaders/chunks/params.ts", ""
    ):
        problems.append("params: the centre y flip is not in the origin text")
    sample = texts.get("packages/scene-engine/src/fx/fx-clip-sample.ts", "")
    for line in (
        "weight = Math.min(weight, (frameInClip + 0.5) / fadeIn)",
        "(clip.durationFrames - frameInClip - 0.5) / fadeOut,",
        "return Math.max(0, Math.min(1, weight))",
    ):
        if line not in sample:
            problems.append("envelope: line not found in the origin: %s" % line)
    if "amount: clampUnit(intensity * pass.weight)," not in texts.get(
        "packages/scene-engine/src/runtime/fx/fx-compositor.ts", ""
    ):
        problems.append("compositor: amount = clampUnit(intensity * pass.weight) not in the origin text")
    decks = texts.get("packages/scene-engine/src/fx/fx-frame-decks.ts", "")
    if "[...track.clips].sort((a, b) => a.startFrame - b.startFrame)" not in decks or "track.hidden" not in decks:
        problems.append("decks: the order and hidden rules are not in the origin text")
    edit = texts.get("apps/scene-studio/src/hooks/useFxEdit.ts", "")
    for phrase in (
        "WITHOUT",
        "recording anything; releasing records ONE undo step",
        "the preview keeps the last legal state",
        "letting go",
        "there changes nothing",
    ):
        if phrase not in edit:
            problems.append("gesture: phrase not found in the origin text: %s" % phrase)
    return problems


# ---- the formulas, evaluated ----------------------------------------------------------------------


def envelope_weight(clip, frame_in_clip):
    weight = 1.0
    fade_in = clip.get("fadeInFrames", 0)
    fade_out = clip.get("fadeOutFrames", 0)
    if fade_in > 0:
        weight = min(weight, (frame_in_clip + 0.5) / fade_in)
    if fade_out > 0:
        weight = min(weight, (clip["durationFrames"] - frame_in_clip - 0.5) / fade_out)
    return max(0.0, min(1.0, weight))


def clip_params(clip):
    given = clip.get("params", {})
    out = {name: given.get(name, default) for name, (_, _, default) in CATALOG.items()}
    out["color"] = given.get("color", dict(zip("rgb", DEFAULT_COLOR)))
    out["center"] = given.get("center", dict(zip("xy", DEFAULT_CENTER)))
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
            params = clip_params(clip)
            amount = max(0.0, min(1.0, params["intensity"] * weight))
            passes.append(
                {
                    "track": track["id"],
                    "clip": clip["id"],
                    "frame_in_clip": in_clip,
                    "weight": weight,
                    "amount": amount,
                    "intensity": params["intensity"],
                }
            )
    return passes


def smoothstep(edge0, edge1, x):
    t = max(0.0, min(1.0, (x - edge0) / (edge1 - edge0)))
    return t * t * (3.0 - 2.0 * t)


def srgb_to_linear(c):
    return c / 12.92 if c <= 0.04045 else ((max(c, 0.0) + 0.055) / 1.055) ** 2.4


def linear_to_srgb(c):
    c = max(0.0, min(1.0, c))
    return c * 12.92 if c < 0.0031308 else 1.055 * c ** (1.0 / 2.4) - 0.055


def vignette_pixels(rgba, width, height, params, amount):
    """The origin's fragment shader, per pixel, with y UP (vUv) as the GLSL has it."""
    if amount <= 0.0:
        return list(rgba)
    out = list(rgba)
    centre = (params["center"]["x"], 1.0 - params["center"]["y"])  # pointParam flips y
    colour = (params["color"]["r"], params["color"]["g"], params["color"]["b"])
    stretch = 1.0 + (width / height - 1.0) * params["roundness"]  # mix(1, w/h, roundness)
    shape = (stretch, 1.0)
    norm = max(math.hypot(*shape), 1.0)
    for y in range(height):
        for x in range(width):
            u = (x + 0.5) / width
            v = 1.0 - (y + 0.5) / height  # row 0 is the top of the picture, vUv.y is up
            r = math.hypot((u - centre[0]) * 2.0 * shape[0], (v - centre[1]) * 2.0 * shape[1]) / norm
            shade = smoothstep(params["radius"], params["radius"] + params["softness"], r)
            at = (y * width + x) * 4
            for channel in range(3):
                linear = srgb_to_linear(rgba[at + channel] / 255.0)
                mixed = linear + (colour[channel] - linear) * (shade * amount)
                out[at + channel] = int(max(0, min(255, math.floor(linear_to_srgb(mixed) * 255.0 + 0.5))))
    return out


def picture(width, height):
    """A deterministic gradient with a different value in every channel."""
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


def track(track_id, *clips, hidden=False):
    value = {"id": track_id, "label": "FX " + track_id, "clips": list(clips)}
    if hidden:
        value["hidden"] = True
    return value


def clip(clip_id, start, duration, **extra):
    value = {"id": clip_id, "effect": "vignette", "startFrame": start, "durationFrames": duration}
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
    found = passes_at(fx, frame)
    expected = list(base)
    for item in found:
        owner = next(c for t in fx["tracks"] for c in t["clips"] if c["id"] == item["clip"])
        expected = vignette_pixels(expected, width, height, clip_params(owner), item["amount"])
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
    full = {"intensity": 1.0, "radius": 0.3, "softness": 0.4, "roundness": 0.6}
    cases = [
        frames_case(
            "defaults-window",
            "A default vignette draws on [start, start+duration) and nowhere else; amount is intensity (0.7)",
            layer(track("t1", clip("c1", 10, 30))),
            [9, 10, 25, 39, 40],
            "fx-clip-sample.ts envelopeWeight, fx-compositor.ts applyPass, fx-frame-decks.ts effectsOn",
        ),
        frames_case(
            "fades",
            "fadeIn/fadeOut weigh the clip at the MIDDLE of each frame; amount = intensity * weight",
            layer(track("t1", clip("c1", 20, 20, fadeInFrames=4, fadeOutFrames=4, params={"intensity": 0.8}))),
            [19, 20, 21, 22, 23, 24, 30, 35, 36, 37, 38, 39, 40],
            "fx-clip-sample.ts envelopeWeight",
        ),
        frames_case(
            "intensity-zero",
            "Intensity 0 still resolves a pass (the clip is there) whose amount is exactly 0",
            layer(track("t1", clip("c1", 5, 10, params={"intensity": 0.0}))),
            [4, 5, 9, 14, 15],
            "fx-compositor.ts applyPass (clampUnit(0 * weight) = 0)",
        ),
        frames_case(
            "hidden",
            "A hidden clip and a hidden track draw nothing; the visible clip still does",
            layer(
                track("t1", clip("hidden-track-clip", 0, 40), hidden=True),
                track("t2", clip("hidden-clip", 0, 40, hidden=True)),
                track("t3", clip("visible", 0, 40)),
            ),
            [0, 20, 39],
            "fx-frame-decks.ts visibleClips (track.hidden, clip.hidden)",
        ),
        frames_case(
            "track-order",
            "Passes come in the order of the tracks, whatever the frame",
            layer(
                track("t1", clip("a", 0, 20, params={"intensity": 0.3})),
                track("t2", clip("b", 10, 20, params={"intensity": 0.6})),
            ),
            [5, 15, 25],
            "fx-frame-decks.ts outputEffects: the order of the tracks",
        ),
        frames_case(
            "last-frames-of-the-edit",
            "A clip may end exactly on the last frame of the edit (96 frames)",
            layer(track("t1", clip("c1", 90, 6))),
            [89, 90, 95],
            "fx-clip-sample.ts envelopeWeight (frame is relative to the clip)",
        ),
        pixels_case(
            "pixels-centre",
            "Full intensity, black, centred: the corners darken, the middle does not",
            layer(track("t1", clip("c1", 0, 20, params=full))),
            5,
            8,
            4,
            "vignette.ts shader + params.ts + color.ts",
        ),
        pixels_case(
            "pixels-off-centre-colour",
            "Centre at the bottom right, a blue tint: the dark region moves with the centre",
            layer(
                track(
                    "t1",
                    clip(
                        "c1",
                        0,
                        20,
                        params=dict(full, center={"x": 1.0, "y": 1.0}, color={"r": 0.0, "g": 0.0, "b": 0.5}),
                    ),
                )
            ),
            5,
            8,
            4,
            "vignette.ts shader + params.ts centre y flip",
        ),
        pixels_case(
            "pixels-amount-zero",
            "Intensity 0 keeps the picture byte for byte",
            layer(track("t1", clip("c1", 0, 20, params=dict(full, intensity=0.0)))),
            5,
            8,
            4,
            "fx-compositor.ts amount = 0",
        ),
        pixels_case(
            "pixels-no-effect",
            "No clip at the frame keeps the picture byte for byte",
            layer(track("t1", clip("c1", 10, 5, params=full))),
            5,
            8,
            4,
            "fx-frame-decks.ts: no pass outside the clip",
        ),
    ]
    return cases


def gesture_cases():
    """Behaviours of an effect drag. `origin` is read from source comments (never run)."""
    layer_json = layer(track("t1", clip("v1", 10, 30, params={"intensity": 0.5})))
    steps = lambda *s: list(s)  # noqa: E731
    update = lambda value: {"do": "update", "intensity": value}  # noqa: E731
    return [
        {
            "id": "preview-records-nothing",
            "origin": "useFxEdit.ts: runs the command and shows it in the preview WITHOUT recording anything",
            "fx": layer_json,
            "steps": steps({"do": "begin"}, update(0.6), update(0.2), {"do": "status"}),
            "expect": {"undo_entries_added": 0, "document_intensity": 0.5, "candidate_intensity": 0.2},
        },
        {
            "id": "one-undo-per-drag",
            "origin": "useFxEdit.ts: releasing records ONE undo step",
            "fx": layer_json,
            "steps": steps({"do": "begin"}, update(0.6), update(0.5), update(0.2), {"do": "commit"}),
            "expect": {"undo_entries_added": 1, "document_intensity": 0.2},
        },
        {
            "id": "refusal-keeps-last-legal",
            "origin": "useFxEdit.ts: a proposal the engine refuses is not shown: the preview keeps the last legal state",
            "fx": layer_json,
            "steps": steps({"do": "begin"}, update(0.3), {"do": "update_refused", "intensity": 2.0, "code": "param_out_of_range"}),
            "expect": {"undo_entries_added": 0, "document_intensity": 0.5, "candidate_intensity": 0.3},
        },
        {
            "id": "release-on-refusal",
            "origin": "useFxEdit.ts: letting go on a refusal changes nothing (the layer goes back)",
            "rust": "DIVERGENCE (by the P1 contract): the commit confirms the last LEGAL candidate",
            "divergence": {"origin": "revert to the layer at press", "rust": "commit the last legal candidate"},
            "fx": layer_json,
            "steps": steps(
                {"do": "begin"},
                update(0.3),
                {"do": "update_refused", "intensity": 2.0, "code": "param_out_of_range"},
                {"do": "commit"},
            ),
            "expect": {"undo_entries_added": 1, "document_intensity": 0.3},
        },
        {
            "id": "cancel-changes-nothing",
            "origin": "useFxEdit.ts onCancel: live.cancel() puts the layer back",
            "fx": layer_json,
            "steps": steps({"do": "begin"}, update(0.9), {"do": "cancel"}),
            "expect": {"undo_entries_added": 0, "document_intensity": 0.5},
        },
        {
            "id": "noop-records-nothing",
            "origin": "fxSlice.ts applyFx: domain.commit(...) answers changed=false when nothing changed",
            "fx": layer_json,
            "steps": steps({"do": "begin"}, update(0.5), update(0.9), update(0.5), {"do": "commit"}),
            "expect": {"undo_entries_added": 0, "document_intensity": 0.5},
        },
    ]


def build_corpus():
    return {
        "schema": SCHEMA,
        "base_bundle": BASE_BUNDLE,
        "edit_duration_frames": 96,
        "note": "Generated by scripts/pilot-fx-corpus.py; edit nothing by hand (verify fails).",
        "cases": build_cases(),
        "gesture_cases": gesture_cases(),
        "refusals": [
            {
                "id": "intensity-above-range",
                "basis": "origin catalog: intensity is 0..1 (vignette.ts). The code is the Rust core's own",
                "op": {"op": "set_param", "clip": "c1", "param": "intensity", "value": 2.0},
                "fx": layer(track("t1", clip("c1", 10, 30))),
                "code": "param_out_of_range",
            },
            {
                "id": "radius-above-range",
                "basis": "origin catalog: radius is 0..1.5 (vignette.ts). The code is the Rust core's own",
                "op": {"op": "set_param", "clip": "c1", "param": "radius", "value": 1.6},
                "fx": layer(track("t1", clip("c1", 10, 30))),
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
    return subprocess.run(
        ["git", "-C", str(origin), *args], check=True, capture_output=True
    ).stdout


def read_origin(origin, snapshot):
    """Blob id, sha256 and text of every source, by read-only git plumbing."""
    sources, texts = [], {}
    for path, role in SOURCES:
        blob = git(origin, "rev-parse", "%s:%s" % (snapshot, path)).decode().strip()
        data = git(origin, "cat-file", "blob", blob)
        sources.append({"path": path, "git_blob": blob, "sha256": sha256_of(data), "bytes": len(data), "role": role})
        if not path.endswith(".png"):
            texts[path] = data.decode("utf-8")
    return sources, texts


def origin_findings(texts):
    problems = check_catalog_against(texts["packages/scene-engine/src/fx/catalog/vignette.ts"])
    problems += check_formulas_against(texts)
    return problems


def build_manifest(corpus_bytes, cases_name, sources, snapshot, full_snapshot):
    return {
        "schema": SCHEMA,
        "purpose": "Provenance of the pilot vignette corpus (cases.json): where each expectation comes from.",
        "origin": {
            "repository": "journeystudios/tesselapp (read only; never installed, built or executed)",
            "snapshot": snapshot,
            "snapshot_commit": full_snapshot,
            "access": "git show / cat-file of the blobs below",
        },
        "corpus": {
            "file": cases_name,
            "sha256": sha256_of(corpus_bytes),
            "base_bundle": BASE_BUNDLE,
            "generator": "scripts/pilot-fx-corpus.py",
            "regenerate": "scripts/pilot-fx-corpus.py generate --origin <path to the origin repo>",
        },
        "expectation_kinds": {
            "origin-formula": "read from the origin source and evaluated in float64 by the generator; the origin was not run",
            "origin-source-behavior": "gesture behaviour stated in the comments of the origin hooks; not executed",
            "rust-contract": "a code or behaviour of the Rust core, flagged as such (not claimed of the origin)",
        },
        "pixel_tolerance": {
            "per_channel": 1,
            "why": "the reference is float64 with the origin's curves; the Rust one may differ by the last rounding of pow",
        },
        "sources": sources,
        "blockers": [
            {
                "id": "ORIGIN_GOLDEN_NOT_COMPARABLE",
                "detail": "The origin golden (catalog/__golden__/vignette.png, a strip drawn at 320x180 over the origin's own test plates, tolerance 2/255 on 0.5% of pixels) needs its harness: a build of the engine and a browser, i.e. executing the origin. That is forbidden in this delivery, and the plates are not part of the snapshot's blobs listed here. No golden was invented and none is compared.",
                "consequence": "Pixel equivalence with the origin is `origin-formula` (an independent float64 evaluation of its shader), never golden equivalence. Matrix status for visual parity stays partial.",
            },
            {
                "id": "ORIGIN_NOT_EXECUTED",
                "detail": "Expectations are transcribed formulas, not the origin's outputs. A misreading shared by this script and the Rust reference would pass both. Mitigation: constants and the formula lines are checked against the origin text at generation and by `verify --origin`; the shader, the colour curves and the centre convention were read from their own blobs.",
                "consequence": "A future execution of the origin (in its own, authorised environment) may replace `origin-formula` by `origin-output` per case.",
            },
        ],
    }


def paths():
    return CORPUS / "cases.json", CORPUS / "manifest.json"


def generate(origin, snapshot):
    sources, texts = read_origin(origin, snapshot)
    problems = origin_findings(texts)
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
    manifest = build_manifest(corpus_bytes, cases_path.name, sources, snapshot, full)
    manifest_path.write_bytes(canonical(manifest))
    print("wrote %s (%d cases, %d gesture cases) and %s" % (
        cases_path.relative_to(ROOT), len(build_cases()), len(gesture_cases()), manifest_path.relative_to(ROOT)))
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
        findings.append("cases.json was edited: its sha256 is not the manifest's")
    if corpus_bytes != canonical(build_corpus()):
        findings.append("cases.json is not what the generator produces today (formulas or cases changed)")
    base = ROOT / manifest["corpus"]["base_bundle"]
    if not base.exists():
        findings.append("the base bundle is missing: " + manifest["corpus"]["base_bundle"])
    kinds = {source["path"] for source in manifest["sources"]}
    for path, _ in SOURCES:
        if path not in kinds:
            findings.append("source not in the manifest: " + path)
    for blocker in ("ORIGIN_GOLDEN_NOT_COMPARABLE", "ORIGIN_NOT_EXECUTED"):
        if blocker not in {b["id"] for b in manifest["blockers"]}:
            findings.append("blocker dropped from the manifest: " + blocker)
    if origin is not None:
        try:
            snapshot = manifest["origin"]["snapshot"]
            sources, texts = read_origin(origin, snapshot)
        except subprocess.CalledProcessError as error:
            print("origin objects are missing (equivalence blocker): %s" % error, file=sys.stderr)
            return 1
        recorded = {s["path"]: s for s in manifest["sources"]}
        for source in sources:
            before = recorded.get(source["path"])
            if before is None or (before["git_blob"], before["sha256"]) != (source["git_blob"], source["sha256"]):
                findings.append("origin blob changed or unrecorded: " + source["path"])
        findings += origin_findings(texts)
        mode = "against the origin blobs"
    else:
        mode = "offline (origin blobs not re-read; pass --origin to re-check them)"
    if findings:
        for finding in findings:
            print("FINDING: " + finding, file=sys.stderr)
        return 1
    print("pilot-fx corpus verified %s: %d cases, %d gesture cases, %d sources" % (
        mode, len(build_cases()), len(gesture_cases()), len(manifest["sources"])))
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("command", choices=["generate", "verify"])
    parser.add_argument("--origin", type=Path, default=None)
    parser.add_argument("--snapshot", default=DEFAULT_SNAPSHOT)
    args = parser.parse_args()
    if args.command == "generate":
        origin = args.origin or DEFAULT_ORIGIN
        return generate(origin, args.snapshot)
    return verify(args.origin)


if __name__ == "__main__":
    sys.exit(main())
