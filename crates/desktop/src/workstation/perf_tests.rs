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

    // The meter reads the executor's clock, so the window can "rest" without sleeping: three
    // seconds with nothing painting fire the one rest timer (a timer is not a frame)...
    cx.executor().advance_clock(Duration::from_secs(3));
    cx.run_until_parked();
    assert_eq!(samples(&view, cx).0, base + 5, "a timer is not a frame");
    // ...and the one redraw it asks for, to write "parado", is the meter's own: not counted.
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
            .next_wakeup(view.perf.elapsed())),
        None,
        "at rest nothing is armed: the window can sleep"
    );
    // Control: the very next frame anyone paints is a frame (the own-redraw mark was spent).
    paint(cx);
    assert_eq!(samples(&view, cx).0, base + 6);
}

fn refreshes(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> u64 {
    view.read_with(cx, |view, _| view.perf.refreshes())
}

#[gpui_kit::test]
fn the_numbers_are_recomputed_at_four_hertz_at_most(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    toggle(&view, cx, PerfMeterCommand::Show);
    let first = refreshes(&view, cx);
    assert!(first >= 1, "showing computes the text once");

    // The clock is the executor's, so time does not move while the paints below run: whatever
    // the machine's speed, eight frames inside one instant recompute nothing.
    for _ in 0..8 {
        paint(cx);
    }
    assert_eq!(refreshes(&view, cx), first, "eight frames, no recompute");

    // Just under the interval: still nothing, however many frames paint.
    cx.executor()
        .advance_clock(perf::REFRESH_EVERY - Duration::from_millis(1));
    for _ in 0..8 {
        paint(cx);
    }
    assert_eq!(refreshes(&view, cx), first, "249 ms is not 4 Hz yet");

    // At the interval exactly one recompute, however many frames follow it.
    cx.executor().advance_clock(Duration::from_millis(1));
    for _ in 0..8 {
        paint(cx);
    }
    assert_eq!(refreshes(&view, cx), first + 1, "250 ms: one recompute");
}

#[gpui_kit::test]
fn hiding_cancels_the_rest_timer_and_nothing_wakes_while_hidden(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    toggle(&view, cx, PerfMeterCommand::Show);
    paint(cx);
    assert!(
        view.read_with(cx, |view, _| (
            view.perf.wake_armed(),
            view.perf.wake_pending()
        )) == (true, true),
        "a paint arms the one-shot rest timer"
    );
    toggle(&view, cx, PerfMeterCommand::Hide);
    let (armed, pending, wakes) = view.read_with(cx, |view, _| {
        (
            view.perf.wake_armed(),
            view.perf.wake_pending(),
            view.perf.wakes(),
        )
    });
    assert!(!armed && !pending, "hiding drops the timer");
    // Far past its deadline: a timer that survived the hide would fire now.
    cx.executor().advance_clock(Duration::from_secs(5));
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |view, _| view.perf.wakes()),
        wakes,
        "a hidden meter never wakes the executor"
    );
    // Showing again arms a fresh timer on the next paint, not a stale one.
    toggle(&view, cx, PerfMeterCommand::Show);
    paint(cx);
    assert!(view.read_with(cx, |view, _| view.perf.wake_armed()));
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

// ---- playback and the meter

/// A silent playback (no audio device in a test) that really goes through `playback_begin`.
fn start_playback(view: &Entity<Workstation>, cx: &mut VisualTestContext, from: i64) {
    view.update(cx, |view, cx| {
        view.playback.preparing = Some(super::playback::Preparing { from });
        view.playback_begin(
            from,
            Err(tessel_editor_core::CommandError::new(
                "audio_output_unavailable",
                "no device in a test",
            )),
            cx,
        );
    });
    cx.run_until_parked();
}

fn readout(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Value {
    node(view, cx)["state"]["readout"].clone()
}

/// The monitor's reading as if the runtime were ready (the test window has no runtime, so the
/// node says `unavailable`; the rules under test are the meter's).
fn monitor_reading(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Value {
    view.read_with(cx, |view, _| {
        view.perf
            .meter()
            .borrow()
            .readout(view.perf.elapsed(), true, None, None)
            .monitor
            .to_json()
    })
}

#[gpui_kit::test]
fn showing_the_meter_on_a_paused_playback_reads_a_rest_and_a_running_one_reads_a_stall(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    start_playback(&view, cx, 0);
    view.update(cx, |view, cx| view.playback_pause(cx).unwrap());
    toggle(&view, cx, PerfMeterCommand::Show);

    // Two seconds of silence on a paused playback: a rest, not a 0 fps stall.
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    let paused = readout(&view, cx);
    assert_eq!(monitor_reading(&view, cx)["state"], "idle");
    assert_eq!(paused["playing"], json!(false), "{paused}");
    assert_eq!(paused["playback_loss"]["state"], "not_playing", "{paused}");

    // Resuming expects pictures again, and two more silent seconds ARE a stall (the control:
    // the rest above is not simply the meter never reading a stall).
    view.update(cx, |view, cx| view.playback_resume(cx).unwrap());
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    let running = readout(&view, cx);
    assert_eq!(running["playing"], json!(true), "{running}");
    let monitor = monitor_reading(&view, cx);
    assert_eq!(monitor["state"], "live", "{monitor}");
    assert_eq!(monitor["fps"], json!(0.0), "{monitor}");
}

#[gpui_kit::test]
fn resuming_a_playback_that_never_paused_keeps_the_loss_counts(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    start_playback(&view, cx, 0);
    // The monitor showed frame 0, then the meter is switched on mid-playback.
    view.update(cx, |view, _| view.viewport.note_presented(Some(0)));
    toggle(&view, cx, PerfMeterCommand::Show);
    view.update(cx, |view, _| {
        view.perf_playback_clock(3);
        view.viewport.note_presented(Some(3));
    });
    let loss = readout(&view, cx)["playback_loss"].clone();
    assert_eq!(
        (loss["span_frames"].clone(), loss["lost"].clone()),
        (json!(4), json!(2)),
        "{loss}"
    );

    // `playback.resume` on a playback that is running: not a new run.
    view.update(cx, |view, cx| view.playback_resume(cx).unwrap());
    assert_eq!(readout(&view, cx)["playback_loss"], loss);

    // After a real pause, resuming starts a new run (the control: the counts do reset then).
    view.update(cx, |view, cx| {
        view.playback_pause(cx).unwrap();
        view.playback_resume(cx).unwrap();
    });
    let restarted = readout(&view, cx)["playback_loss"].clone();
    assert_ne!(restarted, loss);
    assert_eq!(restarted["lost"], json!(0), "{restarted}");
}

#[gpui_kit::test]
fn playback_counts_its_first_frame_only_if_the_monitor_was_showing_it(cx: &mut TestAppContext) {
    // The monitor shows frame 192 and playback starts at 0.
    let (view, cx) = fixture(cx);
    view.update(cx, |view, _| view.viewport.note_presented(Some(192)));
    toggle(&view, cx, PerfMeterCommand::Show);
    start_playback(&view, cx, 0);
    let loss = readout(&view, cx)["playback_loss"].clone();
    assert_eq!(
        loss["presented_frames"],
        json!(0),
        "frame 0 was never shown: {loss}"
    );

    // The control: the monitor was showing frame 0.
    let (view, cx) = fixture(cx);
    view.update(cx, |view, _| view.viewport.note_presented(Some(0)));
    toggle(&view, cx, PerfMeterCommand::Show);
    start_playback(&view, cx, 0);
    let loss = readout(&view, cx)["playback_loss"].clone();
    assert_eq!(loss["presented_frames"], json!(1), "{loss}");
}

// ---- the status bar

fn status_bounds(cx: &mut VisualTestContext, selector: &'static str) -> Bounds<Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("`{selector}` is not on screen"))
}

#[gpui_kit::test]
fn at_960_px_with_the_monitor_maximized_the_strip_gives_way_and_the_rest_stays_whole(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    cx.simulate_resize(size(px(960.), px(640.)));
    toggle(&view, cx, PerfMeterCommand::Show);
    for panel in [
        PanelKind::Library,
        PanelKind::Inspector,
        PanelKind::Timeline,
    ] {
        hide(&view, cx, panel);
    }
    // A long activity next to the strip and the save state: the case that used to overflow.
    view.update(cx, |view, cx| {
        view.activity = Some(Activity {
            source: Source::Cli,
            text: "Exportação em andamento: renderizando o quadro 1234 de 4321".to_string(),
            error: false,
        });
        cx.notify();
    });
    settle(cx);
    assert_eq!(
        node(&view, cx)["state"]["placement"],
        json!({"kind": "status_bar_strip"})
    );

    let window = f32::from(cx.update(|window, _| window.viewport_size().width));
    let strip = status_bounds(cx, "perf-strip");
    let activity = status_bounds(cx, "status-activity");
    let saved = status_bounds(cx, "status-saved");
    let right = status_bounds(cx, "status-right");
    for (name, bounds) in [("strip", strip), ("activity", activity), ("saved", saved)] {
        assert!(
            f32::from(bounds.right()) <= f32::from(right.right()) + 0.5,
            "{name} {bounds:?} sticks out of the right group {right:?}"
        );
        assert!(
            f32::from(bounds.right()) <= window,
            "{name} {bounds:?} is pushed past the window ({window})"
        );
    }
    assert!(
        f32::from(activity.size.height) < 20.0,
        "the activity wrapped onto a second line: {activity:?}"
    );
    assert!(
        f32::from(strip.size.width) > 0.0,
        "the strip is still there, only cut"
    );
}
