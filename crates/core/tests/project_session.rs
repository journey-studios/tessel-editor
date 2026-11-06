//! BL113a: a project is told apart from the one that replaced it by its editing session, not by
//! `(id, revision)`. The ID is persisted and the revision restarts at 0 on every open, so the
//! document reopened is indistinguishable by those two; the session is minted when a state is
//! built and never persisted.
//!
//! Library only (no window, no media host). The consumer of the stamp, the scene attach job, is
//! proven through the real host in `crates/media/tests/scene_attach_job.rs` and through a real
//! `tessel-media` process in `crates/media/tests/scene_attach_process.rs`.

use core::prelude::v1::test;

use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};
use tessel_editor_core::scene_project::{SceneCommand, SceneSource};
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::{Command, EditorState, headless, persist};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tessel-session-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn ok(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn saved_project(name: &str) -> (EditorState, PathBuf) {
    let mut state = EditorState::demo();
    ok(&mut state, json!({"op":"project.create","name":"identity"}));
    let path = temp(name).join("p.tessel.json");
    persist::save(&mut state, &path).unwrap();
    (state, path)
}

fn session(state: &EditorState) -> u64 {
    state.project.session.number()
}

fn attach_scene(state: &mut EditorState, id: &str) {
    state
        .execute(Command::Scene(SceneCommand::Attach {
            id: Some(id.into()),
            name: None,
            source: SceneSource::Text(
                include_str!(
                    "../../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json"
                )
                .to_string(),
            ),
        }))
        .unwrap();
}

#[test]
fn every_state_that_is_built_is_a_new_session_and_a_clone_is_the_same_one() {
    let demo = EditorState::demo();
    let empty = EditorState::empty(&ProjectSettings::default()).unwrap();
    let other = EditorState::empty(&ProjectSettings::default()).unwrap();
    let sessions = [session(&demo), session(&empty), session(&other)];
    assert_eq!(
        sessions
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3,
        "{sessions:?}"
    );
    // Internal clones (undo snapshots, package plans, job inputs) keep the session of the state
    // they were taken from.
    assert_eq!(session(&empty.clone()), session(&empty));
    assert!(empty.project.stamp().is_current(&empty.clone().project));
}

#[test]
fn reopening_a_document_gives_the_same_id_and_revision_in_a_different_session() {
    let (saved, path) = saved_project("reopen");
    let first = persist::load(&path).unwrap();
    let second = persist::load(&path).unwrap();
    for loaded in [&first, &second] {
        assert_eq!(loaded.project.id, saved.project.id, "the ID is persisted");
        assert_eq!(loaded.project.revision, 0, "the revision restarts");
        assert_eq!(loaded.project.revision, saved.project.revision);
    }
    assert_ne!(
        session(&first),
        session(&second),
        "two opens are two sessions"
    );
    assert_ne!(session(&first), session(&saved));

    // The old stamp matches on `(id, revision)` and is still refused: that is the whole point.
    let stamp = saved.project.stamp();
    assert_eq!(
        (&first.project.id, first.project.revision),
        (&saved.project.id, saved.project.revision)
    );
    assert!(stamp.is_current(&saved.project));
    assert!(!stamp.is_current(&first.project));
    assert!(!stamp.is_current(&second.project));
}

#[test]
fn the_session_is_not_in_the_document_and_does_not_change_what_is_saved() {
    let (mut state, path) = saved_project("document");
    attach_scene(&mut state, "street");
    persist::save(&mut state, &path).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(
        !text.contains("session"),
        "runtime identity must not be persisted"
    );

    let reopened = persist::load(&path).unwrap();
    assert_eq!(
        reopened.to_document(None),
        state.to_document(None),
        "two sessions of one project save the same document"
    );
    // Saving the reopened project writes the same bytes: nothing about the session leaks.
    let again = path.with_file_name("again.tessel.json");
    let mut reopened = reopened;
    persist::save(&mut reopened, &again).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&fs::read_to_string(&again).unwrap()).unwrap()["scenes"],
        serde_json::from_str::<Value>(&text).unwrap()["scenes"]
    );
}

#[test]
fn edits_undo_redo_save_and_queries_stay_in_the_same_session() {
    let (mut state, path) = saved_project("same-session");
    let started = session(&state);
    let stamp = state.project.stamp();

    // Queries never change the session or the revision: the stamp stays current.
    for request in [
        json!({"op":"history.status"}),
        json!({"op":"scene.list"}),
        json!({"op":"scene.capabilities"}),
    ] {
        ok(&mut state, request);
    }
    assert!(stamp.is_current(&state.project));
    persist::save(&mut state, &path).unwrap();
    assert!(stamp.is_current(&state.project), "saving is not an edit");

    attach_scene(&mut state, "street");
    assert_eq!(session(&state), started);
    assert!(
        !stamp.is_current(&state.project),
        "an edit moves the revision (BL113)"
    );
    ok(&mut state, json!({"op":"history.undo"}));
    assert_eq!(session(&state), started);
    ok(&mut state, json!({"op":"history.redo"}));
    assert_eq!(session(&state), started, "undo/redo keep the session");
    persist::save(&mut state, &path).unwrap();
    assert_eq!(session(&state), started);
    // A stamp taken now is current until the next edit, however many queries come between.
    let fresh = state.project.stamp();
    ok(&mut state, json!({"op":"history.status"}));
    assert!(fresh.is_current(&state.project));
}

#[test]
fn opening_and_creating_replace_the_session() {
    let (mut state, path) = saved_project("replace");
    let before = session(&state);
    let id = state.project.id.clone();

    ok(&mut state, json!({"op":"project.open","path":path}));
    let opened = session(&state);
    assert_ne!(opened, before);
    assert_eq!(state.project.id, id, "same document, same ID");
    assert_eq!(state.project.revision, 0);

    ok(&mut state, json!({"op":"project.open","path":path}));
    assert_ne!(
        session(&state),
        opened,
        "opening the same document again is another session"
    );

    let current = session(&state);
    ok(&mut state, json!({"op":"project.create","name":"fresh"}));
    assert_ne!(session(&state), current);
    // The status answer carries it so a script (or a process test) can see it change.
    let status = ok(&mut state, json!({"op":"history.status"}));
    assert_eq!(status["project"]["session"], session(&state));
    assert_eq!(status["project"]["id"], json!(state.project.id));
}

#[test]
fn a_failed_open_keeps_the_session_it_was_in() {
    let (mut state, path) = saved_project("failed-open");
    let before = session(&state);
    let stamp = state.project.stamp();
    let broken = path.with_file_name("broken.tessel.json");
    fs::write(&broken, "{ not a document").unwrap();
    assert!(headless::execute(&mut state, &json!({"op":"project.open","path":broken})).is_err());
    assert_eq!(session(&state), before);
    assert!(
        stamp.is_current(&state.project),
        "a rejected open changes nothing"
    );
}
