//! The `group.*`, `text_style.*` and group-clip vocabulary of F069.
//!
//! Shape only: every rule (IDs, ranges, limits, what is in use) belongs to `group` and
//! `state::group_edit`, so a request means the same through the CLI, a script, the socket and
//! the library.

use super::*;
use crate::easing::KeyEasing;
use crate::group::{GroupInstance, Prop, RepeatMode, TextStyle};
use crate::state::group_edit::GroupCommand;

/// `None` when the request does not say (omitting the easing is never a choice of `linear`).
/// A group key accepts the directed three plus `smoothStep`, so this shadows the camera keys'
/// narrower `easing_field` of the parent module.
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

fn value_field(request: &Value, key: &str) -> Result<Raw, CommandError> {
    request
        .get(key)
        .and_then(Raw::from_json)
        .ok_or_else(|| invalid(format!("Expected scalar field `{key}`")))
}

fn object_field(request: &Value, key: &str) -> Result<Value, CommandError> {
    request
        .get(key)
        .filter(|raw| raw.is_object())
        .cloned()
        .ok_or_else(|| invalid(format!("Expected object field `{key}`")))
}

fn prop_field(request: &Value) -> Result<Prop, CommandError> {
    Prop::parse(string(request, "prop")?)
}

fn number(request: &Value, key: &str) -> Result<f64, CommandError> {
    request
        .get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| invalid(format!("Expected number field `{key}`")))
}

/// The instance of a `clip.add` / `clip.group` request: `{def, texts?}`.
pub(super) fn instance_field(request: &Value) -> Result<GroupInstance, CommandError> {
    GroupInstance::from_json(
        request
            .get("group")
            .ok_or_else(|| invalid("Expected object field `group`"))?,
    )
}

/// `None` when `op` is not a group operation.
pub(super) fn group_command(op: &str, request: &Value) -> Result<Option<Command>, CommandError> {
    let group = || string(request, "group").map(str::to_string);
    let child = || string(request, "child").map(str::to_string);
    let command = match op {
        "group.add" => return Ok(Some(Command::AddGroup)),
        "chat.add" => return Ok(Some(Command::AddChat)),
        "group.define" => GroupCommand::Define {
            definition: object_field(request, "definition")?,
        },
        "group.remove" => GroupCommand::Remove { group: group()? },
        "group.set" => GroupCommand::Field {
            group: group()?,
            field: string(request, "field")?.to_string(),
            value: value_field(request, "value")?,
        },
        "group.child" => GroupCommand::ChildSet {
            group: group()?,
            child: object_field(request, "child")?,
        },
        "group.child.remove" => GroupCommand::ChildRemove {
            group: group()?,
            child: child()?,
        },
        "group.child.set" => GroupCommand::ChildField {
            group: group()?,
            child: child()?,
            field: string(request, "field")?.to_string(),
            value: value_field(request, "value")?,
        },
        "group.key" => GroupCommand::Key {
            group: group()?,
            child: child()?,
            prop: prop_field(request)?,
            time_ms: integer(request, "time_ms")?,
            value: number(request, "value")?,
            easing: easing_field(request)?,
        },
        "group.key.remove" => GroupCommand::KeyRemove {
            group: group()?,
            child: child()?,
            prop: prop_field(request)?,
            time_ms: integer(request, "time_ms")?,
        },
        "group.key.set" => GroupCommand::KeyField {
            group: group()?,
            child: child()?,
            prop: prop_field(request)?,
            index: usize::try_from(integer(request, "index")?)
                .map_err(|_| invalid("`index` is a key number from 0"))?,
            field: string(request, "field")?.to_string(),
            value: value_field(request, "value")?,
        },
        "group.repeat" => GroupCommand::Repeat {
            group: group()?,
            child: child()?,
            prop: prop_field(request)?,
            repeat: optional_u32(request, "repeat")?,
            mode: optional_string(request, "mode")?
                .map(|label| RepeatMode::parse(&label))
                .transpose()?,
        },
        "group.track.remove" => GroupCommand::TrackRemove {
            group: group()?,
            child: child()?,
            prop: prop_field(request)?,
        },
        "text_style.set" => GroupCommand::StyleSet {
            style: TextStyle::from_json(&object_field(request, "style")?)?,
        },
        "text_style.field" => GroupCommand::StyleField {
            style: string(request, "style")?.to_string(),
            field: string(request, "field")?.to_string(),
            value: value_field(request, "value")?,
        },
        "text_style.remove" => GroupCommand::StyleRemove {
            style: string(request, "style")?.to_string(),
        },
        "clip.group" => GroupCommand::Use {
            clip: string(request, "clip")?.to_string(),
            group: group()?,
        },
        "clip.group_text" => GroupCommand::Text {
            clip: string(request, "clip")?.to_string(),
            child: child()?,
            // `null` (or no `text`) clears the override: the clip shows the group's own text.
            text: optional_string(request, "text")?,
        },
        _ => return Ok(None),
    };
    Ok(Some(Command::Group(command)))
}
