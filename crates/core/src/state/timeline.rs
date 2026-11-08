//! Sequence model: tracks, clips, playhead, zoom and snap.
//!
//! Gestures and the CLI call the same methods. Nothing here plays, decodes or
//! exports media; positions are in sequence frames of the timeline's [`Rate`].

use serde_json::{Value, json};

use super::error::CommandError;
use crate::camera_keys::CameraKey;
use crate::content::{Camera, TextSpec};
use crate::id::Id;
use crate::time::{Rate, Ratio};

/// Frame rate of the demonstration sequence. Dynamic projects read [`Timeline::rate`].
pub const FPS: i64 = 24;
/// Length of the demonstration sequence: 29 seconds at 24 fps. Dynamic projects read
/// [`Timeline::length`]. Clips cannot be moved past the length, and the playhead is
/// clamped to it.
pub const SEQUENCE_FRAMES: i64 = 29 * FPS;
pub const ZOOM_MIN: f64 = 8.0;
pub const ZOOM_MAX: f64 = 120.0;
pub const ZOOM_DEFAULT: f64 = 28.0;
/// Snap pulls an edge to a neighbour at most this many frames away.
pub const SNAP_FRAMES: i64 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Video,
    Audio,
}

impl TrackKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Audio => "audio",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipKind {
    /// Image or video asset on a video track.
    Video,
    Title,
    Audio,
    /// Three.js scene or Gaussian splat asset.
    Scene,
}

impl ClipKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Title => "title",
            Self::Audio => "audio",
            Self::Scene => "scene",
        }
    }

    /// Audio never lands on a video track and vice versa. Titles and scenes are visual.
    pub fn fits(self, track: TrackKind) -> bool {
        matches!(
            (self, track),
            (Self::Video | Self::Title | Self::Scene, TrackKind::Video)
                | (Self::Audio, TrackKind::Audio)
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub id: Id,
    pub name: String,
    pub kind: TrackKind,
    /// Video tracks can be hidden.
    pub hidden: bool,
    /// Audio tracks can be muted.
    pub muted: bool,
    /// Lane height in logical pixels. A density hint kept with the track so it
    /// follows the track when it is reordered.
    pub height: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub id: Id,
    pub name: String,
    pub kind: ClipKind,
    pub track: Id,
    pub start: i64,
    pub duration: i64,
    /// Library item shown by the monitor and used for thumbnails.
    pub media: Option<Id>,
    /// Where in the asset (seconds) the first frame of the clip reads. Trimming the
    /// start edge advances it so content stays anchored to the picture it showed.
    pub source_start: Ratio,
    /// Text of a title clip.
    pub text: Option<TextSpec>,
    /// Camera of a scene clip.
    pub camera: Option<Camera>,
    /// Keyframes of a scene clip's camera (BL140), sorted by scene time; empty = no keyframes.
    pub camera_keys: Vec<CameraKey>,
}

impl Clip {
    pub fn end(&self) -> i64 {
        self.start + self.duration
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrimEdge {
    Start,
    End,
}

#[derive(Debug, Clone)]
pub struct TrimPlan {
    pub start: i64,
    pub end: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Before,
    After,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackFlag {
    Hidden,
    Muted,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MovePlan {
    pub clip: Id,
    pub track: Id,
    pub start: i64,
    /// The requested start when snap changed it.
    pub snapped_from: Option<i64>,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReorderPlan {
    pub order: Vec<Id>,
    pub changed: bool,
}

#[derive(Debug, Clone)]
pub struct Timeline {
    /// The sequence ID used by `Sequence` previews.
    pub id: Id,
    /// Display name; defaults to the ID. Not an identity: two sequences may share a name.
    pub name: String,
    pub rate: Rate,
    /// Sequence length in frames at `rate`.
    pub length: i64,
    /// Canvas size of the sequence in pixels.
    pub width: u32,
    pub height: u32,
    /// Top to bottom, as drawn.
    pub tracks: Vec<Track>,
    pub clips: Vec<Clip>,
    pub playhead: i64,
    pub zoom: f64,
    pub snap: bool,
    pub selected_clip: Option<Id>,
    pub selected_track: Option<Id>,
}

impl Default for Timeline {
    /// The demonstration sequence. Prefer [`Timeline::demo`] in new code.
    fn default() -> Self {
        Self::demo()
    }
}

/// Settings a project is created with.
#[derive(Debug, Clone, PartialEq)]
pub struct SequenceSettings {
    pub id: Id,
    pub rate: Rate,
    pub length: i64,
    pub width: u32,
    pub height: u32,
}

impl Default for SequenceSettings {
    fn default() -> Self {
        Self {
            id: Id::lit("main"),
            rate: Rate::FPS_24,
            length: 10 * FPS,
            width: 1920,
            height: 1080,
        }
    }
}

pub const MAX_CANVAS: u32 = 8192;
pub const MAX_LENGTH_FRAMES: i64 = 1_000_000;

impl SequenceSettings {
    pub fn validate(&self) -> Result<(), CommandError> {
        if !(1..=MAX_CANVAS).contains(&self.width) || !(1..=MAX_CANVAS).contains(&self.height) {
            return Err(CommandError::invalid_value(
                "Canvas width and height are between 1 and 8192",
                json!({ "max": MAX_CANVAS }),
            ));
        }
        if !(1..=MAX_LENGTH_FRAMES).contains(&self.length) {
            return Err(CommandError::invalid_value(
                "Sequence length is between 1 and 1000000 frames",
                json!({ "max": MAX_LENGTH_FRAMES }),
            ));
        }
        Ok(())
    }
}

impl Timeline {
    /// An empty sequence: no tracks, no clips, playhead at frame 0.
    pub fn empty(settings: &SequenceSettings) -> Self {
        Self {
            id: settings.id,
            name: settings.id.to_string(),
            rate: settings.rate,
            length: settings.length,
            width: settings.width,
            height: settings.height,
            tracks: Vec::new(),
            clips: Vec::new(),
            playhead: 0,
            zoom: ZOOM_DEFAULT,
            snap: true,
            selected_clip: None,
            selected_track: None,
        }
    }

    /// Bootstrap of the illustrative "North expedition" sequence.
    pub fn demo() -> Self {
        let track = |id: &str, name: &str, kind, height| Track {
            id: Id::lit(id),
            name: name.to_string(),
            kind,
            hidden: false,
            muted: false,
            height,
        };
        let clip =
            |id: &str, name: &str, kind, track: &str, start, seconds: i64, media: Option<&str>| {
                Clip {
                    id: Id::lit(id),
                    name: name.to_string(),
                    kind,
                    track: Id::lit(track),
                    start,
                    duration: seconds * FPS,
                    media: media.map(Id::lit),
                    source_start: Ratio::ZERO,
                    text: None,
                    camera: None,
                    camera_keys: Vec::new(),
                }
            };
        let mut title = clip(
            "title-north",
            "Text+ · NORTH",
            ClipKind::Title,
            "titles",
            0,
            5,
            None,
        );
        title.text = TextSpec::new("NORTH", 14.0, "#ffffff").ok();
        Self {
            id: Id::lit("main"),
            name: "Master sequence".to_string(),
            rate: Rate::FPS_24,
            length: SEQUENCE_FRAMES,
            width: 1920,
            height: 1080,
            tracks: vec![
                track("titles", "Titles", TrackKind::Video, 36.0),
                track("picture", "Picture", TrackKind::Video, 72.0),
                track("score", "Score", TrackKind::Audio, 40.0),
                track("ambience", "Ambience", TrackKind::Audio, 40.0),
            ],
            clips: vec![
                title,
                clip(
                    "first-light",
                    "First light",
                    ClipKind::Video,
                    "picture",
                    0,
                    8,
                    Some("ao01"),
                ),
                clip(
                    "atlantic-coast",
                    "Atlantic coast",
                    ClipKind::Video,
                    "picture",
                    8 * FPS,
                    7,
                    Some("ao02"),
                ),
                clip(
                    "river-detail",
                    "River detail",
                    ClipKind::Video,
                    "picture",
                    15 * FPS,
                    6,
                    Some("ao03"),
                ),
                clip(
                    "basalt-cliffs",
                    "Basalt cliffs",
                    ClipKind::Video,
                    "picture",
                    21 * FPS,
                    8,
                    Some("ao04"),
                ),
                clip(
                    "score-clip",
                    "North / original score",
                    ClipKind::Audio,
                    "score",
                    0,
                    29,
                    Some("original-score"),
                ),
                clip(
                    "ambience-clip",
                    "Atlantic / surf texture",
                    ClipKind::Audio,
                    "ambience",
                    0,
                    29,
                    Some("surf-texture"),
                ),
            ],
            playhead: 3 * FPS + 14,
            zoom: ZOOM_DEFAULT,
            snap: true,
            selected_clip: Some(Id::lit("atlantic-coast")),
            selected_track: Some(Id::lit("picture")),
        }
    }

    /// Whole frames per second, for timecode and ruler ticks.
    pub fn fps(&self) -> i64 {
        self.rate.nominal_fps()
    }

    /// Start time of a sequence frame, in seconds.
    pub fn time_of(&self, frame: i64) -> Ratio {
        self.rate.frame_to_time(frame)
    }

    pub fn timecode(&self, frame: i64) -> String {
        timecode_at(frame, self.fps())
    }

    /// Smallest valid length: the end of the last clip.
    pub fn content_end(&self) -> i64 {
        self.clips.iter().map(Clip::end).max().unwrap_or(0)
    }

    /// An ID of the form `<prefix>-<n>` that no track or clip uses.
    pub fn fresh_id(&self, prefix: &str) -> Id {
        (1..)
            .filter_map(|n| Id::new(&format!("{prefix}-{n}")).ok())
            .find(|id| self.track(id).is_none() && self.clip(id).is_none())
            .unwrap_or_else(|| Id::lit(prefix))
    }
}

/// Timecode at the demonstration rate; dynamic sequences use [`Timeline::timecode`].
pub fn timecode(frame: i64) -> String {
    timecode_at(frame, FPS)
}

fn timecode_at(frame: i64, fps: i64) -> String {
    let frame = frame.max(0);
    let seconds = frame / fps;
    format!(
        "{:02}:{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60,
        frame % fps
    )
}

impl Timeline {
    pub fn track(&self, id: impl AsRef<str>) -> Option<&Track> {
        let id = id.as_ref();
        self.tracks.iter().find(|track| track.id == id)
    }

    pub fn clip(&self, id: impl AsRef<str>) -> Option<&Clip> {
        let id = id.as_ref();
        self.clips.iter().find(|clip| clip.id == id)
    }

    pub fn clips_on<'a>(&'a self, track: &'a str) -> impl Iterator<Item = &'a Clip> {
        self.clips.iter().filter(move |clip| clip.track == track)
    }

    pub fn pixels_per_frame(&self) -> f32 {
        self.zoom as f32 / self.fps() as f32
    }

    /// V-tracks count upward from the bottom-most video track, A-tracks
    /// downward from the first audio track, so the label follows the position
    /// after a reorder while the ID stays stable.
    pub fn label(&self, id: impl AsRef<str>) -> String {
        let id = id.as_ref();
        let Some(index) = self.tracks.iter().position(|track| track.id == id) else {
            return String::new();
        };
        let kind = self.tracks[index].kind;
        match kind {
            TrackKind::Video => {
                let below = self.tracks[index + 1..]
                    .iter()
                    .filter(|track| track.kind == kind)
                    .count();
                format!("V{}", below + 1)
            }
            TrackKind::Audio => {
                let above = self.tracks[..index]
                    .iter()
                    .filter(|track| track.kind == kind)
                    .count();
                format!("A{}", above + 1)
            }
        }
    }

    pub fn set_zoom(&mut self, zoom: f64) -> Value {
        let bounded = zoom.clamp(ZOOM_MIN, ZOOM_MAX).round();
        let previous = std::mem::replace(&mut self.zoom, bounded);
        json!({ "target": "timeline.zoom", "field": "value", "value": bounded,
            "unit": "px/s", "previous": previous, "clamped": bounded != zoom,
            "changed": previous != bounded })
    }

    pub fn set_snap(&mut self, snap: bool) -> Value {
        let previous = std::mem::replace(&mut self.snap, snap);
        json!({ "target": "timeline.snap", "field": "value", "value": snap,
            "previous": previous, "changed": previous != snap })
    }

    /// Seeks, clamping to the sequence. The clamp is reported, not an error:
    /// scrubbing past an end is a normal gesture.
    pub fn set_playhead(&mut self, frame: i64) -> Value {
        let bounded = frame.clamp(0, self.length);
        let previous = std::mem::replace(&mut self.playhead, bounded);
        json!({ "target": "timeline.playhead", "field": "frame", "value": bounded,
            "timecode": self.timecode(bounded), "fps": self.fps(), "max": self.length,
            "time": self.time_of(bounded).to_json(),
            "previous": previous, "clamped": bounded != frame, "changed": previous != bounded })
    }

    pub fn select_clip(&mut self, id: &str) -> Result<Value, CommandError> {
        let clip = self
            .clip(id)
            .ok_or_else(|| CommandError::unknown_element("clip", id))?;
        let (clip_id, track) = (clip.id, clip.track);
        let previous = self.selected_clip.replace(clip_id);
        self.selected_track = Some(track);
        Ok(json!({ "target": format!("clip:{id}"), "action": "select",
            "track": format!("track:{track}"), "changed": previous != Some(clip_id) }))
    }

    pub fn select_track(&mut self, id: &str) -> Result<Value, CommandError> {
        let track = self
            .track(id)
            .ok_or_else(|| CommandError::unknown_element("track", id))?
            .id;
        let previous = self.selected_track.replace(track);
        // A clip selection from another track would contradict the highlight.
        if self
            .selected_clip
            .and_then(|clip| self.clip(clip))
            .is_some_and(|clip| clip.track != track)
        {
            self.selected_clip = None;
        }
        Ok(json!({ "target": format!("track:{id}"), "action": "select",
            "changed": previous != Some(track) }))
    }

    pub fn set_track_flag(
        &mut self,
        id: &str,
        flag: TrackFlag,
        value: bool,
    ) -> Result<Value, CommandError> {
        let track = self
            .tracks
            .iter_mut()
            .find(|track| track.id == id)
            .ok_or_else(|| CommandError::unknown_element("track", id))?;
        let (field, supported, slot) = match flag {
            TrackFlag::Hidden => ("hidden", track.kind == TrackKind::Video, &mut track.hidden),
            TrackFlag::Muted => ("muted", track.kind == TrackKind::Audio, &mut track.muted),
        };
        if !supported {
            return Err(CommandError::new(
                "unknown_field",
                format!(
                    "`{field}` is not available on {} tracks",
                    track.kind.label()
                ),
            )
            .target(format!("track:{id}")));
        }
        let previous = std::mem::replace(slot, value);
        Ok(json!({ "target": format!("track:{id}"), "field": field,
            "value": value, "previous": previous, "changed": previous != value }))
    }

    fn snap_candidates(&self, clip: &str) -> Vec<i64> {
        let mut edges = vec![0, self.length, self.playhead];
        for other in self.clips.iter().filter(|other| other.id != clip) {
            edges.push(other.start);
            edges.push(other.end());
        }
        edges
    }

    /// Validates a move without applying it. The drop preview and the commit
    /// both come from here, so feedback can never disagree with the result.
    ///
    /// Policy: starts below 0 and ends past the sequence are rejected rather
    /// than clamped, overlap on the target track is rejected (no ripple, no
    /// overwrite), and a snapped start is used only when it stays in bounds.
    pub fn plan_move(
        &self,
        clip_id: &str,
        track: Option<&str>,
        start: Option<i64>,
        snap: Option<bool>,
    ) -> Result<MovePlan, CommandError> {
        let clip = self
            .clip(clip_id)
            .ok_or_else(|| CommandError::unknown_element("clip", clip_id))?;
        let target_id = track.unwrap_or(clip.track.as_str());
        let target = self
            .track(target_id)
            .ok_or_else(|| CommandError::unknown_element("track", target_id))?;
        if !clip.kind.fits(target.kind) {
            return Err(CommandError::new(
                "incompatible_track",
                format!(
                    "{} clips cannot be placed on {} tracks",
                    clip.kind.label(),
                    target.kind.label()
                ),
            )
            .target(format!("clip:{clip_id}"))
            .details(
                json!({ "clip_kind": clip.kind.label(), "track_kind": target.kind.label() }),
            ));
        }
        let requested = start.unwrap_or(clip.start);
        let length = self.length;
        let in_bounds = |start: i64| start >= 0 && start + clip.duration <= length;
        let bounds_error = |start: i64| {
            CommandError::new(
                "out_of_bounds",
                format!(
                    "Frames {start}..{} fall outside the sequence (0..{length})",
                    start + clip.duration
                ),
            )
            .target(format!("clip:{clip_id}"))
            .details(json!({ "start": start, "duration": clip.duration,
                "sequence_frames": length }))
        };
        if !in_bounds(requested) {
            return Err(bounds_error(requested));
        }
        let mut resolved = requested;
        if snap.unwrap_or(self.snap) {
            let edges = self.snap_candidates(clip_id);
            let nearest = edges
                .iter()
                .flat_map(|edge| {
                    [
                        (edge - requested, *edge),
                        (edge - (requested + clip.duration), edge - clip.duration),
                    ]
                })
                .filter(|(distance, _)| distance.abs() <= SNAP_FRAMES)
                .min_by_key(|(distance, _)| distance.abs())
                .map(|(_, start)| start)
                .filter(|start| in_bounds(*start));
            if let Some(start) = nearest {
                resolved = start;
            }
        }
        let end = resolved + clip.duration;
        if let Some(conflict) = self
            .clips_on(target.id.as_str())
            .find(|other| other.id != clip.id && resolved < other.end() && other.start < end)
        {
            return Err(CommandError::new(
                "overlap",
                format!("Would overlap `{}` on {}", conflict.id, target.name),
            )
            .target(format!("clip:{clip_id}"))
            .details(json!({ "conflicts_with": format!("clip:{}", conflict.id),
                "policy": "reject" })));
        }
        Ok(MovePlan {
            clip: clip.id,
            track: target.id,
            start: resolved,
            snapped_from: (resolved != requested).then_some(requested),
            changed: resolved != clip.start || target.id != clip.track,
        })
    }

    pub fn move_clip(
        &mut self,
        clip: &str,
        track: Option<&str>,
        start: Option<i64>,
        snap: Option<bool>,
    ) -> Result<Value, CommandError> {
        let plan = self.plan_move(clip, track, start, snap)?;
        let Some(entry) = self.clips.iter_mut().find(|entry| entry.id == plan.clip) else {
            return Err(CommandError::unknown_element("clip", clip));
        };
        let previous = (entry.track, entry.start);
        entry.track = plan.track;
        entry.start = plan.start;
        let duration = entry.duration;
        self.selected_clip = Some(plan.clip);
        self.selected_track = Some(plan.track);
        Ok(json!({ "target": format!("clip:{clip}"), "action": "move",
            "track": format!("track:{}", plan.track), "frame": plan.start,
            "end": plan.start + duration, "duration": duration,
            "previous": { "track": format!("track:{}", previous.0), "frame": previous.1 },
            "snapped_from": plan.snapped_from, "changed": plan.changed }))
    }

    pub fn plan_trim(
        &self,
        id: &str,
        edge: TrimEdge,
        frame: i64,
    ) -> Result<TrimPlan, CommandError> {
        let clip = self
            .clip(id)
            .ok_or_else(|| CommandError::unknown_element("clip", id))?;
        if !(0..=self.length).contains(&frame) {
            return Err(CommandError::new(
                "out_of_bounds",
                "Trim must stay inside the sequence",
            ));
        }
        let mut resolved = frame;
        if self.snap
            && let Some(candidate) = self
                .snap_candidates(id)
                .into_iter()
                .filter(|candidate| (*candidate - frame).abs() <= SNAP_FRAMES)
                .filter(|candidate| match edge {
                    TrimEdge::Start => *candidate < clip.end(),
                    TrimEdge::End => *candidate > clip.start,
                })
                .min_by_key(|candidate| (*candidate - frame).abs())
        {
            resolved = candidate;
        }
        let (start, end) = match edge {
            TrimEdge::Start => (resolved, clip.end()),
            TrimEdge::End => (clip.start, resolved),
        };
        if start >= end {
            return Err(CommandError::new(
                "invalid_duration",
                "A clip must contain at least one frame",
            ));
        }
        // WHY: reuse placement validation so trimming and moving have the same overlap policy.
        let mut draft = self.clone();
        let entry = draft
            .clips
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| CommandError::unknown_element("clip", id))?;
        entry.duration = end - start;
        draft.plan_move(id, None, Some(start), Some(false))?;
        Ok(TrimPlan { start, end })
    }

    /// Trims one edge of a clip. `speed` is the clip's playback speed as a ratio (`1` is 100 %):
    /// trimming the head advances `source_start` by the frames removed **at that speed**, so the
    /// first visible frame (and every later one) keeps reading the same source time as before for
    /// video, sound and scene time alike. The shift is exact (`Ratio`); an overflow refuses the
    /// trim instead of leaving the picture slid.
    pub fn trim_clip(
        &mut self,
        id: &str,
        edge: TrimEdge,
        frame: i64,
        speed: Ratio,
    ) -> Result<Value, CommandError> {
        let plan = self.plan_trim(id, edge, frame)?;
        let rate = self.rate;
        let clip = self
            .clips
            .iter_mut()
            .find(|clip| clip.id == id)
            .ok_or_else(|| CommandError::unknown_element("clip", id))?;
        let previous = json!({"start":clip.start,"end":clip.end(),"duration":clip.duration});
        // WHY: trimming the head must not slide the picture; the first visible frame keeps
        // reading the same source time it read before. Source time runs at `speed` per second of
        // timeline, so the removed span is scaled by it. Computed before any mutation.
        let head_shift = plan.start - clip.start;
        let source_start = if head_shift == 0 {
            clip.source_start
        } else {
            rate.frame_to_time(head_shift)
                .mul(speed)
                .and_then(|elapsed| elapsed.add(clip.source_start))
                .ok_or_else(|| {
                    CommandError::new(
                        "invalid_time",
                        "The trim overflows the clip's source time; nothing changed",
                    )
                    .target(format!("clip:{id}"))
                })?
        };
        clip.source_start = source_start;
        clip.start = plan.start;
        clip.duration = plan.end - plan.start;
        self.selected_clip = Some(clip.id);
        self.selected_track = Some(clip.track);
        Ok(
            json!({"target":format!("clip:{id}"),"action":"trim","start":plan.start,
            "end":plan.end,"duration":clip.duration,"previous":previous}),
        )
    }

    /// Validates a track reorder. Video tracks always stay above audio tracks,
    /// as in a conventional NLE; a move that would break that is rejected.
    pub fn plan_reorder(
        &self,
        track: &str,
        anchor: &str,
        placement: Placement,
    ) -> Result<ReorderPlan, CommandError> {
        let moving = self
            .track(track)
            .ok_or_else(|| CommandError::unknown_element("track", track))?;
        if self.track(anchor).is_none() {
            return Err(CommandError::unknown_element("track", anchor));
        }
        if track == anchor {
            return Err(CommandError::new(
                "invalid_target",
                "A track cannot be placed relative to itself",
            )
            .target(format!("track:{track}")));
        }
        let mut order: Vec<Id> = self
            .tracks
            .iter()
            .filter(|entry| entry.id != track)
            .map(|entry| entry.id)
            .collect();
        let anchor_index = order
            .iter()
            .position(|id| *id == anchor)
            .ok_or_else(|| CommandError::unknown_element("track", anchor))?;
        let insert_at = match placement {
            Placement::Before => anchor_index,
            Placement::After => anchor_index + 1,
        };
        order.insert(insert_at, moving.id);
        let kinds: Vec<TrackKind> = order
            .iter()
            .filter_map(|id| self.track(id).map(|track| track.kind))
            .collect();
        let video_after_audio = kinds
            .iter()
            .skip_while(|kind| **kind == TrackKind::Video)
            .any(|kind| *kind == TrackKind::Video);
        if video_after_audio {
            return Err(CommandError::new(
                "invalid_track_order",
                "Video tracks must stay above audio tracks",
            )
            .target(format!("track:{track}")));
        }
        let changed = order
            .iter()
            .zip(&self.tracks)
            .any(|(planned, current)| *planned != current.id);
        Ok(ReorderPlan { order, changed })
    }

    pub fn reorder_track(
        &mut self,
        track: &str,
        anchor: &str,
        placement: Placement,
    ) -> Result<Value, CommandError> {
        let plan = self.plan_reorder(track, anchor, placement)?;
        let mut reordered = Vec::with_capacity(self.tracks.len());
        for id in &plan.order {
            if let Some(track) = self.tracks.iter().find(|track| track.id == *id) {
                reordered.push(track.clone());
            }
        }
        self.tracks = reordered;
        Ok(
            json!({ "target": format!("track:{track}"), "action": "move",
            "order": self.tracks.iter().map(|track| format!("track:{}", track.id)).collect::<Vec<_>>(),
            "labels": self.tracks.iter().map(|track| self.label(track.id)).collect::<Vec<_>>(),
            "changed": plan.changed }),
        )
    }

    fn ensure_free_id(&self, id: &Id) -> Result<(), CommandError> {
        if self.track(id).is_some() || self.clip(id).is_some() {
            return Err(CommandError::new(
                "duplicate_id",
                format!("`{id}` is already used by a track or clip"),
            )
            .target(id.as_str()));
        }
        Ok(())
    }

    /// Creates a track. Video tracks go on top of the stack (drawn above the others) and
    /// audio tracks at the bottom, which keeps the invariant that video stays above audio.
    pub fn add_track(
        &mut self,
        id: Id,
        name: &str,
        kind: TrackKind,
    ) -> Result<Value, CommandError> {
        self.ensure_free_id(&id)?;
        let name = name.trim();
        if name.is_empty() {
            return Err(CommandError::invalid_value(
                "A track needs a name",
                json!({}),
            ));
        }
        let height = match kind {
            TrackKind::Video => 56.0,
            TrackKind::Audio => 40.0,
        };
        let track = Track {
            id,
            name: name.to_string(),
            kind,
            hidden: false,
            muted: false,
            height,
        };
        match kind {
            TrackKind::Video => self.tracks.insert(0, track),
            TrackKind::Audio => self.tracks.push(track),
        }
        self.selected_track = Some(id);
        Ok(
            json!({ "target": format!("track:{id}"), "action": "create", "kind": kind.label(),
            "labels": self.tracks.iter().map(|track| self.label(track.id)).collect::<Vec<_>>(),
            "order": self.tracks.iter().map(|track| format!("track:{}", track.id)).collect::<Vec<_>>(),
            "changed": true }),
        )
    }

    /// Validates and appends a clip. Placement follows the same policy as moves:
    /// inside the sequence and no overlap on the track.
    pub fn add_clip(&mut self, clip: Clip) -> Result<Value, CommandError> {
        self.ensure_free_id(&clip.id)?;
        let track = self
            .track(clip.track)
            .ok_or_else(|| CommandError::unknown_element("track", &clip.track))?;
        if !clip.kind.fits(track.kind) {
            return Err(CommandError::new(
                "incompatible_track",
                format!(
                    "{} clips cannot be placed on {} tracks",
                    clip.kind.label(),
                    track.kind.label()
                ),
            )
            .target(format!("clip:{}", clip.id)));
        }
        if clip.duration <= 0 {
            return Err(CommandError::new(
                "invalid_duration",
                "A clip must contain at least one frame",
            ));
        }
        if clip.start < 0 || clip.end() > self.length {
            return Err(CommandError::new(
                "out_of_bounds",
                format!(
                    "Frames {}..{} fall outside the sequence (0..{})",
                    clip.start,
                    clip.end(),
                    self.length
                ),
            )
            .target(format!("clip:{}", clip.id))
            .details(json!({ "start": clip.start, "duration": clip.duration,
                "sequence_frames": self.length })));
        }
        if let Some(conflict) = self
            .clips_on(&clip.track)
            .find(|other| clip.start < other.end() && other.start < clip.end())
        {
            return Err(CommandError::new(
                "overlap",
                format!("Would overlap `{}` on {}", conflict.id, track.name),
            )
            .target(format!("clip:{}", clip.id))
            .details(json!({ "conflicts_with": format!("clip:{}", conflict.id),
                "policy": "reject" })));
        }
        let outcome = json!({ "target": format!("clip:{}", clip.id), "action": "create",
            "track": format!("track:{}", clip.track), "kind": clip.kind.label(),
            "frame": clip.start, "end": clip.end(), "duration": clip.duration,
            "asset": clip.media, "changed": true });
        self.selected_track = Some(clip.track);
        self.selected_clip = Some(clip.id);
        self.clips.push(clip);
        Ok(outcome)
    }

    /// Changes the sequence length. It may not cut a clip.
    pub fn set_length(&mut self, length: i64) -> Result<Value, CommandError> {
        if !(1..=MAX_LENGTH_FRAMES).contains(&length) {
            return Err(CommandError::invalid_value(
                "Sequence length is between 1 and 1000000 frames",
                json!({ "max": MAX_LENGTH_FRAMES }),
            ));
        }
        if length < self.content_end() {
            return Err(CommandError::new(
                "out_of_bounds",
                format!(
                    "A clip ends at frame {}; the length cannot be shorter",
                    self.content_end()
                ),
            ));
        }
        let previous = std::mem::replace(&mut self.length, length);
        self.playhead = self.playhead.min(length);
        Ok(
            json!({ "target": "timeline.length", "field": "value", "value": length,
            "previous": previous, "changed": previous != length }),
        )
    }

    pub fn duration_seconds(&self) -> f64 {
        self.time_of(self.length).to_f64()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(timeline: &Timeline) -> Vec<&str> {
        timeline
            .tracks
            .iter()
            .map(|track| track.id.as_str())
            .collect()
    }

    #[test]
    fn timecode_formats_hours_seconds_and_frames() {
        assert_eq!(timecode(0), "00:00:00:00");
        assert_eq!(timecode(3 * FPS + 14), "00:00:03:14");
        assert_eq!(timecode(SEQUENCE_FRAMES), "00:00:29:00");
        assert_eq!(timecode(-5), "00:00:00:00");
    }

    #[test]
    fn labels_follow_position() {
        let timeline = Timeline::default();
        let labels: Vec<_> = timeline
            .tracks
            .iter()
            .map(|t| timeline.label(t.id))
            .collect();
        assert_eq!(labels, ["V2", "V1", "A1", "A2"]);
    }

    #[test]
    fn playhead_clamps_and_reports() {
        let mut timeline = Timeline::default();
        let out = timeline.set_playhead(72);
        assert_eq!(out["value"], 72);
        assert_eq!(out["clamped"], false);
        let out = timeline.set_playhead(10_000);
        assert_eq!(out["value"], SEQUENCE_FRAMES);
        assert_eq!(out["clamped"], true);
        assert_eq!(timeline.set_playhead(-3)["value"], 0);
    }

    #[test]
    fn zoom_clamps_to_limits() {
        let mut timeline = Timeline::default();
        assert_eq!(timeline.set_zoom(1000.0)["value"], ZOOM_MAX);
        assert_eq!(timeline.set_zoom(0.0)["value"], ZOOM_MIN);
        assert_eq!(timeline.set_zoom(40.0)["value"], 40.0);
        assert!((timeline.pixels_per_frame() - 40.0 / 24.0).abs() < 1e-6);
    }

    #[test]
    fn selecting_a_clip_selects_its_track() {
        let mut timeline = Timeline::default();
        timeline.select_clip("title-north").unwrap();
        assert_eq!(timeline.selected_clip, Some(Id::lit("title-north")));
        assert_eq!(timeline.selected_track, Some(Id::lit("titles")));
        assert!(timeline.select_clip("ghost").is_err());
        assert_eq!(timeline.selected_clip, Some(Id::lit("title-north")));
    }

    #[test]
    fn selecting_another_track_drops_a_foreign_clip_selection() {
        let mut timeline = Timeline::default();
        timeline.select_track("score").unwrap();
        assert_eq!(timeline.selected_clip, None);
        timeline.select_clip("score-clip").unwrap();
        timeline.select_track("score").unwrap();
        assert_eq!(timeline.selected_clip, Some(Id::lit("score-clip")));
    }

    #[test]
    fn track_flags_are_kind_specific() {
        let mut timeline = Timeline::default();
        assert_eq!(
            timeline
                .set_track_flag("score", TrackFlag::Muted, true)
                .unwrap()["value"],
            true
        );
        assert!(timeline.track("score").unwrap().muted);
        assert_eq!(
            timeline
                .set_track_flag("score", TrackFlag::Hidden, true)
                .unwrap_err()
                .code,
            "unknown_field"
        );
        timeline
            .set_track_flag("picture", TrackFlag::Hidden, true)
            .unwrap();
        assert_eq!(
            timeline
                .set_track_flag("picture", TrackFlag::Muted, true)
                .unwrap_err()
                .code,
            "unknown_field"
        );
        assert_eq!(
            timeline
                .set_track_flag("zzz", TrackFlag::Muted, true)
                .unwrap_err()
                .code,
            "unknown_element"
        );
    }

    #[test]
    fn move_within_a_track_updates_start() {
        let mut timeline = Timeline {
            snap: false,
            ..Timeline::default()
        };
        let out = timeline
            .move_clip("title-north", None, Some(200), None)
            .unwrap();
        assert_eq!(out["frame"], 200);
        assert_eq!(timeline.clip("title-north").unwrap().start, 200);
        assert_eq!(timeline.selected_clip, Some(Id::lit("title-north")));
    }

    #[test]
    fn move_to_another_compatible_track() {
        let mut timeline = Timeline {
            snap: false,
            ..Timeline::default()
        };
        let out = timeline.move_clip("title-north", Some("picture"), Some(400), None);
        assert_eq!(out.unwrap_err().code, "overlap");
        // Titles can move onto the other video track once there is a gap.
        timeline
            .move_clip("basalt-cliffs", Some("titles"), Some(504), None)
            .unwrap();
        assert_eq!(timeline.clip("basalt-cliffs").unwrap().track, "titles");
        timeline
            .move_clip("title-north", Some("picture"), Some(504), None)
            .unwrap();
        assert_eq!(timeline.clip("title-north").unwrap().track, "picture");
    }

    #[test]
    fn incompatible_drops_leave_no_residue() {
        let mut timeline = Timeline::default();
        let before = timeline.clips.clone();
        for (clip, track) in [
            ("score-clip", "picture"),
            ("first-light", "score"),
            ("title-north", "ambience"),
        ] {
            let error = timeline
                .move_clip(clip, Some(track), None, Some(false))
                .unwrap_err();
            assert_eq!(error.code, "incompatible_track", "{clip}->{track}");
        }
        assert_eq!(timeline.clips, before);
    }

    #[test]
    fn negative_and_out_of_sequence_starts_are_rejected() {
        let mut timeline = Timeline::default();
        let before = timeline.clips.clone();
        assert_eq!(
            timeline
                .move_clip("title-north", None, Some(-1), Some(false))
                .unwrap_err()
                .code,
            "out_of_bounds"
        );
        assert_eq!(
            timeline
                .move_clip("title-north", None, Some(SEQUENCE_FRAMES - 10), Some(false))
                .unwrap_err()
                .code,
            "out_of_bounds"
        );
        // A full-length clip fits only at frame 0.
        assert_eq!(
            timeline
                .move_clip("score-clip", None, Some(1), Some(false))
                .unwrap_err()
                .code,
            "out_of_bounds"
        );
        assert_eq!(timeline.clips, before);
    }

    #[test]
    fn overlap_is_rejected_and_names_the_conflict() {
        let mut timeline = Timeline::default();
        let error = timeline
            .move_clip("river-detail", None, Some(8 * FPS + 10), Some(false))
            .unwrap_err();
        assert_eq!(error.code, "overlap");
        assert_eq!(
            error.details.unwrap()["conflicts_with"],
            "clip:atlantic-coast"
        );
        // Touching edges is not an overlap.
        assert!(
            timeline
                .plan_move("river-detail", None, Some(15 * FPS), Some(false))
                .is_ok()
        );
    }

    #[test]
    fn snap_pulls_to_neighbour_edges_within_tolerance() {
        let timeline = Timeline {
            playhead: 400,
            ..Timeline::default()
        };
        // The title is 120 frames long; dropping it 3 frames after the playhead snaps.
        let plan = timeline
            .plan_move("title-north", None, Some(403), Some(true))
            .unwrap();
        assert_eq!(plan.start, 400);
        assert_eq!(plan.snapped_from, Some(403));
        // The clip's end can snap too.
        let plan = timeline
            .plan_move("title-north", None, Some(382), Some(true))
            .unwrap();
        // The end (502) is 2 frames from the neighbouring clip's end at 504.
        assert_eq!(plan.start, 384);
        // Outside the tolerance nothing changes.
        let plan = timeline
            .plan_move("title-north", None, Some(300), Some(true))
            .unwrap();
        assert_eq!((plan.start, plan.snapped_from), (300, None));
        let plan = timeline
            .plan_move("title-north", None, Some(403), Some(false))
            .unwrap();
        assert_eq!(plan.start, 403);
    }

    #[test]
    fn snap_never_leaves_the_sequence() {
        let timeline = Timeline::default();
        // Snapping to the sequence end must still fit the clip.
        let plan = timeline
            .plan_move("title-north", None, Some(SEQUENCE_FRAMES - 120), Some(true))
            .unwrap();
        assert_eq!(plan.start, SEQUENCE_FRAMES - 120);
    }

    #[test]
    fn move_to_current_position_reports_no_change() {
        let mut timeline = Timeline::default();
        let out = timeline
            .move_clip("first-light", None, Some(0), Some(false))
            .unwrap();
        assert_eq!(out["changed"], false);
    }

    #[test]
    fn reorder_swaps_same_kind_tracks_and_relabels() {
        let mut timeline = Timeline::default();
        let out = timeline
            .reorder_track("ambience", "score", Placement::Before)
            .unwrap();
        assert_eq!(ids(&timeline), ["titles", "picture", "ambience", "score"]);
        assert_eq!(out["labels"], json!(["V2", "V1", "A1", "A2"]));
        assert_eq!(timeline.label("ambience"), "A1");
        timeline
            .reorder_track("picture", "titles", Placement::Before)
            .unwrap();
        assert_eq!(ids(&timeline), ["picture", "titles", "ambience", "score"]);
        assert_eq!(timeline.label("picture"), "V2");
    }

    #[test]
    fn reorder_rejects_audio_above_video_and_bad_targets() {
        let mut timeline = Timeline::default();
        for (track, anchor, placement) in [
            ("score", "titles", Placement::Before),
            ("titles", "ambience", Placement::After),
        ] {
            let error = timeline
                .reorder_track(track, anchor, placement)
                .unwrap_err();
            assert_eq!(error.code, "invalid_track_order");
        }
        assert_eq!(
            timeline
                .reorder_track("score", "score", Placement::Before)
                .unwrap_err()
                .code,
            "invalid_target"
        );
        assert_eq!(
            timeline
                .reorder_track("score", "nope", Placement::Before)
                .unwrap_err()
                .code,
            "unknown_element"
        );
        assert_eq!(ids(&timeline), ["titles", "picture", "score", "ambience"]);
    }

    #[test]
    fn reorder_to_the_same_slot_is_a_no_op() {
        let mut timeline = Timeline::default();
        let out = timeline
            .reorder_track("score", "ambience", Placement::Before)
            .unwrap();
        assert_eq!(out["changed"], false);
    }
}

#[cfg(test)]
mod trim_tests {
    use super::*;
    #[test]
    fn each_edge_preserves_the_opposite_edge() {
        let mut t = Timeline {
            snap: false,
            ..Timeline::default()
        };
        let end = t.clip("first-light").unwrap().end();
        t.trim_clip("first-light", TrimEdge::Start, 24, Ratio::from_int(1))
            .unwrap();
        assert_eq!(t.clip("first-light").unwrap().end(), end);
        t.trim_clip("first-light", TrimEdge::End, 120, Ratio::from_int(1))
            .unwrap();
        assert_eq!(t.clip("first-light").unwrap().start, 24);
        assert_eq!(t.clip("first-light").unwrap().duration, 96);
    }
    /// The source time a clip reads at `frame`, written independently of `trim_clip`.
    fn read_at(t: &Timeline, id: &str, frame: i64, speed: Ratio) -> Ratio {
        let clip = t.clip(id).unwrap();
        let elapsed = t.rate.frame_to_time(frame - clip.start);
        clip.source_start.add(elapsed.mul(speed).unwrap()).unwrap()
    }
    #[test]
    fn head_trim_keeps_every_frame_on_the_source_time_it_read_at_any_speed() {
        for (num, den) in [(1, 2), (1, 1), (2, 1)] {
            let speed = Ratio::new(num, den).unwrap();
            let mut t = Timeline {
                snap: false,
                ..Timeline::default()
            };
            let before: Vec<Ratio> = [30, 60, 100]
                .iter()
                .map(|frame| read_at(&t, "first-light", *frame, speed))
                .collect();
            t.trim_clip("first-light", TrimEdge::Start, 24, speed)
                .unwrap();
            for (frame, expected) in [30, 60, 100].iter().zip(before) {
                assert_eq!(read_at(&t, "first-light", *frame, speed), expected);
            }
            // The removed 24 frames are 1 s of timeline: `speed` seconds of source.
            assert_eq!(t.clip("first-light").unwrap().source_start, speed);
        }
    }
    #[test]
    fn head_trim_at_double_speed_differs_from_the_unscaled_shift() {
        let mut t = Timeline {
            snap: false,
            ..Timeline::default()
        };
        t.trim_clip("first-light", TrimEdge::Start, 24, Ratio::from_int(2))
            .unwrap();
        assert_ne!(
            t.clip("first-light").unwrap().source_start,
            t.rate.frame_to_time(24),
            "the shift is not the timeline time"
        );
    }
    #[test]
    fn end_trim_leaves_the_source_start_alone() {
        let mut t = Timeline {
            snap: false,
            ..Timeline::default()
        };
        t.trim_clip("first-light", TrimEdge::End, 120, Ratio::from_int(2))
            .unwrap();
        assert_eq!(t.clip("first-light").unwrap().source_start, Ratio::ZERO);
    }
    #[test]
    fn invalid_trim_is_atomic() {
        let mut t = Timeline {
            snap: false,
            ..Timeline::default()
        };
        let before = t.clips.clone();
        for (edge, frame) in [
            (TrimEdge::Start, -1),
            (TrimEdge::Start, 192),
            (TrimEdge::End, 0),
            (TrimEdge::End, 240),
            (TrimEdge::End, SEQUENCE_FRAMES + 1),
        ] {
            assert!(
                t.trim_clip("first-light", edge, frame, Ratio::from_int(1))
                    .is_err()
            );
            assert_eq!(t.clips, before);
        }
    }
    #[test]
    fn trim_preview_and_snap_do_not_mutate_the_sequence() {
        let mut t = Timeline {
            snap: false,
            ..Timeline::default()
        };
        t.trim_clip("first-light", TrimEdge::End, 120, Ratio::from_int(1))
            .unwrap();
        t.snap = true;
        let before = t.clips.clone();
        assert_eq!(
            t.plan_trim("first-light", TrimEdge::End, 190).unwrap().end,
            192
        );
        assert_eq!(t.clips, before);
    }
}
