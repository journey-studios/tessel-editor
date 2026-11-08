//! Inspector panel: Kit tabs, accordion sections and property rows (Kit
//! `Slider`, `Select` and `Switch`) for the selected clip.
//!
//! Edits change illustrative UI state only; no render or export follows.

use gpui_kit::assets::IconName;
use gpui_kit::component::accordion::Accordion;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::select::Select;
use gpui_kit::component::slider::Slider;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::Workstation;
use super::controls::{
    TabSpec, TabStyle, command_tabs, glyph, property_row, quantize, tool_button, value_after_key,
};
use super::media::demo_image;
use super::slider_gesture::{slider_key_up, slider_keys, slider_press};
use super::tokens::Tokens;
use crate::state::camera_edit::CameraKeysCommand;
use crate::state::inspector::{
    InspectorTab, PropertyDef, PropertyKind, SECTIONS, SectionDef, Value, section_properties,
};
use crate::state::{Command, Raw};
use tessel_editor_core::camera_keys::Easing;

fn tab_label(tab: InspectorTab) -> &'static str {
    match tab {
        InspectorTab::Video => "Vídeo",
        InspectorTab::Audio => "Áudio",
        InspectorTab::Color => "Cor",
    }
}

fn format_number(value: f64, decimals: u32) -> String {
    format!("{value:.*}", decimals as usize)
}

/// Slider state is `f32`; anything closer than this is the same value.
const SLIDER_EPSILON: f32 = 1e-4;

impl Workstation {
    fn property_control(
        &mut self,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<Self>,
        property: &'static PropertyDef,
        clip: &'static str,
    ) -> impl IntoElement + use<> {
        let inspector = &self.state.inspector;
        let enabled = inspector.is_enabled(clip, property);
        let modified = inspector.is_modified(clip, property);
        let value = inspector.value(clip, property);
        let id = property.id;
        let (control, text, unit) = match (property.kind, value) {
            (
                PropertyKind::Slider {
                    min,
                    max,
                    step,
                    decimals,
                    unit,
                },
                Value::Number(current),
            ) => {
                let state = self.sliders.get(id).cloned();
                let control = state.map(|state| {
                    let target = current as f32;
                    // `set_value` does not emit a change, so syncing from the
                    // model (a CLI edit, a different clip) cannot loop.
                    let differs = (state.read(cx).value().end() - target).abs() > SLIDER_EPSILON;
                    if differs {
                        state.update(cx, |slider, cx| slider.set_value(target, window, cx));
                    }
                    // The Kit slider has no keyboard support, so this wrapper
                    // keeps arrow, page and home/end stepping. Pointer and keys both
                    // run as ONE gesture (one undo entry) via `slider_gesture`.
                    div()
                        .id(SharedString::from(format!("slider-{id}")))
                        .relative()
                        .child(self.geometry.probe(format!("control:{id}")))
                        .focusable()
                        .flex_1()
                        // Capture phase: the Kit thumb stops its own mouse-down.
                        .capture_any_mouse_down(slider_press(cx))
                        .on_key_down(slider_keys(cx, id, enabled, move |key, shift| {
                            value_after_key(key, shift, current, min, max, step)
                                .map(|next| quantize(next, min, step))
                        }))
                        .on_key_up(slider_key_up(cx, id, move |key| {
                            value_after_key(key, false, current, min, max, step).is_some()
                        }))
                        .child(
                            Slider::new(&state)
                                .horizontal()
                                .disabled(!enabled)
                                .accessibility_label(property.label),
                        )
                        .into_any_element()
                });
                (
                    control.unwrap_or_else(|| div().into_any_element()),
                    Some(format_number(current, decimals)),
                    unit,
                )
            }
            (PropertyKind::Select { .. }, Value::Choice(current)) => {
                let control = self.selects.get(id).cloned().map(|state| {
                    let stale = state.read(cx).selected_value() != Some(&current);
                    if stale {
                        state.update(cx, |select, cx| {
                            select.set_selected_value(&current, window, cx)
                        });
                    }
                    Select::new(&state)
                        .id(SharedString::from(format!("select-{id}")))
                        .small()
                        .disabled(!enabled)
                        .accessibility_label(property.label)
                        .into_any_element()
                });
                (
                    control.unwrap_or_else(|| div().into_any_element()),
                    None,
                    "",
                )
            }
            (_, Value::Bool(on)) => {
                let workstation = cx.weak_entity();
                let switch = Switch::new(SharedString::from(format!("toggle-{id}")))
                    .checked(on)
                    .accessibility_label(property.label)
                    .disabled(!enabled)
                    .on_click(move |checked: &bool, _, cx: &mut App| {
                        let command = Command::SetProperty {
                            property: id.to_string(),
                            value: Raw::Bool(*checked),
                        };
                        let _ = workstation.update(cx, |this, cx| this.dispatch(command, cx));
                    });
                (switch.into_any_element(), None, "")
            }
            _ => (div().into_any_element(), None, ""),
        };
        property_row(t, property.label, modified, enabled, control, text, unit)
    }

    fn inspector_thumbnail(&self, t: &Tokens) -> impl IntoElement + use<> {
        let media = self.state.subject_media();
        let frame = div()
            .relative()
            .w(px(204.))
            .h(px(115.))
            .rounded(px(3.))
            .overflow_hidden()
            .bg(t.app)
            .border_1()
            .border_color(t.border);
        let frame = match media.and_then(|item| item.image) {
            Some(image) => frame.child(
                img(demo_image(image))
                    .size_full()
                    .object_fit(ObjectFit::Cover),
            ),
            None => frame,
        };
        // Rule-of-thirds guides: a framing aid, not a rendering of the values.
        let line = |fraction: f32, vertical: bool| {
            let base = div().absolute().bg(t.text.opacity(0.28));
            if vertical {
                base.left(relative(fraction))
                    .top(px(0.))
                    .bottom(px(0.))
                    .w(px(1.))
            } else {
                base.top(relative(fraction))
                    .left(px(0.))
                    .right(px(0.))
                    .h(px(1.))
            }
        };
        div().flex().justify_center().py(px(8.)).child(
            frame
                .child(line(1.0 / 3.0, true))
                .child(line(2.0 / 3.0, true))
                .child(line(1.0 / 3.0, false))
                .child(line(2.0 / 3.0, false)),
        )
    }

    fn section_body(
        &mut self,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<Self>,
        section: &'static SectionDef,
        clip: &'static str,
    ) -> impl IntoElement + use<> {
        let mut body = div().flex().flex_col().w_full().py(px(4.)).bg(t.panel);
        if section.id == "transform" {
            body = body.child(self.inspector_thumbnail(t));
        }
        for property in section_properties(section.id) {
            body = body.child(self.property_control(t, window, cx, property, clip));
        }
        if section.id == "camera" {
            body = body.child(self.camera_keys_body(t, cx, clip));
        }
        let timing_entry = self
            .state
            .timeline
            .clip(clip)
            .filter(|_| section.id == "timing");
        for entry in timing_entry.into_iter() {
            for (label, value) in [
                ("Início", self.state.timeline.timecode(entry.start)),
                ("Duração", self.state.timeline.timecode(entry.duration)),
            ] {
                body = body.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .h(px(26.))
                        .px(px(10.))
                        .gap(px(6.))
                        .text_size(px(11.))
                        .child(div().w(px(5.)))
                        .child(div().w(px(76.)).text_color(t.dim).child(label))
                        .child(div().flex_1().text_color(t.text).child(value)),
                );
            }
        }
        body
    }

    /// Camera keyframes of a scene clip (BL140): add a key at the playhead, one row per key
    /// (time, easing that cycles on click, remove) and a button that removes them all. Every
    /// button runs a `CameraKeysCommand`, the same handler `ui op`/`set`/`invoke` reach; the
    /// numeric fields of a key are edited with `set camera_key:<clip>:<n>`.
    fn camera_keys_body(
        &mut self,
        t: &Tokens,
        cx: &mut Context<Self>,
        clip: &'static str,
    ) -> impl IntoElement + use<> {
        let Some(entry) = self.state.timeline.clip(clip) else {
            return div().into_any_element();
        };
        let playhead = self.state.timeline.playhead;
        let inside = playhead >= entry.start && playhead < entry.end();
        let keys = entry.camera_keys.clone();
        let orbiting = entry.camera.is_some_and(|camera| camera.orbit != 0.0);
        let capture = self.state.capture_of_clip(entry.id);
        let capture_row = capture.map(|capture| {
            let button = Button::new("camera-capture-use")
                .ghost()
                .xsmall()
                .icon(IconName::Camera)
                .label("Câmera da captura")
                .tooltip("Volta a câmera do clipe para a que fotografou o PLY do SHARP")
                .accessibility_label("Usar a câmera da captura")
                .disabled(!keys.is_empty())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.dispatch(
                        Command::SetClipCameraFromCapture {
                            clip: clip.to_string(),
                        },
                        cx,
                    );
                }));
            v_flex()
                .items_start()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(t.faint)
                        .child(capture.label()),
                )
                .child(button)
        });
        let hint = if keys.is_empty() {
            "Sem chaves: a câmera é a do clipe. Posicione o playhead e adicione uma chave."
        } else if !inside {
            "Mova o playhead sobre o clipe para gravar uma chave."
        } else {
            "A câmera interpola entre as chaves; o easing vale para o trecho seguinte."
        };
        // The same refusals `Capture` applies and the tree node announces as `disabled_reason`.
        let capture_blocked = if orbiting {
            Some("A câmera orbita: zere a órbita (clip.camera) antes de gravar chaves.")
        } else if !inside {
            Some("Mova o playhead sobre o clipe para gravar uma chave.")
        } else {
            None
        };
        let add = Button::new("camera-key-add")
            .ghost()
            .xsmall()
            .icon(IconName::Plus)
            .label("Chave no playhead")
            .tooltip(
                capture_blocked
                    .unwrap_or("Grava a câmera atual no playhead como uma chave")
                    .to_string(),
            )
            .accessibility_label("Adicionar chave de câmera no playhead")
            .disabled(capture_blocked.is_some())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.dispatch(
                    Command::CameraKeys(CameraKeysCommand::Capture {
                        clip: clip.to_string(),
                        easing: None,
                    }),
                    cx,
                );
            }));
        let clear = Button::new("camera-key-clear")
            .ghost()
            .xsmall()
            .icon(IconName::Trash)
            .tooltip("Remover todas as chaves de câmera")
            .accessibility_label("Remover todas as chaves de câmera")
            .disabled(keys.is_empty())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.dispatch(
                    Command::CameraKeys(CameraKeysCommand::Replace {
                        clip: clip.to_string(),
                        keys: Vec::new(),
                    }),
                    cx,
                );
            }));
        let mut column = v_flex()
            .w_full()
            .gap(px(2.))
            .px(px(10.))
            .py(px(4.))
            .children(capture_row)
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(add)
                    .child(clear),
            )
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(t.faint)
                    .child(hint.to_string()),
            );
        if orbiting {
            column = column.child(
                div()
                    .text_size(px(10.))
                    .text_color(t.faint)
                    .child("A câmera orbita: zere a órbita (clip.camera) antes de gravar chaves."),
            );
        }
        for (index, key) in keys.iter().enumerate() {
            let next = {
                let at = Easing::ALL
                    .iter()
                    .position(|easing| *easing == key.easing)
                    .unwrap_or(0);
                Easing::ALL[(at + 1) % Easing::ALL.len()]
            };
            let cycle = Button::new(SharedString::from(format!("camera-key-easing-{index}")))
                .ghost()
                .xsmall()
                .label(key.easing.label())
                .tooltip("Alternar o easing do trecho que começa nesta chave")
                .accessibility_label(format!("Easing da chave {index}"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.dispatch(
                        Command::CameraKeys(CameraKeysCommand::Field {
                            clip: clip.to_string(),
                            index,
                            field: "easing".to_string(),
                            value: Raw::Text(next.label().to_string()),
                        }),
                        cx,
                    );
                }));
            let time = key.time;
            let remove = tool_button(
                SharedString::from(format!("camera-key-remove-{index}")),
                IconName::Trash,
                &format!("Remover a chave em {:.2} s", time.to_f64()),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.dispatch(
                    Command::CameraKeys(CameraKeysCommand::Remove {
                        clip: clip.to_string(),
                        time,
                    }),
                    cx,
                );
            }));
            column = column.child(
                h_flex()
                    .items_center()
                    .gap(px(6.))
                    .h(px(24.))
                    .text_size(px(11.))
                    .child(
                        div()
                            .w(px(46.))
                            .text_color(t.text)
                            .child(format!("{:.2} s", key.time.to_f64())),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .text_color(t.dim)
                            .child(format!("z {:.2} · fov {:.0}°", key.position[2], key.fov)),
                    )
                    .child(cycle)
                    .child(remove),
            );
        }
        column.into_any_element()
    }

    fn inspector_empty(&self, t: &Tokens, message: &str) -> impl IntoElement + use<> {
        v_flex()
            .items_center()
            .gap(px(8.))
            .p(px(20.))
            .text_size(px(11.))
            .text_color(t.dim)
            .child(glyph(IconName::SlidersHorizontal, 20.0, t.faint))
            .child(div().text_center().child(message.to_string()))
    }

    /// The title row of one accordion section: its name and a reset button.
    fn section_title(
        &self,
        t: &Tokens,
        workstation: WeakEntity<Workstation>,
        section: &'static SectionDef,
        modified: bool,
    ) -> impl IntoElement + use<> {
        let command = Command::ResetSection {
            section: section.id.to_string(),
        };
        let reset = tool_button(
            SharedString::from(format!("reset-{}", section.id)),
            IconName::RotateCcw,
            &format!("Redefinir {}", section.label.to_lowercase()),
        )
        .disabled(!modified)
        .on_click(move |_, _, cx| {
            // The title row toggles the section; the reset must not.
            cx.stop_propagation();
            let _ = workstation.update(cx, |this, cx| this.dispatch(command.clone(), cx));
        });
        h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .text_size(px(12.))
            .text_color(t.text)
            .child(section.label)
            .child(reset)
    }

    pub(super) fn render_clip_inspector(
        &mut self,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let active = self.state.inspector.tab;
        let tabs = InspectorTab::ALL
            .iter()
            .map(|tab| TabSpec {
                label: tab_label(*tab).into(),
                active: *tab == active,
                command: Command::SetInspectorTab { tab: *tab },
            })
            .collect();
        let tab_bar = command_tabs(cx, "inspector-tabs", TabStyle::Underline, tabs);

        let clip = self.state.inspector_clip().map(|clip| (clip.id, clip.kind));
        let media = self.state.subject_media().cloned();
        let heading = match (media.as_ref(), self.state.inspector_clip()) {
            (Some(item), _) => (item.name.to_string(), item.meta.to_string()),
            (None, Some(entry)) => (
                entry.name.to_string(),
                format!("TÍTULO · {}", self.state.timeline.timecode(entry.duration)),
            ),
            (None, None) => (
                "Nada selecionado".to_string(),
                "Selecione um clipe".to_string(),
            ),
        };
        let track_badge = self
            .state
            .inspector_clip()
            .map(|entry| self.state.timeline.label(entry.track));
        let mut thumb = div()
            .flex_shrink_0()
            .w(px(56.))
            .h(px(32.))
            .rounded(px(3.))
            .overflow_hidden()
            .bg(t.field)
            .border_1()
            .border_color(t.border);
        if let Some(image) = media.as_ref().and_then(|item| item.image) {
            thumb = thumb.child(
                img(demo_image(image))
                    .size_full()
                    .object_fit(ObjectFit::Cover),
            );
        }
        let summary = h_flex()
            .items_center()
            .gap(px(8.))
            .px(px(10.))
            .py(px(8.))
            .child(thumb)
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.))
                    .child(div().truncate().text_size(px(12.)).child(heading.0))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(9.))
                            .text_color(t.faint)
                            .child(heading.1),
                    ),
            )
            .children(track_badge.map(|label| {
                div()
                    .px(px(6.))
                    .py(px(2.))
                    .rounded(px(3.))
                    .bg(t.video_fill)
                    .text_size(px(9.))
                    .text_color(t.video_label)
                    .child(label)
            }));

        let mut sections = v_flex().w_full();
        match clip {
            None => {
                sections = sections.child(self.inspector_empty(
                    t,
                    "Selecione um clipe na timeline para editar suas propriedades.",
                ));
            }
            Some((clip_id, kind)) => {
                let applicable: Vec<&'static SectionDef> = SECTIONS
                    .iter()
                    .filter(|section| section.tab == active && section.applies.includes(kind))
                    .collect();
                if applicable.is_empty() {
                    let message = match active {
                        InspectorTab::Audio => "Este clipe não tem áudio.",
                        _ => "Esta aba se aplica apenas a clipes de vídeo e título.",
                    };
                    sections = sections.child(self.inspector_empty(t, message));
                }
                let expanded: Vec<bool> = applicable
                    .iter()
                    .map(|section| self.state.inspector.is_expanded(section.id))
                    .collect();
                let mut accordion = Accordion::new("inspector-sections")
                    .multiple(true)
                    .bordered(false);
                for (index, section) in applicable.iter().enumerate() {
                    let modified = self.state.inspector.section_modified(clip_id, section.id)
                        || (section.id == "camera"
                            && self
                                .state
                                .timeline
                                .clip(clip_id.as_str())
                                .is_some_and(|entry| !entry.camera_keys.is_empty()));
                    let title = self.section_title(t, cx.weak_entity(), section, modified);
                    // An expanded section's body is built; a collapsed one is
                    // skipped, and collapsing leaves its values intact.
                    let body = expanded[index]
                        .then(|| self.section_body(t, window, cx, section, clip_id.as_str()));
                    accordion = accordion.item(|item| {
                        item.open(expanded[index])
                            .title(title)
                            .when_some(body, |item, body| item.child(body))
                    });
                }
                // The accordion reports the full set of open sections; the
                // difference with the model is what the user just toggled.
                let workstation = cx.weak_entity();
                accordion = accordion.on_toggle_click(move |open: &[usize], _, cx: &mut App| {
                    for (index, section) in applicable.iter().enumerate() {
                        let now_open = open.contains(&index);
                        if now_open == expanded[index] {
                            continue;
                        }
                        let command = Command::SetSectionExpanded {
                            section: section.id.to_string(),
                            expanded: now_open,
                        };
                        let _ = workstation.update(cx, |this, cx| this.dispatch(command, cx));
                    }
                });
                sections = sections.child(accordion);
            }
        }

        v_flex()
            .relative()
            .size_full()
            .min_h(px(0.))
            .bg(t.panel)
            .child(tab_bar)
            .child(summary)
            .child(
                div()
                    .id("inspector-scroll")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scrollbar()
                    .child(sections),
            )
    }
}
