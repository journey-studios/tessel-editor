//! Building the plan of an edit frame from an imported scene.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_json::Value;

use crate::directed_scene::{
    AssetAlpha, Bundle, CameraState, DirectedScene, FrameEvaluation, NodeKind, Parent, SceneGraph,
};

use super::model::{
    Domain, PlanFrame, PlanLayer, PlanNode, PlanOutput, PlanParent, PlanResource, PlanSource,
    PlanSplat, ScenePlan, UninterpretedField, UninterpretedReason, UnsupportedDomain,
};
use super::validate::{derive_requires, derive_splat_slots};
use super::{PLAN_VERSION, PlanError};

/// How `ScenePlan::build_with` treats a frame that declares domains the plan does not evaluate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlanOptions {
    /// `false` (the default): refuse with `PlanError::Unsupported`. `true`: return the plan of
    /// what *is* evaluated, with `unsupported` filled and `is_complete()` false. A partial plan
    /// is a decision of the caller (a diagnostic preview, a gate that wants the list); a runtime
    /// that draws it as if it were the picture shows a scene without its lights and weather.
    pub allow_partial: bool,
}

/// Top-level sections the importer keeps raw. None of them is part of the frame's picture
/// (`DEFERRED_FIELDS`); the ones that would change it are `Domain`s instead.
const DEFERRED_ROOT_SECTIONS: [&str; 3] = ["narration", "outline", "metadata"];

/// The same for a scene definition (`contactPlaneId` only matters through lighting and shadows,
/// which are domains; `frameAudit` is a quality record).
const DEFERRED_DEFINITION_SECTIONS: [&str; 2] = ["contactPlaneId", "frameAudit"];

/// A section counts as declared when it is present and not `null`, as in the importer.
fn declared(section: Option<&Value>) -> bool {
    section.is_some_and(|value| !value.is_null())
}

fn plan_parent(parent: &Parent) -> PlanParent {
    match parent {
        Parent::Node(node_id) => PlanParent {
            node_id: node_id.clone(),
            socket: None,
        },
        Parent::Socket { owner, socket } => PlanParent {
            node_id: owner.clone(),
            socket: Some(socket.clone()),
        },
    }
}

/// The scene definition of the frame inside the bundle document: its array index and its object.
fn locate_definition<'a>(
    document: &'a Value,
    definition_id: &str,
) -> Result<(usize, &'a Value), PlanError> {
    document
        .get("sceneDefinitions")
        .and_then(Value::as_array)
        .and_then(|definitions| {
            definitions.iter().enumerate().find(|(_, definition)| {
                definition.get("id").and_then(Value::as_str) == Some(definition_id)
            })
        })
        .ok_or_else(|| PlanError::MalformedDocument {
            pointer: "/sceneDefinitions".to_string(),
        })
}

/// `renderProfile.alphaMode`, which the typed bundle does not keep (only supported values import).
fn output_alpha(document: &Value) -> Result<AssetAlpha, PlanError> {
    match document
        .pointer("/renderProfile/alphaMode")
        .and_then(Value::as_str)
    {
        Some("straight") => Ok(AssetAlpha::Straight),
        Some("opaque") => Ok(AssetAlpha::Opaque),
        _ => Err(PlanError::MalformedDocument {
            pointer: "/renderProfile/alphaMode".to_string(),
        }),
    }
}

/// Every declared domain of the frame's scene that the plan does not evaluate, sorted.
///
/// Scene-level domains and `rigBind`/`reveal` come from the evaluator's own list
/// (`unevaluated`); the node's data maps are added here (splats are evaluated: S7). The nodes' sections
/// are read from the raw document because the typed model does not keep them.
fn unsupported_domains(
    unevaluated: &[&'static str],
    raw_nodes: &[Value],
) -> Vec<UnsupportedDomain> {
    let mut found: BTreeSet<UnsupportedDomain> = BTreeSet::new();
    for label in unevaluated {
        // The node-level labels (`nodes[].rigBind`...) do not parse as a domain on purpose: they
        // are handled by the scan of the nodes below.
        if let Some(domain) = Domain::parse(label).filter(|domain| !domain.is_node_level()) {
            found.insert(UnsupportedDomain {
                domain,
                subject: None,
            });
        }
    }
    for raw in raw_nodes {
        let Some(node_id) = raw.get("id").and_then(Value::as_str) else {
            continue;
        };
        for domain in Domain::NODE_SECTIONS {
            if declared(raw.get(domain.as_str())) {
                found.insert(UnsupportedDomain {
                    domain,
                    subject: Some(node_id.to_string()),
                });
            }
        }
    }
    // The evaluator said a node section is declared; if the scan found no node for it, the domain
    // is still reported (without a subject) rather than dropped.
    for (label, domain) in [
        ("nodes[].rigBind", Domain::RigBind),
        ("nodes[].reveal", Domain::Reveal),
    ] {
        if unevaluated.contains(&label) && !found.iter().any(|entry| entry.domain == domain) {
            found.insert(UnsupportedDomain {
                domain,
                subject: None,
            });
        }
    }
    found.into_iter().collect()
}

/// Indices of the elements of a top-level array of the document that satisfy `keep`.
fn indices_where(document: &Value, key: &str, keep: impl Fn(&Value) -> bool) -> Vec<usize> {
    document
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .enumerate()
                .filter(|entry| keep(entry.1))
                .map(|entry| entry.0)
                .collect()
        })
        .unwrap_or_default()
}

fn has_id(item: &Value, id: &str) -> bool {
    item.get("id").and_then(Value::as_str) == Some(id)
}

fn within(pointer: &str, prefix: &str) -> bool {
    pointer == prefix
        || pointer
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The bundle data the importer kept without interpreting, limited to what the frame touches: the
/// top level of the document, the scene definition, the instance, shot, camera clip, performance
/// tracks, composition passes and assets the plan uses.
///
/// WHY scoped and not the whole bundle: the plan of a frame must not change because another scene
/// of the same bundle got an extra field; the notices of untouched entities say nothing about
/// this picture.
fn uninterpreted_fields(
    scene: &DirectedScene,
    definition_index: usize,
    frame: &FrameEvaluation<'_>,
    layers: &[PlanLayer],
    resources: &[PlanResource],
    definition: &Value,
) -> Vec<UninterpretedField> {
    let document = scene.document();
    let mut scope: Vec<String> = vec![format!("/sceneDefinitions/{definition_index}")];
    let layer_nodes: BTreeSet<&str> = layers.iter().map(|layer| layer.node_id.as_str()).collect();
    let keyed = |key: &str, ids: Vec<usize>, scope: &mut Vec<String>| {
        scope.extend(ids.into_iter().map(|index| format!("/{key}/{index}")));
    };
    keyed(
        "sceneInstances",
        indices_where(document, "sceneInstances", |item| {
            has_id(item, &frame.mapping.instance.id)
        }),
        &mut scope,
    );
    keyed(
        "shots",
        indices_where(document, "shots", |item| {
            has_id(item, &frame.mapping.shot.id)
        }),
        &mut scope,
    );
    keyed(
        "cameraClips",
        indices_where(document, "cameraClips", |item| {
            has_id(item, &frame.mapping.shot.camera_clip_id)
        }),
        &mut scope,
    );
    keyed(
        "performanceTracks",
        indices_where(document, "performanceTracks", |item| {
            frame
                .mapping
                .instance
                .performance_track_ids
                .iter()
                .any(|track| has_id(item, track))
        }),
        &mut scope,
    );
    keyed(
        "compositionPasses",
        indices_where(document, "compositionPasses", |item| {
            item.get("nodeId")
                .and_then(Value::as_str)
                .is_some_and(|node_id| layer_nodes.contains(node_id))
        }),
        &mut scope,
    );
    keyed(
        "assets",
        indices_where(document, "assets", |item| {
            resources.iter().any(|resource| {
                item.get("assetId").and_then(Value::as_str) == Some(resource.asset_id.as_str())
            })
        }),
        &mut scope,
    );

    let mut found: BTreeMap<String, UninterpretedReason> = BTreeMap::new();
    for notice in scene.notices() {
        // A pointer with a single segment is a key of the bundle itself.
        let top_level = notice.path.matches('/').count() == 1;
        if top_level || scope.iter().any(|prefix| within(&notice.path, prefix)) {
            found.insert(notice.path.clone(), UninterpretedReason::UnknownField);
        }
    }
    for section in DEFERRED_ROOT_SECTIONS {
        if declared(document.get(section)) {
            found.insert(format!("/{section}"), UninterpretedReason::DeferredSection);
        }
    }
    for section in DEFERRED_DEFINITION_SECTIONS {
        if declared(definition.get(section)) {
            found.insert(
                format!("/sceneDefinitions/{definition_index}/{section}"),
                UninterpretedReason::DeferredSection,
            );
        }
    }
    found
        .into_iter()
        .map(|(pointer, reason)| UninterpretedField { pointer, reason })
        .collect()
}

/// What every frame of one shot shares, found once: the scene definition the shot plays and the
/// domains it declares without evaluating. `ScenePlan::build_with` prepares this for each call;
/// `PlanTimeline` keeps it for the shot.
///
/// WHY a shot is the unit: a shot fixes the instance, the camera clip and the scene definition, and
/// everything below is a function of those, never of the frame (the performance channels move a
/// node's transform, not its kind, parent, asset, size or `renderOrder`).
pub(super) struct Prepared<'a> {
    definition_id: String,
    definition_index: usize,
    definition: &'a Value,
    graph: &'a SceneGraph,
    unsupported: Vec<UnsupportedDomain>,
}

impl<'a> Prepared<'a> {
    /// The evaluator's own error comes first (the caller evaluated the frame already); what fails
    /// here is the document or the graph not matching what the importer guarantees.
    pub(super) fn new(
        scene: &'a DirectedScene,
        frame: &FrameEvaluation<'_>,
    ) -> Result<Self, PlanError> {
        let bundle = scene.bundle();
        let definition_id = frame.nodes.scene_definition_id().to_string();
        let graph = bundle
            .scene(&definition_id)
            .ok_or_else(|| PlanError::Unresolved {
                kind: "scene definition",
                id: definition_id.clone(),
            })?;
        let (definition_index, definition) = locate_definition(scene.document(), &definition_id)?;
        let raw_nodes = definition
            .get("nodes")
            .and_then(Value::as_array)
            .ok_or_else(|| PlanError::MalformedDocument {
                pointer: format!("/sceneDefinitions/{definition_index}/nodes"),
            })?;
        let unsupported = unsupported_domains(&frame.unevaluated, raw_nodes);
        Ok(Self {
            definition_id,
            definition_index,
            definition,
            graph,
            unsupported,
        })
    }

    /// A complete plan of a frame that declares domains the plan does not evaluate is refused.
    pub(super) fn refuse_unsupported(&self, options: PlanOptions) -> Result<(), PlanError> {
        if options.allow_partial || self.unsupported.is_empty() {
            return Ok(());
        }
        Err(PlanError::Unsupported {
            domains: self.unsupported.clone(),
        })
    }

    /// The validated plan of the frame. Its parts that do not depend on the frame (`source`,
    /// `output`, kinds, parents, assets, `layers`, `resources`, `requires`, `unsupported`,
    /// `uninterpreted`) are what `ScenePlan::at_frame` reuses.
    pub(super) fn assemble(
        &self,
        scene: &DirectedScene,
        frame: &FrameEvaluation<'_>,
    ) -> Result<ScenePlan, PlanError> {
        let bundle = scene.bundle();
        let nodes = plan_nodes(frame, self.graph)?;
        let layers = plan_layers(bundle, frame, &self.definition_id)?;
        let splats = plan_splats(&nodes, &layers, &frame.camera)?;
        let resources = plan_resources(bundle, &nodes)?;
        let uninterpreted = uninterpreted_fields(
            scene,
            self.definition_index,
            frame,
            &layers,
            &resources,
            self.definition,
        );
        let requires = derive_requires(&nodes, &layers, &splats, &frame.node_animation);

        let profile = &bundle.render_profile;
        let mapping = &frame.mapping;
        let plan = ScenePlan {
            version: PLAN_VERSION,
            source: PlanSource {
                bundle_id: bundle.bundle_id.clone(),
                revision_id: bundle.revision_id.clone(),
                scene_definition_id: self.definition_id.clone(),
                scene_instance_id: mapping.instance.id.clone(),
            },
            output: PlanOutput {
                width: profile.width,
                height: profile.height,
                fps: profile.fps,
                alpha_mode: output_alpha(scene.document())?,
            },
            frame: plan_frame(frame),
            camera: frame.camera,
            animation: frame.node_animation.clone(),
            nodes,
            layers,
            splats,
            resources,
            requires,
            unsupported: self.unsupported.clone(),
            uninterpreted,
        };
        // WHY validate what was just built: the invariants (draw order, resource list, requires)
        // are what a runtime relies on, so a build that would break them fails here, loudly,
        // instead of shipping a plan that `from_json_str` would later refuse.
        plan.validate()?;
        Ok(plan)
    }
}

fn plan_frame(frame: &FrameEvaluation<'_>) -> PlanFrame {
    let mapping = &frame.mapping;
    PlanFrame {
        edit_frame: mapping.edit_frame,
        edit_time: mapping.edit_time,
        shot_id: mapping.shot.id.clone(),
        instance_frame: mapping.instance_frame,
        instance_time: mapping.instance_time,
        camera_clip_id: mapping.shot.camera_clip_id.clone(),
        camera_clip_frame: mapping.camera_clip_frame,
        camera_clip_time: mapping.camera_clip_time,
    }
}

impl ScenePlan {
    /// The plan of an edit frame, complete or refused: a frame that declares domains the plan does
    /// not evaluate is `PlanError::Unsupported`, never a quietly reduced picture.
    pub fn build(scene: &DirectedScene, edit_frame: i64) -> Result<Self, PlanError> {
        Self::build_with(scene, edit_frame, PlanOptions::default())
    }

    /// `build` with explicit options. The scene is only read: a refusal changes nothing.
    ///
    /// The errors come in the evaluator's order: first the frame (range, shot, channels, camera,
    /// node overflow: `PlanError::Frame`), then the declared domains.
    ///
    /// Each call lays the scene out again. To plan many frames of one scene use `PlanTimeline`,
    /// which returns the same plan or error for every frame.
    pub fn build_with(
        scene: &DirectedScene,
        edit_frame: i64,
        options: PlanOptions,
    ) -> Result<Self, PlanError> {
        let frame = scene.bundle().evaluate_frame(edit_frame)?;
        let prepared = Prepared::new(scene, &frame)?;
        prepared.refuse_unsupported(options)?;
        prepared.assemble(scene, &frame)
    }

    /// This plan moved to another frame of the same shot: what the frame changes (times, camera,
    /// the nodes' transforms and matrices) is taken from `frame`; everything else is this plan's.
    ///
    /// The nodes of a shot's frames are the same list in the same order (they come from one scene
    /// definition), so a frame that does not line up with this plan is a bug, reported as the node
    /// that does not resolve instead of a plan with some other frame's nodes in it.
    pub(super) fn at_frame(&self, frame: &FrameEvaluation<'_>) -> Result<Self, PlanError> {
        let states = frame.nodes.nodes();
        // The first position where the two lists stop agreeing (one list may simply be longer).
        let misaligned = (0..states.len().max(self.nodes.len())).find_map(|at| {
            let planned = self.nodes.get(at).map(|node| node.id.as_str());
            let evaluated = states.get(at).map(|state| state.node_id.as_str());
            (planned != evaluated).then(|| planned.or(evaluated).unwrap_or_default().to_string())
        });
        if let Some(id) = misaligned {
            return Err(PlanError::Unresolved { kind: "node", id });
        }
        let mut plan = self.clone();
        plan.frame = plan_frame(frame);
        plan.camera = frame.camera;
        for (node, state) in plan.nodes.iter_mut().zip(states) {
            node.local_position = state.local_position;
            node.local_rotation_deg = state.local_rotation_deg;
            node.local_scale = state.local_scale;
            node.local_matrix = state.local_matrix;
            node.world_matrix = state.world_matrix;
        }
        Ok(plan)
    }
}

fn plan_nodes(frame: &FrameEvaluation<'_>, graph: &SceneGraph) -> Result<Vec<PlanNode>, PlanError> {
    frame
        .nodes
        .nodes()
        .iter()
        .map(|state| -> Result<PlanNode, PlanError> {
            let node = graph
                .node(&state.node_id)
                .ok_or_else(|| PlanError::Unresolved {
                    kind: "node",
                    id: state.node_id.clone(),
                })?;
            Ok(PlanNode {
                id: state.node_id.clone(),
                kind: node.kind,
                parent: graph.parent(&state.node_id).map(plan_parent),
                local_position: state.local_position,
                local_rotation_deg: state.local_rotation_deg,
                local_scale: state.local_scale,
                local_matrix: state.local_matrix,
                world_matrix: state.world_matrix,
                render_order: state.render_order,
                asset_id: node.asset.as_ref().map(|asset| asset.asset_id.clone()),
                width_scene_units: node.width_scene_units,
                height_scene_units: node.height_scene_units,
            })
        })
        .collect()
}

/// The image planes back to front. The order is `Bundle::paint_order`'s (composition pass, then
/// `renderOrder`, ties in document order); only its inputs are copied next to it so the order can
/// be checked from the plan alone.
fn plan_layers(
    bundle: &Bundle,
    frame: &FrameEvaluation<'_>,
    definition_id: &str,
) -> Result<Vec<PlanLayer>, PlanError> {
    let ordered = bundle
        .paint_order(definition_id)
        .ok_or_else(|| PlanError::Unresolved {
            kind: "scene definition",
            id: definition_id.to_string(),
        })?;
    // Same rule as `paint_order`: the last pass that names a node wins.
    let pass_orders: HashMap<&str, i64> = bundle
        .composition_passes
        .iter()
        .map(|pass| (pass.node_id.as_str(), pass.order))
        .collect();
    ordered
        .into_iter()
        .map(|node| -> Result<PlanLayer, PlanError> {
            let asset_ref = node.asset.as_ref().ok_or_else(|| PlanError::Unresolved {
                kind: "asset reference",
                id: node.id.clone(),
            })?;
            let asset = bundle
                .asset(&asset_ref.asset_id)
                .ok_or_else(|| PlanError::Unresolved {
                    kind: "asset",
                    id: asset_ref.asset_id.clone(),
                })?;
            let state = frame
                .nodes
                .node(&node.id)
                .ok_or_else(|| PlanError::Unresolved {
                    kind: "node",
                    id: node.id.clone(),
                })?;
            Ok(PlanLayer {
                node_id: node.id.clone(),
                asset_id: asset.asset_id.clone(),
                alpha: asset.alpha,
                render_order: state.render_order,
                pass_order: pass_orders.get(node.id.as_str()).copied(),
            })
        })
        .collect()
}

/// The splat nodes, in document order, each with the asset it reads and the slot it takes among
/// the layers. Their matrices stay in `nodes`; the runtime draws them with the camera of the plan.
fn plan_splats(
    nodes: &[PlanNode],
    layers: &[PlanLayer],
    camera: &CameraState,
) -> Result<Vec<PlanSplat>, PlanError> {
    let splat_nodes: Vec<&PlanNode> = nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Splat)
        .collect();
    let ids: Vec<&str> = splat_nodes.iter().map(|node| node.id.as_str()).collect();
    let slots = derive_splat_slots(camera, nodes, layers, &ids);
    splat_nodes
        .into_iter()
        .zip(slots)
        .map(|(node, after_layers)| {
            let asset_id = node.asset_id.clone().ok_or_else(|| PlanError::Unresolved {
                kind: "asset reference",
                id: node.id.clone(),
            })?;
            Ok(PlanSplat {
                node_id: node.id.clone(),
                asset_id,
                after_layers,
            })
        })
        .collect()
}

/// The textures the image planes use and the splat files the splat nodes read, deduplicated by
/// `assetId` and ascending by it. The metadata comes from the bundle's `assets[]` record; the
/// importer guarantees that the node's own `assetRef` equals it (and that a plane reads a `png`,
/// a splat an `spz`).
fn plan_resources(bundle: &Bundle, nodes: &[PlanNode]) -> Result<Vec<PlanResource>, PlanError> {
    let mut resources: BTreeMap<String, PlanResource> = BTreeMap::new();
    for node in nodes
        .iter()
        .filter(|node| matches!(node.kind, NodeKind::ImagePlane | NodeKind::Splat))
    {
        let Some(asset_id) = &node.asset_id else {
            return Err(PlanError::Unresolved {
                kind: "asset reference",
                id: node.id.clone(),
            });
        };
        if let Some(resource) = resources.get_mut(asset_id) {
            resource.used_by.push(node.id.clone());
            continue;
        }
        let asset = bundle
            .asset(asset_id)
            .ok_or_else(|| PlanError::Unresolved {
                kind: "asset",
                id: asset_id.clone(),
            })?;
        resources.insert(
            asset_id.clone(),
            PlanResource {
                asset_id: asset.asset_id.clone(),
                revision_id: asset.revision_id.clone(),
                content_hash: asset.content_hash.clone(),
                relative_path: asset.relative_path.clone(),
                format: asset.format,
                width_px: asset.width_px,
                height_px: asset.height_px,
                alpha: asset.alpha,
                color_space: asset.color_space,
                used_by: vec![node.id.clone()],
            },
        );
    }
    Ok(resources.into_values().collect())
}
