//! Running the F055 gizmo commands (`scene.gizmo.state|hit`, `scene.pointer.*`, `scene.translate`)
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
//!
//! BL015.1 adds the **rotate tool** on the same rails: a ring drag is the same gesture (`begin`
//! on press, one `set rotation` per move, `commit` on release), and `scene.rotate` is its
//! pointer-free equivalent. The conventions and the arithmetic are
//! `scene_interaction::gizmo::rotation`; here is only the glue.
//!
//! BL015.1 (G1.2) adds the **scale tool**, again on the same rails: a handle drag is the same gesture
//! (`begin`, one `set scale` per move, `commit`), a refused position keeps the last legal candidate,
//! and `scene.scale` is the pointer-free equivalent. The conventions, the limits and the arithmetic
//! are `scene_interaction::gizmo::scaling`; here is only the glue.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::directed_scene::{
    ChannelProperty, DirectedScene, Entity, EntityKind, NodeAnimation, NodeKind, Quat, Vec3,
};
use crate::scene_edit::{EditOp, NodeProperty, PropertyRef, PropertyValue, read_property};
use crate::scene_interaction::{
    Anchor, Applied, CancelReason, Drag, DragAction, FrameAt, GizmoCommand, GizmoSpace, GizmoTool,
    Handle, Hidden, Layout, Overlay, Point, PointerCommand, RingAnchor, RotationFrame, ScaleAnchor,
    ScaleFrame, SelectMode, View, ensure_exists, entity_path, local_delta, movable, outline,
    parse_entity, pick_plane, quat_json, refusal, scale_refusal, world_origin,
};
use crate::scene_plan::{PlanNode, PlanOptions, ScenePlan};
use crate::state::{CommandError, EditorState};

mod light;
use light::{light_at_point, light_layout, light_markers};

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

/// What leaving the effect target said (F061), carried from the `scene.select` a press ran.
fn carry_fx_target(answer: &mut Value, outcome: &Value) {
    for key in ["fx_target_dropped", "cancelled_drag"] {
        if let Some(value) = outcome.get(key) {
            answer[key] = value.clone();
        }
    }
}

pub(super) fn frame_of(scene: &DirectedScene, at: &FrameAt) -> Result<i64, CommandError> {
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

fn rotation_ref(plan: &ScenePlan, node: &str) -> PropertyRef {
    PropertyRef::node(
        &plan.source.scene_definition_id,
        node,
        NodeProperty::Rotation,
    )
}

fn read_rotation(scene: &DirectedScene, property: &PropertyRef) -> Result<Quat, CommandError> {
    match read_property(scene, property)? {
        PropertyValue::Quaternion(rotation) => Ok(rotation),
        _ => Err(CommandError::new(
            "internal",
            "`rotation` is not a quaternion in the catalog",
        )),
    }
}

/// Whether a performance channel writes `node`'s rotation at `frame`: then the base rotation the
/// document holds is not what is shown (the channel adds its Euler offset on top of it) and the
/// rotate tool refuses the node instead of editing something the animation would silently
/// offset. The plan keeps only "animated or not", so the channels are asked again, and only when
/// the frame is animated at all. When the frame cannot be evaluated the node is refused too.
fn animates_rotation(scene: &DirectedScene, plan: &ScenePlan, frame: i64, node: &str) -> bool {
    animates(scene, plan, frame, node, ChannelProperty::LocalRotation)
}

/// Whether a performance channel writes `property` of `node` at `frame` (see
/// [`animates_rotation`]; the scale tool asks the same of `LocalScale`, which a channel replaces).
fn animates(
    scene: &DirectedScene,
    plan: &ScenePlan,
    frame: i64,
    node: &str,
    property: ChannelProperty,
) -> bool {
    if matches!(plan.animation, NodeAnimation::Static) {
        return false;
    }
    match scene.bundle().evaluate_frame(frame) {
        Ok(evaluated) => evaluated.nodes.override_of(node, property).is_some(),
        Err(_) => true,
    }
}

/// The frame of a rotation of `node` in the document the plan was built from: its axes, the
/// rotation above it and the authored quaternion, or the reason it cannot be turned.
fn rotation_frame(
    document: &DirectedScene,
    plan: &ScenePlan,
    frame: i64,
    node: &PlanNode,
    space: GizmoSpace,
) -> Result<RotationFrame, &'static str> {
    let authored = read_rotation(document, &rotation_ref(plan, &node.id))
        .map_err(|_| refusal::DEGENERATE_ROTATION)?;
    let animated = animates_rotation(document, plan, frame, &node.id);
    RotationFrame::of(node, authored, space, animated)
}

fn scale_ref(plan: &ScenePlan, node: &str) -> PropertyRef {
    PropertyRef::node(&plan.source.scene_definition_id, node, NodeProperty::Scale)
}

fn read_scale(scene: &DirectedScene, property: &PropertyRef) -> Result<Vec3, CommandError> {
    match read_property(scene, property)? {
        PropertyValue::Vec3(scale) => Ok(scale),
        _ => Err(CommandError::new(
            "internal",
            "`scale` is not a vector in the catalog",
        )),
    }
}

/// Whether `node` is a splat or has one below it in the hierarchy: Spark draws one uniform scale
/// per splat (BL123), so such a node only scales uniformly. Walks each splat's parent chain, bounded
/// by the number of nodes (the plan has no cycle, the bound only makes the walk total).
fn splat_in_subtree(plan: &ScenePlan, node: &PlanNode) -> bool {
    plan.nodes
        .iter()
        .filter(|candidate| candidate.kind == NodeKind::Splat)
        .any(|splat| {
            let mut current = splat;
            for _ in 0..=plan.nodes.len() {
                if current.id == node.id {
                    return true;
                }
                match current
                    .parent
                    .as_ref()
                    .and_then(|parent| plan.node(&parent.node_id))
                {
                    Some(parent) => current = parent,
                    None => return false,
                }
            }
            false
        })
}

/// The frame of a scale of `node` in the document the plan was built from: what each handle does
/// and the authored `localScale`, or the reason the node cannot be scaled.
fn scale_frame(
    document: &DirectedScene,
    plan: &ScenePlan,
    frame: i64,
    node: &PlanNode,
    space: GizmoSpace,
) -> Result<ScaleFrame, &'static str> {
    let base = read_scale(document, &scale_ref(plan, &node.id))
        .map_err(|_| scale_refusal::DEGENERATE_SCALE)?;
    let animated = animates(document, plan, frame, &node.id, ChannelProperty::LocalScale);
    ScaleFrame::of(node, base, space, animated, splat_in_subtree(plan, node))
}

fn not_scalable(entity: &str, reason: &str) -> CommandError {
    CommandError::new(
        "not_scalable",
        format!("`{entity}` cannot be scaled: {reason}"),
    )
    .target(entity)
    .details(json!({ "reason": reason }))
}

fn not_rotatable(entity: &str, reason: &str) -> CommandError {
    CommandError::new(
        "not_rotatable",
        format!("`{entity}` cannot be rotated: {reason}"),
    )
    .target(entity)
    .details(json!({ "reason": reason }))
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
    frame: i64,
}

impl Scene3d {
    fn new(scene: &DirectedScene, frame: i64) -> Result<Self, CommandError> {
        let plan = plan_of(scene, frame)?;
        let view = View::from_plan(&plan);
        Ok(Self { plan, view, frame })
    }

    /// The handles of `node` for the active tool, or `None` when the tool has none for it (the
    /// rotate and scale tools refuse the node: see [`rotation_frame`], [`scale_frame`]). `document` is the scene the plan
    /// was built from.
    fn layout(
        &self,
        document: &DirectedScene,
        view: &View,
        node: &PlanNode,
        tool: GizmoTool,
        space: GizmoSpace,
    ) -> Option<Layout> {
        match tool {
            GizmoTool::Translate => Layout::at(view, world_origin(node)),
            GizmoTool::Rotate => {
                let frame = rotation_frame(document, &self.plan, self.frame, node, space).ok()?;
                Layout::rings_at(view, world_origin(node), &frame)
            }
            GizmoTool::Scale => {
                let frame = scale_frame(document, &self.plan, self.frame, node, space).ok()?;
                Layout::scale_handles(view, world_origin(node), &frame)
            }
        }
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
            GizmoCommand::Rotate {
                scene,
                entity,
                at,
                axis,
                degrees,
                space,
            } => self.scene_rotate(&scene, &entity, &at, axis, degrees, space),
            GizmoCommand::Scale {
                scene,
                entity,
                at,
                axis,
                factor,
                space,
            } => self.scene_scale(&scene, &entity, &at, axis, factor, space),
            GizmoCommand::Range {
                scene,
                entity,
                at,
                delta,
            } => self.scene_range(&scene, &entity, &at, delta),
            GizmoCommand::Tool { tool, space } => self.gizmo_tool(tool, space),
        }
    }

    /// `scene.gizmo.tool`: reads, and with a field sets, the tool and the space of the rotate and
    /// scale axes. Session state, never an edit. Refused while a drag is open: the handle under the
    /// pointer would change meaning, and the drag's candidate belongs to the tool that began it.
    fn gizmo_tool(
        &mut self,
        tool: Option<GizmoTool>,
        space: Option<GizmoSpace>,
    ) -> Result<Value, CommandError> {
        let (was_tool, was_space) = (self.scene_interaction.tool, self.scene_interaction.space);
        let (tool, space) = (tool.unwrap_or(was_tool), space.unwrap_or(was_space));
        let changes = (tool, space) != (was_tool, was_space);
        if changes && let Some(drag) = self.live_drag() {
            return Err(CommandError::new(
                "gesture_active",
                "A gizmo drag is in progress; finish or cancel it before changing the tool",
            )
            .target(drag.gesture.clone())
            .details(json!({ "gesture": drag.gesture, "tool": drag.tool_id() })));
        }
        self.scene_interaction.tool = tool;
        self.scene_interaction.space = space;
        Ok(json!({
            "action": "scene.gizmo.tool", "tool": tool.as_str(), "space": space.as_str(),
            "previous": { "tool": was_tool.as_str(), "space": was_space.as_str() },
            "tools": GizmoTool::ALL.map(GizmoTool::as_str),
            "spaces": GizmoSpace::ALL.map(GizmoSpace::as_str),
            "changed": changes, "document_changed": false,
        }))
    }

    /// The identity of what an overlay shows: scene, interaction session (a new project is a new
    /// session), project revision and edit frame. An overlay drawn for one stamp must not
    /// answer to a pointer message that was made against another: a message from the page that
    /// was reloaded, from the project that was replaced, or from the frame before a seek is
    /// refused by the adapter by comparing this value and nothing else.
    pub fn gizmo_stamp(&self, scene: &str, frame: i64) -> Value {
        let mut stamp = json!({
            "scene": scene,
            "session": self.scene_interaction.gesture.session(),
            "revision": self.project.revision,
            "frame": frame,
            // BL015.1: a press made against the rings must not act on arrows (or the other way
            // round), nor on axes of the other space, so the tool is part of the identity.
            "tool": self.scene_interaction.tool.as_str(),
            "space": self.scene_interaction.space.as_str(),
        });
        // F061, additive: the effect clip the handles belong to. A message made against the
        // handles of one clip is not taken for another's (nor for the node gizmo's).
        if let Some(target) = self
            .scene_interaction
            .fx_target
            .as_ref()
            .filter(|target| target.scene == scene)
        {
            stamp["fx_clip"] = json!(target.clip);
        }
        stamp
    }

    /// The addresses of the selected entities of `scene`, in selection order: what the handles
    /// and outlines of an overlay belong to. An adapter puts it in its stamp, so a message made
    /// against the overlay of the previous selection is not taken for the current one.
    pub fn gizmo_selection(&self, scene: &str) -> Vec<String> {
        self.scene_interaction
            .selection
            .entities_of(scene)
            .map(entity_path)
            .collect()
    }

    /// What [`gizmo_overlay`](Self::gizmo_overlay) is a function of, as one comparable string: the
    /// stamp (scene, session, project revision, frame), the selected entities of the scene and,
    /// while a drag is open, its gesture, the updates that gesture accepted (the candidate the
    /// handles are laid out on) and whether it is dead. Two equal keys draw the same overlay, so
    /// an adapter that already sent the overlay for a key does not need the core to plan the scene
    /// again for it (M2 of the PR #25 review). Reads only; cheap (no plan).
    pub fn gizmo_overlay_key(&self, scene: &str, frame: i64) -> String {
        let selected = self.gizmo_selection(scene);
        let drag = self.scene_interaction.drag.as_ref().map(|drag| {
            let updates = self
                .scene_interaction
                .gesture
                .current()
                .filter(|gesture| gesture.id() == drag.gesture.as_str())
                .map(|gesture| gesture.updates());
            json!([drag.gesture, drag.scene, updates, self.gizmo_drag_is_dead()])
        });
        json!({
            "stamp": self.gizmo_stamp(scene, frame),
            "selection": selected,
            "drag": drag,
        })
        .to_string()
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
    pub(super) fn live_drag(&mut self) -> Option<Drag> {
        self.scene_interaction.drag.as_ref()?;
        if let Some(drag) = self.scene_interaction.live_drag() {
            return Some(drag.clone());
        }
        self.scene_interaction.drag = None;
        None
    }

    /// What a runtime draws for `scene` at `frame`: the outline of each selected image plane and
    /// the handles of the one selected node (or of the node being dragged, whose candidate is
    /// what the plan is built from).
    pub(super) fn gizmo_overlay(
        &mut self,
        scene_id: &str,
        frame: i64,
    ) -> Result<Overlay, CommandError> {
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
        // F061: the session targets an effect clip of this scene: its handles are the overlay and
        // the node gizmo gives way (`Hidden::FxTarget`). One target at a time.
        let stamp = self.gizmo_stamp(scene_id, frame);
        if let Some(overlay) = self.fx_overlay(scene_id, frame, &shown, dragging.as_ref(), stamp) {
            return Ok(overlay);
        }
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

        let (tool, space) = (self.scene_interaction.tool, self.scene_interaction.space);
        let markers = light_markers(&world, view, &selected);
        let mut overlay = Overlay {
            tool,
            space,
            entity: None,
            layout: None,
            hidden: None,
            outlines,
            markers,
            active: dragging
                .as_ref()
                .and_then(|drag| Some((drag.handle()?, drag.gesture.clone()))),
            fx: None,
            aspect: view.map_or(1.0, View::aspect),
            stamp: self.gizmo_stamp(scene_id, frame),
        };
        // A point light (F060) has its own handles: the one dragged, else the single selected one.
        let light = match (&dragging, selected.as_slice()) {
            (Some(drag), _) => parse_entity(&drag.entity)
                .ok()
                .filter(|entity| entity.kind == EntityKind::Light),
            (None, [entity]) if entity.kind == EntityKind::Light => Some((*entity).clone()),
            _ => None,
        };
        if let Some(entity) = light {
            if entity.scope.as_deref() != Some(definition) {
                overlay.hidden = Some(Hidden::NotInFrame);
                return Ok(overlay);
            }
            return Ok(self.light_overlay(overlay, &world, &shown, &entity));
        }
        // The node the handles belong to: the one being dragged, else the single selected node.
        let node_id = match (&dragging, selected.as_slice()) {
            (Some(drag), _) => drag.node.as_ref().map(|node| node.id.clone()),
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
            Some(view) => {
                if tool == GizmoTool::Rotate
                    && let Err(reason) =
                        rotation_frame(&shown, &world.plan, world.frame, node, space)
                {
                    // The node cannot be turned (an animated rotation, a parent that scales
                    // unevenly...): no rings, and the reason is in the overlay, never a silent
                    // gizmo that would then refuse the press.
                    overlay.hidden = Some(Hidden::NotRotatable(reason));
                    return Ok(overlay);
                }
                if tool == GizmoTool::Scale
                    && let Err(reason) = scale_frame(&shown, &world.plan, world.frame, node, space)
                {
                    // Scale is refused the same way: no handles, and the reason in the overlay.
                    overlay.hidden = Some(Hidden::NotScalable(reason));
                    return Ok(overlay);
                }
                match world.layout(&shown, view, node, tool, space) {
                    Some(layout) => overlay.layout = Some(layout),
                    None => overlay.hidden = Some(Hidden::BehindCamera),
                }
            }
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
        let document = Arc::clone(&self.attached_scene(scene_id)?.scene);
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
            && let Some(layout) = world.layout(
                &document,
                view,
                node,
                self.scene_interaction.tool,
                self.scene_interaction.space,
            )
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
        if let [one] = selected.as_slice()
            && one.kind == EntityKind::Light
            && one.scope.as_deref() == Some(definition)
            && self.scene_interaction.tool == GizmoTool::Translate
            && let Ok(layout) = light_layout(world, view, &document, one)
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
        // A light's marker is above the image planes it is drawn over.
        if let Some(entity) = light_at_point(world, view, point) {
            return Ok((Hit::Entity { entity }, handles));
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
        // F061: a press on a handle of the vignette the session targets; anything else goes on
        // to what a press always did (and selecting something leaves the effect target).
        if let Some(answer) = self.fx_pointer_down(scene, frame, point)? {
            return Ok(answer);
        }
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
                carry_fx_target(&mut answer, &outcome);
            }
            Hit::Nothing if !additive => {
                let outcome = self.scene_select(Some(scene), &[], SelectMode::Clear)?;
                answer["selection_changed"] = outcome["changed"].clone();
                answer["changed"] = outcome["changed"].clone();
                carry_fx_target(&mut answer, &outcome);
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
        if parsed.kind == EntityKind::Light {
            if self.scene_interaction.tool != GizmoTool::Translate {
                return Err(CommandError::invalid_value(
                    "A light has handles with the translate tool only",
                    json!({ "options": [GizmoTool::Translate.as_str()] }),
                )
                .target("tool"));
            }
            return self.begin_light_drag(scene, world, handle, &parsed, point, answer);
        }
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
        let document = Arc::clone(&self.attached_scene(scene)?.scene);
        let tool = self.scene_interaction.tool;
        let action = match tool {
            GizmoTool::Translate => {
                let Some(anchor) = Anchor::begin(view, origin, handle, point) else {
                    return Self::refused_press(answer);
                };
                let base_position = read_position(&document, &position_ref(&world.plan, &node.id))?;
                DragAction::Translate {
                    anchor,
                    base_position,
                }
            }
            GizmoTool::Rotate => {
                let space = self.scene_interaction.space;
                // The same refusals the overlay shows as `hidden`; checked here again because a
                // press may come from a script that never read the overlay.
                let frame = rotation_frame(&document, &world.plan, world.frame, node, space)
                    .map_err(|reason| not_rotatable(entity, reason))?;
                let axis = frame.axis(handle).ok_or_else(|| {
                    CommandError::invalid_value(
                        "The rotate tool has rings for `x`, `y` and `z` only",
                        json!({ "options": ["x", "y", "z"] }),
                    )
                    .target("handle")
                })?;
                let Some(ring) = RingAnchor::begin(view, origin, axis, handle, point) else {
                    return Self::refused_press(answer);
                };
                DragAction::Rotate {
                    ring,
                    frame: Box::new(frame),
                }
            }
            GizmoTool::Scale => {
                let space = self.scene_interaction.space;
                // The same refusals the overlay shows as `hidden` or `disabled_handles`; checked
                // here again because a press may come from a script that never read the overlay.
                let frame = scale_frame(&document, &world.plan, world.frame, node, space)
                    .map_err(|reason| not_scalable(entity, reason))?;
                let direction = match handle {
                    Handle::Uniform => None,
                    Handle::X | Handle::Y | Handle::Z => {
                        Some(frame.direction(handle).ok_or_else(|| {
                            not_scalable(
                                entity,
                                frame.disabled(handle).unwrap_or("handle_unavailable"),
                            )
                        })?)
                    }
                    Handle::Screen | Handle::Range => {
                        return Err(CommandError::invalid_value(
                            "The scale tool has handles `x`, `y`, `z` and `uniform` only",
                            json!({ "options": ["x", "y", "z", "uniform"] }),
                        )
                        .target("handle"));
                    }
                };
                let Some(anchor) = ScaleAnchor::begin(view, origin, handle, direction, point)
                else {
                    return Self::refused_press(answer);
                };
                DragAction::Scale {
                    anchor,
                    frame: Box::new(frame),
                }
            }
        };
        let frame = i64::try_from(world.plan.frame.edit_frame).unwrap_or(i64::MAX);
        let label = match tool {
            GizmoTool::Translate => "Move",
            GizmoTool::Rotate => "Rotate",
            GizmoTool::Scale => "Scale",
        };
        let begun = self.gesture_begin(scene, &[entity.to_string()], Some(label.to_string()))?;
        let gesture = begun["gesture"].as_str().unwrap_or_default().to_string();
        self.scene_interaction.drag = Some(Drag {
            gesture: gesture.clone(),
            scene: scene.to_string(),
            entity: entity.to_string(),
            frame,
            node: Some(node.clone()),
            action,
            last: None,
        });
        answer["gesture"] = json!(gesture);
        answer["dragging"] = json!(true);
        answer["handle"] = json!(handle.as_str());
        answer["tool"] = json!(tool.as_str());
        answer["changed"] = json!(true);
        Ok(())
    }

    /// A press that does not determine a position or an angle (the pointer looks along the axis,
    /// or sees a ring edge-on): refused before a gesture opens, so nothing is left to cancel.
    fn refused_press(answer: &mut Value) -> Result<(), CommandError> {
        answer["dragging"] = json!(false);
        answer["refused"] = json!("degenerate_axis");
        Ok(())
    }

    /// Drops the drag and the gesture under it (never touches the document).
    pub(super) fn discard_drag(&mut self) {
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
        match &drag.action {
            DragAction::Fx { .. } => self.fx_handle_update(drag, point),
            DragAction::Translate {
                anchor,
                base_position,
            } => self.translate_update(drag, anchor, *base_position, point),
            DragAction::Rotate { ring, frame } => self.rotate_update(drag, ring, frame, point),
            DragAction::Scale { anchor, frame } => self.scale_update(drag, anchor, frame, point),
            DragAction::LightTranslate {
                anchor,
                base_position,
            } => self.light_translate_update(drag, anchor, *base_position, point),
            DragAction::LightRange {
                anchor,
                base_radius,
            } => self.light_range_update(drag, anchor, *base_radius, point),
        }
    }

    fn translate_update(
        &mut self,
        drag: &Drag,
        anchor: &Anchor,
        base_position: Vec3,
        point: Point,
    ) -> Result<Value, CommandError> {
        let Some(world_delta) = anchor.delta(point) else {
            // This pointer position does not resolve to a point on the axis or plane; the
            // candidate stays where the last good position put it.
            return Ok(json!({ "applied": false, "reason": "degenerate_pointer" }));
        };
        let Some(node) = drag.node.as_ref() else {
            return Ok(json!({ "applied": false, "reason": "no_node" }));
        };
        let local = local_delta(node, world_delta).ok_or_else(|| {
            CommandError::new(
                "not_translatable",
                "The node's transform cannot be inverted",
            )
            .target(drag.entity.clone())
        })?;
        let position = Vec3 {
            x: base_position.x + local.x,
            y: base_position.y + local.y,
            z: base_position.z + local.z,
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
            open.last = Some(Applied::Translate {
                world_delta,
                position,
            });
        }
        Ok(json!({
            "applied": true, "world_delta": world_delta.to_json(), "position": position.to_json(),
            "would_change": update["would_change"], "changes": update["changes"],
        }))
    }

    /// One ring position: the angle since the press becomes the candidate rotation, computed by
    /// the same [`RotationFrame::rotate`] `scene.rotate` uses, so the two roads give the same
    /// quaternion bit for bit.
    fn rotate_update(
        &mut self,
        drag: &Drag,
        ring: &RingAnchor,
        frame: &RotationFrame,
        point: Point,
    ) -> Result<Value, CommandError> {
        let Some(degrees) = ring.degrees(point) else {
            return Ok(json!({ "applied": false, "reason": "degenerate_pointer" }));
        };
        let rotation = frame.rotate(ring.handle, degrees).ok_or_else(|| {
            CommandError::invalid_value(
                "The dragged rotation is not a finite quaternion",
                json!("finite numbers"),
            )
            .target(drag.entity.clone())
        })?;
        let property = PropertyRef::parse(&format!("{}/rotation", drag.entity))?;
        let update = self.gesture_update(
            &drag.gesture,
            vec![EditOp::set(property, PropertyValue::Quaternion(rotation))],
        )?;
        if let Some(open) = self.scene_interaction.drag.as_mut() {
            open.last = Some(Applied::Rotate { degrees, rotation });
        }
        Ok(json!({
            "applied": true, "degrees": degrees, "axis": ring.handle.as_str(),
            "space": frame.space().as_str(), "rotation": quat_json(rotation),
            "would_change": update["would_change"], "changes": update["changes"],
        }))
    }

    /// One handle position: the ratio of the pointer to the press becomes the candidate scale,
    /// computed by the same [`ScaleFrame::scale`] `scene.scale` uses, so the two roads give the
    /// same vector bit for bit. A position the rules refuse (the pointer crossed the origin, a
    /// limit, a degenerate result) is `applied: false` with the typed reason and the last legal
    /// candidate stays, so a release there still commits what was last legal.
    fn scale_update(
        &mut self,
        drag: &Drag,
        anchor: &ScaleAnchor,
        frame: &ScaleFrame,
        point: Point,
    ) -> Result<Value, CommandError> {
        let Some(factor) = anchor.factor(point) else {
            return Ok(json!({ "applied": false, "reason": "degenerate_pointer" }));
        };
        let scale = match frame.scale(anchor.handle, factor) {
            Ok(scale) => scale,
            Err(reason) => {
                return Ok(json!({ "applied": false, "reason": reason, "factor": factor }));
            }
        };
        let property = PropertyRef::parse(&format!("{}/scale", drag.entity))?;
        let update = self.gesture_update(
            &drag.gesture,
            vec![EditOp::set(property, PropertyValue::Vec3(scale))],
        )?;
        if let Some(open) = self.scene_interaction.drag.as_mut() {
            open.last = Some(Applied::Scale { factor, scale });
        }
        Ok(json!({
            "applied": true, "factor": factor, "axis": anchor.handle.as_str(),
            "space": frame.space().as_str(), "scale": scale.to_json(),
            "would_change": update["would_change"], "changes": update["changes"],
        }))
    }

    fn pointer_move(&mut self, point: Point) -> Result<Value, CommandError> {
        let drag = self.live_drag().ok_or_else(Self::no_drag)?;
        let point = point.checked(false)?;
        let gesture = self.scene_interaction.gesture.find(&drag.gesture)?;
        // The scene moved under the drag (an undo, an edit elsewhere): it can never apply.
        // The drag stays open (and its overlay draws as no drag) until the caller ends it
        // with a release or a cancel, both of which settle the gesture under it; a drag that
        // is dropped here would leave its gesture behind, blocking the next press.
        self.ensure_live(gesture)?;
        let mut answer = self.drag_update(&drag, point)?;
        answer["action"] = json!("scene.pointer");
        answer["phase"] = json!("move");
        answer["gesture"] = json!(drag.gesture);
        answer["entity"] = json!(drag.entity);
        answer["handle"] = json!(drag.handle_id());
        answer["tool"] = json!(drag.tool_id());
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
        let applied = self
            .scene_interaction
            .drag
            .as_ref()
            .and_then(|open| open.last);
        let committed = if drag.is_fx() {
            self.fx_gesture_commit(&drag.gesture)
        } else {
            self.gesture_commit(&drag.gesture)
        };
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
        answer["handle"] = json!(drag.handle_id());
        answer["tool"] = json!(drag.tool_id());
        match applied {
            Some(Applied::Translate {
                world_delta,
                position,
            }) => {
                answer["world_delta"] = world_delta.to_json();
                answer["position"] = position.to_json();
            }
            Some(Applied::Rotate { degrees, rotation }) => {
                answer["degrees"] = json!(degrees);
                answer["axis"] = json!(drag.handle_id());
                answer["rotation"] = quat_json(rotation);
                if let DragAction::Rotate { frame, .. } = &drag.action {
                    answer["space"] = json!(frame.space().as_str());
                }
            }
            Some(Applied::Scale { factor, scale }) => {
                answer["factor"] = json!(factor);
                answer["axis"] = json!(drag.handle_id());
                answer["scale"] = scale.to_json();
                if let DragAction::Scale { frame, .. } = &drag.action {
                    answer["space"] = json!(frame.space().as_str());
                }
            }
            Some(Applied::LightTranslate {
                world_delta,
                position,
            }) => {
                answer["world_delta"] = world_delta.to_json();
                answer["position"] = position.to_json();
            }
            Some(Applied::LightRange { delta, radius }) => {
                answer["delta"] = json!(delta);
                answer["radius"] = json!(radius);
            }
            Some(Applied::Fx { handle, value }) => {
                answer["target"] = json!("fx");
                answer["clip"] = json!(drag.entity);
                answer["param"] = json!(handle.param());
                answer["value"] = value.to_json();
            }
            None => {}
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

    /// `scene.rotate`: the ring drag without the pointer. One node, one axis, an angle in degrees,
    /// written by `edit_scene` (the path `scene.edit` and a gesture commit share), so the same
    /// angle gives the same document by either road and a turn of 0° or of whole turns writes
    /// nothing (no history entry).
    pub(super) fn scene_rotate(
        &mut self,
        scene: &str,
        entity: &str,
        at: &FrameAt,
        axis: Handle,
        degrees: f64,
        space: Option<GizmoSpace>,
    ) -> Result<Value, CommandError> {
        let parsed = parse_entity(entity)?;
        let entry = self.attached_scene(scene)?;
        let document = Arc::clone(&entry.scene);
        ensure_exists(document.bundle(), &parsed)?;
        if parsed.kind != EntityKind::Node {
            return Err(not_rotatable(entity, "only a node has a rotation"));
        }
        if axis == Handle::Screen {
            return Err(CommandError::invalid_value(
                "`axis` must be `x`, `y` or `z`: the screen handle is not a ring",
                json!({ "options": ["x", "y", "z"] }),
            )
            .target("axis"));
        }
        if !degrees.is_finite() {
            return Err(CommandError::invalid_value(
                "`degrees` must be a finite number",
                json!("number"),
            )
            .target("degrees"));
        }
        let space = space.unwrap_or(self.scene_interaction.space);
        let frame = frame_of(&document, at)?;
        let plan = plan_of(&document, frame)?;
        let node = plan
            .node(&parsed.id)
            .filter(|_| parsed.scope.as_deref() == Some(plan.source.scene_definition_id.as_str()))
            .ok_or_else(|| {
                not_rotatable(
                    entity,
                    "the node is in another scene definition than the one shown at this frame",
                )
            })?;
        let rotating = rotation_frame(&document, &plan, frame, node, space)
            .map_err(|reason| not_rotatable(entity, reason))?;
        let rotation = rotating
            .rotate(axis, degrees)
            .ok_or_else(|| not_rotatable(entity, "the result is not a finite rotation"))?;
        let property = rotation_ref(&plan, &node.id);
        let mut answer = self.edit_scene(
            scene,
            &[EditOp::set(property, PropertyValue::Quaternion(rotation))],
            false,
        )?;
        answer["action"] = json!("scene.rotate");
        answer["entity"] = json!(entity);
        answer["frame"] = json!(frame);
        answer["axis"] = json!(axis.as_str());
        answer["space"] = json!(space.as_str());
        answer["degrees"] = json!(degrees);
        answer["rotation"] = quat_json(rotation);
        answer["document_changed"] = answer["changed"].clone();
        Ok(answer)
    }

    /// `scene.scale`: the handle drag without the pointer. One node, one handle, a multiplicative
    /// factor, written by `edit_scene` (the path `scene.edit` and a gesture commit share), so the
    /// same factor gives the same document by either road and a factor of 1 writes nothing (no
    /// history entry).
    pub(super) fn scene_scale(
        &mut self,
        scene: &str,
        entity: &str,
        at: &FrameAt,
        axis: Handle,
        factor: f64,
        space: Option<GizmoSpace>,
    ) -> Result<Value, CommandError> {
        let parsed = parse_entity(entity)?;
        let entry = self.attached_scene(scene)?;
        let document = Arc::clone(&entry.scene);
        ensure_exists(document.bundle(), &parsed)?;
        if parsed.kind != EntityKind::Node {
            return Err(not_scalable(entity, "only a node has a scale"));
        }
        if axis == Handle::Screen {
            return Err(CommandError::invalid_value(
                "`axis` must be `x`, `y`, `z` or `uniform`: the screen handle is not a scale handle",
                json!({ "options": ["x", "y", "z", "uniform"] }),
            )
            .target("axis"));
        }
        if !factor.is_finite() {
            return Err(CommandError::invalid_value(
                "`factor` must be a finite number",
                json!("number"),
            )
            .target("factor"));
        }
        let space = space.unwrap_or(self.scene_interaction.space);
        let frame = frame_of(&document, at)?;
        let plan = plan_of(&document, frame)?;
        let node = plan
            .node(&parsed.id)
            .filter(|_| parsed.scope.as_deref() == Some(plan.source.scene_definition_id.as_str()))
            .ok_or_else(|| {
                not_scalable(
                    entity,
                    "the node is in another scene definition than the one shown at this frame",
                )
            })?;
        let scaling = scale_frame(&document, &plan, frame, node, space)
            .map_err(|reason| not_scalable(entity, reason))?;
        let scale = scaling
            .scale(axis, factor)
            .map_err(|reason| not_scalable(entity, reason))?;
        let property = scale_ref(&plan, &node.id);
        let mut answer = self.edit_scene(
            scene,
            &[EditOp::set(property, PropertyValue::Vec3(scale))],
            false,
        )?;
        answer["action"] = json!("scene.scale");
        answer["entity"] = json!(entity);
        answer["frame"] = json!(frame);
        answer["axis"] = json!(axis.as_str());
        answer["space"] = json!(space.as_str());
        answer["factor"] = json!(factor);
        answer["scale"] = scale.to_json();
        // The component the factor landed on (in `world`, a node turned a quarter turn writes
        // another one than the handle's letter), or `all` for the uniform handle.
        answer["component"] = json!(scaling.component(axis).unwrap_or("all"));
        answer["document_changed"] = answer["changed"].clone();
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
        if parsed.kind == EntityKind::Light {
            return self.scene_translate_light(scene, &parsed, at, delta);
        }
        if parsed.kind != EntityKind::Node {
            return Err(not_translatable("only a node or a light has a position"));
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
