//! The `scene.*` vocabulary as typed data. Parsing a JSON request into it is `ops::parse_op`'s
//! job (the one place every adapter shares); executing it is `run.rs` and `query.rs`. Nothing
//! here decides whether a bundle or an edit is valid.

use std::path::PathBuf;

use serde_json::Value;

use crate::scene_edit::{EditOp, PropertyRef};
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
    },
    Get {
        scene: String,
        property: PropertyRef,
    },
    Camera {
        scene: String,
        at: CameraAt,
    },
}

impl SceneCommand {
    /// Name under which the command appears in the undo list. Reads, dry runs and (through
    /// `EditorState::execute`'s `changed: false` check) no-ops are not edits.
    pub fn history_label(&self) -> Option<&'static str> {
        match self {
            Self::Attach { .. } => Some("Attach scene"),
            Self::Detach { .. } => Some("Remove scene"),
            Self::Edit { dry_run: false, .. } => Some("Edit scene"),
            _ => None,
        }
    }
}
