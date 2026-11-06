//! S6 (BL004/BL005): the start-pose write of a camera, against what the origin answers.
//!
//! `docs/migration/fixtures/scene-edit/expected.json` was produced by the REAL origin functions
//! (`setProperties`, `applyCameraCommands`, `applyPropertyChanges`, `resolveCameraAtClipFrame`),
//! bundled and run by `scripts/scene-edit-oracle/oracle.mjs`. This test replays the same synthetic
//! cases through `scene_edit::apply` and compares what the origin made: the clip after the edit
//! (ids, frames and easings exactly; numbers within `1e-9 + 1e-12·|x|`) and the composed camera at
//! every frame. It never reads a private bundle.
//!
//! Divergences by design, each proved below and not hidden:
//! - `noop-same-start`: the origin rewrites the movement as keyframes even when the value does not
//!   change; the editor leaves the document byte-identical (a no-op is a no-op). Poses agree.
//! - Errors: the origin reports `CAMERA_REFUSED`/`INVALID_COMMAND`; the editor reports its own
//!   typed code (`CANDIDATE_REJECTED` with the same importer findings, or `INVALID_VALUE`).

use serde_json::Value;
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::scene_edit::{EditOp, EditOutcome, apply};

const CASES: &str = include_str!("../../../docs/migration/fixtures/scene-edit/cases.json");
const EXPECTED: &str = include_str!("../../../docs/migration/fixtures/scene-edit/expected.json");
const CLIP: &str = "cam";

fn parse(text: &str) -> Vec<Value> {
    let document: Value = serde_json::from_str(text).expect("fixture is JSON");
    document["cases"].as_array().expect("cases").clone()
}

fn ops_of(case: &Value) -> Vec<EditOp> {
    case["assignments"]
        .as_array()
        .expect("assignments")
        .iter()
        .map(|assignment| {
            EditOp::from_json(&serde_json::json!({
                "set": assignment["path"], "value": assignment["value"]
            }))
            .unwrap_or_else(|error| panic!("{}: {error}", case["id"]))
        })
        .collect()
}

fn scene_of(case: &Value) -> DirectedScene {
    DirectedScene::import_value(case["bundle"].clone()).unwrap_or_else(|error| {
        panic!("{}: the origin accepts it, so must we: {error}", case["id"])
    })
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 + 1e-12 * a.abs().max(b.abs())
}

/// Equal trees: structure, strings, booleans and integers exactly; numbers within tolerance.
fn same(path: &str, actual: &Value, expected: &Value) -> Result<(), String> {
    match (actual, expected) {
        (Value::Number(a), Value::Number(b)) => {
            let (a, b) = (a.as_f64().expect("f64"), b.as_f64().expect("f64"));
            if close(a, b) {
                Ok(())
            } else {
                Err(format!("{path}: {a} != {b}"))
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                return Err(format!("{path}: length {} != {}", a.len(), b.len()));
            }
            a.iter()
                .zip(b)
                .enumerate()
                .try_for_each(|(index, (a, b))| same(&format!("{path}/{index}"), a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            let keys = |map: &serde_json::Map<String, Value>| {
                let mut keys: Vec<String> = map.keys().cloned().collect();
                keys.sort();
                keys
            };
            if keys(a) != keys(b) {
                return Err(format!("{path}: keys {:?} != {:?}", keys(a), keys(b)));
            }
            a.iter()
                .try_for_each(|(key, value)| same(&format!("{path}/{key}"), value, &b[key]))
        }
        (a, b) if a == b => Ok(()),
        (a, b) => Err(format!("{path}: {a} != {b}")),
    }
}

fn pose_json(scene: &DirectedScene, frame: i64) -> Value {
    let state = scene
        .bundle()
        .camera_at_clip_frame(CLIP, frame)
        .unwrap_or_else(|error| panic!("frame {frame}: {error}"));
    let point = |p: tessel_editor_core::directed_scene::Vec3| serde_json::json!({"x": p.x, "y": p.y, "z": p.z});
    serde_json::json!({
        "frame": frame,
        "position": point(state.position),
        "target": point(state.target),
        "fovDeg": state.fov_deg,
    })
}

fn poses_match(scene: &DirectedScene, expected: &Value, id: &str) -> Result<(), String> {
    for pose in expected["poses"].as_array().expect("poses") {
        let frame = pose["frame"].as_i64().expect("frame");
        same(
            &format!("{id}/frame {frame}"),
            &pose_json(scene, frame),
            pose,
        )?;
    }
    Ok(())
}

fn expected_of<'a>(expected: &'a [Value], id: &str) -> &'a Value {
    expected
        .iter()
        .find(|entry| entry["id"] == id)
        .unwrap_or_else(|| panic!("no expected entry for `{id}`"))
}

#[test]
fn the_cases_and_the_expected_answers_cover_each_other() {
    let cases = parse(CASES);
    let expected = parse(EXPECTED);
    let ids = |list: &[Value]| -> Vec<String> {
        list.iter()
            .map(|entry| entry["id"].as_str().expect("id").to_string())
            .collect()
    };
    assert_eq!(ids(&cases), ids(&expected));
    assert!(cases.len() >= 24, "the oracle shrank");
    // Every conversion rule of the origin is exercised: hold, dolly, truck, zoom, orbit (even,
    // odd, three and two frames) and an existing keyframes movement.
    for family in ["convert", "noop", "command", "batch", "error"] {
        assert!(
            cases.iter().any(|case| case["family"] == family),
            "{family}"
        );
    }
}

#[test]
fn every_accepted_case_makes_the_clip_and_the_frames_the_origin_makes() {
    let expected = parse(EXPECTED);
    let mut checked = 0;
    for case in parse(CASES) {
        let id = case["id"].as_str().expect("id").to_string();
        let answer = expected_of(&expected, &id);
        if answer["ok"] != true {
            continue;
        }
        let scene = scene_of(&case);
        let outcome: EditOutcome = apply(&scene, &ops_of(&case))
            .unwrap_or_else(|error| panic!("{id}: the origin accepts it, so must we: {error:?}"));
        let clip = outcome.candidate.document()["cameraClips"]
            .as_array()
            .expect("clips")
            .iter()
            .find(|clip| clip["id"] == CLIP)
            .expect("clip")
            .clone();
        if case["family"] == "noop" {
            // Documented divergence: the document is not rewritten for a value that did not change.
            assert!(!outcome.summary.modified, "{id}");
            assert!(outcome.summary.conversions.is_empty(), "{id}");
            assert_eq!(outcome.candidate.document(), scene.document(), "{id}");
        } else {
            same(&format!("{id}/clip"), &clip, &answer["clip"])
                .unwrap_or_else(|diff| panic!("{diff}"));
        }
        poses_match(&outcome.candidate, answer, &id).unwrap_or_else(|diff| panic!("{diff}"));
        checked += 1;
    }
    assert!(checked >= 20, "only {checked} accepted cases were compared");
}

#[test]
fn a_conversion_reports_what_it_converted_and_only_for_the_cases_that_convert() {
    let expected = parse(EXPECTED);
    let mut converting = 0;
    for case in parse(CASES) {
        let id = case["id"].as_str().expect("id");
        if expected_of(&expected, id)["ok"] != true {
            continue;
        }
        let scene = scene_of(&case);
        let outcome = apply(&scene, &ops_of(&case)).expect("accepted");
        let first_is_keyframes =
            scene.document()["cameraClips"][0]["segments"][0]["recipe"] == "keyframes";
        let moves_position = case["assignments"]
            .as_array()
            .expect("assignments")
            .iter()
            .any(|a| a["path"] == "camera/cam/start/position");
        let expected_count =
            usize::from(moves_position && !first_is_keyframes && case["family"] != "noop");
        assert_eq!(outcome.summary.conversions.len(), expected_count, "{id}");
        converting += expected_count;
    }
    assert!(
        converting >= 12,
        "only {converting} conversions were exercised"
    );
}

#[test]
fn every_refused_case_is_refused_by_the_editor_too_with_the_same_findings() {
    let expected = parse(EXPECTED);
    let mut refused = 0;
    for case in parse(CASES) {
        let id = case["id"].as_str().expect("id").to_string();
        let answer = expected_of(&expected, &id);
        if answer["ok"] != false {
            continue;
        }
        let scene = scene_of(&case);
        let error = match apply(&scene, &ops_of(&case)) {
            Err(error) => error,
            Ok(outcome) => panic!("{id}: the origin refuses it, got {:?}", outcome.summary),
        };
        // The input was not touched, so nothing of a batch that failed half way survives.
        assert_eq!(scene.document(), scene_of(&case).document(), "{id}");
        let origin = &answer["errors"][0];
        let codes: Vec<&str> = origin["cameraCodes"]
            .as_array()
            .expect("cameraCodes")
            .iter()
            .map(|code| code.as_str().expect("code"))
            .collect();
        for code in codes.iter().filter(|code| code.starts_with("CAMERA_")) {
            assert!(
                error.issues().iter().any(|found| found.issue.code == *code),
                "{id}: the origin reports {code}, the editor: {error:?}"
            );
        }
        if codes.contains(&"INVALID_COMMAND") {
            assert_eq!(error.code, "INVALID_VALUE", "{id}: {error:?}");
        } else {
            assert_eq!(error.code, "CANDIDATE_REJECTED", "{id}: {error:?}");
        }
        refused += 1;
    }
    assert!(refused >= 4, "only {refused} refusals were compared");
}

#[test]
fn the_comparison_catches_the_write_without_conversion() {
    // Negative control: writing only `initialPosition` (what the editor did before S6) moves the
    // rest of the path. If this ever matched the origin, the oracle would prove nothing.
    let expected = parse(EXPECTED);
    let mut caught = 0;
    for case in parse(CASES) {
        let id = case["id"].as_str().expect("id").to_string();
        if case["family"] != "convert" || expected_of(&expected, &id)["ok"] != true {
            continue;
        }
        let mut document = case["bundle"].clone();
        document["cameraClips"][0]["initialPosition"] = case["assignments"][0]["value"].clone();
        let Ok(naive) = DirectedScene::import_value(document) else {
            continue;
        };
        if poses_match(&naive, expected_of(&expected, &id), &id).is_err() {
            caught += 1;
        }
    }
    assert!(caught >= 8, "the control caught only {caught} cases");
}
