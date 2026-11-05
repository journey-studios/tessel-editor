//! Scene resources (BB06, initial slice; F038): the manifest of every file a directed-scene
//! bundle depends on, with transitive dependencies and a query by `assetId`.
//!
//! The manifest is pure data derived from a `DirectedScene` (`directed_scene`): no IO, no
//! absolute path, no GPUI, and no dependency on the evaluated scene plan (H). Resources keep the
//! bundle's identity (`assetId`) and the origin's `relativePath`; the manifest never reads,
//! hashes or copies a file. Resolving and materializing them is `tessel_editor_media::directed_resources`.
//! Contract and limits: `docs/features/scene-resources.md`.
//!
//! What it classifies:
//!
//! - **Supported** (`ResourceStatus::Supported`): files the interpreted vocabulary needs: the
//!   asset of an `imagePlane` (PNG) or `splat` (SPZ) node and the audio mix.
//! - **Uninterpreted**: files named only by sections the importer keeps without interpreting
//!   (`surface`, `normalMap`, `sceneDepth`, `relief`, narration takes). They are listed in
//!   `ResourceManifest::uninterpreted`, pulled in by `closure`, and never carried by transport.
//! - **Unreferenced**: declared in `assets[]`, named by nothing.
//!
//! ```
//! use tessel_editor_core::directed_scene::DirectedScene;
//! use tessel_editor_core::scene_resources::ResourceManifest;
//!
//! # fn demo(text: &str) -> Result<(), Box<dyn std::error::Error>> {
//! let scene = DirectedScene::import_str(text)?;
//! let manifest = ResourceManifest::from_scene(&scene);
//! if let Some(hero) = manifest.resource("hero") {
//!     println!("{} lives at {}", hero.key.label(), hero.relative_path);
//! }
//! let plan = manifest.transport_plan(); // Err when a blocking issue stands in the way
//! # let _ = plan; Ok(()) }
//! ```

mod manifest;
mod model;
mod path;
mod query;
mod scan;
mod transport;

pub use manifest::ResourceManifest;
pub use model::{
    Capabilities, CollisionKind, LISTING_TRUNCATED, MAX_LISTED_FILES, ManifestIssue, NamedFile,
    PathCollision, REFERENCE_IGNORED, REFERENCE_MISMATCH, REFERENCE_UNRESOLVED,
    RESOURCE_FORMAT_MISMATCH, RESOURCE_HASH_INVALID, RESOURCE_PATH_COLLISION,
    RESOURCE_PATH_INVALID, ResourceDependency, ResourceEntry, ResourceKey, ResourceKind,
    ResourceStatus, ResourceUse, Severity, SupportedResource, UninterpretedSection, UseRole,
    is_sha256_hex,
};
pub use path::{
    MAX_PATH_BYTES, MAX_SEGMENT_BYTES, MAX_SEGMENTS, PathError, RESERVED_PREFIX, ResourcePath,
};
pub use query::ResourceLookup;
pub use transport::{SkippedResource, TransportItem, TransportPlan, TransportResource};

/// Version of the manifest contract (types, classification, JSON). Bumped when a consumer would
/// read the same bundle differently; H and the packager can refuse a version they do not know.
pub const MANIFEST_VERSION: u32 = 1;

/// What this version reads and what it declares it does not.
pub const CAPABILITIES: Capabilities = Capabilities {
    manifest_version: MANIFEST_VERSION,
    supported: &[
        SupportedResource {
            kind: ResourceKind::Image,
            via: "nodes[].assetRef of an imagePlane node (png)",
        },
        SupportedResource {
            kind: ResourceKind::Splat,
            via: "nodes[].assetRef of a splat node (spz)",
        },
        SupportedResource {
            kind: ResourceKind::Audio,
            via: "audio.mixRelativePath",
        },
    ],
    uninterpreted: &[
        "sceneDefinitions[].nodes[].surface (png map)",
        "sceneDefinitions[].nodes[].normalMap (png map)",
        "sceneDefinitions[].nodes[].sceneDepth (f32 grid + png image)",
        "sceneDefinitions[].nodes[].relief (f32 grid)",
        "narration (takes: tracks[].clips[].audio.relativePath)",
    ],
    unscanned: &[
        "unknown fields (DirectedScene::notices)",
        "lighting, weather, effects, rigs, paths and frameAudit (the origin schema does not let them name files)",
    ],
};
