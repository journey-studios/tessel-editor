//! A shot evaluated at an edit frame outside its own range (F064, P4b S1b): the neighbour deck of
//! a transition window.
//!
//! The strict evaluator (`Bundle::evaluate_frame`, `mapping_inside_instance`) is untouched: it
//! maps a frame to the shot that plays it and refuses a frame past its instance. A transition
//! needs the *other* shot while the needle is under this one, so a frame before the shot's start
//! (the incoming shot's head) or from its end on (the outgoing shot's tail) is asked of a named
//! shot instead.
//!
//! The three clocks of that deck (plan §6.1, D14 and D17):
//!
//! * **instance** and **camera clip** are each clamped, SEPARATELY and each on its own length, to
//!   `[0, length − 1]`; the two flags say, for each one, whether it was actually held (a clamp that
//!   changed nothing reports `false`);
//! * the **scene clock** (lights with flicker, a future weather) keeps the REAL edit frame: the
//!   `edit_frame` and `edit_time` of the mapping are the needle's, whatever the other two hold. A
//!   deck in `hold` still flickers at `f / fps`.
//!
//! The answer is the same [`FrameEvaluation`] the strict evaluator returns, so everything that
//! consumes a frame (the plan) consumes this one. Pure: no state between calls, any order.

use std::borrow::Cow;

use crate::directed_scene::animation::NodeTimeline;
use crate::directed_scene::fx::available_frames;
use crate::directed_scene::{Bundle, SceneInstance, Shot};

use super::TemporalError;
use super::camera::CameraTimeline;
use super::frame::{FrameEvaluation, FrameMapping};

/// A shot's own frames and the real ones that exist around it, in `i64` (plan §6.1: `max(0, …)`
/// by meaning, so a camera shorter than its shot has no tail rather than a negative one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShotReach {
    /// Frames of the instance before the shot's first one (`instanceStartFrame`).
    pub head_instance: i64,
    /// Frames of the instance after the shot's last one.
    pub tail_instance: i64,
    /// Frames of the camera clip before the shot's first one (`cameraInFrame`).
    pub head_camera: i64,
    /// Frames of the camera clip after the shot's last one.
    pub tail_camera: i64,
}

/// A frame of a shot that may be outside it, with what was held to evaluate it.
#[derive(Debug, Clone, PartialEq)]
pub struct OffsetEvaluation<'a> {
    pub evaluation: FrameEvaluation<'a>,
    /// The instance frame was clamped (the real one does not exist).
    pub held_instance: bool,
    /// The camera clip frame was clamped.
    pub held_camera: bool,
}

fn signed(value: u64) -> Result<i64, TemporalError> {
    i64::try_from(value).map_err(|_| TemporalError::FrameOverflow)
}

/// `raw` held to `[0, length − 1]`, and whether holding changed it. Never negative, so the `u64`
/// it returns cannot wrap; a zero-length clip holds frame 0.
fn hold_to(raw: i64, length: u64) -> Result<(u64, bool), TemporalError> {
    let last = signed(length.saturating_sub(1))?;
    let held = raw.clamp(0, last);
    let frame = u64::try_from(held).map_err(|_| TemporalError::FrameOverflow)?;
    // A clip with no frames holds frame 0 always, even when 0 was the frame wanted (the oracle's
    // `empty_clips` rows): there is no real frame to be on.
    Ok((frame, length == 0 || held != raw))
}

impl Bundle {
    /// The frames of the instance and of the camera clip a shot has before and after its own
    /// range. `None` when the shot's instance does not resolve. A camera clip that cannot be
    /// measured has every frame (the oracle's infinity), so it never reports a lack.
    pub fn shot_reach(&self, shot: &Shot) -> Option<ShotReach> {
        let instance = self.instance(&shot.scene_instance_id)?;
        let to_i64 = |value: u64| i64::try_from(value).unwrap_or(i64::MAX);
        let length = to_i64(shot.duration_frames());
        let start = to_i64(shot.instance_start_frame);
        let camera_in = to_i64(shot.camera_in_frame);
        let camera = self.camera_clip_frames(&shot.camera_clip_id).map(to_i64);
        Some(ShotReach {
            head_instance: start,
            tail_instance: available_frames(to_i64(instance.duration_frames), start, length),
            head_camera: camera.map_or(i64::MAX, |_| camera_in),
            tail_camera: camera
                .map_or(i64::MAX, |total| available_frames(total, camera_in, length)),
        })
    }

    /// The shot `shot_id` at `edit_frame`, which may be outside the shot: see the module doc.
    /// `edit_frame` must be inside the edit (`EditFrameOutOfRange`); `shot_id` must exist
    /// (`Unresolved`).
    pub fn evaluate_shot_at_offset(
        &self,
        shot_id: &str,
        edit_frame: i64,
    ) -> Result<OffsetEvaluation<'_>, TemporalError> {
        let shot = self
            .shot(shot_id)
            .ok_or_else(|| TemporalError::Unresolved {
                kind: "shot",
                id: shot_id.to_string(),
            })?;
        self.evaluate_shot_at_offset_with(
            shot,
            edit_frame,
            |instance| NodeTimeline::new(self, instance).map(Cow::Owned),
            |clip_id| self.camera_timeline(clip_id).map(Cow::Owned),
        )
    }

    /// `evaluate_shot_at_offset` on timelines the caller owns (`PlanTimeline` keeps them), the one
    /// body of both. The camera layout comes first because its length is what the camera clock is
    /// held to; the nodes follow.
    pub(crate) fn evaluate_shot_at_offset_with<'a: 't, 't>(
        &'a self,
        shot: &'a Shot,
        edit_frame: i64,
        node_timeline: impl FnOnce(
            &'a SceneInstance,
        ) -> Result<Cow<'t, NodeTimeline<'a>>, TemporalError>,
        camera_timeline: impl FnOnce(&str) -> Result<Cow<'t, CameraTimeline<'a>>, TemporalError>,
    ) -> Result<OffsetEvaluation<'a>, TemporalError> {
        let out_of_range = TemporalError::EditFrameOutOfRange {
            frame: edit_frame,
            duration_frames: self.edit_duration_frames,
        };
        let frame = u64::try_from(edit_frame).map_err(|_| out_of_range.clone())?;
        if frame >= self.edit_duration_frames {
            return Err(out_of_range);
        }
        let instance =
            self.instance(&shot.scene_instance_id)
                .ok_or_else(|| TemporalError::Unresolved {
                    kind: "scene instance",
                    id: shot.scene_instance_id.clone(),
                })?;
        let camera_layout = camera_timeline(&shot.camera_clip_id)?;

        // Negative before the shot's start, `>= length` after its end; checked, never wrapped.
        let offset = edit_frame
            .checked_sub(signed(shot.start_frame)?)
            .ok_or(TemporalError::FrameOverflow)?;
        let raw_instance = signed(shot.instance_start_frame)?
            .checked_add(offset)
            .ok_or(TemporalError::FrameOverflow)?;
        let raw_camera = signed(shot.camera_in_frame)?
            .checked_add(offset)
            .ok_or(TemporalError::FrameOverflow)?;
        let (instance_frame, held_instance) = hold_to(raw_instance, instance.duration_frames)?;
        let (camera_clip_frame, held_camera) = hold_to(raw_camera, camera_layout.frames())?;

        let mapping = FrameMapping {
            // The scene clock: the needle's frame and time, never the held ones.
            edit_frame: frame,
            shot,
            instance,
            instance_frame,
            camera_clip_frame,
            edit_time: self.frame_time(frame)?,
            instance_time: self.frame_time(instance_frame)?,
            camera_clip_time: self.frame_time(camera_clip_frame)?,
        };
        let evaluation =
            self.finish_evaluation(mapping, node_timeline, move |_| Ok(camera_layout))?;
        Ok(OffsetEvaluation {
            evaluation,
            held_instance,
            held_camera,
        })
    }
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use super::hold_to;

    /// The oracle's `deck_frames.empty_clips` rows: a clip with no frames is frame 0, held.
    #[test]
    fn a_clip_without_frames_holds_frame_zero_even_when_zero_was_wanted() {
        for raw in [-3, 0, 5] {
            assert_eq!(hold_to(raw, 0), Ok((0, true)), "raw {raw}");
        }
        // The other clock of the same row has frames and is clamped on its own length.
        assert_eq!(hold_to(4, 10), Ok((4, false)));
    }

    #[test]
    fn holding_is_to_the_first_and_last_frame_and_never_wraps() {
        assert_eq!(hold_to(-1, 5), Ok((0, true)));
        assert_eq!(hold_to(0, 5), Ok((0, false)));
        assert_eq!(hold_to(4, 5), Ok((4, false)));
        assert_eq!(hold_to(5, 5), Ok((4, true)));
        assert_eq!(hold_to(i64::MIN, 5), Ok((0, true)));
        assert_eq!(hold_to(i64::MAX, 5), Ok((4, true)));
        assert_eq!(hold_to(0, 1), Ok((0, false)));
        assert!(
            hold_to(0, u64::MAX).is_err(),
            "a length that does not fit i64 is an overflow"
        );
    }
}
