//! Typed assets: what a library item points at and what is known about it.
//!
//! An asset is registered with a [`Source`] and probed [`Meta`]. Probing reads the file
//! header only (PNG IHDR, WAV `fmt `/`data`, splat byte length, scene JSON); it never
//! decodes media. Containers the core cannot parse (WebM/MP4) require the caller to
//! declare size, rate and duration, and the runtime confirms them when it loads the asset.

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::audio;
use crate::content::TextSpec;
use crate::id::Id;
use crate::state::error::CommandError;
use crate::time::{Rate, Ratio};

pub const SCENE_FORMAT: &str = "tessel-scene";
pub const SCENE_VERSION: u64 = 1;
/// Bytes per splat in the `.splat` layout (see `docs/features/viewport.md`).
pub const SPLAT_STRIDE: u64 = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaKind {
    Still,
    Video,
    Audio,
    Text,
    Scene,
    Splat,
    Timeline,
}

impl MediaKind {
    pub const ASSETS: [Self; 6] = [
        Self::Still,
        Self::Video,
        Self::Audio,
        Self::Text,
        Self::Scene,
        Self::Splat,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Still => "still",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Text => "text",
            Self::Scene => "scene",
            Self::Splat => "splat",
            Self::Timeline => "timeline",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        // `image` is the documented spelling for API callers; `still` is the library label.
        if raw == "image" {
            return Some(Self::Still);
        }
        Self::ASSETS
            .into_iter()
            .chain([Self::Timeline])
            .find(|kind| kind.label() == raw)
    }

    pub fn is_visual(self) -> bool {
        matches!(
            self,
            Self::Still | Self::Video | Self::Text | Self::Scene | Self::Splat
        )
    }
}

/// The two bundled demonstration images (see `docs/proui-reference.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemoImage {
    Highlands,
    Coast,
}

impl DemoImage {
    pub fn file_name(self) -> &'static str {
        match self {
            Self::Highlands => "highlands.png",
            Self::Coast => "coast.png",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    Demo(DemoImage),
    /// One file; the caller supplies an absolute or working-directory-relative path.
    File {
        path: String,
    },
    /// A video stored as numbered images. `pattern` contains one `%0Nd` placeholder.
    FrameSequence {
        pattern: String,
        first: i64,
        count: i64,
    },
    /// Content lives in [`Meta::text`]; there is no file.
    Inline,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Meta {
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Frame rate of video sources. Constant rate only: variable-rate video is unsupported.
    pub rate: Option<Rate>,
    /// Length of video/audio sources.
    pub duration: Option<Ratio>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub splat_count: Option<u64>,
    /// Assets a scene refers to (splats). They must exist before a clip can use the scene.
    pub requires: Vec<Id>,
    pub text: Option<TextSpec>,
    /// Container or format label shown to callers (`png`, `wav`, `splat`, `tessel-scene`, ...).
    pub format: Option<String>,
    /// Audio streams of a container video, in the order FFmpeg lists them. Empty for assets
    /// imported before streams were recorded: [`Meta::audio_stream`] then falls back to
    /// `sample_rate`/`channels` as one stream starting with the picture.
    pub audio_streams: Vec<AudioStream>,
    /// The source has a variable frame rate. Its frames are conformed by timestamp to the constant
    /// grid `rate` (see `docs/features/media-io.md`), never by frame index.
    pub vfr_conformed: bool,
    /// The camera a SHARP splat was captured with (BL143): `None` for every other asset, and for a
    /// splat imported with `capture: ignore` or whose file carries no capture metadata.
    pub capture: Option<crate::sharp::Capture>,
}

/// One audio stream of a container.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioStream {
    /// Ordinal among the audio streams of the file (`0:a:<index>` in FFmpeg).
    pub index: u32,
    pub sample_rate: u32,
    pub channels: u16,
    /// Where the stream's first sample sits on the asset's clock, whose zero is the first picture of
    /// a video (or the first sample for audio-only files). Negative when sound starts before
    /// the picture. WHY stored: the mixer must place sound by timestamp, not assume sample 0 is
    /// frame 0.
    pub offset: Ratio,
}

impl Meta {
    /// The stream a clip asks for: recorded streams first, then the legacy single-stream fields.
    pub fn audio_stream(&self, index: u32) -> Option<AudioStream> {
        if let Some(stream) = self.audio_streams.iter().find(|s| s.index == index) {
            return Some(*stream);
        }
        if !self.audio_streams.is_empty() || index != 0 {
            return None;
        }
        Some(AudioStream {
            index: 0,
            sample_rate: self.sample_rate?,
            channels: self.channels.unwrap_or(2),
            offset: Ratio::ZERO,
        })
    }

    /// Whether the asset carries sound the mixer can use.
    pub fn has_audio(&self) -> bool {
        self.audio_stream(0).is_some()
    }
}

/// Where the bytes of a file-backed asset live relative to the project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Storage {
    /// Not classified: a file registered by path before the workspace existed, or a
    /// generated/inline asset. Treated as a reference when the project is saved.
    #[default]
    Unclassified,
    /// Copied into the project's `media/<hash>/` folder; the document stores a relative path.
    Managed,
    /// Left where the user keeps it; only the path (and `origin`) is recorded.
    Reference,
}

impl Storage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unclassified => "unclassified",
            Self::Managed => "managed",
            Self::Reference => "reference",
        }
    }

    pub fn parse(label: &str) -> Option<Self> {
        [Self::Unclassified, Self::Managed, Self::Reference]
            .into_iter()
            .find(|storage| storage.label() == label)
    }
}

/// Logical identity of the bytes behind an asset. WHY separate from `Source`: the asset ID is
/// the editing identity (clips point at it) while `hash` is the content identity, so two
/// assets can share bytes and a relink can verify it found the same file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Identity {
    /// SHA-256 of the file, lowercase hex. `None` until an adapter hashed the file.
    pub hash: Option<String>,
    pub size: Option<u64>,
    pub storage: Storage,
    /// Where the user's original lived at import time (informational: relink hint).
    pub origin: Option<String>,
    /// File name as imported, kept when the managed copy is renamed to a safe name.
    pub file_name: Option<String>,
    /// Adapter probe summary (container, streams) that a relink compares against.
    pub probe: Option<Value>,
}

/// What an asset is made of: the part of a library item that is not presentation.
#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    pub source: Source,
    pub meta: Meta,
    pub identity: Identity,
    /// Derived media the adapter produced for a container video or compressed audio. Runtime-only: never
    /// persisted (the cache is disposable and rebuilt from the original), and when present the
    /// render backend reads these numbered frames instead of seeking a `<video>` element, so
    /// every engine shows the same pixels for the same frame.
    pub decoded: Option<Decoded>,
}

/// What the media adapter decoded from the original. Each part is optional: a still has none,
/// a silent video only frames, an MP3 only audio.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Decoded {
    pub frames: Option<DecodedFrames>,
    /// One 16-bit PCM WAV per decoded audio stream, at the stream's own rate (stereo at most), for
    /// the mixer. The sample 0 of each file is the stream's first sample: its place on the asset's
    /// clock is [`AudioStream::offset`].
    pub audio: Vec<DecodedAudio>,
    /// Smaller copy of `frames` for interactive preview (see `preview.rs`). Never an export
    /// input; `None` until the media adapter derived it for the active preview policy.
    pub proxy: Option<ProxyFrames>,
}

/// Image format of a preview proxy. JPEG is the default: it decodes several times faster than
/// the PNG original at the proxy size; PNG stays available where an alpha channel matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProxyFormat {
    #[default]
    Jpeg,
    Png,
}

impl ProxyFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Jpeg => "jpeg",
            Self::Png => "png",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "jpeg" | "jpg" => Some(Self::Jpeg),
            "png" => Some(Self::Png),
            _ => None,
        }
    }
}

/// Numbered frames derived from [`DecodedFrames`], same numbering and count, scaled to
/// `width` x `height`. `source_frames` is the sequence it was derived from: a proxy is valid only
/// while it still names the frames the asset is currently drawn from.
#[derive(Debug, Clone, PartialEq)]
pub struct ProxyFrames {
    pub pattern: String,
    pub first: i64,
    pub count: i64,
    pub width: u32,
    pub height: u32,
    pub format: ProxyFormat,
    pub source_frames: String,
}

impl ProxyFrames {
    pub fn to_json(&self) -> Value {
        json!({ "pattern": self.pattern, "first": self.first, "count": self.count,
            "width": self.width, "height": self.height, "format": self.format.label() })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecodedAudio {
    pub stream: u32,
    pub path: String,
}

impl Decoded {
    pub fn audio_path(&self, stream: u32) -> Option<&str> {
        self.audio
            .iter()
            .find(|audio| audio.stream == stream)
            .map(|audio| audio.path.as_str())
    }
}

/// Numbered PNG frames decoded from a container video at the asset's own rate.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedFrames {
    pub pattern: String,
    pub first: i64,
    pub count: i64,
}

impl Asset {
    pub fn to_json(&self, kind: MediaKind) -> Value {
        let source = match &self.source {
            Source::Demo(image) => json!({ "type": "demo", "name": image.file_name() }),
            Source::File { path } => json!({ "type": "file", "path": path }),
            Source::FrameSequence {
                pattern,
                first,
                count,
            } => {
                json!({ "type": "frame_sequence", "pattern": pattern, "first": first, "count": count })
            }
            Source::Inline => json!({ "type": "inline" }),
        };
        let meta = &self.meta;
        json!({
            "kind": kind.label(),
            "source": source,
            "format": meta.format,
            "width": meta.width,
            "height": meta.height,
            "rate": meta.rate.map(Rate::to_json),
            "duration": meta.duration.map(Ratio::to_json),
            "sample_rate": meta.sample_rate,
            "channels": meta.channels,
            "splat_count": meta.splat_count,
            "requires": meta.requires.iter().map(|id| id.as_str()).collect::<Vec<_>>(),
            "text": meta.text.as_ref().map(TextSpec::to_json),
            "hash": self.identity.hash,
            "size": self.identity.size,
            "storage": self.identity.storage.label(),
            "audio_streams": meta.audio_streams.iter().map(|stream| json!({
                "index": stream.index, "sample_rate": stream.sample_rate,
                "channels": stream.channels, "offset": stream.offset.to_json(),
            })).collect::<Vec<_>>(),
            "vfr_conformed": meta.vfr_conformed,
            "capture": meta.capture.as_ref().map(crate::sharp::Capture::to_json),
        })
    }
}

/// Values a caller supplies for formats the core does not parse.
#[derive(Debug, Clone, Default)]
pub struct Declared {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub rate: Option<Rate>,
    pub duration: Option<Ratio>,
    pub text: Option<TextSpec>,
    pub frames: Option<i64>,
    pub first: Option<i64>,
    /// What to do with the capture metadata of a SHARP splat (BL143).
    pub capture: crate::sharp::Choice,
}

fn io_error(path: &str, error: &std::io::Error) -> CommandError {
    CommandError::new("asset_unreadable", format!("Cannot read `{path}`: {error}")).target(path)
}

fn invalid_asset(path: &str, message: impl Into<String>) -> CommandError {
    CommandError::new("invalid_asset", message).target(path)
}

fn read_header(path: &str, max: u64) -> Result<Vec<u8>, CommandError> {
    use std::io::Read;
    let file = fs::File::open(path).map_err(|error| io_error(path, &error))?;
    let mut buffer = Vec::new();
    file.take(max)
        .read_to_end(&mut buffer)
        .map_err(|error| io_error(path, &error))?;
    Ok(buffer)
}

fn png_size(path: &str) -> Result<(u32, u32), CommandError> {
    let header = read_header(path, 24)?;
    if header.len() < 24 || header[..8] != [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a] {
        return Err(invalid_asset(path, "Not a PNG file"));
    }
    let width = u32::from_be_bytes([header[16], header[17], header[18], header[19]]);
    let height = u32::from_be_bytes([header[20], header[21], header[22], header[23]]);
    if width == 0 || height == 0 {
        return Err(invalid_asset(path, "PNG has an empty size"));
    }
    Ok((width, height))
}

/// Picture formats a frame sequence may hold. Every frame of one sequence has the same one, so
/// a sequence never mixes a lossless proxy with a lossy one by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameFormat {
    Png,
    Jpeg,
}

impl FrameFormat {
    fn label(self) -> &'static str {
        match self {
            Self::Png => "frame-sequence/png",
            Self::Jpeg => "frame-sequence/jpeg",
        }
    }
}

/// Most bytes read looking for the size marker of a JPEG: metadata segments (EXIF, ICC, XMP)
/// come before it and each is at most 64 KiB, so a regular file has it well inside this.
const JPEG_HEADER_LIMIT: u64 = 1 << 20;

/// Width and height from the start of a JPEG: the first start-of-frame marker.
fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut at = 2;
    loop {
        // Markers may be preceded by any number of 0xFF fill bytes.
        while *bytes.get(at)? == 0xFF && *bytes.get(at + 1)? == 0xFF {
            at += 1;
        }
        if *bytes.get(at)? != 0xFF {
            return None;
        }
        let marker = *bytes.get(at + 1)?;
        at += 2;
        match marker {
            // Standalone markers carry no length.
            0x01 | 0xD0..=0xD8 => continue,
            // Start of scan or end of image before any frame header: no size to read.
            0xD9 | 0xDA => return None,
            _ => {}
        }
        let length = usize::from(u16::from_be_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]));
        // SOF0..SOF15 except DHT (C4), JPG (C8) and DAC (CC) hold the picture size.
        if matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            let field = |offset: usize| -> Option<u32> {
                Some(u32::from(u16::from_be_bytes([
                    *bytes.get(at + offset)?,
                    *bytes.get(at + offset + 1)?,
                ])))
            };
            return Some((field(5)?, field(3)?));
        }
        if length < 2 {
            return None;
        }
        at += length;
    }
}

/// Size and format of a sequence frame, from its header alone. PNG and JPEG are the formats a
/// proxy sequence may use (the runtime decodes both); anything else is refused by name.
fn frame_info(path: &str) -> Result<(u32, u32, FrameFormat), CommandError> {
    let header = read_header(path, JPEG_HEADER_LIMIT)?;
    if header.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        let (width, height) = png_size(path)?;
        return Ok((width, height, FrameFormat::Png));
    }
    if header.starts_with(&[0xFF, 0xD8]) {
        let (width, height) = jpeg_size(&header)
            .filter(|(width, height)| *width > 0 && *height > 0)
            .ok_or_else(|| invalid_asset(path, "JPEG has no readable size"))?;
        return Ok((width, height, FrameFormat::Jpeg));
    }
    Err(invalid_asset(
        path,
        "A frame sequence holds PNG or JPEG frames; this file is neither",
    ))
}

/// `pattern` with its numeric placeholder replaced by `index`: `%0Nd` (zero padded to N digits)
/// or `%d` (no padding). The runtime and the asset server read the same two forms.
pub fn expand_pattern(pattern: &str, index: i64) -> Option<String> {
    let start = pattern.find('%')?;
    let rest = &pattern[start + 1..];
    let zero_padded = rest.starts_with('0');
    let digits_end = rest.find('d')?;
    let digits = &rest[usize::from(zero_padded)..digits_end];
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let width: usize = if zero_padded { digits.parse().ok()? } else { 0 };
    if !zero_padded && !digits.is_empty() {
        // `%5d` would pad with spaces in printf; there is no file name that means that.
        return None;
    }
    Some(format!(
        "{}{index:0width$}{}",
        &pattern[..start],
        &rest[digits_end + 1..]
    ))
}

impl Asset {
    /// Registers a file-backed asset after reading its header.
    pub fn probe(kind: MediaKind, path: &str, declared: &Declared) -> Result<Self, CommandError> {
        let meta = match kind {
            MediaKind::Still => {
                let (width, height) = png_size(path)?;
                Meta {
                    width: Some(width),
                    height: Some(height),
                    format: Some("png".into()),
                    ..Meta::default()
                }
            }
            MediaKind::Audio => {
                let info = audio::wav_info(&read_whole(path)?)
                    .map_err(|message| invalid_asset(path, message))?;
                Meta {
                    sample_rate: Some(info.sample_rate),
                    channels: Some(info.channels),
                    duration: Ratio::new(info.frames as i64, i64::from(info.sample_rate)),
                    format: Some("wav".into()),
                    ..Meta::default()
                }
            }
            MediaKind::Splat => {
                let info = crate::splat::inspect(Path::new(path))
                    .map_err(|message| invalid_asset(path, message))?;
                let capture = if info.format == crate::splat::SplatFormat::Ply {
                    crate::sharp::read(Path::new(path), declared.capture)?
                } else {
                    None
                };
                Meta {
                    splat_count: Some(info.count),
                    format: Some(info.format.label().into()),
                    capture,
                    ..Meta::default()
                }
            }
            MediaKind::Scene => {
                let text = String::from_utf8(read_whole(path)?)
                    .map_err(|_| invalid_asset(path, "Scene file is not UTF-8"))?;
                let requires =
                    scene_requirements(&text).map_err(|message| invalid_asset(path, message))?;
                Meta {
                    requires,
                    format: Some(SCENE_FORMAT.into()),
                    ..Meta::default()
                }
            }
            MediaKind::Video => {
                fs::metadata(path).map_err(|error| io_error(path, &error))?;
                declared_video(
                    path,
                    declared,
                    Path::new(path).extension().and_then(|e| e.to_str()),
                )?
            }
            MediaKind::Text | MediaKind::Timeline => {
                return Err(CommandError::new(
                    "invalid_asset",
                    format!("`{}` assets are not file-backed", kind.label()),
                ));
            }
        };
        Ok(Self {
            source: Source::File {
                path: path.to_string(),
            },
            meta,
            identity: Identity::default(),
            decoded: None,
        })
    }

    /// A video stored as numbered PNG or JPEG frames (`frames/f%05d.png`, `frames/%d.jpg`).
    pub fn probe_frame_sequence(pattern: &str, declared: &Declared) -> Result<Self, CommandError> {
        let first = declared.first.unwrap_or(0);
        let count = declared
            .frames
            .filter(|count| *count > 0)
            .ok_or_else(|| invalid_asset(pattern, "A frame sequence needs `frames` > 0"))?;
        let rate = declared
            .rate
            .ok_or_else(|| invalid_asset(pattern, "A frame sequence needs `rate`"))?;
        let name_of = |index| {
            expand_pattern(pattern, index)
                .ok_or_else(|| invalid_asset(pattern, "Pattern needs one `%0Nd` placeholder"))
        };
        let first_path = name_of(first)?;
        let (width, height, format) = frame_info(&first_path)?;
        let last = name_of(first + count - 1)?;
        let (last_width, last_height, last_format) = frame_info(&last)?;
        if last_format != format {
            return Err(invalid_asset(
                &last,
                "Frames of a sequence share one format (all PNG or all JPEG)",
            ));
        }
        if (last_width, last_height) != (width, height) {
            return Err(invalid_asset(&last, "Frames of a sequence share one size"));
        }
        // A hole is found here, by name, not minutes later when playback reaches it. Only the
        // names are checked: reading every header of a long sequence would make adding it slow.
        for index in first..first + count {
            let path = name_of(index)?;
            if !fs::metadata(&path).is_ok_and(|meta| meta.is_file()) {
                return Err(invalid_asset(&path, "Frame is missing from the sequence"));
            }
        }
        Ok(Self {
            source: Source::FrameSequence {
                pattern: pattern.to_string(),
                first,
                count,
            },
            meta: Meta {
                width: Some(width),
                height: Some(height),
                rate: Some(rate),
                duration: Ratio::new(count * i64::from(rate.den()), i64::from(rate.num())),
                format: Some(format.label().into()),
                ..Meta::default()
            },
            identity: Identity::default(),
            decoded: None,
        })
    }

    pub fn text(spec: TextSpec) -> Result<Self, CommandError> {
        spec.validate()?;
        Ok(Self {
            source: Source::Inline,
            meta: Meta {
                text: Some(spec),
                format: Some("text".into()),
                ..Meta::default()
            },
            identity: Identity::default(),
            decoded: None,
        })
    }
}

fn read_whole(path: &str) -> Result<Vec<u8>, CommandError> {
    fs::read(path).map_err(|error| io_error(path, &error))
}

fn declared_video(
    path: &str,
    declared: &Declared,
    extension: Option<&str>,
) -> Result<Meta, CommandError> {
    let missing = |field: &str| {
        invalid_asset(
            path,
            format!("Container video needs a declared `{field}`; the core does not parse WebM/MP4"),
        )
    };
    Ok(Meta {
        width: Some(declared.width.ok_or_else(|| missing("width"))?),
        height: Some(declared.height.ok_or_else(|| missing("height"))?),
        rate: Some(declared.rate.ok_or_else(|| missing("rate"))?),
        duration: Some(declared.duration.ok_or_else(|| missing("duration"))?),
        format: Some(extension.unwrap_or("video").to_ascii_lowercase()),
        ..Meta::default()
    })
}

/// Splat asset IDs a scene file refers to, after checking its format header.
pub fn scene_requirements(text: &str) -> Result<Vec<Id>, String> {
    let scene: Value =
        serde_json::from_str(text).map_err(|error| format!("Scene is not JSON: {error}"))?;
    if scene["format"] != SCENE_FORMAT || scene["version"] != SCENE_VERSION {
        return Err(format!(
            "Scene needs \"format\": \"{SCENE_FORMAT}\" and \"version\": {SCENE_VERSION}"
        ));
    }
    let objects = scene["objects"]
        .as_array()
        .ok_or("Scene needs an `objects` array")?;
    let mut requires = Vec::new();
    for object in objects {
        if object["type"] == "splat" {
            let reference = object["asset"]
                .as_str()
                .ok_or("A splat object needs an `asset` ID")?;
            let id = Id::new(reference)
                .map_err(|error| format!("Splat asset `{reference}`: {error}"))?;
            if !requires.contains(&id) {
                requires.push(id);
            }
        }
    }
    Ok(requires)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_expansion_pads_the_index() {
        assert_eq!(expand_pattern("a/f%05d.png", 7).unwrap(), "a/f00007.png");
        assert_eq!(expand_pattern("f%03d", 1234).unwrap(), "f1234");
        assert!(expand_pattern("f.png", 1).is_none());
    }

    #[test]
    fn pattern_expansion_accepts_unpadded_and_rejects_other_widths() {
        assert_eq!(expand_pattern("a/%d.jpg", 12).unwrap(), "a/12.jpg");
        assert_eq!(expand_pattern("x%05d.jpg", 3).unwrap(), "x00003.jpg");
        assert!(expand_pattern("x%5d.jpg", 3).is_none());
        assert!(expand_pattern("x%0d.jpg", 3).is_none());
        assert!(expand_pattern("x%s.jpg", 3).is_none());
    }

    /// A JPEG with the header parts the size reader walks (APP0, then SOF0); no scan data, since
    /// only the core's header read is under test, not decoding (the runtime decodes real ones).
    fn jpeg_bytes(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        bytes.extend_from_slice(b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
        bytes.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08]);
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&[0x03, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        bytes.extend_from_slice(&[0xFF, 0xD9]);
        bytes
    }

    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        bytes
    }

    fn sequence_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("tessel-seq-{name}-{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn declared(frames: i64) -> Declared {
        Declared {
            rate: Rate::new(24, 1),
            frames: Some(frames),
            ..Declared::default()
        }
    }

    #[test]
    fn jpeg_size_reads_the_frame_header_and_rejects_truncation() {
        assert_eq!(jpeg_size(&jpeg_bytes(960, 540)), Some((960, 540)));
        assert_eq!(jpeg_size(&jpeg_bytes(960, 540)[..20]), None);
        assert_eq!(jpeg_size(b"\xFF\xD8\xFF\xD9"), None);
        assert_eq!(jpeg_size(&png_bytes(4, 4)), None);
    }

    #[test]
    fn sequences_of_jpeg_frames_are_probed_with_their_format() {
        let dir = sequence_dir("jpeg");
        for index in 0..4 {
            fs::write(dir.join(format!("x{index:05}.jpg")), jpeg_bytes(960, 540)).unwrap();
        }
        let pattern = dir.join("x%05d.jpg").to_string_lossy().into_owned();
        let asset = Asset::probe_frame_sequence(&pattern, &declared(4)).unwrap();
        assert_eq!(
            (asset.meta.width, asset.meta.height),
            (Some(960), Some(540))
        );
        assert_eq!(asset.meta.format.as_deref(), Some("frame-sequence/jpeg"));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn png_sequences_with_unpadded_numbers_are_probed() {
        let dir = sequence_dir("png");
        for index in 1..=3 {
            fs::write(dir.join(format!("{index}.png")), png_bytes(64, 36)).unwrap();
        }
        let pattern = dir.join("%d.png").to_string_lossy().into_owned();
        let declared = Declared {
            first: Some(1),
            ..declared(3)
        };
        let asset = Asset::probe_frame_sequence(&pattern, &declared).unwrap();
        assert_eq!(asset.meta.format.as_deref(), Some("frame-sequence/png"));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn invalid_sequences_are_rejected_before_registration() {
        let dir = sequence_dir("invalid");
        let pattern = dir.join("f%05d.img").to_string_lossy().into_owned();
        let write = |index: i64, bytes: Vec<u8>| {
            fs::write(dir.join(format!("f{index:05}.img")), bytes).unwrap();
        };
        let reason = |frames: i64| {
            Asset::probe_frame_sequence(&pattern, &declared(frames))
                .unwrap_err()
                .message
        };

        write(0, b"GIF89a not a frame of ours".to_vec());
        assert!(reason(2).contains("PNG or JPEG"), "{}", reason(2));

        write(0, png_bytes(64, 36));
        write(1, jpeg_bytes(64, 36));
        assert!(reason(2).contains("one format"), "{}", reason(2));

        write(1, png_bytes(32, 36));
        assert!(reason(2).contains("one size"), "{}", reason(2));

        write(1, png_bytes(64, 36));
        write(3, png_bytes(64, 36));
        assert!(reason(4).contains("missing"), "{}", reason(4));

        assert!(
            Asset::probe_frame_sequence(&dir.join("fixed.png").to_string_lossy(), &declared(1))
                .unwrap_err()
                .message
                .contains("placeholder")
        );
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn scene_requirements_validate_the_header_and_list_splats() {
        let ok = r#"{"format":"tessel-scene","version":1,"objects":[
            {"type":"box"},{"type":"splat","asset":"cloud"},{"type":"splat","asset":"cloud"}]}"#;
        assert_eq!(scene_requirements(ok).unwrap(), vec![Id::lit("cloud")]);
        assert!(scene_requirements(r#"{"format":"other","version":1,"objects":[]}"#).is_err());
        assert!(
            scene_requirements(r#"{"format":"tessel-scene","version":2,"objects":[]}"#).is_err()
        );
        assert!(scene_requirements("not json").is_err());
        assert!(
            scene_requirements(
                r#"{"format":"tessel-scene","version":1,"objects":[{"type":"splat"}]}"#
            )
            .is_err()
        );
    }

    #[test]
    fn kind_parsing_accepts_the_api_spelling_of_image() {
        assert_eq!(MediaKind::parse("image"), Some(MediaKind::Still));
        assert_eq!(MediaKind::parse("splat"), Some(MediaKind::Splat));
        assert_eq!(MediaKind::parse("gif"), None);
    }
}
