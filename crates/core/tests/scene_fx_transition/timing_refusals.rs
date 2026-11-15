//! P4b S2-C0b (BL082.4.4, BL082.4.20): every wrong `alignment` or `timing` a plan can carry is
//! refused with the pointer of the field that is wrong, by the three doors: the previous wire,
//! wrong types of each number, extra fields, and typed plans that are well formed but incoherent.

use core::prelude::v1::test;
use serde_json::{Value, json};
use tessel_editor_core::directed_scene::Fps;
use tessel_editor_core::directed_scene::fx::Alignment;
use tessel_editor_core::scene_plan::ScenePlan;
use tessel_editor_core::time::Ratio;

use crate::timing::{accepted_by_all, plan, pointer, refused_by_all};

#[test]
fn the_previous_wire_of_a_transition_is_refused_where_the_missing_field_is() {
    let value = plan(44).to_json();
    for (path, key, at) in [
        ("/transition", "alignment", "/transition/alignment"),
        ("/transition", "timing", "/transition/timing"),
        (
            "/transition/timing",
            "editDurationFrames",
            "/transition/timing/editDurationFrames",
        ),
        (
            "/transition/timing/from",
            "shotStartFrame",
            "/transition/timing/from/shotStartFrame",
        ),
        (
            "/transition/timing/to",
            "sampledCameraClipFrame",
            "/transition/timing/to/sampledCameraClipFrame",
        ),
    ] {
        let mut missing = value.clone();
        missing
            .pointer_mut(path)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove(key);
        assert_eq!(pointer(ScenePlan::from_json(&missing).unwrap_err()), at);
        assert_eq!(
            pointer(ScenePlan::from_json_str(&missing.to_string()).unwrap_err()),
            at
        );
    }
    // Both new fields gone is the plan the previous version wrote: refused at the first of them.
    let mut old = value.clone();
    let object = old["transition"].as_object_mut().unwrap();
    object.remove("alignment");
    object.remove("timing");
    assert_eq!(
        pointer(ScenePlan::from_json_str(&old.to_string()).unwrap_err()),
        "/transition/alignment"
    );
}

fn timing_paths() -> Vec<String> {
    const KEYS: [&str; 8] = [
        "shotStartFrame",
        "shotEndFrameExclusive",
        "instanceStartFrame",
        "instanceDurationFrames",
        "cameraInFrame",
        "cameraClipDurationFrames",
        "sampledInstanceFrame",
        "sampledCameraClipFrame",
    ];
    let mut paths = vec!["/transition/timing/editDurationFrames".to_string()];
    for side in ["from", "to"] {
        for key in KEYS {
            paths.push(format!("/transition/timing/{side}/{key}"));
        }
    }
    paths
}

#[test]
fn every_timing_number_refuses_each_wrong_type_with_its_own_pointer() {
    let value = plan(44).to_json();
    for path in timing_paths() {
        for (what, bad) in [
            ("float", json!(2.5)),
            ("whole float", json!(2.0)),
            ("negative", json!(-1)),
            ("bool", json!(true)),
            ("string", json!("2")),
            ("null", Value::Null),
            ("array", json!([2])),
            ("object", json!({})),
            ("beyond u64", json!(1.0e30)),
        ] {
            let mut hostile = value.clone();
            *hostile.pointer_mut(&path).unwrap() = bad;
            assert_eq!(
                pointer(ScenePlan::from_json(&hostile).unwrap_err()),
                path,
                "{what} at {path}"
            );
            assert_eq!(
                pointer(ScenePlan::from_json_str(&hostile.to_string()).unwrap_err()),
                path,
                "{what} at {path}"
            );
        }
    }
}

#[test]
fn extra_fields_and_wrong_alignment_values_are_refused_with_their_own_pointer() {
    let value = plan(44).to_json();
    for parent in [
        "/transition/timing",
        "/transition/timing/from",
        "/transition/timing/to",
    ] {
        let mut hostile = value.clone();
        hostile.pointer_mut(parent).unwrap()["extra"] = json!(1);
        let at = format!("{parent}/extra");
        assert_eq!(pointer(ScenePlan::from_json(&hostile).unwrap_err()), at);
        assert_eq!(
            pointer(ScenePlan::from_json_str(&hostile.to_string()).unwrap_err()),
            at
        );
    }
    for (what, bad) in [
        ("unknown", json!("middle")),
        ("capitalized", json!("Center")),
        ("number", json!(1)),
        ("bool", json!(false)),
        ("null", Value::Null),
        ("empty", json!("")),
        ("object", json!({"center": true})),
    ] {
        let mut hostile = value.clone();
        hostile["transition"]["alignment"] = bad;
        assert_eq!(
            pointer(ScenePlan::from_json(&hostile).unwrap_err()),
            "/transition/alignment",
            "{what}"
        );
        assert_eq!(
            pointer(ScenePlan::from_json_str(&hostile.to_string()).unwrap_err()),
            "/transition/alignment",
            "{what}"
        );
    }
    for name in ["start", "center", "end"] {
        let mut named = value.clone();
        named["transition"]["alignment"] = json!(name);
        // A valid name that is not the window's alignment is refused later, as a window.
        let result = ScenePlan::from_json(&named);
        if name == "center" {
            assert!(result.is_ok());
        } else {
            assert_eq!(pointer(result.unwrap_err()), "/transition/startFrame");
        }
    }
}

// ---- coherence of a typed plan, on the three doors ------------------------------------------

type Case = (i64, &'static str, fn(&mut ScenePlan), &'static str);

fn case(frame: i64, what: &'static str, edit: fn(&mut ScenePlan), at: &'static str) -> Case {
    (frame, what, edit, at)
}

fn t(plan: &mut ScenePlan) -> &mut tessel_editor_core::scene_plan::PlanTransition {
    plan.transition.as_deref_mut().unwrap()
}

#[test]
fn the_primary_frame_must_precede_its_shot_exclusive_end() {
    // At frame 50, the incoming shot starts at 48. End 51 includes this frame; end 50 does not.
    let mut inside = plan(50);
    t(&mut inside).timing.to.shot_end_frame_exclusive = 51;
    accepted_by_all(
        &inside,
        "the frame before the exclusive end belongs to the primary",
    );
    for (frame, what) in [
        (
            50,
            "the frame exactly at the exclusive end is outside the primary",
        ),
        (51, "a frame past the exclusive end is outside the primary"),
    ] {
        let mut outside = plan(frame);
        t(&mut outside).timing.to.shot_end_frame_exclusive = 50;
        refused_by_all(
            &outside,
            "/transition/timing/to/shotEndFrameExclusive",
            what,
        );
    }
}

fn cases() -> Vec<Case> {
    const FROM_END: &str = "/transition/timing/from/shotEndFrameExclusive";
    const TO_END: &str = "/transition/timing/to/shotEndFrameExclusive";
    const START: &str = "/transition/startFrame";
    vec![
        // Intervals, the cut and the primary.
        case(
            44,
            "empty outgoing shot",
            |p| t(p).timing.from.shot_start_frame = 48,
            FROM_END,
        ),
        case(
            44,
            "shot past the edit",
            |p| t(p).timing.edit_duration_frames = 90,
            TO_END,
        ),
        case(
            44,
            "incoming starts after the cut",
            |p| t(p).timing.to.shot_start_frame = 49,
            FROM_END,
        ),
        case(
            44,
            "outgoing ends before the cut",
            |p| t(p).timing.from.shot_end_frame_exclusive = 47,
            FROM_END,
        ),
        case(
            44,
            "cut earlier than the frame: primary too early",
            |p| {
                t(p).timing.from.shot_end_frame_exclusive = 44;
                t(p).timing.to.shot_start_frame = 44;
            },
            "/transition/primary",
        ),
        case(
            50,
            "cut later than the frame: primary too late",
            |p| {
                t(p).timing.from.shot_end_frame_exclusive = 52;
                t(p).timing.to.shot_start_frame = 52;
            },
            "/transition/primary",
        ),
        case(
            44,
            "frame before the primary shot",
            |p| t(p).timing.from.shot_start_frame = 45,
            "/transition/timing/from/shotStartFrame",
        ),
        // The window.
        case(
            44,
            "alignment start on a centred window",
            |p| t(p).alignment = Alignment::Start,
            START,
        ),
        case(
            44,
            "alignment end on a centred window",
            |p| t(p).alignment = Alignment::End,
            START,
        ),
        case(
            44,
            "window past the edit",
            |p| {
                t(p).timing.to.shot_end_frame_exclusive = 50;
                t(p).timing.edit_duration_frames = 50;
            },
            START,
        ),
        // Extremes.
        case(
            44,
            "cut beyond i64",
            |p| t(p).timing.to.shot_start_frame = u64::MAX,
            "/transition/timing/to/shotStartFrame",
        ),
        case(
            44,
            "edit beyond i64",
            |p| t(p).timing.edit_duration_frames = u64::MAX,
            "/transition/timing/editDurationFrames",
        ),
        case(
            44,
            "sample beyond i64",
            |p| t(p).timing.to.sampled_instance_frame = (i64::MAX as u64) + 1,
            "/transition/timing/to/sampledInstanceFrame",
        ),
        // Identities.
        case(
            44,
            "instance too short for its shot",
            |p| t(p).timing.from.instance_duration_frames = 60,
            "/transition/timing/from/instanceDurationFrames",
        ),
        case(
            44,
            "one instance with two lengths",
            |p| {
                let id = t(p).other.source.scene_instance_id.clone();
                p.source.scene_instance_id = id;
                t(p).timing.from.instance_duration_frames = 150;
            },
            "/transition/timing/to/instanceDurationFrames",
        ),
        case(
            44,
            "one camera clip with two lengths",
            |p| {
                let id = t(p).other.frame.camera_clip_id.clone();
                p.frame.camera_clip_id = id;
            },
            "/transition/timing/to/cameraClipDurationFrames",
        ),
        // Samples, raw frames and held flags.
        case(
            44,
            "sample of the instance is not the rule's",
            |p| t(p).timing.from.sampled_instance_frame = 63,
            "/transition/timing/from/sampledInstanceFrame",
        ),
        case(
            44,
            "sample of the neighbour camera is not the rule's",
            |p| t(p).timing.to.sampled_camera_clip_frame = 1,
            "/transition/timing/to/sampledCameraClipFrame",
        ),
        case(
            50,
            "sample without the clamp",
            |p| t(p).timing.from.sampled_camera_clip_frame = 50,
            "/transition/timing/from/sampledCameraClipFrame",
        ),
        case(
            44,
            "primary raw instance frame",
            |p| p.frame.instance_frame = 63,
            "/frame/instanceFrame",
        ),
        case(
            44,
            "primary raw camera frame",
            |p| p.frame.camera_clip_frame = 43,
            "/frame/cameraClipFrame",
        ),
        case(
            44,
            "neighbour instance frame",
            |p| t(p).other.frame.instance_frame = 1,
            "/transition/other/frame/instanceFrame",
        ),
        case(
            44,
            "neighbour camera frame",
            |p| t(p).other.frame.camera_clip_frame = 1,
            "/transition/other/frame/cameraClipFrame",
        ),
        case(
            50,
            "neighbour camera frame without the clamp",
            |p| t(p).other.frame.camera_clip_frame = 50,
            "/transition/other/frame/cameraClipFrame",
        ),
        case(
            44,
            "held instance cleared",
            |p| t(p).held_instance = false,
            "/transition/heldInstance",
        ),
        case(
            44,
            "held camera cleared",
            |p| t(p).held_camera = false,
            "/transition/heldCamera",
        ),
        case(
            50,
            "held instance invented",
            |p| t(p).held_instance = true,
            "/transition/heldInstance",
        ),
        case(
            50,
            "held camera cleared past the clip",
            |p| t(p).held_camera = false,
            "/transition/heldCamera",
        ),
        // Times.
        case(
            44,
            "wrong frame rate on both decks",
            |p| {
                let fps = Fps {
                    numerator: 25,
                    denominator: 1,
                };
                p.output.fps = fps;
                t(p).other.output.fps = fps;
            },
            "/frame/editTime",
        ),
        case(
            44,
            "edit time of both decks",
            |p| {
                let wrong = Ratio::new(5, 2).unwrap();
                p.frame.edit_time = wrong;
                t(p).other.frame.edit_time = wrong;
            },
            "/frame/editTime",
        ),
        case(
            44,
            "instance time",
            |p| p.frame.instance_time = Ratio::new(1, 1).unwrap(),
            "/frame/instanceTime",
        ),
        case(
            44,
            "neighbour camera time",
            |p| t(p).other.frame.camera_clip_time = Ratio::new(1, 1).unwrap(),
            "/transition/other/frame/cameraClipTime",
        ),
    ]
}

#[test]
fn every_incoherent_typed_plan_is_refused_by_the_three_doors_with_its_pointer() {
    for (frame, what, edit, at) in cases() {
        let mut hostile = plan(frame);
        accepted_by_all(&hostile, what);
        edit(&mut hostile);
        refused_by_all(&hostile, at, what);
    }
}
