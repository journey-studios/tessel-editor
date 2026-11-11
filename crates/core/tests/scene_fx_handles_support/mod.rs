//! Shared by the F061 integration tests (`scene_fx_handles.rs`, `scene_fx_target.rs`): the scene
//! session with one vignette, and the one-line wrappers over the JSON executor. Expectations stay
//! in the tests; nothing here computes what the code under test should answer.

// Each test file uses a part of this module.
#![allow(dead_code)]

use std::collections::HashMap;

use serde_json::{Value, json};
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::state::layout::Layout;
use tessel_editor_core::state::tree::{TreeContext, nodes};
use tessel_editor_core::{CommandError, EditorState, headless};

pub const MINIMAL: &str = include_str!(
    "../../../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json"
);
pub const FIGURE: &str = "scene/scene-a/node/figure";
/// The frame every test looks at: inside the clip (10..40).
pub const FRAME: i64 = 20;

pub fn bundle(width: u64, height: u64) -> Value {
    let mut bundle: Value = serde_json::from_str(MINIMAL).unwrap();
    bundle["renderProfile"]["width"] = json!(width);
    bundle["renderProfile"]["height"] = json!(height);
    bundle
}

pub fn run(state: &mut EditorState, request: Value) -> Result<Value, CommandError> {
    headless::execute(state, &request)
}

pub fn ok(state: &mut EditorState, request: Value) -> Value {
    run(state, request.clone()).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

pub fn err(state: &mut EditorState, request: Value) -> CommandError {
    match run(state, request.clone()) {
        Ok(answer) => panic!("{request} should have failed, got {answer}"),
        Err(error) => error,
    }
}

/// A scene `street` of `width`×`height` with one vignette `v1` over frames 10..40.
pub fn session_with(width: u64, height: u64, roundness: f64) -> EditorState {
    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    ok(
        &mut state,
        json!({"op":"scene.attach","id":"street","bundle":bundle(width, height)}),
    );
    ok(
        &mut state,
        json!({"op":"scene.fx.edit","scene":"street","ops":[
            {"op":"add_clip","effect":"vignette","start_frame":10,"duration_frames":30,"id":"v1",
             "params":{"radius":0.4,"softness":0.2,"roundness":roundness,
                       "center":{"x":0.2,"y":0.5}}}]}),
    );
    state
}

pub fn session() -> EditorState {
    session_with(320, 180, 0.6)
}

pub fn selected(state: &mut EditorState) {
    ok(
        state,
        json!({"op":"scene.fx.select","scene":"street","clip":"v1"}),
    );
}

/// What `tree` publishes for the monitor's gizmo: the `target` (`node` or `fx`) and its `fx_clip`.
pub fn published_target(state: &EditorState) -> (Value, Value) {
    let (layout, metrics) = (
        Layout {
            regions: Vec::new(),
            area: (0.0, 0.0),
        },
        HashMap::new(),
    );
    let cx = TreeContext {
        theme: "dark",
        metrics: &metrics,
        layout: &layout,
        perf_meter: false,
    };
    let node = nodes(state, &cx)
        .into_iter()
        .find(|node| node["id"] == "monitor.gizmo")
        .expect("the gizmo node");
    (
        node["state"]["target"].clone(),
        node["state"]["fx_clip"].clone(),
    )
}

pub fn document(state: &EditorState) -> Value {
    state.scenes.get("street").unwrap().scene.document().clone()
}

pub fn undo_depth(state: &EditorState) -> u64 {
    state.history.to_json()["undo_depth"].as_u64().unwrap()
}

pub fn handles(state: &mut EditorState) -> Value {
    ok(
        state,
        json!({"op":"scene.fx.handles","scene":"street","clip":"v1","frame":FRAME}),
    )
}

pub fn handle_at(answer: &Value, id: &str) -> (f64, f64) {
    let handle = answer["handles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|handle| handle["id"] == id)
        .unwrap_or_else(|| panic!("no handle `{id}` in {answer}"));
    (
        handle["at"]["u"].as_f64().unwrap(),
        handle["at"]["v"].as_f64().unwrap(),
    )
}

pub fn disabled(answer: &Value, id: &str) -> Option<String> {
    answer["disabled_handles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|handle| handle["id"] == id)
        .map(|handle| handle["reason"].as_str().unwrap().to_string())
}

pub fn down(state: &mut EditorState, at: (f64, f64)) -> Value {
    ok(
        state,
        json!({"op":"scene.pointer.down","scene":"street","frame":FRAME,
               "point":{"u":at.0,"v":at.1}}),
    )
}

pub fn mv(state: &mut EditorState, at: (f64, f64)) -> Value {
    ok(
        state,
        json!({"op":"scene.pointer.move","point":{"u":at.0,"v":at.1}}),
    )
}

pub fn up(state: &mut EditorState, at: Option<(f64, f64)>) -> Value {
    match at {
        Some(at) => ok(
            state,
            json!({"op":"scene.pointer.up","point":{"u":at.0,"v":at.1}}),
        ),
        None => ok(state, json!({"op":"scene.pointer.up"})),
    }
}
