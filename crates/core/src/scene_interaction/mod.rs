//! F039 (BB07, initial): the semantic interaction model of directed scenes: selection by entity
//! address, a queryable entity tree, the evaluated frame, and the begin/update/commit/cancel
//! cycle of a gesture. No pointer, no window, no pixels: this is what a future overlay or panel
//! drives, and what a script drives today.
//!
//! This module holds the *model* (addresses, selection, gesture state, tree and frame answers).
//! Running it against a project, which needs the `SceneBook`, the history and `scene_edit`, is
//! `scene_project/interaction.rs`; the JSON vocabulary (`scene.tree`, `scene.select`,
//! `scene.gesture.*`...) is parsed once in `ops::parse_op`, like the rest of `scene.*`.
//!
//! What it does not add, on purpose:
//!
//! * **No second executor, no second history.** A gesture's candidate is built by
//!   `scene_edit::apply` and committed by running the same batch through `scene.edit`'s own
//!   path, so the catalog (8 properties), the importer, the structural and temporal gates and
//!   the size budget are the ones that already exist, and a commit is ONE entry of the
//!   project's existing undo stack.
//! * **Nothing ephemeral is saved.** Selection and the open gesture live in
//!   [`SceneInteraction`], a session field of `EditorState`. It is not in the document, not in an
//!   undo snapshot and not restored by undo/redo; opening or creating a project starts empty.
//! * **Selection is never a precondition.** `scene.edit` and a gesture's ops name their targets
//!   by ID; the selection is for the user's focus and is pruned when what it names disappears.
//!
//! Contract and limits: `docs/features/scene-interaction.md`.

use serde_json::{Value, json};

use crate::scene_project::SceneBook;

mod address;
mod command;
mod frame;
mod gesture;
mod gizmo;
mod selection;
mod tree;

pub use address::{
    ensure_exists, entity_json, entity_of_property, entity_path, exists, kind_label, parse_entity,
};
pub use command::{
    CancelReason, FrameAt, FxGestureCommand, GestureCommand, GizmoCommand, PointerCommand,
    SelectMode,
};
pub use frame::frame_json;
pub use gesture::{Gesture, GestureKind, GestureSlot, Invalidation, Refusal};
pub use gizmo::{
    Anchor, CENTER_RADIUS, GizmoSpace, GizmoTool, HANDLE_LENGTH, HIT_RADIUS, Handle, Hidden,
    Layout, LightMarker, MARKER_RADIUS, MAX_RADIUS, MAX_SCALE, MIN_SCALE, OUTSIDE_LIMIT, Overlay,
    Point, RING_RADIUS, RING_SEGMENTS, RangeAnchor, RingAnchor, RotationFrame, ScaleAnchor,
    ScaleFrame, UNIFORM_OFFSET, View, light_position, light_range, light_refusal, local_delta,
    marker_at, movable, outline, pick_marker, pick_plane, quat_json, refusal, scale_refusal,
    world_origin,
};
pub use selection::{Dropped, MAX_SELECTION, SUMMARY_LIMIT, Selected, Selection};
pub use tree::{MAX_TREE_DEPTH, tree_json};

/// The session state of the scene tools of one project: what is selected and which gesture is
/// open. Presentation, not document (see the module docs).
#[derive(Debug, Clone, Default)]
pub struct SceneInteraction {
    pub(crate) selection: Selection,
    pub(crate) gesture: GestureSlot,
    /// The pointer drag of the translation gizmo, if one is open. It always rides on `gesture`
    /// (same ID); it only adds what the pointer needs. See `scene_project/gizmo.rs`.
    pub(crate) drag: Option<Drag>,
    /// What the handles of the selected node do (BL015.1). Session state like the selection: not
    /// in the document, not in an undo snapshot, reset by a new project.
    pub(crate) tool: GizmoTool,
    /// The orientation of the rotate and scale tools' axes.
    pub(crate) space: GizmoSpace,
}

/// What a gizmo drag remembers from the press so that every later position is computed from the
/// press alone (never from the candidate it produced).
#[derive(Debug, Clone)]
pub struct Drag {
    /// The gesture this drag feeds. A drag whose gesture is gone or replaced is dead.
    pub(crate) gesture: String,
    pub(crate) scene: String,
    /// Address of the node being moved.
    pub(crate) entity: String,
    /// The edit frame the press was made at: the camera and the hierarchy of the whole drag.
    pub(crate) frame: i64,
    /// The node as evaluated at the press (its local/world matrices convert a world delta).
    /// `None` for the drag of a light (F060): its position is not behind a node's matrices.
    pub(crate) node: Option<crate::scene_plan::PlanNode>,
    /// What the press is bound to, by tool.
    pub(crate) action: DragAction,
    /// What the last accepted update computed.
    pub(crate) last: Option<Applied>,
}

/// The part of a drag that depends on the tool.
#[derive(Debug, Clone)]
pub(crate) enum DragAction {
    Translate {
        anchor: Anchor,
        /// The document's `localPosition` at the press: the base every update adds to.
        base_position: crate::directed_scene::Vec3,
    },
    Rotate {
        ring: RingAnchor,
        /// The node's axes, parent rotation and authored rotation at the press.
        frame: Box<RotationFrame>,
    },
    Scale {
        anchor: ScaleAnchor,
        /// What each handle does and the authored `localScale`, at the press.
        frame: Box<ScaleFrame>,
    },
    /// A point light's arrows (F060): the translate math on the light's own `position`.
    LightTranslate {
        anchor: Anchor,
        /// The document's `position` of the light at the press.
        base_position: crate::directed_scene::Vec3,
    },
    /// A point light's range knob (F060): the radius follows the pointer along the camera's right.
    LightRange {
        anchor: RangeAnchor,
        /// The document's `radius` of the light at the press.
        base_radius: f64,
    },
}

/// What an update of a drag accepted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Applied {
    Translate {
        world_delta: crate::directed_scene::Vec3,
        position: crate::directed_scene::Vec3,
    },
    Rotate {
        degrees: f64,
        rotation: crate::directed_scene::Quat,
    },
    Scale {
        factor: f64,
        scale: crate::directed_scene::Vec3,
    },
    LightTranslate {
        world_delta: crate::directed_scene::Vec3,
        position: crate::directed_scene::Vec3,
    },
    LightRange {
        delta: f64,
        radius: f64,
    },
}

impl Drag {
    pub fn gesture(&self) -> &str {
        &self.gesture
    }

    pub fn handle(&self) -> Handle {
        match &self.action {
            DragAction::Translate { anchor, .. } | DragAction::LightTranslate { anchor, .. } => {
                anchor.handle
            }
            DragAction::Rotate { ring, .. } => ring.handle,
            DragAction::Scale { anchor, .. } => anchor.handle,
            DragAction::LightRange { .. } => Handle::Range,
        }
    }

    /// The tool the drag was pressed with.
    pub fn tool(&self) -> GizmoTool {
        match &self.action {
            DragAction::Translate { .. }
            | DragAction::LightTranslate { .. }
            | DragAction::LightRange { .. } => GizmoTool::Translate,
            DragAction::Rotate { .. } => GizmoTool::Rotate,
            DragAction::Scale { .. } => GizmoTool::Scale,
        }
    }

    /// The edit frame the drag was pressed at.
    pub fn frame(&self) -> i64 {
        self.frame
    }

    /// The scene (attachment ID) the drag edits.
    pub fn scene(&self) -> &str {
        &self.scene
    }
}

impl SceneInteraction {
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// The gesture in progress, stale or not (`Gesture::invalidation` says which).
    pub fn gesture(&self) -> Option<&Gesture> {
        self.gesture.current()
    }

    /// The active tool of the gizmo.
    pub fn tool(&self) -> GizmoTool {
        self.tool
    }

    /// The orientation of the rotate and scale tools' axes.
    pub fn space(&self) -> GizmoSpace {
        self.space
    }

    /// The gizmo drag in progress, if any.
    pub fn drag(&self) -> Option<&Drag> {
        self.drag.as_ref()
    }

    /// The gizmo drag whose gesture is still the open one: the only kind a pointer command acts
    /// on. A stored drag whose gesture something else settled (`scene.gesture.cancel`, a new
    /// project) is dead: it stays stored until the next pointer command drops it, but it is not
    /// "a drag in progress". One criterion for every reader (the pointer commands and the
    /// settle rule of `EditorState::execute`), so they cannot disagree about a dead drag.
    pub fn live_drag(&self) -> Option<&Drag> {
        let drag = self.drag.as_ref()?;
        self.gesture
            .current()
            .is_some_and(|gesture| gesture.id() == drag.gesture)
            .then_some(drag)
    }

    /// The `scene_interaction` block of `EditorState::summary`, or `None` while there is nothing
    /// to show (no selection, nothing dropped, no gesture), so the summary of a project that
    /// never used the scene tools is what it always was. Read-only: it reflects a selection that
    /// went stale and a gesture that was invalidated, it does not prune or settle either.
    pub fn summary_json(&self, book: &SceneBook, revision: u64) -> Option<Value> {
        let selection = &self.selection;
        if selection.is_empty() && selection.dropped().is_empty() && self.gesture().is_none() {
            return None;
        }
        Some(json!({
            "selection": selection.summary_json(book),
            "gesture": self.gesture().map(|gesture| gesture.to_json(book, revision)),
            // F055: the pointer drag riding on that gesture, if the gizmo opened it.
            "drag": self.drag().map(|drag| json!({
                "gesture": drag.gesture(),
                "handle": drag.handle().as_str(),
                "tool": drag.tool().as_str(),
                "scene": drag.scene,
                "entity": drag.entity,
            })),
        }))
    }
}
