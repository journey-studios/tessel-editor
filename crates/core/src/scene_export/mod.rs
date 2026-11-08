//! F042: exporting a directed scene as video, the domain half (BL008).
//!
//! The core decides *what* is exported and never touches a file, a process or a renderer:
//!
//! * [`SceneExportSpec`] is the parsed request (`scene.export`);
//! * [`SceneExportSnapshot`] freezes the committed scene, the preset and the audio decision, and
//!   answers [`plan`](SceneExportSnapshot::plan) for each output frame with the same complete
//!   [`ScenePlan`](crate::scene_plan::ScenePlan) `scene.plan` and `scene.render` use;
//! * [`SceneExportSchedule`] maps output frames to scene frames in exact rational time;
//! * [`DirectedAudioMix`] maps output samples of the declared WAV mix to file time (F042A, BL016).
//!
//! Resources, the encoder, jobs and publication live in `tessel-editor-media`
//! (`scene_encoder` + `MediaHost`), and the drawing in `tessel-editor-render`; both consume this
//! snapshot. A scene is never a clip of the timeline: the export reads `state.scenes` only.
//! Contract and limits: `docs/features/scene-export.md`.

mod audio;
mod schedule;
mod snapshot;
mod spec;

pub use audio::{
    DirectedAudioMix, MAX_SOURCE_RATE, MIN_SOURCE_RATE, MIX_RATE, SourceCursor, SourceFacts,
    SourcePosition, SourceSpan,
};
pub use schedule::SceneExportSchedule;
pub use snapshot::{AudioPlan, SceneExportSnapshot};
pub use spec::{AudioChoice, SceneExportSpec};
