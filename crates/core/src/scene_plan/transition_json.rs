//! The wire form of `transition` (F064, P4b S1b), written and read by hand like the rest of the
//! plan, and the hostile-input ceilings of a plan that carries one.
//!
//! `other` is a whole plan, read by the very same reader (`ScenePlan::read_deck`) with a flag that
//! refuses a `transition` inside it BEFORE reading it: depth is 1, so there is no recursion to
//! bound. The ceilings are fixed by the S0 contract ([`MAX_TRANSITION_PLAN_BYTES`],
//! [`MAX_TRANSITION_PLAN_ITEMS`]) and apply only to a plan that has a `transition`; a plan without
//! one reads exactly as before.
//!
//! * Items are counted from the array lengths of the two decks, before any item is read.
//! * Bytes are the length of the TEXT given to `from_json_str`. When the text is over the ceiling
//!   and has a top-level `transition` key it is refused before it is parsed (a cheap lexical scan,
//!   no allocation); a text that hides the key behind an escape is refused right after parsing.
//!   A tree handed to `from_json` has no text: its items and depth are bounded, its bytes are the
//!   caller's.

use serde_json::{Value, json};

use crate::directed_scene::fx::{DeckSide, FxEasing, FxValue, ParamKind, transition_effect};

use super::PlanError;
use super::json::Obj;
use super::model::ScenePlan;
use super::temporal::{read_alignment, read_timing, timing_json};
use super::transition::{MAX_TRANSITION_PLAN_BYTES, MAX_TRANSITION_PLAN_ITEMS, PlanTransition};

/// The arrays of a deck whose lengths make its items.
const ITEM_ARRAYS: [&str; 6] = ["nodes", "layers", "splats", "fx", "resources", "lights"];

pub(super) fn to_json(transition: &PlanTransition) -> Value {
    json!({
        "id": transition.id,
        "effect": transition.effect,
        "fromShotId": transition.from_shot_id,
        "toShotId": transition.to_shot_id,
        "primary": transition.primary.name(),
        "startFrame": transition.start_frame,
        "durationFrames": transition.duration_frames,
        "alignment": transition.alignment.name(),
        "timing": timing_json(&transition.timing),
        "frameInWindow": transition.frame_in_window,
        "easing": transition.easing.name(),
        "progress": transition.progress,
        "amount": transition.amount,
        // An ordered list of pairs: an object would be reordered by the build's JSON features.
        "params": transition
            .params
            .iter()
            .map(|(name, value)| json!([name, value.to_json()]))
            .collect::<Vec<_>>(),
        "heldInstance": transition.held_instance,
        "heldCamera": transition.held_camera,
        "other": transition.other.to_json(),
    })
}

fn param_value(
    at: &str,
    spec: &crate::directed_scene::fx::ParamSpec,
    raw: &Value,
) -> Result<FxValue, PlanError> {
    match &spec.kind {
        ParamKind::Number { .. } => raw
            .as_f64()
            .map(FxValue::Number)
            .ok_or_else(|| PlanError::invalid(at, "expected a number")),
        ParamKind::Choice { options, .. } => raw
            .as_str()
            .and_then(|chosen| options.iter().copied().find(|option| *option == chosen))
            .map(FxValue::Choice)
            .ok_or_else(|| PlanError::invalid(at, "not one of the options of the parameter")),
        _ => Err(PlanError::invalid(
            at,
            "a transition has no parameter of this kind",
        )),
    }
}

/// `params`: `[[name, value], ...]`, one pair per parameter of the effect, in catalog order.
fn read_params(
    transition: &Obj<'_>,
    spec: &crate::directed_scene::fx::EffectSpec,
) -> Result<Vec<(String, FxValue)>, PlanError> {
    let items = transition.array("params")?;
    if items.len() != spec.params.len() {
        return Err(transition.fail(
            "params",
            "must hold exactly the parameters of the effect, in catalog order",
        ));
    }
    let mut params = Vec::with_capacity(items.len());
    for (position, (item, param)) in items.iter().zip(spec.params).enumerate() {
        let at = format!("{}/{position}", transition.path("params"));
        let pair = item
            .as_array()
            .filter(|pair| pair.len() == 2)
            .ok_or_else(|| PlanError::invalid(&at, "expected a [name, value] pair"))?;
        if pair[0].as_str() != Some(param.id) {
            return Err(PlanError::invalid(
                &at,
                "must hold exactly the parameters of the effect, in catalog order",
            ));
        }
        params.push((param.id.to_string(), param_value(&at, param, &pair[1])?));
    }
    Ok(params)
}

pub(super) fn read(transition: &Obj<'_>) -> Result<PlanTransition, PlanError> {
    transition.only(&[
        "id",
        "effect",
        "fromShotId",
        "toShotId",
        "primary",
        "startFrame",
        "durationFrames",
        "alignment",
        "timing",
        "frameInWindow",
        "easing",
        "progress",
        "amount",
        "params",
        "heldInstance",
        "heldCamera",
        "other",
    ])?;
    // The old experimental wire has neither: refused at the first, with no default.
    let alignment = read_alignment(transition)?;
    let timing = read_timing(&transition.object("timing")?)?;
    let effect = transition.text("effect")?;
    let spec = transition_effect(&effect)
        .ok_or_else(|| transition.fail("effect", "not a transition effect"))?;
    let primary = DeckSide::parse(transition.text_ref("primary")?)
        .ok_or_else(|| transition.fail("primary", "expected `from` or `to`"))?;
    let easing = FxEasing::parse(transition.text_ref("easing")?)
        .ok_or_else(|| transition.fail("easing", "not an easing of the effects"))?;
    Ok(PlanTransition {
        id: transition.text("id")?,
        from_shot_id: transition.text("fromShotId")?,
        to_shot_id: transition.text("toShotId")?,
        primary,
        start_frame: transition.integer("startFrame")?,
        duration_frames: transition.integer("durationFrames")?,
        alignment,
        timing,
        frame_in_window: transition.integer("frameInWindow")?,
        easing,
        progress: transition.number("progress")?,
        amount: transition.number("amount")?,
        params: read_params(transition, spec)?,
        held_instance: transition.boolean("heldInstance")?,
        held_camera: transition.boolean("heldCamera")?,
        other: ScenePlan::read_deck(transition.field("other")?, &transition.path("other"), true)?,
        effect,
    })
}

fn refusal(message: String) -> PlanError {
    PlanError::invalid("/transition", message)
}

/// The item ceiling, from the array lengths of the two decks and before any item is read. A tree
/// with no `transition` (or not an object) is not this module's business.
pub(super) fn precheck_items(root: &Value) -> Result<(), PlanError> {
    let Some(transition) = root.get("transition") else {
        return Ok(());
    };
    let items = |deck: &Value| -> usize {
        ITEM_ARRAYS
            .iter()
            .map(|key| deck.get(key).and_then(Value::as_array).map_or(0, Vec::len))
            .sum()
    };
    let total = items(root).saturating_add(transition.get("other").map_or(0, items));
    if total > MAX_TRANSITION_PLAN_ITEMS {
        return Err(refusal(format!(
            "the two decks hold {total} items, more than the {MAX_TRANSITION_PLAN_ITEMS} a plan with a transition may have"
        )));
    }
    Ok(())
}

fn too_long(length: usize) -> PlanError {
    refusal(format!(
        "the plan is {length} bytes, more than the {MAX_TRANSITION_PLAN_BYTES} a plan with a transition may have"
    ))
}

/// Refuses an over-long text that has a top-level `transition` before it is parsed.
pub(super) fn precheck_text(text: &str) -> Result<(), PlanError> {
    if text.len() > MAX_TRANSITION_PLAN_BYTES && has_top_level_key(text.as_bytes(), b"transition") {
        return Err(too_long(text.len()));
    }
    Ok(())
}

/// The same ceiling, exactly, once the text is parsed (a key spelled with an escape).
pub(super) fn check_text(text: &str, root: &Value) -> Result<(), PlanError> {
    if text.len() > MAX_TRANSITION_PLAN_BYTES && root.get("transition").is_some() {
        return Err(too_long(text.len()));
    }
    Ok(())
}

/// Whether the text has `key` as a member of its top-level object, found by a lexical scan: depth
/// by brackets outside strings, a string is a key when a `:` follows it. No allocation, one pass.
fn has_top_level_key(bytes: &[u8], key: &[u8]) -> bool {
    let mut depth = 0usize;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'{' | b'[' => depth += 1,
            b'}' | b']' => depth = depth.saturating_sub(1),
            b'"' => {
                let start = index + 1;
                index = start;
                while index < bytes.len() && bytes[index] != b'"' {
                    if bytes[index] == b'\\' {
                        index += 1;
                    }
                    index += 1;
                }
                let end = index.min(bytes.len());
                if depth == 1 && bytes.get(start..end) == Some(key) {
                    let next = bytes[(end + 1).min(bytes.len())..]
                        .iter()
                        .find(|byte| !byte.is_ascii_whitespace());
                    if next == Some(&b':') {
                        return true;
                    }
                }
            }
            _ => {}
        }
        index += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use super::has_top_level_key;

    #[test]
    fn the_scan_finds_a_top_level_key_and_nothing_else() {
        let key = b"transition";
        assert!(has_top_level_key(br#"{"a":1,"transition":{}}"#, key));
        assert!(has_top_level_key(b"{ \"transition\" \n : 1 }", key));
        // Nested, a value, an array element, a longer name, an escaped quote before it.
        assert!(!has_top_level_key(br#"{"a":{"transition":1}}"#, key));
        assert!(!has_top_level_key(br#"{"a":"transition"}"#, key));
        assert!(!has_top_level_key(br#"["transition"]"#, key));
        assert!(!has_top_level_key(br#"{"transitions":1}"#, key));
        assert!(!has_top_level_key(br#"{"a":"x\"transition","b":1}"#, key));
        assert!(!has_top_level_key(b"", key));
        // Truncated text never panics.
        assert!(!has_top_level_key(br#"{"transition"#, key));
        assert!(!has_top_level_key(br#"{"a":"\"#, key));
    }
}
