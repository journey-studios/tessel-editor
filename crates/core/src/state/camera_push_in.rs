//! A source-timed dolly over the authored camera. It works on still, keyed and orbiting cameras
//! without baking over their motion or needing capture metadata.

use serde_json::{Value, json};

use super::timeline::Clip;
use super::{CommandError, EditorState};
use crate::camera_keys::Easing;
use crate::content::Camera;
use crate::time::Ratio;

const DEFAULT_SECONDS: f64 = 1.25;
const DEFAULT_AMOUNT: f64 = 0.2;
const MAX_AMOUNT: f64 = 0.8;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PushInOptions {
    /// Fraction of the camera-to-target distance to travel. Default: 0.2.
    pub amount: Option<f64>,
    /// Number of final sequence frames including the original pose. Default: 1.25 seconds.
    pub frames: Option<i64>,
}

impl PushInOptions {
    pub(crate) fn from_json(value: &Value) -> Result<Self, CommandError> {
        Ok(Self {
            amount: value
                .get("amount")
                .map(|raw| raw.as_f64().ok_or_else(amount_error))
                .transpose()?,
            frames: value
                .get("frames")
                .map(|raw| raw.as_i64().ok_or_else(frames_error))
                .transpose()?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraPushIn {
    from: Ratio,
    to: Ratio,
    amount: f64,
}

fn amount_error() -> CommandError {
    CommandError::invalid_value(
        "Camera push-in amount must be greater than 0 and at most 0.8",
        json!({"max": MAX_AMOUNT}),
    )
}

fn frames_error() -> CommandError {
    CommandError::new(
        "invalid_time",
        "Camera push-in needs 2 or more frames inside the clip",
    )
}

impl CameraPushIn {
    fn new(from: Ratio, to: Ratio, amount: f64) -> Result<Self, CommandError> {
        if !amount.is_finite() || amount <= 0.0 || amount > MAX_AMOUNT {
            return Err(amount_error());
        }
        if from.is_negative() || to <= from || to.sub(from).is_none() {
            return Err(frames_error());
        }
        Ok(Self { from, to, amount })
    }

    pub(crate) fn from_json(value: &Value) -> Result<Self, CommandError> {
        Self::new(
            Ratio::from_json(&value["from"]).ok_or_else(frames_error)?,
            Ratio::from_json(&value["to"]).ok_or_else(frames_error)?,
            value["amount"].as_f64().ok_or_else(amount_error)?,
        )
    }

    pub fn to_json(self) -> Value {
        json!({ "from": self.from.to_json(), "to": self.to.to_json(), "amount": self.amount })
    }

    pub(crate) fn apply(self, camera: Camera, time: Ratio) -> Camera {
        if time <= self.from {
            return camera;
        }
        let fraction = if time >= self.to {
            1.0
        } else {
            let elapsed = time.to_f64() - self.from.to_f64();
            let duration = self.to.sub(self.from).expect("validated duration").to_f64();
            Easing::EaseInOutCubic.apply(elapsed / duration)
        };
        let [dx, dy, dz] = std::array::from_fn(|i| camera.position[i] - camera.target[i]);
        let horizontal = dx.hypot(dz);
        let distance = horizontal.hypot(dy);
        // Saturate smoothly at the valid distance on unusually small camera rigs.
        let min_remaining = (1.000_001e-6 / horizontal).max(1.000_001 * 1e-9_f64.sqrt() / distance);
        let amount = (self.amount * fraction).min((1.0 - min_remaining).max(0.0));
        if amount == 0.0 {
            return camera;
        }
        let mut moved = camera;
        moved.position = std::array::from_fn(|axis| {
            camera.position[axis] * (1.0 - amount) + camera.target[axis] * amount
        });
        // A valid base camera may already be right at the domain's minimum distance.
        // Keep the evaluated pose valid even when the splat's units are extremely small.
        if moved.validate().is_ok() {
            moved
        } else {
            camera
        }
    }
}

impl EditorState {
    fn push_in_range(&self, clip: &Clip, push_in: CameraPushIn) -> Option<[i64; 2]> {
        let last = clip.end() - 1;
        if self.clip_scene_time(clip, last) <= push_in.from {
            return None;
        }
        // Find the last unchanged pose so the preview starts before the motion. The search
        // handles trims, splits and speed edits without scanning a potentially long clip.
        let (mut low, mut high) = (clip.start, last);
        while low < high {
            let middle = low + (high - low) / 2;
            if self.clip_scene_time(clip, middle) <= push_in.from {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        Some([low.saturating_sub(1).max(clip.start), clip.end()])
    }

    /// A half-open range on the active sequence, derived from the clip's current placement.
    pub fn camera_push_in_preview_range(&self, clip: &str) -> Result<[i64; 2], CommandError> {
        let entry = self.scene_clip(clip)?;
        entry
            .camera_push_in
            .and_then(|effect| self.push_in_range(entry, effect))
            .ok_or_else(|| {
                CommandError::new("not_applicable", "This clip has no visible camera push-in")
                    .target(format!("clip:{}", entry.id))
            })
    }

    pub(super) fn set_camera_push_in(
        &mut self,
        clip: &str,
        options: Option<PushInOptions>,
    ) -> Result<Value, CommandError> {
        let entry = self.scene_clip(clip)?;
        let id = entry.id;
        let target = format!("clip:{id}");
        let result = options
            .map(|options| {
                let default_frames = (self.timeline.rate.as_f64() * DEFAULT_SECONDS).round() as i64;
                let frames = options
                    .frames
                    .unwrap_or(default_frames.max(2).min(entry.duration));
                if !(2..=entry.duration).contains(&frames) {
                    return Err(frames_error().target(target.clone()));
                }
                CameraPushIn::new(
                    self.clip_scene_time(entry, entry.end() - frames),
                    self.clip_scene_time(entry, entry.end() - 1),
                    options.amount.unwrap_or(DEFAULT_AMOUNT),
                )
                .map_err(|error| error.target(target.clone()))
            })
            .transpose()?;
        let range = result.and_then(|effect| self.push_in_range(entry, effect));
        let previous = entry.camera_push_in;
        // Replacing a longer adjustment also changes the frames where that adjustment was removed.
        let affected = match (
            range,
            previous.and_then(|effect| self.push_in_range(entry, effect)),
        ) {
            (Some([new, end]), Some([old, _])) => Some([new.min(old), end]),
            (Some(range), None) | (None, Some(range)) => Some(range),
            (None, None) => None,
        };
        let entry = self
            .timeline
            .clips
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| CommandError::unknown_element("clip", clip))?;
        entry.camera_push_in = result;
        Ok(json!({
            "target": target, "field": "camera_push_in",
            "changed": previous != result, "range": affected,
            "value": result.map(CameraPushIn::to_json),
        }))
    }
}
