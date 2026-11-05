//! The scene graph of one scene definition: nodes, hierarchy, sockets and 3D transforms.
//!
//! Rules are those of the origin's `validateSceneGraphStructure` (`scene-graph.ts`) and the
//! graph/node/socket part of `validateBundleIntegrity` (`bundle-integrity.ts`), with its codes.
//! Matrices follow `computeWorldMatrices`: `world = parentWorld * local`, and for a node on a
//! socket `parentWorld = ownerWorld * socketLocal`.

use std::collections::{HashMap, HashSet};

use super::issue::{Issue, NON_FINITE_TRANSFORM};
use super::model::{Asset, AssetFormat, Node, NodeKind, Socket};
use super::read::{at, child};
use super::transform::{Mat4, Transform};

/// A scene definition as read, before its rules have run.
pub(super) struct SceneDef {
    pub id: String,
    pub root: String,
    pub nodes: Vec<Node>,
    pub sockets: Vec<Socket>,
    /// Assets that deferred node sections (surface map, normal map) read as data, so a
    /// `linear-srgb` asset is legitimate when one of them points at it.
    pub data_assets: HashSet<String>,
    /// Sections of the definition that change the frame and that nothing evaluates yet
    /// (`weather`, `lighting`, `nodes[].rigBind`...), sorted, without repeats.
    pub unevaluated: Vec<&'static str>,
    /// JSON Pointer of the definition (`/sceneDefinitions/0`).
    pub pointer: String,
}

/// What a node hangs from, once the declared `parentId`/`attachToSocket` are resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parent {
    Node(String),
    Socket { owner: String, socket: String },
}

impl Parent {
    /// The node that owns this attachment point.
    pub fn owner(&self) -> &str {
        match self {
            Self::Node(id) => id,
            Self::Socket { owner, .. } => owner,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneGraph {
    id: String,
    root: String,
    nodes: Vec<Node>,
    sockets: Vec<Socket>,
    index: HashMap<String, usize>,
    parents: Vec<Option<Parent>>,
    children: HashMap<String, Vec<usize>>,
    /// Node indices parents-first, from the root: the order a world pass must follow.
    order: Vec<usize>,
    local: Vec<Mat4>,
    world: Vec<Mat4>,
    unevaluated: Vec<&'static str>,
}

impl SceneGraph {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn root_id(&self) -> &str {
        &self.root
    }

    /// Nodes in document order.
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn sockets(&self) -> &[Socket] {
        &self.sockets
    }

    pub fn node(&self, id: &str) -> Option<&Node> {
        self.index.get(id).map(|&i| &self.nodes[i])
    }

    pub fn socket(&self, owner: &str, name: &str) -> Option<&Socket> {
        self.sockets
            .iter()
            .find(|socket| socket.owner == owner && socket.name == name)
    }

    /// The effective parent; `None` for the root.
    pub fn parent(&self, id: &str) -> Option<&Parent> {
        self.parents[*self.index.get(id)?].as_ref()
    }

    /// Nodes hanging from `id`, directly or through one of its sockets, in document order.
    pub fn children(&self, id: &str) -> Vec<&Node> {
        self.children
            .get(id)
            .map(|list| list.iter().map(|&i| &self.nodes[i]).collect())
            .unwrap_or_default()
    }

    /// From the parent up to the root, nearest first. Empty for the root.
    pub fn ancestors(&self, id: &str) -> Vec<&Node> {
        let mut out = Vec::new();
        let mut current = id;
        // The graph is acyclic (checked at import); the bound only guards against misuse.
        while let Some(parent) = self.parent(current) {
            let Some(owner) = self.node(parent.owner()) else {
                break;
            };
            out.push(owner);
            current = &owner.id;
            if out.len() > self.nodes.len() {
                break;
            }
        }
        out
    }

    /// The node's own transform composed to a matrix.
    pub fn local_matrix(&self, id: &str) -> Option<&Mat4> {
        self.index.get(id).map(|&i| &self.local[i])
    }

    /// The node in scene space: every ancestor (and socket) transform applied.
    pub fn world_matrix(&self, id: &str) -> Option<&Mat4> {
        self.index.get(id).map(|&i| &self.world[i])
    }

    /// Sections of this scene definition that change a frame and that no evaluator computes yet
    /// (`weather`, `lighting`, `rigs`, `nodes[].rigBind`...), sorted. A frame evaluation of an
    /// instance that plays this scene reports them instead of passing for the whole picture.
    pub fn unevaluated_sections(&self) -> &[&'static str] {
        &self.unevaluated
    }

    /// World matrices for the given local matrices (one per node, in document order), with the
    /// graph's hierarchy and sockets: `world = parentWorld * local`, and for a node on a socket
    /// `parentWorld = ownerWorld * socketLocal`. The static graph uses this with the authored
    /// locals; an animated frame uses it with the locals its channels produced (`animation`).
    pub(in crate::directed_scene) fn compose_world(&self, local: &[Mat4]) -> Vec<Mat4> {
        let mut world = vec![Mat4::IDENTITY; self.nodes.len()];
        for &i in &self.order {
            let parent_world = match &self.parents[i] {
                None => Mat4::IDENTITY,
                Some(Parent::Node(owner)) => world[self.index[owner.as_str()]],
                Some(Parent::Socket { owner, socket }) => {
                    let socket_local =
                        self.socket(owner, socket).map_or(Mat4::IDENTITY, |socket| {
                            Transform {
                                position: socket.position,
                                rotation: socket.rotation,
                                scale: super::transform::Vec3::ONE,
                            }
                            .matrix()
                        });
                    world[self.index[owner.as_str()]].mul(&socket_local)
                }
            };
            world[i] = parent_world.mul(&local[i]);
        }
        world
    }

    pub fn image_planes(&self) -> impl Iterator<Item = &Node> {
        self.nodes
            .iter()
            .filter(|node| node.kind == NodeKind::ImagePlane)
    }
}

// ---- rules ---------------------------------------------------------------------------------

type Parentage<'a> = HashMap<&'a str, Option<&'a str>>;

/// Walks every parent chain once and reports where a chain closes on itself, like the origin's
/// `visit`: the issue names the node at which the cycle is detected.
// WHY iterative: a chain of 50 000 nodes must not overflow the stack of whoever imports it.
fn cycle_issues(
    nodes: &[Node],
    parent_of: &Parentage<'_>,
    code: &'static str,
    label: &str,
    def_pointer: &str,
    issues: &mut Vec<Issue>,
) {
    #[derive(PartialEq)]
    enum State {
        Visiting,
        Visited,
    }
    let mut state: HashMap<&str, State> = HashMap::new();
    for node in nodes {
        let mut trail: Vec<&str> = Vec::new();
        let mut current = Some(node.id.as_str());
        while let Some(id) = current {
            match state.get(id) {
                Some(State::Visited) => break,
                Some(State::Visiting) => {
                    issues.push(Issue::new(
                        code,
                        child(def_pointer, "nodes"),
                        format!("{label} involving node {id}"),
                    ));
                    break;
                }
                None => {}
            }
            state.insert(id, State::Visiting);
            trail.push(id);
            current = parent_of.get(id).copied().flatten();
        }
        for id in trail {
            state.insert(id, State::Visited);
        }
    }
}

pub(super) fn validate(def: &SceneDef, assets: &[Asset], issues: &mut Vec<Issue>) {
    let nodes_path = child(&def.pointer, "nodes");
    let by_id: HashMap<&str, &Node> = def.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let asset_by_id: HashMap<&str, &Asset> =
        assets.iter().map(|a| (a.asset_id.as_str(), a)).collect();
    let socket_keys: HashSet<(&str, &str)> = def
        .sockets
        .iter()
        .map(|s| (s.owner.as_str(), s.name.as_str()))
        .collect();

    if !by_id.contains_key(def.root.as_str()) {
        issues.push(Issue::new(
            "MISSING_ROOT_NODE",
            child(&def.pointer, "rootNodeId"),
            format!("rootNodeId {} not found in nodes", def.root),
        ));
    }

    let mut seen = HashSet::new();
    for (j, node) in def.nodes.iter().enumerate() {
        let node_path = at(&nodes_path, j);
        if !seen.insert(node.id.as_str()) {
            issues.push(Issue::new(
                "DUPLICATE_NODE_ID",
                &node_path,
                format!("Duplicate node id {}", node.id),
            ));
        }
        match &node.parent_id {
            None if node.id != def.root && node.attach_to_socket.is_none() => {
                issues.push(Issue::new(
                    "ORPHAN_NODE",
                    &node_path,
                    format!("Node {} has null parentId but is not root", node.id),
                ));
            }
            Some(parent) if !by_id.contains_key(parent.as_str()) => {
                issues.push(Issue::new(
                    "MISSING_PARENT",
                    child(&node_path, "parentId"),
                    format!("Node {} parentId {parent} does not exist", node.id),
                ));
            }
            _ => {}
        }
    }

    // Declared parents only: `parentId` (an empty string counts as none, as in the origin).
    let declared: Parentage = def
        .nodes
        .iter()
        .map(|n| {
            (
                n.id.as_str(),
                n.parent_id.as_deref().filter(|p| !p.is_empty()),
            )
        })
        .collect();
    cycle_issues(
        &def.nodes,
        &declared,
        "PARENT_CYCLE",
        "Cycle detected",
        &def.pointer,
        issues,
    );

    for (j, node) in def.nodes.iter().enumerate() {
        let node_path = at(&nodes_path, j);
        let is_root = node.id == def.root;
        if is_root && node.attach_to_socket.is_some() {
            issues.push(Issue::new(
                "ROOT_SOCKET_UNSUPPORTED",
                child(&node_path, "attachToSocket"),
                "Root cannot attach to a socket",
            ));
        }
        if is_root && node.parent_id.is_some() {
            issues.push(Issue::new(
                "ROOT_HAS_PARENT",
                child(&node_path, "parentId"),
                format!("rootNodeId {} cannot have a parentId", def.root),
            ));
        }
        let Some(attachment) = &node.attach_to_socket else {
            continue;
        };
        let attach_path = child(&node_path, "attachToSocket");
        if !socket_keys.contains(&(attachment.owner.as_str(), attachment.socket.as_str())) {
            issues.push(Issue::new(
                "MISSING_SOCKET",
                &attach_path,
                format!(
                    "Node {} attachToSocket {} on {} not found",
                    node.id, attachment.socket, attachment.owner
                ),
            ));
        }
        if node.parent_id.as_deref().is_some_and(|p| p != def.root) {
            issues.push(Issue::new(
                "SOCKET_AND_PARENT_CONFLICT",
                &node_path,
                format!(
                    "Node {} cannot set parentId and attachToSocket together in pilot",
                    node.id
                ),
            ));
        }
        if attachment.owner == node.id {
            issues.push(Issue::new(
                "SOCKET_SELF_REFERENCE",
                &attach_path,
                format!("Node {} cannot attach to its own socket", node.id),
            ));
        }
    }

    let effective: Parentage = def
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), effective_parent(n, &def.root)))
        .collect();
    cycle_issues(
        &def.nodes,
        &effective,
        "EFFECTIVE_GRAPH_CYCLE",
        "Effective parenting cycle",
        &def.pointer,
        issues,
    );

    sockets_and_nodes(def, &by_id, &asset_by_id, issues);
}

/// The node's effective parent: the socket owner when attached, else `parentId`, else none.
fn effective_parent<'a>(node: &'a Node, root: &str) -> Option<&'a str> {
    if node.id == root {
        return None;
    }
    if let Some(attachment) = &node.attach_to_socket {
        return Some(attachment.owner.as_str());
    }
    node.parent_id.as_deref().filter(|p| !p.is_empty())
}

fn sockets_and_nodes(
    def: &SceneDef,
    by_id: &HashMap<&str, &Node>,
    asset_by_id: &HashMap<&str, &Asset>,
    issues: &mut Vec<Issue>,
) {
    let sockets_path = child(&def.pointer, "sockets");
    let mut socket_keys = HashSet::new();
    for (k, socket) in def.sockets.iter().enumerate() {
        let path = at(&sockets_path, k);
        let key = format!("{}:{}", socket.owner, socket.name);
        if !socket_keys.insert(key.clone()) {
            issues.push(Issue::new(
                "DUPLICATE_SOCKET",
                &path,
                format!("Duplicate socket {key} on {}", def.id),
            ));
        }
        if !by_id.contains_key(socket.owner.as_str()) {
            issues.push(Issue::new(
                "SOCKET_OWNER_MISSING",
                child(&path, "nodeId"),
                format!("Socket {key} owner node missing on {}", def.id),
            ));
        }
        if socket.rotation.length() < 1e-6 {
            issues.push(Issue::new(
                "DEGENERATE_QUATERNION",
                child(&path, "localRotation"),
                format!("Socket {key} localRotation is zero-length"),
            ));
        }
    }

    let nodes_path = child(&def.pointer, "nodes");
    for (j, node) in def.nodes.iter().enumerate() {
        let path = at(&nodes_path, j);
        let length = node.transform.rotation.length();
        if length < 1e-6 {
            issues.push(Issue::new(
                "DEGENERATE_QUATERNION",
                child(&path, "localRotation"),
                format!("Node {} localRotation is zero-length", node.id),
            ));
        } else if (length - 1.0).abs() > 0.01 {
            issues.push(Issue::new(
                "QUATERNION_NOT_NORMALIZED",
                child(&path, "localRotation"),
                format!(
                    "Node {} localRotation length {length:.4} (expected ~1)",
                    node.id
                ),
            ));
        }
        if node.kind == NodeKind::ImagePlane {
            if node.asset.is_none() {
                issues.push(Issue::new(
                    "IMAGE_PLANE_MISSING_ASSET",
                    &path,
                    format!("imagePlane {} missing assetRef", node.id),
                ));
            }
            if node.width_scene_units.is_none() || node.height_scene_units.is_none() {
                issues.push(Issue::new(
                    "IMAGE_PLANE_MISSING_DIMENSIONS",
                    &path,
                    format!(
                        "imagePlane {} requires widthSceneUnits and heightSceneUnits",
                        node.id
                    ),
                ));
            }
        }
        if node.kind == NodeKind::Splat && node.asset.is_none() {
            issues.push(Issue::new(
                "SPLAT_MISSING_ASSET",
                &path,
                format!("splat {} missing assetRef", node.id),
            ));
        }
        let Some(reference) = &node.asset else {
            continue;
        };
        let ref_path = child(&path, "assetRef");
        let Some(record) = asset_by_id.get(reference.asset_id.as_str()) else {
            issues.push(Issue::new(
                "ASSET_REF_UNKNOWN",
                &ref_path,
                format!(
                    "Node {} references missing asset {}",
                    node.id, reference.asset_id
                ),
            ));
            continue;
        };
        if record.revision_id != reference.revision_id
            || record.content_hash != reference.content_hash
            || record.relative_path != reference.relative_path
        {
            issues.push(Issue::new(
                "ASSET_REF_MISMATCH",
                &ref_path,
                format!(
                    "Node {} assetRef does not match assets[] record for {}",
                    node.id, reference.asset_id
                ),
            ));
            continue;
        }
        let expected = match node.kind {
            NodeKind::ImagePlane => Some(AssetFormat::Png),
            NodeKind::Splat => Some(AssetFormat::Spz),
            NodeKind::Group => None,
        };
        if let Some(expected) = expected.filter(|expected| *expected != record.format) {
            issues.push(Issue::new(
                "ASSET_FORMAT_MISMATCH",
                &ref_path,
                format!(
                    "Node {} needs a {} asset; {} is {}",
                    node.id,
                    expected.as_str(),
                    reference.asset_id,
                    record.format.as_str()
                ),
            ));
        }
    }
}

// ---- build ---------------------------------------------------------------------------------

impl SceneGraph {
    /// Builds the queryable graph from a definition whose rules found nothing. Reports a node
    /// whose world matrix overflows, which no rule of the origin catches.
    pub(super) fn build(def: SceneDef, issues: &mut Vec<Issue>) -> Self {
        let index: HashMap<String, usize> = def
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.clone(), i))
            .collect();
        let parents: Vec<Option<Parent>> = def
            .nodes
            .iter()
            .map(|node| {
                if node.id == def.root {
                    return None;
                }
                if let Some(attachment) = &node.attach_to_socket {
                    return Some(Parent::Socket {
                        owner: attachment.owner.clone(),
                        socket: attachment.socket.clone(),
                    });
                }
                node.parent_id.clone().map(Parent::Node)
            })
            .collect();
        let mut children: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, parent) in parents.iter().enumerate() {
            if let Some(parent) = parent {
                children
                    .entry(parent.owner().to_string())
                    .or_default()
                    .push(i);
            }
        }
        let local: Vec<Mat4> = def.nodes.iter().map(|n| n.transform.matrix()).collect();

        // Top-down from the root, so a parent's world matrix exists before its children's.
        let mut order: Vec<usize> = index.get(&def.root).copied().into_iter().collect();
        let mut cursor = 0;
        while let Some(&i) = order.get(cursor) {
            cursor += 1;
            if let Some(next) = children.get(&def.nodes[i].id) {
                order.extend(next.iter().copied());
            }
        }

        let mut graph = Self {
            id: def.id,
            root: def.root,
            nodes: def.nodes,
            sockets: def.sockets,
            index,
            parents,
            children,
            order,
            local,
            world: Vec::new(),
            unevaluated: def.unevaluated,
        };
        graph.world = graph.compose_world(&graph.local);
        for &i in &graph.order {
            if !graph.world[i].is_finite() {
                issues.push(Issue::new(
                    NON_FINITE_TRANSFORM,
                    at(&child(&def.pointer, "nodes"), i),
                    format!(
                        "The world matrix of node {} is not finite; the hierarchy's transforms overflow",
                        graph.nodes[i].id
                    ),
                ));
            }
        }
        graph
    }
}
