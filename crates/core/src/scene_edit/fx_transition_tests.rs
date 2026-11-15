//! DEFENSIVE tests of the helper that gives an authored spelling back (`restore_authored_params`,
//! BL082.4.26, S2-C0a A02). They exercise the helper directly, with documents a public operation
//! cannot reach today: the only transition effect is `crossDissolve`, and two adjacent cuts differ in
//! both their shots, so the guard on each of `effect`, `fromShotId` and `toShotId` cannot be
//! told apart by a batch. They prove the guard of the helper, not an operation that exists; the
//! real case (an id removed on one cut and created on another) is in
//! `tests/scene_fx_transition/restoration.rs`.

use core::prelude::v1::test;

use serde_json::{Value, json};

use super::fx_transition::restore_authored_params;

fn document(effect: &str, from: &str, to: &str, intensity: Value) -> Value {
    json!({"fx": {"transitions": [{
        "id": "x", "effect": effect, "fromShotId": from, "toShotId": to,
        "params": {"curve": "smooth", "intensity": intensity},
    }]}})
}

fn intensity_after(original: &Value, mut work: Value) -> Value {
    restore_authored_params(original, &mut work);
    work["fx"]["transitions"][0]["params"]["intensity"].clone()
}

#[test]
fn the_same_entity_gets_its_authored_spelling_back() {
    let original = document("crossDissolve", "a", "b", json!(1));
    let work = document("crossDissolve", "a", "b", json!(1.0));
    assert_eq!(intensity_after(&original, work), json!(1));
}

#[test]
fn a_different_effect_is_another_entity_and_keeps_its_representation() {
    let original = document("crossDissolve", "a", "b", json!(1));
    let work = document("somethingElse", "a", "b", json!(1.0));
    assert_eq!(intensity_after(&original, work).to_string(), "1.0");
}

#[test]
fn a_different_outgoing_shot_is_another_entity_and_keeps_its_representation() {
    let original = document("crossDissolve", "a", "b", json!(1));
    let work = document("crossDissolve", "z", "b", json!(1.0));
    assert_eq!(intensity_after(&original, work).to_string(), "1.0");
}

#[test]
fn a_different_incoming_shot_is_another_entity_and_keeps_its_representation() {
    let original = document("crossDissolve", "a", "b", json!(1));
    let work = document("crossDissolve", "a", "z", json!(1.0));
    assert_eq!(intensity_after(&original, work).to_string(), "1.0");
}

#[test]
fn a_value_that_really_changed_is_never_restored() {
    let original = document("crossDissolve", "a", "b", json!(1));
    let work = document("crossDissolve", "a", "b", json!(0.5));
    assert_eq!(intensity_after(&original, work), json!(0.5));
}
