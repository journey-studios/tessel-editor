//! F042: the media half of the directed-scene export (BL008).
//!
//! The core freezes what is exported ([`SceneExportSnapshot`](tessel_editor_core::scene_export::SceneExportSnapshot));
//! this module runs it: a [`SceneFrameRenderer`] (injected, built per export) draws each frame of
//! the plan the core evaluated, FFmpeg encodes the PNGs, ffprobe verifies the file and the
//! staging/publication of the project export (`fsx`) puts it at the destination. The job
//! registry, progress, cancellation and `job.*` operations are the ones every other export uses
//! (`MediaHost`, `scene.export`); there is no second queue.
//!
//! The declared WAV mix (F042A, BL016) is rendered by [`render_audio`] into a 48 kHz stereo stream the
//! encoder muxes as AAC; the rules about which sample plays when live in the core.
//!
//! Contract and limits: `docs/features/scene-export.md`, `docs/features/scene-export-audio.md`.

mod args;
mod audio;
mod job;
mod render;

pub use args::{encode_args, encode_args_with_audio};
pub use audio::{
    InspectError, MixReport, WavLayout, check_layout, inspect, render as render_audio,
};
pub use job::{LISTED_HASHES, SceneExport, preflight, run};
pub use render::{
    FrameJob, FrozenResources, RenderedFrame, SceneFrameRenderer, SceneRendererFactory,
};
