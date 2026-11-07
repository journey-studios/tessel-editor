//! Running the F052 gizmo commands (`scene.gizmo.state|hit`, `scene.pointer.*`, `scene.translate`)
//! against a project. The geometry (projection, hit test, drag math) is
//! `scene_interaction::gizmo`; this file is the glue that needs `EditorState`: the selection, the
//! gesture slot and `edit_scene`.
//!
//! WHY a pointer drag is a *gesture* and not a second mechanism: `down` on a handle runs
//! `scene.gesture.begin`, every `move` runs `scene.gesture.update` with ONE `set position`, and
//! `up` runs `scene.gesture.commit`. Cancel is `scene.gesture.cancel`. So the candidate stays
//! out of the document, a rejected value keeps the last good candidate, a stale gesture is
//! refused with the typed `gesture_invalidated`, and the commit is one `Edit scene` entry: all of
//! that is the existing contract, not something this file re-implements. What the drag adds is
//! only [`Drag`]: the press (camera, node, handle, base position) that turns a pointer position
//! into the `position` value.
//!
//! `scene.translate` is the same last step without the pointer: a world delta, converted the same
//! way, written by `edit_scene` (the path `scene.edit` and a commit share). The same delta gives
//! the same document by either road; a test pins it.

use serde_json::{Value, json};

use crate::directed_scene::{DirectedScene, Entity, EntityKind, Vec3};
use crate::scene_edit::{EditOp, NodeProperty, PropertyRef, PropertyValue, read_property};
use crate::scene_interaction::{
    Anchor, CancelReason, Drag, FrameAt, GizmoCommand, Handle, Hidden, Layout, Overlay, Point,
    PointerCommand, SelectMode, View, ensure_exists, entity_path, local_delta, movable, outline,
    parse_entity, pick_plane, world_origin,
};
use crate::scene_plan::{PlanOptions, ScenePlan};
use crate::state::{CommandError, EditorState};

/// The plan the gizmo works on. Partial on purpose: picking and handles need only the camera and
/// the nodes, so a scene with a domain the renderer cannot draw yet can still be selected and
/// moved (the renderer refuses to draw it; that is its own, typed answer).
fn plan_of(scene: &DirectedScene, frame: i64) -> Result<ScenePlan, CommandError> {
    Ok(ScenePlan::build_with(
        scene,
        frame,
        PlanOptions {
            allow_partial: true,
        },
    )?)
}

fn frame_of(scene: &DirectedScene, at: &FrameAt) -> Result<i64, CommandError> {
    match at {
        FrameAt::Frame(frame) => Ok(*frame),
        FrameAt::Time(time) => Ok(scene.bundle().frame_at_time(*time)?),
    }
}

fn position_ref(plan: &ScenePlan, node: &str) -> PropertyRef {
    PropertyRef::node(
        &plan.source.scene_definition_id,
        node,
        NodeProperty::Position,
    )
}

fn address_of(plan: &ScenePlan, node: &str) -> String {
    entity_path(&Entity::node(&plan.source.scene_definition_id, node))
}

fn read_position(scene: &DirectedScene, property: &PropertyRef) -> Result<Vec3, CommandError> {
    match read_property(scene, property)? {
        PropertyValue::Vec3(position) => Ok(position),
        _ => Err(CommandError::new(
            "internal",
            "`position` is not a vector in the catalog",
        )),
    }
}

fn hit_json(hit: &Hit) -> Value {
    match hit {
        Hit::Handle { handle, entity } => json!({
            "kind": "handle", "handle": handle.as_str(), "entity": entity,
        }),
        Hit::Entity { entity } => json!({ "kind": "entity", "entity": entity }),
        Hit::Nothing => json!({ "kind": "none" }),
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Hit {
    Handle { handle: Handle, entity: String },
    Entity { entity: String },
    Nothing,
}

/// The target of a gizmo and what is drawn, for one plan.
struct Scene3d {
    plan: ScenePlan,
    view: Result<View, CommandError>,
}

impl Scene3d {
    fn new(scene: &DirectedScene, frame: i64) -> Result<Self, CommandError> {
        let plan = plan_of(scene, frame)?;
        let view = View::from_plan(&plan);
        Ok(Self { plan, view })
    }
}

/// Never a position whose JSON would carry a non-finite number.
fn finite(vec: Vec3) -> bool {
    vec.x.is_finite() && vec.y.is_finite() && vec.z.is_finite()
}

impl EditorState {
    pub(super) fn apply_gizmo(&mut self, command: GizmoCommand) -> Result<Value, CommandError> {
        match command {
            GizmoCommand::State { scene, at } => {
                let frame = frame_of(&self.attached_scene(&scene)?.scene, &at)?;
                Ok(json!({
                    "action": "scene.gizmo.state", "scene": scene, "frame": frame,
                    "changed": false, "document_changed": false,
                    "overlay": self.gizmo_overlay(&scene, frame)?.to_json(),
                }))
            }
            GizmoCommand::Hit { scene, at, point } => self.gizmo_hit(&scene, &at, point),
            GizmoCommand::Pointer(pointer) => match pointer {
                PointerCommand::Down {
                    scene,
                    at,
                    point,
                    additive,
                } => self.pointer_down(&scene, &at, point, additive),
                PointerCommand::Move { point } => self.pointer_move(point),
                PointerCommand::Up { point } => self.pointer_up(point),
                PointerCommand::Cancel { reason } => self.pointer_cancel(reason),
            },
            GizmoCommand::Translate {
                scene,
                entity,
                at,
                delta,
            } => self.scene_translate(&scene, &entity, &at, delta),
        }
    }

    /// The identity of what an overlay shows: scene, interaction session (a new project is a new
    /// session), project revision and edit frame. An overlay drawn for one stamp must not
    /// answer to a pointer message that was made against another: a message from the page that
    /// was reloaded, from the project that was replaced, or from the frame before a seek is
    /// refused by the adapter by comparing this value and nothing else.
    pub fn gizmo_stamp(&self, scene: &str, frame: i64) -> Value {
        json!({
            "scene": scene,
            "session": self.scene_interaction.gesture.session(),
            "revision": self.project.revision,
            "frame": frame,
        })
    }

    /// `true` when a drag is open whose gesture can no longer be applied (the scene was edited,
    /// replaced or detached under it, or its gesture was settled by something else). Nobody will
    /// release such a drag usefully; the adapter cancels it. Reads only.
    pub fn gizmo_drag_is_dead(&self) -> bool {
        let Some(drag) = self.scene_interaction.drag.as_ref() else {
            return false;
        };
        match self.scene_interaction.gesture.current() {
            Some(gesture) if gesture.id() == drag.gesture => gesture
                .invalidation(&self.scenes, self.project.revision)
                .is_some(),
            _ => true,
        }
    }

    /// The drag in progress, or `None` after dropping one whose gesture is no longer the open
    /// one (another command settled it, the project was replaced): a dead drag never revives.
    fn live_drag(&mut self) -> Option<Drag> {
        let drag = self.scene_interaction.drag.as_ref()?;
        let alive = self
            .scene_interaction
            .gesture
            .current()
            .is_some_and(|gesture| gesture.id() == drag.gesture);
        if alive {
            return self.scene_interaction.drag.clone();
        }
        self.scene_interaction.drag = None;
        None
    }

    /// What a runtime draws for `scene` at `frame`: the outline of each selected image plane and
    /// the handles of the one selected node (or of the node being dragged, whose candidate is
    /// what the plan is built from).
    fn gizmo_overlay(&mut self, scene_id: &str, frame: i64) -> Result<Overlay, CommandError> {
        // A drag whose gesture went stale (an undo, an edit elsewhere) draws as no drag: its
        // candidate is the scene as it was, not what the document is.
        let dragging = self.live_drag().filter(|drag| {
            drag.scene == scene_id
                && self
                    .scene_interaction
                    .gesture
                    .current()
                    .is_some_and(|gesture| {
                        gesture
                            .invalidation(&self.scenes, self.project.revision)
                            .is_none()
                    })
        });
        let entry = self.attached_scene(scene_id)?;
        let document = std::sync::Arc::clone(&entry.scene);
        let shown = match &dragging {
            Some(drag) => self
                .scene_interaction
                .gesture
                .find(&drag.gesture)
                .map_or(document, |gesture| {
                    std::sync::Arc::clone(gesture.preview_scene())
                }),
            None => document,
        };
        let world = Scene3d::new(&shown, frame)?;
        let definition = world.plan.source.scene_definition_id.as_str();
        let selected: Vec<&Entity> = self
            .scene_interaction
            .selection
            .entities_of(scene_id)
            .collect();

        let view = world.view.as_ref().ok();
        let mut outlines = Vec::new();
        for entity in &selected {
            if entity.kind != EntityKind::Node || entity.scope.as_deref() != Some(definition) {
                continue;
            }
            if let (Some(view), Some(node)) = (view, world.plan.node(&entity.id))
                && let Some(quad) = outline(view, node)
            {
                outlines.push((entity_path(entity), quad));
            }
        }

        let mut overlay = Overlay {
            entity: None,
            layout: None,
            hidden: None,
            outlines,
            active: dragging
                .as_ref()
                .map(|drag| (drag.handle(), drag.gesture.clone())),
            aspect: view.map_or(1.0, View::aspect),
            stamp: self.gizmo_stamp(scene_id, frame),
        };
        // The node the handles belong to: the one being dragged, else the single selected node.
        let node_id = match (&dragging, selected.as_slice()) {
            (Some(drag), _) => Some(drag.node.id.clone()),
            (None, []) => {
                overlay.hidden = Some(Hidden::NoSelection);
                None
            }
            (None, [entity]) if entity.kind != EntityKind::Node => {
                overlay.hidden = Some(Hidden::NotANode);
                None
            }
            (None, [entity]) if entity.scope.as_deref() != Some(definition) => {
                overlay.hidden = Some(Hidden::NotInFrame);
                None
            }
            (None, [entity]) => Some(entity.id.clone()),
            (None, _) => {
                overlay.hidden = Some(Hidden::MultipleSelection);
                None
            }
        };
        let Some(node_id) = node_id else {
            return Ok(overlay);
        };
        overlay.entity = Some(address_of(&world.plan, &node_id));
        let Some(node) = world.plan.node(&node_id) else {
            overlay.hidden = Some(Hidden::NotInFrame);
            return Ok(overlay);
        };
        if let Err(reason) = movable(node) {
            overlay.hidden = Some(Hidden::NotMovable(reason));
            return Ok(overlay);
        }
        match view {
            None => overlay.hidden = Some(Hidden::NotMovable("degenerate_camera")),
            Some(view) => match Layout::at(view, world_origin(node)) {
                Some(layout) => overlay.layout = Some(layout),
                None => overlay.hidden = Some(Hidden::BehindCamera),
            },
        }
        Ok(overlay)
    }

    /// What is under `point`, on the document's scene: a handle of the selected node first,
    /// then the front-most image plane. Reads only.
    fn hit_at(
        &self,
        scene_id: &str,
        world: &Scene3d,
        point: Point,
    ) -> Result<(Hit, Option<(Layout, String)>), CommandError> {
        let view = world.view.as_ref().map_err(Clone::clone)?;
        let definition = world.plan.source.scene_definition_id.as_str();
        let selected: Vec<&Entity> = self
            .scene_interaction
            .selection
            .entities_of(scene_id)
            .collect();
        let mut handles = None;
        if let [one] = selected.as_slice()
            && one.kind == EntityKind::Node
            && one.scope.as_deref() == Some(definition)
            && let Some(node) = world.plan.node(&one.id)
            && movable(node).is_ok()
            && let Some(layout) = Layout::at(view, world_origin(node))
        {
            let entity = entity_path(one);
            if let Some(handle) = layout.hit(view, point) {
                return Ok((
                    Hit::Handle {
                        handle,
                        entity: entity.clone(),
                    },
                    Some((layout, entity)),
                ));
            }
            handles = Some((layout, entity));
        }
        let hit = match pick_plane(&world.plan, view, point) {
            Some(node) => Hit::Entity {
                entity: address_of(&world.plan, &node.id),
            },
            None => Hit::Nothing,
        };
        Ok((hit, handles))
    }

    fn gizmo_hit(&self, scene: &str, at: &FrameAt, point: Point) -> Result<Value, CommandError> {
        let point = point.checked(false)?;
        let entry = self.attached_scene(scene)?;
        let frame = frame_of(&entry.scene, at)?;
        let world = Scene3d::new(&entry.scene, frame)?;
        let (hit, _) = self.hit_at(scene, &world, point)?;
        Ok(json!({
            "action": "scene.gizmo.hit", "scene": scene, "frame": frame,
            "point": point.to_json(), "hit": hit_json(&hit),
            "changed": false, "document_changed": false,
        }))
    }

    fn pointer_down(
        &mut self,
        scene: &str,
        at: &FrameAt,
        point: Point,
        additive: bool,
    ) -> Result<Value, CommandError> {
        let point = point.checked(true)?;
        let entry = self.attached_scene(scene)?;
        let document = std::sync::Arc::clone(&entry.scene);
        let frame = frame_of(&document, at)?;
        let world = Scene3d::new(&document, frame)?;
        // A press while a drag is open means its release was lost (a second pointer, a missed
        // event). The open drag is discarded first, so a press never leaves two of them.
        let mut abandoned = Value::Null;
        if let Some(drag) = self.live_drag() {
            abandoned = json!({ "gesture": drag.gesture, "reason": "replaced_by_press" });
            self.discard_drag();
        }
        let (hit, _) = self.hit_at(scene, &world, point)?;
        let mut answer = json!({
            "action": "scene.pointer", "phase": "down", "scene": scene, "frame": frame,
            "point": point.to_json(), "hit": hit_json(&hit), "abandoned": abandoned,
            "gesture": null, "dragging": false, "selection_changed": false,
            "changed": false, "document_changed": false,
        });
        match &hit {
            Hit::Handle { handle, entity } => {
                self.begin_drag(scene, &world, *handle, entity, point, &mut answer)?;
            }
            Hit::Entity { entity } => {
                let mode = if additive {
                    SelectMode::Toggle
                } else {
                    SelectMode::Replace
                };
                let outcome = self.scene_select(Some(scene), std::slice::from_ref(entity), mode)?;
                answer["selection_changed"] = outcome["changed"].clone();
                answer["changed"] = outcome["changed"].clone();
            }
            Hit::Nothing if !additive => {
                let outcome = self.scene_select(Some(scene), &[], SelectMode::Clear)?;
                answer["selection_changed"] = outcome["changed"].clone();
                answer["changed"] = outcome["changed"].clone();
            }
            Hit::Nothing => {}
        }
        answer["overlay"] = self.gizmo_overlay(scene, frame)?.to_json();
        Ok(answer)
    }

    fn begin_drag(
        &mut self,
        scene: &str,
        world: &Scene3d,
        handle: Handle,
        entity: &str,
        point: Point,
        answer: &mut Value,
    ) -> Result<(), CommandError> {
        let view = world.view.as_ref().map_err(Clone::clone)?;
        let parsed = parse_entity(entity)?;
        let node = world
            .plan
            .node(&parsed.id)
            .ok_or_else(|| CommandError::unknown_element("node", entity))?;
        // `hit_at` only offers handles of a movable node in front of the camera, so these cannot
        // fail here; they are checked anyway because a drag built on them would be wrong.
        movable(node).map_err(|reason| {
            CommandError::new(
                "not_translatable",
                format!("`{entity}` cannot be moved ({reason})"),
            )
            .target(entity)
        })?;
        let origin = world_origin(node);
        let Some(anchor) = Anchor::begin(view, origin, handle, point) else {
            // The pointer looks along the axis: it says nothing about how far along it is. Refused
            // before a gesture opens, so nothing is left to cancel.
            answer["dragging"] = json!(false);
            answer["refused"] = json!("degenerate_axis");
            return Ok(());
        };
        let document = &self.attached_scene(scene)?.scene;
        let base_position = read_position(document, &position_ref(&world.plan, &node.id))?;
        let frame = i64::try_from(world.plan.frame.edit_frame).unwrap_or(i64::MAX);
        let begun = self.gesture_begin(scene, &[entity.to_string()], Some("Move".to_string()))?;
        let gesture = begun["gesture"].as_str().unwrap_or_default().to_string();
        self.scene_interaction.drag = Some(Drag {
            gesture: gesture.clone(),
            scene: scene.to_string(),
            entity: entity.to_string(),
            frame,
            anchor,
            node: node.clone(),
            base_position,
            last: None,
        });
        answer["gesture"] = json!(gesture);
        answer["dragging"] = json!(true);
        answer["handle"] = json!(handle.as_str());
        answer["changed"] = json!(true);
        Ok(())
    }

    /// Drops the drag and the gesture under it (never touches the document).
    fn discard_drag(&mut self) {
        if let Some(drag) = self.scene_interaction.drag.take()
            && self
                .scene_interaction
                .gesture
                .current()
                .is_some_and(|gesture| gesture.id() == drag.gesture)
        {
            self.scene_interaction.gesture.take();
        }
    }

    fn no_drag() -> CommandError {
        CommandError::new(
            "no_drag",
            "No gizmo drag is in progress; press on a handle first",
        )
    }

    /// The update of one pointer position. Returns the answer fields of the position.
    fn drag_update(&mut self, drag: &Drag, point: Point) -> Result<Value, CommandError> {
        let Some(world_delta) = drag.anchor.delta(point) else {
            // This pointer position does not resolve to a point on the axis or plane; the
            // candidate stays where the last good position put it.
            return Ok(json!({ "applied": false, "reason": "degenerate_pointer" }));
        };
        let local = local_delta(&drag.node, world_delta).ok_or_else(|| {
            CommandError::new(
                "not_translatable",
                "The node's transform cannot be inverted",
            )
            .target(drag.entity.clone())
        })?;
        let position = Vec3 {
            x: drag.base_position.x + local.x,
            y: drag.base_position.y + local.y,
            z: drag.base_position.z + local.z,
        };
        if !finite(position) {
            return Err(CommandError::invalid_value(
                "The dragged position is not a finite number",
                json!("finite numbers"),
            )
            .target(drag.entity.clone()));
        }
        let property = PropertyRef::parse(&format!("{}/position", drag.entity))?;
        let update = self.gesture_update(
            &drag.gesture,
            vec![EditOp::set(property, PropertyValue::Vec3(position))],
        )?;
        if let Some(open) = self.scene_interaction.drag.as_mut() {
            open.last = Some((world_delta, position));
        }
        Ok(json!({
            "applied": true, "world_delta": world_delta.to_json(), "position": position.to_json(),
            "would_change": update["would_change"], "changes": update["changes"],
        }))
    }

    fn pointer_move(&mut self, point: Point) -> Result<Value, CommandError> {
        let drag = self.live_drag().ok_or_else(Self::no_drag)?;
        let point = point.checked(false)?;
        let gesture = self.scene_interaction.gesture.find(&drag.gesture)?;
        if let Err(error) = self.ensure_live(gesture) {
            // The scene moved under the drag (an undo, an edit elsewhere): it can never apply.
            // The drag stays open (and its overlay draws as no drag) until the caller ends it
            // with a release or a cancel, both of which settle the gesture under it; a drag that
            // is dropped here would leave its gesture behind, blocking the next press.
            return Err(error);
        }
        let mut answer = self.drag_update(&drag, point)?;
        answer["action"] = json!("scene.pointer");
        answer["phase"] = json!("move");
        answer["gesture"] = json!(drag.gesture);
        answer["entity"] = json!(drag.entity);
        answer["handle"] = json!(drag.handle().as_str());
        answer["point"] = json!(point.to_json());
        answer["changed"] = json!(true);
        answer["document_changed"] = json!(false);
        answer["overlay"] = self.gizmo_overlay(&drag.scene, drag.frame)?.to_json();
        Ok(answer)
    }

    fn pointer_up(&mut self, point: Option<Point>) -> Result<Value, CommandError> {
        let Some(drag) = self.live_drag() else {
            return Ok(json!({
                "action": "scene.pointer", "phase": "up", "dragging": false, "noop": true,
                "gesture": null, "changed": false, "document_changed": false,
            }));
        };
        // The last position is applied first, so a release is exactly "move there, then commit".
        // One the core refuses ends the drag as a cancel: nothing is half applied.
        if let Some(point) = point {
            let moved = point
                .checked(false)
                .and_then(|point| self.drag_update(&drag, point));
            if let Err(error) = moved {
                self.discard_drag();
                return Err(error);
            }
        }
        let position = self
            .scene_interaction
            .drag
            .as_ref()
            .and_then(|open| open.last);
        let committed = self.gesture_commit(&drag.gesture);
        let mut answer = match committed {
            Ok(answer) => answer,
            Err(error) => {
                // A rejected commit gives the gesture back; a release has nothing to retry
                // with, so the candidate is dropped (the drag is still open here, which is
                // what lets `discard_drag` take the gesture with it) and the typed error is
                // the answer.
                self.discard_drag();
                return Err(error);
            }
        };
        self.scene_interaction.drag = None;
        answer["action"] = json!("scene.pointer");
        answer["phase"] = json!("up");
        answer["dragging"] = json!(false);
        answer["entity"] = json!(drag.entity);
        answer["handle"] = json!(drag.handle().as_str());
        if let Some((world_delta, position)) = position {
            answer["world_delta"] = world_delta.to_json();
            answer["position"] = position.to_json();
        }
        answer["overlay"] = self.gizmo_overlay(&drag.scene, drag.frame)?.to_json();
        Ok(answer)
    }

    fn pointer_cancel(&mut self, reason: CancelReason) -> Result<Value, CommandError> {
        let Some(drag) = self.live_drag() else {
            // Escape with nothing to cancel is not an error: the key may be pressed any time.
            self.scene_interaction.drag = None;
            return Ok(json!({
                "action": "scene.pointer", "phase": "cancel", "reason": reason.as_str(),
                "dragging": false, "noop": true, "gesture": null,
                "changed": false, "document_changed": false,
            }));
        };
        self.discard_drag();
        let mut answer = json!({
            "action": "scene.pointer", "phase": "cancel", "reason": reason.as_str(),
            "dragging": false, "gesture": drag.gesture, "entity": drag.entity,
            "changed": true, "document_changed": false,
        });
        answer["overlay"] = self.gizmo_overlay(&drag.scene, drag.frame)?.to_json();
        Ok(answer)
    }

    pub(super) fn scene_translate(
        &mut self,
        scene: &str,
        entity: &str,
        at: &FrameAt,
        delta: Vec3,
    ) -> Result<Value, CommandError> {
        let parsed = parse_entity(entity)?;
        let entry = self.attached_scene(scene)?;
        let document = std::sync::Arc::clone(&entry.scene);
        ensure_exists(document.bundle(), &parsed)?;
        let not_translatable = |why: &str| {
            CommandError::new(
                "not_translatable",
                format!("`{entity}` cannot be translated: {why}"),
            )
            .target(entity)
            .details(json!({ "reason": why }))
        };
        if parsed.kind != EntityKind::Node {
            return Err(not_translatable("only a node has a position"));
        }
        if !finite(delta) {
            return Err(CommandError::invalid_value(
                "`delta` must be finite numbers",
                json!({ "x": "number", "y": "number", "z": "number" }),
            )
            .target("delta"));
        }
        let frame = frame_of(&document, at)?;
        let plan = plan_of(&document, frame)?;
        let node = plan
            .node(&parsed.id)
            .filter(|_| parsed.scope.as_deref() == Some(plan.source.scene_definition_id.as_str()))
            .ok_or_else(|| {
                not_translatable(
                    "the node is in another scene definition than the one shown at this frame",
                )
            })?;
        movable(node).map_err(not_translatable)?;
        let local = local_delta(node, delta)
            .ok_or_else(|| not_translatable("its transform cannot be inverted"))?;
        let property = position_ref(&plan, &node.id);
        let base = read_position(&document, &property)?;
        let position = Vec3 {
            x: base.x + local.x,
            y: base.y + local.y,
            z: base.z + local.z,
        };
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
}
