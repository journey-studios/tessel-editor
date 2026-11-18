//! F072 (BL169.6 step 5): the reveal of a clip, a sweeping edge set in the inspector section
//! "Revelação", through the headless contract: set, plan before, during and after the sweep,
//! undo, save and reopen, the cut, the old document, and the refusals.
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

/// 1920x1080 at 30 fps with one image clip `shot` on `overlay`, frames 10..70.
fn project(tag: &str) -> EditorState {
    let mut state = EditorState::demo();
    run(
        &mut state,
        json!({"op":"project.create","name":"Reveal","rate":{"num":30,"den":1},
            "width":1920,"height":1080,"length":120}),
    );
    run(
        &mut state,
        json!({"op":"track.add","id":"overlay","name":"Overlay","kind":"video"}),
    );
    let dir = std::env::temp_dir().join(format!("tessel-reveal-{tag}-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("shot.png");
    fs::write(&path, quadrants_png(64, 36)).unwrap();
    run(
        &mut state,
        json!({"op":"asset.add","kind":"still","id":"shot","name":"Shot","path":path}),
    );
    run(
        &mut state,
        json!({"op":"clip.add","id":"shot-clip","track":"overlay","asset":"shot",
            "start":10,"duration":60}),
    );
    state
}

fn property(state: &mut EditorState, clip: &str, property: &str, value: Value) -> Value {
    run(
        state,
        json!({"op":"clip.property","clip":clip,"property":property,"value":value}),
    )
}

/// The 23 s reference: the edge leans atan(118 / 540), runs from x = -300 to x = 2229 over 9
/// frames and eases with smoothStep.
fn set_reference(state: &mut EditorState, clip: &str) {
    for (name, value) in [
        ("reveal_mode", json!("wipe")),
        ("reveal_from", json!(-300.0 / 19.2)),
        ("reveal_to", json!(2229.0 / 19.2)),
        (
            "reveal_angle",
            json!((118.0_f64 / 540.0).atan().to_degrees()),
        ),
        ("reveal_duration", json!(9)),
    ] {
        property(state, clip, name, value);
    }
}

fn layers(state: &mut EditorState, frame: i64) -> Vec<Value> {
    run(state, json!({"op":"plan","frame":frame}))["layers"]
        .as_array()
        .unwrap()
        .clone()
}

fn reveal_at(state: &mut EditorState, frame: i64) -> Value {
    layers(state, frame)[0]
        .get("reveal")
        .cloned()
        .unwrap_or(Value::Null)
}

fn smooth(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

#[test]
fn a_clip_without_a_reveal_plans_exactly_what_it_planned_before() {
    let mut state = project("off");
    for frame in [9, 10, 30, 69] {
        for layer in layers(&mut state, frame) {
            assert!(layer.get("reveal").is_none(), "frame {frame}");
        }
    }
    let document = state.to_document(None).to_string();
    assert!(
        !document.contains("reveal_"),
        "nothing about a reveal is written until one is set"
    );
}

#[test]
fn the_edge_is_hidden_before_the_sweep_moves_during_it_and_is_gone_after_it() {
    let mut state = project("sweep");
    set_reference(&mut state, "shot-clip");

    // Frame 10 is the clip's first: the edge is still at x = -300, off the left.
    let first = reveal_at(&mut state, 10);
    assert_eq!(first["direction"], 1.0);
    assert!((first["edge"].as_f64().unwrap() - (-300.0 / 19.2)).abs() < 1e-9);
    assert!((first["angle"].as_f64().unwrap() - 12.3325).abs() < 1e-2);

    // During: frame 13 is 3 of 9 into the sweep.
    let during = reveal_at(&mut state, 13);
    let wanted = -300.0 / 19.2 + (2229.0 / 19.2 + 300.0 / 19.2) * smooth(3.0 / 9.0);
    assert!(
        // The sliders keep three decimals of a percent: 0.001 % of 1920 px is 0.02 px.
        (during["edge"].as_f64().unwrap() - wanted).abs() < 1e-3,
        "{during}"
    );
    assert!(during["edge"].as_f64().unwrap() > first["edge"].as_f64().unwrap());

    // After: the whole canvas is behind the edge, the layer is the plain one.
    for frame in [19, 20, 40, 69] {
        assert!(reveal_at(&mut state, frame).is_null(), "frame {frame}");
    }
    // Before the clip there is no layer at all.
    assert!(layers(&mut state, 9).is_empty());

    // The picture of the last frame is the one a clip without a reveal has: same plan.
    let mut plain = project("sweep-plain");
    assert_eq!(layers(&mut state, 30), layers(&mut plain, 30));
}

#[test]
fn a_sweep_that_stops_short_holds_its_edge_for_the_rest_of_the_clip() {
    let mut state = project("hold");
    set_reference(&mut state, "shot-clip");
    property(&mut state, "shot-clip", "reveal_to", json!(50));
    for frame in [19, 40, 69] {
        assert_eq!(reveal_at(&mut state, frame)["edge"], 50.0, "frame {frame}");
    }
}

#[test]
fn a_delay_waits_and_the_curve_and_direction_are_choices() {
    let mut state = project("delay");
    set_reference(&mut state, "shot-clip");
    property(&mut state, "shot-clip", "reveal_offset", json!(5));
    // Still at the start 5 frames in, then it moves.
    assert_eq!(reveal_at(&mut state, 15), reveal_at(&mut state, 10));
    assert_ne!(reveal_at(&mut state, 16), reveal_at(&mut state, 15));

    property(&mut state, "shot-clip", "reveal_easing", json!("linear"));
    let half = reveal_at(&mut state, 15 + 4)["edge"].as_f64().unwrap();
    let wanted = -300.0 / 19.2 + (2229.0 / 19.2 + 300.0 / 19.2) * (4.0 / 9.0);
    assert!((half - wanted).abs() < 1e-3, "linear: {half} vs {wanted}");

    // Travelling the other way shows the clip to the right of the edge.
    property(&mut state, "shot-clip", "reveal_from", json!(120));
    property(&mut state, "shot-clip", "reveal_to", json!(-20));
    assert_eq!(reveal_at(&mut state, 10)["direction"], -1.0);
}

#[test]
fn every_property_is_one_undo_entry_and_undo_walks_back_to_a_plain_clip() {
    let mut state = project("undo");
    let before = layers(&mut state, 12);
    let depth = state.history.to_json()["undo_depth"].as_i64().unwrap();
    set_reference(&mut state, "shot-clip");
    assert_eq!(
        state.history.to_json()["undo_depth"].as_i64().unwrap(),
        depth + 4,
        "four properties moved off their defaults (the 9-frame duration is one), four entries"
    );
    assert!(!reveal_at(&mut state, 12).is_null());
    for _ in 0..4 {
        run(&mut state, json!({"op":"history.undo"}));
    }
    assert_eq!(layers(&mut state, 12), before, "back to the plain plan");
    run(&mut state, json!({"op":"history.redo"}));
    assert!(!reveal_at(&mut state, 12).is_null() || before == layers(&mut state, 12));
}

#[test]
fn the_settings_survive_save_and_reopen_with_the_same_plans() {
    let mut state = project("persist");
    set_reference(&mut state, "shot-clip");
    property(
        &mut state,
        "shot-clip",
        "reveal_easing",
        json!("easeOutCubic"),
    );
    property(&mut state, "shot-clip", "reveal_offset", json!(-2));
    let planned: Vec<Vec<Value>> = (9..22).map(|frame| layers(&mut state, frame)).collect();
    let document = state.to_document(None);
    let text = document.to_string();
    for key in [
        "reveal_mode",
        "reveal_from",
        "reveal_to",
        "reveal_angle",
        "reveal_easing",
        "reveal_offset",
    ] {
        assert!(text.contains(key), "{key} is saved");
    }
    let mut reopened = EditorState::from_document(&document, None).unwrap();
    for (index, frame) in (9..22).enumerate() {
        assert_eq!(
            layers(&mut reopened, frame),
            planned[index],
            "frame {frame}"
        );
    }
    assert_eq!(
        reopened.to_document(None),
        document,
        "a lossless round trip"
    );
}

#[test]
fn a_hand_edited_document_cannot_put_a_foreign_value_in_a_reveal_property() {
    let mut state = project("hand");
    set_reference(&mut state, "shot-clip");
    let document = state.to_document(None).to_string();
    for (from, to) in [("\"wipe\"", "\"spiral\""), ("\"smoothStep\"", "\"bounce\"")] {
        let tampered = document.replace(from, to);
        if tampered == document {
            continue;
        }
        let value: Value = serde_json::from_str(&tampered).unwrap();
        assert!(
            EditorState::from_document(&value, None).is_err(),
            "{to} is refused on reading"
        );
    }
}

#[test]
fn the_reveal_applies_to_titles_shapes_and_groups_and_is_refused_on_sound() {
    let mut state = project("kinds");
    run(
        &mut state,
        json!({"op":"clip.add","id":"halo","track":"overlay","start":80,"duration":30,
            "shape":{"kind":"ellipse","width":40,"height":40}}),
    );
    set_reference(&mut state, "halo");
    let layer = layers(&mut state, 80).remove(0);
    assert_eq!(layer["kind"], "shape");
    assert_eq!(layer["reveal"]["direction"], 1.0);

    run(
        &mut state,
        json!({"op":"track.add","id":"sound","name":"Sound","kind":"audio"}),
    );
    let document = state.to_document(None);
    let error = refused(
        &mut state,
        json!({"op":"clip.property","clip":"shot-clip","property":"reveal_mode","value":"spiral"}),
    );
    assert_eq!(error.code, "invalid_value");
    let error = refused(
        &mut state,
        json!({"op":"clip.property","clip":"shot-clip","property":"reveal_easing","value":"bounce"}),
    );
    assert_eq!(error.code, "invalid_value");
    assert_eq!(
        state.to_document(None),
        document,
        "a refusal changes nothing"
    );

    // Sliders clamp like every slider.
    let outcome = property(&mut state, "shot-clip", "reveal_duration", json!(900));
    assert_eq!(outcome["clamped"], true);
    assert_eq!(outcome["value"], 240.0);
}

#[test]
fn a_group_clip_gives_every_layer_it_expands_into_the_same_edge() {
    let mut state = project("group");
    run(
        &mut state,
        json!({"op":"group.define","definition":{"id":"pair","name":"Pair","children":[
            {"id":"a","kind":"shape","x":-20,
             "shape":{"kind":"rect","width":20,"height":20,"fill":"#102030"}},
            {"id":"b","kind":"shape","x":20,
             "shape":{"kind":"rect","width":20,"height":20,"fill":"#203040"}}]}}),
    );
    run(
        &mut state,
        json!({"op":"clip.add","id":"pair-clip","track":"overlay","start":80,"duration":30,
            "group":{"def":"pair"}}),
    );
    set_reference(&mut state, "pair-clip");
    let planned = layers(&mut state, 83);
    assert_eq!(planned.len(), 2, "a group is its children's layers");
    assert_eq!(planned[0]["reveal"], planned[1]["reveal"]);
    assert!(!planned[0]["reveal"].is_null());
    let after = layers(&mut state, 95);
    assert!(after.iter().all(|layer| layer.get("reveal").is_none()));
}

#[test]
fn cutting_a_clip_mid_sweep_keeps_every_frame_exactly_where_the_whole_clip_had_it() {
    let mut whole = project("split-whole");
    set_reference(&mut whole, "shot-clip");
    let wanted: Vec<Value> = (10..70).map(|frame| reveal_at(&mut whole, frame)).collect();

    for cut in [12, 15, 19, 40] {
        let mut state = project("split");
        set_reference(&mut state, "shot-clip");
        run(&mut state, json!({"op":"seek","frame":cut}));
        run(
            &mut state,
            json!({"op":"clip.split","clip":"shot-clip","frame":cut,"new_id":"shot-right"}),
        );
        for (index, frame) in (10..70).enumerate() {
            assert_eq!(
                reveal_at(&mut state, frame),
                wanted[index],
                "cut at {cut}, frame {frame}"
            );
        }
        run(&mut state, json!({"op":"history.undo"}));
        for (index, frame) in (10..70).enumerate() {
            assert_eq!(
                reveal_at(&mut state, frame),
                wanted[index],
                "undone, frame {frame}"
            );
        }
    }
}
