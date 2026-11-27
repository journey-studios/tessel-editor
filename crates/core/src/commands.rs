//! Translation of semantic requests into editor [`Command`]s, without GPUI.
//!
//! Desktop automation may additionally validate against its live component tree.
//! Headless callers do not depend on control visibility: domain validation belongs
//! to `EditorState::execute`. Both adapters use the same command vocabulary.

use serde_json::{Value, json};

use crate::perf::{self, PerfMeterCommand};
use crate::perf_history::{self, HistoryCommand};
use crate::plan::PreviewIntent;
use crate::preview::PreviewMode;
use crate::scene_interaction::{GizmoCommand, GizmoSpace, GizmoTool};
use crate::scene_project::SceneCommand;
use crate::state::camera_edit::{CameraKeysCommand, KEY_FIELDS};
use crate::state::clip_key_edit::ClipKeysCommand;
use crate::state::inspector::{InspectorTab, PropertyKind, property_def};
use crate::state::layout::{Destination, GroupRef, Layout, LayoutCommand, PanelKind, Region, Side};
use crate::state::split::SplitAvailability;
use crate::state::timeline::{Placement, TrackFlag};
use crate::state::{Command, CommandError, EditorState, Raw, WorkspaceTab};
use crate::time::Ratio;

/// What the server should do for a parsed request.
// WHY allowed: a `Parsed` lives for one request on the stack; boxing `Command` would force
// every adapter and test to unwrap it for no measurable gain.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, PartialEq)]
pub enum Parsed {
    Run(Command),
    /// A change to the dock; the server validates it against the live area.
    Layout(LayoutCommand),
    ToggleTheme,
    /// Show, hide or toggle the debug performance meter (F049). Window state, like the theme.
    PerfMeter(PerfMeterCommand),
    /// The rolling performance history (BL145.2): record, show, freeze, select, clear, export.
    /// Window state: it measures the live window and monitor.
    PerfHistory(HistoryCommand),
    /// Reload the viewport page. A window concern: headless sessions have no page.
    ReloadViewport,
}

fn missing(what: &str) -> CommandError {
    CommandError::new("missing_argument", format!("Missing {what}"))
}

fn choice<T>(
    raw: &Raw,
    parse: impl Fn(&str) -> Option<T>,
    options: &[&str],
) -> Result<T, CommandError> {
    let text = raw.text();
    parse(&text).ok_or_else(|| {
        CommandError::invalid_value(
            format!("`{text}` is not an option"),
            json!({ "options": options }),
        )
    })
}

fn split<'a>(target: &'a str, prefix: &str) -> Option<&'a str> {
    target.strip_prefix(prefix).filter(|rest| !rest.is_empty())
}

pub fn parse_set(target: &str, field: &str, value: &Raw) -> Result<Parsed, CommandError> {
    let command = match (target, field) {
        ("media.search", "value") => Command::SetMediaSearch {
            query: value.text(),
        },
        ("workspace.tabs", "value") => Command::SetWorkspaceTab {
            tab: choice(
                value,
                WorkspaceTab::parse,
                &WorkspaceTab::ALL.map(WorkspaceTab::id),
            )?,
        },
        ("inspector.tabs", "value") => Command::SetInspectorTab {
            tab: choice(
                value,
                InspectorTab::parse,
                &InspectorTab::ALL.map(InspectorTab::id),
            )?,
        },
        // The visual sequence selector and `sequence.activate` are one rule: an unknown id is an
        // `unknown_element` error naming the sequence, from the command itself.
        ("timeline.sequence", "value") => Command::ActivateSequence {
            sequence: value.text(),
        },
        ("timeline.language", "value") => {
            Command::Localization(crate::localization::Edit::Select(value.text()))
        }
        ("timeline.zoom", "value") => Command::SetTimelineZoom {
            zoom: value.clone(),
        },
        ("timeline.snap", "value") => Command::SetSnap {
            snap: value.boolean()?,
        },
        ("timeline.playhead", "frame") => Command::SetPlayhead {
            frame: value.clone(),
        },
        // BL015.1: the gizmo's tool and rotate space are `monitor.gizmo` fields; setting one is the
        // `scene.gizmo.tool` command (a read-only node would be a control that does nothing).
        ("monitor.gizmo", "tool") => Command::Scene(SceneCommand::Gizmo(GizmoCommand::Tool {
            tool: Some(choice(
                value,
                GizmoTool::parse,
                &GizmoTool::ALL.map(GizmoTool::as_str),
            )?),
            space: None,
        })),
        ("monitor.gizmo", "space") => Command::Scene(SceneCommand::Gizmo(GizmoCommand::Tool {
            tool: None,
            space: Some(choice(
                value,
                GizmoSpace::parse,
                &GizmoSpace::ALL.map(GizmoSpace::as_str),
            )?),
        })),
        ("dock", "layout") => {
            return Ok(Parsed::Layout(LayoutCommand::Load { json: value.text() }));
        }
        (perf::TARGET, "visible") => {
            return Ok(Parsed::PerfMeter(if value.boolean()? {
                PerfMeterCommand::Show
            } else {
                PerfMeterCommand::Hide
            }));
        }
        (perf_history::TARGET, field) => return parse_set_history(field, value),
        _ => return parse_set_entity(target, field, value),
    };
    Ok(Parsed::Run(command))
}

/// `set perf.history <field> <value>`: the same commands as the panel's controls.
fn parse_set_history(field: &str, value: &Raw) -> Result<Parsed, CommandError> {
    let command = match field {
        "recording" => HistoryCommand::Record(value.boolean()?),
        "visible" => {
            if value.boolean()? {
                HistoryCommand::Show
            } else {
                HistoryCommand::Hide
            }
        }
        "frozen" => {
            if value.boolean()? {
                HistoryCommand::Freeze
            } else {
                HistoryCommand::Resume
            }
        }
        // Milliseconds on the history's monotonic clock (`get` publishes the window in them).
        "selected_ms" => {
            let milliseconds = value.number()?;
            if milliseconds < 0.0 {
                return Err(CommandError::invalid_value(
                    "Expected milliseconds since the monotonic origin (>= 0); `clear_selection` clears",
                    json!("milliseconds"),
                ));
            }
            // A finite but absurd value (`1e300`) overflows a `Duration`: refuse it, never panic.
            let at =
                std::time::Duration::try_from_secs_f64(milliseconds / 1000.0).map_err(|_| {
                    CommandError::invalid_value(
                        "Milliseconds are too large to be an instant on the monotonic clock",
                        json!("milliseconds"),
                    )
                })?;
            HistoryCommand::Select(Some(at))
        }
        "export_to" => HistoryCommand::Export(std::path::PathBuf::from(value.text())),
        // What typing (or picking) a path in the panel's field does; `invoke ... export` is the
        // button that writes to it.
        "export_destination" => HistoryCommand::Destination(value.text()),
        _ => {
            return Err(CommandError::new(
                "unknown_field",
                format!("`{}` has no settable field `{field}`", perf_history::TARGET),
            )
            .target(perf_history::TARGET)
            .details(
                json!({ "fields": ["recording", "visible", "frozen", "selected_ms", "export_to", "export_destination"] }),
            ));
        }
    };
    Ok(Parsed::PerfHistory(command))
}

fn parse_invoke_history(action: &str) -> Result<Parsed, CommandError> {
    let command = match action {
        "record_start" => HistoryCommand::Record(true),
        "record_stop" => HistoryCommand::Record(false),
        "show" => HistoryCommand::Show,
        "hide" => HistoryCommand::Hide,
        "freeze" => HistoryCommand::Freeze,
        "resume" => HistoryCommand::Resume,
        "clear" => HistoryCommand::Clear,
        "snapshot" => HistoryCommand::Snapshot,
        "clear_selection" => HistoryCommand::Select(None),
        "export" => HistoryCommand::ExportToDestination,
        _ => {
            return Err(CommandError::new(
                "unknown_action",
                format!("`{}` has no action `{action}`", perf_history::TARGET),
            )
            .target(perf_history::TARGET)
            .details(json!({ "actions": perf_history::ACTIONS })));
        }
    };
    Ok(Parsed::PerfHistory(command))
}

fn parse_panel(id: &str, target: &str) -> Result<PanelKind, CommandError> {
    PanelKind::parse(id).ok_or_else(|| {
        CommandError::new("unknown_element", format!("No panel `{id}`"))
            .target(target)
            .details(json!({ "available": PanelKind::ALL.map(PanelKind::id) }))
    })
}

fn parse_region(id: &str, target: &str) -> Result<Region, CommandError> {
    Region::parse(id).ok_or_else(|| {
        CommandError::new("unknown_element", format!("No dock `{id}`"))
            .target(target)
            .details(json!({ "available": Region::ALL.map(Region::id) }))
    })
}

fn parse_numeric_id(id: &str, prefix: &str, target: &str) -> Result<u64, CommandError> {
    id.parse().map_err(|_| {
        CommandError::new(
            "unknown_element",
            format!("`{target}` is not a {prefix} ID"),
        )
        .target(target)
    })
}

/// `dock:<region>`, `split:<n>` and `panel:<id>` / `region:<id>` fields.
fn parse_set_dock(target: &str, field: &str, value: &Raw) -> Result<Parsed, CommandError> {
    let command = if let Some(id) = split(target, "panel:").or_else(|| split(target, "region:")) {
        match field {
            "visible" => LayoutCommand::SetVisible {
                panel: parse_panel(id, target)?,
                visible: value.boolean()?,
            },
            _ => return Err(unknown_field(target, field)),
        }
    } else if let Some(id) = split(target, "dock:") {
        let region = parse_region(id, target)?;
        match field {
            "size" => LayoutCommand::ResizeRegion {
                region,
                size: value.number()? as f32,
            },
            "open" => LayoutCommand::SetRegionOpen {
                region,
                open: value.boolean()?,
            },
            _ => return Err(unknown_field(target, field)),
        }
    } else if let Some(id) = split(target, "split:") {
        let split_id = parse_numeric_id(id, "split", target)?;
        match field {
            "sizes" => {
                let sizes = value
                    .text()
                    .split(',')
                    .map(|part| {
                        part.trim().parse::<f32>().map_err(|_| {
                            CommandError::invalid_value(
                                format!("`{}` is not a number", part.trim()),
                                json!({ "format": "comma separated pixel sizes, e.g. 300,500" }),
                            )
                            .target(target)
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                LayoutCommand::ResizeSplit {
                    split: split_id,
                    sizes,
                }
            }
            _ => return Err(unknown_field(target, field)),
        }
    } else {
        return Err(unknown_field(target, field));
    };
    Ok(Parsed::Layout(command))
}

fn parse_set_entity(target: &str, field: &str, value: &Raw) -> Result<Parsed, CommandError> {
    let command = if let Some(id) = split(target, "trim:") {
        let (clip, side) = id
            .rsplit_once(':')
            .ok_or_else(|| unknown_field(target, field))?;
        let edge = match side {
            "start" => crate::state::timeline::TrimEdge::Start,
            "end" => crate::state::timeline::TrimEdge::End,
            _ => return Err(unknown_field(target, field)),
        };
        if field != "frame" {
            return Err(unknown_field(target, field));
        }
        Command::TrimClip {
            clip: clip.to_string(),
            edge,
            frame: value.integer()?,
        }
    } else if let Some(id) = split(target, "clip_text:") {
        if !crate::content::TextSpec::FIELDS.contains(&field) {
            return Err(unknown_field(target, field));
        }
        Command::SetClipTextField {
            clip: id.to_string(),
            field: field.to_string(),
            value: value.clone(),
        }
    } else if let Some(id) = split(target, "clip_shape:") {
        if !crate::shape::ShapeSpec::FIELDS.contains(&field) {
            return Err(unknown_field(target, field));
        }
        Command::SetClipShapeField {
            clip: id.to_string(),
            field: field.to_string(),
            value: value.clone(),
        }
    } else if let Some(id) = split(target, "camera_key:") {
        let (clip, index) = parse_camera_key_ref(id, target)?;
        if !KEY_FIELDS.contains(&field) {
            return Err(unknown_field(target, field));
        }
        Command::CameraKeys(CameraKeysCommand::Field {
            clip,
            index,
            field: field.to_string(),
            value: value.clone(),
        })
    } else if let Some(id) = split(target, "clip_key:") {
        let (clip, prop, index) = parse_clip_key_ref(id, target)?;
        if !crate::group::Key::FIELDS.contains(&field) {
            return Err(unknown_field(target, field));
        }
        Command::ClipKeys(ClipKeysCommand::Field {
            clip,
            prop,
            index,
            field: field.to_string(),
            value: value.clone(),
        })
    } else if let Some(command) = parse_set_group(target, field, value)? {
        command
    } else if let Some(command) = parse_set_live(target, field, value)? {
        command
    } else if let Some(id) = split(target, "folder:") {
        match field {
            "expanded" => Command::SetFolderExpanded {
                folder: id.to_string(),
                expanded: value.boolean()?,
            },
            _ => return Err(unknown_field(target, field)),
        }
    } else if let Some(id) = split(target, "property:") {
        match field {
            "value" => Command::SetProperty {
                property: id.to_string(),
                value: value.clone(),
            },
            _ => return Err(unknown_field(target, field)),
        }
    } else if let Some(id) = split(target, "inspector.") {
        match field {
            "expanded" => Command::SetSectionExpanded {
                section: id.to_string(),
                expanded: value.boolean()?,
            },
            _ => return Err(unknown_field(target, field)),
        }
    } else if let Some(id) = split(target, "track:") {
        let flag = match field {
            "hidden" => TrackFlag::Hidden,
            "muted" => TrackFlag::Muted,
            _ => return Err(unknown_field(target, field)),
        };
        Command::SetTrackFlag {
            track: id.to_string(),
            flag,
            value: value.boolean()?,
        }
    } else {
        return parse_set_dock(target, field, value);
    };
    Ok(Parsed::Run(command))
}

/// The `/`-separated parts of a group target (`group_key:<group>/<child>/<prop>/<index>`). IDs
/// never hold a `/`, so the split is unambiguous.
fn group_parts<'a>(
    id: &'a str,
    target: &str,
    shape: &str,
    count: usize,
) -> Result<Vec<&'a str>, CommandError> {
    let parts: Vec<&str> = id.split('/').collect();
    if parts.len() != count || parts.iter().any(|part| part.is_empty()) {
        return Err(
            CommandError::new("unknown_element", format!("`{target}` is not `{shape}`"))
                .target(target),
        );
    }
    Ok(parts)
}

/// `set` on the targets of F069: groups, their children, keys, tracks, text styles and the text
/// a group clip shows. `None` when `target` is not one of them.
fn parse_set_group(
    target: &str,
    field: &str,
    value: &Raw,
) -> Result<Option<Command>, CommandError> {
    use crate::group::{GroupDef, Prop, RepeatMode, TextStyle};
    use crate::state::group_edit::GroupCommand;
    let command = if let Some(id) = split(target, "clip_group:") {
        if field != "group" {
            return Err(unknown_field(target, field));
        }
        GroupCommand::Use {
            clip: id.to_string(),
            group: value.text(),
        }
    } else if let Some(id) = split(target, "group_def:") {
        if !GroupDef::FIELDS.contains(&field) {
            return Err(unknown_field(target, field));
        }
        GroupCommand::Field {
            group: id.to_string(),
            field: field.to_string(),
            value: value.clone(),
        }
    } else if let Some(id) = split(target, "group_child:") {
        let parts = group_parts(id, target, "group_child:<group>/<child>", 2)?;
        // The child's own fields depend on what it draws; the state names the valid ones when
        // the field is not among them.
        GroupCommand::ChildField {
            group: parts[0].to_string(),
            child: parts[1].to_string(),
            field: field.to_string(),
            value: value.clone(),
        }
    } else if let Some(id) = split(target, "group_key:") {
        let parts = group_parts(id, target, "group_key:<group>/<child>/<prop>/<index>", 4)?;
        GroupCommand::KeyField {
            group: parts[0].to_string(),
            child: parts[1].to_string(),
            prop: Prop::parse(parts[2])?,
            index: parts[3].parse::<usize>().map_err(|_| {
                CommandError::new(
                    "unknown_element",
                    format!("`{target}` has no key number `{}`", parts[3]),
                )
                .target(target)
            })?,
            field: field.to_string(),
            value: value.clone(),
        }
    } else if let Some(id) = split(target, "group_track:") {
        let parts = group_parts(id, target, "group_track:<group>/<child>/<prop>", 3)?;
        let (repeat, mode) = match field {
            "repeat" => (
                Some(u32::try_from(value.integer()?).map_err(|_| {
                    CommandError::invalid_value(
                        "`repeat` is a whole number from 1",
                        json!({ "min": 1, "max": crate::group::MAX_REPEAT }),
                    )
                })?),
                None,
            ),
            "mode" => (None, Some(RepeatMode::parse(&value.text())?)),
            _ => return Err(unknown_field(target, field)),
        };
        GroupCommand::Repeat {
            group: parts[0].to_string(),
            child: parts[1].to_string(),
            prop: Prop::parse(parts[2])?,
            repeat,
            mode,
        }
    } else if let Some(id) = split(target, "text_style:") {
        if !TextStyle::FIELDS.contains(&field) {
            return Err(unknown_field(target, field));
        }
        GroupCommand::StyleField {
            style: id.to_string(),
            field: field.to_string(),
            value: value.clone(),
        }
    } else if let Some(id) = split(target, "group_text:") {
        let parts = group_parts(id, target, "group_text:<clip>/<child>", 2)?;
        if field != "text" {
            return Err(unknown_field(target, field));
        }
        GroupCommand::Text {
            clip: parts[0].to_string(),
            child: parts[1].to_string(),
            text: Some(value.text()),
        }
    } else {
        return Ok(None);
    };
    Ok(Some(Command::Group(command)))
}

/// `set` on the targets of F071: a player's fields, a caption track's style and one of its cues.
/// `None` when `target` is not one of them.
fn parse_set_live(target: &str, field: &str, value: &Raw) -> Result<Option<Command>, CommandError> {
    use crate::player::PlayerSpec;
    use crate::state::live_edit::LiveCommand;
    use crate::subtitles::SubtitleSpec;
    let command = if let Some(id) = split(target, "clip_player:") {
        if !PlayerSpec::FIELDS.contains(&field) {
            return Err(unknown_field(target, field));
        }
        LiveCommand::PlayerField {
            clip: id.to_string(),
            field: field.to_string(),
            value: value.clone(),
        }
    } else if let Some(id) = split(target, "clip_subtitles:") {
        if !SubtitleSpec::STYLE_FIELDS.contains(&field) {
            return Err(unknown_field(target, field));
        }
        LiveCommand::SubtitleStyle {
            clip: id.to_string(),
            field: field.to_string(),
            value: value.clone(),
        }
    } else if let Some(id) = split(target, "subtitle_cue:") {
        let (clip, index) = parse_cue_ref(id, target)?;
        if !SubtitleSpec::CUE_FIELDS.contains(&field) {
            return Err(unknown_field(target, field));
        }
        LiveCommand::SubtitleCue {
            clip,
            index,
            field: field.to_string(),
            value: value.clone(),
        }
    } else {
        return Ok(None);
    };
    Ok(Some(Command::Live(command)))
}

/// `<clip>/<index>` of a `subtitle_cue:` target (clip IDs never hold a `/`).
fn parse_cue_ref(id: &str, target: &str) -> Result<(String, usize), CommandError> {
    id.split_once('/')
        .and_then(|(clip, index)| Some((clip.to_string(), index.parse::<usize>().ok()?)))
        .filter(|(clip, _)| !clip.is_empty())
        .ok_or_else(|| {
            CommandError::new(
                "unknown_element",
                format!("`{target}` is not `subtitle_cue:<clip>/<index>`"),
            )
            .target(target)
        })
}

/// `invoke` on the targets of F071: removing a cue and adding one at the playhead. `None` when
/// `target` is not one of them.
fn parse_invoke_live(
    state: &EditorState,
    target: &str,
    action: &str,
) -> Result<Option<Command>, CommandError> {
    use crate::state::live_edit::LiveCommand;
    let unknown = || {
        CommandError::new(
            "unknown_action",
            format!("`{target}` has no action `{action}`"),
        )
        .target(target)
    };
    let command = if let Some(id) = split(target, "subtitle_cue:") {
        let (clip, index) = parse_cue_ref(id, target)?;
        match action {
            "remove" => LiveCommand::SubtitleCueRemove { clip, index },
            _ => return Err(unknown()),
        }
    } else if let Some(id) = split(target, "clip_subtitles:") {
        match action {
            "add_cue" => {
                let (start_ms, end_ms) = state.cue_slot_at_playhead(id)?;
                LiveCommand::SubtitleCueAdd {
                    clip: id.to_string(),
                    start_ms,
                    end_ms,
                    text: crate::state::live_edit::NEW_CUE_TEXT.to_string(),
                }
            }
            _ => return Err(unknown()),
        }
    } else {
        return Ok(None);
    };
    Ok(Some(Command::Live(command)))
}

/// `invoke` on the targets of F069 (`remove`, and `reset` of a text override). `None` when
/// `target` is not one of them.
fn parse_invoke_group(
    state: &EditorState,
    target: &str,
    action: &str,
) -> Result<Option<Command>, CommandError> {
    use crate::group::Prop;
    use crate::state::group_edit::GroupCommand;
    let unknown = || {
        CommandError::new(
            "unknown_action",
            format!("`{target}` has no action `{action}`"),
        )
        .target(target)
    };
    let command = if let Some(id) = split(target, "group_def:") {
        match action {
            "remove" => GroupCommand::Remove {
                group: id.to_string(),
            },
            _ => return Err(unknown()),
        }
    } else if let Some(id) = split(target, "group_child:") {
        let parts = group_parts(id, target, "group_child:<group>/<child>", 2)?;
        match action {
            "remove" => GroupCommand::ChildRemove {
                group: parts[0].to_string(),
                child: parts[1].to_string(),
            },
            _ => return Err(unknown()),
        }
    } else if let Some(id) = split(target, "group_track:") {
        let parts = group_parts(id, target, "group_track:<group>/<child>/<prop>", 3)?;
        match action {
            "remove" => GroupCommand::TrackRemove {
                group: parts[0].to_string(),
                child: parts[1].to_string(),
                prop: Prop::parse(parts[2])?,
            },
            _ => return Err(unknown()),
        }
    } else if let Some(id) = split(target, "group_key:") {
        let parts = group_parts(id, target, "group_key:<group>/<child>/<prop>/<index>", 4)?;
        let prop = Prop::parse(parts[2])?;
        let key = parts[3].parse::<usize>().ok().and_then(|index| {
            state
                .groups
                .def(parts[0])?
                .child(parts[1])?
                .track(prop)?
                .keys
                .get(index)
        });
        match (action, key) {
            ("remove", Some(key)) => GroupCommand::KeyRemove {
                group: parts[0].to_string(),
                child: parts[1].to_string(),
                prop,
                time_ms: key.time_ms,
            },
            ("remove", None) => {
                return Err(CommandError::new(
                    "unknown_element",
                    format!("`{target}` is not a key of the project"),
                )
                .target(target));
            }
            _ => return Err(unknown()),
        }
    } else if let Some(id) = split(target, "text_style:") {
        match action {
            "remove" => GroupCommand::StyleRemove {
                style: id.to_string(),
            },
            _ => return Err(unknown()),
        }
    } else if let Some(id) = split(target, "group_text:") {
        let parts = group_parts(id, target, "group_text:<clip>/<child>", 2)?;
        match action {
            "reset" => GroupCommand::Text {
                clip: parts[0].to_string(),
                child: parts[1].to_string(),
                text: None,
            },
            _ => return Err(unknown()),
        }
    } else {
        return Ok(None);
    };
    Ok(Some(Command::Group(command)))
}

/// `<clip>:<index>` of a `camera_key:` target.
fn parse_camera_key_ref(id: &str, target: &str) -> Result<(String, usize), CommandError> {
    id.rsplit_once(':')
        .and_then(|(clip, index)| Some((clip.to_string(), index.parse::<usize>().ok()?)))
        .filter(|(clip, _)| !clip.is_empty())
        .ok_or_else(|| {
            CommandError::new(
                "unknown_element",
                format!("`{target}` is not `camera_key:<clip>:<index>`"),
            )
            .target(target)
        })
}

/// `<clip>:<prop>:<index>` of a `clip_key:` target (clip IDs hold no `:`).
fn parse_clip_key_ref(
    id: &str,
    target: &str,
) -> Result<(String, crate::group::Prop, usize), CommandError> {
    let bad = || {
        CommandError::new(
            "unknown_element",
            format!("`{target}` is not `clip_key:<clip>:<property>:<index>`"),
        )
        .target(target)
    };
    let mut parts = id.rsplitn(3, ':');
    let index = parts
        .next()
        .and_then(|index| index.parse::<usize>().ok())
        .ok_or_else(bad)?;
    let property = parts.next().ok_or_else(bad)?;
    let clip = parts
        .next()
        .filter(|clip| !clip.is_empty())
        .ok_or_else(bad)?;
    let prop = crate::clip_keys::parse_property(property).map_err(|_| bad())?;
    Ok((clip.to_string(), prop, index))
}

/// The selected clip, which must be one a key can animate (the targets of `clip_keys.*`).
fn selected_keyed_clip(state: &EditorState, target: &str) -> Result<String, CommandError> {
    state
        .selected_clip()
        .filter(|clip| crate::state::inspector::Applies::Framed.includes(clip.kind))
        .map(|clip| clip.id.to_string())
        .ok_or_else(|| {
            CommandError::new(
                "no_selection",
                "Select a video, image, title, shape or scene clip in the timeline first",
            )
            .target(target)
        })
}

fn unknown_field(target: &str, field: &str) -> CommandError {
    CommandError::new(
        "unknown_field",
        format!("`{target}` has no settable field `{field}`"),
    )
    .target(target)
}

fn seek(frame: i64) -> Command {
    Command::SetPlayhead {
        frame: Raw::Number(frame as f64),
    }
}

pub fn parse_invoke(
    state: &EditorState,
    layout: &Layout,
    target: &str,
    action: &str,
) -> Result<Parsed, CommandError> {
    if let Some(parsed) = parse_invoke_dock(layout, target, action)? {
        return Ok(parsed);
    }
    let command = match (target, action) {
        ("toolbar.theme", "toggle") => return Ok(Parsed::ToggleTheme),
        (perf::TARGET, "toggle") => return Ok(Parsed::PerfMeter(PerfMeterCommand::Toggle)),
        (perf_history::TARGET, action) => return parse_invoke_history(action),
        ("timeline.language.approve", "invoke") => {
            if !state.is_localized_view() {
                return Err(CommandError::disabled("Select a translation to approve"));
            }
            Command::Localization(crate::localization::Edit::Approve(
                state.active_language().unwrap_or_default().to_string(),
            ))
        }
        ("timeline.snap", "toggle") => Command::SetSnap {
            snap: !state.timeline.snap,
        },
        ("folder:all", "select") => Command::SelectFolder { folder: None },
        ("timeline.add_title", "invoke") => Command::AddTitle,
        ("timeline.add_shape", "invoke") => Command::AddShape,
        ("timeline.add_group", "invoke") => Command::AddGroup,
        ("timeline.add_chat", "invoke") => Command::AddChat,
        ("timeline.add_player", "invoke") => Command::AddPlayer,
        ("timeline.add_subtitles", "invoke") => Command::AddSubtitles,
        // F017: the selected clip at the playhead. The refusal is the node's `disabled_reason`,
        // so a client that skips the tree (the headless `invoke`) gets the same answer.
        ("timeline.split", "invoke") => match state.split_availability() {
            SplitAvailability::Ready { clip, frame } => Command::SplitClip {
                clip: clip.to_string(),
                frame,
                new_id: None,
            },
            SplitAvailability::Blocked(block) => {
                return Err(CommandError::disabled(block.reason()).target(target));
            }
        },
        ("monitor.go_start", "invoke") => seek(0),
        ("monitor.step_back", "invoke") => seek(state.timeline.playhead - 1),
        ("monitor.step_forward", "invoke") => seek(state.timeline.playhead + 1),
        ("monitor.go_end", "invoke") => seek(state.timeline.length),
        ("monitor.preview.quality", "invoke") => Command::SetPreviewPolicy {
            mode: Some(match state.preview_policy.mode {
                PreviewMode::Original => PreviewMode::Auto,
                PreviewMode::Auto => PreviewMode::Original,
            }),
            max_height: None,
            format: None,
        },
        ("monitor.preview.sequence", "invoke") => Command::SetPreview {
            intent: PreviewIntent::Sequence,
        },
        ("monitor.preview.source", "invoke") => Command::SetPreview {
            intent: PreviewIntent::Source {
                asset: state
                    .previewable_media()
                    .map(|item| item.id)
                    .ok_or_else(|| {
                        CommandError::new(
                            "no_selection",
                            "Select a media item with a file to preview its source",
                        )
                        .target(target)
                    })?,
                time: Ratio::ZERO,
            },
        },
        ("monitor.reload", "invoke") => return Ok(Parsed::ReloadViewport),
        ("camera.push_in", "invoke") => Command::SetCameraPushIn {
            clip: selected_scene_clip(state, target)?,
            options: Some(Default::default()),
        },
        ("camera.push_in.clear", "invoke") => Command::SetCameraPushIn {
            clip: selected_scene_clip(state, target)?,
            options: None,
        },
        ("camera_keys.capture", "invoke") => Command::CameraKeys(CameraKeysCommand::Capture {
            clip: selected_scene_clip(state, target)?,
            easing: None,
        }),
        ("camera.capture", "invoke") => Command::SetClipCameraFromCapture {
            clip: selected_scene_clip(state, target)?,
        },
        ("camera_keys.clear", "invoke") => Command::CameraKeys(CameraKeysCommand::Replace {
            clip: selected_scene_clip(state, target)?,
            keys: Vec::new(),
        }),
        _ => return parse_invoke_entity(state, target, action),
    };
    Ok(Parsed::Run(command))
}

fn selected_scene_clip(state: &EditorState, target: &str) -> Result<String, CommandError> {
    state
        .selected_clip()
        .filter(|clip| clip.kind == crate::state::timeline::ClipKind::Scene)
        .map(|clip| clip.id.to_string())
        .ok_or_else(|| {
            CommandError::new("no_selection", "Select a scene clip in the timeline first")
                .target(target)
        })
}

/// Dock invocations, or `None` when `target` is not a dock element.
fn parse_invoke_dock(
    layout: &Layout,
    target: &str,
    action: &str,
) -> Result<Option<Parsed>, CommandError> {
    let unknown = || {
        CommandError::new(
            "unknown_action",
            format!("`{target}` has no action `{action}`"),
        )
        .target(target)
    };
    let command = match (target, action) {
        ("dock" | "toolbar.layout.reset", "reset" | "invoke") => LayoutCommand::Reset,
        _ => {
            if let Some(id) = split(target, "toolbar.panel.") {
                let panel = parse_panel(id, target)?;
                if action != "toggle" {
                    return Err(unknown());
                }
                LayoutCommand::SetVisible {
                    panel,
                    visible: !layout.is_visible(panel),
                }
            } else if let Some(id) = split(target, "panel:") {
                let panel = parse_panel(id, target)?;
                match action {
                    "hide" => LayoutCommand::SetVisible {
                        panel,
                        visible: false,
                    },
                    "show" => LayoutCommand::SetVisible {
                        panel,
                        visible: true,
                    },
                    "activate" => LayoutCommand::Activate { panel },
                    _ => return Err(unknown()),
                }
            } else if let Some(id) = split(target, "dock:") {
                let region = parse_region(id, target)?;
                let open = layout.region(region).is_some_and(|state| state.open);
                match action {
                    "toggle" => LayoutCommand::SetRegionOpen {
                        region,
                        open: !open,
                    },
                    _ => return Err(unknown()),
                }
            } else {
                return Ok(None);
            }
        }
    };
    Ok(Some(Parsed::Layout(command)))
}

fn parse_invoke_entity(
    state: &EditorState,
    target: &str,
    action: &str,
) -> Result<Parsed, CommandError> {
    let unknown = || {
        CommandError::new(
            "unknown_action",
            format!("`{target}` has no action `{action}`"),
        )
        .target(target)
    };
    let command = if let Some(id) = split(target, "folder:") {
        match action {
            "select" => Command::SelectFolder {
                folder: Some(id.to_string()),
            },
            "toggle" => Command::SetFolderExpanded {
                folder: id.to_string(),
                expanded: !state
                    .library
                    .folder(id)
                    .is_some_and(|folder| folder.expanded),
            },
            _ => return Err(unknown()),
        }
    } else if let Some(id) = split(target, "media:") {
        match action {
            "select" => Command::SelectMedia {
                media: id.to_string(),
            },
            _ => return Err(unknown()),
        }
    } else if let Some(id) = split(target, "clip:") {
        match action {
            "select" => Command::SelectClip {
                clip: id.to_string(),
            },
            _ => return Err(unknown()),
        }
    } else if let Some(id) = split(target, "track:") {
        let track = id.to_string();
        let current = state.timeline.track(id);
        match action {
            "select" => Command::SelectTrack { track },
            "toggle_hidden" => Command::SetTrackFlag {
                track,
                flag: TrackFlag::Hidden,
                value: !current.is_some_and(|track| track.hidden),
            },
            "toggle_muted" => Command::SetTrackFlag {
                track,
                flag: TrackFlag::Muted,
                value: !current.is_some_and(|track| track.muted),
            },
            _ => return Err(unknown()),
        }
    } else if let Some(id) = split(target, "camera_key:") {
        let (clip, index) = parse_camera_key_ref(id, target)?;
        let key = state
            .timeline
            .clip(clip.as_str())
            .and_then(|entry| entry.camera_keys.get(index));
        match (action, key) {
            ("remove", Some(key)) => Command::CameraKeys(CameraKeysCommand::Remove {
                clip,
                time: key.time,
            }),
            ("remove", None) => {
                return Err(CommandError::new(
                    "unknown_element",
                    format!("`{target}` is not a camera key of the project"),
                )
                .target(target));
            }
            _ => return Err(unknown()),
        }
    } else if let Some(id) = split(target, "clip_key:") {
        let (clip, prop, index) = parse_clip_key_ref(id, target)?;
        let key = state
            .timeline
            .clip(clip.as_str())
            .and_then(|entry| entry.keys.iter().find(|track| track.prop == prop))
            .and_then(|track| track.keys.get(index));
        match (action, key) {
            ("remove", Some(key)) => Command::ClipKeys(ClipKeysCommand::Remove {
                clip,
                prop,
                time_ms: key.time_ms,
            }),
            ("remove", None) => {
                return Err(CommandError::new(
                    "unknown_element",
                    format!("`{target}` is not a key of the project"),
                )
                .target(target));
            }
            _ => return Err(unknown()),
        }
    } else if let Some(property) = split(target, "clip_keys.capture:") {
        match action {
            "invoke" => Command::ClipKeys(ClipKeysCommand::Capture {
                clip: selected_keyed_clip(state, target)?,
                prop: crate::clip_keys::parse_property(property)?,
                easing: None,
            }),
            _ => return Err(unknown()),
        }
    } else if let Some(property) = split(target, "clip_keys.clear:") {
        match action {
            "invoke" => Command::ClipKeys(ClipKeysCommand::Clear {
                clip: selected_keyed_clip(state, target)?,
                prop: Some(crate::clip_keys::parse_property(property)?),
            }),
            _ => return Err(unknown()),
        }
    } else if target == "clip_keys.clear" {
        match action {
            "invoke" => Command::ClipKeys(ClipKeysCommand::Clear {
                clip: selected_keyed_clip(state, target)?,
                prop: None,
            }),
            _ => return Err(unknown()),
        }
    } else if let Some(command) = parse_invoke_group(state, target, action)? {
        command
    } else if let Some(command) = parse_invoke_live(state, target, action)? {
        command
    } else if let Some(id) = split(target, "inspector.") {
        match action {
            "toggle" => Command::SetSectionExpanded {
                section: id.to_string(),
                expanded: !state.inspector.is_expanded(id),
            },
            "reset" => Command::ResetSection {
                section: id.to_string(),
            },
            _ => return Err(unknown()),
        }
    } else if let Some(id) = split(target, "property:") {
        let property = id.to_string();
        match action {
            "reset" => Command::ResetProperty { property },
            "toggle" => {
                let def = property_def(id)
                    .filter(|def| matches!(def.kind, PropertyKind::Toggle))
                    .ok_or_else(unknown)?;
                let current = state
                    .inspector_clip()
                    .map_or(def.default, |clip| state.inspector.value(clip.id, def));
                Command::SetProperty {
                    property,
                    value: Raw::Bool(current != crate::state::inspector::Value::Bool(true)),
                }
            }
            _ => return Err(unknown()),
        }
    } else {
        return Err(unknown());
    };
    Ok(Parsed::Run(command))
}

/// `move clip:<id> [--track T] [--frame N] [--snap B]` or
/// `move track:<id> (--before T | --after T)`. `args` holds the flag values as
/// the CLI sent them (strings, or typed JSON).
pub fn parse_move(layout: &Layout, target: &str, args: &Value) -> Result<Parsed, CommandError> {
    let flag = |name: &str| args.get(name).and_then(Raw::from_json);
    if let Some(id) = split(target, "panel:") {
        return parse_move_panel(layout, target, id, args).map(Parsed::Layout);
    }
    if let Some(id) = split(target, "clip:") {
        for unsupported in ["before", "after"] {
            if flag(unsupported).is_some() {
                return Err(CommandError::new(
                    "invalid_argument",
                    format!("`--{unsupported}` applies to tracks, not clips"),
                )
                .target(target));
            }
        }
        let track = flag("track").map(|raw| raw.text());
        let frame = flag("frame").map(|raw| raw.integer()).transpose()?;
        if track.is_none() && frame.is_none() {
            return Err(missing("`--track` or `--frame`").target(target));
        }
        let snap = flag("snap").map(|raw| raw.boolean()).transpose()?;
        return Ok(Parsed::Run(Command::MoveClip {
            clip: id.to_string(),
            track,
            frame,
            snap,
        }));
    }
    if let Some(id) = split(target, "track:") {
        for unsupported in ["track", "frame", "snap"] {
            if flag(unsupported).is_some() {
                return Err(CommandError::new(
                    "invalid_argument",
                    format!("`--{unsupported}` applies to clips, not tracks"),
                )
                .target(target));
            }
        }
        let (anchor, placement) = match (flag("before"), flag("after")) {
            (Some(anchor), None) => (anchor.text(), Placement::Before),
            (None, Some(anchor)) => (anchor.text(), Placement::After),
            (None, None) => return Err(missing("`--before` or `--after`").target(target)),
            (Some(_), Some(_)) => {
                return Err(CommandError::new(
                    "invalid_argument",
                    "Use either `--before` or `--after`",
                )
                .target(target));
            }
        };
        return Ok(Parsed::Run(Command::ReorderTrack {
            track: id.to_string(),
            anchor,
            placement,
        }));
    }
    Err(CommandError::new(
        "unknown_action",
        "Only clips, tracks and panels can be moved",
    )
    .target(target))
}

/// `group:<n>` or `panel:<id>` (the group currently holding that panel), resolved
/// against the live layout so scripts need not know numeric IDs ahead of time.
fn parse_group(layout: &Layout, raw: &Raw, target: &str) -> Result<u64, CommandError> {
    let text = raw.text();
    let invalid = || {
        CommandError::invalid_value(
            format!("`{text}` is not a group"),
            json!({ "format": "group:<n> or panel:<id>" }),
        )
        .target(target)
    };
    let group = if let Some(id) = split(&text, "group:") {
        GroupRef::Id(id.parse().map_err(|_| invalid())?)
    } else if let Some(id) = split(&text, "panel:") {
        GroupRef::HoldingPanel(PanelKind::parse(id).ok_or_else(invalid)?)
    } else {
        return Err(invalid());
    };
    layout.resolve_group(&group)
}

/// `move panel:<id>` with exactly one of `--to <dock>`, `--group <group> [--index N]`
/// or `--beside <group> --side <left|right|top|bottom>`.
fn parse_move_panel(
    layout: &Layout,
    target: &str,
    id: &str,
    args: &Value,
) -> Result<LayoutCommand, CommandError> {
    let panel = parse_panel(id, target)?;
    let flag = |name: &str| args.get(name).and_then(Raw::from_json);
    let invalid = |message: &str| CommandError::new("invalid_argument", message).target(target);
    for unsupported in ["track", "frame", "snap", "before", "after"] {
        if flag(unsupported).is_some() {
            return Err(invalid(&format!(
                "`--{unsupported}` does not apply to panels"
            )));
        }
    }
    let destinations = ["to", "group", "beside"]
        .iter()
        .filter(|name| flag(name).is_some())
        .count();
    if destinations != 1 {
        return Err(missing("exactly one of `--to`, `--group` or `--beside`").target(target));
    }
    if flag("index").is_some() && flag("group").is_none() {
        return Err(invalid("`--index` only applies with `--group`"));
    }
    if flag("side").is_some() && flag("beside").is_none() {
        return Err(invalid("`--side` only applies with `--beside`"));
    }
    let to = if let Some(region) = flag("to") {
        Destination::Region(parse_region(&region.text(), target)?)
    } else if let Some(group) = flag("group") {
        let index = flag("index")
            .map(|raw| raw.integer())
            .transpose()?
            .map(|index| {
                usize::try_from(index).map_err(|_| {
                    CommandError::new("out_of_bounds", "`--index` cannot be negative")
                        .target(target)
                })
            })
            .transpose()?;
        Destination::Group {
            group: parse_group(layout, &group, target)?,
            index,
        }
    } else {
        let group = flag("beside").expect("counted above");
        let side_raw = flag("side").ok_or_else(|| missing("`--side`").target(target))?;
        let side = Side::parse(&side_raw.text()).ok_or_else(|| {
            CommandError::invalid_value(
                format!("`{}` is not a side", side_raw.text()),
                json!({ "options": Side::ALL.map(Side::id) }),
            )
            .target(target)
        })?;
        Destination::Beside {
            group: parse_group(layout, &group, target)?,
            side,
        }
    };
    Ok(LayoutCommand::Move { panel, to })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ProjectSettings;
    use crate::state::layout::default_layout;

    fn run(parsed: Result<Parsed, CommandError>) -> Command {
        match parsed.unwrap() {
            Parsed::Run(command) => command,
            Parsed::Layout(command) => panic!("unexpected dock command {command:?}"),
            Parsed::ToggleTheme
            | Parsed::PerfMeter(_)
            | Parsed::PerfHistory(_)
            | Parsed::ReloadViewport => {
                panic!("unexpected window command")
            }
        }
    }

    fn dock(parsed: Result<Parsed, CommandError>) -> LayoutCommand {
        match parsed.unwrap() {
            Parsed::Layout(command) => command,
            other => panic!("expected a dock command, got {other:?}"),
        }
    }

    #[test]
    fn set_maps_to_the_commands_the_ui_builds() {
        let text = |value: &str| Raw::Text(value.into());
        assert_eq!(
            run(parse_set("property:zoom", "value", &text("110"))),
            Command::SetProperty {
                property: "zoom".into(),
                value: text("110")
            }
        );
        assert_eq!(
            run(parse_set("folder:footage", "expanded", &text("false"))),
            Command::SetFolderExpanded {
                folder: "footage".into(),
                expanded: false
            }
        );
        assert_eq!(
            run(parse_set("inspector.tabs", "value", &text("audio"))),
            Command::SetInspectorTab {
                tab: InspectorTab::Audio
            }
        );
        assert_eq!(
            run(parse_set("track:score", "muted", &text("true"))),
            Command::SetTrackFlag {
                track: "score".into(),
                flag: TrackFlag::Muted,
                value: true
            }
        );
    }

    #[test]
    fn bad_choices_and_fields_are_structured_errors() {
        let text = |value: &str| Raw::Text(value.into());
        let error = parse_set("workspace.tabs", "value", &text("render")).unwrap_err();
        assert_eq!(error.code, "invalid_value");
        assert_eq!(error.details.unwrap()["expected"]["options"][2], "deliver");
        assert_eq!(
            parse_set("timeline.snap", "value", &text("maybe"))
                .unwrap_err()
                .code,
            "invalid_value"
        );
        assert_eq!(
            parse_set("folder:footage", "color", &text("red"))
                .unwrap_err()
                .code,
            "unknown_field"
        );
        assert_eq!(
            parse_set("nothing", "value", &text("1")).unwrap_err().code,
            "unknown_field"
        );
    }

    #[test]
    fn invoke_toggles_use_the_live_value() {
        let mut state = EditorState::default();
        assert_eq!(
            run(parse_invoke(
                &state,
                &default_layout(),
                "timeline.snap",
                "toggle"
            )),
            Command::SetSnap { snap: false }
        );
        state.timeline.snap = false;
        assert_eq!(
            run(parse_invoke(
                &state,
                &default_layout(),
                "timeline.snap",
                "toggle"
            )),
            Command::SetSnap { snap: true }
        );
        assert_eq!(
            run(parse_invoke(
                &state,
                &default_layout(),
                "folder:north-expedition",
                "toggle"
            )),
            Command::SetFolderExpanded {
                folder: "north-expedition".into(),
                expanded: false
            }
        );
        assert_eq!(
            run(parse_invoke(
                &state,
                &default_layout(),
                "monitor.step_forward",
                "invoke"
            )),
            seek(state.timeline.playhead + 1)
        );
        assert_eq!(
            parse_invoke(&state, &default_layout(), "toolbar.theme", "toggle").unwrap(),
            Parsed::ToggleTheme
        );
        assert_eq!(
            parse_invoke(&state, &default_layout(), "toolbar.perf_meter", "toggle").unwrap(),
            Parsed::PerfMeter(PerfMeterCommand::Toggle)
        );
        assert_eq!(
            parse_invoke(&state, &default_layout(), "clip:x", "explode")
                .unwrap_err()
                .code,
            "unknown_action"
        );
    }

    #[test]
    fn the_perf_meter_is_toggled_shown_and_hidden_through_one_parser() {
        let state = EditorState::default();
        let set = |value: Raw| parse_set(perf::TARGET, "visible", &value);
        assert_eq!(
            set(Raw::Bool(true)).unwrap(),
            Parsed::PerfMeter(PerfMeterCommand::Show)
        );
        assert_eq!(
            set(Raw::Text("false".into())).unwrap(),
            Parsed::PerfMeter(PerfMeterCommand::Hide)
        );
        // A value that is not a boolean is refused, and no other field exists.
        assert_eq!(
            set(Raw::Text("maybe".into())).unwrap_err().code,
            "invalid_value"
        );
        assert_eq!(
            parse_set(perf::TARGET, "size", &Raw::Number(1.0))
                .unwrap_err()
                .code,
            "unknown_field"
        );
        assert_eq!(
            parse_invoke(&state, &default_layout(), perf::TARGET, "toggle").unwrap(),
            Parsed::PerfMeter(PerfMeterCommand::Toggle)
        );
        assert_eq!(
            parse_invoke(&state, &default_layout(), perf::TARGET, "explode")
                .unwrap_err()
                .code,
            "unknown_action"
        );
    }

    #[test]
    fn ken_burns_toggle_flips_the_selected_clip_value() {
        let state = EditorState::default();
        assert_eq!(
            run(parse_invoke(
                &state,
                &default_layout(),
                "property:ken_burns",
                "toggle"
            )),
            Command::SetProperty {
                property: "ken_burns".into(),
                value: Raw::Bool(false)
            }
        );
    }

    #[test]
    fn move_requires_a_destination_and_rejects_foreign_flags() {
        let args = |value: Value| value;
        assert_eq!(
            run(parse_move(
                &default_layout(),
                "clip:first-light",
                &args(json!({"track":"titles","frame":"48"}))
            )),
            Command::MoveClip {
                clip: "first-light".into(),
                track: Some("titles".into()),
                frame: Some(48),
                snap: None
            }
        );
        assert_eq!(
            parse_move(&default_layout(), "clip:first-light", &args(json!({})))
                .unwrap_err()
                .code,
            "missing_argument"
        );
        assert_eq!(
            parse_move(
                &default_layout(),
                "clip:first-light",
                &args(json!({"before":"x"}))
            )
            .unwrap_err()
            .code,
            "invalid_argument"
        );
        assert_eq!(
            parse_move(
                &default_layout(),
                "clip:first-light",
                &args(json!({"frame":"1.5"}))
            )
            .unwrap_err()
            .code,
            "invalid_value"
        );
        assert_eq!(
            run(parse_move(
                &default_layout(),
                "track:score",
                &args(json!({"before":"ambience"}))
            )),
            Command::ReorderTrack {
                track: "score".into(),
                anchor: "ambience".into(),
                placement: Placement::Before
            }
        );
        assert_eq!(
            parse_move(
                &default_layout(),
                "track:score",
                &args(json!({"before":"a","after":"b"}))
            )
            .unwrap_err()
            .code,
            "invalid_argument"
        );
        assert_eq!(
            parse_move(
                &default_layout(),
                "track:score",
                &args(json!({"frame":"3","before":"a"}))
            )
            .unwrap_err()
            .code,
            "invalid_argument"
        );
        assert_eq!(
            parse_move(&default_layout(), "media:ao01", &args(json!({"frame":"3"})))
                .unwrap_err()
                .code,
            "unknown_action"
        );
    }

    #[test]
    fn panel_visibility_maps_to_dock_commands_from_every_spelling() {
        let text = |value: &str| Raw::Text(value.into());
        let hide = LayoutCommand::SetVisible {
            panel: PanelKind::Inspector,
            visible: false,
        };
        assert_eq!(
            dock(parse_set("region:inspector", "visible", &text("false"))),
            hide
        );
        assert_eq!(
            dock(parse_set("panel:inspector", "visible", &text("false"))),
            hide
        );
        let state = EditorState::default();
        assert_eq!(
            dock(parse_invoke(
                &state,
                &default_layout(),
                "panel:inspector",
                "hide"
            )),
            hide
        );
        // The header toggle reads the live layout, so a hidden panel is shown again.
        let mut layout = default_layout();
        assert_eq!(
            dock(parse_invoke(
                &state,
                &layout,
                "toolbar.panel.library",
                "toggle"
            )),
            LayoutCommand::SetVisible {
                panel: PanelKind::Library,
                visible: false
            }
        );
        layout
            .regions
            .retain(|region| region.region != Region::Left);
        assert_eq!(
            dock(parse_invoke(
                &state,
                &layout,
                "toolbar.panel.library",
                "toggle"
            )),
            LayoutCommand::SetVisible {
                panel: PanelKind::Library,
                visible: true
            }
        );
        assert_eq!(
            dock(parse_invoke(&state, &default_layout(), "dock", "reset")),
            LayoutCommand::Reset
        );
    }

    #[test]
    fn dock_geometry_and_layout_fields_parse() {
        let text = |value: &str| Raw::Text(value.into());
        assert_eq!(
            dock(parse_set("dock:left", "size", &text("300"))),
            LayoutCommand::ResizeRegion {
                region: Region::Left,
                size: 300.0
            }
        );
        assert_eq!(
            dock(parse_set("dock:bottom", "open", &text("false"))),
            LayoutCommand::SetRegionOpen {
                region: Region::Bottom,
                open: false
            }
        );
        assert_eq!(
            dock(parse_set("split:7", "sizes", &text("120, 300"))),
            LayoutCommand::ResizeSplit {
                split: 7,
                sizes: vec![120.0, 300.0]
            }
        );
        assert_eq!(
            parse_set("split:7", "sizes", &text("a,b"))
                .unwrap_err()
                .code,
            "invalid_value"
        );
        assert_eq!(
            parse_set("dock:nowhere", "size", &text("3"))
                .unwrap_err()
                .code,
            "unknown_element"
        );
        assert_eq!(
            dock(parse_set("dock", "layout", &text("{}"))),
            LayoutCommand::Load { json: "{}".into() }
        );
    }

    #[test]
    fn monitor_controls_map_to_preview_commands_and_a_window_reload() {
        let layout = default_layout();
        let mut state = EditorState::demo();
        let invoke =
            |state: &EditorState, target: &str| parse_invoke(state, &layout, target, "invoke");
        assert_eq!(
            run(invoke(&state, "monitor.preview.sequence")),
            Command::SetPreview {
                intent: PreviewIntent::Sequence
            }
        );
        assert!(matches!(
            invoke(&state, "monitor.reload"),
            Ok(Parsed::ReloadViewport)
        ));
        // The source mode previews whatever the selection points at, from its first frame.
        let Command::SetPreview {
            intent: PreviewIntent::Source { asset, time },
        } = run(invoke(&state, "monitor.preview.source"))
        else {
            panic!("expected a source preview");
        };
        assert_eq!(asset, state.previewable_media().unwrap().id);
        assert_eq!(time, Ratio::ZERO);
        state
            .execute(Command::SetPreview {
                intent: PreviewIntent::Source { asset, time },
            })
            .unwrap();
        assert_eq!(
            run(invoke(&state, "monitor.preview.sequence")),
            Command::SetPreview {
                intent: PreviewIntent::Sequence
            }
        );
    }

    #[test]
    fn source_preview_without_a_previewable_selection_is_refused() {
        let state = EditorState::empty(&ProjectSettings::default()).unwrap();
        let error = parse_invoke(
            &state,
            &default_layout(),
            "monitor.preview.source",
            "invoke",
        )
        .unwrap_err();
        assert_eq!(error.code, "no_selection");
    }

    #[test]
    fn moving_panels_needs_exactly_one_destination() {
        let layout = default_layout();
        let go = |args: Value| parse_move(&layout, "panel:library", &args);
        assert_eq!(
            dock(go(json!({"to":"right"}))),
            LayoutCommand::Move {
                panel: PanelKind::Library,
                to: Destination::Region(Region::Right)
            }
        );
        // `panel:inspector` names whichever group currently holds the inspector.
        assert_eq!(
            dock(go(json!({"group":"panel:inspector","index":"0"}))),
            LayoutCommand::Move {
                panel: PanelKind::Library,
                to: Destination::Group {
                    group: layout
                        .resolve_group(&GroupRef::HoldingPanel(PanelKind::Inspector))
                        .unwrap(),
                    index: Some(0)
                }
            }
        );
        assert_eq!(
            dock(go(json!({"beside":"group:4","side":"top"}))),
            LayoutCommand::Move {
                panel: PanelKind::Library,
                to: Destination::Beside {
                    group: 4,
                    side: Side::Top
                }
            }
        );
        for bad in [
            json!({}),
            json!({"to":"right","group":"group:4"}),
            json!({"beside":"group:4"}),
            json!({"to":"right","side":"top"}),
            json!({"to":"right","index":"1"}),
            json!({"to":"right","frame":"3"}),
            json!({"to":"sideways"}),
            json!({"group":"nonsense"}),
            json!({"beside":"group:4","side":"up"}),
            json!({"group":"group:4","index":"-1"}),
        ] {
            assert!(go(bad.clone()).is_err(), "{bad}");
        }
        assert_eq!(
            go(json!({"group":"group:9999"})).unwrap_err().code,
            "unknown_element"
        );
    }

    #[test]
    fn the_perf_history_is_driven_by_set_and_invoke_through_one_parser() {
        use std::time::Duration;
        let set = |field: &str, value: Raw| parse_set(perf_history::TARGET, field, &value);
        let history = |parsed: Result<Parsed, CommandError>| match parsed.unwrap() {
            Parsed::PerfHistory(command) => command,
            other => panic!("expected a history command, got {other:?}"),
        };
        assert_eq!(
            history(set("recording", Raw::Bool(true))),
            HistoryCommand::Record(true)
        );
        assert_eq!(
            history(set("recording", Raw::Text("off".into()))),
            HistoryCommand::Record(false)
        );
        assert_eq!(
            history(set("visible", Raw::Bool(true))),
            HistoryCommand::Show
        );
        assert_eq!(
            history(set("visible", Raw::Bool(false))),
            HistoryCommand::Hide
        );
        assert_eq!(
            history(set("frozen", Raw::Bool(true))),
            HistoryCommand::Freeze
        );
        assert_eq!(
            history(set("frozen", Raw::Bool(false))),
            HistoryCommand::Resume
        );
        assert_eq!(
            history(set("selected_ms", Raw::Number(1500.0))),
            HistoryCommand::Select(Some(Duration::from_millis(1500)))
        );
        assert_eq!(
            history(set("export_to", Raw::Text("/tmp/a.json".into()))),
            HistoryCommand::Export("/tmp/a.json".into())
        );
        assert_eq!(
            history(set("export_destination", Raw::Text("/tmp/b.json".into()))),
            HistoryCommand::Destination("/tmp/b.json".into())
        );
        assert_eq!(
            set("selected_ms", Raw::Number(-1.0)).unwrap_err().code,
            "invalid_value"
        );
        assert_eq!(
            set("selected_ms", Raw::Text("soon".into()))
                .unwrap_err()
                .code,
            "invalid_value"
        );
        // Finite, non-negative and still not an instant: refused, not a panic in the parser.
        assert_eq!(
            set("selected_ms", Raw::Number(1e300)).unwrap_err().code,
            "invalid_value"
        );
        assert_eq!(
            set("selected_ms", Raw::Text("1e300".into()))
                .unwrap_err()
                .code,
            "invalid_value"
        );
        assert_eq!(
            set("recording", Raw::Number(1.0)).unwrap_err().code,
            "invalid_value"
        );
        assert_eq!(
            set("speed", Raw::Number(1.0)).unwrap_err().code,
            "unknown_field"
        );
        let invoke = |action: &str| {
            history(parse_invoke(
                &EditorState::demo(),
                &default_layout(),
                perf_history::TARGET,
                action,
            ))
        };
        for (action, expected) in [
            ("record_start", HistoryCommand::Record(true)),
            ("record_stop", HistoryCommand::Record(false)),
            ("show", HistoryCommand::Show),
            ("hide", HistoryCommand::Hide),
            ("freeze", HistoryCommand::Freeze),
            ("resume", HistoryCommand::Resume),
            ("clear", HistoryCommand::Clear),
            ("snapshot", HistoryCommand::Snapshot),
            ("clear_selection", HistoryCommand::Select(None)),
            ("export", HistoryCommand::ExportToDestination),
        ] {
            assert_eq!(invoke(action), expected, "{action}");
            assert!(
                perf_history::ACTIONS.contains(&action),
                "{action} is documented"
            );
        }
        assert_eq!(
            parse_invoke(
                &EditorState::demo(),
                &default_layout(),
                perf_history::TARGET,
                "explode"
            )
            .unwrap_err()
            .code,
            "unknown_action"
        );
    }
}
