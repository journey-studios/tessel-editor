//! The window of a transition on the edit's timeline (F064, P4b S1a), in exact integer frames.
//!
//! The origin's `transitionWindow` computes in doubles and trusts its inputs; this is the
//! destination's own, checked version. Order of refusals (plan §5.2): a duration below 1 or above
//! the edit is refused **before any arithmetic**, then the window is computed with `checked_*`
//! (an overflow is refused, never wrapped or saturated), then it must lie inside the edit.
//! Edges `0` and `edit` are inside. Nothing here clamps an invalid window into the edit.
//!
//! For a cut `C` (the first frame of the incoming shot) and a duration `D`:
//!
//! * `center`: `start = C − floor(D/2)`, `end = start + D` (floor, so `D = 5` puts 2 frames before
//!   the cut and 3 from it);
//! * `start`: `[C, C + D)`;
//! * `end`: `[C − D, C)`.

use super::transition::Alignment;

/// A window `[start, end)` and how many of its frames fall before the cut (`before`, inside the
/// outgoing shot) and from it (`after`, inside the incoming one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransitionWindow {
    pub start: i64,
    pub end: i64,
    pub before: i64,
    pub after: i64,
}

/// Why a window cannot be had. `OutsideEdit` still carries the window, so the explain can report
/// the frames it needs from the two shots as the oracle does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowError {
    /// `D < 1`.
    DurationBelowOne,
    /// `D` is longer than the whole edit (refused before any arithmetic).
    DurationAboveEdit,
    /// The `i64` arithmetic of the window does not fit.
    Overflow,
    /// `start < 0` or `end > edit`.
    OutsideEdit(TransitionWindow),
}

impl WindowError {
    /// The window an out-of-range error computed, if any.
    pub fn window(self) -> Option<TransitionWindow> {
        match self {
            Self::OutsideEdit(window) => Some(window),
            _ => None,
        }
    }
}

/// The window of a transition of `duration` frames at `cut`, or why it is refused.
pub fn transition_window(
    cut: i64,
    duration: i64,
    alignment: Alignment,
    edit_duration: i64,
) -> Result<TransitionWindow, WindowError> {
    check_duration(duration, edit_duration)?;
    let window = raw_window(cut, duration, alignment).ok_or(WindowError::Overflow)?;
    if window.start < 0 || window.end > edit_duration {
        return Err(WindowError::OutsideEdit(window));
    }
    Ok(window)
}

/// `1 <= duration <= edit_duration`: the rule that runs before any window arithmetic.
pub fn check_duration(duration: i64, edit_duration: i64) -> Result<(), WindowError> {
    if duration < 1 {
        return Err(WindowError::DurationBelowOne);
    }
    if duration > edit_duration {
        return Err(WindowError::DurationAboveEdit);
    }
    Ok(())
}

/// The window with checked arithmetic and no range rule. `None` on overflow. `duration` must
/// already be valid (`>= 1`).
pub(super) fn raw_window(
    cut: i64,
    duration: i64,
    alignment: Alignment,
) -> Option<TransitionWindow> {
    let start = match alignment {
        Alignment::Start => cut,
        // `div_euclid` is floor for the positive duration this receives: the origin's Math.floor.
        Alignment::Center => cut.checked_sub(duration.div_euclid(2))?,
        Alignment::End => cut.checked_sub(duration)?,
    };
    let end = start.checked_add(duration)?;
    let before = end.min(cut).checked_sub(start)?.max(0);
    let after = duration.checked_sub(before)?;
    Some(TransitionWindow {
        start,
        end,
        before,
        after,
    })
}

/// The longest window two shots of `from_frames` and `to_frames` hold for an alignment.
pub fn max_transition_frames(alignment: Alignment, from_frames: i64, to_frames: i64) -> i64 {
    match alignment {
        Alignment::Start => to_frames,
        Alignment::End => from_frames,
        Alignment::Center => from_frames
            .saturating_mul(2)
            .saturating_add(1)
            .min(to_frames.saturating_mul(2)),
    }
}

/// The duration of a transition added without one: the effect's typical length, cut down to what
/// the two shots hold. `None` when they hold nothing (the pilot refuses with `TRANSITION_TOO_LONG`).
pub fn default_transition_duration(
    typical: i64,
    alignment: Alignment,
    from_frames: i64,
    to_frames: i64,
) -> Option<i64> {
    let most = max_transition_frames(alignment, from_frames, to_frames);
    (most > 0).then(|| typical.min(most))
}

/// Frames a clip or instance of `total` frames has after a shot that starts at `start` and runs
/// `length`: never negative (a camera shorter than its shot has none, not a negative tail).
pub fn available_frames(total: i64, start: i64, length: i64) -> i64 {
    total.saturating_sub(start.saturating_add(length)).max(0)
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use super::*;

    fn window(cut: i64, duration: i64, alignment: Alignment) -> (i64, i64) {
        let window = transition_window(cut, duration, alignment, 144).unwrap();
        (window.start, window.end)
    }

    #[test]
    fn windows_at_the_cut_48() {
        assert_eq!(window(48, 12, Alignment::Center), (42, 54));
        assert_eq!(window(48, 5, Alignment::Center), (46, 51));
        assert_eq!(window(48, 1, Alignment::Center), (48, 49));
        assert_eq!(window(48, 12, Alignment::Start), (48, 60));
        assert_eq!(window(48, 12, Alignment::End), (36, 48));
    }

    #[test]
    fn odd_duration_puts_the_extra_frame_after_the_cut() {
        let window = transition_window(48, 5, Alignment::Center, 144).unwrap();
        assert_eq!((window.before, window.after), (2, 3));
    }

    #[test]
    fn edges_of_the_edit_are_inside() {
        assert!(transition_window(48, 48, Alignment::End, 144).is_ok());
        assert!(transition_window(96, 48, Alignment::Start, 144).is_ok());
    }

    #[test]
    fn invalid_duration_is_refused_before_the_arithmetic() {
        assert_eq!(
            transition_window(i64::MAX, 0, Alignment::Center, 144),
            Err(WindowError::DurationBelowOne)
        );
        assert_eq!(
            transition_window(i64::MAX, 145, Alignment::Start, 144),
            Err(WindowError::DurationAboveEdit)
        );
    }

    #[test]
    fn overflow_is_refused_not_wrapped() {
        assert_eq!(
            transition_window(i64::MAX, 12, Alignment::Start, 144),
            Err(WindowError::Overflow)
        );
        assert_eq!(
            transition_window(i64::MIN, 12, Alignment::End, 144),
            Err(WindowError::Overflow)
        );
    }

    #[test]
    fn leaving_the_edit_reports_the_window() {
        let error = transition_window(48, 100, Alignment::Center, 144).unwrap_err();
        assert_eq!(error.window().map(|w| (w.start, w.end)), Some((-2, 98)));
    }

    #[test]
    fn availability_never_goes_negative() {
        assert_eq!(available_frames(40, 5, 40), 0);
        assert_eq!(available_frames(60, 10, 40), 10);
        assert_eq!(available_frames(0, 0, 48), 0);
    }

    #[test]
    fn longest_window_per_alignment() {
        assert_eq!(max_transition_frames(Alignment::Start, 40, 60), 60);
        assert_eq!(max_transition_frames(Alignment::End, 40, 60), 40);
        assert_eq!(max_transition_frames(Alignment::Center, 40, 60), 81);
        assert_eq!(
            default_transition_duration(24, Alignment::Start, 40, 10),
            Some(10)
        );
        assert_eq!(
            default_transition_duration(24, Alignment::Start, 40, 0),
            None
        );
    }
}
