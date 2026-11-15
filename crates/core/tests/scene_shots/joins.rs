//! T05-T08: continuous joins, the temporal gate of a join, `editLocked` and the instances that grow.

use core::prelude::v1::test;

use serde_json::json;
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::scene_edit::shots::{CONTINUOUS_JOIN, LOCKED};

use super::support::*;
// ---- T05: continuous joins -------------------------------------------------------------------

#[test]
fn t05_growing_the_start_of_a_take_slides_what_carries_on() {
    let out = done(
        &chain(),
        json!({"set_shot_duration": {"shot": "a", "frames": 40}}),
    );
    assert_eq!(
        layout(&out.candidate),
        ["a [0,40) i0 c0", "b [40,70) i40 c40", "c [70,100) i0 c0"]
    );
    assert_eq!(instance_frames(&out.candidate, "ia"), 70);
    assert_eq!(
        notes(&out),
        [json!({"code": "INSTANCE_GREW", "instance": "ia", "from": 60, "to": 70})]
    );
}

#[test]
fn t05_shrinking_the_start_of_a_take_loses_the_tail_of_the_take() {
    let out = done(
        &chain(),
        json!({"set_shot_duration": {"shot": "a", "frames": 20}}),
    );
    assert_eq!(
        layout(&out.candidate),
        ["a [0,20) i0 c0", "b [20,50) i20 c20", "c [50,80) i0 c0"]
    );
    assert_eq!(instance_frames(&out.candidate, "ia"), 60);
    // The end of the take is where the splice sits: growing `b` pulls only `c`.
    let end = done(
        &chain(),
        json!({"set_shot_duration": {"shot": "b", "frames": 40}}),
    );
    assert_eq!(
        layout(&end.candidate),
        ["a [0,30) i0 c0", "b [30,70) i30 c30", "c [70,100) i0 c0"]
    );
}

#[test]
fn t05_a_roll_on_a_continuous_join_keeps_both_sides_continuous() {
    let scene = chain();
    let later = done(
        &scene,
        json!({"move_cut": {"from": "a", "to": "b", "frame": 36}}),
    );
    assert_eq!(
        layout(&later.candidate),
        ["a [0,36) i0 c0", "b [36,60) i36 c36", "c [60,90) i0 c0"]
    );
    for content in ["anchor", "auto"] {
        let out = done(
            &scene,
            json!({"move_cut": {"from": "a", "to": "b", "frame": 24, "content": content}}),
        );
        assert_eq!(
            layout(&out.candidate),
            ["a [0,24) i0 c0", "b [24,60) i24 c24", "c [60,90) i0 c0"],
            "{content}"
        );
        assert!(notes(&out).is_empty(), "auto never falls to follow here");
    }
}

#[test]
fn t05_follow_into_a_continuous_join_and_a_head_cut_of_a_continuation_are_refused() {
    let scene = chain();
    for value in [
        json!({"move_cut": {"from": "a", "to": "b", "frame": 24, "content": "follow"}}),
        json!({"trim_shot": {"shot": "b", "edge": "start", "frame": 36}}),
        json!({"trim_shot": {"shot": "b", "edge": "start", "frame": 24}}),
    ] {
        let error = refused(&scene, value.clone());
        assert_eq!(error.code, CONTINUOUS_JOIN, "{value}: {error}");
        assert_eq!(error.index, Some(0));
    }
}

// ---- T06: the temporal gate of the join ------------------------------------------------------

/// A camera that dollies in at 0.2 scene units a frame for 60 frames, then holds: a join that
/// falls inside the dolly is smooth, one that falls on its end jumps in velocity (0.2 > 0.08).
fn moving_chain() -> DirectedScene {
    let mut document = bundle(
        90,
        &[
            sh("a", "ia", 0, 30, 0, 0),
            sh("b", "ia", 30, 90, 30, 30).carries_on(),
        ],
        &[("ia", 120, false)],
        json!({}),
    );
    document["cameraClips"][0]["segments"] = json!([
        {"recipe": "dollyIn", "durationFrames": 60, "from": "initial",
         "distanceSceneUnits": 12, "easing": "linear"},
        {"recipe": "hold", "durationFrames": 160, "from": "previousEnd"},
    ]);
    scene_of(document)
}

#[test]
fn t06_a_cut_that_breaks_the_camera_pose_at_a_continuous_join_is_rejected_by_the_gate() {
    let scene = moving_chain();
    // Positive control: the cut still falls inside the dolly.
    let inside = done(
        &scene,
        json!({"move_cut": {"from": "a", "to": "b", "frame": 45}}),
    );
    assert_eq!(
        layout(&inside.candidate),
        ["a [0,45) i0 c0", "b [45,90) i45 c45"]
    );
    // The dolly's last frame is 59, and the camera stops there: the structural rules are all met
    // and the temporal gate says no, with the importer's own code and no repair.
    let roll = refused(
        &scene,
        json!({"move_cut": {"from": "a", "to": "b", "frame": 59}}),
    );
    finding(&roll, "CONTINUOUS_VELOCITY_JUMP");
    let ripple = refused(
        &scene,
        json!({"set_shot_duration": {"shot": "a", "frames": 59}}),
    );
    finding(&ripple, "CONTINUOUS_VELOCITY_JUMP");
    assert_eq!(roll.index, Some(0));
}

// ---- T07: editLocked -------------------------------------------------------------------------

/// `a` and `c` are on the locked `il`, in step (`instanceStartFrame - startFrame` is 0 for both).
fn locked(c_instance_start: i64, c_camera_in: i64) -> DirectedScene {
    build(
        90,
        &[
            sh("a", "il", 0, 30, 0, 0),
            sh("b", "ib", 30, 60, 0, 0),
            sh("c", "il", 60, 90, c_instance_start, c_camera_in),
        ],
        &[("il", 120, true), ("ib", 60, false)],
    )
}

#[test]
fn t07_a_ripple_that_moves_one_of_two_locked_shots_out_of_step_is_refused() {
    let error = refused(
        &locked(60, 20),
        json!({"set_shot_duration": {"shot": "b", "frames": 40}}),
    );
    assert_eq!(error.code, LOCKED);
    assert_eq!(detail(&error, "instance"), json!("il"));
    assert_eq!(error.index, Some(0));
}

#[test]
fn t07_shots_that_were_not_in_step_or_a_single_locked_shot_have_nothing_to_keep() {
    let grow = json!({"set_shot_duration": {"shot": "b", "frames": 40}});
    // `c` was 10 frames off a's step.
    let out = done(&locked(50, 20), grow.clone());
    assert_eq!(layout(&out.candidate)[2], "c [70,100) i50 c20");
    // The locked instance has one shot only.
    let single = build(
        90,
        &[
            sh("a", "il", 0, 30, 0, 0),
            sh("b", "ib", 30, 60, 0, 0),
            sh("c", "ic", 60, 90, 0, 0),
        ],
        &[("il", 60, true), ("ib", 60, false), ("ic", 60, false)],
    );
    done(&single, grow);
}

#[test]
fn t07_anchor_keeps_the_step_follow_breaks_it_and_auto_anchors_when_it_can() {
    let scene = locked(60, 20);
    let cut = |content: &str| json!({"move_cut": {"from": "b", "to": "c", "frame": 50, "content": content}});
    let anchor = done(&scene, cut("anchor"));
    assert_eq!(layout(&anchor.candidate)[2], "c [50,90) i50 c10");
    assert_eq!(refused(&scene, cut("follow")).code, LOCKED);
    let auto = done(&scene, cut("auto"));
    assert_eq!(auto.candidate.document(), anchor.candidate.document());
    assert!(notes(&auto).is_empty());
}

#[test]
fn t07_auto_that_cannot_anchor_enough_cannot_follow_either_when_the_step_would_break() {
    // 4 camera frames of head: auto would anchor 4 and follow 6, and follow breaks the step.
    let error = refused(
        &locked(60, 4),
        json!({"move_cut": {"from": "b", "to": "c", "frame": 50}}),
    );
    assert_eq!(error.code, LOCKED);
}

#[test]
fn t07_auto_follows_to_keep_the_step_when_anchoring_is_what_would_break_it() {
    // `a` and `b` share the locked `il` in step: anchoring `b`'s new head frames breaks it.
    let scene = build(
        90,
        &[
            sh("a", "il", 0, 30, 0, 0),
            sh("b", "il", 30, 60, 30, 30),
            sh("c", "ic", 60, 90, 0, 0),
        ],
        &[("il", 120, true), ("ic", 60, false)],
    );
    let trim = |content: Option<&str>| {
        let mut body = json!({"shot": "b", "edge": "start", "frame": 20});
        if let Some(content) = content {
            body["content"] = json!(content);
        }
        json!({"trim_shot": body})
    };
    assert_eq!(refused(&scene, trim(Some("anchor"))).code, LOCKED);
    let auto = done(&scene, trim(None));
    assert_eq!(
        layout(&auto.candidate),
        ["a [0,30) i0 c0", "b [30,70) i30 c30", "c [70,100) i0 c0"]
    );
    assert_eq!(
        notes(&auto),
        [json!({"code": "AUTO_FELL_TO_FOLLOW", "shot": "b", "anchored": 0, "followed": 10})]
    );
}

// ---- T08: instances grow and never shrink ----------------------------------------------------

#[test]
fn t08_an_instance_grows_to_fit_with_a_note_and_never_shrinks() {
    let scene = three(20);
    let grow = done(
        &scene,
        json!({"set_shot_duration": {"shot": "b", "frames": 50}}),
    );
    // 20 head frames + 50 = 70 frames of scene.
    assert_eq!(instance_frames(&grow.candidate, "ib"), 70);
    assert_eq!(
        notes(&grow),
        [json!({"code": "INSTANCE_GREW", "instance": "ib", "from": 60, "to": 70})]
    );
    // The new frames are in the document the importer then accepted (the gate ran).
    let shrink = done(
        &grow.candidate,
        json!({"set_shot_duration": {"shot": "b", "frames": 5}}),
    );
    assert_eq!(
        instance_frames(&shrink.candidate, "ib"),
        70,
        "never shrinks"
    );
    assert!(notes(&shrink).is_empty());
}

#[test]
fn t08_two_shots_on_one_instance_grow_it_to_the_more_demanding_one() {
    let scene = build(
        90,
        &[
            sh("a", "ia", 0, 30, 0, 0),
            sh("b", "ia", 30, 60, 0, 0),
            sh("c", "ic", 60, 90, 0, 0),
        ],
        &[("ia", 40, false), ("ic", 60, false)],
    );
    let out = done(
        &scene,
        json!({"set_shot_duration": {"shot": "b", "frames": 70}}),
    );
    assert_eq!(instance_frames(&out.candidate, "ia"), 70);
    assert_eq!(
        notes(&out),
        [json!({"code": "INSTANCE_GREW", "instance": "ia", "from": 40, "to": 70})]
    );
}

#[test]
fn t08_follow_shows_more_of_the_tail_and_grows_the_instance() {
    let scene = build(
        90,
        &[
            sh("a", "ia", 0, 30, 0, 0),
            sh("b", "ib", 30, 60, 20, 20),
            sh("c", "ic", 60, 90, 0, 0),
        ],
        &[("ia", 60, false), ("ib", 50, false), ("ic", 60, false)],
    );
    let out = done(
        &scene,
        json!({"trim_shot": {"shot": "b", "edge": "start", "frame": 20, "content": "follow"}}),
    );
    assert_eq!(layout(&out.candidate)[1], "b [30,70) i20 c20");
    assert_eq!(instance_frames(&out.candidate, "ib"), 60);
    assert_eq!(
        notes(&out),
        [json!({"code": "INSTANCE_GREW", "instance": "ib", "from": 50, "to": 60})]
    );
}
