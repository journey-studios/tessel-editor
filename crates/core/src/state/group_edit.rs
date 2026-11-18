//! Editing the groups of a project and the texts of its group clips (F069).
//!
//! Every edit builds the *whole* candidate [`GroupBook`], validates it as a unit and only then
//! replaces the project's book, so a rejected command leaves the project untouched (the
//! `execute` funnel records no undo entry for it). The model and its limits live in
//! [`crate::group`].

use serde_json::{Value, json};

use super::EditorState;
use super::error::{CommandError, Raw};
use super::timeline::{Clip, ClipKind};
use crate::easing::KeyEasing as Easing;
use crate::group::{
    Child, ChildContent, GroupBook, GroupDef, GroupInstance, Prop, RepeatMode, Reveal, RevealMode,
    TextStyle,
};
use crate::id::Id;
use crate::shape::ShapeSpec;

/// Groups a project may define. A cap keeps the document and every undo snapshot bounded.
pub const MAX_DEFS: usize = 64;
/// Text styles a project may define.
pub const MAX_STYLES: usize = 64;
/// Width of the box of a chat's texts, percent of the canvas width (inside the card).
const CHAT_TEXT_WIDTH: f64 = 62.0;
/// The colour of the mark that underlines a chat's source: the pixel the render proofs look for.
pub const CHAT_MARK_COLOR: &str = "#ffb020";

/// One edit of the groups, their text styles or the texts a group clip changes.
#[derive(Debug, Clone, PartialEq)]
pub enum GroupCommand {
    /// Defines a group from its document form (`{id, name?, stagger_ms?, children?}`).
    /// The definition is parsed against the project's styles when it runs.
    Define { definition: Value },
    /// Deletes a group no clip uses.
    Remove { group: String },
    /// Sets `name` or `stagger_ms` of a group.
    Field {
        group: String,
        field: String,
        value: Raw,
    },
    /// Adds a child from its document form, or replaces the child with that ID in place.
    ChildSet { group: String, child: Value },
    /// Deletes a child; the text overrides of clips for it go with it.
    ChildRemove { group: String, child: String },
    /// Sets one base value or content field of a child.
    ChildField {
        group: String,
        child: String,
        field: String,
        value: Raw,
    },
    /// Adds a key, or replaces the key at the same time, on a child's track of `prop`.
    Key {
        group: String,
        child: String,
        prop: Prop,
        time_ms: i64,
        value: f64,
        easing: Option<Easing>,
    },
    KeyRemove {
        group: String,
        child: String,
        prop: Prop,
        time_ms: i64,
    },
    /// Edits `time_ms`, `value` or `easing` of the `index`-th key of a track.
    KeyField {
        group: String,
        child: String,
        prop: Prop,
        index: usize,
        field: String,
        value: Raw,
    },
    /// Changes how a track repeats; `None` keeps the value.
    Repeat {
        group: String,
        child: String,
        prop: Prop,
        repeat: Option<u32>,
        mode: Option<RepeatMode>,
    },
    TrackRemove {
        group: String,
        child: String,
        prop: Prop,
    },
    /// Defines a text style, or replaces the one with that ID.
    StyleSet { style: TextStyle },
    StyleField {
        style: String,
        field: String,
        value: Raw,
    },
    /// Deletes a text style no child uses.
    StyleRemove { style: String },
    /// Makes a group clip show another group (its text overrides are cleared).
    Use { clip: String, group: String },
    /// Sets (or, with `None`, clears) the text a group clip shows for one text child.
    Text {
        clip: String,
        child: String,
        text: Option<String>,
    },
}

/// A candidate book plus what the answer reports about it.
struct Edited {
    book: GroupBook,
    target: String,
    field: &'static str,
    value: Value,
    /// Instances lose the text overrides of children that no longer exist as text children,
    /// instead of the edit being refused (a child was deleted on purpose).
    prune: bool,
}

fn unknown_group(group: &str) -> CommandError {
    CommandError::unknown_element("group", group).target(format!("group_def:{group}"))
}

fn unknown_child(group: &str, child: &str) -> CommandError {
    CommandError::unknown_element("group child", child)
        .target(format!("group_child:{group}/{child}"))
}

fn limit(what: &str, max: usize) -> CommandError {
    CommandError::invalid_value(
        format!("A project has at most {max} {what}"),
        json!({ "max": max }),
    )
}

/// Replaces the group `group` by what `edit` makes of it.
fn replace_def(
    book: &mut GroupBook,
    group: &str,
    edit: impl FnOnce(&GroupDef, &[TextStyle]) -> Result<GroupDef, CommandError>,
) -> Result<(), CommandError> {
    let GroupBook { defs, styles } = book;
    let slot = defs
        .iter_mut()
        .find(|def| def.id == group)
        .ok_or_else(|| unknown_group(group))?;
    *slot = edit(slot, styles)?;
    Ok(())
}

/// Replaces the child `child` of `group` by what `edit` makes of it.
fn replace_child(
    book: &mut GroupBook,
    group: &str,
    child: &str,
    edit: impl FnOnce(&Child, &[TextStyle]) -> Result<Child, CommandError>,
) -> Result<(), CommandError> {
    replace_def(book, group, |def, styles| {
        let current = def
            .child(child)
            .ok_or_else(|| unknown_child(group, child))?;
        def.with_child(edit(current, styles)?, styles)
    })
}

fn child_value(book: &GroupBook, group: &str, child: &str) -> Value {
    book.def(group)
        .and_then(|def| def.child(child))
        .map_or(Value::Null, Child::to_json)
}

impl EditorState {
    /// Group clips of every sequence: the instances an edit of the book must keep valid.
    fn group_clips(&self) -> impl Iterator<Item = &Clip> {
        std::iter::once(&self.timeline)
            .chain(self.sequences.iter())
            .flat_map(|timeline| timeline.clips.iter())
            .filter(|clip| clip.kind == ClipKind::Group)
    }

    fn group_clips_mut(&mut self) -> impl Iterator<Item = &mut Clip> {
        std::iter::once(&mut self.timeline)
            .chain(self.sequences.iter_mut())
            .flat_map(|timeline| timeline.clips.iter_mut())
            .filter(|clip| clip.kind == ClipKind::Group)
    }

    /// The clips that show `group`.
    pub(crate) fn clips_using_group(&self, group: &str) -> Vec<Id> {
        self.group_clips()
            .filter(|clip| {
                clip.group
                    .as_ref()
                    .is_some_and(|instance| instance.def == group)
            })
            .map(|clip| clip.id)
            .collect()
    }

    /// The book with `command` applied, validated, or why it cannot be.
    fn edited_book(&self, command: GroupCommand) -> Result<Edited, CommandError> {
        let mut book = self.groups.clone();
        let mut prune = false;
        let (target, field, value) = match command {
            GroupCommand::Define { definition } => {
                if book.defs.len() >= MAX_DEFS {
                    return Err(limit("groups", MAX_DEFS));
                }
                let def = GroupDef::from_json(&definition, &book.styles)?;
                if book.def(def.id.as_str()).is_some() {
                    return Err(CommandError::new(
                        "duplicate_id",
                        format!("`{}` is already a group", def.id),
                    )
                    .target(format!("group_def:{}", def.id)));
                }
                let target = format!("group_def:{}", def.id);
                let value = def.to_json();
                book.defs.push(def);
                (target, "definition", value)
            }
            GroupCommand::Remove { group } => {
                if book.def(&group).is_none() {
                    return Err(unknown_group(&group));
                }
                let using = self.clips_using_group(&group);
                if !using.is_empty() {
                    return Err(CommandError::new(
                        "in_use",
                        format!(
                            "Group `{group}` is shown by {} clip(s): remove them first",
                            using.len()
                        ),
                    )
                    .target(format!("group_def:{group}"))
                    .details(json!({ "clips": using })));
                }
                book.defs.retain(|def| def.id != group.as_str());
                (format!("group_def:{group}"), "removed", Value::Null)
            }
            GroupCommand::Field {
                group,
                field,
                value,
            } => {
                replace_def(&mut book, &group, |def, styles| {
                    def.with_field(&field, &value, styles)
                })?;
                let value = book.def(&group).map_or(Value::Null, GroupDef::to_json);
                (format!("group_def:{group}"), "definition", value)
            }
            GroupCommand::ChildSet { group, child } => {
                let child = Child::from_json(&child, &book.styles)?;
                let id = child.id;
                replace_def(&mut book, &group, |def, styles| {
                    def.with_child(child, styles)
                })?;
                let value = child_value(&book, &group, id.as_str());
                (format!("group_child:{group}/{id}"), "child", value)
            }
            GroupCommand::ChildRemove { group, child } => {
                replace_def(&mut book, &group, |def, _| def.without_child(&child))?;
                prune = true;
                (
                    format!("group_child:{group}/{child}"),
                    "removed",
                    Value::Null,
                )
            }
            GroupCommand::ChildField {
                group,
                child,
                field,
                value,
            } => {
                replace_child(&mut book, &group, &child, |current, styles| {
                    current.with_field(&field, &value, styles)
                })?;
                let value = child_value(&book, &group, &child);
                (format!("group_child:{group}/{child}"), "child", value)
            }
            GroupCommand::Key {
                group,
                child,
                prop,
                time_ms,
                value,
                easing,
            } => {
                replace_child(&mut book, &group, &child, |current, styles| {
                    current.with_key(prop, time_ms, value, easing, styles)
                })?;
                let value = child_value(&book, &group, &child);
                (format!("group_child:{group}/{child}"), "child", value)
            }
            GroupCommand::KeyRemove {
                group,
                child,
                prop,
                time_ms,
            } => {
                replace_child(&mut book, &group, &child, |current, _| {
                    current.without_key(prop, time_ms)
                })?;
                let value = child_value(&book, &group, &child);
                (format!("group_child:{group}/{child}"), "child", value)
            }
            GroupCommand::KeyField {
                group,
                child,
                prop,
                index,
                field,
                value,
            } => {
                replace_child(&mut book, &group, &child, |current, styles| {
                    current.with_key_field(prop, index, &field, &value, styles)
                })?;
                let value = child_value(&book, &group, &child);
                (format!("group_child:{group}/{child}"), "child", value)
            }
            GroupCommand::Repeat {
                group,
                child,
                prop,
                repeat,
                mode,
            } => {
                replace_child(&mut book, &group, &child, |current, _| {
                    current.with_repeat(prop, repeat, mode)
                })?;
                let value = child_value(&book, &group, &child);
                (format!("group_child:{group}/{child}"), "child", value)
            }
            GroupCommand::TrackRemove { group, child, prop } => {
                replace_child(&mut book, &group, &child, |current, _| {
                    current.without_track(prop)
                })?;
                let value = child_value(&book, &group, &child);
                (format!("group_child:{group}/{child}"), "child", value)
            }
            GroupCommand::StyleSet { style } => {
                style.validate()?;
                let id = style.id;
                let at = book.styles.iter().position(|existing| existing.id == id);
                match at {
                    Some(index) => book.styles[index] = style.clone(),
                    None if book.styles.len() >= MAX_STYLES => {
                        return Err(limit("text styles", MAX_STYLES));
                    }
                    None => book.styles.push(style.clone()),
                }
                (format!("text_style:{id}"), "style", style.to_json())
            }
            GroupCommand::StyleField {
                style,
                field,
                value,
            } => {
                let slot = book
                    .styles
                    .iter_mut()
                    .find(|existing| existing.id == style.as_str())
                    .ok_or_else(|| {
                        CommandError::unknown_element("text style", &style)
                            .target(format!("text_style:{style}"))
                    })?;
                *slot = slot.with_field(&field, &value)?;
                (format!("text_style:{style}"), "style", slot.to_json())
            }
            GroupCommand::StyleRemove { style } => {
                if book.style(&style).is_none() {
                    return Err(CommandError::unknown_element("text style", &style)
                        .target(format!("text_style:{style}")));
                }
                if book.style_in_use(&style) {
                    return Err(CommandError::new(
                        "in_use",
                        format!(
                            "Text style `{style}` is used by a text child: change the child first"
                        ),
                    )
                    .target(format!("text_style:{style}")));
                }
                book.styles.retain(|existing| existing.id != style.as_str());
                (format!("text_style:{style}"), "removed", Value::Null)
            }
            GroupCommand::Use { .. } | GroupCommand::Text { .. } => {
                unreachable!("instance commands are applied by `apply_group`")
            }
        };
        book.validate()?;
        Ok(Edited {
            book,
            target,
            field,
            value,
            prune,
        })
    }

    pub(super) fn apply_group(&mut self, command: GroupCommand) -> Result<Value, CommandError> {
        match command {
            GroupCommand::Use { clip, group } => self.apply_group_use(&clip, &group),
            GroupCommand::Text { clip, child, text } => self.apply_group_text(&clip, &child, text),
            other => {
                let edited = self.edited_book(other)?;
                self.commit_book(edited)
            }
        }
    }

    /// Stores a validated book. Every instance must still resolve against it: a clip left
    /// pointing at a deleted child is refused (or, after a child deletion, loses just that
    /// text override), never stored half valid.
    fn commit_book(&mut self, edited: Edited) -> Result<Value, CommandError> {
        let Edited {
            book,
            target,
            field,
            value,
            prune,
        } = edited;
        let mut pruned = Vec::new();
        for clip in self.group_clips() {
            let Some(instance) = &clip.group else {
                continue;
            };
            let Err(error) = book.check_instance(instance) else {
                continue;
            };
            if !prune {
                return Err(error);
            }
            pruned.push(clip.id);
        }
        for clip in self.group_clips_mut() {
            if !pruned.contains(&clip.id) {
                continue;
            }
            let Some(instance) = clip.group.as_mut() else {
                continue;
            };
            instance.texts.retain(|child, _| {
                book.def(instance.def.as_str())
                    .is_some_and(|def| def.child(child.as_str()).is_some_and(Child::is_text))
            });
        }
        let changed = book != self.groups || !pruned.is_empty();
        self.groups = book;
        Ok(json!({ "target": target, "field": field, "value": value, "changed": changed }))
    }

    fn group_clip(&self, clip: &str) -> Result<(Id, GroupInstance), CommandError> {
        let (id, kind) = self.clip_ref(clip)?;
        let instance = self
            .timeline
            .clip(clip)
            .filter(|_| kind == ClipKind::Group)
            .and_then(|entry| entry.group.clone())
            .ok_or_else(|| {
                CommandError::new("not_applicable", "Only group clips show a group")
                    .target(format!("clip:{id}"))
            })?;
        Ok((id, instance))
    }

    fn store_instance(&mut self, clip: Id, instance: GroupInstance) -> bool {
        let Some(entry) = self
            .timeline
            .clips
            .iter_mut()
            .find(|entry| entry.id == clip)
        else {
            return false;
        };
        let changed = entry.group.as_ref() != Some(&instance);
        entry.group = Some(instance);
        changed
    }

    fn apply_group_use(&mut self, clip: &str, group: &str) -> Result<Value, CommandError> {
        let (id, _) = self.group_clip(clip)?;
        let def = self.groups.def(group).ok_or_else(|| unknown_group(group))?;
        let instance = GroupInstance::new(def.id);
        let changed = self.store_instance(id, instance.clone());
        Ok(json!({ "target": format!("clip:{id}"), "field": "group",
            "value": instance.to_json(), "changed": changed }))
    }

    fn apply_group_text(
        &mut self,
        clip: &str,
        child: &str,
        text: Option<String>,
    ) -> Result<Value, CommandError> {
        let (id, mut instance) = self.group_clip(clip)?;
        let def = self
            .groups
            .def(instance.def.as_str())
            .ok_or_else(|| unknown_group(instance.def.as_str()))?;
        let child = def
            .child(child)
            .filter(|candidate| candidate.is_text())
            .ok_or_else(|| unknown_child(def.id.as_str(), child))?;
        match text {
            Some(raw) => {
                let content = crate::text::normalize(&raw)?;
                // The override must be a text its style can draw (length, characters).
                if let ChildContent::Text { style, .. } = &child.content
                    && let Some(style) = self.groups.style(style.as_str())
                {
                    style.spec(&content)?;
                }
                instance.texts.insert(child.id, content);
            }
            None => {
                instance.texts.remove(&child.id);
            }
        }
        let changed = self.store_instance(id, instance.clone());
        Ok(json!({ "target": format!("clip:{id}"), "field": "group",
            "value": instance.to_json(), "changed": changed }))
    }

    /// A fresh group ID that no group of the project uses.
    pub(super) fn fresh_group_id(&self, prefix: &str) -> Id {
        (1..)
            .filter_map(|number| Id::new(&format!("{prefix}-{number}")).ok())
            .find(|id| self.groups.def(id.as_str()).is_none())
            .unwrap_or_else(|| Id::lit("group"))
    }

    /// The group a toolbar "add group" starts from: a panel and a title that fades and slides in,
    /// small enough to read in one look and complete enough to show every part of the model.
    pub(super) fn starter_book(&self) -> Result<(GroupBook, Id), CommandError> {
        let mut book = self.groups.clone();
        if book.defs.len() >= MAX_DEFS {
            return Err(limit("groups", MAX_DEFS));
        }
        if book.style("title").is_none() {
            if book.styles.len() >= MAX_STYLES {
                return Err(limit("text styles", MAX_STYLES));
            }
            let mut style = TextStyle::new(Id::lit("title"));
            style.size = 8.0;
            book.styles.push(style);
        }
        let id = self.fresh_group_id("group");
        let mut def = GroupDef::new(id);
        def.stagger_ms = 200;
        let panel = ShapeSpec {
            width: 60.0,
            height: 30.0,
            radius: 4.0,
            ..ShapeSpec::default()
        };
        let mut panel = Child::shape(Id::lit("panel"), panel);
        let mut title = Child::text(Id::lit("title"), "Título", Id::lit("title"));
        for child in [&mut panel, &mut title] {
            let styles = book.styles.clone();
            *child = child
                .with_key(Prop::Opacity, 0, 0.0, None, &styles)?
                .with_key(
                    Prop::Opacity,
                    400,
                    100.0,
                    Some(Easing::EaseOutCubic),
                    &styles,
                )?
                .with_key(Prop::Y, 0, 4.0, None, &styles)?
                .with_key(Prop::Y, 400, 0.0, Some(Easing::EaseOutCubic), &styles)?;
        }
        def.children = vec![panel, title];
        book.defs.push(def);
        book.validate()?;
        Ok((book, id))
    }

    /// The group a toolbar "add chat" starts from (F070): a card with the campaign's name, a
    /// question that types in, an answer that appears line by line, and a source that types in and
    /// is then underlined by a mark that fades in. Each step waits for the one before (`after`),
    /// so editing a text (the clip's override of a child) retimes the rest by itself. The texts
    /// are fictitious and generic: they are there to be replaced.
    pub(super) fn chat_book(&self) -> Result<(GroupBook, Id), CommandError> {
        let mut book = self.groups.clone();
        if book.defs.len() >= MAX_DEFS {
            return Err(limit("groups", MAX_DEFS));
        }
        // Styles are shared by every chat of the project: an existing one is kept as the user
        // left it, so a second chat looks like the first.
        for (name, size, color) in [
            ("chat-campaign", 3.0, "#8da2c0"),
            ("chat-question", 4.4, "#ffffff"),
            ("chat-answer", 3.6, "#e6ebf5"),
            ("chat-source", 3.0, "#ffcf40"),
        ] {
            if book.style(name).is_some() {
                continue;
            }
            if book.styles.len() >= MAX_STYLES {
                return Err(limit("text styles", MAX_STYLES));
            }
            let mut style = TextStyle::new(Id::lit(name));
            style.size = size;
            style.color = color.to_string();
            style.width = CHAT_TEXT_WIDTH;
            book.styles.push(style);
        }
        let styles = book.styles.clone();
        let fade_in = |child: Child, ms: i64| -> Result<Child, CommandError> {
            child
                .with_key(Prop::Opacity, 0, 0.0, None, &styles)?
                .with_key(
                    Prop::Opacity,
                    ms,
                    100.0,
                    Some(Easing::EaseOutCubic),
                    &styles,
                )
        };
        let text = |id: &'static str, content: &str, style: &'static str, y: f64| {
            let mut child = Child::text(Id::lit(id), content, Id::lit(style));
            child.y = y;
            child
        };
        let shape = |id: &'static str, spec: ShapeSpec, y: f64| {
            let mut child = Child::shape(Id::lit(id), spec);
            child.y = y;
            child
        };

        let card = ShapeSpec {
            width: 72.0,
            height: 60.0,
            radius: 3.0,
            fill: "#141a26".to_string(),
            ..ShapeSpec::default()
        };
        let panel = fade_in(shape("panel", card, 0.0), 300)?;

        let mut campaign = fade_in(
            text(
                "campaign",
                "Campanha: A Torre de Aurora",
                "chat-campaign",
                -24.0,
            ),
            300,
        )?;
        campaign.delay_ms = 150;

        let mut question = text(
            "question",
            "Quem guarda a chave da torre?",
            "chat-question",
            -14.0,
        );
        question.reveal = Some(Reveal::new(RevealMode::Type));
        question.delay_ms = 400;

        let mut answer = text(
            "answer",
            "A chave está com a guardiã Mira, que a escondeu no cofre do sino antes da última sessão.",
            "chat-answer",
            3.0,
        );
        answer.reveal = Some(Reveal::new(RevealMode::Lines));
        answer.after = Some(question.id);
        answer.delay_ms = 300;

        let mut source = text("source", "Fonte: Sessão 12", "chat-source", 19.0);
        source.reveal = Some(Reveal::new(RevealMode::Type));
        source.after = Some(answer.id);
        source.delay_ms = 200;

        let bar = ShapeSpec {
            width: 34.0,
            height: 0.7,
            radius: 0.35,
            fill: CHAT_MARK_COLOR.to_string(),
            ..ShapeSpec::default()
        };
        let mut mark = fade_in(shape("source-mark", bar, 22.5), 300)?;
        mark.after = Some(source.id);

        let id = self.fresh_group_id("chat");
        let mut def = GroupDef::new(id);
        def.children = vec![panel, campaign, question, answer, source, mark];
        book.defs.push(def);
        book.validate()?;
        Ok((book, id))
    }

    /// Whether any clip of any sequence shows `group`.
    pub fn group_in_use(&self, group: &str) -> bool {
        !self.clips_using_group(group).is_empty()
    }
}
