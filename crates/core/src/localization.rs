//! Sparse language overrides over the shared composition (F078).
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::document::{asset_json, parse_sequence, sequence_json};
use crate::state::group_edit::GroupCommand;
use crate::state::inspector::{Inspector, Overrides};
use crate::state::live_edit::LiveCommand;
use crate::state::timeline::{ClipKind, Timeline};
use crate::state::{Command, CommandError, EditorState};
use crate::time::Ratio;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SequencePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    length: Option<i64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    clips: BTreeMap<String, Map<String, Value>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Locale {
    #[serde(default)]
    sequences: BTreeMap<String, SequencePatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    approval: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Localization {
    source: String,
    locales: BTreeMap<String, Locale>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    Add(String),
    Select(String),
    Remove(String),
    Set {
        locale: String,
        sequence: String,
        overrides: SequencePatch,
    },
    Approve(String),
    Get {
        locale: String,
        sequence: String,
    },
    Resolve {
        locale: String,
        sequence: Option<String>,
    },
    Status,
}

#[derive(Debug, Clone)]
pub(crate) struct ProjectedSequence {
    pub(crate) timeline: Timeline,
    pub(crate) properties: Overrides,
}

impl Edit {
    pub fn history_label(&self) -> Option<&'static str> {
        match self {
            Self::Add(_) => Some("Add language"),
            Self::Remove(_) => Some("Remove language"),
            Self::Set { .. } => Some("Edit localization"),
            Self::Approve(_) => Some("Approve localization"),
            Self::Select(_) | Self::Get { .. } | Self::Resolve { .. } | Self::Status => None,
        }
    }
}

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new("invalid_localization", message)
}

fn language(raw: &str) -> Result<String, CommandError> {
    let code = raw.trim().to_ascii_lowercase();
    let mut parts = code.split('-');
    let root = parts.next().unwrap_or_default();
    if code.len() > 35
        || !(2..=8).contains(&root.len())
        || !root.bytes().all(|c| c.is_ascii_lowercase())
        || !parts.all(|part| {
            (1..=8).contains(&part.len()) && part.bytes().all(|c| c.is_ascii_alphanumeric())
        })
    {
        return Err(invalid("Use a language code such as pt-BR, en, or es"));
    }
    Ok(code)
}

impl SequencePatch {
    pub fn from_json(value: &Value) -> Result<Self, CommandError> {
        let patch: Self =
            serde_json::from_value(value.clone()).map_err(|error| invalid(error.to_string()))?;
        patch.validate()?;
        Ok(patch)
    }

    fn validate(&self) -> Result<(), CommandError> {
        for (id, fields) in &self.clips {
            if fields.is_empty() {
                return Err(invalid(format!("Clip `{id}` needs at least one override")));
            }
            for (field, value) in fields {
                let valid = match field.as_str() {
                    "start" | "duration" => value.as_i64().is_some(),
                    "name" | "media" | "group_def" => value.as_str().is_some(),
                    "shape" | "properties" => value.is_object(),
                    "keys" | "camera_keys" => value.is_array(),
                    "title_style" => value.as_object().is_some_and(|style| {
                        style.keys().all(|field| {
                            field != "content"
                                && (field == "width"
                                    || crate::content::TextSpec::FIELDS.contains(&field.as_str()))
                        })
                    }),
                    "source_start" => Ratio::from_json(value).is_some_and(|n| !n.is_negative()),
                    "text" | "audio" => value.is_string() || value.is_null(),
                    "group_texts" => value.as_object().is_some_and(|texts| {
                        texts
                            .values()
                            .all(|text| text.is_string() || text.is_null())
                    }),
                    "cues" => value.is_array() || value.is_null(),
                    _ => false,
                };
                if !valid {
                    return Err(invalid(format!(
                        "Clip `{id}` has an invalid `{field}` override"
                    )));
                }
            }
        }
        Ok(())
    }

    fn is_empty(&self) -> bool {
        self.length.is_none() && self.clips.is_empty()
    }
}

impl Localization {
    pub(crate) fn from_json(value: &Value) -> Result<Self, CommandError> {
        let book: Self =
            serde_json::from_value(value.clone()).map_err(|error| invalid(error.to_string()))?;
        if language(&book.source)? != book.source || book.locales.contains_key(&book.source) {
            return Err(invalid("The source language needs its own unique code"));
        }
        for (code, locale) in &book.locales {
            if language(code)? != *code {
                return Err(invalid(format!("Language code `{code}` must be lowercase")));
            }
            for patch in locale.sequences.values() {
                patch.validate()?;
            }
        }
        Ok(book)
    }

    pub(crate) fn to_json(&self) -> Value {
        json!(self)
    }

    pub fn codes(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.source.as_str()).chain(self.locales.keys().map(String::as_str))
    }

    fn locale(&self, code: &str) -> Result<&Locale, CommandError> {
        self.locales
            .get(code)
            .ok_or_else(|| CommandError::unknown_element("language", code))
    }
}

impl EditorState {
    pub fn base_sequences(&self) -> impl Iterator<Item = &Timeline> {
        std::iter::once(self.base_timeline()).chain(self.sequences.iter())
    }

    pub(crate) fn base_timeline(&self) -> &Timeline {
        self.localization_base
            .as_ref()
            .map_or(&self.timeline, |base| &base.timeline)
    }

    pub(crate) fn base_properties(&self) -> Overrides {
        self.localization_base.as_ref().map_or_else(
            || self.inspector.overrides_snapshot(),
            |base| base.properties.clone(),
        )
    }

    pub fn language_codes(&self) -> Vec<&str> {
        self.localization
            .as_ref()
            .map_or_else(Vec::new, |book| book.codes().collect())
    }

    pub fn active_language(&self) -> Option<&str> {
        self.active_locale
            .as_deref()
            .or_else(|| self.localization.as_ref().map(|book| book.source.as_str()))
    }

    pub fn is_localized_view(&self) -> bool {
        self.active_locale.is_some()
    }

    fn localization_book(&self) -> Result<&Localization, CommandError> {
        self.localization
            .as_ref()
            .ok_or_else(|| invalid("Add a source language first"))
    }

    // Materializing uses the document's existing timeline validation so a batch can move adjacent
    // clips together without temporarily overlapping them or changing the shared timeline.
    fn localized_sequence(
        &self,
        base: &Timeline,
        patch: Option<&SequencePatch>,
    ) -> Result<ProjectedSequence, CommandError> {
        let mut inspector = Inspector::default();
        inspector.restore_overrides(self.base_properties());
        let Some(patch) = patch else {
            return Ok(ProjectedSequence {
                timeline: base.clone(),
                properties: inspector.overrides_snapshot(),
            });
        };
        let mut document = sequence_json(base);
        if let Some(length) = patch.length {
            document["length"] = json!(length);
        }
        let clips = document["clips"]
            .as_array_mut()
            .ok_or_else(|| invalid("Expected sequence clips"))?;
        for (id, fields) in &patch.clips {
            let clip = clips
                .iter_mut()
                .find(|clip| clip["id"] == *id)
                .ok_or_else(|| CommandError::unknown_element("clip", id))?;
            if let Some(group) = fields.get("group_def") {
                apply_override(clip, "group_def", group, id)?;
            }
            for (field, value) in fields {
                if field != "group_def" && field != "properties" {
                    apply_override(clip, field, value, id)?;
                }
            }
            let start = clip["start"].as_i64().unwrap_or_default();
            let duration = clip["duration"].as_i64().unwrap_or_default();
            if start.checked_add(duration).is_none() {
                return Err(invalid(format!(
                    "Clip `{id}` exceeds the sequence frame range"
                )));
            }
        }
        let timeline = parse_sequence(&document, &self.library)?;
        for clip in &timeline.clips {
            let Some(fields) = patch.clips.get(clip.id.as_str()) else {
                continue;
            };
            if let Some(properties) = fields.get("properties").and_then(Value::as_object) {
                inspector.forget_clip(clip.id.as_str());
                for (property, value) in properties {
                    Inspector::lookup(clip.kind, property)?;
                    inspector.restore_override(clip.id.as_str(), property, value)?;
                }
            }
            if fields.contains_key("media") {
                let Some(old) = base
                    .clip(clip.id)
                    .and_then(|entry| entry.media)
                    .and_then(|id| self.library.item(id))
                else {
                    return Err(invalid(format!("Clip `{}` has no source media", clip.id)));
                };
                if !clip
                    .media
                    .and_then(|id| self.library.item(id))
                    .is_some_and(|item| item.kind == old.kind && item.asset.is_some())
                {
                    return Err(invalid(format!(
                        "Replacement media for `{}` must be {}",
                        clip.id,
                        old.kind.label()
                    )));
                }
            }
            if let Some(group) = &clip.group {
                self.groups.check_instance(group)?;
            }
            if let Some(group) = &clip.group
                && let Some(texts) = fields.get("group_texts").and_then(Value::as_object)
            {
                let definition = self.groups.def(group.def.as_str());
                for child in texts.keys() {
                    let entry = definition
                        .and_then(|def| def.child(child))
                        .ok_or_else(|| CommandError::unknown_element("group child", child))?;
                    let crate::group::ChildContent::Text { style, .. } = &entry.content else {
                        return Err(invalid(format!("Group child `{child}` has no text")));
                    };
                    if let Some(text) = group.texts.get(&entry.id) {
                        let style = self.groups.style(style.as_str()).ok_or_else(|| {
                            CommandError::unknown_element("text style", style.as_str())
                        })?;
                        style.spec(text)?;
                    }
                }
            }
            if clip.kind == ClipKind::Audio
                && ["audio", "source_start", "duration", "properties"]
                    .iter()
                    .any(|field| fields.contains_key(*field))
            {
                let audio = clip
                    .media
                    .and_then(|id| self.library.item(id))
                    .filter(|item| item.kind == crate::assets::MediaKind::Audio)
                    .and_then(|item| item.asset.as_ref())
                    .ok_or_else(|| {
                        invalid(format!("Clip `{}` needs a library audio asset", clip.id))
                    })?;
                let span = timeline
                    .rate
                    .frame_to_time(clip.duration)
                    .mul(crate::plan::clip_playback_speed(&inspector, clip.id))
                    .ok_or_else(|| invalid("The audio interval is too large"))?;
                let end = clip
                    .source_start
                    .add(span)
                    .ok_or_else(|| invalid("The audio interval is too large"))?;
                if audio.meta.duration.is_some_and(|length| end > length) {
                    return Err(invalid(format!(
                        "Localized clip `{}` reads past its audio asset",
                        clip.id
                    )));
                }
            }
        }
        Ok(ProjectedSequence {
            timeline,
            properties: inspector.overrides_snapshot(),
        })
    }

    pub(crate) fn check_localized_sequence(&self, base: &Timeline) -> Result<(), CommandError> {
        if let Some(code) = &self.active_locale {
            let variant = self.localization_book()?.locale(code)?;
            self.localized_sequence(base, variant.sequences.get(base.id.as_str()))?;
        }
        Ok(())
    }

    /// Restore the base edit surface, returning the previously selected language if any.
    pub fn detach_localized_view(&mut self) -> Option<String> {
        if let Some(mut base) = self.localization_base.take() {
            copy_view(&self.timeline, &mut base.timeline);
            self.timeline = base.timeline;
            self.inspector.restore_overrides(base.properties);
        }
        self.active_locale.take()
    }

    fn install_locale(&mut self, code: &str) -> Result<(), CommandError> {
        let book = self.localization_book()?;
        if book.source == code {
            self.detach_localized_view();
            self.preview = crate::plan::PreviewIntent::Sequence;
            return Ok(());
        }
        let variant = book.locale(code)?;
        let base = self.base_timeline();
        let mut projected =
            self.localized_sequence(base, variant.sequences.get(base.id.as_str()))?;
        copy_view(&self.timeline, &mut projected.timeline);
        self.detach_localized_view();
        self.localization_base = Some(ProjectedSequence {
            timeline: std::mem::replace(&mut self.timeline, projected.timeline),
            properties: self.inspector.overrides_snapshot(),
        });
        self.inspector.restore_overrides(projected.properties);
        self.active_locale = Some(code.to_string());
        self.preview = crate::plan::PreviewIntent::Sequence;
        Ok(())
    }

    pub(crate) fn restore_localized_view(&mut self, code: Option<&str>) {
        if let Some(code) = code {
            // The restored revision may predate the language or conflict with its overrides.
            // In that case the source stays active and status explains the conflict.
            let _ = self.install_locale(code);
        }
    }

    fn base_revision(&self) -> String {
        let mut document = self.to_document(None);
        let project = document
            .as_object_mut()
            .expect("project document is an object");
        for key in [
            "localization",
            "presentation",
            "active_sequence",
            "export",
            "schema_version",
        ] {
            project.remove(key);
        }
        if let Some(sequences) = document["sequences"].as_array_mut() {
            sequences.sort_unstable_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        }
        let mut used_assets: BTreeSet<_> = self
            .base_sequences()
            .flat_map(|sequence| sequence.clips.iter().filter_map(|clip| clip.media))
            .collect();
        let mut pending: Vec<_> = used_assets.iter().copied().collect();
        while let Some(id) = pending.pop() {
            if let Some(asset) = self.library.item(id).and_then(|item| item.asset.as_ref()) {
                for id in &asset.meta.requires {
                    if used_assets.insert(*id) {
                        pending.push(*id);
                    }
                }
            }
        }
        document["library"] = json!(
            used_assets
                .into_iter()
                .map(|id| { (id.to_string(), self.asset_revision(id.as_str())) })
                .collect::<BTreeMap<_, _>>()
        );
        digest(document)
    }

    fn asset_revision(&self, id: &str) -> Value {
        let Some(asset) = self.library.item(id).and_then(|item| item.asset.as_ref()) else {
            return Value::Null;
        };
        let document = asset_json(asset, None);
        // A verified hash identifies audio equally on disk, in a package, or after relocation.
        if let Some(hash) = document["identity"]["hash"].as_str() {
            return json!({ "hash": hash, "meta": document["meta"] });
        }
        json!({ "source": document["source"], "meta": document["meta"] })
    }

    fn locale_report(&self, variant: &Locale, base_revision: &str) -> Value {
        let mut missing = Vec::new();
        let mut conflicts = Vec::new();
        for sequence in variant.sequences.keys() {
            if self
                .base_sequences()
                .all(|base| base.id != sequence.as_str())
            {
                conflicts
                    .push(json!({ "sequence": sequence, "message": "Sequence no longer exists" }));
            }
        }
        for base in self.base_sequences() {
            let patch = variant.sequences.get(base.id.as_str());
            if let Err(error) = self.localized_sequence(base, patch) {
                conflicts.push(json!({ "sequence": base.id, "message": error.message }));
            }
            for clip in &base.clips {
                let fields = patch.and_then(|patch| patch.clips.get(clip.id.as_str()));
                let mut need = |field: &str, child: Option<&str>| {
                    let supplied = fields.and_then(|fields| fields.get(field));
                    let supplied = match child {
                        Some(child) => supplied.and_then(|texts| texts.get(child)),
                        None => supplied,
                    };
                    if supplied.is_none() {
                        missing.push(json!({ "sequence": base.id, "clip": clip.id, "field": field, "child": child }));
                    }
                };
                match clip.kind {
                    ClipKind::Title => need("text", None),
                    ClipKind::Audio => need("audio", None),
                    ClipKind::Subtitles
                        if clip
                            .subtitles
                            .as_ref()
                            .is_some_and(|spec| !spec.cues.is_empty()) =>
                    {
                        need("cues", None)
                    }
                    ClipKind::Group => {
                        if let Some(def) = clip
                            .group
                            .as_ref()
                            .and_then(|group| self.groups.def(group.def.as_str()))
                        {
                            for child in def.children.iter().filter(|child| child.is_text()) {
                                need("group_texts", Some(child.id.as_str()));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        let media: BTreeMap<_, _> = variant
            .sequences
            .values()
            .flat_map(|patch| patch.clips.values())
            .flat_map(|fields| {
                ["audio", "media"]
                    .into_iter()
                    .filter_map(|key| fields.get(key)?.as_str())
            })
            .map(|id| (id, self.asset_revision(id)))
            .collect();
        let revision = digest(json!([base_revision, variant.sequences, media]));
        let status = if !conflicts.is_empty() {
            "conflict"
        } else if !missing.is_empty() {
            "incomplete"
        } else if variant.approval.as_deref() == Some(&revision) {
            "approved"
        } else if variant.approval.is_some() {
            "stale"
        } else {
            "draft"
        };
        json!({ "status": status, "missing": missing, "conflicts": conflicts,
            "revision": revision, "approved_revision": variant.approval })
    }

    pub fn localization_status(&self) -> Value {
        let Some(book) = &self.localization else {
            return json!({ "enabled": false, "languages": [] });
        };
        let base_revision = self.base_revision();
        let mut reports = vec![json!({ "locale": book.source, "status": "source" })];
        for (code, variant) in &book.locales {
            let mut report = self.locale_report(variant, &base_revision);
            report["locale"] = json!(code);
            reports.push(report);
        }
        json!({ "enabled": true, "source": book.source, "active": self.active_language(),
            "languages": reports })
    }

    pub fn localization_overrides(&self, sequence: &str) -> Value {
        self.active_locale
            .as_ref()
            .and_then(|code| {
                self.localization
                    .as_ref()?
                    .locales
                    .get(code)?
                    .sequences
                    .get(sequence)
            })
            .map_or_else(|| json!({}), |patch| json!(patch))
    }

    /// Serializes one resolved sequence for an external format renderer.
    /// Paths are resolved against the open document and the editable project is untouched.
    pub fn resolve_locale(
        &self,
        locale: &str,
        sequence: Option<&str>,
    ) -> Result<Value, CommandError> {
        let locale = language(locale)?;
        let book = self.localization_book()?;
        let id = sequence.unwrap_or(self.timeline.id.as_str());
        let base = self
            .base_sequences()
            .find(|item| item.id == id)
            .ok_or_else(|| CommandError::unknown_element("sequence", id))?;
        let patch = if locale == book.source {
            None
        } else {
            book.locale(&locale)?.sequences.get(id)
        };
        let mut view = self.localized_sequence(base, patch)?;
        view.properties
            .retain(|clip, _| view.timeline.clip(clip).is_some());
        let mut document = self.to_document(None);
        document["sequences"] = json!([sequence_json(&view.timeline)]);
        document["active_sequence"] = json!(id);
        if let Some(presentation) = document["presentation"].as_object_mut() {
            presentation.retain(|sequence, _| sequence == id);
        }
        document["clip_properties"] = crate::document::property_document(&view.properties);
        document
            .as_object_mut()
            .expect("project object")
            .remove("localization");
        Ok(json!({ "locale": locale, "document": document }))
    }

    pub(crate) fn apply_localization(&mut self, edit: Edit) -> Result<Value, CommandError> {
        let (action, code, changed) = match edit {
            Edit::Resolve { locale, sequence } => {
                return self.resolve_locale(&locale, sequence.as_deref());
            }
            Edit::Status => return Ok(self.localization_status()),
            Edit::Get { locale, sequence } => {
                let code = language(&locale)?;
                let variant = self.localization_book()?.locale(&code)?;
                return Ok(json!({ "locale": code, "sequence": sequence,
                    "overrides": variant.sequences.get(&sequence).cloned().unwrap_or_default() }));
            }
            Edit::Add(raw) => {
                let code = language(&raw)?;
                let changed = match &mut self.localization {
                    None => {
                        self.localization = Some(Localization {
                            source: code.clone(),
                            locales: BTreeMap::new(),
                        });
                        true
                    }
                    Some(book) if book.source == code || book.locales.contains_key(&code) => false,
                    Some(book) => {
                        book.locales.insert(code.clone(), Locale::default());
                        true
                    }
                };
                ("localization.add", code, changed)
            }
            Edit::Select(raw) => {
                let code = language(&raw)?;
                let changed = self.active_language() != Some(&code);
                self.install_locale(&code)?;
                ("localization.select", code, changed)
            }
            Edit::Remove(raw) => {
                let code = language(&raw)?;
                self.localization_book()?.locale(&code)?;
                if self.active_locale.as_deref() == Some(&code) {
                    self.detach_localized_view();
                }
                if let Some(book) = &mut self.localization {
                    book.locales.remove(&code);
                }
                ("localization.remove", code, true)
            }
            Edit::Set {
                locale,
                sequence,
                overrides,
            } => {
                let code = language(&locale)?;
                let variant = self.localization_book()?.locale(&code)?;
                let current = variant.sequences.get(&sequence);
                let changed = if overrides.is_empty() {
                    current.is_some()
                } else {
                    current != Some(&overrides)
                };
                if changed {
                    let base = self
                        .base_sequences()
                        .find(|base| base.id == sequence.as_str());
                    let projection = match base {
                        Some(base) => Some(self.localized_sequence(base, Some(&overrides))?),
                        None if overrides.is_empty() => None,
                        None => return Err(CommandError::unknown_element("sequence", &sequence)),
                    };
                    if let Some(book) = &mut self.localization
                        && let Some(variant) = book.locales.get_mut(&code)
                    {
                        if overrides.is_empty() {
                            variant.sequences.remove(&sequence);
                        } else {
                            variant.sequences.insert(sequence.clone(), overrides);
                        }
                    }
                    if self.active_locale.as_deref() == Some(&code)
                        && self.timeline.id == sequence.as_str()
                        && let Some(mut view) = projection
                    {
                        copy_view(&self.timeline, &mut view.timeline);
                        self.timeline = view.timeline;
                        self.inspector.restore_overrides(view.properties);
                    }
                }
                ("localization.set", code, changed)
            }
            Edit::Approve(raw) => {
                let code = language(&raw)?;
                let variant = self.localization_book()?.locale(&code)?;
                let report = self.locale_report(variant, &self.base_revision());
                if ["conflict", "incomplete"]
                    .contains(&report["status"].as_str().unwrap_or_default())
                {
                    return Err(CommandError::new(
                        "localization_not_ready",
                        "Resolve missing entries and conflicts before approval",
                    )
                    .details(report));
                }
                let revision = report["revision"].as_str().map(str::to_string);
                let changed = variant.approval != revision;
                if let Some(book) = &mut self.localization
                    && let Some(variant) = book.locales.get_mut(&code)
                {
                    variant.approval = revision;
                }
                ("localization.approve", code, changed)
            }
        };
        Ok(json!({ "action": action, "locale": code, "changed": changed }))
    }

    /// Normal timeline/inspector edits use the same commands in either language. While
    /// localizing, capture only the fields the command owns in the localized timeline.
    pub(crate) fn edit_localized_clip(&mut self, command: Command) -> Result<Value, CommandError> {
        let code = self.active_locale.clone().expect("called in a locale view");
        let before = self.timeline.clone();
        let before_subject = self.subject;
        let sequence = self.timeline.id.to_string();
        let mut patch = self
            .localization_book()?
            .locale(&code)?
            .sequences
            .get(&sequence)
            .cloned()
            .unwrap_or_default();
        let (clip, field) = match &command {
            Command::SetClipTextField { clip, field, .. } if field == "content" => {
                (Some(clip.clone()), "text")
            }
            Command::Group(GroupCommand::Text { clip, .. }) => (Some(clip.clone()), "group_texts"),
            Command::MoveClip { clip, track, .. }
                if track.as_deref().is_none_or(|track| {
                    before
                        .clip(clip)
                        .is_some_and(|clip| clip.track == track.trim_start_matches("track:"))
                }) =>
            {
                (Some(clip.clone()), "start")
            }
            Command::TrimClip { clip, .. } => (Some(clip.clone()), "trim"),
            Command::SetSequenceLength { .. } => (None, "length"),
            Command::Live(
                LiveCommand::SubtitleCue { clip, .. }
                | LiveCommand::SubtitleCueAdd { clip, .. }
                | LiveCommand::SubtitleCueRemove { clip, .. }
                | LiveCommand::SubtitleImport { clip, .. },
            ) => (Some(clip.clone()), "cues"),
            _ => return Err(self.shared_edit_error()),
        };
        let outcome = self.apply(command.clone());
        let projected = std::mem::replace(&mut self.timeline, before);
        let subject = std::mem::replace(&mut self.subject, before_subject);
        let outcome = outcome?;
        if let Some(raw) = clip {
            let id = raw.strip_prefix("clip:").unwrap_or(&raw);
            let entry = projected
                .clip(id)
                .ok_or_else(|| CommandError::unknown_element("clip", id))?;
            let fields = patch.clips.entry(id.to_owned()).or_default();
            match field {
                "text" => {
                    fields.insert(
                        "text".into(),
                        json!(entry.text.as_ref().map(|text| &text.content)),
                    );
                }
                "start" => {
                    if outcome["changed"] == false {
                        return Ok(outcome);
                    }
                    fields.insert("start".into(), json!(entry.start));
                }
                "trim" => {
                    if outcome["changed"] == false {
                        return Ok(outcome);
                    }
                    fields.insert("start".into(), json!(entry.start));
                    fields.insert("duration".into(), json!(entry.duration));
                    fields.insert("source_start".into(), entry.source_start.to_json());
                }
                "group_texts" => {
                    if let Command::Group(GroupCommand::Text { child, text, .. }) = command {
                        let child = child.rsplit('/').next().unwrap_or(&child);
                        let child = child.strip_prefix("group_child:").unwrap_or(child);
                        let translation = text.and_then(|_| {
                            entry
                                .group
                                .as_ref()?
                                .texts
                                .iter()
                                .find_map(|(key, value)| (*key == child).then_some(value))
                        });
                        let mut texts = fields
                            .get(field)
                            .and_then(Value::as_object)
                            .cloned()
                            .unwrap_or_default();
                        texts.insert(child.to_string(), json!(translation));
                        fields.insert(field.into(), Value::Object(texts));
                    }
                }
                "cues" => {
                    let spec = entry
                        .subtitles
                        .as_ref()
                        .ok_or_else(|| invalid("Expected a caption clip"))?;
                    fields.insert(
                        field.into(),
                        json!(
                            spec.cues
                                .iter()
                                .map(|cue| cue.to_json())
                                .collect::<Vec<_>>()
                        ),
                    );
                    if self
                        .timeline
                        .clip(id)
                        .is_some_and(|previous| previous.duration != entry.duration)
                    {
                        fields.insert("duration".into(), json!(entry.duration));
                    }
                }
                _ => unreachable!("supported localized edit"),
            }
        } else {
            if outcome["changed"] == false {
                return Ok(outcome);
            }
            patch.length = Some(projected.length);
        }
        let applied = self.apply_localization(Edit::Set {
            locale: code,
            sequence,
            overrides: patch,
        })?;
        self.subject = subject;
        self.timeline.selected_clip = projected.selected_clip;
        self.timeline.selected_track = projected.selected_track;
        self.timeline.playhead = projected.playhead;
        let mut outcome = outcome;
        outcome["changed"] = applied["changed"].clone();
        Ok(outcome)
    }

    pub(crate) fn shared_edit_error(&self) -> CommandError {
        CommandError::new(
            "source_language_required",
            "Select the source language to edit the shared composition",
        )
    }
}

fn copy_view(before: &Timeline, after: &mut Timeline) {
    after.playhead = before.playhead.min(after.length);
    after.zoom = before.zoom;
    after.snap = before.snap;
    after.selected_clip = before.selected_clip.filter(|id| after.clip(id).is_some());
    after.selected_track = before.selected_track.filter(|id| after.track(id).is_some());
}

fn apply_override(
    clip: &mut Value,
    field: &str,
    value: &Value,
    id: &str,
) -> Result<(), CommandError> {
    match field {
        "start" | "duration" | "source_start" | "name" | "keys" => clip[field] = value.clone(),
        "media" if matches!(clip["kind"].as_str(), Some("video" | "scene")) => {
            clip["media"] = value.clone()
        }
        "camera_keys" if clip["kind"] == "scene" => clip[field] = value.clone(),
        "shape" if clip["kind"] == "shape" => clip[field] = value.clone(),
        "group_def" if clip["kind"] == "group" => clip["group"]["def"] = value.clone(),
        "title_style" if clip["kind"] == "title" && clip["text"].is_object() => {
            let style = value
                .as_object()
                .ok_or_else(|| invalid("Expected title style"))?;
            for (key, value) in style {
                clip["text"][key] = value.clone();
            }
        }
        "text" if clip["kind"] == "title" && clip["text"].is_object() => {
            if !value.is_null() {
                clip["text"]["content"] = value.clone();
            }
        }
        "audio" if clip["kind"] == "audio" => {
            if !value.is_null() {
                clip["media"] = value.clone();
            }
        }
        "cues" if clip["kind"] == "subtitles" => {
            if !value.is_null() {
                clip["subtitles"]["cues"] = value.clone();
            }
        }
        "group_texts" if clip["kind"] == "group" => {
            let texts = value
                .as_object()
                .ok_or_else(|| invalid("Expected group text overrides"))?;
            let mut merged = clip["group"]["texts"]
                .as_object()
                .cloned()
                .unwrap_or_default();
            for (child, translation) in texts {
                if !translation.is_null() {
                    merged.insert(child.clone(), translation.clone());
                }
            }
            clip["group"]["texts"] = Value::Object(merged);
        }
        _ => return Err(invalid(format!("Clip `{id}` cannot override `{field}`"))),
    }
    Ok(())
}

fn digest(mut value: Value) -> String {
    // Desktop and render may enable serde_json's ordered-map feature transitively.
    // Approvals must be identical across binaries and independent of document key order.
    value.sort_all_objects();
    format!("{:x}", Sha256::digest(value.to_string().as_bytes()))
}
