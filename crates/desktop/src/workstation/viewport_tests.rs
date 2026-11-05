//! The project operations and the viewport node as the live window serves them. The WebView
//! itself cannot exist in a GPUI test window, so the viewport is reported unavailable here and
//! these tests pin down what that means for the API; the page lifecycle is covered by
//! `viewport_host_tests` and the real picture by `scripts/live-proof.py`.
use serde_json::Value;

use super::input_tests::fixture;
use super::*;
use crate::workstation::OpReply;
use core::prelude::v1::test;
use tessel_editor_core::commands::Parsed;

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

fn build_title_project(view: &Entity<Workstation>, cx: &mut VisualTestContext) {
    for request in [
        json!({"op":"project.create","name":"Ops","width":160,"height":90,"length":48}),
        json!({"op":"asset.add","kind":"text","id":"label","name":"Label",
            "text":{"content":"HI","color":"#ffffff","size":12}}),
        json!({"op":"track.add","id":"titles","name":"Titles","kind":"video"}),
        json!({"op":"clip.add","id":"hi","track":"titles","asset":"label","start":0,"duration":24}),
    ] {
        op(view, cx, request.clone()).unwrap_or_else(|error| panic!("{request}: {error:?}"));
    }
}

#[gpui_kit::test]
fn the_tree_describes_the_viewport_and_where_it_lives(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let (node, monitor) = view.read_with(cx, |view, cx| {
        (
            view.tree_node("monitor.viewport", cx).unwrap(),
            view.tree_node("region:monitor", cx).unwrap(),
        )
    });
    assert_eq!(node["kind"], "viewport");
    assert_eq!(node["parent"], "region:monitor");
    assert!(
        monitor["children"]
            .as_array()
            .unwrap()
            .contains(&json!("monitor.viewport"))
    );
    // No WebView exists in a test window: the node says so, with the setup command.
    assert_eq!(node["state"]["phase"], "unavailable");
    assert!(
        node["state"]["session"]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("pnpm")
    );
}

#[gpui_kit::test]
fn operations_build_a_project_through_the_window_state(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    build_title_project(&view, cx);
    let summary = view.read_with(cx, |view, _| view.state.summary());
    assert_eq!(summary["project"], "Ops");
    assert_eq!(summary["demonstration"], false);

    let moved = op(&view, cx, json!({"op":"seek","frame":12})).unwrap();
    assert_eq!(moved["value"], 12);
    let plan = op(&view, cx, json!({"op":"plan"})).unwrap();
    assert_eq!(plan["layers"][0]["kind"], "text");
    assert_eq!(plan["frame"], 12);
}

#[gpui_kit::test]
fn a_rejected_operation_changes_nothing(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    build_title_project(&view, cx);
    let before = view.read_with(cx, |view, _| view.state.summary());
    let error = op(
        &view,
        cx,
        json!({"op":"clip.add","id":"x","track":"no-such-track","asset":"label","start":0,"duration":5}),
    )
    .unwrap_err();
    assert_eq!(error.code, "unknown_element");
    let error = op(
        &view,
        cx,
        json!({"op":"project.create","name":"Bad","width":0}),
    )
    .unwrap_err();
    assert!(!error.code.is_empty());
    assert_eq!(before, view.read_with(cx, |view, _| view.state.summary()));
}

#[gpui_kit::test]
fn frames_and_capabilities_report_the_missing_runtime_instead_of_faking_one(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    build_title_project(&view, cx);
    let before = view.read_with(cx, |view, _| view.state.summary());
    let frame = view.update(cx, |view, cx| {
        view.run_op(&json!({"op":"render.frame","frame":0}), cx)
    });
    assert_eq!(frame.err().unwrap().code, "backend_unavailable");
    let caps = op(&view, cx, json!({"op":"capabilities"})).unwrap_err();
    assert_eq!(caps.code, "backend_unavailable");
    assert_eq!(before, view.read_with(cx, |view, _| view.state.summary()));
    // Offline audio needs no viewport.
    let audio = op(&view, cx, json!({"op":"render.audio","from":0,"frames":24})).unwrap();
    assert_eq!(audio["kind"], "processed");
}

#[gpui_kit::test]
fn operations_that_are_not_project_operations_are_named_as_such(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let error = op(
        &view,
        cx,
        json!({"op":"set","target":"x","field":"y","value":"z"}),
    )
    .unwrap_err();
    assert_eq!(error.code, "unknown_operation");
}

#[gpui_kit::test]
fn hiding_the_monitor_hides_the_native_view_bookkeeping(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let on_screen = |view: &Entity<Workstation>, cx: &mut VisualTestContext| {
        view.read_with(cx, |view, cx| {
            view.tree_node("monitor.viewport", cx).unwrap()["state"]["on_screen"].clone()
        })
    };
    assert_eq!(on_screen(&view, cx), json!(true));
    view.update_in(cx, |view, window, cx| {
        view.execute_layout(
            LayoutCommand::SetVisible {
                panel: PanelKind::Monitor,
                visible: false,
            },
            Source::Cli,
            window,
            cx,
        )
        .unwrap();
    });
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert_eq!(on_screen(&view, cx), json!(false));
    view.update_in(cx, |view, window, cx| {
        view.execute_layout(
            LayoutCommand::SetVisible {
                panel: PanelKind::Monitor,
                visible: true,
            },
            Source::Cli,
            window,
            cx,
        )
        .unwrap();
    });
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert_eq!(on_screen(&view, cx), json!(true));
}

#[gpui_kit::test]
fn the_monitor_modes_are_semantic_controls(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let node = |view: &Entity<Workstation>, cx: &mut VisualTestContext, id: &str| {
        view.read_with(cx, |view, cx| view.tree_node(id, cx).unwrap())
    };
    assert_eq!(
        node(&view, cx, "monitor.preview.sequence")["state"]["selected"],
        true
    );
    // The demonstration opens on a clip: its media can be previewed alone.
    let source = node(&view, cx, "monitor.preview.source");
    assert_eq!(source["enabled"], true);

    let tree_command = |view: &Entity<Workstation>, cx: &mut VisualTestContext, target: &str| {
        view.read_with(cx, |view, cx| {
            let layout = view.layout_snapshot(cx);
            tessel_editor_core::commands::parse_invoke(&view.state, &layout, target, "invoke")
        })
    };
    let Parsed::Run(command) = tree_command(&view, cx, "monitor.preview.source").unwrap() else {
        panic!("a mode button is a state command");
    };
    view.update(cx, |view, cx| view.execute(command, Source::Cli, cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        node(&view, cx, "monitor.preview.source")["state"]["selected"],
        true
    );
    assert_eq!(
        node(&view, cx, "monitor.preview.sequence")["state"]["selected"],
        false
    );
    let monitor = node(&view, cx, "monitor");
    assert!(
        monitor["state"]["preview"]
            .as_str()
            .unwrap()
            .contains("source")
    );
}
