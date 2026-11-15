//! F064 / P4b S3a: the transition ops through the real executor (`scene.fx.edit`, undo, redo,
//! gestures). An `Accept` scene can only be inserted from inside the crate, so these tests live
//! here, next to `admission_tests`; the public doors stay shut and are proved in
//! `tests/scene_fx_transition/public.rs`.

use core::prelude::v1::test;
use std::sync::Arc;

use serde_json::{Map, Value, json};

use super::AttachedScene;
use crate::directed_scene::fx::TransitionAdmission;
use crate::directed_scene::{DirectedScene, Limits};
use crate::state::ProjectSettings;
use crate::{EditorState, headless};

const CUT: &str = include_str!(
    "../../../../docs/migration/fixtures/bundles/valid-cut-instance-offsets.bundle.json"
);

fn bundle(list: Value) -> Value {
    let mut value: Value = serde_json::from_str(CUT).unwrap();
    value["fx"] = json!({"catalogVersion": 1, "tracks": [], "transitions": list});
    value
}

fn state_with(policy: TransitionAdmission, list: Value) -> EditorState {
    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    let scene =
        DirectedScene::import_value_admitting(bundle(list), &Limits::default(), policy).unwrap();
    state
        .scenes
        .insert(AttachedScene {
            id: "street".into(),
            name: "street".into(),
            scene: Arc::new(scene),
            extra: Map::new(),
        })
        .unwrap();
    state
}

fn existing() -> Value {
    json!([{"id": "tr", "effect": "crossDissolve", "fromShotId": "shot-a",
            "toShotId": "shot-b", "durationFrames": 12, "alignment": "center",
            "handles": "hold", "easing": "easeInOutCubic"}])
}

fn accept(list: Value) -> EditorState {
    state_with(TransitionAdmission::Accept, list)
}

fn run(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn edit(ops: Value) -> Value {
    json!({"op": "scene.fx.edit", "scene": "street", "ops": ops})
}

fn document(state: &EditorState) -> Value {
    state.scenes.get("street").unwrap().scene.document().clone()
}

fn policy(state: &EditorState) -> TransitionAdmission {
    state.scenes.get("street").unwrap().scene.admission()
}

fn revision(state: &mut EditorState) -> Value {
    run(state, json!({"op": "status"}))["persistence"]["revision"].clone()
}

/// What an edit that did nothing must leave exactly as it was.
fn snapshot(state: &mut EditorState) -> (Value, Value, Value, TransitionAdmission) {
    (
        document(state),
        revision(state),
        run(state, json!({"op": "history.status"})),
        policy(state),
    )
}

fn set_default() -> Value {
    json!({"op": "set_transition", "fromShotId": "shot-a", "toShotId": "shot-b",
           "effect": "crossDissolve"})
}

#[test]
fn a_dry_run_reports_the_change_and_writes_nothing() {
    let mut state = accept(json!([]));
    let before = snapshot(&mut state);
    let mut request = edit(json!([set_default()]));
    request["dry_run"] = json!(true);
    let answer = run(&mut state, request);
    assert_eq!(answer["dry_run"], true);
    assert_eq!(answer["changed"], false);
    assert_eq!(answer["would_change"], true);
    assert_eq!(answer["changes"][0]["action"], "set_transition");
    assert_eq!(answer["changes"][0]["transition"], "tr-1");
    assert_eq!(answer["changes"][0]["noop"], false);
    assert_eq!(
        snapshot(&mut state),
        before,
        "document, revision, history and policy"
    );
    // The same batch for real is the same change.
    let real = run(&mut state, edit(json!([set_default()])));
    assert_eq!(real["changed"], true);
    assert_eq!(real["changes"], answer["changes"]);
}

#[test]
fn a_batch_of_looks_and_transitions_is_one_revision_and_one_undo_entry() {
    let mut state = accept(json!([]));
    let before = snapshot(&mut state);
    let answer = run(
        &mut state,
        edit(json!([
            {"op": "add_clip", "effect": "vignette", "start_frame": 0, "duration_frames": 10,
             "id": "v1"},
            set_default(),
            {"op": "set_transition_param", "id": "tr-1", "param": "curve", "value": "linear"},
            {"op": "set_transition_hidden", "id": "tr-1", "hidden": true},
        ])),
    );
    assert_eq!(answer["changed"], true);
    assert_eq!(answer["changed_ops"], 4);
    let after = snapshot(&mut state);
    assert_eq!(
        after.1.as_u64(),
        before.1.as_u64().map(|revision| revision + 1)
    );
    assert_eq!(
        after.2["undo_depth"].as_u64(),
        before.2["undo_depth"].as_u64().map(|n| n + 1)
    );
    assert_eq!(after.0["fx"]["transitions"][0]["params"]["curve"], "linear");
    assert_eq!(after.0["fx"]["transitions"][0]["hidden"], true);
    assert_eq!(after.3, TransitionAdmission::Accept);

    // One undo gives back the document before the batch, looks and transition together.
    run(&mut state, json!({"op": "history.undo"}));
    assert_eq!(document(&state), before.0);
    assert_eq!(policy(&state), TransitionAdmission::Accept);
    // And one redo gives the batch back.
    run(&mut state, json!({"op": "history.redo"}));
    assert_eq!(document(&state), after.0);
    assert_eq!(policy(&state), TransitionAdmission::Accept);
}

#[test]
fn undo_and_redo_walk_a_sequence_of_transition_edits_and_keep_the_policy() {
    let mut state = accept(existing());
    let mut documents = vec![document(&state)];
    for ops in [
        json!([{"op": "resize_transition", "id": "tr", "durationFrames": 20}]),
        json!([{"op": "set_transition_hidden", "id": "tr", "hidden": true}]),
        json!([{"op": "remove_transition", "id": "tr"}]),
    ] {
        run(&mut state, edit(ops));
        documents.push(document(&state));
    }
    assert!(documents.windows(2).all(|pair| pair[0] != pair[1]));
    for expected in documents.iter().rev().skip(1) {
        run(&mut state, json!({"op": "history.undo"}));
        assert_eq!(&document(&state), expected);
        assert_eq!(policy(&state), TransitionAdmission::Accept);
    }
    for expected in documents.iter().skip(1) {
        run(&mut state, json!({"op": "history.redo"}));
        assert_eq!(&document(&state), expected);
        assert_eq!(policy(&state), TransitionAdmission::Accept);
    }
}

#[test]
fn a_no_op_batch_changes_no_revision_and_leaves_no_undo_entry() {
    let mut state = accept(existing());
    let before = snapshot(&mut state);
    for ops in [
        json!([]),
        json!([{"op": "set_transition_hidden", "id": "tr", "hidden": false}]),
        json!([{"op": "resize_transition", "id": "tr", "durationFrames": 12}]),
        json!([
            {"op": "set_transition_hidden", "id": "tr", "hidden": true},
            {"op": "set_transition_hidden", "id": "tr", "hidden": false},
        ]),
    ] {
        let answer = run(&mut state, edit(ops.clone()));
        assert_eq!(answer["changed"], false, "{ops}");
        assert_eq!(answer["noop"], true, "{ops}");
        assert_eq!(snapshot(&mut state), before, "{ops}");
    }
}

#[test]
fn an_edit_built_on_an_old_revision_is_refused_whole() {
    let mut state = accept(existing());
    let stale = revision(&mut state);
    run(
        &mut state,
        edit(json!([{"op": "resize_transition", "id": "tr", "durationFrames": 20}])),
    );
    let before = snapshot(&mut state);
    let mut request = edit(json!([{"op": "set_transition_hidden", "id": "tr", "hidden": true}]));
    request["base_revision"] = stale.clone();
    let error = headless::execute(&mut state, &request).unwrap_err();
    assert_eq!(error.code, "stale_revision");
    assert_eq!(snapshot(&mut state), before);
    // The current revision is accepted.
    request["base_revision"] = before.1.clone();
    assert_eq!(run(&mut state, request)["changed"], true);
}

#[test]
fn a_numeric_round_trip_is_noop_in_dry_run_and_real_edit() {
    let mut list = existing();
    list[0]["params"] = json!({"intensity":1});
    let mut state = accept(list);
    let before = snapshot(&mut state);
    for dry_run in [true, false] {
        let mut request = edit(json!([
            {"op":"set_transition_param","id":"tr","param":"intensity","value":0.5},
            {"op":"set_transition_param","id":"tr","param":"intensity","value":1}
        ]));
        request["dry_run"] = json!(dry_run);
        let answer = run(&mut state, request);
        assert_eq!(answer["changed"], false);
        if dry_run {
            assert_eq!(answer["would_change"], false);
        } else {
            assert_eq!(answer["noop"], true);
        }
        assert_eq!(snapshot(&mut state), before);
    }
}

#[test]
fn transition_edits_report_the_candidate_budget_and_undo_restores_it() {
    let mut state = accept(json!([]));
    let looks: Vec<Value> = (0..8)
        .map(|i| {
            json!({
                "op":"add_clip", "id":format!("v{i}"), "effect":"vignette",
                "start_frame":0, "duration_frames":100
            })
        })
        .collect();
    let base = run(&mut state, edit(json!(looks)));
    assert_eq!(base["budget"]["peak_passes"], 8);
    assert_eq!(base["budget"]["over_budget"], false);
    let added = run(&mut state, edit(json!([set_default()])));
    assert_eq!(added["budget"]["peak_passes"], 9);
    assert_eq!(added["budget"]["over_budget"], true);
    assert!(!added["warnings"].as_array().unwrap().is_empty());
    for op in ["scene.fx.list", "scene.fx.inspect"] {
        assert_eq!(
            run(&mut state, json!({"op":op,"scene":"street"}))["budget"],
            added["budget"]
        );
    }
    let before = snapshot(&mut state);
    let mut removal = edit(json!([{"op":"remove_transition","id":"tr-1"}]));
    removal["dry_run"] = json!(true);
    let dry = run(&mut state, removal.clone());
    assert_eq!(dry["budget"], base["budget"]);
    assert_eq!(snapshot(&mut state), before);
    removal["dry_run"] = json!(false);
    assert_eq!(run(&mut state, removal)["budget"], base["budget"]);
    run(&mut state, json!({"op":"history.undo"}));
    assert_eq!(
        run(&mut state, json!({"op":"scene.fx.list","scene":"street"}))["budget"],
        added["budget"]
    );
}

#[test]
fn a_refusal_names_every_problem_in_english_and_changes_nothing() {
    let mut state = accept(existing());
    let before = snapshot(&mut state);

    // The importer refuses the candidate: `issues` as always, `errors` next to them.
    let error = headless::execute(
        &mut state,
        &edit(json!([{"op": "resize_transition", "id": "tr", "durationFrames": 900}])),
    )
    .unwrap_err();
    assert_eq!(error.code, "invalid_bundle");
    let details = error.details.as_ref().unwrap();
    let issues = details["issues"].as_array().unwrap();
    let errors = details["errors"].as_array().unwrap();
    assert_eq!(errors.len(), issues.len());
    assert_eq!(errors[0]["code"], issues[0]["code"]);
    assert_eq!(errors[0]["message"], issues[0]["message"]);
    assert_eq!(errors[0]["transitionId"], "tr");
    assert_eq!(snapshot(&mut state), before);

    // An op that fails by itself: the command code of the op and the problem of the table.
    let error = headless::execute(
        &mut state,
        &edit(json!([{"op": "remove_transition", "id": "ghost"}])),
    )
    .unwrap_err();
    assert_eq!(error.code, "unknown_transition");
    let errors = error.details.as_ref().unwrap()["errors"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0]["code"], "UNKNOWN_TRANSITION");
    assert_eq!(errors[0]["transitionId"], "ghost");
    assert!(errors[0]["hint"].as_str().unwrap().contains("tr"));
    assert_eq!(snapshot(&mut state), before);

    let error = headless::execute(
        &mut state,
        &edit(
            json!([{"op": "set_transition", "fromShotId": "shot-a", "toShotId": "shot-b",
                      "effect": "crossDissolve", "id": "renamed"}]),
        ),
    )
    .unwrap_err();
    assert_eq!(error.code, "transition_id_mismatch");
    assert_eq!(snapshot(&mut state), before);

    // A batch of looks only: no `errors` appear, the answer is the one it always was.
    let error = headless::execute(
        &mut state,
        &edit(json!([{"op": "remove_clip", "clip": "ghost"}])),
    )
    .unwrap_err();
    assert!(
        error
            .details
            .as_ref()
            .is_none_or(|details| details.get("errors").is_none())
    );
}

#[test]
fn a_batch_past_the_ceiling_is_refused_before_it_runs() {
    let mut state = accept(existing());
    let before = snapshot(&mut state);
    let ops = vec![json!({"op": "set_transition_hidden", "id": "tr", "hidden": true}); 1025];
    let error = headless::execute(&mut state, &edit(Value::Array(ops))).unwrap_err();
    assert_eq!(error.code, "fx_limit");
    assert_eq!(snapshot(&mut state), before);
    // 1024 is a batch (and here a no-op pair repeated, net hidden).
    let ops = vec![json!({"op": "set_transition_hidden", "id": "tr", "hidden": true}); 1024];
    let answer = run(&mut state, edit(Value::Array(ops)));
    assert_eq!(answer["changed"], true);
    assert_eq!(
        answer["changed_ops"], 1,
        "only the first op changed anything"
    );
}

#[test]
fn the_policy_survives_every_transition_edit_and_a_refuse_scene_is_never_opened() {
    let mut state = accept(json!([]));
    run(&mut state, edit(json!([set_default()])));
    run(
        &mut state,
        edit(json!([{"op": "resize_transition", "id": "tr-1", "durationFrames": 20}])),
    );
    assert_eq!(policy(&state), TransitionAdmission::Accept);
    run(&mut state, json!({"op": "history.undo"}));
    run(&mut state, json!({"op": "history.undo"}));
    assert_eq!(policy(&state), TransitionAdmission::Accept);

    // A public (Refuse) scene refuses a transition op: the candidate inherits the policy, no op
    // opens it, and nothing changes.
    let mut state = state_with(TransitionAdmission::Refuse, json!([]));
    let before = snapshot(&mut state);
    let error = headless::execute(&mut state, &edit(json!([set_default()]))).unwrap_err();
    assert_eq!(error.code, "invalid_bundle");
    let details = error.details.as_ref().unwrap();
    assert_eq!(details["issues"][0]["code"], "FX_TRANSITION_NOT_YET");
    assert_eq!(details["errors"][0]["code"], "FX_TRANSITION_NOT_YET");
    assert_eq!(snapshot(&mut state), before);
    assert_eq!(policy(&state), TransitionAdmission::Refuse);
    // The ops that need an existing transition say it does not exist, and still change nothing.
    let error = headless::execute(
        &mut state,
        &edit(json!([{"op": "remove_transition", "id": "tr"}])),
    )
    .unwrap_err();
    assert_eq!(error.code, "unknown_transition");
    assert_eq!(snapshot(&mut state), before);
}

#[test]
fn a_gesture_cannot_carry_a_transition_op() {
    let mut state = accept(json!([]));
    run(
        &mut state,
        edit(
            json!([{"op": "add_clip", "effect": "vignette", "start_frame": 0,
                     "duration_frames": 30, "id": "look"}]),
        ),
    );
    let begin = run(
        &mut state,
        json!({"op": "scene.fx.gesture.begin", "scene": "street", "clip": "look"}),
    );
    let gesture = begin["gesture"].as_str().unwrap().to_string();
    let open_gesture = |state: &mut EditorState| {
        let mut status = run(state, json!({"op": "scene.fx.gesture.status"}));
        // The refusal is recorded as the newest one, like every refused gesture op; the rest of
        // the gesture is what it was.
        let refusal = status["gesture"]["refusal"].take();
        (status, refusal)
    };
    let (status, _) = open_gesture(&mut state);
    let before = snapshot(&mut state);

    for op in [
        set_default(),
        json!({"op": "resize_transition", "id": "tr-1", "durationFrames": 8}),
        json!({"op": "remove_transition", "id": "tr-1"}),
    ] {
        let error = headless::execute(
            &mut state,
            &json!({"op": "scene.fx.gesture.update", "gesture": gesture, "ops": [op]}),
        )
        .unwrap_err();
        assert_eq!(error.code, "gesture_op_not_allowed", "{op}");
        // The refusal says what the op is, not the generic "no clip" it would also fall into.
        assert!(
            error.message.contains("is a transition op"),
            "{op}: {}",
            error.message
        );
        assert_eq!(snapshot(&mut state), before);
        let (now, refusal) = open_gesture(&mut state);
        assert_eq!(now, status);
        assert_eq!(refusal["code"], "gesture_op_not_allowed");
    }
    run(
        &mut state,
        json!({"op": "scene.fx.gesture.cancel", "gesture": gesture}),
    );
    assert_eq!(document(&state), before.0);
}
