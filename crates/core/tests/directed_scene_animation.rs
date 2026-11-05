//! BB03 (nodes): performance channels, node transforms and world matrices of a directed scene,
//! through the public library API only (no GPUI, no window, no runtime, no socket, no CLI).
//!
//! Two kinds of evidence live here and are kept apart:
//!
//! - **Equivalence with the origin.** The expected values come from the origin's own functions
//!   (`renderBundleSchema`, `evaluateEditFrame`, `computeWorldMatrices`), executed through the
//!   migration oracle: (1) every edit frame the oracle probed on the 4 valid corpus bundles
//!   (`docs/migration/fixtures`), the `valid-graph-sockets-performance` case being the one with
//!   channels; (2) the synthetic variants of `directed_scene_animation_fixtures/` (cycle/event/
//!   static edges, combinations, the instance allow-list, writer conflicts, two instances with
//!   offsets and out-of-order seeks, gimbal-lock bases, unknown fields, 20 refused payloads).
//! - **Behaviour of this evaluator**, on bundles written for these tests: analytic values, seek
//!   order, instance-local frames, typed errors, the unevaluated list, serialization. They prove
//!   this module, not equivalence.
//!
//! # Numeric tolerances (fixed before the Rust output was compared with the origin)
//!
//! | Quantity | Rule |
//! |---|---|
//! | frame numbers, ids, property names, counts, `renderOrder`, order of nodes and overrides | exact |
//! | the origin's error messages (range, `Multiple performance channels write ...`) | exact text |
//! | local position/rotation (Euler degrees)/scale, override values, world matrix elements, camera | `abs <= 1e-9 + 1e-12 * abs(expected)` |
//!
//! Why: a node state is a chain of a few dozen double operations plus `sin`/`cos`/`asin`/`atan2`,
//! which the origin gets from V8 (fdlibm) and Rust from the host libm; they can differ by an ulp.
//! Values are O(1..100) (degrees, scene units, matrix elements), so the honest error is below
//! 1e-12; 1e-9 leaves three orders of margin and sits six orders below the smallest effect of a
//! wrong phase, progress, merge or rotation order (>= 1e-3), so it cannot hide one (the two
//! negative controls below fail by that much). It is the rule `directed_scene_temporal.rs` uses
//! for the camera; the rule of the static matrices (`directed_scene_oracle.rs`) is `1e-10 + 1e-12`.
//! Path rule for refused payloads: the origin's issue paths and the importer's JSON Pointers name
//! the same fields (a missing field is reported on its parent, naming the field, as everywhere in
//! the importer), and the number of issues is equal: nothing is skipped and nothing is invented.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tessel_editor_core::CommandError;
use tessel_editor_core::directed_scene::{
    ChannelProperty, DirectedScene, FrameEvaluation, ImportError, Issue, MISSING_FIELD, Mat4,
    NodeAnimation, TemporalError, UNKNOWN_FIELD, Unsupported, Vec3,
};

const BASE: &str = include_str!(
    "../../../docs/migration/fixtures/bundles/valid-graph-sockets-performance.bundle.json"
);
const PINNED_COMMIT: &str = "d6adac15c61f4ef582a7ec259397b2e025d1f059";

const ABSOLUTE: f64 = 1e-9;
const RELATIVE: f64 = 1e-12;

fn oracle_fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/migration/fixtures")
}

fn animation_fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/directed_scene_animation_fixtures")
}

fn read_json(path: impl AsRef<Path>) -> Value {
    let path = path.as_ref();
    serde_json::from_str(&std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path:?}: {e}")))
        .unwrap()
}

fn import(raw: Value) -> DirectedScene {
    DirectedScene::import_value(raw).unwrap_or_else(|error| panic!("import failed: {error:?}"))
}

fn base() -> Value {
    serde_json::from_str(BASE).unwrap()
}

// ---- SHA-256, so the test can check the fixtures are the generated ones (no new dependency) -----

fn sha256_hex(bytes: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut message = bytes.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((bytes.len() as u64) * 8).to_be_bytes());
    for block in message.chunks(64) {
        let mut w = [0_u32; 64];
        for (i, word) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut v = state;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let choice = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(choice)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let majority = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(majority);
            v = [
                t1.wrapping_add(t2),
                v[0],
                v[1],
                v[2],
                v[3].wrapping_add(t1),
                v[4],
                v[5],
                v[6],
            ];
        }
        for (slot, value) in state.iter_mut().zip(v) {
            *slot = slot.wrapping_add(value);
        }
    }
    state.iter().map(|word| format!("{word:08x}")).collect()
}

#[test]
fn the_sha256_used_to_check_fixtures_is_the_real_one() {
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    // Two blocks (the padding crosses a block boundary).
    assert_eq!(
        sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
}

// ---- comparison with the origin ----------------------------------------------------------------

/// Counts and the largest error seen against the origin, so the report can state it (printed with
/// `--nocapture`). Every method returns the first divergence as text instead of panicking, so the
/// negative controls can assert that a wrong evaluator is caught by the very same comparison.
#[derive(Default)]
struct Compare {
    numbers: usize,
    frames: usize,
    errors: usize,
    worst: f64,
    at: String,
}

impl Compare {
    fn close(&mut self, label: &str, actual: f64, expected: f64) -> Result<(), String> {
        let error = (actual - expected).abs();
        // A NaN error (either side non-finite) must fail too, hence the explicit check.
        if error.is_nan() || error > ABSOLUTE + RELATIVE * expected.abs() {
            return Err(format!("{label}: {actual} != {expected} (error {error:e})"));
        }
        self.numbers += 1;
        if error > self.worst {
            self.worst = error;
            self.at = label.to_string();
        }
        Ok(())
    }

    fn number(value: &Value, label: &str) -> Result<f64, String> {
        value
            .as_f64()
            .ok_or_else(|| format!("{label}: expected a number, got {value}"))
    }

    fn vec3(&mut self, label: &str, actual: Vec3, expected: &Value) -> Result<(), String> {
        for (axis, actual) in [("x", actual.x), ("y", actual.y), ("z", actual.z)] {
            let label = format!("{label}.{axis}");
            self.close(&label, actual, Self::number(&expected[axis], &label)?)?;
        }
        Ok(())
    }

    fn matrix(&mut self, label: &str, actual: &Mat4, expected: &Value) -> Result<(), String> {
        let expected = expected
            .as_array()
            .filter(|elements| elements.len() == 16)
            .ok_or_else(|| format!("{label}: expected 16 elements"))?;
        for (i, (actual, expected)) in actual.0.iter().zip(expected).enumerate() {
            let label = format!("{label}[{i}]");
            self.close(&label, *actual, Self::number(expected, &label)?)?;
        }
        Ok(())
    }

    /// One evaluated edit frame against the origin's `{ state, worldMatrices }`.
    fn frame(
        &mut self,
        label: &str,
        actual: &FrameEvaluation<'_>,
        expected: &Value,
    ) -> Result<(), String> {
        let state = &expected["state"];
        let same = |what: &str, actual: String, expected: &Value| {
            let expected = expected.to_string().trim_matches('"').to_string();
            if actual == expected {
                return Ok(());
            }
            Err(format!("{label}.{what}: {actual} != {expected}"))
        };
        same(
            "instanceId",
            actual.mapping.instance.id.clone(),
            &state["instanceId"],
        )?;
        same(
            "instanceFrame",
            actual.mapping.instance_frame.to_string(),
            &state["instanceFrame"],
        )?;
        same(
            "cameraClipFrame",
            actual.mapping.camera_clip_frame.to_string(),
            &state["cameraClipFrame"],
        )?;
        self.vec3(
            &format!("{label}.camera.position"),
            actual.camera.position,
            &state["camera"]["position"],
        )?;
        self.vec3(
            &format!("{label}.camera.target"),
            actual.camera.target,
            &state["camera"]["target"],
        )?;

        let expected_nodes = state["nodes"].as_array().ok_or("nodes is not an array")?;
        if actual.nodes.nodes().len() != expected_nodes.len() {
            return Err(format!("{label}: node count differs"));
        }
        for (node, expected) in actual.nodes.nodes().iter().zip(expected_nodes) {
            let id = expected["nodeId"].as_str().ok_or("nodeId")?;
            same(
                &format!("{id}.nodeId"),
                node.node_id.clone(),
                &expected["nodeId"],
            )?;
            let at = format!("{label} {id}");
            same(
                &format!("{id}.renderOrder"),
                node.render_order.to_string(),
                &expected["renderOrder"],
            )?;
            self.vec3(
                &format!("{at}.localPosition"),
                node.local_position,
                &expected["localPosition"],
            )?;
            self.vec3(
                &format!("{at}.localRotation"),
                node.local_rotation_deg,
                &expected["localRotation"],
            )?;
            self.vec3(
                &format!("{at}.localScale"),
                node.local_scale,
                &expected["localScale"],
            )?;
            self.matrix(
                &format!("{at}.worldMatrix"),
                &node.world_matrix,
                &expected_world(expected_root(state, expected), id),
            )?;
        }

        let expected_overrides = state["performanceOverrides"]
            .as_array()
            .ok_or("performanceOverrides is not an array")?;
        let overrides = actual.nodes.overrides();
        if overrides.len() != expected_overrides.len() {
            return Err(format!("{label}: override count differs"));
        }
        for (i, (entry, expected)) in overrides.iter().zip(expected_overrides).enumerate() {
            same(
                &format!("override {i}.nodeId"),
                entry.node_id.clone(),
                &expected["nodeId"],
            )?;
            same(
                &format!("override {i}.property"),
                entry.property.as_str().to_string(),
                &expected["property"],
            )?;
            self.vec3(
                &format!("{label} override {i}.value"),
                entry.value,
                &expected["value"],
            )?;
        }
        // The node animation status must say what the origin did.
        let animated = matches!(actual.node_animation, NodeAnimation::Animated { .. });
        if animated != !expected_overrides.is_empty() {
            return Err(format!(
                "{label}: node_animation disagrees with the overrides"
            ));
        }
        self.frames += 1;
        Ok(())
    }

    /// Every probed frame of one origin evaluation, in the order it was recorded.
    fn evaluation(
        &mut self,
        name: &str,
        scene: &DirectedScene,
        frames: &[Value],
    ) -> Result<(), String> {
        for frame in frames {
            let edit = frame["editFrame"].as_i64().ok_or("editFrame")?;
            let label = format!("{name} edit {edit}");
            let result = scene.bundle().evaluate_frame(edit);
            if frame["ok"] == false {
                let error = result
                    .err()
                    .ok_or(format!("{label}: the origin fails, Rust succeeds"))?;
                let expected = frame["error"].as_str().ok_or("error text")?;
                if error.to_string() != expected {
                    return Err(format!("{label}: `{error}` != `{expected}`"));
                }
                self.errors += 1;
                continue;
            }
            let evaluated =
                result.map_err(|error| format!("{label}: Rust fails with `{error}`"))?;
            // The frame carries `worldMatrices` next to `state`; fold it in so one value describes it.
            let mut expected = frame.clone();
            expected["state"]["worldMatrices"] = frame["worldMatrices"].clone();
            self.frame(&label, &evaluated, &expected)?;
        }
        Ok(())
    }
}

fn expected_root<'a>(state: &'a Value, _node: &Value) -> &'a Value {
    &state["worldMatrices"]
}

fn expected_world(matrices: &Value, id: &str) -> Value {
    matrices[id].clone()
}

fn valid_corpus() -> Vec<(String, Value, Value)> {
    let root = oracle_fixtures();
    read_json(root.join("cases.json"))["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case["kind"] == "valid")
        .map(|case| {
            (
                case["id"].as_str().unwrap().to_string(),
                read_json(root.join(case["bundle"].as_str().unwrap())),
                read_json(root.join(case["expected"].as_str().unwrap())),
            )
        })
        .collect()
}

fn origin() -> Value {
    read_json(animation_fixtures().join("origin.json"))
}

fn variants_of(origin: &Value, kind: &str) -> Vec<(String, Value, Value)> {
    origin["variants"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(_, entry)| entry["kind"] == kind)
        .map(|(name, entry)| {
            let raw = read_json(animation_fixtures().join(format!("bundles/{name}.bundle.json")));
            (name.clone(), raw, entry.clone())
        })
        .collect()
}

// ---- provenance of the expectations --------------------------------------------------------------

#[test]
fn the_expectations_are_pinned_to_the_origin_commit_and_match_the_files_on_disk() {
    let origin = origin();
    assert_eq!(origin["originCommit"], PINNED_COMMIT);
    assert_eq!(origin["pinnedCommit"], PINNED_COMMIT);
    let sources = origin["sourceFiles"].as_array().unwrap();
    assert_eq!(origin["executedSources"], sources.len());
    assert!(
        sources
            .iter()
            .any(|f| f["path"] == "src/performance/resolve-performance.ts"
                && f["role"] == "loaded-transitively"),
        "the channel evaluator must be among the executed sources"
    );
    assert!(
        sources
            .iter()
            .any(|f| f["path"] == "src/scene/scene-graph.ts"),
        "the world matrices must come from the executed scene graph"
    );
    for source in sources {
        assert_eq!(source["sha256"].as_str().unwrap().len(), 64);
        assert_eq!(source["gitBlob"].as_str().unwrap().len(), 40);
    }
    assert_eq!(origin["toolVersions"]["three"], "0.187.0");

    // Every bundle on disk is listed with the hash it has, and nothing listed is missing.
    let listed = origin["bundleHashes"].as_object().unwrap();
    let mut on_disk: Vec<String> = std::fs::read_dir(animation_fixtures().join("bundles"))
        .unwrap()
        .map(|entry| format!("bundles/{}", entry.unwrap().file_name().to_string_lossy()))
        .collect();
    on_disk.sort();
    let mut names: Vec<&String> = listed.keys().collect();
    names.sort();
    assert_eq!(on_disk, names.into_iter().cloned().collect::<Vec<_>>());
    for (file, hash) in listed {
        let bytes = std::fs::read(animation_fixtures().join(file)).unwrap();
        assert_eq!(
            sha256_hex(&bytes),
            hash.as_str().unwrap(),
            "{file} was edited"
        );
    }
    // Generator and base case are the ones that produced the expectations.
    let generator = std::fs::read(animation_fixtures().join("generate.mjs")).unwrap();
    assert_eq!(sha256_hex(&generator), origin["generatorSha256"]);
    assert_eq!(sha256_hex(BASE.as_bytes()), origin["baseBundleSha256"]);
    // One expectation per bundle.
    assert_eq!(
        origin["variants"].as_object().unwrap().len(),
        listed.len(),
        "a variant has no bundle or the other way round"
    );
}

// ---- equivalence: the oracle corpus ---------------------------------------------------------------

#[test]
fn every_probed_frame_of_the_oracle_corpus_matches_the_origin_including_nodes_and_world_matrices() {
    let mut compare = Compare::default();
    let mut with_channels = 0;
    for (id, raw, expected) in valid_corpus() {
        let animated = raw["performanceTracks"]
            .as_array()
            .is_some_and(|tracks| !tracks.is_empty());
        let scene = import(raw);
        let frames = expected["evaluation"]["editFrames"].as_array().unwrap();
        compare
            .evaluation(&id, &scene, frames)
            .unwrap_or_else(|divergence| panic!("{divergence}"));
        if animated {
            with_channels += 1;
            assert_eq!(id, "valid-graph-sockets-performance");
            // The case the brief names: all 12 probed frames, 2 of them out of range.
            assert_eq!(frames.len(), 12);
        }
    }
    eprintln!(
        "corpus: {} frames + {} errors, {} numbers, worst error {:e} at {}",
        compare.frames, compare.errors, compare.numbers, compare.worst, compare.at
    );
    assert_eq!(with_channels, 1);
    assert!(
        compare.frames >= 30 && compare.errors >= 8,
        "corpus coverage decreased"
    );
    assert!(
        compare.numbers >= 3_000,
        "coverage decreased: {}",
        compare.numbers
    );
}

#[test]
fn the_oracle_case_with_channels_is_animated_in_the_probed_frames() {
    // The comparison above is only meaningful if the case really moves nodes.
    let raw = valid_corpus()
        .into_iter()
        .find(|(id, ..)| id == "valid-graph-sockets-performance")
        .unwrap()
        .1;
    let scene = import(raw);
    let still = scene.bundle().evaluate_frame(0).unwrap();
    let moving = scene.bundle().evaluate_frame(18).unwrap();
    assert_eq!(
        moving.node_animation,
        NodeAnimation::Animated {
            track_ids: vec!["perf-1".to_string()],
            channels: 3
        }
    );
    // The arm swings, the stage lifts, the pen is scaled and its world follows the arm's socket.
    for node in ["arm", "stage", "pen"] {
        let a = still.nodes.world_matrix(node).unwrap();
        let b = moving.nodes.world_matrix(node).unwrap();
        assert!(a.max_abs_diff(b) > 1e-3, "{node} did not move");
    }
    assert_eq!(
        moving.nodes.node("pen").unwrap().local_scale,
        Vec3 {
            x: 1.5,
            y: 1.5,
            z: 1.5
        }
    );
}

// ---- equivalence: synthetic variants --------------------------------------------------------------

#[test]
fn synthetic_variants_match_the_origin_in_every_frame_in_the_recorded_order() {
    let origin = origin();
    let mut compare = Compare::default();
    let mut variants = 0;
    for (name, raw, entry) in variants_of(&origin, "valid") {
        let scene = import(raw);
        let frames = entry["editFrames"].as_array().unwrap();
        compare
            .evaluation(&name, &scene, frames)
            .unwrap_or_else(|divergence| panic!("{divergence}"));
        variants += 1;
    }
    eprintln!(
        "variants: {variants}; {} frames + {} errors, {} numbers, worst error {:e} at {}",
        compare.frames, compare.errors, compare.numbers, compare.worst, compare.at
    );
    assert_eq!(variants, 11);
    assert!(
        compare.frames >= 120 && compare.errors >= 19,
        "coverage decreased"
    );
    assert!(
        compare.numbers >= 17_000,
        "coverage decreased: {}",
        compare.numbers
    );
}

#[test]
fn the_importer_accepts_every_valid_variant_and_refuses_what_the_origin_refuses() {
    let origin = origin();
    for (name, raw, entry) in variants_of(&origin, "valid") {
        assert_eq!(entry["schemaAccepted"], true, "{name}");
        assert!(DirectedScene::import_value(raw).is_ok(), "{name}");
    }
    for (name, raw, entry) in variants_of(&origin, "invalid") {
        assert_eq!(entry["schemaAccepted"], false, "{name}");
        assert!(
            matches!(
                DirectedScene::import_value(raw),
                Err(ImportError::Invalid { .. })
            ),
            "{name}: the origin refuses it and so must the importer"
        );
    }
}

fn pointer_of(dotted: &str) -> String {
    format!("/{}", dotted.replace('.', "/"))
}

fn reports(issues: &[Issue], pointer: &str) -> bool {
    let (parent, field) = pointer.rsplit_once('/').unwrap();
    issues.iter().any(|issue| {
        issue.path == pointer
            || (issue.code == MISSING_FIELD
                && issue.path == parent
                && issue.message.contains(&format!("`{field}`")))
    })
}

#[test]
fn refused_payloads_report_the_origin_paths_and_every_problem() {
    let origin = origin();
    let mut checked = 0;
    for (name, raw, entry) in variants_of(&origin, "invalid") {
        let Err(ImportError::Invalid { issues }) = DirectedScene::import_value(raw) else {
            panic!("{name}: not refused");
        };
        let expected = entry["schemaIssues"].as_array().unwrap();
        assert_eq!(
            issues.len(),
            expected.len(),
            "{name}: {issues:?} vs the origin's {expected:?}"
        );
        for issue in expected {
            let pointer = pointer_of(issue["path"].as_str().unwrap());
            assert!(
                reports(&issues, &pointer),
                "{name}: nothing at {pointer}: {issues:?}"
            );
        }
        for issue in &issues {
            assert!(
                ["MISSING_FIELD", "INVALID_TYPE", "INVALID_VALUE"].contains(&issue.code),
                "{name}: {issue}"
            );
        }
        checked += 1;
    }
    assert_eq!(checked, 20);
}

#[test]
fn a_bad_channel_is_never_dropped_the_whole_import_fails() {
    // Not "channel ignored, scene loaded": one broken channel among good ones refuses the bundle.
    let mut raw = base();
    raw["performanceTracks"][0]["channels"][1]["durationFrames"] = json!(0);
    let Err(ImportError::Invalid { issues }) = DirectedScene::import_value(raw) else {
        panic!("a channel with durationFrames 0 was accepted");
    };
    assert_eq!(issues.len(), 1);
    assert_eq!(
        issues[0].path,
        "/performanceTracks/0/channels/1/durationFrames"
    );
}

#[test]
fn unknown_fields_on_channels_are_kept_reported_and_do_not_move_anything() {
    let origin = origin();
    let (_, raw, _) = variants_of(&origin, "valid")
        .into_iter()
        .find(|(name, ..)| name == "unknown-fields-on-channels")
        .unwrap();
    let scene = import(raw.clone());
    // Round trip: the document is what was imported, extra keys included.
    assert_eq!(scene.document(), &raw);
    assert_eq!(
        serde_json::from_str::<Value>(&scene.to_json_string()).unwrap(),
        raw
    );
    let notices: Vec<&str> = scene
        .notices()
        .iter()
        .filter(|notice| notice.code == UNKNOWN_FIELD)
        .map(|notice| notice.path.as_str())
        .collect();
    for pointer in [
        "/performanceTracks/0/comment",
        "/performanceTracks/0/channels/0/label",
        "/performanceTracks/0/channels/0/delta",
        "/performanceTracks/0/channels/1/value",
        "/performanceTracks/0/channels/2/holdFinal",
    ] {
        assert!(
            notices.contains(&pointer),
            "no notice at {pointer}: {notices:?}"
        );
    }
    // Same channels without the extras: the same frames, bit for bit.
    let plain = import(base());
    for frame in [0, 6, 12, 30, 47] {
        assert_eq!(
            scene.bundle().evaluate_frame(frame).unwrap().nodes,
            plain.bundle().evaluate_frame(frame).unwrap().nodes,
            "edit {frame}"
        );
    }
}

// ---- seek: any order, any number of times ---------------------------------------------------------

#[test]
fn seeking_in_any_order_gives_the_same_bits() {
    let origin = origin();
    for (name, raw, entry) in variants_of(&origin, "valid") {
        let scene = import(raw);
        let bundle = scene.bundle();
        let frames: Vec<i64> = entry["frames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|frame| frame.as_i64().unwrap())
            .collect();
        let evaluate = |frame: i64| {
            bundle
                .evaluate_frame(frame)
                .map(|f| (f.nodes.clone(), f.camera))
        };
        let forward: Vec<_> = frames.iter().map(|&f| evaluate(f)).collect();
        // Backwards, then each frame twice in a row, then a rotated order: same answers.
        let backward: Vec<_> = frames.iter().rev().map(|&f| evaluate(f)).collect();
        let reversed: Vec<_> = backward.into_iter().rev().collect();
        assert_eq!(forward, reversed, "{name}: backwards");
        for (i, &frame) in frames.iter().enumerate() {
            assert_eq!(evaluate(frame), forward[i], "{name}: repeated edit {frame}");
        }
        let mut rotated = frames.clone();
        rotated.rotate_left(frames.len() / 2);
        for frame in rotated {
            let index = frames.iter().position(|&f| f == frame).unwrap();
            assert_eq!(
                evaluate(frame),
                forward[index],
                "{name}: rotated edit {frame}"
            );
        }
    }
}

#[test]
fn a_timeline_answers_many_instance_frames_like_the_one_shot_calls() {
    let scene = import(base());
    let bundle = scene.bundle();
    let timeline = bundle.node_timeline("inst-a").unwrap();
    assert_eq!(timeline.instance_id(), "inst-a");
    for frame in (0..48).rev().chain(0..48) {
        assert_eq!(
            timeline.state_at(frame).unwrap(),
            bundle.evaluate_instance_nodes("inst-a", frame).unwrap(),
            "instance frame {frame}"
        );
    }
}

// ---- instance-local frames, shots and offsets -----------------------------------------------------

fn two_instances() -> DirectedScene {
    let origin = origin();
    let (_, raw, _) = variants_of(&origin, "valid")
        .into_iter()
        .find(|(name, ..)| name == "two-instances-offsets-seek")
        .unwrap();
    import(raw)
}

#[test]
fn channels_run_on_the_frame_of_the_instance_not_of_the_edit() {
    let scene = two_instances();
    let bundle = scene.bundle();
    // shot-1 plays inst-a from its frame 10; shot-3 plays inst-a from its frame 40.
    let first = bundle.evaluate_frame(5).unwrap();
    assert_eq!(
        (
            first.mapping.instance.id.as_str(),
            first.mapping.instance_frame
        ),
        ("inst-a", 15)
    );
    let third = bundle.evaluate_frame(60).unwrap();
    assert_eq!(
        (
            third.mapping.instance.id.as_str(),
            third.mapping.instance_frame
        ),
        ("inst-a", 50)
    );
    assert_eq!(first.nodes.instance_frame(), 15);
    assert_eq!(third.nodes.instance_frame(), 50);

    // The same instance frame reached from two different edit frames is the same state: edit 14
    // (shot-1, instance frame 24) and an instance asked directly for frame 24.
    let by_edit = bundle.evaluate_frame(14).unwrap();
    assert_eq!(by_edit.mapping.instance_frame, 24);
    assert_eq!(
        by_edit.nodes,
        bundle.evaluate_instance_nodes("inst-a", 24).unwrap()
    );

    // inst-a at frame 15: cycle (15+3)%24 = 18 -> sin(1.5π) = -1; the event starts at 30, so 0.
    let arm = first
        .nodes
        .override_of("arm", ChannelProperty::LocalRotation)
        .unwrap();
    assert!((arm.z - (-20.0)).abs() < 1e-12 && arm.x == 0.0 && arm.y == 0.0);
    assert_eq!(
        first
            .nodes
            .override_of("stage", ChannelProperty::LocalPosition)
            .unwrap(),
        Vec3::ZERO
    );

    // The other instance of the same scene has its own channels: shot-2 starts it at frame 25.
    let second = bundle.evaluate_frame(20).unwrap();
    assert_eq!(
        (
            second.mapping.instance.id.as_str(),
            second.mapping.instance_frame
        ),
        ("inst-b", 25)
    );
    assert_eq!(
        second.nodes.override_of("pen", ChannelProperty::LocalScale),
        Some(Vec3 {
            x: 2.0,
            y: 0.5,
            z: 1.0
        })
    );
    assert_eq!(
        second.nodes.node("pen").unwrap().local_scale,
        Vec3 {
            x: 2.0,
            y: 0.5,
            z: 1.0
        }
    );
    // ... and inst-a's channels do not leak into it.
    assert_eq!(
        first.nodes.override_of("pen", ChannelProperty::LocalScale),
        None
    );
}

// ---- analytic behaviour of the channels -----------------------------------------------------------

/// The oracle case with its channels replaced.
fn with_channels(channels: Value) -> DirectedScene {
    let mut raw = base();
    raw["performanceTracks"][0]["channels"] = channels;
    import(raw)
}

fn overrides_at(scene: &DirectedScene, frame: u64, node: &str, property: ChannelProperty) -> Vec3 {
    scene
        .bundle()
        .evaluate_instance_nodes("inst-a", frame)
        .unwrap()
        .override_of(node, property)
        .unwrap()
}

#[test]
fn a_cycle_is_a_sine_of_the_phase_and_wraps_with_its_offset() {
    let scene = with_channels(json!([{
        "kind": "cycle", "channelId": "c", "nodeId": "arm", "property": "localRotation",
        "periodFrames": 8, "amplitude": 10, "axis": { "x": 1, "y": 0, "z": -2 }, "phaseOffsetFrames": 2
    }]));
    for frame in 0..48_u64 {
        let phase = ((frame + 2) % 8) as f64;
        let angle = (phase / 8.0 * std::f64::consts::TAU).sin() * 10.0;
        let value = overrides_at(&scene, frame, "arm", ChannelProperty::LocalRotation);
        assert!((value.x - angle).abs() < 1e-12, "x at {frame}");
        assert_eq!(value.y, 0.0);
        assert!((value.z - (-2.0 * angle)).abs() < 1e-12, "z at {frame}");
        // Periodic to the bit: the same phase is the same computation.
        if frame + 8 < 48 {
            assert_eq!(
                value,
                overrides_at(&scene, frame + 8, "arm", ChannelProperty::LocalRotation)
            );
        }
    }
    // Zero crossings are exact at phase 0 and at half a period.
    assert_eq!(
        overrides_at(&scene, 6, "arm", ChannelProperty::LocalRotation).x,
        0.0
    );
    assert!(
        overrides_at(&scene, 2, "arm", ChannelProperty::LocalRotation)
            .x
            .abs()
            < 1e-9
    );
}

#[test]
fn an_event_goes_from_nothing_to_its_delta_and_holds_it() {
    let scene = with_channels(json!([{
        "kind": "event", "channelId": "e", "nodeId": "stage", "property": "localPosition",
        "startInstanceFrame": 10, "durationFrames": 4, "delta": { "x": 4, "y": -8, "z": 2 },
        "holdFinal": true
    }]));
    let at = |frame| overrides_at(&scene, frame, "stage", ChannelProperty::LocalPosition);
    assert_eq!(
        at(0),
        Vec3 {
            x: 0.0,
            y: -0.0,
            z: 0.0
        }
    );
    assert_eq!(at(9).x, 0.0, "before the start");
    assert_eq!(at(10).x, 0.0, "the first frame is progress 0");
    assert_eq!(
        at(11),
        Vec3 {
            x: 1.0,
            y: -2.0,
            z: 0.5
        }
    );
    assert_eq!(at(12).x, 2.0);
    assert_eq!(
        at(13),
        Vec3 {
            x: 3.0,
            y: -6.0,
            z: 1.5
        }
    );
    assert_eq!(
        at(14),
        Vec3 {
            x: 4.0,
            y: -8.0,
            z: 2.0
        },
        "the end frame holds the full delta"
    );
    assert_eq!(at(47), at(14), "and keeps holding it");
}

#[test]
fn rotation_and_position_add_to_the_authored_transform_and_scale_replaces_it() {
    let scene = with_channels(json!([
        { "kind": "static", "channelId": "r", "nodeId": "arm", "property": "localRotation", "value": { "x": 0, "y": 0, "z": 10 } },
        { "kind": "static", "channelId": "p", "nodeId": "stage", "property": "localPosition", "value": { "x": 1, "y": 1, "z": 1 } },
        { "kind": "static", "channelId": "s", "nodeId": "stage", "property": "localScale", "value": { "x": 3, "y": 3, "z": 3 } }
    ]));
    let frame = scene.bundle().evaluate_instance_nodes("inst-a", 0).unwrap();
    // `stage` is authored at (1, 0, -1) with scale 2: position adds, scale is replaced (not 6).
    let stage = frame.node("stage").unwrap();
    assert_eq!(
        stage.local_position,
        Vec3 {
            x: 2.0,
            y: 1.0,
            z: 0.0
        }
    );
    assert_eq!(
        stage.local_scale,
        Vec3 {
            x: 3.0,
            y: 3.0,
            z: 3.0
        }
    );
    // `arm` is authored at 30 degrees about z: the offset adds to the Euler angle.
    let arm = frame.node("arm").unwrap();
    assert!((arm.local_rotation_deg.z - 40.0).abs() < 1e-9);
    assert!(arm.local_rotation_deg.x.abs() < 1e-9 && arm.local_rotation_deg.y.abs() < 1e-9);
    // Untouched nodes keep their authored state.
    assert_eq!(
        frame.node("backdrop").unwrap().local_position,
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: -3.0
        }
    );
    assert_eq!(frame.node("pen").unwrap().render_order, 2);
    assert_eq!(
        frame.node("root").unwrap().render_order,
        0,
        "an absent renderOrder is 0"
    );
    assert!(frame.node("nobody").is_none());
}

#[test]
fn the_hierarchy_and_the_sockets_follow_the_animated_parents() {
    let scene = import(base());
    let graph = scene.bundle().scene("scene-rig").unwrap();
    let frame = scene
        .bundle()
        .evaluate_instance_nodes("inst-a", 18)
        .unwrap();

    // The animated world is the chain of the animated locals, not the static graph's matrices.
    let local = |id: &str| frame.node(id).unwrap().local_matrix;
    let world = |id: &str| *frame.world_matrix(id).unwrap();
    assert_eq!(world("root"), local("root"));
    assert_eq!(world("stage"), world("root").mul(&local("stage")));
    assert_eq!(world("arm"), world("stage").mul(&local("arm")));
    let hand = graph.socket("arm", "hand").unwrap();
    let socket = tessel_editor_core::directed_scene::Transform {
        position: hand.position,
        rotation: hand.rotation,
        scale: Vec3 {
            x: 1.0,
            y: 1.0,
            z: 1.0,
        },
    }
    .matrix();
    assert_eq!(world("pen"), world("arm").mul(&socket).mul(&local("pen")));
    for node in ["stage", "arm", "pen"] {
        assert!(
            world(node).max_abs_diff(graph.world_matrix(node).unwrap()) > 1e-3,
            "{node}: the static graph is not an animated frame"
        );
    }
    // The child follows: moving only the arm's rotation moves the pen, not the backdrop.
    let at_rest = scene
        .bundle()
        .evaluate_instance_nodes("inst-a", 21)
        .unwrap();
    assert!(
        at_rest
            .world_matrix("pen")
            .unwrap()
            .max_abs_diff(frame.world_matrix("pen").unwrap())
            > 1e-3
    );
    assert_eq!(
        at_rest.world_matrix("backdrop"),
        frame.world_matrix("backdrop")
    );
}

#[test]
fn without_channels_the_nodes_are_the_static_graph() {
    let mut raw = base();
    raw["sceneInstances"][0]["performanceTrackIds"] = json!([]);
    let scene = import(raw);
    let graph = scene.bundle().scene("scene-rig").unwrap();
    let frame = scene.bundle().evaluate_frame(13).unwrap();
    assert_eq!(frame.node_animation, NodeAnimation::Static);
    assert!(frame.nodes.overrides().is_empty());
    for node in graph.nodes() {
        let evaluated = frame.nodes.world_matrix(&node.id).unwrap();
        let authored = graph.world_matrix(&node.id).unwrap();
        // Through quaternion -> Euler -> quaternion: equal up to rounding, not necessarily to the bit.
        assert!(evaluated.max_abs_diff(authored) < 1e-12, "{}", node.id);
    }
}

// ---- typed errors ----------------------------------------------------------------------------------

#[test]
fn two_channels_on_one_property_are_a_typed_error_with_the_origin_message() {
    let origin = origin();
    let (_, raw, entry) = variants_of(&origin, "valid")
        .into_iter()
        .find(|(name, ..)| name == "writer-conflict-same-track")
        .unwrap();
    // The origin accepts the bundle (`finalizeBundle`) and fails only when it evaluates a frame.
    assert_eq!(entry["accepted"], true);
    let scene = import(raw);
    let bundle = scene.bundle();
    let expected = "Multiple performance channels write arm:localRotation on instance inst-a";

    let error = bundle.evaluate_frame(0).unwrap_err();
    assert_eq!(error.to_string(), expected);
    assert_eq!(
        error,
        TemporalError::PerformanceWriterConflict {
            instance_id: "inst-a".to_string(),
            node_id: "arm".to_string(),
            property: ChannelProperty::LocalRotation,
        }
    );
    assert_eq!(error.code(), "invalid_animation");
    assert_eq!(bundle.performance_plan("inst-a").unwrap_err(), error);
    assert_eq!(
        bundle.evaluate_instance_nodes("inst-a", 3).unwrap_err(),
        error
    );
    let command = CommandError::from(error);
    assert_eq!(command.code, "invalid_animation");
    assert_eq!(command.message, expected);

    // The camera does not depend on the nodes: it is still available on its own.
    assert!(bundle.evaluate_camera(0).is_ok());
    // A frame outside the edit reports the range, before any channel is looked at (the origin's order).
    assert_eq!(
        bundle.evaluate_frame(48).unwrap_err().code(),
        "frame_out_of_range"
    );
}

#[test]
fn a_conflict_with_an_unlisted_track_or_across_instances_is_not_a_conflict() {
    // Unlisted: the instance does not apply it (the origin's allow-list).
    let mut raw = base();
    raw["performanceTracks"].as_array_mut().unwrap().push(json!({
        "id": "perf-unlisted", "instanceId": "inst-a",
        "channels": [{ "kind": "static", "channelId": "u", "nodeId": "arm", "property": "localRotation",
                        "value": { "x": 9, "y": 9, "z": 9 } }]
    }));
    let scene = import(raw);
    let plan = scene.bundle().performance_plan("inst-a").unwrap();
    assert_eq!(plan.track_ids(), vec!["perf-1"]);
    assert_eq!(plan.channel_count(), 3);
    assert!(!plan.is_empty());

    // Different instances of one scene may write the same property.
    let scene = two_instances();
    assert!(scene.bundle().performance_plan("inst-a").is_ok());
    assert!(scene.bundle().performance_plan("inst-b").is_ok());
}

#[test]
fn asking_outside_the_instance_or_for_an_unknown_one_is_an_error_not_a_guess() {
    let scene = import(base());
    let bundle = scene.bundle();
    assert!(bundle.evaluate_instance_nodes("inst-a", 47).is_ok());
    let past = bundle.evaluate_instance_nodes("inst-a", 48).unwrap_err();
    assert_eq!(
        past,
        TemporalError::InstanceFrameOutOfRange {
            instance_id: "inst-a".to_string(),
            frame: 48,
            duration_frames: 48
        }
    );
    assert_eq!(past.code(), "frame_out_of_range");
    let unknown = bundle.node_timeline("nobody").unwrap_err();
    assert_eq!(unknown.to_string(), "Unknown scene instance: nobody");
    assert!(bundle.performance_plan("nobody").is_err());
}

#[test]
fn a_node_that_overflows_is_named_instead_of_handing_infinity_to_a_renderer() {
    // A finite amplitude the schema allows, whose offset is not representable.
    let scene = with_channels(json!([{
        "kind": "cycle", "channelId": "c", "nodeId": "arm", "property": "localPosition",
        "periodFrames": 4, "amplitude": 1e308, "axis": { "x": 10, "y": 0, "z": 0 }, "phaseOffsetFrames": 1
    }]));
    let error = scene.bundle().evaluate_frame(0).unwrap_err();
    assert_eq!(
        error,
        TemporalError::NonFiniteNode {
            node_id: "arm".to_string()
        }
    );
    assert_eq!(error.code(), "invalid_animation");

    // Finite locals whose product through the hierarchy overflows: the first node it reaches.
    let scene = with_channels(json!([
        { "kind": "static", "channelId": "a", "nodeId": "stage", "property": "localScale", "value": { "x": 1e200, "y": 1e200, "z": 1e200 } },
        { "kind": "static", "channelId": "b", "nodeId": "arm", "property": "localScale", "value": { "x": 1e200, "y": 1e200, "z": 1e200 } }
    ]));
    assert_eq!(
        scene.bundle().evaluate_frame(0).unwrap_err(),
        TemporalError::NonFiniteNode {
            node_id: "arm".to_string()
        }
    );
}

// ---- what is not evaluated, said out loud -----------------------------------------------------------

#[test]
fn rigs_effects_and_bound_nodes_are_listed_and_a_null_section_is_not() {
    let mut raw = base();
    raw["sceneDefinitions"][0]["effects"] = json!({ "any": "payload" });
    raw["sceneDefinitions"][0]["rigs"] = json!([]);
    raw["sceneDefinitions"][0]["paths"] = Value::Null;
    raw["sceneDefinitions"][0]["nodes"][3]["rigBind"] = json!({ "rigId": "r" });
    raw["sceneDefinitions"][0]["nodes"][4]["reveal"] = json!({ "any": 1 });
    let scene = import(raw);
    let graph = scene.bundle().scene("scene-rig").unwrap();
    // Sorted, no repeats, `paths: null` declares nothing. The raw payloads are kept untouched.
    assert_eq!(
        graph.unevaluated_sections(),
        ["effects", "nodes[].reveal", "nodes[].rigBind", "rigs"]
    );
    assert_eq!(
        scene.document()["sceneDefinitions"][0]["effects"],
        json!({ "any": "payload" })
    );

    let frame = scene.bundle().evaluate_frame(6).unwrap();
    assert!(!frame.is_complete());
    // The channels are still evaluated: only what has no evaluator is reported.
    assert!(matches!(
        frame.node_animation,
        NodeAnimation::Animated { channels: 3, .. }
    ));
    let json = frame.to_json();
    assert_eq!(json["complete"], false);
    assert_eq!(
        json["unevaluated"],
        json!(["effects", "nodes[].reveal", "nodes[].rigBind", "rigs"])
    );
    let Err(TemporalError::Unsupported(Unsupported::DeferredSections {
        scene_definition_id,
        sections,
    })) = frame.require_complete()
    else {
        panic!("a scene with rigs is not a complete frame");
    };
    assert_eq!(scene_definition_id, "scene-rig");
    assert_eq!(sections.len(), 4);

    // The plain oracle case declares none of them.
    let plain = import(base());
    assert!(plain.bundle().evaluate_frame(6).unwrap().is_complete());
}

// ---- the evaluated state, queryable and serializable ------------------------------------------------

#[test]
fn the_evaluated_frame_serializes_for_a_renderer() {
    let scene = two_instances();
    let frame = scene.bundle().evaluate_frame(20).unwrap();
    let json = frame.to_json();
    for key in [
        "editFrame",
        "shotId",
        "instanceId",
        "instanceFrame",
        "cameraClipId",
        "cameraClipFrame",
        "editTime",
        "instanceTime",
        "cameraClipTime",
        "camera",
        "animation",
        "complete",
        "unevaluated",
        "sceneDefinitionId",
        "nodes",
        "performanceOverrides",
    ] {
        assert!(json.get(key).is_some(), "missing `{key}`");
    }
    assert_eq!(json["shotId"], "shot-2");
    assert_eq!(json["instanceId"], "inst-b");
    assert_eq!(json["instanceFrame"], 25);
    assert_eq!(json["cameraClipFrame"], 4);
    assert_eq!(
        json["editTime"],
        json!({ "numerator": 5, "denominator": 6 })
    );
    assert_eq!(json["animation"]["status"], "animated");
    assert_eq!(json["animation"]["trackIds"], json!(["perf-b"]));
    assert_eq!(json["animation"]["channels"], 2);
    assert_eq!(json["complete"], true);

    // What the JSON says is what the accessors say.
    let nodes = json["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), frame.nodes.nodes().len());
    for (value, state) in nodes.iter().zip(frame.nodes.nodes()) {
        assert_eq!(value["nodeId"], state.node_id.as_str());
        assert_eq!(value["renderOrder"], state.render_order);
        let world: Vec<f64> = value["worldMatrix"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_f64().unwrap())
            .collect();
        assert_eq!(world, state.world_matrix.0.to_vec(), "{}", state.node_id);
        assert_eq!(value["localScale"], state.local_scale.to_json());
    }
    let overrides = json["performanceOverrides"].as_array().unwrap();
    assert_eq!(overrides.len(), 2);
    assert_eq!(overrides[1]["property"], "localScale");
    // The JSON is plain text a renderer can parse: nothing in it needs the borrow of the bundle.
    // (Not compared for equality: serde_json parses doubles without `float_roundtrip`, which may
    // land one ulp away; the numbers themselves are compared above from the accessors.)
    let parsed: Value = serde_json::from_str(&json.to_string()).unwrap();
    assert_eq!(parsed["nodes"].as_array().unwrap().len(), nodes.len());
    // A node frame can be kept after the evaluation is dropped.
    let kept = frame.nodes.clone();
    drop(frame);
    assert_eq!(kept.to_json()["nodes"], json["nodes"]);
    assert_eq!(kept.scene_definition_id(), "scene-rig");
}

// ---- the comparison would catch a wrong evaluator ------------------------------------------------------

#[test]
fn negative_control_a_wrong_phase_offset_fails_the_same_comparison() {
    let origin = origin();
    let (name, mut raw, entry) = variants_of(&origin, "valid")
        .into_iter()
        .find(|(name, ..)| name == "cycle-edges")
        .unwrap();
    // One frame of phase: an effect of many orders of magnitude above the tolerance.
    raw["performanceTracks"][0]["channels"][2]["phaseOffsetFrames"] = json!(101);
    let scene = import(raw);
    let divergence = Compare::default()
        .evaluation(&name, &scene, entry["editFrames"].as_array().unwrap())
        .expect_err("a wrong phase offset must not pass the comparison");
    eprintln!("caught: {divergence}");
}

#[test]
fn negative_control_a_wrong_event_duration_fails_the_same_comparison() {
    let origin = origin();
    let (name, mut raw, entry) = variants_of(&origin, "valid")
        .into_iter()
        .find(|(name, ..)| name == "event-edges")
        .unwrap();
    raw["performanceTracks"][0]["channels"][1]["durationFrames"] = json!(21);
    let scene = import(raw);
    Compare::default()
        .evaluation(&name, &scene, entry["editFrames"].as_array().unwrap())
        .expect_err("a wrong duration must not pass the comparison");
}

#[test]
fn negative_control_the_unmodified_variants_pass_the_same_comparison() {
    // The controls above fail because of the mutation, not because the helper always fails.
    let origin = origin();
    let (name, raw, entry) = variants_of(&origin, "valid")
        .into_iter()
        .find(|(name, ..)| name == "cycle-edges")
        .unwrap();
    Compare::default()
        .evaluation(&name, &import(raw), entry["editFrames"].as_array().unwrap())
        .unwrap();
}

// ---- the scene is never changed by evaluating -------------------------------------------------------

#[test]
fn evaluating_never_changes_the_scene() {
    let scene = two_instances();
    let before = scene.clone();
    for frame in [-1, 0, 33, 69, 70] {
        let _ = scene.bundle().evaluate_frame(frame);
    }
    let _ = scene.bundle().performance_plan("inst-a");
    let _ = scene.bundle().evaluate_instance_nodes("inst-b", 59);
    assert_eq!(scene, before);
    assert_eq!(scene.to_json_string(), before.to_json_string());
}

#[test]
fn integer_valued_floats_are_accepted_as_frame_counts_like_javascript_does() {
    // `24.0` is a whole number to the origin (a JS number); serde_json keeps it as a float.
    let text = BASE
        .replace("\"periodFrames\": 24,", "\"periodFrames\": 24.0,")
        .replace("\"durationFrames\": 24,", "\"durationFrames\": 24.0,");
    assert_ne!(text, BASE);
    let scene = DirectedScene::import_str(&text).unwrap();
    assert!(scene.bundle().evaluate_frame(30).is_ok());
}

#[test]
fn evaluated_world_matrices_survive_json_roundtrip_without_losing_binary64_bits() {
    let scene = DirectedScene::import_str(BASE).unwrap();
    for frame in 0..48 {
        let state = scene.bundle().evaluate_frame(frame).unwrap().nodes;
        let decoded: Value = serde_json::from_str(&state.to_json().to_string()).unwrap();
        for (node_index, node) in state.nodes().iter().enumerate() {
            for (index, actual) in node.world_matrix.0.iter().enumerate() {
                let roundtripped = decoded["nodes"][node_index]["worldMatrix"][index]
                    .as_f64()
                    .unwrap();
                assert_eq!(
                    actual.to_bits(),
                    roundtripped.to_bits(),
                    "frame {frame}, node {}, matrix element {index}: {actual} -> {roundtripped}",
                    node.node_id
                );
            }
        }
    }
}
