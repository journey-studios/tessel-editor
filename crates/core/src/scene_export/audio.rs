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
use crate::time::Ratio;

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
    window_start: Ratio,
    window_end: Ratio,
    /// Where in the file output sample 0 sits, in seconds.
    origin: Ratio,
    output_samples: u64,
    mix_samples: u64,
}

fn invalid(code: &'static str, message: impl Into<String>) -> CommandError {
    CommandError::new(code, message)
}

/// The decimal a bundle author wrote (`1.5`, `0.04`), as an exact fraction. `f64`'s shortest
/// round-trip text is that decimal, so `0.1` is `1/10`, not the binary neighbour of one tenth.
/// `None` when it does not fit (a window of `1e30` seconds is not a window).
fn decimal(value: f64) -> Option<Ratio> {
    if !value.is_finite() {
        return None;
    }
    let text = format!("{value}");
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.as_str()),
    };
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    let scale = i64::try_from(fraction.len()).ok()?;
    let mut numerator: i64 = format!("{whole}{fraction}").parse().ok()?;
    if negative {
        numerator = -numerator;
    }
    let denominator = 10i64.checked_pow(u32::try_from(scale).ok()?)?;
    Ratio::new(numerator, denominator)
}

/// `time` seconds on the 48 kHz grid: the nearest sample boundary, ties away from zero, in `i128`
/// so that no window the bundle can write overflows. This is the only place a window and a range
/// are compared: never as seconds in `f64`, which is how `0.3333333333333333 < 8/24` happens.
fn grid(time: Ratio) -> i128 {
    let numerator = i128::from(time.num()) * i128::from(MIX_RATE);
    let denominator = i128::from(time.den());
    let sign = if numerator < 0 { -1 } else { 1 };
    (numerator * 2 + denominator * sign) / (denominator * 2)
}

fn seconds(value: f64, field: &str) -> Result<Ratio, CommandError> {
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
        let from_time = schedule.scene_rate.frame_to_time(schedule.from);
        let range_end = schedule.scene_rate.frame_to_time(schedule.to());
        let window_length = window_end.sub(window_start).ok_or_else(overflow)?;
        let (window_samples, range_end_samples) = (grid(window_length), grid(range_end));
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
        let samples = |time: Ratio| {
            time.mul_int(i64::from(MIX_RATE))
                .map(Ratio::round)
                .and_then(|samples| u64::try_from(samples).ok())
                .ok_or_else(overflow)
        };
        let output_samples = samples(
            Ratio::new(
                i64::try_from(schedule.frames).map_err(|_| overflow())?
                    * i64::from(schedule.output_rate.den()),
                i64::from(schedule.output_rate.num()),
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

    /// File time of output sample 0, seconds.
    pub fn origin(&self) -> Ratio {
        self.origin
    }

    /// Where output sample `n` reads a source at `rate` Hz. Exact: one division of integers.
    pub fn cursor(&self, rate: u32) -> Result<SourceCursor, CommandError> {
        SourceCursor::new(self.origin, rate)
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
        // `frames*1000*d + rate*d < n*rate*1000` for `end = n/d`.
        let (end_num, end_den) = (
            i128::from(self.window_end.num()),
            i128::from(self.window_end.den()),
        );
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
    fn new(origin: Ratio, rate: u32) -> Result<Self, CommandError> {
        let overflow = || invalid("audio_window_invalid", "The audio position overflows");
        let scale = i128::from(origin.den()) * i128::from(MIX_RATE);
        let start = i128::from(origin.num())
            .checked_mul(i128::from(MIX_RATE))
            .and_then(|value| value.checked_mul(i128::from(rate)))
            .ok_or_else(overflow)?;
        Ok(Self {
            start,
            step: i128::from(rate) * i128::from(origin.den()),
            scale,
        })
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
        assert_eq!(decimal(0.1), Ratio::new(1, 10));
        assert_eq!(decimal(4.0), Ratio::new(4, 1));
        assert_eq!(decimal(1.5), Ratio::new(3, 2));
        assert_eq!(decimal(0.04), Ratio::new(1, 25));
        assert_eq!(decimal(f64::NAN), None);
        assert_eq!(decimal(1e30), None);
        assert_eq!(decimal(1e-25), None);
    }

    fn mix(origin: Ratio, samples: u64) -> DirectedAudioMix {
        mix_ending(origin, samples, Ratio::from_int(10))
    }

    fn mix_ending(origin: Ratio, samples: u64, window_end: Ratio) -> DirectedAudioMix {
        DirectedAudioMix {
            relative_path: "a.wav".into(),
            pcm_hash: String::new(),
            window_start: Ratio::ZERO,
            window_end,
            origin,
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
        assert!(decimal(0.3333333333333333).unwrap() < Ratio::new(1, 3).unwrap());
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
