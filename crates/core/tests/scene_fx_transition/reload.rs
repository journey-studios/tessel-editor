//! L (file doors): a transition that arrives **through a file** is refused by the same gate as one
//! that arrives through the executor, and a refusal leaves the project untouched.
//!
//! `public.rs` covers the importers and the in-memory attach; this file covers what text cannot
//! prove by itself and what a text scan of the sources (`boundary.rs`) only guards architecturally:
//!
//! * a hand-edited `.tessel.json` through `persist::load`, `EditorState::from_document` and the
//!   executor's `project.open` (on a fresh and on a populated project);
//! * a bundle file through `AdmittedScene::prepare` (the file handler of `SceneSource::Path`),
//!   the shared `Command::Scene(Attach)` and the executor's `scene.attach` by `path`.
//!
//! Each refusal is paired with a control that succeeds through the same door, so a door that is
//! simply broken cannot pass as "refused". The fixtures are the synthetic edit of `support.rs`
//! (nothing from `projects/`), written under a scratch directory this test owns and removes.

use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::fx::FX_TRANSITION_NOT_YET;
use tessel_editor_core::jobs::CancelToken;
use tessel_editor_core::scene_project::{AdmittedScene, SceneCommand, SceneSource};
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::{Command, CommandError, EditorState, headless, persist};

use crate::support::*;

const NOT_YET: &str =
    "transitions between shots are not implemented yet; leave `transitions` empty";

/// A directory under the system temp dir, named for this test and process and removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("tessel-p4b-s1a-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn file(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, text).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn ok(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn empty() -> EditorState {
    EditorState::empty(&ProjectSettings::default()).unwrap()
}

/// What a refusal must leave alone: the scene documents, the revision and the history.
fn snapshot(state: &mut EditorState) -> (Vec<(String, Value)>, Value, Value) {
    let scenes = state
        .scenes
        .ids()
        .iter()
        .map(|id| {
            (
                id.to_string(),
                state.scenes.get(id).unwrap().scene.document().clone(),
            )
        })
        .collect();
    (
        scenes,
        ok(state, json!({"op": "status"}))["persistence"]["revision"].clone(),
        ok(state, json!({"op": "history.status"})),
    )
}

/// The refusal every door gives, with the message it always had, at the path it always had.
fn assert_not_yet(error: &CommandError) {
    assert_eq!(error.code, "invalid_bundle", "{error:?}");
    let issues = error.details.as_ref().expect("details")["issues"]
        .as_array()
        .expect("issues")
        .clone();
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert_eq!(issues[0]["code"], FX_TRANSITION_NOT_YET, "{issues:?}");
    assert_eq!(issues[0]["path"], "/fx/transitions", "{issues:?}");
    assert_eq!(issues[0]["message"], NOT_YET, "{issues:?}");
}

fn transitions_bundle() -> Value {
    with_transitions(Edit::spec().bundle(), vec![tr(json!({}))])
}

/// A saved project with one plain scene (`street`), and its stored document.
fn saved_project(dir: &Scratch) -> (PathBuf, Value) {
    let mut state = empty();
    ok(
        &mut state,
        json!({"op": "scene.attach", "id": "street", "bundle": Edit::spec().bundle()}),
    );
    let path = dir.0.join("plain.tessel.json");
    persist::save(&mut state, &path).unwrap();
    let document = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    (path, document)
}

/// The stored document with a transition written by hand into the stored bundle.
fn hand_edited(document: &Value) -> Value {
    let mut hacked = document.clone();
    let bundle = &mut hacked["scenes"]["items"][0]["bundle"];
    assert!(bundle.is_object(), "the stored scene has a bundle");
    *bundle = transitions_bundle();
    hacked
}

#[test]
fn a_stored_project_with_a_transition_is_refused_by_every_reload_door() {
    let dir = Scratch::new("reload");
    let (plain, document) = saved_project(&dir);
    let hacked = hand_edited(&document);
    let hacked_path = dir.file(
        "hacked.tessel.json",
        &serde_json::to_string_pretty(&hacked).unwrap(),
    );

    // Controls: the untouched file and an empty `transitions` list go through every door.
    assert!(persist::load(&plain).is_ok(), "the plain project reloads");
    assert!(EditorState::from_document(&document, None).is_ok());
    let mut empty_list = document.clone();
    empty_list["scenes"]["items"][0]["bundle"] =
        with_transitions(Edit::spec().bundle(), Vec::new());
    assert!(EditorState::from_document(&empty_list, None).is_ok());

    // The hand-edited file: `persist::load`, `from_document`, and `project.open`.
    let Err(error) = persist::load(&hacked_path) else {
        panic!("persist::load must refuse");
    };
    assert_not_yet(&error);
    let Err(error) = EditorState::from_document(&hacked, None) else {
        panic!("from_document must refuse");
    };
    assert_not_yet(&error);
    let mut fresh = empty();
    let error = headless::execute(
        &mut fresh,
        &json!({"op": "project.open", "path": hacked_path}),
    )
    .unwrap_err();
    assert_not_yet(&error);
    assert!(
        fresh.scenes.ids().is_empty(),
        "a refused open attaches nothing"
    );

    // On a populated project a refused open changes nothing: scenes, revision, history.
    let mut open = empty();
    ok(
        &mut open,
        json!({"op": "scene.attach", "id": "keep", "bundle": Edit::spec().bundle()}),
    );
    let before = snapshot(&mut open);
    let error = headless::execute(
        &mut open,
        &json!({"op": "project.open", "path": hacked_path}),
    )
    .unwrap_err();
    assert_not_yet(&error);
    assert_eq!(
        snapshot(&mut open),
        before,
        "the refused open left no trace"
    );
    assert!(open.scenes.get("keep").is_some());

    // The same door accepts the plain file, so the refusal above was about the transition.
    ok(&mut open, json!({"op": "project.open", "path": plain}));
    assert_eq!(
        open.scenes.ids(),
        ["street"],
        "the plain project replaced the open one"
    );
}

#[test]
fn a_bundle_file_with_a_transition_is_refused_by_every_file_door() {
    let dir = Scratch::new("attach");
    let text = transitions_bundle().to_string();
    let hacked = dir.file("transitions.bundle.json", &text);
    let plain = dir.file("plain.bundle.json", &Edit::spec().bundle().to_string());
    let sources = || {
        [
            ("path", SceneSource::Path(hacked.clone())),
            ("text", SceneSource::Text(text.clone())),
            ("bundle", SceneSource::Bundle(transitions_bundle())),
        ]
    };

    // The file handler by itself (`AdmittedScene::prepare` is what jobs and hosts run).
    for (door, source) in sources() {
        let error = AdmittedScene::prepare(source, &CancelToken::new())
            .err()
            .unwrap_or_else(|| panic!("prepare/{door} must refuse"));
        assert_not_yet(&error);
    }
    assert!(
        AdmittedScene::prepare(SceneSource::Path(plain.clone()), &CancelToken::new()).is_ok(),
        "control: the same handler admits a plain bundle file"
    );

    // The shared command, on a project that already holds a scene.
    let mut state = empty();
    ok(
        &mut state,
        json!({"op": "scene.attach", "id": "street", "bundle": Edit::spec().bundle()}),
    );
    let before = snapshot(&mut state);
    for (door, source) in sources() {
        let error = state
            .execute(Command::Scene(SceneCommand::Attach {
                id: Some("other".into()),
                name: None,
                source,
            }))
            .err()
            .unwrap_or_else(|| panic!("attach/{door} must refuse"));
        assert_not_yet(&error);
        assert_eq!(snapshot(&mut state), before, "attach/{door} left a trace");
    }

    // The executor's `scene.attach` by `path`, and the other two spellings of the same bundle.
    for request in [
        json!({"op": "scene.attach", "id": "other", "path": hacked}),
        json!({"op": "scene.attach", "id": "other", "text": text}),
        json!({"op": "scene.attach", "id": "other", "bundle": transitions_bundle()}),
    ] {
        let error = headless::execute(&mut state, &request).unwrap_err();
        assert_not_yet(&error);
        assert_eq!(snapshot(&mut state), before, "{request} left a trace");
    }
    assert!(state.scenes.get("other").is_none());

    // Controls through the same two doors: a plain file attaches, one undo entry, one revision.
    ok(
        &mut state,
        json!({"op": "scene.attach", "id": "other", "path": plain}),
    );
    assert!(state.scenes.get("other").is_some());
    let after = snapshot(&mut state);
    assert_ne!(after.1, before.1, "the control attach is a real edit");
}
