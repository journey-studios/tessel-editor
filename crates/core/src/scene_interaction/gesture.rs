//! The state of one semantic gesture over an attached scene, and what makes it stale.
//!
//! A gesture holds a *candidate*: the scene that its latest `update` produced from the scene as
//! it was at `begin`. The candidate is separate from the project: it lives here, is never put in
//! the `SceneBook`, never reaches the document or the history, and is dropped with the gesture.
//! Only `commit` writes, and it does so by running the gesture's ops through the same
//! `scene_edit` batch (and the same gates) as `scene.edit`, so there is no second executor.
//!
//! WHY the candidate is a pure function of `(base, ops)`: an `update` always starts again from
//! the base, so a drag that sends its latest value many times cannot compound, and a rejected
//! update leaves the previous good candidate untouched.
//!
//! A gesture is bound to what it saw: the attached scene (by `Arc` identity, which an edit, an
//! undo or a re-attach all change) and the project revision at `begin`. When either moved, the
//! gesture is *invalidated*: `update`, `commit` and preview reads fail with a typed
//! `gesture_invalidated` error and nothing is applied. Invalidation is checked when a gesture
//! is used, not pushed, because the project has no hook that observes undo/redo; the check is
//! cheap (a pointer comparison and an integer).

use std::sync::Arc;

use serde_json::{Value, json};

use crate::directed_scene::{DirectedScene, Entity};
use crate::scene_edit::{EditOp, EditSummary};
use crate::scene_project::SceneBook;
use crate::state::CommandError;

use super::address::entity_path;

/// Why a gesture can no longer be applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Invalidation {
    /// The attached scene was detached (or its attach was undone).
    SceneDetached,
    /// The attached scene is not the one the gesture started from: a `scene.edit`, an undo/redo
    /// that moved it, or a detach followed by an attach under the same ID.
    SceneChanged,
    /// The scene is the same, but the project revision moved: an edit elsewhere, or an
    /// undo/redo pair that put the same scene back. Conservative on purpose: the contract is
    /// "an edit or undo since `begin` invalidates", not "tries to guess which ones matter".
    ProjectRevision,
}

impl Invalidation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SceneDetached => "scene_detached",
            Self::SceneChanged => "scene_changed",
            Self::ProjectRevision => "project_revision",
        }
    }
}

#[derive(Debug, Clone)]
struct Candidate {
    scene: Arc<DirectedScene>,
    summary: EditSummary,
}

#[derive(Debug, Clone)]
pub struct Gesture {
    id: String,
    scene: String,
    label: Option<String>,
    targets: Vec<Entity>,
    /// The scene as it was at `begin`; every update is computed from it.
    base: Arc<DirectedScene>,
    /// `EditorState::project.revision` at `begin`.
    revision: u64,
    /// The ops of the latest accepted update (empty: nothing to commit).
    ops: Vec<EditOp>,
    candidate: Option<Candidate>,
    updates: u64,
}

impl Gesture {
    pub(crate) fn new(
        id: String,
        scene: String,
        label: Option<String>,
        targets: Vec<Entity>,
        base: Arc<DirectedScene>,
        revision: u64,
    ) -> Self {
        Self {
            id,
            scene,
            label,
            targets,
            base,
            revision,
            ops: Vec::new(),
            candidate: None,
            updates: 0,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// ID of the attached scene the gesture edits.
    pub fn scene(&self) -> &str {
        &self.scene
    }

    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    pub fn targets(&self) -> &[Entity] {
        &self.targets
    }

    /// Accepted updates so far (a rejected one is not counted).
    pub fn updates(&self) -> u64 {
        self.updates
    }

    pub fn ops(&self) -> &[EditOp] {
        &self.ops
    }

    pub fn has_candidate(&self) -> bool {
        self.candidate.is_some()
    }

    /// The scene a preview should show: the candidate, or the base when no update gave one.
    pub fn preview_scene(&self) -> &Arc<DirectedScene> {
        self.candidate
            .as_ref()
            .map_or(&self.base, |candidate| &candidate.scene)
    }

    pub(crate) fn base(&self) -> &Arc<DirectedScene> {
        &self.base
    }

    pub(crate) fn summary(&self) -> Option<&EditSummary> {
        self.candidate.as_ref().map(|candidate| &candidate.summary)
    }

    /// Replaces the candidate (and the ops that produce it). An empty batch returns the gesture
    /// to its start: nothing to preview, nothing to commit.
    pub(crate) fn set_candidate(
        &mut self,
        ops: Vec<EditOp>,
        candidate: Option<(Arc<DirectedScene>, EditSummary)>,
    ) {
        self.ops = ops;
        self.candidate = candidate.map(|(scene, summary)| Candidate { scene, summary });
        self.updates += 1;
    }

    /// `None` while the gesture can still be applied.
    pub fn invalidation(&self, book: &SceneBook, revision: u64) -> Option<Invalidation> {
        let Some(entry) = book.get(&self.scene) else {
            return Some(Invalidation::SceneDetached);
        };
        if !Arc::ptr_eq(&entry.scene, &self.base) {
            return Some(Invalidation::SceneChanged);
        }
        if revision != self.revision {
            return Some(Invalidation::ProjectRevision);
        }
        None
    }

    /// The typed error of a stale gesture, with the reason in `details`.
    pub(crate) fn invalidated_error(&self, reason: Invalidation) -> CommandError {
        CommandError::new(
            "gesture_invalidated",
            format!(
                "Gesture `{}` on scene `{}` is stale ({}); nothing was applied. \
                 Cancel it and begin a new one",
                self.id,
                self.scene,
                reason.as_str()
            ),
        )
        .target(self.id.clone())
        .details(json!({
            "gesture": self.id, "scene": self.scene, "reason": reason.as_str(),
        }))
    }

    /// The gesture as an answer prints it.
    pub fn to_json(&self, book: &SceneBook, revision: u64) -> Value {
        let invalidation = self.invalidation(book, revision);
        let mut value = json!({
            "id": self.id,
            "scene": self.scene,
            "label": self.label,
            "state": if invalidation.is_some() { "invalidated" } else { "open" },
            "invalidation": invalidation.map(Invalidation::as_str),
            "targets": self.targets.iter().map(entity_path).collect::<Vec<_>>(),
            "updates": self.updates,
            "has_candidate": self.candidate.is_some(),
            "revision": self.revision,
        });
        if let Some(summary) = self.summary() {
            value["would_change"] = json!(summary.modified);
            value["changed_ops"] = json!(summary.changed_count());
        }
        value
    }
}

/// The gesture slot of a project session, plus the counter that names gestures.
#[derive(Debug, Clone, Default)]
pub struct GestureSlot {
    current: Option<Gesture>,
    /// Only ever grows: IDs are never reused inside a session, so a late `commit` of a gesture
    /// that was cancelled cannot hit the next one.
    issued: u64,
}

impl GestureSlot {
    pub fn current(&self) -> Option<&Gesture> {
        self.current.as_ref()
    }

    pub(crate) fn next_id(&mut self) -> String {
        self.issued += 1;
        format!("gesture-{}", self.issued)
    }

    pub(crate) fn open(&mut self, gesture: Gesture) {
        self.current = Some(gesture);
    }

    pub(crate) fn take(&mut self) -> Option<Gesture> {
        self.current.take()
    }

    /// The gesture called `id`; `no_gesture` or `unknown_gesture` when there is none such.
    pub fn find(&self, id: &str) -> Result<&Gesture, CommandError> {
        match &self.current {
            None => Err(CommandError::new("no_gesture", "No gesture is in progress").target(id)),
            Some(gesture) if gesture.id == id => Ok(gesture),
            Some(gesture) => Err(CommandError::new(
                "unknown_gesture",
                format!("`{id}` is not the gesture in progress (`{}`)", gesture.id),
            )
            .target(id)),
        }
    }

    pub(crate) fn find_mut(&mut self, id: &str) -> Result<&mut Gesture, CommandError> {
        // `find` decides the error; the second lookup cannot fail once it passed.
        self.find(id)?;
        self.current
            .as_mut()
            .ok_or_else(|| CommandError::new("no_gesture", "No gesture is in progress").target(id))
    }
}
