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
    Dropped, FrameAt, Gesture, GestureCommand, Invalidation, SelectMode, Selected, ensure_exists,
    entity_of_property, entity_path, frame_json, parse_entity, tree_json,
};
use crate::state::{CommandError, EditorState};

use super::run::changes_json;

impl EditorState {
    fn attached_scene(&self, id: &str) -> Result<&super::AttachedScene, CommandError> {
        self.scenes
            .get(id)
            .ok_or_else(|| CommandError::unknown_element("scene", id))
    }

    /// Fails with the typed `gesture_invalidated` when the gesture can no longer be applied.
    fn ensure_live(&self, gesture: &Gesture) -> Result<(), CommandError> {
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

    fn gesture_begin(
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

        // One gesture at a time, like one pointer: a second `begin` would silently orphan the
        // first one's candidate. A stale gesture is dead already, so it is replaced and said so.
        let mut replaced_stale = Value::Null;
        if let Some(open) = self.scene_interaction.gesture.current() {
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
            replaced_stale = json!({ "gesture": open.id(), "reason": reason.as_str() });
        }

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

    /// Computes the candidate for `ops` from the gesture's base. A rejection (any `scene_edit`
    /// error, a target outside the gesture's, an oversized scene) returns before the gesture is
    /// touched, so the previous good candidate is still the one that commit would apply.
    fn gesture_update(&mut self, id: &str, ops: Vec<EditOp>) -> Result<Value, CommandError> {
        let gesture = self.scene_interaction.gesture.find(id)?;
        self.ensure_live(gesture)?;
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
            (
                Some((Arc::new(candidate), summary)),
                changes,
                modified,
                count,
            )
        };

        let gesture = self.scene_interaction.gesture.find_mut(id)?;
        gesture.set_candidate(ops, stored);
        Ok(json!({
            "action": "scene.gesture.update", "gesture": id, "scene": scene,
            "updates": gesture.updates(),
            "would_change": would_change, "changed_ops": changed_ops, "changes": changes,
            "changed": true, "document_changed": false,
        }))
    }

    /// Applies the gesture as ONE `scene.edit` batch. The gesture leaves the slot before the
    /// edit runs and goes back if the edit is rejected, so a rejection keeps the gesture (and
    /// everything else) exactly as it was and the caller can update or cancel.
    fn gesture_commit(&mut self, id: &str) -> Result<Value, CommandError> {
        let gesture = self.scene_interaction.gesture.find(id)?;
        self.ensure_live(gesture)?;
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

    fn gesture_cancel(&mut self, id: &str) -> Result<Value, CommandError> {
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

    fn gesture_status(&self) -> Value {
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
