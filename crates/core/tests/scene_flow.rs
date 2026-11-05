//! BL019 / front P: the library surface of the directed-scene end-to-end flow, over the synthetic
//! corpus in `scripts/directed-scene-proof/corpus` (images, bundle with layers, camera and
//! animation, and an `expected.json`).
//!
//! Where the expected values come from: `scripts/directed-scene-proof/model.py`, closed-form maths
//! written without reading this crate's evaluator and validated against the *origin* oracle
//! (`model.spec.py`). `expected.json` never contains output of this crate, of the runtime or of a
//! render, so an agreement here is two independent derivations meeting, not a snapshot.
//!
//! Surface evidence, kept apart on purpose:
//!
//! - **Lib** (this file): typed `ScenePlan`/`EditorState` API and the in-process JSON executor.
//! - **Headless process, native socket**: `scripts/directed-scene-proof/proof.py`
//!   (`headless_flow`, `socket_flow`). A passing test here proves neither.
//! - Not covered by anything in this file: pixels, the renderer, the monitor, packages, the UI and
//!   any physical pointer. Phases that need an API that does not exist yet are in the harness as
//!   BLOCKED, and the `#[ignore = "BLOCKED ..."]` test below fails loudly when run with `--ignored`.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::scene_plan::{PlanError, ScenePlan};
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::{CommandError, EditorState, headless};

const SCENE: &str = "proof";

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/directed-scene-proof/corpus")
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn expected() -> Value {
    read_json(&corpus().join("expected.json"))
}

/// `abs(a - e) <= absolute + relative * abs(e)`, with the tolerance pinned in `expected.json`.
fn close(actual: f64, wanted: f64, tolerance: &Value) -> bool {
    let absolute = tolerance["absolute"].as_f64().unwrap();
    let relative = tolerance["relative"].as_f64().unwrap();
    (actual - wanted).abs() <= absolute + relative * wanted.abs()
}

fn matrix_close(actual: &[f64], wanted: &Value, tolerance: &Value) -> bool {
    let wanted = wanted.as_array().unwrap();
    actual.len() == 16
        && wanted.len() == 16
        && actual
            .iter()
            .zip(wanted)
            .all(|(a, w)| close(*a, w.as_f64().unwrap(), tolerance))
}

fn json_matrix(value: &Value) -> Vec<f64> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|number| number.as_f64().unwrap())
        .collect()
}

fn frame_of(expected: &Value, frame: i64) -> Value {
    expected["frames"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["editFrame"] == frame)
        .unwrap_or_else(|| panic!("the corpus has no expectation for frame {frame}"))
        .clone()
}

fn seek_order(expected: &Value) -> Vec<i64> {
    expected["seekOrder"]
        .as_array()
        .unwrap()
        .iter()
        .map(|frame| frame.as_i64().unwrap())
        .collect()
}

fn run(state: &mut EditorState, request: Value) -> Result<Value, CommandError> {
    headless::execute(state, &request)
}

fn ok(state: &mut EditorState, request: Value) -> Value {
    run(state, request.clone()).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn err(state: &mut EditorState, request: Value) -> CommandError {
    match run(state, request.clone()) {
        Ok(answer) => panic!("{request} should have failed, got {answer}"),
        Err(error) => error,
    }
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tessel-scene-flow-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A project with the corpus bundle attached by path, the way a user or a script attaches it.
fn attached(workspace: &Path) -> EditorState {
    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    ok(
        &mut state,
        json!({"op":"project.create","name":"scene-flow","dir":workspace}),
    );
    ok(
        &mut state,
        json!({"op":"scene.attach","id":SCENE,"path":corpus().join("bundle.json")}),
    );
    state
}

fn undo_depth(state: &mut EditorState) -> u64 {
    ok(state, json!({"op":"history.status"}))["history"]["undo_depth"]
        .as_u64()
        .unwrap()
}

fn revision(state: &mut EditorState) -> u64 {
    ok(state, json!({"op":"history.status"}))["project"]["revision"]
        .as_u64()
        .unwrap()
}

fn evaluate(state: &mut EditorState, frame: i64, gesture: Option<&str>) -> Value {
    let mut request = json!({"op":"scene.evaluate","scene":SCENE,"frame":frame});
    if let Some(gesture) = gesture {
        request["gesture"] = json!(gesture);
    }
    ok(state, request)
}

fn world(answer: &Value, node: &str) -> Vec<f64> {
    let nodes = answer["nodes"].as_array().unwrap();
    let found = nodes
        .iter()
        .find(|entry| entry["nodeId"] == node)
        .unwrap_or_else(|| panic!("no node {node} in the evaluation"));
    json_matrix(&found["worldMatrix"])
}

fn plan_of(state: &EditorState, frame: i64) -> Result<ScenePlan, PlanError> {
    ScenePlan::build(&state.scenes.get(SCENE).unwrap().scene, frame)
}

// ------------------------------------------------------------------------------------------

#[test]
fn the_corpus_bundle_attaches_clean_and_declares_what_the_expectations_assume() {
    let expected = expected();
    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    let attached = ok(
        &mut state,
        json!({"op":"scene.attach","id":SCENE,"path":corpus().join("bundle.json")}),
    );
    assert_eq!(attached["bundle_id"], expected["bundle"]["bundleId"]);
    assert_eq!(attached["revision_id"], expected["bundle"]["revisionId"]);
    assert_eq!(attached["counts"]["nodes"], 6);
    assert_eq!(attached["counts"]["assets"], 3);
    assert_eq!(attached["counts"]["performance_tracks"], 1);
    assert_eq!(
        attached["deferred_in_use"],
        json!([]),
        "the corpus must not use a family the plan refuses"
    );
    assert_eq!(attached["notices"], json!([]));
    assert_eq!(undo_depth(&mut state), 1, "attach is one undo entry");
}

#[test]
fn frames_asked_out_of_order_match_the_independent_model() {
    let expected = expected();
    let mut state = attached(&temp("evaluate"));
    let tolerance = &expected["tolerances"];
    let mut first_answers = std::collections::HashMap::new();
    for frame in seek_order(&expected) {
        let want = frame_of(&expected, frame);
        let answer = evaluate(&mut state, frame, None);
        for axis in ["x", "y", "z"] {
            for part in ["position", "target"] {
                assert!(
                    close(
                        answer["camera"][part][axis].as_f64().unwrap(),
                        want["camera"][part][axis].as_f64().unwrap(),
                        &tolerance["camera"]
                    ),
                    "frame {frame} camera.{part}.{axis}: {} vs {}",
                    answer["camera"][part][axis],
                    want["camera"][part][axis]
                );
            }
        }
        assert!(close(
            answer["camera"]["fovDeg"].as_f64().unwrap(),
            want["camera"]["fovDeg"].as_f64().unwrap(),
            &tolerance["camera"]
        ));
        assert_eq!(answer["shotId"], want["shotId"], "frame {frame}");
        assert_eq!(answer["instanceFrame"], want["instanceFrame"]);
        for (node, matrix) in want["worldMatrices"].as_object().unwrap() {
            assert!(
                matrix_close(&world(&answer, node), matrix, &tolerance["worldMatrix"]),
                "frame {frame} node {node}: {:?} vs {matrix}",
                world(&answer, node)
            );
        }
        // The same question later, after other frames, gets the same answer to the bit.
        if let Some(earlier) = first_answers.get(&frame) {
            assert_eq!(&answer, earlier, "frame {frame} answered differently later");
        }
        first_answers.insert(frame, answer);
    }
    assert!(
        seek_order(&expected)
            .windows(2)
            .any(|pair| pair[1] < pair[0]),
        "the corpus order must go backwards at least once"
    );
}

#[test]
fn the_plan_of_every_frame_matches_the_model_in_any_order_and_names_its_resources_by_asset_id() {
    let expected = expected();
    let state = attached(&temp("plan"));
    let tolerance = &expected["tolerances"];
    let mut by_frame = std::collections::HashMap::new();
    for frame in seek_order(&expected) {
        let want = frame_of(&expected, frame);
        let plan =
            plan_of(&state, frame).unwrap_or_else(|error| panic!("frame {frame}: {error:?}"));
        assert!(plan.is_complete(), "frame {frame}: {:?}", plan.unsupported);
        assert_eq!(plan.frame.shot_id, want["shotId"].as_str().unwrap());

        let layers: Vec<&str> = plan
            .layers
            .iter()
            .map(|layer| layer.node_id.as_str())
            .collect();
        let painted: Vec<&str> = want["paintOrder"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap())
            .collect();
        assert_eq!(layers, painted, "frame {frame}: paint order, back to front");

        for node in &plan.nodes {
            let matrix = &want["worldMatrices"][node.id.as_str()];
            assert!(
                matrix_close(&node.world_matrix.0, matrix, &tolerance["worldMatrix"]),
                "frame {frame} node {}: {:?} vs {matrix}",
                node.id,
                node.world_matrix.0
            );
        }
        for (axis, got) in [
            ("x", plan.camera.position.x),
            ("y", plan.camera.position.y),
            ("z", plan.camera.position.z),
        ] {
            assert!(close(
                got,
                want["camera"]["position"][axis].as_f64().unwrap(),
                &tolerance["camera"]
            ));
        }

        // Resources: by assetId, sorted, with the identity the corpus declared (real hashes).
        let required: Vec<&str> = plan.required_asset_ids();
        let want_ids: Vec<&str> = expected["resources"]["requiredAssetIds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap())
            .collect();
        assert_eq!(required, want_ids, "frame {frame}");
        for resource in &plan.resources {
            let declared = &expected["assets"][resource.asset_id.as_str()];
            assert_eq!(resource.content_hash, declared["sha256"].as_str().unwrap());
            assert_eq!(resource.relative_path, declared["file"].as_str().unwrap());
            let users: Vec<String> = serde_json::from_value(
                expected["resources"]["usedBy"][resource.asset_id.as_str()].clone(),
            )
            .unwrap();
            let mut got = resource.used_by.clone();
            got.sort();
            assert_eq!(got, users, "frame {frame}: users of {}", resource.asset_id);
        }

        // Wire form round-trips and a repeated frame is the same plan.
        let wire = plan.to_json_string();
        assert_eq!(ScenePlan::from_json_str(&wire).unwrap(), plan);
        if let Some(earlier) = by_frame.get(&frame) {
            assert_eq!(
                &plan, earlier,
                "frame {frame}: plan differs when asked again"
            );
        }
        by_frame.insert(frame, plan);
    }
}

#[test]
fn a_frame_outside_the_edit_is_a_typed_error_and_changes_nothing() {
    let mut state = attached(&temp("range"));
    let before = (
        state.to_document(None),
        state.history.to_json(),
        revision(&mut state),
    );
    let error = plan_of(&state, 96).unwrap_err();
    assert_eq!(error.code(), "frame_out_of_range");
    assert_eq!(
        plan_of(&state, -1).unwrap_err().code(),
        "frame_out_of_range"
    );
    assert_eq!(
        err(
            &mut state,
            json!({"op":"scene.evaluate","scene":SCENE,"frame":96})
        )
        .code,
        "frame_out_of_range"
    );
    let after = (
        state.to_document(None),
        state.history.to_json(),
        revision(&mut state),
    );
    assert_eq!(before, after);
}

#[test]
fn selection_gesture_edit_undo_save_and_reopen_form_one_consistent_flow() {
    let expected = expected();
    let tolerance = &expected["tolerances"];
    let edit = &expected["edit"];
    let gesture = &edit["gesture"];
    let property = gesture["property"].as_str().unwrap();
    let target = gesture["target"].as_str().unwrap();
    let probe_frames: Vec<i64> = gesture["probeFrames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|frame| frame.as_i64().unwrap())
        .collect();
    let workspace = temp("flow");
    let mut state = attached(&workspace);
    let attached_depth = undo_depth(&mut state);
    let attached_revision = revision(&mut state);

    // Selection is session state: no undo entry, no revision.
    ok(
        &mut state,
        json!({"op":"scene.select","scene":SCENE,"entities":edit["select"]}),
    );
    assert!(
        ok(&mut state, json!({"op":"scene.selection"}))
            .to_string()
            .contains(target),
        "the selection holds the hero"
    );
    assert_eq!(undo_depth(&mut state), attached_depth);
    assert_eq!(revision(&mut state), attached_revision);

    // A cancelled gesture leaves nothing.
    let base = ok(
        &mut state,
        json!({"op":"scene.get","scene":SCENE,"property":property}),
    )["value"]
        .clone();
    let begun = ok(
        &mut state,
        json!({"op":"scene.gesture.begin","scene":SCENE,"targets":[target]}),
    );
    let id = begun["gesture"].as_str().unwrap().to_string();
    ok(
        &mut state,
        json!({"op":"scene.gesture.update","gesture":id,"ops":[{"set":property,"value":gesture["updates"][0]}]}),
    );
    ok(
        &mut state,
        json!({"op":"scene.gesture.cancel","gesture":id}),
    );
    assert_eq!(undo_depth(&mut state), attached_depth);
    assert_eq!(
        ok(
            &mut state,
            json!({"op":"scene.get","scene":SCENE,"property":property})
        )["value"],
        base
    );

    // A committed gesture: the preview is a candidate, never the document; commit is one entry.
    let begun = ok(
        &mut state,
        json!({"op":"scene.gesture.begin","scene":SCENE,"targets":[target]}),
    );
    let id = begun["gesture"].as_str().unwrap().to_string();
    ok(
        &mut state,
        json!({"op":"scene.gesture.update","gesture":id,"ops":[{"set":property,"value":gesture["updates"][0]}]}),
    );
    assert_eq!(undo_depth(&mut state), attached_depth);
    assert_eq!(revision(&mut state), attached_revision);
    for frame in &probe_frames {
        let preview = evaluate(&mut state, *frame, Some(&id));
        assert_eq!(preview["source"], "gesture");
        assert!(matrix_close(
            &world(&preview, "hero"),
            &gesture["candidateAfterFirstUpdate"][frame.to_string()],
            &tolerance["worldMatrix"]
        ));
        let document = evaluate(&mut state, *frame, None);
        assert!(matrix_close(
            &world(&document, "hero"),
            &frame_of(&expected, *frame)["worldMatrices"]["hero"],
            &tolerance["worldMatrix"]
        ));
    }
    ok(
        &mut state,
        json!({"op":"scene.gesture.update","gesture":id,"ops":[{"set":property,"value":gesture["updates"][1]}]}),
    );
    ok(
        &mut state,
        json!({"op":"scene.gesture.commit","gesture":id}),
    );
    assert_eq!(
        undo_depth(&mut state),
        attached_depth + 1,
        "one entry for the whole gesture"
    );
    for frame in &probe_frames {
        let answer = evaluate(&mut state, *frame, None);
        assert!(matrix_close(
            &world(&answer, "hero"),
            &gesture["afterCommit"][frame.to_string()],
            &tolerance["worldMatrix"]
        ));
        assert!(
            matrix_close(
                &world(&answer, "echo"),
                &frame_of(&expected, *frame)["worldMatrices"]["echo"],
                &tolerance["worldMatrix"]
            ),
            "an untouched node must not move"
        );
        // The plan of the same frame sees the same committed edit.
        let plan = plan_of(&state, *frame).unwrap();
        let hero = plan.node("hero").unwrap();
        assert!(matrix_close(
            &hero.world_matrix.0,
            &gesture["afterCommit"][frame.to_string()],
            &tolerance["worldMatrix"]
        ));
    }

    // A property edit: dry-run first, then the batch.
    let fov = &edit["fov"];
    let fov_property = fov["property"].as_str().unwrap();
    let dry = ok(
        &mut state,
        json!({"op":"scene.edit","scene":SCENE,"dry_run":true,"ops":[{"set":fov_property,"value":fov["value"]}]}),
    );
    assert_eq!(dry["changed"], false);
    assert_eq!(undo_depth(&mut state), attached_depth + 1);
    ok(
        &mut state,
        json!({"op":"scene.edit","scene":SCENE,"ops":[{"set":fov_property,"value":fov["value"]}]}),
    );
    assert_eq!(undo_depth(&mut state), attached_depth + 2);
    for frame in [0_i64, 60] {
        let camera = evaluate(&mut state, frame, None)["camera"].clone();
        assert!(close(
            camera["fovDeg"].as_f64().unwrap(),
            fov["afterEdit"][frame.to_string()].as_f64().unwrap(),
            &tolerance["camera"]
        ));
    }
    let moved = evaluate(&mut state, 60, None)["camera"]["position"].clone();
    for axis in ["x", "y", "z"] {
        assert!(close(
            moved[axis].as_f64().unwrap(),
            fov["camera"]["60"]["position"][axis].as_f64().unwrap(),
            &tolerance["camera"]
        ));
    }

    // Rejected input is typed and leaves state, history and revision as they were.
    let snapshot = (
        state.to_document(None),
        state.history.to_json(),
        revision(&mut state),
    );
    let refused = err(
        &mut state,
        json!({"op":"scene.edit","scene":SCENE,"ops":[{"set":fov_property,"value":200}]}),
    );
    assert!(!refused.code.is_empty());
    let unknown = err(
        &mut state,
        json!({"op":"scene.get","scene":"nope","property":property}),
    );
    assert!(!unknown.code.is_empty());
    assert_eq!(
        snapshot,
        (
            state.to_document(None),
            state.history.to_json(),
            revision(&mut state)
        )
    );

    // Each undo takes exactly one step.
    ok(&mut state, json!({"op":"history.undo"}));
    assert!(close(
        evaluate(&mut state, 60, None)["camera"]["fovDeg"]
            .as_f64()
            .unwrap(),
        frame_of(&expected, 60)["camera"]["fovDeg"]
            .as_f64()
            .unwrap(),
        &tolerance["camera"]
    ));
    assert!(matrix_close(
        &world(&evaluate(&mut state, 20, None), "hero"),
        &gesture["afterCommit"]["20"],
        &tolerance["worldMatrix"]
    ));
    ok(&mut state, json!({"op":"history.undo"}));
    assert!(matrix_close(
        &world(&evaluate(&mut state, 20, None), "hero"),
        &frame_of(&expected, 20)["worldMatrices"]["hero"],
        &tolerance["worldMatrix"]
    ));
    ok(&mut state, json!({"op":"history.redo"}));
    ok(&mut state, json!({"op":"history.redo"}));

    // Save, then open from the file in a fresh state: nothing may come from memory.
    let document = workspace.join("flow.tessel.json");
    ok(&mut state, json!({"op":"project.save","path":document}));
    let stored = read_json(&document);
    assert_eq!(stored["schema_version"], 2);
    let item = &stored["scenes"]["items"][0];
    assert_eq!(item["id"], SCENE);
    assert!(
        item["bundle"]["assets"]
            .as_array()
            .unwrap()
            .iter()
            .all(|asset| !asset["relativePath"].as_str().unwrap().starts_with('/')),
        "resources are named relative, never by an absolute path"
    );

    let mut reopened = EditorState::empty(&ProjectSettings::default()).unwrap();
    ok(&mut reopened, json!({"op":"project.open","path":document}));
    for frame in &probe_frames {
        assert!(matrix_close(
            &world(&evaluate(&mut reopened, *frame, None), "hero"),
            &gesture["afterCommit"][frame.to_string()],
            &tolerance["worldMatrix"]
        ));
        assert_eq!(
            plan_of(&reopened, *frame).unwrap().to_json(),
            plan_of(&state, *frame).unwrap().to_json(),
            "frame {frame}: the reopened scene plans exactly like the saved one"
        );
    }
    let again = workspace.join("again.tessel.json");
    ok(&mut reopened, json!({"op":"project.save","path":again}));
    assert_eq!(
        read_json(&again)["scenes"],
        stored["scenes"],
        "reopen then save keeps the scene block"
    );

    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn the_swap_bundle_changes_content_identity_but_not_the_geometry() {
    let expected = expected();
    let tolerance = &expected["tolerances"];
    let plain =
        DirectedScene::import_str(&fs::read_to_string(corpus().join("bundle.json")).unwrap())
            .unwrap();
    let swap =
        DirectedScene::import_str(&fs::read_to_string(corpus().join("bundle.swap.json")).unwrap())
            .unwrap();
    for frame in [0_i64, 20, 60] {
        let a = ScenePlan::build(&plain, frame).unwrap();
        let b = ScenePlan::build(&swap, frame).unwrap();
        assert_eq!(
            a.required_asset_ids(),
            b.required_asset_ids(),
            "the assetId set is the same"
        );
        assert_eq!(
            a.layers, b.layers,
            "layers are the same planes in the same order"
        );
        for (left, right) in a.nodes.iter().zip(&b.nodes) {
            assert!(matrix_close(
                &left.world_matrix.0,
                &json!(right.world_matrix.0),
                &tolerance["worldMatrix"]
            ));
        }
        let old = a.resource("hero").unwrap();
        let new = b.resource("hero").unwrap();
        assert_ne!(old.content_hash, new.content_hash);
        assert_ne!(old.revision_id, new.revision_id);
        assert_ne!(old.relative_path, new.relative_path);
        assert_eq!(
            new.content_hash,
            expected["swapAsset"]["sha256"].as_str().unwrap()
        );
        assert_eq!(
            a.resource("sky"),
            b.resource("sky"),
            "untouched assets keep their identity"
        );
    }
}

/// The shared operation returns the same evaluated matrices as the independent model.
#[test]
fn scene_plan_is_available_as_an_operation_and_matches_the_model() {
    let expected = expected();
    let mut state = attached(&temp("blocked-plan"));
    let answer = run(
        &mut state,
        json!({"op":"scene.plan","scene":SCENE,"frame":20}),
    );
    let Ok(answer) = answer else {
        panic!("BLOCKED BL063: {:?}", answer.unwrap_err());
    };
    let plan = &answer["plan"];
    let want = frame_of(&expected, 20);
    for node in plan["nodes"].as_array().unwrap() {
        let id = node["id"].as_str().unwrap();
        assert!(matrix_close(
            &json_matrix(&node["worldMatrix"]),
            &want["worldMatrices"][id],
            &expected["tolerances"]["worldMatrix"]
        ));
    }
}

#[test]
fn scene_plan_uses_a_live_gesture_candidate_without_committing_it() {
    let workspace = temp("candidate-plan");
    let mut state = attached(&workspace);
    let expected = expected();
    let gesture = &expected["edit"]["gesture"];
    let before = ok(
        &mut state,
        json!({"op":"scene.plan","scene":SCENE,"frame":20}),
    );
    let depth = undo_depth(&mut state);
    let begin = ok(
        &mut state,
        json!({"op":"scene.gesture.begin","scene":SCENE,"targets":[gesture["target"]]}),
    );
    ok(
        &mut state,
        json!({"op":"scene.gesture.update","gesture":begin["gesture"],"ops":[{"set":gesture["property"],"value":gesture["updates"][0]}]}),
    );
    let candidate = ok(
        &mut state,
        json!({"op":"scene.plan","scene":SCENE,"frame":20,"gesture":begin["gesture"]}),
    );
    assert_ne!(candidate["plan"]["nodes"], before["plan"]["nodes"]);
    assert_eq!(
        ok(
            &mut state,
            json!({"op":"scene.plan","scene":SCENE,"frame":20})
        ),
        before
    );
    assert_eq!(undo_depth(&mut state), depth);
    ok(
        &mut state,
        json!({"op":"scene.gesture.cancel","gesture":begin["gesture"]}),
    );
    assert!(
        run(
            &mut state,
            json!({"op":"scene.plan","scene":SCENE,"frame":20,"gesture":begin["gesture"]})
        )
        .is_err()
    );
    fs::remove_dir_all(workspace).unwrap();
}
