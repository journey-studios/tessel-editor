//! BL014/BL021: the directed-scene inspector, driven by real GPUI input.
//!
//! The slider tests press, move and release over the Kit slider of a scene property through the
//! window's input path (`simulate_mouse_*`, `simulate_keystrokes`, `simulate_event`), so they
//! cover the Kit slider events, the capture-phase press, the Escape handler and the
//! window-activation observer. They do NOT prove a physical mouse (BL043 stays open), and the
//! monitor is not exercised: the directed scene has no renderer yet (`scene_monitor.rs`).
//!
//! What the model shows and how it is derived is tested without a window in `scene_model.rs`;
//! the core's gesture rules in `crates/core`.
use super::input_tests::fixture;
use super::scene_gesture::SliderKey;
use super::scene_ui::InspectorMode;
use super::*;
// The Kit's test macro expands to `#[test]`; the glob import above would resolve it to the Kit macro again.
use core::prelude::v1::test;

use tessel_editor_core::directed_scene::Vec3;
use tessel_editor_core::scene_edit::{PropertyRef, PropertyValue, read_property};
use tessel_editor_core::scene_interaction::{GestureCommand, SelectMode};
use tessel_editor_core::scene_project::{SceneCommand, SceneSource};

const BUNDLE: &str = include_str!(
    "../../../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json"
);

const FIGURE: &str = "scene/scene-a/node/figure";
const BACKDROP: &str = "scene/scene-a/node/backdrop";
const ROOT: &str = "scene/scene-a/node/root";
const WIDTH: &str = "scene/scene-a/node/figure/width";
const ORIGINAL_WIDTH: f64 = 2.0;

fn run_scene(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    command: SceneCommand,
) -> Result<Value, CommandError> {
    view.update(cx, |view, cx| {
        view.execute(Command::Scene(command), Source::Cli, cx)
    })
}

fn attach(view: &Entity<Workstation>, cx: &mut VisualTestContext, id: &str) {
    run_scene(
        view,
        cx,
        SceneCommand::Attach {
            id: Some(id.into()),
            name: None,
            source: SceneSource::Text(BUNDLE.to_string()),
        },
    )
    .unwrap();
}

fn select(view: &Entity<Workstation>, cx: &mut VisualTestContext, path: &str) {
    run_scene(
        view,
        cx,
        SceneCommand::Select {
            scene: Some("street".into()),
            entities: vec![path.to_string()],
            mode: SelectMode::Replace,
        },
    )
    .unwrap();
    cx.run_until_parked();
}

/// The workstation with the fixture scene attached, the scene inspector showing, and `path`
/// selected, painted once so its sliders exist.
fn mounted<'a>(
    cx: &'a mut TestAppContext,
    path: &str,
) -> (Entity<Workstation>, &'a mut VisualTestContext) {
    let (view, cx) = fixture(cx);
    attach(&view, cx, "street");
    view.update(cx, |view, cx| view.scene_set_mode(InspectorMode::Scene, cx));
    select(&view, cx, path);
    (view, cx)
}

fn document_value(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    path: &str,
) -> PropertyValue {
    view.read_with(cx, |view, _| {
        let attached = view.state.scenes.get("street").unwrap();
        read_property(&attached.scene, &PropertyRef::parse(path).unwrap()).unwrap()
    })
}

fn document_width(view: &Entity<Workstation>, cx: &mut VisualTestContext, path: &str) -> f64 {
    match document_value(view, cx, path) {
        PropertyValue::Number(width) => width,
        other => panic!("width is a number, got {other:?}"),
    }
}

/// `scene.attach` is itself one undoable edit, so the fixture starts one entry deep.
const ATTACH_ENTRIES: u64 = 1;

/// Undo entries recorded by the test body, after the fixture's own attach.
fn undo_depth(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> u64 {
    view.read_with(cx, |view, _| {
        view.state.history.to_json()["undo_depth"].as_u64().unwrap() - ATTACH_ENTRIES
    })
}

fn gesture_open(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |view, _| {
        view.state.scene_interaction.gesture().is_some()
    })
}

fn width_key() -> SliderKey {
    SliderKey {
        scene: "street".into(),
        entity: FIGURE.into(),
        path: WIDTH.into(),
        index: 0,
    }
}

fn track_point(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    key: &SliderKey,
    fraction: f32,
) -> Point<Pixels> {
    let id = format!("control:{}", key.element_id());
    let bounds = view.read_with(cx, |view, _| view.geometry.get(&id).unwrap());
    point(
        bounds.left() + bounds.size.width * fraction,
        bounds.center().y,
    )
}

fn press(cx: &mut VisualTestContext, at: Point<Pixels>) {
    cx.simulate_mouse_move(at, None, Default::default());
    cx.simulate_mouse_down(at, MouseButton::Left, Default::default());
}

/// The first move of a press only starts the Kit drag; the second delivers the position.
fn drag_to(cx: &mut VisualTestContext, at: Point<Pixels>) {
    cx.simulate_mouse_move(at, MouseButton::Left, Default::default());
    cx.simulate_mouse_move(at, MouseButton::Left, Default::default());
}

fn release(cx: &mut VisualTestContext, at: Point<Pixels>) {
    cx.simulate_mouse_up(at, MouseButton::Left, Default::default());
}

fn node(view: &Entity<Workstation>, cx: &mut VisualTestContext, id: &str) -> Value {
    view.read_with(cx, |view, cx| view.tree_node(id, cx))
        .unwrap_or_else(|| panic!("no semantic node `{id}`"))
}

fn property_node(path: &str) -> String {
    format!("scene.property:{path}")
}

#[gpui_kit::test]
fn dragging_a_scene_slider_previews_live_and_records_one_undo_entry(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx, FIGURE);
    let key = width_key();
    let at = track_point(&view, cx, &key, 0.5);
    press(cx, at);
    for fraction in [0.55, 0.65, 0.75] {
        let at = track_point(&view, cx, &key, fraction);
        drag_to(cx, at);
    }

    // The candidate is on screen; the document and the history are untouched until the release.
    assert!(gesture_open(&view, cx));
    assert_eq!(document_width(&view, cx, WIDTH), ORIGINAL_WIDTH);
    assert_eq!(undo_depth(&view, cx), 0);
    let shown = view
        .read_with(cx, |view, _| view.scene_property_now(&key))
        .unwrap();
    let PropertyValue::Number(previewed) = shown else {
        panic!("width is a number");
    };
    assert!(
        previewed > ORIGINAL_WIDTH,
        "the candidate shows {previewed}"
    );
    let semantic = node(&view, cx, &property_node(WIDTH));
    assert_eq!(semantic["state"]["live"].as_f64(), Some(ORIGINAL_WIDTH));
    assert_eq!(semantic["state"]["candidate"].as_f64(), Some(previewed));

    let at = track_point(&view, cx, &key, 0.75);
    release(cx, at);
    assert!(!gesture_open(&view, cx));
    assert_eq!(undo_depth(&view, cx), 1, "the whole drag is one entry");
    assert_eq!(document_width(&view, cx, WIDTH), previewed);

    run_scene_undo(&view, cx);
    assert_eq!(document_width(&view, cx, WIDTH), ORIGINAL_WIDTH);
}

fn run_scene_undo(view: &Entity<Workstation>, cx: &mut VisualTestContext) {
    view.update(cx, |view, cx| view.execute(Command::Undo, Source::Cli, cx))
        .unwrap();
}

#[gpui_kit::test]
fn escape_cancels_a_scene_drag_and_the_rest_of_the_press_is_ignored(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx, FIGURE);
    let key = width_key();
    let at = track_point(&view, cx, &key, 0.5);
    press(cx, at);
    let at = track_point(&view, cx, &key, 0.8);
    drag_to(cx, at);
    assert!(gesture_open(&view, cx));

    cx.simulate_keystrokes("escape");
    assert!(!gesture_open(&view, cx));
    assert_eq!(document_width(&view, cx, WIDTH), ORIGINAL_WIDTH);

    let at = track_point(&view, cx, &key, 0.9);
    drag_to(cx, at);
    release(cx, at);
    assert_eq!(document_width(&view, cx, WIDTH), ORIGINAL_WIDTH);
    assert_eq!(undo_depth(&view, cx), 0);
    assert!(!gesture_open(&view, cx));
    view.read_with(cx, |view, _| {
        assert!(!view.scene_ui.gesture.is_tracking());
    });
}

#[gpui_kit::test]
fn keyboard_steps_are_one_gesture_per_hold(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx, FIGURE);
    let key = width_key();
    // A click focuses the slider wrapper, as for the clip sliders.
    let target = track_point(&view, cx, &key, 0.5);
    press(cx, target);
    release(cx, target);
    let after_click = document_width(&view, cx, WIDTH);
    assert_eq!(undo_depth(&view, cx), 1);

    let right = Keystroke::parse("right").unwrap();
    cx.simulate_keystrokes("right");
    assert!(gesture_open(&view, cx), "a key press begins a gesture");
    assert_eq!(undo_depth(&view, cx), 1);
    for _ in 0..2 {
        cx.simulate_event(KeyDownEvent {
            keystroke: right.clone(),
            is_held: true,
            prefer_character_input: false,
        });
    }
    assert!(gesture_open(&view, cx), "auto-repeat continues the gesture");
    cx.simulate_event(KeyUpEvent {
        keystroke: right.clone(),
    });
    assert!(!gesture_open(&view, cx));
    assert_eq!(undo_depth(&view, cx), 2, "the whole hold is one entry");
    assert!(document_width(&view, cx, WIDTH) > after_click);
}

#[gpui_kit::test]
fn losing_the_window_focus_commits_the_gesture_once(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx, FIGURE);
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    let key = width_key();
    let at = track_point(&view, cx, &key, 0.5);
    press(cx, at);
    let at = track_point(&view, cx, &key, 0.7);
    drag_to(cx, at);
    assert!(gesture_open(&view, cx));

    cx.deactivate_window();
    assert!(!gesture_open(&view, cx));
    assert_eq!(undo_depth(&view, cx), 1);
    assert!(document_width(&view, cx, WIDTH) > ORIGINAL_WIDTH);
}

#[gpui_kit::test]
fn changing_the_selection_mid_drag_never_redirects_the_edit(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx, FIGURE);
    let key = width_key();
    let backdrop_before = document_width(&view, cx, "scene/scene-a/node/backdrop/width");
    let at = track_point(&view, cx, &key, 0.5);
    press(cx, at);
    let at = track_point(&view, cx, &key, 0.7);
    drag_to(cx, at);
    let previewed = view
        .read_with(cx, |view, _| view.scene_property_now(&key))
        .unwrap();

    select(&view, cx, BACKDROP);
    cx.update(|window, _| window.activate_window());
    cx.deactivate_window();

    assert_eq!(
        document_value(&view, cx, WIDTH),
        previewed,
        "the gesture committed on the entity it began on"
    );
    assert_eq!(
        document_width(&view, cx, "scene/scene-a/node/backdrop/width"),
        backdrop_before
    );
    assert_eq!(undo_depth(&view, cx), 1);
    assert!(!gesture_open(&view, cx));
}

#[gpui_kit::test]
fn the_tree_selection_follows_the_keyboard_and_the_pointer(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx, FIGURE);
    // The Kit tree moves its own selection on arrow keys and clicks, without a command.
    view.update(cx, |view, cx| {
        view.scene_ui.tree.update(cx, |tree, cx| {
            let row = tree.index_of(&SharedString::from(BACKDROP));
            assert!(row.is_some(), "the backdrop is a visible row");
            tree.set_selected_index(row, cx);
        });
    });
    cx.run_until_parked();
    let selection = view.read_with(cx, |view, _| {
        view.scene_snapshot().unwrap().selection.clone()
    });
    assert_eq!(selection, vec![BACKDROP.to_string()]);
    // The panel now edits the backdrop.
    let semantic = node(&view, cx, "scene.panel");
    assert_eq!(
        semantic["state"]["target"]["path"],
        json!(BACKDROP),
        "{semantic}"
    );
}

#[gpui_kit::test]
fn reset_is_one_undo_entry_and_says_why_it_is_unavailable(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx, FIGURE);
    let position = "scene/scene-a/node/figure/position";
    let semantic = node(&view, cx, &property_node(position));
    assert_eq!(semantic["actions"][0]["enabled"], json!(true));
    run_invoke(&view, cx, &property_node(position), Some("reset")).unwrap();
    assert_eq!(undo_depth(&view, cx), 1);
    assert_eq!(
        document_value(&view, cx, position),
        PropertyValue::Vec3(Vec3::ZERO)
    );
    // Back at the default: nothing to reset, and the node says so.
    let semantic = node(&view, cx, &property_node(position));
    assert_eq!(semantic["actions"][0]["enabled"], json!(false));
    assert!(semantic["actions"][0]["disabled_reason"].is_string());

    // A property the catalog gives no default: refused with a typed error, no entry.
    select(&view, cx, "camera/cam-main");
    let fov = property_node("camera/cam-main/start/fov");
    let error = run_invoke(&view, cx, &fov, Some("reset")).unwrap_err();
    assert_eq!(error.code, "disabled_element");
    assert!(error.message.contains("padrão"), "{}", error.message);
    assert_eq!(undo_depth(&view, cx), 1);
}

fn run_invoke(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    target: &str,
    action: Option<&str>,
) -> Result<Value, CommandError> {
    view.update(cx, |view, cx| view.scene_invoke(target, action, cx))
        .expect("a scene node")
}

fn run_set(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    target: &str,
    field: &str,
    raw: Raw,
) -> Result<Value, CommandError> {
    view.update(cx, |view, cx| view.scene_set(target, field, &raw, cx))
        .expect("a scene node")
}

#[gpui_kit::test]
fn a_refused_value_is_a_typed_error_on_screen_and_leaves_nothing_behind(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx, FIGURE);
    let error = run_set(&view, cx, &property_node(WIDTH), "value", Raw::Number(-1.0)).unwrap_err();
    assert!(!error.code.is_empty());
    assert_eq!(document_width(&view, cx, WIDTH), ORIGINAL_WIDTH);
    assert_eq!(undo_depth(&view, cx), 0);
    assert!(
        !gesture_open(&view, cx),
        "the failed set does not leave a gesture open"
    );
    cx.run_until_parked();
    let semantic = node(&view, cx, "scene.panel");
    assert_eq!(
        semantic["state"]["notice"]["code"],
        json!(error.code),
        "{semantic}"
    );
}

#[gpui_kit::test]
fn the_semantic_set_is_the_same_edit_as_the_slider_and_one_undo_entry(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx, FIGURE);
    let outcome = run_set(&view, cx, &property_node(WIDTH), "value", Raw::Number(3.5)).unwrap();
    assert_eq!(outcome["changed"], json!(true), "{outcome}");
    assert_eq!(document_width(&view, cx, WIDTH), 3.5);
    assert_eq!(undo_depth(&view, cx), 1);
    run_scene_undo(&view, cx);
    assert_eq!(document_width(&view, cx, WIDTH), ORIGINAL_WIDTH);

    // A component of a vector moves alone.
    let position = "scene/scene-a/node/figure/position";
    run_set(&view, cx, &property_node(position), "y", Raw::Number(1.25)).unwrap();
    let PropertyValue::Vec3(moved) = document_value(&view, cx, position) else {
        panic!("position is a vector");
    };
    assert_eq!((moved.x, moved.y, moved.z), (0.5, 1.25, 0.0));
}

#[gpui_kit::test]
fn a_gesture_begun_through_the_socket_is_shown_as_a_candidate_and_can_be_cancelled(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mounted(cx, FIGURE);
    let begun = run_scene(
        &view,
        cx,
        SceneCommand::Gesture(GestureCommand::Begin {
            scene: "street".into(),
            targets: vec![FIGURE.into()],
            label: None,
        }),
    )
    .unwrap();
    let gesture = begun["gesture"].as_str().unwrap().to_string();
    run_scene(
        &view,
        cx,
        SceneCommand::Gesture(GestureCommand::Update {
            gesture,
            ops: vec![tessel_editor_core::scene_edit::EditOp::set(
                PropertyRef::parse(WIDTH).unwrap(),
                PropertyValue::Number(5.0),
            )],
        }),
    )
    .unwrap();
    cx.run_until_parked();

    let panel = node(&view, cx, "scene.panel");
    assert_eq!(
        panel["state"]["gesture"]["has_candidate"],
        json!(true),
        "{panel}"
    );
    let semantic = node(&view, cx, &property_node(WIDTH));
    assert_eq!(semantic["state"]["candidate"].as_f64(), Some(5.0));
    assert_eq!(semantic["state"]["live"].as_f64(), Some(ORIGINAL_WIDTH));
    assert_eq!(document_width(&view, cx, WIDTH), ORIGINAL_WIDTH);

    // While it is open, reset is unavailable and says why.
    assert_eq!(semantic["actions"][0]["enabled"], json!(false));

    run_invoke(&view, cx, "scene.panel", Some("cancel_gesture")).unwrap();
    assert!(!gesture_open(&view, cx));
    assert_eq!(undo_depth(&view, cx), 0);
    assert_eq!(document_width(&view, cx, WIDTH), ORIGINAL_WIDTH);
}

#[gpui_kit::test]
fn entities_are_inspectable_selectable_and_expandable_by_id(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx, FIGURE);
    let tree = node(&view, cx, "scene.tree");
    assert_eq!(tree["state"]["scene"], json!("street"));
    let figure = node(&view, cx, &format!("scene.entity:{FIGURE}"));
    assert_eq!(figure["state"]["selected"], json!(true));
    assert_eq!(figure["state"]["node_kind"], json!("imagePlane"));
    let backdrop = node(&view, cx, &format!("scene.entity:{BACKDROP}"));
    assert_eq!(backdrop["state"]["selected"], json!(false));

    run_invoke(
        &view,
        cx,
        &format!("scene.entity:{BACKDROP}"),
        Some("select"),
    )
    .unwrap();
    cx.run_until_parked();
    let backdrop = node(&view, cx, &format!("scene.entity:{BACKDROP}"));
    assert_eq!(backdrop["state"]["selected"], json!(true));

    let collapsed =
        run_invoke(&view, cx, &format!("scene.entity:{ROOT}"), Some("collapse")).unwrap();
    assert_eq!(collapsed["expanded"], json!(false));
    let ids: Vec<String> = view
        .read_with(cx, |view, cx| view.tree_nodes(cx))
        .iter()
        .filter_map(|node| node["id"].as_str().map(str::to_string))
        .collect();
    assert!(ids.contains(&format!("scene.entity:{ROOT}")));
    assert!(
        !ids.contains(&format!("scene.entity:{FIGURE}")),
        "a collapsed node hides its children"
    );
    let missing = run_invoke(
        &view,
        cx,
        "scene.entity:scene/scene-a/node/ghost",
        Some("select"),
    );
    assert!(missing.is_err());
}

#[gpui_kit::test]
fn a_group_property_is_disabled_with_its_reason_and_refuses_edits(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx, ROOT);
    let width = property_node("scene/scene-a/node/root/width");
    let semantic = node(&view, cx, &width);
    assert_eq!(semantic["enabled"], json!(false));
    let reason = semantic["disabled_reason"].as_str().unwrap();
    assert!(reason.contains("imagePlane"), "{reason}");
    let error = run_set(&view, cx, &width, "value", Raw::Number(3.0)).unwrap_err();
    assert_eq!(error.code, "disabled_element");
    assert_eq!(undo_depth(&view, cx), 0);
}

#[gpui_kit::test]
fn the_monitor_is_reported_unavailable_and_nothing_pretends_otherwise(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx, FIGURE);
    let monitor = node(&view, cx, "monitor.scene");
    assert_eq!(monitor["enabled"], json!(false));
    assert!(
        monitor["disabled_reason"]
            .as_str()
            .unwrap()
            .contains("monitor")
    );
    assert_eq!(monitor["state"]["available"], json!(false));
    let error = run_invoke(&view, cx, "monitor.scene", None).unwrap_err();
    assert_eq!(error.code, "disabled_element");
    // The monitor region lists the seam; the scene is still editable without it.
    let region = node(&view, cx, "region:monitor");
    assert!(
        region["children"]
            .as_array()
            .unwrap()
            .contains(&json!("monitor.scene"))
    );
    run_set(&view, cx, &property_node(WIDTH), "value", Raw::Number(4.0)).unwrap();
    assert_eq!(document_width(&view, cx, WIDTH), 4.0);
}

#[gpui_kit::test]
fn without_a_scene_the_panel_says_how_to_attach_one(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    view.update(cx, |view, cx| view.scene_set_mode(InspectorMode::Scene, cx));
    cx.run_until_parked();
    let panel = node(&view, cx, "scene.panel");
    assert_eq!(panel["enabled"], json!(false));
    assert!(
        panel["disabled_reason"]
            .as_str()
            .unwrap()
            .contains("scene.attach")
    );
    assert_eq!(panel["state"]["available"], json!(false));
    // The clip inspector stays the default and keeps working.
    let mode = node(&view, cx, "inspector.mode");
    assert_eq!(mode["value"], json!("scene"));
    run_set(
        &view,
        cx,
        "inspector.mode",
        "value",
        Raw::Text("clip".into()),
    )
    .unwrap();
    assert_eq!(node(&view, cx, "inspector.mode")["value"], json!("clip"));
}

#[gpui_kit::test]
fn the_picker_chooses_which_attached_scene_the_panel_edits(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx, FIGURE);
    attach(&view, cx, "alley");
    cx.run_until_parked();
    assert_eq!(
        node(&view, cx, "scene.panel")["state"]["scene"],
        json!("street")
    );
    run_set(
        &view,
        cx,
        "scene.picker",
        "value",
        Raw::Text("alley".into()),
    )
    .unwrap();
    cx.run_until_parked();
    let panel = node(&view, cx, "scene.panel");
    assert_eq!(panel["state"]["scene"], json!("alley"));
    assert_eq!(panel["state"]["target"]["kind"], json!("none"));
    assert!(
        run_set(
            &view,
            cx,
            "scene.picker",
            "value",
            Raw::Text("ghost".into())
        )
        .is_err()
    );
}

#[gpui_kit::test]
fn the_limits_a_client_reads_are_the_limits_the_slider_has(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx, FIGURE);
    let semantic = node(&view, cx, &property_node(WIDTH));
    let field = &semantic["fields"][0];
    let window = view.read_with(cx, |view, _| view.scene_ui.sliders[&width_key()].scrub);
    assert_eq!(field["min"].as_f64(), Some(window.min));
    assert_eq!(field["max"].as_f64(), Some(window.max));
    assert_eq!(field["step"].as_f64(), Some(window.step));

    // After an edit moves the value, the slider keeps (or rebuilds) its window and the node
    // follows it: they never disagree.
    run_set(&view, cx, &property_node(WIDTH), "value", Raw::Number(9.0)).unwrap();
    cx.run_until_parked();
    let semantic = node(&view, cx, &property_node(WIDTH));
    let window = view.read_with(cx, |view, _| view.scene_ui.sliders[&width_key()].scrub);
    assert_eq!(semantic["fields"][0]["max"].as_f64(), Some(window.max));
    assert!(window.max >= 9.0, "{window:?}");
}
