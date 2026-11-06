//! F042A: the audio of a directed-scene export, the media half (BL016).
//!
//! The core decided which instant of the declared WAV every output sample plays
//! ([`DirectedAudioMix`]); this module reads the file and writes the stream FFmpeg then encodes:
//!
//! ```text
//! approved WAV ─▶ identity (size + SHA-256) ─▶ header facts ─▶ core.check_source ─▶ [chunk ─▶ cursor ─▶ s16 stereo]×n ─▶ mix.wav
//! ```
//!
//! * The file is the one the resource map approved: its size and SHA-256 are compared again before
//!   a sample is read, so bytes swapped after approval are `resource_changed`, never mixed. The
//!   export job goes further (BL109): it copies the file into its staging while hashing it and
//!   mixes **only that copy** ([`render_private`]), so a rewrite of the original during the job
//!   cannot reach a sample.
//! * Memory is flat: the file is read one second of output at a time (`seek` + `read_exact`), so a
//!   long mix is as cheap as a short one.
//! * The conversion is the one the core describes: linear interpolation between the two source
//!   frames around the exact position, an exact copy when the position falls on a frame (a 48 kHz
//!   source), mono centred at −3 dB (the `1/√2` the origin's FFmpeg graph and the project mixer use),
//!   declared silence for the tail.
//!
//! Nothing here plays anything: it produces and verifies samples. Hearing them is out of scope.

use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

use serde_json::{Value, json};
use tessel_editor_core::CommandError;
use tessel_editor_core::audio::WavFormat;
use tessel_editor_core::jobs::{JobContext, JobError};
use tessel_editor_core::scene_export::{DirectedAudioMix, MIX_RATE, SourceFacts, SourceSpan};

use crate::fsx::{self, io_error};
use crate::scene_resources::ApprovedResource;

/// Output samples mixed per step: one second.
const CHUNK_SAMPLES: u64 = MIX_RATE as u64;
/// Chunk headers scanned before giving up: a `data` chunk further in is not a plain WAV.
const MAX_CHUNKS: usize = 64;

/// The WAV file as the header describes it: facts for the core's rules plus where the samples are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavLayout {
    pub facts: SourceFacts,
    data_offset: u64,
}

impl WavLayout {
    fn frame_bytes(&self) -> u64 {
        u64::from(self.facts.channels)
            * match self.facts.format {
                WavFormat::Pcm16 => 2,
                WavFormat::Float32 => 4,
            }
    }
}

/// How a mix file fails before the core can judge it: not a file this slice reads (a typed
/// `unsupported`, refused before a job) or a file that cannot be read at all (a preflight blocker).
#[derive(Debug)]
pub enum InspectError {
    Unsupported(CommandError),
    Unreadable(JobError),
}

impl InspectError {
    pub fn code(&self) -> &str {
        match self {
            Self::Unsupported(error) => error.code,
            Self::Unreadable(error) => &error.code,
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::Unsupported(error) => &error.message,
            Self::Unreadable(error) => &error.message,
        }
    }
}

fn unsupported(message: impl Into<String>) -> InspectError {
    InspectError::Unsupported(CommandError::new("unsupported", message).details(json!({
        "accepts": "PCM16 or float32 WAV, mono or stereo, 8000-48000 Hz",
    })))
}

fn le16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn le32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// Reads the RIFF/WAVE structure of `path` without loading the samples. The same encodings the
/// project's own WAV reader takes (`PCM16`, `float32`); the channel and rate rules are the core's.
pub fn inspect(path: &Path) -> Result<WavLayout, InspectError> {
    let read = |error: std::io::Error| InspectError::Unreadable(io_error(path, &error));
    let mut file = File::open(path).map_err(read)?;
    let length = file.metadata().map_err(read)?.len();
    let mut riff = [0u8; 12];
    if file.read_exact(&mut riff).is_err() || &riff[..4] != b"RIFF" || &riff[8..] != b"WAVE" {
        return Err(unsupported(
            "The mix is not a RIFF/WAVE file: only WAV mixes are exported",
        ));
    }
    let mut format: Option<(u16, u16, u32, u16)> = None;
    let mut at = 12u64;
    for _ in 0..MAX_CHUNKS {
        if at + 8 > length {
            break;
        }
        file.seek(SeekFrom::Start(at)).map_err(read)?;
        let mut header = [0u8; 8];
        file.read_exact(&mut header).map_err(read)?;
        let size = u64::from(le32(&header, 4));
        let body = at + 8;
        if &header[..4] == b"fmt " {
            if size < 16 {
                return Err(unsupported("The WAV `fmt ` chunk is truncated"));
            }
            let mut fmt = [0u8; 16];
            file.read_exact(&mut fmt).map_err(read)?;
            format = Some((le16(&fmt, 0), le16(&fmt, 2), le32(&fmt, 4), le16(&fmt, 14)));
        } else if &header[..4] == b"data" {
            let (tag, channels, rate, bits) =
                format.ok_or_else(|| unsupported("The WAV `data` chunk comes before `fmt `"))?;
            let sample_format = match (tag, bits) {
                (1, 16) => WavFormat::Pcm16,
                (3, 32) => WavFormat::Float32,
                _ => {
                    return Err(unsupported(format!(
                        "Unsupported WAV encoding (tag {tag}, {bits} bits): use PCM16 or float32"
                    )));
                }
            };
            if channels == 0 || rate == 0 {
                return Err(unsupported(format!(
                    "Unsupported WAV layout: {channels} channels at {rate} Hz"
                )));
            }
            let layout = WavLayout {
                facts: SourceFacts {
                    format: sample_format,
                    sample_rate: rate,
                    channels,
                    frames: 0,
                },
                data_offset: body,
            };
            let bytes = size.min(length - body);
            return Ok(WavLayout {
                facts: SourceFacts {
                    frames: bytes / layout.frame_bytes(),
                    ..layout.facts
                },
                ..layout
            });
        }
        at = body + size + (size & 1);
    }
    Err(unsupported("The WAV file has no `data` chunk"))
}

/// The file as it must be for the export to use it: header facts, judged by the core.
pub fn check_layout(
    mix: &DirectedAudioMix,
    path: &Path,
) -> Result<(WavLayout, SourceSpan), InspectError> {
    let layout = inspect(path)?;
    let span = mix
        .check_source(&layout.facts)
        .map_err(InspectError::Unsupported)?;
    Ok((layout, span))
}

/// What the written stream measured, for the result and the tests.
#[derive(Debug, Clone, PartialEq)]
pub struct MixReport {
    pub samples: u64,
    pub peak: f32,
    pub rms: f64,
    /// Samples beyond full scale that were clamped.
    pub clipped: u64,
    pub layout: WavLayout,
}

impl MixReport {
    /// The `source` and `measured` parts of `result.audio`.
    pub fn to_json(&self, approved: &ApprovedResource) -> Value {
        let facts = &self.layout.facts;
        json!({
            "source": {
                "sha256": approved.content_hash, "bytes": approved.bytes,
                "format": match facts.format { WavFormat::Pcm16 => "pcm16", WavFormat::Float32 => "float32" },
                "sample_rate": facts.sample_rate, "channels": facts.channels, "frames": facts.frames,
                "converted_rate": facts.sample_rate != MIX_RATE,
                "upmixed_mono": facts.channels == 1,
            },
            "measured": { "samples": self.samples, "peak": self.peak, "rms": self.rms, "clipped": self.clipped },
        })
    }
}

/// A WAV written in pieces (stereo, 48 kHz, PCM16); the header sizes are patched on `finish`.
struct WavWriter {
    file: BufWriter<File>,
    data_bytes: u64,
}

impl WavWriter {
    fn create(path: &Path) -> Result<Self, JobError> {
        let file = File::create(path).map_err(|error| io_error(path, &error))?;
        let mut writer = Self {
            file: BufWriter::new(file),
            data_bytes: 0,
        };
        let mut header = Vec::with_capacity(44);
        header.extend_from_slice(b"RIFF");
        header.extend_from_slice(&0u32.to_le_bytes());
        header.extend_from_slice(b"WAVEfmt ");
        header.extend_from_slice(&16u32.to_le_bytes());
        header.extend_from_slice(&1u16.to_le_bytes());
        header.extend_from_slice(&2u16.to_le_bytes());
        header.extend_from_slice(&MIX_RATE.to_le_bytes());
        header.extend_from_slice(&(MIX_RATE * 4).to_le_bytes());
        header.extend_from_slice(&4u16.to_le_bytes());
        header.extend_from_slice(&16u16.to_le_bytes());
        header.extend_from_slice(b"data");
        header.extend_from_slice(&0u32.to_le_bytes());
        writer
            .file
            .write_all(&header)
            .map_err(|error| io_error(path, &error))?;
        Ok(writer)
    }

    fn write(&mut self, bytes: &[u8], path: &Path) -> Result<(), JobError> {
        self.file
            .write_all(bytes)
            .map_err(|error| io_error(path, &error))?;
        self.data_bytes += bytes.len() as u64;
        Ok(())
    }

    fn finish(mut self, path: &Path) -> Result<(), JobError> {
        let map = |error: std::io::Error| io_error(path, &error);
        self.file.flush().map_err(map)?;
        let mut file = self
            .file
            .into_inner()
            .map_err(|error| io_error(path, error.error()))?;
        let data = u32::try_from(self.data_bytes)
            .map_err(|_| JobError::new("audio_too_long", "The mix exceeds the 4 GiB WAV limit"))?;
        file.seek(SeekFrom::Start(4)).map_err(map)?;
        file.write_all(&(36 + data).to_le_bytes()).map_err(map)?;
        file.seek(SeekFrom::Start(40)).map_err(map)?;
        file.write_all(&data.to_le_bytes()).map_err(map)?;
        file.sync_all().map_err(map)
    }
}

/// Decodes `count` frames starting at the file's frame `first` into `channels` planes of f32.
/// A non-finite sample is silence: one NaN would otherwise poison every neighbour it is mixed with.
fn read_frames(
    file: &mut File,
    path: &Path,
    layout: &WavLayout,
    first: u64,
    count: u64,
) -> Result<Vec<f32>, JobError> {
    let frame_bytes = layout.frame_bytes();
    let mut raw = vec![0u8; (count * frame_bytes) as usize];
    file.seek(SeekFrom::Start(layout.data_offset + first * frame_bytes))
        .and_then(|_| file.read_exact(&mut raw))
        .map_err(|error| io_error(path, &error))?;
    let samples: Vec<f32> = match layout.facts.format {
        WavFormat::Pcm16 => raw
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| f32::from(i16::from_le_bytes(*pair)) / 32768.0)
            .collect(),
        WavFormat::Float32 => raw
            .as_chunks::<4>()
            .0
            .iter()
            .map(|quad| f32::from_le_bytes(*quad))
            .map(|sample| if sample.is_finite() { sample } else { 0.0 })
            .collect(),
    };
    Ok(samples)
}

fn changed(path: &Path, what: &str) -> JobError {
    JobError::new(
        "resource_changed",
        format!(
            "The audio mix `{}` is not the file that was approved ({what}): resolve the resources again",
            path.display()
        ),
    )
}

/// Confirms the file is the approved one: same size, same SHA-256. Streams; observes cancellation.
fn confirm_identity(approved: &ApprovedResource, ctx: &JobContext) -> Result<(), JobError> {
    let size = std::fs::metadata(&approved.path)
        .map_err(|error| io_error(&approved.path, &error))?
        .len();
    if size != approved.bytes {
        return Err(changed(&approved.path, "its size differs"));
    }
    let (hash, _) = fsx::hash_file(&approved.path, ctx.token(), |_| {})?;
    if hash != approved.content_hash {
        return Err(changed(&approved.path, "its SHA-256 differs"));
    }
    Ok(())
}

/// Writes the exported stream (stereo, 48 kHz, PCM16) for `mix` into `out`, reading the file
/// `approved` names after confirming it is still the approved one (size and SHA-256).
pub fn render(
    mix: &DirectedAudioMix,
    approved: &ApprovedResource,
    out: &Path,
    ctx: &JobContext,
) -> Result<MixReport, JobError> {
    ctx.check()?;
    confirm_identity(approved, ctx)?;
    render_private(mix, approved, out, ctx)
}

/// Same stream, for a file that is already the job's private copy (BL109): its bytes were hashed
/// against the approved identity while they were copied, nothing but this job can reach it, and
/// the job checks the copy is intact afterwards. No second hash, and never a read of the
/// original: `approved.path` must be the copy.
pub fn render_private(
    mix: &DirectedAudioMix,
    approved: &ApprovedResource,
    out: &Path,
    ctx: &JobContext,
) -> Result<MixReport, JobError> {
    ctx.check()?;
    let (layout, span) = check_layout(mix, &approved.path).map_err(|error| match error {
        InspectError::Unsupported(error) => {
            let details = error.details.map_or(Value::Null, |boxed| *boxed);
            JobError::new(error.code, error.message).with_details(details)
        }
        InspectError::Unreadable(error) => error,
    })?;
    let cursor = mix
        .cursor(layout.facts.sample_rate)
        .map_err(|error| JobError::new(error.code, error.message))?;
    let mut file = File::open(&approved.path).map_err(|error| io_error(&approved.path, &error))?;
    let mut writer = WavWriter::create(out)?;
    let channels = usize::from(layout.facts.channels);
    let last_frame = layout.facts.frames - 1;
    debug_assert!(span.first + span.frames <= layout.facts.frames);
    let steps = mix.output_samples().div_ceil(CHUNK_SAMPLES);
    ctx.stage("audio", steps);
    let (mut clipped, mut peak, mut energy) = (0u64, 0f32, 0f64);
    let mut bytes: Vec<u8> = Vec::with_capacity(CHUNK_SAMPLES as usize * 4);
    let mut start = 0u64;
    while start < mix.output_samples() {
        ctx.check()?;
        let end = (start + CHUNK_SAMPLES).min(mix.output_samples());
        let live_end = end.min(mix.mix_samples());
        bytes.clear();
        if start < live_end {
            // Frames this chunk reads: the base of its first sample up to one past the base of its
            // last (the interpolation neighbour), never beyond the file.
            let first = cursor.at(start).base;
            let last = (cursor.at(live_end - 1).base + 1).min(last_frame);
            let frames = read_frames(&mut file, &approved.path, &layout, first, last - first + 1)?;
            let frame =
                |index: u64, channel: usize| frames[(index - first) as usize * channels + channel];
            for sample in start..live_end {
                let position = cursor.at(sample);
                let (near, far) = (
                    position.base.min(last_frame),
                    (position.base + 1).min(last_frame),
                );
                let fraction = position.fraction as f32;
                let pair = |channel: usize| {
                    let a = frame(near, channel);
                    if fraction == 0.0 {
                        a
                    } else {
                        a + (frame(far, channel) - a) * fraction
                    }
                };
                let (left, right) = if channels == 1 {
                    let centred = pair(0) * std::f32::consts::FRAC_1_SQRT_2;
                    (centred, centred)
                } else {
                    (pair(0), pair(1))
                };
                for value in [left, right] {
                    if value.abs() > 1.0 {
                        clipped += 1;
                    }
                    peak = peak.max(value.abs());
                    energy += f64::from(value).powi(2);
                    let quantised = (value.clamp(-1.0, 1.0) * 32767.0).round() as i16;
                    bytes.extend_from_slice(&quantised.to_le_bytes());
                }
            }
        }
        // Declared silence: the tail of a rate conversion (and nothing else) is zero.
        let silent = (end - start) - live_end.saturating_sub(start);
        bytes.resize(bytes.len() + silent as usize * 4, 0);
        writer.write(&bytes, out)?;
        start = end;
        ctx.advance(1);
    }
    writer.finish(out)?;
    let measured = mix.mix_samples();
    Ok(MixReport {
        samples: mix.output_samples(),
        peak,
        rms: (energy / (measured.max(1) * 2) as f64).sqrt(),
        clipped,
        layout,
    })
}
