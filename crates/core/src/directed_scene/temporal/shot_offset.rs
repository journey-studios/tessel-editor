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
    /// Contiguous real frames before the shot: `cameraInFrame` up to the clip's exclusive end,
    /// zero beyond it, or `i64::MAX` when its length is unknown.
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

/// Where a shot enters one clip (its instance, or its camera clip) and how long the clip is: the
/// two numbers a clock needs besides the edit frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockSource {
    /// The clip frame the shot starts on (`instanceStartFrame`, `cameraInFrame`).
    pub start_frame: u64,
    /// Frames of the clip: the instance's duration, or the camera clip's own length.
    pub length: u64,
}

/// One clock of a deck at an edit frame: what the shot asks for, what exists, and whether the two
/// differ. The single rule of the neighbour deck, the plan's assembly and its validator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockSample {
    /// `start_frame + (edit_frame − shot_start)`, unclamped: may be negative or past the clip.
    pub raw: i64,
    /// `raw` held to `[0, length − 1]`; `0` for a clip with no frames.
    pub sampled: u64,
    /// The clock was held: the clamp changed the frame, or the clip has no frames at all (a
    /// clamp that changed nothing, as on the clip's last real frame, is `false`).
    pub held: bool,
}

impl ClockSample {
    /// `raw` held to `[0, length − 1]`, and whether holding changed it. Never negative, so the
    /// `u64` it returns cannot wrap; a zero-length clip holds frame 0.
    pub fn of(raw: i64, length: u64) -> Result<Self, TemporalError> {
        let last = signed(length.saturating_sub(1))?;
        let held = raw.clamp(0, last);
        let sampled = u64::try_from(held).map_err(|_| TemporalError::FrameOverflow)?;
        Ok(Self {
            raw,
            sampled,
            // A clip with no frames holds frame 0 always, even when 0 was the frame wanted (the
            // oracle's `empty_clips` rows): there is no real frame to be on.
            held: length == 0 || held != raw,
        })
    }
}

/// The two clocks of a deck at an edit frame, each held on its own length (D14, D17).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeckClocks {
    pub instance: ClockSample,
    pub camera: ClockSample,
}

impl DeckClocks {
    /// The clocks of a shot that starts at `shot_start_frame` at `edit_frame`, which may be
    /// outside the shot. Checked: an unrepresentable frame is `FrameOverflow`, never wrapped.
    pub fn at(
        edit_frame: i64,
        shot_start_frame: u64,
        instance: ClockSource,
        camera: ClockSource,
    ) -> Result<Self, TemporalError> {
        // Negative before the shot's start, `>= length` after its end.
        let delta = edit_frame
            .checked_sub(signed(shot_start_frame)?)
            .ok_or(TemporalError::FrameOverflow)?;
        let raw = |source: ClockSource| {
            signed(source.start_frame)?
                .checked_add(delta)
                .ok_or(TemporalError::FrameOverflow)
        };
        let raw_instance = raw(instance)?;
        let raw_camera = raw(camera)?;
        Ok(Self {
            instance: ClockSample::of(raw_instance, instance.length)?,
            camera: ClockSample::of(raw_camera, camera.length)?,
        })
    }
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
            // The head must end on a real frame. At `camera_in == total` the preceding
            // frames exist; beyond it even the closest preceding frame is already held.
            head_camera: camera.map_or(
                i64::MAX,
                |total| {
                    if camera_in <= total { camera_in } else { 0 }
                },
            ),
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

        // The one clock rule (also the plan's): each clock held on its own length.
        let clocks = DeckClocks::at(
            edit_frame,
            shot.start_frame,
            ClockSource {
                start_frame: shot.instance_start_frame,
                length: instance.duration_frames,
            },
            ClockSource {
                start_frame: shot.camera_in_frame,
                length: camera_layout.frames(),
            },
        )?;
        let (instance_frame, held_instance) = (clocks.instance.sampled, clocks.instance.held);
        let (camera_clip_frame, held_camera) = (clocks.camera.sampled, clocks.camera.held);

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

    use super::{ClockSample, ClockSource, DeckClocks, TemporalError};

    /// `(raw, sampled, held)` of the rule, or its error.
    fn of(raw: i64, length: u64) -> Result<(i64, u64, bool), TemporalError> {
        ClockSample::of(raw, length).map(|clock| (clock.raw, clock.sampled, clock.held))
    }

    /// The oracle's `deck_frames.empty_clips` rows: a clip with no frames is frame 0, held.
    #[test]
    fn a_clip_without_frames_holds_frame_zero_even_when_zero_was_wanted() {
        for raw in [-3, 0, 5] {
            assert_eq!(of(raw, 0), Ok((raw, 0, true)), "raw {raw}");
        }
        // The other clock of the same row has frames and is clamped on its own length.
        assert_eq!(of(4, 10), Ok((4, 4, false)));
    }

    #[test]
    fn holding_is_to_the_first_and_last_frame_and_never_wraps() {
        assert_eq!(of(-1, 5), Ok((-1, 0, true)));
        assert_eq!(of(0, 5), Ok((0, 0, false)));
        assert_eq!(of(4, 5), Ok((4, 4, false)));
        assert_eq!(of(5, 5), Ok((5, 4, true)));
        assert_eq!(of(i64::MIN, 5), Ok((i64::MIN, 0, true)));
        assert_eq!(of(i64::MAX, 5), Ok((i64::MAX, 4, true)));
        assert_eq!(of(0, 1), Ok((0, 0, false)));
        assert!(
            of(0, u64::MAX).is_err(),
            "a length that does not fit i64 is an overflow"
        );
    }

    fn clocks(frame: i64, start: u64, instance: (u64, u64), camera: (u64, u64)) -> DeckClocks {
        let source = |(start_frame, length)| ClockSource {
            start_frame,
            length,
        };
        DeckClocks::at(frame, start, source(instance), source(camera)).unwrap()
    }

    /// Hand numbers: a shot at 48 reading an instance from 20 (200 frames) and a camera from 0
    /// (48 frames). One frame before it, then its first frame, then the camera's last real frame
    /// and the one past it.
    #[test]
    fn the_two_clocks_are_offset_by_the_same_delta_and_held_on_their_own_length() {
        let at = |frame| clocks(frame, 48, (20, 200), (0, 48));
        let before = at(47);
        assert_eq!((before.instance.raw, before.instance.sampled), (19, 19));
        assert_eq!(
            (before.camera.raw, before.camera.sampled, before.camera.held),
            (-1, 0, true)
        );
        assert!(!before.instance.held);
        let first = at(48);
        assert_eq!((first.instance.raw, first.camera.raw), (20, 0));
        assert!(!first.instance.held && !first.camera.held);
        let last = at(95);
        assert_eq!((last.camera.raw, last.camera.sampled), (47, 47));
        assert!(
            !last.camera.held,
            "reaching the last frame is not holding it"
        );
        let past = at(96);
        assert_eq!(
            (past.camera.raw, past.camera.sampled, past.camera.held),
            (48, 47, true)
        );
        assert_eq!((past.instance.raw, past.instance.sampled), (68, 68));
    }

    #[test]
    fn a_clock_start_past_the_clip_end_is_held_before_the_cut_and_overflow_is_an_error() {
        // cameraIn 60 of a 48-frame clip: held on the tail even three frames before the shot.
        let early = clocks(45, 48, (0, 200), (60, 48));
        assert_eq!(
            (early.camera.raw, early.camera.sampled, early.camera.held),
            (57, 47, true)
        );
        let huge = ClockSource {
            start_frame: u64::MAX,
            length: 4,
        };
        let small = ClockSource {
            start_frame: 0,
            length: 4,
        };
        assert!(DeckClocks::at(0, 0, huge, small).is_err());
        assert!(DeckClocks::at(i64::MIN, 1, small, small).is_err());
        assert!(DeckClocks::at(i64::MAX, 0, small, small).is_ok());
        assert!(
            DeckClocks::at(
                i64::MAX,
                0,
                ClockSource {
                    start_frame: 1,
                    length: 4
                },
                small
            )
            .is_err()
        );
    }
}
