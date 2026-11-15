//! BL082.4.29 (S2-C0a): an id is not an identity. A problem the importer finds on the entity the
//! candidate holds is attributed to the ops that touched THAT entity, not to every op of the
//! batch that spelled its id. An id removed and created again is another entity, in the same kind
//! as well as across kinds (the clip/transition case is BL082.4.28, pinned in `edit.rs`).

use serde_json::{Value, json};
use tessel_editor_core::scene_edit::{EditError, apply_fx, parse_fx_batch};

use crate::support::*;

fn refused(scene: &tessel_editor_core::directed_scene::DirectedScene, ops: Value) -> EditError {
    let error = apply_fx(scene, &parse_fx_batch(&ops).unwrap()).expect_err("the batch is refused");
    assert_eq!(error.code, "CANDIDATE_REJECTED", "{error:?}");
    error
}

fn ops_of(error: &EditError) -> Vec<Vec<usize>> {
    error
        .issues()
        .iter()
        .map(|localized| localized.ops.clone())
        .collect()
}

fn create(id: &str, from: &str, to: &str, duration: i64) -> Value {
    json!({"op": "set_transition", "fromShotId": from, "toShotId": to, "effect": "crossDissolve",
           "id": id, "durationFrames": duration})
}

#[test]
fn only_the_op_that_made_the_surviving_entity_answers_for_it() {
    // Created on s1->s2, removed, created again on s2->s3 and too long for its shots: the first two
    // ops never touched the `x` of the candidate.
    let scene = spec_scene(vec![]);
    let error = refused(
        &scene,
        json!([
            create("x", "s1", "s2", 12),
            {"op": "remove_transition", "id": "x"},
            create("x", "s2", "s3", 500),
        ]),
    );
    assert!(!error.issues().is_empty());
    for ops in ops_of(&error) {
        assert_eq!(ops, vec![2]);
    }
    assert_eq!(error.index, Some(2));
    let errors = error.details().unwrap()["errors"].as_array().unwrap();
    assert_eq!(errors[0]["details"]["ops"], json!([2]));
}

#[test]
fn an_entity_of_the_document_that_was_replaced_is_not_the_one_that_failed() {
    // `x` is in the document; the batch resizes it, removes it and creates another `x`, wrong now.
    let scene = spec_scene(vec![tr(json!({"id": "x"}))]);
    let error = refused(
        &scene,
        json!([
            {"op": "resize_transition", "id": "x", "durationFrames": 6},
            {"op": "remove_transition", "id": "x"},
            create("x", "s2", "s3", 500),
        ]),
    );
    for ops in ops_of(&error) {
        assert_eq!(ops, vec![2]);
    }
    assert_eq!(error.index, Some(2));
}

#[test]
fn every_op_on_the_surviving_entity_still_answers_for_it() {
    // The control: nothing was removed, so both ops touched the `x` that fails.
    let scene = spec_scene(vec![tr(json!({"id": "x"}))]);
    let error = refused(
        &scene,
        json!([
            {"op": "resize_transition", "id": "x", "durationFrames": 6},
            {"op": "resize_transition", "id": "x", "durationFrames": 500},
        ]),
    );
    for ops in ops_of(&error) {
        assert_eq!(ops, vec![0, 1]);
    }
    assert_eq!(error.index, Some(0));
    // An upsert of an existing cut is the same entity too: it does not start a generation.
    let error = refused(
        &scene,
        json!([
            create("x", "s1", "s2", 6),
            {"op": "resize_transition", "id": "x", "durationFrames": 500},
        ]),
    );
    for ops in ops_of(&error) {
        assert_eq!(ops, vec![0, 1]);
    }
}

#[test]
fn an_entity_removed_for_good_leaves_no_issue_to_attribute() {
    // Another transition fails; the ops on a removed `x` are not blamed for it.
    let scene = spec_scene(vec![
        tr(json!({"id": "x"})),
        tr(json!({"id": "y", "fromShotId": "s2", "toShotId": "s3"})),
    ]);
    let error = refused(
        &scene,
        json!([
            {"op": "resize_transition", "id": "x", "durationFrames": 6},
            {"op": "remove_transition", "id": "x"},
            {"op": "resize_transition", "id": "y", "durationFrames": 500},
        ]),
    );
    for ops in ops_of(&error) {
        assert_eq!(ops, vec![2]);
    }
}
