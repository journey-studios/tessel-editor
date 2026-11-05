//! Exact time: rational seconds and rational frame rates.
//!
//! WHY rational: accumulating `f64` seconds drifts (`n / 24 * 24 != n` for many `n`)
//! and 23.976/29.97 fps are `24000/1001` and `30000/1001`. All mapping between
//! sequence frames, source time and source frames stays in integer arithmetic;
//! floats appear only at the edge (runtime uniforms, JSON convenience fields).

use std::cmp::Ordering;
use std::fmt;

use serde_json::{Value, json};

fn gcd(mut a: i128, mut b: i128) -> i128 {
    a = a.abs();
    b = b.abs();
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// A reduced fraction with a positive denominator.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ratio {
    num: i64,
    den: i64,
}

impl Ratio {
    pub const ZERO: Self = Self { num: 0, den: 1 };

    /// `None` for a zero denominator or a value that does not fit in `i64` once reduced.
    pub fn new(num: i64, den: i64) -> Option<Self> {
        Self::from_wide(i128::from(num), i128::from(den))
    }

    fn from_wide(num: i128, den: i128) -> Option<Self> {
        if den == 0 {
            return None;
        }
        let sign = if den < 0 { -1 } else { 1 };
        let divisor = gcd(num, den).max(1);
        let num = i64::try_from(sign * num / divisor).ok()?;
        let den = i64::try_from(sign * den / divisor).ok()?;
        Some(Self { num, den })
    }

    pub const fn from_int(value: i64) -> Self {
        Self { num: value, den: 1 }
    }

    pub fn num(self) -> i64 {
        self.num
    }

    pub fn den(self) -> i64 {
        self.den
    }

    // WHY not `std::ops::Add`: these are overflow-checked and return `Option`; an operator
    // impl would have to panic or wrap on overflow, which a time computation must not do.
    #[allow(clippy::should_implement_trait)]
    pub fn add(self, other: Self) -> Option<Self> {
        Self::from_wide(
            i128::from(self.num) * i128::from(other.den)
                + i128::from(other.num) * i128::from(self.den),
            i128::from(self.den) * i128::from(other.den),
        )
    }

    #[allow(clippy::should_implement_trait)]
    pub fn sub(self, other: Self) -> Option<Self> {
        self.add(Self {
            num: other.num.checked_neg()?,
            den: other.den,
        })
    }

    #[allow(clippy::should_implement_trait)]
    pub fn mul(self, other: Self) -> Option<Self> {
        Self::from_wide(
            i128::from(self.num) * i128::from(other.num),
            i128::from(self.den) * i128::from(other.den),
        )
    }

    pub fn mul_int(self, value: i64) -> Option<Self> {
        Self::from_wide(
            i128::from(self.num) * i128::from(value),
            i128::from(self.den),
        )
    }

    pub fn to_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// Largest integer not above the value.
    pub fn floor(self) -> i64 {
        self.num.div_euclid(self.den)
    }

    /// Nearest integer, ties away from zero.
    pub fn round(self) -> i64 {
        let twice =
            i128::from(self.num) * 2 + i128::from(self.den) * if self.num < 0 { -1 } else { 1 };
        (twice / (i128::from(self.den) * 2)) as i64
    }

    pub fn is_negative(self) -> bool {
        self.num < 0
    }

    pub fn to_json(self) -> Value {
        json!({ "num": self.num, "den": self.den, "seconds": self.to_f64() })
    }

    pub fn from_json(value: &Value) -> Option<Self> {
        if let Some(int) = value.as_i64() {
            return Some(Self::from_int(int));
        }
        Self::new(value.get("num")?.as_i64()?, value.get("den")?.as_i64()?)
    }
}

impl Ord for Ratio {
    fn cmp(&self, other: &Self) -> Ordering {
        (i128::from(self.num) * i128::from(other.den))
            .cmp(&(i128::from(other.num) * i128::from(self.den)))
    }
}

impl PartialOrd for Ratio {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Debug for Ratio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.num, self.den)
    }
}

impl fmt::Display for Ratio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

/// Frames per second as `num / den` (24/1, 24000/1001, ...).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rate {
    num: u32,
    den: u32,
}

impl Rate {
    pub const FPS_24: Self = Self { num: 24, den: 1 };

    /// Rates must be positive and at most 1000 fps; the reduced form is stored so
    /// `48/2` equals `24/1`.
    pub fn new(num: u32, den: u32) -> Option<Self> {
        if num == 0 || den == 0 || u64::from(num) > 1000 * u64::from(den) {
            return None;
        }
        let divisor = gcd(i128::from(num), i128::from(den)) as u32;
        Some(Self {
            num: num / divisor,
            den: den / divisor,
        })
    }

    pub fn num(self) -> u32 {
        self.num
    }

    pub fn den(self) -> u32 {
        self.den
    }

    pub fn as_f64(self) -> f64 {
        f64::from(self.num) / f64::from(self.den)
    }

    /// Whole frames per second used by timecode and ruler ticks (24000/1001 shows 24).
    pub fn nominal_fps(self) -> i64 {
        ((i64::from(self.num) * 2 + i64::from(self.den)) / (i64::from(self.den) * 2)).max(1)
    }

    /// Start time of `frame`, in seconds: `frame * den / num`.
    pub fn frame_to_time(self, frame: i64) -> Ratio {
        Ratio::from_wide(
            i128::from(frame) * i128::from(self.den),
            i128::from(self.num),
        )
        // Unreachable for realistic frame counts; ZERO keeps the signature total.
        .unwrap_or(Ratio::ZERO)
    }

    /// Frame whose interval contains `time` (floor).
    pub fn time_to_frame_floor(self, time: Ratio) -> i64 {
        let scaled = i128::from(time.num) * i128::from(self.num);
        let divisor = i128::from(time.den) * i128::from(self.den);
        scaled.div_euclid(divisor) as i64
    }

    pub fn to_json(self) -> Value {
        json!({ "num": self.num, "den": self.den, "fps": self.as_f64() })
    }

    pub fn from_json(value: &Value) -> Option<Self> {
        if let Some(fps) = value.as_u64() {
            return Self::new(u32::try_from(fps).ok()?, 1);
        }
        Self::new(
            u32::try_from(value.get("num")?.as_u64()?).ok()?,
            u32::try_from(value.get("den").and_then(Value::as_u64).unwrap_or(1)).ok()?,
        )
    }
}

impl fmt::Debug for Rate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}fps", self.num, self.den)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratios_reduce_and_compare_exactly() {
        let third = Ratio::new(2, 6).unwrap();
        assert_eq!((third.num(), third.den()), (1, 3));
        assert_eq!(Ratio::new(1, -2).unwrap(), Ratio::new(-1, 2).unwrap());
        assert!(Ratio::new(1, 0).is_none());
        assert!(Ratio::new(1, 3).unwrap() < Ratio::new(334, 1000).unwrap());
        let sum = third.add(third).unwrap().add(third).unwrap();
        assert_eq!(sum, Ratio::from_int(1));
    }

    #[test]
    fn floor_and_round_handle_negatives() {
        assert_eq!(Ratio::new(-1, 2).unwrap().floor(), -1);
        assert_eq!(Ratio::new(3, 2).unwrap().round(), 2);
        assert_eq!(Ratio::new(-3, 2).unwrap().round(), -2);
        assert_eq!(Ratio::new(1, 3).unwrap().round(), 0);
    }

    #[test]
    fn frame_time_round_trips_without_drift_at_ntsc_rates() {
        let rate = Rate::new(24000, 1001).unwrap();
        assert_eq!(rate.nominal_fps(), 24);
        for frame in [0, 1, 23, 24, 1000, 86_399, 1_000_000] {
            assert_eq!(rate.time_to_frame_floor(rate.frame_to_time(frame)), frame);
        }
        // One tick before the next frame boundary still belongs to the previous frame.
        let almost = rate
            .frame_to_time(10)
            .sub(Ratio::new(1, 1_000_000).unwrap())
            .unwrap();
        assert_eq!(rate.time_to_frame_floor(almost), 9);
    }

    #[test]
    fn rates_are_reduced_and_bounded() {
        assert_eq!(Rate::new(48, 2).unwrap(), Rate::FPS_24);
        assert!(Rate::new(0, 1).is_none());
        assert!(Rate::new(24, 0).is_none());
        assert!(Rate::new(1001, 1).is_none());
    }
}
