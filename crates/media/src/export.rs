//! Video export (P10/P11): the immutable project snapshot becomes an H.264/AAC MP4.
//!
//! The same core plan that the monitor shows decides every pixel, every frame time and every
//! audio sample; nothing here screenshots a window. The flow is
//! `preflight → prepare → audio mix → per-frame render → FFmpeg → ffprobe verification → publish`,
//! all inside a staging directory on the destination volume, so a failure or a cancel leaves no
//! partial file at the destination and no staging directory behind.
//!
//! Memory is bounded: one PNG frame at a time crosses the pipe to FFmpeg, and the audio is mixed
//! in 10-second chunks written straight to a WAV file. The one exception is the offline mixer's
//! decoded source cache, which is why preflight caps the WAV sources it will accept.

use std::fs::{self, File};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tessel_editor_core::EditorState;
use tessel_editor_core::assets::{Asset, MediaKind, Source};
use tessel_editor_core::audio::{AudioBackend, OfflineMixer};
use tessel_editor_core::backend::{Capabilities, Output};
use tessel_editor_core::jobs::{JobContext, JobError};
use tessel_editor_core::plan::{
    AudioSource, MIX_SAMPLE_RATE, PlanRequest, PlanTime, PreviewIntent,
};
use tessel_editor_core::preset::{Fit, Geometry, Preset};
use tessel_editor_core::session::Renderer;
use tessel_editor_core::time::{Rate, Ratio};

use crate::derive::Cache;
use crate::fsx::{self, Staging, io_error};
use crate::import::{self, FileState};
use crate::tools::Tools;

/// Audio is mixed this many seconds at a time.
const AUDIO_CHUNK_SECONDS: i64 = 10;
/// The offline mixer holds each source decoded as f32 (twice the WAV size).
const AUDIO_SOURCE_LIMIT: u64 = 1 << 30;
/// A 16-bit stereo 48 kHz WAV stays below 4 GiB for this long.
const MAX_AUDIO_SECONDS: i64 = 6 * 3600;
const SPACE_MARGIN: u64 = 16 << 20;
const DEFAULT_FRAME_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone)]
pub struct ExportRequest {
    pub preset: Preset,
    pub destination: PathBuf,
    /// Sequence frames `[start, end)`; `None` exports the whole sequence.
    pub range: Option<(i64, i64)>,
    pub replace: bool,
    pub frame_timeout: Duration,
}

impl ExportRequest {
    pub fn new(preset: Preset, destination: impl Into<PathBuf>) -> Self {
        Self {
            preset,
            destination: destination.into(),
            range: None,
            replace: false,
            frame_timeout: DEFAULT_FRAME_TIMEOUT,
        }
    }
}

/// Everything derived from the sequence and the preset before any work starts.
#[derive(Debug, Clone)]
pub struct Schedule {
    pub from: i64,
    pub to: i64,
    pub sequence_rate: Rate,
    pub output_rate: Rate,
    pub frames: u64,
    pub geometry: Geometry,
    /// Samples of the 48 kHz mix, equal to the video duration to the sample.
    pub audio_samples: u64,
}

impl Schedule {
    pub fn duration_seconds(&self) -> f64 {
        self.frames as f64 * f64::from(self.output_rate.den()) / f64::from(self.output_rate.num())
    }

    /// Sequence time of output frame `index`. Exact rational arithmetic: a 30000/1001 export
    /// never drifts against the sequence, and a rate change picks the sequence frame that is
    /// showing at that instant.
    fn frame_time(&self, index: u64) -> Ratio {
        let start = self.sequence_rate.frame_to_time(self.from);
        let offset = Ratio::new(
            index as i64 * i64::from(self.output_rate.den()),
            i64::from(self.output_rate.num()),
        )
        .unwrap_or(Ratio::ZERO);
        start.add(offset).unwrap_or(start)
    }

    fn plan_time(&self, index: u64) -> PlanTime {
        if self.output_rate == self.sequence_rate {
            return PlanTime::Frame(self.from + index as i64);
        }
        PlanTime::Time(self.frame_time(index))
    }
}

fn blocker(code: &str, message: impl Into<String>) -> Value {
    json!({ "code": code, "message": message.into() })
}

pub fn schedule(state: &EditorState, request: &ExportRequest) -> Result<Schedule, JobError> {
    let timeline = &state.timeline;
    let (from, to) = request.range.unwrap_or((0, timeline.length));
    if from < 0 || from >= to || to > timeline.length {
        return Err(JobError::new(
            "invalid_range",
            format!(
                "Range {from}..{to} is outside the sequence (0..{})",
                timeline.length
            ),
        )
        .with_details(json!({ "from": from, "to": to, "length": timeline.length })));
    }
    let sequence_rate = timeline.rate;
    let output_rate = request.preset.fps.unwrap_or(sequence_rate);
    let duration = sequence_rate.frame_to_time(to - from);
    let frames = Ratio::new(i64::from(output_rate.num()), i64::from(output_rate.den()))
        .and_then(|rate| duration.mul(rate))
        .map_or(0, Ratio::round)
        .max(1) as u64;
    let seconds = Ratio::new(
        frames as i64 * i64::from(output_rate.den()),
        i64::from(output_rate.num()),
    )
    .and_then(|time| time.mul_int(i64::from(MIX_SAMPLE_RATE)))
    .map_or(0, Ratio::round)
    .max(0) as u64;
    Ok(Schedule {
        from,
        to,
        sequence_rate,
        output_rate,
        frames,
        geometry: request.preset.geometry(timeline.width, timeline.height),
        audio_samples: seconds,
    })
}

/// Library items the range touches: clip media plus what scenes require.
fn used_assets(state: &EditorState, from: i64, to: i64) -> Vec<(String, MediaKind, Asset)> {
    let mut ids: Vec<String> = Vec::new();
    let mut push = |id: &str| {
        if !ids.iter().any(|known| known == id) {
            ids.push(id.to_string());
        }
    };
    for clip in &state.timeline.clips {
        if clip.start >= to || clip.end() <= from {
            continue;
        }
        let Some(item) = clip.media.and_then(|media| state.library.item(media)) else {
            continue;
        };
        push(item.id.as_str());
        if let Some(asset) = &item.asset {
            for required in &asset.meta.requires {
                push(required.as_str());
            }
        }
    }
    ids.iter()
        .filter_map(|id| {
            let item = state.library.item(id)?;
            Some((id.clone(), item.kind, item.asset.clone()?))
        })
        .collect()
}

/// Streams the range plays, one entry per (asset, stream): clips of an audio track and the own
/// sound of container videos, chosen by the core so export and playback agree.
fn audio_sources(state: &EditorState, from: i64, to: i64) -> Result<Vec<AudioSource>, JobError> {
    let mut sources = state
        .audio_sources(from, to)
        .map_err(|error| JobError::new(error.code, error.message))?;
    sources.sort_by_key(|source| (source.asset.as_str().to_string(), source.stream));
    sources.dedup_by_key(|source| (source.asset, source.stream));
    Ok(sources)
}

fn has_audio_clips(state: &EditorState, from: i64, to: i64) -> bool {
    state
        .audio_sources(from, to)
        .is_ok_and(|sources| !sources.is_empty())
}

/// Container video and compressed audio are decoded to deterministic frames/WAV before use.
pub fn needs_decoding(kind: MediaKind, asset: &Asset) -> bool {
    match kind {
        // A frame-sequence video is already numbered PNGs; only container files are decoded.
        MediaKind::Video => matches!(asset.source, Source::File { .. }),
        MediaKind::Audio => asset.meta.format.as_deref() != Some("wav"),
        _ => false,
    }
}

fn estimated_video_bytes(preset: &Preset, schedule: &Schedule) -> u64 {
    // Bits per pixel of x264 at CRF 18 on typical content is ~0.08; it halves every +6 CRF.
    let bits_per_pixel = 0.08 * 2f64.powf((18.0 - f64::from(preset.crf)) / 6.0);
    let pixels = f64::from(preset.width) * f64::from(preset.height);
    (pixels * bits_per_pixel * schedule.frames as f64 / 8.0) as u64
}

/// What an export would do and what stops it, without touching the destination.
///
/// `blockers` are conditions under which `run` refuses to start; `warnings` are choices the user
/// should know about (frame-rate conversion, letterboxing, audio that will not be mixed).
pub fn preflight(
    state: &EditorState,
    request: &ExportRequest,
    tools: &Tools,
    capabilities: Option<&Capabilities>,
) -> Value {
    let mut blockers = Vec::new();
    let mut warnings = Vec::new();
    let preset = &request.preset;

    if let Err(error) = preset.validate() {
        blockers.push(blocker("invalid_preset", error.message));
    }
    if tools.h264_encoder() != Some("libx264") {
        blockers.push(blocker(
            "encoder_unavailable",
            format!("This FFmpeg ({}) has no libx264 encoder", tools.version),
        ));
    }
    if preset.audio && !tools.has_encoder("aac") {
        blockers.push(blocker(
            "encoder_unavailable",
            "This FFmpeg has no AAC encoder",
        ));
    }
    if !tools.has_muxer("mp4") || !tools.has_decoder("png") {
        blockers.push(blocker(
            "encoder_unavailable",
            "This FFmpeg lacks the mp4 muxer or the png decoder",
        ));
    }

    let schedule = match schedule(state, request) {
        Ok(schedule) => schedule,
        Err(error) => {
            blockers.push(blocker(&error.code, error.message));
            return report(blockers, warnings, Value::Null);
        }
    };

    let extension_ok = request
        .destination
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"));
    if !extension_ok {
        blockers.push(blocker(
            "invalid_destination",
            "The destination must be an `.mp4` file",
        ));
    }
    if request.destination.exists() && !request.replace {
        blockers.push(blocker(
            "destination_exists",
            format!("`{}` already exists", request.destination.display()),
        ));
    }
    if let Some(parent) = request
        .destination
        .parent()
        .filter(|parent| parent.exists())
        && !parent.is_dir()
    {
        blockers.push(blocker(
            "invalid_destination",
            format!("`{}` is not a folder", parent.display()),
        ));
    }

    if let Some(max) = capabilities.and_then(|caps| caps.raw["capabilities"]["max_size"].as_u64()) {
        let (width, height) = schedule.geometry.render;
        if u64::from(width.max(height)) > max {
            blockers.push(blocker(
                "render_too_large",
                format!("The renderer draws at most {max}px per side; this export needs {width}x{height}"),
            ));
        }
    }

    let assets = used_assets(state, schedule.from, schedule.to);
    let mut audio_wav_bytes = 0u64;
    let mut to_prepare = Vec::new();
    for (id, kind, asset) in &assets {
        match import::file_state(asset) {
            FileState::Missing => blockers.push(
                blocker(
                    "asset_missing",
                    format!("The file of `{id}` is missing; relink it before exporting"),
                )
                .with_asset(id),
            ),
            FileState::Changed => blockers.push(
                blocker(
                    "asset_changed",
                    format!("The file of `{id}` changed since import; relink it"),
                )
                .with_asset(id),
            ),
            FileState::Ok | FileState::NotFile => {}
        }
        if needs_decoding(*kind, asset) {
            to_prepare.push(id.clone());
        }
        if asset.meta.vfr_conformed {
            warnings.push(
                blocker(
                    "variable_frame_rate",
                    format!("`{id}` has a variable frame rate: its frames are conformed by timestamp to a constant {:.3} fps grid (at most half a frame of timing error)",
                        asset.meta.rate.map_or(0.0, Rate::as_f64)),
                )
                .with_asset(id),
            );
        }
    }
    // Sound is sized from what the mix will actually read: one WAV per played stream.
    let mut audio_source_list = Vec::new();
    match audio_sources(state, schedule.from, schedule.to) {
        Ok(sources) => {
            for source in &sources {
                let Some((_, _, asset)) = assets
                    .iter()
                    .find(|(id, ..)| id.as_str() == source.asset.as_str())
                else {
                    continue;
                };
                let direct_wav = !source.embedded && asset.meta.format.as_deref() == Some("wav");
                audio_wav_bytes += if direct_wav {
                    asset.identity.size.unwrap_or(0)
                } else {
                    let stream = asset.meta.audio_stream(source.stream);
                    let seconds = asset.meta.duration.map_or(0.0, Ratio::to_f64);
                    let rate = stream.map_or(48_000, |s| s.sample_rate);
                    let channels = stream.map_or(2, |s| s.channels.clamp(1, 2));
                    (seconds * f64::from(rate) * f64::from(channels) * 2.0) as u64
                };
                audio_source_list.push(json!({ "clip": source.clip, "asset": source.asset,
                    "stream": source.stream, "embedded": source.embedded }));
            }
        }
        Err(error) => blockers.push(blocker(&error.code, error.message)),
    }
    if !to_prepare.is_empty() && state.project.workspace.is_none() {
        blockers.push(blocker(
            "no_workspace",
            "Video and compressed audio are decoded into the project workspace; save the project first",
        ));
    }

    let mut audio_present = false;
    if preset.audio {
        audio_present = has_audio_clips(state, schedule.from, schedule.to);
        if !audio_present {
            warnings.push(blocker(
                "no_audio_sources",
                "Nothing in the range makes sound (no audio clip and no video with its own audio): the file will have no audio stream",
            ));
        }
    }
    if audio_present {
        if audio_wav_bytes > AUDIO_SOURCE_LIMIT {
            blockers.push(blocker(
                "audio_too_large",
                format!(
                    "Audio sources total {} MiB; the offline mixer accepts up to {} MiB",
                    audio_wav_bytes >> 20,
                    AUDIO_SOURCE_LIMIT >> 20
                ),
            ));
        }
        if schedule.to - schedule.from > MAX_AUDIO_SECONDS * schedule.sequence_rate.nominal_fps() {
            blockers.push(blocker(
                "audio_too_long",
                "The audio of a single export is limited to 6 hours",
            ));
        }
    }

    if schedule.output_rate != schedule.sequence_rate {
        warnings.push(blocker(
            "frame_rate_conversion",
            format!(
                "The sequence runs at {:.3} fps and the preset at {:.3} fps: frames are repeated or dropped, never blended",
                schedule.sequence_rate.as_f64(),
                schedule.output_rate.as_f64()
            ),
        ));
    }
    if schedule.geometry.render != schedule.geometry.output {
        let message = match schedule.geometry.fit {
            Fit::Contain => "The sequence aspect differs from the output: black bars are added",
            Fit::Cover => {
                "The sequence aspect differs from the output: the picture is cropped from the center"
            }
        };
        warnings.push(blocker("aspect_mismatch", message));
    }

    let video = estimated_video_bytes(preset, &schedule);
    let seconds = schedule.duration_seconds();
    let audio_aac = if audio_present {
        (seconds * f64::from(preset.audio_kbps) * 125.0) as u64
    } else {
        0
    };
    let audio_wav = if audio_present {
        schedule.audio_samples * 4
    } else {
        0
    };
    let (render_w, render_h) = schedule.geometry.render;
    let frame_bytes = u64::from(render_w) * u64::from(render_h) * 4;
    let needed = video + audio_aac + audio_wav + frame_bytes;
    let free = request.destination.parent().and_then(fsx::free_bytes);
    if let Some(free) = free
        && free < needed.saturating_add(SPACE_MARGIN)
    {
        blockers.push(blocker(
            "disk_full",
            format!(
                "The export needs about {} MiB (estimate) but only {} MiB are free at the destination",
                needed.div_ceil(1 << 20),
                free >> 20
            ),
        ));
    }

    let summary = json!({
        "frames": schedule.frames,
        "fps": schedule.output_rate.to_json(),
        "duration_seconds": seconds,
        "range": [schedule.from, schedule.to],
        "render": { "width": render_w, "height": render_h },
        "output": { "width": preset.width, "height": preset.height },
        "fit": preset.fit.label(),
        "audio": { "enabled": audio_present, "samples": if audio_present { schedule.audio_samples } else { 0 }, "sample_rate": MIX_SAMPLE_RATE, "sources": audio_source_list },
        "assets": assets.iter().map(|(id, ..)| id).collect::<Vec<_>>(),
        "to_prepare": to_prepare,
        "estimate": { "output_bytes": video + audio_aac, "staging_bytes": needed,
            "memory_bytes": frame_bytes * 3 + audio_wav_bytes * 2,
            "note": "Estimates from typical content; the encoder decides the real size" },
        "free_bytes": free,
        "encoder": tools.h264_encoder(),
        "ffmpeg": tools.version,
        "renderer": capabilities.map(Capabilities::to_json),
        "destination": request.destination,
    });
    report(blockers, warnings, summary)
}

trait WithAsset {
    fn with_asset(self, id: &str) -> Self;
}

impl WithAsset for Value {
    fn with_asset(mut self, id: &str) -> Self {
        self["asset"] = json!(id);
        self
    }
}

fn report(blockers: Vec<Value>, warnings: Vec<Value>, summary: Value) -> Value {
    json!({ "ok": blockers.is_empty(), "blockers": blockers, "warnings": warnings, "summary": summary })
}

/// The FFmpeg video filter that turns the rendered RGB frame into the output picture: crop or
/// pad (with even offsets, so 4:2:0 chroma stays aligned), then an explicit BT.709 TV-range
/// conversion. Without the explicit matrix FFmpeg would guess from the size.
pub fn filter_chain(geometry: &Geometry) -> String {
    let (render_w, render_h) = geometry.render;
    let (out_w, out_h) = geometry.output;
    let centered = |outer: u32, inner: u32| (outer.abs_diff(inner) / 2) & !1;
    let mut stages = vec!["format=rgb24".to_string()];
    if render_w > out_w || render_h > out_h {
        let (crop_w, crop_h) = (render_w.min(out_w), render_h.min(out_h));
        stages.push(format!(
            "crop={crop_w}:{crop_h}:{}:{}",
            centered(render_w, crop_w),
            centered(render_h, crop_h)
        ));
    }
    if render_w < out_w || render_h < out_h {
        let (inner_w, inner_h) = (render_w.min(out_w), render_h.min(out_h));
        stages.push(format!(
            "pad={out_w}:{out_h}:{}:{}:color=black",
            centered(out_w, inner_w),
            centered(out_h, inner_h)
        ));
    }
    stages.push(
        "scale=flags=accurate_rnd+full_chroma_int:out_color_matrix=bt709:out_range=tv".into(),
    );
    stages.push("format=yuv420p".into());
    stages.push("setsar=1".into());
    // The output options alone leave primaries and transfer unset in the stream (verified with
    // ffprobe); tagging the frames makes the encoder write all three into the VUI.
    stages.push("setparams=colorspace=bt709:color_primaries=bt709:color_trc=bt709:range=tv".into());
    stages.join(",")
}

fn rate_arg(rate: Rate) -> String {
    format!("{}/{}", rate.num(), rate.den())
}

fn encode_args(
    request: &ExportRequest,
    schedule: &Schedule,
    audio: Option<&Path>,
    output: &Path,
) -> Vec<String> {
    let preset = &request.preset;
    let mut args: Vec<String> = [
        "-v",
        "error",
        "-nostdin",
        "-y",
        "-f",
        "image2pipe",
        "-framerate",
    ]
    .map(String::from)
    .into();
    args.push(rate_arg(schedule.output_rate));
    args.extend(["-c:v", "png", "-i", "-"].map(String::from));
    if let Some(audio) = audio {
        args.extend(["-i".to_string(), audio.to_string_lossy().into_owned()]);
    }
    args.extend(["-vf".to_string(), filter_chain(&schedule.geometry)]);
    args.extend(["-map", "0:v:0"].map(String::from));
    if audio.is_some() {
        args.extend(["-map", "1:a:0"].map(String::from));
    }
    args.extend(
        [
            "-c:v",
            "libx264",
            "-preset",
            "medium",
            "-profile:v",
            "high",
            "-pix_fmt",
            "yuv420p",
        ]
        .map(String::from),
    );
    args.extend(["-crf".to_string(), preset.crf.to_string()]);
    args.extend(
        [
            "-colorspace",
            "bt709",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-color_range",
            "tv",
        ]
        .map(String::from),
    );
    args.extend([
        "-r".to_string(),
        rate_arg(schedule.output_rate),
        "-fps_mode".into(),
        "cfr".into(),
    ]);
    if audio.is_some() {
        args.extend(["-c:a", "aac", "-ar", "48000", "-ac", "2"].map(String::from));
        args.extend(["-b:a".to_string(), format!("{}k", preset.audio_kbps)]);
    }
    args.extend(
        [
            "-map_metadata",
            "-1",
            "-movflags",
            "+faststart",
            "-f",
            "mp4",
        ]
        .map(String::from),
    );
    args.push(output.to_string_lossy().into_owned());
    args
}

/// A WAV file written in pieces: the header sizes are patched on `finish`, so memory stays flat.
struct WavWriter {
    file: BufWriter<File>,
    path: PathBuf,
    data_bytes: u64,
}

impl WavWriter {
    fn create(path: &Path) -> Result<Self, JobError> {
        let file = File::create(path).map_err(|error| io_error(path, &error))?;
        let mut writer = Self {
            file: BufWriter::new(file),
            path: path.to_path_buf(),
            data_bytes: 0,
        };
        let mut header = Vec::with_capacity(44);
        header.extend_from_slice(b"RIFF");
        header.extend_from_slice(&0u32.to_le_bytes());
        header.extend_from_slice(b"WAVEfmt ");
        header.extend_from_slice(&16u32.to_le_bytes());
        header.extend_from_slice(&1u16.to_le_bytes());
        header.extend_from_slice(&2u16.to_le_bytes());
        header.extend_from_slice(&MIX_SAMPLE_RATE.to_le_bytes());
        header.extend_from_slice(&(MIX_SAMPLE_RATE * 4).to_le_bytes());
        header.extend_from_slice(&4u16.to_le_bytes());
        header.extend_from_slice(&16u16.to_le_bytes());
        header.extend_from_slice(b"data");
        header.extend_from_slice(&0u32.to_le_bytes());
        // The header is not data: the counter starts after it.
        writer
            .file
            .write_all(&header)
            .map_err(|error| io_error(path, &error))?;
        Ok(writer)
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), JobError> {
        self.file
            .write_all(bytes)
            .map_err(|error| io_error(&self.path, &error))?;
        self.data_bytes += bytes.len() as u64;
        Ok(())
    }

    fn write_samples(&mut self, samples: &[f32]) -> Result<(), JobError> {
        let mut bytes = Vec::with_capacity(samples.len() * 2);
        for sample in samples {
            let value = (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16;
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        self.write(&bytes)
    }

    fn finish(mut self) -> Result<u64, JobError> {
        let path = self.path.clone();
        let map = |error: std::io::Error| io_error(&path, &error);
        self.file.flush().map_err(map)?;
        let mut file = self
            .file
            .into_inner()
            .map_err(|error| io_error(&path, error.error()))?;
        let data = self.data_bytes;
        let data = u32::try_from(data)
            .map_err(|_| JobError::new("audio_too_long", "The mix exceeds the 4 GiB WAV limit"))?;
        file.seek(SeekFrom::Start(4)).map_err(map)?;
        file.write_all(&(36 + data).to_le_bytes()).map_err(map)?;
        file.seek(SeekFrom::Start(40)).map_err(map)?;
        file.write_all(&data.to_le_bytes()).map_err(map)?;
        file.sync_all().map_err(map)?;
        Ok(u64::from(data) / 4)
    }
}

struct AudioReport {
    samples: u64,
    clipped: u64,
    peak: f32,
    rms: f64,
}

/// Mixes the range chunk by chunk into `path`. A chunk starts at its exact sample boundary: the
/// planner rounds each chunk independently, so a chunk is trimmed or the gap zero-filled by at
/// most one sample, and the error never accumulates across chunks. The file ends with exactly
/// `schedule.audio_samples` samples, which is what makes video and audio the same length.
fn render_audio(
    state: &EditorState,
    schedule: &Schedule,
    path: &Path,
    ctx: &JobContext,
) -> Result<AudioReport, JobError> {
    let mut writer = WavWriter::create(path)?;
    let mut mixer = OfflineMixer::default();
    let chunk_frames = (schedule.sequence_rate.nominal_fps() * AUDIO_CHUNK_SECONDS).max(1);
    let chunks = (schedule.to - schedule.from + chunk_frames - 1) / chunk_frames;
    ctx.stage("audio", chunks as u64);
    let target = schedule.audio_samples;
    let (mut written, mut clipped, mut peak, mut energy) = (0u64, 0u64, 0f32, 0f64);
    let mut from = schedule.from;
    while from < schedule.to && written < target {
        ctx.check()?;
        let frames = chunk_frames.min(schedule.to - from);
        let plan = state
            .plan_audio(from, frames)
            .map_err(|error| JobError::new("render_failed", error.message))?;
        let pcm = mixer.mix(&plan).map_err(|error| {
            JobError::new(error.code.clone(), format!("Audio mix: {}", error.message))
        })?;
        let exact_start = schedule
            .sequence_rate
            .frame_to_time(from - schedule.from)
            .mul_int(i64::from(MIX_SAMPLE_RATE))
            .map_or(0, Ratio::round)
            .max(0) as u64;
        let mut samples: &[f32] = &pcm.samples;
        if written < exact_start {
            let gap = (exact_start - written).min(target - written);
            writer.write_samples(&vec![0.0; gap as usize * 2])?;
            written += gap;
        } else if written > exact_start {
            let skip = ((written - exact_start) as usize * 2).min(samples.len());
            samples = &samples[skip..];
        }
        let room = (target - written) as usize * 2;
        samples = &samples[..samples.len().min(room)];
        for sample in samples {
            if sample.abs() > 1.0 {
                clipped += 1;
            }
            peak = peak.max(sample.abs());
            energy += f64::from(*sample).powi(2);
        }
        writer.write_samples(samples)?;
        written += samples.len() as u64 / 2;
        from += frames;
        ctx.advance(1);
    }
    if written < target {
        writer.write_samples(&vec![0.0; (target - written) as usize * 2])?;
        written = target;
    }
    let samples = writer.finish()?;
    debug_assert_eq!(samples, written);
    Ok(AudioReport {
        samples,
        clipped,
        peak,
        rms: (energy / (written.max(1) * 2) as f64).sqrt(),
    })
}

fn rss_kib(pid: u32) -> Option<u64> {
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

fn read_tail(path: &Path) -> String {
    let text = fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text
        .lines()
        .rev()
        .filter(|line| !line.trim().is_empty())
        .take(3)
        .collect();
    lines.into_iter().rev().collect::<Vec<_>>().join(" | ")
}

fn encode_error(log: &Path, fallback: &str) -> JobError {
    let tail = read_tail(log);
    let lowered = tail.to_lowercase();
    let code = if lowered.contains("no space left") {
        "disk_full"
    } else {
        "encode_failed"
    };
    let message = if tail.is_empty() {
        fallback.to_string()
    } else {
        format!("FFmpeg: {tail}")
    };
    JobError::new(code, message)
}

/// Width and height from the PNG header, without decoding the picture.
fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let read =
        |at: usize| u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    Some((read(16), read(20)))
}

fn kill(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[derive(Default)]
struct Metrics {
    render_ms: f64,
    pipe_ms: f64,
    slowest_ms: f64,
    host_rss_kib: u64,
    ffmpeg_rss_kib: u64,
}

/// What the frame loop needs besides the renderer and the job context.
struct Encode<'a> {
    tools: &'a Tools,
    snapshot: &'a EditorState,
    request: &'a ExportRequest,
    schedule: &'a Schedule,
    audio: Option<&'a Path>,
    staging: &'a Path,
    output: &'a Path,
}

fn render_and_encode(
    job: &Encode,
    renderer: &mut dyn Renderer,
    ctx: &JobContext,
) -> Result<Metrics, JobError> {
    let Encode {
        tools,
        snapshot,
        request,
        schedule,
        audio,
        staging,
        output,
    } = *job;
    let log = staging.join("ffmpeg.log");
    let frame_path = staging.join("frame.png");
    let log_file = File::create(&log).map_err(|error| io_error(&log, &error))?;
    let mut child = Command::new(&tools.ffmpeg)
        .args(encode_args(request, schedule, audio, output))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(log_file)
        .spawn()
        .map_err(|error| {
            JobError::new("tool_unavailable", format!("Cannot start ffmpeg: {error}"))
        })?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| JobError::new("encode_failed", "ffmpeg has no input pipe"))?;
    let mut metrics = Metrics::default();
    let (width, height) = schedule.geometry.render;
    ctx.stage("render", schedule.frames);
    let result = (|| -> Result<(), JobError> {
        for index in 0..schedule.frames {
            ctx.check()?;
            let plan = snapshot
                .plan(&PlanRequest {
                    intent: PreviewIntent::Sequence,
                    at: schedule.plan_time(index),
                })
                .map_err(|error| {
                    JobError::new("render_failed", format!("Frame {index}: {}", error.message))
                })?;
            let started = Instant::now();
            let output = Output {
                width,
                height,
                png: Some(frame_path.to_string_lossy().into_owned()),
            };
            renderer
                .render_frame(snapshot, plan, output, Vec::new(), request.frame_timeout)
                .map_err(|error| {
                    let disk = error.message.to_lowercase().contains("no space left");
                    JobError::new(
                        if disk { "disk_full" } else { "render_failed" },
                        format!("Frame {index}: {} ({})", error.message, error.code),
                    )
                })?;
            let bytes = fs::read(&frame_path).map_err(|error| io_error(&frame_path, &error))?;
            if png_size(&bytes) != Some((width, height)) {
                return Err(JobError::new(
                    "render_failed",
                    format!(
                        "Frame {index}: the renderer produced {:?}, expected {width}x{height}",
                        png_size(&bytes)
                    ),
                ));
            }
            let rendered = started.elapsed().as_secs_f64() * 1000.0;
            let piped = Instant::now();
            if stdin.write_all(&bytes).is_err() {
                return Err(encode_error(
                    &log,
                    "ffmpeg closed its input before the last frame",
                ));
            }
            metrics.pipe_ms += piped.elapsed().as_secs_f64() * 1000.0;
            metrics.render_ms += rendered;
            metrics.slowest_ms = metrics.slowest_ms.max(rendered);
            if index % 30 == 0 {
                metrics.host_rss_kib = metrics
                    .host_rss_kib
                    .max(rss_kib(std::process::id()).unwrap_or(0));
                metrics.ffmpeg_rss_kib =
                    metrics.ffmpeg_rss_kib.max(rss_kib(child.id()).unwrap_or(0));
            }
            ctx.advance(1);
        }
        Ok(())
    })();
    if let Err(error) = result {
        drop(stdin);
        kill(&mut child);
        return Err(error);
    }
    drop(stdin);
    ctx.stage("finalize", 1);
    let status = loop {
        if ctx.is_cancelled() {
            kill(&mut child);
            return Err(JobError::cancelled());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(error) => return Err(JobError::new("encode_failed", format!("ffmpeg: {error}"))),
        }
    };
    if !status.success() {
        return Err(encode_error(&log, "ffmpeg failed"));
    }
    Ok(metrics)
}

/// Below this much free space, an encoder or renderer failure is a full disk whatever message
/// the tool printed (FFmpeg often exits with no text when the final write of the file fails).
const FULL_DISK_THRESHOLD: u64 = 8 << 20;

fn as_disk_full(error: JobError, staging: &Path) -> JobError {
    let failed_io = matches!(error.code.as_str(), "encode_failed" | "render_failed");
    let Some(free) =
        fsx::free_bytes(staging).filter(|free| failed_io && *free < FULL_DISK_THRESHOLD)
    else {
        return error;
    };
    JobError::new(
        "disk_full",
        format!(
            "The destination volume is full ({} KiB free): {}",
            free >> 10,
            error.message
        ),
    )
}

fn parse_fraction(text: &str) -> Option<(i64, i64)> {
    let (num, den) = text.split_once('/')?;
    Some((num.parse().ok()?, den.parse().ok()?))
}

fn number(value: &Value) -> Option<f64> {
    value
        .as_str()
        .and_then(|text| text.parse().ok())
        .or_else(|| value.as_f64())
}

fn check(checks: &mut Vec<Value>, name: &str, ok: bool, expected: Value, actual: Value) {
    checks.push(json!({ "name": name, "ok": ok, "expected": expected, "actual": actual }));
}

/// Reads the produced file back with ffprobe and compares it with what the schedule promised:
/// container, codec, size, pixel format, color tags, frame rate and count, duration and the
/// audio stream. A mismatch fails the export; the staged file is never published.
pub fn verify(
    tools: &Tools,
    path: &Path,
    request: &ExportRequest,
    schedule: &Schedule,
    expect_audio: bool,
) -> Result<Value, JobError> {
    let output = Command::new(&tools.ffprobe)
        .args([
            "-v",
            "error",
            "-show_format",
            "-show_streams",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .map_err(|error| {
            JobError::new("tool_unavailable", format!("Cannot run ffprobe: {error}"))
        })?;
    let probe: Value = serde_json::from_slice(&output.stdout).map_err(|_| {
        JobError::new(
            "verification_failed",
            "ffprobe could not read the exported file",
        )
    })?;
    let streams = probe["streams"].as_array().cloned().unwrap_or_default();
    let find = |kind: &str| streams.iter().find(|stream| stream["codec_type"] == kind);
    let mut checks = Vec::new();
    let preset = &request.preset;

    let format_name = probe["format"]["format_name"].as_str().unwrap_or_default();
    check(
        &mut checks,
        "container",
        format_name.contains("mp4"),
        json!("mp4"),
        json!(format_name),
    );
    let Some(video) = find("video") else {
        check(
            &mut checks,
            "video_stream",
            false,
            json!(true),
            json!(false),
        );
        return Err(failed(checks));
    };
    check(
        &mut checks,
        "video_codec",
        video["codec_name"] == "h264",
        json!("h264"),
        video["codec_name"].clone(),
    );
    check(
        &mut checks,
        "size",
        video["width"] == preset.width && video["height"] == preset.height,
        json!([preset.width, preset.height]),
        json!([video["width"], video["height"]]),
    );
    check(
        &mut checks,
        "pix_fmt",
        video["pix_fmt"] == "yuv420p",
        json!("yuv420p"),
        video["pix_fmt"].clone(),
    );
    for (name, key) in [
        ("color_space", "color_space"),
        ("color_primaries", "color_primaries"),
        ("color_transfer", "color_transfer"),
    ] {
        check(
            &mut checks,
            name,
            video[key] == "bt709",
            json!("bt709"),
            video[key].clone(),
        );
    }
    check(
        &mut checks,
        "color_range",
        video["color_range"] == "tv",
        json!("tv"),
        video["color_range"].clone(),
    );
    let (num, den) = (
        i64::from(schedule.output_rate.num()),
        i64::from(schedule.output_rate.den()),
    );
    let rate = video["r_frame_rate"].as_str().and_then(parse_fraction);
    check(
        &mut checks,
        "frame_rate",
        rate.is_some_and(|(n, d)| n * den == num * d),
        json!(format!("{num}/{den}")),
        video["r_frame_rate"].clone(),
    );
    let frames = number(&video["nb_frames"]).map(|n| n as u64);
    check(
        &mut checks,
        "frame_count",
        frames == Some(schedule.frames),
        json!(schedule.frames),
        json!(frames),
    );
    let expected_seconds = schedule.duration_seconds();
    let video_seconds = number(&video["duration"]);
    let frame_seconds = 1.0 / schedule.output_rate.as_f64();
    check(
        &mut checks,
        "video_duration",
        video_seconds
            .is_some_and(|seconds| (seconds - expected_seconds).abs() <= frame_seconds * 1.5),
        json!(expected_seconds),
        json!(video_seconds),
    );

    let audio = find("audio");
    check(
        &mut checks,
        "audio_stream",
        audio.is_some() == expect_audio,
        json!(expect_audio),
        json!(audio.is_some()),
    );
    if let (Some(audio), true) = (audio, expect_audio) {
        check(
            &mut checks,
            "audio_codec",
            audio["codec_name"] == "aac",
            json!("aac"),
            audio["codec_name"].clone(),
        );
        check(
            &mut checks,
            "audio_sample_rate",
            number(&audio["sample_rate"]) == Some(f64::from(MIX_SAMPLE_RATE)),
            json!(MIX_SAMPLE_RATE),
            audio["sample_rate"].clone(),
        );
        check(
            &mut checks,
            "audio_channels",
            audio["channels"] == 2,
            json!(2),
            audio["channels"].clone(),
        );
        let audio_seconds = number(&audio["duration"]);
        // AAC frames are 1024 samples and the encoder pads the tail: two frames of slack.
        check(
            &mut checks,
            "av_duration_match",
            audio_seconds.is_some_and(|seconds| {
                (seconds - expected_seconds).abs()
                    <= 2.0 * 1024.0 / f64::from(MIX_SAMPLE_RATE) + 0.01
            }),
            json!(expected_seconds),
            json!(audio_seconds),
        );
        let (video_start, audio_start) = (
            number(&video["start_time"]).unwrap_or(0.0),
            number(&audio["start_time"]).unwrap_or(0.0),
        );
        check(
            &mut checks,
            "av_start_offset",
            (video_start - audio_start).abs() <= 0.05,
            json!(0.0),
            json!(audio_start - video_start),
        );
    }

    let ok = checks.iter().all(|entry| entry["ok"] == true);
    if !ok {
        return Err(failed(checks));
    }
    Ok(
        json!({ "ok": true, "checks": checks, "format": probe["format"]["format_name"],
        "size": probe["format"]["size"], "bit_rate": probe["format"]["bit_rate"] }),
    )
}

fn failed(checks: Vec<Value>) -> JobError {
    let first = checks
        .iter()
        .find(|entry| entry["ok"] == false)
        .map(|entry| entry["name"].as_str().unwrap_or("check").to_string());
    JobError::new(
        "verification_failed",
        format!(
            "The exported file does not match the request ({})",
            first.unwrap_or_else(|| "unknown".into())
        ),
    )
    .with_details(json!({ "checks": checks }))
}

/// Runs the whole export on a snapshot of `state`. The renderer must be dedicated to this
/// export: it is driven frame by frame and shares nothing with the live monitor.
pub fn run(
    state: &EditorState,
    request: &ExportRequest,
    tools: &Tools,
    renderer: &mut dyn Renderer,
    ctx: &JobContext,
) -> Result<Value, JobError> {
    let started = Instant::now();
    let capabilities = renderer.capabilities().ok();
    let report = preflight(state, request, tools, capabilities.as_ref());
    if report["ok"] != true {
        let first = report["blockers"][0]["message"]
            .as_str()
            .unwrap_or("Export blocked")
            .to_string();
        let code = match report["blockers"][0]["code"].as_str() {
            Some("disk_full") => "disk_full",
            Some("destination_exists") => "destination_exists",
            _ => "preflight_failed",
        };
        return Err(JobError::new(code, first).with_details(report));
    }
    let schedule = schedule(state, request)?;
    let mut snapshot = state.clone();

    let assets = used_assets(&snapshot, schedule.from, schedule.to);
    let preparing: Vec<_> = assets
        .iter()
        .filter(|(_, kind, asset)| needs_decoding(*kind, asset))
        .collect();
    if !preparing.is_empty() {
        let workspace = state
            .project
            .workspace
            .clone()
            .ok_or_else(|| JobError::new("preflight_failed", "The project has no workspace"))?;
        let cache = Cache::new(&workspace);
        ctx.stage("prepare", preparing.len() as u64);
        for (id, kind, asset) in preparing {
            ctx.check()?;
            let decoded = cache.prepare(tools, asset, *kind, ctx)?;
            snapshot.set_decoded(id, decoded);
            ctx.advance(1);
        }
    }

    let parent = request
        .destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let staging = Staging::create(parent, "export")?;
    let staged = staging.path().join("export.mp4");

    let has_audio = request.preset.audio && has_audio_clips(&snapshot, schedule.from, schedule.to);
    let audio_path = staging.path().join("audio.wav");
    let audio_report = if has_audio {
        Some(render_audio(&snapshot, &schedule, &audio_path, ctx)?)
    } else {
        None
    };

    let job = Encode {
        tools,
        snapshot: &snapshot,
        request,
        schedule: &schedule,
        audio: audio_report.as_ref().map(|_| audio_path.as_path()),
        staging: staging.path(),
        output: &staged,
    };
    let metrics = render_and_encode(&job, renderer, ctx)
        .map_err(|error| as_disk_full(error, staging.path()))?;
    ctx.check()?;
    let verification = verify(tools, &staged, request, &schedule, has_audio)?;
    let size = fs::metadata(&staged)
        .map_err(|error| io_error(&staged, &error))?
        .len();
    fsx::publish_file(&staged, &request.destination, request.replace)?;

    let elapsed = started.elapsed().as_secs_f64();
    Ok(json!({
        "destination": request.destination,
        "preset": request.preset.id,
        "bytes": size,
        "frames": schedule.frames,
        "fps": schedule.output_rate.to_json(),
        "duration_seconds": schedule.duration_seconds(),
        "render": { "width": schedule.geometry.render.0, "height": schedule.geometry.render.1 },
        "output": { "width": request.preset.width, "height": request.preset.height },
        "audio": audio_report.as_ref().map(|audio| json!({
            "samples": audio.samples, "clipped_samples": audio.clipped, "peak": audio.peak, "rms": audio.rms })),
        "warnings": report["warnings"],
        "verification": verification,
        "metrics": {
            "elapsed_seconds": elapsed,
            "frames_per_second": schedule.frames as f64 / elapsed.max(f64::EPSILON),
            "render_ms_mean": metrics.render_ms / schedule.frames as f64,
            "render_ms_slowest": metrics.slowest_ms,
            "pipe_ms_total": metrics.pipe_ms,
            "host_peak_rss_mib": metrics.host_rss_kib / 1024,
            "ffmpeg_peak_rss_mib": metrics.ffmpeg_rss_kib / 1024,
            "renderer": capabilities.map(|caps| caps.renderer),
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tessel_editor_core::preset::builtins;

    fn preset(id: &str) -> Preset {
        builtins()
            .into_iter()
            .find(|preset| preset.id == id)
            .unwrap()
    }

    #[test]
    fn filter_adds_even_bars_for_contain_and_crops_for_cover() {
        let vertical = preset("vertical").geometry(1920, 1080);
        let chain = filter_chain(&vertical);
        assert!(chain.contains("pad=1080:1920:0:656:color=black"), "{chain}");
        assert!(!chain.contains("crop"), "{chain}");
        let cover = Preset {
            fit: Fit::Cover,
            ..preset("vertical")
        }
        .geometry(1920, 1080);
        let chain = filter_chain(&cover);
        assert!(chain.contains("crop=1080:1920:1166:0"), "{chain}");
        assert!(!chain.contains("pad"), "{chain}");
        let direct = preset("fullhd").geometry(1920, 1080);
        assert!(!filter_chain(&direct).contains("pad") && !filter_chain(&direct).contains("crop"));
        assert!(filter_chain(&direct).contains("out_color_matrix=bt709:out_range=tv"));
    }

    #[test]
    fn png_header_gives_the_size_and_rejects_other_bytes() {
        let mut header = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        header.extend_from_slice(&640u32.to_be_bytes());
        header.extend_from_slice(&360u32.to_be_bytes());
        assert_eq!(png_size(&header), Some((640, 360)));
        assert_eq!(png_size(b"not a png at all, just text.."), None);
    }
}
