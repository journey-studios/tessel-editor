//! Consolidated project packages (P07): a folder or a `.tesselpack` (ZIP64) holding the
//! document, every media file it needs and a manifest with sizes and SHA-256 hashes.
//!
//! ```text
//! project.tessel.json        document with paths relative to the package
//! manifest.json              files, sizes, hashes, completeness, runtime requirements
//! media/<sha256>/<name>      one copy per distinct content
//! media/seq-<asset>/<frame>  frame sequences (each frame listed in the manifest)
//! scenes/<sceneId>/<path>    files of directed scenes, at the paths their bundles declare
//! ```
//!
//! Scene files are carried by `scene_resources::pack` (verified against the bundle's hashes); a
//! package is complete only when nothing a scene names is left out.
//!
//! Creation copies and verifies in a staging area and publishes only when everything checked
//! out; a failure or cancellation leaves the sources and any previous package untouched.
//! Restore validates every path and hash *before* publishing, and the result opens with no
//! access to the original files.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tessel_editor_core::assets::{Source, expand_pattern};
use tessel_editor_core::document::{SPARK_VERSION, THREE_VERSION, checked_canonical_bytes};
use tessel_editor_core::jobs::{CancelToken, JobContext, JobError};
use tessel_editor_core::state::Command;
use tessel_editor_core::{CommandError, EditorState, persist};

use crate::fsx::{self, Staging, copy_file_hashed, hash_file, io_error, safe_file_name};
use crate::import::{FileState, file_state};
use crate::scene_resources::pack;
use crate::zip::{Entry, ZipReader, ZipWriter, check_entry_name};

pub const DOCUMENT_NAME: &str = "project.tessel.json";
pub const MANIFEST_NAME: &str = "manifest.json";
pub const MANIFEST_FORMAT: &str = "tessel-package";
pub const MANIFEST_VERSION: u64 = 1;
/// Package paths are rooted here while the document is encoded: only the relative part is kept.
const VIRTUAL_ROOT: &str = "/tessel-package-root";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageFormat {
    Directory,
    Archive,
}

impl PackageFormat {
    pub fn parse(label: &str) -> Option<Self> {
        match label {
            "directory" | "dir" => Some(Self::Directory),
            "archive" | "zip" | "tesselpack" => Some(Self::Archive),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PackageOptions {
    pub format: PackageFormat,
    /// Sequences to include with the closure of what they use; `None` packs the whole project.
    pub sequences: Option<Vec<String>>,
    /// Pack even when some media is missing; the manifest then says `complete: false`.
    pub allow_partial: bool,
    pub replace: bool,
}

/// Inputs of a package that are not part of the project: where to read the files of a scene that
/// has not been adopted into the workspace yet. Additive to [`PackageOptions`] so existing
/// callers keep compiling; `PackageInputs::default()` is "use the managed bases only".
#[derive(Debug, Clone, Default)]
pub struct PackageInputs {
    /// Scene ID → absolute folder the caller authorizes for this package. Takes precedence over
    /// the managed base `<workspace>/scenes/<id>`. Not stored anywhere: the package carries the
    /// files, and a restored project uses its own root.
    pub scene_bases: BTreeMap<String, PathBuf>,
}

/// One file the package will contain.
#[derive(Debug, Clone)]
struct Planned {
    entry: String,
    source: PathBuf,
    expected_hash: Option<String>,
    size: u64,
    assets: Vec<String>,
}

#[derive(Debug)]
struct Plan {
    state: EditorState,
    files: Vec<Planned>,
    missing: Vec<Value>,
    warnings: Vec<String>,
    /// Which scenes' files go in, and what is reported for the ones that cannot.
    scenes: pack::ScenePlan,
    /// Asset ID → entry names, to rewrite the document.
    rewrites: Vec<(String, Rewrite)>,
}

#[derive(Debug)]
enum Rewrite {
    File(String),
    Sequence(String),
}

fn invalid_request(message: impl Into<String>) -> JobError {
    JobError::new("invalid_request", message)
}

/// Selects sequences and the assets they need (transitively through scenes → splats).
fn select(state: &EditorState, sequences: Option<&[String]>) -> Result<EditorState, JobError> {
    let mut selected = state.clone();
    let Some(wanted) = sequences else {
        return Ok(selected);
    };
    if wanted.is_empty() {
        return Err(invalid_request("`sequences` cannot be empty"));
    }
    for id in wanted {
        if state.sequence(id).is_none() {
            return Err(CommandError::unknown_element("sequence", id).into());
        }
    }
    selected
        .execute(Command::ActivateSequence {
            sequence: wanted[0].clone(),
        })
        .map_err(JobError::from)?;
    let others: Vec<String> = state
        .all_sequences()
        .map(|timeline| timeline.id.as_str().to_string())
        .filter(|id| !wanted.contains(id))
        .collect();
    for id in others {
        selected
            .execute(Command::RemoveSequence { sequence: id })
            .map_err(JobError::from)?;
    }
    let mut needed: Vec<String> = selected
        .all_sequences()
        .flat_map(|timeline| timeline.clips.iter())
        .filter_map(|clip| clip.media.map(|id| id.as_str().to_string()))
        .collect();
    let mut index = 0;
    while index < needed.len() {
        let requires: Vec<String> = selected
            .library
            .item(&needed[index])
            .and_then(|item| item.asset.as_ref())
            .map(|asset| {
                asset
                    .meta
                    .requires
                    .iter()
                    .map(|id| id.as_str().to_string())
                    .collect()
            })
            .unwrap_or_default();
        for id in requires {
            if !needed.contains(&id) {
                needed.push(id);
            }
        }
        index += 1;
    }
    selected
        .library
        .items
        .retain(|item| item.asset.is_none() || needed.iter().any(|id| id == item.id.as_str()));
    Ok(selected)
}

/// `verify` makes the plan read and hash every scene file (see `pack::plan`).
fn plan(
    state: &EditorState,
    options: &PackageOptions,
    inputs: &PackageInputs,
    verify: Option<&CancelToken>,
) -> Result<Plan, JobError> {
    let selected = select(state, options.sequences.as_deref())?;
    let (mut files, mut missing, mut warnings, mut rewrites) =
        (Vec::<Planned>::new(), Vec::new(), Vec::new(), Vec::new());
    for item in &selected.library.items {
        let Some(asset) = &item.asset else { continue };
        let id = item.id.as_str().to_string();
        match &asset.source {
            Source::File { path } => {
                if file_state(asset) != FileState::Ok {
                    missing.push(json!({ "asset": id, "path": path,
                        "state": if file_state(asset) == FileState::Missing { "missing" } else { "changed" } }));
                    continue;
                }
                let size = fs::metadata(path).map_or(0, |meta| meta.len());
                let name =
                    safe_file_name(asset.identity.file_name.as_deref().unwrap_or_else(|| {
                        Path::new(path)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("file")
                    }));
                // Without a recorded hash the content address is only known after copying, so
                // the entry is named after the asset until then; the manifest keeps the hash.
                let folder = asset
                    .identity
                    .hash
                    .clone()
                    .unwrap_or_else(|| format!("unhashed-{id}"));
                let entry = format!("media/{folder}/{name}");
                if let Some(existing) = files.iter_mut().find(|file| file.entry == entry) {
                    existing.assets.push(id.clone());
                } else {
                    files.push(Planned {
                        entry: entry.clone(),
                        source: PathBuf::from(path),
                        expected_hash: asset.identity.hash.clone(),
                        size,
                        assets: vec![id.clone()],
                    });
                }
                if asset.identity.hash.is_none() {
                    warnings.push(format!(
                        "`{id}` was never hashed: it was hashed while packing"
                    ));
                }
                rewrites.push((id, Rewrite::File(entry)));
            }
            Source::FrameSequence {
                pattern,
                first,
                count,
            } => {
                let folder = format!("media/seq-{id}");
                let mut gone = false;
                for index in *first..first + count {
                    let Some(frame) = expand_pattern(pattern, index) else {
                        continue;
                    };
                    let Ok(meta) = fs::metadata(&frame) else {
                        gone = true;
                        break;
                    };
                    let name = safe_file_name(
                        Path::new(&frame)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("frame"),
                    );
                    files.push(Planned {
                        entry: format!("{folder}/{name}"),
                        source: PathBuf::from(&frame),
                        expected_hash: None,
                        size: meta.len(),
                        assets: vec![id.clone()],
                    });
                }
                if gone {
                    missing.push(json!({ "asset": id, "path": pattern, "state": "missing" }));
                    files.retain(|file| !file.entry.starts_with(&format!("{folder}/")));
                    continue;
                }
                rewrites.push((id, Rewrite::Sequence(folder)));
            }
            Source::Demo(_) | Source::Inline => {}
        }
    }
    // Directed scenes travel inside the document; the files they name by relative path travel
    // as `scenes/<id>/…` when a base exists and verifies, and are reported like any other
    // missing media otherwise, so a package is never called complete while a scene points at
    // files it does not hold.
    let scenes = pack::plan(&selected, &inputs.scene_bases, verify)?;
    missing.extend(scenes.missing.iter().cloned());
    Ok(Plan {
        state: selected,
        files,
        missing,
        warnings,
        scenes,
        rewrites,
    })
}

fn has_scene_resources(planned: &Plan) -> bool {
    pack::has_not_packaged(&planned.missing)
}

/// The sentence both the preflight blocker and the create error use.
const SCENE_RESOURCES_MESSAGE: &str = "A directed scene names resource files this package cannot carry (no resource base to read them from, or sections nothing transports yet); allow a partial package to keep the document and its scenes";

/// What `create` would do, without writing anything: files, bytes, missing dependencies.
pub fn preflight(
    state: &EditorState,
    options: &PackageOptions,
    destination: &Path,
) -> Result<Value, JobError> {
    preflight_with(
        state,
        options,
        &PackageInputs::default(),
        destination,
        &CancelToken::new(),
    )
}

/// [`preflight`] with the extra inputs of a package and a token. It reads and hashes every file
/// of every scene that would be carried, so it is a job body, not something for a UI thread.
pub fn preflight_with(
    state: &EditorState,
    options: &PackageOptions,
    inputs: &PackageInputs,
    destination: &Path,
    token: &CancelToken,
) -> Result<Value, JobError> {
    let planned = plan(state, options, inputs, Some(token))?;
    let bytes: u64 = planned
        .files
        .iter()
        .map(|file| file.size)
        .sum::<u64>()
        .saturating_add(planned.scenes.bytes);
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let free = fsx::free_bytes(parent);
    let mut blockers = Vec::new();
    if has_scene_resources(&planned) && !options.allow_partial {
        blockers.push(
            json!({ "code": "unsupported", "message": SCENE_RESOURCES_MESSAGE,
            "assets": planned.missing }),
        );
    } else if !planned.missing.is_empty() && !options.allow_partial {
        blockers.push(json!({ "code": "missing_media", "message": "Some media files are missing or changed; relink them or allow a partial package",
            "assets": planned.missing }));
    }
    if destination.exists() && !options.replace {
        blockers.push(json!({ "code": "destination_exists", "message": format!("`{}` already exists", destination.display()) }));
    }
    if let Some(free) = free
        && free < bytes.saturating_add(64 << 20)
    {
        blockers.push(json!({ "code": "disk_full", "message": "Not enough free space for the package", "needed": bytes, "free": free }));
    }
    Ok(
        json!({ "files": planned.files.len(), "scene_files": scene_file_count(&planned),
        "bytes": bytes, "free_bytes": free,
        "missing": planned.missing, "warnings": planned.warnings, "blockers": blockers,
        "complete": planned.missing.is_empty(), "ok": blockers.is_empty() }),
    )
}

fn scene_file_count(planned: &Plan) -> usize {
    planned
        .scenes
        .carry
        .iter()
        .filter_map(|carry| carry.manifest.transport_plan().ok())
        .map(|plan| plan.items.len())
        .sum()
}

/// Files whose content hash was never recorded (assets registered by path) get their content
/// address before copying, so identical bytes still share one entry.
fn resolve_unhashed(planned: &mut Plan, ctx: &JobContext) -> Result<(), JobError> {
    let pending: Vec<usize> = (0..planned.files.len())
        .filter(|index| planned.files[*index].entry.starts_with("media/unhashed-"))
        .collect();
    if pending.is_empty() {
        return Ok(());
    }
    ctx.stage("hash", pending.len() as u64);
    let mut merged: Vec<Planned> = Vec::new();
    let mut renamed: Vec<(String, String)> = Vec::new();
    for index in 0..planned.files.len() {
        let mut file = planned.files[index].clone();
        if pending.contains(&index) {
            let (hash, _) = hash_file(&file.source, ctx.token(), |_| {})?;
            let name = file.entry.rsplit('/').next().unwrap_or("file").to_string();
            let entry = format!("media/{hash}/{name}");
            renamed.push((file.entry.clone(), entry.clone()));
            file.entry = entry;
            ctx.advance(1);
        }
        match merged.iter_mut().find(|other| other.entry == file.entry) {
            Some(existing) => existing.assets.extend(file.assets),
            None => merged.push(file),
        }
    }
    planned.files = merged;
    for (_, rewrite) in &mut planned.rewrites {
        if let Rewrite::File(entry) = rewrite
            && let Some((_, new)) = renamed.iter().find(|(old, _)| old == entry)
        {
            *entry = new.clone();
        }
    }
    Ok(())
}

fn rewrite_document(
    planned: &Plan,
    hashes: &BTreeMap<String, String>,
) -> Result<Vec<u8>, JobError> {
    let mut packaged = planned.state.clone();
    let root = Path::new(VIRTUAL_ROOT);
    for (id, rewrite) in &planned.rewrites {
        let Some(asset) = packaged
            .library
            .items
            .iter_mut()
            .find(|item| item.id == id.as_str())
            .and_then(|item| item.asset.as_mut())
        else {
            continue;
        };
        match (rewrite, &mut asset.source) {
            (Rewrite::File(entry), Source::File { path }) => {
                *path = root.join(entry).to_string_lossy().into_owned();
                if let Some(hash) = hashes.get(entry) {
                    asset.identity.hash = Some(hash.clone());
                }
            }
            (Rewrite::Sequence(folder), Source::FrameSequence { pattern, .. }) => {
                let name = Path::new(pattern.as_str())
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("f%05d.png")
                    .to_string();
                *pattern = root.join(folder).join(name).to_string_lossy().into_owned();
            }
            _ => continue,
        }
        asset.identity.storage = tessel_editor_core::assets::Storage::Managed;
        // The original location is a development detail, not part of a portable package.
        asset.identity.origin = None;
        asset.decoded = None;
    }
    let mut doc = packaged.to_document(Some(root));
    // Autosave/recovery state and the machine's workspace are not part of the document anyway;
    // the restored project gets its own.
    doc["package"] = json!({ "created_from": planned.state.project.id });
    checked_canonical_bytes(&doc, tessel_editor_core::persist::MAX_DOCUMENT_BYTES)
        .map_err(JobError::from)
}

fn manifest(
    planned: &Plan,
    files: &[(String, u64, String, Vec<String>)],
    scene_files: &[Value],
    document_hash: &str,
    document_size: u64,
) -> Value {
    let mut listed: Vec<Value> = files
        .iter()
        .map(|(entry, size, sha, assets)| json!({ "path": entry, "size": size, "sha256": sha, "role": "media", "assets": assets }))
        .collect();
    listed.extend(scene_files.iter().cloned());
    listed.push(json!({ "path": DOCUMENT_NAME, "size": document_size, "sha256": document_hash, "role": "document" }));
    listed.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    json!({
        "format": MANIFEST_FORMAT, "version": MANIFEST_VERSION,
        "document": DOCUMENT_NAME, "schema_version": planned.state.document_schema_version(),
        "project": { "id": planned.state.project.id, "name": planned.state.project_name,
            "revision": planned.state.project.revision },
        "requires": { "spark": SPARK_VERSION, "three": THREE_VERSION },
        "complete": planned.missing.is_empty(),
        "missing": planned.missing, "warnings": planned.warnings,
        "files": listed,
    })
}

/// Builds the package at `destination`. `state` is a snapshot: edits made while this runs do not
/// change the package.
pub fn create(
    state: &EditorState,
    destination: &Path,
    options: &PackageOptions,
    ctx: &JobContext,
) -> Result<Value, JobError> {
    create_with(state, destination, options, &PackageInputs::default(), ctx)
}

/// [`create`] with the extra inputs of a package (`PackageInputs`).
pub fn create_with(
    state: &EditorState,
    destination: &Path,
    options: &PackageOptions,
    inputs: &PackageInputs,
    ctx: &JobContext,
) -> Result<Value, JobError> {
    // Scene files are verified while they are copied, which already makes any failure abort a
    // package that may not be partial. Only a partial package needs to know *beforehand* which
    // scenes can be carried, so only it pays for the extra read.
    let verify = options.allow_partial.then(|| ctx.token());
    let mut planned = plan(state, options, inputs, verify)?;
    if has_scene_resources(&planned) && !options.allow_partial {
        return Err(JobError::new("unsupported", SCENE_RESOURCES_MESSAGE)
            .with_details(json!({ "missing": planned.missing })));
    }
    if !planned.missing.is_empty() && !options.allow_partial {
        return Err(JobError::new(
            "preflight_failed",
            format!(
                "{} media file(s) are missing or changed; relink them or allow a partial package",
                planned.missing.len()
            ),
        )
        .with_details(json!({ "missing": planned.missing })));
    }
    if destination.exists() && !options.replace {
        return Err(JobError::new(
            "destination_exists",
            format!(
                "`{}` already exists; nothing was overwritten",
                destination.display()
            ),
        ));
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    resolve_unhashed(&mut planned, ctx)?;
    let total: u64 = planned
        .files
        .iter()
        .map(|file| file.size)
        .sum::<u64>()
        .saturating_add(planned.scenes.bytes);
    fs::create_dir_all(parent).map_err(|error| io_error(parent, &error))?;
    // An archive stages the scene files once before streaming them into the ZIP.
    let staged_twice = match options.format {
        PackageFormat::Archive => planned.scenes.bytes,
        PackageFormat::Directory => 0,
    };
    fsx::ensure_space(parent, total.saturating_add(staged_twice), 64 << 20)?;
    let staging = Staging::create(parent, "package")?;
    ctx.stage("copy", planned.files.len() as u64);
    let mut hashes: BTreeMap<String, String> = BTreeMap::new();
    let mut listed: Vec<(String, u64, String, Vec<String>)> = Vec::new();

    let archive_path = staging.path().join("package.tesselpack");
    let mut zip = match options.format {
        PackageFormat::Archive => {
            let file =
                File::create(&archive_path).map_err(|error| io_error(&archive_path, &error))?;
            Some(ZipWriter::new(BufWriter::with_capacity(1 << 20, file)))
        }
        PackageFormat::Directory => None,
    };
    let content_root = staging.path().join("content");
    if matches!(options.format, PackageFormat::Directory) {
        fs::create_dir_all(&content_root).map_err(|error| io_error(&content_root, &error))?;
    }
    for file in &planned.files {
        ctx.check()?;
        let (sha, size) = match &mut zip {
            Some(zip) => {
                let mut input =
                    File::open(&file.source).map_err(|error| io_error(&file.source, &error))?;
                zip.add_stream(&file.entry, &mut input, ctx.token(), &file.source, |_| {})?
            }
            None => {
                let target = content_root.join(&file.entry);
                let folder = target.parent().unwrap_or(&content_root);
                fs::create_dir_all(folder).map_err(|error| io_error(folder, &error))?;
                copy_file_hashed(&file.source, &target, ctx.token(), |_| {})?
            }
        };
        // A file that changed after it was imported must not be packed as if it were the same.
        if let Some(expected) = &file.expected_hash
            && expected != &sha
        {
            return Err(JobError::new(
                "integrity_error",
                format!(
                    "`{}` no longer matches the hash recorded at import",
                    file.source.display()
                ),
            )
            .with_details(json!({ "expected": expected, "found": sha, "assets": file.assets })));
        }
        hashes.insert(file.entry.clone(), sha.clone());
        listed.push((file.entry.clone(), size, sha, file.assets.clone()));
        ctx.advance(1);
    }
    let mut scene_written: Vec<pack::Written> = Vec::new();
    if !planned.scenes.carry.is_empty() {
        ctx.stage("scenes", planned.scenes.carry.len() as u64);
        let scene_parent = match options.format {
            PackageFormat::Directory => content_root.clone(),
            PackageFormat::Archive => staging.path().join("scene-files"),
        };
        for carry in &planned.scenes.carry {
            ctx.check()?;
            let (_, written) = pack::stage(carry, &scene_parent, ctx.token())?;
            if let Some(zip) = &mut zip {
                for file in &written {
                    let mut input =
                        File::open(&file.file).map_err(|error| io_error(&file.file, &error))?;
                    let (sha, size) =
                        zip.add_stream(&file.entry, &mut input, ctx.token(), &file.file, |_| {})?;
                    if sha != file.sha || size != file.size {
                        return Err(JobError::new(
                            "integrity_error",
                            format!("`{}` changed while it was added to the archive", file.entry),
                        ));
                    }
                }
            }
            scene_written.extend(written);
            ctx.advance(1);
        }
    }
    ctx.stage("document", 2);
    let document_bytes = rewrite_document(&planned, &hashes)?;
    let document_hash = fsx::hex(&Sha256::digest(&document_bytes));
    let manifest_json = manifest(
        &planned,
        &listed,
        &pack::manifest_entries(&scene_written),
        &document_hash,
        document_bytes.len() as u64,
    );
    let manifest_bytes = serde_json::to_vec_pretty(&manifest_json)
        .map_err(|e| JobError::new("io_error", e.to_string()))?;
    match zip {
        Some(mut zip) => {
            zip.add_bytes(DOCUMENT_NAME, &document_bytes)?;
            zip.add_bytes(MANIFEST_NAME, &manifest_bytes)?;
            let writer = zip.finish()?;
            let file = writer
                .into_inner()
                .map_err(|e| io_error(&archive_path, e.error()))?;
            file.sync_all()
                .map_err(|error| io_error(&archive_path, &error))?;
        }
        None => {
            fs::write(content_root.join(DOCUMENT_NAME), &document_bytes)
                .map_err(|error| io_error(&content_root, &error))?;
            fs::write(content_root.join(MANIFEST_NAME), &manifest_bytes)
                .map_err(|error| io_error(&content_root, &error))?;
        }
    }
    ctx.stage(
        "verify",
        manifest_json["files"]
            .as_array()
            .map_or(0, |f| f.len() as u64),
    );
    let staged = match options.format {
        PackageFormat::Archive => archive_path.clone(),
        PackageFormat::Directory => content_root.clone(),
    };
    let verified = verify_package(&staged, ctx)?;
    if destination.exists() {
        if destination.is_dir() {
            fs::remove_dir_all(destination)
        } else {
            fs::remove_file(destination)
        }
        .map_err(|error| io_error(destination, &error))?;
    }
    match options.format {
        PackageFormat::Archive => {
            fsx::publish_file(&archive_path, destination, false)?;
            drop(staging);
        }
        PackageFormat::Directory => {
            fs::rename(&content_root, destination)
                .map_err(|error| io_error(destination, &error))?;
            drop(staging);
        }
    }
    Ok(
        json!({ "action": "project.package", "path": destination, "format": match options.format {
            PackageFormat::Archive => "archive", PackageFormat::Directory => "directory" },
        "files": verified["files"], "scene_files": scene_written.len(), "bytes": total,
        "complete": planned.missing.is_empty(),
        "missing": planned.missing, "warnings": planned.warnings, "manifest": manifest_json["project"] }),
    )
}

/// A package on disk, either form, behind one interface.
enum PackageSource {
    Directory(PathBuf),
    Archive(Box<ZipReader>),
}

fn open_source(path: &Path) -> Result<PackageSource, JobError> {
    if path.is_dir() {
        return Ok(PackageSource::Directory(path.to_path_buf()));
    }
    ZipReader::open(path).map(|zip| PackageSource::Archive(Box::new(zip)))
}

fn read_small(source: &mut PackageSource, name: &str, limit: u64) -> Result<Vec<u8>, JobError> {
    use std::io::Read;
    let mut bytes = Vec::new();
    match source {
        PackageSource::Directory(root) => {
            let path = root.join(name);
            let size = fs::metadata(&path)
                .map_err(|_| JobError::new("invalid_package", format!("`{name}` is missing")))?
                .len();
            if size > limit {
                return Err(JobError::new(
                    "invalid_package",
                    format!("`{name}` is unreasonably large"),
                ));
            }
            File::open(&path)
                .and_then(|mut f| f.read_to_end(&mut bytes))
                .map_err(|e| io_error(&path, &e))?;
        }
        PackageSource::Archive(zip) => {
            let entry = zip
                .entries
                .iter()
                .find(|e| e.name == name)
                .cloned()
                .ok_or_else(|| JobError::new("invalid_package", format!("`{name}` is missing")))?;
            if entry.size > limit {
                return Err(JobError::new(
                    "invalid_package",
                    format!("`{name}` is unreasonably large"),
                ));
            }
            zip.open_entry(&entry)?
                .read_to_end(&mut bytes)
                .map_err(|e| io_error(Path::new(name), &e))?;
        }
    }
    Ok(bytes)
}

/// What the manifest promises, as `(path, size, sha256)`.
fn manifest_files(manifest: &Value) -> Result<Vec<(String, u64, String)>, JobError> {
    if manifest["format"] != MANIFEST_FORMAT {
        return Err(JobError::new(
            "invalid_package",
            "Not a Tessel package: the manifest format is wrong",
        ));
    }
    let version = manifest["version"].as_u64().unwrap_or(0);
    if version == 0 || version > MANIFEST_VERSION {
        return Err(JobError::new(
            "unsupported_schema",
            format!(
                "Package manifest version {version} is newer than this editor understands ({MANIFEST_VERSION})"
            ),
        ));
    }
    manifest["files"]
        .as_array()
        .ok_or_else(|| JobError::new("invalid_package", "The manifest lists no files"))?
        .iter()
        .map(|file| {
            let path = file["path"]
                .as_str()
                .ok_or_else(|| JobError::new("invalid_package", "A manifest file has no path"))?;
            check_entry_name(path)?;
            Ok((
                path.to_string(),
                file["size"].as_u64().ok_or_else(|| {
                    JobError::new("invalid_package", format!("`{path}` has no size"))
                })?,
                file["sha256"]
                    .as_str()
                    .ok_or_else(|| {
                        JobError::new("invalid_package", format!("`{path}` has no hash"))
                    })?
                    .to_string(),
            ))
        })
        .collect()
}

/// Streams every manifest file and compares size and SHA-256; also rejects files the manifest
/// does not list. Returns a report; any mismatch is `integrity_error`.
pub fn verify_package(path: &Path, ctx: &JobContext) -> Result<Value, JobError> {
    let mut source = open_source(path)?;
    let manifest: Value =
        serde_json::from_slice(&read_small(&mut source, MANIFEST_NAME, 64 << 20)?).map_err(
            |e| {
                JobError::new(
                    "invalid_package",
                    format!("The manifest is not valid JSON: {e}"),
                )
            },
        )?;
    let files = manifest_files(&manifest)?;
    ctx.stage("verify", files.len() as u64);
    let listed: Vec<&str> = files.iter().map(|(p, _, _)| p.as_str()).collect();
    let present: Vec<String> = match &source {
        PackageSource::Archive(zip) => zip.entries.iter().map(|e| e.name.clone()).collect(),
        PackageSource::Directory(root) => walk(root, root)?,
    };
    for name in &present {
        if name != MANIFEST_NAME && !listed.contains(&name.as_str()) {
            return Err(JobError::new(
                "integrity_error",
                format!("`{name}` is in the package but not in the manifest"),
            ));
        }
    }
    for (name, size, sha) in &files {
        ctx.check()?;
        let (found_sha, found_size) = match &mut source {
            PackageSource::Directory(root) => {
                hash_file(&root.join(name), ctx.token(), |_| {}).map_err(|e| mismatch(name, e))?
            }
            PackageSource::Archive(zip) => {
                let entry: Entry = zip
                    .entries
                    .iter()
                    .find(|e| &e.name == name)
                    .cloned()
                    .ok_or_else(|| {
                        JobError::new(
                            "integrity_error",
                            format!("`{name}` is listed but missing from the archive"),
                        )
                    })?;
                let mut reader = zip.open_entry(&entry)?;
                let result = fsx::copy_hashed(
                    &mut reader,
                    &mut std::io::sink(),
                    ctx.token(),
                    Path::new(name),
                    |_| {},
                )?;
                drop(reader);
                result
            }
        };
        if found_size != *size || &found_sha != sha {
            return Err(JobError::new("integrity_error", format!("`{name}` does not match the manifest"))
                .with_details(json!({ "path": name, "expected_size": size, "found_size": found_size, "expected_sha256": sha, "found_sha256": found_sha })));
        }
        ctx.advance(1);
    }
    Ok(
        json!({ "files": files.len(), "complete": manifest["complete"], "project": manifest["project"], "requires": manifest["requires"], "manifest": manifest }),
    )
}

fn mismatch(name: &str, error: JobError) -> JobError {
    if error.code == "asset_unreadable" {
        return JobError::new(
            "integrity_error",
            format!("`{name}` is listed but missing from the package"),
        );
    }
    error
}

fn walk(root: &Path, dir: &Path) -> Result<Vec<String>, JobError> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| io_error(dir, &e))? {
        let entry = entry.map_err(|e| io_error(dir, &e))?;
        let kind = entry.file_type().map_err(|e| io_error(dir, &e))?;
        if kind.is_symlink() {
            return Err(JobError::new(
                "invalid_package",
                format!("`{}` is a symbolic link", entry.path().display()),
            ));
        }
        if kind.is_dir() {
            out.extend(walk(root, &entry.path())?);
        } else if let Ok(relative) = entry.path().strip_prefix(root) {
            out.push(
                relative
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/"),
            );
        }
    }
    Ok(out)
}

/// Verifies `package`, then extracts it to `destination` (which must not exist) in staging and
/// publishes. The restored folder opens as a project whose workspace is `destination`.
pub fn restore(package: &Path, destination: &Path, ctx: &JobContext) -> Result<Value, JobError> {
    if destination.exists() {
        return Err(JobError::new(
            "destination_exists",
            format!(
                "`{}` already exists; choose a new folder",
                destination.display()
            ),
        ));
    }
    let report = verify_package(package, ctx)?;
    let manifest = &report["manifest"];
    let files = manifest_files(manifest)?;
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let needed: u64 = files.iter().map(|(_, size, _)| size).sum();
    fs::create_dir_all(parent).map_err(|e| io_error(parent, &e))?;
    fsx::ensure_space(parent, needed, 64 << 20)?;
    let staging = Staging::create(parent, "restore")?;
    let root = staging.path().join("content");
    fs::create_dir_all(&root).map_err(|e| io_error(&root, &e))?;
    ctx.stage("extract", files.len() as u64);
    let mut source = open_source(package)?;
    for (name, size, sha) in &files {
        ctx.check()?;
        let target = root.join(name);
        let folder = target.parent().unwrap_or(&root);
        fs::create_dir_all(folder).map_err(|e| io_error(folder, &e))?;
        // The path is already validated by `manifest_files`; this guards the join itself.
        if !target.starts_with(&root) {
            return Err(JobError::new(
                "invalid_package",
                format!("`{name}` escapes the destination"),
            ));
        }
        let (found_sha, found_size) = match &mut source {
            PackageSource::Directory(dir) => {
                copy_file_hashed(&dir.join(name), &target, ctx.token(), |_| {})?
            }
            PackageSource::Archive(zip) => {
                let entry = zip
                    .entries
                    .iter()
                    .find(|e| &e.name == name)
                    .cloned()
                    .ok_or_else(|| {
                        JobError::new(
                            "integrity_error",
                            format!("`{name}` is missing from the archive"),
                        )
                    })?;
                let mut reader = zip.open_entry(&entry)?;
                let mut out = File::create(&target).map_err(|e| io_error(&target, &e))?;
                let result = fsx::copy_hashed(&mut reader, &mut out, ctx.token(), &target, |_| {})?;
                out.sync_all().map_err(|e| io_error(&target, &e))?;
                result
            }
        };
        if found_size != *size || &found_sha != sha {
            return Err(JobError::new(
                "integrity_error",
                format!("`{name}` changed while it was extracted"),
            ));
        }
        ctx.advance(1);
    }
    // The restored document must really load, with every asset it names present.
    let document = root.join(DOCUMENT_NAME);
    let loaded = persist::load(&document).map_err(JobError::from)?;
    let mut unresolved = Vec::new();
    for item in &loaded.library.items {
        if let Some(asset) = &item.asset
            && matches!(file_state(asset), FileState::Missing)
        {
            unresolved.push(item.id.as_str().to_string());
        }
    }
    // The scenes' files must be in the package with the hash each bundle declares: this is what
    // proves the restored project opens with no access to the machine it was packed on.
    let scene_unresolved = pack::verify_restored(&loaded, &files, &root);
    let complete = manifest["complete"].as_bool().unwrap_or(false);
    // `load` resolved paths against the staging folder; the published folder is what counts.
    if complete && (!unresolved.is_empty() || !scene_unresolved.is_empty()) {
        return Err(JobError::new(
            "invalid_package",
            "The package is marked complete but the document names media it does not contain",
        )
        .with_details(json!({ "assets": unresolved, "scenes": scene_unresolved })));
    }
    staging_publish(staging, &root, destination)?;
    Ok(
        json!({ "action": "project.restore", "path": destination, "document": destination.join(DOCUMENT_NAME),
        "files": files.len(), "complete": complete, "project": manifest["project"], "verified": true,
        "unresolved": if complete { Vec::new() } else { unresolved },
        "unresolved_scenes": if complete { Vec::new() } else { scene_unresolved } }),
    )
}

fn staging_publish(staging: Staging, content: &Path, destination: &Path) -> Result<(), JobError> {
    if destination.exists() {
        return Err(JobError::new(
            "destination_exists",
            format!("`{}` appeared while restoring", destination.display()),
        ));
    }
    fs::rename(content, destination).map_err(|error| io_error(destination, &error))?;
    drop(staging);
    Ok(())
}
