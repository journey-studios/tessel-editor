//! The state of an instance's nodes at a frame of the instance: authored transform, plus what the
//! performance channels write, composed to local and world matrices through the hierarchy and the
//! sockets (`evaluate-edit-frame.ts`: `applyOverridesToNode`; `scene-graph.ts`:
//! `computeWorldMatrices`).

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::directed_scene::{
    Bundle, ChannelProperty, Mat4, Quat, SceneGraph, SceneInstance, TemporalError, Vec3,
};

use super::channels::{NodeOverride, PerformancePlan};

/// One node at one frame. Everything a renderer needs to place it, resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeState {
    pub node_id: String,
    pub local_position: Vec3,
    /// XYZ Euler degrees (what the channels add to); the matrices are built from this, not from
    /// the authored quaternion, exactly as the origin does.
    pub local_rotation_deg: Vec3,
    pub local_scale: Vec3,
    pub render_order: i64,
    /// Position, rotation and scale composed (`Matrix4.compose`), relative to the parent.
    pub local_matrix: Mat4,
    /// The node in scene space: every ancestor and socket applied.
    pub world_matrix: Mat4,
}

impl NodeState {
    fn is_local_finite(&self) -> bool {
        [
            self.local_position,
            self.local_rotation_deg,
            self.local_scale,
        ]
        .into_iter()
        .flat_map(|v| [v.x, v.y, v.z])
        .all(f64::is_finite)
            && self.local_matrix.is_finite()
    }

    pub fn to_json(&self) -> Value {
        json!({
            "nodeId": self.node_id,
            "localPosition": self.local_position.to_json(),
            "localRotation": self.local_rotation_deg.to_json(),
            "localScale": self.local_scale.to_json(),
            "renderOrder": self.render_order,
            "worldMatrix": self.world_matrix.0,
        })
    }
}

impl NodeOverride {
    pub fn to_json(&self) -> Value {
        json!({
            "nodeId": self.node_id,
            "property": self.property.as_str(),
            "value": self.value.to_json(),
        })
    }
}

/// All the nodes of an instance's scene at one instance frame, queryable by id and serializable
/// for a renderer. It owns its data: it outlives the bundle borrow it was computed from.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeFrame {
    instance_id: String,
    scene_definition_id: String,
    instance_frame: u64,
    overrides: Vec<NodeOverride>,
    /// Document order of the scene definition (the origin's `nodes` order).
    nodes: Vec<NodeState>,
    index: HashMap<String, usize>,
}

impl NodeFrame {
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    pub fn scene_definition_id(&self) -> &str {
        &self.scene_definition_id
    }

    pub fn instance_frame(&self) -> u64 {
        self.instance_frame
    }

    /// Every node, in the scene definition's document order.
    pub fn nodes(&self) -> &[NodeState] {
        &self.nodes
    }

    pub fn node(&self, node_id: &str) -> Option<&NodeState> {
        self.index.get(node_id).map(|&i| &self.nodes[i])
    }

    pub fn world_matrix(&self, node_id: &str) -> Option<&Mat4> {
        self.node(node_id).map(|node| &node.world_matrix)
    }

    /// What the channels wrote, as the origin lists it (`performanceOverrides`): one entry per
    /// channel that applies, including the ones whose value is zero at this frame.
    pub fn overrides(&self) -> &[NodeOverride] {
        &self.overrides
    }

    /// The value a channel writes to `node_id`'s `property` at this frame, if any does.
    pub fn override_of(&self, node_id: &str, property: ChannelProperty) -> Option<Vec3> {
        self.overrides
            .iter()
            .find(|entry| entry.node_id == node_id && entry.property == property)
            .map(|entry| entry.value)
    }

    /// Stable, camelCase JSON: the origin's `nodes` and `performanceOverrides` of
    /// `ResolvedFrameState`, with each node's `worldMatrix` (column-major, 16 numbers) in place
    /// of the origin's separate `worldMatrices` map.
    pub fn to_json(&self) -> Value {
        json!({
            "instanceId": self.instance_id,
            "sceneDefinitionId": self.scene_definition_id,
            "instanceFrame": self.instance_frame,
            "nodes": self.nodes.iter().map(NodeState::to_json).collect::<Vec<_>>(),
            "performanceOverrides": self.overrides.iter().map(NodeOverride::to_json).collect::<Vec<_>>(),
        })
    }
}

/// The authored transform of a node with its rotation already in the Euler form the channels add
/// to. Computed once per timeline: the quaternion → Euler step is the same at every frame.
#[derive(Debug, Clone, Copy)]
struct BaseNode {
    position: Vec3,
    rotation_deg: Vec3,
    scale: Vec3,
    render_order: i64,
}

/// The nodes of one instance laid out for evaluation. Build it once to ask for many frames, in any
/// order: each answer depends only on the instance frame, never on the frames asked before.
#[derive(Debug, Clone)]
pub struct NodeTimeline<'a> {
    instance: &'a SceneInstance,
    scene: &'a SceneGraph,
    plan: PerformancePlan<'a>,
    base: Vec<BaseNode>,
}

impl<'a> NodeTimeline<'a> {
    pub(in crate::directed_scene) fn new(
        bundle: &'a Bundle,
        instance: &'a SceneInstance,
    ) -> Result<Self, TemporalError> {
        let scene = bundle.scene(&instance.scene_definition_id).ok_or_else(|| {
            TemporalError::Unresolved {
                kind: "scene definition",
                id: instance.scene_definition_id.clone(),
            }
        })?;
        let plan = PerformancePlan::new(bundle, instance)?;
        let base = scene
            .nodes()
            .iter()
            .map(|node| BaseNode {
                position: node.transform.position,
                rotation_deg: node.transform.rotation.to_euler_xyz_degrees(),
                scale: node.transform.scale,
                render_order: node.render_order.unwrap_or(0),
            })
            .collect();
        Ok(Self {
            instance,
            scene,
            plan,
            base,
        })
    }

    pub fn instance_id(&self) -> &str {
        &self.instance.id
    }

    pub fn plan(&self) -> &PerformancePlan<'a> {
        &self.plan
    }

    /// The nodes at `instance_frame`, which must be inside the instance (`0..durationFrames`,
    /// like the origin's `evaluateEditFrame`: it never evaluates past the instance's end).
    pub fn state_at(&self, instance_frame: u64) -> Result<NodeFrame, TemporalError> {
        if instance_frame >= self.instance.duration_frames {
            return Err(TemporalError::InstanceFrameOutOfRange {
                instance_id: self.instance.id.clone(),
                frame: instance_frame,
                duration_frames: self.instance.duration_frames,
            });
        }
        let overrides = self.plan.overrides_at(instance_frame);
        let mut by_node: HashMap<&str, Vec<&NodeOverride>> = HashMap::new();
        for entry in &overrides {
            by_node.entry(&entry.node_id).or_default().push(entry);
        }

        let mut states: Vec<NodeState> = self
            .scene
            .nodes()
            .iter()
            .zip(&self.base)
            .map(|(node, base)| {
                let (mut position, mut rotation, mut scale) =
                    (base.position, base.rotation_deg, base.scale);
                for entry in by_node.get(node.id.as_str()).into_iter().flatten() {
                    match entry.property {
                        ChannelProperty::LocalPosition => position = position.plus(entry.value),
                        ChannelProperty::LocalRotation => rotation = rotation.plus(entry.value),
                        // The one property that replaces instead of adding (`mergeTransformWithOverride`).
                        ChannelProperty::LocalScale => scale = entry.value,
                    }
                }
                let local_matrix =
                    Mat4::compose(position, Quat::from_euler_xyz_degrees(rotation), scale);
                NodeState {
                    node_id: node.id.clone(),
                    local_position: position,
                    local_rotation_deg: rotation,
                    local_scale: scale,
                    render_order: base.render_order,
                    local_matrix,
                    world_matrix: Mat4::IDENTITY,
                }
            })
            .collect();

        let locals: Vec<Mat4> = states.iter().map(|state| state.local_matrix).collect();
        for (state, world) in states.iter_mut().zip(self.scene.compose_world(&locals)) {
            state.world_matrix = world;
        }
        // WHY an error and not a non-finite number in the frame: amplitude, delta and scale are
        // any finite number in the schema, so a bundle can ask for an overflow. The origin hands
        // `Infinity`/`NaN` to the renderer; a typed error names the node instead. The authored
        // pose is checked first so the error points at the cause, not at its descendants.
        let broken = states
            .iter()
            .find(|state| !state.is_local_finite())
            .or_else(|| states.iter().find(|state| !state.world_matrix.is_finite()));
        if let Some(state) = broken {
            return Err(TemporalError::NonFiniteNode {
                node_id: state.node_id.clone(),
            });
        }

        let index = states
            .iter()
            .enumerate()
            .map(|(i, state)| (state.node_id.clone(), i))
            .collect();
        Ok(NodeFrame {
            instance_id: self.instance.id.clone(),
            scene_definition_id: self.scene.id().to_string(),
            instance_frame,
            overrides,
            nodes: states,
            index,
        })
    }
}
