//! The catalog of editable properties: stable paths, type, unit, default, range, applicability.
//!
//! Scope (BB04): the 3D transform of a scene node, the measures and `renderOrder` of an image
//! plane and the start pose of a camera clip, each vector also addressable one component at a
//! time (`position/x`). Lighting, weather, effects, rig, path and the keys of a camera movement
//! are not here; the catalog grows by adding entries, not by a generic setter.
//!
//! WHY a static catalog and not a trait object per property: every entry is data (what a
//! inspector, a CLI `describe` and a batch validator all need), and the importer stays the only
//! owner of integrity rules. The catalog states what a property *is*; it does not repeat what
//! makes a bundle valid, except for the bounds a slider needs (see `NumberRange`).

use crate::directed_scene::{INVALID_TYPE, INVALID_VALUE, NodeKind, Quat, Vec3};

use super::error::EditError;
use super::value::{MAX_SAFE_INT, PropertyValue, ValueKind};

/// Unit of a number, or of each component of a vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// The scene's own length unit (the bundle's `*SceneUnits` fields and positions).
    SceneUnits,
    Degrees,
    /// A multiplier or a quaternion component: no unit.
    Unitless,
}

impl Unit {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SceneUnits => "scene-units",
            Self::Degrees => "degrees",
            Self::Unitless => "unitless",
        }
    }
}

/// One end of a number range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limit {
    pub value: f64,
    /// Whether the value itself is allowed.
    pub inclusive: bool,
}

/// Bounds of a number property. They state the property's contract (what an inspector slider
/// offers and what `NumberRange::describe` tells a caller); the importer still decides the bundle
/// is valid, and a test pins that both agree at the edges.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NumberRange {
    pub min: Option<Limit>,
    pub max: Option<Limit>,
}

impl Limit {
    /// `value` is on the allowed side of this limit taken as a lower bound.
    fn admits_above(self, value: f64) -> bool {
        value > self.value || (self.inclusive && value == self.value)
    }

    /// `value` is on the allowed side of this limit taken as an upper bound.
    fn admits_below(self, value: f64) -> bool {
        value < self.value || (self.inclusive && value == self.value)
    }
}

impl NumberRange {
    pub fn contains(&self, value: f64) -> bool {
        self.min.is_none_or(|min| min.admits_above(value))
            && self.max.is_none_or(|max| max.admits_below(value))
    }

    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some(min) = self.min {
            let word = if min.inclusive {
                "at least"
            } else {
                "greater than"
            };
            parts.push(format!("{word} {}", min.value));
        }
        if let Some(max) = self.max {
            let word = if max.inclusive {
                "at most"
            } else {
                "less than"
            };
            parts.push(format!("{word} {}", max.value));
        }
        parts.join(" and ")
    }
}

/// Where a property can be set.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Applicability {
    /// Nodes of these kinds.
    Node(&'static [NodeKind]),
    CameraClip,
    /// The point lights of a scene definition (F060).
    Light,
}

impl Applicability {
    pub fn accepts_node(&self, kind: NodeKind) -> bool {
        match self {
            Self::Node(kinds) => kinds.contains(&kind),
            Self::CameraClip | Self::Light => false,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Node(kinds) => {
                let names: Vec<&str> = kinds.iter().map(|kind| kind.as_str()).collect();
                format!("{} nodes", names.join(", "))
            }
            Self::CameraClip => "camera clips".to_string(),
            Self::Light => "point lights".to_string(),
        }
    }
}

/// How the property relates to the origin's property layer (`packages/scene-engine`
/// `properties/`), so nobody assumes equivalence the code does not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginRelation {
    /// The origin has no counterpart: it edits nodes only by writing the bundle again.
    EditorOnly,
    /// The origin has a command for it (`setStart.target`/`fovDeg`) but no property path; the
    /// path is this catalog's, spelled in the origin's grammar.
    CommandOnly,
    /// Same path and the same write as the origin's: before the start moves, the clip's first
    /// movement is turned into keyframes that make the same frames, so only the beginning of the
    /// path moves and the rest stays where it was (see `camera_keys`). Compared with the origin
    /// by the fixed oracle in `docs/migration/fixtures/scene-edit`.
    PathEquivalentWrite,
    /// A component of a camera start (`start/position/x`): the origin has no per-axis path or
    /// command. The write is the origin's write of the parent path (`component_of.parent`) with
    /// that one axis replaced, and is only as equivalent as that write is; the fixed oracle builds
    /// the expected answer by composing the vector, it does not ask the origin for an axis.
    ParentWrite,
}

impl OriginRelation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EditorOnly => "editor-only",
            Self::CommandOnly => "command-only",
            Self::PathEquivalentWrite => "path-equivalent-write",
            Self::ParentWrite => "parent-write",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeProperty {
    Position,
    Rotation,
    Scale,
    /// Image planes only.
    Width,
    /// Image planes only.
    Height,
    /// Image planes only: the order the runtime draws them in (back to front).
    RenderOrder,
    PositionX,
    PositionY,
    PositionZ,
    ScaleX,
    ScaleY,
    ScaleZ,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CameraProperty {
    StartPosition,
    StartTarget,
    StartFov,
    StartPositionX,
    StartPositionY,
    StartPositionZ,
    StartTargetX,
    StartTargetY,
    StartTargetZ,
}

/// The properties of a point light (F060). `Radius` is the document's `radius`: the distance at
/// which the light has fallen to a quarter, the "alcance" the range handle edits. `Color*` are
/// the three linear channels, each a number of its own (the catalog has no colour value kind and
/// an inspector wants one slider per channel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LightProperty {
    Position,
    Intensity,
    Radius,
    PositionX,
    PositionY,
    PositionZ,
    ColorR,
    ColorG,
    ColorB,
}

/// Identifies a catalog entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PropertyId {
    Node(NodeProperty),
    Camera(CameraProperty),
    Light(LightProperty),
}

/// A property that is one component of a vector property (`position/x` of `position`).
///
/// Writing a component is writing the parent with that component replaced: same applicability,
/// same checks, same side effects (a camera start component converts the first movement exactly
/// as the whole vector does). It exists so a slider, a script or a gizmo axis can address one
/// number without sending the other two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentOf {
    pub parent: PropertyId,
    /// 0, 1, 2 for `x`, `y`, `z`.
    pub index: usize,
}

/// One property of the catalog.
#[derive(Debug, Clone, PartialEq)]
pub struct PropertySpec {
    pub id: PropertyId,
    /// What follows the target in the path: `position`, `position/x`, `start/fov`.
    pub name: &'static str,
    pub kind: ValueKind,
    pub unit: Unit,
    /// Bounds of a `Number` property; `None` for vectors and quaternions.
    pub range: Option<NumberRange>,
    /// The neutral value `reset` writes. `None` when no neutral value exists (an image plane has
    /// no default size; a camera has no default pose), so a reset is refused, not guessed.
    pub default: Option<PropertyValue>,
    pub applies_to: Applicability,
    /// The key of the node or clip object in the bundle's JSON that stores the property
    /// (`localPosition/x` for a component: a path inside the object).
    pub document_field: &'static str,
    pub origin: OriginRelation,
    /// `Some` for a component of a vector property; `None` for a property in its own right.
    pub component: Option<ComponentOf>,
    /// The bundle may leave the field out, and an absent field reads as `default` (`renderOrder`).
    /// Writing the default over an absent field leaves the document untouched.
    pub optional: bool,
    /// A write that changes the value first turns the first movement of the clip into keyframes
    /// (the origin's `start/position` write).
    pub converts_first_movement: bool,
}

/// `[0, 20]`: the origin's bounds of a light's intensity.
const LIGHT_INTENSITY: NumberRange = NumberRange {
    min: Some(Limit {
        value: 0.0,
        inclusive: true,
    }),
    max: Some(Limit {
        value: crate::directed_scene::lighting::MAX_INTENSITY,
        inclusive: true,
    }),
};

/// `[0, 1]`: one linear channel of a light's colour.
const UNIT_INTERVAL: NumberRange = NumberRange {
    min: Some(Limit {
        value: 0.0,
        inclusive: true,
    }),
    max: Some(Limit {
        value: 1.0,
        inclusive: true,
    }),
};

const EVERY_NODE: &[NodeKind] = &[NodeKind::Group, NodeKind::ImagePlane, NodeKind::Splat];
const IMAGE_PLANES: &[NodeKind] = &[NodeKind::ImagePlane];

/// `> 0`, as the schema says for `widthSceneUnits`/`heightSceneUnits`.
const POSITIVE: NumberRange = NumberRange {
    min: Some(Limit {
        value: 0.0,
        inclusive: false,
    }),
    max: None,
};

/// `(0, 180)`: the field of view a camera can have (the origin's property check and the
/// importer's `FOV_OUT_OF_RANGE` agree).
const FIELD_OF_VIEW: NumberRange = NumberRange {
    min: Some(Limit {
        value: 0.0,
        inclusive: false,
    }),
    max: Some(Limit {
        value: 180.0,
        inclusive: false,
    }),
};

/// `[-(2^53 - 1), 2^53 - 1]`: the integers the bundle (and the origin's JS numbers) hold exactly.
const SAFE_INTEGER: NumberRange = NumberRange {
    min: Some(Limit {
        value: -(MAX_SAFE_INT as f64),
        inclusive: true,
    }),
    max: Some(Limit {
        value: MAX_SAFE_INT as f64,
        inclusive: true,
    }),
};

static NODE_POSITION: PropertySpec = PropertySpec {
    id: PropertyId::Node(NodeProperty::Position),
    name: "position",
    kind: ValueKind::Vec3,
    unit: Unit::SceneUnits,
    range: None,
    default: Some(PropertyValue::Vec3(Vec3::ZERO)),
    applies_to: Applicability::Node(EVERY_NODE),
    document_field: "localPosition",
    origin: OriginRelation::EditorOnly,
    component: None,
    optional: false,
    converts_first_movement: false,
};

// A quaternion, not Euler angles: that is what the bundle stores and composes. Its length must
// stay near 1; that rule belongs to the importer and is reported with the importer's code.
static NODE_ROTATION: PropertySpec = PropertySpec {
    id: PropertyId::Node(NodeProperty::Rotation),
    name: "rotation",
    kind: ValueKind::Quaternion,
    unit: Unit::Unitless,
    range: None,
    default: Some(PropertyValue::Quaternion(Quat::IDENTITY)),
    applies_to: Applicability::Node(EVERY_NODE),
    document_field: "localRotation",
    origin: OriginRelation::EditorOnly,
    component: None,
    optional: false,
    converts_first_movement: false,
};

static NODE_SCALE: PropertySpec = PropertySpec {
    id: PropertyId::Node(NodeProperty::Scale),
    name: "scale",
    kind: ValueKind::Vec3,
    unit: Unit::Unitless,
    range: None,
    default: Some(PropertyValue::Vec3(Vec3::ONE)),
    applies_to: Applicability::Node(EVERY_NODE),
    document_field: "localScale",
    origin: OriginRelation::EditorOnly,
    component: None,
    optional: false,
    converts_first_movement: false,
};

static NODE_WIDTH: PropertySpec = PropertySpec {
    id: PropertyId::Node(NodeProperty::Width),
    name: "width",
    kind: ValueKind::Number,
    unit: Unit::SceneUnits,
    range: Some(POSITIVE),
    default: None,
    applies_to: Applicability::Node(IMAGE_PLANES),
    document_field: "widthSceneUnits",
    origin: OriginRelation::EditorOnly,
    component: None,
    optional: false,
    converts_first_movement: false,
};

static NODE_HEIGHT: PropertySpec = PropertySpec {
    id: PropertyId::Node(NodeProperty::Height),
    name: "height",
    kind: ValueKind::Number,
    unit: Unit::SceneUnits,
    range: Some(POSITIVE),
    default: None,
    applies_to: Applicability::Node(IMAGE_PLANES),
    document_field: "heightSceneUnits",
    origin: OriginRelation::EditorOnly,
    component: None,
    optional: false,
    converts_first_movement: false,
};

static CAMERA_START_POSITION: PropertySpec = PropertySpec {
    id: PropertyId::Camera(CameraProperty::StartPosition),
    name: "start/position",
    kind: ValueKind::Vec3,
    unit: Unit::SceneUnits,
    range: None,
    default: None,
    applies_to: Applicability::CameraClip,
    document_field: "initialPosition",
    origin: OriginRelation::PathEquivalentWrite,
    component: None,
    optional: false,
    converts_first_movement: true,
};

static CAMERA_START_TARGET: PropertySpec = PropertySpec {
    id: PropertyId::Camera(CameraProperty::StartTarget),
    name: "start/target",
    kind: ValueKind::Vec3,
    unit: Unit::SceneUnits,
    range: None,
    default: None,
    applies_to: Applicability::CameraClip,
    document_field: "initialTarget",
    origin: OriginRelation::CommandOnly,
    component: None,
    optional: false,
    converts_first_movement: false,
};

static CAMERA_START_FOV: PropertySpec = PropertySpec {
    id: PropertyId::Camera(CameraProperty::StartFov),
    name: "start/fov",
    kind: ValueKind::Number,
    unit: Unit::Degrees,
    range: Some(FIELD_OF_VIEW),
    default: None,
    applies_to: Applicability::CameraClip,
    document_field: "initialFovDeg",
    origin: OriginRelation::CommandOnly,
    component: None,
    optional: false,
    converts_first_movement: false,
};

/// The draw order of an image plane. The bundle leaves it out for 0, and a composition pass that
/// covers the node overrides it (`Bundle::paint_order`); the catalog stores the node's own value.
static NODE_RENDER_ORDER: PropertySpec = PropertySpec {
    id: PropertyId::Node(NodeProperty::RenderOrder),
    name: "renderOrder",
    kind: ValueKind::Integer,
    unit: Unit::Unitless,
    range: Some(SAFE_INTEGER),
    default: Some(PropertyValue::Integer(0)),
    applies_to: Applicability::Node(IMAGE_PLANES),
    document_field: "renderOrder",
    origin: OriginRelation::EditorOnly,
    component: None,
    optional: true,
    converts_first_movement: false,
};

// A light has no neutral value to reset to (a lamp at the origin, intensity 1, is as arbitrary as
// any other), so none of these has a default: `reset` is refused, not guessed.
static LIGHT_POSITION: PropertySpec = PropertySpec {
    id: PropertyId::Light(LightProperty::Position),
    name: "position",
    kind: ValueKind::Vec3,
    unit: Unit::SceneUnits,
    range: None,
    default: None,
    applies_to: Applicability::Light,
    document_field: "position",
    origin: OriginRelation::EditorOnly,
    component: None,
    optional: false,
    converts_first_movement: false,
};

static LIGHT_INTENSITY_SPEC: PropertySpec = PropertySpec {
    id: PropertyId::Light(LightProperty::Intensity),
    name: "intensity",
    kind: ValueKind::Number,
    unit: Unit::Unitless,
    range: Some(LIGHT_INTENSITY),
    default: None,
    applies_to: Applicability::Light,
    document_field: "intensity",
    origin: OriginRelation::EditorOnly,
    component: None,
    optional: false,
    converts_first_movement: false,
};

static LIGHT_RADIUS: PropertySpec = PropertySpec {
    id: PropertyId::Light(LightProperty::Radius),
    name: "radius",
    kind: ValueKind::Number,
    unit: Unit::SceneUnits,
    range: Some(POSITIVE),
    default: None,
    applies_to: Applicability::Light,
    document_field: "radius",
    origin: OriginRelation::EditorOnly,
    component: None,
    optional: false,
    converts_first_movement: false,
};

const fn light_channel(
    id: LightProperty,
    name: &'static str,
    document_field: &'static str,
) -> PropertySpec {
    PropertySpec {
        id: PropertyId::Light(id),
        name,
        kind: ValueKind::Number,
        unit: Unit::Unitless,
        range: Some(UNIT_INTERVAL),
        default: None,
        applies_to: Applicability::Light,
        document_field,
        origin: OriginRelation::EditorOnly,
        component: None,
        optional: false,
        converts_first_movement: false,
    }
}

static LIGHT_COLOR_R: PropertySpec = light_channel(LightProperty::ColorR, "color/r", "color/r");
static LIGHT_COLOR_G: PropertySpec = light_channel(LightProperty::ColorG, "color/g", "color/g");
static LIGHT_COLOR_B: PropertySpec = light_channel(LightProperty::ColorB, "color/b", "color/b");

/// One component of a vector property. A `const fn` so the twelve entries below stay data: the
/// only things that differ are the names, the index and the neutral value of the component.
const fn component_spec(
    parent: &'static PropertySpec,
    id: PropertyId,
    name: &'static str,
    document_field: &'static str,
    index: usize,
    default: Option<PropertyValue>,
) -> PropertySpec {
    PropertySpec {
        id,
        name,
        kind: ValueKind::Number,
        unit: parent.unit,
        range: None,
        default,
        applies_to: parent.applies_to,
        document_field,
        // A node component stays what its parent is (the origin has no node paths at all); a camera
        // start component is the parent's write with one axis replaced, which is not the origin's
        // path.
        origin: match parent.origin {
            OriginRelation::EditorOnly => OriginRelation::EditorOnly,
            _ => OriginRelation::ParentWrite,
        },
        component: Some(ComponentOf {
            parent: parent.id,
            index,
        }),
        optional: false,
        converts_first_movement: parent.converts_first_movement,
    }
}

const ZERO: Option<PropertyValue> = Some(PropertyValue::Number(0.0));
const ONE: Option<PropertyValue> = Some(PropertyValue::Number(1.0));

macro_rules! node_component {
    ($item:ident, $parent:ident, $variant:ident, $name:literal, $field:literal, $index:literal, $default:expr) => {
        static $item: PropertySpec = component_spec(
            &$parent,
            PropertyId::Node(NodeProperty::$variant),
            $name,
            $field,
            $index,
            $default,
        );
    };
}

macro_rules! camera_component {
    ($item:ident, $parent:ident, $variant:ident, $name:literal, $field:literal, $index:literal) => {
        static $item: PropertySpec = component_spec(
            &$parent,
            PropertyId::Camera(CameraProperty::$variant),
            $name,
            $field,
            $index,
            None,
        );
    };
}

static LIGHT_POSITION_X: PropertySpec = component_spec(
    &LIGHT_POSITION,
    PropertyId::Light(LightProperty::PositionX),
    "position/x",
    "position/x",
    0,
    None,
);
static LIGHT_POSITION_Y: PropertySpec = component_spec(
    &LIGHT_POSITION,
    PropertyId::Light(LightProperty::PositionY),
    "position/y",
    "position/y",
    1,
    None,
);
static LIGHT_POSITION_Z: PropertySpec = component_spec(
    &LIGHT_POSITION,
    PropertyId::Light(LightProperty::PositionZ),
    "position/z",
    "position/z",
    2,
    None,
);

node_component!(
    NODE_POSITION_X,
    NODE_POSITION,
    PositionX,
    "position/x",
    "localPosition/x",
    0,
    ZERO
);
node_component!(
    NODE_POSITION_Y,
    NODE_POSITION,
    PositionY,
    "position/y",
    "localPosition/y",
    1,
    ZERO
);
node_component!(
    NODE_POSITION_Z,
    NODE_POSITION,
    PositionZ,
    "position/z",
    "localPosition/z",
    2,
    ZERO
);
node_component!(
    NODE_SCALE_X,
    NODE_SCALE,
    ScaleX,
    "scale/x",
    "localScale/x",
    0,
    ONE
);
node_component!(
    NODE_SCALE_Y,
    NODE_SCALE,
    ScaleY,
    "scale/y",
    "localScale/y",
    1,
    ONE
);
node_component!(
    NODE_SCALE_Z,
    NODE_SCALE,
    ScaleZ,
    "scale/z",
    "localScale/z",
    2,
    ONE
);
camera_component!(
    CAMERA_START_POSITION_X,
    CAMERA_START_POSITION,
    StartPositionX,
    "start/position/x",
    "initialPosition/x",
    0
);
camera_component!(
    CAMERA_START_POSITION_Y,
    CAMERA_START_POSITION,
    StartPositionY,
    "start/position/y",
    "initialPosition/y",
    1
);
camera_component!(
    CAMERA_START_POSITION_Z,
    CAMERA_START_POSITION,
    StartPositionZ,
    "start/position/z",
    "initialPosition/z",
    2
);
camera_component!(
    CAMERA_START_TARGET_X,
    CAMERA_START_TARGET,
    StartTargetX,
    "start/target/x",
    "initialTarget/x",
    0
);
camera_component!(
    CAMERA_START_TARGET_Y,
    CAMERA_START_TARGET,
    StartTargetY,
    "start/target/y",
    "initialTarget/y",
    1
);
camera_component!(
    CAMERA_START_TARGET_Z,
    CAMERA_START_TARGET,
    StartTargetZ,
    "start/target/z",
    "initialTarget/z",
    2
);

// The first eight entries and their order are the original catalog; later ones are only appended,
// so an index or a listing written against the first slice keeps meaning the same thing.
static CATALOG: [&PropertySpec; 30] = [
    &NODE_POSITION,
    &NODE_ROTATION,
    &NODE_SCALE,
    &NODE_WIDTH,
    &NODE_HEIGHT,
    &CAMERA_START_POSITION,
    &CAMERA_START_TARGET,
    &CAMERA_START_FOV,
    &NODE_RENDER_ORDER,
    &NODE_POSITION_X,
    &NODE_POSITION_Y,
    &NODE_POSITION_Z,
    &NODE_SCALE_X,
    &NODE_SCALE_Y,
    &NODE_SCALE_Z,
    &CAMERA_START_POSITION_X,
    &CAMERA_START_POSITION_Y,
    &CAMERA_START_POSITION_Z,
    &CAMERA_START_TARGET_X,
    &CAMERA_START_TARGET_Y,
    &CAMERA_START_TARGET_Z,
    &LIGHT_POSITION,
    &LIGHT_INTENSITY_SPEC,
    &LIGHT_RADIUS,
    &LIGHT_POSITION_X,
    &LIGHT_POSITION_Y,
    &LIGHT_POSITION_Z,
    &LIGHT_COLOR_R,
    &LIGHT_COLOR_G,
    &LIGHT_COLOR_B,
];

/// Every property the editor can change by ID, in a stable order: the first slice's eight, then
/// `renderOrder`, then the components.
pub fn catalog() -> &'static [&'static PropertySpec] {
    &CATALOG
}

impl NodeProperty {
    pub const ALL: [Self; 12] = [
        Self::Position,
        Self::Rotation,
        Self::Scale,
        Self::Width,
        Self::Height,
        Self::RenderOrder,
        Self::PositionX,
        Self::PositionY,
        Self::PositionZ,
        Self::ScaleX,
        Self::ScaleY,
        Self::ScaleZ,
    ];

    pub fn spec(self) -> &'static PropertySpec {
        match self {
            Self::Position => &NODE_POSITION,
            Self::Rotation => &NODE_ROTATION,
            Self::Scale => &NODE_SCALE,
            Self::Width => &NODE_WIDTH,
            Self::Height => &NODE_HEIGHT,
            Self::RenderOrder => &NODE_RENDER_ORDER,
            Self::PositionX => &NODE_POSITION_X,
            Self::PositionY => &NODE_POSITION_Y,
            Self::PositionZ => &NODE_POSITION_Z,
            Self::ScaleX => &NODE_SCALE_X,
            Self::ScaleY => &NODE_SCALE_Y,
            Self::ScaleZ => &NODE_SCALE_Z,
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|property| property.spec().name == name)
    }
}

impl CameraProperty {
    pub const ALL: [Self; 9] = [
        Self::StartPosition,
        Self::StartTarget,
        Self::StartFov,
        Self::StartPositionX,
        Self::StartPositionY,
        Self::StartPositionZ,
        Self::StartTargetX,
        Self::StartTargetY,
        Self::StartTargetZ,
    ];

    pub fn spec(self) -> &'static PropertySpec {
        match self {
            Self::StartPosition => &CAMERA_START_POSITION,
            Self::StartTarget => &CAMERA_START_TARGET,
            Self::StartFov => &CAMERA_START_FOV,
            Self::StartPositionX => &CAMERA_START_POSITION_X,
            Self::StartPositionY => &CAMERA_START_POSITION_Y,
            Self::StartPositionZ => &CAMERA_START_POSITION_Z,
            Self::StartTargetX => &CAMERA_START_TARGET_X,
            Self::StartTargetY => &CAMERA_START_TARGET_Y,
            Self::StartTargetZ => &CAMERA_START_TARGET_Z,
        }
    }

    /// `name` is what follows `start/` in the path (`fov`, `position/x`).
    pub fn from_start_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|property| {
            property
                .spec()
                .name
                .strip_prefix("start/")
                .is_some_and(|rest| rest == name)
        })
    }
}

impl LightProperty {
    pub const ALL: [Self; 9] = [
        Self::Position,
        Self::Intensity,
        Self::Radius,
        Self::PositionX,
        Self::PositionY,
        Self::PositionZ,
        Self::ColorR,
        Self::ColorG,
        Self::ColorB,
    ];

    pub fn spec(self) -> &'static PropertySpec {
        match self {
            Self::Position => &LIGHT_POSITION,
            Self::Intensity => &LIGHT_INTENSITY_SPEC,
            Self::Radius => &LIGHT_RADIUS,
            Self::PositionX => &LIGHT_POSITION_X,
            Self::PositionY => &LIGHT_POSITION_Y,
            Self::PositionZ => &LIGHT_POSITION_Z,
            Self::ColorR => &LIGHT_COLOR_R,
            Self::ColorG => &LIGHT_COLOR_G,
            Self::ColorB => &LIGHT_COLOR_B,
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|property| property.spec().name == name)
    }

    /// The key track that replaces this property's static value, if the origin has one. `radius`
    /// has no keys in the origin's schema: it is never animated.
    pub fn channel(self) -> Option<crate::directed_scene::lighting::LightChannel> {
        use crate::directed_scene::lighting::LightChannel;
        match self {
            Self::Position | Self::PositionX | Self::PositionY | Self::PositionZ => {
                Some(LightChannel::Position)
            }
            Self::Intensity => Some(LightChannel::Intensity),
            Self::ColorR | Self::ColorG | Self::ColorB => Some(LightChannel::Color),
            Self::Radius => None,
        }
    }
}

impl PropertyId {
    pub fn spec(self) -> &'static PropertySpec {
        match self {
            Self::Node(property) => property.spec(),
            Self::Camera(property) => property.spec(),
            Self::Light(property) => property.spec(),
        }
    }
}

impl PropertySpec {
    /// Checks a value against the entry: its kind, that it is finite, and its range. Whatever
    /// else makes a bundle valid (unit-length quaternions, the world matrix staying finite) is
    /// the importer's rule and is checked on the whole candidate, not repeated here.
    pub fn validate(&self, value: &PropertyValue) -> Result<(), EditError> {
        if value.kind() != self.kind {
            return Err(EditError::new(
                INVALID_TYPE,
                format!(
                    "`{}` expects {}, got a {}",
                    self.name,
                    self.kind.shape(),
                    value.kind().as_str()
                ),
            ));
        }
        if !value.is_finite() {
            return Err(EditError::new(
                INVALID_VALUE,
                format!("`{}` must be finite", self.name),
            ));
        }
        let number = match value {
            PropertyValue::Number(number) => Some(*number),
            PropertyValue::Integer(integer) => Some(*integer as f64),
            _ => None,
        };
        if let (Some(range), Some(number)) = (self.range, number)
            && !range.contains(number)
        {
            return Err(EditError::new(
                INVALID_VALUE,
                format!(
                    "`{}` must be a number {} ({}), got {number}",
                    self.name,
                    range.describe(),
                    self.unit.as_str()
                ),
            ));
        }
        Ok(())
    }
}
