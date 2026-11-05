//! Which scene frame each output frame shows, in exact rational time.
//!
//! The range is `[from, from + frames)` in the scene's own edit frames. When the preset keeps the
//! scene's rate, output frame `i` is edit frame `from + i`. When it asks for another rate, output
//! frame `i` shows the edit frame that contains the instant `from/rate + i/output_rate`: frames
//! are repeated or dropped, never blended, and a `30000/1001` export does not drift against a
//! `24` scene because no float is ever involved (same rule as the project export, F028).

use crate::directed_scene::Bundle;
use crate::preset::{Geometry, Preset};
use crate::state::CommandError;
use crate::time::{Rate, Ratio};
use serde_json::json;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneExportSchedule {
    pub scene_rate: Rate,
    pub output_rate: Rate,
    /// First edit frame of the range.
    pub from: i64,
    /// Length of the range in edit frames (the duration the output must have).
    pub edit_frames: u64,
    /// Frames of the file: the duration at the output rate, rounded, at least one.
    pub frames: u64,
    /// Size the renderer draws at and the size of the file.
    pub geometry: Geometry,
}

fn range_error(
    message: impl Into<String>,
    from: i64,
    frames: Option<i64>,
    length: u64,
) -> CommandError {
    CommandError::new("invalid_range", message).details(json!({
        "from": from, "frames": frames, "edit_duration_frames": length,
    }))
}

fn time_error(message: &str) -> CommandError {
    CommandError::new("invalid_time", message)
}

impl SceneExportSchedule {
    pub fn new(
        bundle: &Bundle,
        preset: &Preset,
        from: i64,
        frames: Option<i64>,
    ) -> Result<Self, CommandError> {
        let length = bundle.edit_duration_frames;
        let total = i64::try_from(length).map_err(|_| time_error("The edit is too long"))?;
        if from < 0 || from >= total {
            return Err(range_error(
                format!("`from` {from} is outside the scene's edit (0..{total})"),
                from,
                frames,
                length,
            ));
        }
        let edit_frames = match frames {
            None => total - from,
            Some(count) if count >= 1 && count <= total - from => count,
            Some(count) => {
                return Err(range_error(
                    format!(
                        "`frames` {count} does not fit: {} frame(s) remain from frame {from}",
                        total - from
                    ),
                    from,
                    frames,
                    length,
                ));
            }
        };
        let scene_rate = bundle
            .fps
            .rate()
            .ok_or_else(|| time_error("The scene's frame rate is not valid"))?;
        let output_rate = preset.fps.unwrap_or(scene_rate);
        let duration = scene_rate.frame_to_time(edit_frames);
        let frames = Ratio::new(i64::from(output_rate.num()), i64::from(output_rate.den()))
            .and_then(|rate| duration.mul(rate))
            .map(Ratio::round)
            .ok_or_else(|| time_error("The duration at the output rate overflows"))?
            .max(1);
        let side = |value: u64| {
            u32::try_from(value).map_err(|_| time_error("The scene's picture size is too large"))
        };
        Ok(Self {
            scene_rate,
            output_rate,
            from,
            edit_frames: u64::try_from(edit_frames).map_err(|_| time_error("Invalid length"))?,
            frames: u64::try_from(frames).map_err(|_| time_error("Invalid frame count"))?,
            geometry: preset.geometry(
                side(bundle.render_profile.width)?,
                side(bundle.render_profile.height)?,
            ),
        })
    }

    /// End of the range, exclusive, in edit frames.
    pub fn to(&self) -> i64 {
        self.from.saturating_add_unsigned(self.edit_frames)
    }

    pub fn duration(&self) -> Ratio {
        self.scene_rate
            .frame_to_time(i64::try_from(self.edit_frames).unwrap_or(i64::MAX))
    }

    /// Seconds of the file: `frames / output_rate`.
    pub fn duration_seconds(&self) -> f64 {
        self.frames as f64 * f64::from(self.output_rate.den()) / f64::from(self.output_rate.num())
    }

    pub fn converts_rate(&self) -> bool {
        self.scene_rate != self.output_rate
    }

    /// The scene instant output frame `index` samples, exact.
    pub fn sample_time(&self, index: u64) -> Result<Ratio, CommandError> {
        let index = i64::try_from(index).map_err(|_| time_error("Frame index overflows"))?;
        let offset = Ratio::new(
            index.saturating_mul(i64::from(self.output_rate.den())),
            i64::from(self.output_rate.num()),
        )
        .ok_or_else(|| time_error("Frame time overflows"))?;
        self.scene_rate
            .frame_to_time(self.from)
            .add(offset)
            .ok_or_else(|| time_error("Frame time overflows"))
    }

    /// The edit frame output frame `index` shows. Always inside `[from, to)`.
    pub fn edit_frame(&self, bundle: &Bundle, index: u64) -> Result<i64, CommandError> {
        if index >= self.frames {
            return Err(CommandError::new(
                "invalid_range",
                format!(
                    "Output frame {index} is past the end of the export ({} frames)",
                    self.frames
                ),
            ));
        }
        let frame = if self.converts_rate() {
            bundle.frame_at_time(self.sample_time(index)?)?
        } else {
            self.from
                .checked_add_unsigned(index)
                .ok_or_else(|| time_error("Frame index overflows"))?
        };
        if frame < self.from || frame >= self.to() {
            // Unreachable by construction (the last sample is before `from + duration`); kept as
            // an error so a future change to the rounding cannot export frames outside the range.
            return Err(time_error("The sampled frame left the exported range"));
        }
        Ok(frame)
    }
}
