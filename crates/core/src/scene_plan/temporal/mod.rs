//! The clocks of a transition window, declared by the plan and checked from its own data (F064,
//! P4b S2-C0b, BL082.4.20/.23).
//!
//! A plan with a `transition` carries, besides the two decks, an `alignment` and a `timing` block:
//! the edit length and, for each shot, where it sits in the edit and in its instance and camera
//! clip, with the two clocks the transition's rule produced for the frame. The rule is the core's
//! one (`directed_scene::DeckClocks`), also used by the neighbour deck's evaluation; here it is
//! used to MAKE the block ([`assemble`]) and to CHECK it ([`validate`]), never copied.
//!
//! This is the plan's internal COHERENCE, not the authenticity of its origin: a whole plan that was
//! edited into another coherent one declares another scene, and nothing here can tell.
//!
//! * [`model`]: the types and how a consumer reads them (`PlanTransition::clocks`).
//! * [`assemble`]: the block from the bundle's real shots, instances and camera clips.
//! * [`json`]: its wire form.
//! * [`validate`]: the relations a plan must keep, each with its pointer.

mod assemble;
mod json;
mod model;
mod validate;

pub use model::{DeckTiming, TransitionTiming};

pub(super) use json::{read_alignment, read_timing, timing_json};
