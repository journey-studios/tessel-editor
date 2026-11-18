//! The caption track of a subtitle clip (F071): timed cues, imported from and exported to SRT, and
//! the style they are drawn in.
//!
//! WHY the cues live in the clip and not in a file next to the project: the cues are edited,
//! undone, saved and reopened with the project like any other clip data, and an SRT is only the
//! interchange format (`subtitles.import`, `subtitles.export`).
//!
//! Cue times are milliseconds on the clip's *content* clock: the clip reads cue time
//! `source_start + elapsed` like a video reads its file, so trimming the head of the clip or
//! cutting it in two keeps every cue on the frame it was on, with no cue rewritten.
//!
//! Like a player (`crate::player`), a caption is resolved at the frame being drawn into ordinary
//! layers (an optional box and a sans text), so the monitor and the export draw it the same way.

use serde_json::{Map, Value, json};

use crate::content::{TextSpec, TitleAlignment, TitleFont};
use crate::group::{Placed, PlacedContent};
use crate::id::Id;
use crate::shape::{SHAPE_MAX_EXTENT, ShapeSpec, is_color};
use crate::state::error::{CommandError, Raw};
use crate::text;
use crate::time::Ratio;

/// Cues a track holds: a cap keeps the document and every undo snapshot bounded.
pub const MAX_CUES: usize = 2000;
/// Longest cue time, one day: a typo of a digit is an error and not a clip nobody can scroll to.
pub const MAX_CUE_MS: i64 = 86_400_000;
/// The value of `box` that draws no box behind the text.
pub const NO_BOX: &str = "none";

/// One caption: shown from `start_ms` (included) to `end_ms` (excluded).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cue {
    pub start_ms: i64,
    pub end_ms: i64,
    /// Normalised text (NFC, only characters the font draws); `\n` breaks a line.
    pub text: String,
}

impl Cue {
    pub fn to_json(&self) -> Value {
        json!({ "start_ms": self.start_ms, "end_ms": self.end_ms, "text": self.text })
    }

    fn from_json(value: &Value) -> Result<Self, CommandError> {
        let object = value
            .as_object()
            .ok_or_else(|| CommandError::new("invalid_request", "A cue is an object"))?;
        if let Some(field) = object
            .keys()
            .find(|key| !matches!(key.as_str(), "start_ms" | "end_ms" | "text"))
        {
            return Err(CommandError::new(
                "unknown_field",
                format!("A cue has no field `{field}`"),
            )
            .details(json!({ "available": ["start_ms", "end_ms", "text"] })));
        }
        let time = |field: &str| {
            object
                .get(field)
                .and_then(Value::as_i64)
                .ok_or_else(|| time_error(field))
        };
        let text = object
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| CommandError::invalid_value("A cue needs `text`", json!("string")))?;
        Ok(Self {
            start_ms: time("start_ms")?,
            end_ms: time("end_ms")?,
            text: text::normalize(text)?,
        })
    }
}

fn time_error(field: &str) -> CommandError {
    CommandError::invalid_value(
        format!("Cue {field} is a whole number of milliseconds"),
        json!({ "field": field, "max": MAX_CUE_MS }),
    )
}

/// The caption style and the cues of a subtitle clip. Sizes are percent of the canvas (width for
/// `width`, height for the others), like the text and shape a caption is drawn with.
#[derive(Debug, Clone, PartialEq)]
pub struct SubtitleSpec {
    /// Sorted by start, each one ending before the next one starts.
    pub cues: Vec<Cue>,
    /// Height of the text.
    pub size: f64,
    /// `#rrggbb`.
    pub color: String,
    /// Width of the box the text wraps in, percent of the canvas width.
    pub width: f64,
    /// Where the middle of the caption sits, percent of the canvas height from the centre
    /// (positive is down).
    pub y: f64,
    /// Colour of the box behind the text (`#rrggbb` or `#rrggbbaa`), or `none`.
    pub box_color: String,
    /// Room between the text and the edge of its box, percent of the canvas height.
    pub pad: f64,
    /// Corner radius of the box, percent of the canvas height.
    pub radius: f64,
    /// Whether an export draws the captions into the picture. Off, the export leaves them out
    /// and writes them as an `.srt` next to the video; the monitor shows them either way.
    pub burn: bool,
}

impl Default for SubtitleSpec {
    fn default() -> Self {
        Self {
            cues: Vec::new(),
            size: 4.6,
            color: "#ffffff".to_string(),
            width: 80.0,
            y: 38.0,
            box_color: "#000000b3".to_string(),
            pad: 1.0,
            radius: 0.8,
            burn: true,
        }
    }
}

fn unknown_field(what: &str, field: &str, available: &[&str]) -> CommandError {
    CommandError::new("unknown_field", format!("{what} has no field `{field}`"))
        .details(json!({ "available": available }))
}

fn range_error(field: &str, min: f64, max: f64, unit: &str) -> CommandError {
    CommandError::invalid_value(
        format!("Subtitle {field} is between {min} and {max} ({unit})"),
        json!({ "field": field, "min": min, "max": max, "unit": unit }),
    )
}

fn out_of_range(index: usize, count: usize) -> CommandError {
    CommandError::new(
        "out_of_bounds",
        format!("There is no cue {index}: the track has {count}"),
    )
    .details(json!({ "index": index, "count": count }))
}

impl SubtitleSpec {
    /// The style fields, in the order a document writes them.
    pub const STYLE_FIELDS: [&'static str; 8] = [
        "size", "color", "width", "y", "box", "pad", "radius", "burn",
    ];
    /// The fields of a cue.
    pub const CUE_FIELDS: [&'static str; 3] = ["start_ms", "end_ms", "text"];

    pub fn validate(&self) -> Result<(), CommandError> {
        for (field, value, min, max, unit) in [
            ("size", self.size, 1.0, 30.0, "% of the canvas height"),
            ("width", self.width, 5.0, 100.0, "% of the canvas width"),
            ("y", self.y, -50.0, 50.0, "% of the canvas height"),
            ("pad", self.pad, 0.0, 20.0, "% of the canvas height"),
            ("radius", self.radius, 0.0, 20.0, "% of the canvas height"),
        ] {
            if !value.is_finite() || !(min..=max).contains(&value) {
                return Err(range_error(field, min, max, unit));
            }
        }
        // The caption is a text: its colour and size are the ones a text has.
        TextSpec::new("x", self.size, self.color.clone())?;
        if self.box_color != NO_BOX && !is_color(&self.box_color) {
            return Err(CommandError::invalid_value(
                "Subtitle box is `#rrggbb`, `#rrggbbaa` or `none`",
                json!({ "field": "box", "format": "#rrggbb, #rrggbbaa or none" }),
            ));
        }
        self.validate_cues()
    }

    fn validate_cues(&self) -> Result<(), CommandError> {
        if self.cues.len() > MAX_CUES {
            return Err(CommandError::invalid_value(
                format!("A caption track has at most {MAX_CUES} cues"),
                json!({ "max": MAX_CUES }),
            ));
        }
        for (index, cue) in self.cues.iter().enumerate() {
            let at = json!({ "cue": index });
            if !(0..=MAX_CUE_MS).contains(&cue.start_ms) || !(0..=MAX_CUE_MS).contains(&cue.end_ms)
            {
                return Err(time_error("start_ms / end_ms").details(at));
            }
            if cue.end_ms <= cue.start_ms {
                return Err(CommandError::new(
                    "invalid_duration",
                    format!(
                        "Cue {index} ends ({} ms) before it starts ({} ms)",
                        cue.end_ms, cue.start_ms
                    ),
                )
                .details(at));
            }
            if cue.text.is_empty() || cue.text.chars().count() > crate::content::TEXT_MAX_CHARS {
                return Err(CommandError::invalid_value(
                    "A cue's text has 1 to 256 characters",
                    json!({ "cue": index, "max_chars": crate::content::TEXT_MAX_CHARS }),
                ));
            }
            if text::normalize(&cue.text)? != cue.text {
                return Err(CommandError::new(
                    "unsupported_text",
                    "A cue's text must be in its composed (NFC) spelling",
                )
                .details(json!({ "cue": index })));
            }
            if let Some(next) = self.cues.get(index + 1)
                && next.start_ms < cue.end_ms
            {
                return Err(CommandError::new(
                    "cue_overlap",
                    format!(
                        "Cue {} starts ({} ms) before cue {index} ends ({} ms): cues do not overlap",
                        index + 1,
                        next.start_ms,
                        cue.end_ms
                    ),
                )
                .details(json!({ "cue": index, "next": index + 1 })));
            }
        }
        Ok(())
    }

    /// The spec with one style field replaced (cues are edited with the `*_cue` methods).
    pub fn with_style(&self, field: &str, value: &Raw) -> Result<Self, CommandError> {
        let mut next = self.clone();
        let color = || value.text().trim().to_ascii_lowercase();
        match field {
            "size" => next.size = value.number()?,
            "color" => next.color = color(),
            "width" => next.width = value.number()?,
            "y" => next.y = value.number()?,
            "box" => next.box_color = color(),
            "pad" => next.pad = value.number()?,
            "radius" => next.radius = value.number()?,
            "burn" => next.burn = value.boolean()?,
            _ => return Err(unknown_field("A caption track", field, &Self::STYLE_FIELDS)),
        }
        next.validate()?;
        Ok(next)
    }

    /// The spec with the cue `index` changed in one field.
    pub fn with_cue_field(
        &self,
        index: usize,
        field: &str,
        value: &Raw,
    ) -> Result<Self, CommandError> {
        let mut next = self.clone();
        let count = next.cues.len();
        let cue = next
            .cues
            .get_mut(index)
            .ok_or_else(|| out_of_range(index, count))?;
        match field {
            "start_ms" => cue.start_ms = value.integer()?,
            "end_ms" => cue.end_ms = value.integer()?,
            "text" => cue.text = text::normalize(&value.text())?,
            _ => return Err(unknown_field("A cue", field, &Self::CUE_FIELDS)),
        }
        next.validate()?;
        Ok(next)
    }

    /// The spec with a cue added where its start puts it, and the index it got.
    pub fn with_cue_added(&self, cue: Cue) -> Result<(Self, usize), CommandError> {
        let mut next = self.clone();
        let index = next
            .cues
            .iter()
            .position(|other| other.start_ms > cue.start_ms)
            .unwrap_or(next.cues.len());
        next.cues.insert(index, cue);
        next.validate()?;
        Ok((next, index))
    }

    pub fn with_cue_removed(&self, index: usize) -> Result<Self, CommandError> {
        if index >= self.cues.len() {
            return Err(out_of_range(index, self.cues.len()));
        }
        let mut next = self.clone();
        next.cues.remove(index);
        Ok(next)
    }

    /// The spec with `cues` instead of its own, in the order of their start.
    pub fn with_cues(&self, mut cues: Vec<Cue>) -> Result<Self, CommandError> {
        cues.sort_by_key(|cue| cue.start_ms);
        let mut next = self.clone();
        next.cues = cues;
        next.validate()?;
        Ok(next)
    }

    /// The cue shown at `time` (seconds on the clip's content clock), if any. Compared as exact
    /// fractions of a second, so a frame that falls on a cue's edge reads it the same on every
    /// frame rate.
    pub fn active(&self, time: Ratio) -> Option<&Cue> {
        let millis = |ms: i64| Ratio::new(ms, 1000);
        self.cues.iter().find(|cue| {
            millis(cue.start_ms).is_some_and(|start| start <= time)
                && millis(cue.end_ms).is_some_and(|end| time < end)
        })
    }

    /// End of the last cue, in milliseconds: how long a clip has to be to show them all.
    pub fn last_end_ms(&self) -> i64 {
        self.cues.last().map_or(0, |cue| cue.end_ms)
    }

    /// The layers of the caption shown at `time` (a box, then the text), centred on the clip's
    /// origin; empty between cues. `canvas` is the sequence's size in pixels.
    pub fn place(&self, time: Ratio, canvas: (u32, u32)) -> Vec<Placed> {
        let Some(cue) = self.active(time) else {
            return Vec::new();
        };
        // `validate` checked the colour and the size, and the text was normalised on its way in,
        // so a refusal here would only mean "draw nothing".
        let Ok(mut spec) = TextSpec::new(cue.text.clone(), self.size, self.color.clone()) else {
            return Vec::new();
        };
        spec.font = TitleFont::Sans;
        spec.align = TitleAlignment::Center;
        spec.width = self.width;
        let mut placed = Vec::with_capacity(2);
        if self.box_color != NO_BOX {
            let lines = text::layout_sans(&spec.content, spec.size, canvas.0, canvas.1, spec.width);
            let (block_w, block_h) = text::sans_block(&lines.lines, spec.size, canvas.0, canvas.1);
            // The padding is the same number of pixels on both axes.
            let pad_x = self.pad * f64::from(canvas.1) / f64::from(canvas.0.max(1));
            let shape = ShapeSpec {
                width: (block_w + 2.0 * pad_x).clamp(0.1, SHAPE_MAX_EXTENT),
                height: block_h + 2.0 * self.pad,
                radius: self.radius,
                fill: self.box_color.clone(),
                ..ShapeSpec::default()
            };
            placed.push(Placed {
                child: Id::lit("box"),
                content: PlacedContent::Shape(shape),
                x: 0.0,
                y: self.y,
                scale: 1.0,
                rotation: 0.0,
                opacity: 1.0,
                visible: None,
            });
        }
        placed.push(Placed {
            child: Id::lit("text"),
            content: PlacedContent::Text(spec),
            x: 0.0,
            y: self.y,
            scale: 1.0,
            rotation: 0.0,
            opacity: 1.0,
            visible: None,
        });
        placed
    }

    fn style_json(&self, field: &str) -> Value {
        match field {
            "size" => json!(self.size),
            "color" => json!(self.color),
            "width" => json!(self.width),
            "y" => json!(self.y),
            "box" => json!(self.box_color),
            "pad" => json!(self.pad),
            "radius" => json!(self.radius),
            "burn" => json!(self.burn),
            _ => Value::Null,
        }
    }

    /// The style fields and the cues, as the document and the tree state them.
    pub fn to_json(&self) -> Value {
        let mut value = Map::new();
        for field in Self::STYLE_FIELDS {
            value.insert(field.to_string(), self.style_json(field));
        }
        value.insert(
            "cues".to_string(),
            Value::Array(self.cues.iter().map(Cue::to_json).collect()),
        );
        Value::Object(value)
    }

    /// Absent style fields take the defaults and absent cues mean none; unknown fields are
    /// refused so a typo cannot pass silently.
    pub fn from_json(value: &Value) -> Result<Self, CommandError> {
        let object = value
            .as_object()
            .ok_or_else(|| CommandError::new("invalid_request", "A caption track is an object"))?;
        if let Some(field) = object
            .keys()
            .find(|key| *key != "cues" && !Self::STYLE_FIELDS.contains(&key.as_str()))
        {
            return Err(unknown_field("A caption track", field, &Self::STYLE_FIELDS));
        }
        let mut spec = Self::default();
        for field in Self::STYLE_FIELDS {
            let Some(raw) = object.get(field) else {
                continue;
            };
            let raw = Raw::from_json(raw).ok_or_else(|| {
                CommandError::invalid_value(
                    format!("Subtitle {field} is a string, a number or a flag"),
                    json!({ "field": field }),
                )
            })?;
            spec = spec.with_style(field, &raw)?;
        }
        if let Some(cues) = object.get("cues") {
            let cues = cues
                .as_array()
                .ok_or_else(|| CommandError::invalid_value("`cues` is a list", json!("array")))?
                .iter()
                .map(Cue::from_json)
                .collect::<Result<Vec<_>, _>>()?;
            // A document stores them sorted: an unsorted list is a damaged one, not to be
            // quietly reordered.
            spec.cues = cues;
        }
        spec.validate()?;
        Ok(spec)
    }
}

// ---- SRT -------------------------------------------------------------------------------

fn srt_error(cue: usize, message: impl Into<String>) -> CommandError {
    CommandError::new("invalid_srt", message).details(json!({ "cue": cue }))
}

/// `HH:MM:SS,mmm` (a `.` is accepted for the comma, and 1 to 3 digits for the fraction).
fn parse_srt_time(raw: &str) -> Option<i64> {
    let (clock, fraction) = raw.split_once([',', '.']).unwrap_or((raw, "0"));
    let mut parts = clock.split(':');
    let hours: i64 = parts.next()?.trim().parse().ok()?;
    let minutes: i64 = parts.next()?.trim().parse().ok()?;
    let seconds: i64 = parts.next()?.trim().parse().ok()?;
    if parts.next().is_some() || minutes > 59 || seconds > 59 {
        return None;
    }
    let fraction = fraction.trim();
    if fraction.is_empty() || fraction.len() > 3 || !fraction.chars().all(|ch| ch.is_ascii_digit())
    {
        return None;
    }
    let millis: i64 = format!("{fraction:0<3}").parse().ok()?;
    Some(((hours * 60 + minutes) * 60 + seconds) * 1000 + millis)
}

/// Text the font can draw: the typographic punctuation an SRT usually carries is turned into its
/// plain spelling first (the built-in font covers printable ASCII and Western European accents).
fn plain_punctuation(line: &str) -> String {
    line.chars()
        .map(|ch| match ch {
            '\u{2018}' | '\u{2019}' | '\u{201a}' => "'".to_string(),
            '\u{201c}' | '\u{201d}' | '\u{201e}' => "\"".to_string(),
            '\u{2013}' | '\u{2014}' | '\u{2212}' => "-".to_string(),
            '\u{2026}' => "...".to_string(),
            '\u{a0}' | '\u{2009}' | '\u{202f}' => " ".to_string(),
            other => other.to_string(),
        })
        .collect()
}

/// `line` without the markup an SRT may carry: `<i>`-style tags and `{\an8}`-style overrides.
fn strip_markup(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut closing: Option<char> = None;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match (closing, ch) {
            (Some(end), _) if ch == end => closing = None,
            (Some(_), _) => {}
            (None, '<')
                if chars
                    .peek()
                    .is_some_and(|next| next.is_alphabetic() || *next == '/') =>
            {
                closing = Some('>');
            }
            (None, '{') if chars.peek() == Some(&'\\') => closing = Some('}'),
            (None, _) => out.push(ch),
        }
    }
    out
}

impl SubtitleSpec {
    /// The cues of an SRT file. All or nothing: the first cue that cannot be read is named
    /// (by its position in the file) and nothing is returned. Cues are sorted by start; two
    /// that overlap are refused (`cue_overlap`), and so is text the font cannot draw.
    pub fn parse_srt(source: &str) -> Result<Vec<Cue>, CommandError> {
        let source = source.trim_start_matches('\u{feff}').replace("\r\n", "\n");
        let source = source.replace('\r', "\n");
        let mut cues = Vec::new();
        let mut block: Vec<&str> = Vec::new();
        for line in source.split('\n').chain(std::iter::once("")) {
            if !line.trim().is_empty() {
                block.push(line);
                continue;
            }
            if block.is_empty() {
                continue;
            }
            let number = cues.len() + 1;
            cues.push(parse_srt_block(&block, number)?);
            block.clear();
        }
        if cues.is_empty() {
            return Err(CommandError::new("invalid_srt", "The SRT has no cues"));
        }
        Ok(cues)
    }

    /// The track as an SRT file (`HH:MM:SS,mmm`, numbered from 1, a blank line after each cue).
    pub fn to_srt(&self) -> String {
        write_srt(&self.cues)
    }

    /// The cues moved by `offset_ms` and cut to the window `[from_ms, to_ms)` of the new clock,
    /// the ones the window leaves nothing of dropped. WHY: an export writes the captions it did
    /// not draw on the exported video's clock (zero at its first frame), not on the clip's
    /// content clock, or the sidecar file would be late by the clip's start on the timeline.
    pub fn retimed(&self, offset_ms: i64, from_ms: i64, to_ms: i64) -> Vec<Cue> {
        self.cues
            .iter()
            .filter_map(|cue| {
                let start_ms = (cue.start_ms + offset_ms).max(from_ms);
                let end_ms = (cue.end_ms + offset_ms).min(to_ms);
                (end_ms > start_ms).then(|| Cue {
                    start_ms,
                    end_ms,
                    text: cue.text.clone(),
                })
            })
            .collect()
    }
}

/// Largest SRT file `read_srt_file` reads: a feature film's is a few hundred kilobytes.
pub const MAX_SRT_BYTES: u64 = 2 * 1024 * 1024;

/// The text of the SRT file at `path`, for `subtitles.import` and the inspector's import field,
/// so both refuse the same files for the same reasons.
pub fn read_srt_file(path: &str) -> Result<String, CommandError> {
    let io_error = |error: std::io::Error| {
        CommandError::new("io_error", format!("Cannot read `{path}`: {error}"))
    };
    let size = std::fs::metadata(path).map_err(io_error)?.len();
    if size > MAX_SRT_BYTES {
        return Err(CommandError::invalid_value(
            format!("The SRT is {size} bytes; the most it can be is {MAX_SRT_BYTES}"),
            json!({ "max_bytes": MAX_SRT_BYTES }),
        ));
    }
    // WHY lossy: SRT files in the wild are often Latin-1; an invalid byte becomes a replacement
    // character, which the font refuses with the cue named, rather than the whole file being
    // unreadable.
    let bytes = std::fs::read(path).map_err(io_error)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
/// `cues` as an SRT file (`HH:MM:SS,mmm`, numbered from 1, a blank line after each cue).
pub fn write_srt(cues: &[Cue]) -> String {
    let clock = |ms: i64| {
        format!(
            "{:02}:{:02}:{:02},{:03}",
            ms / 3_600_000,
            ms / 60_000 % 60,
            ms / 1000 % 60,
            ms % 1000
        )
    };
    cues.iter()
        .enumerate()
        .map(|(index, cue)| {
            format!(
                "{}\n{} --> {}\n{}\n",
                index + 1,
                clock(cue.start_ms),
                clock(cue.end_ms),
                cue.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_srt_block(block: &[&str], number: usize) -> Result<Cue, CommandError> {
    let Some(timing) = block.iter().position(|line| line.contains("-->")) else {
        return Err(srt_error(
            number,
            format!("Cue {number} of the SRT has no `-->` timing line"),
        ));
    };
    // What comes before the timing is the cue's number, which is ignored but must look like one.
    if block[..timing]
        .iter()
        .any(|line| line.trim().parse::<u64>().is_err())
    {
        return Err(srt_error(
            number,
            format!("Cue {number} of the SRT has text before its timing line"),
        ));
    }
    let (from, to) = block[timing].split_once("-->").unwrap_or_default();
    // Players that position a cue write coordinates after the end time; only the time is read.
    let to = to.split_whitespace().next().unwrap_or_default();
    let (Some(start_ms), Some(end_ms)) = (parse_srt_time(from.trim()), parse_srt_time(to)) else {
        return Err(srt_error(
            number,
            format!(
                "Cue {number} of the SRT has a timing that is not HH:MM:SS,mmm --> HH:MM:SS,mmm"
            ),
        ));
    };
    let lines: Vec<String> = block[timing + 1..]
        .iter()
        .map(|line| plain_punctuation(strip_markup(line).trim()))
        .filter(|line| !line.is_empty())
        .collect();
    if lines.is_empty() {
        return Err(srt_error(
            number,
            format!("Cue {number} of the SRT has no text"),
        ));
    }
    let text = text::normalize(&lines.join("\n")).map_err(|error| {
        srt_error(
            number,
            format!("Cue {number} of the SRT: {}", error.message),
        )
    })?;
    Ok(Cue {
        start_ms,
        end_ms,
        text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cue(start_ms: i64, end_ms: i64, text: &str) -> Cue {
        Cue {
            start_ms,
            end_ms,
            text: text.to_string(),
        }
    }

    const SAMPLE: &str = "\u{feff}1\r\n00:00:01,000 --> 00:00:02,500\r\nHello, <i>world</i>\r\n\r\n2\r\n00:00:03,000 --> 00:00:04,000 X1:100\r\n{\\an8}\u{201c}Second\u{201d} cue\r\nin two lines\r\n";

    #[test]
    fn an_srt_is_read_with_its_markup_and_typography_made_plain() {
        let cues = SubtitleSpec::parse_srt(SAMPLE).unwrap();
        assert_eq!(
            cues,
            vec![
                cue(1000, 2500, "Hello, world"),
                cue(3000, 4000, "\"Second\" cue\nin two lines"),
            ]
        );
    }

    #[test]
    fn writing_an_srt_and_reading_it_back_is_the_identity() {
        let spec = SubtitleSpec::default()
            .with_cues(vec![
                cue(1000, 2500, "Hello"),
                cue(3_723_004, 3_725_000, "A\nB"),
            ])
            .unwrap();
        let srt = spec.to_srt();
        assert!(srt.contains("00:00:01,000 --> 00:00:02,500"));
        assert!(srt.contains("01:02:03,004 --> 01:02:05,000"));
        assert_eq!(SubtitleSpec::parse_srt(&srt).unwrap(), spec.cues);
    }

    #[test]
    fn a_bad_srt_names_the_cue_and_returns_nothing() {
        for (source, code) in [
            ("", "invalid_srt"),
            ("1\n00:00:01,000 -> 00:00:02,000\nA\n", "invalid_srt"),
            ("1\n00:00:01,000 --> 00:00:xx,000\nA\n", "invalid_srt"),
            ("1\n00:00:01,000 --> 00:00:02,000\n\n", "invalid_srt"),
            (
                "1\n00:00:01,000 --> 00:00:02,000\nA \u{20ac} B\n",
                "invalid_srt",
            ),
            ("1\n00:00:09,000 --> 00:00:02,000\nA\n", "invalid_duration"),
        ] {
            let error = SubtitleSpec::parse_srt(source)
                .and_then(|cues| SubtitleSpec::default().with_cues(cues).map(|_| ()));
            assert_eq!(error.unwrap_err().code, code, "{source:?}");
        }
        let overlap = SubtitleSpec::parse_srt(
            "1\n00:00:01,000 --> 00:00:03,000\nA\n\n2\n00:00:02,000 --> 00:00:04,000\nB\n",
        )
        .unwrap();
        assert_eq!(
            SubtitleSpec::default().with_cues(overlap).unwrap_err().code,
            "cue_overlap"
        );
    }

    #[test]
    fn the_active_cue_is_exact_at_its_edges_on_any_frame_rate() {
        let spec = SubtitleSpec::default()
            .with_cues(vec![cue(1000, 2000, "A")])
            .unwrap();
        // 24000/1001 fps: frame 24 starts at 1.001 s, frame 23 at 0.959 s.
        let rate = crate::time::Rate::new(24000, 1001).unwrap();
        assert!(spec.active(rate.frame_to_time(23)).is_none());
        assert!(spec.active(rate.frame_to_time(24)).is_some());
        assert!(spec.active(Ratio::new(1999, 1000).unwrap()).is_some());
        assert!(spec.active(Ratio::from_int(2)).is_none());
        assert!(spec.active(Ratio::ZERO).is_none());
    }

    #[test]
    fn editing_keeps_the_track_sorted_and_refuses_what_overlaps() {
        let spec = SubtitleSpec::default()
            .with_cues(vec![cue(1000, 2000, "A"), cue(5000, 6000, "C")])
            .unwrap();
        let (spec, index) = spec.with_cue_added(cue(3000, 4000, "B")).unwrap();
        assert_eq!(index, 1);
        assert_eq!(
            spec.cues
                .iter()
                .map(|cue| cue.text.as_str())
                .collect::<Vec<_>>(),
            ["A", "B", "C"]
        );
        assert_eq!(
            spec.with_cue_field(1, "end_ms", &Raw::Number(5500.0))
                .unwrap_err()
                .code,
            "cue_overlap"
        );
        assert_eq!(
            spec.with_cue_field(1, "text", &Raw::Text(String::new()))
                .unwrap_err()
                .code,
            "invalid_value"
        );
        assert_eq!(
            spec.with_cue_field(9, "text", &Raw::Text("x".into()))
                .unwrap_err()
                .code,
            "out_of_bounds"
        );
        assert_eq!(spec.with_cue_removed(0).unwrap().cues.len(), 2);
        assert!(spec.with_cue_removed(3).is_err());
    }

    #[test]
    fn the_style_round_trips_and_refuses_what_it_does_not_know() {
        let spec = SubtitleSpec::default()
            .with_cues(vec![cue(0, 1500, "Olá, mundo")])
            .unwrap()
            .with_style("box", &Raw::Text("none".into()))
            .unwrap()
            .with_style("burn", &Raw::Bool(false))
            .unwrap();
        assert_eq!(SubtitleSpec::from_json(&spec.to_json()).unwrap(), spec);
        for (field, value) in [
            ("size", Raw::Number(0.0)),
            ("color", Raw::Text("red".into())),
            ("box", Raw::Text("#12".into())),
            ("y", Raw::Number(80.0)),
        ] {
            assert_eq!(
                spec.with_style(field, &value).unwrap_err().code,
                "invalid_value",
                "{field}"
            );
        }
        assert_eq!(
            SubtitleSpec::from_json(&json!({ "font": "x" }))
                .unwrap_err()
                .code,
            "unknown_field"
        );
        assert_eq!(
            SubtitleSpec::from_json(&json!({ "cues": [
                { "start_ms": 2000, "end_ms": 3000, "text": "B" },
                { "start_ms": 0, "end_ms": 1000, "text": "A" },
            ] }))
            .unwrap_err()
            .code,
            "cue_overlap",
            "a document with unsorted cues is damaged"
        );
    }

    #[test]
    fn a_caption_is_a_box_and_a_text_only_while_a_cue_is_active() {
        let spec = SubtitleSpec::default()
            .with_cues(vec![cue(1000, 2000, "Hello there")])
            .unwrap();
        assert!(spec.place(Ratio::ZERO, (1920, 1080)).is_empty());
        let placed = spec.place(Ratio::new(3, 2).unwrap(), (1920, 1080));
        assert_eq!(placed.len(), 2);
        assert!(matches!(placed[0].content, PlacedContent::Shape(_)));
        let PlacedContent::Text(text) = &placed[1].content else {
            panic!("the caption's text is on top");
        };
        assert_eq!(text.content, "Hello there");
        assert_eq!(text.font, TitleFont::Sans);
        let bare = spec.with_style("box", &Raw::Text("none".into())).unwrap();
        assert_eq!(bare.place(Ratio::new(3, 2).unwrap(), (1920, 1080)).len(), 1);
    }
}
