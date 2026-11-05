//! The queryable tree of the entities of one bundle: what a scene panel lists and what a script
//! reads to find an address.
//!
//! The shape follows the data, not a layout:
//!
//! * the root is the bundle; its children are the scene definitions, scene instances, camera
//!   clips, shots, assets and composition passes, in that order and in document order inside
//!   each kind;
//! * a definition has its root node as its only child and the node hierarchy hangs from it
//!   (`SceneGraph::children`, so a node on a socket sits under the socket's owner);
//! * an instance has its performance tracks.
//!
//! Nothing here validates or evaluates; it reads the importer's typed view. Every entry carries
//! its address (`path`), whether it is selected, and for nodes and camera clips the catalog
//! property paths `scene.edit` accepts for it, so one answer is enough to build an edit.

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use crate::directed_scene::{Bundle, DirectedScene, Entity, EntityKind};
use crate::scene_edit::{CameraProperty, NodeProperty, PropertyRef};
use crate::state::CommandError;

use super::address::{ensure_exists, entity_path, kind_label, parse_entity};

/// Deepest hierarchy one answer expands. WHY a cap: a node chain can be as long as the importer's
/// item limit (100 000), and building JSON recursively that deep would overflow the stack of
/// whoever asks. Entries below the cap say so (`truncated`) and still give `children_count`;
/// ask again with `root` to go further.
pub const MAX_TREE_DEPTH: usize = 64;

struct Context<'a> {
    bundle: &'a Bundle,
    selected: HashSet<&'a Entity>,
}

/// Children of an entity, in the order the tree lists them. Never the bundle root.
fn children_of(bundle: &Bundle, entity: &Entity) -> Vec<Entity> {
    match entity.kind {
        EntityKind::SceneDefinition => bundle
            .scene(&entity.id)
            .filter(|graph| graph.node(graph.root_id()).is_some())
            .map(|graph| vec![Entity::node(graph.id(), graph.root_id())])
            .unwrap_or_default(),
        EntityKind::Node => {
            let Some(graph) = entity
                .scope
                .as_deref()
                .and_then(|scope| bundle.scene(scope))
            else {
                return Vec::new();
            };
            graph
                .children(&entity.id)
                .into_iter()
                .map(|node| Entity::node(graph.id(), &node.id))
                .collect()
        }
        EntityKind::SceneInstance => bundle
            .performance_tracks
            .iter()
            .filter(|track| track.instance_id == entity.id)
            .map(|track| Entity::new(EntityKind::PerformanceTrack, &track.id))
            .collect(),
        _ => Vec::new(),
    }
}

fn top_level(bundle: &Bundle) -> Vec<Entity> {
    let definitions = bundle
        .scenes
        .iter()
        .map(|graph| Entity::new(EntityKind::SceneDefinition, graph.id()));
    let instances = bundle
        .instances
        .iter()
        .map(|instance| Entity::new(EntityKind::SceneInstance, &instance.id));
    let clips = bundle
        .camera_clips
        .iter()
        .map(|clip| Entity::new(EntityKind::CameraClip, &clip.id));
    let shots = bundle
        .shots
        .iter()
        .map(|shot| Entity::new(EntityKind::Shot, &shot.id));
    let assets = bundle
        .assets
        .iter()
        .map(|asset| Entity::new(EntityKind::Asset, &asset.asset_id));
    let passes = bundle
        .composition_passes
        .iter()
        .map(|pass| Entity::new(EntityKind::CompositionPass, &pass.fragment_id));
    definitions
        .chain(instances)
        .chain(clips)
        .chain(shots)
        .chain(assets)
        .chain(passes)
        .collect()
}

/// Property paths `scene.edit` accepts for the entity (empty for kinds the catalog does not edit).
fn editable_paths(bundle: &Bundle, entity: &Entity) -> Vec<String> {
    match entity.kind {
        EntityKind::Node => {
            let Some(node) = entity
                .scope
                .as_deref()
                .and_then(|scope| bundle.scene(scope))
                .and_then(|graph| graph.node(&entity.id))
            else {
                return Vec::new();
            };
            let scope = entity.scope.as_deref().unwrap_or_default();
            NodeProperty::ALL
                .into_iter()
                .filter(|property| property.spec().applies_to.accepts_node(node.kind))
                .map(|property| PropertyRef::node(scope, &entity.id, property).path())
                .collect()
        }
        EntityKind::CameraClip => CameraProperty::ALL
            .into_iter()
            .map(|property| PropertyRef::camera(&entity.id, property).path())
            .collect(),
        _ => Vec::new(),
    }
}

/// What is specific to each kind, as plain fields.
fn details(bundle: &Bundle, entity: &Entity, entry: &mut Map<String, Value>) {
    let id = entity.id.as_str();
    match entity.kind {
        EntityKind::SceneDefinition => {
            if let Some(graph) = bundle.scene(id) {
                entry.insert(
                    "root_node".into(),
                    json!(entity_path(&Entity::node(id, graph.root_id()))),
                );
                // Sections the scene declares and nothing evaluates (see `scene.evaluate`).
                entry.insert("unevaluated".into(), json!(graph.unevaluated_sections()));
            }
        }
        EntityKind::Node => {
            let scope = entity.scope.as_deref().unwrap_or_default();
            if let Some(graph) = bundle.scene(scope)
                && let Some(node) = graph.node(id)
            {
                entry.insert("node_kind".into(), json!(node.kind.as_str()));
                let parent = graph
                    .parent(id)
                    .map(|parent| entity_path(&Entity::node(scope, parent.owner())));
                entry.insert("parent".into(), json!(parent));
            }
        }
        EntityKind::SceneInstance => {
            if let Some(instance) = bundle.instance(id) {
                entry.insert(
                    "definition".into(),
                    json!(entity_path(&Entity::new(
                        EntityKind::SceneDefinition,
                        &instance.scene_definition_id
                    ))),
                );
                entry.insert("duration_frames".into(), json!(instance.duration_frames));
                entry.insert("edit_locked".into(), json!(instance.edit_locked));
            }
        }
        EntityKind::PerformanceTrack => {
            if let Some(track) = bundle
                .performance_tracks
                .iter()
                .find(|track| track.id == id)
            {
                entry.insert(
                    "instance".into(),
                    json!(entity_path(&Entity::new(
                        EntityKind::SceneInstance,
                        &track.instance_id
                    ))),
                );
                entry.insert("channels".into(), json!(track.channels.len()));
            }
        }
        EntityKind::CameraClip => {
            entry.insert("frames".into(), json!(bundle.camera_clip_frames(id)));
        }
        EntityKind::Shot => {
            if let Some(shot) = bundle.shot(id) {
                entry.insert(
                    "instance".into(),
                    json!(entity_path(&Entity::new(
                        EntityKind::SceneInstance,
                        &shot.scene_instance_id
                    ))),
                );
                entry.insert(
                    "camera".into(),
                    json!(entity_path(&Entity::new(
                        EntityKind::CameraClip,
                        &shot.camera_clip_id
                    ))),
                );
                entry.insert("start_frame".into(), json!(shot.start_frame));
                entry.insert(
                    "end_frame_exclusive".into(),
                    json!(shot.end_frame_exclusive),
                );
            }
        }
        EntityKind::Asset => {
            if let Some(asset) = bundle.asset(id) {
                // Declared by the bundle, never resolved here (BB06): data, not a file.
                entry.insert("relative_path".into(), json!(asset.relative_path));
                entry.insert("format".into(), json!(asset.format.as_str()));
            }
        }
        EntityKind::CompositionPass => {
            if let Some(pass) = bundle
                .composition_passes
                .iter()
                .find(|pass| pass.fragment_id == id)
            {
                entry.insert("node_id".into(), json!(pass.node_id));
                entry.insert("order".into(), json!(pass.order));
            }
        }
    }
}

fn entry_json(context: &Context<'_>, entity: &Entity, depth_left: usize) -> Value {
    let children = children_of(context.bundle, entity);
    let mut entry = Map::new();
    entry.insert("path".into(), json!(entity_path(entity)));
    entry.insert("kind".into(), json!(kind_label(entity.kind)));
    entry.insert("id".into(), json!(entity.id));
    if let Some(scope) = &entity.scope {
        entry.insert("scope".into(), json!(scope));
    }
    entry.insert("selected".into(), json!(context.selected.contains(entity)));
    details(context.bundle, entity, &mut entry);
    let editable = editable_paths(context.bundle, entity);
    if !editable.is_empty() {
        entry.insert("editable".into(), json!(editable));
    }
    entry.insert("children_count".into(), json!(children.len()));
    if depth_left == 0 {
        if !children.is_empty() {
            entry.insert("truncated".into(), json!(true));
        }
        return Value::Object(entry);
    }
    let expanded: Vec<Value> = children
        .iter()
        .map(|child| entry_json(context, child, depth_left - 1))
        .collect();
    entry.insert("children".into(), Value::Array(expanded));
    Value::Object(entry)
}

/// The tree of `scene` (the whole bundle, or the subtree at `root`), `depth` levels below the
/// starting entry (default and maximum [`MAX_TREE_DEPTH`]). `selected` are the entities of this
/// attached scene that the selection holds.
pub fn tree_json(
    scene: &DirectedScene,
    root: Option<&str>,
    depth: Option<usize>,
    selected: &[&Entity],
) -> Result<Value, CommandError> {
    let depth = depth.unwrap_or(MAX_TREE_DEPTH);
    if depth == 0 || depth > MAX_TREE_DEPTH {
        return Err(CommandError::invalid_value(
            format!("`depth` is between 1 and {MAX_TREE_DEPTH}"),
            json!({ "min": 1, "max": MAX_TREE_DEPTH }),
        ));
    }
    let bundle = scene.bundle();
    let context = Context {
        bundle,
        selected: selected.iter().copied().collect(),
    };
    match root {
        Some(raw) => {
            let entity = parse_entity(raw)?;
            ensure_exists(bundle, &entity)?;
            // `depth` counts levels below the starting entry, so an entry plus its children is 1.
            Ok(entry_json(&context, &entity, depth))
        }
        None => {
            let top = top_level(bundle);
            let children: Vec<Value> = top
                .iter()
                .map(|entity| entry_json(&context, entity, depth - 1))
                .collect();
            Ok(json!({
                "path": "",
                "kind": "bundle",
                "id": bundle.bundle_id,
                "revision_id": bundle.revision_id,
                "children_count": children.len(),
                "children": children,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_depth_outside_the_bound_is_a_typed_error() {
        // No scene is needed to refuse the argument: the check runs before the bundle is read.
        let text = include_str!(
            "../../../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json"
        );
        let scene = DirectedScene::import_str(text).unwrap();
        for depth in [0, MAX_TREE_DEPTH + 1] {
            let error = tree_json(&scene, None, Some(depth), &[]).unwrap_err();
            assert_eq!(error.code, "invalid_value");
        }
        assert!(tree_json(&scene, None, Some(MAX_TREE_DEPTH), &[]).is_ok());
    }

    #[test]
    fn the_tree_lists_every_entity_once_under_the_bundle() {
        let text = include_str!(
            "../../../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json"
        );
        let scene = DirectedScene::import_str(text).unwrap();
        let tree = tree_json(&scene, None, None, &[]).unwrap();
        let mut paths = Vec::new();
        let mut stack = vec![&tree];
        while let Some(entry) = stack.pop() {
            if let Some(path) = entry["path"].as_str().filter(|path| !path.is_empty()) {
                paths.push(path.to_string());
            }
            if let Some(children) = entry["children"].as_array() {
                stack.extend(children);
            }
        }
        let unique: HashSet<&String> = paths.iter().collect();
        assert_eq!(unique.len(), paths.len(), "{paths:?}");
        for expected in [
            "scene/scene-a",
            "scene/scene-a/node/root",
            "scene/scene-a/node/figure",
            "instance/inst-a",
            "camera/cam-main",
            "shot/shot-1",
            "asset/figure",
        ] {
            assert!(paths.iter().any(|path| path == expected), "{expected}");
        }
    }

    #[test]
    fn a_shallow_answer_marks_what_it_left_out() {
        let text = include_str!(
            "../../../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json"
        );
        let scene = DirectedScene::import_str(text).unwrap();
        let tree = tree_json(&scene, Some("scene/scene-a"), Some(1), &[]).unwrap();
        let root = &tree["children"][0];
        assert_eq!(root["path"], "scene/scene-a/node/root");
        assert_eq!(root["truncated"], true);
        assert_eq!(root["children_count"], 2);
        assert!(root.get("children").is_none());
    }
}
