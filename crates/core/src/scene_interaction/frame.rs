//! `scene.evaluate`: the whole evaluated frame of an attached scene (BL020) as JSON.
//!
//! Nothing is computed here. The frame comes from `Bundle::evaluate_frame` (BB03: shot mapping,
//! camera, performance channels, world matrices) and its own `FrameEvaluation::to_json`, so the
//! camera block is the one `scene.camera` prints and a consumer reads both the same way. What
//! this adds is the entity address of every node, which is how the tree and a selection name it.
//!
//! The honest boundary travels in the answer: `complete` and `unevaluated` list what the scene
//! declares and no evaluator computes yet (lighting, weather, rigs...), so a partial frame never
//! passes for the whole picture.

use serde_json::{Value, json};

use crate::directed_scene::{DirectedScene, Entity};
use crate::state::CommandError;

use super::address::entity_path;
use super::command::FrameAt;

/// The frame as a renderer or an adapter reads it. Errors are the evaluator's typed ones
/// (`frame_out_of_range`, `invalid_animation`, `invalid_camera`, `invalid_time`).
pub fn frame_json(scene: &DirectedScene, at: &FrameAt) -> Result<Value, CommandError> {
    let bundle = scene.bundle();
    let frame = match at {
        FrameAt::Frame(frame) => *frame,
        FrameAt::Time(time) => bundle.frame_at_time(*time)?,
    };
    let evaluation = bundle.evaluate_frame(frame)?;
    let definition = evaluation.nodes.scene_definition_id().to_string();
    let mut answer = evaluation.to_json();
    if let Some(Value::Array(nodes)) = answer.get_mut("nodes") {
        for node in nodes {
            let Some(id) = node
                .get("nodeId")
                .and_then(Value::as_str)
                .map(str::to_string)
            else {
                continue;
            };
            node["path"] = json!(entity_path(&Entity::node(&definition, &id)));
        }
    }
    Ok(answer)
}
