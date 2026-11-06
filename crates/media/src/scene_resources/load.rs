//! Loading: from a scene and an explicit base to the approved [`ResolvedResourceMap`].
//!
//! Everything here is a job body: it hashes every file (preflight reads them once) and never
//! touches the project. It reuses `directed_resources::preflight` for every rule (containment,
//! symlinks, hashes, PNG/SPZ headers, limits) and adds none of its own; what it adds is the
//! *result shape* a renderer needs: the canonical path the preflight verified, the content and
//! revision identity, and the generation the caller captured when it scheduled the job.

use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::jobs::{CancelToken, JobError};
use tessel_editor_core::scene_resources::ResourceManifest;

use super::base::ResourceBase;
use super::map::{Generation, ResolvedResource, ResolvedResourceMap};
use crate::directed_resources::{
    Options, PreflightReport, Progress, blocked_by_manifest, preflight, rejected,
};

/// What identifies one load: the scene, the project and generation it is for, and where to read.
#[derive(Debug, Clone, Copy)]
pub struct LoadRequest<'a> {
    pub scene: &'a DirectedScene,
    pub scene_id: &'a str,
    pub project_id: &'a str,
    pub generation: Generation,
    pub base: &'a ResourceBase,
}

/// Whether loading `scene` opens any file: `false` only for a manifest that is valid and names
/// none. Such a scene loads against [`ResourceBase::unbound`]; a blocked manifest answers `true`
/// so its own refusal (`manifest_blocked`) is the one reported.
pub fn needs_files(scene: &DirectedScene) -> bool {
    !ResourceManifest::from_scene(scene)
        .transport_plan()
        .is_ok_and(|plan| plan.items.is_empty())
}

/// Read-only report of whether the scene's supported resources can be approved at `base`.
/// Collects every problem; `Err` only for what stops the run (an unusable base, a cancellation).
pub fn check(
    scene: &DirectedScene,
    base: &ResourceBase,
    options: &Options,
    token: &CancelToken,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<PreflightReport, JobError> {
    let manifest = ResourceManifest::from_scene(scene);
    preflight(&manifest, base.root(), options, token, on_progress)
}

/// Verifies every supported resource and returns the map, or the first typed reason it cannot:
/// `manifest_blocked`, `resources_invalid` (every problem in `details.problems`),
/// `root_unavailable`, `cancelled`. A failure approves nothing and changes nothing.
pub fn load(
    request: &LoadRequest<'_>,
    options: &Options,
    token: &CancelToken,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<ResolvedResourceMap, JobError> {
    let manifest = ResourceManifest::from_scene(request.scene);
    let report = preflight(&manifest, request.base.root(), options, token, on_progress)?;
    if !report.manifest_issues.is_empty() {
        return Err(blocked_by_manifest(&report.manifest_issues));
    }
    if !report.problems.is_empty() {
        return Err(rejected(&report.problems));
    }
    let mut entries = Vec::new();
    for checked in &report.checked {
        for key in &checked.keys {
            // A key of a checked item comes from this manifest's own plan: absent is impossible,
            // and skipping it would silently hand the renderer a smaller map than it asked for.
            let entry = manifest.get(key).ok_or_else(|| {
                JobError::new(
                    "internal_error",
                    format!(
                        "{} is in the transport plan but not in the manifest",
                        key.label()
                    ),
                )
            })?;
            entries.push(ResolvedResource {
                key: key.clone(),
                kind: entry.kind,
                path: checked.resolved.clone(),
                content_hash: checked.hash.clone(),
                revision_id: entry.revision_id.clone(),
                bytes: checked.bytes,
                uses: entry.uses.clone(),
            });
        }
    }
    entries.sort_by(|a, b| a.key.cmp(&b.key));
    let base = match std::fs::canonicalize(request.base.root()) {
        Ok(base) => base,
        // No file was opened (the manifest names none), so there is no folder to vouch for.
        Err(_) if report.checked.is_empty() => request.base.root().to_path_buf(),
        Err(error) => {
            return Err(JobError::new(
                "root_unavailable",
                format!(
                    "the resource base `{}`: {error}",
                    request.base.root().display()
                ),
            ));
        }
    };
    Ok(ResolvedResourceMap {
        scene_id: request.scene_id.to_string(),
        bundle_id: manifest.bundle_id().to_string(),
        bundle_revision: manifest.revision_id().to_string(),
        project_id: request.project_id.to_string(),
        generation: request.generation,
        base,
        entries,
        skipped: report.skipped,
        complete: manifest.is_complete(),
    })
}
