//! F067: the rectangle clip through the headless contract: create, edit, plan, save, reopen, undo.
use serde_json::{Value, json};
use tessel_editor_core::{EditorState, headless};

fn run(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn project() -> EditorState {
    let mut state = EditorState::demo();
    run(
        &mut state,
        json!({"op":"project.create","name":"Shapes","rate":{"num":30,"den":1},
            "width":1920,"height":1080,"length":120}),
    );
    run(
        &mut state,
        json!({"op":"track.add","id":"overlay","name":"Overlay","kind":"video"}),
    );
    state
}

fn add_banner(state: &mut EditorState) {
    run(
        state,
        json!({"op":"clip.add","id":"banner","track":"overlay","start":10,"duration":40,
            "shape":{"kind":"rect","width":50,"height":12.5,"fill":"#102030","radius":20},
            "properties":{"position_y":30,"opacity":80}}),
    );
}

fn layer(state: &mut EditorState, frame: i64) -> Value {
    run(state, json!({"op":"plan","frame":frame}))["layers"][0].clone()
}

fn edit(state: &mut EditorState, field: &str, value: Value) -> Value {
    run(
        state,
        json!({"op":"set","target":"clip_shape:banner","field":field,"value":value}),
    )
}

#[test]
fn a_shape_clip_has_no_file_and_plans_a_shape_layer_with_the_common_transform() {
    let mut state = project();
    add_banner(&mut state);
    let clip = layer(&mut state, 20);
    assert_eq!(clip["kind"], "shape");
    assert!(clip["asset"].is_null(), "a shape names no media: {clip}");
    assert_eq!(clip["shape"]["kind"], "rect");
    assert_eq!(clip["shape"]["width"], 50.0);
    assert_eq!(clip["shape"]["height"], 12.5);
    assert_eq!(clip["shape"]["fill"], "#102030");
    assert_eq!(clip["shape"]["radius"], 20.0);
    assert_eq!(clip["opacity"], 0.8);
    assert_eq!(clip["transform"]["y"], 30.0);

    // Outside the clip nothing is drawn.
    let outside = run(&mut state, json!({"op":"plan","frame":5}));
    assert!(outside["layers"].as_array().unwrap().is_empty());
}

#[test]
fn defaults_fill_what_the_request_leaves_out() {
    let mut state = project();
    run(
        &mut state,
        json!({"op":"clip.add","id":"plain","track":"overlay","start":0,"duration":30,
            "shape":{}}),
    );
    let shape = &layer(&mut state, 3)["shape"];
    assert_eq!(shape["kind"], "rect");
    assert_eq!(shape["width"], 40.0);
    assert_eq!(shape["height"], 20.0);
    assert_eq!(shape["fill"], "#4a90e2");
    assert_eq!(shape["radius"], 0.0);
}

#[test]
fn one_field_edit_keeps_the_others_and_is_one_undoable_entry() {
    let mut state = project();
    add_banner(&mut state);
    let depth = state.history.to_json()["undo_depth"].as_i64().unwrap();
    edit(&mut state, "width", json!(80));
    let wide = layer(&mut state, 20);
    assert_eq!(wide["shape"]["width"], 80.0);
    assert_eq!(wide["shape"]["height"], 12.5);
    assert_eq!(wide["shape"]["fill"], "#102030");
    assert_eq!(
        state.history.to_json()["undo_depth"].as_i64().unwrap(),
        depth + 1
    );

    edit(&mut state, "fill", json!("#ffcc00"));
    assert_eq!(layer(&mut state, 20)["shape"]["fill"], "#ffcc00");
    run(&mut state, json!({"op":"history.undo"}));
    assert_eq!(layer(&mut state, 20)["shape"]["fill"], "#102030");
    assert_eq!(layer(&mut state, 20)["shape"]["width"], 80.0);
    run(&mut state, json!({"op":"history.undo"}));
    assert_eq!(layer(&mut state, 20)["shape"]["width"], 50.0);
    run(&mut state, json!({"op":"history.redo"}));
    assert_eq!(layer(&mut state, 20)["shape"]["width"], 80.0);
}

#[test]
fn replacing_the_whole_shape_goes_through_clip_shape() {
    let mut state = project();
    add_banner(&mut state);
    run(
        &mut state,
        json!({"op":"clip.shape","clip":"banner",
            "shape":{"kind":"rect","width":10,"height":10,"fill":"#ffffff","radius":50}}),
    );
    let shape = layer(&mut state, 20)["shape"].clone();
    assert_eq!(shape["width"], 10.0);
    assert_eq!(shape["radius"], 50.0);
}

#[test]
fn invalid_values_are_rejected_before_any_mutation() {
    let mut state = project();
    add_banner(&mut state);
    let before = state.to_document(None);
    let history = state.history.to_json();
    for (field, value) in [
        ("width", json!(0)),
        ("width", json!(200.5)),
        ("height", json!(-1)),
        ("radius", json!(101)),
        ("fill", json!("red")),
        ("fill", json!("#12345")),
        ("kind", json!("star")),
        ("opacity", json!(1)),
    ] {
        let error = headless::execute(
            &mut state,
            &json!({"op":"set","target":"clip_shape:banner","field":field,"value":value}),
        )
        .unwrap_err();
        assert!(
            ["invalid_value", "unknown_field", "invalid_target"].contains(&error.code),
            "{field}: {error:?}"
        );
    }
    assert_eq!(state.to_document(None), before);
    assert_eq!(state.history.to_json(), history);
}

#[test]
fn a_shape_cannot_be_combined_with_media_or_set_on_another_clip() {
    let mut state = project();
    add_banner(&mut state);
    let before = state.to_document(None);
    let error = headless::execute(
        &mut state,
        &json!({"op":"clip.add","id":"mixed","track":"overlay","start":60,"duration":10,
            "text":{"content":"Hello"},"shape":{}}),
    )
    .unwrap_err();
    assert_eq!(error.code, "invalid_request");
    assert_eq!(state.to_document(None), before);

    run(
        &mut state,
        json!({"op":"clip.add","id":"words","track":"overlay","start":70,"duration":10,
            "text":{"content":"Hello"}}),
    );
    let error = headless::execute(
        &mut state,
        &json!({"op":"clip.shape","clip":"words","shape":{}}),
    )
    .unwrap_err();
    assert_eq!(error.code, "not_applicable");
}

#[test]
fn the_shape_survives_save_and_reopen_and_older_documents_still_read() {
    let mut state = project();
    add_banner(&mut state);
    edit(&mut state, "radius", json!(12.5));
    let planned = layer(&mut state, 20);

    let document = state.to_document(None);
    let mut reopened = EditorState::from_document(&document, None).unwrap();
    assert_eq!(layer(&mut reopened, 20), planned);
    assert_eq!(reopened.to_document(None), document);

    // A project that never had a shape does not gain the key.
    let plain = project();
    assert!(!plain.to_document(None).to_string().contains("\"shape\""));
}

#[test]
fn a_document_with_a_broken_shape_is_refused_with_the_clip_named() {
    let mut state = project();
    add_banner(&mut state);
    let mut document = state.to_document(None);
    let text = document.to_string().replace("#102030", "blue");
    document = serde_json::from_str(&text).unwrap();
    let error = EditorState::from_document(&document, None).unwrap_err();
    assert!(
        format!("{error:?}").contains("banner"),
        "the error names the clip: {error:?}"
    );
}

#[test]
fn splitting_a_shape_keeps_the_shape_on_both_halves() {
    let mut state = project();
    add_banner(&mut state);
    run(
        &mut state,
        json!({"op":"clip.split","clip":"banner","frame":30,"new_id":"banner-right"}),
    );
    let before = layer(&mut state, 20)["shape"].clone();
    let after = layer(&mut state, 40)["shape"].clone();
    assert_eq!(before, after);
    assert_eq!(after["fill"], "#102030");
}

#[test]
fn one_add_shape_action_undoes_both_the_clip_and_the_created_track() {
    let mut state = EditorState::demo();
    run(
        &mut state,
        json!({"op":"project.create","name":"Shapes","rate":{"num":30,"den":1},
            "width":1280,"height":720,"length":300}),
    );
    run(&mut state, json!({"op":"seek","frame":15}));
    let added = run(
        &mut state,
        json!({"op":"invoke","target":"timeline.add_shape","action":"invoke"}),
    );
    assert_eq!(state.timeline.tracks.len(), 1);
    assert_eq!(added["frame"], 15);
    assert_eq!(layer(&mut state, 15)["kind"], "shape");
    assert_eq!(
        state.timeline.selected_clip.unwrap().as_str(),
        added["target"]
            .as_str()
            .unwrap()
            .strip_prefix("clip:")
            .unwrap()
    );
    assert_eq!(state.history.to_json()["undo_depth"], 1);
    run(&mut state, json!({"op":"history.undo"}));
    assert!(state.timeline.tracks.is_empty());
    assert!(state.timeline.clips.is_empty());
}

#[test]
fn the_shape_op_adds_a_clip_like_the_toolbar_button() {
    let mut state = project();
    run(&mut state, json!({"op":"seek","frame":8}));
    run(&mut state, json!({"op":"shape.add"}));
    let clip = layer(&mut state, 8);
    assert_eq!(clip["kind"], "shape");
    assert_eq!(clip["shape"]["fill"], "#4a90e2");
}
