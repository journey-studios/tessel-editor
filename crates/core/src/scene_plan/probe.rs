//! Observable cost counter of the plan (F055 performance suite, BL015.12/BL015.18/BL015.21).
//!
//! It counts how many times a scene was laid out from scratch (`ScenePlan::build_with`, the cost
//! that grows with the node count), twice over:
//!
//! - per THREAD (`layouts_built`), so a test that runs a scenario on its own thread reads exactly
//!   what that scenario spent and a parallel test cannot add to it;
//! - per PROCESS (`layouts_built_in_process`), one relaxed atomic add. It exists so a suite can
//!   tell that no layout escaped the thread it was reading: a handler that moved its planning to
//!   another thread would read 0 per thread and still cost its full layout. Only a binary with a
//!   single test thread can compare the two (`scene_gizmo_perf_threads.rs`); in a parallel run
//!   the process-wide number also holds the other tests' layouts.
//!
//! They count work, never time, and change no behaviour; a monotonic reading is subtracted from
//! another one. The counter is production code on the hot path (one thread-local add plus one
//! relaxed atomic add per layout, with no allocation and no lock): its own cost is never assumed
//! zero, it is what `scripts/gizmo-perf/ab.py` measures by building the baseline WITHOUT the call
//! site. `PlanTimeline::build_with` is not counted: nothing reads such a counter, and an unread
//! counter would be dead code on a hot path (BL015.23 records the unprotected timeline path).
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

thread_local! {
    static LAYOUTS: Cell<u64> = const { Cell::new(0) };
}

static PROCESS_LAYOUTS: AtomicU64 = AtomicU64::new(0);

pub(super) fn note_layout() {
    LAYOUTS.with(|count| count.set(count.get() + 1));
    PROCESS_LAYOUTS.fetch_add(1, Ordering::Relaxed);
}

/// Full layouts of a scene (`ScenePlan::build` / `build_with`) made on this thread so far.
pub fn layouts_built() -> u64 {
    LAYOUTS.with(Cell::get)
}

/// Full layouts made by every thread of the process so far.
pub fn layouts_built_in_process() -> u64 {
    PROCESS_LAYOUTS.load(Ordering::Relaxed)
}
