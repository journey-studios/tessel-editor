//! Reusable groups, text styles and keyframes (F069, BL169.6 step 2).
//!
//! A **group definition** is a small composition of editable children (rectangles, ellipses,
//! paths and text) drawn relative to one origin. It is stored once in the project; any number of
//! **group clips** on the timeline point at it, so editing a child, a style or a key changes every
//! instance. A clip only carries what is local to it: which definition it shows and, optionally,
//! another text for a text child (`GroupInstance::texts`). The group clip's own position, zoom,
//! rotation, opacity, fades, blend and colour correction (the ordinary inspector properties) move
//! the whole composition.
//!
//! The group is **not** a node the compositor knows: the plan (`plan::group_layers`) expands each
//! instance into ordinary shape and text layers at the frame being drawn, so the monitor, the PNG
//! of `render.frame` and the export draw the same pictures with the runtime they already have.
//!
//! **Time.** Every key time is in milliseconds counted from the clip's first frame, so moving or
//! duplicating the clip moves the whole animation with it. A child's own time is the clip time
//! minus `index * stagger_ms` (the *stagger*: each child enters a little after the previous one),
//! so one definition gives a staggered entrance without per-child bookkeeping.
//!
//! **Keyframes.** A child owns at most one [`KeyTrack`] per animatable property ([`Prop`]):
//! position (`x`, `y`), `scale`, `rotation`, `opacity` and, for a shape child, `width`, `height`,
//! `radius` and `stroke_width`. Between two keys the value interpolates linearly with the
//! *earlier* key's easing (the directed-scene three plus `smoothStep`, `easing::KeyEasing`); before the first key
//! it holds the first value and after the last it holds the last. While a track has keys, they
//! win over the child's stored base value of that property.
//!
//! **Repetition.** A track plays `repeat` times (`0` is forever): the cycle is the span between
//! its first and last key. `restart` jumps back to the first key at each cycle; `pingpong` plays
//! alternate cycles backwards, so a pulse never snaps.
//!
//! Everything here is pure: a function of the definition, the instance and a time, with no clock,
//! window or file.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::content::{TextSpec, TitleAlignment, TitleFont};
// Aliased so the key code reads as it did when the easings were the camera's three: the group's
// set is those plus `smoothStep` (F072), and camera keys keep the origin's three.
use crate::easing::KeyEasing as Easing;
use crate::id::Id;
use crate::shape::ShapeSpec;
use crate::state::error::{CommandError, Raw};

/// Most children one definition holds.
pub const MAX_CHILDREN: usize = 32;
/// Most keys one track holds.
pub const MAX_KEYS: usize = 64;
/// Latest key time and longest stagger step, in milliseconds.
pub const MAX_TIME_MS: i64 = 600_000;
pub const MAX_STAGGER_MS: i64 = 10_000;
/// Most plays of a track; `0` repeats forever.
pub const MAX_REPEAT: u32 = 1000;
/// Longest name of a group.
pub const MAX_NAME_CHARS: usize = 80;

fn invalid(message: impl Into<String>, expected: Value) -> CommandError {
    CommandError::invalid_value(message, expected)
}

fn unknown_field(what: &str, field: &str, available: &[&str]) -> CommandError {
    CommandError::new("unknown_field", format!("{what} has no field `{field}`"))
        .details(json!({ "available": available }))
}

/// A number the way a stored value spells it: never `-0`.
fn clean(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}

// ---- properties ------------------------------------------------------------------------

/// What a keyframe track animates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Prop {
    /// Offset from the group's origin, percent of the canvas width.
    X,
    /// Offset from the group's origin, percent of the canvas height (positive is down).
    Y,
    /// Percent of the child's own size; `100` is unscaled.
    Scale,
    /// Clockwise degrees around the child's centre.
    Rotation,
    /// Percent; multiplies the group clip's own opacity.
    Opacity,
    /// Shape children only: the shape's own fields (percent of the canvas).
    Width,
    Height,
    Radius,
    StrokeWidth,
}

impl Prop {
    pub const ALL: [Self; 9] = [
        Self::X,
        Self::Y,
        Self::Scale,
        Self::Rotation,
        Self::Opacity,
        Self::Width,
        Self::Height,
        Self::Radius,
        Self::StrokeWidth,
    ];
    pub const OPTIONS: [&str; 9] = [
        "x",
        "y",
        "scale",
        "rotation",
        "opacity",
        "width",
        "height",
        "radius",
        "stroke_width",
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::X => "x",
            Self::Y => "y",
            Self::Scale => "scale",
            Self::Rotation => "rotation",
            Self::Opacity => "opacity",
            Self::Width => "width",
            Self::Height => "height",
            Self::Radius => "radius",
            Self::StrokeWidth => "stroke_width",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, CommandError> {
        Self::ALL
            .into_iter()
            .find(|prop| prop.label() == raw.trim())
            .ok_or_else(|| {
                invalid(
                    format!("`{}` is not an animatable property", raw.trim()),
                    json!({ "field": "prop", "options": Self::OPTIONS }),
                )
            })
    }

    /// Whether only a shape child has this property (they are fields of its `ShapeSpec`).
    pub fn shape_only(self) -> bool {
        matches!(
            self,
            Self::Width | Self::Height | Self::Radius | Self::StrokeWidth
        )
    }

    /// `(min, max, min_is_exclusive, unit)`; a value outside is refused when it is stored.
    pub fn limits(self) -> (f64, f64, bool, &'static str) {
        match self {
            Self::X => (-200.0, 200.0, false, "% of the canvas width"),
            Self::Y => (-200.0, 200.0, false, "% of the canvas height"),
            Self::Scale => (1.0, 1000.0, false, "%"),
            Self::Rotation => (-3600.0, 3600.0, false, "degrees"),
            Self::Opacity => (0.0, 100.0, false, "%"),
            Self::Width | Self::Height => (0.0, 200.0, true, "% of the canvas"),
            Self::Radius => (0.0, 100.0, false, "% of the canvas height"),
            Self::StrokeWidth => (0.0, 20.0, false, "% of the canvas height"),
        }
    }

    pub fn check(self, value: f64) -> Result<(), CommandError> {
        let (min, max, exclusive, unit) = self.limits();
        let above = if exclusive { value > min } else { value >= min };
        if value.is_finite() && above && value <= max {
            return Ok(());
        }
        let bound = if exclusive { "above" } else { "from" };
        Err(invalid(
            format!(
                "`{}` is a number {bound} {min} up to {max} ({unit})",
                self.label()
            ),
            json!({ "field": self.label(), "min": min, "max": max,
                "min_exclusive": exclusive, "unit": unit }),
        ))
    }

    fn clamp(self, value: f64) -> f64 {
        let (min, max, _, _) = self.limits();
        value.clamp(min, max)
    }

    fn default_base(self) -> f64 {
        match self {
            Self::Scale | Self::Opacity => 100.0,
            _ => 0.0,
        }
    }
}

// ---- keys and tracks ---------------------------------------------------------------------

/// How a repeating track plays its later cycles.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum RepeatMode {
    /// Every cycle starts again from the first key.
    #[default]
    Restart,
    /// Odd cycles play backwards, so the end of one cycle is the start of the next.
    PingPong,
}

impl RepeatMode {
    pub const OPTIONS: [&str; 2] = ["restart", "pingpong"];

    pub fn label(self) -> &'static str {
        match self {
            Self::Restart => "restart",
            Self::PingPong => "pingpong",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, CommandError> {
        match raw.trim() {
            "restart" => Ok(Self::Restart),
            "pingpong" => Ok(Self::PingPong),
            other => Err(invalid(
                format!("`{other}` is not a repeat mode"),
                json!({ "field": "mode", "options": Self::OPTIONS }),
            )),
        }
    }
}

/// The value a property has at `time_ms`, and how it leaves that key.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Key {
    pub time_ms: i64,
    pub value: f64,
    /// Easing of the segment from this key to the next; unused on the last key.
    pub easing: Easing,
}

impl Key {
    pub const FIELDS: [&'static str; 3] = ["time_ms", "value", "easing"];

    pub fn to_json(self) -> Value {
        json!({ "time_ms": self.time_ms, "value": clean(self.value), "easing": self.easing.label() })
    }

    pub fn from_json(value: &Value) -> Result<Self, CommandError> {
        let object = value.as_object().ok_or_else(|| {
            invalid(
                "A key is an object",
                json!({ "format": "{time_ms, value}" }),
            )
        })?;
        if let Some(field) = object
            .keys()
            .find(|key| !Self::FIELDS.contains(&key.as_str()))
        {
            return Err(unknown_field("A key", field, &Self::FIELDS));
        }
        let time_ms = object
            .get("time_ms")
            .and_then(Raw::from_json)
            .ok_or_else(time_error)?
            .integer()
            .map_err(|_| time_error())?;
        let value = object
            .get("value")
            .and_then(Raw::from_json)
            .ok_or_else(|| invalid("A key needs a numeric `value`", json!({ "field": "value" })))?
            .number()?;
        Ok(Self {
            time_ms,
            value,
            easing: parse_easing(object.get("easing"))?,
        })
    }
}

fn time_error() -> CommandError {
    invalid(
        format!("A key `time_ms` is a whole number of milliseconds from 0 to {MAX_TIME_MS}"),
        json!({ "field": "time_ms", "min": 0, "max": MAX_TIME_MS, "unit": "ms" }),
    )
}

fn easing_error(raw: &str) -> CommandError {
    invalid(
        format!("`{raw}` is not an easing"),
        json!({ "field": "easing", "options": Easing::labels() }),
    )
}

fn parse_easing(raw: Option<&Value>) -> Result<Easing, CommandError> {
    match raw {
        None | Some(Value::Null) => Ok(Easing::Linear),
        Some(Value::String(label)) => Easing::parse(label).ok_or_else(|| easing_error(label)),
        Some(other) => Err(easing_error(&other.to_string())),
    }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// The keys of one property of one child.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyTrack {
    pub prop: Prop,
    /// Plays of the whole track: `1` once (the default), `0` forever.
    pub repeat: u32,
    pub mode: RepeatMode,
    /// Strictly increasing in time; never empty.
    pub keys: Vec<Key>,
}

impl KeyTrack {
    pub const FIELDS: [&'static str; 4] = ["prop", "repeat", "mode", "keys"];

    pub fn new(prop: Prop) -> Self {
        Self {
            prop,
            repeat: 1,
            mode: RepeatMode::Restart,
            keys: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), CommandError> {
        if self.keys.is_empty() || self.keys.len() > MAX_KEYS {
            return Err(invalid(
                format!(
                    "A track of `{}` has 1 to {MAX_KEYS} keys",
                    self.prop.label()
                ),
                json!({ "field": "keys", "min": 1, "max": MAX_KEYS }),
            ));
        }
        if self.repeat > MAX_REPEAT {
            return Err(invalid(
                format!("`repeat` is 0 (forever) or 1 to {MAX_REPEAT}"),
                json!({ "field": "repeat", "min": 0, "max": MAX_REPEAT }),
            ));
        }
        for key in &self.keys {
            if !(0..=MAX_TIME_MS).contains(&key.time_ms) {
                return Err(time_error());
            }
            self.prop.check(key.value)?;
        }
        if let Some(pair) = self
            .keys
            .windows(2)
            .find(|pair| pair[0].time_ms >= pair[1].time_ms)
        {
            return Err(invalid(
                format!(
                    "Key times of `{}` must be strictly increasing ({} ms then {} ms)",
                    self.prop.label(),
                    pair[0].time_ms,
                    pair[1].time_ms
                ),
                json!({ "field": "time_ms" }),
            ));
        }
        Ok(())
    }

    /// The first key's time, where a cycle starts.
    fn start(&self) -> f64 {
        self.keys.first().map_or(0.0, |key| key.time_ms as f64)
    }

    /// The span of one cycle.
    fn period(&self) -> f64 {
        match (self.keys.first(), self.keys.last()) {
            (Some(first), Some(last)) => (last.time_ms - first.time_ms) as f64,
            _ => 0.0,
        }
    }

    /// Milliseconds on the child's clock at which the track has played all its repetitions (one
    /// cycle for a track that repeats forever): what a child that starts `after` it waits for.
    pub fn span_ms(&self) -> i64 {
        let (Some(first), Some(last)) = (self.keys.first(), self.keys.last()) else {
            return 0;
        };
        let plays = i64::from(self.repeat.max(1));
        first
            .time_ms
            .saturating_add((last.time_ms - first.time_ms).saturating_mul(plays))
    }

    /// Value between the keys at a time inside the first-to-last span (no repetition).
    fn sample(&self, time: f64) -> f64 {
        let after = self.keys.partition_point(|key| key.time_ms as f64 <= time);
        let (Some(from), Some(to)) = (
            after.checked_sub(1).and_then(|index| self.keys.get(index)),
            self.keys.get(after),
        ) else {
            return self.keys.last().map_or(0.0, |key| key.value);
        };
        let span = (to.time_ms - from.time_ms) as f64;
        let progress = from.easing.apply((time - from.time_ms as f64) / span);
        lerp(from.value, to.value, progress)
    }

    /// The value at `time_ms` of the child's own clock (negative before it entered).
    pub fn evaluate(&self, time_ms: f64) -> f64 {
        let (Some(first), Some(last)) = (self.keys.first(), self.keys.last()) else {
            return 0.0;
        };
        let (start, period) = (self.start(), self.period());
        if !time_ms.is_finite() || time_ms <= start || period <= 0.0 {
            return first.value;
        }
        let elapsed = time_ms - start;
        let cycle = (elapsed / period).floor();
        let finished = self.repeat != 0 && cycle >= f64::from(self.repeat);
        if finished {
            // The state a track rests in once its plays are over: pingpong ends on the first key
            // after an even number of plays (every other cycle ran backwards).
            let backwards_end = self.mode == RepeatMode::PingPong && self.repeat.is_multiple_of(2);
            return if backwards_end {
                first.value
            } else {
                last.value
            };
        }
        let phase = elapsed - cycle * period;
        let backwards = self.mode == RepeatMode::PingPong && (cycle as u64) % 2 == 1;
        let phase = if backwards { period - phase } else { phase };
        self.sample(start + phase)
    }

    pub fn to_json(&self) -> Value {
        let mut value = json!({
            "prop": self.prop.label(),
            "keys": self.keys.iter().map(|key| key.to_json()).collect::<Vec<_>>(),
        });
        if self.repeat != 1 {
            value["repeat"] = json!(self.repeat);
        }
        if self.mode != RepeatMode::Restart {
            value["mode"] = json!(self.mode.label());
        }
        value
    }

    pub fn from_json(value: &Value) -> Result<Self, CommandError> {
        let object = value
            .as_object()
            .ok_or_else(|| invalid("A track is an object", json!({ "format": "{prop, keys}" })))?;
        if let Some(field) = object
            .keys()
            .find(|key| !Self::FIELDS.contains(&key.as_str()))
        {
            return Err(unknown_field("A track", field, &Self::FIELDS));
        }
        let prop = Prop::parse(object.get("prop").and_then(Value::as_str).ok_or_else(|| {
            invalid("A track needs `prop`", json!({ "options": Prop::OPTIONS }))
        })?)?;
        let mut track = Self::new(prop);
        if let Some(repeat) = object.get("repeat").filter(|raw| !raw.is_null()) {
            track.repeat = Raw::from_json(repeat)
                .and_then(|raw| raw.integer().ok())
                .and_then(|number| u32::try_from(number).ok())
                .ok_or_else(|| {
                    invalid(
                        format!("`repeat` is 0 (forever) or 1 to {MAX_REPEAT}"),
                        json!({ "field": "repeat", "min": 0, "max": MAX_REPEAT }),
                    )
                })?;
        }
        if let Some(mode) = object.get("mode").filter(|raw| !raw.is_null()) {
            track.mode = RepeatMode::parse(mode.as_str().unwrap_or_default())?;
        }
        track.keys = object
            .get("keys")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("A track needs a `keys` array", json!({ "field": "keys" })))?
            .iter()
            .map(Key::from_json)
            .collect::<Result<_, _>>()?;
        track.validate()?;
        Ok(track)
    }
}

// ---- text styles -------------------------------------------------------------------------

/// A named text look shared by the text children that use it.
#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    pub id: Id,
    /// Glyph cell height, percent of the canvas height.
    pub size: f64,
    /// `#rrggbb`.
    pub color: String,
    pub font: TitleFont,
    pub align: TitleAlignment,
    /// Width of the box the text wraps in, percent of the canvas width; `0` is the automatic
    /// title-safe width. A chat bubble is a text with a box narrower than the screen.
    pub width: f64,
}

impl TextStyle {
    pub const FIELDS: [&'static str; 5] = ["size", "color", "font", "align", "width"];

    pub fn new(id: Id) -> Self {
        Self {
            id,
            size: 6.0,
            color: "#ffffff".to_string(),
            font: TitleFont::Sans,
            align: TitleAlignment::Center,
            width: 0.0,
        }
    }

    pub fn validate(&self) -> Result<(), CommandError> {
        self.spec("x").map(|_| ())
    }

    /// The text a child shows in this style. The entrance preset of a title is not part of a
    /// style: a child moves with keys.
    pub fn spec(&self, content: &str) -> Result<TextSpec, CommandError> {
        let mut text = TextSpec::new(content, self.size, self.color.clone())?;
        text.font = self.font;
        text.align = self.align;
        text.width = self.width;
        text.validate()?;
        Ok(text)
    }

    pub fn with_field(&self, field: &str, value: &Raw) -> Result<Self, CommandError> {
        let mut next = self.clone();
        match field {
            "size" => next.size = value.number()?,
            "color" => next.color = value.text().trim().to_ascii_lowercase(),
            "font" => next.font = TitleFont::parse(value.text().trim())?,
            "align" => next.align = TitleAlignment::parse(value.text().trim())?,
            "width" => next.width = value.number()?,
            _ => return Err(unknown_field("A text style", field, &Self::FIELDS)),
        }
        next.validate()?;
        Ok(next)
    }

    pub fn field_json(&self, field: &str) -> Value {
        match field {
            "size" => json!(self.size),
            "color" => json!(self.color),
            "font" => json!(self.font.as_str()),
            "align" => json!(self.align.as_str()),
            "width" => json!(self.width),
            _ => Value::Null,
        }
    }

    pub fn to_json(&self) -> Value {
        let mut value = json!({ "id": self.id });
        for field in Self::FIELDS {
            // A style written before F070 has no box: leave `width` out while it is automatic so
            // such a project is byte-identical to what the earlier editor wrote.
            if field == "width" && self.width == 0.0 {
                continue;
            }
            value[field] = self.field_json(field);
        }
        value
    }

    pub fn from_json(value: &Value) -> Result<Self, CommandError> {
        let object = value
            .as_object()
            .ok_or_else(|| invalid("A text style is an object", json!({})))?;
        let mut style = Self::new(id_of(object, "id", "A text style")?);
        for (field, raw) in object {
            if field == "id" {
                continue;
            }
            let raw = Raw::from_json(raw).ok_or_else(|| {
                invalid(
                    format!("Text style {field} is a string or a number"),
                    json!({ "field": field }),
                )
            })?;
            style = style.with_field(field, &raw)?;
        }
        style.validate()?;
        Ok(style)
    }
}

fn id_of(object: &Map<String, Value>, key: &str, what: &str) -> Result<Id, CommandError> {
    let raw = object.get(key).and_then(Value::as_str).ok_or_else(|| {
        CommandError::new("invalid_request", format!("{what} needs a string `{key}`"))
    })?;
    Id::new(raw).map_err(|error| CommandError::new("invalid_id", error.to_string()).target(raw))
}

// ---- reveals -----------------------------------------------------------------------------

/// Shortest and longest step of a reveal, in milliseconds.
pub const MIN_STEP_MS: i64 = 1;
pub const MAX_STEP_MS: i64 = 5000;

/// How a text child shows itself over time (F070).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevealMode {
    /// One character per step, in reading order: typing.
    Type,
    /// One line per step, whole: the line is shown the way the text ends up laid out.
    Lines,
}

impl RevealMode {
    /// `none` is the absence of a reveal (the text is there from the first frame).
    pub const OPTIONS: [&str; 3] = ["none", "type", "lines"];

    pub fn label(self) -> &'static str {
        match self {
            Self::Type => "type",
            Self::Lines => "lines",
        }
    }

    /// A step a reader follows comfortably: about 25 characters a second, or a line every
    /// 450 ms.
    pub fn default_step_ms(self) -> i64 {
        match self {
            Self::Type => 40,
            Self::Lines => 450,
        }
    }

    /// `None` for `none`.
    pub fn parse(raw: &str) -> Result<Option<Self>, CommandError> {
        match raw.trim() {
            "none" | "" => Ok(None),
            "type" => Ok(Some(Self::Type)),
            "lines" => Ok(Some(Self::Lines)),
            other => Err(invalid(
                format!("`{other}` is not a reveal"),
                json!({ "field": "reveal", "options": Self::OPTIONS }),
            )),
        }
    }
}

fn step_error() -> CommandError {
    invalid(
        format!("`step_ms` is a whole number of milliseconds from {MIN_STEP_MS} to {MAX_STEP_MS}"),
        json!({ "field": "step_ms", "min": MIN_STEP_MS, "max": MAX_STEP_MS, "unit": "ms" }),
    )
}

/// The reveal of one text child: what it counts and how long a count takes.
///
/// **Stable breaks.** The lines a text ends up in are decided from the *whole* text (the box,
/// `text::lines_of`) before any character is shown, so typing never re-wraps a word that is
/// still being written and a line revealed whole is the line the finished text has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reveal {
    pub mode: RevealMode,
    /// Milliseconds per character (`type`) or per line (`lines`).
    pub step_ms: i64,
}

impl Reveal {
    pub fn new(mode: RevealMode) -> Self {
        Self {
            mode,
            step_ms: mode.default_step_ms(),
        }
    }

    pub fn validate(&self) -> Result<(), CommandError> {
        if (MIN_STEP_MS..=MAX_STEP_MS).contains(&self.step_ms) {
            return Ok(());
        }
        Err(step_error())
    }

    /// Steps the whole text takes: its characters, or its lines.
    fn units(&self, content: &str, lines: &[String]) -> i64 {
        match self.mode {
            RevealMode::Type => content.chars().count() as i64,
            RevealMode::Lines => lines.len() as i64,
        }
    }

    /// Milliseconds from the first character to the end of the last step.
    pub fn span_ms(&self, content: &str, lines: &[String]) -> i64 {
        self.units(content, lines).saturating_mul(self.step_ms)
    }

    /// How many leading characters of `content` show `time_ms` after the reveal starts: `None`
    /// once all of them do, `Some(0)` before the first. The first character (or line) appears
    /// at `0`, the next one `step_ms` later, and so on.
    pub fn visible(&self, content: &str, lines: &[String], time_ms: f64) -> Option<usize> {
        if !time_ms.is_finite() || time_ms < 0.0 {
            return Some(0);
        }
        // The nudge keeps a frame that lands exactly on a step from reading as the step before
        // (the clock gives milliseconds as a float).
        let steps = (time_ms / self.step_ms as f64 + 1e-9).floor() as usize + 1;
        let total = content.chars().count();
        let shown = match self.mode {
            RevealMode::Type => steps.min(total),
            RevealMode::Lines => {
                let count = steps.min(lines.len());
                let starts = crate::text::line_starts(content, lines);
                match (count.checked_sub(1), starts.get(count.saturating_sub(1))) {
                    (Some(last), Some(start)) => start + lines[last].chars().count(),
                    _ => 0,
                }
            }
        };
        (shown < total).then_some(shown)
    }

    pub fn to_json(&self) -> Value {
        json!({ "mode": self.mode.label(), "step_ms": self.step_ms })
    }

    pub fn from_json(value: &Value) -> Result<Self, CommandError> {
        let object = value.as_object().ok_or_else(|| {
            invalid(
                "A reveal is `{mode, step_ms?}`",
                json!({ "format": "{mode, step_ms?}" }),
            )
        })?;
        if let Some(field) = object
            .keys()
            .find(|key| !["mode", "step_ms"].contains(&key.as_str()))
        {
            return Err(unknown_field("A reveal", field, &["mode", "step_ms"]));
        }
        let mode = RevealMode::parse(object.get("mode").and_then(Value::as_str).unwrap_or(""))?
            .ok_or_else(|| {
                invalid(
                    "A reveal `mode` is `type` or `lines`",
                    json!({ "field": "mode", "options": ["type", "lines"] }),
                )
            })?;
        let mut reveal = Self::new(mode);
        if let Some(step) = object.get("step_ms").filter(|raw| !raw.is_null()) {
            reveal.step_ms = Raw::from_json(step)
                .ok_or_else(step_error)?
                .integer()
                .map_err(|_| step_error())?;
        }
        reveal.validate()?;
        Ok(reveal)
    }
}

// ---- children ----------------------------------------------------------------------------

/// What a child draws.
#[derive(Debug, Clone, PartialEq)]
pub enum ChildContent {
    Shape(ShapeSpec),
    Text {
        /// Replaceable per instance (`GroupInstance::texts`).
        content: String,
        style: Id,
    },
}

/// One editable part of a group.
#[derive(Debug, Clone, PartialEq)]
pub struct Child {
    pub id: Id,
    pub content: ChildContent,
    /// Base values (a track of the same property overrides them while it has keys).
    pub x: f64,
    pub y: f64,
    pub scale: f64,
    pub rotation: f64,
    pub opacity: f64,
    /// One per animated property, in the order they were first keyed.
    pub tracks: Vec<KeyTrack>,
    /// Text children only: typing or line-by-line reveal (F070).
    pub reveal: Option<Reveal>,
    /// The child that must finish before this one starts (it comes earlier in the group); `None`
    /// starts at the group's stagger. With it the sequence follows the content: a longer question
    /// types longer and the answer waits for it.
    pub after: Option<Id>,
    /// Milliseconds after the stagger, or after `after` ends.
    pub delay_ms: i64,
}

const CHILD_JSON_FIELDS: [&str; 14] = [
    "id", "kind", "shape", "content", "style", "x", "y", "scale", "rotation", "opacity", "tracks",
    "reveal", "after", "delay_ms",
];

impl Child {
    /// The settable fields every child has.
    pub const BASE_FIELDS: [&'static str; 5] = ["x", "y", "scale", "rotation", "opacity"];
    /// The scheduling fields every child has (F070), after [`Self::BASE_FIELDS`].
    pub const SCHEDULE_FIELDS: [&'static str; 2] = ["after", "delay_ms"];
    /// The reveal fields of a text child (F070); `reveal` is `none`, `type` or `lines`.
    pub const REVEAL_FIELDS: [&'static str; 2] = ["reveal", "step_ms"];

    pub fn shape(id: Id, shape: ShapeSpec) -> Self {
        Self::of(id, ChildContent::Shape(shape))
    }

    pub fn text(id: Id, content: impl Into<String>, style: Id) -> Self {
        Self::of(
            id,
            ChildContent::Text {
                content: content.into(),
                style,
            },
        )
    }

    fn of(id: Id, content: ChildContent) -> Self {
        Self {
            id,
            content,
            x: 0.0,
            y: 0.0,
            scale: 100.0,
            rotation: 0.0,
            opacity: 100.0,
            tracks: Vec::new(),
            reveal: None,
            after: None,
            delay_ms: 0,
        }
    }

    pub fn is_text(&self) -> bool {
        matches!(self.content, ChildContent::Text { .. })
    }

    pub fn kind_label(&self) -> &'static str {
        if self.is_text() { "text" } else { "shape" }
    }

    /// Every field `with_field` accepts for this child, base fields first.
    pub fn fields(&self) -> Vec<&'static str> {
        let mut fields = Self::BASE_FIELDS.to_vec();
        fields.extend(Self::SCHEDULE_FIELDS);
        match &self.content {
            ChildContent::Shape(_) => fields.extend(ShapeSpec::FIELDS),
            ChildContent::Text { .. } => {
                fields.extend(["content", "style"]);
                fields.extend(Self::REVEAL_FIELDS);
            }
        }
        fields
    }

    pub fn track(&self, prop: Prop) -> Option<&KeyTrack> {
        self.tracks.iter().find(|track| track.prop == prop)
    }

    /// The stored (un-keyed) value of an animatable property.
    pub fn base(&self, prop: Prop) -> f64 {
        match (prop, &self.content) {
            (Prop::X, _) => self.x,
            (Prop::Y, _) => self.y,
            (Prop::Scale, _) => self.scale,
            (Prop::Rotation, _) => self.rotation,
            (Prop::Opacity, _) => self.opacity,
            (Prop::Width, ChildContent::Shape(shape)) => shape.width,
            (Prop::Height, ChildContent::Shape(shape)) => shape.height,
            (Prop::Radius, ChildContent::Shape(shape)) => shape.radius,
            (Prop::StrokeWidth, ChildContent::Shape(shape)) => shape.stroke_width,
            (other, ChildContent::Text { .. }) => other.default_base(),
        }
    }

    fn base_mut(&mut self, field: &str) -> Option<&mut f64> {
        match field {
            "x" => Some(&mut self.x),
            "y" => Some(&mut self.y),
            "scale" => Some(&mut self.scale),
            "rotation" => Some(&mut self.rotation),
            "opacity" => Some(&mut self.opacity),
            _ => None,
        }
    }

    pub fn validate(&self, styles: &[TextStyle]) -> Result<(), CommandError> {
        for prop in [Prop::X, Prop::Y, Prop::Scale, Prop::Rotation, Prop::Opacity] {
            prop.check(self.base(prop))?;
        }
        match &self.content {
            ChildContent::Shape(shape) => shape.validate()?,
            ChildContent::Text { content, style } => {
                let style = styles
                    .iter()
                    .find(|candidate| candidate.id == *style)
                    .ok_or_else(|| {
                        CommandError::unknown_element("text style", style.as_str())
                            .target(format!("text_style:{style}"))
                    })?;
                let spec = style.spec(content)?;
                if spec.content != *content {
                    return Err(CommandError::new(
                        "unsupported_text",
                        "Text must be in its composed (NFC) spelling",
                    ));
                }
            }
        }
        if !(0..=MAX_TIME_MS).contains(&self.delay_ms) {
            return Err(invalid(
                format!("`delay_ms` is 0 to {MAX_TIME_MS} milliseconds"),
                json!({ "field": "delay_ms", "min": 0, "max": MAX_TIME_MS, "unit": "ms" }),
            ));
        }
        if let Some(reveal) = &self.reveal {
            if !self.is_text() {
                return Err(CommandError::new(
                    "not_applicable",
                    format!(
                        "A reveal types or shows lines of text; child `{}` is a shape",
                        self.id
                    ),
                ));
            }
            reveal.validate()?;
        }
        if self.after.is_some_and(|after| after == self.id) {
            return Err(invalid(
                format!("Child `{}` cannot start after itself", self.id),
                json!({ "field": "after" }),
            ));
        }
        for (index, track) in self.tracks.iter().enumerate() {
            track.validate()?;
            if track.prop.shape_only() && self.is_text() {
                return Err(not_applicable(track.prop, &self.id));
            }
            if self.tracks[..index]
                .iter()
                .any(|earlier| earlier.prop == track.prop)
            {
                return Err(CommandError::new(
                    "duplicate_id",
                    format!(
                        "Child `{}` has two tracks for `{}`",
                        self.id,
                        track.prop.label()
                    ),
                ));
            }
        }
        Ok(())
    }

    /// The child with one field replaced, validated as a whole.
    pub fn with_field(
        &self,
        field: &str,
        value: &Raw,
        styles: &[TextStyle],
    ) -> Result<Self, CommandError> {
        let mut next = self.clone();
        if let Some(slot) = next.base_mut(field) {
            *slot = value.number()?;
        } else if field == "delay_ms" {
            next.delay_ms = value.integer()?;
        } else if field == "after" {
            let raw = value.text();
            next.after = match raw.trim() {
                "" | "none" => None,
                other => Some(
                    Id::new(other)
                        .map_err(|error| CommandError::new("invalid_id", error.to_string()))?,
                ),
            };
        } else if next.is_text() && field == "reveal" {
            next.reveal = RevealMode::parse(&value.text())?.map(|mode| match self.reveal {
                // Switching the mode keeps a step the author chose only while it is the same
                // unit; a per-character step is no per-line step.
                Some(current) if current.mode == mode => current,
                _ => Reveal::new(mode),
            });
        } else if next.is_text() && field == "step_ms" {
            let Some(reveal) = next.reveal.as_mut() else {
                return Err(CommandError::new(
                    "not_applicable",
                    format!("Child `{}` has no reveal: set `reveal` first", self.id),
                ));
            };
            reveal.step_ms = value.integer().map_err(|_| step_error())?;
        } else {
            match (&mut next.content, field) {
                (ChildContent::Text { content, .. }, "content") => {
                    *content = crate::text::normalize(&value.text())?;
                }
                (ChildContent::Text { style, .. }, "style") => {
                    *style = Id::new(value.text().trim())
                        .map_err(|error| CommandError::new("invalid_id", error.to_string()))?;
                }
                (ChildContent::Shape(shape), _) if ShapeSpec::FIELDS.contains(&field) => {
                    *shape = shape.with_field(field, value)?;
                }
                _ => return Err(unknown_field("This child", field, &self.fields())),
            }
        }
        next.validate(styles)?;
        Ok(next)
    }

    pub fn field_json(&self, field: &str) -> Value {
        match (field, &self.content) {
            ("x", _) => json!(self.x),
            ("y", _) => json!(self.y),
            ("scale", _) => json!(self.scale),
            ("rotation", _) => json!(self.rotation),
            ("opacity", _) => json!(self.opacity),
            ("delay_ms", _) => json!(self.delay_ms),
            ("after", _) => json!(self.after.map_or(String::new(), |after| after.to_string())),
            ("reveal", ChildContent::Text { .. }) => {
                json!(self.reveal.map_or("none", |reveal| reveal.mode.label()))
            }
            ("step_ms", ChildContent::Text { .. }) => {
                json!(self.reveal.map_or(0, |reveal| reveal.step_ms))
            }
            ("content", ChildContent::Text { content, .. }) => json!(content),
            ("style", ChildContent::Text { style, .. }) => json!(style),
            (_, ChildContent::Shape(shape)) => {
                shape.to_json().get(field).cloned().unwrap_or(Value::Null)
            }
            _ => Value::Null,
        }
    }

    /// Creates the track of `prop` on first use.
    fn track_mut(&mut self, prop: Prop) -> &mut KeyTrack {
        let index = match self.tracks.iter().position(|track| track.prop == prop) {
            Some(index) => index,
            None => {
                self.tracks.push(KeyTrack::new(prop));
                self.tracks.len() - 1
            }
        };
        &mut self.tracks[index]
    }

    /// Adds a key, or replaces the key at the same time, on the track of `prop` (created when
    /// missing). The easing of a replaced key stays unless one is given.
    pub fn with_key(
        &self,
        prop: Prop,
        time_ms: i64,
        value: f64,
        easing: Option<Easing>,
        styles: &[TextStyle],
    ) -> Result<Self, CommandError> {
        if prop.shape_only() && self.is_text() {
            return Err(not_applicable(prop, &self.id));
        }
        let mut next = self.clone();
        let track = next.track_mut(prop);
        match track.keys.iter().position(|key| key.time_ms == time_ms) {
            Some(index) => {
                track.keys[index].value = value;
                if let Some(easing) = easing {
                    track.keys[index].easing = easing;
                }
            }
            None => {
                track.keys.push(Key {
                    time_ms,
                    value,
                    easing: easing.unwrap_or_default(),
                });
                track.keys.sort_by_key(|key| key.time_ms);
            }
        }
        next.validate(styles)?;
        Ok(next)
    }

    /// Removes the key at exactly `time_ms`; the track goes with its last key.
    pub fn without_key(&self, prop: Prop, time_ms: i64) -> Result<Self, CommandError> {
        let mut next = self.clone();
        let missing = || {
            CommandError::new(
                "unknown_element",
                format!(
                    "Child `{}` has no `{}` key at {time_ms} ms",
                    self.id,
                    prop.label()
                ),
            )
        };
        let track = next
            .tracks
            .iter_mut()
            .find(|track| track.prop == prop)
            .ok_or_else(missing)?;
        let before = track.keys.len();
        track.keys.retain(|key| key.time_ms != time_ms);
        if track.keys.len() == before {
            return Err(missing());
        }
        next.tracks.retain(|track| !track.keys.is_empty());
        Ok(next)
    }

    /// Changes how an existing track repeats; `None` keeps the value.
    pub fn with_repeat(
        &self,
        prop: Prop,
        repeat: Option<u32>,
        mode: Option<RepeatMode>,
    ) -> Result<Self, CommandError> {
        let mut next = self.clone();
        let track = next
            .tracks
            .iter_mut()
            .find(|track| track.prop == prop)
            .ok_or_else(|| {
                CommandError::new(
                    "unknown_element",
                    format!("Child `{}` has no track for `{}`", self.id, prop.label()),
                )
            })?;
        track.repeat = repeat.unwrap_or(track.repeat);
        track.mode = mode.unwrap_or(track.mode);
        track.validate()?;
        Ok(next)
    }

    pub fn without_track(&self, prop: Prop) -> Result<Self, CommandError> {
        let mut next = self.clone();
        let before = next.tracks.len();
        next.tracks.retain(|track| track.prop != prop);
        if next.tracks.len() == before {
            return Err(CommandError::new(
                "unknown_element",
                format!("Child `{}` has no track for `{}`", self.id, prop.label()),
            ));
        }
        Ok(next)
    }

    /// Edits one field of the `index`-th key of a track: `time_ms`, `value` or `easing`.
    pub fn with_key_field(
        &self,
        prop: Prop,
        index: usize,
        field: &str,
        value: &Raw,
        styles: &[TextStyle],
    ) -> Result<Self, CommandError> {
        let mut next = self.clone();
        let key = next
            .tracks
            .iter_mut()
            .find(|track| track.prop == prop)
            .and_then(|track| track.keys.get_mut(index))
            .ok_or_else(|| {
                CommandError::new(
                    "unknown_element",
                    format!(
                        "Child `{}` has no `{}` key number {index}",
                        self.id,
                        prop.label()
                    ),
                )
            })?;
        match field {
            "time_ms" => key.time_ms = value.integer().map_err(|_| time_error())?,
            "value" => key.value = value.number()?,
            "easing" => {
                let label = value.text();
                key.easing = Easing::parse(&label).ok_or_else(|| easing_error(&label))?;
            }
            other => return Err(unknown_field("A key", other, &Key::FIELDS)),
        }
        // A retimed key may land past its neighbour; the order is part of validity, so it is
        // refused instead of silently re-sorted under the caller's index.
        next.validate(styles)?;
        Ok(next)
    }

    pub fn to_json(&self) -> Value {
        let mut value = json!({ "id": self.id, "kind": self.kind_label() });
        match &self.content {
            ChildContent::Shape(shape) => value["shape"] = shape.to_document_json(),
            ChildContent::Text { content, style } => {
                value["content"] = json!(content);
                value["style"] = json!(style);
            }
        }
        for field in Self::BASE_FIELDS {
            let neutral = Prop::parse(field).map_or(0.0, Prop::default_base);
            if self.field_json(field) != json!(neutral) {
                value[field] = self.field_json(field);
            }
        }
        if !self.tracks.is_empty() {
            value["tracks"] = Value::Array(self.tracks.iter().map(KeyTrack::to_json).collect());
        }
        // F070 fields only when set, so a group that does not use them is byte-identical to what
        // the F069 editor wrote.
        if let Some(reveal) = &self.reveal {
            value["reveal"] = reveal.to_json();
        }
        if let Some(after) = self.after {
            value["after"] = json!(after);
        }
        if self.delay_ms != 0 {
            value["delay_ms"] = json!(self.delay_ms);
        }
        value
    }

    pub fn from_json(value: &Value, styles: &[TextStyle]) -> Result<Self, CommandError> {
        let object = value
            .as_object()
            .ok_or_else(|| invalid("A group child is an object", json!({})))?;
        if let Some(field) = object
            .keys()
            .find(|key| !CHILD_JSON_FIELDS.contains(&key.as_str()))
        {
            return Err(unknown_field("A group child", field, &CHILD_JSON_FIELDS));
        }
        let id = id_of(object, "id", "A group child")?;
        let mut child = match object.get("kind").and_then(Value::as_str) {
            Some("shape") => Self::shape(
                id,
                ShapeSpec::from_json(object.get("shape").ok_or_else(|| {
                    invalid("A shape child needs `shape`", json!({ "field": "shape" }))
                })?)?,
            ),
            Some("text") => {
                let content = object
                    .get("content")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        invalid(
                            "A text child needs `content`",
                            json!({ "field": "content" }),
                        )
                    })?;
                let style = id_of(object, "style", "A text child")?;
                Self::text(id, crate::text::normalize(content)?, style)
            }
            _ => {
                return Err(invalid(
                    "A group child `kind` is `shape` or `text`",
                    json!({ "field": "kind", "options": ["shape", "text"] }),
                ));
            }
        };
        let foreign: &[&str] = match &child.content {
            ChildContent::Shape(_) => &["content", "style", "reveal"],
            ChildContent::Text { .. } => &["shape"],
        };
        if let Some(field) = foreign.iter().find(|field| object.contains_key(**field)) {
            return Err(CommandError::new(
                "not_applicable",
                format!("A {} child has no `{field}`", child.kind_label()),
            ));
        }
        for field in Child::BASE_FIELDS {
            if let Some(raw) = object.get(field) {
                let raw = Raw::from_json(raw).ok_or_else(|| {
                    invalid(format!("`{field}` is a number"), json!({ "field": field }))
                })?;
                if let Some(slot) = child.base_mut(field) {
                    *slot = raw.number()?;
                }
            }
        }
        if let Some(reveal) = object.get("reveal").filter(|raw| !raw.is_null()) {
            child.reveal = Some(Reveal::from_json(reveal)?);
        }
        if object.get("after").is_some_and(|raw| !raw.is_null()) {
            child.after = Some(id_of(object, "after", "A group child")?);
        }
        if let Some(delay) = object.get("delay_ms").filter(|raw| !raw.is_null()) {
            child.delay_ms = Raw::from_json(delay)
                .ok_or_else(|| invalid("`delay_ms` is a number", json!({ "field": "delay_ms" })))?
                .integer()?;
        }
        if let Some(tracks) = object.get("tracks").filter(|raw| !raw.is_null()) {
            child.tracks = tracks
                .as_array()
                .ok_or_else(|| invalid("`tracks` is an array", json!({ "field": "tracks" })))?
                .iter()
                .map(KeyTrack::from_json)
                .collect::<Result<_, _>>()?;
        }
        child.validate(styles)?;
        Ok(child)
    }
}

fn not_applicable(prop: Prop, child: &Id) -> CommandError {
    CommandError::new(
        "not_applicable",
        format!(
            "`{}` animates a shape; child `{child}` is text",
            prop.label()
        ),
    )
}

// ---- definitions ---------------------------------------------------------------------------

/// A reusable composition. Children are listed bottom to top.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupDef {
    pub id: Id,
    pub name: String,
    /// Milliseconds each child starts after the one before it (its keys shift by `index * step`).
    /// A child with `after` starts at the end of that earlier child instead, plus its own `delay_ms`.
    pub stagger_ms: i64,
    pub children: Vec<Child>,
}

impl GroupDef {
    pub const FIELDS: [&'static str; 2] = ["name", "stagger_ms"];

    pub fn new(id: Id) -> Self {
        Self {
            id,
            name: id.to_string(),
            stagger_ms: 0,
            children: Vec::new(),
        }
    }

    pub fn child(&self, id: &str) -> Option<&Child> {
        self.children.iter().find(|child| child.id == id)
    }

    /// The value `prop` of `child` at `time_ms` on the child's own clock (the instance's time
    /// minus the child's start, see [`GroupBook::timings`]): the keys applied, kept inside the
    /// property's range. The one place a value is computed, so the plan and a key captured at
    /// the playhead agree.
    fn child_value(child: &Child, prop: Prop, time_ms: f64) -> f64 {
        let raw = child
            .track(prop)
            .map_or_else(|| child.base(prop), |track| track.evaluate(time_ms));
        prop.clamp(raw)
    }

    /// The children another child starts after, by ID, for the refusal that names them.
    fn dependants(&self, id: &str) -> Vec<Id> {
        self.children
            .iter()
            .filter(|child| child.after.is_some_and(|after| after == id))
            .map(|child| child.id)
            .collect()
    }

    pub fn validate(&self, styles: &[TextStyle]) -> Result<(), CommandError> {
        if self.name.trim().is_empty() || self.name.chars().count() > MAX_NAME_CHARS {
            return Err(invalid(
                format!("A group name has 1 to {MAX_NAME_CHARS} characters"),
                json!({ "field": "name", "max_chars": MAX_NAME_CHARS }),
            ));
        }
        if !(0..=MAX_STAGGER_MS).contains(&self.stagger_ms) {
            return Err(invalid(
                format!("`stagger_ms` is 0 to {MAX_STAGGER_MS} milliseconds"),
                json!({ "field": "stagger_ms", "min": 0, "max": MAX_STAGGER_MS, "unit": "ms" }),
            ));
        }
        if self.children.len() > MAX_CHILDREN {
            return Err(invalid(
                format!("A group has at most {MAX_CHILDREN} children"),
                json!({ "max": MAX_CHILDREN }),
            ));
        }
        for (index, child) in self.children.iter().enumerate() {
            child.validate(styles)?;
            if self.children[..index]
                .iter()
                .any(|earlier| earlier.id == child.id)
            {
                return Err(CommandError::new(
                    "duplicate_id",
                    format!("Group `{}` has two children `{}`", self.id, child.id),
                ));
            }
            // `after` points back, never forward: the start of a child is computed from children
            // already placed, so a chain can neither loop nor wait for something later.
            if let Some(after) = child.after
                && !self.children[..index]
                    .iter()
                    .any(|earlier| earlier.id == after)
            {
                return Err(invalid(
                    format!(
                        "Child `{}` starts after `{after}`, which is not an earlier child of group `{}`",
                        child.id, self.id
                    ),
                    json!({ "field": "after", "options": self.children[..index]
                        .iter().map(|earlier| earlier.id.to_string()).collect::<Vec<_>>() }),
                ));
            }
        }
        Ok(())
    }

    pub fn with_field(
        &self,
        field: &str,
        value: &Raw,
        styles: &[TextStyle],
    ) -> Result<Self, CommandError> {
        let mut next = self.clone();
        match field {
            "name" => next.name = value.text().trim().to_string(),
            "stagger_ms" => next.stagger_ms = value.integer()?,
            _ => return Err(unknown_field("A group", field, &Self::FIELDS)),
        }
        next.validate(styles)?;
        Ok(next)
    }

    /// Inserts `child`, or replaces the one with its ID in place.
    pub fn with_child(&self, child: Child, styles: &[TextStyle]) -> Result<Self, CommandError> {
        let mut next = self.clone();
        match next
            .children
            .iter()
            .position(|existing| existing.id == child.id)
        {
            Some(index) => next.children[index] = child,
            None => next.children.push(child),
        }
        next.validate(styles)?;
        Ok(next)
    }

    pub fn without_child(&self, id: &str) -> Result<Self, CommandError> {
        let waiting = self.dependants(id);
        if !waiting.is_empty() {
            return Err(CommandError::new(
                "in_use",
                format!(
                    "Child `{id}` is what {} start(s) after: change their `after` first",
                    waiting.len()
                ),
            )
            .target(format!("group_child:{}/{id}", self.id))
            .details(json!({ "children": waiting })));
        }
        let mut next = self.clone();
        let before = next.children.len();
        next.children.retain(|child| child.id != id);
        if next.children.len() == before {
            return Err(CommandError::unknown_element("group child", id)
                .target(format!("group_child:{}/{id}", self.id)));
        }
        Ok(next)
    }

    pub fn to_json(&self) -> Value {
        let mut value = json!({
            "id": self.id,
            "name": self.name,
            "children": self.children.iter().map(Child::to_json).collect::<Vec<_>>(),
        });
        if self.stagger_ms != 0 {
            value["stagger_ms"] = json!(self.stagger_ms);
        }
        value
    }

    pub fn from_json(value: &Value, styles: &[TextStyle]) -> Result<Self, CommandError> {
        let object = value
            .as_object()
            .ok_or_else(|| invalid("A group is an object", json!({})))?;
        const KEYS: [&str; 4] = ["id", "name", "stagger_ms", "children"];
        if let Some(field) = object.keys().find(|key| !KEYS.contains(&key.as_str())) {
            return Err(unknown_field("A group", field, &KEYS));
        }
        let mut def = Self::new(id_of(object, "id", "A group")?);
        if let Some(name) = object.get("name").filter(|raw| !raw.is_null()) {
            def.name = name
                .as_str()
                .ok_or_else(|| invalid("A group `name` is text", json!({ "field": "name" })))?
                .trim()
                .to_string();
        }
        if let Some(stagger) = object.get("stagger_ms").filter(|raw| !raw.is_null()) {
            def.stagger_ms = Raw::from_json(stagger)
                .ok_or_else(|| {
                    invalid("`stagger_ms` is a number", json!({ "field": "stagger_ms" }))
                })?
                .integer()?;
        }
        if let Some(children) = object.get("children").filter(|raw| !raw.is_null()) {
            def.children = children
                .as_array()
                .ok_or_else(|| invalid("`children` is an array", json!({ "field": "children" })))?
                .iter()
                .map(|child| Child::from_json(child, styles))
                .collect::<Result<_, _>>()?;
        }
        def.validate(styles)?;
        Ok(def)
    }
}

// ---- instances -----------------------------------------------------------------------------

/// What one group clip holds: the definition it shows and the texts it changes.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupInstance {
    pub def: Id,
    /// Text child ID to the text this instance shows instead of the definition's.
    pub texts: BTreeMap<Id, String>,
}

impl GroupInstance {
    pub fn new(def: Id) -> Self {
        Self {
            def,
            texts: BTreeMap::new(),
        }
    }

    pub fn to_json(&self) -> Value {
        let mut value = json!({ "def": self.def });
        if !self.texts.is_empty() {
            value["texts"] = Value::Object(
                self.texts
                    .iter()
                    .map(|(child, text)| (child.to_string(), json!(text)))
                    .collect(),
            );
        }
        value
    }

    pub fn from_json(value: &Value) -> Result<Self, CommandError> {
        let object = value
            .as_object()
            .ok_or_else(|| invalid("A group clip is `{def, texts?}`", json!({})))?;
        if let Some(field) = object
            .keys()
            .find(|key| !["def", "texts"].contains(&key.as_str()))
        {
            return Err(unknown_field("A group clip", field, &["def", "texts"]));
        }
        let mut instance = Self::new(id_of(object, "def", "A group clip")?);
        if let Some(texts) = object.get("texts").filter(|raw| !raw.is_null()) {
            for (child, text) in texts
                .as_object()
                .ok_or_else(|| invalid("`texts` maps a child ID to a text", json!({})))?
            {
                let child = Id::new(child).map_err(|error| {
                    CommandError::new("invalid_id", error.to_string()).target(child)
                })?;
                let text = text.as_str().ok_or_else(|| {
                    invalid("A text override is a string", json!({ "field": "texts" }))
                })?;
                instance.texts.insert(child, crate::text::normalize(text)?);
            }
        }
        Ok(instance)
    }
}

// ---- the book ------------------------------------------------------------------------------

/// The project's groups and text styles: part of the document and of every undo snapshot.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GroupBook {
    pub defs: Vec<GroupDef>,
    pub styles: Vec<TextStyle>,
}

/// A child of a group at one instant, ready for the plan.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub child: Id,
    pub content: PlacedContent,
    pub x: f64,
    pub y: f64,
    /// `1.0` is unscaled.
    pub scale: f64,
    pub rotation: f64,
    /// `0..=1`.
    pub opacity: f64,
    /// Text children with a reveal: how many leading characters show; `None` shows them all
    /// (no reveal, or one that already finished). `Some(0)` is not there yet.
    pub visible: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlacedContent {
    Shape(ShapeSpec),
    Text(TextSpec),
}

/// When a child of an instance starts and finishes, in milliseconds on the clip's clock. The
/// start is the stagger (`index × stagger_ms`) or, for a child with `after`, the end of that
/// child, plus the child's `delay_ms`. The end adds what the child plays: its reveal and its
/// key tracks, whichever is longer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    pub start_ms: i64,
    pub end_ms: i64,
}

/// The canvas a plan is drawn in, `(width, height)` in pixels: the lines of a text depend on it.
pub type Canvas = (u32, u32);

impl GroupBook {
    pub fn is_empty(&self) -> bool {
        self.defs.is_empty() && self.styles.is_empty()
    }

    pub fn def(&self, id: &str) -> Option<&GroupDef> {
        self.defs.iter().find(|def| def.id == id)
    }

    pub fn style(&self, id: &str) -> Option<&TextStyle> {
        self.styles.iter().find(|style| style.id == id)
    }

    /// Whole-book validity: each definition and style, unique IDs, and every text child's style.
    pub fn validate(&self) -> Result<(), CommandError> {
        for (index, style) in self.styles.iter().enumerate() {
            style.validate()?;
            if self.styles[..index]
                .iter()
                .any(|earlier| earlier.id == style.id)
            {
                return Err(CommandError::new(
                    "duplicate_id",
                    format!("Two text styles `{}`", style.id),
                ));
            }
        }
        for (index, def) in self.defs.iter().enumerate() {
            def.validate(&self.styles)?;
            if self.defs[..index]
                .iter()
                .any(|earlier| earlier.id == def.id)
            {
                return Err(CommandError::new(
                    "duplicate_id",
                    format!("Two groups `{}`", def.id),
                ));
            }
        }
        Ok(())
    }

    /// Whether any child of any definition uses `style`.
    pub fn style_in_use(&self, style: &str) -> bool {
        self.defs.iter().any(|def| {
            def.children.iter().any(|child| {
                matches!(&child.content, ChildContent::Text { style: used, .. } if used == style)
            })
        })
    }

    /// Document block; `None` while the project has no group and no style, so a project that
    /// never used one is byte-identical to what earlier editors wrote.
    pub fn to_json(&self) -> Option<Value> {
        (!self.is_empty()).then(|| {
            json!({
                "definitions": self.defs.iter().map(GroupDef::to_json).collect::<Vec<_>>(),
                "styles": self.styles.iter().map(TextStyle::to_json).collect::<Vec<_>>(),
            })
        })
    }

    pub fn from_json(value: &Value) -> Result<Self, CommandError> {
        let object = value
            .as_object()
            .ok_or_else(|| invalid("`groups` is an object", json!({})))?;
        if let Some(field) = object
            .keys()
            .find(|key| !["definitions", "styles"].contains(&key.as_str()))
        {
            return Err(unknown_field("`groups`", field, &["definitions", "styles"]));
        }
        let list = |key: &str| -> Result<Vec<Value>, CommandError> {
            match object.get(key) {
                None | Some(Value::Null) => Ok(Vec::new()),
                Some(Value::Array(items)) => Ok(items.clone()),
                Some(_) => Err(invalid(
                    format!("`{key}` is an array"),
                    json!({ "field": key }),
                )),
            }
        };
        let styles = list("styles")?
            .iter()
            .map(TextStyle::from_json)
            .collect::<Result<Vec<_>, _>>()?;
        let defs = list("definitions")?
            .iter()
            .map(|def| GroupDef::from_json(def, &styles))
            .collect::<Result<Vec<_>, _>>()?;
        let book = Self { defs, styles };
        book.validate()?;
        Ok(book)
    }

    /// Checks an instance against the book: the definition exists and every text override names
    /// a text child of it.
    pub fn check_instance(&self, instance: &GroupInstance) -> Result<(), CommandError> {
        let def = self.def(instance.def.as_str()).ok_or_else(|| {
            CommandError::unknown_element("group", instance.def.as_str())
                .target(format!("group_def:{}", instance.def))
        })?;
        for child in instance.texts.keys() {
            if !def.child(child.as_str()).is_some_and(Child::is_text) {
                return Err(CommandError::new(
                    "unknown_element",
                    format!("Group `{}` has no text child `{child}`", def.id),
                )
                .target(format!("group_child:{}/{child}", def.id)));
            }
        }
        Ok(())
    }

    fn def_of(&self, instance: &GroupInstance) -> Result<&GroupDef, CommandError> {
        self.def(instance.def.as_str()).ok_or_else(|| {
            CommandError::unknown_element("group", instance.def.as_str())
                .target(format!("group_def:{}", instance.def))
        })
    }

    /// The text a text child shows in `instance` (the clip's own, else the group's) in its
    /// style, and the lines it is drawn in. A child with a reveal always has a box: the lines
    /// are the core's (`text::lines_of`) from the first letter to the last, so the picture
    /// cannot re-wrap while it is typed. A child without a reveal and without a box in its style
    /// keeps the layout it always had.
    fn shown(
        &self,
        child: &Child,
        instance: &GroupInstance,
        canvas: Canvas,
    ) -> Result<Option<(TextSpec, Vec<String>)>, CommandError> {
        let ChildContent::Text { content, style } = &child.content else {
            return Ok(None);
        };
        let style = self.style(style.as_str()).ok_or_else(|| {
            CommandError::unknown_element("text style", style.as_str())
                .target(format!("text_style:{style}"))
        })?;
        let shown = instance.texts.get(&child.id).unwrap_or(content);
        let mut spec = style.spec(shown)?;
        if child.reveal.is_some() && spec.width == 0.0 {
            spec.width = crate::text::SAFE_AREA * 100.0;
        }
        let lines = if child.reveal.is_some() {
            crate::text::lines_of(&spec, canvas.0, canvas.1).lines
        } else {
            Vec::new()
        };
        Ok(Some((spec, lines)))
    }

    /// Start and end of every child of the instance's group (see [`Timing`]).
    pub fn timings(
        &self,
        instance: &GroupInstance,
        canvas: Canvas,
    ) -> Result<Vec<Timing>, CommandError> {
        let def = self.def_of(instance)?;
        let mut timings: Vec<Timing> = Vec::with_capacity(def.children.len());
        for (index, child) in def.children.iter().enumerate() {
            let waited = child
                .after
                .and_then(|after| def.children[..index].iter().position(|c| c.id == after))
                .map(|earlier| timings[earlier].end_ms);
            let start_ms = waited
                .unwrap_or(index as i64 * def.stagger_ms)
                .saturating_add(child.delay_ms);
            let reveal_ms = match (child.reveal, self.shown(child, instance, canvas)?) {
                (Some(reveal), Some((spec, lines))) => reveal.span_ms(&spec.content, &lines),
                _ => 0,
            };
            let tracks_ms = child
                .tracks
                .iter()
                .map(KeyTrack::span_ms)
                .max()
                .unwrap_or(0);
            timings.push(Timing {
                start_ms,
                end_ms: start_ms.saturating_add(reveal_ms.max(tracks_ms)),
            });
        }
        Ok(timings)
    }

    /// What `prop` of the child `child` of the instance shows `local_ms` after its clip starts;
    /// `None` when the group has no such child. Used to record a key at the playhead.
    pub fn value_at(
        &self,
        instance: &GroupInstance,
        child: &str,
        prop: Prop,
        local_ms: f64,
        canvas: Canvas,
    ) -> Option<f64> {
        let def = self.def_of(instance).ok()?;
        let index = def.children.iter().position(|entry| entry.id == child)?;
        let timing = self.timings(instance, canvas).ok()?[index];
        Some(GroupDef::child_value(
            &def.children[index],
            prop,
            local_ms - timing.start_ms as f64,
        ))
    }

    /// The children of `instance` at `local_ms` milliseconds after its clip starts, on `canvas`.
    pub fn place(
        &self,
        instance: &GroupInstance,
        local_ms: f64,
        canvas: Canvas,
    ) -> Result<Vec<Placed>, CommandError> {
        let def = self.def_of(instance)?;
        let timings = self.timings(instance, canvas)?;
        let mut placed = Vec::with_capacity(def.children.len());
        for (child, timing) in def.children.iter().zip(&timings) {
            let time = local_ms - timing.start_ms as f64;
            let value = |prop: Prop| GroupDef::child_value(child, prop, time);
            let mut visible = None;
            let content = match &child.content {
                ChildContent::Shape(shape) => {
                    let mut shape = shape.clone();
                    shape.width = value(Prop::Width);
                    shape.height = value(Prop::Height);
                    shape.radius = value(Prop::Radius);
                    shape.stroke_width = value(Prop::StrokeWidth);
                    PlacedContent::Shape(shape)
                }
                ChildContent::Text { .. } => {
                    let Some((spec, lines)) = self.shown(child, instance, canvas)? else {
                        continue;
                    };
                    visible = child
                        .reveal
                        .and_then(|reveal| reveal.visible(&spec.content, &lines, time));
                    PlacedContent::Text(spec)
                }
            };
            placed.push(Placed {
                child: child.id,
                content,
                x: value(Prop::X),
                y: value(Prop::Y),
                scale: value(Prop::Scale) / 100.0,
                rotation: value(Prop::Rotation),
                opacity: value(Prop::Opacity) / 100.0,
                visible,
            });
        }
        Ok(placed)
    }
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use super::*;

    fn track(prop: Prop, repeat: u32, mode: RepeatMode, keys: &[(i64, f64, Easing)]) -> KeyTrack {
        KeyTrack {
            prop,
            repeat,
            mode,
            keys: keys
                .iter()
                .map(|&(time_ms, value, easing)| Key {
                    time_ms,
                    value,
                    easing,
                })
                .collect(),
        }
    }

    fn lines(list: &[&str]) -> Vec<String> {
        list.iter().map(|line| line.to_string()).collect()
    }

    #[test]
    fn typing_shows_one_more_character_each_step_and_all_of_them_at_the_end() {
        let reveal = Reveal::new(RevealMode::Type);
        let (content, none) = ("abcd", Vec::new());
        assert_eq!(reveal.visible(content, &none, -1.0), Some(0));
        assert_eq!(reveal.visible(content, &none, 0.0), Some(1));
        assert_eq!(reveal.visible(content, &none, 39.0), Some(1));
        // A frame landing exactly on a step is that step, not the one before.
        assert_eq!(reveal.visible(content, &none, 40.0), Some(2));
        assert_eq!(reveal.visible(content, &none, 119.99), Some(3));
        assert_eq!(reveal.visible(content, &none, 120.0), None);
        assert_eq!(reveal.span_ms(content, &none), 160);
    }

    #[test]
    fn a_line_reveal_ends_each_step_on_a_whole_line_of_the_wrapped_text() {
        let reveal = Reveal {
            mode: RevealMode::Lines,
            step_ms: 100,
        };
        let wrapped = lines(&["um dois", "três", "quatro"]);
        let content = "um dois três quatro";
        assert_eq!(reveal.visible(content, &wrapped, -1.0), Some(0));
        assert_eq!(reveal.visible(content, &wrapped, 0.0), Some(7));
        assert_eq!(reveal.visible(content, &wrapped, 100.0), Some(12));
        assert_eq!(reveal.visible(content, &wrapped, 200.0), None);
        assert_eq!(reveal.span_ms(content, &wrapped), 300);
    }

    #[test]
    fn a_reveal_step_outside_its_range_is_refused() {
        for step_ms in [0, -1, MAX_STEP_MS + 1] {
            let reveal = Reveal {
                mode: RevealMode::Type,
                step_ms,
            };
            assert_eq!(reveal.validate().unwrap_err().code, "invalid_value");
        }
        assert!(Reveal::new(RevealMode::Lines).validate().is_ok());
    }

    #[test]
    fn a_track_holds_before_and_after_and_interpolates_with_the_earlier_easing() {
        let ramp = track(
            Prop::Opacity,
            1,
            RepeatMode::Restart,
            &[
                (100, 0.0, Easing::EaseOutCubic),
                (500, 100.0, Easing::Linear),
            ],
        );
        assert_eq!(
            ramp.evaluate(-50.0),
            0.0,
            "before the first key it holds the first value"
        );
        assert_eq!(ramp.evaluate(100.0), 0.0);
        assert_eq!(
            ramp.evaluate(900.0),
            100.0,
            "after the last key it holds the last value"
        );
        // easeOutCubic(0.25) = 1 - 0.75^3
        let eased = 100.0 * (1.0 - 0.75_f64.powi(3));
        assert!((ramp.evaluate(200.0) - eased).abs() < 1e-9);
        let linear = track(
            Prop::X,
            1,
            RepeatMode::Restart,
            &[(0, 0.0, Easing::Linear), (1000, 50.0, Easing::Linear)],
        );
        assert_eq!(linear.evaluate(500.0), 25.0);
    }

    #[test]
    fn repetition_restarts_or_bounces_and_rests_in_the_right_place() {
        let keys = [(0, 0.0, Easing::Linear), (1000, 10.0, Easing::Linear)];
        let restart = track(Prop::X, 3, RepeatMode::Restart, &keys);
        assert_eq!(restart.evaluate(500.0), 5.0);
        assert_eq!(
            restart.evaluate(1500.0),
            5.0,
            "the second cycle starts over"
        );
        assert_eq!(restart.evaluate(2999.0).round(), 10.0);
        assert_eq!(
            restart.evaluate(3000.0),
            10.0,
            "three plays are over: it rests on the last key"
        );
        assert_eq!(restart.evaluate(99_000.0), 10.0);

        let bounce = track(Prop::X, 2, RepeatMode::PingPong, &keys);
        assert_eq!(bounce.evaluate(250.0), 2.5);
        assert_eq!(
            bounce.evaluate(1250.0),
            7.5,
            "the second cycle runs backwards"
        );
        assert_eq!(
            bounce.evaluate(5000.0),
            0.0,
            "two plays end where the first began"
        );
        let odd = track(Prop::X, 1, RepeatMode::PingPong, &keys);
        assert_eq!(odd.evaluate(5000.0), 10.0);

        let forever = track(Prop::X, 0, RepeatMode::PingPong, &keys);
        assert_eq!(
            forever.evaluate(1_000_250.0),
            2.5,
            "an even cycle runs forwards"
        );
        assert_eq!(forever.evaluate(1_001_250.0), 7.5, "an odd one backwards");
        let single = track(Prop::X, 0, RepeatMode::Restart, &[(0, 4.0, Easing::Linear)]);
        assert_eq!(
            single.evaluate(1234.0),
            4.0,
            "one key is a constant, never a division by zero"
        );
    }

    #[test]
    fn keys_are_validated_as_a_whole() {
        let ok = track(
            Prop::Scale,
            1,
            RepeatMode::Restart,
            &[(0, 50.0, Easing::Linear), (10, 100.0, Easing::Linear)],
        );
        assert!(ok.validate().is_ok());
        for bad in [
            track(Prop::Scale, 1, RepeatMode::Restart, &[]),
            track(
                Prop::Scale,
                1,
                RepeatMode::Restart,
                &[(0, 0.5, Easing::Linear)],
            ),
            track(
                Prop::Opacity,
                1,
                RepeatMode::Restart,
                &[(0, 101.0, Easing::Linear)],
            ),
            track(
                Prop::Width,
                1,
                RepeatMode::Restart,
                &[(0, 0.0, Easing::Linear)],
            ),
            track(
                Prop::X,
                1,
                RepeatMode::Restart,
                &[(5, 0.0, Easing::Linear), (5, 1.0, Easing::Linear)],
            ),
            track(
                Prop::X,
                1,
                RepeatMode::Restart,
                &[(-1, 0.0, Easing::Linear)],
            ),
            track(
                Prop::X,
                1,
                RepeatMode::Restart,
                &[(MAX_TIME_MS + 1, 0.0, Easing::Linear)],
            ),
            track(
                Prop::X,
                MAX_REPEAT + 1,
                RepeatMode::Restart,
                &[(0, 0.0, Easing::Linear)],
            ),
            track(
                Prop::X,
                1,
                RepeatMode::Restart,
                &[(0, f64::NAN, Easing::Linear)],
            ),
        ] {
            assert_eq!(bad.validate().unwrap_err().code, "invalid_value", "{bad:?}");
        }
    }

    fn style() -> TextStyle {
        TextStyle::new(Id::lit("titulo"))
    }

    fn card() -> GroupDef {
        let styles = [style()];
        let back = Child::shape(
            Id::lit("fundo"),
            ShapeSpec::rect(50.0, 30.0, "#336699", 5.0).unwrap(),
        );
        let label = Child::text(Id::lit("rotulo"), "Etapa", Id::lit("titulo"));
        let mut def = GroupDef::new(Id::lit("cartao"));
        def = def.with_child(back, &styles).unwrap();
        def = def.with_child(label, &styles).unwrap();
        def
    }

    #[test]
    fn a_child_edit_is_validated_whole_and_a_refusal_changes_nothing() {
        let styles = [style()];
        let def = card();
        let child = def.child("fundo").unwrap();
        let wider = child
            .with_field("width", &Raw::Number(60.0), &styles)
            .unwrap();
        assert_eq!(wider.base(Prop::Width), 60.0);
        assert_eq!(child.base(Prop::Width), 50.0);
        let moved = child
            .with_field("x", &Raw::Text("12.5".into()), &styles)
            .unwrap();
        assert_eq!(moved.x, 12.5);
        for (field, value) in [
            ("opacity", 101.0),
            ("scale", 0.0),
            ("width", 0.0),
            ("x", 201.0),
        ] {
            assert_eq!(
                child
                    .with_field(field, &Raw::Number(value), &styles)
                    .unwrap_err()
                    .code,
                "invalid_value",
                "{field}"
            );
        }
        assert_eq!(
            child
                .with_field("content", &Raw::Text("x".into()), &styles)
                .unwrap_err()
                .code,
            "unknown_field"
        );
        let text = def.child("rotulo").unwrap();
        assert_eq!(
            text.with_field("radius", &Raw::Number(1.0), &styles)
                .unwrap_err()
                .code,
            "unknown_field"
        );
        assert_eq!(
            text.with_field("style", &Raw::Text("ausente".into()), &styles)
                .unwrap_err()
                .code,
            "unknown_element"
        );
        assert_eq!(
            text.with_field("content", &Raw::Text("a\tb".into()), &styles)
                .unwrap_err()
                .code,
            "unsupported_text"
        );
    }

    #[test]
    fn keys_are_added_replaced_retimed_and_removed_and_text_has_no_shape_tracks() {
        let styles = [style()];
        let def = card();
        let back = def.child("fundo").unwrap();
        let keyed = back
            .with_key(Prop::Opacity, 0, 0.0, Some(Easing::EaseOutCubic), &styles)
            .unwrap();
        let keyed = keyed
            .with_key(Prop::Opacity, 400, 100.0, None, &styles)
            .unwrap();
        let keyed = keyed.with_key(Prop::Width, 0, 10.0, None, &styles).unwrap();
        assert_eq!(keyed.tracks.len(), 2);
        let again = keyed
            .with_key(Prop::Opacity, 0, 20.0, None, &styles)
            .unwrap();
        let first = &again.track(Prop::Opacity).unwrap().keys[0];
        assert_eq!(
            (first.value, first.easing),
            (20.0, Easing::EaseOutCubic),
            "the easing stays"
        );
        let retimed = keyed
            .with_key_field(Prop::Opacity, 1, "time_ms", &Raw::Number(700.0), &styles)
            .unwrap();
        assert_eq!(retimed.track(Prop::Opacity).unwrap().keys[1].time_ms, 700);
        assert_eq!(
            keyed
                .with_key_field(Prop::Opacity, 1, "time_ms", &Raw::Number(0.0), &styles)
                .unwrap_err()
                .code,
            "invalid_value",
            "a key may not land on its neighbour"
        );
        let shorter = keyed.without_key(Prop::Width, 0).unwrap();
        assert!(
            shorter.track(Prop::Width).is_none(),
            "the track goes with its last key"
        );
        assert_eq!(
            keyed.without_key(Prop::Width, 9).unwrap_err().code,
            "unknown_element"
        );
        let looped = keyed
            .with_repeat(Prop::Opacity, Some(0), Some(RepeatMode::PingPong))
            .unwrap();
        assert_eq!(
            (
                looped.track(Prop::Opacity).unwrap().repeat,
                looped.track(Prop::Opacity).unwrap().mode
            ),
            (0, RepeatMode::PingPong)
        );
        assert_eq!(
            keyed.with_repeat(Prop::Y, Some(2), None).unwrap_err().code,
            "unknown_element"
        );

        let text = def.child("rotulo").unwrap();
        assert_eq!(
            text.with_key(Prop::Width, 0, 10.0, None, &styles)
                .unwrap_err()
                .code,
            "not_applicable"
        );
        assert!(text.with_key(Prop::Y, 0, 10.0, None, &styles).is_ok());
    }

    #[test]
    fn stagger_shifts_each_child_by_its_position_and_the_instance_changes_a_text() {
        let styles = vec![style()];
        let mut def = card();
        for child in ["fundo", "rotulo"] {
            let keyed = def
                .child(child)
                .unwrap()
                .with_key(Prop::Opacity, 0, 0.0, None, &styles)
                .unwrap()
                .with_key(Prop::Opacity, 1000, 100.0, None, &styles)
                .unwrap();
            def = def.with_child(keyed, &styles).unwrap();
        }
        def = def
            .with_field("stagger_ms", &Raw::Number(500.0), &styles)
            .unwrap();
        let book = GroupBook {
            defs: vec![def],
            styles,
        };
        book.validate().unwrap();
        let mut instance = GroupInstance::new(Id::lit("cartao"));
        let at = |instance: &GroupInstance, ms: f64| book.place(instance, ms, (1280, 720)).unwrap();
        let opacities =
            |placed: &[Placed]| placed.iter().map(|child| child.opacity).collect::<Vec<_>>();
        assert_eq!(opacities(&at(&instance, 0.0)), [0.0, 0.0]);
        assert_eq!(
            opacities(&at(&instance, 500.0)),
            [0.5, 0.0],
            "the label has not started"
        );
        assert_eq!(opacities(&at(&instance, 1000.0)), [1.0, 0.5]);
        assert_eq!(opacities(&at(&instance, 1500.0)), [1.0, 1.0]);

        instance.texts.insert(Id::lit("rotulo"), "Etapa 2".into());
        book.check_instance(&instance).unwrap();
        let PlacedContent::Text(text) = &at(&instance, 0.0)[1].content else {
            panic!("the label is text");
        };
        assert_eq!(text.content, "Etapa 2");
        assert_eq!(text.size, 6.0, "the look comes from the style");
        instance.texts.insert(Id::lit("fundo"), "x".into());
        assert_eq!(
            book.check_instance(&instance).unwrap_err().code,
            "unknown_element"
        );
    }

    #[test]
    fn shape_properties_are_keyed_on_the_shape_the_runtime_draws() {
        let styles = vec![style()];
        let def = card();
        let grown = def
            .child("fundo")
            .unwrap()
            .with_key(Prop::Width, 0, 10.0, None, &styles)
            .unwrap()
            .with_key(Prop::Width, 1000, 50.0, None, &styles)
            .unwrap()
            .with_key(Prop::Radius, 0, 0.0, None, &styles)
            .unwrap()
            .with_key(Prop::Radius, 1000, 20.0, None, &styles)
            .unwrap();
        let book = GroupBook {
            defs: vec![def.with_child(grown, &styles).unwrap()],
            styles,
        };
        let instance = GroupInstance::new(Id::lit("cartao"));
        let PlacedContent::Shape(shape) =
            &book.place(&instance, 500.0, (1280, 720)).unwrap()[0].content
        else {
            panic!("the background is a shape");
        };
        assert_eq!(
            (shape.width, shape.radius, shape.height),
            (30.0, 10.0, 30.0)
        );
        assert!(shape.validate().is_ok());
    }

    #[test]
    fn the_book_round_trips_through_its_document_form_and_rejects_the_unknown() {
        let styles = vec![style()];
        let mut def = card();
        let pulsing = def
            .child("rotulo")
            .unwrap()
            .with_key(Prop::Scale, 0, 100.0, Some(Easing::EaseInOutCubic), &styles)
            .unwrap()
            .with_key(Prop::Scale, 600, 110.0, None, &styles)
            .unwrap()
            .with_repeat(Prop::Scale, Some(0), Some(RepeatMode::PingPong))
            .unwrap();
        def = def.with_child(pulsing, &styles).unwrap();
        let book = GroupBook {
            defs: vec![def],
            styles,
        };
        let json = book.to_json().unwrap();
        assert_eq!(GroupBook::from_json(&json).unwrap(), book);
        assert!(
            GroupBook::default().to_json().is_none(),
            "no block while there is nothing"
        );

        let mut broken = json.clone();
        broken["definitions"][0]["children"][1]["style"] = json!("ausente");
        assert_eq!(
            GroupBook::from_json(&broken).unwrap_err().code,
            "unknown_element"
        );
        let mut misspelt = json.clone();
        misspelt["definitions"][0]["children"][0]["colour"] = json!("red");
        assert_eq!(
            GroupBook::from_json(&misspelt).unwrap_err().code,
            "unknown_field"
        );
        let mut text_with_shape = json;
        text_with_shape["definitions"][0]["children"][1]["shape"] = json!({});
        assert_eq!(
            GroupBook::from_json(&text_with_shape).unwrap_err().code,
            "not_applicable"
        );

        let instance =
            GroupInstance::from_json(&json!({"def":"cartao","texts":{"rotulo":"Olá"}})).unwrap();
        assert_eq!(
            GroupInstance::from_json(&instance.to_json()).unwrap(),
            instance
        );
        assert!(GroupInstance::from_json(&json!({"def":"cartao","extra":1})).is_err());
    }
}
