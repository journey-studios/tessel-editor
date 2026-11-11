//! F062 (BL082.3, P4a): the effect scope `shot` through the library, the plan and the JSON
//! executor `tessel-headless` and `ui op` share (no window, no GPU).
//!
//! The numbers are written by hand, never read back from the code under test: the shots of the
//! two corpora, the frame at each side of a boundary, the order the passes are composed in. What
//! this proves: the pilot's scope cases; the importer's two rules and the `details` they carry;
//! the atomic edit (`add_clip.scope`, `set_scope`: one undo entry, a same-scope no-op, back to the
//! output removes the key, a refusal leaves nothing); the composition in stages; the plan (a
//! `scope` per pass, `fxShotScope` only when a shot pass exists, a validation that refuses what
//! the frame cannot draw); the budget over both stages; persistence; the reads
//! (`list/frame/catalog/inspect`); and the headless process. The pixels are
//! `crates/render/tests/scene_fx_shot_scope.rs`; the real window is
//! `scripts/scene-fx-shot-scope-proof.py`.
//!
//! Provenance of the pilot cases (`ORIGIN_NOT_EXECUTED`: the pilot's code was read, never run;
//! commit `d40ac3f7c0` of `t3/scene-engine-pilot`, `packages/scene-engine/src/fx/`). The pilot's
//! test bundle (`fx-test-bundle.ts`, blob `ec766844…`) has shots `s1 [0,48)`, `s2 [48,96)`,
//! `s3 [96,144)`; the corpus `valid-minimal-continuous` has the first two as `shot-1` and `shot-2`.
//!
//! | # | pilot case                                                    | origin (blob)                      |
//! |---|---------------------------------------------------------------|------------------------------------|
//! | 1 | clip 0..24: `setScope` shot, back to output removes the key   | `fx-commands-params.spec.ts` `7094de47…` |
//! | 2 | clip at 40, `setScope` s1: `FX_SCOPE_MISMATCH`                | `fx-commands-params.spec.ts` `7094de47…` |
//! | 3 | `addClip` keeps a shot scope and a seed, drops an output scope| `fx-commands-clips.spec.ts` `0b6bb7ba…` |
//! | 4 | a clip inside its shot and one that fills it are valid        | `fx-explain.spec.ts` `0838ac54…`   |
//! | 5 | a shot that does not exist: `UNKNOWN_SHOT`, hint lists shots  | `fx-explain.spec.ts` `0838ac54…`   |
//! | 6 | clip at 40, scope s1: `FX_SCOPE_MISMATCH` on import           | `fx-codes.spec.ts` `8813008a…`     |
//! | 7 | a node scope is still refused (`NODE_SCOPE_NOT_YET`)          | `fx-codes.spec.ts` `8813008a…`     |
//!
//! Divergences, on purpose: the pilot spells the code `NODE_SCOPE_NOT_YET`, here it is
//! `FX_SCOPE_NOT_YET`; the pilot's `addClip` takes a seed, here `set_seed` is its own op; the pilot
//! writes a shot as an object, the text spelling of an op is `shot:<id>` (the id is everything
//! after the first `:`, which the pilot's text parser cannot say).

use std::fs;
use std::io::Write;
use std::process::{Command as Process, Stdio};

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::directed_scene::fx::{self, FxScope};
use tessel_editor_core::scene_plan::{Capability, PlanTimeline, ScenePlan};
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::{CommandError, EditorState, headless, persist};

/// Two shots over 96 frames: `shot-1` is `[0, 48)` and `shot-2` is `[48, 96)`.
const MINIMAL: &str =
    include_str!("../../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json");
/// Two shots over 100 frames joined by a cut: `shot-a` is `[0, 40)` and `shot-b` is `[40, 100)`.
const CUT: &str =
    include_str!("../../../docs/migration/fixtures/bundles/valid-cut-instance-offsets.bundle.json");

fn bundle_of(source: &str) -> Value {
    serde_json::from_str(source).unwrap()
}

fn ok(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn err(state: &mut EditorState, request: Value) -> CommandError {
    match headless::execute(state, &request) {
        Ok(answer) => panic!("{request} should have failed, got {answer}"),
        Err(error) => error,
    }
}

fn session_from(document: Value) -> EditorState {
    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    ok(
        &mut state,
        json!({"op":"scene.attach","id":"street","bundle":document}),
    );
    state
}

fn session() -> EditorState {
    session_from(bundle_of(MINIMAL))
}

/// The first corpus with `layer` as its effects layer.
fn bundle_with(layer: Value) -> Value {
    let mut raw = bundle_of(MINIMAL);
    raw["fx"] = layer;
    raw
}

fn session_with_layer(layer: Value) -> EditorState {
    session_from(bundle_with(layer))
}

fn undo_depth(state: &EditorState) -> u64 {
    state.history.to_json()["undo_depth"].as_u64().unwrap()
}

fn document(state: &EditorState) -> Value {
    state.scenes.get("street").unwrap().scene.document().clone()
}

fn edit(state: &mut EditorState, ops: Value) -> Value {
    ok(
        state,
        json!({"op":"scene.fx.edit","scene":"street","ops":ops}),
    )
}

fn edit_err(state: &mut EditorState, ops: Value) -> CommandError {
    err(
        state,
        json!({"op":"scene.fx.edit","scene":"street","ops":ops}),
    )
}

fn add(id: &str, start: i64, duration: i64) -> Value {
    json!({"op":"add_clip","effect":"vignette","start_frame":start,"duration_frames":duration,"id":id})
}

fn add_scoped(id: &str, start: i64, duration: i64, scope: &str) -> Value {
    let mut op = add(id, start, duration);
    op["scope"] = json!(scope);
    op
}

fn set_scope(clip: &str, scope: &str) -> Value {
    json!({"op":"set_scope","clip":clip,"scope":scope})
}

/// A clip of the document by id, wherever it lies.
fn clip_of(document: &Value, id: &str) -> Value {
    document["fx"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|track| track["clips"].as_array().unwrap().iter())
        .find(|clip| clip["id"] == id)
        .unwrap_or_else(|| panic!("no clip {id} in {document}"))
        .clone()
}

fn frame(state: &mut EditorState, frame: i64) -> Value {
    ok(
        state,
        json!({"op":"scene.fx.frame","scene":"street","frame":frame}),
    )
}

fn passes(state: &mut EditorState, at: i64) -> Vec<Value> {
    frame(state, at)["passes"].as_array().unwrap().clone()
}

/// The clip ids of the passes at a frame, in the order they are composed.
fn order(state: &mut EditorState, at: i64) -> Vec<String> {
    passes(state, at)
        .iter()
        .map(|pass| pass["clipId"].as_str().unwrap().to_string())
        .collect()
}

fn plan_json(state: &mut EditorState, at: i64) -> Value {
    ok(
        state,
        json!({"op":"scene.plan","scene":"street","frame":at}),
    )["plan"]
        .clone()
}

fn scene_of(state: &EditorState) -> DirectedScene {
    DirectedScene::import_value(document(state)).unwrap()
}

fn issue_codes(error: &CommandError) -> Vec<String> {
    error
        .details
        .as_ref()
        .and_then(|details| details["issues"].as_array())
        .map(|issues| {
            issues
                .iter()
                .map(|issue| issue["code"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// The issues an import gives for `document`: `(code, path)` in order.
fn import_issues(document: Value) -> Vec<(String, String)> {
    match DirectedScene::import_value(document) {
        Ok(_) => Vec::new(),
        Err(error) => error
            .issues()
            .iter()
            .map(|issue| (issue.code.to_string(), issue.path.clone()))
            .collect(),
    }
}

fn layer_of(clips: Value) -> Value {
    json!({"catalogVersion":1,"tracks":[{"id":"t","label":"FX","clips":clips}],"transitions":[]})
}

/// Four tracks, written to be read: the output's clip `o1` is on the FIRST track, the clips of the
/// shots on the later ones. So a composition by track alone would draw `o1` first; by stages it is
/// drawn last. `a1` and `a2` fill their shots, `b1` is a grain inside `shot-1`.
fn layer_in_stages() -> Value {
    json!({
        "catalogVersion": 1,
        "tracks": [
            {"id": "t-out", "label": "Out", "clips": [
                {"id": "o1", "effect": "fade", "startFrame": 0, "durationFrames": 96}]},
            {"id": "t-a", "label": "A", "clips": [
                {"id": "a1", "effect": "vignette", "startFrame": 0, "durationFrames": 48,
                 "scope": {"kind": "shot", "shotId": "shot-1"}},
                {"id": "a2", "effect": "vignette", "startFrame": 48, "durationFrames": 48,
                 "scope": {"kind": "shot", "shotId": "shot-2"}}]},
            {"id": "t-b", "label": "B", "clips": [
                {"id": "b1", "effect": "filmGrain", "startFrame": 10, "durationFrames": 10,
                 "scope": {"kind": "shot", "shotId": "shot-1"}}]},
        ],
        "transitions": []
    })
}

// ---- the pilot's cases, by the library ---------------------------------------------------------

#[test]
fn pilot_case_1_set_scope_to_a_shot_and_back_to_the_output_removes_the_key() {
    let mut state = session();
    edit(&mut state, json!([add("fx-01", 0, 24)]));
    let plain = document(&state);
    assert!(
        clip_of(&plain, "fx-01").get("scope").is_none(),
        "the output is the absence of the key"
    );
    let depth = undo_depth(&state);

    let to_shot = edit(&mut state, json!([set_scope("fx-01", "shot:shot-1")]));
    assert_eq!(to_shot["changed"], true);
    assert_eq!(to_shot["changes"][0]["action"], "set_scope");
    assert_eq!(to_shot["changes"][0]["before"], "output");
    assert_eq!(to_shot["changes"][0]["after"], "shot:shot-1");
    assert_eq!(
        clip_of(&document(&state), "fx-01")["scope"],
        json!({"kind":"shot","shotId":"shot-1"})
    );
    assert_eq!(undo_depth(&state), depth + 1, "one undo entry");

    edit(&mut state, json!([set_scope("fx-01", "output")]));
    assert_eq!(
        document(&state),
        plain,
        "back to the output the document is what it was, key and all"
    );
    assert_eq!(undo_depth(&state), depth + 2);

    // Undo walks back one entry at a time, redo returns.
    ok(&mut state, json!({"op":"history.undo"}));
    assert_eq!(
        clip_of(&document(&state), "fx-01")["scope"],
        json!({"kind":"shot","shotId":"shot-1"})
    );
    ok(&mut state, json!({"op":"history.undo"}));
    assert_eq!(document(&state), plain);
    ok(&mut state, json!({"op":"history.redo"}));
    assert_eq!(
        clip_of(&document(&state), "fx-01")["scope"],
        json!({"kind":"shot","shotId":"shot-1"})
    );
}

#[test]
fn pilot_case_2_a_clip_that_leaves_the_shot_is_refused_with_the_legal_interval() {
    let mut state = session();
    // The pilot's `addVignette({ startFrame: 40 })`: 40 for 20 frames, which crosses frame 48.
    edit(&mut state, json!([add("fx-01", 40, 20)]));
    let before = document(&state);
    let depth = undo_depth(&state);

    let error = edit_err(&mut state, json!([set_scope("fx-01", "shot:shot-1")]));
    assert_eq!(error.code, "fx_scope_mismatch", "{error:?}");
    let details = error.details.as_ref().unwrap();
    assert_eq!(details["index"], 0);
    assert_eq!(details["path"], "scope");
    assert_eq!(details["shot"], "shot-1");
    assert_eq!(details["start_frame"], 0);
    assert_eq!(details["end_frame_exclusive"], 48);
    assert_eq!(details["clip"], "fx-01");
    assert_eq!(details["clip_start_frame"], 40);
    assert_eq!(details["clip_end_frame_exclusive"], 60);
    assert!(
        error.message.contains("between frames 0 and 48"),
        "{}",
        error.message
    );
    assert_eq!(document(&state), before, "a refusal writes nothing");
    assert_eq!(undo_depth(&state), depth);

    // The second shot is [48, 96): the same clip does not fit there either, one frame short.
    let error = edit_err(&mut state, json!([set_scope("fx-01", "shot:shot-2")]));
    assert_eq!(error.code, "fx_scope_mismatch", "{error:?}");
    assert_eq!(error.details.as_ref().unwrap()["start_frame"], 48);
    assert_eq!(error.details.as_ref().unwrap()["end_frame_exclusive"], 96);
    assert_eq!(document(&state), before);
}

#[test]
fn pilot_case_3_add_clip_keeps_a_shot_scope_and_a_seed_and_drops_an_output_scope() {
    let mut state = session();
    edit(
        &mut state,
        json!([{"op":"add_clip","effect":"filmGrain","start_frame":0,"duration_frames":20,
                "id":"inside","scope":"shot:shot-1"},
               {"op":"set_seed","clip":"inside","seed":5}]),
    );
    let clip = clip_of(&document(&state), "inside");
    assert_eq!(clip["scope"], json!({"kind":"shot","shotId":"shot-1"}));
    assert_eq!(clip["seed"], 5);

    edit(&mut state, json!([add_scoped("plain", 60, 10, "output")]));
    assert!(
        clip_of(&document(&state), "plain").get("scope").is_none(),
        "an output scope is the default: it is not written"
    );
    // `scope` null or absent is the output as well.
    let mut null_scope = add("null-scope", 70, 5);
    null_scope["scope"] = Value::Null;
    edit(&mut state, json!([null_scope, add("absent", 80, 5)]));
    for id in ["null-scope", "absent"] {
        assert!(
            clip_of(&document(&state), id).get("scope").is_none(),
            "{id}"
        );
    }
    // The listing says it in text: `output` or `shot:<id>`.
    let listing = ok(&mut state, json!({"op":"scene.fx.list","scene":"street"}));
    let scopes: Vec<(String, String)> = listing["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|track| track["clips"].as_array().unwrap().iter())
        .map(|clip| {
            (
                clip["id"].as_str().unwrap().to_string(),
                clip["scope"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert!(scopes.contains(&("inside".to_string(), "shot:shot-1".to_string())));
    assert!(scopes.contains(&("plain".to_string(), "output".to_string())));
}

#[test]
fn pilot_cases_4_and_6_a_clip_inside_its_shot_is_valid_and_one_that_crosses_it_is_not() {
    let with_scope = |start: i64, duration: i64, shot: &str| -> Value {
        bundle_with(layer_of(
            json!([{"id":"c","effect":"vignette","startFrame":start,
            "durationFrames":duration,"scope":{"kind":"shot","shotId":shot}}]),
        ))
    };
    // Case 4: 0..48 fills `shot-1`; the pilot's "inside" clip too.
    assert!(import_issues(with_scope(0, 48, "shot-1")).is_empty());
    assert!(import_issues(with_scope(10, 20, "shot-1")).is_empty());
    // And the other shot: 48..96 fills it, exactly to its last frame.
    assert!(import_issues(with_scope(48, 48, "shot-2")).is_empty());
    assert!(import_issues(with_scope(95, 1, "shot-2")).is_empty());

    // Case 6: the pilot's clip at 40 for 24 frames, scope `s1`.
    let issues = import_issues(with_scope(40, 24, "shot-1"));
    assert_eq!(
        issues,
        [(
            "FX_SCOPE_MISMATCH".to_string(),
            "/fx/tracks/0/clips/0".to_string()
        )]
    );
    // One frame is enough, at either end of the shot.
    assert_eq!(
        import_issues(with_scope(0, 49, "shot-1"))[0].0,
        "FX_SCOPE_MISMATCH",
        "one frame past the end"
    );
    assert_eq!(
        import_issues(with_scope(47, 2, "shot-2"))[0].0,
        "FX_SCOPE_MISMATCH",
        "frame 47 is not in shot-2, which starts at 48"
    );
    assert_eq!(
        import_issues(with_scope(47, 1, "shot-2"))[0].0,
        "FX_SCOPE_MISMATCH"
    );
    assert!(import_issues(with_scope(47, 1, "shot-1")).is_empty());
}

#[test]
fn pilot_case_5_a_shot_that_does_not_exist_is_unknown_and_the_hint_lists_the_shots() {
    let raw = bundle_with(layer_of(
        json!([{"id":"a","effect":"vignette","startFrame":0,
        "durationFrames":10,"scope":{"kind":"shot","shotId":"ghost"}}]),
    ));
    let error = DirectedScene::import_value(raw).unwrap_err();
    let issue = &error.issues()[0];
    assert_eq!(issue.code, "UNKNOWN_SHOT");
    assert_eq!(issue.path, "/fx/tracks/0/clips/0/scope/shotId");
    // The pilot's hint is `Shots: s1, s2, s3.`; this edit has two shots.
    assert!(
        issue.message.contains("Shots: shot-1, shot-2."),
        "{}",
        issue.message
    );
    assert_eq!(
        issue.details,
        Some(json!({"shot": "ghost", "shots": ["shot-1", "shot-2"]}))
    );

    // The same through the edit: the code is lower case and the op is named.
    let mut state = session();
    let before = document(&state);
    let error = edit_err(&mut state, json!([add_scoped("a", 0, 10, "shot:ghost")]));
    assert_eq!(error.code, "unknown_shot", "{error:?}");
    assert_eq!(error.target.as_deref(), Some("ghost"));
    assert_eq!(
        error.details.as_ref().unwrap()["shots"],
        json!(["shot-1", "shot-2"])
    );
    assert_eq!(document(&state), before);
}

#[test]
fn pilot_case_7_a_node_scope_is_still_refused_by_the_importer_and_by_the_edit() {
    let raw = bundle_with(layer_of(
        json!([{"id":"c","effect":"vignette","startFrame":0,
        "durationFrames":10,"scope":{"kind":"node","nodeId":"n1"}}]),
    ));
    assert_eq!(
        import_issues(raw),
        [(
            "FX_SCOPE_NOT_YET".to_string(),
            "/fx/tracks/0/clips/0/scope".to_string()
        )]
    );

    let mut state = session();
    edit(&mut state, json!([add("fx-01", 0, 10)]));
    let before = document(&state);
    for scope in ["node", "node:n1"] {
        let error = edit_err(&mut state, json!([set_scope("fx-01", scope)]));
        assert_eq!(error.code, "fx_scope_not_yet", "{scope}: {error:?}");
    }
    let error = edit_err(&mut state, json!([add_scoped("n", 20, 5, "node:n1")]));
    assert_eq!(error.code, "fx_scope_not_yet");
    assert_eq!(document(&state), before);
}

// ---- the shapes of a scope in a document and in an op -------------------------------------------

#[test]
fn a_hand_written_output_scope_is_accepted_and_kept_until_an_op_touches_the_clip() {
    let layer = layer_of(json!([{"id":"c","effect":"vignette","startFrame":0,
        "durationFrames":10,"scope":{"kind":"output"}}]));
    let mut state = session_with_layer(layer);
    let clip = clip_of(&document(&state), "c");
    assert_eq!(
        clip["scope"],
        json!({"kind":"output"}),
        "the importer keeps what the author wrote"
    );
    let depth = undo_depth(&state);

    // The same scope is a no-op: nothing changes, not even to a canonical spelling.
    let same = edit(&mut state, json!([set_scope("c", "output")]));
    assert_eq!(same["changed"], false);
    assert_eq!(same["noop"], true);
    assert_eq!(same["changes"][0]["noop"], true);
    assert_eq!(undo_depth(&state), depth);
    assert_eq!(
        clip_of(&document(&state), "c")["scope"],
        json!({"kind":"output"})
    );

    // An op on another field of the clip leaves the author's spelling alone.
    edit(
        &mut state,
        json!([{"op":"set_param","clip":"c","param":"intensity","value":0.4}]),
    );
    assert_eq!(
        clip_of(&document(&state), "c")["scope"],
        json!({"kind":"output"})
    );

    // A real change writes the canonical spelling: a shot as an object, the output as no key.
    edit(&mut state, json!([set_scope("c", "shot:shot-1")]));
    assert_eq!(
        clip_of(&document(&state), "c")["scope"],
        json!({"kind":"shot","shotId":"shot-1"})
    );
    edit(&mut state, json!([set_scope("c", "output")]));
    assert!(clip_of(&document(&state), "c").get("scope").is_none());
}

#[test]
fn the_same_scope_is_a_no_op_for_a_shot_too_and_a_dry_run_leaves_no_trace() {
    let mut state = session();
    edit(&mut state, json!([add_scoped("v1", 0, 20, "shot:shot-1")]));
    let before = document(&state);
    let depth = undo_depth(&state);

    let same = edit(&mut state, json!([set_scope("v1", "shot:shot-1")]));
    assert_eq!(same["changed"], false);
    assert_eq!(same["noop"], true);
    assert_eq!(undo_depth(&state), depth);
    assert_eq!(document(&state), before);

    let dry = ok(
        &mut state,
        json!({"op":"scene.fx.edit","scene":"street","dry_run":true,
               "ops":[set_scope("v1", "output")]}),
    );
    assert_eq!(dry["dry_run"], true);
    assert_eq!(dry["would_change"], true);
    assert_eq!(dry["changed"], false);
    assert_eq!(document(&state), before);
    assert_eq!(undo_depth(&state), depth);
    // The dry run refuses what the edit would refuse, with the same code.
    let refused = err(
        &mut state,
        json!({"op":"scene.fx.edit","scene":"street","dry_run":true,
               "ops":[set_scope("v1", "shot:shot-2")]}),
    );
    assert_eq!(refused.code, "fx_scope_mismatch");
}

#[test]
fn a_shot_id_is_everything_after_the_first_colon() {
    // A shot whose id has colons in it: the pilot's text parser cannot say it, the document can.
    let mut raw = bundle_of(MINIMAL);
    raw["shots"][0]["id"] = json!("act:1:shot");
    let mut state = session_from(raw);
    edit(
        &mut state,
        json!([add_scoped("v1", 0, 20, "shot:act:1:shot")]),
    );
    assert_eq!(
        clip_of(&document(&state), "v1")["scope"],
        json!({"kind":"shot","shotId":"act:1:shot"})
    );
    let listing = ok(&mut state, json!({"op":"scene.fx.list","scene":"street"}));
    assert_eq!(listing["tracks"][0]["clips"][0]["scope"], "shot:act:1:shot");
    // And back: what the listing says is what an op accepts.
    edit(&mut state, json!([set_scope("v1", "output")]));
    edit(&mut state, json!([set_scope("v1", "shot:act:1:shot")]));
}

#[test]
fn text_that_is_no_scope_is_an_invalid_value_and_a_missing_scope_an_invalid_op() {
    let mut state = session();
    edit(&mut state, json!([add("v1", 0, 10)]));
    let before = document(&state);
    for text in [
        "",
        "shot:",
        "Shot:shot-1",
        "frame",
        "shot",
        "output:x",
        "shot-1",
    ] {
        let error = edit_err(&mut state, json!([set_scope("v1", text)]));
        assert_eq!(error.code, "invalid_value", "{text:?}: {error:?}");
    }
    // The object spelling of an op is accepted: the pilot's, and the document's.
    edit(
        &mut state,
        json!([{"op":"set_scope","clip":"v1","scope":{"kind":"shot","shotId":"shot-1"}}]),
    );
    edit(
        &mut state,
        json!([{"op":"set_scope","clip":"v1","scope":{"kind":"output"}}]),
    );
    assert_eq!(document(&state), before);
    // `set_scope` needs a scope: absent is a malformed op, and null is not "the output" either.
    for ops in [
        json!([{"op":"set_scope","clip":"v1"}]),
        json!([{"op":"set_scope","scope":"output"}]),
    ] {
        let error = edit_err(&mut state, ops.clone());
        assert_eq!(error.code, "invalid_op", "{ops}: {error:?}");
    }
    let error = edit_err(
        &mut state,
        json!([{"op":"set_scope","clip":"v1","scope":null}]),
    );
    assert_eq!(error.code, "invalid_value", "{error:?}");
    assert_eq!(error.details.as_ref().unwrap()["path"], "scope");
    assert_eq!(document(&state), before);
}

#[test]
fn a_scope_that_is_not_well_formed_in_the_document_says_what_is_wrong() {
    let run = |scope: Value| -> Vec<(String, String)> {
        import_issues(bundle_with(layer_of(json!([{"id":"c","effect":"vignette",
            "startFrame":0,"durationFrames":10,"scope":scope}]))))
    };
    let at = "/fx/tracks/0/clips/0/scope";
    let issue = |code: &str, path: String| vec![(code.to_string(), path)];
    // Shape first, each problem with the pointer of the value that is wrong (a missing field is
    // told at the object that lacks it, as everywhere in the importer).
    assert_eq!(
        run(json!({"kind":"shot"})),
        issue("MISSING_FIELD", at.into())
    );
    assert_eq!(run(json!({})), issue("MISSING_FIELD", at.into()));
    assert_eq!(
        run(json!({"kind":"shot","shotId":""})),
        issue("INVALID_VALUE", format!("{at}/shotId"))
    );
    assert_eq!(
        run(json!({"kind":"shot","shotId":7})),
        issue("INVALID_TYPE", format!("{at}/shotId"))
    );
    assert_eq!(
        run(json!({"kind":"frame"})),
        issue("INVALID_VALUE", format!("{at}/kind"))
    );
    assert_eq!(
        run(json!({"kind":7})),
        issue("INVALID_TYPE", format!("{at}/kind"))
    );
    assert_eq!(run(json!("shot:shot-1")), issue("INVALID_TYPE", at.into()));
    assert_eq!(run(json!(null)), issue("INVALID_TYPE", at.into()));
    assert_eq!(
        run(json!({"kind":"node","nodeId":"n"})),
        issue("FX_SCOPE_NOT_YET", at.into())
    );
    // The two rules that need the shots only run on a clean bundle: a bad shape is told alone.
    let mut raw = bundle_with(layer_of(
        json!([{"id":"c","effect":"vignette","startFrame":0,
        "durationFrames":10,"scope":{"kind":"shot","shotId":"ghost"}},
        {"id":"d","effect":"nope","startFrame":20,"durationFrames":10}]),
    ));
    let issues: Vec<String> = import_issues(raw.clone())
        .into_iter()
        .map(|(code, _)| code)
        .collect();
    assert_eq!(
        issues,
        ["UNKNOWN_EFFECT"],
        "the unknown shot waits for a clean shape"
    );
    raw["fx"]["tracks"][0]["clips"][1]["effect"] = json!("fade");
    let issues: Vec<String> = import_issues(raw)
        .into_iter()
        .map(|(code, _)| code)
        .collect();
    assert_eq!(issues, ["UNKNOWN_SHOT"]);
}

// ---- the importer's two rules at the boundaries of both corpora -------------------------------------

#[test]
fn the_boundary_frames_of_the_first_corpus_belong_to_one_shot_each() {
    let mut state = session_with_layer(layer_of(json!([
        {"id":"first","effect":"vignette","startFrame":0,"durationFrames":48,
         "scope":{"kind":"shot","shotId":"shot-1"}},
        {"id":"second","effect":"vignette","startFrame":48,"durationFrames":48,
         "scope":{"kind":"shot","shotId":"shot-2"}}
    ])));
    // shot-1 is [0, 48): its clip draws at 0 and 47 and not at 48; shot-2 the other way round.
    assert_eq!(order(&mut state, 0), ["first"]);
    assert_eq!(order(&mut state, 47), ["first"]);
    assert_eq!(order(&mut state, 48), ["second"]);
    assert_eq!(order(&mut state, 95), ["second"]);
    let at_47 = frame(&mut state, 47);
    assert_eq!(at_47["shot"]["id"], "shot-1");
    assert_eq!(at_47["shot"]["start_frame"], 0);
    assert_eq!(at_47["shot"]["end_frame_exclusive"], 48);
    assert_eq!(frame(&mut state, 48)["shot"]["id"], "shot-2");
    assert_eq!(frame(&mut state, 48)["shot"]["start_frame"], 48);
}

#[test]
fn the_second_corpus_has_its_own_boundary_at_frame_40() {
    let mut state = session_from(bundle_of(CUT));
    edit(
        &mut state,
        json!([
            add_scoped("a", 0, 40, "shot:shot-a"),
            add_scoped("b", 40, 60, "shot:shot-b")
        ]),
    );
    assert_eq!(order(&mut state, 39), ["a"]);
    assert_eq!(order(&mut state, 40), ["b"]);
    assert_eq!(order(&mut state, 99), ["b"]);
    assert_eq!(frame(&mut state, 39)["shot"]["id"], "shot-a");
    assert_eq!(frame(&mut state, 40)["shot"]["id"], "shot-b");
    // A clip that crosses the cut fits in neither shot, whichever is chosen.
    edit(&mut state, json!([add("across", 39, 2)]));
    for shot in ["shot:shot-a", "shot:shot-b"] {
        let error = edit_err(&mut state, json!([set_scope("across", shot)]));
        assert_eq!(error.code, "fx_scope_mismatch", "{shot}");
    }
    // And the output scope does cross it, as before.
    assert_eq!(order(&mut state, 39), ["a", "across"].map(String::from));
    assert_eq!(order(&mut state, 40), ["b", "across"].map(String::from));
}

// ---- one atomic batch, retime first --------------------------------------------------------------

#[test]
fn a_batch_is_atomic_and_the_refused_op_is_named() {
    let mut state = session();
    let before = document(&state);
    let depth = undo_depth(&state);
    let error = edit_err(
        &mut state,
        json!([
            add("ok-first", 0, 10),
            add_scoped("late", 60, 10, "shot:ghost")
        ]),
    );
    assert_eq!(error.code, "unknown_shot");
    assert_eq!(error.details.as_ref().unwrap()["index"], 1);
    assert_eq!(document(&state), before, "the valid first op went with it");
    assert_eq!(undo_depth(&state), depth);

    // A scope that fails on the second of three ops: the third never runs.
    edit(&mut state, json!([add("v1", 40, 20)]));
    let before = document(&state);
    let error = edit_err(
        &mut state,
        json!([{"op":"set_param","clip":"v1","param":"intensity","value":0.2},
               set_scope("v1", "shot:shot-1"),
               {"op":"set_hidden","clip":"v1","hidden":true}]),
    );
    assert_eq!(error.code, "fx_scope_mismatch");
    assert_eq!(error.details.as_ref().unwrap()["index"], 1);
    assert_eq!(document(&state), before);
}

#[test]
fn retime_first_in_the_same_batch_when_both_are_meant() {
    let mut state = session();
    edit(&mut state, json!([add("v1", 40, 20)]));
    let depth = undo_depth(&state);
    // The clip is at [40, 60): it is not in shot-2 as it stands. Moved to [50, 70) first, it is.
    let moved = edit(
        &mut state,
        json!([{"op":"set_field","clip":"v1","field":"start_frame","value":50},
               set_scope("v1", "shot:shot-2")]),
    );
    assert_eq!(moved["changed"], true);
    assert_eq!(undo_depth(&state), depth + 1, "one batch, one entry");
    let clip = clip_of(&document(&state), "v1");
    assert_eq!(clip["startFrame"], 50);
    assert_eq!(clip["scope"], json!({"kind":"shot","shotId":"shot-2"}));
    // In the other order the scope is judged against the clip as it stands.
    let mut state = session();
    edit(&mut state, json!([add("v1", 40, 20)]));
    let error = edit_err(
        &mut state,
        json!([set_scope("v1", "shot:shot-2"),
               {"op":"set_field","clip":"v1","field":"start_frame","value":50}]),
    );
    assert_eq!(error.code, "fx_scope_mismatch");
    assert_eq!(error.details.as_ref().unwrap()["index"], 0);
}

#[test]
fn a_retime_that_pushes_a_shot_clip_out_of_its_shot_is_judged_by_the_importer() {
    let mut state = session();
    edit(&mut state, json!([add_scoped("v1", 10, 20, "shot:shot-1")]));
    let before = document(&state);
    let depth = undo_depth(&state);
    // 10 + 40 = 50 > 48: the clip would cross the cut. No clamping: a refusal that says which.
    let error = edit_err(
        &mut state,
        json!([{"op":"set_field","clip":"v1","field":"duration_frames","value":40}]),
    );
    assert_eq!(error.code, "invalid_bundle", "{error:?}");
    assert_eq!(issue_codes(&error), ["FX_SCOPE_MISMATCH"]);
    let issue = &error.details.as_ref().unwrap()["issues"][0];
    assert_eq!(issue["path"], "/fx/tracks/0/clips/0");
    assert_eq!(issue["ops"], json!([0]));
    assert_eq!(issue["details"]["shot"], "shot-1");
    assert_eq!(issue["details"]["end_frame_exclusive"], 48);
    assert_eq!(issue["details"]["clip_end_frame_exclusive"], 50);
    assert_eq!(document(&state), before);
    assert_eq!(undo_depth(&state), depth);
    // And the same retime on an output clip is fine: the scope is what constrains it.
    edit(&mut state, json!([set_scope("v1", "output")]));
    edit(
        &mut state,
        json!([{"op":"set_field","clip":"v1","field":"duration_frames","value":40}]),
    );
}

// ---- composition in stages ---------------------------------------------------------------------

#[test]
fn the_passes_of_a_shot_are_composed_before_the_output_whatever_the_track_order() {
    let mut state = session_with_layer(layer_in_stages());
    // Hand-written expectations: the order the passes are drawn in, frame by frame. `o1` is on the
    // first track and is drawn last because it is the output's.
    let expected: [(i64, &[&str]); 8] = [
        (0, &["a1", "o1"]),
        (9, &["a1", "o1"]),
        (10, &["a1", "b1", "o1"]),
        (19, &["a1", "b1", "o1"]),
        (20, &["a1", "o1"]),
        (47, &["a1", "o1"]),
        (48, &["a2", "o1"]),
        (95, &["a2", "o1"]),
    ];
    for (at, ids) in expected {
        assert_eq!(order(&mut state, at), ids, "frame {at}");
    }
    let answer = frame(&mut state, 10);
    assert_eq!(answer["stages"], json!({"deck": 2, "output": 1}));
    assert_eq!(answer["count"], 3);
    assert_eq!(
        answer["passes"][0]["scope"],
        json!({"kind":"shot","shotId":"shot-1"})
    );
    assert_eq!(answer["passes"][2]["scope"], json!({"kind":"output"}));
    assert_eq!(
        frame(&mut state, 60)["stages"],
        json!({"deck": 1, "output": 1})
    );
}

#[test]
fn composition_is_a_function_of_the_frame_alone() {
    let mut state = session_with_layer(layer_in_stages());
    let linear: Vec<Vec<String>> = (0..96).map(|at| order(&mut state, at)).collect();
    // Seek in any order, with repeats: the same answer at every frame.
    let mut seeked: Vec<(i64, Vec<String>)> = (0..96)
        .rev()
        .chain([10, 47, 48, 10])
        .map(|at| (at, order(&mut state, at)))
        .collect();
    seeked.sort_by_key(|(at, _)| *at);
    for (at, ids) in seeked {
        assert_eq!(ids, linear[usize::try_from(at).unwrap()], "frame {at}");
    }
}

#[test]
fn resolve_takes_the_shots_it_is_given_and_without_them_a_shot_clip_draws_nothing() {
    let state = session_with_layer(layer_in_stages());
    let scene = scene_of(&state);
    let bundle = scene.bundle();
    let ids = |passes: Vec<fx::FxPass>| -> Vec<String> {
        passes.into_iter().map(|pass| pass.clip_id).collect()
    };
    assert_eq!(
        ids(bundle.fx.resolve(10, &bundle.shots)),
        ["a1", "b1", "o1"]
    );
    // No shot to put the frame in: only the output draws (the function does not guess a shot).
    assert_eq!(ids(bundle.fx.resolve(10, &[])), ["o1"]);
    // The frame's passes of the bundle are exactly that call.
    assert_eq!(bundle.fx_passes(10), bundle.fx.resolve(10, &bundle.shots));
}

#[test]
fn hidden_clips_and_tracks_draw_nothing_in_either_stage() {
    let mut state = session_with_layer(layer_in_stages());
    edit(
        &mut state,
        json!([{"op":"set_hidden","clip":"b1","hidden":true}]),
    );
    assert_eq!(order(&mut state, 10), ["a1", "o1"], "a hidden shot clip");
    edit(
        &mut state,
        json!([{"op":"set_hidden","clip":"b1","hidden":false},
               {"op":"set_hidden","clip":"o1","hidden":true}]),
    );
    assert_eq!(order(&mut state, 10), ["a1", "b1"], "a hidden output clip");
    // A hidden track: the document's own flag, written by hand.
    let mut layer = layer_in_stages();
    layer["tracks"][1]["hidden"] = json!(true);
    let mut state = session_with_layer(layer);
    assert_eq!(order(&mut state, 10), ["b1", "o1"]);
    assert_eq!(order(&mut state, 60), ["o1"]);
}

// ---- the plan ------------------------------------------------------------------------------------------

#[test]
fn the_plan_lists_the_passes_by_stage_with_a_scope_each_and_asks_for_the_capability() {
    let mut state = session_with_layer(layer_in_stages());
    let plan = plan_json(&mut state, 10);
    let listed: Vec<(&str, Value)> = plan["fx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pass| (pass["clipId"].as_str().unwrap(), pass["scope"].clone()))
        .collect();
    assert_eq!(
        listed,
        [
            ("a1", json!({"kind":"shot","shotId":"shot-1"})),
            ("b1", json!({"kind":"shot","shotId":"shot-1"})),
            ("o1", json!({"kind":"output"})),
        ]
    );
    let requires = plan["requires"].as_array().unwrap();
    for capability in ["outputFx", "fxShotScope", "fxNoise"] {
        assert!(requires.contains(&json!(capability)), "{capability}");
    }
    // The very passes the executor reports for the frame.
    assert_eq!(
        plan["fx"].as_array().cloned().unwrap(),
        passes(&mut state, 10)
    );
    // The wire round-trips.
    let built = ScenePlan::build(&scene_of(&state), 10).unwrap();
    assert_eq!(ScenePlan::from_json(&built.to_json()).unwrap(), built);
}

#[test]
fn the_capability_is_asked_for_only_by_a_plan_with_a_pass_of_a_shot() {
    let mut state = session_with_layer(json!({
        "catalogVersion": 1,
        "tracks": [
            {"id": "t-out", "label": "Out", "clips": [
                {"id": "o1", "effect": "fade", "startFrame": 0, "durationFrames": 96}]},
            {"id": "t-s", "label": "S", "clips": [
                {"id": "s1", "effect": "vignette", "startFrame": 10, "durationFrames": 10,
                 "scope": {"kind": "shot", "shotId": "shot-1"}}]},
        ],
        "transitions": []
    }));
    let requires = |state: &mut EditorState, at: i64| -> Vec<Value> {
        plan_json(state, at)["requires"].as_array().unwrap().clone()
    };
    assert!(
        !requires(&mut state, 5).contains(&json!("fxShotScope")),
        "before the clip"
    );
    assert!(
        requires(&mut state, 5).contains(&json!("outputFx")),
        "the output draws"
    );
    assert!(
        requires(&mut state, 10).contains(&json!("fxShotScope")),
        "first frame"
    );
    assert!(
        requires(&mut state, 19).contains(&json!("fxShotScope")),
        "last frame"
    );
    assert!(
        !requires(&mut state, 20).contains(&json!("fxShotScope")),
        "the end is exclusive"
    );
    assert!(
        !requires(&mut state, 60).contains(&json!("fxShotScope")),
        "another shot"
    );

    // Output passes alone: scope `output` on the wire, no new key, no new capability.
    let mut output_only = session();
    edit(&mut output_only, json!([add("v1", 8, 24)]));
    let plan = plan_json(&mut output_only, 10);
    assert!(
        !plan["requires"]
            .as_array()
            .unwrap()
            .contains(&json!("fxShotScope"))
    );
    let mut keys: Vec<&str> = plan["fx"][0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "amount",
            "clipId",
            "effect",
            "frameInClip",
            "params",
            "scope",
            "trackId",
            "weight"
        ],
        "the pass of an output clip has the same keys as before F062"
    );
    assert_eq!(plan["fx"][0]["scope"], json!({"kind":"output"}));
}

#[test]
fn a_runtime_without_the_capability_refuses_the_shot_plan_by_name_and_serves_the_others() {
    assert_eq!(
        Capability::ALL.len(),
        13,
        "the 13th capability, `fxShotScope`"
    );
    assert_eq!(Capability::ALL[12], Capability::FxShotScope);
    assert_eq!(Capability::FxShotScope.as_str(), "fxShotScope");
    let state = session_with_layer(layer_in_stages());
    let scene = scene_of(&state);
    let without: Vec<Capability> = Capability::ALL
        .into_iter()
        .filter(|capability| *capability != Capability::FxShotScope)
        .collect();
    let in_shot = ScenePlan::build(&scene, 10).unwrap();
    assert_eq!(
        in_shot.missing_capabilities(&without),
        [Capability::FxShotScope]
    );
    assert!(in_shot.require_capabilities(&without).is_err());
    assert_eq!(in_shot.require_capabilities(&Capability::ALL), Ok(()));
    // A runtime that has the capability but not `outputFx` is told `outputFx` too.
    let neither: Vec<Capability> = without
        .iter()
        .copied()
        .filter(|capability| *capability != Capability::OutputFx)
        .collect();
    let missing = in_shot.missing_capabilities(&neither);
    assert!(missing.contains(&Capability::OutputFx));
    assert!(missing.contains(&Capability::FxShotScope));

    // A scene whose effects are all of the output is served by the runtime that predates F062.
    let mut plain = session();
    edit(&mut plain, json!([add("v1", 8, 24)]));
    let plan = ScenePlan::build(&scene_of(&plain), 10).unwrap();
    assert_eq!(plan.require_capabilities(&without), Ok(()));
}

#[test]
fn the_plan_refuses_what_the_frame_cannot_draw() {
    let mut state = session_with_layer(layer_in_stages());
    let good = plan_json(&mut state, 10);
    assert!(ScenePlan::from_json(&good).is_ok());

    // A pass of another shot than the frame's.
    let mut wire = good.clone();
    wire["fx"][0]["scope"]["shotId"] = json!("shot-2");
    let error = ScenePlan::from_json(&wire).unwrap_err().to_string();
    assert!(error.contains("/fx/0/scope/shotId"), "{error}");

    // The output's pass before a shot's: the stages are in order.
    let mut wire = good.clone();
    let listed = wire["fx"].as_array().unwrap().clone();
    wire["fx"] = json!([listed[2], listed[0], listed[1]]);
    let error = ScenePlan::from_json(&wire).unwrap_err().to_string();
    assert!(error.contains("/fx/1/scope"), "{error}");

    // `requires` that does not say `fxShotScope` while a pass is of a shot.
    let mut wire = good.clone();
    let requires: Vec<Value> = wire["requires"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|capability| *capability != "fxShotScope")
        .cloned()
        .collect();
    wire["requires"] = json!(requires);
    let error = ScenePlan::from_json(&wire).unwrap_err().to_string();
    assert!(error.contains("`/requires`"), "{error}");

    // The shapes of the scope.
    for (scope, at) in [
        (json!({"kind":"shot"}), "/fx/0/scope/shotId"),
        (json!({"kind":"shot","shotId":""}), "/fx/0/scope/shotId"),
        (json!({"kind":"node","nodeId":"n"}), "/fx/0/scope"),
        (json!({"kind":"frame"}), "/fx/0/scope"),
        (json!({"kind":"output","shotId":"shot-1"}), "/fx/0/scope"),
    ] {
        let mut wire = good.clone();
        wire["fx"][0]["scope"] = scope.clone();
        let error = ScenePlan::from_json(&wire).unwrap_err().to_string();
        assert!(error.contains(at), "{scope}: {error}");
    }
}

#[test]
fn the_timeline_gives_the_plan_of_a_build_at_every_frame_and_in_any_order() {
    let state = session_with_layer(layer_in_stages());
    let scene = scene_of(&state);
    let timeline = PlanTimeline::new(&scene);
    // Out of order on purpose: the template of a shot is made by whichever frame comes first, and
    // the boundary frames come at both ends.
    for at in (0..96).rev().chain([47, 48, 0, 47, 48]).chain(0..96) {
        assert_eq!(
            timeline.build(at),
            ScenePlan::build(&scene, at),
            "frame {at}"
        );
    }
    let last_of_first = timeline.build(47).unwrap();
    assert_eq!(last_of_first.frame.shot_id, "shot-1");
    assert_eq!(last_of_first.fx[0].clip_id, "a1");
    let first_of_second = timeline.build(48).unwrap();
    assert_eq!(first_of_second.frame.shot_id, "shot-2");
    assert_eq!(first_of_second.fx[0].clip_id, "a2");
}

// ---- the budget: both stages together --------------------------------------------------------------

#[test]
fn the_pass_budget_sums_the_shot_and_the_output() {
    let ops = |shot: usize, output: usize| -> Value {
        let mut ops = Vec::new();
        for index in 0..shot {
            ops.push(add_scoped(&format!("s{index}"), 10, 10, "shot:shot-1"));
        }
        for index in 0..output {
            ops.push(add(&format!("o{index}"), 10, 10));
        }
        Value::Array(ops)
    };
    // 4 + 4 = 8 passes: at the budget, no warning.
    let mut state = session();
    let answer = edit(&mut state, ops(4, 4));
    assert_eq!(answer["budget"]["peak_passes"], 8);
    assert_eq!(answer["budget"]["over_budget"], false);
    assert_eq!(answer["warnings"], json!([]));
    assert_eq!(frame(&mut state, 15)["count"], 8);

    // 5 + 4 = 9 passes: over it, though neither stage alone is.
    let mut state = session();
    let answer = edit(&mut state, ops(5, 4));
    assert_eq!(answer["budget"]["peak_passes"], 9);
    assert_eq!(answer["budget"]["over_budget"], true);
    assert_eq!(answer["budget"]["first_frame_over"], 10);
    assert_eq!(answer["budget"]["frames_over"], 10);
    assert_eq!(answer["warnings"][0]["code"], "FX_PASS_BUDGET");
    let at_15 = frame(&mut state, 15);
    assert_eq!(
        at_15["count"], 9,
        "the budget's peak is what the frame draws"
    );
    assert_eq!(at_15["stages"], json!({"deck": 5, "output": 4}));
    assert_eq!(at_15["over_budget"], true);
    assert_eq!(frame(&mut state, 9)["count"], 0);
    assert_eq!(frame(&mut state, 20)["count"], 0, "the end is exclusive");
    // A warning is not a refusal: the plan is still served, every pass in it.
    assert_eq!(plan_json(&mut state, 15)["fx"].as_array().unwrap().len(), 9);
}

// ---- reads ---------------------------------------------------------------------------------------------

#[test]
fn the_reads_say_the_scope_in_text_and_list_the_shots() {
    let mut state = session_with_layer(layer_in_stages());
    let catalog = ok(&mut state, json!({"op":"scene.fx.catalog"}));
    assert_eq!(catalog["scope"]["scope"], json!(["output", "shot"]));
    assert!(
        catalog["scope"]["scope_text"]["shot"]
            .as_str()
            .unwrap()
            .contains("shot:<id>")
    );
    assert_eq!(
        catalog["scope"]["shot_scope"]["requires"],
        "capability fxShotScope, only in a plan with a shot pass"
    );
    assert_eq!(
        catalog["scope"]["shot_scope"]["refusals"]["a shot the edit does not have"],
        "UNKNOWN_SHOT"
    );
    assert_eq!(
        catalog["scope"]["shot_scope"]["refusals"]["a clip that starts before the shot or ends after it"],
        "FX_SCOPE_MISMATCH"
    );
    assert!(
        catalog["scope"]["refused"]
            .as_object()
            .unwrap()
            .keys()
            .any(|key| key.contains("node")),
        "the node scope is refused, with its reason"
    );

    let listing = ok(&mut state, json!({"op":"scene.fx.list","scene":"street"}));
    assert_eq!(
        listing["shots"],
        json!([
            {"id":"shot-1","start_frame":0,"end_frame_exclusive":48,"intent":"synthetic shot shot-1"},
            {"id":"shot-2","start_frame":48,"end_frame_exclusive":96,"intent":"synthetic shot shot-2"}
        ])
    );
    let texts: Vec<(&str, &str)> = listing["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|track| track["clips"].as_array().unwrap().iter())
        .map(|clip| {
            (
                clip["id"].as_str().unwrap(),
                clip["scope"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        texts,
        [
            ("o1", "output"),
            ("a1", "shot:shot-1"),
            ("a2", "shot:shot-2"),
            ("b1", "shot:shot-1")
        ]
    );

    // The displayed frame's shot: its id, its interval and what it is for.
    let at_60 = frame(&mut state, 60);
    assert_eq!(
        at_60["shot"],
        json!({"id":"shot-2","start_frame":48,"end_frame_exclusive":96,"intent":"synthetic shot shot-2"})
    );
}

#[test]
fn the_inspector_offers_the_scope_as_a_choice_in_three_languages_with_the_op_that_chooses_it() {
    let mut state = session();
    edit(&mut state, json!([add("v1", 10, 20)]));
    let inspect = |state: &mut EditorState, locale: &str| -> Value {
        ok(
            state,
            json!({"op":"scene.fx.inspect","scene":"street","clip":"v1","frame":15,"locale":locale}),
        )
    };
    let answer = inspect(&mut state, "en");
    assert_eq!(answer["shot"]["id"], "shot-1", "the displayed frame's shot");
    assert_eq!(answer["shots"].as_array().unwrap().len(), 2);
    let scopes: Vec<(&str, bool)> = answer["scopes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|scope| {
            (
                scope["kind"].as_str().unwrap(),
                scope["applicable"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        scopes,
        [("output", true), ("shot", true), ("node", false)],
        "the node scope is listed and refused"
    );
    let node = &answer["scopes"][2];
    assert_eq!(node["reason"], "FX_SCOPE_NOT_YET");

    let choice = &answer["clips"][0]["choices"][0];
    assert_eq!(choice["id"], "scope");
    assert_eq!(choice["type"], "choice");
    assert_eq!(choice["label"], "Scope");
    assert_eq!(
        choice["labels"],
        json!({"pt-br":"Escopo","en":"Scope","es":"Alcance"})
    );
    assert_eq!(choice["value"], "output");
    assert_eq!(choice["default"], "output");
    assert_eq!(choice["at_default"], true);
    let options = choice["options"].as_array().unwrap();
    assert_eq!(
        options
            .iter()
            .map(|option| option["value"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["output", "shot:shot-1", "shot:shot-2"]
    );
    assert_eq!(options[0]["selected"], true);
    assert_eq!(options[0]["op"], Value::Null, "the one it has needs no op");
    // The clip is [10, 30): it fits `shot-1`, not `shot-2`, and the choice says so with a code.
    assert_eq!(options[1]["applicable"], true);
    assert_eq!(
        options[1]["labels"],
        json!({"pt-br":"Plano shot-1","en":"Shot shot-1","es":"Plano shot-1"})
    );
    assert_eq!(options[1]["intent"], "synthetic shot shot-1");
    assert_eq!(options[1]["start_frame"], 0);
    assert_eq!(options[1]["end_frame_exclusive"], 48);
    assert_eq!(options[2]["applicable"], false);
    assert_eq!(options[2]["reason"], "FX_SCOPE_MISMATCH");
    assert_eq!(
        options[2]["op"],
        Value::Null,
        "a shot that cannot hold it offers no op"
    );

    // The op the inspector hands out is the op that works.
    let chosen = options[1]["op"].clone();
    assert_eq!(chosen, set_scope("v1", "shot:shot-1"));
    edit(&mut state, json!([chosen]));
    let after = inspect(&mut state, "pt-br");
    let choice = &after["clips"][0]["choices"][0];
    assert_eq!(choice["label"], "Escopo");
    assert_eq!(choice["value"], "shot:shot-1");
    assert_eq!(choice["at_default"], false);
    assert_eq!(choice["options"][1]["selected"], true);
    assert_eq!(choice["options"][1]["label"], "Plano shot-1");
    // And now going back is offered, as the op that works.
    assert_eq!(choice["options"][0]["op"], set_scope("v1", "output"));
    assert_eq!(
        inspect(&mut state, "es")["clips"][0]["choices"][0]["label"],
        "Alcance"
    );
}

// ---- persistence ---------------------------------------------------------------------------------------

#[test]
fn a_shot_scope_survives_save_and_reopen_and_evaluates_the_same() {
    let mut state = session_with_layer(layer_in_stages());
    let scratch = std::env::temp_dir().join(format!("tessel-fx-shot-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).unwrap();
    let path = scratch.join("fx-shot.tessel.json");
    persist::save(&mut state, &path).unwrap();

    let mut reopened = EditorState::empty(&ProjectSettings::default()).unwrap();
    ok(&mut reopened, json!({"op":"project.open","path":path}));
    assert_eq!(document(&reopened), document(&state));
    for at in [0, 9, 10, 19, 20, 47, 48, 60, 95] {
        assert_eq!(
            order(&mut reopened, at),
            order(&mut state, at),
            "frame {at}"
        );
        assert_eq!(
            passes(&mut reopened, at),
            passes(&mut state, at),
            "frame {at}"
        );
    }
    // The reopened project edits and undoes its own scope changes from there.
    let depth = undo_depth(&reopened);
    edit(&mut reopened, json!([set_scope("b1", "output")]));
    assert_eq!(
        order(&mut reopened, 10),
        ["a1", "o1", "b1"].map(String::from)
    );
    assert_eq!(undo_depth(&reopened), depth + 1);
    ok(&mut reopened, json!({"op":"history.undo"}));
    assert_eq!(order(&mut reopened, 10), ["a1", "b1", "o1"]);
    fs::remove_dir_all(&scratch).unwrap();
}

// ---- the headless process ---------------------------------------------------------------------------------

fn edit_request(ops: Value) -> String {
    json!({"op":"scene.fx.edit","scene":"street","ops":ops}).to_string()
}

#[test]
fn the_headless_executable_scopes_a_clip_like_the_library_and_refuses_the_same_way() {
    let requests = [
        json!({"op":"scene.attach","id":"street","bundle":bundle_of(MINIMAL)}).to_string(),
        edit_request(json!([add("v1", 10, 20), add("o1", 0, 96)])),
        edit_request(json!([set_scope("v1", "shot:shot-1")])),
        json!({"op":"scene.fx.frame","scene":"street","frame":15}).to_string(),
        json!({"op":"scene.fx.frame","scene":"street","frame":60}).to_string(),
        json!({"op":"scene.plan","scene":"street","frame":15}).to_string(),
        edit_request(json!([set_scope("v1", "shot:shot-2")])),
        edit_request(json!([set_scope("v1", "shot:ghost")])),
        edit_request(json!([set_scope("v1", "node:n1")])),
        edit_request(json!([set_scope("v1", "output")])),
        json!({"op":"scene.fx.frame","scene":"street","frame":15}).to_string(),
        json!({"op":"history.undo"}).to_string(),
        json!({"op":"scene.fx.list","scene":"street"}).to_string(),
    ];
    let input = requests.join("\n");
    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    let mut library = Vec::new();
    headless::run_session(&mut state, None, input.as_bytes(), &mut library).unwrap();

    let mut child = Process::new(env!("CARGO_BIN_EXE_tessel-headless"))
        .arg("--empty")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    // A session with a refused request exits 1 (the executable's contract), after answering all.
    assert_eq!(output.status.code(), Some(1));
    let lines = |bytes: &[u8]| -> Vec<Value> {
        String::from_utf8_lossy(bytes)
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    };
    let answers = lines(&output.stdout);
    assert_eq!(
        answers,
        lines(&library),
        "the process answers like the library"
    );

    assert_eq!(answers[2]["result"]["changes"][0]["action"], "set_scope");
    assert_eq!(answers[3]["result"]["shot"]["id"], "shot-1");
    assert_eq!(
        answers[3]["result"]["passes"][0]["scope"],
        json!({"kind":"shot","shotId":"shot-1"}),
        "frame 15: the shot's pass first"
    );
    assert_eq!(answers[3]["result"]["passes"][0]["clipId"], "v1");
    assert_eq!(answers[3]["result"]["passes"][1]["clipId"], "o1");
    assert_eq!(answers[4]["result"]["shot"]["id"], "shot-2");
    assert_eq!(
        answers[4]["result"]["passes"].as_array().unwrap().len(),
        1,
        "frame 60 is in the other shot: only the output draws"
    );
    assert!(
        answers[5]["result"]["plan"]["requires"]
            .as_array()
            .unwrap()
            .contains(&json!("fxShotScope"))
    );
    assert_eq!(answers[6]["ok"], false, "{:?}", answers[6]);
    assert_eq!(answers[6]["error"]["code"], "fx_scope_mismatch");
    assert_eq!(answers[6]["error"]["details"]["shot"], "shot-2");
    assert_eq!(answers[7]["error"]["code"], "unknown_shot");
    assert_eq!(answers[8]["error"]["code"], "fx_scope_not_yet");
    assert_eq!(answers[9]["result"]["changed"], true);
    assert_eq!(
        answers[10]["result"]["passes"][0]["scope"],
        json!({"kind":"output"}),
        "back to the output"
    );
    assert_eq!(answers[11]["ok"], true);
    assert_eq!(
        answers[12]["result"]["tracks"][0]["clips"][0]["scope"],
        "shot:shot-1"
    );
}

#[test]
fn scope_is_part_of_the_shape_the_edit_vocabulary_documents() {
    // A request with an unknown op names the shapes: `set_scope` and `scope` are among them.
    let mut state = session();
    let error = edit_err(&mut state, json!([{"op":"explode"}]));
    let text = format!("{} {:?}", error.message, error.details);
    assert!(text.contains("set_scope"), "{text}");
    assert!(text.contains("shot:<id>"), "{text}");
    // The scope of the model is one value: its text round-trips for the output and for a shot.
    assert_eq!(FxScope::parse_text("output"), Ok(FxScope::Output));
    assert_eq!(
        FxScope::parse_text("shot:a:b"),
        Ok(FxScope::Shot("a:b".to_string()))
    );
    assert_eq!(FxScope::Shot("a:b".to_string()).to_text(), "shot:a:b");
}
