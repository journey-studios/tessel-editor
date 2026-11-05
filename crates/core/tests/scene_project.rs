//! F036: directed scenes inside a project, through the public library API and the JSON executor
//! (in process, no window, no GPU, no Chrome).
//!
//! What this file proves, and what it does not:
//!
//! - **Consumer flow**: create a project, attach bundles by ID (inline, as text and by file),
//!   list, query catalog / properties / one property / camera, edit in batches with dry-run,
//!   undo and redo, save and reopen. Every `scene.*` request goes through `headless::execute`,
//!   the executor `tessel-headless` and `ui op` share.
//! - **Equivalence with the origin** only for the camera answers: the expected numbers come from
//!   the oracle corpus (`docs/migration/fixtures`, produced by the origin's own functions, hashes
//!   pinned by `scripts/migration-oracle`), compared with the same tolerance as
//!   `directed_scene_temporal.rs` (`abs <= 1e-9 + 1e-12 * |expected|`). Everything else here is
//!   behaviour of this feature, not equivalence.
//! - **Not covered**: the desktop automation socket and any visual surface (they need the
//!   desktop); packaging guards live in `crates/media/tests/scene_project_guards.rs`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command as Process, Stdio};

use serde_json::{Value, json};
use tessel_editor_core::scene_project::{MAX_SCENES, SceneCommand, SceneSource};
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::{Command, CommandError, EditorState, headless, persist};

/// A scene whose continuous cut has a deliberate velocity jump: it imports, and only the temporal
/// gate refuses it (shared with `scene_edit.rs`).
const LAYERED: &str = include_str!("directed_scene_fixtures/layered_scene.json");

const MINIMAL: &str = "valid-minimal-continuous";
const OFFSETS: &str = "valid-cut-instance-offsets";

const ABSOLUTE: f64 = 1e-9;
const RELATIVE: f64 = 1e-12;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/migration/fixtures")
}

fn bundle_path(name: &str) -> PathBuf {
    fixtures()
        .join("bundles")
        .join(format!("{name}.bundle.json"))
}

fn read_json(path: impl AsRef<Path>) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn bundle(name: &str) -> Value {
    read_json(bundle_path(name))
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tessel-scene-{name}-{}", std::process::id()));
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

fn err(state: &mut EditorState, request: Value) -> CommandError {
    match run(state, request.clone()) {
        Ok(answer) => panic!("{request} should have failed, got {answer}"),
        Err(error) => error,
    }
}

fn new_project() -> EditorState {
    let mut state = EditorState::demo();
    ok(&mut state, json!({"op":"project.create","name":"Scenes"}));
    state
}

fn attach(state: &mut EditorState, id: &str, file: &str) -> Value {
    ok(
        state,
        json!({"op":"scene.attach","id":id,"bundle":bundle(file)}),
    )
}

fn attach_command(id: &str, file: &str) -> Command {
    Command::Scene(SceneCommand::Attach {
        id: Some(id.to_string()),
        name: None,
        source: SceneSource::Bundle(bundle(file)),
    })
}

fn document_of(state: &EditorState, id: &str) -> Value {
    state.scenes.get(id).unwrap().scene.document().clone()
}

fn undo_depth(state: &EditorState) -> u64 {
    state.history.to_json()["undo_depth"].as_u64().unwrap()
}

fn details(error: &CommandError) -> &Value {
    error.details.as_deref().expect("the error carries details")
}

fn issue_codes(error: &CommandError) -> Vec<String> {
    details(error)["issues"]
        .as_array()
        .expect("issues")
        .iter()
        .map(|issue| issue["code"].as_str().unwrap().to_string())
        .collect()
}

/// Everything a rejected, dry-run or no-op command must leave exactly as it was.
fn observable(state: &EditorState) -> (Value, Value, u64, Value) {
    (
        state.to_document(None),
        state.history.to_json(),
        state.project.revision,
        state.summary(),
    )
}

fn close(label: &str, actual: &Value, expected: &Value) {
    let (actual, expected) = (actual.as_f64().unwrap(), expected.as_f64().unwrap());
    let error = (actual - expected).abs();
    assert!(
        error <= ABSOLUTE + RELATIVE * expected.abs(),
        "{label}: {actual} != {expected} (error {error:e})"
    );
}

fn close_vec3(label: &str, actual: &Value, expected: &Value) {
    for axis in ["x", "y", "z"] {
        close(&format!("{label}.{axis}"), &actual[axis], &expected[axis]);
    }
}

// ---- the consumer flow --------------------------------------------------------------------

#[test]
fn a_project_keeps_its_scenes_through_save_and_reopen() {
    let dir = temp("roundtrip");
    let mut state = new_project();
    let mut raw = bundle(MINIMAL);
    raw["x-tool"] = json!({ "keep": [1, 2, 3] });

    let attached = ok(
        &mut state,
        json!({"op":"scene.attach","id":"street","name":"Street","bundle":raw}),
    );
    assert_eq!(attached["scene"], "street");
    assert_eq!(attached["name"], "Street");
    assert_eq!(attached["changed"], true);
    assert_eq!(attached["counts"]["nodes"], 3);
    assert_eq!(attached["counts"]["shots"], 2);
    let notices = attached["notices"].as_array().unwrap();
    assert!(
        notices
            .iter()
            .any(|notice| notice["code"] == "UNKNOWN_FIELD"),
        "an unknown field is reported, not silently dropped: {notices:?}"
    );
    assert!(
        !attached["scope"]["deferred_fields"]
            .as_array()
            .unwrap()
            .is_empty(),
        "attach declares what is deferred"
    );
    assert!(
        !attached["scope"]["not_enabled"]
            .as_array()
            .unwrap()
            .is_empty(),
        "attach declares what is not enabled"
    );
    assert_eq!(attached["deferred_in_use"], json!([]));

    // By file, without a name: the bundle's own ID labels it.
    let second = ok(
        &mut state,
        json!({"op":"scene.attach","id":"offsets","path":bundle_path(OFFSETS)}),
    );
    assert_eq!(second["name"], bundle(OFFSETS)["bundleId"]);
    assert_eq!(
        state.scenes.ids(),
        ["street", "offsets"],
        "attach order is kept"
    );
    assert!(state.project.is_dirty());

    let doc = dir.join("scenes.tessel.json");
    ok(&mut state, json!({"op":"project.save","path":doc}));
    assert!(!state.project.is_dirty());
    let stored = read_json(&doc);
    assert_eq!(stored["scenes"]["version"], 1);
    assert_eq!(stored["scenes"]["items"][0]["id"], "street");
    assert_eq!(
        stored["scenes"]["items"][0]["bundle"]["x-tool"],
        json!({ "keep": [1, 2, 3] }),
        "an unknown field of the bundle is stored as it came"
    );
    assert_eq!(
        stored["schema_version"], 2,
        "legacy readers must refuse scenes rather than erase them"
    );

    let mut reopened = EditorState::demo();
    ok(&mut reopened, json!({"op":"project.open","path":doc}));
    assert_eq!(reopened.scenes.ids(), ["street", "offsets"]);
    assert_eq!(
        state.to_document(Some(&dir)),
        reopened.to_document(Some(&dir)),
        "document -> state -> document is the identity"
    );
    for id in ["street", "offsets"] {
        assert_eq!(
            document_of(&state, id),
            document_of(&reopened, id),
            "{id} came back byte-equal as data"
        );
        let request = json!({"op":"scene.describe","scene":id});
        assert_eq!(ok(&mut state, request.clone()), ok(&mut reopened, request));
    }
    assert!(
        !reopened.history.can_undo(),
        "a reopened project starts with a clean history"
    );

    let first = fs::read(&doc).unwrap();
    ok(&mut reopened, json!({"op":"project.save"}));
    assert_eq!(
        first,
        fs::read(&doc).unwrap(),
        "saving again writes the same bytes"
    );
    fs::remove_dir_all(dir).ok();
}

#[test]
fn a_project_without_scenes_writes_what_it_always_wrote() {
    let mut state = new_project();
    let before = state.to_document(None);
    assert!(
        before.get("scenes").is_none(),
        "no `scenes` key: older editors read it as before"
    );
    let mut keys: Vec<&str> = before
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "active_sequence",
            "clip_properties",
            "export",
            "format",
            "library",
            "presentation",
            "project",
            "schema_version",
            "sequences",
        ],
        "the top-level shape of a version 1 document"
    );
    assert!(state.summary().get("scenes").is_none());

    // Such a document (what every earlier editor wrote) opens with no scenes.
    let reopened = EditorState::from_document(&before, None).unwrap();
    assert!(reopened.scenes.is_empty());
    assert_eq!(reopened.to_document(None), before);

    // Attaching and removing leaves the document as it was.
    attach(&mut state, "street", MINIMAL);
    assert!(state.to_document(None).get("scenes").is_some());
    assert_eq!(state.summary()["scenes"], json!(["street"]));
    ok(&mut state, json!({"op":"scene.detach","scene":"street"}));
    assert_eq!(state.to_document(None), before);
}

#[test]
fn the_stored_block_is_validated_like_an_attach_and_never_half_opens() {
    let mut state = new_project();
    attach(&mut state, "street", MINIMAL);
    let good = state.to_document(None);
    assert!(EditorState::from_document(&good, None).is_ok());

    let mut future = good.clone();
    future["scenes"]["version"] = json!(2);
    let error = EditorState::from_document(&future, None).unwrap_err();
    assert_eq!(error.code, "unsupported_schema");
    assert_eq!(details(&error)["found"], 2);

    let mut broken = good.clone();
    broken["scenes"]["items"][0]["bundle"]["schemaVersion"] = json!("9.9");
    let error = EditorState::from_document(&broken, None).unwrap_err();
    assert_eq!(error.code, "invalid_bundle", "the importer's own code");
    assert_eq!(error.target.as_deref(), Some("scene:street"));
    assert!(issue_codes(&error).contains(&"UNSUPPORTED_SCHEMA_VERSION".to_string()));

    // A bundle that imports but fails the structural gate cannot sneak in through a document.
    let mut gated = good.clone();
    gated["scenes"]["items"][0]["bundle"] = bundle("invalid-join-missing-transition");
    let error = EditorState::from_document(&gated, None).unwrap_err();
    assert_eq!(error.code, "invalid_bundle");
    assert!(issue_codes(&error).contains(&"MISSING_TRANSITION".to_string()));

    let mut renamed = good.clone();
    renamed["scenes"]["items"][0]["id"] = json!("bad id");
    assert_eq!(
        EditorState::from_document(&renamed, None).unwrap_err().code,
        "invalid_id"
    );

    let mut twice = good.clone();
    let copy = twice["scenes"]["items"][0].clone();
    twice["scenes"]["items"].as_array_mut().unwrap().push(copy);
    assert!(EditorState::from_document(&twice, None).is_err());

    for wrong in [json!(null), json!([]), json!({}), json!({"version": 1})] {
        let mut doc = good.clone();
        doc["scenes"] = wrong;
        assert!(EditorState::from_document(&doc, None).is_err());
    }

    // Through the file, the open project stays as it was.
    let dir = temp("broken-open");
    let path = dir.join("broken.tessel.json");
    fs::write(&path, serde_json::to_vec(&broken).unwrap()).unwrap();
    let mut open = new_project();
    let before = observable(&open);
    let error = err(&mut open, json!({"op":"project.open","path":path}));
    assert_eq!(error.code, "invalid_bundle");
    assert_eq!(observable(&open), before);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn unknown_fields_survive_attach_edit_save_and_reopen() {
    let dir = temp("unknown");
    let mut raw = bundle(MINIMAL);
    raw["x-tool"] = json!({ "name": "synthetic" });
    raw["sceneDefinitions"][0]["nodes"][2]["x-note"] = json!("keep me");
    let mut state = new_project();
    ok(
        &mut state,
        json!({"op":"scene.attach","id":"street","bundle":raw}),
    );
    ok(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[
            {"set":"scene/scene-a/node/figure/position","value":{"x":2,"y":0,"z":0}}]}),
    );

    let doc = dir.join("u.tessel.json");
    persist::save(&mut state, &doc).unwrap();
    let reopened = persist::load(&doc).unwrap();
    let saved = document_of(&reopened, "street");
    assert_eq!(saved["x-tool"]["name"], "synthetic");
    assert_eq!(
        saved["sceneDefinitions"][0]["nodes"][2]["x-note"],
        "keep me"
    );
    assert_eq!(
        saved["sceneDefinitions"][0]["nodes"][2]["localPosition"]["x"],
        2
    );

    // Keys of the block and of an entry that this editor does not know are kept too: a newer
    // editor may add them, and an open/save here must not erase what it cannot read.
    let mut stored = read_json(&doc);
    stored["scenes"]["x-block"] = json!({ "future": true });
    stored["scenes"]["items"][0]["x-entry"] = json!("kept");
    let opened = EditorState::from_document(&stored, None).unwrap();
    let again = opened.to_document(None);
    assert_eq!(again["scenes"]["x-block"], json!({ "future": true }));
    assert_eq!(again["scenes"]["items"][0]["x-entry"], "kept");
    fs::remove_dir_all(dir).ok();
}

// ---- queries ------------------------------------------------------------------------------

#[test]
fn catalog_properties_and_camera_are_queryable_by_id_alone() {
    let mut state = new_project();
    attach(&mut state, "street", MINIMAL);

    let capabilities = ok(&mut state, json!({"op":"scene.capabilities"}));
    assert!(
        capabilities["commands"]
            .as_array()
            .unwrap()
            .contains(&json!("scene.edit"))
    );
    assert_eq!(capabilities["max_scenes"], MAX_SCENES);
    assert!(
        !capabilities["scope"]["not_enabled"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // The catalog is the project's: it answers with no scene attached.
    let catalog = ok(&mut new_project(), json!({"op":"scene.catalog"}));
    let properties = catalog["properties"].as_array().unwrap();
    assert_eq!(catalog["count"], properties.len());
    let fov = properties
        .iter()
        .find(|property| property["name"] == "start/fov")
        .expect("the camera field of view is in the catalog");
    assert_eq!(fov["path"], "camera/<clip>/start/fov");
    assert_eq!(fov["unit"], "degrees");
    assert_eq!(fov["range"]["min"]["inclusive"], false);

    let listed = ok(&mut state, json!({"op":"scene.list"}));
    assert_eq!(listed["count"], 1);
    assert_eq!(listed["scenes"][0]["id"], "street");

    let described = ok(&mut state, json!({"op":"scene.describe","scene":"street"}));
    assert_eq!(described["edit_duration_frames"], 96);
    assert_eq!(described["fps"], json!({"numerator": 24, "denominator": 1}));
    assert_eq!(described["camera_clips"], json!(["cam-main"]));
    assert_eq!(described["shots"].as_array().unwrap().len(), 2);

    let all = ok(
        &mut state,
        json!({"op":"scene.properties","scene":"street"}),
    );
    let paths: Vec<&str> = all["properties"]
        .as_array()
        .unwrap()
        .iter()
        .map(|property| property["path"].as_str().unwrap())
        .collect();
    assert!(paths.contains(&"scene/scene-a/node/figure/position"));
    assert!(paths.contains(&"camera/cam-main/start/fov"));

    let cameras = ok(
        &mut state,
        json!({"op":"scene.properties","scene":"street","prefix":"camera/"}),
    );
    assert_eq!(cameras["count"], 3);
    assert!(
        cameras["properties"]
            .as_array()
            .unwrap()
            .iter()
            .all(|property| property["path"].as_str().unwrap().starts_with("camera/"))
    );

    let one = ok(
        &mut state,
        json!({"op":"scene.get","scene":"street","property":"scene/scene-a/node/figure/position"}),
    );
    assert_eq!(one["value"], json!({"x": 0.5, "y": 0, "z": 0}));
    assert_eq!(one["kind"], "vec3");

    // Errors keep the codes of the layer that found them.
    let before = observable(&state);
    let missing = err(
        &mut state,
        json!({"op":"scene.get","scene":"street","property":"scene/scene-a/node/ghost/position"}),
    );
    assert_eq!(missing.code, "unknown_node");
    assert_eq!(missing.target.as_deref(), Some("ghost"));
    assert_eq!(
        err(&mut state, json!({"op":"scene.describe","scene":"nope"})).code,
        "unknown_element"
    );
    assert_eq!(
        err(
            &mut state,
            json!({"op":"scene.get","scene":"street","property":"scene/scene-a/node/figure/colour"})
        )
        .code,
        "unknown_path"
    );
    assert_eq!(
        observable(&state),
        before,
        "reading never changes the project or its history"
    );
}

#[test]
fn the_camera_of_every_edit_frame_matches_the_origin_oracle() {
    let cases = read_json(fixtures().join("cases.json"))["cases"]
        .as_array()
        .unwrap()
        .clone();
    let (mut frames, mut errors) = (0, 0);
    for case in cases.iter().filter(|case| case["kind"] == "valid") {
        let id = case["id"].as_str().unwrap();
        let expected = read_json(fixtures().join(case["expected"].as_str().unwrap()));
        let mut state = new_project();
        ok(
            &mut state,
            json!({"op":"scene.attach","id":id,"path":fixtures().join(case["bundle"].as_str().unwrap())}),
        );
        for probe in expected["evaluation"]["editFrames"].as_array().unwrap() {
            let edit_frame = probe["editFrame"].as_i64().unwrap();
            let label = format!("{id} edit {edit_frame}");
            let request = json!({"op":"scene.camera","scene":id,"frame":edit_frame});
            if probe["ok"] == false {
                let error = err(&mut state, request);
                assert_eq!(error.code, "frame_out_of_range", "{label}");
                assert_eq!(error.message, probe["error"].as_str().unwrap(), "{label}");
                errors += 1;
                continue;
            }
            let want = &probe["state"];
            let answer = ok(&mut state, request);
            for key in [
                "instanceId",
                "instanceFrame",
                "cameraClipId",
                "cameraClipFrame",
            ] {
                assert_eq!(answer[key], want[key], "{label}: {key}");
            }
            assert_eq!(answer["editFrame"], edit_frame, "{label}");
            let (camera, expected) = (&answer["camera"], &want["camera"]);
            close_vec3(
                &format!("{label}.position"),
                &camera["position"],
                &expected["position"],
            );
            close_vec3(
                &format!("{label}.target"),
                &camera["target"],
                &expected["target"],
            );
            close_vec3(
                &format!("{label}.velocity"),
                &camera["velocity"],
                &expected["velocity"],
            );
            close(
                &format!("{label}.fovDeg"),
                &camera["fovDeg"],
                &expected["fovDeg"],
            );
            close(
                &format!("{label}.fovVelocityDegPerFrame"),
                &camera["fovVelocityDegPerFrame"],
                &expected["fovVelocityDegPerFrame"],
            );
            frames += 1;
        }
    }
    assert!(frames >= 24 && errors >= 8, "corpus coverage decreased");
}

#[test]
fn the_camera_can_be_asked_by_frame_exact_time_or_clip_frame() {
    let mut state = new_project();
    attach(&mut state, "street", MINIMAL);
    let by_frame = ok(
        &mut state,
        json!({"op":"scene.camera","scene":"street","frame":10}),
    );
    // 10 / 24 s, exactly: no float seconds on the way in.
    let by_time = ok(
        &mut state,
        json!({"op":"scene.camera","scene":"street","time":{"num":10,"den":24}}),
    );
    assert_eq!(by_frame, by_time);
    // Shot 1 plays the clip from its first frame, so frame 10 of the edit is frame 10 of the clip.
    let by_clip = ok(
        &mut state,
        json!({"op":"scene.camera","scene":"street","clip":"cam-main","clip_frame":10}),
    );
    assert_eq!(by_clip["camera"], by_frame["camera"]);
    assert_eq!(by_clip["cameraClipFrame"], 10);

    for bad in [
        json!({"op":"scene.camera","scene":"street"}),
        json!({"op":"scene.camera","scene":"street","frame":1,"clip":"cam-main","clip_frame":1}),
        json!({"op":"scene.camera","scene":"street","frame":"ten"}),
    ] {
        assert_eq!(err(&mut state, bad).code, "invalid_request");
    }
    assert_eq!(
        err(
            &mut state,
            json!({"op":"scene.camera","scene":"street","clip":"ghost","clip_frame":0})
        )
        .code,
        "invalid_camera"
    );
    assert_eq!(
        err(
            &mut state,
            json!({"op":"scene.camera","scene":"street","frame":96})
        )
        .code,
        "frame_out_of_range"
    );
}

// ---- edits, history -----------------------------------------------------------------------

#[test]
fn undo_and_redo_move_a_whole_batch_as_one_step() {
    let mut state = new_project();
    attach(&mut state, "street", MINIMAL);
    assert_eq!(undo_depth(&state), 1);
    assert_eq!(state.history.undo_label(), Some("Attach scene"));
    let attached = document_of(&state, "street");
    let revision = state.project.revision;

    let edited = ok(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[
            {"set":"scene/scene-a/node/figure/position","value":{"x":1.5,"y":0,"z":0}},
            {"set":"camera/cam-main/start/fov","value":50},
            {"reset":"scene/scene-a/node/backdrop/scale"}]}),
    );
    assert_eq!(edited["changed"], true);
    assert_eq!(
        edited["changed_ops"], 2,
        "the reset of an unchanged scale changed nothing"
    );
    assert_eq!(edited["changes"].as_array().unwrap().len(), 3);
    assert_eq!(edited["changes"][2]["noop"], true);
    assert_eq!(undo_depth(&state), 2, "one batch, one entry");
    assert_eq!(state.history.undo_label(), Some("Edit scene"));
    assert_eq!(state.project.revision, revision + 1);
    let edited_doc = document_of(&state, "street");
    assert_eq!(edited_doc["cameraClips"][0]["initialFovDeg"], 50);
    assert_eq!(
        edited_doc["sceneDefinitions"][0]["nodes"][2]["localPosition"]["x"],
        1.5
    );

    ok(&mut state, json!({"op":"history.undo"}));
    assert_eq!(document_of(&state, "street"), attached);
    assert_eq!(undo_depth(&state), 1);
    ok(&mut state, json!({"op":"history.redo"}));
    assert_eq!(document_of(&state, "street"), edited_doc);
    ok(&mut state, json!({"op":"history.undo"}));
    ok(&mut state, json!({"op":"history.undo"}));
    assert!(
        state.scenes.is_empty(),
        "undoing the attach removes the scene"
    );
    assert_eq!(
        err(&mut state, json!({"op":"history.undo"})).code,
        "nothing_to_undo"
    );
    ok(&mut state, json!({"op":"history.redo"}));
    assert_eq!(document_of(&state, "street"), attached);
}

#[test]
fn several_scene_commands_inside_a_transaction_are_one_undo_entry() {
    let mut state = new_project();
    state
        .transaction("Two scenes", |state| {
            state.execute(attach_command("a", MINIMAL))?;
            state.execute(attach_command("b", OFFSETS))?;
            Ok(())
        })
        .unwrap();
    assert_eq!(undo_depth(&state), 1);
    assert_eq!(state.scenes.ids(), ["a", "b"]);

    let before = observable(&state);
    let failed = state.transaction("Bad", |state| {
        state.execute(attach_command("c", MINIMAL))?;
        state.execute(attach_command("a", MINIMAL))?;
        Ok(())
    });
    assert_eq!(failed.unwrap_err().code, "duplicate_id");
    assert_eq!(
        observable(&state),
        before,
        "the first attach was rolled back"
    );

    state.undo().unwrap();
    assert!(state.scenes.is_empty());
}

#[test]
fn a_late_invalid_batch_changes_nothing_and_keeps_index_path_and_issue_codes() {
    let mut state = new_project();
    attach(&mut state, "street", MINIMAL);
    let before = observable(&state);
    let scene_before = document_of(&state, "street");

    let unknown = err(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[
            {"set":"scene/scene-a/node/figure/position","value":{"x":3,"y":0,"z":0}},
            {"set":"scene/scene-a/node/backdrop/position","value":{"x":0,"y":1,"z":-2}},
            {"set":"scene/scene-a/node/ghost/position","value":{"x":1,"y":1,"z":1}}]}),
    );
    assert_eq!(unknown.code, "unknown_node");
    assert_eq!(unknown.target.as_deref(), Some("ghost"));
    assert_eq!(details(&unknown)["index"], 2);
    assert_eq!(
        details(&unknown)["path"],
        "scene/scene-a/node/ghost/position"
    );
    assert_eq!(observable(&state), before, "ops 0 and 1 left no trace");

    // Every op is valid; the importer refuses the candidate and names the op it came from.
    let rejected = err(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[
            {"set":"scene/scene-a/node/backdrop/position","value":{"x":0,"y":1,"z":-2}},
            {"set":"scene/scene-a/node/figure/rotation","value":{"x":0,"y":0,"z":0,"w":2}}]}),
    );
    assert_eq!(
        rejected.code, "invalid_bundle",
        "an import of that document says the same"
    );
    assert_eq!(details(&rejected)["index"], 1);
    let finding = details(&rejected)["issues"]
        .as_array()
        .unwrap()
        .iter()
        .find(|issue| issue["code"] == "QUATERNION_NOT_NORMALIZED")
        .expect("the importer's issue code is not renamed");
    assert_eq!(
        finding["ops"],
        json!([1]),
        "attributed to the op that touched it"
    );
    assert_eq!(observable(&state), before);
    assert_eq!(document_of(&state, "street"), scene_before);

    // A batch that cannot be decoded is refused before anything runs.
    for bad in [
        json!({"op":"scene.edit","scene":"street"}),
        json!({"op":"scene.edit","scene":"street","ops":"nope"}),
        json!({"op":"scene.edit","scene":"street","ops":[{"set":"scene/scene-a/node/figure/position"}]}),
        json!({"op":"scene.edit","scene":"street","ops":[],"dry_run":"yes"}),
    ] {
        err(&mut state, bad);
    }
    assert_eq!(
        err(
            &mut state,
            json!({"op":"scene.edit","scene":"nope","ops":[]})
        )
        .code,
        "unknown_element"
    );
    assert_eq!(observable(&state), before);
}

#[test]
fn dry_run_and_no_op_leave_state_and_history_alone() {
    let mut state = new_project();
    attach(&mut state, "street", MINIMAL);
    let before = observable(&state);
    let scene_before = document_of(&state, "street");
    let batch = json!([{"set":"scene/scene-a/node/figure/position","value":{"x":9,"y":9,"z":9}}]);

    let dry = ok(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":batch,"dry_run":true}),
    );
    assert_eq!(dry["dry_run"], true);
    assert_eq!(dry["changed"], false);
    assert_eq!(dry["would_change"], true);
    assert_eq!(
        dry["changes"][0]["before"],
        json!({"x": 0.5, "y": 0, "z": 0})
    );
    assert_eq!(dry["changes"][0]["after"], json!({"x": 9, "y": 9, "z": 9}));
    assert_eq!(observable(&state), before);

    // Succeeds if and only if the real run does: the same refusal, with the same details.
    let bad = json!([
        {"set":"scene/scene-a/node/figure/position","value":{"x":9,"y":9,"z":9}},
        {"set":"scene/scene-a/node/ghost/position","value":{"x":1,"y":1,"z":1}}]);
    let dry_error = err(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":bad,"dry_run":true}),
    );
    let real_error = err(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":bad}),
    );
    assert_eq!(dry_error.to_json(), real_error.to_json());
    assert_eq!(observable(&state), before);

    // Writing the value the property already has, or setting and restoring it in one batch.
    for noop in [
        json!([{"set":"scene/scene-a/node/backdrop/position","value":{"x":0,"y":0,"z":-2}}]),
        json!([
            {"set":"scene/scene-a/node/figure/position","value":{"x":9,"y":9,"z":9}},
            {"set":"scene/scene-a/node/figure/position","value":{"x":0.5,"y":0,"z":0}}]),
        json!([]),
    ] {
        let answer = ok(
            &mut state,
            json!({"op":"scene.edit","scene":"street","ops":noop}),
        );
        assert_eq!(answer["changed"], false, "{noop}");
        assert_eq!(answer["noop"], true, "{noop}");
        assert_eq!(observable(&state), before, "{noop}");
        assert_eq!(document_of(&state, "street"), scene_before, "{noop}");
    }
    assert_eq!(undo_depth(&state), 1, "only the attach is in the history");
}

// ---- attach, IDs, gates, sources ----------------------------------------------------------

#[test]
fn attach_runs_the_importer_and_the_structural_gate_before_anything_changes() {
    let mut state = new_project();
    attach(&mut state, "street", MINIMAL);
    let before = observable(&state);

    let gated = err(
        &mut state,
        json!({"op":"scene.attach","id":"cut","bundle":bundle("invalid-join-missing-transition")}),
    );
    assert_eq!(gated.code, "invalid_bundle");
    assert!(
        issue_codes(&gated).contains(&"MISSING_TRANSITION".to_string()),
        "{:?}",
        issue_codes(&gated)
    );
    let jump = err(
        &mut state,
        json!({"op":"scene.attach","id":"jump","bundle":bundle("invalid-join-camera-clip-offset-and-jump")}),
    );
    let codes = issue_codes(&jump);
    assert!(
        codes.contains(&"CAMERA_CLIP_OFFSET_BREAK".to_string()),
        "{codes:?}"
    );
    let schema = err(
        &mut state,
        json!({"op":"scene.attach","id":"old","bundle":bundle("invalid-schema-version")}),
    );
    assert_eq!(schema.code, "invalid_bundle");
    assert!(issue_codes(&schema).contains(&"UNSUPPORTED_SCHEMA_VERSION".to_string()));
    assert_eq!(observable(&state), before, "no refusal touched the project");
}

#[test]
fn the_temporal_gates_run_on_attach_and_on_every_edit() {
    let raw: Value = serde_json::from_str(LAYERED).unwrap();
    let mut state = new_project();
    let before = observable(&state);
    let jump = err(
        &mut state,
        json!({"op":"scene.attach","id":"street","bundle":raw}),
    );
    assert_eq!(jump.code, "invalid_bundle");
    assert!(
        issue_codes(&jump).contains(&"CONTINUOUS_VELOCITY_JUMP".to_string()),
        "{:?}",
        issue_codes(&jump)
    );
    assert_eq!(
        observable(&state),
        before,
        "the refused scene left no trace"
    );

    // The same scene with a legal cut, and a layer that widens the field of view over the clip.
    let mut baseline = raw.clone();
    baseline["shots"][1]["transitionInto"] = json!({"type": "cut", "reason": "synthetic baseline"});
    baseline["cameraClips"][0]["segments"] =
        json!([{"recipe": "hold", "durationFrames": 48, "from": "initial"}]);
    baseline["cameraClips"][0]["layers"] = json!([{"segments": [
        {"recipe": "zoomOut", "durationFrames": 48, "from": "previousEnd",
         "fovDeltaDeg": 20, "easing": "linear"}
    ]}]);
    ok(
        &mut state,
        json!({"op":"scene.attach","id":"street","bundle":baseline}),
    );
    let attached = observable(&state);

    // Each property is legal alone; the composed camera is not, and the edit gate says so.
    let wide = err(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[
            {"set":"camera/wide/start/fov","value":170}]}),
    );
    assert_eq!(wide.code, "invalid_bundle");
    assert_eq!(details(&wide)["index"], 0);
    assert!(issue_codes(&wide).contains(&"FOV_OUT_OF_RANGE".to_string()));
    assert_eq!(observable(&state), attached);
}

#[test]
fn bundles_come_from_an_object_a_text_or_a_file_and_only_one_at_a_time() {
    let dir = temp("sources");
    let mut state = new_project();
    let text = fs::read_to_string(bundle_path(MINIMAL)).unwrap();
    ok(
        &mut state,
        json!({"op":"scene.attach","id":"from-text","text":text}),
    );
    ok(
        &mut state,
        json!({"op":"scene.attach","id":"from-file","path":bundle_path(MINIMAL)}),
    );
    ok(
        &mut state,
        json!({"op":"scene.attach","id":"from-object","bundle":bundle(MINIMAL)}),
    );
    assert_eq!(
        document_of(&state, "from-text"),
        document_of(&state, "from-object")
    );
    assert_eq!(
        document_of(&state, "from-file"),
        document_of(&state, "from-object")
    );

    let before = observable(&state);
    for ambiguous in [
        json!({"op":"scene.attach","id":"x"}),
        json!({"op":"scene.attach","id":"x","bundle":bundle(MINIMAL),"text":text}),
    ] {
        assert_eq!(err(&mut state, ambiguous).code, "invalid_request");
    }
    assert_eq!(
        err(
            &mut state,
            json!({"op":"scene.attach","id":"x","text":"{ not json"})
        )
        .code,
        "invalid_json"
    );
    let missing = err(
        &mut state,
        json!({"op":"scene.attach","id":"x","path":dir.join("nope.json")}),
    );
    assert_eq!(missing.code, "bundle_unreadable");
    assert!(missing.target.is_some());

    // The size is checked before the file is read: a sparse 33 MiB file costs nothing to refuse.
    let big = dir.join("huge.bundle.json");
    fs::File::create(&big)
        .unwrap()
        .set_len(33 * 1024 * 1024)
        .unwrap();
    assert_eq!(
        err(&mut state, json!({"op":"scene.attach","id":"x","path":big})).code,
        "bundle_too_large"
    );
    assert_eq!(observable(&state), before);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn scene_ids_are_caller_chosen_stable_and_bounded() {
    let mut state = new_project();
    let first = ok(
        &mut state,
        json!({"op":"scene.attach","bundle":bundle(MINIMAL)}),
    );
    assert_eq!(first["scene"], "scene-1", "generated IDs are reproducible");
    attach(&mut state, "street", MINIMAL);
    let third = ok(
        &mut state,
        json!({"op":"scene.attach","bundle":bundle(MINIMAL)}),
    );
    assert_eq!(third["scene"], "scene-2");
    ok(&mut state, json!({"op":"scene.detach","scene":"scene-1"}));
    let reused = ok(
        &mut state,
        json!({"op":"scene.attach","bundle":bundle(MINIMAL)}),
    );
    assert_eq!(reused["scene"], "scene-1", "the first free ID is taken");

    // IDs follow the entity-ID alphabet and are plain strings: nothing is interned for them.
    let too_long = "a".repeat(65);
    for bad in ["", "has space", "colon:no", "a/b", too_long.as_str()] {
        let error = err(
            &mut state,
            json!({"op":"scene.attach","id":bad,"bundle":bundle(MINIMAL)}),
        );
        assert_eq!(error.code, "invalid_id", "`{bad}`");
    }
    assert_eq!(
        err(
            &mut state,
            json!({"op":"scene.attach","id":"street","bundle":bundle(MINIMAL)})
        )
        .code,
        "duplicate_id"
    );
    assert_eq!(
        err(&mut state, json!({"op":"scene.detach","scene":"nope"})).code,
        "unknown_element"
    );
    ok(
        &mut state,
        json!({"op":"scene.attach","id":"a".repeat(64),"bundle":bundle(MINIMAL)}),
    );
}

#[test]
fn a_project_holds_a_bounded_number_of_scenes() {
    let mut state = new_project();
    let minimal = bundle(MINIMAL);
    for index in 0..MAX_SCENES {
        ok(
            &mut state,
            json!({"op":"scene.attach","id":format!("s{index}"),"bundle":minimal}),
        );
    }
    let before = observable(&state);
    let error = err(
        &mut state,
        json!({"op":"scene.attach","id":"one-too-many","bundle":minimal}),
    );
    assert_eq!(error.code, "scene_limit");
    assert_eq!(observable(&state), before);
}

// ---- one vocabulary, every adapter --------------------------------------------------------

fn session_requests() -> Vec<String> {
    let path = bundle_path(MINIMAL);
    [
        json!({"op":"scene.capabilities"}),
        json!({"op":"scene.catalog"}),
        json!({"op":"scene.attach","id":"street","path":path}),
        json!({"op":"scene.list"}),
        json!({"op":"scene.describe","scene":"street"}),
        json!({"op":"scene.properties","scene":"street","prefix":"camera/"}),
        json!({"op":"scene.get","scene":"street","property":"camera/cam-main/start/fov"}),
        json!({"op":"scene.camera","scene":"street","frame":10}),
        json!({"op":"scene.edit","scene":"street","dry_run":true,"ops":[
            {"set":"camera/cam-main/start/fov","value":55}]}),
        json!({"op":"scene.edit","scene":"street","ops":[
            {"set":"camera/cam-main/start/fov","value":55}]}),
        json!({"op":"scene.get","scene":"street","property":"camera/cam-main/start/fov"}),
        json!({"op":"history.undo"}),
        json!({"op":"scene.get","scene":"street","property":"camera/cam-main/start/fov"}),
        json!({"op":"history.redo"}),
        json!({"op":"scene.edit","scene":"street","ops":[
            {"set":"scene/scene-a/node/ghost/position","value":{"x":1,"y":1,"z":1}}]}),
        json!({"op":"scene.nope"}),
        json!({"op":"scene.detach","scene":"street"}),
        json!({"op":"scene.list"}),
    ]
    .iter()
    .map(Value::to_string)
    .chain(["this is not json".to_string()])
    .collect()
}

fn parse_lines(bytes: &[u8]) -> Vec<Value> {
    String::from_utf8(bytes.to_vec())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn a_json_lines_session_runs_the_whole_flow_in_process() {
    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    let input = session_requests().join("\n");
    let mut output = Vec::new();
    let failed = headless::run_session(&mut state, None, input.as_bytes(), &mut output).unwrap();
    assert!(failed, "the session reports that some requests failed");

    let responses = parse_lines(&output);
    assert_eq!(responses.len(), 19, "one response per line, in order");
    let ok_at = |index: usize| {
        assert_eq!(responses[index]["ok"], true, "{}", responses[index]);
        &responses[index]["result"]
    };
    assert_eq!(ok_at(2)["scene"], "street");
    assert_eq!(ok_at(3)["count"], 1);
    assert_eq!(ok_at(8)["dry_run"], true);
    assert_eq!(ok_at(9)["changed"], true);
    assert_eq!(ok_at(10)["value"], 55);
    assert_eq!(ok_at(11)["action"], "history.undo");
    assert_eq!(ok_at(12)["value"], 45, "undo restored the field of view");
    assert_eq!(ok_at(13)["action"], "history.redo");
    assert_eq!(ok_at(16)["action"], "scene.detach");
    assert_eq!(ok_at(17)["count"], 0);

    assert_eq!(responses[14]["error"]["code"], "unknown_node");
    assert_eq!(responses[14]["error"]["target"], "ghost");
    assert_eq!(responses[14]["error"]["details"]["index"], 0);
    assert_eq!(responses[15]["error"]["code"], "unknown_operation");
    assert_eq!(responses[18]["error"]["code"], "invalid_json");
    assert!(state.scenes.is_empty());
}

#[test]
fn the_headless_executable_answers_exactly_like_the_library() {
    let requests = session_requests();
    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    let mut in_process = Vec::new();
    headless::run_session(
        &mut state,
        None,
        requests.join("\n").as_bytes(),
        &mut in_process,
    )
    .unwrap();

    let mut child = Process::new(env!("CARGO_BIN_EXE_tessel-headless"))
        .arg("--empty")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for request in &requests {
        writeln!(stdin, "{request}").unwrap();
    }
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "failed requests exit nonzero"
    );
    assert_eq!(
        parse_lines(&output.stdout),
        parse_lines(&in_process),
        "one parser, one executor: the process says what the library says"
    );
}

#[test]
fn scene_documents_require_version_two_but_plain_projects_stay_at_version_one() {
    let mut state = new_project();
    assert_eq!(state.to_document(None)["schema_version"], 1);
    attach(&mut state, "street", MINIMAL);
    let stored = state.to_document(None);
    assert_eq!(stored["schema_version"], 2);
    assert!(EditorState::from_document(&stored, None).is_ok());
    let mut mislabeled = stored;
    mislabeled["schema_version"] = json!(1);
    assert_eq!(
        EditorState::from_document(&mislabeled, None)
            .unwrap_err()
            .code,
        "invalid_document"
    );
    ok(&mut state, json!({"op":"scene.detach","scene":"street"}));
    assert_eq!(state.to_document(None)["schema_version"], 1);
}
