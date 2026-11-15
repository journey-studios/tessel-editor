//! What a hostile or unusual input does to the shot ops: frames at the edge of `i64` (through the
//! JSON and through typed ops, which skip the parser), documents the importer accepts that write
//! `30.0` for a frame, and the fields of the document that no op knows.

use core::prelude::v1::test;

use serde_json::{Value, json};
use tessel_editor_core::scene_edit::INVALID_OP;
use tessel_editor_core::scene_edit::shots::{
    CODES, Edge, Placement, PlanInput, PlanInstance, PlanShot, ShotOp, ShotOptions, apply_shots,
    parse_shot_ops, plan_op,
};

use super::support::*;

/// The largest frame an op or a document may carry (the importer's `MAX_SAFE_INT`).
const BOUND: i64 = (1 << 53) - 1;

fn typed(name: &str, frame: i64) -> ShotOp {
    let options = ShotOptions::default();
    match name {
        "trim_shot" => ShotOp::TrimShot {
            shot: "b".into(),
            edge: Edge::Start,
            frame,
            options,
        },
        "move_cut" => ShotOp::MoveCut {
            from: "a".into(),
            to: "b".into(),
            frame,
            options,
        },
        "set_shot_duration" => ShotOp::SetShotDuration {
            shot: "b".into(),
            frames: frame,
            options,
        },
        _ => ShotOp::SetEditDuration {
            frames: frame,
            options,
        },
    }
}

fn json_of(name: &str, frame: i64) -> Value {
    match name {
        "trim_shot" => json!({"trim_shot": {"shot": "b", "edge": "end", "frame": frame}}),
        "move_cut" => json!({"move_cut": {"from": "a", "to": "b", "frame": frame}}),
        "set_shot_duration" => json!({"set_shot_duration": {"shot": "b", "frames": frame}}),
        _ => json!({"set_edit_duration": {"frames": frame}}),
    }
}

const OPS: [&str; 4] = [
    "trim_shot",
    "move_cut",
    "set_shot_duration",
    "set_edit_duration",
];

// ---- P1-1: the frame of an op never makes the planner panic or wrap around ---------------------

#[test]
fn a_frame_beyond_the_bound_is_invalid_op_through_the_json_for_the_four_ops() {
    for name in OPS {
        for frame in [i64::MIN, i64::MAX, BOUND + 1, -BOUND - 1] {
            let error = parse_shot_ops(&json!([json_of(name, frame)])).unwrap_err();
            assert_eq!(error.code, INVALID_OP, "{name} {frame}");
            assert_eq!(error.index, Some(0), "{name} {frame}");
        }
        // The bound itself is a frame like any other.
        parse_shot_ops(&json!([json_of(name, BOUND)])).unwrap();
        parse_shot_ops(&json!([json_of(name, -BOUND)])).unwrap();
    }
}

#[test]
fn a_frame_beyond_the_bound_in_a_typed_op_is_invalid_op_and_never_panics_or_wraps() {
    let scene = three(20);
    for name in OPS {
        for frame in [i64::MIN, i64::MAX, BOUND + 1, -BOUND - 1] {
            let error = apply_shots(&scene, &[typed(name, frame)]).unwrap_err();
            assert_eq!(error.code, INVALID_OP, "{name} {frame}: {error}");
            assert_eq!(error.index, Some(0), "{name} {frame}");
        }
    }
}

#[test]
fn a_frame_at_the_bound_is_refused_by_the_rules_of_the_domain_not_by_overflow() {
    let scene = three(20);
    for name in OPS {
        for frame in [BOUND, -BOUND] {
            match apply_shots(&scene, &[typed(name, frame)]) {
                Ok(_) => panic!("{name} {frame} was accepted"),
                Err(error) => {
                    assert_ne!(error.code, INVALID_OP, "{name} {frame}: {error}");
                    assert!(CODES.contains(&error.code), "{name} {frame}: {error}");
                }
            }
        }
    }
}

fn plan_shot(id: &str, start: i64, end: i64) -> PlanShot {
    PlanShot {
        id: id.into(),
        instance: format!("i{id}"),
        at: Placement {
            start,
            end,
            instance_start: 0,
            camera_in: 0,
        },
        continuous_in: false,
        reserve: 0,
    }
}

#[test]
fn the_planner_asks_the_bound_again_of_the_document_a_typed_input_describes() {
    let instances = || {
        ["ia", "ib"]
            .iter()
            .map(|id| PlanInstance {
                id: (*id).into(),
                frames: 100,
                locked: false,
            })
            .collect::<Vec<_>>()
    };
    let op = typed("set_shot_duration", 5);
    // A sane input plans.
    let sane = PlanInput::new(
        60,
        vec![plan_shot("a", 0, 30), plan_shot("b", 30, 60)],
        instances(),
    );
    assert!(plan_op(&sane, &op).is_ok());
    // A start, an end or the edit length beyond the bound is `INVALID_OP`, not an overflow.
    for input in [
        PlanInput::new(
            60,
            vec![
                plan_shot("a", 0, 30),
                plan_shot("b", i64::MAX - 1, i64::MAX),
            ],
            instances(),
        ),
        PlanInput::new(
            60,
            vec![plan_shot("a", i64::MIN, 30), plan_shot("b", 30, 60)],
            instances(),
        ),
        PlanInput::new(
            i64::MAX,
            vec![plan_shot("a", 0, 30), plan_shot("b", 30, 60)],
            instances(),
        ),
    ] {
        let refusal = plan_op(&input, &op).unwrap_err();
        assert_eq!(refusal.code, INVALID_OP, "{refusal:?}");
    }
}

// ---- P2-3: a document the importer accepts is a document the ops can edit ----------------------

/// The same document as `three(20)`, with every frame the importer reads leniently written as
/// `N.0`: `editDurationFrames`, the instances' `durationFrames` and, for each shot, its range,
/// `instanceStartFrame` and `cameraInFrame`.
fn floated() -> Value {
    let mut document = bundle(
        90,
        &[
            sh("a", "ia", 0, 30, 0, 0),
            sh("b", "ib", 30, 60, 20, 20),
            sh("c", "ic", 60, 90, 0, 0),
        ],
        &[("ia", 60, false), ("ib", 60, false), ("ic", 60, false)],
        json!({}),
    );
    fn float(object: &mut Value, key: &str) {
        let frames = object[key].as_i64().expect("an integer to start with");
        object[key] = json!(frames as f64);
    }
    float(&mut document, "editDurationFrames");
    for instance in document["sceneInstances"].as_array_mut().unwrap() {
        float(instance, "durationFrames");
    }
    for shot in document["shots"].as_array_mut().unwrap() {
        float(shot, "instanceStartFrame");
        float(shot, "cameraInFrame");
        float(&mut shot["editRange"], "startFrame");
        float(&mut shot["editRange"], "endFrameExclusive");
    }
    document
}

#[test]
fn a_document_with_frames_written_as_floats_is_edited_and_only_what_the_op_moves_is_rewritten() {
    let document = floated();
    // The fixture is what it claims: `30.0` is a float to serde, an integer to the importer.
    assert!(document["shots"][0]["editRange"]["endFrameExclusive"].is_f64());
    let scene = scene_of(document.clone());

    // `c` grows by 10: its end and the edit length change, nothing else does.
    let out = done(
        &scene,
        json!({"set_shot_duration": {"shot": "c", "frames": 40}}),
    );
    let mut expected = document.clone();
    expected["shots"][2]["editRange"]["endFrameExclusive"] = json!(100);
    expected["editDurationFrames"] = json!(100);
    assert_eq!(*out.candidate.document(), expected);
    // Byte for byte: no `0.0` became `0` and no `30.0` became `30` where the op did not write.
    assert_eq!(
        serde_json::to_string(out.candidate.document()).unwrap(),
        serde_json::to_string(&expected).unwrap()
    );

    // An op that changes nothing is `changed: false` and the document is what went in.
    let same = done(
        &scene,
        json!({"set_shot_duration": {"shot": "c", "frames": 30}}),
    );
    assert!(!same.changed);
    assert_eq!(*same.candidate.document(), document);
}

#[test]
fn a_roll_on_a_float_document_leaves_the_untouched_shot_as_it_was() {
    let document = floated();
    let scene = scene_of(document.clone());
    let out = done(
        &scene,
        json!({"move_cut": {"from": "a", "to": "b", "frame": 36}}),
    );
    // `a` ends and `b` starts at 36 and `b` loses 6 head frames; the other numbers keep the way
    // the document wrote them (`layout` would print `30.0`, so the fields are read one by one).
    let shots = out.candidate.document()["shots"].as_array().unwrap();
    assert_eq!(shots[0]["editRange"]["endFrameExclusive"], json!(36));
    assert_eq!(shots[1]["editRange"]["startFrame"], json!(36));
    assert_eq!(shots[1]["instanceStartFrame"], json!(26));
    assert_eq!(shots[1]["cameraInFrame"], json!(26));
    assert_eq!(shots[1]["editRange"]["endFrameExclusive"], json!(60.0));
    assert_eq!(shots[0]["editRange"]["startFrame"], json!(0.0));
    // `c` was not touched: its JSON is the one of the document, `.0` and all.
    assert_eq!(
        serde_json::to_string(&out.candidate.document()["shots"][2]).unwrap(),
        serde_json::to_string(&document["shots"][2]).unwrap()
    );
    assert_eq!(
        out.candidate.document()["editDurationFrames"],
        document["editDurationFrames"]
    );
}

// ---- unknown fields are kept -------------------------------------------------------------------

#[test]
fn fields_the_ops_do_not_know_survive_them() {
    let mut document = bundle(
        90,
        &[
            sh("a", "ia", 0, 30, 0, 0),
            sh("b", "ib", 30, 60, 20, 20),
            sh("c", "ic", 60, 90, 0, 0),
        ],
        &[("ia", 60, false), ("ib", 60, false), ("ic", 60, false)],
        json!({}),
    );
    document["x-root"] = json!({"keep": [1, 2, {"deep": true}]});
    document["shots"][1]["x-shot"] = json!("note on b");
    document["shots"][1]["editRange"]["x-range"] = json!({"label": "kept"});
    document["sceneInstances"][1]["x-instance"] = json!(7);
    let scene = scene_of(document);

    // The second op grows the instance `ib` (20 + 50 > 60): the object that gets written is the
    // one that carries the extra key.
    for (value, instance_frames) in [
        (
            json!({"set_shot_duration": {"shot": "b", "frames": 40}}),
            60,
        ),
        (
            json!({"set_shot_duration": {"shot": "b", "frames": 50}}),
            70,
        ),
    ] {
        let out = done(&scene, value.clone());
        let after = out.candidate.document();
        assert_eq!(
            after["x-root"],
            json!({"keep": [1, 2, {"deep": true}]}),
            "{value}"
        );
        assert_eq!(after["shots"][1]["x-shot"], json!("note on b"), "{value}");
        assert_eq!(
            after["shots"][1]["editRange"]["x-range"],
            json!({"label": "kept"}),
            "{value}"
        );
        assert_eq!(
            after["sceneInstances"][1]["x-instance"],
            json!(7),
            "{value}"
        );
        // And the op did its work on the same objects.
        assert_ne!(
            after["shots"][1]["editRange"],
            scene.document()["shots"][1]["editRange"]
        );
        assert_eq!(
            after["sceneInstances"][1]["durationFrames"],
            json!(instance_frames),
            "{value}"
        );
    }
}
