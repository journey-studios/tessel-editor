//! Translation of semantic requests into editor [`Command`]s, without GPUI.
//!
//! Desktop automation may additionally validate against its live component tree.
//! Headless callers do not depend on control visibility: domain validation belongs
//! to `EditorState::execute`. Both adapters use the same command vocabulary.

use serde_json::{Value, json};

use crate::plan::PreviewIntent;
use crate::preview::PreviewMode;
use crate::state::inspector::{InspectorTab, PropertyKind, property_def};
use crate::state::layout::{Destination, GroupRef, Layout, LayoutCommand, PanelKind, Region, Side};
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
        ("timeline.zoom", "value") => Command::SetTimelineZoom {
            zoom: value.clone(),
        },
        ("timeline.snap", "value") => Command::SetSnap {
            snap: value.boolean()?,
        },
        ("timeline.playhead", "frame") => Command::SetPlayhead {
            frame: value.clone(),
        },
        ("dock", "layout") => {
            return Ok(Parsed::Layout(LayoutCommand::Load { json: value.text() }));
        }
        _ => return parse_set_entity(target, field, value),
    };
    Ok(Parsed::Run(command))
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
        if !["content", "size", "color"].contains(&field) {
            return Err(unknown_field(target, field));
        }
        Command::SetClipTextField {
            clip: id.to_string(),
            field: field.to_string(),
            value: value.clone(),
        }
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
        ("timeline.snap", "toggle") => Command::SetSnap {
            snap: !state.timeline.snap,
        },
        ("folder:all", "select") => Command::SelectFolder { folder: None },
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
        _ => return parse_invoke_entity(state, target, action),
    };
    Ok(Parsed::Run(command))
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
            Parsed::ToggleTheme | Parsed::ReloadViewport => panic!("unexpected window command"),
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
            parse_invoke(&state, &default_layout(), "clip:x", "explode")
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
}
