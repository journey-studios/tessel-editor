//! Library thumbnails of splats and scenes (BL144): the adapter that draws them.
//!
//! The core decides what is wanted and whether a result still belongs
//! (`tessel_editor_core::thumbnails`). This module renders one real frame per target through
//! the renderer the export uses, off the caller's thread, into the disposable cache next to the
//! other derivatives. A target that fails is reported as a [`Made::Failed`], not as a failed
//! job, so one broken scene never costs the others their picture; only cancellation ends the
//! job early, and then nothing is applied.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tessel_editor_core::assets::MediaKind;
use tessel_editor_core::backend::Output;
use tessel_editor_core::jobs::{JobContext, JobError};
use tessel_editor_core::plan::{PlanRequest, PlanTime, PreviewIntent};
use tessel_editor_core::session::Renderer;
use tessel_editor_core::thumbnails::{Failure, Target, Thumbnail};
use tessel_editor_core::time::Ratio;
use tessel_editor_core::{EditorState, Id, headless};

use crate::derive::Cache;
use crate::host::RendererFactory;

/// One frame of a splat or a scene at this size is quick; a stuck renderer is not waited for
/// as long as an export frame.
const FRAME_TIMEOUT: Duration = Duration::from_secs(60);

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
        let outcome = one(snapshot, target, cache, factory.as_ref(), &mut renderer);
        made.push(match outcome {
            Ok(thumbnail) => Made::Ready(target.asset.clone(), thumbnail),
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
    let size = (target.width, target.height);
    let (path, _) = cache.picture(asset, &target.identity, size, |out| {
        let factory = factory.ok_or_else(|| {
            JobError::new(
                "backend_unavailable",
                "No renderer is attached: thumbnails of splats and scenes need `./scripts/render` or the desktop app",
            )
        })?;
        if renderer.is_none() {
            *renderer = Some(factory()?);
        }
        // The runtime draws a scene, not a bare splat: a splat is shown through a one-object
        // scene that only exists in this job's private copy of the project.
        let stand_in;
        let (drawn, asset) = if item.kind == MediaKind::Splat {
            let dir = out.parent().unwrap_or(Path::new("."));
            stand_in = splat_stand_in(snapshot, &target.asset, dir)?;
            (&stand_in.0, stand_in.1)
        } else {
            (snapshot, item.id)
        };
        let plan = drawn
            .plan(&PlanRequest {
                intent: PreviewIntent::Source {
                    asset,
                    time: Ratio::ZERO,
                },
                at: PlanTime::Frame(0),
            })
            .map_err(|error| JobError::new(error.code, error.message))?;
        let output = Output {
            width: size.0,
            height: size.1,
            png: Some(out.to_string_lossy().into_owned()),
        };
        renderer
            .as_mut()
            .ok_or_else(|| JobError::new("backend_unavailable", "The renderer did not start"))?
            .render_frame(drawn, plan, output, Vec::new(), FRAME_TIMEOUT)
            .map(|_| ())
            .map_err(|error| JobError::new("render_failed", format!("{} ({})", error.message, error.code)))
    })?;
    Ok(Thumbnail {
        path: path.to_string_lossy().into_owned(),
        width: size.0,
        height: size.1,
        identity: target.identity.clone(),
    })
}

/// A copy of the project that also holds a scene with the splat `splat` alone at the origin, in
/// front of the same background the library scenes use. Only this job's copy: the open project
/// never sees the stand-in. The scene file is written next to the picture being drawn.
fn splat_stand_in(
    snapshot: &EditorState,
    splat: &str,
    dir: &Path,
) -> Result<(EditorState, Id), JobError> {
    let file = dir.join("splat.tessel-scene.json");
    let scene = json!({
        "format": "tessel-scene", "version": 1, "background": [0.04, 0.05, 0.1, 1.0],
        "objects": [{ "type": "splat", "asset": splat, "position": [0.0, 0.0, 0.0], "scale": 1.0 }],
    });
    let bytes = serde_json::to_vec_pretty(&scene)
        .map_err(|error| JobError::new("render_failed", error.to_string()))?;
    fs::write(&file, bytes).map_err(|error| crate::fsx::io_error(&file, &error))?;
    let id = format!("thumb-{splat}");
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
