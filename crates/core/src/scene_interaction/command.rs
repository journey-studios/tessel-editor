//! The vocabulary of F039 as typed data, embedded in `scene_project::SceneCommand`. Parsing a
//! JSON request into it is `ops::parse_op`'s job; executing it is `scene_project/interaction.rs`.
//! Entity addresses stay text here: whether one is well formed or exists is decided when the
//! command executes, against the bundle, not by the parser.

use crate::directed_scene::Vec3;
use crate::scene_edit::{EditOp, FxOp};
use crate::time::Ratio;

use super::gizmo::{GizmoSpace, GizmoTool, Handle, Point};

/// The edit frame a frame evaluation asks about.
#[derive(Debug, Clone, PartialEq)]
pub enum FrameAt {
    Frame(i64),
    /// Exact seconds; the frame that contains it is evaluated.
    Time(Ratio),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectMode {
    /// The selection becomes exactly these entities (of one attached scene).
    Replace,
    Add,
    Remove,
    Toggle,
    /// Empties the selection; with a scene, only that scene's part of it.
    Clear,
}

impl SelectMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Replace => "replace",
            Self::Add => "add",
            Self::Remove => "remove",
            Self::Toggle => "toggle",
            Self::Clear => "clear",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        [
            Self::Replace,
            Self::Add,
            Self::Remove,
            Self::Toggle,
            Self::Clear,
        ]
        .into_iter()
        .find(|mode| mode.as_str() == raw)
    }
}

/// The semantic gesture cycle: `begin`, any number of `update`s, then `commit` or `cancel`.
/// No pointer, key or coordinate appears here; an adapter that has a drag turns it into these.
#[derive(Debug, Clone, PartialEq)]
pub enum GestureCommand {
    /// Opens a gesture over one attached scene. `targets` are the entities the gesture claims
    /// to edit; when given, an update may only touch properties of those entities.
    Begin {
        scene: String,
        targets: Vec<String>,
        label: Option<String>,
    },
    /// Replaces the candidate with the result of `ops` applied to the scene as it was at `begin`
    /// (absolute, not cumulative: a drag sends its latest value each time).
    Update {
        gesture: String,
        ops: Vec<EditOp>,
    },
    /// Applies the candidate as ONE batch on the project's history.
    Commit {
        gesture: String,
    },
    /// Discards the candidate. Always allowed on a gesture that was invalidated meanwhile.
    Cancel {
        gesture: String,
    },
    Status,
}

/// The gesture cycle over ONE effect clip (F016, BL082.6): the same `begin` / `update` / `commit`
/// / `cancel` contract as [`GestureCommand`], with `fx` ops. An update is absolute (it restarts
/// from the scene as it was at `begin`), a refused one keeps the last legal candidate, and only
/// `commit` writes: ONE `Edit effects` entry, none when the candidate equals the base.
#[derive(Debug, Clone, PartialEq)]
pub enum FxGestureCommand {
    /// Opens a gesture over the clip `clip` of the effects layer of attached scene `scene`.
    Begin {
        scene: String,
        clip: String,
        label: Option<String>,
    },
    /// Replaces the candidate with `ops` applied to the scene as it was at `begin`. Only
    /// `set_param`, `set_field` and `set_hidden` on the gesture's own clip are accepted.
    Update { gesture: String, ops: Vec<FxOp> },
    /// Commits the last legal candidate as one undo entry; a no-op creates none.
    Commit { gesture: String },
    /// Discards the candidate; always allowed, also on a gesture that went stale.
    Cancel { gesture: String },
    /// Reports the slot, with FX budget/warnings only for a live effect candidate.
    Status,
}

/// Why a pointer interaction ended without being committed. Recorded in the answer; the effect is
/// the same for every reason (the candidate is discarded, nothing reaches the document), but a
/// proof or a log can tell Escape from a lost focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelReason {
    /// The Escape key.
    Escape,
    /// The window or the page lost focus: a release may never arrive (BL043), so the drag is
    /// cancelled, never committed.
    Blur,
    /// The platform cancelled the pointer (`pointercancel`, a lost capture).
    PointerCancel,
    /// The view was hidden (monitor closed, a dialog opened, the app went to the background).
    Hidden,
    /// The page that drew the gizmo went away (reload, crash, lost context) or no longer shows
    /// the scene: nobody can release the press any more.
    Unavailable,
    /// A script or another command asked.
    Api,
}

impl CancelReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Escape => "escape",
            Self::Blur => "blur",
            Self::PointerCancel => "pointer_cancel",
            Self::Hidden => "hidden",
            Self::Unavailable => "unavailable",
            Self::Api => "api",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        [
            Self::Escape,
            Self::Blur,
            Self::PointerCancel,
            Self::Hidden,
            Self::Unavailable,
            Self::Api,
        ]
        .into_iter()
        .find(|reason| reason.as_str() == raw)
    }
}

/// One step of a pointer interaction with the gizmo. Positions are on the picture (see
/// `scene_interaction::gizmo`); no device, key or window size appears here.
#[derive(Debug, Clone, PartialEq)]
pub enum PointerCommand {
    /// The press. On a handle of the selected node it begins a drag (a gesture); on an image
    /// plane it selects it (`additive`: toggles it in the selection); on empty picture it clears
    /// the selection of the scene (`additive` keeps it).
    Down {
        scene: String,
        at: FrameAt,
        point: Point,
        additive: bool,
    },
    /// The pointer moved during a drag: the candidate becomes the node at that position.
    Move { point: Point },
    /// The release: commits the drag as ONE undo entry (with `point`, after one last move).
    Up { point: Option<Point> },
    /// Escape, lost focus, a cancelled pointer: discards the drag.
    Cancel { reason: CancelReason },
}

/// The gizmo vocabulary (`scene.gizmo.*`, `scene.pointer.*`, `scene.translate`, `scene.rotate`).
#[derive(Debug, Clone, PartialEq)]
pub enum GizmoCommand {
    /// The overlay of the scene at a frame: what a runtime draws for the current selection (the
    /// live drag's candidate when one is open on this scene).
    State {
        scene: String,
        at: FrameAt,
    },
    /// The hit test alone: what is under `point` (a handle, an image plane or nothing), with no
    /// effect on the selection.
    Hit {
        scene: String,
        at: FrameAt,
        point: Point,
    },
    Pointer(PointerCommand),
    /// The semantic translation: moves one node by `delta` in the world, as a drag of the gizmo
    /// would, in one `scene.edit`. The same result as the drag that reported that `world_delta`.
    Translate {
        scene: String,
        entity: String,
        at: FrameAt,
        delta: Vec3,
    },
    /// The semantic rotation (BL015.1): turns one node by `degrees` about the `axis` ring (`x`,
    /// `y`, `z`; never `screen`) in `space` (the session's when `None`), in one `scene.edit`. The
    /// same result as the ring drag that reported that `degrees`, `axis` and `space`.
    Rotate {
        scene: String,
        entity: String,
        at: FrameAt,
        axis: Handle,
        degrees: f64,
        space: Option<GizmoSpace>,
    },
    /// Reads or sets the tool (translate/rotate) and the space of the rotate axes. With neither
    /// field it is a read. Presentation state, not an edit: no history, no revision. Refused
    /// while a drag is open (a ring cannot turn into an arrow under the pointer).
    Tool {
        tool: Option<GizmoTool>,
        space: Option<GizmoSpace>,
    },
}
