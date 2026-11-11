//! Types of the scene plan. Reused from `directed_scene` where the plan carries the evaluator's
//! own value unchanged (`Vec3`, `Mat4`, `CameraState`, `NodeAnimation`, `Fps`, the asset enums),
//! so the plan cannot drift from what `evaluate_frame` returned.

use crate::directed_scene::fx::FxPass;
use crate::directed_scene::lighting::{LightingReason, Rgb};
use crate::directed_scene::{
    AssetAlpha, AssetColorSpace, AssetFormat, CameraState, Fps, Mat4, NodeAnimation, NodeKind, Vec3,
};
use crate::time::Ratio;

use super::{PLAN_VERSION, PlanError};

/// What a runtime must be able to do to execute a plan. A plan lists only the capabilities its own
/// content needs (`ScenePlan::requires`), so a runtime without `OpaqueAlpha` still runs a plan
/// that has no opaque layer.
///
/// Declaration order is the order they are listed in (the derive of `Ord`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    /// Camera pose and field of view of the frame. Every plan needs it.
    Camera,
    /// Local and world matrices of the nodes. Every plan needs it.
    NodeTransforms,
    /// Nodes that hang from a socket of another node (already resolved in the world matrices).
    Sockets,
    /// The nodes were moved by performance channels (already applied in the matrices).
    PerformanceChannels,
    /// Textured planes drawn back to front.
    ImagePlanes,
    /// A layer whose texture has straight (non-premultiplied) alpha: blend it over what is behind.
    StraightAlpha,
    /// A layer whose texture is opaque: no blending needed.
    OpaqueAlpha,
    /// The draw order of at least one layer comes from a composition pass, not from `renderOrder`.
    CompositionPassOrder,
    /// Gaussian splats (`ScenePlan::splats`): drawn by the runtime's Spark pass, each at its
    /// `after_layers` slot among the image planes, with the node's world matrix and the camera of
    /// the plan. A runtime that does
    /// not list it refuses the plan (`unsupported` + `missing`), never draws it without the splats.
    Splats,
    /// Output-scope effects (`ScenePlan::fx`, F016): passes over the finished picture, drawn in
    /// the listed order after the layers and splats. A runtime that does not list it refuses the
    /// plan (`unsupported` + `missing`), never draws the picture without its effects.
    OutputFx,
    /// Noise effects (`FxPass::seed`, F059): passes that carry a seed and draw grain from the
    /// integer hash of `(seed, cell)`. A runtime that does not list it refuses the plan with
    /// `unsupported` + `missing`, never draws the picture without its noise.
    FxNoise,
    /// Point lights (`ScenePlan::lights`, F060): lit image planes, with the falloff the runtime
    /// documents. A runtime that does not list it refuses the plan with `unsupported` +
    /// `missing`, never draws the planes unlit as if the light were not there. Splats are never
    /// lit, with or without this capability.
    PointLights,
    /// Effects of one shot (`FxPass::scope`, F062): passes drawn on the shot's own picture before
    /// the output passes, listed first in `ScenePlan::fx`. Only a plan with a `shot` pass requires
    /// it (a plan without one keeps its bytes, `requires` included). A runtime that does not list
    /// it refuses the plan with `unsupported` + `missing`, never draws the shot's look over the
    /// whole output nor leaves it out.
    FxShotScope,
}

impl Capability {
    pub const ALL: [Capability; 13] = [
        Self::Camera,
        Self::NodeTransforms,
        Self::Sockets,
        Self::PerformanceChannels,
        Self::ImagePlanes,
        Self::StraightAlpha,
        Self::OpaqueAlpha,
        Self::CompositionPassOrder,
        Self::Splats,
        Self::OutputFx,
        Self::FxNoise,
        Self::PointLights,
        Self::FxShotScope,
    ];

    /// The spelling used on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Camera => "camera",
            Self::NodeTransforms => "nodeTransforms",
            Self::Sockets => "sockets",
            Self::PerformanceChannels => "performanceChannels",
            Self::ImagePlanes => "imagePlanes",
            Self::StraightAlpha => "straightAlpha",
            Self::OpaqueAlpha => "opaqueAlpha",
            Self::CompositionPassOrder => "compositionPassOrder",
            Self::Splats => "splats",
            Self::OutputFx => "outputFx",
            Self::FxNoise => "fxNoise",
            Self::PointLights => "pointLights",
            Self::FxShotScope => "fxShotScope",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|capability| capability.as_str() == text)
    }
}

/// A part of the picture that a scene can declare and that this plan does not carry. Naming them
/// is what keeps "the plan has no lights" from being read as "the scene has no lights".
///
/// The scene-level ones (`Weather` ... `Paths`) are the sections `SceneGraph::unevaluated_sections`
/// reports; the node-level ones hang off a node (the plan names the node in `subject`). The
/// data maps (`Surface`, `NormalMap`, `SceneDepth`, `Relief`) do not change over time, which is
/// why the temporal evaluator does not list them, but they change what a plane looks like, so a
/// plan that ignored them would not be the picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Domain {
    Weather,
    WeatherKeys,
    Lighting,
    Effects,
    WeatherGround,
    Rigs,
    Paths,
    RigBind,
    Reveal,
    Surface,
    NormalMap,
    SceneDepth,
    Relief,
}

impl Domain {
    pub const ALL: [Domain; 13] = [
        Self::Weather,
        Self::WeatherKeys,
        Self::Lighting,
        Self::Effects,
        Self::WeatherGround,
        Self::Rigs,
        Self::Paths,
        Self::RigBind,
        Self::Reveal,
        Self::Surface,
        Self::NormalMap,
        Self::SceneDepth,
        Self::Relief,
    ];

    /// The sections of a node that are read from the raw document: the bundle's own key names.
    pub const NODE_SECTIONS: [Domain; 6] = [
        Self::RigBind,
        Self::Reveal,
        Self::Surface,
        Self::NormalMap,
        Self::SceneDepth,
        Self::Relief,
    ];

    /// The bundle's own key (`weatherKeys`, `rigBind`, ...), also the wire spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Weather => "weather",
            Self::WeatherKeys => "weatherKeys",
            Self::Lighting => "lighting",
            Self::Effects => "effects",
            Self::WeatherGround => "weatherGround",
            Self::Rigs => "rigs",
            Self::Paths => "paths",
            Self::RigBind => "rigBind",
            Self::Reveal => "reveal",
            Self::Surface => "surface",
            Self::NormalMap => "normalMap",
            Self::SceneDepth => "sceneDepth",
            Self::Relief => "relief",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|domain| domain.as_str() == text)
    }

    /// Whether the domain is declared on a node (and so carries a `subject`).
    pub fn is_node_level(self) -> bool {
        Self::NODE_SECTIONS.contains(&self)
    }
}

/// A declared domain the plan does not evaluate. `subject` is the node id for a node-level domain
/// and `None` for a scene-level one.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnsupportedDomain {
    pub domain: Domain,
    pub subject: Option<String>,
    /// Why, when the domain is partly supported and the plan can say which part it refuses:
    /// `Lighting` carries a [`LightingReason`] (F060). `None` for a domain nothing evaluates, and
    /// then absent from the wire form (plans without it keep their bytes).
    pub reason: Option<LightingReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UninterpretedReason {
    /// A key `render-bundle` 1.0 does not declare. The importer keeps it and reports it.
    UnknownField,
    /// A declared section the importer keeps raw (narration, outline, metadata, `contactPlaneId`,
    /// `frameAudit`). None of them is part of the frame's picture; they are listed so they are
    /// not silently dropped, and the ones that would change the picture are domains instead.
    DeferredSection,
}

impl UninterpretedReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnknownField => "unknownField",
            Self::DeferredSection => "deferredSection",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        [Self::UnknownField, Self::DeferredSection]
            .into_iter()
            .find(|reason| reason.as_str() == text)
    }
}

/// Data of the bundle that the plan does not interpret, by JSON Pointer into the bundle document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninterpretedField {
    pub pointer: String,
    pub reason: UninterpretedReason,
}

/// Where the plan comes from. Identity only: the plan never refers to the bundle by a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanSource {
    pub bundle_id: String,
    pub revision_id: String,
    pub scene_definition_id: String,
    pub scene_instance_id: String,
}

/// The picture the frame is drawn into (`renderProfile`). The color space is always
/// `OUTPUT_COLOR_SPACE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanOutput {
    pub width: u64,
    pub height: u64,
    pub fps: Fps,
    /// `straight` or `opaque`; the importer refuses `premultiplied`.
    pub alpha_mode: AssetAlpha,
}

/// Where the edit frame landed. Frames are integers, times are exact `Ratio`s of the frame rate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanFrame {
    pub edit_frame: u64,
    pub edit_time: Ratio,
    pub shot_id: String,
    pub instance_frame: u64,
    pub instance_time: Ratio,
    pub camera_clip_id: String,
    pub camera_clip_frame: u64,
    pub camera_clip_time: Ratio,
}

/// What a node hangs from: another node, or a named socket of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanParent {
    pub node_id: String,
    pub socket: Option<String>,
}

/// One node at the frame, in the scene definition's document order. `world_matrix` already has
/// every ancestor, socket and performance channel applied: a renderer places the node with it and
/// does not walk the hierarchy.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanNode {
    pub id: String,
    pub kind: NodeKind,
    pub parent: Option<PlanParent>,
    pub local_position: Vec3,
    /// XYZ Euler degrees, as the evaluator composes them.
    pub local_rotation_deg: Vec3,
    pub local_scale: Vec3,
    /// Column-major, relative to the parent.
    pub local_matrix: Mat4,
    /// Column-major, in scene space.
    pub world_matrix: Mat4,
    /// The node's own `renderOrder` (0 when absent). The effective draw order is in `layers`.
    pub render_order: i64,
    /// `assetId` of the node's asset (image planes and splats).
    pub asset_id: Option<String>,
    pub width_scene_units: Option<f64>,
    pub height_scene_units: Option<f64>,
}

/// An image plane to draw. `ScenePlan::layers` lists them back to front: the order is the plan's,
/// a runtime draws them as listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanLayer {
    pub node_id: String,
    pub asset_id: String,
    /// The alpha mode of the texture: whether to blend the layer over what is behind it.
    pub alpha: AssetAlpha,
    /// The node's own `renderOrder`.
    pub render_order: i64,
    /// The order a composition pass gives the node; it replaces `render_order` when present.
    pub pass_order: Option<i64>,
}

impl PlanLayer {
    /// The order the layer is sorted by (ties keep the document order of the nodes).
    pub fn effective_order(&self) -> i64 {
        self.pass_order.unwrap_or(self.render_order)
    }
}

/// A Gaussian-splat node to draw (S7, BL025 subset). The node's world matrix is in `nodes`, like
/// a layer's: the plan lists *which* nodes are splats and the asset each one reads, in the document
/// order of the nodes. Each splat says where it falls in the layers' painter's order
/// (`after_layers`); among splats of one slot Spark orders them per camera, so the list order
/// carries no paint order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanSplat {
    pub node_id: String,
    /// The `spz` resource the node reads.
    pub asset_id: String,
    /// How many of `layers` (in plan order) are painted before this splat: `0` is behind every
    /// plane, `layers.len()` in front of every plane. It is the index of the first layer whose
    /// node origin is in front of the camera (depth `> 0`) and strictly nearer to it than the splat
    /// node's origin (depth along the camera's viewing direction), else `layers.len()`: a layer
    /// on or behind the camera is invisible and never counts. Derived from the plan alone
    /// (`derive_splat_slots`); a plan that states another value is refused.
    pub after_layers: usize,
}

/// A texture or splat file the frame requires. The identity is `asset_id`; `relative_path` is where the bundle
/// declared it (relative to the bundle folder, never absolute) and, like `revision_id` and
/// `content_hash`, is metadata for whoever resolves and verifies the file. The plan does not
/// check that the file exists: resolving is the resource manifest's job (BL009).
#[derive(Debug, Clone, PartialEq)]
pub struct PlanResource {
    pub asset_id: String,
    pub revision_id: String,
    pub content_hash: String,
    pub relative_path: String,
    pub format: AssetFormat,
    pub width_px: u64,
    pub height_px: f64,
    pub alpha: AssetAlpha,
    pub color_space: AssetColorSpace,
    /// The nodes that use it (image planes for a `png`, splats for an `spz`), in the document
    /// order of the nodes.
    pub used_by: Vec<String>,
}

impl PlanResource {
    /// Whether the resource is a Gaussian-splat file (`spz`) and not a picture.
    pub fn is_splat(&self) -> bool {
        self.format == AssetFormat::Spz
    }
}

/// One point light of the frame (F060), already evaluated: keys resolved, parent added, flicker
/// applied. A runtime draws it as listed and knows no timeline.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanLight {
    pub id: String,
    /// World position in scene units.
    pub position: Vec3,
    /// Linear colour, each channel `[0, 1]`.
    pub color: Rgb,
    /// Multiplies the colour: the keyed intensity times the flicker level, `[0, 20]`.
    pub intensity: f64,
    /// Distance at which the light has fallen to a quarter (`> 0`, scene units).
    pub radius: f64,
    /// Kept in the scene but not lit: a runtime does not draw it (and it takes no slot), an editor
    /// may still show its marker. Absent from the wire form unless true.
    pub hidden: bool,
}

/// The evaluated plan of one edit frame. See the module documentation.
///
/// The fields are public so a runtime can read them; a plan from outside must come through
/// `from_json_str`, which runs `validate`. `build` validates what it returns.
#[derive(Debug, Clone, PartialEq)]
pub struct ScenePlan {
    pub version: u32,
    pub source: PlanSource,
    pub output: PlanOutput,
    pub frame: PlanFrame,
    pub camera: CameraState,
    /// Whether performance channels moved the nodes (and which); the matrices already include them.
    pub animation: NodeAnimation,
    /// Every node of the frame's scene, in document order.
    pub nodes: Vec<PlanNode>,
    /// The image planes, back to front.
    pub layers: Vec<PlanLayer>,
    /// The splat nodes, in document order; each is drawn at its `after_layers` slot among the layers. Empty for a scene without
    /// splats, and then absent from the wire form (S7: plans without splats keep their bytes).
    pub splats: Vec<PlanSplat>,
    /// The effect passes of the frame, in the order of composition (F016): already resolved
    /// (envelope applied, `amount` final), so a runtime draws them as listed and knows no
    /// timeline. The passes of the frame's shot (scope `shot`, F062) come first and the output's
    /// after them, each stage in track order; `validate` checks it. Empty without effects, and
    /// then absent from the wire form (plans without effects keep their bytes).
    pub fx: Vec<FxPass>,
    /// The point lights of the frame in document order (F060), at most four. Empty without
    /// lights, and then absent from the wire form (plans without lights keep their bytes).
    pub lights: Vec<PlanLight>,
    /// The textures and splat files the frame needs, ascending by `asset_id` (bytewise): the stable list a
    /// resource manifest is asked about.
    pub resources: Vec<PlanResource>,
    /// What a runtime needs to execute the plan; ascending, without repeats.
    pub requires: Vec<Capability>,
    /// Declared domains the plan does not evaluate. Empty for a complete plan.
    pub unsupported: Vec<UnsupportedDomain>,
    /// Bundle data the importer keeps and the plan does not interpret; ascending by pointer.
    pub uninterpreted: Vec<UninterpretedField>,
}

impl ScenePlan {
    /// Whether every domain the scene declares for this frame is in the plan. A partial plan
    /// (only produced on request, `PlanOptions::allow_partial`) is not the whole picture.
    pub fn is_complete(&self) -> bool {
        self.unsupported.is_empty()
    }

    /// The resources the frame requires, by `assetId`, in the stable order of `resources`.
    pub fn required_asset_ids(&self) -> Vec<&str> {
        self.resources
            .iter()
            .map(|resource| resource.asset_id.as_str())
            .collect()
    }

    pub fn resource(&self, asset_id: &str) -> Option<&PlanResource> {
        self.resources
            .iter()
            .find(|resource| resource.asset_id == asset_id)
    }

    pub fn node(&self, node_id: &str) -> Option<&PlanNode> {
        self.nodes.iter().find(|node| node.id == node_id)
    }

    /// Capabilities the plan needs that `supported` does not have, in the plan's order.
    pub fn missing_capabilities(&self, supported: &[Capability]) -> Vec<Capability> {
        self.requires
            .iter()
            .copied()
            .filter(|capability| !supported.contains(capability))
            .collect()
    }

    /// `Ok` when a runtime that has `supported` can execute the plan.
    pub fn require_capabilities(&self, supported: &[Capability]) -> Result<(), PlanError> {
        let missing = self.missing_capabilities(supported);
        if missing.is_empty() {
            return Ok(());
        }
        Err(PlanError::CapabilityUnavailable { missing })
    }
}

/// What this version of the plan can carry and what it refuses, independent of any scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub plan_version: u32,
    /// Everything a plan of this version can ask a runtime for.
    pub supported: &'static [Capability],
    /// Domains a scene can declare and no plan of this version carries.
    pub unsupported: &'static [Domain],
}

/// The capabilities of the plan of this build (`PLAN_VERSION`).
pub fn capabilities() -> Capabilities {
    Capabilities {
        plan_version: PLAN_VERSION,
        supported: &Capability::ALL,
        unsupported: &Domain::ALL,
    }
}
