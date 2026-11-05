//! BB03: time and camera of a directed scene, through the public library API only (no GPUI, no
//! window, no runtime).
//!
//! Two kinds of evidence live here and are kept apart:
//!
//! - **Equivalence with the origin**, on the 4 valid bundles of the oracle corpus
//!   (`docs/migration/fixtures`): every edit frame and clip frame the oracle probed, including the
//!   border frames (-1, first, last, one past the end), the durations, the main-track poses and the
//!   edit-join verdicts of all 55 cases. The expected values come from the origin's own functions.
//! - **Behaviour of this evaluator**, on synthetic bundles written for these tests: positive and
//!   negative cuts, out-of-order and repeated seeks, rational frame rates (30000/1001), the
//!   Unsupported boundary. They prove this module, not equivalence.
//!
//! # Numeric tolerances (fixed before the Rust output was compared with the corpus)
//!
//! | Quantity | Rule |
//! |---|---|
//! | frame numbers, ids, counts, `ok`, the join issue codes | exact |
//! | the origin's two range error messages | exact text |
//! | position, target, `fovDeg`, velocity, `fovVelocityDegPerFrame`, main-track poses | `abs <= 1e-9 + 1e-12 * abs(expected)` |
//!
//! Why: a pose is a chain of at most a few dozen double operations plus `sin`/`cos`/`asin`/`atan2`,
//! which the origin gets from V8 (fdlibm) and Rust from the host libm; they can differ by an ulp.
//! Values are O(1..100) scene units, so the honest error is below 1e-12; velocity is a difference
//! of two poses and no worse. 1e-9 leaves three orders of margin and still sits six orders below
//! the smallest effect of a wrong recipe, easing or key (≥ 1e-3), so it cannot hide one.
//! Static matrices (BB02) keep their own `1e-10 + 1e-12` rule in `directed_scene_oracle.rs`.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tessel_editor_core::CommandError;
use tessel_editor_core::directed_scene::{
    CameraState, DirectedScene, LAYER_FOV_SWEEP_MAX_FRAMES, NodeAnimation, TemporalError,
    Unsupported, Vec3,
};
use tessel_editor_core::time::{Rate, Ratio};

const LAYERED: &str = include_str!("directed_scene_fixtures/layered_scene.json");

const ABSOLUTE: f64 = 1e-9;
const RELATIVE: f64 = 1e-12;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/migration/fixtures")
}

fn read_json(path: impl AsRef<Path>) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn corpus_cases() -> Vec<Value> {
    read_json(fixtures().join("cases.json"))["cases"]
        .as_array()
        .unwrap()
        .clone()
}

fn corpus_valid() -> Vec<(String, Value, Value)> {
    corpus_cases()
        .into_iter()
        .filter(|case| case["kind"] == "valid")
        .map(|case| {
            let root = fixtures();
            (
                case["id"].as_str().unwrap().to_string(),
                read_json(root.join(case["bundle"].as_str().unwrap())),
                read_json(root.join(case["expected"].as_str().unwrap())),
            )
        })
        .collect()
}

/// Largest error seen against the origin, so the report can state it (printed with `--nocapture`).
#[derive(Default)]
struct Worst {
    value: f64,
    at: String,
    compared: usize,
}

impl Worst {
    fn close(&mut self, label: &str, actual: f64, expected: f64) {
        let error = (actual - expected).abs();
        assert!(
            error <= ABSOLUTE + RELATIVE * expected.abs(),
            "{label}: {actual} != {expected} (error {error:e})"
        );
        self.compared += 1;
        if error > self.value {
            self.value = error;
            self.at = label.to_string();
        }
    }

    fn vec3(&mut self, label: &str, actual: Vec3, expected: &Value) {
        self.close(
            &format!("{label}.x"),
            actual.x,
            expected["x"].as_f64().unwrap(),
        );
        self.close(
            &format!("{label}.y"),
            actual.y,
            expected["y"].as_f64().unwrap(),
        );
        self.close(
            &format!("{label}.z"),
            actual.z,
            expected["z"].as_f64().unwrap(),
        );
    }

    fn camera(&mut self, label: &str, actual: &CameraState, expected: &Value) {
        self.vec3(
            &format!("{label}.position"),
            actual.position,
            &expected["position"],
        );
        self.vec3(
            &format!("{label}.target"),
            actual.target,
            &expected["target"],
        );
        self.close(
            &format!("{label}.fovDeg"),
            actual.fov_deg,
            expected["fovDeg"].as_f64().unwrap(),
        );
        self.vec3(
            &format!("{label}.velocity"),
            actual.velocity,
            &expected["velocity"],
        );
        self.close(
            &format!("{label}.fovVelocityDegPerFrame"),
            actual.fov_velocity_deg_per_frame,
            expected["fovVelocityDegPerFrame"].as_f64().unwrap(),
        );
    }
}

// ---- equivalence with the origin ----------------------------------------------------------

#[test]
fn edit_frames_match_the_origin_on_every_valid_bundle_including_the_borders() {
    let mut worst = Worst::default();
    let mut frames = 0;
    let mut errors = 0;
    for (id, raw, expected) in corpus_valid() {
        let scene = DirectedScene::import_value(raw.clone()).unwrap();
        let bundle = scene.bundle();
        for frame in expected["evaluation"]["editFrames"].as_array().unwrap() {
            let edit_frame = frame["editFrame"].as_i64().unwrap();
            let label = format!("{id} edit {edit_frame}");
            if frame["ok"] == false {
                let error = bundle.evaluate_camera(edit_frame).unwrap_err();
                assert_eq!(
                    error.to_string(),
                    frame["error"].as_str().unwrap(),
                    "{label}"
                );
                assert_eq!(bundle.evaluate_frame(edit_frame).unwrap_err(), error);
                assert_eq!(error.code(), "frame_out_of_range");
                errors += 1;
                continue;
            }
            let state = &frame["state"];
            let camera = bundle.evaluate_camera(edit_frame).unwrap();
            let mapping = &camera.mapping;
            assert_eq!(mapping.edit_frame as i64, edit_frame, "{label}");
            assert_eq!(
                mapping.instance.id,
                state["instanceId"].as_str().unwrap(),
                "{label}"
            );
            assert_eq!(
                mapping.instance_frame,
                state["instanceFrame"].as_u64().unwrap(),
                "{label}"
            );
            assert_eq!(
                mapping.shot.camera_clip_id,
                state["cameraClipId"].as_str().unwrap(),
                "{label}"
            );
            assert_eq!(
                mapping.camera_clip_frame,
                state["cameraClipFrame"].as_u64().unwrap(),
                "{label}"
            );
            worst.camera(&label, &camera.camera, &state["camera"]);

            // The whole-frame answer carries the same camera. Node animation is evaluated now (its
            // equivalence with the origin is `directed_scene_animation.rs`); here it only has to
            // agree with the origin on whether channels applied. These bundles declare no lights,
            // weather or rigs, so every frame is whole.
            let whole = bundle.evaluate_frame(edit_frame).unwrap();
            assert_eq!(whole.camera, camera.camera, "{label}");
            let overridden = !state["performanceOverrides"].as_array().unwrap().is_empty();
            assert_eq!(
                matches!(whole.node_animation, NodeAnimation::Animated { .. }),
                overridden,
                "{label}"
            );
            assert!(whole.is_complete(), "{label}");
            frames += 1;
        }
    }
    eprintln!(
        "edit frames: {frames} ok + {errors} out of range; {} numbers, worst error {:e} at {}",
        worst.compared, worst.value, worst.at
    );
    assert!(frames >= 24 && errors >= 8, "corpus coverage decreased");
}

#[test]
fn camera_clips_durations_poses_and_clip_frames_match_the_origin() {
    let mut worst = Worst::default();
    let mut clips = 0;
    for (id, raw, expected) in corpus_valid() {
        let scene = DirectedScene::import_value(raw).unwrap();
        let bundle = scene.bundle();
        for clip in expected["evaluation"]["cameraClips"].as_array().unwrap() {
            let clip_id = clip["clipId"].as_str().unwrap();
            assert_eq!(
                bundle.camera_clip_frames(clip_id),
                clip["durationFrames"].as_u64(),
                "{id} {clip_id}"
            );
            let poses = bundle.main_track_poses(clip_id).unwrap();
            let expected_poses = clip["mainTrackPoses"].as_array().unwrap();
            assert_eq!(poses.len(), expected_poses.len(), "{id} {clip_id}");
            for (i, (actual, expected)) in poses.iter().zip(expected_poses).enumerate() {
                for (edge, pose, expected) in [
                    ("start", actual.start, &expected["start"]),
                    ("end", actual.end, &expected["end"]),
                ] {
                    let label = format!("{id} {clip_id} pose {i} {edge}");
                    worst.vec3(
                        &format!("{label}.position"),
                        pose.position,
                        &expected["position"],
                    );
                    worst.vec3(&format!("{label}.target"), pose.target, &expected["target"]);
                    worst.close(
                        &format!("{label}.fovDeg"),
                        pose.fov_deg,
                        expected["fovDeg"].as_f64().unwrap(),
                    );
                }
            }
            // One layout for all the frames, asked in the order the oracle recorded them.
            let timeline = bundle.camera_timeline(clip_id).unwrap();
            for frame in clip["frames"].as_array().unwrap() {
                let clip_frame = frame["clipFrame"].as_i64().unwrap();
                let label = format!("{id} {clip_id} frame {clip_frame}");
                let result = timeline.state_at(clip_frame);
                if frame["ok"] == false {
                    assert_eq!(
                        result.unwrap_err().to_string(),
                        frame["error"].as_str().unwrap(),
                        "{label}"
                    );
                    continue;
                }
                let camera = result.unwrap();
                worst.camera(&label, &camera, &frame["camera"]);
                // Pose-only and one-shot paths agree with the full state.
                assert_eq!(
                    timeline.pose_at(clip_frame).unwrap(),
                    camera.pose(),
                    "{label}"
                );
                assert_eq!(
                    bundle.camera_at_clip_frame(clip_id, clip_frame).unwrap(),
                    camera
                );
            }
            clips += 1;
        }
    }
    eprintln!(
        "clips: {clips}; {} numbers, worst error {:e} at {}",
        worst.compared, worst.value, worst.at
    );
    // 4 valid bundles; `valid-cut-instance-offsets` has two clips.
    assert_eq!(clips, 5);
    assert!(
        worst.compared >= 850,
        "coverage decreased: {}",
        worst.compared
    );
}

/// Rewrites every easing under `value` to `linear`.
fn flatten_easings(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                if key == "easing" {
                    *item = json!("linear");
                } else {
                    flatten_easings(item);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(flatten_easings),
        _ => {}
    }
}

#[test]
fn the_comparison_would_catch_a_wrong_easing() {
    // Negative control for the two tests above: change the easings and the same comparison must
    // fail by far more than the tolerance, so a pass is not an artefact of loose numbers.
    let (_, mut raw, expected) = corpus_valid()
        .into_iter()
        .find(|(id, _, _)| id == "valid-camera-recipes-layers")
        .unwrap();
    flatten_easings(&mut raw["cameraClips"]);
    let scene = import(raw);
    let mut largest = 0.0_f64;
    for clip in expected["evaluation"]["cameraClips"].as_array().unwrap() {
        let timeline = scene
            .bundle()
            .camera_timeline(clip["clipId"].as_str().unwrap())
            .unwrap();
        for frame in clip["frames"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["ok"] == true)
        {
            let actual = timeline
                .state_at(frame["clipFrame"].as_i64().unwrap())
                .unwrap();
            let expected = &frame["camera"]["position"];
            largest = largest
                .max((actual.position.x - expected["x"].as_f64().unwrap()).abs())
                .max((actual.position.z - expected["z"].as_f64().unwrap()).abs());
        }
    }
    assert!(
        largest > 1e-3,
        "a wrong easing moved positions by only {largest}"
    );
}

/// Origin verdict per shot id: the codes `validateEditJoins` returned.
fn expected_join_codes(expected: &Value) -> Option<Vec<(String, String)>> {
    let joins = expected["validation"]["editJoins"].as_array()?;
    let mut codes: Vec<(String, String)> = joins
        .iter()
        .map(|join| {
            (
                join["code"].as_str().unwrap().to_string(),
                join["shotId"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    codes.sort();
    Some(codes)
}

#[test]
fn structural_plus_temporal_gates_equal_the_origin_edit_joins_on_all_55_cases() {
    let root = fixtures();
    let mut compared = 0;
    let mut with_temporal = 0;
    for case in corpus_cases() {
        let id = case["id"].as_str().unwrap();
        let expected = read_json(root.join(case["expected"].as_str().unwrap()));
        let raw = read_json(root.join(case["bundle"].as_str().unwrap()));
        let Ok(scene) = DirectedScene::import_value(raw) else {
            // The importer already refuses it; nothing to join.
            assert_eq!(expected["validation"]["accepted"], false, "{id}");
            continue;
        };
        let Some(expected_codes) = expected_join_codes(&expected) else {
            continue;
        };
        let shots = &scene.bundle().shots;
        let shot_of = |path: &str| -> String {
            // `/shots/<index>` names the later shot of the join, which is what the origin reports.
            path.strip_prefix("/shots/")
                .and_then(|rest| rest.split('/').next())
                .and_then(|index| index.parse::<usize>().ok())
                .map(|index| shots[index].id.clone())
                .unwrap_or_default()
        };
        let actual: Vec<(String, String)> = scene
            .edit_join_issues()
            .iter()
            .chain(scene.temporal_join_issues().iter())
            .map(|issue| (issue.code.to_string(), shot_of(&issue.path)))
            .collect();
        // Segment resets name a clip, not a shot, in the origin: compare their codes only.
        let blank = |codes: Vec<(String, String)>| -> Vec<(String, String)> {
            codes
                .into_iter()
                .map(|(code, shot)| {
                    if code == "CAMERA_SEGMENT_RESET" {
                        (code, String::new())
                    } else {
                        (code, shot)
                    }
                })
                .collect()
        };
        let mut actual = blank(actual);
        let mut expected_codes = blank(expected_codes);
        actual.sort();
        expected_codes.sort();
        assert_eq!(actual, expected_codes, "{id}");
        compared += 1;
        if scene
            .temporal_join_issues()
            .iter()
            .any(|issue| issue.code.starts_with("CONTINUOUS_"))
        {
            with_temporal += 1;
        }
        // Gate coherence on the corpus: accepted by the origin <=> no finding of either gate.
        let clean = scene.edit_join_issues().is_empty() && scene.temporal_issues().is_empty();
        assert_eq!(
            clean,
            expected["validation"]["accepted"].as_bool().unwrap(),
            "{id}"
        );
    }
    assert!(compared >= 10, "only {compared} cases reached the joins");
    assert_eq!(with_temporal, 1, "the corpus has one temporal-jump case");
}

// ---- the fixture the structural gate lets through -----------------------------------------

fn layered() -> Value {
    serde_json::from_str(LAYERED).unwrap()
}

fn import(value: Value) -> DirectedScene {
    DirectedScene::import_value(value).unwrap_or_else(|error| panic!("{error:?}"))
}

fn codes(issues: &[tessel_editor_core::directed_scene::Issue]) -> Vec<&'static str> {
    issues.iter().map(|issue| issue.code).collect()
}

#[test]
fn layered_scene_passes_the_structural_gate_but_the_temporal_gate_rejects_it() {
    let scene = import(layered());
    assert_eq!(scene.edit_join_issues(), vec![], "structurally sound");
    let temporal = scene.temporal_join_issues();
    // `finalizeBundle` of the origin rejects this very fixture with CONTINUOUS_VELOCITY_JUMP
    // (confirmed by the coordinator with the origin's own functions).
    assert_eq!(
        codes(&temporal),
        ["CONTINUOUS_VELOCITY_JUMP"],
        "{temporal:?}"
    );
    eprintln!("layered join: {}", temporal[0].message);
    assert_eq!(temporal[0].path, "/shots/1");
    assert!(
        temporal[0].message.contains("s1|s2"),
        "{}",
        temporal[0].message
    );
    assert_eq!(scene.temporal_issues(), temporal);

    // The jump, seen from the camera: the last frame of s1 and the first of s2.
    let bundle = scene.bundle();
    let end = bundle.camera_at_clip_frame("wide", 23).unwrap();
    let start = bundle.camera_at_clip_frame("wide", 24).unwrap();
    let jump = (end.velocity.x - start.velocity.x)
        .hypot(end.velocity.y - start.velocity.y)
        .hypot(end.velocity.z - start.velocity.z);
    assert!(
        jump > 0.08,
        "velocity jump {jump} is above the origin's 0.08"
    );
    assert!(
        (end.position.z - start.position.z).abs() < 0.25,
        "the pose itself is continuous"
    );
}

#[test]
fn every_valid_bundle_of_the_corpus_passes_both_gates() {
    for (id, raw, _) in corpus_valid() {
        let scene = import(raw);
        assert_eq!(scene.edit_join_issues(), vec![], "{id}");
        assert_eq!(scene.temporal_issues(), vec![], "{id}");
    }
}

// ---- negative and positive cuts (synthetic variants, also run through the origin) ------------

const VARIANTS: &str = "directed_scene_temporal_fixtures";

fn variants_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(VARIANTS)
}

/// A variant written by `directed_scene_temporal_fixtures/generate.mjs`, imported.
fn variant(name: &str) -> DirectedScene {
    import(read_json(
        variants_dir().join(format!("{name}.bundle.json")),
    ))
}

#[test]
fn a_smooth_continuous_join_passes_and_each_jump_is_named() {
    // Smooth: the join sits inside one dolly, same speed on both sides.
    assert_eq!(variant("smooth-single-dolly").temporal_issues(), vec![]);
    // Smooth: a hold has no speed on either side.
    assert_eq!(variant("resting-hold").temporal_issues(), vec![]);

    // Speed: the pose is continuous but the second dolly starts at 5/23 per frame where the
    // first one stopped (the last frame of a movement has speed 0, as in the origin).
    let velocity = variant("velocity-jump");
    assert_eq!(
        codes(&velocity.temporal_join_issues()),
        ["CONTINUOUS_VELOCITY_JUMP"]
    );
    assert_eq!(velocity.edit_join_issues(), vec![]);

    // Field-of-view speed.
    assert_eq!(
        codes(&variant("fov-velocity-jump").temporal_join_issues()),
        ["CONTINUOUS_FOV_VELOCITY_JUMP"]
    );

    // Position and target: a reset to the clip's start pose mid-clip. The structural gate names
    // the reset, the temporal gate the jump it causes.
    let reset = variant("reset-position-target-jump");
    assert_eq!(
        codes(&reset.temporal_join_issues()),
        ["CONTINUOUS_CAMERA_JUMP", "CONTINUOUS_TARGET_JUMP"]
    );
    assert_eq!(codes(&reset.edit_join_issues()), ["CAMERA_SEGMENT_RESET"]);

    // Field of view.
    let fov = variant("reset-fov-jump");
    assert_eq!(codes(&fov.temporal_join_issues()), ["CONTINUOUS_FOV_JUMP"]);
}

#[test]
fn variants_match_the_origin_verdicts_exactly_and_the_layered_camera_numerically() {
    let origin = read_json(variants_dir().join("origin.json"));
    assert_eq!(
        origin["pinnedCommit"],
        "d6adac15c61f4ef582a7ec259397b2e025d1f059"
    );
    let mut worst = Worst::default();
    let mut verdicts = 0;
    for (name, expected) in origin["variants"].as_object().unwrap() {
        let scene = variant(name);
        let sorted = |mut list: Vec<String>| {
            list.sort();
            list
        };
        let strings = |value: &Value| -> Vec<String> {
            value
                .as_array()
                .unwrap()
                .iter()
                .map(|code| code.as_str().unwrap().to_string())
                .collect()
        };
        // Join codes: the two gates together are the origin's `validateEditJoins`.
        let joins = sorted(
            scene
                .edit_join_issues()
                .iter()
                .chain(&scene.temporal_join_issues())
                .map(|issue| issue.code.to_string())
                .collect(),
        );
        assert_eq!(
            joins,
            strings(&expected["editJoinCodes"]),
            "{name}: join codes"
        );
        // The origin's schema also runs the stacked field-of-view rule; here it is temporal.
        let stacked_fov = scene
            .temporal_issues()
            .iter()
            .any(|issue| issue.code == "FOV_OUT_OF_RANGE");
        assert_eq!(
            stacked_fov,
            strings(&expected["schemaIssueCodes"]).contains(&"FOV_OUT_OF_RANGE".to_string()),
            "{name}: stacked field of view"
        );
        // Accepted by the origin <=> neither gate has a finding.
        let clean = scene.edit_join_issues().is_empty() && scene.temporal_issues().is_empty();
        assert_eq!(
            clean,
            expected["accepted"].as_bool().unwrap(),
            "{name}: verdict"
        );
        verdicts += 1;

        for frame in expected["editFrames"].as_array().into_iter().flatten() {
            let edit_frame = frame["editFrame"].as_i64().unwrap();
            let label = format!("{name} edit {edit_frame}");
            let result = scene.bundle().evaluate_camera(edit_frame);
            if frame["ok"] == false {
                assert_eq!(
                    result.unwrap_err().to_string(),
                    frame["error"].as_str().unwrap()
                );
                continue;
            }
            worst.camera(&label, &result.unwrap().camera, &frame["state"]["camera"]);
        }
    }
    eprintln!(
        "variants: {verdicts} verdicts; layered camera {} numbers, worst error {:e} at {}",
        worst.compared, worst.value, worst.at
    );
    assert_eq!(verdicts, 9);
    assert!(worst.compared >= 48 * 11, "layered coverage decreased");
}

#[test]
fn only_continuous_cuts_on_one_clip_are_compared() {
    // The same jumping clip, but the second shot is a cut: not a continuous join, not compared.
    let cut = variant("cut-after-jumping-clip");
    assert_eq!(cut.temporal_issues(), vec![]);
    assert_eq!(codes(&cut.edit_join_issues()), ["CAMERA_SEGMENT_RESET"]);

    // A continuous join across two clips is the structural gate's (`CONTINUOUS_CROSS_CLIP_...`);
    // the temporal one has no common clip to compare and stays silent.
    let mut two = read_json(variants_dir().join("cut-after-jumping-clip.bundle.json"));
    two["shots"][1]["transitionInto"] = json!({ "type": "continuous" });
    two["cameraClips"].as_array_mut().unwrap().push(json!({
        "id": "other", "initialPosition": {"x": 5.0, "y": 2.0, "z": 5.0},
        "initialTarget": {"x": 0.0, "y": 1.0, "z": 0.0}, "initialFovDeg": 30.0,
        "segments": [{ "recipe": "hold", "durationFrames": 24, "from": "initial" }],
    }));
    two["shots"][1]["cameraClipId"] = json!("other");
    two["shots"][1]["cameraInFrame"] = json!(0);
    let across = import(two);
    assert_eq!(
        codes(&across.edit_join_issues()),
        ["CAMERA_SEGMENT_RESET", "CONTINUOUS_CROSS_CLIP_UNSUPPORTED"]
    );
    assert_eq!(across.temporal_join_issues(), vec![]);
}

#[test]
fn stacked_zooms_that_leave_the_field_of_view_range_are_a_temporal_finding() {
    // Each track alone stays in range, so the import accepts it; together they do not.
    let scene = variant("stacked-fov-out-of-range");
    let findings = scene.temporal_issues();
    let fov = findings
        .iter()
        .find(|issue| issue.code == "FOV_OUT_OF_RANGE")
        .unwrap_or_else(|| panic!("{findings:?}"));
    assert_eq!(fov.path, "/cameraClips/0");
    assert!(
        fov.message.contains("with its tracks together"),
        "{}",
        fov.message
    );
    assert!(
        !scene
            .temporal_join_issues()
            .iter()
            .any(|issue| issue.code == "FOV_OUT_OF_RANGE"),
        "the field-of-view sweep belongs to temporal_issues, not to the joins"
    );
}

#[test]
fn a_clip_too_long_to_sweep_is_reported_not_skipped() {
    let mut value = layered();
    value["cameraClips"][0]["layers"] = json!([{ "segments": [{
        "recipe": "hold", "durationFrames": LAYER_FOV_SWEEP_MAX_FRAMES + 1, "from": "previousEnd",
    }]}]);
    let findings = import(value).temporal_issues();
    assert!(
        findings
            .iter()
            .any(|issue| issue.code == "TEMPORAL_CHECK_BUDGET_EXCEEDED"),
        "{findings:?}"
    );
}

// ---- seek: any order, any repetition ------------------------------------------------------

fn bits(camera: &CameraState) -> [u64; 11] {
    [
        camera.position.x,
        camera.position.y,
        camera.position.z,
        camera.target.x,
        camera.target.y,
        camera.target.z,
        camera.fov_deg,
        camera.velocity.x,
        camera.velocity.y,
        camera.velocity.z,
        camera.fov_velocity_deg_per_frame,
    ]
    .map(f64::to_bits)
}

/// Deterministic shuffle (no dependency): a Fisher–Yates driven by an xorshift.
fn shuffled(mut items: Vec<i64>, mut seed: u64) -> Vec<i64> {
    for i in (1..items.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        items.swap(i, (seed % (i as u64 + 1)) as usize);
    }
    items
}

#[test]
fn seeking_out_of_order_or_twice_gives_the_same_bits() {
    let mut subjects = vec![("layered".to_string(), layered())];
    subjects.extend(corpus_valid().into_iter().map(|(id, raw, _)| (id, raw)));
    for (id, raw) in subjects {
        let scene = import(raw);
        let bundle = scene.bundle();
        let duration = bundle.edit_duration_frames as i64;
        let sequential: Vec<[u64; 11]> = (0..duration)
            .map(|frame| bits(&bundle.evaluate_camera(frame).unwrap().camera))
            .collect();
        for seed in [1_u64, 0x9E37_79B9, 0xDEAD_BEEF] {
            for frame in shuffled((0..duration).collect(), seed) {
                let camera = bundle.evaluate_camera(frame).unwrap().camera;
                assert_eq!(
                    bits(&camera),
                    sequential[frame as usize],
                    "{id} frame {frame}"
                );
                // Asked again right away: same bits.
                let again = bundle.evaluate_camera(frame).unwrap().camera;
                assert_eq!(bits(&again), bits(&camera), "{id} frame {frame} twice");
            }
        }
        // A timeline built once gives the same bits as a layout per call.
        for shot in &bundle.shots {
            let timeline = bundle.camera_timeline(&shot.camera_clip_id).unwrap();
            for frame in shuffled((0..=timeline.frames() as i64 + 3).collect(), 7) {
                assert_eq!(
                    bits(&timeline.state_at(frame).unwrap()),
                    bits(
                        &bundle
                            .camera_at_clip_frame(&shot.camera_clip_id, frame)
                            .unwrap()
                    ),
                    "{id} clip frame {frame}"
                );
            }
        }
    }
}

// ---- time: rational frame rates, boundaries -------------------------------------------------

/// The layered fixture re-declared at another frame rate.
fn layered_at(num: u32, den: u32) -> DirectedScene {
    let mut value = layered();
    for pointer in ["/fps", "/renderProfile/fps"] {
        let fps = value.pointer_mut(pointer).unwrap();
        fps["numerator"] = json!(num);
        fps["denominator"] = json!(den);
    }
    import(value)
}

#[test]
fn mapping_is_exact_at_ntsc_and_integer_rates() {
    for (num, den) in [
        (30000, 1001),
        (24000, 1001),
        (60000, 1001),
        (30, 1),
        (25, 1),
    ] {
        let scene = layered_at(num, den);
        let bundle = scene.bundle();
        let rate = Rate::new(num, den).unwrap();
        for frame in 0..bundle.edit_duration_frames as i64 {
            let mapping = bundle.resolve_frame_mapping(frame).unwrap();
            // Exactly frame * den / num seconds, never a float.
            let exact = Ratio::new(frame * i64::from(den), i64::from(num)).unwrap();
            assert_eq!(mapping.edit_time, exact, "{num}/{den} frame {frame}");
            assert_eq!(mapping.edit_time, rate.frame_to_time(frame));
            // The shot only shifts frames; its times shift by the same exact amount.
            let shift = (mapping.shot.start_frame as i64
                - mapping.shot.instance_start_frame as i64)
                * i64::from(den);
            assert_eq!(
                mapping.edit_time.sub(mapping.instance_time).unwrap(),
                Ratio::new(shift, i64::from(num)).unwrap()
            );
            // A frame's start belongs to that frame; one tick earlier belongs to the previous.
            assert_eq!(bundle.frame_at_time(mapping.edit_time).unwrap(), frame);
            let before = mapping
                .edit_time
                .sub(Ratio::new(1, 1_000_000_000).unwrap())
                .unwrap();
            assert_eq!(bundle.frame_at_time(before).unwrap(), frame - 1);
            let later = mapping
                .edit_time
                .add(Ratio::new(i64::from(den), i64::from(num) * 2).unwrap())
                .unwrap();
            assert_eq!(
                bundle.resolve_time_mapping(later).unwrap().edit_frame as i64,
                frame,
                "half a frame in"
            );
        }
    }
}

#[test]
fn thirty_thousand_over_1001_does_not_drift_over_hours() {
    // 30000/1001 fps: frame 30000 starts at exactly 1001 s. Floats lose this within the hour.
    let scene = layered_at(30000, 1001);
    let bundle = scene.bundle();
    let rate = bundle.fps.rate().unwrap();
    assert_eq!(rate.frame_to_time(30000), Ratio::from_int(1001));
    for frame in (0..1_200_000_i64).step_by(7) {
        assert_eq!(
            bundle.frame_at_time(rate.frame_to_time(frame)).unwrap(),
            frame
        );
    }
    // A time exactly at the end of the edit is outside it, one tick before is the last frame.
    let end = bundle
        .edit_time_of_frame(bundle.edit_duration_frames)
        .unwrap();
    assert_eq!(
        bundle.resolve_time_mapping(end).unwrap_err(),
        TemporalError::EditFrameOutOfRange {
            frame: 48,
            duration_frames: 48
        }
    );
    let last = end.sub(Ratio::new(1, 1_000_000_000).unwrap()).unwrap();
    assert_eq!(bundle.resolve_time_mapping(last).unwrap().edit_frame, 47);
    // Before zero is out of range, not frame 0.
    let negative = Ratio::new(-1, 1_000_000_000).unwrap();
    assert_eq!(bundle.frame_at_time(negative).unwrap(), -1);
    assert!(bundle.resolve_time_mapping(negative).is_err());
}

#[test]
fn shot_boundaries_and_out_of_range_frames() {
    let scene = import(layered());
    let bundle = scene.bundle();
    let last_of_first = bundle.resolve_frame_mapping(23).unwrap();
    let first_of_second = bundle.resolve_frame_mapping(24).unwrap();
    assert_eq!(last_of_first.shot.id, "s1");
    assert_eq!(first_of_second.shot.id, "s2");
    assert_eq!(
        (
            last_of_first.instance_frame,
            last_of_first.camera_clip_frame
        ),
        (23, 23)
    );
    assert_eq!(
        (
            first_of_second.instance_frame,
            first_of_second.camera_clip_frame
        ),
        (24, 24)
    );
    assert_eq!(bundle.resolve_frame_mapping(47).unwrap().shot.id, "s2");
    for outside in [-1, 48, 49, i64::MIN, i64::MAX] {
        let error = bundle.resolve_frame_mapping(outside).unwrap_err();
        assert_eq!(
            error,
            TemporalError::EditFrameOutOfRange {
                frame: outside,
                duration_frames: 48
            }
        );
        assert_eq!(bundle.evaluate_camera(outside).unwrap_err(), error);
    }
    // A clip frame before 0 is an error; past the clip's last frame the camera holds still.
    assert!(matches!(
        bundle.camera_at_clip_frame("wide", -1),
        Err(TemporalError::CameraFrameBeforeStart { frame: -1, .. })
    ));
    let end = bundle.camera_clip_frames("wide").unwrap();
    assert_eq!(end, 48);
    let rest = bundle
        .camera_at_clip_frame("wide", end as i64 + 1000)
        .unwrap();
    assert_eq!(rest.velocity, Vec3::ZERO);
    assert_eq!(rest.fov_velocity_deg_per_frame, 0.0);
    assert!(matches!(
        bundle.camera_at_clip_frame("missing", 0),
        Err(TemporalError::Unresolved { .. })
    ));
}

#[test]
fn the_corpus_cut_with_offsets_maps_instance_and_clip_frames_independently() {
    let (_, raw, expected) = corpus_valid()
        .into_iter()
        .find(|(id, _, _)| id == "valid-cut-instance-offsets")
        .unwrap();
    let scene = import(raw);
    let bundle = scene.bundle();
    let frame = |n: i64| bundle.resolve_frame_mapping(n).unwrap();
    // Neither offset equals the edit frame in at least one shot: the three frames are independent.
    let independent = bundle.shots.iter().any(|shot| {
        shot.start_frame != shot.instance_start_frame
            && shot.instance_start_frame != shot.camera_in_frame
    });
    assert!(independent, "the case is meant to exercise offsets");
    for probe in expected["evaluation"]["editFrames"].as_array().unwrap() {
        if probe["ok"] == true {
            let n = probe["editFrame"].as_i64().unwrap();
            let state = &probe["state"];
            assert_eq!(
                frame(n).instance_frame,
                state["instanceFrame"].as_u64().unwrap()
            );
            assert_eq!(
                frame(n).camera_clip_frame,
                state["cameraClipFrame"].as_u64().unwrap()
            );
        }
    }
}

// ---- the honest boundary: what a frame evaluation does not cover ------------------------------

#[test]
fn a_scene_with_weather_or_lights_reports_them_unevaluated_instead_of_faking_them() {
    let scene = import(layered());
    let bundle = scene.bundle();
    // The camera of this very frame is complete and available on its own.
    assert!(bundle.evaluate_camera(10).is_ok());

    // The layered scene declares `lighting` and `weather`; the channels themselves are evaluated.
    let whole = bundle.evaluate_frame(10).unwrap();
    assert_eq!(
        whole.node_animation,
        NodeAnimation::Animated {
            track_ids: vec!["walk".to_string()],
            channels: 1,
        }
    );
    assert_eq!(whole.unevaluated, vec!["lighting", "weather"]);
    assert!(!whole.is_complete());
    let error = whole.clone().require_complete().unwrap_err();
    let expected = Unsupported::DeferredSections {
        scene_definition_id: "street".to_string(),
        sections: vec!["lighting".to_string(), "weather".to_string()],
    };
    assert_eq!(error, TemporalError::Unsupported(expected));
    assert!(error.to_string().contains("lighting, weather"));
    let command = CommandError::from(error);
    assert_eq!(command.code, "unsupported");
    assert_eq!(
        command.details.as_ref().unwrap()["sections"],
        json!(["lighting", "weather"])
    );

    // Without those sections and without channels the frame is whole and says so.
    let mut value = layered();
    value["performanceTracks"] = json!([]);
    value["sceneInstances"][0]["performanceTrackIds"] = json!([]);
    value["sceneDefinitions"][0]
        .as_object_mut()
        .unwrap()
        .retain(|key, _| key != "weather" && key != "lighting");
    let plain = import(value);
    let frame = plain.bundle().evaluate_frame(10).unwrap();
    assert_eq!(frame.node_animation, NodeAnimation::Static);
    assert!(frame.is_complete());
    assert!(frame.require_complete().is_ok());

    // A track the instance does not list does not apply (the origin's selection).
    let mut unlisted = layered();
    unlisted["sceneInstances"][0]["performanceTrackIds"] = json!([]);
    let unlisted = import(unlisted);
    assert_eq!(
        unlisted.bundle().evaluate_frame(10).unwrap().node_animation,
        NodeAnimation::Static
    );
}

#[test]
fn temporal_errors_travel_as_command_errors() {
    let scene = import(layered());
    let error = scene.bundle().evaluate_camera(99).unwrap_err();
    let command = CommandError::from(error);
    assert_eq!(command.code, "frame_out_of_range");
    assert_eq!(command.message, "editFrame 99 out of range [0, 48)");
}

#[test]
fn evaluating_never_changes_the_scene() {
    let scene = import(layered());
    let before = scene.clone();
    let _ = scene.bundle().evaluate_frame(5);
    let _ = scene.bundle().evaluate_camera(-3);
    let _ = scene.temporal_issues();
    assert_eq!(scene, before);
    assert_eq!(scene.to_json_string(), before.to_json_string());
}
