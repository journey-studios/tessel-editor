//! The seam between the scene export and whatever draws a frame.
//!
//! `tessel-editor-render` depends on this crate, so the encoder cannot name its renderer. It asks
//! for a [`SceneFrameRenderer`] through a factory instead, exactly like the project export asks
//! for a `Renderer`: the export builds its own on its own thread and shares no GPU state, no
//! slot book and no frame timing with a monitor or a `scene.render` session.

use std::path::Path;
use std::sync::Arc;

use tessel_editor_core::jobs::JobError;
use tessel_editor_core::scene_plan::ScenePlan;
use tessel_editor_core::scene_resources::ResourceKey;

use crate::scene_resources::{
    ApprovedResource, ResolveError, ResolvedResourceMap, ResourceResolver,
};

/// One frame to draw.
pub struct FrameJob<'a> {
    pub scene_id: &'a str,
    /// The complete plan of the frame, as `scene.plan` answers it.
    pub plan: &'a ScenePlan,
    /// Where every file of the plan is approved. Never a path the renderer builds itself.
    pub resources: &'a dyn ResourceResolver,
    /// The generation the resources were approved under: constant for the whole export.
    pub generation: u64,
    /// Canvas size: the preset's render geometry, not the plan's declared output.
    pub size: (u32, u32),
    /// Where the PNG of the frame must be written.
    pub png: &'a Path,
}

/// What the renderer measured of the frame it wrote.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedFrame {
    /// The runtime's hash of the RGBA readback: the same field `scene.render` answers for the
    /// same plan and size, which is how an export frame is compared with a preview frame.
    pub hash: String,
    pub render_ms: f64,
}

pub trait SceneFrameRenderer {
    /// Largest side the renderer draws, when it says. Asking may start the renderer.
    fn max_size(&mut self) -> Result<Option<u32>, JobError>;

    /// Draws `job.plan` into `job.png`. A failed frame is an error: nothing is skipped or retried
    /// into a different picture.
    fn render(&mut self, job: &FrameJob<'_>) -> Result<RenderedFrame, JobError>;

    /// The export ended (any way): release the pictures and the browser.
    fn finish(&mut self) {}
}

/// Builds the renderer one export drives, on the export's own thread.
pub type SceneRendererFactory =
    dyn Fn() -> Result<Box<dyn SceneFrameRenderer>, JobError> + Send + Sync;

/// The resources an export reads: the approved map of the scene **as it was when the export
/// started**. WHY a frozen map and not the live session: the session forgets everything when the
/// project changes or a resolver is invalidated, and an export that quietly switched to other
/// bytes halfway would not be one video. The files are verified when the map is approved
/// (`scene.resources.resolve`); this does not hash them again.
#[derive(Debug, Clone)]
pub struct FrozenResources(Arc<ResolvedResourceMap>);

impl FrozenResources {
    pub fn new(map: Arc<ResolvedResourceMap>) -> Self {
        Self(map)
    }

    pub fn map(&self) -> &ResolvedResourceMap {
        &self.0
    }
}

impl ResourceResolver for FrozenResources {
    fn resolve(&self, scene_id: &str, key: &ResourceKey) -> Result<ApprovedResource, ResolveError> {
        if scene_id != self.0.scene_id {
            return Err(ResolveError::SceneNotLoaded {
                scene: scene_id.to_string(),
            });
        }
        self.0.approved(key)
    }
}
