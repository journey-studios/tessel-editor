//! Node animation of a directed scene (BB03, second slice): the performance channels of an
//! instance (`cycle`, `event`, `static`) evaluated at a frame **of the instance**, the resulting
//! transform of every node, and the local and world matrices through hierarchy and sockets.
//!
//! Like `temporal`, everything is a pure function of the imported `Bundle` and an integer frame:
//! no clock, no playback state, the same bits for the same frame however it was reached.
//! `Bundle::evaluate_frame` (in `temporal`) is the entry that maps an edit frame to an instance
//! frame and returns camera and nodes together; this module is what it calls, and it can be used
//! on its own for an instance that no shot plays.
//!
//! | Question | Answer |
//! |---|---|
//! | Which channels does an instance apply, and do two of them fight over a property? | `Bundle::performance_plan` |
//! | What does each channel write at an instance frame? | `PerformancePlan::overrides_at` |
//! | Where is every node, locally and in the scene, at an instance frame? | `Bundle::evaluate_instance_nodes` |
//! | Many frames in any order | `Bundle::node_timeline` → `NodeTimeline::state_at` |
//!
//! What is **not** here: lighting, weather, effects, rigs and paths. A scene that declares them
//! still imports; the evaluation reports them as unevaluated (`SceneGraph::unevaluated_sections`,
//! `FrameEvaluation::unevaluated`) and never invents a state for them.
//!
//! Contract, tolerances and what was compared with the origin: `docs/features/scene-animation.md`.

mod channels;
mod nodes;

pub use channels::{NodeOverride, PerformancePlan};
pub use nodes::{NodeFrame, NodeState, NodeTimeline};

use crate::directed_scene::{Bundle, SceneInstance, TemporalError};

impl Bundle {
    fn instance_or_error(&self, instance_id: &str) -> Result<&SceneInstance, TemporalError> {
        self.instance(instance_id)
            .ok_or_else(|| TemporalError::Unresolved {
                kind: "scene instance",
                id: instance_id.to_string(),
            })
    }

    /// The channels `instance_id` applies, checked: `PerformanceWriterConflict` when two of them
    /// write the same node property. Cheap, and independent of any frame.
    pub fn performance_plan(
        &self,
        instance_id: &str,
    ) -> Result<PerformancePlan<'_>, TemporalError> {
        PerformancePlan::new(self, self.instance_or_error(instance_id)?)
    }

    /// An instance laid out for evaluation: build it once to ask for many frames.
    pub fn node_timeline(&self, instance_id: &str) -> Result<NodeTimeline<'_>, TemporalError> {
        NodeTimeline::new(self, self.instance_or_error(instance_id)?)
    }

    /// The nodes of `instance_id` at one of its frames (`0..durationFrames`). For a whole edit
    /// frame (shot mapping, camera and nodes) use `evaluate_frame`.
    pub fn evaluate_instance_nodes(
        &self,
        instance_id: &str,
        instance_frame: u64,
    ) -> Result<NodeFrame, TemporalError> {
        self.node_timeline(instance_id)?.state_at(instance_frame)
    }
}
