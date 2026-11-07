//! Content-based probe: what a file *is*, from its bytes and FFprobe, never from its extension.

use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};
use tessel_editor_core::assets::{AudioStream, MediaKind, Meta, SCENE_FORMAT, scene_requirements};
use tessel_editor_core::jobs::JobError;
use tessel_editor_core::sharp::{self, Choice};
use tessel_editor_core::splat;
use tessel_editor_core::time::{Rate, Ratio};

use crate::fsx::io_error;
use crate::tools::Tools;

/// Scene files are small JSON documents; anything bigger is not one.
const SCENE_MAX_BYTES: u64 = 4 << 20;
/// Codecs FFprobe reports for still images.
const IMAGE_CODECS: [&str; 7] = ["png", "mjpeg", "webp", "bmp", "tiff", "jpeg2000", "jpegls"];

#[derive(Debug, Clone)]
pub struct Probe {
    pub kind: MediaKind,
    pub meta: Meta,
    pub size: u64,
    pub has_audio: bool,
    pub has_video: bool,
    pub warnings: Vec<String>,
    /// Compact, comparable description of the streams: what a relink checks against.
    pub summary: Value,
}

impl Probe {
    pub fn to_json(&self) -> Value {
        json!({
            "kind": self.kind.label(), "format": self.meta.format, "size": self.size,
            "width": self.meta.width, "height": self.meta.height,
            "rate": self.meta.rate.map(Rate::to_json),
            "duration": self.meta.duration.map(Ratio::to_json),
            "sample_rate": self.meta.sample_rate, "channels": self.meta.channels,
            "splat_count": self.meta.splat_count, "has_audio": self.has_audio,
            "warnings": self.warnings, "streams": self.summary,
        })
    }
}

fn unsupported(path: &Path, message: impl Into<String>) -> JobError {
    JobError::new(
        "unsupported_format",
        format!("`{}`: {}", path.display(), message.into()),
    )
}

fn fraction(text: &str) -> Option<(i64, i64)> {
    let (num, den) = text.split_once('/')?;
    let (num, den) = (
        num.trim().parse::<i64>().ok()?,
        den.trim().parse::<i64>().ok()?,
    );
    (num > 0 && den > 0).then_some((num, den))
}

fn number(value: &Value) -> Option<f64> {
    value.as_f64().or_else(|| value.as_str()?.parse().ok())
}

/// Exact duration of a stream: `duration_ts × time_base`, falling back to the container's
/// decimal seconds at microsecond precision.
fn stream_duration(stream: &Value, format: &Value) -> Option<Ratio> {
    if let (Some(ticks), Some((num, den))) = (
        stream["duration_ts"].as_i64().filter(|ticks| *ticks > 0),
        stream["time_base"].as_str().and_then(fraction),
    ) {
        return Ratio::new(ticks.checked_mul(num)?, den);
    }
    let seconds = number(&stream["duration"]).or_else(|| number(&format["duration"]))?;
    (seconds > 0.0).then(|| Ratio::new((seconds * 1_000_000.0).round() as i64, 1_000_000))?
}

/// Where a stream's first sample/picture sits on the container clock: `start_pts × time_base`,
/// else FFprobe's decimal `start_time` (microsecond precision).
fn stream_start(stream: &Value) -> Ratio {
    if let (Some(ticks), Some((num, den))) = (
        stream["start_pts"].as_i64(),
        stream["time_base"].as_str().and_then(fraction),
    ) && let Some(start) = ticks.checked_mul(num).and_then(|n| Ratio::new(n, den))
    {
        return start;
    }
    number(&stream["start_time"])
        .and_then(|seconds| Ratio::new((seconds * 1_000_000.0).round() as i64, 1_000_000))
        .unwrap_or(Ratio::ZERO)
}

/// Outcome of the packet-timestamp scan of the first video stream.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timing {
    pub frames: u64,
    /// Every gap between consecutive presentation times equals the typical gap within the
    /// timestamp precision (one tick of the stream's time base).
    pub constant: bool,
    /// Largest gap divided by the smallest, for the report.
    pub spread: f64,
}

/// Classifies presentation times (seconds). Pure so the policy is testable without media:
/// jitter of one time-base tick (millisecond timescales alternate 33/34 ms at 30 fps) is still
/// constant; a dropped or delayed frame is not.
pub fn classify_timing(times: &[f64], tick: f64) -> Option<Timing> {
    if times.len() < 3 {
        return None;
    }
    let mut sorted = times.to_vec();
    sorted.sort_by(f64::total_cmp);
    let gaps: Vec<f64> = sorted.windows(2).map(|pair| pair[1] - pair[0]).collect();
    let mut ordered = gaps.clone();
    ordered.sort_by(f64::total_cmp);
    let typical = ordered[ordered.len() / 2];
    if typical <= 0.0 {
        return None;
    }
    let tolerance = tick * 1.5 + typical * 1e-6;
    let constant = gaps.iter().all(|gap| (gap - typical).abs() <= tolerance);
    Some(Timing {
        frames: times.len() as u64,
        constant,
        spread: ordered[ordered.len() - 1] / ordered[0].max(f64::EPSILON),
    })
}

/// Presentation times of every packet of the first video stream. `None` when FFprobe cannot list
/// them: the caller then falls back to comparing the container's two frame-rate estimates.
fn video_timing(tools: &Tools, path: &Path, tick: f64) -> Option<Timing> {
    let output = Command::new(&tools.ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "packet=pts_time",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let times: Vec<f64> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().trim_end_matches(',').parse().ok())
        .collect();
    classify_timing(&times, tick)
}

fn rotation(stream: &Value) -> i64 {
    let from_matrix = stream["side_data_list"].as_array().and_then(|list| {
        list.iter().find_map(|entry| {
            entry["rotation"]
                .as_i64()
                .or_else(|| number(&entry["rotation"]).map(|r| r as i64))
        })
    });
    let from_tag = stream["tags"]["rotate"]
        .as_str()
        .and_then(|r| r.parse::<i64>().ok());
    from_matrix.or(from_tag).unwrap_or(0).rem_euclid(360)
}

fn read_head(path: &Path, max: usize) -> Result<Vec<u8>, JobError> {
    let mut file = File::open(path).map_err(|error| io_error(path, &error))?;
    let mut buffer = vec![0u8; max];
    let mut filled = 0;
    while filled < max {
        let read = file
            .read(&mut buffer[filled..])
            .map_err(|error| io_error(path, &error))?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    buffer.truncate(filled);
    Ok(buffer)
}

/// Splat and scene files first (the core parses them; FFprobe cannot), then FFprobe. A SHARP
/// splat keeps its capture camera (`Choice::Auto`), see [`probe_with`].
pub fn probe(tools: &Tools, path: &Path) -> Result<Probe, JobError> {
    probe_with(tools, path, Choice::Auto)
}

/// [`probe`] with the choice for the capture metadata of a SHARP PLY (BL143): `Auto` uses it when
/// it is complete and refuses the file (`incomplete_capture`/`ambiguous_capture`) when it is not,
/// `Ignore` imports the file as a plain splat.
pub fn probe_with(tools: &Tools, path: &Path, capture: Choice) -> Result<Probe, JobError> {
    let metadata = std::fs::metadata(path).map_err(|error| io_error(path, &error))?;
    if !metadata.is_file() {
        return Err(unsupported(path, "not a regular file"));
    }
    let size = metadata.len();
    if size == 0 {
        return Err(unsupported(path, "the file is empty"));
    }
    let head = read_head(path, 16)?;
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let looks_like_splat = head.starts_with(b"ply")
        || head.starts_with(&[0x1f, 0x8b])
        || extension.as_deref() == Some("splat");
    if looks_like_splat {
        // Content decides: a `.splat` extension on a file that is not a multiple of 32 bytes,
        // or a generic PLY point cloud, is rejected rather than rendered as garbage.
        let info = splat::inspect(path).map_err(|message| unsupported(path, message))?;
        let capture = if info.format == splat::SplatFormat::Ply {
            sharp::read(path, capture).map_err(|error| {
                let mut converted = JobError::new(error.code, error.message);
                if let Some(details) = error.details {
                    converted = converted.with_details(*details);
                }
                converted
            })?
        } else {
            None
        };
        let mut summary = json!({ "splat_format": info.format.label(), "splat_count": info.count });
        if let Some(capture) = &capture {
            summary["capture"] = capture.to_document();
        }
        return Ok(Probe {
            kind: MediaKind::Splat,
            meta: Meta {
                splat_count: Some(info.count),
                format: Some(info.format.label().into()),
                capture,
                ..Meta::default()
            },
            size,
            has_audio: false,
            has_video: false,
            warnings: Vec::new(),
            summary,
        });
    }
    if head.first() == Some(&b'{') && size <= SCENE_MAX_BYTES {
        let text = std::fs::read_to_string(path).map_err(|error| io_error(path, &error))?;
        if text.contains(SCENE_FORMAT) {
            let requires =
                scene_requirements(&text).map_err(|message| unsupported(path, message))?;
            return Ok(Probe {
                kind: MediaKind::Scene,
                meta: Meta {
                    requires,
                    format: Some(SCENE_FORMAT.into()),
                    ..Meta::default()
                },
                size,
                has_audio: false,
                has_video: false,
                warnings: Vec::new(),
                summary: json!({ "scene": SCENE_FORMAT }),
            });
        }
    }
    probe_with_ffprobe(tools, path, size)
}

fn probe_with_ffprobe(tools: &Tools, path: &Path, size: u64) -> Result<Probe, JobError> {
    let output = Command::new(&tools.ffprobe)
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        .output()
        .map_err(|error| JobError::new("tool_unavailable", format!("ffprobe: {error}")))?;
    if !output.status.success() {
        return Err(unsupported(
            path,
            format!(
                "not a recognised media file ({})",
                String::from_utf8_lossy(&output.stderr)
                    .lines()
                    .next()
                    .unwrap_or("ffprobe failed")
            ),
        ));
    }
    let report: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| JobError::new("tool_unavailable", format!("ffprobe output: {error}")))?;
    let streams = report["streams"].as_array().cloned().unwrap_or_default();
    let format = &report["format"];
    let container = format["format_name"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    let is_picture = |stream: &Value| stream["disposition"]["attached_pic"].as_i64() == Some(1);
    let video = streams
        .iter()
        .find(|s| s["codec_type"] == "video" && !is_picture(s));
    let audio = streams.iter().find(|s| s["codec_type"] == "audio");

    let compact: Vec<Value> = streams
        .iter()
        .map(|s| {
            json!({ "type": s["codec_type"], "codec": s["codec_name"],
            "width": s["width"], "height": s["height"], "rate": s["avg_frame_rate"],
            "sample_rate": s["sample_rate"], "channels": s["channels"] })
        })
        .collect();
    let mut warnings = Vec::new();

    if let Some(video) = video {
        let codec = video["codec_name"].as_str().unwrap_or("unknown");
        let (mut width, mut height) = (
            video["width"].as_u64().unwrap_or(0) as u32,
            video["height"].as_u64().unwrap_or(0) as u32,
        );
        if width == 0 || height == 0 {
            return Err(unsupported(path, "the video stream has no size"));
        }
        if matches!(rotation(video), 90 | 270) {
            std::mem::swap(&mut width, &mut height);
        }
        let is_image_container = container.ends_with("_pipe") || container == "image2";
        if IMAGE_CODECS.contains(&codec) && is_image_container {
            let label = match codec {
                "mjpeg" => "jpeg",
                other => other,
            };
            return Ok(Probe {
                kind: MediaKind::Still,
                meta: Meta {
                    width: Some(width),
                    height: Some(height),
                    format: Some(label.into()),
                    ..Meta::default()
                },
                size,
                has_audio: false,
                has_video: true,
                warnings,
                summary: json!(compact),
            });
        }
        if container.contains("gif") || codec == "gif" {
            return Err(unsupported(
                path,
                "animated GIF is not supported; convert it to MP4",
            ));
        }
        let average = video["avg_frame_rate"].as_str().and_then(fraction);
        let nominal = video["r_frame_rate"].as_str().and_then(fraction);
        let fps = |(num, den): (i64, i64)| num as f64 / den as f64;
        let tick = video["time_base"]
            .as_str()
            .and_then(fraction)
            .map_or(0.001, |(num, den)| num as f64 / den as f64);
        // The packet timestamps decide. The two frame-rate estimates only stand in when the scan
        // is unavailable: either alone misses a stream whose estimates happen to agree.
        let timing = video_timing(tools, path, tick);
        let variable = match (timing, average, nominal) {
            (Some(timing), ..) => !timing.constant,
            (None, Some(avg), Some(nom)) => (fps(avg) - fps(nom)).abs() / fps(nom) > 0.005,
            _ => false,
        };
        let (num, den) = match (average, nominal) {
            // A variable-rate source is conformed by timestamp to the grid of its nominal rate (the
            // rate every frame time is a multiple of), else its average when that is not usable.
            (Some(avg), Some(nom)) if variable => {
                if fps(nom) <= 120.0 {
                    nom
                } else {
                    avg
                }
            }
            (Some(_), Some(nom)) => nom,
            (Some(only), None) | (None, Some(only)) => only,
            (None, None) => return Err(unsupported(path, "the video stream has no frame rate")),
        };
        if variable {
            let spread = timing.map_or(String::new(), |t| {
                format!(", longest/shortest frame gap {:.1}×", t.spread)
            });
            warnings.push(format!(
                "variable_frame_rate: frame times are not constant{spread}; frames are conformed by timestamp to a constant {:.3} fps grid (at most half a frame of timing error), never by frame index",
                fps((num, den))
            ));
        }
        let rate = Rate::new(num as u32, den as u32)
            .ok_or_else(|| unsupported(path, "the frame rate is out of range"))?;
        let transfer = video["color_transfer"].as_str().unwrap_or("unknown");
        if matches!(transfer, "smpte2084" | "arib-std-b67") {
            warnings.push(format!(
                "hdr_not_tonemapped: transfer `{transfer}`; frames are converted as SDR without tone mapping"
            ));
        }
        let audio_codec = audio.and_then(|a| a["codec_name"].as_str());
        let mut meta = Meta {
            width: Some(width),
            height: Some(height),
            rate: Some(rate),
            duration: stream_duration(video, format),
            format: Some(container.split(',').next().unwrap_or("video").to_string()),
            vfr_conformed: variable,
            ..Meta::default()
        };
        // The asset's clock starts at the first picture; each sound stream records where it starts
        // on that clock so the mixer places it by timestamp.
        let origin = stream_start(video);
        meta.audio_streams = streams
            .iter()
            .filter(|s| s["codec_type"] == "audio")
            .enumerate()
            .filter_map(|(index, stream)| {
                Some(AudioStream {
                    index: index as u32,
                    sample_rate: stream["sample_rate"].as_str()?.parse().ok()?,
                    channels: stream["channels"].as_u64()? as u16,
                    offset: stream_start(stream).sub(origin).unwrap_or(Ratio::ZERO),
                })
            })
            .collect();
        if let Some(first) = meta.audio_streams.first() {
            meta.sample_rate = Some(first.sample_rate);
            meta.channels = Some(first.channels);
        }
        let mut summary = json!(compact);
        summary = json!({ "streams": summary, "video_codec": codec, "audio_codec": audio_codec,
            "color": { "space": video["color_space"], "transfer": transfer,
                "primaries": video["color_primaries"], "range": video["color_range"],
                "pix_fmt": video["pix_fmt"] } });
        return Ok(Probe {
            kind: MediaKind::Video,
            meta,
            size,
            has_audio: audio.is_some(),
            has_video: true,
            warnings,
            summary,
        });
    }
    if let Some(audio) = audio {
        let sample_rate = audio["sample_rate"]
            .as_str()
            .and_then(|r| r.parse::<u32>().ok());
        let duration = stream_duration(audio, format);
        // Only what the core mixer reads directly is labelled `wav`; every other codec (and
        // 24-bit/multichannel WAV) has a different `format` and gets a decoded WAV derivative.
        let codec = audio["codec_name"].as_str().unwrap_or("audio");
        let channels = audio["channels"].as_u64().unwrap_or(0);
        let audio_streams = streams
            .iter()
            .filter(|s| s["codec_type"] == "audio")
            .enumerate()
            .filter_map(|(index, stream)| {
                Some(AudioStream {
                    index: index as u32,
                    sample_rate: stream["sample_rate"].as_str()?.parse().ok()?,
                    channels: stream["channels"].as_u64()? as u16,
                    offset: Ratio::ZERO,
                })
            })
            .collect();
        let wav = container.contains("wav")
            && matches!(codec, "pcm_s16le" | "pcm_f32le")
            && (1..=2).contains(&channels);
        return Ok(Probe {
            kind: MediaKind::Audio,
            meta: Meta {
                sample_rate,
                channels: audio["channels"].as_u64().map(|c| c as u16),
                duration,
                format: Some(if wav {
                    "wav".to_string()
                } else {
                    codec.to_string()
                }),
                audio_streams,
                ..Meta::default()
            },
            size,
            has_audio: true,
            has_video: false,
            warnings,
            summary: json!(compact),
        });
    }
    Err(unsupported(path, "no video or audio stream was found"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn times(gaps: &[f64]) -> Vec<f64> {
        let mut now = 0.0;
        std::iter::once(0.0)
            .chain(gaps.iter().map(|gap| {
                now += gap;
                now
            }))
            .collect()
    }

    #[test]
    fn millisecond_rounding_is_still_a_constant_rate() {
        // 30 fps in a 1 ms time base alternates 33/34 ms gaps.
        let gaps: Vec<f64> = (0..90)
            .map(|n| if n % 3 == 2 { 0.034 } else { 0.033 })
            .collect();
        let timing = classify_timing(&times(&gaps), 0.001).unwrap();
        assert!(timing.constant, "{timing:?}");
    }

    #[test]
    fn a_dropped_frame_or_a_slow_stretch_is_variable() {
        let mut gaps = vec![1.0 / 24.0; 40];
        gaps[20] = 2.0 / 24.0;
        assert!(
            !classify_timing(&times(&gaps), 1.0 / 12288.0)
                .unwrap()
                .constant
        );
        let slow: Vec<f64> = (0..40)
            .map(|n| if n < 20 { 1.0 / 30.0 } else { 1.0 / 15.0 })
            .collect();
        let timing = classify_timing(&times(&slow), 1.0 / 90000.0).unwrap();
        assert!(!timing.constant && timing.spread > 1.9, "{timing:?}");
    }

    #[test]
    fn b_frame_decode_order_does_not_look_variable() {
        // Packets are listed in decode order: presentation times arrive shuffled.
        let mut shuffled = times(&vec![1.0 / 24.0; 30]);
        shuffled.swap(3, 5);
        shuffled.swap(10, 11);
        assert!(classify_timing(&shuffled, 1.0 / 12288.0).unwrap().constant);
    }

    #[test]
    fn too_little_data_gives_no_verdict() {
        assert!(classify_timing(&[0.0, 0.04], 0.001).is_none());
    }
}
