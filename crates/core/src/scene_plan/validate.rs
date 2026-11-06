//! The invariants of a plan, checked when one is built and when one is read from JSON.
//!
//! They are what a runtime relies on without re-deriving: ids resolve, the draw order is the
//! order of `layers`, the resource list is complete, sorted and has no repeats, `requires` says
//! exactly what the content needs, and nothing the plan does not evaluate is passed off as drawn.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::directed_scene::{AssetAlpha, AssetFormat, CameraState, NodeAnimation, NodeKind, Vec3};

use super::model::{Capability, Domain, PlanLayer, PlanNode, PlanSplat, ScenePlan};
use super::{PLAN_VERSION, PlanError};

/// Where each splat falls among the layers: the index of the first layer (plan order) whose node
/// origin is strictly nearer to the camera than the splat node's origin, else `layers.len()`.
/// Depth is the distance along the camera's viewing direction (`target - position`), taken at the
/// node's world-matrix translation. Splats the nodes list but `splats` does not name are skipped.
pub(super) fn derive_splat_slots(
    camera: &CameraState,
    nodes: &[PlanNode],
    layers: &[PlanLayer],
    splats: &[&str],
) -> Vec<usize> {
    let forward = [
        camera.target.x - camera.position.x,
        camera.target.y - camera.position.y,
        camera.target.z - camera.position.z,
    ];
    let depth = |id: &str| -> f64 {
        nodes
            .iter()
            .find(|node| node.id == id)
            .map(|node| {
                let m = &node.world_matrix.0;
                (m[12] - camera.position.x) * forward[0]
                    + (m[13] - camera.position.y) * forward[1]
                    + (m[14] - camera.position.z) * forward[2]
            })
            .unwrap_or(f64::INFINITY)
    };
    let layer_depths: Vec<f64> = layers.iter().map(|layer| depth(&layer.node_id)).collect();
    splats
        .iter()
        .map(|id| {
            let splat_depth = depth(id);
            layer_depths
                .iter()
                .position(|layer| *layer < splat_depth)
                .unwrap_or(layers.len())
        })
        .collect()
}

/// What a plan with this content asks of a runtime: ascending, without repeats.
pub(super) fn derive_requires(
    nodes: &[PlanNode],
    layers: &[PlanLayer],
    splats: &[PlanSplat],
    animation: &NodeAnimation,
) -> Vec<Capability> {
    let mut found = BTreeSet::from([Capability::Camera, Capability::NodeTransforms]);
    if nodes.iter().any(|node| {
        node.parent
            .as_ref()
            .is_some_and(|parent| parent.socket.is_some())
    }) {
        found.insert(Capability::Sockets);
    }
    if matches!(animation, NodeAnimation::Animated { .. }) {
        found.insert(Capability::PerformanceChannels);
    }
    if !layers.is_empty() {
        found.insert(Capability::ImagePlanes);
    }
    if !splats.is_empty() {
        found.insert(Capability::Splats);
    }
    for layer in layers {
        found.insert(match layer.alpha {
            AssetAlpha::Straight => Capability::StraightAlpha,
            AssetAlpha::Opaque => Capability::OpaqueAlpha,
        });
        if layer.pass_order.is_some() {
            found.insert(Capability::CompositionPassOrder);
        }
    }
    found.into_iter().collect()
}

/// The importer's rule for `relativePath` (not absolute, no drive letter, no `..`, no NUL),
/// repeated here because a plan can come from outside and the core must not hand a runtime a path
/// that leaves the bundle folder. The path is metadata; the identity is the `assetId`.
fn is_relative_to_bundle(path: &str) -> bool {
    let drive = path.len() >= 2 && path.as_bytes()[1] == b':';
    !(path.is_empty()
        || path.starts_with(['/', '\\'])
        || drive
        || path.contains('\0')
        || path.split(['/', '\\']).any(|segment| segment == ".."))
}

fn components(vector: Vec3) -> [f64; 3] {
    [vector.x, vector.y, vector.z]
}

fn require_finite(pointer: &str, values: &[f64]) -> Result<(), PlanError> {
    if values.iter().all(|value| value.is_finite()) {
        return Ok(());
    }
    Err(PlanError::invalid(pointer, "numbers must be finite"))
}

impl ScenePlan {
    /// Checks the plan against its own contract. `Ok` means a runtime can execute it as listed.
    /// Reports the first violation, with a JSON Pointer into the plan.
    pub fn validate(&self) -> Result<(), PlanError> {
        if self.version != PLAN_VERSION {
            return Err(PlanError::UnsupportedVersion {
                found: u64::from(self.version),
                supported: PLAN_VERSION,
            });
        }
        self.check_output_and_camera()?;
        let index = self.check_nodes()?;
        self.check_layers(&index)?;
        self.check_splats(&index)?;
        self.check_resources()?;
        if derive_requires(&self.nodes, &self.layers, &self.splats, &self.animation)
            != self.requires
        {
            return Err(PlanError::invalid(
                "/requires",
                "must list exactly the capabilities the content needs, ascending",
            ));
        }
        self.check_unsupported(&index)?;
        self.check_uninterpreted()
    }

    fn check_output_and_camera(&self) -> Result<(), PlanError> {
        if self.output.width == 0 || self.output.height == 0 {
            return Err(PlanError::invalid(
                "/output",
                "width and height must be positive",
            ));
        }
        if self.output.fps.rate().is_none() {
            return Err(PlanError::invalid(
                "/output/fps",
                "frame rate is not representable",
            ));
        }
        let camera = &self.camera;
        let mut values = Vec::from(components(camera.position));
        values.extend(components(camera.target));
        values.extend(components(camera.velocity));
        values.push(camera.fov_deg);
        values.push(camera.fov_velocity_deg_per_frame);
        require_finite("/camera", &values)
    }

    fn check_nodes(&self) -> Result<HashMap<&str, usize>, PlanError> {
        let mut index: HashMap<&str, usize> = HashMap::new();
        for (position, node) in self.nodes.iter().enumerate() {
            if node.id.is_empty() {
                return Err(PlanError::invalid(
                    format!("/nodes/{position}/id"),
                    "empty node id",
                ));
            }
            if index.insert(node.id.as_str(), position).is_some() {
                return Err(PlanError::invalid(
                    format!("/nodes/{position}/id"),
                    format!("duplicate node id {}", node.id),
                ));
            }
        }
        for (position, node) in self.nodes.iter().enumerate() {
            let at = format!("/nodes/{position}");
            let mut values: Vec<f64> = Vec::new();
            for vector in [
                node.local_position,
                node.local_rotation_deg,
                node.local_scale,
            ] {
                values.extend(components(vector));
            }
            values.extend(node.local_matrix.0);
            values.extend(node.world_matrix.0);
            values.extend(node.width_scene_units);
            values.extend(node.height_scene_units);
            require_finite(&at, &values)?;

            if let Some(parent) = &node.parent {
                if parent.node_id == node.id {
                    return Err(PlanError::invalid(
                        format!("{at}/parent"),
                        "a node cannot be its own parent",
                    ));
                }
                if !index.contains_key(parent.node_id.as_str()) {
                    return Err(PlanError::invalid(
                        format!("{at}/parent"),
                        format!("unknown parent {}", parent.node_id),
                    ));
                }
            }
            match node.kind {
                NodeKind::Group => {
                    if node.asset_id.is_some() {
                        return Err(PlanError::invalid(
                            format!("{at}/assetId"),
                            "a group has no asset",
                        ));
                    }
                }
                NodeKind::Splat => {
                    if node.asset_id.is_none() {
                        return Err(PlanError::invalid(
                            format!("{at}/assetId"),
                            "a splat names its asset",
                        ));
                    }
                }
                NodeKind::ImagePlane => {
                    if node.asset_id.is_none() {
                        return Err(PlanError::invalid(
                            format!("{at}/assetId"),
                            "an image plane names its asset",
                        ));
                    }
                    let sized = node
                        .width_scene_units
                        .zip(node.height_scene_units)
                        .is_some_and(|(width, height)| width > 0.0 && height > 0.0);
                    if !sized {
                        return Err(PlanError::invalid(
                            at,
                            "an image plane needs positive widthSceneUnits and heightSceneUnits",
                        ));
                    }
                }
            }
        }
        // The hierarchy is already in the matrices; a cycle here would only mislead a consumer
        // that walks it, so it is refused as well.
        for (position, node) in self.nodes.iter().enumerate() {
            let mut current = node;
            let mut steps = 0;
            while let Some(parent) = &current.parent {
                steps += 1;
                let next = index
                    .get(parent.node_id.as_str())
                    .map(|&at| &self.nodes[at]);
                match next {
                    Some(owner) if steps <= self.nodes.len() => current = owner,
                    _ => {
                        return Err(PlanError::invalid(
                            format!("/nodes/{position}/parent"),
                            "the parent chain does not end at a root",
                        ));
                    }
                }
            }
        }
        Ok(index)
    }

    fn check_layers(&self, index: &HashMap<&str, usize>) -> Result<(), PlanError> {
        let planes = self
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::ImagePlane)
            .count();
        if self.layers.len() != planes {
            return Err(PlanError::invalid(
                "/layers",
                "there must be exactly one layer per image plane",
            ));
        }
        let mut seen: HashSet<&str> = HashSet::new();
        let mut previous: Option<(i64, usize)> = None;
        for (position, layer) in self.layers.iter().enumerate() {
            let at = format!("/layers/{position}");
            let Some(&node_position) = index.get(layer.node_id.as_str()) else {
                return Err(PlanError::invalid(
                    format!("{at}/nodeId"),
                    format!("unknown node {}", layer.node_id),
                ));
            };
            let node = &self.nodes[node_position];
            if node.kind != NodeKind::ImagePlane {
                return Err(PlanError::invalid(
                    format!("{at}/nodeId"),
                    "only image planes are layers",
                ));
            }
            if !seen.insert(layer.node_id.as_str()) {
                return Err(PlanError::invalid(
                    format!("{at}/nodeId"),
                    format!("node {} is a layer twice", layer.node_id),
                ));
            }
            if node.asset_id.as_deref() != Some(layer.asset_id.as_str()) {
                return Err(PlanError::invalid(
                    format!("{at}/assetId"),
                    "differs from the asset of the node",
                ));
            }
            if node.render_order != layer.render_order {
                return Err(PlanError::invalid(
                    format!("{at}/renderOrder"),
                    "differs from the renderOrder of the node",
                ));
            }
            let Some(resource) = self.resource(&layer.asset_id) else {
                return Err(PlanError::invalid(
                    format!("{at}/assetId"),
                    format!("asset {} is not in resources", layer.asset_id),
                ));
            };
            if resource.alpha != layer.alpha {
                return Err(PlanError::invalid(
                    format!("{at}/alpha"),
                    "differs from the alpha of the resource",
                ));
            }
            // Back to front: ascending by effective order, ties in the document order of the nodes.
            let key = (layer.effective_order(), node_position);
            if previous.is_some_and(|before| before >= key) {
                return Err(PlanError::invalid(
                    at,
                    "layers must be ascending by order, ties in node order",
                ));
            }
            previous = Some(key);
        }
        Ok(())
    }

    fn check_splats(&self, index: &HashMap<&str, usize>) -> Result<(), PlanError> {
        let nodes: Vec<&str> = self
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Splat)
            .map(|node| node.id.as_str())
            .collect();
        let listed: Vec<&str> = self
            .splats
            .iter()
            .map(|splat| splat.node_id.as_str())
            .collect();
        if nodes != listed {
            return Err(PlanError::invalid(
                "/splats",
                "must list every splat node exactly once, in node order",
            ));
        }
        let derived = derive_splat_slots(&self.camera, &self.nodes, &self.layers, &listed);
        for (position, splat) in self.splats.iter().enumerate() {
            let at = format!("/splats/{position}");
            if splat.after_layers != derived[position] {
                return Err(PlanError::invalid(
                    format!("{at}/afterLayers"),
                    "must be the number of layers painted before the splat, from the camera depth of the node origins",
                ));
            }
            let node = &self.nodes[index[splat.node_id.as_str()]];
            if node.asset_id.as_deref() != Some(splat.asset_id.as_str()) {
                return Err(PlanError::invalid(
                    format!("{at}/assetId"),
                    "differs from the asset of the node",
                ));
            }
            match self.resource(&splat.asset_id) {
                Some(resource) if resource.is_splat() => {}
                Some(_) => {
                    return Err(PlanError::invalid(
                        format!("{at}/assetId"),
                        "a splat reads an spz resource",
                    ));
                }
                None => {
                    return Err(PlanError::invalid(
                        format!("{at}/assetId"),
                        format!("asset {} is not in resources", splat.asset_id),
                    ));
                }
            }
        }
        Ok(())
    }

    fn check_resources(&self) -> Result<(), PlanError> {
        if self
            .resources
            .windows(2)
            .any(|pair| pair[0].asset_id >= pair[1].asset_id)
        {
            return Err(PlanError::invalid(
                "/resources",
                "must be ascending by assetId, without repeats",
            ));
        }
        for (position, resource) in self.resources.iter().enumerate() {
            let at = format!("/resources/{position}");
            if resource.asset_id.is_empty() {
                return Err(PlanError::invalid(
                    format!("{at}/assetId"),
                    "empty asset id",
                ));
            }
            if !is_relative_to_bundle(&resource.relative_path) {
                return Err(PlanError::invalid(
                    format!("{at}/relativePath"),
                    "must stay inside the bundle folder (no absolute path, no `..`)",
                ));
            }
            if !matches!(resource.format, AssetFormat::Png | AssetFormat::Spz) {
                return Err(PlanError::invalid(
                    format!("{at}/format"),
                    "an image plane reads a png and a splat an spz",
                ));
            }
            if resource.width_px == 0
                || !(resource.height_px.is_finite() && resource.height_px > 0.0)
            {
                return Err(PlanError::invalid(
                    at,
                    "widthPx and heightPx must be positive and finite",
                ));
            }
            let expected: Vec<&str> = self
                .nodes
                .iter()
                .filter(|node| {
                    let kind = if resource.is_splat() {
                        NodeKind::Splat
                    } else {
                        NodeKind::ImagePlane
                    };
                    node.kind == kind
                        && node.asset_id.as_deref() == Some(resource.asset_id.as_str())
                })
                .map(|node| node.id.as_str())
                .collect();
            let listed: Vec<&str> = resource.used_by.iter().map(String::as_str).collect();
            if expected.is_empty() || listed != expected {
                return Err(PlanError::invalid(
                    format!("{at}/usedBy"),
                    "must be the nodes that use the asset (image planes for a png, splats for an spz), in node order, and not empty",
                ));
            }
        }
        Ok(())
    }

    fn check_unsupported(&self, index: &HashMap<&str, usize>) -> Result<(), PlanError> {
        if self.unsupported.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(PlanError::invalid(
                "/unsupported",
                "must be ascending, without repeats",
            ));
        }
        for (position, entry) in self.unsupported.iter().enumerate() {
            let at = format!("/unsupported/{position}");
            match &entry.subject {
                Some(node_id) => {
                    if !index.contains_key(node_id.as_str()) {
                        return Err(PlanError::invalid(
                            format!("{at}/subject"),
                            format!("unknown node {node_id}"),
                        ));
                    }
                    if !entry.domain.is_node_level() {
                        return Err(PlanError::invalid(
                            format!("{at}/subject"),
                            "a scene-level domain has no subject",
                        ));
                    }
                }
                None => {
                    // `rigBind`/`reveal` may be reported without a node when the evaluator knew the
                    // section is declared but no node could be named.
                    let anonymous = matches!(entry.domain, Domain::RigBind | Domain::Reveal);
                    if entry.domain.is_node_level() && !anonymous {
                        return Err(PlanError::invalid(
                            format!("{at}/subject"),
                            "a node-level domain names its node",
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn check_uninterpreted(&self) -> Result<(), PlanError> {
        if self
            .uninterpreted
            .iter()
            .any(|field| !field.pointer.starts_with('/'))
        {
            return Err(PlanError::invalid(
                "/uninterpreted",
                "pointers are JSON Pointers into the bundle",
            ));
        }
        if self
            .uninterpreted
            .windows(2)
            .any(|pair| pair[0].pointer >= pair[1].pointer)
        {
            return Err(PlanError::invalid(
                "/uninterpreted",
                "must be ascending by pointer, without repeats",
            ));
        }
        Ok(())
    }
}
