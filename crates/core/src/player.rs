//! The player of a player clip (F071): a waveform, a progress bar and a timer tied to the
//! timeline.
//!
//! WHY shapes and text and not a new kind of layer: the compositor already draws rectangles and
//! sans text, with the same colours, rounding and transparency a user edits elsewhere. A player is
//! resolved here, at the frame being drawn, into those ordinary layers (the same way a group is,
//! F069), so the monitor, `render.frame` and the export draw it with the runtime they already
//! use and cannot disagree about it.
//!
//! The clock is the timeline's: progress and timer are the time elapsed since the start of the
//! linked audio clip (or of the player clip itself while nothing is linked) over that clip's
//! duration, both read from the sequence frame being drawn. Nothing here runs on its own.

use serde_json::{Map, Value, json};

use crate::content::{TextSpec, TitleAlignment, TitleFont};
use crate::group::{Placed, PlacedContent};
use crate::id::Id;
use crate::shape::{FillMode, ShapeSpec, is_color};
use crate::state::error::{CommandError, Raw};
use crate::text::SANS_LINE_PITCH;

/// Bars of a waveform: enough to read a shape, few enough to stay a few layers per frame.
pub const MIN_BARS: u32 = 8;
pub const MAX_BARS: u32 = 96;
/// The tallest part of a player, percent of the canvas.
pub const MAX_EXTENT: f64 = 100.0;
/// Share of a bar's height a silent stretch keeps, so the waveform still reads as a row of bars.
const SILENT_FLOOR: f64 = 0.04;

/// The specification of a player clip. Sizes are percent of the canvas (width for `width`, height
/// for the others), like the shapes and texts the player is drawn with.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerSpec {
    /// The audio clip of the sequence whose waveform, start and duration the player follows.
    /// `None` makes the player follow its own clip (a progress bar without a waveform).
    pub audio: Option<Id>,
    /// Width of the waveform, the track and the timer's line, percent of the canvas width.
    pub width: f64,
    /// Height of the waveform.
    pub height: f64,
    /// Number of bars of the waveform.
    pub bars: u32,
    /// Share of a bar's slot that is left empty, `0..=0.9`.
    pub gap: f64,
    /// Whether the waveform, the progress bar and the timer are drawn.
    pub wave: bool,
    pub bar: bool,
    pub time: bool,
    /// Bars the timeline has already passed.
    pub wave_color: String,
    /// Bars still to come.
    pub idle_color: String,
    /// The empty progress bar.
    pub track_color: String,
    /// The elapsed part of the progress bar.
    pub fill_color: String,
    /// `#rrggbb`.
    pub text_color: String,
    /// Thickness of the progress bar.
    pub bar_height: f64,
    /// Height of the timer's text.
    pub text_size: f64,
    /// Space between the parts that are drawn.
    pub spacing: f64,
}

impl Default for PlayerSpec {
    fn default() -> Self {
        Self {
            audio: None,
            width: 60.0,
            height: 14.0,
            bars: 48,
            gap: 0.4,
            wave: true,
            bar: true,
            time: true,
            wave_color: "#ffffff".to_string(),
            idle_color: "#5d6b82".to_string(),
            track_color: "#2a3344".to_string(),
            fill_color: "#4a90e2".to_string(),
            text_color: "#ffffff".to_string(),
            bar_height: 1.2,
            text_size: 3.2,
            spacing: 1.6,
        }
    }
}

/// Where the timeline is inside the span a player follows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerClock {
    /// `0..=1`.
    pub progress: f64,
    pub elapsed_s: f64,
    pub total_s: f64,
}

impl PlayerClock {
    /// The clock at `elapsed_s` of a span of `total_s` seconds; outside the span it is held at the
    /// ends (a player before its audio shows 0, after it shows the whole).
    pub fn at(elapsed_s: f64, total_s: f64) -> Self {
        let total_s = total_s.max(0.0);
        let elapsed_s = elapsed_s.clamp(0.0, total_s);
        let progress = if total_s > 0.0 {
            elapsed_s / total_s
        } else {
            0.0
        };
        Self {
            progress,
            elapsed_s,
            total_s,
        }
    }

    /// `m:ss`, or `h:mm:ss` from an hour on, whole seconds counted down (a timer never shows a
    /// second that has not finished).
    pub fn timecode(seconds: f64) -> String {
        let whole = seconds.max(0.0).floor() as u64;
        match whole / 3600 {
            0 => format!("{}:{:02}", whole / 60, whole % 60),
            hours => format!("{hours}:{:02}:{:02}", whole / 60 % 60, whole % 60),
        }
    }

    /// What the timer shows: `elapsed / total`.
    pub fn label(&self) -> String {
        // The total is rounded, not floored: a 29.97 s file is "0:30" long, and the last frame
        // must not show a timer longer than its total.
        let total = Self::timecode(self.total_s.round());
        // A player held at its end (the timeline is past the audio) reads the whole length,
        // however the total was rounded; before the end the timer counts finished seconds.
        let elapsed = if self.elapsed_s >= self.total_s {
            total.clone()
        } else {
            Self::timecode(self.elapsed_s.min(self.total_s.round()))
        };
        format!("{elapsed} / {total}")
    }
}

fn unknown_field(field: &str) -> CommandError {
    CommandError::new("unknown_field", format!("A player has no field `{field}`"))
        .details(json!({ "available": PlayerSpec::FIELDS }))
}

fn range_error(field: &str, min: f64, max: f64, unit: &str) -> CommandError {
    CommandError::invalid_value(
        format!("Player {field} is between {min} and {max} ({unit})"),
        json!({ "field": field, "min": min, "max": max, "unit": unit }),
    )
}

impl PlayerSpec {
    /// Every field, `audio` first.
    pub const FIELDS: [&'static str; 16] = [
        "audio",
        "width",
        "height",
        "bars",
        "gap",
        "wave",
        "bar",
        "time",
        "wave_color",
        "idle_color",
        "track_color",
        "fill_color",
        "text_color",
        "bar_height",
        "text_size",
        "spacing",
    ];

    pub fn validate(&self) -> Result<(), CommandError> {
        for (field, value, min, max, unit) in [
            (
                "width",
                self.width,
                1.0,
                MAX_EXTENT,
                "% of the canvas width",
            ),
            (
                "height",
                self.height,
                1.0,
                MAX_EXTENT,
                "% of the canvas height",
            ),
            (
                "bar_height",
                self.bar_height,
                0.1,
                20.0,
                "% of the canvas height",
            ),
            (
                "text_size",
                self.text_size,
                1.0,
                30.0,
                "% of the canvas height",
            ),
            ("spacing", self.spacing, 0.0, 20.0, "% of the canvas height"),
            ("gap", self.gap, 0.0, 0.9, "share of a bar's slot"),
        ] {
            if !value.is_finite() || !(min..=max).contains(&value) {
                return Err(range_error(field, min, max, unit));
            }
        }
        if !(MIN_BARS..=MAX_BARS).contains(&self.bars) {
            return Err(CommandError::invalid_value(
                format!("A player has {MIN_BARS} to {MAX_BARS} bars"),
                json!({ "field": "bars", "min": MIN_BARS, "max": MAX_BARS }),
            ));
        }
        for (field, value) in [
            ("wave_color", &self.wave_color),
            ("idle_color", &self.idle_color),
            ("track_color", &self.track_color),
            ("fill_color", &self.fill_color),
        ] {
            if !is_color(value) {
                return Err(CommandError::invalid_value(
                    format!("Player {field} is `#rrggbb` or `#rrggbbaa`"),
                    json!({ "field": field, "format": "#rrggbb or #rrggbbaa" }),
                ));
            }
        }
        // The timer is a text: its colour is the one a text has.
        TextSpec::new("0:00", self.text_size, self.text_color.clone()).map(|_| ())?;
        if !(self.wave || self.bar || self.time) {
            return Err(CommandError::invalid_value(
                "A player draws at least one of the waveform, the progress bar and the timer",
                json!({ "fields": ["wave", "bar", "time"] }),
            ));
        }
        Ok(())
    }

    /// The spec with one field replaced; the whole result is validated, so a refused value leaves
    /// the caller's spec untouched.
    pub fn with_field(&self, field: &str, value: &Raw) -> Result<Self, CommandError> {
        let mut next = self.clone();
        let color = || value.text().trim().to_ascii_lowercase();
        match field {
            "audio" => {
                let raw = value.text();
                let raw = raw.trim();
                next.audio = match raw {
                    "" | "none" => None,
                    clip => Some(Id::new(clip).map_err(|error| {
                        CommandError::invalid_value(
                            format!("A player's audio is a clip ID: {error}"),
                            json!({ "field": "audio" }),
                        )
                    })?),
                };
            }
            "width" => next.width = value.number()?,
            "height" => next.height = value.number()?,
            "bars" => {
                next.bars = u32::try_from(value.integer()?).map_err(|_| {
                    CommandError::invalid_value(
                        "Bars is a whole number",
                        json!({ "field": "bars" }),
                    )
                })?;
            }
            "gap" => next.gap = value.number()?,
            "wave" => next.wave = value.boolean()?,
            "bar" => next.bar = value.boolean()?,
            "time" => next.time = value.boolean()?,
            "wave_color" => next.wave_color = color(),
            "idle_color" => next.idle_color = color(),
            "track_color" => next.track_color = color(),
            "fill_color" => next.fill_color = color(),
            "text_color" => next.text_color = color(),
            "bar_height" => next.bar_height = value.number()?,
            "text_size" => next.text_size = value.number()?,
            "spacing" => next.spacing = value.number()?,
            _ => return Err(unknown_field(field)),
        }
        next.validate()?;
        Ok(next)
    }

    fn field_json(&self, field: &str) -> Value {
        match field {
            "audio" => self.audio.map_or(Value::Null, |audio| json!(audio)),
            "width" => json!(self.width),
            "height" => json!(self.height),
            "bars" => json!(self.bars),
            "gap" => json!(self.gap),
            "wave" => json!(self.wave),
            "bar" => json!(self.bar),
            "time" => json!(self.time),
            "wave_color" => json!(self.wave_color),
            "idle_color" => json!(self.idle_color),
            "track_color" => json!(self.track_color),
            "fill_color" => json!(self.fill_color),
            "text_color" => json!(self.text_color),
            "bar_height" => json!(self.bar_height),
            "text_size" => json!(self.text_size),
            "spacing" => json!(self.spacing),
            _ => Value::Null,
        }
    }

    /// Every field; an unlinked player writes `audio: null`, so a reader never has to guess.
    pub fn to_json(&self) -> Value {
        Value::Object(
            Self::FIELDS
                .iter()
                .map(|field| ((*field).to_string(), self.field_json(field)))
                .collect::<Map<_, _>>(),
        )
    }

    /// Absent fields take the defaults; unknown ones are refused so a typo cannot pass silently.
    pub fn from_json(value: &Value) -> Result<Self, CommandError> {
        let object = value
            .as_object()
            .ok_or_else(|| CommandError::new("invalid_request", "A player is an object"))?;
        if let Some(field) = object
            .keys()
            .find(|key| !Self::FIELDS.contains(&key.as_str()))
        {
            return Err(unknown_field(field));
        }
        let mut spec = Self::default();
        for field in Self::FIELDS {
            let Some(raw) = object.get(field).filter(|raw| !raw.is_null()) else {
                continue;
            };
            let raw = Raw::from_json(raw).ok_or_else(|| {
                CommandError::invalid_value(
                    format!("Player {field} is a string, a number or a flag"),
                    json!({ "field": field }),
                )
            })?;
            spec = spec.with_field(field, &raw)?;
        }
        spec.validate()?;
        Ok(spec)
    }

    /// The layers of the player at `clock`, bottom to top, centred on the clip's origin (the
    /// parts that are drawn are stacked: waveform, progress bar, timer). `peaks` are the bar
    /// heights `0..=1` of the linked audio (any length: it is resampled to `bars`); empty draws
    /// a row of flat bars. `canvas` is the sequence's size in pixels.
    pub fn place(&self, peaks: &[f32], clock: PlayerClock, canvas: (u32, u32)) -> Vec<Placed> {
        let (canvas_w, canvas_h) = (f64::from(canvas.0.max(1)), f64::from(canvas.1.max(1)));
        let timer_height = self.text_size * SANS_LINE_PITCH;
        let heights: Vec<f64> = [
            (self.wave, self.height),
            (self.bar, self.bar_height),
            (self.time, timer_height),
        ]
        .into_iter()
        .filter_map(|(drawn, height)| drawn.then_some(height))
        .collect();
        let total =
            heights.iter().sum::<f64>() + self.spacing * heights.len().saturating_sub(1) as f64;
        let mut top = -total / 2.0;
        let mut placed = Vec::new();

        if self.wave {
            placed.extend(self.bars_layers(peaks, clock.progress, top, (canvas_w, canvas_h)));
            top += self.height + self.spacing;
        }
        if self.bar {
            placed.extend(self.progress_layers(clock.progress, top, (canvas_w, canvas_h)));
            top += self.bar_height + self.spacing;
        }
        // `validate` checked the colour and the size and the label is digits, colons, a slash
        // and spaces, so the text is always made; a refusal would only drop the timer.
        if self.time
            && let Ok(mut text) =
                TextSpec::new(clock.label(), self.text_size, self.text_color.clone())
        {
            text.font = TitleFont::Sans;
            text.align = TitleAlignment::Left;
            placed.push(Placed {
                child: Id::lit("time"),
                content: PlacedContent::Text(text),
                // A left-aligned text is anchored at the top-left of its picture.
                x: -self.width / 2.0,
                y: top,
                scale: 1.0,
                rotation: 0.0,
                opacity: 1.0,
                visible: None,
            });
        }
        placed
    }

    fn bars_layers(
        &self,
        peaks: &[f32],
        progress: f64,
        top: f64,
        canvas: (f64, f64),
    ) -> Vec<Placed> {
        let count = self.bars as usize;
        let slot = self.width / count as f64;
        let bar_width = slot * (1.0 - self.gap);
        // `radius` is a share of the canvas height: a bar's rounded ends are half its width,
        // which is measured on the other axis.
        let round = bar_width * canvas.0 / canvas.1 / 2.0;
        let levels = resample(peaks, count);
        (0..count)
            .map(|index| {
                let level = f64::from(levels[index]).clamp(SILENT_FLOOR, 1.0);
                let height = self.height * level;
                let played = (index as f64 + 0.5) / count as f64 <= progress;
                let shape = ShapeSpec {
                    width: bar_width,
                    height,
                    radius: round.min(height / 2.0),
                    fill: if played {
                        self.wave_color.clone()
                    } else {
                        self.idle_color.clone()
                    },
                    fill_mode: FillMode::Solid,
                    ..ShapeSpec::default()
                };
                Placed {
                    child: bar_id(index),
                    content: PlacedContent::Shape(shape),
                    x: -self.width / 2.0 + slot * (index as f64 + 0.5),
                    y: top + self.height / 2.0,
                    scale: 1.0,
                    rotation: 0.0,
                    opacity: 1.0,
                    visible: None,
                }
            })
            .collect()
    }

    fn progress_layers(&self, progress: f64, top: f64, canvas: (f64, f64)) -> Vec<Placed> {
        let round = (self.bar_height / 2.0).min(self.width * canvas.0 / canvas.1 / 2.0);
        let rect = |width: f64, fill: &str| ShapeSpec {
            width,
            height: self.bar_height,
            radius: round,
            fill: fill.to_string(),
            ..ShapeSpec::default()
        };
        let y = top + self.bar_height / 2.0;
        let mut layers = vec![Placed {
            child: Id::lit("track"),
            content: PlacedContent::Shape(rect(self.width, &self.track_color)),
            x: 0.0,
            y,
            scale: 1.0,
            rotation: 0.0,
            opacity: 1.0,
            visible: None,
        }];
        let filled = self.width * progress;
        // Under a hundredth of a percent of the canvas width the fill is not a visible mark.
        if filled >= 0.01 {
            layers.push(Placed {
                child: Id::lit("fill"),
                content: PlacedContent::Shape(rect(filled, &self.fill_color)),
                x: -self.width / 2.0 + filled / 2.0,
                y,
                scale: 1.0,
                rotation: 0.0,
                opacity: 1.0,
                visible: None,
            });
        }
        layers
    }
}

fn bar_id(index: usize) -> Id {
    // `bar-<n>` is always a valid ID; the interner holds at most `MAX_BARS` of them.
    Id::lit(&format!("bar-{index}"))
}

/// `peaks` as `count` levels, normalised so the loudest bar is `1`. An empty or silent source
/// gives flat bars.
fn resample(peaks: &[f32], count: usize) -> Vec<f32> {
    if peaks.is_empty() {
        return vec![0.0; count];
    }
    let loudest = peaks.iter().fold(0.0f32, |top, peak| top.max(*peak));
    let scale = if loudest > 0.0 { 1.0 / loudest } else { 0.0 };
    (0..count)
        .map(|index| {
            let from = index * peaks.len() / count;
            let to = ((index + 1) * peaks.len() / count)
                .max(from + 1)
                .min(peaks.len());
            peaks[from..to]
                .iter()
                .fold(0.0f32, |top, peak| top.max(*peak))
                * scale
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_holds_at_the_ends_and_counts_whole_seconds() {
        let before = PlayerClock::at(-2.0, 30.0);
        assert_eq!((before.progress, before.elapsed_s), (0.0, 0.0));
        let after = PlayerClock::at(45.0, 30.0);
        assert_eq!((after.progress, after.elapsed_s), (1.0, 30.0));
        assert_eq!(PlayerClock::at(7.9, 30.0).label(), "0:07 / 0:30");
        assert_eq!(PlayerClock::at(3725.0, 3725.0).label(), "1:02:05 / 1:02:05");
        assert_eq!(PlayerClock::at(29.97, 29.97).label(), "0:30 / 0:30");
        assert_eq!(PlayerClock::at(1.0, 0.0).progress, 0.0);
    }

    #[test]
    fn a_player_round_trips_through_json_and_refuses_what_it_does_not_know() {
        let spec = PlayerSpec::default()
            .with_field("audio", &Raw::Text("narration".into()))
            .unwrap()
            .with_field("bars", &Raw::Number(32.0))
            .unwrap()
            .with_field("time", &Raw::Bool(false))
            .unwrap();
        assert_eq!(PlayerSpec::from_json(&spec.to_json()).unwrap(), spec);
        assert_eq!(
            PlayerSpec::from_json(&json!({})).unwrap(),
            PlayerSpec::default()
        );
        assert_eq!(
            PlayerSpec::from_json(&json!({ "colour": "#fff" }))
                .unwrap_err()
                .code,
            "unknown_field"
        );
    }

    #[test]
    fn a_refused_value_names_the_field_and_leaves_the_spec_alone() {
        let spec = PlayerSpec::default();
        for (field, value) in [
            ("bars", Raw::Number(2.0)),
            ("bars", Raw::Number(500.0)),
            ("gap", Raw::Number(1.0)),
            ("fill_color", Raw::Text("blue".into())),
            ("text_color", Raw::Text("#ffffff80".into())),
            ("width", Raw::Number(0.0)),
        ] {
            assert_eq!(
                spec.with_field(field, &value).unwrap_err().code,
                "invalid_value",
                "{field}"
            );
        }
        let none = spec
            .with_field("wave", &Raw::Bool(false))
            .unwrap()
            .with_field("bar", &Raw::Bool(false))
            .unwrap();
        assert!(none.with_field("time", &Raw::Bool(false)).is_err());
    }

    #[test]
    fn the_bars_follow_the_progress_and_the_loudness() {
        let spec = PlayerSpec {
            bars: 8,
            ..PlayerSpec::default()
        };
        let peaks = [0.0, 0.5, 1.0, 0.25];
        let clock = PlayerClock::at(15.0, 30.0);
        let placed = spec.place(&peaks, clock, (1920, 1080));
        let fills: Vec<(&str, f64)> = placed
            .iter()
            .filter_map(|part| match &part.content {
                PlacedContent::Shape(shape) if part.child.as_str().starts_with("bar-") => {
                    Some((shape.fill.as_str(), shape.height))
                }
                _ => None,
            })
            .collect();
        assert_eq!(fills.len(), 8);
        // Half way: the first four bars are played, the last four are not.
        assert!(fills[..4].iter().all(|(fill, _)| *fill == "#ffffff"));
        assert!(fills[4..].iter().all(|(fill, _)| *fill == "#5d6b82"));
        // The loudest source bucket makes the tallest bar, a silent one keeps the floor.
        let tallest = fills.iter().map(|(_, height)| *height).fold(0.0, f64::max);
        assert!((tallest - spec.height).abs() < 1e-9);
        assert!((fills[0].1 - spec.height * SILENT_FLOOR).abs() < 1e-9);
        // The timer reads the same clock.
        let timer = placed.last().unwrap();
        let PlacedContent::Text(text) = &timer.content else {
            panic!("the last part is the timer");
        };
        assert_eq!(text.content, "0:15 / 0:30");
        assert_eq!(text.align, TitleAlignment::Left);
    }

    #[test]
    fn only_the_parts_that_are_switched_on_are_placed() {
        let spec = PlayerSpec {
            wave: false,
            time: false,
            ..PlayerSpec::default()
        };
        let placed = spec.place(&[], PlayerClock::at(0.0, 10.0), (1280, 720));
        assert_eq!(
            placed.len(),
            1,
            "an empty fill is not drawn: only the track"
        );
        assert_eq!(placed[0].child, "track");
        let started = spec.place(&[], PlayerClock::at(5.0, 10.0), (1280, 720));
        let fill = started.iter().find(|part| part.child == "fill").unwrap();
        let PlacedContent::Shape(shape) = &fill.content else {
            panic!("the fill is a shape");
        };
        assert!((shape.width - spec.width / 2.0).abs() < 1e-9);
    }
}
