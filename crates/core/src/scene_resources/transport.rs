//! The pure plan of what a transport root must hold: which files, where, with which hash.
//! The media adapter executes it; nothing here touches a filesystem.

use std::collections::BTreeMap;

use super::manifest::ResourceManifest;
use super::model::{
    ManifestIssue, RESOURCE_HASH_INVALID, RESOURCE_PATH_INVALID, ResourceKey, ResourceKind,
    ResourceStatus, Severity,
};
use super::path::ResourcePath;

/// One resource that lands on a `TransportItem`'s file, keeping its own identity and origin path.
#[derive(Debug, Clone, PartialEq)]
pub struct TransportResource {
    pub key: ResourceKey,
    pub kind: ResourceKind,
    /// The `relativePath` as declared in the bundle (the origin).
    pub declared_path: String,
    pub width_px: Option<u64>,
    pub height_px: Option<f64>,
}

/// A file of the transport root. Several assets may share it when they declare the same path
/// and the same hash: the file is carried once, every asset keeps its `assetId` and path.
#[derive(Debug, Clone, PartialEq)]
pub struct TransportItem {
    pub path: ResourcePath,
    /// SHA-256, lowercase hex.
    pub expected_hash: String,
    pub resources: Vec<TransportResource>,
}

/// A resource the transport does not carry, with why. Never silently dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedResource {
    pub key: ResourceKey,
    pub status: ResourceStatus,
    pub declared_path: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransportPlan {
    /// Ordered by case-folded path.
    pub items: Vec<TransportItem>,
    pub skipped: Vec<SkippedResource>,
}

impl ResourceManifest {
    /// The files a transport root needs for the supported resources.
    ///
    /// `Err` carries the blocking issues (unsafe path, bad hash, collision, format mismatch,
    /// dangling reference of a supported use): a plan built over them would be a guess.
    pub fn transport_plan(&self) -> Result<TransportPlan, Vec<ManifestIssue>> {
        let blocking: Vec<ManifestIssue> = self.blocking_issues().cloned().collect();
        if !blocking.is_empty() {
            return Err(blocking);
        }
        let mut grouped: BTreeMap<String, TransportItem> = BTreeMap::new();
        let mut skipped = Vec::new();
        for entry in self.entries() {
            if !entry.is_supported() {
                skipped.push(SkippedResource {
                    key: entry.key.clone(),
                    status: entry.status,
                    declared_path: entry.relative_path.clone(),
                });
                continue;
            }
            // Both exist whenever there is no blocking issue; failing loudly beats dropping a file.
            let (Some(path), Some(hash)) = (&entry.transport_path, &entry.content_hash) else {
                let code = match entry.transport_path {
                    None => RESOURCE_PATH_INVALID,
                    Some(_) => RESOURCE_HASH_INVALID,
                };
                return Err(vec![ManifestIssue::new(
                    code,
                    Severity::Error,
                    Some(entry.key.clone()),
                    "",
                    format!("{} lacks a path or a hash", entry.key.label()),
                )]);
            };
            let resource = TransportResource {
                key: entry.key.clone(),
                kind: entry.kind,
                declared_path: entry.relative_path.clone(),
                width_px: entry.width_px,
                height_px: entry.height_px,
            };
            grouped
                .entry(path.collision_key())
                .or_insert_with(|| TransportItem {
                    path: path.clone(),
                    expected_hash: hash.clone(),
                    resources: Vec::new(),
                })
                .resources
                .push(resource);
        }
        Ok(TransportPlan {
            items: grouped.into_values().collect(),
            skipped,
        })
    }
}
