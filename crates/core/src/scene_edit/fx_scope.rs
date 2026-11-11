//! The scope op of an effect clip (`bundle.fx`, F062, BL082.3): `set_scope`, and the refusal a clip
//! that does not belong to its scope is given, shared with `add_clip`.
//!
//! The same atomic batch as `fx.rs`, on the `scope` of a clip, and it lives beside it like the key
//! ops do (`fx_keys`). The rule is not here: whether a shot exists and holds a clip is
//! [`fx::scope_fault`], the one place that judges it for the importer and for every edit. This
//! file only writes the op's answer in the document and says the refusal with the facts an
//! adapter can act on.

use serde_json::{Value, json};

use crate::directed_scene::fx::{self, FxScope};

use super::error::EditError;
use super::fx::{
    EditSpan, FxChange, clip_object, clip_ref, clips_of, frame_of, mismatch, text_of, tracks,
    tracks_mut,
};

/// Moves a clip to the output or to a shot (F062). The shot must exist and hold the whole clip
/// as it is now: a clip that does not fit is refused with the shot's legal interval, never
/// clamped (retime it first, in the same batch if both are meant). The same scope is a no-op that
/// leaves the document as written, a hand-written `{ "kind": "output" }` included; going back to
/// the output leaves the `scope` key out, the canonical spelling.
pub(super) fn set_scope(
    work: &mut Value,
    clip: &str,
    scope: &FxScope,
    index: usize,
    span: EditSpan<'_>,
) -> Result<FxChange, EditError> {
    let (track_slot, clip_slot) = clip_ref(work, clip)?;
    let (start, duration) = {
        let found = &clips_of(&tracks(work)[track_slot])[clip_slot];
        (
            frame_of(found, "startFrame"),
            frame_of(found, "durationFrames"),
        )
    };
    let tracks = tracks_mut(work)?;
    let track_id = text_of(&tracks[track_slot], "id").to_string();
    let target = clip_object(&mut tracks[track_slot], clip_slot)?;
    // No key, or a hand-written `{ "kind": "output" }`, is the output. A scope the importer would
    // not have accepted never reaches here: it is the document that is not what it should be.
    let before = match target.get("scope") {
        None => FxScope::Output,
        Some(raw) => FxScope::from_op_json(raw).map_err(|(_, message)| {
            mismatch(format!(
                "the scope of the clip `{clip}` is unreadable: {message}"
            ))
        })?,
    };
    let noop = before == *scope;
    if !noop {
        if let Some(fault) = fx::scope_fault(scope, clip, start, duration, span.shots) {
            return Err(scope_refusal(fault, scope));
        }
        match scope.to_document() {
            Some(document) => {
                target.insert("scope".to_string(), document);
            }
            None => {
                target.remove("scope");
            }
        }
    }
    Ok(FxChange {
        index,
        warnings: Vec::new(),
        action: "set_scope",
        track: track_id,
        clip: clip.to_string(),
        target: Some("scope".to_string()),
        before: Some(json!(before.to_text())),
        after: Some(json!(scope.to_text())),
        noop,
    })
}

/// The refusal of a clip that does not belong to the scope it was given: the importer's own code
/// and words, the path of the op's field, the shot that does not exist as the missing id, and the
/// facts an adapter can act on (the shot's legal interval, the ids there are) in `details`.
pub(super) fn scope_refusal(fault: fx::ScopeFault, scope: &FxScope) -> EditError {
    let error = EditError::new(fault.code, fault.message)
        .with_path("scope")
        .with_details(fault.details);
    match scope.shot_id() {
        Some(shot) if fault.code == fx::UNKNOWN_SHOT => error.with_id(shot),
        _ => error,
    }
}
