//! The `player.*` and `subtitles.*` vocabulary of F071.
//!
//! Shape only: every rule (ranges, colours, the cue order, what a link may point at) belongs to
//! `player`, `subtitles` and `state::live_edit`, so a request means the same through the CLI, a
//! script, the socket and the library.

use super::*;
use crate::player::PlayerSpec;
use crate::state::live_edit::{LiveCommand, LiveSpec};
use crate::subtitles::{SubtitleSpec, read_srt_file};

fn value_field(request: &Value) -> Result<Raw, CommandError> {
    request
        .get("value")
        .and_then(Raw::from_json)
        .ok_or_else(|| invalid("Expected scalar field `value`"))
}

fn index_field(request: &Value) -> Result<usize, CommandError> {
    usize::try_from(integer(request, "index")?)
        .map_err(|_| invalid("`index` is a cue number from 0"))
}

/// The SRT text a `subtitles.import` carries: `srt` inline, or the file at `path`.
fn srt_text(request: &Value) -> Result<String, CommandError> {
    match (
        optional_string(request, "srt")?,
        optional_string(request, "path")?,
    ) {
        (Some(_), Some(_)) => Err(invalid("Use either `srt` (the text) or `path` (a file)")),
        (Some(text), None) => Ok(text),
        (None, Some(path)) => read_srt_file(&path),
        (None, None) => Err(invalid(
            "`subtitles.import` needs `srt` (the text) or `path` (a file)",
        )),
    }
}

/// The spec object of a `clip.add` for a player (`player`) or a caption track (`subtitles`).
pub(super) fn live_spec_field(request: &Value) -> Result<Option<LiveSpec>, CommandError> {
    let player = request.get("player").filter(|raw| !raw.is_null());
    let subtitles = request.get("subtitles").filter(|raw| !raw.is_null());
    let spec = match (player, subtitles) {
        (None, None) => return Ok(None),
        (Some(_), Some(_)) => return Err(invalid("Give `player` or `subtitles`, not both")),
        (Some(player), None) => LiveSpec::Player(PlayerSpec::from_json(player)?),
        (None, Some(subtitles)) => LiveSpec::Subtitles(SubtitleSpec::from_json(subtitles)?),
    };
    // Like a shape or a group, the clip is made of its spec alone.
    if let Some(field) = ["asset", "text", "camera", "shape", "group"]
        .into_iter()
        .find(|field| request.get(*field).is_some_and(|value| !value.is_null()))
    {
        return Err(invalid(format!(
            "A player or subtitle clip has no `{field}`; give its spec alone"
        )));
    }
    Ok(Some(spec))
}

/// `None` when `op` is not a player or subtitle operation.
pub(super) fn live_command(op: &str, request: &Value) -> Result<Option<Command>, CommandError> {
    let clip = || string(request, "clip").map(str::to_string);
    let field = || string(request, "field").map(str::to_string);
    let command = match op {
        "player.add" => return Ok(Some(Command::AddPlayer)),
        "subtitles.add" => return Ok(Some(Command::AddSubtitles)),
        "player.set" => LiveCommand::PlayerField {
            clip: clip()?,
            field: field()?,
            value: value_field(request)?,
        },
        "subtitles.set" => LiveCommand::SubtitleStyle {
            clip: clip()?,
            field: field()?,
            value: value_field(request)?,
        },
        "subtitles.import" => LiveCommand::SubtitleImport {
            clip: clip()?,
            srt: srt_text(request)?,
        },
        "subtitles.cue.add" => LiveCommand::SubtitleCueAdd {
            clip: clip()?,
            start_ms: integer(request, "start_ms")?,
            end_ms: integer(request, "end_ms")?,
            text: string(request, "text")?.to_string(),
        },
        "subtitles.cue.set" => LiveCommand::SubtitleCue {
            clip: clip()?,
            index: index_field(request)?,
            field: field()?,
            value: value_field(request)?,
        },
        "subtitles.cue.remove" => LiveCommand::SubtitleCueRemove {
            clip: clip()?,
            index: index_field(request)?,
        },
        _ => return Ok(None),
    };
    Ok(Some(Command::Live(command)))
}
