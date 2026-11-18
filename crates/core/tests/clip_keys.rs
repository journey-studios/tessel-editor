//! F073: keyframes of a clip's position, zoom, rotation and opacity through the public library
//! and the JSONL entry (`headless::execute`), without a window or a renderer. A shape clip stands
//! for "any clip drawn from one picture": the key rules are the same for an image, a GIF or a
//! video (the pixels of those are proven in `crates/render/tests/media_formats.rs`).
use serde_json::{Value, json};
use tessel_editor_core::{EditorState, headless};

fn run(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn code(state: &mut EditorState, request: Value) -> String {
    headless::execute(state, &request)
        .expect_err(&format!("{request} must fail"))
        .code
        .to_string()
}

/// A 48-frame (2 s at 24 fps) shape clip `k` on its own track of the demo project.
fn project() -> EditorState {
    let mut state = EditorState::demo();
    run(
        &mut state,
        json!({"op":"track.add","id":"kt","name":"Keys","kind":"video"}),
    );
    run(
        &mut state,
        json!({"op":"clip.add","id":"k","track":"kt","start":0,"duration":48,
        "shape":{"kind":"rect","width":50,"height":50,"fill":"#ff0000","radius":0}}),
    );
    state
}

fn layer(state: &mut EditorState, frame: i64) -> Value {
    let plan = run(state, json!({"op":"plan","frame":frame}));
    plan["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|layer| layer["clip"] == "k")
        .unwrap_or_else(|| panic!("clip k is not drawn at frame {frame}: {plan}"))
        .clone()
}

fn approx(actual: &Value, expected: f64) {
    let actual = actual.as_f64().unwrap();
    assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
}

fn key(prop: &str, time_ms: i64, value: f64) -> Value {
    json!({"op":"clip.key","clip":"k","prop":prop,"time_ms":time_ms,"value":value})
}

fn zoom_twice(state: &mut EditorState) {
    run(state, key("zoom", 0, 100.0));
    run(state, key("zoom", 1000, 200.0));
}

#[test]
fn keys_drive_position_zoom_rotation_and_opacity_with_hold_and_interpolation() {
    let mut state = project();
    zoom_twice(&mut state);
    run(&mut state, key("position_x", 0, -20.0));
    run(&mut state, key("position_x", 2000, 20.0));
    run(&mut state, key("rotation", 0, 0.0));
    run(&mut state, key("rotation", 1000, 90.0));
    run(&mut state, key("opacity", 0, 0.0));
    run(&mut state, key("opacity", 500, 100.0));

    // 24 fps: frame 12 is 500 ms, frame 24 is 1000 ms, frame 47 is about 1958 ms.
    let start = layer(&mut state, 0);
    approx(&start["transform"]["scale"], 1.0);
    approx(&start["transform"]["x"], -20.0);
    approx(&start["opacity"], 0.0);

    let middle = layer(&mut state, 12);
    approx(&middle["transform"]["scale"], 1.5);
    approx(&middle["transform"]["rotation"], 45.0);
    approx(&middle["opacity"], 1.0);

    let held = layer(&mut state, 47);
    approx(&held["transform"]["scale"], 2.0);
    approx(&held["transform"]["rotation"], 90.0);
    let x = 20.0 - 40.0 * (2000.0 - 47.0 * 1000.0 / 24.0) / 2000.0;
    approx(&held["transform"]["x"], x);

    // A property without a track keeps the inspector value.
    run(
        &mut state,
        json!({"op":"clip.property","clip":"k","property":"position_y","value":10}),
    );
    approx(&layer(&mut state, 30)["transform"]["y"], 10.0);
}

#[test]
fn an_easing_shapes_the_segment_and_the_fades_still_multiply_the_keyed_opacity() {
    let mut state = project();
    run(
        &mut state,
        json!({"op":"clip.key","clip":"k","prop":"zoom","time_ms":0,"value":100,"easing":"smoothStep"}),
    );
    run(&mut state, key("zoom", 1000, 200.0));
    // smoothStep(0.5) is 0.5; smoothStep(0.25) = 0.15625.
    approx(&layer(&mut state, 12)["transform"]["scale"], 1.5);
    approx(&layer(&mut state, 6)["transform"]["scale"], 1.15625);

    run(&mut state, key("opacity", 0, 50.0));
    run(
        &mut state,
        json!({"op":"clip.property","clip":"k","property":"fade_in","value":10}),
    );
    // Frame 5 is half way through a 10-frame fade-in of a 50 % opacity.
    approx(&layer(&mut state, 5)["opacity"], 0.25);
}

#[test]
fn a_rejected_edit_changes_nothing_and_undo_restores_the_document_byte_for_byte() {
    let mut state = project();
    let before = state.to_document(None);
    zoom_twice(&mut state);
    let keyed = state.to_document(None);
    assert_ne!(before, keyed);

    assert_eq!(code(&mut state, key("crop_left", 0, 1.0)), "invalid_value");
    assert_eq!(code(&mut state, key("zoom", -5, 100.0)), "invalid_value");
    assert_eq!(code(&mut state, key("zoom", 500, 5000.0)), "invalid_value");
    assert_eq!(
        code(
            &mut state,
            json!({"op":"clip.key.set","clip":"k","prop":"zoom","index":0,"field":"time_ms","value":2000}),
        ),
        "invalid_value",
        "a retimed key may not pass its neighbour"
    );
    assert_eq!(
        code(
            &mut state,
            json!({"op":"clip.key.remove","clip":"k","prop":"zoom","time_ms":7})
        ),
        "unknown_element"
    );
    assert_eq!(
        code(
            &mut state,
            json!({"op":"clip.keys.clear","clip":"k","prop":"rotation"})
        ),
        "unknown_element"
    );
    assert_eq!(
        code(
            &mut state,
            json!({"op":"clip.key","clip":"nope","prop":"zoom","time_ms":0,"value":100})
        ),
        "unknown_element"
    );
    assert_eq!(
        state.to_document(None),
        keyed,
        "failed edits leave the clip untouched"
    );

    run(&mut state, json!({"op":"history.undo"}));
    run(&mut state, json!({"op":"history.undo"}));
    assert_eq!(state.to_document(None), before);
    run(&mut state, json!({"op":"history.redo"}));
    run(&mut state, json!({"op":"history.redo"}));
    assert_eq!(state.to_document(None), keyed);
}

#[test]
fn keys_survive_save_and_reopen_and_need_schema_2() {
    let mut state = project();
    assert_eq!(state.document_schema_version(), 1);
    zoom_twice(&mut state);
    run(&mut state, key("opacity", 0, 10.0));
    assert_eq!(state.document_schema_version(), 2);
    let document = state.to_document(None);
    assert_eq!(document["schema_version"], 2);

    let mut reopened = EditorState::from_document(&document, None).unwrap();
    assert_eq!(reopened.to_document(None), document);
    for frame in [0, 7, 12, 24, 40] {
        assert_eq!(
            layer(&mut reopened, frame)["transform"],
            layer(&mut state, frame)["transform"],
            "frame {frame}"
        );
    }

    // A schema 1 reader would drop the keys: a document claiming 1 with keys is refused.
    let mut legacy = document.clone();
    legacy["schema_version"] = json!(1);
    let error = EditorState::from_document(&legacy, None).unwrap_err();
    assert!(error.message.contains("schema 2"), "{error:?}");
}

#[test]
fn trim_move_and_split_keep_each_key_on_its_picture() {
    let mut state = project();
    zoom_twice(&mut state);
    // Trim the head to frame 12 (500 ms): its first frame is the zoom the whole clip had there.
    run(
        &mut state,
        json!({"op":"set","target":"trim:k:start","field":"frame","value":12}),
    );
    approx(&layer(&mut state, 12)["transform"]["scale"], 1.5);
    approx(&layer(&mut state, 24)["transform"]["scale"], 2.0);
    // Moving the clip carries the keys with it.
    run(
        &mut state,
        json!({"op":"move","target":"clip:k","args":{"frame":0}}),
    );
    approx(&layer(&mut state, 0)["transform"]["scale"], 1.5);
    approx(&layer(&mut state, 12)["transform"]["scale"], 2.0);

    // Splitting: both halves read the values the whole clip did.
    let mut state = project();
    zoom_twice(&mut state);
    let whole: Vec<f64> = (0..48)
        .map(|frame| {
            layer(&mut state, frame)["transform"]["scale"]
                .as_f64()
                .unwrap()
        })
        .collect();
    run(
        &mut state,
        json!({"op":"clip.split","clip":"k","frame":18,"new_id":"k2"}),
    );
    for frame in 0..48 {
        let plan = run(&mut state, json!({"op":"plan","frame":frame}));
        let scale = plan["layers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|layer| layer["clip"] == "k" || layer["clip"] == "k2")
            .map(|layer| layer["transform"]["scale"].as_f64().unwrap())
            .unwrap();
        assert!(
            (scale - whole[frame as usize]).abs() < 1e-6,
            "frame {frame}: {scale}"
        );
    }
}

#[test]
fn the_nodes_key_the_value_at_the_playhead_edit_a_key_and_remove_it() {
    let mut state = project();
    run(
        &mut state,
        json!({"op":"invoke","target":"clip:k","action":"select"}),
    );
    run(&mut state, json!({"op":"seek","frame":12}));
    run(
        &mut state,
        json!({"op":"set","target":"property:zoom","field":"value","value":120}),
    );
    run(
        &mut state,
        json!({"op":"invoke","target":"clip_keys.capture:zoom","action":"invoke"}),
    );
    let document = state.to_document(None);
    let track = &document["sequences"][0]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == "k")
        .unwrap()["keys"];
    assert_eq!(track[0]["prop"], "zoom");
    assert_eq!(track[0]["keys"][0]["time_ms"], 500);
    assert_eq!(track[0]["keys"][0]["value"], 120.0);

    run(
        &mut state,
        json!({"op":"set","target":"clip_key:k:zoom:0","field":"value","value":150}),
    );
    approx(&layer(&mut state, 30)["transform"]["scale"], 1.5);
    run(
        &mut state,
        json!({"op":"set","target":"clip_key:k:zoom:0","field":"easing","value":"smoothStep"}),
    );
    run(
        &mut state,
        json!({"op":"invoke","target":"clip_key:k:zoom:0","action":"remove"}),
    );
    approx(&layer(&mut state, 30)["transform"]["scale"], 1.2);
    assert_eq!(
        code(
            &mut state,
            json!({"op":"invoke","target":"clip_keys.clear:zoom","action":"invoke"})
        ),
        "unknown_element",
        "no keys left to remove"
    );

    // The playhead outside the clip cannot key it.
    run(&mut state, json!({"op":"seek","frame":200}));
    assert_eq!(
        code(
            &mut state,
            json!({"op":"invoke","target":"clip_keys.capture:zoom","action":"invoke"})
        ),
        "invalid_time"
    );
}
