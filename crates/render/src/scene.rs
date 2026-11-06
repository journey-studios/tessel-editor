//! The seam between a resolved directed-scene plan and the Three.js runtime (BL007/BL063).
//!
//! Nothing here touches a process, a window or a file's bytes. It turns what the core decided
//! (a [`ScenePlan`]) and what an adapter resolved (a [`SceneResourceMap`]: asset id → an approved
//! local file with the revision and content hash it holds) into the bridge-v1 messages a runtime
//! needs, and it keeps the book of which picture sits in which runtime slot. The headless
//! renderer ([`crate::SceneRenderer`]) and the desktop monitor both feed the same messages to the
//! same runtime page; only the transport differs. Contract: `docs/features/scene-renderer.md`.
//!
//! Identity rules (the runtime enforces the same ones, see `runtime/src/directed-scene.js`):
//!
//! - a resource is `(generation, assetId, revisionId, contentHash)`; the asset id alone is never a
//!   key. The same asset id with another revision or hash is another resource in another slot;
//! - `generation` is the host's project generation: it grows when a project is opened, replaced
//!   or relinked. A map of an older generation than one already used is refused;
//! - runtime slots are a small fixed set of ids (`scene-res-0`...), so the runtime's id space does
//!   not grow with the number of projects or revisions opened.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tessel_editor_core::backend::BackendError;
use tessel_editor_core::directed_scene::{DirectedScene, Issue};
use tessel_editor_core::scene_plan::{PlanError, PlanResource, ScenePlan};
use tessel_editor_core::splat::{self, SplatFormat};

/// `kind` of the layer a runtime that has the `directed_scene` feature draws.
pub const DIRECTED_LAYER_KIND: &str = "directed_scene";
/// Name under which the runtime lists the feature in `ready.capabilities.features`.
pub const DIRECTED_FEATURE: &str = "directed_scene";
/// Slots the host keeps loaded at most: one decoded picture each (and, past the runtime's own
/// bound of 64, the runtime refuses the load with `resource_budget`).
pub const DEFAULT_SLOTS: usize = 32;
/// Decoded RGBA bytes (`width * height * 4`) the host lets its slots hold at once.
pub const DEFAULT_BYTE_BUDGET: u64 = 512 * 1024 * 1024;
/// Splats a frame may draw in all, the sum over its splat *nodes* of the splats of the file each
/// reads (a file used by many nodes counts once per node: each is sorted and drawn, even though the
/// decode is shared). The runtime's `LIMITS.frameSplats` is the same number.
pub const DEFAULT_FRAME_SPLATS: u64 = 8 * 1024 * 1024;

fn error(code: &str, message: impl Into<String>) -> BackendError {
    BackendError::new(code, message)
}

/// A plan error as the bridge reports it: the same `code`, and the list it carries as `details`.
pub fn plan_error(error: &PlanError) -> BackendError {
    let converted = BackendError::new(error.code(), error.to_string());
    match error.details() {
        Some(details) => converted.with_details(details),
        None => converted,
    }
}

fn issue_json(issue: &Issue) -> Value {
    json!({ "code": issue.code, "path": issue.path, "message": issue.message })
}

/// Imports a render bundle and runs the structural and temporal gates the origin's
/// `finalizeBundle` runs, so a renderer never draws a bundle the core itself would refuse (a cut
/// that jumps the camera, a layered field of view out of range). `bundle_invalid` carries the
/// issues as `details.issues`.
pub fn import_gated(text: &str) -> Result<DirectedScene, BackendError> {
    let scene = DirectedScene::import_str(text)
        .map_err(|failure| error("bundle_invalid", failure.to_string()))?;
    let mut issues = scene.edit_join_issues();
    issues.extend(scene.temporal_issues());
    if issues.is_empty() {
        return Ok(scene);
    }
    Err(error(
        "bundle_invalid",
        format!(
            "The bundle has {} issue(s); the first is {}: {}",
            issues.len(),
            issues[0].code,
            issues[0].message
        ),
    )
    .with_details(json!({ "issues": issues.iter().map(issue_json).collect::<Vec<_>>() })))
}

/// One resource an adapter resolved: the file to read and the identity it vouches for.
///
/// The render crate does not hash the file. `content_hash` is what the resolver (the media
/// adapter that materialized or verified the file) says it holds; the runtime compares it with
/// what the plan requires and refuses a mismatch, but proving that the bytes really have that
/// hash is the resolver's job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedResource {
    pub revision_id: String,
    pub content_hash: String,
    /// Absolute path of a local file the host approved for this project.
    pub path: PathBuf,
}

/// The resources of one project generation, by asset id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneResourceMap {
    generation: u64,
    entries: BTreeMap<String, ResolvedResource>,
}

impl SceneResourceMap {
    pub fn new(generation: u64) -> Self {
        Self {
            generation,
            entries: BTreeMap::new(),
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn get(&self, asset_id: &str) -> Option<&ResolvedResource> {
        self.entries.get(asset_id)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Adds (or replaces) the resolution of `asset_id`. The path must be absolute: a relative one
    /// would depend on the working directory, which no resource may.
    pub fn insert(
        &mut self,
        asset_id: impl Into<String>,
        resource: ResolvedResource,
    ) -> Result<(), BackendError> {
        let asset_id = asset_id.into();
        if !resource.path.is_absolute() {
            return Err(error(
                "resource_path_invalid",
                format!(
                    "The path of `{asset_id}` must be absolute, got `{}`",
                    resource.path.display()
                ),
            ));
        }
        self.entries.insert(asset_id, resource);
        Ok(())
    }

    pub fn with(
        mut self,
        asset_id: impl Into<String>,
        resource: ResolvedResource,
    ) -> Result<Self, BackendError> {
        self.insert(asset_id, resource)?;
        Ok(self)
    }

    /// Resolves every resource of `plan` against `root`, an explicit directory the host approved
    /// (the bundle's own folder, or a restored package's root). The plan's `relativePath` is
    /// already known to be relative and free of `..`; this also refuses a symlink that leaves
    /// `root`. Revision and hash are the plan's own: the root is trusted to hold those files.
    pub fn from_root(generation: u64, plan: &ScenePlan, root: &Path) -> Result<Self, BackendError> {
        let canonical_root = root.canonicalize().map_err(|failure| {
            error(
                "resource_unresolved",
                format!(
                    "The resource root `{}` is not readable: {failure}",
                    root.display()
                ),
            )
        })?;
        let mut map = Self::new(generation);
        for resource in &plan.resources {
            let path = canonical_root.join(&resource.relative_path);
            let canonical = path.canonicalize().map_err(|failure| {
                error(
                    "resource_unresolved",
                    format!(
                        "`{}` ({}) is not readable under the root: {failure}",
                        resource.asset_id, resource.relative_path
                    ),
                )
                .with_details(json!({ "asset_id": resource.asset_id }))
            })?;
            if !canonical.starts_with(&canonical_root) {
                return Err(error(
                    "resource_path_invalid",
                    format!("`{}` resolves outside the resource root", resource.asset_id),
                )
                .with_details(json!({ "asset_id": resource.asset_id })));
            }
            map.insert(
                resource.asset_id.clone(),
                ResolvedResource {
                    revision_id: resource.revision_id.clone(),
                    content_hash: resource.content_hash.clone(),
                    path: canonical,
                },
            )?;
        }
        Ok(map)
    }

    /// `{ "generation": 3, "resources": { "<assetId>": { "revisionId", "contentHash", "path" } } }`.
    pub fn from_json(value: &Value) -> Result<Self, BackendError> {
        let invalid = |message: &str| error("invalid_request", format!("Resource map: {message}"));
        let generation = value["generation"]
            .as_u64()
            .ok_or_else(|| invalid("`generation` must be a non-negative integer"))?;
        let resources = value["resources"]
            .as_object()
            .ok_or_else(|| invalid("`resources` must be an object keyed by asset id"))?;
        let mut map = Self::new(generation);
        for (asset_id, entry) in resources {
            let text = |key: &str| {
                entry[key]
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| invalid(&format!("`{asset_id}` needs a string `{key}`")))
            };
            map.insert(
                asset_id.clone(),
                ResolvedResource {
                    revision_id: text("revisionId")?,
                    content_hash: text("contentHash")?,
                    path: PathBuf::from(text("path")?),
                },
            )?;
        }
        Ok(map)
    }
}

/// What identifies a loaded picture in the runtime.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Identity {
    generation: u64,
    asset_id: String,
    revision_id: String,
    content_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotState {
    Loading,
    Ready,
}

#[derive(Debug, Clone)]
struct Slot {
    name: String,
    held: Option<(Identity, SlotState, u64)>,
    last_used: u64,
}

/// A picture the runtime has to load before the frame can be drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingLoad {
    pub slot: String,
    /// The `asset.load` message (bridge v1, one more field `scene` on the descriptor).
    pub message: Value,
}

/// Everything a host sends for one frame, in this order: `releases`, `loads` (wait for each
/// `asset.state` of [`pending_slots`](Self::pending_slots) to say `ready`), then the frame built
/// with [`frame_render_message`] around `layer`.
#[derive(Debug, Clone, PartialEq)]
pub struct Prepared {
    /// `asset.release` messages for slots of an older generation.
    pub releases: Vec<Value>,
    pub loads: Vec<PendingLoad>,
    /// Slots the frame needs that are loaded by an earlier request but not confirmed yet.
    pub waiting: Vec<String>,
    /// The `directed_scene` layer of the `frame.render` plan.
    pub layer: Value,
}

impl Prepared {
    /// Slots whose `asset.state` the host has to see `ready` before it sends the frame.
    pub fn pending_slots(&self) -> Vec<&str> {
        self.loads
            .iter()
            .map(|load| load.slot.as_str())
            .chain(self.waiting.iter().map(String::as_str))
            .collect()
    }
}

/// The book of runtime slots. One per runtime instance; if the runtime restarts (a new process, a
/// reloaded page) call [`reset`](Self::reset), because everything it held is gone.
#[derive(Debug, Clone)]
pub struct SceneSlots {
    slots: Vec<Slot>,
    byte_budget: u64,
    frame_splats: u64,
    clock: u64,
    /// Highest generation any map has used.
    generation: u64,
}

impl Default for SceneSlots {
    fn default() -> Self {
        Self::new(DEFAULT_SLOTS, DEFAULT_BYTE_BUDGET)
    }
}

/// What the runtime holds for a picture: its decoded RGBA. The plan stores the height as a float;
/// for a picture it is a whole number of pixels.
fn picture_bytes(resource: &PlanResource) -> u64 {
    (resource.width_px as f64 * resource.height_px * 4.0) as u64
}

/// What the runtime holds for a splat file: the kept bytes plus 16 packed bytes per splat (the
/// same count the runtime's own budget uses).
fn splat_bytes(length: u64, count: u64) -> u64 {
    length.saturating_add(count.saturating_mul(16))
}

impl SceneSlots {
    pub fn new(capacity: usize, byte_budget: u64) -> Self {
        Self {
            slots: (0..capacity.max(1))
                .map(|index| Slot {
                    name: format!("scene-res-{index}"),
                    held: None,
                    last_used: 0,
                })
                .collect(),
            byte_budget,
            frame_splats: DEFAULT_FRAME_SPLATS,
            clock: 0,
            generation: 0,
        }
    }

    /// The most splats one frame may draw across all its splat nodes (see `DEFAULT_FRAME_SPLATS`).
    pub fn with_frame_splats(mut self, limit: u64) -> Self {
        self.frame_splats = limit;
        self
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Slots that hold something (loading or ready).
    pub fn occupied(&self) -> usize {
        self.slots.iter().filter(|slot| slot.held.is_some()).count()
    }

    /// The slot that holds exactly this identity, if any.
    pub fn slot_of(
        &self,
        generation: u64,
        asset_id: &str,
        revision_id: &str,
        content_hash: &str,
    ) -> Option<&str> {
        self.slots
            .iter()
            .find(|slot| {
                slot.held.as_ref().is_some_and(|(identity, _, _)| {
                    identity.generation == generation
                        && identity.asset_id == asset_id
                        && identity.revision_id == revision_id
                        && identity.content_hash == content_hash
                })
            })
            .map(|slot| slot.name.as_str())
    }

    /// Decides what the runtime must load and release for `plan`, and builds the layer.
    ///
    /// Errors, all typed: `unsupported` (a partial plan), `resource_unresolved` (the map has no
    /// entry or the file is not there), `resource_mismatch` (the map vouches for another revision
    /// or hash than the plan requires), `resource_stale` (a map of a generation older than one
    /// already used), `resource_budget` (the plan needs more than the slots or bytes allow).
    /// A refusal changes nothing: the book is only updated when the whole plan is accepted.
    pub fn prepare(
        &mut self,
        plan: &ScenePlan,
        resources: &SceneResourceMap,
    ) -> Result<Prepared, BackendError> {
        if !plan.is_complete() {
            let mut refusal = error(
                "unsupported",
                "The plan declares domains it does not carry; a renderer draws complete plans only",
            );
            if let Some(details) = (PlanError::Unsupported {
                domains: plan.unsupported.clone(),
            })
            .details()
            {
                refusal = refusal.with_details(details);
            }
            return Err(refusal);
        }
        plan.validate().map_err(|failure| plan_error(&failure))?;
        if resources.generation < self.generation {
            return Err(error(
                "resource_stale",
                format!(
                    "The resource map is of generation {}; generation {} is already in use",
                    resources.generation, self.generation
                ),
            )
            .with_details(
                json!({ "generation": resources.generation, "current": self.generation }),
            ));
        }

        let wanted = self.resolve(plan, resources)?;
        let needed_bytes: u64 = wanted.iter().map(|(_, _, _, bytes)| bytes).sum();
        if needed_bytes > self.byte_budget || wanted.len() > self.slots.len() {
            return Err(error(
                "resource_budget",
                format!(
                    "The frame needs {} resources ({needed_bytes} bytes); the host keeps at most {} ({} bytes)",
                    wanted.len(),
                    self.slots.len(),
                    self.byte_budget
                ),
            )
            .with_details(json!({
                "resources": wanted.len(), "bytes": needed_bytes,
                "max_resources": self.slots.len(), "max_bytes": self.byte_budget,
            })));
        }

        // From here the plan is accepted: nothing below can fail, so the book is never half updated.
        let mut releases = Vec::new();
        if resources.generation > self.generation {
            for slot in &mut self.slots {
                let older = slot
                    .held
                    .as_ref()
                    .is_some_and(|(identity, _, _)| identity.generation < resources.generation);
                if older {
                    slot.held = None;
                    releases.push(release_message(&slot.name));
                }
            }
            self.generation = resources.generation;
        }

        self.clock += 1;
        let identities: Vec<&Identity> =
            wanted.iter().map(|(_, identity, _, _)| identity).collect();
        let incoming: u64 = wanted
            .iter()
            .filter(|(_, identity, _, _)| self.index_of(identity).is_none())
            .map(|(_, _, _, bytes)| *bytes)
            .sum();
        releases.extend(self.evict_to_fit(&identities, incoming));

        let mut loads = Vec::new();
        let mut waiting = Vec::new();
        let mut references = Vec::new();
        for (resource, identity, path, bytes) in &wanted {
            // `wanted.len() <= slots.len()` was checked, and every slot the frame already uses
            // is excluded from eviction, so a slot is always left for a picture that has none.
            let Some(index) = self
                .index_of(identity)
                .or_else(|| self.claim_slot(&identities, &mut releases))
            else {
                return Err(error(
                    "resource_budget",
                    "No runtime slot is left for the plan",
                ));
            };
            let slot = &mut self.slots[index];
            slot.last_used = self.clock;
            match &slot.held {
                Some((_, SlotState::Ready, _)) => {}
                Some((_, SlotState::Loading, _)) => waiting.push(slot.name.clone()),
                None => {
                    slot.held = Some((identity.clone(), SlotState::Loading, *bytes));
                    loads.push(PendingLoad {
                        slot: slot.name.clone(),
                        message: load_message(&slot.name, identity, resource, path),
                    });
                }
            }
            references.push(json!({
                "asset_id": identity.asset_id, "slot": slot.name,
                "revision_id": identity.revision_id, "content_hash": identity.content_hash,
                "generation": identity.generation,
            }));
        }
        Ok(Prepared {
            releases,
            loads,
            waiting,
            layer: directed_layer(plan.to_json(), references),
        })
    }

    fn index_of(&self, identity: &Identity) -> Option<usize> {
        self.slots.iter().position(|slot| {
            slot.held
                .as_ref()
                .is_some_and(|(held, _, _)| held == identity)
        })
    }

    /// An empty slot, or else the least recently used one the frame does not need (released first:
    /// the runtime must not keep the old picture of a slot the book now calls free).
    fn claim_slot(&mut self, needed: &[&Identity], releases: &mut Vec<Value>) -> Option<usize> {
        let evictable = |slot: &Slot| {
            slot.held
                .as_ref()
                .is_none_or(|(identity, _, _)| !needed.contains(&identity))
        };
        let index = (0..self.slots.len())
            .filter(|&index| evictable(&self.slots[index]))
            .min_by_key(|&index| {
                (
                    self.slots[index].held.is_some(),
                    self.slots[index].last_used,
                )
            })?;
        if self.slots[index].held.take().is_some() {
            releases.push(release_message(&self.slots[index].name));
        }
        Some(index)
    }

    /// Forgets (and releases) the least recently used pictures the frame does not need while the
    /// slots hold more than the byte budget allows once `incoming` bytes are added.
    fn evict_to_fit(&mut self, needed: &[&Identity], incoming: u64) -> Vec<Value> {
        let mut releases = Vec::new();
        loop {
            let held: u64 = self
                .slots
                .iter()
                .filter_map(|slot| slot.held.as_ref().map(|(_, _, bytes)| *bytes))
                .sum();
            if held + incoming <= self.byte_budget {
                return releases;
            }
            let victim = self
                .slots
                .iter()
                .enumerate()
                .filter(|(_, slot)| {
                    slot.held
                        .as_ref()
                        .is_some_and(|(identity, _, _)| !needed.contains(&identity))
                })
                .min_by_key(|(_, slot)| slot.last_used)
                .map(|(index, _)| index);
            let Some(index) = victim else {
                return releases;
            };
            self.slots[index].held = None;
            releases.push(release_message(&self.slots[index].name));
        }
    }

    /// The identity, the file and the plan resource of everything the plan needs, checked.
    fn resolve<'a>(
        &self,
        plan: &'a ScenePlan,
        resources: &SceneResourceMap,
    ) -> Result<Vec<(&'a PlanResource, Identity, PathBuf, u64)>, BackendError> {
        let mut wanted = Vec::new();
        let mut frame_splats: u64 = 0;
        for resource in &plan.resources {
            let asset_id = &resource.asset_id;
            let resolved = resources.get(asset_id).ok_or_else(|| {
                error(
                    "resource_unresolved",
                    format!("No resolved file for `{asset_id}`"),
                )
                .with_details(json!({ "asset_id": asset_id }))
            })?;
            if resolved.revision_id != resource.revision_id
                || resolved.content_hash != resource.content_hash
            {
                return Err(error(
                    "resource_mismatch",
                    format!("`{asset_id}` was resolved for another revision or content than the plan requires"),
                )
                .with_details(json!({
                    "asset_id": asset_id,
                    "expected": { "revision_id": resource.revision_id, "content_hash": resource.content_hash },
                    "got": { "revision_id": resolved.revision_id, "content_hash": resolved.content_hash },
                })));
            }
            if !resolved.path.is_file() {
                return Err(error(
                    "resource_unresolved",
                    format!(
                        "`{asset_id}` is not a readable file at `{}`",
                        resolved.path.display()
                    ),
                )
                .with_details(json!({ "asset_id": asset_id })));
            }
            let bytes = if resource.is_splat() {
                match splat::inspect(&resolved.path) {
                    Ok(info) if info.format == SplatFormat::Spz => {
                        // Every node that reads the file sorts and draws all of its splats.
                        let nodes = resource.used_by.len().max(1) as u64;
                        frame_splats =
                            frame_splats.saturating_add(info.count.saturating_mul(nodes));
                        splat_bytes(
                            std::fs::metadata(&resolved.path).map_or(0, |meta| meta.len()),
                            info.count,
                        )
                    }
                    _ => {
                        return Err(error(
                            "resource_mismatch",
                            format!(
                                "`{asset_id}` is planned as an SPZ splat but the file is not one"
                            ),
                        )
                        .with_details(
                            json!({ "asset_id": asset_id, "expected": { "format": "spz" } }),
                        ));
                    }
                }
            } else {
                picture_bytes(resource)
            };
            wanted.push((
                resource,
                Identity {
                    generation: resources.generation,
                    asset_id: asset_id.clone(),
                    revision_id: resource.revision_id.clone(),
                    content_hash: resource.content_hash.clone(),
                },
                resolved.path.clone(),
                bytes,
            ));
        }
        if frame_splats > self.frame_splats {
            return Err(error(
                "resource_budget",
                format!(
                    "The frame would draw {frame_splats} splats across its splat nodes; the host allows {}",
                    self.frame_splats
                ),
            )
            .with_details(json!({
                "frame_splats": frame_splats, "max_frame_splats": self.frame_splats,
            })));
        }
        Ok(wanted)
    }

    /// Whether the runtime confirmed the picture of `slot`.
    pub fn is_ready(&self, slot: &str) -> bool {
        self.slots
            .iter()
            .any(|entry| entry.name == slot && matches!(entry.held, Some((_, SlotState::Ready, _))))
    }

    /// The runtime confirmed the picture of `slot` (`asset.state` `ready`).
    pub fn mark_ready(&mut self, slot: &str) {
        if let Some(entry) = self.slots.iter_mut().find(|entry| entry.name == slot)
            && let Some((_, state, _)) = entry.held.as_mut()
        {
            *state = SlotState::Ready;
        }
    }

    /// The runtime failed to load `slot` (`asset.state` `failed`): forget it, so the next
    /// request loads it again instead of drawing against a slot that holds nothing.
    pub fn mark_failed(&mut self, slot: &str) {
        if let Some(entry) = self.slots.iter_mut().find(|entry| entry.name == slot) {
            entry.held = None;
        }
    }

    /// The project closed: `asset.release` for every slot that holds something, and forget them.
    pub fn release_all(&mut self) -> Vec<Value> {
        let mut releases = Vec::new();
        for slot in &mut self.slots {
            if slot.held.take().is_some() {
                releases.push(release_message(&slot.name));
            }
        }
        releases
    }

    /// The runtime is gone (process restarted, page reloaded): nothing it held is left to release.
    /// The generation is kept, so an older map stays refused.
    pub fn reset(&mut self) {
        for slot in &mut self.slots {
            slot.held = None;
        }
    }
}

fn release_message(slot: &str) -> Value {
    json!({ "v": tessel_editor_core::bridge::BRIDGE_VERSION, "type": "asset.release", "id": slot })
}

fn load_message(slot: &str, identity: &Identity, resource: &PlanResource, path: &Path) -> Value {
    // A splat is loaded as the runtime's `splat` kind (the same descriptor the Spark asset uses);
    // anything else in a plan is a still picture.
    let (kind, meta) = if resource.is_splat() {
        ("splat", json!({ "format": "spz" }))
    } else {
        (
            "still",
            json!({ "width": resource.width_px, "height": resource.height_px }),
        )
    };
    json!({
        "v": tessel_editor_core::bridge::BRIDGE_VERSION, "type": "asset.load",
        "asset": {
            "id": slot, "kind": kind,
            "source": { "type": "file", "path": path.to_string_lossy() },
            "meta": meta,
            // Additive field of the descriptor: the identity the runtime binds this slot to.
            "scene": {
                "asset_id": identity.asset_id, "revision_id": identity.revision_id,
                "content_hash": identity.content_hash, "generation": identity.generation,
            },
        },
    })
}

/// The `directed_scene` layer of a `frame.render` plan: the plan in its wire form, and where each
/// of its resources was loaded.
pub fn directed_layer(scene_plan: Value, references: Vec<Value>) -> Value {
    json!({
        "id": "directed-scene", "clip": null, "asset": null, "kind": DIRECTED_LAYER_KIND,
        "transform": { "x": 0.0, "y": 0.0, "scale": 1.0, "rotation": 0.0, "crop": [0.0, 0.0, 0.0, 0.0] },
        "opacity": 1.0, "blend": "normal",
        "scene_plan": scene_plan, "resources": references,
    })
}

/// What one frame of the monitor or the headless renderer asks for.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneFrameMessage {
    pub id: u64,
    /// Monotonic seek counter; the host shows only the newest revision.
    pub revision: u64,
    /// The edit frame the plan is of (informational for the runtime).
    pub edit_frame: u64,
    pub width: u32,
    pub height: u32,
    pub layer: Value,
    /// Write a PNG here (the Node worker does; the WebView host ignores it).
    pub png: Option<String>,
    pub probes: Vec<(u32, u32)>,
    /// Read the picture back and describe it. `false` is the monitor's interactive path.
    pub inspect: bool,
}

/// The `frame.render` message of a directed scene: the same message the core builds for an
/// ordinary plan, with one `directed_scene` layer. The runtime draws the layer into the canvas
/// like any other, so a monitor shows what a headless frame writes.
pub fn frame_render_message(frame: &SceneFrameMessage) -> Value {
    let mut message = json!({
        "v": tessel_editor_core::bridge::BRIDGE_VERSION, "type": "frame.render",
        "id": frame.id, "revision": frame.revision,
        "plan": {
            "version": 1, "intent": { "mode": "sequence" },
            "canvas": { "width": frame.width, "height": frame.height, "background": [0.0, 0.0, 0.0, 1.0] },
            "frame": frame.edit_frame, "layers": [frame.layer],
            "requires": [], "warnings": [],
        },
        "output": { "width": frame.width, "height": frame.height, "png": frame.png },
        "probes": frame.probes,
    });
    if !frame.inspect {
        message["inspect"] = json!(false);
    }
    message
}
