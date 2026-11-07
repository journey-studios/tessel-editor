//! Library thumbnails of splats and scenes (BL144): the adapter that draws them.
//!
//! The core decides what is wanted and whether a result still belongs
//! (`tessel_editor_core::thumbnails`). This module renders one real frame per target through
//! the renderer the export uses, off the caller's thread, into the disposable cache next to the
//! other derivatives. A target that fails is reported as a [`Made::Failed`], not as a failed
//! job, so one broken scene never costs the others their picture; only cancellation ends the
//! job early, and then nothing is applied.
//!
//! **What a job draws from.** A private project that holds only the targets and what they depend
//! on (`EditorState::thumbnail_snapshot`), never a copy of the open project and its undo history.
//!
//! **A bare splat** has no camera, so it is framed from the file: the extent of its positions is
//! measured (`splat::measure`), the stand-in scene holds the cloud moved and scaled onto the unit
//! sphere and the plan's camera is set from the same box (`splat_framing::fit`). A cloud that cannot
//! be framed, or a picture that came out as the background only, fails with a code and is never
//! offered as a thumbnail.
//!
//! **Cancellation.** The renderer's `render_frame` blocks and cannot be interrupted from here, so a
//! cancelled job stops *between* frames (and before the renderer starts): it lasts at most
//! [`FRAME_TIMEOUT`] plus the renderer's start after the request. The host keeps the next job
//! waiting for it instead of running two renderers (`MediaHost::thumbnails`).

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tessel_editor_core::assets::MediaKind;
use tessel_editor_core::backend::Output;
use tessel_editor_core::jobs::{JobContext, JobError};
use tessel_editor_core::plan::{Content, PlanRequest, PlanTime, PreviewIntent};
use tessel_editor_core::session::Renderer;
use tessel_editor_core::splat::measure;
use tessel_editor_core::splat_framing::{self, Framing};
use tessel_editor_core::thumbnails::{Failure, Target, Thumbnail};
use tessel_editor_core::time::Ratio;
use tessel_editor_core::{EditorState, Id, headless};

use crate::derive::Cache;
use crate::host::RendererFactory;

/// One frame of a splat or a scene at this size is quick; a stuck renderer is not waited for
/// as long as an export frame. It is also the longest a cancelled job can keep the renderer.
pub const FRAME_TIMEOUT: Duration = Duration::from_secs(60);

/// What a job made for one asset.
pub enum Made {
    Ready(String, Thumbnail),
    Failed(String, Failure),
}

impl Made {
    pub fn asset(&self) -> &str {
        match self {
            Self::Ready(asset, _) | Self::Failed(asset, _) => asset,
        }
    }
}

/// Renders every target of `snapshot`, in order. The renderer is built on first need (a
/// cache hit never starts Chrome) and dropped when the job ends.
pub fn make(
    snapshot: &EditorState,
    targets: &[Target],
    cache: &Cache,
    factory: Option<Arc<RendererFactory>>,
    ctx: &JobContext,
) -> Result<Vec<Made>, JobError> {
    ctx.stage("thumbnails", targets.len() as u64);
    let mut renderer: Option<Box<dyn Renderer>> = None;
    let mut made = Vec::with_capacity(targets.len());
    for target in targets {
        ctx.check()?;
        let outcome = one(
            snapshot,
            target,
            cache,
            factory.as_ref(),
            &mut renderer,
            ctx,
        );
        made.push(match outcome {
            Ok(thumbnail) => Made::Ready(target.asset.clone(), thumbnail),
            // Cancellation ends the job; it is not a failure of this one asset.
            Err(error) if error.code == "cancelled" => return Err(error),
            Err(error) => Made::Failed(
                target.asset.clone(),
                Failure {
                    identity: target.identity.clone(),
                    code: error.code,
                    message: error.message,
                },
            ),
        });
        ctx.advance(1);
    }
    if let Some(renderer) = renderer.as_mut() {
        renderer.reset_project();
    }
    Ok(made)
}

fn one(
    snapshot: &EditorState,
    target: &Target,
    cache: &Cache,
    factory: Option<&Arc<RendererFactory>>,
    renderer: &mut Option<Box<dyn Renderer>>,
    ctx: &JobContext,
) -> Result<Thumbnail, JobError> {
    let item = snapshot
        .library
        .item(&target.asset)
        .ok_or_else(|| JobError::new("unknown_asset", format!("`{}` is gone", target.asset)))?;
    let asset = item.asset.as_ref().ok_or_else(|| {
        JobError::new(
            "asset_unavailable",
            format!("`{}` has no media", target.asset),
        )
    })?;
    // The snapshot is what the targets were computed from; a mismatch would draw another picture.
    if snapshot.thumbnail_identity(&target.asset).as_deref() != Some(target.identity.as_str()) {
        return Err(JobError::new(
            "stale_result",
            format!("`{}` changed before its thumbnail was drawn", target.asset),
        ));
    }
    // A scene that names a splat the project lacks cannot be drawn: said without starting a
    // renderer, and remembered under an identity that names the gap, so importing the splat
    // makes the scene pending again.
    let missing: Vec<String> = snapshot
        .thumbnail_dependencies(&target.asset)
        .into_iter()
        .filter(|dependency| dependency.is_missing())
        .map(|dependency| dependency.id)
        .collect();
    if !missing.is_empty() {
        return Err(JobError::new(
            "missing_dependency",
            format!(
                "`{}` is drawn with {}, which the library does not have",
                target.asset,
                missing
                    .iter()
                    .map(|id| format!("`{id}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    }
    let size = (target.width, target.height);
    let (path, _) = cache.picture(asset, &target.identity, size, |out| {
        ctx.check()?;
        let factory = factory.ok_or_else(|| {
            JobError::new(
                "backend_unavailable",
                "No renderer is attached: thumbnails of splats and scenes need `./scripts/render` or the desktop app",
            )
        })?;
        if renderer.is_none() {
            *renderer = Some(factory()?);
        }
        // The renderer starting is the slow part: a cancel that came meanwhile stops here.
        ctx.check()?;
        let mut probes = Vec::new();
        let mut framing = None;
        // The runtime draws a scene, not a bare splat: a splat is shown through a one-object
        // scene that only exists in this job's private copy of the project, framed from the file.
        let stand_in;
        let (drawn, asset) = if item.kind == MediaKind::Splat {
            let path = match &asset.source {
                tessel_editor_core::assets::Source::File { path } => path.clone(),
                _ => {
                    return Err(JobError::new(
                        "asset_unavailable",
                        format!("`{}` has no file to measure", target.asset),
                    ));
                }
            };
            let extent = measure(Path::new(&path))
                .map_err(|message| JobError::new("splat_not_drawable", message))?;
            let fit = splat_framing::fit(
                &extent,
                f64::from(size.0) / f64::from(size.1),
                // Where a capture's own orientation will go (BL144.2).
                None,
            )
            .map_err(|undrawable| JobError::new(undrawable.code, undrawable.message))?;
            let dir = out.parent().unwrap_or(Path::new("."));
            stand_in = splat_stand_in(snapshot, &target.asset, &fit, dir)?;
            probes = splat_framing::probes(size.0, size.1);
            framing = Some(fit);
            (&stand_in.0, stand_in.1)
        } else {
            (snapshot, item.id)
        };
        let mut plan = drawn
            .plan(&PlanRequest {
                intent: PreviewIntent::Source {
                    asset,
                    time: Ratio::ZERO,
                },
                at: PlanTime::Frame(0),
            })
            .map_err(|error| JobError::new(error.code, error.message))?;
        if let Some(fit) = &framing {
            for layer in &mut plan.layers {
                if let Content::Scene { camera, .. } = &mut layer.content {
                    *camera = fit.camera;
                }
            }
        }
        let output = Output {
            width: size.0,
            height: size.1,
            png: Some(out.to_string_lossy().into_owned()),
        };
        let frame = renderer
            .as_mut()
            .ok_or_else(|| JobError::new("backend_unavailable", "The renderer did not start"))?
            .render_frame(drawn, plan, output, probes, FRAME_TIMEOUT)
            .map_err(|error| {
                JobError::new("render_failed", format!("{} ({})", error.message, error.code))
            })?;
        if framing.is_some() && splat_framing::is_blank(frame.mean, &frame.probes) {
            return Err(JobError::new(
                "empty_picture",
                format!(
                    "`{}` came out as the background only: the cloud is not in the picture",
                    target.asset
                ),
            ));
        }
        Ok(())
    })?;
    Ok(Thumbnail {
        path: path.to_string_lossy().into_owned(),
        width: size.0,
        height: size.1,
        identity: target.identity.clone(),
    })
}

/// A copy of the job's private project that also holds a scene with the splat `splat` alone,
/// placed as `framing` says, in front of the same background the library scenes use. Never the
/// open project: it is not even a copy of it. The scene file is written next to the picture being
/// drawn, and its asset gets an ID that is short and not one of the project's.
fn splat_stand_in(
    snapshot: &EditorState,
    splat: &str,
    framing: &Framing,
    dir: &Path,
) -> Result<(EditorState, Id), JobError> {
    let file = dir.join("splat.tessel-scene.json");
    let scene = json!({
        "format": "tessel-scene", "version": 1, "background": splat_framing::BACKGROUND_RGBA,
        "objects": [{ "type": "splat", "asset": splat, "position": framing.position,
            "scale": framing.scale }],
    });
    let bytes = serde_json::to_vec_pretty(&scene)
        .map_err(|error| JobError::new("render_failed", error.to_string()))?;
    fs::write(&file, bytes).map_err(|error| crate::fsx::io_error(&file, &error))?;
    let id = snapshot.thumbnail_stand_in_id(splat);
    let mut copy = snapshot.clone();
    headless::execute(
        &mut copy,
        &json!({ "op": "asset.add", "kind": "scene", "id": id,
            "name": "Thumbnail stand-in", "path": file }),
    )
    .map_err(|error| JobError::new(error.code, error.message))?;
    let id = Id::new(&id).map_err(|error| JobError::new("invalid_id", error.to_string()))?;
    Ok((copy, id))
}
