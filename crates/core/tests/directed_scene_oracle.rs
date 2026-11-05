//! Compare the supported import/structural-join subset with source-generated expectations.
//! This is not temporal camera or performance equivalence.
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::{DirectedScene, Parent};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/migration/fixtures")
}

fn read_json(path: impl AsRef<Path>) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn import_and_structural_joins_match_all_source_corpus_verdicts() {
    let root = fixtures();
    let index = read_json(root.join("cases.json"));
    let cases = index["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 55, "review coverage when the corpus changes");
    for case in cases {
        let expected = read_json(root.join(case["expected"].as_str().unwrap()));
        let raw = read_json(root.join(case["bundle"].as_str().unwrap()));
        let imported = DirectedScene::import_value(raw.clone());
        let accepted = imported
            .as_ref()
            .is_ok_and(|scene| scene.edit_join_issues().is_empty());
        assert_eq!(
            accepted,
            expected["validation"]["accepted"].as_bool().unwrap(),
            "{}: import result {imported:?}",
            case["id"]
        );
        if accepted {
            let scene = imported.unwrap();
            assert_eq!(scene.document(), &raw);
            let reparsed = DirectedScene::import_str(&scene.to_json_string()).unwrap();
            assert_eq!(reparsed, scene);
            for graph in &scene.bundle().scenes {
                let links = &expected["evaluation"]["parentLinks"][graph.id()];
                assert_eq!(links.as_object().unwrap().len(), graph.nodes().len());
                for node in graph.nodes() {
                    let actual = match graph.parent(&node.id) {
                        None => json!({"kind": "root"}),
                        Some(Parent::Node(id)) => json!({"kind": "parent", "parentId": id}),
                        Some(Parent::Socket { owner, socket }) => json!({
                            "kind": "socket", "parentNodeId": owner, "socketName": socket
                        }),
                    };
                    assert_eq!(actual, links[&node.id], "{} / {}", case["id"], node.id);
                }
            }
        }
    }
}

#[test]
fn light_parent_reference_is_checked_even_before_light_rendering_is_migrated() {
    let raw = read_json(fixtures().join("bundles/invalid-light-parent-unknown.bundle.json"));
    let error = DirectedScene::import_value(raw.clone()).unwrap_err();
    assert!(error.issues().iter().any(|issue| {
        issue.code == "LIGHT_PARENT_UNKNOWN"
            && issue.path == "/sceneDefinitions/0/lighting/lights/0/parent"
    }));
    let mut repaired = raw;
    repaired["sceneDefinitions"][0]["lighting"]["lights"][0]["parent"] = json!("root");
    assert!(DirectedScene::import_value(repaired).is_ok());
}

#[test]
fn static_world_matrices_match_source_frames_with_explicit_numeric_tolerance() {
    // Different floating-point multiplication orders can round differently across engines.
    const ABSOLUTE: f64 = 1e-10;
    const RELATIVE: f64 = 1e-12;
    let root = fixtures();
    let index = read_json(root.join("cases.json"));
    let mut compared = 0;
    for case in index["cases"].as_array().unwrap() {
        if case["kind"] != "valid" {
            continue;
        }
        let raw = read_json(root.join(case["bundle"].as_str().unwrap()));
        if raw["performanceTracks"]
            .as_array()
            .is_some_and(|tracks| !tracks.is_empty())
        {
            continue; // Temporal overrides belong to BB03, not a static graph.
        }
        let expected = read_json(root.join(case["expected"].as_str().unwrap()));
        let scene = DirectedScene::import_value(raw).unwrap();
        for frame in expected["evaluation"]["editFrames"].as_array().unwrap() {
            if frame["ok"] != true {
                continue;
            }
            let graph = scene
                .bundle()
                .scene_of_instance(frame["state"]["instanceId"].as_str().unwrap())
                .unwrap();
            for (node_id, elements) in frame["worldMatrices"].as_object().unwrap() {
                let actual = graph.world_matrix(node_id).unwrap();
                let expected_elements = elements.as_array().unwrap();
                assert_eq!(expected_elements.len(), 16);
                for (actual, expected) in actual.0.iter().zip(expected_elements) {
                    let expected = expected.as_f64().unwrap();
                    let tolerance = ABSOLUTE + RELATIVE * expected.abs();
                    assert!(
                        (actual - expected).abs() <= tolerance,
                        "{} frame {} node {node_id}: {actual} != {expected}",
                        case["id"],
                        frame["editFrame"]
                    );
                }
                compared += 1;
            }
        }
    }
    assert!(
        compared >= 30,
        "static matrix coverage unexpectedly decreased"
    );
}
