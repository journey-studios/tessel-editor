//! Types of the resource manifest. Plain data: no IO, no absolute path, no GPUI.

use crate::directed_scene::AssetFormat;

use super::path::{PathError, ResourcePath};

/// The identity of a resource inside a bundle: its `assetId`, which is not its path or hash.
/// The audio mix has no `assetId` (the bundle names only its file), so it has its own key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResourceKey {
    Asset(String),
    AudioMix,
}

impl ResourceKey {
    pub fn asset(asset_id: &str) -> Self {
        Self::Asset(asset_id.to_string())
    }

    pub fn asset_id(&self) -> Option<&str> {
        match self {
            Self::Asset(id) => Some(id),
            Self::AudioMix => None,
        }
    }

    /// `asset:<assetId>` or `audio-mix`: how reports and JSON name a resource.
    pub fn label(&self) -> String {
        match self {
            Self::Asset(id) => format!("asset:{id}"),
            Self::AudioMix => "audio-mix".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResourceKind {
    /// A PNG image (an absent `format` means this).
    Image,
    /// Gaussian splats (`spz`).
    Splat,
    /// A grid of 32-bit floats (`f32`): scene depth or relief.
    Grid,
    /// The bundle's mixed audio file.
    Audio,
}

impl ResourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Splat => "splat",
            Self::Grid => "grid",
            Self::Audio => "audio",
        }
    }

    pub(super) fn of_format(format: AssetFormat) -> Self {
        match format {
            AssetFormat::Png => Self::Image,
            AssetFormat::Spz => Self::Splat,
            AssetFormat::F32 => Self::Grid,
        }
    }
}

/// What a resource is for, in one place that points at it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UseRole {
    /// `assetRef` of an `imagePlane` node. Supported.
    NodeImage,
    /// `assetRef` of a `splat` node. Supported.
    NodeSplat,
    /// `audio.mixRelativePath`. Supported.
    AudioMix,
    /// `nodes[].surface.assetRef`: preserved, not interpreted.
    SurfaceMap,
    /// `nodes[].normalMap.assetRef`: preserved, not interpreted.
    NormalMap,
    /// `nodes[].sceneDepth.depthAssetRef`: preserved, not interpreted.
    SceneDepthGrid,
    /// `nodes[].sceneDepth.imageAssetRef`: preserved, not interpreted.
    SceneDepthImage,
    /// `nodes[].relief.depthAssetRef`: preserved, not interpreted.
    ReliefGrid,
}

impl UseRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NodeImage => "nodeImage",
            Self::NodeSplat => "nodeSplat",
            Self::AudioMix => "audioMix",
            Self::SurfaceMap => "surfaceMap",
            Self::NormalMap => "normalMap",
            Self::SceneDepthGrid => "sceneDepthGrid",
            Self::SceneDepthImage => "sceneDepthImage",
            Self::ReliefGrid => "reliefGrid",
        }
    }

    /// Whether this scene vocabulary is interpreted today (a frame needs the file).
    pub fn is_supported(self) -> bool {
        matches!(self, Self::NodeImage | Self::NodeSplat | Self::AudioMix)
    }

    /// The `assets[].format` this use expects, when it expects one.
    pub fn expected_format(self) -> Option<AssetFormat> {
        match self {
            Self::NodeImage | Self::SurfaceMap | Self::NormalMap | Self::SceneDepthImage => {
                Some(AssetFormat::Png)
            }
            Self::NodeSplat => Some(AssetFormat::Spz),
            Self::SceneDepthGrid | Self::ReliefGrid => Some(AssetFormat::F32),
            Self::AudioMix => None,
        }
    }

    /// A problem with a supported use blocks transport; with a preserved one it is a warning.
    pub(super) fn severity(self) -> Severity {
        if self.is_supported() {
            return Severity::Error;
        }
        Severity::Warning
    }
}

/// One place of the bundle that points at a resource.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ResourceUse {
    pub role: UseRole,
    /// The scene definition, for node uses.
    pub scene_id: Option<String>,
    pub node_id: Option<String>,
    /// JSON Pointer of the field that names the resource, in the bundle document.
    pub pointer: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResourceStatus {
    /// Used by interpreted vocabulary: a frame needs it and transport carries it.
    Supported,
    /// Named only by sections the importer keeps without interpreting: declared, not transported.
    Uninterpreted,
    /// Declared in `assets[]` and named by nothing.
    Unreferenced,
}

impl ResourceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::Uninterpreted => "uninterpreted",
            Self::Unreferenced => "unreferenced",
        }
    }
}

/// A file the bundle depends on.
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceEntry {
    pub key: ResourceKey,
    pub kind: ResourceKind,
    pub status: ResourceStatus,
    /// The declared `assets[].format`; `None` for the audio mix.
    pub format: Option<AssetFormat>,
    /// `None` for the audio mix.
    pub revision_id: Option<String>,
    /// SHA-256, lowercase hex, as declared (`None` when the audio mix declares none).
    pub content_hash: Option<String>,
    /// The origin path, exactly as declared in the bundle.
    pub relative_path: String,
    /// What a materializer may create; `None` when the declared path is not portable
    /// (`path_error` says why, and an issue is raised).
    pub transport_path: Option<ResourcePath>,
    pub path_error: Option<PathError>,
    pub width_px: Option<u64>,
    /// The origin types it as any positive number.
    pub height_px: Option<f64>,
    /// Every place that names it, sorted.
    pub uses: Vec<ResourceUse>,
}

impl ResourceEntry {
    pub fn asset_id(&self) -> Option<&str> {
        self.key.asset_id()
    }

    pub fn is_supported(&self) -> bool {
        self.status == ResourceStatus::Supported
    }
}

/// `from` needs `to`: the node's image has a map, a splat has a depth grid. Resources of
/// supported nodes pull in the preserved ones they point at, so the closure says what is lost.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ResourceDependency {
    pub from: ResourceKey,
    pub to: ResourceKey,
    pub role: UseRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Transport must not proceed: what it would carry cannot be trusted.
    Error,
    /// Informative: concerns something that is preserved but not carried.
    Warning,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
        }
    }
}

pub const RESOURCE_PATH_INVALID: &str = "RESOURCE_PATH_INVALID";
pub const RESOURCE_PATH_COLLISION: &str = "RESOURCE_PATH_COLLISION";
pub const RESOURCE_HASH_INVALID: &str = "RESOURCE_HASH_INVALID";
pub const RESOURCE_FORMAT_MISMATCH: &str = "RESOURCE_FORMAT_MISMATCH";
pub const REFERENCE_UNRESOLVED: &str = "REFERENCE_UNRESOLVED";
pub const REFERENCE_MISMATCH: &str = "REFERENCE_MISMATCH";
pub const REFERENCE_IGNORED: &str = "REFERENCE_IGNORED";
pub const LISTING_TRUNCATED: &str = "LISTING_TRUNCATED";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ManifestIssue {
    pub code: &'static str,
    pub severity: Severity,
    pub key: Option<ResourceKey>,
    /// JSON Pointer in the bundle document, empty when the issue is not about one field.
    pub pointer: String,
    pub message: String,
}

impl ManifestIssue {
    pub(super) fn new(
        code: &'static str,
        severity: Severity,
        key: Option<ResourceKey>,
        pointer: &str,
        message: String,
    ) -> Self {
        Self {
            code,
            severity,
            key,
            pointer: pointer.to_string(),
            message,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CollisionKind {
    /// Two supported resources land on one path with different content hashes.
    DifferentContent,
    /// Two paths that differ only by letter case (one file on a case-insensitive volume).
    CaseOnly,
    /// A resource path is a folder of another resource.
    FileIsDirectory,
}

impl CollisionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DifferentContent => "differentContent",
            Self::CaseOnly => "caseOnly",
            Self::FileIsDirectory => "fileIsDirectory",
        }
    }
}

/// Resources that cannot share one transport root. Assets that declare the same path *and* the
/// same hash are one file, not a collision (`TransportItem` lists them together).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PathCollision {
    pub kind: CollisionKind,
    /// The case-folded path (`ResourcePath::collision_key`) where they meet.
    pub path: String,
    pub keys: Vec<ResourceKey>,
}

/// A file named by a preserved section without an `assetId` (a narration take).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedFile {
    pub pointer: String,
    /// As declared; not validated, because the section is not interpreted.
    pub relative_path: String,
}

/// A section of the bundle that holds data the importer keeps and nothing interprets. Declared
/// here so what it names is never silently lost: it is listed, never transported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninterpretedSection {
    /// JSON Pointer of the section.
    pub pointer: String,
    /// `surface`, `normalMap`, `sceneDepth`, `relief` or `narration`.
    pub name: &'static str,
    pub scene_id: Option<String>,
    pub node_id: Option<String>,
    /// The `assetId`s it names, resolved or not.
    pub asset_ids: Vec<String>,
    /// The files it names directly.
    pub files: Vec<NamedFile>,
    /// More files than `MAX_LISTED_FILES`: the list is incomplete and says so.
    pub truncated: bool,
}

/// What the manifest builder reads and what it declares it does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SupportedResource {
    pub kind: ResourceKind,
    pub via: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub manifest_version: u32,
    /// Resources the manifest classifies as `Supported`.
    pub supported: &'static [SupportedResource],
    /// Sections preserved but not interpreted: their files are listed, never transported.
    pub uninterpreted: &'static [&'static str],
    /// Not scanned at all: unknown fields (`DirectedScene::notices`) and the other sections of
    /// `DEFERRED_FIELDS` (lighting, weather, effects, rigs, paths), which the origin schema
    /// does not let name files.
    pub unscanned: &'static [&'static str],
}

/// Longest list of files read from one preserved section.
pub const MAX_LISTED_FILES: usize = 100_000;

/// A SHA-256 as the bundle and the origin spell it: 64 lowercase hex digits. The origin compares
/// the digest it computes with `contentHash` as plain text, so uppercase never matches there.
pub fn is_sha256_hex(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}
