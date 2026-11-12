//! F060: the desktop half of the point light: the inspector (colour, intensity, range, position),
//! the gizmo seam (`overlay.pointer` messages on the arrows and the range knob) and the window's
//! own Escape path, driven the way the rest of the scene panel is.
//!
//! A GPUI test window has no WebView, so the canvas is `runtime/test/gizmo-overlay.test.mjs`
//! (real Chrome) and the geometry and the equivalences are `crates/core/tests/scene_light.rs`.
//! What is proven here: the inspector writes the same document the gizmo and `scene.range` /
//! `scene.translate` write, a light with an animated position says why it cannot be moved, and
//! Escape ends a light drag without a trace. No physical pointer, no WKWebView.
use serde_json::{Value, json};

use super::input_tests::fixture;
use super::scene_gesture::SliderKey;
use super::scene_ui::InspectorMode;
use super::*;
// The Kit's test macro expands to `#[test]`; the glob import above would resolve it to the Kit macro again.
use core::prelude::v1::test;

use tessel_editor_core::scene_interaction::{FrameAt, GizmoCommand, SelectMode};
use tessel_editor_core::scene_project::{SceneCommand, SceneSource};

const BUNDLE: &str = include_str!(
    "../../../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json"
);
const LAMP: &str = "scene/scene-a/light/lamp";
const RADIUS: &str = "scene/scene-a/light/lamp/radius";
const INTENSITY: &str = "scene/scene-a/light/lamp/intensity";
const POSITION: &str = "scene/scene-a/light/lamp/position";

fn bundle(keyed_position: bool) -> String {
    let mut document: Value = serde_json::from_str(BUNDLE).unwrap();
    let mut light = json!({
        "id": "lamp",
        "position": {"x": -0.5, "y": 0.3, "z": 0.5},
        "color": {"r": 1.0, "g": 0.5, "b": 0.25},
        "intensity": 2.0, "radius": 1.5, "flicker": 0.0,
    });
    if keyed_position {
        light["keys"] = json!({"position": [
            {"frame": 0, "value": {"x": -0.5, "y": 0.3, "z": 0.5}, "easing": "linear"},
            {"frame": 10, "value": {"x": 0.5, "y": 0.3, "z": 0.5}, "easing": "linear"},
        ]});
    }
    document["sceneDefinitions"][0]["lighting"] = json!({"ambient": 1, "lights": [light]});
    document.to_string()
}

fn run(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    command: SceneCommand,
) -> Result<Value, CommandError> {
    view.update(cx, |view, cx| {
        view.execute(Command::Scene(command), Source::Cli, cx)
    })
}

/// The scene with one point light attached, shown on the monitor, the lamp selected, the scene
/// inspector showing, on a page that announced `overlay.gizmo`.
fn mounted(
    cx: &mut TestAppContext,
    keyed_position: bool,
) -> (Entity<Workstation>, &mut VisualTestContext) {
    let (view, cx) = fixture(cx);
    run(
        &view,
        cx,
        SceneCommand::Attach {
            id: Some("street".into()),
            name: None,
            source: SceneSource::Text(bundle(keyed_position)),
        },
    )
    .unwrap();
    view.update(cx, |view, cx| view.scene_set_mode(InspectorMode::Scene, cx));
    run(
        &view,
        cx,
        SceneCommand::Select {
            scene: Some("street".into()),
            entities: vec![LAMP.to_string()],
            mode: SelectMode::Replace,
        },
    )
    .unwrap();
    view.update(cx, |view, cx| {
        view.viewport.pretend_directed_scene("street");
        view.viewport.pretend_overlay_supported(true);
        view.viewport_sync(cx);
    });
    cx.run_until_parked();
    (view, cx)
}

fn node(view: &Entity<Workstation>, cx: &mut VisualTestContext, id: &str) -> Value {
    view.read_with(cx, |view, cx| view.tree_node(id, cx))
        .unwrap_or_else(|| panic!("no semantic node `{id}`"))
}

fn light(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Value {
    view.read_with(cx, |view, _| {
        view.state.scenes.get("street").unwrap().scene.document()["sceneDefinitions"][0]["lighting"]
            ["lights"][0]
            .clone()
    })
}

fn document(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Value {
    view.read_with(cx, |view, _| {
        view.state
            .scenes
            .get("street")
            .unwrap()
            .scene
            .document()
            .clone()
    })
}

fn undo_depth(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> u64 {
    view.read_with(cx, |view, _| {
        view.state.history.to_json()["undo_depth"].as_u64().unwrap()
    })
}

fn set(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    path: &str,
    field: &str,
    raw: Raw,
) -> Result<Value, CommandError> {
    view.update(cx, |view, cx| {
        view.scene_set(&format!("scene.property:{path}"), field, &raw, cx)
    })
    .expect("a scene node")
}

fn overlay(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Value {
    let frame = view.read_with(cx, |view, _| view.state.timeline.playhead);
    run(
        view,
        cx,
        SceneCommand::Gizmo(GizmoCommand::State {
            scene: "street".into(),
            at: FrameAt::Frame(frame),
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
        .unwrap_or_else(|| panic!("no handle `{handle}` in {overlay}"))["to"]
        .clone()
}

fn nudged(tip: &Value, du: f64) -> Value {
    json!({"u": tip["u"].as_f64().unwrap() + du, "v": tip["v"]})
}

fn pointer(view: &Entity<Workstation>, cx: &mut VisualTestContext, message: Value) -> bool {
    let mut message = message;
    message["stamp"] = view.read_with(cx, |view, _| {
        view.overlay_stamp("street", view.state.timeline.playhead)
    });
    message["type"] = json!("overlay.pointer");
    let consumed = view.update(cx, |view, cx| view.overlay_ipc(&message, cx));
    cx.run_until_parked();
    consumed
}

fn dragging(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |view, _| view.state.scene_interaction.drag().is_some())
}

#[gpui_kit::test]
fn the_light_is_in_the_tree_and_its_properties_are_in_the_inspector(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx, false);
    let entity = node(&view, cx, &format!("scene.entity:{LAMP}"));
    assert_eq!(entity["state"]["selected"], json!(true));
    assert_eq!(entity["state"]["entity_kind"], json!("light"));
    for property in [
        POSITION,
        INTENSITY,
        RADIUS,
        "scene/scene-a/light/lamp/color/r",
        "scene/scene-a/light/lamp/color/g",
        "scene/scene-a/light/lamp/color/b",
    ] {
        let semantic = node(&view, cx, &format!("scene.property:{property}"));
        assert_eq!(semantic["enabled"], json!(true), "{property}: {semantic}");
    }
}

#[gpui_kit::test]
fn the_inspector_writes_the_document_scene_range_and_scene_translate_write(
    cx: &mut TestAppContext,
) {
    // Inspector.
    let (view, cx) = mounted(cx, false);
    let depth = undo_depth(&view, cx);
    set(&view, cx, RADIUS, "value", Raw::Number(1.8)).unwrap();
    set(&view, cx, POSITION, "x", Raw::Number(-0.25)).unwrap();
    assert_eq!(undo_depth(&view, cx), depth + 2, "one entry per edit");
    let typed = light(&view, cx);
    assert_eq!(typed["radius"].as_f64(), Some(1.8));
    for _ in 0..2 {
        view.update(cx, |view, cx| view.execute(Command::Undo, Source::Cli, cx))
            .unwrap();
    }
    assert_eq!(light(&view, cx)["radius"].as_f64(), Some(1.5));

    // The numeric commands the gizmo's drags are made of, on the same scene.
    run(
        &view,
        cx,
        SceneCommand::Gizmo(GizmoCommand::Range {
            scene: "street".into(),
            entity: LAMP.into(),
            at: FrameAt::Frame(0),
            // The delta the oracle writes by hand: new radius minus old.
            delta: 1.8 - 1.5,
        }),
    )
    .unwrap();
    run(
        &view,
        cx,
        SceneCommand::Gizmo(GizmoCommand::Translate {
            scene: "street".into(),
            entity: LAMP.into(),
            at: FrameAt::Frame(0),
            delta: tessel_editor_core::directed_scene::Vec3 {
                x: -0.25 - -0.5,
                y: 0.0,
                z: 0.0,
            },
        }),
    )
    .unwrap();
    let numeric = light(&view, cx);
    // The sums differ by an ulp at most (`1.5 + (1.8 - 1.5)` against the typed `1.8`); every other
    // field is the same.
    assert!((numeric["radius"].as_f64().unwrap() - 1.8).abs() < 1e-12);
    assert!((numeric["position"]["x"].as_f64().unwrap() - -0.25).abs() < 1e-12);
    let (mut left, mut right) = (typed, numeric);
    for side in [&mut left, &mut right] {
        side["radius"] = json!(0);
        side["position"]["x"] = json!(0);
    }
    assert_eq!(left, right);
}

#[gpui_kit::test]
fn a_refused_value_is_a_typed_error_and_leaves_the_light(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx, false);
    let before = (document(&view, cx), undo_depth(&view, cx));
    let error = set(&view, cx, RADIUS, "value", Raw::Number(-1.0)).unwrap_err();
    assert!(!error.code.is_empty());
    let error = set(&view, cx, INTENSITY, "value", Raw::Number(21.0)).unwrap_err();
    assert!(!error.code.is_empty());
    assert_eq!((document(&view, cx), undo_depth(&view, cx)), before);
}

#[gpui_kit::test]
fn an_animated_position_is_disabled_with_its_reason_and_refuses_the_edit(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx, true);
    let semantic = node(&view, cx, &format!("scene.property:{POSITION}"));
    assert_eq!(semantic["enabled"], json!(false));
    let reason = semantic["disabled_reason"].as_str().unwrap();
    assert!(reason.contains("keyframes"), "{reason}");
    let before = (document(&view, cx), undo_depth(&view, cx));
    let error = set(&view, cx, POSITION, "x", Raw::Number(0.0)).unwrap_err();
    assert_eq!(error.code, "disabled_element");
    assert_eq!((document(&view, cx), undo_depth(&view, cx)), before);
    // Its other properties are still the user's.
    let radius = node(&view, cx, &format!("scene.property:{RADIUS}"));
    assert_eq!(radius["enabled"], json!(true));
}

#[gpui_kit::test]
fn a_range_drag_from_the_page_is_one_undo_entry_and_escape_leaves_no_trace(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mounted(cx, false);
    let before = (light(&view, cx), undo_depth(&view, cx));
    let knob = tip_of(&overlay(&view, cx), "range");

    // A drag released.
    pointer(&view, cx, json!({"phase":"down","point":knob}));
    assert!(dragging(&view, cx), "the press landed on the range knob");
    pointer(
        &view,
        cx,
        json!({"phase":"move","point":nudged(&knob, 0.05)}),
    );
    // The move is a candidate: the document and the history are untouched until the release.
    assert_eq!((light(&view, cx), undo_depth(&view, cx)), before);
    pointer(&view, cx, json!({"phase":"up","point":nudged(&knob, 0.05)}));
    assert!(!dragging(&view, cx));
    let grown = light(&view, cx)["radius"].as_f64().unwrap();
    assert!(grown > 1.5, "the range grew to {grown}");
    assert_eq!(undo_depth(&view, cx), before.1 + 1);
    view.update(cx, |view, cx| view.execute(Command::Undo, Source::Cli, cx))
        .unwrap();
    assert_eq!(light(&view, cx)["radius"].as_f64().unwrap(), 1.5);

    // A drag cancelled by Escape.
    let before = (light(&view, cx), undo_depth(&view, cx));
    let knob = tip_of(&overlay(&view, cx), "range");
    pointer(&view, cx, json!({"phase":"down","point":knob}));
    pointer(
        &view,
        cx,
        json!({"phase":"move","point":nudged(&knob, 0.05)}),
    );
    view.update_in(cx, |view, window, cx| window.focus(&view.focus, cx));
    cx.simulate_keystrokes("escape");
    assert!(!dragging(&view, cx));
    assert_eq!((light(&view, cx), undo_depth(&view, cx)), before);
}

#[gpui_kit::test]
fn the_radius_slider_is_one_gesture_and_one_undo_entry(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx, false);
    let key = SliderKey {
        scene: "street".into(),
        entity: LAMP.into(),
        path: RADIUS.into(),
        index: 0,
    };
    let id = format!("control:{}", key.element_id());
    let bounds = view.read_with(cx, |view, _| view.geometry.get(&id).unwrap());
    let at = |fraction: f32| {
        point(
            bounds.left() + bounds.size.width * fraction,
            bounds.center().y,
        )
    };
    let before = (light(&view, cx), undo_depth(&view, cx));
    cx.simulate_mouse_move(at(0.5), None, Default::default());
    cx.simulate_mouse_down(at(0.5), MouseButton::Left, Default::default());
    for fraction in [0.55, 0.65, 0.75] {
        cx.simulate_mouse_move(at(fraction), MouseButton::Left, Default::default());
        cx.simulate_mouse_move(at(fraction), MouseButton::Left, Default::default());
    }
    assert_eq!(
        (light(&view, cx), undo_depth(&view, cx)),
        before,
        "a candidate only"
    );
    cx.simulate_mouse_up(at(0.75), MouseButton::Left, Default::default());
    assert_eq!(
        undo_depth(&view, cx),
        before.1 + 1,
        "the whole drag is one entry"
    );
    assert_ne!(light(&view, cx)["radius"], before.0["radius"]);
}
