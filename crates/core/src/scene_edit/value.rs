//! Typed values of the property catalog and how they are spelled in the bundle's JSON.

use serde_json::{Map, Number, Value, json};

use crate::directed_scene::{INVALID_TYPE, Quat, Vec3};

use super::error::EditError;

/// Largest integer a JavaScript number holds exactly (the importer refuses larger "integers").
const MAX_SAFE_INT: f64 = 9_007_199_254_740_991.0;

/// The shape of a property's value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    Number,
    Vec3,
    /// Rotation as the bundle stores it: `{ x, y, z, w }`, not Euler angles.
    Quaternion,
}

impl ValueKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Number => "number",
            Self::Vec3 => "vec3",
            Self::Quaternion => "quaternion",
        }
    }

    /// What a caller must send, for error messages and inspectors.
    pub fn shape(self) -> &'static str {
        match self {
            Self::Number => "a finite number",
            Self::Vec3 => "an object { x, y, z } of finite numbers",
            Self::Quaternion => "an object { x, y, z, w } of finite numbers",
        }
    }
}

/// A value a property holds or is set to. The catalog decides which kind each property takes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PropertyValue {
    Number(f64),
    Vec3(Vec3),
    Quaternion(Quat),
}

fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// An integral value is spelled as an integer (`16`, not `16.0`), as `JSON.stringify` does in the
/// origin, so an edited bundle keeps the look of one the origin wrote. Not finite has no JSON
/// spelling; the catalog refuses it before anything is written.
fn json_number(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() <= MAX_SAFE_INT {
        return Value::from(value as i64);
    }
    Number::from_f64(value).map_or(Value::Null, Value::Number)
}

fn number(value: &Value, what: &str) -> Result<f64, EditError> {
    value.as_f64().ok_or_else(|| {
        EditError::new(
            INVALID_TYPE,
            format!("{what} must be a number, got {}", json_kind(value)),
        )
    })
}

/// An object with no key outside `keys`: a stray key is more likely a mistake (a quaternion sent
/// to a position) than data worth ignoring.
fn strict_object<'a>(
    kind: ValueKind,
    value: &'a Value,
    keys: &[&str],
) -> Result<&'a Map<String, Value>, EditError> {
    let Value::Object(map) = value else {
        return Err(EditError::new(
            INVALID_TYPE,
            format!("expected {}, got {}", kind.shape(), json_kind(value)),
        ));
    };
    if let Some(extra) = map.keys().find(|key| !keys.contains(&key.as_str())) {
        return Err(EditError::new(
            INVALID_TYPE,
            format!(
                "expected {}, found the unexpected key `{extra}`",
                kind.shape()
            ),
        ));
    }
    Ok(map)
}

fn component(kind: ValueKind, map: &Map<String, Value>, key: &str) -> Result<f64, EditError> {
    let Some(value) = map.get(key) else {
        return Err(EditError::new(
            INVALID_TYPE,
            format!("expected {}, `{key}` is missing", kind.shape()),
        ));
    };
    number(value, &format!("`{key}`"))
}

impl PropertyValue {
    pub fn kind(&self) -> ValueKind {
        match self {
            Self::Number(_) => ValueKind::Number,
            Self::Vec3(_) => ValueKind::Vec3,
            Self::Quaternion(_) => ValueKind::Quaternion,
        }
    }

    /// Whether every component is a finite number. A JSON value always is; a value built in Rust
    /// may not be, and the catalog refuses it.
    pub fn is_finite(&self) -> bool {
        match self {
            Self::Number(number) => number.is_finite(),
            Self::Vec3(v) => v.x.is_finite() && v.y.is_finite() && v.z.is_finite(),
            Self::Quaternion(q) => {
                q.x.is_finite() && q.y.is_finite() && q.z.is_finite() && q.w.is_finite()
            }
        }
    }

    /// Decodes the JSON an op carries, strictly: the exact keys of the kind, all numbers.
    /// `INVALID_TYPE` otherwise; ranges and finiteness are the catalog's check, not this one's.
    pub fn from_json(kind: ValueKind, value: &Value) -> Result<Self, EditError> {
        match kind {
            ValueKind::Number => number(value, "the value").map(Self::Number),
            ValueKind::Vec3 => {
                let map = strict_object(kind, value, &["x", "y", "z"])?;
                Ok(Self::Vec3(Vec3 {
                    x: component(kind, map, "x")?,
                    y: component(kind, map, "y")?,
                    z: component(kind, map, "z")?,
                }))
            }
            ValueKind::Quaternion => {
                let map = strict_object(kind, value, &["x", "y", "z", "w"])?;
                Ok(Self::Quaternion(Quat {
                    x: component(kind, map, "x")?,
                    y: component(kind, map, "y")?,
                    z: component(kind, map, "z")?,
                    w: component(kind, map, "w")?,
                }))
            }
        }
    }

    pub fn to_json(&self) -> Value {
        match self {
            Self::Number(number) => json_number(*number),
            Self::Vec3(v) => {
                json!({ "x": json_number(v.x), "y": json_number(v.y), "z": json_number(v.z) })
            }
            Self::Quaternion(q) => json!({
                "x": json_number(q.x),
                "y": json_number(q.y),
                "z": json_number(q.z),
                "w": json_number(q.w),
            }),
        }
    }

    /// Reads a value out of a document the importer accepted. `None` only if the document is not
    /// shaped as the importer guarantees.
    pub(super) fn read_json(kind: ValueKind, value: &Value) -> Option<Self> {
        let field = |key: &str| value.get(key)?.as_f64();
        match kind {
            ValueKind::Number => value.as_f64().map(Self::Number),
            ValueKind::Vec3 => Some(Self::Vec3(Vec3 {
                x: field("x")?,
                y: field("y")?,
                z: field("z")?,
            })),
            ValueKind::Quaternion => Some(Self::Quaternion(Quat {
                x: field("x")?,
                y: field("y")?,
                z: field("z")?,
                w: field("w")?,
            })),
        }
    }

    /// Writes the value into the JSON at `slot`; `false` when a compound value finds no object
    /// there. A compound value sets its components one by one and leaves a component that already
    /// holds the number alone, so keys the schema does not declare, and the spelling of numbers
    /// that did not change, survive the edit (the document is the canonical data).
    pub(super) fn write_json(&self, slot: &mut Value) -> bool {
        let parts = match self {
            Self::Number(number) => {
                *slot = json_number(*number);
                return true;
            }
            Self::Vec3(v) => vec![("x", v.x), ("y", v.y), ("z", v.z)],
            Self::Quaternion(q) => vec![("x", q.x), ("y", q.y), ("z", q.z), ("w", q.w)],
        };
        let Some(map) = slot.as_object_mut() else {
            return false;
        };
        for (key, component) in parts {
            if map.get(key).and_then(Value::as_f64) != Some(component) {
                map.insert(key.to_string(), json_number(component));
            }
        }
        true
    }
}
