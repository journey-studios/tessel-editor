//! BL082.4.26 (permanent, S2-C0a A02): the authored spelling a numeric round trip gives back
//! belongs to the entity that was authored. A transition removed in a batch and another one
//! created with the same id on another cut are two entities, so the new one keeps its own
//! representation even when its typed value equals what the old one spelled differently.

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::scene_edit::{EditError, FxOutcome, apply_fx, parse_fx_batch};

use crate::support::*;

fn run(scene: &DirectedScene, ops: Value) -> Result<FxOutcome, EditError> {
    apply_fx(scene, &parse_fx_batch(&ops).unwrap())
}

fn written(scene: &DirectedScene) -> Vec<Value> {
    scene
        .document()
        .pointer("/fx/transitions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// `x` on the first cut with the intensity spelled as the integer `1`.
fn authored() -> DirectedScene {
    spec_scene(vec![tr(json!({
        "id": "x", "durationFrames": 12, "params": {"curve": "smooth", "intensity": 1},
    }))])
}

fn recreate_on(from: &str, to: &str) -> Value {
    json!([
        {"op": "remove_transition", "id": "x"},
        {"op": "set_transition", "fromShotId": from, "toShotId": to, "effect": "crossDissolve",
         "id": "x", "durationFrames": 12, "params": {"curve": "smooth", "intensity": 1}},
    ])
}

#[test]
fn a_transition_recreated_on_another_cut_keeps_its_own_spelling() {
    let outcome = run(&authored(), recreate_on("s2", "s3")).unwrap();
    assert!(outcome.summary.modified);
    let all = written(&outcome.candidate);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0]["id"], "x");
    assert_eq!(all[0]["fromShotId"], "s2");
    assert_eq!(all[0]["toShotId"], "s3");
    // A new transition is written as the catalog types it (`1.0`); the integer `1` the removed
    // entity spelled is not carried over to it, though the typed values are equal.
    assert_eq!(all[0]["params"]["intensity"].to_string(), "1.0");
    assert_ne!(all[0]["params"]["intensity"], json!(1));
    // The same typed value, on the same cut, with the same id, is still the authored one: the
    // restoration works where the entity is the same (the control of the guard).
    let control = run(
        &authored(),
        json!([
            {"op": "set_transition_param", "id": "x", "param": "intensity", "value": 0.5},
            {"op": "set_transition_param", "id": "x", "param": "intensity", "value": 1},
        ]),
    )
    .unwrap();
    assert!(!control.summary.modified);
    assert_eq!(written(&control.candidate), written(&authored()));
    assert_eq!(written(&authored())[0]["params"]["intensity"], json!(1));
}

#[test]
fn the_other_way_round_the_new_cut_is_not_given_the_old_spelling_either() {
    // The authored transition is on the second cut: recreating it on the first must not borrow it.
    let scene = spec_scene(vec![tr(json!({
        "id": "x", "fromShotId": "s2", "toShotId": "s3", "durationFrames": 12,
        "params": {"curve": "smooth", "intensity": 1},
    }))]);
    let outcome = run(&scene, recreate_on("s1", "s2")).unwrap();
    let all = written(&outcome.candidate);
    assert_eq!(
        (all[0]["fromShotId"].as_str(), all[0]["toShotId"].as_str()),
        (Some("s1"), Some("s2"))
    );
    assert_eq!(all[0]["params"]["intensity"].to_string(), "1.0");
}
