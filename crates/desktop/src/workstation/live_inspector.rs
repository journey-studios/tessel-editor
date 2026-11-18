//! The "Player" and "Legendas" sections of the inspector (F071): one input per field of a player,
//! one per style field of a caption track, an editor for one cue at a time and an SRT import.
//!
//! The inputs follow the model like the shape inputs do. Typing is not an edit: Enter or leaving a
//! field dispatches ONE `LiveCommand`, the same command as `set clip_player:<id> <field>` or
//! `set subtitle_cue:<id>/<n> <field>`, so one commit is one undo entry and a refused value puts
//! the model's value back.
//!
//! WHY one cue at a time and not a row of inputs per cue: a track holds up to 2000 cues, and an
//! input is a live editor entity; the stepper keeps the window at three inputs however long the
//! track is, while the cue list itself stays in the model (and in `subtitles.export`).

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use serde_json::Value;
use tessel_editor_core::player::PlayerSpec;
use tessel_editor_core::subtitles::{SubtitleSpec, read_srt_file};

use super::controls::tool_button;
use super::tokens::Tokens;
use super::{Activity, Source, Workstation};
use crate::state::live_edit::LiveCommand;
use crate::state::timeline::ClipKind;
use crate::state::{Command, Raw};

/// `(field, label, unit)` of every player input, in the order the document writes them.
const PLAYER_FIELDS: [(&str, &str, &str); 16] = [
    ("audio", "Áudio", "ID do clipe ou vazio"),
    ("width", "Largura", "% da tela"),
    ("height", "Altura", "% da altura"),
    ("bars", "Barras", "8 a 96"),
    ("gap", "Espaço", "fração da barra"),
    ("wave", "Onda", "true, false"),
    ("bar", "Progresso", "true, false"),
    ("time", "Tempo", "true, false"),
    ("wave_color", "Onda", "#rrggbb[aa]"),
    ("idle_color", "Onda a tocar", "#rrggbb[aa]"),
    ("track_color", "Trilho", "#rrggbb[aa]"),
    ("fill_color", "Preenchimento", "#rrggbb[aa]"),
    ("text_color", "Texto", "#rrggbb[aa]"),
    ("bar_height", "Barra", "% da altura"),
    ("text_size", "Tamanho", "% da altura"),
    ("spacing", "Espaçamento", "% da altura"),
];

/// `(field, label, unit)` of every caption style input.
const STYLE_FIELDS: [(&str, &str, &str); 8] = [
    ("size", "Tamanho", "% da altura"),
    ("color", "Cor", "#rrggbb"),
    ("width", "Largura", "% da tela"),
    ("y", "Posição Y", "% da altura"),
    ("box", "Caixa", "#rrggbb[aa] ou none"),
    ("pad", "Margem", "% da altura"),
    ("radius", "Raio", "% da altura"),
    ("burn", "Queimar", "true, false"),
];

/// `(field, label, unit)` of the cue being edited.
const CUE_FIELDS: [(&str, &str, &str); 3] = [
    ("start_ms", "Início", "ms"),
    ("end_ms", "Fim", "ms"),
    ("text", "Texto", "Enter confirma"),
];

/// Text an input shows for a field value: numbers and booleans in their shortest spelling, an
/// unset link as empty text.
fn value_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// The swatch beside a colour input; `none` and anything that is not `#rrggbb[aa]` draw nothing.
fn swatch_color(text: &str) -> Option<Hsla> {
    let value = u32::from_str_radix(text.strip_prefix('#')?, 16).ok()?;
    match text.len() {
        7 => Some(rgb(value).into()),
        9 => Some(rgba(value).into()),
        _ => None,
    }
}

fn is_color_field(field: &str) -> bool {
    field.ends_with("color") || field == "box"
}

type FieldInputs = Vec<(&'static str, Entity<InputState>)>;

type Commit = fn(&mut Workstation, &'static str, &Entity<InputState>, &mut Context<Workstation>);

/// One input per field; Enter or leaving it hands the field to `commit`.
fn make_inputs(
    window: &mut Window,
    cx: &mut Context<Workstation>,
    fields: &[(&'static str, &'static str, &'static str)],
    commit: Commit,
) -> FieldInputs {
    fields
        .iter()
        .map(|&(field, label, _)| {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(label));
            cx.subscribe_in(
                &input,
                window,
                move |this, input, event: &InputEvent, _window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                        commit(this, field, input, cx);
                    }
                },
            )
            .detach();
            (field, input)
        })
        .collect()
}
pub(super) struct LiveInputs {
    player: FieldInputs,
    style: FieldInputs,
    cue: FieldInputs,
    srt_path: Entity<InputState>,
    /// What was last pushed into each group of inputs, so the model is written to them only when
    /// it changed, never over what is being typed.
    player_shown: Option<(String, Value)>,
    style_shown: Option<(String, Value)>,
    cue_shown: Option<(String, usize, Value)>,
    /// The cue the editor shows (presentation only: it is not part of the document).
    cue_index: usize,
}

impl LiveInputs {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Workstation>) -> Self {
        let player = make_inputs(window, cx, &PLAYER_FIELDS, Workstation::commit_player_field);
        let style = make_inputs(window, cx, &STYLE_FIELDS, Workstation::commit_style_field);
        let cue = make_inputs(window, cx, &CUE_FIELDS, Workstation::commit_cue_field);
        // Importing replaces every cue, so it takes Enter only: leaving the field with a path in
        // it must not rewrite the track.
        let srt_path =
            cx.new(|cx| InputState::new(window, cx).placeholder("Caminho do arquivo .srt"));
        cx.subscribe_in(
            &srt_path,
            window,
            |this, input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.commit_srt_import(input, cx);
                }
            },
        )
        .detach();
        Self {
            player,
            style,
            cue,
            srt_path,
            player_shown: None,
            style_shown: None,
            cue_shown: None,
            cue_index: 0,
        }
    }

    fn input<'a>(inputs: &'a FieldInputs, field: &str) -> Option<&'a Entity<InputState>> {
        inputs
            .iter()
            .find(|(name, _)| *name == field)
            .map(|(_, input)| input)
    }

    #[cfg(test)]
    pub(super) fn player_input(&self, field: &str) -> Entity<InputState> {
        Self::input(&self.player, field)
            .cloned()
            .expect("a player field")
    }

    #[cfg(test)]
    pub(super) fn style_input(&self, field: &str) -> Entity<InputState> {
        Self::input(&self.style, field)
            .cloned()
            .expect("a style field")
    }

    #[cfg(test)]
    pub(super) fn cue_input(&self, field: &str) -> Entity<InputState> {
        Self::input(&self.cue, field).cloned().expect("a cue field")
    }

    #[cfg(test)]
    pub(super) fn srt_input(&self) -> Entity<InputState> {
        self.srt_path.clone()
    }

    #[cfg(test)]
    pub(super) fn forget(&mut self) {
        self.player_shown = None;
        self.style_shown = None;
        self.cue_shown = None;
    }
}

/// The caption style of a spec without its cues: what the style inputs show.
fn style_value(spec: &SubtitleSpec) -> Value {
    let mut value = spec.to_json();
    if let Some(object) = value.as_object_mut() {
        object.remove("cues");
    }
    value
}

impl Workstation {
    /// The ID of the selected clip when it is of `kind`.
    fn live_clip_id(&self, kind: ClipKind) -> Option<String> {
        self.state
            .inspector_clip()
            .filter(|entry| entry.kind == kind)
            .map(|entry| entry.id.to_string())
    }

    fn commit_player_field(
        &mut self,
        field: &'static str,
        input: &Entity<InputState>,
        cx: &mut Context<Self>,
    ) {
        let Some(clip) = self.live_clip_id(ClipKind::Player) else {
            return;
        };
        // An event from the inputs of a previously selected clip must not edit the new one.
        let shown = self
            .live_inputs
            .player_shown
            .as_ref()
            .map(|shown| shown.0.as_str());
        if shown != Some(clip.as_str()) {
            return;
        }
        let value = Raw::Text(input.read(cx).value().to_string());
        let _ = self.execute(
            Command::Live(LiveCommand::PlayerField {
                clip,
                field: field.to_string(),
                value,
            }),
            Source::Gesture,
            cx,
        );
        // A refused or unchanged text is replaced by the model's on the next paint.
        self.live_inputs.player_shown = None;
        cx.notify();
    }

    fn commit_style_field(
        &mut self,
        field: &'static str,
        input: &Entity<InputState>,
        cx: &mut Context<Self>,
    ) {
        let Some(clip) = self.live_clip_id(ClipKind::Subtitles) else {
            return;
        };
        let shown = self
            .live_inputs
            .style_shown
            .as_ref()
            .map(|shown| shown.0.as_str());
        if shown != Some(clip.as_str()) {
            return;
        }
        let value = Raw::Text(input.read(cx).value().to_string());
        let _ = self.execute(
            Command::Live(LiveCommand::SubtitleStyle {
                clip,
                field: field.to_string(),
                value,
            }),
            Source::Gesture,
            cx,
        );
        self.live_inputs.style_shown = None;
        cx.notify();
    }

    fn commit_cue_field(
        &mut self,
        field: &'static str,
        input: &Entity<InputState>,
        cx: &mut Context<Self>,
    ) {
        let Some(clip) = self.live_clip_id(ClipKind::Subtitles) else {
            return;
        };
        let index = self.live_inputs.cue_index;
        let shown = self
            .live_inputs
            .cue_shown
            .as_ref()
            .map(|shown| (shown.0.as_str(), shown.1));
        if shown != Some((clip.as_str(), index)) {
            return;
        }
        let value = Raw::Text(input.read(cx).value().to_string());
        let _ = self.execute(
            Command::Live(LiveCommand::SubtitleCue {
                clip,
                index,
                field: field.to_string(),
                value,
            }),
            Source::Gesture,
            cx,
        );
        self.live_inputs.cue_shown = None;
        cx.notify();
    }

    /// Reads the SRT at the typed path into the selected track, as `subtitles.import` does. A
    /// file that cannot be read is reported like any refused edit and changes nothing.
    fn commit_srt_import(&mut self, input: &Entity<InputState>, cx: &mut Context<Self>) {
        let Some(clip) = self.live_clip_id(ClipKind::Subtitles) else {
            return;
        };
        let path = input.read(cx).value().trim().to_string();
        if path.is_empty() {
            return;
        }
        match read_srt_file(&path) {
            Ok(srt) => {
                let imported = self.execute(
                    Command::Live(LiveCommand::SubtitleImport { clip, srt }),
                    Source::Gesture,
                    cx,
                );
                if imported.is_ok() {
                    self.live_inputs.cue_index = 0;
                    self.live_inputs.cue_shown = None;
                    self.live_inputs.style_shown = None;
                }
            }
            Err(error) => {
                self.activity = Some(Activity {
                    source: Source::Gesture,
                    text: error.to_string(),
                    error: true,
                });
            }
        }
        cx.notify();
    }

    /// Moves the cue editor to cue `index` (clamped to the track).
    pub(super) fn show_cue(&mut self, index: usize, cx: &mut Context<Self>) {
        let count = self
            .live_clip_id(ClipKind::Subtitles)
            .and_then(|clip| self.state.subtitles_of(&clip).ok())
            .map_or(0, |(_, spec)| spec.cues.len());
        self.live_inputs.cue_index = index.min(count.saturating_sub(1));
        self.live_inputs.cue_shown = None;
        cx.notify();
    }

    /// Adds a cue where the playhead is and shows it: the same slot `invoke add_cue` picks.
    fn add_cue(&mut self, cx: &mut Context<Self>) {
        let Some(clip) = self.live_clip_id(ClipKind::Subtitles) else {
            return;
        };
        let Ok((start_ms, end_ms)) = self.state.cue_slot_at_playhead(&clip) else {
            return;
        };
        let added = self.execute(
            Command::Live(LiveCommand::SubtitleCueAdd {
                clip,
                start_ms,
                end_ms,
                text: crate::state::live_edit::NEW_CUE_TEXT.to_string(),
            }),
            Source::Gesture,
            cx,
        );
        let Some(index) = added
            .ok()
            .and_then(|outcome| outcome["cue"].as_u64())
            .and_then(|index| usize::try_from(index).ok())
        else {
            return;
        };
        self.show_cue(index, cx);
    }

    pub(super) fn remove_cue(&mut self, cx: &mut Context<Self>) {
        let Some(clip) = self.live_clip_id(ClipKind::Subtitles) else {
            return;
        };
        let index = self.live_inputs.cue_index;
        let removed = self.execute(
            Command::Live(LiveCommand::SubtitleCueRemove { clip, index }),
            Source::Gesture,
            cx,
        );
        if removed.is_ok() {
            self.show_cue(index, cx);
        }
    }

    /// Pushes the selected player into its inputs when the clip or its fields changed.
    fn sync_player_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = self
            .state
            .inspector_clip()
            .filter(|entry| entry.kind == ClipKind::Player)
            .and_then(|entry| {
                let spec: &PlayerSpec = entry.player.as_ref()?;
                Some((entry.id.to_string(), spec.to_json()))
            });
        let Some(wanted) = wanted else {
            return;
        };
        if self.live_inputs.player_shown.as_ref() == Some(&wanted) {
            return;
        }
        push_values(&self.live_inputs.player, &wanted.1, window, cx);
        self.live_inputs.player_shown = Some(wanted);
    }

    fn sync_style_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = self
            .live_clip_id(ClipKind::Subtitles)
            .and_then(|clip| self.state.subtitles_of(&clip).ok())
            .map(|(id, spec)| (id.to_string(), style_value(&spec)));
        let Some(wanted) = wanted else {
            return;
        };
        if self.live_inputs.style_shown.as_ref() == Some(&wanted) {
            return;
        }
        push_values(&self.live_inputs.style, &wanted.1, window, cx);
        self.live_inputs.style_shown = Some(wanted);
    }

    /// Pushes the cue being edited into its inputs; `None` when the track has no cue.
    fn sync_cue_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<(usize, usize)> {
        let clip = self.live_clip_id(ClipKind::Subtitles)?;
        let (_, spec) = self.state.subtitles_of(&clip).ok()?;
        let count = spec.cues.len();
        // The track may have lost cues since the editor last looked (undo, the CLI).
        let index = self.live_inputs.cue_index.min(count.saturating_sub(1));
        self.live_inputs.cue_index = index;
        let cue = spec.cues.get(index)?;
        let wanted = (clip, index, cue.to_json());
        if self.live_inputs.cue_shown.as_ref() != Some(&wanted) {
            push_values(&self.live_inputs.cue, &wanted.2, window, cx);
            self.live_inputs.cue_shown = Some(wanted);
        }
        Some((index, count))
    }

    pub(super) fn player_section_body(
        &mut self,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        self.sync_player_inputs(window, cx);
        let values = self
            .live_inputs
            .player_shown
            .as_ref()
            .map(|shown| shown.1.clone());
        let rows = PLAYER_FIELDS
            .iter()
            .map(|&(field, label, unit)| {
                field_row(
                    t,
                    &self.geometry,
                    "player",
                    (field, label, unit),
                    LiveInputs::input(&self.live_inputs.player, field),
                    values.as_ref(),
                    "do player",
                )
            })
            .collect::<Vec<_>>();
        v_flex()
            .w_full()
            .py(px(4.))
            .bg(t.panel)
            .children(rows)
            .child(hint(
                t,
                "Enter confirma. O áudio é o ID de um clipe de áudio da sequência: a onda, o tempo e a barra seguem esse clipe.",
            ))
    }

    pub(super) fn subtitles_section_body(
        &mut self,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        self.sync_style_inputs(window, cx);
        let cue_position = self.sync_cue_inputs(window, cx);
        let values = self
            .live_inputs
            .style_shown
            .as_ref()
            .map(|shown| shown.1.clone());
        let style_rows = STYLE_FIELDS
            .iter()
            .map(|&(field, label, unit)| {
                field_row(
                    t,
                    &self.geometry,
                    "subtitles",
                    (field, label, unit),
                    LiveInputs::input(&self.live_inputs.style, field),
                    values.as_ref(),
                    "das legendas",
                )
            })
            .collect::<Vec<_>>();
        let cue_values = self
            .live_inputs
            .cue_shown
            .as_ref()
            .map(|shown| shown.2.clone());
        let cue_rows = cue_position
            .map(|_| {
                CUE_FIELDS
                    .iter()
                    .map(|&(field, label, unit)| {
                        field_row(
                            t,
                            &self.geometry,
                            "subtitle-cue",
                            (field, label, unit),
                            LiveInputs::input(&self.live_inputs.cue, field),
                            cue_values.as_ref(),
                            "da legenda",
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let (index, count) = cue_position.unwrap_or((0, 0));
        let heading = if count == 0 {
            "Sem legendas".to_string()
        } else {
            format!("Legenda {} de {count}", index + 1)
        };
        let previous = tool_button(
            "subtitle-cue-previous",
            IconName::ChevronLeft,
            "Legenda anterior",
        )
        .disabled(index == 0)
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.show_cue(index.saturating_sub(1), cx);
        }));
        let next = tool_button(
            "subtitle-cue-next",
            IconName::ChevronRight,
            "Próxima legenda",
        )
        .disabled(index + 1 >= count)
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.show_cue(index + 1, cx);
        }));
        let add = tool_button(
            "subtitle-cue-add",
            IconName::Plus,
            "Adicionar legenda no cursor",
        )
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.add_cue(cx)));
        let remove = tool_button("subtitle-cue-remove", IconName::Trash, "Remover legenda")
            .disabled(count == 0)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.remove_cue(cx)));
        v_flex()
            .w_full()
            .py(px(4.))
            .bg(t.panel)
            .child(section_heading(t, "Legendas (SRT)"))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(6.))
                    .h(px(28.))
                    .px(px(10.))
                    .text_size(px(11.))
                    .child(div().w(px(84.)).text_color(t.dim).child("Importar"))
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_w(px(0.))
                            .child(self.geometry.probe("control:subtitles-import"))
                            .child(
                                Input::new(&self.live_inputs.srt_path)
                                    .small()
                                    .aria_label("Caminho do arquivo SRT a importar"),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap(px(2.))
                    .h(px(28.))
                    .px(px(10.))
                    .text_size(px(11.))
                    .text_color(t.dim)
                    .child(div().flex_1().truncate().child(heading))
                    .child(previous)
                    .child(next)
                    .child(add)
                    .child(remove),
            )
            .children(cue_rows)
            .child(section_heading(t, "Estilo"))
            .children(style_rows)
            .child(hint(
                t,
                "Enter confirma. Com Queimar em false, a exportação não desenha as legendas e grava um .srt ao lado do vídeo.",
            ))
    }
}

/// Writes the model's values into the inputs of one group. `set_value` does not emit `Change` or
/// `Blur`, so this cannot loop back into a commit.
fn push_values(
    inputs: &FieldInputs,
    values: &Value,
    window: &mut Window,
    cx: &mut Context<Workstation>,
) {
    for (field, input) in inputs {
        let text = value_text(&values[*field]);
        input.update(cx, |input, cx| input.set_value(text, window, cx));
    }
}

fn section_heading(t: &Tokens, text: &'static str) -> Div {
    div()
        .px(px(10.))
        .pt(px(6.))
        .pb(px(2.))
        .text_size(px(10.))
        .text_color(t.dim)
        .child(text)
}

fn hint(t: &Tokens, text: &'static str) -> Div {
    div()
        .px(px(10.))
        .pb(px(4.))
        .text_size(px(10.))
        .text_color(t.faint)
        .child(text)
}

/// One labelled input row. `scope` names the probe (`control:<scope>-<field>`) and `noun` ends
/// the accessible name, so the repeated "Cor" rows stay apart for a screen reader.
fn field_row(
    t: &Tokens,
    geometry: &super::Geometry,
    scope: &str,
    (field, label, unit): (&'static str, &'static str, &'static str),
    input: Option<&Entity<InputState>>,
    values: Option<&Value>,
    noun: &str,
) -> Div {
    let swatch = values
        .filter(|_| is_color_field(field))
        .and_then(|values| swatch_color(&value_text(&values[field])));
    h_flex()
        .items_center()
        .gap(px(6.))
        .h(px(28.))
        .px(px(10.))
        .text_size(px(11.))
        .child(div().w(px(84.)).truncate().text_color(t.dim).child(label))
        .child(
            div()
                .relative()
                .flex_1()
                .min_w(px(0.))
                .child(geometry.probe(format!("control:{scope}-{field}")))
                .children(input.map(|input| {
                    Input::new(input)
                        .small()
                        .aria_label(format!("{label} ({field}) {noun}"))
                })),
        )
        .children(swatch.map(|color| {
            div()
                .flex_shrink_0()
                .size(px(16.))
                .rounded(px(3.))
                .border_1()
                .border_color(t.border)
                .bg(color)
        }))
        .child(div().w(px(76.)).truncate().text_color(t.faint).child(unit))
}
