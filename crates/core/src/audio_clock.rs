//! BL081.5: the exact sample clock of the mix.
//!
//! One rule maps an **output sample** of the sequence to a **position in a source stream**, in
//! exact rational arithmetic, so a clip cut anywhere keeps its phase:
//!
//! * The output clock is absolute: sample `S(frame) = round(frame / rate * 48 000)` starts frame
//!   `frame`, counted from the first frame of the sequence ([`sample_at`]). A clip sounds on the
//!   samples `S(start) .. S(end)`; a chunk of the sequence is a difference of two `S`, so chunks
//!   written one after the other are the sequence written whole.
//! * Output sample `n` plays the source at the exact time
//!   `source_start + (n / 48 000 - start / rate) * speed - stream.offset` seconds
//!   ([`source_position`]), in source samples. Nothing in that formula is rounded: neither the
//!   boundary of the clip (`S(start)` is not `start / rate` on a fractional grid) nor the step
//!   (`44 100 / 48 000 * speed`).
//!
//! A split keeps `source_start` rational (`source_start + elapsed * speed`), so both halves
//! evaluate the formula of the whole clip sample by sample: no anchor can go stale, because the
//! only stored inputs (`start`, `source_start`, `speed`, the stream offset) are the ones every
//! edit (move, trim, speed, split) already keeps exact.
//!
//! The mixer walks a segment with a [`SourceCursor`]: the same rational position advanced one
//! output sample at a time with integer additions and a carry, never a float accumulator.

use crate::time::{Rate, Ratio};

/// Output sample rate of the mix (the sequence is mixed at this rate, always stereo).
pub use crate::plan::MIX_SAMPLE_RATE;

fn round_div(numerator: i128, denominator: i128) -> i128 {
    // Ties away from zero, like `Ratio::round`.
    let half = denominator / 2;
    if numerator >= 0 {
        (numerator + half) / denominator
    } else {
        (numerator - half) / denominator
    }
}

/// Absolute output sample at which `frame` of a sequence at `rate` starts (nearest sample; the
/// frame boundary is rarely a whole sample at 24000/1001 or 30000/1001). `None` on overflow.
pub fn sample_at(rate: Rate, frame: i64) -> Option<i64> {
    let numerator = i128::from(frame)
        .checked_mul(i128::from(rate.den()))?
        .checked_mul(i128::from(MIX_SAMPLE_RATE))?;
    i64::try_from(round_div(numerator, i128::from(rate.num()))).ok()
}

/// Exact position, in samples of a source stream of `source_rate` Hz, that the output sample `n`
/// (absolute, at [`MIX_SAMPLE_RATE`]) plays, for a clip that starts at sequence frame
/// `clip_start`, reads the asset from `source_start` seconds at `speed` and whose stream starts
/// `stream_offset` seconds after the asset's clock zero. May be negative (before the stream).
pub fn source_position(
    rate: Rate,
    clip_start: i64,
    source_start: Ratio,
    speed: Ratio,
    stream_offset: Ratio,
    source_rate: u32,
    output_sample: i64,
) -> Option<Ratio> {
    let at = Ratio::new(output_sample, i64::from(MIX_SAMPLE_RATE))?;
    let clip_time = Ratio::new(
        clip_start.checked_mul(i64::from(rate.den()))?,
        i64::from(rate.num()),
    )?;
    let elapsed = at.sub(clip_time)?;
    source_start
        .add(elapsed.mul(speed)?)?
        .sub(stream_offset)?
        .mul_int(i64::from(source_rate))
}

/// Source samples read per output sample: `source_rate * speed / output_rate`, exact.
pub fn source_step(source_rate: u32, speed: Ratio, output_rate: u32) -> Option<Ratio> {
    speed
        .mul_int(i64::from(source_rate))?
        .mul(Ratio::new(1, i64::from(output_rate))?)
}

fn gcd(mut a: i128, mut b: i128) -> i128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// An exact, forward-only walk along `start + n * step` (both rational): the whole part and the
/// fraction of every position, with no float accumulation. Positions are `i128` internally, so a
/// walk cannot overflow on any plan whose values fit `Ratio`.
#[derive(Debug, Clone)]
pub struct SourceCursor {
    whole: i128,
    remainder: i128,
    denominator: i128,
    step_whole: i128,
    step_remainder: i128,
}

impl SourceCursor {
    /// Constructs a checked common denominator. With the current i64-backed Ratio,
    /// its arithmetic fits i128; Option retains the checked construction contract.
    pub fn new(start: Ratio, step: Ratio) -> Option<Self> {
        let (a, b) = (i128::from(start.den()), i128::from(step.den()));
        let denominator = (a / gcd(a, b)).checked_mul(b)?;
        let scale = |ratio: Ratio| {
            i128::from(ratio.num()).checked_mul(denominator / i128::from(ratio.den()))
        };
        let (start, step) = (scale(start)?, scale(step)?);
        Some(Self {
            whole: start.div_euclid(denominator),
            remainder: start.rem_euclid(denominator),
            denominator,
            step_whole: step.div_euclid(denominator),
            step_remainder: step.rem_euclid(denominator),
        })
    }

    /// The current position as `(floor, fraction in [0, 1))`, then moves one step on.
    pub fn next_position(&mut self) -> (i128, f32) {
        let fraction = if self.remainder == 0 {
            0.0
        } else {
            (self.remainder as f64 / self.denominator as f64) as f32
        };
        let whole = self.whole;
        self.whole += self.step_whole;
        self.remainder += self.step_remainder;
        if self.remainder >= self.denominator {
            self.remainder -= self.denominator;
            self.whole += 1;
        }
        (whole, fraction)
    }
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use super::*;

    fn exact(num: i64, den: i64) -> Ratio {
        Ratio::new(num, den).unwrap()
    }

    #[test]
    fn the_clock_is_absolute_and_rounds_the_frame_boundary_once() {
        let ntsc = Rate::new(30_000, 1001).unwrap();
        // 1001/30000 s = 1601.6 samples.
        assert_eq!(sample_at(ntsc, 0), Some(0));
        assert_eq!(sample_at(ntsc, 1), Some(1602));
        assert_eq!(sample_at(ntsc, 2), Some(3203));
        assert_eq!(sample_at(Rate::FPS_24, 3), Some(6000));
        assert_eq!(sample_at(Rate::FPS_24, i64::MAX), None);
    }

    #[test]
    fn a_position_is_the_exact_source_time_of_the_output_sample() {
        let ntsc = Rate::new(30_000, 1001).unwrap();
        // A clip at frame 2 (boundary at 3203.2 samples) that reads from 0.25 s at speed 1: its
        // first sample is output sample 3203, 0.2 sample *before* the boundary, and the source is
        // read there (11 999.8), not at 0.25 s (12 000).
        let at = source_position(
            ntsc,
            2,
            exact(1, 4),
            Ratio::from_int(1),
            Ratio::ZERO,
            48_000,
            3203,
        )
        .unwrap();
        assert_eq!(at, exact(59_999, 5));
    }

    #[test]
    fn the_cursor_walks_a_rational_step_without_drift() {
        // 44 100 / 48 000 = 147 / 160 source samples per output sample; start at 1/3.
        let step = source_step(44_100, Ratio::from_int(1), 48_000).unwrap();
        assert_eq!(step, exact(147, 160));
        let mut cursor = SourceCursor::new(exact(1, 3), step).unwrap();
        for n in 0..10_000i128 {
            let (whole, fraction) = cursor.next_position();
            // Exact: floor((1/3 + 147 n / 160)) with denominator 480.
            let numerator = 160 + 441 * n;
            assert_eq!(whole, numerator.div_euclid(480), "n={n}");
            let expected = numerator.rem_euclid(480) as f64 / 480.0;
            assert_eq!(fraction, expected as f32, "n={n}");
        }
        // A negative start floors toward minus infinity and keeps a fraction in [0, 1).
        let mut early = SourceCursor::new(exact(-1, 2), Ratio::from_int(1)).unwrap();
        assert_eq!(early.next_position(), (-1, 0.5));
        assert_eq!(early.next_position(), (0, 0.5));
    }

    #[test]
    fn the_cursor_handles_exact_carries_and_negative_fractional_positions() {
        let mut cursor = SourceCursor::new(exact(-1, 4), exact(1, 4)).unwrap();
        assert_eq!(cursor.next_position(), (-1, 0.75));
        assert_eq!(cursor.next_position(), (0, 0.0));
        assert_eq!(cursor.next_position(), (0, 0.25));
        let mut cursor = SourceCursor::new(exact(1, 4), exact(3, 4)).unwrap();
        assert_eq!(cursor.next_position(), (0, 0.25));
        assert_eq!(cursor.next_position(), (1, 0.0));
        assert_eq!(cursor.next_position(), (1, 0.75));
        assert_eq!(cursor.next_position(), (2, 0.5));
    }
}
