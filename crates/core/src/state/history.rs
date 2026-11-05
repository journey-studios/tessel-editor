//! Undo/redo over domain transactions.
//!
//! WHY snapshots instead of inverse commands: every edit (move, trim, property, import,
//! project-import, directed scenes) would need a hand-written inverse, and a missed one
//! silently corrupts the undo chain. A snapshot of the *model* (library items, every sequence,
//! clip properties, attached scenes) is small compared to the media it points at, and it makes
//! undo exact by construction. The
//! cost is memory proportional to project metadata × `HISTORY_LIMIT`; media bytes are never
//! part of a snapshot, so undo cannot delete a file (see `docs/features/history.md`).
//!
//! Presentation (playhead, zoom, snap, selection, search, tabs) is not undone: those are not
//! edits, and rewinding the playhead on every undo is hostile.
//!
//! A slider drag is ONE transaction, not one per tick: the open gesture (`property_gesture`)
//! lives in [`History`], applies values live without recording, and records a single entry
//! when it ends. Every path that records or rewinds (`transaction`, `undo`, `redo`) settles an
//! open gesture first, so the stack never holds a half-finished one.

use serde_json::{Value, json};

use super::inspector::Overrides;
use super::library::{Folder, MediaItem};
use super::property_gesture::OpenGesture;
use super::timeline::Timeline;
use super::{CommandError, EditorState};
use crate::id::Id;
use crate::scene_project::SceneBook;

/// Entries kept per direction. Older edits are forgotten, newest first.
pub const HISTORY_LIMIT: usize = 200;

/// The persistent model of a project at one instant.
#[derive(Debug, Clone)]
pub struct Snapshot {
    project_name: String,
    folders: Vec<Folder>,
    items: Vec<MediaItem>,
    active: Timeline,
    others: Vec<Timeline>,
    overrides: Overrides,
    /// Shares each attached bundle (`Arc`), so a snapshot costs pointers, not documents.
    scenes: SceneBook,
}

#[derive(Debug, Clone)]
struct Entry {
    label: String,
    snapshot: Snapshot,
}

#[derive(Debug, Clone, Default)]
pub struct History {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    /// > 0 while a transaction runs: nested commands do not record their own entry.
    depth: u32,
    /// The slider gesture that is previewing right now, if any. Not an entry: it becomes one
    /// (or nothing) when it ends.
    pub(super) gesture: Option<OpenGesture>,
    /// Last gesture ID handed out; IDs only grow, so a late event of an old drag cannot
    /// be mistaken for the current gesture.
    pub(super) next_gesture: u64,
}

impl History {
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|entry| entry.label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|entry| entry.label.as_str())
    }

    pub fn is_recording(&self) -> bool {
        self.depth == 0
    }

    pub fn to_json(&self) -> Value {
        json!({
            "can_undo": self.can_undo(), "can_redo": self.can_redo(),
            "undo_label": self.undo_label(), "redo_label": self.redo_label(),
            "undo_depth": self.undo.len(), "redo_depth": self.redo.len(),
            // `null` unless a slider gesture is previewing: its value is applied but is not
            // an entry yet.
            "gesture": self.gesture.as_ref().map(|open| {
                let info = open.info();
                json!({ "id": info.id, "clip": format!("clip:{}", info.clip),
                    "property": format!("property:{}", info.property) })
            }),
        })
    }

    pub(super) fn record(&mut self, label: &str, snapshot: Snapshot) {
        self.undo.push(Entry {
            label: label.to_string(),
            snapshot,
        });
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
}

impl EditorState {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            project_name: self.project_name.clone(),
            folders: self.library.folders.clone(),
            items: self.library.items.clone(),
            active: self.timeline.clone(),
            others: self.sequences.clone(),
            overrides: self.inspector.overrides_snapshot(),
            scenes: self.scenes.clone(),
        }
    }

    /// Puts `snapshot` back, keeping what is presentation (playhead, zoom, snap, selection).
    fn restore(&mut self, snapshot: Snapshot) {
        let presentation = |old: &Timeline, new: &mut Timeline| {
            new.zoom = old.zoom;
            new.snap = old.snap;
            new.playhead = old.playhead.min(new.length);
            new.selected_clip = old.selected_clip.filter(|id| new.clip(id).is_some());
            new.selected_track = old.selected_track.filter(|id| new.track(id).is_some());
        };
        let all_old: Vec<&Timeline> = std::iter::once(&self.timeline)
            .chain(self.sequences.iter())
            .collect();
        let mut active = snapshot.active;
        let mut others = snapshot.others;
        for timeline in std::iter::once(&mut active).chain(others.iter_mut()) {
            if let Some(old) = all_old.iter().find(|old| old.id == timeline.id) {
                presentation(old, timeline);
            }
        }
        self.project_name = snapshot.project_name;
        self.library.folders = snapshot.folders;
        self.library.items = snapshot.items;
        self.timeline = active;
        self.sequences = others;
        self.inspector.restore_overrides(snapshot.overrides);
        self.scenes = snapshot.scenes;
        self.library.selected_folder = self
            .library
            .selected_folder
            .filter(|folder| self.library.folder(folder).is_some());
        self.selected_media = self
            .selected_media
            .filter(|media| self.library.item(media).is_some());
        self.library.search.clear();
    }

    /// Runs `body` as ONE undoable edit that either fully applies or leaves the project
    /// exactly as it was. Used by batch imports and project-to-project merges.
    pub fn transaction<R>(
        &mut self,
        label: &str,
        body: impl FnOnce(&mut Self) -> Result<R, CommandError>,
    ) -> Result<R, CommandError> {
        // A gesture settles BEFORE the snapshot, so its entry stays separate from this batch.
        self.settle_property_gesture();
        let before = self.snapshot();
        self.history.depth += 1;
        let outcome = body(self);
        self.history.depth -= 1;
        match outcome {
            Ok(value) => {
                if self.history.is_recording() {
                    self.history.record(label, before);
                    self.project.revision += 1;
                }
                Ok(value)
            }
            Err(error) => {
                // WHY restore even though commands validate first: a body may run several
                // commands, and the first ones already mutated when a later one fails.
                self.restore_exact(before);
                Err(error)
            }
        }
    }

    /// Like [`restore`](Self::restore) but also resets presentation: used to roll back a
    /// failed transaction, where nothing the user did should survive.
    fn restore_exact(&mut self, snapshot: Snapshot) {
        let selected_media = self.selected_media;
        let playhead = self.timeline.playhead;
        self.restore(snapshot);
        self.selected_media = selected_media.filter(|media| self.library.item(media).is_some());
        self.timeline.playhead = playhead.min(self.timeline.length);
    }

    /// Records `before` as an undo entry for an edit that already ran.
    pub(crate) fn record_edit(&mut self, label: &str, before: Snapshot) {
        if self.history.is_recording() {
            self.history.record(label, before);
            self.project.revision += 1;
        }
    }

    pub fn undo(&mut self) -> Result<Value, CommandError> {
        // Undo acts on a finished edit: a gesture that is still previewing becomes that edit.
        self.settle_property_gesture();
        let Some(entry) = self.history.undo.pop() else {
            return Err(CommandError::new(
                "nothing_to_undo",
                "There is nothing to undo",
            ));
        };
        let current = self.snapshot();
        self.restore(entry.snapshot);
        self.history.redo.push(Entry {
            label: entry.label.clone(),
            snapshot: current,
        });
        self.project.revision += 1;
        Ok(json!({ "action": "history.undo", "label": entry.label,
            "history": self.history.to_json(), "changed": true }))
    }

    pub fn redo(&mut self) -> Result<Value, CommandError> {
        // Settling a gesture that changed the value records an entry and so clears redo:
        // pressing redo mid-drag reports `nothing_to_redo`, exactly as after any new edit.
        self.settle_property_gesture();
        let Some(entry) = self.history.redo.pop() else {
            return Err(CommandError::new(
                "nothing_to_redo",
                "There is nothing to redo",
            ));
        };
        let current = self.snapshot();
        self.restore(entry.snapshot);
        self.history.undo.push(Entry {
            label: entry.label.clone(),
            snapshot: current,
        });
        self.project.revision += 1;
        Ok(json!({ "action": "history.redo", "label": entry.label,
            "history": self.history.to_json(), "changed": true }))
    }

    /// Forgets undo/redo (a loaded or newly created project starts clean).
    pub fn clear_history(&mut self) {
        // The preview value stays in the model (it was on screen); only its entry is dropped
        // with the rest of the history.
        self.settle_property_gesture();
        // Gesture IDs keep growing across a reset: a late event of an old drag must not match
        // the first gesture of the next project.
        let next_gesture = self.history.next_gesture;
        self.history = History::default();
        self.history.next_gesture = next_gesture;
    }

    /// IDs referenced by clips that no longer resolve: a consistency check used after restore.
    pub fn dangling_clip_media(&self) -> Vec<Id> {
        std::iter::once(&self.timeline)
            .chain(self.sequences.iter())
            .flat_map(|timeline| timeline.clips.iter())
            .filter_map(|clip| clip.media)
            .filter(|media| self.library.item(media).is_none())
            .collect()
    }
}
