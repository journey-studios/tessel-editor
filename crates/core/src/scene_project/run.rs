//! Executing the `scene.*` commands that write (attach, detach, edit) and routing the ones that
//! only read to `query.rs`.
//!
//! Every write validates completely before it touches the project: a rejection, a dry run and a
//! no-op leave `EditorState` (and so its history and revision) exactly as they were. The history
//! entry itself is not made here: `EditorState::execute` snapshots before and records after when
//! the outcome says `changed`, so there is one history and one transaction per command.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use serde_json::{Map, Value, json};

use crate::directed_scene::{DirectedScene, ImportError, Limits};
use crate::scene_edit::{self, EditOp, EditSummary};
use crate::scene_interaction::Dropped;
use crate::state::{CommandError, EditorState};

use super::command::{SceneCommand, SceneSource};
use super::query;
use super::scope::{deferred_in_use, scope_json};
use super::{AttachedScene, check_scene_id};

/// The gates a scene must pass to live in a project: the importer already ran (that is how a
/// `DirectedScene` exists), and the structural and temporal gates run here, the same ones
/// `scene_edit::apply` runs on every candidate. WHY here as well: `scene_edit` checks what an
/// *edit* produces; a bundle that arrives by attach or from a document has not been through them,
/// and a scene that fails them could be edited into a state nobody can undo into.
pub fn admit(scene: DirectedScene) -> Result<DirectedScene, CommandError> {
    let mut issues = scene.edit_join_issues();
    issues.extend(scene.temporal_issues());
    if issues.is_empty() {
        return Ok(scene);
    }
    Err(ImportError::Invalid { issues }.into())
}

fn unreadable(path: &Path, error: &std::io::Error) -> CommandError {
    CommandError::new(
        "bundle_unreadable",
        format!("`{}`: {error}", path.display()),
    )
    .target(path.to_string_lossy())
}

/// Reads a bundle file, refusing a file larger than the importer's limit before reading it: the
/// importer enforces the same bound, but only after the text is already in memory.
fn read_bundle_file(path: &Path) -> Result<String, CommandError> {
    let max = Limits::default().max_bytes;
    let meta = fs::metadata(path).map_err(|error| unreadable(path, &error))?;
    let bytes = usize::try_from(meta.len()).unwrap_or(usize::MAX);
    if bytes > max {
        return Err(ImportError::TooLarge { bytes, max }.into());
    }
    fs::read_to_string(path).map_err(|error| unreadable(path, &error))
}

impl SceneSource {
    fn import(self) -> Result<DirectedScene, CommandError> {
        let scene = match self {
            Self::Bundle(value) => DirectedScene::import_value(value),
            Self::Text(text) => DirectedScene::import_str(&text),
            Self::Path(path) => DirectedScene::import_str(&read_bundle_file(&path)?),
        };
        Ok(scene?)
    }
}

pub(super) fn changes_json(summary: &EditSummary) -> Vec<Value> {
    summary
        .changes
        .iter()
        .map(|change| {
            json!({
                "index": change.index,
                "property": change.property.path(),
                "before": change.before.to_json(),
                "after": change.after.to_json(),
                "noop": change.is_noop(),
            })
        })
        .collect()
}

impl EditorState {
    /// Runs one `scene.*` command. Reached through `EditorState::execute`.
    pub(crate) fn apply_scene(&mut self, command: SceneCommand) -> Result<Value, CommandError> {
        match command {
            SceneCommand::Attach { id, name, source } => self.attach_scene(id, name, source),
            SceneCommand::Detach { scene } => self.detach_scene(&scene),
            SceneCommand::Edit {
                scene,
                ops,
                dry_run,
            } => self.edit_scene(&scene, &ops, dry_run),
            SceneCommand::Tree { scene, root, depth } => {
                self.scene_tree(&scene, root.as_deref(), depth)
            }
            SceneCommand::Evaluate { scene, at, gesture } => {
                self.scene_evaluate(&scene, &at, gesture.as_deref())
            }
            SceneCommand::Plan {
                scene,
                at,
                partial,
                gesture,
            } => self.scene_plan(&scene, &at, partial, gesture.as_deref()),
            SceneCommand::Select {
                scene,
                entities,
                mode,
            } => self.scene_select(scene.as_deref(), &entities, mode),
            SceneCommand::Selection => Ok(self.scene_selection()),
            SceneCommand::Gesture(gesture) => self.apply_gesture(gesture),
            read => query::run(&self.scenes, read),
        }
    }

    fn attach_scene(
        &mut self,
        id: Option<String>,
        name: Option<String>,
        source: SceneSource,
    ) -> Result<Value, CommandError> {
        let id = match id {
            Some(raw) => {
                check_scene_id(&raw)?;
                raw
            }
            None => self.scenes.fresh_id(),
        };
        self.scenes.check_room(&id)?;
        let name = match name.map(|raw| raw.trim().to_string()) {
            Some(trimmed) if trimmed.is_empty() => {
                return Err(CommandError::invalid_value(
                    "A scene name cannot be blank",
                    json!("a non-empty string"),
                ));
            }
            other => other,
        };
        let scene = admit(source.import()?)?;
        let name = name.unwrap_or_else(|| scene.bundle().bundle_id.clone());
        let report = json!({
            "action": "scene.attach", "scene": id, "name": name,
            "bundle_id": scene.bundle().bundle_id, "revision_id": scene.bundle().revision_id,
            "counts": query::counts(&scene),
            "notices": query::notices_json(&scene),
            "deferred_in_use": deferred_in_use(scene.document()),
            "scope": scope_json(),
            "changed": true,
        });
        self.scenes.insert(AttachedScene {
            id,
            name,
            scene: Arc::new(scene),
            extra: Map::new(),
        })?;
        Ok(report)
    }

    fn detach_scene(&mut self, id: &str) -> Result<Value, CommandError> {
        self.scenes
            .remove(id)
            .ok_or_else(|| CommandError::unknown_element("scene", id))?;
        let mut outcome = json!({ "action": "scene.detach", "scene": id, "changed": true });
        // Only when something was selected there, so the answer of a project that never
        // selected is what it always was. An open gesture on this scene is not touched: it
        // reports `scene_detached` the next time it is used.
        let dropped = self.scene_interaction.selection.prune(&self.scenes);
        if !dropped.is_empty() {
            outcome["selection_dropped"] =
                Value::Array(dropped.iter().map(Dropped::to_json).collect());
        }
        Ok(outcome)
    }

    /// `scene_edit` owns every rule here (catalog, IDs, importer, joins, temporal gates). This
    /// only decides what the project does with the answer.
    pub(super) fn edit_scene(
        &mut self,
        id: &str,
        ops: &[EditOp],
        dry_run: bool,
    ) -> Result<Value, CommandError> {
        let attached = self
            .scenes
            .get(id)
            .ok_or_else(|| CommandError::unknown_element("scene", id))?;
        let current = Arc::clone(&attached.scene);
        let outcome = scene_edit::apply(&current, ops)?;
        let summary = outcome.summary;
        if dry_run {
            if summary.modified {
                self.scenes.check_replacement_size(id, &outcome.candidate)?;
            }
            return Ok(json!({
                "action": "scene.edit", "scene": id, "dry_run": true, "changed": false,
                "would_change": summary.modified, "changed_ops": summary.changed_count(),
                "changes": changes_json(&summary),
            }));
        }
        if !summary.modified {
            // WHY not replace with the equal candidate: it may spell a restored number
            // differently (`1` vs `1.0`), and a command that changes nothing must not dirty the
            // project nor leave an undo entry.
            return Ok(json!({
                "action": "scene.edit", "scene": id, "dry_run": false, "changed": false,
                "noop": true, "changed_ops": summary.changed_count(),
                "changes": changes_json(&summary),
            }));
        }
        self.scenes.replace_scene(id, outcome.candidate)?;
        Ok(json!({
            "action": "scene.edit", "scene": id, "dry_run": false, "changed": true,
            "noop": false, "changed_ops": summary.changed_count(),
            "changes": changes_json(&summary),
        }))
    }
}
