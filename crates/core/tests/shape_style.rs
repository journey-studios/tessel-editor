//! F068 (BL169.6 step 1B): ellipse, path, contour, gradients, shadow and the rounded image mask
//! through the headless contract: create, edit, plan, save, reopen, undo, and the reading of what
//! step 1A wrote.
use std::fs;

use serde_json::{Value, json};
use tessel_editor_core::fixtures::quadrants_png;
use tessel_editor_core::{EditorState, headless};

fn run(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn refused(state: &mut EditorState, request: Value) -> tessel_editor_core::state::CommandError {
    headless::execute(state, &request).expect_err("the request must be refused")
}

fn project() -> EditorState {
    let mut state = EditorState::demo();
    run(
        &mut state,
        json!({"op":"project.create","name":"Styles","rate":{"num":30,"den":1},
            "width":1920,"height":1080,"length":120}),
    );
    run(
        &mut state,
        json!({"op":"track.add","id":"overlay","name":"Overlay","kind":"video"}),
    );
    state
}

fn layers(state: &mut EditorState, frame: i64) -> Vec<Value> {
    run(state, json!({"op":"plan","frame":frame}))["layers"]
        .as_array()
        .unwrap()
        .clone()
}

fn edit(state: &mut EditorState, clip: &str, field: &str, value: Value) -> Value {
    run(
        state,
        json!({"op":"set","target":format!("clip_shape:{clip}"),"field":field,"value":value}),
    )
}

fn add_halo(state: &mut EditorState) {
    run(
        state,
        json!({"op":"clip.add","id":"halo","track":"overlay","start":0,"duration":60,
            "shape":{"kind":"ellipse","width":40,"height":40,"fill":"#ffd166",
                "fill_mode":"radial","fill_to":"#ffd16600"}}),
    );
}

fn cover_image(state: &mut EditorState, tag: &str) {
    let dir = std::env::temp_dir().join(format!("tessel-style-{tag}-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("cover.png");
    fs::write(&path, quadrants_png(64, 36)).unwrap();
    run(
        state,
        json!({"op":"asset.add","kind":"still","id":"cover","name":"Cover","path":path}),
    );
    run(
        state,
        json!({"op":"clip.add","id":"cover-clip","track":"overlay","asset":"cover",
            "start":0,"duration":60}),
    );
}

#[test]
fn an_ellipse_with_a_radial_gradient_plans_every_field_and_needs_no_file() {
    let mut state = project();
    add_halo(&mut state);
    let layer = layers(&mut state, 5).remove(0);
    assert_eq!(layer["kind"], "shape");
    assert!(layer["asset"].is_null());
    let shape = &layer["shape"];
    assert_eq!(shape["kind"], "ellipse");
    assert_eq!(shape["fill_mode"], "radial");
    assert_eq!(shape["fill"], "#ffd166");
    assert_eq!(shape["fill_to"], "#ffd16600");
    assert_eq!(
        shape["stroke_width"], 0.0,
        "the defaults travel in the plan"
    );
    assert_eq!(shape["shadow"], "#00000080");
    assert_eq!(shape["path"], "M 50 0 L 100 100 L 0 100 Z");
}

#[test]
fn a_path_with_a_contour_is_edited_through_the_semantic_set_and_stored_canonically() {
    let mut state = project();
    run(
        &mut state,
        json!({"op":"clip.add","id":"arrow","track":"overlay","start":0,"duration":60,
            "shape":{"kind":"path","width":20,"height":20,"fill_mode":"none",
                "path":"M10,50L90 50M60,20 L90,50 L60,80","stroke":"#ffffff","stroke_width":1.5}}),
    );
    let shape = layers(&mut state, 3).remove(0)["shape"].clone();
    assert_eq!(shape["path"], "M 10 50 L 90 50 M 60 20 L 90 50 L 60 80");
    assert_eq!(shape["fill_mode"], "none");
    assert_eq!(shape["stroke_width"], 1.5);

    edit(
        &mut state,
        "arrow",
        "path",
        json!("M 0 100 C 30 0 70 0 100 100"),
    );
    let curved = layers(&mut state, 3).remove(0)["shape"]["path"].clone();
    assert_eq!(curved, "M 0 100 C 30 0 70 0 100 100");
    run(&mut state, json!({"op":"history.undo"}));
    assert_eq!(
        layers(&mut state, 3).remove(0)["shape"]["path"],
        "M 10 50 L 90 50 M 60 20 L 90 50 L 60 80"
    );
}

#[test]
fn each_new_field_is_one_undoable_edit_and_leaves_the_others_alone() {
    let mut state = project();
    add_halo(&mut state);
    let before = layers(&mut state, 5).remove(0)["shape"].clone();
    for (field, value) in [
        ("kind", json!("rect")),
        ("fill_mode", json!("linear")),
        ("fill_to", json!("#102030")),
        ("gradient_angle", json!(45)),
        ("stroke", json!("#112233")),
        ("stroke_width", json!(2)),
        ("shadow", json!("#000000cc")),
        ("shadow_blur", json!(3)),
        ("shadow_x", json!(-1)),
        ("shadow_y", json!(2)),
    ] {
        let depth = state.history.to_json()["undo_depth"].as_i64().unwrap();
        let outcome = edit(&mut state, "halo", field, value.clone());
        assert_eq!(outcome["changed"], true, "{field}");
        assert_eq!(
            state.history.to_json()["undo_depth"].as_i64().unwrap(),
            depth + 1,
            "{field} is one history entry"
        );
        let shape = layers(&mut state, 5).remove(0)["shape"].clone();
        match value.as_f64() {
            // `45` and `45.0` are the same number; the plan always carries a float.
            Some(number) => assert_eq!(shape[field].as_f64(), Some(number), "{field}"),
            None => assert_eq!(shape[field], value, "{field}"),
        }
        let mut others = shape.clone();
        let mut reference = before.clone();
        for key in [
            "kind",
            "fill_mode",
            "fill_to",
            "gradient_angle",
            "stroke",
            "stroke_width",
            "shadow",
            "shadow_blur",
            "shadow_x",
            "shadow_y",
        ] {
            if let (Some(a), Some(b)) = (others.as_object_mut(), reference.as_object_mut()) {
                a.remove(key);
                b.remove(key);
            }
        }
        assert_eq!(
            others, reference,
            "width, height, fill, radius and path stay"
        );
    }
    for _ in 0..10 {
        run(&mut state, json!({"op":"history.undo"}));
    }
    assert_eq!(layers(&mut state, 5).remove(0)["shape"], before);
    run(&mut state, json!({"op":"history.redo"}));
    assert_eq!(layers(&mut state, 5).remove(0)["shape"]["kind"], "rect");
}

#[test]
fn refused_values_change_nothing_and_name_the_field() {
    let mut state = project();
    add_halo(&mut state);
    let document = state.to_document(None);
    let history = state.history.to_json();
    for (field, value) in [
        ("kind", json!("polygon")),
        ("fill_mode", json!("conic")),
        ("fill", json!("gold")),
        ("fill_to", json!("#12345")),
        ("gradient_angle", json!(400)),
        ("stroke", json!("#12")),
        ("stroke_width", json!(21)),
        ("stroke_width", json!(-0.5)),
        ("shadow", json!("#1234567")),
        ("shadow_blur", json!(25)),
        ("shadow_x", json!(31)),
        ("shadow_y", json!(-31)),
        ("path", json!("M 10")),
        ("path", json!("M 0 0 A 5 5 0 0 0 9 9")),
        ("path", json!("M 0 0 l 5 5")),
        ("path", json!("M 900 0 L 5 5")),
    ] {
        let error = refused(
            &mut state,
            json!({"op":"set","target":"clip_shape:halo","field":field,"value":value}),
        );
        assert_eq!(error.code, "invalid_value", "{field}: {error:?}");
    }
    let error = refused(
        &mut state,
        json!({"op":"clip.shape","clip":"halo","shape":{"stroke_width":50}}),
    );
    assert_eq!(error.code, "invalid_value");
    let error = refused(
        &mut state,
        json!({"op":"clip.shape","clip":"halo","shape":{"glow":1}}),
    );
    assert_eq!(error.code, "unknown_field");
    assert_eq!(state.to_document(None), document);
    assert_eq!(state.history.to_json(), history);
}

#[test]
fn every_new_field_survives_save_and_reopen_with_the_same_plan() {
    let mut state = project();
    add_halo(&mut state);
    for (field, value) in [
        ("stroke", json!("#102030")),
        ("stroke_width", json!(1.25)),
        ("shadow_blur", json!(4)),
        ("shadow_y", json!(1.5)),
        ("gradient_angle", json!(-30)),
        ("path", json!("M 0 0 Q 50 100 100 0 Z")),
    ] {
        edit(&mut state, "halo", field, value);
    }
    let planned = layers(&mut state, 5);
    let document = state.to_document(None);
    let mut reopened = EditorState::from_document(&document, None).unwrap();
    assert_eq!(layers(&mut reopened, 5), planned);
    assert_eq!(reopened.to_document(None), document);
}

#[test]
fn what_step_1a_wrote_is_read_unchanged_and_a_plain_rectangle_is_written_back_the_same_way() {
    let mut state = project();
    run(
        &mut state,
        json!({"op":"clip.add","id":"banner","track":"overlay","start":10,"duration":40,
            "shape":{"kind":"rect","width":50,"height":12.5,"fill":"#102030","radius":20}}),
    );
    let document = state.to_document(None);
    let clip = document.to_string();
    // The five keys 1A wrote, and nothing else: a 1A editor still opens a project that does not use 1B.
    let stored = document["sequences"].to_string().contains("\"fill_mode\"");
    assert!(
        !stored && !clip.contains("stroke_width") && !clip.contains("shadow"),
        "{clip}"
    );

    // A document written by 1A has exactly those keys and reads with the 1B defaults.
    let reopened = EditorState::from_document(&document, None).unwrap();
    let mut reopened = reopened;
    let shape = layers(&mut reopened, 20).remove(0)["shape"].clone();
    assert_eq!(shape["fill_mode"], "solid");
    assert_eq!(shape["stroke_width"], 0.0);
    assert_eq!(shape["radius"], 20.0);
    assert_eq!(
        reopened.to_document(None),
        document,
        "a lossless round trip"
    );
}

#[test]
fn a_shape_that_paints_nothing_is_skipped_with_a_warning_until_it_has_a_contour() {
    let mut state = project();
    add_halo(&mut state);
    edit(&mut state, "halo", "fill_mode", json!("none"));
    let plan = run(&mut state, json!({"op":"plan","frame":5}));
    assert!(plan["layers"].as_array().unwrap().is_empty());
    assert_eq!(plan["warnings"][0]["code"], "shape_draws_nothing");
    edit(&mut state, "halo", "stroke_width", json!(1));
    assert_eq!(layers(&mut state, 5).len(), 1);
}

#[test]
fn a_toolbar_shape_still_defaults_to_the_1a_rectangle() {
    let mut state = project();
    run(&mut state, json!({"op":"shape.add"}));
    let shape = layers(&mut state, 0).remove(0)["shape"].clone();
    assert_eq!(shape["kind"], "rect");
    assert_eq!(shape["fill_mode"], "solid");
    assert_eq!(shape["stroke_width"], 0.0);
}

#[test]
fn the_image_mask_is_a_clip_property_planned_only_while_it_is_on() {
    let mut state = project();
    cover_image(&mut state, "mask");
    assert!(
        layers(&mut state, 5)[0].get("mask").is_none(),
        "off by default"
    );

    run(
        &mut state,
        json!({"op":"clip.property","clip":"cover-clip","property":"mask_shape","value":"rounded"}),
    );
    run(
        &mut state,
        json!({"op":"clip.property","clip":"cover-clip","property":"mask_radius","value":18}),
    );
    let layer = layers(&mut state, 5).remove(0);
    assert_eq!(layer["kind"], "image");
    assert_eq!(layer["mask"], json!({"shape":"rounded","radius":18.0}));

    run(
        &mut state,
        json!({"op":"clip.property","clip":"cover-clip","property":"mask_shape","value":"ellipse"}),
    );
    assert_eq!(layers(&mut state, 5)[0]["mask"]["shape"], "ellipse");

    // Save, reopen, undo.
    let planned = layers(&mut state, 5);
    let document = state.to_document(None);
    let mut reopened = EditorState::from_document(&document, None).unwrap();
    assert_eq!(layers(&mut reopened, 5), planned);
    run(&mut state, json!({"op":"history.undo"}));
    assert_eq!(layers(&mut state, 5)[0]["mask"]["shape"], "rounded");
    run(&mut state, json!({"op":"history.undo"}));
    run(&mut state, json!({"op":"history.undo"}));
    assert!(layers(&mut state, 5)[0].get("mask").is_none());
}

#[test]
fn the_mask_belongs_to_images_and_videos_only_and_refuses_bad_values() {
    let mut state = project();
    cover_image(&mut state, "refusal");
    run(
        &mut state,
        json!({"op":"track.add","id":"above","name":"Above","kind":"video"}),
    );
    run(
        &mut state,
        json!({"op":"clip.add","id":"halo","track":"above","start":0,"duration":60,
            "shape":{"kind":"ellipse"}}),
    );
    let document = state.to_document(None);

    let error = refused(
        &mut state,
        json!({"op":"clip.property","clip":"halo","property":"mask_shape","value":"rounded"}),
    );
    assert_eq!(error.code, "not_applicable", "a shape has its own radius");
    let error = refused(
        &mut state,
        json!({"op":"clip.property","clip":"cover-clip","property":"mask_shape","value":"star"}),
    );
    assert_eq!(error.code, "invalid_value");
    assert_eq!(state.to_document(None), document);

    // A radius beyond the slider is clamped, like every slider.
    let outcome = run(
        &mut state,
        json!({"op":"clip.property","clip":"cover-clip","property":"mask_radius","value":90}),
    );
    assert_eq!(outcome["clamped"], true);
    assert_eq!(outcome["value"], 50.0);
}
