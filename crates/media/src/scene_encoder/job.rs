//! The export itself: frozen scene + frozen resources → staged MP4 → verified → published.
//!
//! ```text
//! preflight ─▶ staging ─▶ byte snapshot ─▶ [ plan(i) ─▶ renderer ─▶ PNG ─▶ ffmpeg pipe ]×frames ─▶ ffprobe verify ─▶ publish
//! ```
//!
//! Everything happens in a staging directory on the destination volume, the private byte copy of
//! the approved resources included (BL109): the renderer reads that copy, never the originals. A failure, a cancel or a
//! verification mismatch drops the directory with the encoder's output in it, so the destination
//! either holds the finished, verified file or what it held before. Memory is bounded: one PNG
//! crosses the pipe at a time.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tessel_editor_core::CommandError;
use tessel_editor_core::jobs::{JobContext, JobError};
use tessel_editor_core::scene_export::{DirectedAudioMix, SceneExportSnapshot};
use tessel_editor_core::scene_resources::ResourceKey;

use crate::export::{self, ExportRequest, Schedule};
use crate::fsx::{self, Staging, hex, io_error};
use crate::scene_resources::{
    ApprovedResource, ResolveError, ResourceResolver, ResourceSnapshot, SceneResourceSession,
    snapshot_bytes,
};
use crate::tools::Tools;

use super::args::encode_args_with_audio;
use super::audio::{self, InspectError};
use super::render::{FrameJob, FrozenResources, SceneFrameRenderer};

const SPACE_MARGIN: u64 = 16 << 20;
/// Exports up to this many frames report every frame hash (to compare with `scene.render` frame by
/// frame); longer ones report only the first, the last and the digest, so a result stays small.
pub const LISTED_HASHES: u64 = 64;

/// One export, everything it will read and write, frozen when the request was accepted.
#[derive(Debug, Clone)]
pub struct SceneExport {
    pub snapshot: SceneExportSnapshot,
    pub resources: FrozenResources,
    pub destination: PathBuf,
    pub replace: bool,
}

fn resolve_error(error: &ResolveError) -> CommandError {
    CommandError::new(error.code(), error.to_string())
}

impl SceneExport {
    /// Freezes the approved resources of the snapshot's scene next to the snapshot. Refused
    /// before any job exists (nothing started, nothing written) when the scene has no approved
    /// map, the map belongs to another revision of the scene, or it cannot answer an asset the
    /// range draws. Callers run `scene.resources.resolve` first; this never hashes or loads.
    pub fn freeze(
        snapshot: SceneExportSnapshot,
        session: &SceneResourceSession,
        destination: PathBuf,
        replace: bool,
    ) -> Result<Self, CommandError> {
        let scene = snapshot.scene_id();
        let map = session.get(scene).ok_or_else(|| {
            resolve_error(&ResolveError::SceneNotLoaded {
                scene: scene.into(),
            })
        })?;
        let current = session.generation();
        if map.generation != current {
            return Err(resolve_error(&ResolveError::Stale {
                scene: scene.into(),
                held: map.generation,
                current,
            }));
        }
        let revision = &snapshot.scene().bundle().revision_id;
        if &map.bundle_revision != revision {
            return Err(CommandError::new(
                "resources_stale",
                format!(
                    "The resources approved for scene `{scene}` belong to revision `{}` but the scene is `{revision}`: run `scene.resources.resolve` again",
                    map.bundle_revision
                ),
            )
            .target(scene));
        }
        let resources = FrozenResources::new(map);
        for asset in snapshot.required_assets() {
            resources
                .resolve_asset(scene, asset)
                .map_err(|error| resolve_error(&error).target(asset.as_str()))?;
        }
        if let Some(mix) = snapshot.audio().mix() {
            let approved = resources
                .resolve(scene, &ResourceKey::AudioMix)
                .map_err(|error| resolve_error(&error).target("audio-mix"))?;
            // Only the header, and only to refuse what no mix could ever use (not a WAV, 24-bit,
            // surround, 96 kHz, too short for the range) before a job exists. A file that cannot be
            // opened is the job's preflight to report, like a vanished image.
            if let Err(InspectError::Unsupported(error)) = audio::check_layout(mix, &approved.path)
            {
                return Err(error.target("audio-mix"));
            }
        }
        Ok(Self {
            snapshot,
            resources,
            destination,
            replace,
        })
    }
}

/// The approved mix file of an export that carries audio, as the frozen map names it (the
/// original). Only the preflight looks at it, and only with a stat and the header: the mix itself
/// reads the private copy.
fn audio_resource(export: &SceneExport) -> Result<ApprovedResource, ResolveError> {
    export
        .resources
        .resolve(export.snapshot.scene_id(), &ResourceKey::AudioMix)
}

/// What the export reads: every asset its plans draw and, when the file carries audio, the
/// declared mix, as resource keys. The private byte copy (BL109) is taken for exactly these, so
/// the picture and the sound are frozen by the same mechanism.
fn required_keys(snapshot: &SceneExportSnapshot) -> Vec<ResourceKey> {
    let mut keys: Vec<ResourceKey> = snapshot
        .required_assets()
        .iter()
        .map(|asset| ResourceKey::asset(asset))
        .collect();
    if snapshot.audio().mix().is_some() {
        keys.push(ResourceKey::AudioMix);
    }
    keys
}

fn blocker(code: &str, message: impl Into<String>) -> Value {
    json!({ "code": code, "message": message.into() })
}

/// Same Bits-per-pixel model as the project export (x264 at CRF 18 ≈ 0.08, halving every +6).
fn estimated_video_bytes(export: &SceneExport) -> u64 {
    let preset = export.snapshot.preset();
    let bits_per_pixel = 0.08 * 2f64.powf((18.0 - f64::from(preset.crf)) / 6.0);
    let pixels = f64::from(preset.width) * f64::from(preset.height);
    (pixels * bits_per_pixel * export.snapshot.schedule().frames as f64 / 8.0) as u64
}

/// What the export would do and what stops it, without touching the destination. Used by `run`
/// and callable on its own. `max_size` is what the renderer reported (`None` skips that check).
pub fn preflight(export: &SceneExport, tools: &Tools, max_size: Option<u32>) -> Value {
    let mut blockers = Vec::new();
    let snapshot = &export.snapshot;
    let schedule = snapshot.schedule();

    if tools.h264_encoder() != Some("libx264") {
        blockers.push(blocker(
            "encoder_unavailable",
            format!("This FFmpeg ({}) has no libx264 encoder", tools.version),
        ));
    }
    if !tools.has_muxer("mp4") || !tools.has_decoder("png") {
        blockers.push(blocker(
            "encoder_unavailable",
            "This FFmpeg lacks the mp4 muxer or the png decoder",
        ));
    }
    let extension_ok = export
        .destination
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"));
    if !extension_ok {
        blockers.push(blocker(
            "invalid_destination",
            "The destination must be an `.mp4` file",
        ));
    }
    if export.destination.exists() && !export.replace {
        blockers.push(blocker(
            "destination_exists",
            format!("`{}` already exists", export.destination.display()),
        ));
    }
    if let Some(parent) = export.destination.parent().filter(|parent| parent.exists())
        && !parent.is_dir()
    {
        blockers.push(blocker(
            "invalid_destination",
            format!("`{}` is not a folder", parent.display()),
        ));
    }
    let (render_w, render_h) = schedule.geometry.render;
    if let Some(max) = max_size
        && render_w.max(render_h) > max
    {
        blockers.push(blocker(
            "render_too_large",
            format!(
                "The renderer draws at most {max}px per side; this export needs {render_w}x{render_h}"
            ),
        ));
    }
    for asset in snapshot.required_assets() {
        match export.resources.resolve_asset(snapshot.scene_id(), asset) {
            Err(error) => {
                blockers.push(blocker("resource_unresolved", error.to_string()).with_asset(asset))
            }
            // A stat, not a hash: the bytes were verified when the map was approved. A file that
            // vanished or changed size since then is caught here instead of at frame N.
            Ok(approved) => match fs::metadata(&approved.path) {
                Ok(meta) if meta.is_file() && meta.len() == approved.bytes => {}
                Ok(_) => blockers.push(
                    blocker(
                        "resource_changed",
                        format!("The file of `{asset}` is not the one that was approved"),
                    )
                    .with_asset(asset),
                ),
                Err(error) => blockers.push(
                    blocker(
                        "resource_missing",
                        format!("The file of `{asset}` cannot be read: {error}"),
                    )
                    .with_asset(asset),
                ),
            },
        }
    }
    let mut audio_bytes = 0;
    if let Some(mix) = snapshot.audio().mix() {
        audio_blockers(export, mix, tools, &mut blockers);
        // The staged 48 kHz stereo WAV plus its AAC encoding.
        audio_bytes = mix.output_samples() * 4 + audio_aac_bytes(export, mix);
    }
    let frame_bytes = u64::from(render_w) * u64::from(render_h) * 4;
    // The private copy of the approved files (the mix included) lives in the same staging as the
    // video and the mixed stream (BL109): all of it must fit before anything is written.
    let copy_bytes = snapshot_bytes(
        &export.resources,
        snapshot.scene_id(),
        &required_keys(snapshot),
    )
    .unwrap_or(0);
    let needed = estimated_video_bytes(export) + frame_bytes + audio_bytes + copy_bytes;
    let free = export.destination.parent().and_then(fsx::free_bytes);
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
    json!({
        "ok": blockers.is_empty(), "blockers": blockers, "warnings": snapshot.warnings(),
        "summary": snapshot.summary(),
        "estimate": { "output_bytes": estimated_video_bytes(export)
                + snapshot.audio().mix().map_or(0, |mix| audio_aac_bytes(export, mix)),
            "staging_bytes": needed, "resource_copy_bytes": copy_bytes,
            "audio_mix_bytes": audio_bytes,
            "note": "Estimates from typical content; the encoder decides the real size" },
        "free_bytes": free, "encoder": tools.h264_encoder(), "ffmpeg": tools.version,
        "destination": export.destination,
    })
}

fn audio_aac_bytes(export: &SceneExport, mix: &DirectedAudioMix) -> u64 {
    let seconds =
        mix.output_samples() as f64 / f64::from(tessel_editor_core::scene_export::MIX_RATE);
    (seconds * f64::from(export.snapshot.preset().audio_kbps) * 125.0) as u64
}

/// What stops the mix: no AAC encoder, a file that vanished, changed size or cannot serve the range.
fn audio_blockers(
    export: &SceneExport,
    mix: &DirectedAudioMix,
    tools: &Tools,
    blockers: &mut Vec<Value>,
) {
    let audio = |value: Value| {
        let mut value = value;
        value["asset"] = json!("audio-mix");
        value
    };
    if !tools.has_encoder("aac") {
        blockers.push(blocker(
            "encoder_unavailable",
            format!(
                "This FFmpeg ({}) has no aac encoder for the audio mix",
                tools.version
            ),
        ));
    }
    let approved = match audio_resource(export) {
        Ok(approved) => approved,
        Err(error) => {
            blockers.push(audio(blocker("resource_unresolved", error.to_string())));
            return;
        }
    };
    match fs::metadata(&approved.path) {
        Ok(meta) if meta.is_file() && meta.len() == approved.bytes => {}
        Ok(_) => {
            blockers.push(audio(blocker(
                "resource_changed",
                "The audio mix is not the file that was approved",
            )));
            return;
        }
        Err(error) => {
            blockers.push(audio(blocker(
                "resource_missing",
                format!("The audio mix cannot be read: {error}"),
            )));
            return;
        }
    }
    if let Err(error) = audio::check_layout(mix, &approved.path) {
        blockers.push(audio(blocker(error.code(), error.message())));
    }
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
    let code = if tail.to_lowercase().contains("no space left") {
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
}

/// What the frame loop reports besides success: the hashes the runtime computed.
struct Drawn {
    metrics: Metrics,
    first: Option<String>,
    last: Option<String>,
    /// SHA-256 over every frame hash in order: a long export stays one short field in the result.
    digest: String,
    /// Every frame hash, only while the export is short enough ([`LISTED_HASHES`]).
    listed: Vec<String>,
}

fn frame_loop(
    export: &SceneExport,
    resources: &dyn ResourceResolver,
    renderer: &mut dyn SceneFrameRenderer,
    child: &mut Child,
    frame_path: &Path,
    log: &Path,
    ctx: &JobContext,
) -> Result<Drawn, JobError> {
    let snapshot = &export.snapshot;
    let schedule = snapshot.schedule();
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| JobError::new("encode_failed", "ffmpeg has no input pipe"))?;
    let (width, height) = schedule.geometry.render;
    let mut metrics = Metrics::default();
    let (mut first, mut last) = (None, None);
    let mut digest = Sha256::new();
    let mut listed = Vec::new();
    ctx.stage("render", schedule.frames);
    // One timeline per job (BL064): the per-shot setup is paid once, not once per frame.
    let timeline = snapshot.timeline();
    for index in 0..schedule.frames {
        ctx.check()?;
        let plan = snapshot.plan_with(&timeline, index).map_err(|error| {
            JobError::new(
                "render_failed",
                format!("Frame {index}: {} ({})", error.message, error.code),
            )
        })?;
        let started = Instant::now();
        let rendered = renderer
            .render(&FrameJob {
                scene_id: snapshot.scene_id(),
                plan: &plan,
                resources,
                generation: export.resources.map().generation.0,
                size: (width, height),
                png: frame_path,
            })
            .map_err(|error| {
                JobError::new(
                    if error.code == "disk_full" {
                        "disk_full"
                    } else {
                        "render_failed"
                    },
                    format!("Frame {index}: {} ({})", error.message, error.code),
                )
            })?;
        let bytes = fs::read(frame_path).map_err(|error| io_error(frame_path, &error))?;
        if png_size(&bytes) != Some((width, height)) {
            return Err(JobError::new(
                "render_failed",
                format!(
                    "Frame {index}: the renderer produced {:?}, expected {width}x{height}",
                    png_size(&bytes)
                ),
            ));
        }
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        let piped = Instant::now();
        if stdin.write_all(&bytes).is_err() {
            return Err(encode_error(
                log,
                "ffmpeg closed its input before the last frame",
            ));
        }
        metrics.pipe_ms += piped.elapsed().as_secs_f64() * 1000.0;
        metrics.render_ms += elapsed;
        metrics.slowest_ms = metrics.slowest_ms.max(elapsed);
        digest.update(rendered.hash.as_bytes());
        digest.update([0]);
        first.get_or_insert_with(|| rendered.hash.clone());
        if schedule.frames <= LISTED_HASHES {
            listed.push(rendered.hash.clone());
        }
        last = Some(rendered.hash);
        ctx.advance(1);
    }
    Ok(Drawn {
        metrics,
        first,
        last,
        digest: hex(&digest.finalize()),
        listed,
    })
}

/// Runs the whole export. `renderer` must be dedicated to it; it is driven frame by frame.
pub fn run(
    export: &SceneExport,
    tools: &Tools,
    renderer: &mut dyn SceneFrameRenderer,
    ctx: &JobContext,
) -> Result<Value, JobError> {
    let started = Instant::now();
    let snapshot = &export.snapshot;
    let schedule = snapshot.schedule();
    ctx.stage("preflight", 1);
    let max_size = renderer.max_size()?;
    ctx.check()?;
    let report = preflight(export, tools, max_size);
    if report["ok"] != true {
        let code = match report["blockers"][0]["code"].as_str() {
            Some("disk_full") => "disk_full",
            Some("destination_exists") => "destination_exists",
            _ => "preflight_failed",
        };
        let first = report["blockers"][0]["message"]
            .as_str()
            .unwrap_or("Export blocked")
            .to_string();
        return Err(JobError::new(code, first).with_details(report));
    }
    ctx.advance(1);

    let parent = export
        .destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let staging = Staging::create(parent, "scene-export")?;
    let staged = staging.path().join("export.mp4");
    let frame_path = staging.path().join("frame.png");
    let log = staging.path().join("ffmpeg.log");
    let log_file = File::create(&log).map_err(|error| io_error(&log, &error))?;
    // BL109: freeze the *bytes* the plans draw and the mix reads, not only the approved paths.
    // Everything below reads this private copy; the originals can change (or be swapped for
    // same-size content) without reaching a frame or a sample, and a swap before this point is
    // refused here by hash.
    let keys = required_keys(snapshot);
    let copy_total =
        snapshot_bytes(&export.resources, snapshot.scene_id(), &keys).map_err(JobError::from)?;
    ctx.stage("snapshot", copy_total);
    let frozen = ResourceSnapshot::take(
        &export.resources,
        snapshot.scene_id(),
        &keys,
        &staging.path().join("resources"),
        ctx.token(),
        |bytes| ctx.advance(bytes),
    )
    .map_err(|error| as_disk_full(error, staging.path()))?;

    // The mix is rendered next, from the private copy only: a swapped, unreadable or too-short
    // file fails here, before a single frame is drawn, and the encoder is handed a finished
    // stream exactly as long as the video.
    let mixed = match snapshot.audio().mix() {
        Some(mix) => {
            let approved = frozen
                .resolve(snapshot.scene_id(), &ResourceKey::AudioMix)
                .map_err(JobError::from)?;
            let wav = staging.path().join("mix.wav");
            let report = audio::render_private(mix, &approved, &wav, ctx)
                .map_err(|error| as_disk_full(error, staging.path()))?;
            frozen.verify_intact()?;
            Some((wav, report, approved))
        }
        None => None,
    };

    let mut child = Command::new(&tools.ffmpeg)
        .args(encode_args_with_audio(
            snapshot.preset(),
            &schedule.geometry,
            schedule.output_rate,
            mixed.as_ref().map(|(wav, ..)| wav.as_path()),
            &staged,
        ))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(log_file)
        .spawn()
        .map_err(|error| {
            JobError::new("tool_unavailable", format!("Cannot start ffmpeg: {error}"))
        })?;

    // The pipe closes when the loop's `stdin` drops, which is what lets ffmpeg finish.
    let drawn = frame_loop(
        export,
        &frozen,
        renderer,
        &mut child,
        &frame_path,
        &log,
        ctx,
    );
    let drawn = match drawn {
        Ok(drawn) => drawn,
        Err(error) => {
            kill(&mut child);
            return Err(as_disk_full(error, staging.path()));
        }
    };
    if let Err(error) = frozen.verify_intact() {
        kill(&mut child);
        return Err(error);
    }
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
        return Err(as_disk_full(
            encode_error(&log, "ffmpeg failed"),
            staging.path(),
        ));
    }
    ctx.check()?;

    let request = ExportRequest::new(snapshot.preset().clone(), &export.destination);
    let verification = export::verify(
        tools,
        &staged,
        &request,
        &Schedule {
            from: schedule.from,
            to: schedule.to(),
            sequence_rate: schedule.scene_rate,
            output_rate: schedule.output_rate,
            frames: schedule.frames,
            geometry: schedule.geometry,
            audio_samples: mixed.as_ref().map_or(0, |(_, report, _)| report.samples),
        },
        mixed.is_some(),
    )?;
    let size = fs::metadata(&staged)
        .map_err(|error| io_error(&staged, &error))?
        .len();
    fsx::publish_file(&staged, &export.destination, export.replace)?;

    let elapsed = started.elapsed().as_secs_f64();
    Ok(json!({
        "action": "scene.export",
        "destination": export.destination,
        "scene": snapshot.scene_id(),
        "preset": snapshot.preset().id,
        "range": { "from": schedule.from, "frames": schedule.edit_frames, "to": schedule.to() },
        "bytes": size,
        "frames": schedule.frames,
        "fps": schedule.output_rate.to_json(),
        "duration_seconds": schedule.duration_seconds(),
        "render": { "width": schedule.geometry.render.0, "height": schedule.geometry.render.1 },
        "output": { "width": snapshot.preset().width, "height": snapshot.preset().height },
        "audio": audio_result(snapshot, mixed.as_ref()),
        "resources": used_resources(export),
        "snapshot": { "files": frozen.files(), "bytes": frozen.bytes(), "verified": true },
        "warnings": report["warnings"],
        "verification": verification,
        "frame_hashes": { "count": schedule.frames, "first": drawn.first, "last": drawn.last,
            "digest": drawn.digest, "list": (!drawn.listed.is_empty()).then_some(drawn.listed) },
        "metrics": {
            "elapsed_seconds": elapsed,
            "frames_per_second": schedule.frames as f64 / elapsed.max(f64::EPSILON),
            "render_ms_mean": drawn.metrics.render_ms / schedule.frames as f64,
            "render_ms_slowest": drawn.metrics.slowest_ms,
            "pipe_ms_total": drawn.metrics.pipe_ms,
        },
    }))
}

/// `result.audio`: the plan (`included`, `declared`, `mix`) and, when a mix was written, what the
/// file it came from was and what was measured on the stream handed to the encoder.
fn audio_result(
    snapshot: &SceneExportSnapshot,
    mixed: Option<&(PathBuf, audio::MixReport, ApprovedResource)>,
) -> Value {
    let mut result = snapshot.audio().to_json();
    if let Some((_, report, approved)) = mixed {
        let extra = report.to_json(approved);
        result["source"] = extra["source"].clone();
        result["measured"] = extra["measured"].clone();
    }
    result
}

/// The resources the export drew from, as the frozen map identified them: what a reader needs to
/// tell which bytes a published file came from.
fn used_resources(export: &SceneExport) -> Vec<Value> {
    let snapshot = &export.snapshot;
    snapshot
        .required_assets()
        .iter()
        .filter_map(|asset| {
            let approved = export
                .resources
                .resolve_asset(snapshot.scene_id(), asset)
                .ok()?;
            Some(json!({ "asset": asset, "sha256": approved.content_hash,
                "revision": approved.revision_id, "bytes": approved.bytes }))
        })
        .collect()
}

/// Below this much free space, an encoder or renderer failure is a full disk whatever message the
/// tool printed (FFmpeg often exits with no text when the final write of the file fails).
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
