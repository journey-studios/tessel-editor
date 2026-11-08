//! F049 / BL145.2: the rolling performance history, driven through the same entry points as the
//! panel's buttons, the header and `ui set|invoke perf.history`.
//!
//! GPUI's test platform lays out and routes input but paints no pixels, and a test window has no
//! `Root`, so a real dialog cannot open here: these tests prove state (what is recorded, when,
//! and what is not), plumbing and the cost rules with the executor's controlled clock. The
//! charts, the dialog and the theme are judged by the native proof (`scripts/perf-history-proof.py`).
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use super::input_tests::fixture;
use super::perf_history_view::HistoryView;
use super::perf_meter::PerfClock;
use super::viewport_host::ViewportHost;
use super::*;
// The Kit's test macro expands to `#[test]`; the glob import above would resolve it to the Kit macro again.
use core::prelude::v1::test;
use serde_json::{Value, json};
use tessel_editor_core::commands::{Parsed, parse_invoke, parse_set};
use tessel_editor_core::perf::{Meter, REFRESH_EVERY};
use tessel_editor_core::perf_history::{self, EventKind, HistoryCommand, Metric, SessionKind};
use tessel_editor_core::{CommandError, Raw};

use super::viewport_host_tests::{
    frame_done, from_page, page, ready_message, retitle, sent, titled_state,
};

const NODE: &str = perf_history::TARGET;

fn paint(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.refresh();
        window.simulate_next_frame(cx);
    });
    cx.run_until_parked();
}

fn settle(cx: &mut VisualTestContext) {
    for _ in 0..4 {
        paint(cx);
    }
}

fn run(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    command: HistoryCommand,
) -> Result<Value, CommandError> {
    let result = view.update_in(cx, |view, window, cx| {
        view.execute_perf_history(command, Source::Cli, window, cx)
    });
    cx.run_until_parked();
    result
}

fn node(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Value {
    view.read_with(cx, |view, cx| view.tree_node(NODE, cx).unwrap())
}

/// How the last export ended (`get perf.history` -> `export`), once its background write ran.
fn export_state(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Value {
    cx.run_until_parked();
    node(view, cx)["state"]["export"].clone()
}

/// The code of the typed error the last export ended with; panics if it did not fail.
fn export_error(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> String {
    let state = export_state(view, cx);
    assert_eq!(state["state"], "failed", "{state}");
    state["error"]["code"].as_str().unwrap().to_string()
}

fn frames(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> u64 {
    view.read_with(cx, |view, _| {
        let now = view.perf.elapsed();
        view.perf
            .meter()
            .borrow()
            .recorder()
            .view(now)
            .and_then(|snapshot| snapshot.summary(Metric::UiFrames))
            .map_or(0, |summary| summary.count)
    })
}

fn recording(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |view, _| {
        view.perf.meter().borrow().recorder().recording()
    })
}

fn open_panel(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Entity<HistoryView> {
    view.update_in(cx, |view, window, cx| {
        let this = cx.entity();
        let destination = cx.new(|cx| InputState::new(window, cx));
        let panel = cx.new(|cx| HistoryView::new(this, destination, cx));
        view.history_mark_open(panel.clone());
        panel
    })
}

/// Builds the panel's elements (the render logic: the snapshot, the timer decision) without
/// painting them.
fn render_panel(panel: &Entity<HistoryView>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        panel.update(cx, |panel, cx| {
            let _ = panel.render(window, cx);
        });
    });
}

#[gpui_kit::test]
fn the_history_is_off_by_default_and_costs_nothing(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    for _ in 0..6 {
        paint(cx);
    }
    let state = node(&view, cx)["state"].clone();
    assert_eq!(state["recording"], json!(false), "{state}");
    assert_eq!(state["visible"], json!(false));
    assert_eq!(state["has_data"], json!(false));
    view.read_with(cx, |view, _| {
        let meter = view.perf.meter();
        let meter = meter.borrow();
        assert!(
            !meter.enabled(),
            "nothing collects: every feed returns on one bool"
        );
        assert!(
            meter.recorder().view(view.perf.elapsed()).is_none(),
            "no ring exists"
        );
        assert!(!view.perf.wake_armed());
    });
}

#[gpui_kit::test]
fn recording_with_the_card_hidden_counts_paints_and_arms_no_timer(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let started = run(&view, cx, HistoryCommand::Record(true)).unwrap();
    assert_eq!(started["changed"], json!(true), "{started}");
    settle(cx);
    let base = frames(&view, cx);
    assert!(base >= 1, "the first paints after starting are recorded");
    for _ in 0..5 {
        paint(cx);
    }
    assert_eq!(frames(&view, cx), base + 5, "five paints, five frames");

    view.read_with(cx, |view, _| {
        let meter = view.perf.meter();
        let meter = meter.borrow();
        assert!(!meter.card_visible(), "recording never shows the card");
        assert_eq!(
            meter.next_wakeup(view.perf.elapsed()),
            None,
            "no wake-up timer for a hidden card"
        );
        assert!(!view.perf.wake_armed() && !view.perf.wake_pending());
        assert!(
            view.geometry.get("perf.area").is_none(),
            "no probe, no overlay"
        );
    });
    // Time passing arms nothing for the history itself.
    cx.executor().advance_clock(Duration::from_secs(5));
    cx.run_until_parked();
    assert!(!view.read_with(cx, |view, _| view.perf.wake_armed()));

    let state = node(&view, cx)["state"].clone();
    assert_eq!(state["recording"], json!(true));
    assert_eq!(
        state["visible"],
        json!(false),
        "recording is not visibility"
    );
    assert_eq!(state["card_visible"], json!(false));

    // Starting again changes nothing; stopping keeps what was recorded readable.
    let again = run(&view, cx, HistoryCommand::Record(true)).unwrap();
    assert_eq!(again["changed"], json!(false), "{again}");
    run(&view, cx, HistoryCommand::Record(false)).unwrap();
    let kept = frames(&view, cx);
    assert!(kept >= base + 5, "what was recorded stays readable");
    paint(cx);
    assert_eq!(frames(&view, cx), kept, "stopped: nothing more is recorded");
    assert_eq!(node(&view, cx)["state"]["has_data"], json!(true));

    run(&view, cx, HistoryCommand::Clear).unwrap();
    assert_eq!(frames(&view, cx), 0, "clearing forgets it");
}

#[gpui_kit::test]
fn the_card_and_the_recording_are_independent(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    view.update(cx, |view, cx| {
        view.execute_perf(
            tessel_editor_core::perf::PerfMeterCommand::Show,
            Source::Cli,
            cx,
        )
        .unwrap();
    });
    settle(cx);
    assert!(!recording(&view, cx), "showing the card is not recording");
    run(&view, cx, HistoryCommand::Record(true)).unwrap();
    settle(cx);
    view.update(cx, |view, cx| {
        view.execute_perf(
            tessel_editor_core::perf::PerfMeterCommand::Hide,
            Source::Cli,
            cx,
        )
        .unwrap();
    });
    settle(cx);
    assert!(
        recording(&view, cx),
        "hiding the card does not stop the recording"
    );
    let base = frames(&view, cx);
    paint(cx);
    assert_eq!(frames(&view, cx), base + 1, "the light capture goes on");
}

#[gpui_kit::test]
fn the_panel_hides_the_monitor_but_neither_stops_nor_starts_the_recording(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    run(&view, cx, HistoryCommand::Record(true)).unwrap();
    let panel = open_panel(&view, cx);
    view.update(cx, |view, cx| {
        assert!(
            view.workflow.has_dialog(),
            "the native view must step aside"
        );
        assert!(view.history_visible());
        view.update_viewport_visibility(cx);
        assert!(!view.viewport.on_screen);
    });
    assert!(recording(&view, cx));
    assert_eq!(node(&view, cx)["state"]["visible"], json!(true));
    // A dialog already open keeps the panel from opening over it.
    view.update(cx, |view, _| view.workflow.history_open = false);
    view.update(cx, |view, _| view.workflow.pretend_dialog_open(true));
    let refused = run(&view, cx, HistoryCommand::Show).unwrap_err();
    assert_eq!(refused.code, "dialog_open", "{refused:?}");
    view.update(cx, |view, _| view.workflow.pretend_dialog_open(false));

    view.update(cx, |view, cx| {
        view.history_mark_closed();
        view.update_viewport_visibility(cx);
        assert!(view.viewport.on_screen, "closing brings the monitor back");
    });
    assert!(recording(&view, cx), "closing is not stopping");
    drop(panel);
}

#[gpui_kit::test]
fn a_frozen_snapshot_is_one_immutable_window_and_selection_stays_inside_it(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    let nothing = run(&view, cx, HistoryCommand::Freeze).unwrap_err();
    assert_eq!(nothing.code, "perf_history_empty", "{nothing:?}");

    run(&view, cx, HistoryCommand::Record(true)).unwrap();
    for _ in 0..4 {
        paint(cx);
    }
    cx.executor().advance_clock(Duration::from_millis(600));
    paint(cx);
    let frozen_at = frames(&view, cx);
    assert_eq!(
        run(&view, cx, HistoryCommand::Freeze).unwrap()["changed"],
        json!(true)
    );
    assert_eq!(
        run(&view, cx, HistoryCommand::Freeze).unwrap()["changed"],
        json!(false)
    );
    for _ in 0..3 {
        paint(cx);
    }
    assert!(
        frames(&view, cx) == frozen_at,
        "the view is the frozen snapshot"
    );
    let live = view.read_with(cx, |view, _| {
        let meter = view.perf.meter();
        let meter = meter.borrow();
        meter
            .recorder()
            .snapshot(view.perf.elapsed())
            .unwrap()
            .summary(Metric::UiFrames)
            .unwrap()
            .count
    });
    assert!(
        live >= frozen_at + 3,
        "recording continues underneath: {live} vs {frozen_at}"
    );
    assert_eq!(node(&view, cx)["state"]["frozen"], json!(true));

    // Selection: inside the window ok, outside is a typed error and changes nothing.
    let end = view.read_with(cx, |view, _| {
        let meter = view.perf.meter();
        let meter = meter.borrow();
        meter.recorder().frozen().unwrap().captured_at
    });
    let outside = run(
        &view,
        cx,
        HistoryCommand::Select(Some(end + Duration::from_secs(5))),
    )
    .unwrap_err();
    assert_eq!(outside.code, "perf_history_outside_window", "{outside:?}");
    assert!(node(&view, cx)["state"]["selected_ms"].is_null());
    let inside = run(
        &view,
        cx,
        HistoryCommand::Select(Some(end - Duration::from_millis(100))),
    )
    .unwrap();
    assert!(inside["inspection"].is_object(), "{inside}");
    assert!(node(&view, cx)["state"]["selected_ms"].is_number());
    run(&view, cx, HistoryCommand::Select(None)).unwrap();
    assert!(node(&view, cx)["state"]["selected_ms"].is_null());

    assert_eq!(
        run(&view, cx, HistoryCommand::Resume).unwrap()["changed"],
        json!(true)
    );
    assert_eq!(node(&view, cx)["state"]["frozen"], json!(false));
}

#[gpui_kit::test]
fn the_export_is_a_local_versioned_json_without_paths_and_its_errors_are_typed(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    let dir = std::env::temp_dir().join(format!("tessel-history-export-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("history.json");

    let empty = run(&view, cx, HistoryCommand::Export(target.clone())).unwrap_err();
    assert_eq!(empty.code, "perf_history_empty");
    assert!(!target.exists(), "nothing is written for an empty history");

    run(&view, cx, HistoryCommand::Record(true)).unwrap();
    for _ in 0..3 {
        paint(cx);
    }
    // The command answers at once that it started: the disk work is off the window's thread.
    let started = run(&view, cx, HistoryCommand::Export(target.clone())).unwrap();
    assert_eq!(started["state"], "writing", "{started}");
    assert_eq!(started["export_id"], 1);
    let done = export_state(&view, cx);
    assert_eq!(done["state"], "done", "{done}");
    assert!(done["bytes"].as_u64().unwrap() > 0, "{done}");
    assert_eq!(
        view.read_with(cx, |view, _| view.activity.as_ref().map(|a| a.text.clone())),
        Some("Histórico de desempenho: export concluído".to_string())
    );
    let text = std::fs::read_to_string(&target).unwrap();
    let document: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(document["schema"], "tessel.perf-history");
    assert_eq!(document["schema_version"], 1);
    assert!(
        !text.contains(&dir.to_string_lossy().to_string()),
        "the export never carries a local path"
    );
    assert!(
        !done
            .to_string()
            .contains(&dir.to_string_lossy().to_string()),
        "nor does the observable state: {done}"
    );

    // What the disk decides is the typed end of the write, not a refusal of the command.
    run(&view, cx, HistoryCommand::Export(dir.clone())).unwrap();
    assert_eq!(export_error(&view, cx), "perf_history_export_is_directory");
    run(
        &view,
        cx,
        HistoryCommand::Export(dir.join("missing").join("history.json")),
    )
    .unwrap();
    assert_eq!(
        export_error(&view, cx),
        "perf_history_export_parent_missing"
    );
    assert_eq!(
        view.read_with(cx, |view, _| view.activity.as_ref().map(|a| a.error)),
        Some(true),
        "the status line says the export did not happen"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn the_export_is_written_off_the_window_thread_from_a_snapshot_taken_at_the_call(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    let dir = std::env::temp_dir().join(format!("tessel-history-async-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (first, second) = (dir.join("first.json"), dir.join("second.json"));
    run(&view, cx, HistoryCommand::Record(true)).unwrap();
    for _ in 0..3 {
        paint(cx);
    }
    let before_clear = frames(&view, cx);
    assert!(before_clear >= 3, "the premise: three paints are recorded");
    // Everything below happens before the executor gets a turn: the write has not run yet, so
    // what the file holds can only be what the call took.
    let (started, refused, cleared) = view.update_in(cx, |view, window, cx| {
        let started = view.execute_perf_history(
            HistoryCommand::Export(first.clone()),
            Source::Cli,
            window,
            cx,
        );
        assert!(!first.exists(), "the window's thread wrote nothing");
        let refused = view.execute_perf_history(
            HistoryCommand::Export(second.clone()),
            Source::Cli,
            window,
            cx,
        );
        let cleared = view.execute_perf_history(HistoryCommand::Clear, Source::Cli, window, cx);
        (started, refused, cleared)
    });
    assert_eq!(started.unwrap()["state"], "writing");
    assert_eq!(
        refused.unwrap_err().code,
        "perf_history_export_in_progress",
        "one export at a time"
    );
    cleared.unwrap();
    assert_eq!(export_state(&view, cx)["state"], "done");
    assert!(!second.exists(), "the refused export wrote nothing");
    let document: Value = serde_json::from_str(&std::fs::read_to_string(&first).unwrap()).unwrap();
    assert!(
        document["summary"]["ui_frames"]["count"].as_u64().unwrap() >= 3,
        "the snapshot of the call, not the history cleared right after it: {}",
        document["summary"]["ui_frames"]
    );
    // The recording CONTINUES after a clear (only the past is forgotten), so the paints that follow
    // it are recorded again: the proof that the clear happened is a history shorter than the one
    // that was cleared, its first segment opened by the clear and its event line starting there.
    assert!(
        frames(&view, cx) < before_clear,
        "the clear did happen: {} frames after, {before_clear} before",
        frames(&view, cx)
    );
    view.read_with(cx, |view, _| {
        let now = view.perf.elapsed();
        let meter = view.perf.meter();
        let meter = meter.borrow();
        let snapshot = meter.recorder().snapshot(now).unwrap();
        assert!(
            snapshot
                .events
                .iter()
                .any(|event| event.kind == EventKind::RecordingCleared),
            "the event line says the history was cleared"
        );
        assert_eq!(
            snapshot.segments.first().map(|segment| segment.kind),
            Some(SessionKind::Cleared),
            "the history now starts at the clear"
        );
    });
    // The next export starts fresh once the first ended.
    run(&view, cx, HistoryCommand::Record(true)).unwrap();
    let again = run(&view, cx, HistoryCommand::Export(second.clone())).unwrap();
    assert_eq!(again["export_id"], 2);
    assert_eq!(export_state(&view, cx)["state"], "done");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn set_and_invoke_reach_the_same_handler_as_the_buttons(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let apply = |view: &Entity<Workstation>, cx: &mut VisualTestContext, parsed: Parsed| {
        let Parsed::PerfHistory(command) = parsed else {
            panic!("not a history command: {parsed:?}");
        };
        run(view, cx, command)
    };
    let set = |field: &str, value: Raw| parse_set(NODE, field, &value).unwrap();
    apply(&view, cx, set("recording", Raw::Bool(true))).unwrap();
    assert!(recording(&view, cx));
    settle(cx);
    let layout = view.read_with(cx, |view, cx| view.layout_snapshot(cx));
    let state = view.read_with(cx, |view, _| view.state.clone());
    let invoke = |action: &str| parse_invoke(&state, &layout, NODE, action).unwrap();
    let frozen = apply(&view, cx, invoke("freeze")).unwrap();
    assert_eq!(frozen["target"], NODE);
    apply(&view, cx, invoke("resume")).unwrap();
    let snapshot = apply(&view, cx, invoke("snapshot")).unwrap();
    assert_eq!(
        snapshot["snapshot"]["schema"], "tessel.perf-history",
        "{snapshot}"
    );
    apply(&view, cx, invoke("record_stop")).unwrap();
    assert!(!recording(&view, cx));
}

#[gpui_kit::test]
fn a_playhead_move_is_an_event_and_playback_ticks_are_not(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    // Off: nothing is kept.
    view.update(cx, |view, cx| {
        view.execute(
            Command::SetPlayhead {
                frame: Raw::Number(3.0),
            },
            Source::Cli,
            cx,
        )
        .unwrap();
    });
    assert_eq!(
        view.read_with(cx, |view, _| view
            .perf
            .meter()
            .borrow()
            .recorder()
            .events_held()),
        0
    );
    run(&view, cx, HistoryCommand::Record(true)).unwrap();
    view.update(cx, |view, cx| {
        view.execute(
            Command::SetPlayhead {
                frame: Raw::Number(5.0),
            },
            Source::Cli,
            cx,
        )
        .unwrap();
    });
    let kinds: Vec<EventKind> = view.read_with(cx, |view, _| {
        let meter = view.perf.meter();
        let meter = meter.borrow();
        meter
            .recorder()
            .snapshot(view.perf.elapsed())
            .unwrap()
            .events
            .iter()
            .map(|event| event.kind)
            .collect()
    });
    assert_eq!(kinds, vec![EventKind::RecordingStarted, EventKind::Seek]);
}

#[gpui_kit::test]
fn the_panel_refreshes_at_most_four_times_a_second_and_only_while_the_recording_moves(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    let panel = open_panel(&view, cx);
    render_panel(&panel, cx);
    assert!(
        !panel.read_with(cx, |panel, _| panel.timer_armed()),
        "nothing records: no timer"
    );

    run(&view, cx, HistoryCommand::Record(true)).unwrap();
    settle(cx);
    render_panel(&panel, cx);
    assert!(panel.read_with(cx, |panel, _| panel.timer_armed()));

    // Four seconds in 100 ms steps: 16 refreshes, never one per step.
    for _ in 0..40 {
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }
    let ticks = panel.read_with(cx, |panel, _| panel.ticks());
    assert!(
        (15..=16).contains(&ticks),
        "{ticks} refreshes in 4 s at {REFRESH_EVERY:?}"
    );

    // The refresh is the diagnostic's own redraw: the next paint is not a UI frame...
    assert!(view.read_with(cx, |view, _| view.perf.own_redraw_pending()));
    let before = frames(&view, cx);
    paint(cx);
    assert_eq!(
        frames(&view, cx),
        before,
        "the panel's own redraw is not a UI frame"
    );
    // ...and the one after it is (the mark was spent).
    paint(cx);
    assert_eq!(frames(&view, cx), before + 1);

    // Frozen: the snapshot cannot move, so the timer goes.
    run(&view, cx, HistoryCommand::Freeze).unwrap();
    render_panel(&panel, cx);
    assert!(!panel.read_with(cx, |panel, _| panel.timer_armed()));
    let frozen_ticks = panel.read_with(cx, |panel, _| panel.ticks());
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    assert_eq!(panel.read_with(cx, |panel, _| panel.ticks()), frozen_ticks);

    // Closed: the view, and its timer with it, is gone.
    run(&view, cx, HistoryCommand::Resume).unwrap();
    render_panel(&panel, cx);
    assert!(panel.read_with(cx, |panel, _| panel.timer_armed()));
    let weak = panel.downgrade();
    view.update(cx, |view, _| view.history_mark_closed());
    drop(panel);
    cx.run_until_parked();
    assert!(weak.upgrade().is_none(), "no panel, no timer");
}

// ---- the host's feeds, without a window

#[test]
fn the_host_feeds_latency_stale_answers_and_sessions_to_the_history() {
    let mut state = titled_state("Um");
    let meter = Rc::new(RefCell::new(Meter::default()));
    let clock = PerfClock::wall();
    meter
        .borrow_mut()
        .start_recording(clock.elapsed(), None, None);
    let mut host = ViewportHost::new(Ok(page("history")));
    host.attach_meter(meter.clone(), clock.clone());
    from_page(&mut host, &state, json!({"type":"page.ready"}));
    sent(&mut host);
    from_page(&mut host, &state, ready_message());
    let first = sent(&mut host);
    let first_id = first[0]["id"].as_u64().unwrap();
    let first_revision = first[0]["revision"].as_u64().unwrap();

    // The edit supersedes the frame in flight: a second request, admitted after the first.
    retitle(&mut state, "Dois");
    host.sync(&state);
    let second = sent(&mut host);
    let second_id = second[1]["id"].as_u64().unwrap();
    std::thread::sleep(Duration::from_millis(15));
    from_page(
        &mut host,
        &state,
        frame_done(second_id, second[1]["revision"].as_u64().unwrap()),
    );
    // The old answer arrives last and is dropped: stale, never a picture, never a latency.
    from_page(&mut host, &state, frame_done(first_id, first_revision));

    let snapshot = meter.borrow().recorder().snapshot(clock.elapsed()).unwrap();
    let latency = snapshot
        .summary(Metric::LatencyMs)
        .expect("one matched picture");
    assert_eq!(latency.count, 1, "only the accepted picture has a latency");
    assert!(
        latency.max >= 15.0,
        "from admission to acceptance: {}",
        latency.max
    );
    assert_eq!(
        snapshot.summary(Metric::RenderMs).unwrap().max,
        1.5,
        "render_ms is the runtime's CPU time, kept apart from latency"
    );
    assert_eq!(snapshot.summary(Metric::MonitorFrames).unwrap().count, 1);
    assert_eq!(snapshot.counters.stale_responses, 1);
    assert_eq!(snapshot.counters.unmatched_images, 0);
    assert!(
        snapshot.context.is_some(),
        "the picture's format is the context"
    );

    // A replaced project and a reloaded page open sessions: nothing is measured across them.
    host.reset_project();
    assert!(host.begin_reload());
    let after = meter.borrow().recorder().snapshot(clock.elapsed()).unwrap();
    let kinds: Vec<EventKind> = after.events.iter().map(|event| event.kind).collect();
    assert!(kinds.contains(&EventKind::ProjectReplaced), "{kinds:?}");
    assert!(kinds.contains(&EventKind::ViewportReloaded), "{kinds:?}");
    assert_eq!(after.segments.len(), 3, "recording + project + reload");
}

#[test]
fn a_host_with_no_recording_keeps_nothing_of_what_it_hears() {
    let state = titled_state("Um");
    let meter = Rc::new(RefCell::new(Meter::default()));
    let mut host = ViewportHost::new(Ok(page("history-off")));
    host.attach_meter(meter.clone(), PerfClock::wall());
    from_page(&mut host, &state, json!({"type":"page.ready"}));
    sent(&mut host);
    from_page(&mut host, &state, ready_message());
    let first = sent(&mut host);
    from_page(
        &mut host,
        &state,
        frame_done(
            first[0]["id"].as_u64().unwrap(),
            first[0]["revision"].as_u64().unwrap(),
        ),
    );
    host.reset_project();
    let meter = meter.borrow();
    assert!(!meter.enabled());
    assert!(meter.recorder().snapshot(Duration::from_secs(1)).is_none());
    assert_eq!(meter.recorder().events_held(), 0);
}

#[gpui_kit::test]
fn the_export_goes_only_where_the_user_said_and_refuses_what_it_would_have_to_guess_or_replace(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    let dir = std::env::temp_dir().join(format!("tessel-history-dest-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let state_of = |view: &Entity<Workstation>, cx: &mut VisualTestContext| {
        node(view, cx)["state"]["export_destination"].clone()
    };
    assert_eq!(
        state_of(&view, cx),
        json!(""),
        "no destination until one is given"
    );

    run(&view, cx, HistoryCommand::Record(true)).unwrap();
    for _ in 0..3 {
        paint(cx);
    }
    // No destination: refused, nothing written anywhere (not even the temp folder).
    let none = run(&view, cx, HistoryCommand::ExportToDestination).unwrap_err();
    assert_eq!(none.code, "perf_history_export_path_missing");

    // A relative path would land wherever the editor started: refused.
    let said = run(
        &view,
        cx,
        HistoryCommand::Destination("history.json".into()),
    )
    .unwrap();
    assert_eq!(said["export_destination"], "history.json");
    assert_eq!(state_of(&view, cx), json!("history.json"));
    let relative = run(&view, cx, HistoryCommand::ExportToDestination).unwrap_err();
    assert_eq!(relative.code, "perf_history_export_relative");

    // Choosing the path writes nothing by itself; the export does, once.
    let target = dir.join("chosen.json");
    run(
        &view,
        cx,
        HistoryCommand::Destination(target.to_string_lossy().into_owned()),
    )
    .unwrap();
    assert!(!target.exists(), "a destination is not an export");
    let started = run(&view, cx, HistoryCommand::ExportToDestination).unwrap();
    assert_eq!(started["state"], "writing", "{started}");
    let done = export_state(&view, cx);
    assert!(done["bytes"].as_u64().unwrap() > 0, "{done}");
    let written = std::fs::read_to_string(&target).unwrap();
    let document: Value = serde_json::from_str(&written).unwrap();
    assert_eq!(document["schema"], "tessel.perf-history");

    // The same name again is an existing file: refused, the first export is untouched.
    run(&view, cx, HistoryCommand::ExportToDestination).unwrap();
    assert_eq!(export_error(&view, cx), "perf_history_export_exists");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), written);

    // A file that is not ours (a project) is never replaced either.
    let project = dir.join("project.tessel.json");
    std::fs::write(&project, "{}").unwrap();
    run(
        &view,
        cx,
        HistoryCommand::Destination(project.to_string_lossy().into_owned()),
    )
    .unwrap();
    run(&view, cx, HistoryCommand::ExportToDestination).unwrap();
    assert_eq!(export_error(&view, cx), "perf_history_export_exists");
    assert_eq!(std::fs::read_to_string(&project).unwrap(), "{}");
    let names = std::fs::read_dir(&dir).unwrap().count();
    assert_eq!(names, 2, "no temporary and nothing else was created");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn the_destination_field_of_the_open_panel_and_the_command_agree(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let panel = open_panel(&view, cx);
    run(
        &view,
        cx,
        HistoryCommand::Destination("/tmp/from-the-cli.json".into()),
    )
    .unwrap();
    assert_eq!(
        panel.read_with(cx, |panel, cx| panel.destination_text(cx)),
        "/tmp/from-the-cli.json",
        "set export_destination fills the field the user sees"
    );
    // What is typed in the field is what `get` reports (the panel mirrors every edit).
    panel.update_in(cx, |panel, window, cx| {
        panel.type_destination("/tmp/typed.json", window, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        node(&view, cx)["state"]["export_destination"],
        json!("/tmp/typed.json")
    );
}

/// A host that records into a fresh meter on the wall clock, with the page ready.
fn recording_host(tag: &str) -> (Rc<RefCell<Meter>>, PerfClock, ViewportHost, EditorState) {
    let state = titled_state("Um");
    let meter = Rc::new(RefCell::new(Meter::default()));
    let clock = PerfClock::wall();
    meter
        .borrow_mut()
        .start_recording(clock.elapsed(), None, None);
    let mut host = ViewportHost::new(Ok(page(tag)));
    host.attach_meter(meter.clone(), clock.clone());
    from_page(&mut host, &state, json!({"type":"page.ready"}));
    sent(&mut host);
    from_page(&mut host, &state, ready_message());
    (meter, clock, host, state)
}

fn event_kinds(meter: &Rc<RefCell<Meter>>, clock: &PerfClock) -> Vec<EventKind> {
    meter
        .borrow()
        .recorder()
        .snapshot(clock.elapsed())
        .unwrap()
        .events
        .iter()
        .map(|event| event.kind)
        .collect()
}

#[test]
fn the_context_carries_the_sequence_rate_and_never_a_target_nobody_knows() {
    let (meter, clock, mut host, state) = recording_host("target-fps");
    let request = sent(&mut host).remove(0);
    from_page(
        &mut host,
        &state,
        frame_done(
            request["id"].as_u64().unwrap(),
            request["revision"].as_u64().unwrap(),
        ),
    );
    let snapshot = meter.borrow().recorder().snapshot(clock.elapsed()).unwrap();
    let context = snapshot.context.expect("a picture was accepted");
    assert_eq!(
        context.sequence_fps,
        Some(24.0),
        "the project's 24 fps, not an assumed 60"
    );
    assert_eq!(
        context.ui_target_fps, None,
        "the interface has no known target"
    );
    let document = meter
        .borrow()
        .recorder()
        .export_document(clock.elapsed())
        .unwrap();
    assert_eq!(document["context"]["sequence_target_fps"], json!(24.0));
    assert_eq!(document["context"]["ui_target_fps"], Value::Null);
}

#[test]
fn a_plan_the_core_refuses_is_one_failure_event_not_one_per_state_change() {
    use tessel_editor_core::plan::PreviewIntent;
    use tessel_editor_core::time::Ratio;
    let (meter, clock, mut host, mut state) = recording_host("refused-plan");
    sent(&mut host);
    let failures = |meter: &Rc<RefCell<Meter>>| {
        event_kinds(meter, &clock)
            .into_iter()
            .filter(|kind| *kind == EventKind::RequestFailed)
            .count()
    };
    // A preview of an asset that is not in the project: the planner refuses it, every time.
    state.preview = PreviewIntent::Source {
        asset: tessel_editor_core::Id::new("gone").unwrap(),
        time: Ratio::ZERO,
    };
    for _ in 0..5 {
        host.sync(&state);
    }
    assert_eq!(
        failures(&meter),
        1,
        "the same refusal repeated is one event"
    );
    // Back to a plan the core accepts, then refused again: that is news.
    state.preview = PreviewIntent::Sequence;
    host.sync(&state);
    state.preview = PreviewIntent::Source {
        asset: tessel_editor_core::Id::new("gone").unwrap(),
        time: Ratio::ZERO,
    };
    host.sync(&state);
    assert_eq!(
        failures(&meter),
        2,
        "a failure after a new request is a new event"
    );
}

#[test]
fn leaving_a_directed_scene_is_one_scene_change_and_staying_is_none() {
    let (meter, clock, mut host, _state) = recording_host("scene-change");
    // The monitor going back to the sequence when no scene was shown is not a scene change.
    host.leave_directed();
    let before = meter.borrow().recorder().snapshot(clock.elapsed()).unwrap();
    assert_eq!(before.segments.len(), 1, "only the recording's own segment");
    assert!(!event_kinds(&meter, &clock).contains(&EventKind::SceneChanged));

    host.pretend_directed_scene("scene-a");
    host.leave_directed();
    assert_eq!(host.directed_scene(), None);
    let after = meter.borrow().recorder().snapshot(clock.elapsed()).unwrap();
    assert_eq!(after.segments.len(), 2);
    assert_eq!(
        after.segments.last().map(|segment| segment.kind),
        Some(SessionKind::SceneChanged)
    );
    let changes = event_kinds(&meter, &clock)
        .into_iter()
        .filter(|kind| *kind == EventKind::SceneChanged)
        .count();
    assert_eq!(changes, 1);
    // The event never says which scene: the id is the segment number and the host's clock.
    let text = serde_json::to_string(
        &meter
            .borrow()
            .recorder()
            .export_document(clock.elapsed())
            .unwrap(),
    )
    .unwrap();
    assert!(!text.contains("scene-a"), "no scene name in the export");
}

#[test]
fn a_lost_and_restored_context_are_seen_once_each_in_the_event_line() {
    let (meter, clock, mut host, state) = recording_host("context-events");
    let request = sent(&mut host).remove(0);
    from_page(
        &mut host,
        &state,
        frame_done(
            request["id"].as_u64().unwrap(),
            request["revision"].as_u64().unwrap(),
        ),
    );
    from_page(&mut host, &state, json!({"type":"context.lost"}));
    // The page repeating itself while the context is down is still one loss.
    from_page(&mut host, &state, json!({"type":"context.lost"}));
    from_page(&mut host, &state, json!({"type":"context.restored"}));
    let kinds = event_kinds(&meter, &clock);
    let lost = kinds
        .iter()
        .filter(|kind| **kind == EventKind::ContextLost)
        .count();
    let restored = kinds
        .iter()
        .filter(|kind| **kind == EventKind::ContextRestored)
        .count();
    assert_eq!((lost, restored), (1, 1), "{kinds:?}");
    let lost_at = kinds
        .iter()
        .position(|kind| *kind == EventKind::ContextLost);
    let restored_at = kinds
        .iter()
        .position(|kind| *kind == EventKind::ContextRestored);
    assert!(lost_at < restored_at, "{kinds:?}");
    assert!(
        !kinds.contains(&EventKind::RequestFailed),
        "a loss is not also a failed request"
    );
}

#[test]
fn a_frame_that_times_out_reaches_the_event_line_as_a_failed_request() {
    let (meter, clock, mut host, _state) = recording_host("timeout-event");
    sent(&mut host);
    assert!(host.tick(std::time::Instant::now() + Duration::from_secs(31)));
    assert_eq!(host.failure().unwrap().code, "timeout");
    let events = meter
        .borrow()
        .recorder()
        .snapshot(clock.elapsed())
        .unwrap()
        .events;
    let failed: Vec<_> = events
        .iter()
        .filter(|event| event.kind == EventKind::RequestFailed)
        .collect();
    assert_eq!(failed.len(), 1, "{events:?}");
    assert_eq!(failed[0].detail.as_deref(), Some("timeout"));
}

#[test]
fn only_export_jobs_reach_the_history_and_only_their_id_state_and_error_code() {
    use tessel_editor_core::jobs::{JobKind, JobRegistry, JobState};
    use tessel_editor_core::perf_history::ExportJob;
    let registry = JobRegistry::new(2);
    let (release, wait) = std::sync::mpsc::channel::<()>();
    let export = registry.spawn(JobKind::VideoExport, "Meu filme", move |_| {
        let _ = wait.recv();
        Ok(Value::Null)
    });
    registry.spawn(JobKind::Hash, "other work", |_| Ok(Value::Null));
    let wait_for = |wanted: JobState| {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let jobs = super::perf_meter::export_jobs(&registry);
            if jobs.iter().any(|job| job.state == wanted) {
                return jobs;
            }
            assert!(std::time::Instant::now() < deadline, "never {wanted:?}");
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let running = wait_for(JobState::Running);
    assert_eq!(running.len(), 1, "a hash job is not an export");
    assert_eq!(
        running[0],
        ExportJob {
            id: export.clone(),
            state: JobState::Running,
            error_code: None
        }
    );
    // The jobs feed the recorder as events: the start, then the end, each once.
    let mut meter = Meter::default();
    meter.start_recording(Duration::ZERO, None, None);
    let recorder = meter.recorder_mut();
    recorder.exports_observed(Duration::from_millis(10), &[]);
    recorder.exports_observed(Duration::from_millis(1_100), &running);
    release.send(()).unwrap();
    let finished = wait_for(JobState::Succeeded);
    meter
        .recorder_mut()
        .exports_observed(Duration::from_millis(2_200), &finished);
    let kinds: Vec<EventKind> = meter
        .recorder()
        .snapshot(Duration::from_secs(3))
        .unwrap()
        .events
        .iter()
        .map(|event| event.kind)
        .filter(|kind| matches!(kind, EventKind::ExportStarted | EventKind::ExportFinished))
        .collect();
    assert_eq!(
        kinds,
        vec![EventKind::ExportStarted, EventKind::ExportFinished]
    );
    let text = serde_json::to_string(
        &meter
            .recorder()
            .export_document(Duration::from_secs(3))
            .unwrap(),
    )
    .unwrap();
    assert!(
        !text.contains("Meu filme"),
        "a job's label never reaches the history"
    );
}

#[gpui_kit::test]
fn a_destination_from_the_panel_does_not_read_its_own_update_lease(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let panel = open_panel(&view, cx);
    panel.update_in(cx, |panel, window, cx| {
        panel.run(
            HistoryCommand::Destination("/tmp/picked-history.json".into()),
            window,
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(
        panel.read_with(cx, |panel, cx| panel.destination_text(cx)),
        "/tmp/picked-history.json"
    );
    assert_eq!(
        node(&view, cx)["state"]["export_destination"],
        json!("/tmp/picked-history.json")
    );
}
