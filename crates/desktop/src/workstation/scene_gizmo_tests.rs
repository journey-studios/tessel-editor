//! F055: the desktop half of the translation gizmo, driven through the entry the page uses
//! (`overlay.pointer` messages) and the window's own Escape and focus paths.
//!
//! A GPUI test window has no WebView, so the canvas itself is `runtime/test/gizmo-overlay.test.mjs`
//! (real Chrome, real mouse events) and the geometry is `crates/core/tests/scene_gizmo.rs`. What
//! is proven here is the seam: a message becomes the one core command, a failure is settled, and
//! Escape / focus loss / a hidden monitor end the drag without committing.
use serde_json::{Value, json};

use super::input_tests::fixture;
use super::*;
// The Kit's test macro expands to `#[test]`; the glob import above would resolve it to the Kit macro again.
use core::prelude::v1::test;

use tessel_editor_core::scene_interaction::{CancelReason, GizmoCommand, SelectMode};
use tessel_editor_core::scene_project::{SceneCommand, SceneSource};

const BUNDLE: &str = include_str!(
    "../../../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json"
);
const FIGURE: &str = "scene/scene-a/node/figure";

fn run(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    command: SceneCommand,
) -> Result<Value, CommandError> {
    view.update(cx, |view, cx| {
        view.execute(Command::Scene(command), Source::Cli, cx)
    })
}

/// The fixture scene attached, shown on the monitor and the figure selected.
fn mounted(cx: &mut TestAppContext) -> (Entity<Workstation>, &mut VisualTestContext) {
    let (view, cx) = fixture(cx);
    run(
        &view,
        cx,
        SceneCommand::Attach {
            id: Some("street".into()),
            name: None,
            source: SceneSource::Text(BUNDLE.to_string()),
        },
    )
    .unwrap();
    run(
        &view,
        cx,
        SceneCommand::Select {
            scene: Some("street".into()),
            entities: vec![FIGURE.to_string()],
            mode: SelectMode::Replace,
        },
    )
    .unwrap();
    view.update(cx, |view, _| {
        view.viewport.pretend_directed_scene("street");
        view.viewport.pretend_overlay_supported(true);
    });
    cx.run_until_parked();
    (view, cx)
}

/// The overlay the core would send the page now: at the frame the monitor shows (the playhead).
fn overlay(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Value {
    let frame = view.read_with(cx, |view, _| view.state.timeline.playhead);
    run(
        view,
        cx,
        SceneCommand::Gizmo(GizmoCommand::State {
            scene: "street".into(),
            at: tessel_editor_core::scene_interaction::FrameAt::Frame(frame),
        }),
    )
    .unwrap()["overlay"]
        .clone()
}

fn tip_of(overlay: &Value, handle: &str) -> Value {
    overlay["handles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["id"] == handle)
        .unwrap()["to"]
        .clone()
}

/// The stamp of the overlay the core would draw now: what a page that holds the latest overlay
/// echoes in every message.
fn current_stamp(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Value {
    view.read_with(cx, |view, _| {
        view.state
            .gizmo_stamp("street", view.state.timeline.playhead)
    })
}

/// A message of a page that holds the latest overlay (it echoes the current stamp).
fn pointer(view: &Entity<Workstation>, cx: &mut VisualTestContext, message: Value) -> bool {
    let mut message = message;
    if message.get("stamp").is_none() {
        message["stamp"] = current_stamp(view, cx);
    }
    send(view, cx, message)
}

/// A message exactly as given (a stamp of another overlay, or none).
fn send(view: &Entity<Workstation>, cx: &mut VisualTestContext, message: Value) -> bool {
    let mut message = message;
    message["type"] = json!("overlay.pointer");
    let consumed = view.update(cx, |view, cx| view.overlay_ipc(&message, cx));
    cx.run_until_parked();
    consumed
}

fn dragging(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |view, _| view.state.scene_interaction.drag().is_some())
}

fn figure_x(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> f64 {
    view.read_with(cx, |view, _| {
        let document = view
            .state
            .scenes
            .get("street")
            .unwrap()
            .scene
            .document()
            .clone();
        document["sceneDefinitions"][0]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["id"] == "figure")
            .unwrap()["localPosition"]["x"]
            .as_f64()
            .unwrap()
    })
}

fn undo_depth(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> u64 {
    view.read_with(cx, |view, _| {
        view.state.history.to_json()["undo_depth"].as_u64().unwrap()
    })
}

fn nudged(tip: &Value, du: f64) -> Value {
    json!({"u": tip["u"].as_f64().unwrap() + du, "v": tip["v"]})
}

/// Presses the x handle, moves it a little to the right and returns the press point.
fn start_drag(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Value {
    let tip = tip_of(&overlay(view, cx), "x");
    pointer(view, cx, json!({"phase":"down","point":tip}));
    assert!(dragging(view, cx), "the press landed on the x handle");
    pointer(view, cx, json!({"phase":"move","point":nudged(&tip, 0.05)}));
    tip
}

#[gpui_kit::test]
fn a_pointer_drag_from_the_page_is_one_undo_entry_and_moves_the_node(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    let before = (figure_x(&view, cx), undo_depth(&view, cx));
    let tip = start_drag(&view, cx);
    // The move is a candidate: the document and the history are untouched until the release.
    assert_eq!((figure_x(&view, cx), undo_depth(&view, cx)), before);
    pointer(&view, cx, json!({"phase":"up","point":nudged(&tip, 0.05)}));
    assert!(!dragging(&view, cx));
    assert!(
        figure_x(&view, cx) > before.0,
        "the node moved to the right"
    );
    assert_eq!(
        undo_depth(&view, cx),
        before.1 + 1,
        "one entry for the whole drag"
    );

    view.update(cx, |view, cx| view.execute(Command::Undo, Source::Cli, cx))
        .unwrap();
    assert_eq!(figure_x(&view, cx), before.0);
}

#[gpui_kit::test]
fn escape_cancels_the_gizmo_drag_without_a_trace(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    let before = (figure_x(&view, cx), undo_depth(&view, cx));
    let tip = start_drag(&view, cx);
    // The window holds the focus (in the app the WebView would, and the page then handles its
    // own Escape: `overlay.js`); the key reaches the workstation's handler.
    view.update_in(cx, |view, window, cx| window.focus(&view.focus, cx));
    cx.simulate_keystrokes("escape");
    assert!(!dragging(&view, cx));
    assert_eq!((figure_x(&view, cx), undo_depth(&view, cx)), before);
    // The rest of the press (a move and a release the page may still send) is ignored quietly.
    assert!(pointer(
        &view,
        cx,
        json!({"phase":"move","point":nudged(&tip, 0.1)})
    ));
    assert!(pointer(
        &view,
        cx,
        json!({"phase":"up","point":nudged(&tip, 0.1)})
    ));
    assert_eq!((figure_x(&view, cx), undo_depth(&view, cx)), before);
}

#[gpui_kit::test]
fn losing_the_window_focus_cancels_the_drag_and_never_commits_it(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    let before = (figure_x(&view, cx), undo_depth(&view, cx));
    start_drag(&view, cx);
    cx.deactivate_window();
    assert!(!dragging(&view, cx));
    assert_eq!((figure_x(&view, cx), undo_depth(&view, cx)), before);
}

#[gpui_kit::test]
fn a_hidden_monitor_cancels_the_drag(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    let before = (figure_x(&view, cx), undo_depth(&view, cx));
    start_drag(&view, cx);
    view.update(cx, |view, cx| {
        view.cancel_gizmo_drag(CancelReason::Hidden, cx)
    });
    assert!(!dragging(&view, cx));
    assert_eq!((figure_x(&view, cx), undo_depth(&view, cx)), before);
    // Nothing open: a second cancel is a no-op, not an error.
    view.update(cx, |view, cx| {
        view.cancel_gizmo_drag(CancelReason::Hidden, cx)
    });
}

#[gpui_kit::test]
fn a_stale_move_or_release_and_a_foreign_message_are_quiet(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    let before = (figure_x(&view, cx), undo_depth(&view, cx));
    assert!(pointer(
        &view,
        cx,
        json!({"phase":"move","point":{"u":0.5,"v":0.5}})
    ));
    assert!(pointer(
        &view,
        cx,
        json!({"phase":"up","point":{"u":0.5,"v":0.5}})
    ));
    assert!(pointer(
        &view,
        cx,
        json!({"phase":"cancel","reason":"escape"})
    ));
    assert!(pointer(&view, cx, json!({"phase":"nonsense"})));
    assert_eq!((figure_x(&view, cx), undo_depth(&view, cx)), before);
    let foreign = view.update(cx, |view, cx| {
        view.overlay_ipc(&json!({"type":"frame.done"}), cx)
    });
    assert!(
        !foreign,
        "a message that is not the overlay's is not consumed"
    );
}

#[gpui_kit::test]
fn nothing_is_pressed_when_the_monitor_shows_no_directed_scene(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    let tip = tip_of(&overlay(&view, cx), "x");
    view.update(cx, |view, _| view.viewport.clear_directed());
    pointer(&view, cx, json!({"phase":"down","point":tip}));
    assert!(!dragging(&view, cx));
}

#[gpui_kit::test]
fn an_edit_under_the_drag_ends_it_and_the_old_candidate_never_applies(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    let tip = start_drag(&view, cx);
    let before = figure_x(&view, cx);
    view.update(cx, |view, cx| {
        view.run_op(
            &json!({"op":"scene.edit","scene":"street","ops":[
                {"set": "scene/scene-a/node/figure/width", "value": 3.0}]}),
            cx,
        )
    })
    .unwrap();
    cx.run_until_parked();
    pointer(
        &view,
        cx,
        json!({"phase":"move","point":nudged(&tip, 0.08)}),
    );
    assert!(
        !dragging(&view, cx),
        "the stale drag is settled, not left open"
    );
    pointer(&view, cx, json!({"phase":"up","point":nudged(&tip, 0.08)}));
    assert_eq!(
        figure_x(&view, cx),
        before,
        "the candidate was never applied"
    );
}

// ---- identity: a message answers the overlay it was made against ----------------------------

fn stale_count(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> u64 {
    view.read_with(cx, |view, _| {
        view.viewport.overlay_report()["stale"].as_u64().unwrap()
    })
}

#[gpui_kit::test]
fn a_press_that_answers_an_older_overlay_is_refused_and_counted(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    let tip = tip_of(&overlay(&view, cx), "x");
    let before = (figure_x(&view, cx), undo_depth(&view, cx));
    let current = current_stamp(&view, cx);
    for (what, stamp) in [
        ("an older revision", {
            let mut stamp = current.clone();
            stamp["revision"] = json!(stamp["revision"].as_u64().unwrap() + 1);
            stamp
        }),
        ("another frame", {
            let mut stamp = current.clone();
            stamp["frame"] = json!(stamp["frame"].as_i64().unwrap() + 5);
            stamp
        }),
        ("another project session", {
            let mut stamp = current.clone();
            stamp["session"] = json!("00000000-0");
            stamp
        }),
        ("no stamp at all", Value::Null),
    ] {
        let counted = stale_count(&view, cx);
        send(&view, cx, json!({"phase":"down","point":tip,"stamp":stamp}));
        assert!(!dragging(&view, cx), "{what}: no drag opens");
        assert_eq!(stale_count(&view, cx), counted + 1, "{what}: counted");
    }
    assert_eq!((figure_x(&view, cx), undo_depth(&view, cx)), before);
}

#[gpui_kit::test]
fn a_move_or_release_of_another_overlay_ends_the_drag_instead_of_leaving_it_open(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mounted(cx);
    let before = (figure_x(&view, cx), undo_depth(&view, cx));
    let tip = start_drag(&view, cx);
    let mut foreign = current_stamp(&view, cx);
    foreign["session"] = json!("00000000-0");
    send(
        &view,
        cx,
        json!({"phase":"move","point":nudged(&tip, 0.1),"stamp":foreign}),
    );
    assert!(!dragging(&view, cx), "the drag is cancelled, not stuck");
    send(
        &view,
        cx,
        json!({"phase":"up","point":nudged(&tip, 0.1),"stamp":foreign}),
    );
    assert_eq!((figure_x(&view, cx), undo_depth(&view, cx)), before);
    assert!(stale_count(&view, cx) >= 1);
}

#[gpui_kit::test]
fn a_cancel_is_honoured_whatever_overlay_it_answers(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    start_drag(&view, cx);
    send(&view, cx, json!({"phase":"cancel","reason":"escape"}));
    assert!(!dragging(&view, cx));
}

#[gpui_kit::test]
fn a_replaced_project_does_not_answer_to_the_old_projects_press(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    let tip = tip_of(&overlay(&view, cx), "x");
    let old = current_stamp(&view, cx);
    // The same scene ID is attached again in a project of the same revision count: only the
    // session differs, and that is enough to refuse the old page's press.
    view.update(cx, |view, cx| {
        view.run_op(&json!({"op":"project.create","name":"Other"}), cx)
    })
    .unwrap();
    run(
        &view,
        cx,
        SceneCommand::Attach {
            id: Some("street".into()),
            name: None,
            source: SceneSource::Text(BUNDLE.to_string()),
        },
    )
    .unwrap();
    view.update(cx, |view, _| {
        view.viewport.pretend_directed_scene("street");
        view.viewport.pretend_overlay_supported(true);
    });
    run(
        &view,
        cx,
        SceneCommand::Select {
            scene: Some("street".into()),
            entities: vec![FIGURE.to_string()],
            mode: SelectMode::Replace,
        },
    )
    .unwrap();
    send(&view, cx, json!({"phase":"down","point":tip,"stamp":old}));
    assert!(
        !dragging(&view, cx),
        "the old project's overlay does not act on the new one"
    );
    // The new overlay's own press works.
    let tip = tip_of(&overlay(&view, cx), "x");
    pointer(&view, cx, json!({"phase":"down","point":tip}));
    assert!(dragging(&view, cx));
}

// ---- a drag nobody can finish is cancelled ----------------------------------------------------

fn settle(view: &Entity<Workstation>, cx: &mut VisualTestContext) {
    view.update(cx, |view, cx| view.settle_gizmo_drag(cx));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_page_that_went_away_cancels_the_drag(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    let before = (figure_x(&view, cx), undo_depth(&view, cx));
    start_drag(&view, cx);
    settle(&view, cx);
    assert!(dragging(&view, cx), "a live page keeps its drag");
    // The WebView crashed, reloaded or lost its context: nothing will release this press.
    view.update(cx, |view, _| view.viewport.pretend_overlay_supported(false));
    settle(&view, cx);
    assert!(!dragging(&view, cx));
    assert_eq!((figure_x(&view, cx), undo_depth(&view, cx)), before);
}

#[gpui_kit::test]
fn a_monitor_that_no_longer_shows_the_scene_cancels_the_drag(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    let before = (figure_x(&view, cx), undo_depth(&view, cx));
    start_drag(&view, cx);
    view.update(cx, |view, _| view.viewport.clear_directed());
    settle(&view, cx);
    assert!(!dragging(&view, cx));
    assert_eq!((figure_x(&view, cx), undo_depth(&view, cx)), before);
}

#[gpui_kit::test]
fn a_scene_detached_under_the_drag_cancels_it_without_another_pointer_event(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mounted(cx);
    start_drag(&view, cx);
    // The scene is gone (an undo of the attach, a removed import): the candidate can never apply.
    view.update(cx, |view, cx| {
        view.run_op(&json!({"op":"scene.detach","scene":"street"}), cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        !dragging(&view, cx),
        "settled by the sync the command triggers"
    );
    view.read_with(cx, |view, _| {
        assert!(view.state.scene_interaction.gesture().is_none());
    });
}

#[gpui_kit::test]
fn an_undo_under_the_drag_cancels_it_and_the_candidate_is_gone(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    // A committed move first, so that undo has something to take back.
    let tip = start_drag(&view, cx);
    pointer(&view, cx, json!({"phase":"up","point":nudged(&tip, 0.05)}));
    let moved = figure_x(&view, cx);
    start_drag(&view, cx);
    view.update(cx, |view, cx| view.execute(Command::Undo, Source::Cli, cx))
        .unwrap();
    cx.run_until_parked();
    assert!(!dragging(&view, cx));
    assert!(
        figure_x(&view, cx) < moved,
        "the undo took the first move back"
    );
    view.read_with(cx, |view, _| {
        assert!(
            view.state.scene_interaction.gesture().is_none(),
            "the invalidated candidate does not linger"
        );
    });
}

#[gpui_kit::test]
fn seeking_away_from_the_pressed_frame_cancels_the_drag(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    let before = (figure_x(&view, cx), undo_depth(&view, cx));
    start_drag(&view, cx);
    view.update(cx, |view, cx| {
        view.run_op(&json!({"op":"seek","frame":12}), cx)
    })
    .unwrap();
    settle(&view, cx);
    assert!(!dragging(&view, cx));
    assert_eq!((figure_x(&view, cx), undo_depth(&view, cx)), before);
}

#[gpui_kit::test]
fn a_dialog_over_the_monitor_hides_the_page_and_cancels_the_drag(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    let before = (figure_x(&view, cx), undo_depth(&view, cx));
    start_drag(&view, cx);
    // The native view paints above every dialog, so a dialog hides it, and a hidden page gets
    // no release.
    view.update(cx, |view, _| view.workflow.pretend_dialog_open(true));
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(view.read_with(cx, |view, _| view.workflow.has_dialog()));
    assert!(!view.read_with(cx, |view, _| view.viewport.on_screen));
    assert!(!dragging(&view, cx));
    assert_eq!((figure_x(&view, cx), undo_depth(&view, cx)), before);
}
