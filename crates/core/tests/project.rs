//! P01/P02: the project document, multiple sequences, save/open, undo/redo and recovery,
//! driven through the public library API and the JSON executor (no window).

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tessel_editor_core::fixtures::{StandardSet, proof_requests, write_standard_set};
use tessel_editor_core::plan::PlanRequest;
use tessel_editor_core::{EditorState, headless, persist};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tessel-project-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(state: &mut EditorState, request: Value) -> Result<Value, tessel_editor_core::CommandError> {
    headless::execute(state, &request)
}

fn ok(state: &mut EditorState, request: Value) -> Value {
    run(state, request.clone()).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn proof_state(set: &StandardSet) -> EditorState {
    let mut state = EditorState::demo();
    for request in proof_requests(set) {
        ok(&mut state, request);
    }
    state
}

/// Every plan of the active sequence, as JSON: what "reproduces the project" means.
fn plans(state: &EditorState) -> Vec<Value> {
    (0..state.timeline.length)
        .step_by(7)
        .map(|frame| {
            let request = PlanRequest {
                intent: tessel_editor_core::plan::PreviewIntent::Sequence,
                at: tessel_editor_core::plan::PlanTime::Frame(frame),
            };
            state.plan(&request).unwrap().to_json()
        })
        .collect()
}

#[test]
fn save_and_open_reproduce_the_project_and_the_plan() {
    let dir = temp("roundtrip");
    let set = write_standard_set(&dir.join("fx")).unwrap();
    let mut state = proof_state(&set);
    ok(
        &mut state,
        json!({"op":"sequence.add","id":"vertical","name":"Vertical","width":180,"height":320,"activate":false}),
    );
    ok(
        &mut state,
        json!({"op":"clip.property","clip":"c-img","property":"zoom","value":80}),
    );

    let doc = dir.join("proof.tessel.json");
    ok(&mut state, json!({"op":"project.save","path":doc}));
    assert!(!state.project.is_dirty());
    let first = fs::read(&doc).unwrap();

    let mut reopened = EditorState::demo();
    let opened = ok(&mut reopened, json!({"op":"project.open","path":doc}));
    assert_eq!(opened["sequences"], json!(["main", "vertical"]));
    assert_eq!(
        plans(&state),
        plans(&reopened),
        "the plan is the proof of an exact round trip"
    );
    assert_eq!(
        state.to_document(Some(&dir)),
        reopened.to_document(Some(&dir)),
        "document -> state -> document is the identity"
    );
    // Canonical output: saving the reopened project again writes identical bytes.
    ok(&mut reopened, json!({"op":"project.save"}));
    assert_eq!(first, fs::read(&doc).unwrap());
    assert_eq!(
        reopened.project.id, state.project.id,
        "project ID is stable across saves"
    );
    fs::remove_dir_all(dir).ok();
}

#[test]
fn an_invalid_document_never_touches_the_open_project() {
    let dir = temp("invalid");
    let set = write_standard_set(&dir.join("fx")).unwrap();
    let mut state = proof_state(&set);
    let before = state.summary();

    let garbage = dir.join("garbage.tessel.json");
    fs::write(&garbage, "{ not json").unwrap();
    let error = run(&mut state, json!({"op":"project.open","path":garbage})).unwrap_err();
    assert_eq!(error.code, "invalid_document");

    let future = dir.join("future.tessel.json");
    let mut doc = state.to_document(Some(&dir));
    doc["schema_version"] = json!(99);
    fs::write(&future, serde_json::to_vec(&doc).unwrap()).unwrap();
    let error = run(&mut state, json!({"op":"project.open","path":future})).unwrap_err();
    assert_eq!(error.code, "unsupported_schema");
    assert!(error.message.contains("99"), "{}", error.message);

    // A clip pointing at a missing asset, a relative path that climbs out of the folder.
    let mut broken = state.to_document(Some(&dir));
    broken["sequences"][0]["clips"][0]["media"] = json!("ghost");
    let error = EditorState::from_document(&broken, Some(&dir)).unwrap_err();
    assert!(error.message.contains("ghost"), "{}", error.message);
    let mut escaping = state.to_document(Some(&dir));
    escaping["library"]["assets"][0]["asset"]["source"] =
        json!({"type":"file","path":"../outside.png"});
    let error = EditorState::from_document(&escaping, Some(&dir)).unwrap_err();
    assert_eq!(error.code, "invalid_document");

    let missing = run(
        &mut state,
        json!({"op":"project.open","path":dir.join("nope.tessel.json")}),
    );
    assert_eq!(missing.unwrap_err().code, "document_unreadable");
    assert_eq!(
        state.summary(),
        before,
        "every rejection left the session untouched"
    );
    fs::remove_dir_all(dir).ok();
}

#[test]
fn a_failed_save_keeps_the_previous_valid_document() {
    let dir = temp("atomic");
    let mut state = EditorState::demo();
    let doc = dir.join("a.tessel.json");
    persist::save(&mut state, &doc).unwrap();
    let good = fs::read(&doc).unwrap();

    // A directory squatting on the destination makes the final rename fail.
    let blocked = dir.join("b.tessel.json");
    fs::create_dir_all(blocked.join("child")).unwrap();
    let error = persist::save(&mut state, &blocked).unwrap_err();
    assert_eq!(error.code, "write_failed");
    assert!(blocked.is_dir(), "the destination was not replaced");
    let leftovers: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp-"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "temporary file cleaned up: {leftovers:?}"
    );
    assert_eq!(good, fs::read(&doc).unwrap());

    let wrong_name = dir.join("c.json");
    assert_eq!(
        persist::save(&mut state, &wrong_name).unwrap_err().code,
        "invalid_path"
    );
    fs::remove_dir_all(dir).ok();
}

#[test]
fn undo_redo_cover_edits_transactions_and_never_touch_files() {
    let dir = temp("history");
    let set = write_standard_set(&dir.join("fx")).unwrap();
    let mut state = proof_state(&set);
    let baseline = state.to_document(None);
    let revision = state.project.revision;

    ok(
        &mut state,
        json!({"op":"clip.property","clip":"c-img","property":"zoom","value":120}),
    );
    ok(
        &mut state,
        json!({"op":"set","target":"timeline.playhead","field":"frame","value":9}),
    );
    assert!(state.history.can_undo());
    let after_edit = state.to_document(None);
    assert_ne!(after_edit, baseline);

    let undone = ok(&mut state, json!({"op":"history.undo"}));
    assert_eq!(undone["label"], "Set property");
    assert_eq!(
        state.to_document(None)["clip_properties"],
        baseline["clip_properties"]
    );
    assert_eq!(
        state.timeline.playhead, 9,
        "undo does not rewind the playhead"
    );
    assert!(state.history.can_redo());
    ok(&mut state, json!({"op":"history.redo"}));
    assert_eq!(
        state.to_document(None)["clip_properties"],
        after_edit["clip_properties"]
    );
    assert!(state.project.revision > revision);

    // A transaction is one entry and rolls back as a whole when a step fails.
    let depth = state.history.to_json()["undo_depth"].as_u64().unwrap();
    let failed = state.transaction("Two clips", |state| {
        state.execute(tessel_editor_core::Command::SetSequenceLength { frames: 60 })?;
        state.execute(tessel_editor_core::Command::SelectClip {
            clip: "ghost".into(),
        })
    });
    assert!(failed.is_err());
    assert_eq!(state.timeline.length, 48, "the first step was rolled back");
    assert_eq!(
        state.history.to_json()["undo_depth"].as_u64().unwrap(),
        depth
    );
    state
        .transaction("Longer", |state| {
            state.execute(tessel_editor_core::Command::SetSequenceLength { frames: 60 })?;
            state.execute(tessel_editor_core::Command::SetSequenceLength { frames: 72 })
        })
        .unwrap();
    assert_eq!(
        state.history.to_json()["undo_depth"].as_u64().unwrap(),
        depth + 1
    );
    ok(&mut state, json!({"op":"history.undo"}));
    assert_eq!(state.timeline.length, 48);

    // A failed command is not an edit.
    let before = state.history.to_json();
    assert!(
        run(
            &mut state,
            json!({"op":"clip.property","clip":"nope","property":"zoom","value":1})
        )
        .is_err()
    );
    assert_eq!(state.history.to_json(), before);
    assert_eq!(
        run(&mut EditorState::demo(), json!({"op":"history.undo"}))
            .unwrap_err()
            .code,
        "nothing_to_undo"
    );

    // Undoing an asset registration drops the reference, never the file.
    let mut fresh = EditorState::demo();
    ok(
        &mut fresh,
        json!({"op":"project.create","name":"N","length":24}),
    );
    ok(
        &mut fresh,
        json!({"op":"asset.add","kind":"image","id":"img","name":"Q","path":set.image}),
    );
    ok(&mut fresh, json!({"op":"history.undo"}));
    assert!(fresh.library.item("img").is_none());
    assert!(set.image.is_file(), "undo must not delete the user's file");
    fs::remove_dir_all(dir).ok();
}

#[test]
fn multiple_sequences_are_independent_and_keep_clip_ids_unique() {
    let dir = temp("sequences");
    let set = write_standard_set(&dir.join("fx")).unwrap();
    let mut state = proof_state(&set);
    ok(
        &mut state,
        json!({"op":"sequence.add","id":"short","name":"Short cut","length":24}),
    );
    assert!(
        run(&mut state, json!({"op":"sequence.add","id":"short"})).is_err(),
        "duplicate sequence ID"
    );
    ok(
        &mut state,
        json!({"op":"sequence.activate","sequence":"short"}),
    );
    assert_eq!(state.timeline.id, "short");
    ok(
        &mut state,
        json!({"op":"track.add","id":"v","name":"V","kind":"video"}),
    );
    // `c-img` lives in `main`: clip IDs are global because clip properties are keyed by them.
    let error = run(
        &mut state,
        json!({"op":"clip.add","id":"c-img","track":"v","asset":"img","start":0,"duration":4}),
    )
    .unwrap_err();
    assert_eq!(error.code, "duplicate_id");
    ok(
        &mut state,
        json!({"op":"clip.add","id":"s-img","track":"v","asset":"img","start":0,"duration":4}),
    );
    assert_eq!(state.sequence("main").unwrap().clips.len(), 5);
    assert_eq!(state.timeline.clips.len(), 1);
    assert_eq!(
        run(
            &mut state,
            json!({"op":"sequence.remove","sequence":"short"})
        )
        .unwrap_err()
        .code,
        "sequence_active"
    );
    ok(
        &mut state,
        json!({"op":"sequence.activate","sequence":"main"}),
    );
    ok(
        &mut state,
        json!({"op":"sequence.remove","sequence":"short"}),
    );
    assert!(state.sequence("short").is_none());
    ok(&mut state, json!({"op":"history.undo"}));
    assert!(
        state.sequence("short").is_some(),
        "removing a sequence is undoable"
    );
    fs::remove_dir_all(dir).ok();
}

#[test]
fn autosave_survives_a_crash_and_never_overwrites_the_saved_document() {
    let dir = temp("recovery");
    let set = write_standard_set(&dir.join("fx")).unwrap();
    let mut state = proof_state(&set);
    let doc = dir.join("work.tessel.json");
    ok(&mut state, json!({"op":"project.save","path":doc}));
    let saved_bytes = fs::read(&doc).unwrap();
    assert_eq!(
        ok(&mut state, json!({"op":"project.autosave"}))["changed"],
        false,
        "clean: nothing to autosave"
    );

    ok(
        &mut state,
        json!({"op":"clip.property","clip":"c-img","property":"zoom","value":33}),
    );
    let first = ok(&mut state, json!({"op":"project.autosave"}));
    assert_eq!(first["changed"], true);
    assert_eq!(
        ok(&mut state, json!({"op":"project.autosave"}))["changed"],
        false,
        "same revision twice"
    );
    ok(
        &mut state,
        json!({"op":"clip.property","clip":"c-img","property":"zoom","value":34}),
    );
    ok(&mut state, json!({"op":"project.autosave"}));
    let edited = state.to_document(Some(&dir));

    // "Crash": the process state is gone; only the files remain.
    drop(state);
    let mut revived = EditorState::demo();
    let opened = ok(&mut revived, json!({"op":"project.open","path":doc}));
    assert_eq!(
        opened["recoveries"].as_array().unwrap().len(),
        2,
        "two autosaved revisions offered"
    );
    assert_ne!(
        revived.to_document(Some(&dir)),
        edited,
        "opening shows the saved version, not the recovery"
    );
    let listed = ok(
        &mut revived,
        json!({"op":"project.recover","path":doc,"apply":false}),
    );
    assert_eq!(listed["applied"], false);

    ok(&mut revived, json!({"op":"project.recover","path":doc}));
    assert_eq!(
        revived.to_document(Some(&dir))["clip_properties"],
        edited["clip_properties"]
    );
    assert!(
        revived.project.is_dirty(),
        "a recovered project is unsaved until the user saves"
    );
    assert_eq!(
        saved_bytes,
        fs::read(&doc).unwrap(),
        "recovery never writes the explicit document"
    );

    // Saving supersedes the autosaves; discarding is explicit otherwise.
    ok(&mut revived, json!({"op":"project.save"}));
    assert!(persist::recoveries_for_document(&doc).is_empty());
    assert_eq!(
        run(&mut revived, json!({"op":"project.recover","path":doc}))
            .unwrap_err()
            .code,
        "no_recovery"
    );
    fs::remove_dir_all(dir).ok();
}

#[test]
fn project_without_a_folder_cannot_autosave_and_save_needs_a_path() {
    let mut state = EditorState::demo();
    assert_eq!(
        run(&mut state, json!({"op":"project.autosave"}))
            .unwrap_err()
            .code,
        "no_workspace"
    );
    assert_eq!(
        run(&mut state, json!({"op":"project.save"}))
            .unwrap_err()
            .code,
        "no_document"
    );
}

#[test]
fn managed_media_is_stored_relative_to_the_document_and_resolves_after_a_move() {
    let dir = temp("relative");
    let set = write_standard_set(&dir.join("fx")).unwrap();
    // Pretend the image was copied into the workspace (the media adapter does this for real).
    let managed = dir.join("media/abc");
    fs::create_dir_all(&managed).unwrap();
    fs::copy(&set.image, managed.join("quadrants.png")).unwrap();
    let mut state = EditorState::demo();
    ok(
        &mut state,
        json!({"op":"project.create","name":"Rel","length":24,"dir":dir}),
    );
    ok(
        &mut state,
        json!({"op":"asset.add","kind":"image","id":"img","name":"Q","path":managed.join("quadrants.png")}),
    );
    let doc = dir.join("rel.tessel.json");
    ok(&mut state, json!({"op":"project.save","path":doc}));
    let stored: Value = serde_json::from_slice(&fs::read(&doc).unwrap()).unwrap();
    assert_eq!(
        stored["library"]["assets"][0]["asset"]["source"]["path"], "media/abc/quadrants.png",
        "managed paths are relative to the document, not absolute or cwd-based"
    );

    // Move the whole folder somewhere else and open it from another working directory.
    let moved = temp("relative-moved");
    fs::remove_dir_all(&moved).unwrap();
    fs::rename(&dir, &moved).unwrap();
    let mut reopened = EditorState::demo();
    ok(
        &mut reopened,
        json!({"op":"project.open","path":moved.join("rel.tessel.json")}),
    );
    let asset = reopened
        .library
        .item("img")
        .unwrap()
        .asset
        .as_ref()
        .unwrap();
    let tessel_editor_core::assets::Source::File { path } = &asset.source else {
        panic!("file source expected");
    };
    assert_eq!(Path::new(path), moved.join("media/abc/quadrants.png"));
    assert!(Path::new(path).is_file());
    fs::remove_dir_all(moved).ok();
}
