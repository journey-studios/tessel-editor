//! Rules of the cuts between shots (`validateEditJoins`, `edit-joins.ts`), in two separate gates.
//!
//! The origin runs these on every write and on preview, not when it parses a bundle, so they are
//! separate queries here too:
//!
//! - **Structural** (`check`, `DirectedScene::edit_join_issues`): the integer rules. Unchanged
//!   since BB02; a bundle that passes it is not certified smooth.
//! - **Temporal** (`check_temporal`, `DirectedScene::temporal_join_issues`): the pose comparison
//!   at a continuous join (`CONTINUOUS_CAMERA_JUMP`, `CONTINUOUS_TARGET_JUMP`,
//!   `CONTINUOUS_FOV_JUMP`, `CONTINUOUS_VELOCITY_JUMP`, `CONTINUOUS_FOV_VELOCITY_JUMP`), which
//!   needs the camera evaluated on both sides of the join (BB03).
//!
//! WHY two: the structural gate is cheap, needs no evaluation and has callers (the oracle test,
//! BB02 users) that must keep its exact meaning. Only both together equal the origin's
//! `validateEditJoins`.

use std::collections::HashMap;

use super::issue::Issue;
use super::model::{Bundle, SegmentStart, Shot, Transition};
use super::read::{at, child};
use super::temporal::{CameraState, CameraTimeline, TemporalError, distance};

/// Scene-unit tolerance for camera position/target at continuous joins (origin constant).
pub const JOIN_POSITION_TOLERANCE_SCENE_UNITS: f64 = 0.25;
/// Degrees; field of view must match within this at continuous joins (origin constant).
pub const JOIN_FOV_TOLERANCE_DEG: f64 = 2.0;
/// Scene units per frame; velocity jump tolerance at continuous joins (origin constant).
pub const JOIN_VELOCITY_TOLERANCE_SCENE_UNITS_PER_FRAME: f64 = 0.08;
/// Degrees per frame (origin constant).
pub const JOIN_FOV_VELOCITY_TOLERANCE_DEG_PER_FRAME: f64 = 0.5;

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

// ---- temporal gate --------------------------------------------------------------------------

/// The camera must not jump at a continuous join: the last frame of the previous shot and the
/// first frame of the next one, on the same clip, are compared on pose, field of view and both
/// speeds. Joins the structural gate rejects before comparing (gap, cut, missing transition,
/// another clip) are skipped here exactly as in the origin; joins that are structurally wrong
/// but on one clip (a wrong `cameraInFrame`, another instance) are still compared, as there.
pub(super) fn check_temporal(bundle: &Bundle) -> Vec<Issue> {
    let mut issues = Vec::new();
    let mut order: Vec<usize> = (0..bundle.shots.len()).collect();
    order.sort_by_key(|&i| bundle.shots[i].start_frame);
    // One layout per clip however many joins use it.
    let mut timelines: HashMap<&str, Result<CameraTimeline<'_>, TemporalError>> = HashMap::new();
    for pair in order.windows(2) {
        let (prev, next) = (&bundle.shots[pair[0]], &bundle.shots[pair[1]]);
        let continuous = next.transition_into == Some(Transition::Continuous);
        if prev.end_frame_exclusive != next.start_frame
            || !continuous
            || prev.camera_clip_id != next.camera_clip_id
        {
            continue;
        }
        let path = at("/shots", pair[1]);
        let timeline = timelines
            .entry(prev.camera_clip_id.as_str())
            .or_insert_with(|| bundle.camera_timeline(&prev.camera_clip_id));
        let states = timeline
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|timeline| {
                let end = last_clip_frame(prev).ok_or(TemporalError::FrameOverflow)?;
                let start = i64::try_from(next.camera_in_frame)
                    .map_err(|_| TemporalError::FrameOverflow)?;
                Ok((timeline.state_at(end)?, timeline.state_at(start)?))
            });
        match states {
            Ok((end, start)) => {
                compare_continuous_pose(&prev.id, &next.id, &end, &start, &path, &mut issues)
            }
            Err(error) => issues.push(Issue::new(
                "CAMERA_EVALUATION_FAILED",
                &path,
                format!("Join {}|{} cannot be compared: {error}", prev.id, next.id),
            )),
        }
    }
    issues
}

/// Clip frame the previous shot shows on its last edit frame.
fn last_clip_frame(shot: &Shot) -> Option<i64> {
    let after = shot.camera_in_frame.checked_add(shot.duration_frames())?;
    i64::try_from(after).ok()?.checked_sub(1)
}

fn compare_continuous_pose(
    prev_id: &str,
    next_id: &str,
    end: &CameraState,
    start: &CameraState,
    path: &str,
    issues: &mut Vec<Issue>,
) {
    let position = distance(end.position, start.position);
    if position > JOIN_POSITION_TOLERANCE_SCENE_UNITS {
        issues.push(Issue::new(
            "CONTINUOUS_CAMERA_JUMP",
            path,
            format!("Camera position jump at join {prev_id}|{next_id}: {position:.4} scene units"),
        ));
    }
    let target = distance(end.target, start.target);
    if target > JOIN_POSITION_TOLERANCE_SCENE_UNITS {
        issues.push(Issue::new(
            "CONTINUOUS_TARGET_JUMP",
            path,
            format!("Camera target jump at join {prev_id}|{next_id}: {target:.4} scene units"),
        ));
    }
    let fov = (end.fov_deg - start.fov_deg).abs();
    if fov > JOIN_FOV_TOLERANCE_DEG {
        issues.push(Issue::new(
            "CONTINUOUS_FOV_JUMP",
            path,
            format!("FOV jump at join {prev_id}|{next_id}: {fov:.4} deg"),
        ));
    }
    let velocity = distance(end.velocity, start.velocity);
    if velocity > JOIN_VELOCITY_TOLERANCE_SCENE_UNITS_PER_FRAME {
        issues.push(Issue::new(
            "CONTINUOUS_VELOCITY_JUMP",
            path,
            format!(
                "Camera velocity jump at join {prev_id}|{next_id}: {velocity:.4} scene units/frame"
            ),
        ));
    }
    let fov_velocity = (end.fov_velocity_deg_per_frame - start.fov_velocity_deg_per_frame).abs();
    if fov_velocity > JOIN_FOV_VELOCITY_TOLERANCE_DEG_PER_FRAME {
        issues.push(Issue::new(
            "CONTINUOUS_FOV_VELOCITY_JUMP",
            path,
            format!("FOV velocity jump at join {prev_id}|{next_id}: {fov_velocity:.4} deg/frame"),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directed_scene::Vec3;

    #[test]
    fn temporal_join_thresholds_accept_the_boundary_and_reject_only_above_it() {
        fn check_threshold(set: fn(&mut CameraState, f64), code: &str, limit: f64) {
            let end = CameraState {
                position: Vec3::ZERO,
                target: Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: -1.0,
                },
                fov_deg: 45.0,
                velocity: Vec3::ZERO,
                fov_velocity_deg_per_frame: 0.0,
            };
            for (delta, rejected) in [(limit - 1e-8, false), (limit, false), (limit + 1e-8, true)] {
                let mut start = end;
                set(&mut start, delta);
                let mut issues = Vec::new();
                compare_continuous_pose("a", "b", &end, &start, "/shots/1", &mut issues);
                assert_eq!(
                    issues.iter().any(|issue| issue.code == code),
                    rejected,
                    "{code}: {delta}"
                );
            }
        }
        check_threshold(
            |camera, delta| camera.position.x = delta,
            "CONTINUOUS_CAMERA_JUMP",
            JOIN_POSITION_TOLERANCE_SCENE_UNITS,
        );
        check_threshold(
            |camera, delta| camera.target.x = delta,
            "CONTINUOUS_TARGET_JUMP",
            JOIN_POSITION_TOLERANCE_SCENE_UNITS,
        );
        check_threshold(
            |camera, delta| camera.fov_deg += delta,
            "CONTINUOUS_FOV_JUMP",
            JOIN_FOV_TOLERANCE_DEG,
        );
        check_threshold(
            |camera, delta| camera.velocity.x = delta,
            "CONTINUOUS_VELOCITY_JUMP",
            JOIN_VELOCITY_TOLERANCE_SCENE_UNITS_PER_FRAME,
        );
        check_threshold(
            |camera, delta| camera.fov_velocity_deg_per_frame = delta,
            "CONTINUOUS_FOV_VELOCITY_JUMP",
            JOIN_FOV_VELOCITY_TOLERANCE_DEG_PER_FRAME,
        );
    }
}
