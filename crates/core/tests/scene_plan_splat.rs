//! F048 — Gaussian splats in the directed-scene plan (S7, BL025 subset), through the public
//! library API only: no GPUI, no window, no runtime, no IO beyond the fixtures.
//!
//! Proved here: a splat node is *evaluated* (it becomes `ScenePlan::splats` with its world matrix
//! and its approved `spz` resource, and asks for the `splats` capability), the matrix and camera
//! are the evaluator's own values, the plan round-trips and its tampering is refused, a plan
//! without splats keeps its exact bytes, and what the plan still does not carry (lighting, weather,
//! rigs, data maps of a splat node) keeps being refused with a typed error.
//!
//! Not proved here: that Spark draws the splat (`runtime/test/directed-scene-browser.test.mjs`,
//! `crates/render/tests/scene_splat_frames.rs`) or that the file exists and is a real SPZ
//! (`tessel-editor-media`, F040).

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::{AssetFormat, DirectedScene, NodeKind};
use tessel_editor_core::scene_plan::{
    Capability, Domain, PlanError, PlanOptions, PlanSplat, PlanTimeline, ScenePlan,
};

const RECIPES: &str = include_str!(
    "../../../docs/migration/fixtures/bundles/valid-camera-recipes-layers.bundle.json"
);
const SOCKETS: &str = include_str!(
    "../../../docs/migration/fixtures/bundles/valid-graph-sockets-performance.bundle.json"
);

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn import(raw: Value) -> DirectedScene {
    DirectedScene::import_value(raw).unwrap_or_else(|error| panic!("import failed: {error:?}"))
}

fn definition_mut<'a>(doc: &'a mut Value, scene: &str) -> &'a mut Value {
    doc["sceneDefinitions"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|definition| definition["id"] == scene)
        .unwrap()
}

fn add_asset(doc: &mut Value, id: &str, path: &str) {
    doc["assets"].as_array_mut().unwrap().push(json!({
        "assetId": id,
        "revisionId": format!("{id}-r1"),
        "contentHash": format!("hash-{id}"),
        "relativePath": path,
        "format": "spz",
        "widthPx": 1,
        "heightPx": 1,
        "alphaMode": "opaque",
        "colorSpace": "srgb",
    }));
}

/// A splat node `node` under `parent` reading the `spz` asset `asset`, at `position`.
fn add_splat_node(
    doc: &mut Value,
    scene: &str,
    parent: &str,
    node: &str,
    asset: &str,
    position: [f64; 3],
) {
    definition_mut(doc, scene)["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": node,
            "parentId": parent,
            "type": "splat",
            "assetRef": {
                "assetId": asset,
                "revisionId": format!("{asset}-r1"),
                "contentHash": format!("hash-{asset}"),
                "relativePath": format!("splats/{asset}.spz"),
            },
            "localPosition": { "x": position[0], "y": position[1], "z": position[2] },
            "localRotation": { "x": 0, "y": 0, "z": 0, "w": 1 },
            "localScale": { "x": 1, "y": 1, "z": 1 },
        }));
}

fn set_node(doc: &mut Value, scene: &str, node: &str, key: &str, value: Value) {
    let nodes = definition_mut(doc, scene)["nodes"].as_array_mut().unwrap();
    nodes.iter_mut().find(|n| n["id"] == node).unwrap()[key] = value;
}

fn scale(x: f64, y: f64, z: f64) -> Value {
    json!({ "x": x, "y": y, "z": z })
}

/// A rotation of `degrees` about z, as the quaternion the bundle stores.
fn about_z(degrees: f64) -> Value {
    let half = degrees.to_radians() / 2.0;
    json!({ "x": 0, "y": 0, "z": half.sin(), "w": half.cos() })
}

fn with_splat(base: &str, scene: &str, parent: &str) -> Value {
    let mut doc = parse(base);
    add_asset(&mut doc, "cloud", "splats/cloud.spz");
    add_splat_node(
        &mut doc,
        scene,
        parent,
        "cloud-node",
        "cloud",
        [1.0, 2.0, -3.0],
    );
    doc
}

fn domains(error: &PlanError) -> Vec<(Domain, Option<&str>)> {
    match error {
        PlanError::Unsupported { domains } => domains
            .iter()
            .map(|entry| (entry.domain, entry.subject.as_deref()))
            .collect(),
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

fn tampered(plan: &ScenePlan, edit: impl FnOnce(&mut Value)) -> Result<ScenePlan, PlanError> {
    let mut wire = plan.to_json();
    edit(&mut wire);
    ScenePlan::from_json(&wire)
}

fn assert_invalid(result: Result<ScenePlan, PlanError>, pointer: &str) {
    match result {
        Err(PlanError::Invalid { pointer: found, .. }) => assert_eq!(found, pointer),
        other => panic!("expected invalid_plan at {pointer}, got {other:?}"),
    }
}

#[test]
fn a_splat_node_is_evaluated_with_its_matrix_resource_and_capability() {
    let scene = import(with_splat(RECIPES, "scene-a", "root"));
    let plan = ScenePlan::build(&scene, 0).expect("a splat is not an unsupported domain any more");

    assert!(plan.is_complete());
    assert!(plan.unsupported.is_empty());
    assert_eq!(
        plan.splats,
        [PlanSplat {
            node_id: "cloud-node".to_string(),
            asset_id: "cloud".to_string(),
            // (1, 2, -3) is farther from the camera than both planes (z -2 and 0).
            after_layers: 0,
        }]
    );
    assert!(plan.requires.contains(&Capability::Splats));
    // A splat is not an image plane: the paint order of the planes is unchanged.
    let layers: Vec<&str> = plan.layers.iter().map(|l| l.node_id.as_str()).collect();
    assert_eq!(layers, ["backdrop", "figure"]);

    // Its transform is the evaluator's, not a second computation.
    let evaluation = scene.bundle().evaluate_frame(0).unwrap();
    let evaluated = evaluation.nodes.node("cloud-node").unwrap();
    let node = plan.node("cloud-node").unwrap();
    assert_eq!(node.kind, NodeKind::Splat);
    assert_eq!(node.world_matrix, evaluated.world_matrix);
    assert_eq!(node.local_position, evaluated.local_position);
    assert_eq!(plan.camera, evaluation.camera);
    // Translation of the world matrix (column-major): the node sits at (1, 2, -3).
    assert_eq!(&node.world_matrix.0[12..15], &[1.0, 2.0, -3.0]);

    // The `spz` resource is listed, by assetId, between the pictures (ascending).
    let ids = plan.required_asset_ids();
    assert_eq!(ids, ["backdrop", "cloud", "figure"]);
    let resource = plan.resource("cloud").unwrap();
    assert_eq!(resource.format, AssetFormat::Spz);
    assert!(resource.is_splat());
    assert_eq!(resource.used_by, ["cloud-node"]);
    assert_eq!(resource.relative_path, "splats/cloud.spz");
    assert_eq!(
        (
            resource.revision_id.as_str(),
            resource.content_hash.as_str()
        ),
        ("cloud-r1", "hash-cloud")
    );
    assert!(!plan.resource("backdrop").unwrap().is_splat());

    // Wire form: additive `splats`, lossless.
    let wire = plan.to_json();
    assert_eq!(
        wire["splats"],
        json!([{ "nodeId": "cloud-node", "assetId": "cloud", "afterLayers": 0 }])
    );
    assert!(
        wire["requires"]
            .as_array()
            .unwrap()
            .contains(&json!("splats"))
    );
    assert_eq!(wire["complete"], true);
    assert_eq!(ScenePlan::from_json(&wire), Ok(plan.clone()));
    assert_eq!(ScenePlan::from_json_str(&plan.to_json_string()), Ok(plan));
}

#[test]
fn a_splat_is_painted_after_the_planes_that_are_not_nearer_to_the_camera() {
    // Camera at z~9 looking at the origin; the planes (plan order) are `backdrop` (0, 0, -2) and
    // `figure` (0.5, 0, 0). The splat shares the figure's x so equal z is equal depth.
    let slot_of = |z: f64| -> usize {
        let mut doc = parse(RECIPES);
        add_asset(&mut doc, "cloud", "splats/cloud.spz");
        add_splat_node(
            &mut doc,
            "scene-a",
            "root",
            "cloud-node",
            "cloud",
            [0.5, 0.0, z],
        );
        let plan = ScenePlan::build(&import(doc), 0).unwrap();
        assert_eq!(plan.layers.len(), 2);
        // The wire form carries the slot and the reader checks it from the plan alone.
        assert_eq!(
            plan.to_json()["splats"][0]["afterLayers"],
            json!(plan.splats[0].after_layers)
        );
        assert_eq!(ScenePlan::from_json(&plan.to_json()), Ok(plan.clone()));
        plan.splats[0].after_layers
    };
    assert_eq!(slot_of(-5.0), 0, "behind both planes");
    assert_eq!(slot_of(-1.0), 1, "between the planes");
    assert_eq!(slot_of(3.0), 2, "in front of both planes");
    // Equal depth is not nearer: a splat on the figure's own plane is painted after it.
    assert_eq!(slot_of(0.0), 2);
}

#[test]
fn a_plan_without_splats_keeps_its_wire_form_and_asks_for_no_splats() {
    let plan = ScenePlan::build(&import(parse(RECIPES)), 0).unwrap();
    assert!(plan.splats.is_empty());
    assert!(!plan.requires.contains(&Capability::Splats));
    let wire = plan.to_json();
    assert!(
        wire.get("splats").is_none(),
        "the additive key is written only when there are splats (golden bytes of the planner)"
    );
    // The reader accepts the absent key and an explicit empty one, and refuses an inconsistent one.
    assert_eq!(ScenePlan::from_json(&wire), Ok(plan.clone()));
    let mut explicit = wire.clone();
    explicit["splats"] = json!([]);
    assert_eq!(ScenePlan::from_json(&explicit), Ok(plan.clone()));
    let mut invented = wire;
    invented["splats"] = json!([{ "nodeId": "backdrop", "assetId": "backdrop", "afterLayers": 0 }]);
    assert_invalid(ScenePlan::from_json(&invented), "/splats");
}

#[test]
fn the_splat_follows_the_performance_channel_of_its_parent_frame_by_frame() {
    // `stage` is lifted by 0.5 over instance frames 12..36 (an event channel with holdFinal).
    let scene = import(with_splat(SOCKETS, "scene-rig", "stage"));
    let timeline = PlanTimeline::new(&scene);
    let y_of = |frame: i64| {
        let plan = timeline.build(frame).unwrap();
        assert_eq!(
            plan,
            ScenePlan::build(&scene, frame).unwrap(),
            "frame {frame}"
        );
        plan.node("cloud-node").unwrap().world_matrix.0[13]
    };
    let before = y_of(0);
    let middle = y_of(24);
    let after = y_of(47);
    // The cloud sits 2 units above `stage`, which is scaled by 2 (y 4); the lift is 0.5 in the
    // parent's space (`root`, unscaled).
    assert_eq!(before, 4.0);
    assert!((middle - 4.25).abs() < 1e-9, "half way: {middle}");
    assert_eq!(after, 4.5);
    // Out of order and repeated, the answer depends on the frame only.
    assert_eq!(y_of(24), middle);
    assert_eq!(y_of(0), before);
}

#[test]
fn two_nodes_may_share_one_splat_file() {
    let mut doc = with_splat(RECIPES, "scene-a", "root");
    add_splat_node(
        &mut doc,
        "scene-a",
        "root",
        "cloud-twin",
        "cloud",
        [0.0, 0.0, 0.0],
    );
    let plan = ScenePlan::build(&import(doc), 0).unwrap();
    let nodes: Vec<&str> = plan.splats.iter().map(|s| s.node_id.as_str()).collect();
    assert_eq!(nodes, ["cloud-node", "cloud-twin"], "document order");
    assert_eq!(
        plan.resource("cloud").unwrap().used_by,
        ["cloud-node", "cloud-twin"]
    );
    assert_eq!(
        plan.resources
            .iter()
            .filter(|r| r.asset_id == "cloud")
            .count(),
        1
    );
    assert_eq!(ScenePlan::from_json(&plan.to_json()), Ok(plan));
}

#[test]
fn light_weather_and_rigs_stay_refused_even_with_a_splat_in_the_scene() {
    let mut doc = with_splat(RECIPES, "scene-a", "root");
    let definition = definition_mut(&mut doc, "scene-a").as_object_mut().unwrap();
    definition.insert("weather".into(), json!("rain"));
    definition.insert("lighting".into(), json!({ "ambient": 0.5, "lights": [] }));
    definition.insert("rigs".into(), json!([{ "id": "rig-1" }]));
    let scene = import(doc);
    let error = ScenePlan::build(&scene, 0).unwrap_err();
    let refused: Vec<Domain> = domains(&error)
        .into_iter()
        .map(|(domain, _)| domain)
        .collect();
    for domain in [Domain::Weather, Domain::Lighting, Domain::Rigs] {
        assert!(refused.contains(&domain), "{domain:?} in {refused:?}");
    }
    assert_eq!(error.code(), "unsupported");

    // The partial plan (an explicit diagnostic decision) is still not "the picture", and it
    // still draws the splat as a *splat*: the unsupported list is what the viewer must not hide.
    let partial = ScenePlan::build_with(
        &scene,
        0,
        PlanOptions {
            allow_partial: true,
        },
    )
    .unwrap();
    assert!(!partial.is_complete());
    assert_eq!(partial.splats.len(), 1);
    assert_eq!(ScenePlan::from_json(&partial.to_json()), Ok(partial));
}

#[test]
fn data_maps_and_binding_declared_on_the_splat_node_are_refused_by_name() {
    for (section, domain) in [
        ("rigBind", Domain::RigBind),
        ("reveal", Domain::Reveal),
        ("sceneDepth", Domain::SceneDepth),
    ] {
        let mut doc = with_splat(RECIPES, "scene-a", "root");
        let nodes = definition_mut(&mut doc, "scene-a")["nodes"]
            .as_array_mut()
            .unwrap();
        let splat = nodes.iter_mut().find(|n| n["id"] == "cloud-node").unwrap();
        splat[section] = json!({ "declared": true });
        let scene = import(doc);
        let error = ScenePlan::build(&scene, 0).unwrap_err();
        assert_eq!(
            domains(&error),
            [(domain, Some("cloud-node"))],
            "{section} on a splat node"
        );
    }
}

#[test]
fn a_plan_that_lies_about_its_splats_is_refused_with_the_pointer() {
    let plan = ScenePlan::build(&import(with_splat(RECIPES, "scene-a", "root")), 0).unwrap();

    // The splat node is dropped from the list.
    assert_invalid(
        tampered(&plan, |wire| wire["splats"] = json!([])),
        "/splats",
    );
    // The slot is derived from the camera depth of the node origins; another one is a lie.
    assert_invalid(
        tampered(&plan, |wire| wire["splats"][0]["afterLayers"] = json!(1)),
        "/splats/0/afterLayers",
    );
    assert_invalid(
        tampered(&plan, |wire| wire["splats"][0]["afterLayers"] = json!(-1)),
        "/splats/0/afterLayers",
    );
    // The splat names another asset than its node.
    assert_invalid(
        tampered(&plan, |wire| wire["splats"][0]["assetId"] = json!("figure")),
        "/splats/0/assetId",
    );
    // The splat names a picture, not an spz.
    assert_invalid(
        tampered(&plan, |wire| {
            wire["nodes"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .for_each(|node| {
                    if node["id"] == "cloud-node" {
                        node["assetId"] = json!("figure");
                    }
                });
            wire["splats"][0]["assetId"] = json!("figure");
        }),
        "/splats/0/assetId",
    );
    // `requires` hides the capability the splats need.
    assert_invalid(
        tampered(&plan, |wire| {
            let requires = wire["requires"].as_array_mut().unwrap();
            requires.retain(|name| name != "splats");
        }),
        "/requires",
    );
    // The resource claims to be a picture while a splat reads it.
    assert_invalid(
        tampered(&plan, |wire| {
            for resource in wire["resources"].as_array_mut().unwrap() {
                if resource["assetId"] == "cloud" {
                    resource["format"] = json!("png");
                }
            }
        }),
        "/splats/0/assetId",
    );
    // An image plane (and its layer) reads the spz of a splat: only the runtime used to refuse it.
    assert_invalid(
        tampered(&plan, |wire| {
            for node in wire["nodes"].as_array_mut().unwrap() {
                if node["id"] == "figure" {
                    node["assetId"] = json!("cloud");
                }
            }
            for layer in wire["layers"].as_array_mut().unwrap() {
                if layer["nodeId"] == "figure" {
                    layer["assetId"] = json!("cloud");
                }
            }
        }),
        "/layers/1/assetId",
    );
    // A splat that is also drawn as a layer.
    assert_invalid(
        tampered(&plan, |wire| {
            wire["layers"].as_array_mut().unwrap().push(json!({
                "nodeId": "cloud-node", "assetId": "cloud", "alpha": "straight",
                "renderOrder": 0, "passOrder": null,
            }));
        }),
        "/layers",
    );
    // The retired `splat` domain no longer parses: a plan that still listed it is not this version's.
    assert_invalid(
        tampered(&plan, |wire| {
            wire["unsupported"] = json!([{ "domain": "splat", "subject": "cloud-node" }]);
            wire["complete"] = json!(false);
        }),
        "/unsupported/0/domain",
    );
    assert_eq!(plan.validate(), Ok(()), "the original is untouched");
}

#[test]
fn a_runtime_without_the_splats_capability_refuses_the_plan_by_name() {
    let plan = ScenePlan::build(&import(with_splat(RECIPES, "scene-a", "root")), 0).unwrap();
    let before_s7: Vec<Capability> = Capability::ALL
        .into_iter()
        .filter(|capability| *capability != Capability::Splats)
        .collect();
    assert_eq!(plan.missing_capabilities(&before_s7), [Capability::Splats]);
    let error = plan.require_capabilities(&before_s7).unwrap_err();
    assert_eq!(error.code(), "unsupported");
    assert_eq!(error.details().unwrap()["missing"], json!(["splats"]));
    assert_eq!(plan.require_capabilities(&Capability::ALL), Ok(()));
    // A scene without splats still runs on such a runtime.
    let plain = ScenePlan::build(&import(parse(RECIPES)), 0).unwrap();
    assert_eq!(plain.require_capabilities(&before_s7), Ok(()));
}

#[test]
fn timeline_recomputes_splat_order_when_the_camera_orbits_past_the_planes() {
    let mut doc = parse(RECIPES);
    add_asset(&mut doc, "cloud", "splats/cloud.spz");
    add_splat_node(
        &mut doc,
        "scene-a",
        "root",
        "cloud-node",
        "cloud",
        [0.5, 0.0, -1.0],
    );
    doc["cameraClips"][0]["segments"] = json!([{
        "recipe":"orbit", "durationFrames":120, "from":"initial",
        "yawDeg":180, "pitchDeg":0, "easing":"linear"
    }]);
    doc["cameraClips"][0]["layers"] = json!([]);
    let scene = import(doc);
    let timeline = PlanTimeline::new(&scene);
    let mut slots = std::collections::BTreeSet::new();
    for frame in [0, 30, 60, 90, 119] {
        let plan = timeline.build(frame).unwrap();
        plan.validate().unwrap();
        assert_eq!(plan, ScenePlan::build(&scene, frame).unwrap());
        slots.insert(plan.splats[0].after_layers);
    }
    assert!(
        slots.len() > 1,
        "camera must cross a depth ordering boundary"
    );
}

fn refused_transform(error: PlanError, node: &str) -> &'static str {
    match error {
        PlanError::SplatTransform { node_id, reason } => {
            assert_eq!(node_id, node);
            reason
        }
        other => panic!("expected SplatTransform, got {other:?}"),
    }
}

/// BL123: Spark keeps one uniform scale per splat (`(sx + sy + sz) / 3`, no shear), so a plan whose
/// splat matrix is anything else than a similarity is refused with the node and the reason, from
/// the node's own scale or from an ancestor's, instead of drawn approximated.
#[test]
fn a_splat_whose_matrix_is_not_a_similarity_is_refused_never_approximated() {
    let cases: [(&str, &str, Value, &str); 5] = [
        (
            "own non-uniform scale",
            "cloud-node",
            scale(2.0, 1.0, 1.0),
            "the scale is not uniform",
        ),
        (
            "own mirror",
            "cloud-node",
            scale(-1.0, 1.0, 1.0),
            "the transform is mirrored",
        ),
        (
            "mirrored parent",
            "root",
            scale(-1.0, 1.0, 1.0),
            "the transform is mirrored",
        ),
        (
            "non-uniform parent",
            "root",
            scale(1.0, 3.0, 1.0),
            "the scale is not uniform",
        ),
        (
            "collapsed axis",
            "cloud-node",
            scale(1.0, 0.0, 1.0),
            "the scale is degenerate",
        ),
    ];
    for (name, target, value, reason) in cases {
        let mut doc = with_splat(RECIPES, "scene-a", "root");
        set_node(&mut doc, "scene-a", target, "localScale", value);
        let scene = import(doc);
        let error = ScenePlan::build(&scene, 0).unwrap_err();
        assert_eq!(error.code(), "unsupported", "{name}");
        assert_eq!(error.details().unwrap()["node"], "cloud-node", "{name}");
        assert_eq!(refused_transform(error, "cloud-node"), reason, "{name}");
        // The timeline answers like `build`, and a partial plan is no way around it.
        assert_eq!(
            PlanTimeline::new(&scene).build(0).unwrap_err().to_string(),
            ScenePlan::build(&scene, 0).unwrap_err().to_string(),
            "{name}"
        );
        let partial = ScenePlan::build_with(
            &scene,
            0,
            PlanOptions {
                allow_partial: true,
            },
        );
        assert!(
            matches!(partial, Err(PlanError::SplatTransform { .. })),
            "{name}"
        );
    }

    // A parent that scales non-uniformly while the child is rotated shears the child: refused too.
    let mut doc = with_splat(RECIPES, "scene-a", "root");
    set_node(
        &mut doc,
        "scene-a",
        "root",
        "localScale",
        scale(1.0, 2.0, 1.0),
    );
    set_node(
        &mut doc,
        "scene-a",
        "cloud-node",
        "localRotation",
        about_z(45.0),
    );
    let error = ScenePlan::build(&import(doc), 0).unwrap_err();
    assert!(
        matches!(error, PlanError::SplatTransform { .. }),
        "{error:?}"
    );
}

#[test]
fn rotation_with_uniform_positive_scale_and_two_mirrors_are_still_drawn() {
    // Uniform scale 2.5 on the node and 2 on the parent, a rotated node: a similarity.
    let mut doc = with_splat(RECIPES, "scene-a", "root");
    set_node(
        &mut doc,
        "scene-a",
        "root",
        "localScale",
        scale(2.0, 2.0, 2.0),
    );
    set_node(
        &mut doc,
        "scene-a",
        "cloud-node",
        "localScale",
        scale(2.5, 2.5, 2.5),
    );
    set_node(
        &mut doc,
        "scene-a",
        "cloud-node",
        "localRotation",
        about_z(37.0),
    );
    let plan = ScenePlan::build(&import(doc), 0).expect("rotation + uniform scale is a similarity");
    assert_eq!(plan.validate(), Ok(()));
    // Two mirrors make a half turn (determinant +1): not a reflection.
    let mut doc = with_splat(RECIPES, "scene-a", "root");
    set_node(
        &mut doc,
        "scene-a",
        "root",
        "localScale",
        scale(-1.0, 1.0, 1.0),
    );
    set_node(
        &mut doc,
        "scene-a",
        "cloud-node",
        "localScale",
        scale(-1.0, 1.0, 1.0),
    );
    ScenePlan::build(&import(doc), 0).expect("a double mirror is a rotation");
}

#[test]
fn the_reader_refuses_a_splat_matrix_that_is_not_a_similarity_and_the_timeline_checks_each_frame() {
    // A plan from outside whose splat matrix was stretched after the fact.
    let plan = ScenePlan::build(&import(with_splat(RECIPES, "scene-a", "root")), 0).unwrap();
    let stretched = tampered(&plan, |wire| {
        for node in wire["nodes"].as_array_mut().unwrap() {
            if node["id"] == "cloud-node" {
                node["worldMatrix"][0] = json!(2.0);
            }
        }
    });
    assert!(
        matches!(stretched, Err(PlanError::SplatTransform { .. })),
        "{stretched:?}"
    );
    assert_eq!(plan.validate(), Ok(()));

    // This fixed stretched hierarchy is refused at every sampled frame. In general, animated
    // rotations can change similarity even with static scales, so the timeline checks each frame.
    let mut doc = with_splat(SOCKETS, "scene-rig", "arm");
    set_node(
        &mut doc,
        "scene-rig",
        "stage",
        "localScale",
        scale(2.0, 1.0, 1.0),
    );
    let scene = import(doc);
    let timeline = PlanTimeline::new(&scene);
    for frame in [24, 0, 47, 12] {
        let fresh = ScenePlan::build(&scene, frame);
        assert!(
            matches!(fresh, Err(PlanError::SplatTransform { .. })),
            "frame {frame}"
        );
        assert_eq!(timeline.build(frame), fresh, "frame {frame}");
    }
}

/// BL124: a layer on or behind the camera is invisible, so it never counts as "nearer".
#[test]
fn a_plane_behind_the_camera_does_not_take_the_splats_slot() {
    // Camera (2, 1, 9) looking at (0, 1, 0). Layers in plan order: `backdrop`, `figure`.
    let slot_of = |backdrop: [f64; 3], figure: [f64; 3], splat: [f64; 3]| -> usize {
        let at = |p: [f64; 3]| json!({ "x": p[0], "y": p[1], "z": p[2] });
        let mut doc = with_splat(RECIPES, "scene-a", "root");
        set_node(
            &mut doc,
            "scene-a",
            "backdrop",
            "localPosition",
            at(backdrop),
        );
        set_node(&mut doc, "scene-a", "figure", "localPosition", at(figure));
        set_node(
            &mut doc,
            "scene-a",
            "cloud-node",
            "localPosition",
            at(splat),
        );
        let plan = ScenePlan::build(&import(doc), 0).unwrap();
        assert_eq!(
            plan.validate(),
            Ok(()),
            "the validator derives the same slot"
        );
        assert_eq!(ScenePlan::from_json(&plan.to_json()), Ok(plan.clone()));
        plan.splats[0].after_layers
    };
    // The review's case: `backdrop` is behind the camera (z 12), `figure` (z -2) is opaque and
    // farther than the splat (z 0). The splat is painted after both: 2, not 0.
    assert_eq!(
        slot_of([0.0, 0.0, 12.0], [0.5, 0.0, -2.0], [0.5, 0.0, 0.0]),
        2
    );
    // Only the plane in front of the camera counts: `figure` (z 0) is nearer than a splat at z -5.
    assert_eq!(
        slot_of([0.0, 0.0, 12.0], [0.5, 0.0, 0.0], [0.5, 0.0, -5.0]),
        1
    );
    // Control: with both planes in front the first layer that is nearer decides, as before.
    assert_eq!(
        slot_of([0.0, 0.0, -2.0], [0.5, 0.0, 0.0], [0.5, 0.0, -5.0]),
        0
    );
    // A splat that is itself behind the camera has nothing in front of it.
    assert_eq!(
        slot_of([0.0, 0.0, -2.0], [0.5, 0.0, 0.0], [0.5, 0.0, 12.0]),
        2
    );
}

#[test]
fn the_timeline_ignores_a_plane_the_camera_flies_past() {
    // The camera dollies 12 units along its view: it starts in front of both planes and ends past
    // them, with the splat (farthest) still ahead of it only part of the way.
    let mut doc = with_splat(RECIPES, "scene-a", "root");
    set_node(
        &mut doc,
        "scene-a",
        "cloud-node",
        "localPosition",
        json!({ "x": 0.0, "y": 1.0, "z": -6.0 }),
    );
    doc["cameraClips"][0]["segments"] = json!([
        { "recipe": "dollyIn", "durationFrames": 100, "from": "initial", "distanceSceneUnits": 12.0, "easing": "linear" }
    ]);
    doc["cameraClips"][0]["layers"] = json!([]);
    let scene = import(doc);
    let timeline = PlanTimeline::new(&scene);
    let mut slots = Vec::new();
    for frame in [0, 20, 40, 60, 80, 99, 10, 70] {
        let plan = timeline.build(frame).unwrap();
        assert_eq!(plan.validate(), Ok(()), "frame {frame}");
        assert_eq!(
            Ok(plan.clone()),
            ScenePlan::build(&scene, frame),
            "frame {frame}"
        );
        slots.push((frame, plan.splats[0].after_layers));
    }
    // Before the flight the splat (z -6) is behind both planes; after it every plane and the splat
    // are behind the camera and nothing is "nearer": the slot is the end of the layers.
    assert_eq!(slots[0], (0, 0));
    assert_eq!(slots[5], (99, 2));
    assert!(slots.iter().any(|(_, slot)| *slot == 0) && slots.iter().any(|(_, slot)| *slot == 2));
}
