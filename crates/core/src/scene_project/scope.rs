//! What an attached scene covers and what it does not, as data an adapter can print.
//!
//! The lists are the importer's own (`DEFERRED_FIELDS`, `DEFERRED_RULES`); they are not retyped
//! here, so they cannot drift from what the importer really skips.

use serde_json::{Value, json};

use crate::directed_scene::{DEFERRED_FIELDS, DEFERRED_RULES, SCHEMA_VERSION};

/// Deferred fields that sit on a scene definition, and on a node of one.
const DEFINITION_FIELDS: &[&str] = &[
    "contactPlaneId",
    "frameAudit",
    "weather",
    "weatherKeys",
    "lighting",
    "effects",
    "weatherGround",
    "rigs",
    "paths",
];
const NODE_FIELDS: &[&str] = &[
    "surface",
    "normalMap",
    "sceneDepth",
    "relief",
    "rigBind",
    "reveal",
];
const TOP_FIELDS: &[&str] = &["narration", "outline"];

fn present(object: &Value, key: &str) -> bool {
    object.get(key).is_some_and(|value| !value.is_null())
}

fn array_of<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// The deferred families a bundle actually uses, with how many places use them. Those are kept
/// verbatim in the document and round-trip, but nothing validates, evaluates or renders them, so
/// a caller can tell "this scene is plain" from "this scene carries lighting we ignore".
pub fn deferred_in_use(document: &Value) -> Vec<Value> {
    let definitions = array_of(document, "sceneDefinitions");
    let mut counts: Vec<(String, usize)> = Vec::new();
    for key in TOP_FIELDS {
        counts.push(((*key).to_string(), usize::from(present(document, key))));
    }
    for key in DEFINITION_FIELDS {
        counts.push((
            format!("sceneDefinitions[].{key}"),
            definitions
                .iter()
                .filter(|definition| present(definition, key))
                .count(),
        ));
    }
    let nodes: Vec<&Value> = definitions
        .iter()
        .flat_map(|definition| array_of(definition, "nodes"))
        .collect();
    for key in NODE_FIELDS {
        counts.push((
            format!("sceneDefinitions[].nodes[].{key}"),
            nodes.iter().filter(|node| present(node, key)).count(),
        ));
    }
    counts
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .map(|(field, count)| json!({ "field": field, "count": count }))
        .collect()
}

/// The declared boundary of this feature. Returned by `scene.attach` and `scene.capabilities`.
pub fn scope_json() -> Value {
    json!({
        "format": "render-bundle",
        "schema_version": SCHEMA_VERSION,
        "role": "interchange data kept in the project document",
        "validated": [
            "schema subset (BB01/BB02): layers, hierarchy, references, 3D transforms, shots, camera clips",
            "integrity rules the importer implements",
            "structural edit joins (cuts between shots)",
            "temporal gates (layered field of view, continuous-cut pose and velocity jumps)",
            "the effects layer `fx` (F016): catalog `vignette`, output scope, ranges, overlap, envelope; edited by `scene.fx.edit`",
        ],
        "deferred_fields": DEFERRED_FIELDS,
        "deferred_rules": DEFERRED_RULES,
        "not_evaluated": [
            "`scene.evaluate` answers camera and nodes; lighting, weather, effects, rigs and paths have no state (the answer lists them in `unevaluated`)",
            "the families listed in `deferred_in_use` (lighting, weather, rigs...) have no state anywhere",
        ],
        "not_enabled": [
            "render, preview, export and playback of a directed scene",
            "tessel-scene@1 runtime and MediaKind::Scene assets (a bundle is never one)",
            "scene resources: relative asset paths are kept as data and are not resolved, copied or packaged (BB05/BB06)",
            "UI: no panel, tree node or gesture drives scenes yet",
        ],
        "transport": {
            "document": "preserved verbatim, unknown fields included",
            "package": "scenes travel in the document; a scene that names resource files makes the package incomplete, and `unsupported` unless a partial package is allowed (resources are not packaged until BB05/BB06)",
            "project_import": "importing sequences never carries scenes (they belong to the project); the plan says so in its warnings",
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_document_uses_no_deferred_family() {
        let document = json!({
            "sceneDefinitions": [{ "id": "a", "nodes": [{ "id": "n" }] }],
        });
        assert!(deferred_in_use(&document).is_empty());
    }

    #[test]
    fn deferred_families_are_counted_where_they_are_used() {
        let document = json!({
            "narration": { "takes": [] },
            "outline": null,
            "sceneDefinitions": [
                { "id": "a", "lighting": {}, "nodes": [{ "id": "n", "surface": {}, "reveal": {} }, { "id": "m" }] },
                { "id": "b", "lighting": {}, "weather": {}, "nodes": [{ "id": "n", "reveal": {} }] },
            ],
        });
        let found = deferred_in_use(&document);
        let count = |field: &str| {
            found
                .iter()
                .find(|entry| entry["field"] == field)
                .map(|entry| entry["count"].as_u64().unwrap())
        };
        assert_eq!(count("narration"), Some(1));
        assert_eq!(count("outline"), None, "null is absent");
        assert_eq!(count("sceneDefinitions[].lighting"), Some(2));
        assert_eq!(count("sceneDefinitions[].weather"), Some(1));
        assert_eq!(count("sceneDefinitions[].nodes[].reveal"), Some(2));
        assert_eq!(count("sceneDefinitions[].nodes[].surface"), Some(1));
    }
}
