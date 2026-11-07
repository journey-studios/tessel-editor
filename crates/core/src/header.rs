//! What the window header says about the open project (BL144).
//!
//! The header used to show a fixed "North / Master" whatever was open. It now reads the live
//! [`EditorState`], so a project opened, created, restored or renamed through the library, the
//! headless CLI or the window is the one the header names, and `project.header` answers the
//! same text on every surface. Presentation only: nothing here edits the project.

use serde_json::{Value, json};

use crate::state::EditorState;

/// The application name shown before the project.
pub const APP: &str = "Tessel Editor";

#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    pub app: &'static str,
    pub project: String,
    /// The active sequence.
    pub sequence: String,
    pub project_id: String,
    pub dirty: bool,
    /// The `.tessel.json` the project lives in, `None` while it was never saved.
    pub document: Option<String>,
}

impl Header {
    /// `Project / Sequence`: the one line the header shows.
    pub fn label(&self) -> String {
        if self.sequence.is_empty() {
            self.project.clone()
        } else {
            format!("{} / {}", self.project, self.sequence)
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "app": self.app, "label": self.label(), "project": self.project,
            "sequence": self.sequence, "project_id": self.project_id,
            "dirty": self.dirty, "document": self.document,
        })
    }
}

impl EditorState {
    pub fn header(&self) -> Header {
        Header {
            app: APP,
            project: self.project_name.clone(),
            sequence: self.timeline.name.clone(),
            project_id: self.project.id.clone(),
            dirty: self.project.is_dirty(),
            document: self
                .project
                .document
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn the_label_follows_the_project_and_the_active_sequence() {
        let mut state = EditorState::demo();
        assert_eq!(state.header().label(), "North expedition / Master sequence");
        state.project_name = "Another film".into();
        state.timeline.name = "Cut 2".into();
        assert_eq!(state.header().label(), "Another film / Cut 2");
        state.timeline.name.clear();
        assert_eq!(state.header().label(), "Another film");
    }

    #[test]
    fn the_json_names_the_identity_and_the_save_state() {
        let state = EditorState::demo();
        let json = state.header().to_json();
        assert_eq!(json["app"], APP);
        assert_eq!(json["project_id"], state.project.id);
        assert_eq!(json["dirty"], state.project.is_dirty());
        assert!(json["document"].is_null());
    }
}
