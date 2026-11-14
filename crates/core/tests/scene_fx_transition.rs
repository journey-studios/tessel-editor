//! F064 / P4b S1a (BL082.4.2): the domain foundation of `fx.transitions` and its admission gate.
//!
//! * `document`: the document model through the hidden admitting importer (T01, T02, T03 doc,
//!   T05, T28 doc).
//! * `corpus`: the committed oracle `dissolve-cases.json`, read by Rust.
//! * `parameter_order`: transition diagnostics have one order whatever `serde_json` features the
//!   build unifies (run it with `--features serde_json/preserve_order` too).
//! * `public`: every public door still refuses transitions (library, executor, a real
//!   `tessel-headless` process), and edit candidates keep the policy of their scene.
//! * `reload`: the doors a *file* reaches (project open, reload, attach by path) refuse
//!   transitions dynamically, with the state, revision and history untouched.
//! * `boundary` and `scan`: no production code opens the gate (an architectural text scan, not a
//!   security barrier); `scan_checks` proves the scanner against hostile text.
//!
//! P4b S1b (BL082.4.3), internal only (the public gate stays closed):
//!
//! * `clocks`: the neighbour deck's clocks (T06, T10).
//! * `stages`: one frame in stages, progress, amount, border clips, seed, budget (T03, T07, T11).
//! * `plan`: `ScenePlan.transition`, resources of two decks, requirement, corruption (T08, T09, T28).
//! * `limits`: bytes, items and depth ceilings of a plan with a transition.
//! * `compat`: the Rust capability list against the runtime's.

#[path = "scene_fx_transition/boundary.rs"]
mod boundary;
#[path = "scene_fx_transition/clocks.rs"]
mod clocks;
#[path = "scene_fx_transition/compat.rs"]
mod compat;
#[path = "scene_fx_transition/corpus.rs"]
mod corpus;
#[path = "scene_fx_transition/document.rs"]
mod document;
#[path = "scene_fx_transition/limits.rs"]
mod limits;
#[path = "scene_fx_transition/parameter_order.rs"]
mod parameter_order;
#[path = "scene_fx_transition/plan.rs"]
mod plan;
#[path = "scene_fx_transition/public.rs"]
mod public;
#[path = "scene_fx_transition/reload.rs"]
mod reload;
#[path = "scene_fx_transition/scan.rs"]
mod scan;
#[path = "scene_fx_transition/scan_checks.rs"]
mod scan_checks;
#[path = "scene_fx_transition/stages.rs"]
mod stages;
#[path = "scene_fx_transition/support.rs"]
mod support;
