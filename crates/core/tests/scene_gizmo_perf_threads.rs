//! F055 / BL015.21: no layout escapes the thread the structural suite reads.
//!
//! `probe::layouts_built` is per thread, which is what makes `scene_gizmo_perf_structure.rs` exact in a parallel
//! `cargo test`. The price is a blind spot: a handler that moved its planning to another thread would read 0 on
//! the caller's thread and pass every upper bound while costing its full layout. The process-wide counter
//! (`probe::layouts_built_in_process`) closes it, but only a binary with ONE test can compare the two (a parallel
//! test would add its layouts to the process total), which is why this is a file of its own with a single `#[test]`.
//!
//! It proves two things: (1) the shared journey, on every corpus, spends exactly the same number of layouts per
//! request when read per thread and when read per process (nothing escaped); (2) the detector is alive: a journey
//! run on ANOTHER thread leaves the caller's thread counter untouched and moves the process one.
use core::prelude::v1::test;
use serde_json::Value;
use tessel_editor_core::scene_plan::probe;

#[path = "gizmo_perf_support/mod.rs"]
mod support;
use support::core_surface::CoreSurface;
use support::*;

/// The same core surface, but reading the process-wide layout counter.
struct ProcessWide(CoreSurface);

impl Surface for ProcessWide {
    fn name(&self) -> &'static str {
        "core"
    }

    fn call(&mut self, request: &Value) -> Reply {
        self.0.call(request)
    }

    fn layouts(&self) -> Option<u64> {
        Some(probe::layouts_built_in_process())
    }
}

fn layouts_of(run: &Run) -> Vec<(String, u64)> {
    run.samples
        .iter()
        .map(|sample| (sample.step.clone(), sample.layouts.expect("counted")))
        .collect()
}

#[test]
fn no_layout_escapes_the_thread_the_suite_reads_and_the_detector_sees_one_that_does() {
    let journey = Journey::load();
    for name in CORPORA {
        let corpus = journey.corpus(name);
        let per_thread = run(&mut CoreSurface::new(), &journey, &corpus);
        let per_process = run(&mut ProcessWide(CoreSurface::new()), &journey, &corpus);
        assert_eq!(
            layouts_of(&per_thread),
            layouts_of(&per_process),
            "{name}: a request laid the scene out on a thread the structural suite does not read"
        );
        assert!(
            layouts_of(&per_thread).iter().any(|(_, count)| *count > 0),
            "{name}: the journey counted no layout at all: the comparison would be empty"
        );
    }

    let thread_before = probe::layouts_built();
    let process_before = probe::layouts_built_in_process();
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let corpus = journey.corpus("small");
                run(&mut CoreSurface::new(), &journey, &corpus);
            })
            .join()
            .expect("the journey ran on the other thread");
    });
    assert_eq!(
        probe::layouts_built(),
        thread_before,
        "the layouts of another thread must not show up on this one"
    );
    assert!(
        probe::layouts_built_in_process() > process_before,
        "the process-wide counter must see the layouts another thread spent"
    );
}
