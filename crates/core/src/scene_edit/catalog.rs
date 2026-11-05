//! The catalog of editable properties: stable paths, type, unit, default, range, applicability.
//!
//! Scope of this first slice (BB04): the 3D transform of a scene node, the measures of an image
//! plane and the start pose of a camera clip. Lighting, weather, effects, rig, path and the
//! keys of a camera movement are not here; the catalog grows by adding entries, not by a
//! generic setter.
//!
//! WHY a static catalog and not a trait object per property: every entry is data (what a
//! inspector, a CLI `describe` and a batch validator all need), and the importer stays the only
//! owner of integrity rules. The catalog states what a property *is*; it does not repeat what
//! makes a bundle valid, except for the bounds a slider needs (see `NumberRange`).

use crate::directed_scene::{INVALID_TYPE, INVALID_VALUE, NodeKind, Quat, Vec3};

use super::error::EditError;
use super::value::{PropertyValue, ValueKind};

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
}

impl Applicability {
    pub fn accepts_node(&self, kind: NodeKind) -> bool {
        match self {
            Self::Node(kinds) => kinds.contains(&kind),
            Self::CameraClip => false,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Node(kinds) => {
                let names: Vec<&str> = kinds.iter().map(|kind| kind.as_str()).collect();
                format!("{} nodes", names.join(", "))
            }
            Self::CameraClip => "camera clips".to_string(),
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
    /// Same path as the origin's, but a narrower write: the origin first turns the clip's first
    /// movement into keyframes so the rest of the path stays where it was; here only the
    /// initial pose changes. Preserving the path needs temporal evaluation (BB03).
    PathNarrowerWrite,
}

impl OriginRelation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EditorOnly => "editor-only",
            Self::CommandOnly => "command-only",
            Self::PathNarrowerWrite => "path-narrower-write",
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CameraProperty {
    StartPosition,
    StartTarget,
    StartFov,
}

/// Identifies a catalog entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PropertyId {
    Node(NodeProperty),
    Camera(CameraProperty),
}

/// One property of the catalog.
#[derive(Debug, Clone, PartialEq)]
pub struct PropertySpec {
    pub id: PropertyId,
    /// What follows the target in the path: `position`, `start/fov`.
    pub name: &'static str,
    pub kind: ValueKind,
    pub unit: Unit,
    /// Bounds of a `Number` property; `None` for vectors and quaternions.
    pub range: Option<NumberRange>,
    /// The neutral value `reset` writes. `None` when no neutral value exists (an image plane has
    /// no default size; a camera has no default pose), so a reset is refused, not guessed.
    pub default: Option<PropertyValue>,
    pub applies_to: Applicability,
    /// The key of the node or clip object in the bundle's JSON that stores the property.
    pub document_field: &'static str,
    pub origin: OriginRelation,
}

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
    origin: OriginRelation::PathNarrowerWrite,
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
};

static CATALOG: [&PropertySpec; 8] = [
    &NODE_POSITION,
    &NODE_ROTATION,
    &NODE_SCALE,
    &NODE_WIDTH,
    &NODE_HEIGHT,
    &CAMERA_START_POSITION,
    &CAMERA_START_TARGET,
    &CAMERA_START_FOV,
];

/// Every property the editor can change by ID, in a stable order.
pub fn catalog() -> &'static [&'static PropertySpec] {
    &CATALOG
}

impl NodeProperty {
    pub const ALL: [Self; 5] = [
        Self::Position,
        Self::Rotation,
        Self::Scale,
        Self::Width,
        Self::Height,
    ];

    pub fn spec(self) -> &'static PropertySpec {
        match self {
            Self::Position => &NODE_POSITION,
            Self::Rotation => &NODE_ROTATION,
            Self::Scale => &NODE_SCALE,
            Self::Width => &NODE_WIDTH,
            Self::Height => &NODE_HEIGHT,
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|property| property.spec().name == name)
    }
}

impl CameraProperty {
    pub const ALL: [Self; 3] = [Self::StartPosition, Self::StartTarget, Self::StartFov];

    pub fn spec(self) -> &'static PropertySpec {
        match self {
            Self::StartPosition => &CAMERA_START_POSITION,
            Self::StartTarget => &CAMERA_START_TARGET,
            Self::StartFov => &CAMERA_START_FOV,
        }
    }

    /// `name` is what follows `start/` in the path.
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

impl PropertyId {
    pub fn spec(self) -> &'static PropertySpec {
        match self {
            Self::Node(property) => property.spec(),
            Self::Camera(property) => property.spec(),
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
        if let (Some(range), PropertyValue::Number(number)) = (self.range, value)
            && !range.contains(*number)
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
