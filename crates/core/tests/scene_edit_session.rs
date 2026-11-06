//! S6 (BL004/BL005): the property catalog extension through the JSON executor that
//! `tessel-headless` and `ui op` share, with a project and its history (no window, no GPU).
//!
//! Proves the additive contract (capabilities, listing flag, catalog fields), the batch semantics
//! seen by a session (atomic, reset, no-op, one undo entry) and that the headless executable
//! answers like the library. The origin equivalence of the camera conversion is in
//! `scene_edit_oracle.rs`.

use std::io::Write;
use std::process::{Command as Process, Stdio};

use serde_json::{Value, json};
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::{CommandError, EditorState, headless};

const LAYERED: &str = include_str!("directed_scene_fixtures/layered_scene.json");

fn bundle() -> Value {
    let mut document: Value = serde_json::from_str(LAYERED).unwrap();
    document["shots"][1]["transitionInto"] = json!({
        "type": "cut", "reason": "synthetic editing baseline"
    });
    document
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

fn session() -> EditorState {
    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    ok(
        &mut state,
        json!({"op":"scene.attach","id":"street","bundle":bundle()}),
    );
    state
}

fn undo_depth(state: &EditorState) -> u64 {
    state.history.to_json()["undo_depth"].as_u64().unwrap()
}

fn document(state: &EditorState) -> Value {
    state.scenes.get("street").unwrap().scene.document().clone()
}

fn value_of(state: &mut EditorState, path: &str) -> Value {
    ok(
        state,
        json!({"op":"scene.get","scene":"street","property":path}),
    )["value"]
        .clone()
}

fn names(listing: &Value) -> Vec<String> {
    listing["properties"]
        .as_array()
        .unwrap()
        .iter()
        .map(|property| property["path"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn the_extension_is_negotiated_and_the_default_listing_has_no_components() {
    let mut state = session();
    let capabilities = ok(&mut state, json!({"op":"scene.capabilities"}));
    assert_eq!(capabilities["edit"]["catalog_version"], 2);
    assert_eq!(capabilities["edit"]["components"], true);
    assert_eq!(capabilities["edit"]["render_order"], true);
    assert_eq!(capabilities["edit"]["batch"], "atomic");

    let plain = ok(
        &mut state,
        json!({"op":"scene.properties","scene":"street"}),
    );
    let with = ok(
        &mut state,
        json!({"op":"scene.properties","scene":"street","components":true}),
    );
    let (plain, with) = (names(&plain), names(&with));
    assert!(plain.iter().all(|path| !path.ends_with("/x")));
    // The default listing is not the first slice's: it gains the `renderOrder` rows of the image
    // planes (the fixture has three). Only the components are opt-in.
    let render_order: Vec<_> = plain
        .iter()
        .filter(|p| p.ends_with("/renderOrder"))
        .collect();
    assert_eq!(render_order.len(), 3, "{plain:?}");
    let axes = with
        .iter()
        .filter(|path| ["/x", "/y", "/z"].iter().any(|axis| path.ends_with(axis)))
        .count();
    assert!(axes > 0);
    assert_eq!(with.len() - plain.len(), axes, "only components are opt-in");
    assert!(with.len() > plain.len());
    assert!(with.contains(&"scene/street/node/hero-plane/position/x".to_string()));
    assert!(with.contains(&"camera/wide/start/target/z".to_string()));
    // Per entity, the components follow its own properties; nothing else is reordered.
    let last_own = with
        .iter()
        .position(|path| path == "scene/street/node/hero-plane/renderOrder")
        .unwrap();
    assert_eq!(
        with[last_own + 1],
        "scene/street/node/hero-plane/position/x",
        "{with:?}"
    );
    let without_components: Vec<_> = with.iter().filter(|path| plain.contains(path)).collect();
    assert_eq!(without_components, plain.iter().collect::<Vec<_>>());

    let catalog = ok(&mut state, json!({"op":"scene.catalog"}));
    let entries = catalog["properties"].as_array().unwrap();
    let find = |name: &str| entries.iter().find(|entry| entry["name"] == name).unwrap();
    assert_eq!(find("renderOrder")["kind"], "integer");
    assert_eq!(find("renderOrder")["optional"], true);
    assert_eq!(find("position/x")["component_of"]["parent"], "position");
    assert_eq!(find("position/x")["component_of"]["index"], 0);
    assert_eq!(find("start/position")["converts_first_movement"], true);
    assert_eq!(find("start/position")["origin"], "path-equivalent-write");
    // The origin has no per-axis path: an axis of the camera start is the parent's write with one
    // axis replaced, and says so; node axes stay what their parent is.
    for axis in ["x", "y", "z"] {
        assert_eq!(
            find(&format!("start/position/{axis}"))["origin"],
            "parent-write"
        );
        assert_eq!(
            find(&format!("start/target/{axis}"))["origin"],
            "parent-write"
        );
    }
    assert_eq!(find("position/x")["origin"], "editor-only");
    assert_eq!(find("start/target")["origin"], "command-only");
    assert_eq!(find("start/fov")["converts_first_movement"], false);
}

#[test]
fn a_component_and_render_order_edit_is_one_undo_entry() {
    let mut state = session();
    let before = document(&state);
    let depth = undo_depth(&state);

    let edited = ok(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[
            {"set":"scene/street/node/hero-plane/position/y","value":3.5},
            {"set":"scene/street/node/prop-plane/renderOrder","value":7},
            {"set":"scene/street/node/hero-plane/scale/x","value":2}]}),
    );
    assert_eq!(edited["changed"], true);
    assert_eq!(edited["changed_ops"], 3);
    assert!(edited.get("conversions").is_none(), "nothing converts");
    assert_eq!(undo_depth(&state), depth + 1);
    assert_eq!(
        value_of(&mut state, "scene/street/node/hero-plane/position")["y"],
        3.5
    );
    assert_eq!(
        value_of(&mut state, "scene/street/node/prop-plane/renderOrder"),
        7
    );

    ok(&mut state, json!({"op":"history.undo"}));
    assert_eq!(document(&state), before);
    assert_eq!(
        value_of(&mut state, "scene/street/node/prop-plane/renderOrder"),
        0,
        "an order the bundle leaves out reads as the default"
    );
}

#[test]
fn a_refused_batch_changes_nothing_and_a_no_op_leaves_no_entry() {
    let mut state = session();
    let before = document(&state);
    let depth = undo_depth(&state);

    let error = err(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[
            {"set":"scene/street/node/hero-plane/position/x","value":1},
            {"set":"scene/street/node/hero-plane/renderOrder","value":1.5}]}),
    );
    assert_eq!(error.code, "invalid_type");
    assert_eq!(error.details.as_deref().unwrap()["index"], 1);
    assert_eq!(document(&state), before);
    assert_eq!(undo_depth(&state), depth);

    // Group nodes have no renderOrder; the whole batch is refused at the op that asked.
    let error = err(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[
            {"set":"scene/street/node/root/renderOrder","value":2}]}),
    );
    assert_eq!(error.code, "not_applicable");
    assert_eq!(document(&state), before);

    let current = value_of(&mut state, "scene/street/node/hero-plane/renderOrder");
    let noop = ok(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[
            {"set":"scene/street/node/hero-plane/renderOrder","value":current},
            {"reset":"scene/street/node/prop-plane/renderOrder"}]}),
    );
    assert_eq!(noop["changed"], false);
    assert_eq!(noop["noop"], true);
    assert_eq!(undo_depth(&state), depth);
    assert_eq!(document(&state), before);
}

#[test]
fn moving_the_camera_start_converts_the_first_movement_and_undo_restores_it() {
    let mut state = session();
    let before = document(&state);
    let depth = undo_depth(&state);
    let clip_before = before["cameraClips"][0].clone();

    let dry = ok(
        &mut state,
        json!({"op":"scene.edit","scene":"street","dry_run":true,"ops":[
            {"set":"camera/wide/start/position/x","value":1.5}]}),
    );
    assert_eq!(dry["would_change"], true);
    assert_eq!(dry["conversions"].as_array().unwrap().len(), 1, "{dry}");
    assert_eq!(document(&state), before, "a dry run writes nothing");

    let edited = ok(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[
            {"set":"camera/wide/start/position/x","value":1.5}]}),
    );
    assert_eq!(edited["changed"], true);
    assert_eq!(edited["conversions"], dry["conversions"]);
    assert_eq!(edited["conversions"][0]["clip"], "wide");
    assert_eq!(edited["conversions"][0]["recipe"], "dollyIn");
    let after = document(&state);
    assert_eq!(after["cameraClips"][0]["initialPosition"]["x"], 1.5);
    assert_eq!(
        after["cameraClips"][0]["segments"][0]["recipe"],
        "keyframes"
    );
    assert_eq!(
        undo_depth(&state),
        depth + 1,
        "the conversion is part of the one entry"
    );

    ok(&mut state, json!({"op":"history.undo"}));
    assert_eq!(document(&state)["cameraClips"][0], clip_before);

    // Target and field of view are written in place, with nothing converted.
    let plain = ok(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[
            {"set":"camera/wide/start/target/y","value":1.25}]}),
    );
    assert!(plain.get("conversions").is_none());
    assert_eq!(
        document(&state)["cameraClips"][0]["segments"][0]["recipe"],
        "dollyIn"
    );
}

fn x_of_hero(state: &mut EditorState) -> f64 {
    value_of(state, "scene/street/node/hero-plane/position")["x"]
        .as_f64()
        .unwrap()
}

#[test]
fn an_axis_edit_hidden_between_parent_writes_is_a_real_edit_with_one_undo_entry() {
    // BL004a. Hero plane starts at x = 3. Judged per path this batch looked like a no-op
    // (`position` 3 -> 3, `position/x` 2 -> 2) and the edit was discarded.
    let mut state = session();
    let before = document(&state);
    let depth = undo_depth(&state);
    let edited = ok(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[
            {"set":"scene/street/node/hero-plane/position","value":{"x":2,"y":0,"z":0}},
            {"set":"scene/street/node/hero-plane/position/x","value":5},
            {"set":"scene/street/node/hero-plane/position","value":{"x":3,"y":0,"z":0}},
            {"set":"scene/street/node/hero-plane/position/x","value":2}]}),
    );
    assert_eq!(edited["changed"], true, "{edited}");
    assert_eq!(edited["noop"], false);
    assert_eq!(x_of_hero(&mut state), 2.0);
    assert_eq!(undo_depth(&state), depth + 1);
    ok(&mut state, json!({"op":"history.undo"}));
    assert_eq!(document(&state), before);
    assert_eq!(x_of_hero(&mut state), 3.0);
    assert_eq!(undo_depth(&state), depth);
}

#[test]
fn an_axis_restored_by_the_parent_vector_is_a_no_op_without_dirt_or_undo_entry() {
    let mut state = session();
    let before = document(&state);
    let depth = undo_depth(&state);
    for ops in [
        json!([
            {"set":"scene/street/node/hero-plane/position/x","value":9},
            {"set":"scene/street/node/hero-plane/position","value":{"x":3,"y":0,"z":0}}]),
        json!([
            {"set":"camera/wide/start/position/x","value":9},
            {"set":"camera/wide/start/position","value":{"x":0,"y":2,"z":10}}]),
        // A -> B -> A on the start: the first write converts inside the batch, the answer does not.
        json!([
            {"set":"camera/wide/start/position","value":{"x":1,"y":2,"z":10}},
            {"set":"camera/wide/start/position","value":{"x":0,"y":2,"z":10}}]),
    ] {
        let noop = ok(
            &mut state,
            json!({"op":"scene.edit","scene":"street","ops":ops}),
        );
        assert_eq!(noop["changed"], false, "{ops}: {noop}");
        assert_eq!(noop["noop"], true, "{ops}");
        assert!(noop.get("conversions").is_none(), "{ops}: {noop}");
        assert_eq!(undo_depth(&state), depth, "{ops}");
        assert_eq!(document(&state), before, "{ops}");
    }
    let dry = ok(
        &mut state,
        json!({"op":"scene.edit","scene":"street","dry_run":true,"ops":[
            {"set":"camera/wide/start/position","value":{"x":1,"y":2,"z":10}},
            {"set":"camera/wide/start/position","value":{"x":0,"y":2,"z":10}}]}),
    );
    assert_eq!(dry["would_change"], false);
    assert!(dry.get("conversions").is_none(), "{dry}");
}

#[test]
fn a_render_order_beyond_a_number_is_invalid_value_over_json_lines() {
    let mut state = session();
    let before = document(&state);
    for value in [json!(1e16), json!(18_446_744_073_709_551_615_u64)] {
        let error = err(
            &mut state,
            json!({"op":"scene.edit","scene":"street","ops":[
                {"set":"scene/street/node/hero-plane/renderOrder","value":value}]}),
        );
        assert_eq!(error.code, "invalid_value", "{value}: {error:?}");
    }
    let error = err(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[
            {"set":"scene/street/node/hero-plane/renderOrder","value":0.5}]}),
    );
    assert_eq!(error.code, "invalid_type");
    assert_eq!(document(&state), before);
}

fn requests() -> Vec<String> {
    [
        json!({"op":"scene.attach","id":"street","bundle":bundle()}),
        json!({"op":"scene.properties","scene":"street","prefix":"camera/","components":true}),
        json!({"op":"scene.edit","scene":"street","ops":[
            {"set":"scene/street/node/prop-plane/renderOrder","value":5},
            {"set":"camera/wide/start/position/z","value":9}]}),
        json!({"op":"scene.get","scene":"street","property":"camera/wide/start/position/z"}),
        json!({"op":"history.undo"}),
        json!({"op":"scene.get","scene":"street","property":"scene/street/node/prop-plane/renderOrder"}),
    ]
    .iter()
    .map(Value::to_string)
    .collect()
}

fn lines(bytes: &[u8]) -> Vec<Value> {
    String::from_utf8(bytes.to_vec())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn the_headless_executable_answers_exactly_like_the_library() {
    let input = requests().join("\n");
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
    assert!(output.status.success());
    assert_eq!(lines(&output.stdout), lines(&library));

    let answers = lines(&output.stdout);
    assert!(
        answers.iter().all(|answer| answer["ok"] == true),
        "{answers:?}"
    );
    assert_eq!(answers[2]["result"]["conversions"][0]["clip"], "wide");
    assert_eq!(answers[3]["result"]["value"], 9);
    assert_eq!(answers[5]["result"]["value"], 0);
}
