//! Directed scenes (BB01/BB02): import, validate and round-trip `render-bundle.json` v1.0 and
//! inspect its minimal graph of layers, hierarchy, references and 3D transforms.
//!
//! The editor's canonical document stays `.tessel.json` (`document.rs`); a bundle is an
//! *interchange* format that this module reads and writes without touching that document, the
//! playback state or the runtime. Adapters (command, headless, desktop) are not wired yet.
//! Contract, policies and divergences from the origin: `docs/features/directed-scene.md`.
//!
//! Time and camera (BB03) live in `temporal`: frame mapping, camera pose and speed, and the
//! temporal gate (`DirectedScene::temporal_join_issues` / `temporal_issues`). Contract:
//! `docs/features/scene-temporal.md`. Node animation (BB03, nodes) lives in `animation`: the
//! performance channels, the transform of every node and the world matrices at a frame of an
//! instance, joined to the camera by `Bundle::evaluate_frame`. Contract:
//! `docs/features/scene-animation.md`. Lighting, weather, effects and rigs are not evaluated and a
//! frame says so.
//!
//! ```
//! use tessel_editor_core::directed_scene::DirectedScene;
//!
//! # fn demo(text: &str) -> Result<(), Box<dyn std::error::Error>> {
//! let scene = DirectedScene::import_str(text)?;
//! let bundle = scene.bundle();
//! for shot in &bundle.shots {
//!     println!("{} plays frames {}..{}", shot.id, shot.start_frame, shot.end_frame_exclusive);
//! }
//! let exported = scene.to_json_string(); // same data, unknown fields included
//! # let _ = exported; Ok(()) }
//! ```

mod animation;
mod graph;
mod integrity;
mod issue;
mod joins;
mod model;
mod parse;
mod read;
mod refs;
mod temporal;
mod transform;

use serde_json::Value;

pub use animation::{NodeFrame, NodeOverride, NodeState, NodeTimeline, PerformancePlan};
pub use graph::{Parent, SceneGraph};
pub use integrity::KNOWN_EASINGS;
pub use issue::{
    FPS_OUT_OF_RANGE, INVALID_TYPE, INVALID_VALUE, ImportError, Issue, LIMIT_EXCEEDED,
    MISSING_FIELD, NON_FINITE_TRANSFORM, Notice, UNKNOWN_FIELD, UNSAFE_RELATIVE_PATH,
    UNSUPPORTED_SCHEMA_VERSION,
};
pub use joins::{
    JOIN_FOV_TOLERANCE_DEG, JOIN_FOV_VELOCITY_TOLERANCE_DEG_PER_FRAME,
    JOIN_POSITION_TOLERANCE_SCENE_UNITS, JOIN_VELOCITY_TOLERANCE_SCENE_UNITS_PER_FRAME,
};
pub use model::{
    Asset, AssetAlpha, AssetColorSpace, AssetFormat, AssetRef, AudioMix, Bundle, CameraClip,
    CameraKey, CameraMotion, CameraSegment, ChannelKind, ChannelMotion, ChannelProperty,
    CompositionPass, Fps, MoveRecipe, Node, NodeKind, PerformanceChannel, PerformanceTrack,
    RenderProfile, SceneInstance, SegmentStart, Shot, Socket, SocketAttachment, Transition,
    ZoomRecipe,
};
pub use refs::{Entity, EntityKind, Reference};
pub use temporal::{
    CameraEvaluation, CameraPose, CameraState, CameraTimeline, FrameEvaluation, FrameMapping,
    LAYER_FOV_SWEEP_MAX_FRAMES, NodeAnimation, SegmentPoses, TemporalError, Unsupported,
    apply_easing, movement_end_pose, movement_pose_at,
};
pub use transform::{Mat4, Quat, Transform, Vec3};

/// The only `schemaVersion` this importer reads, for the bundle and for each scene definition.
pub const SCHEMA_VERSION: &str = "1.0";

/// A clip stacks at most this many movement tracks on its main one (`MAX_CAMERA_LAYERS`).
pub const MAX_CAMERA_LAYERS: usize = 3;

/// Fields of the origin schema that are kept in the document but not interpreted yet, by JSON
/// path pattern (`[]` is any array index). Round-trip preserves them; nothing validates them.
pub const DEFERRED_FIELDS: &[&str] = &[
    "narration",
    "outline",
    "metadata (only `pilotFrameAudit` is checked)",
    "sceneDefinitions[].contactPlaneId",
    "sceneDefinitions[].frameAudit",
    "sceneDefinitions[].weather",
    "sceneDefinitions[].weatherKeys",
    "sceneDefinitions[].lighting",
    "sceneDefinitions[].effects",
    "sceneDefinitions[].weatherGround",
    "sceneDefinitions[].rigs",
    "sceneDefinitions[].paths",
    "sceneDefinitions[].nodes[].surface (only its assetRef.assetId is read)",
    "sceneDefinitions[].nodes[].normalMap (only its assetRef.assetId is read)",
    "sceneDefinitions[].nodes[].sceneDepth",
    "sceneDefinitions[].nodes[].relief",
    "sceneDefinitions[].nodes[].rigBind",
    "sceneDefinitions[].nodes[].reveal",
];

/// Rules of the origin that this importer does not run yet, by origin code or code family.
pub const DEFERRED_RULES: &[&str] = &[
    "SURFACE_*, NORMAL_MAP_*, RELIEF_*, SCENE_DEPTH_* (data-map sections of nodes)",
    "LIGHT_* except LIGHT_PARENT_UNKNOWN, DUPLICATE_LIGHT_ID, SHADOW_CASTER_* and keyframe rules of lights/weather (M2/M3)",
    "AUDIT_* (frame audit)",
    "weather name, rig, path and effects rules (M3/M6)",
    "narration and outline rules (M5)",
    "the weatherGround node must be an image plane",
    "two channels writing one node property: the origin raises it when it evaluates the instance (`performance_plan` and `evaluate_frame` do), not when it validates",
    // These two need the camera evaluated: import does not run them, `temporal_issues` does.
    "CAMERA_LAYER composed field-of-view check, frame by frame (run by `temporal_issues`, not by import)",
    "CONTINUOUS_*_JUMP pose comparison at joins (run by `temporal_join_issues`, not by import)",
];

/// Bounds on what an import accepts, so untrusted input cannot exhaust memory or stack.
///
/// The origin has none (it trusts a local file); these are this importer's addition and are
/// enforced before and during reading, never by truncating.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    /// Size of the JSON text (`ImportError::TooLarge`).
    pub max_bytes: usize,
    /// Nesting depth of the JSON tree, a document counting as 1 (`ImportError::Syntax`).
    pub max_depth: usize,
    /// Items in any one array (`LIMIT_EXCEEDED` issue): nodes, shots, keys, assets...
    pub max_items: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: 32 * 1024 * 1024,
            // serde_json refuses deeper text on its own; the same bound applies to a `Value`.
            max_depth: 128,
            max_items: 100_000,
        }
    }
}

/// An imported bundle validated against the BB01/BB02 subset, not the full origin schema.
///
/// The document (a JSON tree) is the canonical data and is never altered by the typed view, so
/// `export` reproduces everything that was imported, unknown fields included. The view
/// (`bundle`) is what the importer understood of it.
#[derive(Debug, Clone, PartialEq)]
pub struct DirectedScene {
    document: Value,
    bundle: Bundle,
    notices: Vec<Notice>,
}

fn exceeds_depth(root: &Value, max: usize) -> bool {
    let mut stack = vec![(root, 1_usize)];
    while let Some((value, depth)) = stack.pop() {
        if depth > max {
            return true;
        }
        match value {
            Value::Array(items) => stack.extend(items.iter().map(|item| (item, depth + 1))),
            Value::Object(map) => stack.extend(map.values().map(|item| (item, depth + 1))),
            _ => {}
        }
    }
    false
}

impl DirectedScene {
    /// Parses and validates bundle text with the default `Limits`.
    pub fn import_str(text: &str) -> Result<Self, ImportError> {
        Self::import_str_with(text, &Limits::default())
    }

    pub fn import_str_with(text: &str, limits: &Limits) -> Result<Self, ImportError> {
        if text.len() > limits.max_bytes {
            return Err(ImportError::TooLarge {
                bytes: text.len(),
                max: limits.max_bytes,
            });
        }
        let value: Value = serde_json::from_str(text).map_err(|error| ImportError::Syntax {
            message: error.to_string(),
            line: error.line(),
            column: error.column(),
        })?;
        Self::import_value_with(value, limits)
    }

    /// Validates a JSON tree the caller already has (an `ops` payload, a package entry).
    pub fn import_value(value: Value) -> Result<Self, ImportError> {
        Self::import_value_with(value, &Limits::default())
    }

    pub fn import_value_with(value: Value, limits: &Limits) -> Result<Self, ImportError> {
        if exceeds_depth(&value, limits.max_depth) {
            return Err(ImportError::Syntax {
                message: format!("nesting is deeper than {} levels", limits.max_depth),
                line: 0,
                column: 0,
            });
        }
        let sink = read::Sink::new(limits.max_items);
        let draft = parse::read_bundle(&value, &sink);
        let (mut issues, notices) = sink.into_parts();
        let Some(draft) = draft.filter(|_| issues.is_empty()) else {
            if issues.is_empty() {
                issues.push(Issue::new(
                    INVALID_VALUE,
                    "",
                    "the document is not a valid bundle",
                ));
            }
            return Err(ImportError::Invalid { issues });
        };

        let integrity = integrity::check(&draft, &value);
        if !integrity.is_empty() {
            return Err(ImportError::Invalid { issues: integrity });
        }

        let mut issues = Vec::new();
        let scenes: Vec<SceneGraph> = draft
            .defs
            .into_iter()
            .map(|def| SceneGraph::build(def, &mut issues))
            .collect();
        if !issues.is_empty() {
            return Err(ImportError::Invalid { issues });
        }
        let bundle = Bundle {
            bundle_id: draft.bundle_id,
            revision_id: draft.revision_id,
            fps: draft.fps,
            edit_duration_frames: draft.edit_duration_frames,
            render_profile: draft.render_profile,
            assets: draft.assets,
            scenes,
            instances: draft.instances,
            performance_tracks: draft.tracks,
            camera_clips: draft.clips,
            shots: draft.shots,
            composition_passes: draft.passes,
            audio: draft.audio,
        };
        Ok(Self {
            document: value,
            bundle,
            notices,
        })
    }

    /// Checks the supported import subset without keeping the scene.
    /// Deferred rules and temporal joins still require their own validation.
    pub fn validate_str(text: &str) -> Result<(), ImportError> {
        Self::import_str(text).map(drop)
    }

    /// What the importer understood of the document: layers, hierarchy, shots, cameras.
    pub fn bundle(&self) -> &Bundle {
        &self.bundle
    }

    /// Fields found that `render-bundle` 1.0 does not declare. They are kept on export.
    pub fn notices(&self) -> &[Notice] {
        &self.notices
    }

    /// The canonical document as imported, every field included.
    pub fn document(&self) -> &Value {
        &self.document
    }

    /// Consumes the scene and returns its document.
    pub fn into_document(self) -> Value {
        self.document
    }

    /// Bundle JSON, compact. Equal as data to what was imported: same keys (including unknown
    /// ones), same values, same numbers. Formatting and key order are not preserved: object
    /// keys come out in `serde_json`'s map order.
    pub fn to_json_string(&self) -> String {
        self.document.to_string()
    }

    /// Like `to_json_string`, indented, ending in a newline (what `scene` writes to disk).
    pub fn to_json_pretty(&self) -> String {
        // A `Value` always serializes; the fallback only keeps the signature total.
        let mut text = serde_json::to_string_pretty(&self.document).unwrap_or_default();
        text.push('\n');
        text
    }

    /// The structural rules of the cuts between shots (the origin's `validateEditJoins`, minus
    /// the camera-pose comparison, which is `temporal_join_issues`). Empty means no *structural*
    /// finding: it does not certify that the camera is smooth at continuous cuts.
    pub fn edit_join_issues(&self) -> Vec<Issue> {
        joins::check(&self.bundle)
    }

    /// The camera-pose rules of continuous cuts (`CONTINUOUS_CAMERA_JUMP`, `_TARGET_JUMP`,
    /// `_FOV_JUMP`, `_VELOCITY_JUMP`, `_FOV_VELOCITY_JUMP`), each with the origin's tolerances
    /// (`JOIN_*`). Additive to `edit_join_issues`: the origin's `validateEditJoins` is the two
    /// together. `CAMERA_EVALUATION_FAILED` appears where a join's camera cannot be evaluated.
    pub fn temporal_join_issues(&self) -> Vec<Issue> {
        joins::check_temporal(&self.bundle)
    }

    /// Everything that needs the camera evaluated: the stacked field of view of layered clips
    /// (`FOV_OUT_OF_RANGE`, which the origin checks at import) and `temporal_join_issues`.
    /// Empty, together with `edit_join_issues` being empty, is what the origin's `finalizeBundle`
    /// accepts for the part of the bundle this module interprets.
    pub fn temporal_issues(&self) -> Vec<Issue> {
        let mut issues = temporal::layered_fov_issues(&self.bundle);
        issues.extend(self.temporal_join_issues());
        issues
    }
}
