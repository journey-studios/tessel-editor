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
mod selection;
mod tree;

pub use address::{
    ensure_exists, entity_json, entity_of_property, entity_path, exists, kind_label, parse_entity,
};
pub use command::{FrameAt, GestureCommand, SelectMode};
pub use frame::frame_json;
pub use gesture::{Gesture, GestureSlot, Invalidation};
pub use selection::{Dropped, MAX_SELECTION, SUMMARY_LIMIT, Selected, Selection};
pub use tree::{MAX_TREE_DEPTH, tree_json};

/// The session state of the scene tools of one project: what is selected and which gesture is
/// open. Presentation, not document (see the module docs).
#[derive(Debug, Clone, Default)]
pub struct SceneInteraction {
    pub(crate) selection: Selection,
    pub(crate) gesture: GestureSlot,
}

impl SceneInteraction {
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// The gesture in progress, stale or not (`Gesture::invalidation` says which).
    pub fn gesture(&self) -> Option<&Gesture> {
        self.gesture.current()
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
        }))
    }
}
