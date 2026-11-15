//! The effects of one edit frame in stages, with the transition window (F064, P4b S1b).
//!
//! [`Fx::resolve`] is the list a frame without transition draws: the passes of the shot's
//! picture, then the output's. [`Fx::resolve_frame`] answers the same question for a frame that
//! may sit in a transition window and keeps the stages apart, because a transition draws TWO
//! decks and mixes them before the output:
//!
//! ```text
//! primary deck passes  ┐
//! other deck passes    ┘ → blend (progress, amount) → output passes (once)
//! ```
//!
//! Nothing here is stored as a shader weight: `progress` is the eased position in the window
//! (`ease((f − start + 0.5)/D)`), `amount` the clamped `intensity`. The shader's own smoothing of
//! `progress` (`curve`) is the runtime's.
//!
//! The deck under the needle is the *primary* one (the plan's top-level deck); the other is the
//! neighbour. Before the cut `C` the primary is the outgoing shot (`from`) and the neighbour the
//! incoming one in its head; from `C` on it is the other way round. A frame is never drawn with
//! the `from` deck always on top.
//!
//! Pure: a function of the document and the frame, in any order (no state between frames).

use serde_json::{Value, json};

use crate::directed_scene::Shot;

use super::{Fx, FxPass, FxStage, FxTransition, TransitionWindow, pass_of, transition_window};

/// Which of the transition's two shots a deck is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeckSide {
    /// The outgoing shot (`fromShotId`).
    From,
    /// The incoming shot (`toShotId`).
    To,
}

impl DeckSide {
    pub const NAMES: [&'static str; 2] = ["from", "to"];

    pub fn name(self) -> &'static str {
        match self {
            Self::From => "from",
            Self::To => "to",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "from" => Some(Self::From),
            "to" => Some(Self::To),
            _ => None,
        }
    }

    pub fn other(self) -> Self {
        match self {
            Self::From => Self::To,
            Self::To => Self::From,
        }
    }
}

/// A frame inside a transition window.
#[derive(Debug, Clone, PartialEq)]
pub struct FxTransitionFrame<'a> {
    pub transition: &'a FxTransition,
    /// The deck under the needle: `from` before the cut, `to` from it.
    pub primary: DeckSide,
    pub start_frame: i64,
    /// `f − start_frame`, inside `[0, durationFrames)`.
    pub frame_in_window: i64,
    /// `ease(easing, (frame_in_window + 0.5) / durationFrames)`: the value the plan stores.
    pub progress: f64,
    /// `clamp01(intensity)`. The blend's own, not the looks' `intensity × weight`.
    pub amount: f64,
    pub primary_shot_id: &'a str,
    pub other_shot_id: &'a str,
    /// The neighbour's `shot`-scope passes at its border frame (D15), drawn before the blend.
    pub other_passes: Vec<FxPass>,
}

impl FxTransitionFrame<'_> {
    /// The window as `scene.fx.frame` reports it (the style of that answer: snake_case keys).
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.transition.id, "effect": self.transition.effect.id,
            "from_shot_id": self.transition.from_shot_id, "to_shot_id": self.transition.to_shot_id,
            "primary": self.primary.name(),
            "primary_shot_id": self.primary_shot_id, "other_shot_id": self.other_shot_id,
            "start_frame": self.start_frame, "duration_frames": self.transition.duration_frames,
            "frame_in_window": self.frame_in_window, "easing": self.transition.easing.name(),
            "progress": self.progress, "amount": self.amount,
            "other_passes": self.other_passes.iter().map(FxPass::to_json).collect::<Vec<_>>(),
            "blend_passes": 1,
        })
    }
}

/// The passes of one edit frame, by stage.
#[derive(Debug, Clone, PartialEq)]
pub struct FxFrame<'a> {
    /// The `shot` passes of the shot under the needle (the primary deck), in track order.
    pub primary_passes: Vec<FxPass>,
    /// `Some` inside the window of a visible transition.
    pub transition: Option<FxTransitionFrame<'a>>,
    /// The `output` passes: once, after the blend, whatever the decks drew.
    pub output_passes: Vec<FxPass>,
}

impl FxFrame<'_> {
    /// The primary deck's passes, then the output's: the list [`Fx::resolve`] returns for the
    /// same frame, which is the plan's top-level `fx` (the neighbour's live in `transition.other`).
    pub fn flat_passes(&self) -> Vec<FxPass> {
        self.primary_passes
            .iter()
            .chain(&self.output_passes)
            .cloned()
            .collect()
    }

    /// Passes drawn at this frame: both decks, the blend (one) and the output, each once. What
    /// [`FX_PASS_BUDGET`](super::FX_PASS_BUDGET) counts for a frame.
    pub fn pass_count(&self) -> usize {
        let transition = self
            .transition
            .as_ref()
            .map_or(0, |frame| frame.other_passes.len() + 1);
        self.primary_passes.len() + transition + self.output_passes.len()
    }
}

/// The transition whose window holds `edit_frame`, with its window and its two shots. Hidden
/// transitions and ones whose shots or window do not resolve draw nothing (the importer refuses
/// them, so a bundle it accepted never reaches that case). Windows do not overlap.
pub(super) fn active<'a>(
    fx: &'a Fx,
    shots: &'a [Shot],
    edit_duration_frames: u64,
    edit_frame: i64,
) -> Option<(&'a FxTransition, TransitionWindow, &'a Shot, &'a Shot)> {
    let edit = i64::try_from(edit_duration_frames).ok()?;
    fx.transitions
        .iter()
        .filter(|transition| !transition.hidden)
        .find_map(|transition| {
            let (from, to) = shots_of(transition, shots)?;
            let cut = i64::try_from(to.start_frame).ok()?;
            let window =
                transition_window(cut, transition.duration_frames, transition.alignment, edit)
                    .ok()?;
            (window.start <= edit_frame && edit_frame < window.end)
                .then_some((transition, window, from, to))
        })
}

pub(super) fn shots_of<'a>(
    transition: &FxTransition,
    shots: &'a [Shot],
) -> Option<(&'a Shot, &'a Shot)> {
    let find = |id: &str| shots.iter().find(|shot| shot.id == id);
    Some((
        find(&transition.from_shot_id)?,
        find(&transition.to_shot_id)?,
    ))
}

/// The frame of `shot` a deck holds at `edit_frame`: `clamp(f, start, end − 1)` (D15).
pub(super) fn border_frame(shot: &Shot, edit_frame: i64) -> i64 {
    let first = i64::try_from(shot.start_frame).unwrap_or(i64::MAX);
    let last = i64::try_from(shot.end_frame_exclusive.saturating_sub(1)).unwrap_or(i64::MAX);
    edit_frame.clamp(first, first.max(last))
}

impl Fx {
    /// The effects of `edit_frame` in stages: the primary deck's, the transition (when the frame
    /// is in the window of a visible one) with the neighbour deck's, and the output's.
    ///
    /// * The neighbour's `shot` clips are sampled at its border frame (`deckEditFrame`, D15): a
    ///   clip that touches the border holds there (weight and keys as at the border, not zero and
    ///   not interpolated), a clip in the middle of the shot does not enter.
    /// * The one deliberate exception is the seed of a noise pass: it is taken from the REAL
    ///   frame, so a reseeding grain does not freeze while its deck holds.
    /// * The output passes act once, after the blend; they never appear in the neighbour's list.
    pub fn resolve_frame<'a>(
        &'a self,
        shots: &'a [Shot],
        edit_duration_frames: u64,
        edit_frame: i64,
    ) -> FxFrame<'a> {
        let (primary_passes, output_passes) = self
            .resolve(edit_frame)
            .into_iter()
            .partition(|pass| pass.scope.stage() == FxStage::Deck);
        let transition = active(self, shots, edit_duration_frames, edit_frame).map(
            |(transition, window, from, to)| {
                self.transition_frame(transition, window, from, to, edit_frame)
            },
        );
        FxFrame {
            primary_passes,
            transition,
            output_passes,
        }
    }

    fn transition_frame<'a>(
        &self,
        transition: &'a FxTransition,
        window: TransitionWindow,
        from: &'a Shot,
        to: &'a Shot,
        edit_frame: i64,
    ) -> FxTransitionFrame<'a> {
        let cut = i64::try_from(to.start_frame).unwrap_or(i64::MAX);
        let primary = if edit_frame < cut {
            DeckSide::From
        } else {
            DeckSide::To
        };
        let (primary_shot, other_shot) = match primary {
            DeckSide::From => (from, to),
            DeckSide::To => (to, from),
        };
        let frame_in_window = edit_frame - window.start;
        let progress = transition
            .easing
            .apply((frame_in_window as f64 + 0.5) / transition.duration_frames as f64);
        let amount = transition
            .effect
            .params
            .iter()
            .position(|spec| spec.id == "intensity")
            .and_then(|index| transition.params.get(index))
            .and_then(|value| value.as_number())
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        FxTransitionFrame {
            transition,
            primary,
            start_frame: window.start,
            frame_in_window,
            progress,
            amount,
            primary_shot_id: &primary_shot.id,
            other_shot_id: &other_shot.id,
            other_passes: self.deck_passes_at_border(other_shot, edit_frame),
        }
    }

    /// The `shot` passes of `shot` as its deck draws them at `edit_frame`, outside its range.
    pub(super) fn deck_passes_at_border(&self, shot: &Shot, edit_frame: i64) -> Vec<FxPass> {
        let held = border_frame(shot, edit_frame);
        self.tracks
            .iter()
            .filter(|track| !track.hidden)
            .flat_map(|track| {
                track
                    .clips
                    .iter()
                    .filter(|clip| {
                        !clip.hidden
                            && clip.scope.shot_id() == Some(shot.id.as_str())
                            && clip.start_frame <= held
                            && held < clip.end_frame_exclusive()
                    })
                    .map(move |clip| {
                        let mut pass = pass_of(track, clip, held);
                        // The one clock the deck does not hold: the noise seed (see above).
                        pass.seed = clip.seed_at(edit_frame.saturating_sub(clip.start_frame));
                        pass
                    })
            })
            .collect()
    }
}
