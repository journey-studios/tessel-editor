//! Why a shot op was refused (D19, F065): stable codes and the facts an adapter can act on.
//!
//! A [`Refusal`] is what the pure planner returns. It becomes an [`EditError`] at the edge of the
//! batch (`into_edit_error`), with the facts in `details`: `shot`, `edge`, `asked_frame` and
//! `limit_frame` (the nearest frame, in the op's own clock, where this same rule does not refuse,
//! decision #33), plus the few extra keys a rule has (`from`/`to`, `instance`, `option`).
//!
//! The command-code mapping (`error.rs` of `scene_edit`) and the localized texts are S3's; until
//! then an `EditError` of this module reaches a `CommandError` as `invalid_edit`.

use serde_json::{Map, Value, json};

use crate::scene_edit::EditError;

/// The plan would leave a shot with less than one frame.
pub const SHOT_TOO_SHORT: &str = "SHOT_TOO_SHORT";
/// An op names a shot the edit does not have.
pub const SHOT_NOT_FOUND: &str = "SHOT_NOT_FOUND";
/// `move_cut` between two shots that are not neighbours, or a roll with no neighbour to roll with.
pub const SHOTS_NOT_ADJACENT: &str = "SHOTS_NOT_ADJACENT";
/// The edit would pass [`super::MAX_EDIT_FRAMES`].
pub const EDIT_TOO_LONG: &str = "EDIT_TOO_LONG";
/// The op would take two shots of an `editLocked` instance out of step (#10).
pub const LOCKED: &str = "LOCKED";
/// The op would cut the head of a shot that carries on the previous one, or follow into a
/// continuous join (#8, #30).
pub const CONTINUOUS_JOIN: &str = "CONTINUOUS_JOIN";
/// `anchor` with fewer head frames than the shot gains (#9).
pub const NO_HEAD_FRAMES: &str = "NO_HEAD_FRAMES";
/// A deferred field with edit time in a shape the planner cannot move (D12). **Unreachable with
/// the list audited in S0**: the code exists for a field that joins the list later.
pub const SHOTS_LAYER_NOT_YET: &str = "SHOTS_LAYER_NOT_YET";
/// An option this phase refuses (`with_narration`, D13).
pub const SHOTS_OPTION_NOT_YET: &str = "SHOTS_OPTION_NOT_YET";

/// Every stable code of the module (D19), `TRANSITION_TOO_LONG` and `CANDIDATE_REJECTED`
/// included: the first is the layer's own (the S2 raises it), the second the gate's.
pub const CODES: [&str; 11] = [
    SHOT_TOO_SHORT,
    SHOT_NOT_FOUND,
    SHOTS_NOT_ADJACENT,
    EDIT_TOO_LONG,
    LOCKED,
    CONTINUOUS_JOIN,
    NO_HEAD_FRAMES,
    crate::directed_scene::fx::TRANSITION_TOO_LONG,
    SHOTS_LAYER_NOT_YET,
    SHOTS_OPTION_NOT_YET,
    crate::scene_edit::CANDIDATE_REJECTED,
];

/// The facts of a [`Refusal`].
#[derive(Debug, Clone, PartialEq)]
pub struct Facts {
    pub code: &'static str,
    pub message: String,
    pub shot: Option<String>,
    /// `start` or `end`: the border of `shot` the op moved.
    pub edge: Option<&'static str>,
    /// The frame the op asked for, in its own clock (the end of the shot for a duration).
    pub asked_frame: Option<i64>,
    /// The nearest frame where this rule does not refuse (#33), in the same clock as `asked_frame`.
    pub limit_frame: Option<i64>,
    /// Extra facts of the rule (`from`, `to`, `instance`, `shots`, `option`, `reason`).
    pub extra: Map<String, Value>,
}

/// A refusal of the planner, before it knows which op of the batch it was. Boxed, so the `Err` of
/// every planning function stays small.
#[derive(Debug, Clone, PartialEq)]
pub struct Refusal(Box<Facts>);

impl std::ops::Deref for Refusal {
    type Target = Facts;

    fn deref(&self) -> &Facts {
        &self.0
    }
}

impl std::ops::DerefMut for Refusal {
    fn deref_mut(&mut self) -> &mut Facts {
        &mut self.0
    }
}

impl Refusal {
    pub(super) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self(Box::new(Facts {
            code,
            message: message.into(),
            shot: None,
            edge: None,
            asked_frame: None,
            limit_frame: None,
            extra: Map::new(),
        }))
    }

    pub(super) fn shot(mut self, id: &str) -> Self {
        self.shot = Some(id.to_string());
        self
    }

    pub(super) fn limit(mut self, frame: i64) -> Self {
        self.limit_frame = Some(frame);
        self
    }

    pub(super) fn with(mut self, key: &str, value: Value) -> Self {
        self.extra.insert(key.to_string(), value);
        self
    }

    /// Fills what the planner of one rule does not know about the op that carried it.
    pub(super) fn in_op(
        mut self,
        shot: Option<&str>,
        edge: Option<&'static str>,
        asked: i64,
    ) -> Self {
        if self.shot.is_none() {
            self.shot = shot.map(str::to_string);
        }
        if self.edge.is_none() {
            self.edge = edge;
        }
        if self.asked_frame.is_none() {
            self.asked_frame = Some(asked);
        }
        self
    }

    /// The facts as the `details` of the error: only what the rule has.
    pub fn details(&self) -> Value {
        let mut details = self.extra.clone();
        if let Some(shot) = &self.shot {
            details.insert("shot".into(), json!(shot));
        }
        if let Some(edge) = self.edge {
            details.insert("edge".into(), json!(edge));
        }
        if let Some(asked) = self.asked_frame {
            details.insert("asked_frame".into(), json!(asked));
        }
        if let Some(limit) = self.limit_frame {
            details.insert("limit_frame".into(), json!(limit));
        }
        Value::Object(details)
    }

    pub fn into_edit_error(self) -> EditError {
        let details = self.details();
        let mut error = EditError::new(self.code, self.message.clone()).with_details(details);
        if self.code == SHOT_NOT_FOUND
            && let Some(shot) = &self.shot
        {
            error = error.with_id(shot);
        }
        error
    }
}
