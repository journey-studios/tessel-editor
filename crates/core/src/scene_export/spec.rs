//! `scene.export` as typed data: what the caller asked for, before anything is read from the scene.
//!
//! Shape only. Whether the scene exists, whether the range fits it and whether the scene's audio can
//! travel are decided by [`SceneExportSnapshot::capture`](super::SceneExportSnapshot::capture), the
//! one place that owns those rules, so a script, the CLI and a Rust caller mean the same thing.

use serde_json::Value;

use crate::EditorState;
use crate::preset::{self, Preset};
use crate::state::CommandError;

/// Whether the file carries the scene's own sound. Only [`Omitted`](Self::Omitted) is ever honoured:
/// there is no mix of directed-scene audio yet, so asking for it on a scene that declares audio is
/// an error rather than a silent video (see `SceneExportSnapshot::capture`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioChoice {
    /// The default: the preset decides. A scene that declares audio is then refused.
    #[default]
    Preset,
    /// `audio: false`: the caller knowingly exports video only.
    Omitted,
}

/// A parsed `scene.export` request (without the destination, which is an adapter concern).
#[derive(Debug, Clone, PartialEq)]
pub struct SceneExportSpec {
    pub scene: String,
    pub preset: Preset,
    /// First edit frame of the scene (the scene's own timeline, not a sequence's).
    pub from: i64,
    /// Number of edit frames; `None` runs to the end of the edit.
    pub frames: Option<i64>,
    pub audio: AudioChoice,
}

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new("invalid_request", message)
}

/// An optional whole number. A float is refused even when it is integral (`2.0`): frame indices
/// are exact, and `from: 1.5` must not become frame 1 by truncation.
fn whole(request: &Value, key: &str) -> Result<Option<i64>, CommandError> {
    match request.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(raw) => raw
            .as_i64()
            .map(Some)
            .ok_or_else(|| invalid(format!("`{key}` must be a whole number of frames"))),
    }
}

impl SceneExportSpec {
    pub fn from_json(state: &EditorState, request: &Value) -> Result<Self, CommandError> {
        let scene = request
            .get("scene")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("Expected string field `scene`"))?
            .to_string();
        let preset = match request.get("preset") {
            Some(Value::String(id)) => preset::find(id, &state.project.user_presets)?,
            Some(inline @ Value::Object(_)) => Preset::from_json(inline)?,
            _ => match state.project.export_preset.as_deref() {
                Some(id) => preset::find(id, &state.project.user_presets)?,
                None => {
                    return Err(invalid(
                        "Expected `preset`: a preset ID or an inline preset",
                    ));
                }
            },
        };
        let from = whole(request, "from")?.unwrap_or(0);
        let frames = whole(request, "frames")?;
        let audio = match request.get("audio") {
            None | Some(Value::Null) | Some(Value::Bool(true)) => AudioChoice::Preset,
            Some(Value::Bool(false)) => AudioChoice::Omitted,
            Some(_) => return Err(invalid("`audio` is a boolean: `false` exports video only")),
        };
        Ok(Self {
            scene,
            preset,
            from,
            frames,
            audio,
        })
    }
}
