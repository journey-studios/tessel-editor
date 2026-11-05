//! Project-to-project import, the byte-moving half (P08). The ID mapping and the transactional
//! merge live in `tessel_editor_core::interchange`; this module reads the other project (a
//! document or a package), copies the bytes of what is imported into the destination workspace
//! (deduplicated by hash) and returns the assets to register.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tessel_editor_core::assets::{Asset, Decoded, MediaKind, Source, Storage, expand_pattern};
use tessel_editor_core::interchange::{ImportPlan, plan_import};
use tessel_editor_core::jobs::{JobContext, JobError};
use tessel_editor_core::{EditorState, persist};

use crate::fsx::{self, Staging, copy_file_hashed, hash_file, hex, io_error, safe_file_name};
use crate::import::{FileState, file_state, managed_path};
use crate::package;

pub struct PreparedProjectImport {
    pub source: EditorState,
    pub plan: ImportPlan,
    pub replacements: HashMap<String, Asset>,
    pub copied: u64,
    pub reused: u64,
}

/// Loads the other project. A package is verified and extracted to a scratch folder inside the
/// destination workspace; `scratch` is removed by the caller (see [`Loaded::cleanup`]).
pub struct Loaded {
    pub state: EditorState,
    scratch: Option<PathBuf>,
}

impl Loaded {
    pub fn cleanup(self) {
        if let Some(scratch) = self.scratch {
            let _ = fs::remove_dir_all(scratch);
        }
    }
}

pub fn load_source(path: &Path, workspace: &Path, ctx: &JobContext) -> Result<Loaded, JobError> {
    let is_package = path.is_dir() && path.join(package::MANIFEST_NAME).is_file()
        || path
            .extension()
            .is_some_and(|extension| extension == "tesselpack");
    if !is_package {
        return Ok(Loaded {
            state: persist::load(path).map_err(JobError::from)?,
            scratch: None,
        });
    }
    let scratch_parent = workspace.join(".tessel").join("import-scratch");
    let scratch = scratch_parent.join(format!("src-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch_parent).map_err(|error| io_error(&scratch_parent, &error))?;
    package::restore(path, &scratch, ctx)?;
    let state = persist::load(&scratch.join(package::DOCUMENT_NAME)).map_err(JobError::from);
    match state {
        Ok(state) => Ok(Loaded {
            state,
            scratch: Some(scratch),
        }),
        Err(error) => {
            let _ = fs::remove_dir_all(&scratch);
            Err(error)
        }
    }
}

/// Rewrites the splat references of a scene file with the plan's asset renames.
fn rewrite_scene(bytes: &[u8], plan: &ImportPlan) -> Result<Vec<u8>, JobError> {
    let mut scene: Value = serde_json::from_slice(bytes)
        .map_err(|error| JobError::new("invalid_asset", format!("Scene is not JSON: {error}")))?;
    if let Some(objects) = scene["objects"].as_array_mut() {
        for object in objects {
            if object["type"] == "splat"
                && let Some(old) = object["asset"].as_str().map(str::to_string)
                && let Some(new) = plan.map_asset(&old)
            {
                object["asset"] = json!(new);
            }
        }
    }
    serde_json::to_vec_pretty(&scene).map_err(|error| JobError::new("io_error", error.to_string()))
}

fn place_file(
    workspace: &Path,
    source: &Path,
    name: &str,
    ctx: &JobContext,
) -> Result<(PathBuf, String, u64, bool), JobError> {
    let media = workspace.join("media");
    let staging = Staging::create(&media, "project-import")?;
    let staged = staging.path().join(safe_file_name(name));
    let (hash, size) = hash_file(source, ctx.token(), |_| {})?;
    let target = managed_path(workspace, &hash, name);
    if target.exists() && fs::metadata(&target).is_ok_and(|meta| meta.len() == size) {
        return Ok((target, hash, size, false));
    }
    fsx::ensure_space(&media, size, 64 << 20)?;
    let (copied_hash, copied) = copy_file_hashed(source, &staged, ctx.token(), |_| {})?;
    if copied_hash != hash || copied != size {
        return Err(JobError::new(
            "integrity_error",
            format!("`{}` changed while it was imported", source.display()),
        ));
    }
    let folder = target.parent().unwrap_or(&media);
    fs::create_dir_all(folder).map_err(|error| io_error(folder, &error))?;
    fs::rename(&staged, &target).map_err(|error| io_error(&target, &error))?;
    Ok((target, hash, size, true))
}

/// Plans the merge and copies the bytes it needs. Nothing in the destination project changes
/// here; call `tessel_editor_core::interchange::apply_import` with the result.
pub fn prepare(
    destination: &EditorState,
    source: EditorState,
    workspace: &Path,
    sequences: &[String],
    ctx: &JobContext,
) -> Result<PreparedProjectImport, JobError> {
    let plan = plan_import(destination, &source, sequences).map_err(JobError::from)?;
    // Everything must exist before the first byte is copied: a missing file fails the whole
    // import with the full list instead of half-importing.
    let mut missing = Vec::new();
    for (old, _) in &plan.assets {
        if let Some(asset) = source
            .library
            .item(old)
            .and_then(|item| item.asset.as_ref())
            && matches!(asset.source, Source::File { .. })
            && file_state(asset) == FileState::Missing
        {
            missing.push(old.clone());
        }
    }
    if !missing.is_empty() {
        return Err(JobError::new(
            "missing_media",
            format!(
                "{} file(s) of the source project are missing",
                missing.len()
            ),
        )
        .with_details(json!({ "assets": missing })));
    }
    ctx.stage("copy", plan.assets.len() as u64);
    let (mut replacements, mut copied, mut reused) = (HashMap::new(), 0, 0);
    for (old, new) in &plan.assets {
        ctx.check()?;
        let item = source
            .library
            .item(old)
            .ok_or_else(|| JobError::new("invalid_request", format!("Unknown asset `{old}`")))?;
        let Some(asset) = item.asset.as_ref() else {
            continue;
        };
        let mut placed = asset.clone();
        match &asset.source {
            Source::File { path } => {
                let name = asset
                    .identity
                    .file_name
                    .clone()
                    .or_else(|| {
                        Path::new(path)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                    })
                    .unwrap_or_else(|| "file".into());
                let renamed_splats = item.kind == MediaKind::Scene
                    && asset.meta.requires.iter().any(|id| {
                        plan.map_asset(id.as_str())
                            .is_some_and(|to| to != id.as_str())
                    });
                let (target, hash, size, new_bytes) = if renamed_splats {
                    // The scene file itself names its splats: it must be rewritten, so it is a
                    // new file with its own hash, not a copy of the source's.
                    let bytes = rewrite_scene(
                        &fs::read(path).map_err(|e| io_error(Path::new(path), &e))?,
                        &plan,
                    )?;
                    let hash = hex(&Sha256::digest(&bytes));
                    let target = managed_path(workspace, &hash, &name);
                    let wrote = !target.exists();
                    if wrote {
                        let folder = target.parent().unwrap_or(workspace);
                        fs::create_dir_all(folder).map_err(|e| io_error(folder, &e))?;
                        fs::write(&target, &bytes).map_err(|e| io_error(&target, &e))?;
                    }
                    (target, hash, bytes.len() as u64, wrote)
                } else {
                    place_file(workspace, Path::new(path), &name, ctx)?
                };
                placed.source = Source::File {
                    path: target.to_string_lossy().into_owned(),
                };
                placed.identity.hash = Some(hash);
                placed.identity.size = Some(size);
                placed.identity.file_name = Some(name);
                if new_bytes {
                    copied += size
                } else {
                    reused += 1
                }
            }
            Source::FrameSequence {
                pattern,
                first,
                count,
            } => {
                let folder = workspace.join("media").join(format!("seq-{new}"));
                fs::create_dir_all(&folder).map_err(|e| io_error(&folder, &e))?;
                let frame_name = Path::new(pattern)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "f%05d.png".into());
                for index in *first..first + count {
                    ctx.check()?;
                    let Some(frame) = expand_pattern(pattern, index) else {
                        continue;
                    };
                    let target = folder.join(Path::new(&frame).file_name().unwrap_or_default());
                    if !target.exists() {
                        let (_, size) =
                            copy_file_hashed(Path::new(&frame), &target, ctx.token(), |_| {})?;
                        copied += size;
                    }
                }
                placed.source = Source::FrameSequence {
                    pattern: folder.join(frame_name).to_string_lossy().into_owned(),
                    first: *first,
                    count: *count,
                };
            }
            Source::Demo(_) | Source::Inline => {}
        }
        placed.identity.storage = Storage::Managed;
        placed.identity.origin = None;
        placed.decoded = None::<Decoded>;
        replacements.insert(old.clone(), placed);
        ctx.advance(1);
    }
    Ok(PreparedProjectImport {
        source,
        plan,
        replacements,
        copied,
        reused,
    })
}
