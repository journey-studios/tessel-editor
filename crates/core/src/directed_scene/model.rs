//! Typed, read-only view of an imported bundle.
//!
//! WHY a view and not the document: the canonical data of an imported bundle is the JSON tree
//! kept in `DirectedScene` (lossless, so unknown and not-yet-interpreted fields round-trip). These
//! types are what the importer understood of it. They hold only supported vocabulary: values the
//! origin accepts but this importer rejects (`supportPlane`, premultiplied alpha, `rec709-video`,
//! dissolve) never reach a successfully imported scene, so they have no variant here.
//! Only the pieces BB01/BB02 need exist: layered planes, hierarchy and references, 3D
//! transforms, and the structure of shots and camera clips. Lighting, weather, effects, rig,
//! paths, narration and outline are kept raw and listed in `DEFERRED_FIELDS`.

use crate::time::{Rate, Ratio};

use super::graph::SceneGraph;
use super::transform::{Transform, Vec3};

/// Frame rate as declared: the integers of the bundle, before reduction.
///
/// WHY not only a `Rate`: the origin's `FPS_MISMATCH` rule compares `bundle.fps` and
/// `renderProfile.fps` literally, so `48/2` and `24/1` are different declarations there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fps {
    pub numerator: u32,
    pub denominator: u32,
}

impl Fps {
    /// Always `Some` for a scene that was imported (the importer rejects rates `Rate` refuses).
    pub fn rate(self) -> Option<Rate> {
        Rate::new(self.numerator, self.denominator)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetFormat {
    /// An image with alpha. This is also what an absent `format` means.
    Png,
    /// Gaussian splats.
    Spz,
    /// A grid of 32-bit floats (scene depth, relief).
    F32,
}

impl AssetFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Spz => "spz",
            Self::F32 => "f32",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetAlpha {
    Straight,
    Opaque,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetColorSpace {
    Srgb,
    /// Only accepted for assets a node reads as data (surface/normal maps, f32 grids).
    LinearSrgb,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    pub asset_id: String,
    pub revision_id: String,
    pub content_hash: String,
    /// Relative to the bundle folder; the importer guarantees it does not escape it.
    pub relative_path: String,
    pub format: AssetFormat,
    pub width_px: u64,
    /// The origin types this as any positive number (`z.number().positive()`), not an integer.
    pub height_px: f64,
    pub alpha: AssetAlpha,
    pub color_space: AssetColorSpace,
}

/// The four fields that pin an asset revision; a node's copy must equal its `assets[]` record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetRef {
    pub asset_id: String,
    pub revision_id: String,
    pub content_hash: String,
    pub relative_path: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderProfile {
    pub width: u64,
    pub height: u64,
    pub fps: Fps,
    pub three_revision: String,
    pub browser_channel: String,
}

/// Node kinds this importer supports. The origin schema also names `supportPlane` and `overlay`;
/// the origin's own graph validation rejects them (`UNSUPPORTED_NODE_TYPE`), so does this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Group,
    ImagePlane,
    Splat,
}

impl NodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Group => "group",
            Self::ImagePlane => "imagePlane",
            Self::Splat => "splat",
        }
    }
}

/// `attachToSocket`: the node hangs from a named socket of another node instead of a `parentId`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketAttachment {
    pub owner: String,
    pub socket: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub id: String,
    /// The declared `parentId`; the effective parent is `SceneGraph::parent`.
    pub parent_id: Option<String>,
    pub kind: NodeKind,
    pub asset: Option<AssetRef>,
    pub transform: Transform,
    pub width_scene_units: Option<f64>,
    pub height_scene_units: Option<f64>,
    pub render_order: Option<i64>,
    pub socket_id: Option<String>,
    pub attach_to_socket: Option<SocketAttachment>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Socket {
    pub owner: String,
    pub name: String,
    /// Rotation and translation relative to the owner; sockets have no scale.
    pub position: Vec3,
    pub rotation: super::transform::Quat,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneInstance {
    pub id: String,
    pub scene_definition_id: String,
    pub duration_frames: u64,
    pub performance_track_ids: Vec<String>,
    pub edit_locked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelKind {
    Cycle,
    Event,
    Static,
}

/// The transform property of a node a performance channel writes. Rotation is in XYZ Euler
/// degrees, as in the origin (`NodeTransformOverride.property`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelProperty {
    LocalRotation,
    LocalPosition,
    LocalScale,
}

impl ChannelProperty {
    /// The bundle's own spelling (`localRotation`, ...), also what the origin's errors print.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalRotation => "localRotation",
            Self::LocalPosition => "localPosition",
            Self::LocalScale => "localScale",
        }
    }
}

/// What a performance channel does to its node property, as declared. Evaluating it over the
/// instance's frames is `animation` (BB03, nodes).
#[derive(Debug, Clone, PartialEq)]
pub enum ChannelMotion {
    /// `sin` swing around the base: `axis * sin(phase / period * 2π) * amplitude`, where
    /// `phase = (instanceFrame + phaseOffsetFrames) % periodFrames`. Never `localScale`.
    Cycle {
        property: ChannelProperty,
        period_frames: u64,
        amplitude: f64,
        axis: Vec3,
        phase_offset_frames: u64,
    },
    /// Linear move by `delta` over `durationFrames` from `startInstanceFrame`, then held at
    /// `delta` (the schema requires `holdFinal: true`, so there is no variant without the hold).
    /// Never `localScale`.
    Event {
        property: ChannelProperty,
        start_instance_frame: u64,
        duration_frames: u64,
        delta: Vec3,
    },
    /// A constant `value` at every frame.
    Static {
        property: ChannelProperty,
        value: Vec3,
    },
}

impl ChannelMotion {
    pub fn property(&self) -> ChannelProperty {
        match self {
            Self::Cycle { property, .. }
            | Self::Event { property, .. }
            | Self::Static { property, .. } => *property,
        }
    }
}

/// A performance channel: its identity, its target and what it does there (`kind` repeats the
/// variant of `motion`, kept so identity-only consumers need not match on the payload).
#[derive(Debug, Clone, PartialEq)]
pub struct PerformanceChannel {
    pub kind: ChannelKind,
    pub channel_id: String,
    pub node_id: String,
    pub motion: ChannelMotion,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PerformanceTrack {
    pub id: String,
    pub instance_id: String,
    pub channels: Vec<PerformanceChannel>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentStart {
    Initial,
    PreviousEnd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveRecipe {
    DollyIn,
    DollyOut,
    TruckLeft,
    TruckRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoomRecipe {
    In,
    Out,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CameraKey {
    pub frame: i64,
    pub position: Vec3,
    pub target: Vec3,
    pub fov_deg: f64,
    pub easing: String,
}

/// What a camera movement does. The numbers are as declared; evaluating them over time is BB03.
#[derive(Debug, Clone, PartialEq)]
pub enum CameraMotion {
    Hold,
    Move {
        recipe: MoveRecipe,
        distance_scene_units: f64,
        easing: String,
    },
    Zoom {
        recipe: ZoomRecipe,
        fov_delta_deg: f64,
        easing: String,
    },
    Orbit {
        yaw_deg: f64,
        /// `0` when the bundle omits it (the origin schema defaults it to 0).
        pitch_deg: f64,
        easing: String,
    },
    Keyframes {
        keys: Vec<CameraKey>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct CameraSegment {
    pub duration_frames: u64,
    pub from: SegmentStart,
    pub motion: CameraMotion,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CameraClip {
    pub id: String,
    pub initial_position: Vec3,
    pub initial_target: Vec3,
    pub initial_fov_deg: f64,
    pub segments: Vec<CameraSegment>,
    /// Extra tracks that add to the main one (at most 3).
    pub layers: Vec<Vec<CameraSegment>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transition {
    Continuous,
    Cut { reason: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Shot {
    pub id: String,
    pub scene_instance_id: String,
    pub start_frame: u64,
    pub end_frame_exclusive: u64,
    pub instance_start_frame: u64,
    pub camera_clip_id: String,
    pub camera_in_frame: u64,
    pub transition_into: Option<Transition>,
    pub intent: String,
}

impl Shot {
    pub fn duration_frames(&self) -> u64 {
        self.end_frame_exclusive.saturating_sub(self.start_frame)
    }

    /// Whether `frame` is one of the edit frames of this shot: `[start_frame, end_frame_exclusive)`.
    /// The one predicate of "which shot plays at this frame".
    pub fn holds_frame(&self, frame: u64) -> bool {
        self.start_frame <= frame && frame < self.end_frame_exclusive
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompositionPass {
    pub fragment_id: String,
    pub node_id: String,
    pub order: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioMix {
    pub mix_relative_path: String,
    pub edit_start_seconds: f64,
    pub edit_end_seconds_exclusive: f64,
    pub source_pcm_hash_sha256: Option<String>,
}

/// What the importer understood of a bundle.
#[derive(Debug, Clone, PartialEq)]
pub struct Bundle {
    pub bundle_id: String,
    pub revision_id: String,
    pub fps: Fps,
    pub edit_duration_frames: u64,
    pub render_profile: RenderProfile,
    pub assets: Vec<Asset>,
    /// `sceneDefinitions`, in document order.
    pub scenes: Vec<SceneGraph>,
    pub instances: Vec<SceneInstance>,
    pub performance_tracks: Vec<PerformanceTrack>,
    pub camera_clips: Vec<CameraClip>,
    pub shots: Vec<Shot>,
    pub composition_passes: Vec<CompositionPass>,
    pub audio: Option<AudioMix>,
    /// The effects layer (`bundle.fx`, F016): looks over the edit's timeline, on the output or
    /// (F062) on one shot's picture.
    pub fx: super::fx::Fx,
}

impl Bundle {
    pub fn asset(&self, asset_id: &str) -> Option<&Asset> {
        self.assets.iter().find(|asset| asset.asset_id == asset_id)
    }

    pub fn scene(&self, scene_id: &str) -> Option<&SceneGraph> {
        self.scenes.iter().find(|scene| scene.id() == scene_id)
    }

    pub fn instance(&self, instance_id: &str) -> Option<&SceneInstance> {
        self.instances
            .iter()
            .find(|instance| instance.id == instance_id)
    }

    pub fn camera_clip(&self, clip_id: &str) -> Option<&CameraClip> {
        self.camera_clips.iter().find(|clip| clip.id == clip_id)
    }

    pub fn shot(&self, shot_id: &str) -> Option<&Shot> {
        self.shots.iter().find(|shot| shot.id == shot_id)
    }

    /// The scene definition an instance plays.
    pub fn scene_of_instance(&self, instance_id: &str) -> Option<&SceneGraph> {
        self.scene(&self.instance(instance_id)?.scene_definition_id)
    }

    /// Length of the whole edit, exact at any frame rate (`24000/1001` included).
    pub fn edit_duration(&self) -> Option<Ratio> {
        let frames = i64::try_from(self.edit_duration_frames).ok()?;
        Some(self.fps.rate()?.frame_to_time(frames))
    }

    /// Start time of a frame of the edit.
    pub fn edit_time_of_frame(&self, frame: u64) -> Option<Ratio> {
        Some(self.fps.rate()?.frame_to_time(i64::try_from(frame).ok()?))
    }

    /// Shot that plays at an edit frame (shots tile the edit without gaps).
    pub fn shot_at_frame(&self, frame: u64) -> Option<&Shot> {
        self.shots.iter().find(|shot| shot.holds_frame(frame))
    }
}
