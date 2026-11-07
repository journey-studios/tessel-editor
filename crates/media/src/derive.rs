//! Derived media (P06): thumbnails, waveforms, preview proxies and the deterministic decoders.
//!
//! Everything lives in a disposable cache (`<workspace>/.tessel/cache/<hash>/…`) next to a
//! `cache.json` that records what it was built from. A derivative is reused only when the
//! source hash/size, the parameters, the FFmpeg version and the cache schema all still match;
//! otherwise it is rebuilt in staging and swapped in. The original is never modified, and a
//! proxy is only for preview: export always asks for the original (`Derived::for_export`).

use std::fs;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};
use tessel_editor_core::assets::{
    Asset, AudioStream, Decoded, DecodedAudio, DecodedFrames, MediaKind, ProxyFormat, ProxyFrames,
    Source, expand_pattern,
};
use tessel_editor_core::jobs::{JobContext, JobError};

use crate::fsx::{self, Staging, directory_size, io_error};
use crate::tools::Tools;

/// Bump when the layout or the way a derivative is produced changes.
const CACHE_VERSION: u32 = 2;
/// Sample rate of the waveform analysis: peaks need no more than this.
const WAVEFORM_RATE: u32 = 8000;

pub struct Cache {
    pub root: PathBuf,
}

fn tool_failed(what: &str, stderr: &[u8]) -> JobError {
    let text = String::from_utf8_lossy(stderr);
    JobError::new(
        "decode_failed",
        format!(
            "{what}: {}",
            text.lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("ffmpeg failed")
        ),
    )
}

/// Width and height from the IHDR chunk of a PNG, without decoding it.
fn png_size(path: &Path) -> Result<(u32, u32), JobError> {
    let mut header = [0u8; 24];
    fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .map_err(|error| io_error(path, &error))?;
    if header[..8] != [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a] {
        return Err(JobError::new(
            "decode_failed",
            format!("`{}` is not a PNG frame", path.display()),
        ));
    }
    let word = |at: usize| {
        u32::from_be_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
    };
    Ok((word(16), word(20)))
}

fn file_path(asset: &Asset) -> Result<&str, JobError> {
    match &asset.source {
        Source::File { path } => Ok(path),
        _ => Err(JobError::new(
            "unsupported_format",
            "This asset has no file to derive from",
        )),
    }
}

/// Everything a cached derivative depends on, serialised: equality means "still valid".
fn fingerprint(tools: &Tools, asset: &Asset, params: &Value) -> Value {
    json!({ "cache": CACHE_VERSION, "ffmpeg": tools.version, "hash": asset.identity.hash,
        "size": asset.identity.size, "params": params })
}

impl Cache {
    pub fn new(workspace: &Path) -> Self {
        Self {
            root: workspace.join(".tessel").join("cache"),
        }
    }

    fn entry_dir(&self, asset: &Asset, name: &str) -> Result<PathBuf, JobError> {
        let hash = asset.identity.hash.as_deref().ok_or_else(|| {
            JobError::new(
                "integrity_error",
                "The asset was never hashed (import it first)",
            )
        })?;
        Ok(self.root.join(hash).join(name))
    }

    fn valid(dir: &Path, expected: &Value) -> bool {
        fs::read(dir.join("cache.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .is_some_and(|stored| stored["fingerprint"] == *expected)
    }

    /// Builds `name` unless a valid one exists. `build` fills the staging directory and returns
    /// extra manifest data.
    fn ensure(
        &self,
        asset: &Asset,
        name: &str,
        fingerprint: Value,
        build: impl FnOnce(&Path) -> Result<Value, JobError>,
    ) -> Result<(PathBuf, bool, Value), JobError> {
        let dir = self.entry_dir(asset, name)?;
        if Self::valid(&dir, &fingerprint) {
            let extra = fs::read(dir.join("cache.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .map_or(Value::Null, |stored| stored["extra"].clone());
            return Ok((dir, true, extra));
        }
        let parent = dir.parent().unwrap_or(&self.root);
        let staging = Staging::create(parent, "derive")?;
        let extra = build(staging.path())?;
        let manifest = json!({ "fingerprint": fingerprint, "extra": extra });
        fs::write(
            staging.path().join("cache.json"),
            serde_json::to_vec_pretty(&manifest).unwrap_or_default(),
        )
        .map_err(|error| io_error(&dir, &error))?;
        // A stale derivative is replaced only now that the new one is complete.
        if dir.exists() {
            fs::remove_dir_all(&dir).map_err(|error| io_error(&dir, &error))?;
        }
        staging.publish(&dir)?;
        Ok((dir, false, extra))
    }

    /// PNG thumbnail `width` pixels wide. Stills use the image, videos the frame at `at_seconds`.
    pub fn thumbnail(
        &self,
        tools: &Tools,
        asset: &Asset,
        kind: MediaKind,
        width: u32,
        at_seconds: f64,
    ) -> Result<PathBuf, JobError> {
        let source = file_path(asset)?.to_string();
        let params = json!({ "width": width, "at": at_seconds, "kind": kind.label() });
        let (dir, _, _) = self.ensure(
            asset,
            &format!("thumb-{width}"),
            fingerprint(tools, asset, &params),
            |staging| {
                let out = staging.join("thumb.png");
                let mut command = Command::new(&tools.ffmpeg);
                command.args(["-v", "error", "-nostdin", "-y"]);
                if kind == MediaKind::Video && at_seconds > 0.0 {
                    command.args(["-ss", &format!("{at_seconds:.3}")]);
                }
                let output = command
                    .arg("-i")
                    .arg(&source)
                    .args([
                        "-frames:v",
                        "1",
                        "-an",
                        "-vf",
                        &format!("scale={width}:-2:flags=bicubic,format=rgba"),
                    ])
                    .arg(&out)
                    .output()
                    .map_err(|error| {
                        JobError::new("tool_unavailable", format!("ffmpeg: {error}"))
                    })?;
                if !output.status.success() || !out.is_file() {
                    return Err(tool_failed("thumbnail", &output.stderr));
                }
                Ok(Value::Null)
            },
        )?;
        Ok(dir.join("thumb.png"))
    }

    /// A PNG picture of `asset` identified by `identity` (BL144: splat and scene thumbnails),
    /// drawn by `render` into staging unless the cache already holds that exact picture. The
    /// entry name carries a digest of the identity, so a different picture never reuses a path
    /// (a window's image cache is keyed by path). Returns the PNG and whether the cache had it.
    pub fn picture(
        &self,
        asset: &Asset,
        identity: &str,
        size: (u32, u32),
        render: impl FnOnce(&Path) -> Result<(), JobError>,
    ) -> Result<(PathBuf, bool), JobError> {
        let name = format!(
            "picture-{}x{}-{:08x}",
            size.0,
            size.1,
            crc32fast::hash(identity.as_bytes())
        );
        let fingerprint = json!({ "cache": CACHE_VERSION, "identity": identity });
        let dir = self.entry_dir(asset, &name)?;
        if Self::valid(&dir, &fingerprint) && dir.join("thumb.png").is_file() {
            return Ok((dir.join("thumb.png"), true));
        }
        let (dir, _, _) = self.ensure(asset, &name, fingerprint, |staging| {
            let out = staging.join("thumb.png");
            render(&out)?;
            if !out.is_file() {
                return Err(JobError::new(
                    "render_failed",
                    "The renderer reported success but wrote no picture",
                ));
            }
            let got = png_size(&out)?;
            if got != size {
                return Err(JobError::new(
                    "render_failed",
                    format!(
                        "The renderer drew {}x{}, the thumbnail needs {}x{}",
                        got.0, got.1, size.0, size.1
                    ),
                ));
            }
            Ok(Value::Null)
        })?;
        Ok((dir.join("thumb.png"), false))
    }

    /// Peaks (0..1) of the first audio stream in `buckets` equal slices of its duration.
    pub fn waveform(
        &self,
        tools: &Tools,
        asset: &Asset,
        buckets: usize,
        ctx: &JobContext,
    ) -> Result<Vec<f32>, JobError> {
        let source = file_path(asset)?.to_string();
        let duration = asset
            .meta
            .duration
            .map(|d| d.to_f64())
            .filter(|d| *d > 0.0)
            .ok_or_else(|| JobError::new("unsupported_format", "The asset has no duration"))?;
        let params = json!({ "buckets": buckets });
        let (_, _, peaks) = self.ensure(
            asset,
            &format!("wave-{buckets}"),
            fingerprint(tools, asset, &params),
            |_| {
                let total = (duration * f64::from(WAVEFORM_RATE)).ceil() as u64;
                let mut child = Command::new(&tools.ffmpeg)
                    .args(["-v", "error", "-nostdin", "-i"])
                    .arg(&source)
                    .args([
                        "-vn",
                        "-ac",
                        "1",
                        "-ar",
                        &WAVEFORM_RATE.to_string(),
                        "-f",
                        "s16le",
                        "-",
                    ])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|error| {
                        JobError::new("tool_unavailable", format!("ffmpeg: {error}"))
                    })?;
                let mut peaks = vec![0f32; buckets];
                let mut stdout = BufReader::new(
                    child
                        .stdout
                        .take()
                        .ok_or_else(|| JobError::new("io_error", "no pipe"))?,
                );
                let mut chunk = vec![0u8; 1 << 16];
                let (mut index, mut carry) = (0u64, None::<u8>);
                loop {
                    if ctx.is_cancelled() {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(JobError::cancelled());
                    }
                    let read = stdout
                        .read(&mut chunk)
                        .map_err(|error| io_error(Path::new(&source), &error))?;
                    if read == 0 {
                        break;
                    }
                    let mut bytes = Vec::with_capacity(read + 1);
                    bytes.extend(carry.take());
                    bytes.extend_from_slice(&chunk[..read]);
                    if bytes.len() % 2 == 1 {
                        carry = bytes.pop();
                    }
                    for pair in bytes.as_chunks::<2>().0 {
                        let sample =
                            f32::from(i16::from_le_bytes([pair[0], pair[1]])).abs() / 32768.0;
                        let bucket = ((index * buckets as u64) / total.max(1))
                            .min(buckets as u64 - 1) as usize;
                        peaks[bucket] = peaks[bucket].max(sample);
                        index += 1;
                    }
                }
                let status = child
                    .wait()
                    .map_err(|error| io_error(Path::new(&source), &error))?;
                if !status.success() {
                    return Err(JobError::new(
                        "decode_failed",
                        "ffmpeg could not decode the audio stream",
                    ));
                }
                Ok(json!(peaks))
            },
        )?;
        Ok(peaks.as_array().map_or_else(Vec::new, |values| {
            values
                .iter()
                .filter_map(Value::as_f64)
                .map(|v| v as f32)
                .collect()
        }))
    }

    /// Low-resolution H.264 copy for smooth preview. It keeps the source frame rate and
    /// duration (checked by the tests), is marked `preview_only` and is never an export input.
    pub fn proxy(
        &self,
        tools: &Tools,
        asset: &Asset,
        height: u32,
        ctx: &JobContext,
    ) -> Result<PathBuf, JobError> {
        let source = file_path(asset)?.to_string();
        let rate = asset
            .meta
            .rate
            .ok_or_else(|| JobError::new("unsupported_format", "Only video has a proxy"))?;
        let encoder = tools
            .h264_encoder()
            .ok_or_else(|| JobError::new("tool_unavailable", "No H.264 encoder is available"))?;
        let params = json!({ "height": height, "encoder": encoder, "rate": rate.to_json() });
        let (dir, _, _) = self.ensure(
            asset,
            &format!("proxy-{height}"),
            fingerprint(tools, asset, &params),
            |staging| {
                let out = staging.join("proxy.mp4");
                let mut child = Command::new(&tools.ffmpeg)
                    .args(["-v", "error", "-nostdin", "-y", "-i"])
                    .arg(&source)
                    .args([
                        "-an",
                        "-vf",
                        &format!("fps={}/{},scale=-2:{height}", rate.num(), rate.den()),
                    ])
                    .args(["-c:v", encoder, "-pix_fmt", "yuv420p"])
                    .args(if encoder == "libx264" {
                        vec!["-preset", "veryfast", "-crf", "28"]
                    } else {
                        vec!["-b:v", "2M"]
                    })
                    .arg(&out)
                    .stderr(Stdio::piped())
                    .spawn()
                    .map_err(|error| {
                        JobError::new("tool_unavailable", format!("ffmpeg: {error}"))
                    })?;
                wait_cancellable(&mut child, ctx, "proxy")?;
                Ok(json!({ "preview_only": true }))
            },
        )?;
        Ok(dir.join("proxy.mp4"))
    }

    /// Deterministic frame cache of a container video: constant-rate PNGs, always the same pixels
    /// for the same frame whatever engine shows them. Returns the frames and whether the cache
    /// was reused.
    pub fn decode_video(
        &self,
        tools: &Tools,
        asset: &Asset,
        ctx: &JobContext,
    ) -> Result<(DecodedFrames, bool), JobError> {
        let source = file_path(asset)?.to_string();
        let rate = asset
            .meta
            .rate
            .ok_or_else(|| JobError::new("unsupported_format", "Only video has frames"))?;
        let (width, height) = (
            asset.meta.width.unwrap_or(0),
            asset.meta.height.unwrap_or(0),
        );
        // Untagged HD is Rec.709, untagged SD Rec.601: swscale's default is 601 for both, which
        // shifts HD colours, so the matrix is stated explicitly.
        let tagged = asset
            .identity
            .probe
            .as_ref()
            .and_then(|probe| probe["color"]["space"].as_str())
            .unwrap_or("unknown");
        let matrix = match tagged {
            "bt709" => "bt709",
            "bt470bg" | "smpte170m" => "smpte170m",
            "bt2020nc" | "bt2020c" => "bt2020",
            _ if height >= 720 || width >= 1280 => "bt709",
            _ => "smpte170m",
        };
        // `vfr` is part of the key: the same file conformed under another policy is another cache.
        let params = json!({ "rate": rate.to_json(), "matrix": matrix, "format": "png-rgba",
            "vfr": asset.meta.vfr_conformed });
        let frames_estimate = asset
            .meta
            .duration
            .map_or(0, |d| (d.to_f64() * rate.as_f64()).ceil() as u64);
        let (dir, reused, extra) = self.ensure(asset, "frames", fingerprint(tools, asset, &params), |staging| {
            let bytes = frames_estimate.saturating_mul(u64::from(width) * u64::from(height) * 2);
            fsx::ensure_space(&self.root, bytes, 256 << 20)?;
            let frames = staging.join("frames");
            fs::create_dir_all(&frames).map_err(|error| io_error(&frames, &error))?;
            let filter = format!(
                "fps={}/{}:round=near,scale=in_color_matrix={matrix}:out_color_matrix=bt709,format=rgba",
                rate.num(),
                rate.den()
            );
            let mut child = Command::new(&tools.ffmpeg)
                .args(["-v", "error", "-nostdin", "-y", "-i"])
                .arg(&source)
                .args(["-an", "-vf", &filter, "-fps_mode", "passthrough", "-compression_level", "1", "-start_number", "0"])
                .arg(frames.join("f%06d.png"))
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|error| JobError::new("tool_unavailable", format!("ffmpeg: {error}")))?;
            wait_cancellable(&mut child, ctx, "decode")?;
            let count = fs::read_dir(&frames).map_or(0, |entries| entries.filter_map(Result::ok).count());
            if count == 0 {
                return Err(JobError::new("decode_failed", "The video produced no frames"));
            }
            Ok(json!({ "count": count }))
        })?;
        let count = extra["count"].as_i64().unwrap_or(0);
        Ok((
            DecodedFrames {
                pattern: dir
                    .join("frames")
                    .join("f%06d.png")
                    .to_string_lossy()
                    .into_owned(),
                first: 0,
                count,
            },
            reused,
        ))
    }

    /// Smaller numbered frames derived from the decoded `frames`, for interactive preview only
    /// (export always reads the original, see `Derived::for_export`). Same numbering and count as
    /// the source, scaled to exactly `height` rows; a frame count that differs is an error, not
    /// a shorter proxy that would drift against the timeline. Returns the proxy and whether the
    /// cache was reused.
    pub fn preview_frames(
        &self,
        tools: &Tools,
        asset: &Asset,
        frames: &DecodedFrames,
        height: u32,
        format: ProxyFormat,
        ctx: &JobContext,
    ) -> Result<(ProxyFrames, bool), JobError> {
        let first = expand_pattern(&frames.pattern, frames.first)
            .ok_or_else(|| JobError::new("integrity_error", "The frame pattern is not numbered"))?;
        // The decoded frames are the truth about the picture size (a rotated video swaps the
        // probed width and height), so the proxy is sized from them.
        let (source_width, source_height) = png_size(Path::new(&first))?;
        if source_height <= height {
            return Err(JobError::new(
                "unsupported_format",
                format!("The video is {source_height} rows tall: no smaller proxy to derive"),
            ));
        }
        // Even width: the encoders behind JPEG and the runtime's texture upload both prefer it,
        // and the quad is sized from the original, so the odd pixel never shows.
        let width = (((u64::from(source_width) * u64::from(height) + u64::from(source_height) / 2)
            / u64::from(source_height)) as u32
            & !1)
            .max(2);
        let params = json!({ "height": height, "width": width, "format": format.label(),
            "frames": frames.pattern, "first": frames.first, "count": frames.count, "quality": 3 });
        let name = format!("preview-{height}-{}", format.extension());
        let (dir, reused, _) = self.ensure(
            asset,
            &name,
            fingerprint(tools, asset, &params),
            |staging| {
                let bytes = (frames.count.max(0) as u64)
                    .saturating_mul(u64::from(width) * u64::from(height) * 3);
                fsx::ensure_space(&self.root, bytes, 256 << 20)?;
                let out = staging.join("frames");
                fs::create_dir_all(&out).map_err(|error| io_error(&out, &error))?;
                let pixel = match format {
                    ProxyFormat::Jpeg => "yuvj420p",
                    ProxyFormat::Png => "rgba",
                };
                let mut command = Command::new(&tools.ffmpeg);
                command
                    .args(["-v", "error", "-nostdin", "-y", "-start_number"])
                    .arg(frames.first.to_string())
                    .arg("-i")
                    .arg(&frames.pattern)
                    .args(["-an", "-vf"])
                    .arg(format!(
                        "scale={width}:{height}:flags=bicubic,format={pixel}"
                    ))
                    .args(["-fps_mode", "passthrough", "-start_number"])
                    .arg(frames.first.to_string());
                match format {
                    ProxyFormat::Jpeg => command.args(["-q:v", "3"]),
                    ProxyFormat::Png => command.args(["-compression_level", "1"]),
                };
                let mut child = command
                    .arg(out.join(format!("p%06d.{}", format.extension())))
                    .stderr(Stdio::piped())
                    .spawn()
                    .map_err(|error| {
                        JobError::new("tool_unavailable", format!("ffmpeg: {error}"))
                    })?;
                wait_cancellable(&mut child, ctx, "preview proxy")?;
                let count =
                    fs::read_dir(&out).map_or(0, |entries| entries.filter_map(Result::ok).count());
                if count as i64 != frames.count {
                    return Err(JobError::new(
                        "decode_failed",
                        format!("The proxy has {count} frames, the video {}", frames.count),
                    ));
                }
                Ok(json!({ "count": count }))
            },
        )?;
        Ok((
            ProxyFrames {
                pattern: dir
                    .join("frames")
                    .join(format!("p%06d.{}", format.extension()))
                    .to_string_lossy()
                    .into_owned(),
                first: frames.first,
                count: frames.count,
                width,
                height,
                format,
                source_frames: frames.pattern.clone(),
            },
            reused,
        ))
    }

    /// WAV (16-bit, at the stream's own rate, at most stereo) of one audio stream for the mixer.
    /// Sample 0 is the stream's first sample, so nothing is trimmed or padded here: the stream's
    /// offset on the asset's clock (`AudioStream::offset`) is applied by the plan, in one place.
    pub fn decode_audio(
        &self,
        tools: &Tools,
        asset: &Asset,
        stream: AudioStream,
        ctx: &JobContext,
    ) -> Result<(String, bool), JobError> {
        let source = file_path(asset)?.to_string();
        let channels = stream.channels.clamp(1, 2);
        let params =
            json!({ "stream": stream.index, "rate": stream.sample_rate, "channels": channels });
        let (dir, reused, _) = self.ensure(
            asset,
            &format!("audio-{}", stream.index),
            fingerprint(tools, asset, &params),
            |staging| {
                fsx::ensure_space(
                    &self.root,
                    asset.identity.size.unwrap_or(0).saturating_mul(12),
                    64 << 20,
                )?;
                let mut child = Command::new(&tools.ffmpeg)
                    .args(["-v", "error", "-nostdin", "-y", "-i"])
                    .arg(&source)
                    .args(["-map", &format!("0:a:{}", stream.index), "-vn", "-ac"])
                    .arg(channels.to_string())
                    .args(["-ar"])
                    .arg(stream.sample_rate.to_string())
                    .args(["-c:a", "pcm_s16le"])
                    .arg(staging.join("audio.wav"))
                    .stderr(Stdio::piped())
                    .spawn()
                    .map_err(|error| {
                        JobError::new("tool_unavailable", format!("ffmpeg: {error}"))
                    })?;
                wait_cancellable(&mut child, ctx, "audio decode")?;
                Ok(Value::Null)
            },
        )?;
        Ok((dir.join("audio.wav").to_string_lossy().into_owned(), reused))
    }

    /// Every audio stream of `asset` decoded to a WAV, in stream order.
    fn decode_streams(
        &self,
        tools: &Tools,
        asset: &Asset,
        ctx: &JobContext,
    ) -> Result<Vec<DecodedAudio>, JobError> {
        let streams: Vec<AudioStream> = if asset.meta.audio_streams.is_empty() {
            asset.meta.audio_stream(0).into_iter().collect()
        } else {
            asset.meta.audio_streams.clone()
        };
        streams
            .into_iter()
            .map(|stream| {
                let (path, _) = self.decode_audio(tools, asset, stream, ctx)?;
                Ok(DecodedAudio {
                    stream: stream.index,
                    path,
                })
            })
            .collect()
    }

    /// Everything the renderer and the mixer need from `asset`, or `None` when the original is
    /// already directly usable (stills, WAV, splats, scenes). A video yields its picture frames
    /// and the sound it carries, so it never needs a second, manual step.
    pub fn prepare(
        &self,
        tools: &Tools,
        asset: &Asset,
        kind: MediaKind,
        ctx: &JobContext,
    ) -> Result<Option<Decoded>, JobError> {
        match kind {
            MediaKind::Video => {
                let (frames, _) = self.decode_video(tools, asset, ctx)?;
                Ok(Some(Decoded {
                    frames: Some(frames),
                    audio: self.decode_streams(tools, asset, ctx)?,
                    proxy: None,
                }))
            }
            MediaKind::Audio if asset.meta.format.as_deref() != Some("wav") => Ok(Some(Decoded {
                frames: None,
                audio: self.decode_streams(tools, asset, ctx)?,
                proxy: None,
            })),
            _ => Ok(None),
        }
    }

    pub fn size(&self) -> u64 {
        directory_size(&self.root)
    }

    /// Deletes the whole cache. It only ever holds derivatives; no project data lives here.
    pub fn clear(&self) -> Result<u64, JobError> {
        let freed = self.size();
        if self.root.exists() {
            fs::remove_dir_all(&self.root).map_err(|error| io_error(&self.root, &error))?;
        }
        Ok(freed)
    }

    /// Keeps the cache under `max_bytes` by removing the least recently modified assets first.
    pub fn prune(&self, max_bytes: u64) -> Result<u64, JobError> {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return Ok(0);
        };
        let mut dirs: Vec<(std::time::SystemTime, PathBuf, u64)> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_dir())
            .map(|entry| {
                let modified = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::UNIX_EPOCH);
                (modified, entry.path(), directory_size(&entry.path()))
            })
            .collect();
        let mut total: u64 = dirs.iter().map(|(_, _, size)| size).sum();
        dirs.sort_by_key(|(modified, _, _)| *modified);
        let mut freed = 0;
        for (_, path, size) in dirs {
            if total <= max_bytes {
                break;
            }
            if fs::remove_dir_all(&path).is_ok() {
                total -= size;
                freed += size;
            }
        }
        Ok(freed)
    }
}

/// Waits for an FFmpeg child while honouring cancellation: the process is killed and reaped
/// before the error returns, so the caller's staging can be removed safely.
pub fn wait_cancellable(
    child: &mut std::process::Child,
    ctx: &JobContext,
    what: &str,
) -> Result<(), JobError> {
    use std::time::Duration;
    loop {
        if ctx.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(JobError::cancelled());
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    return Ok(());
                }
                let mut stderr = Vec::new();
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = pipe.read_to_end(&mut stderr);
                }
                return Err(tool_failed(what, &stderr));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(error) => return Err(JobError::new("io_error", format!("{what}: {error}"))),
        }
    }
}

/// Which file a consumer may read for `purpose`. A proxy is a preview-only derivative, so export
/// never gets one: it always reads the original.
pub fn for_export(asset: &Asset) -> Option<&str> {
    match &asset.source {
        Source::File { path } => Some(path),
        _ => None,
    }
}
