//! Editing the camera keyframes of a scene clip (BL140).
//!
//! Every edit builds the *whole* candidate key list, validates it as a unit
//! (`camera_keys::validate_keys`) and only then replaces the clip's list, so a rejected command
//! leaves the clip untouched. The time base and the evaluation are documented in
//! [`crate::camera_keys`].

use serde_json::{Value, json};

use super::EditorState;
use super::error::{CommandError, Raw};
use super::timeline::{Clip, ClipKind};
use crate::camera_keys::{self, CameraKey, Easing};
use crate::id::Id;
use crate::time::Ratio;

/// One edit of a scene clip's camera keys.
#[derive(Debug, Clone, PartialEq)]
pub enum CameraKeysCommand {
    /// Adds a key, or replaces the key at the same scene time.
    Set { clip: String, key: CameraKey },
    /// Removes the key at exactly `time` (scene seconds).
    Remove { clip: String, time: Ratio },
    /// Replaces every key (an empty list removes them all).
    Replace { clip: String, keys: Vec<CameraKey> },
    /// Records the camera the clip shows at the playhead as a key at that scene time.
    Capture { clip: String, easing: Easing },
    /// Edits one field of the `index`-th key (sorted by time): `time` (seconds, millisecond
    /// resolution), `easing`, `fov`, `position_x|y|z`, `target_x|y|z`.
    Field {
        clip: String,
        index: usize,
        field: String,
        value: Raw,
    },
}

/// Fields `CameraKeysCommand::Field` understands.
pub const KEY_FIELDS: [&str; 9] = [
    "time",
    "easing",
    "fov",
    "position_x",
    "position_y",
    "position_z",
    "target_x",
    "target_y",
    "target_z",
];

fn unknown_field(field: &str) -> CommandError {
    CommandError::new(
        "unknown_field",
        format!("A camera key has no field `{field}`"),
    )
    .details(json!({ "fields": KEY_FIELDS }))
}

fn set_field(key: &mut CameraKey, field: &str, value: &Raw) -> Result<(), CommandError> {
    let axis = |name: &str| match name.chars().last() {
        Some('x') => Some(0),
        Some('y') => Some(1),
        Some('z') => Some(2),
        _ => None,
    };
    match field {
        "time" => {
            let seconds = value.number()?;
            let millis = (seconds * 1000.0).round();
            if !millis.is_finite() || millis.abs() > 1e12 {
                return Err(CommandError::invalid_value(
                    "Camera key `time` is a finite number of seconds",
                    json!({}),
                ));
            }
            key.time = Ratio::new(millis as i64, 1000).unwrap_or(key.time);
        }
        "easing" => {
            let text = value.text();
            key.easing = Easing::parse(&text).ok_or_else(|| {
                CommandError::invalid_value(
                    format!("`{text}` is not an easing"),
                    json!({ "options": Easing::labels() }),
                )
            })?;
        }
        "fov" => key.fov = value.number()?,
        name if name.starts_with("position_") && axis(name).is_some() => {
            key.position[axis(name).unwrap_or(0)] = value.number()?;
        }
        name if name.starts_with("target_") && axis(name).is_some() => {
            key.target[axis(name).unwrap_or(0)] = value.number()?;
        }
        other => return Err(unknown_field(other)),
    }
    Ok(())
}

impl EditorState {
    /// Scene time of `clip` at sequence `frame`: the clip's `source_start` plus the elapsed clip
    /// time at its speed. The same time the plan reports as `scene_time`.
    pub fn clip_scene_time(&self, clip: &Clip, frame: i64) -> Ratio {
        self.clip_source_time(clip, frame)
    }

    fn scene_clip(&self, clip: &str) -> Result<&Clip, CommandError> {
        let entry = self
            .timeline
            .clip(clip)
            .ok_or_else(|| CommandError::unknown_element("clip", clip))?;
        if entry.kind != ClipKind::Scene {
            return Err(
                CommandError::new("not_applicable", "Only scene clips carry camera keys")
                    .target(format!("clip:{}", entry.id)),
            );
        }
        Ok(entry)
    }

    pub(super) fn apply_camera_keys(
        &mut self,
        command: CameraKeysCommand,
    ) -> Result<Value, CommandError> {
        let (clip_ref, candidate) = match command {
            CameraKeysCommand::Set { clip, key } => {
                let mut keys = self.scene_clip(&clip)?.camera_keys.clone();
                camera_keys::upsert(&mut keys, key);
                (clip, keys)
            }
            CameraKeysCommand::Remove { clip, time } => {
                let mut keys = self.scene_clip(&clip)?.camera_keys.clone();
                let before = keys.len();
                keys.retain(|key| key.time != time);
                if keys.len() == before {
                    return Err(CommandError::new(
                        "unknown_element",
                        format!("Clip `{clip}` has no camera key at {time} s"),
                    )
                    .target(format!("clip:{clip}")));
                }
                (clip, keys)
            }
            CameraKeysCommand::Replace { clip, mut keys } => {
                self.scene_clip(&clip)?;
                camera_keys::sort_keys(&mut keys);
                (clip, keys)
            }
            CameraKeysCommand::Capture { clip, easing } => {
                let entry = self.scene_clip(&clip)?;
                let frame = self.timeline.playhead;
                if frame < entry.start || frame >= entry.end() {
                    return Err(CommandError::new(
                        "invalid_time",
                        format!(
                            "The playhead (frame {frame}) is outside clip `{}` ({}..{}): move it over the clip to key its camera",
                            entry.id,
                            entry.start,
                            entry.end()
                        ),
                    )
                    .target(format!("clip:{}", entry.id)));
                }
                let time = self.clip_scene_time(entry, frame);
                let camera = self.clip_camera_at(entry, frame);
                let mut keys = entry.camera_keys.clone();
                camera_keys::upsert(&mut keys, CameraKey::from_camera(time, &camera, easing));
                (clip, keys)
            }
            CameraKeysCommand::Field {
                clip,
                index,
                field,
                value,
            } => {
                let mut keys = self.scene_clip(&clip)?.camera_keys.clone();
                let count = keys.len();
                let key = keys.get_mut(index).ok_or_else(|| {
                    CommandError::new(
                        "unknown_element",
                        format!("Clip `{clip}` has {count} camera keys, none at index {index}"),
                    )
                    .target(format!("clip:{clip}"))
                })?;
                set_field(key, &field, &value)?;
                camera_keys::sort_keys(&mut keys);
                (clip, keys)
            }
        };
        self.commit_camera_keys(&clip_ref, candidate)
    }

    /// Validates `keys` as a unit and stores them, or changes nothing.
    fn commit_camera_keys(
        &mut self,
        clip: &str,
        keys: Vec<CameraKey>,
    ) -> Result<Value, CommandError> {
        let target = format!("clip:{}", self.scene_clip(clip)?.id);
        camera_keys::validate_keys(&keys).map_err(|error| error.target(target.clone()))?;
        let id: Id = self.scene_clip(clip)?.id;
        let orbiting = self
            .timeline
            .clip(id)
            .and_then(|entry| entry.camera)
            .is_some_and(|camera| camera.orbit != 0.0);
        if orbiting && !keys.is_empty() {
            return Err(CommandError::new(
                "camera_orbit_conflict",
                "The camera orbits: set `orbit` to 0 before keying it (keys replace the orbit)",
            )
            .target(target));
        }
        let Some(entry) = self.timeline.clips.iter_mut().find(|entry| entry.id == id) else {
            return Err(CommandError::unknown_element("clip", clip));
        };
        let changed = entry.camera_keys != keys;
        entry.camera_keys = keys;
        Ok(json!({
            "target": target,
            "field": "camera_keys",
            "count": entry.camera_keys.len(),
            "keys": camera_keys::keys_to_json(&entry.camera_keys),
            "changed": changed,
        }))
    }
}
