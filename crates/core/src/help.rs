//! User guides shared by the command-line adapters and their JSONL protocol.

use std::process::ExitCode;

use serde_json::{Value, json};

use crate::CommandError;

const GUIDES: &[(&str, &str, &str)] = &[
    (
        "splat",
        "Gerar um splat a partir de uma imagem",
        include_str!("../../../docs/guides/splat.md"),
    ),
    (
        "camera",
        "Aproximar a câmera de um splat",
        include_str!("../../../docs/guides/camera.md"),
    ),
    (
        "preview",
        "Rever só um trecho",
        include_str!("../../../docs/guides/preview.md"),
    ),
];

fn topics() -> Value {
    json!(
        GUIDES
            .iter()
            .map(|(id, title, _)| json!({ "id": id, "title": title }))
            .collect::<Vec<_>>()
    )
}

fn markdown(topic: Option<&str>) -> Result<&'static str, CommandError> {
    let Some(topic) = topic else {
        return Ok(include_str!("../../../docs/guides/README.md"));
    };
    GUIDES
        .iter()
        .find(|(id, _, _)| *id == topic)
        .map(|(_, _, doc)| *doc)
        .ok_or_else(|| {
            CommandError::new(
                "unknown_topic",
                format!("Unknown help topic `{topic}`. Run `help` to list topics."),
            )
            .details(json!({ "topics": topics() }))
        })
}

/// Returns help without accessing any project, file system or backend.
pub fn guide(topic: Option<&str>) -> Result<Value, CommandError> {
    let markdown = markdown(topic)?;
    Ok(match topic {
        Some(id) => json!({ "topic": id, "markdown": markdown }),
        None => json!({ "topics": topics(), "markdown": markdown }),
    })
}

/// Handles the standalone help invocation before an adapter connects to any backend.
pub fn cli(args: &[String], usage: &str) -> Option<ExitCode> {
    let (command, rest) = args.split_first()?;
    if !matches!(command.as_str(), "help" | "--help" | "-h") {
        return None;
    }
    let topic = match rest {
        [] => None,
        [topic] => Some(topic.as_str()),
        _ => {
            eprintln!("Usage: help [topic]");
            return Some(ExitCode::from(2));
        }
    };
    Some(match markdown(topic) {
        Ok(doc) => {
            if topic.is_none() {
                println!("{usage}\n");
            }
            print!("{doc}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{}", error.message);
            ExitCode::from(2)
        }
    })
}
