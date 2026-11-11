//! Observable cost counters of the plan (F055 performance suite, BL015.12/BL015.18).
//!
//! Two thread-local counters, so a test that runs a scenario on its own thread reads exactly what
//! that scenario spent and a parallel test cannot add to it: how many times a scene was laid out
//! from scratch (`ScenePlan::build_with`, the cost that grows with the node count) and how many
//! plans a `PlanTimeline` produced from its shared layouts. They count work, never time, and
//! change no behaviour; a monotonic reading is subtracted from another one.
use std::cell::Cell;

thread_local! {
    static LAYOUTS: Cell<u64> = const { Cell::new(0) };
    static TIMELINE_PLANS: Cell<u64> = const { Cell::new(0) };
}

pub(super) fn note_layout() {
    LAYOUTS.with(|count| count.set(count.get() + 1));
}

pub(super) fn note_timeline_plan() {
    TIMELINE_PLANS.with(|count| count.set(count.get() + 1));
}

/// Full layouts of a scene (`ScenePlan::build` / `build_with`) made on this thread so far.
pub fn layouts_built() -> u64 {
    LAYOUTS.with(Cell::get)
}

/// Plans a `PlanTimeline` built on this thread so far.
pub fn timeline_plans_built() -> u64 {
    TIMELINE_PLANS.with(Cell::get)
}
