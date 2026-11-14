//! Independent review regressions: hostile wire/typed plans, deck context and item ceilings.

use core::prelude::v1::test;
use serde_json::{Value, json};
use tessel_editor_core::scene_plan::{MAX_TRANSITION_PLAN_ITEMS, PlanError, ScenePlan};

use crate::support::*;

fn plan() -> ScenePlan {
    plan_at(&spec_scene(vec![tr(json!({}))]), 44)
}

fn pointer(error: PlanError) -> String {
    match error {
        PlanError::Invalid { pointer, .. } => pointer,
        other => panic!("expected a deck-qualified invalid plan: {other:?}"),
    }
}

fn refused_in_both_doors(plan: &ScenePlan, at: &str) {
    let before = plan.to_json_string();
    assert_eq!(pointer(plan.validate().unwrap_err()), at);
    assert_eq!(
        pointer(ScenePlan::from_json(&plan.to_json()).unwrap_err()),
        at
    );
    assert_eq!(pointer(ScenePlan::from_json_str(&before).unwrap_err()), at);
    assert_eq!(plan.to_json_string(), before, "validation is read-only");
}

#[test]
fn two_failed_frame_conversions_never_make_a_valid_window_relation() {
    let mut plan = plan();
    plan.frame.edit_frame = (i64::MAX as u64) + 6;
    let t = plan.transition.as_mut().unwrap();
    t.other.frame.edit_frame = plan.frame.edit_frame;
    t.start_frame = i64::MAX;
    t.frame_in_window = 1;
    t.progress = 0.0078125; // 4 * ((1 + 0.5) / 12)^3, independently fixed.
    refused_in_both_doors(&plan, "/transition/frameInWindow");
}

#[test]
fn equal_edit_frames_do_not_allow_different_real_edit_times() {
    let mut plan = plan();
    let time = plan.frame.edit_time;
    plan.transition.as_mut().unwrap().other.frame.edit_time =
        tessel_editor_core::time::Ratio::new(2, 1).unwrap();
    assert_ne!(
        plan.transition.as_ref().unwrap().other.frame.edit_time,
        time
    );
    refused_in_both_doors(&plan, "/transition/other/frame/editTime");
}

#[test]
fn two_shot_passes_cannot_reuse_a_clip_id_across_decks() {
    let scene = spec_scene_with_tracks(
        vec![tr(json!({}))],
        json!([
            shot_clip("ta", "ca", "vignette", "s1", 0, 48),
            shot_clip("tb", "cb", "vignette", "s2", 48, 48)
        ]),
    );
    let mut plan = plan_at(&scene, 44);
    plan.validate().unwrap();
    assert_eq!(plan.fx.len(), 1);
    let id = plan.fx[0].clip_id.clone();
    let other = &mut plan.transition.as_mut().unwrap().other;
    assert_eq!(other.fx.len(), 1);
    other.fx[0].clip_id = id;
    refused_in_both_doors(&plan, "/transition/other/fx/0/clipId");
}

#[test]
fn lights_of_either_deck_count_before_any_item_is_read() {
    for deck in ["", "/transition/other"] {
        let mut value = plan().to_json();
        for at in ["", "/transition/other"] {
            for key in ["nodes", "layers", "splats", "fx", "resources", "lights"] {
                value.pointer_mut(at).unwrap()[key] = json!([]);
            }
        }
        value.pointer_mut(deck).unwrap()["lights"] =
            Value::Array(vec![Value::Null; MAX_TRANSITION_PLAN_ITEMS + 1]);
        assert_eq!(
            pointer(ScenePlan::from_json(&value).unwrap_err()),
            "/transition"
        );
    }
}

#[test]
fn a_typed_neighbour_version_error_names_the_neighbour() {
    let mut plan = plan();
    plan.transition.as_mut().unwrap().other.version = 99;
    refused_in_both_doors(&plan, "/transition/other/planVersion");
}

#[test]
fn a_splat_transform_error_with_colliding_node_ids_names_the_bad_deck() {
    let mut raw = with_transitions(Edit::spec().bundle(), vec![tr(json!({}))]);
    raw["assets"].as_array_mut().unwrap().push(json!({
        "assetId": "cloud", "revisionId": "r1", "contentHash": "cloud-hash",
        "relativePath": "splats/cloud.spz", "format": "spz",
        "widthPx": 1, "heightPx": 1, "alphaMode": "opaque", "colorSpace": "srgb"
    }));
    for scene in raw["sceneDefinitions"].as_array_mut().unwrap() {
        scene["nodes"].as_array_mut().unwrap().push(json!({
            "id": "cloud", "parentId": "root", "type": "splat",
            "assetRef": {"assetId": "cloud", "revisionId": "r1", "contentHash": "cloud-hash",
                         "relativePath": "splats/cloud.spz"},
            "localPosition": {"x": 0, "y": 0, "z": -1},
            "localRotation": {"x": 0, "y": 0, "z": 0, "w": 1},
            "localScale": {"x": 1, "y": 1, "z": 1}
        }));
    }
    let mut plan = plan_at(&scene_of(raw), 44);
    plan.validate().unwrap();
    let other = &mut plan.transition.as_mut().unwrap().other;
    let node = other.nodes.iter_mut().find(|n| n.id == "cloud").unwrap();
    node.world_matrix.0[0] = -1.0;
    refused_in_both_doors(&plan, "/transition/other/nodes");
    let mut primary = plan.clone();
    primary
        .transition
        .as_mut()
        .unwrap()
        .other
        .nodes
        .iter_mut()
        .find(|n| n.id == "cloud")
        .unwrap()
        .world_matrix
        .0[0] = 1.0;
    primary
        .nodes
        .iter_mut()
        .find(|n| n.id == "cloud")
        .unwrap()
        .world_matrix
        .0[0] = -1.0;
    // The existing single-deck error contract is preserved at the top level.
    assert!(matches!(
        primary.validate(),
        Err(PlanError::SplatTransform { .. })
    ));
}
