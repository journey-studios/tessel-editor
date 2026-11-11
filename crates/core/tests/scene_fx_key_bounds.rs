//! F058 (BL082.31/.32/.34, P3): the arithmetic, the budgets and the no-op of the key ops, through
//! the library and the JSON executor (no window, no GPU).
//!
//! What it proves: a scale from the lowest anchor is refused with a typed error and leaves the
//! document, the history and the revision untouched (the arithmetic of the first version left
//! `i128` there; the probe is a unit test next to it) and the half-up rounding of the keys is
//! exact on both sides of the anchor; a batch over the ops budget is refused BEFORE anything runs
//! (8000 key ops never start), a batch inside it does its work in place (one scan of the layer,
//! no curve copied) and stops at the first op that cannot fit; the per-curve, per-clip and
//! per-layer budgets refuse the key that does not fit instead of growing past them; and a key op
//! that writes what the key already is — in another JSON spelling — is a typed no-op: no write,
//! no entry, no revision.

use core::prelude::v1::test;
use std::time::Instant;

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::fx::{
    MAX_FX_KEYS_PER_CLIP, MAX_FX_KEYS_PER_CURVE, MAX_FX_KEYS_PER_LAYER,
};
use tessel_editor_core::directed_scene::{DirectedScene, Limits};
use tessel_editor_core::scene_edit::{MAX_FX_BATCH_OPS, apply_fx_measured, parse_fx_batch};
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::{CommandError, EditorState, headless};

const MINIMAL: &str =
    include_str!("../../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json");

fn bundle() -> Value {
    serde_json::from_str(MINIMAL).unwrap()
}

fn ok(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn session() -> EditorState {
    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    ok(
        &mut state,
        json!({"op":"scene.attach","id":"street","bundle":bundle()}),
    );
    state
}

fn edit(state: &mut EditorState, ops: Value) -> Value {
    ok(
        state,
        json!({"op":"scene.fx.edit","scene":"street","ops":ops}),
    )
}

fn edit_err(state: &mut EditorState, ops: Value) -> CommandError {
    match headless::execute(
        state,
        &json!({"op":"scene.fx.edit","scene":"street","ops":ops}),
    ) {
        Ok(answer) => panic!("{ops} should have failed, got {answer}"),
        Err(error) => error,
    }
}

fn document(state: &EditorState) -> Value {
    state.scenes.get("street").unwrap().scene.document().clone()
}

fn undo_depth(state: &EditorState) -> u64 {
    state.history.to_json()["undo_depth"].as_u64().unwrap()
}

fn footprint(state: &EditorState) -> (Value, u64, u64) {
    (document(state), undo_depth(state), state.project.revision)
}

fn frames_of(state: &EditorState, param: &str) -> Vec<i64> {
    document(state)["fx"]["tracks"][0]["clips"][0]["keys"][param]
        .as_array()
        .map(|keys| {
            keys.iter()
                .map(|item| item["frame"].as_i64().unwrap())
                .collect()
        })
        .unwrap_or_default()
}

fn grain() -> Value {
    json!({"op":"add_clip","effect":"filmGrain","start_frame":0,"duration_frames":90,"id":"g"})
}

fn key(param: &str, frame: i64, value: Value) -> Value {
    json!({"op":"key_param","clip":"g","param":param,"frame":frame,"value":value})
}

fn scene_of(state: &EditorState) -> DirectedScene {
    state.scenes.get("street").unwrap().scene.as_ref().clone()
}

// ---- F1: the arithmetic of scale_keys ------------------------------------------------------------

#[test]
fn a_scale_from_the_lowest_anchor_is_a_typed_refusal_that_writes_nothing() {
    let mut state = session();
    edit(
        &mut state,
        json!([
            grain(),
            key("amount", 5, json!(0.2)),
            key("amount", 9, json!(0.4))
        ]),
    );
    let before = footprint(&state);
    for (anchor, num, den) in [
        (i64::MIN, i64::MAX, 1),
        (i64::MIN, i64::MAX, 2),
        (i64::MIN, 1, 2),
        (i64::MAX, i64::MAX, 1),
        (0, i64::MAX, 1),
    ] {
        let refused = edit_err(
            &mut state,
            json!([{"op":"scale_keys","clip":"g","param":"amount",
                    "anchor":anchor,"num":num,"den":den}]),
        );
        assert_eq!(
            refused.code, "key_out_of_clip",
            "{anchor} {num}/{den}: {refused:?}"
        );
        assert_eq!(footprint(&state), before, "{anchor} {num}/{den}");
    }
    // The identity is a valid no-op from any anchor, the lowest included.
    let same = edit(
        &mut state,
        json!([{"op":"scale_keys","clip":"g","param":"amount",
                "anchor":i64::MIN,"num":3,"den":3}]),
    );
    assert_eq!(same["changed"], false);
    assert_eq!(footprint(&state), before);
}

#[test]
fn the_rounding_of_a_scale_is_half_up_on_both_sides_of_the_anchor() {
    let mut state = session();
    edit(
        &mut state,
        json!([
            grain(),
            key("amount", 1, json!(0.1)),
            key("amount", 3, json!(0.3))
        ]),
    );
    // Anchor 0, half the distance: 0.5 -> 1 and 1.5 -> 2.
    edit(
        &mut state,
        json!([{"op":"scale_keys","clip":"g","param":"amount","anchor":0,"num":1,"den":2}]),
    );
    assert_eq!(frames_of(&state, "amount"), [1, 2]);
    // Anchor 4, half the distance, on the negative side: 1 is -3 away (-1.5 -> -1 -> 3) and 2 is
    // -2 away (-1 -> 3 too): they would collide, so the order is refused, nothing written.
    let before = footprint(&state);
    let collide = edit_err(
        &mut state,
        json!([{"op":"scale_keys","clip":"g","param":"amount","anchor":4,"num":1,"den":2}]),
    );
    assert!(!collide.code.is_empty());
    assert_eq!(footprint(&state), before);
    // Anchor 4 over keys 1 and 3 (distances -3 and -1): -1.5 -> -1 and -0.5 -> 0, so 3 and 4.
    let mut state = session();
    edit(
        &mut state,
        json!([
            grain(),
            key("amount", 1, json!(0.1)),
            key("amount", 3, json!(0.3))
        ]),
    );
    edit(
        &mut state,
        json!([{"op":"scale_keys","clip":"g","param":"amount","anchor":4,"num":1,"den":2}]),
    );
    assert_eq!(frames_of(&state, "amount"), [3, 4]);
}

// ---- F-perf: the budget of a batch and of a curve ---------------------------------------------------

#[test]
fn a_batch_over_the_ops_budget_is_refused_before_any_op_runs() {
    let mut state = session();
    edit(&mut state, json!([grain()]));
    let before = footprint(&state);
    let eight_thousand: Vec<Value> = (0..8000)
        .map(|n| key("amount", n % 90, json!(0.5)))
        .collect();
    let refused = edit_err(&mut state, Value::Array(eight_thousand.clone()));
    assert_eq!(refused.code, "fx_limit", "{refused:?}");
    assert_eq!(footprint(&state), before);

    // The same refusal at the library: the work counters say nothing started.
    let (outcome, work) = {
        let ops = vec![
            json!({"op":"remove_key","clip":"g","param":"amount","frame":0});
            MAX_FX_BATCH_OPS + 1
        ];
        let parsed = parse_fx_batch(&Value::Array(ops));
        assert!(parsed.is_err(), "the parser refuses it first");
        // A batch built past the parser (a library caller) is refused by the runner too.
        let inside = parse_fx_batch(&Value::Array(vec![
            json!({"op":"remove_key","clip":"g","param":"amount","frame":0});
            MAX_FX_BATCH_OPS
        ]))
        .unwrap();
        let mut too_many = inside.clone();
        too_many.extend(inside.into_iter().take(1));
        apply_fx_measured(&scene_of(&state), &too_many, &Limits::default())
    };
    assert!(outcome.is_err());
    assert_eq!(work.ops_run, 0);
    assert_eq!(work.document_scans, 0);
    assert_eq!(work.curve_items_copied, 0);
}

#[test]
fn a_batch_inside_the_budget_works_in_place_and_stops_at_the_first_refusal() {
    let mut state = session();
    edit(&mut state, json!([grain()]));
    let scene = scene_of(&state);
    // 1024 upserts over 90 frames: each one writes into the curve it found.
    let ops: Vec<Value> = (0..MAX_FX_BATCH_OPS as i64)
        .map(|n| key("amount", n % 90, json!((n % 100) as f64 / 100.0)))
        .collect();
    let parsed = parse_fx_batch(&Value::Array(ops)).unwrap();
    let (outcome, work) = apply_fx_measured(&scene, &parsed, &Limits::default());
    outcome.unwrap();
    assert_eq!(work.ops_run, MAX_FX_BATCH_OPS);
    assert!(
        work.document_scans <= 1,
        "the layer is read once per batch: {work:?}"
    );
    assert_eq!(
        work.curve_items_copied, 0,
        "a key op copies no curve: {work:?}"
    );

    // Op 3 names a frame the clip does not have: ops 0-2 ran, 3 refused, 4.. never started.
    let mut ops: Vec<Value> = (0..10).map(|n| key("amount", n, json!(0.5))).collect();
    ops[3] = key("amount", 90, json!(0.5));
    let parsed = parse_fx_batch(&Value::Array(ops)).unwrap();
    let (outcome, work) = apply_fx_measured(&scene, &parsed, &Limits::default());
    let refused = outcome.unwrap_err();
    assert_eq!(refused.index, Some(3));
    assert_eq!(work.ops_run, 4);
}

/// A document whose effects layer holds exactly `MAX_FX_KEYS_PER_LAYER` keys: every clip holds
/// the most a 96-frame edit lets it (384), the last one the rest.
fn full_layer() -> Value {
    let mut document = bundle();
    let curve = |count: i64| -> Value {
        Value::Array(
            (0..count)
                .map(|n| json!({"frame": n, "value": 0.5}))
                .collect(),
        )
    };
    let mut tracks = Vec::new();
    let mut left = MAX_FX_KEYS_PER_LAYER;
    let mut n = 0;
    while left > 0 {
        let share = left.min(384);
        let (a, b, c) = (
            share.min(96),
            share.saturating_sub(96).min(96),
            share.saturating_sub(192).min(96),
        );
        let d = share - a - b - c;
        let mut keys = serde_json::Map::new();
        for (name, count) in [("amount", a), ("size", b), ("luma", c), ("intensity", d)] {
            if count > 0 {
                keys.insert(name.to_string(), curve(count as i64));
            }
        }
        // One clip per track: clips of one track may not overlap.
        tracks.push(
            json!({"id": format!("t{n}"), "label": format!("FX {n}"), "clips": [
            {"id": format!("c{n}"), "effect": "filmGrain",
             "startFrame": 0, "durationFrames": 96, "keys": keys}]}),
        );
        left -= share;
        n += 1;
    }
    document["fx"] = json!({"catalogVersion":1,
        "tracks":tracks,"transitions":[]});
    document
}

#[test]
fn the_budgets_of_a_curve_a_clip_and_a_layer_refuse_the_key_that_does_not_fit() {
    assert_eq!(MAX_FX_KEYS_PER_CURVE, 256);
    assert_eq!(MAX_FX_KEYS_PER_CLIP, 1024);
    let document = full_layer();
    let scene = DirectedScene::import_value(document).expect("a layer exactly at its budget");
    // The last clip is short of 96 on its last curve; a key there is the layer's one more.
    let last = scene
        .document()
        .pointer("/fx/tracks")
        .and_then(Value::as_array)
        .and_then(|tracks| tracks.last())
        .and_then(|track| track["clips"][0].as_object().cloned())
        .map(Value::Object)
        .unwrap();
    let id = last["id"].as_str().unwrap().to_string();
    let spare = (0..96)
        .find(|frame| {
            !last["keys"]["intensity"]
                .as_array()
                .is_some_and(|items| items.iter().any(|item| item["frame"] == *frame))
        })
        .expect("the last curve has a free frame");
    let ops = parse_fx_batch(&json!([
        {"op":"key_param","clip":id,"param":"intensity","frame":spare,"value":0.5}
    ]))
    .unwrap();
    let (outcome, work) = apply_fx_measured(&scene, &ops, &Limits::default());
    let refused = outcome.unwrap_err();
    assert_eq!(refused.code, "FX_LIMIT", "{refused:?}");
    assert_eq!(work.document_scans, 1, "{work:?}");
    assert_eq!(
        work.curve_items_copied, 0,
        "refused before anything grew: {work:?}"
    );
    // Replacing a key of a full layer is not growth: it still goes through.
    let replace = parse_fx_batch(&json!([
        {"op":"key_param","clip":"c0","param":"amount","frame":0,"value":0.9}
    ]))
    .unwrap();
    apply_fx_measured(&scene, &replace, &Limits::default())
        .0
        .unwrap();
}

/// A short paired measurement, not a gate (a clock is no assertion): the same batch of 1 and of
/// 64 key ops on an empty layer and on a layer at its budget. What it shows is where the cost of a
/// batch is: per batch (the importer judges the whole candidate once), not per op. Run it with
/// `-- --ignored --nocapture`; the deterministic counts are asserted.
#[test]
#[ignore = "measurement; prints medians, asserts only the deterministic work counts"]
fn paired_measurement_of_a_batch_on_a_small_and_a_full_layer() {
    let small = {
        let mut state = session();
        edit(&mut state, json!([grain()]));
        scene_of(&state)
    };
    let full = DirectedScene::import_value(full_layer()).unwrap();
    let batch = |clip: &str, count: i64| {
        parse_fx_batch(&Value::Array(
            (0..count)
                .map(|n| {
                    json!({"op":"key_param","clip":clip,"param":"amount","frame":n,
                           "value":(n % 10) as f64 / 10.0 + 0.05})
                })
                .collect(),
        ))
        .unwrap()
    };
    let median = |scene: &DirectedScene, clip: &str, count: i64| {
        let ops = batch(clip, count);
        let mut times: Vec<u128> = (0..9)
            .map(|_| {
                let start = Instant::now();
                let (outcome, work) = apply_fx_measured(scene, &ops, &Limits::default());
                outcome.unwrap();
                assert_eq!(work.curve_items_copied, 0);
                assert!(work.document_scans <= 1);
                start.elapsed().as_micros()
            })
            .collect();
        times.sort_unstable();
        times[times.len() / 2]
    };
    for (name, scene, clip) in [("empty layer", &small, "g"), ("full layer", &full, "c0")] {
        println!(
            "paired: {name}: 1 op {} us, 64 ops {} us",
            median(scene, clip, 1),
            median(scene, clip, 64)
        );
    }
}

// ---- F8: a key op that writes what is there is a typed no-op ----------------------------------------

#[test]
fn a_key_that_already_holds_the_value_is_a_no_op_in_any_spelling() {
    let mut state = session();
    edit(
        &mut state,
        json!([
            grain(),
            key("amount", 3, json!(1.0)),
            key("amount", 4, json!(0.0)),
            {"op":"key_param","clip":"g","param":"luma","frame":5,"value":0.25,"easing":"easeInOutCubic"},
        ]),
    );
    let before = footprint(&state);
    for ops in [
        // `1` is the number the key holds as `1.0`.
        json!([key("amount", 3, json!(1))]),
        json!([key("amount", 3, json!(1.0e0))]),
        // Minus zero is the zero the key holds.
        json!([key("amount", 4, json!(-0.0))]),
        // The easing it already has, named again.
        json!([{"op":"key_param","clip":"g","param":"luma","frame":5,"value":0.25,"easing":"easeInOutCubic"}]),
        json!([key("luma", 5, json!(0.250))]),
    ] {
        let answer = edit(&mut state, ops.clone());
        assert_eq!(answer["changed"], false, "{ops}");
        assert_eq!(footprint(&state), before, "{ops}");
    }
    // A real change is still a change, and the next write of it is again nothing.
    let changed = edit(&mut state, json!([key("amount", 3, json!(0.5))]));
    assert_eq!(changed["changed"], true);
    assert_eq!(undo_depth(&state), before.1 + 1);
    let again = edit(&mut state, json!([key("amount", 3, json!(0.5))]));
    assert_eq!(again["changed"], false);
}
