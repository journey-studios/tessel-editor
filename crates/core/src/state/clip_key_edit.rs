//! Editing the keyframes of what a clip shows (F073): position, zoom, rotation, opacity.
//!
//! Like the camera keys, every edit builds the *whole* candidate track list, validates it as a
//! unit (`clip_keys::normalise`) and only then replaces the clip's list, so a rejected command
//! leaves the clip untouched. The model and its rules are documented in [`crate::clip_keys`].

use serde_json::{Value, json};

use super::EditorState;
use super::error::{CommandError, Raw};
use super::inspector::{Applies, Value as InspectorValue, property_def};
use super::timeline::Clip;
use crate::clip_keys;
use crate::easing::KeyEasing;
use crate::group::{KeyTrack, Prop};
use crate::id::Id;

/// One edit of a clip's keys.
#[derive(Debug, Clone, PartialEq)]
pub enum ClipKeysCommand {
    /// Adds a key, or replaces the key at the same time. `easing` `None` means "not said": an
    /// existing key keeps its easing, a new key is linear.
    Set {
        clip: String,
        prop: Prop,
        time_ms: i64,
        value: f64,
        easing: Option<KeyEasing>,
    },
    /// Removes the key at exactly `time_ms`.
    Remove {
        clip: String,
        prop: Prop,
        time_ms: i64,
    },
    /// Removes the track of `prop`, or every track when `prop` is `None`.
    Clear { clip: String, prop: Option<Prop> },
    /// Records the value the clip shows at the playhead as a key at that source time: the keyed
    /// value when the property already has a track, the inspector value otherwise.
    Capture {
        clip: String,
        prop: Prop,
        easing: Option<KeyEasing>,
    },
    /// Edits one field (`time_ms`, `value`, `easing`) of the `index`-th key of `prop`.
    Field {
        clip: String,
        prop: Prop,
        index: usize,
        field: String,
        value: Raw,
    },
}

impl EditorState {
    fn keyed_clip(&self, clip: &str) -> Result<&Clip, CommandError> {
        let entry = self
            .timeline
            .clip(clip)
            .ok_or_else(|| CommandError::unknown_element("clip", clip))?;
        if !Applies::Framed.includes(entry.kind) {
            return Err(CommandError::new(
                "not_applicable",
                format!("A {} clip has no keys", entry.kind.label()),
            )
            .target(format!("clip:{}", entry.id)));
        }
        Ok(entry)
    }

    /// Milliseconds on the clip's media clock at sequence `frame` (its source time, so a trim, a
    /// move or a split keeps every key on the picture it was set on): the time its keys are read
    /// at, the same on the monitor, in `render.frame` and in the export.
    pub fn clip_local_ms(&self, clip: &Clip, frame: i64) -> f64 {
        self.clip_source_time(clip, frame).to_f64() * 1000.0
    }

    /// The value of keyed property `prop` the clip shows at `frame` before fades: the keyed value
    /// when it has a track, else the inspector's.
    pub fn clip_keyed_value(&self, clip: &Clip, prop: Prop, frame: i64) -> f64 {
        if let Some(value) = clip_keys::evaluate(&clip.keys, prop, self.clip_local_ms(clip, frame))
        {
            return value;
        }
        match property_def(clip_keys::property_id(prop))
            .map(|def| self.inspector.value(clip.id, def))
        {
            Some(InspectorValue::Number(number)) => number,
            _ => 0.0,
        }
    }

    pub(super) fn apply_clip_keys(
        &mut self,
        command: ClipKeysCommand,
    ) -> Result<Value, CommandError> {
        let (clip_ref, candidate) = match command {
            ClipKeysCommand::Set {
                clip,
                prop,
                time_ms,
                value,
                easing,
            } => {
                let tracks = &self.keyed_clip(&clip)?.keys;
                let next = clip_keys::with_key(tracks, prop, time_ms, value, easing)?;
                (clip, next)
            }
            ClipKeysCommand::Remove {
                clip,
                prop,
                time_ms,
            } => {
                let tracks = &self.keyed_clip(&clip)?.keys;
                let next = clip_keys::without_key(tracks, prop, time_ms)
                    .map_err(|error| error.target(format!("clip:{clip}")))?;
                (clip, next)
            }
            ClipKeysCommand::Clear { clip, prop } => {
                let tracks = &self.keyed_clip(&clip)?.keys;
                if prop.is_some_and(|prop| tracks.iter().all(|track| track.prop != prop)) {
                    return Err(CommandError::new(
                        "unknown_element",
                        format!("Clip `{clip}` has no keys on that property"),
                    )
                    .target(format!("clip:{clip}")));
                }
                let next = clip_keys::cleared(tracks, prop);
                (clip, next)
            }
            ClipKeysCommand::Capture { clip, prop, easing } => {
                let entry = self.keyed_clip(&clip)?;
                let frame = self.timeline.playhead;
                if frame < entry.start || frame >= entry.end() {
                    return Err(CommandError::new(
                        "invalid_time",
                        format!(
                            "The playhead (frame {frame}) is outside clip `{}` ({}..{}): move it over the clip to key it",
                            entry.id,
                            entry.start,
                            entry.end()
                        ),
                    )
                    .target(format!("clip:{}", entry.id)));
                }
                let time_ms = self.clip_local_ms(entry, frame).round() as i64;
                let value = self.clip_keyed_value(entry, prop, frame);
                let next = clip_keys::with_key(&entry.keys, prop, time_ms, value, easing)?;
                (clip, next)
            }
            ClipKeysCommand::Field {
                clip,
                prop,
                index,
                field,
                value,
            } => {
                let tracks = &self.keyed_clip(&clip)?.keys;
                let next = clip_keys::with_key_field(tracks, prop, index, &field, &value)?;
                (clip, next)
            }
        };
        self.commit_clip_keys(&clip_ref, candidate)
    }

    /// Stores `tracks` or changes nothing. (A zoom track replaces Ken Burns in the plan: both
    /// drive the same zoom and the keys are the more specific statement.)
    fn commit_clip_keys(
        &mut self,
        clip: &str,
        tracks: Vec<KeyTrack>,
    ) -> Result<Value, CommandError> {
        let id: Id = self.keyed_clip(clip)?.id;
        let target = format!("clip:{id}");
        let tracks = clip_keys::normalise(tracks).map_err(|error| error.target(target.clone()))?;
        let Some(entry) = self.timeline.clips.iter_mut().find(|entry| entry.id == id) else {
            return Err(CommandError::unknown_element("clip", clip));
        };
        let changed = entry.keys != tracks;
        entry.keys = tracks;
        Ok(json!({
            "target": target,
            "field": "keys",
            "tracks": clip_keys::tracks_to_json(&entry.keys),
            "count": entry.keys.iter().map(|track| track.keys.len()).sum::<usize>(),
            "changed": changed,
        }))
    }
}
