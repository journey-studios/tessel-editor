//! Export of the history to a local JSON file: the typed errors and the safe write.
//!
//! The document itself is [`Snapshot::to_document`](super::Snapshot::to_document). This file only
//! decides where and how it lands: an explicit destination, written to a temporary sibling,
//! synced and published without replacement, so the target is never a partial document.
//! Nothing here touches the project, and nothing exports by itself.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

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
    /// The destination is a relative path: where it would land depends on the process, so an
    /// explicit destination must be absolute.
    ExportRelative,
    /// The destination is a directory.
    ExportIsDirectory,
    /// A file is already there. The diagnostic never replaces anything: name a new file.
    ExportExists,
    /// The hidden sibling this export writes first already exists (another writer, or what a
    /// process of the same id left behind). The destination may well be free: the other file is
    /// never touched, and nothing was exported.
    ExportTemporaryBusy,
    /// The destination's folder does not exist.
    ExportParentMissing,
    /// The file could not be written. `kind` is the OS error kind and `stage` where it happened
    /// (`serialize`, `create_temporary`, `write`, `sync`, or `publish`, the hard link that
    /// makes the file appear without replacing anything): never a path.
    ExportWriteFailed { kind: String, stage: &'static str },
}

impl HistoryError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoRecording => "perf_history_empty",
            Self::OutsideWindow { .. } => "perf_history_outside_window",
            Self::ExportPathMissing => "perf_history_export_path_missing",
            Self::ExportRelative => "perf_history_export_relative",
            Self::ExportIsDirectory => "perf_history_export_is_directory",
            Self::ExportExists => "perf_history_export_exists",
            Self::ExportTemporaryBusy => "perf_history_export_temporary_busy",
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
            Self::ExportRelative => {
                "Use an absolute path: a relative one would land wherever the editor was started"
                    .to_string()
            }
            Self::ExportIsDirectory => {
                "The destination is a folder: name a file inside it".to_string()
            }
            Self::ExportExists => {
                "A file already exists there; the diagnostic never replaces one. Name a new file"
                    .to_string()
            }
            Self::ExportTemporaryBusy => {
                "A temporary file of another export is in the way, so nothing was written (the destination itself may be free; the other file was not touched). Try again or name another file"
                    .to_string()
            }
            Self::ExportParentMissing => {
                "The destination's folder does not exist; create it first".to_string()
            }
            Self::ExportWriteFailed { kind, stage } if *stage == "publish" => format!(
                "The diagnostic could not be published ({kind}). Publishing without replacing needs hard links, which this folder's volume may not support: choose a folder on another volume. Nothing was changed"
            ),
            Self::ExportWriteFailed { kind, stage } => {
                format!("The diagnostic could not be written ({kind} at {stage}); nothing was changed")
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
            HistoryError::ExportWriteFailed { kind, stage } => {
                Some(json!({ "io_kind": kind, "stage": stage }))
            }
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

/// A document taken at one instant and the place it is going to, so the file can be written
/// somewhere else (a background thread) while the recorder keeps recording. The document is a
/// coherent copy: what is written is what the snapshot said when it was taken, frozen or not.
#[derive(Debug, Clone)]
pub struct PreparedExport {
    document: Value,
    path: PathBuf,
}

impl PreparedExport {
    /// Checks only what needs no disk (a name, an absolute path); everything that touches the
    /// filesystem happens in [`Self::write`].
    pub(super) fn new(document: Value, path: PathBuf) -> Result<Self, HistoryError> {
        check_shape(&path)?;
        Ok(Self { document, path })
    }

    /// Whether the document is the frozen snapshot.
    pub fn frozen(&self) -> bool {
        self.document["recording"]["frozen"] == json!(true)
    }

    /// The window the document covers.
    pub fn range(&self) -> Value {
        self.document["window"].clone()
    }

    /// Writes it: all the filesystem work (see [`write_export`]). Returns the byte count. No
    /// recorder, no clock and no window are involved, so it may run on any thread.
    pub fn write(self) -> Result<usize, HistoryError> {
        write_export(&self.path, &self.document)
    }
}

/// The part of the destination's validation that needs no disk.
fn check_shape(path: &Path) -> Result<(), HistoryError> {
    if path.as_os_str().is_empty() {
        return Err(HistoryError::ExportPathMissing);
    }
    if !path.is_absolute() {
        return Err(HistoryError::ExportRelative);
    }
    Ok(())
}

/// Writes `document` to a new file at `path` and returns the byte count.
///
/// The destination must be a new file at an absolute path whose folder exists. The bytes go to a
/// hidden sibling (`create_new`), are synced, and are linked to the target, which fails if
/// anything appeared there meanwhile: an existing file is refused, never replaced, and a
/// failure leaves no sibling behind and nothing changed. Only a sibling this call created is
/// ever removed: if the hidden name is taken (`perf_history_export_temporary_busy`) the other
/// file stays as it was. The publish step needs a volume with hard links; there is deliberately
/// no fallback to a rename, which could replace a file that appeared in the meantime.
pub fn write_export(path: &Path, document: &Value) -> Result<usize, HistoryError> {
    check_shape(path)?;
    if path.is_dir() {
        return Err(HistoryError::ExportIsDirectory);
    }
    // `symlink_metadata`: a dangling link is also "something there".
    if fs::symlink_metadata(path).is_ok() {
        return Err(HistoryError::ExportExists);
    }
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => return Err(HistoryError::ExportPathMissing),
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
            stage: "serialize",
        })?;
    let temporary = parent.join(format!(".{name}.tmp-{}", std::process::id()));
    let mut owns_temporary = false;
    let mut stage = "create_temporary";
    let written = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        owns_temporary = true;
        stage = "write";
        file.write_all(&bytes)?;
        file.write_all(b"\n")?;
        stage = "sync";
        file.sync_all()?;
        // No-clobber publish: unlike a rename, a hard link fails when the name exists.
        stage = "publish";
        fs::hard_link(&temporary, path)
    })();
    if owns_temporary {
        let _ = fs::remove_file(&temporary);
    }
    if let Err(error) = written {
        let kind = error.kind();
        if kind == std::io::ErrorKind::AlreadyExists {
            // Creating the sibling is where the hidden name was taken; at the link it is the
            // destination that appeared meanwhile.
            return Err(if stage == "create_temporary" {
                HistoryError::ExportTemporaryBusy
            } else {
                HistoryError::ExportExists
            });
        }
        return Err(HistoryError::ExportWriteFailed {
            kind: format!("{kind:?}"),
            stage,
        });
    }
    Ok(bytes.len() + 1)
}
