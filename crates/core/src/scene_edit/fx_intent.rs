//! What an effect gesture is FOR (F016/F058, BL082.33): the verb family and the target an
//! `scene.fx.gesture.update` proposes, so a gesture that began as a drag of one key cannot turn
//! into a slider write, a retime of the whole curve, a seed or another parameter's key halfway.
//!
//! The intent is a pure function of the ops. The gesture binds the first one it accepts and
//! refuses every later proposal that is not the same intent: the refusal is typed, it leaves the
//! candidate, its last legal ops and the gesture's revision binding exactly as they were, and it
//! is recorded as the newest refusal. A direct `scene.fx.edit` is not a gesture: it may still
//! apply a deliberate batch that mixes keys and base values.
//!
//! Compatibility with the first gestures (P2): every static write (`set_param`, `set_field`,
//! `set_hidden`, and since F062 `set_scope`) is ONE intent, `value`, with no further target, as
//! before; a slider gesture may still change from one field to another between updates, because
//! each update derives from the base. What is new is that the key verbs have their own targets:
//! a single key is named by its parameter and its frame **in the base the gesture began on** (a
//! `move_key` names its `from`), a curve verb by its parameter (or the whole clip).

use super::FxOp;

/// The refusal code of a proposal whose verb is not the gesture's.
pub const GESTURE_INTENT_MISMATCH: &str = "gesture_intent_mismatch";
/// The refusal code of a proposal of the gesture's verb at another parameter or key.
pub const GESTURE_TARGET_MISMATCH: &str = "gesture_target_mismatch";

/// The verb family and target of a gesture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FxIntent {
    /// `value`, `key_param`, `set_key_easing`, `remove_key`, `move_key`, `shift_keys`,
    /// `scale_keys`, `clear_keys` or `set_seed`.
    pub verb: &'static str,
    /// The parameter the verb works on (`None` for `value`, `set_seed`, and a curve verb that
    /// names the whole clip).
    pub param: Option<String>,
    /// The frame of the single key the verb works on, in the gesture's base.
    pub key: Option<i64>,
}

/// Why a proposal is not the gesture's intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntentRefusal {
    /// Another verb (a key op in a value gesture, two verbs in one batch).
    Verb {
        index: usize,
        expected: FxIntent,
        found: FxIntent,
    },
    /// The same verb at another parameter or key.
    Target {
        index: usize,
        expected: FxIntent,
        found: FxIntent,
    },
}

impl FxIntent {
    /// The intent of one op; `None` for the ops a gesture does not take (adding or removing a clip).
    pub fn of(op: &FxOp) -> Option<Self> {
        let named = |verb, param: &str, key| Self {
            verb,
            param: Some(param.to_string()),
            key,
        };
        let curve = |verb, param: &Option<String>| Self {
            verb,
            param: param.clone(),
            key: None,
        };
        Some(match op {
            FxOp::AddClip(_) | FxOp::RemoveClip { .. } => return None,
            FxOp::SetParam { .. }
            | FxOp::SetField { .. }
            | FxOp::SetHidden { .. }
            | FxOp::SetScope { .. } => Self {
                verb: "value",
                param: None,
                key: None,
            },
            FxOp::KeyParam { param, frame, .. } => named("key_param", param, Some(*frame)),
            FxOp::SetKeyEasing { param, frame, .. } => named("set_key_easing", param, Some(*frame)),
            FxOp::RemoveKey { param, frame, .. } => named("remove_key", param, Some(*frame)),
            FxOp::MoveKey { param, from, .. } => named("move_key", param, Some(*from)),
            FxOp::ShiftKeys { param, .. } => curve("shift_keys", param),
            FxOp::ScaleKeys { param, .. } => curve("scale_keys", param),
            FxOp::ClearKeys { param, .. } => curve("clear_keys", param),
            FxOp::SetSeed { .. } => Self {
                verb: "set_seed",
                param: None,
                key: None,
            },
        })
    }

    /// The intent of a whole proposal: every op the same, none of them an add or remove. `None`
    /// for an empty proposal (the gesture back at its start, which binds nothing). Checks it
    /// against `bound` when the gesture has one.
    pub fn of_batch(
        ops: &[FxOp],
        bound: Option<&FxIntent>,
    ) -> Result<Option<FxIntent>, IntentRefusal> {
        let mut found: Option<FxIntent> = bound.cloned();
        let mut first: Option<FxIntent> = None;
        for (index, op) in ops.iter().enumerate() {
            let Some(intent) = Self::of(op) else {
                continue;
            };
            match &found {
                None => {
                    found = Some(intent.clone());
                    first = Some(intent);
                }
                Some(expected) if expected.verb != intent.verb => {
                    return Err(IntentRefusal::Verb {
                        index,
                        expected: expected.clone(),
                        found: intent,
                    });
                }
                Some(expected) if *expected != intent => {
                    return Err(IntentRefusal::Target {
                        index,
                        expected: expected.clone(),
                        found: intent,
                    });
                }
                Some(_) => {
                    if first.is_none() {
                        first = Some(intent);
                    }
                }
            }
        }
        Ok(first)
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({ "verb": self.verb, "param": self.param, "key": self.key })
    }

    /// The words of an intent in a refusal.
    pub fn describe(&self) -> String {
        match (&self.param, self.key) {
            (Some(param), Some(key)) => format!("{} on `{param}` at frame {key}", self.verb),
            (Some(param), None) => format!("{} on `{param}`", self.verb),
            (None, _) => self.verb.to_string(),
        }
    }
}
