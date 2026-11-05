//! Edit frame → shot → instance frame → camera clip frame (`resolve-shot-at-edit-frame.ts`,
//! `evaluate-edit-frame.ts`), and the honest boundary of what a frame evaluation covers.

use std::fmt;

use serde_json::{Value, json};

use crate::directed_scene::{Bundle, CameraClip, SceneInstance, Shot};
use crate::time::{Rate, Ratio};

use super::TemporalError;
use super::camera::{CameraState, CameraTimeline};

/// Where an edit frame lands: the shot that plays it and the matching frame of the instance and of
/// the camera clip. The frames are integers (shots only shift them); the times are exact
/// `Ratio`s of the bundle's frame rate, so `24000/1001` and `30000/1001` do not drift.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameMapping<'a> {
    pub edit_frame: u64,
    pub shot: &'a Shot,
    pub instance: &'a SceneInstance,
    pub instance_frame: u64,
    pub camera_clip_frame: u64,
    /// Start time of the edit frame, `frame * den / num` seconds.
    pub edit_time: Ratio,
    pub instance_time: Ratio,
    pub camera_clip_time: Ratio,
}

/// The camera at an edit frame, with where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct CameraEvaluation<'a> {
    pub mapping: FrameMapping<'a>,
    pub camera: CameraState,
}

/// What this evaluator does not compute. Returned inside a result (never silently dropped) so a
/// caller cannot mistake a partial frame for a complete one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unsupported {
    /// The instance plays performance channels (cycles, events, static overrides of node
    /// transforms). Their evaluation is not part of BB03's first slice: the nodes would sit at
    /// their static transforms while the origin moves them.
    PerformanceChannels {
        instance_id: String,
        track_ids: Vec<String>,
        channels: usize,
    },
}

impl Unsupported {
    pub fn to_json(&self) -> Value {
        match self {
            Self::PerformanceChannels {
                instance_id,
                track_ids,
                channels,
            } => json!({
                "unsupported": "performance_channels",
                "instanceId": instance_id,
                "trackIds": track_ids,
                "channels": channels,
            }),
        }
    }
}

impl fmt::Display for Unsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PerformanceChannels {
                instance_id,
                channels,
                ..
            } => write!(
                f,
                "instance {instance_id} plays {channels} performance channel(s); node animation is not evaluated"
            ),
        }
    }
}

/// State of the scene nodes at a frame, as far as this evaluator knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeAnimation {
    /// No performance channel applies: the nodes sit at the transforms of their `SceneGraph`.
    Static,
    /// Channels apply and are not evaluated. The graph's static transforms are **not** the nodes'
    /// transforms at this frame.
    Unsupported(Unsupported),
}

/// A whole frame: mapping, camera and what is known about node animation.
///
/// WHY not a single `evaluate_frame` that errors when channels exist: the camera is complete and
/// useful on its own (preview, gates); refusing it would hide a correct answer. The caller that
/// needs the whole truth asks `is_complete` / `require_complete`.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameEvaluation<'a> {
    pub mapping: FrameMapping<'a>,
    pub camera: CameraState,
    pub node_animation: NodeAnimation,
}

impl FrameEvaluation<'_> {
    /// Whether every part of the frame was evaluated.
    pub fn is_complete(&self) -> bool {
        self.node_animation == NodeAnimation::Static
    }

    /// The evaluation, or the typed `Unsupported` that makes it partial.
    pub fn require_complete(self) -> Result<Self, TemporalError> {
        match self.node_animation {
            NodeAnimation::Static => Ok(self),
            NodeAnimation::Unsupported(what) => Err(TemporalError::Unsupported(what)),
        }
    }
}

impl Bundle {
    fn rate(&self) -> Result<Rate, TemporalError> {
        self.fps.rate().ok_or(TemporalError::InvalidFrameRate)
    }

    /// Start time of a frame, exact. Checked: a frame too large for the integer ratio is an
    /// error, where `Rate::frame_to_time` would answer zero.
    fn frame_time(&self, frame: u64) -> Result<Ratio, TemporalError> {
        let rate = self.rate()?;
        let seconds_per_frame = Ratio::new(i64::from(rate.den()), i64::from(rate.num()));
        seconds_per_frame
            .zip(i64::try_from(frame).ok())
            .and_then(|(per_frame, frames)| per_frame.mul_int(frames))
            .ok_or(TemporalError::FrameOverflow)
    }

    /// Edit frame whose interval contains `time` (floor, so a boundary belongs to the frame that
    /// starts there). Negative times give negative frames; whether the frame is inside the edit is
    /// for `resolve_frame_mapping` to say.
    pub fn frame_at_time(&self, time: Ratio) -> Result<i64, TemporalError> {
        let rate = self.rate()?;
        // WHY here and not `Rate::time_to_frame_floor`: that one narrows with `as` and wraps for a
        // time beyond `i64` frames, which a hand-made `Ratio` can reach.
        let scaled = i128::from(time.num()) * i128::from(rate.num());
        let divisor = i128::from(time.den()) * i128::from(rate.den());
        i64::try_from(scaled.div_euclid(divisor)).map_err(|_| TemporalError::FrameOverflow)
    }

    /// Which shot plays an edit frame and which frames of the instance and of the camera clip it
    /// shows. Pure integer arithmetic; shots only shift frames.
    pub fn resolve_frame_mapping(
        &self,
        edit_frame: i64,
    ) -> Result<FrameMapping<'_>, TemporalError> {
        let out_of_range = TemporalError::EditFrameOutOfRange {
            frame: edit_frame,
            duration_frames: self.edit_duration_frames,
        };
        let frame = u64::try_from(edit_frame).map_err(|_| out_of_range.clone())?;
        if frame >= self.edit_duration_frames {
            return Err(out_of_range);
        }
        let shot = self
            .shot_at_frame(frame)
            .ok_or(TemporalError::NoShotAtFrame { frame })?;
        let instance =
            self.instance(&shot.scene_instance_id)
                .ok_or_else(|| TemporalError::Unresolved {
                    kind: "scene instance",
                    id: shot.scene_instance_id.clone(),
                })?;
        // `shot_at_frame` guarantees `start_frame <= frame`.
        let offset = frame - shot.start_frame;
        let instance_frame = shot
            .instance_start_frame
            .checked_add(offset)
            .ok_or(TemporalError::FrameOverflow)?;
        let camera_clip_frame = shot
            .camera_in_frame
            .checked_add(offset)
            .ok_or(TemporalError::FrameOverflow)?;
        Ok(FrameMapping {
            edit_frame: frame,
            shot,
            instance,
            instance_frame,
            camera_clip_frame,
            edit_time: self.frame_time(frame)?,
            instance_time: self.frame_time(instance_frame)?,
            camera_clip_time: self.frame_time(camera_clip_frame)?,
        })
    }

    /// `resolve_frame_mapping` for the frame that starts at or contains `time`.
    pub fn resolve_time_mapping(&self, time: Ratio) -> Result<FrameMapping<'_>, TemporalError> {
        self.resolve_frame_mapping(self.frame_at_time(time)?)
    }

    /// A clip laid out for evaluation. Build it once to ask for many frames.
    pub fn camera_timeline(&self, clip_id: &str) -> Result<CameraTimeline<'_>, TemporalError> {
        CameraTimeline::new(self.clip_or_error(clip_id)?)
    }

    fn clip_or_error(&self, clip_id: &str) -> Result<&CameraClip, TemporalError> {
        self.camera_clip(clip_id)
            .ok_or_else(|| TemporalError::Unresolved {
                kind: "camera clip",
                id: clip_id.to_string(),
            })
    }

    /// Length of a camera clip in frames: until its last track ends. `None` for an unknown clip
    /// or one whose frame count overflows.
    pub fn camera_clip_frames(&self, clip_id: &str) -> Option<u64> {
        Some(self.camera_timeline(clip_id).ok()?.frames())
    }

    /// The camera of a clip at one of its frames. Past the clip's end it rests on its last pose.
    pub fn camera_at_clip_frame(
        &self,
        clip_id: &str,
        clip_frame: i64,
    ) -> Result<CameraState, TemporalError> {
        self.camera_timeline(clip_id)?.state_at(clip_frame)
    }

    /// The pose each main-track movement of a clip starts and ends on. Layers add to these.
    pub fn main_track_poses(
        &self,
        clip_id: &str,
    ) -> Result<Vec<super::SegmentPoses>, TemporalError> {
        self.camera_timeline(clip_id)?.main_track_poses()
    }

    fn mapping_inside_instance<'a>(
        &'a self,
        edit_frame: i64,
    ) -> Result<FrameMapping<'a>, TemporalError> {
        let mapping = self.resolve_frame_mapping(edit_frame)?;
        if mapping.instance_frame >= mapping.instance.duration_frames {
            return Err(TemporalError::InstanceFrameOutOfRange {
                instance_id: mapping.instance.id.clone(),
                frame: mapping.instance_frame,
                duration_frames: mapping.instance.duration_frames,
            });
        }
        Ok(mapping)
    }

    fn camera_of(&self, mapping: &FrameMapping<'_>) -> Result<CameraState, TemporalError> {
        let clip_frame =
            i64::try_from(mapping.camera_clip_frame).map_err(|_| TemporalError::FrameOverflow)?;
        self.camera_at_clip_frame(&mapping.shot.camera_clip_id, clip_frame)
    }

    /// The camera at an edit frame: shot, instance and clip frames, then the pose and speed with
    /// the layers on top. Complete for the camera; says nothing about the nodes (use
    /// `evaluate_frame` for that boundary).
    pub fn evaluate_camera(&self, edit_frame: i64) -> Result<CameraEvaluation<'_>, TemporalError> {
        let mapping = self.mapping_inside_instance(edit_frame)?;
        let camera = self.camera_of(&mapping)?;
        Ok(CameraEvaluation { mapping, camera })
    }

    /// `evaluate_camera` plus the state of node animation: `Static` when no performance channel
    /// applies, `Unsupported` (typed, with the tracks) when one does. Never fakes node state.
    pub fn evaluate_frame(&self, edit_frame: i64) -> Result<FrameEvaluation<'_>, TemporalError> {
        let mapping = self.mapping_inside_instance(edit_frame)?;
        let camera = self.camera_of(&mapping)?;
        let node_animation = self.node_animation_of(mapping.instance);
        Ok(FrameEvaluation {
            mapping,
            camera,
            node_animation,
        })
    }

    /// The same selection the origin makes: tracks of this instance that the instance lists.
    fn node_animation_of(&self, instance: &SceneInstance) -> NodeAnimation {
        let applied: Vec<_> = self
            .performance_tracks
            .iter()
            .filter(|track| {
                track.instance_id == instance.id
                    && instance.performance_track_ids.contains(&track.id)
                    && !track.channels.is_empty()
            })
            .collect();
        if applied.is_empty() {
            return NodeAnimation::Static;
        }
        NodeAnimation::Unsupported(Unsupported::PerformanceChannels {
            instance_id: instance.id.clone(),
            track_ids: applied.iter().map(|track| track.id.clone()).collect(),
            channels: applied.iter().map(|track| track.channels.len()).sum(),
        })
    }
}
