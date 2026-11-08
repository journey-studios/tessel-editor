//! F042A: the audio of a directed-scene export, the domain half (BL016).
//!
//! A bundle declares **one mix file** (`audio.mixRelativePath`) and a window of it
//! (`editStartSeconds`..`editEndSecondsExclusive`). The window is how the origin's transport reads the
//! file: scene edit time `t` plays the file at `editStartSeconds + t`, and nothing is played at or past
//! `editEndSecondsExclusive`. This module turns that declaration plus an export's exact
//! [`SceneExportSchedule`] into [`DirectedAudioMix`]: which instant of the file each sample of the
//! exported 48 kHz stereo stream comes from, in exact rational time, never in floats.
//!
//! The core decides *what* sound goes where and what is refused; it never opens a file. The media
//! adapter reads the WAV header into [`SourceFacts`], asks [`DirectedAudioMix::check_source`] whether
//! the file can serve, and then draws every sample from [`SourceCursor::at`].
//!
//! ## Time
//!
//! * Output sample `n` (at [`MIX_RATE`]) belongs to scene instant `from/scene_rate + n/48000`, and plays
//!   the file at `window_start + that instant`. The mapping is one rational per sample; a source at
//!   another rate (`44100`, `32000`, ...) is read at the fractional frame `source_time × rate`, so a
//!   window that starts between two source frames is interpolated, not rounded.
//! * The stream is exactly as long as the video: `round(frames / output_rate × 48000)` samples. When
//!   the preset changes the frame rate, the video can be up to half an output frame longer than the
//!   edit range; that tail is **declared silence** ([`DirectedAudioMix::silence_tail_samples`]), never
//!   sound read past the window.
//!
//! ## Refused, never approximated
//!
//! * the exported range must lie inside the declared window (`audio_window_short`): a half-mixed file
//!   would pass for a finished one. The comparison is made on the output's own 48 kHz grid, in
//!   integers: a bundle that wrote `0.3333333333333333` for `8/24` s is the whole of those 8 frames,
//!   and a window one sample shorter is still refused (BL016.6);
//! * the file must hold every frame of the **declared window**, not only the part this range reads
//!   (`audio_source_short`): the origin refuses a source shorter than `editEndSecondsExclusive` before
//!   rendering any range of it (BL016.7);
//! * the stream must fit a PCM16 WAV and the project export's 6 hours (`audio_too_long`), refused
//!   here, before a byte is written (BL016.10);
//! * only PCM16 / float32 WAV with 1 or 2 channels at 8–48 kHz (`unsupported`): up-conversion is
//!   linear, and there is no low-pass for a down-conversion, so a source above 48 kHz is refused
//!   instead of aliased.
//!
//! Narration, takes and per-line gains are not part of this slice and stay `unsupported`.

use serde_json::{Value, json};

use crate::audio::WavFormat;
use crate::directed_scene::AudioMix;
use crate::plan::MIX_SAMPLE_RATE;
use crate::state::CommandError;
use crate::time::{Rate, Ratio};

use super::schedule::SceneExportSchedule;

/// Rate of the exported stream: the project export's rate, so both exports share one mix contract.
pub const MIX_RATE: u32 = MIX_SAMPLE_RATE;
/// Longest stream one export mixes: the project export's ceiling (`MAX_AUDIO_SECONDS`). Six hours of
/// 48 kHz stereo PCM16 is 4.147 GB, under the 4 GiB a RIFF `data` size can name.
pub const MAX_MIX_SECONDS: u64 = 6 * 3600;
/// Slowest source accepted.
pub const MIN_SOURCE_RATE: u32 = 8_000;
/// Fastest source accepted: faster needs a low-pass this slice does not have.
pub const MAX_SOURCE_RATE: u32 = MIX_RATE;

/// What the media adapter learned from the header of the mix file. Facts only: the rules are here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceFacts {
    pub format: WavFormat,
    pub sample_rate: u32,
    pub channels: u16,
    /// Sample frames in the data chunk.
    pub frames: u64,
}

/// The frames of the source one export reads: `first .. first + frames`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceSpan {
    pub first: u64,
    pub frames: u64,
}

/// A fractional frame of the source: `base + fraction`. `fraction` is exactly `0.0` when the
/// position falls on a source frame, which is what makes a 48 kHz source copy sample for sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourcePosition {
    pub base: u64,
    pub fraction: f64,
}

/// The mix of one export, frozen with the snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectedAudioMix {
    relative_path: String,
    pcm_hash: String,
    window_start: Exact,
    window_end: Exact,
    /// Where in the file output sample 0 sits, in seconds.
    origin: Exact,
    output_samples: u64,
    mix_samples: u64,
}

fn invalid(code: &'static str, message: impl Into<String>) -> CommandError {
    CommandError::new(code, message)
}

/// Digits of a bundle's seconds that are kept: `1e-12` s, a picosecond. The error of rounding there is at
/// most `5e-13` s, about `2.4e-8` of one 48 kHz sample, and it is what keeps every denominator of
/// the arithmetic below small (`10^12`), whatever `f64` the bundle wrote.
const DECIMAL_PLACES: usize = 12;
/// A declared time is below this many seconds. The largest source a WAV can hold is 4 GiB, which is
/// 268 435 s of 8 kHz mono PCM16, so no bundle's window can usefully reach further; the cap only keeps
/// the integer arithmetic far from `i128`'s end.
const MAX_DECLARED_SECONDS: i128 = 1_000_000_000;

fn gcd(a: i128, b: i128) -> i128 {
    let (mut a, mut b) = (a.unsigned_abs(), b.unsigned_abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    i128::try_from(a).unwrap_or(1)
}

/// An exact fraction of seconds in `i128`, reduced, with a positive denominator. [`Ratio`] stops at
/// `i64`, which a window start such as `1/24` plus a few minutes already leaves; here every
/// operation is checked, and a result that does not fit is `None` (`audio_window_invalid`), never a
/// wrapped number.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Exact {
    num: i128,
    den: i128,
}

impl std::fmt::Debug for Exact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.num, self.den)
    }
}

impl Exact {
    #[cfg(test)]
    const ZERO: Self = Self { num: 0, den: 1 };

    fn new(num: i128, den: i128) -> Option<Self> {
        if den == 0 {
            return None;
        }
        let divisor = gcd(num, den).max(1);
        let sign = if den < 0 { -1 } else { 1 };
        Some(Self {
            num: (num / divisor).checked_mul(sign)?,
            den: (den / divisor).checked_mul(sign)?,
        })
    }

    #[cfg(test)]
    fn from_ratio(value: Ratio) -> Self {
        Self {
            num: i128::from(value.num()),
            den: i128::from(value.den()),
        }
    }

    /// The instant of `frame` at `rate`: `frame × den / num`.
    fn frame_time(rate: Rate, frame: i64) -> Option<Self> {
        Self::new(
            i128::from(frame).checked_mul(i128::from(rate.den()))?,
            i128::from(rate.num()),
        )
    }

    fn add(self, other: Self) -> Option<Self> {
        // Over the least common denominator, so it does not grow faster than it must.
        let divisor = gcd(self.den, other.den);
        let (mine, theirs) = (self.den / divisor, other.den / divisor);
        Self::new(
            self.num
                .checked_mul(theirs)?
                .checked_add(other.num.checked_mul(mine)?)?,
            self.den.checked_mul(theirs)?,
        )
    }

    fn sub(self, other: Self) -> Option<Self> {
        self.add(Self {
            num: other.num.checked_neg()?,
            den: other.den,
        })
    }

    fn mul_int(self, value: i128) -> Option<Self> {
        Self::new(self.num.checked_mul(value)?, self.den)
    }

    /// Nearest integer, ties away from zero.
    fn round(self) -> Option<i128> {
        let sign = if self.num < 0 { -1 } else { 1 };
        let twice_den = self.den.checked_mul(2)?;
        Some(
            self.num
                .checked_mul(2)?
                .checked_add(self.den.checked_mul(sign)?)?
                / twice_den,
        )
    }

    fn to_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// The same shape as [`Ratio::to_json`]; a numerator or denominator beyond `i64` is a string.
    fn to_json(self) -> Value {
        let integer = |value: i128| {
            i64::try_from(value).map_or_else(|_| json!(value.to_string()), |value| json!(value))
        };
        json!({ "num": integer(self.num), "den": integer(self.den), "seconds": self.to_f64() })
    }

    /// `None` when the reduced fraction is beyond `i64`.
    fn ratio(self) -> Option<Ratio> {
        Ratio::new(i64::try_from(self.num).ok()?, i64::try_from(self.den).ok()?)
    }
}

/// The decimal a bundle author wrote (`1.5`, `0.04`), as an exact fraction. `f64`'s shortest
/// round-trip text is that decimal, so `0.1` is `1/10`, not the binary neighbour of one tenth.
/// Text beyond [`DECIMAL_PLACES`] is rounded there, half away from zero, on the digits themselves
/// (no float arithmetic): `0.041666666666666664` is `41666666667/10^12`, within `5e-13` s of `1/24`.
/// `None` when the value is not finite or not below [`MAX_DECLARED_SECONDS`].
fn decimal(value: f64) -> Option<Exact> {
    if !value.is_finite() {
        return None;
    }
    let text = format!("{value}");
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.as_str()),
    };
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    let whole: i128 = whole.parse().ok()?;
    if whole >= MAX_DECLARED_SECONDS {
        return None;
    }
    let kept = &fraction[..fraction.len().min(DECIMAL_PLACES)];
    let one = 10i128.pow(u32::try_from(DECIMAL_PLACES).ok()?);
    let mut scaled = whole * one + format!("{kept:0<DECIMAL_PLACES$}").parse::<i128>().ok()?;
    if fraction
        .as_bytes()
        .get(DECIMAL_PLACES)
        .is_some_and(|digit| *digit >= b'5')
    {
        scaled += 1;
    }
    Exact::new(if negative { -scaled } else { scaled }, one)
}

/// `time` seconds on the 48 kHz grid: the nearest sample boundary, ties away from zero. This is the
/// only place a window and a range are compared: never as seconds in `f64`, which is how
/// `0.3333333333333333 < 8/24` happens.
fn grid(time: Exact) -> Option<i128> {
    time.mul_int(i128::from(MIX_RATE))?.round()
}

fn seconds(value: f64, field: &str) -> Result<Exact, CommandError> {
    decimal(value).ok_or_else(|| {
        invalid(
            "audio_window_invalid",
            format!("`audio.{field}` ({value}) is not a time this export can place exactly"),
        )
    })
}

impl DirectedAudioMix {
    /// Places the declared mix against an export. Typed refusals: `audio_window_invalid` (a time that
    /// does not fit exact arithmetic), `audio_window_short` (the range leaves the declared window).
    pub fn plan(audio: &AudioMix, schedule: &SceneExportSchedule) -> Result<Self, CommandError> {
        let window_start = seconds(audio.edit_start_seconds, "editStartSeconds")?;
        let window_end = seconds(audio.edit_end_seconds_exclusive, "editEndSecondsExclusive")?;
        let overflow = || {
            invalid(
                "audio_window_invalid",
                "The audio window overflows exact time",
            )
        };
        let from_time =
            Exact::frame_time(schedule.scene_rate, schedule.from).ok_or_else(overflow)?;
        let range_end =
            Exact::frame_time(schedule.scene_rate, schedule.to()).ok_or_else(overflow)?;
        let window_length = window_end.sub(window_start).ok_or_else(overflow)?;
        let (window_samples, range_end_samples) = (
            grid(window_length).ok_or_else(overflow)?,
            grid(range_end).ok_or_else(overflow)?,
        );
        if range_end_samples > window_samples {
            return Err(invalid(
                "audio_window_short",
                format!(
                    "The audio window covers {window_samples} samples ({:.6} s) of edit time but the \
                     range reaches {range_end_samples} samples ({:.6} s): export a shorter range or \
                     pass `audio: false`",
                    window_length.to_f64(),
                    range_end.to_f64()
                ),
            )
            .details(json!({
                "window_start": window_start.to_json(), "window_end": window_end.to_json(),
                "range_end": range_end.to_json(), "hint": "audio: false",
                "window_samples": window_samples.to_string(),
                "range_end_samples": range_end_samples.to_string(),
            })));
        }
        let origin = window_start.add(from_time).ok_or_else(overflow)?;
        let samples = |time: Exact| {
            time.mul_int(i128::from(MIX_RATE))
                .and_then(Exact::round)
                .and_then(|samples| u64::try_from(samples).ok())
                .ok_or_else(overflow)
        };
        let output_samples = samples(
            Exact::new(
                i128::from(schedule.frames)
                    .checked_mul(i128::from(schedule.output_rate.den()))
                    .ok_or_else(overflow)?,
                i128::from(schedule.output_rate.num()),
            )
            .ok_or_else(overflow)?,
        )?
        .max(1);
        if output_samples > MAX_MIX_SECONDS * u64::from(MIX_RATE) {
            return Err(invalid(
                "audio_too_long",
                format!(
                    "The audio of one export is limited to {} hours; this range mixes {} samples \
                     ({:.2} h at {MIX_RATE} Hz): export a shorter range or pass `audio: false`",
                    MAX_MIX_SECONDS / 3600,
                    output_samples,
                    output_samples as f64 / f64::from(MIX_RATE) / 3600.0
                ),
            )
            .details(json!({
                "samples": output_samples, "max_samples": MAX_MIX_SECONDS * u64::from(MIX_RATE),
                "max_seconds": MAX_MIX_SECONDS, "hint": "audio: false",
            })));
        }
        let range_samples = samples(range_end.sub(from_time).ok_or_else(overflow)?)?.max(1);
        Ok(Self {
            relative_path: audio.mix_relative_path.clone(),
            pcm_hash: audio.source_pcm_hash_sha256.clone().unwrap_or_default(),
            window_start,
            window_end,
            origin,
            output_samples,
            mix_samples: output_samples.min(range_samples),
        })
    }

    pub fn relative_path(&self) -> &str {
        &self.relative_path
    }

    /// The SHA-256 the bundle pins the file to.
    pub fn pcm_hash(&self) -> &str {
        &self.pcm_hash
    }

    /// Samples of the exported stream (stereo frames at [`MIX_RATE`]): equal to the video's length.
    pub fn output_samples(&self) -> u64 {
        self.output_samples
    }

    /// Samples read from the file; the rest of the stream is [`silence_tail_samples`](Self::silence_tail_samples).
    pub fn mix_samples(&self) -> u64 {
        self.mix_samples
    }

    pub fn silence_tail_samples(&self) -> u64 {
        self.output_samples - self.mix_samples
    }

    /// File time of output sample 0, seconds. The window start is a decimal rounded to a picosecond,
    /// so this is the exact sum of that and the range's first instant; `None` only when the reduced
    /// fraction does not fit a [`Ratio`] (the cursor and the JSON report it in full regardless).
    pub fn origin(&self) -> Option<Ratio> {
        self.origin.ratio()
    }

    /// Where output sample `n` reads a source at `rate` Hz. Exact: one division of integers.
    pub fn cursor(&self, rate: u32) -> Result<SourceCursor, CommandError> {
        SourceCursor::new(self.origin, rate, self.output_samples)
    }

    /// Whether a file with these facts can serve this export, and which frames it must provide.
    pub fn check_source(&self, facts: &SourceFacts) -> Result<SourceSpan, CommandError> {
        if !(1..=2).contains(&facts.channels) {
            return Err(unsupported(
                format!(
                    "The mix has {} channels: only mono and stereo",
                    facts.channels
                ),
                facts,
            ));
        }
        if !(MIN_SOURCE_RATE..=MAX_SOURCE_RATE).contains(&facts.sample_rate) {
            return Err(unsupported(
                format!(
                    "The mix is {} Hz: only {MIN_SOURCE_RATE}-{MAX_SOURCE_RATE} Hz are converted",
                    facts.sample_rate
                ),
                facts,
            ));
        }
        // The origin's rule (`node/render.ts`, `AUDIO_SHORT`): the source must cover the whole
        // declared window, `duration + 1 ms >= editEndSecondsExclusive`, whatever part of it one
        // export reads. Exact in integers: `frames/rate + 1/1000 < end` is
        // `frames*1000*d + rate*d < n*rate*1000` for `end = n/d`. A window end is a decimal below
        // 1e9 s at 12 places, so `n < 1e21` and `d <= 10^12`: every product is far inside `i128`.
        let (end_num, end_den) = (self.window_end.num, self.window_end.den);
        let rate = i128::from(facts.sample_rate);
        if i128::from(facts.frames) * 1000 * end_den + rate * end_den < end_num * rate * 1000 {
            return Err(invalid(
                "audio_source_short",
                format!(
                    "The mix file holds {} frames ({:.3} s at {} Hz) but the declared window ends at \
                     {:.3} s: the source must cover the whole window, not only the exported range",
                    facts.frames,
                    facts.frames as f64 / f64::from(facts.sample_rate),
                    facts.sample_rate,
                    self.window_end.to_f64()
                ),
            )
            .details(json!({
                "file_frames": facts.frames, "sample_rate": facts.sample_rate,
                "window_end": self.window_end.to_json(), "rule": "window",
            })));
        }
        let cursor = self.cursor(facts.sample_rate)?;
        let first = cursor.at(0).base;
        let last = cursor.at(self.mix_samples - 1).base;
        let frames = last - first + 1;
        if last >= facts.frames {
            return Err(invalid(
                "audio_source_short",
                format!(
                    "The mix file holds {} frames but this range reads up to frame {last} \
                     (window {:.3}-{:.3} s at {} Hz)",
                    facts.frames,
                    self.window_start.to_f64(),
                    self.window_end.to_f64(),
                    facts.sample_rate
                ),
            )
            .details(json!({
                "file_frames": facts.frames, "needed_through": last, "sample_rate": facts.sample_rate,
                "rule": "range",
            })));
        }
        Ok(SourceSpan { first, frames })
    }

    pub fn to_json(&self) -> Value {
        json!({
            "path": self.relative_path,
            "sha256": self.pcm_hash,
            "window": { "start": self.window_start.to_json(), "end": self.window_end.to_json() },
            "source_start": self.origin.to_json(),
            "sample_rate": MIX_RATE,
            "channels": 2,
            "samples": self.output_samples,
            "mixed_samples": self.mix_samples,
            "silence_tail_samples": self.silence_tail_samples(),
        })
    }
}

fn unsupported(message: String, facts: &SourceFacts) -> CommandError {
    CommandError::new("unsupported", message).details(json!({
        "sample_rate": facts.sample_rate, "channels": facts.channels,
        "accepts": "PCM16 or float32 WAV, mono or stereo, 8000-48000 Hz",
    }))
}

/// Integer arithmetic for `position(n) = origin × rate + n × rate / 48000` in source frames.
#[derive(Debug, Clone, Copy)]
pub struct SourceCursor {
    /// `position(n) = (start + n × step) / scale`.
    start: i128,
    step: i128,
    scale: i128,
}

impl SourceCursor {
    /// `samples` is how many output samples will be asked for: the position of the last one is
    /// computed here, checked, so [`at`](Self::at) can add without a check.
    fn new(origin: Exact, rate: u32, samples: u64) -> Result<Self, CommandError> {
        let overflow = || invalid("audio_window_invalid", "The audio position overflows");
        let scale = origin
            .den
            .checked_mul(i128::from(MIX_RATE))
            .ok_or_else(overflow)?;
        let start = origin
            .num
            .checked_mul(i128::from(MIX_RATE))
            .and_then(|value| value.checked_mul(i128::from(rate)))
            .ok_or_else(overflow)?;
        let step = i128::from(rate)
            .checked_mul(origin.den)
            .ok_or_else(overflow)?;
        step.checked_mul(i128::from(samples))
            .and_then(|span| span.checked_add(start))
            .ok_or_else(overflow)?;
        Ok(Self { start, step, scale })
    }

    pub fn at(&self, sample: u64) -> SourcePosition {
        let numerator = self.start + i128::from(sample) * self.step;
        SourcePosition {
            base: u64::try_from(numerator.div_euclid(self.scale)).unwrap_or(0),
            fraction: if numerator.rem_euclid(self.scale) == 0 {
                0.0
            } else {
                numerator.rem_euclid(self.scale) as f64 / self.scale as f64
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_decimal_is_the_number_the_author_wrote() {
        let exact = |num: i128, den: i128| Exact::new(num, den);
        assert_eq!(decimal(0.1), exact(1, 10));
        assert_eq!(decimal(4.0), exact(4, 1));
        assert_eq!(decimal(1.5), exact(3, 2));
        assert_eq!(decimal(0.04), exact(1, 25));
        assert_eq!(decimal(-2.5), exact(-5, 2));
        assert_eq!(decimal(f64::NAN), None);
        assert_eq!(decimal(f64::INFINITY), None);
        assert_eq!(decimal(1e30), None);
    }

    /// The honest precision: twelve places, rounded on the digits, half away from zero. A picosecond
    /// is 2.4e-8 of a sample, so the rounding cannot move a sample; it is what bounds the arithmetic.
    #[test]
    fn a_decimal_keeps_twelve_places_and_rounds_the_rest_on_its_digits() {
        let exact = |num: i128| Exact::new(num, 1_000_000_000_000);
        // Below the twelfth place there is nothing left: 1e-25 s is zero, not "not a time".
        assert_eq!(decimal(1e-25), Some(Exact::ZERO));
        assert_eq!(decimal(4e-13), Some(Exact::ZERO));
        // The thirteenth digit decides: 5 and up rounds away from zero, 4 and down does not.
        assert_eq!(decimal(5e-13), exact(1));
        assert_eq!(decimal(-5e-13), exact(-1));
        assert_eq!(decimal(0.0000000000014), exact(1));
        assert_eq!(decimal(0.0000000000016), exact(2));
        // The shortest text of `1/24` and `1/960` as an `f64`, 17-19 digits long.
        assert_eq!(decimal(1.0 / 24.0), exact(41_666_666_667));
        assert_eq!(decimal(1.0 / 960.0), exact(1_041_666_667));
        assert_eq!(decimal(1.0 / 3.0), exact(333_333_333_333));
        assert_eq!(decimal(-1.0 / 3.0), exact(-333_333_333_333));
        // Each is within 5e-13 s of the instant the author meant (a millionth of a sample).
        for (value, num, den) in [(1.0 / 24.0, 1.0, 24.0), (1.0 / 960.0, 1.0, 960.0)] {
            let kept = decimal(value).unwrap().to_f64();
            assert!((kept - num / den).abs() <= 5e-13, "{value}: {kept}");
        }
        // The cap: below it a bundle's time is a time; at it, it is not (no WAV reaches that far).
        assert!(decimal(999_999_999.5).is_some());
        assert_eq!(decimal(1_000_000_000.0), None);
        assert_eq!(decimal(-1_000_000_000.0), None);
    }

    /// The `i128` fraction is checked everywhere: what it cannot hold is `None`, never a wrapped value.
    #[test]
    fn exact_arithmetic_reports_overflow_instead_of_wrapping() {
        let big = Exact::new(i128::MAX / 2 + 1, 1).unwrap();
        assert_eq!(big.add(big), None);
        assert_eq!(big.mul_int(2), None);
        assert_eq!(Exact::new(1, 0), None);
        assert_eq!(Exact::new(i128::MIN, -1), None);
        let third = Exact::new(1, 3).unwrap();
        assert_eq!(third.add(third).unwrap().add(third), Exact::new(1, 1));
        assert_eq!(Exact::new(-5, 2).unwrap().round(), Some(-3));
        assert_eq!(Exact::new(5, 2).unwrap().round(), Some(3));
        assert_eq!(Exact::new(7, 3).unwrap().round(), Some(2));
    }

    fn mix(origin: Ratio, samples: u64) -> DirectedAudioMix {
        mix_ending(origin, samples, Ratio::from_int(10))
    }

    fn mix_ending(origin: Ratio, samples: u64, window_end: Ratio) -> DirectedAudioMix {
        DirectedAudioMix {
            relative_path: "a.wav".into(),
            pcm_hash: String::new(),
            window_start: Exact::ZERO,
            window_end: Exact::from_ratio(window_end),
            origin: Exact::from_ratio(origin),
            output_samples: samples,
            mix_samples: samples,
        }
    }

    #[test]
    fn a_48k_source_copies_sample_for_sample_from_the_window_start() {
        let mix = mix(Ratio::new(3, 2).unwrap(), 100);
        let cursor = mix.cursor(48_000).unwrap();
        // 1.5 s is exactly frame 72 000.
        assert_eq!(
            cursor.at(0),
            SourcePosition {
                base: 72_000,
                fraction: 0.0
            }
        );
        assert_eq!(
            cursor.at(99),
            SourcePosition {
                base: 72_099,
                fraction: 0.0
            }
        );
    }

    #[test]
    fn a_44k_source_is_read_at_fractional_frames_in_exact_time() {
        let mix = mix(Ratio::ZERO, 48_000);
        let cursor = mix.cursor(44_100).unwrap();
        assert_eq!(cursor.at(0).base, 0);
        // One second of output is exactly 44 100 source frames, with no drift.
        assert_eq!(
            cursor.at(48_000),
            SourcePosition {
                base: 44_100,
                fraction: 0.0
            }
        );
        // Sample 1 sits at 0.91875 of a source frame.
        let one = cursor.at(1);
        assert_eq!(one.base, 0);
        assert!((one.fraction - 0.91875).abs() < 1e-12, "{one:?}");
    }

    #[test]
    fn a_source_must_cover_the_range_and_have_a_supported_shape() {
        // Window [0, 2): the file is the 2 s the window names, and this export reads its second half.
        let mix = mix_ending(Ratio::from_int(1), 48_000, Ratio::from_int(2));
        let facts = SourceFacts {
            format: WavFormat::Pcm16,
            sample_rate: 48_000,
            channels: 1,
            frames: 96_000,
        };
        assert_eq!(
            mix.check_source(&facts).unwrap(),
            SourceSpan {
                first: 48_000,
                frames: 48_000
            }
        );
        // One frame short: inside the origin's 1 ms tolerance for the window, but the range reads
        // that frame, so the samples are not invented.
        let short = SourceFacts {
            frames: 95_999,
            ..facts
        };
        let refused = mix.check_source(&short).unwrap_err();
        assert_eq!(refused.code, "audio_source_short");
        assert_eq!(refused.details.as_ref().unwrap()["rule"], "range");
        for bad in [
            SourceFacts {
                channels: 6,
                ..facts
            },
            SourceFacts {
                channels: 0,
                ..facts
            },
            SourceFacts {
                sample_rate: 96_000,
                ..facts
            },
            SourceFacts {
                sample_rate: 4_000,
                ..facts
            },
        ] {
            assert_eq!(
                mix.check_source(&bad).unwrap_err().code,
                "unsupported",
                "{bad:?}"
            );
        }
    }

    /// BL016.7: the origin refuses a source shorter than `editEndSecondsExclusive` whatever range is
    /// rendered (`duration + 0.001 < editEnd`); this export reads 100 samples at the start of a 4 s
    /// window, so only the window rule can refuse the short files below.
    #[test]
    fn a_source_shorter_than_the_window_is_refused_even_when_the_range_fits() {
        let mix = mix_ending(Ratio::ZERO, 100, Ratio::from_int(4));
        let file = |frames: u64, sample_rate: u32| SourceFacts {
            format: WavFormat::Pcm16,
            sample_rate,
            channels: 2,
            frames,
        };
        // The whole window, and exactly 1 ms less (the origin's tolerance: `3.999 + 0.001 < 4` is false).
        assert!(mix.check_source(&file(192_000, 48_000)).is_ok());
        assert!(mix.check_source(&file(192_000 - 48, 48_000)).is_ok());
        // One frame beyond the tolerance is short.
        let refused = mix.check_source(&file(192_000 - 49, 48_000)).unwrap_err();
        assert_eq!(refused.code, "audio_source_short");
        assert_eq!(refused.details.as_ref().unwrap()["rule"], "window");
        // A 2 s file for a [0, 4) window, the case the limited export used to accept.
        let refused = mix.check_source(&file(96_000, 48_000)).unwrap_err();
        assert_eq!(refused.code, "audio_source_short");
        // Same decision at another rate: 44.1 kHz, the tolerance is 44.1 frames (floor 44 passes).
        assert!(mix.check_source(&file(176_400, 44_100)).is_ok());
        assert!(mix.check_source(&file(176_400 - 44, 44_100)).is_ok());
        assert!(mix.check_source(&file(176_400 - 45, 44_100)).is_err());
        // A window that is not a whole second is compared exactly too (0.1 = 1/10).
        let tenth = mix_ending(Ratio::ZERO, 100, Ratio::new(1, 10).unwrap());
        assert!(tenth.check_source(&file(4_800, 48_000)).is_ok());
        assert!(tenth.check_source(&file(4_800 - 48, 48_000)).is_ok());
        assert!(tenth.check_source(&file(4_800 - 49, 48_000)).is_err());
    }

    fn schedule(frames: u64) -> SceneExportSchedule {
        use crate::preset::{Fit, Geometry};
        use crate::time::Rate;
        let rate = Rate::new(24, 1).unwrap();
        SceneExportSchedule {
            scene_rate: rate,
            output_rate: rate,
            from: 0,
            edit_frames: frames,
            frames,
            geometry: Geometry {
                render: (320, 180),
                output: (320, 180),
                fit: Fit::Contain,
            },
        }
    }

    fn declared(start: f64, end: f64) -> AudioMix {
        AudioMix {
            mix_relative_path: "a.wav".into(),
            edit_start_seconds: start,
            edit_end_seconds_exclusive: end,
            source_pcm_hash_sha256: None,
        }
    }

    fn schedule_of(rate: crate::time::Rate, from: i64, frames: u64) -> SceneExportSchedule {
        SceneExportSchedule {
            scene_rate: rate,
            output_rate: rate,
            from,
            edit_frames: frames,
            ..schedule(frames)
        }
    }

    /// The seconds a bundle writes for an instant that is not a decimal: the shortest `f64` text.
    fn written(numerator: u32, denominator: u32) -> f64 {
        f64::from(numerator) / f64::from(denominator)
    }

    /// BL016.6, second half: a window that *starts* at `1/24`, `1/3` or `1/960` s is as valid as one
    /// that starts at zero. The shortest `f64` text of these has 16-19 digits, which used to overflow
    /// the `i64` fraction (`1/24` from 74 s, `1/960` at any length) before the 48 kHz grid was reached.
    #[test]
    fn a_window_that_starts_on_a_non_decimal_instant_plans_whatever_its_length() {
        use crate::time::Rate;
        let rate = Rate::new(24, 1).unwrap();
        // (window start, the seconds of window the range must fit in)
        for (start, seconds) in [
            (written(1, 24), 120u64),
            (written(1, 24), 600),
            (written(1, 12), 93),
            (written(1, 6), 185),
            (1.0 + written(1, 24), 924),
            (written(1, 3), 1_000),
            (written(1, 960), 1),
            (written(1, 960), 120),
            (written(1, 960), 6 * 3600),
        ] {
            let frames = seconds * 24;
            let end = start + seconds as f64;
            DirectedAudioMix::plan(&declared(start, end), &schedule_of(rate, 0, frames))
                .unwrap_or_else(|error| panic!("start {start}, {seconds} s: {error:?}"));
            // One sample (1/48000 s) less is still refused, whatever the start.
            let shorter = end - 1.5 / 48_000.0;
            let refused =
                DirectedAudioMix::plan(&declared(start, shorter), &schedule_of(rate, 0, frames))
                    .unwrap_err();
            assert_eq!(
                refused.code, "audio_window_short",
                "start {start}, {seconds} s"
            );
        }
    }

    /// The same defect through `window_start + from_time`: `1/3` plus the instant of frame 7372 at
    /// 24 fps (307 s) overflowed, even though the window fit.
    #[test]
    fn a_non_decimal_start_and_a_late_range_place_the_first_sample_exactly() {
        use crate::time::Rate;
        let rate = Rate::new(24, 1).unwrap();
        let start = written(1, 3);
        for from in [7_371i64, 7_372, 8_000, 24 * 3_600] {
            let planned = DirectedAudioMix::plan(
                &declared(start, start + 40_000.0),
                &schedule_of(rate, from, 24),
            )
            .unwrap_or_else(|error| panic!("from {from}: {error:?}"));
            // The first sample reads the file at `start + from/24`, to far less than a sample.
            let exact_frames = (1.0 / 3.0 + from as f64 / 24.0) * 48_000.0;
            let cursor = planned.cursor(48_000).unwrap();
            let position = cursor.at(0);
            let got = position.base as f64 + position.fraction;
            assert!(
                (got - exact_frames).abs() < 1e-3,
                "from {from}: {got} vs {exact_frames}"
            );
        }
        // 30000/1001, a rate whose frame times are not decimal either, and a start that is not.
        let ntsc = Rate::new(30_000, 1_001).unwrap();
        DirectedAudioMix::plan(
            &declared(written(1, 960), 5_000.0),
            &schedule_of(ntsc, 100_000, 300),
        )
        .unwrap();
        // 29.97 fps written as `2997/100`, a start of `1/3` and a range 10 000 s in: the sum's reduced
        // denominator is ~3e15 and its numerator ~3e19, which an `i64` fraction cannot hold even
        // though the window is quantised to twelve places.
        let odd = Rate::new(2_997, 100).unwrap();
        let planned = DirectedAudioMix::plan(
            &declared(start, start + 20_000.0),
            &schedule_of(odd, 300_000, 30),
        )
        .unwrap();
        let position = planned.cursor(48_000).unwrap().at(0);
        let exact = (1.0 / 3.0 + 300_000.0 * 100.0 / 2_997.0) * 48_000.0;
        let got = position.base as f64 + position.fraction;
        assert!((got - exact).abs() < 1e-2, "{got} vs {exact}");
        assert_eq!(
            planned.origin(),
            None,
            "too wide for a `Ratio`, still exact in the cursor"
        );
        // The refusal the arithmetic cannot avoid is still typed: a start beyond any file.
        let refused = DirectedAudioMix::plan(&declared(1e30, 2e30), &schedule(24)).unwrap_err();
        assert_eq!(refused.code, "audio_window_invalid");
    }

    /// BL016.10: six hours is the ceiling, refused when the export is planned (nothing written, no
    /// 4 GiB generated here: the schedule is a value, not a render).
    #[test]
    fn six_hours_of_audio_plan_and_one_frame_more_is_refused_before_any_write() {
        let six_hours_of_frames = 6 * 3600 * 24;
        let window = declared(0.0, 30_000.0);
        let planned =
            DirectedAudioMix::plan(&window, &schedule(six_hours_of_frames)).expect("6 h fits");
        assert_eq!(planned.output_samples(), 6 * 3600 * 48_000);
        // The WAV `data` size and `36 + data` still fit a u32 at the ceiling.
        assert!(planned.output_samples() * 4 + 36 <= u64::from(u32::MAX));
        let refused =
            DirectedAudioMix::plan(&window, &schedule(six_hours_of_frames + 1)).unwrap_err();
        assert_eq!(refused.code, "audio_too_long");
        let details = refused.details.as_ref().unwrap();
        assert_eq!(details["max_seconds"], 6 * 3600);
        assert_eq!(details["samples"], 6 * 3600 * 48_000 + 2_000);
    }

    /// BL016.6: `8/24` s is `1/3`, which no decimal writes; the author's `0.3333333333333333` is the
    /// same 16 000 samples on the 48 kHz grid, and a window one sample shorter is not.
    #[test]
    fn a_window_is_compared_on_the_48k_grid_not_as_float_seconds() {
        let eight_frames = schedule(8);
        // The decimal the author wrote is below the exact third by 3.3e-17 s: a comparison in
        // seconds refuses the whole range, and the grid does not.
        let written = decimal(0.3333333333333333).unwrap();
        assert!(
            written.num * 3 < written.den,
            "{written:?} is below the exact third"
        );
        DirectedAudioMix::plan(&declared(0.0, 0.3333333333333333), &eight_frames).unwrap();
        // One sample (1/48000 s) less than the 16 000 the range needs.
        let refused =
            DirectedAudioMix::plan(&declared(0.0, 15_999.0 / 48_000.0), &eight_frames).unwrap_err();
        assert_eq!(refused.code, "audio_window_short");
        assert_eq!(refused.details.as_ref().unwrap()["window_samples"], "15999");
        assert_eq!(
            refused.details.as_ref().unwrap()["range_end_samples"],
            "16000"
        );
        // A start offset moves the window, not the range: [1, 1 + 1/3) is a window of one third.
        DirectedAudioMix::plan(&declared(1.0, 1.0 + 8.0 / 24.0), &eight_frames).unwrap();
        // Every whole-frame duration of a 24 fps scene, written the way a float would write it.
        for frames in 1..=2_000u64 {
            let seconds = frames as f64 / 24.0;
            DirectedAudioMix::plan(&declared(0.0, seconds), &schedule(frames))
                .unwrap_or_else(|error| panic!("{frames} frames, {seconds}: {error:?}"));
            let shorter = (frames * 2_000 - 1) as f64 / 48_000.0;
            assert_eq!(
                DirectedAudioMix::plan(&declared(0.0, shorter), &schedule(frames))
                    .unwrap_err()
                    .code,
                "audio_window_short",
                "{frames} frames, {shorter}"
            );
        }
    }
}
