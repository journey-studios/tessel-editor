//! P4b S2-C0b (BL082.4.21): the hostile-input ceilings of a plan with a transition, met by a plan
//! that is *valid* rather than by nulls: exactly 20,000 items and exactly 4,194,304 bytes are
//! accepted by the three doors, one item or one byte more is refused as `/transition`.
//!
//! The plan is built in memory (nothing is committed): the real window plan of the spec edit at
//! frame 44 with one `fade` pass of each shot, then cloned passes with fresh clip ids. The item
//! count is taken here, from the literal length of the six arrays of each deck, never from the
//! code under test. A valid plan from outside is not a promise that a bundle can make one: the
//! document's own quotas (`MAX_FX_TRACKS`) stop a bundle far below these numbers.

use core::prelude::v1::test;

use serde_json::{Value, json};
use tessel_editor_core::scene_plan::{
    MAX_TRANSITION_PLAN_BYTES, MAX_TRANSITION_PLAN_ITEMS, PlanError, ScenePlan,
};

use crate::support::*;

const ARRAYS: [&str; 6] = ["nodes", "layers", "splats", "fx", "resources", "lights"];

/// The window plan at frame 44 (`s1` under the needle, `s2` the neighbour), each shot with a
/// `fade` pass and each scene definition with `lamps` point lights, as a JSON tree.
fn base(lamps: (usize, usize)) -> Value {
    let mut bundle = with_transitions(Edit::spec().bundle(), vec![tr(json!({}))]);
    bundle["fx"]["tracks"] = json!([
        shot_clip("ta", "ca", "fade", "s1", 0, 48),
        shot_clip("tb", "cb", "fade", "s2", 48, 48)
    ]);
    for (scene, count) in [("scene-a", lamps.0), ("scene-b", lamps.1)] {
        let definition = bundle["sceneDefinitions"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|definition| definition["id"] == scene)
            .unwrap();
        let lights: Vec<Value> = (0..count)
            .map(|index| {
                json!({"id": format!("lamp-{index}"), "position": {"x": 0, "y": 1, "z": 2},
                       "color": {"r": 1, "g": 1, "b": 1}, "intensity": 2, "radius": 5,
                       "flicker": 0})
            })
            .collect();
        definition["lighting"] = json!({"ambient": 1, "lights": lights});
    }
    plan_at(&scene_of(bundle), 44).to_json()
}

/// The lengths of the six arrays of one deck, in `ARRAYS` order.
fn lengths(deck: &Value) -> [usize; 6] {
    ARRAYS.map(|key| deck.get(key).and_then(Value::as_array).map_or(0, Vec::len))
}

/// Both decks' items, counted by hand from the arrays of the tree.
fn counted(value: &Value) -> usize {
    let own: usize = lengths(value).iter().sum();
    let other: usize = lengths(&value["transition"]["other"]).iter().sum();
    own + other
}

/// Grows the passes of one deck to `total` by cloning its first, with short fresh ids.
fn grow(deck: &mut Value, prefix: &str, total: usize) {
    let template = deck["fx"][0].clone();
    let list = deck["fx"].as_array_mut().unwrap();
    assert_eq!(list.len(), 1, "one real pass to clone");
    list.reserve(total);
    for index in 1..total {
        let mut pass = template.clone();
        pass["trackId"] = json!("t");
        pass["clipId"] = json!(format!("{prefix}{index}"));
        list.push(pass);
    }
}

/// A plan with `fades` passes in each deck and `lamps` point lights in them.
fn plan_of(fades: (usize, usize), lamps: (usize, usize)) -> Value {
    let mut value = base(lamps);
    grow(&mut value, "a", fades.0);
    grow(&mut value["transition"]["other"], "b", fades.1);
    value
}

fn invalid(error: PlanError) -> (String, String) {
    match error {
        PlanError::Invalid { pointer, message } => (pointer, message),
        other => panic!("expected a pointed refusal: {other:?}"),
    }
}

/// The three doors refuse the count with `/transition` and the number in the message.
fn refused_as_a_count(value: &Value, what: &str) {
    let (pointer, message) = invalid(ScenePlan::from_json(value).expect_err(what));
    assert_eq!(pointer, "/transition", "{what}");
    assert!(message.contains("20000"), "{what}: {message}");
    let text = value.to_string();
    assert!(
        text.len() < MAX_TRANSITION_PLAN_BYTES,
        "{what}: the refusal must be the count's, not the bytes' ({} bytes)",
        text.len()
    );
    let (pointer, message) = invalid(ScenePlan::from_json_str(&text).expect_err(what));
    assert_eq!(pointer, "/transition", "{what}");
    assert!(message.contains("20000"), "{what}: {message}");
}

/// Accepted by the three doors, complete, equal after its own wire form.
fn accepted_as_a_whole(value: &Value, what: &str) -> ScenePlan {
    let plan = ScenePlan::from_json(value).unwrap_or_else(|error| panic!("{what}: {error:?}"));
    plan.validate().expect(what);
    assert!(plan.is_complete(), "{what}");
    let text = plan.to_json_string();
    assert!(
        text.len() <= MAX_TRANSITION_PLAN_BYTES,
        "{what}: {}",
        text.len()
    );
    assert_eq!(ScenePlan::from_json_str(&text).expect(what), plan, "{what}");
    assert_eq!(
        ScenePlan::from_json_str(&value.to_string()).expect(what),
        plan,
        "{what}"
    );
    plan
}

#[test]
fn a_valid_plan_of_exactly_twenty_thousand_items_is_accepted_by_the_three_doors() {
    // The base: 3 nodes, 2 layers, no splats, one pass, 2 resources, no light, in each deck.
    let small = base((0, 0));
    assert_eq!(lengths(&small), [3, 2, 0, 1, 2, 0]);
    assert_eq!(lengths(&small["transition"]["other"]), [3, 2, 0, 1, 2, 0]);

    let value = plan_of((9_993, 9_993), (0, 0));
    assert_eq!(lengths(&value), [3, 2, 0, 9_993, 2, 0]);
    assert_eq!(
        lengths(&value["transition"]["other"]),
        [3, 2, 0, 9_993, 2, 0]
    );
    assert_eq!(counted(&value), 20_000);
    assert_eq!(counted(&value), MAX_TRANSITION_PLAN_ITEMS);
    let plan = accepted_as_a_whole(&value, "20000 items");
    assert_eq!(plan.fx.len(), 9_993);
    assert_eq!(plan.transition.as_deref().unwrap().other.fx.len(), 9_993);
    // The text stays under the byte ceiling by itself: this is the item ceiling's own limit.
    let bytes = plan.to_json_string().len();
    assert!(bytes < MAX_TRANSITION_PLAN_BYTES, "{bytes}");
}

#[test]
fn one_more_valid_pass_in_either_deck_is_refused_by_the_three_doors() {
    for fades in [(9_994, 9_993), (9_993, 9_994)] {
        let value = plan_of(fades, (0, 0));
        assert_eq!(counted(&value), 20_001);
        refused_as_a_count(&value, &format!("{fades:?}"));
    }
    // The count of the decks together, not of each: half in one is not over.
    let lopsided = plan_of((14_000, 5_986), (0, 0));
    assert_eq!(counted(&lopsided), 20_000);
    accepted_as_a_whole(&lopsided, "20000 items, lopsided");
    let over = plan_of((14_001, 5_986), (0, 0));
    assert_eq!(counted(&over), 20_001);
    refused_as_a_count(&over, "lopsided and over");
}

#[test]
fn the_typed_plan_is_judged_by_the_same_count() {
    let mut plan = accepted_as_a_whole(&plan_of((9_993, 9_993), (0, 0)), "20000 items");
    let extra = plan.fx[1].clone();
    let mut pass = extra;
    pass.clip_id = "typed-extra".into();
    plan.fx.push(pass);
    let (pointer, message) = invalid(plan.validate().expect_err("20001 typed items"));
    assert_eq!(pointer, "/transition");
    assert!(message.contains("20000"), "{message}");
    // The neighbour's items are in the same count.
    let mut plan = accepted_as_a_whole(&plan_of((9_993, 9_993), (0, 0)), "20000 items");
    let transition = plan.transition.as_deref_mut().unwrap();
    let mut pass = transition.other.fx[1].clone();
    pass.clip_id = "typed-neighbour".into();
    transition.other.fx.push(pass);
    let (pointer, _) = invalid(plan.validate().expect_err("20001 typed items"));
    assert_eq!(pointer, "/transition");
}

#[test]
fn lights_and_the_neighbour_count_toward_the_same_ceiling() {
    // 19,999 items that are not lights, plus one valid light: exactly 20,000.
    let one = plan_of((9_993, 9_992), (1, 0));
    assert_eq!(lengths(&one)[5], 1);
    assert_eq!(counted(&one), 20_000);
    accepted_as_a_whole(&one, "19999 + one light");
    // A second light in the same deck (two of the four allowed) is the 20,001st item.
    let same = plan_of((9_993, 9_992), (2, 0));
    assert_eq!(lengths(&same)[5], 2);
    assert_eq!(counted(&same), 20_001);
    refused_as_a_count(&same, "a second light in the primary");
    // A light of the neighbour is counted too: one in each deck over 19,999 is 20,001.
    let both = plan_of((9_993, 9_992), (1, 1));
    assert_eq!(lengths(&both["transition"]["other"])[5], 1);
    assert_eq!(counted(&both), 20_001);
    refused_as_a_count(&both, "a light in each deck");
    // And one in each deck with one pass fewer is exactly the ceiling again.
    let fits = plan_of((9_993, 9_991), (1, 1));
    assert_eq!(counted(&fits), 20_000);
    accepted_as_a_whole(&fits, "19998 + a light in each deck");
    // The lights alone, at the limit of four a deck, do not reach it.
    let lamps = plan_of((1, 1), (4, 4));
    assert_eq!(counted(&lamps), 2 * (3 + 2 + 1 + 2 + 4));
    accepted_as_a_whole(&lamps, "four lights in each deck");
}

fn padded(text: &str, length: usize) -> String {
    assert!(text.len() <= length, "{} > {length}", text.len());
    let mut padded = String::with_capacity(length);
    padded.push_str(text);
    padded.extend(std::iter::repeat_n(' ', length - text.len()));
    padded
}

#[test]
fn a_valid_plan_of_exactly_the_byte_ceiling_is_accepted_and_one_byte_more_is_refused() {
    // The same twenty-thousand-item plan, whitespace to the exact byte.
    let value = plan_of((9_993, 9_993), (0, 0));
    let plan = accepted_as_a_whole(&value, "20000 items");
    let text = plan.to_json_string();
    assert_eq!(counted(&value), 20_000);
    let at = padded(&text, MAX_TRANSITION_PLAN_BYTES);
    assert_eq!(at.len(), 4_194_304);
    assert_eq!(ScenePlan::from_json_str(&at).unwrap(), plan);
    let above = padded(&text, MAX_TRANSITION_PLAN_BYTES + 1);
    let (pointer, message) = invalid(ScenePlan::from_json_str(&above).unwrap_err());
    assert_eq!(pointer, "/transition");
    assert!(message.contains("4194304"), "{message}");
    assert!(
        message.contains(&above.len().to_string()),
        "the refusal names the length it found: {message}"
    );
    // The same plan with the key spelled by an escape is found after parsing, to the same byte.
    let escaped = text.replacen("\"transition\"", "\"transitio\\u006e\"", 1);
    let at = padded(&escaped, MAX_TRANSITION_PLAN_BYTES);
    assert_eq!(ScenePlan::from_json_str(&at).unwrap(), plan);
    let above = padded(&escaped, MAX_TRANSITION_PLAN_BYTES + 1);
    assert_eq!(
        invalid(ScenePlan::from_json_str(&above).unwrap_err()).0,
        "/transition"
    );
}
