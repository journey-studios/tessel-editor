//! The types of the clocks a transition window declares (F064, P4b S2-C0b), and how a consumer
//! reads them. No arithmetic of its own: every clock comes from the one core rule
//! (`directed_scene::DeckClocks`), the same one the neighbour deck's evaluation uses.

use crate::directed_scene::fx::DeckSide;
use crate::directed_scene::{ClockSource, DeckClocks, TemporalError};

use super::super::PlanError;
use super::super::transition::PlanTransition;

/// What one shot of a transition reads, and where its two clocks land at the frame of the plan.
///
/// The ids stay where they already are (the shot in `fromShotId`/`toShotId`, the instance in the
/// deck's `source.sceneInstanceId`, the camera clip in its `frame.cameraClipId`); this holds the
/// numbers only. `sampled_*` are what the transition's clock rule produces for the frame, which for
/// a neighbour deck is also what its `frame` carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeckTiming {
    /// First edit frame of the shot (`editRange.startFrame`).
    pub shot_start_frame: u64,
    /// One past its last edit frame.
    pub shot_end_frame_exclusive: u64,
    /// Instance frame the shot starts on.
    pub instance_start_frame: u64,
    /// Frames of the instance.
    pub instance_duration_frames: u64,
    /// Camera clip frame the shot starts on.
    pub camera_in_frame: u64,
    /// Frames of the camera clip: until the last of its tracks, main or layer, ends.
    pub camera_clip_duration_frames: u64,
    /// The instance clock held to `[0, instance_duration_frames − 1]` at the plan's edit frame.
    pub sampled_instance_frame: u64,
    /// The camera clock held to `[0, camera_clip_duration_frames − 1]` at the plan's edit frame.
    pub sampled_camera_clip_frame: u64,
}

impl DeckTiming {
    /// The wire key and value of every number, in wire order: the one list the writer and the
    /// validator share, so a key is spelled once.
    pub(super) fn fields(&self) -> [(&'static str, u64); 8] {
        [
            ("shotStartFrame", self.shot_start_frame),
            ("shotEndFrameExclusive", self.shot_end_frame_exclusive),
            ("instanceStartFrame", self.instance_start_frame),
            ("instanceDurationFrames", self.instance_duration_frames),
            ("cameraInFrame", self.camera_in_frame),
            ("cameraClipDurationFrames", self.camera_clip_duration_frames),
            ("sampledInstanceFrame", self.sampled_instance_frame),
            ("sampledCameraClipFrame", self.sampled_camera_clip_frame),
        ]
    }

    /// Where the shot enters its instance, and the instance's length.
    pub fn instance_source(&self) -> ClockSource {
        ClockSource {
            start_frame: self.instance_start_frame,
            length: self.instance_duration_frames,
        }
    }

    /// Where the shot enters its camera clip, and the clip's length.
    pub fn camera_source(&self) -> ClockSource {
        ClockSource {
            start_frame: self.camera_in_frame,
            length: self.camera_clip_duration_frames,
        }
    }

    /// The two clocks of this shot at `edit_frame`: raw, sampled and held for each, by the rule
    /// the core has once (`DeckClocks::at`).
    pub fn clocks_at(&self, edit_frame: i64) -> Result<DeckClocks, TemporalError> {
        DeckClocks::at(
            edit_frame,
            self.shot_start_frame,
            self.instance_source(),
            self.camera_source(),
        )
    }
}

/// The two shots a transition joins, with the length of the edit: everything a consumer needs to
/// check which deck is under the needle and what each deck's clocks hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransitionTiming {
    pub edit_duration_frames: u64,
    /// The outgoing shot (`fromShotId`).
    pub from: DeckTiming,
    /// The incoming shot (`toShotId`).
    pub to: DeckTiming,
}

impl TransitionTiming {
    pub fn deck(&self, side: DeckSide) -> &DeckTiming {
        match side {
            DeckSide::From => &self.from,
            DeckSide::To => &self.to,
        }
    }

    /// The cut `C`: the first edit frame of the incoming shot.
    pub fn cut(&self) -> u64 {
        self.to.shot_start_frame
    }
}

impl PlanTransition {
    /// The edit frame of the plan that carries this transition: `startFrame + frameInWindow`.
    pub fn edit_frame(&self) -> Result<i64, TemporalError> {
        self.start_frame
            .checked_add(self.frame_in_window)
            .ok_or(TemporalError::FrameOverflow)
    }

    /// The clocks of the deck of `side` at the plan's frame, from the timing the plan declares:
    /// raw, sampled and held for each of the instance and the camera clip. Available for both
    /// decks (the primary's `held` is not serialized: it is this).
    pub fn clocks(&self, side: DeckSide) -> Result<DeckClocks, PlanError> {
        Ok(self.timing.deck(side).clocks_at(self.edit_frame()?)?)
    }
}
