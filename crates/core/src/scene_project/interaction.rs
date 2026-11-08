//! Running the F039 commands (`scene.tree`, `scene.evaluate`, `scene.select`, `scene.selection`,
//! `scene.gesture.*`) against a project. The model they act on (addresses, selection, gesture
//! state, tree and frame answers) is `scene_interaction`; this file is the glue that needs
//! `EditorState`: the `SceneBook`, the project revision and `scene_edit` through the same
//! `edit_scene` that `scene.edit` runs.
//!
//! Which of them touch the project:
//!
//! * **Nothing:** tree, evaluate, selection, begin, update, cancel and status. They have no
//!   history label (`SceneCommand::history_label`), never write the `SceneBook` and never bump
//!   the project revision, so they cannot create an undo entry or dirty the document.
//!   `changed` in their answers means "the session state of the scene tools changed" (as it does
//!   for selecting a media item), and `document_changed: false` says the project did not.
//! * **One thing:** commit. It runs the gesture's ops through `edit_scene`, i.e. the catalog,
//!   the importer, the structural/temporal gates and the size budget of `scene.edit`, and
//!   `EditorState::execute` records one undo entry when the answer says `changed`.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::directed_scene::Entity;
use crate::scene_edit::{self, EditOp, EditOutcome};
use crate::scene_interaction::{
    Dropped, FrameAt, Gesture, GestureCommand, GestureKind, Invalidation, SelectMode, Selected,
    ensure_exists, entity_of_property, entity_path, frame_json, parse_entity, tree_json,
};
use crate::state::{CommandError, EditorState};

use super::run::{changes_json, with_conversions};

/// A gesture of one kind refuses the verbs of the other: its candidate and its ops are of a
/// different type, and its commit has a different history entry. `use_instead` names the verb
/// that does apply, so the error says what to call.
pub(super) fn ensure_kind(
    gesture: &Gesture,
    wanted: &GestureKind,
    use_instead: &str,
) -> Result<(), CommandError> {
    let matches = match (gesture.kind(), wanted) {
        (GestureKind::Properties, GestureKind::Properties) => true,
        (GestureKind::Fx { .. }, GestureKind::Fx { .. }) => true,
        _ => false,
    };
    if matches {
        return Ok(());
    }
    Err(CommandError::new(
        "gesture_kind_mismatch",
        format!(
            "Gesture `{}` edits {}; use `{use_instead}` for it. Nothing was applied",
            gesture.id(),
            gesture.kind().as_str(),
        ),
    )
    .target(gesture.id())
    .details(json!({
        "gesture": gesture.id(), "kind": gesture.kind().as_str(), "use": use_instead,
    })))
}

impl EditorState {
    pub(super) fn attached_scene(&self, id: &str) -> Result<&super::AttachedScene, CommandError> {
        self.scenes
            .get(id)
            .ok_or_else(|| CommandError::unknown_element("scene", id))
    }

    /// Fails with the typed `gesture_invalidated` when the gesture can no longer be applied.
    pub(super) fn ensure_live(&self, gesture: &Gesture) -> Result<(), CommandError> {
        match gesture.invalidation(&self.scenes, self.project.revision) {
            Some(reason) => Err(gesture.invalidated_error(reason)),
            None => Ok(()),
        }
    }

    pub(super) fn scene_tree(
        &self,
        scene: &str,
        root: Option<&str>,
        depth: Option<usize>,
    ) -> Result<Value, CommandError> {
        let entry = self.attached_scene(scene)?;
        let selected: Vec<&Entity> = self
            .scene_interaction
            .selection
            .entities_of(scene)
            .collect();
        let tree = tree_json(&entry.scene, root, depth, &selected)?;
        Ok(json!({ "action": "scene.tree", "scene": scene, "tree": tree }))
    }

    pub(super) fn scene_evaluate(
        &self,
        scene: &str,
        at: &FrameAt,
        gesture: Option<&str>,
    ) -> Result<Value, CommandError> {
        let entry = self.attached_scene(scene)?;
        let Some(gesture_id) = gesture else {
            let mut answer = frame_json(&entry.scene, at)?;
            answer["action"] = json!("scene.evaluate");
            answer["scene"] = json!(scene);
            answer["source"] = json!("document");
            return Ok(answer);
        };
        let gesture = self.scene_interaction.gesture.find(gesture_id)?;
        if gesture.scene() != scene {
            return Err(CommandError::new(
                "gesture_scene_mismatch",
                format!(
                    "Gesture `{gesture_id}` edits scene `{}`, not `{scene}`",
                    gesture.scene()
                ),
            )
            .target(gesture_id)
            .details(json!({ "gesture": gesture_id, "scene": gesture.scene() })));
        }
        self.ensure_live(gesture)?;
        let mut answer = frame_json(gesture.preview_scene(), at)?;
        answer["action"] = json!("scene.evaluate");
        answer["scene"] = json!(scene);
        answer["source"] = json!("gesture");
        answer["gesture"] = json!(gesture_id);
        // `false` until an update gave a candidate: the frame is then the document's own.
        answer["has_candidate"] = json!(gesture.has_candidate());
        Ok(answer)
    }

    pub(super) fn scene_plan(
        &self,
        scene: &str,
        at: &FrameAt,
        partial: bool,
        gesture: Option<&str>,
    ) -> Result<Value, CommandError> {
        let entry = self.attached_scene(scene)?;
        let directed = if let Some(id) = gesture {
            let candidate = self.scene_interaction.gesture.find(id)?;
            self.ensure_live(candidate)?;
            if candidate.scene() != scene {
                return Err(CommandError::new(
                    "gesture_scene_mismatch",
                    "The gesture belongs to another scene",
                ));
            }
            candidate.preview_scene()
        } else {
            &entry.scene
        };
        let frame = match at {
            FrameAt::Frame(frame) => *frame,
            FrameAt::Time(time) => directed.bundle().frame_at_time(*time)?,
        };
        // Admission also protects callers that construct EditorState directly.
        super::run::check_admission(directed)?;
        let plan = crate::scene_plan::ScenePlan::build_with(
            directed,
            frame,
            crate::scene_plan::PlanOptions {
                allow_partial: partial,
            },
        )?;
        Ok(json!({ "action": "scene.plan", "scene": scene,
            "complete": plan.is_complete(), "plan": plan.to_json() }))
    }

    pub(super) fn scene_select(
        &mut self,
        scene: Option<&str>,
        entities: &[String],
        mode: SelectMode,
    ) -> Result<Value, CommandError> {
        let mut parsed = Vec::with_capacity(entities.len());
        if mode == SelectMode::Clear {
            if !entities.is_empty() {
                return Err(CommandError::new(
                    "invalid_request",
                    "`clear` takes no entities",
                ));
            }
        } else {
            let Some(id) = scene else {
                return Err(CommandError::new(
                    "invalid_request",
                    format!("`{}` needs a `scene`", mode.as_str()),
                ));
            };
            let entry = self.attached_scene(id)?;
            for raw in entities {
                let entity = parse_entity(raw)?;
                // Removing what is already gone must stay possible; selecting it must not.
                if mode != SelectMode::Remove {
                    ensure_exists(entry.scene.bundle(), &entity)?;
                }
                parsed.push(entity);
            }
        }
        let changed = self
            .scene_interaction
            .selection
            .apply(scene, parsed, mode)?;
        let selection = &self.scene_interaction.selection;
        Ok(json!({
            "action": "scene.select", "mode": mode.as_str(),
            "changed": changed, "document_changed": false,
            "count": selection.entries().len(),
            "entities": selection.entries().iter().map(Selected::to_json).collect::<Vec<_>>(),
        }))
    }

    /// The selection, after dropping what the project no longer holds. `dropped` also lists what
    /// earlier prunes removed since the selection was last set, so a caller that did not see
    /// the undo that took an entity away still learns why it left.
    pub(super) fn scene_selection(&mut self) -> Value {
        let removed = self.scene_interaction.selection.prune(&self.scenes);
        let selection = &self.scene_interaction.selection;
        json!({
            "action": "scene.selection",
            "changed": !removed.is_empty(), "document_changed": false,
            "count": selection.entries().len(),
            "entities": selection.entries().iter().map(Selected::to_json).collect::<Vec<_>>(),
            "dropped": selection.dropped().iter().map(Dropped::to_json).collect::<Vec<_>>(),
        })
    }

    pub(super) fn apply_gesture(&mut self, command: GestureCommand) -> Result<Value, CommandError> {
        match command {
            GestureCommand::Begin {
                scene,
                targets,
                label,
            } => self.gesture_begin(&scene, &targets, label),
            GestureCommand::Update { gesture, ops } => self.gesture_update(&gesture, ops),
            GestureCommand::Commit { gesture } => self.gesture_commit(&gesture),
            GestureCommand::Cancel { gesture } => self.gesture_cancel(&gesture),
            GestureCommand::Status => Ok(self.gesture_status()),
        }
    }

    pub(super) fn gesture_begin(
        &mut self,
        scene: &str,
        targets: &[String],
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
        let mut entities: Vec<Entity> = Vec::with_capacity(targets.len());
        for raw in targets {
            let entity = parse_entity(raw)?;
            ensure_exists(entry.scene.bundle(), &entity)?;
            if !entities.contains(&entity) {
                entities.push(entity);
            }
        }
        let base = Arc::clone(&entry.scene);

        let replaced_stale = self.claim_gesture_slot()?;

        let id = self.scene_interaction.gesture.next_id();
        let gesture = Gesture::new(
            id.clone(),
            scene.to_string(),
            label,
            entities,
            base,
            self.project.revision,
        );
        let answer = json!({
            "action": "scene.gesture.begin", "gesture": id, "scene": scene,
            "label": gesture.label(),
            "targets": gesture.targets().iter().map(entity_path).collect::<Vec<_>>(),
            "revision": self.project.revision,
            "replaced_stale": replaced_stale,
            "changed": true, "document_changed": false,
        });
        self.scene_interaction.gesture.open(gesture);
        Ok(answer)
    }

    /// The frame of the attached scene `scene` that the NLE playhead shows: the playhead's time
    /// (its sequence rate) converted by the *scene's own* clock (`frame_at_time`, floor). The
    /// playhead is a frame of the sequence and a bundle frame is a frame of the bundle: they are
    /// the same number only when the two rates are, so no adapter may hand one to the other. An
    /// instant the edit does not hold is the typed error of the conversion, never a clamped frame.
    pub fn scene_frame_at_playhead(&self, scene: &str) -> Result<i64, CommandError> {
        let entry = self.attached_scene(scene)?;
        let time = self.timeline.time_of(self.timeline.playhead);
        Ok(entry.scene.bundle().frame_at_time(time)?)
    }

    /// One gesture at a time, like one pointer: a second `begin` would silently orphan the first
    /// one's candidate. A stale gesture is dead already, so it is replaced and said so (the
    /// returned value names it; `null` when the slot was empty). Shared by every kind of gesture.
    pub(super) fn claim_gesture_slot(&self) -> Result<Value, CommandError> {
        let Some(open) = self.scene_interaction.gesture.current() else {
            return Ok(Value::Null);
        };
        let Some(reason) = open.invalidation(&self.scenes, self.project.revision) else {
            return Err(CommandError::new(
                "gesture_active",
                format!(
                    "Gesture `{}` is already in progress; commit or cancel it first",
                    open.id()
                ),
            )
            .target(open.id())
            .details(json!({ "gesture": open.id(), "scene": open.scene() })));
        };
        Ok(json!({ "gesture": open.id(), "reason": reason.as_str() }))
    }

    /// Computes the candidate for `ops` from the gesture's base. A rejection (any `scene_edit`
    /// error, a target outside the gesture's, an oversized scene) returns before the gesture is
    /// touched, so the previous good candidate is still the one that commit would apply.
    pub(super) fn gesture_update(
        &mut self,
        id: &str,
        ops: Vec<EditOp>,
    ) -> Result<Value, CommandError> {
        let gesture = self.scene_interaction.gesture.find(id)?;
        self.ensure_live(gesture)?;
        ensure_kind(gesture, &GestureKind::Properties, "scene.fx.gesture.update")?;
        let scene = gesture.scene().to_string();
        let base = Arc::clone(gesture.base());
        if !gesture.targets().is_empty() {
            for (index, op) in ops.iter().enumerate() {
                let entity = entity_of_property(op.property());
                if gesture.targets().contains(&entity) {
                    continue;
                }
                let path = op.property().path();
                return Err(CommandError::new(
                    "gesture_target_mismatch",
                    format!(
                        "Gesture `{id}` was begun on other entities; operation {index} edits `{}`",
                        entity_path(&entity)
                    ),
                )
                .target(entity_path(&entity))
                .details(json!({ "gesture": id, "index": index, "path": path })));
            }
        }

        let mut conversions = None;
        let (stored, changes, would_change, changed_ops) = if ops.is_empty() {
            // An empty batch is the gesture back at its start: nothing to preview or commit.
            (None, Vec::new(), false, 0)
        } else {
            let EditOutcome { candidate, summary } = scene_edit::apply(&base, &ops)?;
            if summary.modified {
                self.scenes.check_replacement_size(&scene, &candidate)?;
            }
            let changes = changes_json(&summary);
            let (modified, count) = (summary.modified, summary.changed_count());
            conversions = Some(summary.clone());
            (
                Some((Arc::new(candidate), summary)),
                changes,
                modified,
                count,
            )
        };

        let gesture = self.scene_interaction.gesture.find_mut(id)?;
        gesture.set_candidate(ops, stored);
        let answer = json!({
            "action": "scene.gesture.update", "gesture": id, "scene": scene,
            "updates": gesture.updates(),
            "would_change": would_change, "changed_ops": changed_ops, "changes": changes,
            "changed": true, "document_changed": false,
        });
        Ok(match conversions {
            Some(summary) => with_conversions(answer, &summary),
            None => answer,
        })
    }

    /// Applies the gesture as ONE `scene.edit` batch. The gesture leaves the slot before the
    /// edit runs and goes back if the edit is rejected, so a rejection keeps the gesture (and
    /// everything else) exactly as it was and the caller can update or cancel.
    pub(super) fn gesture_commit(&mut self, id: &str) -> Result<Value, CommandError> {
        let gesture = self.scene_interaction.gesture.find(id)?;
        self.ensure_live(gesture)?;
        ensure_kind(gesture, &GestureKind::Properties, "scene.fx.gesture.commit")?;
        let scene = gesture.scene().to_string();
        let ops = gesture.ops().to_vec();
        let taken = self.scene_interaction.gesture.take();
        if ops.is_empty() {
            return Ok(json!({
                "action": "scene.gesture.commit", "gesture": id, "scene": scene,
                "changed": false, "document_changed": false, "noop": true,
                "changed_ops": 0, "changes": [],
            }));
        }
        match self.edit_scene(&scene, &ops, false) {
            Ok(mut outcome) => {
                outcome["action"] = json!("scene.gesture.commit");
                outcome["gesture"] = json!(id);
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

    pub(super) fn gesture_cancel(&mut self, id: &str) -> Result<Value, CommandError> {
        let gesture = self.scene_interaction.gesture.find(id)?;
        let invalidation = gesture.invalidation(&self.scenes, self.project.revision);
        let answer = json!({
            "action": "scene.gesture.cancel", "gesture": id, "scene": gesture.scene(),
            "discarded_updates": gesture.updates(),
            "was_invalidated": invalidation.is_some(),
            "invalidation": invalidation.map(Invalidation::as_str),
            "changed": true, "document_changed": false,
        });
        self.scene_interaction.gesture.take();
        Ok(answer)
    }

    pub(super) fn gesture_status(&self) -> Value {
        match self.scene_interaction.gesture.current() {
            None => json!({
                "action": "scene.gesture.status", "active": false, "gesture": null,
                "changed": false, "document_changed": false,
            }),
            Some(gesture) => json!({
                "action": "scene.gesture.status", "active": true,
                "gesture": gesture.to_json(&self.scenes, self.project.revision),
                "changed": false, "document_changed": false,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    //! White-box on purpose. The public API cannot reach the commit-time budget refusal: every
    //! operation that changes the size of the book also bumps `project.revision`, which
    //! invalidates the open gesture first. The branch is still live code (the gesture leaves the
    //! slot before the edit and must come back), so these tests make the book grow *without* a
    //! revision bump, the way only a bug elsewhere could, and check that nothing is lost.
    use serde_json::{Value, json};

    use super::*;
    use crate::scene_interaction::GestureCommand;
    use crate::scene_project::{MAX_SCENE_BYTES, SceneCommand, SceneSource};
    use crate::state::{Command, ProjectSettings};

    const FIGURE: &str = "scene/scene-a/node/figure";

    fn bundle() -> Value {
        serde_json::from_str(include_str!(
            "../../../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json"
        ))
        .unwrap()
    }

    fn scene_command(command: SceneCommand) -> Command {
        Command::Scene(command)
    }

    fn two_scenes() -> EditorState {
        let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
        for id in ["street", "other"] {
            state
                .execute(scene_command(SceneCommand::Attach {
                    id: Some(id.to_string()),
                    name: None,
                    source: SceneSource::Bundle(bundle()),
                }))
                .unwrap();
        }
        state
    }

    fn gesture_command(command: GestureCommand) -> Command {
        scene_command(SceneCommand::Gesture(command))
    }

    fn move_figure(x: f64) -> Vec<EditOp> {
        let ops = json!([{"set": format!("{FIGURE}/position"), "value": {"x": x, "y": 0, "z": 0}}]);
        crate::scene_edit::parse_batch(&ops).unwrap()
    }

    #[test]
    fn a_commit_that_busts_the_byte_budget_is_refused_and_the_gesture_comes_back_whole() {
        let mut state = two_scenes();
        let begun = state
            .execute(gesture_command(GestureCommand::Begin {
                scene: "street".to_string(),
                targets: vec![FIGURE.to_string()],
                label: None,
            }))
            .unwrap();
        let id = begun["gesture"].as_str().unwrap().to_string();
        state
            .execute(gesture_command(GestureCommand::Update {
                gesture: id.clone(),
                ops: move_figure(2.0),
            }))
            .unwrap();

        // The other scene "grows" to fill the whole budget behind the gesture's back.
        let other = state
            .scenes
            .iter()
            .position(|entry| entry.id == "other")
            .unwrap();
        let honest = state.scenes.scene_bytes[other];
        state.scenes.scene_bytes[other] = MAX_SCENE_BYTES;

        let document = state.to_document(None);
        let history = state.history.to_json();
        let revision = state.project.revision;
        let street = Arc::clone(&state.scenes.get("street").unwrap().scene);
        let ops = state.scene_interaction.gesture().unwrap().ops().to_vec();

        let error = state
            .execute(gesture_command(GestureCommand::Commit {
                gesture: id.clone(),
            }))
            .unwrap_err();
        assert_eq!(error.code, "scenes_too_large");
        assert_eq!(error.target.as_deref(), Some("street"));

        // Scene, history, revision and document are what they were; the gesture is back in the
        // slot with its candidate and ops, so the caller can update, cancel or retry.
        assert!(Arc::ptr_eq(
            &street,
            &state.scenes.get("street").unwrap().scene
        ));
        assert_eq!(state.to_document(None), document);
        assert_eq!(state.history.to_json(), history);
        assert_eq!(state.project.revision, revision);
        let gesture = state.scene_interaction.gesture().expect("gesture is back");
        assert_eq!(gesture.id(), id);
        assert!(gesture.has_candidate());
        assert_eq!(gesture.ops(), ops.as_slice());
        assert_eq!(gesture.updates(), 1);
        assert!(
            gesture
                .invalidation(&state.scenes, state.project.revision)
                .is_none(),
            "a refusal does not invalidate"
        );

        // Once the budget is honest again the very same gesture commits as ONE entry.
        state.scenes.scene_bytes[other] = honest;
        let depth = state.history.to_json()["undo_depth"].as_u64().unwrap();
        let committed = state
            .execute(gesture_command(GestureCommand::Commit { gesture: id }))
            .unwrap();
        assert_eq!(committed["changed"], true);
        assert_eq!(
            state.history.to_json()["undo_depth"].as_u64().unwrap(),
            depth + 1
        );
        assert!(state.scene_interaction.gesture().is_none());
    }
}
