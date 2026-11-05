//! Shared directed-scene request preparation for headless and native-window adapters.
use crate::{FrameOptions, ResolvedResource, SceneResourceMap};
use serde_json::{Value, json};
use tessel_editor_core::headless::backend_error;
use tessel_editor_core::ops::{Op, parse_op};
use tessel_editor_core::scene_plan::ScenePlan;
use tessel_editor_core::{CommandError, EditorState};
use tessel_editor_media::scene_resources::{ResourceResolver, SceneResourceSession};

/// Maps only files that the media adapter has already verified. No file hashing on the UI thread.
pub fn approved_map(
    plan: &ScenePlan,
    scene: &str,
    session: &SceneResourceSession,
) -> Result<SceneResourceMap, CommandError> {
    let mut map = SceneResourceMap::new(session.generation().0);
    for resource in &plan.resources {
        let approved = session
            .resolve_asset(scene, &resource.asset_id)
            .map_err(|error| CommandError::new(error.code(), error.to_string()))?;
        map.insert(
            resource.asset_id.clone(),
            ResolvedResource {
                revision_id: approved.revision_id.unwrap_or_default(),
                content_hash: approved.content_hash,
                path: approved.path,
            },
        )
        .map_err(backend_error)?;
    }
    Ok(map)
}

/// Uses the same core parser, admission and frame-geometry validation as the ordinary renderer.
pub fn prepare(
    state: &mut EditorState,
    request: &Value,
    session: &SceneResourceSession,
) -> Result<(ScenePlan, SceneResourceMap, FrameOptions), CommandError> {
    let mut plan_request = request.clone();
    plan_request["op"] = json!("scene.plan");
    plan_request["partial"] = json!(false);
    let Some(Op::Run(command)) = parse_op(state, &plan_request)? else {
        return Err(CommandError::new(
            "invalid_request",
            "Expected a scene frame request",
        ));
    };
    let answer = state.execute(command)?;
    let plan = ScenePlan::from_json(&answer["plan"])?;
    let scene = request["scene"]
        .as_str()
        .ok_or_else(|| CommandError::new("invalid_request", "Expected scene id"))?;
    session.observe_project(&state.project.id);
    let map = approved_map(&plan, scene, session)?;
    let mut frame_request = request.clone();
    frame_request["op"] = json!("render.frame");
    let Some(Op::RenderFrame(frame)) = parse_op(state, &frame_request)? else {
        return Err(CommandError::new(
            "invalid_request",
            "Expected frame output options",
        ));
    };
    let width = u32::try_from(plan.output.width)
        .map_err(|_| CommandError::new("invalid_request", "Invalid scene width"))?;
    let height = u32::try_from(plan.output.height)
        .map_err(|_| CommandError::new("invalid_request", "Invalid scene height"))?;
    let (width, height) = tessel_editor_core::headless::output_geometry(width, height, &frame)?;
    Ok((
        plan,
        map,
        FrameOptions {
            size: Some((width, height)),
            png: frame.out,
            probes: frame.probes,
            inspect: true,
        },
    ))
}
