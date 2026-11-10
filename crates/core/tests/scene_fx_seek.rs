//! F058/F016 (BL082.30/.33, P3): going to a key, the revision a key edit was built on, and what
//! a key gesture is allowed to do, through the library and the JSON executor (no window, no GPU).
//!
//! What it proves: `scene.fx.seek` finds the previous/next/first/last key (or the one at a clip
//! frame) of one parameter, converts it clip -> bundle -> time -> sequence in exact rationals and
//! moves ONLY the playhead (no document, history entry or revision), over a sequence at the
//! bundle's rate, at 30 fps, at 30000/1001 and at a rate that skips bundle frames (refused, never
//! a neighbour), with the effect clip starting at a frame other than 0 in every case; each
//! refusal is typed and moves nothing. A key edit built on a project revision that has moved is
//! refused (`stale_revision`), written never. A key gesture binds its verb and target at the
//! first update and refuses any other without touching the candidate, the last legal ops or the
//! revision binding; a direct `scene.fx.edit` may still mix keys and base values.
//!
//! Speed and offset of a scene clip do not exist on this path (the sequence maps time to the
//! bundle through the bundle's own clock only), so they are N/A here, and the test says so by
//! asserting that the only scale between the two is the rate.

use core::prelude::v1::test;

use serde_json::{Value, json};
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::time::Rate;
use tessel_editor_core::{CommandError, EditorState, headless};

const MINIMAL: &str =
    include_str!("../../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json");

fn bundle() -> Value {
    serde_json::from_str(MINIMAL).unwrap()
}

fn ok(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn err(state: &mut EditorState, request: Value) -> CommandError {
    match headless::execute(state, &request) {
        Ok(answer) => panic!("{request} should have failed, got {answer}"),
        Err(error) => error,
    }
}

/// A scene `street` in a sequence at `rate`, with a grain `g` over the edit frames 10..50 whose
/// `amount` has keys at clip frames 0, 10 and 30 (edit frames 10, 20, 40).
fn session(rate: (u32, u32)) -> EditorState {
    let mut settings = ProjectSettings::default();
    settings.sequence.rate = Rate::new(rate.0, rate.1).unwrap();
    let mut state = EditorState::empty(&settings).unwrap();
    ok(
        &mut state,
        json!({"op":"scene.attach","id":"street","bundle":bundle()}),
    );
    ok(
        &mut state,
        json!({"op":"scene.fx.edit","scene":"street","ops":[
            {"op":"add_clip","effect":"filmGrain","start_frame":10,"duration_frames":40,"id":"g"},
            {"op":"key_param","clip":"g","param":"amount","frame":0,"value":0.1},
            {"op":"key_param","clip":"g","param":"amount","frame":10,"value":0.5},
            {"op":"key_param","clip":"g","param":"amount","frame":30,"value":0.9}]}),
    );
    state
}

fn undo_depth(state: &EditorState) -> u64 {
    state.history.to_json()["undo_depth"].as_u64().unwrap()
}

fn document(state: &EditorState) -> Value {
    state.scenes.get("street").unwrap().scene.document().clone()
}

fn footprint(state: &EditorState) -> (Value, u64, u64) {
    (document(state), undo_depth(state), state.project.revision)
}

fn seek(state: &mut EditorState, to: Value) -> Result<Value, CommandError> {
    headless::execute(
        state,
        &json!({"op":"scene.fx.seek","scene":"street","clip":"g","param":"amount","to":to}),
    )
}

fn at(state: &EditorState) -> i64 {
    state.timeline.playhead
}

/// The bundle frame the monitor shows.
fn shown(state: &EditorState) -> i64 {
    state.scene_frame_at_playhead("street").unwrap()
}

#[test]
fn seek_walks_the_keys_in_both_directions_and_moves_only_the_playhead() {
    let mut state = session((24, 1));
    let before = footprint(&state);
    let first = seek(&mut state, json!("first")).unwrap();
    assert_eq!(first["document_changed"], false);
    assert_eq!(first["frame_in_clip"], 0);
    assert_eq!(first["edit_frame"], 10, "the clip starts at 10");
    assert_eq!((at(&state), shown(&state)), (10, 10));
    for (to, local, playhead) in [("next", 10, 20), ("next", 30, 40)] {
        let answer = seek(&mut state, json!(to)).unwrap();
        assert_eq!(answer["frame_in_clip"], local);
        assert_eq!((at(&state), shown(&state)), (playhead, playhead));
    }
    let end = err(
        &mut state,
        json!({"op":"scene.fx.seek","scene":"street","clip":"g","param":"amount","to":"next"}),
    );
    assert_eq!(end.code, "no_next_key");
    assert_eq!(at(&state), 40, "a refusal moves nothing");
    seek(&mut state, json!("previous")).unwrap();
    assert_eq!(at(&state), 20);
    seek(&mut state, json!("last")).unwrap();
    assert_eq!(at(&state), 40);
    seek(&mut state, json!(10)).unwrap();
    assert_eq!(at(&state), 20, "a frame of the CLIP, not of the edit");
    seek(&mut state, json!("first")).unwrap();
    let start = err(
        &mut state,
        json!({"op":"scene.fx.seek","scene":"street","clip":"g","param":"amount","to":"previous"}),
    );
    assert_eq!(start.code, "no_previous_key");
    assert_eq!(
        footprint(&state),
        before,
        "no document, entry or revision moved"
    );
}

#[test]
fn from_between_keys_previous_and_next_are_the_nearest_not_the_adjacent_index() {
    let mut state = session((24, 1));
    ok(&mut state, json!({"op":"seek","frame":25}));
    assert_eq!(
        shown(&state),
        25,
        "clip frame 15, between the keys at 10 and 30"
    );
    seek(&mut state, json!("previous")).unwrap();
    assert_eq!(at(&state), 20);
    ok(&mut state, json!({"op":"seek","frame":25}));
    seek(&mut state, json!("next")).unwrap();
    assert_eq!(at(&state), 40);
    // Before the clip every key is "next", none is "previous".
    ok(&mut state, json!({"op":"seek","frame":2}));
    assert_eq!(
        err(
            &mut state,
            json!({"op":"scene.fx.seek","scene":"street","clip":"g","param":"amount","to":"previous"})
        )
        .code,
        "no_previous_key"
    );
    seek(&mut state, json!("next")).unwrap();
    assert_eq!(at(&state), 10);
}

#[test]
fn every_refusal_of_a_seek_is_typed_and_moves_nothing() {
    let mut state = session((24, 1));
    ok(&mut state, json!({"op":"seek","frame":3}));
    let before = (footprint(&state), at(&state));
    for (request, code) in [
        (
            json!({"op":"scene.fx.seek","scene":"street","clip":"g","param":"amount","to":5}),
            "unknown_keyframe",
        ),
        (
            json!({"op":"scene.fx.seek","scene":"street","clip":"g","param":"size","to":"first"}),
            "no_keys",
        ),
        (
            json!({"op":"scene.fx.seek","scene":"street","clip":"g","param":"nope","to":"first"}),
            "unknown_param",
        ),
        (
            json!({"op":"scene.fx.seek","scene":"street","clip":"zz","param":"amount","to":"first"}),
            "unknown_element",
        ),
        (
            json!({"op":"scene.fx.seek","scene":"zz","clip":"g","param":"amount","to":"first"}),
            "unknown_element",
        ),
        (
            json!({"op":"scene.fx.seek","scene":"street","clip":"g","param":"amount","to":"middle"}),
            "invalid_request",
        ),
    ] {
        let refused = err(&mut state, request.clone());
        assert_eq!(refused.code, code, "{request}: {refused:?}");
        assert_eq!((footprint(&state), at(&state)), before, "{request}");
    }
}

#[test]
fn the_conversion_is_exact_at_30_fps_and_at_ntsc_with_the_clip_not_at_zero() {
    // (rate, [(clip frame of the key, sequence frame the core must choose)]): the first sequence
    // frame at or after b / 24 s whose own time lies in bundle frame b.
    //   30 fps:       b=10 -> ceil(12.5)=13,  b=20 -> 25, b=40 -> 50
    //   30000/1001:   b=10 -> ceil(12.4875)=13, b=20 -> ceil(24.975)=25, b=40 -> ceil(49.95)=50
    for rate in [(30, 1), (30000, 1001)] {
        let mut state = session(rate);
        let before = footprint(&state);
        for (to, expected) in [
            (json!("first"), 13),
            (json!("next"), 25),
            (json!("next"), 50),
        ] {
            seek(&mut state, to.clone()).unwrap();
            assert_eq!(at(&state), expected, "{rate:?} {to}");
        }
        // And what the monitor shows at each is exactly the key's bundle frame (not a neighbour).
        for (local, bundle_frame) in [(0, 10), (10, 20), (30, 40)] {
            let answer = seek(&mut state, json!(local)).unwrap();
            assert_eq!(answer["edit_frame"], bundle_frame, "{rate:?}");
            assert_eq!(shown(&state), bundle_frame, "{rate:?}");
        }
        assert_eq!(footprint(&state), before, "{rate:?}");
    }
}

#[test]
fn a_bundle_frame_the_sequence_never_lands_on_is_refused_not_rounded() {
    // 12 fps over a 24 fps bundle: only the even bundle frames have a sequence frame.
    let mut state = session((12, 1));
    ok(
        &mut state,
        json!({"op":"scene.fx.edit","scene":"street","ops":[
            {"op":"key_param","clip":"g","param":"amount","frame":1,"value":0.2},
            {"op":"key_param","clip":"g","param":"amount","frame":2,"value":0.3}]}),
    );
    let before = (footprint(&state), at(&state));
    seek(&mut state, json!(0)).unwrap();
    assert_eq!((at(&state), shown(&state)), (5, 10));
    let before_odd = (footprint(&state), at(&state));
    let refused = err(
        &mut state,
        json!({"op":"scene.fx.seek","scene":"street","clip":"g","param":"amount","to":1}),
    );
    assert_eq!(refused.code, "frame_not_on_sequence", "{refused:?}");
    assert_eq!((footprint(&state), at(&state)), before_odd);
    seek(&mut state, json!(2)).unwrap();
    assert_eq!((at(&state), shown(&state)), (6, 12));
    assert_eq!(before.0, footprint(&state), "only the playhead ever moved");
}

#[test]
fn the_inspector_names_the_neighbours_the_seek_goes_to() {
    let mut state = session((30, 1));
    seek(&mut state, json!("next")).unwrap(); // from 0, the first key: edit frame 10
    assert_eq!(shown(&state), 10);
    seek(&mut state, json!("next")).unwrap();
    let frame = shown(&state);
    assert_eq!(frame, 20);
    let answer = ok(
        &mut state,
        json!({"op":"scene.fx.inspect","scene":"street","clip":"g","frame":frame}),
    );
    let animation = &answer["clips"][0]["animation"];
    assert_eq!(animation["local_frame"], 10);
    let amount = animation["params"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["param"] == "amount")
        .unwrap();
    assert_eq!(
        (&amount["previous_key"], &amount["next_key"]),
        (&json!(0), &json!(30))
    );
    let go = |name: &str| {
        amount["operations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|operation| operation["name"] == name)
            .unwrap_or_else(|| panic!("no {name}"))
            .clone()
    };
    for name in ["go_previous", "go_next", "go_first", "go_last"] {
        assert_eq!(go(name)["enabled"], true, "{name}");
    }
    // The template of an enabled go runs as it is, once the scene is named.
    let mut request = go("go_next")["template"].clone();
    request["scene"] = json!("street");
    ok(&mut state, request);
    assert_eq!(shown(&state), 40);
    // At the last key there is nothing after it, and the inspector says why with the seek's code.
    let answer = ok(
        &mut state,
        json!({"op":"scene.fx.inspect","scene":"street","clip":"g","frame":40}),
    );
    let amount = answer["clips"][0]["animation"]["params"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["param"] == "amount")
        .unwrap()
        .clone();
    let next = amount["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|operation| operation["name"] == "go_next")
        .unwrap();
    assert_eq!(next["enabled"], false);
    assert_eq!(next["reason"], "no_next_key");
}

// ---- the revision a key edit was built on -------------------------------------------------------------

#[test]
fn an_edit_built_on_a_revision_that_moved_is_refused_and_writes_nothing() {
    let mut state = session((24, 1));
    let revision = state.project.revision;
    let edit = |base: Option<u64>, value: f64| {
        let mut request = json!({"op":"scene.fx.edit","scene":"street","ops":[
            {"op":"key_param","clip":"g","param":"amount","frame":10,"value":value}]});
        if let Some(base) = base {
            request["base_revision"] = json!(base);
        }
        request
    };
    // The revision the editor read: accepted, and it moves the revision.
    ok(&mut state, edit(Some(revision), 0.6));
    assert!(state.project.revision > revision);
    // The same view, used again: stale, refused, nothing written.
    let before = footprint(&state);
    let stale = err(&mut state, edit(Some(revision), 0.7));
    assert_eq!(stale.code, "stale_revision", "{stale:?}");
    assert_eq!(footprint(&state), before);
    // An undo moves the revision too, so a view from before it is stale as well.
    let seen = state.project.revision;
    ok(&mut state, json!({"op":"history.undo"}));
    assert_eq!(
        err(&mut state, edit(Some(seen), 0.8)).code,
        "stale_revision"
    );
    // Without a base the edit applies to what is there (the library's own use).
    ok(&mut state, edit(None, 0.8));
    // A malformed base is a request error, not a silent none.
    let mut bad = edit(None, 0.9);
    bad["base_revision"] = json!("x");
    assert_eq!(err(&mut state, bad).code, "invalid_request");
}

// ---- F5: what a key gesture may do ----------------------------------------------------------------------

fn begin(state: &mut EditorState) -> String {
    ok(
        state,
        json!({"op":"scene.fx.gesture.begin","scene":"street","clip":"g"}),
    )["gesture"]
        .as_str()
        .unwrap()
        .to_string()
}

fn update(state: &mut EditorState, gesture: &str, ops: Value) -> Result<Value, CommandError> {
    headless::execute(
        state,
        &json!({"op":"scene.fx.gesture.update","gesture":gesture,"ops":ops}),
    )
}

fn candidate_keys(state: &mut EditorState, gesture: &str) -> Value {
    ok(
        state,
        json!({"op":"scene.fx.list","scene":"street","gesture":gesture}),
    )["tracks"][0]["clips"][0]["keys"]["amount"]
        .clone()
}

fn status(state: &mut EditorState) -> Value {
    ok(state, json!({"op":"scene.fx.gesture.status"}))["gesture"].clone()
}

#[test]
fn a_key_gesture_refuses_another_verb_or_target_and_keeps_everything_it_had() {
    let mut state = session((24, 1));
    let gesture = begin(&mut state);
    let drag = |value: f64| json!([{"op":"key_param","clip":"g","param":"amount","frame":10,"value":value}]);
    update(&mut state, &gesture, drag(0.7)).unwrap();
    let kept = candidate_keys(&mut state, &gesture);
    let intent = status(&mut state)["intent"].clone();
    assert_eq!(intent["verb"], "key_param");
    assert_eq!(intent["param"], "amount");
    assert_eq!(intent["key"], 10);
    let before = footprint(&state);

    for (ops, code) in [
        // A slider write in a key drag.
        (
            json!([{"op":"set_param","clip":"g","param":"amount","value":0.4}]),
            "gesture_intent_mismatch",
        ),
        // Another key of the same curve, another parameter, the seed, a retime of the curve.
        (
            json!([{"op":"key_param","clip":"g","param":"amount","frame":30,"value":0.2}]),
            "gesture_target_mismatch",
        ),
        (
            json!([{"op":"key_param","clip":"g","param":"size","frame":10,"value":1.0}]),
            "gesture_target_mismatch",
        ),
        (
            json!([{"op":"set_seed","clip":"g","seed":7}]),
            "gesture_intent_mismatch",
        ),
        (
            json!([{"op":"shift_keys","clip":"g","param":"amount","delta":1}]),
            "gesture_intent_mismatch",
        ),
        (
            json!([{"op":"remove_key","clip":"g","param":"amount","frame":10}]),
            "gesture_intent_mismatch",
        ),
        // A mixed batch: the first verb is the gesture's, the second is not.
        (
            json!([
                {"op":"key_param","clip":"g","param":"amount","frame":10,"value":0.8},
                {"op":"set_param","clip":"g","param":"amount","value":0.4}
            ]),
            "gesture_intent_mismatch",
        ),
    ] {
        let refused = update(&mut state, &gesture, ops.clone()).unwrap_err();
        assert_eq!(refused.code, code, "{ops}: {refused:?}");
        assert_eq!(refused.details.as_deref().unwrap()["candidate_kept"], true);
        assert_eq!(
            candidate_keys(&mut state, &gesture),
            kept,
            "the candidate is what it was: {ops}"
        );
        assert_eq!(footprint(&state), before, "{ops}");
        let now = status(&mut state);
        assert_eq!(now["refusal"]["code"], code, "the refusal is recorded");
        assert_eq!(now["intent"], intent, "the binding does not move");
    }
    // The gesture is alive: the same intent still works, and its commit is ONE entry of the drag.
    update(&mut state, &gesture, drag(0.35)).unwrap();
    assert!(status(&mut state)["refusal"].is_null());
    ok(
        &mut state,
        json!({"op":"scene.fx.gesture.commit","gesture":gesture}),
    );
    assert_eq!(undo_depth(&state), before.1 + 1);
    let keys = document(&state)["fx"]["tracks"][0]["clips"][0]["keys"]["amount"].clone();
    assert_eq!(keys[1]["value"], 0.35);
    assert_eq!(keys[0]["value"], 0.1, "the siblings are what they were");
    assert_eq!(keys[2]["value"], 0.9);
}

#[test]
fn a_value_gesture_refuses_a_key_op_and_a_direct_edit_may_still_mix_both() {
    let mut state = session((24, 1));
    let gesture = begin(&mut state);
    update(
        &mut state,
        &gesture,
        json!([{"op":"set_param","clip":"g","param":"size","value":2.0}]),
    )
    .unwrap();
    let before = footprint(&state);
    let refused = update(
        &mut state,
        &gesture,
        json!([{"op":"key_param","clip":"g","param":"amount","frame":10,"value":0.2}]),
    )
    .unwrap_err();
    assert_eq!(refused.code, "gesture_intent_mismatch");
    assert_eq!(footprint(&state), before);
    ok(
        &mut state,
        json!({"op":"scene.fx.gesture.cancel","gesture":gesture}),
    );
    // A direct edit is not a gesture: a deliberate batch of keys and base values applies whole.
    let mixed = ok(
        &mut state,
        json!({"op":"scene.fx.edit","scene":"street","ops":[
            {"op":"key_param","clip":"g","param":"amount","frame":10,"value":0.2},
            {"op":"set_param","clip":"g","param":"size","value":2.0}]}),
    );
    assert_eq!(mixed["changed"], true);
}
