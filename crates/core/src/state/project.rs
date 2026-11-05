//! Project identity, revision/dirty tracking and sequence management.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use super::timeline::{SequenceSettings, Timeline};
use super::{CommandError, EditorState};
use crate::id::Id;
use crate::preset::Preset;

/// What a project document carries besides the model: identity and bookkeeping.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectMeta {
    /// Stable across saves and renames; autosave and recovery are keyed by it.
    pub id: String,
    /// Bumped by every edit, undo and redo. Not persisted as the edit counter of the next session.
    pub revision: u64,
    /// Revision that is on disk, `None` while the project was never saved.
    pub saved_revision: Option<u64>,
    /// Folder holding managed media, derived caches and recovery data.
    pub workspace: Option<PathBuf>,
    /// The `.tessel.json` the project was opened from or last saved to.
    pub document: Option<PathBuf>,
    /// Export preset the project selected (a preset ID, see `export.rs`).
    pub export_preset: Option<String>,
    /// User presets saved with the project, as validated preset JSON.
    pub user_presets: Vec<Value>,
}

impl ProjectMeta {
    pub fn new() -> Self {
        Self {
            id: new_project_id(),
            revision: 0,
            saved_revision: None,
            workspace: None,
            document: None,
            export_preset: None,
            user_presets: Vec::new(),
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.saved_revision != Some(self.revision)
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id, "revision": self.revision, "saved_revision": self.saved_revision,
            "dirty": self.is_dirty(),
            "workspace": self.workspace.as_ref().map(|path| path.to_string_lossy()),
            "document": self.document.as_ref().map(|path| path.to_string_lossy()),
        })
    }
}

impl Default for ProjectMeta {
    fn default() -> Self {
        Self::new()
    }
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// `p-<hex>` from the clock, the process and a counter: unique enough to key recovery data
/// without a random-number dependency. It is an identifier, not a secret.
pub fn new_project_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mixed = (nanos as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .rotate_left(17)
        ^ (u64::from(std::process::id()) << 32)
        ^ count.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    format!("p-{mixed:016x}")
}

impl EditorState {
    pub(crate) fn save_preset(&mut self, value: &Value) -> Result<Value, CommandError> {
        let preset = Preset::from_json(value)?;
        if crate::preset::BUILTIN_IDS.contains(&preset.id.as_str()) {
            return Err(CommandError::new(
                "invalid_preset",
                format!(
                    "`{}` is a built-in preset; save a copy under another ID",
                    preset.id
                ),
            )
            .target(preset.id));
        }
        let stored = preset.to_json();
        let replaced = match self
            .project
            .user_presets
            .iter_mut()
            .find(|existing| existing["id"] == stored["id"])
        {
            Some(existing) => {
                *existing = stored.clone();
                true
            }
            None => {
                self.project.user_presets.push(stored.clone());
                false
            }
        };
        Ok(
            json!({ "action": "preset.save", "preset": stored, "replaced": replaced, "changed": true }),
        )
    }

    pub(crate) fn delete_preset(&mut self, id: &str) -> Result<Value, CommandError> {
        let before = self.project.user_presets.len();
        self.project
            .user_presets
            .retain(|preset| preset["id"] != id);
        if self.project.user_presets.len() == before {
            return Err(CommandError::unknown_element("preset", id));
        }
        if self.project.export_preset.as_deref() == Some(id) {
            self.project.export_preset = None;
        }
        Ok(json!({ "action": "preset.delete", "id": id, "changed": true }))
    }

    pub(crate) fn select_preset(&mut self, id: &str) -> Result<Value, CommandError> {
        crate::preset::find(id, &self.project.user_presets)?;
        let changed = self.project.export_preset.as_deref() != Some(id);
        self.project.export_preset = Some(id.to_string());
        Ok(json!({ "action": "preset.select", "id": id, "changed": changed }))
    }

    /// Every sequence, active first.
    pub fn all_sequences(&self) -> impl Iterator<Item = &Timeline> {
        std::iter::once(&self.timeline).chain(self.sequences.iter())
    }

    pub fn sequence(&self, id: impl AsRef<str>) -> Option<&Timeline> {
        let id = id.as_ref();
        self.all_sequences().find(|timeline| timeline.id == id)
    }

    pub(crate) fn clip_id_used_in_other_sequence(&self, id: &Id) -> bool {
        self.sequences
            .iter()
            .any(|timeline| timeline.clip(id).is_some())
    }

    /// `<prefix>-<n>` not used as a clip in ANY sequence: clip properties are keyed by clip ID.
    pub(crate) fn fresh_clip_id(&self, prefix: &str) -> Id {
        (1..)
            .filter_map(|n| Id::new(&format!("{prefix}-{n}")).ok())
            .find(|id| {
                self.timeline.track(id).is_none()
                    && self.timeline.clip(id).is_none()
                    && !self.clip_id_used_in_other_sequence(id)
            })
            .unwrap_or_else(|| Id::lit(prefix))
    }

    pub(crate) fn add_sequence(
        &mut self,
        id: Option<String>,
        name: Option<String>,
        settings: Option<SequenceSettings>,
        activate: bool,
    ) -> Result<Value, CommandError> {
        let id = match id {
            Some(raw) => Id::new(&raw)
                .map_err(|error| CommandError::new("invalid_id", error.to_string()).target(raw))?,
            None => (2..)
                .filter_map(|n| Id::new(&format!("sequence-{n}")).ok())
                .find(|id| self.sequence(id).is_none())
                .unwrap_or_else(|| Id::lit("sequence")),
        };
        if self.sequence(id).is_some() {
            return Err(CommandError::new(
                "duplicate_id",
                format!("A sequence with ID `{id}` already exists"),
            )
            .target(id.as_str()));
        }
        // WHY default to the active sequence's format: a new sequence in a project is most
        // often a variant (cut-down, vertical) of the current one.
        let base = settings.unwrap_or(SequenceSettings {
            id,
            rate: self.timeline.rate,
            length: self.timeline.length,
            width: self.timeline.width,
            height: self.timeline.height,
        });
        let settings = SequenceSettings { id, ..base };
        settings.validate()?;
        let mut timeline = Timeline::empty(&settings);
        if let Some(name) = name
            .map(|name| name.trim().to_string())
            .filter(|n| !n.is_empty())
        {
            timeline.name = name;
        }
        self.sequences.push(timeline);
        if activate {
            self.activate_sequence(id.as_str())?;
        }
        Ok(
            json!({ "action": "sequence.add", "sequence": id, "active": activate,
            "sequences": self.all_sequences().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            "changed": true }),
        )
    }

    pub(crate) fn activate_sequence(&mut self, id: &str) -> Result<Value, CommandError> {
        if self.timeline.id == id {
            return Ok(json!({ "action": "sequence.activate", "sequence": id, "changed": false }));
        }
        let index = self
            .sequences
            .iter()
            .position(|timeline| timeline.id == id)
            .ok_or_else(|| CommandError::unknown_element("sequence", id))?;
        std::mem::swap(&mut self.timeline, &mut self.sequences[index]);
        // The sequence that was active keeps its slot, so the order of `sequences` is stable.
        self.subject = super::Subject::Media;
        Ok(json!({ "action": "sequence.activate", "sequence": id,
            "previous": self.sequences[index].id, "changed": true }))
    }

    pub(crate) fn remove_sequence(&mut self, id: &str) -> Result<Value, CommandError> {
        if self.timeline.id == id {
            return Err(CommandError::new(
                "sequence_active",
                "Activate another sequence before removing this one",
            )
            .target(id));
        }
        let index = self
            .sequences
            .iter()
            .position(|timeline| timeline.id == id)
            .ok_or_else(|| CommandError::unknown_element("sequence", id))?;
        let removed = self.sequences.remove(index);
        for clip in &removed.clips {
            self.inspector.forget_clip(clip.id.as_str());
        }
        Ok(
            json!({ "action": "sequence.remove", "sequence": id, "clips": removed.clips.len(),
            "changed": true }),
        )
    }

    pub(crate) fn rename_sequence(&mut self, id: &str, name: &str) -> Result<Value, CommandError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(CommandError::invalid_value(
                "A sequence needs a name",
                json!({}),
            ));
        }
        let timeline = if self.timeline.id == id {
            Some(&mut self.timeline)
        } else {
            self.sequences.iter_mut().find(|timeline| timeline.id == id)
        }
        .ok_or_else(|| CommandError::unknown_element("sequence", id))?;
        let changed = timeline.name != name;
        timeline.name = name.to_string();
        Ok(json!({ "action": "sequence.rename", "sequence": id, "name": name, "changed": changed }))
    }
}
