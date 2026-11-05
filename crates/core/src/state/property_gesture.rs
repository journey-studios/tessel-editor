//! One slider drag as ONE undoable edit (BL062; F005 and F024).
//!
//! A slider reports a stream of values while the pointer or a key moves it. Recording every
//! value as its own edit (`SetProperty` through `execute`) filled the 200-entry history with
//! micro-steps and made a single drag need dozens of undos. A gesture keeps the live preview
//! (every value is applied to the model at once, so the monitor follows) but records the
//! history entry only when the gesture ends:
//!
//! - `Begin` locks the clip and the property and remembers the value to return to, plus a
//!   snapshot of the model. Nothing is recorded and redo is not touched.
//! - `Update` applies a value with exactly the rules of `SetProperty` (type, clamp, rounding,
//!   `requires`). A rejected value leaves the model and the gesture as they were.
//! - `Commit` records ONE entry from the original model to the final one, or none when the
//!   value came back to where it started. It is the only point that clears redo.
//! - `Cancel` puts the original value back and records nothing.
//!
//! WHY the rules live here and not in the desktop: the CLI/JSONL (`property.gesture.*`) and a
//! Rust caller need the same "many values, one undo" step as a drag, and a second stack in the
//! window would let the three surfaces disagree (see `docs/features/history.md`).
//!
//! Policy for everything that can happen while a gesture is open, so no path leaves a preview
//! value without an owner:
//!
//! - Any other *edit* (a command with a history label, a sequence switch, a batch
//!   [`transaction`](EditorState::transaction), `undo`, `redo`) first **settles** the gesture
//!   as if it had been released: the value the user sees stays, as one entry, and the new edit
//!   or undo then acts on the result. A gesture is never left half-applied across an edit.
//! - Presentation commands (selection, playhead, zoom, tabs) do not end it. The gesture stays
//!   locked to its clip, so a value can never land on another clip because the selection moved.
//! - Opening or creating a project replaces the whole state, gesture included; nothing is
//!   recorded in the new project.
//! - If the document moved underneath the gesture through a path that bypassed the settle
//!   rule (the project revision or the active sequence changed, or the clip is gone), the
//!   gesture is dropped with `stale_gesture`. When only our own preview changed the document it
//!   is rolled back to the original value; when something else changed it, the model is left
//!   alone because we cannot tell whose value is whose.
//!
//! The project revision is bumped by every preview value that changed something (and by a
//! cancel that rolled one back), so "dirty", autosave and in-flight job checks see the
//! preview as the unsaved edit it is. A commit does not bump it again.

use serde_json::{Value, json};

use super::history::Snapshot;
use super::inspector::{PropertyDef, PropertyKind, Value as PropertyValue, property_def};
use super::timeline::ClipKind;
use super::{Command, CommandError, EditorState, Raw};
use crate::id::Id;

/// Undo-list name of a settled gesture: the same as a discrete `SetProperty`, so a drag and a
/// typed value are indistinguishable in the history (a test pins the equality).
const LABEL: &str = "Set property";

/// The four phases of a gesture, as a command.
#[derive(Debug, Clone, PartialEq)]
pub enum GestureCommand {
    /// Locks `clip` (the selected clip when `None`) and the slider `property`.
    Begin {
        clip: Option<String>,
        property: String,
    },
    /// Applies `value` to the locked target. `gesture` is the ID `Begin` returned; `None`
    /// means "the open one" (what a script that runs one gesture at a time sends).
    Update { gesture: Option<u64>, value: Raw },
    /// Ends the gesture, keeping the preview as one undo entry (none if nothing changed).
    Commit { gesture: Option<u64> },
    /// Ends the gesture and puts the original value back, with no entry.
    Cancel { gesture: Option<u64> },
}

/// The gesture that is open, as adapters see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GestureInfo {
    pub id: u64,
    pub clip: Id,
    pub property: &'static str,
}

/// State of the open gesture. Lives in [`History`](super::history::History) because its
/// lifetime is the history's: it ends in an entry or in nothing, and clearing the history
/// ends it.
#[derive(Debug, Clone)]
pub(super) struct OpenGesture {
    pub(super) id: u64,
    clip: Id,
    kind: ClipKind,
    definition: &'static PropertyDef,
    /// Active sequence at `Begin`: a gesture belongs to one sequence's clip.
    sequence: Id,
    /// Effective value (override or default) the gesture returns to on cancel.
    original: PropertyValue,
    /// The model before the first preview value, kept as the undo entry on commit.
    before: Snapshot,
    /// Some preview value changed the model, so the revision moved and a cancel must revert.
    touched: bool,
    /// Project revision after the last change this gesture made. Anything else moving it
    /// means the document changed behind the gesture.
    revision: u64,
}

impl OpenGesture {
    pub(super) fn info(&self) -> GestureInfo {
        GestureInfo {
            id: self.id,
            clip: self.clip,
            property: self.definition.id,
        }
    }

    fn target(&self) -> String {
        format!("property:{}", self.definition.id)
    }
}

impl Command {
    /// Edits that must not run on top of an unsettled gesture. Presentation commands are
    /// excluded on purpose (see the module docs); `Undo`/`Redo` settle inside themselves so
    /// the direct library calls follow the same rule.
    pub(super) fn settles_property_gesture(&self) -> bool {
        self.history_label().is_some() || matches!(self, Self::ActivateSequence { .. })
    }
}

fn no_gesture() -> CommandError {
    CommandError::new("no_gesture", "No property gesture is open")
}

impl EditorState {
    /// The open slider gesture, if any.
    pub fn open_property_gesture(&self) -> Option<GestureInfo> {
        self.history.gesture.as_ref().map(OpenGesture::info)
    }

    pub(super) fn run_property_gesture(
        &mut self,
        command: GestureCommand,
    ) -> Result<Value, CommandError> {
        match command {
            GestureCommand::Begin { clip, property } => {
                self.begin_property_gesture(clip.as_deref(), &property)
            }
            GestureCommand::Update { gesture, value } => {
                self.update_property_gesture(gesture, &value)
            }
            GestureCommand::Commit { gesture } => self.commit_property_gesture(gesture),
            GestureCommand::Cancel { gesture } => self.cancel_property_gesture(gesture),
        }
    }

    /// Starts a gesture on the slider `property` of `clip` (the selected clip when `None`).
    /// A gesture that is already open is settled first, after the new request has validated,
    /// so a rejected `Begin` does not disturb it.
    pub fn begin_property_gesture(
        &mut self,
        clip: Option<&str>,
        property: &str,
    ) -> Result<Value, CommandError> {
        let (clip, kind) = match clip {
            Some(id) => self.clip_ref(id)?,
            None => self.require_clip()?,
        };
        let definition = property_def(property)
            .ok_or_else(|| CommandError::unknown_element("property", property))?;
        let target = format!("property:{property}");
        if !matches!(definition.kind, PropertyKind::Slider { .. }) {
            return Err(CommandError::new(
                "not_applicable",
                format!("`{property}` is not a slider: set it with a discrete command"),
            )
            .target(target));
        }
        let original = self.inspector.value(clip, definition);
        let PropertyValue::Number(current) = original else {
            return Err(CommandError::new(
                "not_applicable",
                format!("`{property}` has no numeric value"),
            )
            .target(target));
        };
        // WHY a dry run on a copy: it asks `set_property` itself whether the property applies
        // to this clip and is enabled (`requires`), so the gesture cannot disagree with a
        // discrete set, and the copy keeps a document value that is not on the rounding grid
        // (a hand-edited file) exactly as it was.
        self.inspector
            .clone()
            .set_property(clip, kind, property, &Raw::Number(current))?;

        self.settle_property_gesture();
        self.history.next_gesture += 1;
        let id = self.history.next_gesture;
        self.history.gesture = Some(OpenGesture {
            id,
            clip,
            kind,
            definition,
            sequence: self.timeline.id,
            original,
            before: self.snapshot(),
            touched: false,
            revision: self.project.revision,
        });
        Ok(
            json!({ "target": target, "action": "gesture.begin", "gesture": id,
            "clip": format!("clip:{clip}"), "value": original.to_json(), "changed": false }),
        )
    }

    /// Previews `raw` on the locked target: same validation, clamp and rounding as
    /// `SetProperty`, no history entry. A rejected value changes nothing and keeps the
    /// gesture open.
    pub fn update_property_gesture(
        &mut self,
        gesture: Option<u64>,
        raw: &Raw,
    ) -> Result<Value, CommandError> {
        self.require_live_gesture(gesture)?;
        let Some(open) = self.history.gesture.as_ref() else {
            return Err(no_gesture());
        };
        let (id, clip, kind, definition) = (open.id, open.clip, open.kind, open.definition);
        let mut outcome = self
            .inspector
            .set_property(clip, kind, definition.id, raw)?;
        if outcome["changed"] == true {
            self.project.revision += 1;
            if let Some(open) = self.history.gesture.as_mut() {
                open.touched = true;
                open.revision = self.project.revision;
            }
        }
        outcome["action"] = json!("gesture.update");
        outcome["gesture"] = json!(id);
        Ok(outcome)
    }

    /// Ends the gesture as ONE undo entry from the original model to the current one, or as
    /// no entry when the value is back where it started. Clears redo only when it records.
    pub fn commit_property_gesture(&mut self, gesture: Option<u64>) -> Result<Value, CommandError> {
        self.require_live_gesture(gesture)?;
        let Some(open) = self.history.gesture.take() else {
            return Err(no_gesture());
        };
        let current = self.inspector.value(open.clip, open.definition);
        let changed = current != open.original;
        // Inside a batch `transaction` the batch owns the one entry; a gesture committed in
        // its body only contributes its value.
        let recorded = changed && self.history.is_recording();
        let mut outcome = json!({ "target": open.target(), "action": "gesture.commit",
            "gesture": open.id, "clip": format!("clip:{}", open.clip),
            "value": current.to_json(), "previous": open.original.to_json(),
            "changed": changed, "recorded": recorded });
        if recorded {
            self.history.record(LABEL, open.before);
        }
        outcome["history"] = self.history.to_json();
        Ok(outcome)
    }

    /// Ends the gesture and restores the original value. No entry, redo untouched.
    pub fn cancel_property_gesture(&mut self, gesture: Option<u64>) -> Result<Value, CommandError> {
        self.require_live_gesture(gesture)?;
        let Some(open) = self.history.gesture.take() else {
            return Err(no_gesture());
        };
        self.revert_gesture(&open)?;
        Ok(json!({ "target": open.target(), "action": "gesture.cancel",
            "gesture": open.id, "clip": format!("clip:{}", open.clip),
            "value": open.original.to_json(), "changed": open.touched, "recorded": false,
            "history": self.history.to_json() }))
    }

    /// Settles an open gesture as if it had been released (see the module docs). Called
    /// before any edit, `undo`, `redo` and batch transaction.
    pub fn settle_property_gesture(&mut self) {
        if self.history.gesture.is_none() {
            return;
        }
        // WHY the result is dropped: the only failure is a stale gesture, which has already
        // removed itself (and rolled back its own preview) before reporting; there is nothing
        // left to commit and the caller's edit must still run.
        let _ = self.commit_property_gesture(None);
    }

    /// Checks that a gesture is open, is the one asked for and still points at a live target.
    /// A stale one is dropped here (see the module docs for what is rolled back).
    fn require_live_gesture(&mut self, requested: Option<u64>) -> Result<(), CommandError> {
        let Some(open) = self.history.gesture.as_ref() else {
            return Err(no_gesture());
        };
        if requested.is_some_and(|id| id != open.id) {
            return Err(CommandError::new(
                "stale_gesture",
                format!("Gesture {} is not the open gesture", requested.unwrap_or(0)),
            )
            .details(json!({ "open": open.id })));
        }
        let revision_kept = self.project.revision == open.revision;
        let target_alive = self.timeline.id == open.sequence
            && self
                .timeline
                .clip(open.clip)
                .is_some_and(|clip| clip.kind == open.kind);
        if revision_kept && target_alive {
            return Ok(());
        }
        let (reason, what) = if revision_kept {
            ("target_changed", "target")
        } else {
            ("document_changed", "document")
        };
        let id = open.id;
        if let Some(stale) = self.history.gesture.take() {
            // Only our own preview changed the document: undo it. Otherwise leave the model.
            if revision_kept {
                self.revert_gesture(&stale)?;
            }
        }
        Err(CommandError::new(
            "stale_gesture",
            format!("Gesture {id} ended: the {what} changed while it was open"),
        )
        .details(json!({ "gesture": id, "reason": reason })))
    }

    /// Puts the original value back when a preview changed it.
    fn revert_gesture(&mut self, open: &OpenGesture) -> Result<(), CommandError> {
        if !open.touched {
            return Ok(());
        }
        // WHY `restore_override` and not `set_property`: the original was valid when the
        // gesture began, and putting it back must not fail because a dependency (`requires`)
        // was switched off in the meantime.
        self.inspector.restore_override(
            &open.clip,
            open.definition.id,
            &open.original.to_json(),
        )?;
        self.project.revision += 1;
        Ok(())
    }
}

/// Parses the `property.gesture.*` operations of the JSON vocabulary into a [`Command`].
///
/// Shape only, like `scene_command` in `ops`: every rule (clamp, applicability, stale
/// targets) belongs to the commands above, so the CLI, a script, the window socket and the
/// library mean the same thing. `ops::parse_op` delegates here for the `property.gesture.`
/// prefix (see `docs/slider-gesture-history-report.md` for the one-line hookup).
///
/// ```json
/// {"op":"property.gesture.begin","property":"zoom","clip":"c-img"}
/// {"op":"property.gesture.update","value":120,"gesture":1}
/// {"op":"property.gesture.commit"}
/// {"op":"property.gesture.cancel"}
/// ```
pub fn parse_op(op: &str, request: &Value) -> Result<Command, CommandError> {
    let invalid = |message: String| CommandError::new("invalid_request", message);
    let text = |key: &str| -> Result<Option<String>, CommandError> {
        match request.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.clone())),
            Some(_) => Err(invalid(format!("Expected string field `{key}`"))),
        }
    };
    let gesture = || -> Result<Option<u64>, CommandError> {
        match request.get("gesture") {
            None | Some(Value::Null) => Ok(None),
            Some(value) => value
                .as_u64()
                .map(Some)
                .ok_or_else(|| invalid("Expected whole number field `gesture`".to_string())),
        }
    };
    let command = match op {
        "property.gesture.begin" => GestureCommand::Begin {
            clip: text("clip")?,
            property: text("property")?
                .ok_or_else(|| invalid("Expected string field `property`".to_string()))?,
        },
        "property.gesture.update" => GestureCommand::Update {
            gesture: gesture()?,
            value: request
                .get("value")
                .and_then(Raw::from_json)
                .ok_or_else(|| invalid("Expected scalar field `value`".to_string()))?,
        },
        "property.gesture.commit" => GestureCommand::Commit {
            gesture: gesture()?,
        },
        "property.gesture.cancel" => GestureCommand::Cancel {
            gesture: gesture()?,
        },
        other => {
            return Err(CommandError::new(
                "unknown_operation",
                format!("`{other}` is not a property gesture operation"),
            ));
        }
    };
    Ok(Command::PropertyGesture(command))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operations_parse_into_gesture_commands() {
        let parse = |request: Value| {
            let op = request["op"].as_str().unwrap().to_string();
            parse_op(&op, &request)
        };
        assert_eq!(
            parse(json!({"op":"property.gesture.begin","property":"zoom","clip":"c"})).unwrap(),
            Command::PropertyGesture(GestureCommand::Begin {
                clip: Some("c".into()),
                property: "zoom".into()
            })
        );
        assert_eq!(
            parse(json!({"op":"property.gesture.update","value":"120","gesture":3})).unwrap(),
            Command::PropertyGesture(GestureCommand::Update {
                gesture: Some(3),
                value: Raw::Text("120".into())
            })
        );
        assert_eq!(
            parse(json!({"op":"property.gesture.commit"})).unwrap(),
            Command::PropertyGesture(GestureCommand::Commit { gesture: None })
        );
        assert_eq!(
            parse(json!({"op":"property.gesture.cancel","gesture":null})).unwrap(),
            Command::PropertyGesture(GestureCommand::Cancel { gesture: None })
        );
    }

    #[test]
    fn malformed_requests_are_rejected_with_a_typed_error() {
        let code = |request: Value| {
            let op = request["op"].as_str().unwrap().to_string();
            parse_op(&op, &request).unwrap_err().code
        };
        assert_eq!(
            code(json!({"op":"property.gesture.begin"})),
            "invalid_request"
        );
        assert_eq!(
            code(json!({"op":"property.gesture.update"})),
            "invalid_request"
        );
        assert_eq!(
            code(json!({"op":"property.gesture.update","value":1,"gesture":"x"})),
            "invalid_request"
        );
        assert_eq!(
            code(json!({"op":"property.gesture.begin","property":7})),
            "invalid_request"
        );
        assert_eq!(
            code(json!({"op":"property.gesture.explode"})),
            "unknown_operation"
        );
    }
}
