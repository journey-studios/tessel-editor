//! F039: selection, the entity tree, the evaluated frame and the gesture cycle of directed
//! scenes, through the public library API and the JSON executor (in process: no window, no GPU,
//! no Chrome, no GPUI).
//!
//! What this file proves, and what it does not:
//!
//! - **Consumer flow**: attach a bundle, read its tree, select entities by address, evaluate a
//!   frame, run a gesture (begin, update, preview, commit/cancel), undo it, and drive the same
//!   flow as a JSON Lines session. Every request goes through `headless::execute`, the executor
//!   `tessel-headless` and `ui op` share.
//! - **Errors and invariance**: a rejected, cancelled, stale or read-only command leaves the
//!   document, the history, the revision and the status exactly as they were.
//! - **Equivalence** only where a reference exists in the repository: `scene.evaluate` against
//!   `Bundle::evaluate_frame` (same function, same JSON), a committed gesture against the same
//!   batch sent as `scene.edit`.
//! - **Not covered**: the `tessel-headless` executable, the desktop socket and any visual
//!   surface; no physical pointer gesture exists here or is claimed.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::{DirectedScene, Vec3};
use tessel_editor_core::scene_edit::{EditOp, NodeProperty, PropertyRef, PropertyValue};
use tessel_editor_core::scene_interaction::GestureCommand;
use tessel_editor_core::scene_project::{SceneCommand, SceneSource};
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::{Command, CommandError, EditorState, headless};

const MINIMAL: &str = "valid-minimal-continuous";
const OFFSETS: &str = "valid-cut-instance-offsets";
const RIG: &str = "valid-graph-sockets-performance";

const FIGURE: &str = "scene/scene-a/node/figure";
const FIGURE_POSITION: &str = "scene/scene-a/node/figure/position";
const FOV: &str = "camera/cam-main/start/fov";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/migration/fixtures")
}

fn bundle(name: &str) -> Value {
    let path = fixtures()
        .join("bundles")
        .join(format!("{name}.bundle.json"));
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

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

fn new_project() -> EditorState {
    let mut state = EditorState::demo();
    ok(&mut state, json!({"op":"project.create","name":"Scenes"}));
    state
}

fn attach(state: &mut EditorState, id: &str, file: &str) {
    ok(
        state,
        json!({"op":"scene.attach","id":id,"bundle":bundle(file)}),
    );
}

fn street() -> EditorState {
    let mut state = new_project();
    attach(&mut state, "street", MINIMAL);
    state
}

/// Everything a rejected, read-only, cancelled or stale command must leave exactly as it was.
///
/// The summary leaves out its `scene_interaction` block (BL068): selecting or beginning a gesture
/// is *meant* to show up there, and `scene_session.rs` tests that block. Everything else in the
/// summary (the project, the playhead, the timeline) must stay.
fn observable(state: &EditorState) -> (Value, Value, u64, Value) {
    let mut summary = state.summary();
    summary.as_object_mut().unwrap().remove("scene_interaction");
    (
        state.to_document(None),
        state.history.to_json(),
        state.project.revision,
        summary,
    )
}

fn undo_depth(state: &EditorState) -> u64 {
    state.history.to_json()["undo_depth"].as_u64().unwrap()
}

fn document_of(state: &EditorState, id: &str) -> Value {
    state.scenes.get(id).unwrap().scene.document().clone()
}

fn move_figure(x: f64) -> Value {
    json!({"set": FIGURE_POSITION, "value": {"x": x, "y": 0, "z": 0}})
}

fn figure_x(state: &mut EditorState) -> f64 {
    ok(
        state,
        json!({"op":"scene.get","scene":"street","property":FIGURE_POSITION}),
    )["value"]["x"]
        .as_f64()
        .unwrap()
}

fn begin(state: &mut EditorState) -> String {
    let answer = ok(
        state,
        json!({"op":"scene.gesture.begin","scene":"street","targets":[FIGURE]}),
    );
    answer["gesture"].as_str().unwrap().to_string()
}

fn update(state: &mut EditorState, gesture: &str, ops: Vec<Value>) -> Value {
    ok(
        state,
        json!({"op":"scene.gesture.update","gesture":gesture,"ops":ops}),
    )
}

fn flatten(tree: &Value) -> Vec<&Value> {
    let mut out = Vec::new();
    let mut stack = vec![tree];
    while let Some(entry) = stack.pop() {
        out.push(entry);
        if let Some(children) = entry["children"].as_array() {
            stack.extend(children.iter());
        }
    }
    out
}

fn node_of<'a>(frame: &'a Value, id: &str) -> &'a Value {
    frame["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["nodeId"] == id)
        .unwrap_or_else(|| panic!("no node {id} in {frame}"))
}

// ---- tree and selection -------------------------------------------------------------------

#[test]
fn the_tree_lists_entities_by_address_and_the_catalog_keeps_its_first_eight() {
    let mut state = street();
    let before = observable(&state);

    let answer = ok(&mut state, json!({"op":"scene.tree","scene":"street"}));
    assert_eq!(answer["action"], "scene.tree");
    let tree = &answer["tree"];
    assert_eq!(tree["kind"], "bundle");
    let entries = flatten(tree);
    let paths: Vec<&str> = entries
        .iter()
        .map(|entry| entry["path"].as_str().unwrap())
        .collect();
    for expected in [
        "scene/scene-a",
        "scene/scene-a/node/root",
        "scene/scene-a/node/backdrop",
        FIGURE,
        "instance/inst-a",
        "camera/cam-main",
        "shot/shot-1",
        "shot/shot-2",
        "asset/figure",
    ] {
        assert!(
            paths.contains(&expected),
            "{expected} missing from {paths:?}"
        );
    }

    let figure = entries
        .iter()
        .find(|entry| entry["path"] == FIGURE)
        .unwrap();
    assert_eq!(figure["kind"], "node");
    assert_eq!(figure["node_kind"], "imagePlane");
    assert_eq!(figure["parent"], "scene/scene-a/node/root");
    assert_eq!(figure["selected"], false);
    let editable = figure["editable"].as_array().unwrap();
    assert!(editable.contains(&json!(FIGURE_POSITION)));
    assert!(editable.contains(&json!("scene/scene-a/node/figure/width")));
    let root = entries
        .iter()
        .find(|entry| entry["path"] == "scene/scene-a/node/root")
        .unwrap();
    assert!(
        !root["editable"]
            .as_array()
            .unwrap()
            .contains(&json!("scene/scene-a/node/root/width")),
        "a group has no width"
    );
    let clip = entries
        .iter()
        .find(|entry| entry["path"] == "camera/cam-main")
        .unwrap();
    assert!(clip["editable"].as_array().unwrap().contains(&json!(FOV)));

    // A subtree, and a depth that leaves children out says so.
    let sub = ok(
        &mut state,
        json!({"op":"scene.tree","scene":"street","root":"scene/scene-a/node/root","depth":1}),
    );
    assert_eq!(sub["tree"]["children_count"], 2);
    assert_eq!(sub["tree"]["children"].as_array().unwrap().len(), 2);
    let shallow = ok(
        &mut state,
        json!({"op":"scene.tree","scene":"street","root":"scene/scene-a","depth":1}),
    );
    assert_eq!(shallow["tree"]["children"][0]["truncated"], true);

    assert_eq!(
        err(
            &mut state,
            json!({"op":"scene.tree","scene":"street","root":"scene/scene-a/node/ghost"})
        )
        .code,
        "unknown_entity"
    );
    assert_eq!(
        err(
            &mut state,
            json!({"op":"scene.tree","scene":"street","root":"node/figure"})
        )
        .code,
        "invalid_entity_path"
    );
    assert_eq!(
        err(
            &mut state,
            json!({"op":"scene.tree","scene":"street","depth":0})
        )
        .code,
        "invalid_value"
    );
    assert_eq!(
        err(&mut state, json!({"op":"scene.tree","scene":"nope"})).code,
        "unknown_element"
    );
    assert_eq!(
        observable(&state),
        before,
        "reading never changes the project"
    );

    let catalog = ok(&mut state, json!({"op":"scene.catalog"}));
    // S6 appends `renderOrder` and the components; the original eight keep their place.
    assert_eq!(catalog["count"], 21);
    let names: Vec<&str> = catalog["properties"]
        .as_array()
        .unwrap()
        .iter()
        .take(8)
        .map(|property| property["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "position",
            "rotation",
            "scale",
            "width",
            "height",
            "start/position",
            "start/target",
            "start/fov"
        ]
    );
    // The tree's `editable` list names the component paths too.
    assert!(editable.contains(&json!("scene/scene-a/node/figure/position/x")));
    assert!(editable.contains(&json!("scene/scene-a/node/figure/renderOrder")));
}

#[test]
fn selection_is_by_address_validated_atomic_and_never_an_undo_entry() {
    let mut state = street();
    let before = observable(&state);

    let selected = ok(
        &mut state,
        json!({"op":"scene.select","scene":"street","entities":[FIGURE, "camera/cam-main", FIGURE]}),
    );
    assert_eq!(selected["changed"], true);
    assert_eq!(selected["document_changed"], false);
    assert_eq!(selected["count"], 2, "a repeated address collapses");
    assert_eq!(selected["entities"][0]["path"], FIGURE);
    assert_eq!(selected["entities"][0]["scene"], "street");
    assert_eq!(
        observable(&state),
        before,
        "selecting is presentation: no history, no revision, no document change"
    );

    // The tree marks what is selected.
    let tree = ok(&mut state, json!({"op":"scene.tree","scene":"street"}));
    let marked: Vec<&str> = flatten(&tree["tree"])
        .into_iter()
        .filter(|entry| entry["selected"] == true)
        .map(|entry| entry["path"].as_str().unwrap())
        .collect();
    assert_eq!(marked.len(), 2);
    assert!(marked.contains(&FIGURE) && marked.contains(&"camera/cam-main"));

    // Modes.
    ok(
        &mut state,
        json!({"op":"scene.select","scene":"street","mode":"toggle","entities":["camera/cam-main","shot/shot-1"]}),
    );
    let after_toggle = ok(&mut state, json!({"op":"scene.selection"}));
    let paths: Vec<&str> = after_toggle["entities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, [FIGURE, "shot/shot-1"]);
    ok(
        &mut state,
        json!({"op":"scene.select","scene":"street","mode":"remove","entities":["shot/shot-1","shot/gone"]}),
    );
    assert_eq!(ok(&mut state, json!({"op":"scene.selection"}))["count"], 1);

    // A bad entry rejects the whole request and keeps the selection.
    for request in [
        json!({"op":"scene.select","scene":"street","mode":"add","entities":["shot/shot-2", "scene/scene-a/node/ghost"]}),
        json!({"op":"scene.select","scene":"street","mode":"add","entities":["shot/shot-2", "not an address"]}),
        json!({"op":"scene.select","scene":"nope","entities":[FIGURE]}),
        json!({"op":"scene.select","mode":"add","entities":[FIGURE]}),
        json!({"op":"scene.select","scene":"street","mode":"clear","entities":[FIGURE]}),
        json!({"op":"scene.select","scene":"street","mode":"nonsense","entities":[]}),
    ] {
        let error = err(&mut state, request.clone());
        assert!(
            [
                "unknown_entity",
                "invalid_entity_path",
                "unknown_element",
                "invalid_request",
                "invalid_value"
            ]
            .contains(&error.code),
            "{request}: {}",
            error.code
        );
        assert_eq!(
            ok(&mut state, json!({"op":"scene.selection"}))["entities"][0]["path"],
            FIGURE,
            "{request} must keep the selection"
        );
    }
    assert_eq!(observable(&state), before);

    // Clear empties it.
    ok(&mut state, json!({"op":"scene.select","mode":"clear"}));
    assert_eq!(ok(&mut state, json!({"op":"scene.selection"}))["count"], 0);
}

#[test]
fn an_edit_needs_no_selection_and_leaves_the_selection_alone() {
    let mut state = street();
    assert_eq!(ok(&mut state, json!({"op":"scene.selection"}))["count"], 0);
    ok(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[move_figure(2.0)]}),
    );
    assert_eq!(
        figure_x(&mut state),
        2.0,
        "nothing was selected and the edit ran"
    );

    ok(
        &mut state,
        json!({"op":"scene.select","scene":"street","entities":["camera/cam-main"]}),
    );
    ok(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[move_figure(3.0)]}),
    );
    assert_eq!(figure_x(&mut state), 3.0, "the edit named its own target");
    let selection = ok(&mut state, json!({"op":"scene.selection"}));
    assert_eq!(selection["count"], 1);
    assert_eq!(selection["entities"][0]["path"], "camera/cam-main");
}

#[test]
fn the_selection_drops_what_disappears_and_says_so() {
    let mut state = new_project();
    attach(&mut state, "street", MINIMAL);
    attach(&mut state, "other", OFFSETS);
    ok(
        &mut state,
        json!({"op":"scene.select","scene":"street","entities":[FIGURE]}),
    );
    ok(
        &mut state,
        json!({"op":"scene.select","scene":"other","mode":"add","entities":["shot/shot-a"]}),
    );

    let detached = ok(&mut state, json!({"op":"scene.detach","scene":"other"}));
    let dropped = detached["selection_dropped"].as_array().unwrap();
    assert_eq!(dropped.len(), 1);
    assert_eq!(dropped[0]["path"], "shot/shot-a");
    assert_eq!(dropped[0]["scene"], "other");
    assert_eq!(dropped[0]["reason"], "scene_detached");

    // Undo brings the scene back, not the selection: selection is not part of the undo state.
    ok(&mut state, json!({"op":"history.undo"}));
    assert!(state.scenes.get("other").is_some());
    let selection = ok(&mut state, json!({"op":"scene.selection"}));
    assert_eq!(selection["count"], 1);
    assert_eq!(selection["entities"][0]["path"], FIGURE);
    assert_eq!(selection["dropped"][0]["reason"], "scene_detached");

    // An undo that removes the scene prunes the selection right then (BL069) and says so in its
    // own answer; reading the selection afterwards has nothing left to prune but still tells
    // the whole story.
    let first = ok(&mut state, json!({"op":"history.undo"}));
    assert!(
        first.get("selection_dropped").is_none(),
        "the figure is still there"
    );
    let undone = ok(&mut state, json!({"op":"history.undo"}));
    assert_eq!(undone["selection_dropped"][0]["path"], FIGURE);
    assert!(state.scenes.is_empty());
    let emptied = ok(&mut state, json!({"op":"scene.selection"}));
    assert_eq!(emptied["count"], 0);
    assert_eq!(emptied["changed"], false, "the undo already pruned it");
    assert_eq!(emptied["dropped"].as_array().unwrap().len(), 2);

    // Setting a selection again starts a clean report.
    ok(&mut state, json!({"op":"history.redo"}));
    ok(
        &mut state,
        json!({"op":"scene.select","scene":"street","entities":[FIGURE]}),
    );
    assert!(
        ok(&mut state, json!({"op":"scene.selection"}))["dropped"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn interaction_state_is_never_saved_nor_restored() {
    let mut state = street();
    let document = state.to_document(None);
    ok(
        &mut state,
        json!({"op":"scene.select","scene":"street","entities":[FIGURE]}),
    );
    let gesture = begin(&mut state);
    update(&mut state, &gesture, vec![move_figure(2.0)]);
    assert_eq!(
        state.to_document(None),
        document,
        "selection and an open gesture are not in the document"
    );

    let reopened = EditorState::from_document(&document, None).unwrap();
    assert!(reopened.scene_interaction.selection().is_empty());
    assert!(reopened.scene_interaction.gesture().is_none());
    assert!(state.scene_interaction.gesture().is_some());
}

// ---- evaluation ---------------------------------------------------------------------------

#[test]
fn evaluate_returns_camera_nodes_and_unevaluated_like_the_library() {
    for (file, frame) in [(MINIMAL, 10_i64), (RIG, 12), (OFFSETS, 55)] {
        let mut state = new_project();
        attach(&mut state, "scene", file);
        let before = observable(&state);
        let answer = ok(
            &mut state,
            json!({"op":"scene.evaluate","scene":"scene","frame":frame}),
        );
        assert_eq!(answer["action"], "scene.evaluate");
        assert_eq!(answer["source"], "document");

        let expected = DirectedScene::import_value(bundle(file))
            .unwrap()
            .bundle()
            .evaluate_frame(frame)
            .unwrap()
            .to_json();
        assert_eq!(answer["camera"], expected["camera"], "{file}");
        assert_eq!(answer["unevaluated"], expected["unevaluated"], "{file}");
        assert_eq!(answer["complete"], expected["complete"], "{file}");
        assert_eq!(answer["animation"], expected["animation"], "{file}");
        let nodes = answer["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), expected["nodes"].as_array().unwrap().len());
        for (node, want) in nodes.iter().zip(expected["nodes"].as_array().unwrap()) {
            let mut without_path = node.clone();
            let path = without_path
                .as_object_mut()
                .unwrap()
                .remove("path")
                .unwrap();
            assert_eq!(&without_path, want, "{file}");
            let id = node["nodeId"].as_str().unwrap();
            assert!(
                path.as_str().unwrap().ends_with(&format!("/node/{id}")),
                "{path}"
            );
        }

        // The camera block is the one `scene.camera` prints.
        let camera = ok(
            &mut state,
            json!({"op":"scene.camera","scene":"scene","frame":frame}),
        );
        assert_eq!(answer["camera"], camera["camera"], "{file}");
        assert_eq!(answer["shotId"], camera["shotId"], "{file}");
        assert_eq!(observable(&state), before, "evaluating changes nothing");
    }
}

#[test]
fn evaluate_shows_animated_nodes_and_accepts_exact_time() {
    let mut state = new_project();
    attach(&mut state, "rig", RIG);
    let by_frame = ok(
        &mut state,
        json!({"op":"scene.evaluate","scene":"rig","frame":12}),
    );
    assert_eq!(by_frame["animation"]["status"], "animated");
    assert_eq!(
        node_of(&by_frame, "arm")["path"],
        "scene/scene-rig/node/arm"
    );
    // 12 frames at 24 fps is half a second.
    let by_time = ok(
        &mut state,
        json!({"op":"scene.evaluate","scene":"rig","time":{"num":1,"den":2}}),
    );
    assert_eq!(by_time["editFrame"], 12);
    assert_eq!(by_time["nodes"], by_frame["nodes"]);
}

#[test]
fn evaluate_refuses_bad_requests_with_typed_errors_and_changes_nothing() {
    let mut state = street();
    let before = observable(&state);
    assert_eq!(
        err(
            &mut state,
            json!({"op":"scene.evaluate","scene":"street","frame":96})
        )
        .code,
        "frame_out_of_range"
    );
    assert_eq!(
        err(
            &mut state,
            json!({"op":"scene.evaluate","scene":"street","frame":-1})
        )
        .code,
        "frame_out_of_range"
    );
    assert_eq!(
        err(
            &mut state,
            json!({"op":"scene.evaluate","scene":"street","frame":1,"time":1})
        )
        .code,
        "invalid_request"
    );
    assert_eq!(
        err(&mut state, json!({"op":"scene.evaluate","scene":"street"})).code,
        "invalid_request"
    );
    assert_eq!(
        err(
            &mut state,
            json!({"op":"scene.evaluate","scene":"nope","frame":1})
        )
        .code,
        "unknown_element"
    );
    assert_eq!(observable(&state), before);
}

// ---- gestures -----------------------------------------------------------------------------

#[test]
fn a_gesture_previews_apart_and_commits_as_one_undo_entry() {
    let mut state = street();
    let original = document_of(&state, "street");
    let before = observable(&state);
    let attached_depth = undo_depth(&state);

    let begun = ok(
        &mut state,
        json!({"op":"scene.gesture.begin","scene":"street","targets":[FIGURE],"label":"drag figure"}),
    );
    // IDs are opaque (BL066): read it from the answer, never spell it.
    let id = begun["gesture"].as_str().unwrap().to_string();
    assert!(id.starts_with("gesture-"));
    assert_eq!(begun["targets"], json!([FIGURE]));
    assert_eq!(begun["document_changed"], false);

    // Two properties in one update: both move together in the commit.
    let first = update(
        &mut state,
        &id,
        vec![
            move_figure(2.0),
            json!({"set": "scene/scene-a/node/figure/scale", "value": {"x": 2, "y": 2, "z": 2}}),
        ],
    );
    assert_eq!(first["would_change"], true);
    assert_eq!(first["changed_ops"], 2);
    assert_eq!(first["changes"][0]["property"], FIGURE_POSITION);
    assert_eq!(first["changes"][0]["before"], json!({"x":0.5,"y":0,"z":0}));
    assert_eq!(first["document_changed"], false);

    // Absolute, not cumulative: the second update starts again from the base.
    let second = update(&mut state, &id, vec![move_figure(3.0)]);
    assert_eq!(second["updates"], 2);
    assert_eq!(second["changed_ops"], 1);
    assert_eq!(
        second["changes"][0]["before"],
        json!({"x":0.5,"y":0,"z":0}),
        "the base, not the previous candidate"
    );

    // The preview is separate: the document, history and revision did not move.
    assert_eq!(observable(&state), before);
    assert_eq!(document_of(&state, "street"), original);
    assert_eq!(figure_x(&mut state), 0.5);
    let preview = ok(
        &mut state,
        json!({"op":"scene.evaluate","scene":"street","frame":10,"gesture":id}),
    );
    assert_eq!(preview["source"], "gesture");
    assert_eq!(preview["has_candidate"], true);
    assert_eq!(node_of(&preview, "figure")["localPosition"]["x"], 3.0);
    let document_frame = ok(
        &mut state,
        json!({"op":"scene.evaluate","scene":"street","frame":10}),
    );
    assert_eq!(
        node_of(&document_frame, "figure")["localPosition"]["x"],
        0.5
    );
    assert_eq!(
        ok(&mut state, json!({"op":"scene.gesture.status"}))["gesture"]["state"],
        "open"
    );
    assert_eq!(observable(&state), before, "previews and status are reads");

    let committed = ok(
        &mut state,
        json!({"op":"scene.gesture.commit","gesture":id}),
    );
    assert_eq!(committed["action"], "scene.gesture.commit");
    assert_eq!(committed["changed"], true);
    assert_eq!(committed["document_changed"], true);
    assert_eq!(figure_x(&mut state), 3.0);
    assert_eq!(
        undo_depth(&state),
        attached_depth + 1,
        "the whole gesture is one entry"
    );
    assert_eq!(state.history.undo_label(), Some("Edit scene"));
    assert_eq!(
        ok(&mut state, json!({"op":"scene.gesture.status"}))["active"],
        false
    );

    // The semantic equivalent of the gesture is the same batch as `scene.edit`.
    let mut direct = street();
    ok(
        &mut direct,
        json!({"op":"scene.edit","scene":"street","ops":[move_figure(3.0)]}),
    );
    assert_eq!(
        document_of(&state, "street"),
        document_of(&direct, "street")
    );

    ok(&mut state, json!({"op":"history.undo"}));
    assert_eq!(document_of(&state, "street"), original);
    assert_eq!(undo_depth(&state), attached_depth);
}

const CAMERA: &str = "camera/cam-main";
const START_X: &str = "camera/cam-main/start/position/x";

fn first_recipe(state: &EditorState) -> Value {
    document_of(state, "street")["cameraClips"][0]["segments"][0]["recipe"].clone()
}

#[test]
fn a_gesture_on_the_camera_start_reports_its_conversion_and_commits_it_as_one_undo_entry() {
    // BL005a: the slider of `start/position` is a gesture; the first movement becomes keys in the
    // preview (separate from the document) and in the commit, as one entry. `conversions` rides
    // on `update` and on `commit`, never when the candidate nets out to nothing.
    let mut state = street();
    let original = document_of(&state, "street");
    let before = observable(&state);
    let attached_depth = undo_depth(&state);
    assert_eq!(first_recipe(&state), json!("dollyIn"));

    let begun = ok(
        &mut state,
        json!({"op":"scene.gesture.begin","scene":"street","targets":[CAMERA]}),
    );
    let id = begun["gesture"].as_str().unwrap().to_string();

    let first = update(&mut state, &id, vec![json!({"set": START_X, "value": 2})]);
    assert_eq!(first["would_change"], true);
    assert_eq!(first["document_changed"], false);
    let conversions = first["conversions"].as_array().expect("conversions");
    assert_eq!(conversions.len(), 1, "{first}");
    assert_eq!(conversions[0]["clip"], "cam-main");
    assert_eq!(conversions[0]["segment"], 0);
    assert_eq!(conversions[0]["recipe"], "dollyIn");
    assert_eq!(conversions[0]["keys"], 1);
    // The preview converted; the document did not.
    assert_eq!(observable(&state), before);
    assert_eq!(first_recipe(&state), json!("dollyIn"));

    // Absolute from the base, so a second update converts from the base again, once.
    let second = update(&mut state, &id, vec![json!({"set": START_X, "value": 3})]);
    assert_eq!(second["conversions"].as_array().unwrap().len(), 1);
    assert_eq!(second["changes"][0]["before"], json!(0));

    // Back to where it started within the same update: nothing to commit, nothing converted.
    let back = update(
        &mut state,
        &id,
        vec![
            json!({"set": START_X, "value": 2}),
            json!({"set": START_X, "value": 0}),
        ],
    );
    assert_eq!(back["would_change"], false);
    assert!(back.get("conversions").is_none(), "{back}");

    update(&mut state, &id, vec![json!({"set": START_X, "value": 3})]);
    let committed = ok(
        &mut state,
        json!({"op":"scene.gesture.commit","gesture":id}),
    );
    assert_eq!(committed["changed"], true);
    assert_eq!(committed["document_changed"], true);
    assert_eq!(committed["conversions"].as_array().unwrap().len(), 1);
    assert_eq!(committed["conversions"][0]["recipe"], "dollyIn");
    assert_eq!(first_recipe(&state), json!("keyframes"));
    assert_eq!(
        document_of(&state, "street")["cameraClips"][0]["initialPosition"]["x"],
        3
    );
    assert_eq!(
        undo_depth(&state),
        attached_depth + 1,
        "the conversion is part of the one entry"
    );

    // The same batch sent as `scene.edit` gives the same document.
    let mut direct = street();
    ok(
        &mut direct,
        json!({"op":"scene.edit","scene":"street","ops":[{"set": START_X, "value": 3}]}),
    );
    assert_eq!(
        document_of(&state, "street"),
        document_of(&direct, "street")
    );

    ok(&mut state, json!({"op":"history.undo"}));
    assert_eq!(document_of(&state, "street"), original);
    assert_eq!(first_recipe(&state), json!("dollyIn"));
    assert_eq!(undo_depth(&state), attached_depth);
}

#[test]
fn cancel_discards_the_candidate_and_leaves_no_trace() {
    let mut state = street();
    let before = observable(&state);
    let gesture = begin(&mut state);
    update(&mut state, &gesture, vec![move_figure(2.0)]);
    let cancelled = ok(
        &mut state,
        json!({"op":"scene.gesture.cancel","gesture":gesture}),
    );
    assert_eq!(cancelled["discarded_updates"], 1);
    assert_eq!(cancelled["was_invalidated"], false);
    assert_eq!(observable(&state), before);
    assert!(state.scene_interaction.gesture().is_none());

    for request in [
        json!({"op":"scene.gesture.commit","gesture":gesture}),
        json!({"op":"scene.gesture.update","gesture":gesture,"ops":[]}),
        json!({"op":"scene.gesture.cancel","gesture":gesture}),
    ] {
        assert_eq!(err(&mut state, request).code, "no_gesture");
    }
    assert_eq!(observable(&state), before);

    // IDs are not reused: a late commit of the cancelled gesture cannot hit the next one.
    let next = begin(&mut state);
    assert_ne!(next, gesture);
    update(&mut state, &next, vec![move_figure(2.0)]);
    let late = err(
        &mut state,
        json!({"op":"scene.gesture.commit","gesture":gesture}),
    );
    assert_eq!(late.code, "unknown_gesture");
    assert_eq!(figure_x(&mut state), 0.5);
    assert_eq!(observable(&state), before);
}

#[test]
fn empty_and_net_no_op_gestures_commit_without_an_undo_entry() {
    let mut state = street();
    let before = observable(&state);

    let empty = begin(&mut state);
    let answer = ok(
        &mut state,
        json!({"op":"scene.gesture.commit","gesture":empty}),
    );
    assert_eq!(answer["changed"], false);
    assert_eq!(answer["noop"], true);
    assert_eq!(observable(&state), before);

    let same = begin(&mut state);
    let preview = update(&mut state, &same, vec![move_figure(0.5)]);
    assert_eq!(preview["would_change"], false);
    let answer = ok(
        &mut state,
        json!({"op":"scene.gesture.commit","gesture":same}),
    );
    assert_eq!(answer["changed"], false);
    assert_eq!(
        observable(&state),
        before,
        "writing the value it had is no edit"
    );

    // An empty update returns a gesture to its start.
    let back = begin(&mut state);
    update(&mut state, &back, vec![move_figure(2.0)]);
    update(&mut state, &back, vec![]);
    let answer = ok(
        &mut state,
        json!({"op":"scene.gesture.commit","gesture":back}),
    );
    assert_eq!(answer["changed"], false);
    assert_eq!(observable(&state), before);
}

#[test]
fn a_rejected_update_keeps_the_last_good_candidate_and_changes_nothing() {
    let mut state = street();
    let before = observable(&state);
    let gesture = begin(&mut state);
    update(&mut state, &gesture, vec![move_figure(2.0)]);

    let ghost = err(
        &mut state,
        json!({"op":"scene.gesture.update","gesture":gesture,"ops":[
            {"set":"scene/scene-a/node/ghost/position","value":{"x":1,"y":1,"z":1}}]}),
    );
    // Gesture target authorization runs before validating the edit payload.
    assert_eq!(ghost.code, "gesture_target_mismatch");
    let wrong_field = err(
        &mut state,
        json!({"op":"scene.gesture.update","gesture":gesture,"ops":[
            {"set":"scene/scene-a/node/figure/colour","value":1}]}),
    );
    assert_eq!(wrong_field.code, "unknown_path");
    let range = err(
        &mut state,
        json!({"op":"scene.gesture.update","gesture":gesture,"ops":[
            {"set":"scene/scene-a/node/figure/width","value":-1}]}),
    );
    assert_eq!(
        range.code, "invalid_value",
        "the catalog of scene.edit decides, not a second rule set"
    );
    assert_eq!(observable(&state), before);

    let status = ok(&mut state, json!({"op":"scene.gesture.status"}));
    assert_eq!(
        status["gesture"]["updates"], 1,
        "a rejection is not an update"
    );
    assert_eq!(status["gesture"]["state"], "open");

    ok(
        &mut state,
        json!({"op":"scene.gesture.commit","gesture":gesture}),
    );
    assert_eq!(
        figure_x(&mut state),
        2.0,
        "the last good candidate was committed"
    );
}

#[test]
fn a_gesture_may_only_touch_the_entities_it_was_begun_on() {
    let mut state = street();
    let before = observable(&state);
    let gesture = begin(&mut state);
    let error = err(
        &mut state,
        json!({"op":"scene.gesture.update","gesture":gesture,"ops":[
            move_figure(2.0),
            {"set":FOV,"value":55}]}),
    );
    assert_eq!(error.code, "gesture_target_mismatch");
    assert_eq!(error.target.as_deref(), Some("camera/cam-main"));
    assert_eq!(error.details.as_deref().unwrap()["index"], 1);
    assert_eq!(observable(&state), before);

    // One gesture at a time: a second begin neither replaces nor disturbs the live one.
    let still_open = err(
        &mut state,
        json!({"op":"scene.gesture.begin","scene":"street"}),
    );
    assert_eq!(still_open.code, "gesture_active");
    assert_eq!(still_open.target.as_deref(), Some(gesture.as_str()));
    assert_eq!(
        state.scene_interaction.gesture().unwrap().id(),
        gesture.as_str()
    );
    ok(
        &mut state,
        json!({"op":"scene.gesture.cancel","gesture":gesture}),
    );

    // Without declared targets any catalog property may be edited.
    let open = ok(
        &mut state,
        json!({"op":"scene.gesture.begin","scene":"street"}),
    );
    assert_eq!(open["replaced_stale"], Value::Null);
    let free = open["gesture"].as_str().unwrap().to_string();
    let preview = update(
        &mut state,
        &free,
        vec![move_figure(2.0), json!({"set":FOV,"value":55})],
    );
    assert_eq!(preview["changed_ops"], 2);
    ok(
        &mut state,
        json!({"op":"scene.gesture.cancel","gesture":free}),
    );
    assert_eq!(observable(&state), before);
}

#[test]
fn begin_validates_its_scene_targets_and_label() {
    let mut state = street();
    let before = observable(&state);
    for (request, code) in [
        (
            json!({"op":"scene.gesture.begin","scene":"nope"}),
            "unknown_element",
        ),
        (
            json!({"op":"scene.gesture.begin","scene":"street","targets":["scene/scene-a/node/ghost"]}),
            "unknown_entity",
        ),
        (
            json!({"op":"scene.gesture.begin","scene":"street","targets":["???"]}),
            "invalid_entity_path",
        ),
        (
            json!({"op":"scene.gesture.begin","scene":"street","label":"  "}),
            "invalid_value",
        ),
    ] {
        assert_eq!(err(&mut state, request).code, code);
    }
    assert!(state.scene_interaction.gesture().is_none());
    assert_eq!(observable(&state), before);
}

#[test]
fn an_edit_invalidates_the_gesture_and_the_stale_candidate_is_never_applied() {
    let mut state = street();
    let gesture = begin(&mut state);
    update(&mut state, &gesture, vec![move_figure(2.0)]);

    // Another edit of the same scene lands first.
    ok(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[{"set":FOV,"value":55}]}),
    );
    let after_edit = observable(&state);

    for request in [
        json!({"op":"scene.gesture.update","gesture":gesture,"ops":[{"set":FIGURE_POSITION,"value":{"x":9,"y":0,"z":0}}]}),
        json!({"op":"scene.gesture.commit","gesture":gesture}),
        json!({"op":"scene.evaluate","scene":"street","frame":1,"gesture":gesture}),
    ] {
        let error = err(&mut state, request.clone());
        assert_eq!(error.code, "gesture_invalidated", "{request}");
        assert_eq!(error.target.as_deref(), Some(gesture.as_str()));
        assert_eq!(error.details.as_deref().unwrap()["reason"], "scene_changed");
    }
    assert_eq!(observable(&state), after_edit, "nothing stale was applied");
    assert_eq!(figure_x(&mut state), 0.5);
    assert_eq!(undo_depth(&state), 2);

    let status = ok(&mut state, json!({"op":"scene.gesture.status"}));
    assert_eq!(status["gesture"]["state"], "invalidated");
    assert_eq!(status["gesture"]["invalidation"], "scene_changed");

    // Cancel always works on a stale gesture, and a new one can begin.
    let cancelled = ok(
        &mut state,
        json!({"op":"scene.gesture.cancel","gesture":gesture}),
    );
    assert_eq!(cancelled["was_invalidated"], true);
    let fresh = begin(&mut state);
    update(&mut state, &fresh, vec![move_figure(4.0)]);
    ok(
        &mut state,
        json!({"op":"scene.gesture.commit","gesture":fresh}),
    );
    assert_eq!(figure_x(&mut state), 4.0);
    assert_eq!(undo_depth(&state), 3);
}

#[test]
fn undo_and_redo_invalidate_a_gesture_even_when_the_scene_comes_back() {
    // An undo that moves the scene.
    let mut state = street();
    ok(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[{"set":FOV,"value":55}]}),
    );
    let gesture = begin(&mut state);
    update(&mut state, &gesture, vec![move_figure(2.0)]);
    ok(&mut state, json!({"op":"history.undo"}));
    let after_undo = observable(&state);
    let error = err(
        &mut state,
        json!({"op":"scene.gesture.commit","gesture":gesture}),
    );
    assert_eq!(error.code, "gesture_invalidated");
    assert_eq!(error.details.as_deref().unwrap()["reason"], "scene_changed");
    assert_eq!(observable(&state), after_undo);

    // An undo/redo pair puts the very same scene back; the revision still says it happened.
    let mut state = street();
    let gesture = begin(&mut state);
    update(&mut state, &gesture, vec![move_figure(2.0)]);
    ok(&mut state, json!({"op":"history.undo"}));
    ok(&mut state, json!({"op":"history.redo"}));
    assert!(state.scenes.get("street").is_some());
    let before = observable(&state);
    let error = err(
        &mut state,
        json!({"op":"scene.gesture.commit","gesture":gesture}),
    );
    assert_eq!(error.code, "gesture_invalidated");
    assert_eq!(
        error.details.as_deref().unwrap()["reason"],
        "project_revision"
    );
    assert_eq!(observable(&state), before);
    assert_eq!(figure_x(&mut state), 0.5);
}

#[test]
fn an_unrelated_edit_also_invalidates_conservatively() {
    let mut state = street();
    let gesture = begin(&mut state);
    update(&mut state, &gesture, vec![move_figure(2.0)]);
    ok(
        &mut state,
        json!({"op":"track.add","name":"V","kind":"video"}),
    );
    let error = err(
        &mut state,
        json!({"op":"scene.gesture.commit","gesture":gesture}),
    );
    assert_eq!(error.code, "gesture_invalidated");
    assert_eq!(
        error.details.as_deref().unwrap()["reason"],
        "project_revision"
    );
    assert_eq!(figure_x(&mut state), 0.5);
}

#[test]
fn detaching_the_scene_invalidates_the_gesture_even_if_it_is_attached_again() {
    let mut state = street();
    let gesture = begin(&mut state);
    update(&mut state, &gesture, vec![move_figure(2.0)]);
    ok(&mut state, json!({"op":"scene.detach","scene":"street"}));
    let detached = err(
        &mut state,
        json!({"op":"scene.gesture.commit","gesture":gesture}),
    );
    assert_eq!(detached.code, "gesture_invalidated");
    assert_eq!(
        detached.details.as_deref().unwrap()["reason"],
        "scene_detached"
    );
    assert!(state.scenes.is_empty());

    attach(&mut state, "street", MINIMAL);
    let again = err(
        &mut state,
        json!({"op":"scene.gesture.commit","gesture":gesture}),
    );
    assert_eq!(again.code, "gesture_invalidated");
    assert_eq!(figure_x(&mut state), 0.5);

    // The stale gesture does not block the next one, and the answer says it was replaced.
    let next = ok(
        &mut state,
        json!({"op":"scene.gesture.begin","scene":"street"}),
    );
    assert_eq!(next["replaced_stale"]["gesture"], gesture);
    assert_eq!(next["replaced_stale"]["reason"], "scene_changed");
}

#[test]
fn a_gesture_is_checked_against_the_scene_it_was_begun_on() {
    let mut state = new_project();
    attach(&mut state, "street", MINIMAL);
    attach(&mut state, "other", OFFSETS);
    let gesture = begin(&mut state);
    let error = err(
        &mut state,
        json!({"op":"scene.evaluate","scene":"other","frame":1,"gesture":gesture}),
    );
    assert_eq!(error.code, "gesture_scene_mismatch");
    let unknown = err(
        &mut state,
        json!({"op":"scene.gesture.commit","gesture":"gesture-99"}),
    );
    assert_eq!(unknown.code, "unknown_gesture");
    assert!(state.scene_interaction.gesture().is_some());
}

// ---- the typed library API ----------------------------------------------------------------

#[test]
fn the_library_drives_the_same_cycle_without_json() {
    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    state
        .execute(Command::Scene(SceneCommand::Attach {
            id: Some("street".to_string()),
            name: None,
            source: SceneSource::Bundle(bundle(MINIMAL)),
        }))
        .unwrap();

    let begun = state
        .execute(Command::Scene(SceneCommand::Gesture(
            GestureCommand::Begin {
                scene: "street".to_string(),
                targets: vec![FIGURE.to_string()],
                label: None,
            },
        )))
        .unwrap();
    let id = begun["gesture"].as_str().unwrap().to_string();
    let open = state.scene_interaction.gesture().unwrap();
    assert_eq!(open.id(), id);
    assert_eq!(open.scene(), "street");
    assert!(!open.has_candidate());
    assert_eq!(undo_depth(&state), 1, "only the attach");

    let ops = vec![EditOp::set(
        PropertyRef::node("scene-a", "figure", NodeProperty::Position),
        PropertyValue::Vec3(Vec3 {
            x: 2.0,
            y: 0.0,
            z: 0.0,
        }),
    )];
    state
        .execute(Command::Scene(SceneCommand::Gesture(
            GestureCommand::Update {
                gesture: id.clone(),
                ops,
            },
        )))
        .unwrap();
    assert!(state.scene_interaction.gesture().unwrap().has_candidate());
    assert_eq!(undo_depth(&state), 1);

    let committed = state
        .execute(Command::Scene(SceneCommand::Gesture(
            GestureCommand::Commit { gesture: id },
        )))
        .unwrap();
    assert_eq!(committed["changed"], true);
    assert_eq!(undo_depth(&state), 2);
    assert!(state.scene_interaction.gesture().is_none());
}

// ---- one vocabulary, in a JSON Lines session ----------------------------------------------

/// Requests spell the gesture as `$gesture`: an ID is opaque (BL066) and only known once `begin`
/// answered, so the driver below fills in the latest one before it writes each line.
fn session_requests() -> Vec<String> {
    [
        json!({"op":"scene.attach","id":"street","bundle":bundle(MINIMAL)}),
        json!({"op":"scene.tree","scene":"street"}),
        json!({"op":"scene.select","scene":"street","entities":[FIGURE]}),
        json!({"op":"scene.selection"}),
        json!({"op":"scene.evaluate","scene":"street","frame":10}),
        json!({"op":"scene.gesture.begin","scene":"street","targets":[FIGURE]}),
        json!({"op":"scene.gesture.update","gesture":"$gesture","ops":[move_figure(2.0)]}),
        json!({"op":"scene.evaluate","scene":"street","frame":10,"gesture":"$gesture"}),
        json!({"op":"scene.gesture.update","gesture":"$gesture","ops":[
            {"set":"scene/scene-a/node/ghost/position","value":{"x":1,"y":1,"z":1}}]}),
        json!({"op":"scene.gesture.commit","gesture":"$gesture"}),
        json!({"op":"scene.get","scene":"street","property":FIGURE_POSITION}),
        json!({"op":"history.undo"}),
        json!({"op":"scene.get","scene":"street","property":FIGURE_POSITION}),
        json!({"op":"scene.gesture.begin","scene":"street"}),
        json!({"op":"scene.gesture.cancel","gesture":"$gesture"}),
        json!({"op":"scene.gesture.commit","gesture":"$gesture"}),
        json!({"op":"scene.select","scene":"street","mode":"add","entities":["camera/ghost"]}),
        json!({"op":"scene.gesture.status"}),
    ]
    .iter()
    .map(Value::to_string)
    .chain(["this is not json".to_string()])
    .collect()
}

#[test]
fn a_json_lines_session_runs_the_whole_interaction_flow_in_process() {
    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    // One `run_session` call per line over the same state: the same executor and answers as a
    // single call with every line, with the chance to read the gesture ID in between.
    let mut gesture = String::new();
    let mut output = Vec::new();
    let mut failed = false;
    for line in session_requests() {
        let line = line.replace("$gesture", &gesture);
        failed |= headless::run_session(&mut state, None, line.as_bytes(), &mut output).unwrap();
        let answer: Value = serde_json::from_str(
            String::from_utf8_lossy(&output)
                .lines()
                .last()
                .expect("one answer per line"),
        )
        .unwrap();
        if let Some(id) = answer["result"]["gesture"]
            .as_str()
            .filter(|_| answer["result"]["action"] == "scene.gesture.begin")
        {
            gesture = id.to_string();
        }
    }
    assert!(failed, "the session reports that some requests failed");

    let responses: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(responses.len(), 19, "one response per line, in order");
    let result = |index: usize| {
        assert_eq!(responses[index]["ok"], true, "{}", responses[index]);
        &responses[index]["result"]
    };
    let error_code = |index: usize| {
        assert_eq!(responses[index]["ok"], false, "{}", responses[index]);
        responses[index]["error"]["code"]
            .as_str()
            .unwrap()
            .to_string()
    };

    assert_eq!(result(1)["tree"]["kind"], "bundle");
    assert_eq!(result(2)["count"], 1);
    assert_eq!(result(3)["entities"][0]["path"], FIGURE);
    assert_eq!(
        node_of(result(4), "figure")["localPosition"]["x"],
        0.5,
        "the document's own frame"
    );
    let first_gesture = result(5)["gesture"].as_str().unwrap().to_string();
    assert_eq!(result(6)["would_change"], true);
    assert_eq!(result(7)["source"], "gesture");
    assert_eq!(
        node_of(result(7), "figure")["localPosition"]["x"],
        2.0,
        "the candidate's frame"
    );
    assert_eq!(error_code(8), "gesture_target_mismatch");
    assert_eq!(responses[8]["error"]["details"]["index"], 0);
    assert_eq!(result(9)["changed"], true);
    assert_eq!(result(10)["value"]["x"], 2.0);
    assert_eq!(result(11)["action"], "history.undo");
    assert_eq!(
        result(12)["value"]["x"],
        0.5,
        "one undo reverted the whole gesture"
    );
    let second_gesture = result(13)["gesture"].as_str().unwrap();
    assert_ne!(second_gesture, first_gesture, "an ID is never reused");
    assert_eq!(result(14)["action"], "scene.gesture.cancel");
    assert_eq!(error_code(15), "no_gesture");
    assert_eq!(error_code(16), "unknown_entity");
    assert_eq!(result(17)["active"], false);
    assert_eq!(error_code(18), "invalid_json");
    assert_eq!(
        undo_depth(&state),
        1,
        "the attach is the only entry left after the undo"
    );
}
