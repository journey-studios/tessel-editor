//! The part of bundle integrity that needs the camera evaluated: the field of view of a clip's
//! tracks stacked together (`validateCameraLayers`, end of `bundle-integrity.ts`).

use crate::directed_scene::read::{at, child};
use crate::directed_scene::{Bundle, Issue};

use super::camera::CameraTimeline;

/// Longest clip, in frames, whose stacked field of view is swept frame by frame. Past it the gate
/// reports `TEMPORAL_CHECK_BUDGET_EXCEEDED` instead of running unbounded on a hostile duration
/// (the importer bounds counts, not frame numbers). 1 000 000 frames is 11 hours at 24 fps.
pub const LAYER_FOV_SWEEP_MAX_FRAMES: u64 = 1_000_000;

/// For each clip with layers: the composed field of view must stay inside (0, 180) at every
/// frame, because the layers' zooms add to the main track's. One issue per clip, at the first
/// frame that breaks. Clips without layers are covered by the importer already (their zooms are
/// checked per movement).
pub(in crate::directed_scene) fn layered_fov_issues(bundle: &Bundle) -> Vec<Issue> {
    let mut issues = Vec::new();
    for (index, clip) in bundle.camera_clips.iter().enumerate() {
        if clip.layers.is_empty() {
            continue;
        }
        let path = at("/cameraClips", index);
        let timeline = match CameraTimeline::new(clip) {
            Ok(timeline) => timeline,
            Err(error) => {
                issues.push(evaluation_failed(&path, &clip.id, &error));
                continue;
            }
        };
        if timeline.frames() > LAYER_FOV_SWEEP_MAX_FRAMES {
            issues.push(Issue::new(
                "TEMPORAL_CHECK_BUDGET_EXCEEDED",
                &path,
                format!(
                    "Clip {} has {} frames; the stacked field of view is swept up to {LAYER_FOV_SWEEP_MAX_FRAMES} and was not checked",
                    clip.id,
                    timeline.frames()
                ),
            ));
            continue;
        }
        for frame in 0..timeline.frames() {
            // `frames()` is at most the budget, so it fits `i64`.
            let verdict = i64::try_from(frame)
                .map_err(|_| super::TemporalError::FrameOverflow)
                .and_then(|frame| timeline.pose_at(frame));
            match verdict {
                // `!(a && b)` and not `<= 0 || >= 180`: a NaN field of view is out of range too.
                Ok(pose) if !(pose.fov_deg > 0.0 && pose.fov_deg < 180.0) => {
                    issues.push(Issue::new(
                        "FOV_OUT_OF_RANGE",
                        &path,
                        format!(
                            "Clip {} zooms out of range at frame {frame} with its tracks together",
                            clip.id
                        ),
                    ));
                    break;
                }
                Ok(_) => {}
                Err(error) => {
                    issues.push(evaluation_failed(&path, &clip.id, &error));
                    break;
                }
            }
        }
    }
    issues
}

fn evaluation_failed(path: &str, clip_id: &str, error: &super::TemporalError) -> Issue {
    Issue::new(
        "CAMERA_EVALUATION_FAILED",
        child(path, "layers"),
        format!("Clip {clip_id} cannot be evaluated: {error}"),
    )
}
