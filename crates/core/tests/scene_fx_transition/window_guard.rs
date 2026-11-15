//! S3a (plan §9.6, decision D16/Q1): the one predicate that says whether an overlay may be edited
//! at a frame. It is the window `resolve_frame` draws, asked once, with no state.

use serde_json::json;
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::directed_scene::fx::{
    Locale, OverlayTarget, TRANSITION_WINDOW, WindowRefusal,
};

use crate::support::*;

fn ask(scene: &DirectedScene, frame: i64, target: OverlayTarget) -> Option<WindowRefusal> {
    let bundle = scene.bundle();
    bundle
        .fx
        .overlay_refusal(&bundle.shots, bundle.edit_duration_frames, frame, target)
}

/// The frames the picture is a blend at (the oracle for the predicate).
fn blended(scene: &DirectedScene, frame: i64) -> bool {
    let bundle = scene.bundle();
    bundle
        .fx
        .resolve_frame(&bundle.shots, bundle.edit_duration_frames, frame)
        .transition
        .is_some()
}

const TARGETS: [OverlayTarget; 2] = [OverlayTarget::Gizmo, OverlayTarget::Handle];

#[test]
fn the_window_is_half_open_and_both_targets_are_refused_alike() {
    // 12 frames centred on the cut at 48: `[42, 54)`.
    let scene = spec_scene(vec![tr(json!({}))]);
    for target in TARGETS {
        assert_eq!(ask(&scene, 41, target), None);
        let first = ask(&scene, 42, target).expect("the first frame of the window");
        assert_eq!(
            first,
            WindowRefusal::TransitionWindow {
                transition_id: "tr".into(),
                start_frame: 42,
                end_frame: 54,
                edit_frame: 42,
            }
        );
        assert!(ask(&scene, 53, target).is_some(), "the last frame");
        assert_eq!(ask(&scene, 54, target), None, "the end is exclusive");
        assert_eq!(ask(&scene, 0, target), None);
        assert_eq!(ask(&scene, 143, target), None);
    }
}

#[test]
fn it_agrees_with_the_picture_on_every_frame_of_the_edit() {
    let scene = spec_scene(vec![
        tr(json!({"id": "a", "durationFrames": 12})),
        tr(
            json!({"id": "b", "fromShotId": "s2", "toShotId": "s3", "alignment": "start",
                  "durationFrames": 20}),
        ),
    ]);
    let mut windows = 0;
    for frame in 0..144 {
        let refused = ask(&scene, frame, OverlayTarget::Gizmo);
        assert_eq!(refused.is_some(), blended(&scene, frame), "frame {frame}");
        windows += i32::from(refused.is_some());
    }
    assert_eq!(windows, 12 + 20);
    // The refusal names the transition whose window it is.
    assert_eq!(
        ask(&scene, 50, OverlayTarget::Gizmo)
            .unwrap()
            .transition_id(),
        "a"
    );
    assert_eq!(
        ask(&scene, 100, OverlayTarget::Handle)
            .unwrap()
            .transition_id(),
        "b"
    );
}

#[test]
fn a_hidden_transition_has_no_window_and_a_scene_without_one_never_refuses() {
    let hidden = spec_scene(vec![tr(json!({"hidden": true}))]);
    let none = spec_scene(vec![]);
    for frame in 0..144 {
        assert_eq!(ask(&hidden, frame, OverlayTarget::Gizmo), None, "{frame}");
        assert_eq!(ask(&none, frame, OverlayTarget::Handle), None, "{frame}");
    }
    // Frames outside the edit are no window either.
    let shown = spec_scene(vec![tr(json!({}))]);
    assert_eq!(ask(&shown, -1, OverlayTarget::Gizmo), None);
    assert_eq!(ask(&shown, 144, OverlayTarget::Gizmo), None);
    assert_eq!(ask(&shown, i64::MAX, OverlayTarget::Gizmo), None);
    assert_eq!(ask(&shown, i64::MIN, OverlayTarget::Gizmo), None);
}

#[test]
fn the_answer_does_not_depend_on_the_order_of_the_questions() {
    let scene = spec_scene(vec![tr(json!({}))]);
    let frames: Vec<i64> = (30..70).collect();
    let forward: Vec<_> = frames
        .iter()
        .map(|&frame| ask(&scene, frame, OverlayTarget::Gizmo))
        .collect();
    let mut backward: Vec<_> = frames
        .iter()
        .rev()
        .map(|&frame| ask(&scene, frame, OverlayTarget::Gizmo))
        .collect();
    backward.reverse();
    assert_eq!(forward, backward);
    // Asking twice, or with the other target in between, changes nothing.
    assert_eq!(
        ask(&scene, 48, OverlayTarget::Gizmo),
        ask(&scene, 48, OverlayTarget::Gizmo)
    );
    let _ = ask(&scene, 48, OverlayTarget::Handle);
    assert_eq!(ask(&scene, 47, OverlayTarget::Gizmo), forward[17]);
}

#[test]
fn the_refusal_is_a_localized_problem_of_the_same_table() {
    let scene = spec_scene(vec![tr(json!({}))]);
    let refusal = ask(&scene, 50, OverlayTarget::Gizmo).unwrap();
    assert_eq!(refusal.code(), TRANSITION_WINDOW);
    assert_eq!(refusal.code(), "transition_window");
    let english = refusal.problem(Locale::En);
    assert_eq!(
        english.message,
        "Frame 50 is inside the window of transition tr (frames 42 to 54)"
    );
    assert_eq!(
        english.hint.as_deref(),
        Some("Move the playhead outside that window to edit this overlay.")
    );
    assert_eq!(english.transition_id.as_deref(), Some("tr"));
    for locale in Locale::ALL {
        let problem = refusal.problem(locale);
        assert_eq!(problem.code, "transition_window");
        assert!(!problem.message.contains('{'));
        assert!(problem.message.contains("50") && problem.message.contains("42"));
        assert!(problem.details.get("untranslated").is_none());
        assert_eq!(problem.details["args"]["end"], 54);
    }
    assert_ne!(
        refusal.problem(Locale::PtBr).message,
        refusal.problem(Locale::En).message
    );
}
