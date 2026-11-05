//! Export presets (P11): the versioned description of an output, independent of any encoder.
//!
//! A preset says *what* to produce (size, frame rate policy, fit, codec family, audio, color);
//! the media adapter decides *how* with whatever FFmpeg build it finds and reports a blocker when
//! it cannot. Built-in presets are read-only; user presets live in the project document and are
//! always validated, so a hand-edited document cannot smuggle in an impossible output.

use serde_json::{Value, json};

use crate::state::CommandError;
use crate::time::Rate;

pub const PRESET_VERSION: u64 = 1;
/// H.264 4:2:0 needs even sizes; 8192 is the render surface limit of the runtime.
pub const MIN_SIDE: u32 = 16;
pub const MAX_SIDE: u32 = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    /// Whole sequence visible, bars where the aspect ratios differ (no scaling of the picture
    /// beyond the render size, no cropping).
    Contain,
    /// Output fully covered: the sequence is rendered large enough and cropped from the center.
    Cover,
}

impl Fit {
    pub fn label(self) -> &'static str {
        match self {
            Self::Contain => "contain",
            Self::Cover => "cover",
        }
    }

    pub fn parse(label: &str) -> Option<Self> {
        match label {
            "contain" | "fit" => Some(Self::Contain),
            "cover" | "fill" => Some(Self::Cover),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub builtin: bool,
    pub width: u32,
    pub height: u32,
    /// `None` keeps the sequence's own rate (the default); `Some` is an explicit override.
    pub fps: Option<Rate>,
    pub fit: Fit,
    /// x264 constant-rate-factor: lower is better (1..=51).
    pub crf: u8,
    pub audio: bool,
    pub audio_kbps: u32,
}

pub const BUILTIN_IDS: [&str; 4] = ["fullhd", "uhd4k", "vertical", "square"];

fn builtin(id: &str, name: &str, width: u32, height: u32) -> Preset {
    Preset {
        id: id.into(),
        name: name.into(),
        builtin: true,
        width,
        height,
        fps: None,
        fit: Fit::Contain,
        crf: 18,
        audio: true,
        audio_kbps: 192,
    }
}

pub fn builtins() -> Vec<Preset> {
    vec![
        builtin("fullhd", "Full HD 1080p", 1920, 1080),
        builtin("uhd4k", "UHD 4K", 3840, 2160),
        builtin("vertical", "Full HD vertical", 1080, 1920),
        builtin("square", "Square 1080", 1080, 1080),
    ]
}

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new("invalid_preset", message)
}

impl Preset {
    pub fn validate(&self) -> Result<(), CommandError> {
        let id_ok = !self.id.is_empty()
            && self.id.len() <= 48
            && self
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'));
        if !id_ok {
            return Err(invalid("A preset ID is 1-48 letters, digits, `-` or `_`"));
        }
        if self.name.trim().is_empty() || self.name.len() > 80 {
            return Err(invalid("A preset needs a name of at most 80 characters"));
        }
        for (label, side) in [("width", self.width), ("height", self.height)] {
            if !(MIN_SIDE..=MAX_SIDE).contains(&side) || side % 2 != 0 {
                return Err(CommandError::invalid_value(
                    format!(
                        "Preset {label} must be an even number between {MIN_SIDE} and {MAX_SIDE} (H.264 4:2:0)"
                    ),
                    json!({ "min": MIN_SIDE, "max": MAX_SIDE, "even": true }),
                ));
            }
        }
        // CRF 0 is x264's lossless mode, which needs the High 4:4:4 profile; this preset family
        // is 4:2:0 for playback compatibility, so the lowest valid value is 1.
        if !(1..=51).contains(&self.crf) {
            return Err(CommandError::invalid_value(
                "`crf` is between 1 and 51",
                json!({ "min": 1, "max": 51 }),
            ));
        }
        if !(32..=512).contains(&self.audio_kbps) {
            return Err(CommandError::invalid_value(
                "`audio_kbps` is between 32 and 512",
                json!({ "min": 32, "max": 512 }),
            ));
        }
        if let Some(fps) = self.fps
            && fps.as_f64() > 240.0
        {
            return Err(CommandError::invalid_value(
                "The output frame rate is at most 240 fps",
                json!({ "max": 240 }),
            ));
        }
        Ok(())
    }

    pub fn to_json(&self) -> Value {
        json!({
            "version": PRESET_VERSION, "id": self.id, "name": self.name, "builtin": self.builtin,
            "width": self.width, "height": self.height,
            "fps": self.fps.map(Rate::to_json), "fit": self.fit.label(),
            "container": "mp4",
            "video": { "codec": "h264", "crf": self.crf, "pix_fmt": "yuv420p" },
            "audio": { "enabled": self.audio, "codec": "aac", "bitrate_kbps": self.audio_kbps, "sample_rate": 48_000 },
            "color": { "space": "rec709", "range": "tv" },
        })
    }

    /// Parses and validates a preset document. Unknown containers/codecs are rejected rather
    /// than ignored: a preset must mean what it says.
    pub fn from_json(value: &Value) -> Result<Self, CommandError> {
        let version = value["version"].as_u64().unwrap_or(PRESET_VERSION);
        if version > PRESET_VERSION {
            return Err(CommandError::new(
                "unsupported_schema",
                format!(
                    "Preset version {version} is newer than this editor supports ({PRESET_VERSION})"
                ),
            ));
        }
        if let Some(container) = value["container"].as_str()
            && container != "mp4"
        {
            return Err(invalid(format!(
                "Container `{container}` is not supported; only `mp4`"
            )));
        }
        if let Some(codec) = value["video"]["codec"].as_str()
            && codec != "h264"
        {
            return Err(invalid(format!(
                "Video codec `{codec}` is not supported; only `h264`"
            )));
        }
        if let Some(codec) = value["audio"]["codec"].as_str()
            && codec != "aac"
        {
            return Err(invalid(format!(
                "Audio codec `{codec}` is not supported; only `aac`"
            )));
        }
        if let Some(rate) = value["audio"]["sample_rate"].as_u64()
            && rate != 48_000
        {
            return Err(invalid(
                "The audio sample rate of the first delivery is 48000 Hz",
            ));
        }
        let side = |key: &str| {
            value[key]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| invalid(format!("A preset needs a whole number `{key}`")))
        };
        let fps = match value.get("fps") {
            None | Some(Value::Null) => None,
            Some(raw) => Some(Rate::from_json(raw).ok_or_else(|| {
                invalid("`fps` is a rate such as 30 or {\"num\":30000,\"den\":1001}")
            })?),
        };
        let fit = match value["fit"].as_str() {
            None => Fit::Contain,
            Some(label) => {
                Fit::parse(label).ok_or_else(|| invalid("`fit` is `contain` or `cover`"))?
            }
        };
        let preset = Self {
            id: value["id"].as_str().unwrap_or_default().to_string(),
            name: value["name"].as_str().unwrap_or_default().to_string(),
            builtin: false,
            width: side("width")?,
            height: side("height")?,
            fps,
            fit,
            crf: value["video"]["crf"]
                .as_u64()
                .map_or(18, |n| n.min(255) as u8),
            audio: value["audio"]["enabled"].as_bool().unwrap_or(true),
            audio_kbps: value["audio"]["bitrate_kbps"]
                .as_u64()
                .map_or(192, |n| n.min(u64::from(u32::MAX)) as u32),
        };
        preset.validate()?;
        Ok(preset)
    }

    /// Size the renderer draws at, and the output size, for a sequence of `width × height`.
    ///
    /// `Contain` renders at the largest size with the sequence's aspect that fits inside the
    /// output (bars are added around it); `Cover` renders at the smallest that covers the
    /// output (the excess is cropped). Both are real renders at that size, never an upscale of
    /// a smaller picture. Equal aspect ratios render directly at the output size.
    pub fn geometry(&self, width: u32, height: u32) -> Geometry {
        let (out_w, out_h) = (u64::from(self.width), u64::from(self.height));
        let (seq_w, seq_h) = (u64::from(width.max(1)), u64::from(height.max(1)));
        let same_aspect = out_w * seq_h == out_h * seq_w;
        if same_aspect {
            return Geometry {
                render: (self.width, self.height),
                output: (self.width, self.height),
                fit: self.fit,
            };
        }
        // Compare out_w/seq_w with out_h/seq_h without floats.
        let width_limited = out_w * seq_h <= out_h * seq_w;
        let (scale_num, scale_den) = match (self.fit, width_limited) {
            (Fit::Contain, true) | (Fit::Cover, false) => (out_w, seq_w),
            (Fit::Contain, false) | (Fit::Cover, true) => (out_h, seq_h),
        };
        let even = |value: u64| (value.div_ceil(2) * 2).max(2) as u32;
        let render_w = even((seq_w * scale_num).div_ceil(scale_den));
        let render_h = even((seq_h * scale_num).div_ceil(scale_den));
        Geometry {
            render: (render_w, render_h),
            output: (self.width, self.height),
            fit: self.fit,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub render: (u32, u32),
    pub output: (u32, u32),
    pub fit: Fit,
}

/// `preset.list`: built-ins first, then the project's own, and the selected ID.
pub fn list(state: &crate::EditorState) -> Value {
    let user: Vec<Value> = state
        .project
        .user_presets
        .iter()
        .filter_map(|value| Preset::from_json(value).ok())
        .map(|preset| preset.to_json())
        .collect();
    json!({ "presets": builtins().iter().map(Preset::to_json).chain(user).collect::<Vec<_>>(),
        "selected": state.project.export_preset })
}

/// Looks `id` up among the built-ins, then the user presets.
pub fn find(id: &str, user: &[Value]) -> Result<Preset, CommandError> {
    if let Some(preset) = builtins().into_iter().find(|preset| preset.id == id) {
        return Ok(preset);
    }
    user.iter()
        .filter_map(|value| Preset::from_json(value).ok())
        .find(|preset| preset.id == id)
        .ok_or_else(|| CommandError::unknown_element("preset", id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_are_valid_and_cover_the_planned_outputs() {
        let all = builtins();
        assert_eq!(
            all.iter()
                .map(|p| (p.id.as_str(), p.width, p.height))
                .collect::<Vec<_>>(),
            [
                ("fullhd", 1920, 1080),
                ("uhd4k", 3840, 2160),
                ("vertical", 1080, 1920),
                ("square", 1080, 1080)
            ]
        );
        for preset in &all {
            preset.validate().unwrap();
            assert_eq!(
                Preset::from_json(&preset.to_json()).unwrap().width,
                preset.width
            );
        }
    }

    #[test]
    fn geometry_renders_at_final_size_and_never_scales_a_small_picture_up() {
        let hd = builtins()[0].clone();
        // Same aspect: direct.
        assert_eq!(hd.geometry(320, 180).render, (1920, 1080));
        // 16:9 sequence into a vertical frame: contain renders 1080x608, bars fill the rest.
        let vertical = builtins()[2].clone();
        assert_eq!(vertical.geometry(1920, 1080).render, (1080, 608));
        let cover = Preset {
            fit: Fit::Cover,
            ..vertical.clone()
        };
        assert_eq!(
            cover.geometry(1920, 1080).render,
            (3414, 1920),
            "covers 1080x1920 with real pixels"
        );
        // Vertical sequence into a 16:9 output.
        assert_eq!(hd.geometry(1080, 1920).render, (608, 1080));
        // Square from 16:9.
        assert_eq!(builtins()[3].geometry(1920, 1080).render, (1080, 608));
    }

    #[test]
    fn invalid_presets_are_rejected_with_the_reason() {
        let base = builtins()[0].to_json();
        for (patch, needle) in [
            (json!({"width": 1921}), "even"),
            (json!({"width": 8194}), "between"),
            (json!({"height": 8}), "between"),
            (json!({"container": "mov"}), "Container"),
            (json!({"video": {"codec": "hevc"}}), "Video codec"),
            (json!({"audio": {"codec": "opus"}}), "Audio codec"),
            (json!({"fit": "stretch"}), "fit"),
            (json!({"fps": 500}), "fps"),
            (json!({"version": 9}), "newer"),
            (json!({"id": "bad id!"}), "ID"),
            (json!({"video": {"crf": 99}}), "crf"),
        ] {
            let mut value = base.clone();
            for (key, new) in patch.as_object().unwrap() {
                value[key] = new.clone();
            }
            let error = Preset::from_json(&value).unwrap_err();
            assert!(
                error.message.contains(needle)
                    || error
                        .message
                        .to_lowercase()
                        .contains(&needle.to_lowercase()),
                "{patch}: {}",
                error.message
            );
        }
    }
}
