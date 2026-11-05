//! The reference graph of a bundle: which entity points at which, and the paint order of planes.

use std::collections::HashMap;

use super::model::{Bundle, Node, NodeKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntityKind {
    Asset,
    SceneDefinition,
    /// Node ids are unique inside a scene definition only; `Entity::scope` names it.
    Node,
    SceneInstance,
    PerformanceTrack,
    CameraClip,
    Shot,
    CompositionPass,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Entity {
    pub kind: EntityKind,
    /// The scene definition id for a `Node`, `None` for every other kind.
    pub scope: Option<String>,
    pub id: String,
}

impl Entity {
    pub fn new(kind: EntityKind, id: &str) -> Self {
        Self {
            kind,
            scope: None,
            id: id.to_string(),
        }
    }

    pub fn node(scene_id: &str, id: &str) -> Self {
        Self {
            kind: EntityKind::Node,
            scope: Some(scene_id.to_string()),
            id: id.to_string(),
        }
    }
}

/// `from` points at `to` through `field` (the bundle field that holds the id).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub from: Entity,
    pub to: Entity,
    pub field: &'static str,
}

impl Bundle {
    /// Every reference between entities, in document order. After a successful import all of
    /// them resolve to an existing entity.
    pub fn references(&self) -> Vec<Reference> {
        let mut out = Vec::new();
        let mut push = |from: Entity, to: Entity, field: &'static str| {
            out.push(Reference { from, to, field });
        };

        for scene in &self.scenes {
            for node in scene.nodes() {
                let from = Entity::node(scene.id(), &node.id);
                if let Some(asset) = &node.asset {
                    push(
                        from.clone(),
                        Entity::new(EntityKind::Asset, &asset.asset_id),
                        "assetRef.assetId",
                    );
                }
                if let Some(parent) = &node.parent_id {
                    push(from.clone(), Entity::node(scene.id(), parent), "parentId");
                }
                if let Some(attachment) = &node.attach_to_socket {
                    push(
                        from,
                        Entity::node(scene.id(), &attachment.owner),
                        "attachToSocket.parentNodeId",
                    );
                }
            }
        }
        for instance in &self.instances {
            let from = Entity::new(EntityKind::SceneInstance, &instance.id);
            push(
                from.clone(),
                Entity::new(EntityKind::SceneDefinition, &instance.scene_definition_id),
                "sceneDefinitionId",
            );
            for track in &instance.performance_track_ids {
                push(
                    from.clone(),
                    Entity::new(EntityKind::PerformanceTrack, track),
                    "performanceTrackIds",
                );
            }
        }
        for track in &self.performance_tracks {
            let from = Entity::new(EntityKind::PerformanceTrack, &track.id);
            push(
                from.clone(),
                Entity::new(EntityKind::SceneInstance, &track.instance_id),
                "instanceId",
            );
            let scene = self.scene_of_instance(&track.instance_id);
            for channel in &track.channels {
                if let Some(scene) = scene {
                    push(
                        from.clone(),
                        Entity::node(scene.id(), &channel.node_id),
                        "channels.nodeId",
                    );
                }
            }
        }
        for shot in &self.shots {
            let from = Entity::new(EntityKind::Shot, &shot.id);
            push(
                from.clone(),
                Entity::new(EntityKind::SceneInstance, &shot.scene_instance_id),
                "sceneInstanceId",
            );
            push(
                from,
                Entity::new(EntityKind::CameraClip, &shot.camera_clip_id),
                "cameraClipId",
            );
        }
        // Passes are global to the bundle: the target is every scene that has a node of that id.
        for pass in &self.composition_passes {
            for scene in &self.scenes {
                if scene.node(&pass.node_id).is_some() {
                    push(
                        Entity::new(EntityKind::CompositionPass, &pass.fragment_id),
                        Entity::node(scene.id(), &pass.node_id),
                        "nodeId",
                    );
                }
            }
        }
        out
    }

    /// Entities that point at `target`: what would break if it were removed or renamed.
    pub fn referrers_of(&self, target: &Entity) -> Vec<Reference> {
        self.references()
            .into_iter()
            .filter(|reference| &reference.to == target)
            .collect()
    }

    /// Image planes of a scene from back to front, by the order the runtime draws them: a
    /// composition pass overrides the node's own `renderOrder` (`three-scene-runtime.ts`), and
    /// the default is 0. Ties keep document order.
    ///
    /// Limit: this is the declared order only. Where two planes share an order, the origin's
    /// renderer breaks the tie by depth to the camera (Three.js), which needs the camera
    /// evaluated (BB03) and is not modeled here.
    pub fn paint_order(&self, scene_id: &str) -> Option<Vec<&Node>> {
        let scene = self.scene(scene_id)?;
        let pass_order: HashMap<&str, i64> = self
            .composition_passes
            .iter()
            .map(|pass| (pass.node_id.as_str(), pass.order))
            .collect();
        let mut planes: Vec<&Node> = scene
            .nodes()
            .iter()
            .filter(|node| node.kind == NodeKind::ImagePlane)
            .collect();
        planes.sort_by_key(|node| {
            pass_order
                .get(node.id.as_str())
                .copied()
                .or(node.render_order)
                .unwrap_or(0)
        });
        Some(planes)
    }
}
