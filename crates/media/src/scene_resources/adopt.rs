//! Adoption: copying a scene's resources from where it was authored (an explicit base) into the
//! project's managed base, `<workspace>/scenes/<sceneId>/`.
//!
//! After adoption the project no longer depends on the origin: the files are inside the
//! workspace, verified against the bundle's hashes, and every later operation (load, package,
//! restore, interchange) finds them by the same derived base. It is also the **relink**: run it
//! again with another source folder and whatever already sits at the destination with the
//! expected hash is reused, missing files are copied, and a file with other content is a
//! `destination_conflict` that is never overwritten.
//!
//! All the work is `directed_resources::materialize_observed`, so adoption inherits staging,
//! per-file cancellation with rollback of only its own files (BL080) and the exclusive root
//! lease (BL079). Folders this call creates (`scenes/`, `scenes/<id>/`) are removed again when it
//! fails, so a refused adoption leaves the workspace as it was.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::jobs::{CancelToken, JobError};
use tessel_editor_core::scene_resources::ResourceManifest;

use super::base::{self, MANAGED_DIR, ResourceBase};
use crate::directed_resources::{
    MaterializeReport, Options, Progress, Published, blocked_by_manifest, materialize_observed,
};
use crate::fsx::io_error;

#[derive(Debug, Clone, PartialEq)]
pub struct Adoption {
    /// Where the resources are now (managed).
    pub base: ResourceBase,
    pub report: MaterializeReport,
}

/// Copies the supported resources of `scene` from `source` into the managed base of `scene_id`.
///
/// Errors leave the workspace as it was: `manifest_blocked`, `resources_invalid`,
/// `root_unavailable`, `root_busy`, `scene_id_collision`, `scene_id_not_portable`,
/// `destination_conflict`, `disk_full`, `cancelled`, `io_error`.
#[allow(clippy::too_many_arguments)]
pub fn adopt(
    scene: &DirectedScene,
    scene_id: &str,
    workspace: &Path,
    source: &ResourceBase,
    options: &Options,
    token: &CancelToken,
    on_progress: &mut dyn FnMut(Progress),
    on_published: &mut dyn FnMut(Published<'_>),
) -> Result<Adoption, JobError> {
    let manifest = ResourceManifest::from_scene(scene);
    let managed = ResourceBase::managed(workspace, scene_id)?;
    let plan = manifest
        .transport_plan()
        .map_err(|issues| blocked_by_manifest(&issues))?;
    if plan.items.is_empty() {
        // A data-only scene needs no folder: creating an empty one would be a mutation for nothing.
        return Ok(Adoption {
            base: managed,
            report: MaterializeReport {
                items: Vec::new(),
                skipped: plan.skipped,
                complete: manifest.is_complete(),
                bytes_copied: 0,
            },
        });
    }
    base::check_managed_destination(workspace, scene_id)?;
    let created = create_folders(workspace, &managed)?;
    let outcome = materialize_observed(
        &manifest,
        source.root(),
        managed.root(),
        options,
        token,
        on_progress,
        on_published,
    );
    match outcome {
        Ok(report) => Ok(Adoption {
            base: managed,
            report,
        }),
        Err(error) => {
            remove_created(&created);
            Err(error)
        }
    }
}

/// Creates `scenes/` and `scenes/<id>/` one folder at a time and returns the ones that did not
/// exist, in creation order.
fn create_folders(workspace: &Path, managed: &ResourceBase) -> Result<Vec<PathBuf>, JobError> {
    let mut created = Vec::new();
    for folder in [workspace.join(MANAGED_DIR), managed.root().to_path_buf()] {
        match fs::create_dir(&folder) {
            Ok(()) => created.push(folder),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                remove_created(&created);
                return Err(io_error(&folder, &error));
            }
        }
    }
    Ok(created)
}

/// Deepest first; a folder that is not empty (the user's, or one a failed rollback could not
/// clear) is left alone: `remove_dir` refuses it.
fn remove_created(created: &[PathBuf]) {
    for folder in created.iter().rev() {
        let _ = fs::remove_dir(folder);
    }
}
