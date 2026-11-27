//! The "Forma" section of the inspector (F067, F068): kind, size, radius, path, fill (solid or
//! gradient), contour and shadow of the selected shape clip. One input per `ShapeSpec` field.
//!
//! The inputs follow the model like the title text does. Typing is not an edit: Enter or leaving
//! a field dispatches ONE `SetClipShapeField`, the same command as `set clip_shape:<id> <field>`,
//! so one commit is one undo entry and a refused value puts the model's value back.

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use tessel_editor_core::shape::ShapeSpec;

use super::tokens::Tokens;
use super::{Source, Workstation};
use crate::state::timeline::ClipKind;
use crate::state::{Command, Raw};

/// `(field, label, unit, heading)` of every input, in the order the section lists them. A
/// non-empty heading opens a group, so the fields of one concern sit together.
const SHAPE_FIELDS: [(&str, &str, &str, &str); 16] = [
    ("kind", "Tipo", "rect, ellipse, path", ""),
    ("width", "Largura", "% da tela", ""),
    ("height", "Altura", "% da tela", ""),
    ("radius", "Raio", "% da altura", ""),
    ("path", "Caminho", "M L H V C Q Z", ""),
    (
        "fill_mode",
        "Modo",
        "solid, linear, radial, none",
        "Preenchimento",
    ),
    ("fill", "Cor", "#rrggbb[aa]", ""),
    ("fill_to", "Cor final", "#rrggbb[aa]", ""),
    ("stops", "Paradas", "pos #cor ...", ""),
    ("gradient_angle", "Ângulo", "graus", ""),
    ("stroke_width", "Espessura", "% da altura", "Contorno"),
    ("stroke", "Cor", "#rrggbb[aa]", ""),
    ("shadow", "Cor", "#rrggbb[aa]", "Sombra"),
    ("shadow_blur", "Desfoque", "% da altura", ""),
    ("shadow_x", "Desloc. X", "% da largura", ""),
    ("shadow_y", "Desloc. Y", "% da altura", ""),
];

/// Text an input shows for a field of the spec. Numbers use the shortest exact spelling, so a
/// value typed as `12.5` comes back as `12.5` and `40.0` as `40`.
fn field_text(spec: &ShapeSpec, field: &str) -> String {
    match field {
        "kind" => spec.kind.as_str().to_string(),
        "fill_mode" => spec.fill_mode.as_str().to_string(),
        "width" => spec.width.to_string(),
        "height" => spec.height.to_string(),
        "radius" => spec.radius.to_string(),
        "gradient_angle" => spec.gradient_angle.to_string(),
        "stroke_width" => spec.stroke_width.to_string(),
        "shadow_blur" => spec.shadow_blur.to_string(),
        "shadow_x" => spec.shadow_x.to_string(),
        "shadow_y" => spec.shadow_y.to_string(),
        "fill_to" => spec.fill_to.clone(),
        "stops" => spec.stops.clone(),
        "stroke" => spec.stroke.clone(),
        "shadow" => spec.shadow.clone(),
        "path" => spec.path.clone(),
        _ => spec.fill.clone(),
    }
}

/// Whether a field holds a colour, so its row shows a swatch.
fn is_color_field(field: &str) -> bool {
    matches!(field, "fill" | "fill_to" | "stroke" | "shadow")
}

/// The swatch beside a colour input. The spec is validated `#rrggbb` or `#rrggbbaa`, so a parse
/// failure can only mean an empty model, which draws nothing.
fn swatch_color(spec: &ShapeSpec, field: &str) -> Option<Hsla> {
    let text = field_text(spec, field);
    let digits = text.strip_prefix('#')?;
    let value = u32::from_str_radix(digits, 16).ok()?;
    match digits.len() {
        6 => Some(rgb(value).into()),
        8 => Some(rgba(value).into()),
        _ => None,
    }
}

pub(super) struct ShapeInputs {
    inputs: Vec<(&'static str, Entity<InputState>)>,
    /// The `(clip, shape)` last pushed into the inputs, so the model is written to them only when
    /// it changed, never over what is being typed.
    shown: Option<(String, ShapeSpec)>,
}

impl ShapeInputs {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Workstation>) -> Self {
        let inputs = SHAPE_FIELDS
            .iter()
            .map(|&(field, label, _, _)| {
                let input = cx.new(|cx| InputState::new(window, cx).placeholder(label));
                cx.subscribe_in(
                    &input,
                    window,
                    move |this, input, event: &InputEvent, _window, cx| {
                        if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                            this.commit_shape_field(field, input, cx);
                        }
                    },
                )
                .detach();
                (field, input)
            })
            .collect();
        Self {
            inputs,
            shown: None,
        }
    }

    fn input(&self, field: &str) -> Option<&Entity<InputState>> {
        self.inputs
            .iter()
            .find(|(name, _)| *name == field)
            .map(|(_, input)| input)
    }

    #[cfg(test)]
    pub(super) fn input_of(&self, field: &str) -> Entity<InputState> {
        self.input(field).cloned().expect("a shape field")
    }

    #[cfg(test)]
    pub(super) fn forget(&mut self) {
        self.shown = None;
    }
}

impl Workstation {
    /// Writes one input's text into the selected shape, through the shared command.
    fn commit_shape_field(
        &mut self,
        field: &'static str,
        input: &Entity<InputState>,
        cx: &mut Context<Self>,
    ) {
        let Some(clip) = self
            .state
            .inspector_clip()
            .filter(|entry| entry.kind == ClipKind::Shape)
            .map(|entry| entry.id.to_string())
        else {
            return;
        };
        // An event from the inputs of a previously selected clip must not edit the new one.
        let shown = self
            .shape_inputs
            .shown
            .as_ref()
            .map(|shown| shown.0.as_str());
        if shown != Some(clip.as_str()) {
            return;
        }
        let text = input.read(cx).value().to_string();
        let _ = self.execute(
            Command::SetClipShapeField {
                clip,
                field: field.to_string(),
                value: Raw::Text(text),
            },
            Source::Gesture,
            cx,
        );
        // A refused or unchanged text is replaced by the model's on the next paint.
        self.shape_inputs.shown = None;
        cx.notify();
    }

    /// Pushes the selected shape into the inputs when the clip or its shape changed.
    fn sync_shape_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = self
            .state
            .inspector_clip()
            .filter(|entry| entry.kind == ClipKind::Shape)
            .and_then(|entry| {
                entry
                    .shape
                    .as_ref()
                    .map(|shape| (entry.id.to_string(), shape.clone()))
            });
        let Some(wanted) = wanted else {
            return;
        };
        if self.shape_inputs.shown.as_ref() == Some(&wanted) {
            return;
        }
        for (field, _, _, _) in SHAPE_FIELDS {
            let Some(input) = self.shape_inputs.input(field) else {
                continue;
            };
            let text = field_text(&wanted.1, field);
            // `set_value` does not emit `Change`/`Blur`, so this cannot loop.
            input.update(cx, |input, cx| input.set_value(text, window, cx));
        }
        self.shape_inputs.shown = Some(wanted);
    }

    pub(super) fn shape_section_body(
        &mut self,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        self.sync_shape_inputs(window, cx);
        let shape = self.shape_inputs.shown.as_ref().map(|(_, shape)| shape);
        let rows = SHAPE_FIELDS
            .iter()
            .map(|&(field, label, unit, heading)| {
                let input = self.shape_inputs.input(field).map(|input| {
                    // The field name keeps the four "Cor" rows apart for a screen reader.
                    Input::new(input)
                        .small()
                        .aria_label(format!("{label} ({field}) da forma"))
                });
                let swatch = shape
                    .filter(|_| is_color_field(field))
                    .and_then(|shape| swatch_color(shape, field));
                let row = h_flex()
                    .items_center()
                    .gap(px(6.))
                    .h(px(28.))
                    .px(px(10.))
                    .text_size(px(11.))
                    .child(div().w(px(84.)).text_color(t.dim).child(label))
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_w(px(0.))
                            .child(self.geometry.probe(format!("control:shape-{field}")))
                            .children(input),
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
                    .child(div().w(px(76.)).truncate().text_color(t.faint).child(unit));
                v_flex()
                    .when(!heading.is_empty(), |group| {
                        group.child(
                            div()
                                .px(px(10.))
                                .pt(px(6.))
                                .pb(px(2.))
                                .text_size(px(10.))
                                .text_color(t.dim)
                                .child(heading),
                        )
                    })
                    .child(row)
            })
            .collect::<Vec<_>>();
        v_flex()
            .w_full()
            .py(px(4.))
            .bg(t.panel)
            .children(rows)
            .child(
                div()
                    .px(px(10.))
                    .pb(px(4.))
                    .text_size(px(10.))
                    .text_color(t.faint)
                    .child("Enter confirma. Opacidade e fades ficam em Composição e Tempo."),
            )
    }
}
