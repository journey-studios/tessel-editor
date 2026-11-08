//! Composition plan: what the monitor or an offline render has to draw at one instant.
//!
//! The plan is the single resolution of the project state into layers, source times,
//! transforms, cameras and audio segments. Backends (the Three.js runtime, the offline
//! mixer) receive it and apply no editing rules of their own. It has no GPUI/DOM
//! dependency and is built from `&EditorState` only, so the desktop monitor, the
//! headless CLI and a Rust caller derive identical plans for the same request.
//!
//! IO note: the planner never touches the filesystem, except to read a source audio file
//! when a waveform preview is requested (`audio::waveform_peaks`).

use serde_json::{Value, json};

use crate::assets::{Asset, AudioStream, MediaKind, Source};
use crate::audio::{AudioPlan, AudioSegment, resolve_fades};
use crate::content::{Camera, TextSpec};
use crate::id::Id;
use crate::state::inspector::{self, Value as PropertyValue};
use crate::state::library::MediaItem;
use crate::state::timeline::{Clip, ClipKind, TrackKind};
use crate::state::{CommandError, EditorState};
use crate::time::{Rate, Ratio};

pub const PLAN_VERSION: u64 = 1;
/// Output rate of the offline mix.
pub const MIX_SAMPLE_RATE: u32 = 48_000;
/// Buckets of a waveform preview.
pub const WAVEFORM_BUCKETS: usize = 96;

/// What the single monitor is asked to show.
#[derive(Debug, Clone, PartialEq)]
pub enum PreviewIntent {
    /// The sequence at the playhead (or the frame given to the planner).
    Sequence,
    /// One asset at a source time, isolated from the sequence.
    Source { asset: Id, time: Ratio },
}

impl PreviewIntent {
    pub fn to_json(&self) -> Value {
        match self {
            Self::Sequence => json!({ "mode": "sequence" }),
            Self::Source { asset, time } => {
                json!({ "mode": "source", "asset": asset, "time": time.to_json() })
            }
        }
    }

    pub fn from_json(value: &Value) -> Result<Self, CommandError> {
        match value["mode"].as_str() {
            Some("sequence") => Ok(Self::Sequence),
            Some("source") => {
                let asset = value["asset"].as_str().ok_or_else(|| {
                    CommandError::new("invalid_request", "A source preview needs `asset`")
                })?;
                let asset = Id::new(asset)
                    .map_err(|error| CommandError::new("invalid_id", error.to_string()))?;
                let time = match value.get("time") {
                    None | Some(Value::Null) => Ratio::ZERO,
                    Some(raw) => Ratio::from_json(raw).ok_or_else(|| {
                        CommandError::invalid_value(
                            "`time` is seconds as {num, den}",
                            json!({ "format": {"num": 1, "den": 24} }),
                        )
                    })?,
                };
                Ok(Self::Source { asset, time })
            }
            _ => Err(CommandError::invalid_value(
                "`mode` is `sequence` or `source`",
                json!({ "options": ["sequence", "source"] }),
            )),
        }
    }
}

/// When to evaluate a sequence preview.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlanTime {
    Playhead,
    Frame(i64),
    Time(Ratio),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlanRequest {
    pub intent: PreviewIntent,
    pub at: PlanTime,
}

impl PlanRequest {
    pub fn of(state: &EditorState) -> Self {
        Self {
            intent: state.preview.clone(),
            at: PlanTime::Playhead,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Transform {
    /// Offset from the canvas center, percent of the canvas width.
    pub x: f64,
    /// Offset from the canvas center, percent of the canvas height (positive is down).
    pub y: f64,
    /// 1.0 is the "contain" fit of the source inside the canvas.
    pub scale: f64,
    /// Clockwise degrees.
    pub rotation: f64,
    /// Percent of the source removed at left, right, top, bottom.
    pub crop: [f64; 4],
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            scale: 1.0,
            rotation: 0.0,
            crop: [0.0; 4],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    Image,
    Video {
        source_time: Ratio,
        source_frame: i64,
    },
    Text(TextSpec),
    Scene {
        camera: Camera,
        time: Ratio,
        /// Degrees around X, Y, Z (`XYZ` order) applied to the splat of a bare splat layer: the
        /// SHARP axes (BL143). Zero for scene files and plain splats.
        orientation: [f64; 3],
    },
    Waveform {
        peaks: Vec<f32>,
        position: f64,
    },
}

impl Content {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video { .. } => "video",
            Self::Text(_) => "text",
            Self::Scene { .. } => "scene",
            Self::Waveform { .. } => "waveform",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub id: String,
    pub clip: Option<Id>,
    pub asset: Option<Id>,
    pub content: Content,
    pub transform: Transform,
    pub opacity: f64,
    pub blend: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Warning {
    pub code: &'static str,
    pub message: String,
    pub clip: Option<Id>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderPlan {
    pub intent: PreviewIntent,
    pub width: u32,
    pub height: u32,
    /// Sequence frame for sequence previews.
    pub frame: Option<i64>,
    /// Sequence time, or source time for a source preview.
    pub time: Ratio,
    pub rate: Rate,
    /// Bottom to top.
    pub layers: Vec<Layer>,
    /// Assets that must be loaded by the backend, dependencies included.
    pub requires: Vec<Id>,
    pub warnings: Vec<Warning>,
}

impl RenderPlan {
    pub fn to_json(&self) -> Value {
        json!({
            "version": PLAN_VERSION,
            "intent": self.intent.to_json(),
            "canvas": { "width": self.width, "height": self.height, "background": [0.0, 0.0, 0.0, 1.0] },
            "frame": self.frame,
            "time": self.time.to_json(),
            "rate": self.rate.to_json(),
            "layers": self.layers.iter().map(layer_json).collect::<Vec<_>>(),
            "requires": self.requires.iter().map(|id| id.as_str()).collect::<Vec<_>>(),
            "warnings": self.warnings.iter().map(|warning| json!({
                "code": warning.code, "message": warning.message, "clip": warning.clip,
            })).collect::<Vec<_>>(),
        })
    }
}

fn layer_json(layer: &Layer) -> Value {
    let transform = &layer.transform;
    let mut value = json!({
        "id": layer.id,
        "kind": layer.content.kind(),
        "clip": layer.clip,
        "asset": layer.asset,
        "opacity": layer.opacity,
        "blend": layer.blend,
        "transform": {
            "x": transform.x, "y": transform.y, "scale": transform.scale,
            "rotation": transform.rotation, "crop": transform.crop,
        },
    });
    match &layer.content {
        Content::Image => {}
        Content::Video {
            source_time,
            source_frame,
        } => {
            value["source_time"] = source_time.to_json();
            value["source_frame"] = json!(source_frame);
        }
        Content::Text(text) => value["text"] = text.to_json(),
        Content::Scene {
            camera,
            time,
            orientation,
        } => {
            value["camera"] = camera.to_json();
            value["scene_time"] = time.to_json();
            if *orientation != [0.0; 3] {
                value["orientation"] = json!(orientation);
            }
        }
        Content::Waveform { peaks, position } => {
            value["peaks"] = json!(peaks);
            value["position"] = json!(position);
        }
    }
    value
}

fn invalid_time(message: String, details: Value) -> CommandError {
    CommandError::new("invalid_time", message).details(details)
}

struct AudibleClip<'a> {
    clip: &'a Clip,
    item: &'a MediaItem,
    asset: &'a Asset,
    /// The sound is the clip's own (a container video) rather than an audio-track clip's.
    embedded: bool,
    stream: AudioStream,
    start: i64,
    end: i64,
}

/// A stream an audible clip plays, as the adapters see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioSource {
    pub clip: Id,
    pub asset: Id,
    pub stream: u32,
    /// The clip's own sound (a container video) rather than an audio-track clip.
    pub embedded: bool,
}

struct Resolver<'a> {
    state: &'a EditorState,
    requires: Vec<Id>,
    warnings: Vec<Warning>,
}

impl Resolver<'_> {
    fn require(&mut self, item: &MediaItem) {
        if !self.requires.contains(&item.id) {
            if let Some(asset) = &item.asset {
                for dependency in &asset.meta.requires {
                    if let Some(dep) = self.state.library.item(dependency) {
                        self.require(dep);
                    }
                }
            }
            self.requires.push(item.id);
        }
    }

    fn number(&self, clip: &Clip, property: &str) -> f64 {
        let Some(def) = inspector::property_def(property) else {
            return 0.0;
        };
        match self.state.inspector.value(clip.id, def) {
            PropertyValue::Number(number) => number,
            _ => 0.0,
        }
    }

    fn flag(&self, clip: &Clip, property: &str) -> bool {
        inspector::property_def(property).is_some_and(|def| {
            self.state.inspector.value(clip.id, def) == PropertyValue::Bool(true)
        })
    }

    fn choice(&self, clip: &Clip, property: &str) -> String {
        inspector::property_def(property)
            .map(|def| match self.state.inspector.value(clip.id, def) {
                PropertyValue::Choice(choice) => choice.to_string(),
                _ => String::new(),
            })
            .unwrap_or_default()
    }

    /// Playback speed of a clip as a ratio (`100 %` is 1): the one place the picture and the sound
    /// read it, so they cannot drift apart.
    fn speed_ratio(&self, clip: &Clip) -> Ratio {
        let percent = self.number(clip, "speed").round() as i64;
        Ratio::new(percent.max(1), 100).unwrap_or(Ratio::from_int(1))
    }

    fn speed(&self, clip: &Clip) -> f64 {
        self.speed_ratio(clip).to_f64()
    }

    fn source_time(&self, clip: &Clip, frame: i64) -> Ratio {
        let timeline = &self.state.timeline;
        let elapsed = timeline.rate.frame_to_time(frame - clip.start);
        let speed = self.speed_ratio(clip);
        let scaled = elapsed.mul(speed).unwrap_or(elapsed);
        clip.source_start.add(scaled).unwrap_or(clip.source_start)
    }

    /// Fade ramps apply to opacity, evaluated at the frame being drawn.
    fn opacity(&self, clip: &Clip, frame: i64) -> f64 {
        let local = (frame - clip.start) as f64;
        let remaining = (clip.duration - (frame - clip.start)) as f64;
        let fade_in = self.number(clip, "fade_in");
        let fade_out = self.number(clip, "fade_out");
        let mut opacity = self.number(clip, "opacity") / 100.0;
        if fade_in > 0.0 {
            opacity *= (local / fade_in).min(1.0);
        }
        if fade_out > 0.0 {
            opacity *= (remaining / fade_out).min(1.0);
        }
        opacity.clamp(0.0, 1.0)
    }

    fn transform(&self, clip: &Clip, frame: i64, ken_burns: bool) -> Transform {
        let mut zoom = self.number(clip, "zoom");
        if ken_burns && self.flag(clip, "ken_burns") {
            let span = (clip.duration - 1).max(1) as f64;
            let progress = ((frame - clip.start) as f64 / span).clamp(0.0, 1.0);
            zoom += (self.number(clip, "end_zoom") - zoom) * progress;
        }
        Transform {
            x: self.number(clip, "position_x"),
            y: self.number(clip, "position_y"),
            scale: zoom / 100.0,
            rotation: self.number(clip, "rotation"),
            crop: [
                self.number(clip, "crop_left"),
                self.number(clip, "crop_right"),
                self.number(clip, "crop_top"),
                self.number(clip, "crop_bottom"),
            ],
        }
    }

    fn asset_layer(
        &mut self,
        item: &MediaItem,
        asset: &Asset,
        source_time: Ratio,
        scene_camera: Camera,
        id: String,
        clip: Option<&Clip>,
    ) -> Result<Layer, CommandError> {
        let content = match item.kind {
            MediaKind::Still => Content::Image,
            MediaKind::Video => {
                let rate = asset.meta.rate.ok_or_else(|| {
                    CommandError::new(
                        "asset_unavailable",
                        format!("`{}` has no frame rate", item.id),
                    )
                })?;
                let count = match &asset.source {
                    Source::FrameSequence { count, .. } => Some(*count),
                    _ => asset
                        .meta
                        .duration
                        .map(|duration| rate.time_to_frame_floor(duration).max(1)),
                };
                let mut frame = rate.time_to_frame_floor(source_time).max(0);
                if let Some(count) = count
                    && frame >= count
                {
                    self.warnings.push(Warning {
                        code: "source_end_hold",
                        message: format!(
                            "Source frame {frame} is past the end of `{}`; holding frame {}",
                            item.id,
                            count - 1
                        ),
                        clip: clip.map(|clip| clip.id),
                    });
                    frame = count - 1;
                }
                Content::Video {
                    source_time,
                    source_frame: frame,
                }
            }
            MediaKind::Scene | MediaKind::Splat => {
                let capture = asset.meta.capture.filter(|_| item.kind == MediaKind::Splat);
                Content::Scene {
                    camera: scene_camera,
                    time: source_time,
                    orientation: capture.map_or([0.0; 3], |_| crate::sharp::ORIENTATION),
                }
            }
            MediaKind::Text => match &asset.meta.text {
                Some(text) => Content::Text(text.clone()),
                None => {
                    return Err(CommandError::new(
                        "asset_unavailable",
                        format!("Text asset `{}` has no text", item.id),
                    ));
                }
            },
            MediaKind::Audio | MediaKind::Timeline => {
                return Err(CommandError::new(
                    "invalid_preview",
                    format!("`{}` has no picture", item.id),
                ));
            }
        };
        self.require(item);
        Ok(Layer {
            id,
            clip: clip.map(|clip| clip.id),
            asset: Some(item.id),
            content,
            transform: Transform::default(),
            opacity: 1.0,
            blend: "normal".to_string(),
        })
    }
}

impl EditorState {
    /// Resolves `request` into a plan. Never mutates the state: the playhead, selection
    /// and preview intent stay as they are.
    pub fn plan(&self, request: &PlanRequest) -> Result<RenderPlan, CommandError> {
        match &request.intent {
            PreviewIntent::Sequence => self.plan_sequence(request.at),
            PreviewIntent::Source { asset, time } => self.plan_source(asset, *time),
        }
    }

    fn plan_sequence(&self, at: PlanTime) -> Result<RenderPlan, CommandError> {
        let timeline = &self.timeline;
        let frame = match at {
            PlanTime::Playhead => timeline.playhead,
            PlanTime::Frame(frame) => frame,
            PlanTime::Time(time) => timeline.rate.time_to_frame_floor(time),
        };
        if !(0..=timeline.length).contains(&frame) {
            return Err(invalid_time(
                format!(
                    "Frame {frame} is outside the sequence (0..{})",
                    timeline.length
                ),
                json!({ "frame": frame, "length": timeline.length }),
            ));
        }
        let mut resolver = Resolver {
            state: self,
            requires: Vec::new(),
            warnings: Vec::new(),
        };
        let mut layers = Vec::new();
        // Tracks are stored top to bottom, the plan is bottom to top.
        for track in timeline.tracks.iter().rev() {
            if track.kind != TrackKind::Video || track.hidden {
                continue;
            }
            let Some(clip) = timeline
                .clips_on(&track.id)
                .find(|clip| clip.start <= frame && frame < clip.end())
            else {
                continue;
            };
            if let Some(layer) = self.clip_layer(&mut resolver, clip, frame)? {
                layers.push(layer);
            }
        }
        Ok(RenderPlan {
            intent: PreviewIntent::Sequence,
            width: timeline.width,
            height: timeline.height,
            frame: Some(frame),
            time: timeline.time_of(frame),
            rate: timeline.rate,
            layers,
            requires: resolver.requires,
            warnings: resolver.warnings,
        })
    }

    fn clip_layer(
        &self,
        resolver: &mut Resolver<'_>,
        clip: &Clip,
        frame: i64,
    ) -> Result<Option<Layer>, CommandError> {
        let layer_id = format!("layer:{}", clip.id);
        let mut layer = match clip.kind {
            ClipKind::Title => {
                let Some(text) = clip.text.clone() else {
                    resolver.warnings.push(Warning {
                        code: "title_without_text",
                        message: format!("Title `{}` has no text and is skipped", clip.id),
                        clip: Some(clip.id),
                    });
                    return Ok(None);
                };
                Layer {
                    id: layer_id,
                    clip: Some(clip.id),
                    asset: None,
                    content: Content::Text(text),
                    transform: Transform::default(),
                    opacity: 1.0,
                    blend: "normal".to_string(),
                }
            }
            ClipKind::Audio => return Ok(None),
            ClipKind::Video | ClipKind::Scene => {
                let item = clip
                    .media
                    .and_then(|media| self.library.item(media))
                    .ok_or_else(|| {
                        CommandError::new(
                            "asset_missing",
                            format!(
                                "Clip `{}` refers to an asset that is not in the library",
                                clip.id
                            ),
                        )
                        .target(format!("clip:{}", clip.id))
                    })?;
                let Some(asset) = &item.asset else {
                    resolver.warnings.push(Warning {
                        code: "asset_unavailable",
                        message: format!("`{}` has no media behind it and is skipped", item.id),
                        clip: Some(clip.id),
                    });
                    return Ok(None);
                };
                let camera = clip.camera.unwrap_or_default();
                let source_time = resolver.source_time(clip, frame);
                let mut layer =
                    resolver.asset_layer(item, asset, source_time, camera, layer_id, Some(clip))?;
                if let Content::Scene { camera, .. } = &mut layer.content {
                    *camera = self.clip_camera_at(clip, frame);
                }
                layer
            }
        };
        let still = matches!(layer.content, Content::Image);
        layer.transform = resolver.transform(clip, frame, still);
        layer.opacity = resolver.opacity(clip, frame);
        layer.blend = resolver.choice(clip, "blend");
        Ok(Some(layer))
    }

    /// Scene time (exact seconds) a clip reads at sequence `frame`: `source_start` plus the
    /// elapsed clip time at its speed. The plan reports it as `scene_time`; camera keys use it.
    pub fn clip_source_time(&self, clip: &Clip, frame: i64) -> Ratio {
        Resolver {
            state: self,
            requires: Vec::new(),
            warnings: Vec::new(),
        }
        .source_time(clip, frame)
    }

    /// Playback speed of `clip` as an exact ratio (`1` is 100 %): the value the picture, the
    /// sound and the camera keys all read, and the one a head trim scales by.
    pub fn clip_speed(&self, clip: &Clip) -> Ratio {
        Resolver {
            state: self,
            requires: Vec::new(),
            warnings: Vec::new(),
        }
        .speed_ratio(clip)
    }

    /// The camera a scene clip shows at sequence `frame`: its keyframes (BL140) at scene time, or
    /// without keys the still/orbiting camera counted from the clip start. One function for the
    /// plan (so preview and export agree) and for capturing a key at the playhead.
    pub fn clip_camera_at(&self, clip: &Clip, frame: i64) -> Camera {
        let source_time = self.clip_source_time(clip, frame);
        if let Some(camera) = crate::camera_keys::evaluate(&clip.camera_keys, source_time.to_f64())
        {
            return camera;
        }
        let seconds = source_time
            .sub(clip.source_start)
            .unwrap_or(Ratio::ZERO)
            .to_f64();
        clip.camera.unwrap_or_default().at(seconds)
    }

    fn plan_source(&self, asset_id: &Id, time: Ratio) -> Result<RenderPlan, CommandError> {
        let item = self
            .library
            .item(asset_id)
            .ok_or_else(|| CommandError::unknown_element("media", asset_id))?;
        let timeline = &self.timeline;
        if time.is_negative() {
            return Err(invalid_time(
                "Source time cannot be negative".to_string(),
                json!({ "time": time.to_json() }),
            ));
        }
        let Some(asset) = &item.asset else {
            return Err(CommandError::new(
                "asset_unavailable",
                format!("`{}` has no media behind it (demonstration entry)", item.id),
            )
            .target(asset_id.as_str()));
        };
        if let Some(duration) = asset.meta.duration
            && time > duration
        {
            return Err(invalid_time(
                format!("Source time {time} is past the end of `{asset_id}` ({duration} s)"),
                json!({ "time": time.to_json(), "duration": duration.to_json() }),
            ));
        }
        let mut resolver = Resolver {
            state: self,
            requires: Vec::new(),
            warnings: Vec::new(),
        };
        let layer = if item.kind == MediaKind::Audio {
            let path = match &asset.source {
                Source::File { path } => path.clone(),
                _ => {
                    return Err(CommandError::new(
                        "asset_unavailable",
                        format!("`{asset_id}` has no readable audio file"),
                    ));
                }
            };
            let peaks =
                crate::audio::waveform_peaks(&path, WAVEFORM_BUCKETS).map_err(|message| {
                    CommandError::new("decode_failed", message).target(asset_id.as_str())
                })?;
            let duration = asset
                .meta
                .duration
                .map_or(1.0, Ratio::to_f64)
                .max(f64::EPSILON);
            resolver.require(item);
            Layer {
                id: format!("layer:source:{asset_id}"),
                clip: None,
                asset: Some(item.id),
                content: Content::Waveform {
                    peaks,
                    position: (time.to_f64() / duration).clamp(0.0, 1.0),
                },
                transform: Transform::default(),
                opacity: 1.0,
                blend: "normal".to_string(),
            }
        } else {
            // A SHARP splat is previewed from the camera that took its picture (BL143).
            let camera = asset
                .meta
                .capture
                .map_or_else(Camera::default, |capture| capture.camera());
            resolver.asset_layer(
                item,
                asset,
                time,
                camera.at(time.to_f64()),
                format!("layer:source:{asset_id}"),
                None,
            )?
        };
        Ok(RenderPlan {
            intent: PreviewIntent::Source {
                asset: *asset_id,
                time,
            },
            width: timeline.width,
            height: timeline.height,
            frame: None,
            time,
            rate: timeline.rate,
            layers: vec![layer],
            requires: resolver.requires,
            warnings: resolver.warnings,
        })
    }

    /// The clips that make sound in `[from, to)`, with the stream each plays. The one selection
    /// rule for the mix, the playback and the export preflight, so they cannot disagree about
    /// what is audible: audio clips on unmuted audio tracks, and the own sound of container
    /// videos on any track; a clip with `audio_muted` is silent, hiding a video track is not.
    fn audible_clips<'a>(
        &'a self,
        from: i64,
        to: i64,
        resolver: &Resolver<'_>,
    ) -> Result<Vec<AudibleClip<'a>>, CommandError> {
        let timeline = &self.timeline;
        let mut audible = Vec::new();
        for track in &timeline.tracks {
            // `muted` is an audio-track flag; a video track carries sound through its clips.
            if track.kind == TrackKind::Audio && track.muted {
                continue;
            }
            for clip in timeline.clips_on(&track.id) {
                let (start, end) = (clip.start.max(from), clip.end().min(to));
                if start >= end || resolver.flag(clip, "audio_muted") {
                    continue;
                }
                let embedded = match clip.kind {
                    ClipKind::Audio => false,
                    ClipKind::Video => true,
                    _ => continue,
                };
                let Some(item) = clip.media.and_then(|media| self.library.item(media)) else {
                    continue;
                };
                let Some(asset) = item.asset.as_ref() else {
                    continue;
                };
                // Frame sequences and stills have no sound of their own.
                if !matches!(asset.source, Source::File { .. }) {
                    continue;
                }
                let index = resolver.number(clip, "audio_stream").round() as u32;
                let Some(stream) = asset.meta.audio_stream(index) else {
                    // A video without sound is silent; asking for a stream that does not exist
                    // is an error, never silence.
                    if embedded && index == 0 {
                        continue;
                    }
                    return Err(CommandError::new(
                        "audio_stream_missing",
                        format!(
                            "`{}` has no audio stream {index} ({} found)",
                            item.id,
                            asset.meta.audio_streams.len()
                        ),
                    )
                    .target(item.id.as_str()));
                };
                audible.push(AudibleClip {
                    clip,
                    item,
                    asset,
                    embedded,
                    stream,
                    start,
                    end,
                });
            }
        }
        Ok(audible)
    }

    /// Which stream of which asset each audible clip in `[from, to)` plays: what an adapter must
    /// decode before the mix can run. Same selection as [`EditorState::plan_audio`].
    pub fn audio_sources(&self, from: i64, to: i64) -> Result<Vec<AudioSource>, CommandError> {
        let resolver = Resolver {
            state: self,
            requires: Vec::new(),
            warnings: Vec::new(),
        };
        Ok(self
            .audible_clips(from.max(0), to.min(self.timeline.length), &resolver)?
            .into_iter()
            .map(|audible| AudioSource {
                clip: audible.clip.id,
                asset: audible.item.id,
                stream: audible.stream.index,
                embedded: audible.embedded,
            })
            .collect())
    }

    /// Audio segments for `frames` sequence frames starting at `from`, mixed at
    /// [`MIX_SAMPLE_RATE`]. Muted tracks are skipped; clip volume (dB), pan and fades apply.
    pub fn plan_audio(&self, from: i64, frames: i64) -> Result<AudioPlan, CommandError> {
        let timeline = &self.timeline;
        if from < 0 || frames <= 0 || from + frames > timeline.length {
            return Err(invalid_time(
                format!(
                    "Audio range {from}..{} is outside the sequence (0..{})",
                    from + frames,
                    timeline.length
                ),
                json!({ "from": from, "frames": frames, "length": timeline.length }),
            ));
        }
        let resolver = Resolver {
            state: self,
            requires: Vec::new(),
            warnings: Vec::new(),
        };
        let rate = timeline.rate;
        // Output sample index of a sequence frame boundary. Rational, so frames never drift.
        let sample_at = |frame: i64| -> u64 {
            let seconds = rate.frame_to_time(frame - from);
            seconds
                .mul_int(i64::from(MIX_SAMPLE_RATE))
                .map_or(0, Ratio::round)
                .max(0) as u64
        };
        let total = sample_at(from + frames);
        let mut segments = Vec::new();
        for audible in self.audible_clips(from, from + frames, &resolver)? {
            let AudibleClip {
                clip,
                item,
                asset,
                embedded,
                stream,
                start,
                end,
            } = audible;
            let Source::File { path } = &asset.source else {
                continue;
            };
            // Compressed audio and the sound of videos are mixed from the adapter's WAV; a WAV
            // original is mixed as is.
            let decoded_audio = asset
                .decoded
                .as_ref()
                .and_then(|decoded| decoded.audio_path(stream.index));
            let direct_wav =
                !embedded && stream.index == 0 && asset.meta.format.as_deref() == Some("wav");
            let path = match decoded_audio {
                Some(decoded) => decoded,
                None if direct_wav => path,
                None => {
                    return Err(CommandError::new(
                        "asset_not_decoded",
                        format!(
                            "`{}` is `{}`: its audio is not decoded yet (export, playback and the \
                             media jobs prepare it automatically)",
                            item.id,
                            asset.meta.format.as_deref().unwrap_or("unknown")
                        ),
                    )
                    .target(item.id.as_str()));
                }
            };
            // Where the first output sample reads: the clip's source position (trim and speed
            // already applied) on the asset's clock, minus where the stream starts on it.
            let source_time = resolver
                .source_time(clip, start)
                .sub(stream.offset)
                .unwrap_or(Ratio::ZERO);
            let source_start = source_time
                .mul_int(i64::from(stream.sample_rate))
                .map_or(0, Ratio::round);
            let output_start = sample_at(start);
            let output_end = sample_at(end);
            let samples_of = |frames: i64| -> u64 {
                rate.frame_to_time(frames)
                    .mul_int(i64::from(MIX_SAMPLE_RATE))
                    .map_or(0, Ratio::round)
                    .max(0) as u64
            };
            // The fades of an audio clip are relative to the clip (its current start and end,
            // so a trim or a move changes where they sit), not to the requested range. The
            // fades of a video clip are visual (opacity): its sound does not follow them.
            let clip_frames = samples_of(clip.end() - clip.start);
            let (fade_in, fade_out) = if embedded {
                (0, 0)
            } else {
                resolve_fades(
                    samples_of(resolver.number(clip, "fade_in").round() as i64),
                    samples_of(resolver.number(clip, "fade_out").round() as i64),
                    clip_frames,
                )
            };
            segments.push(AudioSegment {
                clip: clip.id,
                asset: item.id,
                path: path.to_string(),
                output_start,
                frames: output_end - output_start,
                source_start,
                source_rate: stream.sample_rate,
                speed: resolver.speed(clip),
                gain: 10f32.powf(resolver.number(clip, "volume") as f32 / 20.0),
                pan: (resolver.number(clip, "pan") / 100.0) as f32,
                fade_in,
                fade_out,
                clip_offset: if embedded {
                    0
                } else {
                    samples_of(start - clip.start)
                },
                clip_frames: if embedded { 0 } else { clip_frames },
            });
        }
        Ok(AudioPlan {
            sample_rate: MIX_SAMPLE_RATE,
            frames: total,
            segments,
        })
    }
}

#[cfg(test)]
mod tests;
