//! Carrying directed scenes from another project (BL011), keeping every reference and every
//! file apart.
//!
//! `project.import` merges *sequences* and says scenes belong to the project, not to a
//! sequence; this is the companion for the scenes themselves (`project.import_scenes`):
//!
//! * **IDs.** A scene keeps its ID unless the destination already uses it (compared without
//!   letter case, because the ID is also a folder name), or a folder of that name already sits in
//!   `<workspace>/scenes/`; then it gets the first free `<id>-<n>`. The map from source ID to
//!   destination ID is the result.
//! * **References.** A bundle is self-contained: asset IDs, node IDs and instances refer to each
//!   other *inside* it, so the bundle JSON is attached byte-for-byte as data. Nothing needs
//!   rewriting, and nothing outside the bundle can name a scene by ID except the project's own
//!   commands, which the import cannot invalidate.
//! * **Paths.** Every scene gets its own managed folder, `scenes/<destId>/`, so two scenes that
//!   both declare `assets/backdrop.png` with different content never meet (the collision the
//!   manifest would otherwise report inside one folder).
//! * **Files.** Copied by `adopt`: verified against the bundle hash, staged, rolled back on any
//!   failure, resumable (reused when already there). If a later scene fails, the folders this
//!   import already created are removed again.
//!
//! The project change is one transaction of `scene.attach` commands (one undo step); the files
//! are prepared on a job thread first, like every other import.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::jobs::{CancelToken, JobError};
use tessel_editor_core::scene_project::{SceneCommand, SceneSource};
use tessel_editor_core::scene_resources::ResourceManifest;
use tessel_editor_core::state::Command;
use tessel_editor_core::{CommandError, EditorState};

use super::adopt::adopt;
use super::base::{self, MANAGED_DIR, ResourceBase};
use crate::directed_resources::{Options, Progress};

/// One scene of the import and where it will live.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedScene {
    pub from: String,
    pub to: String,
    pub name: String,
}

impl PlannedScene {
    pub fn renamed(&self) -> bool {
        self.from != self.to
    }
}

/// A scene whose files are already in the destination workspace, ready to attach.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedScene {
    pub planned: PlannedScene,
    /// The source bundle, untouched.
    pub bundle: Value,
    pub files: usize,
    pub copied: u64,
    pub reused: usize,
    /// Declared files that stay behind (preserved sections, unreferenced assets).
    pub not_carried: usize,
    /// Set when this import created the folder (and so may remove it again).
    created_root: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreparedSceneImport {
    pub project: String,
    pub scenes: Vec<PreparedScene>,
    pub warnings: Vec<String>,
    scenes_dir_created: Option<PathBuf>,
}

impl PreparedSceneImport {
    pub fn to_json(&self) -> Value {
        json!({
            "scenes": self.scenes.iter().map(|scene| json!({
                "from": scene.planned.from, "to": scene.planned.to,
                "renamed": scene.planned.renamed(),
                "files": scene.files, "copiedBytes": scene.copied,
                "reused": scene.reused, "notCarried": scene.not_carried,
            })).collect::<Vec<_>>(),
            "warnings": self.warnings,
        })
    }

    /// Removes the folders this import created. For a result that will not be applied.
    pub fn discard(self) {
        remove_created(&self);
    }
}

fn taken_folders(workspace: &Path) -> Vec<String> {
    fs::read_dir(workspace.join(MANAGED_DIR))
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().to_lowercase())
                .collect()
        })
        .unwrap_or_default()
}

/// First free ID: the wanted one, then `<wanted>-2`, `-3`, ... within the ID length limit.
fn free_id(wanted: &str, is_taken: impl Fn(&str) -> bool) -> String {
    if !is_taken(wanted) {
        return wanted.to_string();
    }
    (2..)
        .map(|n| {
            let suffix = format!("-{n}");
            let keep = tessel_editor_core::id::MAX_ID_LEN.saturating_sub(suffix.len());
            let head: String = wanted.chars().take(keep).collect();
            format!("{head}{suffix}")
        })
        .find(|candidate| !is_taken(candidate))
        .unwrap_or_else(|| wanted.to_string())
}

fn select<'a>(
    source: &'a EditorState,
    wanted: &[String],
) -> Result<Vec<&'a tessel_editor_core::scene_project::AttachedScene>, JobError> {
    if wanted.is_empty() {
        return Ok(source.scenes.iter().collect());
    }
    wanted
        .iter()
        .map(|id| {
            source
                .scenes
                .get(id)
                .ok_or_else(|| JobError::from(CommandError::unknown_element("scene", id)))
        })
        .collect()
}

/// Maps source scene IDs to free destination IDs without touching anything.
pub fn plan(
    destination: &EditorState,
    source: &EditorState,
    wanted: &[String],
) -> Result<Vec<PlannedScene>, JobError> {
    let chosen = select(source, wanted)?;
    let folders = destination
        .project
        .workspace
        .as_deref()
        .map(taken_folders)
        .unwrap_or_default();
    let mut reserved: Vec<String> = Vec::new();
    let mut planned = Vec::new();
    for scene in chosen {
        base::folder_name(&scene.id)?;
        let to = free_id(&scene.id, |candidate| {
            let folded = candidate.to_lowercase();
            destination
                .scenes
                .ids()
                .iter()
                .any(|id| id.to_lowercase() == folded)
                || reserved.contains(&folded)
                || folders.contains(&folded)
        });
        reserved.push(to.to_lowercase());
        planned.push(PlannedScene {
            from: scene.id.clone(),
            to,
            name: scene.name.clone(),
        });
    }
    Ok(planned)
}

/// Copies the resources of the chosen scenes into `workspace` (the destination's) and returns
/// what [`apply`] needs. `source_bases` maps source scene IDs to an absolute folder the caller
/// authorizes; without one the source project's managed base is used. The destination project
/// is not changed, and on any error neither is the workspace.
pub fn prepare(
    destination: &EditorState,
    source: &EditorState,
    workspace: &Path,
    wanted: &[String],
    source_bases: &BTreeMap<String, PathBuf>,
    token: &CancelToken,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<PreparedSceneImport, JobError> {
    let planned = plan(destination, source, wanted)?;
    let scenes_dir = workspace.join(MANAGED_DIR);
    let scenes_dir_created = (!scenes_dir.exists()).then_some(scenes_dir);
    let mut prepared = PreparedSceneImport {
        project: destination.project.id.clone(),
        scenes: Vec::new(),
        warnings: Vec::new(),
        scenes_dir_created,
    };
    for scene in planned {
        token.check().inspect_err(|_| remove_created(&prepared))?;
        match prepare_scene(source, workspace, &scene, source_bases, token, on_progress) {
            Ok(done) => {
                if done.not_carried > 0 {
                    prepared.warnings.push(format!(
                        "scene `{}` names {} file(s) that no version carries yet (preserved sections, unreferenced assets); the scene arrives with them listed, not copied",
                        done.planned.from, done.not_carried
                    ));
                }
                prepared.scenes.push(done);
            }
            Err(error) => {
                remove_created(&prepared);
                return Err(error);
            }
        }
    }
    Ok(prepared)
}

fn prepare_scene(
    source: &EditorState,
    workspace: &Path,
    planned: &PlannedScene,
    source_bases: &BTreeMap<String, PathBuf>,
    token: &CancelToken,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<PreparedScene, JobError> {
    let attached = source
        .scenes
        .get(&planned.from)
        .ok_or_else(|| JobError::from(CommandError::unknown_element("scene", &planned.from)))?;
    let scene: &DirectedScene = &attached.scene;
    let manifest = ResourceManifest::from_scene(scene);
    let bundle = scene.document().clone();
    let not_carried = manifest
        .entries()
        .filter(|entry| !entry.is_supported())
        .count()
        + manifest
            .uninterpreted()
            .iter()
            .map(|section| section.files.len())
            .sum::<usize>();
    let needs_files = manifest.supported().next().is_some();
    if !needs_files {
        // Data only: attach it, there is no folder to create.
        return Ok(PreparedScene {
            planned: planned.clone(),
            bundle,
            files: 0,
            copied: 0,
            reused: 0,
            not_carried,
            created_root: None,
        });
    }
    let base = match source_bases.get(&planned.from) {
        Some(root) => ResourceBase::explicit(root)?,
        None => {
            let workspace = source.project.workspace.as_deref().ok_or_else(|| {
                unavailable(
                    &planned.from,
                    "the source project has no workspace and no base was given",
                )
            })?;
            ResourceBase::managed(workspace, &planned.from)?
        }
    };
    if !base.exists() {
        return Err(unavailable(
            &planned.from,
            &format!(
                "its resource base `{}` does not exist",
                base.root().display()
            ),
        ));
    }
    let target = ResourceBase::managed(workspace, &planned.to)?;
    let existed = target.root().exists();
    let adopted = adopt(
        scene,
        &planned.to,
        workspace,
        &base,
        &Options::default(),
        token,
        on_progress,
        &mut |_| {},
    )?;
    Ok(PreparedScene {
        planned: planned.clone(),
        bundle,
        files: adopted.report.items.len(),
        copied: adopted.report.bytes_copied,
        reused: adopted.report.reused(),
        not_carried,
        created_root: (!existed).then(|| adopted.base.root().to_path_buf()),
    })
}

fn unavailable(scene: &str, why: &str) -> JobError {
    JobError::new(
        "missing_media",
        format!("the resources of scene `{scene}` cannot be read: {why}"),
    )
    .with_details(json!({ "scenes": [scene] }))
}

fn remove_created(prepared: &PreparedSceneImport) {
    for scene in &prepared.scenes {
        if let Some(root) = &scene.created_root {
            let _ = fs::remove_dir_all(root);
        }
    }
    if let Some(dir) = &prepared.scenes_dir_created {
        let _ = fs::remove_dir(dir);
    }
}

/// Attaches the prepared scenes as one undo step. A rejection (scene limit, byte budget) leaves
/// the project as it was and removes the folders [`prepare`] created.
pub fn apply(
    state: &mut EditorState,
    prepared: PreparedSceneImport,
) -> Result<Value, CommandError> {
    if state.project.id != prepared.project {
        prepared.discard();
        return Err(CommandError::new(
            "stale_result",
            "The open project changed while the scenes were imported; nothing was attached",
        ));
    }
    let outcome = state.transaction("Import scenes", |state| {
        for scene in &prepared.scenes {
            state.execute(Command::Scene(SceneCommand::Attach {
                id: Some(scene.planned.to.clone()),
                name: Some(scene.planned.name.clone()),
                source: SceneSource::Bundle(scene.bundle.clone()),
            }))?;
        }
        Ok(())
    });
    if let Err(error) = outcome {
        remove_created(&prepared);
        return Err(error);
    }
    Ok(
        json!({ "action": "project.import_scenes", "changed": !prepared.scenes.is_empty(),
        "result": prepared.to_json() }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_free_id_keeps_its_name_and_a_taken_one_gets_the_first_free_suffix() {
        assert_eq!(free_id("street", |_| false), "street");
        assert_eq!(free_id("street", |id| id == "street"), "street-2");
        assert_eq!(
            free_id("street", |id| matches!(
                id,
                "street" | "street-2" | "street-3"
            )),
            "street-4"
        );
    }

    #[test]
    fn a_long_id_loses_its_tail_before_the_suffix_and_stays_within_the_limit() {
        let long = "a".repeat(tessel_editor_core::id::MAX_ID_LEN);
        let free = free_id(&long, |id| id == long);
        assert!(free.len() <= tessel_editor_core::id::MAX_ID_LEN);
        assert!(free.ends_with("-2"));
        assert_ne!(free, long);
    }
}
