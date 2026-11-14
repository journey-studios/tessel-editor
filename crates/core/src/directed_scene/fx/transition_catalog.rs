//! The separate table of transition effects (F064, P4b S1a).
//!
//! A transition is an effect between two shots, with its own table and family (`transition`):
//! [`catalog`](super::catalog), [`effect`](super::effect) and [`catalog_json`](super::catalog_json)
//! stay the three looks, so the selector, `scene.fx.catalog` and the offers of the editor do not
//! change. One entry so far, `crossDissolve`; a bundle that names a look here, or a transition as a
//! clip, is refused as `WRONG_FAMILY`, not as an unknown effect.

use super::Reseed;
use super::catalog::{EffectSpec, Labels, ParamKind, ParamSpec};

/// Parameter order of `crossDissolve`, as the pilot lists them (`curve`, then `intensity`).
/// Indices into `CROSS_DISSOLVE.params` and into `FxTransition::params`.
pub const CROSS_DISSOLVE_CURVE: usize = 0;
pub const CROSS_DISSOLVE_INTENSITY: usize = 1;

/// The accepted spellings of `curve`, in the pilot's order (`smooth` is the default).
pub const CROSS_DISSOLVE_CURVES: &[&str] = &["linear", "smooth"];

/// `crossDissolve`: the outgoing shot dissolves into the incoming one over the transition window.
/// Defines only the contract (name, params, defaults): the pixels are the runtime's (S2).
pub static CROSS_DISSOLVE: EffectSpec = EffectSpec {
    id: "crossDissolve",
    family: "transition",
    label: Labels::new(
        "Dissolução cruzada",
        "Cross dissolve",
        "Disolvencia cruzada",
    ),
    summary: Labels::new(
        "A cena que sai se dissolve na que entra durante a janela da transição.",
        "The outgoing shot dissolves into the incoming one over the transition window.",
        "La escena que sale se disuelve en la que entra durante la ventana de la transición.",
    ),
    typical_duration_frames: 24,
    reseed: Reseed::Never,
    params: &[
        ParamSpec {
            id: "curve",
            label: Labels::new("Curva", "Curve", "Curva"),
            kind: ParamKind::Choice {
                options: CROSS_DISSOLVE_CURVES,
                default: "smooth",
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

static TRANSITIONS: [&EffectSpec; 1] = [&CROSS_DISSOLVE];

/// Every transition effect of this build, in table order. Not the public catalog.
pub fn transition_catalog() -> &'static [&'static EffectSpec] {
    &TRANSITIONS
}

/// The transition effect with this id (a look is `None`: see [`super::effect`]).
pub fn transition_effect(id: &str) -> Option<&'static EffectSpec> {
    TRANSITIONS.iter().copied().find(|spec| spec.id == id)
}
