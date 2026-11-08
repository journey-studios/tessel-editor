//! BL143: importing the PLY of SHARP through the public library and the JSONL entry
//! (`headless::execute`, the call `tessel-headless` makes), without a window or a renderer.
//!
//! Every file is synthetic (`fixtures::write_sharp_ply`): arithmetic, no model, no weights.
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tessel_editor_core::fixtures::{
    SHARP_FIXTURE_IMAGE, SHARP_FIXTURE_PIXELS, SHARP_FIXTURE_VFOV, SharpSpec, write_sharp_ply,
    write_standard_set,
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
    // The refusal is `camera_keys_override`, not an orbit conflict: the capture camera does not
    // orbit, it would only be an inert undo entry under the keys (BL143.9).
    run(
        &mut state,
        json!({"op":"clip.camera","clip":"c","position":[0,0,1],"target":[0,0,-1],"fov":50,"orbit":0}),
    );
    run(
        &mut state,
        json!({"op":"clip.camera_key","clip":"c","time":0,"position":[0,0,0],
            "target":[0,0,-1],"fov":50}),
    );
    let before = state.to_document(None);
    let (code, _) = fail(&mut state, json!({"op":"clip.camera_capture","clip":"c"}));
    assert_eq!(code, "camera_keys_override");
    assert_eq!(
        state.to_document(None),
        before,
        "the refusal changes nothing"
    );
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

/// Puts `extra` header lines before `end_header`, keeping the binary body untouched.
fn with_header_lines(path: &Path, extra: &str) {
    let bytes = fs::read(path).unwrap();
    let marker = b"end_header\n";
    let at = bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .unwrap();
    let mut edited = bytes[..at].to_vec();
    edited.extend_from_slice(extra.as_bytes());
    edited.extend_from_slice(&bytes[at..]);
    fs::write(path, edited).unwrap();
}

#[test]
fn a_plain_ply_with_an_odd_header_still_imports_with_auto_and_ignore() {
    let dir = scratch("plain-odd");
    let mut state = project();
    let plain = SharpSpec {
        metadata: false,
        ..SharpSpec::default()
    };
    // What the previous reader accepted and the SHARP scanner must not start refusing:
    // a list property, an unknown type, a line with extra tokens, a count that is not a number.
    let headers = [
        "element face 1\nproperty list uchar int vertex_indices\n",
        "element extra 2\nproperty half weight\n",
        "element face 1 extra\nproperty uchar flag\n",
        "element face many\nproperty float w\n",
    ];
    for (index, lines) in headers.iter().enumerate() {
        for capture in [None, Some("auto"), Some("ignore")] {
            let path = ply(&dir, &format!("odd-{index}.ply"), &plain);
            with_header_lines(&path, lines);
            let id = format!("odd-{index}-{}", capture.unwrap_or("default"));
            let result = add(&mut state, &id, &path, capture);
            assert!(
                result["asset"]["capture"].is_null(),
                "{lines:?} with {capture:?}: {result}"
            );
            assert!(state.library.item(&id).is_some());
        }
    }
}

#[test]
fn a_hostile_header_is_a_typed_error_and_the_project_stays_untouched() {
    let dir = scratch("hostile");
    let mut state = project();
    let before = state.to_document(None);
    for count in [
        "18446744073709551615",
        "4611686018427387904",
        "1099511627776",
    ] {
        let path = ply(&dir, "hostile.ply", &SharpSpec::default());
        let bytes = fs::read(&path).unwrap();
        let header = String::from_utf8_lossy(&bytes[..200])
            .replace("element vertex 5", &format!("element vertex {count}"));
        let mut edited = header.into_bytes();
        edited.extend_from_slice(&bytes[200..]);
        fs::write(&path, edited).unwrap();
        let (code, message) = fail(
            &mut state,
            json!({"op":"asset.add","kind":"splat","id":"bad","name":"bad","path":path}),
        );
        assert_eq!(code, "invalid_asset", "{count}: {message}");
        assert_eq!(
            state.to_document(None),
            before,
            "{count}: nothing registered"
        );
    }
}

#[test]
fn a_missing_color_space_is_reported_with_its_own_warning() {
    let dir = scratch("no-color-space");
    let mut state = project();
    let spec = SharpSpec {
        color_space: None,
        ..SharpSpec::default()
    };
    let result = add(&mut state, "shot", &ply(&dir, "a.ply", &spec), None);
    assert_eq!(result["asset"]["capture"]["color_space"], "sRGB");
    assert_eq!(result["asset"]["capture"]["color_space_declared"], false);
    let codes: Vec<&str> = result["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|warning| warning["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["capture_color_space"]);
}

#[test]
fn a_capture_on_an_asset_that_is_not_a_splat_is_refused_everywhere() {
    use tessel_editor_core::assets::{Asset, Declared, MediaKind};
    use tessel_editor_core::sharp::{self, Choice};
    use tessel_editor_core::state::Command;

    let dir = scratch("non-splat");
    let set = write_standard_set(&dir.join("set")).unwrap();
    let mut state = project();
    add(
        &mut state,
        "shot",
        &ply(&dir, "shot.ply", &SharpSpec::default()),
        None,
    );
    run(
        &mut state,
        json!({"op":"asset.add","kind":"still","id":"img","name":"img","path":set.image}),
    );

    // Registration: a still that carries a capture camera never reaches the library.
    let capture = sharp::read(&ply(&dir, "again.ply", &SharpSpec::default()), Choice::Auto)
        .unwrap()
        .unwrap();
    let mut asset = Asset::probe(
        MediaKind::Still,
        &set.image.to_string_lossy(),
        &Declared::default(),
    )
    .unwrap();
    asset.meta.capture = Some(capture);
    let before = state.to_document(None);
    let error = state
        .execute(Command::AddAsset {
            id: Some("img2".into()),
            name: "img2".into(),
            kind: MediaKind::Still,
            asset: Box::new(asset.clone()),
            folder: None,
        })
        .unwrap_err();
    assert_eq!(error.code, "invalid_asset");
    assert_eq!(state.to_document(None), before);

    let error = state
        .execute(Command::ReplaceAsset {
            asset_id: "img".into(),
            asset: Box::new(asset),
        })
        .unwrap_err();
    assert_eq!(error.code, "invalid_asset");
    assert_eq!(state.to_document(None), before);

    // A document that says so is refused whole and changes nothing.
    let doc = dir.join("doc.tessel.json");
    run(&mut state, json!({"op":"project.save","path":doc}));
    let mut saved = read(&doc);
    let stored = find_asset(&saved, "shot")["asset"]["meta"]["capture"].clone();
    set_asset_capture(&mut saved, "img", |capture| *capture = stored);
    let forged = dir.join("forged.tessel.json");
    fs::write(&forged, serde_json::to_vec(&saved).unwrap()).unwrap();
    let mut target = EditorState::demo();
    let before = target.to_document(None);
    let (code, message) = fail(&mut target, json!({"op":"project.open","path":forged}));
    assert_eq!(code, "invalid_document", "{message}");
    assert!(message.contains("only splats"), "{message}");
    assert_eq!(target.to_document(None), before);
}

// ---- BL143.8: a quantitative colour and framing check with an independent oracle ----------------
//
// Scope, honestly: this checks the *file contract* (what the PLY stores, what the editor reads from
// it, where the camera of the file puts every point) in pure Rust, with no browser. It does NOT
// render: the pixels of a real render against the real source PNG are still unproven (BL143.8).
// The metric is what the old dominant-channel probe lacked: it moves when the colours are darkened.

/// SH degree-0 constant of `ml-sharp`'s `rgb2sh`: `colour = 0.5 + C0 * f_dc`.
const SH_C0: f64 = 0.282_094_791_773_878_14;

/// A reader written for the test alone (the header by `split_whitespace`, the body by fixed
/// offsets, the colour by the published formula): it shares no code with `sharp::read`.
fn oracle_vertices(path: &Path) -> Vec<([f64; 3], [f64; 3])> {
    let bytes = fs::read(path).unwrap();
    let marker = b"end_header\n";
    let at = bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .unwrap()
        + marker.len();
    let header = std::str::from_utf8(&bytes[..at]).unwrap();
    let (mut count, mut names, mut in_vertex) = (0usize, Vec::<String>::new(), false);
    for line in header.lines() {
        match line.split_whitespace().collect::<Vec<_>>().as_slice() {
            ["element", name, n] => {
                in_vertex = *name == "vertex";
                if in_vertex {
                    count = n.parse().unwrap();
                }
            }
            ["property", "float", name] if in_vertex => names.push((*name).to_string()),
            _ => {}
        }
    }
    let column = |name: &str| {
        names
            .iter()
            .position(|candidate| candidate == name)
            .unwrap()
    };
    let stride = names.len() * 4;
    (0..count)
        .map(|index| {
            let record = &bytes[at + index * stride..at + (index + 1) * stride];
            let value = |name: &str| {
                let start = column(name) * 4;
                f64::from(f32::from_le_bytes(
                    record[start..start + 4].try_into().unwrap(),
                ))
            };
            let colour =
                |channel: usize| (0.5 + SH_C0 * value(&format!("f_dc_{channel}"))).clamp(0.0, 1.0);
            (
                [value("x"), value("y"), value("z")],
                [colour(0), colour(1), colour(2)],
            )
        })
        .collect()
}

struct Fidelity {
    /// Largest per-channel difference to the intended colour, `0..=1`.
    colour: f64,
    /// Largest distance in pixels between the oracle's pinhole and the intended pixel.
    oracle_pixels: f64,
    /// The same for the editor's own camera model (`Capture::project_in_editor`).
    editor_pixels: f64,
}

fn fidelity(dir: &Path, name: &str, written: &SharpSpec) -> Fidelity {
    let intended = SharpSpec::default();
    let path = ply(dir, &format!("{name}.ply"), written);
    let capture = tessel_editor_core::sharp::read(&path, tessel_editor_core::sharp::Choice::Auto)
        .unwrap()
        .unwrap();
    let [width, height] = SHARP_FIXTURE_IMAGE;
    // The pinhole of the intended camera, from the constants of the fixture only.
    let fy = f64::from(height) / 2.0 / (SHARP_FIXTURE_VFOV.to_radians() / 2.0).tan();
    let (cx, cy) = (f64::from(width) / 2.0, f64::from(height) / 2.0);
    let distance =
        |a: [f64; 2], b: [f64; 2]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
    let mut result = Fidelity {
        colour: 0.0,
        oracle_pixels: 0.0,
        editor_pixels: 0.0,
    };
    for (index, (position, colour)) in oracle_vertices(&path).into_iter().enumerate() {
        for (got, want) in colour.iter().zip(intended.points[index].color) {
            result.colour = result.colour.max((got - f64::from(want)).abs());
        }
        let want = SHARP_FIXTURE_PIXELS[index];
        let seen = [
            fy * position[0] / position[2] + cx,
            fy * position[1] / position[2] + cy,
        ];
        result.oracle_pixels = result.oracle_pixels.max(distance(seen, want));
        let editor = capture
            .project_in_editor(position, capture.image)
            .map_or(f64::INFINITY, |pixel| distance(pixel, want));
        result.editor_pixels = result.editor_pixels.max(editor);
    }
    result
}

#[test]
fn colours_and_framing_match_an_independent_oracle_and_the_metric_detects_errors() {
    let dir = scratch("fidelity");
    let honest = fidelity(&dir, "honest", &SharpSpec::default());
    assert!(honest.colour < 1e-3, "colour error {}", honest.colour);
    assert!(honest.oracle_pixels < 0.05, "{}", honest.oracle_pixels);
    assert!(honest.editor_pixels < 0.05, "{}", honest.editor_pixels);

    // Negative controls: each class of error must move the metric well past the tolerance.
    let altered = |edit: &dyn Fn(&mut tessel_editor_core::fixtures::SharpPoint)| {
        let mut spec = SharpSpec::default();
        spec.points.iter_mut().for_each(edit);
        spec
    };
    // A dominant-channel probe cannot see a darkened image; this metric does.
    let darker = fidelity(
        &dir,
        "darker",
        &altered(&|point| point.color = point.color.map(|channel| channel * 0.8)),
    );
    assert!(darker.colour > 0.15, "darkening: {}", darker.colour);
    assert!(darker.oracle_pixels < 0.05, "darkening moves no pixel");
    let swapped = fidelity(
        &dir,
        "swapped",
        &altered(&|point| point.color = [point.color[2], point.color[1], point.color[0]]),
    );
    assert!(swapped.colour > 0.5, "red/blue swap: {}", swapped.colour);
    // A vertical mirror (the missing axis turn, seen from the image) and a horizontal one.
    let upside_down = fidelity(
        &dir,
        "upside-down",
        &altered(&|p| p.position[1] = -p.position[1]),
    );
    assert!(
        upside_down.oracle_pixels > 10.0,
        "{}",
        upside_down.oracle_pixels
    );
    assert!(
        upside_down.editor_pixels > 10.0,
        "{}",
        upside_down.editor_pixels
    );
    assert!(
        upside_down.colour < 1e-3,
        "framing errors do not move colours"
    );
    let mirrored = fidelity(
        &dir,
        "mirrored",
        &altered(&|p| p.position[0] = -p.position[0]),
    );
    assert!(mirrored.oracle_pixels > 10.0 && mirrored.editor_pixels > 10.0);
    // A cloud behind the camera (z flipped) is nowhere: infinite for the editor's model.
    let behind = fidelity(
        &dir,
        "behind",
        &altered(&|p| p.position[2] = -p.position[2]),
    );
    assert!(behind.editor_pixels.is_infinite());
}
