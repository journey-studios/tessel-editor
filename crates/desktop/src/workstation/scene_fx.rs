//! The "Efeitos" section of the scene inspector (F016, `docs/features/effects.md`).
//!
//! Nothing here owns an effect or a candidate. The section is built each frame from the attached
//! scene's `bundle.fx` (`scene.fx.list`'s own data) or, while an effect gesture is open on it,
//! from that gesture's candidate (`scene.fx.list {gesture}`'s data); the monitor draws the same
//! candidate (`viewport_sync` passes the open gesture to the render request). Every write is a
//! core command through [`Workstation::execute`]: a one-shot `scene.fx.edit` (`set`, `invoke`,
//! the buttons) or the gesture cycle `scene.fx.gesture.begin|update|commit|cancel` that the
//! sliders and the keyboard drive (BL082.6/.16). So the library, `tessel-headless`, `ui op` and
//! this panel share the handlers, the typed errors and the single undo entry of a commit.
//!
//! The adapter below only translates input into those commands (like `scene_gesture.rs`):
//!
//! - **Begin**: the first `Change` of a slider (or a fresh key press) after the last interaction.
//! - **Update**: every later `Change` and key repeat, as an absolute value from the document the
//!   gesture began on. A refused value (the core's typed error is on screen) keeps the last legal
//!   candidate, so the preview and the commit never show or confirm the refused one.
//! - **Commit**: `Release`, the key-up of a step key, Enter, or the window losing focus.
//! - **Cancel**: Escape, or the panel following another scene. Nothing was written.
//!
//! The semantic nodes (`scene.fx`, `scene.fx.clip:<id>`, `scene.fx.budget`) answer
//! `get`/`set`/`invoke` through the same handlers; the pass-budget warning is on screen in the
//! section AND on the `scene.fx.budget` node, from the same view, so they cannot disagree.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use serde_json::{Value, json};
use tessel_editor_core::directed_scene::fx::PassBudget;
use tessel_editor_core::directed_scene::fx::{self, EffectSpec, FxClip, Locale};
use tessel_editor_core::scene_edit::{
    AddClip, FieldGroup, FxOp, InspectorField, REASON_SINGLE_VALUE, inspector_fields, reset_ops,
};
use tessel_editor_core::scene_interaction::{FxGestureCommand, Gesture, GestureCommand};
use tessel_editor_core::scene_project::SceneCommand;

use super::controls::{disabled_tool_button, glyph, tool_button, value_after_key};
use super::scene_model::{PanelFit, format_number};
use super::scene_panel::SliderRow;
use super::tokens::Tokens;
use super::{Source, Workstation};
use crate::state::{Command, CommandError, Raw};

const SLIDER_EPSILON: f32 = 1e-4;

/// Which control of which effect clip of which scene.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FxKey {
    pub scene: String,
    pub clip: String,
    /// `intensity`, `center_x`, `color_r`, `fade_in_frames`... (the node field name).
    pub control: String,
}

impl FxKey {
    /// Element and probe ID; also what the tests use to find the control.
    pub fn element_id(&self) -> String {
        format!("fx:{}:{}", self.clip, self.control)
    }
}

/// A slider the section built with the range it was built for (`SliderState` cannot change its
/// range, so a clip whose duration moved gets a fresh one).
pub struct FxSlider {
    state: Entity<SliderState>,
    range: (f64, f64, f64),
}

/// Where the interaction with the control being driven stands.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    /// The core has an open effect gesture with this ID; values go to it.
    Open(String),
    /// The gesture ended (Escape, a rejected begin, a stale target, focus loss) while the pointer
    /// or key is still down: ignore everything until the release or the next press.
    Ended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    Commit,
    Cancel,
}

/// Presentation state of the section: the Kit sliders and the control being driven. The value a
/// drag shows is NOT kept here: it is the core's candidate.
#[derive(Default)]
pub struct FxUi {
    sliders: HashMap<FxKey, FxSlider>,
    tracked: Option<(FxKey, Phase)>,
}

impl FxUi {
    /// Drops widget tracking when the section resets; candidate ownership stays with the core.
    pub fn clear(&mut self) {
        self.sliders.clear();
        self.tracked = None;
    }

    fn is_tracked(&self, key: &FxKey) -> bool {
        self.tracked
            .as_ref()
            .is_some_and(|(tracked, _)| tracked == key)
    }

    #[cfg(test)]
    pub fn is_tracking(&self) -> bool {
        self.tracked.is_some()
    }
}

/// Errors that mean the core no longer holds the gesture the drag was feeding.
fn gesture_is_gone(error: &CommandError) -> bool {
    matches!(
        error.code,
        "gesture_invalidated" | "no_gesture" | "unknown_gesture"
    )
}

// ---- the model: what the section shows, with no GPUI ----
//
// The list of fields of a clip (ids, names, ranges, steps, defaults, the order) is the core's
// inspector model (`scene_edit::InspectorField`): this file only paints it and turns what the
// person does into the ops that model builds.

/// One slider of a clip, as the core's inspector describes it.
pub type FxControl = InspectorField;

/// The language the window prints the catalog in. The editor's own is Portuguese; the model also
/// answers in English and Spanish for any adapter that asks.
const LOCALE: Locale = Locale::PtBr;

fn effect_label(spec: &EffectSpec) -> &'static str {
    spec.label.get(LOCALE)
}

/// The sliders of `clip`: the core's fields, parameters first (strength ahead), then its frames.
/// `edit_frames` bounds the frame fields; the core still refuses what the layer rules refuse
/// (overlap, envelope), with a typed code on screen, so these bounds only keep the thumb inside
/// what can ever be valid.
pub fn controls_of(clip: &FxClip, edit_frames: i64) -> Vec<FxControl> {
    inspector_fields(clip, edit_frames)
}

/// What the slider prints beside the value: frames show `q`; ratios no unit.
fn unit_text(control: &FxControl) -> &'static str {
    if control.integer { "q" } else { "" }
}

/// The value a key press means, from `from`: the Kit's keyboard stepping (arrows, shift for ten
/// steps, page keys, home/end) inside the field's range, on the same grid as the pointer.
/// `None` for keys the slider ignores.
fn control_after_key(control: &FxControl, key: &str, shift: bool, from: f64) -> Option<f64> {
    value_after_key(key, shift, from, control.min, control.max, control.step)
        .map(|next| control.quantize(next))
}

/// A clip of the layer with the track it lives on.
#[derive(Debug, Clone)]
pub struct ClipView {
    pub track_id: String,
    pub track_label: String,
    pub clip: FxClip,
}

/// What the section shows for the active scene.
#[derive(Debug, Clone)]
pub struct FxView {
    pub scene: String,
    pub edit_frames: i64,
    pub clips: Vec<ClipView>,
    /// The gesture whose candidate this view shows; `None` for the document.
    pub gesture: Option<String>,
    /// The layer against the pass budget, for what the view shows (the candidate while a gesture
    /// is open): the section's warning and the `scene.fx.budget` node are this.
    pub budget: PassBudget,
    /// The frame of the scene's own edit that the monitor shows now (the core converts the NLE
    /// playhead; `None` when the playhead is outside the edit). Where an insertion lands.
    pub frame_here: Option<i64>,
}

impl FxView {
    pub fn clip(&self, id: &str) -> Option<&ClipView> {
        self.clips.iter().find(|view| view.clip.id == id)
    }
}

pub const CLIP_PREFIX: &str = "scene.fx.clip:";
/// The node of the pass-budget warning: always present, `visible` only past the budget.
pub const BUDGET_ID: &str = "scene.fx.budget";
pub const SECTION_ID: &str = "scene.fx";
const ADD_PREFIX: &str = "add:";
/// `reset:<field>` on a clip node restores one field to its default; `reset` the whole look.
const RESET_PREFIX: &str = "reset:";

/// The words of a typed reason an inspector field does not apply.
fn reason_text(reason: &str) -> &'static str {
    match reason {
        REASON_SINGLE_VALUE => "A faixa tem um único valor nesta edição",
        _ => "Não se aplica a este efeito",
    }
}

/// The actions of a clip node: remove, hide, and the resets the inspector model says make sense
/// (the whole look, then every field away from its default). A reset with nothing to restore is
/// listed disabled with its reason, never as a callback that does nothing.
fn clip_actions(clip: &FxClip, edit_frames: i64) -> Vec<Value> {
    let controls = controls_of(clip, edit_frames);
    let looks_changed = controls.iter().any(|control| {
        control.group == FieldGroup::Param && control.default.is_some() && !control.at_default()
    });
    let mut actions = vec![
        json!({ "action": "remove", "label": "Remover efeito" }),
        json!({
            "action": "toggle_hidden",
            "label": if clip.hidden { "Mostrar efeito" } else { "Ocultar efeito" },
        }),
        json!({
            "action": "reset", "label": "Redefinir parâmetros",
            "enabled": looks_changed,
            "disabled_reason": (!looks_changed).then_some(ALREADY_DEFAULT),
        }),
    ];
    for control in controls.iter().filter(|control| control.default.is_some()) {
        let away = !control.at_default();
        actions.push(json!({
            "action": format!("{RESET_PREFIX}{}", control.id),
            "label": format!("Redefinir {}", control.label(LOCALE)),
            "enabled": away,
            "disabled_reason": (!away).then_some(ALREADY_DEFAULT),
        }));
    }
    actions
}

const ALREADY_DEFAULT: &str = "Já está no padrão";

/// The words of the warning, one string for the screen and the node. It says what happens (the
/// frames are slower) and what does not (no pass is dropped, nothing is refused or hidden).
pub fn budget_message(budget: &PassBudget) -> String {
    format!(
        "{} quadros desenham mais de {} passes de efeito (até {}, a partir do quadro {}). \
         Todos os passes são desenhados; esses quadros ficam mais lentos.",
        budget.frames_over,
        budget.budget,
        budget.peak,
        budget.first_frame_over.unwrap_or(0),
    )
}

fn budget_node(view: &FxView) -> Value {
    let budget = &view.budget;
    json!({
        "id": BUDGET_ID, "kind": "warning", "label": "Aviso de passes de efeito",
        "parent": SECTION_ID, "children": [], "enabled": true, "visible": budget.over(),
        "fields": [], "actions": [],
        "state": {
            "over_budget": budget.over(), "max_passes": budget.budget,
            "peak_passes": budget.peak, "first_frame_over": budget.first_frame_over,
            "frames_over": budget.frames_over, "policy": "warn",
            "source": if view.gesture.is_some() { "gesture" } else { "document" },
            "message": budget.over().then(|| budget_message(budget)),
            "warnings": budget.warnings(),
        },
    })
}

pub fn clip_node_id(clip: &str) -> String {
    format!("{CLIP_PREFIX}{clip}")
}

// ---- the window: reads, writes, nodes, painting ----

impl Workstation {
    /// The open effect gesture of `scene`, while the core can still apply it.
    fn fx_open_gesture(&self, scene: &str) -> Option<&Gesture> {
        let gesture = self.state.scene_interaction.gesture()?;
        (gesture.scene() == scene
            && gesture.fx_clip().is_some()
            && gesture
                .invalidation(&self.state.scenes, self.state.project.revision)
                .is_none())
        .then_some(gesture)
    }

    /// The effects of the scene the panel follows, as the panel shows them: the candidate of the
    /// open effect gesture, else the document. `None` without an attached scene.
    pub(super) fn fx_view(&self) -> Option<FxView> {
        self.fx_view_with(true)
    }

    /// The same, always from the document: what an update of a gesture derives from.
    fn fx_document_view(&self) -> Option<FxView> {
        self.fx_view_with(false)
    }

    fn fx_view_with(&self, candidate: bool) -> Option<FxView> {
        let id = self.scene_active_id()?;
        let attached = self.state.scenes.get(&id)?;
        let open = candidate.then(|| self.fx_open_gesture(&id)).flatten();
        let scene = match open {
            Some(gesture) => Arc::clone(gesture.preview_scene()),
            None => Arc::clone(&attached.scene),
        };
        let bundle = scene.bundle();
        let clips = bundle
            .fx
            .tracks
            .iter()
            .flat_map(|track| {
                track.clips.iter().map(|clip| ClipView {
                    track_id: track.id.clone(),
                    track_label: track.label.clone(),
                    clip: clip.clone(),
                })
            })
            .collect();
        Some(FxView {
            frame_here: self.state.scene_frame_at_playhead(&id).ok(),
            scene: id,
            edit_frames: i64::try_from(bundle.edit_duration_frames).unwrap_or(i64::MAX),
            clips,
            gesture: open.map(|gesture| gesture.id().to_owned()),
            budget: bundle.fx.pass_budget(bundle.edit_duration_frames),
        })
    }

    /// `scene.fx.edit` with one op, in the scene of `view`: one history entry, the core's typed
    /// refusal on screen.
    fn fx_run(
        &mut self,
        scene: &str,
        op: FxOp,
        source: Source,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        self.scene_execute(
            SceneCommand::FxEdit {
                scene: scene.to_string(),
                ops: vec![op],
                dry_run: false,
            },
            source,
            cx,
        )
    }

    /// Adds a clip of `effect` where the monitor is: the NLE playhead's *time*, which the core
    /// converts to the frame of this scene's own edit (`start_time`, so a sequence frame is never
    /// taken for a bundle frame). It lands on the first track where it fits; a time the edit does
    /// not hold is refused atomically with the core's typed error.
    pub(super) fn fx_add(
        &mut self,
        effect: &str,
        source: Source,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        let view = self.fx_view().ok_or_else(no_scene)?;
        let op = FxOp::AddClip(AddClip {
            track: None,
            clip: None,
            effect: effect.to_string(),
            start_frame: 0,
            start_time: Some(self.state.timeline.time_of(self.state.timeline.playhead)),
            duration_frames: None,
            params: Vec::new(),
        });
        self.fx_run(&view.scene, op, source, cx)
    }

    pub(super) fn fx_remove(
        &mut self,
        clip: &str,
        source: Source,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        let view = self.fx_view().ok_or_else(no_scene)?;
        self.fx_known(&view, clip)?;
        let op = FxOp::RemoveClip {
            clip: clip.to_string(),
        };
        self.fx_run(&view.scene, op, source, cx)
    }

    /// Restores the look of a clip (every parameter away from its default) or one field to its
    /// default, as ONE `scene.fx.edit`: the ops are the core inspector's own. Nothing to restore
    /// is the core's no-op (no history entry), not an error.
    pub(super) fn fx_reset(
        &mut self,
        clip: &str,
        field: Option<&str>,
        source: Source,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        let view = self.fx_document_view().ok_or_else(no_scene)?;
        let found = self.fx_known(&view, clip)?;
        let controls = controls_of(&found.clip, view.edit_frames);
        let ops: Vec<FxOp> = match field {
            Some(id) => {
                let control = controls
                    .iter()
                    .find(|control| control.id == id)
                    .ok_or_else(|| unknown_control(clip, id))?;
                control.reset_op(&found.clip).into_iter().collect()
            }
            None => reset_ops(&controls, &found.clip),
        };
        self.scene_execute(
            SceneCommand::FxEdit {
                scene: view.scene.clone(),
                ops,
                dry_run: false,
            },
            source,
            cx,
        )
    }

    pub(super) fn fx_set_hidden(
        &mut self,
        clip: &str,
        hidden: bool,
        source: Source,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        let view = self.fx_view().ok_or_else(no_scene)?;
        self.fx_known(&view, clip)?;
        let op = FxOp::SetHidden {
            clip: clip.to_string(),
            hidden,
        };
        self.fx_run(&view.scene, op, source, cx)
    }

    /// The view of `scene`, only while it is the scene the panel shows: a gesture belongs to the scene
    /// it began on and never edits another one that happens to have a clip of the same id.
    fn fx_view_of(&self, scene: &str) -> Result<FxView, CommandError> {
        let view = self.fx_view().ok_or_else(no_scene)?;
        if view.scene != scene {
            return Err(stale_scene(scene, &view.scene));
        }
        Ok(view)
    }

    fn fx_known<'a>(&self, view: &'a FxView, clip: &str) -> Result<&'a ClipView, CommandError> {
        view.clip(clip)
            .ok_or_else(|| CommandError::unknown_element("effect clip", clip))
    }

    /// One slider (or the semantic `set`) moved a control to `value`.
    pub(super) fn fx_set_control(
        &mut self,
        clip: &str,
        control: &str,
        value: f64,
        source: Source,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        // From the DOCUMENT: a component of a point or a colour keeps the siblings the document has,
        // never the uncommitted candidate of a gesture that happens to be open on the same clip.
        let view = self.fx_document_view().ok_or_else(no_scene)?;
        let found = self.fx_known(&view, clip)?;
        let controls = controls_of(&found.clip, view.edit_frames);
        let Some(target) = controls.iter().find(|candidate| candidate.id == control) else {
            return Err(CommandError::new(
                "unknown_field",
                format!("`{}` has no settable field `{control}`", clip_node_id(clip)),
            )
            .details(
                json!({ "fields": controls.iter().map(|c| c.id.clone()).collect::<Vec<_>>() }),
            ));
        };
        let op = target.op(&found.clip, value)?;
        self.fx_run(&view.scene, op, source, cx)
    }

    // ---- the gesture adapter: input in, core commands out ----

    /// Translates Kit Change/Release events into the same core gesture cycle used by semantic clients.
    pub(super) fn fx_slider_event(
        &mut self,
        key: FxKey,
        event: &SliderEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            SliderEvent::Change(value) => self.fx_changed(&key, f64::from(value.end()), cx),
            SliderEvent::Release(_) => self.fx_released(&key, cx),
        }
    }

    /// A new press: whatever is still tracked lost its release and ends as released.
    pub(super) fn fx_pressed(&mut self, cx: &mut Context<Self>) {
        self.fx_settle(cx);
    }

    /// The control `key` is now at `raw` (a thumb position or a key step): begin the gesture on the
    /// first value, update it on every later one.
    pub(super) fn fx_changed(&mut self, key: &FxKey, raw: f64, cx: &mut Context<Self>) {
        if self
            .scene_ui
            .fx
            .tracked
            .as_ref()
            .is_some_and(|(tracked, _)| tracked != key)
        {
            // Another slider took over (focus moved while a key was held).
            self.fx_settle(cx);
        }
        let phase = self
            .scene_ui
            .fx
            .tracked
            .as_ref()
            .map(|(_, phase)| phase.clone());
        let id = match phase {
            Some(Phase::Ended) => return,
            Some(Phase::Open(id)) => id,
            None => {
                let begun = self.fx_gesture_begin(key, cx);
                self.scene_ui.fx.tracked =
                    Some((key.clone(), begun.clone().map_or(Phase::Ended, Phase::Open)));
                let Some(id) = begun else {
                    return;
                };
                id
            }
        };
        let ops = match self.fx_update_op(key, raw) {
            Ok(op) => op.into_iter().collect(),
            Err(error) => {
                self.scene_report(error, cx);
                return;
            }
        };
        let update = SceneCommand::FxGesture(FxGestureCommand::Update {
            gesture: id.clone(),
            ops,
        });
        // A rejected value leaves the gesture open with its last legal candidate: the panel shows
        // the typed error and the next value may be fine. A lost or invalidated gesture stops
        // the drag.
        if let Err(error) = self.scene_execute(update, Source::Gesture, cx)
            && gesture_is_gone(&error)
        {
            // A replacement may already own the slot. Give back only the gesture this drag used.
            if self
                .state
                .scene_interaction
                .gesture()
                .is_some_and(|open| open.id() == id)
            {
                let _ = self.state.execute(Command::Scene(SceneCommand::FxGesture(
                    FxGestureCommand::Cancel { gesture: id },
                )));
            }
            self.viewport_sync(cx);
            self.scene_ui.fx.tracked = Some((key.clone(), Phase::Ended));
        }
    }

    /// `scene.fx.gesture.begin` on the clip of `key`, in the scene the panel shows.
    fn fx_gesture_begin(&mut self, key: &FxKey, cx: &mut Context<Self>) -> Option<String> {
        let result = self.fx_view_of(&key.scene).and_then(|view| {
            self.fx_known(&view, &key.clip)?;
            self.scene_execute(
                SceneCommand::FxGesture(FxGestureCommand::Begin {
                    scene: key.scene.clone(),
                    clip: key.clip.clone(),
                    label: None,
                }),
                Source::Gesture,
                cx,
            )
        });
        match result {
            Ok(outcome) => outcome["gesture"].as_str().map(str::to_string),
            Err(error) => {
                // A panel that moved on (or a clip that is gone) is shown like any refusal.
                self.scene_report(error, cx);
                None
            }
        }
    }

    /// The op that moves the control of `key` to the (quantized) `raw`, from the DOCUMENT's clip:
    /// every update restarts from the base the gesture began on, and a component of a point or a
    /// colour keeps its siblings as the document has them. `None` when the thumb rests where the
    /// document already is: the update is then empty, so the commit is a no-op (writing down a
    /// default the clip leaves out would be a change of the document, and a tap is not an edit).
    fn fx_update_op(&self, key: &FxKey, raw: f64) -> Result<Option<FxOp>, CommandError> {
        let view = self.fx_document_view().ok_or_else(no_scene)?;
        if view.scene != key.scene {
            return Err(stale_scene(&key.scene, &view.scene));
        }
        let found = self.fx_known(&view, &key.clip)?;
        let controls = controls_of(&found.clip, view.edit_frames);
        let control = controls
            .iter()
            .find(|candidate| candidate.id == key.control)
            .ok_or_else(|| unknown_control(&key.clip, &key.control))?;
        let value = control.quantize(raw);
        if (value - control.value).abs() < 1e-9 {
            return Ok(None);
        }
        control.op(&found.clip, value).map(Some)
    }

    /// The pointer was released or the step key came up (or Enter): the gesture is committed.
    pub(super) fn fx_released(&mut self, key: &FxKey, cx: &mut Context<Self>) {
        let Some((tracked, phase)) = self.scene_ui.fx.tracked.clone() else {
            return;
        };
        if &tracked != key {
            return;
        }
        self.scene_ui.fx.tracked = None;
        self.fx_finish(phase, End::Commit, cx);
    }

    /// The window lost the focus: a release may never arrive, so the gesture is committed ONCE
    /// (the tracking ends, so a late release finds nothing to commit again).
    pub(super) fn fx_focus_lost(&mut self, cx: &mut Context<Self>) {
        let Some((key, phase)) = self.scene_ui.fx.tracked.clone() else {
            return;
        };
        self.scene_ui.fx.tracked = Some((key, Phase::Ended));
        self.fx_finish(phase, End::Commit, cx);
    }

    /// Escape: the gesture is cancelled and the rest of the press is ignored.
    pub(super) fn fx_cancel_drag(&mut self, cx: &mut Context<Self>) {
        let Some((key, phase)) = self.scene_ui.fx.tracked.clone() else {
            return;
        };
        self.scene_ui.fx.tracked = Some((key, Phase::Ended));
        self.fx_finish(phase, End::Cancel, cx);
    }

    /// The panel follows another scene (or the project changed): what the old scene's gesture
    /// holds is dropped in the core, never carried over to the control of the same name there.
    pub(super) fn fx_scene_left(&mut self, cx: &mut Context<Self>) {
        self.scene_ui.fx.tracked = None;
        let open = self
            .state
            .scene_interaction
            .gesture()
            .filter(|gesture| gesture.fx_clip().is_some())
            .map(|gesture| gesture.id().to_owned());
        if let Some(gesture) = open {
            let _ = self.state.execute(Command::Scene(SceneCommand::FxGesture(
                FxGestureCommand::Cancel { gesture },
            )));
            // The monitor was drawing that candidate: it goes back to the confirmed scene now,
            // not at the next unrelated command.
            self.viewport_sync(cx);
        }
    }

    /// Settles the tracked drag once when focus changes; an already ended phase cannot commit again.
    fn fx_settle(&mut self, cx: &mut Context<Self>) {
        let Some((_, phase)) = self.scene_ui.fx.tracked.take() else {
            return;
        };
        self.fx_finish(phase, End::Commit, cx);
    }

    /// Sends `end` for the gesture of `phase`, unless the core no longer holds it (another edit
    /// invalidated it, the project was swapped): there is then nothing to report.
    fn fx_finish(&mut self, phase: Phase, end: End, cx: &mut Context<Self>) {
        let Phase::Open(id) = phase else {
            return;
        };
        if self
            .state
            .scene_interaction
            .gesture()
            .is_none_or(|open| open.id() != id)
        {
            return;
        }
        match end {
            End::Cancel => {
                let _ = self.scene_execute(
                    SceneCommand::FxGesture(FxGestureCommand::Cancel { gesture: id }),
                    Source::Gesture,
                    cx,
                );
            }
            End::Commit => {
                let commit = SceneCommand::FxGesture(FxGestureCommand::Commit {
                    gesture: id.clone(),
                });
                if self.scene_execute(commit, Source::Gesture, cx).is_err() {
                    // A rejected commit gives the gesture back to the core (so a caller could
                    // retry). A drag has nothing to retry with: drop the candidate so the panel
                    // shows the document again, and keep the typed error on screen.
                    let _ = self.state.execute(Command::Scene(SceneCommand::Gesture(
                        GestureCommand::Cancel { gesture: id },
                    )));
                    // The monitor may still hold the candidate that was just given up.
                    self.viewport_sync(cx);
                }
            }
        }
    }

    /// A key press on the slider of `key`. Arrows, shift, page keys and home/end step the value
    /// on the control's own grid and range (the same rule as the pointer); Enter confirms the
    /// gesture in progress. Escape is the window's (it cancels). Returns whether the key was the
    /// slider's. `held` is auto-repeat: it continues the gesture instead of starting another.
    pub(super) fn fx_key_down(
        &mut self,
        key: &FxKey,
        pressed: &str,
        shift: bool,
        held: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if pressed == "enter" {
            if self.scene_ui.fx.is_tracked(key) {
                self.fx_released(key, cx);
                return true;
            }
            return false;
        }
        let Some(from) = self.fx_control_now(key) else {
            return false;
        };
        let Some(next) = control_after_key(&from.1, pressed, shift, from.0) else {
            return false;
        };
        if !held {
            self.fx_pressed(cx);
        }
        self.fx_changed(key, next, cx);
        true
    }

    /// The key-up that commits a keyboard gesture. Only step keys count: releasing Shift while an
    /// arrow is still held must not end the gesture.
    pub(super) fn fx_key_up(&mut self, key: &FxKey, released: &str, cx: &mut Context<Self>) {
        let is_step = self.fx_control_now(key).is_some_and(|(value, control)| {
            control_after_key(&control, released, false, value).is_some()
        });
        if is_step {
            self.fx_released(key, cx);
        }
    }

    /// What the control of `key` shows now (the candidate's value while its gesture is open) and
    /// the control, for stepping.
    fn fx_control_now(&self, key: &FxKey) -> Option<(f64, FxControl)> {
        let view = self.fx_view()?;
        if view.scene != key.scene {
            return None;
        }
        let clip = &view.clip(&key.clip)?.clip;
        controls_of(clip, view.edit_frames)
            .into_iter()
            .find(|control| control.id == key.control)
            .map(|control| (control.value, control))
    }

    // ---- automation ----

    /// The `scene.fx` section node and one node per clip.
    pub(super) fn fx_nodes(&self) -> Vec<Value> {
        let Some(view) = self.fx_view() else {
            return Vec::new();
        };
        let mut nodes = Vec::new();
        let mut children = Vec::new();
        for entry in &view.clips {
            let clip = &entry.clip;
            children.push(json!(clip_node_id(&clip.id)));
            let mut fields: Vec<Value> = controls_of(clip, view.edit_frames)
                .iter()
                .map(|control| {
                    json!({
                        "field": control.id, "label": format!("{} {}", effect_label(clip.effect), control.label(LOCALE)),
                        "type": "number", "value": control.value, "min": control.min,
                        "max": control.max, "step": control.step, "integer": control.integer,
                        "default": control.default, "at_default": control.at_default(),
                        "group": control.group.name(),
                        "enabled": control.applicable,
                        "disabled_reason": control.reason.map(reason_text),
                    })
                })
                .collect();
            fields.push(json!({
                "field": "hidden", "label": "Oculto", "type": "boolean", "value": clip.hidden,
            }));
            let params: serde_json::Map<String, Value> = clip
                .effect
                .params
                .iter()
                .zip(&clip.params)
                .map(|(spec, value)| (spec.id.to_string(), value.to_json()))
                .collect();
            nodes.push(json!({
                "id": clip_node_id(&clip.id), "kind": "fx_clip",
                "label": format!("{} {}", effect_label(clip.effect), clip.id),
                "parent": SECTION_ID, "children": [], "enabled": true, "visible": true,
                "fields": fields,
                "actions": clip_actions(clip, view.edit_frames),
                "state": {
                    "clip": clip.id, "track": entry.track_id, "track_label": entry.track_label,
                    "effect": clip.effect.id, "start_frame": clip.start_frame,
                    "duration_frames": clip.duration_frames,
                    "end_frame_exclusive": clip.end_frame_exclusive(),
                    "fade_in_frames": clip.fade_in_frames, "fade_out_frames": clip.fade_out_frames,
                    "hidden": clip.hidden, "params": params,
                },
            }));
        }
        children.push(json!(BUDGET_ID));
        nodes.push(budget_node(&view));
        let gesture = self
            .state
            .scene_interaction
            .gesture()
            .filter(|open| open.scene() == view.scene && open.fx_clip().is_some());
        let gesture_state =
            gesture.map(|open| open.to_json(&self.state.scenes, self.state.project.revision));
        let actions: Vec<Value> = fx::catalog()
            .iter()
            .map(|spec| {
                json!({
                    "action": format!("{ADD_PREFIX}{}", spec.id),
                    "label": format!("Adicionar {}", spec.label.get(LOCALE).to_lowercase()),
                    "enabled": true,
                })
            })
            .collect();
        nodes.insert(
            0,
            json!({
                "id": SECTION_ID, "kind": "section", "label": "Efeitos",
                "parent": "scene.panel", "children": children, "enabled": true, "visible": true,
                "fields": [], "actions": actions,
                "state": {
                    "scene": view.scene, "catalog_version": fx::FX_CATALOG_VERSION,
                    "clip_count": view.clips.len(), "scope": ["output"],
                    "effects": fx::catalog().iter().map(|spec| spec.id).collect::<Vec<_>>(),
                    "edit_duration_frames": view.edit_frames,
                    "playhead_frame": view.frame_here,
                    "source": if view.gesture.is_some() { "gesture" } else { "document" },
                    "gesture": gesture_state,
                    "over_budget": view.budget.over(),
                },
            }),
        );
        nodes
    }

    /// `invoke` on the section or a clip. `None` when the target is not one of them.
    pub(super) fn fx_invoke(
        &mut self,
        target: &str,
        action: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Option<Result<Value, CommandError>> {
        if target == SECTION_ID {
            let result = match action {
                Some(add) if add.starts_with(ADD_PREFIX) => {
                    self.fx_add(&add[ADD_PREFIX.len()..], Source::Cli, cx)
                }
                other => Err(unknown_action(
                    target,
                    other.unwrap_or(""),
                    &fx::catalog()
                        .iter()
                        .map(|spec| format!("{ADD_PREFIX}{}", spec.id))
                        .collect::<Vec<_>>(),
                )),
            };
            return Some(result);
        }
        let clip = target.strip_prefix(CLIP_PREFIX)?;
        Some(match action {
            Some("remove") => self.fx_remove(clip, Source::Cli, cx),
            Some("toggle_hidden") => match self
                .fx_view()
                .and_then(|view| view.clip(clip).map(|entry| entry.clip.hidden))
            {
                Some(hidden) => self.fx_set_hidden(clip, !hidden, Source::Cli, cx),
                None => Err(CommandError::unknown_element("effect clip", clip)),
            },
            Some("reset") => self.fx_reset(clip, None, Source::Cli, cx),
            Some(reset) if reset.starts_with(RESET_PREFIX) => {
                self.fx_reset(clip, Some(&reset[RESET_PREFIX.len()..]), Source::Cli, cx)
            }
            other => Err(unknown_action(
                target,
                other.unwrap_or(""),
                &self
                    .fx_view()
                    .and_then(|view| {
                        view.clip(clip).map(|entry| {
                            clip_actions(&entry.clip, view.edit_frames)
                                .iter()
                                .filter_map(|action| action["action"].as_str().map(str::to_string))
                                .collect::<Vec<_>>()
                        })
                    })
                    .unwrap_or_else(|| vec!["remove".to_string(), "toggle_hidden".to_string()]),
            )),
        })
    }

    /// `set` on a clip node: a number field, or `hidden`. `None` when the target is not a clip.
    pub(super) fn fx_set(
        &mut self,
        target: &str,
        field: &str,
        raw: &Raw,
        cx: &mut Context<Self>,
    ) -> Option<Result<Value, CommandError>> {
        let clip = target.strip_prefix(CLIP_PREFIX)?;
        let result = (|| {
            if field == "hidden" {
                let hidden = raw.boolean().map_err(|error| error.target(target))?;
                return self.fx_set_hidden(clip, hidden, Source::Cli, cx);
            }
            let value = raw.number().map_err(|error| error.target(target))?;
            self.fx_set_control(clip, field, value, Source::Cli, cx)
        })();
        Some(result.map_err(|error| {
            if error.target.is_none() {
                error.target(target)
            } else {
                error
            }
        }))
    }

    // ---- painting ----

    /// The section: a header with one add button per catalog effect, then one block per clip.
    pub(super) fn scene_fx_section(
        &mut self,
        t: &Tokens,
        fit: PanelFit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let Some(view) = self.fx_view() else {
            return div();
        };
        let mut header = h_flex()
            .items_center()
            .justify_between()
            .px(px(10.))
            .pt(px(8.))
            .text_size(px(11.))
            .text_color(t.text)
            .child(div().child("Efeitos"));
        let mut adds = h_flex().gap(px(2.));
        for spec in fx::catalog() {
            let label = format!("Adicionar {}", spec.label.get(LOCALE).to_lowercase());
            let id = SharedString::from(format!("fx-add:{}", spec.id));
            let workstation = cx.weak_entity();
            let effect = spec.id;
            let probe = self.geometry.probe(format!("control:fx-add:{effect}"));
            adds = adds.child(div().relative().child(probe).child(
                tool_button(id, IconName::Plus, &label).on_click(move |_, _, cx| {
                    let _ = workstation.update(cx, |this, cx| {
                        let _ = this.fx_add(effect, Source::Gesture, cx);
                    });
                }),
            ));
        }
        header = header.child(adds);
        let mut section = v_flex().w_full().child(header);
        if view.budget.over() {
            section = section.child(
                h_flex()
                    .id("fx-budget-warning")
                    .relative()
                    .child(self.geometry.probe("control:fx-budget-warning"))
                    .items_start()
                    .gap(px(6.))
                    .mx(px(10.))
                    .mt(px(4.))
                    .p(px(6.))
                    .rounded(px(3.))
                    .border_1()
                    .border_color(t.title_border)
                    .text_size(px(10.))
                    .text_color(t.text)
                    .child(glyph(IconName::CircleAlert, 14.0, t.title_border))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .child(budget_message(&view.budget)),
                    ),
            );
        }
        if view.clips.is_empty() {
            section = section.child(
                div()
                    .id("fx-empty")
                    .px(px(10.))
                    .pb(px(4.))
                    .text_size(px(10.))
                    .text_color(t.faint)
                    .child("Nenhum efeito na cena. Adicione um efeito pelos botões acima (vinheta, fade)."),
            );
        }
        for entry in &view.clips {
            let block = self.fx_clip_block(t, fit, &view, entry, window, cx);
            section = section.child(block);
        }
        section
    }

    fn fx_clip_block(
        &mut self,
        t: &Tokens,
        fit: PanelFit,
        view: &FxView,
        entry: &ClipView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let clip = &entry.clip;
        let id = clip.id.clone();
        let workstation = cx.weak_entity();
        let toggle = {
            let workstation = workstation.clone();
            let clip_id = id.clone();
            Switch::new(SharedString::from(format!("fx-visible:{id}")))
                .checked(!clip.hidden)
                .accessibility_label(format!("{} visível", effect_label(clip.effect)))
                .on_click(move |checked: &bool, _, cx: &mut App| {
                    let hidden = !*checked;
                    let clip_id = clip_id.clone();
                    let _ = workstation.update(cx, |this, cx| {
                        let _ = this.fx_set_hidden(&clip_id, hidden, Source::Gesture, cx);
                    });
                })
        };
        let remove_label = format!("Remover {}", clip.id);
        let remove = {
            let clip_id = id.clone();
            let workstation = workstation.clone();
            let button = tool_button(
                SharedString::from(format!("fx-remove:{id}")),
                IconName::Trash,
                &remove_label,
            )
            .on_click(move |_, _, cx| {
                let clip_id = clip_id.clone();
                let _ = workstation.update(cx, |this, cx| {
                    let _ = this.fx_remove(&clip_id, Source::Gesture, cx);
                });
            });
            div()
                .relative()
                .child(self.geometry.probe(format!("control:fx-remove:{id}")))
                .child(button)
        };
        let reset = {
            let looks_changed = controls_of(clip, view.edit_frames).iter().any(|control| {
                control.group == FieldGroup::Param
                    && control.default.is_some()
                    && !control.at_default()
            });
            let label = format!("Redefinir parâmetros de {}", clip.id);
            let button_id = SharedString::from(format!("fx-reset:{id}"));
            let button = if looks_changed {
                let clip_id = id.clone();
                let workstation = workstation.clone();
                tool_button(button_id, IconName::RotateCcw, &label).on_click(move |_, _, cx| {
                    let clip_id = clip_id.clone();
                    let _ = workstation.update(cx, |this, cx| {
                        let _ = this.fx_reset(&clip_id, None, Source::Gesture, cx);
                    });
                })
            } else {
                disabled_tool_button(button_id, IconName::RotateCcw, &label, ALREADY_DEFAULT)
            };
            div()
                .relative()
                .child(self.geometry.probe(format!("control:fx-reset:{id}")))
                .child(button)
        };
        let title = h_flex()
            .items_center()
            .gap(px(6.))
            .px(px(10.))
            .pt(px(6.))
            .text_size(px(11.))
            .text_color(if clip.hidden { t.faint } else { t.text })
            .child(
                div()
                    .id(SharedString::from(format!("fx-title:{id}")))
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .child(format!(
                        "{} · {} ({})",
                        effect_label(clip.effect),
                        clip.id,
                        entry.track_label
                    )),
            )
            .child(toggle)
            .child(reset)
            .child(remove);
        let mut block = v_flex().w_full().child(title);
        for control in controls_of(clip, view.edit_frames) {
            // A field whose range is one value has nothing to move: it stays out of the panel (the
            // semantic node still lists it, disabled, with the reason).
            if !control.applicable {
                continue;
            }
            let key = FxKey {
                scene: view.scene.clone(),
                clip: clip.id.clone(),
                control: control.id.clone(),
            };
            // The candidate's value while a gesture is open (the view is the candidate's), else the
            // document's: never a buffer of the widget.
            let shown = control.value;
            let control_label = control.label(LOCALE);
            let element = key.element_id();
            let slider = self.fx_slider(&key, &control, window, cx);
            let value = if control.integer {
                format!("{}", shown.round() as i64)
            } else {
                format_number(shown)
            };
            // Labels here are words ("Intensidade"), not the one letter of a vector component, so the
            // label takes its own line above the slider whatever the width.
            let stacked = PanelFit {
                compact: true,
                ..fit
            };
            block = block.child(self.scene_slider_row(
                t,
                stacked,
                SliderRow {
                    element: &element,
                    label: &control_label,
                    modified: false,
                    value,
                    unit: unit_text(&control),
                },
                slider,
            ));
        }
        block
    }

    fn fx_slider(
        &mut self,
        key: &FxKey,
        control: &FxControl,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let range = (control.min, control.max, control.step);
        let stale = self
            .scene_ui
            .fx
            .sliders
            .get(key)
            .is_some_and(|slider| slider.range != range);
        if stale && !self.scene_ui.fx.is_tracked(key) {
            self.scene_ui.fx.sliders.remove(key);
        }
        if !self.scene_ui.fx.sliders.contains_key(key) {
            let state = cx.new(|_| {
                SliderState::new()
                    .min(control.min as f32)
                    .max(control.max as f32)
                    .step(control.step as f32)
            });
            let slider_key = key.clone();
            cx.subscribe(&state, move |this, _, event: &SliderEvent, cx| {
                this.fx_slider_event(slider_key.clone(), event, cx);
            })
            .detach();
            self.scene_ui
                .fx
                .sliders
                .insert(key.clone(), FxSlider { state, range });
        }
        let Some(entry) = self.scene_ui.fx.sliders.get(key) else {
            return div().into_any_element();
        };
        let state = entry.state.clone();
        // While a drag owns the control the thumb is the user's; otherwise it follows the model.
        // `set_value` does not emit a change, so following it cannot loop.
        if !self.scene_ui.fx.is_tracked(key) {
            let target = control.value as f32;
            if (state.read(cx).value().end() - target).abs() > SLIDER_EPSILON {
                state.update(cx, |slider, cx| slider.set_value(target, window, cx));
            }
        }
        let element = key.element_id();
        let stepper = key.clone();
        let releaser = key.clone();
        div()
            .id(SharedString::from(format!("fx-slider-{element}")))
            .relative()
            .flex_1()
            .focusable()
            .child(self.geometry.probe(format!("control:{element}")))
            // Capture phase: the Kit thumb stops its own mouse-down.
            .capture_any_mouse_down(fx_slider_press(cx))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if this.fx_key_down(
                    &stepper,
                    event.keystroke.key.as_str(),
                    event.keystroke.modifiers.shift,
                    event.is_held,
                    cx,
                ) {
                    cx.stop_propagation();
                }
            }))
            .on_key_up(cx.listener(move |this, event: &KeyUpEvent, _, cx| {
                this.fx_key_up(&releaser, event.keystroke.key.as_str(), cx);
            }))
            .child(
                Slider::new(&state)
                    .horizontal()
                    .accessibility_label(format!("Efeito {}", control.label(LOCALE))),
            )
            .into_any_element()
    }
}

/// A capture-phase press listener for a slider's wrapper: a new press settles what a lost
/// release left tracked, and ends a framework drag whose mouse-up never came.
fn fx_slider_press(
    cx: &mut Context<Workstation>,
) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
    cx.listener(|this, event: &MouseDownEvent, window, cx| {
        if event.button == MouseButton::Left {
            cx.stop_active_drag(window);
            this.fx_pressed(cx);
        }
    })
}

fn stale_scene(was: &str, now: &str) -> CommandError {
    CommandError::new(
        "stale_gesture",
        format!(
            "The effect gesture belonged to scene `{was}` and the panel now shows `{now}`: it was dropped, nothing changed"
        ),
    )
    .target(was)
}

fn unknown_control(clip: &str, control: &str) -> CommandError {
    CommandError::new(
        "unknown_field",
        format!("`{}` has no settable field `{control}`", clip_node_id(clip)),
    )
}

fn no_scene() -> CommandError {
    CommandError::new("unknown_element", super::scene_ui::NO_SCENE_REASON)
}

fn unknown_action(target: &str, action: &str, available: &[String]) -> CommandError {
    CommandError::new(
        "unknown_action",
        format!("`{target}` has no action `{action}`"),
    )
    .target(target)
    .details(json!({ "actions": available }))
}
