//! The "Efeitos" section of the scene inspector (F016, `docs/features/effects.md`).
//!
//! Nothing here owns an effect. The section is built each frame from the attached scene's
//! `bundle.fx` (`scene.fx.list`'s own data), and every gesture is one `scene.fx.edit` through
//! [`Workstation::execute`] (`Command::Scene(SceneCommand::FxEdit)`): one command, one undo entry,
//! the core's atomic refusal and typed error codes. The semantic nodes (`scene.fx`,
//! `scene.fx.clip:<id>`) answer `get`/`set`/`invoke` through the handlers the pointer uses.
//!
//! A slider is a *drag buffer*, not a gesture: `Change` only remembers the value the thumb shows,
//! `Release` (or the window losing focus) sends the single edit, Escape drops it. There is no live
//! preview of the picture while dragging (recorded as a gap in the feature doc); the release
//! redraws the monitor like any other edit.

use std::collections::HashMap;

use gpui_kit::assets::IconName;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use serde_json::{Value, json};
use tessel_editor_core::directed_scene::fx::{
    self, EffectSpec, FxClip, FxValue, ParamKind, ParamSpec,
};
use tessel_editor_core::scene_edit::{AddClip, ClipField, FxOp};
use tessel_editor_core::scene_project::SceneCommand;

use super::controls::tool_button;
use super::scene_model::{PanelFit, format_number};
use super::scene_panel::SliderRow;
use super::tokens::Tokens;
use super::{Source, Workstation};
use crate::state::{CommandError, Raw};

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

/// Presentation state of the section: the Kit sliders and the value a drag shows before release.
#[derive(Default)]
pub struct FxUi {
    sliders: HashMap<FxKey, FxSlider>,
    pending: HashMap<FxKey, f64>,
}

impl FxUi {
    pub fn clear(&mut self) {
        self.sliders.clear();
        self.pending.clear();
    }

    /// Drops every drag buffer that belongs to a scene other than `scene` (the panel moved on).
    pub fn retain_scene(&mut self, scene: &str) {
        self.pending.retain(|key, _| key.scene == scene);
    }

    #[cfg(test)]
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// A drag buffer as the slider would have left it (tests: the pointer path is covered separately).
    #[cfg(test)]
    pub fn hold(&mut self, key: FxKey, value: f64) {
        self.pending.insert(key, value);
    }
}

// ---- the model: what the section shows, with no GPUI ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Component {
    Whole,
    X,
    Y,
    R,
    G,
    B,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Param(&'static str, Component),
    Field(ClipField),
}

/// One slider of a clip: a number of a parameter (or one component of a point or a colour) or a
/// whole-frame field.
#[derive(Debug, Clone, PartialEq)]
pub struct FxControl {
    pub id: String,
    pub label: String,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub value: f64,
    pub integer: bool,
    pub unit: &'static str,
    target: Target,
}

/// The Portuguese label the section prints for a catalog parameter. The catalog keeps ids and
/// English summaries; a parameter without an entry here prints its id, never nothing.
fn param_label(id: &str) -> &str {
    match id {
        "intensity" => "Intensidade",
        "radius" => "Raio",
        "softness" => "Suavidade",
        "roundness" => "Arredondamento",
        "center" => "Centro",
        "color" => "Cor",
        other => other,
    }
}

fn effect_label(spec: &EffectSpec) -> &'static str {
    spec.label_pt
}

fn control_of(
    spec: &ParamSpec,
    id: String,
    label: String,
    component: Component,
    range: (f64, f64, f64),
    value: f64,
) -> FxControl {
    FxControl {
        id,
        label,
        min: range.0,
        max: range.1,
        step: range.2,
        value,
        integer: false,
        unit: "",
        target: Target::Param(spec.id, component),
    }
}

/// The sliders of `clip`, parameters first, then its frames. `edit_frames` bounds the frame
/// fields; the core still refuses what the layer rules refuse (overlap, envelope), with a typed
/// code on screen, so these bounds only keep the thumb inside what can ever be valid.
pub fn controls_of(clip: &FxClip, edit_frames: i64) -> Vec<FxControl> {
    let mut controls = Vec::new();
    for (index, spec) in clip.effect.params.iter().enumerate() {
        let Some(value) = clip.params.get(index).copied() else {
            continue;
        };
        let label = param_label(spec.id);
        match (spec.kind, value) {
            (ParamKind::Number { min, max, step, .. }, FxValue::Number(number)) => {
                controls.push(control_of(
                    spec,
                    spec.id.to_string(),
                    label.to_string(),
                    Component::Whole,
                    (min, max, step),
                    number,
                ));
            }
            (ParamKind::Point { min, max, .. }, FxValue::Point { x, y }) => {
                for (suffix, name, component, value) in
                    [("x", "X", Component::X, x), ("y", "Y", Component::Y, y)]
                {
                    controls.push(control_of(
                        spec,
                        format!("{}_{suffix}", spec.id),
                        format!("{label} {name}"),
                        component,
                        (min, max, 0.01),
                        value,
                    ));
                }
            }
            (ParamKind::Color { .. }, FxValue::Color { r, g, b }) => {
                for (suffix, name, component, value) in [
                    ("r", "R", Component::R, r),
                    ("g", "G", Component::G, g),
                    ("b", "B", Component::B, b),
                ] {
                    controls.push(control_of(
                        spec,
                        format!("{}_{suffix}", spec.id),
                        format!("{label} {name}"),
                        component,
                        (0.0, 1.0, 0.01),
                        value,
                    ));
                }
            }
            _ => {}
        }
    }
    // The strength of the effect first: it is the control people reach for.
    controls.sort_by_key(|control| control.id != "intensity");
    let frames = |field: ClipField, label: &str, min: i64, max: i64, value: i64| FxControl {
        id: field.name().to_string(),
        label: label.to_string(),
        min: min as f64,
        max: max.max(min) as f64,
        step: 1.0,
        value: value as f64,
        integer: true,
        unit: "q",
        target: Target::Field(field),
    };
    controls.push(frames(
        ClipField::StartFrame,
        "Início",
        0,
        edit_frames - 1,
        clip.start_frame,
    ));
    controls.push(frames(
        ClipField::DurationFrames,
        "Duração",
        1,
        edit_frames,
        clip.duration_frames,
    ));
    controls.push(frames(
        ClipField::FadeInFrames,
        "Fade in",
        0,
        clip.duration_frames,
        clip.fade_in_frames,
    ));
    controls.push(frames(
        ClipField::FadeOutFrames,
        "Fade out",
        0,
        clip.duration_frames,
        clip.fade_out_frames,
    ));
    controls
}

impl FxControl {
    /// What a slider thumb at `raw` means for this control: the step's grid (anchored at the minimum),
    /// inside the range, written in the decimals of the step. The Kit slider hands over an `f32`, so
    /// `0.4` arrives as `0.4000000059604645`; the document must hold `0.4`, not that. Only the pointer
    /// gesture quantizes: a value typed or sent through `set` is the author's own and is checked, not
    /// rounded, by the core.
    pub fn quantize(&self, raw: f64) -> f64 {
        if !raw.is_finite() || self.step <= 0.0 {
            return raw;
        }
        let steps = ((raw - self.min) / self.step).round();
        let snapped = (self.min + steps * self.step).clamp(self.min, self.max);
        if self.integer {
            return snapped.round();
        }
        // Decimals of the step (0.01 -> 2) remove the float dust of `min + n * step`.
        let decimals = (-self.step.log10().floor()).clamp(0.0, 9.0) as i32;
        let scale = 10f64.powi(decimals);
        ((snapped * scale).round() / scale).clamp(self.min, self.max)
    }

    /// The edit op that moves this control of `clip` to `value`. A component of a point or a
    /// colour keeps its siblings as the document has them.
    pub fn op(&self, clip: &FxClip, value: f64) -> Result<FxOp, CommandError> {
        let finite = |value: f64| -> Result<f64, CommandError> {
            value.is_finite().then_some(value).ok_or_else(|| {
                CommandError::invalid_value("Expected a finite number", json!("number"))
            })
        };
        let value = finite(value)?;
        match self.target {
            Target::Field(field) => {
                if value.fract() != 0.0 {
                    return Err(CommandError::invalid_value(
                        "Expected a whole number of frames",
                        json!("integer"),
                    ));
                }
                Ok(FxOp::SetField {
                    clip: clip.id.clone(),
                    field,
                    value: value as i64,
                })
            }
            Target::Param(param, component) => {
                let current = clip.param(param).ok_or_else(|| {
                    CommandError::new("unknown_param", format!("`{param}` is not a parameter"))
                })?;
                let written = match (current, component) {
                    (_, Component::Whole) => json!(value),
                    (FxValue::Point { y, .. }, Component::X) => json!({ "x": value, "y": y }),
                    (FxValue::Point { x, .. }, Component::Y) => json!({ "x": x, "y": value }),
                    (FxValue::Color { g, b, .. }, Component::R) => {
                        json!({ "r": value, "g": g, "b": b })
                    }
                    (FxValue::Color { r, b, .. }, Component::G) => {
                        json!({ "r": r, "g": value, "b": b })
                    }
                    (FxValue::Color { r, g, .. }, Component::B) => {
                        json!({ "r": r, "g": g, "b": value })
                    }
                    _ => {
                        return Err(CommandError::new(
                            "invalid_value",
                            format!("`{}` has no such component", self.id),
                        ));
                    }
                };
                Ok(FxOp::SetParam {
                    clip: clip.id.clone(),
                    param: param.to_string(),
                    value: written,
                })
            }
        }
    }
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
}

impl FxView {
    pub fn clip(&self, id: &str) -> Option<&ClipView> {
        self.clips.iter().find(|view| view.clip.id == id)
    }
}

pub const CLIP_PREFIX: &str = "scene.fx.clip:";
pub const SECTION_ID: &str = "scene.fx";
const ADD_PREFIX: &str = "add:";

pub fn clip_node_id(clip: &str) -> String {
    format!("{CLIP_PREFIX}{clip}")
}

// ---- the window: reads, writes, nodes, painting ----

impl Workstation {
    /// The effects of the scene the panel follows; `None` without an attached scene.
    pub(super) fn fx_view(&self) -> Option<FxView> {
        let id = self.scene_active_id()?;
        let attached = self.state.scenes.get(&id)?;
        let bundle = attached.scene.bundle();
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
            scene: id,
            edit_frames: i64::try_from(bundle.edit_duration_frames).unwrap_or(i64::MAX),
            clips,
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

    /// Adds a clip of `effect` at the start of the edit, on the first track where it fits.
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
            return Err(CommandError::new(
                "stale_gesture",
                format!(
                    "The effect gesture belonged to scene `{scene}` and the panel now shows `{}`: it was dropped, nothing changed",
                    view.scene
                ),
            )
            .target(scene));
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
        let view = self.fx_view().ok_or_else(no_scene)?;
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

    // ---- drag buffer ----

    pub(super) fn fx_slider_event(
        &mut self,
        key: FxKey,
        event: &SliderEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            SliderEvent::Change(value) => {
                self.scene_ui.fx.pending.insert(key, f64::from(value.end()));
                cx.notify();
            }
            SliderEvent::Release(_) => self.fx_commit_pending(&key, cx),
        }
    }

    /// The drag of `key` ends: one edit with the value the thumb shows, in the scene the gesture began
    /// on. A drag whose scene is no longer the one the panel shows, or whose clip is gone, is not
    /// applied to whatever is there now: it is dropped and the typed reason goes on screen like any
    /// refusal. A thumb that rests where the document already is sends nothing.
    fn fx_commit_pending(&mut self, key: &FxKey, cx: &mut Context<Self>) {
        let Some(value) = self.scene_ui.fx.pending.remove(key) else {
            return;
        };
        if let Err(error) = self.fx_commit_value(key, value, cx) {
            self.scene_report(error, cx);
        }
        cx.notify();
    }

    fn fx_commit_value(
        &mut self,
        key: &FxKey,
        value: f64,
        cx: &mut Context<Self>,
    ) -> Result<Option<Value>, CommandError> {
        let view = self.fx_view_of(&key.scene)?;
        let found = self.fx_known(&view, &key.clip)?;
        let controls = controls_of(&found.clip, view.edit_frames);
        let control = controls
            .iter()
            .find(|candidate| candidate.id == key.control)
            .ok_or_else(|| {
                CommandError::new(
                    "unknown_field",
                    format!(
                        "`{}` has no settable field `{}`",
                        clip_node_id(&key.clip),
                        key.control
                    ),
                )
            })?;
        let value = control.quantize(value);
        if (value - control.value).abs() < 1e-9 {
            // Nothing to write: no command, no history entry, and a stale notice is left as it was.
            return Ok(None);
        }
        let op = control.op(&found.clip, value)?;
        self.fx_run(&view.scene, op, Source::Gesture, cx).map(Some)
    }

    /// The window lost the focus: a release may never arrive, so what a drag shows is committed.
    pub(super) fn fx_focus_lost(&mut self, cx: &mut Context<Self>) {
        let keys: Vec<FxKey> = self.scene_ui.fx.pending.keys().cloned().collect();
        for key in keys {
            self.fx_commit_pending(&key, cx);
        }
    }

    /// Escape: the drag is dropped and the slider goes back to the document's value.
    pub(super) fn fx_cancel_drag(&mut self, cx: &mut Context<Self>) {
        if !self.scene_ui.fx.pending.is_empty() {
            self.scene_ui.fx.pending.clear();
            cx.notify();
        }
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
                        "field": control.id, "label": format!("{} {}", effect_label(clip.effect), control.label),
                        "type": "number", "value": control.value, "min": control.min,
                        "max": control.max, "step": control.step, "integer": control.integer,
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
                "actions": [
                    { "action": "remove", "label": "Remover efeito" },
                    { "action": "toggle_hidden", "label": if clip.hidden { "Mostrar efeito" } else { "Ocultar efeito" } },
                ],
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
        let actions: Vec<Value> = fx::catalog()
            .iter()
            .map(|spec| {
                json!({
                    "action": format!("{ADD_PREFIX}{}", spec.id),
                    "label": format!("Adicionar {}", spec.label_pt.to_lowercase()),
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
            other => Err(unknown_action(
                target,
                other.unwrap_or(""),
                &["remove".to_string(), "toggle_hidden".to_string()],
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
            let label = format!("Adicionar {}", spec.label_pt.to_lowercase());
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
        if view.clips.is_empty() {
            section = section.child(
                div()
                    .id("fx-empty")
                    .px(px(10.))
                    .pb(px(4.))
                    .text_size(px(10.))
                    .text_color(t.faint)
                    .child("Nenhum efeito na cena. Adicione uma vinheta para escurecer as bordas."),
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
            .child(remove);
        let mut block = v_flex().w_full().child(title);
        for control in controls_of(clip, view.edit_frames) {
            let key = FxKey {
                scene: view.scene.clone(),
                clip: clip.id.clone(),
                control: control.id.clone(),
            };
            let shown = self
                .scene_ui
                .fx
                .pending
                .get(&key)
                .copied()
                .unwrap_or(control.value);
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
                    label: &control.label,
                    modified: false,
                    value,
                    unit: control.unit,
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
        if stale && !self.scene_ui.fx.pending.contains_key(key) {
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
        if !self.scene_ui.fx.pending.contains_key(key) {
            let target = control.value as f32;
            if (state.read(cx).value().end() - target).abs() > SLIDER_EPSILON {
                state.update(cx, |slider, cx| slider.set_value(target, window, cx));
            }
        }
        let element = key.element_id();
        div()
            .id(SharedString::from(format!("fx-slider-{element}")))
            .relative()
            .flex_1()
            .child(self.geometry.probe(format!("control:{element}")))
            .child(
                Slider::new(&state)
                    .horizontal()
                    .accessibility_label(format!("{} {}", "Efeito", control.label)),
            )
            .into_any_element()
    }
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
