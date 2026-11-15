//! F042: the renderer half of the directed-scene export (BL008).
//!
//! A [`ProcessSceneRenderer`] is the same [`SceneRenderer`] `scene.render` uses (Node worker +
//! Chrome + Three.js), behind the media crate's [`SceneFrameRenderer`] seam. It owns no domain
//! rule: the core decided the plan, the media crate decided which files are approved, and this
//! only moves them to the runtime and writes the PNG. Each export builds its own instance, so it
//! never shares a browser, a slot book or frame timing with a monitor or a `scene.render` session.

use std::sync::Arc;
use std::time::Duration;

use tessel_editor_core::backend::BackendError;
use tessel_editor_core::bridge::BridgeBackend;
use tessel_editor_core::jobs::JobError;
use tessel_editor_core::scene_plan::ScenePlan;
use tessel_editor_media::scene_encoder::{
    FrameJob, RenderedFrame, SceneFrameRenderer, SceneRendererFactory,
};
use tessel_editor_media::scene_resources::ResourceResolver;

use crate::resource_union::{attribute_backend, union_failure, union_of};
use crate::scene::{ResolvedResource, SceneResourceMap};
use crate::{FrameOptions, HANDSHAKE_TIMEOUT, ProcessTransport, RuntimeConfig, SceneRenderer};

/// One frame at 4K on the software renderer can take many seconds; the project export allows the
/// same two minutes before it calls a frame lost.
pub const EXPORT_FRAME_TIMEOUT: Duration = Duration::from_secs(120);

/// The map of the files `plan` draws, read from the approved resolver. WHY a map per frame and
/// not one per export: the renderer's slot book decides what stays loaded, and the plan of a frame
/// names exactly the resources that frame needs: both decks of a transition (the union, by asset
/// id), looked up under `scene_id`, the scene ATTACHED to the project. A refusal keeps the
/// resolver's own code and names the asset and its decks in `details.resource`.
pub fn resource_map(
    plan: &ScenePlan,
    scene_id: &str,
    resolver: &dyn ResourceResolver,
    generation: u64,
) -> Result<SceneResourceMap, BackendError> {
    let mut map = SceneResourceMap::new(generation);
    let union = union_of(plan).map_err(|failure| union_failure(plan, &failure))?;
    for resource in &union {
        let approved = resolver
            .resolve_asset(scene_id, &resource.asset_id)
            .map_err(|error| {
                attribute_backend(BackendError::new(error.code(), error.to_string()), resource)
            })?;
        map.insert(
            resource.asset_id.clone(),
            ResolvedResource {
                revision_id: approved.revision_id.unwrap_or_default(),
                content_hash: approved.content_hash,
                path: approved.path,
            },
        )
        .map_err(|error| attribute_backend(error, resource))?;
    }
    Ok(map)
}

fn job_error(error: BackendError) -> JobError {
    // A frame superseded or cancelled is the export's own cancellation, not a render failure.
    let code = match error.code.as_str() {
        "cancelled" => return JobError::cancelled(),
        "backend_unavailable" => "backend_unavailable",
        _ => "render_failed",
    };
    let mut details = serde_json::json!({ "backend_code": error.code });
    // Only the attribution travels on (the message already names the decks): a job error never
    // carried the rest of a backend's details, and this does not widen that.
    if let Some(resource) = error
        .details
        .as_ref()
        .and_then(|details| details.get("resource"))
    {
        details["resource"] = resource.clone();
    }
    JobError::new(code, error.message).with_details(details)
}

pub struct ProcessSceneRenderer {
    renderer: SceneRenderer<ProcessTransport>,
}

impl ProcessSceneRenderer {
    pub fn new(config: RuntimeConfig) -> Self {
        let backend = BridgeBackend::new(
            "three-directed-export",
            ProcessTransport::new(config),
            HANDSHAKE_TIMEOUT,
        );
        Self {
            renderer: SceneRenderer::new(backend).with_timeout(EXPORT_FRAME_TIMEOUT),
        }
    }
}

impl SceneFrameRenderer for ProcessSceneRenderer {
    fn max_size(&mut self) -> Result<Option<u32>, JobError> {
        let capabilities = self.renderer.ensure_ready().map_err(job_error)?;
        Ok(capabilities.raw["capabilities"]["max_size"]
            .as_u64()
            .and_then(|max| u32::try_from(max).ok()))
    }

    fn render(&mut self, job: &FrameJob<'_>) -> Result<RenderedFrame, JobError> {
        let map = resource_map(job.plan, job.scene_id, job.resources, job.generation)
            .map_err(job_error)?;
        let result = self
            .renderer
            .render(
                job.plan,
                &map,
                &FrameOptions {
                    size: Some(job.size),
                    png: Some(job.png.to_path_buf()),
                    probes: Vec::new(),
                    inspect: true,
                },
            )
            .map_err(job_error)?;
        Ok(RenderedFrame {
            hash: result.hash,
            render_ms: result.render_ms,
        })
    }

    fn finish(&mut self) {
        let _ = self.renderer.release_all();
        self.renderer.shutdown();
    }
}

/// The factory `MediaHost::with_scene_renderer` takes.
pub fn scene_renderer_factory(config: RuntimeConfig) -> Arc<SceneRendererFactory> {
    Arc::new(move || {
        Ok(Box::new(ProcessSceneRenderer::new(config.clone())) as Box<dyn SceneFrameRenderer>)
    })
}
