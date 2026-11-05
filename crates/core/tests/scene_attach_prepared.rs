//! BL100 (core side): a bundle can be read, parsed and validated away from the project and then
//! attached cheaply. `AdmittedScene::prepare` is the heavy half of `scene.attach`; attaching its
//! result (`SceneSource::Admitted`) is the same core command with the same ID, room and budget
//! rules, one undo entry, and no second validation.
//!
//! Everything runs through the public library (no window, no GPU, no media crate). The job that
//! drives `prepare` off-thread, and the project/revision guard, are proven in
//! `crates/media/tests/scene_attach_job.rs`.

use core::prelude::v1::test;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tessel_editor_core::jobs::{CancelToken, JobError};
use tessel_editor_core::scene_project::{AdmittedScene, SceneCommand, SceneSource};
use tessel_editor_core::{Command, CommandError, EditorState, headless};

fn bundle_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json")
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tessel-attach-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn project() -> EditorState {
    let mut state = EditorState::demo();
    headless::execute(&mut state, &json!({"op":"project.create","name":"attach"})).unwrap();
    state
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

#[test]
fn an_admitted_scene_attaches_exactly_like_the_path_it_came_from() {
    let path = bundle_path();
    let admitted = AdmittedScene::prepare(SceneSource::Path(path.clone()), &CancelToken::new())
        .expect("a valid bundle is admitted");

    let mut direct = project();
    let expected = attach(&mut direct, SceneSource::Path(path), "street").unwrap();
    let mut prepared = project();
    let answer = attach(&mut prepared, SceneSource::Admitted(admitted), "street").unwrap();

    assert_eq!(answer, expected, "the attach answers the same");
    assert_eq!(
        prepared.scenes.get("street").unwrap().scene,
        direct.scenes.get("street").unwrap().scene,
        "the same scene is held"
    );
    assert_eq!(undo_depth(&mut prepared), 1, "one undo entry");
    headless::execute(&mut prepared, &json!({"op":"history.undo"})).unwrap();
    assert!(prepared.scenes.get("street").is_none());
}

#[test]
fn preparing_refuses_what_attach_refuses_with_the_same_codes() {
    let dir = temp("codes");
    let not_a_bundle = dir.join("wrong.json");
    fs::write(&not_a_bundle, r#"{"bundleVersion": 99}"#).unwrap();
    let not_json = dir.join("broken.json");
    fs::write(&not_json, "{ not json").unwrap();
    let missing = dir.join("nowhere.json");
    let not_utf8 = dir.join("binary.json");
    fs::write(&not_utf8, [0xFF, 0xFE, 0x00]).unwrap();

    for (path, code) in [
        (not_a_bundle, "invalid_bundle"),
        (not_json, "invalid_json"),
        (missing, "bundle_unreadable"),
        (not_utf8, "bundle_unreadable"),
    ] {
        let prepared = AdmittedScene::prepare(SceneSource::Path(path.clone()), &CancelToken::new())
            .unwrap_err();
        let mut state = project();
        let attached = attach(&mut state, SceneSource::Path(path.clone()), "street").unwrap_err();
        assert_eq!(prepared.code, code, "{}", path.display());
        assert_eq!(prepared.code, attached.code, "{}", path.display());
        // The refusal survives the trip through a job error (what the media host does).
        let through_job = CommandError::from(JobError::from(prepared));
        assert_eq!(through_job.code, code, "{}", path.display());
    }
}

#[test]
fn a_file_over_the_limit_is_refused_before_it_is_read() {
    let dir = temp("too-large");
    let huge = dir.join("huge.json");
    // Sparse: the size is what matters, and nothing is allocated for it.
    let file = fs::File::create(&huge).unwrap();
    file.set_len(33 * 1024 * 1024).unwrap();
    let error = AdmittedScene::prepare(SceneSource::Path(huge), &CancelToken::new()).unwrap_err();
    assert_eq!(error.code, "bundle_too_large", "{error:?}");
}

#[test]
fn a_cancelled_preparation_holds_no_scene_and_changes_no_project() {
    let token = CancelToken::new();
    token.cancel();
    let error = AdmittedScene::prepare(SceneSource::Path(bundle_path()), &token).unwrap_err();
    assert_eq!(error.code, "cancelled");
    assert!(JobError::from(error).is_cancelled());

    // Cancelled while a file is coming in: a large valid-size file is stopped at the first block.
    let dir = temp("cancel-read");
    let big = dir.join("big.json");
    fs::File::create(&big)
        .unwrap()
        .set_len(8 * 1024 * 1024)
        .unwrap();
    let error = AdmittedScene::prepare(SceneSource::Path(big), &token).unwrap_err();
    assert_eq!(error.code, "cancelled");
}

#[test]
fn an_admitted_scene_still_meets_the_projects_id_rules() {
    let path = bundle_path();
    let admit = || AdmittedScene::prepare(SceneSource::Path(path.clone()), &CancelToken::new());
    let mut state = project();
    attach(
        &mut state,
        SceneSource::Admitted(admit().unwrap()),
        "street",
    )
    .unwrap();
    let depth = undo_depth(&mut state);
    let revision = state.project.revision;

    // Taken ID: refused by the core, nothing changes.
    let taken = attach(
        &mut state,
        SceneSource::Admitted(admit().unwrap()),
        "street",
    )
    .unwrap_err();
    assert_eq!(taken.code, "duplicate_id", "{taken:?}");
    // A malformed ID and a blank name are refused too.
    let malformed = attach(
        &mut state,
        SceneSource::Admitted(admit().unwrap()),
        "bad id!",
    )
    .unwrap_err();
    assert_eq!(malformed.code, "invalid_id", "{malformed:?}");
    let blank = state.execute(Command::Scene(SceneCommand::Attach {
        id: Some("other".into()),
        name: Some("  ".into()),
        source: SceneSource::Admitted(admit().unwrap()),
    }));
    assert!(blank.is_err());

    assert_eq!(undo_depth(&mut state), depth);
    assert_eq!(state.project.revision, revision);
    assert_eq!(state.scenes.ids().len(), 1);
}
