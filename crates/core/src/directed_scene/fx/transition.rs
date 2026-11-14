//! The typed view of one `fx.transitions` entry (F064, P4b S1a).
//!
//! Only the document: it says which two shots meet, how long the window is and where it sits on
//! the cut. How the window is drawn is the runtime's (S2); how it is evaluated frame by frame is
//! the plan's (S1b). Read by [`transition_read`](super::transition_read), checked against the
//! bundle by [`transition_explain`](super::transition_explain).

use super::{EffectSpec, FxEasing, FxValue};

/// Where the window sits on the cut `C` (the first frame of the incoming shot) for a duration `D`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Alignment {
    /// `[C − floor(D/2), C − floor(D/2) + D)`: the cut is in the window (the middle for even `D`).
    Center,
    /// `[C, C + D)`: the window starts on the cut, inside the incoming shot.
    Start,
    /// `[C − D, C)`: the window ends on the cut, inside the outgoing shot.
    End,
}

impl Alignment {
    pub const ALL: [Self; 3] = [Self::Center, Self::Start, Self::End];
    pub const NAMES: [&'static str; 3] = ["center", "start", "end"];

    pub fn name(self) -> &'static str {
        match self {
            Self::Center => "center",
            Self::Start => "start",
            Self::End => "end",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|alignment| alignment.name() == name)
    }
}

/// What a window does where an instance or a camera has no frames left to feed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Handles {
    /// The window reads the frames beyond the shot's own range; they must exist.
    Use,
    /// Missing frames are held (the first or last frame repeats). Never a refusal for lack of
    /// handles.
    Hold,
    /// The window must stay inside the shots' own frames: it needs real frames like `Use`, and
    /// the contract refuses rather than hold.
    Refuse,
}

impl Handles {
    pub const ALL: [Self; 3] = [Self::Use, Self::Hold, Self::Refuse];
    pub const NAMES: [&'static str; 3] = ["use", "hold", "refuse"];

    pub fn name(self) -> &'static str {
        match self {
            Self::Use => "use",
            Self::Hold => "hold",
            Self::Refuse => "refuse",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|handles| handles.name() == name)
    }

    /// Whether the window needs real frames before/after the shots' ranges.
    pub fn needs_frames(self) -> bool {
        !matches!(self, Self::Hold)
    }
}

/// One transition: `from_shot_id` to `to_shot_id` over `duration_frames`. `params` holds a value
/// per parameter of `effect`, in catalog order (a missing one is the default).
#[derive(Debug, Clone, PartialEq)]
pub struct FxTransition {
    pub id: String,
    pub effect: &'static EffectSpec,
    pub from_shot_id: String,
    pub to_shot_id: String,
    pub duration_frames: i64,
    pub alignment: Alignment,
    pub handles: Handles,
    pub easing: FxEasing,
    pub params: Vec<FxValue>,
    /// `0..=u32::MAX`, kept exactly as written.
    pub seed: Option<u32>,
    /// Only `true` is writable; the field is omitted otherwise.
    pub hidden: bool,
}

impl FxTransition {
    /// The value of a parameter by id.
    pub fn param(&self, id: &str) -> Option<FxValue> {
        let (index, _) = self.effect.param(id)?;
        self.params.get(index).copied()
    }
}
