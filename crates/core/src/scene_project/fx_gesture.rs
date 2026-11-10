//! Running the F016 effect gesture (`scene.fx.gesture.*`, BL082.6) and the effect reads that can
//! look at a gesture's candidate (`scene.fx.list` / `scene.fx.frame` with a `gesture`).
//!
//! The candidate of an effect gesture is the *core's*: it is built here, by `scene_edit::apply_fx`
//! from the scene as it was at `begin`, and held in the same gesture slot as every other
//! gesture (`scene_interaction::GestureSlot`). A window slider, a keyboard step, a script line
//! and a test all feed the same `update`, so no widget buffer decides what the preview shows or
//! what is committed.
//!
//! What each phase writes:
//!
//! * `begin`, `update`, `cancel`, `status` and every read: **nothing**. No document, no file, no
//!   undo entry, no revision bump (`SceneCommand::history_label` is `None` for them).
//! * `commit`: ONE `scene.fx.edit` of the gesture's ops, through `edit_fx`, so the catalog, the
//!   importer, the structural/temporal gates and the size budget are the ones of `scene.fx.edit`.
//!   That is ONE `Edit effects` entry, or none when the candidate equals the base (a drag that
//!   came back to where it started).
//!
//! What makes a gesture stale (checked when it is used, see `Gesture::invalidation`): the scene
//! was detached, replaced (any edit, undo or redo of it), or the project revision moved. A stale
//! gesture can only be cancelled; it never confirms values it began with. A new project is a new
//! interaction session, so the ID of a gesture of the old one is `unknown_gesture`
//! (`reason: other_session`).
//!
//! A refused `update` (the layer's rules: overlap, range, envelope...) returns the typed error,
//! leaves the gesture and its last LEGAL candidate untouched, and records the refusal so that
//! `status` can say that the newest proposal is not the one on screen.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::directed_scene::DirectedScene;
use crate::directed_scene::fx::{FX_CATALOG_VERSION, Locale};
use crate::scene_edit::{self, FxOp};
use crate::scene_interaction::{FrameAt, FxGestureCommand, GestureKind};
use crate::state::{CommandError, EditorState};

use super::FxProbe;
use super::fx::{changes_json, frame_answer, list_answer};
use super::interaction::ensure_kind;

/// Error `details` with the gesture's own facts added, whatever the refusal already carried.
fn with_gesture_details(error: CommandError, id: &str, candidate_kept: bool) -> CommandError {
    let mut details = match error.details.as_deref() {
        Some(Value::Object(map)) => Value::Object(map.clone()),
        Some(other) => json!({ "refusal_details": other }),
        None => json!({}),
    };
    details["gesture"] = json!(id);
    details["candidate_kept"] = json!(candidate_kept);
    CommandError {
        details: Some(Box::new(details)),
        ..error
    }
}

impl EditorState {
    /// Routes the FX gesture commands through the shared slot; status only budgets an applicable FX preview.
    pub(super) fn apply_fx_gesture(
        &mut self,
        command: FxGestureCommand,
    ) -> Result<Value, CommandError> {
        match command {
            FxGestureCommand::Begin { scene, clip, label } => {
                self.fx_gesture_begin(&scene, &clip, label)
            }
            FxGestureCommand::Update { gesture, ops } => self.fx_gesture_update(&gesture, ops),
            FxGestureCommand::Commit { gesture } => self.fx_gesture_commit(&gesture),
            FxGestureCommand::Cancel { gesture } => {
                let mut answer = self.gesture_cancel(&gesture)?;
                answer["action"] = json!("scene.fx.gesture.cancel");
                Ok(answer)
            }
            FxGestureCommand::Status => {
                let mut answer = self.gesture_status();
                answer["action"] = json!("scene.fx.gesture.status");
                if let Some(gesture) = self.scene_interaction.gesture.current().filter(|gesture| {
                    gesture.fx_clip().is_some()
                        && gesture
                            .invalidation(&self.scenes, self.project.revision)
                            .is_none()
                }) {
                    let scene = gesture.preview_scene();
                    let budget = scene
                        .bundle()
                        .fx
                        .pass_budget(scene.bundle().edit_duration_frames);
                    answer["budget"] = budget.to_json();
                    answer["warnings"] = Value::Array(budget.warnings());
                }
                Ok(answer)
            }
        }
    }

    /// Claims the shared gesture slot and captures this clip's scene/revision without changing the document.
    fn fx_gesture_begin(
        &mut self,
        scene: &str,
        clip: &str,
        label: Option<String>,
    ) -> Result<Value, CommandError> {
        let entry = self.attached_scene(scene)?;
        let label = match label.map(|raw| raw.trim().to_string()) {
            Some(trimmed) if trimmed.is_empty() => {
                return Err(CommandError::invalid_value(
                    "A gesture label cannot be blank",
                    json!("a non-empty string"),
                ));
            }
            other => other,
        };
        let exists = entry
            .scene
            .bundle()
            .fx
            .tracks
            .iter()
            .any(|track| track.clips.iter().any(|candidate| candidate.id == clip));
        if !exists {
            return Err(CommandError::unknown_element("effect clip", clip)
                .details(json!({ "scene": scene })));
        }
        let base = Arc::clone(&entry.scene);
        let replaced_stale = self.claim_gesture_slot()?;
        let id = self.scene_interaction.gesture.next_id();
        let gesture = crate::scene_interaction::Gesture::new_fx(
            id.clone(),
            scene.to_string(),
            label,
            clip.to_string(),
            base,
            self.project.revision,
        );
        let answer = json!({
            "action": "scene.fx.gesture.begin", "gesture": id, "scene": scene, "clip": clip,
            "label": gesture.label(), "kind": "fx",
            "project": self.project.id, "revision": self.project.revision,
            "replaced_stale": replaced_stale,
            "changed": true, "document_changed": false,
        });
        self.scene_interaction.gesture.open(gesture);
        Ok(answer)
    }

    /// The candidate for `ops`, from the gesture's base. A rejection (any `apply_fx` error, an op
    /// outside the gesture's clip, an oversized scene) returns before the candidate is replaced,
    /// so the last legal one is still what a preview shows and a commit applies.
    fn fx_gesture_update(&mut self, id: &str, ops: Vec<FxOp>) -> Result<Value, CommandError> {
        let gesture = self.scene_interaction.gesture.find(id)?;
        self.ensure_live(gesture)?;
        ensure_kind(
            gesture,
            &GestureKind::Fx {
                clip: String::new(),
            },
            "scene.gesture.update",
        )?;
        let scene = gesture.scene().to_string();
        let clip = gesture.fx_clip().unwrap_or_default().to_string();
        let base = Arc::clone(gesture.base());
        let kept = gesture.has_candidate();

        let attempt = self
            .fx_gesture_candidate(id, &scene, &clip, &base, &ops)
            .map_err(|error| with_gesture_details(error, id, kept));
        let (stored, summary) = match attempt {
            Ok(accepted) => accepted,
            Err(error) => {
                if let Ok(gesture) = self.scene_interaction.gesture.find_mut(id) {
                    gesture.note_refusal(&error);
                }
                return Err(error);
            }
        };

        let (modified, changed_ops) = summary.as_ref().map_or((false, 0), |summary| {
            (summary.modified, summary.changed_count())
        });
        let changes = summary.as_ref().map(changes_json).unwrap_or_default();
        let (budget_scene, stored_scene) = match &stored {
            Some((scene, _)) => (Arc::clone(scene), Some(Arc::clone(scene))),
            None => (Arc::clone(&base), None),
        };
        let budget = budget_scene
            .bundle()
            .fx
            .pass_budget(budget_scene.bundle().edit_duration_frames);
        let clips = budget_scene.bundle().fx.clip_count();
        drop(stored_scene);
        let gesture = self.scene_interaction.gesture.find_mut(id)?;
        gesture.set_fx_candidate(ops, stored);
        Ok(json!({
            "action": "scene.fx.gesture.update", "gesture": id, "scene": scene, "clip": clip,
            "updates": gesture.updates(),
            "would_change": modified, "changed_ops": changed_ops, "changes": changes,
            "clip_count": clips,
            "budget": budget.to_json(), "warnings": budget.warnings(),
            "changed": true, "document_changed": false,
        }))
    }

    /// Validates `ops` against the gesture and builds the candidate; touches nothing.
    #[allow(clippy::type_complexity)]
    fn fx_gesture_candidate(
        &self,
        id: &str,
        scene: &str,
        clip: &str,
        base: &Arc<DirectedScene>,
        ops: &[FxOp],
    ) -> Result<
        (
            Option<(Arc<DirectedScene>, scene_edit::FxSummary)>,
            Option<scene_edit::FxSummary>,
        ),
        CommandError,
    > {
        for (index, op) in ops.iter().enumerate() {
            let target = match op {
                FxOp::SetParam { clip, .. }
                | FxOp::SetField { clip, .. }
                | FxOp::SetHidden { clip, .. } => clip,
                FxOp::AddClip(_) | FxOp::RemoveClip { .. } => {
                    return Err(CommandError::new(
                        "gesture_op_not_allowed",
                        format!(
                            "Gesture `{id}` edits one clip; operation {index} adds or removes a \
                             clip. Use `scene.fx.edit` for that"
                        ),
                    )
                    .target(id)
                    .details(json!({ "gesture": id, "index": index })));
                }
            };
            if target != clip {
                return Err(CommandError::new(
                    "gesture_target_mismatch",
                    format!(
                        "Gesture `{id}` was begun on clip `{clip}`; operation {index} edits `{target}`"
                    ),
                )
                .target(target.clone())
                .details(json!({ "gesture": id, "index": index, "clip": target })));
            }
        }
        if ops.is_empty() {
            // The gesture back at its start: nothing to preview, nothing to commit.
            return Ok((None, None));
        }
        let outcome = scene_edit::apply_fx(base, ops)?;
        if outcome.summary.modified {
            self.scenes
                .check_replacement_size(scene, &outcome.candidate)?;
        }
        let summary = outcome.summary.clone();
        Ok((
            Some((Arc::new(outcome.candidate), outcome.summary)),
            Some(summary),
        ))
    }

    /// Applies the gesture as ONE `scene.fx.edit`. The gesture leaves the slot before the edit
    /// and goes back when the edit is rejected, so a rejection keeps everything as it was.
    fn fx_gesture_commit(&mut self, id: &str) -> Result<Value, CommandError> {
        let gesture = self.scene_interaction.gesture.find(id)?;
        self.ensure_live(gesture)?;
        ensure_kind(
            gesture,
            &GestureKind::Fx {
                clip: String::new(),
            },
            "scene.gesture.commit",
        )?;
        let scene = gesture.scene().to_string();
        let clip = gesture.fx_clip().unwrap_or_default().to_string();
        let ops = gesture.fx_ops().to_vec();
        let taken = self.scene_interaction.gesture.take();
        if ops.is_empty() {
            return Ok(json!({
                "action": "scene.fx.gesture.commit", "gesture": id, "scene": scene, "clip": clip,
                "changed": false, "document_changed": false, "noop": true,
                "changed_ops": 0, "changes": [],
            }));
        }
        match self.edit_fx(&scene, &ops, false) {
            Ok(mut outcome) => {
                outcome["action"] = json!("scene.fx.gesture.commit");
                outcome["gesture"] = json!(id);
                outcome["clip"] = json!(clip);
                outcome["document_changed"] = outcome["changed"].clone();
                Ok(outcome)
            }
            Err(error) => {
                if let Some(gesture) = taken {
                    self.scene_interaction.gesture.open(gesture);
                }
                Err(error)
            }
        }
    }

    /// The scene a read looks at: the document's, or, with `gesture`, that gesture's candidate.
    /// The gesture must belong to `scene` and still be applicable.
    fn fx_read_scene(
        &self,
        scene: &str,
        gesture: Option<&str>,
    ) -> Result<(Arc<DirectedScene>, Option<String>), CommandError> {
        let entry = self.attached_scene(scene)?;
        let Some(id) = gesture else {
            return Ok((Arc::clone(&entry.scene), None));
        };
        let open = self.scene_interaction.gesture.find(id)?;
        if open.scene() != scene {
            return Err(CommandError::new(
                "gesture_scene_mismatch",
                format!(
                    "Gesture `{id}` edits scene `{}`, not `{scene}`",
                    open.scene()
                ),
            )
            .target(id)
            .details(json!({ "gesture": id, "scene": open.scene() })));
        }
        self.ensure_live(open)?;
        // A gizmo drag or a slider gesture (`properties`) has a candidate too, but it is not an
        // effect candidate: the read refuses it instead of answering as if it were one.
        ensure_kind(
            open,
            &GestureKind::Fx {
                clip: String::new(),
            },
            "the same query without `gesture`",
        )?;
        Ok((Arc::clone(open.preview_scene()), Some(id.to_string())))
    }

    /// Lists committed FX, or the explicitly requested live candidate after checking its scene and kind.
    pub(super) fn fx_list(
        &self,
        scene: &str,
        gesture: Option<&str>,
    ) -> Result<Value, CommandError> {
        let (shown, id) = self.fx_read_scene(scene, gesture)?;
        let mut answer = list_answer(scene, &shown);
        answer["source"] = json!(if id.is_some() { "gesture" } else { "document" });
        if let Some(id) = id {
            answer["gesture"] = json!(id);
            answer["has_candidate"] = json!(
                self.scene_interaction
                    .gesture
                    .current()
                    .is_some_and(|open| open.has_candidate())
            );
        }
        Ok(answer)
    }

    /// The inspector model of the layer (`scene.fx.inspect`): one entry per clip with its fields,
    /// the effects that can be added and the scopes that are not drawn yet. From the document, or
    /// from a live gesture's candidate (then with the gesture's newest refusal, so a client that
    /// shows the last legal value can also show why the proposal was not taken). A read: nothing
    /// is written, whatever the `probe` says.
    pub(super) fn fx_inspect(
        &self,
        scene: &str,
        clip: Option<&str>,
        gesture: Option<&str>,
        locale: Locale,
        probe: Option<&FxProbe>,
    ) -> Result<Value, CommandError> {
        let (shown, id) = self.fx_read_scene(scene, gesture)?;
        let bundle = shown.bundle();
        let edit_frames = i64::try_from(bundle.edit_duration_frames).unwrap_or(i64::MAX);
        let mut inspectors = scene_edit::layer_inspector(&bundle.fx, edit_frames);
        let probed = probe.map(|probe| probe_answer(&shown, &inspectors, probe));
        if let Some(wanted) = clip {
            inspectors.retain(|inspector| inspector.clip == wanted);
            if inspectors.is_empty() {
                return Err(CommandError::unknown_element("effect clip", wanted));
            }
        }
        let budget = bundle.fx.pass_budget(bundle.edit_duration_frames);
        let mut answer = json!({
            "action": "scene.fx.inspect", "scene": scene, "locale": locale.tag(),
            "catalog_version": FX_CATALOG_VERSION,
            "source": if id.is_some() { "gesture" } else { "document" },
            "edit_duration_frames": bundle.edit_duration_frames,
            "scopes": scene_edit::scopes().iter().map(|(kind, drawn, reason)| json!({
                "kind": kind, "applicable": drawn, "reason": reason,
            })).collect::<Vec<_>>(),
            "offers": scene_edit::offers().iter().map(|offer| offer.to_json(locale)).collect::<Vec<_>>(),
            "clip_count": inspectors.len(),
            "clips": inspectors.iter().map(|inspector| {
                let mut described = inspector.to_json(locale);
                if let Some((_, clip)) = bundle.fx.clip(&inspector.clip) {
                    described["reset_ops"] = json!(inspector.reset_ops_json(clip));
                    for (field_json, field) in described["fields"].as_array_mut().unwrap().iter_mut().zip(&inspector.fields) {
                        if field.default.is_some() {
                            field_json["reset"]["op"] = json!(field.reset_op_json(clip));
                        }
                    }
                }
                described
            }).collect::<Vec<_>>(),
            "budget": budget.to_json(), "warnings": budget.warnings(),
        });
        if let Some(id) = id {
            let open = self.scene_interaction.gesture.find(&id)?;
            answer["gesture"] = json!({
                "id": id, "clip": open.fx_clip(),
                "refusal": open.refusal().map(|refusal| json!({
                    "code": refusal.code, "message": refusal.message,
                })),
            });
        }
        if let Some(probed) = probed {
            answer["probe"] = probed;
        }
        Ok(answer)
    }

    /// Evaluates FX at a frame from the document or a validated candidate, without committing it.
    pub(super) fn fx_frame(
        &self,
        scene: &str,
        at: &FrameAt,
        gesture: Option<&str>,
    ) -> Result<Value, CommandError> {
        let (shown, id) = self.fx_read_scene(scene, gesture)?;
        let mut answer = frame_answer(scene, &shown, at)?;
        answer["source"] = json!(if id.is_some() { "gesture" } else { "document" });
        if let Some(id) = id {
            answer["gesture"] = json!(id);
        }
        Ok(answer)
    }
}

/// Judges one proposed value of one field without applying it: the typed refusal of the inspector
/// (a value that is not finite, not whole for a frame field, an unknown clip or field) or of the
/// core's own dry run of the very op the field would send. `ok: true` carries the op and whether
/// it would change the document; nothing is written either way.
fn probe_answer(
    scene: &DirectedScene,
    inspectors: &[scene_edit::ClipInspector],
    probe: &FxProbe,
) -> Value {
    let refuse = |error: CommandError, value: f64| {
        json!({
            "clip": probe.clip, "field": probe.field, "value": value, "requested_value": probe.value, "ok": false,
            "error": { "code": error.code, "message": error.message },
        })
    };
    let Some(inspector) = inspectors
        .iter()
        .find(|inspector| inspector.clip == probe.clip)
    else {
        return refuse(
            CommandError::unknown_element("effect clip", &probe.clip),
            probe.value,
        );
    };
    let Some(field) = inspector.field(&probe.field) else {
        return refuse(
            CommandError::new(
                "unknown_field",
                format!("`{}` has no field `{}`", probe.clip, probe.field),
            ),
            probe.value,
        );
    };
    let value = if probe.quantize {
        field.quantize(probe.value)
    } else {
        probe.value
    };
    let Some((_, clip)) = scene.bundle().fx.clip(&probe.clip) else {
        return refuse(
            CommandError::unknown_element("effect clip", &probe.clip),
            probe.value,
        );
    };
    let op = match field.op(clip, value) {
        Ok(op) => op,
        Err(error) => return refuse(error, value),
    };
    match scene_edit::dry_run_fx(scene, std::slice::from_ref(&op)) {
        Ok(summary) => json!({
            "clip": probe.clip, "field": probe.field, "value": value, "requested_value": probe.value, "ok": true,
            "would_change": summary.modified, "changes": changes_json(&summary),
        }),
        Err(error) => refuse(CommandError::from(error), value),
    }
}
