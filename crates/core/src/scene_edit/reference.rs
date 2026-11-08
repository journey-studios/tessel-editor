//! Property paths: the stable, URI-encoded names of catalog properties, and the typed reference
//! behind them.
//!
//! The grammar is the origin's (`properties/property-paths.ts`): `scene/<sceneId>/...` and
//! `camera/<clipId>/...`, every ID spelled as `encodeURIComponent` does, so an ID holding a `/`
//! cannot be mistaken for two segments. Node IDs are unique only inside a scene, so a node path
//! always carries its scene: `scene/<sceneId>/node/<nodeId>/<property>`.

use std::fmt;

use super::catalog::{CameraProperty, NodeProperty, PropertySpec};
use super::error::{EditError, UNKNOWN_PATH};

/// What `encodeURIComponent` leaves as it is, besides ASCII letters and digits.
const UNRESERVED: &[u8] = b"-_.!~*'()";

/// `encodeURIComponent`: UTF-8 bytes outside the unreserved set become `%XX` (uppercase hex).
fn encode(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    for byte in id.bytes() {
        if byte.is_ascii_alphanumeric() || UNRESERVED.contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// `decodeURIComponent`; `None` for a truncated or non-hex escape, or bytes that are not UTF-8.
fn decode(segment: &str) -> Option<String> {
    let bytes = segment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != b'%' {
            out.push(bytes[at]);
            at += 1;
            continue;
        }
        let hex = segment.get(at + 1..at + 3)?;
        // `from_str_radix` would take `+1`; an escape is exactly two hex digits.
        if !hex.bytes().all(|digit| digit.is_ascii_hexdigit()) {
            return None;
        }
        out.push(u8::from_str_radix(hex, 16).ok()?);
        at += 3;
    }
    String::from_utf8(out).ok()
}

/// A property of one node or camera clip, by ID. Nothing here depends on a selection.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PropertyRef {
    Node {
        scene_id: String,
        node_id: String,
        property: NodeProperty,
    },
    Camera {
        clip_id: String,
        property: CameraProperty,
    },
}

impl PropertyRef {
    pub fn node(scene_id: &str, node_id: &str, property: NodeProperty) -> Self {
        Self::Node {
            scene_id: scene_id.to_string(),
            node_id: node_id.to_string(),
            property,
        }
    }

    pub fn camera(clip_id: &str, property: CameraProperty) -> Self {
        Self::Camera {
            clip_id: clip_id.to_string(),
            property,
        }
    }

    pub fn spec(&self) -> &'static PropertySpec {
        match self {
            Self::Node { property, .. } => property.spec(),
            Self::Camera { property, .. } => property.spec(),
        }
    }

    /// The canonical path: what `parse` accepts and what errors and summaries print.
    pub fn path(&self) -> String {
        match self {
            Self::Node {
                scene_id,
                node_id,
                property,
            } => format!(
                "scene/{}/node/{}/{}",
                encode(scene_id),
                encode(node_id),
                property.spec().name
            ),
            Self::Camera { clip_id, property } => {
                format!("camera/{}/{}", encode(clip_id), property.spec().name)
            }
        }
    }

    /// Parses a path. Syntax only: whether the scene, node or clip exists is checked against a
    /// scene when the op runs. A literal character in an ID segment is accepted as well as its
    /// `%XX` form, as the origin's `splitPath` does; `path()` always prints the encoded one.
    pub fn parse(path: &str) -> Result<Self, EditError> {
        let unknown = |message: String| EditError::new(UNKNOWN_PATH, message).with_path(path);
        let Some(decoded) = path.split('/').map(decode).collect::<Option<Vec<String>>>() else {
            return Err(unknown(format!("`{path}` is not valid URI encoding")));
        };
        let parts: Vec<&str> = decoded.iter().map(String::as_str).collect();
        // A property name is one segment (`position`) or two (`position/x`, `start/fov`). A single
        // segment that decodes to something holding a `/` is an ID-style escape, not a name.
        let name_of = |segments: &[&str]| -> Option<String> {
            (segments.len() <= 2 && segments.iter().all(|part| !part.contains('/')))
                .then(|| segments.join("/"))
        };
        match parts.as_slice() {
            ["scene", scene_id, "node", node_id, rest @ ..]
                if !scene_id.is_empty() && !node_id.is_empty() && (1..=2).contains(&rest.len()) =>
            {
                let name = name_of(rest).unwrap_or_default();
                let Some(property) = NodeProperty::from_name(&name) else {
                    return Err(unknown(format!(
                        "no node property `{name}`; known: {}",
                        known_names(NodeProperty::ALL.map(|property| property.spec().name))
                    )));
                };
                Ok(Self::node(scene_id, node_id, property))
            }
            ["camera", clip_id, "start", rest @ ..]
                if !clip_id.is_empty() && (1..=2).contains(&rest.len()) =>
            {
                let name = name_of(rest).unwrap_or_default();
                let Some(property) = CameraProperty::from_start_name(&name) else {
                    return Err(unknown(format!(
                        "no camera property `start/{name}`; known: {}",
                        known_names(CameraProperty::ALL.map(|property| property.spec().name))
                    )));
                };
                Ok(Self::camera(clip_id, property))
            }
            _ => Err(unknown(format!(
                "`{path}` is not a property path; paths look like \
                 scene/<sceneId>/node/<nodeId>/position, scene/<sceneId>/node/<nodeId>/position/x \
                 or camera/<clipId>/start/fov"
            ))),
        }
    }
}

fn known_names<const N: usize>(names: [&str; N]) -> String {
    names.join(", ")
}

impl fmt::Display for PropertyRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.path())
    }
}
