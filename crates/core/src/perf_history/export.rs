//! Export of the history to a local JSON file: the typed errors and the safe write.
//!
//! The document itself is [`Snapshot::to_document`](super::Snapshot::to_document). This file only
//! decides where and how it lands: an explicit destination, written to a temporary sibling,
//! synced and renamed, so a failure leaves either the old file or none, never half a document.
//! Nothing here touches the project, and nothing exports by itself.

use std::fs;
use std::io::Write;
use std::path::Path;

use serde_json::{Value, json};

use crate::state::CommandError;

/// Why a history operation was refused. Observable through [`CommandError`] codes.
#[derive(Debug, Clone, PartialEq)]
pub enum HistoryError {
    /// Nothing was ever recorded (or it was cleared while stopped).
    NoRecording,
    /// The selected instant is not inside the window being read.
    OutsideWindow {
        at_ms: f64,
        start_ms: f64,
        end_ms: f64,
    },
    /// No destination was given.
    ExportPathMissing,
    /// The destination is a directory.
    ExportIsDirectory,
    /// The destination's folder does not exist.
    ExportParentMissing,
    /// The file could not be written; `kind` is the OS error kind, never a path.
    ExportWriteFailed { kind: String },
}

impl HistoryError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoRecording => "perf_history_empty",
            Self::OutsideWindow { .. } => "perf_history_outside_window",
            Self::ExportPathMissing => "perf_history_export_path_missing",
            Self::ExportIsDirectory => "perf_history_export_is_directory",
            Self::ExportParentMissing => "perf_history_export_parent_missing",
            Self::ExportWriteFailed { .. } => "perf_history_export_failed",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::NoRecording => {
                "There is no history: start recording first (nothing before that is recoverable)"
                    .to_string()
            }
            Self::OutsideWindow { .. } => {
                "The selected instant is outside the 60-second window being read".to_string()
            }
            Self::ExportPathMissing => "Name the file to write the diagnostic to".to_string(),
            Self::ExportIsDirectory => {
                "The destination is a folder: name a file inside it".to_string()
            }
            Self::ExportParentMissing => {
                "The destination's folder does not exist; create it first".to_string()
            }
            Self::ExportWriteFailed { kind } => {
                format!("The diagnostic could not be written ({kind}); nothing was changed")
            }
        }
    }
}

impl From<HistoryError> for CommandError {
    fn from(error: HistoryError) -> Self {
        let details = match &error {
            HistoryError::OutsideWindow {
                at_ms,
                start_ms,
                end_ms,
            } => Some(
                json!({ "at_ms": at_ms, "window_start_ms": start_ms, "window_end_ms": end_ms }),
            ),
            HistoryError::ExportWriteFailed { kind } => Some(json!({ "io_kind": kind })),
            _ => None,
        };
        let mapped = CommandError::new(error.code(), error.message()).target(super::TARGET);
        match details {
            Some(details) => mapped.details(details),
            None => mapped,
        }
    }
}

/// What built this binary, when known: version, profile and the commit if the build set
/// `TESSEL_GIT_SHA`. Unknown is `null`, never a guess.
pub fn build_info() -> Value {
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "git_sha": option_env!("TESSEL_GIT_SHA"),
    })
}

/// Writes `document` to `path` atomically and returns the byte count.
///
/// The destination must be a file path whose folder exists. The bytes go to a hidden sibling
/// (`create_new`, so nothing is overwritten by accident), are synced, and replace the target by
/// a rename; on any failure the sibling is removed and the target is as it was.
pub fn write_export(path: &Path, document: &Value) -> Result<usize, HistoryError> {
    if path.as_os_str().is_empty() {
        return Err(HistoryError::ExportPathMissing);
    }
    if path.is_dir() {
        return Err(HistoryError::ExportIsDirectory);
    }
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    if !parent.is_dir() {
        return Err(HistoryError::ExportParentMissing);
    }
    let name = path
        .file_name()
        .ok_or(HistoryError::ExportPathMissing)?
        .to_string_lossy()
        .into_owned();
    let bytes =
        serde_json::to_vec_pretty(document).map_err(|_| HistoryError::ExportWriteFailed {
            kind: "serialize".to_string(),
        })?;
    let temporary = parent.join(format!(".{name}.tmp-{}", std::process::id()));
    let failed = |error: std::io::Error| HistoryError::ExportWriteFailed {
        kind: format!("{:?}", error.kind()),
    };
    let written = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if let Err(error) = written {
        let _ = fs::remove_file(&temporary);
        return Err(failed(error));
    }
    Ok(bytes.len() + 1)
}
