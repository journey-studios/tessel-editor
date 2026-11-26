//! Apple SHARP inference stays on the media job thread. Its PLY keeps the capture
//! metadata the editor already uses for library previews and timeline cameras.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};
use tessel_editor_core::assets::MediaKind;
use tessel_editor_core::jobs::{JobContext, JobError};
use tessel_editor_core::sharp::Choice;
use tessel_editor_core::state::{Command as EditCommand, ProjectSession};
use tessel_editor_core::{CommandError, EditorState};

use crate::derive::wait_cancellable;
use crate::fsx::{Staging, io_error};
use crate::import::{self, ImportMode, ImportRequest, ItemStatus, PreparedItem};
use crate::probe;
use crate::tools::Tools;

pub(crate) struct Placement {
    pub id: Option<String>,
    pub track: String,
    pub start: i64,
    pub duration: i64,
}

pub(crate) struct Generated {
    pub item: PreparedItem,
    pub session: ProjectSession,
    pub workspace: PathBuf,
    pub folder: Option<String>,
    pub placement: Option<Placement>,
}

pub(crate) fn prepare(
    tools: &Tools,
    workspace: &Path,
    image: &Path,
    ctx: &JobContext,
) -> Result<PreparedItem, JobError> {
    let image = fs::canonicalize(image).map_err(|error| io_error(image, &error))?;
    if probe::probe(tools, &image)?.kind != MediaKind::Still {
        return Err(JobError::new(
            "not_image",
            "Select a still image to generate a splat",
        ));
    }
    let sharp = std::env::var_os("TESSEL_SHARP").unwrap_or_else(|| "sharp".into());
    let checkpoint = std::env::var_os("TESSEL_SHARP_CHECKPOINT")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(|home| {
                    PathBuf::from(home).join(".cache/torch/hub/checkpoints/sharp_2572gikvuh.pt")
                })
        })
        .filter(|path| path.is_file())
        .ok_or_else(|| {
            JobError::new(
                "sharp_checkpoint_missing",
                "Set TESSEL_SHARP_CHECKPOINT to the local Apple SHARP checkpoint",
            )
        })?;
    let device = std::env::var_os("TESSEL_SHARP_DEVICE").unwrap_or_else(|| {
        if cfg!(target_os = "macos") {
            "mps"
        } else {
            "cpu"
        }
        .into()
    });
    let staging = Staging::create(workspace, "sharp")?;
    let log = staging.path().join("sharp.log");
    let stderr = File::create(&log).map_err(|error| io_error(&log, &error))?;

    ctx.stage("generate_splat", 0);
    ctx.check()?;
    let mut child = Command::new(&sharp)
        .arg("predict")
        .arg("-i")
        .arg(&image)
        .arg("-o")
        .arg(staging.path())
        .arg("-c")
        .arg(checkpoint)
        .arg("--device")
        .arg(device)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr)
        .spawn()
        .map_err(|error| {
            JobError::new(
                "sharp_unavailable",
                format!("Cannot start SHARP ({error}). Install Apple ml-sharp or set TESSEL_SHARP"),
            )
        })?;
    wait_cancellable(&mut child, ctx, "SHARP inference").map_err(|error| {
        if error.is_cancelled() {
            return error;
        }
        let mut tail = String::new();
        if let Ok(mut file) = File::open(&log)
            && let Ok(metadata) = file.metadata()
            && file
                .seek(SeekFrom::Start(metadata.len().saturating_sub(4096)))
                .is_ok()
        {
            let mut bytes = Vec::new();
            let _ = file.take(4096).read_to_end(&mut bytes);
            tail = String::from_utf8_lossy(&bytes).trim().to_owned();
        }
        JobError::new(
            "sharp_failed",
            if tail.is_empty() { error.message } else { tail },
        )
    })?;

    let stem = image.file_stem().unwrap_or_default();
    let mut filename = stem.to_os_string();
    filename.push(".ply");
    let output = staging.path().join(filename);
    let capture = tessel_editor_core::sharp::read(&output, Choice::Auto).map_err(|error| {
        JobError::new("invalid_splat", format!("SHARP output: {}", error.message))
    })?;
    if capture.is_none() {
        return Err(JobError::new(
            "invalid_splat",
            "SHARP did not produce a calibrated PLY",
        ));
    }
    let request = ImportRequest {
        paths: vec![output],
        mode: ImportMode::Copy,
        folder: None,
        capture: Choice::Auto,
    };
    // Dedupe against the live library on the apply thread, not the snapshot
    // from before model inference. Imports can finish while SHARP is running.
    let mut items = import::prepare(tools, workspace, &request, &HashMap::new(), ctx)?;
    let mut item = items.pop().expect("one requested import yields one report");
    if let Some(error) = item.error.take() {
        return Err(error);
    }
    item.source = image;
    if let Some(asset) = item.asset.as_mut() {
        asset.identity.origin = None;
    }
    Ok(item)
}

impl Generated {
    pub(crate) fn apply(self, state: &mut EditorState) -> Result<Value, CommandError> {
        if !self.session.is_same(state.project.session)
            || state.project.workspace.as_ref() != Some(&self.workspace)
        {
            return Err(CommandError::new(
                "stale_result",
                "The project changed while SHARP was running",
            ));
        }
        let mut item = self.item;
        let existing = item.asset.as_ref().and_then(|asset| {
            let hash = asset.identity.hash.as_ref()?;
            state.library.items.iter().find(|existing| {
                existing.kind == MediaKind::Splat
                    && existing
                        .asset
                        .as_ref()
                        .is_some_and(|current| current.identity.hash.as_ref() == Some(hash))
            })
        });
        if let Some(existing) = existing {
            item.status = ItemStatus::Duplicate {
                of: existing.id.to_string(),
            };
            item.asset = None;
        }
        let duplicate = matches!(item.status, ItemStatus::Duplicate { .. });
        let unchanged = duplicate && self.placement.is_none();
        let apply = |state: &mut EditorState| {
            let imported = import::commit(state, &[item], self.folder.as_deref())?;
            let entry = &imported["items"][0];
            let id = entry["id"]
                .as_str()
                .or_else(|| entry["duplicate_of"].as_str())
                .ok_or_else(|| {
                    let error = &entry["error"];
                    CommandError::new(
                        if duplicate {
                            "capture_conflict"
                        } else {
                            "invalid_splat"
                        },
                        error["message"]
                            .as_str()
                            .unwrap_or("The generated splat was not imported"),
                    )
                })?;
            let clip = self
                .placement
                .map(|place| {
                    state.execute(EditCommand::AddClip {
                        id: place.id,
                        name: None,
                        track: place.track,
                        asset: Some(id.into()),
                        start: place.start,
                        duration: place.duration,
                        source_start: None,
                        text: None,
                        camera: None,
                        properties: Vec::new(),
                    })
                })
                .transpose()?;
            Ok(
                json!({ "action": "media.splat.generate", "asset": id, "duplicate": duplicate,
                "changed": imported["changed"] == true || clip.is_some(),
                "clip": clip, "item": entry }),
            )
        };
        if unchanged {
            return apply(state);
        }
        state.transaction("Generate splat", apply)
    }
}
