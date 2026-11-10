//! The animation controls of the "Efeitos" section (F057/F058, P3): the keys of each parameter,
//! the seed of a noise effect and the switch of a boolean parameter.
//!
//! Like the rest of the section this file owns no state. What the person sees is the core's
//! [`ClipAnimation`] of the clip at the frame the monitor shows (`FxView::frame_here`); every
//! gesture is an op of `scene.fx.edit` through the same handlers a script reaches by `set` and
//! `invoke` on the clip node, so one capture, move, easing change, removal or seed is ONE undo
//! entry and the typed refusal is on screen. Nothing keys by itself: moving a slider edits the
//! base value of the parameter (the core warns when keys prevail), and a key exists only after the
//! capture button or the `key:<param>` action.
//!
//! Node surface of `scene.fx.clip:<id>` (beyond the fields of P2):
//!
//! * fields: `seed` (uint32, noise effects), `key:<control>:<frame>` (put a key of the parameter of
//!   that control at that frame of the clip, at the value sent; a switch takes a boolean);
//! * actions: `key:<param>` (capture at the monitor's frame), `key:<param>:<frame>`,
//!   `remove_key:<param>:<frame>`, `easing:<param>:<frame>:<name>`,
//!   `move_key:<param>:<from>:<to>`, `shift_keys:<param>:<delta>`, `remove_animation:<param>`,
//!   `seed:auto`.
//!
//! Each action is listed with `enabled` and a `disabled_reason` computed from the same model.

use gpui_kit::assets::IconName;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use serde_json::{Value, json};
use tessel_editor_core::directed_scene::fx::{FX_EASINGS, FxClip, FxValue};
use tessel_editor_core::scene_edit::{
    ClipAnimation, FieldKind, FxOp, InspectorField, ParamAnimation,
};

use super::controls::{disabled_tool_button, tool_button};
use super::scene_fx::{ClipView, FxView, LOCALE, clip_node_id, controls_of};
use super::tokens::Tokens;
use super::{Source, Workstation};
use crate::state::{CommandError, Raw};

pub(super) const KEY_PREFIX: &str = "key:";
const REMOVE_KEY: &str = "remove_key:";
const EASING: &str = "easing:";
const MOVE_KEY: &str = "move_key:";
const SHIFT_KEYS: &str = "shift_keys:";
const REMOVE_ANIMATION: &str = "remove_animation:";
pub(super) const SEED_FIELD: &str = "seed";
const SEED_AUTO: &str = "seed:auto";

/// The animation of `clip` at the frame the monitor shows.
pub(super) fn animation_at(view: &FxView, clip: &FxClip) -> ClipAnimation {
    ClipAnimation::of(clip, view.frame_here)
}

/// The words of a typed reason an animation operation does not apply.
fn reason_text(reason: &str) -> &'static str {
    match reason {
        "no_frame" => "O monitor não mostra um quadro desta cena",
        "frame_outside_clip" => "O quadro atual está fora do clipe",
        "no_keys" => "O parâmetro não tem chaves",
        "needs_two_keys" => "Precisa de duas chaves",
        "key_on_this_frame" => "Já existe uma chave neste quadro",
        "seed_is_automatic" => "A semente já é automática",
        "key_order" => "A chave não pode cruzar a vizinha nem sair do clipe",
        _ => "Não se aplica agora",
    }
}

fn action(name: String, label: String, enabled: bool, reason: Option<&str>) -> Value {
    json!({
        "action": name, "label": label, "enabled": enabled,
        "disabled_reason": if enabled { None } else { reason.map(reason_text) },
    })
}

/// The animation actions of a clip node: one set per parameter, one per key, and the seed.
pub(super) fn animation_actions(animation: &ClipAnimation) -> Vec<Value> {
    let mut actions = Vec::new();
    for param in &animation.params {
        let id = param.spec.id;
        let label = param.spec.label.get(LOCALE);
        let capture_reason = match animation.frame_in_clip {
            Some(_) => None,
            None if animation.edit_frame.is_none() => Some("no_frame"),
            None => Some("frame_outside_clip"),
        };
        actions.push(action(
            format!("{KEY_PREFIX}{id}"),
            format!("Criar chave de {label} no quadro atual"),
            capture_reason.is_none(),
            capture_reason,
        ));
        actions.push(action(
            format!("{REMOVE_ANIMATION}{id}"),
            format!("Remover animação de {label}"),
            param.animated,
            Some("no_keys"),
        ));
        for key in &param.keys {
            let frame = key.frame;
            actions.push(action(
                format!("{REMOVE_KEY}{id}:{frame}"),
                format!("Remover chave de {label} em q{frame}"),
                true,
                None,
            ));
            for easing in FX_EASINGS {
                if easing != key.easing.name() {
                    actions.push(action(
                        format!("{EASING}{id}:{frame}:{easing}"),
                        format!("Curva {easing} para a chave de {label} em q{frame}"),
                        true,
                        None,
                    ));
                }
            }
            for delta in [-1_i64, 1] {
                let to = frame + delta;
                let free = to >= 0
                    && to < animation.duration_frames
                    && !param.keys.iter().any(|other| other.frame == to);
                actions.push(action(
                    format!("{MOVE_KEY}{id}:{frame}:{to}"),
                    format!("Mover a chave de {label} de q{frame} para q{to}"),
                    free,
                    Some("key_order"),
                ));
            }
        }
    }
    if let Some(seed) = &animation.seed {
        actions.push(action(
            SEED_AUTO.to_string(),
            "Semente automática".to_string(),
            seed.explicit.is_some(),
            Some("seed_is_automatic"),
        ));
    }
    actions
}

/// The fields the animation adds to a clip node: the seed of a noise effect.
pub(super) fn animation_fields(animation: &ClipAnimation) -> Vec<Value> {
    animation
        .seed
        .iter()
        .map(|seed| {
            json!({
                "field": SEED_FIELD, "label": "Semente", "type": "integer",
                "value": seed.base, "min": 0, "max": u32::MAX, "step": 1,
                "integer": true, "explicit": seed.explicit.is_some(),
                "at_frame": seed.at_frame, "reseed": seed.reseed,
                "enabled": true,
            })
        })
        .collect()
}

fn whole(raw: f64, what: &str) -> Result<i64, CommandError> {
    if raw.fract() != 0.0 || !raw.is_finite() {
        return Err(CommandError::invalid_value(
            format!("{what} must be a whole number"),
            json!("integer"),
        ));
    }
    Ok(raw as i64)
}

fn bad_action(action: &str) -> CommandError {
    CommandError::new(
        "unknown_action",
        format!("`{action}` is not an animation action"),
    )
}

/// Splits `a:b:c` into at most `n` parts after the prefix.
fn parts<'a>(rest: &'a str, n: usize, action: &str) -> Result<Vec<&'a str>, CommandError> {
    let found: Vec<&str> = rest.split(':').collect();
    if found.len() == n && found.iter().all(|part| !part.is_empty()) {
        Ok(found)
    } else {
        Err(bad_action(action))
    }
}

fn frame_part(text: &str) -> Result<i64, CommandError> {
    text.parse::<i64>().map_err(|_| {
        CommandError::invalid_value(
            format!("`{text}` is not a frame of the clip"),
            json!("integer"),
        )
    })
}

impl Workstation {
    /// The ops an animation action of `clip` means, or `None` when `name` is not one of them.
    fn fx_animation_op(
        &self,
        view: &FxView,
        entry: &ClipView,
        name: &str,
    ) -> Option<Result<FxOp, CommandError>> {
        let clip = entry.clip.id.clone();
        let known = |param: &str| {
            entry.clip.effect.param(param).map(|_| ()).ok_or_else(|| {
                CommandError::new(
                    "unknown_param",
                    format!("`{param}` is not a parameter of this effect"),
                )
            })
        };
        let result = (|| -> Result<Option<FxOp>, CommandError> {
            if let Some(rest) = name.strip_prefix(REMOVE_KEY) {
                let p = parts(rest, 2, name)?;
                known(p[0])?;
                return Ok(Some(FxOp::RemoveKey {
                    clip,
                    param: p[0].to_string(),
                    frame: frame_part(p[1])?,
                }));
            }
            if let Some(rest) = name.strip_prefix(EASING) {
                let p = parts(rest, 3, name)?;
                known(p[0])?;
                return Ok(Some(FxOp::SetKeyEasing {
                    clip,
                    param: p[0].to_string(),
                    frame: frame_part(p[1])?,
                    easing: p[2].to_string(),
                }));
            }
            if let Some(rest) = name.strip_prefix(MOVE_KEY) {
                let p = parts(rest, 3, name)?;
                known(p[0])?;
                return Ok(Some(FxOp::MoveKey {
                    clip,
                    param: p[0].to_string(),
                    from: frame_part(p[1])?,
                    to: frame_part(p[2])?,
                }));
            }
            if let Some(rest) = name.strip_prefix(SHIFT_KEYS) {
                let p = parts(rest, 2, name)?;
                known(p[0])?;
                return Ok(Some(FxOp::ShiftKeys {
                    clip,
                    param: Some(p[0].to_string()),
                    delta: frame_part(p[1])?,
                }));
            }
            if let Some(rest) = name.strip_prefix(REMOVE_ANIMATION) {
                known(rest)?;
                return Ok(Some(FxOp::ClearKeys {
                    clip,
                    param: Some(rest.to_string()),
                }));
            }
            if name == SEED_AUTO {
                return Ok(Some(FxOp::SetSeed { clip, seed: None }));
            }
            if let Some(rest) = name.strip_prefix(KEY_PREFIX) {
                // `key:<param>` captures at the monitor's frame; `key:<param>:<frame>` at a frame
                // of the clip. No value: the capture records what the parameter shows there.
                let mut split = rest.split(':');
                let param = split.next().unwrap_or_default();
                known(param)?;
                let frame = match split.next() {
                    Some(text) => frame_part(text)?,
                    None => animation_at(view, &entry.clip)
                        .frame_in_clip
                        .ok_or_else(|| {
                            CommandError::new(
                                "frame_outside_clip",
                                reason_text(if view.frame_here.is_none() {
                                    "no_frame"
                                } else {
                                    "frame_outside_clip"
                                }),
                            )
                        })?,
                };
                if split.next().is_some() {
                    return Err(bad_action(name));
                }
                return Ok(Some(FxOp::KeyParam {
                    clip,
                    param: param.to_string(),
                    frame,
                    value: None,
                    easing: None,
                }));
            }
            Ok(None)
        })();
        result.transpose()
    }

    /// `invoke` of an animation action on a clip node. `None` when `name` is not one.
    pub(super) fn fx_animation_invoke(
        &mut self,
        clip: &str,
        name: &str,
        source: Source,
        cx: &mut Context<Self>,
    ) -> Option<Result<Value, CommandError>> {
        let view = self.fx_view()?;
        let entry = view.clip(clip)?.clone();
        let op = self.fx_animation_op(&view, &entry, name)?;
        Some(op.and_then(|op| self.fx_run(&view.scene, op, source, cx)))
    }

    /// `set` of the seed or of a key on a clip node. `None` when `field` is neither.
    pub(super) fn fx_animation_set(
        &mut self,
        clip: &str,
        field: &str,
        raw: &Raw,
        cx: &mut Context<Self>,
    ) -> Option<Result<Value, CommandError>> {
        let view = self.fx_document_view()?;
        let entry = view.clip(clip)?.clone();
        if field == SEED_FIELD {
            let op = (|| {
                if animation_at(&view, &entry.clip).seed.is_none() {
                    return Err(CommandError::new(
                        "not_applicable",
                        "This effect draws no noise, so it has no seed",
                    ));
                }
                let seed = whole(raw.number()?, "A seed")?;
                u32::try_from(seed).map_err(|_| {
                    CommandError::invalid_value(
                        "A seed is a whole number from 0 to 4294967295",
                        json!("uint32"),
                    )
                })
            })();
            return Some(op.and_then(|seed| {
                self.fx_run(
                    &view.scene,
                    FxOp::SetSeed {
                        clip: clip.to_string(),
                        seed: Some(seed),
                    },
                    Source::Cli,
                    cx,
                )
            }));
        }
        let rest = field.strip_prefix(KEY_PREFIX)?;
        let result = (|| {
            let (control, frame) = rest.split_once(':').ok_or_else(|| bad_action(field))?;
            let frame = frame_part(frame)?;
            let controls = controls_of(&entry.clip, view.edit_frames);
            let target = controls
                .iter()
                .find(|candidate| candidate.id == control)
                .ok_or_else(|| {
                    CommandError::new(
                        "unknown_field",
                        format!("`{}` has no settable field `{control}`", clip_node_id(clip)),
                    )
                })?;
            let value = match target.kind {
                FieldKind::Boolean => f64::from(u8::from(raw.boolean()?)),
                _ => raw.number()?,
            };
            target.key_op(&entry.clip, frame, value)
        })();
        Some(result.and_then(|op| self.fx_run(&view.scene, op, Source::Cli, cx)))
    }

    /// The `seed` stepper: one more or one less than the base shown (wrapping inside uint32 is
    /// refused, not wrapped), written as an explicit seed.
    fn fx_seed_step(&mut self, clip: &str, delta: i64, cx: &mut Context<Self>) {
        let Some(view) = self.fx_document_view() else {
            return;
        };
        let Some(entry) = view.clip(clip) else {
            return;
        };
        let Some(seed) = animation_at(&view, &entry.clip).seed else {
            return;
        };
        let next = i64::from(seed.base) + delta;
        let result = u32::try_from(next)
            .map_err(|_| {
                CommandError::invalid_value(
                    "A seed is a whole number from 0 to 4294967295",
                    json!("uint32"),
                )
            })
            .and_then(|next| {
                self.fx_run(
                    &view.scene,
                    FxOp::SetSeed {
                        clip: clip.to_string(),
                        seed: Some(next),
                    },
                    Source::Gesture,
                    cx,
                )
            });
        if let Err(error) = result {
            self.scene_report(error, cx);
        }
    }

    fn fx_animation_click(&mut self, clip: &str, name: String, cx: &mut Context<Self>) {
        if let Some(Err(error)) = self.fx_animation_invoke(clip, &name, Source::Gesture, cx) {
            self.scene_report(error, cx);
        }
    }

    /// A switch of a boolean parameter: one `set_param`, one undo entry.
    pub(super) fn fx_switch_row(
        &mut self,
        t: &Tokens,
        clip: &FxClip,
        control: &InspectorField,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let workstation = cx.weak_entity();
        let clip_id = clip.id.clone();
        let control_id = control.id.clone();
        let label = control.label(LOCALE);
        let element = format!("fx:{}:{}", clip.id, control.id);
        let toggle = Switch::new(SharedString::from(format!("fx-switch:{element}")))
            .checked(control.value != 0.0)
            .accessibility_label(format!("Efeito {label}"))
            .on_click(move |checked: &bool, _, cx: &mut App| {
                let (clip_id, control_id, value) = (
                    clip_id.clone(),
                    control_id.clone(),
                    f64::from(u8::from(*checked)),
                );
                let _ = workstation.update(cx, |this, cx| {
                    if let Err(error) =
                        this.fx_set_control(&clip_id, &control_id, value, Source::Gesture, cx)
                    {
                        this.scene_report(error, cx);
                    }
                });
            });
        h_flex()
            .id(SharedString::from(format!("fx-switch-row:{element}")))
            .relative()
            .items_center()
            .justify_between()
            .px(px(10.))
            .py(px(2.))
            .text_size(px(11.))
            .text_color(t.text)
            .child(div().child(label))
            .child(
                div()
                    .relative()
                    .child(self.geometry.probe(format!("control:{element}")))
                    .child(toggle),
            )
    }

    /// The keys of every parameter and the seed, below the sliders of a clip.
    pub(super) fn fx_animation_block(
        &mut self,
        t: &Tokens,
        view: &FxView,
        entry: &ClipView,
        cx: &mut Context<Self>,
    ) -> Div {
        let clip = &entry.clip;
        let animation = animation_at(view, clip);
        let mut block = v_flex().w_full();
        // The seed first: it is one line, and the keys below it can be many.
        if let Some(seed) = &animation.seed {
            block = block.child(self.fx_seed_row(t, clip, seed.base, seed.explicit.is_some(), cx));
        }
        for param in &animation.params {
            block = block.child(self.fx_param_keys(t, clip, &animation, param, cx));
        }
        block
    }

    // WHY the long list: a button is built from its parts (what it is called, what it does, why it
    // cannot) and a struct for one call site would only rename the arguments.
    #[allow(clippy::too_many_arguments)]
    fn fx_icon_button(
        &mut self,
        id: String,
        icon: IconName,
        label: String,
        reason: Option<&str>,
        clip: &str,
        action: String,
        cx: &mut Context<Self>,
    ) -> Div {
        let probe = self.geometry.probe(format!("control:{id}"));
        let button_id = SharedString::from(id);
        let button = match reason {
            None => {
                let workstation = cx.weak_entity();
                let clip = clip.to_string();
                tool_button(button_id, icon, &label).on_click(move |_, _, cx| {
                    let (clip, action) = (clip.clone(), action.clone());
                    let _ = workstation.update(cx, |this, cx| {
                        this.fx_animation_click(&clip, action, cx);
                    });
                })
            }
            Some(reason) => disabled_tool_button(button_id, icon, &label, reason),
        };
        div().relative().child(probe).child(button)
    }

    fn fx_param_keys(
        &mut self,
        t: &Tokens,
        clip: &FxClip,
        animation: &ClipAnimation,
        param: &ParamAnimation,
        cx: &mut Context<Self>,
    ) -> Div {
        let id = param.spec.id;
        let label = param.spec.label.get(LOCALE);
        let capture_reason = match animation.frame_in_clip {
            Some(_) => None,
            None if animation.edit_frame.is_none() => Some(reason_text("no_frame")),
            None => Some(reason_text("frame_outside_clip")),
        };
        let now = match (param.animated, param.sampled) {
            (true, Some(value)) => format!(" · agora {}", value_text(value)),
            (true, None) => " · animado".to_string(),
            _ => String::new(),
        };
        let mut head = h_flex()
            .items_center()
            .justify_between()
            .px(px(10.))
            .pt(px(4.))
            .text_size(px(10.))
            .text_color(if param.animated { t.text } else { t.faint })
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .child(format!("{label} · {} chave(s){now}", param.keys.len())),
            );
        head = head.child(self.fx_icon_button(
            format!("fx-key:{}:{id}", clip.id),
            IconName::Plus,
            format!("Criar chave de {label} no quadro atual"),
            capture_reason,
            &clip.id,
            format!("{KEY_PREFIX}{id}"),
            cx,
        ));
        head = head.child(self.fx_icon_button(
            format!("fx-unkey:{}:{id}", clip.id),
            IconName::Trash,
            format!("Remover animação de {label}"),
            (!param.animated).then(|| reason_text("no_keys")),
            &clip.id,
            format!("{REMOVE_ANIMATION}{id}"),
            cx,
        ));
        let mut rows = v_flex().w_full().child(head);
        for key in &param.keys {
            let frame = key.frame;
            let next_easing = {
                let at = FX_EASINGS
                    .iter()
                    .position(|name| *name == key.easing.name())
                    .unwrap_or(0);
                FX_EASINGS[(at + 1) % FX_EASINGS.len()]
            };
            let can_move = |to: i64| {
                to >= 0
                    && to < animation.duration_frames
                    && !param.keys.iter().any(|other| other.frame == to)
            };
            let mut row = h_flex()
                .items_center()
                .gap(px(2.))
                .pl(px(18.))
                .pr(px(10.))
                .text_size(px(10.))
                .text_color(t.text)
                .child(
                    div()
                        .id(SharedString::from(format!(
                            "fx-keylabel:{}:{id}:{frame}",
                            clip.id
                        )))
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .child(format!(
                            "q{frame} · {} · {}",
                            value_text(key.value),
                            key.easing.name()
                        )),
                );
            for (delta, icon, word) in [
                (-1_i64, IconName::ChevronLeft, "para trás"),
                (1, IconName::ChevronRight, "para frente"),
            ] {
                let to = frame + delta;
                row = row.child(self.fx_icon_button(
                    format!("fx-keymove:{}:{id}:{frame}:{to}", clip.id),
                    icon,
                    format!("Mover chave de {label} {word} (q{frame} → q{to})"),
                    (!can_move(to)).then(|| reason_text("key_order")),
                    &clip.id,
                    format!("{MOVE_KEY}{id}:{frame}:{to}"),
                    cx,
                ));
            }
            row = row.child(self.fx_icon_button(
                format!("fx-keyease:{}:{id}:{frame}", clip.id),
                IconName::Replace,
                format!(
                    "Curva da chave em q{frame}: {} (trocar para {next_easing})",
                    key.easing.name()
                ),
                None,
                &clip.id,
                format!("{EASING}{id}:{frame}:{next_easing}"),
                cx,
            ));
            row = row.child(self.fx_icon_button(
                format!("fx-keyrm:{}:{id}:{frame}", clip.id),
                IconName::Close,
                format!("Remover chave de {label} em q{frame}"),
                None,
                &clip.id,
                format!("{REMOVE_KEY}{id}:{frame}"),
                cx,
            ));
            rows = rows.child(row);
        }
        rows
    }

    fn fx_seed_row(
        &mut self,
        t: &Tokens,
        clip: &FxClip,
        base: u32,
        explicit: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let id = clip.id.clone();
        let mut row = h_flex()
            .items_center()
            .gap(px(2.))
            .px(px(10.))
            .pt(px(4.))
            .text_size(px(10.))
            .text_color(t.text)
            .child(
                div()
                    .id(SharedString::from(format!("fx-seed:{id}")))
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .child(format!(
                        "Semente {base}{}",
                        if explicit { "" } else { " (automática)" }
                    )),
            );
        for (delta, icon, word) in [
            (-1_i64, IconName::Minus, "Diminuir"),
            (1, IconName::Plus, "Aumentar"),
        ] {
            let workstation = cx.weak_entity();
            let clip_id = id.clone();
            let label = format!("{word} a semente de {id}");
            let button_id = SharedString::from(format!("fx-seed-step:{id}:{delta}"));
            row = row.child(
                div()
                    .relative()
                    .child(
                        self.geometry
                            .probe(format!("control:fx-seed-step:{id}:{delta}")),
                    )
                    .child(
                        tool_button(button_id, icon, &label).on_click(move |_, _, cx| {
                            let clip_id = clip_id.clone();
                            let _ = workstation
                                .update(cx, |this, cx| this.fx_seed_step(&clip_id, delta, cx));
                        }),
                    ),
            );
        }
        row = row.child(self.fx_icon_button(
            format!("fx-seed-auto:{id}"),
            IconName::RotateCw,
            format!("Semente automática de {id}"),
            (!explicit).then(|| reason_text("seed_is_automatic")),
            &id,
            SEED_AUTO.to_string(),
            cx,
        ));
        row
    }
}

fn value_text(value: FxValue) -> String {
    match value {
        FxValue::Number(number) => super::scene_model::format_number(number),
        FxValue::Bool(on) => if on { "sim" } else { "não" }.to_string(),
        FxValue::Color { r, g, b } => format!(
            "({}, {}, {})",
            super::scene_model::format_number(r),
            super::scene_model::format_number(g),
            super::scene_model::format_number(b)
        ),
        FxValue::Point { x, y } => format!(
            "({}, {})",
            super::scene_model::format_number(x),
            super::scene_model::format_number(y)
        ),
    }
}
