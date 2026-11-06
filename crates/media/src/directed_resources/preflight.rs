//! Read-only verification of every file the manifest says a frame needs.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tessel_editor_core::jobs::{CancelToken, JobError};
use tessel_editor_core::scene_resources::{
    ManifestIssue, ResourceKey, ResourceManifest, ResourcePath, SkippedResource, TransportItem,
};

use super::source::Root;
use super::{
    Failure, Options, ProblemKind, Progress, ResourceProblem, Stop, check, classify, issue_json,
};
use crate::fsx::copy_hashed;

/// A transport file that passed every check.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckedResource {
    pub path: ResourcePath,
    /// Every resource that lands on this file, with its own `assetId`.
    pub keys: Vec<ResourceKey>,
    pub bytes: u64,
    /// SHA-256 (lowercase hex) the file was verified against.
    pub hash: String,
    /// Canonical path of the verified file, inside the canonical source root. This is the
    /// *approved* path a renderer may open (`scene_resources::ResolvedResource::path`).
    pub resolved: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreflightReport {
    pub checked: Vec<CheckedResource>,
    /// Every file that failed, in plan order.
    pub problems: Vec<ResourceProblem>,
    /// Blocking issues of the manifest itself. When there are any no file was looked at.
    pub manifest_issues: Vec<ManifestIssue>,
    /// Resources the transport does not carry (preserved sections, unreferenced assets).
    pub skipped: Vec<SkippedResource>,
    pub total_bytes: u64,
}

impl PreflightReport {
    /// Every file the supported resources need is present, contained and verified.
    pub fn ok(&self) -> bool {
        self.problems.is_empty() && self.manifest_issues.is_empty()
    }

    pub fn to_json(&self) -> Value {
        json!({
            "ok": self.ok(),
            "totalBytes": self.total_bytes,
            "checked": self.checked.iter().map(|checked| json!({
                "path": checked.path.as_str(),
                "resources": checked.keys.iter().map(ResourceKey::label).collect::<Vec<_>>(),
                "bytes": checked.bytes,
            })).collect::<Vec<_>>(),
            "problems": self.problems.iter().map(ResourceProblem::to_json).collect::<Vec<_>>(),
            "manifestIssues": self.manifest_issues.iter().map(issue_json).collect::<Vec<_>>(),
            "skipped": self.skipped.iter().map(|skipped| json!({
                "resource": skipped.key.label(),
                "status": skipped.status.as_str(),
                "declaredPath": skipped.declared_path,
            })).collect::<Vec<_>>(),
        })
    }
}

/// Verifies, without writing anything, that the supported resources of `manifest` exist under
/// `source_root`, stay inside it, match their SHA-256 and look like what they claim to be.
///
/// Every problem is collected (one per file), so a caller can show them all. `Err` only for what
/// stops the run: an unusable root, a cancellation, a full read error of the whole job.
pub fn preflight(
    manifest: &ResourceManifest,
    source_root: &Path,
    options: &Options,
    token: &CancelToken,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<PreflightReport, JobError> {
    let plan = match manifest.transport_plan() {
        Ok(plan) => plan,
        Err(issues) => {
            return Ok(PreflightReport {
                checked: Vec::new(),
                problems: Vec::new(),
                manifest_issues: issues,
                skipped: Vec::new(),
                total_bytes: 0,
            });
        }
    };
    // A manifest with no files has nothing to open: requiring a folder for it would make a scene
    // that carries only data unloadable (BL101). Any file keeps the full containment checks.
    let root = if plan.items.is_empty() {
        None
    } else {
        Some(Root::open(source_root, "source")?)
    };
    let mut report = PreflightReport {
        checked: Vec::new(),
        problems: Vec::new(),
        manifest_issues: Vec::new(),
        skipped: plan.skipped.clone(),
        total_bytes: 0,
    };
    for (index, item) in plan.items.iter().enumerate() {
        token.check()?;
        let Some(root) = &root else { break };
        match check_item(root, item, options, token) {
            Ok((bytes, resolved)) => {
                report.total_bytes = report.total_bytes.saturating_add(bytes);
                report.checked.push(CheckedResource {
                    path: item.path.clone(),
                    keys: item.resources.iter().map(|r| r.key.clone()).collect(),
                    bytes,
                    hash: item.expected_hash.clone(),
                    resolved,
                });
            }
            Err(Stop::Problem(failure)) => report.problems.push(failure.of(item)),
            Err(Stop::Abort(error)) => return Err(error),
        }
        on_progress(Progress {
            done: index + 1,
            total: plan.items.len(),
            bytes: report.total_bytes,
        });
    }
    if report.total_bytes > options.max_total_bytes {
        report.problems.push(ResourceProblem {
            keys: Vec::new(),
            path: String::new(),
            kind: ProblemKind::TooLarge,
            detail: format!(
                "the files add up to {} bytes; the limit is {}",
                report.total_bytes, options.max_total_bytes
            ),
        });
    }
    Ok(report)
}

/// One transport file: contained, hashed, then its header. Returns its size and the canonical
/// path that was verified.
fn check_item(
    root: &Root,
    item: &TransportItem,
    options: &Options,
    token: &CancelToken,
) -> Result<(u64, PathBuf), Stop> {
    let mut source = root.open_file(&item.path, options)?;
    let (observed, bytes) = copy_hashed(
        &mut source.file,
        &mut io::sink(),
        token,
        Path::new(item.path.as_str()),
        |_| {},
    )
    .map_err(classify)?;
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
        check::content(&source.resolved, resource)?;
    }
    Ok((bytes, source.resolved))
}
