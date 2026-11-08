//! Regression tests drive actual GPUI input routing, not direct state commands.
use super::*;
// The Kit's test macro expands to `#[test]`; the glob import above would resolve it to the Kit macro again.
use core::prelude::v1::test;

/// Mounts the workstation the way the app does: Kit initialised, dark theme, the
/// real dock around the timeline whose handles the tests drag.
pub(super) fn fixture(cx: &mut TestAppContext) -> (Entity<Workstation>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_perf_meter(cx);
        crate::theme::apply(cx, true);
    });
    let (view, cx) = cx.add_window_view(Workstation::new);
    cx.run_until_parked();
    (view, cx)
}

#[gpui_kit::test]
fn dragging_trim_handle_changes_duration(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let (start, end) = view.read_with(cx, |view, _| {
        view.trim_gesture_points("trim:first-light:end", 120)
            .unwrap()
    });
    cx.simulate_mouse_move(start, None, Default::default());
    cx.simulate_mouse_down(start, MouseButton::Left, Default::default());
    for point in [start + (end - start) * 0.5, end, end] {
        cx.simulate_mouse_move(point, MouseButton::Left, Default::default());
    }
    cx.simulate_mouse_up(end, MouseButton::Left, Default::default());
    assert_eq!(
        view.read_with(cx, |view, _| view
            .state
            .timeline
            .clip("first-light")
            .unwrap()
            .duration),
        120
    );
}

#[gpui_kit::test]
fn dragging_trim_handle_with_one_motion_commits_on_release(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let (start, end) = view.read_with(cx, |view, _| {
        view.trim_gesture_points("trim:first-light:end", 120)
            .unwrap()
    });
    cx.simulate_mouse_move(start, None, Default::default());
    cx.simulate_mouse_down(start, MouseButton::Left, Default::default());
    cx.simulate_mouse_move(end, MouseButton::Left, Default::default());
    cx.simulate_mouse_up(end, MouseButton::Left, Default::default());
    assert_eq!(
        view.read_with(cx, |view, _| view
            .state
            .timeline
            .clip("first-light")
            .unwrap()
            .duration),
        120
    );
}

#[gpui_kit::test]
fn left_trim_handle_preserves_the_end(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let (start, end) = view.read_with(cx, |view, _| {
        view.trim_gesture_points("trim:first-light:start", 24)
            .unwrap()
    });
    cx.simulate_mouse_move(start, None, Default::default());
    cx.simulate_mouse_down(start, MouseButton::Left, Default::default());
    cx.simulate_mouse_move(end, MouseButton::Left, Default::default());
    cx.simulate_mouse_up(end, MouseButton::Left, Default::default());
    let clip = view.read_with(cx, |view, _| {
        view.state.timeline.clip("first-light").unwrap().clone()
    });
    assert_eq!((clip.start, clip.end()), (24, 192));
}

#[gpui_kit::test]
fn escape_cancels_handle_drag_even_when_released_over_timeline(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let (start, end) = view.read_with(cx, |view, _| {
        view.trim_gesture_points("trim:first-light:end", 120)
            .unwrap()
    });
    cx.simulate_mouse_move(start, None, Default::default());
    cx.simulate_mouse_down(start, MouseButton::Left, Default::default());
    cx.simulate_mouse_move(end, MouseButton::Left, Default::default());
    cx.simulate_mouse_move(end, MouseButton::Left, Default::default());
    cx.simulate_keystrokes("escape");
    cx.simulate_mouse_up(end, MouseButton::Left, Default::default());
    assert_eq!(
        view.read_with(cx, |view, _| view
            .state
            .timeline
            .clip("first-light")
            .unwrap()
            .duration),
        192
    );
}

/// A point on the ruler (the first 24 px of the timeline content) over `frame`.
fn ruler_point(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    frame: i64,
) -> Point<Pixels> {
    view.read_with(cx, |view, _| {
        let content = view
            .geometry
            .get("timeline-content")
            .expect("timeline is rendered");
        point(
            content.origin.x + px(frame as f32 * view.state.timeline.pixels_per_frame()),
            content.origin.y + px(10.),
        )
    })
}

fn playhead(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> i64 {
    view.read_with(cx, |view, _| view.state.timeline.playhead)
}

#[gpui_kit::test]
fn dragging_on_the_ruler_follows_the_pressed_pointer(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let (from, middle, to) = (
        ruler_point(&view, cx, 10),
        ruler_point(&view, cx, 25),
        ruler_point(&view, cx, 40),
    );
    cx.simulate_mouse_move(from, None, Default::default());
    cx.simulate_mouse_down(from, MouseButton::Left, Default::default());
    for position in [middle, to] {
        cx.simulate_mouse_move(position, MouseButton::Left, Default::default());
    }
    assert_eq!(playhead(&view, cx), 40);
    cx.simulate_mouse_up(to, MouseButton::Left, Default::default());
    assert_eq!(playhead(&view, cx), 40);
}

/// B6 (docs/viewport-bugfix-report.md): GPUI hands every mouse move to `on_drag_move` while a
/// drag value exists, with or without a pressed button, and only a MouseUp clears it. When that
/// MouseUp never arrives (released over another window, an injected gesture that stops
/// halfway), later hover moves used to drag the playhead.
#[gpui_kit::test]
fn moves_without_a_pressed_button_do_not_scrub_after_a_lost_mouse_up(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let (from, middle, to, away) = (
        ruler_point(&view, cx, 10),
        ruler_point(&view, cx, 25),
        ruler_point(&view, cx, 40),
        ruler_point(&view, cx, 90),
    );
    cx.simulate_mouse_move(from, None, Default::default());
    cx.simulate_mouse_down(from, MouseButton::Left, Default::default());
    // The move that crosses GPUI's drag threshold only starts the drag; the next one scrubs.
    cx.simulate_mouse_move(middle, MouseButton::Left, Default::default());
    cx.simulate_mouse_move(to, MouseButton::Left, Default::default());
    assert_eq!(playhead(&view, cx), 40, "the pressed drag scrubs");

    // No MouseUp: the pointer keeps moving with no button held.
    cx.simulate_mouse_move(away, None, Default::default());
    assert_eq!(playhead(&view, cx), 40, "a hover move is not a scrub");
    // The stale drag is gone, not just ignored once: a later hover over the ruler stays inert.
    let hover = ruler_point(&view, cx, 70);
    cx.simulate_mouse_move(hover, None, Default::default());
    assert_eq!(playhead(&view, cx), 40);
    // And a fresh press scrubs again.
    let again = ruler_point(&view, cx, 20);
    cx.simulate_mouse_down(again, MouseButton::Left, Default::default());
    assert_eq!(playhead(&view, cx), 20);
}

#[gpui_kit::test]
fn kit_select_keyboard_confirmation_updates_shared_state(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let select = view.read_with(cx, |view, _| view.selects["blend"].clone());
    cx.update(|window, cx| select.update(cx, |select, cx| select.focus(window, cx)));
    cx.simulate_keystrokes("enter down enter");
    let value = view.read_with(cx, |view, _| {
        let property = crate::state::inspector::PROPERTIES
            .iter()
            .find(|property| property.id == "blend")
            .unwrap();
        view.state.inspector.value("atlantic-coast", property)
    });
    assert_eq!(value, crate::state::inspector::Value::Choice("multiply"));
}

#[gpui_kit::test]
fn kit_select_escape_discards_unconfirmed_keyboard_choice(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let select = view.read_with(cx, |view, _| view.selects["blend"].clone());
    cx.update(|window, cx| select.update(cx, |select, cx| select.focus(window, cx)));
    cx.simulate_keystrokes("enter down escape");
    let value = view.read_with(cx, |view, _| {
        let property = crate::state::inspector::PROPERTIES
            .iter()
            .find(|property| property.id == "blend")
            .unwrap();
        view.state.inspector.value("atlantic-coast", property)
    });
    assert_eq!(value, crate::state::inspector::Value::Choice("normal"));
    assert_eq!(
        select.read_with(cx, |select, _| select.selected_value().copied()),
        Some("normal")
    );
}

#[gpui_kit::test]
fn kit_slider_pointer_clamps_and_updates_shared_state(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let bounds = view.read_with(cx, |view, _| view.geometry.get("control:zoom").unwrap());
    let start = bounds.center();
    for (x, expected) in [
        (bounds.right() + px(50.), 400.0),
        (bounds.left() - px(50.), 10.0),
    ] {
        let end = point(x, start.y);
        cx.simulate_mouse_move(start, None, Default::default());
        cx.simulate_mouse_down(start, MouseButton::Left, Default::default());
        // The first move starts the Kit drag; the next delivers its position.
        cx.simulate_mouse_move(end, MouseButton::Left, Default::default());
        cx.simulate_mouse_move(end, MouseButton::Left, Default::default());
        cx.simulate_mouse_up(end, MouseButton::Left, Default::default());
        let value = view.read_with(cx, |view, _| {
            let property = crate::state::inspector::PROPERTIES
                .iter()
                .find(|property| property.id == "zoom")
                .unwrap();
            view.state.inspector.value("atlantic-coast", property)
        });
        assert_eq!(value, crate::state::inspector::Value::Number(expected));
    }
}
