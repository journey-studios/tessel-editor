//! F016: the "Efeitos" section of the scene inspector, driven through its semantic nodes and by
//! real GPUI pointer input over the Kit slider. The rules (catalog, ranges, overlap, envelope,
//! atomic refusal) are the core's and are tested in `crates/core/tests/scene_fx.rs`; these tests
//! prove the window reaches them: one command and one undo entry per gesture, the typed refusal on
//! screen, and nothing left behind by a refused or cancelled gesture.
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
        json!(["scene.fx.clip:fx-1"])
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
    // During the drag the document and the history do not move; the buffer holds the value.
    assert_eq!(intensity(&view, cx, "fx-1"), 0.7);
    assert_eq!(undo_depth(&view, cx), depth);
    assert_eq!(view.read_with(cx, |view, _| view.scene_ui.fx.pending()), 1);

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
    assert_eq!(view.read_with(cx, |view, _| view.scene_ui.fx.pending()), 0);
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
    assert_eq!(view.read_with(cx, |view, _| view.scene_ui.fx.pending()), 1);

    cx.simulate_keystrokes("escape");
    assert_eq!(view.read_with(cx, |view, _| view.scene_ui.fx.pending()), 0);
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
        json!(["scene.fx.clip:fx-2"])
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

fn hold(view: &Entity<Workstation>, cx: &mut VisualTestContext, key: FxKey, value: f64) {
    view.update(cx, |view, _| view.scene_ui.fx.hold(key, value));
}

fn pending(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |view, _| view.scene_ui.fx.pending())
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
fn a_drag_buffer_does_not_follow_the_panel_to_another_scene(cx: &mut TestAppContext) {
    let (view, cx) = two_scenes(cx);
    let depth = total_depth(&view, cx);

    hold(&view, cx, key("street", "fx-1", "intensity"), 0.2);
    assert_eq!(pending(&view, cx), 1);
    // The same scene again is not a move: the drag is still the author's.
    choose(&view, cx, "street");
    assert_eq!(
        pending(&view, cx),
        1,
        "choosing the scene already shown moves nothing"
    );

    choose(&view, cx, "yard");
    assert_eq!(
        pending(&view, cx),
        0,
        "the gesture of `street` was dropped with the panel's move"
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
fn a_commit_is_refused_for_a_scene_the_panel_no_longer_shows_even_if_a_clip_of_that_id_exists_there(
    cx: &mut TestAppContext,
) {
    let (view, cx) = two_scenes(cx);
    let depth = total_depth(&view, cx);

    // The panel moves without telling the section (the commit's own guard is what is under test).
    hold(&view, cx, key("street", "fx-1", "intensity"), 0.2);
    view.update(cx, |view, _| view.scene_ui.set_chosen_for_test("yard"));
    blur(&view, cx);

    assert_eq!(
        intensity_in(&view, cx, "yard", "fx-1"),
        0.7,
        "`yard` has an `fx-1` and must not be edited"
    );
    assert_eq!(intensity_in(&view, cx, "street", "fx-1"), 0.7);
    assert_eq!(total_depth(&view, cx), depth);
    assert_eq!(pending(&view, cx), 0);
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

    // A drag buffer that outlived its clip (the clip was removed by the CLI during the gesture).
    hold(&view, cx, key("street", "fx-1", "intensity"), 0.2);
    blur(&view, cx);

    assert_eq!(pending(&view, cx), 0);
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
    assert_eq!(pending(&view, cx), 1, "the drag is in `street`");

    choose(&view, cx, "yard");
    assert_eq!(pending(&view, cx), 0);
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
    hold(
        &view,
        cx,
        key("street", "fx-1", "intensity"),
        f64::from(0.7_f32),
    );
    blur(&view, cx);
    assert_eq!(undo_depth(&view, cx), depth, "0.7 -> 0.7 is no edit");
    assert_eq!(panel_notice(&view, cx), Value::Null);
    assert_eq!(intensity(&view, cx, "fx-1"), 0.7);

    // The value stored is the step's, not the f32's.
    hold(
        &view,
        cx,
        key("street", "fx-1", "intensity"),
        f64::from(0.4_f32),
    );
    blur(&view, cx);
    assert_eq!(intensity(&view, cx, "fx-1"), 0.4);
    assert_eq!(undo_depth(&view, cx), depth + 1);

    // A thumb past the end is the range's end, not a refusal.
    hold(&view, cx, key("street", "fx-1", "intensity"), 1.7);
    blur(&view, cx);
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
