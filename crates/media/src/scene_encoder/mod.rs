//! F042: the media half of the directed-scene export (BL008).
//!
//! The core freezes what is exported ([`SceneExportSnapshot`](tessel_editor_core::scene_export::SceneExportSnapshot));
//! this module runs it: a [`SceneFrameRenderer`] (injected, built per export) draws each frame of
//! the plan the core evaluated, FFmpeg encodes the PNGs, ffprobe verifies the file and the
//! staging/publication of the project export (`fsx`) puts it at the destination. The job
//! registry, progress, cancellation and `job.*` operations are the ones every other export uses
//! (`MediaHost`, `scene.export`); there is no second queue.
//!
//! Contract and limits: `docs/features/scene-export.md`.

mod args;
mod job;
mod render;

pub use args::encode_args;
pub use job::{LISTED_HASHES, SceneExport, preflight, run};
pub use render::{
    FrameJob, FrozenResources, RenderedFrame, SceneFrameRenderer, SceneRendererFactory,
};
