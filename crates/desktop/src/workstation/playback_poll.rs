//! When the playback clock is asked again (BL157.6).
//!
//! The playhead follows a clock that is *asked*: a timer wakes the UI thread, the clock says which frame it is, and a frame that
//! changed is requested from the monitor. Until BL157.6 the timer was armed for a fixed half frame *after* each answer, so with
//! the timer's own lateness (≈ 1.5 ms) a 24 fps frame (41.67 ms) was 1.87 polls long: 13% of the frame changes were seen at the
//! first poll after them and 87% at the second, a regular judder of 22 and 44 ms pictures, and a change was seen 11 ms after it
//! happened on average ([`docs/features/viewport-asset-io.md`](../../../../docs/features/viewport-asset-io.md), BL157.6).
//!
//! [`PollMode::Deadline`] keeps the question at least twice per frame (a late answer is still caught by the next one, and a seek,
//! a resume or a device that re-estimates its clock is seen within half a frame) but, when the next frame boundary is closer than
//! half a frame, arms the question for that boundary plus a small guard: the change is then seen as it happens. It is selected with
//! [`POLL_ENV`] so the same build can be measured against itself; [`PollMode::Fixed`] is the previous schedule.

use std::time::Duration;

/// `TESSEL_PLAYBACK_POLL=fixed|deadline` (BL157.6): how the playback clock is polled. Read once when the window is created; the
/// same build answers both ways. `playback.status` reports the mode in effect (`poll`) and the host trace names it in
/// `play_clock_started`. A value that is neither name is a typo: it is warned about on stderr and the default is used.
pub const POLL_ENV: &str = "TESSEL_PLAYBACK_POLL";

/// The clock is asked at least this many times per frame; a delayed answer only costs skipped frames.
pub const TICKS_PER_FRAME: f64 = 2.0;

/// How long after a frame boundary the deadline question is armed: the clock is an extrapolation between audio callbacks (about
/// 10.7 ms apart), and a question that finds it a hair before the boundary would cost a whole extra wait.
pub const LANDING_GUARD: Duration = Duration::from_millis(1);

/// The wait of a rate that cannot be used (zero, negative, not a number): half a frame of 24 fps. The fixed schedule never had to
/// say what it does then, because such a rate cannot reach it.
const FALLBACK_INTERVAL: Duration = Duration::from_micros(20_833);

/// How the clock is polled (see [`POLL_ENV`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PollMode {
    /// Every half frame after the previous answer: the schedule before BL157.6.
    #[default]
    Fixed,
    /// Every half frame at most, and sooner when the next frame boundary is closer than that: the question lands just after it.
    Deadline,
}

impl PollMode {
    pub fn as_str(self) -> &'static str {
        match self {
            PollMode::Fixed => "fixed",
            PollMode::Deadline => "deadline",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "fixed" => Some(PollMode::Fixed),
            "deadline" => Some(PollMode::Deadline),
            _ => None,
        }
    }
}

/// The mode an environment value asks for: `Ok(None)` when it is unset or empty, `Err(warning)` for a value that is neither name
/// (the default is used).
pub fn poll_mode_from_env(value: Option<&str>) -> Result<Option<PollMode>, String> {
    match value.map(str::trim) {
        None | Some("") => Ok(None),
        Some(text) => PollMode::parse(text).map(Some).ok_or_else(|| {
            format!(
                "{POLL_ENV}={text:?} is not `fixed` or `deadline`: using `{}`",
                PollMode::default().as_str()
            )
        }),
    }
}

/// What the last poll read from the clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
    /// Where the clock stood, in (fractional) timeline frames.
    pub frame: f64,
    /// Whether it had moved forward since the poll before. A held clock (paused, stopped, a device that stalled) never reaches
    /// its next boundary, so it is not waited for.
    pub advancing: bool,
    /// How long ago the clock was read. The tick's own work (moving the playhead, asking the monitor for the frame) runs between
    /// the reading and the arming of the next wait, and the boundary does not wait for it: that time is taken out of the wait.
    pub age: Duration,
}

/// Whether the clock moved forward since the reading before it (`before`, `None` for the first reading of a playback).
///
/// A paused clock never reaches its next frame. A clock that stood still (a device that stalled) or went back (a seek) says
/// nothing about when it will, and the first reading has nothing before it to be compared with: none of them is waited for.
pub fn advancing(paused: bool, before: Option<f64>, now: f64) -> bool {
    !paused && before.is_some_and(|before| now > before)
}

/// The fixed wait: a frame over [`TICKS_PER_FRAME`].
pub fn interval(fps: f64) -> Duration {
    if fps.is_finite() && fps > 0.0 {
        Duration::from_secs_f64(1.0 / (fps * TICKS_PER_FRAME))
    } else {
        FALLBACK_INTERVAL
    }
}

/// How long to wait before asking the clock again.
///
/// [`PollMode::Fixed`] always says [`interval`]. [`PollMode::Deadline`] says the time to the next frame boundary (less the
/// reading's age) plus [`LANDING_GUARD`], and never more than [`interval`] (a boundary further away than half a frame is reached
/// by a mid-frame poll that re-reads the clock, so a seek or a late answer is seen within half a frame). With no reading yet (the
/// device is opening), a clock that is not advancing (held, paused, stalled) or one that is not a number, it also says
/// [`interval`].
pub fn next_poll(mode: PollMode, fps: f64, reading: Option<Reading>) -> Duration {
    let cap = interval(fps);
    if mode != PollMode::Deadline {
        return cap;
    }
    let Some(Reading {
        frame,
        advancing: true,
        age,
    }) = reading
    else {
        return cap;
    };
    if !frame.is_finite() || !fps.is_finite() || fps <= 0.0 {
        return cap;
    }
    // The clock runs at the nominal rate, so the boundary (the next whole frame) was this many seconds away when it was read: in
    // (0, 1 / fps]. (The clamp is for `Duration`, which panics on a negative: the distance is never negative.)
    let to_boundary = ((frame.floor() + 1.0 - frame) / fps).max(0.0);
    (Duration::from_secs_f64(to_boundary).saturating_sub(age) + LANDING_GUARD).min(cap)
}
