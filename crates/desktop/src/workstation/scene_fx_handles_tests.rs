//! F061 (G4.1, BL082.35): the desktop half of the vignette handles, driven through the entry the
//! page uses (`overlay.pointer` messages), the panel's `handles` action and the window's own
//! Escape and dialog paths.
//!
//! A GPUI test window has no WebView, so the canvas itself is
//! `runtime/test/fx-handles-overlay.test.mjs` (real Chrome, real mouse events) and the geometry
//! and the drag rules are `crates/core/tests/scene_fx_handles.rs`. What is proven here is the
//! seam: the panel action is the `scene.fx.select` a script sends, a message becomes the one core
//! command, a stale one is counted and cancels, and Escape / a dialog / another selection end the
//! drag without a trace.
//!
//! Not proved here: pixels in the monitor and a physical pointer on the WebView.
use serde_json::{Value, json};

use super::scene_fx_tests::{center_of, field_value, invoke, mounted, node, undo_depth};
use super::*;
// The Kit's test macro expands to `#[test]`; the glob import above would resolve it to the Kit macro again.
use core::prelude::v1::test;

use tessel_editor_core::directed_scene::fx::FxValue;
use tessel_editor_core::scene_interaction::{FrameAt, SelectMode};
use tessel_editor_core::scene_project::SceneCommand;

const CLIP: &str = "scene.fx.clip:fx-1";

/// The fixture scene with one vignette (`fx-1`, at the head of the edit) shown on a page that
/// announced `overlay.gizmo`, and the clip's handles on.
fn with_handles(cx: &mut TestAppContext) -> (Entity<Workstation>, &mut VisualTestContext) {
    let (view, cx) = mounted(cx);
    invoke(&view, cx, "scene.fx", "add:vignette").unwrap();
    view.update(cx, |view, cx| {
        view.viewport.pretend_directed_scene("street");
        view.viewport.pretend_overlay_supported(true);
        view.viewport_sync(cx);
    });
    cx.run_until_parked();
    invoke(&view, cx, CLIP, "handles").unwrap();
    (view, cx)
}

fn handles(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Value {
    let frame = view.read_with(cx, |view, _| view.state.timeline.playhead);
    view.update(cx, |view, cx| {
        view.execute(
            Command::Scene(SceneCommand::FxHandles {
                scene: "street".into(),
                clip: Some("fx-1".into()),
                at: FrameAt::Frame(frame),
                probe: None,
            }),
            Source::Cli,
            cx,
        )
    })
    .unwrap()
}

fn at_of(answer: &Value, handle: &str) -> Value {
    answer["handles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["id"] == handle)
        .unwrap_or_else(|| panic!("no handle `{handle}` in {answer}"))["at"]
        .clone()
}

fn nudged(at: &Value, du: f64) -> Value {
    json!({"u": at["u"].as_f64().unwrap() + du, "v": at["v"]})
}

fn current_stamp(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Value {
    view.read_with(cx, |view, _| {
        view.overlay_stamp("street", view.state.timeline.playhead)
    })
}

fn send(view: &Entity<Workstation>, cx: &mut VisualTestContext, message: Value) -> bool {
    let mut message = message;
    message["type"] = json!("overlay.pointer");
    let consumed = view.update(cx, |view, cx| view.overlay_ipc(&message, cx));
    cx.run_until_parked();
    consumed
}

/// A message of a page that holds the latest overlay (it echoes the current stamp).
fn pointer(view: &Entity<Workstation>, cx: &mut VisualTestContext, message: Value) -> bool {
    let mut message = message;
    if message.get("stamp").is_none() {
        message["stamp"] = current_stamp(view, cx);
    }
    send(view, cx, message)
}

fn dragging(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |view, _| view.state.scene_interaction.drag().is_some())
}

fn stale_count(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> u64 {
    view.read_with(cx, |view, _| {
        view.viewport.overlay_report()["stale"].as_u64().unwrap()
    })
}

fn radius(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> f64 {
    field_value(&node(view, cx, CLIP), "radius")
        .as_f64()
        .unwrap()
}

/// The radius the document holds (the panel shows the candidate while a gesture is open).
fn document_radius(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> f64 {
    view.read_with(cx, |view, _| {
        let attached = view.state.scenes.get("street").unwrap();
        let (_, clip) = attached.scene.bundle().fx.clip("fx-1").unwrap();
        match clip.param("radius").unwrap() {
            FxValue::Number(value) => value,
            other => panic!("radius is a number, got {other:?}"),
        }
    })
}

fn gizmo(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Value {
    node(view, cx, "monitor.gizmo")
}

/// Presses the radius handle and moves it a little to the right; returns where it was pressed.
fn start_drag(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Value {
    let at = at_of(&handles(view, cx), "radius");
    pointer(view, cx, json!({"phase":"down","point":at}));
    assert!(dragging(view, cx), "the press landed on the radius handle");
    pointer(view, cx, json!({"phase":"move","point":nudged(&at, 0.05)}));
    at
}

#[gpui_kit::test]
fn the_handles_action_is_the_shared_select_and_the_tree_says_what_is_targeted(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mounted(cx);
    invoke(&view, cx, "scene.fx", "add:vignette").unwrap();
    invoke(&view, cx, "scene.fx", "add:fade").unwrap();

    let vignette = node(&view, cx, CLIP);
    let action = vignette["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["action"] == "handles")
        .expect("the vignette offers its handles")
        .clone();
    assert_eq!(action["enabled"], json!(true));
    assert_eq!(vignette["state"]["handles"], json!(false));
    assert_eq!(gizmo(&view, cx)["state"]["target"], json!("node"));

    // The effect without handles says why, and the action is refused by name.
    let fade = node(&view, cx, "scene.fx.clip:fx-2");
    let off = fade["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["action"] == "handles")
        .unwrap();
    assert_eq!(off["enabled"], json!(false));
    assert_eq!(
        off["disabled_reason"],
        json!("Só a vinheta tem alças no monitor")
    );
    let refused = invoke(&view, cx, "scene.fx.clip:fx-2", "handles").unwrap_err();
    assert_eq!(refused.code, "fx_has_no_handles");

    // On: the same state `scene.fx.select` leaves, on the clip node and the gizmo node.
    let before = undo_depth(&view, cx);
    invoke(&view, cx, CLIP, "handles").unwrap();
    let lit = node(&view, cx, CLIP);
    assert_eq!(lit["state"]["handles"], json!(true));
    let lit_action = lit["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["action"] == "handles")
        .unwrap();
    assert_eq!(
        lit_action["label"], action["label"],
        "the label is static: whether it is on is the state, not a second wording"
    );
    let on = gizmo(&view, cx);
    assert_eq!(on["state"]["target"], json!("fx"));
    assert_eq!(on["state"]["fx_clip"], json!("fx-1"));
    assert_eq!(undo_depth(&view, cx), before, "a target is not an edit");

    // Off again: the same action toggles.
    invoke(&view, cx, CLIP, "handles").unwrap();
    assert_eq!(node(&view, cx, CLIP)["state"]["handles"], json!(false));
    assert_eq!(gizmo(&view, cx)["state"]["target"], json!("node"));
}

/// F062 x F061 (BL082.64): a vignette laid on a shot has no handles on the monitor. The core says it
/// once (`FxClip::handles_refusal`) and the node, the button and the select answer with it; a clip
/// that turns into a shot clip while it is the target leaves the target at the edit, with no read.
#[gpui_kit::test]
fn a_vignette_on_a_shot_has_no_handles_and_the_target_leaves_it(cx: &mut TestAppContext) {
    const REASON: &str = "As alças só existem para efeitos na saída; este efeito está num shot";
    let (view, cx) = mounted(cx);
    let handles_action = |view: &Entity<Workstation>, cx: &mut VisualTestContext| {
        node(view, cx, CLIP)["actions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|action| action["action"] == "handles")
            .expect("the node always lists `handles`")
            .clone()
    };
    let handles_on = |view: &Entity<Workstation>, cx: &mut VisualTestContext| {
        view.read_with(cx, |view, _| view.fx_handles_on("street", "fx-1"))
    };
    let click_button = |view: &Entity<Workstation>, cx: &mut VisualTestContext| {
        // A paint follows the change: the probe is the button of this state.
        view.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        let button = center_of(view, cx, "control:fx-handles:fx-1");
        cx.simulate_click(button, Default::default());
        cx.run_until_parked();
    };

    // Laid on the first shot: the action says why, and nothing the window offers turns them on. The
    // button is inert: a click neither shows the handles nor raises the refusal a live one would.
    invoke(&view, cx, "scene.fx", "add_in_shot:vignette").unwrap();
    let off = handles_action(&view, cx);
    assert_eq!(off["enabled"], json!(false));
    assert_eq!(off["disabled_reason"], json!(REASON));
    let depth = undo_depth(&view, cx);
    click_button(&view, cx);
    assert!(!handles_on(&view, cx), "the disabled button does nothing");
    assert!(
        view.read_with(cx, |view, _| view.scene_ui.notice.is_none()),
        "and it asks the core nothing: no refusal reaches the panel"
    );
    let refused = invoke(&view, cx, CLIP, "handles").unwrap_err();
    assert_eq!(refused.code, "scope_not_supported");
    assert_eq!(refused.target.as_deref(), Some("fx-1"));
    let selected = view.update(cx, |view, cx| {
        view.execute(
            Command::Scene(SceneCommand::FxSelect {
                scene: "street".into(),
                clip: Some("fx-1".into()),
            }),
            Source::Cli,
            cx,
        )
    });
    assert_eq!(selected.unwrap_err().code, "scope_not_supported");
    assert_eq!(gizmo(&view, cx)["state"]["target"], json!("node"));
    assert_eq!(undo_depth(&view, cx), depth, "a refusal writes nothing");

    // Back on the output: the same action and the same button are live.
    invoke(&view, cx, CLIP, "set_scope:output").unwrap();
    let on = handles_action(&view, cx);
    assert_eq!(on["enabled"], json!(true));
    assert_eq!(on["disabled_reason"], Value::Null);
    click_button(&view, cx);
    assert!(handles_on(&view, cx), "the button shows the handles");
    assert_eq!(gizmo(&view, cx)["state"]["fx_clip"], json!("fx-1"));

    // The target clip is laid on a shot: the target is dropped by the edit itself, before any read.
    invoke(&view, cx, CLIP, "set_scope:shot:shot-1").unwrap();
    assert_eq!(
        (
            gizmo(&view, cx)["state"]["target"].clone(),
            gizmo(&view, cx)["state"]["fx_clip"].clone()
        ),
        (json!("node"), json!(""))
    );
    assert!(!handles_on(&view, cx));
    assert_eq!(handles_action(&view, cx)["enabled"], json!(false));
}

#[gpui_kit::test]
fn a_drag_from_the_page_is_one_undo_entry_and_moves_the_radius(cx: &mut TestAppContext) {
    let (view, cx) = with_handles(cx);
    let (value, depth) = (document_radius(&view, cx), undo_depth(&view, cx));
    let at = start_drag(&view, cx);
    // The move is a candidate: the panel shows it, the document and the history are untouched
    // until the release.
    assert!(
        radius(&view, cx) > value,
        "the panel previews the candidate"
    );
    assert_eq!(
        (document_radius(&view, cx), undo_depth(&view, cx)),
        (value, depth)
    );
    pointer(&view, cx, json!({"phase":"up","point":nudged(&at, 0.05)}));
    assert!(!dragging(&view, cx));
    assert!(document_radius(&view, cx) > value, "the radius grew");
    assert_eq!(undo_depth(&view, cx), depth + 1, "one entry for the drag");
    assert_eq!(
        view.read_with(cx, |view, _| view.state.history.to_json()["undo_label"]
            .clone()),
        json!("Edit effects")
    );

    view.update(cx, |view, cx| view.execute(Command::Undo, Source::Cli, cx))
        .unwrap();
    assert_eq!(document_radius(&view, cx), value);
}

#[gpui_kit::test]
fn escape_cancels_the_handle_drag_without_a_trace(cx: &mut TestAppContext) {
    let (view, cx) = with_handles(cx);
    let (value, depth) = (document_radius(&view, cx), undo_depth(&view, cx));
    let at = start_drag(&view, cx);
    view.update_in(cx, |view, window, cx| window.focus(&view.focus, cx));
    cx.simulate_keystrokes("escape");
    assert!(!dragging(&view, cx));
    assert_eq!(
        (document_radius(&view, cx), undo_depth(&view, cx)),
        (value, depth)
    );
    // The rest of the press (a move and a release the page may still send) is ignored quietly.
    assert!(pointer(
        &view,
        cx,
        json!({"phase":"move","point":nudged(&at, 0.1)})
    ));
    assert!(pointer(
        &view,
        cx,
        json!({"phase":"up","point":nudged(&at, 0.1)})
    ));
    assert_eq!(
        (document_radius(&view, cx), undo_depth(&view, cx)),
        (value, depth)
    );
}

#[gpui_kit::test]
fn a_stale_press_is_counted_and_a_stale_move_ends_the_drag(cx: &mut TestAppContext) {
    let (view, cx) = with_handles(cx);
    let (value, depth) = (document_radius(&view, cx), undo_depth(&view, cx));
    let at = at_of(&handles(&view, cx), "radius");

    // A page that still holds the node gizmo's overlay (no clip in its stamp) is not the page of
    // the handles: its press opens nothing and is counted.
    let mut node_stamp = current_stamp(&view, cx);
    node_stamp.as_object_mut().unwrap().remove("fx_clip");
    let counted = stale_count(&view, cx);
    send(
        &view,
        cx,
        json!({"phase":"down","point":at,"stamp":node_stamp}),
    );
    assert!(!dragging(&view, cx));
    assert_eq!(stale_count(&view, cx), counted + 1);

    // A page of another project session loses a drag it had open.
    let at = start_drag(&view, cx);
    let mut foreign = current_stamp(&view, cx);
    foreign["session"] = json!("00000000-0");
    send(
        &view,
        cx,
        json!({"phase":"move","point":nudged(&at, 0.1),"stamp":foreign}),
    );
    assert!(!dragging(&view, cx), "cancelled, not stuck");
    assert_eq!(
        (document_radius(&view, cx), undo_depth(&view, cx)),
        (value, depth)
    );
}

#[gpui_kit::test]
fn selecting_another_target_under_the_drag_ends_it_and_leaves_no_phantom(cx: &mut TestAppContext) {
    let (view, cx) = with_handles(cx);
    let (value, depth) = (document_radius(&view, cx), undo_depth(&view, cx));
    let at = start_drag(&view, cx);
    view.update(cx, |view, cx| {
        view.execute(
            Command::Scene(SceneCommand::Select {
                scene: Some("street".into()),
                entities: vec!["scene/scene-a/node/figure".into()],
                mode: SelectMode::Replace,
            }),
            Source::Cli,
            cx,
        )
    })
    .unwrap();
    assert!(!dragging(&view, cx));
    assert_eq!(gizmo(&view, cx)["state"]["target"], json!("node"));
    // The release of the old press finds no drag and writes nothing.
    pointer(&view, cx, json!({"phase":"up","point":nudged(&at, 0.1)}));
    assert_eq!(
        (document_radius(&view, cx), undo_depth(&view, cx)),
        (value, depth)
    );
}

#[gpui_kit::test]
fn a_dialog_over_the_monitor_cancels_the_handle_drag(cx: &mut TestAppContext) {
    let (view, cx) = with_handles(cx);
    let (value, depth) = (document_radius(&view, cx), undo_depth(&view, cx));
    start_drag(&view, cx);
    view.update(cx, |view, _| view.workflow.pretend_dialog_open(true));
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(!dragging(&view, cx), "the dialog took the drag with it");
    assert_eq!(
        (document_radius(&view, cx), undo_depth(&view, cx)),
        (value, depth)
    );
}

#[gpui_kit::test]
fn an_undo_under_the_drag_cancels_it_and_the_candidate_is_gone(cx: &mut TestAppContext) {
    let (view, cx) = with_handles(cx);
    // One real edit first, so an undo has something to take.
    invoke(&view, cx, "scene.fx", "add:fade").unwrap();
    let value = document_radius(&view, cx);
    start_drag(&view, cx);
    view.update(cx, |view, cx| view.execute(Command::Undo, Source::Cli, cx))
        .unwrap();
    view.update(cx, |view, cx| view.viewport_sync(cx));
    cx.run_until_parked();
    assert!(!dragging(&view, cx));
    assert_eq!(
        document_radius(&view, cx),
        value,
        "the candidate never applied"
    );
    assert_eq!(
        radius(&view, cx),
        value,
        "and the panel is back on the document"
    );
}
