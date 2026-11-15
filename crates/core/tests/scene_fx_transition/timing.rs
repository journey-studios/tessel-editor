//! P4b S2-C0b (BL082.4.4, BL082.4.20, BL082.4.23): the plan of a transition window declares the
//! `alignment` and the exact `timing` of both shots, and the one clock rule they all share.
//!
//! Every number is written by hand from the spec edit (`s1` `[0,48)` on `inst1` from frame 20 and
//! `cam1` (48 frames) from 0; `s2` `[48,96)` on `inst2` from 0 and `cam2` (96 frames) from 0; the
//! edit has 144 frames and the window is the 12 frames `[42,54)` centred on the cut 48). Nothing is
//! read back from the code under test. The refusals are asked of the three doors: the typed
//! `validate`, `from_json` and `from_json_str`.

use core::prelude::v1::test;
use serde_json::json;
use tessel_editor_core::directed_scene::ClockSample;
use tessel_editor_core::directed_scene::fx::{Alignment, DeckSide};
use tessel_editor_core::scene_plan::{
    DeckTiming, PLAN_VERSION, PlanError, PlanTimeline, ScenePlan, TransitionTiming,
};

use crate::support::*;

pub fn plan(frame: i64) -> ScenePlan {
    plan_at(&spec_scene(vec![tr(json!({}))]), frame)
}

pub fn pointer(error: PlanError) -> String {
    match error {
        PlanError::Invalid { pointer, .. } => pointer,
        other => panic!("expected a pointed refusal: {other:?}"),
    }
}

/// The three doors answer the same pointer, and none changes the plan.
pub fn refused_by_all(plan: &ScenePlan, at: &str, what: &str) {
    let text = plan.to_json_string();
    assert_eq!(
        pointer(plan.validate().expect_err(what)),
        at,
        "validate: {what}"
    );
    assert_eq!(
        pointer(ScenePlan::from_json(&plan.to_json()).expect_err(what)),
        at,
        "from_json: {what}"
    );
    assert_eq!(
        pointer(ScenePlan::from_json_str(&text).expect_err(what)),
        at,
        "from_json_str: {what}"
    );
    assert_eq!(plan.to_json_string(), text, "validation is read-only");
}

pub fn accepted_by_all(plan: &ScenePlan, what: &str) {
    plan.validate().expect(what);
    assert_eq!(&ScenePlan::from_json(&plan.to_json()).expect(what), plan);
    assert_eq!(
        &ScenePlan::from_json_str(&plan.to_json_string()).expect(what),
        plan
    );
}

fn timing(plan: &ScenePlan) -> TransitionTiming {
    plan.transition.as_deref().unwrap().timing
}

#[allow(clippy::too_many_arguments)]
fn deck(
    start: u64,
    end: u64,
    inst_start: u64,
    inst_len: u64,
    cam_in: u64,
    cam_len: u64,
    sampled_inst: u64,
    sampled_cam: u64,
) -> DeckTiming {
    DeckTiming {
        shot_start_frame: start,
        shot_end_frame_exclusive: end,
        instance_start_frame: inst_start,
        instance_duration_frames: inst_len,
        camera_in_frame: cam_in,
        camera_clip_duration_frames: cam_len,
        sampled_instance_frame: sampled_inst,
        sampled_camera_clip_frame: sampled_cam,
    }
}

fn sample(raw: i64, sampled: u64, held: bool) -> ClockSample {
    ClockSample { raw, sampled, held }
}

// ---- the declared numbers ------------------------------------------------------------------

#[test]
fn a_window_plan_declares_alignment_and_the_timing_of_both_shots() {
    let before = plan(44);
    let t = before.transition.as_deref().unwrap();
    assert_eq!(t.alignment, Alignment::Center);
    assert_eq!(
        t.timing,
        TransitionTiming {
            edit_duration_frames: 144,
            // 44: from keeps its raw clocks (20 + 44, 0 + 44); to is 4 frames before its shot.
            from: deck(0, 48, 20, 200, 0, 48, 64, 44),
            to: deck(48, 96, 0, 200, 0, 96, 0, 0),
        }
    );
    let after = plan(50);
    assert_eq!(
        timing(&after),
        TransitionTiming {
            edit_duration_frames: 144,
            // 50: from is two frames past its camera (48 frames): the sample is its last frame.
            from: deck(0, 48, 20, 200, 0, 48, 70, 47),
            to: deck(48, 96, 0, 200, 0, 96, 2, 2),
        }
    );
    assert_eq!(timing(&after).cut(), 48);
    assert_eq!(timing(&after).deck(DeckSide::To).shot_start_frame, 48);
}

#[test]
fn both_decks_clocks_are_readable_from_the_plan_with_raw_sampled_and_held() {
    let before = plan(44);
    let t = before.transition.as_deref().unwrap();
    let from = t.clocks(DeckSide::From).unwrap();
    assert_eq!(from.instance, sample(64, 64, false));
    assert_eq!(from.camera, sample(44, 44, false));
    let to = t.clocks(DeckSide::To).unwrap();
    assert_eq!(to.instance, sample(-4, 0, true));
    assert_eq!(to.camera, sample(-4, 0, true));
    assert_eq!(t.edit_frame().unwrap(), 44);
    assert!(t.held_instance && t.held_camera, "the neighbour is `to`");

    let after = plan(50);
    let t = after.transition.as_deref().unwrap();
    let from = t.clocks(DeckSide::From).unwrap();
    assert_eq!(from.instance, sample(70, 70, false));
    assert_eq!(from.camera, sample(50, 47, true));
    let to = t.clocks(DeckSide::To).unwrap();
    assert_eq!(to.instance, sample(2, 2, false));
    assert_eq!(to.camera, sample(2, 2, false));
    assert!(!t.held_instance && t.held_camera, "the neighbour is `from`");
    // The primary keeps its raw frames, the neighbour sits on its samples.
    assert_eq!(
        (after.frame.instance_frame, after.frame.camera_clip_frame),
        (2, 2)
    );
    assert_eq!(
        (
            t.other.frame.instance_frame,
            t.other.frame.camera_clip_frame
        ),
        (70, 47)
    );
}

#[test]
fn the_cut_the_last_real_frame_and_the_first_held_one_are_exact() {
    // C-1: `from` is primary on its last shot frame; its camera has no frame past 47 yet.
    let last = plan(47);
    let t = last.transition.as_deref().unwrap();
    assert_eq!(t.primary, DeckSide::From);
    let from = t.clocks(DeckSide::From).unwrap();
    assert_eq!(from.instance, sample(67, 67, false));
    assert_eq!(
        from.camera,
        sample(47, 47, false),
        "the last real frame is not held"
    );
    let to = t.clocks(DeckSide::To).unwrap();
    assert_eq!(to.instance, sample(-1, 0, true));
    assert_eq!(to.camera, sample(-1, 0, true));
    // C: `to` takes over on its first frame, and `from` is held on its camera for the first time.
    let cut = plan(48);
    let t = cut.transition.as_deref().unwrap();
    assert_eq!(t.primary, DeckSide::To);
    let to = t.clocks(DeckSide::To).unwrap();
    assert_eq!(to.instance, sample(0, 0, false));
    assert_eq!(
        to.camera,
        sample(0, 0, false),
        "the first frame is a real one"
    );
    let from = t.clocks(DeckSide::From).unwrap();
    assert_eq!(from.instance, sample(68, 68, false));
    assert_eq!(from.camera, sample(48, 47, true));
    assert!(!t.held_instance && t.held_camera);
    // A camera one frame longer makes frame 48 the neighbour's last real frame: still not held.
    let scene = scene_of(with_transitions(
        Edit::spec().camera("cam1", 49).bundle(),
        vec![tr(json!({}))],
    ));
    let at_cut = plan_at(&scene, 48);
    let t = at_cut.transition.as_deref().unwrap();
    assert_eq!(
        t.clocks(DeckSide::From).unwrap().camera,
        sample(48, 48, false)
    );
    assert!(!t.held_camera);
    assert_eq!(t.other.frame.camera_clip_frame, 48);
    assert!(
        plan_at(&scene, 49)
            .transition
            .as_deref()
            .unwrap()
            .held_camera
    );
}

#[test]
fn the_helper_holds_a_clip_without_frames_and_every_overflow_is_an_error() {
    let empty = deck(10, 20, 3, 0, 4, 0, 0, 0);
    let clocks = empty.clocks_at(12).unwrap();
    assert_eq!(clocks.instance, sample(5, 0, true));
    assert_eq!(clocks.camera, sample(6, 0, true));
    let normal = deck(10, 20, 3, 8, 4, 6, 0, 0);
    // The last real frame is not held, the one after it is.
    assert_eq!(
        normal.clocks_at(14).unwrap().camera,
        sample(8, 5, true),
        "frame 4+4=8 is past a 6-frame clip"
    );
    assert_eq!(normal.clocks_at(11).unwrap().camera, sample(5, 5, false));
    assert_eq!(normal.clocks_at(10).unwrap().camera, sample(4, 4, false));
    assert_eq!(normal.clocks_at(6).unwrap().camera, sample(0, 0, false));
    assert_eq!(normal.clocks_at(5).unwrap().camera, sample(-1, 0, true));
    let far = deck(0, 1, u64::MAX, 8, 0, 6, 0, 0);
    assert!(far.clocks_at(1).is_err());
    assert!(normal.clocks_at(i64::MIN).is_err());
}

#[test]
fn a_camera_offset_past_its_end_holds_its_tail_before_the_cut() {
    // `s2` enters `cam2` (48 frames) at 60: before C the head asks past the clip, so the camera
    // sits on its last frame (47) while the instance (from frame 6) is real: 2 at frame 44.
    let edit = Edit::spec()
        .camera("cam1", 60)
        .camera("cam2", 48)
        .shot("s2", |shot| {
            shot.instance_start = 6;
            shot.camera_in = 60;
        });
    let scene = scene_of(with_transitions(edit.bundle(), vec![tr(json!({}))]));
    let head = plan_at(&scene, 44);
    accepted_by_all(&head, "a camera held on its tail before the cut");
    let t = head.transition.as_deref().unwrap();
    assert_eq!(t.timing.to, deck(48, 96, 6, 200, 60, 48, 2, 47));
    assert_eq!(t.timing.from, deck(0, 48, 20, 200, 0, 60, 64, 44));
    assert_eq!(t.clocks(DeckSide::To).unwrap().camera, sample(56, 47, true));
    assert_eq!(
        t.clocks(DeckSide::To).unwrap().instance,
        sample(2, 2, false)
    );
    assert_eq!(
        (
            t.other.frame.instance_frame,
            t.other.frame.camera_clip_frame
        ),
        (2, 47)
    );
    assert!(t.held_camera && !t.held_instance);
    // From the cut the primary keeps the raw frame the shot asks for.
    let cut = plan_at(&scene, 50);
    accepted_by_all(&cut, "the primary camera past its clip");
    assert_eq!(cut.frame.camera_clip_frame, 62);
    assert_eq!(
        cut.transition
            .as_deref()
            .unwrap()
            .clocks(DeckSide::To)
            .unwrap()
            .camera,
        sample(62, 47, true)
    );
}

#[test]
fn a_short_primary_camera_keeps_the_raw_frame_and_samples_the_clip() {
    // The legacy example: raw 23 on a 12-frame clip samples frame 11.
    let edit = Edit::spec().camera("cam2", 12).shot("s2", |shot| {
        shot.camera_in = 18;
    });
    let scene = scene_of(with_transitions(edit.bundle(), vec![tr(json!({}))]));
    let late = plan_at(&scene, 53);
    accepted_by_all(&late, "a primary camera past its short clip");
    let t = late.transition.as_deref().unwrap();
    assert_eq!(t.primary, DeckSide::To);
    assert_eq!(late.frame.camera_clip_frame, 23);
    assert_eq!(t.timing.to.sampled_camera_clip_frame, 11);
    assert_eq!(t.timing.to.camera_clip_duration_frames, 12);
    assert_eq!(t.clocks(DeckSide::To).unwrap().camera, sample(23, 11, true));
    assert_eq!(
        t.clocks(DeckSide::To).unwrap().instance,
        sample(5, 5, false)
    );
}

#[test]
fn the_camera_length_is_the_longest_of_the_main_track_and_its_layers() {
    let mut raw = with_transitions(Edit::spec().bundle(), vec![tr(json!({}))]);
    // `cam1`: a 20-frame main track and a 60-frame layer.
    raw["cameraClips"][0]["segments"] =
        json!([{"recipe": "hold", "durationFrames": 20, "from": "initial"}]);
    raw["cameraClips"][0]["layers"] = json!([
        {"segments": [{"recipe": "hold", "durationFrames": 60, "from": "previousEnd"}]}
    ]);
    let scene = scene_of(raw);
    let before = plan_at(&scene, 44);
    accepted_by_all(&before, "main and layers of different lengths");
    assert_eq!(timing(&before).from.camera_clip_duration_frames, 60);
    assert_eq!(timing(&before).from.sampled_camera_clip_frame, 44);
    // `from` is the neighbour at 50: frame 50 is real on the layer, so it is not held; measuring
    // only the main track (20 frames) would hold it on 19.
    let after = plan_at(&scene, 50);
    accepted_by_all(&after, "the neighbour reads the longest track");
    let t = after.transition.as_deref().unwrap();
    assert_eq!(
        t.clocks(DeckSide::From).unwrap().camera,
        sample(50, 50, false)
    );
    assert_eq!(t.other.frame.camera_clip_frame, 50);
    assert!(!t.held_camera);
    // 60 and beyond: held on the last of the 60.
    let far = scene_of({
        let mut raw = with_transitions(Edit::spec().bundle(), vec![tr(json!({}))]);
        raw["cameraClips"][0]["segments"] =
            json!([{"recipe": "hold", "durationFrames": 20, "from": "initial"}]);
        raw["cameraClips"][0]["layers"] = json!([
            {"segments": [{"recipe": "hold", "durationFrames": 49, "from": "previousEnd"}]}
        ]);
        raw
    });
    let held = plan_at(&far, 50);
    assert_eq!(
        held.transition
            .as_deref()
            .unwrap()
            .clocks(DeckSide::From)
            .unwrap()
            .camera,
        sample(50, 48, true)
    );
}

// ---- the window's own shape ----------------------------------------------------------------

#[test]
fn every_alignment_and_duration_declares_its_window_and_its_primary() {
    for (overrides, alignment, start, end) in [
        (json!({"durationFrames": 1}), "center", 48, 49),
        (json!({"durationFrames": 5}), "center", 46, 51),
        (
            json!({"durationFrames": 6, "alignment": "start"}),
            "start",
            48,
            54,
        ),
        (
            json!({"durationFrames": 6, "alignment": "end"}),
            "end",
            42,
            48,
        ),
        (
            json!({"durationFrames": 7, "alignment": "end"}),
            "end",
            41,
            48,
        ),
    ] {
        let scene = spec_scene(vec![tr(overrides)]);
        let flat = spec_scene(vec![]);
        for frame in 36..60 {
            let built = plan_at(&scene, frame);
            let inside = (start..end).contains(&frame);
            assert_eq!(built.transition.is_some(), inside, "{alignment} {frame}");
            if !inside {
                // Outside the window the plan is the plain plan of the real primary.
                assert_eq!(built, plan_at(&flat, frame), "{alignment} {frame}");
                assert_eq!(
                    built.to_json_string(),
                    plan_at(&flat, frame).to_json_string()
                );
                continue;
            }
            accepted_by_all(&built, &format!("{alignment} {frame}"));
            let t = built.transition.as_deref().unwrap();
            assert_eq!(t.start_frame, start);
            assert_eq!(t.duration_frames, end - start);
            assert_eq!(t.frame_in_window, frame - start);
            assert_eq!(
                t.alignment.name(),
                alignment,
                "the alignment is on the wire form"
            );
            let expected = if frame < 48 {
                DeckSide::From
            } else {
                DeckSide::To
            };
            assert_eq!(t.primary, expected, "{alignment} {frame}");
            assert_eq!(t.timing.cut(), 48);
        }
    }
}

// ---- wire form -----------------------------------------------------------------------------

#[test]
fn the_wire_form_has_the_documented_timing_keys_integers_and_exact_bytes() {
    let built = plan(44);
    let value = built.to_json();
    assert_eq!(value["transition"]["alignment"], json!("center"));
    let timing = &value["transition"]["timing"];
    assert_eq!(
        timing,
        &json!({
            "editDurationFrames": 144,
            "from": {
                "shotStartFrame": 0, "shotEndFrameExclusive": 48, "instanceStartFrame": 20,
                "instanceDurationFrames": 200, "cameraInFrame": 0, "cameraClipDurationFrames": 48,
                "sampledInstanceFrame": 64, "sampledCameraClipFrame": 44
            },
            "to": {
                "shotStartFrame": 48, "shotEndFrameExclusive": 96, "instanceStartFrame": 0,
                "instanceDurationFrames": 200, "cameraInFrame": 0, "cameraClipDurationFrames": 96,
                "sampledInstanceFrame": 0, "sampledCameraClipFrame": 0
            }
        })
    );
    for number in [
        &timing["editDurationFrames"],
        &timing["from"]["shotStartFrame"],
        &timing["to"]["sampledCameraClipFrame"],
    ] {
        assert!(number.is_u64(), "{number}");
    }
    let text = built.to_json_string();
    assert_eq!(text, plan(44).to_json_string(), "deterministic");
    assert_eq!(
        ScenePlan::from_json_str(&text).unwrap().to_json_string(),
        text
    );
    accepted_by_all(&built, "the plan as built");
}

#[test]
fn a_plan_without_a_transition_keeps_its_version_and_its_bytes() {
    let flat = plan(20);
    assert_eq!(PLAN_VERSION, 1);
    assert_eq!(flat.version, 1);
    assert!(flat.transition.is_none());
    let text = flat.to_json_string();
    for key in ["alignment", "timing", "transition", "editDurationFrames"] {
        assert!(!text.contains(key), "{key}");
    }
    assert_eq!(text, plan_at(&spec_scene(vec![]), 20).to_json_string());
    assert_eq!(
        PlanTimeline::new(&spec_scene(vec![tr(json!({}))]))
            .build(20)
            .unwrap()
            .to_json_string(),
        text
    );
}
#[test]
fn a_shared_id_with_the_same_length_and_distinct_ids_with_different_lengths_are_valid() {
    // Different ids, different lengths (the spec edit: 48 and 96, `inst1` and `inst2`).
    accepted_by_all(&plan(44), "distinct ids");
    // The same camera clip on both shots (s2 and s3 share `cam2`): one length, accepted.
    let scene = scene_of(with_transitions(
        Edit::spec().bundle(),
        vec![tr(
            json!({"id": "tr-2", "fromShotId": "s2", "toShotId": "s3"}),
        )],
    ));
    let shared = plan_at(&scene, 95);
    accepted_by_all(&shared, "cam2 is on both shots");
    assert_eq!(
        timing(&shared).from.camera_clip_duration_frames,
        timing(&shared).to.camera_clip_duration_frames
    );
    assert_eq!(
        (
            timing(&shared).from.camera_in_frame,
            timing(&shared).to.camera_in_frame
        ),
        (0, 48)
    );
}

// ---- builders ------------------------------------------------------------------------------

#[test]
fn the_cold_build_and_the_timeline_agree_in_every_order() {
    let scene = spec_scene(vec![
        tr(json!({})),
        tr(json!({"id": "tr-2", "fromShotId": "s2", "toShotId": "s3", "alignment": "end"})),
    ]);
    let cold: Vec<String> = (0..144)
        .map(|frame| plan_at(&scene, frame).to_json_string())
        .collect();
    let timeline = PlanTimeline::new(&scene);
    let ascending = PlanTimeline::new(&scene);
    for frame in 0..144 {
        assert_eq!(
            ascending.build(frame).unwrap().to_json_string(),
            cold[frame as usize],
            "ascending {frame}"
        );
    }
    for frame in (0..144).rev() {
        assert_eq!(
            timeline.build(frame).unwrap().to_json_string(),
            cold[frame as usize],
            "descending {frame}"
        );
    }
    for frame in [44, 44, 50, 47, 48, 47, 90, 84, 95, 96, 44] {
        assert_eq!(
            timeline.build(frame).unwrap().to_json_string(),
            cold[frame as usize],
            "repeated {frame}"
        );
    }
}
