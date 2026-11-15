//! The `timing` block of a plan, made from the bundle's real data.
//!
//! Both builders (`ScenePlan::build_with` and `PlanTimeline`) call [`TransitionTiming::derive`]
//! with the length of each camera clip in hand: `build_with` measures it, the timeline takes it
//! from its cache. The clocks come from `DeckTiming::clocks_at`, that is the core's one rule.

use crate::directed_scene::fx::FxTransition;
use crate::directed_scene::{Bundle, Shot, TemporalError};

use super::super::PlanError;
use super::model::{DeckTiming, TransitionTiming};

impl DeckTiming {
    fn of(
        bundle: &Bundle,
        shot: &Shot,
        edit_frame: i64,
        camera_frames: &impl Fn(&str) -> Result<u64, TemporalError>,
    ) -> Result<Self, PlanError> {
        let instance =
            bundle
                .instance(&shot.scene_instance_id)
                .ok_or_else(|| PlanError::Unresolved {
                    kind: "scene instance",
                    id: shot.scene_instance_id.clone(),
                })?;
        let mut timing = Self {
            shot_start_frame: shot.start_frame,
            shot_end_frame_exclusive: shot.end_frame_exclusive,
            instance_start_frame: shot.instance_start_frame,
            instance_duration_frames: instance.duration_frames,
            camera_in_frame: shot.camera_in_frame,
            camera_clip_duration_frames: camera_frames(&shot.camera_clip_id)?,
            sampled_instance_frame: 0,
            sampled_camera_clip_frame: 0,
        };
        let clocks = timing.clocks_at(edit_frame)?;
        timing.sampled_instance_frame = clocks.instance.sampled;
        timing.sampled_camera_clip_frame = clocks.camera.sampled;
        Ok(timing)
    }
}

impl TransitionTiming {
    /// The timing of the two shots of `transition` at `edit_frame`. `camera_frames` answers the
    /// length of a camera clip by id (`CameraTimeline::frames`: the longest of the main track and
    /// the layers), so the length is never measured on the main track alone.
    pub(in crate::scene_plan) fn derive(
        bundle: &Bundle,
        transition: &FxTransition,
        edit_frame: i64,
        camera_frames: impl Fn(&str) -> Result<u64, TemporalError>,
    ) -> Result<Self, PlanError> {
        let shot = |id: &str| {
            bundle.shot(id).ok_or_else(|| PlanError::Unresolved {
                kind: "shot",
                id: id.to_string(),
            })
        };
        Ok(Self {
            edit_duration_frames: bundle.edit_duration_frames,
            from: DeckTiming::of(
                bundle,
                shot(&transition.from_shot_id)?,
                edit_frame,
                &camera_frames,
            )?,
            to: DeckTiming::of(
                bundle,
                shot(&transition.to_shot_id)?,
                edit_frame,
                &camera_frames,
            )?,
        })
    }
}
