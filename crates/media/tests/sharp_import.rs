//! BL143: a SHARP PLY through the media host (a job, off the UI thread): import with its capture,
//! the refusal of a file whose metadata is not usable, save/reopen, package/restore and relink.
//! Synthetic files only; the renderer is the mock (real pixels are in `crates/render/tests`).

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};
use tessel_editor_core::fixtures::{SharpSpec, write_sharp_ply};
use tessel_editor_core::headless;
use tessel_editor_core::plan::{PlanRequest, PlanTime, PreviewIntent};
use tessel_editor_core::{CommandError, EditorState};
use tessel_editor_media::host::{MediaHost, RendererFactory};

use common::mock::{Mock, Settings};
use common::temp;

fn host() -> MediaHost {
    let factory: Arc<RendererFactory> = Arc::new(|| Ok(Box::new(Mock::new(Settings::default()))));
    MediaHost::new().with_renderer(factory)
}

fn run(
    host: &mut MediaHost,
    state: &mut EditorState,
    request: Value,
) -> Result<Value, CommandError> {
    headless::execute_ext(state, None, Some(host), &request)
}

fn ok(host: &mut MediaHost, state: &mut EditorState, request: Value) -> Value {
    run(host, state, request.clone()).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn project(host: &mut MediaHost, dir: &Path, name: &str) -> EditorState {
    let mut state = EditorState::demo();
    let workspace = dir.join(name);
    ok(
        host,
        &mut state,
        json!({"op":"project.create","name":name,"sequence":"main","rate":24,
            "width":320,"height":180,"length":48,"dir":workspace}),
    );
    ok(
        host,
        &mut state,
        json!({"op":"project.save","path":workspace.join(format!("{name}.tessel.json"))}),
    );
    state
}

fn ply(dir: &Path, name: &str, spec: &SharpSpec) -> PathBuf {
    let path = dir.join(name);
    write_sharp_ply(&path, spec).unwrap();
    path
}

fn plan(state: &EditorState, frame: i64) -> Value {
    state
        .plan(&PlanRequest {
            intent: PreviewIntent::Sequence,
            at: PlanTime::Frame(frame),
        })
        .unwrap()
        .to_json()
}

fn capture_of(state: &EditorState, id: &str) -> Option<tessel_editor_core::sharp::Capture> {
    state
        .library
        .item(id)
        .and_then(|item| item.asset.as_ref())
        .and_then(|asset| asset.meta.capture)
}

#[test]
fn the_import_job_keeps_the_capture_and_refuses_what_it_cannot_read() {
    let dir = temp("sharp-import");
    let mut host = host();
    let mut state = project(&mut host, &dir, "alpha");
    let good = ply(&dir, "good.ply", &SharpSpec::default());
    let report = ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[good]}),
    );
    assert_eq!(report["applied"]["imported"], 1, "{report}");
    let id = report["applied"]["items"][0]["id"].as_str().unwrap();
    let capture = capture_of(&state, id).expect("the capture survived the job");
    assert!((capture.vfov - 50.0).abs() < 1e-3);
    assert_eq!(capture.image, [160, 120]);
    // The managed copy is what the project uses.
    let stored = state.library.item(id).unwrap().asset.as_ref().unwrap();
    assert_eq!(stored.identity.storage.label(), "managed");

    // A file whose capture cannot be a camera fails alone, with its reason, and the batch goes on.
    let broken = ply(
        &dir,
        "broken.ply",
        &SharpSpec {
            image_size: None,
            ..SharpSpec::default()
        },
    );
    let other = ply(
        &dir,
        "other.ply",
        &SharpSpec {
            version: Some([1, 6, 0]),
            points: SharpSpec::default().points[..3].to_vec(),
            ..SharpSpec::default()
        },
    );
    let before = state.library.items.len();
    let report = ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[broken, other]}),
    );
    assert_eq!(report["applied"]["imported"], 1, "{report}");
    assert_eq!(report["applied"]["failed"], 1, "{report}");
    assert!(
        report.to_string().contains("incomplete_capture"),
        "the reason is reported: {report}"
    );
    assert_eq!(state.library.items.len(), before + 1);

    // The same refused file imports as a plain splat when the caller says so.
    let report = ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[broken],"capture":"ignore"}),
    );
    assert_eq!(report["applied"]["imported"], 1, "{report}");
    let plain = report["applied"]["items"][0]["id"].as_str().unwrap();
    assert!(capture_of(&state, plain).is_none());
    // A bad choice is a request error before any job starts.
    let error = run(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[good],"capture":"maybe"}),
    )
    .unwrap_err();
    assert_eq!(error.code, "invalid_request");
}

#[test]
fn save_reopen_package_and_restore_keep_the_capture_camera() {
    let dir = temp("sharp-package");
    let mut host = host();
    let mut state = project(&mut host, &dir, "beta");
    let source = ply(&dir, "shot.ply", &SharpSpec::default());
    let report = ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[source]}),
    );
    let id = report["applied"]["items"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    ok(
        &mut host,
        &mut state,
        json!({"op":"track.add","id":"t","name":"t","kind":"video"}),
    );
    ok(
        &mut host,
        &mut state,
        json!({"op":"clip.add","id":"c","track":"t","asset":id,"start":0,"duration":48}),
    );
    ok(&mut host, &mut state, json!({"op":"project.save"}));
    let saved_plan = plan(&state, 12);
    assert_eq!(
        saved_plan["layers"][0]["orientation"],
        json!([180.0, 0.0, 0.0])
    );

    // Save, reopen.
    let document = dir.join("beta/beta.tessel.json");
    let mut reopened = EditorState::demo();
    ok(
        &mut host,
        &mut reopened,
        json!({"op":"project.open","path":document}),
    );
    assert_eq!(plan(&reopened, 12), saved_plan);
    assert_eq!(capture_of(&reopened, &id), capture_of(&state, &id));

    // Package, then the original is gone and the project is restored elsewhere.
    let archive = dir.join("share/beta.tesselpack");
    let packaged = ok(
        &mut host,
        &mut state,
        json!({"op":"project.package","path":archive,"format":"archive"}),
    );
    assert_eq!(packaged["complete"], true, "{packaged}");
    let verified = ok(
        &mut host,
        &mut state,
        json!({"op":"project.verify","path":archive}),
    );
    assert_eq!(verified["complete"], true, "{verified}");
    fs::remove_file(dir.join("shot.ply")).unwrap();
    let mut elsewhere = EditorState::demo();
    ok(
        &mut host,
        &mut elsewhere,
        json!({"op":"project.restore","path":archive,"folder":dir.join("restored/beta")}),
    );
    assert_eq!(capture_of(&elsewhere, &id), capture_of(&state, &id));
    assert_eq!(
        plan(&elsewhere, 12)["layers"],
        saved_plan["layers"],
        "the restored project looks through the same camera"
    );
    let status = ok(&mut host, &mut elsewhere, json!({"op":"media.status"}));
    assert_eq!(status["missing"], 0, "{status}");
}

#[test]
fn relinking_a_capture_to_a_file_with_other_intrinsics_is_refused() {
    let dir = temp("sharp-relink");
    let mut host = host();
    let mut state = project(&mut host, &dir, "gamma");
    let report = ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[ply(&dir, "shot.ply", &SharpSpec::default())],
            "mode":"reference"}),
    );
    let id = report["applied"]["items"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    // Same bytes elsewhere: accepted.
    let copy = dir.join("moved.ply");
    fs::copy(dir.join("shot.ply"), &copy).unwrap();
    ok(
        &mut host,
        &mut state,
        json!({"op":"media.relink","asset":id,"path":copy}),
    );
    // Another capture (a wider lens): not the file the project was built with, even if allowed.
    let wide = SharpSpec {
        intrinsic: Some([64.0, 0.0, 80.0, 0.0, 64.0, 60.0, 0.0, 0.0, 1.0]),
        ..SharpSpec::default()
    };
    let wide = ply(&dir, "wide.ply", &wide);
    let error = run(
        &mut host,
        &mut state,
        json!({"op":"media.relink","asset":id,"path":wide,"accept_changed":true}),
    )
    .unwrap_err();
    assert_eq!(
        error
            .details
            .as_ref()
            .map(|details| details["job_code"].clone()),
        Some(json!("relink_mismatch")),
        "{error:?}"
    );
    assert!((capture_of(&state, &id).unwrap().vfov - 50.0).abs() < 1e-3);
}

fn warning_codes(item: &Value) -> Vec<String> {
    item["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|warning| {
            warning
                .as_str()
                .unwrap()
                .split(':')
                .next()
                .unwrap()
                .to_string()
        })
        .collect()
}

#[test]
fn the_import_report_carries_the_capture_warnings_of_asset_add() {
    let dir = temp("sharp-warnings");
    let mut host = host();
    let mut state = project(&mut host, &dir, "delta");
    let cases = [
        ("clean.ply", SharpSpec::default(), Vec::<&str>::new()),
        (
            "linear.ply",
            SharpSpec {
                color_space: Some(1),
                points: SharpSpec::default().points[..4].to_vec(),
                ..SharpSpec::default()
            },
            vec!["capture_linear_colors"],
        ),
        (
            "no-version.ply",
            SharpSpec {
                version: None,
                points: SharpSpec::default().points[..3].to_vec(),
                ..SharpSpec::default()
            },
            vec!["capture_version"],
        ),
        (
            "no-color-space.ply",
            SharpSpec {
                color_space: None,
                points: SharpSpec::default().points[..2].to_vec(),
                ..SharpSpec::default()
            },
            vec!["capture_color_space"],
        ),
        (
            "writer-2.ply",
            SharpSpec {
                color_space: Some(1),
                version: Some([2, 0, 0]),
                points: SharpSpec::default().points[..1].to_vec(),
                ..SharpSpec::default()
            },
            vec!["capture_linear_colors", "capture_version"],
        ),
    ];
    let paths: Vec<PathBuf> = cases
        .iter()
        .map(|(name, spec, _)| ply(&dir, name, spec))
        .collect();
    let report = ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":paths}),
    );
    assert_eq!(report["applied"]["imported"], 5, "{report}");
    let items = report["applied"]["items"].as_array().unwrap();
    for ((name, _, expected), item) in cases.iter().zip(items) {
        assert_eq!(&warning_codes(item), expected, "{name}: {item}");
    }
    // `capture: ignore` drops the metadata, so there is nothing to warn about.
    let ignored = ply(
        &dir,
        "ignored.ply",
        &SharpSpec {
            color_space: Some(1),
            version: None,
            points: SharpSpec::default().points[..2].to_vec(),
            ..SharpSpec::default()
        },
    );
    let report = ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[ignored],"capture":"ignore"}),
    );
    assert!(warning_codes(&report["applied"]["items"][0]).is_empty());
}

fn history_depth(state: &EditorState) -> Value {
    state.history.to_json()["undo_depth"].clone()
}

#[test]
fn a_duplicate_with_another_capture_choice_is_refused_and_changes_nothing() {
    let dir = temp("sharp-conflict");
    let mut host = host();
    let mut state = project(&mut host, &dir, "epsilon");
    let source = ply(&dir, "shot.ply", &SharpSpec::default());
    let report = ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[source],"capture":"ignore"}),
    );
    let id = report["applied"]["items"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(capture_of(&state, &id).is_none());
    ok(
        &mut host,
        &mut state,
        json!({"op":"track.add","id":"t","name":"t","kind":"video"}),
    );
    ok(
        &mut host,
        &mut state,
        json!({"op":"clip.add","id":"c","track":"t","asset":id,"start":0,"duration":48}),
    );
    let document = state.to_document(None);
    let depth = history_depth(&state);

    // The same bytes, now asking for the capture camera: refused with its reason, never `duplicate`.
    let other = ply(
        &dir,
        "other.ply",
        &SharpSpec {
            points: SharpSpec::default().points[..3].to_vec(),
            ..SharpSpec::default()
        },
    );
    let report = ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[source,other]}),
    );
    let applied = &report["applied"];
    assert_eq!(
        applied["imported"], 1,
        "the rest of the batch goes on: {report}"
    );
    assert_eq!(applied["failed"], 1, "{report}");
    assert_eq!(applied["duplicates"], 0, "{report}");
    let refused = &applied["items"][0];
    assert_eq!(refused["status"], "failed", "{report}");
    assert_eq!(refused["error"]["code"], "capture_conflict", "{report}");
    assert_eq!(refused["error"]["details"]["existing_asset"], id.as_str());
    assert_eq!(refused["error"]["details"]["existing_has_capture"], false);
    assert_eq!(refused["error"]["details"]["requested_has_capture"], true);
    assert!(
        capture_of(&state, &id).is_none(),
        "the existing asset is as it was"
    );
    assert_eq!(
        state.timeline.clip("c").unwrap().media.unwrap().as_str(),
        id
    );

    // Alone, the refusal is a whole no-op: the document and the undo history are untouched.
    let before = state.to_document(None);
    let depth_before = history_depth(&state);
    let report = ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[source]}),
    );
    assert_eq!(report["applied"]["imported"], 0, "{report}");
    assert_eq!(report["applied"]["failed"], 1, "{report}");
    assert_eq!(state.to_document(None), before);
    assert_eq!(history_depth(&state), depth_before);
    assert_ne!(
        history_depth(&state),
        depth,
        "(the batch above did add `other`)"
    );
    assert_ne!(state.to_document(None), document);

    // The same choice is a plain duplicate, as before.
    let report = ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[source],"capture":"ignore"}),
    );
    assert_eq!(report["applied"]["duplicates"], 1, "{report}");
    assert_eq!(report["applied"]["failed"], 0, "{report}");
    assert_eq!(report["applied"]["items"][0]["status"], "duplicate");
    assert_eq!(state.to_document(None), before);

    // And the other way round: with the camera, then asking to ignore it.
    let kept = ply(
        &dir,
        "kept.ply",
        &SharpSpec {
            points: SharpSpec::default().points[..2].to_vec(),
            ..SharpSpec::default()
        },
    );
    let report = ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[kept]}),
    );
    let kept_id = report["applied"]["items"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(capture_of(&state, &kept_id).is_some());
    let report = ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[kept],"capture":"ignore"}),
    );
    assert_eq!(
        report["applied"]["items"][0]["error"]["code"], "capture_conflict",
        "{report}"
    );
    assert!(
        capture_of(&state, &kept_id).is_some(),
        "its camera was kept"
    );
    let report = ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[kept]}),
    );
    assert_eq!(report["applied"]["duplicates"], 1, "{report}");
}

#[test]
fn a_hostile_or_truncated_header_fails_that_file_alone_in_the_batch() {
    let dir = temp("sharp-hostile");
    let mut host = host();
    let mut state = project(&mut host, &dir, "zeta");
    let good = ply(&dir, "good.ply", &SharpSpec::default());
    let header_with = |name: &str, find: &str, replace: &str| {
        let path = ply(&dir, name, &SharpSpec::default());
        let bytes = fs::read(&path).unwrap();
        let marker = b"end_header\n";
        let at = bytes
            .windows(marker.len())
            .position(|window| window == marker)
            .unwrap()
            + marker.len();
        let header = String::from_utf8(bytes[..at].to_vec())
            .unwrap()
            .replacen(find, replace, 1);
        let mut edited = header.into_bytes();
        edited.extend_from_slice(&bytes[at..]);
        fs::write(&path, edited).unwrap();
        path
    };
    let overflow = header_with(
        "overflow.ply",
        "element vertex 5",
        "element vertex 18446744073709551615",
    );
    let far_too_big = header_with(
        "big.ply",
        "element vertex 5",
        "element vertex 1099511627776",
    );
    let truncated = ply(&dir, "truncated.ply", &SharpSpec::default());
    let bytes = fs::read(&truncated).unwrap();
    fs::write(&truncated, &bytes[..bytes.len() - 7]).unwrap();
    let duplicate = header_with(
        "duplicate.ply",
        "element version 3\n",
        "element version 3\nproperty uchar version\nelement version 3\n",
    );

    let before = state.library.items.len();
    let report = ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[overflow, far_too_big, truncated, duplicate, good]}),
    );
    let applied = &report["applied"];
    assert_eq!(applied["imported"], 1, "{report}");
    assert_eq!(applied["failed"], 4, "{report}");
    let items = applied["items"].as_array().unwrap();
    let codes: Vec<&str> = items
        .iter()
        .map(|item| item["error"]["code"].as_str().unwrap_or("ok"))
        .collect();
    assert_eq!(
        codes,
        [
            "invalid_asset",
            "invalid_asset",
            "invalid_asset",
            "ambiguous_capture",
            "ok"
        ],
        "{report}"
    );
    assert_eq!(state.library.items.len(), before + 1);
}
