//! T01-T04, T09 and T10: the four ops, one border at a time, and the refusals with their limits.

use core::prelude::v1::test;

use serde_json::json;
use tessel_editor_core::scene_edit::shots::{
    EDIT_TOO_LONG, MAX_EDIT_FRAMES, NO_HEAD_FRAMES, SHOT_NOT_FOUND, SHOT_TOO_SHORT,
    SHOTS_NOT_ADJACENT,
};

use super::support::*;
// ---- T01: set_shot_duration, ripple ----------------------------------------------------------

#[test]
fn t01_a_longer_shot_moves_the_following_ones_and_grows_the_edit() {
    let scene = three(20);
    let out = done(
        &scene,
        json!({"set_shot_duration": {"shot": "b", "frames": 40}}),
    );
    assert!(out.changed);
    assert_eq!(
        layout(&out.candidate),
        ["a [0,30) i0 c0", "b [30,70) i20 c20", "c [70,100) i0 c0"]
    );
    assert_eq!(edit_frames(&out.candidate), 100);
    // The scene holds the 20 + 40 frames the shot now shows: nothing grows.
    assert_eq!(instance_frames(&out.candidate, "ib"), 60);
    assert!(notes(&out).is_empty());

    let first = done(
        &scene,
        json!({"set_shot_duration": {"shot": "a", "frames": 40}}),
    );
    assert_eq!(
        layout(&first.candidate),
        ["a [0,40) i0 c0", "b [40,70) i20 c20", "c [70,100) i0 c0"]
    );
    let last = done(
        &scene,
        json!({"set_shot_duration": {"shot": "c", "frames": 40}}),
    );
    assert_eq!(
        layout(&last.candidate),
        ["a [0,30) i0 c0", "b [30,60) i20 c20", "c [60,100) i0 c0"]
    );
    assert_eq!(edit_frames(&last.candidate), 100);
}

#[test]
fn t01_a_shorter_shot_pulls_the_following_ones_back_and_the_instance_keeps_its_length() {
    let scene = three(20);
    let out = done(
        &scene,
        json!({"set_shot_duration": {"shot": "b", "frames": 10}}),
    );
    assert_eq!(
        layout(&out.candidate),
        ["a [0,30) i0 c0", "b [30,40) i20 c20", "c [40,70) i0 c0"]
    );
    assert_eq!(edit_frames(&out.candidate), 70);
    assert_eq!(instance_frames(&out.candidate, "ib"), 60);
    assert!(notes(&out).is_empty());
    // A shot may shrink to one frame.
    let one = done(
        &scene,
        json!({"set_shot_duration": {"shot": "a", "frames": 1}}),
    );
    assert_eq!(layout(&one.candidate)[0], "a [0,1) i0 c0");
    assert_eq!(layout(&one.candidate)[1], "b [1,31) i20 c20");
}

#[test]
fn t01_the_end_of_a_shot_is_its_duration_and_the_same_length_is_a_no_op() {
    let scene = three(20);
    let by_edge = done(
        &scene,
        json!({"trim_shot": {"shot": "b", "edge": "end", "frame": 70}}),
    );
    let by_length = done(
        &scene,
        json!({"set_shot_duration": {"shot": "b", "frames": 40}}),
    );
    assert_eq!(by_edge.candidate.document(), by_length.candidate.document());

    let same = done(
        &scene,
        json!({"set_shot_duration": {"shot": "b", "frames": 30}}),
    );
    assert!(!same.changed);
    assert_eq!(same.candidate.document(), scene.document());
    assert!(notes(&same).is_empty());
}

#[test]
fn t01_a_shot_shorter_than_one_frame_is_refused_with_the_legal_end() {
    let scene = three(20);
    let error = refused(
        &scene,
        json!({"set_shot_duration": {"shot": "b", "frames": 0}}),
    );
    assert_eq!(error.code, SHOT_TOO_SHORT);
    assert_eq!(detail(&error, "shot"), json!("b"));
    assert_eq!(detail(&error, "edge"), json!("end"));
    assert_eq!(detail(&error, "asked_frame"), json!(30));
    assert_eq!(detail(&error, "limit_frame"), json!(31));
    assert_eq!(error.index, Some(0));
    // At the limit the op does not refuse for that code (#33).
    let out = done(
        &scene,
        json!({"set_shot_duration": {"shot": "b", "frames": 1}}),
    );
    assert_eq!(layout(&out.candidate)[1], "b [30,31) i20 c20");

    let before = refused(
        &scene,
        json!({"trim_shot": {"shot": "b", "edge": "end", "frame": -5}}),
    );
    assert_eq!(before.code, SHOT_TOO_SHORT);
    assert_eq!(detail(&before, "asked_frame"), json!(-5));
    assert_eq!(detail(&before, "limit_frame"), json!(31));
}

// ---- T02: move_cut and roll ------------------------------------------------------------------

#[test]
fn t02_a_cut_moved_later_costs_the_next_shot_its_head_and_keeps_the_edit() {
    let out = done(
        &three(20),
        json!({"move_cut": {"from": "a", "to": "b", "frame": 40}}),
    );
    assert_eq!(
        layout(&out.candidate),
        ["a [0,40) i0 c0", "b [40,60) i30 c30", "c [60,90) i0 c0"]
    );
    assert_eq!(edit_frames(&out.candidate), 90);
}

#[test]
fn t02_a_cut_moved_earlier_anchors_or_follows_the_picture() {
    let scene = three(20);
    for content in [None, Some("anchor"), Some("auto")] {
        let mut body = json!({"from": "a", "to": "b", "frame": 20});
        if let Some(content) = content {
            body["content"] = json!(content);
        }
        let out = done(&scene, json!({"move_cut": body}));
        // The picture stays on its edit frames: the shot gains 10 head frames.
        assert_eq!(
            layout(&out.candidate),
            ["a [0,20) i0 c0", "b [20,60) i10 c10", "c [60,90) i0 c0"],
            "{content:?}"
        );
        assert!(notes(&out).is_empty(), "{content:?}");
    }
    let follow = done(
        &scene,
        json!({"move_cut": {"from": "a", "to": "b", "frame": 20, "content": "follow"}}),
    );
    // The scene starts at the new cut: the picture slides earlier, the take grows at its tail.
    assert_eq!(
        layout(&follow.candidate),
        ["a [0,20) i0 c0", "b [20,60) i20 c20", "c [60,90) i0 c0"]
    );
}

#[test]
fn t02_the_edges_in_roll_are_the_cut_with_the_neighbour() {
    let scene = three(20);
    let cut = done(
        &scene,
        json!({"move_cut": {"from": "a", "to": "b", "frame": 40}}),
    );
    for value in [
        json!({"trim_shot": {"shot": "a", "edge": "end", "frame": 40, "mode": "roll"}}),
        json!({"trim_shot": {"shot": "b", "edge": "start", "frame": 40, "mode": "roll"}}),
        json!({"set_shot_duration": {"shot": "a", "frames": 40, "mode": "roll"}}),
    ] {
        let out = done(&scene, value.clone());
        assert_eq!(
            out.candidate.document(),
            cut.candidate.document(),
            "{value}"
        );
    }
}

#[test]
fn t02_a_cut_cannot_cross_the_far_border_of_its_neighbours() {
    let scene = three(20);
    let left = refused(
        &scene,
        json!({"move_cut": {"from": "a", "to": "b", "frame": 0}}),
    );
    assert_eq!(left.code, SHOT_TOO_SHORT);
    assert_eq!(detail(&left, "limit_frame"), json!(1));
    let right = refused(
        &scene,
        json!({"move_cut": {"from": "a", "to": "b", "frame": 60}}),
    );
    assert_eq!(right.code, SHOT_TOO_SHORT);
    assert_eq!(detail(&right, "limit_frame"), json!(59));
    // At each limit the op is accepted.
    done(
        &scene,
        json!({"move_cut": {"from": "a", "to": "b", "frame": 1}}),
    );
    done(
        &scene,
        json!({"move_cut": {"from": "a", "to": "b", "frame": 59}}),
    );
    // The cut where it is changes nothing.
    let same = done(
        &scene,
        json!({"move_cut": {"from": "a", "to": "b", "frame": 30}}),
    );
    assert!(!same.changed);
}

// ---- T03: trim_shot start --------------------------------------------------------------------

#[test]
fn t03_cutting_the_head_keeps_the_shot_in_place_and_advances_its_scene() {
    let out = done(
        &three(20),
        json!({"trim_shot": {"shot": "b", "edge": "start", "frame": 40}}),
    );
    assert_eq!(
        layout(&out.candidate),
        ["a [0,30) i0 c0", "b [30,50) i30 c30", "c [50,80) i0 c0"]
    );
    assert_eq!(edit_frames(&out.candidate), 80);
}

#[test]
fn t03_a_head_that_goes_earlier_follows_content_anchor_follow_or_auto() {
    let scene = three(20);
    let trim = |content: &str| json!({"trim_shot": {"shot": "b", "edge": "start", "frame": 20, "content": content}});
    let anchor = done(&scene, trim("anchor"));
    assert_eq!(
        layout(&anchor.candidate),
        ["a [0,30) i0 c0", "b [30,70) i10 c10", "c [70,100) i0 c0"]
    );
    let follow = done(&scene, trim("follow"));
    assert_eq!(
        layout(&follow.candidate),
        ["a [0,30) i0 c0", "b [30,70) i20 c20", "c [70,100) i0 c0"]
    );
    assert!(notes(&follow).is_empty(), "only `auto` says it fell back");
    // With 20 head frames `auto` anchors every one of the 10: the same document, no note.
    let auto = done(&scene, trim("auto"));
    assert_eq!(auto.candidate.document(), anchor.candidate.document());
    assert!(notes(&auto).is_empty());
}

#[test]
fn t03_auto_anchors_the_head_frames_it_has_and_says_it_followed_the_rest() {
    let out = done(
        &three(6),
        json!({"trim_shot": {"shot": "b", "edge": "start", "frame": 20}}),
    );
    assert_eq!(
        layout(&out.candidate),
        ["a [0,30) i0 c0", "b [30,70) i0 c0", "c [70,100) i0 c0"]
    );
    assert_eq!(
        notes(&out),
        [json!({"code": "AUTO_FELL_TO_FOLLOW", "shot": "b", "anchored": 6, "followed": 4})]
    );
    // The camera clip is the smaller head: 3 of the 10 frames are anchored.
    let scene = build(
        90,
        &[
            sh("a", "ia", 0, 30, 0, 0),
            sh("b", "ib", 30, 60, 6, 3),
            sh("c", "ic", 60, 90, 0, 0),
        ],
        &[("ia", 60, false), ("ib", 60, false), ("ic", 60, false)],
    );
    let out = done(
        &scene,
        json!({"trim_shot": {"shot": "b", "edge": "start", "frame": 20}}),
    );
    assert_eq!(
        notes(&out),
        [json!({"code": "AUTO_FELL_TO_FOLLOW", "shot": "b", "anchored": 3, "followed": 7})]
    );
}

#[test]
fn t03_the_first_shot_has_no_head_so_follow_and_auto_take_everything_to_the_tail() {
    let scene = three(20);
    let follow = done(
        &scene,
        json!({"trim_shot": {"shot": "a", "edge": "start", "frame": -10, "content": "follow"}}),
    );
    assert_eq!(layout(&follow.candidate)[0], "a [0,40) i0 c0");
    assert!(notes(&follow).is_empty());
    let auto = done(
        &scene,
        json!({"trim_shot": {"shot": "a", "edge": "start", "frame": -10}}),
    );
    assert_eq!(layout(&auto.candidate)[0], "a [0,40) i0 c0");
    assert_eq!(
        notes(&auto),
        [json!({"code": "AUTO_FELL_TO_FOLLOW", "shot": "a", "anchored": 0, "followed": 10})]
    );
}

#[test]
fn t03_a_head_cut_to_the_end_of_the_shot_is_too_short() {
    let scene = three(20);
    let error = refused(
        &scene,
        json!({"trim_shot": {"shot": "b", "edge": "start", "frame": 60}}),
    );
    assert_eq!(error.code, SHOT_TOO_SHORT);
    assert_eq!(detail(&error, "edge"), json!("start"));
    assert_eq!(detail(&error, "limit_frame"), json!(59));
    done(
        &scene,
        json!({"trim_shot": {"shot": "b", "edge": "start", "frame": 59}}),
    );
    let same = done(
        &scene,
        json!({"trim_shot": {"shot": "b", "edge": "start", "frame": 30}}),
    );
    assert!(!same.changed);
}

// ---- T04: set_edit_duration ------------------------------------------------------------------

#[test]
fn t04_the_edit_length_is_the_end_of_the_last_shot() {
    let scene = three(20);
    // 20 more frames of `c`: its scene (60 frames) already holds them.
    let grow = done(&scene, json!({"set_edit_duration": {"frames": 110}}));
    assert_eq!(
        layout(&grow.candidate),
        ["a [0,30) i0 c0", "b [30,60) i20 c20", "c [60,110) i0 c0"]
    );
    assert_eq!(edit_frames(&grow.candidate), 110);
    assert!(notes(&grow).is_empty());
    // 70 more: it does not, and the instance grows to show them.
    let past = done(&scene, json!({"set_edit_duration": {"frames": 160}}));
    assert_eq!(edit_frames(&past.candidate), 160);
    assert_eq!(instance_frames(&past.candidate, "ic"), 100);
    assert_eq!(
        notes(&past),
        [json!({"code": "INSTANCE_GREW", "instance": "ic", "from": 60, "to": 100})]
    );
    let shrink = done(&scene, json!({"set_edit_duration": {"frames": 70}}));
    assert_eq!(layout(&shrink.candidate)[2], "c [60,70) i0 c0");
    assert_eq!(edit_frames(&shrink.candidate), 70);
    assert!(!done(&scene, json!({"set_edit_duration": {"frames": 90}})).changed);
}

#[test]
fn t04_an_edit_that_leaves_no_frame_to_the_last_shot_is_refused_with_the_limit() {
    let scene = three(20);
    let error = refused(&scene, json!({"set_edit_duration": {"frames": 60}}));
    assert_eq!(error.code, SHOT_TOO_SHORT);
    assert_eq!(detail(&error, "shot"), json!("c"));
    assert_eq!(detail(&error, "asked_frame"), json!(60));
    assert_eq!(detail(&error, "limit_frame"), json!(61));
    let out = done(&scene, json!({"set_edit_duration": {"frames": 61}}));
    assert_eq!(layout(&out.candidate)[2], "c [60,61) i0 c0");
}

// ---- T09: NO_HEAD_FRAMES ---------------------------------------------------------------------

#[test]
fn t09_anchor_without_enough_head_frames_is_refused_with_the_smaller_limit() {
    let scene = three(5);
    let trim = |frame: i64, content: &str| json!({"trim_shot": {"shot": "b", "edge": "start", "frame": frame, "content": content}});
    let error = refused(&scene, trim(20, "anchor"));
    assert_eq!(error.code, NO_HEAD_FRAMES);
    assert_eq!(detail(&error, "shot"), json!("b"));
    assert_eq!(detail(&error, "asked_frame"), json!(20));
    assert_eq!(detail(&error, "limit_frame"), json!(25));
    // At the limit anchor passes.
    done(&scene, trim(25, "anchor"));
    // The same for a cut.
    let cut = refused(
        &scene,
        json!({"move_cut": {"from": "a", "to": "b", "frame": 20, "content": "anchor"}}),
    );
    assert_eq!(cut.code, NO_HEAD_FRAMES);
    assert_eq!(detail(&cut, "limit_frame"), json!(25));

    // The camera has the shorter head: its 3 frames are the limit.
    let short_camera = build(
        90,
        &[
            sh("a", "ia", 0, 30, 0, 0),
            sh("b", "ib", 30, 60, 20, 3),
            sh("c", "ic", 60, 90, 0, 0),
        ],
        &[("ia", 60, false), ("ib", 60, false), ("ic", 60, false)],
    );
    let error = refused(&short_camera, trim(20, "anchor"));
    assert_eq!(detail(&error, "limit_frame"), json!(27));
}

#[test]
fn t09_follow_and_auto_never_refuse_for_lack_of_head_frames() {
    let scene = three(5);
    for content in ["follow", "auto"] {
        done(
            &scene,
            json!({"trim_shot": {"shot": "b", "edge": "start", "frame": 20, "content": content}}),
        );
    }
}

// ---- T10: the other refusals -----------------------------------------------------------------

#[test]
fn t10_an_edit_past_the_limit_is_refused_with_the_frame_that_reaches_it() {
    let scene = three(20);
    let edit = refused(
        &scene,
        json!({"set_edit_duration": {"frames": MAX_EDIT_FRAMES + 1}}),
    );
    assert_eq!(edit.code, EDIT_TOO_LONG);
    assert_eq!(detail(&edit, "limit_frame"), json!(MAX_EDIT_FRAMES));
    assert_eq!(detail(&edit, "max_frames"), json!(MAX_EDIT_FRAMES));
    let shot = refused(
        &scene,
        json!({"set_shot_duration": {"shot": "a", "frames": MAX_EDIT_FRAMES}}),
    );
    assert_eq!(shot.code, EDIT_TOO_LONG);
    // 60 frames stay after `a`: its end may be 216000 - 60.
    assert_eq!(detail(&shot, "limit_frame"), json!(MAX_EDIT_FRAMES - 60));
}

#[test]
fn t10_an_edit_of_exactly_the_limit_is_allowed() {
    let out = done(
        &three(20),
        json!({"set_edit_duration": {"frames": MAX_EDIT_FRAMES}}),
    );
    assert_eq!(edit_frames(&out.candidate), MAX_EDIT_FRAMES);
}

#[test]
fn t10_an_unknown_shot_and_shots_that_are_not_neighbours_are_refused() {
    let scene = three(20);
    let unknown = refused(
        &scene,
        json!({"set_shot_duration": {"shot": "zz", "frames": 10}}),
    );
    assert_eq!(unknown.code, SHOT_NOT_FOUND);
    assert_eq!(unknown.id.as_deref(), Some("zz"));
    let cut = refused(
        &scene,
        json!({"move_cut": {"from": "a", "to": "zz", "frame": 40}}),
    );
    assert_eq!(cut.code, SHOT_NOT_FOUND);
    for (from, to) in [("b", "a"), ("a", "c")] {
        let error = refused(
            &scene,
            json!({"move_cut": {"from": from, "to": to, "frame": 45}}),
        );
        assert_eq!(error.code, SHOTS_NOT_ADJACENT, "{from}->{to}");
        assert_eq!(detail(&error, "from"), json!(from));
    }
    // A roll needs the neighbour on the side of the edge.
    let last = refused(
        &scene,
        json!({"trim_shot": {"shot": "c", "edge": "end", "frame": 80, "mode": "roll"}}),
    );
    assert_eq!(last.code, SHOTS_NOT_ADJACENT);
    let first = refused(
        &scene,
        json!({"trim_shot": {"shot": "a", "edge": "start", "frame": 5, "mode": "roll"}}),
    );
    assert_eq!(first.code, SHOTS_NOT_ADJACENT);
}

#[test]
fn t10_the_shots_are_read_by_start_when_the_document_lists_them_out_of_order() {
    let mut document = bundle(
        90,
        &[
            sh("a", "ia", 0, 30, 0, 0),
            sh("b", "ib", 30, 60, 20, 20),
            sh("c", "ic", 60, 90, 0, 0),
        ],
        &[("ia", 60, false), ("ib", 60, false), ("ic", 60, false)],
        json!({}),
    );
    document["shots"].as_array_mut().unwrap().reverse();
    let scene = scene_of(document);
    let out = done(
        &scene,
        json!({"set_shot_duration": {"shot": "b", "frames": 40}}),
    );
    let by_start = |id: &str| {
        out.candidate.document()["shots"]
            .as_array()
            .unwrap()
            .iter()
            .find(|shot| shot["id"] == id)
            .unwrap()["editRange"]
            .clone()
    };
    assert_eq!(
        by_start("b"),
        json!({"startFrame": 30, "endFrameExclusive": 70})
    );
    assert_eq!(
        by_start("c"),
        json!({"startFrame": 70, "endFrameExclusive": 100})
    );
}
