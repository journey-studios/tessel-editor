//! Entity addresses: the stable text a script, a CLI line or (later) a UI uses to name one entity
//! of an attached bundle, and the check that the entity exists.
//!
//! The grammar is the one of the property paths (`scene_edit::PropertyRef`) cut before the
//! property name, so an entity address is always the prefix of the property paths of that entity:
//! `scene/<definition>/node/<node>` is the prefix of `scene/<definition>/node/<node>/position`,
//! and `camera/<clip>` of `camera/<clip>/start/fov`. Every ID is spelled as `encodeURIComponent`
//! does, so an ID holding a `/` cannot be taken for two segments.
//!
//! | Address | Entity |
//! |---|---|
//! | `scene/<definition>` | scene definition (`sceneDefinitions[]`) |
//! | `scene/<definition>/node/<node>` | node (IDs are unique inside a definition only) |
//! | `scene/<definition>/light/<light>` | point light (`lighting.lights[]`, F060) |
//! | `instance/<id>` | scene instance |
//! | `track/<id>` | performance track |
//! | `camera/<id>` | camera clip |
//! | `shot/<id>` | shot |
//! | `asset/<id>` | asset |
//! | `pass/<fragmentId>` | composition pass |
//!
//! Naming caveat: the *attached scene* of a project (`scene.attach --id street`) is a bundle; the
//! `scene/<definition>` of an address is a definition inside that bundle. A request names both:
//! `{"scene": "street", "entities": ["scene/scene-a/node/figure"]}`.

use serde_json::{Value, json};

use crate::directed_scene::{Bundle, Entity, EntityKind};
use crate::scene_edit::PropertyRef;
use crate::state::CommandError;

/// What `encodeURIComponent` leaves as it is, besides ASCII letters and digits.
///
/// WHY a copy of `scene_edit::reference`'s codec and not a call into it: that codec is private to
/// `scene_edit` and only encodes whole property paths. The two must agree, which a test pins by
/// comparing an address with the prefix of the property path of the same entity.
const UNRESERVED: &[u8] = b"-_.!~*'()";

const GRAMMAR: &str = "addresses look like scene/<definition>/node/<node>, camera/<clip>, \
     scene/<definition>/light/<light>, scene/<definition>, instance/<id>, track/<id>, shot/<id>, asset/<id> or pass/<id>";

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

/// Lower-case, stable name of a kind (what the JSON answers print as `kind`).
pub fn kind_label(kind: EntityKind) -> &'static str {
    match kind {
        EntityKind::Asset => "asset",
        EntityKind::SceneDefinition => "scene_definition",
        EntityKind::Node => "node",
        EntityKind::SceneInstance => "scene_instance",
        EntityKind::PerformanceTrack => "performance_track",
        EntityKind::CameraClip => "camera_clip",
        EntityKind::Shot => "shot",
        EntityKind::CompositionPass => "composition_pass",
        EntityKind::Light => "light",
    }
}

/// The canonical address of an entity: what `parse_entity` accepts and every answer prints.
pub fn entity_path(entity: &Entity) -> String {
    let id = encode(&entity.id);
    match entity.kind {
        EntityKind::Node => format!(
            "scene/{}/node/{id}",
            encode(entity.scope.as_deref().unwrap_or_default())
        ),
        EntityKind::Light => format!(
            "scene/{}/light/{id}",
            encode(entity.scope.as_deref().unwrap_or_default())
        ),
        EntityKind::SceneDefinition => format!("scene/{id}"),
        EntityKind::SceneInstance => format!("instance/{id}"),
        EntityKind::PerformanceTrack => format!("track/{id}"),
        EntityKind::CameraClip => format!("camera/{id}"),
        EntityKind::Shot => format!("shot/{id}"),
        EntityKind::Asset => format!("asset/{id}"),
        EntityKind::CompositionPass => format!("pass/{id}"),
    }
}

/// Parses an address. Syntax only: whether the entity exists is `ensure_exists`'s question,
/// asked against a bundle.
pub fn parse_entity(path: &str) -> Result<Entity, CommandError> {
    let invalid = |why: &str| {
        CommandError::new("invalid_entity_path", format!("`{path}` {why}; {GRAMMAR}")).target(path)
    };
    let Some(decoded) = path.split('/').map(decode).collect::<Option<Vec<String>>>() else {
        return Err(invalid("is not valid URI encoding"));
    };
    let parts: Vec<&str> = decoded.iter().map(String::as_str).collect();
    let single = |kind: EntityKind, id: &str| {
        if id.is_empty() {
            return Err(invalid("has an empty ID"));
        }
        Ok(Entity::new(kind, id))
    };
    match parts.as_slice() {
        ["scene", id] => single(EntityKind::SceneDefinition, id),
        ["scene", scene, "node", node] => {
            if scene.is_empty() || node.is_empty() {
                return Err(invalid("has an empty ID"));
            }
            Ok(Entity::node(scene, node))
        }
        ["scene", scene, "light", light] => {
            if scene.is_empty() || light.is_empty() {
                return Err(invalid("has an empty ID"));
            }
            Ok(Entity::light(scene, light))
        }
        ["instance", id] => single(EntityKind::SceneInstance, id),
        ["track", id] => single(EntityKind::PerformanceTrack, id),
        ["camera", id] => single(EntityKind::CameraClip, id),
        ["shot", id] => single(EntityKind::Shot, id),
        ["asset", id] => single(EntityKind::Asset, id),
        ["pass", id] => single(EntityKind::CompositionPass, id),
        _ => Err(invalid("is not an entity address")),
    }
}

/// Whether the bundle holds the entity right now.
pub fn exists(bundle: &Bundle, entity: &Entity) -> bool {
    let id = entity.id.as_str();
    match entity.kind {
        EntityKind::Asset => bundle.asset(id).is_some(),
        EntityKind::SceneDefinition => bundle.scene(id).is_some(),
        EntityKind::Node => entity
            .scope
            .as_deref()
            .and_then(|scope| bundle.scene(scope))
            .is_some_and(|graph| graph.node(id).is_some()),
        // A light whose id is declared twice is not one entity: it cannot be addressed.
        EntityKind::Light => entity
            .scope
            .as_deref()
            .and_then(|scope| bundle.scene(scope))
            .is_some_and(|graph| graph.light(id).is_some()),
        EntityKind::SceneInstance => bundle.instance(id).is_some(),
        EntityKind::PerformanceTrack => {
            bundle.performance_tracks.iter().any(|track| track.id == id)
        }
        EntityKind::CameraClip => bundle.camera_clip(id).is_some(),
        EntityKind::Shot => bundle.shot(id).is_some(),
        EntityKind::CompositionPass => bundle
            .composition_passes
            .iter()
            .any(|pass| pass.fragment_id == id),
    }
}

pub fn ensure_exists(bundle: &Bundle, entity: &Entity) -> Result<(), CommandError> {
    if exists(bundle, entity) {
        return Ok(());
    }
    let path = entity_path(entity);
    Err(CommandError::new(
        "unknown_entity",
        format!("The scene has no {} `{path}`", kind_label(entity.kind)),
    )
    .target(path))
}

/// The entity a catalog property belongs to (the node or the camera clip it edits).
pub fn entity_of_property(property: &PropertyRef) -> Entity {
    match property {
        PropertyRef::Node {
            scene_id, node_id, ..
        } => Entity::node(scene_id, node_id),
        PropertyRef::Camera { clip_id, .. } => Entity::new(EntityKind::CameraClip, clip_id),
        PropertyRef::Light {
            scene_id, light_id, ..
        } => Entity::light(scene_id, light_id),
    }
}

/// `{ "path", "kind", "id", "scope"? }`: the address plus its parts, so a consumer need not
/// parse the text back.
pub fn entity_json(entity: &Entity) -> Value {
    let mut value = json!({
        "path": entity_path(entity),
        "kind": kind_label(entity.kind),
        "id": entity.id,
    });
    if let Some(scope) = &entity.scope {
        value["scope"] = json!(scope);
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene_edit::{CameraProperty, NodeProperty};

    #[test]
    fn an_address_round_trips_for_every_kind_even_with_awkward_ids() {
        let entities = [
            Entity::new(EntityKind::Asset, "a/b"),
            Entity::new(EntityKind::SceneDefinition, "scene a"),
            Entity::node("def/1", "hero plane%"),
            Entity::light("def/1", "lamp 1/%"),
            Entity::new(EntityKind::SceneInstance, "inst"),
            Entity::new(EntityKind::PerformanceTrack, "perf-1"),
            Entity::new(EntityKind::CameraClip, "é"),
            Entity::new(EntityKind::Shot, "shot.1"),
            Entity::new(EntityKind::CompositionPass, "frag"),
        ];
        for entity in entities {
            let path = entity_path(&entity);
            assert_eq!(parse_entity(&path).unwrap(), entity, "{path}");
        }
    }

    #[test]
    fn an_address_is_the_prefix_of_the_property_paths_of_the_same_entity() {
        let node = PropertyRef::node("def/1", "hero plane", NodeProperty::Position);
        let node_entity = entity_of_property(&node);
        assert!(
            node.path()
                .starts_with(&format!("{}/", entity_path(&node_entity)))
        );
        let clip = PropertyRef::camera("cam é", CameraProperty::StartFov);
        let clip_entity = entity_of_property(&clip);
        assert!(
            clip.path()
                .starts_with(&format!("{}/", entity_path(&clip_entity)))
        );
    }

    #[test]
    fn malformed_addresses_are_typed_and_name_the_input() {
        for raw in [
            "",
            "node/x",
            "scene",
            "scene//node/x",
            "scene/a/node/",
            "scene/a/node/x/position",
            "camera/%zz",
            "camera/%E9",
            "shot/",
        ] {
            let error = parse_entity(raw).unwrap_err();
            assert_eq!(error.code, "invalid_entity_path", "`{raw}`");
            assert_eq!(error.target.as_deref(), Some(raw));
        }
    }
}
