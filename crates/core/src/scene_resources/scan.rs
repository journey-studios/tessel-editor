//! Reads the sections of the bundle document that the importer keeps raw (`DEFERRED_FIELDS`) and
//! that can name files: the data maps of nodes and the narration takes.
//!
//! WHY the document and not the typed `Bundle`: the typed view only keeps the `assetId` of
//! `surface` and `normalMap`, and drops `sceneDepth`, `relief` and `narration` entirely. Those
//! sections are still part of what the bundle depends on, so they are read here, defensively
//! (the importer did not validate their shape) and only for the fields that name a file.

use serde_json::Value;

use super::model::{MAX_LISTED_FILES, NamedFile, UninterpretedSection, UseRole};

/// `(section, [(field holding an assetRef, role of the file it names)])`.
const DATA_MAPS: [(&str, &[(&str, UseRole)]); 4] = [
    ("surface", &[("assetRef", UseRole::SurfaceMap)]),
    ("normalMap", &[("assetRef", UseRole::NormalMap)]),
    (
        "sceneDepth",
        &[
            ("depthAssetRef", UseRole::SceneDepthGrid),
            ("imageAssetRef", UseRole::SceneDepthImage),
        ],
    ),
    ("relief", &[("depthAssetRef", UseRole::ReliefGrid)]),
];

/// An `assetRef` found inside a preserved section, as written there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RawReference {
    pub role: UseRole,
    pub pointer: String,
    pub scene_id: String,
    pub node_id: String,
    pub asset_id: String,
    pub revision_id: Option<String>,
    pub content_hash: Option<String>,
    pub relative_path: Option<String>,
}

#[derive(Debug, Default)]
pub(super) struct Scan {
    pub references: Vec<RawReference>,
    pub sections: Vec<UninterpretedSection>,
}

fn items(value: Option<&Value>) -> &[Value] {
    value
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_string)
}

/// Present and not `null`: the origin's schema would refuse `null`, and it carries no file.
fn declared<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.get(key).filter(|section| !section.is_null())
}

pub(super) fn scan(document: &Value) -> Scan {
    let mut scan = Scan::default();
    for (scene_index, definition) in items(document.get("sceneDefinitions")).iter().enumerate() {
        let Some(scene_id) = text(definition, "id") else {
            continue;
        };
        for (node_index, node) in items(definition.get("nodes")).iter().enumerate() {
            let Some(node_id) = text(node, "id") else {
                continue;
            };
            let base = format!("/sceneDefinitions/{scene_index}/nodes/{node_index}");
            for (section, fields) in DATA_MAPS {
                let Some(value) = declared(node, section) else {
                    continue;
                };
                let pointer = format!("{base}/{section}");
                let mut record = UninterpretedSection {
                    pointer: pointer.clone(),
                    name: section,
                    scene_id: Some(scene_id.clone()),
                    node_id: Some(node_id.clone()),
                    asset_ids: Vec::new(),
                    files: Vec::new(),
                    truncated: false,
                };
                for &(field, role) in fields {
                    let Some(reference) = declared(value, field) else {
                        continue;
                    };
                    let Some(asset_id) = text(reference, "assetId") else {
                        continue;
                    };
                    record.asset_ids.push(asset_id.clone());
                    scan.references.push(RawReference {
                        role,
                        pointer: format!("{pointer}/{field}"),
                        scene_id: scene_id.clone(),
                        node_id: node_id.clone(),
                        asset_id,
                        revision_id: text(reference, "revisionId"),
                        content_hash: text(reference, "contentHash"),
                        relative_path: text(reference, "relativePath"),
                    });
                }
                scan.sections.push(record);
            }
        }
    }
    if let Some(narration) = declared(document, "narration") {
        scan.sections.push(narration_section(narration));
    }
    scan
}

/// The takes of the narration (`tracks[].clips[].audio.relativePath`), listed up to
/// `MAX_LISTED_FILES`. The section is not interpreted (BB09/M5), so the paths are as declared.
fn narration_section(narration: &Value) -> UninterpretedSection {
    let mut record = UninterpretedSection {
        pointer: "/narration".to_string(),
        name: "narration",
        scene_id: None,
        node_id: None,
        asset_ids: Vec::new(),
        files: Vec::new(),
        truncated: false,
    };
    for (track_index, track) in items(narration.get("tracks")).iter().enumerate() {
        for (clip_index, clip) in items(track.get("clips")).iter().enumerate() {
            let Some(path) = clip
                .get("audio")
                .and_then(|audio| text(audio, "relativePath"))
            else {
                continue;
            };
            if record.files.len() >= MAX_LISTED_FILES {
                record.truncated = true;
                return record;
            }
            record.files.push(NamedFile {
                pointer: format!(
                    "/narration/tracks/{track_index}/clips/{clip_index}/audio/relativePath"
                ),
                relative_path: path,
            });
        }
    }
    record
}
