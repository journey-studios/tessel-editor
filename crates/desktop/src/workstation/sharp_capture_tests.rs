//! BL143: the SHARP capture as the live window serves it: the library says what the file is, the
//! camera section offers "use capture camera" for a scene clip of a SHARP splat only, and the button
//! (or `invoke`) runs the same handler the JSONL op does.
use serde_json::Value;
use tessel_editor_core::commands::Parsed;
use tessel_editor_core::fixtures::{SharpSpec, write_sharp_ply};

use super::input_tests::fixture;
use super::*;
use crate::workstation::OpReply;
use core::prelude::v1::test;

fn op(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    request: Value,
) -> Result<Value, CommandError> {
    let reply = view.update(cx, |view, cx| view.run_op(&request, cx));
    cx.run_until_parked();
    match reply? {
        OpReply::Done(value) => Ok(value),
        OpReply::Pending(_) => panic!("{request} should not wait for a frame"),
    }
}

fn node(view: &Entity<Workstation>, cx: &mut VisualTestContext, id: &str) -> Option<Value> {
    view.read_with(cx, |view, cx| view.tree_node(id, cx))
}

fn invoke(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    target: &str,
    action: &str,
) -> Result<Value, CommandError> {
    let parsed = view.read_with(cx, |view, cx| {
        let layout = view.layout_snapshot(cx);
        tessel_editor_core::commands::parse_invoke(&view.state, &layout, target, action)
    })?;
    let Parsed::Run(command) = parsed else {
        panic!("a camera control is a state command");
    };
    let outcome = view.update(cx, |view, cx| view.execute(command, Source::Cli, cx));
    cx.run_until_parked();
    outcome
}

/// A project with a SHARP splat clip (`c-shot`) and a plain-import clip (`c-plain`) of the same file.
fn project(view: &Entity<Workstation>, cx: &mut VisualTestContext, tag: &str) {
    let dir = std::env::temp_dir().join(format!("tessel-sharp-ui-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("shot.ply");
    write_sharp_ply(&path, &SharpSpec::default()).unwrap();
    for request in [
        json!({"op":"project.create","name":"SHARP","sequence":"main","rate":24,
            "width":320,"height":180,"length":48}),
        json!({"op":"asset.add","kind":"splat","id":"shot","name":"shot","path":path}),
        json!({"op":"asset.add","kind":"splat","id":"plain","name":"plain","path":path,
            "capture":"ignore"}),
        json!({"op":"track.add","id":"t1","name":"t1","kind":"video"}),
        json!({"op":"track.add","id":"t2","name":"t2","kind":"video"}),
        json!({"op":"clip.add","id":"c-shot","track":"t1","asset":"shot","start":0,"duration":48}),
        json!({"op":"clip.add","id":"c-plain","track":"t2","asset":"plain","start":0,"duration":48}),
    ] {
        op(view, cx, request).unwrap();
    }
}

#[gpui_kit::test]
fn the_library_names_the_capture_and_the_camera_control_follows_the_selected_clip(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    project(&view, cx, "tree");
    let shot = node(&view, cx, "media:shot").unwrap();
    assert!(
        shot["state"]["meta"]
            .as_str()
            .unwrap()
            .contains("SHARP 160×120 · FOV 50.0°"),
        "{shot}"
    );
    assert_eq!(shot["state"]["capture"]["orientation"][0], 180.0);
    assert!(node(&view, cx, "media:plain").unwrap()["state"]["capture"].is_null());

    invoke(&view, cx, "clip:c-shot", "select").unwrap();
    let button = node(&view, cx, "camera.capture").unwrap();
    assert_eq!(button["enabled"], true, "{button}");
    assert_eq!(button["parent"], "inspector.camera");
    assert_eq!(button["state"]["matches"], true);
    assert_eq!(button["state"]["capture"]["image"][0], 160);

    invoke(&view, cx, "clip:c-plain", "select").unwrap();
    let button = node(&view, cx, "camera.capture").unwrap();
    assert_eq!(button["enabled"], false);
    assert!(
        button["disabled_reason"]
            .as_str()
            .unwrap()
            .contains("capture")
    );
    assert_eq!(
        invoke(&view, cx, "camera.capture", "invoke")
            .unwrap_err()
            .code,
        "no_capture",
        "a disabled control is also refused by the handler"
    );
}

#[gpui_kit::test]
fn the_button_restores_the_capture_camera_with_one_undo_step(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    project(&view, cx, "restore");
    invoke(&view, cx, "clip:c-shot", "select").unwrap();
    op(
        &view,
        cx,
        json!({"op":"clip.camera","clip":"c-shot","position":[3,1,6],"target":[0,0,0],
            "fov":30,"orbit":0}),
    )
    .unwrap();
    assert_eq!(
        node(&view, cx, "camera.capture").unwrap()["state"]["matches"],
        false
    );
    let revision = view.read_with(cx, |view, _| view.state.project.revision);
    let done = invoke(&view, cx, "camera.capture", "invoke").unwrap();
    assert_eq!(done["source"], "capture");
    assert!(view.read_with(cx, |view, _| view.state.project.revision) > revision);
    let button = node(&view, cx, "camera.capture").unwrap();
    assert_eq!(button["state"]["matches"], true);
    let plan = op(&view, cx, json!({"op":"plan","frame":0})).unwrap();
    assert_eq!(
        plan["layers"][0]["camera"]["position"],
        json!([0.0, 0.0, 0.0])
    );
    assert_eq!(plan["layers"][0]["orientation"], json!([180.0, 0.0, 0.0]));

    // The window shares the history with the CLI: one undo puts the edited camera back.
    op(&view, cx, json!({"op":"history.undo"})).unwrap();
    assert_eq!(
        node(&view, cx, "camera.capture").unwrap()["state"]["matches"],
        false
    );

    // With camera keys the control says why it cannot act.
    op(
        &view,
        cx,
        json!({"op":"clip.camera_key","clip":"c-shot","time":0,"position":[0,0,0],
            "target":[0,0,-1],"fov":50}),
    )
    .unwrap();
    let button = node(&view, cx, "camera.capture").unwrap();
    assert_eq!(button["enabled"], false);
    assert!(
        button["disabled_reason"]
            .as_str()
            .unwrap()
            .contains("camera keys")
    );
    assert_eq!(
        invoke(&view, cx, "camera.capture", "invoke")
            .unwrap_err()
            .code,
        "camera_orbit_conflict"
    );
}
