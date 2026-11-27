//! Clip-level content that is not an asset file: text and the scene camera.

use serde_json::{Value, json};

use crate::state::error::CommandError;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum TitleFont {
    #[default]
    Bitmap,
    Sans,
}

impl TitleFont {
    pub const OPTIONS: [&str; 2] = ["bitmap", "sans"];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bitmap => "bitmap",
            Self::Sans => "sans",
        }
    }

    pub fn parse(value: &str) -> Result<Self, CommandError> {
        match value {
            "bitmap" => Ok(Self::Bitmap),
            "sans" => Ok(Self::Sans),
            _ => Err(invalid_choice("font", &Self::OPTIONS)),
        }
    }
}

/// Built-in entrance effects; the sequence clock evaluates these into each render plan.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum TitleAnimation {
    #[default]
    None,
    Rise,
    Fade,
    Pop,
}

impl TitleAnimation {
    pub const OPTIONS: [&str; 4] = ["none", "rise", "fade", "pop"];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Rise => "rise",
            Self::Fade => "fade",
            Self::Pop => "pop",
        }
    }

    pub fn parse(value: &str) -> Result<Self, CommandError> {
        match value {
            "none" => Ok(Self::None),
            "rise" => Ok(Self::Rise),
            "fade" => Ok(Self::Fade),
            "pop" => Ok(Self::Pop),
            _ => Err(invalid_choice("animation", &Self::OPTIONS)),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum TitleAlignment {
    /// Preserve the existing centered title and its vertical safe-area behavior.
    #[default]
    Center,
    /// Position and scale the title relative to the top-left corner of its text box.
    Left,
}

impl TitleAlignment {
    pub const OPTIONS: [&str; 2] = ["center", "left"];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Center => "center",
            Self::Left => "left",
        }
    }

    pub fn parse(value: &str) -> Result<Self, CommandError> {
        match value {
            "center" => Ok(Self::Center),
            "left" => Ok(Self::Left),
            _ => Err(invalid_choice("align", &Self::OPTIONS)),
        }
    }
}

fn invalid_choice(field: &str, choices: &[&str]) -> CommandError {
    CommandError::invalid_value(
        format!("Unknown text {field}"),
        json!({ "field": field, "options": choices }),
    )
}

/// Text of a title clip, or of a text asset used as a source. Content is always NFC.
#[derive(Debug, Clone, PartialEq)]
pub struct TextSpec {
    pub content: String,
    /// Glyph cell height as a percentage of the canvas height.
    pub size: f64,
    /// `#rrggbb`.
    pub color: String,
    pub font: TitleFont,
    pub animation: TitleAnimation,
    pub align: TitleAlignment,
    pub animation_ms: u32,
    /// Width of the box the text wraps in, percent of the canvas width. `0` (the default and
    /// what every title written before F070 has) is the title-safe area. A text with a box is
    /// broken into lines by the core (`text::layout_sans`, `text::layout_in`), so a reveal by
    /// line and a typing animation agree with the picture.
    pub width: f64,
}

pub const TEXT_MAX_CHARS: usize = 256;
pub const DEFAULT_TITLE_ANIMATION_MS: u32 = 450;
pub const MAX_TITLE_ANIMATION_MS: u32 = 3000;

fn invalid_animation_ms() -> CommandError {
    CommandError::invalid_value(
        "Text animation duration must be a whole number of milliseconds",
        json!({ "field": "animation_ms", "min": 1, "max": MAX_TITLE_ANIMATION_MS }),
    )
}

impl TextSpec {
    pub const FIELDS: [&str; 7] = [
        "content",
        "size",
        "color",
        "font",
        "animation",
        "align",
        "animation_ms",
    ];

    pub fn new(
        content: impl Into<String>,
        size: f64,
        color: impl Into<String>,
    ) -> Result<Self, CommandError> {
        let spec = Self {
            content: crate::text::normalize(&content.into())?,
            size,
            color: color.into(),
            font: TitleFont::default(),
            animation: TitleAnimation::default(),
            align: TitleAlignment::default(),
            animation_ms: DEFAULT_TITLE_ANIMATION_MS,
            width: 0.0,
        };
        spec.validate()?;
        Ok(spec)
    }

    pub fn validate(&self) -> Result<(), CommandError> {
        if self.content.is_empty() || self.content.chars().count() > TEXT_MAX_CHARS {
            return Err(CommandError::invalid_value(
                "Text must have 1..=256 characters",
                json!({ "max_chars": TEXT_MAX_CHARS }),
            ));
        }
        // Every constructor normalizes; a stored spelling that is not NFC would break equality of
        // composed and decomposed titles, so it is refused rather than silently kept.
        if crate::text::normalize(&self.content)? != self.content {
            return Err(CommandError::new(
                "unsupported_text",
                "Text must be in its composed (NFC) spelling",
            ));
        }
        if !self.size.is_finite() || !(1.0..=100.0).contains(&self.size) {
            return Err(CommandError::invalid_value(
                "Text size is a percentage of the canvas height between 1 and 100",
                json!({ "min": 1, "max": 100 }),
            ));
        }
        let hex = self.color.strip_prefix('#').unwrap_or_default();
        if hex.len() != 6 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err(CommandError::invalid_value(
                "Text color is `#rrggbb`",
                json!({ "format": "#rrggbb" }),
            ));
        }
        if !(1..=MAX_TITLE_ANIMATION_MS).contains(&self.animation_ms) {
            return Err(invalid_animation_ms());
        }
        if !self.width.is_finite() || !(0.0..=100.0).contains(&self.width) {
            return Err(CommandError::invalid_value(
                "Text box width is 0 (automatic) or a percentage of the canvas width up to 100",
                json!({ "field": "width", "min": 0, "max": 100 }),
            ));
        }
        Ok(())
    }

    pub fn to_json(&self) -> Value {
        // Preserve the existing JSON representation for older projects and render clients.
        let mut value = json!({ "content": self.content, "size": self.size, "color": self.color });
        if self.font != TitleFont::default() {
            value["font"] = json!(self.font.as_str());
        }
        if self.animation != TitleAnimation::default() {
            value["animation"] = json!(self.animation.as_str());
        }
        if self.align != TitleAlignment::default() {
            value["align"] = json!(self.align.as_str());
        }
        if self.animation_ms != DEFAULT_TITLE_ANIMATION_MS {
            value["animation_ms"] = json!(self.animation_ms);
        }
        if self.width != 0.0 {
            value["width"] = json!(self.width);
        }
        value
    }

    pub fn from_json(value: &Value) -> Result<Self, CommandError> {
        let content = value
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| CommandError::new("invalid_request", "Text needs a string `content`"))?;
        let mut text = Self::new(
            content,
            value.get("size").and_then(Value::as_f64).unwrap_or(10.0),
            value
                .get("color")
                .and_then(Value::as_str)
                .unwrap_or("#ffffff"),
        )?;
        for (field, options) in [
            ("font", TitleFont::OPTIONS.as_slice()),
            ("animation", TitleAnimation::OPTIONS.as_slice()),
            ("align", TitleAlignment::OPTIONS.as_slice()),
        ] {
            if let Some(value) = value.get(field) {
                let value = value
                    .as_str()
                    .ok_or_else(|| invalid_choice(field, options))?;
                match field {
                    "font" => text.font = TitleFont::parse(value)?,
                    "animation" => text.animation = TitleAnimation::parse(value)?,
                    _ => text.align = TitleAlignment::parse(value)?,
                }
            }
        }
        if let Some(duration) = value.get("animation_ms") {
            text.animation_ms = duration
                .as_u64()
                .and_then(|number| u32::try_from(number).ok())
                .ok_or_else(invalid_animation_ms)?;
        }
        if let Some(width) = value.get("width") {
            text.width = width.as_f64().unwrap_or(f64::NAN);
        }
        text.validate()?;
        Ok(text)
    }
}

/// Scene camera of a scene clip. `orbit` makes the camera circle the target around
/// the Y axis, so seeking changes the picture without keyframes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub position: [f64; 3],
    pub target: [f64; 3],
    /// Vertical field of view in degrees.
    pub fov: f64,
    /// Degrees per second around the target's Y axis; `0` is a still camera.
    pub orbit: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            position: [0.0, 1.0, 5.0],
            target: [0.0, 0.0, 0.0],
            fov: 50.0,
            orbit: 0.0,
        }
    }
}

impl Camera {
    pub fn validate(&self) -> Result<(), CommandError> {
        let finite = self
            .position
            .iter()
            .chain(&self.target)
            .chain([&self.fov, &self.orbit])
            .all(|value| value.is_finite());
        if !finite {
            return Err(CommandError::invalid_value(
                "Camera values must be finite numbers",
                json!({}),
            ));
        }
        if !(1.0..170.0).contains(&self.fov) {
            return Err(CommandError::invalid_value(
                "Camera fov is between 1 and 170 degrees",
                json!({ "min": 1, "max": 170 }),
            ));
        }
        let distance: f64 = self
            .position
            .iter()
            .zip(&self.target)
            .map(|(a, b)| (a - b).powi(2))
            .sum();
        if distance < 1e-9 {
            return Err(CommandError::invalid_value(
                "Camera position and target must differ",
                json!({}),
            ));
        }
        // Looking straight along Y makes the fixed up vector degenerate.
        let dx = self.position[0] - self.target[0];
        let dz = self.position[2] - self.target[2];
        if dx * dx + dz * dz < 1e-12 {
            return Err(CommandError::invalid_value(
                "The camera cannot look exactly along the Y axis",
                json!({}),
            ));
        }
        Ok(())
    }

    /// Camera at `seconds` of clip-local time: the position rotated around the target's Y axis.
    pub fn at(&self, seconds: f64) -> Self {
        if self.orbit == 0.0 {
            return *self;
        }
        let angle = (self.orbit * seconds).to_radians();
        let (sin, cos) = angle.sin_cos();
        let dx = self.position[0] - self.target[0];
        let dz = self.position[2] - self.target[2];
        Self {
            position: [
                self.target[0] + dx * cos + dz * sin,
                self.position[1],
                self.target[2] - dx * sin + dz * cos,
            ],
            ..*self
        }
    }

    pub fn to_json(&self) -> Value {
        json!({ "position": self.position, "target": self.target, "fov": self.fov, "orbit": self.orbit })
    }

    pub fn from_json(value: &Value) -> Result<Self, CommandError> {
        let vec3 = |key: &str, default: [f64; 3]| -> Result<[f64; 3], CommandError> {
            let Some(raw) = value.get(key) else {
                return Ok(default);
            };
            let items = raw.as_array().filter(|items| items.len() == 3);
            let numbers: Option<Vec<f64>> =
                items.and_then(|items| items.iter().map(Value::as_f64).collect());
            match numbers {
                Some(numbers) => Ok([numbers[0], numbers[1], numbers[2]]),
                None => Err(CommandError::invalid_value(
                    format!("Camera `{key}` is [x, y, z]"),
                    json!({ "format": "[x,y,z]" }),
                )),
            }
        };
        let base = Self::default();
        let camera = Self {
            position: vec3("position", base.position)?,
            target: vec3("target", base.target)?,
            fov: value.get("fov").and_then(Value::as_f64).unwrap_or(base.fov),
            orbit: value.get("orbit").and_then(Value::as_f64).unwrap_or(0.0),
        };
        camera.validate()?;
        Ok(camera)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_rejects_unsupported_characters_bad_size_and_bad_color() {
        assert!(TextSpec::new("TESSEL 01", 12.0, "#ff8800").is_ok());
        assert_eq!(
            TextSpec::new("", 12.0, "#ffffff").unwrap_err().code,
            "invalid_value"
        );
        assert_eq!(
            TextSpec::new("preço €", 12.0, "#ffffff").unwrap_err().code,
            "unsupported_text"
        );
        assert_eq!(
            TextSpec::new("linha\tlinha", 12.0, "#ffffff")
                .unwrap_err()
                .code,
            "unsupported_text"
        );
        assert_eq!(
            TextSpec::new("linha\nlinha", 12.0, "#ffffff")
                .unwrap()
                .content,
            "linha\nlinha",
            "an explicit line break is kept (BL141.1)"
        );
        assert!(TextSpec::new("A", 0.0, "#ffffff").is_err());
        assert!(TextSpec::new("A", 10.0, "red").is_err());
    }

    #[test]
    fn camera_orbit_is_a_pure_function_of_time() {
        let camera = Camera {
            position: [4.0, 1.0, 0.0],
            target: [0.0, 0.0, 0.0],
            fov: 40.0,
            orbit: 90.0,
        };
        assert_eq!(camera.at(0.0), camera);
        let quarter = camera.at(1.0);
        assert!((quarter.position[0]).abs() < 1e-9);
        assert!((quarter.position[2] + 4.0).abs() < 1e-9);
        assert_eq!(quarter.position[1], 1.0);
        assert_eq!(camera.at(1.0), camera.at(1.0));
    }

    #[test]
    fn camera_validation_rejects_degenerate_input() {
        let mut camera = Camera::default();
        assert!(camera.validate().is_ok());
        camera.fov = 0.5;
        assert!(camera.validate().is_err());
        camera = Camera {
            position: camera.target,
            ..Camera::default()
        };
        assert!(camera.validate().is_err());
        camera = Camera {
            position: [0.0, 5.0, 0.0],
            ..Camera::default()
        };
        assert!(camera.validate().is_err());
        assert!(Camera::from_json(&json!({"position":[1,2]})).is_err());
    }
}
