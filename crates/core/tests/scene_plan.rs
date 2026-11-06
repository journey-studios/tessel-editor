//! F037 — the evaluated plan of a directed-scene frame (BB05, first slice), through the public
//! library API only: no GPUI, no window, no runtime, no socket, no CLI, no IO beyond the fixtures.
//!
//! What is proved here, and what is not:
//!
//! - **The plan is the evaluator's frame, not a second evaluation.** On the four valid corpus
//!   bundles (`docs/migration/fixtures`, whose camera and node values are already compared with
//!   the origin by `directed_scene_temporal.rs` and `directed_scene_animation.rs`), every frame of
//!   the plan equals what `Bundle::evaluate_frame` returns, bit for bit. This proves the plan
//!   carries the evaluation unchanged; equivalence with the origin stays with those two suites.
//! - **Behaviour of the plan itself**, on bundles written for these tests: draw order, resource
//!   list, requires, refusals of undeclared domains, uninterpreted fields, determinism, JSON
//!   round trip and the refusal of tampered plans. These prove this module, not the origin.
//! - **Not proved:** that a renderer draws the plan (there is none yet: BL007), that a resource
//!   file exists (BL009), and the CLI/UI surfaces (none yet).

use serde_json::{Value, json};
use tessel_editor_core::CommandError;
use tessel_editor_core::directed_scene::{
    AssetAlpha, DirectedScene, NodeAnimation, NodeKind, TemporalError,
};
use tessel_editor_core::scene_plan::{
    Capability, Domain, PLAN_KIND, PLAN_VERSION, PlanError, PlanOptions, ScenePlan,
    UninterpretedReason, capabilities,
};
use tessel_editor_core::time::Ratio;

const SOCKETS: &str = include_str!(
    "../../../docs/migration/fixtures/bundles/valid-graph-sockets-performance.bundle.json"
);
const OFFSETS: &str =
    include_str!("../../../docs/migration/fixtures/bundles/valid-cut-instance-offsets.bundle.json");
const RECIPES: &str = include_str!(
    "../../../docs/migration/fixtures/bundles/valid-camera-recipes-layers.bundle.json"
);
const MINIMAL: &str =
    include_str!("../../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json");
const LAYERED: &str = include_str!("directed_scene_fixtures/layered_scene.json");

// ---- helpers -------------------------------------------------------------------------------

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn import(raw: Value) -> DirectedScene {
    DirectedScene::import_value(raw).unwrap_or_else(|error| panic!("import failed: {error:?}"))
}

fn node_mut<'a>(doc: &'a mut Value, scene: &str, node: &str) -> &'a mut Value {
    let definition = doc["sceneDefinitions"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|definition| definition["id"] == scene)
        .unwrap();
    definition["nodes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|candidate| candidate["id"] == node)
        .unwrap()
}

/// The synthetic layered scene without the sections a plan refuses (weather, lighting, the surface
/// map of the hero), so it is a *complete* frame. It keeps its socket, performance channel,
/// composition pass, unknown fields and 24000/1001 frame rate.
fn layered_complete() -> Value {
    let mut doc = parse(LAYERED);
    let definition = doc["sceneDefinitions"][0].as_object_mut().unwrap();
    definition.remove("weather");
    definition.remove("lighting");
    node_mut(&mut doc, "street", "hero-plane")
        .as_object_mut()
        .unwrap()
        .remove("surface");
    // The surface map was the only reader of this linear-srgb asset; an unread one is refused.
    doc["assets"]
        .as_array_mut()
        .unwrap()
        .retain(|asset| asset["assetId"] != "hero-surface");
    doc
}

fn plan_of(scene: &DirectedScene, frame: i64) -> ScenePlan {
    ScenePlan::build(scene, frame).unwrap_or_else(|error| panic!("frame {frame}: {error}"))
}

fn layer_ids(plan: &ScenePlan) -> Vec<&str> {
    plan.layers
        .iter()
        .map(|layer| layer.node_id.as_str())
        .collect()
}

fn pointers(plan: &ScenePlan) -> Vec<(&str, UninterpretedReason)> {
    plan.uninterpreted
        .iter()
        .map(|field| (field.pointer.as_str(), field.reason))
        .collect()
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

/// A plan's JSON after `edit`, read back.
fn tampered(plan: &ScenePlan, edit: impl FnOnce(&mut Value)) -> Result<ScenePlan, PlanError> {
    let mut wire = plan.to_json();
    edit(&mut wire);
    ScenePlan::from_json(&wire)
}

fn assert_invalid(result: Result<ScenePlan, PlanError>, pointer_prefix: &str) {
    match result {
        Err(PlanError::Invalid { pointer, message }) => assert!(
            pointer.starts_with(pointer_prefix),
            "pointer `{pointer}` should start with `{pointer_prefix}` ({message})"
        ),
        other => panic!("expected Invalid at `{pointer_prefix}`, got {other:?}"),
    }
}

fn frames_of(scene: &DirectedScene) -> Vec<i64> {
    let duration = i64::try_from(scene.bundle().edit_duration_frames).unwrap();
    vec![0, 1, duration / 2, duration - 1]
}

// ---- the plan carries the evaluation unchanged -----------------------------------------------

#[test]
fn every_corpus_frame_of_the_plan_equals_the_evaluator() {
    for (name, text) in [
        ("sockets-performance", SOCKETS),
        ("cut-instance-offsets", OFFSETS),
        ("camera-recipes-layers", RECIPES),
        ("minimal-continuous", MINIMAL),
    ] {
        let scene = import(parse(text));
        for frame in frames_of(&scene) {
            let plan = plan_of(&scene, frame);
            let evaluation = scene.bundle().evaluate_frame(frame).unwrap();
            let label = format!("{name} frame {frame}");

            assert!(plan.is_complete(), "{label}");
            assert_eq!(plan.version, PLAN_VERSION, "{label}");
            assert_eq!(plan.camera, evaluation.camera, "{label}");
            assert_eq!(plan.animation, evaluation.node_animation, "{label}");
            assert_eq!(
                plan.frame.edit_frame, evaluation.mapping.edit_frame,
                "{label}"
            );
            assert_eq!(plan.frame.shot_id, evaluation.mapping.shot.id, "{label}");
            assert_eq!(
                plan.frame.instance_frame, evaluation.mapping.instance_frame,
                "{label}"
            );
            assert_eq!(
                plan.frame.camera_clip_frame, evaluation.mapping.camera_clip_frame,
                "{label}"
            );
            assert_eq!(
                plan.frame.edit_time, evaluation.mapping.edit_time,
                "{label}"
            );
            assert_eq!(
                plan.frame.instance_time, evaluation.mapping.instance_time,
                "{label}"
            );
            assert_eq!(
                plan.source.scene_instance_id, evaluation.mapping.instance.id,
                "{label}"
            );
            assert_eq!(plan.nodes.len(), evaluation.nodes.nodes().len(), "{label}");
            for (planned, state) in plan.nodes.iter().zip(evaluation.nodes.nodes()) {
                assert_eq!(planned.id, state.node_id, "{label}");
                assert_eq!(planned.local_position, state.local_position, "{label}");
                assert_eq!(
                    planned.local_rotation_deg, state.local_rotation_deg,
                    "{label}"
                );
                assert_eq!(planned.local_scale, state.local_scale, "{label}");
                assert_eq!(planned.local_matrix, state.local_matrix, "{label}");
                assert_eq!(planned.world_matrix, state.world_matrix, "{label}");
                assert_eq!(planned.render_order, state.render_order, "{label}");
            }
            assert_eq!(plan.validate(), Ok(()), "{label}");
        }
    }
}

#[test]
fn the_plan_reads_hierarchy_sockets_and_performance_from_the_graph() {
    let scene = import(parse(SOCKETS));
    let plan = plan_of(&scene, 20);

    assert_eq!(plan.source.bundle_id, scene.bundle().bundle_id);
    assert_eq!(plan.source.scene_definition_id, "scene-rig");
    assert_eq!(plan.source.scene_instance_id, "inst-a");
    assert_eq!((plan.output.width, plan.output.height), (320, 180));
    assert_eq!(plan.output.alpha_mode, AssetAlpha::Straight);

    // `pen` hangs from the `hand` socket of `arm`; `arm` from `stage`; the matrices already
    // include the socket and the three performance channels.
    let pen = plan.node("pen").unwrap();
    let parent = pen.parent.as_ref().unwrap();
    assert_eq!(
        (parent.node_id.as_str(), parent.socket.as_deref()),
        ("arm", Some("hand"))
    );
    assert_eq!(plan.node("root").unwrap().parent, None);
    assert_eq!(pen.kind, NodeKind::ImagePlane);
    assert_eq!(
        plan.animation,
        NodeAnimation::Animated {
            track_ids: vec!["perf-1".to_string()],
            channels: 3,
        }
    );
    assert!(plan.requires.contains(&Capability::Sockets));
    assert!(plan.requires.contains(&Capability::PerformanceChannels));
}

#[test]
fn times_are_exact_at_a_fractional_frame_rate() {
    let scene = import(layered_complete());
    let plan = plan_of(&scene, 12);
    // 12 frames of 24000/1001 fps: 12 * 1001 / 24000 = 1001/2000 s, not a rounded float.
    assert_eq!(plan.frame.edit_time, Ratio::new(1001, 2000).unwrap());
    assert_eq!(plan.frame.instance_time, plan.frame.edit_time);
    assert_eq!(
        (plan.output.fps.numerator, plan.output.fps.denominator),
        (24000, 1001)
    );
}

// ---- draw order, transparency, resources -----------------------------------------------------

#[test]
fn layers_are_back_to_front_with_the_composition_pass_first() {
    let scene = import(layered_complete());
    let plan = plan_of(&scene, 12);

    // hero-plane: renderOrder 1, but the pass `hero-first` puts it at -1; bg-plane and
    // prop-plane tie at 0 and keep the document order of the nodes.
    assert_eq!(layer_ids(&plan), ["hero-plane", "bg-plane", "prop-plane"]);
    let hero = &plan.layers[0];
    assert_eq!((hero.render_order, hero.pass_order), (1, Some(-1)));
    assert_eq!(hero.effective_order(), -1);
    assert_eq!(plan.layers[1].pass_order, None);

    // The order is the graph's own `paint_order`, not a second implementation.
    let painted: Vec<&str> = scene
        .bundle()
        .paint_order("street")
        .unwrap()
        .into_iter()
        .map(|node| node.id.as_str())
        .collect();
    assert_eq!(layer_ids(&plan), painted);
    // Every capability but `splats`: this scene has no splat node (S7, `scene_plan_splat.rs`).
    let without_splats: Vec<Capability> = Capability::ALL
        .into_iter()
        .filter(|capability| *capability != Capability::Splats)
        .collect();
    assert_eq!(plan.requires, without_splats);
}

#[test]
fn equal_orders_keep_the_node_order_and_a_lower_order_moves_a_layer_back() {
    let mut doc = parse(RECIPES);
    let scene = import(doc.clone());
    // backdrop 0, figure 1.
    assert_eq!(layer_ids(&plan_of(&scene, 0)), ["backdrop", "figure"]);

    node_mut(&mut doc, "scene-a", "figure")["renderOrder"] = json!(0);
    assert_eq!(
        layer_ids(&plan_of(&import(doc.clone()), 0)),
        ["backdrop", "figure"]
    );

    node_mut(&mut doc, "scene-a", "figure")["renderOrder"] = json!(-5);
    assert_eq!(layer_ids(&plan_of(&import(doc), 0)), ["figure", "backdrop"]);
}

#[test]
fn transparency_follows_the_asset_alpha_and_the_render_profile() {
    let mut doc = parse(SOCKETS);
    let plan = plan_of(&import(doc.clone()), 0);
    let by_node = |id: &str| {
        plan.layers
            .iter()
            .find(|layer| layer.node_id == id)
            .unwrap()
    };
    assert_eq!(by_node("backdrop").alpha, AssetAlpha::Opaque);
    assert_eq!(by_node("pen").alpha, AssetAlpha::Straight);
    assert!(plan.requires.contains(&Capability::OpaqueAlpha));
    assert!(plan.requires.contains(&Capability::StraightAlpha));

    // An opaque render profile and an all-straight scene: the plan says so, and no longer asks a
    // runtime for opaque layers.
    doc["renderProfile"]["alphaMode"] = json!("opaque");
    doc["assets"][0]["alphaMode"] = json!("straight");
    let plan = plan_of(&import(doc), 0);
    assert_eq!(plan.output.alpha_mode, AssetAlpha::Opaque);
    assert!(
        plan.layers
            .iter()
            .all(|layer| layer.alpha == AssetAlpha::Straight)
    );
    assert!(!plan.requires.contains(&Capability::OpaqueAlpha));
}

#[test]
fn resources_are_stable_and_identified_by_asset_id_not_by_path() {
    let scene = import(parse(SOCKETS));
    let plan = plan_of(&scene, 0);
    assert_eq!(plan.required_asset_ids(), ["backdrop", "figure"]);
    let figure = plan.resource("figure").unwrap();
    assert_eq!(figure.used_by, ["pen"]);
    assert_eq!(figure.relative_path, "assets/figure.png");
    assert_eq!((figure.width_px, figure.height_px), (32, 32.0));
    assert_eq!(figure.alpha, AssetAlpha::Straight);

    // Same bundle, assets declared in the other order and moved to another folder: the list of
    // identities is the same, the path is only metadata.
    let mut doc = parse(SOCKETS);
    doc["assets"].as_array_mut().unwrap().reverse();
    let moved = doc
        .to_string()
        .replace("assets/figure.png", "other/dir/figure.png");
    let moved_plan = plan_of(&import(parse(&moved)), 0);
    assert_eq!(moved_plan.required_asset_ids(), plan.required_asset_ids());
    assert_eq!(
        moved_plan.resource("figure").unwrap().relative_path,
        "other/dir/figure.png"
    );
    assert_eq!(moved_plan.layers, plan.layers);
}

#[test]
fn an_asset_two_planes_share_is_listed_once_with_both_users() {
    let mut doc = parse(RECIPES);
    // `figure` now uses the backdrop's asset too: same `assetRef`, so the bundle stays valid.
    let backdrop_ref = node_mut(&mut doc, "scene-a", "backdrop")["assetRef"].clone();
    node_mut(&mut doc, "scene-a", "figure")["assetRef"] = backdrop_ref;
    let plan = plan_of(&import(doc), 0);
    assert_eq!(plan.required_asset_ids(), ["backdrop"]);
    assert_eq!(
        plan.resource("backdrop").unwrap().used_by,
        ["backdrop", "figure"]
    );
    assert_eq!(plan.layers.len(), 2);
}

#[test]
fn the_plan_never_carries_an_absolute_path() {
    // The original layered scene is refused (surface/lighting), so read its partial plan.
    let scene = import(parse(LAYERED));
    let plan = ScenePlan::build_with(
        &scene,
        12,
        PlanOptions {
            allow_partial: true,
        },
    )
    .unwrap();
    assert!(!plan.resources.is_empty());
    for resource in &plan.resources {
        assert!(!resource.relative_path.starts_with('/'));
        assert!(!resource.relative_path.contains(".."));
    }
    assert!(!plan.to_json_string().contains(env!("CARGO_MANIFEST_DIR")));
}

// ---- domains the plan does not evaluate ------------------------------------------------------

#[test]
fn undeclared_domains_are_refused_not_dropped() {
    // The synthetic scene declares weather, lighting and a surface map on the hero.
    let scene = import(parse(LAYERED));
    let before = scene.clone();
    let error = ScenePlan::build(&scene, 12).unwrap_err();
    assert_eq!(
        domains(&error),
        [
            (Domain::Weather, None),
            (Domain::Lighting, None),
            (Domain::Surface, Some("hero-plane")),
        ]
    );
    assert_eq!(error.code(), "unsupported");
    assert!(error.to_string().contains("surface on hero-plane"));
    // Rejection leaves the scene as it was, and a clean scene still builds from the same bundle.
    assert_eq!(scene, before);
    assert_eq!(ScenePlan::build(&scene, 12), Err(error.clone()));

    let command = CommandError::from(error);
    assert_eq!(command.code, "unsupported");
    let details = command.details.unwrap();
    assert_eq!(details["unsupported"], "scene_plan_domains");
    assert_eq!(details["domains"][2]["subject"], "hero-plane");
}

#[test]
fn every_node_level_domain_is_named_with_its_node() {
    let mut doc = parse(RECIPES);
    {
        let figure = node_mut(&mut doc, "scene-a", "figure")
            .as_object_mut()
            .unwrap();
        figure.insert("rigBind".into(), json!({ "rigId": "r" }));
        figure.insert("reveal".into(), json!({ "kind": "wipe" }));
        figure.insert("relief".into(), json!({ "depth": 1 }));
        figure.insert("sceneDepth".into(), json!({ "map": 1 }));
        // `null` is "not declared", as for the evaluator.
        figure.insert("normalMap".into(), Value::Null);
    }
    node_mut(&mut doc, "scene-a", "backdrop")
        .as_object_mut()
        .unwrap()
        .insert("surface".into(), json!({ "floor": null }));
    let error = ScenePlan::build(&import(doc), 0).unwrap_err();
    assert_eq!(
        domains(&error),
        [
            (Domain::RigBind, Some("figure")),
            (Domain::Reveal, Some("figure")),
            (Domain::Surface, Some("backdrop")),
            (Domain::SceneDepth, Some("figure")),
            (Domain::Relief, Some("figure")),
        ]
    );
}

#[test]
fn scene_level_domains_are_the_evaluators_own_list() {
    let mut doc = parse(RECIPES);
    let definition = doc["sceneDefinitions"][0].as_object_mut().unwrap();
    // The importer keeps these raw; it only reads `lighting.lights[].parent`, hence the empty list.
    definition.insert("weather".into(), json!("rain"));
    definition.insert("weatherKeys".into(), json!([]));
    definition.insert("lighting".into(), json!({ "ambient": 1, "lights": [] }));
    definition.insert("effects".into(), json!({}));
    definition.insert("weatherGround".into(), json!({ "nodeId": "backdrop" }));
    definition.insert("rigs".into(), json!([]));
    definition.insert("paths".into(), json!([]));
    definition.insert("contactPlaneId".into(), json!("backdrop"));
    let scene = import(doc);
    let error = ScenePlan::build(&scene, 0).unwrap_err();

    let expected: Vec<Domain> = vec![
        Domain::Weather,
        Domain::WeatherKeys,
        Domain::Lighting,
        Domain::Effects,
        Domain::WeatherGround,
        Domain::Rigs,
        Domain::Paths,
    ];
    let found: Vec<Domain> = domains(&error)
        .into_iter()
        .map(|(domain, _)| domain)
        .collect();
    assert_eq!(found, expected);
    // Every one of them is what `unevaluated_sections` reports: nothing is invented here.
    let evaluator: Vec<&str> = scene
        .bundle()
        .scene("scene-a")
        .unwrap()
        .unevaluated_sections()
        .to_vec();
    for domain in &expected {
        assert!(evaluator.contains(&domain.as_str()), "{domain:?}");
    }
}

#[test]
fn a_partial_plan_differs_from_the_complete_one_only_by_what_it_lists() {
    let complete_doc = layered_complete();
    let complete = plan_of(&import(complete_doc.clone()), 12);

    // The same scene with lighting and weather declared: the evaluated parts do not move.
    let mut doc = complete_doc;
    let definition = doc["sceneDefinitions"][0].as_object_mut().unwrap();
    definition.insert("weather".into(), json!("rain"));
    definition.insert("lighting".into(), json!({ "ambient": 1, "lights": [] }));
    let scene = import(doc);
    assert!(matches!(
        ScenePlan::build(&scene, 12),
        Err(PlanError::Unsupported { .. })
    ));
    let partial = ScenePlan::build_with(
        &scene,
        12,
        PlanOptions {
            allow_partial: true,
        },
    )
    .unwrap();

    assert!(!partial.is_complete());
    assert_eq!(
        partial
            .unsupported
            .iter()
            .map(|entry| entry.domain)
            .collect::<Vec<_>>(),
        [Domain::Weather, Domain::Lighting]
    );
    assert_eq!(partial.camera, complete.camera);
    assert_eq!(partial.nodes, complete.nodes);
    assert_eq!(partial.layers, complete.layers);
    assert_eq!(partial.resources, complete.resources);
    assert_eq!(partial.requires, complete.requires);
    assert_eq!(partial.frame, complete.frame);
}

#[test]
fn a_frame_that_cannot_be_evaluated_is_reported_before_the_domains() {
    // `LAYERED` declares weather, but the frame is outside the edit: the evaluator's error wins,
    // as in the origin's order of checks.
    let scene = import(parse(LAYERED));
    for frame in [-1_i64, 48, 1_000_000] {
        let error = ScenePlan::build(&scene, frame).unwrap_err();
        assert_eq!(
            error,
            PlanError::Frame(TemporalError::EditFrameOutOfRange {
                frame,
                duration_frames: 48,
            })
        );
        assert_eq!(error.code(), "frame_out_of_range");
    }
}

// ---- fields the importer keeps without interpreting ------------------------------------------

#[test]
fn uninterpreted_fields_are_listed_sorted_with_their_reason() {
    let plan = plan_of(&import(layered_complete()), 12);
    assert_eq!(
        pointers(&plan),
        [
            ("/metadata", UninterpretedReason::DeferredSection),
            (
                "/sceneDefinitions/0/nodes/1/x-note",
                UninterpretedReason::UnknownField
            ),
            ("/sceneInstances/0/label", UninterpretedReason::UnknownField),
            ("/x-tool", UninterpretedReason::UnknownField),
        ]
    );

    let mut doc = layered_complete();
    doc["narration"] = json!({ "segments": [] });
    doc["sceneDefinitions"][0]["contactPlaneId"] = json!("bg-plane");
    let plan = plan_of(&import(doc), 12);
    let listed = pointers(&plan);
    assert!(listed.contains(&("/narration", UninterpretedReason::DeferredSection)));
    assert!(listed.contains(&(
        "/sceneDefinitions/0/contactPlaneId",
        UninterpretedReason::DeferredSection
    )));
}

#[test]
fn unknown_fields_never_change_the_picture() {
    let base = plan_of(&import(layered_complete()), 12);
    let mut doc = layered_complete();
    doc["x-new"] = json!(1);
    node_mut(&mut doc, "street", "bg-plane")
        .as_object_mut()
        .unwrap()
        .insert("x-other".into(), json!({ "a": [1, 2] }));
    let plan = plan_of(&import(doc), 12);

    assert_eq!(plan.camera, base.camera);
    assert_eq!(plan.nodes, base.nodes);
    assert_eq!(plan.layers, base.layers);
    assert_eq!(plan.resources, base.resources);
    assert_eq!(plan.requires, base.requires);
    assert_eq!(plan.uninterpreted.len(), base.uninterpreted.len() + 2);
}

#[test]
fn what_another_scene_declares_does_not_change_this_frame() {
    // Two scenes, two shots. Extra fields on everything that belongs to scene-b leave the plan of
    // a frame of scene-a identical, and show up on a frame of scene-b.
    let doc = parse(OFFSETS);
    let scene = import(doc.clone());
    let (in_a, in_b) = (
        i64::try_from(scene.bundle().shots[0].start_frame).unwrap(),
        i64::try_from(scene.bundle().shots[1].start_frame).unwrap(),
    );
    let (base_a, base_b) = (plan_of(&scene, in_a), plan_of(&scene, in_b));
    assert_eq!(base_a.source.scene_definition_id, "scene-a");
    assert_eq!(base_b.source.scene_definition_id, "scene-b");
    assert_eq!(
        base_a.frame.instance_frame, 10,
        "shot-a starts at instance frame 10"
    );

    let mut edited = doc;
    edited["sceneDefinitions"][1]["x-extra"] = json!(1);
    edited["sceneInstances"][1]["x-extra"] = json!(1);
    edited["shots"][1]["x-extra"] = json!(1);
    let edited = import(edited);

    assert_eq!(plan_of(&edited, in_a), base_a);
    let plan_b = plan_of(&edited, in_b);
    assert_eq!(plan_b.nodes, base_b.nodes);
    assert_eq!(plan_b.uninterpreted.len(), base_b.uninterpreted.len() + 3);
    let listed = pointers(&plan_b);
    for pointer in [
        "/sceneDefinitions/1/x-extra",
        "/sceneInstances/1/x-extra",
        "/shots/1/x-extra",
    ] {
        assert!(
            listed.contains(&(pointer, UninterpretedReason::UnknownField)),
            "{pointer}"
        );
        assert!(!pointers(&base_a).iter().any(|entry| entry.0 == pointer));
    }
}

// ---- determinism and invariance --------------------------------------------------------------

#[test]
fn the_plan_depends_only_on_the_frame_never_on_the_order_of_the_questions() {
    let scene = import(layered_complete());
    let before = scene.clone();
    let order = [47_i64, 0, 12, 12, 30, 0, 47, 24, 23];
    let plans: Vec<(i64, ScenePlan)> = order
        .iter()
        .map(|&frame| (frame, plan_of(&scene, frame)))
        .collect();
    for (frame, plan) in &plans {
        let again = plan_of(&scene, *frame);
        assert_eq!(*plan, again, "frame {frame}");
        assert_eq!(
            plan.to_json_string(),
            again.to_json_string(),
            "frame {frame}"
        );
    }
    // Same bits whichever path led there: a fresh import of the same text, built in ascending
    // order, gives the same plans.
    let fresh = import(layered_complete());
    let mut ascending: Vec<i64> = order.to_vec();
    ascending.sort_unstable();
    ascending.dedup();
    for frame in ascending {
        let expected = &plans.iter().find(|(f, _)| *f == frame).unwrap().1;
        assert_eq!(&plan_of(&fresh, frame), expected, "frame {frame}");
    }
    assert_eq!(scene, before, "building plans never alters the scene");
}

#[test]
fn different_frames_give_different_plans_and_the_animation_is_visible() {
    let scene = import(layered_complete());
    let (first, later) = (plan_of(&scene, 0), plan_of(&scene, 3));
    assert_ne!(first.frame, later.frame);
    assert_ne!(first.camera, later.camera);
    // `hero-plane` bobs (cycle, period 12, on localPosition): a quarter period in, the sine is at
    // its crest, so the world matrix moves and its layer does not.
    assert_ne!(
        first.node("hero-plane").unwrap().world_matrix,
        later.node("hero-plane").unwrap().world_matrix
    );
    assert_eq!(first.layers, later.layers);
    assert_eq!(first.resources, later.resources);
}

#[test]
fn a_frame_in_another_shot_resolves_that_shots_instance_and_clip() {
    let scene = import(parse(OFFSETS));
    let second = i64::try_from(scene.bundle().shots[1].start_frame).unwrap();
    let plan = plan_of(&scene, second);
    assert_eq!(plan.frame.shot_id, "shot-b");
    assert_eq!(plan.source.scene_instance_id, "inst-b");
    assert_eq!(plan.frame.instance_frame, 0);
    assert_eq!(
        plan.frame.camera_clip_id,
        scene.bundle().shots[1].camera_clip_id
    );
}

// ---- wire form -------------------------------------------------------------------------------

#[test]
fn the_plan_round_trips_through_json_losslessly_and_canonically() {
    for text in [SOCKETS, OFFSETS, RECIPES, MINIMAL] {
        let scene = import(parse(text));
        for frame in frames_of(&scene) {
            let plan = plan_of(&scene, frame);
            let wire = plan.to_json_string();
            let read = ScenePlan::from_json_str(&wire).unwrap();
            assert_eq!(read, plan, "frame {frame}");
            assert_eq!(read.to_json_string(), wire, "frame {frame}: canonical text");
            assert_eq!(ScenePlan::from_json_str(&plan.to_json_pretty()), Ok(plan));
        }
    }
    let scene = import(layered_complete());
    let plan = plan_of(&scene, 12);
    assert_eq!(ScenePlan::from_json(&plan.to_json()), Ok(plan));
}

#[test]
fn the_wire_form_names_its_kind_version_and_requirements() {
    let plan = plan_of(&import(parse(SOCKETS)), 0);
    let wire = plan.to_json();
    assert_eq!(wire["kind"], PLAN_KIND);
    assert_eq!(wire["planVersion"], PLAN_VERSION);
    assert_eq!(wire["complete"], true);
    assert_eq!(
        wire["requires"],
        json!([
            "camera",
            "nodeTransforms",
            "sockets",
            "performanceChannels",
            "imagePlanes",
            "straightAlpha",
            "opaqueAlpha"
        ])
    );
    assert_eq!(wire["output"]["colorSpace"], "srgb-display");
    assert_eq!(wire["source"]["sceneDefinitionId"], "scene-rig");
    assert_eq!(wire["resources"][0]["assetId"], "backdrop");
    assert_eq!(
        wire["nodes"][0]["worldMatrix"].as_array().unwrap().len(),
        16
    );
}

#[test]
fn a_plan_of_another_version_or_kind_is_refused_before_its_fields_are_read() {
    let plan = plan_of(&import(parse(MINIMAL)), 0);
    // Version 2 may carry fields this build does not know: refused as a version, not as malformed.
    let result = tampered(&plan, |wire| {
        wire["planVersion"] = json!(2);
        wire["brandNewSection"] = json!({});
        wire.as_object_mut().unwrap().remove("nodes");
    });
    assert_eq!(
        result,
        Err(PlanError::UnsupportedVersion {
            found: 2,
            supported: PLAN_VERSION,
        })
    );
    assert_eq!(result.unwrap_err().code(), "unsupported_plan_version");

    assert_invalid(
        tampered(&plan, |wire| wire["kind"] = json!("other")),
        "/kind",
    );
    assert_invalid(
        tampered(&plan, |wire| wire["planVersion"] = json!("1")),
        "/planVersion",
    );
    assert_invalid(ScenePlan::from_json_str("not json"), "");
    assert_invalid(ScenePlan::from_json_str("[]"), "");
}

#[test]
fn unknown_and_missing_fields_are_refused_with_a_pointer() {
    let plan = plan_of(&import(parse(SOCKETS)), 0);
    assert_invalid(tampered(&plan, |wire| wire["extra"] = json!(1)), "/extra");
    assert_invalid(
        tampered(&plan, |wire| wire["nodes"][1]["extra"] = json!(1)),
        "/nodes/1/extra",
    );
    assert_invalid(
        tampered(&plan, |wire| {
            wire["camera"].as_object_mut().unwrap().remove("fovDeg");
        }),
        "/camera/fovDeg",
    );
    assert_invalid(
        tampered(&plan, |wire| {
            wire["nodes"][0].as_object_mut().unwrap().remove("assetId");
        }),
        "/nodes/0/assetId",
    );
    assert_invalid(
        tampered(&plan, |wire| {
            wire["output"]["colorSpace"] = json!("rec709-video")
        }),
        "/output/colorSpace",
    );
    assert_invalid(
        tampered(&plan, |wire| {
            wire["nodes"][0]["worldMatrix"] = json!([1, 2, 3])
        }),
        "/nodes/0/worldMatrix",
    );
    assert_invalid(
        tampered(&plan, |wire| {
            wire["frame"]["editTime"] = json!({ "numerator": 2, "denominator": 4 })
        }),
        "/frame/editTime",
    );
    assert_invalid(
        tampered(&plan, |wire| {
            wire["requires"] = json!(["camera", "teleport"])
        }),
        "/requires",
    );
}

#[test]
fn a_plan_that_breaks_its_own_contract_is_refused() {
    let plan = plan_of(&import(parse(SOCKETS)), 0);

    // Draw order: the layers swapped.
    assert_invalid(
        tampered(&plan, |wire| {
            wire["layers"].as_array_mut().unwrap().reverse();
        }),
        "/layers",
    );
    // Resource list: out of order, then a path that leaves the bundle folder, then a missing one.
    assert_invalid(
        tampered(&plan, |wire| {
            wire["resources"].as_array_mut().unwrap().reverse();
        }),
        "/resources",
    );
    assert_invalid(
        tampered(&plan, |wire| {
            wire["resources"][0]["relativePath"] = json!("/abs/backdrop.png")
        }),
        "/resources/0/relativePath",
    );
    assert_invalid(
        tampered(&plan, |wire| {
            wire["resources"][1]["relativePath"] = json!("../x/figure.png")
        }),
        "/resources/1/relativePath",
    );
    assert_invalid(
        tampered(&plan, |wire| {
            wire["resources"].as_array_mut().unwrap().pop();
        }),
        "/layers",
    );
    // A layer that disagrees with the resource it draws.
    assert_invalid(
        tampered(&plan, |wire| wire["layers"][0]["alpha"] = json!("straight")),
        "/layers/0/alpha",
    );
    // Requirements that do not match the content.
    assert_invalid(
        tampered(&plan, |wire| {
            wire["requires"] = json!(["camera", "nodeTransforms"])
        }),
        "/requires",
    );
    // A node that is its own parent, an unknown parent, a cycle.
    assert_invalid(
        tampered(&plan, |wire| {
            wire["nodes"][1]["parent"]["nodeId"] = json!("backdrop")
        }),
        "/nodes/1/parent",
    );
    assert_invalid(
        tampered(&plan, |wire| {
            wire["nodes"][1]["parent"]["nodeId"] = json!("nobody")
        }),
        "/nodes/1/parent",
    );
    assert_invalid(
        tampered(&plan, |wire| {
            wire["nodes"][0]["parent"] = json!({ "nodeId": "backdrop", "socket": null });
        }),
        "/nodes",
    );
    // Duplicate ids.
    assert_invalid(
        tampered(&plan, |wire| wire["nodes"][1]["id"] = json!("root")),
        "/nodes/1/id",
    );
    // `complete` that contradicts `unsupported`.
    assert_invalid(
        tampered(&plan, |wire| wire["complete"] = json!(false)),
        "/complete",
    );
    assert_invalid(
        tampered(&plan, |wire| {
            wire["unsupported"] = json!([{ "domain": "lighting", "subject": null }]);
        }),
        "/complete",
    );
}

// ---- capabilities ----------------------------------------------------------------------------

#[test]
fn a_runtime_is_checked_against_what_the_plan_needs() {
    let plan = plan_of(&import(parse(SOCKETS)), 0);
    let basic = [Capability::Camera, Capability::NodeTransforms];
    assert_eq!(
        plan.missing_capabilities(&basic),
        [
            Capability::Sockets,
            Capability::PerformanceChannels,
            Capability::ImagePlanes,
            Capability::StraightAlpha,
            Capability::OpaqueAlpha,
        ]
    );
    let error = plan.require_capabilities(&basic).unwrap_err();
    assert_eq!(error.code(), "unsupported");
    assert_eq!(error.details().unwrap()["missing"][0], "sockets", "{error}");
    assert_eq!(plan.require_capabilities(&Capability::ALL), Ok(()));
    assert_eq!(plan.require_capabilities(capabilities().supported), Ok(()));

    // A plan without a composition pass or sockets does not ask for them.
    let simple = plan_of(&import(parse(MINIMAL)), 0);
    assert!(!simple.requires.contains(&Capability::CompositionPassOrder));
    assert!(!simple.requires.contains(&Capability::Sockets));
}

#[test]
fn this_versions_capabilities_name_what_it_carries_and_refuses() {
    let caps = capabilities();
    assert_eq!(caps.plan_version, PLAN_VERSION);
    assert_eq!(caps.supported, Capability::ALL);
    for domain in [
        Domain::Lighting,
        Domain::Weather,
        Domain::Rigs,
        Domain::Effects,
        Domain::Paths,
        Domain::Surface,
    ] {
        assert!(caps.unsupported.contains(&domain), "{domain:?}");
        assert_eq!(Domain::parse(domain.as_str()), Some(domain));
    }
    for capability in Capability::ALL {
        assert_eq!(Capability::parse(capability.as_str()), Some(capability));
    }
}
