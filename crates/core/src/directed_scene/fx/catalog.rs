//! The catalog of effects: what an effect is called, which parameters it has and their range.
//!
//! Data only, like the origin's `defineFxEffect` (pilot `scene-engine/src/fx/catalog/*`): the code
//! that draws an effect lives in the runtime, never in the bundle, and a bundle that names an
//! effect this catalog does not list is refused (`UNKNOWN_EFFECT`), not ignored. Three looks are
//! entered so far (`vignette`, `fade`, `filmGrain`); the table is the extension point (a look is
//! one more `EffectSpec`, one more runtime shader and its pixel test).

use serde_json::{Value, json};

use super::{FxValue, Reseed};

/// A language the editor can print the catalog in. `pt-br` is the editor's own; `en` and `es` are
/// the other two languages of the pilot's catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Locale {
    #[default]
    PtBr,
    En,
    Es,
}

impl Locale {
    pub const ALL: [Self; 3] = [Self::PtBr, Self::En, Self::Es];

    /// The tag used in JSON keys and requests.
    pub fn tag(self) -> &'static str {
        match self {
            Self::PtBr => "pt-br",
            Self::En => "en",
            Self::Es => "es",
        }
    }

    /// `pt-br`, `pt`, `en`, `es` in any case (also `pt_BR`); anything else is not a locale.
    pub fn parse(tag: &str) -> Option<Self> {
        match tag.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "pt-br" | "pt" => Some(Self::PtBr),
            "en" | "en-us" | "en-gb" => Some(Self::En),
            "es" | "es-es" | "es-419" => Some(Self::Es),
            _ => None,
        }
    }
}

/// One text in the three languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Labels {
    pub pt_br: &'static str,
    pub en: &'static str,
    pub es: &'static str,
}

impl Labels {
    pub const fn new(pt_br: &'static str, en: &'static str, es: &'static str) -> Self {
        Self { pt_br, en, es }
    }

    pub fn get(&self, locale: Locale) -> &'static str {
        match locale {
            Locale::PtBr => self.pt_br,
            Locale::En => self.en,
            Locale::Es => self.es,
        }
    }

    pub fn to_json(&self) -> Value {
        json!({ "pt-br": self.pt_br, "en": self.en, "es": self.es })
    }
}

/// A catalog parameter: its id (the key in the clip's `params`), name, kind, range and default.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamSpec {
    pub id: &'static str,
    pub label: Labels,
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
    /// `true | false` (the pilot's `boolean`). A number is never a boolean: `0`/`1` are refused.
    Bool { default: bool },
}

impl ParamKind {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Number { .. } => "number",
            Self::Color { .. } => "color",
            Self::Point { .. } => "vec2",
            Self::Bool { .. } => "boolean",
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
            Self::Bool { default } => FxValue::Bool(default),
        }
    }
}

/// One effect of the catalog.
#[derive(Debug, PartialEq)]
pub struct EffectSpec {
    pub id: &'static str,
    /// `look`: a pass over the picture (the pilot also has `transition`, not entered yet).
    pub family: &'static str,
    pub label: Labels,
    pub summary: Labels,
    /// A typical length in frames, the default of a clip added without one.
    pub typical_duration_frames: i64,
    /// Whether the effect draws noise and how its seed moves over the clip (the pilot's
    /// `needs: ["seed"]` and `reseed`). Only effects that need a seed carry one in the plan.
    pub reseed: Reseed,
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
            "label": self.label.to_json(),
            "summary": self.summary.to_json(),
            "typical_duration_frames": self.typical_duration_frames,
            "needs_seed": self.reseed != Reseed::Never,
            "reseed": match self.reseed {
                Reseed::Never => "none",
                Reseed::Static => "static",
                Reseed::PerFrame => "per-frame",
            },
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
            "id": param.id, "label": param.label.to_json(), "kind": "number", "min": min,
            "max": max, "default": default, "step": step, "unit": unit,
        }),
        ParamKind::Color { default } => json!({
            "id": param.id, "label": param.label.to_json(), "kind": "color",
            "default": { "r": default[0], "g": default[1], "b": default[2] },
        }),
        ParamKind::Point { min, max, default } => json!({
            "id": param.id, "label": param.label.to_json(), "kind": "vec2", "min": min,
            "max": max, "default": { "x": default[0], "y": default[1] },
        }),
        ParamKind::Bool { default } => json!({
            "id": param.id, "label": param.label.to_json(), "kind": "boolean",
            "default": default,
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
    label: Labels::new("Vinheta", "Vignette", "Viñeta"),
    summary: Labels::new(
        "Escurece as bordas do quadro com uma queda elíptica e suave.",
        "Darkens the edges of the frame with a soft elliptical falloff.",
        "Oscurece los bordes del cuadro con una caída elíptica y suave.",
    ),
    typical_duration_frames: 96,
    reseed: Reseed::Never,
    params: &[
        ParamSpec {
            id: "radius",
            label: Labels::new("Raio", "Radius", "Radio"),
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
            label: Labels::new("Suavidade", "Softness", "Suavidad"),
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
            label: Labels::new("Intensidade", "Intensity", "Intensidad"),
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
            label: Labels::new("Cor", "Color", "Color"),
            kind: ParamKind::Color {
                default: [0.0, 0.0, 0.0],
            },
        },
        ParamSpec {
            id: "roundness",
            label: Labels::new("Arredondamento", "Roundness", "Redondez"),
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
            label: Labels::new("Centro", "Center", "Centro"),
            kind: ParamKind::Point {
                min: 0.0,
                max: 1.0,
                default: [0.5, 0.5],
            },
        },
    ],
};

/// Parameter order of the fade: `color`, `amount`, `intensity`, as in the pilot (`fade.ts` lists
/// `color` and `amount`; the pilot's `defineFxEffect` appends the `intensity` every effect shares,
/// default 1). Indices into `FADE.params`.
pub const FADE_COLOR: usize = 0;
pub const FADE_AMOUNT: usize = 1;
pub const FADE_INTENSITY: usize = 2;

/// `fade`: mixes the whole picture toward `color` by `amount × intensity × envelope`, in linear
/// light (the pilot's `fade` shader: `mix(source, color, amount × uAmount)`, `uAmount` being the
/// pass's `intensity × weight`). Exactly the identity when any of the three is 0. `amount` is
/// keyable (P3: two keys fade to black and back); the clip's `fadeInFrames`/`fadeOutFrames`
/// envelope is what opens from or closes to the colour.
pub static FADE: EffectSpec = EffectSpec {
    id: "fade",
    family: "look",
    label: Labels::new("Fade", "Fade", "Fundido"),
    summary: Labels::new(
        "Mistura a imagem com uma cor, com quantidade fixa e envelope de entrada e saída.",
        "Mixes the picture with a colour, with a fixed amount and an entry and exit envelope.",
        "Mezcla la imagen con un color, con cantidad fija y una envolvente de entrada y salida.",
    ),
    typical_duration_frames: 24,
    reseed: Reseed::Never,
    params: &[
        ParamSpec {
            id: "color",
            label: Labels::new("Cor", "Color", "Color"),
            kind: ParamKind::Color {
                default: [0.0, 0.0, 0.0],
            },
        },
        ParamSpec {
            id: "amount",
            label: Labels::new("Quantidade", "Amount", "Cantidad"),
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 1.0,
                step: 0.01,
                unit: "ratio",
            },
        },
        ParamSpec {
            id: "intensity",
            label: Labels::new("Intensidade", "Intensity", "Intensidad"),
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 1.0,
                step: 0.01,
                unit: "ratio",
            },
        },
    ],
};

/// Parameter order of the film grain: `amount`, `size`, `luma`, `colored`, then the shared
/// `intensity` the pilot's `defineFxEffect` appends. Indices into `FILM_GRAIN.params`.
pub const FILM_GRAIN_AMOUNT: usize = 0;
pub const FILM_GRAIN_SIZE: usize = 1;
pub const FILM_GRAIN_LUMA: usize = 2;
pub const FILM_GRAIN_COLORED: usize = 3;
pub const FILM_GRAIN_INTENSITY: usize = 4;

/// `filmGrain`: triangular noise from the integer hash of `(seed, cell)` added to the picture in
/// the **encoded (perceptual) sRGB space**, weighed more in the shadows by `luma`, then mixed back
/// in linear light by the pass's `amount × intensity × envelope` (the pilot's `filmGrain` shader).
/// The seed moves on every frame (`per-frame`) yet is a pure function of the frame, so a seek draws
/// what playback drew. `size` is in pixels **at 720p** and scales with the height of the picture.
pub static FILM_GRAIN: EffectSpec = EffectSpec {
    id: "filmGrain",
    family: "look",
    label: Labels::new("Grão de filme", "Film grain", "Grano de película"),
    summary: Labels::new(
        "Grão de película que ferve a cada quadro, mais forte nas sombras.",
        "Film grain that boils every frame, stronger in the shadows.",
        "Grano de película que hierve en cada cuadro, más fuerte en las sombras.",
    ),
    typical_duration_frames: 96,
    reseed: Reseed::PerFrame,
    params: &[
        ParamSpec {
            id: "amount",
            label: Labels::new("Quantidade", "Amount", "Cantidad"),
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.25,
                step: 0.01,
                unit: "ratio",
            },
        },
        ParamSpec {
            id: "size",
            label: Labels::new("Tamanho", "Size", "Tamaño"),
            kind: ParamKind::Number {
                min: 0.5,
                max: 4.0,
                default: 1.4,
                step: 0.1,
                unit: "px",
            },
        },
        ParamSpec {
            id: "luma",
            label: Labels::new(
                "Resposta de luminância",
                "Luminance response",
                "Respuesta de luminancia",
            ),
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.7,
                step: 0.01,
                unit: "ratio",
            },
        },
        ParamSpec {
            id: "colored",
            label: Labels::new("Colorido", "Colored", "Coloreado"),
            kind: ParamKind::Bool { default: false },
        },
        ParamSpec {
            id: "intensity",
            label: Labels::new("Intensidade", "Intensity", "Intensidad"),
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 1.0,
                step: 0.01,
                unit: "ratio",
            },
        },
    ],
};

static CATALOG: [&EffectSpec; 3] = [&VIGNETTE, &FADE, &FILM_GRAIN];

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
