//! Structural rules of the cuts between shots (`validateEditJoins`, `edit-joins.ts`).
//!
//! The origin runs these on every write and on preview, not when it parses a bundle, so they are
//! a separate query here too (`DirectedScene::edit_join_issues`). Only the integer rules are
//! ported. The pose comparison at a continuous join (`CONTINUOUS_CAMERA_JUMP`,
//! `CONTINUOUS_TARGET_JUMP`, `CONTINUOUS_FOV_JUMP`, `CONTINUOUS_VELOCITY_JUMP`,
//! `CONTINUOUS_FOV_VELOCITY_JUMP`) needs the camera evaluated at both sides of the join: BB03.

use super::issue::Issue;
use super::model::{Bundle, SegmentStart, Transition};
use super::read::{at, child};

pub(super) fn check(bundle: &Bundle) -> Vec<Issue> {
    let mut issues = Vec::new();
    for (c, clip) in bundle.camera_clips.iter().enumerate() {
        for (i, segment) in clip.segments.iter().enumerate().skip(1) {
            if segment.from == SegmentStart::Initial {
                issues.push(Issue::new(
                    "CAMERA_SEGMENT_RESET",
                    child(&at(&child(&at("/cameraClips", c), "segments"), i), "from"),
                    format!(
                        "Clip {} segment {i} uses from:initial mid-clip (resets pose)",
                        clip.id
                    ),
                ));
            }
        }
    }

    let mut order: Vec<usize> = (0..bundle.shots.len()).collect();
    order.sort_by_key(|&i| bundle.shots[i].start_frame);
    for pair in order.windows(2) {
        let (prev, next) = (&bundle.shots[pair[0]], &bundle.shots[pair[1]]);
        let path = at("/shots", pair[1]);
        if prev.end_frame_exclusive != next.start_frame {
            issues.push(Issue::new(
                "EDIT_GAP_OR_OVERLAP",
                &path,
                format!("Shots {} and {} do not meet exactly", prev.id, next.id),
            ));
            continue;
        }
        match &next.transition_into {
            None => issues.push(Issue::new(
                "MISSING_TRANSITION",
                &path,
                format!("Shot {} missing transitionInto", next.id),
            )),
            Some(Transition::Cut { reason }) => {
                if reason.trim().is_empty() {
                    issues.push(Issue::new(
                        "CUT_WITHOUT_REASON",
                        child(&path, "transitionInto"),
                        format!("Cut into {} requires reason", next.id),
                    ));
                }
            }
            Some(Transition::Continuous) => continuous_join(prev, next, &path, &mut issues),
        }
    }
    issues
}

fn continuous_join(
    prev: &super::model::Shot,
    next: &super::model::Shot,
    path: &str,
    issues: &mut Vec<Issue>,
) {
    if prev.scene_instance_id != next.scene_instance_id {
        issues.push(Issue::new(
            "CONTINUOUS_INSTANCE_MISMATCH",
            child(path, "sceneInstanceId"),
            format!(
                "Continuous join requires same instance between {} and {}",
                prev.id, next.id
            ),
        ));
    }
    let expected_instance = prev.instance_start_frame + prev.duration_frames();
    if next.instance_start_frame != expected_instance {
        issues.push(Issue::new(
            "INSTANCE_FRAME_DISCONTINUITY",
            child(path, "instanceStartFrame"),
            format!(
                "Instance mapping breaks at {}: expected start {expected_instance}, got {}",
                next.id, next.instance_start_frame
            ),
        ));
    }
    if prev.camera_clip_id != next.camera_clip_id {
        issues.push(Issue::new(
            "CONTINUOUS_CROSS_CLIP_UNSUPPORTED",
            child(path, "cameraClipId"),
            format!(
                "Pilot MVP rejects continuous joins across different camera clips ({}|{})",
                prev.id, next.id
            ),
        ));
        return;
    }
    let expected_camera_in = prev.camera_in_frame + prev.duration_frames();
    if next.camera_in_frame != expected_camera_in {
        issues.push(Issue::new(
            "CAMERA_CLIP_OFFSET_BREAK",
            child(path, "cameraInFrame"),
            format!(
                "Continuous same-clip join: {} cameraInFrame should be {expected_camera_in}, got {}",
                next.id, next.camera_in_frame
            ),
        ));
    }
}
