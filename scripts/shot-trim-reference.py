#!/usr/bin/env python3
"""Independent reference (oracle X) of the P5 shot durations and cuts (BL082.5.1, F065).

Plan: `docs/migration/p5-shot-duration-plan.md` (S0). Contract of the origin, copied literally:
`docs/migration/origin-specs/scene-shot-trim-plan.md` and `scene-shot-trim-decisions.md` (decisions `#1`..`#34`).

WHAT THIS IS. A transcription, in plain Python, of the rules those two documents state for four operations on a
minimal document: `trim_shot`, `move_cut`, `set_shot_duration` and `set_edit_duration`, with the options `mode`,
`content`, `keep_absolute`, `shrink_transitions` and `with_narration`. It is the oracle X of the Rust planner of S1:
it was written WITHOUT reading any Rust of `crates/` (only the `DEFERRED_FIELDS` list of `directed_scene/mod.rs`,
for the audit). The rules come from the two origin documents and from the schema and validation of the origin
(pin `d6adac15`, plus the `fx/` and `resolve-shot-at-offset` files that only exist on the tip `3655790`, see MANIFEST).

WHAT THIS IS NOT. It is not the feature (nothing here is in the editor), and it is not the origin: the pilot code of
the plan commands (`packages/scene-engine/src/shots/`) is NOT in the clone, so the origin was never executed
(`ORIGIN_NOT_EXECUTED`). Where the two documents are silent or ambiguous the oracle picks a reading and says so in
`ORACLE_CHOICES` (and in the manifest); the corpus never depends on an ambiguity unless it is tagged with it.

    scripts/shot-trim-reference.py              # (re)writes cases.json and manifest.json
    scripts/shot-trim-reference.py --check      # regenerates in memory and compares byte for byte; writes nothing
    scripts/shot-trim-reference.py --verify-origin PATH   # re-reads the origin blobs by git id (read only)

Exit codes: 0 verified | 1 a difference or a failed origin check | 2 usage | 5 the interpreter is not a real Python.
Python standard library only, 3.9 to 3.13. No bare `assert` is used for validation: the file behaves the same under
`python -O`.
"""

import argparse
import copy
import hashlib
import json
import subprocess
import sys
from fractions import Fraction
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FIXTURE_DIR = ROOT / "docs/migration/fixtures/shot-trim"
CASES_NAME = "cases.json"
MANIFEST_NAME = "manifest.json"
GENERATOR_VERSION = "shot-trim-reference/1"
SCHEMA = 1
MAX_EDIT_FRAMES = 216000  # decision #18: one hour at 60 fps
MIN_PYTHON = (3, 9)
EXIT_OK, EXIT_FINDING, EXIT_USAGE, EXIT_BAD_INTERPRETER = 0, 1, 2, 5

PIN = "d6adac15c61f4ef582a7ec259397b2e025d1f059"
TIP = "365579076782c1fc19c6464f3bc0199b30c0ab8f"
ORIGIN_SRC = "packages/scene-engine/src/"
ORIGIN_DOCS = [
    "docs/migration/origin-specs/scene-shot-trim-plan.md",
    "docs/migration/origin-specs/scene-shot-trim-decisions.md",
]

# (which commit, path under ORIGIN_SRC, git blob id, sha256 of the blob). `pin` is `d6adac15` (the main of the
# origin, no `fx/`), `tip` is `3655790` (the effects branch the P4b oracle already reads). A file is read at the
# commit that has it; the blob ids are what `--verify-origin` re-checks, read only, by `git cat-file`.
ORIGIN_SOURCES = [
    ("pin", "schema/render-bundle.schema.ts", "d4b10d3617d7baece759c21080d8defb4b2fcc00", "953ddfbdfca5dc98a2f3baa29ddbc59f26922131bfa32b998b1f70bc107e0f30"),
    ("pin", "validate/bundle-integrity.ts", "d45b0e302abcac5fb480c3734b6ba0dc7b1d5b57", "ddb33e6fca2ede35a7a3eaa3d060629558c7b8351cff29aa9aaf838676dd2b56"),
    ("pin", "validate/edit-joins.ts", "32985a9140c4d9f8b14e9724ad70c6d8459e66cf", "14a5c2486820b1dd361a34180e0ca1bee5b0deefef4792aabe7207905396bedd"),
    ("pin", "edit/resolve-shot-at-edit-frame.ts", "ef8fffa87ab1f31456742c0171ec52a920617194", "bc9e72b01b6149b39adf8d062100e5a79956439c4188bee343697a07892c2b28"),
    ("pin", "evaluator/evaluate-edit-frame.ts", "bb178c234c592d3bff261942670d7d249c89acbd", "653513af4ca6c8ee5b775970ce9d7afe63d718d9e8618cfe7aea83867b6cb46d"),
    ("pin", "performance/resolve-performance.ts", "a5212bda7f4c966852584f07e5d048cbc808ee24", "3e3f2f4f888a48eddf00bae6d712995035f1e7e74dd58f938a003e6fabeedcc7"),
    ("pin", "keyframes/keyframe-rules.ts", "aaa7d0c87e7cf54f0f21a937488d2f7867a5fbb0", "e1beb696ba477a40b0ff0188acc3e18fee79180aaf8f79f0c2eac2bf14f74ae2"),
    ("pin", "narration/narration-schema.ts", "b9aee27cb16ade8599276e71556843e379586357", "0af031103fcef0eacb761d14342cebebff976253556a783baeaa2123e6fec461"),
    ("pin", "outline/outline-schema.ts", "eea86648083ebbd7a6f853d950481d9f8f9465f8", "00305d09ef920188fbe47a4f038ea2814b5dc1f7ea48451ae3abb8298b7c57b4"),
    ("pin", "effects/scene-effects.ts", "616d018bb3d50f793376c10c6b911fdc3406a3e3", "d76e0bce25684d1dda57beb2181380f23188182ea93621893c011352277effa4"),
    ("pin", "rig/rig-schema.ts", "648eccb920752fc9bd0abd39518c29f772193192", "ea094dccfd1c69bf452672ae4fc11658b06cc37eea9c8493f9675955bb5be834"),
    ("pin", "rig/follow-schema.ts", "a3f2ffc0dc4fa421dee9d38d02a009add95abcd5", "b437822b5c62c46d670c626afc136fca934c2eca9ca33aa9617dfdc17d40048c"),
    ("pin", "rig/rig-targets.ts", "c2f9affe47c221b36843e401ab127b8dc6a8a685", "dfcd1c93afd89d853fcd18c388ab049b725a0724b3b4114442ae0e89ce1d4dff"),
    ("pin", "path/path-schema.ts", "e0780672769dbcba2027391d3f10b651cfe10915", "79b53b9f089a795720ee7789a7a53e72283b847d3522fd7bb3fd724df9d63e15"),
    ("pin", "weather/weather-keys.ts", "96c34bcefad88c1c54a6e6aa9a215c91d6669f29", "28e54ca8fa5f6a69c7174da2da57c94cecc00d4168d3097cfa5f8c3a82413985"),
    ("pin", "lighting/light-keys.ts", "240d4a59309b51db4bea4a691b990b4399014119", "5df1b807e779c5f36b096e3215cf72e0401b42762f0b6e75e102a4f7ab61da1f"),
    ("pin", "runtime/three-scene-runtime.ts", "e57f90df88571f904f8e519196a57ab5dbacb673", "787c409e999ca87bfc403fcae85db379633c905566721ab0f1461f28bb58a0fd"),
    ("pin", "runtime/rig-layer.ts", "ed197947236ec267919cb8af253fb889cda53071", "22c77ea8a98617024ca21ad09f52fb809505151264bef79c6a1df5b42bd841b6"),
    ("tip", "edit/resolve-shot-at-offset.ts", "0851da51e1cc8c6a8961f4cd3dcc503382417cdb", "18e82cc298a29e3386c5328364503f31e75a26864a990542c045697c290ab047"),
    ("tip", "edit/resolve-shot-at-offset.spec.ts", "c19160b92b5d3fba32cb1dd979c70410a3f50053", "7a61db5bfe29a8461730e8eecc87ba88a25a1be5faadff81b133bfa68ab0c26b"),
    ("tip", "docs/video-production/scene-fx-plan.md", "0efd01c7f416949bd9ff0d07533f7f3e820f4b02", "5eb282c0b9ea98ba480255ebf1226da24f8125f3d9ba0b7a2644a78d44858069"),
    ("tip", "evaluator/evaluate-edit-frame.ts", "a8d0c64b35a14fd971fa8aa7edd4cd9de5645290", "a67280dd59aea96c05e12c34c04a142db856fd1bc8923f717668607c71daa910"),
    ("tip", "evaluator/evaluate-shot-frame.spec.ts", "534967558b8d6dac94b4f2c6579919eb019dff60", "7452f3e94b539f374dbb138bea4eb9c5a7f76ee765bb4753d35fc358ffb80317"),
    ("tip", "fx/fx-schema.ts", "5c85ce56f21fe22bf7c53314386b0c2124e20361", "4044611e48418de75cd05dd95ae27ae777568156fda92fc6cb6e6b53b8e2a21d"),
    ("tip", "fx/fx-windows.ts", "bcf38615372e8cf830ea21a87159e390f3a7674f", "55db48362fc406695cc2ff08e29cf79cfa7563f851b2f88e39c0a726d70c4bd3"),
    ("tip", "fx/fx-explain-clips.ts", "0cb7607fe7010c6f2c50159cb2c53219f4e50abc", "10a447cc467093e4a311c51ad7e64c6637d2952b5cc2d255539a1999f80ab61c"),
    ("tip", "fx/fx-explain-transitions.ts", "f075eec56b82012ad9facfacf68cad8ecdb36805", "6aa0dd88253702b91a13cc5a57c4e977257d79f272c791a6c2f0e1123acfaabf"),
    ("tip", "fx/fx-keys.ts", "f4ecbfbbce4e5e4aa5a6cb4bf9669f505a1922a0", "4d174b21f8440b50a67c44c99c8c46f557b0f3435042aaad2297288052d69b5d"),
    ("tip", "fx/fx-command-clips.ts", "3fe9ccf1bdcf669c3249f590848d0e4417f2fc62", "e4e0e9c9c3c3b4e2b230f451bec76c594839a5e9b1432bc17511e47f22ef84b8"),
    ("tip", "fx/fx-command-step.ts", "8e2aacd77d6b9f79baa74653d61f0aaef81a2da0", "594a2a71bb82587cb9164d1c5a8ace7b5fe49eb74f08c9f01e0e9e035cbf8381"),
    ("tip", "fx/fx-command-transitions.ts", "bde079b7fe26b3c25fb0ef6fe931f2233a1a9b8e", "fb1006ff87527afb0696eff165d1055976a68d5416fd00d927eeeafdb354749b"),
    ("tip", "fx/resolve-fx-frame.ts", "44c4c11f8725450e9c05bf835a273c57bcca8e0c", "20fac1cb27ac7d2cb77e2f93f2163312525057ac2defb2ff1bf986c5d015ea18"),
]



def origin_path(path):
    """The path inside the origin repository: sources live under `packages/scene-engine/src/`, docs by full path."""
    if path.startswith("docs/"):
        return path
    return ORIGIN_SRC + path


# The one list read from the destination: the fields the Rust importer keeps without interpreting
# (`crates/core/src/directed_scene/mod.rs`, `DEFERRED_FIELDS`, lines 81-100 of the base `de83087`). `lighting` is
# audited elsewhere (decision D11) and is not here.
DEFERRED_FIELDS_SOURCE = "crates/core/src/directed_scene/mod.rs:81-100"

# ---------------------------------------------------------------------------------------------------------
# The audit of the deferred fields (decision D12, question Q4). `clock` is one of: edit-frames, edit-seconds,
# instance-frames, clip-frames, none. `destination`: remap (edit frames in the shape [{frame: int, ...}], rule #15),
# refuse (SHOTS_LAYER_NOT_YET: edit time in another shape, or an uncertain clock) or stays (no time, or the clock of
# the scene or of a clip, #5; narration is #16/D13). The evidence is `commit:path:line` of the origin.
# ---------------------------------------------------------------------------------------------------------

DEFERRED_AUDIT = [
    {"field": "narration", "clock": "edit-seconds", "shape": "narration.tracks[].clips[].startSeconds / durationSeconds / fromMix.{startSeconds,durationSeconds} (float seconds)", "destination": "stays",
     "why": "Real edit seconds, but in another shape and moved only with `withNarration` (#16, D13): without it no line moves, and the option is refused here (SHOTS_OPTION_NOT_YET).",
     "evidence": ["pin:narration/narration-schema.ts:4-5", "pin:narration/narration-schema.ts:24-25", "pin:narration/narration-schema.ts:49-58"]},
    {"field": "outline", "clock": "none", "shape": "groups and items: ids, labels, order (no frame, no second)", "destination": "stays",
     "why": "Nothing to move (#23).",
     "evidence": ["pin:outline/outline-schema.ts:28-54"]},
    {"field": "metadata (only `pilotFrameAudit` is checked)", "clock": "none", "shape": "free record; only pilotFrameAudit is read, and only to refuse it", "destination": "stays",
     "why": "The origin reads no time in it.",
     "evidence": ["pin:schema/render-bundle.schema.ts:515", "pin:validate/bundle-integrity.ts:896-900"]},
    {"field": "sceneDefinitions[].contactPlaneId", "clock": "none", "shape": "a node id", "destination": "stays",
     "why": "No time.",
     "evidence": ["pin:schema/render-bundle.schema.ts:246"]},
    {"field": "sceneDefinitions[].frameAudit", "clock": "none", "shape": "contacts, coplanar groups, draw order, coverage margin: pixels and scene units ('frame' is the picture frame)", "destination": "stays",
     "why": "No time.",
     "evidence": ["pin:schema/render-bundle.schema.ts:211-229", "pin:schema/render-bundle.schema.ts:247"]},
    {"field": "sceneDefinitions[].weather", "clock": "none", "shape": "a name with an intensity (\"snow:0.6\")", "destination": "stays",
     "why": "The name carries no frame; its intensity is replaced by `weatherKeys` over time, and the settling snow integrates the intensity of every earlier edit frame, so what is drawn after an edited point changes on purpose.",
     "evidence": ["pin:schema/render-bundle.schema.ts:250-259", "pin:weather/weather-keys.ts:36-58"]},
    {"field": "sceneDefinitions[].weatherKeys", "clock": "edit-frames", "shape": "weatherKeys[].frame (integer, from 0, strictly increasing; {frame, value, easing})", "destination": "remap",
     "why": "Keys 'on the edit's frames'; the runtime reads them with the edit frame. Same shape as a light's keys (#15: after the point by the delta, dropped inside the removed stretch, an empty list is absent).",
     "evidence": ["pin:schema/render-bundle.schema.ts:133-147", "pin:schema/render-bundle.schema.ts:260-267", "pin:runtime/three-scene-runtime.ts:1373-1381", "pin:validate/bundle-integrity.ts:570-575", "pin:keyframes/keyframe-rules.ts:10-44"]},
    {"field": "sceneDefinitions[].effects", "clock": "none", "shape": "an amount 0..1 per look (strict object); the look reads the edit frame only as a seed", "destination": "stays",
     "why": "No time in the data; the drawn grain depends on the edit frame, so it changes after an edited point on purpose.",
     "evidence": ["pin:effects/scene-effects.ts:4-7", "pin:effects/scene-effects.ts:19-37"]},
    {"field": "sceneDefinitions[].weatherGround", "clock": "none", "shape": "{nodeId, top}", "destination": "stays",
     "why": "No time.",
     "evidence": ["pin:schema/render-bundle.schema.ts:284-290"]},
    {"field": "sceneDefinitions[].rigs", "clock": "instance-frames", "shape": "rigs[].constraints[].target.keys[].frame and rigs[].follows[].progress[].frame ('frame of the scene instance')", "destination": "stays",
     "why": "Scene clock (#5): the rig, the baked pose and the follow run on the instance frame, which does not change when a shot grows. Frames past the baked track hold.",
     "evidence": ["pin:rig/rig-schema.ts:43-49", "pin:rig/rig-schema.ts:68", "pin:rig/rig-schema.ts:79", "pin:rig/follow-schema.ts:12-18", "pin:rig/rig-targets.ts:89-93", "pin:runtime/rig-layer.ts:72"]},
    {"field": "sceneDefinitions[].paths", "clock": "none", "shape": "lines in the image px of a layer; the pen's progress is in the follow of a rig", "destination": "stays",
     "why": "No time of its own (the path file has no frame or second).",
     "evidence": ["pin:path/path-schema.ts:3-6", "pin:path/path-schema.ts:28-38", "pin:rig/follow-schema.ts:12-26"]},
    {"field": "sceneDefinitions[].nodes[].surface (only its assetRef.assetId is read)", "clock": "none", "shape": "a map and a camera/floor", "destination": "stays",
     "why": "No time.", "evidence": ["pin:schema/render-bundle.schema.ts:63-84", "pin:schema/render-bundle.schema.ts:192"]},
    {"field": "sceneDefinitions[].nodes[].normalMap (only its assetRef.assetId is read)", "clock": "none", "shape": "a map and its source", "destination": "stays",
     "why": "No time.", "evidence": ["pin:schema/render-bundle.schema.ts:103-110", "pin:schema/render-bundle.schema.ts:196"]},
    {"field": "sceneDefinitions[].nodes[].sceneDepth", "clock": "none", "shape": "a depth grid of a splat and its camera", "destination": "stays",
     "why": "No time.", "evidence": ["pin:schema/render-bundle.schema.ts:86-101", "pin:schema/render-bundle.schema.ts:194"]},
    {"field": "sceneDefinitions[].nodes[].relief", "clock": "none", "shape": "a depth grid of a plane and its camera", "destination": "stays",
     "why": "No time.", "evidence": ["pin:schema/render-bundle.schema.ts:112-131", "pin:schema/render-bundle.schema.ts:198"]},
    {"field": "sceneDefinitions[].nodes[].rigBind", "clock": "none", "shape": "{rigId, mode, boneId?, blendPx?}", "destination": "stays",
     "why": "No time.", "evidence": ["pin:rig/rig-schema.ts:83-91", "pin:schema/render-bundle.schema.ts:200"]},
    {"field": "sceneDefinitions[].nodes[].reveal", "clock": "none", "shape": "{rigId, followId}: the time is in the follow, on the instance clock", "destination": "stays",
     "why": "No time of its own; the follow it names counts instance frames (#5).",
     "evidence": ["pin:rig/follow-schema.ts:37-44", "pin:schema/render-bundle.schema.ts:202"]},
]

# ---------------------------------------------------------------------------------------------------------
# D07 / Q2: when a shot outlasts its content and the instance grows (#6), are the new frames HELD (the last
# frame again) or EVALUATED AS REAL frames of a longer instance? Read at the pin and at the tip (`resolveShotAtOffset`
# and `heldInstance` only exist at the tip, `3655790`); the pilot code of the plan commands is not in the clone.
# ---------------------------------------------------------------------------------------------------------

D07_AUDIT = {
    "question": "Frames added by the growth of an instance (#6): held or evaluated as real?",
    "conclusion": "real",
    "q2_triggered": False,
    "summary": "The frames of an instance grown by #6 are evaluated as real frames of a longer instance. `heldInstance` is not what holds them: it is set only for the OTHER deck of an effects transition, when the wanted instance frame falls outside `[0, durationFrames - 1]` of the (already grown) instance; and inside the shot's own range `resolveShotAtOffset` answers what `resolveShotAtEditFrame` answers. What 'holds' in decision #6 are the channels that hold by themselves (event channels with `holdFinal`, the baked rig pose, the reveal); a `cycle` channel keeps cycling.",
    "findings": [
        {"claim": "The evaluator never clamps the instance frame of a shot's own range: it throws past the end, and passes the real frame to the overrides and to the rig and reveal layers. The integrity rule makes the instance grow so that it never does.",
         "evidence": ["pin:evaluator/evaluate-edit-frame.ts:70-74", "pin:evaluator/evaluate-edit-frame.ts:83-87", "pin:runtime/three-scene-runtime.ts:1430-1438", "pin:validate/bundle-integrity.ts:817-828"]},
        {"claim": "`heldInstance` exists only at the tip, in `resolveShotAtOffset`: it clamps to `[0, instance.durationFrames - 1]` and is true only when that moved the frame; its reader is the other deck of a transition, and inside the range it equals `resolveShotAtEditFrame`.",
         "evidence": ["tip:edit/resolve-shot-at-offset.ts:16-24", "tip:edit/resolve-shot-at-offset.ts:35-44", "tip:edit/resolve-shot-at-offset.ts:54-86", "tip:edit/resolve-shot-at-offset.spec.ts:89-101", "tip:docs/video-production/scene-fx-plan.md:369-376", "tip:docs/video-production/scene-fx-plan.md:578-590"]},
        {"claim": "The channels: `cycle` continues with a modulo of the instance frame (not held); `event` holds its final value by design; the baked rig pose and the reveal hold past the end of their own track.",
         "evidence": ["pin:performance/resolve-performance.ts:64-82", "pin:performance/resolve-performance.ts:17-30", "pin:performance/resolve-performance.ts:84-100", "pin:schema/render-bundle.schema.ts:308-317", "pin:runtime/rig-layer.ts:72-76", "pin:rig/rig-targets.ts:89-93"]},
    ],
    "for_q2": "The plan's D07 behaviour (new frames evaluated as real: `cycle` continues, `event` holds its final value) is what the origin does, so it is NOT a divergence and Q2 is not triggered: the owner has no decision to take. Caveats: decision #6 says 'the runtime holds the last frame ... heldInstance', which reads like a hold of the whole instance; the code does not do that, and the pilot code of #6 (`shots/`) is not in the clone (ORIGIN_NOT_EXECUTED), so the reading is from the text and the code around it. The reference does not evaluate frames: D07 changes no case of the corpus.",
}

# Where the two origin documents are silent or ambiguous, and what this oracle does (mirrored in the manifest).
ORACLE_CHOICES = {
    "AUTO_RESERVE": "Decision #29 writes `a = min(d, instanceStartFrame, cameraInFrame)` minus the transition reserve. The oracle reads it as `a = max(0, min(d, min(instanceStartFrame, cameraInFrame) - reserve))` (the reserve is taken out of the head frames that EXIST, so a short slip is still fully anchored). The literal reading is `max(0, min(d, instanceStartFrame, cameraInFrame) - reserve)`. The two differ when `d` is smaller than the head frames: the corpus has a case tagged AUTO_RESERVE that shows it, and the coordinator should settle it before S1 (no pilot code to execute).",
    "BOUNDARY": "A point (a key) at the frame where frames are inserted moves with them (`frame >= at`), and a clip that starts there moves whole; a clip that ENDS there (exclusive end) does not stretch. For a removed stretch [a, b): a point inside is dropped, a point at or after `b` moves back by `b - a`. The origin documents say 'after the point' and 'inside the removed stretch' without the border.",
    "CHAIN_MIDDLE": "A clip that CONTAINS the removed stretch keeps its keys at the same clip-relative frames (#12 'not re-timed'), pruned at the new end; a clip whose HEAD is cut shifts its keys by the cut (instants kept), pruned below 0 (D09).",
    "EMPTY_CONTAINER": "When the last key track of a light is removed, its `keys` object is removed too (the list rule of #15 says 'undefined'; the container is not mentioned).",
    "REFUSAL_ORDER": "When two rules refuse the same op the oracle reports, in order: SHOTS_OPTION_NOT_YET, SHOT_NOT_FOUND, SHOTS_NOT_ADJACENT, SHOTS_LAYER_NOT_YET, SHOT_TOO_SHORT, CONTINUOUS_JOIN, NO_HEAD_FRAMES, EDIT_TOO_LONG, LOCKED, TRANSITION_TOO_LONG, then the gate (CANDIDATE_REJECTED). The corpus has one rule per refusal case.",
    "LOCKED_AUTO": "Under `editLocked`, `auto` takes the first policy that keeps the step (#10, #30): its own mix, then a full anchor (the `move_cut` direction of #30, where following changes the step), then a full follow (the head ripple, where anchoring changes the step and following keeps it); if none keeps it, the op is LOCKED.",
    "GATE": "Findings of the final gate are reported as CANDIDATE_REJECTED with the codes of the origin rules the candidate breaks (integrity, joins, fx clips and keys, transitions); the pose check of continuous joins (CONTINUOUS_*_JUMP) and the lighting reader are not modelled (T06).",
    "AUDIO_WARNING": "`explain` compares the edit with the LENGTH of the audio window on the edit (`editEndSecondsExclusive - editStartSeconds`, since edit time t plays the source at `editStartSeconds + t`) and allows one frame of slack; `locked` is the `editLocked` flag of the instance; `NARRATION_PAST_EDIT` is not modelled.",
    "TRANSITION_KEYS": "Transitions in the minimal document have no keys; `shrink_transitions` only changes `durationFrames`.",
}


def interpreter_problem(executable=None, version_info=None, size=None):
    """Why this interpreter must not run the oracle, or None. On Windows `python` can be the Microsoft Store stub
    (an app-execution alias under `...\\WindowsApps\\`, zero bytes, no interpreter behind it)."""
    executable = sys.executable if executable is None else executable
    version_info = tuple(sys.version_info[:2]) if version_info is None else tuple(version_info[:2])
    if not executable:
        return "sys.executable is empty: there is no interpreter to trust"
    if version_info < MIN_PYTHON:
        return "Python %d.%d is older than %d.%d" % (version_info + MIN_PYTHON)
    lowered = str(executable).replace("\\", "/").lower()
    real_store = "/windowsapps/" in lowered and "pythonsoftwarefoundation" in lowered
    if "/windowsapps/" in lowered and not real_store:
        return "%s is the Microsoft Store stub (an app-execution alias), not an interpreter" % executable
    if size is None:
        try:
            size = Path(executable).stat().st_size
        except OSError:
            size = None
    if size == 0 and not real_store:
        return "%s is an empty file, not an interpreter" % executable
    return None


# =========================================================================================================
# The model: a minimal document
# =========================================================================================================
#
#   editDurationFrames, fps {numerator, denominator} (only for `explain`), audio {editStartSeconds,
#   editEndSecondsExclusive} (only for `explain`), narration (never touched),
#   shots[] {id, sceneInstanceId, editRange {startFrame, endFrameExclusive}, instanceStartFrame, cameraClipId,
#            cameraInFrame, transitionInto {type: cut (+reason) | continuous}, intent},
#   sceneInstances[] {id, sceneDefinitionId?, durationFrames, editLocked?: true},
#   cameraClips[] {id, durationFrames},
#   fx {tracks[] {id, clips[] {id, startFrame, durationFrames, scope?: {kind: output} | {kind: shot, shotId},
#                              keys? {param: [{frame, value, easing?}]}, fadeInFrames?, fadeOutFrames?}],
#       transitions[] {id, fromShotId, toShotId, durationFrames, alignment, handles?: hold|use|refuse}},
#   sceneDefinitions[] {id, lighting? {lights[] {id, keys? {intensity|color|position: [{frame, value, easing}]}}},
#                       weatherKeys?: [{frame, value, easing}], ...any deferred field, never touched}


class Refusal(Exception):
    """A refusal of an op: a stable code (D19) and, when the rule has one, the nearest legal frame (#33)."""

    def __init__(self, code, limit_frame=None, detail=None, findings=None):
        Exception.__init__(self, code)
        self.code = code
        self.limit_frame = limit_frame
        self.detail = detail
        self.findings = findings


def _start(shot):
    return shot["editRange"]["startFrame"]


def _end(shot):
    return shot["editRange"]["endFrameExclusive"]


def _length(shot):
    return _end(shot) - _start(shot)


def ordered(doc):
    """The shots by start frame (the document may keep them in another order). The dicts are the document's own."""
    return sorted(doc["shots"], key=_start)


def _find_shot(doc, shot_id):
    for shot in doc["shots"]:
        if shot["id"] == shot_id:
            return shot
    raise Refusal("SHOT_NOT_FOUND", detail={"shot": shot_id})


def _instance_of(doc, shot):
    for instance in doc["sceneInstances"]:
        if instance["id"] == shot["sceneInstanceId"]:
            return instance
    return None


def _clip_of(doc, shot):
    for clip in doc["cameraClips"]:
        if clip["id"] == shot["cameraClipId"]:
            return clip
    return None


def _is_continuous(shot):
    transition = shot.get("transitionInto")
    return bool(transition) and transition.get("type") == "continuous"


def continuation(seq, index):
    """The shots that carry on the one at `index` through continuous joins, in order."""
    out = []
    j = index + 1
    while j < len(seq) and _is_continuous(seq[j]):
        out.append(seq[j])
        j += 1
    return out


def take_end(seq, index):
    """The old end of the last shot of the take that starts at `index`: the end of the chain (#8)."""
    carried = continuation(seq, index)
    last = carried[-1] if carried else seq[index]
    return _end(last)


def _snapshot(doc):
    return dict(
        (shot["id"], [_start(shot), _end(shot), shot["instanceStartFrame"], shot["cameraInFrame"]])
        for shot in doc["shots"]
    )


def _transitions(doc):
    fx = doc.get("fx") or {}
    return fx.get("transitions") or []


# ---------------------------------------------------------------------------------------------------------
# Transitions: the window (fx-windows.ts) and the longest one two shots hold
# ---------------------------------------------------------------------------------------------------------


def transition_window(cut, duration, alignment):
    """`[start, end)` of a transition of `duration` at `cut` (the first frame of the incoming shot), and how many of
    its frames fall before and after the cut (`transitionWindow`, fx-windows.ts)."""
    start = cut
    if alignment == "center":
        start = cut - duration // 2
    if alignment == "end":
        start = cut - duration
    end = start + duration
    before = max(0, min(end, cut) - start)
    return {"start": start, "end": end, "before": before, "after": duration - before}


def max_transition_frames(alignment, from_frames, to_frames):
    """The longest transition of `alignment` two shots hold inside themselves (`maxTransitionFrames`)."""
    if alignment == "start":
        return to_frames
    if alignment == "end":
        return from_frames
    return min(2 * from_frames + 1, 2 * to_frames)


def _min_lengths(doc, shot_ids):
    """The shortest each shot can be before a transition on one of its cuts stops fitting (1 without one)."""
    out = dict((shot_id, 1) for shot_id in shot_ids)
    for transition in _transitions(doc):
        window = transition_window(0, transition["durationFrames"], transition["alignment"])
        if transition["fromShotId"] in out:
            out[transition["fromShotId"]] = max(out[transition["fromShotId"]], window["before"])
        if transition["toShotId"] in out:
            out[transition["toShotId"]] = max(out[transition["toShotId"]], window["after"])
    return out


def _judge_transitions(doc, new, changed_ids, shrink, limit_for):
    """The transitions that the change touched and that no longer fit. Without `shrink` the first one refuses
    (TRANSITION_TOO_LONG, with `limit_for()` as the legal frame); with it each is reduced to the longest the two
    shots hold. Returns `[(transition id, from, to)]` of the reductions."""
    shrinks = []
    for transition in _transitions(doc):
        if not (transition["fromShotId"] in changed_ids or transition["toShotId"] in changed_ids):
            continue
        if transition["fromShotId"] not in new or transition["toShotId"] not in new:
            continue
        len_from = new[transition["fromShotId"]][1] - new[transition["fromShotId"]][0]
        len_to = new[transition["toShotId"]][1] - new[transition["toShotId"]][0]
        window = transition_window(0, transition["durationFrames"], transition["alignment"])
        if len_from >= window["before"] and len_to >= window["after"]:
            continue
        if not shrink:
            raise Refusal("TRANSITION_TOO_LONG", limit_frame=limit_for(), detail={"transition": transition["id"]})
        longest = max_transition_frames(transition["alignment"], len_from, len_to)
        shrinks.append((transition["id"], transition["durationFrames"], longest))
    return shrinks


def transition_reserve(doc, shot):
    """The head frames the transition INTO `shot` draws before the cut (decision #29): none when its `handles` is
    `hold` (or when it has none)."""
    for transition in _transitions(doc):
        if transition["toShotId"] != shot["id"]:
            continue
        if transition.get("handles", "hold") == "hold":
            return 0
        return transition_window(0, transition["durationFrames"], transition["alignment"])["before"]
    return 0


# ---------------------------------------------------------------------------------------------------------
# The content policy (#28..#30) and the lock (#10)
# ---------------------------------------------------------------------------------------------------------


def anchored_frames(d, head, reserve):
    """`auto`: how many of the `d` frames the entering side gains are anchored (the picture stays on the same edit
    frames): the head frames that exist, less the transition reserve (see ORACLE_CHOICES AUTO_RESERVE)."""
    return max(0, min(d, head - reserve))


def _anchor_candidates(content, d, head, reserve, continuous_in, no_head_limit):
    """The values of `a` (anchored frames) to try, in order. `anchor` anchors all `d` or refuses with
    NO_HEAD_FRAMES; `follow` anchors none and is CONTINUOUS_JOIN into a continuous join; `auto` anchors what it can
    and never refuses for lack of head (and never follows into a continuous join)."""
    if content == "anchor":
        if head < d:
            raise Refusal("NO_HEAD_FRAMES", limit_frame=no_head_limit)
        return [d]
    if content == "follow":
        if continuous_in:
            raise Refusal("CONTINUOUS_JOIN", detail={"reason": "follow into a continuous join"})
        return [0]
    if continuous_in:
        if head < d:
            raise Refusal("NO_HEAD_FRAMES", limit_frame=no_head_limit)
        return [d]
    first = anchored_frames(d, head, reserve)
    out = [first]
    if first < d and head >= d:
        out.append(d)
    if 0 not in out:
        out.append(0)
    return out


def _lock_problem(doc, before, after):
    """`editLocked` (#10): shots of one locked instance that were in step (same `instanceStartFrame - startFrame`)
    must stay in step."""
    seq = ordered(doc)
    for instance in doc["sceneInstances"]:
        if not instance.get("editLocked"):
            continue
        ids = [shot["id"] for shot in seq if shot["sceneInstanceId"] == instance["id"]]
        for x in range(len(ids)):
            for y in range(x + 1, len(ids)):
                a, b = ids[x], ids[y]
                if before[a][2] - before[a][0] != before[b][2] - before[b][0]:
                    continue
                if after[a][2] - after[a][0] != after[b][2] - after[b][0]:
                    return {"instance": instance["id"], "shots": [a, b]}
    return None


# ---------------------------------------------------------------------------------------------------------
# The chain of splices (#31) and what the layers do with it
# ---------------------------------------------------------------------------------------------------------
#
# A splice is ("ins", at, n): n frames are inserted at `at` (frames >= at move by n), or ("del", a, b): the
# stretch [a, b) is removed (frames inside are dropped, frames >= b move back by b - a). The chain applies them in
# order, each in the coordinates the previous one left.


def map_point(frame, chain):
    """Where a point of the edit (a key) ends up, or None when it falls in a removed stretch."""
    for splice in chain:
        if splice[0] == "ins":
            if frame >= splice[1]:
                frame += splice[2]
        else:
            a, b = splice[1], splice[2]
            if a <= frame < b:
                return None
            if frame >= b:
                frame -= b - a
    return frame


def map_clip(start, end, chain):
    """A clip `[start, end)` through the chain: `(start, end, head)` where `head` is how many of its own head frames
    were cut (its keys shift by it), or None when it ends up inside a removed stretch."""
    head = 0
    for splice in chain:
        if splice[0] == "ins":
            at, n = splice[1], splice[2]
            if start >= at:
                start += n
                end += n
            elif end > at:
                end += n
            continue
        a, b = splice[1], splice[2]
        n = b - a
        if end <= a:
            continue
        if start >= b:
            start -= n
            end -= n
            continue
        if start >= a and end <= b:
            return None
        if start < a and end <= b:
            end = a
            continue
        if start < a and end > b:
            end -= n
            continue
        head += b - start  # a <= start < b < end: the head is cut up to b
        start = a
        end -= n
    return start, end, head


def reframe_shot_clip(start, end, delta, shot_start, shot_end):
    """A clip of a shot: it moves with the picture by `delta` (the change of `startFrame - instanceStartFrame` of
    its shot), then is cut to the shot. `(start, end, head)` or None when nothing is left."""
    moved_start, moved_end = start + delta, end + delta
    new_start, new_end = max(moved_start, shot_start), min(moved_end, shot_end)
    if new_end <= new_start:
        return None
    return new_start, new_end, new_start - moved_start


def prune_keys(keys, shift, duration):
    """The keys of a clip after its head moved by `shift` (a key keeps its instant, so its relative frame drops by
    it) and its length became `duration`: those outside `[0, duration)` are dropped. `(keys or None, dropped)`."""
    if not keys:
        return keys, {}
    kept = {}
    dropped = {}
    for param in keys:
        inside = []
        for key in keys[param]:
            frame = key["frame"] - shift
            if 0 <= frame < duration:
                moved = dict(key)
                moved["frame"] = frame
                inside.append(moved)
            else:
                dropped.setdefault(param, []).append(key["frame"])
        if inside:
            kept[param] = inside
    return (kept or None), dropped


def fit_envelope(clip):
    """Fades cut down until `fadeIn + fadeOut <= durationFrames`, the fade in first (`fitEnvelope`)."""
    fade_in = clip.get("fadeInFrames", 0)
    fade_out = clip.get("fadeOutFrames", 0)
    excess = fade_in + fade_out - clip["durationFrames"]
    if excess <= 0:
        return []
    next_in = max(0, fade_in - excess)
    next_out = fade_out - (excess - (fade_in - next_in))
    notes = []
    for field, old, new in (("fadeInFrames", fade_in, next_in), ("fadeOutFrames", fade_out, next_out)):
        if new > 0:
            clip[field] = new
        else:
            clip.pop(field, None)
        if new != old:
            notes.append({"code": "FADE_REDUCED", "clip": clip["id"], "field": field, "from": old, "to": new})
    return notes


def _commit_clip(clip, start, end, head, notes):
    old_start, old_duration = clip["startFrame"], clip["durationFrames"]
    duration = end - start
    if start == old_start and duration == old_duration and head == 0:
        return
    clip["startFrame"] = start
    clip["durationFrames"] = duration
    keys = clip.get("keys")
    if keys:
        kept, dropped = prune_keys(keys, head, duration)
        if kept:
            clip["keys"] = kept
        else:
            clip.pop("keys", None)
        for param in sorted(dropped):
            notes.append({"code": "KEYS_PRUNED", "clip": clip["id"], "param": param, "frames": dropped[param]})
    if duration != old_duration:
        notes.extend(fit_envelope(clip))


def remap_key_list(keys, chain):
    out, dropped = [], []
    for key in keys:
        frame = map_point(key["frame"], chain)
        if frame is None:
            dropped.append(key["frame"])
            continue
        moved = dict(key)
        moved["frame"] = frame
        out.append(moved)
    return out, dropped


LIGHT_CHANNELS = ("intensity", "color", "position")


def _apply_layers(doc, old, new, chain, keep_absolute, notes):
    fx = doc.get("fx")
    if fx:
        for track in fx.get("tracks", []):
            survivors = []
            for clip in track["clips"]:
                scope = clip.get("scope") or {"kind": "output"}
                start, end = clip["startFrame"], clip["startFrame"] + clip["durationFrames"]
                if scope["kind"] == "shot":
                    shot_id = scope.get("shotId")
                    if shot_id not in new or shot_id not in old:
                        # A clip of a shot the edit does not have: an invalid document the gate refuses; the
                        # remap leaves the clip as it is instead of failing.
                        survivors.append(clip)
                        continue
                    delta = (new[shot_id][0] - new[shot_id][2]) - (old[shot_id][0] - old[shot_id][2])
                    result = reframe_shot_clip(start, end, delta, new[shot_id][0], new[shot_id][1])
                elif keep_absolute:
                    result = (start, end, 0)
                else:
                    result = map_clip(start, end, chain)
                if result is None:
                    notes.append({"code": "CLIP_REMOVED", "clip": clip["id"], "scope": scope["kind"]})
                    continue
                _commit_clip(clip, result[0], result[1], result[2], notes)
                survivors.append(clip)
            track["clips"] = survivors
    if keep_absolute:
        return
    for definition in doc.get("sceneDefinitions", []):
        lighting = definition.get("lighting") or {}
        for light in lighting.get("lights", []):
            keys = light.get("keys")
            if not keys:
                continue
            for channel in LIGHT_CHANNELS:
                if channel not in keys:
                    continue
                kept, dropped = remap_key_list(keys[channel], chain)
                if dropped:
                    notes.append({"code": "LIGHT_KEYS_DROPPED", "definition": definition["id"], "light": light["id"], "channel": channel, "frames": dropped})
                if kept:
                    keys[channel] = kept
                else:
                    del keys[channel]
                    notes.append({"code": "LIGHT_TRACK_REMOVED", "definition": definition["id"], "light": light["id"], "channel": channel})
            if not keys:
                del light["keys"]
        if "weatherKeys" in definition:  # the audit's only `remap` field
            kept, dropped = remap_key_list(definition["weatherKeys"], chain)
            if dropped:
                notes.append({"code": "WEATHER_KEYS_DROPPED", "definition": definition["id"], "frames": dropped})
            if kept:
                definition["weatherKeys"] = kept
            else:
                del definition["weatherKeys"]
                notes.append({"code": "WEATHER_KEYS_REMOVED", "definition": definition["id"]})


# ---------------------------------------------------------------------------------------------------------
# Applying a plan to the document
# ---------------------------------------------------------------------------------------------------------


def shift_later_shots(new, seq, index, delta):
    """Ripple: every shot after `seq[index]` moves by `delta` on the edit (its picture mapping stays)."""
    for later in seq[index + 1:]:
        new[later["id"]][0] += delta
        new[later["id"]][1] += delta


def slide_continuation(new, seq, index, frames):
    """The shots that carry on `seq[index]` start `frames` later in the scene and in the camera clip (#8, #30)."""
    for carried in continuation(seq, index):
        new[carried["id"]][2] += frames
        new[carried["id"]][3] += frames


def _finish(doc, old, new, chain, edit_duration, opts, changed_ids, limit_for, extra_notes):
    """Lock, transitions, then the write: shots, instances, edit length, transitions and layers. Notes in order."""
    problem = _lock_problem(doc, old, new)
    if problem:
        raise Refusal("LOCKED", detail=problem)
    shrinks = _judge_transitions(doc, new, changed_ids, opts["shrink_transitions"], limit_for)
    notes = list(extra_notes)
    for shot in doc["shots"]:
        state = new[shot["id"]]
        shot["editRange"] = {"startFrame": state[0], "endFrameExclusive": state[1]}
        shot["instanceStartFrame"] = state[2]
        shot["cameraInFrame"] = state[3]
    doc["editDurationFrames"] = edit_duration
    needs = {}
    for shot in doc["shots"]:
        state = new[shot["id"]]
        needs[shot["sceneInstanceId"]] = max(needs.get(shot["sceneInstanceId"], 0), state[2] + state[1] - state[0])
    for instance in doc["sceneInstances"]:
        need = needs.get(instance["id"], 0)
        if need > instance["durationFrames"]:
            notes.append({"code": "INSTANCE_GREW", "instance": instance["id"], "from": instance["durationFrames"], "to": need})
            instance["durationFrames"] = need
    for transition_id, before, after in shrinks:
        for transition in _transitions(doc):
            if transition["id"] == transition_id:
                transition["durationFrames"] = after
        notes.append({"code": "TRANSITION_SHRUNK", "transition": transition_id, "from": before, "to": after})
    _apply_layers(doc, old, new, chain, opts["keep_absolute"], notes)
    return notes


def _follow_note(shot, anchored, followed, content):
    """#28: `auto` that falls back to follow says so. An explicit `follow` asked for it: no note."""
    if followed <= 0 or content != "auto":
        return []
    return [{"code": "AUTO_FELL_TO_FOLLOW", "shot": shot["id"], "anchored": anchored, "followed": followed}]


# ---------------------------------------------------------------------------------------------------------
# The three planners
# ---------------------------------------------------------------------------------------------------------


def edge_end_ripple(doc, shot, frame, opts):
    """The end of `shot` goes to `frame` (exclusive); everything after moves with it (#3)."""
    seq = ordered(doc)
    index = seq.index(shot)
    start, end = _start(shot), _end(shot)
    if frame == end:
        return []
    if frame - start < 1:
        raise Refusal("SHOT_TOO_SHORT", limit_frame=start + 1, detail={"shot": shot["id"]})
    delta = frame - end
    edit = doc["editDurationFrames"] + delta
    if edit > MAX_EDIT_FRAMES:
        raise Refusal("EDIT_TOO_LONG", limit_frame=end + (MAX_EDIT_FRAMES - doc["editDurationFrames"]), detail={"max": MAX_EDIT_FRAMES})
    old = _snapshot(doc)
    new = copy.deepcopy(old)
    new[shot["id"]][1] = frame
    shift_later_shots(new, seq, index, delta)
    slide_continuation(new, seq, index, delta)
    chain_end = take_end(seq, index)
    if delta > 0:
        chain = [("ins", chain_end, delta)]
    else:
        chain = [("del", chain_end + delta, chain_end)]

    def limit_for():
        return start + _min_lengths(doc, [shot["id"]])[shot["id"]]

    return _finish(doc, old, new, chain, edit, opts, set([shot["id"]]), limit_for, [])


def edge_start_ripple(doc, shot, frame, opts):
    """The start of `shot` goes to `frame`; the shot stays where it is, the picture of its head leaves (or enters) and
    everything after moves (#11, #32)."""
    seq = ordered(doc)
    index = seq.index(shot)
    start, end = _start(shot), _end(shot)
    d = start - frame
    if d == 0:
        return []
    if d < 0 and end - frame < 1:
        raise Refusal("SHOT_TOO_SHORT", limit_frame=end - 1, detail={"shot": shot["id"]})
    if _is_continuous(shot):
        raise Refusal("CONTINUOUS_JOIN", detail={"shot": shot["id"], "reason": "head of a shot that carries on the previous one"})
    old = _snapshot(doc)
    inst_start, cam_in = shot["instanceStartFrame"], shot["cameraInFrame"]
    head = min(inst_start, cam_in)
    if d < 0:
        h = -d
        edit = doc["editDurationFrames"] - h
        new = copy.deepcopy(old)
        new[shot["id"]][1] = end - h
        new[shot["id"]][2] += h
        new[shot["id"]][3] += h
        shift_later_shots(new, seq, index, -h)
        chain = [("del", start, start + h)]

        def limit_cut():
            return end - _min_lengths(doc, [shot["id"]])[shot["id"]]

        return _finish(doc, old, new, chain, edit, opts, set([shot["id"]]), limit_cut, [])
    edit = doc["editDurationFrames"] + d
    candidates = _anchor_candidates(opts["content"], d, head, transition_reserve(doc, shot), False, start - head)
    if edit > MAX_EDIT_FRAMES:
        raise Refusal("EDIT_TOO_LONG", limit_frame=start - (MAX_EDIT_FRAMES - doc["editDurationFrames"]), detail={"max": MAX_EDIT_FRAMES})
    chain_end = take_end(seq, index)

    def limit_early():
        return end - _min_lengths(doc, [shot["id"]])[shot["id"]]

    last = None
    for a in candidates:
        followed = d - a
        new = copy.deepcopy(old)
        new[shot["id"]][1] = end + d
        new[shot["id"]][2] -= a
        new[shot["id"]][3] -= a
        shift_later_shots(new, seq, index, d)
        if followed > 0:
            slide_continuation(new, seq, index, followed)
        chain = []
        if a > 0:
            chain.append(("ins", start, a))
        if followed > 0:
            chain.append(("ins", chain_end + a, followed))
        try:
            return _finish(doc, old, new, chain, edit, opts, set([shot["id"]]), limit_early, _follow_note(shot, a, followed, opts["content"]))
        except Refusal as refusal:
            if refusal.code != "LOCKED" or opts["content"] != "auto":
                raise
            last = refusal
    raise last


def edge_roll(doc, previous, following, frame, opts):
    """The cut between `previous` and `following` goes to `frame`; the edit keeps its length (#3, #28..#31)."""
    seq = ordered(doc)
    q_index = seq.index(following)
    cut = _end(previous)
    if frame == cut:
        return []
    p_start, q_end = _start(previous), _end(following)
    if frame <= p_start:
        raise Refusal("SHOT_TOO_SHORT", limit_frame=p_start + 1, detail={"shot": previous["id"]})
    if frame >= q_end:
        raise Refusal("SHOT_TOO_SHORT", limit_frame=q_end - 1, detail={"shot": following["id"]})
    old = _snapshot(doc)
    changed = set([previous["id"], following["id"]])
    lows = _min_lengths(doc, [previous["id"], following["id"]])

    def limit_for():
        low, high = p_start + lows[previous["id"]], q_end - lows[following["id"]]
        if low > high:
            return None
        return min(max(frame, low), high)

    if frame > cut:
        later = frame - cut
        new = copy.deepcopy(old)
        new[previous["id"]][1] = frame
        new[following["id"]][0] = frame
        new[following["id"]][2] += later
        new[following["id"]][3] += later
        return _finish(doc, old, new, [], doc["editDurationFrames"], opts, changed, limit_for, [])
    d = cut - frame
    head = min(following["instanceStartFrame"], following["cameraInFrame"])
    candidates = _anchor_candidates(opts["content"], d, head, transition_reserve(doc, following), _is_continuous(following), cut - head)
    chain_end = take_end(seq, q_index)
    last = None
    for a in candidates:
        followed = d - a
        new = copy.deepcopy(old)
        new[previous["id"]][1] = frame
        new[following["id"]][0] = frame
        new[following["id"]][2] -= a
        new[following["id"]][3] -= a
        chain = []
        if followed > 0:
            slide_continuation(new, seq, q_index, followed)
            chain = [("del", frame, cut)]
            if a > 0:
                chain.append(("ins", frame, a))
            chain.append(("ins", chain_end - followed, followed))
        try:
            return _finish(doc, old, new, chain, doc["editDurationFrames"], opts, changed, limit_for, _follow_note(following, a, followed, opts["content"]))
        except Refusal as refusal:
            if refusal.code != "LOCKED" or opts["content"] != "auto":
                raise
            last = refusal
    raise last


# ---------------------------------------------------------------------------------------------------------
# The ops
# ---------------------------------------------------------------------------------------------------------

OP_FIELDS = {
    "trim_shot": (("shot", "edge", "frame"), ("mode", "content", "keep_absolute", "shrink_transitions", "with_narration")),
    "move_cut": (("from", "to", "frame"), ("content", "keep_absolute", "shrink_transitions", "with_narration")),
    "set_shot_duration": (("shot", "frames"), ("mode", "content", "keep_absolute", "shrink_transitions", "with_narration")),
    "set_edit_duration": (("frames",), ("content", "keep_absolute", "shrink_transitions", "with_narration")),
}
OPTION_DEFAULTS = {"mode": "ripple", "content": "auto", "keep_absolute": False, "shrink_transitions": False, "with_narration": False}


def normalize_op(wrapped):
    """`{"trim_shot": {...}}` to `(name, fields with the defaults)`. A malformed op is a ValueError (INVALID_OP):
    it is not a refusal of the domain."""
    if not isinstance(wrapped, dict) or len(wrapped) != 1:
        raise ValueError("INVALID_OP: an op is an object with exactly one key")
    name = next(iter(wrapped))
    if name not in OP_FIELDS:
        raise ValueError("INVALID_OP: unknown op %r (expected one of %s)" % (name, ", ".join(sorted(OP_FIELDS))))
    body = wrapped[name]
    required, optional = OP_FIELDS[name]
    if not isinstance(body, dict):
        raise ValueError("INVALID_OP: %s takes an object" % name)
    for key in body:
        if key not in required and key not in optional:
            raise ValueError("INVALID_OP: %s has no field %r" % (name, key))
    for key in required:
        if key not in body:
            raise ValueError("INVALID_OP: %s needs %r" % (name, key))
    fields = dict(OPTION_DEFAULTS)
    fields.update(body)
    for key in ("frame", "frames"):
        if key in fields and (isinstance(fields[key], bool) or not isinstance(fields[key], int)):
            raise ValueError("INVALID_OP: %s must be an integer" % key)
    if fields["mode"] not in ("ripple", "roll"):
        raise ValueError("INVALID_OP: mode is ripple or roll")
    if fields["content"] not in ("anchor", "follow", "auto"):
        raise ValueError("INVALID_OP: content is anchor, follow or auto")
    if name == "trim_shot" and fields["edge"] not in ("start", "end"):
        raise ValueError("INVALID_OP: edge is start or end")
    for key in ("keep_absolute", "shrink_transitions", "with_narration"):
        if not isinstance(fields[key], bool):
            raise ValueError("INVALID_OP: %s is a boolean" % key)
    return name, fields


def _refused_layers():
    """The audited fields this slice refuses (destination `refuse`): the audit leaves none today."""
    return [entry for entry in DEFERRED_AUDIT if entry["destination"] == "refuse"]


def _has_field(doc, entry):
    """Whether any scene definition (or the bundle) carries the audited field, for the `refuse` destination."""
    name = entry["field"].split(" ")[0]
    if name.startswith("sceneDefinitions[]."):
        key = name[len("sceneDefinitions[]."):]
        return any(key in definition for definition in doc.get("sceneDefinitions", []))
    return name in doc


def apply_op(doc, wrapped):
    """Runs one op on `doc` (mutated in place). Returns the notes, or raises Refusal."""
    name, fields = normalize_op(wrapped)
    if fields["with_narration"]:
        raise Refusal("SHOTS_OPTION_NOT_YET", detail={"option": "with_narration"})
    seq = ordered(doc)
    shot = None
    if name == "set_edit_duration":
        shot = seq[-1]
    elif name == "move_cut":
        previous = _find_shot(doc, fields["from"])
        following = _find_shot(doc, fields["to"])
        position = seq.index(previous)
        if position + 1 >= len(seq) or seq[position + 1] is not following:
            raise Refusal("SHOTS_NOT_ADJACENT", detail={"from": previous["id"], "to": following["id"]})
    else:
        shot = _find_shot(doc, fields["shot"])
    for entry in _refused_layers():
        if _has_field(doc, entry):
            raise Refusal("SHOTS_LAYER_NOT_YET", detail={"field": entry["field"]})
    if name == "set_edit_duration":
        return edge_end_ripple(doc, shot, fields["frames"], fields)
    if name == "move_cut":
        return edge_roll(doc, previous, following, fields["frame"], fields)
    index = seq.index(shot)
    if name == "set_shot_duration":
        frame, edge = _start(shot) + fields["frames"], "end"
    else:
        frame, edge = fields["frame"], fields["edge"]
    if edge == "end":
        if fields["mode"] == "roll":
            if index + 1 >= len(seq):
                raise Refusal("SHOTS_NOT_ADJACENT", detail={"shot": shot["id"], "reason": "no shot after it to roll with"})
            return edge_roll(doc, shot, seq[index + 1], frame, fields)
        return edge_end_ripple(doc, shot, frame, fields)
    if fields["mode"] == "roll":
        if index == 0:
            raise Refusal("SHOTS_NOT_ADJACENT", detail={"shot": shot["id"], "reason": "no shot before it to roll with"})
        return edge_roll(doc, seq[index - 1], shot, frame, fields)
    return edge_start_ripple(doc, shot, frame, fields)


def apply_ops(document, ops):
    """The batch (D02): the ops run in order on a candidate; one gate at the end. The input is never changed.
    `{"ok": true, "document", "notes", "changed"}` or `{"ok": false, "document" (the input), "refusal": {...}}`."""
    work = copy.deepcopy(document)
    notes = []
    for index, wrapped in enumerate(ops):
        try:
            notes.extend(apply_op(work, wrapped))
        except Refusal as refusal:
            return _refusal_result(document, refusal, index)
    findings = validate_document(work)
    if findings:
        return _refusal_result(document, Refusal("CANDIDATE_REJECTED", findings=findings), None)
    return {"ok": True, "document": work, "notes": notes, "changed": work != document}


def _refusal_result(document, refusal, index):
    body = {"code": refusal.code}
    if index is not None:
        body["index"] = index
    if refusal.limit_frame is not None:
        body["limitFrame"] = refusal.limit_frame
    if refusal.detail:
        body["detail"] = refusal.detail
    if refusal.findings is not None:
        body["findings"] = refusal.findings
    return {"ok": False, "document": copy.deepcopy(document), "refusal": body}


# ---------------------------------------------------------------------------------------------------------
# The gate: what makes a document valid (the origin rules this oracle models)
# ---------------------------------------------------------------------------------------------------------


def _keyframe_order_issues(keys, label):
    previous = -1
    out = []
    for key in keys:
        frame = key["frame"]
        if isinstance(frame, bool) or not isinstance(frame, int) or frame < 0 or frame <= previous:
            out.append({"code": "KEYFRAME_ORDER", "ref": label})
        previous = frame
    return out


def validate_document(doc):
    """The codes of the origin rules the document breaks (bundle integrity, edit joins, the fx layer, key order),
    as `[{code, ref}]`. Empty means valid. The pose comparison of the continuous joins is not modelled."""
    out = []
    seq = ordered(doc)
    edit = doc["editDurationFrames"]
    instances = dict((i["id"], i) for i in doc["sceneInstances"])
    clips = dict((c["id"], c) for c in doc["cameraClips"])
    if not seq or _start(seq[0]) != 0:
        out.append({"code": "EDIT_RANGE_START", "ref": ""})
    expected = 0
    for shot in seq:
        instance = instances.get(shot["sceneInstanceId"])
        if instance is None:
            out.append({"code": "SHOT_UNKNOWN_INSTANCE", "ref": shot["id"]})
        elif shot["instanceStartFrame"] + _length(shot) > instance["durationFrames"]:
            out.append({"code": "SHOT_EXCEEDS_INSTANCE_DURATION", "ref": shot["id"]})
        if shot["cameraClipId"] not in clips:
            out.append({"code": "SHOT_UNKNOWN_CAMERA_CLIP", "ref": shot["id"]})
        if _start(shot) != expected:
            out.append({"code": "EDIT_GAP_OR_OVERLAP", "ref": shot["id"]})
        if _end(shot) <= _start(shot):
            out.append({"code": "EDIT_RANGE_INVALID", "ref": shot["id"]})
        expected = _end(shot)
    if expected != edit:
        out.append({"code": "EDIT_DURATION_MISMATCH", "ref": ""})
    for previous, following in zip(seq, seq[1:]):
        transition = following.get("transitionInto")
        if not transition:
            out.append({"code": "MISSING_TRANSITION", "ref": following["id"]})
            continue
        if transition["type"] == "cut":
            if not str(transition.get("reason", "")).strip():
                out.append({"code": "CUT_WITHOUT_REASON", "ref": following["id"]})
            continue
        if previous["sceneInstanceId"] != following["sceneInstanceId"]:
            out.append({"code": "CONTINUOUS_INSTANCE_MISMATCH", "ref": following["id"]})
        if following["instanceStartFrame"] != previous["instanceStartFrame"] + _length(previous):
            out.append({"code": "INSTANCE_FRAME_DISCONTINUITY", "ref": following["id"]})
        if previous["cameraClipId"] != following["cameraClipId"]:
            out.append({"code": "CONTINUOUS_CROSS_CLIP_UNSUPPORTED", "ref": following["id"]})
            continue
        if following["cameraInFrame"] != previous["cameraInFrame"] + _length(previous):
            out.append({"code": "CAMERA_CLIP_OFFSET_BREAK", "ref": following["id"]})
    shots_by_id = dict((s["id"], s) for s in doc["shots"])
    fx = doc.get("fx") or {}
    for track in fx.get("tracks", []):
        longest = None
        for clip in sorted(track["clips"], key=lambda c: c["startFrame"]):
            end = clip["startFrame"] + clip["durationFrames"]
            scope = clip.get("scope") or {"kind": "output"}
            if scope["kind"] == "shot":
                shot = shots_by_id.get(scope["shotId"])
                if shot is None:
                    out.append({"code": "UNKNOWN_SHOT", "ref": clip["id"]})
                elif clip["startFrame"] < _start(shot) or end > _end(shot):
                    out.append({"code": "FX_SCOPE_MISMATCH", "ref": clip["id"]})
            if clip["durationFrames"] < 1:
                out.append({"code": "FX_CLIP_EMPTY", "ref": clip["id"]})
            if clip["startFrame"] < 0 or end > edit:
                out.append({"code": "FX_CLIP_OUT_OF_RANGE", "ref": clip["id"]})
            if clip.get("fadeInFrames", 0) + clip.get("fadeOutFrames", 0) > clip["durationFrames"]:
                out.append({"code": "ENVELOPE_TOO_LONG", "ref": clip["id"]})
            if longest is not None and clip["startFrame"] < longest["startFrame"] + longest["durationFrames"]:
                out.append({"code": "FX_CLIP_OVERLAP", "ref": clip["id"]})
            if longest is None or end > longest["startFrame"] + longest["durationFrames"]:
                longest = clip
            for param in sorted(clip.get("keys") or {}):
                keys = clip["keys"][param]
                out.extend(_keyframe_order_issues(keys, clip["id"] + "/" + param))
                for key in keys:
                    if not (0 <= key["frame"] < clip["durationFrames"]):
                        out.append({"code": "KEY_OUT_OF_CLIP", "ref": clip["id"] + "/" + param})
    windows = []
    for transition in fx.get("transitions", []):
        source, target = shots_by_id.get(transition["fromShotId"]), shots_by_id.get(transition["toShotId"])
        if source is None or target is None:
            out.append({"code": "UNKNOWN_SHOT", "ref": transition["id"]})
            continue
        position = seq.index(source)
        if position + 1 >= len(seq) or seq[position + 1] is not target:
            out.append({"code": "TRANSITION_NOT_AT_CUT", "ref": transition["id"]})
            continue
        if transition["durationFrames"] < 1:
            out.append({"code": "TRANSITION_OUT_OF_RANGE", "ref": transition["id"]})
            continue
        window = transition_window(_start(target), transition["durationFrames"], transition["alignment"])
        windows.append((transition["id"], window))
        if window["start"] < 0 or window["end"] > edit:
            out.append({"code": "TRANSITION_OUT_OF_RANGE", "ref": transition["id"]})
        if _length(source) < window["before"] or _length(target) < window["after"]:
            out.append({"code": "TRANSITION_TOO_LONG", "ref": transition["id"]})
        if transition.get("handles", "hold") != "hold":
            lacks = _handle_lacks(doc, source, target, window)
            if lacks:
                out.append({"code": "TRANSITION_NO_HANDLES", "ref": transition["id"]})
    windows.sort(key=lambda item: item[1]["start"])
    for (_, first), (second_id, second) in zip(windows, windows[1:]):
        if second["start"] < first["end"]:
            out.append({"code": "TRANSITION_OVERLAP", "ref": second_id})
    for definition in doc.get("sceneDefinitions", []):
        for light in (definition.get("lighting") or {}).get("lights", []):
            for channel in LIGHT_CHANNELS:
                out.extend(_keyframe_order_issues((light.get("keys") or {}).get(channel, []), "%s/%s/%s" % (definition["id"], light["id"], channel)))
        out.extend(_keyframe_order_issues(definition.get("weatherKeys", []), definition["id"] + "/weatherKeys"))
    return out


def shot_handles(doc, shot):
    """The real frames of scene and camera clip before and after what the shot shows (`fxShotHandles`,
    `fxCameraHandles`)."""
    instance = _instance_of(doc, shot)
    clip = _clip_of(doc, shot)
    length = _length(shot)
    scene_after = max(0, instance["durationFrames"] - (shot["instanceStartFrame"] + length)) if instance else 0
    camera_after = max(0, clip["durationFrames"] - (shot["cameraInFrame"] + length)) if clip else 0
    return {
        "scene_before": shot["instanceStartFrame"],
        "scene_after": scene_after,
        "camera_before": shot["cameraInFrame"],
        "camera_after": camera_after,
    }


def _handle_lacks(doc, source, target, window):
    after_source = shot_handles(doc, source)
    before_target = shot_handles(doc, target)
    lacks = []
    if window["after"] > min(after_source["scene_after"], after_source["camera_after"]):
        lacks.append(source["id"])
    if window["before"] > min(before_target["scene_before"], before_target["camera_before"]):
        lacks.append(target["id"])
    return lacks


# ---------------------------------------------------------------------------------------------------------
# The query (D16): `scene.shots.explain`
# ---------------------------------------------------------------------------------------------------------


def _number(value):
    value = Fraction(value)
    if value.denominator == 1:
        return int(value)
    return float(value)


def explain_shots(doc):
    """Where each shot sits, what it has before and after, and the audio warnings (one frame of slack)."""
    seq = ordered(doc)
    locked = dict((i["id"], bool(i.get("editLocked"))) for i in doc["sceneInstances"])
    shots = []
    for shot in seq:
        transition = shot.get("transitionInto")
        shots.append({
            "id": shot["id"],
            "start": _start(shot),
            "end": _end(shot),
            "frames": _length(shot),
            "instance": shot["sceneInstanceId"],
            "instance_start": shot["instanceStartFrame"],
            "camera_clip": shot["cameraClipId"],
            "camera_in": shot["cameraInFrame"],
            "transition_into": transition["type"] if transition else None,
            "intent": shot.get("intent", ""),
            "handles": shot_handles(doc, shot),
            "locked": locked.get(shot["sceneInstanceId"], False),
        })
    out = {"edit": doc["editDurationFrames"], "shots": shots, "audio": None, "warnings": []}
    audio = doc.get("audio")
    fps = doc.get("fps")
    if audio and fps:
        per_second = Fraction(fps["numerator"], fps["denominator"])
        start_seconds = Fraction(str(audio["editStartSeconds"]))
        end_seconds = Fraction(str(audio["editEndSecondsExclusive"]))
        # Edit time t plays the source at editStartSeconds + t: the window lasts end - start on the edit.
        audio_frames = (end_seconds - start_seconds) * per_second
        out["audio"] = {
            "edit_start_seconds": _number(start_seconds),
            "edit_end_seconds_exclusive": _number(end_seconds),
            "frames": _number(audio_frames),
        }
        edit = doc["editDurationFrames"]
        if edit > audio_frames + 1:
            out["warnings"].append({"code": "EDIT_LONGER_THAN_AUDIO", "edit_frames": edit, "audio_frames": _number(audio_frames)})
        if edit + 1 < audio_frames:
            out["warnings"].append({"code": "EDIT_SHORTER_THAN_AUDIO", "edit_frames": edit, "audio_frames": _number(audio_frames)})
    return out


# =========================================================================================================
# The corpus: small synthetic documents written from scratch, and the results the reference gives
# =========================================================================================================

CUT = {"type": "cut", "reason": "scene change"}
CONT = {"type": "continuous"}


def mk_doc(shots, instances, cam_frames=300, tracks=None, transitions=None, defs=None, extra=None, definition_of=None):
    """`shots`: `(id, start, end, instance, instance_start, camera_in, join)` (join None for the first shot);
    `instances`: `(id, duration, locked)`."""
    out_shots = []
    for index, item in enumerate(shots):
        shot_id, start, end, instance, inst_start, cam_in = item[:6]
        join = item[6] if len(item) > 6 else (None if index == 0 else CUT)
        entry = {
            "id": shot_id,
            "sceneInstanceId": instance,
            "editRange": {"startFrame": start, "endFrameExclusive": end},
            "instanceStartFrame": inst_start,
            "cameraClipId": "cam",
            "cameraInFrame": cam_in,
            "intent": "shot " + shot_id,
        }
        if join is not None:
            entry["transitionInto"] = dict(join)
        out_shots.append(entry)
    out_instances = []
    for instance_id, duration, locked in instances:
        entry = {"id": instance_id, "sceneDefinitionId": "def-" + instance_id, "durationFrames": duration}
        if locked:
            entry["editLocked"] = True
        out_instances.append(entry)
    doc = {
        "editDurationFrames": max(item[2] for item in shots),
        "fps": {"numerator": 24, "denominator": 1},
        "shots": out_shots,
        "sceneInstances": out_instances,
        "cameraClips": [{"id": "cam", "durationFrames": cam_frames}],
        "sceneDefinitions": defs if defs is not None else [],
        "fx": {"tracks": tracks or [], "transitions": transitions or []},
    }
    if extra:
        doc.update(extra)
    return doc


def clip(clip_id, start, duration, shot=None, keys=None, fade_in=None, fade_out=None):
    entry = {"id": clip_id, "startFrame": start, "durationFrames": duration}
    if shot:
        entry["scope"] = {"kind": "shot", "shotId": shot}
    if keys:
        entry["keys"] = dict((param, [{"frame": f, "value": v, "easing": "linear"} for f, v in pairs]) for param, pairs in keys.items())
    if fade_in is not None:
        entry["fadeInFrames"] = fade_in
    if fade_out is not None:
        entry["fadeOutFrames"] = fade_out
    return entry


def transition(transition_id, source, target, duration, alignment, handles="hold"):
    return {"id": transition_id, "fromShotId": source, "toShotId": target, "durationFrames": duration, "alignment": alignment, "handles": handles}


def key_list(pairs):
    return [{"frame": f, "value": v, "easing": "linear"} for f, v in pairs]


def base3(**kw):
    """a [0,30) b [30,60) c [60,90), three instances of 60 frames, every head and camera-in at 0."""
    return mk_doc(
        [("a", 0, 30, "ia", 0, 0), ("b", 30, 60, "ib", 0, 0), ("c", 60, 90, "ic", 0, 0)],
        [("ia", 60, False), ("ib", 60, False), ("ic", 60, False)],
        **kw
    )


def headed(b_inst_start=20, b_cam_in=20, **kw):
    """Like base3, but b starts `b_inst_start` frames into its scene and `b_cam_in` into its camera clip."""
    return mk_doc(
        [("a", 0, 30, "ia", 0, 0), ("b", 30, 60, "ib", b_inst_start, b_cam_in), ("c", 60, 90, "ic", 0, 0)],
        [("ia", 60, False), ("ib", 120, False), ("ic", 60, False)],
        **kw
    )


def chain3(**kw):
    """One take of three shots on one instance: a [0,30), b [30,60), c [60,90), joined by continuous joins."""
    return mk_doc(
        [("a", 0, 30, "ia", 0, 0), ("b", 30, 60, "ia", 30, 30, CONT), ("c", 60, 90, "ia", 60, 60, CONT)],
        [("ia", 90, False)],
        **kw
    )


def locked_pair(c_inst_start=60, c_cam_in=60, **kw):
    """a and c share the locked instance `il` and are in step (instanceStartFrame - startFrame = 0); b is another."""
    return mk_doc(
        [("a", 0, 30, "il", 0, 0), ("b", 30, 60, "ib", 0, 0), ("c", 60, 90, "il", c_inst_start, c_cam_in)],
        [("il", 120, True), ("ib", 60, False)],
        **kw
    )


def op_set(shot, frames, **opts):
    body = {"shot": shot, "frames": frames}
    body.update(opts)
    return {"set_shot_duration": body}


def op_trim(shot, edge, frame, **opts):
    body = {"shot": shot, "edge": edge, "frame": frame}
    body.update(opts)
    return {"trim_shot": body}


def op_cut(source, target, frame, **opts):
    body = {"from": source, "to": target, "frame": frame}
    body.update(opts)
    return {"move_cut": body}


def op_edit(frames, **opts):
    body = {"frames": frames}
    body.update(opts)
    return {"set_edit_duration": body}


def build_cases():
    """Every case: id, rows (the T lines it covers), title, tags, document, ops, `want` ("ok" or the refusal code the
    author meant). The generator refuses to write a case whose result is not what the author meant."""
    cases = []

    def add(case_id, rows, title, document, ops, want="ok", tags=()):
        cases.append({"id": case_id, "kind": "ops", "rows": list(rows), "title": title, "tags": list(tags),
                      "document": document, "ops": ops, "want": want})

    def add_explain(case_id, rows, title, document):
        cases.append({"id": case_id, "kind": "explain", "rows": list(rows), "title": title, "tags": [],
                      "document": document, "ops": [], "want": "ok"})

    # ---- T01: set_shot_duration / trim_shot end, ripple -------------------------------------------------
    add("t01-grow-middle", ["T01"], "Growing b by 10: c moves, the edit grows, the instance already holds the frames", base3(), [op_set("b", 40)])
    add("t01-grow-grows-instance", ["T01", "T08"], "Growing b past its scene: the instance grows to fit, with a note", mk_doc(
        [("a", 0, 30, "ia", 0, 0), ("b", 30, 60, "ib", 0, 0), ("c", 60, 90, "ic", 0, 0)],
        [("ia", 30, False), ("ib", 30, False), ("ic", 30, False)]), [op_set("b", 45)])
    add("t01-shrink-middle", ["T01"], "Shrinking b to 10: c moves back, the edit shrinks, the instance does not shrink", base3(), [op_set("b", 10)])
    add("t01-grow-first", ["T01"], "Growing the first shot moves both that follow", base3(), [op_set("a", 50)])
    add("t01-grow-last", ["T01"], "Growing the last shot grows only the edit", base3(), [op_set("c", 40)])
    add("t01-trim-end-same-as-duration", ["T01"], "trim_shot end to frame 70 is set_shot_duration 40", base3(), [op_trim("b", "end", 70)])
    add("t01-noop", ["T01"], "The same duration is a no-op: nothing changes", base3(), [op_set("b", 30)])
    add("t01-shrink-first-to-one", ["T01"], "A shot can shrink to one frame", base3(), [op_set("a", 1)])
    add("t01-too-short", ["T01", "T04"], "Zero frames: SHOT_TOO_SHORT with the legal end frame", base3(), [op_set("b", 0)], "SHOT_TOO_SHORT")
    add("t01-too-short-negative-end", ["T01"], "An end before the start: SHOT_TOO_SHORT, limit start + 1", base3(), [op_trim("b", "end", 25)], "SHOT_TOO_SHORT")
    add("t01-at-limit", ["T01"], "The op at the limit frame does not refuse for the same code", base3(), [op_trim("b", "end", 31)])

    # ---- T02: move_cut and the roll forms ----------------------------------------------------------------
    add("t02-cut-later", ["T02"], "The cut moves 10 frames later: b loses 10 head frames, the edit keeps its length", base3(), [op_cut("a", "b", 40)])
    add("t02-cut-earlier-anchor", ["T02", "T03"], "The cut moves earlier with anchor: b gains head frames, the picture stays", headed(), [op_cut("a", "b", 20, content="anchor")])
    add("t02-cut-earlier-auto", ["T02", "T03"], "auto with enough head frames anchors them all", headed(), [op_cut("a", "b", 20)])
    add("t02-cut-earlier-follow", ["T02", "T03"], "follow: the scene starts at the new cut, the picture of b slides earlier", headed(), [op_cut("a", "b", 20, content="follow")], tags=["FOLLOW"])
    add("t02-trim-end-roll", ["T02"], "trim_shot end in roll is the cut with the next shot", headed(), [op_trim("a", "end", 25, mode="roll")])
    add("t02-trim-start-roll", ["T02"], "trim_shot start in roll is the cut with the previous shot", base3(), [op_trim("b", "start", 40, mode="roll")])
    add("t02-set-duration-roll", ["T02"], "set_shot_duration in roll: the next shot absorbs the difference", headed(), [op_set("a", 20, mode="roll")])
    add("t02-cut-later-last-pair", ["T02"], "Moving the last cut later", base3(), [op_cut("b", "c", 70)])
    add("t02-too-short-left", ["T02", "T04"], "The cut at or before the previous start: SHOT_TOO_SHORT, limit start + 1", base3(), [op_cut("b", "c", 30)], "SHOT_TOO_SHORT")
    add("t02-too-short-right", ["T02"], "The cut at or after the next end: SHOT_TOO_SHORT, limit end - 1", base3(), [op_cut("a", "b", 60)], "SHOT_TOO_SHORT")
    add("t02-noop", ["T02"], "The cut where it is: a no-op", base3(), [op_cut("a", "b", 30)])

    # ---- T03: trim_shot start in ripple and the three contents ------------------------------------------
    add("t03-head-cut-ripple", ["T03"], "Cutting 10 frames of b's head: b stays at its start, its scene advances, c moves back", base3(), [op_trim("b", "start", 40)])
    add("t03-head-earlier-anchor", ["T03"], "Head 10 frames earlier with anchor: b gains frames at the head", headed(), [op_trim("b", "start", 20, content="anchor")])
    add("t03-head-earlier-follow", ["T03"], "Head earlier with follow: the 10 frames go to the tail of the take, with a note only for auto", headed(), [op_trim("b", "start", 20, content="follow")], tags=["FOLLOW"])
    add("t03-head-earlier-auto-mixed", ["T03"], "auto anchors the 6 head frames it has and follows the other 4, with a note", headed(b_inst_start=6, b_cam_in=9), [op_trim("b", "start", 20)], tags=["FOLLOW"])
    add("t03-head-earlier-auto-camera-limits", ["T03"], "The smaller of scene and camera head limits what auto anchors", headed(b_inst_start=20, b_cam_in=3), [op_trim("b", "start", 20)], tags=["FOLLOW"])
    add("t03-first-negative-follow", ["T03"], "The first shot with a negative start under follow: the frames end up at its tail (#32)", base3(), [op_trim("a", "start", -10, content="follow")], tags=["FOLLOW"])
    add("t03-first-negative-auto", ["T03"], "The first shot with a negative start under auto: it has no head, so everything is followed", base3(), [op_trim("a", "start", -10)], tags=["FOLLOW"])
    add("t03-roll-earlier-auto-mixed", ["T03"], "A roll earlier under auto with 6 head frames: anchors 6, follows 4", headed(b_inst_start=6, b_cam_in=6), [op_cut("a", "b", 20)], tags=["FOLLOW"])
    add("t03-head-cut-too-short", ["T03"], "The head cut to the end of the shot: SHOT_TOO_SHORT, limit end - 1", base3(), [op_trim("b", "start", 60)], "SHOT_TOO_SHORT")
    add("t03-head-noop", ["T03"], "The start where it is: a no-op", base3(), [op_trim("b", "start", 30)])

    # ---- T04: set_edit_duration --------------------------------------------------------------------------
    add("t04-edit-grow", ["T04"], "A longer edit grows only the last shot", base3(), [op_edit(100)])
    add("t04-edit-shrink", ["T04"], "A shorter edit shrinks only the last shot", base3(), [op_edit(75)])
    add("t04-edit-noop", ["T04"], "The same length: a no-op", base3(), [op_edit(90)])
    add("t04-edit-too-short", ["T04"], "Not past the start of the last shot: SHOT_TOO_SHORT, limit start + 1", base3(), [op_edit(60)], "SHOT_TOO_SHORT")
    add("t04-edit-at-limit", ["T04"], "The shortest legal edit leaves one frame to the last shot", base3(), [op_edit(61)])

    # ---- T05: continuous joins ----------------------------------------------------------------------------
    add("t05-end-ripple-pulls-continuation", ["T05"], "Growing a: the take that carries on slides in the scene and the camera, the instance grows", chain3(), [op_set("a", 40)])
    add("t05-end-ripple-shrink", ["T05"], "Shrinking a: the continuation slides back; what the edit loses is the tail of the take", chain3(), [op_set("a", 20)])
    add("t05-end-ripple-middle", ["T05"], "Growing b pulls only c", chain3(), [op_set("b", 36)])
    add("t05-roll-valid-later", ["T05", "T02"], "A roll on a continuous join moves both sides of the cut together", chain3(), [op_cut("a", "b", 36)])
    add("t05-roll-earlier-anchor", ["T05", "T02"], "A roll earlier on a continuous join with anchor: the continuity holds", chain3(), [op_cut("a", "b", 24, content="anchor")])
    add("t05-roll-earlier-auto-anchors", ["T05", "T03"], "auto never follows into a continuous join: it anchors", chain3(), [op_cut("a", "b", 24)])
    add("t05-roll-earlier-follow-refused", ["T05", "T03"], "follow into a continuous join: CONTINUOUS_JOIN", chain3(), [op_cut("a", "b", 24, content="follow")], "CONTINUOUS_JOIN")
    add("t05-head-cut-refused", ["T05"], "Cutting the head of a shot that carries on the previous one: CONTINUOUS_JOIN", chain3(), [op_trim("b", "start", 36)], "CONTINUOUS_JOIN")
    add("t05-head-extend-refused", ["T05"], "Extending that head is CONTINUOUS_JOIN too", chain3(), [op_trim("b", "start", 24)], "CONTINUOUS_JOIN")
    add("t05-roll-follow-continuation-outside", ["T05", "T03"], "follow on a cut whose entering shot is carried on by the next: the take slides in the scene", mk_doc(
        [("a", 0, 30, "ia", 0, 0), ("b", 30, 60, "ib", 0, 0), ("c", 60, 90, "ib", 30, 30, CONT)],
        [("ia", 60, False), ("ib", 120, False)]), [op_cut("a", "b", 20, content="follow")], tags=["FOLLOW"])

    # ---- T07: LOCKED ----------------------------------------------------------------------------------------
    add("t07-ripple-between-breaks-step", ["T07"], "Growing b moves c on the edit but not in its scene: the pair of a locked instance would leave its step", locked_pair(), [op_set("b", 40)], "LOCKED")
    add("t07-ripple-first-of-pair", ["T07"], "Growing a moves c: LOCKED", locked_pair(), [op_set("a", 40)], "LOCKED")
    add("t07-not-in-step-is-free", ["T07"], "Shots of the locked instance that were not in step have nothing to keep", locked_pair(c_inst_start=50, c_cam_in=50), [op_set("b", 40)])
    add("t07-single-shot-locked-is-free", ["T07"], "A locked instance with one shot has no step to keep", mk_doc(
        [("a", 0, 30, "ia", 0, 0), ("b", 30, 60, "ib", 0, 0), ("c", 60, 90, "ic", 0, 0)],
        [("ia", 60, True), ("ib", 60, True), ("ic", 60, True)]), [op_set("b", 40)])
    add("t07-follow-breaks-step", ["T07", "T03"], "follow on c moves its step away from a's: LOCKED", locked_pair(), [op_cut("b", "c", 50, content="follow")], "LOCKED", tags=["FOLLOW"])
    add("t07-auto-cannot-follow", ["T07", "T03"], "auto with too few head frames would follow: LOCKED", locked_pair(c_inst_start=60, c_cam_in=4), [op_cut("b", "c", 50)], "LOCKED", tags=["FOLLOW"])
    add("t07-anchor-keeps-step", ["T07", "T03"], "anchor keeps the step of c, so a locked pair stays in step", locked_pair(), [op_cut("b", "c", 50, content="anchor")])
    add("t07-auto-anchors-when-it-can", ["T07", "T03"], "auto with enough head frames anchors, so a locked pair stays in step", locked_pair(), [op_cut("b", "c", 50)])
    add("t07-follow-continuation-in-step", ["T07", "T05"], "The entering shot and its continuation stay in step with each other under follow", mk_doc(
        [("a", 0, 30, "ia", 0, 0), ("b", 30, 60, "il", 0, 0), ("c", 60, 90, "il", 30, 30, CONT)],
        [("ia", 60, False), ("il", 120, True)]), [op_cut("a", "b", 20, content="follow")], tags=["FOLLOW"])
    add("t07-follow-with-other-in-take-outside", ["T07", "T03"], "Another shot of the instance outside the take that was in step: follow is LOCKED", mk_doc(
        [("a", 0, 30, "il", 0, 0), ("b", 30, 60, "il", 30, 30, CUT), ("c", 60, 90, "ic", 0, 0)],
        [("il", 120, True), ("ic", 60, False)]), [op_cut("a", "b", 20, content="follow")], "LOCKED", tags=["FOLLOW"])

    # ---- T08: the instance grows and never shrinks ---------------------------------------------------------
    add("t08-never-shrinks", ["T08"], "Shrinking a shot leaves the instance as long as it was, with no note", base3(), [op_set("b", 5)])
    add("t08-shared-instance-max", ["T08"], "Two shots on one instance: it grows to what the more demanding one needs", mk_doc(
        [("a", 0, 30, "ix", 0, 0), ("b", 30, 60, "iy", 0, 0), ("c", 60, 90, "ix", 40, 40)],
        [("ix", 70, False), ("iy", 60, False)]), [op_set("c", 50)])
    add("t08-grows-by-continuation-slide", ["T08", "T05"], "The slide of a take that carries on makes the instance grow", chain3(), [op_set("b", 31)])
    add("t08-follow-grows-instance", ["T08", "T03"], "follow shows more of the tail of the scene: the instance grows", mk_doc(
        [("a", 0, 30, "ia", 0, 0), ("b", 30, 60, "ib", 0, 0), ("c", 60, 90, "ic", 0, 0)],
        [("ia", 60, False), ("ib", 30, False), ("ic", 60, False)]), [op_cut("a", "b", 20, content="follow")], tags=["FOLLOW"])

    # ---- T09: NO_HEAD_FRAMES ------------------------------------------------------------------------------
    add("t09-anchor-scene-short", ["T09"], "anchor with 5 head frames and a cut 10 earlier: NO_HEAD_FRAMES, limit 25", headed(b_inst_start=5, b_cam_in=20), [op_cut("a", "b", 20, content="anchor")], "NO_HEAD_FRAMES")
    add("t09-anchor-camera-short", ["T09"], "The camera clip is the one that has too few frames: the limit is the smaller", headed(b_inst_start=20, b_cam_in=3), [op_cut("a", "b", 20, content="anchor")], "NO_HEAD_FRAMES")
    add("t09-anchor-at-limit", ["T09"], "At the limit frame anchor passes", headed(b_inst_start=5, b_cam_in=20), [op_cut("a", "b", 25, content="anchor")])
    add("t09-head-ripple-anchor", ["T09", "T03"], "trim start earlier with anchor and too few head frames: NO_HEAD_FRAMES", headed(b_inst_start=5, b_cam_in=5), [op_trim("b", "start", 20, content="anchor")], "NO_HEAD_FRAMES")
    add("t09-head-ripple-at-limit", ["T09", "T03"], "trim start at the limit with anchor", headed(b_inst_start=5, b_cam_in=5), [op_trim("b", "start", 25, content="anchor")])
    add("t09-auto-never-refuses", ["T09", "T03"], "auto does not refuse for lack of head frames", headed(b_inst_start=0, b_cam_in=0), [op_cut("a", "b", 20)], tags=["FOLLOW"])

    # ---- T10: limits and unknown ids ---------------------------------------------------------------------
    add("t10-edit-too-long", ["T10"], "More than 216000 frames: EDIT_TOO_LONG with the limit", base3(), [op_edit(MAX_EDIT_FRAMES + 1)], "EDIT_TOO_LONG")
    add("t10-edit-at-limit", ["T10"], "Exactly 216000 frames is allowed", base3(), [op_edit(MAX_EDIT_FRAMES)])
    add("t10-shot-duration-too-long", ["T10"], "A shot duration that takes the edit past the limit: the limit is the end frame", base3(), [op_set("a", MAX_EDIT_FRAMES)], "EDIT_TOO_LONG")
    add("t10-head-extend-too-long", ["T10", "T03"], "A head extended past the limit", headed(), [op_trim("b", "start", 30 - MAX_EDIT_FRAMES)], "EDIT_TOO_LONG")
    add("t10-shot-not-found", ["T10"], "An unknown shot id: SHOT_NOT_FOUND", base3(), [op_set("zz", 10)], "SHOT_NOT_FOUND")
    add("t10-cut-shot-not-found", ["T10"], "An unknown id in move_cut", base3(), [op_cut("a", "zz", 40)], "SHOT_NOT_FOUND")
    add("t10-not-adjacent-reversed", ["T10"], "move_cut with the shots in the wrong order: SHOTS_NOT_ADJACENT", base3(), [op_cut("b", "a", 20)], "SHOTS_NOT_ADJACENT")
    add("t10-not-adjacent-skipping", ["T10"], "move_cut between a and c: SHOTS_NOT_ADJACENT", base3(), [op_cut("a", "c", 45)], "SHOTS_NOT_ADJACENT")
    add("t10-roll-last-end", ["T10", "T02"], "The end of the last shot has no shot after it to roll with", base3(), [op_trim("c", "end", 80, mode="roll")], "SHOTS_NOT_ADJACENT")
    add("t10-roll-first-start", ["T10", "T02"], "The start of the first shot has no shot before it to roll with", base3(), [op_trim("a", "start", 5, mode="roll")], "SHOTS_NOT_ADJACENT")
    add("t10-with-narration-refused", ["T10", "T17"], "with_narration is refused in this phase (D13)", base3(extra={"narration": {"tracks": []}}), [op_set("b", 40, with_narration=True)], "SHOTS_OPTION_NOT_YET")
    add("t10-shots-out-of-order-in-document", ["T10"], "The document may list the shots out of order: the planner reads them by start", mk_doc(
        [("c", 60, 90, "ic", 0, 0, CUT), ("a", 0, 30, "ia", 0, 0), ("b", 30, 60, "ib", 0, 0, CUT)],
        [("ia", 60, False), ("ib", 60, False), ("ic", 60, False)]), [op_set("b", 40)])

    # ---- T11: clips of a shot ----------------------------------------------------------------------------
    shot_clips = [{"id": "fx", "clips": [
        clip("s1", 32, 6, shot="b"),
        clip("s2", 40, 15, shot="b", keys={"amount": [(0, 0.2), (7, 0.5), (14, 0.9)]}, fade_in=3, fade_out=3),
        clip("s3", 56, 4, shot="b"),
    ]}]
    add("t11-head-cut-moves-clips", ["T11", "T13"], "Cutting the head by 10: clips of b move back with the picture and are cut at its start", base3(tracks=shot_clips), [op_trim("b", "start", 40)])
    add("t11-end-shrink-cuts-tail", ["T11"], "Shrinking b cuts the clips at its new end and removes the ones left outside", base3(tracks=shot_clips), [op_set("b", 15)])
    add("t11-grow-keeps-clips", ["T11"], "Growing b leaves its clips where they are", base3(tracks=shot_clips), [op_set("b", 45)])
    add("t11-head-earlier-anchor-moves-clips", ["T11"], "Head earlier with anchor: the picture moves later by 10 and so do the clips", headed(tracks=shot_clips), [op_trim("b", "start", 20, content="anchor")])
    add("t11-roll-follow-slides-clips", ["T11", "T03"], "A roll earlier with follow: the clips of b slide earlier with the picture", headed(tracks=shot_clips), [op_cut("a", "b", 20, content="follow")], tags=["FOLLOW"])
    add("t11-roll-later-cuts-head-clips", ["T11", "T02"], "The cut later: b loses its head and the clips that were there", base3(tracks=shot_clips), [op_cut("a", "b", 42)])
    add("t11-continuation-clips-follow", ["T11", "T05"], "The clip of a shot that carries on slides with its picture", mk_doc(
        [("a", 0, 30, "ia", 0, 0), ("b", 30, 60, "ib", 0, 0), ("c", 60, 90, "ib", 30, 30, CONT)],
        [("ia", 60, False), ("ib", 120, False)],
        tracks=[{"id": "fx", "clips": [clip("cc", 64, 10, shot="c")]}]), [op_cut("a", "b", 20, content="follow")], tags=["FOLLOW"])
    add("t11-later-shot-clips-move-with-plan", ["T11", "T01"], "The clips of a shot after the edited one move with it", base3(tracks=[{"id": "fx", "clips": [clip("cc", 62, 10, shot="c")]}]), [op_set("a", 50)])

    # ---- T12: clips of the output ---------------------------------------------------------------------------
    out_clips = [{"id": "o", "clips": [
        clip("o1", 5, 10),
        clip("o2", 40, 10),
        clip("o3", 25, 20),
        clip("o4", 55, 25),
        clip("o5", 70, 15),
    ]}]
    out_tracks = [{"id": "o", "clips": [clip("o1", 5, 10)]}, {"id": "p", "clips": [clip("o2", 40, 10)]}, {"id": "q", "clips": [clip("o3", 25, 20)]}, {"id": "r", "clips": [clip("o4", 55, 25)]}, {"id": "s", "clips": [clip("o5", 70, 15)]}]
    del out_clips
    add("t12-shrink-removes-and-cuts", ["T12"], "Shrinking b by 20: clips before stay, one inside the removed stretch goes, crossing ones are cut, later ones move back", base3(tracks=out_tracks), [op_set("b", 10)])
    add("t12-grow-stretches-crossing", ["T12"], "Growing b by 10: later clips move, a crossing clip is stretched", base3(tracks=out_tracks), [op_set("b", 40)])
    add("t12-keep-absolute-nothing-moves", ["T12"], "keep_absolute leaves every output clip where it is", base3(tracks=out_tracks), [op_set("b", 40, keep_absolute=True)])
    add("t12-keep-absolute-out-of-range", ["T12"], "keep_absolute with an edit that shrinks: a clip leaves the edit and the gate refuses (FX_CLIP_OUT_OF_RANGE)", base3(tracks=out_tracks), [op_edit(80, keep_absolute=True)], "CANDIDATE_REJECTED")
    add("t12-keep-absolute-overlap", ["T12"], "keep_absolute and a clip of a shot that moves into an output clip of the same track: FX_CLIP_OVERLAP", headed(tracks=[{"id": "t", "clips": [clip("so", 32, 6, shot="b"), clip("oo", 44, 4)]}]), [op_trim("b", "start", 20, content="anchor", keep_absolute=True)], "CANDIDATE_REJECTED")
    add("t12-head-cut-clip", ["T12", "T13"], "A clip whose head falls in the removed stretch loses its head", base3(tracks=[{"id": "t", "clips": [clip("oh", 50, 25, keys={"x": [(0, 1), (9, 2), (10, 3), (20, 4)]})]}]), [op_set("b", 10)])
    add("t12-roll-leaves-output", ["T12", "T02"], "A roll with anchor does not move any output clip", headed(tracks=out_tracks), [op_cut("a", "b", 20, content="anchor")])
    add("t12-roll-follow-chain", ["T12", "T03"], "A roll earlier with follow: output clips in the take slide earlier, the ceded stretch is cut", headed(tracks=out_tracks), [op_cut("a", "b", 20, content="follow")], tags=["FOLLOW"])
    add("t12-head-earlier-chain", ["T12", "T03"], "Head earlier with auto mixed: the output clips pass through two insertions", headed(b_inst_start=6, b_cam_in=6, tracks=out_tracks), [op_trim("b", "start", 20)], tags=["FOLLOW"])
    add("t12-continuation-chain-end", ["T12", "T05"], "The splice of a take is at the end of the chain, not at the end of the edited shot", chain3(tracks=[{"id": "t", "clips": [clip("w1", 20, 20), clip("w2", 70, 15)]}]), [op_set("a", 40)])

    # ---- T13: keys and fades ------------------------------------------------------------------------------
    keyed = [{"id": "t", "clips": [clip("k1", 40, 30, keys={"amount": [(0, 0.1), (12, 0.4), (25, 0.8)], "tint": [(5, 1.0)]}, fade_in=10, fade_out=10)]}]
    add("t13-moves-whole-keys-intact", ["T13", "T12"], "A clip that moves whole keeps its keys as they are", base3(tracks=[{"id": "t", "clips": [clip("k1", 62, 20, keys={"amount": [(0, 0.1), (19, 0.8)]})]}]), [op_set("a", 50)])
    add("t13-head-cut-shifts-and-prunes", ["T13"], "A clip with its head cut: keys shift by the cut and the ones below 0 are dropped, with a note", base3(tracks=keyed), [op_set("b", 5)])
    add("t13-tail-cut-prunes", ["T13"], "An output clip with its tail cut: keys at the new end or after are dropped", base3(tracks=[{"id": "t", "clips": [clip("k2", 60, 25, keys={"amount": [(0, 0.1), (10, 0.5), (24, 0.9)]})]}]), [op_edit(70)])
    add("t13-stretched-keeps-keys", ["T13"], "A stretched clip keeps its keys at the same relative frames", base3(tracks=[{"id": "t", "clips": [clip("k3", 40, 30, keys={"amount": [(0, 0.1), (29, 0.9)]})]}]), [op_set("b", 50)])
    add("t13-fade-reduced", ["T13"], "Fades that no longer fit are reduced, the fade in first", base3(tracks=[{"id": "t", "clips": [clip("f1", 40, 30, fade_in=14, fade_out=14)]}]), [op_set("b", 20)])
    add("t13-fade-out-reduced", ["T13"], "When the fade in alone fits, the fade out takes the excess", base3(tracks=[{"id": "t", "clips": [clip("f2", 45, 20, fade_in=4, fade_out=14)]}]), [op_set("b", 20)])
    add("t13-fade-gone", ["T13"], "A fade reduced to zero is removed from the clip", base3(tracks=[{"id": "t", "clips": [clip("f3", 40, 30, fade_in=10, fade_out=20)]}]), [op_set("b", 5)])
    add("t13-all-keys-pruned", ["T13"], "A clip that loses every key has no `keys`", base3(tracks=[{"id": "t", "clips": [clip("k4", 50, 25, keys={"amount": [(0, 0.5), (3, 0.6)]})]}]), [op_set("b", 15)])

    # ---- T14: transitions ---------------------------------------------------------------------------------
    t_center = [transition("tr1", "b", "c", 10, "center")]
    add("t14-fits-untouched", ["T14"], "A transition whose shots keep their length is left alone", base3(transitions=t_center), [op_set("a", 40)])
    add("t14-too-long", ["T14"], "b too short for a centred transition of 10: TRANSITION_TOO_LONG with the legal end frame", base3(transitions=t_center), [op_set("b", 3)], "TRANSITION_TOO_LONG")
    add("t14-at-limit", ["T14"], "The op at the limit frame fits", base3(transitions=t_center), [op_set("b", 5)])
    add("t14-shrink-center", ["T14"], "shrink_transitions reduces the transition to the longest the shots hold", base3(transitions=t_center), [op_set("b", 3, shrink_transitions=True)])
    add("t14-shrink-start", ["T14"], "A transition aligned at the start can be as long as the incoming shot", base3(transitions=[transition("tr1", "b", "c", 20, "start")]), [op_cut("b", "c", 75, shrink_transitions=True)])
    add("t14-shrink-end", ["T14"], "A transition aligned at the end can be as long as the outgoing shot", base3(transitions=[transition("tr1", "b", "c", 20, "end")]), [op_set("b", 8, shrink_transitions=True)])
    add("t14-too-long-start-incoming", ["T14"], "A start-aligned transition needs the incoming shot to be as long", base3(transitions=[transition("tr1", "b", "c", 20, "start")]), [op_set("c", 12)], "TRANSITION_TOO_LONG")
    add("t14-too-long-roll", ["T14", "T02"], "A roll that leaves a shot too short for its transition: the limit is the nearest legal cut", base3(transitions=[transition("tr1", "b", "c", 10, "center")]), [op_cut("b", "c", 33)], "TRANSITION_TOO_LONG")
    add("t14-too-long-head", ["T14", "T03"], "A head cut that leaves the incoming shot too short for its transition", base3(transitions=[transition("tr1", "a", "b", 20, "start")]), [op_trim("b", "start", 45)], "TRANSITION_TOO_LONG")
    add("t14-edit-duration", ["T14", "T04"], "set_edit_duration judges the transitions of the last shot", base3(transitions=[transition("tr1", "b", "c", 10, "center")]), [op_edit(62)], "TRANSITION_TOO_LONG")
    add("t14-two-transitions-shrink", ["T14"], "Two transitions reduced in one batch, each by the op that shortens the shot it lies in", base3(transitions=[transition("tr1", "a", "b", 20, "end"), transition("tr2", "b", "c", 20, "start")]), [op_set("a", 6, shrink_transitions=True), op_edit(42, shrink_transitions=True)])
    add("t14-keeps-others", ["T14"], "A transition between untouched shots is not judged", mk_doc(
        [("a", 0, 30, "ia", 0, 0), ("b", 30, 60, "ib", 0, 0), ("c", 60, 90, "ic", 0, 0), ("d", 90, 120, "id", 0, 0)],
        [("ia", 60, False), ("ib", 60, False), ("ic", 60, False), ("id", 60, False)],
        transitions=[transition("tr1", "b", "c", 10, "center")]), [op_set("d", 5)])

    # ---- T14b: handles and the reserve of auto (needs the admission of transitions of the destination) ------
    hold_use = [transition("tr1", "b", "c", 6, "end", handles="use")]
    locked_head = mk_doc(
        [("a", 0, 30, "il", 0, 0), ("b", 30, 60, "il", 30, 30), ("c", 60, 90, "ib", 0, 0)],
        [("il", 120, True), ("ib", 60, False)])
    add("t07-head-ripple-auto-follows-to-keep-step", ["T07", "T03"], "a and b are in step on a locked instance: anchoring b's new head frames would break the step, so auto follows", locked_head, [op_trim("b", "start", 20)], tags=["FOLLOW"])
    add("t07-head-ripple-anchor-locked", ["T07", "T03"], "The same head ripple with anchor breaks the step: LOCKED", locked_head, [op_trim("b", "start", 20, content="anchor")], "LOCKED")
    add("t14b-anchor-loses-head-handle", ["T14b"], "anchor takes the head frames the transition needs: the gate refuses with TRANSITION_NO_HANDLES", mk_doc(
        [("a", 0, 30, "ia", 0, 0), ("b", 30, 60, "ib", 0, 0), ("c", 60, 90, "ic", 10, 10)],
        [("ia", 60, False), ("ib", 60, False), ("ic", 120, False)], transitions=hold_use), [op_cut("b", "c", 54, content="anchor")], "CANDIDATE_REJECTED", tags=["AUTO_RESERVE"])
    add("t14b-auto-keeps-reserve", ["T14b", "T03"], "auto keeps the 6 head frames the transition draws before the cut and follows the rest", mk_doc(
        [("a", 0, 30, "ia", 0, 0), ("b", 30, 60, "ib", 0, 0), ("c", 60, 90, "ic", 10, 10)],
        [("ia", 60, False), ("ib", 60, False), ("ic", 120, False)], transitions=hold_use), [op_cut("b", "c", 54)], tags=["AUTO_RESERVE", "FOLLOW"])
    add("t14b-auto-reserve-small-slip", ["T14b", "T03"], "A slip shorter than the head minus the reserve is anchored whole (the literal reading would follow it)", mk_doc(
        [("a", 0, 30, "ia", 0, 0), ("b", 30, 60, "ib", 0, 0), ("c", 60, 90, "ic", 20, 20)],
        [("ia", 60, False), ("ib", 60, False), ("ic", 120, False)], transitions=hold_use), [op_cut("b", "c", 56)], tags=["AUTO_RESERVE"])
    add("t14b-hold-needs-nothing", ["T14b"], "A transition with handles hold needs no frames: the same roll passes", mk_doc(
        [("a", 0, 30, "ia", 0, 0), ("b", 30, 60, "ib", 0, 0), ("c", 60, 90, "ic", 10, 10)],
        [("ia", 60, False), ("ib", 60, False), ("ic", 120, False)], transitions=[transition("tr1", "b", "c", 6, "end")]), [op_cut("b", "c", 54, content="anchor")])
    add("t14b-follow-spends-tail-handle", ["T14b"], "auto follows 6 frames into c, which shows its scene to the end: the use transition out of c lacks tail frames (#29)", mk_doc(
        [("a", 0, 30, "ia", 0, 0), ("b", 30, 60, "ib", 0, 0), ("c", 60, 90, "ic", 4, 4), ("d", 90, 120, "id", 0, 0)],
        [("ia", 60, False), ("ib", 60, False), ("ic", 40, False), ("id", 60, False)], transitions=[transition("tr1", "c", "d", 6, "start", handles="use")]), [op_cut("b", "c", 50)], "CANDIDATE_REJECTED", tags=["AUTO_RESERVE", "FOLLOW"])
    add("t14b-locked-auto-falls-back", ["T07", "T14b"], "auto cannot follow in a locked pair, so it anchors all 10: the transition then lacks head frames", mk_doc(
        [("a", 0, 30, "il", 0, 0), ("b", 30, 60, "ib", 0, 0), ("c", 60, 90, "il", 60, 12)],
        [("il", 120, True), ("ib", 60, False)], transitions=[transition("tr1", "b", "c", 6, "end", handles="use")]), [op_cut("b", "c", 50)], "CANDIDATE_REJECTED", tags=["AUTO_RESERVE", "FOLLOW"])

    # ---- T15 and T16: keys of the light and fields of the deferred list --------------------------------------
    lit = [{"id": "def-ia", "lighting": {"ambient": 1, "lights": [{"id": "lamp", "keys": {
        "intensity": key_list([(5, 1.0), (45, 2.0), (70, 3.0)]),
        "color": key_list([(50, 0.5)]),
        "position": key_list([(0, 0.0), (65, 1.0)]),
    }}, {"id": "torch"}]}}, {"id": "def-ib"}]
    add("t15-keys-move-after-point", ["T15"], "Growing b by 10 moves the keys after the point by 10", base3(defs=copy.deepcopy(lit)), [op_set("b", 40)])
    add("t15-keys-dropped-in-removed", ["T15"], "A removed stretch drops the keys inside it, with a note, and moves the later ones back", base3(defs=copy.deepcopy(lit)), [op_set("b", 10)])
    add("t15-empty-track-removed", ["T15"], "A track left without keys is removed, and the empty `keys` too", base3(defs=[{"id": "def-ia", "lighting": {"ambient": 1, "lights": [{"id": "lamp", "keys": {"color": key_list([(40, 0.5), (50, 0.7)])}}]}}]), [op_set("b", 10)])
    add("t15-keep-absolute", ["T15"], "keep_absolute leaves the keys where they are", base3(defs=copy.deepcopy(lit)), [op_set("b", 10, keep_absolute=True)])
    add("t15-key-at-insertion-point-moves", ["T15"], "A key at the frame where frames are inserted moves with them", base3(defs=[{"id": "def-ia", "lighting": {"ambient": 1, "lights": [{"id": "lamp", "keys": {"intensity": key_list([(60, 1.0)])}}]}}]), [op_set("b", 40)])
    add("t15-roll-follow-chain", ["T15", "T03"], "A roll earlier with follow passes the keys through the chain", headed(defs=copy.deepcopy(lit)), [op_cut("a", "b", 20, content="follow")], tags=["FOLLOW"])
    add("t15-roll-anchor-leaves-keys", ["T15", "T02"], "A roll with anchor leaves every key", headed(defs=copy.deepcopy(lit)), [op_cut("a", "b", 20, content="anchor")])
    weather = [{"id": "def-ia", "weather": "snow:0.5", "weatherKeys": key_list([(10, 0.1), (50, 0.9), (80, 0.3)])}]
    add("t16-weather-keys-remapped", ["T16"], "weatherKeys (edit frames) move after the point like the keys of a light", base3(defs=copy.deepcopy(weather)), [op_set("b", 40)])
    add("t16-weather-keys-dropped", ["T16"], "weatherKeys inside the removed stretch are dropped with a note", base3(defs=copy.deepcopy(weather)), [op_set("b", 10)])
    add("t16-weather-keys-emptied", ["T16"], "weatherKeys left empty are removed", base3(defs=[{"id": "def-ia", "weather": "snow:0.5", "weatherKeys": key_list([(40, 0.1), (50, 0.9)])}]), [op_set("b", 10)])
    add("t16-weather-keep-absolute", ["T16"], "keep_absolute leaves weatherKeys", base3(defs=copy.deepcopy(weather)), [op_set("b", 10, keep_absolute=True)])
    other = [{"id": "def-ia", "weather": "rain", "contactPlaneId": "floor",
              "effects": {"film": 0.5},
              "rigs": [{"id": "r1", "constraints": [{"id": "ik", "target": {"keys": key_list([(10, 0.0), (40, 1.0)])}}], "follows": [{"id": "f1", "progress": key_list([(0, 0.0), (50, 1.0)])}]}]}]
    add("t16-instance-clock-and-narration-stay", ["T16", "T17"], "Rig keys (instance frames), effects, narration and outline do not move", base3(defs=other, extra={
        "narration": {"tracks": [{"id": "n", "clips": [{"id": "l1", "startSeconds": 1.5, "durationSeconds": 2.0}]}]},
        "outline": {"groups": [], "items": []}, "audio": {"editStartSeconds": 0, "editEndSecondsExclusive": 3.75}}), [op_set("a", 50)])

    # ---- batches ----------------------------------------------------------------------------------------------
    add("batch-three-ops", ["T01", "T02", "T04"], "A batch runs in order on one candidate: grow b, move the cut after it, set the edit length", base3(), [op_set("b", 40), op_cut("a", "b", 20), op_edit(120)])
    add("batch-second-op-refused", ["T10"], "A refusal at index 1 leaves the whole document as it was", base3(), [op_set("b", 40), op_set("zz", 10)], "SHOT_NOT_FOUND")
    add("batch-sees-previous-op", ["T01"], "The second op reads the shots the first one left", base3(), [op_set("a", 50), op_set("c", 10)])
    add("batch-grow-then-shrink", ["T01", "T08"], "Growing and shrinking the same shot: the instance keeps the longer", base3(), [op_set("b", 80), op_set("b", 10)])
    add("batch-with-output-clips", ["T12", "T13", "T15"], "A batch moving clips and keys through two chains", base3(tracks=[{"id": "o", "clips": [clip("o1", 40, 30, keys={"x": [(0, 1), (25, 2)]})]}], defs=copy.deepcopy(lit)), [op_set("b", 50), op_set("a", 10)])
    add("batch-gate-refusal", ["T12"], "The gate runs once at the end: a batch that ends with a clip outside the edit is CANDIDATE_REJECTED", base3(tracks=out_tracks), [op_set("c", 50, keep_absolute=True), op_edit(80, keep_absolute=True)], "CANDIDATE_REJECTED")

    # ---- T19: explain ---------------------------------------------------------------------------------------
    add_explain("t19-explain-handles", ["T19"], "Handles of scene and camera clip before and after what each shot shows", mk_doc(
        [("a", 0, 30, "ia", 5, 7), ("b", 30, 60, "ib", 0, 0), ("c", 60, 90, "ia", 40, 20)],
        [("ia", 100, False), ("ib", 30, False)], cam_frames=60,
        extra={"audio": {"editStartSeconds": 0, "editEndSecondsExclusive": 3.75}}))
    add_explain("t19-explain-locked-and-continuous", ["T19"], "The locked flag and the transition into each shot", mk_doc(
        [("a", 0, 30, "il", 0, 0), ("b", 30, 60, "il", 30, 30, CONT), ("c", 60, 90, "ic", 0, 0, CUT)],
        [("il", 90, True), ("ic", 60, False)],
        extra={"audio": {"editStartSeconds": 0, "editEndSecondsExclusive": 3.75}}))
    add_explain("t19-audio-longer-than-edit", ["T19"], "The audio passes the edit by more than a frame", base3(extra={"audio": {"editStartSeconds": 0, "editEndSecondsExclusive": 5.0}}))
    add_explain("t19-edit-longer-than-audio", ["T19"], "The edit passes the audio by more than a frame", base3(extra={"audio": {"editStartSeconds": 0, "editEndSecondsExclusive": 2.5}}))
    add_explain("t19-audio-within-one-frame", ["T19"], "One frame of slack on either side: no warning", base3(extra={"audio": {"editStartSeconds": 0, "editEndSecondsExclusive": 3.75}}))
    add_explain("t19-no-audio", ["T19"], "Without audio there is no warning", base3())
    add_explain("t19-window-not-at-zero", ["T19"], "The window 2.0-3.75 s lasts 1.75 s (42 frames) on the edit: the 90-frame edit passes it", base3(extra={"audio": {"editStartSeconds": 2.0, "editEndSecondsExclusive": 3.75}}))
    return cases


def run_case(case):
    """The result the reference gives to a case, in the shape stored in the corpus."""
    if case["kind"] == "explain":
        return {"explain": explain_shots(case["document"])}
    result = apply_ops(case["document"], case["ops"])
    out = {"ok": result["ok"]}
    if result["ok"]:
        out["changed"] = result["changed"]
        out["document"] = result["document"]
        out["notes"] = result["notes"]
    else:
        out["refusal"] = result["refusal"]
    return out


def _outcome(result):
    if "explain" in result:
        return "ok"
    if result["ok"]:
        return "ok"
    return result["refusal"]["code"]


def op_at_limit(document, wrapped, limit_frame):
    """The same op with its frame moved to `limit_frame` (#33: always a frame of the edit, in the op's own frame; in
    the duration ops it is the END of the shot, so the duration is `limit_frame - start`)."""
    name = next(iter(wrapped))
    body = dict(wrapped[name])
    if name in ("trim_shot", "move_cut"):
        body["frame"] = limit_frame
    elif name == "set_shot_duration":
        body["frames"] = limit_frame - _start(_find_shot(document, body["shot"]))
    else:
        body["frames"] = limit_frame
    return {name: body}


def _limit_companion(case, result):
    """A refusal that names a `limitFrame` gets a twin whose op sits AT that frame: it must not refuse with the same
    code (#33). Only single-op cases (a batch would have to rebuild the intermediate document)."""
    if case["kind"] != "ops" or "ok" not in result or result["ok"] or len(case["ops"]) != 1:
        return None
    refusal = result["refusal"]
    if refusal.get("limitFrame") is None:
        return None
    return {
        "id": case["id"] + "@limit",
        "kind": "ops",
        "rows": list(case["rows"]) + ["T21"],
        "title": "At the limitFrame of %s the op does not refuse with %s" % (case["id"], refusal["code"]),
        "tags": list(case["tags"]) + ["AT_LIMIT"],
        "document": case["document"],
        "ops": [op_at_limit(case["document"], case["ops"][0], refusal["limitFrame"])],
        "want": "not " + refusal["code"],
    }


def build_corpus():
    """`cases.json` as a Python value. Raises when a case does not do what its author meant."""
    cases = []
    for case in build_cases():
        result = run_case(case)
        outcome = _outcome(result)
        want = case["want"]
        if outcome != want:
            raise SystemExit("ORACLE_CASE_MISMATCH: %s wanted %s, the reference gave %s: %s" % (case["id"], want, outcome, json.dumps(result.get("refusal"))))
        entry = dict((key, case[key]) for key in ("id", "kind", "rows", "title", "tags", "document", "ops"))
        entry["expect"] = result
        cases.append(entry)
        companion = _limit_companion(case, result)
        if companion is None:
            continue
        again = run_case(companion)
        code = result["refusal"]["code"]
        if _outcome(again) == code:
            raise SystemExit("ORACLE_LIMIT_MISMATCH: %s refuses again with %s at its own limitFrame %s" % (case["id"], code, result["refusal"]["limitFrame"]))
        extra = dict((key, companion[key]) for key in ("id", "kind", "rows", "title", "tags", "document", "ops"))
        extra["limit_of"] = case["id"]
        extra["expect"] = again
        cases.append(extra)
    return {
        "schema": SCHEMA,
        "generator": GENERATOR_VERSION,
        "kind": "origin-text transcription (ORIGIN_NOT_EXECUTED)",
        "max_edit_frames": MAX_EDIT_FRAMES,
        "oracle_choices": ORACLE_CHOICES,
        "deferred_audit": DEFERRED_AUDIT,
        "d07_audit": D07_AUDIT,
        "cases": cases,
    }


# =========================================================================================================
# The manifest and the command line
# =========================================================================================================


def _sha256_file(path):
    """The sha256 of a text file with the line ends normalised to LF (a checkout with CRLF hashes the same)."""
    return hashlib.sha256(Path(path).read_bytes().replace(b"\r\n", b"\n")).hexdigest()


def build_manifest(corpus_text):
    sources = []
    for which, path, blob, digest in ORIGIN_SOURCES:
        sources.append({"commit": PIN if which == "pin" else TIP, "which": which, "path": origin_path(path), "blob": blob, "sha256": digest})
    docs = [{"path": path, "sha256": _sha256_file(ROOT / path)} for path in ORIGIN_DOCS]
    return {
        "schema": SCHEMA,
        "generator": GENERATOR_VERSION,
        "flags": ["ORIGIN_NOT_EXECUTED", "PLAN_COMMANDS_ABSENT_FROM_CLONE", "PIN_HAS_NO_FX"],
        "origin": {
            "repository": "journey-studios tesselapp, local clone, read only by git objects",
            "pin": PIN,
            "tip": TIP,
            "notes": [
                "ORIGIN_NOT_EXECUTED: no origin code was run; every rule is a transcription of the two documents or of the file read.",
                "PLAN_COMMANDS_ABSENT_FROM_CLONE: `packages/scene-engine/src/shots/` (the pilot of the plan commands) does not exist in the clone; no `trimShot` appears in any ref.",
                "PIN_HAS_NO_FX: `d6adac15` is the main of the origin and has no `src/fx/`, `evaluateShotFrame` or `resolveShotAtOffset`; those are read at the tip `3655790` (the effects branch the P4b oracle already reads). The schema, integrity, joins, evaluator of the edit frame, light/weather keys, narration and rig files are read at the pin.",
            ],
        },
        "rule_documents": docs,
        "rule_sources": sources,
        "deferred_fields_source": DEFERRED_FIELDS_SOURCE,
        "oracle_choices": sorted(ORACLE_CHOICES),
        "corpus": {"file": CASES_NAME, "sha256": hashlib.sha256(corpus_text.encode("utf-8")).hexdigest(), "cases": len(json.loads(corpus_text)["cases"])},
        "regenerate": "python scripts/shot-trim-reference.py   # writes; add --check to compare without writing",
    }


def dump(value):
    return json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n"


def generate_texts():
    corpus_text = dump(build_corpus())
    return {CASES_NAME: corpus_text, MANIFEST_NAME: dump(build_manifest(corpus_text))}


def evidence_refs():
    """Every `which:path:first-last` the audit and the D07 conclusion cite, as `(which, path, first, last)`."""
    refs = []
    items = [(entry["field"], entry["evidence"]) for entry in DEFERRED_AUDIT]
    items.extend((finding["claim"], finding["evidence"]) for finding in D07_AUDIT["findings"])
    for _, evidence in items:
        for text in evidence:
            which, rest = text.split(":", 1)
            path, span = rest.rsplit(":", 1)
            first, _, last = span.partition("-")
            refs.append((which, path, int(first), int(last or first)))
    return refs


def verify_origin(path):
    """Re-reads every source blob of the manifest by git id, read only, compares the sha256, and checks that every
    cited line range exists in the file it cites."""
    findings = []
    lines = {}
    for which, rel, blob, digest in ORIGIN_SOURCES:
        try:
            data = subprocess.run(["git", "-C", str(path), "cat-file", "blob", blob], capture_output=True, check=True).stdout
        except (OSError, subprocess.CalledProcessError) as error:
            findings.append("%s: blob %s unavailable (%s)" % (rel, blob, error))
            continue
        if hashlib.sha256(data).hexdigest() != digest:
            findings.append("%s: sha256 differs from the manifest" % rel)
        lines[(which, rel)] = len(data.decode("utf-8").split("\n"))
    for which, rel, first, last in evidence_refs():
        if (which, rel) not in lines:
            findings.append("%s:%s cited as evidence but it is not a verified source" % (which, rel))
        elif first < 1 or last < first or last > lines[(which, rel)]:
            findings.append("%s:%s:%d-%d is outside the file (%d lines)" % (which, rel, first, last, lines[(which, rel)]))
    return findings


def main(argv=None):
    parser = argparse.ArgumentParser(description="Reference of the P5 shot durations and cuts: writes or checks the corpus.")
    parser.add_argument("--check", action="store_true", help="regenerate in memory and compare byte for byte; write nothing")
    parser.add_argument("--verify-origin", metavar="PATH", help="re-read the origin blobs by git id (read only) and compare the sha256")
    parser.add_argument("--out", metavar="DIR", help="fixture directory (default: docs/migration/fixtures/shot-trim)")
    args = parser.parse_args(argv)
    problem = interpreter_problem()
    if problem:
        sys.stderr.write("INTERPRETER_NOT_REAL: %s\n" % problem)
        return EXIT_BAD_INTERPRETER
    out_dir = Path(args.out) if args.out else FIXTURE_DIR
    if args.verify_origin:
        findings = verify_origin(args.verify_origin)
        for finding in findings:
            sys.stderr.write("ORIGIN_MISMATCH: %s\n" % finding)
        if findings:
            return EXIT_FINDING
        print("origin blobs verified (%d); the origin was NOT executed" % len(ORIGIN_SOURCES))
        return EXIT_OK
    texts = generate_texts()
    if args.check:
        findings = []
        for name, text in texts.items():
            target = out_dir / name
            if not target.exists():
                findings.append("%s is missing" % name)
            elif target.read_bytes() != text.encode("utf-8"):
                findings.append("%s differs from what the reference generates" % name)
        for finding in findings:
            sys.stderr.write("CHECK_FAILED: %s\n" % finding)
        if findings:
            return EXIT_FINDING
        print("check ok: %d cases, %d files reproduced byte for byte, nothing written" % (len(json.loads(texts[CASES_NAME])["cases"]), len(texts)))
        return EXIT_OK
    out_dir.mkdir(parents=True, exist_ok=True)
    for name, text in texts.items():
        with open(str(out_dir / name), "w", encoding="utf-8", newline="\n") as handle:
            handle.write(text)
    print("wrote %d cases to %s" % (len(json.loads(texts[CASES_NAME])["cases"]), out_dir))
    return EXIT_OK


if __name__ == "__main__":
    sys.exit(main())
