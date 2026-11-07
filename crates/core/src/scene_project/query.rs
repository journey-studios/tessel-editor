//! The `scene.*` commands that only read. They take `&SceneBook`, so they cannot change the
//! project, and `SceneCommand::history_label` gives them no history entry.
//!
//! Nothing here evaluates or validates by itself: properties come from `scene_edit`, camera
//! frames from the importer's `Bundle::evaluate_camera` / `camera_at_clip_frame` /
//! `frame_at_time`, and the JSON keys are the ones `FrameEvaluation::to_json` already uses, so a
//! consumer reads a camera the same way wherever it comes from.

use serde_json::{Value, json};

use crate::directed_scene::{CameraState, DirectedScene, FrameMapping};
use crate::scene_edit::{self, Applicability, NumberRange, PropertySpec};
use crate::state::CommandError;
use crate::time::Ratio;

use super::command::{CameraAt, SceneCommand};
use super::scope::{deferred_in_use, scope_json};
use super::{AttachedScene, MAX_SCENES, SceneBook};

fn attached<'a>(book: &'a SceneBook, id: &str) -> Result<&'a AttachedScene, CommandError> {
    book.get(id)
        .ok_or_else(|| CommandError::unknown_element("scene", id))
}

pub(super) fn notices_json(scene: &DirectedScene) -> Vec<Value> {
    scene
        .notices()
        .iter()
        .map(
            |notice| json!({ "code": notice.code, "path": notice.path, "message": notice.message }),
        )
        .collect()
}

pub(super) fn counts(scene: &DirectedScene) -> Value {
    let bundle = scene.bundle();
    json!({
        "definitions": bundle.scenes.len(),
        "nodes": bundle.scenes.iter().map(|graph| graph.nodes().len()).sum::<usize>(),
        "instances": bundle.instances.len(),
        "shots": bundle.shots.len(),
        "camera_clips": bundle.camera_clips.len(),
        "performance_tracks": bundle.performance_tracks.len(),
        "assets": bundle.assets.len(),
    })
}

fn summary(entry: &AttachedScene) -> Value {
    let bundle = entry.scene.bundle();
    json!({
        "id": entry.id, "name": entry.name,
        "bundle_id": bundle.bundle_id, "revision_id": bundle.revision_id,
        "counts": counts(&entry.scene),
    })
}

fn range_json(range: &NumberRange) -> Value {
    let limit = |limit: Option<crate::scene_edit::Limit>| {
        limit.map(|limit| json!({ "value": limit.value, "inclusive": limit.inclusive }))
    };
    json!({ "min": limit(range.min), "max": limit(range.max), "description": range.describe() })
}

fn spec_json(spec: &PropertySpec) -> Value {
    let (target, path_pattern) = match spec.applies_to {
        Applicability::CameraClip => ("camera-clip", format!("camera/<clip>/{}", spec.name)),
        Applicability::Node(_) => ("node", format!("scene/<scene>/node/<node>/{}", spec.name)),
    };
    json!({
        "name": spec.name,
        "path": path_pattern,
        "target": target,
        "applies_to": spec.applies_to.describe(),
        "kind": spec.kind.as_str(),
        "shape": spec.kind.shape(),
        "unit": spec.unit.as_str(),
        "range": spec.range.as_ref().map(range_json),
        "default": spec.default.as_ref().map(|value| value.to_json()),
        "document_field": spec.document_field,
        "origin": spec.origin.as_str(),
        // The property this one is a component of (`position` for `position/x`), or null.
        "component_of": spec.component.map(|component| json!({
            "parent": component.parent.spec().name,
            "index": component.index,
        })),
        // An absent field in the bundle reads as `default`.
        "optional": spec.optional,
        // A change first turns the clip's first movement into keyframes (the origin's write).
        "converts_first_movement": spec.converts_first_movement,
    })
}

fn describe(entry: &AttachedScene) -> Value {
    let scene = &entry.scene;
    let bundle = scene.bundle();
    let definitions: Vec<Value> = bundle
        .scenes
        .iter()
        .map(|graph| {
            let nodes: Vec<Value> = graph
                .nodes()
                .iter()
                .map(|node| json!({ "id": node.id, "kind": node.kind.as_str() }))
                .collect();
            json!({ "id": graph.id(), "nodes": nodes })
        })
        .collect();
    let shots: Vec<Value> = bundle
        .shots
        .iter()
        .map(|shot| {
            json!({
                "id": shot.id, "instance": shot.scene_instance_id,
                "start_frame": shot.start_frame, "end_frame_exclusive": shot.end_frame_exclusive,
                "camera_clip": shot.camera_clip_id,
            })
        })
        .collect();
    let instances: Vec<Value> = bundle
        .instances
        .iter()
        .map(|instance| {
            json!({
                "id": instance.id, "definition": instance.scene_definition_id,
                "duration_frames": instance.duration_frames,
            })
        })
        .collect();
    let mut described = summary(entry);
    described["action"] = json!("scene.describe");
    described["scene"] = json!(entry.id);
    described["fps"] = json!({
        "numerator": bundle.fps.numerator, "denominator": bundle.fps.denominator,
    });
    described["edit_duration_frames"] = json!(bundle.edit_duration_frames);
    described["definitions"] = Value::Array(definitions);
    described["instances"] = Value::Array(instances);
    described["shots"] = Value::Array(shots);
    described["camera_clips"] = json!(
        bundle
            .camera_clips
            .iter()
            .map(|clip| clip.id.as_str())
            .collect::<Vec<_>>()
    );
    described["notices"] = Value::Array(notices_json(scene));
    described["deferred_in_use"] = Value::Array(deferred_in_use(scene.document()));
    described["unknown_entry_keys"] = json!(entry.extra.keys().collect::<Vec<_>>());
    described
}

fn camera_json(mapping: &FrameMapping<'_>, camera: &CameraState) -> Value {
    json!({
        "editFrame": mapping.edit_frame,
        "shotId": mapping.shot.id,
        "instanceId": mapping.instance.id,
        "instanceFrame": mapping.instance_frame,
        "cameraClipId": mapping.shot.camera_clip_id,
        "cameraClipFrame": mapping.camera_clip_frame,
        "editTime": mapping.edit_time.to_json(),
        "instanceTime": mapping.instance_time.to_json(),
        "cameraClipTime": mapping.camera_clip_time.to_json(),
        "camera": camera_state_json(camera),
    })
}

fn camera_state_json(camera: &CameraState) -> Value {
    json!({
        "position": camera.position.to_json(),
        "target": camera.target.to_json(),
        "fovDeg": camera.fov_deg,
        "velocity": camera.velocity.to_json(),
        "fovVelocityDegPerFrame": camera.fov_velocity_deg_per_frame,
    })
}

fn camera(scene: &DirectedScene, at: &CameraAt) -> Result<Value, CommandError> {
    let bundle = scene.bundle();
    let edit_frame = |frame: i64| -> Result<Value, CommandError> {
        let evaluation = bundle.evaluate_camera(frame)?;
        Ok(camera_json(&evaluation.mapping, &evaluation.camera))
    };
    match at {
        CameraAt::EditFrame(frame) => edit_frame(*frame),
        CameraAt::EditTime(time) => edit_frame(frame_at(scene, *time)?),
        CameraAt::ClipFrame { clip, frame } => {
            let state = bundle.camera_at_clip_frame(clip, *frame)?;
            Ok(json!({
                "cameraClipId": clip,
                "cameraClipFrame": frame,
                "camera": camera_state_json(&state),
            }))
        }
    }
}

fn frame_at(scene: &DirectedScene, time: Ratio) -> Result<i64, CommandError> {
    Ok(scene.bundle().frame_at_time(time)?)
}

/// Runs a read-only command. The writing ones never reach it (see `EditorState::apply_scene`).
pub(super) fn run(book: &SceneBook, command: SceneCommand) -> Result<Value, CommandError> {
    match command {
        SceneCommand::List => {
            let scenes: Vec<Value> = book.iter().map(summary).collect();
            Ok(json!({
                "action": "scene.list", "count": scenes.len(), "max": MAX_SCENES,
                "scenes": scenes,
            }))
        }
        SceneCommand::Capabilities => Ok(json!({
            "action": "scene.capabilities",
            "commands": [
                "scene.attach", "scene.detach", "scene.edit", "scene.list", "scene.describe",
                "scene.catalog", "scene.properties", "scene.get", "scene.camera",
                "scene.capabilities",
                // F039 (docs/features/scene-interaction.md).
                "scene.tree", "scene.evaluate", "scene.plan", "scene.select", "scene.selection",
                "scene.gesture.begin", "scene.gesture.update", "scene.gesture.commit",
                "scene.gesture.cancel", "scene.gesture.status",
            ],
            "max_scenes": MAX_SCENES,
            // Additive contract of the property catalog (docs/features/scene-edit.md): a caller
            // that does not read it gets the default property listing (including renderOrder).
            "edit": {
                "catalog_version": 2,
                "components": true,
                "render_order": true,
                "camera_start_conversion": "origin-keyframes",
                "batch": "atomic",
                "properties_option_components": true,
            },
            "scope": scope_json(),
        })),
        SceneCommand::Catalog => {
            let properties: Vec<Value> = scene_edit::catalog()
                .iter()
                .map(|spec| spec_json(spec))
                .collect();
            Ok(json!({
                "action": "scene.catalog", "count": properties.len(), "properties": properties,
            }))
        }
        SceneCommand::Describe { scene } => Ok(describe(attached(book, &scene)?)),
        SceneCommand::Properties {
            scene,
            path_prefix,
            components,
        } => {
            let entry = attached(book, &scene)?;
            let listed = if components {
                scene_edit::properties_with_components(&entry.scene)
            } else {
                scene_edit::properties_of(&entry.scene)
            };
            let properties: Vec<Value> = listed
                .into_iter()
                .filter(|info| {
                    path_prefix
                        .as_deref()
                        .is_none_or(|prefix| info.property.path().starts_with(prefix))
                })
                .map(|info| {
                    let spec = info.spec();
                    json!({
                        "path": info.property.path(),
                        "kind": spec.kind.as_str(),
                        "unit": spec.unit.as_str(),
                        "value": info.value.to_json(),
                    })
                })
                .collect();
            Ok(json!({
                "action": "scene.properties", "scene": scene, "count": properties.len(),
                "properties": properties,
            }))
        }
        SceneCommand::Get { scene, property } => {
            let entry = attached(book, &scene)?;
            let value = scene_edit::read_property(&entry.scene, &property)?;
            let spec = property.spec();
            Ok(json!({
                "action": "scene.get", "scene": scene, "property": property.path(),
                "kind": spec.kind.as_str(), "unit": spec.unit.as_str(), "value": value.to_json(),
            }))
        }
        SceneCommand::Camera { scene, at } => {
            let entry = attached(book, &scene)?;
            let mut answer = camera(&entry.scene, &at)?;
            answer["action"] = json!("scene.camera");
            answer["scene"] = json!(scene);
            Ok(answer)
        }
        // The F039 commands also need the session state (selection, gesture), which this
        // function does not get: they run in `interaction.rs`.
        SceneCommand::Attach { .. }
        | SceneCommand::Detach { .. }
        | SceneCommand::Edit { .. }
        | SceneCommand::Tree { .. }
        | SceneCommand::Evaluate { .. }
        | SceneCommand::Plan { .. }
        | SceneCommand::Select { .. }
        | SceneCommand::Selection
        | SceneCommand::Gesture(_) => Err(CommandError::new(
            "internal",
            "a scene command reached the read path that does not serve it",
        )),
    }
}
