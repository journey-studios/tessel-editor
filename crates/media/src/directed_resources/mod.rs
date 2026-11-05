//! Resolution, preflight and materialization of directed-scene resources (BB06 initial slice;
//! F038): turns the pure `ResourceManifest` of `tessel_editor_core::scene_resources` into real
//! files under a *transport root*.
//!
//! ```text
//! manifest ──transport_plan──▶ items ──preflight──▶ report            (read-only)
//!                                    └─materialize─▶ <transport root>/<relativePath>
//! ```
//!
//! The transport root is held under an exclusive [`RootLease`] (BL079), so two *Tessel*
//! operations never publish into one root together. That is cooperative: a hostile writer
//! replacing a parent directory with a link is only narrowed (a containment check around each
//! link), not excluded; the source root is read-only and verified by hash. Limits:
//! `docs/features/scene-resources.md` and `docs/features/scene-resource-integration.md`.
//! What it guarantees (the lease and the roots' own ownership being the precondition):
//!
//! - **Containment.** A joined path does not keep a symbolic link inside its root, so every source
//!   path is walked component by component (no symlink unless `SymlinkPolicy::AllowContained`),
//!   canonicalized and checked to stay under the canonical source root, must be a regular file,
//!   and the opened handle is compared with the verified file. Nothing outside the source root is
//!   ever read, and nothing is written through a symlink or outside the transport root.
//! - **Identity.** Files land at the declared `relativePath` (normalized by `ResourcePath`), keeping
//!   the bundle's `assetId`s: no second content-addressed store is created. A file already at the
//!   destination with the expected hash is *reused*; with another content it is a conflict and is
//!   never overwritten. The package's `media/<sha256>` store dedupes later, at package time.
//! - **Verification.** Every file is hashed against `contentHash` (SHA-256). PNGs also have their
//!   signature and IHDR size checked against `widthPx`/`heightPx`; splats must be SPZ. Alpha and
//!   pixels are not decoded. Preflight reports every problem; materialize stops at the first
//!   content problem.
//! - **Atomicity.** Files are copied and verified into a staging folder inside the transport root,
//!   then published without overwriting. An error or a cancellation before publishing removes the
//!   staging folder and leaves the root as it was; an error or a cancellation while publishing
//!   (checked before every file, BL080) removes exactly the files and folders this call created,
//!   never what was already in the root. Publication requires hard-link support; failures are
//!   returned, never downgraded to a check-then-rename that could overwrite.
//!
//! The consumers that wire this into the editor (resolution for the renderer, adoption into the
//! project, package/restore and scene interchange) are `crate::scene_resources`.

mod check;
mod destination;
mod lease;
mod materialize;
mod preflight;
mod source;

pub use lease::{LEASE_FILE, RootLease};
pub use materialize::{
    MaterializeReport, MaterializedItem, Outcome, Published, materialize, materialize_leased,
    materialize_observed,
};
pub use preflight::{CheckedResource, PreflightReport, preflight};

use std::path::Path;

use serde_json::{Value, json};
use tessel_editor_core::jobs::JobError;
use tessel_editor_core::scene_resources::{
    ManifestIssue, ResourceKey, TransportItem, TransportResource,
};

/// Header check of one file against what the bundle says about the resource (PNG signature and
/// size, SPZ). The hash is the caller's business; this is what lets a restore re-prove a file
/// it did not copy itself without reading it again.
pub(crate) fn verify_content(path: &Path, resource: &TransportResource) -> Result<(), String> {
    check::content(path, resource).map_err(|failure| failure.detail)
}

/// Whether a symbolic link may appear on the way to a source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SymlinkPolicy {
    /// Any symbolic link on the path is refused. The default: the bundle folder is untrusted
    /// input and a link is the one thing a textual path check cannot see.
    #[default]
    Reject,
    /// A link may be followed when the final canonical path still lies inside the source root.
    AllowContained,
}

#[derive(Debug, Clone)]
pub struct Options {
    pub symlinks: SymlinkPolicy,
    /// Largest accepted file, in bytes (`too_large`).
    pub max_resource_bytes: u64,
    /// Largest accepted sum of all files, in bytes.
    pub max_total_bytes: u64,
    /// Free space kept beyond what materializing needs.
    pub space_margin: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            symlinks: SymlinkPolicy::Reject,
            max_resource_bytes: 1 << 30,
            max_total_bytes: 4 << 30,
            space_margin: 64 << 20,
        }
    }
}

/// Per-item progress, reported after each file is verified; publication follows verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
    pub bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProblemKind {
    /// No file at the path (a dangling link included).
    Missing,
    /// A symbolic link on the path and the policy refuses them.
    SymlinkRejected,
    /// The canonical path leaves the source root.
    EscapesRoot,
    /// A folder, device or pipe where a file belongs, or a file where a folder belongs.
    NotRegularFile,
    TooLarge,
    HashMismatch,
    SizeMismatch,
    /// A PNG whose IHDR size is not `widthPx` x `heightPx`.
    DimensionMismatch,
    /// Not the format the bundle declares (not a PNG, not an SPZ).
    InvalidContent,
    Unreadable,
    /// The file changed between being verified and being opened.
    Changed,
    /// The transport root already holds something else at that path.
    DestinationConflict,
}

impl ProblemKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::SymlinkRejected => "symlink_rejected",
            Self::EscapesRoot => "escapes_root",
            Self::NotRegularFile => "not_regular_file",
            Self::TooLarge => "too_large",
            Self::HashMismatch => "hash_mismatch",
            Self::SizeMismatch => "size_mismatch",
            Self::DimensionMismatch => "dimension_mismatch",
            Self::InvalidContent => "invalid_content",
            Self::Unreadable => "unreadable",
            Self::Changed => "changed_during_check",
            Self::DestinationConflict => "destination_conflict",
        }
    }
}

/// Why one transport file cannot be carried, with every resource that points at it.
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceProblem {
    pub keys: Vec<ResourceKey>,
    /// Normalized transport path; empty for a problem of the whole set.
    pub path: String,
    pub kind: ProblemKind,
    pub detail: String,
}

impl ResourceProblem {
    pub fn to_json(&self) -> Value {
        json!({
            "resources": self.keys.iter().map(ResourceKey::label).collect::<Vec<_>>(),
            "path": self.path,
            "code": self.kind.code(),
            "detail": self.detail,
        })
    }
}

/// A problem not yet attached to an item.
#[derive(Debug)]
struct Failure {
    kind: ProblemKind,
    detail: String,
}

impl Failure {
    fn new(kind: ProblemKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }

    fn of(self, item: &TransportItem) -> ResourceProblem {
        ResourceProblem {
            keys: item.resources.iter().map(|r| r.key.clone()).collect(),
            path: item.path.to_string(),
            kind: self.kind,
            detail: self.detail,
        }
    }
}

/// What stops processing one item: a problem with that item (collected or reported), or an
/// abort of the whole run (cancellation, a full disk).
#[derive(Debug)]
enum Stop {
    Problem(Failure),
    Abort(JobError),
}

impl From<Failure> for Stop {
    fn from(failure: Failure) -> Self {
        Self::Problem(failure)
    }
}

/// Splits a `JobError` from a streaming copy: cancellation and a full disk abort the run, any
/// other IO error is about the file being read.
fn classify(error: JobError) -> Stop {
    if error.is_cancelled() || error.code == "disk_full" {
        return Stop::Abort(error);
    }
    Stop::Problem(Failure::new(ProblemKind::Unreadable, error.message))
}

fn issue_json(issue: &ManifestIssue) -> Value {
    json!({
        "code": issue.code,
        "severity": issue.severity.as_str(),
        "resource": issue.key.as_ref().map(ResourceKey::label),
        "pointer": issue.pointer,
        "message": issue.message,
    })
}

/// The manifest has blocking issues: no plan, so nothing was looked at.
pub(crate) fn blocked_by_manifest(issues: &[ManifestIssue]) -> JobError {
    JobError::new(
        "manifest_blocked",
        format!(
            "{} blocking issue(s) in the resource manifest; nothing was read or written",
            issues.len()
        ),
    )
    .with_details(json!({ "issues": issues.iter().map(issue_json).collect::<Vec<_>>() }))
}

pub(crate) fn rejected(problems: &[ResourceProblem]) -> JobError {
    JobError::new(
        "resources_invalid",
        format!(
            "{} resource problem(s); nothing was published",
            problems.len()
        ),
    )
    .with_details(
        json!({ "problems": problems.iter().map(ResourceProblem::to_json).collect::<Vec<_>>() }),
    )
}
