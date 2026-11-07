//! F049: the debug performance meter, driven through the same entry points as the shortcut, the
//! header button and `ui invoke|set`.
//!
//! GPUI's test platform lays out and routes input but paints no pixels, so these tests prove
//! geometry (where the card is relative to the native view's rectangle and to the panels),
//! counting (what the meter treats as a frame) and plumbing; the colours and glyphs are judged by
//! the native screenshots of the live proof (`docs/features/perf-meter.md`).
use std::time::Duration;

use super::input_tests::fixture;
use super::perf_meter::TogglePerfMeter;
use super::*;
use gpui_kit::component::ActiveTheme;
// The Kit's test macro expands to `#[test]`; the glob import above would resolve it to the Kit macro again.
use core::prelude::v1::test;
use serde_json::{Value, json};
use tessel_editor_core::commands::{Parsed, parse_set};
use tessel_editor_core::perf::{
    self, CARD_HEIGHT, CARD_MARGIN, CARD_WIDTH, PerfMeterCommand, Rect,
};

const NODE: &str = "toolbar.perf_meter";

/// Draws until the layout stops moving: the test platform has no frame loop of its own.
fn settle(cx: &mut VisualTestContext) {
    for _ in 0..4 {
        cx.run_until_parked();
        cx.update(|window, cx| window.simulate_next_frame(cx));
    }
    cx.run_until_parked();
}

/// One frame the window really paints.
fn paint(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.refresh();
        window.simulate_next_frame(cx);
    });
    cx.run_until_parked();
}

fn toggle(view: &Entity<Workstation>, cx: &mut VisualTestContext, command: PerfMeterCommand) {
    view.update(cx, |view, cx| {
        view.execute_perf(command, Source::Cli, cx).unwrap();
    });
    settle(cx);
}

fn node(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Value {
    view.read_with(cx, |view, cx| view.tree_node(NODE, cx).unwrap())
}

fn samples(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> (usize, usize) {
    view.read_with(cx, |view, _| view.perf.meter().borrow().samples())
}

fn hide(view: &Entity<Workstation>, cx: &mut VisualTestContext, panel: PanelKind) {
    view.update_in(cx, |view, window, cx| {
        view.execute_layout(
            LayoutCommand::SetVisible {
                panel,
                visible: false,
            },
            Source::Cli,
            window,
            cx,
        )
        .unwrap();
    });
    settle(cx);
}

/// The card's rectangle in window coordinates, when the node says it floats.
fn card_rect(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Option<Rect> {
    let placement = node(view, cx)["state"]["placement"].clone();
    let y = placement["y"].as_f64()? as f32;
    let area = view.read_with(cx, |view, _| view.geometry.get("perf.area").unwrap());
    Some(Rect::new(
        f32::from(area.origin.x + area.size.width) - CARD_MARGIN - CARD_WIDTH,
        f32::from(area.origin.y) + y,
        CARD_WIDTH,
        CARD_HEIGHT,
    ))
}

fn native_view(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Rect {
    let bounds = view.read_with(cx, |view, _| view.geometry.get("viewport.frame").unwrap());
    Rect::new(
        f32::from(bounds.origin.x),
        f32::from(bounds.origin.y),
        f32::from(bounds.size.width),
        f32::from(bounds.size.height),
    )
}

fn regions(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Vec<(String, [f32; 4])> {
    let mut regions: Vec<_> = view
        .read_with(cx, |view, _| view.geometry.regions())
        .into_iter()
        .collect();
    regions.sort_by(|a, b| a.0.cmp(&b.0));
    regions
}

// ---- the shortcut

#[gpui_kit::test]
fn the_shortcut_is_bound_once_and_no_other_binding_uses_it(cx: &mut TestAppContext) {
    let (_view, cx) = fixture(cx);
    let wanted = Keystroke::parse(perf::SHORTCUT).unwrap();
    cx.update(|_, cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let same_keys: Vec<String> = keymap
            .bindings()
            .filter(|binding| {
                binding.keystrokes().len() == 1
                    && binding.keystrokes()[0].key() == wanted.key
                    && *binding.keystrokes()[0].modifiers() == wanted.modifiers
            })
            .map(|binding| binding.action().name().to_string())
            .collect();
        assert_eq!(
            same_keys,
            vec![TogglePerfMeter.name().to_string()],
            "{} must be the meter's alone",
            perf::SHORTCUT
        );
    });
}

#[gpui_kit::test]
fn the_shortcut_toggles_the_meter_whatever_the_focus_and_says_so_in_the_status_bar(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    assert!(!view.read_with(cx, |view, _| view.perf_visible()));
    cx.simulate_keystrokes(perf::SHORTCUT);
    assert!(view.read_with(cx, |view, _| view.perf_visible()));
    assert_eq!(node(&view, cx)["state"]["visible"], json!(true));
    let said = view.read_with(cx, |view, _| view.activity.as_ref().unwrap().text.clone());
    assert!(
        said.contains("ligado") && said.contains(shortcut_label()),
        "{said}"
    );
    cx.simulate_keystrokes(perf::SHORTCUT);
    assert!(!view.read_with(cx, |view, _| view.perf_visible()));
}

#[gpui_kit::test]
fn the_node_the_cli_reads_and_the_shortcut_share_one_state(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let hidden = node(&view, cx);
    assert_eq!(hidden["state"]["visible"], json!(false));
    assert_eq!(hidden["state"]["shortcut"], json!(perf::SHORTCUT));
    assert_eq!(
        hidden["state"]["readout"],
        Value::Null,
        "nothing is collected while hidden"
    );
    assert_eq!(hidden["fields"][0]["field"], "visible");

    // `ui set toolbar.perf_meter visible true`: the node declares the field, the parser maps it,
    // and the very same handler as the shortcut runs.
    let command = parse_set(
        NODE,
        "visible",
        &tessel_editor_core::Raw::from_json(&json!(true)).unwrap(),
    )
    .unwrap();
    let Parsed::PerfMeter(command) = command else {
        panic!("not a meter command")
    };
    toggle(&view, cx, command);
    let shown = node(&view, cx);
    assert_eq!(shown["state"]["visible"], json!(true));
    assert!(shown["state"]["readout"]["ui"]["state"].is_string());
    assert_eq!(shown["state"]["readout"]["monitor"]["state"], "unavailable");
    cx.simulate_keystrokes(perf::SHORTCUT);
    assert_eq!(node(&view, cx)["state"]["visible"], json!(false));
}

// ---- cost

#[gpui_kit::test]
fn a_hidden_meter_adds_nothing_to_the_window_and_records_nothing(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    for _ in 0..6 {
        paint(cx);
    }
    assert_eq!(samples(&view, cx), (0, 0));
    let (wakeup, enabled, armed) = view.read_with(cx, |view, _| {
        (
            view.perf
                .meter()
                .borrow()
                .next_wakeup(Duration::from_secs(5)),
            view.perf.meter().borrow().enabled(),
            view.perf.wake_armed(),
        )
    });
    assert_eq!(
        (wakeup, enabled, armed),
        (None, false, false),
        "no timer, no collection"
    );
    // No probe either: the area is only measured while the meter is shown.
    assert!(view.read_with(cx, |view, _| view.geometry.get("perf.area").is_none()));
}

#[gpui_kit::test]
fn the_ui_rate_counts_frames_the_window_paints_and_not_timer_ticks(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    toggle(&view, cx, PerfMeterCommand::Show);
    let base = samples(&view, cx).0;
    for _ in 0..5 {
        paint(cx);
    }
    assert_eq!(samples(&view, cx).0, base + 5, "five paints, five frames");

    // Time passing with nothing painting is not a frame, whatever fires meanwhile.
    cx.executor().advance_clock(Duration::from_secs(3));
    cx.run_until_parked();
    assert_eq!(samples(&view, cx).0, base + 5, "a timer is not a frame");

    // Once the window rested for real (the meter reads the wall clock, as the app does), the one
    // wake-up redraws the card to say "parado" and that redraw is not counted either.
    std::thread::sleep(Duration::from_millis(1150));
    cx.executor().advance_clock(Duration::from_secs(3));
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
    });
    cx.run_until_parked();
    assert_eq!(
        samples(&view, cx).0,
        base + 5,
        "the rest redraw is the meter's own"
    );
    assert_eq!(node(&view, cx)["state"]["readout"]["ui"]["state"], "idle");
    assert_eq!(
        view.read_with(cx, |view, _| view
            .perf
            .meter()
            .borrow()
            .next_wakeup(Duration::from_secs(9))),
        None,
        "at rest nothing is armed: the window can sleep"
    );
}

#[gpui_kit::test]
fn the_numbers_are_recomputed_at_four_hertz_at_most(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    toggle(&view, cx, PerfMeterCommand::Show);
    let first = view.read_with(cx, |view, _| view.perf.shown_at().unwrap());
    for _ in 0..8 {
        paint(cx);
    }
    // Eight more frames inside the refresh interval did not recompute the text.
    let now = view.read_with(cx, |view, _| view.perf.epoch().elapsed());
    let latest = view.read_with(cx, |view, _| view.perf.shown_at().unwrap());
    if now.saturating_sub(first) < perf::REFRESH_EVERY {
        assert_eq!(latest, first);
    } else {
        assert!(latest.saturating_sub(first) >= perf::REFRESH_EVERY);
    }
}

#[gpui_kit::test]
fn hiding_forgets_the_samples(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    toggle(&view, cx, PerfMeterCommand::Show);
    paint(cx);
    assert!(samples(&view, cx).0 > 0);
    toggle(&view, cx, PerfMeterCommand::Hide);
    for _ in 0..3 {
        paint(cx);
    }
    assert_eq!(samples(&view, cx), (0, 0));
}

// ---- where it floats

#[gpui_kit::test]
fn by_default_the_card_floats_at_the_right_edge_clear_of_the_native_view(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    toggle(&view, cx, PerfMeterCommand::Show);
    let card = card_rect(&view, cx).expect("a floating card");
    let native = native_view(&view, cx);
    assert!(
        !card.intersects(&native),
        "{card:?} hides behind the native view {native:?}"
    );
    let area = view.read_with(cx, |view, _| view.geometry.get("perf.area").unwrap());
    assert_eq!(
        card.right(),
        f32::from(area.origin.x + area.size.width) - CARD_MARGIN
    );
}

#[gpui_kit::test]
fn with_the_inspector_closed_the_card_moves_out_of_the_native_view_instead_of_hiding(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    toggle(&view, cx, PerfMeterCommand::Show);
    hide(&view, cx, PanelKind::Inspector);
    let native = native_view(&view, cx);
    let area = view.read_with(cx, |view, _| view.geometry.get("perf.area").unwrap());
    // The monitor now reaches the right edge: the corner is the native view's.
    assert!(
        native.right() > f32::from(area.size.width) - CARD_WIDTH - CARD_MARGIN,
        "the inspector's closing must have widened the monitor"
    );
    // The timeline's end is free of native views: the card sits at the bottom of the area.
    let card = card_rect(&view, cx).expect("a floating card");
    assert!(
        !card.intersects(&native),
        "{card:?} hides behind the native view {native:?}"
    );
    assert!(
        card.y > native.bottom(),
        "{card:?} is below the picture {native:?}"
    );
}

#[gpui_kit::test]
fn with_the_monitor_maximized_the_meter_becomes_a_status_bar_strip(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    toggle(&view, cx, PerfMeterCommand::Show);
    for panel in [
        PanelKind::Library,
        PanelKind::Inspector,
        PanelKind::Timeline,
    ] {
        hide(&view, cx, panel);
    }
    assert_eq!(
        node(&view, cx)["state"]["placement"],
        json!({"kind": "status_bar_strip"}),
        "nothing is left to float over: the strip is the only place the native view cannot cover"
    );
    // Bringing a panel back gives the corner back.
    view.update_in(cx, |view, window, cx| {
        view.execute_layout(
            LayoutCommand::SetVisible {
                panel: PanelKind::Inspector,
                visible: true,
            },
            Source::Cli,
            window,
            cx,
        )
        .unwrap();
    });
    settle(cx);
    assert_eq!(node(&view, cx)["state"]["placement"]["kind"], "floating");
}

#[gpui_kit::test]
fn the_meter_displaces_no_panel(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    settle(cx);
    let without = regions(&view, cx);
    toggle(&view, cx, PerfMeterCommand::Show);
    assert_eq!(regions(&view, cx), without);
    assert!(!without.is_empty());
}

// ---- the pointer

/// A card over a button: the click reaches the button.
struct Underneath {
    clicks: usize,
    readout: perf::Readout,
}

impl Render for Underneath {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Tokens::new(cx);
        let mono = cx.theme().mono_font_family.clone();
        div()
            .relative()
            .size_full()
            .child(
                div()
                    .id("under")
                    .absolute()
                    .top(px(0.))
                    .left(px(0.))
                    .size_full()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.clicks += 1;
                        cx.notify();
                    })),
            )
            .child(perf_meter::card(&t, mono, &self.readout, 8.))
    }
}

#[gpui_kit::test]
fn a_click_on_the_card_reaches_what_is_underneath(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::apply(cx, true);
    });
    let (view, cx) = cx.add_window_view(|_, _| Underneath {
        clicks: 0,
        readout: perf::Meter::default().readout(Duration::ZERO, true, None, None),
    });
    cx.run_until_parked();
    let size = cx.update(|window, _| window.viewport_size());
    let on_card = point(
        size.width - px(CARD_MARGIN + CARD_WIDTH / 2.0),
        px(CARD_MARGIN + CARD_HEIGHT / 2.0),
    );
    cx.simulate_click(on_card, Default::default());
    assert_eq!(view.read_with(cx, |view, _| view.clicks), 1);
}
