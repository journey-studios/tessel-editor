//! The playback loop's poll schedule inside the window (BL157.6). The rules themselves (and the model that reproduces the native
//! measurements) are pure and live in `playback_poll_tests.rs`; these prove that the loop applies them: the mode in effect is
//! reported, the tick turns what it read into the next wait, a paused clock is asked at the fixed interval, and the loop arms its
//! timer for the wait the tick said. The timer is the executor's (a mock clock), so no test waits for real.
//!
//! The silent clock these tests read is a function of the real time, and the real time between two ticks of a test is whatever
//! the window's redraw and the executor spent (tens of milliseconds were measured): a test that depends on where inside a frame
//! the clock stands scripts what it reads (`script_wall_clock`) or holds it paused (a held clock does not move). What the age of
//! a reading does to the wait is the pure rule's (`playback_poll_tests.rs`): nothing here waits for real time to pass.
use std::time::{Duration, Instant};

use super::input_tests::fixture;
use super::playback::{Tick, script_wall_clock};
use super::playback_poll::{self, LANDING_GUARD, PollMode};
use super::*;
// The Kit's test macro expands to `#[test]`; the glob import above would resolve it to the Kit macro again.
use core::prelude::v1::test;
use serde_json::{Value, json};

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// A silent playback (no audio device in a test) from frame 0, polled the way `poll` says, with the host trace on.
fn start(view: &Entity<Workstation>, cx: &mut VisualTestContext, poll: PollMode) {
    start_with(view, cx, PlaybackState::new(poll));
}

/// The same, from a playback state built by the caller.
fn start_with(view: &Entity<Workstation>, cx: &mut VisualTestContext, playback: PlaybackState) {
    view.update(cx, |view, cx| {
        view.viewport.trace().set_enabled(true);
        view.playback = playback;
        view.playback.preparing = Some(super::playback::Preparing { from: 0 });
        view.playback_begin(
            0,
            Err(CommandError::new(
                "audio_output_unavailable",
                "no device in a test",
            )),
            cx,
        );
    });
    cx.run_until_parked();
}

/// The frame rate the running playback reports. (The status reads the clock: call it before scripting one.)
fn fps(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> f64 {
    view.read_with(cx, |view, _| view.playback.report()["fps"].as_f64())
        .expect("a running playback reports its rate")
}

fn events_of(view: &Entity<Workstation>, cx: &mut VisualTestContext, kind: &str) -> Vec<Value> {
    view.read_with(cx, |view, _| {
        view.viewport.trace().events_since(0, 1000)["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["kind"] == kind)
            .map(|event| event["fields"].clone())
            .collect()
    })
}

/// Lets the loop's timer fire: the executor's clock moves by `by`, and what was due runs.
fn advance(cx: &mut VisualTestContext, by: Duration) {
    cx.executor().advance_clock(by);
    cx.run_until_parked();
}

/// One tick of the loop run by hand (the timer is not involved), returning the wait it armed.
fn tick(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Option<Duration> {
    view.update(cx, |view, cx| {
        match view.playback_tick(cx, Instant::now(), Duration::ZERO) {
            Tick::Continue { next, .. } => Some(next),
            Tick::Stop => None,
        }
    })
}

/// The wait a traced tick said the loop would make, in milliseconds.
fn next_ms(tick: &Value) -> f64 {
    tick["next_ms"].as_f64().expect("a tick that continues")
}

/// The tick after `ticks` ticks comes `wait` milliseconds after the previous one's deadline, not before and not after: the
/// executor's clock is `already` milliseconds past that deadline, goes to a millisecond short of the wait (no tick), then past it
/// (the tick), and ends a millisecond past the new deadline.
fn expect_next_tick_after(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    wait: f64,
    already: f64,
    ticks: usize,
) {
    advance(cx, Duration::from_secs_f64((wait - already - 1.0) / 1000.0));
    assert_eq!(events_of(view, cx, "tick").len(), ticks, "a tick early");
    advance(cx, Duration::from_millis(2));
    assert_eq!(events_of(view, cx, "tick").len(), ticks + 1, "no tick");
}

#[gpui_kit::test]
fn the_status_and_the_trace_name_the_poll_mode_in_effect(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    for (mode, name) in [(PollMode::Fixed, "fixed"), (PollMode::Deadline, "deadline")] {
        start(&view, cx, mode);
        let status = view.read_with(cx, |view, _| view.playback.report());
        assert_eq!(status["state"], "playing", "{status}");
        assert_eq!(status["poll"], name, "{status}");
        let started = events_of(&view, cx, "play_clock_started");
        assert_eq!(
            started.last().map(|event| event["poll"].clone()),
            Some(json!(name)),
            "{started:?}"
        );
        let stopped = view.update(cx, |view, cx| view.playback_stop(cx));
        assert_eq!(stopped["report"]["poll"], name, "{stopped}");
    }
}

#[gpui_kit::test]
fn a_state_nobody_configured_polls_with_the_previous_schedule(cx: &mut TestAppContext) {
    // What a window without `TESSEL_PLAYBACK_POLL` starts with, until the native A/B accepts the other schedule.
    let (view, cx) = fixture(cx);
    start_with(&view, cx, PlaybackState::default());
    let status = view.read_with(cx, |view, _| view.playback.report());
    assert_eq!(status["poll"], "fixed", "{status}");
}

#[gpui_kit::test]
fn the_fixed_schedule_arms_every_wait_at_half_a_frame(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    start(&view, cx, PollMode::Fixed);
    let cap = playback_poll::interval(fps(&view, cx));
    // The clock sits a quarter of a frame from its boundary, and it is moving: the fixed schedule does not care.
    let _script = script_wall_clock(&[10.75, 10.8]);
    assert_eq!(tick(&view, cx), Some(cap));
    assert_eq!(tick(&view, cx), Some(cap));
}

#[gpui_kit::test]
fn the_deadline_schedule_arms_the_wait_for_the_next_frame_boundary(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    start(&view, cx, PollMode::Deadline);
    let rate = fps(&view, cx);
    let cap = playback_poll::interval(rate);
    // Six tenths into frame 10, then seven tenths: at 24 fps the boundary is then 12.5 ms away, the fixed interval 20.8 ms.
    let _script = script_wall_clock(&[10.6, 10.7]);

    // The first reading has nothing before it, so it is not known to be moving: the fixed interval.
    assert_eq!(tick(&view, cx), Some(cap));

    // The second moved forward: the distance to the boundary (less the little the tick has spent since) plus the guard.
    let wait = tick(&view, cx).expect("the tick continues");
    let landing = 0.3 / rate * 1000.0 + ms(LANDING_GUARD);
    assert!(
        wait >= LANDING_GUARD && ms(wait) <= landing + 0.001,
        "{wait:?} is not the landing {landing} ms"
    );
    // What the tick did after reading the clock (moving the playhead, asking the monitor for the frame) is taken out of the wait:
    // the boundary did not wait for it, so the wait is strictly shorter than the bare landing.
    assert!(
        ms(wait) < landing - 0.001,
        "{wait:?} does not account for the tick's own work, the landing is {landing} ms"
    );
    let ticks = events_of(&view, cx, "tick");
    let traced = ticks.last().expect("a traced tick");
    assert_eq!(traced["clock"], 10.7, "{traced}");
    assert!(
        (next_ms(traced) - ms(wait)).abs() < 0.001,
        "the trace says the wait the tick returned: {traced}"
    );
}

#[gpui_kit::test]
fn a_paused_clock_is_asked_at_the_fixed_interval_in_either_mode(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    start(&view, cx, PollMode::Deadline);
    let rate = fps(&view, cx);
    let cap = playback_poll::interval(rate);
    // A reading far from where the clock will be read next: the first reading after the pause moves forward by a lot.
    let first = script_wall_clock(&[2.0]);
    assert!(tick(&view, cx).is_some());
    drop(first);
    view.update(cx, |view, cx| view.playback_pause(cx).unwrap());

    // Seven tenths into a frame, paused: the boundary is never reached, so it is not waited for, though the reading is ahead of
    // the one before it.
    let held = script_wall_clock(&[10.7]);
    assert_eq!(tick(&view, cx), Some(cap));
    drop(held);

    // Resuming moves the clock again: the first reading that moves is waited for.
    view.update(cx, |view, cx| view.playback_resume(cx).unwrap());
    let _script = script_wall_clock(&[10.8]);
    let wait = tick(&view, cx).expect("the tick continues");
    // 10.7 held, 10.8 read: a fifth of a frame short of the boundary at 11.
    let landing = 0.2 / rate * 1000.0 + ms(LANDING_GUARD);
    assert!(
        wait >= LANDING_GUARD && ms(wait) <= landing + 0.001 && wait < cap,
        "{wait:?} is not the landing {landing} ms"
    );
}

#[gpui_kit::test]
fn a_seek_back_is_not_a_clock_that_is_moving(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    start(&view, cx, PollMode::Deadline);
    let cap = playback_poll::interval(fps(&view, cx));
    // A reading well ahead, then the clock is sent back to seven tenths into an earlier frame: the next reading is behind the one
    // before it, so it says nothing about when its boundary comes.
    let script = script_wall_clock(&[20.2]);
    let _ = tick(&view, cx);
    drop(script);
    view.update(cx, |view, cx| view.playback_seek(5, cx).unwrap());
    let _script = script_wall_clock(&[7.7]);
    assert_eq!(tick(&view, cx), Some(cap));
}

#[gpui_kit::test]
fn the_loop_arms_its_timer_for_the_wait_the_tick_said(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    start(&view, cx, PollMode::Deadline);
    let rate = fps(&view, cx);
    let cap = playback_poll::interval(rate);
    // Three readings: the clock moves a tenth of a frame between ticks, so each tick after the first lands on the boundary.
    let _script = script_wall_clock(&[10.6, 10.7, 10.8]);

    // The loop's own first wait is the fixed one: nothing has been read yet.
    advance(cx, cap - Duration::from_millis(1));
    assert!(events_of(&view, cx, "tick").is_empty(), "a tick early");
    // The executor's clock lands exactly on each deadline, so the steps below are exact.
    advance(cx, Duration::from_millis(1));
    advance(cx, cap);

    let ticks = events_of(&view, cx, "tick");
    assert_eq!(ticks.len(), 2, "{ticks:?}");
    // The first tick said the fixed interval, and the loop made exactly that wait (the second tick fired when the executor's clock
    // moved by it)...
    assert!((next_ms(&ticks[0]) - ms(cap)).abs() < 0.001, "{}", ticks[0]);
    assert_eq!(ticks[1]["clock"], 10.7, "{}", ticks[1]);
    // ...and whatever the second one said is what the loop waits next, a landing and not the fixed interval: not a millisecond less,
    // not a millisecond more. (The executor's clock is exactly on the second tick's deadline.)
    let wait = next_ms(&ticks[1]);
    assert!(
        wait < ms(cap) - 2.0,
        "the second tick should land before the half frame: {}",
        ticks[1]
    );
    expect_next_tick_after(&view, cx, wait, 0.0, 2);

    // And the third reading is waited for in turn: the loop takes each wait from the tick that just ran.
    let ticks = events_of(&view, cx, "tick");
    assert_eq!(ticks[2]["clock"], 10.8, "{}", ticks[2]);
    let wait = next_ms(&ticks[2]);
    assert!(wait < ms(cap) - 2.0, "{}", ticks[2]);
    // (The helper left the executor's clock a millisecond past the third tick's deadline.)
    expect_next_tick_after(&view, cx, wait, 1.0, 3);
}

#[gpui_kit::test]
fn the_fixed_schedule_loop_waits_half_a_frame_whatever_the_clock_says(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    start(&view, cx, PollMode::Fixed);
    let cap = playback_poll::interval(fps(&view, cx));
    let _script = script_wall_clock(&[10.75, 10.8, 10.85]);
    advance(cx, cap);
    advance(cx, cap);
    let ticks = events_of(&view, cx, "tick");
    assert_eq!(ticks.len(), 2, "{ticks:?}");
    for traced in &ticks {
        assert!((next_ms(traced) - ms(cap)).abs() < 0.001, "{traced}");
    }
    advance(cx, cap - Duration::from_millis(1));
    assert_eq!(events_of(&view, cx, "tick").len(), 2, "a tick early");
    advance(cx, Duration::from_millis(2));
    assert_eq!(events_of(&view, cx, "tick").len(), 3, "no tick");
}

#[gpui_kit::test]
fn stopping_ends_the_loop_with_no_wait(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    start(&view, cx, PollMode::Deadline);
    let cap = playback_poll::interval(fps(&view, cx));
    advance(cx, cap);
    view.update(cx, |view, cx| view.playback_stop(cx));
    assert_eq!(
        tick(&view, cx),
        None,
        "a tick with nothing playing waits for nothing"
    );

    // The timer that was already armed fires once, finds nothing playing and ends the loop: no wait, and no more ticks.
    advance(cx, cap * 2);
    let ticks = events_of(&view, cx, "tick");
    let last = ticks.last().unwrap();
    assert_eq!(last["continues"], false, "{last}");
    assert!(last["next_ms"].is_null(), "{last}");
    let count = ticks.len();
    advance(cx, cap * 10);
    assert_eq!(
        events_of(&view, cx, "tick").len(),
        count,
        "the loop went on"
    );
}
