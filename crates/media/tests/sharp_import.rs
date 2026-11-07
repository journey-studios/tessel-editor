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
