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
//! Listed actions carry `enabled` and `disabled_reason` from the model. Parameterized capture,
//! shift and scale are also accepted by invoke without listing every possible argument.

use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use serde_json::{Value, json};
use tessel_editor_core::directed_scene::fx::{FX_EASINGS, FxClip, FxValue};
use tessel_editor_core::scene_edit::{ClipAnimation, FieldKind, FxOp, InspectorField};
use tessel_editor_core::scene_project::{KeySeek, SceneCommand};

use super::scene_fx::{ClipView, FxControl, FxView, LOCALE, clip_node_id, controls_in};
use super::tokens::Tokens;
use super::{Source, Workstation};
use crate::state::{CommandError, Raw};

pub(super) const KEY_PREFIX: &str = "key:";
pub(super) const REMOVE_KEY: &str = "remove_key:";
pub(super) const EASING: &str = "easing:";
pub(super) const MOVE_KEY: &str = "move_key:";
pub(super) const SHIFT_KEYS: &str = "shift_keys:";
pub(super) const REMOVE_ANIMATION: &str = "remove_animation:";
pub(super) const KEY_FRAME: &str = "key_frame:";
pub(super) const SCALE_KEYS: &str = "scale_keys:";
pub(super) const GO: &str = "go:";
pub(super) const SEED_FIELD: &str = "seed";
pub(super) const SEED_AUTO: &str = "seed:auto";

/// The animation of `clip` at the frame the monitor shows.
pub(super) fn animation_at(view: &FxView, clip: &FxClip) -> ClipAnimation {
    ClipAnimation::of(clip, view.frame_here)
}

/// The words of a typed reason an animation operation does not apply.
pub(super) fn reason_text(reason: &str) -> &'static str {
    match reason {
        "no_frame" => "O monitor não mostra um quadro desta cena",
        "frame_outside_clip" => "O quadro atual está fora do clipe",
        "no_keys" => "O parâmetro não tem chaves",
        "needs_two_keys" => "Precisa de duas chaves",
        "seed_is_automatic" => "A semente já é automática",
        "key_order" => "A chave não pode cruzar a vizinha nem sair do clipe",
        "no_previous_key" => "Não há chave antes do quadro atual",
        "no_next_key" => "Não há chave depois do quadro atual",
        "base_under_keys" => {
            "O parâmetro está animado: edite as chaves (o valor base fica por baixo)"
        }
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
        for (to, label_to, reason, enabled) in [
            (
                "previous",
                "anterior",
                "no_previous_key",
                param.previous.is_some(),
            ),
            ("next", "próxima", "no_next_key", param.next.is_some()),
            ("first", "primeira", "no_keys", param.animated),
            ("last", "última", "no_keys", param.animated),
        ] {
            let why = if !param.animated {
                "no_keys"
            } else if animation.edit_frame.is_none() && matches!(to, "previous" | "next") {
                "no_frame"
            } else {
                reason
            };
            actions.push(action(
                format!("{GO}{id}:{to}"),
                format!("Ir para a chave {label_to} de {label}"),
                enabled,
                Some(why),
            ));
        }
        for key in &param.keys {
            let frame = key.frame;
            actions.push(action(
                format!("{GO}{id}:{frame}"),
                format!("Ir para a chave de {label} em q{frame}"),
                true,
                None,
            ));
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

/// The fields the animation adds to a clip node: the seed of a noise effect, and for each
/// parameter its key frames and values (`key_frame:<param>:<frame>` moves the key to the frame
/// sent, `key:<control>:<frame>` sets the key's value for one component) and the shift of its keys.
pub(super) fn animation_fields(animation: &ClipAnimation, controls: &[FxControl]) -> Vec<Value> {
    let mut fields: Vec<Value> = animation
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
        .collect();
    let last_frame = animation.duration_frames - 1;
    for param in &animation.params {
        let id = param.spec.id;
        let label = param.spec.label.get(LOCALE);
        // Like the panel, the shift of a curve exists only where there is a curve.
        if param.animated {
            fields.push(json!({
                "field": format!("{SHIFT_KEYS}{id}"),
                "label": format!("Deslocar chaves de {label}"),
                "type": "integer", "value": 0, "step": 1, "integer": true, "enabled": true,
            }));
        }
        for key in &param.keys {
            let frame = key.frame;
            fields.push(json!({
                "field": format!("{KEY_FRAME}{id}:{frame}"),
                "label": format!("Quadro da chave de {label} em q{frame}"),
                "type": "integer", "value": frame, "min": 0, "max": last_frame, "step": 1,
                "integer": true, "enabled": true,
            }));
            for control in controls
                .iter()
                .filter(|control| control.param() == Some(id))
            {
                let Some(value) = control.read(key.value) else {
                    continue;
                };
                let mut field = json!({
                    "field": format!("{KEY_PREFIX}{}:{frame}", control.id),
                    "label": format!("{} da chave em q{frame}", control.label(LOCALE)),
                    "value": value, "enabled": true,
                });
                if control.kind == FieldKind::Boolean {
                    field["type"] = json!("boolean");
                    field["value"] = json!(value != 0.0);
                } else {
                    field["type"] = json!("number");
                    field["min"] = json!(control.min);
                    field["max"] = json!(control.max);
                    field["step"] = json!(control.step);
                }
                fields.push(field);
            }
        }
    }
    fields
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
            if let Some(rest) = name.strip_prefix(SCALE_KEYS) {
                let p = parts(rest, 4, name)?;
                known(p[0])?;
                return Ok(Some(FxOp::ScaleKeys {
                    clip,
                    param: Some(p[0].to_string()),
                    anchor: frame_part(p[1])?,
                    num: frame_part(p[2])?,
                    den: frame_part(p[3])?,
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
                            let reason = if view.frame_here.is_none() {
                                "no_frame"
                            } else {
                                "frame_outside_clip"
                            };
                            CommandError::new(reason, reason_text(reason))
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
        self.fx_animation_invoke_guarded(clip, name, None, source, cx)
    }

    /// The same, built on the project revision the caller's view had (`base_revision`): a key
    /// that moved, was undone or was edited from elsewhere since is refused with `stale_revision`,
    /// nothing written. A seek is not an edit and has no revision to guard.
    pub(super) fn fx_animation_invoke_guarded(
        &mut self,
        clip: &str,
        name: &str,
        base_revision: Option<u64>,
        source: Source,
        cx: &mut Context<Self>,
    ) -> Option<Result<Value, CommandError>> {
        let view = self.fx_view()?;
        let entry = view.clip(clip)?.clone();
        if let Some(rest) = name.strip_prefix(GO) {
            return Some(self.fx_go(&view, &entry, rest, name, source, cx));
        }
        let op = self.fx_animation_op(&view, &entry, name)?;
        Some(op.and_then(|op| self.fx_run_guarded(&view.scene, op, base_revision, source, cx)))
    }

    /// `go:<param>:previous|next|first|last|<frame of the clip>`: the core's `scene.fx.seek`.
    fn fx_go(
        &mut self,
        view: &FxView,
        entry: &ClipView,
        rest: &str,
        name: &str,
        source: Source,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        let p = parts(rest, 2, name)?;
        entry.clip.effect.param(p[0]).ok_or_else(|| {
            CommandError::new(
                "unknown_param",
                format!("`{}` is not a parameter of this effect", p[0]),
            )
        })?;
        let to = match p[1] {
            "previous" => KeySeek::Previous,
            "next" => KeySeek::Next,
            "first" => KeySeek::First,
            "last" => KeySeek::Last,
            frame => KeySeek::Frame(frame_part(frame)?),
        };
        self.scene_execute(
            SceneCommand::FxSeek {
                scene: view.scene.clone(),
                clip: entry.clip.id.clone(),
                param: p[0].to_string(),
                to,
            },
            source,
            cx,
        )
    }

    /// `set` of the seed or of a key on a clip node. `None` when `field` is neither.
    pub(super) fn fx_animation_set(
        &mut self,
        clip: &str,
        field: &str,
        raw: &Raw,
        cx: &mut Context<Self>,
    ) -> Option<Result<Value, CommandError>> {
        self.fx_animation_set_guarded(clip, field, raw, None, Source::Cli, cx)
    }

    /// `set` of an animation field, optionally built on a project revision (see
    /// [`Self::fx_animation_invoke_guarded`]). The text of an editor reaches here as `Raw::Text`,
    /// so a value that is not a number is a typed error, never a 0.
    pub(super) fn fx_animation_set_guarded(
        &mut self,
        clip: &str,
        field: &str,
        raw: &Raw,
        base_revision: Option<u64>,
        source: Source,
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
                let seed = raw.integer().map_err(|_| {
                    CommandError::invalid_value(
                        "A seed is a whole number from 0 to 4294967295",
                        json!("uint32"),
                    )
                })?;
                u32::try_from(seed).map_err(|_| {
                    CommandError::invalid_value(
                        "A seed is a whole number from 0 to 4294967295",
                        json!("uint32"),
                    )
                })
            })();
            return Some(op.and_then(|seed| {
                self.fx_run_guarded(
                    &view.scene,
                    FxOp::SetSeed {
                        clip: clip.to_string(),
                        seed: Some(seed),
                    },
                    base_revision,
                    source,
                    cx,
                )
            }));
        }
        if let Some(rest) = field.strip_prefix(KEY_FRAME) {
            let result = (|| {
                let (param, from) = rest.split_once(':').ok_or_else(|| bad_action(field))?;
                entry.clip.effect.param(param).ok_or_else(|| {
                    CommandError::new(
                        "unknown_param",
                        format!("`{param}` is not a parameter of this effect"),
                    )
                })?;
                Ok(FxOp::MoveKey {
                    clip: clip.to_string(),
                    param: param.to_string(),
                    from: frame_part(from)?,
                    to: raw.integer()?,
                })
            })();
            return Some(
                result
                    .and_then(|op| self.fx_run_guarded(&view.scene, op, base_revision, source, cx)),
            );
        }
        if let Some(param) = field.strip_prefix(SHIFT_KEYS) {
            let result = (|| {
                entry.clip.effect.param(param).ok_or_else(|| {
                    CommandError::new(
                        "unknown_param",
                        format!("`{param}` is not a parameter of this effect"),
                    )
                })?;
                Ok(FxOp::ShiftKeys {
                    clip: clip.to_string(),
                    param: Some(param.to_string()),
                    delta: raw.integer()?,
                })
            })();
            return Some(
                result
                    .and_then(|op| self.fx_run_guarded(&view.scene, op, base_revision, source, cx)),
            );
        }
        let rest = field.strip_prefix(KEY_PREFIX)?;
        let result = (|| {
            let (control, frame) = rest.split_once(':').ok_or_else(|| bad_action(field))?;
            let frame = frame_part(frame)?;
            let controls = controls_in(&view, &entry.clip);
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
        Some(result.and_then(|op| self.fx_run_guarded(&view.scene, op, base_revision, source, cx)))
    }

    /// A click on an animation button: the action of the clip node it stands for, built on the
    /// project revision the panel was painted at.
    pub(super) fn fx_animation_click(
        &mut self,
        clip: &str,
        name: String,
        revision: u64,
        cx: &mut Context<Self>,
    ) {
        if let Some(Err(error)) =
            self.fx_animation_invoke_guarded(clip, &name, Some(revision), Source::Gesture, cx)
        {
            self.scene_report(error, cx);
        }
    }

    /// A switch of a boolean parameter: one `set_param`, one undo entry. While keys prevail over
    /// the parameter the switch is the BASE and is disabled (the keys' switches edit the curve).
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
        let label = if control.animated {
            format!("{} (base)", control.label(LOCALE))
        } else {
            control.label(LOCALE).to_string()
        };
        let element = format!("fx:{}:{}", clip.id, control.id);
        let toggle = Switch::new(SharedString::from(format!("fx-switch:{element}")))
            .checked(control.value != 0.0)
            .disabled(control.animated)
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
            .text_color(if control.animated { t.faint } else { t.text })
            .child(div().child(label))
            .child(
                div()
                    .relative()
                    .child(self.geometry.probe(format!("control:{element}")))
                    .child(toggle),
            )
    }

    /// The `seed` stepper: one more or one less than the base shown (wrapping inside uint32 is
    /// refused, not wrapped), written as an explicit seed.
    pub(super) fn fx_seed_step(&mut self, clip: &str, delta: i64, cx: &mut Context<Self>) {
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
}

pub(super) fn value_text(value: FxValue) -> String {
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

#[cfg(test)]
mod capture_tests {
    use super::*;
    use crate::workstation::scene_fx_tests::{invoke, mounted, undo_depth};
    use core::prelude::v1::test;

    #[gpui_kit::test]
    fn capture_without_a_monitor_frame_reports_no_frame(cx: &mut TestAppContext) {
        let (view, cx) = mounted(cx);
        invoke(&view, cx, "scene.fx", "add:filmGrain").unwrap();
        let depth = undo_depth(&view, cx);
        // Exercise the adapter's absent-frame view directly: an out-of-edit playhead still
        // has an honest (unclamped) bundle frame, so it is the other refusal, not this case.
        let error = view.read_with(cx, |workstation, _| {
            let mut state = workstation.fx_view().unwrap();
            state.frame_here = None;
            let entry = state.clip("fx-1").unwrap();
            workstation
                .fx_animation_op(&state, entry, "key:amount")
                .unwrap()
                .unwrap_err()
        });
        assert_eq!(error.code, "no_frame");
        assert_eq!(undo_depth(&view, cx), depth);
    }
}
