//! BL100a: what attaching a scene reports and budgets is measured by `AdmittedScene::prepare`
//! (a job thread), not by the attach (the window's thread).
//!
//! Profile that chose this (debug core, 24 MiB bundle; `scene_attach_cost.rs` repeats it): of the
//! ~530 ms the attach took, ~517 ms was serializing the document to learn its byte size for the
//! scene budget. On a bundle of 90 000 nodes (30 MB) the attach also spent ~150 ms scanning the
//! nodes for deferred families and ~50 ms building one notice per unknown field. Snapshotting and
//! the history entry cost microseconds.
//!
//! What these tests prove, through the public library only (no window, no GPU, no media crate):
//! the prepared facts are exactly what an independent computation from the bundle gives; every
//! source (value, text, file, prepared) attaches with the same answer and the same document; the
//! byte budget is enforced from the measured size and refuses atomically; and attaching a
//! prepared scene no longer scales with the size of the bundle.

use core::prelude::v1::test;

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::jobs::CancelToken;
use tessel_editor_core::scene_project::{
    AdmittedScene, MAX_SCENE_BYTES, SceneCommand, SceneSource, deferred_in_use,
};
use tessel_editor_core::{Command, CommandError, EditorState, headless};

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/migration/fixtures/bundles"
);

fn fixture(name: &str) -> Value {
    let path = PathBuf::from(FIXTURES).join(name);
    serde_json::from_slice(&fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
        .unwrap()
}

fn project() -> EditorState {
    let mut state = EditorState::demo();
    headless::execute(&mut state, &json!({"op":"project.create","name":"facts"})).unwrap();
    state
}

fn prepare(source: SceneSource) -> Result<AdmittedScene, CommandError> {
    AdmittedScene::prepare(source, &CancelToken::new())
}

fn attach(state: &mut EditorState, source: SceneSource, id: &str) -> Result<Value, CommandError> {
    state.execute(Command::Scene(SceneCommand::Attach {
        id: Some(id.into()),
        name: None,
        source,
    }))
}

fn undo_depth(state: &mut EditorState) -> u64 {
    headless::execute(state, &json!({"op":"history.status"})).unwrap()["history"]["undo_depth"]
        .as_u64()
        .unwrap()
}

fn compact_len(document: &Value) -> usize {
    serde_json::to_vec(document).unwrap().len()
}

/// The `counts` of an attach answer computed from the raw JSON alone, with none of the typed
/// model the core uses, so the comparison does not share a bug with it.
fn raw_counts(document: &Value) -> Value {
    let len = |key: &str| document[key].as_array().map_or(0, Vec::len);
    let nodes: usize = document["sceneDefinitions"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|definition| definition["nodes"].as_array().map_or(0, Vec::len))
        .sum();
    json!({
        "definitions": len("sceneDefinitions"), "nodes": nodes, "instances": len("sceneInstances"),
        "shots": len("shots"), "camera_clips": len("cameraClips"),
        "performance_tracks": len("performanceTracks"), "assets": len("assets"),
    })
}

/// A valid bundle that also uses deferred families and carries fields the importer does not
/// know (each is a notice), so the reported lists are not empty.
fn busy() -> Value {
    let mut bundle = fixture("valid-minimal-continuous.bundle.json");
    bundle["narration"] = json!({"lines": ["kept verbatim"]});
    bundle["x-extension"] = json!({"anything": [1, 2, 3]});
    bundle["sceneDefinitions"][0]["lighting"] = json!({"key": "sun"});
    bundle["sceneDefinitions"][0]["nodes"][0]["surface"] = json!({"roughness": 0.5});
    bundle
}

fn padded(bytes: usize) -> Value {
    let mut bundle = fixture("valid-minimal-continuous.bundle.json");
    bundle["x-padding"] = json!("0".repeat(bytes));
    bundle
}

#[test]
fn the_facts_of_a_prepared_scene_are_what_the_bundle_says() {
    let mut fixtures: Vec<Value> = [
        "valid-camera-recipes-layers.bundle.json",
        "valid-cut-instance-offsets.bundle.json",
        "valid-graph-sockets-performance.bundle.json",
        "valid-minimal-continuous.bundle.json",
    ]
    .iter()
    .map(|name| fixture(name))
    .collect();
    fixtures.push(busy());

    for (index, bundle) in fixtures.into_iter().enumerate() {
        let imported = DirectedScene::import_value(bundle.clone()).unwrap();
        let admitted = prepare(SceneSource::Bundle(bundle.clone())).unwrap();
        assert_eq!(
            admitted.serialized_len(),
            compact_len(imported.document()),
            "fixture {index}: the size the budget counts is the compact document"
        );

        let mut state = project();
        let report = attach(&mut state, SceneSource::Admitted(admitted), "scene").unwrap();
        assert_eq!(
            report["counts"],
            raw_counts(imported.document()),
            "fixture {index}"
        );
        assert_eq!(
            report["deferred_in_use"],
            json!(deferred_in_use(imported.document())),
            "fixture {index}"
        );
        let notices: Vec<Value> = imported
            .notices()
            .iter()
            .map(|n| json!({"code": n.code, "path": n.path, "message": n.message}))
            .collect();
        assert_eq!(report["notices"], json!(notices), "fixture {index}");
        assert_eq!(report["bundle_id"], imported.bundle().bundle_id);
        assert_eq!(report["revision_id"], imported.bundle().revision_id);
        assert_eq!(report["changed"], true);
    }

    // The busy bundle really exercises the lists (otherwise the equalities above are vacuous).
    let mut state = project();
    let report = attach(&mut state, SceneSource::Bundle(busy()), "busy").unwrap();
    assert!(
        !report["notices"].as_array().unwrap().is_empty(),
        "{report}"
    );
    assert!(
        !report["deferred_in_use"].as_array().unwrap().is_empty(),
        "{report}"
    );
}

#[test]
fn every_source_attaches_with_the_same_answer_the_same_document_and_one_undo_step() {
    let bundle = busy();
    let dir = std::env::temp_dir().join(format!("tessel-facts-sources-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("busy.bundle.json");
    fs::write(&path, serde_json::to_vec(&bundle).unwrap()).unwrap();

    let sources = [
        ("value", SceneSource::Bundle(bundle.clone())),
        (
            "text",
            SceneSource::Text(serde_json::to_string(&bundle).unwrap()),
        ),
        ("path", SceneSource::Path(path)),
        (
            "prepared",
            SceneSource::Admitted(prepare(SceneSource::Bundle(bundle.clone())).unwrap()),
        ),
    ];
    let mut outcomes = Vec::new();
    for (label, source) in sources {
        let mut state = project();
        let report = attach(&mut state, source, "scene").unwrap();
        assert_eq!(undo_depth(&mut state), 1, "{label}: one undo entry");
        outcomes.push((label, report, state.to_document(None)["scenes"].clone()));
        headless::execute(&mut state, &json!({"op":"history.undo"})).unwrap();
        assert!(state.scenes.is_empty(), "{label}: undo removes it");
    }
    let (first, rest) = outcomes.split_first().unwrap();
    for other in rest {
        assert_eq!(other.1, first.1, "{} answers like {}", other.0, first.0);
        assert_eq!(other.2, first.2, "{} stores like {}", other.0, first.0);
    }
}

#[test]
fn a_prepared_scene_refuses_what_attach_refuses_and_changes_nothing() {
    let mut state = project();
    attach(&mut state, SceneSource::Bundle(busy()), "taken").unwrap();
    let before = (
        state.to_document(None),
        state.project.revision,
        undo_depth(&mut state),
    );
    let prepared = prepare(SceneSource::Bundle(busy())).unwrap();
    let direct = |state: &mut EditorState, id: &str| attach(state, SceneSource::Bundle(busy()), id);
    let via_prepared = |state: &mut EditorState, id: &str| {
        attach(state, SceneSource::Admitted(prepared.clone()), id)
    };
    for id in ["taken", "bad id!"] {
        let expected = direct(&mut state, id).unwrap_err();
        let actual = via_prepared(&mut state, id).unwrap_err();
        assert_eq!(
            (actual.code, &actual.message),
            (expected.code, &expected.message),
            "{id}"
        );
    }
    let after = (
        state.to_document(None),
        state.project.revision,
        undo_depth(&mut state),
    );
    assert_eq!(after, before, "nothing changed");
}

/// The budget is the sum of measured sizes: the attach that would pass the limit is refused with
/// the exact total, atomically, and the accounting is still right afterwards.
#[test]
fn the_budget_is_enforced_from_the_measured_size_and_refuses_atomically() {
    // The importer allows 32 MiB per bundle; four of ~30 MB fit in 128 MiB and a fifth does not.
    let bundle = padded(30 * 1024 * 1024);
    let admitted = prepare(SceneSource::Bundle(bundle)).unwrap();
    let one = admitted.serialized_len();
    assert!(
        one * 4 <= MAX_SCENE_BYTES && one * 5 > MAX_SCENE_BYTES,
        "{one}"
    );

    let mut state = project();
    // The same prepared scene under four IDs shares one bundle in memory.
    for n in 0..4 {
        attach(
            &mut state,
            SceneSource::Admitted(admitted.clone()),
            &format!("s{n}"),
        )
        .unwrap();
    }
    let before = (
        state.to_document(None),
        state.project.revision,
        undo_depth(&mut state),
    );

    let refused = attach(&mut state, SceneSource::Admitted(admitted.clone()), "s4").unwrap_err();
    assert_eq!(refused.code, "scenes_too_large", "{refused:?}");
    assert!(
        refused
            .message
            .contains(&format!("would make it {}", one * 5)),
        "the total is the sum of the measured sizes: {}",
        refused.message
    );
    assert_eq!(refused.target.as_deref(), Some("s4"));
    let after = (
        state.to_document(None),
        state.project.revision,
        undo_depth(&mut state),
    );
    assert_eq!(after, before, "a refused attach changes nothing");

    // The accounting follows detach and undo: room for exactly one more appears, no more.
    headless::execute(&mut state, &json!({"op":"scene.detach","scene":"s0"})).unwrap();
    attach(&mut state, SceneSource::Admitted(admitted.clone()), "s4").unwrap();
    let again = attach(&mut state, SceneSource::Admitted(admitted), "s5").unwrap_err();
    assert_eq!(again.code, "scenes_too_large");
}

/// Before BL100a the attach cost O(bundle) (it serialized the document); now it must not. Best of
/// a few replicas on both sides so a busy machine does not decide the outcome, and relative to the
/// preparation measured in the same process so the check holds in debug and release alike. Before
/// BL100a the attach (~530 ms in debug) took six times longer than the parse it followed (~85 ms);
/// preparing now includes the measuring, and the attach has to take under a tenth of it.
#[test]
fn attaching_a_prepared_scene_does_not_scale_with_the_bundle() {
    let bundle = padded(24 * 1024 * 1024);
    let text = serde_json::to_string(&bundle).unwrap();
    let mut preparing = Duration::MAX;
    let mut admitted = None;
    for _ in 0..3 {
        let started = Instant::now();
        let scene = prepare(SceneSource::Text(text.clone())).unwrap();
        preparing = preparing.min(started.elapsed());
        admitted = Some(scene);
    }
    let admitted = admitted.unwrap();

    let mut attaching = Duration::MAX;
    for round in 0..5 {
        let mut state = project();
        let started = Instant::now();
        attach(&mut state, SceneSource::Admitted(admitted.clone()), "big").unwrap();
        attaching = attaching.min(started.elapsed());
        assert_eq!(state.scenes.len(), 1, "round {round}");
    }
    assert!(
        attaching * 10 < preparing,
        "attaching took {attaching:?} against {preparing:?} to prepare: the attach is walking the bundle again"
    );
}
