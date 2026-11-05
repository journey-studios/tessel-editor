//! R10 (BL008/BL019): the lib/headless half of the independent export proof.
//!
//! The corpus in `scripts/directed-scene-export-proof/corpus` is written by that folder's own
//! generator from the independent reference model (`scripts/directed-scene-proof/model.py`) and
//! from exact fractions; nothing here (or there) is read back from an export. These tests check
//! that the editor's *domain* agrees with those expectations before any pixel exists:
//!
//! - every corpus bundle is accepted by the core and every case's schedule (frame count, exact
//!   rational rate, duration, output size) equals the independent number;
//! - the ranges the corpus says are invalid are refused with a typed code and change nothing;
//! - the declared audio mix is never silently dropped (refused, mixed or explicitly omitted);
//! - the request the Python adapter would send (`adapters.json`) parses in the core, so an
//!   adapter that drifts from the contract fails here and not only against a built binary.
//!
//! What this does NOT prove: pixels, codecs, ffprobe, cancellation, restore, socket or UI. Those
//! are the Python harness (`proof.py`), reachable from the `#[ignore]` tests below, which fail
//! with the harness's exit code (3 = BLOCKED: an operation or a window flow is still missing).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};
use tessel_editor_core::scene_export::{SceneExportSnapshot, SceneExportSpec};
use tessel_editor_core::{CommandError, EditorState, headless};

fn proof_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/directed-scene-export-proof")
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap_or_else(|e| panic!("{path:?}: {e}")))
        .unwrap_or_else(|e| panic!("{path:?}: {e}"))
}

fn expected() -> Value {
    read_json(&proof_dir().join("corpus/expected.json"))
}

fn adapters() -> Value {
    read_json(&proof_dir().join("adapters.json"))
}

fn bundle(name: &str) -> Value {
    read_json(&proof_dir().join(format!("corpus/bundle.{name}.json")))
}

fn project_with(scene: &str, bundle: Value) -> EditorState {
    let mut state = EditorState::demo();
    headless::execute(&mut state, &json!({"op": "project.create", "name": "R10"})).unwrap();
    headless::execute(
        &mut state,
        &json!({"op": "scene.attach", "id": scene, "bundle": bundle}),
    )
    .unwrap_or_else(|e| panic!("`{scene}` is rejected by the core: {e:?}"));
    state
}

fn capture(state: &EditorState, request: Value) -> Result<SceneExportSnapshot, CommandError> {
    let mut request = request;
    request["op"] = json!("scene.export");
    let spec = SceneExportSpec::from_json(state, &request)?;
    SceneExportSnapshot::capture(state, &spec)
}

/// `"n/d"` as two integers.
fn fraction(text: &str) -> (i128, i128) {
    let (num, den) = text.split_once('/').unwrap_or((text, "1"));
    (num.parse().unwrap(), den.parse().unwrap())
}

#[test]
fn every_case_schedule_equals_the_independent_expectation() {
    let expected = expected();
    let cases = expected["cases"].as_object().unwrap();
    assert!(cases.len() >= 7, "the corpus lost cases: {}", cases.len());
    for (id, case) in cases {
        let name = case["bundle"].as_str().unwrap();
        let state = project_with("proof", bundle(name));
        let snapshot = capture(
            &state,
            json!({
                "scene": "proof",
                "preset": case["preset"],
                "from": case["from"],
                "frames": case["frames"],
                // The corpus's audio bundle declares a mix; the schedule is audio independent.
                "audio": false,
            }),
        )
        .unwrap_or_else(|e| panic!("{id}: {e:?}"));
        let schedule = snapshot.schedule();

        let want_frames = case["frames"].as_u64().unwrap();
        assert_eq!(schedule.frames, want_frames, "{id}: frame count");
        assert_eq!(
            schedule.from,
            case["from"].as_i64().unwrap(),
            "{id}: first frame"
        );

        // Exact rational rate, never a float: 30000/1001 must not become 30 or 29.97.
        let (num, den) = fraction(case["fps"].as_str().unwrap());
        assert_eq!(
            (
                i128::from(schedule.output_rate.num()),
                i128::from(schedule.output_rate.den())
            ),
            (num, den),
            "{id}: rate"
        );

        // duration = frames * den / num, compared by cross-multiplication.
        let (dur_num, dur_den) = fraction(case["durationSeconds"].as_str().unwrap());
        assert_eq!(
            i128::from(schedule.frames) * den * dur_den,
            dur_num * num,
            "{id}: duration {} s",
            case["durationSeconds"]
        );

        let output = snapshot.summary()["output"].clone();
        assert_eq!(
            (output["width"].as_u64(), output["height"].as_u64()),
            (case["output"][0].as_u64(), case["output"][1].as_u64()),
            "{id}: output size"
        );
    }
}

#[test]
fn the_ntsc_scene_keeps_an_exact_rational_rate_in_its_bundle() {
    // The corpus must really carry 30000/1001 (not 30 or 29.97) or the rate checks prove nothing.
    let ntsc = bundle("ntsc");
    let text = ntsc["fps"].to_string();
    assert!(
        text.contains("30000") && text.contains("1001"),
        "ntsc bundle fps: {text}"
    );
}

#[test]
fn ranges_the_corpus_calls_invalid_are_refused_typed_and_change_nothing() {
    let expected = expected();
    let adapters = adapters();
    let accepted: Vec<&str> = adapters["error_codes"]["range"]
        .as_array()
        .unwrap()
        .iter()
        .map(|code| code.as_str().unwrap())
        .collect();
    let state = project_with("proof", bundle("main"));
    // What a refused request must leave alone: the scenes the project holds, as the host lists them.
    let listing =
        |state: &mut EditorState| headless::execute(state, &json!({"op": "scene.list"})).unwrap();
    let mut state = state;
    let before = listing(&mut state);
    for case in expected["rangeErrors"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let error = capture(
            &state,
            json!({"scene": "proof", "preset": "fullhd", "from": case["frm"], "frames": case["frames"]}),
        )
        .err()
        .unwrap_or_else(|| panic!("{id}: an invalid range was accepted"));
        assert!(
            accepted.contains(&error.code),
            "{id}: code `{}` is not typed as a range error ({accepted:?})",
            error.code
        );
    }
    assert_eq!(
        before,
        listing(&mut state),
        "a refused request mutated the project"
    );
}

#[test]
fn a_scene_that_declares_audio_is_never_silently_exported_without_it() {
    let adapters = adapters();
    let refusals: Vec<&str> = adapters["error_codes"]["audio_refusal"]
        .as_array()
        .unwrap()
        .iter()
        .map(|code| code.as_str().unwrap())
        .collect();
    let state = project_with("proof", bundle("audio"));
    let request = json!({"scene": "proof", "preset": "fullhd", "from": 36, "frames": 24});

    // Default request: the declared mix is either refused (typed) or in the file.
    match capture(&state, request.clone()) {
        Err(error) => assert!(
            refusals.contains(&error.code),
            "audio refusal code `{}` is not one of {refusals:?}",
            error.code
        ),
        Ok(snapshot) => {
            let audio = snapshot.summary()["audio"].clone();
            assert_eq!(
                audio["included"],
                json!(true),
                "the export accepted a scene with a declared mix but does not include it: {audio}"
            );
        }
    }

    // `audio: false` is the one way to a picture-only file and the result must say what it left out.
    let mut video_only = request;
    video_only["audio"] = json!(false);
    let summary = capture(&state, video_only).unwrap().summary();
    let audio = summary["audio"].clone();
    let typed_warning = adapters["error_codes"]["audio_omitted_warning_codes"]
        .as_array()
        .unwrap();
    assert!(
        summary["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| typed_warning.contains(&warning["code"])),
        "the omission carries no typed warning: {}",
        summary["warnings"]
    );
    assert_eq!(audio["included"], json!(false), "{audio}");
    assert!(
        audio["declared"].as_array().is_some_and(|d| !d.is_empty()),
        "{audio}"
    );
    assert!(
        audio["omitted"].is_string(),
        "the omission is not stated: {audio}"
    );

    // A scene that declares nothing owes nothing.
    let silent = project_with("proof", bundle("main"));
    let summary = capture(
        &silent,
        json!({"scene": "proof", "preset": "fullhd", "from": 0, "frames": 8}),
    )
    .unwrap()
    .summary();
    assert_eq!(summary["audio"]["included"], json!(false));
    assert_eq!(summary["audio"]["declared"], json!([]));
}

#[test]
fn the_request_the_adapter_sends_parses_in_the_core() {
    let adapters = adapters();
    let args = adapters["ops"]["export"]["args"].as_object().unwrap();
    assert_eq!(adapters["ops"]["export"]["op"], "scene.export");
    for contract_key in ["scene", "preset", "destination", "from", "frames"] {
        assert!(
            args.contains_key(contract_key),
            "adapter lost contract field `{contract_key}`"
        );
    }
    // Every placeholder the adapter uses, filled with a real value (null drops the key, as in the harness).
    let values = json!({
        "$scene": "proof", "$preset": "fullhd", "$destination": "/tmp/te-r10-never.mp4",
        "$from": 12, "$frames": 8, "$wait": true, "$replace": false, "$audio": false,
    });
    let mut request = json!({"op": "scene.export"});
    for (key, value) in args {
        let name = value.as_str().unwrap();
        let filled = values
            .get(name)
            .unwrap_or_else(|| panic!("unknown placeholder `{name}` for `{key}`"));
        request[key] = filled.clone();
    }
    let state = project_with("proof", bundle("audio"));
    let spec = SceneExportSpec::from_json(&state, &request)
        .expect("the adapter's request is not parsed by the core");
    assert_eq!((spec.from, spec.frames), (12, Some(8)));
}

/// Runs the Python harness; its exit code is the verdict (0 pass, 1 fail, 3 BLOCKED, 2 usage).
fn run_harness(phases: &str) {
    let proof = proof_dir();
    let root = proof.join("../..");
    let lock = root.join("scripts/directed-scene-proof/with-gpu-lock.py");
    let report = std::env::temp_dir().join("te-r10-rust-report.json");
    let status = Command::new("python3")
        .current_dir(&root)
        .arg(lock)
        .args(["python3"])
        .arg(proof.join("proof.py"))
        .args(["--gpu-lock-held", "--phases", phases, "--report"])
        .arg(&report)
        .status()
        .expect("python3 is required for the export proof harness");
    let _ = fs::remove_file(&report);
    match status.code() {
        Some(0) => {}
        Some(3) => panic!(
            "BLOCKED (exit 3): an operation or window flow the proof needs is missing; see the harness output"
        ),
        other => panic!("the export proof harness failed (exit {other:?}); see the harness output"),
    }
}

#[test]
#[ignore = "runs the real renderer and encoder under the GPU lock (minutes); BL008 acceptance"]
fn export_proof_headless_phases() {
    run_harness(
        "export_main_1080_event,export_main_1080_cut,export_main_4k_cut,export_ntsc_1080,export_detail_4k,\
         export_invalid_requests,export_destination_exists,export_cancel,export_snapshot,export_audio_policy",
    );
}

#[test]
#[ignore = "runs the real host under the GPU lock; BL019 acceptance: a restored project without its origin"]
fn export_proof_restored_projects() {
    run_harness("export_restored_package,export_restored_managed");
}

#[test]
#[ignore = "needs the release `tessel-editor` built from the integrated tree (build it with the serialized helper first); BLOCKED, not failed, when the build has no flow.export_scene"]
fn export_proof_window_flows() {
    run_harness("socket_export,ui_export_flow");
}
