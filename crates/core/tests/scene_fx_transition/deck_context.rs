//! BL082.4.19 (S2-C0a): an error of the NEIGHBOUR deck says it is the neighbour's. Both builders
//! (`ScenePlan::build_with`, `PlanTimeline::build_with`) qualify every step of the deck that is
//! not under the needle with the pointer a plan read back from JSON uses
//! (`/transition/other…`, `PlanError::Invalid`), and leave the deck of the primary as it was.

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::scene_plan::{PlanError, PlanOptions, PlanTimeline, ScenePlan};

use crate::support::*;

/// The spec edit with a transition on `s1 -> s2` (window `[42, 54)`), `s1` in `scene-a` and
/// `s2` in `scene-b`; `change` edits the bundle first.
fn scene_with(change: impl FnOnce(&mut Value)) -> DirectedScene {
    let mut bundle = with_transitions(Edit::spec().bundle(), vec![tr(json!({}))]);
    change(&mut bundle);
    scene_of(bundle)
}

fn definition<'a>(bundle: &'a mut Value, id: &str) -> &'a mut Value {
    bundle["sceneDefinitions"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|definition| definition["id"] == id)
        .unwrap()
}

/// A splat, with the id both scenes use, whose scale is not a similarity only in `scene-a`.
fn stretched_in_scene_a(bundle: &mut Value) {
    add_splats(bundle, &["scene-a", "scene-b"]);
    let node = definition(bundle, "scene-a")["nodes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|node| node["id"] == "cloud-node")
        .unwrap();
    node["localScale"] = json!({"x": 2, "y": 1, "z": 1});
}

fn both_builders(
    scene: &DirectedScene,
    frame: i64,
    options: PlanOptions,
) -> Result<ScenePlan, PlanError> {
    let fresh = ScenePlan::build_with(scene, frame, options);
    let timeline = PlanTimeline::new(scene);
    // Twice: the second answer comes from the shared layouts and must not be wrapped again.
    assert_eq!(timeline.build_with(frame, options), fresh, "frame {frame}");
    assert_eq!(
        timeline.build_with(frame, options),
        fresh,
        "frame {frame}, again"
    );
    fresh
}

fn pointer_of(error: Result<ScenePlan, PlanError>) -> (String, String) {
    match error {
        Err(PlanError::Invalid { pointer, message }) => (pointer, message),
        other => panic!("expected an invalid plan, got {other:?}"),
    }
}

#[test]
fn a_splat_that_only_the_neighbour_stretches_is_refused_under_the_other_deck() {
    let scene = scene_with(stretched_in_scene_a);
    // Frame 50: `s2` (scene-b, fine) is under the needle and `s1` is the neighbour.
    let (pointer, message) = pointer_of(both_builders(&scene, 50, PlanOptions::default()));
    assert_eq!(pointer, "/transition/other/nodes");
    assert!(message.contains("cloud-node"), "{message}");
    // The prefix is applied once.
    assert!(!pointer.contains("other/transition/other"), "{pointer}");
}

#[test]
fn the_deck_under_the_needle_keeps_its_own_error_even_with_the_same_node_id() {
    let scene = scene_with(stretched_in_scene_a);
    // Frames 44 and 10: `s1` (scene-a) is the primary, in the window and outside it.
    for frame in [44, 10] {
        assert!(
            matches!(
                both_builders(&scene, frame, PlanOptions::default()),
                Err(PlanError::SplatTransform { ref node_id, .. }) if node_id == "cloud-node"
            ),
            "frame {frame}"
        );
    }
    // Past the window `s2` is alone and fine.
    assert!(both_builders(&scene, 70, PlanOptions::default()).is_ok());
}

#[test]
fn a_domain_the_plan_does_not_evaluate_in_the_neighbour_is_qualified_too() {
    let scene = scene_with(|bundle| {
        definition(bundle, "scene-a")["weather"] = json!("rain");
    });
    let (pointer, message) = pointer_of(both_builders(&scene, 50, PlanOptions::default()));
    assert_eq!(pointer, "/transition/other");
    assert!(message.contains("weather"), "{message}");
    // The primary's own refusal keeps its type.
    assert!(matches!(
        both_builders(&scene, 44, PlanOptions::default()),
        Err(PlanError::Unsupported { .. })
    ));
    // An explicit partial plan still accepts what the neighbour does not evaluate.
    let partial = PlanOptions {
        allow_partial: true,
    };
    assert!(both_builders(&scene, 50, partial).is_ok());
}
