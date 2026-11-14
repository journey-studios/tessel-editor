//! L and H (public surfaces): every door a user, a file, a project or another process can reach
//! keeps refusing a non-empty `fx.transitions` with `FX_TRANSITION_NOT_YET`, exactly as before this
//! delivery, and a refusal leaves the document, the revision and the undo history untouched.
//!
//! The admitting importer of `support::admit` is the only way these tests read a transition; none
//! of the doors below can reach it.

use std::io::Write;
use std::process::{Command as Process, Stdio};

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::fx::{self, TransitionAdmission};
use tessel_editor_core::directed_scene::{DirectedScene, Limits};
use tessel_editor_core::scene_edit::{apply, apply_fx, parse_batch, parse_fx_batch};
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::{EditorState, headless};

use crate::support::*;

const NOT_YET: &str =
    "transitions between shots are not implemented yet; leave `transitions` empty";

fn document_with_transitions() -> Value {
    with_transitions(Edit::spec().bundle(), vec![tr(json!({}))])
}

fn assert_not_yet(error: &tessel_editor_core::directed_scene::ImportError) {
    assert_eq!(codes(error), [fx::FX_TRANSITION_NOT_YET]);
    let issue = &error.issues()[0];
    assert_eq!(issue.path, "/fx/transitions");
    assert_eq!(issue.message, NOT_YET);
}

#[test]
fn every_public_importer_refuses_a_transition_with_the_message_it_always_had() {
    let value = document_with_transitions();
    let text = value.to_string();
    let limits = Limits::default();
    assert_not_yet(&DirectedScene::import_value(value.clone()).err().unwrap());
    assert_not_yet(
        &DirectedScene::import_value_with(value.clone(), &limits)
            .err()
            .unwrap(),
    );
    assert_not_yet(&DirectedScene::import_str(&text).err().unwrap());
    assert_not_yet(
        &DirectedScene::import_str_with(&text, &limits)
            .err()
            .unwrap(),
    );
    assert!(DirectedScene::validate_str(&text).is_err());
    // Even a transition that would be wrong in every other way says only this.
    let broken = with_transitions(
        Edit::spec().bundle(),
        vec![json!({"id": 3}), json!({"effect": "vignette"}), json!("x")],
    );
    assert_not_yet(&DirectedScene::import_value(broken).err().unwrap());
    // The empty list stays accepted by every public importer; the key stays required, as it was
    // (the same answer under both policies).
    let mut bundle = Edit::spec().bundle();
    bundle["fx"] = json!({"catalogVersion": 1, "tracks": [], "transitions": []});
    assert!(DirectedScene::import_value(bundle.clone()).is_ok());
    assert!(admit(bundle).is_ok());
    let mut bundle = Edit::spec().bundle();
    bundle["fx"] = json!({"catalogVersion": 1, "tracks": []});
    let public = DirectedScene::import_value(bundle.clone()).err().unwrap();
    assert_eq!(codes(&public), ["MISSING_FIELD"]);
    assert_eq!(codes(&admit(bundle).err().unwrap()), ["MISSING_FIELD"]);
}

#[test]
fn the_default_policy_is_refuse_and_it_is_recorded_on_the_scene() {
    assert_eq!(TransitionAdmission::default(), TransitionAdmission::Refuse);
    let scene = DirectedScene::import_str(MINIMAL).unwrap();
    assert_eq!(scene.admission(), TransitionAdmission::Refuse);
    assert_eq!(
        admit_str(MINIMAL).unwrap().admission(),
        TransitionAdmission::Accept
    );
}

#[test]
fn the_edit_candidate_inherits_the_policy_of_its_scene() {
    let accepted = admit(document_with_transitions()).unwrap();
    let refused = DirectedScene::import_str(CUT).unwrap();
    let look = json!([{"op": "add_clip", "effect": "vignette", "start_frame": 0,
                       "duration_frames": 10, "id": "v1"}]);
    let property = json!([{"set": "scene/scene-a/node/backdrop/width", "value": 2}]);

    // Accept stays Accept, and the transition survives the edit: a candidate that dropped the
    // policy would refuse its own document (FX_TRANSITION_NOT_YET).
    let outcome = apply_fx(&accepted, &parse_fx_batch(&look).unwrap()).unwrap();
    assert_eq!(outcome.candidate.admission(), TransitionAdmission::Accept);
    assert_eq!(outcome.candidate.bundle().fx.transitions.len(), 1);
    assert_eq!(outcome.candidate.bundle().fx.transitions[0].id, "tr");
    let outcome = apply(&accepted, &parse_batch(&property).unwrap()).unwrap();
    assert_eq!(outcome.candidate.admission(), TransitionAdmission::Accept);
    assert_eq!(outcome.candidate.bundle().fx.transitions.len(), 1);

    // Refuse stays Refuse: an edit never opens the gate.
    let outcome = apply_fx(&refused, &parse_fx_batch(&look).unwrap()).unwrap();
    assert_eq!(outcome.candidate.admission(), TransitionAdmission::Refuse);
    let outcome = apply(&refused, &parse_batch(&property).unwrap()).unwrap();
    assert_eq!(outcome.candidate.admission(), TransitionAdmission::Refuse);
    assert!(outcome.candidate.bundle().fx.transitions.is_empty());
}

// ---- the executor every adapter shares ------------------------------------------------------

fn ok(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn state_with_scene() -> EditorState {
    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    ok(
        &mut state,
        json!({"op": "scene.attach", "id": "street", "bundle": bundle_of(CUT)}),
    );
    state
}

fn snapshot(state: &mut EditorState) -> (Value, Value, Value) {
    (
        state.scenes.get("street").unwrap().scene.document().clone(),
        ok(state, json!({"op": "status"}))["persistence"]["revision"].clone(),
        ok(state, json!({"op": "history.status"})),
    )
}

#[test]
fn attach_and_edit_through_the_executor_refuse_transitions_and_change_nothing() {
    let mut state = state_with_scene();
    let before = snapshot(&mut state);

    // Attach: the bundle, the text and a second ID.
    let with = document_with_transitions();
    for request in [
        json!({"op": "scene.attach", "id": "other", "bundle": with}),
        json!({"op": "scene.attach", "id": "other", "text": with.to_string()}),
    ] {
        let error = headless::execute(&mut state, &request).unwrap_err();
        assert_eq!(error.code, "invalid_bundle", "{error:?}");
        let issues = error.details.as_ref().unwrap()["issues"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(issues[0]["code"], fx::FX_TRANSITION_NOT_YET, "{issues:?}");
        assert_eq!(issues[0]["message"], NOT_YET);
    }
    assert!(state.scenes.get("other").is_none());
    assert_eq!(snapshot(&mut state), before);

    // Edit: a transition cannot be written by hand through `scene.fx.edit` either (the ops of the
    // layer know no transition), and a failed op leaves the revision where it was.
    let error = headless::execute(
        &mut state,
        &json!({"op": "scene.fx.edit", "scene": "street",
                "ops": [{"op": "add_clip", "effect": "crossDissolve", "start_frame": 0}]}),
    )
    .unwrap_err();
    assert_eq!(error.code, "unknown_effect", "{error:?}");
    assert_eq!(snapshot(&mut state), before);

    // The public catalog is the three looks and nothing else.
    let catalog = ok(&mut state, json!({"op": "scene.fx.catalog"}));
    let ids: Vec<&str> = catalog["effects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|effect| effect["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["vignette", "fade", "filmGrain"]);
    assert_eq!(catalog["catalog_version"], 1);
}

#[test]
fn a_real_headless_process_refuses_transitions_and_keeps_its_session() {
    let with = document_with_transitions();
    let requests = [
        json!({"op": "scene.attach", "id": "street", "bundle": bundle_of(CUT)}),
        json!({"op": "scene.fx.catalog"}),
        json!({"op": "status"}),
        json!({"op": "history.status"}),
        json!({"op": "scene.attach", "id": "other", "bundle": with}),
        json!({"op": "scene.attach", "id": "other", "text": with.to_string()}),
        json!({"op": "scene.list"}),
        json!({"op": "status"}),
        json!({"op": "history.status"}),
        json!({"op": "scene.fx.catalog"}),
    ];
    let input = requests
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");

    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    let mut library = Vec::new();
    let failed = headless::run_session(&mut state, None, input.as_bytes(), &mut library).unwrap();
    assert!(failed, "the library reports the refused requests");

    let mut child = Process::new(env!("CARGO_BIN_EXE_tessel-headless"))
        .arg("--empty")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    // A session with a refused request exits 1 (the executable's contract), after answering all.
    assert_eq!(output.status.code(), Some(1));

    let lines = |bytes: &[u8]| -> Vec<Value> {
        String::from_utf8_lossy(bytes)
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    };
    let answers = lines(&output.stdout);
    // Two sessions are two projects: their random ID and session counter are the only difference.
    let scrub = |mut answers: Vec<Value>| {
        fn walk(value: &mut Value) {
            match value {
                Value::Object(map) => {
                    for (key, inner) in map.iter_mut() {
                        if key == "project_id" || key == "session" {
                            *inner = Value::Null;
                        } else if key == "project" && inner.is_object() {
                            inner["id"] = Value::Null;
                            walk(inner);
                        } else {
                            walk(inner);
                        }
                    }
                }
                Value::Array(items) => items.iter_mut().for_each(walk),
                _ => {}
            }
        }
        answers.iter_mut().for_each(walk);
        answers
    };
    assert_eq!(
        scrub(answers.clone()),
        scrub(lines(&library)),
        "the process answers like the library"
    );
    assert_eq!(answers.len(), requests.len());

    assert_eq!(answers[0]["ok"], true, "{:?}", answers[0]);
    let looks = |answer: &Value| -> Vec<String> {
        answer["result"]["effects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|effect| effect["id"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(looks(&answers[1]), ["vignette", "fade", "filmGrain"]);
    assert_eq!(looks(&answers[9]), looks(&answers[1]));
    for index in [4, 5] {
        assert_eq!(answers[index]["ok"], false, "{:?}", answers[index]);
        assert_eq!(answers[index]["error"]["code"], "invalid_bundle");
        assert_eq!(
            answers[index]["error"]["details"]["issues"][0]["code"],
            fx::FX_TRANSITION_NOT_YET
        );
    }
    // Nothing moved: same scenes, same revision, same history.
    let scene_ids = |answer: &Value| answer["result"].to_string();
    assert!(scene_ids(&answers[6]).contains("street"));
    assert!(!scene_ids(&answers[6]).contains("other"));
    assert_eq!(
        answers[7]["result"]["persistence"]["revision"],
        answers[2]["result"]["persistence"]["revision"]
    );
    assert_eq!(answers[8]["result"], answers[3]["result"]);
}
