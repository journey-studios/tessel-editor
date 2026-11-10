//! F058 (BL082.2, P3): the keys of an effect clip and their ops, through the library and the JSON
//! executor `tessel-headless` and `ui op` share (no window, no GPU).
//!
//! What it proves: the importer's refusals of a curve (order, range, easing, the boolean, the
//! budget) with the document untouched; the exact sampling the plan carries (holds, a key's own
//! value, the easing of the arrival key, the step of a boolean); the atomic key ops (upsert that
//! keeps an easing, a capture, a static `set_param` under keys that warns and keeps the curve, a
//! reset that does not delete the animation, the explicit removals); an ordered gesture whose
//! refused update keeps the last legal candidate and whose commit is ONE undo entry; and that the
//! document survives save and reopen. The pixels of a keyed grain are `crates/render` and the
//! runtime tests.

use core::prelude::v1::test;
use std::fs;

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::directed_scene::fx;
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::{CommandError, EditorState, headless, persist};

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

fn undo_depth(state: &EditorState) -> u64 {
    state.history.to_json()["undo_depth"].as_u64().unwrap()
}

fn document(state: &EditorState) -> Value {
    state.scenes.get("street").unwrap().scene.document().clone()
}

fn footprint(state: &EditorState) -> (Value, u64, u64) {
    (document(state), undo_depth(state), state.project.revision)
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

fn issue_codes(error: &CommandError) -> Vec<String> {
    error
        .details
        .as_ref()
        .and_then(|details| details["issues"].as_array())
        .map(|issues| {
            issues
                .iter()
                .map(|issue| issue["code"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// A grain clip `g` over the edit frames 10..50 (clip frames 0..40).
fn grain() -> Value {
    json!({"op":"add_clip","effect":"filmGrain","start_frame":10,"duration_frames":40,"id":"g"})
}

fn key(param: &str, frame: i64, value: Value) -> Value {
    json!({"op":"key_param","clip":"g","param":param,"frame":frame,"value":value})
}

fn keyed(param: &str, frame: i64, value: Value, easing: &str) -> Value {
    json!({"op":"key_param","clip":"g","param":param,"frame":frame,"value":value,"easing":easing})
}

fn keys_of(state: &EditorState, param: &str) -> Vec<Value> {
    document(state)["fx"]["tracks"][0]["clips"][0]["keys"][param]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn frames_of(state: &EditorState, param: &str) -> Vec<i64> {
    keys_of(state, param)
        .iter()
        .map(|item| item["frame"].as_i64().unwrap())
        .collect()
}

fn pass_at(state: &mut EditorState, edit_frame: i64) -> Value {
    ok(
        state,
        json!({"op":"scene.fx.frame","scene":"street","frame":edit_frame}),
    )["passes"][0]
        .clone()
}

fn param_at(state: &mut EditorState, edit_frame: i64, param: &str) -> Value {
    pass_at(state, edit_frame)["params"][param].clone()
}

// ---- the importer judges a curve --------------------------------------------------------------------

fn import_codes(change: &dyn Fn(&mut Value)) -> Vec<String> {
    let mut document = bundle();
    document["fx"] = json!({"catalogVersion":1,"tracks":[{"id":"t","label":"FX","clips":[
        {"id":"c","effect":"filmGrain","startFrame":0,"durationFrames":10}]}],"transitions":[]});
    change(&mut document);
    match DirectedScene::import_value(document) {
        Ok(_) => Vec::new(),
        Err(error) => error
            .issues()
            .iter()
            .map(|issue| issue.code.to_string())
            .collect(),
    }
}

#[test]
fn a_curve_is_judged_by_the_importer_and_a_refused_import_names_the_rule() {
    let with_keys = |keys: Value| {
        move |document: &mut Value| document["fx"]["tracks"][0]["clips"][0]["keys"] = keys.clone()
    };
    let ok_keys = json!({"amount":[{"frame":0,"value":0.1},{"frame":9,"value":0.9,"easing":"easeOutCubic"}],
        "colored":[{"frame":3,"value":true}]});
    assert!(
        import_codes(&with_keys(ok_keys)).is_empty(),
        "the base case is valid"
    );
    let cases: Vec<(Value, &str)> = vec![
        (
            json!({"amount":[{"frame":4,"value":0.1},{"frame":2,"value":0.2}]}),
            "KEYFRAME_ORDER",
        ),
        (
            json!({"amount":[{"frame":4,"value":0.1},{"frame":4,"value":0.2}]}),
            "KEYFRAME_ORDER",
        ),
        (
            json!({"amount":[{"frame":10,"value":0.1}]}),
            "KEY_OUT_OF_CLIP",
        ),
        (
            json!({"amount":[{"frame":-1,"value":0.1}]}),
            "KEY_OUT_OF_CLIP",
        ),
        (
            json!({"amount":[{"frame":1,"value":0.1,"easing":"EaseOutCubic"}]}),
            "UNKNOWN_KEYFRAME_EASING",
        ),
        (
            json!({"amount":[{"frame":1,"value":0.1,"easing":"easeInCubic"}]}),
            "UNKNOWN_KEYFRAME_EASING",
        ),
        (
            json!({"amount":[{"frame":1,"value":1.5}]}),
            "PARAM_OUT_OF_RANGE",
        ),
        (json!({"amount":[{"frame":1,"value":"0.5"}]}), "PARAM_TYPE"),
        (json!({"colored":[{"frame":1,"value":1}]}), "PARAM_TYPE"),
        (
            json!({"colored":[{"frame":1,"value":"true"}]}),
            "PARAM_TYPE",
        ),
        (
            json!({"amount":[{"frame":1.5,"value":0.1}]}),
            "INVALID_VALUE",
        ),
        (json!({"amount":[{"value":0.1}]}), "MISSING_FIELD"),
        (json!({"amount":[{"frame":1}]}), "MISSING_FIELD"),
        (json!({"amount":[]}), "INVALID_VALUE"),
        (json!({"amount":{"frame":1}}), "INVALID_TYPE"),
        (json!({"radius":[{"frame":1,"value":0.1}]}), "UNKNOWN_PARAM"),
    ];
    for (keys, code) in cases {
        assert_eq!(import_codes(&with_keys(keys.clone())), [code], "{keys}");
    }
    // The boolean is strict: no number, no string, and a bool param's static value too.
    assert_eq!(
        import_codes(&|d| d["fx"]["tracks"][0]["clips"][0]["params"] = json!({"colored":1})),
        ["PARAM_TYPE"]
    );
}

#[test]
fn the_data_budget_of_a_curve_and_a_clip_is_a_limit_not_a_truncation() {
    // The frames do not fit the 10-frame clip, so other issues are expected; only the limit matters.
    let curve = |count: i64| -> Value {
        Value::Array(
            (0..count)
                .map(|n| json!({"frame": n, "value": 0.5}))
                .collect(),
        )
    };
    let limited = |keys: Value| {
        import_codes(&move |d| d["fx"]["tracks"][0]["clips"][0]["keys"] = keys.clone())
            .contains(&"FX_LIMIT".to_string())
    };
    assert!(
        !limited(json!({"amount": curve(256)})),
        "256 is the limit of a curve"
    );
    assert!(
        limited(json!({"amount": curve(257)})),
        "one more is refused, not cut"
    );
    let four = json!({"amount": curve(256), "size": curve(256), "luma": curve(256), "intensity": curve(256)});
    assert!(!limited(four.clone()), "4 x 256 is the limit of a clip");
    let mut five = four;
    five["colored"] = json!([{"frame":0,"value":true}]);
    assert!(limited(five), "one key over the clip's 1024 is refused");
}

// ---- the sampling the plan carries -------------------------------------------------------------------

#[test]
fn the_passes_carry_the_exact_sampling_of_the_keys() {
    let mut state = session();
    edit(
        &mut state,
        json!([
            grain(),
            key("amount", 4, json!(0.2)),
            keyed("amount", 14, json!(0.8), "easeOutCubic"),
            keyed("amount", 30, json!(0.3), "easeInOutCubic"),
            key("colored", 10, json!(false)),
            key("colored", 20, json!(true)),
        ]),
    );
    // Clip frame f is edit frame 10 + f.
    let at = |state: &mut EditorState, f: i64| param_at(state, 10 + f, "amount").as_f64().unwrap();
    assert_eq!(
        at(&mut state, 0),
        0.2,
        "holds the first value before the first key"
    );
    assert_eq!(
        at(&mut state, 4).to_bits(),
        0.2_f64.to_bits(),
        "a key frame is the stored value"
    );
    assert_eq!(at(&mut state, 14).to_bits(), 0.8_f64.to_bits());
    assert_eq!(at(&mut state, 30).to_bits(), 0.3_f64.to_bits());
    assert_eq!(
        at(&mut state, 39),
        0.3,
        "holds the last value after the last key"
    );
    // The easing belongs to the key the span arrives at: halfway to the easeOutCubic key.
    let eased = 0.2 + (0.8 - 0.2) * (1.0 - 0.5_f64.powi(3));
    assert!(
        (at(&mut state, 9) - eased).abs() < 1e-12,
        "{}",
        at(&mut state, 9)
    );
    // easeInOutCubic arriving at 30: the midpoint of the span 14..30 is 22, where it is 0.5.
    assert!((at(&mut state, 22) - 0.55).abs() < 1e-12);
    // The boolean keeps its value until the next key's frame and takes it there.
    assert_eq!(param_at(&mut state, 10, "colored"), false);
    assert_eq!(param_at(&mut state, 19 + 10, "colored"), false);
    assert_eq!(param_at(&mut state, 20 + 10, "colored"), true);
    assert_eq!(param_at(&mut state, 39 + 10, "colored"), true);
    // The intensity is applied once, on top of the sampled amount, never inside it.
    let pass = pass_at(&mut state, 14);
    assert_eq!(pass["amount"], 1.0);
    assert_eq!(pass["params"]["intensity"], 1.0);
    assert!(pass["seed"].is_u64(), "a noise pass carries its seed");
}

#[test]
fn the_seed_is_per_frame_and_a_function_of_the_frame_the_base_and_nothing_else() {
    let mut state = session();
    edit(&mut state, json!([grain()]));
    let seeds: Vec<u64> = (10..50)
        .map(|frame| pass_at(&mut state, frame)["seed"].as_u64().unwrap())
        .collect();
    let distinct: std::collections::BTreeSet<_> = seeds.iter().collect();
    assert_eq!(
        distinct.len(),
        seeds.len(),
        "every frame draws its own grain"
    );
    // Asking out of order or twice gives the same seed.
    assert_eq!(pass_at(&mut state, 33)["seed"].as_u64().unwrap(), seeds[23]);
    assert_eq!(pass_at(&mut state, 10)["seed"].as_u64().unwrap(), seeds[0]);
    // An explicit base seed changes every frame's seed and nothing else.
    edit(&mut state, json!([{"op":"set_seed","clip":"g","seed":7}]));
    let other: Vec<u64> = (10..50)
        .map(|frame| pass_at(&mut state, frame)["seed"].as_u64().unwrap())
        .collect();
    assert_ne!(other, seeds);
    // The first frame of the clip draws lowbias32(base ^ 0); the seed is the pilot's, not a copy of it.
    assert_eq!(other[0], u64::from(fx::lowbias32(7)));
    assert_eq!(other[5], u64::from(fx::lowbias32(7 ^ 5)));
}

// ---- atomic key ops ----------------------------------------------------------------------------------

#[test]
fn an_upsert_keeps_an_omitted_easing_and_a_capture_records_what_the_curve_shows() {
    let mut state = session();
    edit(
        &mut state,
        json!([
            grain(),
            key("amount", 0, json!(0.0)),
            keyed("amount", 20, json!(1.0), "easeOutCubic"),
        ]),
    );
    // Changing only the value keeps the easing the key has.
    edit(&mut state, json!([key("amount", 20, json!(0.5))]));
    assert_eq!(keys_of(&state, "amount")[1]["easing"], "easeOutCubic");
    assert_eq!(keys_of(&state, "amount")[1]["value"], 0.5);
    // A key written without an easing stays without one (no invented default).
    assert!(keys_of(&state, "amount")[0].get("easing").is_none());
    // A capture keys what the curve shows at that frame: halfway on a linear arrival.
    edit(
        &mut state,
        json!([keyed("amount", 30, json!(0.1), "linear")]),
    );
    let depth = undo_depth(&state);
    let answer = edit(
        &mut state,
        json!([{"op":"key_param","clip":"g","param":"amount","frame":25}]),
    );
    assert_eq!(answer["changed"], true);
    let captured = keys_of(&state, "amount")
        .into_iter()
        .find(|item| item["frame"] == 25)
        .unwrap();
    let expected = 0.5 + (0.1 - 0.5) * 0.5;
    assert!(
        (captured["value"].as_f64().unwrap() - expected).abs() < 1e-12,
        "{captured}"
    );
    assert_eq!(
        frames_of(&state, "amount"),
        [0, 20, 25, 30],
        "the order is kept"
    );
    assert_eq!(undo_depth(&state), depth + 1);
    // Capturing where a key already is changes nothing: no revision, no undo entry.
    let before = footprint(&state);
    let answer = edit(
        &mut state,
        json!([{"op":"key_param","clip":"g","param":"amount","frame":25}]),
    );
    assert_eq!(answer["changed"], false);
    assert_eq!(footprint(&state), before);
    // A capture with no curve records the base value.
    edit(
        &mut state,
        json!([{"op":"set_param","clip":"g","param":"size","value":2.5}]),
    );
    edit(
        &mut state,
        json!([{"op":"key_param","clip":"g","param":"size","frame":5}]),
    );
    assert_eq!(keys_of(&state, "size")[0]["value"], 2.5);
}

#[test]
fn a_static_set_under_keys_keeps_the_curve_and_warns_and_a_reset_does_not_delete_the_animation() {
    let mut state = session();
    edit(
        &mut state,
        json!([
            grain(),
            key("amount", 0, json!(0.1)),
            key("amount", 20, json!(0.9))
        ]),
    );
    let answer = edit(
        &mut state,
        json!([{"op":"set_param","clip":"g","param":"amount","value":0.6}]),
    );
    let warnings = answer["warnings"].as_array().unwrap();
    assert!(
        warnings
            .iter()
            .any(|warning| warning["code"] == "FX_STATIC_UNDER_KEYS"),
        "{answer}"
    );
    assert_eq!(answer["changes"][0]["warnings"][0]["param"], "amount");
    assert_eq!(
        frames_of(&state, "amount"),
        [0, 20],
        "the curve is still there"
    );
    assert_eq!(
        document(&state)["fx"]["tracks"][0]["clips"][0]["params"]["amount"],
        0.6,
        "the base value was written"
    );
    assert_eq!(
        param_at(&mut state, 10, "amount"),
        0.1,
        "the picture follows the curve"
    );
    // A reset restores the base to its default and leaves the animation alone.
    edit(
        &mut state,
        json!([{"op":"set_param","clip":"g","param":"amount","value":0.25}]),
    );
    assert_eq!(frames_of(&state, "amount"), [0, 20]);
    // A param with no keys never warns.
    let answer = edit(
        &mut state,
        json!([{"op":"set_param","clip":"g","param":"size","value":2.0}]),
    );
    assert!(
        answer["changes"][0]["warnings"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn removals_are_explicit_and_the_last_key_takes_the_curve_but_not_the_base() {
    let mut state = session();
    edit(
        &mut state,
        json!([
            grain(),
            {"op":"set_param","clip":"g","param":"amount","value":0.4},
            key("amount", 0, json!(0.1)),
            key("amount", 20, json!(0.9)),
            key("luma", 5, json!(0.2)),
        ]),
    );
    edit(
        &mut state,
        json!([{"op":"remove_key","clip":"g","param":"amount","frame":0}]),
    );
    assert_eq!(frames_of(&state, "amount"), [20]);
    edit(
        &mut state,
        json!([{"op":"remove_key","clip":"g","param":"amount","frame":20}]),
    );
    assert!(
        document(&state)["fx"]["tracks"][0]["clips"][0]["keys"]
            .get("amount")
            .is_none()
    );
    assert_eq!(
        document(&state)["fx"]["tracks"][0]["clips"][0]["params"]["amount"],
        0.4,
        "the base value never moved"
    );
    assert_eq!(param_at(&mut state, 30, "amount"), 0.4, "back on the base");
    // `clear_keys` removes one animation, or all of them, and the `keys` object leaves with the last.
    edit(
        &mut state,
        json!([{"op":"clear_keys","clip":"g","param":"luma"}]),
    );
    assert!(
        document(&state)["fx"]["tracks"][0]["clips"][0]
            .get("keys")
            .is_none()
    );
    // Clearing what is not there is a no-op that writes nothing.
    let before = footprint(&state);
    let answer = edit(&mut state, json!([{"op":"clear_keys","clip":"g"}]));
    assert_eq!(answer["changed"], false);
    assert_eq!(footprint(&state), before);
    // A key that is not there is a typed refusal that changes nothing.
    let error = edit_err(
        &mut state,
        json!([{"op":"remove_key","clip":"g","param":"amount","frame":3}]),
    );
    assert_eq!(error.code, "unknown_keyframe", "{error:?}");
    assert_eq!(footprint(&state), before);
}

#[test]
fn move_shift_and_scale_are_ordered_atomic_and_exact() {
    let mut state = session();
    edit(
        &mut state,
        json!([
            grain(),
            key("amount", 5, json!(0.1)),
            key("amount", 15, json!(0.5)),
            key("amount", 25, json!(0.9)),
            key("size", 5, json!(1.0)),
            key("size", 10, json!(3.0)),
        ]),
    );
    edit(
        &mut state,
        json!([{"op":"move_key","clip":"g","param":"amount","from":15,"to":18}]),
    );
    assert_eq!(frames_of(&state, "amount"), [5, 18, 25]);
    assert_eq!(
        keys_of(&state, "amount")[1]["value"],
        0.5,
        "the value travels with the key"
    );
    // Crossing a neighbour or landing on one is refused and the document is untouched.
    let before = footprint(&state);
    for to in [25, 30, 5, 2, -1, 40] {
        let error = edit_err(
            &mut state,
            json!([{"op":"move_key","clip":"g","param":"amount","from":18,"to":to}]),
        );
        assert!(
            ["keyframe_order", "key_out_of_clip"].contains(&error.code),
            "to {to}: {error:?}"
        );
        assert_eq!(footprint(&state), before, "to {to}");
    }
    // A shift moves the whole curve, or every curve when no parameter is named.
    edit(
        &mut state,
        json!([{"op":"shift_keys","clip":"g","delta":3}]),
    );
    assert_eq!(frames_of(&state, "amount"), [8, 21, 28]);
    assert_eq!(frames_of(&state, "size"), [8, 13]);
    // A shift that would push any key out of the clip refuses the whole batch (even the part
    // that fits) and names the op.
    let error = edit_err(
        &mut state,
        json!([{"op":"shift_keys","clip":"g","param":"size","delta":-1},
               {"op":"shift_keys","clip":"g","param":"amount","delta":12}]),
    );
    assert_eq!(error.code, "key_out_of_clip", "{error:?}");
    assert_eq!(error.details.as_ref().unwrap()["index"], 1);
    assert_eq!(
        frames_of(&state, "size"),
        [8, 13],
        "the first op did not survive"
    );
    // A scale about an anchor is exact in integers: (f - 8) * 3 / 2, half up.
    edit(
        &mut state,
        json!([{"op":"scale_keys","clip":"g","param":"amount","anchor":8,"num":1,"den":2}]),
    );
    assert_eq!(frames_of(&state, "amount"), [8, 15, 18]);
    // A scale that merges two keys is refused: nothing is reshuffled to make it fit.
    let before = footprint(&state);
    let error = edit_err(
        &mut state,
        json!([{"op":"scale_keys","clip":"g","param":"amount","anchor":8,"num":1,"den":10}]),
    );
    assert_eq!(error.code, "keyframe_order", "{error:?}");
    assert_eq!(footprint(&state), before);
    let error = edit_err(
        &mut state,
        json!([{"op":"scale_keys","clip":"g","anchor":8,"num":0,"den":2}]),
    );
    assert_eq!(error.code, "invalid_op", "{error:?}");
}

#[test]
fn a_clip_that_would_leave_a_key_behind_is_not_resized_and_the_seed_has_a_type() {
    let mut state = session();
    edit(&mut state, json!([grain(), key("amount", 30, json!(0.4))]));
    let before = footprint(&state);
    let error = edit_err(
        &mut state,
        json!([{"op":"set_field","clip":"g","field":"duration_frames","value":20}]),
    );
    assert_eq!(error.code, "invalid_bundle", "{error:?}");
    assert_eq!(issue_codes(&error), ["KEY_OUT_OF_CLIP"]);
    assert_eq!(footprint(&state), before);
    for seed in [json!(-1), json!(4_294_967_296_u64), json!(1.5), json!("7")] {
        let error = edit_err(
            &mut state,
            json!([{"op":"set_seed","clip":"g","seed":seed}]),
        );
        assert_eq!(error.code, "invalid_op", "{seed}: {error:?}");
    }
    let edge = edit(
        &mut state,
        json!([{"op":"set_seed","clip":"g","seed":4_294_967_295_u64}]),
    );
    assert_eq!(edge["changed"], true);
    edit(
        &mut state,
        json!([{"op":"set_seed","clip":"g","seed":null}]),
    );
    assert!(
        document(&state)["fx"]["tracks"][0]["clips"][0]
            .get("seed")
            .is_none()
    );
    // A look with no noise has no seed to set.
    edit(
        &mut state,
        json!([{"op":"add_clip","effect":"vignette","start_frame":60,"duration_frames":5,"id":"v"}]),
    );
    let error = edit_err(&mut state, json!([{"op":"set_seed","clip":"v","seed":3}]));
    assert_eq!(error.code, "not_applicable", "{error:?}");
}

// ---- gesture ----------------------------------------------------------------------------------------

#[test]
fn a_key_drag_keeps_the_last_legal_candidate_and_commits_as_one_entry() {
    let mut state = session();
    edit(
        &mut state,
        json!([
            grain(),
            key("amount", 5, json!(0.1)),
            key("amount", 15, json!(0.5)),
            key("amount", 25, json!(0.9))
        ]),
    );
    let before = footprint(&state);
    let gesture = ok(
        &mut state,
        json!({"op":"scene.fx.gesture.begin","scene":"street","clip":"g"}),
    )["gesture"]
        .as_str()
        .unwrap()
        .to_string();
    let update = |state: &mut EditorState, to: i64| {
        headless::execute(
            state,
            &json!({"op":"scene.fx.gesture.update","gesture":gesture,
                "ops":[{"op":"move_key","clip":"g","param":"amount","from":15,"to":to}]}),
        )
    };
    update(&mut state, 17).unwrap();
    update(&mut state, 20).unwrap();
    // 26 crosses the next key: refused, and the candidate on show stays at 20.
    let refused = update(&mut state, 26).unwrap_err();
    assert_eq!(refused.code, "keyframe_order", "{refused:?}");
    assert_eq!(refused.details.as_ref().unwrap()["candidate_kept"], true);
    let shown = ok(
        &mut state,
        json!({"op":"scene.fx.list","scene":"street","gesture":gesture}),
    );
    assert_eq!(
        shown["tracks"][0]["clips"][0]["keys"]["amount"][1]["frame"], 20,
        "the last legal candidate"
    );
    assert_eq!(footprint(&state), before, "a preview writes nothing");
    let status = ok(&mut state, json!({"op":"scene.fx.gesture.status"}));
    assert_eq!(status["action"], "scene.fx.gesture.status");
    let committed = ok(
        &mut state,
        json!({"op":"scene.fx.gesture.commit","gesture":gesture}),
    );
    assert_eq!(committed["changed"], true);
    assert_eq!(frames_of(&state, "amount"), [5, 20, 25]);
    assert_eq!(
        undo_depth(&state),
        before.1 + 1,
        "the whole drag is one entry"
    );
    assert_eq!(state.project.revision, before.2 + 1);
    // One undo gives back the exact document.
    ok(&mut state, json!({"op":"history.undo"}));
    assert_eq!(document(&state), before.0);
}

#[test]
fn a_cancelled_key_drag_leaves_nothing_and_a_drag_back_to_the_start_is_not_an_entry() {
    let mut state = session();
    edit(
        &mut state,
        json!([
            grain(),
            key("amount", 5, json!(0.1)),
            key("amount", 15, json!(0.5))
        ]),
    );
    let before = footprint(&state);
    let begin = |state: &mut EditorState| {
        ok(
            state,
            json!({"op":"scene.fx.gesture.begin","scene":"street","clip":"g"}),
        )["gesture"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let gesture = begin(&mut state);
    ok(
        &mut state,
        json!({"op":"scene.fx.gesture.update","gesture":gesture,
            "ops":[{"op":"shift_keys","clip":"g","delta":4}]}),
    );
    ok(
        &mut state,
        json!({"op":"scene.fx.gesture.cancel","gesture":gesture}),
    );
    assert_eq!(footprint(&state), before);
    // Back to the start: the candidate equals the base, so the commit writes nothing.
    let gesture = begin(&mut state);
    for delta in [4, 9, 0] {
        ok(
            &mut state,
            json!({"op":"scene.fx.gesture.update","gesture":gesture,
                "ops":[{"op":"shift_keys","clip":"g","delta":delta}]}),
        );
    }
    let committed = ok(
        &mut state,
        json!({"op":"scene.fx.gesture.commit","gesture":gesture}),
    );
    assert_eq!(committed["changed"], false);
    assert_eq!(footprint(&state), before);
}

// ---- persistence ------------------------------------------------------------------------------------

#[test]
fn keys_and_the_seed_survive_save_and_reopen_and_sample_the_same() {
    let mut state = session();
    edit(
        &mut state,
        json!([grain(), keyed("amount", 3, json!(0.3), "easeInOutCubic"),
               key("amount", 33, json!(0.9)), key("colored", 12, json!(true)),
               {"op":"set_seed","clip":"g","seed":12345}]),
    );
    let sampled: Vec<Value> = (10..50).map(|frame| pass_at(&mut state, frame)).collect();
    let scratch = std::env::temp_dir().join(format!("tessel-fx-keys-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).unwrap();
    let path = scratch.join("keys.tessel.json");
    persist::save(&mut state, &path).unwrap();
    let mut reopened = EditorState::empty(&ProjectSettings::default()).unwrap();
    ok(&mut reopened, json!({"op":"project.open","path":path}));
    assert_eq!(document(&reopened), document(&state));
    let again: Vec<Value> = (10..50)
        .map(|frame| pass_at(&mut reopened, frame))
        .collect();
    assert_eq!(
        again, sampled,
        "the reopened project samples every frame the same"
    );
    fs::remove_dir_all(&scratch).unwrap();
}
