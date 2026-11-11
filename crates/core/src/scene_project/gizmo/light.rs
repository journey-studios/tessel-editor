//! The point light's part of the gizmo glue (F060, G2.1): the overlay of a selected light, the
//! hit test of its handles and markers, the drag of its arrows and range knob, and the two
//! pointer-free equivalents (`scene.translate` on a light, `scene.range`).
//!
//! WHY it lives next to `gizmo.rs` and not in a copy of it: a light drag IS the gesture the node
//! drags are (`begin` on press, one `set` per move, `commit` on release, cancel discards, one
//! `Edit scene` entry, the same revision/frame/selection/epoch guards), because it is built from
//! the same pieces. What differs is only *what is written* (the light's `position` or `radius`,
//! by the same property paths `scene.edit` accepts) and *how the number is computed*, which is the
//! two pure functions of `scene_interaction::gizmo::light` that the numeric commands call too.

use super::*;
use crate::directed_scene::lighting::LightChannel;
use crate::scene_edit::LightProperty;
use crate::scene_interaction::{
    LightMarker, RangeAnchor, light_position, light_range, light_refusal, pick_marker,
};
use crate::scene_plan::PlanLight;

fn light_property(scene: &str, id: &str, property: LightProperty) -> PropertyRef {
    PropertyRef::light(scene, id, property)
}

fn read_number(scene: &DirectedScene, property: &PropertyRef) -> Result<f64, CommandError> {
    match read_property(scene, property)? {
        PropertyValue::Number(number) => Ok(number),
        _ => Err(CommandError::new(
            "internal",
            "a light's `radius` is a number in the catalog",
        )),
    }
}

fn not_translatable_light(entity: &str, reason: &str) -> CommandError {
    CommandError::new(
        "not_translatable",
        format!("`{entity}` cannot be moved: {reason}"),
    )
    .target(entity)
    .details(json!({ "reason": reason }))
}

fn not_rangeable(entity: &str, reason: &str) -> CommandError {
    CommandError::new(
        "not_rangeable",
        format!("the range of `{entity}` cannot change: {reason}"),
    )
    .target(entity)
    .details(json!({ "reason": reason }))
}

/// The light of `entity` in the plan, if the frame places it (the scene's lighting is supported).
fn planned<'a>(plan: &'a ScenePlan, entity: &Entity) -> Option<&'a PlanLight> {
    (entity.scope.as_deref() == Some(plan.source.scene_definition_id.as_str()))
        .then(|| plan.lights.iter().find(|light| light.id == entity.id))
        .flatten()
}

fn position_is_keyed(document: &DirectedScene, entity: &Entity) -> bool {
    entity
        .scope
        .as_deref()
        .and_then(|scope| document.bundle().scene(scope))
        .and_then(|graph| graph.light(&entity.id))
        .is_some_and(|light| light.is_keyed(LightChannel::Position.as_str()))
}

/// The handles of a selected light, or why there are none. `document` is the scene the plan
/// (`world`) was built from.
pub(super) fn light_layout(
    world: &Scene3d,
    view: &View,
    document: &DirectedScene,
    entity: &Entity,
) -> Result<Layout, Hidden> {
    let light = planned(&world.plan, entity).ok_or(Hidden::LightNotEvaluated)?;
    let animated = position_is_keyed(document, entity).then_some(light_refusal::ANIMATED_POSITION);
    Layout::light_at(
        view,
        [light.position.x, light.position.y, light.position.z],
        light.radius,
        animated,
    )
    .ok_or(Hidden::BehindCamera)
}

/// A marker for each light of the frame, in document order.
pub(super) fn light_markers(
    world: &Scene3d,
    view: Option<&View>,
    selected: &[&Entity],
) -> Vec<LightMarker> {
    let Some(view) = view else {
        return Vec::new();
    };
    let definition = &world.plan.source.scene_definition_id;
    world
        .plan
        .lights
        .iter()
        .filter_map(|light| {
            let entity = Entity::light(definition, &light.id);
            let at = view.project([light.position.x, light.position.y, light.position.z])?;
            Some(LightMarker {
                selected: selected.contains(&&entity),
                entity: entity_path(&entity),
                at,
                hidden: light.hidden,
            })
        })
        .collect()
}

/// The light a press at `point` lands on, if any: the nearest marker within reach.
pub(super) fn light_at_point(world: &Scene3d, view: &View, point: Point) -> Option<String> {
    let definition = &world.plan.source.scene_definition_id;
    let lights: Vec<(String, [f64; 3])> = world
        .plan
        .lights
        .iter()
        .map(|light| {
            (
                entity_path(&Entity::light(definition, &light.id)),
                [light.position.x, light.position.y, light.position.z],
            )
        })
        .collect();
    pick_marker(
        view,
        lights.iter().map(|(entity, at)| (entity.as_str(), *at)),
        point,
    )
    .map(str::to_string)
}

impl EditorState {
    /// Fills in the overlay for a selected (or dragged) light: the arrows and the range knob of
    /// the translate tool, or the reason the active tool has nothing for a light.
    pub(super) fn light_overlay(
        &self,
        mut overlay: Overlay,
        world: &Scene3d,
        shown: &DirectedScene,
        entity: &Entity,
    ) -> Overlay {
        overlay.entity = Some(entity_path(entity));
        match overlay.tool {
            GizmoTool::Rotate => {
                overlay.hidden = Some(Hidden::NotRotatable(light_refusal::NO_ROTATION));
                return overlay;
            }
            GizmoTool::Scale => {
                overlay.hidden = Some(Hidden::NotScalable(light_refusal::NO_SCALE));
                return overlay;
            }
            GizmoTool::Translate => {}
        }
        match world.view.as_ref() {
            Err(_) => overlay.hidden = Some(Hidden::NotMovable("degenerate_camera")),
            Ok(view) => match light_layout(world, view, shown, entity) {
                Ok(layout) => overlay.layout = Some(layout),
                Err(hidden) => overlay.hidden = Some(hidden),
            },
        }
        overlay
    }

    /// The press on a handle of a light: the arrows begin a move, the knob a range drag.
    pub(super) fn begin_light_drag(
        &mut self,
        scene: &str,
        world: &Scene3d,
        handle: Handle,
        parsed: &Entity,
        point: Point,
        answer: &mut Value,
    ) -> Result<(), CommandError> {
        let entity = entity_path(parsed);
        let view = world.view.as_ref().map_err(Clone::clone)?;
        let document = Arc::clone(&self.attached_scene(scene)?.scene);
        let light = planned(&world.plan, parsed)
            .ok_or_else(|| not_translatable_light(&entity, light_refusal::NOT_EVALUATED))?;
        let origin = [light.position.x, light.position.y, light.position.z];
        let scope = parsed.scope.as_deref().unwrap_or_default();
        let action = match handle {
            Handle::X | Handle::Y | Handle::Z | Handle::Screen => {
                // The same refusal the overlay shows as disabled handles; checked again because a
                // press may come from a script that never read the overlay.
                if position_is_keyed(&document, parsed) {
                    return Err(not_translatable_light(
                        &entity,
                        light_refusal::ANIMATED_POSITION,
                    ));
                }
                let Some(anchor) = Anchor::begin(view, origin, handle, point) else {
                    return Self::refused_press(answer);
                };
                let base_position = read_position(
                    &document,
                    &light_property(scope, &parsed.id, LightProperty::Position),
                )?;
                DragAction::LightTranslate {
                    anchor,
                    base_position,
                }
            }
            Handle::Range => {
                let Some(anchor) = RangeAnchor::begin(view, origin, point) else {
                    return Self::refused_press(answer);
                };
                let base_radius = read_number(
                    &document,
                    &light_property(scope, &parsed.id, LightProperty::Radius),
                )?;
                DragAction::LightRange {
                    anchor,
                    base_radius,
                }
            }
            Handle::Uniform => {
                return Err(CommandError::invalid_value(
                    "A light has the handles `x`, `y`, `z`, `screen` and `range`",
                    json!({ "options": ["x", "y", "z", "screen", "range"] }),
                )
                .target("handle"));
            }
        };
        let frame = i64::try_from(world.plan.frame.edit_frame).unwrap_or(i64::MAX);
        let label = if handle == Handle::Range {
            "Light range"
        } else {
            "Move light"
        };
        let begun = self.gesture_begin(
            scene,
            std::slice::from_ref(&entity),
            Some(label.to_string()),
        )?;
        let gesture = begun["gesture"].as_str().unwrap_or_default().to_string();
        self.scene_interaction.drag = Some(Drag {
            gesture: gesture.clone(),
            scene: scene.to_string(),
            entity,
            frame,
            node: None,
            action,
            last: None,
        });
        answer["gesture"] = json!(gesture);
        answer["dragging"] = json!(true);
        answer["handle"] = json!(handle.as_str());
        answer["tool"] = json!(GizmoTool::Translate.as_str());
        answer["changed"] = json!(true);
        Ok(())
    }

    /// One pointer position of the arrows' drag: the world delta becomes the candidate `position`,
    /// computed by the same [`light_position`] `scene.translate` uses.
    pub(super) fn light_translate_update(
        &mut self,
        drag: &Drag,
        anchor: &Anchor,
        base_position: Vec3,
        point: Point,
    ) -> Result<Value, CommandError> {
        let Some(world_delta) = anchor.delta(point) else {
            return Ok(json!({ "applied": false, "reason": "degenerate_pointer" }));
        };
        let position = match light_position(base_position, world_delta) {
            Ok(position) => position,
            Err(reason) => return Ok(json!({ "applied": false, "reason": reason })),
        };
        let property = PropertyRef::parse(&format!("{}/position", drag.entity))?;
        let update = self.gesture_update(
            &drag.gesture,
            vec![EditOp::set(property, PropertyValue::Vec3(position))],
        )?;
        if let Some(open) = self.scene_interaction.drag.as_mut() {
            open.last = Some(Applied::LightTranslate {
                world_delta,
                position,
            });
        }
        Ok(json!({
            "applied": true, "world_delta": world_delta.to_json(), "position": position.to_json(),
            "would_change": update["would_change"], "changes": update["changes"],
        }))
    }

    /// One pointer position of the knob's drag: the distance moved along the camera's right
    /// becomes the candidate `radius`, computed by the same [`light_range`] `scene.range` uses. A
    /// radius the rules refuse is `applied: false` with the typed reason and the last legal
    /// candidate stays, so a release there still commits what was last legal.
    pub(super) fn light_range_update(
        &mut self,
        drag: &Drag,
        anchor: &RangeAnchor,
        base_radius: f64,
        point: Point,
    ) -> Result<Value, CommandError> {
        let Some(delta) = anchor.delta(point) else {
            return Ok(json!({ "applied": false, "reason": "degenerate_pointer" }));
        };
        let radius = match light_range(base_radius, delta) {
            Ok(radius) => radius,
            Err(reason) => {
                return Ok(json!({ "applied": false, "reason": reason, "delta": delta }));
            }
        };
        let property = PropertyRef::parse(&format!("{}/radius", drag.entity))?;
        let update = self.gesture_update(
            &drag.gesture,
            vec![EditOp::set(property, PropertyValue::Number(radius))],
        )?;
        if let Some(open) = self.scene_interaction.drag.as_mut() {
            open.last = Some(Applied::LightRange { delta, radius });
        }
        Ok(json!({
            "applied": true, "delta": delta, "radius": radius,
            "would_change": update["would_change"], "changes": update["changes"],
        }))
    }

    /// `scene.translate` of a light: the arrows' drag without the pointer, one `scene.edit` of the
    /// light's `position` (the path a gesture commit shares), through the same
    /// [`light_position`].
    pub(super) fn scene_translate_light(
        &mut self,
        scene: &str,
        parsed: &Entity,
        at: &FrameAt,
        delta: Vec3,
    ) -> Result<Value, CommandError> {
        let entity = entity_path(parsed);
        let document = Arc::clone(&self.attached_scene(scene)?.scene);
        if !finite(delta) {
            return Err(CommandError::invalid_value(
                "`delta` must be finite numbers",
                json!({ "x": "number", "y": "number", "z": "number" }),
            )
            .target("delta"));
        }
        let frame = frame_of(&document, at)?;
        let scope = parsed.scope.as_deref().unwrap_or_default();
        let property = light_property(scope, &parsed.id, LightProperty::Position);
        let base = read_position(&document, &property)?;
        let position = light_position(base, delta)
            .map_err(|reason| not_translatable_light(&entity, reason))?;
        let mut answer = self.edit_scene(
            scene,
            &[EditOp::set(property, PropertyValue::Vec3(position))],
            false,
        )?;
        answer["action"] = json!("scene.translate");
        answer["entity"] = json!(entity);
        answer["frame"] = json!(frame);
        answer["world_delta"] = delta.to_json();
        answer["position"] = position.to_json();
        answer["document_changed"] = answer["changed"].clone();
        Ok(answer)
    }

    /// `scene.range`: the knob's drag without the pointer. One light, a signed `delta` in scene
    /// units added to its radius, written by `edit_scene` (the path `scene.edit` and a gesture
    /// commit share), so the same delta gives the same document by either road and a delta of 0
    /// writes nothing (no history entry).
    pub(super) fn scene_range(
        &mut self,
        scene: &str,
        entity: &str,
        at: &FrameAt,
        delta: f64,
    ) -> Result<Value, CommandError> {
        let parsed = parse_entity(entity)?;
        let document = Arc::clone(&self.attached_scene(scene)?.scene);
        ensure_exists(document.bundle(), &parsed)?;
        if parsed.kind != EntityKind::Light {
            return Err(not_rangeable(entity, "only a point light has a range"));
        }
        if !delta.is_finite() {
            return Err(CommandError::invalid_value(
                "`delta` must be a finite number",
                json!("number"),
            )
            .target("delta"));
        }
        let frame = frame_of(&document, at)?;
        let scope = parsed.scope.as_deref().unwrap_or_default();
        let property = light_property(scope, &parsed.id, LightProperty::Radius);
        let base = read_number(&document, &property)?;
        let radius = light_range(base, delta).map_err(|reason| not_rangeable(entity, reason))?;
        let mut answer = self.edit_scene(
            scene,
            &[EditOp::set(property, PropertyValue::Number(radius))],
            false,
        )?;
        answer["action"] = json!("scene.range");
        answer["entity"] = json!(entity);
        answer["frame"] = json!(frame);
        answer["delta"] = json!(delta);
        answer["radius"] = json!(radius);
        answer["document_changed"] = answer["changed"].clone();
        Ok(answer)
    }
}
