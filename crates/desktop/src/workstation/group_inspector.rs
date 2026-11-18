//! The "Grupo" section of the inspector (F069): the group a clip shows, its children, their key
//! tracks and keys, the texts this clip changes and the project's text styles.
//!
//! Every value is a text input that follows the model like the shape inputs do. Typing is not an
//! edit: Enter or leaving a field dispatches ONE `GroupCommand`, the same command as
//! `set group_child:<group>/<child> ...`, so one commit is one undo entry and a refused value puts
//! the model's value back. The rows are rebuilt from the model on every paint (a key added or
//! removed through the CLI shows up here), and an input is keyed by the target it edits.

use std::collections::{HashMap, HashSet};

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use serde_json::Value;
use tessel_editor_core::easing::KeyEasing as Easing;
use tessel_editor_core::group::{
    Child, ChildContent, GroupDef, GroupInstance, MAX_TIME_MS, Prop, RepeatMode, TextStyle,
};
use tessel_editor_core::shape::ShapeSpec;

use super::controls::{click, tool_button};
use super::tokens::Tokens;
use super::{Source, Workstation};
use crate::state::group_edit::GroupCommand;
use crate::state::timeline::ClipKind;
use crate::state::{Command, Raw};

/// What committing one input does. Holds only IDs, so a row built for a clip that is no longer
/// selected can at worst be refused by the model (`unknown_element`), never edit another clip.
#[derive(Debug, Clone, PartialEq)]
enum RowEdit {
    UseGroup {
        clip: String,
    },
    DefField {
        group: String,
        field: &'static str,
    },
    ChildField {
        group: String,
        child: String,
        field: String,
    },
    Repeat {
        group: String,
        child: String,
        prop: Prop,
    },
    KeyField {
        group: String,
        child: String,
        prop: Prop,
        index: usize,
        field: &'static str,
    },
    Text {
        clip: String,
        child: String,
    },
    StyleField {
        style: String,
        field: &'static str,
    },
}

impl RowEdit {
    /// The command for the text typed into the row, or `None` when the text cannot be one
    /// (a repeat count that is not a number): the input then goes back to the model's value.
    fn command(&self, text: String) -> Option<Command> {
        let value = Raw::Text(text.clone());
        let command = match self.clone() {
            Self::UseGroup { clip } => GroupCommand::Use {
                clip,
                group: text.trim().to_string(),
            },
            Self::DefField { group, field } => GroupCommand::Field {
                group,
                field: field.to_string(),
                value,
            },
            Self::ChildField {
                group,
                child,
                field,
            } => GroupCommand::ChildField {
                group,
                child,
                field,
                value,
            },
            Self::Repeat { group, child, prop } => GroupCommand::Repeat {
                group,
                child,
                prop,
                repeat: Some(text.trim().parse::<u32>().ok()?),
                mode: None,
            },
            Self::KeyField {
                group,
                child,
                prop,
                index,
                field,
            } => GroupCommand::KeyField {
                group,
                child,
                prop,
                index,
                field: field.to_string(),
                value,
            },
            Self::Text { clip, child } => GroupCommand::Text {
                clip,
                child,
                text: Some(text),
            },
            Self::StyleField { style, field } => GroupCommand::StyleField {
                style,
                field: field.to_string(),
                value,
            },
        };
        Some(Command::Group(command))
    }
}

struct GroupRow {
    input: Entity<InputState>,
    edit: RowEdit,
    /// The model's text last pushed into the input; `None` forces it in on the next paint. While
    /// the model's text is unchanged the input is left alone, so typing is never overwritten.
    shown: Option<String>,
}

pub(super) struct GroupInputs {
    rows: HashMap<String, GroupRow>,
    /// Rows painted by the current pass; the others belong to something that is gone.
    seen: HashSet<String>,
    /// The property the "chave no cursor" buttons record; the button beside them cycles it.
    add_prop: Prop,
}

impl GroupInputs {
    pub(super) fn new() -> Self {
        Self {
            rows: HashMap::new(),
            seen: HashSet::new(),
            add_prop: Prop::Opacity,
        }
    }

    #[cfg(test)]
    pub(super) fn input_for(&self, edit_key_part: &str) -> Option<Entity<InputState>> {
        self.rows
            .iter()
            .find(|(key, _)| key.contains(edit_key_part))
            .map(|(_, row)| row.input.clone())
    }

    #[cfg(test)]
    pub(super) fn forget(&mut self) {
        for row in self.rows.values_mut() {
            row.shown = None;
        }
    }
}

/// A copy of what the section paints, so no borrow of the state outlives the rows' creation.
struct GroupView {
    clip: String,
    instance: GroupInstance,
    def: Option<GroupDef>,
    styles: Vec<TextStyle>,
    /// Milliseconds the playhead is after the clip starts; `None` outside the clip.
    playhead_ms: Option<f64>,
    /// The project canvas: how a revealed text breaks into lines depends on it.
    canvas: (u32, u32),
}

fn number_text(value: f64) -> String {
    value.to_string()
}

fn json_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number
            .as_f64()
            .map_or_else(|| number.to_string(), number_text),
        other => other.to_string(),
    }
}

fn base_label(field: &str) -> (&'static str, &'static str) {
    match field {
        "x" => ("X", "% da largura"),
        "y" => ("Y", "% da altura"),
        "scale" => ("Escala", "%"),
        "rotation" => ("Rotação", "graus"),
        _ => ("Opacidade", "%"),
    }
}

fn cycle<T: Copy + PartialEq>(all: &[T], current: T) -> T {
    let at = all.iter().position(|item| *item == current).unwrap_or(0);
    all[(at + 1) % all.len()]
}

fn labelled_row(t: &Tokens, label: &str, unit: &str, input: Input) -> Div {
    h_flex()
        .items_center()
        .gap(px(6.))
        .h(px(28.))
        .px(px(10.))
        .text_size(px(11.))
        .child(
            div()
                .w(px(84.))
                .truncate()
                .text_color(t.dim)
                .child(label.to_string()),
        )
        .child(div().flex_1().min_w(px(0.)).child(input))
        .child(
            div()
                .w(px(64.))
                .truncate()
                .text_color(t.faint)
                .child(unit.to_string()),
        )
}

fn heading(t: &Tokens, text: String, buttons: Vec<AnyElement>) -> Div {
    h_flex()
        .items_center()
        .justify_between()
        .gap(px(4.))
        .px(px(10.))
        .pt(px(8.))
        .pb(px(2.))
        .text_size(px(10.))
        .text_color(t.dim)
        .child(div().flex_1().min_w(px(0.)).truncate().child(text))
        .children(buttons)
}

impl Workstation {
    fn group_view(&self) -> Option<GroupView> {
        let entry = self
            .state
            .inspector_clip()
            .filter(|entry| entry.kind == ClipKind::Group)?;
        let instance = entry.group.clone()?;
        let timeline = &self.state.timeline;
        let inside = timeline.playhead >= entry.start && timeline.playhead < entry.end();
        Some(GroupView {
            clip: entry.id.to_string(),
            def: self.state.groups.def(instance.def.as_str()).cloned(),
            instance,
            canvas: (timeline.width, timeline.height),
            styles: self.state.groups.styles.clone(),
            playhead_ms: inside.then(|| {
                timeline
                    .rate
                    .frame_to_time(timeline.playhead - entry.start)
                    .to_f64()
                    * 1000.0
            }),
        })
    }

    /// Writes one input's text into the model, through the shared command.
    fn commit_group_row(&mut self, key: &str, input: &Entity<InputState>, cx: &mut Context<Self>) {
        let Some(row) = self.group_inputs.rows.get_mut(key) else {
            return;
        };
        let text = input.read(cx).value().to_string();
        // Leaving a field that was not touched is not an edit.
        if row.shown.as_deref() == Some(text.as_str()) {
            return;
        }
        let command = row.edit.command(text);
        // A refused or unchanged text is replaced by the model's on the next paint.
        row.shown = None;
        if let Some(command) = command {
            let _ = self.execute(command, Source::Gesture, cx);
        }
        cx.notify();
    }

    /// The input of `edit`, created on first use and showing `value`.
    fn group_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        edit: RowEdit,
        value: String,
        aria: String,
    ) -> Input {
        let key = format!("{edit:?}");
        if !self.group_inputs.rows.contains_key(&key) {
            let input = cx.new(|cx| InputState::new(window, cx));
            let row_key = key.clone();
            cx.subscribe_in(
                &input,
                window,
                move |this, input, event: &InputEvent, _window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                        this.commit_group_row(&row_key, input, cx);
                    }
                },
            )
            .detach();
            self.group_inputs.rows.insert(
                key.clone(),
                GroupRow {
                    input,
                    edit: edit.clone(),
                    shown: None,
                },
            );
        }
        let Some(row) = self.group_inputs.rows.get_mut(&key) else {
            unreachable!("the row was just inserted");
        };
        row.edit = edit;
        if row.shown.as_deref() != Some(value.as_str()) {
            let text = value.clone();
            // `set_value` does not emit `Change`/`Blur`, so this cannot loop.
            row.input
                .update(cx, |input, cx| input.set_value(text, window, cx));
            row.shown = Some(value);
        }
        let input = row.input.clone();
        self.group_inputs.seen.insert(key);
        Input::new(&input).small().aria_label(aria)
    }

    #[allow(clippy::too_many_arguments)]
    fn group_row(
        &mut self,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<Self>,
        edit: RowEdit,
        label: &str,
        unit: &str,
        value: String,
        aria: String,
    ) -> Div {
        let input = self.group_input(window, cx, edit, value, aria);
        labelled_row(t, label, unit, input)
    }

    fn group_button(
        cx: &mut Context<Self>,
        id: String,
        icon: IconName,
        label: &str,
        command: Command,
    ) -> Button {
        tool_button(SharedString::from(id), icon, label).on_click(click(cx, command))
    }

    pub(super) fn group_section_body(
        &mut self,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        self.group_inputs.seen.clear();
        let mut body = v_flex().w_full().py(px(4.)).bg(t.panel);
        let Some(view) = self.group_view() else {
            return body;
        };
        let clip = view.clip.clone();
        let styles: Vec<&str> = view.styles.iter().map(|style| style.id.as_str()).collect();
        body = body.child(self.group_row(
            t,
            window,
            cx,
            RowEdit::UseGroup { clip: clip.clone() },
            "Grupo",
            "ID",
            view.instance.def.to_string(),
            format!("Grupo do clipe {clip}"),
        ));
        let Some(def) = &view.def else {
            return body.child(
                div()
                    .px(px(10.))
                    .text_size(px(10.))
                    .text_color(t.faint)
                    .child("O grupo deste clipe não existe mais no projeto."),
            );
        };
        let group = def.id.to_string();
        for (field, label, unit, value) in [
            ("name", "Nome", "", def.name.clone()),
            ("stagger_ms", "Escalonar", "ms", def.stagger_ms.to_string()),
        ] {
            body = body.child(self.group_row(
                t,
                window,
                cx,
                RowEdit::DefField {
                    group: group.clone(),
                    field,
                },
                label,
                unit,
                value,
                format!("{label} do grupo {group}"),
            ));
        }
        for (index, child) in def.children.iter().enumerate() {
            body = body.child(self.child_block(t, window, cx, &view, def, index, child, &styles));
        }
        let can_text = !view.styles.is_empty();
        let add_shape = Self::group_button(
            cx,
            "group-add-shape".to_string(),
            IconName::Square,
            "Adicionar uma forma ao grupo",
            Command::Group(GroupCommand::ChildSet {
                group: group.clone(),
                child: serde_json::json!({
                    "id": fresh_child_id(def, "shape"), "kind": "shape", "shape": {},
                }),
            }),
        );
        let add_text = Self::group_button(
            cx,
            "group-add-text".to_string(),
            IconName::Type,
            "Adicionar um texto ao grupo",
            Command::Group(GroupCommand::ChildSet {
                group: group.clone(),
                child: serde_json::json!({
                    "id": fresh_child_id(def, "text"), "kind": "text", "content": "Texto",
                    "style": styles.first().copied().unwrap_or("title"),
                }),
            }),
        )
        .disabled(!can_text);
        body = body.child(heading(
            t,
            "Filhos".to_string(),
            vec![add_shape.into_any_element(), add_text.into_any_element()],
        ));

        // The texts this clip shows instead of the group's own.
        for child in def.children.iter().filter(|child| child.is_text()) {
            let ChildContent::Text { content, .. } = &child.content else {
                continue;
            };
            let overridden = view.instance.texts.get(&child.id);
            let reset = Self::group_button(
                cx,
                format!("group-text-reset-{}", child.id),
                IconName::RotateCcw,
                "Voltar ao texto do grupo",
                Command::Group(GroupCommand::Text {
                    clip: clip.clone(),
                    child: child.id.to_string(),
                    text: None,
                }),
            )
            .disabled(overridden.is_none());
            let row = self.group_row(
                t,
                window,
                cx,
                RowEdit::Text {
                    clip: clip.clone(),
                    child: child.id.to_string(),
                },
                &format!("Texto · {}", child.id),
                "neste clipe",
                overridden.unwrap_or(content).clone(),
                format!("Texto de {} neste clipe", child.id),
            );
            body = body.child(row.child(reset));
        }

        // Text styles are the project's, shared by every group that uses them.
        let add_style = Self::group_button(
            cx,
            "group-add-style".to_string(),
            IconName::Plus,
            "Adicionar um estilo de texto",
            Command::Group(GroupCommand::StyleSet {
                style: TextStyle::new(
                    tessel_editor_core::id::Id::new(&fresh_style_id(&view.styles))
                        .unwrap_or_else(|_| tessel_editor_core::id::Id::lit("style")),
                ),
            }),
        );
        body = body.child(heading(
            t,
            "Estilos de texto".to_string(),
            vec![add_style.into_any_element()],
        ));
        for style in &view.styles {
            body = body.child(self.style_block(t, window, cx, style));
        }
        body.child(
            div()
                .px(px(10.))
                .pt(px(6.))
                .pb(px(4.))
                .text_size(px(10.))
                .text_color(t.faint)
                .child(
                    "Enter confirma. A chave vale a partir do seu tempo e o easing, até a chave seguinte. \
                     Posição, zoom e opacidade do clipe movem o grupo todo.",
                ),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn child_block(
        &mut self,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<Self>,
        view: &GroupView,
        def: &GroupDef,
        index: usize,
        child: &Child,
        styles: &[&str],
    ) -> Div {
        let group = def.id.to_string();
        let id = child.id.to_string();
        let add_prop = if child.is_text() && self.group_inputs.add_prop.shape_only() {
            Prop::Opacity
        } else {
            self.group_inputs.add_prop
        };
        let applicable: Vec<Prop> = Prop::ALL
            .into_iter()
            .filter(|prop| !(child.is_text() && prop.shape_only()))
            .collect();
        let next_prop = cycle(&applicable, add_prop);
        let prop_button = Button::new(SharedString::from(format!("group-prop-{id}")))
            .ghost()
            .xsmall()
            .label(add_prop.label())
            .tooltip("Propriedade que a chave grava; clique para trocar")
            .accessibility_label(format!("Propriedade da chave de {id}"))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.group_inputs.add_prop = next_prop;
                cx.notify();
            }));
        // The key is on the child's own clock, which starts when the child does: after the
        // stagger, or after the child it waits for, plus its delay (`GroupBook::timings`).
        let start_ms = self
            .state
            .groups
            .timings(&view.instance, view.canvas)
            .ok()
            .and_then(|timings| timings.get(index).map(|timing| timing.start_ms))
            .unwrap_or(0);
        let child_ms = view.playhead_ms.map(|ms| ms - start_ms as f64);
        let key_command = view.playhead_ms.and_then(|ms| {
            let time_ms = (child_ms.unwrap_or(0.0).round() as i64).clamp(0, MAX_TIME_MS);
            let value =
                self.state
                    .groups
                    .value_at(&view.instance, &id, add_prop, ms, view.canvas)?;
            Some(Command::Group(GroupCommand::Key {
                group: group.clone(),
                child: id.clone(),
                prop: add_prop,
                time_ms,
                value: (value * 100.0).round() / 100.0,
                easing: None,
            }))
        });
        let key_button = match key_command {
            Some(command) => tool_button(
                SharedString::from(format!("group-key-add-{id}")),
                IconName::Plus,
                "Gravar uma chave no cursor com o valor atual",
            )
            .on_click(click(cx, command)),
            None => tool_button(
                SharedString::from(format!("group-key-add-{id}")),
                IconName::Plus,
                "Mova o cursor sobre o clipe para gravar uma chave",
            )
            .disabled(true),
        };
        let remove = Self::group_button(
            cx,
            format!("group-child-remove-{id}"),
            IconName::Trash,
            "Remover o filho do grupo",
            Command::Group(GroupCommand::ChildRemove {
                group: group.clone(),
                child: id.clone(),
            }),
        );
        let mut block = v_flex().child(heading(
            t,
            format!("{id} · {}", if child.is_text() { "texto" } else { "forma" }),
            vec![
                prop_button.into_any_element(),
                key_button.into_any_element(),
                remove.into_any_element(),
            ],
        ));

        for field in Child::BASE_FIELDS {
            let (label, unit) = base_label(field);
            block = block.child(self.group_row(
                t,
                window,
                cx,
                RowEdit::ChildField {
                    group: group.clone(),
                    child: id.clone(),
                    field: field.to_string(),
                },
                label,
                unit,
                json_text(&child.field_json(field)),
                format!("{label} de {id}"),
            ));
        }
        let own_fields: Vec<(&'static str, &'static str)> = match &child.content {
            ChildContent::Text { .. } => vec![("content", "Texto"), ("style", "Estilo")],
            ChildContent::Shape(_) => ShapeSpec::FIELDS
                .iter()
                .map(|field| (*field, *field))
                .collect(),
        };
        for (field, label) in own_fields {
            let unit = if field == "style" {
                styles.join(", ")
            } else {
                String::new()
            };
            block = block.child(self.group_row(
                t,
                window,
                cx,
                RowEdit::ChildField {
                    group: group.clone(),
                    child: id.clone(),
                    field: field.to_string(),
                },
                label,
                &unit,
                json_text(&child.field_json(field)),
                format!("{label} de {id}"),
            ));
        }

        // When the child starts and how its text appears (F070). A step needs a reveal to mean
        // anything, so its row only exists while the child has one.
        let earlier: Vec<String> = def
            .children
            .iter()
            .take_while(|other| other.id != child.id)
            .map(|other| other.id.to_string())
            .collect();
        let mut schedule: Vec<(&'static str, &'static str, String)> = vec![
            (
                "after",
                "Depois de",
                format!("vazio ou {}", earlier.join(", ")),
            ),
            ("delay_ms", "Atraso", "ms".to_string()),
        ];
        if child.is_text() {
            schedule.push(("reveal", "Revelar", "none, type, lines".to_string()));
            if child.reveal.is_some() {
                schedule.push(("step_ms", "Passo", "ms por letra ou linha".to_string()));
            }
        }
        for (field, label, unit) in schedule {
            block = block.child(self.group_row(
                t,
                window,
                cx,
                RowEdit::ChildField {
                    group: group.clone(),
                    child: id.clone(),
                    field: field.to_string(),
                },
                label,
                &unit,
                json_text(&child.field_json(field)),
                format!("{label} de {id}"),
            ));
        }

        for track in &child.tracks {
            block = block.child(self.track_block(t, window, cx, &group, &id, track));
        }
        block
    }

    fn track_block(
        &mut self,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<Self>,
        group: &str,
        child: &str,
        track: &tessel_editor_core::group::KeyTrack,
    ) -> Div {
        let prop = track.prop;
        let next_mode = cycle(&[RepeatMode::Restart, RepeatMode::PingPong], track.mode);
        let mode = Button::new(SharedString::from(format!(
            "group-mode-{group}-{child}-{}",
            prop.label()
        )))
        .ghost()
        .xsmall()
        .label(track.mode.label())
        .tooltip("Como as repetições tocam: restart volta ao início, pingpong vai e volta")
        .accessibility_label(format!("Modo de repetição de {} em {child}", prop.label()))
        .on_click(click(
            cx,
            Command::Group(GroupCommand::Repeat {
                group: group.to_string(),
                child: child.to_string(),
                prop,
                repeat: None,
                mode: Some(next_mode),
            }),
        ));
        let remove = Self::group_button(
            cx,
            format!("group-track-remove-{group}-{child}-{}", prop.label()),
            IconName::Trash,
            "Remover a animação desta propriedade",
            Command::Group(GroupCommand::TrackRemove {
                group: group.to_string(),
                child: child.to_string(),
                prop,
            }),
        );
        let mut block = v_flex().child(heading(
            t,
            format!("{child} · {} ({} chaves)", prop.label(), track.keys.len()),
            vec![mode.into_any_element(), remove.into_any_element()],
        ));
        block = block.child(self.group_row(
            t,
            window,
            cx,
            RowEdit::Repeat {
                group: group.to_string(),
                child: child.to_string(),
                prop,
            },
            "Repetições",
            "0 = sempre",
            track.repeat.to_string(),
            format!("Repetições de {} em {child}", prop.label()),
        ));
        for (index, key) in track.keys.iter().enumerate() {
            let edit = |field: &'static str| RowEdit::KeyField {
                group: group.to_string(),
                child: child.to_string(),
                prop,
                index,
                field,
            };
            let time = self.group_input(
                window,
                cx,
                edit("time_ms"),
                key.time_ms.to_string(),
                format!("Tempo da chave {index} de {} em {child}", prop.label()),
            );
            let value = self.group_input(
                window,
                cx,
                edit("value"),
                number_text(key.value),
                format!("Valor da chave {index} de {} em {child}", prop.label()),
            );
            let next_easing = cycle(&Easing::ALL, key.easing);
            let easing = Button::new(SharedString::from(format!(
                "group-easing-{group}-{child}-{}-{index}",
                prop.label()
            )))
            .ghost()
            .xsmall()
            .label(key.easing.label())
            .tooltip("Alternar o easing do trecho que começa nesta chave")
            .accessibility_label(format!("Easing da chave {index}"))
            .on_click(click(
                cx,
                Command::Group(GroupCommand::KeyField {
                    group: group.to_string(),
                    child: child.to_string(),
                    prop,
                    index,
                    field: "easing".to_string(),
                    value: Raw::Text(next_easing.label().to_string()),
                }),
            ));
            let remove = Self::group_button(
                cx,
                format!("group-key-remove-{group}-{child}-{}-{index}", prop.label()),
                IconName::Trash,
                &format!("Remover a chave em {} ms", key.time_ms),
                Command::Group(GroupCommand::KeyRemove {
                    group: group.to_string(),
                    child: child.to_string(),
                    prop,
                    time_ms: key.time_ms,
                }),
            );
            block = block.child(
                h_flex()
                    .items_center()
                    .gap(px(4.))
                    .h(px(28.))
                    .px(px(10.))
                    .text_size(px(11.))
                    .child(div().w(px(64.)).child(time))
                    .child(div().flex_1().min_w(px(0.)).child(value))
                    .child(easing)
                    .child(remove),
            );
        }
        block
    }

    fn style_block(
        &mut self,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<Self>,
        style: &TextStyle,
    ) -> Div {
        let id = style.id.to_string();
        let in_use = self.state.groups.style_in_use(&id);
        let remove = Self::group_button(
            cx,
            format!("group-style-remove-{id}"),
            IconName::Trash,
            if in_use {
                "Um filho de texto usa este estilo"
            } else {
                "Remover o estilo"
            },
            Command::Group(GroupCommand::StyleRemove { style: id.clone() }),
        )
        .disabled(in_use);
        let mut block = v_flex().child(heading(
            t,
            format!("Estilo · {id}"),
            vec![remove.into_any_element()],
        ));
        for (field, label, unit) in [
            ("size", "Tamanho", "% da altura"),
            ("color", "Cor", "#rrggbb"),
            ("font", "Fonte", "bitmap, sans"),
            ("align", "Alinhamento", "left, center"),
            ("width", "Caixa", "% da largura, 0 = auto"),
        ] {
            block = block.child(self.group_row(
                t,
                window,
                cx,
                RowEdit::StyleField {
                    style: id.clone(),
                    field,
                },
                label,
                unit,
                json_text(&style.field_json(field)),
                format!("{label} do estilo {id}"),
            ));
        }
        block
    }
}

/// A child ID that no child of `def` uses, `shape-2`, `text-3`, ...
fn fresh_child_id(def: &GroupDef, prefix: &str) -> String {
    (1..)
        .map(|number| format!("{prefix}-{number}"))
        .find(|candidate| def.child(candidate).is_none())
        .unwrap_or_else(|| prefix.to_string())
}

fn fresh_style_id(styles: &[TextStyle]) -> String {
    (1..)
        .map(|number| format!("style-{number}"))
        .find(|candidate| !styles.iter().any(|style| style.id == candidate.as_str()))
        .unwrap_or_else(|| "style".to_string())
}
