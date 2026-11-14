//! Offline audio: WAV decoding, the audio plan and the reference mixer.
//!
//! This is *processed audio*: the mixer turns a plan into PCM samples that tests and the
//! CLI can inspect (peak, RMS, exact sample values) and write to a WAV file. It is not
//! *heard audio*: nothing here opens an output device, so it proves neither device
//! output nor audio/video sync during playback (see `docs/features/viewport.md`).

use std::collections::HashMap;
use std::fs;

use serde_json::{Value, json};

use crate::audio_clock::{SourceCursor, source_step};
use crate::backend::BackendError;
use crate::id::Id;
use crate::time::Ratio;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WavFormat {
    Pcm16,
    Float32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavInfo {
    pub format: WavFormat,
    pub sample_rate: u32,
    pub channels: u16,
    /// Sample frames (one per channel group).
    pub frames: u64,
    data_offset: usize,
    data_len: usize,
}

fn le_u16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn le_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// Parses the RIFF/WAVE header of a PCM16 or float32 file with 1..=2 channels.
pub fn wav_info(bytes: &[u8]) -> Result<WavInfo, String> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("Not a RIFF/WAVE file".into());
    }
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = le_u32(bytes, at + 4).ok_or("Truncated chunk header")? as usize;
        let body = at + 8;
        if id == b"fmt " {
            let tag = le_u16(bytes, body).ok_or("Truncated fmt chunk")?;
            let channels = le_u16(bytes, body + 2).ok_or("Truncated fmt chunk")?;
            let rate = le_u32(bytes, body + 4).ok_or("Truncated fmt chunk")?;
            let bits = le_u16(bytes, body + 14).ok_or("Truncated fmt chunk")?;
            fmt = Some((tag, channels, rate, bits));
        } else if id == b"data" {
            let (tag, channels, rate, bits) = fmt.ok_or("`data` chunk before `fmt `")?;
            let format = match (tag, bits) {
                (1, 16) => WavFormat::Pcm16,
                (3, 32) => WavFormat::Float32,
                _ => {
                    return Err(format!(
                        "Unsupported WAV encoding (tag {tag}, {bits} bits): use PCM16 or float32"
                    ));
                }
            };
            if !(1..=2).contains(&channels) || rate == 0 {
                return Err(format!(
                    "Unsupported WAV layout: {channels} channels at {rate} Hz"
                ));
            }
            let len = size.min(bytes.len() - body);
            let frame_bytes = usize::from(channels) * usize::from(bits / 8);
            return Ok(WavInfo {
                format,
                sample_rate: rate,
                channels,
                frames: (len / frame_bytes) as u64,
                data_offset: body,
                data_len: len / frame_bytes * frame_bytes,
            });
        }
        at = body + size + (size & 1);
    }
    Err("WAV file has no `data` chunk".into())
}

/// Interleaved float samples.
#[derive(Debug, Clone, PartialEq)]
pub struct Pcm {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<f32>,
}

impl Pcm {
    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels)
    }

    pub fn peak(&self) -> f32 {
        self.samples
            .iter()
            .fold(0.0, |peak, sample| peak.max(sample.abs()))
    }

    pub fn rms(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let sum: f64 = self.samples.iter().map(|s| f64::from(*s).powi(2)).sum();
        (sum / self.samples.len() as f64).sqrt() as f32
    }

    pub fn sample(&self, frame: usize, channel: usize) -> f32 {
        self.samples
            .get(frame * usize::from(self.channels) + channel)
            .copied()
            .unwrap_or(0.0)
    }
}

pub fn decode_wav(bytes: &[u8]) -> Result<Pcm, String> {
    let info = wav_info(bytes)?;
    let data = &bytes[info.data_offset..info.data_offset + info.data_len];
    let samples = match info.format {
        WavFormat::Pcm16 => data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32768.0)
            .collect(),
        WavFormat::Float32 => data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|quad| f32::from_le_bytes([quad[0], quad[1], quad[2], quad[3]]))
            .collect(),
    };
    Ok(Pcm {
        sample_rate: info.sample_rate,
        channels: info.channels,
        samples,
    })
}

/// Encodes PCM16 (clipping to full scale).
pub fn encode_wav_pcm16(pcm: &Pcm) -> Vec<u8> {
    let data_len = pcm.samples.len() * 2;
    let mut out = Vec::with_capacity(44 + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&pcm.channels.to_le_bytes());
    out.extend_from_slice(&pcm.sample_rate.to_le_bytes());
    out.extend_from_slice(&(pcm.sample_rate * u32::from(pcm.channels) * 2).to_le_bytes());
    out.extend_from_slice(&(pcm.channels * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    for sample in &pcm.samples {
        let value = (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

/// Peak amplitude per bucket of a WAV file, for waveform previews. Values are `0.0..=1.0`.
pub fn waveform_peaks(path: &str, buckets: usize) -> Result<Vec<f32>, String> {
    let bytes = fs::read(path).map_err(|error| format!("Cannot read `{path}`: {error}"))?;
    let pcm = decode_wav(&bytes)?;
    let frames = pcm.frames();
    if frames == 0 || buckets == 0 {
        return Ok(vec![0.0; buckets]);
    }
    Ok((0..buckets)
        .map(|bucket| {
            let from = bucket * frames / buckets;
            let to = ((bucket + 1) * frames / buckets).max(from + 1).min(frames);
            (from..to)
                .flat_map(|frame| {
                    (0..usize::from(pcm.channels)).map(move |channel| (frame, channel))
                })
                .fold(0.0f32, |peak, (frame, channel)| {
                    peak.max(pcm.sample(frame, channel).abs())
                })
        })
        .collect())
}

/// One clip's contribution to the mix, already clipped to the requested interval.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioSegment {
    pub clip: Id,
    pub asset: Id,
    pub path: String,
    /// First output sample frame this segment writes.
    pub output_start: u64,
    /// Output sample frames written.
    pub frames: u64,
    /// Whole part (floor) of the position in the source, in source sample frames, of the first
    /// output frame. The exact position is `source_start + source_phase`; see
    /// [`crate::audio_clock`]: the phase is what keeps a cut clip on the same samples.
    pub source_start: i64,
    /// Fractional part of that position, in source sample frames, in `[0, 1)`.
    pub source_phase: Ratio,
    pub source_rate: u32,
    /// Source frames read per output frame, relative to the rates: `1` plays in time, `2`
    /// twice as fast (varispeed: pitch follows, there is no time-stretch). Exact; the mixer
    /// steps `source_rate * speed / sample_rate` source frames per output frame.
    pub speed: Ratio,
    pub gain: f32,
    /// `-1.0` left .. `1.0` right.
    pub pan: f32,
    /// Fade-in length in output sample frames, from the clip's first sample. Already resolved
    /// against the clip length by [`resolve_fades`]; `0` is no fade.
    pub fade_in: u64,
    /// Fade-out length in output sample frames, up to the clip's last sample.
    pub fade_out: u64,
    /// Output sample frames from the clip's first sample to this segment's first sample. A
    /// segment is only the part of a clip inside the requested interval, so the fades are
    /// placed by the clip, never by the interval: a mix in chunks is the mix in one piece.
    pub clip_offset: u64,
    /// Output sample frames of the whole clip (its fades are relative to this length).
    pub clip_frames: u64,
}

impl AudioSegment {
    /// The fade multiplier of the `index`-th output frame of this segment: a linear ramp from
    /// silence over the first `fade_in` frames of the clip and to silence over the last
    /// `fade_out`, `1.0` everywhere else.
    pub fn fade_gain(&self, index: u64) -> f32 {
        let at = self.clip_offset + index;
        let mut gain = 1.0;
        if self.fade_in > 0 && at < self.fade_in {
            gain *= at as f32 / self.fade_in as f32;
        }
        let remaining = self.clip_frames.saturating_sub(at);
        if self.fade_out > 0 && remaining <= self.fade_out {
            gain *= remaining as f32 / self.fade_out as f32;
        }
        gain
    }
}

/// Fade lengths for a clip of `clip_frames` output frames. Fades that fit are kept as they
/// are. When their sum is longer than the clip they are shortened in proportion to their
/// requested lengths until they exactly fill it, so the two ramps meet at full gain at one
/// frame and never multiply each other into a dip; a single fade longer than the clip is
/// cut to the clip.
pub fn resolve_fades(fade_in: u64, fade_out: u64, clip_frames: u64) -> (u64, u64) {
    let total = fade_in + fade_out;
    if total <= clip_frames {
        return (fade_in, fade_out);
    }
    let shortened = (u128::from(fade_in) * u128::from(clip_frames) / u128::from(total)) as u64;
    (shortened, clip_frames - shortened)
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioPlan {
    /// Output sample rate; the mix is always stereo.
    pub sample_rate: u32,
    /// Output sample frames in the interval.
    pub frames: u64,
    pub segments: Vec<AudioSegment>,
}

impl AudioPlan {
    pub fn to_json(&self) -> Value {
        json!({
            "sample_rate": self.sample_rate,
            "frames": self.frames,
            "segments": self.segments.iter().map(|segment| json!({
                "clip": segment.clip, "asset": segment.asset,
                "output_start": segment.output_start, "frames": segment.frames,
                "source_start": segment.source_start,
                "source_phase": segment.source_phase.to_json(),
                "source_rate": segment.source_rate,
                "speed": segment.speed.to_f64(), "speed_exact": segment.speed.to_json(),
                "gain": segment.gain, "pan": segment.pan,
                "fade_in": segment.fade_in, "fade_out": segment.fade_out,
                "clip_offset": segment.clip_offset, "clip_frames": segment.clip_frames,
            })).collect::<Vec<_>>(),
        })
    }
}

/// Backend that turns an [`AudioPlan`] into stereo PCM.
pub trait AudioBackend {
    fn name(&self) -> &'static str;
    fn mix(&mut self, plan: &AudioPlan) -> Result<Pcm, BackendError>;
}

/// Reference mixer: decodes WAV files and sums segments with gain, pan and linear fades (placed by the clip, see [`AudioSegment::fade_gain`]).
#[derive(Debug, Default)]
pub struct OfflineMixer {
    cache: HashMap<String, Pcm>,
}

impl OfflineMixer {
    fn source(&mut self, path: &str) -> Result<&Pcm, BackendError> {
        if !self.cache.contains_key(path) {
            let bytes = fs::read(path).map_err(|error| {
                BackendError::new("asset_unreadable", format!("Cannot read `{path}`: {error}"))
            })?;
            let pcm = decode_wav(&bytes)
                .map_err(|message| BackendError::new("decode_failed", message))?;
            self.cache.insert(path.to_string(), pcm);
        }
        Ok(&self.cache[path])
    }
}

impl AudioBackend for OfflineMixer {
    fn name(&self) -> &'static str {
        "offline-mixer"
    }

    fn mix(&mut self, plan: &AudioPlan) -> Result<Pcm, BackendError> {
        let mut out = vec![0.0f32; plan.frames as usize * 2];
        for segment in &plan.segments {
            let pcm = self.source(&segment.path)?;
            if pcm.sample_rate != segment.source_rate {
                return Err(BackendError::new(
                    "decode_failed",
                    format!(
                        "`{}` is {} Hz but the asset declares {} Hz",
                        segment.path, pcm.sample_rate, segment.source_rate
                    ),
                ));
            }
            // The position of every output sample is exact (`audio_clock`): the cursor carries
            // the phase of the first sample and the rational step, so a segment, a cut half of
            // it and a chunk of it read the same samples.
            let unrepresentable = || {
                BackendError::new(
                    "invalid_audio_plan",
                    format!(
                        "The source position of `{}` is not representable",
                        segment.clip
                    ),
                )
            };
            let step = source_step(segment.source_rate, segment.speed, plan.sample_rate)
                .ok_or_else(unrepresentable)?;
            let start = Ratio::from_int(segment.source_start)
                .add(segment.source_phase)
                .ok_or_else(unrepresentable)?;
            let mut cursor = SourceCursor::new(start, step).ok_or_else(unrepresentable)?;
            let angle = f64::from(segment.pan.clamp(-1.0, 1.0) + 1.0) * std::f64::consts::FRAC_PI_4;
            let (mono_left, mono_right) = (angle.cos() as f32, angle.sin() as f32);
            let (balance_left, balance_right) =
                (1.0f32.min(1.0 - segment.pan), 1.0f32.min(1.0 + segment.pan));
            for index in 0..segment.frames {
                let (whole, fraction) = cursor.next_position();
                if whole < 0 || whole >= pcm.frames() as i128 {
                    continue;
                }
                let base = whole as usize;
                let channel = |channel: usize| {
                    let a = pcm.sample(base, channel);
                    let b = if base + 1 < pcm.frames() {
                        pcm.sample(base + 1, channel)
                    } else {
                        a
                    };
                    a + (b - a) * fraction
                };
                let (left, right) = if pcm.channels == 1 {
                    let value = channel(0);
                    (value * mono_left, value * mono_right)
                } else {
                    (channel(0) * balance_left, channel(1) * balance_right)
                };
                let mut gain = segment.gain;
                if segment.fade_in > 0 || segment.fade_out > 0 {
                    gain *= segment.fade_gain(index);
                }
                let at = (segment.output_start + index) as usize * 2;
                if let Some(slot) = out.get_mut(at) {
                    *slot += left * gain;
                }
                if let Some(slot) = out.get_mut(at + 1) {
                    *slot += right * gain;
                }
            }
        }
        Ok(Pcm {
            sample_rate: plan.sample_rate,
            channels: 2,
            samples: out,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, frames: usize, value: f32) -> Pcm {
        Pcm {
            sample_rate: rate,
            channels: 1,
            samples: vec![value; frames],
        }
    }

    #[test]
    fn wav_round_trips_and_reports_its_header() {
        let pcm = Pcm {
            sample_rate: 8000,
            channels: 2,
            samples: vec![0.5, -0.5, 0.25, -0.25],
        };
        let bytes = encode_wav_pcm16(&pcm);
        let info = wav_info(&bytes).unwrap();
        assert_eq!((info.sample_rate, info.channels, info.frames), (8000, 2, 2));
        let decoded = decode_wav(&bytes).unwrap();
        for (a, b) in decoded.samples.iter().zip(&pcm.samples) {
            assert!((a - b).abs() < 1e-4);
        }
        assert!(wav_info(b"RIFFxxxxWAVE").is_err());
        assert!(wav_info(&bytes[..30]).is_err());
    }

    #[test]
    fn mixer_applies_gain_pan_and_offset_exactly() {
        let dir = std::env::temp_dir().join(format!("tessel-audio-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("half.wav");
        fs::write(&path, encode_wav_pcm16(&tone(1000, 1000, 0.5))).unwrap();
        let path = path.to_string_lossy().into_owned();
        let segment = |output_start, pan, gain| AudioSegment {
            clip: Id::lit("c"),
            asset: Id::lit("a"),
            path: path.clone(),
            output_start,
            frames: 100,
            source_start: 0,
            source_rate: 1000,
            source_phase: Ratio::ZERO,
            speed: Ratio::from_int(1),
            gain,
            pan,
            fade_in: 0,
            fade_out: 0,
            clip_offset: 0,
            clip_frames: 100,
        };
        let plan = AudioPlan {
            sample_rate: 1000,
            frames: 300,
            segments: vec![segment(100, -1.0, 1.0), segment(200, 0.0, 0.5)],
        };
        let mixed = OfflineMixer::default().mix(&plan).unwrap();
        assert_eq!(mixed.frames(), 300);
        assert_eq!(mixed.sample(50, 0), 0.0, "silence before the first segment");
        assert!(
            (mixed.sample(150, 0) - 0.5).abs() < 1e-3,
            "full-left pan keeps the left channel"
        );
        assert!(mixed.sample(150, 1).abs() < 1e-3);
        let centered = 0.5 * 0.5 * std::f32::consts::FRAC_1_SQRT_2;
        assert!((mixed.sample(250, 0) - centered).abs() < 1e-3);
        assert!((mixed.sample(250, 1) - centered).abs() < 1e-3);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn missing_source_is_a_backend_error() {
        let plan = AudioPlan {
            sample_rate: 1000,
            frames: 10,
            segments: vec![AudioSegment {
                clip: Id::lit("c"),
                asset: Id::lit("a"),
                path: "/nonexistent/none.wav".into(),
                output_start: 0,
                frames: 10,
                source_start: 0,
                source_rate: 1000,
                source_phase: Ratio::ZERO,
                speed: Ratio::from_int(1),
                gain: 1.0,
                pan: 0.0,
                fade_in: 0,
                fade_out: 0,
                clip_offset: 0,
                clip_frames: 10,
            }],
        };
        assert_eq!(
            OfflineMixer::default().mix(&plan).unwrap_err().code,
            "asset_unreadable"
        );
    }
}
