//! BB04 (minimal): the property catalog and atomic ID-addressed edit batches, through the public
//! library API only (no GPUI, no window, no runtime, no selection).
//!
//! The scene is the synthetic `directed_scene_fixtures/layered_scene.json` (owned by the
//! document/graph tests); nothing here is read from a private bundle. These tests prove the
//! behaviour of this module and its use of the importer. They do not prove equivalence with the
//! origin and they do not certify temporal continuity or render output.

use std::f64::consts::FRAC_1_SQRT_2;

use serde_json::{Value, json};
use tessel_editor_core::CommandError;
use tessel_editor_core::directed_scene::{
    DirectedScene, ImportError, Limits, NodeKind, Quat, Transform, Vec3,
};
use tessel_editor_core::scene_edit::{
    Applicability, CANDIDATE_REJECTED, CameraProperty, EditError, EditOp, INVALID_OP,
    LocalizedIssue, NO_DEFAULT, NOT_APPLICABLE, NodeProperty, OriginRelation, PropertyRef,
    PropertyValue, UNKNOWN_CLIP, UNKNOWN_NODE, UNKNOWN_PATH, UNKNOWN_SCENE, Unit, ValueKind, apply,
    apply_with, catalog, dry_run, parse_batch, properties_of, read_property,
};

const LAYERED: &str = include_str!("directed_scene_fixtures/layered_scene.json");

// Position of each node in `sceneDefinitions[0].nodes` of the fixture: root, stage, bg-plane,
// hero-plane, prop-plane. Importer findings point at these positions.
const HERO_POINTER: &str = "/sceneDefinitions/0/nodes/3";

fn fixture() -> DirectedScene {
    let mut document: Value = serde_json::from_str(LAYERED).expect("the fixture is JSON");
    // The source fixture has a deliberate velocity jump; editing tests start from a valid cut.
    document["shots"][1]["transitionInto"] = json!({
        "type": "cut", "reason": "synthetic editing baseline"
    });
    let scene = DirectedScene::import_value(document).expect("the layered fixture imports");
    assert!(scene.edit_join_issues().is_empty());
    assert!(scene.temporal_issues().is_empty());
    scene
}

fn node(node_id: &str, property: NodeProperty) -> PropertyRef {
    PropertyRef::node("street", node_id, property)
}

fn clip(property: CameraProperty) -> PropertyRef {
    PropertyRef::camera("wide", property)
}

fn vec3(x: f64, y: f64, z: f64) -> PropertyValue {
    PropertyValue::Vec3(Vec3 { x, y, z })
}

fn number(value: f64) -> PropertyValue {
    PropertyValue::Number(value)
}

fn set(property: PropertyRef, value: PropertyValue) -> EditOp {
    EditOp::set(property, value)
}

fn hero_position(x: f64, y: f64, z: f64) -> EditOp {
    set(node("hero-plane", NodeProperty::Position), vec3(x, y, z))
}

#[track_caller]
fn failure(result: Result<impl std::fmt::Debug, EditError>) -> EditError {
    match result {
        Ok(value) => panic!("expected the batch to be refused, got {value:?}"),
        Err(error) => error,
    }
}

/// The importer finding with `code`, whatever else the importer also reported.
#[track_caller]
fn finding_of<'a>(error: &'a EditError, code: &str) -> &'a LocalizedIssue {
    error
        .issues()
        .iter()
        .find(|found| found.issue.code == code)
        .unwrap_or_else(|| panic!("no `{code}` among {:?}", error.issues()))
}

fn set_pointer(document: &mut Value, pointer: &str, value: Value) {
    *document
        .pointer_mut(pointer)
        .unwrap_or_else(|| panic!("fixture has no `{pointer}`")) = value;
}

// ---- catalog and paths ---------------------------------------------------------------------

#[test]
fn catalog_describes_every_property_with_unit_default_range_and_applicability() {
    let specs = catalog();
    let names: Vec<&str> = specs.iter().map(|spec| spec.name).collect();
    assert_eq!(
        names,
        [
            "position",
            "rotation",
            "scale",
            "width",
            "height",
            "start/position",
            "start/target",
            "start/fov"
        ]
    );

    let position = NodeProperty::Position.spec();
    assert_eq!(position.kind, ValueKind::Vec3);
    assert_eq!(position.unit, Unit::SceneUnits);
    assert_eq!(position.default, Some(vec3(0.0, 0.0, 0.0)));
    assert_eq!(position.document_field, "localPosition");
    assert_eq!(position.origin, OriginRelation::EditorOnly);

    let rotation = NodeProperty::Rotation.spec();
    assert_eq!(rotation.kind, ValueKind::Quaternion);
    assert_eq!(
        rotation.default,
        Some(PropertyValue::Quaternion(Quat::IDENTITY))
    );

    let scale = NodeProperty::Scale.spec();
    assert_eq!(scale.unit, Unit::Unitless);
    assert_eq!(scale.default, Some(vec3(1.0, 1.0, 1.0)));

    // A width has no neutral value and applies to image planes only.
    let width = NodeProperty::Width.spec();
    assert_eq!(width.kind, ValueKind::Number);
    assert_eq!(width.default, None);
    assert!(width.applies_to.accepts_node(NodeKind::ImagePlane));
    assert!(!width.applies_to.accepts_node(NodeKind::Group));
    assert!(!width.applies_to.accepts_node(NodeKind::Splat));
    let width_range = width.range.expect("width has a range");
    assert!(width_range.contains(0.25));
    assert!(!width_range.contains(0.0));
    assert!(!width_range.contains(-1.0));

    assert!(
        NodeProperty::Position
            .spec()
            .applies_to
            .accepts_node(NodeKind::Group)
    );
    assert_eq!(
        CameraProperty::StartFov.spec().applies_to,
        Applicability::CameraClip
    );
    assert!(
        !CameraProperty::StartFov
            .spec()
            .applies_to
            .accepts_node(NodeKind::ImagePlane)
    );

    let fov = CameraProperty::StartFov.spec();
    assert_eq!(fov.unit, Unit::Degrees);
    assert_eq!(fov.default, None);
    let fov_range = fov.range.expect("fov has a range");
    assert!(fov_range.contains(50.0) && fov_range.contains(179.999));
    assert!(!fov_range.contains(0.0) && !fov_range.contains(180.0));
    assert_eq!(fov_range.describe(), "greater than 0 and less than 180");

    // The relation to the origin is stated per property, never implied.
    assert_eq!(
        CameraProperty::StartPosition.spec().origin,
        OriginRelation::PathNarrowerWrite
    );
    assert_eq!(
        CameraProperty::StartTarget.spec().origin,
        OriginRelation::CommandOnly
    );
}

#[test]
fn every_default_satisfies_its_own_catalog_entry() {
    for spec in catalog() {
        let Some(default) = spec.default else {
            continue;
        };
        assert_eq!(spec.validate(&default), Ok(()), "{}", spec.name);
    }
}

#[test]
fn paths_follow_the_origin_grammar_and_round_trip() {
    assert_eq!(
        node("hero-plane", NodeProperty::Position).path(),
        "scene/street/node/hero-plane/position"
    );
    // Same spelling as the origin's `cameraStartPath`.
    assert_eq!(
        clip(CameraProperty::StartPosition).path(),
        "camera/wide/start/position"
    );
    assert_eq!(
        clip(CameraProperty::StartFov).path(),
        "camera/wide/start/fov"
    );

    for property in NodeProperty::ALL {
        let reference = node("hero-plane", property);
        assert_eq!(PropertyRef::parse(&reference.path()), Ok(reference));
    }
    for property in CameraProperty::ALL {
        let reference = clip(property);
        assert_eq!(PropertyRef::parse(&reference.path()), Ok(reference));
    }

    // IDs are URI-encoded so a `/` in an ID is not taken for two segments.
    let odd = PropertyRef::node("a/b c", "é", NodeProperty::Scale);
    assert_eq!(odd.path(), "scene/a%2Fb%20c/node/%C3%A9/scale");
    assert_eq!(PropertyRef::parse(&odd.path()), Ok(odd));

    // A literal space is read as well; the canonical spelling is the encoded one.
    let spaced = PropertyRef::parse("scene/street/node/hero plane/width").expect("parses");
    assert_eq!(spaced.path(), "scene/street/node/hero%20plane/width");
}

#[test]
fn unknown_paths_are_refused_with_the_known_names() {
    let refused = |path: &str| {
        let error = failure(PropertyRef::parse(path));
        assert_eq!(error.code, UNKNOWN_PATH, "{path}");
        assert_eq!(error.path.as_deref(), Some(path));
        error
    };
    let unknown_name = refused("scene/street/node/hero-plane/opacity");
    assert!(unknown_name.message.contains("position"), "{unknown_name}");
    let unknown_camera = refused("camera/wide/start/roll");
    assert!(
        unknown_camera.message.contains("start/fov"),
        "{unknown_camera}"
    );
    refused("light/lamp/intensity");
    refused("scene/street/node//position");
    refused("scene/street/node/hero-plane/position/x");
    refused("camera/wide/track/0/segment/0/end/position");
    refused("scene/street/node/hero-plane/position%");
    refused("scene/street/node/hero-plane/position%zz");
    refused("scene/street/node/%FF/position");
}

#[test]
fn properties_of_lists_what_applies_with_current_values_and_no_selection() {
    let scene = fixture();
    let properties = properties_of(&scene);
    // Groups root and stage: 3 each. Image planes bg, hero and prop: 5 each. One clip: 3.
    assert_eq!(properties.len(), 6 + 15 + 3);

    let value_of = |reference: PropertyRef| {
        properties
            .iter()
            .find(|info| info.property == reference)
            .map(|info| info.value)
    };
    assert_eq!(
        value_of(node("hero-plane", NodeProperty::Position)),
        Some(vec3(3.0, 0.0, 0.0))
    );
    assert_eq!(
        value_of(node("hero-plane", NodeProperty::Rotation)),
        Some(PropertyValue::Quaternion(Quat {
            x: 0.0,
            y: FRAC_1_SQRT_2,
            z: 0.0,
            w: FRAC_1_SQRT_2
        }))
    );
    assert_eq!(
        value_of(node("stage", NodeProperty::Scale)),
        Some(vec3(2.0, 2.0, 2.0))
    );
    assert_eq!(
        value_of(node("bg-plane", NodeProperty::Height)),
        Some(number(9.0))
    );
    assert_eq!(
        value_of(clip(CameraProperty::StartPosition)),
        Some(vec3(0.0, 2.0, 10.0))
    );
    assert_eq!(value_of(clip(CameraProperty::StartFov)), Some(number(50.0)));
    // A group has no width to list.
    assert_eq!(value_of(node("stage", NodeProperty::Width)), None);

    assert_eq!(
        read_property(&scene, &node("hero-plane", NodeProperty::Width)),
        Ok(number(1.0))
    );
    let group_width = failure(read_property(&scene, &node("stage", NodeProperty::Width)));
    assert_eq!(group_width.code, NOT_APPLICABLE);
    assert_eq!(group_width.id.as_deref(), Some("stage"));
}

// ---- batches -------------------------------------------------------------------------------

#[test]
fn a_valid_batch_produces_a_candidate_and_a_summary_without_touching_the_input() {
    let scene = fixture();
    let before = scene.clone();
    let ops = [
        hero_position(4.0, 1.0, 0.0),
        set(node("stage", NodeProperty::Scale), vec3(3.0, 3.0, 3.0)),
        set(node("bg-plane", NodeProperty::Width), number(20.0)),
        set(node("bg-plane", NodeProperty::Height), number(11.25)),
        set(clip(CameraProperty::StartFov), number(40.0)),
        set(clip(CameraProperty::StartTarget), vec3(0.0, 1.5, 0.0)),
        set(
            node("hero-plane", NodeProperty::Rotation),
            PropertyValue::Quaternion(Quat::IDENTITY),
        ),
    ];
    let outcome = apply(&scene, &ops).expect("the batch is valid");

    assert_eq!(scene, before, "the input scene is immutable");

    let summary = &outcome.summary;
    assert!(summary.modified);
    assert_eq!(summary.changes.len(), 7);
    assert_eq!(summary.changed_count(), 7);
    assert_eq!(summary.changes[0].index, 0);
    assert_eq!(summary.changes[0].before, vec3(3.0, 0.0, 0.0));
    assert_eq!(summary.changes[0].after, vec3(4.0, 1.0, 0.0));
    assert_eq!(
        summary.changes[0].property.path(),
        "scene/street/node/hero-plane/position"
    );
    assert_eq!(summary.changes[2].before, number(16.0));
    assert_eq!(summary.changes[2].after, number(20.0));
    assert_eq!(summary.changes[4].before, number(50.0));

    let street = outcome.candidate.bundle().scene("street").expect("scene");
    let hero = street.node("hero-plane").expect("node");
    assert_eq!(
        hero.transform.position,
        Vec3 {
            x: 4.0,
            y: 1.0,
            z: 0.0
        }
    );
    assert_eq!(hero.transform.rotation, Quat::IDENTITY);
    assert_eq!(
        street.node("stage").expect("node").transform.scale,
        Vec3 {
            x: 3.0,
            y: 3.0,
            z: 3.0
        }
    );
    let background = street.node("bg-plane").expect("node");
    assert_eq!(background.width_scene_units, Some(20.0));
    assert_eq!(background.height_scene_units, Some(11.25));
    let camera = outcome
        .candidate
        .bundle()
        .camera_clip("wide")
        .expect("clip");
    assert_eq!(camera.initial_fov_deg, 40.0);
    assert_eq!(
        camera.initial_target,
        Vec3 {
            x: 0.0,
            y: 1.5,
            z: 0.0
        }
    );

    // The graph is the importer's, so the world matrix already reflects the edit: stage at
    // (1, 2, 3) scaled by 3 carries the hero at local (4, 1, 0) to (13, 5, 3).
    let world = street.world_matrix("hero-plane").expect("world matrix");
    for (axis, expected) in [(12, 13.0), (13, 5.0), (14, 3.0)] {
        assert!(
            (world.0[axis] - expected).abs() < 1e-9,
            "axis {axis}: {}",
            world.0[axis]
        );
    }
    // And the original scene still has the old one: (1,2,3) + 2 * (3,0,0) = (7, 2, 3).
    let original = scene.bundle().scene("street").expect("scene");
    assert!((original.world_matrix("hero-plane").expect("matrix").0[12] - 7.0).abs() < 1e-9);
}

#[test]
fn dry_run_reports_exactly_what_apply_would_and_changes_nothing() {
    let scene = fixture();
    let before = scene.clone();
    let ops = [
        hero_position(4.0, 1.0, 0.0),
        set(clip(CameraProperty::StartFov), number(40.0)),
    ];
    let summary = dry_run(&scene, &ops).expect("valid");
    let outcome = apply(&scene, &ops).expect("valid");
    assert_eq!(summary, outcome.summary);
    assert_eq!(scene, before);
    assert_eq!(scene.to_json_string(), before.to_json_string());

    let bad = [
        hero_position(4.0, 1.0, 0.0),
        set(node("stage", NodeProperty::Width), number(1.0)),
    ];
    assert_eq!(failure(dry_run(&scene, &bad)), failure(apply(&scene, &bad)));
    assert_eq!(scene, before);
}

#[test]
fn a_late_failure_rolls_back_the_whole_batch() {
    let scene = fixture();
    let before = scene.clone();
    let ops = [
        hero_position(4.0, 1.0, 0.0),
        set(node("bg-plane", NodeProperty::Width), number(20.0)),
        set(node("ghost", NodeProperty::Position), vec3(1.0, 1.0, 1.0)),
    ];
    let error = failure(apply(&scene, &ops));
    assert_eq!(error.code, UNKNOWN_NODE);
    assert_eq!(error.index, Some(2));
    assert_eq!(error.id.as_deref(), Some("ghost"));
    assert_eq!(
        error.path.as_deref(),
        Some("scene/street/node/ghost/position")
    );
    assert_eq!(scene, before, "ops 0 and 1 left no trace");

    // The first op that fails ends the batch, even if a later one is worse.
    let two_bad = [
        set(node("bg-plane", NodeProperty::Width), number(0.0)),
        set(node("ghost", NodeProperty::Position), vec3(1.0, 1.0, 1.0)),
    ];
    assert_eq!(failure(apply(&scene, &two_bad)).index, Some(0));
}

#[test]
fn a_candidate_the_importer_refuses_is_localized_to_the_op_and_pointer() {
    let scene = fixture();
    let before = scene.clone();
    // Finite and well typed, so the catalog accepts it; the importer owns the unit-length rule.
    let ops = [
        set(node("bg-plane", NodeProperty::Width), number(20.0)),
        set(
            node("hero-plane", NodeProperty::Rotation),
            PropertyValue::Quaternion(Quat {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 2.0,
            }),
        ),
    ];
    let error = failure(apply(&scene, &ops));
    assert_eq!(error.code, CANDIDATE_REJECTED);
    assert_eq!(error.index, Some(1));
    let finding = finding_of(&error, "QUATERNION_NOT_NORMALIZED");
    assert_eq!(finding.issue.path, format!("{HERO_POINTER}/localRotation"));
    assert_eq!(finding.ops, [1]);
    assert!(matches!(error.cause(), Some(ImportError::Invalid { .. })));
    assert_eq!(scene, before);

    let zero = [set(
        node("hero-plane", NodeProperty::Rotation),
        PropertyValue::Quaternion(Quat {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            w: 0.0,
        }),
    )];
    let degenerate = failure(apply(&scene, &zero));
    assert_eq!(finding_of(&degenerate, "DEGENERATE_QUATERNION").ops, [0]);

    // A camera rule: the importer points at the clip itself, which is where the op wrote.
    let looking_at_itself = [set(clip(CameraProperty::StartTarget), vec3(0.0, 2.0, 10.0))];
    let camera = failure(apply(&scene, &looking_at_itself));
    let finding = finding_of(&camera, "CAMERA_DIRECTION_DEGENERATE");
    assert_eq!(finding.issue.path, "/cameraClips/0");
    assert_eq!(finding.ops, [0]);
    assert_eq!(camera.index, Some(0));
    assert_eq!(scene, before);
}

#[test]
fn a_consequence_outside_the_touched_node_has_no_op_but_keeps_its_pointer() {
    let scene = fixture();
    // The stage itself stays finite; its children's world matrices overflow.
    let ops = [set(
        node("stage", NodeProperty::Scale),
        vec3(1e308, 1e308, 1e308),
    )];
    let error = failure(apply(&scene, &ops));
    assert_eq!(error.code, CANDIDATE_REJECTED);
    assert_eq!(error.index, None);
    assert!(!error.issues().is_empty());
    for finding in error.issues() {
        assert_eq!(finding.issue.code, "NON_FINITE_TRANSFORM");
        assert!(finding.issue.path.starts_with("/sceneDefinitions/0/nodes/"));
        assert!(finding.ops.is_empty(), "{finding:?}");
    }
}

#[test]
fn ids_that_do_not_exist_are_reported_by_kind() {
    let scene = fixture();

    let no_scene = failure(apply(
        &scene,
        &[set(
            PropertyRef::node("ghost", "hero-plane", NodeProperty::Position),
            vec3(0.0, 0.0, 0.0),
        )],
    ));
    assert_eq!(
        (no_scene.code, no_scene.id.as_deref()),
        (UNKNOWN_SCENE, Some("ghost"))
    );
    assert_eq!(no_scene.index, Some(0));

    let no_node = failure(apply(
        &scene,
        &[
            hero_position(1.0, 1.0, 1.0),
            set(node("ghost", NodeProperty::Scale), vec3(1.0, 1.0, 1.0)),
        ],
    ));
    assert_eq!(
        (no_node.code, no_node.id.as_deref()),
        (UNKNOWN_NODE, Some("ghost"))
    );
    assert_eq!(no_node.index, Some(1));

    let no_clip = failure(apply(
        &scene,
        &[set(
            PropertyRef::camera("ghost", CameraProperty::StartFov),
            number(40.0),
        )],
    ));
    assert_eq!(
        (no_clip.code, no_clip.id.as_deref()),
        (UNKNOWN_CLIP, Some("ghost"))
    );

    // A node ID belongs to its scene: `street`'s nodes do not exist under another scene ID, and
    // a clip ID is not a node ID.
    assert_eq!(
        failure(read_property(
            &scene,
            &PropertyRef::node("wide", "hero-plane", NodeProperty::Position)
        ))
        .code,
        UNKNOWN_SCENE
    );
}

#[test]
fn wrong_types_and_out_of_range_values_are_refused_with_index_and_path() {
    let scene = fixture();
    let refused = |op: EditOp, code: &str| {
        // A valid op first, so the reported index is the second one.
        let error = failure(apply(&scene, &[hero_position(4.0, 1.0, 0.0), op.clone()]));
        assert_eq!(error.code, code, "{op:?}: {error}");
        assert_eq!(error.index, Some(1));
        assert_eq!(error.path, Some(op.property().path()));
        error
    };

    // Wrong type: the kind the entry declares is the only one accepted.
    let wrong = refused(
        set(node("hero-plane", NodeProperty::Position), number(1.0)),
        "INVALID_TYPE",
    );
    assert!(wrong.message.contains("position"), "{wrong}");
    refused(
        set(node("hero-plane", NodeProperty::Width), vec3(1.0, 1.0, 1.0)),
        "INVALID_TYPE",
    );
    refused(
        set(
            node("hero-plane", NodeProperty::Position),
            PropertyValue::Quaternion(Quat::IDENTITY),
        ),
        "INVALID_TYPE",
    );

    // Range and finiteness.
    for width in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        refused(
            set(node("hero-plane", NodeProperty::Width), number(width)),
            "INVALID_VALUE",
        );
    }
    for fov in [0.0, 180.0, 200.0, -5.0] {
        refused(
            set(clip(CameraProperty::StartFov), number(fov)),
            "INVALID_VALUE",
        );
    }
    refused(
        set(
            node("hero-plane", NodeProperty::Position),
            vec3(f64::NAN, 0.0, 0.0),
        ),
        "INVALID_VALUE",
    );
    refused(
        set(
            clip(CameraProperty::StartTarget),
            vec3(0.0, f64::NEG_INFINITY, 0.0),
        ),
        "INVALID_VALUE",
    );

    // The edges the range allows.
    for fov in [0.001, 179.999] {
        assert!(apply(&scene, &[set(clip(CameraProperty::StartFov), number(fov))]).is_ok());
    }
}

#[test]
fn a_property_that_does_not_apply_to_the_node_is_refused() {
    let scene = fixture();
    let error = failure(apply(
        &scene,
        &[set(node("stage", NodeProperty::Width), number(2.0))],
    ));
    assert_eq!(error.code, NOT_APPLICABLE);
    assert_eq!(error.id.as_deref(), Some("stage"));
    assert_eq!(error.index, Some(0));
    assert!(error.message.contains("imagePlane"), "{error}");
}

#[test]
fn reset_writes_the_catalog_default_or_is_refused_when_there_is_none() {
    let scene = fixture();
    let outcome = apply(
        &scene,
        &[
            EditOp::reset(node("stage", NodeProperty::Position)),
            EditOp::reset(node("stage", NodeProperty::Rotation)),
            EditOp::reset(node("stage", NodeProperty::Scale)),
        ],
    )
    .expect("every reset has a default");
    let stage = outcome
        .candidate
        .bundle()
        .scene("street")
        .expect("scene")
        .node("stage")
        .expect("node")
        .transform;
    assert_eq!(stage, Transform::IDENTITY);
    assert_eq!(outcome.summary.changes[0].before, vec3(1.0, 2.0, 3.0));
    assert_eq!(outcome.summary.changes[2].before, vec3(2.0, 2.0, 2.0));
    // The rotation already was the identity: recorded, but the document is left alone.
    assert!(outcome.summary.changes[1].is_noop());
    assert_eq!(outcome.summary.changed_count(), 2);

    for reference in [
        node("hero-plane", NodeProperty::Width),
        clip(CameraProperty::StartPosition),
        clip(CameraProperty::StartFov),
    ] {
        let error = failure(apply(&scene, &[EditOp::reset(reference.clone())]));
        assert_eq!(error.code, NO_DEFAULT, "{reference}");
        assert_eq!(error.index, Some(0));
        assert_eq!(error.path, Some(reference.path()));
    }
}

#[test]
fn references_unknown_fields_and_everything_else_are_preserved() {
    let scene = fixture();
    let ops = [
        hero_position(4.0, 1.0, 0.0),
        set(node("bg-plane", NodeProperty::Width), number(20.0)),
        set(clip(CameraProperty::StartFov), number(40.0)),
    ];
    let outcome = apply(&scene, &ops).expect("valid");
    let candidate = &outcome.candidate;

    // The candidate differs from the input in exactly the three fields edited.
    let mut expected = scene.document().clone();
    set_pointer(
        &mut expected,
        "/sceneDefinitions/0/nodes/3/localPosition",
        json!({ "x": 4, "y": 1, "z": 0 }),
    );
    set_pointer(
        &mut expected,
        "/sceneDefinitions/0/nodes/2/widthSceneUnits",
        json!(20),
    );
    set_pointer(&mut expected, "/cameraClips/0/initialFovDeg", json!(40));
    assert_eq!(candidate.document(), &expected);

    // Unknown fields, in the node and at the top, survive; the notices are the same.
    assert_eq!(
        candidate
            .document()
            .pointer("/sceneDefinitions/0/nodes/1/x-note"),
        scene
            .document()
            .pointer("/sceneDefinitions/0/nodes/1/x-note")
    );
    assert!(candidate.document().get("x-tool").is_some());
    assert_eq!(candidate.notices(), scene.notices());

    // Every reference still resolves to the same entity, and the structure around is unchanged.
    let (new, old) = (candidate.bundle(), scene.bundle());
    assert_eq!(new.references(), old.references());
    assert_eq!(new.assets, old.assets);
    assert_eq!(new.shots, old.shots);
    assert_eq!(new.instances, old.instances);
    assert_eq!(new.composition_passes, old.composition_passes);
    assert_eq!(
        new.camera_clip("wide").expect("clip").segments,
        old.camera_clip("wide").expect("clip").segments
    );
    assert_eq!(
        new.paint_order("street").expect("order").len(),
        old.paint_order("street").expect("order").len()
    );
    // The structural gate of the shot joins sees the same thing; it is not a temporal gate.
    assert_eq!(candidate.edit_join_issues(), scene.edit_join_issues());
}

#[test]
fn an_op_that_writes_the_current_value_leaves_the_document_identical() {
    let scene = fixture();
    let ops = [
        hero_position(3.0, 0.0, 0.0),
        set(node("hero-plane", NodeProperty::Width), number(1.0)),
        set(clip(CameraProperty::StartFov), number(50.0)),
    ];
    let outcome = apply(&scene, &ops).expect("valid");
    assert!(
        outcome
            .summary
            .changes
            .iter()
            .all(|change| change.is_noop())
    );
    assert!(!outcome.summary.modified);
    assert_eq!(outcome.summary.changed_count(), 0);
    assert_eq!(outcome.candidate.document(), scene.document());
    assert_eq!(outcome.candidate, scene);
}

#[test]
fn later_ops_see_earlier_ones_and_restoring_a_value_is_not_a_modification() {
    let scene = fixture();
    let stage = |x, y, z| set(node("stage", NodeProperty::Position), vec3(x, y, z));

    let restored = apply(&scene, &[stage(9.0, 9.0, 9.0), stage(1.0, 2.0, 3.0)]).expect("valid");
    assert_eq!(restored.summary.changes[1].before, vec3(9.0, 9.0, 9.0));
    assert_eq!(restored.summary.changed_count(), 2);
    assert!(!restored.summary.modified, "net effect is none");
    assert_eq!(restored.candidate.document(), scene.document());

    let moved = apply(&scene, &[stage(9.0, 9.0, 9.0), stage(5.0, 5.0, 5.0)]).expect("valid");
    assert_eq!(moved.summary.changes[0].before, vec3(1.0, 2.0, 3.0));
    assert_eq!(moved.summary.changes[1].before, vec3(9.0, 9.0, 9.0));
    assert!(moved.summary.modified);
}

#[test]
fn an_empty_batch_is_a_valid_no_op() {
    let scene = fixture();
    let outcome = apply(&scene, &[]).expect("valid");
    assert_eq!(outcome.candidate, scene);
    assert!(outcome.summary.changes.is_empty());
    assert!(!outcome.summary.modified);
}

#[test]
fn a_candidate_survives_a_round_trip_through_text() {
    let scene = fixture();
    let outcome = apply(
        &scene,
        &[
            hero_position(4.5, 1.0, -2.0),
            set(node("bg-plane", NodeProperty::Height), number(11.25)),
        ],
    )
    .expect("valid");
    let reimported =
        DirectedScene::import_str(&outcome.candidate.to_json_string()).expect("imports again");
    assert_eq!(reimported.bundle(), outcome.candidate.bundle());
    let pretty =
        DirectedScene::import_str(&outcome.candidate.to_json_pretty()).expect("imports again");
    assert_eq!(pretty.bundle(), outcome.candidate.bundle());
}

#[test]
fn ids_with_a_slash_or_a_space_are_addressed_through_their_encoded_path() {
    let text = fixture()
        .to_json_string()
        .replace("\"street\"", "\"a/b c\"");
    let scene = DirectedScene::import_str(&text).expect("renamed fixture imports");
    let op = EditOp::from_json(&json!({
        "set": "scene/a%2Fb%20c/node/hero-plane/position",
        "value": { "x": 4, "y": 1, "z": 0 }
    }))
    .expect("decodes");
    let outcome = apply(&scene, &[op]).expect("valid");
    let hero = outcome
        .candidate
        .bundle()
        .scene("a/b c")
        .expect("scene")
        .node("hero-plane")
        .expect("node");
    assert_eq!(
        hero.transform.position,
        Vec3 {
            x: 4.0,
            y: 1.0,
            z: 0.0
        }
    );
}

#[test]
fn limits_of_the_import_are_the_ones_the_candidate_is_checked_with() {
    let scene = fixture();
    let tight = Limits {
        max_items: 1,
        ..Limits::default()
    };
    // `nodes` has five items, which an import with `max_items: 1` refuses outright.
    let error = failure(apply_with(&scene, &[hero_position(4.0, 1.0, 0.0)], &tight));
    assert_eq!(error.code, CANDIDATE_REJECTED);
    finding_of(&error, "LIMIT_EXCEEDED");
}

// ---- JSON ops ------------------------------------------------------------------------------

#[test]
fn json_ops_decode_by_the_kind_the_catalog_declares() {
    let set_position = EditOp::from_json(&json!({
        "set": "scene/street/node/hero-plane/position",
        "value": { "x": 4, "y": 1, "z": 0 }
    }))
    .expect("decodes");
    assert_eq!(set_position, hero_position(4.0, 1.0, 0.0));

    let set_fov = EditOp::from_json(&json!({ "set": "camera/wide/start/fov", "value": 40 }))
        .expect("decodes");
    assert_eq!(set_fov, set(clip(CameraProperty::StartFov), number(40.0)));

    let reset =
        EditOp::from_json(&json!({ "reset": "scene/street/node/stage/scale" })).expect("decodes");
    assert_eq!(reset, EditOp::reset(node("stage", NodeProperty::Scale)));

    // Decoding checks the shape only: the range is the batch's check, with the op's index.
    let zero_width =
        EditOp::from_json(&json!({ "set": "scene/street/node/hero-plane/width", "value": 0 }))
            .expect("decodes");
    let error = failure(apply(&fixture(), &[zero_width]));
    assert_eq!((error.code, error.index), ("INVALID_VALUE", Some(0)));
}

#[test]
fn json_ops_with_the_wrong_shape_are_refused_before_they_reach_the_scene() {
    let position = "scene/street/node/hero-plane/position";
    let wrong_type = |value: Value| {
        let error = failure(EditOp::from_json(
            &json!({ "set": position, "value": value }),
        ));
        assert_eq!(error.code, "INVALID_TYPE", "{error}");
        assert_eq!(error.path.as_deref(), Some(position));
    };
    wrong_type(json!(3));
    wrong_type(json!("1,2,3"));
    wrong_type(json!([1, 2, 3]));
    wrong_type(json!(null));
    wrong_type(json!({ "x": 1, "y": 2 }));
    wrong_type(json!({ "x": 1, "y": 2, "z": "3" }));
    wrong_type(json!({ "x": 1, "y": 2, "z": 3, "w": 4 }));

    let wrong_width = failure(EditOp::from_json(
        &json!({ "set": "scene/street/node/hero-plane/width", "value": "wide" }),
    ));
    assert_eq!(wrong_width.code, "INVALID_TYPE");
    let wrong_rotation = failure(EditOp::from_json(&json!({
        "set": "scene/street/node/hero-plane/rotation",
        "value": { "x": 0, "y": 0, "z": 0 }
    })));
    assert_eq!(wrong_rotation.code, "INVALID_TYPE");

    let unknown = failure(EditOp::from_json(
        &json!({ "set": "scene/street/node/hero-plane/opacity", "value": 1 }),
    ));
    assert_eq!(unknown.code, UNKNOWN_PATH);

    for malformed in [
        json!("text"),
        json!({ "foo": 1 }),
        json!({ "set": position }),
        json!({ "value": 1 }),
        json!({ "set": 3, "value": 1 }),
        json!({ "set": position, "value": { "x": 1, "y": 2, "z": 3 }, "extra": true }),
        json!({ "reset": position, "value": 1 }),
        json!({ "reset": 7 }),
    ] {
        assert_eq!(
            failure(EditOp::from_json(&malformed)).code,
            INVALID_OP,
            "{malformed}"
        );
    }
}

#[test]
fn a_json_batch_reports_the_position_of_the_op_that_does_not_decode() {
    let ok = json!({ "set": "scene/street/node/hero-plane/width", "value": 2 });
    let bad = json!({ "set": "scene/street/node/hero-plane/width", "value": "two" });
    let batch = parse_batch(&json!([ok.clone(), ok.clone()])).expect("decodes");
    assert_eq!(batch.len(), 2);

    let error = failure(parse_batch(&json!([ok.clone(), bad, ok])));
    assert_eq!(error.code, "INVALID_TYPE");
    assert_eq!(error.index, Some(1));

    assert_eq!(
        failure(parse_batch(&json!({ "set": "x" }))).code,
        INVALID_OP
    );
}

// ---- agreement with the importer -----------------------------------------------------------

/// Whether the importer accepts the fixture with the value at `pointer` replaced. The catalog's
/// ranges state what a property accepts; the importer decides what a bundle accepts. They must
/// agree at the edges, or an inspector would offer values the importer refuses (or the reverse).
fn importer_accepts(pointer: &str, value: f64) -> bool {
    let mut document: Value = serde_json::from_str(LAYERED).expect("fixture is JSON");
    set_pointer(&mut document, pointer, json!(value));
    DirectedScene::import_value(document).is_ok()
}

#[test]
fn catalog_ranges_agree_with_the_importer_at_the_edges() {
    let fov = CameraProperty::StartFov.spec();
    for value in [-1.0, 0.0, 0.001, 50.0, 179.999, 180.0, 200.0] {
        assert_eq!(
            fov.validate(&number(value)).is_ok(),
            importer_accepts("/cameraClips/0/initialFovDeg", value),
            "fov {value}"
        );
    }
    let width = NodeProperty::Width.spec();
    for value in [-1.0, 0.0, 1e-9, 0.25, 1e9] {
        assert_eq!(
            width.validate(&number(value)).is_ok(),
            importer_accepts("/sceneDefinitions/0/nodes/3/widthSceneUnits", value),
            "width {value}"
        );
    }
    let height = NodeProperty::Height.spec();
    for value in [-1.0, 0.0, 1e-9, 2.0] {
        assert_eq!(
            height.validate(&number(value)).is_ok(),
            importer_accepts("/sceneDefinitions/0/nodes/3/heightSceneUnits", value),
            "height {value}"
        );
    }
}

// ---- adapters speak CommandError -----------------------------------------------------------

#[test]
fn edit_errors_convert_to_command_errors_with_position_and_findings() {
    let scene = fixture();

    let missing = failure(apply(
        &scene,
        &[
            hero_position(1.0, 1.0, 1.0),
            set(node("ghost", NodeProperty::Scale), vec3(1.0, 1.0, 1.0)),
        ],
    ));
    let command = CommandError::from(missing);
    assert_eq!(command.code, "unknown_node");
    assert_eq!(command.target.as_deref(), Some("ghost"));
    let details = command.details.as_deref().expect("details");
    assert_eq!(details["index"], json!(1));
    assert_eq!(details["path"], json!("scene/street/node/ghost/scale"));

    let rejected = failure(apply(
        &scene,
        &[set(
            node("hero-plane", NodeProperty::Rotation),
            PropertyValue::Quaternion(Quat {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 2.0,
            }),
        )],
    ));
    let command = CommandError::from(rejected);
    // The same code an import of that document would give.
    assert_eq!(command.code, "invalid_bundle");
    let details = command.details.as_deref().expect("details");
    assert_eq!(details["index"], json!(0));
    assert_eq!(
        details["issues"][0]["code"],
        json!("QUATERNION_NOT_NORMALIZED")
    );
    assert_eq!(details["issues"][0]["ops"], json!([0]));

    let range = failure(apply(
        &scene,
        &[set(clip(CameraProperty::StartFov), number(180.0))],
    ));
    assert_eq!(CommandError::from(range).code, "invalid_value");
    let wrong = failure(apply(
        &scene,
        &[set(clip(CameraProperty::StartFov), vec3(1.0, 1.0, 1.0))],
    ));
    assert_eq!(CommandError::from(wrong).code, "invalid_type");
}

#[test]
fn batch_rejects_temporally_broken_input_instead_of_returning_a_candidate() {
    let scene = DirectedScene::import_str(LAYERED).unwrap();
    let before = scene.clone();
    let error = apply(&scene, &[hero_position(8.0, 1.0, 0.0)]).unwrap_err();
    assert_eq!(error.code, CANDIDATE_REJECTED);
    finding_of(&error, "CONTINUOUS_VELOCITY_JUMP");
    assert_eq!(scene, before);
    assert!(dry_run(&scene, &[]).is_err());
}

#[test]
fn batch_rejects_a_structurally_broken_join_and_keeps_the_input() {
    let mut raw = fixture().into_document();
    raw["shots"][1]
        .as_object_mut()
        .unwrap()
        .remove("transitionInto");
    let scene = DirectedScene::import_value(raw).unwrap();
    let before = scene.clone();
    let error = apply(&scene, &[]).unwrap_err();
    finding_of(&error, "MISSING_TRANSITION");
    assert_eq!(scene, before);
}

#[test]
fn batch_rejects_an_edit_that_breaks_composed_camera_fov_and_dry_run_agrees() {
    let mut raw = fixture().into_document();
    raw["shots"][1]["transitionInto"] = json!({"type": "cut", "reason": "synthetic edit test"});
    raw["cameraClips"][0]["segments"] = json!([
        {"recipe": "hold", "durationFrames": 48, "from": "initial"}
    ]);
    raw["cameraClips"][0]["layers"] = json!([{"segments": [
        {"recipe": "zoomOut", "durationFrames": 48, "from": "previousEnd",
         "fovDeltaDeg": 20, "easing": "linear"}
    ]}]);
    let scene = DirectedScene::import_value(raw).unwrap();
    assert!(scene.temporal_issues().is_empty());
    let before = scene.clone();
    let ops = [set(clip(CameraProperty::StartFov), number(170.0))];
    let error = apply(&scene, &ops).unwrap_err();
    finding_of(&error, "FOV_OUT_OF_RANGE");
    assert_eq!(error.index, Some(0));
    let dry_error = dry_run(&scene, &ops).unwrap_err();
    assert_eq!(dry_error, error);
    assert_eq!(scene, before);
}
