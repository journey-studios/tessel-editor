//! The inspector of the effects layer: a neutral, typed model of what a person can set on an
//! effect clip, derived from the catalog and the clip's own state (F016, P2, BL004/BL014/BL029/
//! BL040/BL082.1).
//!
//! WHY in the core: the window used to build its own list of sliders from the catalog (ids,
//! labels, ranges, the order, which one comes first) and so did nothing else; a script, the
//! JSONL answer and a test would have had to guess the same list. Here the model is one value —
//! [`ClipInspector`] — that the library returns, `scene.fx.inspect` serialises and the GPUI panel
//! paints, so a new effect in the catalog appears in all three without a line of adapter code and
//! no adapter owns a range, a step, a default or a name. The model carries no widget, no pixel,
//! no focus and no gesture state; it derives only from [`Fx`] and the length of the edit.
//!
//! Each field says what it is (`id`, the node field name), what it is called in three languages,
//! where it sits (`min`, `max`, `step`, `default`, `value`, `at_default`), whether it applies
//! (`applicable` with a typed `reason`) and how to reach it ([`InspectorField::op`] builds the
//! very [`FxOp`] a `scene.fx.edit` would run, [`InspectorField::reset_op`] the one that restores
//! the default). Editing stays where it was: the op goes through `apply_fx`, the gesture cycle or
//! `scene.fx.edit`, so the model owns no second editing path.

use serde_json::{Value, json};

use crate::directed_scene::Shot;
use crate::directed_scene::fx::{
    EffectSpec, Fx, FxClip, FxTrack, FxValue, Labels, Locale, ParamKind, catalog, default_duration,
    quantize_control,
};
use crate::state::CommandError;

use super::fx_choice::{InspectorChoice, scope_choice};
use super::{ClipField, FxOp};

/// The words of the timing fields, which every effect clip has and the catalog does not list.
pub const START_LABEL: Labels = Labels::new("Início", "Start", "Inicio");
pub const DURATION_LABEL: Labels = Labels::new("Duração", "Duration", "Duración");
pub const FADE_IN_LABEL: Labels = Labels::new("Fade in", "Fade in", "Fundido de entrada");
pub const FADE_OUT_LABEL: Labels = Labels::new("Fade out", "Fade out", "Fundido de salida");

/// The words of the part of a point or a colour a slider sets (`Centro X`, `Cor G`).
fn component_suffix(component: Component) -> &'static str {
    match component {
        Component::Whole => "",
        Component::X => " X",
        Component::Y => " Y",
        Component::R => " R",
        Component::G => " G",
        Component::B => " B",
    }
}

/// Which part of a parameter value a field sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Component {
    Whole,
    X,
    Y,
    R,
    G,
    B,
}

/// Where a field writes: a catalog parameter (or a component of one) or a whole-frame field of the
/// clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Param(&'static str, Component),
    Field(ClipField),
}

/// What kind of value a field holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldGroup {
    /// A parameter of the effect, from the catalog.
    Param,
    /// The clip's own timing (start, duration, fades), the same for every effect.
    Timing,
}

impl FieldGroup {
    pub fn name(self) -> &'static str {
        match self {
            Self::Param => "param",
            Self::Timing => "timing",
        }
    }
}

/// What a field holds: a scalar the adapter edits with a slider or a text box, or a switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Number,
    /// A boolean parameter (`colored`): the value is 0 or 1 here, the written value `false`/`true`.
    Boolean,
}

impl FieldKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Number => "number",
            Self::Boolean => "boolean",
        }
    }
}

/// Why a field does not apply to the clip as it stands (a code a person and a script can read).
pub const REASON_SINGLE_VALUE: &str = "range_is_a_single_value";

/// Why the BASE value of a parameter is not edited from its control: keys prevail over it, so a
/// slider on the base would move something the picture does not show. The keys are the way to
/// edit an animated parameter from the window; the library's `set_param` still writes the base
/// (and answers with `FX_STATIC_UNDER_KEYS`).
pub const REASON_ANIMATED: &str = "base_under_keys";

/// One setting of a clip: a number of a parameter (or one component of a point or a colour) or a
/// whole-frame field. Every field is a scalar the adapter can show with a slider or a text box.
#[derive(Debug, Clone, PartialEq)]
pub struct InspectorField {
    /// The node field name: `intensity`, `center_x`, `color_r`, `fade_in_frames`...
    pub id: String,
    pub group: FieldGroup,
    pub kind: FieldKind,
    /// The name without the component (`Centro`); the full text comes from [`Self::label`].
    pub base: Labels,
    component: Component,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub value: f64,
    /// The value that a reset restores. `None` when the field has no default (the start frame).
    pub default: Option<f64>,
    /// Whole-frame fields come out as integers.
    pub integer: bool,
    /// `ratio` for a fraction, `frames` for whole frames, `` otherwise.
    pub unit: &'static str,
    /// Whether setting this field means anything for the clip as it stands.
    pub applicable: bool,
    /// Why it does not apply (`REASON_*`), only when `applicable` is false.
    pub reason: Option<&'static str>,
    /// Keys prevail over this parameter's base (`value` is the BASE, what shows where the curve
    /// does not reach). A control built from this field edits the keys, not `value`; see
    /// [`REASON_ANIMATED`]. Always false for a timing field.
    pub animated: bool,
    target: Target,
}

impl InspectorField {
    /// The text of the field in `locale`: the parameter's name plus the part it sets (`Centro X`).
    pub fn label(&self, locale: Locale) -> String {
        format!(
            "{}{}",
            self.base.get(locale),
            component_suffix(self.component)
        )
    }

    pub fn target(&self) -> Target {
        self.target
    }

    /// The scalar this field holds of a whole parameter value (the `x` of a point, the `g` of a
    /// colour, a switch as 0 or 1): what a key or a sampled value shows in this field's control.
    /// `None` for a value of another kind, and for a timing field.
    pub fn read(&self, value: FxValue) -> Option<f64> {
        match (value, self.component) {
            (FxValue::Number(number), Component::Whole) => Some(number),
            (FxValue::Bool(on), Component::Whole) => Some(f64::from(u8::from(on))),
            (FxValue::Point { x, .. }, Component::X) => Some(x),
            (FxValue::Point { y, .. }, Component::Y) => Some(y),
            (FxValue::Color { r, .. }, Component::R) => Some(r),
            (FxValue::Color { g, .. }, Component::G) => Some(g),
            (FxValue::Color { b, .. }, Component::B) => Some(b),
            _ => None,
        }
    }

    /// The scalar of `value` in this field's control, for the key of this field's parameter at a
    /// frame: the same as [`Self::read`] with the parameter named, so a caller can pick the right
    /// field of a parameter that has several (`center_x`, `center_y`).
    pub fn param(&self) -> Option<&'static str> {
        match self.target {
            Target::Param(param, _) => Some(param),
            Target::Field(_) => None,
        }
    }

    /// Whether the field holds its default. A field without a default is never "at default".
    pub fn at_default(&self) -> bool {
        self.default
            .is_some_and(|default| (default - self.value).abs() < 1e-9)
    }

    /// What a pointer or a key at `raw` means for this field: the step's grid (anchored at the
    /// minimum), inside the range, written in the decimals of the step. The rule is the core's
    /// ([`quantize_control`]), so the pointer, the keyboard and a script agree. Only gestures
    /// quantize: a value typed or sent through `set` is the author's own and is checked, not
    /// rounded, by the core.
    pub fn quantize(&self, raw: f64) -> f64 {
        quantize_control(raw, self.min, self.max, self.step, self.integer)
    }

    /// The edit op that moves this field of `clip` to `value`. A component of a point or a colour
    /// keeps its siblings as the clip has them. Refuses, with a typed code, a value that is not
    /// finite, or not whole for a frame field; the range and the layer's rules are the core's
    /// (`scene.fx.edit`), which judges the op when it runs.
    pub fn op(&self, clip: &FxClip, value: f64) -> Result<FxOp, CommandError> {
        if !value.is_finite() {
            return Err(CommandError::invalid_value(
                "Expected a finite number",
                json!("number"),
            ));
        }
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
                Ok(FxOp::SetParam {
                    clip: clip.id.clone(),
                    param: param.to_string(),
                    value: self.written(current, component, value)?,
                })
            }
        }
    }

    /// The whole value of the parameter this field belongs to, with this field's part set to
    /// `value` and the other parts as `current` has them: what a `set_param` or a `key_param`
    /// carries. A switch takes 0 or 1 only.
    fn written(
        &self,
        current: FxValue,
        component: Component,
        value: f64,
    ) -> Result<Value, CommandError> {
        Ok(match (current, component) {
            (FxValue::Bool(_), Component::Whole) => match value {
                0.0 => json!(false),
                1.0 => json!(true),
                _ => {
                    return Err(CommandError::invalid_value(
                        "Expected 0 or 1 for a switch",
                        json!("boolean"),
                    ));
                }
            },
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
        })
    }

    /// The op that puts a key of this field's parameter on `frame_in_clip`, at `value` for this
    /// field's part. The other parts of a point or a colour are what the parameter is at that
    /// frame (its curve, or its base), so keying one channel never moves the others. A timing
    /// field has no keys: refused with `not_applicable`.
    pub fn key_op(
        &self,
        clip: &FxClip,
        frame_in_clip: i64,
        value: f64,
    ) -> Result<FxOp, CommandError> {
        if !value.is_finite() {
            return Err(CommandError::invalid_value(
                "Expected a finite number",
                json!("number"),
            ));
        }
        let Target::Param(param, component) = self.target else {
            return Err(CommandError::new(
                "not_applicable",
                format!("`{}` is a timing field: only parameters have keys", self.id),
            ));
        };
        let (index, _) = clip.effect.param(param).ok_or_else(|| {
            CommandError::new("unknown_param", format!("`{param}` is not a parameter"))
        })?;
        let current = clip
            .value_at(index, frame_in_clip)
            .ok_or_else(|| CommandError::new("unknown_param", format!("`{param}` has no value")))?;
        Ok(FxOp::KeyParam {
            clip: clip.id.clone(),
            param: param.to_string(),
            frame: frame_in_clip,
            value: Some(self.written(current, component, value)?),
            easing: None,
        })
    }

    /// The op that restores the default of this field, or `None` when it has no default or the
    /// clip already holds it (a reset of a field at its default is nothing to do, not an edit).
    pub fn reset_op(&self, clip: &FxClip) -> Option<FxOp> {
        let default = self.default?;
        if self.at_default() {
            return None;
        }
        self.op(clip, default).ok()
    }

    /// A directly executable JSONL operation for this field, preserving the other components.
    pub fn reset_op_json(&self, clip: &FxClip) -> Option<Value> {
        self.reset_op(clip).and_then(|op| match op {
            FxOp::SetParam { clip, param, value } => {
                Some(json!({ "op": "set_param", "clip": clip, "param": param, "value": value }))
            }
            FxOp::SetField { clip, field, value } => Some(
                json!({ "op": "set_field", "clip": clip, "field": field.name(), "value": value }),
            ),
            _ => None,
        })
    }

    pub fn to_json(&self, locale: Locale) -> Value {
        let labels = json!({
            "pt-br": self.label(Locale::PtBr), "en": self.label(Locale::En),
            "es": self.label(Locale::Es),
        });
        if self.kind == FieldKind::Boolean {
            // A switch has no range or step to show: `true`/`false`, its default and the reset.
            return json!({
                "id": self.id, "group": self.group.name(),
                "label": self.label(locale), "labels": labels,
                "type": "boolean", "value": self.value != 0.0,
                "default": self.default.map(|default| default != 0.0),
                "at_default": self.at_default(),
                "applicable": self.applicable, "reason": self.reason,
                "animated": self.animated,
                "base_edit": self.base_edit_json(),
                "reset": self.default.map(|default| json!({
                    "to": default != 0.0, "needed": !self.at_default(),
                })),
            });
        }
        json!({
            "id": self.id, "group": self.group.name(),
            "label": self.label(locale), "labels": labels,
            "type": "number", "value": self.value, "min": self.min, "max": self.max,
            "step": self.step, "default": self.default, "at_default": self.at_default(),
            "integer": self.integer, "unit": self.unit,
            "applicable": self.applicable, "reason": self.reason,
            "animated": self.animated,
            "base_edit": self.base_edit_json(),
            "reset": self.default.map(|default| json!({ "to": default, "needed": !self.at_default() })),
        })
    }

    /// Whether the control of this field edits the base: not while keys prevail over it. The
    /// reset of the base stays (it is an explicit write of the base, and keeps the curve).
    fn base_edit_json(&self) -> Value {
        let enabled = self.applicable && !self.animated;
        json!({
            "enabled": enabled,
            "reason": if self.animated { Some(REASON_ANIMATED) } else { self.reason },
            "edit_with": self.animated.then_some("keys"),
        })
    }
}

/// The inspector of one effect clip.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipInspector {
    pub clip: String,
    pub track: String,
    pub track_label: String,
    pub effect: &'static EffectSpec,
    pub hidden: bool,
    /// Parameters first (`intensity` ahead of the others, the control people reach for), then
    /// the timing fields, so every adapter lists them in one order.
    pub fields: Vec<InspectorField>,
    /// The settings that are one of a few options rather than a number (F062: the scope).
    pub choices: Vec<InspectorChoice>,
}

impl ClipInspector {
    /// The inspector of `clip` on `track`, against an edit of `edit_frames` frames (the bounds of
    /// the timing fields; the layer's own rules — overlap, envelope — still judge each op) whose
    /// shots are `shots` (the options of the scope choice).
    pub fn of(track: &FxTrack, clip: &FxClip, edit_frames: i64, shots: &[Shot]) -> Self {
        Self {
            clip: clip.id.clone(),
            track: track.id.clone(),
            track_label: track.label.clone(),
            effect: clip.effect,
            hidden: clip.hidden,
            fields: fields_of(clip, edit_frames, shots),
            choices: vec![scope_choice(clip, shots)],
        }
    }

    pub fn field(&self, id: &str) -> Option<&InspectorField> {
        self.fields.iter().find(|field| field.id == id)
    }

    /// The ops that restore every *parameter* that is not at its default, in field order: one
    /// atomic `scene.fx.edit` resets the effect's look. The timing (start, duration, fades) is
    /// not part of it: putting a clip back to its defaults must not move or resize it; each
    /// timing field has its own [`InspectorField::reset_op`].
    pub fn reset_ops(&self, clip: &FxClip) -> Vec<FxOp> {
        reset_ops(&self.fields, clip)
    }

    /// [`Self::reset_ops`] as the `scene.fx.edit` ops a script sends to do the same reset
    /// (`op`, `clip`, `param`, `value`): the answer of `scene.fx.inspect`, so the JSONL surface
    /// resets with the very ops the window runs.
    pub fn reset_ops_json(&self, clip: &FxClip) -> Vec<Value> {
        self.reset_ops(clip)
            .into_iter()
            .filter_map(|op| match op {
                FxOp::SetParam { clip, param, value } => {
                    Some(json!({ "op": "set_param", "clip": clip, "param": param, "value": value }))
                }
                _ => None,
            })
            .collect()
    }

    /// Whether any parameter is away from its default (what a "reset" would change).
    pub fn has_reset(&self) -> bool {
        has_reset(&self.fields)
    }

    pub fn to_json(&self, locale: Locale) -> Value {
        json!({
            "clip": self.clip, "track": self.track, "track_label": self.track_label,
            "effect": self.effect.id,
            "effect_label": self.effect.label.get(locale),
            "effect_labels": self.effect.label.to_json(),
            "hidden": self.hidden,
            "fields": self.fields.iter().map(|field| field.to_json(locale)).collect::<Vec<_>>(),
            "choices": self.choices.iter()
                .map(|choice| choice.to_json(&self.clip, locale))
                .collect::<Vec<_>>(),
        })
    }
}

/// Whether the visual parameters have anything to reset; shared by every adapter.
pub fn has_reset(fields: &[InspectorField]) -> bool {
    fields.iter().any(|field| {
        field.group == FieldGroup::Param && field.default.is_some() && !field.at_default()
    })
}

/// The ops that restore every *parameter* among `fields` that is away from its default, as one
/// op per parameter. The components of a point or a colour (`center_x`/`center_y`, `color_r`/`_g`/`_b`)
/// are fields of one parameter: their resets are merged into ONE `set_param` that carries every
/// component's default, because one op per component would each be built from the clip as it
/// stands and the last would overwrite the others. Timing fields are never part of it.
pub fn reset_ops(fields: &[InspectorField], clip: &FxClip) -> Vec<FxOp> {
    let mut ops: Vec<FxOp> = Vec::new();
    let mut merged: Vec<&'static str> = Vec::new();
    for field in fields
        .iter()
        .filter(|field| field.group == FieldGroup::Param)
    {
        let Target::Param(param, component) = field.target else {
            continue;
        };
        if component == Component::Whole {
            ops.extend(field.reset_op(clip));
            continue;
        }
        if merged.contains(&param) {
            continue;
        }
        let siblings: Vec<&InspectorField> = fields
            .iter()
            .filter(|other| matches!(other.target, Target::Param(name, _) if name == param))
            .collect();
        if siblings.iter().all(|sibling| sibling.at_default()) {
            continue;
        }
        merged.push(param);
        // Every component goes to its default, in one value.
        let mut value = match clip.param(param) {
            Some(FxValue::Point { x, y }) => [x, y, 0.0],
            Some(FxValue::Color { r, g, b }) => [r, g, b],
            _ => continue,
        };
        for sibling in &siblings {
            let Some(default) = sibling.default else {
                continue;
            };
            let slot = match sibling.target {
                Target::Param(_, Component::X | Component::R) => 0,
                Target::Param(_, Component::Y | Component::G) => 1,
                Target::Param(_, Component::B) => 2,
                _ => continue,
            };
            value[slot] = default;
        }
        let written = match clip.param(param) {
            Some(FxValue::Point { .. }) => json!({ "x": value[0], "y": value[1] }),
            _ => json!({ "r": value[0], "g": value[1], "b": value[2] }),
        };
        ops.push(FxOp::SetParam {
            clip: clip.id.clone(),
            param: param.to_string(),
            value: written,
        });
    }
    ops
}

/// What a scalar field is made of before the rules of applicability are read from its range.
struct Draft {
    id: String,
    group: FieldGroup,
    base: Labels,
    component: Component,
    range: (f64, f64, f64),
    value: f64,
    default: Option<f64>,
    target: Target,
}

fn scalar(draft: Draft) -> InspectorField {
    let (min, max, step) = draft.range;
    let applicable = max > min;
    InspectorField {
        id: draft.id,
        group: draft.group,
        kind: FieldKind::Number,
        base: draft.base,
        component: draft.component,
        min,
        max,
        step,
        value: draft.value,
        default: draft.default,
        integer: false,
        unit: "ratio",
        applicable,
        reason: (!applicable).then_some(REASON_SINGLE_VALUE),
        animated: false,
        target: draft.target,
    }
}

/// The fields of `clip`, in the order of [`ClipInspector::fields`].
fn fields_of(clip: &FxClip, edit_frames: i64, shots: &[Shot]) -> Vec<InspectorField> {
    let mut fields = Vec::new();
    for (index, spec) in clip.effect.params.iter().enumerate() {
        let Some(value) = clip.params.get(index).copied() else {
            continue;
        };
        match (spec.kind, value) {
            (
                ParamKind::Number {
                    min,
                    max,
                    step,
                    default,
                    unit,
                },
                FxValue::Number(number),
            ) => {
                let mut field = scalar(Draft {
                    id: spec.id.to_string(),
                    group: FieldGroup::Param,
                    base: spec.label,
                    component: Component::Whole,
                    range: (min, max, step),
                    value: number,
                    default: Some(default),
                    target: Target::Param(spec.id, Component::Whole),
                });
                field.unit = unit;
                fields.push(field);
            }
            (
                ParamKind::Point {
                    min, max, default, ..
                },
                FxValue::Point { x, y },
            ) => {
                for (suffix, component, value, default) in [
                    ("x", Component::X, x, default[0]),
                    ("y", Component::Y, y, default[1]),
                ] {
                    fields.push(scalar(Draft {
                        id: format!("{}_{suffix}", spec.id),
                        group: FieldGroup::Param,
                        base: spec.label,
                        component,
                        range: (min, max, 0.01),
                        value,
                        default: Some(default),
                        target: Target::Param(spec.id, component),
                    }));
                }
            }
            (ParamKind::Color { default }, FxValue::Color { r, g, b }) => {
                for (suffix, component, value, default) in [
                    ("r", Component::R, r, default[0]),
                    ("g", Component::G, g, default[1]),
                    ("b", Component::B, b, default[2]),
                ] {
                    fields.push(scalar(Draft {
                        id: format!("{}_{suffix}", spec.id),
                        group: FieldGroup::Param,
                        base: spec.label,
                        component,
                        range: (0.0, 1.0, 0.01),
                        value,
                        default: Some(default),
                        target: Target::Param(spec.id, component),
                    }));
                }
            }
            (ParamKind::Bool { default }, FxValue::Bool(on)) => {
                let mut field = scalar(Draft {
                    id: spec.id.to_string(),
                    group: FieldGroup::Param,
                    base: spec.label,
                    component: Component::Whole,
                    range: (0.0, 1.0, 1.0),
                    value: f64::from(u8::from(on)),
                    default: Some(f64::from(u8::from(default))),
                    target: Target::Param(spec.id, Component::Whole),
                });
                field.kind = FieldKind::Boolean;
                field.integer = true;
                field.unit = "";
                fields.push(field);
            }
            _ => {}
        }
    }
    for field in &mut fields {
        if let Target::Param(param, _) = field.target {
            field.animated = clip.curve_of(param).is_some_and(|keys| !keys.is_empty());
        }
    }
    // The strength of the effect first: it is the control people reach for.
    fields.sort_by_key(|field| field.id != "intensity");
    let start = clip.start_frame;
    let typical = default_duration(clip.effect, &clip.scope, start, shots, edit_frames);
    let frames =
        |field: ClipField, base: Labels, min: i64, max: i64, value: i64, default: Option<i64>| {
            let mut built = scalar(Draft {
                id: field.name().to_string(),
                group: FieldGroup::Timing,
                base,
                component: Component::Whole,
                range: (min as f64, max.max(min) as f64, 1.0),
                value: value as f64,
                default: default.map(|default| default as f64),
                target: Target::Field(field),
            });
            built.integer = true;
            built.unit = "frames";
            built
        };
    fields.push(frames(
        ClipField::StartFrame,
        START_LABEL,
        0,
        edit_frames - 1,
        clip.start_frame,
        None,
    ));
    fields.push(frames(
        ClipField::DurationFrames,
        DURATION_LABEL,
        1,
        edit_frames,
        clip.duration_frames,
        Some(typical),
    ));
    fields.push(frames(
        ClipField::FadeInFrames,
        FADE_IN_LABEL,
        0,
        clip.duration_frames,
        clip.fade_in_frames,
        Some(0),
    ));
    fields.push(frames(
        ClipField::FadeOutFrames,
        FADE_OUT_LABEL,
        0,
        clip.duration_frames,
        clip.fade_out_frames,
        Some(0),
    ));
    fields
}

/// One effect an author may add (the catalog entry as the inspector offers it).
#[derive(Debug, Clone, PartialEq)]
pub struct Offer {
    pub effect: &'static EffectSpec,
    pub applicable: bool,
    /// Why it cannot be added now, only when not `applicable`.
    pub reason: Option<&'static str>,
}

/// Why an effect cannot be added.
pub const REASON_FAMILY_NOT_YET: &str = "FX_TRANSITION_NOT_YET";

/// Every effect of the catalog as an offer for the output scope, in catalog order. A family the
/// layer does not draw yet (a transition) is listed as not applicable with its refusal code, never
/// silently left out and never enabled.
pub fn offers() -> Vec<Offer> {
    catalog()
        .iter()
        .map(|effect| {
            let applicable = effect.family == "look";
            Offer {
                effect,
                applicable,
                reason: (!applicable).then_some(REASON_FAMILY_NOT_YET),
            }
        })
        .collect()
}

impl Offer {
    pub fn to_json(&self, locale: Locale) -> Value {
        json!({
            "effect": self.effect.id, "family": self.effect.family,
            "label": self.effect.label.get(locale), "labels": self.effect.label.to_json(),
            "summary": self.effect.summary.get(locale),
            "typical_duration_frames": self.effect.typical_duration_frames,
            "applicable": self.applicable, "reason": self.reason,
            "scope": "output",
        })
    }
}

/// The fields of a clip without the track around it (a window that already holds the clip): the
/// same list [`ClipInspector::of`] returns.
pub fn inspector_fields(clip: &FxClip, edit_frames: i64, shots: &[Shot]) -> Vec<InspectorField> {
    fields_of(clip, edit_frames, shots)
}

/// The inspector of a whole layer: the offers and one [`ClipInspector`] per clip. The data of
/// `scene.fx.inspect` before the gesture and the probe are added. `shots` are the edit's, for the
/// options of each clip's scope choice.
pub fn layer_inspector(fx: &Fx, edit_frames: i64, shots: &[Shot]) -> Vec<ClipInspector> {
    fx.tracks
        .iter()
        .flat_map(|track| {
            track
                .clips
                .iter()
                .map(move |clip| ClipInspector::of(track, clip, edit_frames, shots))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use super::*;
    use crate::directed_scene::fx::{FADE, FxScope, VIGNETTE};

    fn clip_of(spec: &'static EffectSpec) -> (FxTrack, FxClip) {
        let clip = FxClip {
            id: "c1".into(),
            effect: spec,
            start_frame: 10,
            duration_frames: 20,
            params: spec
                .params
                .iter()
                .map(|param| param.kind.default_value())
                .collect(),
            keys: Vec::new(),
            fade_in_frames: 0,
            fade_out_frames: 0,
            seed: None,
            hidden: false,
            scope: FxScope::Output,
        };
        let track = FxTrack {
            id: "t1".into(),
            label: "FX 1".into(),
            hidden: false,
            clips: vec![clip.clone()],
        };
        (track, clip)
    }

    #[test]
    fn every_offered_effect_has_every_text_in_three_languages() {
        for effect in catalog() {
            for locale in Locale::ALL {
                assert!(
                    !effect.label.get(locale).trim().is_empty(),
                    "{} has no {} label",
                    effect.id,
                    locale.tag()
                );
                assert!(!effect.summary.get(locale).trim().is_empty());
            }
            for param in effect.params {
                for locale in Locale::ALL {
                    assert!(
                        !param.label.get(locale).trim().is_empty(),
                        "{}.{} has no {} label",
                        effect.id,
                        param.id,
                        locale.tag()
                    );
                }
            }
        }
        for timing in [START_LABEL, DURATION_LABEL, FADE_IN_LABEL, FADE_OUT_LABEL] {
            for locale in Locale::ALL {
                assert!(!timing.get(locale).is_empty());
            }
        }
    }

    #[test]
    fn a_vignette_is_listed_with_intensity_first_and_the_timing_last() {
        let (track, clip) = clip_of(&VIGNETTE);
        let inspector = ClipInspector::of(&track, &clip, 96, &[]);
        let ids: Vec<&str> = inspector
            .fields
            .iter()
            .map(|field| field.id.as_str())
            .collect();
        assert_eq!(
            ids,
            [
                "intensity",
                "radius",
                "softness",
                "color_r",
                "color_g",
                "color_b",
                "roundness",
                "center_x",
                "center_y",
                "start_frame",
                "duration_frames",
                "fade_in_frames",
                "fade_out_frames",
            ]
        );
        assert!(inspector.field("intensity").unwrap().at_default());
    }

    #[test]
    fn a_fade_is_listed_by_the_same_rules_with_no_special_case() {
        let (track, clip) = clip_of(&FADE);
        let inspector = ClipInspector::of(&track, &clip, 96, &[]);
        let ids: Vec<&str> = inspector
            .fields
            .iter()
            .map(|field| field.id.as_str())
            .collect();
        assert_eq!(
            ids,
            [
                "intensity",
                "color_r",
                "color_g",
                "color_b",
                "amount",
                "start_frame",
                "duration_frames",
                "fade_in_frames",
                "fade_out_frames",
            ]
        );
        let amount = inspector.field("amount").unwrap();
        assert_eq!(
            (amount.min, amount.max, amount.default),
            (0.0, 1.0, Some(1.0))
        );
        assert_eq!(amount.label(Locale::Es), "Cantidad");
    }

    #[test]
    fn a_field_away_from_its_default_resets_to_it_and_one_at_it_has_nothing_to_reset() {
        let (track, mut clip) = clip_of(&VIGNETTE);
        clip.params[2] = FxValue::Number(0.2);
        let inspector = ClipInspector::of(&track, &clip, 96, &[]);
        let intensity = inspector.field("intensity").unwrap();
        assert!(!intensity.at_default());
        assert_eq!(
            intensity.reset_op(&clip),
            Some(FxOp::SetParam {
                clip: "c1".into(),
                param: "intensity".into(),
                value: json!(0.7),
            })
        );
        assert_eq!(inspector.field("radius").unwrap().reset_op(&clip), None);
        // The look resets as one batch of its parameters and never touches the timing.
        assert!(inspector.has_reset());
        assert_eq!(inspector.reset_ops(&clip).len(), 1);
        // The start frame has no default, so it is never reset.
        assert_eq!(
            inspector.field("start_frame").unwrap().reset_op(&clip),
            None
        );
        // The typical 96 is cut where the edit ends, 86 from frame 10; the duration of 20 is not it.
        let duration = inspector.field("duration_frames").unwrap();
        assert_eq!(duration.default, Some(86.0));
        assert!(!duration.at_default());
    }

    #[test]
    fn resetting_several_components_of_one_parameter_is_one_op_that_carries_them_all() {
        // A colour moved in all three channels and a point moved on both axes: one op per
        // parameter, each carrying every default (one op per component would overwrite the others).
        let (track, mut clip) = clip_of(&VIGNETTE);
        clip.params[3] = FxValue::Color {
            r: 1.0,
            g: 0.5,
            b: 0.25,
        };
        clip.params[5] = FxValue::Point { x: 0.1, y: 0.9 };
        let inspector = ClipInspector::of(&track, &clip, 96, &[]);
        let ops = inspector.reset_ops(&clip);
        assert_eq!(
            ops,
            [
                FxOp::SetParam {
                    clip: "c1".into(),
                    param: "color".into(),
                    value: json!({ "r": 0.0, "g": 0.0, "b": 0.0 }),
                },
                FxOp::SetParam {
                    clip: "c1".into(),
                    param: "center".into(),
                    value: json!({ "x": 0.5, "y": 0.5 }),
                },
            ]
        );
        // Only one channel away (and the point home): the others are written as they are.
        clip.params[5] = FxValue::Point { x: 0.5, y: 0.5 };
        clip.params[3] = FxValue::Color {
            r: 0.0,
            g: 0.5,
            b: 0.0,
        };
        let inspector = ClipInspector::of(&track, &clip, 96, &[]);
        assert_eq!(
            inspector.reset_ops(&clip),
            [FxOp::SetParam {
                clip: "c1".into(),
                param: "color".into(),
                value: json!({ "r": 0.0, "g": 0.0, "b": 0.0 }),
            }]
        );
        // Nothing away: nothing to do.
        clip.params[3] = FxValue::Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
        };
        clip.params[5] = FxValue::Point { x: 0.5, y: 0.5 };
        let inspector = ClipInspector::of(&track, &clip, 96, &[]);
        assert!(inspector.reset_ops(&clip).is_empty());
    }

    #[test]
    fn a_component_op_keeps_its_siblings_and_a_frame_op_refuses_a_fraction() {
        let (track, mut clip) = clip_of(&VIGNETTE);
        clip.params[5] = FxValue::Point { x: 0.2, y: 0.9 };
        let inspector = ClipInspector::of(&track, &clip, 96, &[]);
        let op = inspector.field("center_x").unwrap().op(&clip, 0.4).unwrap();
        assert_eq!(
            op,
            FxOp::SetParam {
                clip: "c1".into(),
                param: "center".into(),
                value: json!({"x": 0.4, "y": 0.9}),
            }
        );
        let error = inspector
            .field("start_frame")
            .unwrap()
            .op(&clip, 3.5)
            .unwrap_err();
        assert_eq!(error.code, "invalid_value");
        let error = inspector
            .field("intensity")
            .unwrap()
            .op(&clip, f64::NAN)
            .unwrap_err();
        assert_eq!(error.code, "invalid_value");
    }

    #[test]
    fn a_range_of_one_value_is_not_applicable_with_a_reason() {
        let (track, clip) = clip_of(&VIGNETTE);
        let inspector = ClipInspector::of(&track, &clip, 1, &[]);
        let start = inspector.field("start_frame").unwrap();
        assert!(!start.applicable);
        assert_eq!(start.reason, Some(REASON_SINGLE_VALUE));
        assert!(inspector.field("intensity").unwrap().applicable);
    }

    #[test]
    fn offers_cover_the_catalog() {
        let listed: Vec<&str> = offers().iter().map(|offer| offer.effect.id).collect();
        let catalog_ids: Vec<&str> = catalog().iter().map(|effect| effect.id).collect();
        assert_eq!(listed, catalog_ids);
        assert!(offers().iter().all(|offer| offer.applicable));
    }

    #[test]
    fn locales_parse_the_three_tags() {
        assert_eq!(Locale::parse("pt-BR"), Some(Locale::PtBr));
        assert_eq!(Locale::parse("pt_br"), Some(Locale::PtBr));
        assert_eq!(Locale::parse("EN"), Some(Locale::En));
        assert_eq!(Locale::parse("es"), Some(Locale::Es));
        assert_eq!(Locale::parse("fr"), None);
    }
}
