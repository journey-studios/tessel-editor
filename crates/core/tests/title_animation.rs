use serde_json::{Value, json};
use tessel_editor_core::{EditorState, headless};

fn run(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn project() -> EditorState {
    let mut state = EditorState::demo();
    run(
        &mut state,
        json!({"op":"project.create","name":"Animated titles","rate":{"num":30000,"den":1001},
            "width":1920,"height":1080,"length":120}),
    );
    state
}

fn add_title(state: &mut EditorState, duration: i64) {
    run(
        state,
        json!({"op":"track.add","id":"titles","name":"Titles","kind":"video"}),
    );
    run(
        state,
        json!({"op":"clip.add","id":"title","track":"titles","start":12,"duration":duration,
            "text":{"content":"Sessão\nAção","font":"sans","align":"left","animation":"rise",
                "animation_ms":500,"size":10,"color":"#e0c0ff"},
            "properties":{"position_x":-30,"position_y":-20,"opacity":75}}),
    );
}

fn title_at(state: &mut EditorState, frame: i64) -> Value {
    run(state, json!({"op":"plan","frame":frame}))["layers"][0].clone()
}

fn edit(state: &mut EditorState, field: &str, value: Value) -> Value {
    run(
        state,
        json!({"op":"set","target":"clip_text:title","field":field,"value":value}),
    )
}

#[test]
fn title_animation_uses_the_clip_clock_at_fractional_rate_and_survives_reopen() {
    let mut state = project();
    add_title(&mut state, 60);
    let first = title_at(&mut state, 12);
    assert_eq!(first["opacity"], 0.0);
    assert_eq!(first["text"]["reveal"], 0.0);
    assert!(first["transform"]["y"].as_f64().unwrap() > -20.0);

    // At 30000/1001 fps the 15th offset frame begins after 500 ms. The preceding
    // frame is still animated, even when reached by a backward seek.
    let settled = title_at(&mut state, 27);
    assert_eq!(settled["opacity"], 0.75);
    assert_eq!(settled["transform"]["x"], -30.0);
    assert_eq!(settled["transform"]["y"], -20.0);
    assert!(settled["text"].get("reveal").is_none());
    assert!(settled["text"].get("lines").is_none());
    let late = title_at(&mut state, 26);
    assert!(late["opacity"].as_f64().unwrap() < 0.75);
    let middle = title_at(&mut state, 17);
    assert!(middle["opacity"].as_f64().unwrap() < late["opacity"].as_f64().unwrap());
    assert_eq!(title_at(&mut state, 12), first);

    let mut reopened = EditorState::from_document(&state.to_document(None), None).unwrap();
    assert_eq!(title_at(&mut reopened, 17), middle);
    assert_eq!(title_at(&mut reopened, 27), settled);
}

#[test]
fn changing_one_title_field_preserves_the_other_fields_and_is_undoable() {
    let mut state = project();
    add_title(&mut state, 60);
    edit(&mut state, "content", json!("Nova sessão"));
    let rise = title_at(&mut state, 17);
    assert_eq!(rise["text"]["font"], "sans");
    assert_eq!(rise["text"]["align"], "left");
    assert_eq!(rise["text"]["animation"], "rise");
    assert_eq!(rise["text"]["animation_ms"], 500);

    edit(&mut state, "animation", json!("pop"));
    let pop = title_at(&mut state, 12);
    assert!(pop["text"].get("reveal").is_none());
    assert!(pop["transform"]["scale"].as_f64().unwrap() < 1.0);
    assert!(
        title_at(&mut state, 21)["transform"]["scale"]
            .as_f64()
            .unwrap()
            > 1.0
    );
    run(&mut state, json!({"op":"history.undo"}));
    assert_eq!(title_at(&mut state, 17), rise);
    run(&mut state, json!({"op":"history.redo"}));
    assert_eq!(title_at(&mut state, 12), pop);
    edit(&mut state, "animation", json!("fade"));
    assert_eq!(title_at(&mut state, 12)["opacity"], 0.0);
    assert_eq!(title_at(&mut state, 17)["transform"]["scale"], 1.0);
    edit(&mut state, "animation", json!("none"));
    assert_eq!(title_at(&mut state, 12)["opacity"], 0.75);
}

#[test]
fn invalid_styles_are_rejected_before_mutation() {
    let mut state = project();
    add_title(&mut state, 60);
    let before = state.to_document(None);
    let history = state.history.to_json();
    for (field, value) in [
        ("animation", json!("blur")),
        ("font", json!("system-unknown")),
        ("animation_ms", json!(0)),
        ("animation_ms", json!(3001)),
        ("align", json!("right")),
    ] {
        let error = headless::execute(
            &mut state,
            &json!({"op":"set","target":"clip_text:title","field":field,"value":value}),
        )
        .unwrap_err();
        assert_eq!(error.code, "invalid_value", "{field}");
    }
    assert_eq!(state.to_document(None), before);
    assert_eq!(state.history.to_json(), history);
}

#[test]
fn one_frame_titles_are_visible_and_short_titles_settle_by_their_midpoint() {
    for duration in [1, 4] {
        let mut state = project();
        add_title(&mut state, duration);
        let last = title_at(&mut state, 12 + duration / 2);
        assert_eq!(last["opacity"], 0.75);
        assert!(last["text"].get("reveal").is_none());
    }
}

#[test]
fn one_add_title_action_undoes_both_the_clip_and_the_created_track() {
    let mut state = project();
    run(&mut state, json!({"op":"seek","frame":15}));
    let added = run(
        &mut state,
        json!({"op":"invoke","target":"timeline.add_title","action":"invoke"}),
    );
    assert_eq!(state.timeline.tracks.len(), 1);
    assert_eq!(added["frame"], 15);
    assert_eq!(added["duration"], 90);
    assert_eq!(title_at(&mut state, 15)["text"]["font"], "sans");
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
