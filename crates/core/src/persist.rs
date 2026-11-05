//! Saving, opening, autosave and crash recovery of the project document.
//!
//! Only `std::fs` is used. Every write goes through a temporary file in the destination folder
//! and an atomic rename, so the previous valid document survives a failed or interrupted save.
//! Opening builds a candidate [`EditorState`] first; the caller swaps it in only on success.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::document::{self, canonical_bytes};
use crate::state::{CommandError, EditorState};

pub const DOCUMENT_SUFFIX: &str = ".tessel.json";
/// A project document larger than this is refused instead of being read into memory.
pub const MAX_DOCUMENT_BYTES: u64 = 256 * 1024 * 1024;
/// Autosave revisions kept per project.
pub const AUTOSAVE_KEEP: usize = 5;
const RECOVERY_DIR: &str = ".tessel/recovery";

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn io_error(code: &'static str, path: &Path, error: &std::io::Error) -> CommandError {
    CommandError::new(code, format!("`{}`: {error}", path.display())).target(path.to_string_lossy())
}

/// Writes `bytes` to `path` so that a reader sees either the old file or the complete new one.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), CommandError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| io_error("write_failed", parent, &error))?;
    let name = path.file_name().map_or_else(
        || "document".to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let temp = parent.join(format!(
        ".{name}.tmp-{}-{}",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let written = (|| {
        let mut file = File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)
    })();
    if let Err(error) = written {
        let _ = fs::remove_file(&temp);
        return Err(io_error("write_failed", path, &error));
    }
    Ok(())
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

/// Document folder of `path` (absolute).
pub fn document_folder(path: &Path) -> PathBuf {
    let path = absolute(path);
    path.parent().map(Path::to_path_buf).unwrap_or(path)
}

/// Saves the project to `path`. The folder becomes the workspace when none is set.
pub fn save(state: &mut EditorState, path: &Path) -> Result<Value, CommandError> {
    let path = absolute(path);
    if !path.to_string_lossy().ends_with(DOCUMENT_SUFFIX) {
        return Err(CommandError::new(
            "invalid_path",
            format!("A project document is named `*{DOCUMENT_SUFFIX}`"),
        )
        .target(path.to_string_lossy()));
    }
    let folder = document_folder(&path);
    let document = state.to_document(Some(&folder));
    let bytes = canonical_bytes(&document);
    atomic_write(&path, &bytes)?;
    state.project.document = Some(path.clone());
    if state.project.workspace.is_none() {
        state.project.workspace = Some(folder.clone());
    }
    state.project.saved_revision = Some(state.project.revision);
    // The explicit save supersedes anything autosaved before it.
    discard_recovery(&folder, &state.project.id);
    Ok(
        json!({ "action": "project.save", "path": path, "bytes": bytes.len(),
        "revision": state.project.revision, "project": state.project.id, "changed": true }),
    )
}

/// Reads and validates the document at `path` into a candidate project.
pub fn load(path: &Path) -> Result<EditorState, CommandError> {
    let path = absolute(path);
    let meta =
        fs::metadata(&path).map_err(|error| io_error("document_unreadable", &path, &error))?;
    if meta.len() > MAX_DOCUMENT_BYTES {
        return Err(CommandError::new(
            "document_too_large",
            format!(
                "The document is {} bytes; the limit is {MAX_DOCUMENT_BYTES}",
                meta.len()
            ),
        )
        .target(path.to_string_lossy()));
    }
    let mut text = String::new();
    File::open(&path)
        .and_then(|mut file| file.read_to_string(&mut text))
        .map_err(|error| io_error("document_unreadable", &path, &error))?;
    let doc: Value = serde_json::from_str(&text).map_err(|error| {
        CommandError::new(
            "invalid_document",
            format!("`{}` is not JSON: {error}", path.display()),
        )
        .target(path.to_string_lossy())
    })?;
    let folder = document_folder(&path);
    let mut state = EditorState::from_document(&doc, Some(&folder))
        .map_err(|error| error.target(path.to_string_lossy()))?;
    state.project.document = Some(path);
    state.project.workspace = Some(folder);
    state.project.saved_revision = Some(0);
    Ok(state)
}

fn recovery_dir(workspace: &Path, project_id: &str) -> PathBuf {
    workspace.join(RECOVERY_DIR).join(project_id)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Recovery {
    pub revision: u64,
    pub path: PathBuf,
    pub written_ms: u64,
}

impl Recovery {
    pub fn to_json(&self) -> Value {
        json!({ "revision": self.revision, "path": self.path, "written_ms": self.written_ms })
    }
}

/// Writes the current revision into the recovery area. A no-op (`changed:false`) while clean
/// or when this revision was already written. Never touches the explicit document.
pub fn autosave(state: &EditorState) -> Result<Value, CommandError> {
    let workspace = state.project.workspace.clone().ok_or_else(|| {
        CommandError::new(
            "no_workspace",
            "The project has no folder yet; save it once or create it with a `dir`",
        )
    })?;
    if !state.project.is_dirty() {
        return Ok(json!({ "action": "project.autosave", "changed": false, "reason": "clean" }));
    }
    let directory = recovery_dir(&workspace, &state.project.id);
    let target = directory.join(format!("rev-{:08}.json", state.project.revision));
    if target.is_file() {
        return Ok(
            json!({ "action": "project.autosave", "changed": false, "reason": "already written",
            "revision": state.project.revision }),
        );
    }
    let envelope = json!({
        "format": "tessel-recovery", "version": 1,
        "project": state.project.id, "revision": state.project.revision,
        "written_ms": now_ms(),
        "document": state.to_document(Some(&workspace)),
    });
    atomic_write(&target, &canonical_bytes(&envelope))?;
    prune(&directory);
    Ok(json!({ "action": "project.autosave", "path": target,
        "revision": state.project.revision, "changed": true }))
}

fn prune(directory: &Path) {
    let mut files = revisions_in(directory);
    files.sort_by_key(|recovery| recovery.revision);
    while files.len() > AUTOSAVE_KEEP {
        let oldest = files.remove(0);
        let _ = fs::remove_file(oldest.path);
    }
}

fn revisions_in(directory: &Path) -> Vec<Recovery> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let revision = name
                .strip_prefix("rev-")?
                .strip_suffix(".json")?
                .parse::<u64>()
                .ok()?;
            let written_ms = entry
                .metadata()
                .ok()?
                .modified()
                .ok()?
                .duration_since(UNIX_EPOCH)
                .ok()?
                .as_millis() as u64;
            Some(Recovery {
                revision,
                path: entry.path(),
                written_ms,
            })
        })
        .collect()
}

/// Autosaves of a project, newest first.
pub fn recoveries(workspace: &Path, project_id: &str) -> Vec<Recovery> {
    let mut found = revisions_in(&recovery_dir(workspace, project_id));
    found.sort_by_key(|recovery| std::cmp::Reverse(recovery.revision));
    found
}

/// Recoverable autosaves of the project stored in `document` (read without opening it fully).
pub fn recoveries_for_document(document_path: &Path) -> Vec<Recovery> {
    let folder = document_folder(document_path);
    let id = fs::read_to_string(document_path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|doc| doc["project"]["id"].as_str().map(str::to_string));
    id.map(|id| recoveries(&folder, &id)).unwrap_or_default()
}

/// Loads an autosave (the newest by default) as a candidate project. It stays *dirty* and keeps
/// the original document path, so recovering never overwrites the explicit save.
pub fn recover(document_path: &Path, revision: Option<u64>) -> Result<EditorState, CommandError> {
    let document_path = absolute(document_path);
    let folder = document_folder(&document_path);
    let original = load(&document_path)?;
    let candidates = recoveries(&folder, &original.project.id);
    let chosen = match revision {
        Some(revision) => candidates.iter().find(|r| r.revision == revision),
        None => candidates.first(),
    }
    .ok_or_else(|| {
        CommandError::new("no_recovery", "There is no autosaved revision to recover")
            .target(document_path.to_string_lossy())
    })?;
    let text = fs::read_to_string(&chosen.path)
        .map_err(|error| io_error("document_unreadable", &chosen.path, &error))?;
    let envelope: Value = serde_json::from_str(&text).map_err(|error| {
        CommandError::new(
            "invalid_document",
            format!("The autosave is damaged: {error}"),
        )
    })?;
    if envelope["format"] != "tessel-recovery" {
        return Err(CommandError::new(
            "invalid_document",
            "The autosave is not a Tessel recovery file",
        ));
    }
    let mut state = EditorState::from_document(&envelope["document"], Some(&folder))?;
    state.project.document = Some(document_path);
    state.project.workspace = Some(folder);
    // Differs from `saved_revision`, so the recovered project reports itself as unsaved.
    state.project.saved_revision = Some(0);
    state.project.revision = 1;
    Ok(state)
}

pub fn discard_recovery(workspace: &Path, project_id: &str) {
    let _ = fs::remove_dir_all(recovery_dir(workspace, project_id));
}

/// Shortcut used by tests and the CLI: schema constants of the written file.
pub fn describe(path: &Path) -> Result<Value, CommandError> {
    let state = load(path)?;
    Ok(json!({
        "format": document::FORMAT, "schema_version": document::SCHEMA_VERSION,
        "project": state.project_name, "id": state.project.id,
        "sequences": state.all_sequences().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        "assets": state.library.items.len(),
        "recoveries": recoveries_for_document(path).iter().map(Recovery::to_json).collect::<Vec<_>>(),
    }))
}
