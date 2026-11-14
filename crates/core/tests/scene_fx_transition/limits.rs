//! M3 limits: a plan that carries a `transition` has hostile-input ceilings fixed by the S0
//! contract (bytes 4194304, items 20000, depth 1). A plan without one has none of them. The numbers
//! are not changed to fit a test.

use core::prelude::v1::test;

use serde_json::{Value, json};
use tessel_editor_core::scene_plan::{
    MAX_TRANSITION_PLAN_BYTES, MAX_TRANSITION_PLAN_ITEMS, PlanError, ScenePlan,
};

use crate::support::*;

fn window_text() -> String {
    plan_at(&spec_scene(vec![tr(json!({}))]), 44).to_json_string()
}

fn refusal(text: &str) -> (String, String) {
    match ScenePlan::from_json_str(text) {
        Err(PlanError::Invalid { pointer, message }) => (pointer, message),
        other => panic!(
            "expected an invalid plan, got {:?}",
            other.map(|_| "a plan")
        ),
    }
}

#[test]
fn the_ceilings_are_the_contracted_numbers() {
    assert_eq!(MAX_TRANSITION_PLAN_BYTES, 4_194_304);
    assert_eq!(MAX_TRANSITION_PLAN_ITEMS, 20_000);
}

/// Padding with whitespace keeps the plan the same plan while its text grows to the exact byte.
fn padded(text: &str, length: usize) -> String {
    assert!(text.len() <= length);
    let mut padded = String::with_capacity(length);
    padded.push_str(text);
    padded.extend(std::iter::repeat_n(' ', length - text.len()));
    padded
}

#[test]
fn a_text_of_exactly_the_byte_ceiling_is_accepted_and_one_more_is_refused() {
    let text = window_text();
    let plan = ScenePlan::from_json_str(&text).unwrap();
    assert!(
        text.len() < 100_000,
        "the real plan of the corpus edit is {} bytes, far under the ceiling",
        text.len()
    );
    let at_limit = padded(&text, MAX_TRANSITION_PLAN_BYTES);
    assert_eq!(at_limit.len(), 4_194_304);
    assert_eq!(ScenePlan::from_json_str(&at_limit).unwrap(), plan);
    let above = padded(&text, MAX_TRANSITION_PLAN_BYTES + 1);
    let (pointer, message) = refusal(&above);
    assert_eq!(pointer, "/transition");
    assert!(message.contains("4194304"), "{message}");
}

#[test]
fn a_plan_without_a_transition_has_no_such_ceiling() {
    let plain = plan_at(&spec_scene(vec![tr(json!({}))]), 20);
    let text = plain.to_json_string();
    let big = padded(&text, MAX_TRANSITION_PLAN_BYTES + 4096);
    assert_eq!(ScenePlan::from_json_str(&big).unwrap(), plain);
}

#[test]
fn a_text_over_the_ceiling_with_a_transition_is_refused_before_it_is_parsed() {
    // A truncated body would be a JSON error if the text were read: the refusal comes first, so the
    // answer is the ceiling's, not a parser's, and nothing was allocated for the tree.
    let mut text = String::from("{\"transition\":");
    text.extend(std::iter::repeat_n('[', MAX_TRANSITION_PLAN_BYTES));
    let (pointer, _) = refusal(&text);
    assert_eq!(pointer, "/transition");
}

#[test]
fn the_key_spelled_with_an_escape_is_refused_right_after_parsing() {
    let text = window_text().replacen("\"transition\"", "\"transitio\\u006e\"", 1);
    assert!(text.contains("\\u006e"));
    assert!(
        ScenePlan::from_json_str(&text).is_ok(),
        "the escape alone is a valid plan"
    );
    let above = padded(&text, MAX_TRANSITION_PLAN_BYTES + 1);
    assert_eq!(refusal(&above).0, "/transition");
}

/// The count comes from the array lengths, before any item is read: twenty thousand and one nulls
/// are refused as a count, twenty thousand are read (and refused as nulls, for a different reason).
#[test]
fn the_item_ceiling_is_counted_from_the_arrays_before_any_item_is_read() {
    let mut value: Value = serde_json::from_str(&window_text()).unwrap();
    value["transition"]["other"]["nodes"] =
        Value::Array(vec![Value::Null; MAX_TRANSITION_PLAN_ITEMS]);
    value["nodes"] = json!([]);
    value["layers"] = json!([]);
    value["resources"] = json!([]);
    value["transition"]["other"]["layers"] = json!([]);
    value["transition"]["other"]["resources"] = json!([]);
    value["transition"]["other"]["fx"] = json!([]);
    value["fx"] = json!([]);
    // At the limit the count passes: the failure is the first null, not the ceiling.
    let at = match ScenePlan::from_json(&value) {
        Err(PlanError::Invalid { pointer, message }) => (pointer, message),
        other => panic!("{:?}", other.map(|_| "a plan")),
    };
    assert_ne!(at.0, "/transition", "{at:?}");
    assert!(!at.1.contains("20000"), "{at:?}");
    // One above: the count is what is refused.
    value["transition"]["other"]["nodes"] =
        Value::Array(vec![Value::Null; MAX_TRANSITION_PLAN_ITEMS + 1]);
    let above = match ScenePlan::from_json(&value) {
        Err(PlanError::Invalid { pointer, message }) => (pointer, message),
        other => panic!("{:?}", other.map(|_| "a plan")),
    };
    assert_eq!(above.0, "/transition");
    assert!(above.1.contains("20000"), "{above:?}");
    // Both decks count together: half in each, one over.
    let mut split: Value = serde_json::from_str(&window_text()).unwrap();
    split["nodes"] = Value::Array(vec![Value::Null; MAX_TRANSITION_PLAN_ITEMS / 2]);
    split["transition"]["other"]["nodes"] =
        Value::Array(vec![Value::Null; MAX_TRANSITION_PLAN_ITEMS / 2 + 1]);
    for deck in ["layers", "resources", "fx", "lights", "splats"] {
        split[deck] = json!([]);
        split["transition"]["other"][deck] = json!([]);
    }
    let (pointer, message) = match ScenePlan::from_json(&split) {
        Err(PlanError::Invalid { pointer, message }) => (pointer, message),
        other => panic!("{:?}", other.map(|_| "a plan")),
    };
    assert_eq!(
        (pointer.as_str(), message.contains("20000")),
        ("/transition", true)
    );
}

#[test]
fn nesting_and_hostile_shapes_are_refused_without_recursion_or_panic() {
    let text = window_text();
    let value: Value = serde_json::from_str(&text).unwrap();
    // Depth 2.
    let mut nested = value.clone();
    nested["transition"]["other"]["transition"] = value["transition"].clone();
    assert_eq!(
        refusal(&nested.to_string()).0,
        "/transition/other/transition"
    );
    // A bomb: a thousand deep, built by hand. The reader never descends into it.
    let mut bomb = json!({"x": 1});
    for _ in 0..1000 {
        bomb = json!({"transition": {"other": bomb}});
    }
    let mut hostile = value.clone();
    hostile["transition"]["other"]["transition"] = bomb;
    // As a tree: refused at the nesting, before the neighbour's transition is read at all.
    match ScenePlan::from_json(&hostile) {
        Err(PlanError::Invalid { pointer, .. }) => {
            assert_eq!(pointer, "/transition/other/transition");
        }
        other => panic!("{:?}", other.map(|_| "a plan")),
    }
    // As text: the JSON parser's own depth limit refuses it first, with no plan read.
    assert!(ScenePlan::from_json_str(&hostile.to_string()).is_err());
    // Truncated text, wrong types, empty and absurd counts: all refusals, none a panic.
    for cut in [1, 2, text.len() / 3, text.len() / 2, text.len() - 1] {
        assert!(ScenePlan::from_json_str(&text[..cut]).is_err(), "cut {cut}");
    }
    for hostile in [
        "",
        "null",
        "[]",
        "1",
        "\"x\"",
        "{\"transition\":null}",
        "{\"transition\":[]}",
    ] {
        assert!(ScenePlan::from_json_str(hostile).is_err(), "{hostile}");
    }
    let mut huge = value.clone();
    huge["transition"]["params"] = json!(vec![json!(["curve", "smooth"]); 100_000]);
    assert!(ScenePlan::from_json(&huge).is_err());
    // NaN and infinity are not JSON: the text is refused; a tree cannot hold them either.
    for text in [
        text.replacen("\"progress\":", "\"progress\":NaN,\"x\":", 1),
        text.replacen("\"amount\":", "\"amount\":Infinity,\"x\":", 1),
    ] {
        assert!(ScenePlan::from_json_str(&text).is_err());
    }
}

/// The ceiling is on the plan a builder hands out too: one that could not be read back is never
/// returned as a success.
#[test]
fn a_built_plan_is_always_readable_back() {
    let scene = spec_scene(vec![tr(json!({}))]);
    for frame in 42..54 {
        let plan = plan_at(&scene, frame);
        assert!(plan.to_json_string().len() <= MAX_TRANSITION_PLAN_BYTES);
        assert_eq!(
            ScenePlan::from_json_str(&plan.to_json_string()).unwrap(),
            plan
        );
    }
}
