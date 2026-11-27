//! Editing the player and caption clips of a project (F071).
//!
//! Every edit builds the whole candidate spec, validates it as a unit and only then replaces the
//! clip's, so a rejected command leaves the project untouched (and `execute` records no undo
//! entry for it). The models and their limits live in [`crate::player`] and
//! [`crate::subtitles`]; this is the one place the clip, the timeline and the history meet them,
//! for the window, the CLI and a script alike.

use serde_json::{Value, json};

use super::timeline::{Clip, ClipKind};
use super::{CommandError, EditorState, Generated, Raw};
use crate::id::Id;
use crate::player::PlayerSpec;
use crate::subtitles::{Cue, SubtitleSpec};
use crate::time::Ratio;

/// What a player or caption clip is made of.
#[derive(Debug, Clone, PartialEq)]
pub enum LiveSpec {
    Player(PlayerSpec),
    Subtitles(SubtitleSpec),
}

/// One edit of a player or of a caption track.
#[derive(Debug, Clone, PartialEq)]
pub enum LiveCommand {
    /// Sets one field of a player (`audio`, `width`, `bars`, a colour, ...).
    PlayerField {
        clip: String,
        field: String,
        value: Raw,
    },
    /// Sets one style field of a caption track (`size`, `color`, `box`, `burn`, ...).
    SubtitleStyle {
        clip: String,
        field: String,
        value: Raw,
    },
    /// Replaces the cues of a caption track by those of an SRT text. The clip grows, when there
    /// is room, to cover the last cue.
    SubtitleImport {
        clip: String,
        srt: String,
    },
    /// Adds a cue where its start puts it.
    SubtitleCueAdd {
        clip: String,
        start_ms: i64,
        end_ms: i64,
        text: String,
    },
    /// Sets `start_ms`, `end_ms` or `text` of the `index`-th cue.
    SubtitleCue {
        clip: String,
        index: usize,
        field: String,
        value: Raw,
    },
    SubtitleCueRemove {
        clip: String,
        index: usize,
    },
}

impl LiveCommand {
    pub fn history_label(&self) -> &'static str {
        match self {
            Self::PlayerField { .. } => "Edit player",
            Self::SubtitleStyle { .. } => "Edit subtitles style",
            Self::SubtitleImport { .. } => "Import subtitles",
            Self::SubtitleCueAdd { .. }
            | Self::SubtitleCue { .. }
            | Self::SubtitleCueRemove { .. } => "Edit subtitles",
        }
    }
}

/// Length of the cue the toolbar adds at the playhead, and of the one a new track starts with.
const STARTER_CUE_MS: i64 = 2000;
/// The text of a cue added at the playhead, there to be replaced.
pub const NEW_CUE_TEXT: &str = "Nova legenda";

impl EditorState {
    /// The `(start, end)` in milliseconds of the cue "add cue" puts at the playhead of the
    /// caption clip `clip`: it starts where the playhead is on the clip's content clock (the
    /// clip's start when the playhead is outside it) and runs `STARTER_CUE_MS`, cut short by the
    /// next cue. WHY the core and not each caller: the toolbar, the CLI and a script must put
    /// the cue in the same place.
    pub fn cue_slot_at_playhead(&self, clip: &str) -> Result<(i64, i64), CommandError> {
        let entry = self.live_clip(clip, ClipKind::Subtitles, "subtitle")?;
        let timeline = &self.timeline;
        let elapsed = (timeline.playhead - entry.start).clamp(0, entry.duration);
        let overflow = || {
            CommandError::new("invalid_time", "The cue time overflows")
                .target(format!("clip:{}", entry.id))
        };
        let content = entry
            .source_start
            .add(timeline.rate.frame_to_time(elapsed))
            .ok_or_else(overflow)?;
        let start_ms = content.mul_int(1000).ok_or_else(overflow)?.round();
        let end_ms = entry
            .subtitles
            .iter()
            .flat_map(|spec| spec.cues.iter())
            .map(|cue| cue.start_ms)
            .filter(|next| *next > start_ms)
            .min()
            .map_or(start_ms + STARTER_CUE_MS, |next| {
                next.min(start_ms + STARTER_CUE_MS)
            });
        Ok((start_ms, end_ms))
    }

    fn live_clip<'a>(
        &'a self,
        clip: &str,
        kind: ClipKind,
        what: &str,
    ) -> Result<&'a Clip, CommandError> {
        let entry = self
            .timeline
            .clip(clip)
            .ok_or_else(|| CommandError::unknown_element("clip", clip))?;
        if entry.kind != kind {
            return Err(CommandError::new(
                "not_applicable",
                format!("Only {what} clips carry this"),
            )
            .target(format!("clip:{}", entry.id)));
        }
        Ok(entry)
    }

    /// A player may follow no clip, or an audio clip of the active sequence.
    pub(super) fn check_player(&self, player: &PlayerSpec) -> Result<(), CommandError> {
        player.validate()?;
        let Some(link) = player.audio else {
            return Ok(());
        };
        let audio = self
            .timeline
            .clip(link.as_str())
            .is_some_and(|clip| clip.kind == ClipKind::Audio);
        if audio {
            return Ok(());
        }
        Err(CommandError::new(
            "invalid_link",
            format!("`{link}` is not an audio clip of the sequence"),
        )
        .target(format!("clip:{link}"))
        .details(json!({ "field": "audio" })))
    }

    fn store_live(&mut self, id: Id, spec: LiveSpec) -> bool {
        let Some(entry) = self.timeline.clips.iter_mut().find(|entry| entry.id == id) else {
            return false;
        };
        match spec {
            LiveSpec::Player(player) => {
                let changed = entry.player.as_ref() != Some(&player);
                entry.player = Some(player);
                changed
            }
            LiveSpec::Subtitles(subtitles) => {
                let changed = entry.subtitles.as_ref() != Some(&subtitles);
                entry.subtitles = Some(subtitles);
                changed
            }
        }
    }

    fn player_outcome(id: Id, player: &PlayerSpec, changed: bool) -> Value {
        json!({ "target": format!("clip:{id}"), "field": "player",
            "value": player.to_json(), "changed": changed })
    }

    fn subtitles_outcome(id: Id, subtitles: &SubtitleSpec, changed: bool) -> Value {
        json!({ "target": format!("clip:{id}"), "field": "subtitles",
            "value": subtitles.to_json(), "changed": changed })
    }

    pub(super) fn apply_live(&mut self, command: LiveCommand) -> Result<Value, CommandError> {
        match command {
            LiveCommand::PlayerField { clip, field, value } => {
                let entry = self.live_clip(&clip, ClipKind::Player, "player")?;
                let id = entry.id;
                let current = entry.player.clone().unwrap_or_default();
                let next = current
                    .with_field(&field, &value)
                    .map_err(|error| error.target(format!("clip_player:{id}")))?;
                self.check_player(&next)?;
                let changed = self.store_live(id, LiveSpec::Player(next.clone()));
                Ok(Self::player_outcome(id, &next, changed))
            }
            LiveCommand::SubtitleStyle { clip, field, value } => {
                let (id, current) = self.subtitles_of(&clip)?;
                let next = current
                    .with_style(&field, &value)
                    .map_err(|error| error.target(format!("clip_subtitles:{id}")))?;
                let changed = self.store_live(id, LiveSpec::Subtitles(next.clone()));
                Ok(Self::subtitles_outcome(id, &next, changed))
            }
            LiveCommand::SubtitleImport { clip, srt } => {
                let (id, current) = self.subtitles_of(&clip)?;
                let next = SubtitleSpec::parse_srt(&srt)
                    .and_then(|cues| current.with_cues(cues))
                    .map_err(|error| error.target(format!("clip_subtitles:{id}")))?;
                let imported = next.cues.len();
                let changed = self.store_live(id, LiveSpec::Subtitles(next.clone()));
                let fitted = self.fit_subtitle_clip(id);
                let mut outcome = Self::subtitles_outcome(id, &next, changed || fitted);
                outcome["imported"] = json!(imported);
                outcome["fitted"] = json!(fitted);
                Ok(outcome)
            }
            LiveCommand::SubtitleCueAdd {
                clip,
                start_ms,
                end_ms,
                text,
            } => {
                let (id, current) = self.subtitles_of(&clip)?;
                let cue = Cue {
                    start_ms,
                    end_ms,
                    text: crate::text::normalize(&text)?,
                };
                let (next, index) = current
                    .with_cue_added(cue)
                    .map_err(|error| error.target(format!("clip_subtitles:{id}")))?;
                let changed = self.store_live(id, LiveSpec::Subtitles(next.clone()));
                let mut outcome = Self::subtitles_outcome(id, &next, changed);
                outcome["cue"] = json!(index);
                Ok(outcome)
            }
            LiveCommand::SubtitleCue {
                clip,
                index,
                field,
                value,
            } => {
                let (id, current) = self.subtitles_of(&clip)?;
                let next = current
                    .with_cue_field(index, &field, &value)
                    .map_err(|error| error.target(format!("subtitle_cue:{id}/{index}")))?;
                let changed = self.store_live(id, LiveSpec::Subtitles(next.clone()));
                Ok(Self::subtitles_outcome(id, &next, changed))
            }
            LiveCommand::SubtitleCueRemove { clip, index } => {
                let (id, current) = self.subtitles_of(&clip)?;
                let next = current
                    .with_cue_removed(index)
                    .map_err(|error| error.target(format!("subtitle_cue:{id}/{index}")))?;
                let changed = self.store_live(id, LiveSpec::Subtitles(next.clone()));
                Ok(Self::subtitles_outcome(id, &next, changed))
            }
        }
    }

    /// The ID and caption track of the subtitle clip `clip`.
    pub fn subtitles_of(&self, clip: &str) -> Result<(Id, SubtitleSpec), CommandError> {
        let entry = self.live_clip(clip, ClipKind::Subtitles, "subtitle")?;
        Ok((entry.id, entry.subtitles.clone().unwrap_or_default()))
    }

    /// The SRT text of the subtitle clip `clip` (what `subtitles.export` writes).
    pub fn subtitles_srt(&self, clip: &str) -> Result<String, CommandError> {
        Ok(self.subtitles_of(clip)?.1.to_srt())
    }

    /// Lengthens the subtitle clip `id` so that it covers its last cue, as far as the sequence
    /// and the next clip of its track allow; `true` when the clip changed. WHY not an error when
    /// there is no room: the cues are still in the track and a later trim reveals them, and a
    /// refused import would lose the user's SRT for want of a gap on the timeline.
    fn fit_subtitle_clip(&mut self, id: Id) -> bool {
        let timeline = &self.timeline;
        let Some(clip) = timeline.clip(id.as_str()) else {
            return false;
        };
        let Some(spec) = &clip.subtitles else {
            return false;
        };
        // The content time the last cue ends at, counted from where the clip starts reading.
        let Some(last) =
            Ratio::new(spec.last_end_ms(), 1000).and_then(|end| end.sub(clip.source_start))
        else {
            return false;
        };
        let Some(negated) = Ratio::new(-last.num(), last.den()) else {
            return false;
        };
        // Whole frames needed, rounded up.
        let needed = -timeline.rate.time_to_frame_floor(negated);
        let room = timeline
            .clips_on(&clip.track)
            .filter(|other| other.id != clip.id && other.start >= clip.end())
            .map(|other| other.start)
            .min()
            .unwrap_or(timeline.length)
            .min(timeline.length);
        let end = clip.start.saturating_add(needed).min(room);
        if end <= clip.end() {
            return false;
        }
        let duration = end - clip.start;
        match self.timeline.clips.iter_mut().find(|entry| entry.id == id) {
            Some(entry) => entry.duration = duration,
            None => return false,
        }
        true
    }

    /// The audio clip a new player follows: the selected one, else the one under the playhead,
    /// else the first of the sequence.
    fn audio_to_follow(&self) -> Option<&Clip> {
        let timeline = &self.timeline;
        let audio = |clip: &&Clip| clip.kind == ClipKind::Audio;
        timeline
            .selected_clip
            .and_then(|id| timeline.clip(id.as_str()))
            .filter(audio)
            .or_else(|| {
                timeline
                    .clips
                    .iter()
                    .filter(audio)
                    .find(|clip| clip.start <= timeline.playhead && timeline.playhead < clip.end())
            })
            .or_else(|| {
                timeline
                    .clips
                    .iter()
                    .filter(audio)
                    .min_by_key(|clip| clip.start)
            })
    }

    /// The toolbar "add player": a player over the audio clip it follows (same start and
    /// length, so its progress runs exactly across that sound), or at the playhead when the
    /// sequence has no audio clip. One undo entry.
    pub(super) fn add_player(&mut self) -> Result<Value, CommandError> {
        let followed = self
            .audio_to_follow()
            .map(|clip| (clip.id, clip.start, clip.duration));
        let player = PlayerSpec {
            audio: followed.map(|(id, _, _)| id),
            ..PlayerSpec::default()
        };
        let generated = Generated::Live(LiveSpec::Player(player));
        match followed {
            Some((_, start, duration)) => self.add_generated_clip_at(
                ClipKind::Player,
                ("players", "Players"),
                generated,
                start,
                duration,
            ),
            None => self.add_generated_clip(
                ClipKind::Player,
                ("players", "Players"),
                generated,
                super::DEFAULT_GENERATED_SECONDS,
            ),
        }
    }

    /// The toolbar "add subtitles": a caption track with one cue to replace (or an SRT to
    /// import) at the playhead.
    pub(super) fn add_subtitles(&mut self) -> Result<Value, CommandError> {
        let mut spec = SubtitleSpec::default();
        spec.cues.push(Cue {
            start_ms: 0,
            end_ms: STARTER_CUE_MS,
            text: "Sua legenda".to_string(),
        });
        self.add_generated_clip(
            ClipKind::Subtitles,
            ("subtitles", "Legendas"),
            Generated::Live(LiveSpec::Subtitles(spec)),
            super::DEFAULT_GENERATED_SECONDS,
        )
    }
}

/// The captions an export did not draw, as the SRT file written beside the video.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptionSidecar {
    pub clip: Id,
    pub name: String,
    pub cues: usize,
    /// The SRT on the exported video's clock (zero at its first frame).
    pub srt: String,
}

impl EditorState {
    /// For an export snapshot of frames `[from, to)`: removes the caption clips whose `burn` is
    /// off from the exported sequence, so they are not drawn, and returns each one's cues as an
    /// SRT on the video's clock. WHY only the exported sequence: a caption inside a nested
    /// sequence is part of a clip that plays on another clock, so it stays drawn rather than
    /// becoming a file whose times would not match the video.
    pub fn set_aside_unburned_captions(&mut self, from: i64, to: i64) -> Vec<CaptionSidecar> {
        let sidecars = self.unburned_caption_sidecars(from, to);
        self.timeline
            .clips
            .retain(|clip| !clip.subtitles.as_ref().is_some_and(|spec| !spec.burn));
        sidecars
    }

    pub fn unburned_caption_sidecars(&self, from: i64, to: i64) -> Vec<CaptionSidecar> {
        let rate = self.timeline.rate;
        let millis = |time: Ratio| {
            Ratio::new(1000, 1)
                .and_then(|thousand| time.mul(thousand))
                .map_or(0, Ratio::round)
        };
        let origin = rate.frame_to_time(from);
        let window = (
            0,
            millis(rate.frame_to_time(to).sub(origin).unwrap_or(Ratio::ZERO)),
        );
        let mut sidecars = Vec::new();
        for clip in &self.timeline.clips {
            let Some(spec) = clip.subtitles.as_ref().filter(|spec| !spec.burn) else {
                continue;
            };
            // Content time `c` shows at sequence time `start + c - source_start`; the video's
            // clock then subtracts the first exported frame.
            let offset = rate
                .frame_to_time(clip.start)
                .sub(clip.source_start)
                .and_then(|time| time.sub(origin))
                .map_or(0, millis);
            let clip_from = millis(
                rate.frame_to_time(clip.start)
                    .sub(origin)
                    .unwrap_or(Ratio::ZERO),
            );
            let clip_to = millis(
                rate.frame_to_time(clip.start + clip.duration)
                    .sub(origin)
                    .unwrap_or(Ratio::ZERO),
            );
            let cues = spec.retimed(offset, clip_from.max(window.0), clip_to.min(window.1));
            sidecars.push(CaptionSidecar {
                clip: clip.id,
                name: clip.name.clone(),
                cues: cues.len(),
                srt: crate::subtitles::write_srt(&cues),
            });
        }
        sidecars
    }
}
