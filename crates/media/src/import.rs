//! Batch import (P05): probe by content, copy or reference, dedup, relink.
//!
//! Two phases on purpose. `prepare` is the slow part (probe, hash, copy) and only touches the
//! file system, so it runs as a cancellable job off the UI thread and never sees the project.
//! `commit` applies the results to the project in ONE transaction (one undo unit). A failure of
//! one file is a per-file result; the rest still import (documented partial success).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tessel_editor_core::assets::{Asset, Identity, MediaKind, Storage};
use tessel_editor_core::jobs::{JobContext, JobError};
use tessel_editor_core::sharp::{Capture, Choice};
use tessel_editor_core::state::Command;
use tessel_editor_core::{CommandError, EditorState};

use crate::fsx::{self, Staging, hash_file, io_error, safe_file_name};
use crate::probe::{Probe, probe_with};
use crate::tools::Tools;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportMode {
    /// Copy into `<workspace>/media/<hash>/` (default: the project stays self-contained).
    Copy,
    /// Keep the original where it is and record its path (large archives).
    Reference,
}

impl ImportMode {
    pub fn parse(label: &str) -> Option<Self> {
        match label {
            "copy" => Some(Self::Copy),
            "reference" => Some(Self::Reference),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ImportRequest {
    pub paths: Vec<PathBuf>,
    pub mode: ImportMode,
    pub folder: Option<String>,
    /// What to do with the camera metadata of a SHARP splat (BL143).
    pub capture: Choice,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ItemStatus {
    Ready,
    /// The same bytes are already in the library (or the workspace): nothing new to register.
    Duplicate {
        of: String,
    },
    Failed,
}

#[derive(Debug, Clone)]
pub struct PreparedItem {
    pub source: PathBuf,
    pub status: ItemStatus,
    pub kind: Option<MediaKind>,
    pub name: String,
    pub asset: Option<Asset>,
    pub probe: Option<Value>,
    pub warnings: Vec<String>,
    /// The SHARP capture this import resolved for the file (`None`: not a SHARP file, or
    /// `capture: ignore`). A duplicate is compared with the library asset on it (BL143.6).
    pub capture: Option<Capture>,
    pub error: Option<JobError>,
}

impl PreparedItem {
    fn failed(source: &Path, error: JobError) -> Self {
        Self {
            source: source.to_path_buf(),
            status: ItemStatus::Failed,
            kind: None,
            name: display_name(source),
            asset: None,
            probe: None,
            warnings: Vec::new(),
            capture: None,
            error: Some(error),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "source": self.source,
            "status": match &self.status {
                ItemStatus::Ready => "ready", ItemStatus::Duplicate { .. } => "duplicate",
                ItemStatus::Failed => "failed",
            },
            "duplicate_of": match &self.status { ItemStatus::Duplicate { of } => json!(of), _ => Value::Null },
            "kind": self.kind.map(MediaKind::label), "name": self.name,
            "hash": self.asset.as_ref().and_then(|a| a.identity.hash.clone()),
            "size": self.asset.as_ref().and_then(|a| a.identity.size),
            "stored": self.asset.as_ref().and_then(|a| match &a.source {
                tessel_editor_core::assets::Source::File { path } => Some(path.clone()), _ => None }),
            "probe": self.probe, "warnings": self.warnings,
            "error": self.error.as_ref().map(JobError::to_json),
        })
    }
}

fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || "media".into(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// `<workspace>/media/<hash>/<safe name>`: what a managed copy is called.
pub fn managed_path(workspace: &Path, hash: &str, file_name: &str) -> PathBuf {
    workspace
        .join("media")
        .join(hash)
        .join(safe_file_name(file_name))
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Hashes already in the project (`hash → asset id`), so a duplicate is detected without
/// copying it again.
pub fn known_hashes(state: &EditorState) -> HashMap<String, String> {
    state
        .library
        .items
        .iter()
        .filter_map(|item| {
            let hash = item.asset.as_ref()?.identity.hash.clone()?;
            Some((hash, item.id.as_str().to_string()))
        })
        .collect()
}

fn build_asset(
    probe: Probe,
    path: &Path,
    hash: String,
    size: u64,
    storage: Storage,
    origin: &Path,
    file_name: &str,
) -> Asset {
    Asset {
        source: tessel_editor_core::assets::Source::File {
            path: path.to_string_lossy().into_owned(),
        },
        meta: probe.meta,
        identity: Identity {
            hash: Some(hash),
            size: Some(size),
            storage,
            origin: Some(origin.to_string_lossy().into_owned()),
            file_name: Some(file_name.to_string()),
            probe: Some(probe.summary),
        },
        decoded: None,
    }
}

fn prepare_one(
    tools: &Tools,
    workspace: &Path,
    mode: ImportMode,
    capture: Choice,
    known: &mut HashMap<String, String>,
    source: &Path,
    ctx: &JobContext,
) -> Result<PreparedItem, JobError> {
    let source = absolute(source);
    let probed = probe_with(tools, &source, capture)?;
    let name = display_name(&source);
    let mut item = PreparedItem {
        source: source.clone(),
        status: ItemStatus::Ready,
        kind: Some(probed.meta_kind()),
        name: name.clone(),
        asset: None,
        probe: Some(probed.to_json()),
        warnings: probed.warnings.clone(),
        capture: probed.meta.capture,
        error: None,
    };
    let size = probed.size;
    // Streaming progress is in bytes of this file; the job's total counts files.
    let (hash, stored_path, storage) = match mode {
        ImportMode::Reference => {
            let (hash, _) = hash_file(&source, ctx.token(), |_| {})?;
            (hash, source.clone(), Storage::Reference)
        }
        ImportMode::Copy => {
            let media = workspace.join("media");
            let staging = Staging::create(&media, "import")?;
            let staged = staging.path().join(safe_file_name(&name));
            fsx::ensure_space(&media, size, 64 << 20)?;
            let (hash, copied) = fsx::copy_file_hashed(&source, &staged, ctx.token(), |_| {})?;
            if copied != size {
                return Err(JobError::new(
                    "integrity_error",
                    format!(
                        "`{}` changed while it was being copied ({size} → {copied} bytes)",
                        source.display()
                    ),
                ));
            }
            let destination = managed_path(workspace, &hash, &name);
            if let Some(existing) = known.get(&hash) {
                item.status = ItemStatus::Duplicate {
                    of: existing.clone(),
                };
                item.asset = None;
                return Ok(item);
            }
            if destination.exists() {
                // Bytes are already in the workspace (an earlier import that was undone): reuse.
                let same = fs::metadata(&destination).is_ok_and(|meta| meta.len() == size);
                if !same {
                    return Err(JobError::new(
                        "integrity_error",
                        format!(
                            "`{}` exists with a different size; refusing to overwrite",
                            destination.display()
                        ),
                    ));
                }
            } else {
                let folder = destination.parent().unwrap_or(&media);
                fs::create_dir_all(folder).map_err(|error| io_error(folder, &error))?;
                fs::rename(&staged, &destination)
                    .map_err(|error| io_error(&destination, &error))?;
            }
            (hash, destination, Storage::Managed)
        }
    };
    if let Some(existing) = known.get(&hash) {
        item.status = ItemStatus::Duplicate {
            of: existing.clone(),
        };
        return Ok(item);
    }
    known.insert(hash.clone(), format!("pending:{}", source.display()));
    item.asset = Some(build_asset(
        probed,
        &stored_path,
        hash,
        size,
        storage,
        &source,
        &name,
    ));
    Ok(item)
}

trait KindOf {
    fn meta_kind(&self) -> MediaKind;
}

impl KindOf for Probe {
    fn meta_kind(&self) -> MediaKind {
        self.kind
    }
}

/// Probes, hashes and (in copy mode) copies every path. Originals are only ever read.
/// One bad file does not stop the batch; cancellation stops it at the next chunk and removes
/// the staging of the file in flight (files already published stay: they are valid managed
/// media that a later import deduplicates).
pub fn prepare(
    tools: &Tools,
    workspace: &Path,
    request: &ImportRequest,
    known: &HashMap<String, String>,
    ctx: &JobContext,
) -> Result<Vec<PreparedItem>, JobError> {
    let mut known = known.clone();
    ctx.stage("import", request.paths.len() as u64);
    let mut items = Vec::new();
    for path in &request.paths {
        ctx.check()?;
        let item = match prepare_one(
            tools,
            workspace,
            request.mode,
            request.capture,
            &mut known,
            path,
            ctx,
        ) {
            Ok(item) => item,
            Err(error) if error.is_cancelled() => return Err(error),
            Err(error) => PreparedItem::failed(path, error),
        };
        items.push(item);
        ctx.advance(1);
    }
    Ok(items)
}

fn asset_id_for(state: &EditorState, name: &str, taken: &[String]) -> String {
    let stem = Path::new(name)
        .file_stem()
        .map_or_else(|| "media".into(), |s| s.to_string_lossy().into_owned());
    let mut base: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .take(40)
        .collect();
    if base.trim_matches('-').is_empty() {
        base = "media".into();
    }
    let free = |candidate: &str| {
        state.library.item(candidate).is_none() && !taken.iter().any(|id| id == candidate)
    };
    if free(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|candidate| free(candidate))
        .unwrap_or(base)
}

/// A duplicate (same bytes already in the library) whose requested SHARP capture choice differs
/// from what the existing asset kept: `capture: auto` over an asset imported with `ignore`, or
/// `ignore` over one imported with its camera. Silently reporting `duplicate` would drop what the
/// caller asked for, so the file is refused (`capture_conflict`) and nothing about the existing
/// asset, its clips, the project or the undo history changes.
fn capture_conflict(state: &EditorState, item: &PreparedItem) -> Option<JobError> {
    let ItemStatus::Duplicate { of } = &item.status else {
        return None;
    };
    // Ids of the same batch (`pending:<path>`) are not in the library: same request, same choice.
    let existing = state.library.item(of)?.asset.as_ref()?;
    if existing.meta.capture == item.capture {
        return None;
    }
    let state_of = |capture: &Option<Capture>| if capture.is_some() { "with" } else { "without" };
    Some(
        JobError::new(
            "capture_conflict",
            format!(
                "`{}` has the same bytes as library asset `{of}`, which was imported {} its \
                 capture camera; this import asks for the other choice. Nothing changed: remove \
                 `{of}` first, or import the file with the same `capture` choice",
                item.source.display(),
                state_of(&existing.meta.capture),
            ),
        )
        .with_details(json!({
            "existing_asset": of,
            "existing_has_capture": existing.meta.capture.is_some(),
            "requested_has_capture": item.capture.is_some(),
            "requested": state_of(&item.capture),
        })),
    )
}

/// Registers every `Ready` item as ONE undo unit. Returns the per-file report.
pub fn commit(
    state: &mut EditorState,
    items: &[PreparedItem],
    folder: Option<&str>,
) -> Result<Value, CommandError> {
    let mut report = Vec::new();
    let mut taken: Vec<String> = Vec::new();
    let ready: Vec<&PreparedItem> = items
        .iter()
        .filter(|item| item.status == ItemStatus::Ready && item.asset.is_some())
        .collect();
    let mut ids = HashMap::new();
    if !ready.is_empty() {
        state.transaction("Import media", |state| {
            for item in &ready {
                let (Some(asset), Some(kind)) = (&item.asset, item.kind) else {
                    continue;
                };
                let id = asset_id_for(state, &item.name, &taken);
                taken.push(id.clone());
                state.execute(Command::AddAsset {
                    id: Some(id.clone()),
                    name: item.name.clone(),
                    kind,
                    asset: Box::new(asset.clone()),
                    folder: folder.map(str::to_string),
                })?;
                ids.insert(item.source.clone(), id);
            }
            Ok(())
        })?;
    }
    let mut imported = 0;
    let mut conflicts = 0;
    for item in items {
        let mut entry = item.to_json();
        if let Some(error) = capture_conflict(state, item) {
            entry["status"] = json!("failed");
            entry["duplicate_of"] = Value::Null;
            entry["error"] = error.to_json();
            conflicts += 1;
            report.push(entry);
            continue;
        }
        // A duplicate of a file earlier in the same batch points at the ID that file received.
        if let ItemStatus::Duplicate { of } = &item.status
            && let Some(source) = of.strip_prefix("pending:")
            && let Some(id) = ids.get(Path::new(source))
        {
            entry["duplicate_of"] = json!(id);
        }
        // `ids` is keyed by source path, so only the item that was actually registered counts.
        if item.status == ItemStatus::Ready
            && let Some(id) = ids.get(&item.source)
        {
            entry["id"] = json!(id);
            imported += 1;
        }
        report.push(entry);
    }
    let failed = items
        .iter()
        .filter(|item| item.status == ItemStatus::Failed)
        .count()
        + conflicts;
    let duplicates = items
        .iter()
        .filter(|item| matches!(item.status, ItemStatus::Duplicate { .. }))
        .count()
        - conflicts;
    Ok(
        json!({ "action": "media.import", "imported": imported, "duplicates": duplicates,
        "failed": failed, "items": report, "changed": imported > 0 }),
    )
}

/// Result of checking an asset's file.
#[derive(Debug, Clone, PartialEq)]
pub enum FileState {
    Ok,
    Missing,
    /// Size differs from the recorded one: the content certainly changed.
    Changed,
    /// Inline, demo or generated assets have nothing to verify.
    NotFile,
}

pub fn file_state(asset: &Asset) -> FileState {
    let tessel_editor_core::assets::Source::File { path } = &asset.source else {
        return FileState::NotFile;
    };
    let Ok(meta) = fs::metadata(path) else {
        return FileState::Missing;
    };
    match asset.identity.size {
        Some(size) if size != meta.len() => FileState::Changed,
        _ => FileState::Ok,
    }
}

/// `media.status`: every file-backed asset and whether its bytes are where the project says.
pub fn status(state: &EditorState) -> Value {
    let assets: Vec<Value> = state
        .library
        .items
        .iter()
        .filter_map(|item| {
            let asset = item.asset.as_ref()?;
            let file = file_state(asset);
            (file != FileState::NotFile).then(|| {
                json!({ "id": item.id, "name": item.name, "kind": item.kind.label(),
                    "state": match file { FileState::Ok => "ok", FileState::Missing => "missing",
                        FileState::Changed => "changed", FileState::NotFile => "n/a" },
                    "storage": asset.identity.storage.label(),
                    "path": match &asset.source { tessel_editor_core::assets::Source::File { path } => Some(path.clone()), _ => None },
                    "hash": asset.identity.hash, "size": asset.identity.size,
                    "thumbnail": state.thumbnail_report(item.id.as_str()) })
            })
        })
        .collect();
    let missing = assets.iter().filter(|a| a["state"] == "missing").count();
    let changed = assets.iter().filter(|a| a["state"] == "changed").count();
    json!({ "assets": assets, "missing": missing, "changed": changed })
}

/// Checks that `candidate` is the file the asset was imported from. Same name is not enough:
/// size and SHA-256 must match; with `accept_changed` a different file of the same kind and
/// duration is accepted and the identity is refreshed (reported as `content_changed`).
pub fn relink_candidate(
    tools: &Tools,
    asset: &Asset,
    candidate: &Path,
    accept_changed: bool,
    ctx: &JobContext,
) -> Result<(Asset, Vec<String>), JobError> {
    let candidate = absolute(candidate);
    let mut replacement = asset.clone();
    let mut warnings = Vec::new();
    // A splat imported with its capture is compared with the candidate's; one imported without
    // (`capture: ignore`, or a plain splat) stays without.
    let capture = if asset.meta.capture.is_some() {
        Choice::Auto
    } else {
        Choice::Ignore
    };
    let probed = probe_with(tools, &candidate, capture)?;
    let (hash, size) = hash_file(&candidate, ctx.token(), |_| {})?;
    let same_bytes = asset.identity.hash.as_deref() == Some(hash.as_str());
    if !same_bytes {
        let old = &asset.meta;
        let new = &probed.meta;
        let comparable = old.duration == new.duration
            && old.width == new.width
            && old.height == new.height
            && old.sample_rate == new.sample_rate
            && old.channels == new.channels
            && old.splat_count == new.splat_count
            && old.capture == new.capture;
        if !accept_changed || !comparable {
            return Err(JobError::new(
                "relink_mismatch",
                format!(
                    "`{}` is not the file the project was built with ({})",
                    candidate.display(),
                    if comparable {
                        "same streams but different bytes: pass accept_changed"
                    } else {
                        "duration, size or streams differ"
                    }
                ),
            )
            .with_details(
                json!({ "expected_hash": asset.identity.hash, "found_hash": hash,
                "expected_size": asset.identity.size, "found_size": size }),
            ));
        }
        warnings.push(
            "content_changed: the file differs byte-wise; identity was refreshed".to_string(),
        );
    }
    replacement.source = tessel_editor_core::assets::Source::File {
        path: candidate.to_string_lossy().into_owned(),
    };
    replacement.identity.hash = Some(hash);
    replacement.identity.size = Some(size);
    replacement.identity.storage = Storage::Reference;
    replacement.identity.probe = Some(probed.summary);
    replacement.decoded = None;
    Ok((replacement, warnings))
}

/// Finds a file for each missing asset under `folder` (bounded walk): candidates are those with
/// the recorded size, confirmed by hash. Returns `(asset id, new path)` for every match.
pub fn search_folder(
    state: &EditorState,
    folder: &Path,
    ctx: &JobContext,
) -> Result<Vec<(String, PathBuf)>, JobError> {
    const MAX_FILES: usize = 100_000;
    let wanted: Vec<(String, u64, Option<String>)> = state
        .library
        .items
        .iter()
        .filter_map(|item| {
            let asset = item.asset.as_ref()?;
            if file_state(asset) != FileState::Missing {
                return None;
            }
            Some((
                item.id.as_str().to_string(),
                asset.identity.size?,
                asset.identity.hash.clone(),
            ))
        })
        .collect();
    let mut found = Vec::new();
    let mut stack = vec![folder.to_path_buf()];
    let mut visited = 0usize;
    ctx.stage("search", wanted.len() as u64);
    while let Some(dir) = stack.pop() {
        ctx.check()?;
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            visited += 1;
            if visited > MAX_FILES {
                return Ok(found);
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            // Symlinks are not followed: a link loop must not hang the search.
            if kind.is_dir() {
                stack.push(path);
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            for (id, size, hash) in &wanted {
                if meta.len() != *size
                    || found.iter().any(|(done, _): &(String, PathBuf)| done == id)
                {
                    continue;
                }
                let (candidate_hash, _) = hash_file(&path, ctx.token(), |_| {})?;
                if hash
                    .as_deref()
                    .is_none_or(|expected| expected == candidate_hash)
                {
                    found.push((id.clone(), path.clone()));
                    ctx.advance(1);
                }
            }
        }
    }
    Ok(found)
}
