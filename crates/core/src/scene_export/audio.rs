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
//!   would pass for a finished one;
//! * the file must hold every frame the range reads (`audio_source_short`);
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
        if range_end > window_length {
            return Err(invalid(
                "audio_window_short",
                format!(
                    "The audio window covers {:.3} s of edit time but the range reaches {:.3} s: \
                     export a shorter range or pass `audio: false`",
                    window_length.to_f64(),
                    range_end.to_f64()
                ),
            )
            .details(json!({
                "window_start": window_start.to_json(), "window_end": window_end.to_json(),
                "range_end": range_end.to_json(), "hint": "audio: false",
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
        DirectedAudioMix {
            relative_path: "a.wav".into(),
            pcm_hash: String::new(),
            window_start: Ratio::ZERO,
            window_end: Ratio::from_int(10),
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
        let mix = mix(Ratio::from_int(1), 48_000);
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
        let short = SourceFacts {
            frames: 95_999,
            ..facts
        };
        assert_eq!(
            mix.check_source(&short).unwrap_err().code,
            "audio_source_short"
        );
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
}
