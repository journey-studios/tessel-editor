//! Materializes the supported resources of a manifest under a transport root.
//!
//! Three passes, so a failure never leaves a half-written root:
//!
//! 1. **Decide** (no writes): for each file, look at the destination. Something already there is
//!    reused when its SHA-256 is the expected one and is a conflict otherwise; a free slot needs a
//!    verified readable source. Every problem of this pass is collected.
//! 2. **Stage**: copy each new file into a staging folder inside the transport root while hashing
//!    it, compare with the declared hash and check its header. The first problem stops the run and
//!    the staging folder is removed. Cancellation is checked between chunks.
//! 3. **Publish**: create the folders and move each staged file into place without overwriting.
//!    Cancellation is observed before **every** file (BL080); on a cancellation or an error,
//!    exactly the files and folders created by this call are removed, never anything that was
//!    already in the root. [`materialize_observed`] reports each published file, which is also
//!    what makes a cancellation in the middle of publication testable without threads.
//!
//! The transport root is held under a [`RootLease`](super::RootLease) for the whole call
//! (BL079): a second Tessel operation on the same root is refused with `root_busy`.

use std::fs::File;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tessel_editor_core::jobs::{CancelToken, JobError};
use tessel_editor_core::scene_resources::{
    ResourceKey, ResourceManifest, ResourcePath, SkippedResource, TransportItem, TransportPlan,
};

use super::destination::{self, Journal, Slot};
use super::lease::RootLease;
use super::source::Root;
use super::{
    Failure, Options, ProblemKind, Progress, ResourceProblem, Stop, blocked_by_manifest, check,
    classify, rejected,
};
use crate::fsx::{Staging, copy_hashed, ensure_space, hash_file, io_error};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Written by this call.
    Copied,
    /// Already there with the expected content; left untouched.
    Reused,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Copied => "copied",
            Self::Reused => "reused",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MaterializedItem {
    pub path: ResourcePath,
    /// Every resource that lands on this file; each keeps its `assetId` and origin path in the
    /// manifest.
    pub keys: Vec<ResourceKey>,
    pub bytes: u64,
    /// SHA-256 (lowercase hex) of the file, as the bundle declares and the copy was verified.
    pub hash: String,
    pub outcome: Outcome,
}

/// One file made visible in the transport root; the next cancellation check follows it.
#[derive(Debug, Clone, Copy)]
pub struct Published<'a> {
    pub path: &'a ResourcePath,
    pub done: usize,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MaterializeReport {
    pub items: Vec<MaterializedItem>,
    /// Not carried: preserved sections and unreferenced assets. Never silently dropped.
    pub skipped: Vec<SkippedResource>,
    /// The manifest says everything the bundle names was carried. `false` is the normal state of
    /// this slice whenever the bundle has preserved sections: the transport is partial.
    pub complete: bool,
    pub bytes_copied: u64,
}

impl MaterializeReport {
    pub fn copied(&self) -> usize {
        self.count(Outcome::Copied)
    }

    pub fn reused(&self) -> usize {
        self.count(Outcome::Reused)
    }

    fn count(&self, outcome: Outcome) -> usize {
        self.items
            .iter()
            .filter(|item| item.outcome == outcome)
            .count()
    }

    pub fn to_json(&self) -> Value {
        json!({
            "complete": self.complete,
            "bytesCopied": self.bytes_copied,
            "items": self.items.iter().map(|item| json!({
                "path": item.path.as_str(),
                "resources": item.keys.iter().map(ResourceKey::label).collect::<Vec<_>>(),
                "bytes": item.bytes,
                "sha256": item.hash,
                "outcome": item.outcome.as_str(),
            })).collect::<Vec<_>>(),
            "skipped": self.skipped.iter().map(|skipped| json!({
                "resource": skipped.key.label(),
                "status": skipped.status.as_str(),
                "declaredPath": skipped.declared_path,
            })).collect::<Vec<_>>(),
        })
    }
}

/// What pass 1 decided for one file.
struct Work<'a> {
    item: &'a TransportItem,
    outcome: Outcome,
    bytes: u64,
}

/// Puts the supported resources of `manifest` under `transport_root`, at their declared relative
/// paths, reading them only from under `source_root`.
///
/// The transport root must exist. Errors (all leave the root as it was, except that a rollback
/// that itself fails says what remains in `details.rollbackLeftovers`):
/// `manifest_blocked`, `resources_invalid` (`details.problems`), `root_unavailable`,
/// `root_not_directory`, `root_busy`, `disk_full`, `cancelled`, `destination_conflict`,
/// `destination_escaped`, `permission_denied`, `io_error`.
pub fn materialize(
    manifest: &ResourceManifest,
    source_root: &Path,
    transport_root: &Path,
    options: &Options,
    token: &CancelToken,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<MaterializeReport, JobError> {
    materialize_observed(
        manifest,
        source_root,
        transport_root,
        options,
        token,
        on_progress,
        &mut |_| {},
    )
}

/// [`materialize`] that also reports every file as it is published.
pub fn materialize_observed(
    manifest: &ResourceManifest,
    source_root: &Path,
    transport_root: &Path,
    options: &Options,
    token: &CancelToken,
    on_progress: &mut dyn FnMut(Progress),
    on_published: &mut dyn FnMut(Published<'_>),
) -> Result<MaterializeReport, JobError> {
    let plan = manifest
        .transport_plan()
        .map_err(|issues| blocked_by_manifest(&issues))?;
    let source = Root::open(source_root, "source")?;
    let lease = RootLease::acquire(transport_root)?;
    run(
        manifest,
        plan,
        &source,
        lease.root(),
        options,
        token,
        on_progress,
        on_published,
    )
}

/// [`materialize_observed`] for a caller that already holds the lease of the transport root
/// (it has more to do there than this call, such as a package assembling several scenes).
pub fn materialize_leased(
    manifest: &ResourceManifest,
    source_root: &Path,
    lease: &RootLease,
    options: &Options,
    token: &CancelToken,
    on_progress: &mut dyn FnMut(Progress),
    on_published: &mut dyn FnMut(Published<'_>),
) -> Result<MaterializeReport, JobError> {
    let plan = manifest
        .transport_plan()
        .map_err(|issues| blocked_by_manifest(&issues))?;
    let source = Root::open(source_root, "source")?;
    run(
        manifest,
        plan,
        &source,
        lease.root(),
        options,
        token,
        on_progress,
        on_published,
    )
}

#[allow(clippy::too_many_arguments)]
fn run(
    manifest: &ResourceManifest,
    plan: TransportPlan,
    source: &Root,
    destination_root: &Path,
    options: &Options,
    token: &CancelToken,
    on_progress: &mut dyn FnMut(Progress),
    on_published: &mut dyn FnMut(Published<'_>),
) -> Result<MaterializeReport, JobError> {
    let destination = Root::open(destination_root, "transport")?;
    let total = plan.items.len();

    let works = decide(&plan.items, source, &destination, options, token)?;
    let reused_bytes: u64 = works
        .iter()
        .filter(|work| work.outcome == Outcome::Reused)
        .map(|work| work.bytes)
        .sum();
    let copy_bytes: u64 = works
        .iter()
        .filter(|work| work.outcome == Outcome::Copied)
        .map(|work| work.bytes)
        .sum();
    if copy_bytes.saturating_add(reused_bytes) > options.max_total_bytes {
        return Err(rejected(&[ResourceProblem {
            keys: Vec::new(),
            path: String::new(),
            kind: ProblemKind::TooLarge,
            detail: format!(
                "the files add up to {} bytes; the limit is {}",
                copy_bytes.saturating_add(reused_bytes),
                options.max_total_bytes
            ),
        }]));
    }
    ensure_space(destination.path(), copy_bytes, options.space_margin)?;

    let reused = works
        .iter()
        .filter(|work| work.outcome == Outcome::Reused)
        .count();
    if reused > 0 {
        on_progress(Progress {
            done: reused,
            total,
            bytes: reused_bytes,
        });
    }

    let staging = Staging::create(destination.path(), "resources")?;
    let mut staged: Vec<(&Work<'_>, PathBuf)> = Vec::new();
    let mut staged_bytes = 0u64;
    for (index, work) in works
        .iter()
        .enumerate()
        .filter(|(_, work)| work.outcome == Outcome::Copied)
    {
        token.check()?;
        let staged_path = staging.path().join(format!("{index}.part"));
        stage(work.item, source, &staged_path, options, token)
            .map_err(|stop| stop_to_error(stop, work.item))?;
        staged.push((work, staged_path));
        staged_bytes = staged_bytes.saturating_add(work.bytes);
        on_progress(Progress {
            done: reused + staged.len(),
            total,
            bytes: reused_bytes.saturating_add(staged_bytes),
        });
    }

    let mut journal = Journal::new(destination.path());
    if let Err(error) = publish(
        &staged,
        destination.path(),
        token,
        &mut journal,
        on_published,
    ) {
        let leftovers = journal.rollback();
        return Err(with_leftovers(error, leftovers));
    }
    // Dropping the staging folder removes the staged names; the published files keep their data.
    drop(staging);

    Ok(MaterializeReport {
        items: works
            .iter()
            .map(|work| MaterializedItem {
                path: work.item.path.clone(),
                keys: work.item.resources.iter().map(|r| r.key.clone()).collect(),
                bytes: work.bytes,
                hash: work.item.expected_hash.clone(),
                outcome: work.outcome,
            })
            .collect(),
        skipped: plan.skipped,
        complete: manifest.is_complete(),
        bytes_copied: copy_bytes,
    })
}

/// Pass 1: reuse, copy or reject each file. No write happens here.
fn decide<'a>(
    items: &'a [TransportItem],
    source: &Root,
    destination: &Root,
    options: &Options,
    token: &CancelToken,
) -> Result<Vec<Work<'a>>, JobError> {
    let mut works = Vec::new();
    let mut problems = Vec::new();
    for item in items {
        token.check()?;
        match decide_item(item, source, destination, options, token) {
            Ok((outcome, bytes)) => works.push(Work {
                item,
                outcome,
                bytes,
            }),
            Err(Stop::Problem(failure)) => problems.push(failure.of(item)),
            Err(Stop::Abort(error)) => return Err(error),
        }
    }
    if !problems.is_empty() {
        return Err(rejected(&problems));
    }
    Ok(works)
}

fn decide_item(
    item: &TransportItem,
    source: &Root,
    destination: &Root,
    options: &Options,
    token: &CancelToken,
) -> Result<(Outcome, u64), Stop> {
    let slot = destination::inspect(destination.path(), &item.path)?;
    let Slot::File(existing) = slot else {
        let opened = source.open_file(&item.path, options)?;
        return Ok((Outcome::Copied, opened.size));
    };
    let (observed, bytes) = hash_file(&existing, token, |_| {}).map_err(classify)?;
    if observed != item.expected_hash {
        return Err(Failure::new(
            ProblemKind::DestinationConflict,
            format!(
                "`{}` is already in the transport root with sha256 {observed}, not the expected {}; it was not overwritten",
                item.path, item.expected_hash
            ),
        )
        .into());
    }
    for resource in &item.resources {
        check::content(&existing, resource)?;
    }
    Ok((Outcome::Reused, bytes))
}

/// Pass 2 for one file: verified copy into staging, then hash and header of the staged bytes.
/// Checking the copy (not the source again) means what gets published is what was verified.
fn stage(
    item: &TransportItem,
    source: &Root,
    staged_path: &Path,
    options: &Options,
    token: &CancelToken,
) -> Result<(), Stop> {
    let mut opened = source.open_file(&item.path, options)?;
    let mut output = File::create_new(staged_path)
        .map_err(|error| Stop::Abort(io_error(staged_path, &error)))?;
    let (observed, _) =
        copy_hashed(&mut opened.file, &mut output, token, staged_path, |_| {}).map_err(classify)?;
    output
        .sync_all()
        .map_err(|error| Stop::Abort(io_error(staged_path, &error)))?;
    if observed != item.expected_hash {
        return Err(Failure::new(
            ProblemKind::HashMismatch,
            format!(
                "expected sha256 {} but the file has {observed}",
                item.expected_hash
            ),
        )
        .into());
    }
    for resource in &item.resources {
        check::content(staged_path, resource)?;
    }
    Ok(())
}

/// Pass 3: folders, then each staged file, in plan order. Cancellation is checked before each
/// file, so what is visible when it stops is exactly what the journal can remove.
fn publish(
    staged: &[(&Work<'_>, PathBuf)],
    root: &Path,
    token: &CancelToken,
    journal: &mut Journal,
    on_published: &mut dyn FnMut(Published<'_>),
) -> Result<(), JobError> {
    for (index, (work, staged_path)) in staged.iter().enumerate() {
        token.check()?;
        journal.create_parents(root, &work.item.path)?;
        journal.publish(staged_path, &destination::target(root, &work.item.path))?;
        on_published(Published {
            path: &work.item.path,
            done: index + 1,
            total: staged.len(),
        });
    }
    Ok(())
}

fn stop_to_error(stop: Stop, item: &TransportItem) -> JobError {
    match stop {
        Stop::Abort(error) => error,
        Stop::Problem(failure) => rejected(&[failure.of(item)]),
    }
}

fn with_leftovers(error: JobError, leftovers: Vec<String>) -> JobError {
    if leftovers.is_empty() {
        return error;
    }
    // Indexing a non-object would panic, so only an existing object is extended.
    let mut details = match error.details.clone() {
        Some(details @ Value::Object(_)) => details,
        _ => json!({}),
    };
    details["rollbackLeftovers"] = json!(leftovers);
    error.with_details(details)
}
