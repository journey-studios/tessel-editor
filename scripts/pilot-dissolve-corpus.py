#!/usr/bin/env python3
"""Pilot crossDissolve corpus: an independent oracle for P4b-S0 (BL082.4.1, F064).

Plan: `docs/migration/p4b-cross-dissolve-plan.md` (S0). Siblings: `scripts/pilot-fx-corpus.py`,
`pilot-fade-corpus.py`, `pilot-grain-corpus.py`. This one is standalone: it shares no routine with them,
with the Rust `directed_scene::fx` module (which has no transitions yet) or with the runtime's GLSL.

Writes (`generate`) and checks (`verify`, `generate --check`), in `docs/migration/fixtures/pilot-fx/`:

  dissolve-cases.json     the vectors: the transition window (alignment, odd/even/1 frame, edit edges),
                          the progress and the effective weight, the 8-bit pixel mix in linear light,
                          which frame of each deck is drawn (instance and camera clamped apart, the real
                          edit clock kept), the shot-clip edge rule, the handle lacks, the ordered
                          refusals, ids/seeds/keys/hidden, the alpha verdict, and the tolerances
                          registered BEFORE any pixel was produced.
  dissolve-manifest.json  the provenance: the pin the plan names, the tip that was read, the blob id and
                          sha256 of every origin source, what the coordinator attested and what this
                          script re-checks, the documented differences and the blockers.

WHAT THIS IS NOT. It is a contract + oracle in Python. It is NOT the feature: there is no Rust, JSONL,
runtime or UI for transitions (F064 stays Pending for them). THE ORIGIN IS NEVER INSTALLED, BUILT OR
EXECUTED: every expectation is `origin-formula` (a transcription of the TS/GLSL text evaluated here),
`origin-spec-vector` (a number the origin's own spec pins), or a clearly labelled destination
contract/hardening. Verifying an origin blob by its id is `origin-blob verified`; it is not `origin
executed`. No origin golden is read or compared (ORIGIN_GOLDEN_NOT_COMPARABLE).

    scripts/pilot-dissolve-corpus.py generate --origin PATH [--out DIR]
    scripts/pilot-dissolve-corpus.py generate --check [--origin PATH] [--out DIR]   # writes nothing
    scripts/pilot-dissolve-corpus.py verify [--origin PATH] [--require-pin] [--out DIR]
    scripts/pilot-dissolve-corpus.py digests --origin PATH   # prints the constants to refresh

Exit codes: 0 verified | 1 finding (a difference, a tamper, a failed transcription) | 2 usage |
3 origin unavailable (ORIGIN_UNAVAILABLE: no tip object, git missing, `generate` without --origin) |
4 the pin could not be verified and --require-pin was given (PIN_UNVERIFIED) |
5 the interpreter is not a real Python (INTERPRETER_NOT_REAL: the Microsoft Store stub, an empty
executable or an interpreter older than 3.9). A refusal is never a skip: nothing is checked and nothing is written.
Every run prints `python: <sys.version>` on stderr, so the evidence names the interpreter that produced it.
Offline `verify` (no --origin) re-reads no origin blob and says so. Python standard library only.
No bare `assert` is used for validation: the file behaves the same under `python -O`.
"""

import argparse
import hashlib
import json
import math
import re
import subprocess
import sys
from fractions import Fraction
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CORPUS = ROOT / "docs/migration/fixtures/pilot-fx"
SCHEMA = 1
CASES_NAME = "dissolve-cases.json"
MANIFEST_NAME = "dissolve-manifest.json"

EXIT_OK, EXIT_FINDING, EXIT_ORIGIN_UNAVAILABLE, EXIT_PIN_UNVERIFIED, EXIT_BAD_INTERPRETER = 0, 1, 3, 4, 5
MIN_PYTHON = (3, 9)


def interpreter_problem(executable=None, version_info=None, size=None):
    """Why this interpreter must not run the oracle, or None. On Windows `python` and `python3` can resolve to the
    Microsoft Store *stub* (an app-execution alias under `...\\WindowsApps\\`, zero bytes, no interpreter behind it);
    a launcher that forwards to it must not be taken for Python. A real Store install lives under
    `WindowsApps\\PythonSoftwareFoundation.Python.*` and is accepted. `size` is injectable for the spec; by default
    it is read from the file."""
    executable = sys.executable if executable is None else executable
    version_info = tuple(sys.version_info[:2]) if version_info is None else tuple(version_info[:2])
    if not executable:
        return "sys.executable is empty: there is no interpreter to trust"
    if version_info < MIN_PYTHON:
        return "Python %d.%d is older than %d.%d" % (version_info + MIN_PYTHON)
    lowered = str(executable).replace("\\", "/").lower()
    if "/windowsapps/" in lowered and "pythonsoftwarefoundation" not in lowered:
        return "%s is the Microsoft Store stub (an app-execution alias), not an interpreter" % executable
    if size is None:
        try:
            size = Path(executable).stat().st_size
        except OSError:
            size = None  # an alias can fail to stat; the stub path rule above already caught the known shape
    if size == 0 and not ("/windowsapps/" in lowered and "pythonsoftwarefoundation" in lowered):
        return "%s is an empty file, not an interpreter" % executable
    return None

PLAN_HEAD = "2395719046875a06c4986e4e8a949a448bfcb9d3"
F016_CODEC_QUOTE = "tolerância 16, maior mudança 69 — discrimina"  # the text of F016 (effects.md, Export) the codec limit is imported from
PIN = "d40ac3f7c0"
TIP = "365579076782c1fc19c6464f3bc0199b30c0ab8f"
SE = "packages/scene-engine/src/"

# ---- the origin sources -----------------------------------------------------------------------------
# (path under SE, role). `ATTESTED` is what the Mac coordinator reported comparing 21 files of the pin
# with the tip by git object ids: (pin blob, tip blob). The Windows author could read the TIP only (the
# pin commit is absent from that clone), so the tip blob and its sha256 are re-checked here and the pin
# blob is an attestation. TIP_ONLY are files this oracle also reads at the tip that nobody compared with
# the pin: they say so in the manifest.

SOURCES = [
    ("fx/fx-windows.ts", "transitionWindow, maxTransitionFrames: the window arithmetic"),
    ("fx/fx-explain-transitions.ts", "transitionIssues: codes, order, handle lacks, messages, overlap"),
    ("fx/fx-normalize-transitions.ts", "legacy dissolve read as a crossDissolve (S6, not implemented here)"),
    ("fx/fx-frame-decks.ts", "shotClipFrame: the handle-edge rule of shot clips"),
    ("fx/fx-frame-derived.ts", "which transitions draw (hidden skipped) and their windows"),
    ("fx/fx-explain-budget.ts", "FX_PASS_BUDGET and the transition's cost (read; not modelled in S0)"),
    ("fx/fx-command-transitions.ts", "setTransition: defaults of a NEW transition"),
    ("fx/resolve-fx-frame.ts", "buildTransition: progress = ease((frameInWindow + 0.5) / D)"),
    ("fx/fx-schema.ts", "FxTransition shape, FX_RESERVED_IDS, FX_MAX_SEED, FX_ERROR_CODES"),
    ("fx/catalog/crossDissolve.ts", "crossDissolve: family transition, curve enum, typical 24"),
    ("fx/catalog/fx-effect-meta.ts", "the shared `intensity` 0..1 default 1 and checkParamValue"),
    ("edit/resolve-shot-at-offset.ts", "resolveShotAtOffset: independent instance and camera clamps"),
    ("camera/easing.ts", "applyEasing and the three easings"),
    ("runtime/fx/fx-compositor.ts", "amount = clampUnit(intensity) for a transition; progress is passed through"),
    ("runtime/fx/shaders/crossDissolve.ts", "the mix: plain, curved, smooth, amount (GLSL)"),
    ("fx/fx-explain-handles.spec.ts", "the origin's own handle-lack messages (origin-spec-vector)"),
    ("fx/resolve-fx-frame-transitions.spec.ts", "the origin's own window/progress/deck-frame vectors"),
    ("validate/edit-joins.ts", "a dissolve join is not pose-checked (S6)"),
    ("schema/render-bundle.schema.ts", "alphaMode enum and the legacy dissolve shape (S6)"),
    ("fx/fx-explain-legacy-dissolve.spec.ts", "UNSUPPORTED_ALPHA on a legacy dissolve (origin spec)"),
    ("fx/fx-explain.ts", "UNSUPPORTED_ALPHA: a non-opaque profile with any transition or clip"),
]

ATTESTED = {
    "fx/fx-windows.ts": ("bcf38615372e8cf830ea21a87159e390f3a7674f", "bcf38615372e8cf830ea21a87159e390f3a7674f"),
    "fx/fx-explain-transitions.ts": ("f075eec56b82012ad9facfacf68cad8ecdb36805", "f075eec56b82012ad9facfacf68cad8ecdb36805"),
    "fx/fx-normalize-transitions.ts": ("8c20747634919afdcc793274816d8228d76921a7", "8c20747634919afdcc793274816d8228d76921a7"),
    "fx/fx-frame-decks.ts": ("3adf58c6510c13442f47a950e493ed196ab3a428", "3adf58c6510c13442f47a950e493ed196ab3a428"),
    "fx/fx-frame-derived.ts": ("db526541051b667e4fed421464224e0c91ad9982", "db526541051b667e4fed421464224e0c91ad9982"),
    "fx/fx-explain-budget.ts": ("02aafbd677f311479f83a64ce6be76894d431347", "02aafbd677f311479f83a64ce6be76894d431347"),
    "fx/fx-command-transitions.ts": ("bde079b7fe26b3c25fb0ef6fe931f2233a1a9b8e", "bde079b7fe26b3c25fb0ef6fe931f2233a1a9b8e"),
    "fx/resolve-fx-frame.ts": ("44c4c11f8725450e9c05bf835a273c57bcca8e0c", "44c4c11f8725450e9c05bf835a273c57bcca8e0c"),
    "fx/fx-schema.ts": ("5ce474989c62eb21c7c3ecff56c8a78d824dc038", "5c85ce56f21fe22bf7c53314386b0c2124e20361"),
    "fx/catalog/crossDissolve.ts": ("f202c14a66b24429284fbf7f7eafa1d3c04cd1c7", "f202c14a66b24429284fbf7f7eafa1d3c04cd1c7"),
    "fx/catalog/fx-effect-meta.ts": ("999c2c67a77832349bcbb0209c62ac19d9d12cfe", "999c2c67a77832349bcbb0209c62ac19d9d12cfe"),
    "edit/resolve-shot-at-offset.ts": ("0851da51e1cc8c6a8961f4cd3dcc503382417cdb", "0851da51e1cc8c6a8961f4cd3dcc503382417cdb"),
    "camera/easing.ts": ("6d660fc81a3bd6c3adf9fbd6df44735b8f78d56b", "6d660fc81a3bd6c3adf9fbd6df44735b8f78d56b"),
    "runtime/fx/fx-compositor.ts": ("17ab35a3d6c8c4624e6abd56dc06bd7f30fab037", "17ab35a3d6c8c4624e6abd56dc06bd7f30fab037"),
    "runtime/fx/shaders/crossDissolve.ts": ("43b5427bd296c1d267744b5669ca3056a95fbb1f", "43b5427bd296c1d267744b5669ca3056a95fbb1f"),
    "fx/fx-explain-handles.spec.ts": ("e49dba5b24788fa7333120da8d9d0b662489ad11", "e49dba5b24788fa7333120da8d9d0b662489ad11"),
    "fx/resolve-fx-frame-transitions.spec.ts": ("adc745cb4db1c016fcb6440e65cc25fdbc3802b2", "adc745cb4db1c016fcb6440e65cc25fdbc3802b2"),
    "validate/edit-joins.ts": ("98b08e5b311b38e42a1a31048d758e19b94e3b24", "98b08e5b311b38e42a1a31048d758e19b94e3b24"),
    "schema/render-bundle.schema.ts": ("5bdbb4974995fb8797a4839a1bda19da2d22b3ac", "11c3a49dc9963abaab5ed477fea54a299b835ec0"),
    "fx/fx-explain-legacy-dissolve.spec.ts": ("5dd03d895c0fb70f89b1b1ebdcaf742d2bec5063", "5dd03d895c0fb70f89b1b1ebdcaf742d2bec5063"),
    "fx/fx-explain.ts": ("547dd07a4f5fb05d5342e134262f8e205394be55", "547dd07a4f5fb05d5342e134262f8e205394be55"),
}

# The files below were read at the tip only; the pin was not compared for them.
TIP_ONLY = [
    ("fx/fx-explain-effects.ts", "effectIssues/paramIssues: UNKNOWN_EFFECT vs WRONG_FAMILY, param messages"),
    ("fx/fx-explain-context.ts", "shotLength; shots sorted by start frame"),
    ("fx/fx-ids.ts", "RESERVED_ID (lowercased compare) and DUPLICATE_ID"),
    ("narration/narration-schema.ts", "idSchema: ^[a-z0-9][a-z0-9_-]{0,63}$ (case-insensitive)"),
    ("fx/catalog/vignette.ts", "vignette is family `look` (the WRONG_FAMILY case)"),
]

# What a re-read of each tip blob must give: (tip blob id, sha256, bytes). Refresh with `digests`.
TIP_DIGEST = {
    "camera/easing.ts": ("6d660fc81a3bd6c3adf9fbd6df44735b8f78d56b", "61acdcfbca0f7224696f370a1b77165210e7b36654c2f7ded3f7e82c71ed3abd", 952),
    "edit/resolve-shot-at-offset.ts": ("0851da51e1cc8c6a8961f4cd3dcc503382417cdb", "18e82cc298a29e3386c5328364503f31e75a26864a990542c045697c290ab047", 3275),
    "fx/catalog/crossDissolve.ts": ("f202c14a66b24429284fbf7f7eafa1d3c04cd1c7", "80cbc7310badd8bffbef931d2d6edbb3917745b754a4531657a7a7b15a50d0c4", 1287),
    "fx/catalog/fx-effect-meta.ts": ("999c2c67a77832349bcbb0209c62ac19d9d12cfe", "3e5e7d9b63896a61df5009ddc117987227d39adedf9214efd5edd60fb1146a19", 14960),
    "fx/catalog/vignette.ts": ("95b137d2a386f6f75dc5bbdb1a89255ac6410684", "6bd55eb8c9c129e90ca0cd7a1e515090100cb5511dbabb12b401ca867a2255c8", 1883),
    "fx/fx-command-transitions.ts": ("bde079b7fe26b3c25fb0ef6fe931f2233a1a9b8e", "fb1006ff87527afb0696eff165d1055976a68d5416fd00d927eeeafdb354749b", 7646),
    "fx/fx-explain-budget.ts": ("02aafbd677f311479f83a64ce6be76894d431347", "9ff1c564d2ac8f7caf59935d177f322f35ebe9fdfaa3ddb3f0d4d50a54eb190a", 7121),
    "fx/fx-explain-context.ts": ("0c4c1535647b6723e35c22a3338519679e45482b", "aae067fbe54239e4f62138b0440d3ca7e0b849965420e39a0068dd78f6f570be", 3989),
    "fx/fx-explain-effects.ts": ("376f07862b508702ed1f979e33bc371406dab0d8", "fb0c12fd6f1eaffa67e2fc5cae4c1991752b492833a649b4e560609e7f9a948e", 4475),
    "fx/fx-explain-handles.spec.ts": ("e49dba5b24788fa7333120da8d9d0b662489ad11", "3b6c852858c1a1da4f29bfb4efe0b8327fc80a2084774352aa894557e1b10844", 7467),
    "fx/fx-explain-legacy-dissolve.spec.ts": ("5dd03d895c0fb70f89b1b1ebdcaf742d2bec5063", "175ee0438271f93e2fcf5f16b97de6a81ce365612df36895b5faecfcc344dce7", 7056),
    "fx/fx-explain-transitions.ts": ("f075eec56b82012ad9facfacf68cad8ecdb36805", "6aa0dd88253702b91a13cc5a57c4e977257d79f272c791a6c2f0e1123acfaabf", 16892),
    "fx/fx-explain.ts": ("547dd07a4f5fb05d5342e134262f8e205394be55", "71e0285fd359d7094a4d5454415802ad7ce529551daa1466d7e8b9172c186f1c", 6247),
    "fx/fx-frame-decks.ts": ("3adf58c6510c13442f47a950e493ed196ab3a428", "dedc9994f62d500d123e4d6bb2ee31a3c15f0e93bebbdb2d1f29dc1e36db07a8", 8796),
    "fx/fx-frame-derived.ts": ("db526541051b667e4fed421464224e0c91ad9982", "d78b3fd817f8f216a10946cfa91189d590c04dea6e1e28ee8b90ec9db30d8b7d", 3193),
    "fx/fx-ids.ts": ("cf86e011094d6718b6c545aac4bda1b0b4c2c3f7", "43853b3def88ae8d8b83406c17e2fe6634b8a493335ab9377d2ca915021231e1", 2846),
    "fx/fx-normalize-transitions.ts": ("8c20747634919afdcc793274816d8228d76921a7", "a8e5a5763a4eee1075b5d360f10e341d86af859e12828a6b60049fe96c547767", 2252),
    "fx/fx-schema.ts": ("5c85ce56f21fe22bf7c53314386b0c2124e20361", "4044611e48418de75cd05dd95ae27ae777568156fda92fc6cb6e6b53b8e2a21d", 8329),
    "fx/fx-windows.ts": ("bcf38615372e8cf830ea21a87159e390f3a7674f", "55db48362fc406695cc2ff08e29cf79cfa7563f851b2f88e39c0a726d70c4bd3", 2524),
    "fx/resolve-fx-frame-transitions.spec.ts": ("adc745cb4db1c016fcb6440e65cc25fdbc3802b2", "10e9c31b5feb0260f04b26f62e9f9cbc7d5c2a021323d27513d69bca5962c240", 7612),
    "fx/resolve-fx-frame.ts": ("44c4c11f8725450e9c05bf835a273c57bcca8e0c", "20fac1cb27ac7d2cb77e2f93f2163312525057ac2defb2ff1bf986c5d015ea18", 4953),
    "narration/narration-schema.ts": ("b9aee27cb16ade8599276e71556843e379586357", "0af031103fcef0eacb761d14342cebebff976253556a783baeaa2123e6fec461", 4093),
    "runtime/fx/fx-compositor.ts": ("17ab35a3d6c8c4624e6abd56dc06bd7f30fab037", "ae9c446fbd02045b75723e75118e46bcb6b9b25815c216155575cba358753729", 12148),
    "runtime/fx/shaders/crossDissolve.ts": ("43b5427bd296c1d267744b5669ca3056a95fbb1f", "25cdcd1be3f4ccf209ce188dd030bbde3a534014457c11bd2bdc5d988449e62e", 939),
    "schema/render-bundle.schema.ts": ("11c3a49dc9963abaab5ed477fea54a299b835ec0", "21386068a759a8d7ef3f913146e5a6e520adf004260b89f48bb2afa20123e59b", 22781),
    "validate/edit-joins.ts": ("98b08e5b311b38e42a1a31048d758e19b94e3b24", "cfcc1449f42614e48da39f650cef799452c038bacc4cabeb6e6f534a41f64b82", 7004),
}

TARGET_FILES = {
    "parse": "crates/core/src/directed_scene/parse.rs",
    "export_test": "crates/render/tests/scene_fx_export.rs",
    "bundle": "docs/migration/fixtures/bundles/valid-cut-instance-offsets.bundle.json",
    "legacy_expected": "docs/migration/fixtures/expected/invalid-dissolve-transition.expected.json",
    "legacy_bundle": "docs/migration/fixtures/bundles/invalid-dissolve-transition.bundle.json",
    "effects_doc": "docs/features/effects.md",
}


# ---- origin text the transcription depends on -------------------------------------------------------


def check_against(texts):
    """Lines and numbers transcribed here must be the origin's text. Returns problems."""
    problems = []

    def need(path, lines):
        text = texts.get(path, "").replace("\r\n", "\n")
        for line in lines:
            if line not in text:
                problems.append("%s: not found in the origin text: %s" % (path.rsplit("/", 1)[-1], line.replace("\n", "\\n")))

    need("fx/fx-windows.ts", [
        "startFrame = cutFrame - Math.floor(durationFrames / 2)",
        "startFrame = cutFrame - durationFrames",
        "const endFrameExclusive = startFrame + durationFrames",
        "Math.min(endFrameExclusive, cutFrame) - startFrame,",
        "framesAfterCut: durationFrames - framesBeforeCut,",
        "return toFrames",
        "return fromFrames",
        "return Math.min(2 * fromFrames + 1, 2 * toFrames)",
    ])
    need("fx/resolve-fx-frame.ts", [
        "const frameInWindow = editFrame - windowStart",
        "progress: applyEasing(\n      (frameInWindow + 0.5) / transition.durationFrames,\n      transition.easing,\n    ),",
        "editFrame < window.startFrame ||\n      editFrame >= window.endFrameExclusive",
    ])
    need("camera/easing.ts", [
        '"linear",\n  "easeInOutCubic",\n  "easeOutCubic",',
        "return 4 * t * t * t",
        "const p = -2 * t + 2",
        "return 1 - (p * p * p) / 2",
        "const p = 1 - t\n  return 1 - p * p * p",
        "throw new Error(`Unknown camera easing: ${easingId}`)",
    ])
    need("runtime/fx/shaders/crossDissolve.ts", [
        "float shaped = mix(uProgress, smoothstep(0.0, 1.0, uProgress), uSmooth);",
        "vec3 plain = mix(a, b, uProgress);",
        "vec3 curved = mix(a, b, shaped);",
        "gl_FragColor = vec4(mix(plain, curved, uAmount), 1.0);",
        'uSmooth: Number(stringParam(params, "curve") === "smooth"),',
    ])
    need("runtime/fx/fx-compositor.ts", ['amount: clampUnit(numberParam(transition.params, "intensity")),'])
    need("fx/catalog/crossDissolve.ts", [
        'family: "transition",',
        'values: ["linear", "smooth"],\n      default: "smooth",',
        "typicalDurationFrames: 24,",
        "needs: [\"two-inputs\"],",
    ])
    need("fx/catalog/fx-effect-meta.ts", [
        'const INTENSITY_PARAM: FxParamDef = {\n  id: "intensity",\n  kind: "number",\n  min: 0,\n  max: 1,\n  default: 1,\n  step: 0.01,\n  unit: "ratio",\n}',
        "return `number ${def.min}..${def.max}, default ${def.default}${unitSuffix(def.unit)}`",
        "return `one of ${def.values.join(\"|\")}, default ${def.default}`",
        "Number.isFinite(value) && value >= min && value <= max",
        "message: `Parameter ${def.id} has the wrong type`,",
        "message: `Parameter ${def.id} is ${got}, outside what it accepts`,",
    ])
    need("edit/resolve-shot-at-offset.ts", [
        "const frame = Math.max(0, Math.min(frames - 1, wanted))",
        "return { frame: 0, held: true }",
        "shot.instanceStartFrame + deltaFrames,",
        "shot.cameraInFrame + deltaFrames,",
        "editFrame: shot.editRange.startFrame + deltaFrames,",
        "return { frame, held: frame !== wanted }",
    ])
    need("fx/fx-frame-decks.ts", [
        "if (clipCovers(clip, editFrame)) {\n    return editFrame\n  }",
        "editFrame < shot.editRange.startFrame &&\n    clip.startFrame === shot.editRange.startFrame",
        "return clip.startFrame",
        "editFrame >= shot.editRange.endFrameExclusive &&\n    clipEnd(clip) === shot.editRange.endFrameExclusive",
        "return clipEnd(clip) - 1",
        "editFrame >= clip.startFrame && editFrame < clipEnd(clip)",
    ])
    need("fx/fx-frame-derived.ts", ["if (transition.hidden) {\n      continue\n    }"])
    need("fx/fx-explain-transitions.ts", [
        'code: "TRANSITION_NO_HANDLES",',
        "message: `Transition ${transition.id} (handles: ${transition.handles}): ${lacks.map(describeLack).join(\" and \")}`,",
        "return `${lack.shot.id} lacks ${most} frames ${lack.side} (${what})`",
        'let what = "camera clip"',
        "what = `scene instance ${lack.instance}, camera clip ${lack.camera}`",
        'what = "scene instance"',
        "const instance = Math.max(0, needed - instanceHandle)",
        "const camera = Math.max(0, needed - cameraHandle)",
        "head: shot.instanceStartFrame,",
        "instance.durationFrames - (shot.instanceStartFrame + shotLength(shot)),",
        "cameraClipFrames(clip) - (shot.cameraInFrame + shotLength(shot)),",
        "head: shot.cameraInFrame,",
        "fxCameraHandles(from, context)?.tail ?? Infinity",
        "fxCameraHandles(to, context)?.head ?? Infinity",
        'side: "after its end" | "before its start"',
        "give ${lack.shot.id} ${lack.instance} more frames of its scene instance ${lack.shot.sceneInstanceId} ${lack.side}",
        "extend its camera clip ${lack.shot.cameraClipId} by ${lack.camera} frames ${lack.side}",
        "Set handles to \"hold\" to freeze the first/last frame, shorten the transition, or ${lacks.flatMap(fixesOf).join(\"; \")}.",
        ' "refuse" asks for the same real frames as "use" (they differ in no calculation yet, scene-fx-decisions.md §18 #6); only "hold" accepts frozen ones.',
        'code: "UNKNOWN_KEYFRAME_EASING",',
        "message: `Transition ${transition.id} easing ${transition.easing} is not supported`,",
        "hint: `Use ${CAMERA_EASINGS.join(\", \")}.`,",
        'code: "UNKNOWN_SHOT",',
        "message: `Transition ${transition.id} names shot ${shot.id}, which does not exist`,",
        "hint: `Shots: ${context.shots.map((candidate) => candidate.id).join(\", \")}.`,",
        'code: "TRANSITION_NOT_AT_CUT",',
        "message: `Transition ${transition.id}: ${to.id} does not follow ${from.id} in the edit`,",
        "hint: `A transition sits on a cut, between a shot and the one after it${nextHint(next)}.`,",
        'return " (this is the last shot)"',
        "return ` (after it comes ${next.id})`",
        'const cutKey = `${from.id}|${to.id}`',
        'code: "TRANSITION_DUPLICATE",',
        "message: `Transitions ${first} and ${transition.id} are on the same cut (${from.id} to ${to.id})`,",
        'hint: "One transition per cut: setTransition replaces the one that is there.",',
        "message: `Transition ${transition.id} and the dissolve of shot ${to.id} are on the same cut`,",
        'if (!isLegacy && dissolve?.type === "dissolve") {',
        "which the effects layer already draws as a crossDissolve.",
        'code: "TRANSITION_OUT_OF_RANGE",',
        "if (transition.durationFrames < 1) {",
        "message: `Transition ${transition.id} lasts ${transition.durationFrames} frames`,",
        'hint: "A transition lasts at least 1 frame.",',
        "message: `Transition ${transition.id} (frames ${window.startFrame} to ${window.endFrameExclusive}) leaves the edit (0 to ${context.editDurationFrames})`,",
        'hint: "Shorten it or change its alignment so the window stays inside the edit.",',
        "window.startFrame < 0 ||\n      window.endFrameExclusive > context.editDurationFrames",
        "shotLength(from) < window.framesBeforeCut ||\n      shotLength(to) < window.framesAfterCut",
        'code: "TRANSITION_TOO_LONG",',
        "message: `Transition ${transition.id} needs ${window.framesBeforeCut} frames inside ${from.id} (${shotLength(from)}) and ${window.framesAfterCut} inside ${to.id} (${shotLength(to)})`,",
        "hint: `With alignment ${transition.alignment} the longest these two shots hold is ${most} frames.`,",
        'code: "TRANSITION_OVERLAP",',
        "if (after.window.startFrame < before.window.endFrameExclusive) {",
        "(a, b) => a.window.startFrame - b.window.startFrame,",
        "message: `Transitions ${before.id} (frames ${before.window.startFrame} to ${before.window.endFrameExclusive}) and ${after.id} (from frame ${after.window.startFrame}) overlap`,",
        'hint: "Shorten one of them or change its alignment: the windows of neighbouring cuts cannot share a frame.",',
    ])
    need("fx/fx-explain-handles.spec.ts", [
        '"s2 lacks 4 frames after its end (camera clip)"',
        '"s1 lacks 6 frames after its end (camera clip)"',
        '"s2 lacks 6 frames before its start"',
        '"(scene instance 6, camera clip 6)"',
        '"s2 lacks 6 frames before its start (scene instance)"',
        '"s3 lacks 8 frames before its start (camera clip)"',
        "expect(fxCameraHandles(shot(\"s2\"), context)).toEqual({ head: 0, tail: 48 })",
        "expect(fxShotHandles(shot(\"s1\"), context)).toEqual({ head: 20, tail: 132 })",
    ])
    need("fx/resolve-fx-frame-transitions.spec.ts", [
        "expect(state(41)).toBeNull()",
        "expect(state(42)?.frameInWindow).toBe(0)",
        "expect(state(53)?.frameInWindow).toBe(11)",
        "expect(state(54)).toBeNull()",
        "expect(state(42)?.progress).toBeCloseTo(0.5 / 12, 12)",
        "expect(state(48)?.progress).toBeCloseTo(6.5 / 12, 12)",
        "expect(state(53)?.progress).toBeCloseTo(11.5 / 12, 12)",
        "instanceFrame: 70,",
        "cameraClipFrame: 47,",
        "heldInstance: false,\n        heldCamera: true,",
        "instanceFrame: 2,\n        cameraClipFrame: 2,",
        "editFrame: 44,\n      instanceFrame: 0,\n      cameraClipFrame: 0,\n      heldInstance: true,\n      heldCamera: true,",
    ])
    need("fx/fx-explain.ts", [
        'if (!isEmpty && bundle.renderProfile.alphaMode !== "opaque") {',
        'code: "UNSUPPORTED_ALPHA",',
        "const isEmpty = transitions.count === 0 && !hasClips",
    ])
    need("fx/fx-explain-legacy-dissolve.spec.ts", [
        'it("is UNSUPPORTED_ALPHA when the profile is not opaque, because the compositor draws it opaque"',
        'it("leaves a straight-alpha bundle with no dissolve and no fx alone"',
    ])
    need("fx/fx-schema.ts", [
        'export const FX_RESERVED_IDS = ["track", "fx", "transition"] as const',
        "export const FX_MAX_SEED = 4_294_967_295",
        "const seedSchema = z.number().int().min(0).max(FX_MAX_SEED)",
        "hidden: z.literal(true).optional(),",
        "durationFrames: z.number().int(),",
        '"WRONG_FAMILY",',
        '"UNKNOWN_EFFECT",',
        '"UNSUPPORTED_ALPHA",',
    ])
    need("schema/render-bundle.schema.ts", [
        'alphaMode: z.enum(["straight", "premultiplied", "opaque"]),',
        'type: z.literal("dissolve"),\n    durationFrames: z.number().int().positive(),',
    ])
    need("fx/fx-command-transitions.ts", [
        'alignment: "center",',
        'handles: "hold",',
        'easing: "easeInOutCubic",',
    ])
    need("fx/fx-explain-budget.ts", ["export const FX_PASS_BUDGET = 8"])
    need("validate/edit-joins.ts", ['if (transition.type === "dissolve") {'])
    need("fx/fx-explain-effects.ts", [
        'code: "WRONG_FAMILY",',
        'if (what === "transition" && meta.family !== "transition") {',
        "message: `${what} ${id}: ${effect} ${reason}`,",
        "`is a ${meta.family} effect, not a transition`",
        '"A transition takes an effect of the transition family; a look goes on a fx track as a clip."',
        'code: "UNKNOWN_EFFECT",',
        "message: `${what} ${id}: there is no effect ${effect}`,",
        "message: `${owner.label}: ${meta.id} has no parameter ${name}`,",
        "hint: `Parameters of ${meta.id}: ${known}.`,",
        "message: `${owner.label}: ${problem.message}`,",
    ])
    need("fx/fx-explain-context.ts", [
        "endFrameExclusive - shot.editRange.startFrame",
        "(a, b) => a.editRange.startFrame - b.editRange.startFrame,",
    ])
    need("fx/fx-ids.ts", [
        "FX_RESERVED_IDS.some((reserved) => reserved === id.toLowerCase())",
        'code: "RESERVED_ID",',
        'code: "DUPLICATE_ID",',
    ])
    need("narration/narration-schema.ts", ["/^[a-z0-9][a-z0-9_-]{0,63}$/i"])
    need("fx/catalog/vignette.ts", ['id: "vignette",\n  family: "look",'])
    return problems


# ---- strict inputs (destination hardening; the codes are the oracle's, not product codes) ---------------

I64_MIN, I64_MAX = -(2**63), 2**63 - 1
SAFE_MAX = 2**53 - 1
PROGRESS_EXACT_LIMIT = 2**52  # (f + 0.5) is exact in a double for f < 2^52
SEED_MAX = 4294967295
ALIGNMENTS = ["center", "start", "end"]
HANDLE_MODES = ["use", "hold", "refuse"]
EASINGS = ["linear", "easeInOutCubic", "easeOutCubic"]
CURVES = ["linear", "smooth"]


class Refusal(Exception):
    """A typed refusal: the oracle never coerces, clamps an invalid window or saturates."""

    def __init__(self, code, detail=""):
        super().__init__("%s: %s" % (code, detail))
        self.code = code
        self.detail = detail


def strict_int(value, field):
    if isinstance(value, bool) or not isinstance(value, int):
        raise Refusal("ORACLE_NOT_AN_INTEGER", field)  # 12.5, 12.0, true, "12", null
    if value < I64_MIN or value > I64_MAX:
        raise Refusal("ORACLE_INTEGER_OUT_OF_I64", field)
    if abs(value) > SAFE_MAX:
        raise Refusal("ORACLE_INTEGER_NOT_SAFE", field)  # beyond 2^53 - 1: not a safe integer
    return value


def checked(value, op):
    """i64 arithmetic that refuses instead of wrapping or saturating."""
    if value < I64_MIN or value > I64_MAX:
        raise Refusal("ORACLE_ARITHMETIC_OVERFLOW", op)
    return value


def strict_seed(value):
    if isinstance(value, bool) or not isinstance(value, int):
        raise Refusal("ORACLE_BAD_SEED", "not an integer")
    if value < 0 or value > SEED_MAX:
        raise Refusal("ORACLE_BAD_SEED", "outside uint32")
    return value


def attempt(function, *args):
    """A JSON-friendly outcome of a function that may refuse."""
    try:
        return {"ok": True, "value": function(*args)}
    except Refusal as refusal:
        return {"ok": False, "code": refusal.code, "detail": refusal.detail}


# ---- the window (fx-windows.ts) ---------------------------------------------------------------------


def raw_window(cut, duration, alignment):
    """`transitionWindow`, unvalidated, as the origin writes it (floor division = Math.floor)."""
    start = cut
    if alignment == "center":
        start = cut - duration // 2
    if alignment == "end":
        start = cut - duration
    end = start + duration
    before = max(0, min(end, cut) - start)
    return {"start": start, "end": end, "before": before, "after": duration - before}


def max_transition_frames(alignment, from_frames, to_frames):
    if alignment == "start":
        return to_frames
    if alignment == "end":
        return from_frames
    return min(2 * from_frames + 1, 2 * to_frames)


def checked_window(cut, duration, alignment, edit_duration):
    """The window the destination may accept: strict integers, D checked against the edit BEFORE the
    arithmetic, checked arithmetic, and an invalid window REFUSED (never clamped into the edit)."""
    cut = strict_int(cut, "cut")
    duration = strict_int(duration, "durationFrames")
    edit_duration = strict_int(edit_duration, "editDurationFrames")
    if alignment not in ALIGNMENTS:
        raise Refusal("ORACLE_UNKNOWN_ALIGNMENT", str(alignment))
    if cut < 0 or edit_duration < 1:
        raise Refusal("ORACLE_BAD_EDIT", "cut or edit duration")
    if duration < 1:
        raise Refusal("TRANSITION_OUT_OF_RANGE", "a transition lasts at least 1 frame")
    if duration > edit_duration:
        raise Refusal("TRANSITION_OUT_OF_RANGE", "the duration exceeds the edit")
    window = raw_window(cut, duration, alignment)
    for key in ("start", "end", "before", "after"):
        checked(window[key], key)
    if window["start"] < 0 or window["end"] > edit_duration:
        raise Refusal("TRANSITION_OUT_OF_RANGE", "window [%d, %d) leaves the edit [0, %d)" % (window["start"], window["end"], edit_duration))
    return window


def available(total_frames, start, length):
    """Frames a clip/instance has AFTER the shot's last one (the tail): never negative (semantic clamp)."""
    return max(0, total_frames - (start + length))


# ---- progress and weight ----------------------------------------------------------------------------


def clamp01(value):
    if value <= 0:
        return 0.0
    if value >= 1:
        return 1.0
    return value


def apply_easing(t, name):
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
    raise Refusal("UNKNOWN_KEYFRAME_EASING", str(name))


def mix(x, y, a):
    """GLSL `mix`: x * (1 - a) + y * a."""
    return x * (1.0 - a) + y * a


def smoothstep01(x):
    t = clamp01(x)
    return t * t * (3.0 - 2.0 * t)


def raw_t(frame_in_window, duration):
    """t = (f - start + 0.5) / D, sampled at the middle of the frame."""
    return (frame_in_window + 0.5) / duration


def progress_of(frame_in_window, duration, easing):
    duration = strict_int(duration, "durationFrames")
    frame_in_window = strict_int(frame_in_window, "frameInWindow")
    if duration < 1 or duration > PROGRESS_EXACT_LIMIT:
        raise Refusal("ORACLE_PROGRESS_DURATION", str(duration))
    if frame_in_window < 0 or frame_in_window >= duration:
        raise Refusal("ORACLE_FRAME_NOT_IN_WINDOW", str(frame_in_window))
    return apply_easing(raw_t(frame_in_window, duration), easing)


def shaped_of(progress, curve):
    return mix(progress, smoothstep01(progress), 1.0 if curve == "smooth" else 0.0)


def weight_of(progress, curve, intensity):
    """The effective weight of deck B: mix(progress, shaped, amount). intensity 0 is the eased progress,
    never 0 and never the un-eased t; curve linear ignores intensity (shaped == progress)."""
    return mix(progress, shaped_of(progress, curve), clamp01(intensity))


# ---- colour: 8-bit decks, linear-light mix, IEC 61966-2-1 -------------------------------------------


def decode(code8):
    v = code8 / 255.0
    return v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4


def encode(linear):
    v = clamp01(linear)
    return v * 12.92 if v <= 0.0031308 else 1.055 * v ** (1.0 / 2.4) - 0.055


def quantize_input(unit):
    """A deck colour given as a unit float becomes an 8-bit code BEFORE the mix: floor(x * 255 + 0.5)."""
    if isinstance(unit, bool) or not isinstance(unit, (int, float)) or not math.isfinite(unit):
        raise Refusal("ORACLE_BAD_COLOUR", repr(unit))
    return int(min(255, max(0, math.floor(unit * 255.0 + 0.5))))


def quant8(encoded):
    """The output is rounded half-up to 8 bits: floor(x * 255 + 0.5)."""
    return int(min(255, max(0, math.floor(encoded * 255.0 + 0.5))))


def tie_margin(encoded):
    frac = encoded * 255.0 - math.floor(encoded * 255.0)
    return abs(frac - 0.5)


def r9(value):
    return round(value, 9) + 0.0


def mix_pixel(deck_a, deck_b, progress, curve, intensity):
    """The shader's three mixes on 8-bit sRGB decks, in linear light: plain, curved, then amount."""
    shaped = shaped_of(progress, curve)
    amount = clamp01(intensity)
    linear, unrounded, rgb8 = [], [], []
    for a8, b8 in zip(deck_a, deck_b):
        a, b = decode(a8), decode(b8)
        plain = mix(a, b, progress)
        curved = mix(a, b, shaped)
        out = mix(plain, curved, amount)
        encoded = encode(out)
        if tie_margin(encoded) < 1e-6:
            raise Refusal("ORACLE_TIE_MARGIN", "%r is too close to a rounding tie to be host independent" % encoded)
        linear.append(r9(out))
        unrounded.append(r9(encoded * 255.0))
        rgb8.append(quant8(encoded))
    return {"linear": linear, "unrounded_255": unrounded, "rgb8": rgb8, "alpha8": 255}


# ---- which frame of a deck is drawn (resolve-shot-at-offset.ts, fx-frame-decks.ts) ---------------------


def limit_to_frames(wanted, frames):
    if frames <= 0:
        return 0, True
    frame = max(0, min(frames - 1, wanted))
    return frame, frame != wanted


def resolve_shot_at_offset(shot, instance_frames, camera_frames, delta):
    """What `shot` shows `delta` frames after its own first frame; the two clamps are independent and the
    edit clock is the REAL one (light, weather and flicker run on it)."""
    instance, held_instance = limit_to_frames(shot["instance_start"] + delta, instance_frames)
    camera, held_camera = limit_to_frames(shot["camera_in"] + delta, camera_frames)
    return {
        "delta": delta,
        "edit_frame": shot["start"] + delta,
        "instance_frame": instance,
        "camera_clip_frame": camera,
        "held_instance": held_instance,
        "held_camera": held_camera,
    }


def shot_clip_frame(clip_start, clip_duration, shot_start, shot_end, edit_frame):
    """The frame of a `shot:X` clip that deck X draws at `edit_frame`, or None."""
    clip_end = clip_start + clip_duration
    if clip_start <= edit_frame < clip_end:
        return edit_frame
    if edit_frame < shot_start and clip_start == shot_start:
        return clip_start
    if edit_frame >= shot_end and clip_end == shot_end:
        return clip_end - 1
    return None


# ---- the catalog subset, the edit models and the ordered refusals (fx-explain-*.ts) --------------------

CATALOG = {
    "crossDissolve": {
        "family": "transition",
        "params": [
            {"id": "curve", "kind": "enum", "values": ["linear", "smooth"], "default": "smooth"},
            {"id": "intensity", "kind": "number", "min": 0, "max": 1, "default": 1, "unit": "ratio"},
        ],
    },
    "vignette": {"family": "look", "params": None},  # its parameters are not modelled: the oracle refuses to guess
}
RESERVED_IDS = ["track", "fx", "transition"]
ID_PATTERN = re.compile(r"[a-z0-9][a-z0-9_-]{0,63}", re.ASCII | re.IGNORECASE)
REFUSE_NOTE = ' "refuse" asks for the same real frames as "use" (they differ in no calculation yet, scene-fx-decisions.md §18 #6); only "hold" accepts frozen ones.'


def shot(shot_id, start, end, instance, instance_start, camera, camera_in, into=None):
    return {"id": shot_id, "start": start, "end": end, "instance": instance, "instance_start": instance_start, "camera": camera, "camera_in": camera_in, "into": into}


def edit_model(duration, shots, instances, cameras):
    return {"edit_duration": duration, "shots": shots, "instances": instances, "camera_frames": cameras}


def corpus_edit(facts):
    """`valid-cut-instance-offsets` as the repository ships it (read, never copied)."""
    return facts["corpus_edit"]


def spec_edit():
    """RECONSTRUCTED from the numbers the origin specs show (`fx-test-bundle.ts` was not read): three
    48-frame shots; only the numbers those specs pin are claimed."""
    return edit_model(
        144,
        [
            shot("s1", 0, 48, "inst1", 20, "cam1", 0),
            shot("s2", 48, 96, "inst2", 0, "cam2", 0),
            shot("s3", 96, 144, "inst3", 20, "cam2", 48),
        ],
        {"inst1": 200, "inst2": 200, "inst3": 200},
        {"cam1": 48, "cam2": 96},
    )


def with_changes(edit, cameras=None, shot_changes=None):
    cloned = json.loads(json.dumps(edit))
    cloned["camera_frames"].update(cameras or {})
    for shot_id, changes in (shot_changes or {}).items():
        for item in cloned["shots"]:
            if item["id"] == shot_id:
                item.update(changes)
    return cloned


def tr(transition_id="tr", effect="crossDissolve", from_id="s1", to_id="s2", duration=12, alignment="center",
       handles="hold", easing="easeInOutCubic", **extra):
    out = {"id": transition_id, "effect": effect, "fromShotId": from_id, "toShotId": to_id, "durationFrames": duration,
           "alignment": alignment, "handles": handles, "easing": easing}
    out.update(extra)
    return out


def shot_handles(item, edit):
    frames = edit["instances"].get(item["instance"])
    if frames is None:
        return None
    return {"head": item["instance_start"], "tail": available(frames, item["instance_start"], item["end"] - item["start"])}


def camera_handles(item, edit):
    frames = edit["camera_frames"].get(item["camera"])
    if frames is None:
        return None
    return {"head": item["camera_in"], "tail": available(frames, item["camera_in"], item["end"] - item["start"])}


def lack_of(item, side, needed, instance_handle, camera_handle):
    instance = max(0, needed - instance_handle)
    camera = max(0, needed - camera_handle)
    if instance == 0 and camera == 0:
        return None
    return {"shot": item, "side": side, "instance": instance, "camera": camera}


def describe_lack(lack):
    most = max(lack["instance"], lack["camera"])
    what = "camera clip"
    if lack["instance"] > 0 and lack["camera"] > 0:
        what = "scene instance %d, camera clip %d" % (lack["instance"], lack["camera"])
    elif lack["instance"] > 0:
        what = "scene instance"
    return "%s lacks %d frames %s (%s)" % (lack["shot"]["id"], most, lack["side"], what)


def fixes_of(lack):
    fixes = []
    if lack["instance"] > 0:
        fixes.append("give %s %d more frames of its scene instance %s %s" % (lack["shot"]["id"], lack["instance"], lack["shot"]["instance"], lack["side"]))
    if lack["camera"] > 0:
        fixes.append("extend its camera clip %s by %d frames %s" % (lack["shot"]["camera"], lack["camera"], lack["side"]))
    return fixes


def handle_issue(transition, from_shot, to_shot, window, edit):
    if transition["handles"] == "hold":
        return []
    from_handles, to_handles = shot_handles(from_shot, edit), shot_handles(to_shot, edit)
    if from_handles is None or to_handles is None:
        return []
    from_camera = camera_handles(from_shot, edit)
    to_camera = camera_handles(to_shot, edit)
    from_tail = math.inf if from_camera is None else from_camera["tail"]
    to_head = math.inf if to_camera is None else to_camera["head"]
    lacks = [
        lack_of(from_shot, "after its end", window["after"], from_handles["tail"], from_tail),
        lack_of(to_shot, "before its start", window["before"], to_handles["head"], to_head),
    ]
    lacks = [lack for lack in lacks if lack is not None]
    if not lacks:
        return []
    hint = 'Set handles to "hold" to freeze the first/last frame, shorten the transition, or %s.' % "; ".join(fix for lack in lacks for fix in fixes_of(lack))
    if transition["handles"] == "refuse":
        hint += REFUSE_NOTE
    return [{
        "code": "TRANSITION_NO_HANDLES",
        "transitionId": transition["id"],
        "message": "Transition %s (handles: %s): %s" % (transition["id"], transition["handles"], " and ".join(describe_lack(lack) for lack in lacks)),
        "hint": hint,
        "lacks": [{"shot": lack["shot"]["id"], "side": lack["side"], "instance": lack["instance"], "camera": lack["camera"]} for lack in lacks],
    }]


def describe_param(definition):
    if definition["kind"] == "number":
        return "number %s..%s, default %s %s" % (definition["min"], definition["max"], definition["default"], definition["unit"])
    return "one of %s, default %s" % ("|".join(definition["values"]), definition["default"])


def corpus_numeric_issue_text(value):
    """Match the origin's integral-float spelling for corpus values, not all ECMAScript formatting."""
    if isinstance(value, float) and math.isfinite(value) and value.is_integer() and abs(value) < 1e21:
        return str(int(value))
    return json.dumps(value)


def param_issues(transition, meta):
    issues, label = [], "Transition %s" % transition["id"]
    known = ", ".join(definition["id"] for definition in meta["params"])
    for name, value in (transition.get("params") or {}).items():
        definition = next((d for d in meta["params"] if d["id"] == name), None)
        if definition is None:
            issues.append({"code": "UNKNOWN_PARAM", "transitionId": transition["id"],
                           "message": "%s: crossDissolve has no parameter %s" % (label, name), "hint": "Parameters of crossDissolve: %s." % known})
            continue
        problem = None
        if definition["kind"] == "number":
            if isinstance(value, bool) or not isinstance(value, (int, float)):
                problem = ("PARAM_TYPE", "Parameter %s has the wrong type" % name)
            elif not (math.isfinite(value) and definition["min"] <= value <= definition["max"]):
                problem = ("PARAM_OUT_OF_RANGE", "Parameter %s is %s, outside what it accepts" % (name, corpus_numeric_issue_text(value)))
        else:
            if not isinstance(value, str):
                problem = ("PARAM_TYPE", "Parameter %s has the wrong type" % name)
            elif value not in definition["values"]:
                problem = ("PARAM_OUT_OF_RANGE", 'Parameter %s is "%s", outside what it accepts' % (name, value))
        if problem:
            issues.append({"code": problem[0], "transitionId": transition["id"], "message": "%s: %s" % (label, problem[1]),
                           "hint": "%s: %s." % (name, describe_param(definition))})
    return issues


def explain_transitions(edit, transitions):
    """`transitionIssues` of the origin on DECLARED transitions: codes in the origin's order, with its
    messages. `keys` and the parameters of a look are not modelled (the oracle refuses to guess)."""
    shots = sorted(edit["shots"], key=lambda item: item["start"])
    by_id = {item["id"]: item for item in shots}
    issues, windows, cuts = [], [], {}
    for transition in transitions:
        tid = transition["id"]
        if "keys" in transition:
            raise Refusal("ORACLE_KEYS_NOT_MODELLED", tid)
        meta = CATALOG.get(transition["effect"])
        if meta is None:
            issues.append({"code": "UNKNOWN_EFFECT", "transitionId": tid, "message": "transition %s: there is no effect %s" % (tid, transition["effect"])})
        elif meta["family"] != "transition":
            issues.append({"code": "WRONG_FAMILY", "transitionId": tid,
                           "message": "transition %s: %s is a %s effect, not a transition" % (tid, transition["effect"], meta["family"]),
                           "hint": "A transition takes an effect of the transition family; a look goes on a fx track as a clip."})
            if transition.get("params"):
                raise Refusal("ORACLE_LOOK_PARAMS_NOT_MODELLED", tid)
        else:
            issues.extend(param_issues(transition, meta))
        if transition["easing"] not in EASINGS:
            issues.append({"code": "UNKNOWN_KEYFRAME_EASING", "transitionId": tid,
                           "message": "Transition %s easing %s is not supported" % (tid, transition["easing"]),
                           "hint": "Use %s." % ", ".join(EASINGS)})
        from_shot, to_shot = by_id.get(transition["fromShotId"]), by_id.get(transition["toShotId"])
        for wanted, found in ((transition["fromShotId"], from_shot), (transition["toShotId"], to_shot)):
            if found is None:
                issues.append({"code": "UNKNOWN_SHOT", "transitionId": tid,
                               "message": "Transition %s names shot %s, which does not exist" % (tid, wanted),
                               "hint": "Shots: %s." % ", ".join(item["id"] for item in shots)})
        if from_shot is None or to_shot is None:
            continue
        index = shots.index(from_shot)
        following = shots[index + 1] if index + 1 < len(shots) else None
        if following is None or following["id"] != to_shot["id"]:
            tail = " (this is the last shot)" if following is None else " (after it comes %s)" % following["id"]
            issues.append({"code": "TRANSITION_NOT_AT_CUT", "transitionId": tid,
                           "message": "Transition %s: %s does not follow %s in the edit" % (tid, to_shot["id"], from_shot["id"]),
                           "hint": "A transition sits on a cut, between a shot and the one after it%s." % tail})
            continue
        cut_key = "%s|%s" % (from_shot["id"], to_shot["id"])
        first = cuts.get(cut_key)
        if first:
            issues.append({"code": "TRANSITION_DUPLICATE", "transitionId": tid,
                           "message": "Transitions %s and %s are on the same cut (%s to %s)" % (first, tid, from_shot["id"], to_shot["id"]),
                           "hint": "One transition per cut: setTransition replaces the one that is there."})
        cuts[cut_key] = tid
        dissolve = to_shot.get("into")
        if dissolve and dissolve.get("type") == "dissolve":
            issues.append({"code": "TRANSITION_DUPLICATE", "transitionId": tid,
                           "message": "Transition %s and the dissolve of shot %s are on the same cut" % (tid, to_shot["id"]),
                           "hint": ("Shot %s still has a legacy dissolve of %d frames on this cut (`transitionInto`), which the effects layer already draws as a crossDissolve. "
                                    "The effects layer does not edit it: in render-bundle.json change `transitionInto` of shot %s to {\"type\":\"cut\",\"reason\":\"the transition draws this cut\"} "
                                    "and set the transition again, or keep the dissolve and leave the cut alone. One transition per cut.") % (to_shot["id"], dissolve["duration"], to_shot["id"])})
        duration = transition["durationFrames"]
        if duration < 1:
            issues.append({"code": "TRANSITION_OUT_OF_RANGE", "transitionId": tid,
                           "message": "Transition %s lasts %d frames" % (tid, duration), "hint": "A transition lasts at least 1 frame."})
            continue
        window = raw_window(to_shot["start"], duration, transition["alignment"])
        windows.append({"id": tid, "window": window})
        if window["start"] < 0 or window["end"] > edit["edit_duration"]:
            issues.append({"code": "TRANSITION_OUT_OF_RANGE", "transitionId": tid,
                           "message": "Transition %s (frames %d to %d) leaves the edit (0 to %d)" % (tid, window["start"], window["end"], edit["edit_duration"]),
                           "hint": "Shorten it or change its alignment so the window stays inside the edit."})
        from_length, to_length = from_shot["end"] - from_shot["start"], to_shot["end"] - to_shot["start"]
        if from_length < window["before"] or to_length < window["after"]:
            most = max_transition_frames(transition["alignment"], from_length, to_length)
            issues.append({"code": "TRANSITION_TOO_LONG", "transitionId": tid,
                           "message": "Transition %s needs %d frames inside %s (%d) and %d inside %s (%d)" % (tid, window["before"], from_shot["id"], from_length, window["after"], to_shot["id"], to_length),
                           "hint": "With alignment %s the longest these two shots hold is %d frames." % (transition["alignment"], most)})
        issues.extend(handle_issue(transition, from_shot, to_shot, window, edit))
    ordered = sorted(windows, key=lambda item: item["window"]["start"])
    for index in range(1, len(ordered)):
        before, after = ordered[index - 1], ordered[index]
        if after["window"]["start"] < before["window"]["end"]:
            issues.append({"code": "TRANSITION_OVERLAP", "transitionId": after["id"],
                           "message": "Transitions %s (frames %d to %d) and %s (from frame %d) overlap" % (before["id"], before["window"]["start"], before["window"]["end"], after["id"], after["window"]["start"]),
                           "hint": "Shorten one of them or change its alignment: the windows of neighbouring cuts cannot share a frame."})
    return issues


def id_problems(identifier, taken):
    """fx-ids.ts: RESERVED_ID (lowercased compare) first, then DUPLICATE_ID; the pattern is the schema's."""
    problems = []
    if not isinstance(identifier, str) or ID_PATTERN.fullmatch(identifier) is None:
        problems.append("ID_PATTERN")
    if isinstance(identifier, str) and identifier.lower() in RESERVED_IDS:
        problems.append("RESERVED_ID")
    if identifier in taken:
        problems.append("DUPLICATE_ID")
    return problems


def origin_alpha_verdict(alpha_mode, transitions_drawn, has_clips):
    """fx-explain.ts: any transition (legacy dissolves included) or clip on a non-opaque profile."""
    if not (transitions_drawn == 0 and not has_clips) and alpha_mode != "opaque":
        return {"verdict": "refused", "code": "UNSUPPORTED_ALPHA"}
    return {"verdict": "accepted"}


# ---- tolerances: registered BEFORE any pixel of the destination was produced ------------------------------

TOLERANCES = {
    "status": "pre-registered before any pixel was produced; design review pending (A validar); no pixel validated; no GPU measured",
    "delta_convention": "delta = implementation 8-bit code minus the oracle's 8-bit code, per channel, signed",
    "failure_policy": "an excess is a FAIL; a tolerance is never retuned after a result",
    "monitor_and_render": {
        "max_abs_delta_8bit": 1,
        "bias_limit": {
            "max_abs_mean_signed_delta_8bit": 0.25,
            "min_probes_per_channel": 24,
            "rationale": (
                "Per-pixel 1/255 alone lets a systematic mistake through: an implementation that truncates instead of rounding is off by exactly one code on "
                "about half of the probes and passes a max-delta-1 gate. Its mean signed delta is about -0.5 (about -0.4 to -0.5 here). A correct "
                "implementation of this oracle's model differs only where the exact value sits within float error of a rounding tie, symmetrically, so its "
                "mean is near 0. The limit is half of the truncation bias, far from both."),
        },
        "valid_only_if": [
            "decks are 8-bit sRGB codes before the mix (the oracle quantizes colour inputs first: floor(x * 255 + 0.5))",
            "the output is rounded half-up to 8 bits and its alpha is 255",
            "the probe set has at least min_probes_per_channel probes for the channel",
        ],
        "not_claimed": "the origin mixes half-float linear targets; the destination mixes 8-bit decks. This gate compares the destination with this oracle, not with the origin's pixels.",
    },
    "export_h264": {
        "max_abs_delta_8bit": 16,
        "source": "docs/features/effects.md (F016, Export), reconferred on 2026-11-14, and crates/render/tests/scene_fx_export.rs const CODEC_TOLERANCE (the limit F016/F062 export proofs already use)",
        "f016_quote": F016_CODEC_QUOTE,
        "rationale": "Codec quantization and chroma subsampling (an H.264 CRF 18 round trip on both sides of the comparison, on top of 8-bit rounding). The limit is imported, not derived from a measurement of this feature.",
        "discrimination_requirement": "the biggest change a mutant produces must exceed the limit (F016: biggest change 69 over 16); a limit that would not discriminate is reopened as a contract amendment, never retuned after a result",
        "not_used_as_tolerance": "the worst delta observed in earlier export runs (7/255, docs/features/effects.md) is an observation, not a criterion",
    },
    "plan_ceilings": {
        "status": "pre-registered by S0 as contract (plan 5.3), NOT measured; A validar: S1b reports the real plan sizes of the corpus and reopens a number only as a documented contract amendment",
        "MAX_TRANSITION_PLAN_BYTES": 4194304,
        "MAX_TRANSITION_PLAN_ITEMS": 20000,
        "TRANSITION_PLAN_MAX_DEPTH": 1,
        "items_are": "nodes + layers + splats + passes + resources + lights of BOTH decks, summed",
        "same_in": "Rust ScenePlan::from_json_str and runtime parseTransition",
        "refusal": "invalid with the JSON path; a plan WITHOUT `transition` has no new ceiling (its bytes do not change)",
        "rationale": "Order-of-magnitude bounds chosen WITHOUT a measurement of a real plan (none exists yet for a two-deck plan): they exist to refuse hostile input with a typed error before memory or stack is exhausted, not to constrain authored scenes. That is why the status is A validar: S1b measures the corpus plans and reports how far below the ceilings they sit.",
    },
    "oracle_internal": {"progress_symmetry_abs": 1e-12, "float_decimals_for_pow_values": 9, "min_distance_from_a_rounding_tie": 1e-6},
}

# ---- hand vectors, independent of the functions above (checked at build time and in the spec) --------------

FIXED_VECTORS = {
    "windows_cut48": [
        ["center", 12, 42, 54], ["center", 5, 46, 51], ["center", 1, 48, 49], ["start", 12, 48, 60], ["end", 12, 36, 48],
    ],
    # table 6.2 of the plan, 6 decimals: k, progress after easeInOutCubic, weight after smoothstep (D = 12)
    "d12_cubic_smooth": [
        [0, 0.000289, 0.0], [1, 0.007813, 0.000182], [3, 0.099248, 0.027595], [5, 0.385127, 0.330723], [6, 0.614873, 0.669277], [11, 0.999711, 1.0],
    ],
    # k = 5, D = 12, cubic, smooth, channel 200 -> 40: intensity -> (unrounded, code)
    "k5_channel_200_40": [[0.0, 162.4061, 162], [0.5, 165.4320, 165], [1.0, 168.3881, 168]],
    # the same frame with curve = linear: every intensity gives the eased progress, so the three are equal
    "k5_channel_200_40_linear": [[0.0, 162.4061, 162], [0.5, 162.4061, 162], [1.0, 162.4061, 162]],
    # plan 6.2 colour vector (k = 5, defaults): decoded A, decoded B, weight, linear mix, encoded value
    "k5_colour_steps": {"decoded_a": 0.577580, "decoded_b": 0.021219, "weight": 0.330723, "linear_mix": 0.393579, "encoded_255": 168.39, "code": 168},
}


def self_check():
    """The functions above must reproduce the hand vectors. Returns problems (never an assert)."""
    problems = []
    for alignment, duration, start, end in FIXED_VECTORS["windows_cut48"]:
        window = raw_window(48, duration, alignment)
        if (window["start"], window["end"]) != (start, end):
            problems.append("window %s/%d is %r, not [%d, %d)" % (alignment, duration, window, start, end))
    for k, progress, weight in FIXED_VECTORS["d12_cubic_smooth"]:
        p = progress_of(k, 12, "easeInOutCubic")
        w = weight_of(p, "smooth", 1.0)
        half_unit = 5e-7 + 1e-12  # the plan prints 6 decimals: the value may be off by half the last digit, inclusive
        if abs(p - progress) > half_unit or abs(w - weight) > half_unit:
            problems.append("D=12 k=%d: progress %.9f weight %.9f differ from %.6f / %.6f" % (k, p, w, progress, weight))
    p = progress_of(3, 12, "easeInOutCubic")
    if abs(p - 0.099247685185) > 1e-12:
        problems.append("k=3 D=12 progress is %.12f, not 0.099247685185" % p)
    for amount, unrounded, code in FIXED_VECTORS["k5_channel_200_40"]:
        pixel = mix_pixel([200, 0, 0], [40, 0, 0], progress_of(5, 12, "easeInOutCubic"), "smooth", amount)
        if abs(pixel["unrounded_255"][0] - unrounded) > 5e-4 or pixel["rgb8"][0] != code:
            problems.append("k=5 intensity %s: %r, expected %.4f -> %d" % (amount, pixel, unrounded, code))
    for amount, unrounded, code in FIXED_VECTORS["k5_channel_200_40_linear"]:
        pixel = mix_pixel([200, 0, 0], [40, 0, 0], progress_of(5, 12, "easeInOutCubic"), "linear", amount)
        if abs(pixel["unrounded_255"][0] - unrounded) > 5e-4 or pixel["rgb8"][0] != code:
            problems.append("k=5 linear intensity %s: %r, expected %.4f -> %d" % (amount, pixel, unrounded, code))
    steps = FIXED_VECTORS["k5_colour_steps"]
    weight = weight_of(progress_of(5, 12, "easeInOutCubic"), "smooth", 1.0)
    linear_mix = mix(decode(200), decode(40), weight)
    for label, got, want, tolerance in (("decoded A", decode(200), steps["decoded_a"], 5e-7 + 1e-12), ("decoded B", decode(40), steps["decoded_b"], 5e-7 + 1e-12),
                                        ("weight", weight, steps["weight"], 5e-7 + 1e-12), ("linear mix", linear_mix, steps["linear_mix"], 5e-7 + 1e-12),
                                        ("encoded", encode(linear_mix) * 255.0, steps["encoded_255"], 5e-3 + 1e-9)):
        if abs(got - want) > tolerance:
            problems.append("colour step %s is %.9f, the plan prints %s" % (label, got, want))
    return problems


# ---- the cases ---------------------------------------------------------------------------------------------


def deck_resources(bundle, shot_id):
    """The resource table of one deck as the plan would carry it: every node that names an `assetRef`, keyed by
    `assetId`, with the node ids of THIS deck only (plan 7.3: each deck keeps its own table)."""
    chosen = next(item for item in bundle["shots"] if item["id"] == shot_id)
    instance = next(item for item in bundle["sceneInstances"] if item["id"] == chosen["sceneInstanceId"])
    definition = next(item for item in bundle["sceneDefinitions"] if item["id"] == instance["sceneDefinitionId"])
    table = {}
    for node in definition["nodes"]:
        reference = node.get("assetRef")
        if reference:
            entry = table.setdefault(reference["assetId"], {"revisionId": reference["revisionId"], "contentHash": reference["contentHash"],
                                                           "relativePath": reference["relativePath"], "used_by": []})
            entry["used_by"].append(node["id"])
    for entry in table.values():
        entry["used_by"].sort()
    return {"scene": definition["id"], "resources": table}


def repo_facts():
    """What the corpus reads from this repository (never writes): the public fixture bundle and the facts about
    the destination the alpha cases state."""
    bundle_path = ROOT / TARGET_FILES["bundle"]
    bundle = json.loads(bundle_path.read_text(encoding="utf-8"))
    shots = [
        shot(item["id"], item["editRange"]["startFrame"], item["editRange"]["endFrameExclusive"], item["sceneInstanceId"],
             item["instanceStartFrame"], item["cameraClipId"], item["cameraInFrame"], item.get("transitionInto"))
        for item in bundle["shots"]
    ]
    for item in shots:
        into = item["into"]
        if into is not None and into.get("type") == "dissolve":
            item["into"] = {"type": "dissolve", "duration": into["durationFrames"]}
    instances = {item["id"]: item["durationFrames"] for item in bundle["sceneInstances"]}
    cameras = {clip["id"]: sum(segment["durationFrames"] for segment in clip["segments"]) for clip in bundle["cameraClips"]}
    parse_text = (ROOT / TARGET_FILES["parse"]).read_text(encoding="utf-8")
    export_text = (ROOT / TARGET_FILES["export_test"]).read_text(encoding="utf-8")
    legacy_bundle = json.loads((ROOT / TARGET_FILES["legacy_bundle"]).read_text(encoding="utf-8"))
    legacy_expected = (ROOT / TARGET_FILES["legacy_expected"]).read_text(encoding="utf-8")
    effects_text = (ROOT / TARGET_FILES["effects_doc"]).read_text(encoding="utf-8")
    return {
        "decks": {item["id"]: deck_resources(bundle, item["id"]) for item in shots},
        "f016_codec_text_present": F016_CODEC_QUOTE in effects_text,
        "corpus_edit": edit_model(bundle["editDurationFrames"], shots, instances, cameras),
        "bundle_sha256": hashlib.sha256(bundle_path.read_bytes()).hexdigest(),
        "premultiplied_refused": "PREMULTIPLIED_ALPHA_UNSUPPORTED" in parse_text,
        "dissolve_unsupported_in_parser": "DISSOLVE_UNSUPPORTED" in parse_text,
        "legacy_expected_message_unchanged": "Dissolve transitions are not supported in pilot MVP" in legacy_expected,
        "legacy_bundle_alpha": legacy_bundle["renderProfile"]["alphaMode"],
        "codec_tolerance_is_16": "const CODEC_TOLERANCE: i32 = 16;" in export_text
        and "An H.264 CRF 18 round trip on both sides of the comparison, on top of 8-bit rounding." in export_text,
        "bundle_fps": bundle["fps"],
        "bundle_alpha_mode": bundle["renderProfile"]["alphaMode"],
    }


def window_cases():
    rows = []
    for alignment in ALIGNMENTS:
        for duration in (1, 2, 3, 4, 5, 6, 11, 12, 13, 24):
            rows.append({"cut": 48, "duration": duration, "alignment": alignment, "window": raw_window(48, duration, alignment)})
    ceilings = []
    for alignment in ALIGNMENTS:
        for from_frames, to_frames in ((48, 48), (0, 5), (5, 0), (1, 1), (10, 3)):
            ceilings.append({"alignment": alignment, "from_frames": from_frames, "to_frames": to_frames,
                             "max": max_transition_frames(alignment, from_frames, to_frames)})
    bounds = []
    edge_cases = [
        (50, 100, "center", 100), (50, 101, "center", 100), (0, 12, "start", 100), (12, 12, "end", 100), (11, 12, "end", 100),
        (6, 12, "center", 100), (5, 12, "center", 100), (94, 12, "center", 100), (95, 12, "center", 100), (100, 1, "start", 100),
        (99, 1, "start", 100), (48, 1, "center", 144), (48, 0, "center", 144), (48, -1, "center", 144), (48, 145, "center", 144),
    ]
    for cut, duration, alignment, edit in edge_cases:
        bounds.append({"cut": cut, "duration": duration, "alignment": alignment, "edit_duration": edit, "outcome": attempt(checked_window, cut, duration, alignment, edit)})
    return {"raw": rows, "max_transition_frames": ceilings, "bounds": bounds}


def strict_cases():
    bad_ints = [12.5, 12.0, True, False, "12", None, [12], 2**53, 2**63 - 1, 2**63, -(2**63) - 1, -(2**53)]
    durations = [{"value": value, "outcome": attempt(checked_window, 48, value, "center", 144)} for value in bad_ints]
    cuts = [{"value": value, "outcome": attempt(checked_window, value, 12, "center", 144)} for value in (12.5, True, "48", None, -1, 2**53, SAFE_MAX)]
    edits = [{"value": value, "outcome": attempt(checked_window, 48, 12, "center", value)} for value in (0, -5, 143.5, True, None, 2**53)]
    refused_not_clamped = [
        {"note": "start -2: refused, not clamped to 0", "outcome": attempt(checked_window, 4, 12, "center", 100)},
        {"note": "end 101: refused, not clamped to 100", "outcome": attempt(checked_window, 95, 12, "center", 100)},
        {"note": "huge cut: window past the edit is refused, nothing saturates", "outcome": attempt(checked_window, SAFE_MAX, 2, "start", 100)},
    ]
    seeds = [{"value": value, "outcome": attempt(strict_seed, value)} for value in (0, 1, 99, SEED_MAX, -1, SEED_MAX + 1, 1.5, True, "7", None, 2**63)]
    availability = [
        {"total": 40, "start": 5, "length": 40, "available": available(40, 5, 40), "note": "a camera clip shorter than its shot: no negative tail"},
        {"total": 60, "start": 10, "length": 40, "available": available(60, 10, 40)},
        {"total": 0, "start": 0, "length": 48, "available": available(0, 0, 48)},
    ]
    return {"duration": durations, "cut": cuts, "edit_duration": edits, "invalid_window_is_refused": refused_not_clamped, "seeds": seeds, "availability": availability}


def progress_cases():
    tables = []
    for duration in (1, 2, 5, 12, 13, 24):
        for easing in EASINGS:
            rows = []
            for k in range(duration):
                p = progress_of(k, duration, easing)
                rows.append({"k": k, "t": raw_t(k, duration), "progress": p,
                             "weight_smooth_1": weight_of(p, "smooth", 1.0), "weight_linear_1": weight_of(p, "linear", 1.0)})
            tables.append({"duration": duration, "easing": easing, "frames": rows})
    symmetry = []
    for duration in (1, 2, 5, 12, 13, 24, 96, 1000):
        worst = max(abs(progress_of(k, duration, "easeInOutCubic") + progress_of(duration - 1 - k, duration, "easeInOutCubic") - 1.0) for k in range(duration))
        symmetry.append({"duration": duration, "easing": "easeInOutCubic", "max_abs_error": worst, "within_1e-12": worst <= 1e-12})
    extremes = []
    for duration in (1, 2, 2**31, 2**52):
        for k in sorted({0, duration - 1}):
            row = {"duration": duration, "k": k}
            for easing in EASINGS:
                p = progress_of(k, duration, easing)
                row[easing] = {"progress": p, "finite": math.isfinite(p), "in_unit_interval": 0.0 <= p <= 1.0}
            extremes.append(row)
    intensity = []
    for k in (3, 5, 8):
        p = progress_of(k, 12, "easeInOutCubic")
        for curve in CURVES:
            for amount in (0.0, 0.25, 0.5, 0.75, 1.0):
                weight = weight_of(p, curve, amount)
                intensity.append({"k": k, "curve": curve, "intensity": amount, "progress": p, "weight": weight,
                                  "weight_equals_progress": weight == p, "weight_is_zero": weight == 0.0,
                                  "weight_equals_uneased_t": weight == raw_t(k, 12)})
    return {"tables": tables, "symmetry": symmetry, "extremes": extremes, "intensity": intensity,
            "notes": ["intensity 0 is NOT identity of the effect: the weight is the eased progress (not 0, not t)",
                      "curve linear ignores intensity (shaped == progress), so plain == curved",
                      "linear/linear cannot discriminate a mutant that ignores intensity: use smooth"]}


def quantization_cases():
    inputs = [0.0, 1.0, 0.5, 0.45, 0.3333, 0.1, -0.1, 1.2, 2.0 / 255.0, 127.5 / 255.0]
    decode_codes = [0, 1, 10, 11, 40, 128, 200, 255]
    roundtrip_failures = [c for c in range(256) if quant8(encode(decode(c))) != c]
    return {
        "rounding": "half-up: floor(x * 255 + 0.5), clamped to 0..255",
        "inputs": [{"unit": value, "code": quantize_input(value)} for value in inputs],
        "decode": [{"code": c, "linear": r9(decode(c))} for c in decode_codes],
        "decode_encode_roundtrip_failures": roundtrip_failures,
    }


def pixel_case(case_id, covers, deck_a, deck_b, k, duration, easing, curve, intensity, kind="origin-formula", **extra):
    progress = progress_of(k, duration, easing)
    out = {"id": case_id, "covers": covers, "kind": kind, "deck_a": deck_a, "deck_b": deck_b, "frame_in_window": k, "duration": duration,
           "easing": easing, "curve": curve, "intensity": intensity, "progress": progress, "weight": weight_of(progress, curve, intensity),
           "expected": mix_pixel(deck_a, deck_b, progress, curve, intensity), "tolerance": "monitor_and_render"}
    out.update(extra)
    return out


def pixel_cases():
    a, b = [200, 128, 16], [40, 128, 240]
    cases = [pixel_case("k5-d12-cubic-smooth-intensity-%s" % str(amount).replace(".", "_"), ["T07", "T16"], a, b, 5, 12, "easeInOutCubic", "smooth", amount,
                        note="plan 6.2 vector (channel R 200 -> 40); documentary, not engine output") for amount in (0.0, 0.5, 1.0)]
    cases += [
        pixel_case("k0-first-frame", ["T07", "T16"], a, b, 0, 12, "easeInOutCubic", "smooth", 1.0),
        pixel_case("k11-last-frame", ["T07", "T16"], a, b, 11, 12, "easeInOutCubic", "smooth", 1.0),
        pixel_case("d1-single-frame", ["T05", "T07"], a, b, 0, 1, "easeInOutCubic", "smooth", 1.0),
        pixel_case("linear-linear-i0", ["T07"], a, b, 5, 12, "linear", "linear", 0.0),
        pixel_case("linear-linear-i1", ["T07"], a, b, 5, 12, "linear", "linear", 1.0),
        pixel_case("curve-linear-cubic-i0", ["T07"], a, b, 4, 12, "easeInOutCubic", "linear", 0.0),
        pixel_case("curve-linear-cubic-i1", ["T07"], a, b, 4, 12, "easeInOutCubic", "linear", 1.0),
        pixel_case("toe-of-the-transfer-curve", ["T16"], [0, 3, 10], [12, 11, 11], 6, 12, "linear", "linear", 1.0),
        pixel_case("black-to-white", ["T16"], [0, 0, 0], [255, 255, 255], 3, 12, "easeOutCubic", "smooth", 0.5),
        pixel_case("equal-decks-stay-equal", ["T16"], [128, 77, 200], [128, 77, 200], 7, 12, "easeInOutCubic", "smooth", 1.0),
    ]
    quantized = [0.45, 0.3333, 0.5]
    deck_from_floats = [quantize_input(value) for value in quantized]
    cases.append(pixel_case("decks-quantized-from-float-input", ["T16"], deck_from_floats, [40, 128, 240], 5, 12, "easeInOutCubic", "smooth", 1.0,
                            deck_a_unit_input=quantized, note="the unit floats become 8-bit codes (115, 85, 128) before the mix"))
    cases.append(pixel_case("partially-transparent-decks", ["T16", "ALPHA"], a, b, 5, 12, "easeInOutCubic", "smooth", 1.0,
                            kind="origin-formula (rgb) + destination-contract-assumed (alpha)", deck_a_alpha=128, deck_b_alpha=0,
                            note="the shader reads rgb only and writes alpha 1.0: the output alpha is 255 whatever the decks' alpha. Output-alpha-1 is an ASSUMED destination contract (divergence declared in F064), not an origin result."))
    pairs = [([200, 128, 16], [40, 128, 240]), ([255, 0, 0], [0, 0, 255]), ([20, 20, 20], [235, 235, 235]), ([90, 160, 30], [250, 10, 120]),
             ([5, 250, 128], [128, 5, 250]), ([64, 64, 64], [192, 192, 192])]
    grid = []
    for pair_index, (deck_a, deck_b) in enumerate(pairs):
        for k in range(12):
            p = progress_of(k, 12, "easeInOutCubic")
            for amount in (0.5, 1.0):
                grid.append({"pair": pair_index, "k": k, "intensity": amount, "rgb8": mix_pixel(deck_a, deck_b, p, "smooth", amount)["rgb8"]})
    return {"cases": cases, "grid": {"decks": [{"a": x, "b": y} for x, y in pairs], "duration": 12, "easing": "easeInOutCubic", "curve": "smooth",
                                    "probes": grid, "probes_per_channel": len(grid), "tolerance": "monitor_and_render"}}


def deck_frame_cases(facts):
    corpus = corpus_edit(facts)
    by_id = {item["id"]: item for item in corpus["shots"]}
    first, second = by_id["shot-a"], by_id["shot-b"]

    def rows(edit, item, deltas):
        instance_frames = edit["instances"][item["instance"]]
        camera_frames = edit["camera_frames"][item["camera"]]
        return [resolve_shot_at_offset(item, instance_frames, camera_frames, delta) for delta in deltas]

    spec = spec_edit()
    spec_s1, spec_s2 = spec["shots"][0], spec["shots"][1]
    return {
        "corpus_valid_cut_instance_offsets": {
            "kind": "origin-formula", "covers": ["T10"], "bundle_sha256": facts["bundle_sha256"], "cut": second["start"],
            "assumption": "CAMERA_FRAMES_IS_SUM_OF_SEGMENT_DURATIONS",
            "from": {"shot": first["id"], "instance_frames": corpus["instances"][first["instance"]], "camera_frames": corpus["camera_frames"][first["camera"]],
                     "instance_start": first["instance_start"], "camera_in": first["camera_in"],
                     "tail_instance": shot_handles(first, corpus)["tail"], "tail_camera": camera_handles(first, corpus)["tail"],
                     "rows": rows(corpus, first, [-6, -1, 0, 1, 34, 35, 39, 40, 45, 49, 50, 51])},
            "to": {"shot": second["id"], "instance_frames": corpus["instances"][second["instance"]], "camera_frames": corpus["camera_frames"][second["camera"]],
                   "instance_start": second["instance_start"], "camera_in": second["camera_in"],
                   "head_instance": shot_handles(second, corpus)["head"], "head_camera": camera_handles(second, corpus)["head"],
                   "rows": rows(corpus, second, [-12, -6, -1, 0, 1, 29, 30, 31, 59, 60, 66])},
        },
        "empty_clips": {
            "kind": "origin-formula", "covers": ["T10"],
            "rows": [resolve_shot_at_offset(shot("e", 0, 10, "i", 0, "c", 0), 0, 0, delta) for delta in (-3, 0, 5)]
            + [resolve_shot_at_offset(shot("e", 0, 10, "i", 0, "c", 0), 20, 0, 4)],
            "note": "no frames: frame 0 and always held, even when 0 was wanted; the instance clamp is separate from the camera's",
        },
        "origin_spec_vectors": {
            "kind": "origin-spec-vector", "covers": ["T06", "T10"], "source": "fx/resolve-fx-frame-transitions.spec.ts",
            "fixture": "RECONSTRUCTED from the numbers the spec shows (fx-test-bundle.ts was not read); only the observable numbers are claimed",
            "edit": spec,
            "rows": [
                {"frame": 50, "shot": "s1", "row": resolve_shot_at_offset(spec_s1, 200, 48, 50 - spec_s1["start"]),
                 "spec": {"instanceFrame": 70, "cameraClipFrame": 47, "heldInstance": False, "heldCamera": True}},
                {"frame": 50, "shot": "s2", "row": resolve_shot_at_offset(spec_s2, 200, 96, 50 - spec_s2["start"]),
                 "spec": {"instanceFrame": 2, "cameraClipFrame": 2, "heldInstance": False, "heldCamera": False}},
                {"frame": 44, "shot": "s2", "row": resolve_shot_at_offset(spec_s2, 200, 96, 44 - spec_s2["start"]),
                 "spec": {"instanceFrame": 0, "cameraClipFrame": 0, "heldInstance": True, "heldCamera": True}},
            ],
        },
        "joint_clamp_is_wrong": {
            "kind": "origin-formula", "covers": ["T10"],
            "note": "an instance longer than its camera runs out of one first: at delta 35 the camera is held and the rig is not",
            "row": resolve_shot_at_offset(first, corpus["instances"][first["instance"]], corpus["camera_frames"][first["camera"]], 35),
        },
    }


def shot_clip_cases():
    shot_start, shot_end = 48, 96
    scenarios = [
        ("covers-inside-the-shot", 60, 12, 70), ("covers-and-reaches-before-the-shot", 40, 20, 44), ("touches-the-start-in-the-head", 48, 12, 44),
        ("touches-the-start-further-back", 48, 12, 30), ("middle-clip-stays-out-of-the-head", 60, 10, 44), ("touches-the-end-in-the-tail", 80, 16, 100),
        ("touches-the-end-further-out", 80, 16, 140), ("middle-clip-stays-out-of-the-tail", 80, 10, 100), ("last-frame-of-the-clip", 80, 16, 95),
        ("first-frame-after-the-shot-is-tail", 80, 16, 96), ("clip-end-is-exclusive", 60, 12, 72), ("starts-at-the-shot-but-the-frame-is-after-the-clip", 48, 12, 60),
        ("covering-clip-wins-over-the-edge-rule", 48, 12, 48),
    ]
    rows = []
    for name, clip_start, clip_duration, edit_frame in scenarios:
        rows.append({"name": name, "clip_start": clip_start, "clip_duration": clip_duration, "shot_start": shot_start, "shot_end": shot_end,
                     "edit_frame": edit_frame, "frame": shot_clip_frame(clip_start, clip_duration, shot_start, shot_end, edit_frame)})
    return {"kind": "origin-formula", "covers": ["T11"], "rows": rows}


def handle_cases(facts):
    corpus = corpus_edit(facts)
    spec = spec_edit()
    scenarios = [
        ("corpus-center-12-use-lacks-camera-and-head", "origin-formula", corpus, tr(from_id="shot-a", to_id="shot-b", handles="use")),
        ("corpus-center-12-hold-needs-nothing", "origin-formula", corpus, tr(from_id="shot-a", to_id="shot-b", handles="hold")),
        ("corpus-center-12-refuse-reads-as-use", "origin-formula", corpus, tr(from_id="shot-a", to_id="shot-b", handles="refuse")),
        ("corpus-start-12-use-needs-only-the-tail", "origin-formula", corpus, tr(from_id="shot-a", to_id="shot-b", handles="use", alignment="start")),
        ("corpus-end-6-use-needs-only-the-head", "origin-formula", corpus, tr(from_id="shot-a", to_id="shot-b", handles="use", alignment="end", duration=6)),
        ("variant-only-the-camera-is-short", "origin-formula", with_changes(corpus, shot_changes={"shot-b": {"instance_start": 10}}),
         tr(from_id="shot-a", to_id="shot-b", handles="use", alignment="end", duration=6)),
        ("origin-spec-s1-s2-use", "origin-spec-vector", spec, tr(handles="use")),
        ("origin-spec-s2-s3-camera-cut-to-50", "origin-spec-vector", with_changes(spec, cameras={"cam2": 50}), tr(from_id="s2", to_id="s3", handles="use")),
        ("origin-spec-s1-s2-cam1-96-both-lack-the-head", "origin-spec-vector", with_changes(spec, cameras={"cam1": 96}), tr(handles="use")),
        ("origin-spec-s2-camera-in-10-instance-only", "origin-spec-vector", with_changes(spec, cameras={"cam1": 96}, shot_changes={"s2": {"camera_in": 10}}), tr(handles="use")),
        ("origin-spec-s3-camera-in-4-end-12", "origin-spec-vector", with_changes(spec, shot_changes={"s3": {"camera_in": 4}}), tr(from_id="s2", to_id="s3", alignment="end", handles="use")),
        ("origin-spec-s2-s3-use-has-the-frames", "origin-spec-vector", spec, tr(from_id="s2", to_id="s3", handles="use")),
    ]
    out = []
    for name, kind, edit, transition in scenarios:
        window = raw_window(next(item for item in edit["shots"] if item["id"] == transition["toShotId"])["start"], transition["durationFrames"], transition["alignment"])
        out.append({"name": name, "kind": kind, "covers": ["T02", "T10"], "transition": transition, "window": window, "issues": explain_transitions(edit, [transition])})
    return out


def explain_cases(facts):
    spec = spec_edit()
    corpus = corpus_edit(facts)
    with_dissolve = with_changes(spec, shot_changes={"s2": {"into": {"type": "dissolve", "duration": 12}}})
    cases = [
        ("a-valid-transition", spec, [tr(handles="hold")]),
        ("unknown-effect", spec, [tr(effect="nope")]),
        ("a-look-is-the-wrong-family-not-an-unknown-effect", spec, [tr(effect="vignette")]),
        ("unknown-param", spec, [tr(params={"speed": 1})]),
        ("param-type-curve-number", spec, [tr(params={"curve": 1})]),
        ("param-type-intensity-bool", spec, [tr(params={"intensity": True})]),
        ("param-range-enum", spec, [tr(params={"curve": "wavy"})]),
        ("param-range-intensity-high", spec, [tr(params={"intensity": 1.5})]),
        ("param-range-intensity-integral-float", spec, [tr(params={"intensity": 2.0})]),
        ("param-range-intensity-negative", spec, [tr(params={"intensity": -0.25})]),
        ("param-ok-edges", spec, [tr(params={"curve": "linear", "intensity": 0})]),
        ("unknown-easing", spec, [tr(easing="bounce")]),
        ("unknown-shot-from", spec, [tr(from_id="ghost")]),
        ("unknown-shot-both", spec, [tr(from_id="ghost", to_id="phantom")]),
        ("not-at-cut-skips-a-shot", spec, [tr(from_id="s1", to_id="s3")]),
        ("not-at-cut-reversed", spec, [tr(from_id="s2", to_id="s1")]),
        ("not-at-cut-last-shot-has-no-next", spec, [tr(from_id="s3", to_id="s1")]),
        ("duplicate-on-one-cut", spec, [tr(transition_id="a"), tr(transition_id="b")]),
        ("duration-zero", spec, [tr(duration=0)]),
        ("duration-negative", spec, [tr(duration=-3)]),
        ("window-leaves-the-edit-at-the-start", spec, [tr(duration=100, handles="hold")]),
        ("window-leaves-the-edit-at-the-end", spec, [tr(from_id="s2", to_id="s3", duration=100, handles="hold")]),
        ("window-edges-exactly-on-the-edit", spec, [tr(from_id="s2", to_id="s3", duration=96, alignment="start", handles="hold")]),
        ("too-long-for-the-shots", spec, [tr(duration=100, handles="hold")]),
        ("no-handles-use", spec, [tr(handles="use")]),
        ("overlap-of-neighbouring-cuts", spec, [tr(transition_id="a", duration=40), tr(transition_id="b", from_id="s2", to_id="s3", duration=60)]),
        ("neighbouring-windows-that-touch-do-not-overlap", spec, [tr(transition_id="a", duration=12), tr(transition_id="b", from_id="s2", to_id="s3", duration=12)]),
        ("order-easing-before-the-cut", spec, [tr(easing="bounce", from_id="s1", to_id="s3")]),
        ("order-family-then-easing-then-shot", spec, [tr(effect="vignette", easing="bounce", from_id="ghost")]),
        ("declared-next-to-a-legacy-dissolve", with_dissolve, [tr(handles="hold")]),
        ("corpus-valid-center-12-hold", corpus, [tr(from_id="shot-a", to_id="shot-b", handles="hold")]),
        ("corpus-edit-is-100-so-center-100-fits-at-cut-50-only-if-the-shots-do", corpus, [tr(from_id="shot-a", to_id="shot-b", duration=100, handles="hold")]),
    ]
    out = []
    for name, edit, transitions in cases:
        issues = explain_transitions(edit, transitions)
        out.append({"name": name, "covers": ["T02"], "kind": "origin-formula", "edit": "corpus" if edit is corpus else ("spec" if edit is spec else "spec+legacy-dissolve"),
                    "transitions": transitions, "codes": [issue["code"] for issue in issues], "issues": issues})
    return out


def id_cases():
    valid = ["tr-1", "A", "a_b-9", "0x", "x" * 64, "TR-UPPER"]
    invalid = ["", "-x", "_x", "x" * 65, "tr 1", "tr\n", "\ntr", "ſhot", "K-kelvin", "tr-é", "tr.1", "tr/1", "ｔｒ"]
    reserved = ["track", "TRACK", "Fx", "transition", "Transition", "fx"]
    taken = ["a", "track-1"]
    return {
        "kind": "origin-formula", "covers": ["T02"],
        "pattern": "^[a-z0-9][a-z0-9_-]{0,63}$ (ASCII, case-insensitive, whole string: a trailing newline does not match)",
        "valid": [{"id": value, "problems": id_problems(value, [])} for value in valid],
        "invalid": [{"id": value, "problems": id_problems(value, [])} for value in invalid],
        "reserved": [{"id": value, "problems": id_problems(value, [])} for value in reserved],
        "duplicate": [{"id": value, "taken": taken, "problems": id_problems(value, taken)} for value in ("a", "track-1", "b")],
        "reserved_and_duplicate": {"id": "fx", "taken": ["fx"], "problems": id_problems("fx", ["fx"])},
        "negative_only": "these refusals do not enable any product behaviour",
    }


def keys_and_hidden_cases():
    return {
        "keys": {
            "kind": "destination-contract-candidate", "covers": ["T02"],
            "origin": "accepted: keys are recut to the window (not modelled here)",
            "destination_plan": "refused with FX_TRANSITION_KEYS_NOT_YET (plan D10, BL082.4.9); nothing is enabled",
            "divergence": "KEYS_REFUSED",
            "oracle": attempt(lambda: explain_transitions(spec_edit(), [tr(keys={"intensity": [{"frame": 0, "value": 0.5, "easing": "linear"}]})])),
        },
        "hidden": {
            "kind": "origin-formula", "covers": ["T04"],
            "origin_schema": "hidden: z.literal(true).optional(): `true` or absent",
            "cases": [{"hidden": True, "origin": "accepted", "draws": False}, {"hidden": None, "origin": "accepted", "draws": True},
                      {"hidden": False, "origin": "refused by the schema (literal true)", "draws": None}],
            "destination": "DECIDED: transitions accept true or an absent hidden field, matching the origin; explicit false is a structural refusal before explain (S1a). Existing look clips keep their current contract.",
        },
        "seed": {"kind": "origin-formula", "range": "uint32 0..4294967295", "consumed_by_crossDissolve": False, "preserved": True,
                 "note": "the effect draws no noise; the seed is kept in the document (Reseed::Never in the plan)"},
    }


def fps_cases():
    rates = [(24, 1), (25, 1), (60, 1), (30000, 1001)]
    rows = []
    for numerator, denominator in rates:
        edit_clock = Fraction(44 * denominator, numerator)
        instance_clock = Fraction(0, 1)
        rows.append({
            "fps": [numerator, denominator], "window_center_12_cut_48": raw_window(48, 12, "center"),
            "progress_k3_d12_cubic": progress_of(3, 12, "easeInOutCubic"),
            "edit_clock_seconds_frame_44": "%d/%d" % (edit_clock.numerator, edit_clock.denominator),
            "held_instance_clock_seconds": "%d/%d" % (instance_clock.numerator, instance_clock.denominator),
            "window_start_seconds": "%d/%d" % ((Fraction(42 * denominator, numerator)).numerator, (Fraction(42 * denominator, numerator)).denominator),
        })
    return {"kind": "origin-formula", "covers": ["T06"], "rows": rows,
            "rule": "the window and the progress are in frames and do not depend on the fps; light/weather/flicker run on the REAL edit frame over fps, "
                    "while the instance and camera frames are clamped apart (a held instance still lets the light flicker)"}


def alpha_cases(facts):
    rows = [
        ("opaque-with-a-declared-transition", "opaque", 1, False, "accepted", None, "same-verdict"),
        ("straight-with-a-declared-transition", "straight", 1, False, "accepted-and-rendered-opaque", None, "DIVERGENCE_VERDICT"),
        ("premultiplied-with-a-declared-transition", "premultiplied", 1, False, "refused", "PREMULTIPLIED_ALPHA_UNSUPPORTED", "same-verdict-other-code"),
        ("straight-with-only-a-look-clip", "straight", 0, True, "accepted-and-rendered-opaque", None, "DIVERGENCE_VERDICT (already true for the three looks)"),
        ("straight-with-an-empty-fx-and-no-dissolve", "straight", 0, False, "accepted", None, "same-verdict"),
        ("straight-with-a-legacy-dissolve-only", "straight", 1, False, "refused", "DISSOLVE_UNSUPPORTED", "same-verdict-other-code"),
        ("opaque-with-a-legacy-dissolve", "opaque", 1, False, "refused", "DISSOLVE_UNSUPPORTED", "DIVERGENCE_VERDICT (legacy dissolve is unsupported until S6)"),
    ]
    out = []
    for name, alpha_mode, drawn, clips, target, code, relation in rows:
        entry = {"name": name, "alpha_mode": alpha_mode, "transitions_drawn": drawn, "has_look_clips": clips,
                 "origin": origin_alpha_verdict(alpha_mode, drawn, clips), "destination_contract": {"verdict": target}, "relation": relation}
        if code:
            entry["destination_contract"]["code"] = code
        out.append(entry)
    return {
        "kind": "origin-formula (origin) + destination-contract-declared (destination)",
        "covers": ["ALPHA"],
        "status": "declared divergence of VERDICT, pending design review (plan Q3); the destination accepts what the origin refuses",
        "never_say": "the origin accepts a straight profile with a transition: it does NOT (UNSUPPORTED_ALPHA, also with a legacy dissolve and no fx)",
        "rows": out,
        "legacy_fixture": {
            "untouched": "invalid-dissolve-transition.expected.json (bundleSha256, schema message) and the parser code DISSOLVE_UNSUPPORTED are not edited",
            "profile_alpha_mode": facts["legacy_bundle_alpha"],
            "origin_would_say": origin_alpha_verdict(facts["legacy_bundle_alpha"], 1, False),
            "destination_says": "DISSOLVE_UNSUPPORTED (unchanged; S6 is a separate, future track)",
        },
        "destination_facts_read_from_the_repository": {
            "premultiplied_refused_by_the_document_parser": facts["premultiplied_refused"],
            "document_parser_still_has_DISSOLVE_UNSUPPORTED": facts["dissolve_unsupported_in_parser"],
            "legacy_expected_schema_message_unchanged": facts["legacy_expected_message_unchanged"],
        },
        "output_alpha": "255 (the shader writes 1.0): an ASSUMED destination contract, not measured",
        "future": "the S6 amendment (new oracle lane, separate PR) will decide the legacy dissolve; nothing here implements it",
    }


# ---- plan 8, 5.3, 7.3: the tracer, the ceilings, the default duration and the resource union -------------

TYPICAL_DURATION = 24


def plan_limit_verdict(plan_bytes, plan_items, depth):
    """Plan 5.3: one verdict per ceiling, the exact limit accepted and one above refused; depth 1 only."""
    ceilings = TOLERANCES["plan_ceilings"]
    if depth > ceilings["TRANSITION_PLAN_MAX_DEPTH"]:
        return {"verdict": "refused", "code": "invalid", "path": "/transition/other/transition", "detail": "no nesting"}
    if plan_bytes > ceilings["MAX_TRANSITION_PLAN_BYTES"]:
        return {"verdict": "refused", "code": "invalid", "path": "/transition", "detail": "MAX_TRANSITION_PLAN_BYTES"}
    if plan_items > ceilings["MAX_TRANSITION_PLAN_ITEMS"]:
        return {"verdict": "refused", "code": "invalid", "path": "/transition", "detail": "MAX_TRANSITION_PLAN_ITEMS"}
    return {"verdict": "accepted"}


def fx_limit_verdict(shots, count):
    """Plan 5.2 rule 1: at most `shots - 1` transitions, checked before any cost is computed."""
    most = max(0, shots - 1)
    return {"max": most, "verdict": "accepted" if count <= most else "refused", **({} if count <= most else {"code": "FX_LIMIT"})}


def default_duration(alignment, from_frames, to_frames):
    """D06: an omitted duration is the typical one CUT to what the two shots hold; a maximum of 0 is TOO_LONG."""
    most = max_transition_frames(alignment, from_frames, to_frames)
    if most <= 0:
        raise Refusal("TRANSITION_TOO_LONG", "the two shots hold no frame of this alignment")
    return min(TYPICAL_DURATION, most)


def explicit_duration_fits(alignment, duration, from_frames, to_frames):
    """The origin's TOO_LONG test for a duration that was GIVEN: refused, never cut."""
    start = raw_window(0, duration, alignment)
    return not (from_frames < start["before"] or to_frames < start["after"])


def limits_cases():
    ceilings = TOLERANCES["plan_ceilings"]
    rows = []
    for name, plan_bytes, plan_items, depth in (
        ("bytes-at-the-limit", ceilings["MAX_TRANSITION_PLAN_BYTES"], 10, 1), ("bytes-one-above", ceilings["MAX_TRANSITION_PLAN_BYTES"] + 1, 10, 1),
        ("items-at-the-limit", 100, ceilings["MAX_TRANSITION_PLAN_ITEMS"], 1), ("items-one-above", 100, ceilings["MAX_TRANSITION_PLAN_ITEMS"] + 1, 1),
        ("depth-1-is-the-only-depth", 100, 10, 1), ("depth-2-hostile-nesting", 100, 10, 2), ("depth-bomb-refused-without-recursing", 100, 10, 1000),
    ):
        rows.append({"name": name, "bytes": plan_bytes, "items": plan_items, "depth": depth, "outcome": plan_limit_verdict(plan_bytes, plan_items, depth)})
    count_rows = [{"shots": shots, "transitions": count, "outcome": fx_limit_verdict(shots, count)}
                  for shots, count in ((2, 1), (2, 2), (1, 0), (1, 1), (0, 0), (0, 1), (5, 4), (5, 5), (64, 63), (64, 64))]
    default_rows = []
    for alignment, from_frames, to_frames in (("center", 40, 60), ("center", 5, 5), ("center", 0, 10), ("center", 10, 0), ("start", 12, 6), ("start", 10, 0),
                                              ("end", 6, 12), ("end", 0, 12), ("end", 30, 30), ("center", 11, 12), ("center", 12, 12)):
        default_rows.append({"alignment": alignment, "from_frames": from_frames, "to_frames": to_frames,
                             "max": max_transition_frames(alignment, from_frames, to_frames), "omitted_duration": attempt(default_duration, alignment, from_frames, to_frames)})
    explicit_rows = [{"alignment": alignment, "duration": duration, "from_frames": from_frames, "to_frames": to_frames,
                      "fits": explicit_duration_fits(alignment, duration, from_frames, to_frames),
                      "on_overflow": "TRANSITION_TOO_LONG (refused, never cut)"}
                     for alignment, duration, from_frames, to_frames in (("center", 24, 5, 5), ("center", 10, 5, 5), ("center", 11, 5, 5), ("start", 7, 12, 6), ("start", 6, 12, 6), ("end", 13, 12, 30), ("end", 12, 12, 30))]
    return {
        "kind": "destination-contract-candidate", "covers": ["T02", "T28"],
        "ceilings": {key: ceilings[key] for key in ("MAX_TRANSITION_PLAN_BYTES", "MAX_TRANSITION_PLAN_ITEMS", "TRANSITION_PLAN_MAX_DEPTH")},
        "status": ceilings["status"], "plan_rows": rows, "fx_limit": count_rows, "default_duration": default_rows, "explicit_duration": explicit_rows,
        "typical_duration": TYPICAL_DURATION,
        "note": "plan 5.2/5.3 and D06: candidates fixed before any code; the origin has no equivalent of the ceilings or of FX_LIMIT",
    }


def resource_union(primary, other):
    """Plan 7.3: the union is DERIVED (never serialized), ordered by `assetId`, `used_by` qualified by deck; the same
    `assetId` with other content in the two decks is an invalid plan. Node ids may collide between decks, which is
    why the reference of a node is always the pair (deck, node id) and a resource is identified by `assetId`."""
    union = {}
    for deck_name, deck in (("primary", primary), ("other", other)):
        for asset_id, entry in deck["resources"].items():
            current = union.get(asset_id)
            if current is not None and any(current[key] != entry[key] for key in ("revisionId", "contentHash", "relativePath")):
                raise Refusal("RESOURCE_CONFLICT", "/transition/other/resources: the same assetId %s, different content" % asset_id)
            if current is None:
                current = union.setdefault(asset_id, {"revisionId": entry["revisionId"], "contentHash": entry["contentHash"],
                                                      "relativePath": entry["relativePath"], "used_by": []})
            current["used_by"].extend({"deck": deck_name, "node_id": node_id} for node_id in entry["used_by"])
    return {asset_id: union[asset_id] for asset_id in sorted(union)}


def resource_union_cases(facts):
    first, second = facts["decks"]["shot-a"], facts["decks"]["shot-b"]
    colliding_nodes = sorted(set(node for entry in first["resources"].values() for node in entry["used_by"])
                             & set(node for entry in second["resources"].values() for node in entry["used_by"]))
    changed = json.loads(json.dumps(second))
    changed["resources"]["figure"]["contentHash"] = "synthetic-sha256-figure-changed"
    only_neighbour = json.loads(json.dumps(second))
    only_neighbour["resources"]["sky"] = {"revisionId": "rev-1", "contentHash": "synthetic-sha256-sky", "relativePath": "assets/sky.png", "used_by": ["sky-plane"]}
    return {
        "kind": "destination-contract-candidate", "covers": ["T09", "T10"],
        "source": "docs/migration/fixtures/bundles/valid-cut-instance-offsets.bundle.json read at generation (sceneDefinitions[].nodes[].assetRef), never copied",
        "decks": {"primary": first, "other": second},
        "colliding_node_ids_between_decks": colliding_nodes,
        "union": attempt(resource_union, first, second),
        "requires_added_by_a_transition": ["fxTransition"],
        "order": "assets by assetId; inside an asset, deck `primary` then `other`, node ids ascending in each deck (the oracle's proposal; S1b confirms or amends it)",
        "no_cross_deck_identifier": "a node is the pair (deck, node id); no identifier crosses decks",
        "conflict": {"note": "the neighbour names the same assetId with other content: the plan is invalid", "outcome": attempt(resource_union, first, changed)},
        "only_the_neighbour_uses_an_asset": {"note": "the preflight must cover an asset that only the other deck uses (plan 8.4, condition 2)",
                                              "outcome": attempt(resource_union, first, only_neighbour)},
    }


def tracer_case(facts):
    """Plan 13, tracer bullet: the public bundle with one crossDissolve, center, D = 12, hold; cut C = 40, window [34, 46)."""
    edit = corpus_edit(facts)
    by_id = {item["id"]: item for item in edit["shots"]}
    first, second = by_id["shot-a"], by_id["shot-b"]
    cut, duration, frame = second["start"], 12, 37
    window = checked_window(cut, duration, "center", edit["edit_duration"])
    k = frame - window["start"]
    progress = progress_of(k, duration, "easeInOutCubic")
    weight = weight_of(progress, "smooth", 1.0)
    primary = "from" if frame < cut else "to"
    top, neighbour = (first, second) if primary == "from" else (second, first)
    neighbour_row = resolve_shot_at_offset(neighbour, edit["instances"][neighbour["instance"]], edit["camera_frames"][neighbour["camera"]], frame - neighbour["start"])
    return {
        "kind": "origin-formula", "covers": ["T05", "T07", "T10", "T16"], "bundle_sha256": facts["bundle_sha256"],
        "transition": {"id": "tr-1", "effect": "crossDissolve", "fromShotId": "shot-a", "toShotId": "shot-b", "durationFrames": duration,
                       "alignment": "center", "handles": "hold", "easing": "easeInOutCubic", "params": {"curve": "smooth", "intensity": 1}},
        "cut": cut, "window": window, "edit_frame": frame, "frame_in_window": k, "primary": primary,
        "other_shot": neighbour["id"], "other_deck_frame": neighbour_row, "progress": r9(progress), "weight_not_stored_in_the_plan": r9(weight),
        "pixel_constant_decks": {"deck_a": [200, 128, 16], "deck_b": [40, 128, 240], "expected": mix_pixel([200, 128, 16], [40, 128, 240], progress, "smooth", 1.0)},
        "alpha_note": "the bundle's renderProfile.alphaMode is %s: the case is Tessel contract (opaque output), NOT accepted by the origin (UNSUPPORTED_ALPHA)" % facts["bundle_alpha_mode"],
        "edit_clock": "the neighbour's scene clock (light, flicker) runs at the REAL edit frame %d, not at its held instance frame" % frame,
    }


def build_corpus(facts):
    problems = self_check()
    if problems:
        raise Refusal("ORACLE_SELF_CHECK", "; ".join(problems))
    if not facts["codec_tolerance_is_16"]:
        raise Refusal("ORACLE_TARGET_FACT", "CODEC_TOLERANCE is no longer 16 in scene_fx_export.rs")
    if not facts["f016_codec_text_present"]:
        raise Refusal("ORACLE_TARGET_FACT", "F016 (effects.md) no longer states the codec tolerance the corpus imports")
    if not (facts["premultiplied_refused"] and facts["dissolve_unsupported_in_parser"] and facts["legacy_expected_message_unchanged"]):
        raise Refusal("ORACLE_TARGET_FACT", "the alpha facts about the destination changed")
    return {
        "schema": SCHEMA,
        "note": "Generated by scripts/pilot-dissolve-corpus.py; edit nothing by hand (verify fails). An oracle, not the feature (F064).",
        "formula": {
            "t": "(f - start + 0.5) / D, f the edit frame, D durationFrames, sampled at the middle of the frame",
            "progress": "ease(t) with easing in linear|easeInOutCubic|easeOutCubic",
            "shaped": "mix(progress, smoothstep(0, 1, progress), curve == smooth)",
            "weight": "mix(progress, shaped, clamp01(intensity))",
            "pixel": "out = mix(mix(a, b, progress), mix(a, b, shaped), amount) in linear light; decks are 8-bit sRGB codes",
            "alpha": "output alpha is 255",
        },
        "constants": {"alignments": ALIGNMENTS, "handles": HANDLE_MODES, "easings": EASINGS, "curves": CURVES,
                      "defaults_of_a_new_transition": {"alignment": "center", "handles": "hold", "easing": "easeInOutCubic", "curve": "smooth", "intensity": 1},
                      "typical_duration_frames": 24, "seed_max": SEED_MAX, "safe_integer_max": SAFE_MAX, "progress_exact_duration_limit": PROGRESS_EXACT_LIMIT},
        "tolerances": TOLERANCES,
        "windows": window_cases(),
        "strict_inputs": strict_cases(),
        "progress": progress_cases(),
        "quantization": quantization_cases(),
        "pixels": pixel_cases(),
        "deck_frames": deck_frame_cases(facts),
        "shot_clip_frames": shot_clip_cases(),
        "handles": handle_cases(facts),
        "explain": explain_cases(facts),
        "ids": id_cases(),
        "keys_hidden_seed": keys_and_hidden_cases(),
        "fps": fps_cases(),
        "alpha": alpha_cases(facts),
        "tracer_bullet": tracer_case(facts),
        "limits": limits_cases(),
        "resource_union": resource_union_cases(facts),
    }


# ---- provenance -----------------------------------------------------------------------------------------


def sha256_of(data):
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    return (json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False, allow_nan=False) + "\n").encode("utf-8")


class OriginUnavailable(Exception):
    pass


def git(origin, *args, check=True):
    try:
        done = subprocess.run(["git", "-C", str(origin), *args], capture_output=True, timeout=120)
    except (OSError, subprocess.SubprocessError) as error:
        raise OriginUnavailable("git could not run: %s" % error) from error
    if check and done.returncode != 0:
        raise OriginUnavailable("git %s failed: %s" % (" ".join(args[:2]), done.stderr.decode("utf-8", "replace").strip()))
    return done


def read_tip(origin):
    """Every source at the tip: path -> (blob id, bytes). Read-only git (rev-parse, cat-file)."""
    git(origin, "cat-file", "-e", TIP + "^{commit}")
    found = {}
    for path in [p for p, _ in SOURCES] + [p for p, _ in TIP_ONLY]:
        blob = git(origin, "rev-parse", "%s:%s%s" % (TIP, SE, path)).stdout.decode().strip()
        found[path] = (blob, git(origin, "cat-file", "blob", blob).stdout)
    return found


def check_origin(origin, require_pin):
    """Returns (findings, warnings). `origin-blob verified` is never `origin executed`."""
    findings, warnings = [], []
    found = read_tip(origin)
    texts = {}
    for path, (blob, data) in found.items():
        texts[path] = data.decode("utf-8")
        recorded = TIP_DIGEST.get(path)
        if recorded is None:
            findings.append("no recorded digest for the tip source (run `digests`): " + path)
        elif (blob, sha256_of(data), len(data)) != tuple(recorded):
            findings.append("the tip blob changed or was tampered with: " + path)
        attested = ATTESTED.get(path)
        if attested is not None and attested[1] != blob:
            findings.append("the tip blob is not the one the coordinator attested: " + path)
    findings += check_against(texts)
    pin_commit = git(origin, "cat-file", "-e", PIN + "^{commit}", check=False).returncode == 0
    if not pin_commit:
        warnings.append("PIN_COMMIT_ABSENT: the pin %s is not a commit of this origin clone; its blobs rest on the coordinator's attestation" % PIN)
    unverified = []
    for path, (pin_blob, tip_blob) in ATTESTED.items():
        if pin_commit:
            actual = git(origin, "rev-parse", "%s:%s%s" % (PIN, SE, path), check=False)
            if actual.returncode != 0 or actual.stdout.decode().strip() != pin_blob:
                findings.append("the pin blob is not the attested one: " + path)
            continue
        if pin_blob == tip_blob:
            continue  # the same object id: verified by the tip read, but its membership in the pin is attested
        if git(origin, "cat-file", "-e", pin_blob, check=False).returncode == 0:
            warnings.append("PIN_BLOB_PRESENT_BY_ID: %s (%s) exists as an object; origin-blob verified, not origin-executed" % (path, pin_blob[:10]))
        else:
            unverified.append(path)
            warnings.append("PIN_BLOB_UNVERIFIED: %s (%s) is not an object here; attested by the coordinator only" % (path, pin_blob[:10]))
    if require_pin and not pin_commit:
        findings.append("PIN_UNVERIFIED")
    return findings, warnings


def build_manifest(corpus_bytes, corpus, facts):
    sources = []
    for path, role in SOURCES:
        pin_blob, tip_blob = ATTESTED[path]
        blob, digest, size = TIP_DIGEST[path]
        sources.append({"path": SE + path, "role": role, "group": "attested-by-the-coordinator-and-read-at-the-tip",
                        "pin_blob_attested": pin_blob, "tip_blob": blob, "pin_equals_tip": pin_blob == tip_blob,
                        "tip_sha256": digest, "tip_bytes": size, "tip_blob_is_the_attested_one": tip_blob == blob})
    for path, role in TIP_ONLY:
        blob, digest, size = TIP_DIGEST[path]
        sources.append({"path": SE + path, "role": role, "group": "read-at-the-tip-only-not-compared-with-the-pin", "tip_blob": blob, "tip_sha256": digest, "tip_bytes": size})
    counts = {name: len(corpus[name]) if isinstance(corpus[name], (list, dict)) else 0 for name in ("explain", "handles")}
    counts["pixel_cases"] = len(corpus["pixels"]["cases"])
    counts["pixel_grid_probes"] = corpus["pixels"]["grid"]["probes_per_channel"]
    counts["window_rows"] = len(corpus["windows"]["raw"])
    counts["progress_tables"] = len(corpus["progress"]["tables"])
    return {
        "schema": SCHEMA,
        "purpose": "Provenance of the pilot crossDissolve corpus (dissolve-cases.json): where each expectation comes from. An oracle for P4b-S0; not the feature.",
        "authoring": {
            "plan": "docs/migration/p4b-cross-dissolve-plan.md, version 2",
            "plan_head_read": PLAN_HEAD,
            "plan_head_note": "the head of the PR #58 branch whose plan this corpus follows; a later plan head may change the contract and then this corpus is regenerated and re-reviewed, never edited",
            "interpreter": "recorded in the S0 report (version, prefix, executable digest), not here: the JSON must not depend on the host",
        },
        "origin": {
            "repository": "tesselapp (read only; never installed, built or executed)",
            "pin_named_by_the_plan": PIN,
            "tip_commit_read": TIP,
            "access": "git rev-parse / cat-file of the blobs listed under `sources` (no fetch, no checkout, no execution)",
            "what_the_windows_author_read": "the TIP blobs, whose ids and sha256 this script re-checks with `--origin`",
            "what_the_coordinator_attested": "the Mac coordinator compared 21 files of the pin with the tip by git object ids: all identical except fx/fx-schema.ts (a comment only: CLIP_OVERLAP -> CLIP_PAST_END) and schema/render-bundle.schema.ts (a behaviour difference in the narration part, at the tip, not a comment; not used by any rule of this corpus except the alphaMode enum and the legacy dissolve shape, both re-checked at the tip). Not re-verified here: the pin commit is not in the clone of the Windows author.",
            "observed_when_authoring": {"host": "windows-x64 author clone", "pin_commit_present": False},
            "origin_executed": False,
        },
        "corpus": {
            "file": CASES_NAME,
            "sha256": sha256_of(corpus_bytes),
            "generator": "scripts/pilot-dissolve-corpus.py",
            "regenerate": "scripts/pilot-dissolve-corpus.py generate --origin <path to the origin repo>",
            "check": "scripts/pilot-dissolve-corpus.py generate --check   (writes nothing; offline)",
            "counts": counts,
            "repository_fixture_read": {"path": TARGET_FILES["bundle"], "sha256": facts["bundle_sha256"]},
        },
        "expectation_kinds": {
            "origin-formula": "read from the origin text and evaluated by this script; the origin was not run",
            "origin-spec-vector": "a number or message the origin's own spec pins (re-checked in the origin text at generation and by `--origin`)",
            "destination-contract-candidate": "what the plan proposes for the destination where it differs from the origin; a candidate, not a result",
            "destination-contract-declared": "the destination's declared contract (F064 draft); pending review",
            "destination-hardening": "stricter input rules of the oracle; its codes start with ORACLE_ and are not product codes",
        },
        "tolerances_registered": {"in": "dissolve-cases.json / tolerances", "status": TOLERANCES["status"],
                                  "monitor_and_render_max_delta": TOLERANCES["monitor_and_render"]["max_abs_delta_8bit"],
                                  "bias_limit": TOLERANCES["monitor_and_render"]["bias_limit"]["max_abs_mean_signed_delta_8bit"],
                                  "export_h264_max_delta": TOLERANCES["export_h264"]["max_abs_delta_8bit"],
                                  "export_h264_source": TOLERANCES["export_h264"]["source"],
                                  "plan_ceilings": {key: TOLERANCES["plan_ceilings"][key] for key in ("MAX_TRANSITION_PLAN_BYTES", "MAX_TRANSITION_PLAN_ITEMS", "TRANSITION_PLAN_MAX_DEPTH")},
                                  "plan_ceilings_status": TOLERANCES["plan_ceilings"]["status"], "not_validated_on_pixels": True},
        "assumptions": [
            {"id": "CAMERA_FRAMES_IS_SUM_OF_SEGMENT_DURATIONS", "detail": "camera/resolve-camera-clip.ts was not read; the origin handle spec is consistent with a clip of N poses having tail N - (cameraIn + shotLength), and the fixture bundle's clips have one segment."},
            {"id": "RECONSTRUCTED_SPEC_FIXTURE", "detail": "fx-test-bundle.ts was not read; the three-shot edit is rebuilt from the numbers the origin specs show and only those are claimed."},
            {"id": "OUTPUT_ALPHA_255", "detail": "the shader writes alpha 1.0; the destination's output alpha is assumed opaque. Not measured."},
            {"id": "ROUNDING_HALF_UP", "detail": "decks and output are rounded half-up; GPU rounding is not modelled beyond the 1/255 + bias gate."},
        ],
        "documented_differences": [
            {"id": "DECKS_8_BIT", "detail": "The origin mixes half-float linear targets; the destination mixes 8-bit sRGB decks. The oracle models the destination's 8-bit decks and quantized output, so equivalence is by formula, not by the origin's pixels."},
            {"id": "ALPHA_VERDICT", "detail": "The origin refuses (UNSUPPORTED_ALPHA) any non-opaque profile that has a transition, a legacy dissolve or a clip; the destination accepts straight and renders opaque. A declared divergence of verdict (plan Q3), pending review; it already holds for the three looks."},
            {"id": "KEYS_REFUSED", "detail": "Keys on a transition are accepted by the origin and refused by the plan's destination (FX_TRANSITION_KEYS_NOT_YET)."},
            {"id": "NARRATION_OUT_OF_SCOPE", "detail": "At the tip, render-bundle.schema.ts imports editFrameToSeconds and calls explainNarration with the duration: the NARRATION behaviour differs from the pin (a behaviour difference, not a comment). The contract is valid WITHOUT narration (the editor has none: BL017/BL026); TRANSITION_OVER_SPEECH is N/A until narration exists. Every other rule used here is the same at the pin and at the tip."},
            {"id": "LEGACY_FIXTURES_UNTOUCHED", "detail": "The legacy corpus (invalid-dissolve-transition, DISSOLVE_UNSUPPORTED) is history of an older pin and is not edited; the legacy dissolve is a separate S6 track with its own oracle lane (plan D04, Q2). Nothing here changes a legacy byte."},
            {"id": "HIDDEN_FALSE", "detail": "The coordinator chose true or absent for transition hidden, matching the origin. Explicit false must be refused structurally in S1a; look clip compatibility is unchanged."},
            {"id": "STRICT_INTEGERS", "detail": "The oracle refuses floats, booleans, strings and integers beyond 2^53 - 1 for frame fields (ORACLE_* codes); the origin's schema only requires `int()`."},
            {"id": "DURATION_CHECKED_FIRST", "detail": "D is compared with the edit before the window arithmetic (same code, TRANSITION_OUT_OF_RANGE, as the origin gives for any window that cannot fit)."},
        ],
        "typed_codes_not_in_the_origin": ["ORACLE_NOT_AN_INTEGER", "ORACLE_INTEGER_OUT_OF_I64", "ORACLE_INTEGER_NOT_SAFE", "ORACLE_ARITHMETIC_OVERFLOW", "ORACLE_BAD_SEED",
                                          "ORACLE_UNKNOWN_ALIGNMENT", "ORACLE_BAD_EDIT", "ORACLE_PROGRESS_DURATION", "ORACLE_FRAME_NOT_IN_WINDOW", "ORACLE_BAD_COLOUR",
                                          "ORACLE_TIE_MARGIN", "ORACLE_KEYS_NOT_MODELLED", "ORACLE_LOOK_PARAMS_NOT_MODELLED"],
        "exit_codes": {"0": "verified", "1": "finding", "2": "usage", "3": "ORIGIN_UNAVAILABLE", "4": "PIN_UNVERIFIED (with --require-pin)"},
        "sources": sources,
        "blockers": [
            {"id": "ORIGIN_GOLDEN_NOT_COMPARABLE", "detail": "The origin golden (catalog/__golden__/crossDissolve.png) needs the origin's harness. Executing the origin is forbidden here. No golden was read or compared.",
             "consequence": "Pixel equivalence is `origin-formula`, never golden equivalence. Visual parity of the dissolve stays partial."},
            {"id": "ORIGIN_NOT_EXECUTED", "detail": "Expectations are transcribed formulas, not the origin's outputs. Mitigation: the transcribed lines and constants are checked against the origin text at generation and by `verify --origin`.",
             "consequence": "`origin-blob verified` is not `origin executed`."},
            {"id": "PIN_COMMIT_ABSENT_FOR_THE_WINDOWS_AUTHOR", "detail": "The pin commit is not in the author's clone of the origin; the pin blob ids are the coordinator's attestation.",
             "consequence": "`verify --origin --require-pin` exits 4 where the pin cannot be checked."},
            {"id": "NOT_THE_FEATURE", "detail": "No Rust, JSONL, runtime or UI exists for transitions. A Python oracle is not the library, the headless CLI or the window (F064 Pending there).",
             "consequence": "No parity is claimed for the destination."},
        ],
    }


def paths(directory):
    return directory / CASES_NAME, directory / MANIFEST_NAME


def expected_bytes():
    facts = repo_facts()
    corpus = build_corpus(facts)
    corpus_bytes = canonical(corpus)
    return corpus_bytes, canonical(build_manifest(corpus_bytes, corpus, facts))


def say(stream, text):
    print(text, file=stream)


def report(findings, warnings):
    for warning in warnings:
        say(sys.stderr, "WARNING: " + warning)
    for finding in findings:
        say(sys.stderr, "FINDING: " + finding)


def cmd_digests(origin):
    try:
        found = read_tip(origin)
    except OriginUnavailable as error:
        say(sys.stderr, "ORIGIN_UNAVAILABLE: %s" % error)
        return EXIT_ORIGIN_UNAVAILABLE
    for path in sorted(found):
        blob, data = found[path]
        say(sys.stdout, '    "%s": ("%s", "%s", %d),' % (path, blob, sha256_of(data), len(data)))
    return EXIT_OK


def cmd_generate(origin, directory, check, require_pin):
    if require_pin and origin is None:
        say(sys.stderr, "PIN_UNVERIFIED: --require-pin needs --origin PATH; offline the pin is not checked")
        return EXIT_PIN_UNVERIFIED
    if origin is None and not check:
        say(sys.stderr, "ORIGIN_UNAVAILABLE: `generate` re-checks the origin text it transcribes; pass --origin PATH (or use --check offline)")
        return EXIT_ORIGIN_UNAVAILABLE
    findings, warnings = [], []
    if origin is not None:
        try:
            findings, warnings = check_origin(origin, require_pin)
        except OriginUnavailable as error:
            say(sys.stderr, "ORIGIN_UNAVAILABLE: %s" % error)
            return EXIT_ORIGIN_UNAVAILABLE
    if findings:
        report(findings, warnings)
        return EXIT_PIN_UNVERIFIED if findings == ["PIN_UNVERIFIED"] else EXIT_FINDING
    try:
        corpus_bytes, manifest_bytes = expected_bytes()
    except Refusal as refusal:
        say(sys.stderr, "FINDING: the oracle refused to build the corpus: %s" % refusal)
        return EXIT_FINDING
    cases_path, manifest_path = paths(directory)
    if check:
        differences = []
        for path, wanted in ((cases_path, corpus_bytes), (manifest_path, manifest_bytes)):
            if not path.exists():
                differences.append("missing: %s" % path.name)
            elif path.read_bytes() != wanted:
                differences.append("%s is not what the generator produces today" % path.name)
        report(differences, warnings)
        if differences:
            return EXIT_FINDING
        say(sys.stdout, "pilot-dissolve --check: the generator reproduces both files byte for byte (nothing written)")
        return EXIT_OK
    directory.mkdir(parents=True, exist_ok=True)
    cases_path.write_bytes(corpus_bytes)
    manifest_path.write_bytes(manifest_bytes)
    report([], warnings)
    say(sys.stdout, "wrote %s and %s" % (cases_path.name, manifest_path.name))
    return EXIT_OK


def cmd_verify(origin, directory, require_pin):
    if require_pin and origin is None:
        say(sys.stderr, "PIN_UNVERIFIED: --require-pin needs --origin PATH; offline the pin is not checked")
        return EXIT_PIN_UNVERIFIED
    cases_path, manifest_path = paths(directory)
    findings, warnings = [], []
    if not cases_path.exists() or not manifest_path.exists():
        say(sys.stderr, "FINDING: missing corpus or manifest: run `generate`")
        return EXIT_FINDING
    corpus_bytes = cases_path.read_bytes()
    manifest_bytes = manifest_path.read_bytes()
    try:
        manifest = json.loads(manifest_bytes.decode("utf-8"))
        recorded = manifest["corpus"]["sha256"]
    except (ValueError, KeyError, TypeError) as error:
        say(sys.stderr, "FINDING: the manifest is not readable: %s" % error)
        return EXIT_FINDING
    if sha256_of(corpus_bytes) != recorded:
        findings.append("dissolve-cases.json was edited: its sha256 is not the manifest's")
    try:
        wanted_corpus, wanted_manifest = expected_bytes()
    except Refusal as refusal:
        findings.append("the oracle refused to rebuild the corpus: %s" % refusal)
        wanted_corpus = wanted_manifest = None
    if wanted_corpus is not None and corpus_bytes != wanted_corpus:
        findings.append("dissolve-cases.json is not what the generator produces today (formulas, cases or fixtures changed)")
    if wanted_manifest is not None and manifest_bytes != wanted_manifest:
        findings.append("dissolve-manifest.json is not what the generator produces today (edited, or its sources changed)")
    blockers = {item.get("id") for item in manifest.get("blockers", [])} if isinstance(manifest, dict) else set()
    for needed in ("ORIGIN_GOLDEN_NOT_COMPARABLE", "ORIGIN_NOT_EXECUTED"):
        if needed not in blockers:
            findings.append("blocker dropped from the manifest: " + needed)
    if b"origin-output" in corpus_bytes:
        findings.append("the corpus calls an expectation an origin output")
    if origin is not None:
        try:
            more, warnings = check_origin(origin, require_pin)
        except OriginUnavailable as error:
            report(findings, [])
            say(sys.stderr, "ORIGIN_UNAVAILABLE: %s" % error)
            return EXIT_ORIGIN_UNAVAILABLE
        findings += more
        pin_absent = any(warning.startswith("PIN_COMMIT_ABSENT:") for warning in warnings)
        tier = "pin NOT verified (attestation only)" if pin_absent else "pin commit verified for %d attested sources; %d tip-only sources" % (len(ATTESTED), len(TIP_ONLY))
        mode = "against the origin blobs (origin-blob verified; %s; the origin was NOT executed)" % tier
    else:
        mode = "offline (origin blobs not re-read; pass --origin to re-check them)"
    report(findings, warnings)
    if findings:
        return EXIT_PIN_UNVERIFIED if findings == ["PIN_UNVERIFIED"] else EXIT_FINDING
    say(sys.stdout, "pilot-dissolve corpus verified %s: %d sources" % (mode, len(manifest["sources"])))
    return EXIT_OK


def main(argv=None):
    problem = interpreter_problem()
    if problem:
        say(sys.stderr, "FINDING: INTERPRETER_NOT_REAL: %s" % problem)
        return EXIT_BAD_INTERPRETER
    say(sys.stderr, "python: %s" % sys.version.replace("\n", " "))
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    for name in ("generate", "verify", "digests"):
        command = sub.add_parser(name)
        command.add_argument("--origin", type=Path, default=None, required=(name == "digests"))
        if name != "digests":
            command.add_argument("--out", type=Path, default=None)
            command.add_argument("--require-pin", action="store_true")
        if name == "generate":
            command.add_argument("--check", action="store_true")
        if name == "verify":
            command.add_argument("--check", action="store_true")  # accepted for symmetry: verify never writes
    args = parser.parse_args(argv)
    if args.command == "digests":
        return cmd_digests(args.origin)
    directory = args.out or CORPUS
    if args.command == "generate":
        return cmd_generate(args.origin, directory, args.check, args.require_pin)
    return cmd_verify(args.origin, directory, args.require_pin)


if __name__ == "__main__":
    sys.exit(main())
