//! Scene resources inside a project package (BL011): which scenes' files go in, from where, and
//! how a restored package proves it can open without the originals.
//!
//! Layout: `scenes/<sceneId>/<relativePath>` inside the package, the same layout as the managed
//! base of a workspace (`base.rs`). A restored package therefore uses *its own root* as the base
//! and nothing records, or needs, where the files were authored.
//!
//! What a package may claim (the guard that stays, narrowed to what is true):
//!
//! * A scene whose supported resources were verified and copied is carried. If the manifest also
//!   has parts nothing interprets yet (maps, depth, relief, narration takes) or unreferenced
//!   assets, the package still reports those as `not_packaged` and is **partial**.
//! * A scene with no base to read from, a blocked manifest or a file that fails verification is
//!   not carried at all (all or nothing per scene), and is reported as `not_packaged` (nothing
//!   to carry from) or `missing`/`changed` (a verified read failed), exactly like media.
//! * A package is `complete` only when nothing is reported. `allow_partial` is the only way to
//!   write one that is not.
//!
//! The file layer is `directed_resources::materialize_*` (staging, hashes, headers, rollback,
//! lease), so packaging adds no copy or verification rules of its own.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tessel_editor_core::EditorState;
use tessel_editor_core::jobs::{CancelToken, JobError};
use tessel_editor_core::scene_resources::{ResourceKey, ResourceManifest};

use super::base::{self, ResourceBase};
use super::load;
use crate::directed_resources::{self, MaterializeReport, Options, materialize};

/// `state` of a `missing` entry standing for resources of a scene the package does not carry.
/// Kept from the first guard (F036) so existing readers of `missing` still understand it.
pub const NOT_PACKAGED: &str = "not_packaged";

/// A scene whose files go into the package.
#[derive(Debug, Clone)]
pub(crate) struct Carry {
    pub scene_id: String,
    pub manifest: ResourceManifest,
    pub base: ResourceBase,
}

#[derive(Debug, Default)]
pub(crate) struct ScenePlan {
    pub carry: Vec<Carry>,
    /// Entries for the package's `missing` list.
    pub missing: Vec<Value>,
    /// Estimated bytes of the files to carry (their size on disk now).
    pub bytes: u64,
}

fn entry(scene: &str, state: &str, reason: &str, paths: &[String]) -> Value {
    json!({ "asset": format!("scene:{scene}"), "state": state, "reason": reason, "paths": paths })
}

fn estimate(manifest: &ResourceManifest, base: &ResourceBase) -> u64 {
    let Ok(plan) = manifest.transport_plan() else {
        return 0;
    };
    plan.items
        .iter()
        .map(|item| {
            let mut path = base.root().to_path_buf();
            path.extend(item.path.segments());
            fs::metadata(path).map_or(0, |meta| meta.len())
        })
        .sum()
}

/// Declared paths of what a carried scene still leaves behind (preserved sections, unreferenced
/// assets, narration takes).
fn left_behind(manifest: &ResourceManifest) -> Vec<String> {
    let mut paths: Vec<String> = manifest
        .uninterpreted()
        .iter()
        .flat_map(|section| section.files.iter().map(|file| file.relative_path.clone()))
        .collect();
    if let Ok(plan) = manifest.transport_plan() {
        paths.extend(
            plan.skipped
                .iter()
                .map(|skipped| skipped.declared_path.clone()),
        );
    }
    paths.sort();
    paths.dedup();
    paths
}

/// Decides, for every scene of `state` that names files, whether the package carries them.
///
/// `explicit` maps scene IDs to an absolute folder the caller authorized for this package (the
/// folder a bundle was authored in); without an entry the managed base of the workspace is used
/// when it exists. `verify` makes the decision read and hash every file (package preflight, or a
/// create that may be partial); without it only the existence of the base is checked and
/// `materialize` verifies while it copies, which is enough when any failure aborts the package.
pub(crate) fn plan(
    state: &EditorState,
    explicit: &BTreeMap<String, PathBuf>,
    verify: Option<&CancelToken>,
) -> Result<ScenePlan, JobError> {
    let mut planned = ScenePlan::default();
    let named: BTreeMap<&str, Vec<&str>> = state.scenes.resource_paths().into_iter().collect();
    for entry_scene in state.scenes.iter() {
        let id = entry_scene.id.as_str();
        let Some(paths) = named.get(id) else { continue };
        let paths: Vec<String> = paths.iter().map(|path| (*path).to_string()).collect();
        let manifest = ResourceManifest::from_scene(&entry_scene.scene);
        if let Err(issues) = manifest.transport_plan() {
            let mut blocked = entry(id, NOT_PACKAGED, "manifest_blocked", &paths);
            blocked["issues"] = json!(
                issues
                    .iter()
                    .map(|issue| json!({
                "code": issue.code, "resource": issue.key.as_ref().map(ResourceKey::label),
                "message": issue.message }))
                    .collect::<Vec<_>>()
            );
            planned.missing.push(blocked);
            continue;
        }
        let base = match explicit.get(id) {
            Some(root) => Some(ResourceBase::explicit(root)?),
            None => state
                .project
                .workspace
                .as_deref()
                .map(|workspace| ResourceBase::managed(workspace, id))
                .transpose()?
                .filter(ResourceBase::exists),
        };
        let Some(base) = base else {
            planned
                .missing
                .push(entry(id, NOT_PACKAGED, "no_resource_base", &paths));
            continue;
        };
        if let Some(token) = verify {
            match load::check(
                &entry_scene.scene,
                &base,
                &Options::default(),
                token,
                &mut |_| {},
            ) {
                Err(error) if error.is_cancelled() => return Err(error),
                Err(error) => {
                    let mut gone = entry(id, "missing", &error.code, &paths);
                    gone["detail"] = json!(error.message);
                    planned.missing.push(gone);
                    continue;
                }
                Ok(report) if !report.ok() => {
                    let state_label = if report
                        .problems
                        .iter()
                        .all(|problem| problem.kind == directed_resources::ProblemKind::Missing)
                        && report.manifest_issues.is_empty()
                    {
                        "missing"
                    } else {
                        "changed"
                    };
                    let problem_paths: Vec<String> = report
                        .problems
                        .iter()
                        .map(|problem| problem.path.clone())
                        .collect();
                    let mut bad = entry(id, state_label, "verification_failed", &problem_paths);
                    bad["problems"] = json!(
                        report
                            .problems
                            .iter()
                            .map(|p| p.to_json())
                            .collect::<Vec<_>>()
                    );
                    planned.missing.push(bad);
                    continue;
                }
                Ok(_) => {}
            }
        }
        let behind = left_behind(&manifest);
        if !manifest.is_complete() {
            planned.missing.push(entry(
                id,
                NOT_PACKAGED,
                "preserved_not_transported",
                &behind,
            ));
        }
        planned.bytes = planned.bytes.saturating_add(estimate(&manifest, &base));
        planned.carry.push(Carry {
            scene_id: id.to_string(),
            manifest,
            base,
        });
    }
    // Folders are named after scene IDs: two that differ only by case would share one folder on
    // a case-insensitive volume and mix their resources. Not something `allow_partial` fixes.
    let colliding = base::folding_collisions(planned.carry.iter().map(|c| c.scene_id.as_str()));
    if let Some((first, second)) = colliding.first() {
        return Err(JobError::new(
            "scene_id_collision",
            format!(
                "scenes `{first}` and `{second}` differ only by letter case: their resource folders would be one on a case-insensitive volume"
            ),
        ));
    }
    Ok(planned)
}

/// Whether any entry of `missing` stands for resources of a scene that cannot be carried at all.
pub(crate) fn has_not_packaged(missing: &[Value]) -> bool {
    missing.iter().any(|entry| entry["state"] == NOT_PACKAGED)
}

/// One file written for the package, named as the package's manifest names it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Written {
    /// `scenes/<id>/<path>`.
    pub entry: String,
    /// Where the verified copy is (a staging folder, or the package content itself).
    pub file: PathBuf,
    pub size: u64,
    pub sha: String,
    pub scene: String,
    pub assets: Vec<String>,
}

/// Materializes the files of `carry` under `<parent>/scenes/<id>/` (verified, atomic, leased)
/// and lists them for the package manifest.
pub(crate) fn stage(
    carry: &Carry,
    parent: &Path,
    token: &CancelToken,
) -> Result<(MaterializeReport, Vec<Written>), JobError> {
    let prefix = base::entry_prefix(&carry.scene_id)?;
    let destination = parent.join(&prefix);
    fs::create_dir_all(&destination).map_err(|error| crate::fsx::io_error(&destination, &error))?;
    let report = materialize(
        &carry.manifest,
        carry.base.root(),
        &destination,
        &Options::default(),
        token,
        &mut |_| {},
    )
    .map_err(|error| in_scene(error, &carry.scene_id))?;
    let written = report
        .items
        .iter()
        .map(|item| {
            let mut file = destination.clone();
            file.extend(item.path.segments());
            Written {
                entry: format!("{prefix}/{}", item.path),
                file,
                size: item.bytes,
                sha: item.hash.clone(),
                scene: carry.scene_id.clone(),
                assets: item.keys.iter().map(ResourceKey::label).collect(),
            }
        })
        .collect();
    Ok((report, written))
}

/// Says which scene a resource error belongs to, keeping its code and details.
fn in_scene(mut error: JobError, scene: &str) -> JobError {
    error.message = format!("Scene `{scene}`: {}", error.message);
    let mut details = match error.details.take() {
        Some(details @ Value::Object(_)) => details,
        _ => json!({}),
    };
    details["scene"] = json!(scene);
    error.with_details(details)
}

/// Entries of the package manifest for the written files.
pub(crate) fn manifest_entries(written: &[Written]) -> Vec<Value> {
    written
        .iter()
        .map(|file| {
            json!({ "path": file.entry, "size": file.size, "sha256": file.sha,
                "role": "scene-resource", "scene": file.scene, "assets": file.assets })
        })
        .collect()
}

/// After a restore: for every scene of the restored project, proves its supported resources are
/// in the package at `scenes/<id>/<path>` with the *bundle's own* declared hash (not only the
/// package's), and still look like what the bundle says (PNG header and size, SPZ). The files
/// were already hashed and extracted, so this reads only headers: it is what lets a restored
/// project open with no access to where it came from.
///
/// Returns one entry per scene that is not fully there (empty when every scene resolves).
pub(crate) fn verify_restored(
    loaded: &EditorState,
    listed: &[(String, u64, String)],
    root: &Path,
) -> Vec<Value> {
    let mut unresolved = Vec::new();
    for scene in loaded.scenes.iter() {
        let id = scene.id.as_str();
        let manifest = ResourceManifest::from_scene(&scene.scene);
        let plan = match manifest.transport_plan() {
            Ok(plan) => plan,
            Err(_) => {
                unresolved.push(entry(id, "unresolved", "manifest_blocked", &[]));
                continue;
            }
        };
        let Ok(prefix) = base::entry_prefix(id) else {
            unresolved.push(entry(id, "unresolved", "scene_id_not_portable", &[]));
            continue;
        };
        let mut reasons: Vec<String> = Vec::new();
        let mut paths: Vec<String> = Vec::new();
        for item in &plan.items {
            let name = format!("{prefix}/{}", item.path);
            let file = root.join(&name);
            let hash_ok = listed
                .iter()
                .any(|(path, _, sha)| *path == name && *sha == item.expected_hash);
            let mut problem = (!hash_ok).then(|| {
                "the package does not carry it with the hash the bundle declares".to_string()
            });
            if problem.is_none() {
                problem = item
                    .resources
                    .iter()
                    .find_map(|resource| directed_resources::verify_content(&file, resource).err());
            }
            if let Some(problem) = problem {
                paths.push(item.path.to_string());
                reasons.push(format!("{}: {problem}", item.path));
            }
        }
        if !paths.is_empty() {
            let mut gone = entry(id, "unresolved", "not_in_package", &paths);
            gone["details"] = json!(reasons);
            unresolved.push(gone);
        }
    }
    unresolved
}
