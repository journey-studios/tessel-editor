//! Edit frame → shot → instance frame → camera clip frame (`resolve-shot-at-edit-frame.ts`,
//! `evaluate-edit-frame.ts`), and the honest boundary of what a frame evaluation covers.

use std::fmt;

use serde_json::{Value, json};

use crate::directed_scene::animation::{NodeFrame, NodeTimeline};
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

/// What a frame evaluation does not compute. Returned inside a result (never silently dropped) so
/// a caller cannot mistake a partial frame for a complete one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unsupported {
    /// The scene the instance plays declares sections that change the picture and that no
    /// evaluator computes yet (`weather`, `lighting`, `rigs`, `nodes[].rigBind`...). The camera and
    /// the node transforms of the frame are still evaluated; these sections have no state here.
    DeferredSections {
        scene_definition_id: String,
        sections: Vec<String>,
    },
}

impl Unsupported {
    pub fn to_json(&self) -> Value {
        match self {
            Self::DeferredSections {
                scene_definition_id,
                sections,
            } => json!({
                "unsupported": "deferred_sections",
                "sceneDefinitionId": scene_definition_id,
                "sections": sections,
            }),
        }
    }
}

impl fmt::Display for Unsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeferredSections {
                scene_definition_id,
                sections,
            } => write!(
                f,
                "scene {scene_definition_id} declares {} that no evaluator computes yet",
                sections.join(", ")
            ),
        }
    }
}

/// Whether performance channels moved the nodes of a frame. The node states themselves are always
/// in `FrameEvaluation::nodes`; this says where they came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeAnimation {
    /// No performance channel applies: the nodes sit at the transforms of their `SceneGraph`
    /// (through the same Euler path as an animated frame, so the two compare equal).
    Static,
    /// `channels` channel(s) of these tracks were evaluated at the frame.
    Animated {
        track_ids: Vec<String>,
        channels: usize,
    },
}

/// A whole frame: mapping, camera and nodes, and what is known not to be covered.
///
/// WHY `evaluate_frame` succeeds with `unevaluated` filled instead of erroring when the scene has
/// weather or lights: the camera and the nodes are complete and useful on their own (preview,
/// gates); refusing them would hide a correct answer. The caller that needs the whole truth asks
/// `is_complete` / `require_complete`.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameEvaluation<'a> {
    pub mapping: FrameMapping<'a>,
    pub camera: CameraState,
    pub node_animation: NodeAnimation,
    /// Every node of the instance's scene at this frame, with its matrices, queryable by id.
    pub nodes: NodeFrame,
    /// Sections the scene declares and nothing evaluates (see `Unsupported::DeferredSections`).
    pub unevaluated: Vec<&'static str>,
}

impl FrameEvaluation<'_> {
    /// Whether no declared animated/lighting section was deferred by this evaluator.
    /// This does not validate static material maps, assets, or render capabilities.
    pub fn is_complete(&self) -> bool {
        self.unevaluated.is_empty()
    }

    /// The evaluation, or the typed `Unsupported` that makes it partial.
    pub fn require_complete(self) -> Result<Self, TemporalError> {
        if self.unevaluated.is_empty() {
            return Ok(self);
        }
        Err(TemporalError::Unsupported(Unsupported::DeferredSections {
            scene_definition_id: self.mapping.instance.scene_definition_id.clone(),
            sections: self.unevaluated.iter().map(|s| s.to_string()).collect(),
        }))
    }

    /// The evaluated frame for a renderer or an adapter: stable camelCase JSON, the origin's
    /// `ResolvedFrameState` plus the world matrices, the animation provenance and what is left
    /// unevaluated. Numbers are the exact `f64`s; the times are exact `{numerator, denominator}`.
    pub fn to_json(&self) -> Value {
        let ratio = |value: Ratio| json!({ "numerator": value.num(), "denominator": value.den() });
        let camera = &self.camera;
        let animation = match &self.node_animation {
            NodeAnimation::Static => json!({ "status": "static" }),
            NodeAnimation::Animated {
                track_ids,
                channels,
            } => json!({ "status": "animated", "trackIds": track_ids, "channels": channels }),
        };
        let mut frame = json!({
            "editFrame": self.mapping.edit_frame,
            "shotId": self.mapping.shot.id,
            "instanceId": self.mapping.instance.id,
            "instanceFrame": self.mapping.instance_frame,
            "cameraClipId": self.mapping.shot.camera_clip_id,
            "cameraClipFrame": self.mapping.camera_clip_frame,
            "editTime": ratio(self.mapping.edit_time),
            "instanceTime": ratio(self.mapping.instance_time),
            "cameraClipTime": ratio(self.mapping.camera_clip_time),
            "camera": {
                "position": camera.position.to_json(),
                "target": camera.target.to_json(),
                "fovDeg": camera.fov_deg,
                "velocity": camera.velocity.to_json(),
                "fovVelocityDegPerFrame": camera.fov_velocity_deg_per_frame,
            },
            "animation": animation,
            "complete": self.is_complete(),
            "unevaluated": self.unevaluated,
        });
        // The node block is the one `NodeFrame::to_json` defines; merged flat so that the frame
        // keeps the origin's key names (`nodes`, `performanceOverrides`) at its top level.
        if let (Value::Object(map), Value::Object(nodes)) = (&mut frame, self.nodes.to_json()) {
            for key in ["sceneDefinitionId", "nodes", "performanceOverrides"] {
                if let Some(value) = nodes.get(key) {
                    map.insert(key.to_string(), value.clone());
                }
            }
        }
        frame
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

    /// The whole frame: shot mapping, camera, and the state of every node (performance channels
    /// applied, matrices composed through hierarchy and sockets), all at the instance frame the
    /// shot maps the edit frame to. Never fakes state: what the scene declares and nothing
    /// evaluates (lights, weather, rigs...) is listed in `unevaluated`.
    ///
    /// The checks run in the origin's order (mapping, instance range, channels, camera), so a frame
    /// that fails twice reports what the origin reports.
    pub fn evaluate_frame(&self, edit_frame: i64) -> Result<FrameEvaluation<'_>, TemporalError> {
        let mapping = self.mapping_inside_instance(edit_frame)?;
        let timeline = NodeTimeline::new(self, mapping.instance)?;
        let camera = self.camera_of(&mapping)?;
        let nodes = timeline.state_at(mapping.instance_frame)?;
        let plan = timeline.plan();
        let node_animation = if plan.is_empty() {
            NodeAnimation::Static
        } else {
            NodeAnimation::Animated {
                track_ids: plan.track_ids().into_iter().map(str::to_string).collect(),
                channels: plan.channel_count(),
            }
        };
        let unevaluated = self
            .scene(&mapping.instance.scene_definition_id)
            .map(|scene| scene.unevaluated_sections().to_vec())
            .unwrap_or_default();
        Ok(FrameEvaluation {
            mapping,
            camera,
            node_animation,
            nodes,
            unevaluated,
        })
    }
}
