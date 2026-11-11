//! BB01/BB02: import, validate and round-trip `render-bundle` v1.0 and inspect its graph,
//! through the public library API only (no GPUI, no window, no runtime).
//!
//! Every fixture is synthetic and hand-written for this test (`directed_scene_fixtures/`); none
//! was produced by running the origin, so these tests prove the behaviour of this importer, not
//! equivalence with the origin (that is the oracle's job: docs/migration/fixtures).

use serde_json::{Value, json};
use tessel_editor_core::CommandError;
use tessel_editor_core::directed_scene::{
    DirectedScene, Entity, EntityKind, ImportError, Limits, NodeKind, Parent, Transition,
    UNKNOWN_FIELD, Vec3,
};
use tessel_editor_core::time::Ratio;

const LAYERED: &str = include_str!("directed_scene_fixtures/layered_scene.json");

fn fixture() -> Value {
    serde_json::from_str(LAYERED).expect("the fixture is valid JSON")
}

fn imported() -> DirectedScene {
    DirectedScene::import_str(LAYERED).expect("the layered fixture imports")
}

/// The fixture with `edit` applied, imported.
fn with(edit: impl FnOnce(&mut Value)) -> Result<DirectedScene, ImportError> {
    let mut value = fixture();
    edit(&mut value);
    DirectedScene::import_value(value)
}

/// Sets (or adds) the value at a JSON Pointer whose parent exists.
fn set(value: &mut Value, pointer: &str, new: Value) {
    let (parent, key) = pointer.rsplit_once('/').expect("a pointer has a parent");
    let parent = value
        .pointer_mut(parent)
        .unwrap_or_else(|| panic!("fixture has no `{parent}`"));
    match parent {
        Value::Object(map) => {
            map.insert(key.to_string(), new);
        }
        Value::Array(items) => items[key.parse::<usize>().expect("an index")] = new,
        other => panic!("`{pointer}` hangs from {other}"),
    }
}

fn remove(value: &mut Value, parent: &str, key: &str) {
    value
        .pointer_mut(parent)
        .and_then(Value::as_object_mut)
        .unwrap_or_else(|| panic!("fixture has no object `{parent}`"))
        .remove(key)
        .unwrap_or_else(|| panic!("`{parent}` has no `{key}`"));
}

fn push(value: &mut Value, pointer: &str, item: Value) {
    value
        .pointer_mut(pointer)
        .and_then(Value::as_array_mut)
        .unwrap_or_else(|| panic!("fixture has no array `{pointer}`"))
        .push(item);
}

fn issues_of(result: Result<DirectedScene, ImportError>) -> Vec<(String, String)> {
    match result {
        Ok(_) => panic!("expected the import to be refused"),
        Err(error) => error
            .issues()
            .iter()
            .map(|issue| (issue.code.to_string(), issue.path.clone()))
            .collect(),
    }
}

/// Asserts the import fails with `code` at `path` (`None`: any path).
#[track_caller]
fn refused(result: Result<DirectedScene, ImportError>, code: &str, path: Option<&str>) {
    let issues = issues_of(result);
    let found = issues
        .iter()
        .any(|(c, p)| c == code && path.is_none_or(|expected| p == expected));
    assert!(found, "expected {code} at {path:?}, got {issues:#?}");
}

fn near(actual: Vec3, expected: (f64, f64, f64)) {
    let distance = actual.distance(Vec3 {
        x: expected.0,
        y: expected.1,
        z: expected.2,
    });
    assert!(distance < 1e-9, "{actual:?} is not {expected:?}");
}

// ---- import and inspection ----------------------------------------------------------------

#[test]
fn imports_the_layered_fixture_and_reads_its_structure() {
    let scene = imported();
    let bundle = scene.bundle();

    assert_eq!(bundle.bundle_id, "layered-street");
    assert_eq!(bundle.assets.len(), 4);
    assert_eq!(bundle.instances.len(), 1);
    assert_eq!(bundle.shots.len(), 2);
    assert_eq!(
        bundle.shots[1].transition_into,
        Some(Transition::Continuous)
    );
    assert_eq!(bundle.shots[0].transition_into, None);
    assert_eq!(bundle.camera_clip("wide").unwrap().layers.len(), 1);

    // Exact time, not floats: 24000/1001 fps, 48 frames, 24 frames in.
    assert_eq!(bundle.fps.rate().unwrap().num(), 24000);
    assert_eq!(bundle.edit_duration(), Ratio::new(1001, 500));
    assert_eq!(bundle.edit_time_of_frame(24), Ratio::new(1001, 1000));
    assert_eq!(bundle.shot_at_frame(23).unwrap().id, "s1");
    assert_eq!(bundle.shot_at_frame(24).unwrap().id, "s2");
    assert!(bundle.shot_at_frame(48).is_none());

    // The bare asset format means PNG.
    assert_eq!(bundle.asset("bg").unwrap().format.as_str(), "png");
    assert_eq!(scene.edit_join_issues(), vec![]);
}

#[test]
fn exposes_ids_hierarchy_sockets_and_layers() {
    let scene = imported();
    let street = scene.bundle().scene("street").unwrap();

    assert_eq!(street.root_id(), "root");
    let ids: Vec<&str> = street.nodes().iter().map(|n| n.id.as_str()).collect();
    assert_eq!(
        ids,
        ["root", "stage", "bg-plane", "hero-plane", "prop-plane"]
    );
    assert_eq!(
        street.node("hero-plane").unwrap().kind,
        NodeKind::ImagePlane
    );
    assert!(street.node("ghost").is_none());

    assert!(street.parent("root").is_none());
    assert_eq!(street.parent("stage"), Some(&Parent::Node("root".into())));
    // A node on a socket hangs from the socket's owner, not from a `parentId`.
    assert_eq!(
        street.parent("prop-plane"),
        Some(&Parent::Socket {
            owner: "hero-plane".into(),
            socket: "hand".into()
        })
    );
    let stage_children: Vec<&str> = street
        .children("stage")
        .iter()
        .map(|n| n.id.as_str())
        .collect();
    assert_eq!(stage_children, ["bg-plane", "hero-plane"]);
    assert_eq!(street.children("hero-plane")[0].id, "prop-plane");
    let ancestors: Vec<&str> = street
        .ancestors("prop-plane")
        .iter()
        .map(|n| n.id.as_str())
        .collect();
    assert_eq!(ancestors, ["hero-plane", "stage", "root"]);
    assert_eq!(street.image_planes().count(), 3);
    assert_eq!(street.socket("hero-plane", "hand").unwrap().position.y, 1.0);

    // Draw order: the composition pass (-1) beats the node's own renderOrder (1); the rest keep
    // document order at the default 0.
    let paint: Vec<&str> = scene
        .bundle()
        .paint_order("street")
        .unwrap()
        .iter()
        .map(|n| n.id.as_str())
        .collect();
    assert_eq!(paint, ["hero-plane", "bg-plane", "prop-plane"]);
    assert!(scene.bundle().paint_order("elsewhere").is_none());
}

#[test]
fn computes_effective_3d_transforms_through_hierarchy_and_socket() {
    let scene = imported();
    let street = scene.bundle().scene("street").unwrap();
    let translation = |id: &str| street.world_matrix(id).unwrap().translation();

    near(translation("root"), (0.0, 0.0, 0.0));
    // stage: translate (1,2,3), scale 2.
    near(translation("stage"), (1.0, 2.0, 3.0));
    // bg: (0,0,-5) inside the stage -> 2 * (0,0,-5) + (1,2,3).
    near(translation("bg-plane"), (1.0, 2.0, -7.0));
    near(translation("hero-plane"), (7.0, 2.0, 3.0));
    // prop: (0.5,0,0) -> socket (0,1,0) -> hero turned 90 degrees about Y and placed at x=3
    // -> stage: world (7, 4, 2).
    near(translation("prop-plane"), (7.0, 4.0, 2.0));

    // The hero's local x axis points down -z after the quarter turn about Y.
    let local = street.local_matrix("hero-plane").unwrap();
    assert!((local.0[2] + 1.0).abs() < 1e-12 && local.0[0].abs() < 1e-12);

    // A point in the prop's own space lands in scene space through every ancestor.
    let point = street
        .world_matrix("prop-plane")
        .unwrap()
        .transform_point(Vec3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        });
    near(point, (7.0, 4.0, 0.0));
}

#[test]
fn lists_references_and_who_depends_on_an_entity() {
    let scene = imported();
    let bundle = scene.bundle();
    let references = bundle.references();

    let hero = Entity::new(EntityKind::Asset, "hero");
    let users = bundle.referrers_of(&hero);
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].from, Entity::node("street", "hero-plane"));
    assert_eq!(users[0].field, "assetRef.assetId");

    let clip = Entity::new(EntityKind::CameraClip, "wide");
    let clip_users = bundle.referrers_of(&clip);
    let shots: Vec<&str> = clip_users.iter().map(|r| r.from.id.as_str()).collect();
    assert_eq!(shots, ["s1", "s2"]);

    // The node that the pass, the perf channel, the socket and the prop all name.
    let hero_node = Entity::node("street", "hero-plane");
    let node_users = bundle.referrers_of(&hero_node);
    let mut fields: Vec<&str> = node_users.iter().map(|r| r.field).collect();
    fields.sort_unstable();
    assert_eq!(
        fields,
        ["attachToSocket.parentNodeId", "channels.nodeId", "nodeId"]
    );
    // Every reference points at something that exists.
    assert!(references.iter().all(|r| {
        match r.to.kind {
            EntityKind::Asset => bundle.asset(&r.to.id).is_some(),
            EntityKind::SceneDefinition => bundle.scene(&r.to.id).is_some(),
            EntityKind::SceneInstance => bundle.instance(&r.to.id).is_some(),
            EntityKind::CameraClip => bundle.camera_clip(&r.to.id).is_some(),
            EntityKind::Node => bundle
                .scene(r.to.scope.as_deref().unwrap())
                .and_then(|s| s.node(&r.to.id))
                .is_some(),
            EntityKind::PerformanceTrack => {
                bundle.performance_tracks.iter().any(|t| t.id == r.to.id)
            }
            EntityKind::Shot | EntityKind::CompositionPass | EntityKind::Light => true,
        }
    }));
}

// ---- round trip, unknown and deferred data -------------------------------------------------

#[test]
fn round_trip_keeps_every_field_number_and_unknown_key() {
    let original = fixture();
    let scene = imported();

    // The document is the canonical data: nothing is lost, nothing is added.
    assert_eq!(scene.document(), &original);
    let compact = DirectedScene::import_str(&scene.to_json_string()).unwrap();
    assert_eq!(compact.document(), &original);
    let pretty_text = scene.to_json_pretty();
    assert!(pretty_text.ends_with("}\n"));
    let pretty = DirectedScene::import_str(&pretty_text).unwrap();
    assert_eq!(pretty.document(), &original);
    // Idempotent: a second pass changes nothing.
    assert_eq!(compact.to_json_string(), scene.to_json_string());
    assert_eq!(compact.bundle(), scene.bundle());

    // Integers stay integers and fractions stay exact.
    let doc = scene.document();
    assert!(doc["fps"]["numerator"].is_u64());
    assert_eq!(doc["audio"]["editEndSecondsExclusive"], json!(2.002));
    // Fields the importer does not interpret survive untouched.
    assert_eq!(doc["x-tool"]["name"], "synthetic");
    assert_eq!(doc["sceneDefinitions"][0]["weather"], "rain");
    assert_eq!(
        doc["sceneDefinitions"][0]["nodes"][1]["x-note"],
        "kept although render-bundle 1.0 does not declare it"
    );
    assert_eq!(
        doc["performanceTracks"][0]["channels"][0]["axis"],
        json!({ "x": 0, "y": 1, "z": 0 })
    );
    assert_eq!(scene.clone().into_document(), original);
}

#[test]
fn reports_keys_the_origin_schema_does_not_declare() {
    let scene = imported();
    let notices: Vec<(&str, &str)> = scene
        .notices()
        .iter()
        .map(|n| (n.code, n.path.as_str()))
        .collect();
    // Only what the origin would silently strip. Declared-but-deferred sections (weather,
    // lighting, surface, channel payloads, metadata) are not "unknown".
    assert_eq!(
        notices,
        [
            (UNKNOWN_FIELD, "/sceneDefinitions/0/nodes/1/x-note"),
            (UNKNOWN_FIELD, "/sceneInstances/0/label"),
            (UNKNOWN_FIELD, "/x-tool"),
        ]
    );
}

#[test]
fn deferred_sections_are_kept_verbatim_and_not_validated() {
    // Policy: lighting is declared by the origin but not interpreted by BB01/BB02. Whatever it
    // holds is preserved; judging it belongs to the lighting phase (docs: DEFERRED_RULES).
    let scene = with(|v| {
        set(
            v,
            "/sceneDefinitions/0/lighting",
            json!({ "anything": [1, 2.5, null, { "deep": true }] }),
        );
        set(v, "/narration", json!("opaque"));
    })
    .unwrap();
    assert_eq!(
        scene.document()["sceneDefinitions"][0]["lighting"]["anything"][1],
        json!(2.5)
    );
    assert_eq!(scene.document()["narration"], "opaque");
    assert_eq!(
        DirectedScene::import_str(&scene.to_json_string())
            .unwrap()
            .document(),
        scene.document()
    );
}

// ---- graph rules ----------------------------------------------------------------------------

#[test]
fn rejects_duplicate_ids_with_the_origin_codes() {
    refused(
        with(|v| set(v, "/sceneDefinitions/0/nodes/2/id", json!("stage"))),
        "DUPLICATE_NODE_ID",
        Some("/sceneDefinitions/0/nodes/2"),
    );
    refused(
        with(|v| set(v, "/assets/1/assetId", json!("bg"))),
        "DUPLICATE_ASSET_ID",
        Some("/assets/1/assetId"),
    );
    refused(
        with(|v| set(v, "/shots/1/id", json!("s1"))),
        "DUPLICATE_ID",
        Some("/shots/1"),
    );
    refused(
        with(|v| {
            let def = v["sceneDefinitions"][0].clone();
            push(v, "/sceneDefinitions", def);
        }),
        "DUPLICATE_SCENE_DEFINITION_ID",
        Some("/sceneDefinitions/1"),
    );
    refused(
        with(|v| {
            let instance = v["sceneInstances"][0].clone();
            push(v, "/sceneInstances", instance);
        }),
        "DUPLICATE_SCENE_INSTANCE_ID",
        None,
    );
    refused(
        with(|v| {
            let socket = v["sceneDefinitions"][0]["sockets"][0].clone();
            push(v, "/sceneDefinitions/0/sockets", socket);
        }),
        "DUPLICATE_SOCKET",
        Some("/sceneDefinitions/0/sockets/1"),
    );
}

#[test]
fn rejects_a_hierarchy_that_loops_back_on_itself() {
    // stage -> bg-plane -> stage: neither reaches the root.
    let issues = issues_of(with(|v| {
        set(v, "/sceneDefinitions/0/nodes/1/parentId", json!("bg-plane"));
    }));
    let codes: Vec<&str> = issues.iter().map(|(c, _)| c.as_str()).collect();
    assert!(codes.contains(&"PARENT_CYCLE"), "{codes:?}");
    assert!(codes.contains(&"EFFECTIVE_GRAPH_CYCLE"), "{codes:?}");

    // A node that is its own parent.
    refused(
        with(|v| set(v, "/sceneDefinitions/0/nodes/2/parentId", json!("bg-plane"))),
        "PARENT_CYCLE",
        None,
    );
    // A cycle through a socket that the declared `parentId`s do not show.
    refused(
        with(|v| {
            set(v, "/sceneDefinitions/0/nodes/3/parentId", Value::Null);
            set(
                v,
                "/sceneDefinitions/0/nodes/3/attachToSocket",
                json!({ "parentNodeId": "prop-plane", "socketName": "hand" }),
            );
            push(
                v,
                "/sceneDefinitions/0/sockets",
                json!({
                    "nodeId": "prop-plane", "name": "hand",
                    "localPosition": { "x": 0, "y": 0, "z": 0 },
                    "localRotation": { "x": 0, "y": 0, "z": 0, "w": 1 }
                }),
            );
        }),
        "EFFECTIVE_GRAPH_CYCLE",
        None,
    );
}

#[test]
fn rejects_broken_references() {
    let nodes = "/sceneDefinitions/0/nodes";
    refused(
        with(|v| set(v, &format!("{nodes}/2/parentId"), json!("ghost"))),
        "MISSING_PARENT",
        Some("/sceneDefinitions/0/nodes/2/parentId"),
    );
    refused(
        with(|v| set(v, "/sceneDefinitions/0/rootNodeId", json!("nope"))),
        "MISSING_ROOT_NODE",
        None,
    );
    refused(
        with(|v| set(v, &format!("{nodes}/2/parentId"), Value::Null)),
        "ORPHAN_NODE",
        Some("/sceneDefinitions/0/nodes/2"),
    );
    refused(
        with(|v| set(v, &format!("{nodes}/0/parentId"), json!("stage"))),
        "ROOT_HAS_PARENT",
        None,
    );
    refused(
        with(|v| {
            set(
                v,
                &format!("{nodes}/4/attachToSocket/socketName"),
                json!("foot"),
            )
        }),
        "MISSING_SOCKET",
        Some("/sceneDefinitions/0/nodes/4/attachToSocket"),
    );
    refused(
        with(|v| set(v, &format!("{nodes}/4/parentId"), json!("stage"))),
        "SOCKET_AND_PARENT_CONFLICT",
        None,
    );
    refused(
        with(|v| {
            set(
                v,
                &format!("{nodes}/4/attachToSocket/parentNodeId"),
                json!("prop-plane"),
            )
        }),
        "SOCKET_SELF_REFERENCE",
        None,
    );
    refused(
        with(|v| set(v, "/sceneDefinitions/0/sockets/0/nodeId", json!("ghost"))),
        "SOCKET_OWNER_MISSING",
        Some("/sceneDefinitions/0/sockets/0/nodeId"),
    );
    refused(
        with(|v| set(v, &format!("{nodes}/2/assetRef/assetId"), json!("nope"))),
        "ASSET_REF_UNKNOWN",
        Some("/sceneDefinitions/0/nodes/2/assetRef"),
    );
    refused(
        with(|v| {
            set(
                v,
                &format!("{nodes}/2/assetRef/contentHash"),
                json!("stale"),
            )
        }),
        "ASSET_REF_MISMATCH",
        None,
    );
    refused(
        with(|v| set(v, "/assets/1/format", json!("spz"))),
        "ASSET_FORMAT_MISMATCH",
        None,
    );
    refused(
        with(|v| remove(v, &format!("{nodes}/2"), "assetRef")),
        "IMAGE_PLANE_MISSING_ASSET",
        None,
    );
    refused(
        with(|v| remove(v, &format!("{nodes}/2"), "widthSceneUnits")),
        "IMAGE_PLANE_MISSING_DIMENSIONS",
        None,
    );
    refused(
        with(|v| set(v, "/sceneInstances/0/sceneDefinitionId", json!("elsewhere"))),
        "UNKNOWN_SCENE_DEFINITION",
        Some("/sceneInstances/0/sceneDefinitionId"),
    );
    refused(
        with(|v| set(v, "/sceneInstances/0/performanceTrackIds/0", json!("ghost"))),
        "PERFORMANCE_TRACK_REF_MISSING",
        None,
    );
    refused(
        with(|v| set(v, "/performanceTracks/0/instanceId", json!("other"))),
        "PERFORMANCE_TRACK_INSTANCE_MISMATCH",
        None,
    );
    refused(
        with(|v| set(v, "/performanceTracks/0/channels/0/nodeId", json!("ghost"))),
        "PERFORMANCE_NODE_MISSING",
        Some("/performanceTracks/0/channels/0/nodeId"),
    );
    refused(
        with(|v| set(v, "/compositionPasses/0/nodeId", json!("ghost"))),
        "COMPOSITION_NODE_MISSING",
        None,
    );
    refused(
        with(|v| {
            push(
                v,
                "/compositionPasses",
                json!({ "fragmentId": "other", "nodeId": "hero-plane", "stage": "prePost", "order": 3 }),
            )
        }),
        "COMPOSITION_ORDER_CONFLICT",
        Some("/compositionPasses/1"),
    );
}

#[test]
fn rejects_degenerate_and_unnormalized_rotations() {
    let rotation = "/sceneDefinitions/0/nodes/3/localRotation";
    refused(
        with(|v| set(v, rotation, json!({ "x": 0, "y": 0, "z": 0, "w": 0 }))),
        "DEGENERATE_QUATERNION",
        Some(rotation),
    );
    refused(
        with(|v| set(v, rotation, json!({ "x": 0, "y": 0, "z": 0, "w": 2 }))),
        "QUATERNION_NOT_NORMALIZED",
        Some(rotation),
    );
    // Within the origin's 1% tolerance a rotation is accepted as given, not renormalized.
    let scene = with(|v| set(v, rotation, json!({ "x": 0, "y": 0, "z": 0, "w": 1.005 }))).unwrap();
    assert_eq!(
        scene.document()["sceneDefinitions"][0]["nodes"][3]["localRotation"]["w"],
        json!(1.005)
    );
}

#[test]
fn a_deep_chain_imports_without_overflowing_the_stack() {
    let count = 20_000;
    let node = |i: usize| {
        json!({
            "id": format!("n{i}"),
            "parentId": if i == 0 { Value::Null } else { json!(format!("n{}", i - 1)) },
            "type": "group",
            "localPosition": { "x": 1, "y": 0, "z": 0 },
            "localRotation": { "x": 0, "y": 0, "z": 0, "w": 1 },
            "localScale": { "x": 1, "y": 1, "z": 1 }
        })
    };
    let scene = with(|v| {
        set(
            v,
            "/sceneDefinitions/0",
            json!({
                "id": "street", "schemaVersion": "1.0", "rootNodeId": "n0",
                "nodes": (0..count).map(node).collect::<Vec<_>>()
            }),
        );
        set(v, "/performanceTracks", json!([]));
        set(v, "/sceneInstances/0/performanceTrackIds", json!([]));
        set(v, "/compositionPasses", json!([]));
        set(v, "/assets", json!([fixture()["assets"][0].clone()]));
    })
    .unwrap();
    let street = scene.bundle().scene("street").unwrap();
    near(
        street
            .world_matrix(&format!("n{}", count - 1))
            .unwrap()
            .translation(),
        // every one of the 20 000 nodes, the root included, moves x by 1.
        (count as f64, 0.0, 0.0),
    );
    assert_eq!(
        street.ancestors(&format!("n{}", count - 1)).len(),
        count - 1
    );
}

#[test]
fn rejects_transforms_that_overflow_through_the_hierarchy() {
    // Each level is finite; the product is not. The origin has no rule for this.
    refused(
        with(|v| {
            for node in 1..4 {
                set(
                    v,
                    &format!("/sceneDefinitions/0/nodes/{node}/localScale"),
                    json!({ "x": 1e200, "y": 1e200, "z": 1e200 }),
                );
            }
        }),
        "NON_FINITE_TRANSFORM",
        None,
    );
}

// ---- shape, numbers, versions, unsupported vocabulary --------------------------------------

#[test]
fn reports_wrong_types_and_missing_fields_with_pointers() {
    refused(
        with(|v| set(v, "/sceneDefinitions/0/nodes/1/localScale/x", json!("2"))),
        "INVALID_TYPE",
        Some("/sceneDefinitions/0/nodes/1/localScale/x"),
    );
    refused(
        with(|v| remove(v, "", "bundleId")),
        "MISSING_FIELD",
        Some(""),
    );
    refused(
        with(|v| set(v, "/assets/0/assetId", json!(""))),
        "INVALID_VALUE",
        Some("/assets/0/assetId"),
    );
    // `null` is not "absent" for an optional field, as in the origin's zod.
    refused(
        with(|v| set(v, "/sceneDefinitions/0/nodes/2/assetRef", Value::Null)),
        "INVALID_TYPE",
        None,
    );
    refused(
        with(|v| set(v, "/assets", json!([]))),
        "INVALID_VALUE",
        Some("/assets"),
    );
    refused(
        with(|v| set(v, "/assets/0/alphaMode", json!("weird"))),
        "INVALID_VALUE",
        Some("/assets/0/alphaMode"),
    );
    refused(
        with(|v| set(v, "/sceneInstances/0/editLocked", json!(false))),
        "INVALID_VALUE",
        None,
    );
    refused(
        DirectedScene::import_value(json!([1, 2])),
        "INVALID_TYPE",
        Some(""),
    );
}

#[test]
fn reports_invalid_numbers_by_kind_and_range() {
    // Zero, negative, fractional and out-of-range counts.
    refused(
        with(|v| set(v, "/fps/denominator", json!(0))),
        "INVALID_VALUE",
        Some("/fps/denominator"),
    );
    refused(
        with(|v| set(v, "/editDurationFrames", json!(1.5))),
        "INVALID_VALUE",
        Some("/editDurationFrames"),
    );
    refused(
        with(|v| set(v, "/editDurationFrames", json!(-48))),
        "INVALID_VALUE",
        None,
    );
    refused(
        with(|v| set(v, "/shots/0/editRange/startFrame", json!(1u64 << 60))),
        "INVALID_VALUE",
        Some("/shots/0/editRange/startFrame"),
    );
    refused(
        with(|v| set(v, "/sceneDefinitions/0/nodes/2/widthSceneUnits", json!(-1))),
        "INVALID_VALUE",
        None,
    );
    refused(
        with(|v| set(v, "/cameraClips/0/initialFovDeg", json!(0))),
        "INVALID_VALUE",
        Some("/cameraClips/0/initialFovDeg"),
    );
    // An integer written as `24.0` is still the integer 24 (as with JSON.parse in the origin).
    let scene = DirectedScene::import_str(
        &LAYERED.replace("\"editDurationFrames\": 48", "\"editDurationFrames\": 48.0"),
    )
    .unwrap();
    assert_eq!(scene.bundle().edit_duration_frames, 48);

    // JSON has no NaN/Infinity, and a literal beyond f64 is a syntax error, not a number.
    for text in [
        r#"{"fps": NaN}"#,
        r#"{"fps": Infinity}"#,
        &LAYERED.replace("0.7071067811865476", "1e999"),
    ] {
        assert!(
            matches!(
                DirectedScene::import_str(text),
                Err(ImportError::Syntax { .. })
            ),
            "{text:.60}"
        );
    }
}

#[test]
fn frame_rates_outside_the_time_module_are_refused() {
    let both = |n: u64, d: u64| {
        with(move |v| {
            for path in ["/fps", "/renderProfile/fps"] {
                set(v, path, json!({ "numerator": n, "denominator": d }));
            }
        })
    };
    refused(both(5000, 1), "FPS_OUT_OF_RANGE", None);
    refused(both(1 << 40, 1), "FPS_OUT_OF_RANGE", None);
    assert_eq!(both(48, 2).unwrap().bundle().fps.rate().unwrap().num(), 24);
    // The origin compares the declarations literally, so 48/2 against 24/1 is a mismatch.
    refused(
        with(|v| set(v, "/fps", json!({ "numerator": 48, "denominator": 2 }))),
        "FPS_MISMATCH",
        Some("/fps"),
    );
}

#[test]
fn versions_other_than_1_0_are_refused_before_anything_else_is_read() {
    for version in [json!("2.0"), json!("1"), json!("1.0.0")] {
        let issues = issues_of(with(|v| set(v, "/schemaVersion", version)));
        assert_eq!(
            issues,
            vec![(
                "UNSUPPORTED_SCHEMA_VERSION".to_string(),
                "/schemaVersion".to_string()
            )]
        );
    }
    refused(
        with(|v| set(v, "/schemaVersion", json!(1))),
        "INVALID_TYPE",
        Some("/schemaVersion"),
    );
    refused(
        with(|v| set(v, "/sceneDefinitions/0/schemaVersion", json!("2.0"))),
        "UNSUPPORTED_SCHEMA_VERSION",
        Some("/sceneDefinitions/0/schemaVersion"),
    );
    refused(
        with(|v| remove(v, "", "schemaVersion")),
        "MISSING_FIELD",
        None,
    );
}

#[test]
fn types_and_values_the_origin_refuses_are_refused_with_its_code() {
    let node = "/sceneDefinitions/0/nodes/2/type";
    refused(
        with(|v| set(v, node, json!("supportPlane"))),
        "UNSUPPORTED_NODE_TYPE",
        Some(node),
    );
    refused(
        with(|v| set(v, node, json!("overlay"))),
        "UNSUPPORTED_NODE_TYPE",
        None,
    );
    // A type the origin schema does not even name is a plain invalid value.
    refused(
        with(|v| set(v, node, json!("hologram"))),
        "INVALID_VALUE",
        Some(node),
    );
    refused(
        with(|v| set(v, "/renderProfile/colorSpace", json!("rec709-video"))),
        "REC709_VIDEO_UNSUPPORTED",
        Some("/renderProfile/colorSpace"),
    );
    refused(
        with(|v| set(v, "/renderProfile/alphaMode", json!("premultiplied"))),
        "PREMULTIPLIED_ALPHA_UNSUPPORTED",
        None,
    );
    refused(
        with(|v| set(v, "/renderProfile/premultiplyOnUpload", json!(true))),
        "PREMULTIPLY_UNSUPPORTED",
        None,
    );
    refused(
        with(|v| set(v, "/assets/0/alphaMode", json!("premultiplied"))),
        "ASSET_PREMULTIPLIED_UNSUPPORTED",
        None,
    );
    // linear-srgb is for data maps only; the fixture's surface map is one, bg is not.
    refused(
        with(|v| set(v, "/assets/0/colorSpace", json!("linear-srgb"))),
        "ASSET_LINEAR_SRGB_UNSUPPORTED",
        Some("/assets/0/colorSpace"),
    );
    refused(
        with(|v| {
            set(
                v,
                "/shots/1/transitionInto",
                json!({ "type": "dissolve", "durationFrames": 6 }),
            )
        }),
        "DISSOLVE_UNSUPPORTED",
        None,
    );
    refused(
        with(|v| set(v, "/shots/1/transitionInto", json!({ "type": "wipe" }))),
        "INVALID_VALUE",
        None,
    );
}

#[test]
fn paths_that_leave_the_bundle_folder_are_refused() {
    for bad in [
        "../secret.png",
        "/etc/passwd",
        "a/../../b.png",
        "C:\\x.png",
        "..\\x.png",
    ] {
        refused(
            with(|v| set(v, "/assets/0/relativePath", json!(bad))),
            "UNSAFE_RELATIVE_PATH",
            Some("/assets/0/relativePath"),
        );
    }
    refused(
        with(|v| set(v, "/audio/mixRelativePath", json!("../mix.wav"))),
        "UNSAFE_RELATIVE_PATH",
        Some("/audio/mixRelativePath"),
    );
    // Dots inside a name are fine; only a whole `..` segment climbs out.
    let scene = with(|v| set(v, "/audio/mixRelativePath", json!("audio/v1..2/mix.wav"))).unwrap();
    assert_eq!(
        scene.bundle().audio.as_ref().unwrap().mix_relative_path,
        "audio/v1..2/mix.wav"
    );
}

#[test]
fn shape_problems_hide_integrity_ones() {
    // A wrong type AND a dangling reference: only the shape issue is reported, because the
    // reference check would run on data that did not read.
    let issues = issues_of(with(|v| {
        set(v, "/sceneDefinitions/0/nodes/1/localScale/x", json!("2"));
        set(v, "/shots/0/cameraClipId", json!("ghost"));
    }));
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert_eq!(issues[0].0, "INVALID_TYPE");
    // Several shape problems are all reported, not just the first.
    let issues = issues_of(with(|v| {
        set(v, "/sceneDefinitions/0/nodes/1/localScale/x", json!("2"));
        set(v, "/fps/denominator", json!(0));
        set(v, "/assets/0/assetId", json!(""));
    }));
    assert_eq!(issues.len(), 3, "{issues:?}");
}

// ---- shots and camera -----------------------------------------------------------------------

#[test]
fn rejects_shots_that_do_not_tile_the_edit() {
    refused(
        with(|v| set(v, "/shots/1/editRange/startFrame", json!(25))),
        "EDIT_GAP_OR_OVERLAP",
        None,
    );
    refused(
        with(|v| {
            set(v, "/shots/0/editRange/startFrame", json!(1));
            set(v, "/shots/0/editRange/endFrameExclusive", json!(24));
        }),
        "EDIT_RANGE_START",
        Some("/shots"),
    );
    refused(
        with(|v| set(v, "/editDurationFrames", json!(50))),
        "EDIT_DURATION_MISMATCH",
        None,
    );
    refused(
        with(|v| set(v, "/sceneInstances/0/durationFrames", json!(30))),
        "SHOT_EXCEEDS_INSTANCE_DURATION",
        Some("/shots/1/instanceStartFrame"),
    );
    refused(
        with(|v| set(v, "/shots/0/editRange/endFrameExclusive", json!(0))),
        "INVALID_VALUE",
        None,
    );
    refused(
        with(|v| set(v, "/shots/1/sceneInstanceId", json!("ghost"))),
        "SHOT_UNKNOWN_INSTANCE",
        Some("/shots/1/sceneInstanceId"),
    );
    refused(
        with(|v| set(v, "/shots/1/cameraClipId", json!("ghost"))),
        "SHOT_UNKNOWN_CAMERA_CLIP",
        Some("/shots/1/cameraClipId"),
    );
    // Order in the document does not matter, the start frames do.
    let shuffled = with(|v| {
        let shots = v["shots"].as_array_mut().unwrap();
        shots.reverse();
    })
    .unwrap();
    assert_eq!(shuffled.bundle().shots[0].id, "s2");
    assert_eq!(shuffled.bundle().shot_at_frame(0).unwrap().id, "s1");
}

#[test]
fn rejects_camera_clips_the_origin_rules_refuse() {
    let clip = "/cameraClips/0";
    refused(
        with(|v| set(v, &format!("{clip}/initialFovDeg"), json!(180))),
        "FOV_OUT_OF_RANGE",
        Some("/cameraClips/0/initialFovDeg"),
    );
    refused(
        with(|v| {
            set(
                v,
                &format!("{clip}/initialTarget"),
                json!({ "x": 0, "y": 2, "z": 10 }),
            )
        }),
        "CAMERA_DIRECTION_DEGENERATE",
        Some(clip),
    );
    refused(
        with(|v| set(v, &format!("{clip}/segments/0/easing"), json!("bounce"))),
        "UNKNOWN_CAMERA_EASING",
        Some("/cameraClips/0/segments/0/easing"),
    );
    refused(
        with(|v| set(v, &format!("{clip}/segments/0/durationFrames"), json!(1))),
        "CAMERA_SEGMENT_TOO_SHORT",
        None,
    );
    refused(
        with(|v| set(v, &format!("{clip}/segments/1/keys/0/frame"), json!(0))),
        "CAMERA_KEY_FRAME_ORDER",
        Some("/cameraClips/0/segments/1/keys/0/frame"),
    );
    refused(
        with(|v| set(v, &format!("{clip}/segments/1/keys/1/frame"), json!(22))),
        "CAMERA_KEYS_END_SHORT",
        None,
    );
    refused(
        with(|v| set(v, &format!("{clip}/segments/1/keys/1/fovDeg"), json!(200))),
        "FOV_OUT_OF_RANGE",
        None,
    );
    // A zoom that carries the field of view out of (0, 180).
    refused(
        with(|v| {
            set(
                v,
                &format!("{clip}/segments/0"),
                json!({ "recipe": "zoomIn", "durationFrames": 24, "from": "initial", "fovDeltaDeg": 60, "easing": "linear" }),
            )
        }),
        "FOV_OUT_OF_RANGE",
        Some("/cameraClips/0/segments/0"),
    );
    refused(
        with(|v| {
            set(
                v,
                &format!("{clip}/layers/0/segments/0/from"),
                json!("initial"),
            )
        }),
        "CAMERA_LAYER_FROM_INITIAL",
        None,
    );
    refused(
        with(|v| {
            let keys = v["cameraClips"][0]["segments"][1].clone();
            set(v, &format!("{clip}/layers/0/segments/0"), keys);
        }),
        "CAMERA_LAYER_KEYFRAMES",
        None,
    );
    // A recipe that is not one of the origin's nine, and a payload that does not read.
    refused(
        with(|v| set(v, &format!("{clip}/segments/0/recipe"), json!("crane"))),
        "INVALID_VALUE",
        Some("/cameraClips/0/segments/0/recipe"),
    );
    refused(
        with(|v| remove(v, &format!("{clip}/segments/0"), "distanceSceneUnits")),
        "MISSING_FIELD",
        None,
    );
    // At most 3 stacked tracks.
    refused(
        with(|v| {
            let layer = v["cameraClips"][0]["layers"][0].clone();
            set(
                v,
                &format!("{clip}/layers"),
                json!([layer, layer, layer, layer]),
            );
        }),
        "INVALID_VALUE",
        Some("/cameraClips/0/layers"),
    );
}

#[test]
fn camera_defaults_and_motions_are_typed() {
    let scene = with(|v| {
        set(
            v,
            "/cameraClips/0/layers/0/segments/0",
            json!({ "recipe": "orbit", "durationFrames": 48, "from": "previousEnd", "yawDeg": 10, "pitchDeg": 5, "easing": "linear" }),
        );
    })
    .unwrap();
    let clip = scene.bundle().camera_clip("wide").unwrap();
    assert_eq!(clip.segments.len(), 2);
    assert!(matches!(
        clip.layers[0][0].motion,
        tessel_editor_core::directed_scene::CameraMotion::Orbit { pitch_deg, .. } if pitch_deg == 5.0
    ));
    // pitchDeg absent reads as 0, as the origin schema defaults it, but is not written back.
    let plain = imported();
    assert!(matches!(
        plain.bundle().camera_clip("wide").unwrap().layers[0][0].motion,
        tessel_editor_core::directed_scene::CameraMotion::Orbit { pitch_deg, .. } if pitch_deg == 0.0
    ));
    assert!(plain.document()["cameraClips"][0]["layers"][0]["segments"][0]["pitchDeg"].is_null());
}

#[test]
fn audio_and_legacy_metadata_rules() {
    refused(
        with(|v| remove(v, "/audio", "sourcePcmHashSha256")),
        "AUDIO_HASH_REQUIRED",
        None,
    );
    refused(
        with(|v| set(v, "/audio/sourcePcmHashSha256", json!(""))),
        "AUDIO_HASH_REQUIRED",
        None,
    );
    refused(
        with(|v| set(v, "/audio/editStartSeconds", json!(3.0))),
        "AUDIO_RANGE_INVALID",
        Some("/audio"),
    );
    refused(
        with(|v| set(v, "/metadata/pilotFrameAudit", json!({}))),
        "LEGACY_FRAME_AUDIT",
        None,
    );
    // The audio section is optional.
    assert!(
        with(|v| remove(v, "", "audio"))
            .unwrap()
            .bundle()
            .audio
            .is_none()
    );
}

#[test]
fn edit_joins_are_a_separate_structural_query() {
    let codes = |edit: fn(&mut Value)| -> Vec<String> {
        with(edit)
            .unwrap()
            .edit_join_issues()
            .into_iter()
            .map(|i| i.code.to_string())
            .collect()
    };
    assert_eq!(
        codes(|v| remove(v, "/shots/1", "transitionInto")),
        ["MISSING_TRANSITION"]
    );
    assert_eq!(
        codes(|v| set(
            v,
            "/shots/1/transitionInto",
            json!({ "type": "cut", "reason": "  " })
        )),
        ["CUT_WITHOUT_REASON"]
    );
    assert!(
        codes(|v| set(
            v,
            "/shots/1/transitionInto",
            json!({ "type": "cut", "reason": "new place" })
        ))
        .is_empty()
    );
    // 24 frames of shot 1 map instance frames 0..24, so shot 2 must resume at 24.
    assert_eq!(
        codes(|v| set(v, "/shots/1/instanceStartFrame", json!(30))),
        ["INSTANCE_FRAME_DISCONTINUITY"]
    );
    assert_eq!(
        codes(|v| set(v, "/shots/1/cameraInFrame", json!(10))),
        ["CAMERA_CLIP_OFFSET_BREAK"]
    );
    assert_eq!(
        codes(|v| {
            let clip = v["cameraClips"][0].clone();
            let mut other = clip;
            other["id"] = json!("close");
            push(v, "/cameraClips", other);
            set(v, "/shots/1/cameraClipId", json!("close"));
        }),
        ["CONTINUOUS_CROSS_CLIP_UNSUPPORTED"]
    );
    assert_eq!(
        codes(|v| set(v, "/cameraClips/0/segments/1/from", json!("initial"))),
        ["CAMERA_SEGMENT_RESET"]
    );
}

// ---- limits, errors, integration ------------------------------------------------------------

#[test]
fn enforces_the_limits_before_trusting_the_input() {
    let tight = Limits {
        max_bytes: 100,
        ..Limits::default()
    };
    assert!(matches!(
        DirectedScene::import_str_with(LAYERED, &tight),
        Err(ImportError::TooLarge { max: 100, .. })
    ));

    let few_items = Limits {
        max_items: 3,
        ..Limits::default()
    };
    let issues = issues_of(DirectedScene::import_str_with(LAYERED, &few_items));
    assert!(
        issues.iter().any(|(c, _)| c == "LIMIT_EXCEEDED"),
        "{issues:?}"
    );

    let mut deep = json!(0);
    for _ in 0..200 {
        deep = json!([deep]);
    }
    assert!(matches!(
        DirectedScene::import_value(deep),
        Err(ImportError::Syntax { .. })
    ));
    assert!(matches!(
        DirectedScene::import_str("{"),
        Err(ImportError::Syntax { line: 1, .. })
    ));
    // The default limits are generous enough for the fixture.
    assert!(DirectedScene::import_str_with(LAYERED, &Limits::default()).is_ok());
}

#[test]
fn import_errors_travel_as_command_errors() {
    let error = with(|v| set(v, "/shots/1/cameraClipId", json!("ghost"))).unwrap_err();
    assert_eq!(error.code(), "invalid_bundle");
    assert!(error.to_string().contains("SHOT_UNKNOWN_CAMERA_CLIP"));
    let command: CommandError = error.into();
    assert_eq!(command.code, "invalid_bundle");
    let json = command.to_json();
    assert_eq!(json["ok"], false);
    assert_eq!(
        json["error"]["details"]["issues"][0]["code"],
        "SHOT_UNKNOWN_CAMERA_CLIP"
    );
    assert_eq!(
        json["error"]["details"]["issues"][0]["path"],
        "/shots/1/cameraClipId"
    );

    let syntax: CommandError = DirectedScene::import_str("not json").unwrap_err().into();
    assert_eq!(syntax.code, "invalid_json");
    assert!(DirectedScene::validate_str(LAYERED).is_ok());
    assert!(DirectedScene::validate_str("{}").is_err());
}

#[test]
fn a_failed_import_leaves_the_caller_data_alone() {
    // `import_value` takes ownership, so rejection cannot half-apply anything; the text API
    // never mutates its input. Importing twice gives the same scene.
    assert_eq!(imported(), imported());
    let before = fixture();
    let _ = with(|v| set(v, "/shots/1/cameraClipId", json!("ghost")));
    assert_eq!(fixture(), before);
}

/// Every JSON Pointer of the document (containers and leaves, not the root).
fn pointers(value: &Value, here: &str, out: &mut Vec<String>) {
    let children: Vec<(String, &Value)> = match value {
        Value::Object(map) => map
            .iter()
            .map(|(key, item)| (key.replace('~', "~0").replace('/', "~1"), item))
            .collect(),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(index, item)| (index.to_string(), item))
            .collect(),
        _ => return,
    };
    for (key, item) in children {
        let path = format!("{here}/{key}");
        out.push(path.clone());
        pointers(item, &path, out);
    }
}

#[test]
fn no_single_field_mutation_panics_and_every_refusal_is_explained() {
    let base = fixture();
    let mut all = Vec::new();
    pointers(&base, "", &mut all);
    assert!(
        all.len() > 250,
        "the fixture should be rich enough: {}",
        all.len()
    );

    let junk = [
        Value::Null,
        json!("x"),
        json!(""),
        json!(-1),
        json!(0),
        json!(1.5),
        json!(1e300),
        json!(true),
        json!([]),
        json!({}),
        json!([1, 2, 3]),
    ];
    let mut refused_count = 0;
    for pointer in &all {
        // Replace the value by each kind of junk...
        for replacement in &junk {
            let mut value = base.clone();
            set(&mut value, pointer, replacement.clone());
            if let Err(error) = DirectedScene::import_value(value) {
                refused_count += 1;
                // A refusal always says what and where; never an empty list.
                assert!(
                    !error.issues().is_empty(),
                    "{pointer} <- {replacement}: refused with no issue"
                );
            }
        }
        // ...and remove it.
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        let mut value = base.clone();
        match value.pointer_mut(parent).unwrap() {
            Value::Object(map) => {
                map.remove(&key.replace("~1", "/").replace("~0", "~"));
            }
            Value::Array(items) => {
                items.remove(key.parse::<usize>().unwrap());
            }
            _ => unreachable!(),
        }
        if let Err(error) = DirectedScene::import_value(value) {
            refused_count += 1;
            assert!(!error.issues().is_empty(), "removing {pointer}: no issue");
        }
    }
    assert!(
        refused_count > 1000,
        "mutations should mostly be caught: {refused_count}"
    );
}

#[test]
fn extreme_signed_integers_are_rejected_without_panicking() {
    for value in [i64::MIN, -9_007_199_254_740_992, 9_007_199_254_740_992] {
        let error =
            with(|bundle| bundle["compositionPasses"][0]["order"] = json!(value)).unwrap_err();
        assert!(
            error
                .issues()
                .iter()
                .any(|issue| issue.path == "/compositionPasses/0/order")
        );
    }
}

#[test]
fn javascript_safe_integer_endpoints_remain_valid() {
    for value in [-9_007_199_254_740_991_i64, 9_007_199_254_740_991] {
        with(|bundle| bundle["compositionPasses"][0]["order"] = json!(value)).unwrap();
    }
}
