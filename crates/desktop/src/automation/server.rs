use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::time::Duration;

use gpui_kit::*;
use serde_json::{Value, json};

use super::commands::{Parsed, parse_invoke, parse_move, parse_set};
use super::model;
use crate::header;
use crate::shell::EditorShell;
use crate::state::layout::LayoutCommand;
use crate::state::{Command, CommandError, Raw};
use crate::theme::{apply as apply_scheme, scheme_name, toggle as toggle_scheme};
use crate::workstation::{FrameWaiter, OpReply, Source, Workstation};

pub fn start(
    window: AnyWindowHandle,
    shell: Entity<EditorShell>,
    cx: &mut App,
) -> Result<(), String> {
    let path = super::socket_path()?;
    let directory = path.parent().ok_or("Invalid socket path")?;
    fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    if let Ok(metadata) = fs::symlink_metadata(&path) {
        if !metadata.file_type().is_socket() {
            return Err("Socket path is not a socket".into());
        }
        if UnixStream::connect(&path).is_ok() {
            return Err("An editor instance is already running".into());
        }
        fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    let listener = UnixListener::bind(&path).map_err(|e| e.to_string())?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    let (sender, receiver) = async_channel::bounded(8);
    std::thread::spawn(move || {
        for incoming in listener.incoming() {
            let Ok(mut stream) = incoming else { break };
            if stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .is_err()
                || stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .is_err()
            {
                continue;
            }
            let mut line = String::new();
            let read = BufReader::new(&stream).take(65_537).read_line(&mut line);
            if read.is_err() || line.len() > 65_536 || !line.ends_with('\n') {
                let _ = writeln!(
                    stream,
                    "{}",
                    model::error("invalid_request", "Expected one JSON line, maximum 64 KiB")
                );
                continue;
            }
            let request: Value = match serde_json::from_str(&line) {
                Ok(request) => request,
                Err(_) => {
                    let _ = writeln!(
                        stream,
                        "{}",
                        model::error("invalid_json", "Request must be JSON")
                    );
                    continue;
                }
            };
            if sender.send_blocking((request, stream)).is_err() {
                break;
            }
        }
    });
    cx.spawn(async move |cx| {
        while let Ok((request, mut stream)) = receiver.recv().await {
            let result = window.update(cx, |_, window, cx| {
                let closing = request["command"] == "invoke"
                    && request["target"] == "window.close"
                    && request["version"] == 1;
                let workstation = shell.read(cx).workstation.clone();
                let waiting = match dispatch(&request, &workstation, window, cx) {
                    Reply::Now(response) => {
                        // WHY: acknowledge close before the UI exits and drops the IPC transport.
                        let _ = writeln!(stream, "{response}");
                        None
                    }
                    Reply::Later(waiter) => Some((stream, waiter)),
                };
                if closing {
                    window.remove_window();
                }
                waiting
            });
            match result {
                Err(_) => break,
                // A deferred answer must not stop the loop: other commands keep being served
                // while the viewport draws.
                Ok(Some((mut stream, waiter))) => {
                    cx.spawn(async move |_| {
                        let response = match waiter.recv().await {
                            Ok(Ok(data)) => json!({"ok":true,"version":1,"data":data}),
                            Ok(Err(error)) => error.to_json(),
                            Err(_) => model::error(
                                "cancelled",
                                "The viewport dropped the request without an answer",
                            ),
                        };
                        let _ = writeln!(stream, "{response}");
                    })
                    .detach();
                }
                Ok(None) => {}
            }
        }
    })
    .detach();
    Ok(())
}

/// Most commands answer at once; `op` with `render.frame` answers when the frame is drawn.
enum Reply {
    Now(Value),
    Later(FrameWaiter),
}

fn dispatch(
    request: &Value,
    workstation: &Entity<Workstation>,
    window: &mut Window,
    cx: &mut App,
) -> Reply {
    if request["command"] != "op" || request["version"] != 1 {
        return Reply::Now(dispatch_now(request, workstation, window, cx));
    }
    match workstation.update(cx, |workstation, cx| {
        workstation.run_op(&request["request"], cx)
    }) {
        Ok(OpReply::Done(data)) => Reply::Now(json!({"ok":true,"version":1,"data":data})),
        Ok(OpReply::Pending(waiter)) => Reply::Later(waiter),
        Err(error) => Reply::Now(error.to_json()),
    }
}

fn dispatch_now(
    request: &Value,
    workstation: &Entity<Workstation>,
    window: &mut Window,
    cx: &mut App,
) -> Value {
    if request["version"] != 1 {
        return model::error("unsupported_version", "Expected protocol version 1");
    }
    let target = request["target"].as_str().unwrap_or_default();
    let data = match request["command"].as_str() {
        Some("tree") => {
            let mut nodes = workstation.read(cx).tree_nodes(cx);
            let header_nodes = header::nodes();
            let roots: Vec<Value> = header_nodes
                .iter()
                .filter(|node| node["parent"] == "region:header")
                .map(|node| node["id"].clone())
                .collect();
            for node in &mut nodes {
                if node["id"] == "region:header"
                    && let Some(children) = node["children"].as_array_mut()
                {
                    children.extend(roots.iter().cloned());
                }
            }
            nodes.extend(header_nodes);
            nodes.push(window_node(window));
            json!({"source":"live_editor_state+live_dock_area","nodes":nodes,
                "native_actions":model::native_actions()})
        }
        Some("actions") => {
            let mut actions = workstation.read(cx).tree_actions(cx);
            if let Some(native) = model::native_actions().as_array() {
                actions.extend(native.iter().cloned());
            }
            json!(actions)
        }
        Some("status") => {
            let bounds = window.bounds();
            json!({"pid":std::process::id(),"maximized":window.is_maximized(),
                "active":window.is_window_active(),"theme":scheme_name(cx),
                "window":{"width":f32::from(bounds.size.width),"height":f32::from(bounds.size.height)},
                "editor":workstation.read(cx).state.summary()})
        }
        Some("get") if target == "window" => window_node(window),
        Some("get") => match lookup(workstation, target, cx) {
            Ok(node) => node,
            Err(error) => return error.to_json(),
        },
        Some("theme") => return invoke(target, None, workstation, window, cx),
        Some("invoke") => {
            return invoke(target, request["action"].as_str(), workstation, window, cx);
        }
        Some("set") if target == "window" => {
            if request["field"] != "size" {
                return model::error("unknown_field", "Window exposes the size field");
            }
            let raw = request["value"].as_str().unwrap_or_default();
            let (width, height) = match model::parse_window_size(raw) {
                Ok(size) => size,
                Err(message) => return model::error("invalid_value", &message),
            };
            window.resize(size(px(width), px(height)));
            json!({"target":"window","requested_size":[width,height],"dispatched":true})
        }
        Some("set") => return set(request, workstation, window, cx),
        Some("move") => return move_item(request, workstation, window, cx),
        _ => {
            return model::error(
                "unknown_command",
                "Use tree, actions, status, get, set, move, invoke, op or theme",
            );
        }
    };
    json!({"ok":true,"version":1,"data":data})
}

/// Finds a node in the live tree or the header. `dock` also carries the full
/// layout so one `get dock` answers every question about it.
fn lookup(workstation: &Entity<Workstation>, id: &str, cx: &App) -> Result<Value, CommandError> {
    let node = if id.starts_with("node:") {
        header::node(id)
    } else {
        workstation.read(cx).tree_node(id, cx)
    };
    let node = node.map(|mut node| {
        if id == "dock" {
            node["layout"] = workstation.read(cx).dock_report(cx);
        }
        node
    });
    node.ok_or_else(|| CommandError::new("unknown_element", "Element ID does not exist").target(id))
}

fn require_enabled(node: &Value) -> Result<(), CommandError> {
    if node["enabled"] == false {
        let reason = node["disabled_reason"]
            .as_str()
            .unwrap_or("This element is disabled");
        return Err(CommandError::disabled(reason).target(node["id"].as_str().unwrap_or_default()));
    }
    Ok(())
}

fn run(workstation: &Entity<Workstation>, command: Command, cx: &mut App) -> Value {
    match workstation.update(cx, |workstation, cx| {
        workstation.execute(command, Source::Cli, cx)
    }) {
        Ok(outcome) => json!({"ok":true,"version":1,"data":outcome}),
        Err(error) => error.to_json(),
    }
}

/// The debug meter (F049): the same `Workstation::execute_perf` the header button and the
/// shortcut call.
fn run_perf(
    workstation: &Entity<Workstation>,
    command: tessel_editor_core::perf::PerfMeterCommand,
    cx: &mut App,
) -> Value {
    match workstation.update(cx, |workstation, cx| {
        workstation.execute_perf(command, Source::Cli, cx)
    }) {
        Ok(outcome) => json!({"ok":true,"version":1,"data":outcome}),
        Err(error) => error.to_json(),
    }
}

/// Dock edits go through the same handler as the header buttons and panel menus.
fn run_layout(
    workstation: &Entity<Workstation>,
    command: LayoutCommand,
    window: &mut Window,
    cx: &mut App,
) -> Value {
    match workstation.update(cx, |workstation, cx| {
        workstation.execute_layout(command, Source::Cli, window, cx)
    }) {
        Ok(outcome) => json!({"ok":true,"version":1,"data":outcome}),
        Err(error) => error.to_json(),
    }
}

fn set(
    request: &Value,
    workstation: &Entity<Workstation>,
    window: &mut Window,
    cx: &mut App,
) -> Value {
    let target = request["target"].as_str().unwrap_or_default();
    let field = request["field"].as_str().unwrap_or_default();
    if target.starts_with("dialog.field.") {
        // Dialog fields are window state: set the form, never run the operation.
        let value = request["value"]
            .as_str()
            .map_or_else(|| request["value"].to_string(), str::to_string);
        let outcome = workstation.update(cx, |workstation, cx| {
            workstation.workflow_set(target, field, &value, window, cx)
        });
        return match outcome {
            Some(Ok(data)) => json!({"ok":true,"version":1,"data":data}),
            Some(Err(error)) => error.target(target).to_json(),
            None => CommandError::new("unknown_field", "Not settable").to_json(),
        };
    }
    if target == "inspector.mode" || target.starts_with("scene.") {
        // The scene inspector's nodes: a property component runs the same begin/update/commit the
        // slider sends, so one `set` is one undo entry (`scene_ui.rs`).
        let Some(raw) = Raw::from_json(&request["value"]) else {
            return CommandError::new(
                "missing_argument",
                "Missing a string, number or boolean value",
            )
            .target(target)
            .to_json();
        };
        let outcome = workstation.update(cx, |workstation, cx| {
            workstation.scene_set(target, field, &raw, cx)
        });
        return match outcome {
            Some(Ok(data)) => json!({"ok":true,"version":1,"data":data}),
            Some(Err(error)) if error.target.is_none() => error.target(target).to_json(),
            Some(Err(error)) => error.to_json(),
            None => CommandError::new("unknown_field", "Not settable").to_json(),
        };
    }
    let prepared = (|| {
        let node = lookup(workstation, target, cx)?;
        require_enabled(&node)?;
        let declared: Vec<&str> = node["fields"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|entry| entry["field"].as_str())
            .collect();
        if !declared.contains(&field) {
            return Err(CommandError::new(
                "unknown_field",
                format!("`{target}` has no settable field `{field}`"),
            )
            .target(target)
            .details(json!({ "fields": declared })));
        }
        let raw = Raw::from_json(&request["value"]).ok_or_else(|| {
            CommandError::new(
                "missing_argument",
                "Missing a string, number or boolean value",
            )
            .target(target)
        })?;
        parse_set(target, field, &raw)
    })();
    match prepared {
        Ok(Parsed::Run(command)) => run(workstation, command, cx),
        Ok(Parsed::Layout(command)) => run_layout(workstation, command, window, cx),
        Ok(Parsed::PerfMeter(command)) => run_perf(workstation, command, cx),
        Ok(Parsed::ToggleTheme | Parsed::ReloadViewport) => {
            CommandError::new("unknown_field", "Not settable").to_json()
        }
        Err(error) => error.to_json(),
    }
}

fn move_item(
    request: &Value,
    workstation: &Entity<Workstation>,
    window: &mut Window,
    cx: &mut App,
) -> Value {
    let target = request["target"].as_str().unwrap_or_default();
    let prepared = lookup(workstation, target, cx)
        .and_then(|node| require_enabled(&node))
        .and_then(|()| {
            let layout = workstation.read(cx).layout_snapshot(cx);
            parse_move(&layout, target, &request["args"])
        });
    match prepared {
        Ok(Parsed::Run(command)) => run(workstation, command, cx),
        Ok(Parsed::Layout(command)) => run_layout(workstation, command, window, cx),
        Ok(Parsed::PerfMeter(_) | Parsed::ToggleTheme | Parsed::ReloadViewport) => {
            CommandError::new("unknown_action", "Not movable").to_json()
        }
        Err(error) => error.to_json(),
    }
}

fn native(target: &str, window: &mut Window, cx: &mut App) -> Option<Value> {
    match target {
        "window.close" => {}
        "window.minimize" => window.minimize_window(),
        "window.maximize" => window.zoom_window(),
        "window.fullscreen" => window.toggle_fullscreen(),
        "window.activate" => {
            cx.activate(true);
            window.activate_window();
        }
        "theme.dark" | "theme.light" => apply_scheme(cx, target == "theme.dark"),
        _ => return None,
    }
    Some(
        json!({"ok":true,"version":1,"data":{"action":target,"dispatched":true,"theme":scheme_name(cx)}}),
    )
}

fn invoke(
    target: &str,
    action: Option<&str>,
    workstation: &Entity<Workstation>,
    window: &mut Window,
    cx: &mut App,
) -> Value {
    // `theme dark|light` arrives as a bare scheme name.
    let target = match target {
        "dark" | "light" => format!("theme.{target}"),
        other => other.to_string(),
    };
    if let Some(response) = native(&target, window, cx) {
        return response;
    }
    // Flows, dialog buttons and the header's undo/redo/save/import/export are window gestures.
    if let Some(outcome) = workstation.update(cx, |workstation, cx| {
        workstation.workflow_invoke(&target, action, window, cx)
    }) {
        return match outcome {
            Ok(data) => json!({"ok":true,"version":1,"data":data}),
            Err(error) => error.target(&target).to_json(),
        };
    }
    // Entities, properties and the scene panel of the directed-scene inspector.
    if let Some(outcome) = workstation.update(cx, |workstation, cx| {
        workstation.scene_invoke(&target, action, cx)
    }) {
        return match outcome {
            Ok(data) => json!({"ok":true,"version":1,"data":data}),
            Err(error) if error.target.is_none() => error.target(&target).to_json(),
            Err(error) => error.to_json(),
        };
    }
    let prepared = (|| {
        let node = lookup(workstation, &target, cx)?;
        require_enabled(&node)?;
        let available: Vec<&str> = node["actions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|entry| entry["action"].as_str())
            .collect();
        let action = match (action, available.as_slice()) {
            (Some(action), _) => action,
            (None, [only]) => only,
            (None, []) => {
                return Err(
                    CommandError::new("unknown_action", "This element has no actions")
                        .target(&target),
                );
            }
            (None, _) => {
                return Err(
                    CommandError::new("missing_argument", "Name the action to invoke")
                        .target(&target)
                        .details(json!({ "actions": available })),
                );
            }
        };
        if !available.contains(&action) {
            return Err(CommandError::new(
                "unknown_action",
                format!("`{target}` has no action `{action}`"),
            )
            .target(&target)
            .details(json!({ "actions": available })));
        }
        let layout = workstation.read(cx).layout_snapshot(cx);
        parse_invoke(&workstation.read(cx).state, &layout, &target, action)
    })();
    match prepared {
        Ok(Parsed::Run(command)) => run(workstation, command, cx),
        Ok(Parsed::Layout(command)) => run_layout(workstation, command, window, cx),
        Ok(Parsed::PerfMeter(command)) => run_perf(workstation, command, cx),
        Ok(Parsed::ReloadViewport) => {
            let reloaded =
                workstation.update(cx, |workstation, cx| workstation.viewport_reload(cx));
            if !reloaded {
                return CommandError::new(
                    "backend_unavailable",
                    "There is no viewport page to reload: build it with `pnpm --dir runtime run build`",
                )
                .to_json();
            }
            json!({"ok":true,"version":1,"data":{"target":"monitor.reload","action":"reload","changed":true}})
        }
        Ok(Parsed::ToggleTheme) => {
            toggle_scheme(cx);
            json!({"ok":true,"version":1,"data":{"target":"toolbar.theme","theme":scheme_name(cx)}})
        }
        Err(error) => error.to_json(),
    }
}

fn window_node(window: &Window) -> Value {
    let bounds = window.bounds();
    json!({"id":"window","kind":"window","enabled":true,"visible":true,
        "width":f32::from(bounds.size.width),"height":f32::from(bounds.size.height),
        "fields":[{"field":"size","format":"WIDTHxHEIGHT","minimum":[960,640],"maximum":[8192,8192]}],
        "actions":[]})
}
