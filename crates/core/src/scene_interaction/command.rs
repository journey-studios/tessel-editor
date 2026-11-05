//! The vocabulary of F039 as typed data, embedded in `scene_project::SceneCommand`. Parsing a
//! JSON request into it is `ops::parse_op`'s job; executing it is `scene_project/interaction.rs`.
//! Entity addresses stay text here: whether one is well formed or exists is decided when the
//! command executes, against the bundle, not by the parser.

use crate::scene_edit::EditOp;
use crate::time::Ratio;

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
