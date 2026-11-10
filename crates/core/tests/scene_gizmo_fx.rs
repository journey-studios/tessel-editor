//! F055 (G1) x F057 (P2): the rotation gizmo and the effects layer in ONE session, through the
//! JSON executor (in process: no window, no GPU, no Chrome, no pointer device).
//!
//! The two lines of work landed on `main` separately; each is proved on its own
//! (`scene_gizmo_rotation.rs`, `scene_fx_gesture.rs`, `scene_fx.rs`). This file only pins what
//! happens where they meet, with expectations written here and not read from the code under test:
//!
//! - **One gesture slot.** An open effect gesture refuses a ring press and an open ring drag
//!   refuses an effect gesture, both typed `gesture_active`, and neither refusal moves the
//!   document, the history or the other gesture's candidate.
//! - **Tool switching is session state.** Choosing the rotate tool while an effect gesture is open
//!   is not an edit and does not touch the gesture.
//! - **Reads leave a drag alone.** The effect list, inspector, catalog and frame preview under an
//!   open ring drag write nothing and the drag survives.
//! - **One history.** A rotation and an effect edit are separate undo entries in one stack; undo
//!   and redo walk both without one touching the other's values. An effect edit under an open
//!   ring drag kills the drag (never confirmed late).
//! - **Not covered**: pixels, the window and a physical pointer (`runtime/test/`, the GPUI tests
//!   and `scripts/scene-gizmo-rotation-proof.py`).

use std::fs;
use std::path::Path;

use core::prelude::v1::test;
use serde_json::{Value, json};
use tessel_editor_core::{CommandError, EditorState, headless};

const FIGURE: &str = "scene/scene-a/node/figure";

fn run(state: &mut EditorState, request: Value) -> Result<Value, CommandError> {
    headless::execute(state, &request)
}

fn ok(state: &mut EditorState, request: Value) -> Value {
    run(state, request.clone()).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn err(state: &mut EditorState, request: Value) -> CommandError {
    match run(state, request.clone()) {
        Ok(answer) => panic!("{request} should have failed, got {answer}"),
        Err(error) => error,
    }
}

/// The scene `street` with one vignette `v1` (frames 10..40, intensity 0.5), the figure selected
/// and the rotate tool in world space.
fn session() -> EditorState {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json");
    let bundle: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let mut state = EditorState::demo();
    ok(&mut state, json!({"op":"project.create","name":"Meeting"}));
    ok(
        &mut state,
        json!({"op":"scene.attach","id":"street","bundle":bundle}),
    );
    ok(
        &mut state,
        json!({"op":"scene.fx.edit","scene":"street","ops":[
            {"op":"add_clip","effect":"vignette","start_frame":10,"duration_frames":30,
             "id":"v1","params":{"intensity":0.5}}]}),
    );
    ok(
        &mut state,
        json!({"op":"scene.select","scene":"street","entities":[FIGURE]}),
    );
    ok(
        &mut state,
        json!({"op":"scene.gizmo.tool","tool":"rotate","space":"world"}),
    );
    state
}

fn undo_depth(state: &EditorState) -> u64 {
    state.history.to_json()["undo_depth"].as_u64().unwrap()
}

/// What a refused or read-only call must leave as it was.
fn observable(state: &EditorState) -> (Value, Value, u64) {
    (
        state.to_document(None),
        state.history.to_json(),
        state.project.revision,
    )
}

fn figure_rotation(state: &EditorState) -> Value {
    let document = state.scenes.get("street").unwrap().scene.document().clone();
    document["sceneDefinitions"][0]["nodes"]
        .as_array()
        .and_then(|nodes| nodes.iter().find(|node| node["id"] == "figure"))
        .map(|node| node["localRotation"].clone())
        .unwrap_or_else(|| panic!("no figure node in {document}"))
}

fn clips(state: &mut EditorState) -> usize {
    let answer = ok(state, json!({"op":"scene.fx.list","scene":"street"}));
    answer["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|track| track["clips"].as_array().unwrap().len())
        .sum()
}

fn intensity(state: &mut EditorState) -> f64 {
    let answer = ok(state, json!({"op":"scene.fx.list","scene":"street"}));
    answer["tracks"][0]["clips"][0]["params"]["intensity"]
        .as_f64()
        .unwrap()
}

fn ring_point(state: &mut EditorState) -> Value {
    let shown = ok(
        state,
        json!({"op":"scene.gizmo.state","scene":"street","frame":0}),
    )["overlay"]
        .clone();
    assert_eq!(shown["visible"], true, "{shown}");
    let ring = shown["handles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|handle| handle["id"] == "z")
        .unwrap_or_else(|| panic!("no z ring in {shown}"));
    let points = ring["points"].as_array().unwrap();
    points[points.len() / 8].clone()
}

fn press(state: &mut EditorState, at: Value) -> Result<Value, CommandError> {
    run(
        state,
        json!({"op":"scene.pointer.down","scene":"street","frame":0,"point":at}),
    )
}

fn begin_fx(state: &mut EditorState) -> Result<Value, CommandError> {
    run(
        state,
        json!({"op":"scene.fx.gesture.begin","scene":"street","clip":"v1"}),
    )
}

#[test]
fn an_open_effect_gesture_refuses_a_ring_press_and_the_tool_switch_does_not_touch_it() {
    let mut state = session();
    ok(
        &mut state,
        json!({"op":"scene.gizmo.tool","tool":"translate","space":"world"}),
    );
    let gesture = begin_fx(&mut state).unwrap()["gesture"]
        .as_str()
        .unwrap()
        .to_string();
    ok(
        &mut state,
        json!({"op":"scene.fx.gesture.update","gesture":gesture,
               "ops":[{"op":"set_param","clip":"v1","param":"intensity","value":0.8}]}),
    );
    let before = observable(&state);

    // Choosing a tool is session state: allowed under an effect gesture, not an edit.
    let switched = ok(
        &mut state,
        json!({"op":"scene.gizmo.tool","tool":"rotate","space":"local"}),
    );
    assert_eq!(switched["changed"], true);
    assert_eq!(switched["document_changed"], false);
    assert_eq!(observable(&state), before);

    // The rings are drawn, but a press cannot open a second gesture.
    let at = ring_point(&mut state);
    assert_eq!(
        press(&mut state, at.clone()).unwrap_err().code,
        "gesture_active"
    );
    assert_eq!(observable(&state), before);
    let status = ok(&mut state, json!({"op":"scene.fx.gesture.status"}));
    assert_eq!(status["active"], true);
    assert!(status["gesture"]["refusal"].is_null(), "{status}");
    let candidate = ok(
        &mut state,
        json!({"op":"scene.fx.list","scene":"street","gesture":gesture}),
    );
    assert_eq!(candidate["source"], "gesture");
    assert_eq!(
        candidate["tracks"][0]["clips"][0]["params"]["intensity"],
        0.8
    );
    assert_eq!(intensity(&mut state), 0.5, "the document is the base");

    // After the cancel the same press opens a ring drag.
    ok(
        &mut state,
        json!({"op":"scene.gesture.cancel","gesture":gesture}),
    );
    assert_eq!(observable(&state), before);
    ok(
        &mut state,
        json!({"op":"scene.gizmo.tool","tool":"rotate","space":"world"}),
    );
    let at = ring_point(&mut state);
    assert_eq!(press(&mut state, at).unwrap()["dragging"], true);
    ok(&mut state, json!({"op":"scene.pointer.cancel"}));
    assert_eq!(observable(&state), before);
}

#[test]
fn an_open_ring_drag_refuses_an_effect_gesture_and_effect_reads_leave_it_alone() {
    let mut state = session();
    let baseline = undo_depth(&state);
    let before = observable(&state);
    let at = ring_point(&mut state);
    assert_eq!(press(&mut state, at).unwrap()["dragging"], true);
    assert_eq!(observable(&state), before, "a press writes nothing");

    assert_eq!(begin_fx(&mut state).unwrap_err().code, "gesture_active");
    assert_eq!(observable(&state), before);
    assert!(state.scene_interaction.drag().is_some());

    // The effects layer's reads: list, inspector, catalog and a frame preview.
    for request in [
        json!({"op":"scene.fx.list","scene":"street"}),
        json!({"op":"scene.fx.inspect","scene":"street"}),
        json!({"op":"scene.fx.inspect","scene":"street","clip":"v1"}),
        json!({"op":"scene.fx.catalog"}),
        json!({"op":"scene.fx.frame","scene":"street","frame":20}),
    ] {
        ok(&mut state, request);
        assert_eq!(observable(&state), before);
        assert!(
            state.scene_interaction.drag().is_some() && !state.gizmo_drag_is_dead(),
            "a read must not end the drag"
        );
    }

    ok(&mut state, json!({"op":"scene.pointer.cancel"}));
    assert_eq!(observable(&state), before);
    assert_eq!(
        undo_depth(&state),
        baseline,
        "the initial history is unchanged"
    );
}

#[test]
fn a_rotation_and_an_effect_edit_are_two_entries_of_one_history_that_never_touch_each_other() {
    let mut state = session();
    let baseline = undo_depth(&state);
    let authored = figure_rotation(&state);
    assert_eq!(undo_depth(&state), baseline);

    ok(
        &mut state,
        json!({"op":"scene.rotate","scene":"street","entity":FIGURE,"frame":0,
               "axis":"z","degrees":30.0,"space":"world"}),
    );
    let rotated = figure_rotation(&state);
    assert_ne!(rotated, authored);
    ok(
        &mut state,
        json!({"op":"scene.fx.edit","scene":"street","ops":[
            {"op":"set_param","clip":"v1","param":"intensity","value":0.9}]}),
    );
    assert_eq!(undo_depth(&state), baseline + 2);
    assert_eq!(
        figure_rotation(&state),
        rotated,
        "an effect edit leaves the rotation"
    );

    ok(&mut state, json!({"op":"history.undo"}));
    assert_eq!(intensity(&mut state), 0.5);
    assert_eq!(
        figure_rotation(&state),
        rotated,
        "undoing the effect keeps the turn"
    );
    ok(&mut state, json!({"op":"history.undo"}));
    assert_eq!(figure_rotation(&state), authored);
    assert_eq!(
        clips(&mut state),
        1,
        "undoing the turn keeps the effect clip"
    );
    assert_eq!(intensity(&mut state), 0.5);

    ok(&mut state, json!({"op":"history.redo"}));
    assert_eq!(figure_rotation(&state), rotated);
    assert_eq!(intensity(&mut state), 0.5);
    ok(&mut state, json!({"op":"history.redo"}));
    assert_eq!(figure_rotation(&state), rotated);
    assert_eq!(intensity(&mut state), 0.9);
}

#[test]
fn an_effect_edit_under_an_open_ring_drag_ends_the_drag_and_the_late_release_writes_nothing() {
    let mut state = session();
    let baseline = undo_depth(&state);
    let authored = figure_rotation(&state);
    let at = ring_point(&mut state);
    assert_eq!(press(&mut state, at).unwrap()["dragging"], true);
    ok(
        &mut state,
        json!({"op":"scene.fx.edit","scene":"street","ops":[
            {"op":"set_param","clip":"v1","param":"intensity","value":0.7}]}),
    );
    let after_edit = observable(&state);
    let moved = err(
        &mut state,
        json!({"op":"scene.pointer.move","point":{"u":0.4,"v":0.4}}),
    );
    assert_eq!(moved.code, "gesture_invalidated");
    assert!(state.gizmo_drag_is_dead());
    let _ = run(
        &mut state,
        json!({"op":"scene.pointer.up","point":{"u":0.4,"v":0.4}}),
    );
    assert_eq!(figure_rotation(&state), authored);
    assert_eq!(observable(&state).0, after_edit.0);
    assert_eq!(
        undo_depth(&state),
        baseline + 1,
        "one effect edit beyond the initial history; no rotation entry"
    );
    assert_eq!(intensity(&mut state), 0.7);
}

#[test]
fn an_effect_read_names_an_effect_gesture_and_refuses_the_id_of_a_ring_drag() {
    // F2 of the PR 40 review: `scene.fx.list/inspect/frame {gesture}` documents "scene and kind";
    // the id of a ring drag (a `properties` gesture) must not be read as an effect candidate.
    let mut state = session();
    let before = observable(&state);
    let at = ring_point(&mut state);
    let pressed = press(&mut state, at).unwrap();
    assert_eq!(pressed["dragging"], true, "{pressed}");
    let drag = pressed["gesture"].as_str().unwrap().to_string();
    assert_eq!(observable(&state), before, "a press writes nothing");

    for request in [
        json!({"op":"scene.fx.list","scene":"street","gesture":drag}),
        json!({"op":"scene.fx.inspect","scene":"street","gesture":drag}),
        json!({"op":"scene.fx.frame","scene":"street","frame":20,"gesture":drag}),
    ] {
        let refused = err(&mut state, request.clone());
        assert_eq!(
            refused.code, "gesture_kind_mismatch",
            "{request}: {refused:?}"
        );
        assert_eq!(observable(&state), before, "{request}");
        assert!(
            state.scene_interaction.drag().is_some() && !state.gizmo_drag_is_dead(),
            "a refused read must not end the drag"
        );
    }
    // The document read (no id) still works under the same drag.
    assert_eq!(
        ok(&mut state, json!({"op":"scene.fx.list","scene":"street"}))["source"],
        "document"
    );
    ok(&mut state, json!({"op":"scene.pointer.cancel"}));
    assert_eq!(observable(&state), before);
}
