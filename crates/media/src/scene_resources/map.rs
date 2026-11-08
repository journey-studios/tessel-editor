//! The resource map a renderer reads: for each resource of one scene, the **approved path** plus
//! the identity of the content that was approved and the generation it was approved under.
//!
//! The contract, in the words the renderer's side needs:
//!
//! * `assetId` identifies a resource *inside one scene's bundle*. It is never a cache key by
//!   itself: two projects, two scenes or two revisions can reuse an ID for different bytes.
//!   [`ApprovedResource::cache_key`] folds in the project, the generation, the scene, the bundle
//!   revision and the content hash, so a stale hit is impossible by construction.
//! * `path` is canonical, lies inside the canonical base the preflight verified, and names a file
//!   whose SHA-256 and header were checked against the bundle. The renderer opens that path and
//!   nothing else; it never joins `relativePath` onto a directory itself.
//! * A [`Generation`] is bumped whenever everything resolved before it must be forgotten (project
//!   switch, reload, context loss, asset swap). A map or an `ApprovedResource` of an older
//!   generation is refused by the session ([`ResolveError::Stale`]).
//!
//! No dependency on the render crate: `tessel-editor-render` already depends on this one, so the
//! seam is the [`ResourceResolver`] trait below and the plain data around it.

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tessel_editor_core::jobs::JobError;
use tessel_editor_core::scene_resources::{
    ResourceKey, ResourceKind, ResourceUse, SkippedResource,
};

use crate::fsx::hex;

/// A monotonically increasing epoch of the resources the editor may hand to a renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Generation(pub u64);

/// One approved file, with every use of it in the bundle (node IDs), so a renderer can release
/// what a deleted node held.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedResource {
    pub key: ResourceKey,
    pub kind: ResourceKind,
    /// Canonical path of the verified file, inside the base.
    pub path: PathBuf,
    /// SHA-256 (lowercase hex) the file was verified against.
    pub content_hash: String,
    pub revision_id: Option<String>,
    pub bytes: u64,
    pub uses: Vec<ResourceUse>,
}

/// Everything one scene needs, approved under one generation.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedResourceMap {
    pub scene_id: String,
    pub bundle_id: String,
    pub bundle_revision: String,
    pub project_id: String,
    pub generation: Generation,
    /// The canonical base every `path` lies in.
    pub base: PathBuf,
    pub entries: Vec<ResolvedResource>,
    /// Declared but not carried (preserved sections, unreferenced assets): never silently lost.
    pub skipped: Vec<SkippedResource>,
    /// The manifest says everything the bundle names is covered. `false` is the usual state
    /// while a bundle has preserved sections (the map is partial, not wrong).
    pub complete: bool,
}

/// What a renderer receives: owned, cheap to clone, and sufficient on its own to open, key and
/// later invalidate a resource.
#[derive(Debug, Clone, PartialEq)]
pub struct ApprovedResource {
    pub scene_id: String,
    pub key: ResourceKey,
    pub kind: ResourceKind,
    pub path: PathBuf,
    pub content_hash: String,
    pub revision_id: Option<String>,
    pub bytes: u64,
    pub generation: Generation,
    /// Safe as a global cache key (see the module docs).
    pub cache_key: String,
    /// Nodes of the scene that name the resource, for releasing by node.
    pub node_ids: Vec<String>,
}

/// Why a resource is not available. Typed so a renderer can tell "load the scene first" from
/// "this asset does not exist" from "everything you hold is old".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// No map is installed for the scene (never loaded, released, or invalidated).
    SceneNotLoaded { scene: String },
    /// The bundle does not declare it.
    UnknownAsset { scene: String, resource: String },
    /// Declared but not carried by this version (preserved sections, unreferenced assets).
    NotCarried { scene: String, resource: String },
    /// The map or resource belongs to a generation that has been invalidated.
    Stale {
        scene: String,
        held: Generation,
        current: Generation,
    },
    /// Approving it would exceed the session's byte budget.
    BudgetExceeded { needed: u64, budget: u64 },
}

impl ResolveError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::SceneNotLoaded { .. } => "scene_resources_not_loaded",
            Self::UnknownAsset { .. } => "resource_unknown",
            Self::NotCarried { .. } => "resource_not_carried",
            Self::Stale { .. } => "stale_generation",
            Self::BudgetExceeded { .. } => "resource_budget_exceeded",
        }
    }
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SceneNotLoaded { scene } => {
                write!(f, "no resources are loaded for scene `{scene}`")
            }
            Self::UnknownAsset { scene, resource } => {
                write!(f, "scene `{scene}` does not declare `{resource}`")
            }
            Self::NotCarried { scene, resource } => write!(
                f,
                "`{resource}` of scene `{scene}` is declared but this version does not carry it"
            ),
            Self::Stale {
                scene,
                held,
                current,
            } => write!(
                f,
                "the resources of scene `{scene}` were approved under generation {} but generation {} is current",
                held.0, current.0
            ),
            Self::BudgetExceeded { needed, budget } => write!(
                f,
                "approving the resources needs {needed} bytes; the session budget is {budget}"
            ),
        }
    }
}

impl std::error::Error for ResolveError {}

impl From<ResolveError> for JobError {
    fn from(error: ResolveError) -> Self {
        let details = match &error {
            ResolveError::Stale { held, current, .. } => {
                json!({ "held": held.0, "current": current.0 })
            }
            ResolveError::BudgetExceeded { needed, budget } => {
                json!({ "needed": needed, "budget": budget })
            }
            _ => json!({}),
        };
        JobError::new(error.code(), error.to_string()).with_details(details)
    }
}

/// The seam between resources and a renderer: the renderer holds a `&dyn ResourceResolver`
/// (implemented by `SceneResourceSession`) and asks by scene and key. It never needs the
/// project, the manifest, the base or any path rule.
pub trait ResourceResolver: Send + Sync {
    fn resolve(&self, scene_id: &str, key: &ResourceKey) -> Result<ApprovedResource, ResolveError>;

    /// The common case: an `assetId`.
    fn resolve_asset(
        &self,
        scene_id: &str,
        asset_id: &str,
    ) -> Result<ApprovedResource, ResolveError> {
        self.resolve(scene_id, &ResourceKey::asset(asset_id))
    }
}

impl ResolvedResourceMap {
    /// Bytes of the approved files, a file shared by several assets counted once. This is the
    /// number the session budget bounds; it is the size on disk, not what a renderer holds once
    /// decoded (the renderer owns its own texture budget).
    pub fn approved_bytes(&self) -> u64 {
        let mut seen = BTreeSet::new();
        self.entries
            .iter()
            .filter(|entry| seen.insert(&entry.path))
            .map(|entry| entry.bytes)
            .sum()
    }

    pub fn get(&self, key: &ResourceKey) -> Option<&ResolvedResource> {
        self.entries.iter().find(|entry| &entry.key == key)
    }

    pub fn approved(&self, key: &ResourceKey) -> Result<ApprovedResource, ResolveError> {
        if let Some(entry) = self.get(key) {
            return Ok(self.approve(entry));
        }
        let resource = key.label();
        if self.skipped.iter().any(|skipped| &skipped.key == key) {
            return Err(ResolveError::NotCarried {
                scene: self.scene_id.clone(),
                resource,
            });
        }
        Err(ResolveError::UnknownAsset {
            scene: self.scene_id.clone(),
            resource,
        })
    }

    fn approve(&self, entry: &ResolvedResource) -> ApprovedResource {
        ApprovedResource {
            scene_id: self.scene_id.clone(),
            key: entry.key.clone(),
            kind: entry.kind,
            path: entry.path.clone(),
            content_hash: entry.content_hash.clone(),
            revision_id: entry.revision_id.clone(),
            bytes: entry.bytes,
            generation: self.generation,
            cache_key: self.cache_key(entry),
            node_ids: entry
                .uses
                .iter()
                .filter_map(|usage| usage.node_id.clone())
                .collect(),
        }
    }

    /// SHA-256 over every component that makes two resolutions interchangeable. NUL separates
    /// the components so `("a", "bc")` and `("ab", "c")` cannot collide.
    pub fn cache_key(&self, entry: &ResolvedResource) -> String {
        let mut hasher = Sha256::new();
        for part in [
            self.project_id.as_str(),
            &self.generation.0.to_string(),
            &self.scene_id,
            &self.bundle_revision,
            &entry.key.label(),
            &entry.content_hash,
        ] {
            hasher.update(part.as_bytes());
            hasher.update([0]);
        }
        hex(&hasher.finalize())
    }

    pub fn to_json(&self) -> Value {
        json!({
            "scene": self.scene_id,
            "bundleId": self.bundle_id,
            "bundleRevision": self.bundle_revision,
            "project": self.project_id,
            "generation": self.generation.0,
            // `null` when the scene names no files and was loaded without a folder.
            "base": Some(&self.base).filter(|base| !base.as_os_str().is_empty()),
            "complete": self.complete,
            "approvedBytes": self.approved_bytes(),
            "resources": self.entries.iter().map(|entry| json!({
                "resource": entry.key.label(),
                "kind": entry.kind.as_str(),
                "path": entry.path,
                "sha256": entry.content_hash,
                "revisionId": entry.revision_id,
                "bytes": entry.bytes,
                "nodes": entry.uses.iter().filter_map(|usage| usage.node_id.clone()).collect::<Vec<_>>(),
                "cacheKey": self.cache_key(entry),
            })).collect::<Vec<_>>(),
            "skipped": self.skipped.iter().map(|skipped| json!({
                "resource": skipped.key.label(),
                "status": skipped.status.as_str(),
                "declaredPath": skipped.declared_path,
            })).collect::<Vec<_>>(),
        })
    }
}
