//! BL140: the camera-key controls of the inspector as the live window serves them: the tree
//! offers them for a scene clip only, `invoke`/`set` run the shared command handler, and the
//! picture the plan describes follows the keys.
use serde_json::Value;
use tessel_editor_core::commands::Parsed;
use tessel_editor_core::fixtures::{proof_requests, write_standard_set};

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
        panic!("a camera key control is a state command");
    };
    let outcome = view.update(cx, |view, cx| view.execute(command, Source::Cli, cx));
    cx.run_until_parked();
    outcome
}

fn set(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    target: &str,
    field: &str,
    value: Raw,
) -> Result<Value, CommandError> {
    let Parsed::Run(command) = tessel_editor_core::commands::parse_set(target, field, &value)?
    else {
        panic!("a camera key field is a state command");
    };
    let outcome = view.update(cx, |view, cx| view.execute(command, Source::Cli, cx));
    cx.run_until_parked();
    outcome
}

fn scene_project(view: &Entity<Workstation>, cx: &mut VisualTestContext, tag: &str) {
    let dir = std::env::temp_dir().join(format!("tessel-camkeys-ui-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let set = write_standard_set(&dir).unwrap();
    for request in proof_requests(&set) {
        op(view, cx, request).unwrap();
    }
    op(
        view,
        cx,
        json!({"op":"clip.camera","clip":"c-scene","position":[0,1.2,5],"target":[0,0,0],"fov":50,"orbit":0}),
    )
    .unwrap();
}

fn keys(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |view, _| {
        view.state
            .timeline
            .clip("c-scene")
            .unwrap()
            .camera_keys
            .len()
    })
}

#[gpui_kit::test]
fn the_camera_section_and_its_controls_exist_for_scene_clips_only(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    scene_project(&view, cx, "tree");
    // The demonstration clip selected by default is not a scene: nothing to key.
    invoke(&view, cx, "clip:c-img", "select").unwrap();
    assert_eq!(
        node(&view, cx, "inspector.camera").unwrap()["visible"],
        false
    );
    assert!(
        invoke(&view, cx, "camera_keys.capture", "invoke").is_err(),
        "an image clip has no camera to key"
    );
    invoke(&view, cx, "clip:c-scene", "select").unwrap();
    let section = node(&view, cx, "inspector.camera").unwrap();
    assert_eq!(section["visible"], true);
    assert_eq!(section["kind"], "accordion");
    let capture = node(&view, cx, "camera_keys.capture").unwrap();
    assert_eq!(capture["enabled"], true);
    assert_eq!(capture["parent"], "inspector.camera");
    assert_eq!(
        node(&view, cx, "camera_keys.clear").unwrap()["enabled"],
        false,
        "nothing to clear yet, and the node says why"
    );
    assert!(node(&view, cx, "camera_keys.clear").unwrap()["disabled_reason"].is_string());
}

#[gpui_kit::test]
fn capture_edit_and_remove_run_through_the_shared_handler(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    scene_project(&view, cx, "flow");
    invoke(&view, cx, "clip:c-scene", "select").unwrap();
    set(&view, cx, "timeline.playhead", "frame", Raw::Number(12.0)).unwrap();
    invoke(&view, cx, "camera_keys.capture", "invoke").unwrap();
    set(&view, cx, "timeline.playhead", "frame", Raw::Number(36.0)).unwrap();
    let before = view.read_with(cx, |view, _| view.state.project.revision);
    invoke(&view, cx, "camera_keys.capture", "invoke").unwrap();
    assert_eq!(keys(&view, cx), 2);
    assert!(view.read_with(cx, |view, _| view.state.project.revision) > before);

    let key = node(&view, cx, "camera_key:c-scene:1").unwrap();
    assert_eq!(key["kind"], "camera-key");
    assert_eq!(key["state"]["time"]["seconds"], 1.5);
    assert_eq!(key["fields"].as_array().unwrap().len(), 9);

    set(
        &view,
        cx,
        "camera_key:c-scene:1",
        "position_z",
        Raw::Number(2.0),
    )
    .unwrap();
    set(
        &view,
        cx,
        "camera_key:c-scene:1",
        "easing",
        Raw::Text("easeInOutCubic".into()),
    )
    .unwrap();
    let key = node(&view, cx, "camera_key:c-scene:1").unwrap();
    assert_eq!(key["state"]["position"][2], 2.0);
    assert_eq!(key["state"]["easing"], "easeInOutCubic");

    // The picture follows: halfway between the keys the planned camera moved.
    let plan = op(&view, cx, json!({"op":"plan","frame":24})).unwrap();
    let z = plan["layers"][0]["camera"]["position"][2].as_f64().unwrap();
    assert!((z - 5.0).abs() > 0.1 && z > 2.0 && z < 5.0, "{z}");

    // A rejected edit changes nothing and says why.
    let revision = view.read_with(cx, |view, _| view.state.project.revision);
    let error = set(&view, cx, "camera_key:c-scene:1", "fov", Raw::Number(500.0)).unwrap_err();
    assert_eq!(error.code, "invalid_value");
    assert_eq!(
        view.read_with(cx, |view, _| view.state.project.revision),
        revision
    );

    invoke(&view, cx, "camera_key:c-scene:0", "remove").unwrap();
    assert_eq!(keys(&view, cx), 1);
    invoke(&view, cx, "camera_keys.clear", "invoke").unwrap();
    assert_eq!(keys(&view, cx), 0);
    // One undo brings the keys back: the window uses the same history as the CLI.
    op(&view, cx, json!({"op":"history.undo"})).unwrap();
    assert_eq!(keys(&view, cx), 1);
}

#[gpui_kit::test]
fn capture_is_disabled_with_a_reason_when_the_playhead_is_outside_the_clip(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    scene_project(&view, cx, "outside");
    invoke(&view, cx, "clip:c-scene", "select").unwrap();
    set(&view, cx, "trim:c-scene:start", "frame", Raw::Number(30.0)).unwrap();
    set(&view, cx, "timeline.playhead", "frame", Raw::Number(2.0)).unwrap();
    let capture = node(&view, cx, "camera_keys.capture").unwrap();
    assert_eq!(capture["enabled"], false);
    assert!(
        capture["disabled_reason"]
            .as_str()
            .unwrap()
            .contains("playhead")
    );
    assert_eq!(keys(&view, cx), 0);
}

#[gpui_kit::test]
fn capture_is_disabled_with_a_reason_while_the_camera_orbits_and_its_button_matches(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    scene_project(&view, cx, "orbiting");
    invoke(&view, cx, "clip:c-scene", "select").unwrap();
    // Put the orbit back (the fixture keys a still camera) and the playhead over the clip.
    op(
        &view,
        cx,
        json!({"op":"clip.camera","clip":"c-scene","position":[0,1.2,5],"target":[0,0,0],
            "fov":50,"orbit":30}),
    )
    .unwrap();
    set(&view, cx, "timeline.playhead", "frame", Raw::Number(2.0)).unwrap();
    let capture = node(&view, cx, "camera_keys.capture").unwrap();
    assert_eq!(capture["enabled"], false);
    assert!(
        capture["disabled_reason"]
            .as_str()
            .unwrap()
            .contains("orbit"),
        "{capture}"
    );
    // The disabled node is not a callback that fails: the command's own refusal is the same code.
    let error = invoke(&view, cx, "camera_keys.capture", "invoke").unwrap_err();
    assert_eq!(error.code, "camera_orbit_conflict");
    assert_eq!(keys(&view, cx), 0);
}
