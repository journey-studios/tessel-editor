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
//! lease (BL079). Failure attempts rollback of this call's publications and empty folders.
//! A proof or removal failure can leave files behind; claims failures report those in `kept`.
//!
//! BL089: a successful adoption also says what it created ([`Adoption::created`]: the files it
//! copied and the folders it made, never a reused file or a folder that was already there). A
//! caller whose result may still be discarded (the job finished but the project or the scene
//! changed meanwhile) moves that record into a [`Provisional`] with [`Adoption::provisional`] and
//! either accepts it ([`Provisional::accept`], fallible) or lets it remove exactly those files.
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

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::jobs::{CancelToken, JobError};
use tessel_editor_core::scene_resources::{ResourceManifest, ResourcePath};
use tessel_editor_core::{CommandError, EditorState};

use super::base::{self, MANAGED_DIR, ResourceBase};
use super::claims::Ledger;
use super::cleanup::{
    CleanupReport, Created, Provisional, Unclaimed, refused_accept, same_folder, with_cleanup,
};
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
    /// accepted. Call it once; the adoption keeps describing `base` and `report`.
    pub fn provisional(&mut self) -> Provisional {
        Provisional::new(std::mem::take(&mut self.created))
    }
}

/// Copies the supported resources of `scene` from `source` into the managed base of `scene_id`.
///
/// Errors refuse the adoption and attempt safe rollback: `manifest_blocked`, `resources_invalid`,
/// `root_unavailable`, `root_busy`, `scene_id_collision`, `scene_id_not_portable`,
/// `destination_conflict`, `disk_full`, `cancelled`, `io_error`, `claims_unavailable`.
///
/// `claims_unavailable` has two meanings, told apart by `details.phase`: `before_changes` (the
/// ledger could not be used, nothing was touched) and `after_changes` (it stopped working once
/// copying had begun: rollback of the unclaimed file was attempted, the rest was
/// discarded, and `details.takenBack`, `details.kept` and `details.cleanup` say what stays).
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
    // The owner is declared here, outside the call that holds the lease: on every exit (an
    // error, a cancellation, a panic) the lease is released first and then this owner removes
    // what the call created, which takes the lease again. A success moves the record out.
    let mut owned = Provisional::default();
    let ledger = owned.created_mut().bind(workspace);
    let copied = copy_and_claim(
        &plan.items,
        &manifest,
        &managed,
        workspace,
        &ledger,
        source,
        options,
        token,
        on_progress,
        on_published,
        owned.created_mut(),
    );
    match copied {
        Ok(report) => Ok(Adoption {
            base: managed,
            report,
            created: std::mem::take(owned.created_mut()),
        }),
        Err(error) => Err(settle(error, owned)),
    }
}

/// A failed adoption attempts rollback. A `claims_unavailable` one also says
/// what discarding the result did, because with a ledger that stopped working that is not always
/// everything: what cannot be released is kept and listed. A failed publication rollback may
/// leave an unclaimed file outside ledger recovery; see `kept`.
fn settle(error: JobError, owned: Provisional) -> JobError {
    if error.code != "claims_unavailable" {
        drop(owned);
        return error;
    }
    let report = owned.discard();
    with_cleanup(error, &report)
}

/// The part of an adoption that holds the root lease: folders, copies, claims. Returns with the
/// lease released, so the caller's cleanup of a failure can take it again.
#[allow(clippy::too_many_arguments)]
fn copy_and_claim(
    items: &[tessel_editor_core::scene_resources::TransportItem],
    manifest: &ResourceManifest,
    managed: &ResourceBase,
    workspace: &Path,
    ledger: &Ledger,
    source: &ResourceBase,
    options: &Options,
    token: &CancelToken,
    on_progress: &mut dyn FnMut(Progress),
    on_published: &mut dyn FnMut(Published<'_>),
    created: &mut Created,
) -> Result<MaterializeReport, JobError> {
    create_folders(workspace, managed, ledger, created)?;
    let lease = RootLease::acquire(managed.root())?;
    let canonical = lease.root().to_path_buf();
    let hashes: HashMap<&ResourcePath, &str> = items
        .iter()
        .map(|item| (&item.path, item.expected_hash.as_str()))
        .collect();
    // Folders below the root that exist before publishing are not ours; `materialize` makes the
    // missing ones and does not say which, so the difference is taken here, under the lease so
    // that no other adoption can make one in between.
    let mut existing =
        existing_ancestors(managed.root(), &canonical, items.iter().map(|i| &i.path));
    // What the ledger could not claim after it was copied. The callback cannot stop the copy, so
    // it takes each such file back at once (still under the lease) and the failure is raised
    // when the copy returns.
    let mut unclaimed: Option<Unclaimed> = None;
    let copied = materialize_leased(
        manifest,
        source.root(),
        &lease,
        options,
        token,
        on_progress,
        &mut |published| {
            let hash = hashes.get(published.path).copied().unwrap_or_default();
            if let Err(failure) = record_published(
                created,
                &mut existing,
                managed.root(),
                &canonical,
                published.path,
                hash,
            ) {
                for folder in &failure.forget {
                    existing.remove(folder);
                }
                match &mut unclaimed {
                    Some(earlier) => earlier.merge(failure),
                    None => unclaimed = Some(failure),
                }
            }
            on_published(published);
        },
    );
    let report = match copied {
        Ok(report) => report,
        // The copy rolled back whatever it had published, which no longer includes what was
        // taken back already: those are not leftovers. A cancellation stays a cancellation.
        Err(error) => {
            return Err(match unclaimed {
                Some(failure) if failure.left_behind() => failure.into_error(),
                Some(failure) => without_taken_back(error, &failure, managed.root()),
                None => error,
            });
        }
    };
    if let Some(failure) = unclaimed {
        return Err(failure.into_error());
    }
    // Still under the lease: what was reused from another pending result, and the folders it
    // made that this call put files into, are relied on from here on (BL096).
    for item in &report.items {
        if item.outcome == Outcome::Reused {
            created.join_file(managed.root(), &canonical, &item.path)?;
        }
        for (folder, key) in folders_of(managed.root(), &canonical, &item.path) {
            created.join_dir(folder, key)?;
        }
    }
    Ok(report)
}

/// `error` of the copy without the leftovers it lists for things this adoption already took back
/// (the copy's own rollback found them gone). The copy spells paths below the resolved root and
/// this adoption below the root as it was given, so they are compared below `root`.
fn without_taken_back(mut error: JobError, failure: &Unclaimed, root: &Path) -> JobError {
    let Some(details) = error.details.as_mut() else {
        return error;
    };
    let gone: Vec<String> = failure
        .taken_back
        .iter()
        .filter_map(|path| path.strip_prefix(root).ok())
        .map(|relative| format!("/{}`:", relative.display()))
        .collect();
    if let Some(Value::Array(leftovers)) = details.get_mut("rollbackLeftovers") {
        leftovers.retain(|line| {
            let line = line.as_str().unwrap_or_default();
            !gone.iter().any(|tail| line.contains(tail.as_str()))
        });
        if leftovers.is_empty()
            && let Some(map) = details.as_object_mut()
        {
            map.remove("rollbackLeftovers");
        }
    }
    error
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
    /// Captures the project, workspace and bundle revision that must still match at apply.
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

/// Reads the attached bundle revision, or None after the scene has been detached.
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
    /// Acceptance is explicit and fallible: the files are the project's only once the claims
    /// ledger says so. A ledger that refuses discards (safe whether or not the failed write
    /// reached the disk) and answers `claims_unavailable` with what that did.
    ///
    /// The removal reads and hashes every file it takes back, so a caller that must not block
    /// (the window's tick) uses [`PendingAdoption::settle`] and removes from a job instead.
    pub fn apply(self, state: &EditorState) -> Result<(), CommandError> {
        let Err(stale) = self.settle_waiting(state, true) else {
            return Ok(());
        };
        let StaleAdoption {
            why,
            refusal,
            pending,
        } = *stale;
        let scene = pending.target.scene.clone();
        let report = pending.discard();
        if let Some(error) = refusal {
            return Err(refused_accept(
                error,
                Some(&report),
                "the adoption was not confirmed",
            ));
        }
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

    /// [`apply`](PendingAdoption::apply) without the removal: accepted, the files are the
    /// project's; refused, the adoption comes back **still owning its files** (and still claimed)
    /// so the caller can remove them wherever it may block (BL094, BL112). Reads and hashes no
    /// resource file; accepting does read and write the workspace's `claims.json` under its
    /// `flock`, without waiting for it: this is the window's tick, so a lock that another
    /// process holds at that instant makes the ledger unavailable for this call (BL112.3)
    /// instead of holding the tick up.
    ///
    /// Two refusals: the project no longer fits (`refusal` is `None`), or the ledger could not
    /// record the acceptance (`refusal` is the `claims_unavailable` error and nothing was
    /// accepted). In both the result stays the owner until it is discarded.
    pub fn settle(self, state: &EditorState) -> Result<(), Box<StaleAdoption>> {
        self.settle_waiting(state, false)
    }

    /// [`settle`](PendingAdoption::settle); with `wait` the acceptance may wait (bounded) for
    /// the ledger lock, for a caller that is allowed to block (`apply`).
    fn settle_waiting(mut self, state: &EditorState, wait: bool) -> Result<(), Box<StaleAdoption>> {
        if let Some(why) = self.target.mismatch(state) {
            return Err(Box::new(StaleAdoption {
                why,
                refusal: None,
                pending: self,
            }));
        }
        let accepted = if wait {
            self.owned.accept()
        } else {
            self.owned.accept_nowait()
        };
        match accepted {
            Ok(()) => Ok(()),
            Err(error) => Err(Box::new(StaleAdoption {
                why: UNCONFIRMED,
                refusal: Some(error),
                pending: self,
            })),
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

/// Why a result that fits the project is still refused: the ledger would not record the acceptance.
const UNCONFIRMED: &str = "the claims ledger could not record that the result was accepted";

/// An adoption that was not accepted ([`PendingAdoption::settle`]). It still owns what it copied:
/// dropping or discarding it removes those files, which is the work to do off the UI thread.
#[derive(Debug)]
pub struct StaleAdoption {
    /// Why it was not accepted, in words.
    pub why: &'static str,
    /// Set when the project fit but the ledger refused the acceptance (`claims_unavailable`,
    /// `details.phase = "accept"`); `None` for a result that no longer fits the project.
    pub refusal: Option<JobError>,
    pub pending: PendingAdoption,
}

impl StaleAdoption {
    /// The project-facing `claims_unavailable` error of a ledger refusal, for a caller that
    /// hands `pending` to a discard job instead of removing it here; `None` for plain staleness.
    pub fn refusal_error(&self) -> Option<CommandError> {
        self.refusal
            .as_ref()
            .map(|error| refused_accept(error.clone(), None, "the adoption was not confirmed"))
    }
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

/// Records the file just published and the folders it needed that did not exist before. `Err`
/// is a publication the ledger could not claim: rollback was attempted under proof, and
/// [`Unclaimed`] reports what was taken back and what remained (see [`Created::published`]).
fn record_published(
    created: &mut Created,
    existing: &mut HashSet<PathBuf>,
    root: &Path,
    canonical_root: &Path,
    path: &ResourcePath,
    sha256: &str,
) -> Result<(), Unclaimed> {
    let new_dirs: Vec<(PathBuf, PathBuf)> = folders_of(root, canonical_root, path)
        .into_iter()
        .filter(|(folder, _)| existing.insert(folder.clone()))
        .collect();
    created.published(root, canonical_root, path, sha256, &new_dirs)
}

/// Creates `scenes/` and `scenes/<id>/` one folder at a time and records the ones that did not
/// exist (in creation order). A folder that is there already is relied on if another pending
/// result made it. Creating and recording share one registry lock: another adoption that finds
/// the folder "already there" then also finds its entry.
fn create_folders(
    workspace: &Path,
    managed: &ResourceBase,
    ledger: &Ledger,
    created: &mut Created,
) -> Result<(), JobError> {
    let mut tracker = ledger.lock_creating();
    // A ledger that cannot be used cannot tell this adoption whom a reused file belongs to:
    // refuse before anything is created. This is the only place where "nothing was changed" is
    // true; a ledger that fails later is a different error (`after_changes`).
    if let Some(why) = tracker.fault() {
        return Err(refused_before_changes(why));
    }
    if ledger.dir().to_str().is_none() {
        return Err(refused_before_changes(
            "the workspace path is not valid UTF-8, so its claims cannot be written down",
        ));
    }
    for folder in [workspace.join(MANAGED_DIR), managed.root().to_path_buf()] {
        match fs::create_dir(&folder) {
            Ok(()) => {
                let key = canonical_or_spelled(&folder);
                if let Err(why) = created.dir_with(&mut tracker, folder.clone(), key) {
                    // Still holding the ledger lock, so no other adoption can have found this
                    // folder yet: it is taken back before anyone can rely on it.
                    let mut failure = Unclaimed {
                        why,
                        ..Unclaimed::default()
                    };
                    match fs::remove_dir(&folder) {
                        Ok(()) => failure.taken_back.push(folder),
                        Err(error) => failure
                            .kept
                            .push(format!("`{}`: {error}", folder.display())),
                    }
                    return Err(failure.into_error());
                }
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let key = canonical_or_spelled(&folder);
                created.join_dir_with(&mut tracker, folder, key)?;
            }
            // What was made so far is removed by the caller's owner when it drops.
            Err(error) => return Err(io_error(&folder, &error)),
        }
    }
    Ok(())
}

/// Refuses an unusable ledger before publication, when no workspace mutation has occurred.
fn refused_before_changes(why: &str) -> JobError {
    JobError::new(
        "claims_unavailable",
        format!("the claims ledger of this workspace cannot be used; nothing was changed: {why}"),
    )
    .with_details(json!({ "phase": "before_changes", "why": why }))
}

/// Uses the canonical folder for identity comparison, falling back when it cannot be resolved.
fn canonical_or_spelled(folder: &Path) -> PathBuf {
    fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf())
}
