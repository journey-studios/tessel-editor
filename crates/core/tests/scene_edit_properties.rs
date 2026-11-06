//! S6 (BL004/BL005): typed component properties, `renderOrder` and the origin-equivalent write of
//! a camera's start pose, through the public library API only (no GPUI, no window, no runtime).
//!
//! Uses the synthetic `layered_scene.json` fixture. The equivalence with the origin of the camera
//! conversion is proved against the fixed oracle in `scene_edit_oracle.rs`; this file proves the
//! batch semantics (atomic, reset, no-op, order) and the invariants of the conversion.

use serde_json::{Value, json};
use tessel_editor_core::CommandError;
use tessel_editor_core::directed_scene::{DirectedScene, Vec3};
use tessel_editor_core::scene_edit::{
    CAMERA_CONVERSION_FAILED, CANDIDATE_REJECTED, CameraProperty, ComponentOf, EditError, EditOp,
    INVALID_OP, NO_DEFAULT, NOT_APPLICABLE, NodeProperty, OriginRelation, PropertyId, PropertyRef,
    PropertyValue, Unit, ValueKind, apply, catalog, dry_run, properties_of,
    properties_with_components, read_property,
};

const LAYERED: &str = include_str!("directed_scene_fixtures/layered_scene.json");

fn document() -> Value {
    let mut document: Value = serde_json::from_str(LAYERED).expect("the fixture is JSON");
    document["shots"][1]["transitionInto"] = json!({
        "type": "cut", "reason": "synthetic editing baseline"
    });
    document
}

fn scene_of(document: Value) -> DirectedScene {
    DirectedScene::import_value(document).expect("the document imports")
}

fn fixture() -> DirectedScene {
    scene_of(document())
}

fn node(node_id: &str, property: NodeProperty) -> PropertyRef {
    PropertyRef::node("street", node_id, property)
}

fn clip(property: CameraProperty) -> PropertyRef {
    PropertyRef::camera("wide", property)
}

fn number(value: f64) -> PropertyValue {
    PropertyValue::Number(value)
}

fn vec3(x: f64, y: f64, z: f64) -> PropertyValue {
    PropertyValue::Vec3(Vec3 { x, y, z })
}

fn set(property: PropertyRef, value: PropertyValue) -> EditOp {
    EditOp::set(property, value)
}

#[track_caller]
fn failure(result: Result<impl std::fmt::Debug, EditError>) -> EditError {
    match result {
        Ok(value) => panic!("expected the batch to be refused, got {value:?}"),
        Err(error) => error,
    }
}

fn hero_json(scene: &DirectedScene) -> &Value {
    &scene.document()["sceneDefinitions"][0]["nodes"][3]
}

fn prop_json(scene: &DirectedScene) -> &Value {
    &scene.document()["sceneDefinitions"][0]["nodes"][4]
}

fn clip_json(scene: &DirectedScene) -> &Value {
    &scene.document()["cameraClips"][0]
}

/// A JSON number whatever its spelling (`2` and `2.0` are the same coordinate).
#[track_caller]
fn xyz(value: &Value) -> [f64; 3] {
    ["x", "y", "z"].map(|axis| {
        value[axis]
            .as_f64()
            .unwrap_or_else(|| panic!("{value}.{axis}"))
    })
}

fn path_of(property: &PropertyRef) -> String {
    property.path()
}

// ---- catalog -------------------------------------------------------------------------------

#[test]
fn components_are_catalogued_next_to_their_parent_with_unit_default_and_applicability() {
    let by_name = |name: &str| {
        catalog()
            .iter()
            .find(|spec| spec.name == name)
            .unwrap_or_else(|| panic!("no `{name}` in the catalog"))
    };
    for (parent, defaults) in [
        ("position", 0.0),
        ("scale", 1.0),
        ("start/position", f64::NAN),
        ("start/target", f64::NAN),
    ] {
        for (index, axis) in ["x", "y", "z"].into_iter().enumerate() {
            let spec = by_name(&format!("{parent}/{axis}"));
            let owner = by_name(parent);
            assert_eq!(spec.kind, ValueKind::Number);
            assert_eq!(spec.unit, owner.unit, "{}", spec.name);
            assert_eq!(spec.applies_to, owner.applies_to, "{}", spec.name);
            assert_eq!(spec.converts_first_movement, owner.converts_first_movement);
            assert_eq!(
                spec.component,
                Some(ComponentOf {
                    parent: owner.id,
                    index
                })
            );
            assert_eq!(
                spec.range, None,
                "components are unbounded, as their parent"
            );
            match (&spec.default, defaults.is_nan()) {
                (None, true) => {}
                (Some(PropertyValue::Number(value)), false) => assert_eq!(*value, defaults),
                other => panic!("{}: unexpected default {other:?}", spec.name),
            }
            // A node axis is what its parent is (editor-only); a camera-start axis is the parent's
            // write with one axis replaced, not the origin's path (BL004c).
            let expected = if spec.name.starts_with("start/") {
                OriginRelation::ParentWrite
            } else {
                owner.origin
            };
            assert_eq!(spec.origin, expected, "{}", spec.name);
        }
    }
    // A whole vector is never a component.
    assert!(by_name("position").component.is_none());
    // The catalog order of S5 is kept: components and renderOrder are appended.
    assert_eq!(catalog().len(), 21);
    let names: Vec<&str> = catalog().iter().map(|spec| spec.name).collect();
    // The properties of S5 keep their relative order: the 8 of S5 come first in that order; the rest is appended.
    let s5 = [
        "position",
        "rotation",
        "scale",
        "width",
        "height",
        "start/position",
        "start/target",
        "start/fov",
    ];
    let positions: Vec<usize> = s5
        .iter()
        .map(|name| names.iter().position(|found| found == name).expect(name))
        .collect();
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "{names:?}"
    );
}

#[test]
fn render_order_is_an_optional_integer_of_image_planes_only() {
    let spec = catalog()
        .iter()
        .find(|spec| spec.name == "renderOrder")
        .expect("renderOrder is catalogued");
    assert_eq!(spec.kind, ValueKind::Integer);
    assert_eq!(spec.unit, Unit::Unitless);
    assert!(spec.optional);
    assert_eq!(spec.default, Some(PropertyValue::Integer(0)));
    assert_eq!(spec.document_field, "renderOrder");
    assert_eq!(spec.id, PropertyId::Node(NodeProperty::RenderOrder));
    assert!(spec.component.is_none());
}

#[test]
fn component_paths_follow_the_origin_grammar_and_unknown_axes_are_refused() {
    let parsed = PropertyRef::parse("scene/street/node/hero-plane/position/x").expect("parses");
    assert_eq!(parsed, node("hero-plane", NodeProperty::PositionX));
    assert_eq!(path_of(&parsed), "scene/street/node/hero-plane/position/x");
    let camera = PropertyRef::parse("camera/wide/start/target/z").expect("parses");
    assert_eq!(camera, clip(CameraProperty::StartTargetZ));
    assert_eq!(
        PropertyRef::parse("scene/street/node/hero-plane/renderOrder").expect("parses"),
        node("hero-plane", NodeProperty::RenderOrder)
    );
    for refused in [
        "scene/street/node/hero-plane/position/w",
        "scene/street/node/hero-plane/rotation/x",
        "scene/street/node/hero-plane/position/x/y",
        "camera/wide/start/fov/x",
        "camera/wide/start/target/w",
    ] {
        assert_eq!(
            PropertyRef::parse(refused).expect_err(refused).code,
            "UNKNOWN_PATH",
            "{refused}"
        );
    }
}

#[test]
fn listing_hides_components_by_default_and_the_flag_adds_them_after_the_parents() {
    let scene = fixture();
    let plain = properties_of(&scene);
    let full = properties_with_components(&scene);
    assert!(plain.iter().all(|info| info.spec().component.is_none()));
    // The plain list is the S5 list plus renderOrder of each image plane (3).
    assert_eq!(plain.len(), 6 + 18 + 3);
    // Components: 6 per node (position, scale) and 6 for the camera start.
    assert_eq!(full.len(), plain.len() + 5 * 6 + 6);
    let paths: Vec<String> = full.iter().map(|info| info.property.path()).collect();
    for info in &plain {
        assert!(paths.contains(&info.property.path()));
    }
    let component = full
        .iter()
        .find(|info| info.property == node("hero-plane", NodeProperty::PositionY))
        .expect("hero position/y is listed");
    let Some(parent) = full
        .iter()
        .find(|info| info.property == node("hero-plane", NodeProperty::Position))
    else {
        panic!("the parent is listed");
    };
    let (PropertyValue::Vec3(whole), PropertyValue::Number(part)) =
        (&parent.value, &component.value)
    else {
        panic!("types");
    };
    assert_eq!(whole.y, *part);
    // Reading a component works with or without the listing.
    assert_eq!(
        read_property(&scene, &node("hero-plane", NodeProperty::PositionY)),
        Ok(number(whole.y))
    );
}

// ---- components ----------------------------------------------------------------------------

#[test]
fn setting_one_component_changes_only_that_axis_in_the_document() {
    let scene = fixture();
    let before = hero_json(&scene)["localPosition"].clone();
    let outcome = apply(
        &scene,
        &[set(
            node("hero-plane", NodeProperty::PositionX),
            number(4.5),
        )],
    )
    .expect("applies");
    let after = &hero_json(&outcome.candidate)["localPosition"];
    assert_eq!(after["x"], json!(4.5));
    assert_eq!(after["y"], before["y"]);
    assert_eq!(after["z"], before["z"]);
    assert_eq!(outcome.summary.changes.len(), 1);
    assert_eq!(outcome.summary.changes[0].after, number(4.5));
    assert!(outcome.summary.conversions.is_empty());
    // Nothing else moved.
    let mut expected = scene.document().clone();
    expected["sceneDefinitions"][0]["nodes"][3]["localPosition"]["x"] = json!(4.5);
    assert_eq!(outcome.candidate.document(), &expected);
}

#[test]
fn three_components_are_the_same_batch_as_the_whole_vector() {
    let scene = fixture();
    let by_axis = apply(
        &scene,
        &[
            set(node("hero-plane", NodeProperty::ScaleX), number(2.0)),
            set(node("hero-plane", NodeProperty::ScaleY), number(3.0)),
            set(node("hero-plane", NodeProperty::ScaleZ), number(4.0)),
        ],
    )
    .expect("axes apply");
    let whole = apply(
        &scene,
        &[set(
            node("hero-plane", NodeProperty::Scale),
            vec3(2.0, 3.0, 4.0),
        )],
    )
    .expect("vector applies");
    assert_eq!(by_axis.candidate.document(), whole.candidate.document());
}

#[test]
fn later_ops_see_earlier_components_and_the_parent_before_value_follows() {
    let scene = fixture();
    let outcome = apply(
        &scene,
        &[
            set(node("hero-plane", NodeProperty::PositionX), number(9.0)),
            set(
                node("hero-plane", NodeProperty::Position),
                vec3(1.0, 2.0, 3.0),
            ),
            set(node("hero-plane", NodeProperty::PositionX), number(7.0)),
        ],
    )
    .expect("applies");
    let changes = &outcome.summary.changes;
    assert_eq!(
        changes[1].before,
        vec3(9.0, changes_y(&scene), changes_z(&scene))
    );
    assert_eq!(changes[2].before, number(1.0));
    assert_eq!(
        xyz(&hero_json(&outcome.candidate)["localPosition"]),
        [7.0, 2.0, 3.0]
    );
}

fn changes_y(scene: &DirectedScene) -> f64 {
    hero_json(scene)["localPosition"]["y"].as_f64().expect("y")
}

fn changes_z(scene: &DirectedScene) -> f64 {
    hero_json(scene)["localPosition"]["z"].as_f64().expect("z")
}

#[test]
fn a_component_reset_writes_the_neutral_value_or_is_refused_without_one() {
    let scene = fixture();
    let outcome = apply(
        &scene,
        &[
            EditOp::reset(node("stage", NodeProperty::PositionY)),
            EditOp::reset(node("stage", NodeProperty::ScaleX)),
        ],
    )
    .expect("applies");
    let stage = &outcome.candidate.document()["sceneDefinitions"][0]["nodes"][1];
    assert_eq!(stage["localPosition"]["y"].as_f64(), Some(0.0));
    // `stage` scales 2 uniformly: only x went back to 1.
    assert_eq!(xyz(&stage["localScale"]), [1.0, 2.0, 2.0]);

    for property in [
        clip(CameraProperty::StartPositionX),
        clip(CameraProperty::StartTargetZ),
    ] {
        let error = failure(apply(&scene, &[EditOp::reset(property.clone())]));
        assert_eq!(error.code, NO_DEFAULT, "{property:?}");
        assert_eq!(error.index, Some(0));
        assert_eq!(error.path, Some(path_of(&property)));
    }
}

#[test]
fn writing_the_current_component_is_a_no_op_and_the_document_stays_identical() {
    let scene = fixture();
    let current = changes_y(&scene);
    let outcome = apply(
        &scene,
        &[
            set(node("hero-plane", NodeProperty::PositionY), number(current)),
            set(clip(CameraProperty::StartPositionZ), number(10.0)),
        ],
    )
    .expect("applies");
    assert!(!outcome.summary.modified);
    assert!(
        outcome
            .summary
            .changes
            .iter()
            .all(|change| change.is_noop())
    );
    assert!(outcome.summary.conversions.is_empty());
    assert_eq!(outcome.candidate.document(), scene.document());
}

#[test]
fn a_component_takes_a_number_only_and_a_failed_batch_leaves_no_candidate() {
    let scene = fixture();
    let refused = |op: EditOp, code: &str| {
        let error = failure(apply(
            &scene,
            &[
                set(node("hero-plane", NodeProperty::PositionX), number(5.0)),
                op.clone(),
            ],
        ));
        assert_eq!(error.code, code, "{op:?}: {error}");
        assert_eq!(error.index, Some(1));
        assert_eq!(error.path, Some(op.property().path()));
    };
    refused(
        set(
            node("hero-plane", NodeProperty::PositionX),
            vec3(1.0, 1.0, 1.0),
        ),
        "INVALID_TYPE",
    );
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        refused(
            set(node("hero-plane", NodeProperty::ScaleZ), number(value)),
            "INVALID_VALUE",
        );
    }
    // A scale of zero is a valid number for a component; the importer decides on the result.
    let degenerate = apply(
        &scene,
        &[set(node("hero-plane", NodeProperty::ScaleX), number(0.0))],
    );
    if let Err(error) = degenerate {
        assert_eq!(error.code, CANDIDATE_REJECTED);
    }
    // The input is never touched.
    assert_eq!(
        hero_json(&scene)["localPosition"]["x"],
        hero_json(&fixture())["localPosition"]["x"]
    );
}

#[test]
fn component_ops_decode_from_json_by_the_kind_the_catalog_declares() {
    let scene = fixture();
    let ops = tessel_editor_core::scene_edit::parse_batch(&json!([
        {"set": "scene/street/node/hero-plane/position/x", "value": 6.0},
        {"set": "camera/wide/start/target/y", "value": 1.25},
        {"reset": "scene/street/node/hero-plane/scale/z"},
    ]))
    .expect("decodes");
    assert!(dry_run(&scene, &ops).is_ok());
    let wrong = tessel_editor_core::scene_edit::parse_batch(&json!([
        {"set": "scene/street/node/hero-plane/position/x", "value": {"x": 1.0, "y": 0.0, "z": 0.0}},
    ]));
    let error = failure(wrong);
    assert_eq!(error.code, "INVALID_TYPE");
    assert_eq!(error.index, Some(0));
    assert_eq!(CommandError::from(error).code, "invalid_type");
}

// ---- renderOrder ---------------------------------------------------------------------------

#[test]
fn render_order_reads_the_stored_value_and_the_default_when_absent() {
    let scene = fixture();
    assert_eq!(
        read_property(&scene, &node("hero-plane", NodeProperty::RenderOrder)),
        Ok(PropertyValue::Integer(1))
    );
    // `prop-plane` leaves the field out.
    assert!(prop_json(&scene).get("renderOrder").is_none());
    assert_eq!(
        read_property(&scene, &node("prop-plane", NodeProperty::RenderOrder)),
        Ok(PropertyValue::Integer(0))
    );
    let group = failure(read_property(
        &scene,
        &node("stage", NodeProperty::RenderOrder),
    ));
    assert_eq!(group.code, NOT_APPLICABLE);
}

#[test]
fn render_order_writes_the_field_and_the_typed_node_follows() {
    let scene = fixture();
    let outcome = apply(
        &scene,
        &[
            set(
                node("prop-plane", NodeProperty::RenderOrder),
                PropertyValue::Integer(5),
            ),
            set(
                node("hero-plane", NodeProperty::RenderOrder),
                PropertyValue::Integer(-2),
            ),
        ],
    )
    .expect("applies");
    assert_eq!(prop_json(&outcome.candidate)["renderOrder"], json!(5));
    assert_eq!(hero_json(&outcome.candidate)["renderOrder"], json!(-2));
    let typed = |scene: &DirectedScene, id: &str| {
        scene
            .bundle()
            .scene("street")
            .and_then(|found| found.node(id))
            .and_then(|found| found.render_order)
    };
    assert_eq!(typed(&outcome.candidate, "prop-plane"), Some(5));
    assert_eq!(typed(&outcome.candidate, "hero-plane"), Some(-2));
    assert_eq!(outcome.summary.changes[0].before, PropertyValue::Integer(0));
}

#[test]
fn render_order_equal_to_the_default_over_an_absent_field_is_a_no_op_but_a_reset_over_a_value_writes()
 {
    let scene = fixture();
    let untouched = apply(
        &scene,
        &[
            set(
                node("prop-plane", NodeProperty::RenderOrder),
                PropertyValue::Integer(0),
            ),
            EditOp::reset(node("prop-plane", NodeProperty::RenderOrder)),
        ],
    )
    .expect("applies");
    assert!(!untouched.summary.modified);
    assert!(prop_json(&untouched.candidate).get("renderOrder").is_none());
    assert_eq!(untouched.candidate.document(), scene.document());

    let reset = apply(
        &scene,
        &[EditOp::reset(node("hero-plane", NodeProperty::RenderOrder))],
    )
    .expect("applies");
    assert!(reset.summary.modified);
    assert_eq!(hero_json(&reset.candidate)["renderOrder"], json!(0));
}

#[test]
fn render_order_refuses_non_integers_out_of_range_and_non_planes() {
    let scene = fixture();
    let refused = |value: Value, code: &str| {
        let ops = tessel_editor_core::scene_edit::parse_batch(&json!([
            {"set": "scene/street/node/hero-plane/renderOrder", "value": value}
        ]));
        let error = match ops {
            Err(error) => error,
            Ok(ops) => failure(apply(&scene, &ops)),
        };
        assert_eq!(error.code, code, "{value}: {error}");
    };
    refused(json!(1.5), "INVALID_TYPE");
    refused(json!("1"), "INVALID_TYPE");
    refused(json!(9_007_199_254_740_992_i64), "INVALID_VALUE");
    refused(json!(-9_007_199_254_740_992_i64), "INVALID_VALUE");
    // An integral float is an integer, as in the origin's JSON.
    let ok = tessel_editor_core::scene_edit::parse_batch(&json!([
        {"set": "scene/street/node/hero-plane/renderOrder", "value": 2.0}
    ]))
    .expect("2.0 is 2");
    assert_eq!(
        apply(&scene, &ok).expect("applies").summary.changes[0].after,
        PropertyValue::Integer(2)
    );
    let group = failure(apply(
        &scene,
        &[set(
            node("stage", NodeProperty::RenderOrder),
            PropertyValue::Integer(1),
        )],
    ));
    assert_eq!(group.code, NOT_APPLICABLE);
}

#[test]
fn a_composition_pass_keeps_overriding_the_edited_render_order() {
    let scene = fixture();
    let ids = |scene: &DirectedScene| -> Vec<String> {
        scene
            .bundle()
            .paint_order("street")
            .expect("scene")
            .iter()
            .map(|node| node.id.clone())
            .collect()
    };
    // `hero-first` pass: hero-plane (-1) < bg-plane (0) < prop-plane (absent -> 0).
    assert_eq!(ids(&scene), ["hero-plane", "bg-plane", "prop-plane"]);
    let outcome = apply(
        &scene,
        &[
            set(
                node("hero-plane", NodeProperty::RenderOrder),
                PropertyValue::Integer(50),
            ),
            set(
                node("bg-plane", NodeProperty::RenderOrder),
                PropertyValue::Integer(10),
            ),
        ],
    )
    .expect("applies");
    assert_eq!(
        ids(&outcome.candidate),
        ["hero-plane", "prop-plane", "bg-plane"]
    );
}

// ---- camera start: the origin's write ------------------------------------------------------

fn segment_recipes(scene: &DirectedScene) -> Vec<String> {
    clip_json(scene)["segments"]
        .as_array()
        .expect("segments")
        .iter()
        .map(|segment| segment["recipe"].as_str().expect("recipe").to_string())
        .collect()
}

#[test]
fn a_start_position_change_turns_the_first_movement_into_one_keyframe_ending_where_it_ended() {
    let scene = fixture();
    let end_before = scene.bundle().main_track_poses("wide").expect("poses")[0].end;
    let outcome = apply(
        &scene,
        &[set(
            clip(CameraProperty::StartPosition),
            vec3(1.0, 2.0, 10.0),
        )],
    )
    .expect("applies");

    let converted = &clip_json(&outcome.candidate)["segments"][0];
    assert_eq!(converted["recipe"], json!("keyframes"));
    assert_eq!(converted["durationFrames"], json!(24));
    assert_eq!(converted["from"], json!("initial"));
    let keys = converted["keys"].as_array().expect("keys");
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0]["frame"], json!(23));
    assert_eq!(keys[0]["easing"], json!("easeInOutCubic"));
    // The pose the movement ended on is where the key puts the camera.
    let end_after = outcome
        .candidate
        .bundle()
        .main_track_poses("wide")
        .expect("poses")[0]
        .end;
    assert_eq!(end_after, end_before);
    assert_eq!(
        xyz(&clip_json(&outcome.candidate)["initialPosition"]),
        [1.0, 2.0, 10.0]
    );
    // What follows is untouched, so the rest of the clip plays as before.
    assert_eq!(
        clip_json(&outcome.candidate)["segments"][1],
        clip_json(&scene)["segments"][1]
    );
    for frame in 24..48 {
        let before = scene
            .bundle()
            .camera_at_clip_frame("wide", frame)
            .expect("before");
        let after = outcome
            .candidate
            .bundle()
            .camera_at_clip_frame("wide", frame)
            .expect("after");
        assert_eq!(before.position, after.position, "frame {frame}");
        assert_eq!(before.target, after.target, "frame {frame}");
        assert_eq!(before.fov_deg, after.fov_deg, "frame {frame}");
    }
    // The summary says what was converted.
    assert_eq!(outcome.summary.conversions.len(), 1);
    let conversion = &outcome.summary.conversions[0];
    assert_eq!(conversion.clip, "wide");
    assert_eq!(conversion.index, 0);
    assert_eq!(conversion.segment, 0);
    assert_eq!(conversion.recipe, "dollyIn");
    assert_eq!(conversion.keys, 1);
}

#[test]
fn only_the_first_write_that_changes_the_start_converts_and_a_component_is_the_same_write() {
    let scene = fixture();
    let twice = apply(
        &scene,
        &[
            set(clip(CameraProperty::StartPositionX), number(1.0)),
            set(clip(CameraProperty::StartPositionY), number(3.0)),
        ],
    )
    .expect("applies");
    assert_eq!(twice.summary.conversions.len(), 1);
    assert_eq!(segment_recipes(&twice.candidate)[0], "keyframes");

    let whole = apply(
        &scene,
        &[set(
            clip(CameraProperty::StartPosition),
            vec3(1.0, 3.0, 10.0),
        )],
    )
    .expect("applies");
    assert_eq!(twice.candidate.document(), whole.candidate.document());
}

#[test]
fn target_and_fov_are_written_in_place_without_converting() {
    let scene = fixture();
    let outcome = apply(
        &scene,
        &[
            set(clip(CameraProperty::StartTarget), vec3(0.0, 1.5, 0.0)),
            set(clip(CameraProperty::StartTargetY), number(1.75)),
            set(clip(CameraProperty::StartFov), number(40.0)),
        ],
    )
    .expect("applies");
    assert!(outcome.summary.conversions.is_empty());
    assert_eq!(segment_recipes(&outcome.candidate), segment_recipes(&scene));
    assert_eq!(
        xyz(&clip_json(&outcome.candidate)["initialTarget"]),
        [0.0, 1.75, 0.0]
    );
}

#[test]
fn a_first_movement_that_already_is_keyframes_is_not_converted() {
    let mut doc = document();
    doc["cameraClips"][0]["segments"][0] = json!({
        "recipe": "keyframes", "durationFrames": 24, "from": "initial",
        "keys": [{
            "frame": 23, "position": {"x": 0, "y": 2, "z": 8},
            "target": {"x": 0, "y": 1, "z": 0}, "fovDeg": 50, "easing": "linear"
        }]
    });
    let scene = scene_of(doc);
    let outcome = apply(
        &scene,
        &[set(
            clip(CameraProperty::StartPosition),
            vec3(1.0, 2.0, 10.0),
        )],
    )
    .expect("applies");
    assert!(outcome.summary.conversions.is_empty());
    assert_eq!(
        clip_json(&outcome.candidate)["segments"][0],
        clip_json(&scene)["segments"][0]
    );
}

#[test]
fn dry_run_reports_the_conversion_apply_would_make_and_changes_nothing() {
    let scene = fixture();
    let before = scene.document().clone();
    let ops = [set(clip(CameraProperty::StartPositionZ), number(12.0))];
    let summary = dry_run(&scene, &ops).expect("dry run");
    let outcome = apply(&scene, &ops).expect("apply");
    assert_eq!(summary, outcome.summary);
    assert_eq!(summary.conversions.len(), 1);
    assert_eq!(scene.document(), &before);
}

#[test]
fn a_conversion_that_cannot_be_computed_is_refused_at_the_op_that_asked_for_it() {
    let scene = fixture();
    // Camera on its target: the first movement has no direction to convert.
    let error = failure(apply(
        &scene,
        &[
            set(clip(CameraProperty::StartTarget), vec3(0.0, 2.0, 10.0)),
            set(clip(CameraProperty::StartPositionX), number(1.0)),
        ],
    ));
    assert_eq!(error.code, CAMERA_CONVERSION_FAILED, "{error}");
    assert_eq!(error.index, Some(1));
    assert_eq!(error.path.as_deref(), Some("camera/wide/start/position/x"));
    assert_eq!(CommandError::from(error).code, "invalid_camera");
}

#[test]
fn a_one_frame_movement_cannot_be_converted_and_the_candidate_is_rejected_whole() {
    let mut doc = document();
    doc["cameraClips"][0]["segments"][0] =
        json!({"recipe": "hold", "durationFrames": 1, "from": "initial"});
    let scene = scene_of(doc);
    let error = failure(apply(
        &scene,
        &[
            set(clip(CameraProperty::StartFov), number(45.0)),
            set(clip(CameraProperty::StartPositionX), number(1.0)),
        ],
    ));
    // The origin refuses the same conversion (`CAMERA_SEGMENT_TOO_SHORT`, `CAMERA_KEY_FRAME_ORDER`).
    assert_eq!(error.code, CANDIDATE_REJECTED, "{error}");
    let codes: Vec<&str> = error
        .issues()
        .iter()
        .map(|found| found.issue.code)
        .collect();
    assert!(codes.contains(&"CAMERA_SEGMENT_TOO_SHORT"), "{codes:?}");
    assert!(codes.contains(&"CAMERA_KEY_FRAME_ORDER"), "{codes:?}");
    // Neither op left a trace in the input.
    assert_eq!(clip_json(&scene)["initialFovDeg"], json!(50));
}

#[test]
fn a_rejected_batch_rolls_back_the_conversion_with_everything_else() {
    let scene = fixture();
    let before = scene.document().clone();
    let error = failure(apply(
        &scene,
        &[
            set(clip(CameraProperty::StartPosition), vec3(1.0, 2.0, 10.0)),
            set(node("hero-plane", NodeProperty::Width), number(-1.0)),
        ],
    ));
    assert_eq!(error.code, "INVALID_VALUE");
    assert_eq!(error.index, Some(1));
    assert_eq!(scene.document(), &before);
}

#[test]
fn start_components_go_through_the_same_validation_as_the_whole_vector() {
    let scene = fixture();
    let error = failure(apply(
        &scene,
        &[set(clip(CameraProperty::StartPositionX), number(f64::NAN))],
    ));
    assert_eq!(error.code, "INVALID_VALUE");
    let error = failure(apply(
        &scene,
        &[set(
            node("hero-plane", NodeProperty::Width),
            PropertyValue::Integer(3),
        )],
    ));
    assert!(matches!(error.code, "INVALID_TYPE" | INVALID_OP), "{error}");
}

// ---- net effect: a component and its parent are one datum (BL004a, BL005b) -----------------

/// `position` of the hero plane in the fixture is `(3, 0, 0)`; `start/position` of `wide` is
/// `(0, 2, 10)`.
fn hero_x(scene: &DirectedScene) -> f64 {
    xyz(&hero_json(scene)["localPosition"])[0]
}

#[test]
fn an_axis_edit_that_survives_a_parent_restore_in_between_is_a_real_edit() {
    // position -> (2,0,0); position/x -> 5; position -> (3,0,0) (the original); position/x -> 2.
    // Judged per path the vector looks restored and `position/x` looks like 2 -> 2, so a real edit
    // used to be reported as a no-op and discarded.
    let scene = fixture();
    let outcome = apply(
        &scene,
        &[
            set(
                node("hero-plane", NodeProperty::Position),
                vec3(2.0, 0.0, 0.0),
            ),
            set(node("hero-plane", NodeProperty::PositionX), number(5.0)),
            set(
                node("hero-plane", NodeProperty::Position),
                vec3(3.0, 0.0, 0.0),
            ),
            set(node("hero-plane", NodeProperty::PositionX), number(2.0)),
        ],
    )
    .expect("applies");
    assert!(outcome.summary.modified, "{:?}", outcome.summary);
    assert_eq!(hero_x(&outcome.candidate), 2.0);
    assert_eq!(hero_x(&scene), 3.0);
}

#[test]
fn the_same_sequence_on_the_camera_start_is_a_real_edit_and_keeps_its_conversion() {
    let scene = fixture();
    let outcome = apply(
        &scene,
        &[
            set(clip(CameraProperty::StartPosition), vec3(2.0, 2.0, 10.0)),
            set(clip(CameraProperty::StartPositionX), number(5.0)),
            set(clip(CameraProperty::StartPosition), vec3(0.0, 2.0, 10.0)),
            set(clip(CameraProperty::StartPositionX), number(2.0)),
        ],
    )
    .expect("applies");
    assert!(outcome.summary.modified);
    assert_eq!(
        xyz(&clip_json(&outcome.candidate)["initialPosition"]),
        [2.0, 2.0, 10.0]
    );
    assert_eq!(segment_recipes(&outcome.candidate)[0], "keyframes");
    assert_eq!(outcome.summary.conversions.len(), 1);
}

#[test]
fn an_axis_restored_by_the_parent_vector_is_a_net_no_op_with_nothing_left_behind() {
    // position/x -> 9, then position -> the original vector: nothing changed. Judged per path the
    // two writes looked like a change, and the scene would have been replaced and dirtied.
    let scene = fixture();
    let outcome = apply(
        &scene,
        &[
            set(node("hero-plane", NodeProperty::PositionX), number(9.0)),
            set(
                node("hero-plane", NodeProperty::Position),
                vec3(3.0, 0.0, 0.0),
            ),
        ],
    )
    .expect("applies");
    assert!(!outcome.summary.modified);
    assert_eq!(outcome.summary.changed_count(), 2, "both ops did write");
    assert_eq!(outcome.candidate.document(), scene.document());

    // The same on the camera start, where the first write also converts the first movement.
    let outcome = apply(
        &scene,
        &[
            set(clip(CameraProperty::StartPositionX), number(9.0)),
            set(clip(CameraProperty::StartPosition), vec3(0.0, 2.0, 10.0)),
        ],
    )
    .expect("applies");
    assert!(!outcome.summary.modified);
    assert_eq!(outcome.candidate.document(), scene.document());
}

#[test]
fn a_start_set_and_restored_converts_nothing_for_the_caller() {
    // A -> B -> A: the first write converts inside the batch, but the net effect is nil, so the
    // answer is a no-op with the input scene back and no conversion listed. (The origin would keep
    // the keys its first write made; the editor treats a net no-op as nothing. Documented in
    // F034 and in the header of `scene_edit_oracle.rs`.)
    let scene = fixture();
    let ops = [
        set(clip(CameraProperty::StartPosition), vec3(1.0, 2.0, 10.0)),
        set(clip(CameraProperty::StartPosition), vec3(0.0, 2.0, 10.0)),
    ];
    let outcome = apply(&scene, &ops).expect("applies");
    assert!(!outcome.summary.modified);
    assert!(outcome.summary.conversions.is_empty());
    assert_eq!(outcome.candidate.document(), scene.document());
    assert_eq!(segment_recipes(&outcome.candidate)[0], "dollyIn");
    let summary = dry_run(&scene, &ops).expect("dry run");
    assert_eq!(summary, outcome.summary);

    // An orbit is the case where keys would not be the same movement (chords between keys): the
    // candidate must be the arc the input had, frame by frame.
    let mut doc = document();
    doc["cameraClips"][0]["segments"][0] = json!({
        "recipe": "orbit", "durationFrames": 24, "from": "initial",
        "yawDeg": 120, "easing": "linear"
    });
    let orbit = scene_of(doc);
    let outcome = apply(&orbit, &ops).expect("applies");
    assert!(!outcome.summary.modified);
    for frame in (1..24).step_by(2) {
        let before = orbit
            .bundle()
            .camera_at_clip_frame("wide", frame)
            .expect("before");
        let after = outcome
            .candidate
            .bundle()
            .camera_at_clip_frame("wide", frame)
            .expect("after");
        assert_eq!(before.position, after.position, "frame {frame}");
    }
}

#[test]
fn a_restored_start_with_another_real_edit_keeps_the_conversion_it_made() {
    // The batch is not a no-op (the field of view changed), so what the first write did stays, as
    // in the origin, and the conversion is reported because it is in the candidate.
    let scene = fixture();
    let outcome = apply(
        &scene,
        &[
            set(clip(CameraProperty::StartPosition), vec3(1.0, 2.0, 10.0)),
            set(clip(CameraProperty::StartPosition), vec3(0.0, 2.0, 10.0)),
            set(clip(CameraProperty::StartFov), number(40.0)),
        ],
    )
    .expect("applies");
    assert!(outcome.summary.modified);
    assert_eq!(outcome.summary.conversions.len(), 1);
    assert_eq!(segment_recipes(&outcome.candidate)[0], "keyframes");
}

#[test]
fn a_conversion_keeps_the_fields_the_schema_does_not_declare() {
    // The importer keeps and reports unknown fields; a conversion must not drop them silently.
    // (The origin drops them too, but its zod parse strips them before any command sees them.)
    let mut doc = document();
    doc["cameraClips"][0]["segments"][0]["note"] = json!("hand-written");
    doc["cameraClips"][0]["segments"][0]["extra"] = json!({"kept": true});
    let scene = scene_of(doc);
    let outcome = apply(
        &scene,
        &[set(
            clip(CameraProperty::StartPosition),
            vec3(1.0, 2.0, 10.0),
        )],
    )
    .expect("applies");
    let segment = &clip_json(&outcome.candidate)["segments"][0];
    assert_eq!(segment["recipe"], json!("keyframes"));
    assert_eq!(segment["note"], json!("hand-written"));
    assert_eq!(segment["extra"], json!({"kept": true}));
    // What the recipe owned is retired with it.
    assert!(segment.get("easing").is_none());
    assert!(segment.get("distanceSceneUnits").is_none());
    // Both are still reported as unknown fields by the importer.
    let notices: Vec<String> = outcome
        .candidate
        .notices()
        .iter()
        .map(|notice| notice.path.clone())
        .collect();
    assert!(
        notices.contains(&"/cameraClips/0/segments/0/note".to_string()),
        "{notices:?}"
    );
}

#[test]
fn render_order_beyond_what_a_number_holds_is_a_value_error_not_a_type_error() {
    let scene = fixture();
    let code_of = |value: Value| {
        let ops = tessel_editor_core::scene_edit::parse_batch(&json!([
            {"set": "scene/street/node/hero-plane/renderOrder", "value": value}
        ]));
        match ops {
            Err(error) => error.code,
            Ok(ops) => failure(apply(&scene, &ops)).code,
        }
    };
    // Whole, but outside +-(2^53 - 1): a float and integers above and below `i64`.
    for value in [
        json!(1e16),
        json!(-1e16),
        json!(1e300),
        json!(18_446_744_073_709_551_615_u64),
        json!(9_223_372_036_854_775_807_i64),
    ] {
        assert_eq!(code_of(value.clone()), "INVALID_VALUE", "{value}");
    }
    // Not whole, or not a number: still a type error.
    for value in [json!(0.5), json!(1e-3), json!("7"), json!(null), json!([1])] {
        assert_eq!(code_of(value.clone()), "INVALID_TYPE", "{value}");
    }
}
