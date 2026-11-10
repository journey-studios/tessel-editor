use std::io::{BufRead, BufReader, Write};
use std::time::Duration;

use serde_json::{Map, Value, json};

use super::UnixStream;

const HELP: &str = "tessel-editor ui <command>

  tree                          live component tree (regions, controls, values)
  status                        window, theme and editor summary
  actions                       every operation currently available
  get <id>                      one node: state, fields, actions, limits
  set <id> <field> <value>      e.g. set property:zoom value 110
                                     set folder:footage expanded false
                                     set timeline.playhead frame 72
  move clip:<id> [--track T] [--frame N] [--snap true|false]
  move track:<id> (--before T | --after T)
  invoke <id> [action]          e.g. invoke clip:first-light select
                                     invoke window.minimize
  theme <dark|light>
  op <json | @file>             a project operation, e.g.
                                     op '{\"op\":\"seek\",\"frame\":24}'
                                     op '{\"op\":\"render.frame\",\"probes\":[[10,10]]}'
                                (the same JSON the headless CLI takes; render.frame answers
                                 once the viewport has drawn the frame)

Docking (acts on the live DockArea; the same handlers as the header buttons and drags):
  dock                          live layout: docks, groups, splits, panels, hidden panels
  dock show|hide|activate <panel>
  dock move <panel> --to <left|right|bottom|center>
  dock move <panel> --group <group:N|panel:P> [--index N]
  dock move <panel> --beside <group:N|panel:P> --side <left|right|top|bottom>
  dock resize <left|right|bottom> <px>
  dock open|close <left|right|bottom>
  dock split <split:N> <px,px,...>
  dock load <json | @file>      restore a layout produced by `dock`'s `layout.dump`
  dock reset                    the default layout
  Panels: library, monitor, timeline, inspector. Same operations exist as
  `invoke panel:<id> hide`, `set dock:left size 300`, `move panel:<id> --to bottom`.

IDs come from `tree`. Failures print a JSON error and exit nonzero.";

pub fn run_cli(args: &[String]) -> Result<(), String> {
    if args.is_empty() || args == ["--help"] {
        println!("{HELP}");
        return Ok(());
    }
    let request = request(args)?;
    let mut stream = UnixStream::connect(super::socket_path()?)
        .map_err(|error| format!("Cannot connect to the running editor: {error}"))?;
    // A frame request waits for the viewport (assets included); the editor itself gives up
    // after 30 s and answers with a timeout error, so this only has to outlast that.
    let wait = if request["command"] == "op" { 45 } else { 5 };
    stream
        .set_read_timeout(Some(Duration::from_secs(wait)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    writeln!(stream, "{request}").map_err(|e| e.to_string())?;
    let mut response = String::new();
    BufReader::new(stream)
        .read_line(&mut response)
        .map_err(|e| e.to_string())?;
    if response.is_empty() {
        return Err("Editor closed without a response".into());
    }
    let value: Value = serde_json::from_str(&response).map_err(|e| e.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
    );
    if value["ok"] != true {
        return Err("The editor rejected the command; see the JSON error above".into());
    }
    Ok(())
}

/// `--flag value` pairs after the target. Values stay strings; the editor
/// parses them against the control's type.
fn flags(args: &[String]) -> Result<Map<String, Value>, String> {
    let mut map = Map::new();
    let mut iter = args.iter();
    while let Some(flag) = iter.next() {
        let name = flag
            .strip_prefix("--")
            .ok_or_else(|| format!("Unexpected argument `{flag}`; flags look like --frame 72"))?;
        let value = iter
            .next()
            .ok_or_else(|| format!("`--{name}` needs a value"))?;
        map.insert(name.to_string(), json!(value));
    }
    Ok(map)
}

/// `dock ...` is sugar over get/set/move/invoke, so there is one protocol.
fn dock_request(args: &[String]) -> Result<Value, String> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let panel = |id: &&str| format!("panel:{id}");
    let region = |id: &&str| format!("dock:{id}");
    match words.as_slice() {
        [] => Ok(json!({"version":1,"command":"get","target":"dock"})),
        ["reset"] => Ok(json!({"version":1,"command":"invoke","target":"dock","action":"reset"})),
        [action @ ("show" | "hide" | "activate"), id] => {
            Ok(json!({"version":1,"command":"invoke","target":panel(id),"action":action}))
        }
        ["move", id, ..] => {
            Ok(json!({"version":1,"command":"move","target":panel(id),"args":flags(&args[2..])?}))
        }
        ["resize", id, size] => {
            Ok(json!({"version":1,"command":"set","target":region(id),"field":"size","value":size}))
        }
        [action @ ("open" | "close"), id] => Ok(json!({"version":1,"command":"set",
            "target":region(id),"field":"open","value":(*action == "open").to_string()})),
        ["split", id, sizes] => {
            let target = if id.starts_with("split:") {
                id.to_string()
            } else {
                format!("split:{id}")
            };
            Ok(json!({"version":1,"command":"set","target":target,"field":"sizes","value":sizes}))
        }
        ["load", source] => {
            let json = match source.strip_prefix('@') {
                Some(path) => std::fs::read_to_string(path)
                    .map_err(|error| format!("Cannot read `{path}`: {error}"))?,
                None => source.to_string(),
            };
            Ok(json!({"version":1,"command":"set","target":"dock","field":"layout","value":json}))
        }
        _ => Err("Invalid dock command. Run tessel-editor ui --help".into()),
    }
}

pub(super) fn request(args: &[String]) -> Result<Value, String> {
    if let [first, rest @ ..] = args
        && first == "dock"
    {
        return dock_request(rest);
    }
    if let [first, source] = args
        && first == "op"
    {
        let text = match source.strip_prefix('@') {
            Some(path) => std::fs::read_to_string(path)
                .map_err(|error| format!("Cannot read `{path}`: {error}"))?,
            None => source.clone(),
        };
        let operation: Value = serde_json::from_str(&text)
            .map_err(|error| format!("`op` takes one JSON object: {error}"))?;
        if !operation.is_object() {
            return Err("`op` takes one JSON object".into());
        }
        return Ok(json!({"version":1,"command":"op","request":operation}));
    }
    match args {
        [command] if ["tree", "status", "actions"].contains(&command.as_str()) => {
            Ok(json!({"version":1,"command":command}))
        }
        [command, target] if ["get", "theme"].contains(&command.as_str()) => {
            Ok(json!({"version":1,"command":command,"target":target}))
        }
        [command, target] if command == "invoke" => {
            Ok(json!({"version":1,"command":"invoke","target":target}))
        }
        [command, target, action] if command == "invoke" => {
            Ok(json!({"version":1,"command":"invoke","target":target,"action":action}))
        }
        [command, target, field, value @ ..] if command == "set" && !value.is_empty() => {
            // Joining keeps `set media.search value river detail` unquoted-friendly.
            Ok(
                json!({"version":1,"command":"set","target":target,"field":field,
                "value":value.join(" ")}),
            )
        }
        [command, target, rest @ ..] if command == "move" => {
            Ok(json!({"version":1,"command":"move","target":target,"args":flags(rest)?}))
        }
        _ => Err("Invalid command. Run tessel-editor ui --help".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| part.to_string()).collect()
    }

    #[test]
    fn set_joins_the_value_words() {
        let request = request(&args(&["set", "media.search", "value", "river", "detail"])).unwrap();
        assert_eq!(request["value"], "river detail");
        assert_eq!(request["field"], "value");
    }

    #[test]
    fn move_collects_flags_as_strings() {
        let request = request(&args(&[
            "move", "clip:a", "--track", "titles", "--frame", "-5",
        ]))
        .unwrap();
        assert_eq!(request["args"]["track"], "titles");
        assert_eq!(request["args"]["frame"], "-5");
    }

    #[test]
    fn op_wraps_one_json_object() {
        let request = request(&args(&["op", r#"{"op":"seek","frame":24}"#])).unwrap();
        assert_eq!(request["command"], "op");
        assert_eq!(request["request"]["frame"], 24);
        assert!(super::request(&args(&["op", "[1]"])).is_err());
        assert!(super::request(&args(&["op", "not json"])).is_err());
        assert!(super::request(&args(&["op"])).is_err());
    }

    #[test]
    fn malformed_invocations_are_rejected_locally() {
        for bad in [
            vec!["set", "property:zoom", "value"],
            vec!["move", "clip:a", "frame", "3"],
            vec!["move", "clip:a", "--frame"],
            vec!["invoke"],
            vec!["bogus"],
        ] {
            assert!(request(&args(&bad)).is_err(), "{bad:?}");
        }
    }
}
