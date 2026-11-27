//! The relations a plan's `timing` keeps with the rest of the plan (F064, P4b S2-C0b).
//!
//! They run from `check_transition`, so the three doors (`validate`, `from_json` and
//! `from_json_str`) and both builders judge a plan by the same code. Every refusal is
//! `PlanError::Invalid` with the pointer of the field that is wrong. Arithmetic is checked: an
//! overflow is a refusal, never a saturation, and nothing compares two failed conversions equal.
//!
//! Clocks come from `DeckTiming::clocks_at` (the core's one rule) and times from
//! `frame_start_time`; this file only compares. It judges coherence, not where the plan came from.

use crate::directed_scene::fx::{DeckSide, WindowError, transition_window};
use crate::directed_scene::{DeckClocks, frame_start_time};

use super::super::PlanError;
use super::super::model::ScenePlan;
use super::super::transition::PlanTransition;

fn invalid(pointer: impl Into<String>, message: impl Into<String>) -> PlanError {
    PlanError::invalid(pointer, message)
}

/// The pointer of one number of the timing of `side`.
fn at(side: DeckSide, key: &str) -> String {
    format!("/transition/timing/{}/{key}", side.name())
}

fn signed(pointer: &str, value: u64) -> Result<i64, PlanError> {
    i64::try_from(value).map_err(|_| invalid(pointer, "does not fit a signed 64-bit frame"))
}

impl ScenePlan {
    /// The deck of `side`: the plan itself when it is the primary, else the neighbour.
    fn deck_of<'a>(&'a self, transition: &'a PlanTransition, side: DeckSide) -> &'a ScenePlan {
        if side == transition.primary {
            self
        } else {
            &transition.other
        }
    }

    /// The declared clocks, windows and ids agree with each other and with the two decks.
    pub(in crate::scene_plan) fn check_transition_timing(
        &self,
        t: &PlanTransition,
    ) -> Result<(), PlanError> {
        let edit_frame = self.check_timing_shape(t)?;
        self.check_timing_window(t)?;
        self.check_timing_identities(t)?;
        self.check_timing_clocks(t, edit_frame)?;
        self.check_timing_times(t)
    }

    /// Representable numbers, non-empty shots inside the edit, one cut, the right primary and a
    /// frame inside it. Answers the edit frame as a signed number.
    fn check_timing_shape(&self, t: &PlanTransition) -> Result<i64, PlanError> {
        let timing = &t.timing;
        let edit = signed(
            "/transition/timing/editDurationFrames",
            timing.edit_duration_frames,
        )?;
        for side in [DeckSide::From, DeckSide::To] {
            let deck = timing.deck(side);
            for (key, value) in deck.fields() {
                signed(&at(side, key), value)?;
            }
            let end = at(side, "shotEndFrameExclusive");
            if deck.shot_start_frame >= deck.shot_end_frame_exclusive {
                return Err(invalid(
                    end,
                    "a shot has at least one frame: its end is after its start",
                ));
            }
            if deck.shot_end_frame_exclusive > timing.edit_duration_frames {
                return Err(invalid(
                    end,
                    "a shot ends inside the edit (editDurationFrames)",
                ));
            }
        }
        if timing.from.shot_end_frame_exclusive != timing.cut() {
            return Err(invalid(
                at(DeckSide::From, "shotEndFrameExclusive"),
                "the outgoing shot ends on the cut, the first frame of the incoming one",
            ));
        }
        let frame = i64::try_from(self.frame.edit_frame)
            .ok()
            .filter(|frame| *frame < edit)
            .ok_or_else(|| invalid("/frame/editFrame", "the frame is inside the edit"))?;
        let primary = if frame < signed("/transition/timing/to/shotStartFrame", timing.cut())? {
            DeckSide::From
        } else {
            DeckSide::To
        };
        if t.primary != primary {
            return Err(invalid(
                "/transition/primary",
                "the primary deck is `from` before the cut and `to` from it",
            ));
        }
        let deck = timing.deck(primary);
        if self.frame.edit_frame < deck.shot_start_frame {
            return Err(invalid(
                at(primary, "shotStartFrame"),
                "the frame is inside the primary shot: it starts at or before it",
            ));
        }
        if self.frame.edit_frame >= deck.shot_end_frame_exclusive {
            return Err(invalid(
                at(primary, "shotEndFrameExclusive"),
                "the frame is inside the primary shot: it ends after it",
            ));
        }
        Ok(frame)
    }

    /// The window of the cut, alignment and duration is the one the plan says.
    fn check_timing_window(&self, t: &PlanTransition) -> Result<(), PlanError> {
        let timing = &t.timing;
        let cut = signed("/transition/timing/to/shotStartFrame", timing.cut())?;
        let edit = signed(
            "/transition/timing/editDurationFrames",
            timing.edit_duration_frames,
        )?;
        let window =
            transition_window(cut, t.duration_frames, t.alignment, edit).map_err(|error| {
                match error {
                    WindowError::DurationBelowOne | WindowError::DurationAboveEdit => invalid(
                        "/transition/durationFrames",
                        "must be from 1 to the length of the edit",
                    ),
                    WindowError::Overflow => invalid(
                        at(DeckSide::To, "shotStartFrame"),
                        "the window of this cut overflows",
                    ),
                    WindowError::OutsideEdit(_) => invalid(
                        "/transition/startFrame",
                        "the window of this cut, alignment and duration is not inside the edit",
                    ),
                }
            })?;
        if window.start != t.start_frame {
            return Err(invalid(
                "/transition/startFrame",
                "must be where the window of the cut, the alignment and the duration starts",
            ));
        }
        Ok(())
    }

    /// One instance or one camera clip seen from both decks has one length; each instance
    /// covers its whole shot, including the neighbour's (the camera may not, decision 5).
    fn check_timing_identities(&self, t: &PlanTransition) -> Result<(), PlanError> {
        let timing = &t.timing;
        let from = self.deck_of(t, DeckSide::From);
        let to = self.deck_of(t, DeckSide::To);
        if from.source.scene_instance_id == to.source.scene_instance_id
            && timing.from.instance_duration_frames != timing.to.instance_duration_frames
        {
            return Err(invalid(
                at(DeckSide::To, "instanceDurationFrames"),
                "an instance seen from both decks has one length",
            ));
        }
        if from.frame.camera_clip_id == to.frame.camera_clip_id
            && timing.from.camera_clip_duration_frames != timing.to.camera_clip_duration_frames
        {
            return Err(invalid(
                at(DeckSide::To, "cameraClipDurationFrames"),
                "a camera clip seen from both decks has one length",
            ));
        }
        for side in [DeckSide::From, DeckSide::To] {
            let deck = timing.deck(side);
            let frames = deck.shot_end_frame_exclusive - deck.shot_start_frame;
            if deck
                .instance_start_frame
                .checked_add(frames)
                .is_none_or(|end| end > deck.instance_duration_frames)
            {
                return Err(invalid(
                    at(side, "instanceDurationFrames"),
                    "the instance holds all of its shot's frames",
                ));
            }
        }
        Ok(())
    }

    /// The declared samples are the rule's; the legacy frame fields are the primary's raw clocks
    /// and the neighbour's samples, and the neighbour's held flags are the rule's own.
    fn check_timing_clocks(&self, t: &PlanTransition, edit_frame: i64) -> Result<(), PlanError> {
        let clocks = |side: DeckSide| -> Result<DeckClocks, PlanError> {
            t.timing.deck(side).clocks_at(edit_frame).map_err(|_| {
                invalid(
                    at(side, "shotStartFrame"),
                    "the clocks of this shot overflow",
                )
            })
        };
        for side in [DeckSide::From, DeckSide::To] {
            let (deck, found) = (t.timing.deck(side), clocks(side)?);
            if deck.sampled_instance_frame != found.instance.sampled {
                return Err(invalid(
                    at(side, "sampledInstanceFrame"),
                    "must be the instance clock held to the instance's frames",
                ));
            }
            if deck.sampled_camera_clip_frame != found.camera.sampled {
                return Err(invalid(
                    at(side, "sampledCameraClipFrame"),
                    "must be the camera clock held to the camera clip's frames",
                ));
            }
        }
        // The primary keeps the raw clocks (its camera may be past its clip: decision 5).
        let primary = clocks(t.primary)?;
        if u64::try_from(primary.instance.raw).ok() != Some(self.frame.instance_frame) {
            return Err(invalid(
                "/frame/instanceFrame",
                "the primary deck keeps the raw instance frame of its shot",
            ));
        }
        if u64::try_from(primary.camera.raw).ok() != Some(self.frame.camera_clip_frame) {
            return Err(invalid(
                "/frame/cameraClipFrame",
                "the primary deck keeps the raw camera clip frame of its shot",
            ));
        }
        let other = clocks(t.primary.other())?;
        if t.other.frame.instance_frame != other.instance.sampled {
            return Err(invalid(
                "/transition/other/frame/instanceFrame",
                "the neighbour deck sits on its held instance frame",
            ));
        }
        if t.other.frame.camera_clip_frame != other.camera.sampled {
            return Err(invalid(
                "/transition/other/frame/cameraClipFrame",
                "the neighbour deck sits on its held camera clip frame",
            ));
        }
        if t.held_instance != other.instance.held {
            return Err(invalid(
                "/transition/heldInstance",
                "true exactly when the neighbour's instance clock was held",
            ));
        }
        if t.held_camera != other.camera.held {
            return Err(invalid(
                "/transition/heldCamera",
                "true exactly when the neighbour's camera clock was held",
            ));
        }
        Ok(())
    }

    /// Every time of both decks is the exact time of its own frame counter at the plan's rate.
    fn check_timing_times(&self, t: &PlanTransition) -> Result<(), PlanError> {
        let rate = self
            .output
            .fps
            .rate()
            .ok_or_else(|| invalid("/output/fps", "frame rate is not representable"))?;
        for (deck, prefix) in [(self, ""), (&t.other, "/transition/other")] {
            let frame = &deck.frame;
            for (counter, time, key) in [
                (frame.edit_frame, frame.edit_time, "editTime"),
                (frame.instance_frame, frame.instance_time, "instanceTime"),
                (
                    frame.camera_clip_frame,
                    frame.camera_clip_time,
                    "cameraClipTime",
                ),
            ] {
                if frame_start_time(rate, counter).ok() != Some(time) {
                    return Err(invalid(
                        format!("{prefix}/frame/{key}"),
                        "must be the exact time of its frame at the frame rate of the plan",
                    ));
                }
            }
        }
        Ok(())
    }
}
