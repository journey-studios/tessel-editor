//! Edit operations: set or reset one catalog property of one node or clip, by ID.
//!
//! There is no free-form setter: an op names a catalog property, and a JSON value is decoded by
//! the kind that property declares, so a value cannot reach the document by another route.

use serde_json::Value;

use super::error::{EditError, INVALID_OP};
use super::reference::PropertyRef;
use super::value::PropertyValue;

/// The accepted JSON shapes, printed in the error of an op that matches none. They are the
/// origin's `{ "set", "value" }` entry plus a `reset`.
const SHAPES: &str =
    r#"{ "set": "<property path>", "value": <json> } or { "reset": "<property path>" }"#;

#[derive(Debug, Clone, PartialEq)]
pub enum EditOp {
    /// Sets the property. The value is checked against the catalog when the batch runs.
    Set {
        property: PropertyRef,
        value: PropertyValue,
    },
    /// Sets the property to the catalog's default; refused (`NO_DEFAULT`) when it has none.
    Reset { property: PropertyRef },
}

impl EditOp {
    pub fn set(property: PropertyRef, value: PropertyValue) -> Self {
        Self::Set { property, value }
    }

    pub fn reset(property: PropertyRef) -> Self {
        Self::Reset { property }
    }

    pub fn property(&self) -> &PropertyRef {
        match self {
            Self::Set { property, .. } | Self::Reset { property } => property,
        }
    }

    /// Decodes one JSON op. The value is decoded by the kind of the property its path names
    /// (`INVALID_TYPE` if the shape is wrong); range and applicability are checked by the batch.
    pub fn from_json(value: &Value) -> Result<Self, EditError> {
        let invalid =
            |message: &str| EditError::new(INVALID_OP, format!("{message}; expected {SHAPES}"));
        let Value::Object(map) = value else {
            return Err(invalid("an op must be an object"));
        };
        if let Some(reset) = map.get("reset") {
            if map.len() != 1 {
                return Err(invalid("a `reset` op has no other key"));
            }
            let Some(path) = reset.as_str() else {
                return Err(invalid("`reset` must be a property path string"));
            };
            return Ok(Self::reset(PropertyRef::parse(path)?));
        }
        let (Some(path), Some(new_value)) = (map.get("set"), map.get("value")) else {
            return Err(invalid("an op needs `set` and `value`, or `reset`"));
        };
        if map.len() != 2 {
            return Err(invalid("a `set` op has only `set` and `value`"));
        }
        let Some(path) = path.as_str() else {
            return Err(invalid("`set` must be a property path string"));
        };
        let property = PropertyRef::parse(path)?;
        let new_value = PropertyValue::from_json(property.spec().kind, new_value)
            .map_err(|error| error.with_path(property.path()))?;
        Ok(Self::set(property, new_value))
    }
}

/// Decodes a JSON array of ops. The error carries the position of the op that failed to decode.
pub fn parse_batch(value: &Value) -> Result<Vec<EditOp>, EditError> {
    let Some(items) = value.as_array() else {
        return Err(EditError::new(
            INVALID_OP,
            "a batch must be an array of ops",
        ));
    };
    items
        .iter()
        .enumerate()
        .map(|(index, item)| EditOp::from_json(item).map_err(|error| error.at_op(index)))
        .collect()
}
