//! F016 (BL082.6/.16): the effect gesture, owned by the core, through the library and the JSON
//! executor, with no window and no GPU.
//!
//! What it proves: begin/update/commit/cancel share one handler for every adapter; the candidate
//! derives from the base and is what `scene.fx.list/frame/plan` show for the gesture, while the
//! document, the history and the revision do not move until a commit; a refused update keeps the
//! last legal candidate with a typed error; cancel, a scene or project switch, an external edit
//! or an undo never confirm old values; a changed commit is ONE undo entry, a no-op or a cancel
//! none; undo and redo restore values AND pixels (against the reference of the pixel math); the
//! effect inserted at a *time* is converted by the core; the >8 pass warning follows the
//! candidate and never drops a pass. The real process is the last test.

use std::io::Write;
use std::process::{Command as Process, Stdio};

use core::prelude::v1::test;
use serde_json::{Value, json};
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::directed_scene::fx;
use tessel_editor_core::state::ProjectSettings;
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

/// A scene `street` with one vignette `v1` over frames 10..40, intensity 0.5.
fn session() -> EditorState {
    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    ok(
        &mut state,
        json!({"op":"scene.attach","id":"street","bundle":bundle()}),
    );
    ok(
        &mut state,
        json!({"op":"scene.fx.edit","scene":"street","ops":[
            {"op":"add_clip","effect":"vignette","start_frame":10,"duration_frames":30,
             "id":"v1","params":{"intensity":0.5}}]}),
    );
    state
}

fn undo_depth(state: &EditorState) -> u64 {
    state.history.to_json()["undo_depth"].as_u64().unwrap()
}

fn redo_depth(state: &EditorState) -> u64 {
    state.history.to_json()["redo_depth"].as_u64().unwrap()
}

fn document(state: &EditorState, scene: &str) -> Value {
    state.scenes.get(scene).unwrap().scene.document().clone()
}

/// Everything a preview must not touch: documents, history depth, project revision.
fn footprint(state: &EditorState) -> (Value, u64, u64, u64) {
    (
        document(state, "street"),
        undo_depth(state),
        redo_depth(state),
        state.project.revision,
    )
}

fn begin(state: &mut EditorState) -> String {
    let answer = ok(
        state,
        json!({"op":"scene.fx.gesture.begin","scene":"street","clip":"v1"}),
    );
    assert_eq!(answer["document_changed"], false);
    answer["gesture"].as_str().unwrap().to_string()
}

fn update(state: &mut EditorState, gesture: &str, ops: Value) -> Result<Value, CommandError> {
    headless::execute(
        state,
        &json!({"op":"scene.fx.gesture.update","gesture":gesture,"ops":ops}),
    )
}

fn set(param: &str, value: Value) -> Value {
    json!({"op":"set_param","clip":"v1","param":param,"value":value})
}

fn candidate_intensity(state: &mut EditorState, gesture: &str) -> f64 {
    let answer = ok(
        state,
        json!({"op":"scene.fx.list","scene":"street","gesture":gesture}),
    );
    assert_eq!(answer["source"], "gesture");
    answer["tracks"][0]["clips"][0]["params"]["intensity"]
        .as_f64()
        .unwrap()
}

fn document_intensity(state: &mut EditorState) -> f64 {
    let answer = ok(state, json!({"op":"scene.fx.list","scene":"street"}));
    assert_eq!(answer["source"], "document");
    answer["tracks"][0]["clips"][0]["params"]["intensity"]
        .as_f64()
        .unwrap()
}

#[test]
fn the_preview_shows_the_candidate_and_writes_neither_document_nor_history() {
    let mut state = session();
    let before = footprint(&state);
    let gesture = begin(&mut state);

    let answer = update(&mut state, &gesture, json!([set("intensity", json!(0.9))])).unwrap();
    assert_eq!(answer["would_change"], true);
    assert_eq!(answer["document_changed"], false);
    assert_eq!(candidate_intensity(&mut state, &gesture), 0.9);
    assert_eq!(
        document_intensity(&mut state),
        0.5,
        "the document is the base"
    );

    // The frame and the plan of the gesture are the candidate's; the ones without it are not.
    let frame = ok(
        &mut state,
        json!({"op":"scene.fx.frame","scene":"street","frame":20,"gesture":gesture}),
    );
    assert_eq!(frame["source"], "gesture");
    assert_eq!(frame["passes"][0]["amount"], 0.9);
    let plan = ok(
        &mut state,
        json!({"op":"scene.plan","scene":"street","frame":20,"gesture":gesture}),
    );
    assert_eq!(plan["plan"]["fx"][0]["amount"], 0.9, "{plan}");
    let plain = ok(
        &mut state,
        json!({"op":"scene.plan","scene":"street","frame":20}),
    );
    assert_eq!(plain["plan"]["fx"][0]["amount"], 0.5);

    // Many updates still change nothing outside the gesture.
    for value in [0.1, 0.2, 0.3] {
        update(
            &mut state,
            &gesture,
            json!([set("intensity", json!(value))]),
        )
        .unwrap();
    }
    assert_eq!(footprint(&state), before);
    assert_eq!(
        ok(&mut state, json!({"op":"scene.fx.gesture.status"}))["active"],
        true
    );
}

#[test]
fn every_update_derives_from_the_base_not_from_the_previous_update() {
    let mut state = session();
    let gesture = begin(&mut state);
    update(&mut state, &gesture, json!([set("intensity", json!(0.2))])).unwrap();
    // This update does not mention the intensity: it is the base's again.
    update(&mut state, &gesture, json!([set("softness", json!(0.4))])).unwrap();
    assert_eq!(candidate_intensity(&mut state, &gesture), 0.5);
    let committed = ok(
        &mut state,
        json!({"op":"scene.fx.gesture.commit","gesture":gesture}),
    );
    assert_eq!(committed["changed"], true);
    assert_eq!(document_intensity(&mut state), 0.5);
}

#[test]
fn a_changed_commit_is_one_undo_entry_and_undo_and_redo_restore_the_values() {
    let mut state = session();
    let before = footprint(&state);
    let base_depth = undo_depth(&state);
    let gesture = begin(&mut state);
    for value in [0.6, 0.7, 0.8] {
        update(
            &mut state,
            &gesture,
            json!([set("intensity", json!(value))]),
        )
        .unwrap();
    }
    let committed = ok(
        &mut state,
        json!({"op":"scene.fx.gesture.commit","gesture":gesture}),
    );
    assert_eq!(committed["action"], "scene.fx.gesture.commit");
    assert_eq!(committed["changed"], true);
    assert_eq!(
        undo_depth(&state),
        base_depth + 1,
        "one entry for the whole drag"
    );
    assert_eq!(document_intensity(&mut state), 0.8);
    let after = document(&state, "street");
    assert_eq!(
        ok(&mut state, json!({"op":"scene.fx.gesture.status"}))["active"],
        false
    );

    ok(&mut state, json!({"op":"history.undo"}));
    assert_eq!(document(&state, "street"), before.0);
    assert_eq!(undo_depth(&state), base_depth);
    ok(&mut state, json!({"op":"history.redo"}));
    assert_eq!(document(&state, "street"), after);
}

#[test]
fn undo_and_redo_restore_the_pixels_against_the_reference() {
    let (width, height) = (64, 36);
    let mut baseline = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let base = 120 + ((x * 40) / width) as u8;
            baseline.extend_from_slice(&[
                base,
                base.saturating_add((y * 30 / height) as u8),
                150,
                255,
            ]);
        }
    }
    let render = |state: &EditorState| {
        let scene = DirectedScene::import_value(document(state, "street")).unwrap();
        let mut rgba = baseline.clone();
        for pass in scene.bundle().fx.resolve(20) {
            assert!(fx::apply_pass_to_rgba8(&pass, &mut rgba, width, height));
        }
        rgba
    };
    let mut state = session();
    let committed_before = render(&state);
    let gesture = begin(&mut state);
    update(&mut state, &gesture, json!([set("intensity", json!(1.0))])).unwrap();
    assert_eq!(
        render(&state),
        committed_before,
        "the preview is not in the document pixels"
    );
    ok(
        &mut state,
        json!({"op":"scene.fx.gesture.commit","gesture":gesture}),
    );
    let stronger = render(&state);
    assert_ne!(stronger, committed_before);
    ok(&mut state, json!({"op":"history.undo"}));
    assert_eq!(render(&state), committed_before);
    ok(&mut state, json!({"op":"history.redo"}));
    assert_eq!(render(&state), stronger);

    // Amount 0 is the identity: the very bytes of the baseline.
    let gesture = begin(&mut state);
    update(&mut state, &gesture, json!([set("intensity", json!(0.0))])).unwrap();
    ok(
        &mut state,
        json!({"op":"scene.fx.gesture.commit","gesture":gesture}),
    );
    assert_eq!(render(&state), baseline);
}

#[test]
fn an_invalid_update_keeps_the_last_legal_candidate_with_a_typed_error() {
    let mut state = session();
    let gesture = begin(&mut state);
    update(&mut state, &gesture, json!([set("intensity", json!(0.3))])).unwrap();
    let before = footprint(&state);

    let refused = update(&mut state, &gesture, json!([set("intensity", json!(2.0))])).unwrap_err();
    assert_eq!(refused.code, "param_out_of_range");
    let details = refused.details.as_deref().unwrap();
    assert_eq!(details["gesture"], gesture.as_str());
    assert_eq!(details["candidate_kept"], true);
    assert_eq!(
        candidate_intensity(&mut state, &gesture),
        0.3,
        "the last legal one"
    );
    assert_eq!(footprint(&state), before);

    let status = ok(&mut state, json!({"op":"scene.fx.gesture.status"}));
    assert_eq!(status["gesture"]["refusal"]["code"], "param_out_of_range");
    // The refusal does not poison the next legal update, which clears it.
    update(&mut state, &gesture, json!([set("intensity", json!(0.4))])).unwrap();
    let status = ok(&mut state, json!({"op":"scene.fx.gesture.status"}));
    assert!(status["gesture"]["refusal"].is_null());

    // A drag that ends on a refusal commits the last legal candidate, never the refused value.
    update(&mut state, &gesture, json!([set("intensity", json!(-1.0))])).unwrap_err();
    ok(
        &mut state,
        json!({"op":"scene.fx.gesture.commit","gesture":gesture}),
    );
    assert_eq!(document_intensity(&mut state), 0.4);
}

#[test]
fn a_range_the_edit_cannot_hold_is_refused_and_the_gesture_survives() {
    let mut state = session();
    let gesture = begin(&mut state);
    update(&mut state, &gesture, json!([set("intensity", json!(0.7))])).unwrap();
    let refused = update(
        &mut state,
        &gesture,
        json!([{"op":"set_field","clip":"v1","field":"duration_frames","value":100000}]),
    )
    .unwrap_err();
    // The layer's own gate (the re-import of the candidate) refuses a clip the edit cannot hold.
    assert!(
        refused.code == "invalid_bundle"
            || refused.code.eq_ignore_ascii_case("fx_clip_out_of_range"),
        "{}",
        refused.code
    );
    assert_eq!(refused.details.as_deref().unwrap()["candidate_kept"], true);
    assert_eq!(candidate_intensity(&mut state, &gesture), 0.7);
}

#[test]
fn ops_outside_the_gestures_clip_are_refused_without_losing_the_candidate() {
    let mut state = session();
    ok(
        &mut state,
        json!({"op":"scene.fx.edit","scene":"street","ops":[
            {"op":"add_clip","effect":"vignette","start_frame":50,"duration_frames":10,"id":"v2"}]}),
    );
    let gesture = begin(&mut state);
    update(&mut state, &gesture, json!([set("intensity", json!(0.6))])).unwrap();
    let other = update(
        &mut state,
        &gesture,
        json!([{"op":"set_param","clip":"v2","param":"intensity","value":0.1}]),
    )
    .unwrap_err();
    assert_eq!(other.code, "gesture_target_mismatch");
    let add = update(
        &mut state,
        &gesture,
        json!([{"op":"add_clip","effect":"vignette","start_frame":70,"duration_frames":5}]),
    )
    .unwrap_err();
    assert_eq!(add.code, "gesture_op_not_allowed");
    let remove = update(
        &mut state,
        &gesture,
        json!([{"op":"remove_clip","clip":"v1"}]),
    )
    .unwrap_err();
    assert_eq!(remove.code, "gesture_op_not_allowed");
    assert_eq!(candidate_intensity(&mut state, &gesture), 0.6);
}

#[test]
fn a_noop_commit_and_a_cancel_leave_zero_undo_entries() {
    let mut state = session();
    let before = footprint(&state);

    // Commit with no update.
    let gesture = begin(&mut state);
    let noop = ok(
        &mut state,
        json!({"op":"scene.fx.gesture.commit","gesture":gesture}),
    );
    assert_eq!(noop["changed"], false);
    assert_eq!(footprint(&state), before);

    // An update to the base's own value, and one that goes away and comes back.
    let gesture = begin(&mut state);
    let same = update(&mut state, &gesture, json!([set("intensity", json!(0.5))])).unwrap();
    assert_eq!(same["would_change"], false);
    update(&mut state, &gesture, json!([set("intensity", json!(0.9))])).unwrap();
    update(&mut state, &gesture, json!([])).unwrap();
    let noop = ok(
        &mut state,
        json!({"op":"scene.fx.gesture.commit","gesture":gesture}),
    );
    assert_eq!(noop["changed"], false);
    assert_eq!(footprint(&state), before);

    // Cancel after a real change.
    let gesture = begin(&mut state);
    update(&mut state, &gesture, json!([set("intensity", json!(0.9))])).unwrap();
    ok(
        &mut state,
        json!({"op":"scene.fx.gesture.cancel","gesture":gesture}),
    );
    assert_eq!(footprint(&state), before);
    let after_cancel = err(
        &mut state,
        json!({"op":"scene.fx.gesture.commit","gesture":gesture}),
    );
    assert_eq!(after_cancel.code, "no_gesture");
    assert_eq!(footprint(&state), before);
}

#[test]
fn an_external_edit_or_undo_makes_the_gesture_stale_and_it_never_confirms_old_values() {
    let mut state = session();
    let gesture = begin(&mut state);
    update(&mut state, &gesture, json!([set("intensity", json!(0.9))])).unwrap();
    // An edit from another surface in the middle of the drag.
    ok(
        &mut state,
        json!({"op":"scene.fx.edit","scene":"street","ops":[
            {"op":"set_field","clip":"v1","field":"fade_in_frames","value":3}]}),
    );
    let with_external = footprint(&state);

    let stale = update(&mut state, &gesture, json!([set("intensity", json!(0.8))])).unwrap_err();
    assert_eq!(stale.code, "gesture_invalidated");
    let commit = err(
        &mut state,
        json!({"op":"scene.fx.gesture.commit","gesture":gesture}),
    );
    assert_eq!(commit.code, "gesture_invalidated");
    let read = err(
        &mut state,
        json!({"op":"scene.fx.list","scene":"street","gesture":gesture}),
    );
    assert_eq!(read.code, "gesture_invalidated");
    assert_eq!(
        footprint(&state),
        with_external,
        "the external edit is untouched"
    );
    // Cancel is always possible.
    ok(
        &mut state,
        json!({"op":"scene.fx.gesture.cancel","gesture":gesture}),
    );

    // An undo of the external edit does not revive the old candidate either.
    let gesture = begin(&mut state);
    update(&mut state, &gesture, json!([set("intensity", json!(0.9))])).unwrap();
    ok(&mut state, json!({"op":"history.undo"}));
    let commit = err(
        &mut state,
        json!({"op":"scene.fx.gesture.commit","gesture":gesture}),
    );
    assert_eq!(commit.code, "gesture_invalidated");
    assert_eq!(document_intensity(&mut state), 0.5);
}

#[test]
fn a_scene_switch_or_detach_cannot_confirm_the_gesture() {
    let mut state = session();
    ok(
        &mut state,
        json!({"op":"scene.attach","id":"yard","bundle":bundle()}),
    );
    let gesture = begin(&mut state);
    update(&mut state, &gesture, json!([set("intensity", json!(0.9))])).unwrap();

    // Reading the gesture as if it belonged to the other scene is a typed mismatch.
    let wrong = err(
        &mut state,
        json!({"op":"scene.fx.list","scene":"yard","gesture":gesture}),
    );
    assert_eq!(wrong.code, "gesture_scene_mismatch");

    // Editing the other scene (the user switched scene and changed something) stales it.
    ok(
        &mut state,
        json!({"op":"scene.fx.edit","scene":"yard","ops":[
            {"op":"add_clip","effect":"vignette","start_frame":0,"duration_frames":5,"id":"y1"}]}),
    );
    let before = footprint(&state);
    let commit = err(
        &mut state,
        json!({"op":"scene.fx.gesture.commit","gesture":gesture}),
    );
    assert_eq!(commit.code, "gesture_invalidated");
    assert_eq!(footprint(&state), before);
    ok(
        &mut state,
        json!({"op":"scene.fx.gesture.cancel","gesture":gesture}),
    );

    let gesture = begin(&mut state);
    update(&mut state, &gesture, json!([set("intensity", json!(0.9))])).unwrap();
    ok(&mut state, json!({"op":"scene.detach","scene":"street"}));
    let commit = err(
        &mut state,
        json!({"op":"scene.fx.gesture.commit","gesture":gesture}),
    );
    assert_eq!(commit.code, "gesture_invalidated");
}

#[test]
fn a_new_project_does_not_confirm_a_gesture_of_the_old_one() {
    let mut state = session();
    let gesture = begin(&mut state);
    update(&mut state, &gesture, json!([set("intensity", json!(0.9))])).unwrap();
    ok(&mut state, json!({"op":"project.create","name":"Other"}));
    let commit = err(
        &mut state,
        json!({"op":"scene.fx.gesture.commit","gesture":gesture}),
    );
    assert_eq!(commit.code, "no_gesture", "{commit:?}");
    assert_eq!(
        commit.details.as_deref().unwrap()["reason"],
        "other_session"
    );
    assert_eq!(undo_depth(&state), 0);
}

#[test]
fn one_gesture_at_a_time_and_the_two_kinds_do_not_cross() {
    let mut state = session();
    let gesture = begin(&mut state);
    let second = err(
        &mut state,
        json!({"op":"scene.fx.gesture.begin","scene":"street","clip":"v1"}),
    );
    assert_eq!(second.code, "gesture_active");

    let crossed = err(
        &mut state,
        json!({"op":"scene.gesture.update","gesture":gesture,"ops":[]}),
    );
    assert_eq!(crossed.code, "gesture_kind_mismatch");
    let crossed = err(
        &mut state,
        json!({"op":"scene.gesture.commit","gesture":gesture}),
    );
    assert_eq!(crossed.code, "gesture_kind_mismatch");
    // The generic cancel serves every kind.
    ok(
        &mut state,
        json!({"op":"scene.gesture.cancel","gesture":gesture}),
    );

    let unknown = err(
        &mut state,
        json!({"op":"scene.fx.gesture.begin","scene":"street","clip":"nope"}),
    );
    assert_eq!(unknown.code, "unknown_element");
    assert_eq!(
        ok(&mut state, json!({"op":"scene.fx.gesture.status"}))["active"],
        false
    );
}

#[test]
fn a_vignette_is_inserted_where_the_time_falls_in_the_edit_and_a_bad_time_refuses_atomically() {
    let mut state = session();
    // 24 fps: half a second is frame 12; a quarter of a frame later is still frame 12.
    let added = ok(
        &mut state,
        json!({"op":"scene.fx.edit","scene":"street","ops":[
            {"op":"add_clip","effect":"vignette","start_time":{"num":1,"den":2},
             "duration_frames":6,"id":"v2"}]}),
    );
    assert_eq!(added["changed"], true);
    let listing = ok(&mut state, json!({"op":"scene.fx.list","scene":"street"}));
    let start = listing["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|track| track["clips"].as_array().unwrap())
        .find(|clip| clip["id"] == "v2")
        .unwrap()["start_frame"]
        .clone();
    assert_eq!(start, 12);

    let before = footprint(&state);
    // 96 frames at 24 fps = 4 s: 5 s is no frame of the edit.
    let late = err(
        &mut state,
        json!({"op":"scene.fx.edit","scene":"street","ops":[
            {"op":"set_param","clip":"v1","param":"intensity","value":0.9},
            {"op":"add_clip","effect":"vignette","start_time":5,"id":"v3"}]}),
    );
    assert!(
        late.code.eq_ignore_ascii_case("fx_clip_out_of_range"),
        "{}",
        late.code
    );
    assert_eq!(
        footprint(&state),
        before,
        "nothing of the batch was applied"
    );

    let both = err(
        &mut state,
        json!({"op":"scene.fx.edit","scene":"street","ops":[
            {"op":"add_clip","effect":"vignette","start_frame":3,"start_time":1,"id":"v3"}]}),
    );
    assert_eq!(both.code, "invalid_op", "{both:?}");
    assert_eq!(footprint(&state), before);
}

fn crowd(count: usize) -> EditorState {
    let mut document = bundle();
    document["fx"] = json!({"catalogVersion":1,"transitions":[],"tracks":(0..count).map(|n| json!({
        "id": format!("t{n}"), "label": format!("FX {}", n + 1),
        "clips":[{"id": format!("c{n}"), "effect":"vignette", "startFrame": 0, "durationFrames": 20}]
    })).collect::<Vec<_>>()});
    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    ok(
        &mut state,
        json!({"op":"scene.attach","id":"street","bundle":document}),
    );
    state
}

#[test]
fn the_budget_warning_follows_the_candidate_and_never_drops_a_pass() {
    // Nine clips alive at once: over the budget of 8 already in the document.
    let mut state = crowd(9);
    let listing = ok(&mut state, json!({"op":"scene.fx.list","scene":"street"}));
    assert_eq!(listing["budget"]["over_budget"], true);
    let gesture = ok(
        &mut state,
        json!({"op":"scene.fx.gesture.begin","scene":"street","clip":"c0"}),
    )["gesture"]
        .as_str()
        .unwrap()
        .to_string();

    // Hiding the clip in the candidate brings the candidate back inside the budget...
    let hidden = ok(
        &mut state,
        json!({"op":"scene.fx.gesture.update","gesture":gesture,"ops":[
            {"op":"set_hidden","clip":"c0","hidden":true}]}),
    );
    assert_eq!(hidden["budget"]["over_budget"], false);
    assert_eq!(hidden["warnings"], json!([]));
    // ...while the document still warns until the commit.
    let listing = ok(&mut state, json!({"op":"scene.fx.list","scene":"street"}));
    assert_eq!(listing["budget"]["over_budget"], true);

    // A candidate that keeps all nine warns and keeps all nine passes.
    let kept = ok(
        &mut state,
        json!({"op":"scene.fx.gesture.update","gesture":gesture,"ops":[
            {"op":"set_param","clip":"c0","param":"intensity","value":0.3}]}),
    );
    assert_eq!(kept["budget"]["peak_passes"], 9);
    assert_eq!(kept["warnings"][0]["code"], "FX_PASS_BUDGET");
    let frame = ok(
        &mut state,
        json!({"op":"scene.fx.frame","scene":"street","frame":5,"gesture":gesture}),
    );
    assert_eq!(frame["count"], 9, "nothing is truncated");
    assert_eq!(frame["over_budget"], true);
    let status = ok(&mut state, json!({"op":"scene.fx.gesture.status"}));
    assert_eq!(status["warnings"][0]["code"], "FX_PASS_BUDGET");
    ok(
        &mut state,
        json!({"op":"scene.fx.gesture.cancel","gesture":gesture}),
    );
}

/// A real `tessel-headless` process, one request at a time: the gesture ID is the process's own.
struct Process_ {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    stdout: std::io::BufReader<std::process::ChildStdout>,
}

impl Process_ {
    fn start() -> Self {
        let mut child = Process::new(env!("CARGO_BIN_EXE_tessel-headless"))
            .arg("--empty")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = std::io::BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
        }
    }

    fn ask(&mut self, request: Value) -> Value {
        use std::io::BufRead;
        writeln!(self.stdin, "{request}").unwrap();
        self.stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }
}

#[test]
fn the_headless_executable_runs_the_gesture_without_a_window_and_refuses_with_a_typed_error() {
    let mut process = Process_::start();
    let intensity =
        |answer: &Value| answer["result"]["tracks"][0]["clips"][0]["params"]["intensity"].clone();
    let depth = |answer: &Value| answer["result"]["history"]["undo_depth"].as_u64().unwrap();

    process.ask(json!({"op":"scene.attach","id":"street","bundle":bundle()}));
    process.ask(json!({"op":"scene.fx.edit","scene":"street","ops":[
        {"op":"add_clip","effect":"vignette","start_frame":10,"duration_frames":30,
         "id":"v1","params":{"intensity":0.5}}]}));
    let base_depth = depth(&process.ask(json!({"op":"history.status"})));

    let begun = process.ask(json!({"op":"scene.fx.gesture.begin","scene":"street","clip":"v1"}));
    assert_eq!(begun["result"]["document_changed"], false);
    let gesture = begun["result"]["gesture"].as_str().unwrap().to_string();

    let updated = process.ask(json!({"op":"scene.fx.gesture.update","gesture":gesture,
        "ops":[set("intensity", json!(0.8))]}));
    assert_eq!(updated["result"]["would_change"], true);
    let candidate = process.ask(json!({"op":"scene.fx.list","scene":"street","gesture":gesture}));
    assert_eq!(intensity(&candidate), 0.8, "the candidate is queryable");
    let document = process.ask(json!({"op":"scene.fx.list","scene":"street"}));
    assert_eq!(intensity(&document), 0.5, "the document is not");
    assert_eq!(
        depth(&process.ask(json!({"op":"history.status"}))),
        base_depth,
        "updates write no undo entry"
    );

    let refused = process.ask(json!({"op":"scene.fx.gesture.update","gesture":gesture,
        "ops":[set("intensity", json!(9))]}));
    assert_eq!(refused["ok"], false);
    assert_eq!(refused["error"]["code"], "param_out_of_range");
    assert_eq!(refused["error"]["details"]["candidate_kept"], true);

    let committed = process.ask(json!({"op":"scene.fx.gesture.commit","gesture":gesture}));
    assert_eq!(
        committed["result"]["changed"], true,
        "the last legal candidate"
    );
    let again = process.ask(json!({"op":"scene.fx.gesture.commit","gesture":gesture}));
    assert_eq!(again["ok"], false, "a single commit");
    assert_eq!(again["error"]["code"], "no_gesture");
    assert_eq!(
        depth(&process.ask(json!({"op":"history.status"}))),
        base_depth + 1
    );
    process.ask(json!({"op":"history.undo"}));
    let after_undo = process.ask(json!({"op":"scene.fx.list","scene":"street"}));
    assert_eq!(intensity(&after_undo), 0.5, "undo restores the value");

    drop(process.stdin);
    let status = process.child.wait().unwrap();
    assert_eq!(
        status.code(),
        Some(1),
        "a session with a refused request exits 1"
    );
}
