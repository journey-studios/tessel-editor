//! Clip-level content that is not an asset file: text and the scene camera.

use serde_json::{Value, json};

use crate::state::error::CommandError;

/// Text of a title clip, or of a text asset used as a source.
///
/// Rendered by the runtime's built-in bitmap font (see `docs/features/viewport.md`):
/// ASCII only, no shaping, no system fonts, so output does not depend on installed fonts.
#[derive(Debug, Clone, PartialEq)]
pub struct TextSpec {
    pub content: String,
    /// Glyph cell height as a percentage of the canvas height.
    pub size: f64,
    /// `#rrggbb`.
    pub color: String,
}

pub const TEXT_MAX_CHARS: usize = 256;

impl TextSpec {
    pub fn new(
        content: impl Into<String>,
        size: f64,
        color: impl Into<String>,
    ) -> Result<Self, CommandError> {
        let spec = Self {
            content: content.into(),
            size,
            color: color.into(),
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
        if let Some(ch) = self
            .content
            .chars()
            .find(|ch| !ch.is_ascii() || ch.is_ascii_control())
        {
            return Err(CommandError::new(
                "unsupported_text",
                format!("`{ch}` is outside the built-in ASCII bitmap font"),
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
        Ok(())
    }

    pub fn to_json(&self) -> Value {
        json!({ "content": self.content, "size": self.size, "color": self.color })
    }

    pub fn from_json(value: &Value) -> Result<Self, CommandError> {
        let content = value
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| CommandError::new("invalid_request", "Text needs a string `content`"))?;
        Self::new(
            content,
            value.get("size").and_then(Value::as_f64).unwrap_or(10.0),
            value
                .get("color")
                .and_then(Value::as_str)
                .unwrap_or("#ffffff"),
        )
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
    fn text_rejects_non_ascii_bad_size_and_bad_color() {
        assert!(TextSpec::new("TESSEL 01", 12.0, "#ff8800").is_ok());
        assert_eq!(
            TextSpec::new("", 12.0, "#ffffff").unwrap_err().code,
            "invalid_value"
        );
        assert_eq!(
            TextSpec::new("café", 12.0, "#ffffff").unwrap_err().code,
            "unsupported_text"
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
