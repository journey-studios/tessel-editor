//! The one predicate that says whether an overlay may be edited at a frame (F064, P4b S3a; the
//! plan's §9.6, decision D16/Q1).
//!
//! Inside the window of a visible transition the picture is the blend of two decks, and an overlay
//! (a gizmo, a handle) drawn over it would act on a deck that is not the one the person sees. The
//! rule lives here, once: [`Fx::overlay_refusal`]. It reuses the choice `resolve_frame` makes
//! (`frame::active`: a visible transition, the half-open window `[start, end)`), so the guard and
//! the picture can never disagree about where a window is. Nothing calls it yet: the overlay
//! consumers (hit test, press, handles, the guard of an open gesture) arrive with S4a, and
//! accepting overlays on the top deck later is changing the body of this one function.
//!
//! Pure: it depends on the document and an integer frame, keeps no state, and gives the same answer
//! in any order of questions.

use super::frame::active;
use super::transition_problem::{RawProblem, TransitionProblem, project_problem};
use super::{Fx, Locale};
use crate::directed_scene::Shot;
use serde_json::json;

/// The code of a refusal by window (gesture level, like `gesture_op_not_allowed`; not an importer
/// issue).
pub const TRANSITION_WINDOW: &str = "transition_window";

/// Which overlay asks. Both are refused alike today; the parameter exists so a later phase can
/// accept one of them without a new predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayTarget {
    /// A node gizmo (`scene.gizmo.*`).
    Gizmo,
    /// A handle of a look clip (`fx_handle`).
    Handle,
}

/// Why an overlay may not be edited at this frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowRefusal {
    /// The frame is inside the window of a visible transition.
    TransitionWindow {
        transition_id: String,
        /// First frame of the window.
        start_frame: i64,
        /// First frame after the window (half-open).
        end_frame: i64,
        /// The frame asked.
        edit_frame: i64,
    },
}

impl WindowRefusal {
    /// The stable code ([`TRANSITION_WINDOW`]).
    pub fn code(&self) -> &'static str {
        match self {
            Self::TransitionWindow { .. } => TRANSITION_WINDOW,
        }
    }

    /// The transition whose window refuses.
    pub fn transition_id(&self) -> &str {
        match self {
            Self::TransitionWindow { transition_id, .. } => transition_id,
        }
    }

    /// The refusal as a localized problem: the same shape and the same table as every other
    /// transition problem.
    pub fn problem(&self, locale: Locale) -> TransitionProblem {
        let Self::TransitionWindow {
            transition_id,
            start_frame,
            end_frame,
            edit_frame,
        } = self;
        let details = json!({
            "transitionId": transition_id,
            "args": {
                "transitionId": transition_id, "start": start_frame, "end": end_frame,
                "frame": edit_frame,
            },
        });
        project_problem(
            &RawProblem {
                code: TRANSITION_WINDOW,
                path: None,
                message: "",
                details: Some(&details),
            },
            locale,
        )
    }
}

impl Fx {
    /// `Some` when `edit_frame` is inside the window of a visible transition, so an overlay of
    /// `target` must not be edited there; `None` outside every window (the end is exclusive), for
    /// a hidden transition, and for a transition whose shots or window do not resolve (the
    /// importer refuses those, so an accepted bundle never has one).
    pub fn overlay_refusal(
        &self,
        shots: &[Shot],
        edit_duration_frames: u64,
        edit_frame: i64,
        target: OverlayTarget,
    ) -> Option<WindowRefusal> {
        // Exhaustive on purpose: a new target must decide here whether it is refused.
        match target {
            OverlayTarget::Gizmo | OverlayTarget::Handle => {}
        }
        let (transition, window, _, _) = active(self, shots, edit_duration_frames, edit_frame)?;
        Some(WindowRefusal::TransitionWindow {
            transition_id: transition.id.clone(),
            start_frame: window.start,
            end_frame: window.end,
            edit_frame,
        })
    }
}
