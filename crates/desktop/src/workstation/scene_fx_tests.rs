//! F016: the "Efeitos" section of the scene inspector, driven through its semantic nodes and by
//! real GPUI pointer input over the Kit slider. The rules (catalog, ranges, overlap, envelope,
//! atomic refusal) are the core's and are tested in `crates/core/tests/scene_fx.rs`; these tests
//! prove the window reaches them: one command and one undo entry per gesture, the typed refusal on
//! screen, and nothing left behind by a refused or cancelled gesture.
//!
//! The gesture is the core's (BL082.6/.16): these tests drive the adapter (`fx_changed`,
//! `fx_released`, `fx_key_down`...) and the real Kit slider, and read the candidate, the budget
//! warning and the history through the semantic nodes and the core state.
//!
//! Not proved here: pixels in the monitor (the native WebView does not exist in a GPUI test
//! window; see `crates/render/tests/scene_fx.rs` and `scene_fx_export.rs` for the runtime and the
//! export) and a physical mouse.
use super::input_tests::fixture;
use super::scene_ui::InspectorMode;
use super::*;
// The Kit's test macro expands to `#[test]`; the glob import above would resolve it to the Kit macro again.
use core::prelude::v1::test;

use tessel_editor_core::directed_scene::fx::FxValue;
use tessel_editor_core::scene_project::{SceneCommand, SceneSource};

const BUNDLE: &str = include_str!(
    "../../../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json"
);

/// `scene.attach` is itself one undoable edit, so the fixture starts one entry deep.
const ATTACH_ENTRIES: u64 = 1;

pub(super) fn mounted(cx: &mut TestAppContext) -> (Entity<Workstation>, &mut VisualTestContext) {
    let (view, cx) = fixture(cx);
    view.update(cx, |view, cx| {
        view.execute(
            Command::Scene(SceneCommand::Attach {
                id: Some("street".into()),
                name: None,
                source: SceneSource::Text(BUNDLE.to_string()),
            }),
            Source::Cli,
            cx,
        )
        .unwrap();
        view.scene_set_mode(InspectorMode::Scene, cx);
        // Where an insertion lands is the playhead's: start every test at the head of the edit.
        view.state.timeline.set_playhead(0);
    });
    cx.run_until_parked();
    (view, cx)
}

pub(super) fn undo_depth(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> u64 {
    view.read_with(cx, |view, _| {
        view.state.history.to_json()["undo_depth"].as_u64().unwrap() - ATTACH_ENTRIES
    })
}

pub(super) fn invoke(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    target: &str,
    action: &str,
) -> Result<Value, CommandError> {
    let result = view
        .update(cx, |view, cx| view.scene_invoke(target, Some(action), cx))
        .expect("a scene node");
    cx.run_until_parked();
    result
}

pub(super) fn set(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    target: &str,
    field: &str,
    raw: Raw,
) -> Result<Value, CommandError> {
    let result = view
        .update(cx, |view, cx| view.scene_set(target, field, &raw, cx))
        .expect("a scene node");
    cx.run_until_parked();
    result
}

pub(super) fn node(view: &Entity<Workstation>, cx: &mut VisualTestContext, id: &str) -> Value {
    view.read_with(cx, |view, cx| view.tree_node(id, cx))
        .unwrap_or_else(|| panic!("no semantic node `{id}`"))
}

pub(super) fn field_value(node: &Value, field: &str) -> Value {
    node["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["field"] == field)
        .unwrap_or_else(|| panic!("no field `{field}` in {node}"))["value"]
        .clone()
}

pub(super) fn clips(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |view, _| {
        view.state
            .scenes
            .get("street")
            .unwrap()
            .scene
            .bundle()
            .fx
            .clip_count()
    })
}

fn intensity(view: &Entity<Workstation>, cx: &mut VisualTestContext, clip: &str) -> f64 {
    view.read_with(cx, |view, _| {
        let attached = view.state.scenes.get("street").unwrap();
        let clip = attached
            .scene
            .bundle()
            .fx
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .find(|candidate| candidate.id == clip)
            .unwrap();
        match clip.param("intensity").unwrap() {
            FxValue::Number(value) => value,
            other => panic!("intensity is a number, got {other:?}"),
        }
    })
}

fn add_vignette(view: &Entity<Workstation>, cx: &mut VisualTestContext) {
    invoke(view, cx, "scene.fx", "add:vignette").unwrap();
}

const CLIP: &str = "scene.fx.clip:fx-1";

#[gpui_kit::test]
fn the_section_adds_a_vignette_as_one_undo_entry_and_the_nodes_describe_it(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mounted(cx);
    let section = node(&view, cx, "scene.fx");
    assert_eq!(section["state"]["clip_count"], json!(0));
    assert_eq!(
        section["state"]["effects"],
        json!(["vignette", "fade", "filmGrain"])
    );
    assert_eq!(section["actions"][0]["action"], json!("add:vignette"));
    assert_eq!(section["actions"][0]["enabled"], json!(true));
    assert_eq!(undo_depth(&view, cx), 0);

    let added = invoke(&view, cx, "scene.fx", "add:vignette").unwrap();
    assert_eq!(added["changed"], json!(true));
    assert_eq!(clips(&view, cx), 1);
    assert_eq!(undo_depth(&view, cx), 1, "one gesture, one entry");

    let clip = node(&view, cx, CLIP);
    assert_eq!(clip["kind"], json!("fx_clip"));
    assert_eq!(clip["state"]["effect"], json!("vignette"));
    assert_eq!(clip["state"]["start_frame"], json!(0));
    assert_eq!(field_value(&clip, "intensity"), json!(0.7));
    assert_eq!(field_value(&clip, "center_x"), json!(0.5));
    assert_eq!(field_value(&clip, "hidden"), json!(false));
    let panel = node(&view, cx, "scene.panel");
    assert!(
        panel["children"]
            .as_array()
            .unwrap()
            .contains(&json!("scene.fx"))
    );
    assert_eq!(
        node(&view, cx, "scene.fx")["children"],
        json!(["scene.fx.clip:fx-1", "scene.fx.budget"])
    );

    // The core resolves a pass for the frames the clip covers: the monitor and export draw it.
    let passes = view.read_with(cx, |view, _| {
        view.state
            .scenes
            .get("street")
            .unwrap()
            .scene
            .bundle()
            .fx_passes(10)
            .len()
    });
    assert_eq!(passes, 1);

    // Undo and redo are the shared history.
    view.update(cx, |view, cx| view.execute(Command::Undo, Source::Cli, cx))
        .unwrap();
    assert_eq!(clips(&view, cx), 0);
    view.update(cx, |view, cx| view.execute(Command::Redo, Source::Cli, cx))
        .unwrap();
    assert_eq!(clips(&view, cx), 1);
}

#[gpui_kit::test]
fn setting_a_control_through_its_node_is_one_entry_and_a_component_keeps_its_siblings(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mounted(cx);
    add_vignette(&view, cx);
    let before = undo_depth(&view, cx);

    set(&view, cx, CLIP, "intensity", Raw::Number(0.25)).unwrap();
    assert_eq!(intensity(&view, cx, "fx-1"), 0.25);
    assert_eq!(undo_depth(&view, cx), before + 1);

    set(&view, cx, CLIP, "center_x", Raw::Number(0.2)).unwrap();
    let clip = node(&view, cx, CLIP);
    assert_eq!(field_value(&clip, "center_x"), json!(0.2));
    assert_eq!(
        field_value(&clip, "center_y"),
        json!(0.5),
        "the sibling stays"
    );

    set(&view, cx, CLIP, "color_b", Raw::Number(1.0)).unwrap();
    let clip = node(&view, cx, CLIP);
    assert_eq!(field_value(&clip, "color_b"), json!(1.0));
    assert_eq!(field_value(&clip, "color_r"), json!(0.0));

    set(&view, cx, CLIP, "fade_in_frames", Raw::Number(12.0)).unwrap();
    assert_eq!(node(&view, cx, CLIP)["state"]["fade_in_frames"], json!(12));

    // Writing what is already there is a no-op: no entry.
    let depth = undo_depth(&view, cx);
    let noop = set(&view, cx, CLIP, "intensity", Raw::Number(0.25)).unwrap();
    assert_eq!(noop["changed"], json!(false));
    assert_eq!(undo_depth(&view, cx), depth);
}

#[gpui_kit::test]
fn a_refused_edit_is_a_typed_error_on_screen_and_leaves_nothing_behind(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    add_vignette(&view, cx);
    let depth = undo_depth(&view, cx);

    let error = set(&view, cx, CLIP, "intensity", Raw::Number(2.0)).unwrap_err();
    assert_eq!(error.code, "param_out_of_range", "{error:?}");
    assert_eq!(intensity(&view, cx, "fx-1"), 0.7);
    assert_eq!(undo_depth(&view, cx), depth);
    let panel = node(&view, cx, "scene.panel");
    assert_eq!(
        panel["state"]["notice"]["code"],
        json!("param_out_of_range"),
        "{panel}"
    );

    // A fractional frame, an unknown field, an unknown clip and a text for a number.
    let error = set(&view, cx, CLIP, "start_frame", Raw::Number(1.5)).unwrap_err();
    assert_eq!(error.code, "invalid_value", "{error:?}");
    let error = set(&view, cx, CLIP, "nope", Raw::Number(1.0)).unwrap_err();
    assert_eq!(error.code, "unknown_field");
    let error = set(
        &view,
        cx,
        "scene.fx.clip:ghost",
        "intensity",
        Raw::Number(1.0),
    )
    .unwrap_err();
    assert_eq!(error.code, "unknown_element");
    let error = set(&view, cx, CLIP, "intensity", Raw::Text("abc".into())).unwrap_err();
    assert_eq!(error.code, "invalid_value");
    // An overlapping clip on the same track is refused by the layer's rules.
    let error = set(&view, cx, CLIP, "duration_frames", Raw::Number(500.0)).unwrap_err();
    assert!(!error.code.is_empty());
    assert_eq!(undo_depth(&view, cx), depth);
    assert_eq!(clips(&view, cx), 1);

    // The next accepted edit clears the notice.
    set(&view, cx, CLIP, "intensity", Raw::Number(0.5)).unwrap();
    assert_eq!(
        node(&view, cx, "scene.panel")["state"]["notice"],
        Value::Null
    );
}

#[gpui_kit::test]
fn hide_toggle_and_remove_go_through_the_same_command_and_undo_restores_them(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mounted(cx);
    add_vignette(&view, cx);
    set(&view, cx, CLIP, "hidden", Raw::Bool(true)).unwrap();
    assert_eq!(field_value(&node(&view, cx, CLIP), "hidden"), json!(true));
    view.read_with(cx, |view, _| {
        let bundle = view.state.scenes.get("street").unwrap().scene.bundle();
        assert!(
            bundle.fx_passes(10).is_empty(),
            "a hidden clip draws nothing"
        );
    });
    invoke(&view, cx, CLIP, "toggle_hidden").unwrap();
    assert_eq!(field_value(&node(&view, cx, CLIP), "hidden"), json!(false));

    let depth = undo_depth(&view, cx);
    invoke(&view, cx, CLIP, "remove").unwrap();
    assert_eq!(clips(&view, cx), 0);
    assert_eq!(undo_depth(&view, cx), depth + 1);
    view.update(cx, |view, cx| view.execute(Command::Undo, Source::Cli, cx))
        .unwrap();
    assert_eq!(clips(&view, cx), 1);

    let error = invoke(&view, cx, CLIP, "explode").unwrap_err();
    assert_eq!(error.code, "unknown_action");
    let error = invoke(&view, cx, "scene.fx", "add:sparkle").unwrap_err();
    assert!(
        !error.code.is_empty(),
        "an effect outside the catalog is refused"
    );
    assert_eq!(clips(&view, cx), 1);
}

fn slider_point(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    control: &str,
    fraction: f32,
) -> Point<Pixels> {
    let id = format!("control:fx:fx-1:{control}");
    let bounds = view.read_with(cx, |view, _| {
        view.geometry
            .get(&id)
            .unwrap_or_else(|| panic!("no probe `{id}`"))
    });
    point(
        bounds.left() + bounds.size.width * fraction,
        bounds.center().y,
    )
}

#[gpui_kit::test]
fn dragging_the_intensity_slider_commits_once_on_release(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    add_vignette(&view, cx);
    let depth = undo_depth(&view, cx);

    let at = slider_point(&view, cx, "intensity", 0.5);
    cx.simulate_mouse_move(at, None, Default::default());
    cx.simulate_mouse_down(at, MouseButton::Left, Default::default());
    for fraction in [0.3, 0.2, 0.1] {
        let at = slider_point(&view, cx, "intensity", fraction);
        cx.simulate_mouse_move(at, MouseButton::Left, Default::default());
        cx.simulate_mouse_move(at, MouseButton::Left, Default::default());
    }
    // During the drag the document and the history do not move; the core holds the candidate and
    // the section shows it.
    assert_eq!(intensity(&view, cx, "fx-1"), 0.7);
    assert_eq!(undo_depth(&view, cx), depth);
    assert!(driving(&view, cx));
    assert!(gesture_open(&view, cx));
    let shown = field_value(&node(&view, cx, CLIP), "intensity")
        .as_f64()
        .unwrap();
    assert!(shown < 0.5, "the section follows the candidate: {shown}");
    assert_eq!(
        node(&view, cx, "scene.fx")["state"]["source"],
        json!("gesture")
    );

    let end = slider_point(&view, cx, "intensity", 0.1);
    cx.simulate_mouse_up(end, MouseButton::Left, Default::default());
    cx.run_until_parked();
    let committed = intensity(&view, cx, "fx-1");
    assert!(
        (committed - 0.1).abs() < 0.08,
        "the thumb ended near 0.1, the document says {committed}"
    );
    assert_eq!(
        undo_depth(&view, cx),
        depth + 1,
        "the whole drag is one entry"
    );
    assert!(!driving(&view, cx));
    assert!(!gesture_open(&view, cx));
    assert_eq!(
        node(&view, cx, "scene.fx")["state"]["source"],
        json!("document")
    );
}

#[gpui_kit::test]
fn escape_drops_a_drag_and_the_slider_goes_back_to_the_document(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    add_vignette(&view, cx);
    let depth = undo_depth(&view, cx);

    let at = slider_point(&view, cx, "intensity", 0.5);
    cx.simulate_mouse_move(at, None, Default::default());
    cx.simulate_mouse_down(at, MouseButton::Left, Default::default());
    let moved = slider_point(&view, cx, "intensity", 0.2);
    cx.simulate_mouse_move(moved, MouseButton::Left, Default::default());
    cx.simulate_mouse_move(moved, MouseButton::Left, Default::default());
    assert!(gesture_open(&view, cx));

    cx.simulate_keystrokes("escape");
    assert!(
        !gesture_open(&view, cx),
        "Escape cancelled the gesture in the core"
    );
    cx.simulate_mouse_up(moved, MouseButton::Left, Default::default());
    cx.run_until_parked();
    assert_eq!(intensity(&view, cx, "fx-1"), 0.7, "nothing was committed");
    assert_eq!(undo_depth(&view, cx), depth);
}

pub(super) fn center_of(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    id: &str,
) -> Point<Pixels> {
    view.read_with(cx, |view, _| {
        view.geometry
            .get(id)
            .unwrap_or_else(|| panic!("no probe `{id}`"))
            .center()
    })
}

#[gpui_kit::test]
fn the_add_and_remove_buttons_run_the_shared_handlers_when_clicked(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    let depth = undo_depth(&view, cx);
    let add = center_of(&view, cx, "control:fx-add:vignette");
    cx.simulate_click(add, Default::default());
    cx.run_until_parked();
    assert_eq!(clips(&view, cx), 1);
    assert_eq!(undo_depth(&view, cx), depth + 1);

    // A second click adds a second clip on a new track: the first one covers the same frames.
    cx.simulate_click(add, Default::default());
    cx.run_until_parked();
    assert_eq!(clips(&view, cx), 2);
    assert_eq!(
        node(&view, cx, "scene.fx.clip:fx-2")["state"]["track"],
        json!("fx-track-2")
    );

    // The first clip's button is in the visible part of the scrolling panel.
    let remove = center_of(&view, cx, "control:fx-remove:fx-1");
    cx.simulate_click(remove, Default::default());
    cx.run_until_parked();
    assert_eq!(clips(&view, cx), 1);
    assert_eq!(
        node(&view, cx, "scene.fx")["children"],
        json!(["scene.fx.clip:fx-2", "scene.fx.budget"])
    );
    assert_eq!(undo_depth(&view, cx), depth + 3);
}

// ---- review R1: a gesture belongs to the scene it began in (A4) and a slider writes a step (A6) ------------

use super::scene_fx::{FxKey, controls_of};

fn attach_yard(view: &Entity<Workstation>, cx: &mut VisualTestContext) {
    view.update(cx, |view, cx| {
        view.execute(
            Command::Scene(SceneCommand::Attach {
                id: Some("yard".into()),
                name: None,
                source: SceneSource::Text(BUNDLE.to_string()),
            }),
            Source::Cli,
            cx,
        )
        .unwrap();
    });
    cx.run_until_parked();
}

fn choose(view: &Entity<Workstation>, cx: &mut VisualTestContext, scene: &str) {
    view.update(cx, |view, cx| view.scene_choose(scene, cx))
        .expect("an attached scene");
    cx.run_until_parked();
}

/// Every entry of the history, the attaches included: a test that compares two moments needs no offset.
fn total_depth(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> u64 {
    view.read_with(cx, |view, _| {
        view.state.history.to_json()["undo_depth"].as_u64().unwrap()
    })
}

fn intensity_in(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    scene: &str,
    clip: &str,
) -> f64 {
    view.read_with(cx, |view, _| {
        let attached = view.state.scenes.get(scene).unwrap();
        let clip = attached
            .scene
            .bundle()
            .fx
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .find(|candidate| candidate.id == clip)
            .unwrap_or_else(|| panic!("no clip `{clip}` in `{scene}`"));
        match clip.param("intensity").unwrap() {
            FxValue::Number(value) => value,
            other => panic!("intensity is a number, got {other:?}"),
        }
    })
}

fn key(scene: &str, clip: &str, control: &str) -> FxKey {
    FxKey {
        scene: scene.to_string(),
        clip: clip.to_string(),
        control: control.to_string(),
    }
}

/// One value of the control reaches the adapter, as a slider `Change` or a key step would.
fn change(view: &Entity<Workstation>, cx: &mut VisualTestContext, key: FxKey, value: f64) {
    view.update(cx, |view, cx| view.fx_changed(&key, value, cx));
    cx.run_until_parked();
}

fn release(view: &Entity<Workstation>, cx: &mut VisualTestContext, key: FxKey) {
    view.update(cx, |view, cx| view.fx_released(&key, cx));
    cx.run_until_parked();
}

/// The adapter is following a control.
fn driving(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |view, _| view.scene_ui.fx.is_tracking())
}

/// The core holds an effect gesture.
fn gesture_open(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |view, _| {
        view.state
            .scene_interaction
            .gesture()
            .is_some_and(|gesture| gesture.fx_clip().is_some())
    })
}

fn blur(view: &Entity<Workstation>, cx: &mut VisualTestContext) {
    view.update(cx, |view, cx| view.fx_focus_lost(cx));
    cx.run_until_parked();
}

fn panel_notice(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Value {
    node(view, cx, "scene.panel")["state"]["notice"].clone()
}

/// Both scenes carry an `fx-1` with intensity 0.7; the panel shows `street`.
fn two_scenes(cx: &mut TestAppContext) -> (Entity<Workstation>, &mut VisualTestContext) {
    let (view, cx) = mounted(cx);
    choose(&view, cx, "street");
    add_vignette(&view, cx);
    attach_yard(&view, cx);
    choose(&view, cx, "yard");
    add_vignette(&view, cx);
    choose(&view, cx, "street");
    assert_eq!(intensity_in(&view, cx, "street", "fx-1"), 0.7);
    assert_eq!(intensity_in(&view, cx, "yard", "fx-1"), 0.7);
    (view, cx)
}

#[gpui_kit::test]
fn a_gesture_does_not_follow_the_panel_to_another_scene(cx: &mut TestAppContext) {
    let (view, cx) = two_scenes(cx);
    let depth = total_depth(&view, cx);

    change(&view, cx, key("street", "fx-1", "intensity"), 0.2);
    assert!(gesture_open(&view, cx));
    // The same scene again is not a move: the drag is still the author's.
    choose(&view, cx, "street");
    assert!(
        gesture_open(&view, cx),
        "choosing the scene already shown moves nothing"
    );

    choose(&view, cx, "yard");
    assert!(
        !gesture_open(&view, cx) && !driving(&view, cx),
        "the gesture of `street` was cancelled in the core with the panel's move"
    );
    choose(&view, cx, "street");
    blur(&view, cx);
    assert_eq!(intensity_in(&view, cx, "street", "fx-1"), 0.7);
    assert_eq!(intensity_in(&view, cx, "yard", "fx-1"), 0.7);
    assert_eq!(
        total_depth(&view, cx),
        depth,
        "no edit came out of a dropped gesture"
    );
}

#[gpui_kit::test]
fn a_gesture_is_refused_for_a_scene_the_panel_no_longer_shows_even_if_a_clip_of_that_id_exists_there(
    cx: &mut TestAppContext,
) {
    let (view, cx) = two_scenes(cx);
    let depth = total_depth(&view, cx);

    // The panel moves without telling the section (the begin's own guard is what is under test).
    view.update(cx, |view, _| view.scene_ui.set_chosen_for_test("yard"));
    change(&view, cx, key("street", "fx-1", "intensity"), 0.2);
    blur(&view, cx);

    assert_eq!(
        intensity_in(&view, cx, "yard", "fx-1"),
        0.7,
        "`yard` has an `fx-1` and must not be edited"
    );
    assert_eq!(intensity_in(&view, cx, "street", "fx-1"), 0.7);
    assert_eq!(total_depth(&view, cx), depth);
    assert!(!gesture_open(&view, cx));
    assert_eq!(
        panel_notice(&view, cx)["code"],
        json!("stale_gesture"),
        "{}",
        node(&view, cx, "scene.panel")
    );
}

#[gpui_kit::test]
fn a_gesture_on_a_clip_that_is_gone_shows_the_refusal_instead_of_swallowing_it(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mounted(cx);
    add_vignette(&view, cx);
    invoke(&view, cx, CLIP, "remove").unwrap();
    assert_eq!(clips(&view, cx), 0);
    let depth = undo_depth(&view, cx);
    assert_eq!(panel_notice(&view, cx), Value::Null);

    // A slider that outlived its clip (the clip was removed by the CLI during the interaction).
    change(&view, cx, key("street", "fx-1", "intensity"), 0.2);
    blur(&view, cx);

    assert!(!gesture_open(&view, cx));
    assert_eq!(clips(&view, cx), 0, "nothing was resurrected");
    assert_eq!(undo_depth(&view, cx), depth);
    assert_eq!(
        panel_notice(&view, cx)["code"],
        json!("unknown_element"),
        "{}",
        node(&view, cx, "scene.panel")
    );
}

#[gpui_kit::test]
fn a_real_drag_cut_by_a_scene_change_edits_nothing_on_release(cx: &mut TestAppContext) {
    let (view, cx) = two_scenes(cx);
    let depth = total_depth(&view, cx);

    let at = slider_point(&view, cx, "intensity", 0.5);
    cx.simulate_mouse_move(at, None, Default::default());
    cx.simulate_mouse_down(at, MouseButton::Left, Default::default());
    let moved = slider_point(&view, cx, "intensity", 0.2);
    cx.simulate_mouse_move(moved, MouseButton::Left, Default::default());
    cx.simulate_mouse_move(moved, MouseButton::Left, Default::default());
    assert!(gesture_open(&view, cx), "the drag is in `street`");

    choose(&view, cx, "yard");
    assert!(!gesture_open(&view, cx));
    cx.simulate_mouse_up(moved, MouseButton::Left, Default::default());
    cx.run_until_parked();

    assert_eq!(intensity_in(&view, cx, "street", "fx-1"), 0.7);
    assert_eq!(
        intensity_in(&view, cx, "yard", "fx-1"),
        0.7,
        "the other scene's `fx-1` took nothing"
    );
    assert_eq!(total_depth(&view, cx), depth);
}

/// The control `id` of the first clip of the scene the panel shows, as the slider is built.
fn control_value_after(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    id: &str,
    raw: f64,
) -> f64 {
    view.read_with(cx, |view, _| {
        let fx = view.fx_view().expect("a scene with effects");
        let found = fx.clip("fx-1").expect("fx-1");
        controls_of(&found.clip, fx.edit_frames)
            .iter()
            .find(|control| control.id == id)
            .unwrap_or_else(|| panic!("no control `{id}`"))
            .quantize(raw)
    })
}

#[gpui_kit::test]
fn a_slider_value_is_put_on_the_step_inside_the_range(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    add_vignette(&view, cx);
    // The Kit slider speaks f32: 0.4 reaches the section as 0.4000000059604645.
    let noisy = f64::from(0.4_f32);
    assert_ne!(noisy, 0.4, "the premise: f32 noise exists");
    let table = [
        ("intensity", noisy, 0.4),
        ("intensity", f64::from(0.3_f32), 0.3),
        ("intensity", 0.7, 0.7),
        ("intensity", 1.7, 1.0),
        ("intensity", -0.3, 0.0),
        ("radius", 1.2000000476837158, 1.2),
        ("radius", 9.0, 1.5),
        // The grid is anchored at the minimum (softness starts at 0.05).
        ("softness", f64::from(0.35_f32), 0.35),
        ("softness", 0.0, 0.05),
        // Whole-frame fields are integers.
        ("start_frame", 7.6, 8.0),
        ("start_frame", -3.0, 0.0),
    ];
    for (id, raw, want) in table {
        assert_eq!(
            control_value_after(&view, cx, id, raw),
            want,
            "{id} <- {raw}"
        );
    }
    assert!(
        control_value_after(&view, cx, "intensity", f64::NAN).is_nan(),
        "garbage is passed on to be refused, not hidden"
    );
}

#[gpui_kit::test]
fn a_released_slider_stores_the_step_value_clamped_and_a_resting_thumb_writes_nothing(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mounted(cx);
    add_vignette(&view, cx);
    let depth = undo_depth(&view, cx);

    // f32 noise on the value the document already has: nothing to write.
    change(
        &view,
        cx,
        key("street", "fx-1", "intensity"),
        f64::from(0.7_f32),
    );
    blur(&view, cx);
    // The release that follows a focus loss ends the press, so the next one is a new gesture.
    release(&view, cx, key("street", "fx-1", "intensity"));
    assert_eq!(undo_depth(&view, cx), depth, "0.7 -> 0.7 is no edit");
    assert_eq!(panel_notice(&view, cx), Value::Null);
    assert_eq!(intensity(&view, cx, "fx-1"), 0.7);

    // The value stored is the step's, not the f32's.
    change(
        &view,
        cx,
        key("street", "fx-1", "intensity"),
        f64::from(0.4_f32),
    );
    blur(&view, cx);
    // The release that follows a focus loss ends the press, so the next one is a new gesture.
    release(&view, cx, key("street", "fx-1", "intensity"));
    assert_eq!(intensity(&view, cx, "fx-1"), 0.4);
    assert_eq!(undo_depth(&view, cx), depth + 1);

    // A thumb past the end is the range's end, not a refusal.
    change(&view, cx, key("street", "fx-1", "intensity"), 1.7);
    blur(&view, cx);
    // The release that follows a focus loss ends the press, so the next one is a new gesture.
    release(&view, cx, key("street", "fx-1", "intensity"));
    assert_eq!(intensity(&view, cx, "fx-1"), 1.0);
    assert_eq!(undo_depth(&view, cx), depth + 2);
    assert_eq!(panel_notice(&view, cx), Value::Null);

    // The pointer path ends on the grid as well.
    let at = slider_point(&view, cx, "intensity", 0.5);
    cx.simulate_mouse_move(at, None, Default::default());
    cx.simulate_mouse_down(at, MouseButton::Left, Default::default());
    let end = slider_point(&view, cx, "intensity", 0.37);
    cx.simulate_mouse_move(end, MouseButton::Left, Default::default());
    cx.simulate_mouse_move(end, MouseButton::Left, Default::default());
    cx.simulate_mouse_up(end, MouseButton::Left, Default::default());
    cx.run_until_parked();
    let dragged = intensity(&view, cx, "fx-1");
    assert_ne!(dragged, 1.0, "the drag moved the value");
    assert_eq!(
        dragged,
        (dragged * 100.0).round() / 100.0,
        "{dragged} is not a two-decimal step value"
    );
}

// ---- BL082.6/.16: the gesture is the core's, the keyboard is shared, the warning is visible ----------------

pub(super) fn clip_state(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    clip: &str,
) -> Value {
    node(view, cx, &format!("scene.fx.clip:{clip}"))["state"].clone()
}

#[gpui_kit::test]
fn the_preview_is_the_cores_candidate_and_the_commit_is_one_entry_with_undo_and_redo(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mounted(cx);
    add_vignette(&view, cx);
    let depth = undo_depth(&view, cx);
    let revision = view.read_with(cx, |view, _| view.state.project.revision);
    let intensity_key = key("street", "fx-1", "intensity");

    for value in [0.6, 0.5, 0.2] {
        change(&view, cx, intensity_key.clone(), value);
    }
    // Before the commit: the candidate is what the panel, the plan the monitor draws and the CLI
    // read; the document, the history and the revision did not move.
    assert_eq!(intensity(&view, cx, "fx-1"), 0.7);
    assert_eq!(undo_depth(&view, cx), depth);
    assert_eq!(
        view.read_with(cx, |view, _| view.state.project.revision),
        revision
    );
    assert_eq!(field_value(&node(&view, cx, CLIP), "intensity"), json!(0.2));
    let gesture = view.read_with(cx, |view, _| {
        view.state
            .scene_interaction
            .gesture()
            .unwrap()
            .id()
            .to_owned()
    });
    let plan = view.update(cx, |view, cx| {
        view.scene_execute(
            SceneCommand::Plan {
                scene: "street".into(),
                at: tessel_editor_core::scene_interaction::FrameAt::Frame(10),
                partial: false,
                gesture: Some(gesture),
            },
            Source::Cli,
            cx,
        )
    });
    assert_eq!(plan.unwrap()["plan"]["fx"][0]["amount"], json!(0.2));
    let state = node(&view, cx, "scene.fx")["state"].clone();
    assert_eq!(state["gesture"]["would_change"], json!(true));
    assert_eq!(state["gesture"]["clip"], json!("fx-1"));

    release(&view, cx, intensity_key);
    assert_eq!(intensity(&view, cx, "fx-1"), 0.2);
    assert_eq!(undo_depth(&view, cx), depth + 1, "the drag is one entry");
    assert!(!gesture_open(&view, cx));
    view.update(cx, |view, cx| view.execute(Command::Undo, Source::Cli, cx))
        .unwrap();
    assert_eq!(intensity(&view, cx, "fx-1"), 0.7);
    view.update(cx, |view, cx| view.execute(Command::Redo, Source::Cli, cx))
        .unwrap();
    assert_eq!(intensity(&view, cx, "fx-1"), 0.2);
}

#[gpui_kit::test]
fn a_refused_value_keeps_the_last_legal_candidate_and_the_commit_confirms_that_one(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mounted(cx);
    view.update(cx, |view, _| view.state.timeline.set_playhead(12));
    add_vignette(&view, cx);
    let depth = undo_depth(&view, cx);
    let duration = key("street", "fx-1", "duration_frames");

    change(&view, cx, duration.clone(), 20.0);
    assert_eq!(clip_state(&view, cx, "fx-1")["duration_frames"], json!(20));
    // 96 frames from frame 12 do not fit the edit: the layer refuses, with a typed error on screen.
    change(&view, cx, duration.clone(), 96.0);
    assert!(
        !panel_notice(&view, cx).is_null(),
        "{}",
        node(&view, cx, "scene.panel")
    );
    assert!(
        gesture_open(&view, cx),
        "a refused value does not end the gesture"
    );
    assert_eq!(
        clip_state(&view, cx, "fx-1")["duration_frames"],
        json!(20),
        "the candidate is the last legal one"
    );
    let gesture = node(&view, cx, "scene.fx")["state"]["gesture"].clone();
    assert!(gesture["refusal"]["code"].is_string(), "{gesture}");

    release(&view, cx, duration);
    assert_eq!(clip_state(&view, cx, "fx-1")["duration_frames"], json!(20));
    assert_eq!(undo_depth(&view, cx), depth + 1);
    assert_eq!(
        panel_notice(&view, cx),
        Value::Null,
        "the commit clears the notice"
    );
}

#[gpui_kit::test]
fn focus_loss_commits_once_and_a_late_release_does_not_commit_again(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    add_vignette(&view, cx);
    let depth = undo_depth(&view, cx);
    let intensity_key = key("street", "fx-1", "intensity");

    change(&view, cx, intensity_key.clone(), 0.3);
    blur(&view, cx);
    assert_eq!(intensity(&view, cx, "fx-1"), 0.3);
    assert_eq!(undo_depth(&view, cx), depth + 1);
    blur(&view, cx);
    // The rest of that press is ignored: a late Change does not start another gesture, and the
    // release that finally arrives commits nothing again.
    change(&view, cx, intensity_key.clone(), 0.9);
    assert!(!gesture_open(&view, cx));
    release(&view, cx, intensity_key);
    assert_eq!(undo_depth(&view, cx), depth + 1, "no second commit");
    assert!(!gesture_open(&view, cx));
    assert_eq!(intensity(&view, cx, "fx-1"), 0.3);
}

#[gpui_kit::test]
fn the_keyboard_steps_on_the_controls_grid_and_confirms_or_cancels(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    add_vignette(&view, cx);
    let depth = undo_depth(&view, cx);
    let intensity_key = key("street", "fx-1", "intensity");
    let down = |view: &Entity<Workstation>,
                cx: &mut VisualTestContext,
                pressed: &str,
                shift: bool,
                held: bool| {
        let key = key("street", "fx-1", "intensity");
        let handled = view.update(cx, |view, cx| {
            view.fx_key_down(&key, pressed, shift, held, cx)
        });
        cx.run_until_parked();
        handled
    };
    let candidate = |view: &Entity<Workstation>, cx: &mut VisualTestContext| {
        field_value(&node(view, cx, CLIP), "intensity")
            .as_f64()
            .unwrap()
    };

    // A press steps by one step of the control (0.01), a held key continues the same gesture.
    assert!(down(&view, cx, "right", false, false));
    assert_eq!(candidate(&view, cx), 0.71);
    assert!(down(&view, cx, "right", false, true));
    assert_eq!(candidate(&view, cx), 0.72);
    assert_eq!(
        intensity(&view, cx, "fx-1"),
        0.7,
        "nothing is written while the key is down"
    );
    assert_eq!(undo_depth(&view, cx), depth);
    view.update(cx, |view, cx| view.fx_key_up(&intensity_key, "right", cx));
    assert_eq!(intensity(&view, cx, "fx-1"), 0.72);
    assert_eq!(undo_depth(&view, cx), depth + 1);

    // Shift is ten steps, the range ends at the control's own bounds, and keys it ignores are not its.
    assert!(down(&view, cx, "left", true, false));
    assert_eq!(candidate(&view, cx), 0.62);
    assert!(down(&view, cx, "end", false, true));
    assert_eq!(candidate(&view, cx), 1.0);
    assert!(down(&view, cx, "right", false, true));
    assert_eq!(
        candidate(&view, cx),
        1.0,
        "past the range is the range's end"
    );
    assert!(!down(&view, cx, "x", false, true), "a letter is not a step");
    // Enter confirms the gesture in progress: one more entry, and the key-up does not repeat it.
    assert!(down(&view, cx, "enter", false, false));
    assert_eq!(intensity(&view, cx, "fx-1"), 1.0);
    assert_eq!(undo_depth(&view, cx), depth + 2);
    view.update(cx, |view, cx| view.fx_key_up(&intensity_key, "end", cx));
    assert_eq!(undo_depth(&view, cx), depth + 2);

    // Escape cancels what the keys changed (a press on the slider gave the window the focus).
    let at = slider_point(&view, cx, "intensity", 0.5);
    cx.simulate_mouse_move(at, None, Default::default());
    cx.simulate_mouse_down(at, MouseButton::Left, Default::default());
    assert!(down(&view, cx, "home", false, true));
    assert_eq!(candidate(&view, cx), 0.0);
    cx.simulate_keystrokes("escape");
    assert!(!gesture_open(&view, cx));
    cx.simulate_mouse_up(at, MouseButton::Left, Default::default());
    cx.run_until_parked();
    assert_eq!(intensity(&view, cx, "fx-1"), 1.0);
    assert_eq!(undo_depth(&view, cx), depth + 2);
}

#[gpui_kit::test]
fn the_vignette_is_inserted_where_the_monitor_is_and_a_frame_outside_the_edit_refuses(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mounted(cx);
    view.update(cx, |view, _| view.state.timeline.set_playhead(12));
    let frame = view.read_with(cx, |view, _| {
        view.state.scene_frame_at_playhead("street").unwrap()
    });
    assert_eq!(
        node(&view, cx, "scene.fx")["state"]["playhead_frame"],
        json!(frame)
    );
    add_vignette(&view, cx);
    let state = clip_state(&view, cx, "fx-1");
    assert_eq!(state["start_frame"], json!(frame));
    assert!(
        state["end_frame_exclusive"].as_i64().unwrap() <= 96,
        "the duration fits the edit: {state}"
    );

    // Extend the fixture if necessary so this assertion cannot silently skip the refusal.
    let past = view.update(cx, |view, _| {
        view.state.timeline.length = view.state.timeline.length.max(192);
        let frame = view.state.timeline.length - 1;
        view.state.timeline.set_playhead(frame);
        let bundle_frame = view.state.scene_frame_at_playhead("street").unwrap();
        let duration = view
            .state
            .scenes
            .get("street")
            .unwrap()
            .scene
            .bundle()
            .edit_duration_frames;
        bundle_frame >= i64::try_from(duration).unwrap()
    });
    assert!(past, "the premise must be outside the bundle edit");
    let depth = undo_depth(&view, cx);
    let clips_before = clips(&view, cx);
    let error = invoke(&view, cx, "scene.fx", "add:vignette").unwrap_err();
    assert!(!error.code.is_empty());
    assert_eq!(undo_depth(&view, cx), depth);
    assert_eq!(clips(&view, cx), clips_before);
}

fn nine_vignettes(cx: &mut TestAppContext) -> (Entity<Workstation>, &mut VisualTestContext) {
    let (view, cx) = mounted(cx);
    for _ in 0..9 {
        add_vignette(&view, cx);
    }
    (view, cx)
}

fn budget(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Value {
    node(view, cx, "scene.fx.budget")
}

#[gpui_kit::test]
fn more_than_eight_passes_warns_visibly_and_on_a_node_and_never_drops_a_pass(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mounted(cx);
    for _ in 0..8 {
        add_vignette(&view, cx);
    }
    // Eight is the budget: no warning, the node says so and is hidden.
    assert_eq!(budget(&view, cx)["state"]["over_budget"], json!(false));
    assert_eq!(budget(&view, cx)["visible"], json!(false));
    assert!(view.read_with(cx, |view, _| {
        view.geometry.get("control:fx-budget-warning").is_none()
    }));

    add_vignette(&view, cx);
    let warning = budget(&view, cx);
    assert_eq!(warning["state"]["over_budget"], json!(true));
    assert_eq!(warning["visible"], json!(true));
    assert_eq!(warning["state"]["peak_passes"], json!(9));
    assert_eq!(
        warning["state"]["warnings"][0]["code"],
        json!("FX_PASS_BUDGET")
    );
    assert!(warning["state"]["message"].as_str().unwrap().contains("9"));
    assert!(
        view.read_with(cx, |view, _| view
            .geometry
            .get("control:fx-budget-warning")
            .is_some()),
        "the warning is painted in the Effects section"
    );
    assert_eq!(
        node(&view, cx, "scene.fx")["state"]["over_budget"],
        json!(true)
    );
    // Nothing was truncated, refused or hidden: every pass is still resolved.
    let passes = view.read_with(cx, |view, _| {
        view.state
            .scenes
            .get("street")
            .unwrap()
            .scene
            .bundle()
            .fx_passes(0)
            .len()
    });
    assert_eq!(passes, 9);
}

#[gpui_kit::test]
fn the_warning_follows_hide_remove_candidate_and_scene_switch(cx: &mut TestAppContext) {
    let (view, cx) = nine_vignettes(cx);
    assert_eq!(budget(&view, cx)["state"]["over_budget"], json!(true));

    // Hiding one clip is an edit: back inside the budget; showing it warns again.
    set(&view, cx, "scene.fx.clip:fx-9", "hidden", Raw::Bool(true)).unwrap();
    assert_eq!(budget(&view, cx)["state"]["over_budget"], json!(false));
    assert_eq!(budget(&view, cx)["state"]["message"], Value::Null);
    set(&view, cx, "scene.fx.clip:fx-9", "hidden", Raw::Bool(false)).unwrap();
    assert_eq!(budget(&view, cx)["state"]["over_budget"], json!(true));

    // A candidate that hides one is shown by the warning BEFORE the commit (and the document warns).
    let gesture = view
        .update(cx, |view, cx| {
            view.scene_execute(
                SceneCommand::FxGesture(
                    tessel_editor_core::scene_interaction::FxGestureCommand::Begin {
                        scene: "street".into(),
                        clip: "fx-9".into(),
                        label: None,
                    },
                ),
                Source::Cli,
                cx,
            )
        })
        .unwrap()["gesture"]
        .as_str()
        .unwrap()
        .to_owned();
    view.update(cx, |view, cx| {
        view.scene_execute(
            SceneCommand::FxGesture(
                tessel_editor_core::scene_interaction::FxGestureCommand::Update {
                    gesture,
                    ops: vec![tessel_editor_core::scene_edit::FxOp::SetHidden {
                        clip: "fx-9".into(),
                        hidden: true,
                    }],
                },
            ),
            Source::Cli,
            cx,
        )
    })
    .unwrap();
    cx.run_until_parked();
    let candidate = budget(&view, cx);
    assert_eq!(candidate["state"]["over_budget"], json!(false));
    assert_eq!(candidate["state"]["source"], json!("gesture"));
    view.update(cx, |view, cx| {
        view.scene_cancel_open_gesture(Source::Cli, cx)
    })
    .unwrap();
    assert_eq!(budget(&view, cx)["state"]["over_budget"], json!(true));
    assert_eq!(budget(&view, cx)["state"]["source"], json!("document"));

    // Removing clips brings it down; undo brings the warning back.
    invoke(&view, cx, "scene.fx.clip:fx-9", "remove").unwrap();
    assert_eq!(budget(&view, cx)["state"]["over_budget"], json!(false));
    view.update(cx, |view, cx| view.execute(Command::Undo, Source::Cli, cx))
        .unwrap();
    assert_eq!(budget(&view, cx)["state"]["over_budget"], json!(true));

    // Another scene has its own layer: no warning there, and it returns with the first scene.
    attach_yard(&view, cx);
    choose(&view, cx, "yard");
    assert_eq!(budget(&view, cx)["state"]["over_budget"], json!(false));
    assert_eq!(budget(&view, cx)["visible"], json!(false));
    choose(&view, cx, "street");
    assert_eq!(budget(&view, cx)["state"]["over_budget"], json!(true));
}

// ---- BL082.20: the MONITOR goes back to the confirmed scene when a gesture is invalidated or ends ------------
//
// A test window has no page, so the picture itself is not drawn here; what is proven is the request
// the monitor holds for its next picture (`directed_desired_plan`: the same plan the page would get)
// and the failure it would show. A gesture the core can no longer apply (an undo, an external edit)
// stays in its slot until cancelled, and asking the plan for it is `gesture_invalidated`: the monitor
// must ask for the document instead, with no failure the panel does not explain.

/// The verified resources of `scene`, as `scene.resources.resolve` would leave them. The plan the
/// monitor asks for refuses a scene whose files were never approved (`scene_resources_not_loaded`),
/// and a test window has no folder to resolve them from, so the approved map is installed directly
/// with the paths and hashes the bundle declares. No file is read: nothing draws in a test window.
/// An absolute folder on every platform (`/synthetic` has no drive on Windows, so the resource
/// map refuses it). It does not need to exist: nothing is read.
fn synthetic_base() -> std::path::PathBuf {
    std::env::temp_dir().join("synthetic")
}

fn load_resources(view: &Entity<Workstation>, scene: &str, cx: &mut VisualTestContext) {
    use tessel_editor_core::scene_resources::{ResourceKey, ResourceKind};
    use tessel_editor_media::scene_resources::map::{ResolvedResource, ResolvedResourceMap};

    view.update(cx, |view, _| {
        let session = view.workflow.scene_resources();
        let generation = session.observe_project(&view.state.project.id);
        let bundle = view.state.scenes.get(scene).unwrap().scene.bundle().clone();
        let entries = bundle
            .assets
            .iter()
            .map(|asset| ResolvedResource {
                key: ResourceKey::asset(&asset.asset_id),
                kind: ResourceKind::Image,
                path: synthetic_base().join(&asset.relative_path),
                content_hash: asset.content_hash.clone(),
                revision_id: Some(asset.revision_id.clone()),
                bytes: 1,
                uses: Vec::new(),
            })
            .collect();
        session
            .install(ResolvedResourceMap {
                scene_id: scene.to_owned(),
                bundle_id: bundle.bundle_id.clone(),
                bundle_revision: bundle.revision_id.clone(),
                project_id: view.state.project.id.clone(),
                generation,
                base: synthetic_base(),
                entries,
                skipped: Vec::new(),
                complete: true,
            })
            .expect("the synthetic map belongs to the current project and generation");
    });
}

/// The monitor shows `street` as a directed scene, as the app does, and follows the state once.
fn watch_street(view: &Entity<Workstation>, cx: &mut VisualTestContext) {
    load_resources(view, "street", cx);
    view.update(cx, |view, cx| {
        view.viewport.pretend_directed_scene("street");
        view.viewport_sync(cx);
    });
    cx.run_until_parked();
}

/// `street` with a vignette (`fx-1`, intensity 0.7, from frame 0 and with no fades), on the monitor.
fn monitored(cx: &mut TestAppContext) -> (Entity<Workstation>, &mut VisualTestContext) {
    let (view, cx) = mounted(cx);
    add_vignette(&view, cx);
    watch_street(&view, cx);
    (view, cx)
}

/// The plan the monitor holds for its next picture.
fn monitor_plan(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Value {
    view.read_with(cx, |view, _| view.viewport.directed_desired_plan())
        .expect("the monitor holds a request for the scene it shows")
}

/// The vignette's amount in that plan: frame 0 is the clip's first frame, so it is the intensity.
fn monitor_amount(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Value {
    monitor_plan(view, cx)["fx"][0]["amount"].clone()
}

/// The code of the failure the monitor shows, if any.
fn monitor_failure(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Option<String> {
    view.read_with(cx, |view, _| {
        view.viewport.failure().map(|error| error.code.to_string())
    })
}

/// The plan the core gives for the CONFIRMED scene at the playhead, with no gesture: what the monitor
/// must be asked to draw once only the document is left to show.
fn document_plan(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Value {
    view.update(cx, |view, _| {
        let frame = view.state.scene_frame_at_playhead("street").unwrap();
        let request = json!({ "scene": "street", "frame": frame, "gesture": null });
        tessel_editor_render::scene_operation::prepare(
            &mut view.state,
            &request,
            &view.workflow.scene_resources(),
        )
        .unwrap()
        .0
        .to_json()
    })
}

/// The core holds a gesture it can no longer apply.
fn gesture_invalidated(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |view, _| {
        view.state
            .scene_interaction
            .gesture()
            .is_some_and(|gesture| {
                gesture
                    .invalidation(&view.state.scenes, view.state.project.revision)
                    .is_some()
            })
    })
}

/// The monitor draws the document: the confirmed amount, the plan the core gives with no gesture,
/// and no failure.
fn assert_monitor_shows_the_document(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    amount: f64,
    why: &str,
) {
    assert_eq!(monitor_failure(view, cx), None, "{why}: no failure");
    assert_eq!(monitor_amount(view, cx), json!(amount), "{why}: the amount");
    let expected = document_plan(view, cx);
    assert_eq!(
        monitor_plan(view, cx),
        expected,
        "{why}: the plan is the confirmed scene's"
    );
}

#[gpui_kit::test]
fn an_external_edit_under_the_drag_sends_the_monitor_back_to_the_document_and_the_commit_is_refused(
    cx: &mut TestAppContext,
) {
    let (view, cx) = monitored(cx);
    let intensity_key = key("street", "fx-1", "intensity");
    assert_monitor_shows_the_document(&view, cx, 0.7, "before the drag");

    change(&view, cx, intensity_key.clone(), 0.2);
    assert_eq!(
        monitor_amount(&view, cx),
        json!(0.2),
        "the premise: the monitor draws the candidate while the drag is valid"
    );
    assert_eq!(monitor_failure(&view, cx), None);

    // Another writer (a script, the CLI) edits the scene under the drag.
    view.update(cx, |view, cx| {
        view.run_op(
            &json!({"op":"scene.edit","scene":"street","ops":[
                {"set": "scene/scene-a/node/figure/width", "value": 3.0}]}),
            cx,
        )
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        gesture_invalidated(&view, cx),
        "the premise: the gesture stays in its slot, no longer applicable"
    );
    let depth = undo_depth(&view, cx);

    // The monitor does not ask for the invalid gesture (that would be `gesture_invalidated`): it
    // draws the document, and the panel, which ignores the gesture too, shows the same value.
    assert_monitor_shows_the_document(&view, cx, 0.7, "after the external edit");
    assert_eq!(field_value(&node(&view, cx, CLIP), "intensity"), json!(0.7));

    // The release is a stale commit: refused with the typed error, no entry, no value written.
    release(&view, cx, intensity_key);
    assert_eq!(
        panel_notice(&view, cx)["code"],
        json!("gesture_invalidated")
    );
    assert_eq!(undo_depth(&view, cx), depth, "no extra undo entry");
    assert_eq!(intensity(&view, cx, "fx-1"), 0.7);
    assert!(
        !gesture_open(&view, cx),
        "the refused gesture was given back"
    );
    assert_monitor_shows_the_document(&view, cx, 0.7, "after the refused commit");
}

#[gpui_kit::test]
fn an_undo_under_the_drag_sends_the_monitor_to_the_undone_scene_and_redo_is_not_lost(
    cx: &mut TestAppContext,
) {
    let (view, cx) = monitored(cx);
    let intensity_key = key("street", "fx-1", "intensity");
    // A confirmed edit first, so there is something to undo: 0.7 -> 0.5.
    change(&view, cx, intensity_key.clone(), 0.5);
    release(&view, cx, intensity_key.clone());
    assert_eq!(intensity(&view, cx, "fx-1"), 0.5);
    assert_monitor_shows_the_document(&view, cx, 0.5, "after the confirmed edit");

    change(&view, cx, intensity_key.clone(), 0.2);
    assert_eq!(
        monitor_amount(&view, cx),
        json!(0.2),
        "the premise: the second drag is on the monitor"
    );

    view.update(cx, |view, cx| view.execute(Command::Undo, Source::Cli, cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(intensity(&view, cx, "fx-1"), 0.7);
    assert!(
        gesture_invalidated(&view, cx),
        "the premise: the undo left the drag invalid in its slot"
    );
    let depth = total_depth(&view, cx);

    // Neither the candidate (0.2) nor the undone value (0.5): the scene as confirmed after the undo.
    assert_monitor_shows_the_document(&view, cx, 0.7, "after the undo");
    assert_eq!(field_value(&node(&view, cx, CLIP), "intensity"), json!(0.7));

    release(&view, cx, intensity_key);
    assert_eq!(
        total_depth(&view, cx),
        depth,
        "the stale commit added nothing"
    );
    assert_eq!(intensity(&view, cx, "fx-1"), 0.7);
    assert_monitor_shows_the_document(&view, cx, 0.7, "after the refused commit");
    // The refusal wrote nothing, so the redo stack is still there for the user.
    view.update(cx, |view, cx| view.execute(Command::Redo, Source::Cli, cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(intensity(&view, cx, "fx-1"), 0.5);
    assert_monitor_shows_the_document(&view, cx, 0.5, "after the redo");
}

#[gpui_kit::test]
fn switching_the_scene_cancelling_and_a_resting_thumb_leave_the_monitor_on_the_document(
    cx: &mut TestAppContext,
) {
    let (view, cx) = two_scenes(cx);
    watch_street(&view, cx);
    let depth = total_depth(&view, cx);
    let intensity_key = key("street", "fx-1", "intensity");

    // The panel moves to another scene mid-drag: the core drops the gesture and the monitor, which
    // was drawing its candidate, is told at once (not at the next unrelated command).
    change(&view, cx, intensity_key.clone(), 0.2);
    assert_eq!(monitor_amount(&view, cx), json!(0.2), "the premise");
    choose(&view, cx, "yard");
    assert!(!gesture_open(&view, cx));
    assert_monitor_shows_the_document(&view, cx, 0.7, "after the switch");
    choose(&view, cx, "street");

    // Escape drops the drag and the monitor goes back.
    change(&view, cx, intensity_key.clone(), 0.3);
    assert_eq!(monitor_amount(&view, cx), json!(0.3), "the premise");
    view.update(cx, |view, cx| view.fx_cancel_drag(cx));
    cx.run_until_parked();
    assert!(!gesture_open(&view, cx));
    assert_monitor_shows_the_document(&view, cx, 0.7, "after Escape");

    // A thumb that rests where the document already is writes nothing and shows nothing else.
    change(&view, cx, intensity_key.clone(), 0.7);
    assert_monitor_shows_the_document(&view, cx, 0.7, "during a resting thumb");
    release(&view, cx, intensity_key);
    assert_monitor_shows_the_document(&view, cx, 0.7, "after a resting thumb");
    assert_eq!(
        total_depth(&view, cx),
        depth,
        "none of the three made an entry"
    );
    assert_eq!(intensity_in(&view, cx, "street", "fx-1"), 0.7);
    assert_eq!(intensity_in(&view, cx, "yard", "fx-1"), 0.7);
}

#[gpui_kit::test]
fn a_semantic_set_never_writes_the_candidate_of_an_open_gesture_into_the_document(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mounted(cx);
    add_vignette(&view, cx);
    let before_x = field_value(&node(&view, cx, CLIP), "center_x");
    // A drag moves `center_x` (a candidate: nothing is written yet) ...
    change(&view, cx, key("street", "fx-1", "center_x"), 0.2);
    assert_eq!(
        field_value(&node(&view, cx, CLIP), "center_x"),
        json!(0.2),
        "the premise: the panel shows the candidate"
    );
    // ... and a script sets the SIBLING component of the same point. The write keeps the document's
    // `center_x`, not the drag's uncommitted one.
    set(&view, cx, CLIP, "center_y", Raw::Number(0.8)).unwrap();
    assert_eq!(
        field_value(&node(&view, cx, CLIP), "center_x"),
        before_x,
        "the candidate's component did not leak into the document"
    );
    assert_eq!(field_value(&node(&view, cx, CLIP), "center_y"), json!(0.8));
}

#[test]
fn the_grid_of_a_fine_step_is_not_lost_through_the_adapter() {
    // The rule is shared by pointer, keyboard and scripts: a 0.25 or 0.125 step stays on its grid
    // (0.25 is not 0.3) and a value past the range goes to the range's end.
    use tessel_editor_core::directed_scene::fx::quantize_control;
    assert_eq!(quantize_control(0.74, 0.0, 1.0, 0.25, false), 0.75);
    assert_eq!(quantize_control(0.26, 0.0, 1.0, 0.25, false), 0.25);
    assert_eq!(quantize_control(0.13, 0.0, 1.0, 0.125, false), 0.125);
    assert_eq!(quantize_control(0.4, 0.0, 1.0, 0.01, false), 0.4);
    assert_eq!(quantize_control(1.7, 0.0, 1.0, 0.25, false), 1.0);
}

#[gpui_kit::test]
fn choosing_the_displayed_fallback_scene_keeps_its_effect_candidate(cx: &mut TestAppContext) {
    let (view, cx) = two_scenes(cx);
    view.update(cx, |view, _| view.scene_ui.set_chosen_for_test("detached"));
    let scene = view.read_with(cx, |view, _| view.scene_active_id().unwrap());
    change(&view, cx, key(&scene, "fx-1", "intensity"), 0.2);
    assert!(gesture_open(&view, cx));
    choose(&view, cx, &scene);
    assert!(
        gesture_open(&view, cx),
        "the displayed scene did not change"
    );
}

/// The scene the panel shows by fallback (no choice, or a choice that is no longer attached) and the
/// other one.
fn shown_and_other(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> (String, String) {
    let shown = view.read_with(cx, |view, _| view.scene_active_id().unwrap());
    let other = if shown == "street" { "yard" } else { "street" };
    (shown, other.to_string())
}

#[gpui_kit::test]
fn a_panel_with_no_choice_keeps_the_gesture_of_the_scene_it_shows_and_drops_it_for_the_other(
    cx: &mut TestAppContext,
) {
    let (view, cx) = two_scenes(cx);
    view.update(cx, |view, _| view.scene_ui.clear_chosen_for_test());
    let (shown, other) = shown_and_other(&view, cx);
    let depth = total_depth(&view, cx);

    change(&view, cx, key(&shown, "fx-1", "intensity"), 0.2);
    assert!(gesture_open(&view, cx));
    choose(&view, cx, &shown);
    assert!(
        gesture_open(&view, cx),
        "the fallback scene was already the one displayed"
    );

    choose(&view, cx, &other);
    assert!(
        !gesture_open(&view, cx) && !driving(&view, cx),
        "another scene drops the candidate of the displayed one"
    );
    blur(&view, cx);
    assert_eq!(intensity_in(&view, cx, &shown, "fx-1"), 0.7);
    assert_eq!(intensity_in(&view, cx, &other, "fx-1"), 0.7);
    assert_eq!(total_depth(&view, cx), depth, "nothing was committed");
}

#[gpui_kit::test]
fn a_detached_choice_falls_back_and_another_scene_still_drops_the_gesture(cx: &mut TestAppContext) {
    let (view, cx) = two_scenes(cx);
    view.update(cx, |view, _| view.scene_ui.set_chosen_for_test("detached"));
    let (shown, other) = shown_and_other(&view, cx);
    let depth = total_depth(&view, cx);

    change(&view, cx, key(&shown, "fx-1", "intensity"), 0.2);
    assert!(gesture_open(&view, cx));
    choose(&view, cx, &other);
    assert!(!gesture_open(&view, cx), "the displayed scene did change");
    assert_eq!(intensity_in(&view, cx, &shown, "fx-1"), 0.7);
    assert_eq!(total_depth(&view, cx), depth);
}

#[gpui_kit::test]
fn entering_the_scene_that_owns_the_gesture_keeps_it(cx: &mut TestAppContext) {
    let (view, cx) = two_scenes(cx);
    let gesture = view
        .update(cx, |view, cx| {
            view.scene_execute(
                SceneCommand::FxGesture(
                    tessel_editor_core::scene_interaction::FxGestureCommand::Begin {
                        scene: "yard".into(),
                        clip: "fx-1".into(),
                        label: None,
                    },
                ),
                Source::Cli,
                cx,
            )
        })
        .unwrap()["gesture"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(gesture_open(&view, cx), "begun on `yard`, shown: `street`");

    choose(&view, cx, "yard");
    let still = view.read_with(cx, |view, _| {
        view.state
            .scene_interaction
            .gesture()
            .map(|open| open.id().to_owned())
    });
    assert_eq!(still, Some(gesture), "entering the owning scene keeps it");
}

#[gpui_kit::test]
fn switching_between_other_scenes_keeps_a_third_scenes_gesture(cx: &mut TestAppContext) {
    let (view, cx) = two_scenes(cx);
    view.update(cx, |view, cx| {
        view.execute(
            Command::Scene(SceneCommand::Attach {
                id: Some("plaza".into()),
                name: None,
                source: SceneSource::Text(BUNDLE.to_string()),
            }),
            Source::Cli,
            cx,
        )
        .unwrap();
    });
    choose(&view, cx, "street");
    let gesture = view
        .update(cx, |view, cx| {
            view.scene_execute(
                SceneCommand::FxGesture(
                    tessel_editor_core::scene_interaction::FxGestureCommand::Begin {
                        scene: "yard".into(),
                        clip: "fx-1".into(),
                        label: None,
                    },
                ),
                Source::Cli,
                cx,
            )
        })
        .unwrap()["gesture"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(gesture_open(&view, cx), "begun on `yard`, shown: `street`");

    choose(&view, cx, "plaza");
    let still = view.read_with(cx, |view, _| {
        view.state
            .scene_interaction
            .gesture()
            .map(|open| open.id().to_owned())
    });
    assert_eq!(
        still,
        Some(gesture),
        "leaving street must not cancel a gesture owned by yard"
    );
}

// --- Integration: an effect gesture while the diagnostic history records (BL082.20 x BL145.6) ---
//
// The two features were built on separate branches. Here they run in one window: the gesture is a
// core object that the monitor draws only through `viewport_sync`, and the history is a recorder fed
// by paints and by the monitor host. Neither may stop, inflate or invent anything for the other.

fn history(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    command: tessel_editor_core::perf_history::HistoryCommand,
) -> Result<Value, tessel_editor_core::CommandError> {
    let result = view.update_in(cx, |view, window, cx| {
        view.execute_perf_history(command, Source::Cli, window, cx)
    });
    cx.run_until_parked();
    result
}

fn paint_a_frame(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.refresh();
        window.simulate_next_frame(cx);
    });
    cx.run_until_parked();
}

/// The frames the LIVE recorder holds, whatever the panel shows (frozen or not).
fn live_history_frames(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> u64 {
    use tessel_editor_core::perf_history::Metric;
    view.read_with(cx, |view, _| {
        let now = view.perf.elapsed();
        let meter = view.perf.meter();
        let meter = meter.borrow();
        meter
            .recorder()
            .snapshot(now)
            .and_then(|snapshot| snapshot.summary(Metric::UiFrames))
            .map_or(0, |summary| summary.count)
    })
}

fn history_events(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
) -> Vec<tessel_editor_core::perf_history::EventKind> {
    view.read_with(cx, |view, _| {
        let now = view.perf.elapsed();
        let meter = view.perf.meter();
        let meter = meter.borrow();
        meter
            .recorder()
            .snapshot(now)
            .map(|snapshot| snapshot.events.iter().map(|event| event.kind).collect())
            .unwrap_or_default()
    })
}

fn history_recording(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |view, _| {
        view.perf.meter().borrow().recorder().recording()
    })
}

#[gpui_kit::test]
fn a_gesture_during_a_history_recording_neither_stops_it_nor_leaks_into_it(
    cx: &mut TestAppContext,
) {
    use std::time::Duration;
    use tessel_editor_core::perf_history::{EventKind, HistoryCommand};

    let (view, cx) = two_scenes(cx);
    watch_street(&view, cx);
    let intensity_key = key("street", "fx-1", "intensity");
    history(&view, cx, HistoryCommand::Record(true)).unwrap();
    for _ in 0..3 {
        paint_a_frame(cx);
    }
    cx.executor().advance_clock(Duration::from_millis(600));
    paint_a_frame(cx);
    let depth = total_depth(&view, cx);

    // A drag: the monitor draws the candidate and the recording does not notice.
    change(&view, cx, intensity_key.clone(), 0.2);
    paint_a_frame(cx);
    assert_eq!(monitor_amount(&view, cx), json!(0.2), "the premise");
    assert!(history_recording(&view, cx), "a drag does not stop it");

    // Freeze mid-drag: one immutable window, while the recording and the drag go on underneath.
    history(&view, cx, HistoryCommand::Freeze).unwrap();
    let frozen_at = live_history_frames(&view, cx);
    change(&view, cx, intensity_key.clone(), 0.3);
    for _ in 0..3 {
        paint_a_frame(cx);
    }
    assert!(
        live_history_frames(&view, cx) >= frozen_at + 3,
        "the recording continues while frozen and dragging"
    );
    assert_eq!(monitor_amount(&view, cx), json!(0.3), "the drag continues");

    // Select inside the frozen window, then export it, both while the drag is open.
    let captured = view.read_with(cx, |view, _| {
        view.perf
            .meter()
            .borrow()
            .recorder()
            .frozen()
            .unwrap()
            .captured_at
    });
    let inspected = history(
        &view,
        cx,
        HistoryCommand::Select(Some(captured - Duration::from_millis(100))),
    )
    .unwrap();
    assert!(inspected["inspection"].is_object(), "{inspected}");
    let dir = std::env::temp_dir().join(format!("tessel-fx-history-smoke-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("history.json");
    history(&view, cx, HistoryCommand::Export(target.clone())).unwrap();
    let text = std::fs::read_to_string(&target).expect("the export was written");
    let document: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(document["schema"], json!("tessel.perf-history"));
    assert!(
        !text.contains(dir.to_str().unwrap()),
        "no local path in the document"
    );
    let _ = std::fs::remove_dir_all(&dir);
    history(&view, cx, HistoryCommand::Select(None)).unwrap();
    history(&view, cx, HistoryCommand::Resume).unwrap();

    // The release is one undo entry and the monitor returns to the document.
    release(&view, cx, intensity_key.clone());
    assert_eq!(total_depth(&view, cx), depth + 1, "one entry");
    assert_eq!(intensity_in(&view, cx, "street", "fx-1"), 0.3);
    assert_monitor_shows_the_document(&view, cx, 0.3, "after the release");

    // Undo, cancel and a scene switch under a drag: each one leaves the monitor on the document.
    view.update(cx, |view, cx| view.execute(Command::Undo, Source::Cli, cx))
        .unwrap();
    cx.run_until_parked();
    assert_monitor_shows_the_document(&view, cx, 0.7, "after the undo");
    change(&view, cx, intensity_key.clone(), 0.4);
    view.update(cx, |view, cx| view.fx_cancel_drag(cx));
    cx.run_until_parked();
    assert!(!gesture_open(&view, cx));
    assert_monitor_shows_the_document(&view, cx, 0.7, "after Escape");
    change(&view, cx, intensity_key, 0.2);
    choose(&view, cx, "yard");
    assert!(!gesture_open(&view, cx));
    assert_monitor_shows_the_document(&view, cx, 0.7, "after the switch");

    // Through all of it the recording ran, and the effect gesture invented no event of its own.
    assert!(history_recording(&view, cx));
    paint_a_frame(cx);
    let events = history_events(&view, cx);
    assert_eq!(
        events.first(),
        Some(&EventKind::RecordingStarted),
        "{events:?}"
    );
    for forbidden in [
        EventKind::PlaybackStarted,
        EventKind::RequestFailed,
        EventKind::RuntimeUnavailable,
    ] {
        assert!(
            !events.contains(&forbidden),
            "a gesture is no {forbidden:?}: {events:?}"
        );
    }
    assert_eq!(monitor_failure(&view, cx), None);
}

#[gpui_kit::test]
fn starting_and_clearing_the_history_during_a_drag_does_not_touch_the_gesture(
    cx: &mut TestAppContext,
) {
    use tessel_editor_core::perf_history::HistoryCommand;

    let (view, cx) = monitored(cx);
    let intensity_key = key("street", "fx-1", "intensity");
    change(&view, cx, intensity_key.clone(), 0.2);
    // Recording starts, is cleared and stops under an open drag: the candidate stays what it was.
    history(&view, cx, HistoryCommand::Record(true)).unwrap();
    paint_a_frame(cx);
    history(&view, cx, HistoryCommand::Clear).unwrap();
    history(&view, cx, HistoryCommand::Record(false)).unwrap();
    assert!(gesture_open(&view, cx));
    assert_eq!(monitor_amount(&view, cx), json!(0.2));
    release(&view, cx, intensity_key);
    assert_eq!(intensity(&view, cx, "fx-1"), 0.2);
    assert_monitor_shows_the_document(&view, cx, 0.2, "after the release");
}

#[gpui_kit::test]
fn another_change_after_invalidation_cancels_only_the_old_drag(cx: &mut TestAppContext) {
    let (view, cx) = monitored(cx);
    let control = key("street", "fx-1", "intensity");
    change(&view, cx, control.clone(), 0.2);
    view.update(cx, |view, cx| {
        view.run_op(
            &json!({"op":"scene.edit","scene":"street",
        "ops":[{"set":"scene/scene-a/node/figure/width","value":3.0}]}),
            cx,
        )
    })
    .unwrap();
    assert!(gesture_invalidated(&view, cx));
    let depth = total_depth(&view, cx);
    change(&view, cx, control.clone(), 0.3);
    assert!(
        !gesture_open(&view, cx),
        "Change must give back the invalidated slot before release"
    );
    assert_monitor_shows_the_document(&view, cx, 0.7, "invalidated Change");
    release(&view, cx, control);
    assert_eq!(total_depth(&view, cx), depth);
    assert_monitor_shows_the_document(&view, cx, 0.7, "release after invalidated Change");
}

#[gpui_kit::test]
fn another_change_after_invalidation_preserves_a_replacement_gesture(cx: &mut TestAppContext) {
    let (view, cx) = monitored(cx);
    let control = key("street", "fx-1", "intensity");
    change(&view, cx, control.clone(), 0.2);
    view.update(cx, |view, cx| {
        view.run_op(
            &json!({"op":"scene.edit","scene":"street",
        "ops":[{"set":"scene/scene-a/node/figure/width","value":3.0}]}),
            cx,
        )
    })
    .unwrap();
    view.update(cx, |view, cx| {
        view.run_op(
            &json!({
        "op":"scene.fx.gesture.begin","scene":"street","clip":"fx-1"}),
            cx,
        )
    })
    .unwrap();
    let id = view.read_with(cx, |view, _| {
        view.state
            .scene_interaction
            .gesture()
            .unwrap()
            .id()
            .to_owned()
    });
    change(&view, cx, control.clone(), 0.3);
    release(&view, cx, control);
    assert_eq!(
        view.read_with(cx, |view, _| view
            .state
            .scene_interaction
            .gesture()
            .map(|gesture| gesture.id().to_owned())),
        Some(id)
    );
}

// ---- P2: the neutral inspector model and the second look (fade) ----------------------------------------------

fn field_of(node: &Value, field: &str) -> Value {
    node["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["field"] == field)
        .unwrap_or_else(|| panic!("no field `{field}` in {node}"))
        .clone()
}

fn action_of(node: &Value, action: &str) -> Value {
    node["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["action"] == action)
        .unwrap_or_else(|| panic!("no action `{action}` in {node}"))
        .clone()
}

fn fade_amount(view: &Entity<Workstation>, cx: &mut VisualTestContext, clip: &str) -> f64 {
    view.read_with(cx, |view, _| {
        let attached = view.state.scenes.get("street").unwrap();
        let clip = attached
            .scene
            .bundle()
            .fx
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .find(|candidate| candidate.id == clip)
            .unwrap();
        match clip.param("amount").unwrap() {
            FxValue::Number(value) => value,
            other => panic!("amount is a number, got {other:?}"),
        }
    })
}

#[gpui_kit::test]
fn the_fade_is_listed_by_the_same_model_as_the_vignette_with_no_section_of_its_own(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mounted(cx);
    let section = node(&view, cx, "scene.fx");
    let names: Vec<&str> = section["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|action| action["action"].as_str().unwrap())
        .collect();
    // The plain adds are the catalog's, in its order. F062 offers the same effects again on the shot
    // the monitor shows (`add_in_shot:`) and nothing else: no section of its own for any of them.
    let plain: Vec<&str> = names
        .iter()
        .copied()
        .filter(|name| name.starts_with("add:"))
        .collect();
    assert_eq!(plain, ["add:vignette", "add:fade", "add:filmGrain"]);
    let in_shot: Vec<&str> = names
        .iter()
        .copied()
        .filter(|name| name.starts_with("add_in_shot:"))
        .collect();
    assert_eq!(
        in_shot,
        [
            "add_in_shot:vignette",
            "add_in_shot:fade",
            "add_in_shot:filmGrain"
        ]
    );
    assert_eq!(names.len(), plain.len() + in_shot.len());
    assert!(
        section["actions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|action| action["enabled"] == json!(true))
    );

    let depth = undo_depth(&view, cx);
    invoke(&view, cx, "scene.fx", "add:fade").unwrap();
    assert_eq!(undo_depth(&view, cx), depth + 1);
    let clip = node(&view, cx, CLIP);
    assert_eq!(clip["state"]["effect"], json!("fade"));
    let ids: Vec<String> = clip["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|field| field["field"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        ids,
        [
            "intensity",
            "color_r",
            "color_g",
            "color_b",
            "amount",
            "start_frame",
            "duration_frames",
            "fade_in_frames",
            "fade_out_frames",
            "hidden",
            "scope"
        ]
    );
    let amount = field_of(&clip, "amount");
    assert_eq!(
        (amount["min"].clone(), amount["max"].clone()),
        (json!(0.0), json!(1.0))
    );
    assert_eq!(amount["default"], json!(1.0));
    assert_eq!(amount["at_default"], json!(true));
    assert_eq!(amount["label"], json!("Fade Quantidade"));
    assert_eq!(
        clip["state"]["params"]["color"],
        json!({"r": 0.0, "g": 0.0, "b": 0.0})
    );
}

#[gpui_kit::test]
fn a_fade_edits_by_node_and_by_slider_and_a_reset_is_one_undo_entry(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    invoke(&view, cx, "scene.fx", "add:fade").unwrap();

    // Nothing away from the default yet: the reset is listed, disabled, with a reason.
    let clip = node(&view, cx, CLIP);
    let reset = action_of(&clip, "reset");
    assert_eq!(reset["enabled"], json!(false));
    assert!(reset["disabled_reason"].is_string());
    let depth = undo_depth(&view, cx);
    let noop = invoke(&view, cx, CLIP, "reset").unwrap();
    assert_eq!(noop["changed"], json!(false));
    assert_eq!(
        undo_depth(&view, cx),
        depth,
        "a reset with nothing to restore is no entry"
    );

    // A node edit and a component edit.
    set(&view, cx, CLIP, "amount", Raw::Number(0.4)).unwrap();
    set(&view, cx, CLIP, "color_r", Raw::Number(1.0)).unwrap();
    assert_eq!(fade_amount(&view, cx, "fx-1"), 0.4);
    let clip = node(&view, cx, CLIP);
    assert_eq!(
        clip["state"]["params"]["color"],
        json!({"r": 1.0, "g": 0.0, "b": 0.0})
    );
    assert_eq!(field_of(&clip, "amount")["at_default"], json!(false));
    assert_eq!(action_of(&clip, "reset:amount")["enabled"], json!(true));
    assert_eq!(undo_depth(&view, cx), depth + 2);

    // The real slider: one drag, one entry, quantized to the catalog step.
    let at = slider_point(&view, cx, "amount", 0.4);
    cx.simulate_mouse_move(at, None, Default::default());
    cx.simulate_mouse_down(at, MouseButton::Left, Default::default());
    let moved = slider_point(&view, cx, "amount", 0.8);
    cx.simulate_mouse_move(moved, MouseButton::Left, Default::default());
    cx.simulate_mouse_move(moved, MouseButton::Left, Default::default());
    assert_eq!(
        fade_amount(&view, cx, "fx-1"),
        0.4,
        "the document waits for the release"
    );
    assert!(gesture_open(&view, cx));
    cx.simulate_mouse_up(moved, MouseButton::Left, Default::default());
    cx.run_until_parked();
    let committed = fade_amount(&view, cx, "fx-1");
    assert!((committed - 0.8).abs() < 0.08, "{committed}");
    assert_eq!(
        (committed * 100.0).round() / 100.0,
        committed,
        "on the 0.01 step"
    );
    assert_eq!(undo_depth(&view, cx), depth + 3);

    // One field back, then the whole look back: each one entry, the timing untouched.
    let timing_before = node(&view, cx, CLIP)["state"]["duration_frames"].clone();
    invoke(&view, cx, CLIP, "reset:amount").unwrap();
    assert_eq!(fade_amount(&view, cx, "fx-1"), 1.0);
    assert_eq!(undo_depth(&view, cx), depth + 4);
    invoke(&view, cx, CLIP, "reset").unwrap();
    let clip = node(&view, cx, CLIP);
    assert_eq!(
        clip["state"]["params"]["color"],
        json!({"r": 0.0, "g": 0.0, "b": 0.0})
    );
    assert_eq!(undo_depth(&view, cx), depth + 5);
    assert_eq!(clip["state"]["duration_frames"], timing_before);

    // Undo puts the colour back; redo restores the reset.
    view.update(cx, |view, cx| view.execute(Command::Undo, Source::Cli, cx))
        .unwrap();
    assert_eq!(
        node(&view, cx, CLIP)["state"]["params"]["color"],
        json!({"r": 1.0, "g": 0.0, "b": 0.0})
    );
}

#[gpui_kit::test]
fn the_reset_button_runs_the_shared_handler_and_a_vignette_and_a_fade_compose_in_order(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mounted(cx);
    let add = center_of(&view, cx, "control:fx-add:fade");
    cx.simulate_click(add, Default::default());
    cx.run_until_parked();
    let add = center_of(&view, cx, "control:fx-add:vignette");
    cx.simulate_click(add, Default::default());
    cx.run_until_parked();
    assert_eq!(clips(&view, cx), 2);

    // The plan order is the track order: the fade was added first, so it is drawn first.
    let order = view.read_with(cx, |view, _| {
        view.state
            .scenes
            .get("street")
            .unwrap()
            .scene
            .bundle()
            .fx_passes(10)
            .iter()
            .map(|pass| pass.effect.clone())
            .collect::<Vec<_>>()
    });
    assert_eq!(order, ["fade".to_string(), "vignette".to_string()]);

    set(&view, cx, CLIP, "amount", Raw::Number(0.5)).unwrap();
    let depth = undo_depth(&view, cx);
    let button = center_of(&view, cx, "control:fx-reset:fx-1");
    cx.simulate_click(button, Default::default());
    cx.run_until_parked();
    assert_eq!(fade_amount(&view, cx, "fx-1"), 1.0);
    assert_eq!(undo_depth(&view, cx), depth + 1);
    assert_eq!(
        intensity(&view, cx, "fx-2"),
        0.7,
        "the other clip is untouched"
    );
}

#[gpui_kit::test]
fn a_reset_puts_every_channel_of_a_colour_back_not_only_the_last_one(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    invoke(&view, cx, "scene.fx", "add:fade").unwrap();
    for channel in ["color_r", "color_g", "color_b"] {
        set(&view, cx, CLIP, channel, Raw::Number(1.0)).unwrap();
    }
    set(&view, cx, CLIP, "amount", Raw::Number(0.5)).unwrap();
    let depth = undo_depth(&view, cx);
    invoke(&view, cx, CLIP, "reset").unwrap();
    assert_eq!(
        undo_depth(&view, cx),
        depth + 1,
        "one entry for the whole look"
    );
    let clip = node(&view, cx, CLIP);
    assert_eq!(
        clip["state"]["params"]["color"],
        json!({"r": 0.0, "g": 0.0, "b": 0.0})
    );
    assert_eq!(fade_amount(&view, cx, "fx-1"), 1.0);

    // The same for a point of a vignette: both axes come back with one reset.
    invoke(&view, cx, "scene.fx", "add:vignette").unwrap();
    let vignette = "scene.fx.clip:fx-2";
    set(&view, cx, vignette, "center_x", Raw::Number(0.1)).unwrap();
    set(&view, cx, vignette, "center_y", Raw::Number(0.9)).unwrap();
    invoke(&view, cx, vignette, "reset").unwrap();
    let clip = node(&view, cx, vignette);
    assert_eq!(
        clip["state"]["params"]["center"],
        json!({"x": 0.5, "y": 0.5})
    );
}

#[gpui_kit::test]
fn field_reset_matches_the_inspector_jsonl_op_and_reset_without_default_is_unknown(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mounted(cx);
    invoke(&view, cx, "scene.fx", "add:fade").unwrap();
    set(&view, cx, CLIP, "color_r", Raw::Number(0.2)).unwrap();
    set(&view, cx, CLIP, "color_g", Raw::Number(0.7)).unwrap();
    set(&view, cx, CLIP, "color_b", Raw::Number(0.8)).unwrap();
    set(&view, cx, CLIP, "duration_frames", Raw::Number(40.0)).unwrap();
    for field in ["color_g", "duration_frames"] {
        let info = view
            .update(cx, |view, _| {
                tessel_editor_core::headless::execute(
                    &mut view.state,
                    &json!({"op":"scene.fx.inspect","scene":"street","clip":"fx-1"}),
                )
            })
            .unwrap();
        let described = info["clips"][0]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["id"] == field)
            .unwrap();
        let op = described["reset"]["op"].clone();
        assert!(op.is_object(), "{field}: {info}");
        let depth = undo_depth(&view, cx);
        invoke(&view, cx, CLIP, &format!("reset:{field}")).unwrap();
        let gui = node(&view, cx, CLIP)["state"].clone();
        assert_eq!(undo_depth(&view, cx), depth + 1);
        view.update(cx, |view, cx| view.execute(Command::Undo, Source::Cli, cx))
            .unwrap();
        view.update(cx, |view, _| {
            tessel_editor_core::headless::execute(
                &mut view.state,
                &json!({"op":"scene.fx.edit","scene":"street","ops":[op]}),
            )
        })
        .unwrap();
        assert_eq!(
            node(&view, cx, CLIP)["state"],
            gui,
            "same reset via UI and JSONL: {field}"
        );
        assert_eq!(undo_depth(&view, cx), depth + 1);
    }
    let before = node(&view, cx, CLIP)["state"].clone();
    let depth = undo_depth(&view, cx);
    let error = invoke(&view, cx, CLIP, "reset:start_frame").unwrap_err();
    assert_eq!(error.code, "unknown_action");
    let actions = error.details.unwrap()["actions"].clone();
    let advertised = node(&view, cx, CLIP)["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|action| action["action"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        actions,
        json!(advertised),
        "unknown reset advertises the real actions"
    );
    let fallback = invoke(&view, cx, CLIP, "no-such-action").unwrap_err();
    assert_eq!(fallback.details.unwrap()["actions"], actions);
    assert_eq!(node(&view, cx, CLIP)["state"], before);
    assert_eq!(undo_depth(&view, cx), depth);
}
