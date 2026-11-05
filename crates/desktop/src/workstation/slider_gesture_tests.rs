//! BL062 (F005/F024): a property-slider drag is ONE undo entry, driven by real GPUI input.
//!
//! Pointer tests press, move and release over the Kit slider of the "zoom" property through
//! the window's input path (`simulate_mouse_*`, `simulate_keystrokes`, `simulate_event`), so
//! they cover the Kit slider events, the capture-phase press, the Escape handler and the
//! window-activation observer. They do NOT prove a physical mouse (the OS never delivers a
//! MouseUp to a released-outside-window press the same way: BL043 stays open).
//!
//! The history rules themselves are tested without a window in
//! `crates/core/tests/slider_gesture.rs`.
use super::input_tests::fixture;
use super::*;
// The Kit's test macro expands to `#[test]`; the glob import above would resolve it to the Kit macro again.
use core::prelude::v1::test;

use crate::state::inspector::{Value as PropertyValue, property_def};

/// The zoom of the clip the fixture selects, as the inspector would show it.
const CLIP: &str = "atlantic-coast";
const ORIGINAL_ZOOM: f64 = 100.0;

fn zoom_of(view: &Entity<Workstation>, cx: &mut VisualTestContext, clip: &str) -> f64 {
    view.read_with(cx, |view, _| {
        let property = property_def("zoom").expect("zoom is a property");
        match view.state.inspector.value(clip, property) {
            PropertyValue::Number(number) => number,
            other => panic!("zoom is a number, got {other:?}"),
        }
    })
}

fn undo_depth(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> u64 {
    view.read_with(cx, |view, _| {
        view.state.history.to_json()["undo_depth"].as_u64().unwrap()
    })
}

fn redo_depth(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> u64 {
    view.read_with(cx, |view, _| {
        view.state.history.to_json()["redo_depth"].as_u64().unwrap()
    })
}

fn gesture_open(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |view, _| view.state.open_property_gesture().is_some())
}

fn revision(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> u64 {
    view.read_with(cx, |view, _| view.state.project.revision)
}

/// A point of the zoom slider track at `fraction` of its width, on its centre line.
fn track_point(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    fraction: f32,
) -> Point<Pixels> {
    let bounds = view.read_with(cx, |view, _| view.geometry.get("control:zoom").unwrap());
    point(
        bounds.left() + bounds.size.width * fraction,
        bounds.center().y,
    )
}

fn press(cx: &mut VisualTestContext, at: Point<Pixels>) {
    cx.simulate_mouse_move(at, None, Default::default());
    cx.simulate_mouse_down(at, MouseButton::Left, Default::default());
}

/// Moves the pressed pointer to `at`. The first move of a press only starts the Kit drag; the
/// second delivers the position (see `kit_slider_pointer_clamps_and_updates_shared_state`).
fn drag_to(cx: &mut VisualTestContext, at: Point<Pixels>) {
    cx.simulate_mouse_move(at, MouseButton::Left, Default::default());
    cx.simulate_mouse_move(at, MouseButton::Left, Default::default());
}

fn release(cx: &mut VisualTestContext, at: Point<Pixels>) {
    cx.simulate_mouse_up(at, MouseButton::Left, Default::default());
}

fn run(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    command: Command,
) -> Result<Value, CommandError> {
    view.update(cx, |view, cx| view.execute(command, Source::Cli, cx))
}

#[gpui_kit::test]
fn dragging_a_slider_previews_live_and_records_one_undo_entry(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let revision_before = revision(&view, cx);
    let at = track_point(&view, cx, 0.5);
    press(cx, at);
    for fraction in [0.55, 0.65, 0.75, 0.85] {
        let at = track_point(&view, cx, fraction);
        drag_to(cx, at);
    }

    // Live preview: the value already moved, the history did not.
    let previewed = zoom_of(&view, cx, CLIP);
    assert!(previewed > ORIGINAL_ZOOM, "preview shows {previewed}");
    assert!(gesture_open(&view, cx));
    assert_eq!(undo_depth(&view, cx), 0);
    assert!(
        revision(&view, cx) >= revision_before + 3,
        "the drag really produced several updates"
    );

    let at = track_point(&view, cx, 0.85);
    release(cx, at);
    assert!(!gesture_open(&view, cx));
    assert_eq!(undo_depth(&view, cx), 1);
    assert_eq!(zoom_of(&view, cx, CLIP), previewed);

    // One undo goes back to the value before the drag; one redo returns to its end.
    run(&view, cx, Command::Undo).unwrap();
    assert_eq!(zoom_of(&view, cx, CLIP), ORIGINAL_ZOOM);
    assert_eq!(undo_depth(&view, cx), 0);
    run(&view, cx, Command::Redo).unwrap();
    assert_eq!(zoom_of(&view, cx, CLIP), previewed);
}

#[gpui_kit::test]
fn a_track_click_is_one_gesture_with_one_undo_entry(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let target = track_point(&view, cx, 0.8);
    press(cx, target);
    release(cx, target);
    assert!(zoom_of(&view, cx, CLIP) > ORIGINAL_ZOOM);
    assert!(!gesture_open(&view, cx));
    assert_eq!(undo_depth(&view, cx), 1);
}

#[gpui_kit::test]
fn escape_during_a_slider_drag_restores_the_value_and_records_nothing(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let at = track_point(&view, cx, 0.5);
    press(cx, at);
    let at = track_point(&view, cx, 0.8);
    drag_to(cx, at);
    assert!(zoom_of(&view, cx, CLIP) > ORIGINAL_ZOOM);

    cx.simulate_keystrokes("escape");
    assert!(!gesture_open(&view, cx));
    assert_eq!(zoom_of(&view, cx, CLIP), ORIGINAL_ZOOM);

    // The pointer is still down: nothing it does now may start a second gesture or move the
    // value, and releasing records no entry.
    let at = track_point(&view, cx, 0.9);
    drag_to(cx, at);
    assert_eq!(zoom_of(&view, cx, CLIP), ORIGINAL_ZOOM);
    let at = track_point(&view, cx, 0.9);
    release(cx, at);
    assert_eq!(zoom_of(&view, cx, CLIP), ORIGINAL_ZOOM);
    assert_eq!(undo_depth(&view, cx), 0);
    assert!(!gesture_open(&view, cx));
}

#[gpui_kit::test]
fn dragging_back_to_the_original_value_records_no_empty_step(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    // Press exactly on the value the clip already has, move away and come back.
    let origin = track_point(&view, cx, (ORIGINAL_ZOOM as f32 - 10.0) / 390.0);
    press(cx, origin);
    let at = track_point(&view, cx, 0.9);
    drag_to(cx, at);
    assert!(zoom_of(&view, cx, CLIP) > ORIGINAL_ZOOM);
    drag_to(cx, origin);
    release(cx, origin);
    assert_eq!(zoom_of(&view, cx, CLIP), ORIGINAL_ZOOM);
    assert_eq!(undo_depth(&view, cx), 0);
    assert!(!gesture_open(&view, cx));
}

/// BL043: a MouseUp that never arrives (released over another window) must not leave the
/// gesture open for the rest of the session. The next press settles it as released.
#[gpui_kit::test]
fn a_lost_mouse_up_is_settled_by_the_next_press(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let at = track_point(&view, cx, 0.5);
    press(cx, at);
    let at = track_point(&view, cx, 0.7);
    drag_to(cx, at);
    let first = zoom_of(&view, cx, CLIP);
    assert!(gesture_open(&view, cx));
    // No release. The pointer comes back with no button down; the next press settles it.
    let returned = track_point(&view, cx, 0.2);
    cx.simulate_mouse_move(returned, None, Default::default());

    let second_press = track_point(&view, cx, 0.3);
    cx.simulate_mouse_down(second_press, MouseButton::Left, Default::default());
    assert_eq!(undo_depth(&view, cx), 1, "the first drag was committed");
    // A press on the thumb emits no Change; the new gesture starts on its first value.
    let next_value = track_point(&view, cx, 0.4);
    drag_to(cx, next_value);
    assert!(gesture_open(&view, cx), "the new drag owns its own gesture");
    release(cx, next_value);
    assert_eq!(undo_depth(&view, cx), 2);
    assert!(zoom_of(&view, cx, CLIP) < first);
}

/// The window may be deactivated mid-drag; the OS then never sends the MouseUp to us.
#[gpui_kit::test]
fn losing_the_window_focus_commits_the_gesture_and_ignores_the_rest_of_the_press(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    let at = track_point(&view, cx, 0.5);
    press(cx, at);
    let at = track_point(&view, cx, 0.7);
    drag_to(cx, at);
    let previewed = zoom_of(&view, cx, CLIP);
    assert!(gesture_open(&view, cx));

    cx.deactivate_window();
    assert!(!gesture_open(&view, cx));
    assert_eq!(undo_depth(&view, cx), 1);

    let at = track_point(&view, cx, 0.9);
    drag_to(cx, at);
    assert_eq!(zoom_of(&view, cx, CLIP), previewed);
    let at = track_point(&view, cx, 0.9);
    release(cx, at);
    assert_eq!(undo_depth(&view, cx), 1);
}

#[gpui_kit::test]
fn changing_the_selection_mid_drag_never_applies_the_value_to_another_clip(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    let other_before = zoom_of(&view, cx, "river-detail");
    let at = track_point(&view, cx, 0.5);
    press(cx, at);
    let at = track_point(&view, cx, 0.6);
    drag_to(cx, at);

    run(
        &view,
        cx,
        Command::SelectClip {
            clip: "river-detail".into(),
        },
    )
    .unwrap();
    cx.run_until_parked();
    let at = track_point(&view, cx, 0.9);
    drag_to(cx, at);
    let at = track_point(&view, cx, 0.9);
    release(cx, at);

    assert_eq!(zoom_of(&view, cx, "river-detail"), other_before);
    assert!(zoom_of(&view, cx, CLIP) > ORIGINAL_ZOOM);
    assert_eq!(undo_depth(&view, cx), 1);
    assert!(!gesture_open(&view, cx));
}

#[gpui_kit::test]
fn undo_during_a_drag_settles_it_and_later_pointer_values_are_dropped(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let at = track_point(&view, cx, 0.5);
    press(cx, at);
    let at = track_point(&view, cx, 0.7);
    drag_to(cx, at);

    // The core settles the gesture (one entry), then undoes it: back at the original.
    run(&view, cx, Command::Undo).unwrap();
    assert_eq!(zoom_of(&view, cx, CLIP), ORIGINAL_ZOOM);
    assert_eq!(undo_depth(&view, cx), 0);
    assert_eq!(redo_depth(&view, cx), 1);

    // The pointer keeps dragging: those values belong to a gesture that no longer exists and
    // must neither re-apply nor start a new entry (and the redo stays available).
    let at = track_point(&view, cx, 0.9);
    drag_to(cx, at);
    let at = track_point(&view, cx, 0.9);
    release(cx, at);
    assert_eq!(zoom_of(&view, cx, CLIP), ORIGINAL_ZOOM);
    assert_eq!(undo_depth(&view, cx), 0);
    assert_eq!(redo_depth(&view, cx), 1);
}

#[gpui_kit::test]
fn keyboard_steps_commit_on_key_up_and_a_held_key_is_one_entry(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    // A click focuses the slider wrapper (the track is not the thumb, which swallows presses).
    let target = track_point(&view, cx, 0.5);
    press(cx, target);
    release(cx, target);
    let after_click = zoom_of(&view, cx, CLIP);
    assert_eq!(undo_depth(&view, cx), 1);

    let right = Keystroke::parse("right").unwrap();
    cx.simulate_keystrokes("right");
    assert!(gesture_open(&view, cx), "a key press begins a gesture");
    assert_eq!(
        undo_depth(&view, cx),
        1,
        "nothing recorded while the key is down"
    );
    for _ in 0..2 {
        cx.simulate_event(KeyDownEvent {
            keystroke: right.clone(),
            is_held: true,
            prefer_character_input: false,
        });
    }
    assert!(
        gesture_open(&view, cx),
        "auto-repeat continues the same gesture"
    );
    assert_eq!(undo_depth(&view, cx), 1);

    cx.simulate_event(KeyUpEvent {
        keystroke: right.clone(),
    });
    assert!(!gesture_open(&view, cx));
    assert_eq!(undo_depth(&view, cx), 2, "the whole key hold is one entry");
    assert!(zoom_of(&view, cx, CLIP) > after_click);

    // A second tap is its own entry.
    cx.simulate_keystrokes("right");
    cx.simulate_event(KeyUpEvent { keystroke: right });
    assert_eq!(undo_depth(&view, cx), 3);
}

#[gpui_kit::test]
fn releasing_a_modifier_does_not_end_a_keyboard_gesture(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let target = track_point(&view, cx, 0.5);
    press(cx, target);
    release(cx, target);

    cx.simulate_keystrokes("shift-right");
    assert!(gesture_open(&view, cx));
    cx.simulate_event(KeyUpEvent {
        keystroke: Keystroke::parse("shift").unwrap(),
    });
    assert!(gesture_open(&view, cx), "only the step key ends it");
    cx.simulate_event(KeyUpEvent {
        keystroke: Keystroke::parse("right").unwrap(),
    });
    assert!(!gesture_open(&view, cx));
}

/// Adapter-level view of the same keyboard contract, without depending on which element
/// holds the focus: press, auto-repeat and release by the handlers the wrapper installs.
#[gpui_kit::test]
fn the_adapter_commits_a_value_stream_once_on_release(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    view.update(cx, |view, cx| {
        view.slider_pressed(cx);
        view.slider_changed("zoom", 101.0, cx);
        view.slider_changed("zoom", 102.0, cx);
        view.slider_changed("zoom", 103.0, cx);
    });
    assert!(gesture_open(&view, cx));
    assert_eq!(undo_depth(&view, cx), 0);
    view.update(cx, |view, cx| view.slider_released("zoom", cx));
    assert!(!gesture_open(&view, cx));
    assert_eq!(undo_depth(&view, cx), 1);
    assert_eq!(zoom_of(&view, cx, CLIP), 103.0);
    // A release nobody is waiting for is harmless.
    view.update(cx, |view, cx| view.slider_released("zoom", cx));
    assert_eq!(undo_depth(&view, cx), 1);
}

#[gpui_kit::test]
fn a_discrete_set_property_still_records_one_entry(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    run(
        &view,
        cx,
        Command::SetProperty {
            property: "zoom".into(),
            value: Raw::Number(140.0),
        },
    )
    .unwrap();
    assert_eq!(undo_depth(&view, cx), 1);
    assert_eq!(zoom_of(&view, cx, CLIP), 140.0);
}

#[gpui_kit::test]
fn pressing_again_clears_the_tracked_interaction(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    view.update(cx, |view, cx| {
        view.slider_changed("zoom", 120.0, cx);
        assert!(view.slider_gesture.is_tracking());
        view.slider_pressed(cx);
        assert!(!view.slider_gesture.is_tracking());
    });
    assert_eq!(undo_depth(&view, cx), 1);
}
