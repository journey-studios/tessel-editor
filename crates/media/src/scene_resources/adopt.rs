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
//!
//! BL089: a successful adoption also says what it created ([`Adoption::created`]: the files it
//! copied and the folders it made, never a reused file or a folder that was already there). A
//! caller whose result may still be discarded (the job finished but the project or the scene
//! changed meanwhile) moves that record into a [`Provisional`] with [`Adoption::provisional`] and
//! either releases it when the result is accepted or lets it remove exactly those files.
//!
//! BL096: the whole call runs under the root lease, and everything it reuses that another
//! *pending* result copied is claimed before the lease is released (`join_file`/`join_dir`), so a
//! discard of that other result can neither slip in between "reused" and "relied on" nor remove
//! the file later while this one is accepted. A bare `Adoption` that is dropped counts as
//! accepted.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::jobs::{CancelToken, JobError};
use tessel_editor_core::scene_resources::{ResourceManifest, ResourcePath};
use tessel_editor_core::{CommandError, EditorState};

use super::base::{self, MANAGED_DIR, ResourceBase};
use super::claims;
use super::cleanup::{CleanupReport, Created, Provisional, same_folder};
use crate::directed_resources::{
    MaterializeReport, Options, Outcome, Progress, Published, RootLease, blocked_by_manifest,
    materialize_leased,
};
use crate::fsx::io_error;

/// Not `Clone`: `created` is the removable record of one adoption (see [`Provisional`]). Dropped
/// without calling [`Adoption::provisional`] it is an accepted result: its files stay.
#[derive(Debug)]
pub struct Adoption {
    /// Where the resources are now (managed).
    pub base: ResourceBase,
    pub report: MaterializeReport,
    /// What this call wrote: the copied files and the folders it made. Empty for a data-only
    /// scene and for a relink that found every file already in place.
    pub created: Created,
}

impl Adoption {
    /// Moves the record of what this adoption created into an owner that removes it unless it is
    /// released. Call it once; the adoption keeps describing `base` and `report`.
    pub fn provisional(&mut self) -> Provisional {
        Provisional::new(std::mem::take(&mut self.created))
    }
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
            created: Created::default(),
        });
    }
    base::check_managed_destination(workspace, scene_id)?;
    // Declared before the lease on purpose: on every exit (an error, a cancellation, a panic) the
    // lease is released first and then this owner removes what the call created, which takes the
    // lease again. A success moves the record out with `release`.
    let mut owned = Provisional::default();
    create_folders(workspace, &managed, owned.created_mut())?;
    let lease = RootLease::acquire(managed.root())?;
    let canonical = lease.root().to_path_buf();
    let hashes: HashMap<&ResourcePath, &str> = plan
        .items
        .iter()
        .map(|item| (&item.path, item.expected_hash.as_str()))
        .collect();
    // Folders below the root that exist before publishing are not ours; `materialize` makes the
    // missing ones and does not say which, so the difference is taken here, under the lease so
    // that no other adoption can make one in between.
    let mut existing = existing_ancestors(
        managed.root(),
        &canonical,
        plan.items.iter().map(|item| &item.path),
    );
    let report = materialize_leased(
        &manifest,
        source.root(),
        &lease,
        options,
        token,
        on_progress,
        &mut |published| {
            let hash = hashes.get(published.path).copied().unwrap_or_default();
            record_published(
                owned.created_mut(),
                &mut existing,
                managed.root(),
                &canonical,
                published.path,
                hash,
            );
            on_published(published);
        },
    )?;
    // Still under the lease: what was reused from another pending result, and the folders it
    // made that this call put files into, are relied on from here on (BL096).
    for item in &report.items {
        if item.outcome == Outcome::Reused {
            owned
                .created_mut()
                .join_file(managed.root(), &canonical, &item.path);
        }
        for (folder, key) in folders_of(managed.root(), &canonical, &item.path) {
            owned.created_mut().join_dir(folder, key);
        }
    }
    Ok(Adoption {
        base: managed,
        report,
        created: owned.release(),
    })
}

/// The project, the workspace and the scene an adoption was started for. Taken on the caller's
/// thread when the job is created, so a job that finishes after any of them changed can be told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdoptionTarget {
    project: String,
    /// The workspace the files are copied into, which is where the scene's managed base is
    /// derived from: the same project ID opened from a copy of the folder is another place.
    workspace: Option<PathBuf>,
    scene: String,
    /// `None` when the scene was not attached at that moment (nothing could have been adopted
    /// for it, so any later state is a mismatch).
    bundle_revision: Option<String>,
}

impl AdoptionTarget {
    pub fn of(state: &EditorState, scene: &str) -> Self {
        Self {
            project: state.project.id.clone(),
            workspace: state.project.workspace.clone(),
            scene: scene.to_string(),
            bundle_revision: bundle_revision(state, scene),
        }
    }

    /// Why the result no longer fits, or `None` when project, workspace and scene are as they were.
    fn mismatch(&self, state: &EditorState) -> Option<&'static str> {
        if state.project.id != self.project {
            return Some("another project is open now");
        }
        let same_workspace = self
            .workspace
            .as_deref()
            .is_some_and(|workspace| same_folder(state.project.workspace.as_deref(), workspace));
        if !same_workspace {
            return Some("the project is open from another workspace now");
        }
        if self.bundle_revision.is_none()
            || bundle_revision(state, &self.scene) != self.bundle_revision
        {
            return Some("the scene was detached or its bundle replaced");
        }
        None
    }
}

fn bundle_revision(state: &EditorState, scene: &str) -> Option<String> {
    state
        .scenes
        .get(scene)
        .map(|attached| attached.scene.bundle().revision_id.clone())
}

/// A finished adoption whose files belong to nobody until [`PendingAdoption::apply`] confirms the
/// project, its workspace and the scene are still the ones it was started for (BL089). Dropping
/// it unapplied removes what the adoption copied that no other pending result relies on (BL096),
/// and only that. Not `Clone`.
#[derive(Debug)]
pub struct PendingAdoption {
    target: AdoptionTarget,
    owned: Provisional,
}

impl PendingAdoption {
    /// Takes over what `adoption` created.
    pub fn new(target: AdoptionTarget, adoption: &mut Adoption) -> Self {
        Self {
            target,
            owned: adoption.provisional(),
        }
    }

    /// Keeps the files when the project, its workspace and the scene still match; otherwise
    /// removes them and answers `stale_result`. An adoption changes no project data, so nothing
    /// else is applied.
    ///
    /// The removal reads and hashes every file it takes back, so a caller that must not block
    /// (the window's tick) uses [`PendingAdoption::settle`] and removes from a job instead.
    pub fn apply(self, state: &EditorState) -> Result<(), CommandError> {
        match self.settle(state) {
            Ok(()) => Ok(()),
            Err(stale) => {
                let StaleAdoption { why, pending } = *stale;
                let scene = pending.target.scene.clone();
                let report = pending.discard();
                let mut kept = String::new();
                if !report.shared.is_empty() {
                    kept.push_str(&format!(
                        "; {} item(s) stay because another result relies on them",
                        report.shared.len()
                    ));
                }
                if !report.kept.is_empty() {
                    kept.push_str(&format!("; kept: {}", report.kept.join("; ")));
                }
                Err(CommandError::new(
                    "stale_result",
                    format!(
                        "Scene `{scene}` changed while its resources were adopted ({why}); the {} file(s) this adoption copied were removed{kept}",
                        report.removed_files
                    ),
                ))
            }
        }
    }

    /// [`apply`](PendingAdoption::apply) without the removal: accepted, the files are the
    /// project's; refused, the adoption comes back **still owning its files** so the caller can
    /// remove them wherever it may block (BL094, BL112). Reads no file.
    pub fn settle(mut self, state: &EditorState) -> Result<(), Box<StaleAdoption>> {
        match self.target.mismatch(state) {
            None => {
                // Dropping the released record is the acceptance: the files are the project's.
                drop(self.owned.release());
                Ok(())
            }
            Some(why) => Err(Box::new(StaleAdoption { why, pending: self })),
        }
    }

    /// The scene this adoption was started for.
    pub fn scene(&self) -> &str {
        &self.target.scene
    }

    /// Removes what the adoption created without looking at the project.
    pub fn discard(self) -> CleanupReport {
        self.owned.discard()
    }
}

/// An adoption the project refused ([`PendingAdoption::settle`]). It still owns what it copied:
/// dropping or discarding it removes those files, which is the work to do off the UI thread.
#[derive(Debug)]
pub struct StaleAdoption {
    /// Why it no longer fits the project, in words.
    pub why: &'static str,
    pub pending: PendingAdoption,
}

/// The folders of `paths` below `root` that exist now.
fn existing_ancestors<'a>(
    root: &Path,
    canonical_root: &Path,
    paths: impl Iterator<Item = &'a ResourcePath>,
) -> HashSet<PathBuf> {
    let mut found = HashSet::new();
    for path in paths {
        for (folder, _) in folders_of(root, canonical_root, path) {
            if fs::symlink_metadata(&folder).is_ok_and(|metadata| metadata.is_dir()) {
                found.insert(folder);
            }
        }
    }
    found
}

/// Folders between `root` and the file of `path`, shallowest first: each as the caller spells it
/// and as the claims registry keys it (below the canonical root).
fn folders_of(root: &Path, canonical_root: &Path, path: &ResourcePath) -> Vec<(PathBuf, PathBuf)> {
    let segments: Vec<&str> = path.segments().collect();
    let (mut spelled, mut canonical) = (root.to_path_buf(), canonical_root.to_path_buf());
    segments[..segments.len() - 1]
        .iter()
        .map(|segment| {
            spelled.push(segment);
            canonical.push(segment);
            (spelled.clone(), canonical.clone())
        })
        .collect()
}

/// Records the file just published and the folders it needed that did not exist before.
fn record_published(
    created: &mut Created,
    existing: &mut HashSet<PathBuf>,
    root: &Path,
    canonical_root: &Path,
    path: &ResourcePath,
    sha256: &str,
) {
    for (folder, key) in folders_of(root, canonical_root, path) {
        if existing.insert(folder.clone()) {
            created.dir(folder, key);
        }
    }
    created.file(root, canonical_root, path, sha256);
}

/// Creates `scenes/` and `scenes/<id>/` one folder at a time and records the ones that did not
/// exist (in creation order). A folder that is there already is relied on if another pending
/// result made it. Creating and recording share one registry lock: another adoption that finds
/// the folder "already there" then also finds its entry.
fn create_folders(
    workspace: &Path,
    managed: &ResourceBase,
    created: &mut Created,
) -> Result<(), JobError> {
    let mut tracker = claims::lock();
    for folder in [workspace.join(MANAGED_DIR), managed.root().to_path_buf()] {
        match fs::create_dir(&folder) {
            Ok(()) => {
                let key = canonical_or_spelled(&folder);
                created.dir_with(&mut tracker, folder, key);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let key = canonical_or_spelled(&folder);
                created.join_dir_with(&mut tracker, folder, key);
            }
            // What was made so far is removed by the caller's owner when it drops.
            Err(error) => return Err(io_error(&folder, &error)),
        }
    }
    Ok(())
}

fn canonical_or_spelled(folder: &Path) -> PathBuf {
    fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf())
}
