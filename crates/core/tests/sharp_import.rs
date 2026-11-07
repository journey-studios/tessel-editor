//! BL143: importing the PLY of SHARP through the public library and the JSONL entry
//! (`headless::execute`, the call `tessel-headless` makes), without a window or a renderer.
//!
//! Every file is synthetic (`fixtures::write_sharp_ply`): arithmetic, no model, no weights.
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tessel_editor_core::fixtures::{
    SHARP_FIXTURE_VFOV, SharpSpec, write_sharp_ply, write_standard_set,
};
use tessel_editor_core::{EditorState, headless};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tessel-sharp-core-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn fail(state: &mut EditorState, request: Value) -> (String, String) {
    let error = headless::execute(state, &request).expect_err(&format!("{request} must fail"));
    (error.code.to_string(), error.message)
}

fn project() -> EditorState {
    let mut state = EditorState::demo();
    run(
        &mut state,
        json!({"op":"project.create","name":"SHARP","sequence":"main","rate":24,
            "width":320,"height":180,"length":48}),
    );
    run(
        &mut state,
        json!({"op":"track.add","id":"t","name":"t","kind":"video"}),
    );
    state
}

fn ply(dir: &Path, name: &str, spec: &SharpSpec) -> PathBuf {
    let path = dir.join(name);
    write_sharp_ply(&path, spec).unwrap();
    path
}

fn add(state: &mut EditorState, id: &str, path: &Path, capture: Option<&str>) -> Value {
    let mut request = json!({"op":"asset.add","kind":"splat","id":id,"name":id,"path":path});
    if let Some(capture) = capture {
        request["capture"] = json!(capture);
    }
    run(state, request)
}

fn approx(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-3, "{a} != {b}");
}

#[test]
fn the_capture_of_a_sharp_file_is_read_from_its_own_metadata() {
    let dir = scratch("read");
    let mut state = project();
    let result = add(
        &mut state,
        "shot",
        &ply(&dir, "shot.ply", &SharpSpec::default()),
        None,
    );
    let capture = &result["asset"]["capture"];
    assert_eq!(capture["source"], "sharp");
    assert_eq!(capture["axes"], "opencv");
    assert_eq!(capture["image"], json!([160, 120]));
    assert_eq!(capture["orientation"], json!([180.0, 0.0, 0.0]));
    assert_eq!(capture["up"], json!([0.0, -1.0, 0.0]));
    assert_eq!(capture["color_space"], "sRGB");
    assert_eq!(capture["version"], json!([1, 5, 0]));
    approx(capture["vfov"].as_f64().unwrap(), SHARP_FIXTURE_VFOV);
    assert_eq!(capture["camera"]["position"], json!([0.0, 0.0, 0.0]));
    assert_eq!(capture["camera"]["target"], json!([0.0, 0.0, -1.0]));
    assert!(
        result.get("warnings").is_none(),
        "a 1.x sRGB file has nothing to warn about: {result}"
    );

    // Linear colours and an unknown writer import, with the reason in `warnings`.
    let odd = SharpSpec {
        color_space: Some(1),
        version: Some([2, 0, 0]),
        ..SharpSpec::default()
    };
    let result = add(&mut state, "odd", &ply(&dir, "odd.ply", &odd), None);
    let codes: Vec<&str> = result["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|warning| warning["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["capture_linear_colors", "capture_version"]);
}

#[test]
fn a_splat_clip_starts_on_the_capture_camera_and_plans_the_axis_turn() {
    let dir = scratch("clip");
    let mut state = project();
    add(
        &mut state,
        "shot",
        &ply(&dir, "shot.ply", &SharpSpec::default()),
        None,
    );
    run(
        &mut state,
        json!({"op":"clip.add","id":"c","track":"t","asset":"shot","start":0,"duration":48}),
    );
    let plan = run(&mut state, json!({"op":"plan","frame":0}));
    let layer = &plan["layers"][0];
    assert_eq!(layer["kind"], "scene");
    assert_eq!(layer["orientation"], json!([180.0, 0.0, 0.0]));
    assert_eq!(layer["camera"]["position"], json!([0.0, 0.0, 0.0]));
    assert_eq!(layer["camera"]["orbit"], 0.0);
    approx(layer["camera"]["fov"].as_f64().unwrap(), SHARP_FIXTURE_VFOV);
    // Still: the same camera later.
    let later = run(&mut state, json!({"op":"plan","frame":30}));
    assert_eq!(later["layers"][0]["camera"], layer["camera"]);

    // An explicit camera at creation wins over the capture.
    run(
        &mut state,
        json!({"op":"track.add","id":"t2","name":"t2","kind":"video"}),
    );
    run(
        &mut state,
        json!({"op":"clip.add","id":"c2","track":"t2","asset":"shot","start":0,"duration":48,
            "camera":{"position":[0,1,5],"target":[0,0,0],"fov":40,"orbit":0}}),
    );
    let clip = state.timeline.clip("c2").unwrap();
    approx(clip.camera.unwrap().fov, 40.0);

    // The source preview of the asset is seen from the capture too.
    run(
        &mut state,
        json!({"op":"preview.set","mode":"source","asset":"shot"}),
    );
    let source = run(&mut state, json!({"op":"plan"}));
    assert_eq!(source["layers"][0]["orientation"], json!([180.0, 0.0, 0.0]));
    assert_eq!(
        source["layers"][0]["camera"]["position"],
        json!([0.0, 0.0, 0.0])
    );
}

#[test]
fn a_plain_splat_or_an_ignored_capture_gets_no_camera_and_no_axis_turn() {
    let dir = scratch("plain");
    let set = write_standard_set(&dir.join("set")).unwrap();
    let mut state = project();
    // A Gaussian PLY with no capture elements at all.
    let plain = add(&mut state, "plain", &set.ply, None);
    assert_eq!(plain["asset"]["capture"], Value::Null);
    // A SHARP file the caller chose not to interpret.
    let shot = ply(&dir, "shot.ply", &SharpSpec::default());
    let ignored = add(&mut state, "ignored", &shot, Some("ignore"));
    assert_eq!(ignored["asset"]["capture"], Value::Null);
    for id in ["plain", "ignored"] {
        let track = format!("t-{id}");
        run(
            &mut state,
            json!({"op":"track.add","id":track,"name":track,"kind":"video"}),
        );
        run(
            &mut state,
            json!({"op":"clip.add","id":format!("c-{id}"),"track":track,"asset":id,
                "start":0,"duration":48}),
        );
    }
    let plan = run(&mut state, json!({"op":"plan","frame":0}));
    for layer in plan["layers"].as_array().unwrap() {
        assert!(layer.get("orientation").is_none(), "{layer}");
        assert_eq!(layer["camera"]["position"], json!([0.0, 1.0, 5.0]));
    }
    // A plain file's `capture: ignore` is a no-op, and a bad value is rejected.
    add(&mut state, "plain2", &set.ply, Some("ignore"));
    let (code, _) = fail(
        &mut state,
        json!({"op":"asset.add","kind":"splat","id":"x","name":"x","path":shot,"capture":"maybe"}),
    );
    assert_eq!(code, "invalid_request");
    let (code, _) = fail(
        &mut state,
        json!({"op":"clip.camera_capture","clip":"c-plain"}),
    );
    assert_eq!(code, "no_capture");
}

#[test]
fn missing_or_contradictory_metadata_is_refused_and_leaves_the_project_intact() {
    let dir = scratch("refuse");
    let mut state = project();
    add(
        &mut state,
        "kept",
        &ply(&dir, "kept.ply", &SharpSpec::default()),
        None,
    );
    let cases: [(&str, SharpSpec, &str); 4] = [
        (
            "no-size",
            SharpSpec {
                image_size: None,
                ..SharpSpec::default()
            },
            "incomplete_capture",
        ),
        (
            "no-intrinsic",
            SharpSpec {
                intrinsic: None,
                ..SharpSpec::default()
            },
            "incomplete_capture",
        ),
        (
            "non-square",
            SharpSpec {
                intrinsic: Some([100.0, 0.0, 80.0, 0.0, 130.0, 60.0, 0.0, 0.0, 1.0]),
                ..SharpSpec::default()
            },
            "ambiguous_capture",
        ),
        (
            "off-centre",
            SharpSpec {
                intrinsic: Some([128.0, 0.0, 120.0, 0.0, 128.0, 60.0, 0.0, 0.0, 1.0]),
                ..SharpSpec::default()
            },
            "ambiguous_capture",
        ),
    ];
    for (name, spec, expected) in cases {
        let path = ply(&dir, &format!("{name}.ply"), &spec);
        let before = state.to_document(None);
        let revision = state.project.revision;
        let undo_depth = state.history.to_json()["undo_depth"].clone();
        let (code, message) = fail(
            &mut state,
            json!({"op":"asset.add","kind":"splat","id":name,"name":name,"path":path}),
        );
        assert_eq!(code, expected, "{name}: {message}");
        assert!(
            message.contains("ignore") || message.contains("capture"),
            "{name}: the refusal says how to go on: {message}"
        );
        assert_eq!(state.to_document(None), before, "{name}: project intact");
        assert_eq!(state.project.revision, revision, "{name}: no revision");
        assert_eq!(
            state.history.to_json()["undo_depth"],
            undo_depth,
            "{name}: no undo step"
        );
        // The same file imports as a plain splat when the caller says so.
        let path_again = path.clone();
        let accepted = add(
            &mut state,
            &format!("{name}-plain"),
            &path_again,
            Some("ignore"),
        );
        assert_eq!(accepted["asset"]["capture"], Value::Null, "{name}");
        assert_eq!(accepted["asset"]["splat_count"], 5);
    }
    // Nothing was registered by the refused attempts.
    assert!(state.library.item("no-size").is_none());
    assert!(state.library.item("non-square").is_none());
    // Truncated bytes are an invalid asset, not a capture.
    let truncated = dir.join("cut.ply");
    let bytes = fs::read(ply(&dir, "whole.ply", &SharpSpec::default())).unwrap();
    fs::write(&truncated, &bytes[..bytes.len() - 20]).unwrap();
    let (code, _) = fail(
        &mut state,
        json!({"op":"asset.add","kind":"splat","id":"cut","name":"cut","path":truncated}),
    );
    assert_eq!(code, "invalid_asset");

    assert!(state.library.item("cut").is_none());
    assert!(
        state
            .library
            .item("kept")
            .unwrap()
            .asset
            .as_ref()
            .unwrap()
            .meta
            .capture
            .is_some()
    );
}

#[test]
fn the_capture_camera_can_be_restored_and_every_step_undoes() {
    let dir = scratch("restore");
    let mut state = project();
    add(
        &mut state,
        "shot",
        &ply(&dir, "shot.ply", &SharpSpec::default()),
        None,
    );
    run(
        &mut state,
        json!({"op":"clip.add","id":"c","track":"t","asset":"shot","start":0,"duration":48}),
    );
    let captured = state.timeline.clip("c").unwrap().camera;
    // A no-op restore records nothing.
    let depth = state.history.to_json()["undo_depth"].clone();
    let same = run(&mut state, json!({"op":"clip.camera_capture","clip":"c"}));
    assert_eq!(same["changed"], false);
    assert_eq!(state.history.to_json()["undo_depth"], depth);

    run(
        &mut state,
        json!({"op":"clip.camera","clip":"c","position":[2,1,6],"target":[0,0,0],"fov":30,"orbit":0}),
    );
    assert_ne!(state.timeline.clip("c").unwrap().camera, captured);
    let restored = run(&mut state, json!({"op":"clip.camera_capture","clip":"c"}));
    assert_eq!(restored["changed"], true);
    assert_eq!(restored["source"], "capture");
    assert_eq!(state.timeline.clip("c").unwrap().camera, captured);
    run(&mut state, json!({"op":"history.undo"}));
    approx(state.timeline.clip("c").unwrap().camera.unwrap().fov, 30.0);

    // With camera keys the base camera is not what plays: the restore is refused until they go.
    run(&mut state, json!({"op":"clip.camera_capture","clip":"c"}));
    run(
        &mut state,
        json!({"op":"clip.camera_key","clip":"c","time":0,"position":[0,0,0],
            "target":[0,0,-1],"fov":50}),
    );
    run(
        &mut state,
        json!({"op":"clip.camera","clip":"c","position":[0,0,1],"target":[0,0,-1],"fov":50,"orbit":0}),
    );
    let (code, _) = fail(&mut state, json!({"op":"clip.camera_capture","clip":"c"}));
    assert_eq!(code, "camera_orbit_conflict");
    let (code, _) = fail(
        &mut state,
        json!({"op":"clip.camera_capture","clip":"missing"}),
    );
    assert_eq!(code, "unknown_element");
}

#[test]
fn save_and_reopen_keep_the_capture_and_old_projects_keep_schema_one() {
    let dir = scratch("save");
    let mut state = project();
    let legacy = dir.join("legacy.tessel.json");
    run(&mut state, json!({"op":"project.save","path":legacy}));
    assert_eq!(
        read(&legacy)["schema_version"],
        1,
        "no capture, no schema bump"
    );

    add(
        &mut state,
        "shot",
        &ply(&dir, "shot.ply", &SharpSpec::default()),
        None,
    );
    run(
        &mut state,
        json!({"op":"clip.add","id":"c","track":"t","asset":"shot","start":0,"duration":48}),
    );
    let doc = dir.join("shot.tessel.json");
    run(&mut state, json!({"op":"project.save","path":doc}));
    let saved = read(&doc);
    assert_eq!(saved["schema_version"], 2);
    let stored = &find_asset(&saved, "shot")["asset"]["meta"]["capture"];
    assert_eq!(stored["image"], json!([160, 120]));
    assert!(
        stored.get("vfov").is_none(),
        "the field of view is derived, never stored"
    );

    let mut reopened = EditorState::demo();
    run(&mut reopened, json!({"op":"project.open","path":doc}));
    assert_eq!(
        reopened
            .library
            .item("shot")
            .unwrap()
            .asset
            .as_ref()
            .unwrap()
            .meta
            .capture,
        state
            .library
            .item("shot")
            .unwrap()
            .asset
            .as_ref()
            .unwrap()
            .meta
            .capture
    );
    assert_eq!(
        run(&mut state, json!({"op":"plan","frame":0})),
        run(&mut reopened, json!({"op":"plan","frame":0})),
        "the plan is the proof of an exact round trip"
    );
    assert_eq!(
        state.to_document(Some(&dir)),
        reopened.to_document(Some(&dir))
    );

    // A hand-edited focal length changes the derived field of view (never disagrees with itself).
    let mut edited = saved.clone();
    set_asset_capture(&mut edited, "shot", |capture| {
        capture["fx"] = json!(64.0);
        capture["fy"] = json!(64.0);
    });
    let edited_path = dir.join("edited.tessel.json");
    fs::write(&edited_path, serde_json::to_vec(&edited).unwrap()).unwrap();
    let mut other = EditorState::demo();
    run(&mut other, json!({"op":"project.open","path":edited_path}));
    run(
        &mut other,
        json!({"op":"preview.set","mode":"source","asset":"shot"}),
    );
    let wide = run(&mut other, json!({"op":"plan"}));
    let expected = 2.0 * (120.0_f64 / (2.0 * 64.0)).atan().to_degrees();
    approx(
        wide["layers"][0]["camera"]["fov"].as_f64().unwrap_or(0.0),
        expected,
    );
    // (the clip stored its own camera at creation: the capture itself is what changed)
    approx(
        other
            .library
            .item("shot")
            .unwrap()
            .asset
            .as_ref()
            .unwrap()
            .meta
            .capture
            .unwrap()
            .vfov,
        expected,
    );

    // A stored capture that cannot be a camera refuses the whole document and changes nothing.
    let mut broken = saved;
    set_asset_capture(&mut broken, "shot", |capture| capture["fx"] = json!(-3.0));
    let broken_path = dir.join("broken.tessel.json");
    fs::write(&broken_path, serde_json::to_vec(&broken).unwrap()).unwrap();
    let mut target = EditorState::demo();
    let before = target.to_document(None);
    let (code, _) = fail(&mut target, json!({"op":"project.open","path":broken_path}));
    assert_eq!(code, "invalid_document");
    assert_eq!(target.to_document(None), before);
}

fn read(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn find_asset<'a>(document: &'a Value, id: &str) -> &'a Value {
    document["library"]["assets"]
        .as_array()
        .and_then(|assets| assets.iter().find(|asset| asset["id"] == id))
        .unwrap_or_else(|| panic!("{id} is not in {document}"))
}

fn set_asset_capture(document: &mut Value, id: &str, edit: impl FnOnce(&mut Value)) {
    let assets = document["library"]["assets"].as_array_mut().unwrap();
    let asset = assets.iter_mut().find(|asset| asset["id"] == id).unwrap();
    edit(&mut asset["asset"]["meta"]["capture"]);
}
