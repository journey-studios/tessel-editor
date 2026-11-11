//! Where an effect clip lies (F062): on the finished picture (`output`) or on one shot of the
//! edit (`shot`), the pilot's plan scope.
//!
//! The scope belongs to the clip. In the document it is `scope: { "kind": "shot", "shotId": … }`;
//! the scope `output` is the **canonical spelling left out** (the pilot never writes it, and its
//! `setScope` back to `output` removes the key), while a hand-written `{ "kind": "output" }` is
//! still read and, as the document is the lossless tree, kept as written until an op touches the
//! clip. In reads and ops the scope is **text**: `output`, `shot:<id>`; the id is everything after
//! the first `:`, so an id that holds a `:` round-trips.
//!
//! A clip with scope `shot` lies **entirely** inside `[shot.start, shot.end)` (the pilot's
//! `FX_SCOPE_MISMATCH`); an id the edit does not have is `UNKNOWN_SHOT`. [`scope_fault`] is the one
//! rule: the importer judges a document with it and the edit ops judge their candidate with it, so
//! the two cannot drift. Scope `node` is still refused (`FX_SCOPE_NOT_YET`).
//!
//! The composition is in **stages**, never in track order alone: the passes of the shot's own
//! picture (the *deck*) come first, then the passes of the output, each stage in track order
//! (see [`FxStage`] and `Fx::resolve`).

use serde_json::{Value, json};

use super::{FX_SCOPE_MISMATCH, FX_SCOPE_NOT_YET, UNKNOWN_SHOT};
use crate::directed_scene::{INVALID_VALUE, Shot};

/// The text of the scope `output`.
pub const SCOPE_OUTPUT: &str = "output";
/// The text of a shot scope is this prefix and the shot's id.
pub const SCOPE_SHOT_PREFIX: &str = "shot:";

/// What the scope text or JSON of an op got wrong: a code (`INVALID_VALUE`, or `FX_SCOPE_NOT_YET`
/// for `node`) and a message naming what was expected.
pub type ScopeSyntax = (&'static str, String);

/// Where a clip is laid. Extensible: scope `node` joins here when a node target exists.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FxScope {
    /// The finished picture of the frame (what every clip did before F062).
    #[default]
    Output,
    /// The picture of one shot, before the output passes. The clip lies inside the shot.
    Shot(String),
}

/// The two stages of the composition, in the order they are drawn: the deck (the shot's own
/// picture) and the output. A transition (P4b) will sit between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FxStage {
    Deck,
    Output,
}

impl FxStage {
    pub fn name(self) -> &'static str {
        match self {
            Self::Deck => "deck",
            Self::Output => "output",
        }
    }
}

impl FxScope {
    /// The stage the passes of this scope are drawn in.
    pub fn stage(&self) -> FxStage {
        match self {
            Self::Output => FxStage::Output,
            Self::Shot(_) => FxStage::Deck,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Output => SCOPE_OUTPUT,
            Self::Shot(_) => "shot",
        }
    }

    pub fn is_output(&self) -> bool {
        matches!(self, Self::Output)
    }

    /// The shot a `shot` scope names.
    pub fn shot_id(&self) -> Option<&str> {
        match self {
            Self::Output => None,
            Self::Shot(id) => Some(id),
        }
    }

    /// The spelling of reads and ops: `output`, `shot:<id>`.
    pub fn to_text(&self) -> String {
        match self {
            Self::Output => SCOPE_OUTPUT.to_string(),
            Self::Shot(id) => format!("{SCOPE_SHOT_PREFIX}{id}"),
        }
    }

    /// The spelling of the plan's passes: `{ "kind": "output" }`, `{ "kind": "shot", "shotId" }`.
    pub fn to_wire(&self) -> Value {
        match self {
            Self::Output => json!({ "kind": "output" }),
            Self::Shot(id) => json!({ "kind": "shot", "shotId": id }),
        }
    }

    /// What a clip's document holds for this scope: nothing for the output (the canonical
    /// spelling leaves the key out), the object for a shot.
    pub fn to_document(&self) -> Option<Value> {
        match self {
            Self::Output => None,
            Self::Shot(id) => Some(json!({ "kind": "shot", "shotId": id })),
        }
    }

    /// Reads the text of a scope. The id of a shot is everything after the first `:` and is not
    /// empty. `node` (and `node:<id>`) is a scope this build does not draw yet.
    pub fn parse_text(text: &str) -> Result<Self, ScopeSyntax> {
        if text == SCOPE_OUTPUT {
            return Ok(Self::Output);
        }
        if let Some(id) = text.strip_prefix(SCOPE_SHOT_PREFIX) {
            if id.is_empty() {
                return Err((
                    INVALID_VALUE,
                    "a shot scope names its shot: `shot:<id>`".to_string(),
                ));
            }
            return Ok(Self::Shot(id.to_string()));
        }
        if text == "node" || text.starts_with("node:") {
            return Err((FX_SCOPE_NOT_YET, node_not_yet()));
        }
        Err((
            INVALID_VALUE,
            format!("`{text}` is not a scope; use `output` or `shot:<id>`"),
        ))
    }

    /// Reads a scope an op carries: the text (`output`, `shot:<id>`) or the document's own
    /// object (`{ "kind": "shot", "shotId": "<id>" }`).
    pub fn from_op_json(value: &Value) -> Result<Self, ScopeSyntax> {
        match value {
            Value::String(text) => Self::parse_text(text),
            Value::Object(map) => match map.get("kind").and_then(Value::as_str) {
                Some(SCOPE_OUTPUT) => Ok(Self::Output),
                Some("shot") => match map.get("shotId").and_then(Value::as_str) {
                    Some(id) if !id.is_empty() => Ok(Self::Shot(id.to_string())),
                    _ => Err((
                        INVALID_VALUE,
                        "a shot scope names its shot: { \"kind\": \"shot\", \"shotId\": \"<id>\" }"
                            .to_string(),
                    )),
                },
                Some("node") => Err((FX_SCOPE_NOT_YET, node_not_yet())),
                _ => Err((
                    INVALID_VALUE,
                    "expected an object whose `kind` is `output` or `shot`".to_string(),
                )),
            },
            _ => Err((
                INVALID_VALUE,
                "a scope is the text `output` or `shot:<id>`".to_string(),
            )),
        }
    }
}

fn node_not_yet() -> String {
    "scope `node` is not drawn yet; use scope output or scope shot".to_string()
}

/// Why a clip does not belong to its scope: a stable code, the words, and the facts an adapter
/// may act on (the legal interval of the shot, the shots there are).
#[derive(Debug, Clone, PartialEq)]
pub struct ScopeFault {
    /// `UNKNOWN_SHOT` or `FX_SCOPE_MISMATCH`.
    pub code: &'static str,
    pub message: String,
    pub details: Value,
}

/// The shot an edit frame plays in: the first whose `[start, end)` holds it. The importer
/// guarantees the shots tile the edit, so there is one; a frame outside the edit has none.
pub fn shot_at(shots: &[Shot], edit_frame: i64) -> Option<&Shot> {
    let frame = u64::try_from(edit_frame).ok()?;
    shots
        .iter()
        .find(|shot| shot.start_frame <= frame && frame < shot.end_frame_exclusive)
}

/// A shot as the effect reads list it (`id`, `start_frame`, `end_frame_exclusive`, `intent`): where
/// it lies in the edit, so a clip with scope `shot` can be placed inside it.
pub fn shot_json(shot: &Shot) -> Value {
    json!({
        "id": shot.id,
        "start_frame": shot.start_frame,
        "end_frame_exclusive": shot.end_frame_exclusive,
        "intent": shot.intent,
    })
}

/// The shots of the edit, in document order, as [`shot_json`].
pub fn shots_json(shots: &[Shot]) -> Value {
    Value::Array(shots.iter().map(shot_json).collect())
}

/// The one rule of the scope against the shots of the edit, for a clip that runs
/// `start_frame..start_frame + duration_frames`: the shot must exist (`UNKNOWN_SHOT`) and hold the
/// whole clip (`FX_SCOPE_MISMATCH`: the clip starts before the shot or ends after it). The output
/// scope has no rule here. `None` means the clip belongs to its scope.
pub fn scope_fault(
    scope: &FxScope,
    clip: &str,
    start_frame: i64,
    duration_frames: i64,
    shots: &[Shot],
) -> Option<ScopeFault> {
    let FxScope::Shot(id) = scope else {
        return None;
    };
    let Some(shot) = shots.iter().find(|shot| &shot.id == id) else {
        let listed: Vec<&str> = shots.iter().map(|shot| shot.id.as_str()).collect();
        return Some(ScopeFault {
            code: UNKNOWN_SHOT,
            message: format!(
                "Unknown shot `{id}`. Shots: {}.",
                if listed.is_empty() {
                    "none".to_string()
                } else {
                    listed.join(", ")
                }
            ),
            details: json!({ "shot": id, "shots": listed }),
        });
    };
    let end = start_frame.saturating_add(duration_frames);
    // The shot's limits are unsigned frames of the edit; a clip is judged in signed frames, so a
    // negative start or an end past `i64` is outside either way.
    let (first, last) = (
        i64::try_from(shot.start_frame).unwrap_or(i64::MAX),
        i64::try_from(shot.end_frame_exclusive).unwrap_or(i64::MAX),
    );
    if start_frame >= first && end <= last {
        return None;
    }
    Some(ScopeFault {
        code: FX_SCOPE_MISMATCH,
        message: format!(
            "Clip {clip} (frames {start_frame} to {end}) does not fit shot {id} (frames {first} to {last}); \
             a clip with scope shot stays inside its shot: keep it between frames {first} and {last}, \
             or use scope output"
        ),
        details: json!({
            "shot": id, "start_frame": first, "end_frame_exclusive": last,
            "clip": clip, "clip_start_frame": start_frame, "clip_end_frame_exclusive": end,
        }),
    })
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use super::*;

    #[test]
    fn the_text_of_a_scope_round_trips_and_the_id_is_everything_after_the_first_colon() {
        assert_eq!(FxScope::parse_text("output"), Ok(FxScope::Output));
        assert_eq!(
            FxScope::parse_text("shot:s1"),
            Ok(FxScope::Shot("s1".into()))
        );
        // A shot id may hold a `:` (the importer accepts any non-empty id): it is not cut.
        let scope = FxScope::parse_text("shot:act:1:shot").unwrap();
        assert_eq!(scope.shot_id(), Some("act:1:shot"));
        assert_eq!(scope.to_text(), "shot:act:1:shot");
        assert_eq!(FxScope::parse_text(&scope.to_text()), Ok(scope));
        assert_eq!(FxScope::Output.to_text(), "output");
    }

    #[test]
    fn a_scope_that_is_not_drawn_or_not_a_scope_is_refused_with_its_code() {
        for node in ["node", "node:n1"] {
            assert_eq!(FxScope::parse_text(node).unwrap_err().0, FX_SCOPE_NOT_YET);
        }
        for bad in ["", "shot:", "Shot:s1", "frame", "shot", "output:x"] {
            assert_eq!(
                FxScope::parse_text(bad).unwrap_err().0,
                INVALID_VALUE,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_scope_is_read_from_the_document_object_too() {
        assert_eq!(
            FxScope::from_op_json(&json!({ "kind": "shot", "shotId": "s1" })),
            Ok(FxScope::Shot("s1".into()))
        );
        assert_eq!(
            FxScope::from_op_json(&json!({ "kind": "output" })),
            Ok(FxScope::Output)
        );
        assert_eq!(
            FxScope::from_op_json(&json!({ "kind": "node", "nodeId": "n" }))
                .unwrap_err()
                .0,
            FX_SCOPE_NOT_YET
        );
        for bad in [
            json!({ "kind": "shot" }),
            json!({ "kind": "shot", "shotId": "" }),
            json!({ "kind": "shot", "shotId": 7 }),
            json!({ "kind": "frame" }),
            json!(7),
            json!(null),
        ] {
            assert_eq!(
                FxScope::from_op_json(&bad).unwrap_err().0,
                INVALID_VALUE,
                "{bad}"
            );
        }
    }

    #[test]
    fn the_output_is_left_out_of_the_document_and_a_shot_is_an_object() {
        assert_eq!(FxScope::Output.to_document(), None);
        assert_eq!(
            FxScope::Shot("s1".into()).to_document(),
            Some(json!({ "kind": "shot", "shotId": "s1" }))
        );
        // The plan always says where a pass is.
        assert_eq!(FxScope::Output.to_wire(), json!({ "kind": "output" }));
        assert_eq!(
            FxScope::Shot("s1".into()).to_wire(),
            json!({ "kind": "shot", "shotId": "s1" })
        );
    }

    #[test]
    fn the_deck_is_drawn_before_the_output() {
        assert!(FxStage::Deck < FxStage::Output);
        assert_eq!(FxScope::Shot("s1".into()).stage(), FxStage::Deck);
        assert_eq!(FxScope::Output.stage(), FxStage::Output);
        assert_eq!(
            (FxStage::Deck.name(), FxStage::Output.name()),
            ("deck", "output")
        );
    }

    /// The shots of the pilot's `fx-test-bundle.ts` (blob `ec766844…`, `ORIGIN_NOT_EXECUTED`: the
    /// intervals are copied by hand): s1 `[0, 48)`, s2 `[48, 96)`, s3 `[96, 144)`.
    fn pilot_shots() -> Vec<Shot> {
        [("s1", 0, 48), ("s2", 48, 96), ("s3", 96, 144)]
            .into_iter()
            .map(|(id, start, end)| Shot {
                id: id.to_string(),
                scene_instance_id: "i1".to_string(),
                start_frame: start,
                end_frame_exclusive: end,
                instance_start_frame: 0,
                camera_clip_id: "cam".to_string(),
                camera_in_frame: 0,
                transition_into: None,
                intent: String::new(),
            })
            .collect()
    }

    fn fault(shot: &str, start: i64, duration: i64) -> Option<ScopeFault> {
        scope_fault(
            &FxScope::Shot(shot.to_string()),
            "c1",
            start,
            duration,
            &pilot_shots(),
        )
    }

    #[test]
    fn a_clip_inside_its_shot_belongs_to_it_up_to_the_last_frame() {
        // Pilot `fx-commands-clips.spec.ts` cases (blob `0b6bb7ba…`): 0 + 24 in s1 is fine, and a
        // clip of the whole shot (start 0, duration 48) is valid.
        assert_eq!(fault("s1", 0, 24), None);
        assert_eq!(fault("s1", 0, 48), None);
        assert_eq!(fault("s2", 48, 48), None);
        assert_eq!(fault("s3", 143, 1), None);
        // The output has no rule against the shots, whatever the clip.
        assert_eq!(
            scope_fault(&FxScope::Output, "c1", -5, 1_000, &pilot_shots()),
            None
        );
    }

    #[test]
    fn a_clip_that_leaves_its_shot_by_one_frame_either_side_is_a_mismatch() {
        // Pilot cases 40 + 20 and 40 + 24 in s1 (s1 ends at 48).
        for (start, duration) in [(40, 20), (40, 24), (0, 49)] {
            let fault = fault("s1", start, duration).expect("past the end of s1");
            assert_eq!(fault.code, FX_SCOPE_MISMATCH, "{start} + {duration}");
        }
        // One frame before the start of s2, and one frame past its end.
        assert_eq!(fault("s2", 47, 2).unwrap().code, FX_SCOPE_MISMATCH);
        assert_eq!(fault("s2", 95, 2).unwrap().code, FX_SCOPE_MISMATCH);
        // A clip that sits in another shot is outside this one.
        assert_eq!(fault("s1", 48, 1).unwrap().code, FX_SCOPE_MISMATCH);
    }

    #[test]
    fn a_mismatch_carries_the_legal_interval_in_its_details() {
        let fault = fault("s1", 40, 20).unwrap();
        assert_eq!(
            fault.details,
            json!({
                "shot": "s1", "start_frame": 0, "end_frame_exclusive": 48,
                "clip": "c1", "clip_start_frame": 40, "clip_end_frame_exclusive": 60,
            })
        );
        assert!(
            fault.message.contains("between frames 0 and 48"),
            "{}",
            fault.message
        );
    }

    #[test]
    fn a_shot_the_edit_does_not_have_is_unknown_and_the_hint_lists_the_shots() {
        // Pilot `fx-commands-params.spec.ts` (blob `7094de47…`): `ghost` is `UNKNOWN_SHOT` with the
        // hint "Shots: s1, s2, s3.".
        let fault = fault("ghost", 0, 24).unwrap();
        assert_eq!(fault.code, UNKNOWN_SHOT);
        assert!(
            fault.message.contains("Shots: s1, s2, s3."),
            "{}",
            fault.message
        );
        assert_eq!(
            fault.details,
            json!({ "shot": "ghost", "shots": ["s1", "s2", "s3"] })
        );
        // No shot at all: the hint says so instead of an empty list.
        let none = scope_fault(&FxScope::Shot("s1".into()), "c1", 0, 1, &[]).unwrap();
        assert!(none.message.contains("Shots: none."), "{}", none.message);
    }

    #[test]
    fn a_clip_far_outside_the_signed_range_is_a_mismatch_not_a_panic() {
        assert_eq!(fault("s1", i64::MAX, 10).unwrap().code, FX_SCOPE_MISMATCH);
        assert_eq!(fault("s1", -1, 5).unwrap().code, FX_SCOPE_MISMATCH);
        assert_eq!(fault("s1", 0, i64::MAX).unwrap().code, FX_SCOPE_MISMATCH);
    }

    #[test]
    fn the_shot_of_a_frame_changes_exactly_on_the_boundary() {
        let shots = pilot_shots();
        let at = |frame| shot_at(&shots, frame).map(|shot| shot.id.as_str());
        assert_eq!(at(0), Some("s1"));
        assert_eq!(at(47), Some("s1"));
        assert_eq!(at(48), Some("s2"));
        assert_eq!(at(95), Some("s2"));
        assert_eq!(at(96), Some("s3"));
        assert_eq!(at(143), Some("s3"));
        // Outside the edit there is none.
        assert_eq!(at(144), None);
        assert_eq!(at(-1), None);
    }
}
