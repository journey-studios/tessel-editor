//! The `clip.key*` vocabulary of F073: keyframes of a clip's position, zoom, rotation and opacity.
//!
//! Shape only: every rule (which clips and properties can be keyed, limits, ordering) belongs to
//! `clip_keys` and `state::clip_key_edit`, so a request means the same through the CLI, a script,
//! the socket and the library.

use super::*;
use crate::clip_keys;
use crate::easing::KeyEasing;
use crate::group::Prop;
use crate::state::clip_key_edit::ClipKeysCommand;

/// `None` when the request does not say (omitting the easing is never a choice of `linear`).
/// Shadows the camera keys' narrower `easing_field` of the parent module: a clip key accepts the
/// directed three plus `smoothStep`, like a group key.
fn easing_field(request: &Value) -> Result<Option<KeyEasing>, CommandError> {
    match request.get("easing").filter(|raw| !raw.is_null()) {
        None => Ok(None),
        Some(raw) => raw
            .as_str()
            .and_then(KeyEasing::parse)
            .map(Some)
            .ok_or_else(|| {
                CommandError::invalid_value(
                    "`easing` is not an option",
                    json!({ "options": KeyEasing::labels() }),
                )
            }),
    }
}

/// The keyed inspector property a request names: `prop` (`position_x`, `position_y`, `zoom`,
/// `rotation`, `opacity`).
fn prop_field(request: &Value) -> Result<Prop, CommandError> {
    clip_keys::parse_property(string(request, "prop")?)
}

fn number(request: &Value, key: &str) -> Result<f64, CommandError> {
    request
        .get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| invalid(format!("Expected number field `{key}`")))
}

/// `None` when `op` is not a clip-key operation.
pub(super) fn clip_key_command(op: &str, request: &Value) -> Result<Option<Command>, CommandError> {
    let clip = || string(request, "clip").map(str::to_string);
    let command = match op {
        "clip.key" => ClipKeysCommand::Set {
            clip: clip()?,
            prop: prop_field(request)?,
            time_ms: integer(request, "time_ms")?,
            value: number(request, "value")?,
            easing: easing_field(request)?,
        },
        "clip.key.remove" => ClipKeysCommand::Remove {
            clip: clip()?,
            prop: prop_field(request)?,
            time_ms: integer(request, "time_ms")?,
        },
        "clip.key.capture" => ClipKeysCommand::Capture {
            clip: clip()?,
            prop: prop_field(request)?,
            easing: easing_field(request)?,
        },
        "clip.key.set" => ClipKeysCommand::Field {
            clip: clip()?,
            prop: prop_field(request)?,
            index: usize::try_from(integer(request, "index")?)
                .map_err(|_| invalid("`index` is a key number from 0"))?,
            field: string(request, "field")?.to_string(),
            value: request
                .get("value")
                .and_then(Raw::from_json)
                .ok_or_else(|| invalid("Expected scalar field `value`"))?,
        },
        "clip.keys.clear" => ClipKeysCommand::Clear {
            clip: clip()?,
            prop: match request.get("prop").filter(|raw| !raw.is_null()) {
                None => None,
                Some(_) => Some(prop_field(request)?),
            },
        },
        _ => return Ok(None),
    };
    Ok(Some(Command::ClipKeys(command)))
}
