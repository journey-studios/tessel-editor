//! Questions a renderer, a packager or a test asks of a manifest, all by `assetId`.

use std::collections::{BTreeSet, VecDeque};

use serde_json::{Value, json};

use super::MANIFEST_VERSION;
use super::manifest::ResourceManifest;
use super::model::{
    ManifestIssue, PathCollision, ResourceDependency, ResourceEntry, ResourceKey, ResourceUse,
    Severity, UninterpretedSection,
};

/// The answer to asking for a list of `assetId`s: what exists, in the order asked, and what the
/// bundle does not declare. An unknown id is information, not a panic or a silent skip.
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceLookup<'a> {
    pub found: Vec<&'a ResourceEntry>,
    pub unknown: Vec<String>,
}

impl ResourceManifest {
    pub fn bundle_id(&self) -> &str {
        &self.bundle_id
    }

    pub fn revision_id(&self) -> &str {
        &self.revision_id
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every resource, ordered by key (`assetId`, then the audio mix).
    pub fn entries(&self) -> impl Iterator<Item = &ResourceEntry> {
        self.entries.values()
    }

    /// The ones a frame needs and transport carries.
    pub fn supported(&self) -> impl Iterator<Item = &ResourceEntry> {
        self.entries().filter(|entry| entry.is_supported())
    }

    pub fn get(&self, key: &ResourceKey) -> Option<&ResourceEntry> {
        self.entries.get(key)
    }

    /// A resource by the identity the bundle gives it. Not its path, not its hash.
    pub fn resource(&self, asset_id: &str) -> Option<&ResourceEntry> {
        self.entries.get(&ResourceKey::asset(asset_id))
    }

    pub fn audio_mix(&self) -> Option<&ResourceEntry> {
        self.entries.get(&ResourceKey::AudioMix)
    }

    pub fn lookup<'a>(&'a self, asset_ids: &[&str]) -> ResourceLookup<'a> {
        let mut seen = BTreeSet::new();
        let mut lookup = ResourceLookup {
            found: Vec::new(),
            unknown: Vec::new(),
        };
        for asset_id in asset_ids {
            if !seen.insert(*asset_id) {
                continue;
            }
            match self.resource(asset_id) {
                Some(entry) => lookup.found.push(entry),
                None => lookup.unknown.push((*asset_id).to_string()),
            }
        }
        lookup
    }

    /// Where a resource is named, sorted. Empty for an unknown id or an unreferenced asset.
    pub fn uses_of(&self, asset_id: &str) -> &[ResourceUse] {
        self.resource(asset_id)
            .map(|entry| entry.uses.as_slice())
            .unwrap_or_default()
    }

    pub fn dependencies(&self) -> &[ResourceDependency] {
        &self.dependencies
    }

    /// What `asset_id` needs directly.
    pub fn dependencies_of(&self, asset_id: &str) -> Vec<&ResourceDependency> {
        let key = ResourceKey::asset(asset_id);
        self.dependencies
            .iter()
            .filter(|dependency| dependency.from == key)
            .collect()
    }

    /// `asset_id` and everything it needs, transitively, ordered by key. `None` for an unknown id.
    pub fn closure(&self, asset_id: &str) -> Option<Vec<&ResourceEntry>> {
        let key = ResourceKey::asset(asset_id);
        self.entries.get(&key)?;
        Some(self.entries_of(&self.closure_of_keys([key])))
    }

    /// The resources a scene definition names, with what they need transitively.
    pub fn for_scene(&self, scene_id: &str) -> Vec<&ResourceEntry> {
        let roots = self
            .entries()
            .filter(|entry| {
                entry
                    .uses
                    .iter()
                    .any(|usage| usage.scene_id.as_deref() == Some(scene_id))
            })
            .map(|entry| entry.key.clone());
        self.entries_of(&self.closure_of_keys(roots))
    }

    /// The resources one node names directly (a node id is unique only inside its scene).
    pub fn for_node(&self, scene_id: &str, node_id: &str) -> Vec<&ResourceEntry> {
        self.entries()
            .filter(|entry| {
                entry.uses.iter().any(|usage| {
                    usage.scene_id.as_deref() == Some(scene_id)
                        && usage.node_id.as_deref() == Some(node_id)
                })
            })
            .collect()
    }

    pub fn uninterpreted(&self) -> &[UninterpretedSection] {
        &self.uninterpreted
    }

    pub fn issues(&self) -> &[ManifestIssue] {
        &self.issues
    }

    pub fn blocking_issues(&self) -> impl Iterator<Item = &ManifestIssue> {
        self.issues
            .iter()
            .filter(|issue| issue.severity == Severity::Error)
    }

    pub fn collisions(&self) -> &[PathCollision] {
        &self.collisions
    }

    /// No error stands between the supported resources and a transport root.
    pub fn is_transportable(&self) -> bool {
        self.blocking_issues().next().is_none()
    }

    /// Transport would carry everything the bundle names: no preserved section holds files
    /// that stay behind and nothing is dangling or stale. A transportable manifest can be
    /// incomplete; that is what "partial" means for BB06.
    pub fn is_complete(&self) -> bool {
        self.uninterpreted.is_empty() && self.issues.is_empty()
    }

    fn closure_of_keys(
        &self,
        roots: impl IntoIterator<Item = ResourceKey>,
    ) -> BTreeSet<ResourceKey> {
        let mut seen = BTreeSet::new();
        let mut queue: VecDeque<ResourceKey> = VecDeque::new();
        for root in roots {
            if seen.insert(root.clone()) {
                queue.push_back(root);
            }
        }
        while let Some(current) = queue.pop_front() {
            for dependency in self
                .dependencies
                .iter()
                .filter(|dependency| dependency.from == current)
            {
                if seen.insert(dependency.to.clone()) {
                    queue.push_back(dependency.to.clone());
                }
            }
        }
        seen
    }

    fn entries_of(&self, keys: &BTreeSet<ResourceKey>) -> Vec<&ResourceEntry> {
        keys.iter()
            .filter_map(|key| self.entries.get(key))
            .collect()
    }

    /// A stable JSON form: same manifest, same text. Object keys are sorted by `serde_json`
    /// and every list is in manifest order.
    pub fn to_json(&self) -> Value {
        json!({
            "manifestVersion": MANIFEST_VERSION,
            "bundleId": self.bundle_id,
            "revisionId": self.revision_id,
            "transportable": self.is_transportable(),
            "complete": self.is_complete(),
            "resources": self.entries().map(entry_json).collect::<Vec<_>>(),
            "dependencies": self.dependencies.iter().map(|dependency| json!({
                "from": dependency.from.label(),
                "to": dependency.to.label(),
                "role": dependency.role.as_str(),
            })).collect::<Vec<_>>(),
            "uninterpreted": self.uninterpreted.iter().map(|section| json!({
                "pointer": section.pointer,
                "name": section.name,
                "sceneId": section.scene_id,
                "nodeId": section.node_id,
                "assetIds": section.asset_ids,
                "files": section.files.iter().map(|file| json!({
                    "pointer": file.pointer,
                    "relativePath": file.relative_path,
                })).collect::<Vec<_>>(),
                "truncated": section.truncated,
            })).collect::<Vec<_>>(),
            "issues": self.issues.iter().map(|issue| json!({
                "code": issue.code,
                "severity": issue.severity.as_str(),
                "resource": issue.key.as_ref().map(ResourceKey::label),
                "pointer": issue.pointer,
                "message": issue.message,
            })).collect::<Vec<_>>(),
            "collisions": self.collisions.iter().map(|collision| json!({
                "kind": collision.kind.as_str(),
                "path": collision.path,
                "resources": collision.keys.iter().map(ResourceKey::label).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        })
    }
}

fn entry_json(entry: &ResourceEntry) -> Value {
    json!({
        "key": entry.key.label(),
        "assetId": entry.asset_id(),
        "kind": entry.kind.as_str(),
        "status": entry.status.as_str(),
        "format": entry.format.map(|format| format.as_str()),
        "revisionId": entry.revision_id,
        "contentHash": entry.content_hash,
        "relativePath": entry.relative_path,
        "transportPath": entry.transport_path.as_ref().map(|path| path.as_str()),
        "widthPx": entry.width_px,
        "heightPx": entry.height_px,
        "uses": entry.uses.iter().map(|usage| json!({
            "role": usage.role.as_str(),
            "sceneId": usage.scene_id,
            "nodeId": usage.node_id,
            "pointer": usage.pointer,
        })).collect::<Vec<_>>(),
    })
}
