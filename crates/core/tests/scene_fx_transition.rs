//! F064 / P4b S1a (BL082.4.2): the domain foundation of `fx.transitions` and its admission gate.
//!
//! * `document`: the document model through the hidden admitting importer (T01, T02, T03 doc,
//!   T05, T28 doc).
//! * `corpus`: the committed oracle `dissolve-cases.json`, read by Rust.
//! * `public`: every public door still refuses transitions (library, executor, a real
//!   `tessel-headless` process), and edit candidates keep the policy of their scene.
//! * `boundary`: no production code opens the gate.

#[path = "scene_fx_transition/boundary.rs"]
mod boundary;
#[path = "scene_fx_transition/corpus.rs"]
mod corpus;
#[path = "scene_fx_transition/document.rs"]
mod document;
#[path = "scene_fx_transition/public.rs"]
mod public;
#[path = "scene_fx_transition/support.rs"]
mod support;
