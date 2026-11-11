//! The playback clock's poll schedule (BL157.6): pure, with no window and no timer.
//!
//! The first group runs the schedule against a model of the loop in `playback.rs` (a perfect clock, a timer that answers a little
//! late, a tick that reads the clock and asks the schedule for the next wait). The model has to reproduce what the native A/B
//! measured for the fixed schedule (13% of the pictures 22 ms after the previous one, 87% 44 ms after) before its answer for the
//! deadline schedule means anything: that is the control of these tests.

use std::time::Duration;

use super::playback_poll::{
    LANDING_GUARD, POLL_ENV, PollMode, Reading, TICKS_PER_FRAME, advancing, interval, next_poll,
    poll_mode_from_env,
};

const FPS: f64 = 24.0;
/// How late the native sessions' timer answered on average (the hop to the UI thread included).
const MEASURED_LATE: Duration = Duration::from_micros(1_460);

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn frame_ms(fps: f64) -> f64 {
    1000.0 / fps
}

/// One poll of the modelled loop.
struct Poll {
    /// Seconds since the clock started.
    at: f64,
    /// Where the clock stood, in frames.
    clock: f64,
    /// Whether the frame under the playhead changed at this poll.
    changed: bool,
}

/// The loop of `playback.rs` against a perfect clock (frame `n` starts at `n / fps` seconds): the timer answers `late(index)`
/// after it was due, the clock is read there, the tick's own work takes `body`, and the next wait (armed when that work is done)
/// comes from [`next_poll`].
fn run_with_body(
    mode: PollMode,
    fps: f64,
    seconds: f64,
    late: impl Fn(usize) -> Duration,
    body: Duration,
) -> Vec<Poll> {
    let mut polls = Vec::new();
    let (mut at, mut delay) = (0.0_f64, interval(fps));
    let (mut last_clock, mut last_frame): (Option<f64>, i64) = (None, 0);
    while at < seconds {
        at += delay.as_secs_f64() + late(polls.len()).as_secs_f64();
        let clock = at * fps;
        let advancing = last_clock.is_some_and(|last| clock > last);
        let frame = clock.floor() as i64;
        polls.push(Poll {
            at,
            clock,
            changed: frame != last_frame,
        });
        last_clock = Some(clock);
        last_frame = frame;
        at += body.as_secs_f64();
        delay = next_poll(
            mode,
            fps,
            Some(Reading {
                frame: clock,
                advancing,
                age: body,
            }),
        );
    }
    polls
}

fn run_with(mode: PollMode, fps: f64, seconds: f64, late: impl Fn(usize) -> Duration) -> Vec<Poll> {
    run_with_body(mode, fps, seconds, late, Duration::ZERO)
}

fn run(mode: PollMode, fps: f64, seconds: f64, late: Duration) -> Vec<Poll> {
    run_with(mode, fps, seconds, |_| late)
}

/// Milliseconds between consecutive pictures (frame changes seen) after the first `warmup` seconds.
fn delivered_gaps(polls: &[Poll], warmup: f64) -> Vec<f64> {
    let times: Vec<f64> = polls
        .iter()
        .filter(|poll| poll.changed && poll.at >= warmup)
        .map(|poll| poll.at * 1000.0)
        .collect();
    times.windows(2).map(|pair| pair[1] - pair[0]).collect()
}

/// How long after its boundary each frame change was seen (the clock's fraction of a frame, in milliseconds).
fn change_lags(polls: &[Poll], fps: f64, warmup: f64) -> Vec<f64> {
    polls
        .iter()
        .filter(|poll| poll.changed && poll.at >= warmup)
        .map(|poll| (poll.clock - poll.clock.floor()) / fps * 1000.0)
        .collect()
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

fn share(values: &[f64], test: impl Fn(f64) -> bool) -> f64 {
    values.iter().filter(|value| test(**value)).count() as f64 / values.len() as f64
}

fn max(values: &[f64]) -> f64 {
    values.iter().copied().fold(f64::MIN, f64::max)
}

fn min(values: &[f64]) -> f64 {
    values.iter().copied().fold(f64::MAX, f64::min)
}

fn reading(frame: f64) -> Option<Reading> {
    aged(frame, Duration::ZERO)
}

/// A clock that is moving, read `age` ago.
fn aged(frame: f64, age: Duration) -> Option<Reading> {
    Some(Reading {
        frame,
        advancing: true,
        age,
    })
}

// ---- the model first: the control

#[test]
fn the_fixed_schedule_reproduces_the_pattern_the_native_runs_measured() {
    let polls = run(PollMode::Fixed, FPS, 120.0, MEASURED_LATE);
    let gaps = delivered_gaps(&polls, 1.0);
    assert!(gaps.len() > 2_500, "{} deliveries", gaps.len());

    // 13.0% (every session group of the native A/B) of the pictures came less than 30 ms after the previous one.
    let short = share(&gaps, |gap| gap < 30.0);
    assert!(
        (0.12..=0.14).contains(&short),
        "{short} of the deliveries were short"
    );
    // The poll is 22.29 ms (20.83 + the timer's 1.46): a frame is seen after one poll (22.3 ms) or after two (44.6 ms), and nothing else.
    let poll = ms(interval(FPS)) + ms(MEASURED_LATE);
    assert!(
        gaps.iter()
            .all(|gap| { (gap - poll).abs() < 0.05 || (gap - 2.0 * poll).abs() < 0.05 }),
        "a delivery that is neither one nor two polls apart"
    );
    // No frame is lost: the pattern is a judder, the average is the frame.
    assert!(
        (mean(&gaps) - frame_ms(FPS)).abs() < 0.05,
        "{} ms on average",
        mean(&gaps)
    );
}

#[test]
fn the_fixed_schedule_sees_a_frame_change_up_to_a_poll_after_it_happened() {
    let polls = run(PollMode::Fixed, FPS, 120.0, MEASURED_LATE);
    let lags = change_lags(&polls, FPS, 1.0);
    // Uniform between 0 and one poll (22.3 ms): about 11 ms late on average.
    assert!((8.0..=14.0).contains(&mean(&lags)), "{} ms", mean(&lags));
    assert!(max(&lags) > 20.0, "{} ms", max(&lags));
}

// ---- the deadline schedule

#[test]
fn the_deadline_schedule_delivers_each_frame_on_its_boundary() {
    let polls = run(PollMode::Deadline, FPS, 120.0, MEASURED_LATE);
    let gaps = delivered_gaps(&polls, 1.0);
    assert!(gaps.len() > 2_500, "{} deliveries", gaps.len());

    assert_eq!(
        share(&gaps, |gap| gap < 30.0),
        0.0,
        "a picture that came in the catch-up step of the old pattern"
    );
    assert!(
        gaps.iter().all(|gap| (gap - frame_ms(FPS)).abs() < 2.0),
        "gaps from {} to {} ms",
        min(&gaps),
        max(&gaps)
    );
    assert!((mean(&gaps) - frame_ms(FPS)).abs() < 0.05);
}

#[test]
fn the_deadline_schedule_sees_a_frame_change_just_after_its_boundary() {
    let polls = run(PollMode::Deadline, FPS, 120.0, MEASURED_LATE);
    let lags = change_lags(&polls, FPS, 1.0);
    // The guard plus the timer's lateness, and nothing more.
    let landing = ms(LANDING_GUARD) + ms(MEASURED_LATE);
    assert!(max(&lags) < landing + 0.2, "{} ms", max(&lags));
    assert!(mean(&lags) > landing - 0.2, "{} ms", mean(&lags));

    // And that is the gain: the same model, the same timer, about 8 ms less on average than the fixed schedule.
    let fixed = change_lags(&run(PollMode::Fixed, FPS, 120.0, MEASURED_LATE), FPS, 1.0);
    assert!(
        mean(&fixed) - mean(&lags) > 5.0,
        "fixed {} ms, deadline {} ms",
        mean(&fixed),
        mean(&lags)
    );
}

#[test]
fn the_deadline_schedule_lands_after_the_boundary_whatever_the_timer_lateness() {
    for late_us in [0_u64, 500, 1_460, 3_000, 5_000] {
        let late = Duration::from_micros(late_us);
        let polls = run(PollMode::Deadline, FPS, 60.0, late);
        let lags = change_lags(&polls, FPS, 1.0);
        let landing = ms(LANDING_GUARD) + ms(late);
        assert!(
            max(&lags) < landing + 0.2,
            "late {late:?}: {} ms after the boundary",
            max(&lags)
        );
        assert!(
            min(&lags) > 0.0,
            "late {late:?}: a frame change seen before its boundary"
        );
        let gaps = delivered_gaps(&polls, 1.0);
        assert!(
            gaps.iter().all(|gap| (gap - frame_ms(FPS)).abs() < 2.0),
            "late {late:?}: gaps from {} to {} ms",
            min(&gaps),
            max(&gaps)
        );
    }
}

#[test]
fn the_deadline_schedule_asks_about_as_often_as_the_fixed_one() {
    let seconds = 60.0;
    let fixed = run(PollMode::Fixed, FPS, seconds, MEASURED_LATE).len() as f64;
    let deadline = run(PollMode::Deadline, FPS, seconds, MEASURED_LATE).len() as f64;
    let frames = seconds * FPS;
    // 1.87 polls per frame for the fixed schedule, 2 for the deadline one (the mid-frame poll stays: it is the safety).
    assert!((fixed / frames - 1.87).abs() < 0.03, "{}", fixed / frames);
    assert!(
        (1.95..=2.1).contains(&(deadline / frames)),
        "{} polls per frame",
        deadline / frames
    );
}

#[test]
fn a_poll_that_runs_two_frames_late_is_caught_up_and_the_landing_comes_back() {
    // One poll, early in the run, answers 100 ms late: the UI thread was busy. The clock is read there (the frame jumps) and the
    // very next waits are computed from that reading: the boundary landing is back within a few polls.
    let stalled = 200;
    let polls = run_with(PollMode::Deadline, FPS, 30.0, |index| {
        if index == stalled {
            Duration::from_millis(100)
        } else {
            MEASURED_LATE
        }
    });
    let jump = polls[stalled].clock.floor() - polls[stalled - 1].clock.floor();
    assert!(jump >= 2.0, "the stall skipped {jump} frames");
    let landing = ms(LANDING_GUARD) + ms(MEASURED_LATE);
    let recovered: Vec<f64> = polls[stalled + 1..stalled + 6]
        .iter()
        .filter(|poll| poll.changed)
        .map(|poll| (poll.clock - poll.clock.floor()) / FPS * 1000.0)
        .collect();
    assert!(
        recovered.iter().any(|lag| *lag < landing + 0.2),
        "no landing back on the boundary within five polls: {recovered:?}"
    );
}

// ---- the rule of one wait

#[test]
fn the_wait_lands_just_after_the_next_boundary() {
    // At 10.75 the next boundary (11.0) is a quarter of a frame away: 10.417 ms at 24 fps, plus the guard.
    let wait = next_poll(PollMode::Deadline, FPS, reading(10.75));
    let expected = 0.25 * frame_ms(FPS) + ms(LANDING_GUARD);
    assert!(
        (ms(wait) - expected).abs() < 0.001,
        "{} ms, expected {expected}",
        ms(wait)
    );
}

#[test]
fn the_work_of_the_tick_is_not_waited_for_twice() {
    // The clock said 10.75 five milliseconds before the wait was armed: the boundary is that much closer.
    let wait = next_poll(
        PollMode::Deadline,
        FPS,
        aged(10.75, Duration::from_millis(5)),
    );
    let expected = 0.25 * frame_ms(FPS) - 5.0 + ms(LANDING_GUARD);
    assert!(
        (ms(wait) - expected).abs() < 0.001,
        "{} ms, expected {expected}",
        ms(wait)
    );
}

#[test]
fn a_tick_that_ran_past_the_boundary_waits_only_the_guard() {
    // The work took longer than the quarter frame that was left: the change is already there to be read.
    let wait = next_poll(
        PollMode::Deadline,
        FPS,
        aged(10.99, Duration::from_millis(50)),
    );
    assert_eq!(wait, LANDING_GUARD);
}

#[test]
fn the_age_only_shortens_the_wait_and_the_fixed_schedule_does_not_read_it() {
    for frame in [10.1, 10.5, 10.75, 10.95] {
        let mut previous = next_poll(PollMode::Deadline, FPS, reading(frame));
        for age_us in [100_u64, 500, 2_000, 8_000, 30_000] {
            let age = Duration::from_micros(age_us);
            let wait = next_poll(PollMode::Deadline, FPS, aged(frame, age));
            assert!(
                wait <= previous,
                "{frame} aged {age:?}: {wait:?} > {previous:?}"
            );
            previous = wait;
            assert_eq!(
                next_poll(PollMode::Fixed, FPS, aged(frame, age)),
                interval(FPS),
                "{frame} aged {age:?}"
            );
        }
    }
}

#[test]
fn the_deadline_schedule_lands_on_the_boundary_even_when_the_tick_has_work_to_do() {
    // A tick that takes 3 ms (it moves the playhead and asks the monitor for the frame): the landing is where it was.
    let body = Duration::from_millis(3);
    let polls = run_with_body(PollMode::Deadline, FPS, 60.0, |_| MEASURED_LATE, body);
    let lags = change_lags(&polls, FPS, 1.0);
    let landing = ms(LANDING_GUARD) + ms(MEASURED_LATE);
    assert!(max(&lags) < landing + 0.2, "{} ms", max(&lags));
    assert!(min(&lags) > landing - 0.2, "{} ms", min(&lags));
    let gaps = delivered_gaps(&polls, 1.0);
    assert!(
        gaps.iter().all(|gap| (gap - frame_ms(FPS)).abs() < 2.0),
        "gaps from {} to {} ms",
        min(&gaps),
        max(&gaps)
    );
}

#[test]
fn a_boundary_further_away_than_the_fixed_interval_keeps_the_fixed_interval() {
    // Just after a boundary the next one is a whole frame away: the poll stays at half a frame (the mid-frame poll).
    for frame in [10.0, 10.05, 10.2] {
        assert_eq!(
            next_poll(PollMode::Deadline, FPS, reading(frame)),
            interval(FPS),
            "at {frame}"
        );
    }
}

#[test]
fn a_poll_a_hair_before_the_boundary_still_waits_the_guard() {
    // An early wake: the clock says 10.999999. The next question is not a tight loop.
    let wait = next_poll(PollMode::Deadline, FPS, reading(10.999_999));
    assert!(wait >= LANDING_GUARD, "{wait:?}");
    assert!(
        ms(wait) < ms(LANDING_GUARD) + 0.01,
        "{} ms: the boundary was a hair away",
        ms(wait)
    );
}

#[test]
fn a_clock_that_is_not_advancing_is_asked_at_the_fixed_interval() {
    // Paused, held, a device that stopped: the boundary is never reached, so waiting for it would spin or sleep for nothing.
    let held = Some(Reading {
        frame: 10.999,
        advancing: false,
        age: Duration::ZERO,
    });
    assert_eq!(next_poll(PollMode::Deadline, FPS, held), interval(FPS));
    let held_and_old = Some(Reading {
        frame: 10.999,
        advancing: false,
        age: Duration::from_millis(30),
    });
    assert_eq!(
        next_poll(PollMode::Deadline, FPS, held_and_old),
        interval(FPS)
    );
}

#[test]
fn no_reading_yet_is_asked_at_the_fixed_interval() {
    // The device is still opening: there is no clock to read.
    assert_eq!(next_poll(PollMode::Deadline, FPS, None), interval(FPS));
}

// ---- when a clock counts as moving

#[test]
fn a_clock_ahead_of_its_last_reading_is_advancing() {
    assert!(advancing(false, Some(10.0), 10.5));
    assert!(advancing(false, Some(10.0), 10.000_001));
    assert!(advancing(false, Some(-1.0), 0.0));
}

#[test]
fn a_clock_level_with_its_last_reading_is_not_advancing() {
    // A device that stopped answering: the same position twice. Its next boundary is not coming.
    assert!(!advancing(false, Some(10.5), 10.5));
    assert!(!advancing(false, Some(0.0), 0.0));
}

#[test]
fn a_clock_behind_its_last_reading_is_not_advancing() {
    // A seek back: the position says nothing about the time to the next boundary.
    assert!(!advancing(false, Some(100.2), 30.7));
    assert!(!advancing(false, Some(10.0), 9.999_999));
}

#[test]
fn a_paused_clock_is_not_advancing_whatever_it_reads() {
    assert!(!advancing(true, Some(10.0), 10.5));
    assert!(!advancing(true, Some(10.0), 10.0));
    assert!(!advancing(true, Some(10.5), 10.0));
    assert!(!advancing(true, None, 10.0));
}

#[test]
fn the_first_reading_is_not_advancing_because_nothing_is_before_it() {
    assert!(!advancing(false, None, 10.5));
    assert!(!advancing(false, None, 0.0));
}

#[test]
fn readings_that_are_not_numbers_are_not_advancing() {
    assert!(!advancing(false, Some(10.0), f64::NAN));
    assert!(!advancing(false, Some(f64::NAN), 10.0));
    assert!(!advancing(false, Some(f64::NAN), f64::NAN));
}

#[test]
fn what_is_advancing_is_waited_for_and_what_is_not_is_not() {
    // The two rules together, the way the loop uses them: a reading that is not advancing never shortens the wait.
    for (paused, before, now) in [
        (false, Some(10.0), 10.0),
        (false, Some(11.0), 10.6),
        (true, Some(10.0), 10.6),
        (false, None, 10.6),
    ] {
        let reading = Some(Reading {
            frame: now,
            advancing: advancing(paused, before, now),
            age: Duration::ZERO,
        });
        assert_eq!(
            next_poll(PollMode::Deadline, FPS, reading),
            interval(FPS),
            "{paused} {before:?} {now}"
        );
    }
    let moving = Some(Reading {
        frame: 10.6,
        advancing: advancing(false, Some(10.5), 10.6),
        age: Duration::ZERO,
    });
    assert!(next_poll(PollMode::Deadline, FPS, moving) < interval(FPS));
}

#[test]
fn the_fixed_mode_ignores_the_clock() {
    for frame in [0.0, 10.0, 10.5, 10.75, 10.999_999, 1_000.3] {
        assert_eq!(
            next_poll(PollMode::Fixed, FPS, reading(frame)),
            interval(FPS),
            "at {frame}"
        );
    }
    assert_eq!(next_poll(PollMode::Fixed, FPS, None), interval(FPS));
}

#[test]
fn the_wait_is_never_shorter_than_the_guard_and_never_longer_than_the_fixed_interval() {
    for fps in [24.0, 25.0, 29.97, 30.0, 48.0, 50.0, 59.94, 60.0, 120.0] {
        let cap = interval(fps);
        for step in 0..1_000 {
            let frame = 100.0 + (f64::from(step) + 0.5) / 1_000.0;
            let wait = next_poll(PollMode::Deadline, fps, reading(frame));
            assert!(wait >= LANDING_GUARD, "{fps} fps at {frame}: {wait:?}");
            assert!(wait <= cap, "{fps} fps at {frame}: {wait:?} over {cap:?}");
        }
    }
}

#[test]
fn a_rate_whose_fixed_interval_is_shorter_than_the_guard_never_waits_longer_than_it() {
    let cap = interval(1_000.0);
    assert!(cap < LANDING_GUARD);
    assert_eq!(next_poll(PollMode::Deadline, 1_000.0, reading(5.9)), cap);
}

#[test]
fn readings_and_rates_that_are_not_numbers_fall_back_to_the_fixed_interval() {
    for frame in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            next_poll(PollMode::Deadline, FPS, reading(frame)),
            interval(FPS)
        );
    }
    for fps in [f64::NAN, f64::INFINITY, 0.0, -24.0] {
        // No panic, and a wait that is a half frame of the usual 24 fps, not zero.
        let fallback = interval(fps);
        assert_eq!(next_poll(PollMode::Fixed, fps, reading(1.5)), fallback);
        assert_eq!(next_poll(PollMode::Deadline, fps, reading(1.5)), fallback);
        assert!(
            (ms(fallback) - ms(interval(24.0))).abs() < 0.001,
            "{fallback:?}"
        );
    }
}

#[test]
fn the_guard_is_the_one_millisecond_the_native_ab_measured() {
    // The landing numbers of the model above and of the A/B are for this value: a change is a new experiment, not a tweak.
    assert_eq!(LANDING_GUARD, Duration::from_millis(1));
}

#[test]
fn the_fixed_interval_is_a_frame_over_the_ticks_per_frame() {
    assert_eq!(TICKS_PER_FRAME, 2.0, "two questions per frame, as before");
    assert!((ms(interval(24.0)) - 20.833).abs() < 0.001);
    assert!((ms(interval(30.0)) - 16.667).abs() < 0.001);
    assert!((ms(interval(60.0)) - 8.333).abs() < 0.001);
}

// ---- the switch

#[test]
fn the_environment_names_the_two_modes_whatever_the_case_and_spacing() {
    assert_eq!(
        poll_mode_from_env(Some("deadline")),
        Ok(Some(PollMode::Deadline))
    );
    assert_eq!(
        poll_mode_from_env(Some("  Deadline ")),
        Ok(Some(PollMode::Deadline))
    );
    assert_eq!(poll_mode_from_env(Some("FIXED")), Ok(Some(PollMode::Fixed)));
    assert_eq!(poll_mode_from_env(Some("fixed")), Ok(Some(PollMode::Fixed)));
}

#[test]
fn unset_or_empty_keeps_the_default() {
    assert_eq!(poll_mode_from_env(None), Ok(None));
    assert_eq!(poll_mode_from_env(Some("")), Ok(None));
    assert_eq!(poll_mode_from_env(Some("   ")), Ok(None));
}

#[test]
fn a_typo_is_a_warning_that_names_the_variable_and_the_mode_in_use() {
    let warning = poll_mode_from_env(Some("deadlines")).unwrap_err();
    assert!(warning.contains(POLL_ENV), "{warning}");
    assert!(warning.contains("deadlines"), "{warning}");
    assert!(
        warning.contains("`fixed`") && warning.contains("`deadline`"),
        "{warning}"
    );
    assert!(
        warning.contains(&format!("using `{}`", PollMode::default().as_str())),
        "{warning}"
    );
}

#[test]
fn the_names_round_trip_and_the_variable_is_the_documented_one() {
    for mode in [PollMode::Fixed, PollMode::Deadline] {
        assert_eq!(PollMode::parse(mode.as_str()), Some(mode));
    }
    assert_eq!(PollMode::Fixed.as_str(), "fixed");
    assert_eq!(PollMode::Deadline.as_str(), "deadline");
    assert_eq!(POLL_ENV, "TESSEL_PLAYBACK_POLL");
    assert_eq!(PollMode::parse("nonsense"), None);
    // The parser is not only reached through the environment reader: it ignores case and spacing itself.
    assert_eq!(PollMode::parse("  DEADLINE\n"), Some(PollMode::Deadline));
    assert_eq!(PollMode::parse(" Fixed "), Some(PollMode::Fixed));
}

#[test]
fn the_default_is_the_fixed_schedule_until_the_native_ab_accepts_the_deadline_one() {
    assert_eq!(PollMode::default(), PollMode::Fixed);
}
