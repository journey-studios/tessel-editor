//! The batch, the parsing of ops, the refusal of an op that would move a layer S1 does not move
//! (T16; the rest of that family is in `layers.rs`), narration (T17) and the planner alone with a
//! reserve (T14b).

use core::prelude::v1::test;

use serde_json::{Value, json};
use tessel_editor_core::scene_edit::shots::{
    CODES, CONTINUOUS_JOIN, Content, EDIT_TOO_LONG, Edge, LOCKED, MAX_SHOT_BATCH_OPS,
    NO_HEAD_FRAMES, Placement, PlanInput, PlanInstance, PlanShot, SHOT_NOT_FOUND, SHOT_TOO_SHORT,
    SHOTS_LAYER_NOT_YET, SHOTS_NOT_ADJACENT, SHOTS_OPTION_NOT_YET, ShotOp, ShotOptions,
    apply_shots, parse_shot_ops, plan_op,
};
use tessel_editor_core::scene_edit::{CANDIDATE_REJECTED, INVALID_OP};

use super::support::*;
// ---- the batch -------------------------------------------------------------------------------

#[test]
fn a_batch_applies_op_after_op_and_is_atomic() {
    let scene = three(20);
    let ops = parse_shot_ops(&json!([
        {"set_shot_duration": {"shot": "b", "frames": 40}},
        {"move_cut": {"from": "b", "to": "c", "frame": 80}},
    ]))
    .unwrap();
    let out = apply_shots(&scene, &ops).unwrap();
    // The second op reads the document the first one left: `b` is `[30,70)`, `c` `[70,100)`.
    assert_eq!(
        layout(&out.candidate),
        ["a [0,30) i0 c0", "b [30,80) i20 c20", "c [80,100) i10 c10"]
    );
    assert!(out.changed);

    let ops = parse_shot_ops(&json!([
        {"set_shot_duration": {"shot": "b", "frames": 40}},
        {"set_shot_duration": {"shot": "b", "frames": 0}},
    ]))
    .unwrap();
    let error = apply_shots(&scene, &ops).unwrap_err();
    assert_eq!(error.code, SHOT_TOO_SHORT);
    assert_eq!(error.index, Some(1));
}

#[test]
fn a_batch_of_no_ops_is_a_no_op_and_an_empty_batch_too() {
    let scene = three(20);
    let ops = parse_shot_ops(&json!([
        {"set_shot_duration": {"shot": "b", "frames": 30}},
        {"move_cut": {"from": "a", "to": "b", "frame": 30}},
    ]))
    .unwrap();
    let out = apply_shots(&scene, &ops).unwrap();
    assert!(!out.changed);
    assert_eq!(out.candidate.document(), scene.document());
    assert!(!apply_shots(&scene, &[]).unwrap().changed);
}

#[test]
fn malformed_ops_are_invalid_op_with_their_position() {
    let bad = [
        json!({"nope": {}}),
        json!({"trim_shot": {"shot": "a", "edge": "middle", "frame": 1}}),
        json!({"trim_shot": {"shot": "a", "edge": "start"}}),
        json!({"trim_shot": {"shot": "a", "edge": "start", "frame": 1.5}}),
        json!({"trim_shot": {"shot": "a", "edge": "start", "frame": 1, "speed": 2}}),
        // `mode` belongs to the ops of one border only.
        json!({"move_cut": {"from": "a", "to": "b", "frame": 1, "mode": "roll"}}),
        json!({"set_edit_duration": {"frames": 5, "mode": "ripple"}}),
        json!({"set_shot_duration": {"shot": "a", "frames": 5, "content": "keep"}}),
        json!({"set_shot_duration": {"shot": "a", "frames": 5, "keep_absolute": "yes"}}),
        json!({"set_edit_duration": {"frames": 1_i64 << 53}}),
        json!({"set_edit_duration": {"frames": 5}, "extra": 1}),
        json!("set_edit_duration"),
    ];
    for value in bad {
        let error = parse_shot_ops(&json!([{"set_edit_duration": {"frames": 5}}, value.clone()]))
            .unwrap_err();
        assert_eq!(error.code, INVALID_OP, "{value}");
        assert_eq!(error.index, Some(1), "{value}");
    }
    assert_eq!(parse_shot_ops(&json!({})).unwrap_err().code, INVALID_OP);
    let many: Vec<Value> = (0..=MAX_SHOT_BATCH_OPS)
        .map(|_| json!({"set_edit_duration": {"frames": 5}}))
        .collect();
    assert_eq!(
        parse_shot_ops(&Value::Array(many)).unwrap_err().code,
        INVALID_OP
    );
}

// ---- T16, T17: layers and narration ----------------------------------------------------------

#[test]
fn t16_the_layer_code_exists_and_every_code_is_listed() {
    assert_eq!(SHOTS_LAYER_NOT_YET, "SHOTS_LAYER_NOT_YET");
    for code in [
        SHOT_TOO_SHORT,
        SHOT_NOT_FOUND,
        SHOTS_NOT_ADJACENT,
        EDIT_TOO_LONG,
        LOCKED,
        CONTINUOUS_JOIN,
        NO_HEAD_FRAMES,
        SHOTS_LAYER_NOT_YET,
        SHOTS_OPTION_NOT_YET,
        CANDIDATE_REJECTED,
    ] {
        assert!(CODES.contains(&code), "{code}");
    }
    let mut sorted = CODES.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), CODES.len(), "a code is listed twice");
}

#[test]
fn t16_an_op_that_would_move_an_output_clip_is_refused_and_one_that_leaves_it_alone_is_not() {
    let fx = json!({"fx": {"tracks": [{"id": "o", "clips": [
        {"id": "o1", "startFrame": 40, "durationFrames": 40, "keys": {}},
    ]}], "transitions": []}});
    let scene = scene_of(bundle(
        90,
        &[
            sh("a", "ia", 0, 30, 0, 0),
            sh("b", "ib", 30, 60, 20, 20),
            sh("c", "ic", 60, 90, 0, 0),
        ],
        &[("ia", 60, false), ("ib", 60, false), ("ic", 60, false)],
        fx,
    ));
    // The clip [40,80) crosses the seam at 60 that a longer `b` opens: the reference stretches it,
    // S1 does not, and the op is refused instead of leaving the clip where it was.
    let error = refused(
        &scene,
        json!({"set_shot_duration": {"shot": "b", "frames": 40}}),
    );
    assert_eq!(error.code, SHOTS_LAYER_NOT_YET, "{error}");
    assert_eq!(error.index, Some(0));
    assert_eq!(detail(&error, "layer"), json!("fx_clip_output"));
    assert_eq!(detail(&error, "path"), json!("/fx/tracks/0/clips/0"));
    // `shot` and `edge` are the target of the op; the frame is the end it asked for.
    assert_eq!(detail(&error, "shot"), json!("b"));
    assert_eq!(detail(&error, "edge"), json!("end"));
    assert_eq!(detail(&error, "asked_frame"), json!(70));
    // A shrinking edit that cuts the clip's tail is refused the same way.
    let shrink = refused(&scene, json!({"set_edit_duration": {"frames": 70}}));
    assert_eq!(shrink.code, SHOTS_LAYER_NOT_YET, "{shrink}");
    // `keep_absolute` freezes output clips: the first op passes with `fx` byte for byte, and the
    // second is the gate's legitimate refusal (the clip leaves the edit).
    let kept = done(
        &scene,
        json!({"set_shot_duration": {"shot": "b", "frames": 40, "keep_absolute": true}}),
    );
    assert_eq!(kept.candidate.document()["fx"], scene.document()["fx"]);
    let out = refused(
        &scene,
        json!({"set_edit_duration": {"frames": 70, "keep_absolute": true}}),
    );
    finding(&out, "FX_CLIP_OUT_OF_RANGE");
    // The seam of a longer `c` is at frame 90, after the clip: nothing to move, `fx` untouched.
    let out = done(
        &scene,
        json!({"set_shot_duration": {"shot": "c", "frames": 40}}),
    );
    assert_eq!(out.candidate.document()["fx"], scene.document()["fx"]);
}
#[test]
fn t17_narration_is_refused_as_an_option_and_never_moves_with_the_shots() {
    let extra = json!({
        "audio": {"editStartSeconds": 0, "editEndSecondsExclusive": 3.75},
        "narration": {"tracks": [{"id": "n", "clips": [
            {"id": "l1", "startSeconds": 1.5, "durationSeconds": 2.0},
        ]}]},
        "outline": {"groups": [], "items": []},
    });
    let scene = scene_of(bundle(
        90,
        &[
            sh("a", "ia", 0, 30, 0, 0),
            sh("b", "ib", 30, 60, 0, 0),
            sh("c", "ic", 60, 90, 0, 0),
        ],
        &[("ia", 60, false), ("ib", 60, false), ("ic", 60, false)],
        extra,
    ));
    let error = refused(
        &scene,
        json!({"set_shot_duration": {"shot": "b", "frames": 40, "with_narration": true}}),
    );
    assert_eq!(error.code, SHOTS_OPTION_NOT_YET);
    assert_eq!(detail(&error, "option"), json!("with_narration"));
    for value in [
        json!({"set_shot_duration": {"shot": "a", "frames": 50}}),
        json!({"move_cut": {"from": "a", "to": "b", "frame": 10}}),
        json!({"trim_shot": {"shot": "b", "edge": "start", "frame": 40}}),
        json!({"set_edit_duration": {"frames": 120}}),
    ] {
        let out = done(&scene, value.clone());
        for field in ["narration", "audio", "outline"] {
            assert_eq!(
                serde_json::to_string(&out.candidate.document()[field]).unwrap(),
                serde_json::to_string(&scene.document()[field]).unwrap(),
                "{field} after {value}"
            );
        }
    }
}

// ---- the planner alone: the reserve of `auto` (T14b) -----------------------------------------

fn plan_shot(
    id: &str,
    instance: &str,
    (start, end): (i64, i64),
    (instance_start, camera_in): (i64, i64),
    continuous_in: bool,
    reserve: i64,
) -> PlanShot {
    PlanShot {
        id: id.into(),
        instance: instance.into(),
        at: Placement {
            start,
            end,
            instance_start,
            camera_in,
        },
        continuous_in,
        reserve,
    }
}

fn instances(ids: &[&str]) -> Vec<PlanInstance> {
    ids.iter()
        .map(|id| PlanInstance {
            id: (*id).into(),
            frames: 200,
            locked: false,
        })
        .collect()
}

fn options(content: Content) -> ShotOptions {
    ShotOptions {
        content,
        ..ShotOptions::default()
    }
}

#[test]
fn the_reserve_of_a_transition_comes_off_what_auto_anchors() {
    let input = |reserve: i64| {
        PlanInput::new(
            90,
            vec![
                plan_shot("a", "ia", (0, 30), (0, 0), false, 0),
                plan_shot("b", "ib", (30, 60), (12, 12), false, reserve),
                plan_shot("c", "ic", (60, 90), (0, 0), false, 0),
            ],
            instances(&["ia", "ib", "ic"]),
        )
    };
    let trim = |content: Content| ShotOp::TrimShot {
        shot: "b".into(),
        edge: Edge::Start,
        frame: 20,
        options: options(content),
    };
    // 12 head frames, 5 of them kept for the transition: auto anchors 7 of the 10 and follows 3.
    let plan = plan_op(&input(5), &trim(Content::Auto)).unwrap().unwrap();
    let follow = plan.follow.as_ref().expect("auto fell to follow");
    assert_eq!((follow.anchored, follow.followed), (7, 3));
    assert_eq!(plan.placement("b").unwrap().instance_start, 5);
    // Without a transition it anchors the 10.
    let plan = plan_op(&input(0), &trim(Content::Auto)).unwrap().unwrap();
    assert!(plan.follow.is_none());
    assert_eq!(plan.placement("b").unwrap().instance_start, 2);
    // `anchor` is the person's word: the reserve does not apply.
    let plan = plan_op(&input(5), &trim(Content::Anchor)).unwrap().unwrap();
    assert!(plan.follow.is_none());
    assert_eq!(plan.placement("b").unwrap().instance_start, 2);
}

#[test]
fn the_reserve_never_makes_auto_follow_into_a_continuous_join() {
    let input = PlanInput::new(
        90,
        vec![
            plan_shot("a", "ia", (0, 30), (0, 0), false, 0),
            plan_shot("b", "ia", (30, 60), (30, 30), true, 25),
            plan_shot("c", "ic", (60, 90), (0, 0), false, 0),
        ],
        instances(&["ia", "ic"]),
    );
    let cut = ShotOp::MoveCut {
        from: "a".into(),
        to: "b".into(),
        frame: 20,
        options: options(Content::Auto),
    };
    let plan = plan_op(&input, &cut).unwrap().unwrap();
    assert!(plan.follow.is_none(), "auto anchors every frame here");
    assert_eq!(plan.placement("b").unwrap().instance_start, 20);
}
