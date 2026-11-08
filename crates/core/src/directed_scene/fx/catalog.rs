//! The catalog of effects: what an effect is called, which parameters it has and their range.
//!
//! Data only, like the origin's `defineFxEffect` (pilot `scene-engine/src/fx/catalog/*`): the code
//! that draws an effect lives in the runtime, never in the bundle, and a bundle that names an
//! effect this catalog does not list is refused (`UNKNOWN_EFFECT`), not ignored. One effect is
//! entered so far (`vignette`); the table is the extension point (a second look is one more
//! `EffectSpec`, one more runtime shader and its pixel test).

use serde_json::{Value, json};

use super::FxValue;

/// A catalog parameter: its id (the key in the clip's `params`), kind, range and default.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamSpec {
    pub id: &'static str,
    pub kind: ParamKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamKind {
    /// A number in `[min, max]`. `step` is the UI granularity, not a validation rule. `unit`
    /// is `ratio` (a fraction of the frame's half-diagonal or of 1).
    Number {
        min: f64,
        max: f64,
        default: f64,
        step: f64,
        unit: &'static str,
    },
    /// `{ "r", "g", "b" }`, each channel in `[0, 1]`, in linear light (the pilot's `color` kind).
    Color { default: [f64; 3] },
    /// `{ "x", "y" }`, each in `[min, max]`, in picture fractions: `x` to the right, `y` DOWN
    /// (`0.5, 0.5` is the middle), the pilot's `vec2` with `unit: "screen"`.
    Point {
        min: f64,
        max: f64,
        default: [f64; 2],
    },
}

impl ParamKind {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Number { .. } => "number",
            Self::Color { .. } => "color",
            Self::Point { .. } => "vec2",
        }
    }

    pub fn default_value(&self) -> FxValue {
        match *self {
            Self::Number { default, .. } => FxValue::Number(default),
            Self::Color { default } => FxValue::Color {
                r: default[0],
                g: default[1],
                b: default[2],
            },
            Self::Point { default, .. } => FxValue::Point {
                x: default[0],
                y: default[1],
            },
        }
    }
}

/// One effect of the catalog.
#[derive(Debug, PartialEq)]
pub struct EffectSpec {
    pub id: &'static str,
    /// `look`: a pass over the picture (the pilot also has `transition`, not entered yet).
    pub family: &'static str,
    pub label_en: &'static str,
    pub label_pt: &'static str,
    pub summary_en: &'static str,
    /// A typical length in frames, the default of a clip added without one.
    pub typical_duration_frames: i64,
    pub params: &'static [ParamSpec],
}

impl EffectSpec {
    pub fn param(&self, id: &str) -> Option<(usize, &'static ParamSpec)> {
        self.params
            .iter()
            .enumerate()
            .find(|(_, param)| param.id == id)
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "family": self.family,
            "label": { "en": self.label_en, "pt-br": self.label_pt },
            "summary": self.summary_en,
            "typical_duration_frames": self.typical_duration_frames,
            "params": self.params.iter().map(param_json).collect::<Vec<_>>(),
        })
    }
}

fn param_json(param: &ParamSpec) -> Value {
    match param.kind {
        ParamKind::Number {
            min,
            max,
            default,
            step,
            unit,
        } => json!({
            "id": param.id, "kind": "number", "min": min, "max": max,
            "default": default, "step": step, "unit": unit,
        }),
        ParamKind::Color { default } => json!({
            "id": param.id, "kind": "color",
            "default": { "r": default[0], "g": default[1], "b": default[2] },
        }),
        ParamKind::Point { min, max, default } => json!({
            "id": param.id, "kind": "vec2", "min": min, "max": max,
            "default": { "x": default[0], "y": default[1] },
        }),
    }
}

/// Parameter order of the vignette: `radius`, `softness`, `intensity`, `color`, `roundness`,
/// `center`, as in the pilot's catalog. Indices into `VIGNETTE.params`.
pub const VIGNETTE_RADIUS: usize = 0;
pub const VIGNETTE_SOFTNESS: usize = 1;
pub const VIGNETTE_INTENSITY: usize = 2;
pub const VIGNETTE_COLOR: usize = 3;
pub const VIGNETTE_ROUNDNESS: usize = 4;
pub const VIGNETTE_CENTER: usize = 5;

/// `vignette`: darkens the edges with an elliptical falloff from `center`. The distance is
/// normalised so the farthest corner is 1 (`radius` 1 is dark only at the corners); `roundness`
/// blends the frame's ellipse (0) into a circle in pixels (1); the colour mixes in linear light.
pub static VIGNETTE: EffectSpec = EffectSpec {
    id: "vignette",
    family: "look",
    label_en: "Vignette",
    label_pt: "Vinheta",
    summary_en: "Darkens the edges of the frame with a soft elliptical falloff.",
    typical_duration_frames: 96,
    params: &[
        ParamSpec {
            id: "radius",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.5,
                default: 0.75,
                step: 0.01,
                unit: "ratio",
            },
        },
        ParamSpec {
            id: "softness",
            kind: ParamKind::Number {
                min: 0.05,
                max: 1.0,
                default: 0.45,
                step: 0.01,
                unit: "ratio",
            },
        },
        ParamSpec {
            id: "intensity",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.7,
                step: 0.01,
                unit: "ratio",
            },
        },
        ParamSpec {
            id: "color",
            kind: ParamKind::Color {
                default: [0.0, 0.0, 0.0],
            },
        },
        ParamSpec {
            id: "roundness",
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.6,
                step: 0.01,
                unit: "ratio",
            },
        },
        ParamSpec {
            id: "center",
            kind: ParamKind::Point {
                min: 0.0,
                max: 1.0,
                default: [0.5, 0.5],
            },
        },
    ],
};

static CATALOG: [&EffectSpec; 1] = [&VIGNETTE];

/// Every effect of this build, in catalog order.
pub fn catalog() -> &'static [&'static EffectSpec] {
    &CATALOG
}

/// The effect with this id.
pub fn effect(id: &str) -> Option<&'static EffectSpec> {
    CATALOG.iter().copied().find(|spec| spec.id == id)
}

/// The catalog as JSON, for `scene.fx.catalog`.
pub fn catalog_json() -> Value {
    json!({
        "catalog_version": super::FX_CATALOG_VERSION,
        "effects": catalog().iter().map(|spec| spec.to_json()).collect::<Vec<_>>(),
    })
}
