//! T01-T10, T16, T17, T21 and T22 of the F065 matrix (P5 S1, BL082.5.2): the shot ops of
//! `scene_edit::shots`, through the public library API only (no GPUI, no window, no runtime).
//!
//! Every number is written by hand from the pilot's decisions (`scene-shot-trim-decisions.md`,
//! #1..#34); the oracle's corpus is read by `shot_trim_corpus.rs`, never here. The documents are
//! built with the corpus projector (`shot_trim_support`), so they are real render-bundle 1.0
//! scenes the importer accepts.
//!
//! * `support`: the builders and the readers of a result.
//! * `ops`: T01-T04 (one border at a time), T09 (`NO_HEAD_FRAMES`), T10 (the other refusals).
//! * `joins`: T05 (continuous joins), T06 (the temporal gate), T07 (`editLocked`), T08 (instances).
//! * `batch`: the batch, malformed ops, T16, T17 and the planner alone with a reserve.
//! * `property`: T21 (random sequences) and T22 (no shot path in the catalog).

mod shot_trim_support;

#[path = "scene_shots/batch.rs"]
mod batch;
#[path = "scene_shots/joins.rs"]
mod joins;
#[path = "scene_shots/ops.rs"]
mod ops;
#[path = "scene_shots/property.rs"]
mod property;
#[path = "scene_shots/support.rs"]
mod support;
