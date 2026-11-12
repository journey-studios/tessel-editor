//! Demonstration editor state, independent of GPUI.
//!
//! Every interaction, whether it comes from a gesture or from the automation
//! socket, becomes a [`Command`] and goes through [`EditorState::execute`].
//! There is no second copy of this state anywhere.

pub mod camera_edit;
pub mod error;
pub mod history;
pub mod inspector;
pub mod layout;
pub mod library;
pub mod project;
pub mod property_gesture;
pub mod timeline;
pub mod tree;

use serde_json::{Value, json};

pub use error::{CommandError, Raw};
use history::History;
use inspector::{Inspector, InspectorTab};
use library::{Library, MediaItem};
pub use project::{ProjectMeta, ProjectSession, ProjectStamp};
use timeline::{Clip, ClipKind, Placement, SequenceSettings, Timeline, TrackFlag, TrackKind};

use crate::assets::{Asset, MediaKind, ProxyFormat};
use crate::content::{Camera, TextSpec};
use crate::id::Id;
use crate::plan::PreviewIntent;
use crate::preview::{PreviewMode, PreviewPolicy};
use crate::scene_interaction::{GizmoCommand, PointerCommand, SceneInteraction};
use crate::scene_project::{SceneBook, SceneCommand};
use crate::time::Ratio;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceTab {
    Edit,
    Color,
    Deliver,
}

impl WorkspaceTab {
    pub const ALL: [Self; 3] = [Self::Edit, Self::Color, Self::Deliver];

    pub fn id(self) -> &'static str {
        match self {
            Self::Edit => "edit",
            Self::Color => "color",
            Self::Deliver => "deliver",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tab| tab.id() == raw.trim())
    }
}

/// What the monitor and the inspector header describe: the last thing the user
/// picked, a library item or a timeline clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subject {
    Media,
    Clip,
}

#[derive(Debug, Clone)]
pub struct EditorState {
    pub project_name: String,
    /// `true` only for the illustrative bootstrap; projects built through the API are real.
    pub demonstration: bool,
    pub library: Library,
    pub inspector: Inspector,
    /// The ACTIVE sequence. Every plan, edit and the monitor read this one; other sequences
    /// wait in [`sequences`](Self::sequences). WHY a swap model instead of a `Vec` plus an
    /// index: the whole editing surface (and 200+ call sites) addresses `timeline` directly,
    /// so multi-sequence arrives without a second code path (see `docs/features/sequences.md`).
    pub timeline: Timeline,
    /// Inactive sequences in a stable order (activating swaps one with `timeline`).
    pub sequences: Vec<Timeline>,
    pub project: ProjectMeta,
    pub history: History,
    pub workspace_tab: WorkspaceTab,
    pub selected_media: Option<Id>,
    pub subject: Subject,
    /// What the single monitor is asked to show. Independent of selection: clicking a
    /// library item never changes the sequence preview unless a caller asks for it.
    pub preview: PreviewIntent,
    /// Which quality the monitor's interactive frames may use (see `preview.rs`). A viewing
    /// preference like [`preview`](Self::preview): not saved, not an edit.
    pub preview_policy: PreviewPolicy,
    /// Directed scenes attached to the project (F036). Part of the document and of every undo
    /// snapshot; empty for a project that never attached one.
    pub scenes: SceneBook,
    /// F039: selection of scene entities and the open scene gesture. Session state, like the
    /// playhead: not in the document, not in an undo snapshot (so undo/redo never restores it),
    /// and empty in every project that is created or opened.
    pub scene_interaction: SceneInteraction,
    /// BL144: library thumbnails of splats and scenes. Session state like the playhead: not in
    /// the document, not in an undo snapshot, empty in every project that is created or opened.
    pub thumbnails: crate::thumbnails::ThumbnailBook,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    SetFolderExpanded {
        folder: String,
        expanded: bool,
    },
    SelectFolder {
        folder: Option<String>,
    },
    SetMediaSearch {
        query: String,
    },
    SelectMedia {
        media: String,
    },
    SetInspectorTab {
        tab: InspectorTab,
    },
    SetWorkspaceTab {
        tab: WorkspaceTab,
    },
    SetSectionExpanded {
        section: String,
        expanded: bool,
    },
    ResetSection {
        section: String,
    },
    SetProperty {
        property: String,
        value: Raw,
    },
    ResetProperty {
        property: String,
    },
    SetTimelineZoom {
        zoom: Raw,
    },
    SetSnap {
        snap: bool,
    },
    SetPlayhead {
        frame: Raw,
    },
    SelectTrack {
        track: String,
    },
    SetTrackFlag {
        track: String,
        flag: TrackFlag,
        value: bool,
    },
    SelectClip {
        clip: String,
    },
    MoveClip {
        clip: String,
        track: Option<String>,
        frame: Option<i64>,
        snap: Option<bool>,
    },
    TrimClip {
        clip: String,
        edge: timeline::TrimEdge,
        frame: i64,
    },
    ReorderTrack {
        track: String,
        anchor: String,
        placement: Placement,
    },
    /// Creates a track. With `id: None` the core generates `<kind>-<n>`; batch callers pass IDs.
    CreateTrack {
        id: Option<String>,
        name: String,
        kind: TrackKind,
    },
    /// Registers an already probed asset in the library.
    AddAsset {
        id: Option<String>,
        name: String,
        kind: MediaKind,
        asset: Box<Asset>,
        folder: Option<String>,
    },
    /// Points an existing library item at other bytes (relink, hash refresh). Clips keep their
    /// ID and overrides; the new asset must still cover every clip that reads from it.
    ReplaceAsset {
        asset_id: String,
        asset: Box<Asset>,
    },
    /// Creates a clip from explicit IDs; selection is not required nor consulted.
    AddClip {
        id: Option<String>,
        name: Option<String>,
        track: String,
        asset: Option<String>,
        start: i64,
        duration: i64,
        source_start: Option<Ratio>,
        text: Option<TextSpec>,
        camera: Option<Camera>,
        /// Inspector values applied together with the creation (all or nothing).
        properties: Vec<(String, Raw)>,
    },
    /// Same as `SetProperty`, with an explicit clip instead of the selection.
    SetClipProperty {
        clip: String,
        property: String,
        value: Raw,
    },
    SetClipText {
        clip: String,
        text: TextSpec,
    },
    /// Edits one field (`content`, `size` or `color`) of a title's text, keeping the others. The
    /// whole new text is validated and normalized (NFC) before anything is stored.
    SetClipTextField {
        clip: String,
        field: String,
        value: Raw,
    },
    SetClipCamera {
        clip: String,
        camera: Camera,
    },
    /// Puts the camera of the splat's capture (BL143) back on a scene clip of that splat.
    SetClipCameraFromCapture {
        clip: String,
    },
    SetSequenceLength {
        frames: i64,
    },
    SetPreview {
        intent: PreviewIntent,
    },
    /// Changes the preview quality policy; `None` fields keep their value.
    SetPreviewPolicy {
        mode: Option<PreviewMode>,
        max_height: Option<u32>,
        format: Option<ProxyFormat>,
    },
    Undo,
    Redo,
    /// Saves a user export preset in the project (replaces one with the same ID).
    SavePreset {
        preset: Value,
    },
    DeletePreset {
        id: String,
    },
    /// Selects the preset the export dialog starts from; built-in or user.
    SelectPreset {
        id: String,
    },
    /// Creates a sequence; without `settings` it copies the format of the active one.
    AddSequence {
        id: Option<String>,
        name: Option<String>,
        settings: Option<SequenceSettings>,
        activate: bool,
    },
    ActivateSequence {
        sequence: String,
    },
    RemoveSequence {
        sequence: String,
    },
    RenameSequence {
        sequence: String,
        name: String,
    },
    /// A `clip.camera_key*` edit of a scene clip's camera keyframes (BL140, see `camera_edit`).
    CameraKeys(camera_edit::CameraKeysCommand),
    /// A `scene.*` operation on the directed scenes of the project (see `scene_project`).
    Scene(SceneCommand),
    /// One slider drag as a single undoable edit: begin, update (live preview), then commit
    /// or cancel. Not an edit by itself (no history label): the entry is recorded at commit.
    PropertyGesture(property_gesture::GestureCommand),
}

impl Command {
    /// Name under which an edit appears in the undo list; `None` for presentation commands
    /// (selection, zoom, tabs, playhead, preview) that are not part of the document.
    pub fn history_label(&self) -> Option<&'static str> {
        Some(match self {
            Self::SetProperty { .. } | Self::SetClipProperty { .. } => "Set property",
            Self::ResetProperty { .. } | Self::ResetSection { .. } => "Reset property",
            Self::SetTrackFlag { .. } => "Track flag",
            Self::MoveClip { .. } => "Move clip",
            Self::TrimClip { .. } => "Trim clip",
            Self::ReorderTrack { .. } => "Reorder track",
            Self::CreateTrack { .. } => "Add track",
            Self::AddAsset { .. } => "Add asset",
            Self::ReplaceAsset { .. } => "Relink asset",
            Self::AddClip { .. } => "Add clip",
            Self::SetClipText { .. } | Self::SetClipTextField { .. } => "Edit text",
            Self::SetClipCamera { .. } => "Edit camera",
            Self::SetClipCameraFromCapture { .. } => "Use capture camera",
            Self::CameraKeys(_) => "Edit camera keys",
            Self::SetSequenceLength { .. } => "Sequence length",
            Self::AddSequence { .. } => "Add sequence",
            Self::ActivateSequence { .. } => return None,
            Self::RemoveSequence { .. } => "Remove sequence",
            Self::RenameSequence { .. } => "Rename sequence",
            Self::SavePreset { .. } | Self::DeletePreset { .. } => "Export preset",
            Self::SelectPreset { .. } => "Select export preset",
            Self::Scene(scene) => return scene.history_label(),
            _ => return None,
        })
    }
}

impl Default for EditorState {
    /// The demonstration project. Prefer [`EditorState::demo`] so the bootstrap is explicit.
    fn default() -> Self {
        Self::demo()
    }
}

/// Settings of a new project.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectSettings {
    pub name: String,
    pub sequence: SequenceSettings,
    /// Folder for managed media, caches and recovery data. `None` until the first save.
    pub workspace: Option<std::path::PathBuf>,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            name: "Untitled".to_string(),
            sequence: SequenceSettings::default(),
            workspace: None,
        }
    }
}

fn explicit_id(raw: &str) -> Result<Id, CommandError> {
    Id::new(raw).map_err(|error| CommandError::new("invalid_id", error.to_string()).target(raw))
}

impl EditorState {
    /// Explicit bootstrap of the illustrative "North expedition" project.
    pub fn demo() -> Self {
        Self {
            project_name: "North expedition".to_string(),
            demonstration: true,
            library: Library::demo(),
            inspector: Inspector::default(),
            timeline: Timeline::demo(),
            sequences: Vec::new(),
            project: ProjectMeta::new(),
            history: History::default(),
            workspace_tab: WorkspaceTab::Edit,
            selected_media: Some(Id::lit("ao02")),
            subject: Subject::Clip,
            preview: PreviewIntent::Sequence,
            preview_policy: PreviewPolicy::default(),
            scenes: SceneBook::default(),
            scene_interaction: SceneInteraction::default(),
            thumbnails: Default::default(),
        }
    }

    /// A real, empty project: no assets, tracks or clips.
    pub fn empty(settings: &ProjectSettings) -> Result<Self, CommandError> {
        settings.sequence.validate()?;
        let name = settings.name.trim();
        if name.is_empty() {
            return Err(CommandError::invalid_value(
                "A project needs a name",
                json!({}),
            ));
        }
        let mut project = ProjectMeta::new();
        project.workspace = settings.workspace.clone();
        Ok(Self {
            project_name: name.to_string(),
            demonstration: false,
            library: Library::empty(name),
            inspector: Inspector::default(),
            timeline: Timeline::empty(&settings.sequence),
            sequences: Vec::new(),
            project,
            history: History::default(),
            workspace_tab: WorkspaceTab::Edit,
            selected_media: None,
            subject: Subject::Media,
            preview: PreviewIntent::Sequence,
            preview_policy: PreviewPolicy::default(),
            scenes: SceneBook::default(),
            scene_interaction: SceneInteraction::default(),
            thumbnails: Default::default(),
        })
    }
}

impl EditorState {
    pub fn selected_clip(&self) -> Option<&Clip> {
        self.timeline
            .selected_clip
            .and_then(|id| self.timeline.clip(id))
    }

    pub fn subject_media(&self) -> Option<&MediaItem> {
        let id = match self.subject {
            Subject::Clip => self.selected_clip().and_then(|clip| clip.media),
            Subject::Media => self.selected_media,
        };
        id.and_then(|id| self.library.item(id))
    }

    /// The media a source preview would show: what the monitor describes, if it has a file
    /// or generated content behind it (a sequence cannot be previewed as a source).
    pub fn previewable_media(&self) -> Option<&MediaItem> {
        self.subject_media()
            .filter(|item| item.kind != crate::assets::MediaKind::Timeline && item.asset.is_some())
    }

    /// The clip the inspector edits: the timeline selection, whatever the
    /// monitor currently describes.
    pub fn inspector_clip(&self) -> Option<&Clip> {
        self.selected_clip()
    }

    fn require_clip(&self) -> Result<(Id, ClipKind), CommandError> {
        self.selected_clip()
            .map(|clip| (clip.id, clip.kind))
            .ok_or_else(|| CommandError::new("no_selection", "Select a clip in the timeline first"))
    }

    /// Stores a validated text on a title clip; the same rule for `clip.text`, `set clip_text:` and the
    /// inspector input.
    fn apply_clip_text(&mut self, clip: &str, text: TextSpec) -> Result<Value, CommandError> {
        text.validate()?;
        let (id, kind) = self.clip_ref(clip)?;
        if kind != ClipKind::Title {
            return Err(
                CommandError::new("not_applicable", "Only title clips carry text")
                    .target(format!("clip:{id}")),
            );
        }
        let entry = self.timeline.clips.iter_mut().find(|entry| entry.id == id);
        let Some(entry) = entry else {
            return Err(CommandError::unknown_element("clip", clip));
        };
        let changed = entry.text.as_ref() != Some(&text);
        entry.text = Some(text.clone());
        Ok(json!({ "target": format!("clip:{id}"), "field": "text",
            "value": text.to_json(), "changed": changed }))
    }

    /// The SHARP capture of the splat a scene clip shows (BL143), if there is one.
    pub fn capture_of_clip(&self, id: Id) -> Option<crate::sharp::Capture> {
        let clip = self.timeline.clip(id.as_str())?;
        if clip.kind != ClipKind::Scene {
            return None;
        }
        let item = self.library.item(clip.media?)?;
        // Only a splat has a capture camera, whatever an asset description claims.
        if item.kind != MediaKind::Splat {
            return None;
        }
        item.asset.as_ref()?.meta.capture
    }

    fn clip_ref(&self, id: &str) -> Result<(Id, ClipKind), CommandError> {
        self.timeline
            .clip(id)
            .map(|clip| (clip.id, clip.kind))
            .ok_or_else(|| CommandError::unknown_element("clip", id))
    }

    pub fn status_summary(&self) -> String {
        let timeline = &self.timeline;
        format!(
            "{:.1} seconds · {} clips · {} × {} · {} fps · Rec.709",
            timeline.duration_seconds(),
            timeline.clips.len(),
            timeline.width,
            timeline.height,
            timeline.rate.nominal_fps()
        )
    }

    /// Compact live summary for `ui status`.
    pub fn summary(&self) -> Value {
        let timeline = &self.timeline;
        let mut summary = json!({
            "demonstration": self.demonstration,
            "project": self.project_name,
            "header": self.header().to_json(),
            // What an editor needs to show "saved / unsaved" without reading the files.
            "persistence": {
                "document": self.project.document,
                "dirty": self.project.is_dirty(),
                "revision": self.project.revision,
                "saved_revision": self.project.saved_revision,
            },
            "single_monitor": true,
            "preview": self.preview.to_json(),
            "assets": self.library.items.iter().map(|item| format!("media:{}", item.id)).collect::<Vec<_>>(),
            "workspace": self.workspace_tab.id(),
            "inspector_tab": self.inspector.tab.id(),
            "selection": {
                "clip": self.selected_clip().map(|clip| format!("clip:{}", clip.id)),
                "track": timeline.selected_track.map(|track| format!("track:{track}")),
                "media": self.selected_media.map(|media| format!("media:{media}")),
                "folder": self.library.selected_folder.map(|folder| format!("folder:{folder}")),
                "monitor": self.subject_media().map(|item| format!("media:{}", item.id)),
            },
            "search": { "value": self.library.search, "results": self.library.results().len() },
            "playhead": { "frame": timeline.playhead,
                "timecode": timeline.timecode(timeline.playhead), "fps": timeline.fps(),
                "time": timeline.time_of(timeline.playhead).to_json() },
            "timeline": {
                "id": timeline.id, "rate": timeline.rate.to_json(),
                "width": timeline.width, "height": timeline.height,
                "zoom": timeline.zoom, "snap": timeline.snap,
                "duration_frames": timeline.length,
                "tracks": timeline.tracks.iter().map(|track| json!({
                    "id": format!("track:{}", track.id), "label": timeline.label(track.id),
                    "hidden": track.hidden, "muted": track.muted,
                })).collect::<Vec<_>>(),
                "clips": timeline.clips.iter().map(|clip| json!({
                    "id": format!("clip:{}", clip.id), "track": format!("track:{}", clip.track),
                    "start": clip.start, "duration": clip.duration,
                    "kind": clip.kind.label(),
                    "asset": clip.media.map(|media| format!("media:{media}")),
                })).collect::<Vec<_>>(),
            },
            "summary": self.status_summary(),
        });
        // Only when there is one, so the status of a project without scenes is what it always was.
        if !self.scenes.is_empty() {
            summary["scenes"] = json!(self.scenes.ids());
        }
        // BL068: the selection and gesture of the scene tools are session state, not document, but
        // a client polling the summary needs to see them (and that they went stale). Only present
        // when there is something to show.
        if let Some(interaction) = self
            .scene_interaction
            .summary_json(&self.scenes, self.project.revision)
        {
            summary["scene_interaction"] = interaction;
        }
        summary
    }

    /// Runs one command. Edits (see [`Command::history_label`]) become one undo entry and bump
    /// the project revision when they changed something; a rejected command changes nothing.
    pub fn execute(&mut self, command: Command) -> Result<Value, CommandError> {
        // Gesture phases manage their own history entry (see `property_gesture`).
        let command = match command {
            Command::PropertyGesture(gesture) => return self.run_property_gesture(gesture),
            other => other,
        };
        // An edit never runs on top of a slider gesture that is still previewing.
        if command.settles_property_gesture() && !self.is_idle_gizmo_release(&command) {
            self.settle_property_gesture();
        }
        let Some(label) = command.history_label() else {
            return self.apply(command);
        };
        // F061: the release of an effect handle's drag is an effect edit, not a scene edit.
        let label = if self.is_fx_handle_release(&command) {
            "Edit effects"
        } else {
            label
        };
        let before = self.snapshot();
        let outcome = self.apply(command)?;
        if outcome["changed"] != false {
            self.record_edit(label, before);
        }
        Ok(outcome)
    }

    /// A `scene.pointer.up` while no gizmo drag is open (a script, a release the page sends late)
    /// writes nothing, so it is not an edit that has to settle a slider gesture somebody else
    /// opened: it never commits it (S1 of the PR #25 review). With a *live* drag open it is the
    /// commit of that drag and settles like every edit. "Live" is the criterion `pointer_up`
    /// itself uses ([`SceneInteraction::live_drag`]): a drag whose gesture another command
    /// settled (`scene.gesture.cancel`) is stored but dead, `pointer_up` answers `noop` for it,
    /// so it must not settle the slider gesture either.
    fn is_idle_gizmo_release(&self, command: &Command) -> bool {
        matches!(
            command,
            Command::Scene(SceneCommand::Gizmo(GizmoCommand::Pointer(
                PointerCommand::Up { .. }
            )))
        ) && self.scene_interaction.live_drag().is_none()
    }

    /// The funnel of every command: whatever it did to the scenes, an effect target the project no
    /// longer holds is dropped before anything reads it (also after an error, which may have moved
    /// the project halfway).
    fn apply(&mut self, command: Command) -> Result<Value, CommandError> {
        let outcome = self.apply_command(command);
        self.prune_fx_target();
        outcome
    }

    fn apply_command(&mut self, command: Command) -> Result<Value, CommandError> {
        match command {
            Command::SetFolderExpanded { folder, expanded } => {
                self.library.set_expanded(&folder, expanded)
            }
            Command::SelectFolder { folder } => self.library.select_folder(folder.as_deref()),
            Command::SetMediaSearch { query } => Ok(self.library.set_search(&query)),
            Command::SelectMedia { media } => {
                let item = self
                    .library
                    .item(&media)
                    .ok_or_else(|| CommandError::unknown_element("media", &media))?;
                let id = item.id;
                let previous = self.selected_media.replace(id);
                self.subject = Subject::Media;
                Ok(
                    json!({ "target": format!("media:{media}"), "action": "select",
                    "monitor": id, "changed": previous != Some(id) }),
                )
            }
            Command::SetInspectorTab { tab } => Ok(self.inspector.set_tab(tab)),
            Command::SetWorkspaceTab { tab } => {
                let previous = std::mem::replace(&mut self.workspace_tab, tab);
                Ok(
                    json!({ "target": "workspace.tabs", "field": "value", "value": tab.id(),
                    "previous": previous.id(), "changed": previous != tab,
                    "implemented": tab == WorkspaceTab::Edit }),
                )
            }
            Command::SetSectionExpanded { section, expanded } => {
                self.inspector.set_expanded(&section, expanded)
            }
            Command::ResetSection { section } => {
                let (clip, kind) = self.require_clip()?;
                if section == "camera" && kind == ClipKind::Scene {
                    // The camera section has no sliders: resetting it removes the keyframes.
                    return self.apply_camera_keys(camera_edit::CameraKeysCommand::Replace {
                        clip: clip.to_string(),
                        keys: Vec::new(),
                    });
                }
                self.inspector.reset_section(clip, kind, &section)
            }
            Command::SetProperty { property, value } => {
                let (clip, kind) = self.require_clip()?;
                self.inspector.set_property(clip, kind, &property, &value)
            }
            Command::ResetProperty { property } => {
                let (clip, kind) = self.require_clip()?;
                self.inspector.reset_property(clip, kind, &property)
            }
            Command::SetTimelineZoom { zoom } => Ok(self.timeline.set_zoom(zoom.number()?)),
            Command::SetSnap { snap } => Ok(self.timeline.set_snap(snap)),
            Command::SetPlayhead { frame } => Ok(self.timeline.set_playhead(frame.integer()?)),
            Command::SelectTrack { track } => self.timeline.select_track(&track),
            Command::SetTrackFlag { track, flag, value } => {
                self.timeline.set_track_flag(&track, flag, value)
            }
            Command::SelectClip { clip } => {
                let outcome = self.timeline.select_clip(&clip)?;
                if let Some(media) = self.timeline.clip(&clip).and_then(|clip| clip.media) {
                    self.selected_media = Some(media);
                }
                self.subject = Subject::Clip;
                Ok(outcome)
            }
            Command::MoveClip {
                clip,
                track,
                frame,
                snap,
            } => {
                let outcome = self
                    .timeline
                    .move_clip(&clip, track.as_deref(), frame, snap)?;
                self.subject = Subject::Clip;
                Ok(outcome)
            }
            Command::TrimClip { clip, edge, frame } => {
                // The head shift of a trim is counted in source time, which runs at the speed.
                let speed = self
                    .timeline
                    .clip(&clip)
                    .map_or(Ratio::from_int(1), |entry| self.clip_speed(entry));
                let outcome = self.timeline.trim_clip(&clip, edge, frame, speed)?;
                self.subject = Subject::Clip;
                Ok(outcome)
            }
            Command::ReorderTrack {
                track,
                anchor,
                placement,
            } => self.timeline.reorder_track(&track, &anchor, placement),
            Command::CreateTrack { id, name, kind } => {
                let id = match id {
                    Some(raw) => explicit_id(&raw)?,
                    None => self.timeline.fresh_id(kind.label()),
                };
                self.timeline.add_track(id, &name, kind)
            }
            Command::AddAsset {
                id,
                name,
                kind,
                asset,
                folder,
            } => self.add_asset(id, &name, kind, *asset, folder.as_deref()),
            Command::ReplaceAsset { asset_id, asset } => self.replace_asset(&asset_id, *asset),
            Command::AddClip {
                id,
                name,
                track,
                asset,
                start,
                duration,
                source_start,
                text,
                camera,
                properties,
            } => {
                // WHY: creation and its properties are one command; a rejected property must
                // not leave a half-configured clip behind.
                let backup = self.clone();
                let outcome = self.add_clip(AddClip {
                    id,
                    name,
                    track,
                    asset,
                    start,
                    duration,
                    source_start,
                    text,
                    camera,
                    properties,
                });
                if outcome.is_err() {
                    *self = backup;
                }
                outcome
            }
            Command::SetClipProperty {
                clip,
                property,
                value,
            } => {
                let (clip, kind) = self.clip_ref(&clip)?;
                self.inspector.set_property(clip, kind, &property, &value)
            }
            Command::SetClipText { clip, text } => self.apply_clip_text(&clip, text),
            Command::SetClipTextField { clip, field, value } => {
                let (id, kind) = self.clip_ref(&clip)?;
                let current = self
                    .timeline
                    .clip(&clip)
                    .and_then(|entry| entry.text.clone())
                    .filter(|_| kind == ClipKind::Title)
                    .ok_or_else(|| {
                        CommandError::new("not_applicable", "Only title clips carry text")
                            .target(format!("clip:{id}"))
                    })?;
                let text = match field.as_str() {
                    "content" => TextSpec::new(value.text(), current.size, current.color)?,
                    "size" => TextSpec::new(current.content, value.number()?, current.color)?,
                    "color" => TextSpec::new(current.content, current.size, value.text())?,
                    _ => {
                        return Err(CommandError::new(
                            "unknown_field",
                            format!("A title's text has no field `{field}`"),
                        )
                        .target(format!("clip_text:{id}"))
                        .details(json!({ "available": ["content", "size", "color"] })));
                    }
                };
                self.apply_clip_text(&clip, text)
            }
            Command::SetClipCamera { clip, camera } => {
                camera.validate()?;
                let (id, kind) = self.clip_ref(&clip)?;
                if kind != ClipKind::Scene {
                    return Err(CommandError::new(
                        "not_applicable",
                        "Only scene clips carry a camera",
                    )
                    .target(format!("clip:{id}")));
                }
                let entry = self.timeline.clips.iter_mut().find(|entry| entry.id == id);
                let Some(entry) = entry else {
                    return Err(CommandError::unknown_element("clip", &clip));
                };
                if camera.orbit != 0.0 && !entry.camera_keys.is_empty() {
                    return Err(CommandError::new(
                        "camera_orbit_conflict",
                        "The clip has camera keys, which replace the orbit: clear them first",
                    )
                    .target(format!("clip:{id}")));
                }
                // WHY: with keys the clip camera is never drawn (the keys are the camera), so a
                // different `clip.camera` would be an undo entry and `changed: true` that alter
                // nothing visible. Refuse it; edit the keys, or clear them to use this camera.
                if !entry.camera_keys.is_empty() && entry.camera != Some(camera) {
                    return Err(CommandError::new(
                        "camera_keys_override",
                        "The clip has camera keys, which replace its camera: edit the keys or clear them first",
                    )
                    .target(format!("clip:{id}")));
                }
                let changed = entry.camera != Some(camera);
                entry.camera = Some(camera);
                Ok(json!({ "target": format!("clip:{id}"), "field": "camera",
                    "value": camera.to_json(), "changed": changed }))
            }
            Command::SetClipCameraFromCapture { clip } => {
                let (id, kind) = self.clip_ref(&clip)?;
                let target = format!("clip:{id}");
                let capture = self.capture_of_clip(id).ok_or_else(|| {
                    CommandError::new(
                        "no_capture",
                        if kind == ClipKind::Scene {
                            "The clip's asset has no SHARP capture camera (a plain splat, a scene \
                                 file or an import with `capture: ignore`)"
                        } else {
                            "Only scene clips of a SHARP splat carry a capture camera"
                        },
                    )
                    .target(target.clone())
                })?;
                let camera = capture.camera();
                let Some(entry) = self.timeline.clips.iter_mut().find(|entry| entry.id == id)
                else {
                    return Err(CommandError::unknown_element("clip", &clip));
                };
                // The capture camera has no orbit, so this is not an orbit conflict: with keys the
                // clip camera is never drawn, and applying it would be an inert undo entry
                // (the same refusal `clip.camera` gives, BL140.7).
                if !entry.camera_keys.is_empty() && entry.camera != Some(camera) {
                    return Err(CommandError::new(
                        "camera_keys_override",
                        "The clip has camera keys, which replace its camera: clear them first",
                    )
                    .target(target));
                }
                let changed = entry.camera != Some(camera);
                entry.camera = Some(camera);
                Ok(
                    json!({ "target": target, "field": "camera", "source": "capture",
                    "value": camera.to_json(), "changed": changed }),
                )
            }
            Command::CameraKeys(command) => self.apply_camera_keys(command),
            Command::SetSequenceLength { frames } => self.timeline.set_length(frames),
            Command::SavePreset { preset } => self.save_preset(&preset),
            Command::DeletePreset { id } => self.delete_preset(&id),
            Command::SelectPreset { id } => self.select_preset(&id),
            Command::Undo => self.undo(),
            Command::Redo => self.redo(),
            Command::AddSequence {
                id,
                name,
                settings,
                activate,
            } => self.add_sequence(id, name, settings, activate),
            Command::ActivateSequence { sequence } => self.activate_sequence(&sequence),
            Command::RemoveSequence { sequence } => self.remove_sequence(&sequence),
            Command::RenameSequence { sequence, name } => self.rename_sequence(&sequence, &name),
            Command::Scene(command) => self.apply_scene(command),
            Command::PropertyGesture(gesture) => self.run_property_gesture(gesture),
            Command::SetPreviewPolicy {
                mode,
                max_height,
                format,
            } => {
                let mut next = self.preview_policy;
                next.mode = mode.unwrap_or(next.mode);
                next.format = format.unwrap_or(next.format);
                if let Some(height) = max_height {
                    if !(crate::preview::MIN_HEIGHT..=4320).contains(&height) {
                        return Err(CommandError::invalid_value(
                            "A proxy height is between 144 and 4320 pixels",
                            json!({ "min": crate::preview::MIN_HEIGHT, "max": 4320 }),
                        ));
                    }
                    next.max_height = height;
                }
                let changed = next != self.preview_policy;
                self.preview_policy = next;
                Ok(
                    json!({ "target": "monitor.preview_policy", "field": "value",
                    "value": next.to_json(), "changed": changed }),
                )
            }
            Command::SetPreview { intent } => {
                if let PreviewIntent::Source { asset, time } = &intent {
                    let item = self
                        .library
                        .item(asset)
                        .ok_or_else(|| CommandError::unknown_element("media", asset))?;
                    if item.kind == MediaKind::Timeline {
                        return Err(CommandError::new(
                            "invalid_preview",
                            "A sequence is previewed with the `sequence` intent",
                        )
                        .target(asset.as_str()));
                    }
                    if time.is_negative() {
                        return Err(CommandError::invalid_value(
                            "Source time cannot be negative",
                            json!({ "min": 0 }),
                        ));
                    }
                }
                let changed = self.preview != intent;
                self.preview = intent;
                Ok(json!({ "target": "monitor.preview", "field": "value",
                    "value": self.preview.to_json(), "changed": changed }))
            }
        }
    }
}

/// Arguments of [`Command::AddClip`], grouped so the handler stays readable.
struct AddClip {
    id: Option<String>,
    name: Option<String>,
    track: String,
    asset: Option<String>,
    start: i64,
    duration: i64,
    source_start: Option<Ratio>,
    text: Option<TextSpec>,
    camera: Option<Camera>,
    properties: Vec<(String, Raw)>,
}

impl EditorState {
    fn add_asset(
        &mut self,
        id: Option<String>,
        name: &str,
        kind: MediaKind,
        asset: Asset,
        folder: Option<&str>,
    ) -> Result<Value, CommandError> {
        if !MediaKind::ASSETS.contains(&kind) {
            return Err(CommandError::new(
                "invalid_asset",
                format!("`{}` is not an asset kind", kind.label()),
            ));
        }
        let name = name.trim();
        if name.is_empty() {
            return Err(CommandError::invalid_value(
                "An asset needs a name",
                json!({}),
            ));
        }
        let id = match id {
            Some(raw) => explicit_id(&raw)?,
            None => (1..)
                .filter_map(|n| Id::new(&format!("{}-{n}", kind.label())).ok())
                .find(|id| self.library.item(id).is_none())
                .unwrap_or_else(|| Id::lit("asset")),
        };
        let folder =
            match folder {
                Some(raw) => explicit_id(raw)?,
                None => self.library.root().map(|folder| folder.id).ok_or_else(|| {
                    CommandError::new("unknown_element", "The library has no bin")
                })?,
            };
        // A capture camera is a property of a splat file (BL143): any other asset carrying one
        // is a corrupt description, refused before it reaches the library or a plan.
        if kind != MediaKind::Splat && asset.meta.capture.is_some() {
            return Err(CommandError::new(
                "invalid_asset",
                format!(
                    "A {} asset cannot carry a SHARP capture camera: only splats do",
                    kind.label()
                ),
            )
            .target(id.as_str()));
        }
        // WHY: a scene that names a missing splat would fail only at render time; failing
        // at registration keeps the project free of dangling references.
        for required in &asset.meta.requires {
            let satisfied = self
                .library
                .item(required)
                .is_some_and(|item| item.kind == MediaKind::Splat);
            if !satisfied {
                return Err(CommandError::new(
                    "missing_dependency",
                    format!("Scene needs splat asset `{required}`; register it first"),
                )
                .target(id.as_str()));
            }
        }
        let ordinal = self
            .library
            .items
            .iter()
            .filter(|item| item.kind == kind)
            .count()
            + 1;
        let prefix = match kind {
            MediaKind::Still => "IMG",
            MediaKind::Video => "VID",
            MediaKind::Audio => "AUD",
            MediaKind::Text => "TXT",
            MediaKind::Scene => "SCN",
            MediaKind::Splat => "SPL",
            MediaKind::Timeline => "TL",
        };
        let item = MediaItem {
            id,
            code: format!("{prefix}{ordinal:02}"),
            name: name.to_string(),
            meta: asset_summary(kind, &asset),
            kind,
            folder,
            image: None,
            asset: Some(asset),
        };
        let description = item.asset.as_ref().map(|asset| asset.to_json(kind));
        let warnings: Vec<Value> = item
            .asset
            .as_ref()
            .and_then(|asset| asset.meta.capture)
            .map(|capture| {
                capture
                    .warnings()
                    .into_iter()
                    .map(|(code, message)| json!({ "code": code, "message": message }))
                    .collect()
            })
            .unwrap_or_default();
        self.library.add_item(item)?;
        self.selected_media = Some(id);
        let mut outcome = json!({ "target": format!("media:{id}"), "action": "create",
            "kind": kind.label(), "folder": format!("folder:{folder}"), "asset": description,
            "changed": true });
        if !warnings.is_empty() {
            outcome["warnings"] = Value::Array(warnings);
        }
        Ok(outcome)
    }

    fn replace_asset(&mut self, id: &str, asset: Asset) -> Result<Value, CommandError> {
        let item = self
            .library
            .item(id)
            .ok_or_else(|| CommandError::unknown_element("media", id))?;
        let kind = item.kind;
        if kind != MediaKind::Splat && asset.meta.capture.is_some() {
            return Err(CommandError::new(
                "invalid_asset",
                "Only splat assets can carry a SHARP capture camera",
            )
            .target(id));
        }
        let old = item.asset.as_ref().ok_or_else(|| {
            CommandError::new(
                "asset_unavailable",
                format!("`{id}` has no media behind it"),
            )
            .target(id)
        })?;
        if !MediaKind::ASSETS.contains(&kind) {
            return Err(CommandError::new(
                "invalid_asset",
                "This item has no replaceable media",
            ));
        }
        // WHY: relinking must not silently break edits; a shorter source would leave clips
        // reading past its end.
        if let Some(duration) = asset.meta.duration {
            for timeline in self.all_sequences() {
                for clip in timeline
                    .clips
                    .iter()
                    .filter(|clip| clip.media == Some(item.id))
                {
                    let span = timeline.rate.frame_to_time(clip.duration.max(0));
                    let needed = clip.source_start.add(span).unwrap_or(duration);
                    if needed > duration {
                        return Err(CommandError::new(
                            "source_out_of_range",
                            format!("Clip `{}` reads up to {needed} s but the new media lasts {duration} s", clip.id),
                        )
                        .target(format!("clip:{}", clip.id)));
                    }
                }
            }
        }
        let changed = *old != asset;
        let summary = asset_summary(kind, &asset);
        let described = asset.to_json(kind);
        let Some(item) = self.library.items.iter_mut().find(|item| item.id == id) else {
            return Err(CommandError::unknown_element("media", id));
        };
        item.meta = summary;
        item.asset = Some(asset);
        Ok(
            json!({ "target": format!("media:{id}"), "action": "replace", "asset": described,
            "changed": changed }),
        )
    }

    /// Stores (or clears) the preview proxy of an asset's decoded frames. A proxy that does not
    /// derive from the frames currently attached is refused: it would show another picture.
    pub fn set_preview_proxy(
        &mut self,
        id: &str,
        proxy: Option<crate::assets::ProxyFrames>,
    ) -> bool {
        let Some(decoded) = self
            .library
            .items
            .iter_mut()
            .find(|item| item.id == id)
            .and_then(|item| item.asset.as_mut())
            .and_then(|asset| asset.decoded.as_mut())
        else {
            return false;
        };
        let derived_from_current = proxy.as_ref().is_none_or(|proxy| {
            decoded
                .frames
                .as_ref()
                .is_some_and(|frames| frames.pattern == proxy.source_frames)
        });
        if !derived_from_current {
            return false;
        }
        decoded.proxy = proxy;
        true
    }

    /// Stores (or clears) the derived media of an asset. Not an edit: the cache is disposable and
    /// never persisted, so it does not touch history or the revision.
    pub fn set_decoded(&mut self, id: &str, decoded: Option<crate::assets::Decoded>) -> bool {
        let Some(asset) = self
            .library
            .items
            .iter_mut()
            .find(|item| item.id == id)
            .and_then(|item| item.asset.as_mut())
        else {
            return false;
        };
        // WHY keep the proxy: re-preparing a video that resolves to the same frames must not
        // throw away the (slow to build) proxy derived from them.
        let kept = asset
            .decoded
            .as_mut()
            .and_then(|current| current.proxy.take());
        asset.decoded = decoded.map(|mut next| {
            if next.proxy.is_none()
                && let Some(proxy) = kept
                && next
                    .frames
                    .as_ref()
                    .is_some_and(|frames| frames.pattern == proxy.source_frames)
            {
                next.proxy = Some(proxy);
            }
            next
        });
        true
    }

    fn add_clip(&mut self, args: AddClip) -> Result<Value, CommandError> {
        let id = match args.id {
            Some(raw) => explicit_id(&raw)?,
            None => self.fresh_clip_id("clip"),
        };
        // WHY: clip properties are keyed by clip ID, so an ID must be unique across sequences.
        if self.clip_id_used_in_other_sequence(&id) {
            return Err(CommandError::new(
                "duplicate_id",
                format!("`{id}` is already a clip of another sequence"),
            )
            .target(id.as_str()));
        }
        let media = match &args.asset {
            Some(raw) => Some(
                self.library
                    .item(raw)
                    .ok_or_else(|| CommandError::unknown_element("media", raw))?,
            ),
            None => None,
        };
        let kind = match media.map(|item| item.kind) {
            Some(MediaKind::Still | MediaKind::Video) => ClipKind::Video,
            Some(MediaKind::Scene | MediaKind::Splat) => ClipKind::Scene,
            Some(MediaKind::Audio) => ClipKind::Audio,
            Some(MediaKind::Text) => ClipKind::Title,
            Some(MediaKind::Timeline) => {
                return Err(CommandError::new(
                    "invalid_asset",
                    "A sequence cannot be placed on a track",
                ));
            }
            None if args.text.is_some() => ClipKind::Title,
            None => {
                return Err(CommandError::new(
                    "missing_argument",
                    "A clip needs an `asset` or, for a title, `text`",
                ));
            }
        };
        if let Some(item) = media
            && item.asset.is_none()
        {
            return Err(CommandError::new(
                "asset_unavailable",
                format!("`{}` has no media behind it (demonstration entry)", item.id),
            )
            .target(item.id.as_str()));
        }
        let source_start = args.source_start.unwrap_or(Ratio::ZERO);
        if source_start.is_negative() {
            return Err(CommandError::invalid_value(
                "`source_start` cannot be negative",
                json!({ "min": 0 }),
            ));
        }
        if let Some(duration) = media
            .and_then(|item| item.asset.as_ref())
            .and_then(|asset| asset.meta.duration)
            && matches!(kind, ClipKind::Video | ClipKind::Audio)
        {
            let span = self.timeline.rate.frame_to_time(args.duration.max(0));
            let needed = source_start.add(span).unwrap_or(duration);
            if needed > duration {
                return Err(CommandError::new(
                    "source_out_of_range",
                    format!("The clip reads up to {needed} s of a {duration} s source"),
                )
                .target(format!("clip:{id}"))
                .details(
                    json!({ "source_duration": duration.to_json(), "needed": needed.to_json() }),
                ));
            }
        }
        let text = match (kind, args.text) {
            (ClipKind::Title, Some(text)) => Some(text),
            (ClipKind::Title, None) => media
                .and_then(|item| item.asset.as_ref())
                .and_then(|asset| asset.meta.text.clone()),
            (_, Some(_)) => {
                return Err(CommandError::new(
                    "not_applicable",
                    "Only title clips carry text",
                ));
            }
            (_, None) => None,
        };
        if kind == ClipKind::Title {
            match &text {
                Some(text) => text.validate()?,
                None => {
                    return Err(CommandError::new(
                        "missing_argument",
                        "A title clip needs `text`",
                    ));
                }
            }
        }
        let camera = match (kind, args.camera) {
            (ClipKind::Scene, camera) => {
                // WHY: a SHARP splat is only right seen from the camera that took its picture;
                // the default `0,1.2,5` orbit would look at it from a place it was never captured.
                let captured = media
                    .and_then(|item| item.asset.as_ref())
                    .and_then(|asset| asset.meta.capture)
                    .map(|capture| capture.camera());
                let camera = camera.or(captured).unwrap_or_default();
                camera.validate()?;
                Some(camera)
            }
            (_, Some(_)) => {
                return Err(CommandError::new(
                    "not_applicable",
                    "Only scene clips carry a camera",
                ));
            }
            (_, None) => None,
        };
        let name = args
            .name
            .or_else(|| media.map(|item| item.name.clone()))
            .or_else(|| text.as_ref().map(|text| text.content.clone()))
            .unwrap_or_else(|| id.to_string());
        let track = self
            .timeline
            .track(&args.track)
            .map(|track| track.id)
            .ok_or_else(|| CommandError::unknown_element("track", &args.track))?;
        let media_id = media.map(|item| item.id);
        let media_kind = media.map(|item| item.kind);
        let outcome = self.timeline.add_clip(Clip {
            id,
            name,
            kind,
            track,
            start: args.start,
            duration: args.duration,
            media: media_id,
            source_start,
            text,
            camera,
            camera_keys: Vec::new(),
        });
        let mut outcome = outcome?;
        self.subject = Subject::Clip;
        // WHY: API-created stills start without the demo's Ken Burns drift, so a frame
        // rendered at a known time shows the asset exactly where it was placed.
        let still = media_kind == Some(MediaKind::Still);
        let mut properties = args.properties;
        if still && !properties.iter().any(|(name, _)| name == "ken_burns") {
            properties.insert(0, ("ken_burns".to_string(), Raw::Bool(false)));
        }
        let mut applied = Vec::new();
        for (name, value) in &properties {
            self.inspector.set_property(id, kind, name, value)?;
            applied.push(name.clone());
        }
        outcome["properties"] = json!(applied);
        Ok(outcome)
    }
}

fn asset_summary(kind: MediaKind, asset: &Asset) -> String {
    let meta = &asset.meta;
    let size = match (meta.width, meta.height) {
        (Some(width), Some(height)) => format!(" · {width} × {height}"),
        _ => String::new(),
    };
    let detail = match kind {
        MediaKind::Audio => format!(
            " · {} Hz · {} ch",
            meta.sample_rate.unwrap_or_default(),
            meta.channels.unwrap_or_default()
        ),
        MediaKind::Splat => format!(
            " · {} splats{}",
            meta.splat_count.unwrap_or_default(),
            meta.capture
                .map(|capture| format!(" · {}", capture.label()))
                .unwrap_or_default()
        ),
        MediaKind::Video => meta
            .duration
            .map(|duration| format!(" · {:.2} s", duration.to_f64()))
            .unwrap_or_default(),
        _ => String::new(),
    };
    format!("{}{size}{detail}", kind.label().to_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use inspector::property_def;

    fn text(value: &str) -> Raw {
        Raw::Text(value.into())
    }

    #[test]
    fn initial_selection_matches_the_reference_composition() {
        let state = EditorState::default();
        assert_eq!(state.selected_clip().unwrap().id, "atlantic-coast");
        assert_eq!(state.subject_media().unwrap().code, "AO02");
        assert_eq!(state.timeline.playhead, 86);
        assert_eq!(
            state.status_summary(),
            "29.0 seconds · 7 clips · 1920 × 1080 · 24 fps · Rec.709"
        );
    }

    #[test]
    fn selecting_media_feeds_the_monitor_but_not_the_clip_selection() {
        let mut state = EditorState::default();
        state
            .execute(Command::SelectMedia {
                media: "ao04".into(),
            })
            .unwrap();
        assert_eq!(state.subject_media().unwrap().code, "AO04");
        assert_eq!(state.selected_clip().unwrap().id, "atlantic-coast");
        assert!(
            state
                .execute(Command::SelectMedia {
                    media: "none".into()
                })
                .is_err()
        );
        assert_eq!(state.subject_media().unwrap().code, "AO04");
    }

    #[test]
    fn selecting_a_clip_changes_monitor_and_inspector_data() {
        let mut state = EditorState::default();
        state
            .execute(Command::SetProperty {
                property: "zoom".into(),
                value: text("150"),
            })
            .unwrap();
        state
            .execute(Command::SelectClip {
                clip: "river-detail".into(),
            })
            .unwrap();
        assert_eq!(state.subject_media().unwrap().code, "AO03");
        let zoom = property_def("zoom").unwrap();
        assert_eq!(
            state.inspector.value("river-detail", zoom),
            inspector::Value::Number(100.0)
        );
        assert_eq!(
            state.inspector.value("atlantic-coast", zoom),
            inspector::Value::Number(150.0)
        );
    }

    #[test]
    fn property_commands_need_a_selected_clip_and_a_matching_kind() {
        let mut state = EditorState::default();
        state
            .execute(Command::SelectClip {
                clip: "score-clip".into(),
            })
            .unwrap();
        let error = state
            .execute(Command::SetProperty {
                property: "zoom".into(),
                value: text("120"),
            })
            .unwrap_err();
        assert_eq!(error.code, "not_applicable");
        state
            .execute(Command::SetProperty {
                property: "volume".into(),
                value: text("-6"),
            })
            .unwrap();
        state.timeline.selected_clip = None;
        let error = state
            .execute(Command::SetProperty {
                property: "volume".into(),
                value: text("-6"),
            })
            .unwrap_err();
        assert_eq!(error.code, "no_selection");
    }

    #[test]
    fn workspace_tabs_hold_state() {
        // Panel visibility moved to the live DockArea; its port is
        // `layout::tests::panel_visibility_is_a_layout_fact`.
        let mut state = EditorState::default();
        let out = state
            .execute(Command::SetWorkspaceTab {
                tab: WorkspaceTab::Color,
            })
            .unwrap();
        assert_eq!(out["implemented"], false);
        assert_eq!(state.workspace_tab, WorkspaceTab::Color);
        assert_eq!(WorkspaceTab::parse("deliver"), Some(WorkspaceTab::Deliver));
        assert_eq!(WorkspaceTab::parse("library"), None);
    }

    #[test]
    fn failed_commands_do_not_change_state() {
        let mut state = EditorState::default();
        let before = format!("{:?}", state);
        for command in [
            Command::SelectClip {
                clip: "ghost".into(),
            },
            Command::SetFolderExpanded {
                folder: "footage".into(),
                expanded: false,
            },
            Command::SetPlayhead { frame: text("x") },
            Command::SetTimelineZoom { zoom: text("wide") },
            Command::MoveClip {
                clip: "score-clip".into(),
                track: Some("picture".into()),
                frame: None,
                snap: None,
            },
        ] {
            assert!(state.execute(command).is_err());
        }
        assert_eq!(format!("{:?}", state), before);
    }
}
