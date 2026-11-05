//! BL018 / K (F032, round 2): the extended oracle corpus of `docs/migration/fixtures/extended-round2`
//! against the public library API only (no GPUI, no window, no runtime).
//!
//! The expectations come from the origin's own functions at the pinned snapshot (`d6adac15…`),
//! executed by `scripts/migration-oracle/extended.mjs`; nothing here was adapted to what the Rust
//! returns. Provenance and regeneration: `docs/migration/fixtures/extended-round2/manifest.json`,
//! contract and limits: `docs/features/migration-oracle.md`.
//!
//! # What this file compares
//!
//! | Family | Question |
//! |---|---|
//! | `join-boundary` | Does the Rust gate accept/refuse a continuous join exactly where the origin does (below, equal, above each `JOIN_*` limit)? Are the poses on both sides the origin's? |
//! | `fps` | `Rate`/`Ratio` against `fpsToRate`, `editFrameToSeconds`, `secondsToEditFrame`, `editSecondsToFrame` at 24000/1001, 30000/1001, 60000/1001, 25, 30, 60 and an unreduced 48/2 |
//! | `out-of-order` | Seeking forward, backward, shuffled and repeated gives the same answer (origin and Rust) |
//! | `world-matrix` | Node world matrices (gimbal threshold, extreme scales, chained sockets, channels) and the camera `to_world_matrix` |
//!
//! # Numeric tolerances (fixed before the Rust output was compared; see `tolerances.json`)
//!
//! | Quantity | Rule |
//! |---|---|
//! | ids, frame numbers, counts, `ok`, accepted, join codes | exact |
//! | node world matrices | `abs <= 1e-10 + 1e-12 * abs(expected)` |
//! | node local position / Euler degrees / scale, channel overrides | `abs <= 1e-9 + 1e-12 * abs(expected)` |
//! | camera position, target, fov, velocities | `abs <= 1e-9 + 1e-12 * abs(expected)` |
//! | camera world matrix (Three.js `lookAt`) | `abs <= 1e-9 + 1e-12 * abs(expected)` |
//! | seconds (`frame_to_time`, durations) | `abs <= 1e-12 + 1e-12 * abs(expected)` |
//!
//! `declared_tolerances_are_the_ones_fixed_before_the_comparison` fails if this table and
//! `tolerances.json` ever disagree, so a tolerance cannot be loosened in one place only.
//!
//! # Where Rust and the origin are allowed to differ, and why
//!
//! - `fps-above-1000`: the origin accepts any positive rational rate; this importer documents a
//!   1000 fps ceiling (`FPS_OUT_OF_RANGE`). Asserted explicitly, not skipped silently.
//! - Seconds → frame at exact frame boundaries: the origin computes `Math.floor(seconds * rate)` in
//!   floating point, which is one frame short at some NTSC boundaries (14 probes in the corpus). The
//!   fixture marks them `originMatchesExact = false`; Rust (exact rationals) must equal the exact
//!   floor there and the origin everywhere else. The count is pinned, so a change forces a re-triage.
//!   The decision (keep exact, or reproduce the origin's float behaviour) is BL-proposed in the K report.
//! - The camera world matrix of the origin is the Three.js library's `lookAt` sequence of the
//!   runtime (the runtime method itself is private and needs WebGL): recorded as
//!   `source: "three-library"`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::{
    CameraPose, CameraState, DirectedScene, JOIN_FOV_TOLERANCE_DEG,
    JOIN_FOV_VELOCITY_TOLERANCE_DEG_PER_FRAME, JOIN_POSITION_TOLERANCE_SCENE_UNITS,
    JOIN_VELOCITY_TOLERANCE_SCENE_UNITS_PER_FRAME, Vec3,
};
use tessel_editor_core::time::{Rate, Ratio};

#[derive(Clone, Copy)]
struct Tolerance {
    absolute: f64,
    relative: f64,
}

const WORLD_MATRIX: Tolerance = Tolerance {
    absolute: 1e-10,
    relative: 1e-12,
};
const NODE_LOCAL: Tolerance = Tolerance {
    absolute: 1e-9,
    relative: 1e-12,
};
const CAMERA: Tolerance = Tolerance {
    absolute: 1e-9,
    relative: 1e-12,
};
const CAMERA_WORLD_MATRIX: Tolerance = Tolerance {
    absolute: 1e-9,
    relative: 1e-12,
};
const TIME_SECONDS: Tolerance = Tolerance {
    absolute: 1e-12,
    relative: 1e-12,
};

/// Review the coverage when the corpus changes (the number is part of the manifest summary).
const EXPECTED_CASES: usize = 47;
/// Probes where the origin's floating-point `secondsToEditFrame` is not the exact floor.
const EXPECTED_ORIGIN_FLOAT_ARTIFACTS: usize = 14;
/// Cases where this importer documents a difference from the origin, with the code it must give.
const DOCUMENTED_RUST_LIMITS: [(&str, &str); 1] = [("fps-above-1000", "FPS_OUT_OF_RANGE")];

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/migration/fixtures/extended-round2")
}

fn read_json(path: impl AsRef<Path>) -> Value {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{} is not JSON: {error}", path.display()))
}

struct Case {
    id: String,
    raw: Value,
    expected: Value,
}

fn cases() -> Vec<Case> {
    let root = fixtures();
    let index = read_json(root.join("cases.json"));
    index["cases"]
        .as_array()
        .expect("cases.json has a `cases` array")
        .iter()
        .map(|entry| Case {
            id: entry["id"].as_str().expect("case id").to_string(),
            raw: read_json(root.join(entry["bundle"].as_str().expect("bundle path"))),
            expected: read_json(root.join(entry["expected"].as_str().expect("expected path"))),
        })
        .collect()
}

fn documented_limit(id: &str) -> Option<&'static str> {
    DOCUMENTED_RUST_LIMITS
        .iter()
        .find(|(case, _)| *case == id)
        .map(|(_, code)| *code)
}

fn manifest_summary() -> Value {
    read_json(fixtures().join("manifest.json"))["summary"].clone()
}

// ---- comparison machinery ----------------------------------------------------------------
//
// Every comparison pushes a readable divergence (label, both values, distance, tolerance) into a
// `Report` instead of panicking at the first one, so a run shows the whole picture. The negative
// controls call the very same functions with a perturbed expectation.

#[derive(Default)]
struct Report {
    failures: Vec<String>,
    compared: usize,
    worst: BTreeMap<&'static str, (f64, String)>,
}

impl Report {
    fn fail(&mut self, message: String) {
        self.failures.push(message);
    }

    fn number(&mut self, label: &str, value: &Value) -> Option<f64> {
        let number = value.as_f64();
        if number.is_none() {
            self.fail(format!("{label}: expected a number, got {value}"));
        }
        number
    }

    fn close(
        &mut self,
        kind: &'static str,
        label: &str,
        actual: f64,
        expected: f64,
        tolerance: Tolerance,
    ) {
        let distance = (actual - expected).abs();
        let limit = tolerance.absolute + tolerance.relative * expected.abs();
        self.compared += 1;
        // A NaN distance (either side not finite) must fail too, hence the explicit check.
        if distance.is_nan() || distance > limit {
            self.fail(format!(
                "{label}: Rust {actual} != origin {expected} (distance {distance:e}, tolerance {limit:e})"
            ));
            return;
        }
        let entry = self.worst.entry(kind).or_insert((0.0, String::new()));
        if distance > entry.0 {
            *entry = (distance, label.to_string());
        }
    }

    fn close_json(
        &mut self,
        kind: &'static str,
        label: &str,
        actual: &Value,
        expected: &Value,
        tolerance: Tolerance,
    ) {
        let actual = self.number(&format!("{label} (Rust)"), actual);
        let expected = self.number(&format!("{label} (origin)"), expected);
        if let (Some(actual), Some(expected)) = (actual, expected) {
            self.close(kind, label, actual, expected, tolerance);
        }
    }

    fn vec3(
        &mut self,
        kind: &'static str,
        label: &str,
        actual: &Value,
        expected: &Value,
        tolerance: Tolerance,
    ) {
        for axis in ["x", "y", "z"] {
            self.close_json(
                kind,
                &format!("{label}.{axis}"),
                &actual[axis],
                &expected[axis],
                tolerance,
            );
        }
    }

    fn matrix(
        &mut self,
        kind: &'static str,
        label: &str,
        actual: &Value,
        expected: &Value,
        tolerance: Tolerance,
    ) {
        let (Some(actual), Some(expected)) = (actual.as_array(), expected.as_array()) else {
            self.fail(format!("{label}: a matrix is not an array"));
            return;
        };
        if actual.len() != 16 || expected.len() != 16 {
            self.fail(format!(
                "{label}: matrix lengths Rust {} / origin {}, expected 16",
                actual.len(),
                expected.len()
            ));
            return;
        }
        for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            self.close_json(
                kind,
                &format!("{label}[{index}]"),
                actual,
                expected,
                tolerance,
            );
        }
    }

    fn same(&mut self, label: &str, actual: &Value, expected: &Value) {
        self.compared += 1;
        if actual != expected {
            self.fail(format!("{label}: Rust {actual} != origin {expected}"));
        }
    }

    fn finish(self, name: &str) {
        for (kind, (worst, at)) in &self.worst {
            eprintln!("{name}: worst {kind} distance {worst:e} at {at}");
        }
        eprintln!("{name}: {} values compared", self.compared);
        assert!(
            self.failures.is_empty(),
            "{name}: {} divergence(s) from the origin:\n{}",
            self.failures.len(),
            self.failures
                .iter()
                .take(40)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

fn camera_json(camera: &CameraState) -> Value {
    json!({
        "position": camera.position.to_json(),
        "target": camera.target.to_json(),
        "fovDeg": camera.fov_deg,
        "velocity": camera.velocity.to_json(),
        "fovVelocityDegPerFrame": camera.fov_velocity_deg_per_frame,
    })
}

fn compare_camera(report: &mut Report, label: &str, actual: &Value, expected: &Value) {
    for key in ["position", "target", "velocity"] {
        report.vec3(
            "camera",
            &format!("{label}.{key}"),
            &actual[key],
            &expected[key],
            CAMERA,
        );
    }
    for key in ["fovDeg", "fovVelocityDegPerFrame"] {
        report.close_json(
            "camera",
            &format!("{label}.{key}"),
            &actual[key],
            &expected[key],
            CAMERA,
        );
    }
}

/// One evaluated frame (`FrameEvaluation::to_json`) against the origin's `{ state, worldMatrices }`.
fn compare_frame(report: &mut Report, label: &str, actual: &Value, expected_frame: &Value) {
    let state = &expected_frame["state"];
    for key in [
        "instanceId",
        "instanceFrame",
        "cameraClipId",
        "cameraClipFrame",
    ] {
        report.same(&format!("{label}.{key}"), &actual[key], &state[key]);
    }
    compare_camera(
        report,
        &format!("{label}.camera"),
        &actual["camera"],
        &state["camera"],
    );

    let (Some(nodes), Some(expected_nodes)) =
        (actual["nodes"].as_array(), state["nodes"].as_array())
    else {
        report.fail(format!("{label}: `nodes` is not an array"));
        return;
    };
    if nodes.len() != expected_nodes.len() {
        report.fail(format!(
            "{label}: Rust has {} nodes, the origin {}",
            nodes.len(),
            expected_nodes.len()
        ));
        return;
    }
    for (node, expected) in nodes.iter().zip(expected_nodes) {
        let id = expected["nodeId"].as_str().unwrap_or("?");
        let at = format!("{label} {id}");
        report.same(
            &format!("{at}.nodeId"),
            &node["nodeId"],
            &expected["nodeId"],
        );
        report.same(
            &format!("{at}.renderOrder"),
            &node["renderOrder"],
            &expected["renderOrder"],
        );
        for key in ["localPosition", "localRotation", "localScale"] {
            report.vec3(
                "node local",
                &format!("{at}.{key}"),
                &node[key],
                &expected[key],
                NODE_LOCAL,
            );
        }
        report.matrix(
            "world matrix",
            &format!("{at}.worldMatrix"),
            &node["worldMatrix"],
            &expected_frame["worldMatrices"][id],
            WORLD_MATRIX,
        );
    }

    let (Some(overrides), Some(expected_overrides)) = (
        actual["performanceOverrides"].as_array(),
        state["performanceOverrides"].as_array(),
    ) else {
        report.fail(format!("{label}: `performanceOverrides` is not an array"));
        return;
    };
    if overrides.len() != expected_overrides.len() {
        report.fail(format!(
            "{label}: Rust has {} overrides, the origin {}",
            overrides.len(),
            expected_overrides.len()
        ));
        return;
    }
    for (index, (entry, expected)) in overrides.iter().zip(expected_overrides).enumerate() {
        let at = format!("{label} override {index}");
        report.same(
            &format!("{at}.nodeId"),
            &entry["nodeId"],
            &expected["nodeId"],
        );
        report.same(
            &format!("{at}.property"),
            &entry["property"],
            &expected["property"],
        );
        report.vec3(
            "node local",
            &format!("{at}.value"),
            &entry["value"],
            &expected["value"],
            NODE_LOCAL,
        );
    }
}

type Evaluated = Result<Value, String>;

fn evaluate(scene: &DirectedScene, edit_frame: i64) -> Evaluated {
    scene
        .bundle()
        .evaluate_frame(edit_frame)
        .map(|frame| frame.to_json())
        .map_err(|error| error.to_string())
}

fn compare_evaluated(report: &mut Report, label: &str, actual: &Evaluated, expected_frame: &Value) {
    if expected_frame["ok"] == false {
        match actual {
            Ok(_) => report.fail(format!("{label}: the origin fails, Rust succeeds")),
            Err(text) => report.same(label, &json!(text), &expected_frame["error"]),
        }
        return;
    }
    match actual {
        Err(text) => report.fail(format!(
            "{label}: the origin evaluates, Rust fails with `{text}`"
        )),
        Ok(frame) => compare_frame(report, label, frame, expected_frame),
    }
}

fn import(case: &Case) -> Result<DirectedScene, String> {
    DirectedScene::import_value(case.raw.clone())
        .map_err(|error| format!("{}: Rust import refuses it: {error:?}", case.id))
}

// ---- tolerances and corpus shape ---------------------------------------------------------

#[test]
fn declared_tolerances_are_the_ones_fixed_before_the_comparison() {
    let declared = read_json(fixtures().join("tolerances.json"));
    for (name, tolerance) in [
        ("worldMatrix", WORLD_MATRIX),
        ("nodeLocal", NODE_LOCAL),
        ("camera", CAMERA),
        ("cameraWorldMatrix", CAMERA_WORLD_MATRIX),
        ("timeSeconds", TIME_SECONDS),
    ] {
        assert_eq!(
            declared["numeric"][name]["absolute"].as_f64(),
            Some(tolerance.absolute),
            "{name}.absolute differs from tolerances.json"
        );
        assert_eq!(
            declared["numeric"][name]["relative"].as_f64(),
            Some(tolerance.relative),
            "{name}.relative differs from tolerances.json"
        );
    }
    // The JOIN_* limits are the origin's domain rules (not comparison tolerances): the Rust gate
    // must carry the very same numbers the real module exports.
    for (name, value) in [
        (
            "JOIN_POSITION_TOLERANCE_SCENE_UNITS",
            JOIN_POSITION_TOLERANCE_SCENE_UNITS,
        ),
        ("JOIN_FOV_TOLERANCE_DEG", JOIN_FOV_TOLERANCE_DEG),
        (
            "JOIN_VELOCITY_TOLERANCE_SCENE_UNITS_PER_FRAME",
            JOIN_VELOCITY_TOLERANCE_SCENE_UNITS_PER_FRAME,
        ),
        (
            "JOIN_FOV_VELOCITY_TOLERANCE_DEG_PER_FRAME",
            JOIN_FOV_VELOCITY_TOLERANCE_DEG_PER_FRAME,
        ),
    ] {
        assert_eq!(
            declared["joinRules"][name].as_f64(),
            Some(value),
            "{name} differs from the origin's constant"
        );
    }
}

#[test]
fn corpus_has_the_declared_shape() {
    let all = cases();
    assert_eq!(
        all.len(),
        EXPECTED_CASES,
        "review coverage when the corpus changes"
    );
    assert_eq!(
        manifest_summary()["cases"].as_u64(),
        Some(EXPECTED_CASES as u64)
    );
    for case in &all {
        assert_eq!(case.expected["case"], json!(case.id), "{}", case.id);
        assert_eq!(case.expected["formatVersion"], json!(1), "{}", case.id);
    }
    for family in ["join-boundary", "fps", "out-of-order", "world-matrix"] {
        assert!(
            all.iter().any(|case| case.expected["family"] == family),
            "no case of family {family}"
        );
    }
}

// ---- verdicts and join gates -------------------------------------------------------------

fn shot_id(raw: &Value, path: &str) -> String {
    // The issue path is a JSON Pointer into the document: `/shots/<index>` is the array index of
    // the later shot of the join, which is the shot the origin reports.
    path.strip_prefix("/shots/")
        .and_then(|rest| rest.split('/').next())
        .and_then(|index| index.parse::<usize>().ok())
        .and_then(|index| raw["shots"][index]["id"].as_str().map(str::to_string))
        .unwrap_or_default()
}

/// A segment reset names a clip, not a shot, in the origin: compare its code only.
fn blank_reset_shot(code: String, shot: String) -> (String, String) {
    if code == "CAMERA_SEGMENT_RESET" {
        return (code, String::new());
    }
    (code, shot)
}

fn distance(a: &Value, b: &Value) -> f64 {
    let axis =
        |key: &str| a[key].as_f64().unwrap_or(f64::NAN) - b[key].as_f64().unwrap_or(f64::NAN);
    axis("x").hypot(axis("y")).hypot(axis("z"))
}

/// The origin's own poses on both sides of each join, measured against the `JOIN_*` limits, for
/// messages: this is what makes a boundary divergence readable.
fn join_margins(expected: &Value) -> String {
    let Some(joins) = expected["joins"].as_array() else {
        return String::new();
    };
    joins
        .iter()
        .filter_map(|join| {
            let (end, start) = (&join["end"]["camera"], &join["start"]["camera"]);
            if end.is_null() || start.is_null() {
                return None;
            }
            let fov = |camera: &Value, key: &str| camera[key].as_f64().unwrap_or(f64::NAN);
            Some(format!(
                "join {}|{}: position {:.17} (limit {}), target {:.17} (limit {}), fov {:.17} (limit {}), velocity {:.17} (limit {}), fov velocity {:.17} (limit {})",
                join["previousShotId"].as_str().unwrap_or("?"),
                join["nextShotId"].as_str().unwrap_or("?"),
                distance(&end["position"], &start["position"]),
                JOIN_POSITION_TOLERANCE_SCENE_UNITS,
                distance(&end["target"], &start["target"]),
                JOIN_POSITION_TOLERANCE_SCENE_UNITS,
                (fov(end, "fovDeg") - fov(start, "fovDeg")).abs(),
                JOIN_FOV_TOLERANCE_DEG,
                distance(&end["velocity"], &start["velocity"]),
                JOIN_VELOCITY_TOLERANCE_SCENE_UNITS_PER_FRAME,
                (fov(end, "fovVelocityDegPerFrame") - fov(start, "fovVelocityDegPerFrame")).abs(),
                JOIN_FOV_VELOCITY_TOLERANCE_DEG_PER_FRAME,
            ))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Verdict of the origin for one case against the Rust import plus both join gates.
fn compare_verdict(report: &mut Report, id: &str, raw: &Value, expected: &Value) {
    let validation = &expected["validation"];
    let origin_accepted = validation["accepted"].as_bool().unwrap_or(false);
    report.compared += 1;
    match DirectedScene::import_value(raw.clone()) {
        Err(error) => {
            if origin_accepted {
                report.fail(format!(
                    "{id}: the origin accepts, Rust import refuses: {error:?}"
                ));
                return;
            }
            let rust_codes: Vec<&str> = error.issues().iter().map(|issue| issue.code).collect();
            let origin_issues = validation["schema"]["issues"]
                .as_array()
                .map_or(&[][..], Vec::as_slice);
            // The code is compared only when the origin refuses by a named rule alone. A structural
            // zod issue (e.g. `fps.denominator` too small) also makes the origin's FPS_MISMATCH
            // refinement fire on the unreadable fps, an accident of zod that is not a contract:
            // there only the verdict is compared.
            let rule_only = origin_issues
                .iter()
                .all(|issue| issue["sourceCode"].is_string());
            for issue in origin_issues.iter().filter(|_| rule_only) {
                let Some(code) = issue["sourceCode"].as_str() else {
                    continue;
                };
                if !rust_codes.contains(&code) {
                    report.fail(format!(
                        "{id}: the origin refuses with {code}, Rust import refuses with {rust_codes:?}"
                    ));
                }
            }
        }
        Ok(scene) => {
            let mut rust: Vec<(String, String)> = scene
                .edit_join_issues()
                .iter()
                .chain(scene.temporal_issues().iter())
                .map(|issue| blank_reset_shot(issue.code.to_string(), shot_id(raw, &issue.path)))
                .collect();
            let mut origin: Vec<(String, String)> = validation["editJoins"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|join| {
                    blank_reset_shot(
                        join["code"].as_str().unwrap_or("?").to_string(),
                        join["shotId"].as_str().unwrap_or("").to_string(),
                    )
                })
                .collect();
            rust.sort();
            origin.sort();
            if rust != origin {
                report.fail(format!(
                    "{id}: join findings Rust {rust:?} != origin {origin:?}; {}",
                    join_margins(expected)
                ));
            }
            if rust.is_empty() != origin_accepted {
                report.fail(format!(
                    "{id}: verdict Rust accepted={} != origin accepted={origin_accepted}; {}",
                    rust.is_empty(),
                    join_margins(expected)
                ));
            }
        }
    }
}

#[test]
fn verdicts_and_join_findings_match_the_origin_on_every_case() {
    let mut report = Report::default();
    let mut boundaries = 0;
    for case in cases() {
        if let Some(code) = documented_limit(&case.id) {
            // Origin accepts, importer documents a limit: the divergence is asserted, not skipped.
            assert_eq!(case.expected["validation"]["accepted"], true, "{}", case.id);
            let error = DirectedScene::import_value(case.raw.clone())
                .expect_err("a documented limit must refuse the import");
            assert!(
                error.issues().iter().any(|issue| issue.code == code),
                "{}: expected {code}, got {error:?}",
                case.id
            );
            continue;
        }
        if case.expected["kind"] == "boundary" {
            boundaries += 1;
            eprintln!(
                "boundary {}: origin accepted={} {}",
                case.id,
                case.expected["validation"]["accepted"],
                join_margins(&case.expected)
            );
        }
        compare_verdict(&mut report, &case.id, &case.raw, &case.expected);
    }
    assert_eq!(boundaries, 6, "origin-decided boundary cases changed");
    report.finish("verdicts");
}

#[test]
fn camera_poses_on_both_sides_of_every_continuous_join_match_the_origin() {
    let mut report = Report::default();
    let mut joins = 0;
    for case in cases() {
        let Some(samples) = case.expected["joins"].as_array() else {
            continue;
        };
        let scene = match import(&case) {
            Ok(scene) => scene,
            Err(message) => {
                report.fail(message);
                continue;
            }
        };
        for sample in samples {
            let clip = sample["cameraClipId"].as_str().expect("clip id");
            for side in ["end", "start"] {
                let label = format!(
                    "{} {}|{} {side}",
                    case.id,
                    sample["previousShotId"].as_str().unwrap_or("?"),
                    sample["nextShotId"].as_str().unwrap_or("?")
                );
                let origin = &sample[side];
                let frame = origin["clipFrame"].as_i64().expect("clip frame");
                match scene.bundle().camera_at_clip_frame(clip, frame) {
                    Ok(camera) => {
                        compare_camera(
                            &mut report,
                            &label,
                            &camera_json(&camera),
                            &origin["camera"],
                        );
                    }
                    Err(error) => report.fail(format!("{label}: Rust fails with `{error}`")),
                }
            }
            joins += 1;
        }
    }
    assert_eq!(
        joins as u64,
        manifest_summary()["joinSamples"].as_u64().unwrap(),
        "join coverage differs from the manifest"
    );
    report.finish("join poses");
}

// ---- frames, order independence, matrices ------------------------------------------------

#[test]
fn edit_frames_and_world_matrices_match_the_origin_on_every_probed_frame() {
    let mut report = Report::default();
    let mut visited = 0_usize;
    for case in cases() {
        let Some(frames) = case.expected["evaluation"]["editFrames"].as_array() else {
            continue;
        };
        visited += frames.len();
        if documented_limit(&case.id).is_some() {
            continue;
        }
        let scene = match import(&case) {
            Ok(scene) => scene,
            Err(message) => {
                report.fail(message);
                continue;
            }
        };
        for frame in frames {
            let edit = frame["editFrame"].as_i64().expect("edit frame");
            compare_evaluated(
                &mut report,
                &format!("{} edit {edit}", case.id),
                &evaluate(&scene, edit),
                frame,
            );
        }
    }
    assert_eq!(
        visited as u64,
        manifest_summary()["editFramesEvaluated"].as_u64().unwrap(),
        "frame coverage differs from the manifest"
    );
    report.finish("edit frames");
}

#[test]
fn seeking_in_any_order_gives_the_origin_answer_and_the_same_answer_every_time() {
    let mut report = Report::default();
    let mut orders_run = 0;
    for case in cases() {
        let orders = &case.expected["orders"];
        if orders.is_null() {
            continue;
        }
        assert_eq!(
            orders["identicalToCanonical"], true,
            "{}: the origin itself depends on the order",
            case.id
        );
        let canonical: BTreeMap<i64, &Value> = case.expected["evaluation"]["editFrames"]
            .as_array()
            .expect("evaluation frames")
            .iter()
            .map(|frame| (frame["editFrame"].as_i64().expect("edit frame"), frame))
            .collect();
        // The first answer Rust gave to each frame, in any order: later answers must equal it.
        let mut first_answer: BTreeMap<i64, Evaluated> = BTreeMap::new();
        for order in orders["results"].as_array().expect("orders") {
            let name = order["name"].as_str().unwrap_or("?");
            // A fresh import per order: nothing carries over from the previous seek pattern.
            let scene = match import(&case) {
                Ok(scene) => scene,
                Err(message) => {
                    report.fail(message);
                    continue;
                }
            };
            for frame in order["frames"].as_array().expect("order frames") {
                let edit = frame.as_i64().expect("edit frame");
                let label = format!("{} [{name}] edit {edit}", case.id);
                let actual = evaluate(&scene, edit);
                match canonical.get(&edit) {
                    Some(expected) => compare_evaluated(&mut report, &label, &actual, expected),
                    None => report.fail(format!("{label}: no canonical origin answer")),
                }
                match first_answer.get(&edit) {
                    Some(previous) if *previous != actual => report.fail(format!(
                        "{label}: Rust answers differently than the first time it was asked"
                    )),
                    Some(_) => {}
                    None => {
                        first_answer.insert(edit, actual);
                    }
                }
            }
            orders_run += 1;
        }
    }
    assert_eq!(orders_run, 7, "seek-order coverage changed");
    report.finish("seek orders");
}

fn pose_of(camera: &Value) -> CameraPose {
    let vec3 = |value: &Value| Vec3 {
        x: value["x"].as_f64().expect("x"),
        y: value["y"].as_f64().expect("y"),
        z: value["z"].as_f64().expect("z"),
    };
    CameraPose {
        position: vec3(&camera["position"]),
        target: vec3(&camera["target"]),
        fov_deg: camera["fovDeg"].as_f64().expect("fovDeg"),
    }
}

fn compare_camera_matrix(report: &mut Report, label: &str, pose: &CameraPose, expected: &Value) {
    report.matrix(
        "camera world matrix",
        label,
        &json!(pose.to_world_matrix().0),
        expected,
        CAMERA_WORLD_MATRIX,
    );
}

#[test]
fn camera_to_world_matrix_matches_the_three_look_at_of_the_origin_poses() {
    let mut report = Report::default();
    let mut matrices = 0;
    for case in cases() {
        let Some(world) = case.expected["cameraWorld"].as_array() else {
            continue;
        };
        let frames = case.expected["evaluation"]["editFrames"]
            .as_array()
            .expect("evaluation frames");
        assert_eq!(world.len(), frames.len(), "{}", case.id);
        let scene = if documented_limit(&case.id).is_some() {
            None
        } else {
            Some(import(&case).expect("accepted bundles import"))
        };
        for (entry, frame) in world.iter().zip(frames) {
            if entry["ok"] == false {
                assert_eq!(
                    frame["ok"], false,
                    "{} edit {}",
                    case.id, entry["editFrame"]
                );
                continue;
            }
            assert_eq!(entry["source"], "three-library", "{}", case.id);
            let edit = entry["editFrame"].as_i64().expect("edit frame");
            let label = format!("{} edit {edit}", case.id);
            // 1) the matrix of the pose the ORIGIN evaluated: isolates `to_world_matrix`.
            compare_camera_matrix(
                &mut report,
                &format!("{label} (origin pose)"),
                &pose_of(&frame["state"]["camera"]),
                &entry["matrix"],
            );
            // 2) end to end: the pose Rust evaluates, then its matrix.
            if let Some(scene) = &scene {
                match scene.bundle().evaluate_frame(edit) {
                    Ok(evaluated) => compare_camera_matrix(
                        &mut report,
                        &format!("{label} (Rust pose)"),
                        &evaluated.camera.pose(),
                        &entry["matrix"],
                    ),
                    Err(error) => report.fail(format!("{label}: Rust fails with `{error}`")),
                }
            }
            matrices += 1;
        }
    }
    assert!(
        matrices >= 300,
        "camera matrix coverage decreased: {matrices}"
    );
    report.finish("camera world matrices");
}

// ---- rational frame rates and time conversion --------------------------------------------

fn rate_of(fps: &Value) -> Option<Rate> {
    Rate::new(
        u32::try_from(fps["numerator"].as_u64()?).ok()?,
        u32::try_from(fps["denominator"].as_u64()?).ok()?,
    )
}

fn ratio_of(exact: &Value) -> Ratio {
    let parse = |key: &str| {
        exact[key]
            .as_str()
            .and_then(|text| text.parse::<i64>().ok())
            .unwrap_or_else(|| panic!("`{key}` of {exact} is not an i64 string"))
    };
    Ratio::new(parse("num"), parse("den")).expect("a representable ratio")
}

/// What a seconds → frame probe says, for readable failure messages.
struct ProbeCheck<'a> {
    label: &'a str,
    origin: i64,
    exact: i64,
}

impl ProbeCheck<'_> {
    fn compare(&self, report: &mut Report, what: &str, actual: i64, expected: i64) {
        report.compared += 1;
        if actual != expected {
            report.fail(format!(
                "{}: {what} Rust {actual} != expected {expected} (origin {}, exact floor {})",
                self.label, self.origin, self.exact
            ));
        }
    }
}

/// Compares one case's time block with a rate; returns how many probes were float artifacts.
fn compare_time(
    report: &mut Report,
    id: &str,
    rate: Rate,
    time: &Value,
    scene: Option<&DirectedScene>,
) -> usize {
    report.close_json(
        "time",
        &format!("{id} rate"),
        &json!(rate.as_f64()),
        &time["rate"],
        TIME_SECONDS,
    );
    let duration = time["editDurationFrames"].as_i64().expect("duration");
    for (name, frame) in [
        ("timelineDurationSeconds", duration),
        ("maxEditSeconds", duration - 1),
    ] {
        report.close_json(
            "time",
            &format!("{id} {name}"),
            &json!(rate.frame_to_time(frame).to_f64()),
            &time[name],
            TIME_SECONDS,
        );
    }
    for entry in time["frames"].as_array().expect("time frames") {
        let frame = entry["frame"].as_i64().expect("frame");
        let seconds = json!(rate.frame_to_time(frame).to_f64());
        for key in ["seconds", "editSeconds"] {
            report.close_json(
                "time",
                &format!("{id} frame {frame} {key}"),
                &seconds,
                &entry[key],
                TIME_SECONDS,
            );
        }
    }
    let mut artifacts = 0;
    for probe in time["seconds"].as_array().expect("time probes") {
        let ratio = ratio_of(&probe["exact"]);
        let frame = probe["frame"].as_i64().expect("probe frame");
        let label = format!(
            "{id} {} of frame {frame} (exact {}/{})",
            probe["label"].as_str().unwrap_or("?"),
            probe["exact"]["num"].as_str().unwrap_or("?"),
            probe["exact"]["den"].as_str().unwrap_or("?"),
        );
        let origin = probe["floorFrame"].as_i64().expect("floorFrame");
        let exact = probe["exactFloorFrame"].as_i64().expect("exactFloorFrame");
        // Where the origin's floating-point floor is not the exact floor the fixture says so;
        // Rust (exact rationals) must then equal the exact floor, and the origin everywhere else.
        let expected_floor = if probe["originMatchesExact"] == true {
            origin
        } else {
            artifacts += 1;
            exact
        };
        let probe_check = ProbeCheck {
            label: &label,
            origin,
            exact,
        };
        let floor = rate.time_to_frame_floor(ratio);
        probe_check.compare(report, "time_to_frame_floor", floor, expected_floor);
        if let Some(scene) = scene {
            match scene.bundle().frame_at_time(ratio) {
                Ok(by_bundle) => {
                    probe_check.compare(report, "Bundle::frame_at_time", by_bundle, expected_floor);
                }
                Err(error) => report.fail(format!(
                    "{label}: Bundle::frame_at_time fails with `{error}`"
                )),
            }
        }
        let expected_clamped = if probe["originMatchesExact"] == true {
            probe["clampedFrame"].as_i64().expect("clampedFrame")
        } else {
            exact.clamp(0, duration - 1)
        };
        probe_check.compare(
            report,
            "clamped to the edit",
            floor.clamp(0, duration - 1),
            expected_clamped,
        );
    }
    artifacts
}

#[test]
fn rational_frame_rates_convert_time_like_the_origin() {
    let mut report = Report::default();
    let mut artifacts = 0;
    let mut cases_with_time = 0;
    for case in cases() {
        let time = &case.expected["time"];
        if time.is_null() {
            continue;
        }
        cases_with_time += 1;
        let Some(rate) = rate_of(&time["fps"]) else {
            assert!(
                documented_limit(&case.id).is_some(),
                "{}: Rate refuses {}",
                case.id,
                time["fps"]
            );
            continue;
        };
        let scene = match import(&case) {
            Ok(scene) => scene,
            Err(message) => {
                report.fail(message);
                continue;
            }
        };
        let found = compare_time(&mut report, &case.id, rate, time, Some(&scene));
        assert_eq!(
            found as u64,
            time["originFloatArtifacts"].as_u64().unwrap(),
            "{}: float-artifact count differs from the fixture",
            case.id
        );
        artifacts += found;
    }
    assert_eq!(cases_with_time, 8, "time coverage changed");
    assert_eq!(
        artifacts, EXPECTED_ORIGIN_FLOAT_ARTIFACTS,
        "the number of probes where the origin's floor is not exact changed: re-triage"
    );
    assert_eq!(
        manifest_summary()["originFloatArtifacts"].as_u64(),
        Some(EXPECTED_ORIGIN_FLOAT_ARTIFACTS as u64)
    );
    report.finish("rational time");
}

#[test]
fn ntsc_frames_start_on_the_exact_rational_second() {
    // The case this whole family exists for: 30000/1001 fps, frame 30000 starts at 1001 s.
    let rate = Rate::new(30000, 1001).expect("30000/1001 is a valid rate");
    assert!(rate.frame_to_time(30000) == Ratio::from_int(1001));
    assert_eq!(rate.time_to_frame_floor(Ratio::from_int(1001)), 30000);
    let just_before = Ratio::new(1001 * 1_000_000 - 1, 1_000_000).expect("ratio");
    assert_eq!(rate.time_to_frame_floor(just_before), 29999);
}

// ---- negative controls: the comparisons above can fail ------------------------------------

fn case_named(name: &str) -> Case {
    cases()
        .into_iter()
        .find(|case| case.id == name)
        .unwrap_or_else(|| panic!("case {name} is missing"))
}

#[test]
fn control_tolerance_is_applied_exactly() {
    for (kind, tolerance) in [
        ("world matrix", WORLD_MATRIX),
        ("camera", CAMERA),
        ("time", TIME_SECONDS),
    ] {
        let expected = 100.0;
        let limit = tolerance.absolute + tolerance.relative * expected;
        let mut inside = Report::default();
        inside.close(kind, "inside", expected + 0.5 * limit, expected, tolerance);
        assert!(inside.failures.is_empty(), "{kind}: {:?}", inside.failures);
        let mut outside = Report::default();
        outside.close(kind, "outside", expected + 2.0 * limit, expected, tolerance);
        assert_eq!(outside.failures.len(), 1, "{kind}");
        let mut not_a_number = Report::default();
        not_a_number.close(kind, "nan", f64::NAN, expected, tolerance);
        assert_eq!(not_a_number.failures.len(), 1, "{kind}: NaN must fail");
    }
}

#[test]
fn control_a_flipped_origin_verdict_is_reported_with_the_distances() {
    let case = case_named("join-position-above");
    let mut expected = case.expected.clone();
    expected["validation"]["accepted"] = json!(true);
    expected["validation"]["editJoins"] = json!([]);
    let mut report = Report::default();
    compare_verdict(&mut report, &case.id, &case.raw, &expected);
    assert_eq!(report.failures.len(), 2, "{:?}", report.failures);
    assert!(
        report.failures[0].contains("CONTINUOUS_CAMERA_JUMP"),
        "{:?}",
        report.failures
    );
    assert!(
        report.failures[0].contains("limit 0.25"),
        "the message must carry the distances: {:?}",
        report.failures
    );
}

#[test]
fn control_a_changed_camera_behaviour_flips_the_join_verdict() {
    // Same expectation, different bundle: the dolly of the "below" case made too long to be smooth,
    // and the "above" case made short enough. The comparison must notice both.
    let below = case_named("join-position-below");
    let mut too_fast = below.raw.clone();
    too_fast["cameraClips"][0]["segments"][0]["distanceSceneUnits"] = json!(4.1);
    let mut report = Report::default();
    compare_verdict(&mut report, &below.id, &too_fast, &below.expected);
    assert!(
        !report.failures.is_empty(),
        "a faster dolly passed as smooth"
    );

    let above = case_named("join-position-above");
    let mut slower = above.raw.clone();
    slower["cameraClips"][0]["segments"][0]["distanceSceneUnits"] = json!(3.9);
    let mut report = Report::default();
    compare_verdict(&mut report, &above.id, &slower, &above.expected);
    assert!(!report.failures.is_empty(), "a slower dolly kept the jump");

    let mut untouched = Report::default();
    compare_verdict(&mut untouched, &above.id, &above.raw, &above.expected);
    assert!(untouched.failures.is_empty(), "{:?}", untouched.failures);
}

#[test]
fn control_a_perturbed_world_matrix_is_caught_and_a_tiny_one_is_not() {
    let case = case_named("world-sockets-chain-animated");
    let scene = import(&case).expect("accepted bundle");
    let frame = case.expected["evaluation"]["editFrames"]
        .as_array()
        .expect("frames")
        .iter()
        .find(|frame| frame["editFrame"] == 8)
        .expect("edit frame 8 is probed")
        .clone();
    let actual = evaluate(&scene, 8);
    let mut baseline = Report::default();
    compare_evaluated(&mut baseline, "baseline", &actual, &frame);
    assert!(baseline.failures.is_empty(), "{:?}", baseline.failures);

    let node = frame["state"]["nodes"][3]["nodeId"]
        .as_str()
        .expect("node")
        .to_string();
    let original = frame["worldMatrices"][node.as_str()][12]
        .as_f64()
        .expect("element");

    let mut perturbed = frame.clone();
    perturbed["worldMatrices"][node.as_str()][12] = json!(original + 1e-6);
    let mut report = Report::default();
    compare_evaluated(&mut report, "perturbed", &actual, &perturbed);
    assert_eq!(report.failures.len(), 1, "{:?}", report.failures);
    assert!(report.failures[0].contains(&node), "{:?}", report.failures);

    let mut tiny = frame.clone();
    tiny["worldMatrices"][node.as_str()][12] = json!(original + 1e-13);
    let mut report = Report::default();
    compare_evaluated(&mut report, "tiny", &actual, &tiny);
    assert!(report.failures.is_empty(), "{:?}", report.failures);

    // A wrong evaluator, not only a wrong expectation: the same expectation at another frame.
    let other = evaluate(&scene, 16);
    let mut report = Report::default();
    compare_evaluated(&mut report, "other frame", &other, &frame);
    assert!(!report.failures.is_empty(), "frame 16 passed as frame 8");
}

#[test]
fn control_a_transposed_camera_matrix_and_a_wrong_rate_are_caught() {
    let case = case_named("world-camera-lookat-poses");
    let frames = case.expected["evaluation"]["editFrames"]
        .as_array()
        .expect("frames");
    let world = case.expected["cameraWorld"].as_array().expect("matrices");
    let index = frames
        .iter()
        .position(|frame| frame["editFrame"] == 5)
        .expect("edit frame 5 is probed");
    let pose = pose_of(&frames[index]["state"]["camera"]);
    let mut matrix = world[index]["matrix"].clone();
    let mut honest = Report::default();
    compare_camera_matrix(&mut honest, "honest", &pose, &matrix);
    assert!(honest.failures.is_empty(), "{:?}", honest.failures);

    let (a, b) = (matrix[1].clone(), matrix[4].clone());
    matrix[1] = b;
    matrix[4] = a;
    let mut transposed = Report::default();
    compare_camera_matrix(&mut transposed, "transposed", &pose, &matrix);
    assert!(
        !transposed.failures.is_empty(),
        "a transposed matrix passed"
    );

    let time_case = case_named("fps-30000-1001");
    let mut wrong = Report::default();
    let thirty = Rate::new(30, 1).expect("30 fps");
    compare_time(
        &mut wrong,
        &time_case.id,
        thirty,
        &time_case.expected["time"],
        None,
    );
    assert!(
        wrong.failures.len() > 10,
        "30/1 passed as 30000/1001: {} failures",
        wrong.failures.len()
    );
}

#[test]
fn control_the_float_artifact_rule_does_not_hide_a_wrong_frame() {
    // A probe the fixture marks as an origin float artifact must still fail when Rust is neither the
    // origin's value nor the exact floor.
    let case = case_named("fps-30000-1001");
    let mut time = case.expected["time"].clone();
    let rate = rate_of(&time["fps"]).expect("rate");
    let probes = time["seconds"].as_array_mut().expect("probes");
    let artifact = probes
        .iter_mut()
        .find(|probe| probe["originMatchesExact"] == false)
        .expect("the corpus has a float artifact at 30000/1001");
    let exact = artifact["exactFloorFrame"].as_i64().expect("exact floor");
    artifact["exactFloorFrame"] = json!(exact + 7);
    let mut report = Report::default();
    compare_time(&mut report, &case.id, rate, &time, None);
    assert!(
        !report.failures.is_empty(),
        "a wrong exact floor was accepted"
    );
}
