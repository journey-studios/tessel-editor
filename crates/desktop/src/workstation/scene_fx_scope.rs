//! The scope of an effect clip in the window (F062, `docs/features/fx-shot-scope.md`): the selector
//! on the clip, the scope in its title and the action that adds a clip on the shot the monitor shows.
//!
//! Like the rest of the "Efeitos" section this file owns no document. What the selector offers is
//! the core's [`InspectorChoice`] (the options, the shots they are for, whether the clip fits each
//! one and the refusal code when it does not); what it does is one `set_scope` op of
//! `scene.fx.edit`, through the handlers the clip node's `set` and `invoke` use, so a pick, a click
//! and a script are the same command with the same typed refusal and ONE undo entry. A refusal
//! (`fx_scope_mismatch`, `unknown_shot`, `fx_scope_not_yet`) is the panel's notice; nothing is
//! clamped, nothing is written, and the selector goes back to the scope the document holds.
//!
//! Node surface (beyond the fields of P2/P3):
//!
//! * `scene.fx.clip:<id>`: the field `scope` (`type: "choice"`, value `output` or `shot:<id>`; the
//!   options carry `applicable`/`reason`), one action `set_scope:<scope>` per option (disabled with
//!   its reason when it is the scope the clip holds or the clip does not fit it) and `scope`,
//!   `scope_label`, `shot` in the state;
//! * `scene.fx`: one action `add_in_shot:<effect>` per effect of the catalog, which adds the clip at
//!   the frame the monitor shows with the scope of the shot that holds that frame (disabled, with
//!   its reason, when the monitor shows no frame of the edit), and `shots`/`shot_here` in the
//!   state. The clip lasts the effect's typical duration, cut at the end of the shot: the window
//!   picks that length, the core never does.

use std::collections::HashMap;

use gpui_kit::assets::IconName;
use gpui_kit::component::searchable_list::SearchableVec;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{Sizable, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use serde_json::{Value, json};
use tessel_editor_core::directed_scene::Shot;
use tessel_editor_core::directed_scene::fx::{
    self, FX_SCOPE_MISMATCH, FX_SCOPE_NOT_YET, FxClip, FxScope, UNKNOWN_SHOT, shot_at, shots_json,
};
use tessel_editor_core::scene_edit::{
    AddClip, ChoiceOption, FxOp, InspectorChoice, OUTPUT_LABEL, SHOT_LABEL, inspector_choices,
    offers, scopes,
};

use super::controls::{disabled_tool_button, tool_button};
use super::scene_fx::{CLIP_PREFIX, ClipView, FxView, LOCALE, SECTION_ID};
use super::tokens::Tokens;
use super::{Source, Workstation};
use crate::state::{CommandError, Raw};

/// `set_scope:<scope>` on a clip node lays the clip on `<scope>` (`output`, `shot:<id>`).
pub(super) const SET_SCOPE_PREFIX: &str = "set_scope:";
/// `add_in_shot:<effect>` on the section adds a clip of `effect` on the shot the monitor shows.
pub(super) const ADD_IN_SHOT_PREFIX: &str = "add_in_shot:";
/// The node field of the scope.
pub(super) const SCOPE_FIELD: &str = "scope";

const ALREADY_HERE: &str = "O efeito já está neste escopo";
const OUTSIDE_THE_EDIT: &str = "O monitor não mostra um quadro da edição desta cena";
const NOT_IN_A_SHOT: &str = "O quadro exibido está fora da edição: nenhum plano o contém";

// ---- the model: what the window shows, with no GPUI ----

/// The words of a scope in the title of a clip: `Saída`, `Plano s1`.
pub(super) fn scope_title(scope: &FxScope) -> String {
    match scope {
        FxScope::Output => OUTPUT_LABEL.get(LOCALE).to_string(),
        FxScope::Shot(id) => format!("{} {id}", SHOT_LABEL.get(LOCALE)),
    }
}

/// The text of an option in the selector: `Saída`, `Plano s1 · 0–47` (a shot adds the frames it
/// covers, so two shots are never told apart by the id alone).
fn option_text(option: &ChoiceOption) -> String {
    match option.frames {
        Some((start, end)) => format!(
            "{} · {start}–{}",
            option.label(LOCALE),
            end.saturating_sub(1)
        ),
        None => option.label(LOCALE),
    }
}

/// The words of a typed reason a clip cannot be laid on an option.
fn reason_text(code: &str) -> &'static str {
    match code {
        FX_SCOPE_MISMATCH => {
            "O clipe não cabe inteiro neste plano: ajuste o início e a duração antes"
        }
        UNKNOWN_SHOT => "Este plano não existe na edição",
        FX_SCOPE_NOT_YET => "Este escopo ainda não é desenhado",
        _ => "Não se aplica a este clipe",
    }
}

/// The scopes the build draws, by name (`output`, `shot`): the state of the section.
pub(super) fn drawn_scopes() -> Vec<&'static str> {
    scopes()
        .into_iter()
        .filter(|(_, drawn, _)| *drawn)
        .map(|(name, _, _)| name)
        .collect()
}

/// The scope choice of `clip` against the shots of the view.
fn choice_of(view: &FxView, clip: &FxClip) -> InspectorChoice {
    inspector_choices(clip, &view.shots).remove(0)
}

/// The names of the `set_scope:<scope>` actions of a clip node, in the order of its options: what an
/// `unknown_action` hint advertises beyond the base actions, so it lists what the node lists.
pub(super) fn scope_action_names(view: &FxView, clip: &FxClip) -> Vec<String> {
    choice_of(view, clip)
        .options
        .iter()
        .map(|option| format!("{SET_SCOPE_PREFIX}{}", option.value))
        .collect()
}

/// The shot the monitor shows, or why none: where `add_in_shot` lands.
fn shot_here(view: &FxView) -> Result<&Shot, &'static str> {
    let frame = view.frame_here.ok_or(OUTSIDE_THE_EDIT)?;
    shot_at(&view.shots, frame).ok_or(NOT_IN_A_SHOT)
}

/// What the catalog offers on the shot the monitor shows: enabled when the effect can be added and
/// the monitor is in a shot; otherwise disabled with the reason, never a callback that does nothing.
fn in_shot_state(
    view: &FxView,
    applicable: bool,
    offer_reason: Option<&str>,
) -> (bool, Option<String>) {
    if !applicable {
        return (false, offer_reason.map(str::to_string));
    }
    match shot_here(view) {
        Ok(_) => (true, None),
        Err(reason) => (false, Some(reason.to_string())),
    }
}

/// Adds the scope to a clip node built by `fx_nodes`: the `scope` choice field, one `set_scope:`
/// action per option and the scope in the state. The options are the very JSON `scene.fx.inspect`
/// answers (one model, so the window and the library cannot disagree).
pub(super) fn decorate_clip_node(node: &mut Value, view: &FxView, entry: &ClipView) {
    let clip = &entry.clip;
    let choice = choice_of(view, clip);
    let mut field = choice.to_json(&clip.id, LOCALE);
    field["field"] = json!(SCOPE_FIELD);
    field["label"] = json!(format!(
        "{} {}",
        clip.effect.label.get(LOCALE),
        choice.label(LOCALE)
    ));
    field["enabled"] = json!(true);
    field["disabled_reason"] = Value::Null;
    if let Some(fields) = node["fields"].as_array_mut() {
        fields.push(field);
    }
    if let Some(actions) = node["actions"].as_array_mut() {
        for option in &choice.options {
            let selected = option.value == choice.value;
            let enabled = option.applicable && !selected;
            let reason = if selected {
                Some(ALREADY_HERE)
            } else {
                option.reason.map(reason_text)
            };
            actions.push(json!({
                "action": format!("{SET_SCOPE_PREFIX}{}", option.value),
                "label": format!("Mover para {}", option_text(option)),
                "enabled": enabled,
                "disabled_reason": reason.filter(|_| !enabled),
            }));
        }
    }
    node["state"]["scope"] = json!(clip.scope.to_text());
    node["state"]["scope_label"] = json!(scope_title(&clip.scope));
    node["state"]["shot"] = json!(clip.scope.shot_id());
}

/// Adds the shots and the `add_in_shot:<effect>` actions to the `scene.fx` section node.
pub(super) fn decorate_section_node(node: &mut Value, view: &FxView) {
    if let Some(actions) = node["actions"].as_array_mut() {
        for offer in offers() {
            let spec = offer.effect;
            let (enabled, reason) = in_shot_state(view, offer.applicable, offer.reason);
            actions.push(json!({
                "action": format!("{ADD_IN_SHOT_PREFIX}{}", spec.id),
                "label": format!(
                    "Adicionar {} no plano exibido",
                    spec.label.get(LOCALE).to_lowercase()
                ),
                "enabled": enabled,
                "disabled_reason": reason,
            }));
        }
    }
    node["state"]["shots"] = shots_json(&view.shots);
    node["state"]["shot_here"] = json!(shot_here(view).ok().map(|shot| shot.id.clone()));
}

// ---- the selector: Kit state per clip ----

/// Which clip's selector.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct ScopeKey {
    scene: String,
    clip: String,
}

/// A Kit `Select` with the `(scope, text)` pairs it was last given.
pub(super) struct ScopePick {
    select: Entity<SelectState<SearchableVec<String>>>,
    options: Vec<(String, String)>,
}

/// The selectors of the section, by clip.
pub(super) type ScopePicks = HashMap<ScopeKey, ScopePick>;

fn no_scene() -> CommandError {
    CommandError::new("unknown_element", super::scene_ui::NO_SCENE_REASON)
}

#[cfg(test)]
impl Workstation {
    /// The select of `clip` and the `(scope, text)` pairs it lists: what the tests pick through.
    pub(super) fn scope_select_of(
        &self,
        scene: &str,
        clip: &str,
    ) -> Option<(
        Entity<SelectState<SearchableVec<String>>>,
        Vec<(String, String)>,
    )> {
        let pick = self.scene_ui.fx.scopes.get(&ScopeKey {
            scene: scene.to_string(),
            clip: clip.to_string(),
        })?;
        Some((pick.select.clone(), pick.options.clone()))
    }
}

fn with_target(error: CommandError, target: &str) -> CommandError {
    if error.target.is_none() {
        error.target(target)
    } else {
        error
    }
}

impl Workstation {
    // ---- writes ----

    /// `set_scope` on `clip` with the text of a scope: ONE `scene.fx.edit`, one history entry. The
    /// text that is no scope, a `node` scope and a shot that is missing or does not hold the whole
    /// clip are typed refusals with nothing written; the scope the clip holds is the core's no-op.
    pub(super) fn fx_set_scope(
        &mut self,
        clip: &str,
        scope: &str,
        source: Source,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        let view = self.fx_view().ok_or_else(no_scene)?;
        let found = view
            .clip(clip)
            .ok_or_else(|| CommandError::unknown_element("effect clip", clip))?;
        let op = match choice_of(&view, &found.clip).op(&found.clip, scope) {
            Ok(op) => op,
            Err(error) => {
                // It never reached a command: the notice is ours to show.
                self.scene_report(error.clone(), cx);
                return Err(error);
            }
        };
        self.fx_run(&view.scene, op, source, cx)
    }

    /// Adds a clip of `effect` at the frame the monitor shows, on the shot that holds it: ONE
    /// `scene.fx.edit`. The clip lasts the effect's typical duration, cut at the end of the shot (a
    /// clip must lie inside its shot and the core never shortens one). A monitor outside the edit
    /// is a typed `disabled_element` with nothing written.
    pub(super) fn fx_add_in_shot(
        &mut self,
        effect: &str,
        source: Source,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        let view = self.fx_view().ok_or_else(no_scene)?;
        let shot = match shot_here(&view) {
            Ok(shot) => shot.clone(),
            Err(reason) => {
                let error = CommandError::disabled(reason).target(SECTION_ID);
                self.scene_report(error.clone(), cx);
                return Err(error);
            }
        };
        let frame = view.frame_here.unwrap_or_default();
        let end = i64::try_from(shot.end_frame_exclusive).unwrap_or(i64::MAX);
        // An effect the catalog does not have is the core's to refuse (`unknown_effect`).
        let duration = fx::effect(effect).map(|spec| spec.typical_duration_frames.min(end - frame));
        let op = FxOp::AddClip(AddClip {
            track: None,
            clip: None,
            effect: effect.to_string(),
            start_frame: frame,
            start_time: None,
            duration_frames: duration,
            params: Vec::new(),
            scope: FxScope::Shot(shot.id),
        });
        self.fx_run(&view.scene, op, source, cx)
    }

    // ---- automation ----

    /// `invoke` of `set_scope:<scope>` on a clip node or `add_in_shot:<effect>` on the section.
    /// `None` for any other target or action.
    pub(super) fn fx_scope_invoke(
        &mut self,
        target: &str,
        action: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Option<Result<Value, CommandError>> {
        let action = action?;
        if target == SECTION_ID {
            let effect = action.strip_prefix(ADD_IN_SHOT_PREFIX)?;
            return Some(
                self.fx_add_in_shot(effect, Source::Cli, cx)
                    .map_err(|error| with_target(error, target)),
            );
        }
        let clip = target.strip_prefix(CLIP_PREFIX)?;
        let scope = action.strip_prefix(SET_SCOPE_PREFIX)?;
        Some(
            self.fx_set_scope(clip, scope, Source::Cli, cx)
                .map_err(|error| with_target(error, target)),
        )
    }

    /// `set` of the field `scope` on a clip node, with the text of a scope. `None` for any other
    /// target or field.
    pub(super) fn fx_scope_set(
        &mut self,
        target: &str,
        field: &str,
        raw: &Raw,
        cx: &mut Context<Self>,
    ) -> Option<Result<Value, CommandError>> {
        let clip = target.strip_prefix(CLIP_PREFIX)?;
        (field == SCOPE_FIELD).then(|| {
            let Raw::Text(text) = raw else {
                let error = CommandError::invalid_value(
                    "Expected a scope: `output` or `shot:<id>`",
                    json!("output | shot:<id>"),
                )
                .target(target);
                self.scene_report(error.clone(), cx);
                return Err(error);
            };
            self.fx_set_scope(clip, text, Source::Cli, cx)
                .map_err(|error| with_target(error, target))
        })
    }

    /// The selector of the clip was used: the option with `label` was confirmed.
    fn fx_scope_picked(&mut self, key: &ScopeKey, label: &str, cx: &mut Context<Self>) {
        let Some(value) = self
            .scene_ui
            .fx
            .scopes
            .get(key)
            .and_then(|pick| pick.options.iter().find(|(_, text)| text == label))
            .map(|(value, _)| value.clone())
        else {
            return;
        };
        let current = self
            .fx_view()
            .filter(|view| view.scene == key.scene)
            .and_then(|view| view.clip(&key.clip).map(|entry| entry.clip.scope.to_text()));
        // The same option again is not an edit. A refusal is already the panel's notice (shown by
        // the command, or by `fx_set_scope` for a text that never reached one); the selector goes
        // back to the scope the document holds on the next paint.
        if current.as_deref() != Some(value.as_str()) {
            let _ = self.fx_set_scope(&key.clip, &value, Source::Gesture, cx);
        }
        cx.notify();
    }

    /// Drops the selectors whose clip the document no longer holds.
    pub(super) fn fx_scopes_prune(&mut self, view: &FxView) {
        self.scene_ui
            .fx
            .scopes
            .retain(|key, _| key.scene == view.scene && view.clip(&key.clip).is_some());
    }

    // ---- painting ----

    /// The Kit select of `clip`: built once, then kept in step with the document (the options
    /// when the shots or the clip's fit moved, the selection to the scope the clip holds).
    /// `set_items`/`set_selected_value` do not emit a confirmation, so keeping it in step cannot
    /// write.
    fn fx_scope_select(
        &mut self,
        view: &FxView,
        clip: &FxClip,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SelectState<SearchableVec<String>>> {
        let choice = choice_of(view, clip);
        let options: Vec<(String, String)> = choice
            .options
            .iter()
            .map(|option| (option.value.clone(), option_text(option)))
            .collect();
        let labels: Vec<String> = options.iter().map(|(_, text)| text.clone()).collect();
        let current = options
            .iter()
            .find(|(value, _)| *value == choice.value)
            .map(|(_, text)| text.clone())
            .unwrap_or_default();
        let key = ScopeKey {
            scene: view.scene.clone(),
            clip: clip.id.clone(),
        };
        if !self.scene_ui.fx.scopes.contains_key(&key) {
            let select =
                cx.new(|cx| SelectState::new(SearchableVec::new(labels.clone()), None, window, cx));
            let bound = key.clone();
            cx.subscribe(
                &select,
                move |this, _, event: &SelectEvent<SearchableVec<String>>, cx| {
                    let SelectEvent::Confirm(Some(label)) = event else {
                        return;
                    };
                    this.fx_scope_picked(&bound, label, cx);
                },
            )
            .detach();
            self.scene_ui.fx.scopes.insert(
                key.clone(),
                ScopePick {
                    select,
                    options: Vec::new(),
                },
            );
        }
        let Some(pick) = self.scene_ui.fx.scopes.get_mut(&key) else {
            return cx.new(|cx| SelectState::new(SearchableVec::new(labels), None, window, cx));
        };
        let select = pick.select.clone();
        if pick.options != options {
            pick.options = options;
            select.update(cx, |select, cx| {
                select.set_items(SearchableVec::new(labels), window, cx)
            });
        }
        if select.read(cx).selected_value() != Some(&current) {
            select.update(cx, |select, cx| {
                select.set_selected_value(&current, window, cx)
            });
        }
        select
    }

    /// The row of a clip: "Escopo" and its selector.
    pub(super) fn fx_scope_row(
        &mut self,
        t: &Tokens,
        view: &FxView,
        entry: &ClipView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let clip = &entry.clip;
        let select = self.fx_scope_select(view, clip, window, cx);
        let id = clip.id.clone();
        h_flex()
            .items_center()
            .gap(px(6.))
            .px(px(10.))
            .pt(px(4.))
            .text_size(px(10.))
            .text_color(t.faint)
            .child(div().w(px(44.)).flex_shrink_0().child("Escopo"))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w(px(0.))
                    .child(self.geometry.probe(format!("control:fx-scope:{id}")))
                    .child(
                        Select::new(&select)
                            .id(SharedString::from(format!("fx-scope:{id}")))
                            .small()
                            .accessibility_label(format!("Escopo de {id}")),
                    ),
            )
    }

    /// The row that adds an effect on the shot the monitor shows: its caption says which shot, and
    /// each button is disabled with its reason when the monitor shows no frame of the edit.
    pub(super) fn fx_scope_add_row(
        &mut self,
        t: &Tokens,
        view: &FxView,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let caption = match shot_here(view) {
            Ok(shot) => format!("No plano exibido ({})", shot.id),
            Err(_) => "No plano exibido".to_string(),
        };
        let mut adds = h_flex().gap(px(2.));
        for offer in offers() {
            let spec = offer.effect;
            let label = format!(
                "Adicionar {} no plano exibido",
                spec.label.get(LOCALE).to_lowercase()
            );
            let id = SharedString::from(format!("fx-add-shot:{}", spec.id));
            let effect = spec.id;
            let probe = self.geometry.probe(format!("control:fx-add-shot:{effect}"));
            let (enabled, reason) = in_shot_state(view, offer.applicable, offer.reason);
            let button = if enabled {
                let workstation = cx.weak_entity();
                tool_button(id, IconName::Plus, &label).on_click(move |_, _, cx| {
                    let _ = workstation.update(cx, |this, cx| {
                        // The refusal is the panel's notice, already shown by the command.
                        let _ = this.fx_add_in_shot(effect, Source::Gesture, cx);
                    });
                })
            } else {
                disabled_tool_button(
                    id,
                    IconName::Plus,
                    &label,
                    reason.as_deref().unwrap_or("Indisponível"),
                )
            };
            adds = adds.child(div().relative().child(probe).child(button));
        }
        h_flex()
            .id("fx-add-shot-row")
            .items_center()
            .justify_between()
            .px(px(10.))
            .pt(px(2.))
            .text_size(px(10.))
            .text_color(t.faint)
            .child(div().child(caption))
            .child(adds)
    }
}
