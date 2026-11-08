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

fn mounted(cx: &mut TestAppContext) -> (Entity<Workstation>, &mut VisualTestContext) {
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

fn undo_depth(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> u64 {
    view.read_with(cx, |view, _| {
        view.state.history.to_json()["undo_depth"].as_u64().unwrap() - ATTACH_ENTRIES
    })
}

fn invoke(
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

fn set(
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

fn node(view: &Entity<Workstation>, cx: &mut VisualTestContext, id: &str) -> Value {
    view.read_with(cx, |view, cx| view.tree_node(id, cx))
        .unwrap_or_else(|| panic!("no semantic node `{id}`"))
}

fn field_value(node: &Value, field: &str) -> Value {
    node["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["field"] == field)
        .unwrap_or_else(|| panic!("no field `{field}` in {node}"))["value"]
        .clone()
}

fn clips(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> usize {
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
    assert_eq!(section["state"]["effects"], json!(["vignette"]));
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
            .fx
            .resolve(10)
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
        let fx = &view.state.scenes.get("street").unwrap().scene.bundle().fx;
        assert!(fx.resolve(10).is_empty(), "a hidden clip draws nothing");
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

fn center_of(view: &Entity<Workstation>, cx: &mut VisualTestContext, id: &str) -> Point<Pixels> {
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

fn clip_state(view: &Entity<Workstation>, cx: &mut VisualTestContext, clip: &str) -> Value {
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

    // The very end of the sequence is past this 96 frame edit when the sequence is longer: the
    // core refuses atomically and the panel says why.
    let past = view.update(cx, |view, _| {
        let length = view.state.timeline.length;
        view.state.timeline.set_playhead(length);
        view.state.scene_frame_at_playhead("street").is_err()
    });
    if past {
        let depth = undo_depth(&view, cx);
        let clips_before = clips(&view, cx);
        let error = invoke(&view, cx, "scene.fx", "add:vignette").unwrap_err();
        assert!(!error.code.is_empty());
        assert_eq!(undo_depth(&view, cx), depth);
        assert_eq!(clips(&view, cx), clips_before);
    }
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
            .fx
            .resolve(0)
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
