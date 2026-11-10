//! The `scene.*` vocabulary as typed data. Parsing a JSON request into it is `ops::parse_op`'s
//! job (the one place every adapter shares); executing it is `run.rs` and `query.rs`. Nothing
//! here decides whether a bundle or an edit is valid.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

use crate::directed_scene::DirectedScene;
use crate::directed_scene::fx::Locale;

use crate::scene_edit::{EditOp, FxOp, PropertyRef};
use crate::scene_interaction::{
    FrameAt, FxGestureCommand, GestureCommand, GizmoCommand, PointerCommand, SelectMode,
};
use crate::time::Ratio;

/// Where the bytes of a bundle come from. It is read and validated when the command *executes*,
/// not when it is parsed, so a parser never carries an importer's rules.
#[derive(Debug, Clone, PartialEq)]
pub enum SceneSource {
    /// A JSON tree the caller already holds (a script line, a package entry).
    Bundle(Value),
    Text(String),
    /// A `render-bundle.json` on disk. Only the file is read: the resources it names by relative
    /// path are not opened.
    Path(PathBuf),
    /// A scene that was already read, imported and admitted (see [`AdmittedScene`]): attaching
    /// it only checks ID, room and budget, so the heavy work can run on a job thread.
    Admitted(AdmittedScene),
}

/// A bundle that passed the importer and the structural and temporal gates of `admit`, held
/// ready to attach, together with every fact attach needs that depends only on the bundle.
/// WHY opaque: the only way to make one is [`AdmittedScene::prepare`], so `SceneSource::Admitted`
/// can skip validation without letting a caller skip the rules, and the facts below are always
/// derived by the core from the very scene they travel with (a caller cannot hand in a count).
/// WHY the facts live here (BL100a): they cost O(bundle) (serializing 24 MiB of JSON to learn its
/// size, scanning every node for deferred families, one notice per unknown field) and `prepare`
/// runs on a job thread, whereas attaching runs on the caller's (the window's) thread.
#[derive(Debug, Clone, PartialEq)]
pub struct AdmittedScene {
    scene: Arc<DirectedScene>,
    // `Arc` so that cloning an admitted scene never copies what can be tens of thousands of notices.
    facts: Arc<SceneFacts>,
}

/// What `scene.attach` reports and budgets, computed once by [`AdmittedScene::prepare`].
#[derive(Debug, Clone, PartialEq)]
pub(super) struct SceneFacts {
    /// Compact JSON size of the document: what the project's scene budget counts.
    pub(super) serialized_len: usize,
    pub(super) counts: Value,
    pub(super) notices: Vec<Value>,
    pub(super) deferred_in_use: Vec<Value>,
}

impl AdmittedScene {
    pub(super) fn new(scene: DirectedScene) -> Self {
        let facts = SceneFacts {
            serialized_len: super::serialized_len(&scene),
            counts: super::query::counts(&scene),
            notices: super::query::notices_json(&scene),
            deferred_in_use: super::scope::deferred_in_use(scene.document()),
        };
        Self {
            scene: Arc::new(scene),
            facts: Arc::new(facts),
        }
    }

    pub fn scene(&self) -> &DirectedScene {
        &self.scene
    }

    /// Compact JSON bytes this scene will count against the project's scene budget
    /// ([`MAX_SCENE_BYTES`](super::MAX_SCENE_BYTES)), measured when it was prepared.
    pub fn serialized_len(&self) -> usize {
        self.facts.serialized_len
    }

    /// The scene and its facts, the facts moved out when this is their only owner.
    pub(super) fn into_parts(self) -> (Arc<DirectedScene>, SceneFacts) {
        let facts = Arc::try_unwrap(self.facts).unwrap_or_else(|shared| (*shared).clone());
        (self.scene, facts)
    }
}

/// The frame a camera query asks about.
#[derive(Debug, Clone, PartialEq)]
pub enum CameraAt {
    /// A frame of the edit (shot → instance → clip mapping included in the answer).
    EditFrame(i64),
    /// A time of the edit in exact seconds; the frame that contains it is evaluated.
    EditTime(Ratio),
    /// A frame of one camera clip, without going through a shot.
    ClipFrame { clip: String, frame: i64 },
}

#[derive(Debug, Clone, PartialEq)]
pub enum SceneCommand {
    /// Validates the bundle (importer, structural joins, temporal gates) and attaches it. With
    /// `id: None` the core generates `scene-<n>`.
    Attach {
        id: Option<String>,
        name: Option<String>,
        source: SceneSource,
    },
    Detach {
        scene: String,
    },
    /// An atomic batch of catalog edits. `dry_run` validates exactly as a real run would and
    /// changes nothing.
    Edit {
        scene: String,
        ops: Vec<EditOp>,
        dry_run: bool,
    },
    List,
    /// What `scene.*` can do and what a scene does not do yet.
    Capabilities,
    /// The editable properties (project-independent).
    Catalog,
    Describe {
        scene: String,
    },
    Properties {
        scene: String,
        /// Keeps the properties whose canonical path starts with it (`scene/street/node/`).
        path_prefix: Option<String>,
        /// Also lists the component properties (`position/x`); off by default. The default
        /// listing is not the first slice's: it lists the original eight kinds plus `renderOrder`
        /// of image planes; only the components are opt-in.
        components: bool,
    },
    Get {
        scene: String,
        property: PropertyRef,
    },
    Camera {
        scene: String,
        at: CameraAt,
    },
    /// F039: the entity tree of an attached scene (the whole bundle, or the subtree at `root`).
    Tree {
        scene: String,
        root: Option<String>,
        depth: Option<usize>,
    },
    /// F039: the whole evaluated frame (camera, nodes, what is left unevaluated). With a
    /// `gesture`, the frame of that gesture's candidate instead of the document's scene.
    Evaluate {
        scene: String,
        at: FrameAt,
        gesture: Option<String>,
    },
    /// Evaluated, versioned frame plan for render adapters. Partial is diagnostic only.
    Plan {
        scene: String,
        at: FrameAt,
        partial: bool,
        gesture: Option<String>,
    },
    /// F039: selects entities by address. `scene` is only optional for `SelectMode::Clear`.
    Select {
        scene: Option<String>,
        entities: Vec<String>,
        mode: SelectMode,
    },
    /// F039: the current selection (pruned of what no longer exists).
    Selection,
    /// F039: the begin/update/commit/cancel cycle.
    Gesture(GestureCommand),
    /// F055: the translation gizmo (overlay, hit test, pointer drag, semantic translate).
    Gizmo(GizmoCommand),
    /// F016: an atomic batch on the effects layer of a scene. `dry_run` validates exactly as a
    /// real run would and changes nothing.
    FxEdit {
        scene: String,
        ops: Vec<FxOp>,
        dry_run: bool,
        /// The project revision the caller's view was built on. With it, the edit is applied only
        /// while the project is still at that revision (`stale_revision` otherwise, nothing
        /// written), so an editor that was open on a key cannot write after the key moved, was
        /// undone or was edited from elsewhere. Without it the edit applies to what is there now.
        base_revision: Option<u64>,
    },
    /// F016: the effects layer of a scene (tracks, clips, every parameter resolved). With a
    /// `gesture`, the layer of that gesture's candidate (what a preview shows), not the document's.
    FxList {
        scene: String,
        gesture: Option<String>,
    },
    /// F016: the effects the editor draws, with their parameters and ranges.
    FxCatalog,
    /// F016: the passes the layer resolves at an edit frame (what the plan hands a runtime).
    /// With a `gesture`, the passes of its candidate.
    FxFrame {
        scene: String,
        at: FrameAt,
        gesture: Option<String>,
    },
    /// F058 (BL082.30): moves the monitor (the playhead) to a key of one parameter of an effect
    /// clip. The core names the bundle frame and the sequence frame; it moves no document.
    FxSeek {
        scene: String,
        clip: String,
        param: String,
        to: KeySeek,
    },
    /// F016 (BL082.6): the begin/update/commit/cancel cycle over one effect clip.
    FxGesture(FxGestureCommand),
    /// F016 (P2): the inspector model of the effects layer (what a person can set on each clip,
    /// with ranges, defaults, labels in a language, applicability) and the effects that can be
    /// added. A read: it writes nothing. With a `gesture`, the model of that gesture's candidate
    /// (plus the gesture's last refusal); with a `probe`, whether one proposed value would be
    /// accepted, judged by a dry run of the very edit the field would send.
    FxInspect {
        scene: String,
        clip: Option<String>,
        gesture: Option<String>,
        locale: Locale,
        probe: Option<FxProbe>,
        /// P3: the frame of the edit the animation is sampled at (`frame` or exact `time`); without
        /// one the keys and the base values are listed and nothing is sampled.
        at: Option<FrameAt>,
    },
}

/// Which key `scene.fx.seek` goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySeek {
    /// The nearest key before the frame the monitor shows.
    Previous,
    /// The nearest key after it.
    Next,
    First,
    Last,
    /// The key at this frame **of the clip** (not of the edit).
    Frame(i64),
}

impl KeySeek {
    pub fn parse(value: &Value) -> Option<Self> {
        match value {
            Value::String(name) => match name.as_str() {
                "previous" => Some(Self::Previous),
                "next" => Some(Self::Next),
                "first" => Some(Self::First),
                "last" => Some(Self::Last),
                _ => None,
            },
            other => other.as_i64().map(Self::Frame),
        }
    }
}

/// A value proposed for one field of one clip, to be judged without being applied.
#[derive(Debug, Clone, PartialEq)]
pub struct FxProbe {
    pub clip: String,
    pub field: String,
    pub value: f64,
    /// Round to the field's grid first, as a pointer or a key would (a typed value is judged as is).
    pub quantize: bool,
}

impl SceneCommand {
    /// Name under which the command appears in the undo list. Reads, dry runs and (through
    /// `EditorState::execute`'s `changed: false` check) no-ops are not edits.
    pub fn history_label(&self) -> Option<&'static str> {
        match self {
            Self::Attach { .. } => Some("Attach scene"),
            Self::Detach { .. } => Some("Remove scene"),
            Self::Edit { dry_run: false, .. } => Some("Edit scene"),
            // The label of a gesture commit is the one of `scene.edit` on purpose: a commit IS
            // that edit, so the history shows the same entry for either. Every outcome of a
            // commit carries `changed`, and `EditorState::execute` records nothing when it is
            // false (empty gesture, net no-op) or when the commit is rejected.
            Self::Gesture(GestureCommand::Commit { .. }) => Some("Edit scene"),
            // The release of a gizmo drag IS that commit, and `scene.translate` IS a
            // `scene.edit`: both report `changed` as "the document changed", so a click that
            // moved nothing leaves no entry. Press, move, cancel and the reads never do.
            Self::Gizmo(GizmoCommand::Pointer(PointerCommand::Up { .. })) => Some("Edit scene"),
            Self::Gizmo(GizmoCommand::Translate { .. }) => Some("Edit scene"),
            Self::FxEdit { dry_run: false, .. } => Some("Edit effects"),
            // The commit of an fx gesture IS a `scene.fx.edit` of the same ops (it runs through
            // `edit_fx`), so it shows the same entry; `changed: false` records nothing.
            Self::FxGesture(FxGestureCommand::Commit { .. }) => Some("Edit effects"),
            _ => None,
        }
    }
}
