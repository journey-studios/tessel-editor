//! The typed editors of the key controls of an effect clip (F058, BL082.30): the frame and the value
//! of every key, the seed, the shift and scale of a curve, and the buttons that go from key to key.
//!
//! Like the rest of the "Efeitos" section this file owns no document. What an editor shows is the
//! core's [`ClipAnimation`]; what it does is one op of `scene.fx.edit` (or one `scene.fx.seek`)
//! through the handlers the clip node's `set` and `invoke` use, so a typed value, a click and a
//! script are the same command with the same typed refusal and ONE undo entry.
//!
//! An [`Editor`] is a Kit `Input` plus the binding it was built on:
//!
//! * the **scene, clip, parameter and key frame** are in its [`EditorKey`]; a commit looks them up
//!   again in the document and refuses (`stale_scene`, `unknown_element`, `unknown_keyframe`) a
//!   target that is gone;
//! * the **project revision** it last saw clean is sent as `base_revision`: an edit built on a
//!   revision that moved (an undo, a script, another key) is refused with `stale_revision` and
//!   writes nothing. While the person is typing the binding does not follow the model, which is
//!   exactly what makes the refusal possible;
//! * **Enter or leaving the field** commits ONCE: the text is marked settled before the command
//!   runs, so the `Blur` that follows an Enter (or the re-render in between) finds nothing to do;
//! * a text that is not a number, a whole number or a `uint32` is a typed error on screen and
//!   writes nothing (never a 0); the field then shows the model's value again;
//! * the siblings of a key's other components (the `y` of a point, the `g` and `b` of a colour)
//!   and its easing are the key's own, never the base's (the core's `key_op` reads the key).
//!
//! The shift and scale rows are drafts: their texts follow their defaults until edited and are
//! applied by their button (or Enter), never by leaving the field.

use std::collections::HashMap;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use serde_json::Value;
use tessel_editor_core::directed_scene::fx::{FX_EASINGS, FxClip};
use tessel_editor_core::scene_edit::{ClipAnimation, FieldKind, ParamAnimation};

use super::controls::{disabled_tool_button, tool_button};
use super::scene_fx::{ClipView, FxView, LOCALE, controls_in};
use super::scene_fx_keys::{
    EASING, GO, KEY_FRAME, KEY_PREFIX, MOVE_KEY, REMOVE_ANIMATION, REMOVE_KEY, SCALE_KEYS,
    SEED_AUTO, SEED_FIELD, SHIFT_KEYS, animation_at, reason_text, value_text,
};
use super::scene_model::format_number;
use super::tokens::Tokens;
use super::{Source, Workstation};
use crate::state::{CommandError, Raw};

/// What one editor edits.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum EditorField {
    Seed,
    /// The frame of the key of `param` now at `frame` of the clip (a commit moves the key).
    KeyFrame {
        param: String,
        frame: i64,
    },
    /// One component (`control`: `center_x`, `color_g`...) of the key at `frame`.
    KeyValue {
        control: String,
        frame: i64,
    },
    Shift {
        param: String,
    },
    ScaleAnchor {
        param: String,
    },
    ScaleNum {
        param: String,
    },
    ScaleDen {
        param: String,
    },
}

impl EditorField {
    /// A draft is applied by its button, never by leaving the field.
    fn is_draft(&self) -> bool {
        matches!(
            self,
            Self::Shift { .. }
                | Self::ScaleAnchor { .. }
                | Self::ScaleNum { .. }
                | Self::ScaleDen { .. }
        )
    }

    fn tag(&self) -> String {
        match self {
            Self::Seed => "seed".to_string(),
            Self::KeyFrame { param, frame } => format!("keyframe:{param}:{frame}"),
            Self::KeyValue { control, frame } => format!("keyvalue:{control}:{frame}"),
            Self::Shift { param } => format!("shift:{param}"),
            Self::ScaleAnchor { param } => format!("scale-anchor:{param}"),
            Self::ScaleNum { param } => format!("scale-num:{param}"),
            Self::ScaleDen { param } => format!("scale-den:{param}"),
        }
    }
}

/// Which editor of which clip of which scene.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct EditorKey {
    pub(super) scene: String,
    pub(super) clip: String,
    pub(super) field: EditorField,
}

impl EditorKey {
    /// Element and probe ID (`control:<this>`); also what the tests use to find the editor.
    pub(super) fn element_id(&self) -> String {
        format!("fx-editor:{}:{}", self.clip, self.field.tag())
    }

    /// Whether the document still holds what this editor is bound to.
    fn exists_in(&self, view: &FxView) -> bool {
        if view.scene != self.scene {
            return false;
        }
        let Some(entry) = view.clip(&self.clip) else {
            return false;
        };
        let clip = &entry.clip;
        let param_ok = |param: &str| clip.effect.param(param).is_some();
        match &self.field {
            EditorField::Seed => animation_at(view, clip).seed.is_some(),
            EditorField::KeyFrame { param, frame } => has_key(clip, param, *frame),
            EditorField::KeyValue { control, frame } => controls_in(view, clip)
                .iter()
                .find(|candidate| candidate.id == *control)
                .and_then(|candidate| candidate.param())
                .is_some_and(|param| has_key(clip, param, *frame)),
            EditorField::Shift { param }
            | EditorField::ScaleAnchor { param }
            | EditorField::ScaleNum { param }
            | EditorField::ScaleDen { param } => param_ok(param),
        }
    }
}

fn has_key(clip: &FxClip, param: &str, frame: i64) -> bool {
    clip.effect
        .param(param)
        .and_then(|(index, _)| clip.curve(index))
        .is_some_and(|keys| keys.iter().any(|key| key.frame == frame))
}

/// A Kit input with the text the model last gave it and the revision that text was clean at.
pub(super) struct Editor {
    input: Entity<InputState>,
    /// The model's text last pushed into the input.
    shown: String,
    /// The text already committed (or refused): leaving the field with it again does nothing, and
    /// the next paint puts the model's text back.
    settled: Option<String>,
    /// The project revision at the last paint where the text was not being edited.
    revision: u64,
}

impl Editor {
    fn dirty(&self, text: &str) -> bool {
        text != self.shown && self.settled.as_deref() != Some(text)
    }
}

/// The editors of the section, by what they edit.
pub(super) type Editors = HashMap<EditorKey, Editor>;

fn no_scene() -> CommandError {
    CommandError::new("no_scene", "No scene is attached")
}

impl Workstation {
    /// The input of an editor, built the first time and kept in step with `model` after: while the
    /// person is typing it keeps the draft (and the revision it was clean at), otherwise it shows
    /// the model.
    fn fx_editor(
        &mut self,
        key: EditorKey,
        model: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        let revision = self.state.project.revision;
        if !self.scene_ui.fx.editors.contains_key(&key) {
            let input = cx.new(|cx| InputState::new(window, cx));
            let bound = key.clone();
            cx.subscribe_in(
                &input,
                window,
                move |this, _input, event: &InputEvent, _window, cx| match event {
                    InputEvent::PressEnter { .. } => this.fx_editor_commit(&bound, true, cx),
                    InputEvent::Blur => this.fx_editor_commit(&bound, false, cx),
                    _ => {}
                },
            )
            .detach();
            // `set_value` does not emit `Change`/`Blur`, so this cannot commit.
            input.update(cx, |input, cx| input.set_value(model.clone(), window, cx));
            self.scene_ui.fx.editors.insert(
                key.clone(),
                Editor {
                    input,
                    shown: model.clone(),
                    settled: None,
                    revision,
                },
            );
        }
        let Some(editor) = self.scene_ui.fx.editors.get_mut(&key) else {
            return cx.new(|cx| InputState::new(window, cx));
        };
        let input = editor.input.clone();
        let text = input.read(cx).value().to_string();
        if text == model {
            editor.shown = model;
            editor.settled = None;
            editor.revision = revision;
        } else if !editor.dirty(&text) {
            editor.shown = model.clone();
            editor.settled = None;
            editor.revision = revision;
            input.update(cx, |input, cx| input.set_value(model, window, cx));
        }
        input
    }

    /// Drops the editors whose target the document no longer holds (a key removed, a clip gone).
    pub(super) fn fx_editors_prune(&mut self, view: &FxView) {
        self.scene_ui
            .fx
            .editors
            .retain(|key, _| key.exists_in(view));
    }

    /// Enter or leaving the field of an editor.
    pub(super) fn fx_editor_commit(
        &mut self,
        key: &EditorKey,
        enter: bool,
        cx: &mut Context<Self>,
    ) {
        if key.field.is_draft() {
            if enter {
                self.fx_draft_apply(key, cx);
            }
            return;
        }
        let Some(editor) = self.scene_ui.fx.editors.get_mut(key) else {
            return;
        };
        let text = editor.input.read(cx).value().to_string();
        if !editor.dirty(&text) {
            return;
        }
        // Settled BEFORE the command runs: the Blur that follows an Enter finds nothing to do.
        editor.settled = Some(text.clone());
        let revision = editor.revision;
        if let Err(error) = self.fx_editor_apply(key, &text, revision, cx) {
            self.scene_report(error, cx);
        }
        cx.notify();
    }

    /// The commit of an editor's text: the `set` of the clip node's field that the editor stands
    /// for, built on `revision`. A binding the document no longer holds is refused first.
    pub(super) fn fx_editor_apply(
        &mut self,
        key: &EditorKey,
        text: &str,
        revision: u64,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        let view = self.fx_document_view().ok_or_else(no_scene)?;
        if view.scene != key.scene {
            return Err(CommandError::new(
                "stale_scene",
                format!(
                    "The editor was built on scene `{}`, the panel follows `{}`",
                    key.scene, view.scene
                ),
            ));
        }
        if view.clip(&key.clip).is_none() {
            return Err(CommandError::unknown_element("effect clip", &key.clip));
        }
        if !key.exists_in(&view) {
            return Err(CommandError::new(
                "unknown_keyframe",
                "The key this editor was built on is not in the document any more; nothing written",
            ));
        }
        let field = match &key.field {
            EditorField::Seed => SEED_FIELD.to_string(),
            EditorField::KeyFrame { param, frame } => format!("{KEY_FRAME}{param}:{frame}"),
            EditorField::KeyValue { control, frame } => format!("{KEY_PREFIX}{control}:{frame}"),
            _ => {
                return Err(CommandError::new(
                    "not_applicable",
                    "A draft is applied by its button",
                ));
            }
        };
        self.fx_animation_set_guarded(
            &key.clip,
            &field,
            &Raw::Text(text.to_string()),
            Some(revision),
            Source::Gesture,
            cx,
        )
        .unwrap_or_else(|| Err(CommandError::unknown_element("effect clip", &key.clip)))
    }

    /// The text an editor of `key` holds now (tests and the semantic surface read it).
    #[cfg(test)]
    pub(super) fn fx_editor_text(&self, key: &EditorKey, cx: &App) -> Option<String> {
        self.scene_ui
            .fx
            .editors
            .get(key)
            .map(|editor| editor.input.read(cx).value().to_string())
    }

    /// The input entity of an editor (tests type into it like a person).
    #[cfg(test)]
    pub(super) fn fx_editor_input(&self, key: &EditorKey) -> Option<Entity<InputState>> {
        self.scene_ui
            .fx
            .editors
            .get(key)
            .map(|editor| editor.input.clone())
    }

    /// Applies the shift or scale draft of `param`: from the texts of its editors, through the same
    /// handlers as `set shift_keys:<param>` and `invoke scale_keys:<param>:<a>:<n>:<d>`.
    fn fx_draft_apply(&mut self, key: &EditorKey, cx: &mut Context<Self>) {
        let (scene, clip) = (key.scene.clone(), key.clip.clone());
        let param = match &key.field {
            EditorField::Shift { param }
            | EditorField::ScaleAnchor { param }
            | EditorField::ScaleNum { param }
            | EditorField::ScaleDen { param } => param.clone(),
            _ => return,
        };
        let editor_key = |field: EditorField| EditorKey {
            scene: scene.clone(),
            clip: clip.clone(),
            field,
        };
        let shift = matches!(key.field, EditorField::Shift { .. });
        let wanted: Vec<EditorKey> = if shift {
            vec![editor_key(EditorField::Shift {
                param: param.clone(),
            })]
        } else {
            vec![
                editor_key(EditorField::ScaleAnchor {
                    param: param.clone(),
                }),
                editor_key(EditorField::ScaleNum {
                    param: param.clone(),
                }),
                editor_key(EditorField::ScaleDen {
                    param: param.clone(),
                }),
            ]
        };
        let mut texts = Vec::new();
        let mut revision = u64::MAX;
        for each in &wanted {
            let Some(editor) = self.scene_ui.fx.editors.get(each) else {
                return;
            };
            texts.push(editor.input.read(cx).value().to_string());
            revision = revision.min(editor.revision);
        }
        let result = (|| -> Result<Value, CommandError> {
            let view = self.fx_document_view().ok_or_else(no_scene)?;
            if view.scene != scene {
                return Err(CommandError::new(
                    "stale_scene",
                    format!(
                        "The editor was built on scene `{scene}`, the panel follows `{}`",
                        view.scene
                    ),
                ));
            }
            if shift {
                let field = format!("{SHIFT_KEYS}{param}");
                return self
                    .fx_animation_set_guarded(
                        &clip,
                        &field,
                        &Raw::Text(texts[0].clone()),
                        Some(revision),
                        Source::Gesture,
                        cx,
                    )
                    .unwrap_or_else(|| Err(CommandError::unknown_element("effect clip", &clip)));
            }
            let mut numbers = Vec::new();
            for text in &texts {
                numbers.push(Raw::Text(text.clone()).integer()?);
            }
            let name = format!(
                "{SCALE_KEYS}{param}:{}:{}:{}",
                numbers[0], numbers[1], numbers[2]
            );
            self.fx_animation_invoke_guarded(&clip, &name, Some(revision), Source::Gesture, cx)
                .unwrap_or_else(|| Err(CommandError::unknown_element("effect clip", &clip)))
        })();
        // The drafts go back to their defaults at the next paint, whatever the result.
        for (each, text) in wanted.iter().zip(texts) {
            if let Some(editor) = self.scene_ui.fx.editors.get_mut(each) {
                editor.settled = Some(text);
            }
        }
        if let Err(error) = result {
            self.scene_report(error, cx);
        }
        cx.notify();
    }

    /// The seed, the keys of every parameter and the shift/scale of their curves, below the sliders
    /// of a clip.
    pub(super) fn fx_animation_block(
        &mut self,
        t: &Tokens,
        view: &FxView,
        entry: &ClipView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let clip = &entry.clip;
        let animation = animation_at(view, clip);
        let mut block = v_flex().w_full();
        // The seed first: it is one line, and the keys below it can be many.
        if let Some(seed) = &animation.seed {
            block = block.child(self.fx_seed_row(
                t,
                view,
                clip,
                seed.base,
                seed.explicit.is_some(),
                window,
                cx,
            ));
        }
        for param in &animation.params {
            block = block.child(self.fx_param_keys(t, view, clip, &animation, param, window, cx));
        }
        block
    }

    /// A Kit input of an editor, with its probe, at a fixed width.
    fn fx_editor_box(
        &mut self,
        key: EditorKey,
        model: String,
        width: f32,
        label: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let probe_id = format!("control:{}", key.element_id());
        let input = self.fx_editor(key, model, window, cx);
        let probe = self.geometry.probe(probe_id);
        div()
            .relative()
            .w(px(width))
            .flex_none()
            .child(probe)
            .child(Input::new(&input).small().aria_label(label))
    }

    // WHY the long list: a button is built from its parts (what it is called, what it does, why it
    // cannot, the revision it was painted at) and a struct for one call site would only rename the
    // arguments.
    #[allow(clippy::too_many_arguments)]
    fn fx_icon_button(
        &mut self,
        id: String,
        icon: IconName,
        label: String,
        reason: Option<&str>,
        clip: &str,
        action: String,
        revision: u64,
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
                        this.fx_animation_click(&clip, action, revision, cx);
                    });
                })
            }
            Some(reason) => disabled_tool_button(button_id, icon, &label, reason),
        };
        div().relative().child(probe).child(button)
    }

    #[allow(clippy::too_many_arguments)]
    fn fx_param_keys(
        &mut self,
        t: &Tokens,
        view: &FxView,
        clip: &FxClip,
        animation: &ClipAnimation,
        param: &ParamAnimation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let id = param.spec.id;
        let label = param.spec.label.get(LOCALE);
        let revision = self.state.project.revision;
        let capture_reason = match animation.frame_in_clip {
            Some(_) => None,
            None if animation.edit_frame.is_none() => Some(reason_text("no_frame")),
            None => Some(reason_text("frame_outside_clip")),
        };
        // The value the monitor shows now, apart from the base the slider holds.
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
            revision,
            cx,
        ));
        head = head.child(self.fx_icon_button(
            format!("fx-unkey:{}:{id}", clip.id),
            IconName::Trash,
            format!("Remover animação de {label}"),
            (!param.animated).then(|| reason_text("no_keys")),
            &clip.id,
            format!("{REMOVE_ANIMATION}{id}"),
            revision,
            cx,
        ));
        let mut rows = v_flex().w_full().child(head);
        if !param.animated {
            return rows;
        }

        // Navigation: where the monitor goes among the keys of this parameter (the core's seek).
        let mut nav = h_flex()
            .items_center()
            .gap(px(2.))
            .pl(px(18.))
            .pr(px(10.))
            .text_size(px(10.))
            .text_color(t.faint)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .child("Ir para a chave"),
            );
        for (to, icon, word, available, why) in [
            ("first", IconName::SkipBack, "primeira", true, "no_keys"),
            (
                "previous",
                IconName::ChevronLeft,
                "anterior",
                param.previous.is_some(),
                if animation.edit_frame.is_none() {
                    "no_frame"
                } else {
                    "no_previous_key"
                },
            ),
            (
                "next",
                IconName::ChevronRight,
                "próxima",
                param.next.is_some(),
                if animation.edit_frame.is_none() {
                    "no_frame"
                } else {
                    "no_next_key"
                },
            ),
            ("last", IconName::SkipForward, "última", true, "no_keys"),
        ] {
            nav = nav.child(self.fx_icon_button(
                format!("fx-go:{}:{id}:{to}", clip.id),
                icon,
                format!("Ir para a chave {word} de {label}"),
                (!available).then(|| reason_text(why)),
                &clip.id,
                format!("{GO}{id}:{to}"),
                revision,
                cx,
            ));
        }
        rows = rows.child(nav);

        let controls: Vec<_> = controls_in(view, clip)
            .into_iter()
            .filter(|control| control.param() == Some(id))
            .collect();
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
            // Line 1: the key's frame and the value of each of its components, typed.
            let scope = |field| EditorKey {
                scene: view.scene.clone(),
                clip: clip.id.clone(),
                field,
            };
            let mut values = h_flex()
                .items_center()
                .gap(px(3.))
                .pl(px(18.))
                .pr(px(10.))
                .text_size(px(10.))
                .text_color(t.text)
                .child(div().child("q"))
                .child(self.fx_editor_box(
                    scope(EditorField::KeyFrame {
                        param: id.to_string(),
                        frame,
                    }),
                    frame.to_string(),
                    44.,
                    format!("Quadro da chave de {label} em q{frame}"),
                    window,
                    cx,
                ));
            for control in &controls {
                let Some(shown) = control.read(key.value) else {
                    continue;
                };
                if control.kind == FieldKind::Boolean {
                    let (workstation, clip_id, control_id) =
                        (cx.weak_entity(), clip.id.clone(), control.id.clone());
                    values = values.child(
                        Switch::new(SharedString::from(format!(
                            "fx-keyswitch:{}:{}:{frame}",
                            clip.id, control.id
                        )))
                        .checked(shown != 0.0)
                        .accessibility_label(format!("{label} em q{frame}"))
                        .on_click(
                            move |checked: &bool, _, cx: &mut App| {
                                let (clip_id, control_id, on) =
                                    (clip_id.clone(), control_id.clone(), *checked);
                                let _ = workstation.update(cx, |this, cx| {
                                    let field = format!("{KEY_PREFIX}{control_id}:{frame}");
                                    if let Some(Err(error)) = this.fx_animation_set_guarded(
                                        &clip_id,
                                        &field,
                                        &Raw::Bool(on),
                                        Some(revision),
                                        Source::Gesture,
                                        cx,
                                    ) {
                                        this.scene_report(error, cx);
                                    }
                                });
                            },
                        ),
                    );
                    continue;
                }
                values = values.child(self.fx_editor_box(
                    scope(EditorField::KeyValue {
                        control: control.id.clone(),
                        frame,
                    }),
                    format_number(shown),
                    52.,
                    format!("{} da chave em q{frame}", control.label(LOCALE)),
                    window,
                    cx,
                ));
            }
            rows = rows.child(values);
            // Line 2: go to the key, nudge it a frame, change its curve, remove it.
            let mut buttons = h_flex()
                .items_center()
                .gap(px(2.))
                .pl(px(18.))
                .pr(px(10.))
                .text_size(px(10.))
                .text_color(t.faint)
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
            buttons = buttons.child(self.fx_icon_button(
                format!("fx-keygo:{}:{id}:{frame}", clip.id),
                IconName::Crosshair,
                format!("Ir para a chave de {label} em q{frame}"),
                None,
                &clip.id,
                format!("{GO}{id}:{frame}"),
                revision,
                cx,
            ));
            for (delta, icon, word) in [
                (-1_i64, IconName::ChevronLeft, "para trás"),
                (1, IconName::ChevronRight, "para frente"),
            ] {
                let to = frame + delta;
                buttons = buttons.child(self.fx_icon_button(
                    format!("fx-keymove:{}:{id}:{frame}:{to}", clip.id),
                    icon,
                    format!("Mover chave de {label} {word} (q{frame} → q{to})"),
                    (!can_move(to)).then(|| reason_text("key_order")),
                    &clip.id,
                    format!("{MOVE_KEY}{id}:{frame}:{to}"),
                    revision,
                    cx,
                ));
            }
            buttons = buttons.child(self.fx_icon_button(
                format!("fx-keyease:{}:{id}:{frame}", clip.id),
                IconName::Replace,
                format!(
                    "Curva da chave em q{frame}: {} (trocar para {next_easing})",
                    key.easing.name()
                ),
                None,
                &clip.id,
                format!("{EASING}{id}:{frame}:{next_easing}"),
                revision,
                cx,
            ));
            buttons = buttons.child(self.fx_icon_button(
                format!("fx-keyrm:{}:{id}:{frame}", clip.id),
                IconName::Close,
                format!("Remover chave de {label} em q{frame}"),
                None,
                &clip.id,
                format!("{REMOVE_KEY}{id}:{frame}"),
                revision,
                cx,
            ));
            rows = rows.child(buttons);
        }

        // The curve as a whole: shift every key, or scale their distance from an anchor.
        let scope = |field| EditorKey {
            scene: view.scene.clone(),
            clip: clip.id.clone(),
            field,
        };
        let first = param.keys.first().map_or(0, |key| key.frame);
        let shift_key = scope(EditorField::Shift {
            param: id.to_string(),
        });
        let mut shift = h_flex()
            .items_center()
            .gap(px(3.))
            .pl(px(18.))
            .pr(px(10.))
            .text_size(px(10.))
            .text_color(t.text)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .child("Deslocar chaves"),
            )
            .child(self.fx_editor_box(
                shift_key.clone(),
                "0".to_string(),
                44.,
                format!("Deslocamento das chaves de {label} em quadros"),
                window,
                cx,
            ));
        shift = shift.child(self.fx_draft_button(
            format!("fx-shift:{}:{id}", clip.id),
            format!("Aplicar deslocamento às chaves de {label}"),
            None,
            shift_key,
            cx,
        ));
        rows = rows.child(shift);
        let anchor_key = scope(EditorField::ScaleAnchor {
            param: id.to_string(),
        });
        let mut scale = h_flex()
            .items_center()
            .gap(px(3.))
            .pl(px(18.))
            .pr(px(10.))
            .text_size(px(10.))
            .text_color(t.text)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .child("Escalar (âncora, num, den)"),
            );
        for (field, model, word) in [
            (
                EditorField::ScaleAnchor {
                    param: id.to_string(),
                },
                first.to_string(),
                "Âncora",
            ),
            (
                EditorField::ScaleNum {
                    param: id.to_string(),
                },
                "1".to_string(),
                "Numerador",
            ),
            (
                EditorField::ScaleDen {
                    param: id.to_string(),
                },
                "1".to_string(),
                "Denominador",
            ),
        ] {
            scale = scale.child(self.fx_editor_box(
                scope(field),
                model,
                38.,
                format!("{word} da escala das chaves de {label}"),
                window,
                cx,
            ));
        }
        scale = scale.child(self.fx_draft_button(
            format!("fx-scale:{}:{id}", clip.id),
            format!("Aplicar escala às chaves de {label}"),
            (param.keys.len() < 2).then(|| reason_text("needs_two_keys")),
            anchor_key,
            cx,
        ));
        rows.child(scale)
    }

    /// The apply button of a shift or scale draft.
    fn fx_draft_button(
        &mut self,
        id: String,
        label: String,
        reason: Option<&str>,
        key: EditorKey,
        cx: &mut Context<Self>,
    ) -> Div {
        let probe = self.geometry.probe(format!("control:{id}"));
        let button_id = SharedString::from(id);
        let button = match reason {
            None => {
                let workstation = cx.weak_entity();
                tool_button(button_id, IconName::Check, &label).on_click(move |_, _, cx| {
                    let key = key.clone();
                    let _ = workstation.update(cx, |this, cx| this.fx_draft_apply(&key, cx));
                })
            }
            Some(reason) => disabled_tool_button(button_id, IconName::Check, &label, reason),
        };
        div().relative().child(probe).child(button)
    }

    /// The seed: its value typed (a `uint32`), stepped by one, or back to automatic.
    #[allow(clippy::too_many_arguments)]
    fn fx_seed_row(
        &mut self,
        t: &Tokens,
        view: &FxView,
        clip: &FxClip,
        base: u32,
        explicit: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let id = clip.id.clone();
        let revision = self.state.project.revision;
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
                    .child(if explicit {
                        "Semente".to_string()
                    } else {
                        "Semente (automática)".to_string()
                    }),
            )
            .child(self.fx_editor_box(
                EditorKey {
                    scene: view.scene.clone(),
                    clip: id.clone(),
                    field: EditorField::Seed,
                },
                base.to_string(),
                84.,
                format!("Semente de {id}"),
                window,
                cx,
            ));
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
        row.child(self.fx_icon_button(
            format!("fx-seed-auto:{id}"),
            IconName::RotateCw,
            format!("Semente automática de {id}"),
            (!explicit).then(|| reason_text("seed_is_automatic")),
            &id,
            SEED_AUTO.to_string(),
            revision,
            cx,
        ))
    }
}
