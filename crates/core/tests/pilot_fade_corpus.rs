//! P2 (BL029/BL040/BL082.1): the pinned pilot FADE corpus, run against the Rust core.
//!
//! `docs/migration/fixtures/pilot-fx/fade-cases.json` was derived from the pilot's `fade` and
//! `vignette` shader sources by `scripts/pilot-fade-corpus.py` (standalone, shares no routine with
//! the Rust reference; `fade-manifest.json` names the origin snapshot, the blob ids and sha256 of
//! every source, the documented differences and the blockers). Agreeing here is two derivations of
//! the origin's shaders agreeing, never the reference compared with itself.
//!
//! What it does NOT prove: the origin's golden `fade.png` (it needs the origin's harness, which is
//! not run here: manifest blocker `ORIGIN_GOLDEN_NOT_COMPARABLE`) and the origin's own outputs
//! (blocker `ORIGIN_NOT_EXECUTED`). The corpus hash and the origin blobs are checked by
//! `scripts/pilot-fade-corpus.py verify [--origin PATH]`.

use core::prelude::v1::test;
use serde_json::{Value, json};
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::directed_scene::fx;
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::{CommandError, EditorState, headless};

const CORPUS: &str = include_str!("../../../docs/migration/fixtures/pilot-fx/fade-cases.json");
const MANIFEST: &str = include_str!("../../../docs/migration/fixtures/pilot-fx/fade-manifest.json");

const BASE: &str =
    include_str!("../../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json");

fn corpus() -> Value {
    serde_json::from_str(CORPUS).unwrap()
}

fn bundle_with(fx: &Value) -> Value {
    let mut bundle: Value = serde_json::from_str(BASE).unwrap();
    bundle["fx"] = fx.clone();
    bundle
}

fn session(fx: &Value) -> EditorState {
    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    run(
        &mut state,
        json!({"op":"scene.attach","id":"street","bundle":bundle_with(fx)}),
    );
    state
}

fn run(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn refuse(state: &mut EditorState, request: Value) -> CommandError {
    match headless::execute(state, &request) {
        Ok(answer) => panic!("{request} should have been refused, got {answer}"),
        Err(error) => error,
    }
}

fn near(left: f64, right: f64) -> bool {
    (left - right).abs() < 1e-9
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len() / 2)
        .map(|index| u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).unwrap())
        .collect()
}

/// The largest channel difference between two pictures of the same size.
fn worst_difference(left: &[u8], right: &[u8]) -> u8 {
    assert_eq!(left.len(), right.len());
    left.iter()
        .zip(right)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0)
}

#[test]
fn the_manifest_names_the_fade_sources_the_differences_and_the_blockers_it_could_not_close() {
    let manifest: Value = serde_json::from_str(MANIFEST).unwrap();
    assert_eq!(manifest["origin"]["snapshot"], "d40ac3f7c0");
    assert_eq!(manifest["corpus"]["file"], "fade-cases.json");
    assert_eq!(manifest["corpus"]["sha256"].as_str().unwrap().len(), 64);
    let sources = manifest["sources"].as_array().unwrap();
    for source in sources {
        assert_eq!(source["git_blob"].as_str().unwrap().len(), 40, "{source}");
        assert_eq!(source["sha256"].as_str().unwrap().len(), 64, "{source}");
    }
    let paths: Vec<_> = sources
        .iter()
        .map(|s| s["path"].as_str().unwrap())
        .collect();
    for needed in [
        "packages/scene-engine/src/fx/catalog/fade.ts",
        "packages/scene-engine/src/runtime/fx/shaders/fade.ts",
        "packages/scene-engine/src/fx/catalog/__golden__/fade.png",
    ] {
        assert!(paths.contains(&needed), "{needed}");
    }
    let blockers: Vec<_> = manifest["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["id"].as_str().unwrap())
        .collect();
    assert!(blockers.contains(&"ORIGIN_GOLDEN_NOT_COMPARABLE"));
    assert!(blockers.contains(&"ORIGIN_NOT_EXECUTED"));
    let differences: Vec<_> = manifest["documented_differences"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["id"].as_str().unwrap())
        .collect();
    assert!(differences.contains(&"ZERO_WEIGHT_IS_BYTE_IDENTITY"));
}

#[test]
fn the_fade_passes_are_the_pilots_in_amount_weight_order_and_the_amount_parameter() {
    let corpus = corpus();
    let mut compared = 0;
    for case in corpus["cases"].as_array().unwrap() {
        if case["kind"] != "frames" {
            continue;
        }
        let id = case["id"].as_str().unwrap();
        let mut state = session(&case["fx"]);
        for frame in case["frames"].as_array().unwrap() {
            let at = frame["frame"].as_i64().unwrap();
            let answer = run(
                &mut state,
                json!({"op":"scene.fx.frame","scene":"street","frame":at}),
            );
            let got = answer["passes"].as_array().unwrap();
            let want = frame["passes"].as_array().unwrap();
            assert_eq!(got.len(), want.len(), "{id} frame {at}: pass count");
            for (got, want) in got.iter().zip(want) {
                assert_eq!(got["clipId"], want["clip"], "{id} frame {at}: order");
                assert_eq!(got["effect"], want["effect"], "{id} frame {at}");
                assert_eq!(got["frameInClip"], want["frame_in_clip"], "{id} frame {at}");
                for key in ["weight", "amount"] {
                    assert!(
                        near(got[key].as_f64().unwrap(), want[key].as_f64().unwrap()),
                        "{id} frame {at}: {key} is {} but the pilot says {}",
                        got[key],
                        want[key]
                    );
                }
                if let Some(fade_amount) = want["fade_amount"].as_f64() {
                    assert!(
                        near(got["params"]["amount"].as_f64().unwrap(), fade_amount),
                        "{id} frame {at}: the amount parameter"
                    );
                }
                compared += 1;
            }
        }
    }
    assert_eq!(compared, 16, "the corpus changed: regenerate and review");
}

#[test]
fn the_fade_pixels_are_the_pilots_shader_and_a_tampered_expectation_is_caught() {
    let corpus = corpus();
    let mut compared = 0;
    let mut identities = 0;
    let mut outputs: Vec<(String, Vec<u8>)> = Vec::new();
    for case in corpus["cases"].as_array().unwrap() {
        if case["kind"] != "pixels" {
            continue;
        }
        let id = case["id"].as_str().unwrap();
        let (width, height) = (
            case["width"].as_u64().unwrap() as usize,
            case["height"].as_u64().unwrap() as usize,
        );
        let input = unhex(case["input_rgba_hex"].as_str().unwrap());
        let expected = unhex(case["expected_rgba_hex"].as_str().unwrap());
        let tolerance = case["tolerance_per_channel"].as_u64().unwrap() as u8;

        let scene = DirectedScene::import_value(bundle_with(&case["fx"])).unwrap();
        let mut rgba = input.clone();
        for pass in scene.bundle().fx.resolve(case["frame"].as_i64().unwrap()) {
            assert!(
                fx::apply_pass_to_rgba8(&pass, &mut rgba, width, height),
                "{id}"
            );
        }
        let worst = worst_difference(&rgba, &expected);
        assert!(
            worst <= tolerance,
            "{id}: worst channel difference {worst} > {tolerance}"
        );
        let identity = case["identity"].as_bool().unwrap();
        if identity {
            assert_eq!(rgba, input, "{id}: byte for byte");
            identities += 1;
        } else {
            assert_ne!(rgba, input, "{id}: the effect must change the picture");
            // Negative control: an expectation moved by more than the tolerance must not pass.
            let mut tampered = expected.clone();
            let last = tampered.len() - 2;
            tampered[last] = tampered[last].wrapping_add(tolerance + 4);
            assert!(
                worst_difference(&rgba, &tampered) > tolerance,
                "{id}: the comparison is blind to a changed expectation"
            );
        }
        outputs.push((id.to_string(), rgba));
        compared += 1;
    }
    assert_eq!(compared, 7);
    assert_eq!(identities, 2, "amount 0 and intensity 0 are the baseline");
    // The two stacking cases differ in order only: the pilot's bytes differ, so do ours.
    let find = |name: &str| &outputs.iter().find(|(id, _)| id == name).unwrap().1;
    assert_ne!(find("fade-then-vignette"), find("vignette-then-fade"));
}

#[test]
fn the_fade_refusals_the_catalog_ranges_imply_are_typed() {
    for case in corpus()["refusals"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let mut state = session(&case["fx"]);
        let before = state.history.to_json()["undo_depth"].as_u64().unwrap();
        let error = refuse(
            &mut state,
            json!({"op":"scene.fx.edit","scene":"street","ops":[case["op"].clone()]}),
        );
        assert_eq!(error.code, case["code"].as_str().unwrap(), "{id}");
        assert_eq!(
            state.history.to_json()["undo_depth"].as_u64().unwrap(),
            before,
            "{id}: a refusal leaves no entry"
        );
    }
}
