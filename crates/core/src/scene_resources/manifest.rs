//! The manifest: every file a directed-scene bundle depends on, classified and deterministic.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::directed_scene::{Bundle, DirectedScene, NodeKind};

use super::model::{
    CollisionKind, LISTING_TRUNCATED, ManifestIssue, PathCollision, REFERENCE_IGNORED,
    REFERENCE_MISMATCH, REFERENCE_UNRESOLVED, RESOURCE_FORMAT_MISMATCH, RESOURCE_HASH_INVALID,
    RESOURCE_PATH_COLLISION, RESOURCE_PATH_INVALID, ResourceDependency, ResourceEntry, ResourceKey,
    ResourceKind, ResourceStatus, ResourceUse, Severity, UninterpretedSection, UseRole,
    is_sha256_hex,
};
use super::path::ResourcePath;
use super::scan::{self, RawReference};

/// The resources of one bundle, by identity.
///
/// Built from a `DirectedScene` alone: it needs neither the filesystem nor the evaluated scene
/// plan (H), so a renderer, a packager and a test all ask the same questions of the same IDs.
/// The same bundle always yields the same manifest: entries are ordered by key, uses and issues
/// are sorted, and nothing depends on hash-map iteration.
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceManifest {
    pub(super) bundle_id: String,
    pub(super) revision_id: String,
    pub(super) entries: BTreeMap<ResourceKey, ResourceEntry>,
    pub(super) dependencies: Vec<ResourceDependency>,
    pub(super) uninterpreted: Vec<UninterpretedSection>,
    pub(super) issues: Vec<ManifestIssue>,
    pub(super) collisions: Vec<PathCollision>,
}

impl ResourceManifest {
    /// Never fails: everything wrong with a resource is an entry or an issue, because a bundle
    /// that imported is still worth describing (and a scene with a bad path must say so).
    pub fn from_scene(scene: &DirectedScene) -> Self {
        let bundle = scene.bundle();
        let mut builder = Builder::new(bundle);
        builder.declare_assets();
        builder.add_node_uses();
        builder.add_audio();
        builder.add_preserved_sections(scene.document());
        builder.finish()
    }
}

struct Builder<'a> {
    bundle: &'a Bundle,
    entries: BTreeMap<ResourceKey, ResourceEntry>,
    dependencies: BTreeSet<ResourceDependency>,
    uninterpreted: Vec<UninterpretedSection>,
    issues: Vec<ManifestIssue>,
}

impl<'a> Builder<'a> {
    fn new(bundle: &'a Bundle) -> Self {
        Self {
            bundle,
            entries: BTreeMap::new(),
            dependencies: BTreeSet::new(),
            uninterpreted: Vec::new(),
            issues: Vec::new(),
        }
    }

    /// Every `assets[]` record becomes an entry, referenced or not.
    fn declare_assets(&mut self) {
        for asset in &self.bundle.assets {
            let (transport_path, path_error) = match ResourcePath::parse(&asset.relative_path) {
                Ok(path) => (Some(path), None),
                Err(error) => (None, Some(error)),
            };
            let key = ResourceKey::asset(&asset.asset_id);
            self.entries.entry(key.clone()).or_insert(ResourceEntry {
                key,
                kind: ResourceKind::of_format(asset.format),
                status: ResourceStatus::Unreferenced,
                format: Some(asset.format),
                revision_id: Some(asset.revision_id.clone()),
                content_hash: Some(asset.content_hash.clone()),
                relative_path: asset.relative_path.clone(),
                transport_path,
                path_error,
                width_px: Some(asset.width_px),
                height_px: Some(asset.height_px),
                uses: Vec::new(),
            });
        }
    }

    /// `assetRef` of the nodes the importer interprets: image planes and splats.
    fn add_node_uses(&mut self) {
        let bundle = self.bundle;
        for (scene_index, scene) in bundle.scenes.iter().enumerate() {
            for (node_index, node) in scene.nodes().iter().enumerate() {
                let Some(asset_ref) = &node.asset else {
                    continue;
                };
                let pointer =
                    format!("/sceneDefinitions/{scene_index}/nodes/{node_index}/assetRef");
                let role = match node.kind {
                    NodeKind::ImagePlane => UseRole::NodeImage,
                    NodeKind::Splat => UseRole::NodeSplat,
                    NodeKind::Group => {
                        // A group draws nothing, so the file would be dropped without a trace.
                        self.issues.push(ManifestIssue::new(
                            REFERENCE_IGNORED,
                            Severity::Warning,
                            Some(ResourceKey::asset(&asset_ref.asset_id)),
                            &pointer,
                            format!(
                                "group node {} names asset {}, which no evaluator reads",
                                node.id, asset_ref.asset_id
                            ),
                        ));
                        continue;
                    }
                };
                self.add_use(
                    &asset_ref.asset_id,
                    ResourceUse {
                        role,
                        scene_id: Some(scene.id().to_string()),
                        node_id: Some(node.id.clone()),
                        pointer,
                    },
                );
            }
        }
    }

    fn add_audio(&mut self) {
        let Some(audio) = &self.bundle.audio else {
            return;
        };
        let (transport_path, path_error) = match ResourcePath::parse(&audio.mix_relative_path) {
            Ok(path) => (Some(path), None),
            Err(error) => (None, Some(error)),
        };
        self.entries.insert(
            ResourceKey::AudioMix,
            ResourceEntry {
                key: ResourceKey::AudioMix,
                kind: ResourceKind::Audio,
                status: ResourceStatus::Unreferenced,
                format: None,
                revision_id: None,
                // An empty hash counts as absent, as in the origin.
                content_hash: audio
                    .source_pcm_hash_sha256
                    .clone()
                    .filter(|hash| !hash.is_empty()),
                relative_path: audio.mix_relative_path.clone(),
                transport_path,
                path_error,
                width_px: None,
                height_px: None,
                uses: vec![ResourceUse {
                    role: UseRole::AudioMix,
                    scene_id: None,
                    node_id: None,
                    pointer: "/audio/mixRelativePath".to_string(),
                }],
            },
        );
    }

    fn add_preserved_sections(&mut self, document: &Value) {
        let found = scan::scan(document);
        for reference in &found.references {
            let usage = ResourceUse {
                role: reference.role,
                scene_id: Some(reference.scene_id.clone()),
                node_id: Some(reference.node_id.clone()),
                pointer: reference.pointer.clone(),
            };
            if !self.add_use(&reference.asset_id, usage) {
                continue;
            }
            self.check_declared_reference(reference);
            self.add_dependency(reference);
        }
        for section in &found.sections {
            if section.truncated {
                self.issues.push(ManifestIssue::new(
                    LISTING_TRUNCATED,
                    Severity::Warning,
                    None,
                    &section.pointer,
                    format!(
                        "{} names more files than the manifest lists; the list is incomplete",
                        section.name
                    ),
                ));
            }
        }
        self.uninterpreted = found.sections;
    }

    /// Records a use. `false` when no `assets[]` record has that id (an issue was raised).
    fn add_use(&mut self, asset_id: &str, usage: ResourceUse) -> bool {
        let key = ResourceKey::asset(asset_id);
        let Some(entry) = self.entries.get_mut(&key) else {
            self.issues.push(ManifestIssue::new(
                REFERENCE_UNRESOLVED,
                usage.role.severity(),
                Some(key),
                &usage.pointer,
                format!(
                    "{} names asset {asset_id}, which assets[] does not declare",
                    usage.role.as_str()
                ),
            ));
            return false;
        };
        // For node uses the importer already refuses a wrong format (`ASSET_FORMAT_MISMATCH`), so
        // this is defense in depth there; for preserved sections nothing else checks it.
        let expected = usage.role.expected_format();
        if expected.is_some() && entry.format != expected {
            let (wanted, declared) = (
                expected.map_or("", |format| format.as_str()),
                entry.format.map_or("", |format| format.as_str()),
            );
            self.issues.push(ManifestIssue::new(
                RESOURCE_FORMAT_MISMATCH,
                usage.role.severity(),
                Some(key),
                &usage.pointer,
                format!(
                    "{} expects a {wanted} asset but {asset_id} is declared {declared}",
                    usage.role.as_str()
                ),
            ));
        }
        entry.uses.push(usage);
        true
    }

    /// The origin enforces `assetRef` against `assets[]` for the nodes it interprets (the
    /// importer does too); preserved sections are not validated, so a stale copy is reported.
    fn check_declared_reference(&mut self, reference: &RawReference) {
        let key = ResourceKey::asset(&reference.asset_id);
        let Some(entry) = self.entries.get(&key) else {
            return;
        };
        let mut differing = Vec::new();
        if reference
            .revision_id
            .as_deref()
            .is_some_and(|value| entry.revision_id.as_deref() != Some(value))
        {
            differing.push("revisionId");
        }
        if reference
            .content_hash
            .as_deref()
            .is_some_and(|value| entry.content_hash.as_deref() != Some(value))
        {
            differing.push("contentHash");
        }
        if reference
            .relative_path
            .as_deref()
            .is_some_and(|value| entry.relative_path != value)
        {
            differing.push("relativePath");
        }
        if differing.is_empty() {
            return;
        }
        self.issues.push(ManifestIssue::new(
            REFERENCE_MISMATCH,
            Severity::Warning,
            Some(key),
            &reference.pointer,
            format!(
                "the reference to {} differs from its assets[] record in {}",
                reference.asset_id,
                differing.join(", ")
            ),
        ));
    }

    /// The asset of the node that carries the section needs the asset the section names.
    fn add_dependency(&mut self, reference: &RawReference) {
        let owner = self
            .bundle
            .scene(&reference.scene_id)
            .and_then(|scene| scene.node(&reference.node_id))
            .and_then(|node| node.asset.as_ref());
        let Some(owner) = owner else {
            return;
        };
        let (from, to) = (
            ResourceKey::asset(&owner.asset_id),
            ResourceKey::asset(&reference.asset_id),
        );
        if from == to || !self.entries.contains_key(&from) {
            return;
        }
        self.dependencies.insert(ResourceDependency {
            from,
            to,
            role: reference.role,
        });
    }

    fn finish(mut self) -> ResourceManifest {
        for entry in self.entries.values_mut() {
            entry.uses.sort();
            entry.status = status_of(entry);
        }
        let mut issues = self.issues;
        for entry in self.entries.values() {
            issues.extend(entry_issues(entry));
        }
        let collisions = find_collisions(&self.entries);
        for collision in &collisions {
            issues.push(ManifestIssue::new(
                RESOURCE_PATH_COLLISION,
                Severity::Error,
                None,
                "",
                format!(
                    "{} resources meet at `{}` ({})",
                    collision.kind.as_str(),
                    collision.path,
                    collision
                        .keys
                        .iter()
                        .map(ResourceKey::label)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }
        issues.sort();
        ResourceManifest {
            bundle_id: self.bundle.bundle_id.clone(),
            revision_id: self.bundle.revision_id.clone(),
            entries: self.entries,
            dependencies: self.dependencies.into_iter().collect(),
            uninterpreted: self.uninterpreted,
            issues,
            collisions,
        }
    }
}

fn status_of(entry: &ResourceEntry) -> ResourceStatus {
    if entry.uses.iter().any(|usage| usage.role.is_supported()) {
        return ResourceStatus::Supported;
    }
    if entry.uses.is_empty() {
        return ResourceStatus::Unreferenced;
    }
    ResourceStatus::Uninterpreted
}

/// Path and hash problems of one entry. An unreferenced asset is neither read nor carried, so
/// it raises nothing; a preserved one only warns, because transport does not touch it.
fn entry_issues(entry: &ResourceEntry) -> Vec<ManifestIssue> {
    let severity = match entry.status {
        ResourceStatus::Supported => Severity::Error,
        ResourceStatus::Uninterpreted => Severity::Warning,
        ResourceStatus::Unreferenced => return Vec::new(),
    };
    let pointer = entry
        .uses
        .first()
        .map(|usage| usage.pointer.as_str())
        .unwrap_or_default();
    let mut issues = Vec::new();
    if let Some(error) = &entry.path_error {
        issues.push(ManifestIssue::new(
            RESOURCE_PATH_INVALID,
            severity,
            Some(entry.key.clone()),
            pointer,
            format!(
                "{} declares `{}`: {error}",
                entry.key.label(),
                entry.relative_path
            ),
        ));
    }
    let hash_ok = entry.content_hash.as_deref().is_some_and(is_sha256_hex);
    if !hash_ok {
        issues.push(ManifestIssue::new(
            RESOURCE_HASH_INVALID,
            severity,
            Some(entry.key.clone()),
            pointer,
            format!(
                "{} has no SHA-256 (64 lowercase hex digits) to verify its file against",
                entry.key.label()
            ),
        ));
    }
    issues
}

/// Supported resources that cannot share one transport root. Preserved and unreferenced ones
/// are never written, so they cannot collide.
fn find_collisions(entries: &BTreeMap<ResourceKey, ResourceEntry>) -> Vec<PathCollision> {
    let mut by_key: BTreeMap<String, Vec<&ResourceEntry>> = BTreeMap::new();
    for entry in entries.values().filter(|entry| entry.is_supported()) {
        if let Some(path) = &entry.transport_path {
            by_key.entry(path.collision_key()).or_default().push(entry);
        }
    }
    let mut collisions = Vec::new();
    for (path, group) in &by_key {
        let first = group[0];
        let same_path = group
            .iter()
            .all(|entry| entry.transport_path == first.transport_path);
        let same_hash = group
            .iter()
            .all(|entry| entry.content_hash == first.content_hash);
        if same_path && same_hash {
            continue;
        }
        let kind = match same_path {
            true => CollisionKind::DifferentContent,
            false => CollisionKind::CaseOnly,
        };
        collisions.push(PathCollision {
            kind,
            path: path.clone(),
            keys: group.iter().map(|entry| entry.key.clone()).collect(),
        });
    }
    for (path, group) in &by_key {
        for (index, _) in path.match_indices('/') {
            let Some(folder) = by_key.get(&path[..index]) else {
                continue;
            };
            let mut keys: Vec<ResourceKey> = folder
                .iter()
                .chain(group.iter())
                .map(|entry| entry.key.clone())
                .collect();
            keys.sort();
            keys.dedup();
            collisions.push(PathCollision {
                kind: CollisionKind::FileIsDirectory,
                path: path[..index].to_string(),
                keys,
            });
        }
    }
    collisions.sort();
    collisions
}
