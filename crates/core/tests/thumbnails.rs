//! BL144: the rules of library thumbnails and the header identity, through the library and the
//! JSON executor (no window, no renderer): what is wanted, whether a result still belongs, and
//! what the header says after another project is opened.

use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};
use tessel_editor_core::fixtures::{StandardSet, proof_requests, write_standard_set};
use tessel_editor_core::thumbnails::{Failure, Thumbnail, WIDTH, size_for};
use tessel_editor_core::{CommandError, EditorState, headless};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tessel-thumbs-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(state: &mut EditorState, request: Value) -> Result<Value, CommandError> {
    headless::execute(state, &request)
}

fn ok(state: &mut EditorState, request: Value) -> Value {
    run(state, request.clone()).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

/// The viewport proof project, with its files "hashed" the way the media import would.
fn project(set: &StandardSet) -> EditorState {
    let mut state = EditorState::demo();
    for request in proof_requests(set) {
        ok(&mut state, request);
    }
    for item in &mut state.library.items {
        if let Some(asset) = item.asset.as_mut() {
            asset.identity.hash = Some(format!("hash-of-{}", item.id));
        }
    }
    state
}

fn picture(state: &EditorState, id: &str, path: &str) -> Thumbnail {
    Thumbnail {
        path: path.to_string(),
        width: WIDTH,
        height: size_for(state.timeline.width, state.timeline.height).1,
        identity: state.thumbnail_identity(id).expect("an identity"),
    }
}

fn ids(state: &EditorState, retry: bool) -> Vec<String> {
    state
        .thumbnail_targets(retry)
        .into_iter()
        .map(|target| target.asset)
        .collect()
}

#[test]
fn only_hashed_splats_and_scenes_are_wanted() {
    let dir = temp("targets");
    let set = write_standard_set(&dir).unwrap();
    let mut state = project(&set);
    assert_eq!(ids(&state, false), ["cloud", "scn"]);
    for id in ["img", "vid", "tone", "label"] {
        assert_eq!(state.thumbnail_report(id)["state"], "none", "{id}");
    }

    // A file nobody hashed has no identity: the cache is addressed by content hash.
    state.library.items[3].asset.as_mut().unwrap().identity.hash = None;
    assert!(
        state.thumbnail_identity("cloud").is_none(),
        "the unhashed splat has no identity"
    );
    assert_eq!(ids(&state, false), ["scn"]);
}

#[test]
fn the_identity_follows_the_file_and_the_picture_size_not_the_edits() {
    let dir = temp("identity");
    let set = write_standard_set(&dir).unwrap();
    let mut state = project(&set);
    let before = state.thumbnail_identity("cloud").unwrap();
    let revision = state.project.revision;

    // An edit that does not change the picture keeps the identity.
    ok(
        &mut state,
        json!({"op":"clip.property","clip":"c-img","property":"zoom","value":80}),
    );
    assert_ne!(state.project.revision, revision);
    assert_eq!(state.thumbnail_identity("cloud").unwrap(), before);

    // Another file, or another aspect ratio, is another picture.
    state.library.items[3].asset.as_mut().unwrap().identity.hash = Some("another".into());
    let relinked = state.thumbnail_identity("cloud").unwrap();
    assert_ne!(relinked, before);
    state.timeline.width = 1080;
    state.timeline.height = 1920;
    assert_ne!(state.thumbnail_identity("cloud").unwrap(), relinked);
    assert_eq!(
        state.thumbnail_targets(false)[0].height,
        size_for(1080, 1920).1
    );
}

#[test]
fn a_stored_thumbnail_is_shown_only_while_it_is_the_picture_of_the_asset() {
    let dir = temp("show");
    let set = write_standard_set(&dir).unwrap();
    let mut state = project(&set);
    let stamp = state.thumbnail_stamp();
    let revision = state.project.revision;
    let document = state.to_document(Some(&dir));

    state
        .set_thumbnail(
            &stamp,
            "cloud",
            picture(&state, "cloud", "/cache/cloud.png"),
        )
        .unwrap();
    assert_eq!(state.thumbnail("cloud").unwrap().path, "/cache/cloud.png");
    assert_eq!(state.thumbnail_report("cloud")["state"], "ready");
    assert_eq!(
        ids(&state, false),
        ["scn"],
        "a drawn splat is not asked for again"
    );

    // Session state: no edit, nothing in the document, and undo has nothing to give back.
    assert_eq!(state.project.revision, revision);
    assert_eq!(state.to_document(Some(&dir)), document);

    // The file is replaced (relink): the old picture is hidden at once and wanted again.
    state.library.items[3].asset.as_mut().unwrap().identity.hash = Some("replacement".into());
    assert!(state.thumbnail("cloud").is_none());
    assert_eq!(state.thumbnail_report("cloud")["state"], "pending");
    assert_eq!(ids(&state, false), ["cloud", "scn"]);

    // Forgetting what no longer matches keeps the book from growing.
    assert!(!state.thumbnails.is_empty());
    state.prune_thumbnails();
    assert!(state.thumbnails.is_empty());
}

#[test]
fn a_result_of_another_project_or_session_is_refused_and_changes_nothing() {
    let dir = temp("stale");
    let set = write_standard_set(&dir).unwrap();
    let mut first = project(&set);
    let stamp = first.thumbnail_stamp();
    let made = picture(&first, "cloud", "/cache/first.png");

    // A different project with the same asset IDs and hashes: same names, other project.
    let mut other = project(&set);
    let refused = other
        .set_thumbnail(&stamp, "cloud", made.clone())
        .unwrap_err();
    assert_eq!(refused.code, "stale_result", "{refused:?}");
    assert!(other.thumbnails.is_empty());
    assert!(other.thumbnail("cloud").is_none());

    // The same document opened again is a new session even with the same ID and revision.
    let document = dir.join("film.tessel.json");
    ok(&mut first, json!({"op":"project.save","path":document}));
    let mut reopened = EditorState::demo();
    ok(&mut reopened, json!({"op":"project.open","path":document}));
    assert_eq!(reopened.project.id, first.project.id);
    for item in &mut reopened.library.items {
        if let Some(asset) = item.asset.as_mut() {
            asset.identity.hash = Some(format!("hash-of-{}", item.id));
        }
    }
    let refused = reopened
        .set_thumbnail(&stamp, "cloud", made.clone())
        .unwrap_err();
    assert_eq!(refused.code, "stale_result");
    assert!(reopened.thumbnails.is_empty());
    assert_eq!(refused.target.as_deref(), Some("cloud"));

    // The project it was drawn for still takes it.
    first.set_thumbnail(&stamp, "cloud", made).unwrap();
    assert!(first.thumbnail("cloud").is_some());
}

#[test]
fn a_result_for_an_asset_that_changed_while_it_was_drawn_is_refused() {
    let dir = temp("changed");
    let set = write_standard_set(&dir).unwrap();
    let mut state = project(&set);
    let stamp = state.thumbnail_stamp();
    let made = picture(&state, "cloud", "/cache/old.png");
    state.library.items[3].asset.as_mut().unwrap().identity.hash = Some("newer".into());

    let refused = state.set_thumbnail(&stamp, "cloud", made).unwrap_err();
    assert_eq!(refused.code, "stale_result");
    assert!(state.thumbnail("cloud").is_none());
    let refused = state
        .set_thumbnail(&stamp, "ghost", picture(&state, "scn", "/cache/x.png"))
        .unwrap_err();
    assert_eq!(refused.code, "stale_result", "an asset that is gone");
}

#[test]
fn a_failure_is_remembered_for_that_picture_and_retried_on_request() {
    let dir = temp("failure");
    let set = write_standard_set(&dir).unwrap();
    let mut state = project(&set);
    let stamp = state.thumbnail_stamp();
    let identity = state.thumbnail_identity("scn").unwrap();
    state
        .set_thumbnail_failure(
            &stamp,
            "scn",
            Failure {
                identity: identity.clone(),
                code: "backend_unavailable".into(),
                message: "run ./scripts/runtime setup".into(),
            },
        )
        .unwrap();

    let report = state.thumbnail_report("scn");
    assert_eq!(report["state"], "failed");
    assert_eq!(report["code"], "backend_unavailable");
    assert_eq!(
        ids(&state, false),
        ["cloud"],
        "not asked for again every tick"
    );
    assert_eq!(ids(&state, true), ["cloud", "scn"], "unless asked to retry");

    // Another file is another picture: the old failure does not apply to it.
    state.library.items[4].asset.as_mut().unwrap().identity.hash = Some("fixed".into());
    assert_eq!(state.thumbnail_report("scn")["state"], "pending");
    assert!(ids(&state, false).contains(&"scn".to_string()));

    // A success replaces the failure.
    let stamp = state.thumbnail_stamp();
    state
        .set_thumbnail(&stamp, "scn", picture(&state, "scn", "/cache/scn.png"))
        .unwrap();
    assert_eq!(state.thumbnail_report("scn")["state"], "ready");
}

#[test]
fn the_header_follows_the_project_that_is_open() {
    let dir = temp("header");
    let mut state = EditorState::demo();
    let demo = ok(&mut state, json!({"op":"project.header"}));
    assert_eq!(demo["label"], "North expedition / Master sequence");
    assert_eq!(ok(&mut state, json!({"op":"status"}))["header"], demo);

    ok(
        &mut state,
        json!({"op":"project.create","name":"Rain film","sequence":"cut-one","rate":24,
            "width":320,"height":180,"length":48}),
    );
    let created = ok(&mut state, json!({"op":"project.header"}));
    assert_eq!(created["project"], "Rain film");
    assert_eq!(created["sequence"], "cut-one");
    assert_eq!(created["label"], "Rain film / cut-one");
    assert_eq!(created["dirty"], true);
    assert!(created["document"].is_null());
    assert_ne!(created["project_id"], demo["project_id"]);

    // Saving, then opening that document in a fresh session, names the same project.
    let document = dir.join("rain.tessel.json");
    ok(&mut state, json!({"op":"project.save","path":document}));
    let saved = ok(&mut state, json!({"op":"project.header"}));
    assert_eq!(saved["dirty"], false);
    assert_eq!(saved["document"], json!(document));
    let mut reopened = EditorState::demo();
    ok(&mut reopened, json!({"op":"project.open","path":document}));
    let opened = ok(&mut reopened, json!({"op":"project.header"}));
    assert_eq!(opened["label"], "Rain film / cut-one");
    assert_eq!(opened["project_id"], created["project_id"]);

    // Adding and activating another sequence changes the second half, not the project.
    ok(
        &mut reopened,
        json!({"op":"sequence.add","id":"cut-two","name":"Cut two","width":180,"height":320,"activate":true}),
    );
    assert_eq!(
        ok(&mut reopened, json!({"op":"project.header"}))["label"],
        "Rain film / Cut two"
    );
}
