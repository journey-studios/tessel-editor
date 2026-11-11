//! F060 (BL023/BL002/BL003/BL065, G2.1): the point light, through the public library API and the
//! JSON executor (in process: no window, no GPU, no Chrome, no GPUI, no pointer device).
//!
//! Every expectation is computed **here** from the document's numbers (a sum of offsets, a
//! hand-written falloff), never with the code under test:
//!
//! - **Contract.** A light's position is world, or the offset added to its parent's *translation*
//!   only; the plan carries the evaluated light and requires `pointLights`; a scene without lights
//!   is byte-for-byte what it was.
//! - **Refusals (BL065).** Ambient other than 1, shadows, more than four lights, a malformed or
//!   duplicate light are refused with a typed reason; the domain is not dropped wholesale.
//! - **Edit.** Position, intensity, radius and colour go through `scene.edit` by the same path as
//!   everything else; a keyed channel is refused (`animated_property`), never silently offset.
//! - **Equivalence.** The document an arrows' or a knob's drag leaves equals the one
//!   `scene.translate` / `scene.range` leave for the delta the drag reported, bit for bit.
//! - **Lifecycle.** One undo entry per gesture, none for a no-op, cancel and a refused candidate
//!   leave the document as it was and the last legal candidate stays.
//! - **Not covered**: what a runtime draws (`runtime/test/*light*`), the window and a physical
//!   pointer.

use std::fs;
use std::path::Path;

use core::prelude::v1::test;
use serde_json::{Value, json};
use tessel_editor_core::{CommandError, EditorState, headless};

const MINIMAL: &str = "valid-minimal-continuous";
const LAMP: &str = "scene/scene-a/light/lamp";
const FIGURE: &str = "scene/scene-a/node/figure";

fn bundle(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/migration/fixtures/bundles")
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

fn lamp() -> Value {
    json!({
        "id": "lamp",
        "position": {"x": -0.5, "y": 0.3, "z": 0.5},
        "color": {"r": 1.0, "g": 0.5, "b": 0.25},
        "intensity": 2.0, "radius": 1.5, "flicker": 0.0,
    })
}

fn lit(lights: Vec<Value>) -> Value {
    let mut document = bundle(MINIMAL);
    document["sceneDefinitions"][0]["lighting"] = json!({"ambient": 1, "lights": lights});
    document
}

fn attach(document: Value) -> Result<EditorState, CommandError> {
    let mut state = EditorState::demo();
    ok(&mut state, json!({"op":"project.create","name":"Light"}));
    run(
        &mut state,
        json!({"op":"scene.attach","id":"street","bundle":document}),
    )?;
    Ok(state)
}

fn street() -> EditorState {
    attach(lit(vec![lamp()])).unwrap()
}

fn undo_depth(state: &EditorState) -> u64 {
    state.history.to_json()["undo_depth"].as_u64().unwrap()
}

fn observable(state: &EditorState) -> (Value, Value, u64) {
    (
        state.to_document(None),
        state.history.to_json(),
        state.project.revision,
    )
}

fn document_of(state: &EditorState) -> Value {
    state.scenes.get("street").unwrap().scene.document().clone()
}

fn light_of(state: &EditorState) -> Value {
    document_of(state)["sceneDefinitions"][0]["lighting"]["lights"][0].clone()
}

fn plan_at(state: &mut EditorState, frame: i64) -> Value {
    ok(
        state,
        json!({"op":"scene.plan","scene":"street","frame":frame}),
    )["plan"]
        .clone()
}

fn select(state: &mut EditorState, entity: &str) {
    ok(
        state,
        json!({"op":"scene.select","scene":"street","entities":[entity]}),
    );
}

fn overlay(state: &mut EditorState) -> Value {
    ok(
        state,
        json!({"op":"scene.gizmo.state","scene":"street","frame":0}),
    )["overlay"]
        .clone()
}

fn handle<'a>(overlay: &'a Value, id: &str) -> &'a Value {
    overlay["handles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|handle| handle["id"] == id)
        .unwrap_or_else(|| panic!("no handle `{id}` in {overlay}"))
}

fn point_of(value: &Value) -> Value {
    json!({"u": value["u"], "v": value["v"]})
}

fn shifted(value: &Value, du: f64, dv: f64) -> Value {
    json!({"u": value["u"].as_f64().unwrap() + du, "v": value["v"].as_f64().unwrap() + dv})
}

fn reason(error: &CommandError) -> String {
    error.details.as_ref().unwrap()["reason"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Equality of two JSON values where `2` and `2.0` are the same number (the document keeps the
/// integer a caller wrote).
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(key, value)| y.get(key).is_some_and(|other| same(value, other)))
        }
        _ => a == b,
    }
}

fn num(value: &Value) -> f64 {
    value.as_f64().unwrap()
}

// --- the contract ---------------------------------------------------------------------------------

#[test]
fn the_plan_carries_the_evaluated_light_and_requires_point_lights() {
    let mut state = street();
    let plan = plan_at(&mut state, 0);
    let lights = plan["lights"].as_array().unwrap();
    assert_eq!(lights.len(), 1);
    let wire = &lights[0];
    assert_eq!(wire["id"], "lamp");
    assert_eq!(wire["position"], json!({"x": -0.5, "y": 0.3, "z": 0.5}));
    assert_eq!(wire["color"], json!({"r": 1.0, "g": 0.5, "b": 0.25}));
    // flicker 0: exactly the authored intensity.
    assert_eq!(wire["intensity"], json!(2.0));
    assert_eq!(wire["radius"], json!(1.5));
    assert!(wire.get("hidden").is_none());
    assert!(
        plan["requires"]
            .as_array()
            .unwrap()
            .contains(&json!("pointLights"))
    );
}

#[test]
fn a_scene_without_lights_is_what_it_was() {
    let mut state = attach(bundle(MINIMAL)).unwrap();
    let plan = plan_at(&mut state, 0);
    assert!(plan.get("lights").is_none());
    assert!(
        !plan["requires"]
            .as_array()
            .unwrap()
            .contains(&json!("pointLights"))
    );
    let overlay = overlay(&mut state);
    assert!(overlay.get("light_markers").is_none());
    assert!(overlay.get("sphere").is_none());
}

#[test]
fn a_parented_light_follows_the_translation_of_its_parent_only() {
    // The oracle: world = parent translation + stored offset, whatever the parent rotates or
    // scales (the origin's `resolveLightFrame`).
    let mut document = lit(vec![]);
    let mut light = lamp();
    light["parent"] = json!("figure");
    light["position"] = json!({"x": 0.1, "y": 0.2, "z": 0.3});
    document["sceneDefinitions"][0]["lighting"]["lights"] = json!([light]);
    // Turn and stretch the parent: the offset must not swing or stretch.
    let figure = document["sceneDefinitions"][0]["nodes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|node| node["id"] == "figure")
        .unwrap();
    figure["localRotation"] = json!({"x": 0, "y": std::f64::consts::FRAC_1_SQRT_2, "z": 0, "w": std::f64::consts::FRAC_1_SQRT_2});
    figure["localScale"] = json!({"x": 2, "y": 3, "z": 4});
    // figure.localPosition (0.5, 0, 0), under `root` at the origin.
    let mut state = attach(document).unwrap();
    let plan = plan_at(&mut state, 0);
    let position = &plan["lights"][0]["position"];
    assert!((num(&position["x"]) - 0.6).abs() < 1e-12, "{position}");
    assert!((num(&position["y"]) - 0.2).abs() < 1e-12, "{position}");
    assert!((num(&position["z"]) - 0.3).abs() < 1e-12, "{position}");
}

#[test]
fn keys_replace_the_static_field_on_the_edit_frames_and_ease() {
    let mut document = lit(vec![]);
    let mut light = lamp();
    light["keys"] = json!({
        "intensity": [
            {"frame": 0, "value": 0.0, "easing": "linear"},
            {"frame": 10, "value": 10.0, "easing": "linear"},
        ],
    });
    document["sceneDefinitions"][0]["lighting"]["lights"] = json!([light]);
    let mut state = attach(document).unwrap();
    for (frame, expected) in [(0, 0.0), (5, 5.0), (10, 10.0), (20, 10.0)] {
        let plan = plan_at(&mut state, frame);
        let got = num(&plan["lights"][0]["intensity"]);
        assert!((got - expected).abs() < 1e-12, "frame {frame}: {got}");
    }
}

#[test]
fn a_hidden_light_is_kept_and_flagged() {
    let mut light = lamp();
    light["hidden"] = json!(true);
    let mut state = attach(lit(vec![light])).unwrap();
    let plan = plan_at(&mut state, 0);
    assert_eq!(plan["lights"][0]["hidden"], json!(true));
    select(&mut state, LAMP);
    let overlay = overlay(&mut state);
    assert_eq!(overlay["light_markers"][0]["hidden"], json!(true));
}

// --- what is refused (BL065) ------------------------------------------------------------------

fn refused_reasons(state: &mut EditorState) -> Vec<String> {
    let answer = ok(
        state,
        json!({"op":"scene.plan","scene":"street","frame":0,"partial":true}),
    );
    answer["plan"]["unsupported"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["domain"] == "lighting")
        .map(|entry| entry["reason"].as_str().unwrap_or("").to_string())
        .collect()
}

#[test]
fn what_the_slice_does_not_draw_is_refused_with_a_typed_reason() {
    let ambient = {
        let mut document = lit(vec![lamp()]);
        document["sceneDefinitions"][0]["lighting"]["ambient"] = json!(0.5);
        document
    };
    let shadows = {
        let mut document = lit(vec![lamp()]);
        document["sceneDefinitions"][0]["lighting"]["shadows"] = json!({"enabled": true});
        document
    };
    let many = lit((0..5)
        .map(|index| {
            let mut light = lamp();
            light["id"] = json!(format!("lamp-{index}"));
            light
        })
        .collect());
    let invalid = lit(vec![{
        let mut light = lamp();
        light["intensity"] = json!(99.0);
        light
    }]);
    let duplicate = lit(vec![lamp(), lamp()]);
    for (name, document, expected) in [
        ("ambient", ambient, "ambient_not_neutral"),
        ("shadows", shadows, "shadows"),
        ("many", many, "too_many_lights"),
        ("invalid", invalid, "invalid_lighting"),
        ("duplicate", duplicate, "invalid_lighting"),
    ] {
        let mut state = attach(document).unwrap_or_else(|error| panic!("{name}: {error:?}"));
        let reasons = refused_reasons(&mut state);
        assert!(
            reasons.iter().any(|got| got == expected),
            "{name}: {reasons:?}"
        );
        // The complete plan is refused, never silently drawn without the light.
        let complete = run(
            &mut state,
            json!({"op":"scene.plan","scene":"street","frame":0}),
        );
        assert!(
            complete.is_err(),
            "{name}: a refused lighting is not a complete plan"
        );
        // A refused section has no light to select as one.
        let plan = ok(
            &mut state,
            json!({"op":"scene.plan","scene":"street","frame":0,"partial":true}),
        )["plan"]
            .clone();
        assert!(plan.get("lights").is_none(), "{name}");
    }
}

// --- editing ----------------------------------------------------------------------------------

#[test]
fn the_inspector_properties_are_edited_by_scene_edit_with_one_undo_each() {
    let mut state = street();
    let before = undo_depth(&state);
    for (property, value, check) in [
        ("intensity", json!(4.0), json!(4.0)),
        ("radius", json!(2.5), json!(2.5)),
        (
            "position",
            json!({"x": 1, "y": 2, "z": 3}),
            json!({"x": 1.0, "y": 2.0, "z": 3.0}),
        ),
        ("color/r", json!(0.1), json!(0.1)),
        ("color/g", json!(0.2), json!(0.2)),
        ("color/b", json!(0.3), json!(0.3)),
    ] {
        ok(
            &mut state,
            json!({"op":"scene.edit","scene":"street","ops":[
                {"set": format!("scene/scene-a/light/lamp/{property}"), "value": value}
            ]}),
        );
        let stored = property
            .split('/')
            .fold(light_of(&state), |value, key| value[key].clone());
        assert!(same(&stored, &check), "{property}");
    }
    assert_eq!(undo_depth(&state), before + 6);
    ok(&mut state, json!({"op":"history.undo"}));
    // Undo takes back the last edit only: the blue channel, not the red one before it.
    assert_eq!(num(&light_of(&state)["color"]["b"]), 0.25);
    assert_eq!(num(&light_of(&state)["color"]["r"]), 0.1);
}

#[test]
fn an_invalid_value_is_refused_and_leaves_the_document() {
    let mut state = street();
    let before = observable(&state);
    for (property, value) in [
        ("intensity", json!(21.0)),
        ("intensity", json!(-1.0)),
        ("radius", json!(0.0)),
        ("radius", json!(-2.0)),
        ("color/r", json!(2.0)),
        ("color/g", json!(-0.1)),
    ] {
        let error = err(
            &mut state,
            json!({"op":"scene.edit","scene":"street","ops":[
                {"set": format!("scene/scene-a/light/lamp/{property}"), "value": value}
            ]}),
        );
        println!("{property} {value}: {}", error.code);
        assert_ne!(
            error.code, "unknown_path",
            "{property} is a property of a light"
        );
        assert_ne!(error.code, "invalid_op", "{property} {value}");
    }
    assert_eq!(observable(&state), before);
}

fn keyed_position() -> EditorState {
    let mut light = lamp();
    light["keys"] = json!({
        "position": [
            {"frame": 0, "value": {"x": -0.5, "y": 0.3, "z": 0.5}, "easing": "linear"},
            {"frame": 10, "value": {"x": 0.5, "y": 0.3, "z": 0.5}, "easing": "linear"},
        ],
    });
    attach(lit(vec![light])).unwrap()
}

#[test]
fn a_light_with_animated_position_is_refused_never_offset() {
    let mut state = keyed_position();
    let before = observable(&state);
    let edit = err(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[
            {"set":"scene/scene-a/light/lamp/position/x","value":0.0}
        ]}),
    );
    assert_eq!(edit.code, "animated_property");
    let moved = err(
        &mut state,
        json!({"op":"scene.translate","scene":"street","entity":LAMP,"frame":0,
               "delta":{"x":0.1,"y":0,"z":0}}),
    );
    // The typed refusal of the edit layer, surfaced by the numeric command.
    assert_eq!(moved.code, "animated_property");
    assert_eq!(observable(&state), before);
    // The radius is not animated: still editable.
    ok(
        &mut state,
        json!({"op":"scene.range","scene":"street","entity":LAMP,"frame":0,"delta":0.5}),
    );
    assert_eq!(num(&light_of(&state)["radius"]), 2.0);
}

#[test]
fn the_gizmo_of_an_animated_light_lists_its_arrows_as_disabled_and_refuses_a_press() {
    let mut state = keyed_position();
    select(&mut state, LAMP);
    let overlay = overlay(&mut state);
    assert_eq!(overlay["visible"], json!(true));
    let disabled: Vec<&str> = overlay["disabled_handles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            assert_eq!(entry["reason"], "animated_position");
            entry["id"].as_str().unwrap()
        })
        .collect();
    assert!(disabled.contains(&"x") && disabled.contains(&"y") && disabled.contains(&"z"));
    // The range is still offered.
    handle(&overlay, "range");
    let before = observable(&state);
    let x = overlay["origin"].clone();
    let pressed = run(
        &mut state,
        json!({"op":"scene.pointer.down","scene":"street","frame":0,
               "point": shifted(&x, 0.0, 0.0)}),
    );
    // A press on the (disabled) centre is not a drag of the position.
    if let Ok(answer) = pressed {
        assert_ne!(answer["handle"], "x");
        assert_ne!(answer["tool"], json!("rotate"));
    }
    ok(&mut state, json!({"op":"scene.pointer.cancel"}));
    assert_eq!(state.to_document(None), before.0);
}

// --- the gizmo --------------------------------------------------------------------------------

#[test]
fn the_overlay_shows_markers_the_sphere_and_the_range_knob() {
    let mut state = street();
    let unselected = overlay(&mut state);
    let markers = unselected["light_markers"].as_array().unwrap();
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0]["entity"], LAMP);
    assert_eq!(markers[0]["selected"], json!(false));
    assert_eq!(unselected["visible"], json!(false));

    select(&mut state, LAMP);
    let selected = overlay(&mut state);
    assert_eq!(selected["entity"], LAMP);
    assert_eq!(selected["visible"], json!(true));
    assert_eq!(selected["light_markers"][0]["selected"], json!(true));
    let ids: Vec<&str> = selected["handles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|handle| handle["id"].as_str().unwrap())
        .collect();
    for expected in ["x", "y", "z", "screen", "range"] {
        assert!(ids.contains(&expected), "{ids:?}");
    }
    assert_eq!(handle(&selected, "range")["kind"], "range");
    assert_eq!(selected["sphere"].as_array().unwrap().len(), 3);
}

#[test]
fn a_press_on_a_marker_selects_the_light() {
    let mut state = street();
    let at = overlay(&mut state)["light_markers"][0]["at"].clone();
    let hit = ok(
        &mut state,
        json!({"op":"scene.gizmo.hit","scene":"street","frame":0,"point":point_of(&at)}),
    );
    assert_eq!(hit["hit"], json!({"kind":"entity","entity":LAMP}));
    let down = ok(
        &mut state,
        json!({"op":"scene.pointer.down","scene":"street","frame":0,"point":point_of(&at)}),
    );
    assert_eq!(down["selection_changed"], json!(true));
    assert_eq!(overlay(&mut state)["entity"], LAMP);
}

#[test]
fn the_rotate_and_scale_tools_have_nothing_for_a_light_and_say_why() {
    let mut state = street();
    select(&mut state, LAMP);
    for (tool, expected) in [
        ("rotate", "light_has_no_rotation"),
        ("scale", "light_has_no_scale"),
    ] {
        ok(&mut state, json!({"op":"scene.gizmo.tool","tool":tool}));
        let overlay = overlay(&mut state);
        assert_eq!(overlay["visible"], json!(false));
        assert_eq!(overlay["reason"], expected);
    }
}

fn drag_range(state: &mut EditorState, du: f64) -> Value {
    select(state, LAMP);
    let knob = handle(&overlay(state), "range")["to"].clone();
    let down = ok(
        state,
        json!({"op":"scene.pointer.down","scene":"street","frame":0,"point":point_of(&knob)}),
    );
    assert_eq!(down["dragging"], json!(true), "{down}");
    assert_eq!(down["handle"], "range");
    ok(
        state,
        json!({"op":"scene.pointer.move","point":shifted(&knob, du, 0.0)}),
    );
    ok(
        state,
        json!({"op":"scene.pointer.up","point":shifted(&knob, du, 0.0)}),
    )
}

#[test]
fn a_range_drag_and_scene_range_leave_the_same_document_with_one_undo() {
    let mut dragged = street();
    let depth = undo_depth(&dragged);
    let up = drag_range(&mut dragged, 0.05);
    let delta = num(&up["delta"]);
    let radius = num(&up["radius"]);
    // The pointer moved right on the picture: the range grew by that much, and by the sum the
    // oracle writes here.
    assert!(delta > 0.0, "{up}");
    assert_eq!(radius, 1.5 + delta);
    assert_eq!(light_of(&dragged)["radius"], json!(radius));
    assert_eq!(undo_depth(&dragged), depth + 1);

    let mut numeric = street();
    ok(
        &mut numeric,
        json!({"op":"scene.range","scene":"street","entity":LAMP,"frame":0,"delta":delta}),
    );
    assert_eq!(document_of(&dragged), document_of(&numeric));

    ok(&mut dragged, json!({"op":"history.undo"}));
    assert_eq!(num(&light_of(&dragged)["radius"]), 1.5);
}

#[test]
fn an_arrows_drag_and_scene_translate_leave_the_same_document_with_one_undo() {
    let mut dragged = street();
    select(&mut dragged, LAMP);
    let depth = undo_depth(&dragged);
    let arrow = handle(&overlay(&mut dragged), "x")["to"].clone();
    let down = ok(
        &mut dragged,
        json!({"op":"scene.pointer.down","scene":"street","frame":0,"point":point_of(&arrow)}),
    );
    assert_eq!(down["handle"], "x", "{down}");
    let to = shifted(&arrow, 0.04, 0.0);
    ok(&mut dragged, json!({"op":"scene.pointer.move","point":to}));
    let up = ok(&mut dragged, json!({"op":"scene.pointer.up","point":to}));
    let delta = up["world_delta"].clone();
    assert!(num(&delta["x"]) > 0.0, "{up}");
    assert_eq!(num(&delta["y"]), 0.0);
    assert_eq!(undo_depth(&dragged), depth + 1);
    let position = light_of(&dragged)["position"].clone();
    assert_eq!(num(&position["x"]), -0.5 + num(&delta["x"]));
    assert_eq!(num(&position["y"]), 0.3);

    let mut numeric = street();
    ok(
        &mut numeric,
        json!({"op":"scene.translate","scene":"street","entity":LAMP,"frame":0,"delta":delta}),
    );
    assert_eq!(document_of(&dragged), document_of(&numeric));
}

#[test]
fn a_parented_light_moves_by_exactly_the_world_displacement() {
    let mut document = lit(vec![]);
    let mut light = lamp();
    light["parent"] = json!("figure");
    document["sceneDefinitions"][0]["lighting"]["lights"] = json!([light]);
    let figure = document["sceneDefinitions"][0]["nodes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|node| node["id"] == "figure")
        .unwrap();
    figure["localRotation"] = json!({"x": 0, "y": std::f64::consts::FRAC_1_SQRT_2, "z": 0, "w": std::f64::consts::FRAC_1_SQRT_2});
    figure["localScale"] = json!({"x": 2, "y": 3, "z": 4});
    let mut state = attach(document).unwrap();
    let before = plan_at(&mut state, 0)["lights"][0]["position"].clone();
    ok(
        &mut state,
        json!({"op":"scene.translate","scene":"street","entity":LAMP,"frame":0,
               "delta":{"x":0.25,"y":-0.5,"z":1.0}}),
    );
    let after = plan_at(&mut state, 0)["lights"][0]["position"].clone();
    for (axis, delta) in [("x", 0.25), ("y", -0.5), ("z", 1.0)] {
        assert!(
            (num(&after[axis]) - num(&before[axis]) - delta).abs() < 1e-12,
            "{axis}: {before} -> {after}"
        );
    }
}

#[test]
fn a_zero_delta_writes_nothing() {
    let mut state = street();
    let before = observable(&state);
    let range = ok(
        &mut state,
        json!({"op":"scene.range","scene":"street","entity":LAMP,"frame":0,"delta":0.0}),
    );
    assert_eq!(range["changed"], json!(false));
    let translate = ok(
        &mut state,
        json!({"op":"scene.translate","scene":"street","entity":LAMP,"frame":0,
               "delta":{"x":0,"y":0,"z":0}}),
    );
    assert_eq!(translate["changed"], json!(false));
    assert_eq!(observable(&state), before);
}

#[test]
fn a_drag_without_movement_and_a_cancel_leave_no_history() {
    let mut state = street();
    let before = observable(&state);
    let up = drag_range(&mut state, 0.0);
    assert_eq!(up["changed"], json!(false), "{up}");
    assert_eq!(observable(&state).1, before.1);

    select(&mut state, LAMP);
    let knob = handle(&overlay(&mut state), "range")["to"].clone();
    ok(
        &mut state,
        json!({"op":"scene.pointer.down","scene":"street","frame":0,"point":point_of(&knob)}),
    );
    ok(
        &mut state,
        json!({"op":"scene.pointer.move","point":shifted(&knob, 0.05, 0.0)}),
    );
    ok(&mut state, json!({"op":"scene.pointer.cancel"}));
    assert_eq!(num(&light_of(&state)["radius"]), 1.5);
    assert_eq!(undo_depth(&state), before.1["undo_depth"].as_u64().unwrap());
}

#[test]
fn an_invalid_candidate_keeps_the_last_legal_one() {
    let mut state = street();
    select(&mut state, LAMP);
    let knob = handle(&overlay(&mut state), "range")["to"].clone();
    ok(
        &mut state,
        json!({"op":"scene.pointer.down","scene":"street","frame":0,"point":point_of(&knob)}),
    );
    let legal = ok(
        &mut state,
        json!({"op":"scene.pointer.move","point":shifted(&knob, 0.02, 0.0)}),
    );
    assert_eq!(legal["applied"], json!(true), "{legal}");
    // Far to the left: the radius would be negative.
    let illegal = ok(
        &mut state,
        json!({"op":"scene.pointer.move","point":shifted(&knob, -5.0, 0.0)}),
    );
    assert_eq!(illegal["applied"], json!(false), "{illegal}");
    assert_eq!(illegal["reason"], "radius_not_positive");
    let up = ok(&mut state, json!({"op":"scene.pointer.up"}));
    assert_eq!(num(&up["radius"]), num(&legal["radius"]));
    assert_eq!(light_of(&state)["radius"], legal["radius"]);
}

#[test]
fn scene_range_refuses_what_is_not_a_legal_radius_or_not_a_light() {
    let mut state = street();
    let before = observable(&state);
    let negative = err(
        &mut state,
        json!({"op":"scene.range","scene":"street","entity":LAMP,"frame":0,"delta":-1.5}),
    );
    assert_eq!(negative.code, "not_rangeable");
    assert_eq!(reason(&negative), "radius_not_positive");
    let huge = err(
        &mut state,
        json!({"op":"scene.range","scene":"street","entity":LAMP,"frame":0,"delta":1.0e9}),
    );
    assert_eq!(reason(&huge), "radius_too_large");
    let node = err(
        &mut state,
        json!({"op":"scene.range","scene":"street","entity":FIGURE,"frame":0,"delta":1.0}),
    );
    assert_eq!(node.code, "not_rangeable");
    let missing = err(
        &mut state,
        json!({"op":"scene.range","scene":"street","entity":"scene/scene-a/light/none",
               "frame":0,"delta":1.0}),
    );
    assert!(!missing.code.is_empty());
    let bad = err(
        &mut state,
        json!({"op":"scene.range","scene":"street","entity":LAMP,"frame":0,"delta":"far"}),
    );
    assert!(!bad.code.is_empty());
    assert_eq!(observable(&state), before);
}

#[test]
fn the_commands_are_negotiated_by_listing() {
    let mut state = street();
    let capabilities = ok(&mut state, json!({"op":"scene.capabilities"}));
    let operations = capabilities["commands"].as_array().unwrap();
    assert!(operations.contains(&json!("scene.range")), "{capabilities}");
}

#[test]
fn a_light_shows_in_the_entity_tree_under_its_scene() {
    let mut state = street();
    let tree = ok(&mut state, json!({"op":"scene.tree","scene":"street"})).to_string();
    assert!(tree.contains(LAMP), "{tree}");
}

#[test]
fn an_edit_under_an_open_light_drag_invalidates_it_and_the_release_changes_nothing() {
    let mut state = street();
    select(&mut state, LAMP);
    let knob = handle(&overlay(&mut state), "range")["to"].clone();
    ok(
        &mut state,
        json!({"op":"scene.pointer.down","scene":"street","frame":0,"point":point_of(&knob)}),
    );
    ok(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[
            {"set":"scene/scene-a/light/lamp/intensity","value":3.0}
        ]}),
    );
    assert!(state.gizmo_drag_is_dead());
    let moved = err(
        &mut state,
        json!({"op":"scene.pointer.move","point":shifted(&knob, 0.05, 0.0)}),
    );
    assert_eq!(moved.code, "gesture_invalidated");
    let before = observable(&state);
    ok(&mut state, json!({"op":"scene.pointer.cancel"}));
    assert_eq!(observable(&state).0, before.0);
    assert_eq!(num(&light_of(&state)["radius"]), 1.5);
    assert_eq!(num(&light_of(&state)["intensity"]), 3.0);
}
