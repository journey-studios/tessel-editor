//! Editable vector shapes drawn by the compositor itself (F067 step 1A, F068 step 1B).
//!
//! A shape clip has no media file: its specification lives in the document, the plan carries it
//! to the runtime, and the monitor and the export draw the same picture from it. `ShapeKind` is
//! the seam new geometries extend: the rectangle (1A), the ellipse and the path (1B).
//!
//! Sizes are percentages of the sequence canvas, so a project keeps its layout when the output
//! resolution changes, like the position and zoom of every other visual clip.
//!
//! Step 1B adds, on the same flat spec so one `set clip_shape:<id> <field> <value>` serves every
//! property: a gradient fill (`fill_mode`, `fill_to`, `gradient_angle`), a contour (`stroke`,
//! `stroke_width`), a drop shadow (`shadow*`) and the path data (`path`). Every new field has a
//! neutral default, so a spec written by step 1A reads back unchanged and draws the same pixels.

use serde_json::{Map, Value, json};

use crate::shape_path;
use crate::state::error::{CommandError, Raw};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ShapeKind {
    #[default]
    Rect,
    Ellipse,
    /// SVG-subset path data stretched over the shape's box (`ShapeSpec::path`).
    Path,
}

impl ShapeKind {
    pub const OPTIONS: [&str; 3] = ["rect", "ellipse", "path"];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rect => "rect",
            Self::Ellipse => "ellipse",
            Self::Path => "path",
        }
    }

    pub fn parse(value: &str) -> Result<Self, CommandError> {
        match value {
            "rect" => Ok(Self::Rect),
            "ellipse" => Ok(Self::Ellipse),
            "path" => Ok(Self::Path),
            _ => Err(CommandError::invalid_value(
                "Unknown shape kind",
                json!({ "field": "kind", "options": Self::OPTIONS }),
            )),
        }
    }
}

/// How the inside of a shape is painted.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum FillMode {
    /// `fill` alone.
    #[default]
    Solid,
    /// `fill` to `fill_to` along `gradient_angle`.
    Linear,
    /// `fill` at the centre to `fill_to` at the edge of the shape's box (an ellipse of the box).
    Radial,
    /// No fill: only the contour (and the shadow of it) is drawn.
    None,
}

impl FillMode {
    pub const OPTIONS: [&str; 4] = ["solid", "linear", "radial", "none"];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Solid => "solid",
            Self::Linear => "linear",
            Self::Radial => "radial",
            Self::None => "none",
        }
    }

    pub fn parse(value: &str) -> Result<Self, CommandError> {
        match value {
            "solid" => Ok(Self::Solid),
            "linear" => Ok(Self::Linear),
            "radial" => Ok(Self::Radial),
            "none" => Ok(Self::None),
            _ => Err(CommandError::invalid_value(
                "Unknown fill mode",
                json!({ "field": "fill_mode", "options": Self::OPTIONS }),
            )),
        }
    }
}

/// Largest side of a shape, in percent of the canvas side. Twice the canvas lets a shape bleed
/// past every edge after the clip's own position and zoom are applied.
pub const SHAPE_MAX_EXTENT: f64 = 200.0;
/// Largest corner radius, in percent of the canvas height. The drawn radius never passes half of
/// the shorter side of the rectangle, so a large value gives a pill, not a distorted corner.
pub const SHAPE_MAX_RADIUS: f64 = 100.0;
/// Largest contour and shadow blur, in percent of the canvas height. They grow the picture the
/// runtime draws beyond the shape's box, so they are bounded to keep that picture a texture.
pub const SHAPE_MAX_STROKE: f64 = 20.0;
pub const SHAPE_MAX_BLUR: f64 = 20.0;
/// Largest shadow offset on either axis, in percent of the canvas width (x) or height (y).
pub const SHAPE_MAX_OFFSET: f64 = 30.0;
/// Gradient direction limit, in degrees.
pub const SHAPE_MAX_ANGLE: f64 = 360.0;
pub const DEFAULT_SHAPE_WIDTH: f64 = 40.0;
pub const DEFAULT_SHAPE_HEIGHT: f64 = 20.0;
pub const DEFAULT_SHAPE_FILL: &str = "#4a90e2";
pub const DEFAULT_FILL_TO: &str = "#ffffff";
pub const DEFAULT_STROKE: &str = "#ffffff";
/// Black at half strength: a shadow that is visible as soon as a blur or an offset is set.
pub const DEFAULT_SHADOW: &str = "#00000080";
/// Most colour stops a gradient holds between `fill` (at 0) and `fill_to` (at 1).
pub const SHAPE_MAX_STOPS: usize = 6;

/// The canonical spelling of intermediate gradient stops (F072): `<at> <colour>` pairs separated by
/// single spaces, `at` in `(0, 1)` and strictly increasing, the colour `#rrggbb[aa]` in lower
/// case. Commas and any run of whitespace between tokens are accepted on input; the empty text is
/// "no intermediate stop" (the two-colour gradient of step 1B). Stored canonically so the same
/// gradient has the same text and the same cached picture.
pub fn canonical_stops(text: &str) -> Result<String, CommandError> {
    let tokens: Vec<&str> = text
        .split(|ch: char| ch.is_whitespace() || ch == ',')
        .filter(|token| !token.is_empty())
        .collect();
    let error = |message: String| {
        CommandError::invalid_value(
            message,
            json!({ "field": "stops", "format": "<at> <#rrggbb[aa]> ...", "max": SHAPE_MAX_STOPS }),
        )
    };
    if !tokens.len().is_multiple_of(2) {
        return Err(error("Shape stops are `<at> <colour>` pairs".to_string()));
    }
    if tokens.len() / 2 > SHAPE_MAX_STOPS {
        return Err(error(format!(
            "A gradient holds at most {SHAPE_MAX_STOPS} intermediate stops"
        )));
    }
    let mut previous = 0.0;
    let mut pairs = Vec::new();
    for pair in tokens.chunks(2) {
        let at = pair[0]
            .parse::<f64>()
            .ok()
            .filter(|at| at.is_finite() && *at > previous && *at < 1.0)
            .ok_or_else(|| {
                error(format!(
                    "Stop position `{}` must be above the previous one (or 0) and below 1",
                    pair[0]
                ))
            })?;
        let color = pair[1].to_ascii_lowercase();
        if !is_color(&color) {
            return Err(error(format!(
                "Stop colour `{}` is `#rrggbb` or `#rrggbbaa`",
                pair[1]
            )));
        }
        previous = at;
        pairs.push(format!("{at} {color}"));
    }
    Ok(pairs.join(" "))
}

/// The stops of a canonical text as `(at, colour)`; empty for the empty text.
pub fn parse_stops(canonical: &str) -> Vec<(f64, String)> {
    canonical
        .split(' ')
        .collect::<Vec<_>>()
        .chunks(2)
        .filter_map(|pair| Some((pair.first()?.parse().ok()?, (*pair.get(1)?).to_string())))
        .collect()
}

/// Specification of a shape clip.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeSpec {
    pub kind: ShapeKind,
    /// Percent of the canvas width.
    pub width: f64,
    /// Percent of the canvas height.
    pub height: f64,
    /// `#rrggbb` or `#rrggbbaa`, lower case. The solid colour, or the gradient's start. The clip's
    /// opacity and fades are the shape's transparency, on top of any alpha in the colour.
    pub fill: String,
    /// Percent of the canvas height; `0` is a sharp corner. Rectangles only.
    pub radius: f64,
    pub fill_mode: FillMode,
    /// The gradient's end colour (`#rrggbb` or `#rrggbbaa`). Ignored by `solid` and `none`.
    pub fill_to: String,
    /// Intermediate stops of the gradient between `fill` (at 0) and `fill_to` (at 1), in the
    /// canonical text of [`canonical_stops`]; empty is the two-colour gradient (F072).
    pub stops: String,
    /// Degrees of the linear gradient: `0` runs left to right, `90` top to bottom.
    pub gradient_angle: f64,
    /// Contour colour (`#rrggbb` or `#rrggbbaa`). Drawn only while `stroke_width` is above `0`.
    pub stroke: String,
    /// Contour thickness, percent of the canvas height, centred on the edge of the box so half of
    /// it lies outside the declared size. `0` draws no contour.
    pub stroke_width: f64,
    /// Shadow colour (`#rrggbb` or `#rrggbbaa`). Drawn while the blur or an offset is not `0`.
    pub shadow: String,
    /// Shadow softness, percent of the canvas height.
    pub shadow_blur: f64,
    /// Shadow offset, percent of the canvas width (positive is right).
    pub shadow_x: f64,
    /// Shadow offset, percent of the canvas height (positive is down).
    pub shadow_y: f64,
    /// Canonical path data in a 100 × 100 box (see [`shape_path`]). Drawn by `kind = path` only.
    pub path: String,
}

impl Default for ShapeSpec {
    fn default() -> Self {
        Self {
            kind: ShapeKind::Rect,
            width: DEFAULT_SHAPE_WIDTH,
            height: DEFAULT_SHAPE_HEIGHT,
            fill: DEFAULT_SHAPE_FILL.to_string(),
            radius: 0.0,
            fill_mode: FillMode::Solid,
            fill_to: DEFAULT_FILL_TO.to_string(),
            stops: String::new(),
            gradient_angle: 0.0,
            stroke: DEFAULT_STROKE.to_string(),
            stroke_width: 0.0,
            shadow: DEFAULT_SHADOW.to_string(),
            shadow_blur: 0.0,
            shadow_x: 0.0,
            shadow_y: 0.0,
            path: shape_path::DEFAULT_PATH.to_string(),
        }
    }
}

fn unknown_field(field: &str) -> CommandError {
    CommandError::new("unknown_field", format!("A shape has no field `{field}`"))
        .details(json!({ "available": ShapeSpec::FIELDS }))
}

fn extent_error(field: &str) -> CommandError {
    CommandError::invalid_value(
        format!("Shape {field} is a percentage of the canvas above 0 and up to {SHAPE_MAX_EXTENT}"),
        json!({ "field": field, "min_exclusive": 0, "max": SHAPE_MAX_EXTENT }),
    )
}

fn range_error(field: &str, min: f64, max: f64, unit: &str) -> CommandError {
    CommandError::invalid_value(
        format!("Shape {field} is between {min} and {max} ({unit})"),
        json!({ "field": field, "min": min, "max": max, "unit": unit }),
    )
}

/// `#rrggbb` or `#rrggbbaa` in lower case, the one spelling a stored colour has.
pub(crate) fn is_color(value: &str) -> bool {
    let Some(hex) = value.strip_prefix('#') else {
        return false;
    };
    matches!(hex.len(), 6 | 8)
        && hex
            .chars()
            .all(|ch| ch.is_ascii_digit() || ('a'..='f').contains(&ch))
}

fn color_error(field: &str) -> CommandError {
    CommandError::invalid_value(
        format!("Shape {field} is `#rrggbb` or `#rrggbbaa`"),
        json!({ "field": field, "format": "#rrggbb or #rrggbbaa" }),
    )
}

impl ShapeSpec {
    /// Every field, `kind` first: `from_json` applies them in this order, so a kind is known
    /// before the fields that depend on it are looked at.
    pub const FIELDS: [&str; 16] = [
        "kind",
        "width",
        "height",
        "fill",
        "radius",
        "fill_mode",
        "fill_to",
        "stops",
        "gradient_angle",
        "stroke",
        "stroke_width",
        "shadow",
        "shadow_blur",
        "shadow_x",
        "shadow_y",
        "path",
    ];
    /// The fields step 1A wrote. A document keeps exactly these while the rest sit at their
    /// defaults, so a project that only uses what 1A offered still opens in a 1A editor.
    const BASE_FIELDS: [&str; 5] = ["kind", "width", "height", "fill", "radius"];

    pub fn rect(
        width: f64,
        height: f64,
        fill: impl Into<String>,
        radius: f64,
    ) -> Result<Self, CommandError> {
        let spec = Self {
            kind: ShapeKind::Rect,
            width,
            height,
            fill: fill.into().to_ascii_lowercase(),
            radius,
            ..Self::default()
        };
        spec.validate()?;
        Ok(spec)
    }

    pub fn validate(&self) -> Result<(), CommandError> {
        for (field, value) in [("width", self.width), ("height", self.height)] {
            if !value.is_finite() || value <= 0.0 || value > SHAPE_MAX_EXTENT {
                return Err(extent_error(field));
            }
        }
        if !self.radius.is_finite() || !(0.0..=SHAPE_MAX_RADIUS).contains(&self.radius) {
            return Err(CommandError::invalid_value(
                format!(
                    "Shape radius is a percentage of the canvas height between 0 and {SHAPE_MAX_RADIUS}"
                ),
                json!({ "field": "radius", "min": 0, "max": SHAPE_MAX_RADIUS }),
            ));
        }
        for (field, value, min, max, unit) in [
            (
                "gradient_angle",
                self.gradient_angle,
                -SHAPE_MAX_ANGLE,
                SHAPE_MAX_ANGLE,
                "degrees",
            ),
            (
                "stroke_width",
                self.stroke_width,
                0.0,
                SHAPE_MAX_STROKE,
                "% of the canvas height",
            ),
            (
                "shadow_blur",
                self.shadow_blur,
                0.0,
                SHAPE_MAX_BLUR,
                "% of the canvas height",
            ),
            (
                "shadow_x",
                self.shadow_x,
                -SHAPE_MAX_OFFSET,
                SHAPE_MAX_OFFSET,
                "% of the canvas width",
            ),
            (
                "shadow_y",
                self.shadow_y,
                -SHAPE_MAX_OFFSET,
                SHAPE_MAX_OFFSET,
                "% of the canvas height",
            ),
        ] {
            if !value.is_finite() || !(min..=max).contains(&value) {
                return Err(range_error(field, min, max, unit));
            }
        }
        for (field, value) in [
            ("fill", &self.fill),
            ("fill_to", &self.fill_to),
            ("stroke", &self.stroke),
            ("shadow", &self.shadow),
        ] {
            if !is_color(value) {
                return Err(color_error(field));
            }
        }
        if canonical_stops(&self.stops)? != self.stops {
            return Err(CommandError::invalid_value(
                "Shape stops are not in their canonical spelling",
                json!({ "field": "stops", "canonical": "0.5 #ff000080" }),
            ));
        }
        // The stored path is already canonical, so a hand-built spec with another spelling is a
        // mistake to surface here, not to silently redraw differently.
        if shape_path::canonicalize(&self.path)? != self.path {
            return Err(CommandError::invalid_value(
                "Shape path is not in its canonical spelling",
                json!({ "field": "path", "canonical": "M 0 0 L 10 10 Z" }),
            ));
        }
        Ok(())
    }

    /// Whether the shape paints any pixel of its own: a fill, or a contour with a thickness.
    pub fn draws_anything(&self) -> bool {
        self.fill_mode != FillMode::None || self.stroke_width > 0.0
    }

    /// The spec with one field replaced. The whole result is validated before it is returned, so
    /// a refused value leaves the caller's spec untouched.
    pub fn with_field(&self, field: &str, value: &Raw) -> Result<Self, CommandError> {
        let mut next = self.clone();
        let color = || value.text().trim().to_ascii_lowercase();
        match field {
            "kind" => next.kind = ShapeKind::parse(value.text().trim())?,
            "fill_mode" => next.fill_mode = FillMode::parse(value.text().trim())?,
            "width" => next.width = value.number()?,
            "height" => next.height = value.number()?,
            "radius" => next.radius = value.number()?,
            "gradient_angle" => next.gradient_angle = value.number()?,
            "stroke_width" => next.stroke_width = value.number()?,
            "shadow_blur" => next.shadow_blur = value.number()?,
            "shadow_x" => next.shadow_x = value.number()?,
            "shadow_y" => next.shadow_y = value.number()?,
            "fill" => next.fill = color(),
            "fill_to" => next.fill_to = color(),
            "stroke" => next.stroke = color(),
            "shadow" => next.shadow = color(),
            "path" => next.path = shape_path::canonicalize(&value.text())?,
            "stops" => next.stops = canonical_stops(&value.text())?,
            _ => return Err(unknown_field(field)),
        }
        next.validate()?;
        Ok(next)
    }

    fn field_json(&self, field: &str) -> Value {
        match field {
            "kind" => json!(self.kind.as_str()),
            "width" => json!(self.width),
            "height" => json!(self.height),
            "fill" => json!(self.fill),
            "radius" => json!(self.radius),
            "fill_mode" => json!(self.fill_mode.as_str()),
            "fill_to" => json!(self.fill_to),
            "stops" => json!(self.stops),
            "gradient_angle" => json!(self.gradient_angle),
            "stroke" => json!(self.stroke),
            "stroke_width" => json!(self.stroke_width),
            "shadow" => json!(self.shadow),
            "shadow_blur" => json!(self.shadow_blur),
            "shadow_x" => json!(self.shadow_x),
            "shadow_y" => json!(self.shadow_y),
            "path" => json!(self.path),
            _ => Value::Null,
        }
    }

    /// Every field: what the plan carries to the runtime, so the compositor never depends on the
    /// defaults of the editor that wrote the plan.
    pub fn to_json(&self) -> Value {
        Value::Object(
            Self::FIELDS
                .iter()
                .map(|field| ((*field).to_string(), self.field_json(field)))
                .collect::<Map<_, _>>(),
        )
    }

    /// What the project document stores: the step-1A fields always, and a step-1B field only when
    /// it differs from its default. Reading fills the defaults back, so the round trip is exact,
    /// and a project that never used a 1B feature stays byte-identical to what 1A wrote.
    pub fn to_document_json(&self) -> Value {
        let defaults = Self::default();
        Value::Object(
            Self::FIELDS
                .iter()
                .filter(|field| {
                    Self::BASE_FIELDS.contains(field)
                        || self.field_json(field) != defaults.field_json(field)
                })
                .map(|field| ((*field).to_string(), self.field_json(field)))
                .collect::<Map<_, _>>(),
        )
    }

    /// Absent fields take the defaults; unknown ones are refused so a typo cannot pass silently.
    pub fn from_json(value: &Value) -> Result<Self, CommandError> {
        let object = value
            .as_object()
            .ok_or_else(|| CommandError::new("invalid_request", "A shape is an object"))?;
        if let Some(field) = object
            .keys()
            .find(|key| !Self::FIELDS.contains(&key.as_str()))
        {
            return Err(unknown_field(field));
        }
        let mut spec = Self::default();
        for field in Self::FIELDS {
            let Some(raw) = object.get(field) else {
                continue;
            };
            let raw = Raw::from_json(raw).ok_or_else(|| {
                CommandError::invalid_value(
                    format!("Shape {field} is a string or a number"),
                    json!({ "field": field }),
                )
            })?;
            spec = spec.with_field(field, &raw)?;
        }
        spec.validate()?;
        Ok(spec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shape_round_trips_through_json_with_every_field() {
        let spec = ShapeSpec::rect(30.0, 12.5, "#FFAA00", 4.0).unwrap();
        assert_eq!(spec.fill, "#ffaa00", "the fill is stored in one spelling");
        let json = spec.to_json();
        assert_eq!(json["kind"], "rect");
        assert_eq!(ShapeSpec::from_json(&json).unwrap(), spec);
        assert_eq!(
            ShapeSpec::from_json(&json!({})).unwrap(),
            ShapeSpec::default()
        );
        for field in ShapeSpec::FIELDS {
            assert!(!json[field].is_null(), "the plan carries `{field}`");
        }
    }

    #[test]
    fn invalid_values_are_refused_before_a_spec_exists() {
        for (width, height, fill, radius) in [
            (0.0, 10.0, "#ffffff", 0.0),
            (10.0, -1.0, "#ffffff", 0.0),
            (200.5, 10.0, "#ffffff", 0.0),
            (10.0, f64::NAN, "#ffffff", 0.0),
            (10.0, 10.0, "red", 0.0),
            (10.0, 10.0, "#fff", 0.0),
            (10.0, 10.0, "#fffffff", 0.0),
            (10.0, 10.0, "#ffffff", -0.1),
            (10.0, 10.0, "#ffffff", 100.1),
        ] {
            let error = ShapeSpec::rect(width, height, fill, radius).unwrap_err();
            assert_eq!(
                error.code, "invalid_value",
                "{width} {height} {fill} {radius}"
            );
        }
    }

    #[test]
    fn one_field_changes_without_touching_the_others_and_a_refusal_changes_nothing() {
        let spec = ShapeSpec::default();
        let wider = spec.with_field("width", &Raw::Text("55".into())).unwrap();
        assert_eq!(wider.width, 55.0);
        assert_eq!((wider.height, &wider.fill), (spec.height, &spec.fill));
        assert!(spec.with_field("width", &Raw::Number(0.0)).is_err());
        assert_eq!(
            spec.with_field("depth", &Raw::Number(1.0))
                .unwrap_err()
                .code,
            "unknown_field"
        );
        assert_eq!(spec, ShapeSpec::default());
    }

    #[test]
    fn unknown_keys_and_kinds_are_refused() {
        assert_eq!(
            ShapeSpec::from_json(&json!({"colour": "#ffffff"}))
                .unwrap_err()
                .code,
            "unknown_field"
        );
        assert_eq!(
            ShapeSpec::from_json(&json!({"kind": "star"}))
                .unwrap_err()
                .code,
            "invalid_value"
        );
        assert_eq!(
            ShapeSpec::from_json(&json!({"fill_mode": "conic"}))
                .unwrap_err()
                .code,
            "invalid_value"
        );
    }

    #[test]
    fn the_new_kinds_fills_contour_and_shadow_are_edited_one_field_at_a_time() {
        let mut spec = ShapeSpec::default();
        for (field, value) in [
            ("kind", "ellipse"),
            ("fill_mode", "radial"),
            ("fill", "#FFD166"),
            ("fill_to", "#ffd16600"),
            ("stroke", "#102030"),
            ("stroke_width", "1.5"),
            ("shadow", "#000000cc"),
            ("shadow_blur", "4"),
            ("shadow_x", "-1"),
            ("shadow_y", "2.5"),
            ("gradient_angle", "-45"),
        ] {
            spec = spec
                .with_field(field, &Raw::Text(value.into()))
                .unwrap_or_else(|error| panic!("{field}: {error:?}"));
        }
        assert_eq!(spec.kind, ShapeKind::Ellipse);
        assert_eq!(spec.fill_mode, FillMode::Radial);
        assert_eq!(spec.fill, "#ffd166");
        assert_eq!(spec.fill_to, "#ffd16600");
        assert_eq!((spec.shadow_x, spec.shadow_y), (-1.0, 2.5));
        assert_eq!(ShapeSpec::from_json(&spec.to_json()).unwrap(), spec);
        assert_eq!(
            ShapeSpec::from_json(&spec.to_document_json()).unwrap(),
            spec
        );
    }

    #[test]
    fn out_of_range_new_values_are_refused_and_name_the_field() {
        let spec = ShapeSpec::default();
        for (field, value) in [
            ("stroke_width", "20.5"),
            ("stroke_width", "-1"),
            ("shadow_blur", "21"),
            ("shadow_x", "30.5"),
            ("shadow_y", "-31"),
            ("gradient_angle", "361"),
            ("fill_to", "white"),
            ("stroke", "#12"),
            ("shadow", "#1234567"),
            ("path", "M 10"),
            ("path", "M 0 0 A 1 1 0 0 0 5 5"),
            ("fill_mode", "dotted"),
            ("kind", "polygon"),
        ] {
            let error = spec
                .with_field(field, &Raw::Text(value.into()))
                .unwrap_err();
            assert_eq!(error.code, "invalid_value", "{field}={value}");
        }
    }

    #[test]
    fn a_path_is_stored_canonically_and_a_hand_built_other_spelling_is_refused() {
        let spec = ShapeSpec::default()
            .with_field("path", &Raw::Text("M0,0L100 100".into()))
            .unwrap();
        assert_eq!(spec.path, "M 0 0 L 100 100");
        let mut loose = spec.clone();
        loose.path = "M0 0 L100 100".into();
        assert!(loose.validate().is_err());
    }

    #[test]
    fn the_document_keeps_the_1a_keys_and_only_the_1b_fields_that_were_changed() {
        let plain = ShapeSpec::rect(50.0, 12.5, "#102030", 20.0).unwrap();
        let document = plain.to_document_json();
        let mut keys: Vec<_> = document.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["fill", "height", "kind", "radius", "width"]);
        // A 1A reader sees exactly the object it wrote; reading it back is lossless.
        assert_eq!(ShapeSpec::from_json(&document).unwrap(), plain);

        let outlined = plain
            .with_field("stroke_width", &Raw::Number(2.0))
            .unwrap()
            .with_field("shadow_blur", &Raw::Number(3.0))
            .unwrap();
        let document = outlined.to_document_json();
        assert_eq!(document["stroke_width"], 2.0);
        assert_eq!(document["shadow_blur"], 3.0);
        assert!(document.get("fill_mode").is_none());
        assert!(document.get("path").is_none());
        assert_eq!(ShapeSpec::from_json(&document).unwrap(), outlined);
    }

    #[test]
    fn stops_are_stored_canonically_refused_when_malformed_and_left_out_of_a_plain_document() {
        let spec = ShapeSpec::default()
            .with_field("fill_mode", &Raw::Text("linear".into()))
            .unwrap()
            .with_field(
                "stops",
                &Raw::Text(" 0.58,#C6A7FF60\t0.92   #c6a7fffa ".into()),
            )
            .unwrap();
        assert_eq!(spec.stops, "0.58 #c6a7ff60 0.92 #c6a7fffa");
        assert_eq!(
            parse_stops(&spec.stops),
            vec![
                (0.58, "#c6a7ff60".to_string()),
                (0.92, "#c6a7fffa".to_string())
            ]
        );
        assert!(parse_stops("").is_empty());

        // The document keeps the key only when there are stops, and reads it back exactly.
        let document = spec.to_document_json();
        assert_eq!(document["stops"], "0.58 #c6a7ff60 0.92 #c6a7fffa");
        assert_eq!(ShapeSpec::from_json(&document).unwrap(), spec);
        assert!(
            ShapeSpec::default()
                .to_document_json()
                .get("stops")
                .is_none(),
            "a shape with no stops is written as it was before they existed"
        );

        for (text, why) in [
            ("0.5", "a position without a colour"),
            ("0.5 red", "a colour that is not hex"),
            ("0 #ffffff", "a stop at the start is `fill`"),
            ("1 #ffffff", "a stop at the end is `fill_to`"),
            ("0.7 #ffffff 0.7 #000000", "equal positions"),
            ("0.7 #ffffff 0.3 #000000", "out of order"),
            ("1.5 #ffffff", "past the end"),
            ("0.1 #fff", "a short colour"),
            (
                "0.1 #ffffff 0.2 #ffffff 0.3 #ffffff 0.4 #ffffff 0.5 #ffffff 0.6 #ffffff 0.7 #ffffff",
                "seven stops",
            ),
        ] {
            let error = spec
                .with_field("stops", &Raw::Text(text.into()))
                .unwrap_err();
            assert_eq!(error.code, "invalid_value", "{why}: {text}");
        }
        assert_eq!(
            spec.with_field("stops", &Raw::Text("".into()))
                .unwrap()
                .stops,
            ""
        );

        // A hand-built spec in another spelling is a mistake surfaced by `validate`.
        let mut loose = spec.clone();
        loose.stops = "0.58  #C6A7FF60".into();
        assert!(loose.validate().is_err());
    }

    #[test]
    fn a_shape_that_paints_nothing_says_so() {
        let spec = ShapeSpec::default();
        assert!(spec.draws_anything());
        let hollow = spec
            .with_field("fill_mode", &Raw::Text("none".into()))
            .unwrap();
        assert!(!hollow.draws_anything(), "no fill and no contour");
        let outlined = hollow
            .with_field("stroke_width", &Raw::Number(1.0))
            .unwrap();
        assert!(outlined.draws_anything());
    }
}
