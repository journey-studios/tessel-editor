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
//!   state. The clip lasts what the core gives a clip with no duration: the effect's typical
//!   length, cut where the shot ends; the window picks nothing.

use std::collections::HashMap;

use gpui_kit::assets::IconName;
use gpui_kit::component::searchable_list::SearchableVec;
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::{Sizable, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use serde_json::{Value, json};
use tessel_editor_core::directed_scene::Shot;
use tessel_editor_core::directed_scene::fx::{
    EffectSpec, FX_SCOPE_MISMATCH, FX_SCOPE_NOT_YET, FxClip, FxScope, UNKNOWN_SHOT, shot_at,
};
use tessel_editor_core::scene_edit::{
    AddClip, ChoiceOption, FxOp, InspectorChoice, offers, scope_choice, scopes,
};

use super::controls::{disabled_tool_button, tool_button};
use super::scene_fx::{ADD_PREFIX, CLIP_PREFIX, ClipView, FxView, LOCALE, SECTION_ID};
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

/// The frame the monitor shows and the shot that holds it, or why none: where `add_in_shot` lands.
fn shot_here(view: &FxView) -> Result<(i64, &Shot), &'static str> {
    let frame = view.frame_here.ok_or(OUTSIDE_THE_EDIT)?;
    let shot = shot_at(&view.shots, frame).ok_or(NOT_IN_A_SHOT)?;
    Ok((frame, shot))
}

/// The id of the shot the monitor shows: the state of the section.
pub(super) fn shot_here_id(view: &FxView) -> Option<String> {
    shot_here(view).ok().map(|(_, shot)| shot.id.clone())
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

/// The `scope` field of a clip node: the choice with the very JSON `scene.fx.inspect` answers (one
/// model, so the window and the library cannot disagree), put where the node lists its fields.
pub(super) fn scope_field(view: &FxView, clip: &FxClip) -> Value {
    let choice = scope_choice(clip, &view.shots);
    let mut field = choice.to_json(&clip.id, LOCALE);
    field["field"] = json!(SCOPE_FIELD);
    field["label"] = json!(format!(
        "{} {}",
        clip.effect.label.get(LOCALE),
        choice.label(LOCALE)
    ));
    field["enabled"] = json!(true);
    field["disabled_reason"] = Value::Null;
    field
}

/// The `set_scope:<scope>` actions of a clip node, one per option of its choice: disabled, with
/// the reason, for the scope the clip holds and for a shot that does not hold the clip.
pub(super) fn scope_actions(view: &FxView, clip: &FxClip) -> Vec<Value> {
    let choice = scope_choice(clip, &view.shots);
    choice
        .options
        .iter()
        .map(|option| {
            let selected = option.value == choice.value;
            let enabled = option.applicable && !selected;
            let reason = if selected {
                Some(ALREADY_HERE)
            } else {
                option.reason.map(reason_text)
            };
            json!({
                "action": format!("{SET_SCOPE_PREFIX}{}", option.value),
                "label": format!("Mover para {}", option_text(option)),
                "enabled": enabled,
                "disabled_reason": reason.filter(|_| !enabled),
            })
        })
        .collect()
}

/// The words of the action that adds `spec` on the shot the monitor shows.
fn add_in_shot_label(spec: &EffectSpec) -> String {
    format!(
        "Adicionar {} no plano exibido",
        spec.label.get(LOCALE).to_lowercase()
    )
}

/// The `add_in_shot:<effect>` actions of the section node, one per effect of the catalog.
pub(super) fn add_in_shot_actions(view: &FxView) -> Vec<Value> {
    offers()
        .iter()
        .map(|offer| {
            let spec = offer.effect;
            let (enabled, reason) = in_shot_state(view, offer.applicable, offer.reason);
            json!({
                "action": format!("{ADD_IN_SHOT_PREFIX}{}", spec.id),
                "label": add_in_shot_label(spec),
                "enabled": enabled,
                "disabled_reason": reason,
            })
        })
        .collect()
}

/// The actions of the section node: add each effect on the output, then on the shot the monitor
/// shows (F062).
pub(super) fn section_actions(view: &FxView) -> Vec<Value> {
    let mut actions: Vec<Value> = offers()
        .iter()
        .map(|offer| {
            let spec = offer.effect;
            json!({
                "action": format!("{ADD_PREFIX}{}", spec.id),
                "label": format!("Adicionar {}", spec.label.get(LOCALE).to_lowercase()),
                "enabled": offer.applicable,
                "disabled_reason": offer.reason,
            })
        })
        .collect();
    actions.extend(add_in_shot_actions(view));
    actions
}

// ---- the selector: Kit state per clip ----

/// Which clip's selector.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct ScopeKey {
    scene: String,
    clip: String,
}

/// One option of a scope selector: the text of the scope (what a pick carries) and the words it is
/// listed with.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ScopeItem {
    pub(super) value: String,
    pub(super) title: SharedString,
}

impl SelectItem for ScopeItem {
    type Value = String;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn value(&self) -> &String {
        &self.value
    }
}

/// The Kit state of one scope `Select`.
type ScopeSelect = Entity<SelectState<SearchableVec<ScopeItem>>>;

/// A Kit `Select` with the options it was last given, and the revision of the project it was last
/// painted at: the clip it shows is the one of THAT document (an id is not an identity: the core
/// gives a new clip the smallest id that is free, so a removed `fx-1` is the next clip's `fx-1`).
pub(super) struct ScopePick {
    select: ScopeSelect,
    items: Vec<ScopeItem>,
    revision: u64,
}

/// The selectors of the section, by clip.
pub(super) type ScopePicks = HashMap<ScopeKey, ScopePick>;

fn no_scene() -> CommandError {
    CommandError::new("unknown_element", super::scene_ui::NO_SCENE_REASON)
}

#[cfg(test)]
impl Workstation {
    /// The select of `clip` and the options it lists: what the tests pick through.
    pub(super) fn scope_select_of(
        &self,
        scene: &str,
        clip: &str,
    ) -> Option<(ScopeSelect, Vec<ScopeItem>)> {
        let pick = self.scene_ui.fx.scopes.get(&ScopeKey {
            scene: scene.to_string(),
            clip: clip.to_string(),
        })?;
        Some((pick.select.clone(), pick.items.clone()))
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
        self.fx_set_scope_in(&view, clip, scope, source, cx)
    }

    /// The same, on a view the caller already checked is the scene the pick was made in, so a pick
    /// never lands on another scene that has a clip of the same id (BL082.49).
    fn fx_set_scope_in(
        &mut self,
        view: &FxView,
        clip: &str,
        scope: &str,
        source: Source,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        let found = view
            .clip(clip)
            .ok_or_else(|| CommandError::unknown_element("effect clip", clip))?;
        let op = match InspectorChoice::set_scope_op(&found.clip, scope) {
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
    /// `scene.fx.edit`. It carries no duration, so the core gives the clip the effect's typical
    /// length cut where the shot ends. A monitor outside the edit is a typed `disabled_element`
    /// with nothing written.
    pub(super) fn fx_add_in_shot(
        &mut self,
        effect: &str,
        source: Source,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        let view = self.fx_view().ok_or_else(no_scene)?;
        let (frame, shot) = match shot_here(&view) {
            Ok((frame, shot)) => (frame, shot.id.clone()),
            Err(reason) => {
                let error = CommandError::disabled(reason).target(SECTION_ID);
                self.scene_report(error.clone(), cx);
                return Err(error);
            }
        };
        // An effect the catalog does not have is the core's to refuse (`unknown_effect`).
        let op = FxOp::AddClip(AddClip {
            track: None,
            clip: None,
            effect: effect.to_string(),
            start_frame: frame,
            start_time: None,
            duration_frames: None,
            params: Vec::new(),
            scope: FxScope::Shot(shot),
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

    /// The selector of the clip was used: the option `value` (the text of a scope) was confirmed.
    fn fx_scope_picked(&mut self, key: &ScopeKey, value: &str, cx: &mut Context<Self>) {
        // A selector of another scene, or of a clip that is gone, writes nothing (BL082.49): the
        // pick belongs to the scene it was made in, and never edits one that happens to have a clip
        // of the same id. The same option again is not an edit either. A refusal is already the
        // panel's notice (shown by the command, or by `fx_set_scope_in` for a text that never
        // reached one); the selector goes back to the scope the document holds on the next paint.
        // Nor does it edit a clip that took the id of the one it was made on: a pick that reaches
        // a document other than the one the selector was painted for (an edit, an undo, a removed
        // clip and a new one, in between) is the selector of a clip the user no longer sees.
        let painted = self
            .scene_ui
            .fx
            .scopes
            .get(key)
            .is_some_and(|pick| pick.revision == self.state.project.revision);
        if painted
            && let Ok(view) = self.fx_view_of(&key.scene)
            && let Some(entry) = view.clip(&key.clip)
            && entry.clip.scope.to_text() != value
        {
            let _ = self.fx_set_scope_in(&view, &key.clip, value, Source::Gesture, cx);
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
    ) -> ScopeSelect {
        let choice = scope_choice(clip, &view.shots);
        let items: Vec<ScopeItem> = choice
            .options
            .iter()
            .map(|option| ScopeItem {
                value: option.value.clone(),
                title: option_text(option).into(),
            })
            .collect();
        let key = ScopeKey {
            scene: view.scene.clone(),
            clip: clip.id.clone(),
        };
        let revision = self.state.project.revision;
        let pick = self
            .scene_ui
            .fx
            .scopes
            .entry(key.clone())
            .or_insert_with(|| {
                let select = cx.new(|cx| {
                    SelectState::new(SearchableVec::new(items.clone()), None, window, cx)
                });
                cx.subscribe(
                    &select,
                    move |this, _, event: &SelectEvent<SearchableVec<ScopeItem>>, cx| {
                        if let SelectEvent::Confirm(Some(value)) = event {
                            this.fx_scope_picked(&key, value, cx);
                        }
                    },
                )
                .detach();
                ScopePick {
                    select,
                    items: items.clone(),
                    revision,
                }
            });
        pick.revision = revision;
        let select = pick.select.clone();
        if pick.items != items {
            pick.items = items.clone();
            select.update(cx, |select, cx| {
                select.set_items(SearchableVec::new(items), window, cx)
            });
        }
        if select.read(cx).selected_value() != Some(&choice.value) {
            select.update(cx, |select, cx| {
                select.set_selected_value(&choice.value, window, cx)
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
            Ok((_, shot)) => format!("No plano exibido ({})", shot.id),
            Err(_) => "No plano exibido".to_string(),
        };
        let mut adds = h_flex().gap(px(2.));
        for offer in offers() {
            let spec = offer.effect;
            let label = add_in_shot_label(spec);
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
