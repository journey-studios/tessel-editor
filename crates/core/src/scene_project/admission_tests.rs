//! F064 / P4b S1a: the transition policy survives the editor's own edit paths. An `Accept` scene
//! can only be inserted from inside the crate (`SceneBook::insert` is `pub(crate)`), so the
//! integration tests cannot reach this seam; here the real executor (`scene.edit`,
//! `scene.fx.edit`, undo and redo) runs on such a scene and the policy must never change.

use core::prelude::v1::test;
use std::sync::Arc;

use serde_json::{Map, Value, json};

use super::AttachedScene;
use crate::directed_scene::fx::TransitionAdmission;
use crate::directed_scene::{DirectedScene, Limits};
use crate::state::ProjectSettings;
use crate::{EditorState, headless};

const CUT: &str = include_str!(
    "../../../../docs/migration/fixtures/bundles/valid-cut-instance-offsets.bundle.json"
);

fn bundle(with_transition: bool) -> Value {
    let mut value: Value = serde_json::from_str(CUT).unwrap();
    let list = if with_transition {
        json!([{"id": "tr", "effect": "crossDissolve", "fromShotId": "shot-a",
                "toShotId": "shot-b", "durationFrames": 12, "alignment": "center",
                "handles": "hold", "easing": "easeInOutCubic"}])
    } else {
        json!([])
    };
    value["fx"] = json!({"catalogVersion": 1, "tracks": [], "transitions": list});
    value
}

fn state_with(policy: TransitionAdmission) -> EditorState {
    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    let scene = DirectedScene::import_value_admitting(
        bundle(policy == TransitionAdmission::Accept),
        &Limits::default(),
        policy,
    )
    .unwrap();
    state
        .scenes
        .insert(AttachedScene {
            id: "street".into(),
            name: "street".into(),
            scene: Arc::new(scene),
            extra: Map::new(),
        })
        .unwrap();
    state
}

fn run(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn policy(state: &EditorState) -> TransitionAdmission {
    state.scenes.get("street").unwrap().scene.admission()
}

fn transitions(state: &EditorState) -> usize {
    state
        .scenes
        .get("street")
        .unwrap()
        .scene
        .bundle()
        .fx
        .transitions
        .len()
}

#[test]
fn every_edit_and_undo_keeps_the_policy_and_the_transition_of_an_accept_scene() {
    let mut state = state_with(TransitionAdmission::Accept);
    assert_eq!(
        (policy(&state), transitions(&state)),
        (TransitionAdmission::Accept, 1)
    );
    let document = state.scenes.get("street").unwrap().scene.document().clone();

    run(
        &mut state,
        json!({"op": "scene.fx.edit", "scene": "street", "ops": [
            {"op": "add_clip", "effect": "vignette", "start_frame": 0,
             "duration_frames": 10, "id": "v1"}]}),
    );
    assert_eq!(
        (policy(&state), transitions(&state)),
        (TransitionAdmission::Accept, 1)
    );

    run(
        &mut state,
        json!({"op": "scene.edit", "scene": "street", "ops": [
            {"set": "scene/scene-a/node/backdrop/width", "value": 2}]}),
    );
    assert_eq!(
        (policy(&state), transitions(&state)),
        (TransitionAdmission::Accept, 1)
    );

    // A refused edit leaves the scene, and so its policy, alone.
    assert!(
        headless::execute(
            &mut state,
            &json!({"op": "scene.fx.edit", "scene": "street", "ops": [
            {"op": "add_clip", "effect": "crossDissolve", "start_frame": 0}]})
        )
        .is_err()
    );
    assert_eq!(
        (policy(&state), transitions(&state)),
        (TransitionAdmission::Accept, 1)
    );

    for _ in 0..2 {
        run(&mut state, json!({"op": "history.undo"}));
        assert_eq!(
            (policy(&state), transitions(&state)),
            (TransitionAdmission::Accept, 1)
        );
    }
    assert_eq!(
        state.scenes.get("street").unwrap().scene.document(),
        &document
    );
    for _ in 0..2 {
        run(&mut state, json!({"op": "history.redo"}));
        assert_eq!(
            (policy(&state), transitions(&state)),
            (TransitionAdmission::Accept, 1)
        );
    }
}

#[test]
fn a_refuse_scene_never_becomes_accept_through_an_edit() {
    let mut state = state_with(TransitionAdmission::Refuse);
    run(
        &mut state,
        json!({"op": "scene.fx.edit", "scene": "street", "ops": [
            {"op": "add_clip", "effect": "vignette", "start_frame": 0,
             "duration_frames": 10, "id": "v1"}]}),
    );
    assert_eq!(
        (policy(&state), transitions(&state)),
        (TransitionAdmission::Refuse, 0)
    );
    run(
        &mut state,
        json!({"op": "scene.edit", "scene": "street", "ops": [
            {"set": "scene/scene-a/node/backdrop/width", "value": 2}]}),
    );
    assert_eq!(
        (policy(&state), transitions(&state)),
        (TransitionAdmission::Refuse, 0)
    );
    run(&mut state, json!({"op": "history.undo"}));
    assert_eq!(policy(&state), TransitionAdmission::Refuse);

    // And the document the edit would need to carry a transition is refused by the candidate.
    let error = headless::execute(
        &mut state,
        &json!({"op": "scene.attach", "id": "other", "bundle": bundle(true)}),
    )
    .unwrap_err();
    assert_eq!(error.code, "invalid_bundle");
}
