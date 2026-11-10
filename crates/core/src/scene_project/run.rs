//! Executing the `scene.*` commands that write (attach, detach, edit) and routing the ones that
//! only read to `query.rs`.
//!
//! Every write validates completely before it touches the project: a rejection, a dry run and a
//! no-op leave `EditorState` (and so its history and revision) exactly as they were. The history
//! entry itself is not made here: `EditorState::execute` snapshots before and records after when
//! the outcome says `changed`, so there is one history and one transaction per command.

use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use serde_json::{Map, Value, json};

use crate::directed_scene::{DirectedScene, ImportError, Limits};
use crate::jobs::CancelToken;
use crate::scene_edit::{self, EditOp, EditSummary};
use crate::scene_interaction::Dropped;
use crate::state::{CommandError, EditorState};

use super::AttachedScene;
use super::command::{AdmittedScene, SceneCommand, SceneSource};
use super::query;
use super::scope::scope_json;

/// The gates a scene must pass to live in a project: the importer already ran (that is how a
/// `DirectedScene` exists), and the structural and temporal gates run here, the same ones
/// `scene_edit::apply` runs on every candidate. WHY here as well: `scene_edit` checks what an
/// *edit* produces; a bundle that arrives by attach or from a document has not been through them,
/// and a scene that fails them could be edited into a state nobody can undo into.
pub fn admit(scene: DirectedScene) -> Result<DirectedScene, CommandError> {
    check_admission(&scene)?;
    Ok(scene)
}

pub(super) fn check_admission(scene: &DirectedScene) -> Result<(), CommandError> {
    let mut issues = scene.edit_join_issues();
    issues.extend(scene.temporal_issues());
    if issues.is_empty() {
        return Ok(());
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
/// importer enforces the same bound, but only after the text is already in memory. Reads in
/// blocks so a job can be cancelled while a large file is still coming in.
fn read_bundle_file(path: &Path, token: &CancelToken) -> Result<String, CommandError> {
    const BLOCK: usize = 1 << 20;
    let max = Limits::default().max_bytes;
    let meta = fs::metadata(path).map_err(|error| unreadable(path, &error))?;
    let bytes = usize::try_from(meta.len()).unwrap_or(usize::MAX);
    if bytes > max {
        return Err(ImportError::TooLarge { bytes, max }.into());
    }
    let mut file = File::open(path).map_err(|error| unreadable(path, &error))?;
    let mut data = Vec::with_capacity(bytes.min(max));
    let mut block = vec![0_u8; BLOCK];
    loop {
        token.check().map_err(CommandError::from)?;
        let read = file
            .read(&mut block)
            .map_err(|error| unreadable(path, &error))?;
        if read == 0 {
            break;
        }
        data.extend_from_slice(&block[..read]);
        // The file may have grown after the size check.
        if data.len() > max {
            return Err(ImportError::TooLarge {
                bytes: data.len(),
                max,
            }
            .into());
        }
    }
    String::from_utf8(data).map_err(|error| {
        unreadable(
            path,
            &std::io::Error::new(std::io::ErrorKind::InvalidData, error),
        )
    })
}

impl SceneSource {
    /// Reads and imports the bundle. `token` only matters for a file: it is checked between
    /// blocks, so a cancelled job stops reading instead of finishing a large file.
    fn import(self, token: &CancelToken) -> Result<DirectedScene, CommandError> {
        let scene = match self {
            Self::Bundle(value) => DirectedScene::import_value(value),
            Self::Text(text) => DirectedScene::import_str(&text),
            Self::Path(path) => DirectedScene::import_str(&read_bundle_file(&path, token)?),
            Self::Admitted(admitted) => return Ok(admitted.scene().clone()),
        };
        Ok(scene?)
    }
}

impl AdmittedScene {
    /// Everything attach does that costs time: read, parse, import, then the structural and
    /// temporal gates. It touches no project, so it may run on a job thread; attaching the result
    /// (`SceneSource::Admitted`) is cheap. `token` is checked between phases (and between blocks
    /// of a file): a cancelled preparation fails with the `cancelled` code and holds no scene.
    /// Errors are the ones `scene.attach` answers (`bundle_unreadable`, `invalid_bundle`, ...).
    pub fn prepare(source: SceneSource, token: &CancelToken) -> Result<Self, CommandError> {
        let cancelled = || token.check().map_err(CommandError::from);
        if let SceneSource::Admitted(admitted) = source {
            return Ok(admitted);
        }
        cancelled()?;
        let scene = source.import(token)?;
        cancelled()?;
        let scene = admit(scene)?;
        cancelled()?;
        Ok(Self::new(scene))
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

/// Adds `conversions` to an edit answer when the batch turned movements into keyframes. Absent
/// otherwise, so the answer of every other batch keeps the shape it always had.
pub(super) fn with_conversions(mut answer: Value, summary: &EditSummary) -> Value {
    if !summary.conversions.is_empty() {
        let conversions: Vec<Value> = summary
            .conversions
            .iter()
            .map(|conversion| {
                json!({
                    "index": conversion.index, "clip": conversion.clip,
                    "segment": conversion.segment, "recipe": conversion.recipe,
                    "keys": conversion.keys,
                })
            })
            .collect();
        answer["conversions"] = Value::Array(conversions);
    }
    answer
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
            SceneCommand::FxEdit {
                scene,
                ops,
                dry_run,
                base_revision,
            } => {
                self.ensure_base_revision(base_revision)?;
                self.edit_fx(&scene, &ops, dry_run)
            }
            SceneCommand::FxSeek {
                scene,
                clip,
                param,
                to,
            } => self.fx_seek(&scene, &clip, &param, &to),
            SceneCommand::FxList { scene, gesture } => self.fx_list(&scene, gesture.as_deref()),
            SceneCommand::FxFrame { scene, at, gesture } => {
                self.fx_frame(&scene, &at, gesture.as_deref())
            }
            SceneCommand::FxInspect {
                scene,
                clip,
                gesture,
                locale,
                probe,
                at,
            } => self.fx_inspect(
                &scene,
                clip.as_deref(),
                gesture.as_deref(),
                locale,
                probe.as_ref(),
                at.as_ref(),
            ),
            SceneCommand::FxGesture(gesture) => self.apply_fx_gesture(gesture),
            SceneCommand::Gesture(gesture) => self.apply_gesture(gesture),
            SceneCommand::Gizmo(gizmo) => self.apply_gizmo(gizmo),
            read => query::run(&self.scenes, read),
        }
    }

    fn attach_scene(
        &mut self,
        id: Option<String>,
        name: Option<String>,
        source: SceneSource,
    ) -> Result<Value, CommandError> {
        self.scenes.check_attach(id.as_deref(), name.as_deref())?;
        let id = id.unwrap_or_else(|| self.scenes.fresh_id());
        let name = name.map(|raw| raw.trim().to_string());
        // One door for every source: a bundle that was not prepared elsewhere is prepared here,
        // by the same function a job thread runs, so the gates and the facts cannot differ.
        let admitted = match source {
            SceneSource::Admitted(admitted) => admitted,
            source => AdmittedScene::prepare(source, &CancelToken::new())?,
        };
        let (scene, facts) = admitted.into_parts();
        let name = name.unwrap_or_else(|| scene.bundle().bundle_id.clone());
        let mut report = json!({
            "action": "scene.attach", "scene": id, "name": name,
            "bundle_id": scene.bundle().bundle_id, "revision_id": scene.bundle().revision_id,
            "scope": scope_json(),
            "changed": true,
        });
        // Moved in, not copied through `json!`: the lists can hold one entry per node.
        report["counts"] = facts.counts;
        report["notices"] = Value::Array(facts.notices);
        report["deferred_in_use"] = Value::Array(facts.deferred_in_use);
        self.scenes.insert_measured(
            AttachedScene {
                id,
                name,
                scene,
                extra: Map::new(),
            },
            facts.serialized_len,
        )?;
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
            return Ok(with_conversions(
                json!({
                    "action": "scene.edit", "scene": id, "dry_run": true, "changed": false,
                    "would_change": summary.modified, "changed_ops": summary.changed_count(),
                    "changes": changes_json(&summary),
                }),
                &summary,
            ));
        }
        if !summary.modified {
            // WHY no replacement: a command whose net effect is nil must not dirty the project
            // nor leave an undo entry. The candidate is the input scene itself in that case (no
            // restored number spelled differently, no keys from a conversion made on the way).
            return Ok(with_conversions(
                json!({
                    "action": "scene.edit", "scene": id, "dry_run": false, "changed": false,
                    "noop": true, "changed_ops": summary.changed_count(),
                    "changes": changes_json(&summary),
                }),
                &summary,
            ));
        }
        self.scenes.replace_scene(id, outcome.candidate)?;
        Ok(with_conversions(
            json!({
                "action": "scene.edit", "scene": id, "dry_run": false, "changed": true,
                "noop": false, "changed_ops": summary.changed_count(),
                "changes": changes_json(&summary),
            }),
            &summary,
        ))
    }
}
