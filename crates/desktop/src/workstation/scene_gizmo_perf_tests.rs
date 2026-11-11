//! F055 / BL015.21: the GPUI host half of the gizmo performance suite, deterministic (nothing here
//! reads a clock to pass or fail).
//!
//! It runs the SAME journey the core runs (`crates/core/tests/gizmo_perf_support`, included by
//! path: the same `journey.json`, the same driver, the same assertions on the stored transform,
//! the history and the commit/cancel/undo/redo equalities) through `Workstation::run_op`, the
//! handler the window's socket and the CLI share, in a GPUI test window. What that adds over the
//! core run: the host's own work around every command (the monitor's plan, the overlay memo, the
//! adoption of the overlay a command answered with) is counted per request, and the window steps
//! of the journey that a test window CAN run (`monitor.show_scene`, a resize) run.
//!
//! What a test window is not, so what this does NOT prove: it has no WebView (the page that draws
//! the overlay and presents the picture is absent; the host is told the page announced
//! `overlay.gizmo`, as `scene_gizmo_tests.rs` does), no GPU, no compositor and no physical pointer.
//! "First frame presented" is the native proof's (a real window, part of the timing stack, BL015.23).
use core::prelude::v1::test;

use serde_json::{Value, json};

use super::input_tests::fixture;
use super::ops::OpReply;
use super::*;
use tessel_editor_core::scene_plan::probe;

#[path = "../../../core/tests/gizmo_perf_support/mod.rs"]
mod support;
use support::{CORPORA, Journey, Reply, Run, Surface, json_file, transforms_equal};

struct HostSurface<'a> {
    view: Entity<Workstation>,
    cx: &'a mut VisualTestContext,
}

impl HostSurface<'_> {
    /// Ticks the window the way its timer does until the job (the resolve) has been applied.
    fn settle_jobs(&mut self) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            self.view.update(self.cx, |view, cx| view.workflow_tick(cx));
            self.cx.run_until_parked();
            if self
                .view
                .read_with(self.cx, |view, _| view.workflow.jobs_line().is_none())
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the resolve job never ended"
            );
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        self.view.update(self.cx, |view, cx| view.workflow_tick(cx));
        self.cx.run_until_parked();
    }

    /// A request that must succeed; its answer.
    fn ok(&mut self, request: Value) -> Value {
        let reply = self.call(&request);
        assert!(reply.ok, "{request}: {}", reply.value);
        reply.value
    }

    /// The scene the monitor holds as its directed scene: what the gizmo routes the overlay and the pointer by.
    fn directed_scene(&mut self) -> Option<String> {
        self.view.read_with(self.cx, |view, _| {
            view.viewport.directed_scene().map(str::to_owned)
        })
    }
}

impl Surface for HostSurface<'_> {
    fn name(&self) -> &'static str {
        "host"
    }

    fn call(&mut self, request: &Value) -> Reply {
        let answer = self
            .view
            .update(self.cx, |view, cx| view.run_op(request, cx));
        self.cx.run_until_parked();
        if request["op"] == "scene.resources.resolve" && answer.is_ok() {
            self.settle_jobs();
        }
        match answer {
            Ok(OpReply::Done(value)) => Reply { ok: true, value },
            Ok(OpReply::Pending(_)) => Reply {
                ok: false,
                value: json!({"code": "pending", "message": "the journey asks for no deferred operation"}),
            },
            Err(error) => Reply {
                ok: false,
                value: json!({"code": error.code, "message": error.message, "details": error.details}),
            },
        }
    }

    fn layouts(&self) -> Option<u64> {
        Some(probe::layouts_built())
    }

    /// A test window runs the monitor's host, not the page: the steps that only need the host.
    fn presents(&self) -> bool {
        true
    }

    fn ui(&mut self, words: &[String]) -> Option<Reply> {
        let done = Reply {
            ok: true,
            value: json!({}),
        };
        match words
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .as_slice()
        {
            ["set", "window", "size", size_text] => {
                let (w, h) = size_text.split_once('x')?;
                self.cx
                    .simulate_resize(size(px(w.parse().ok()?), px(h.parse().ok()?)));
                self.cx.run_until_parked();
                Some(done)
            }
            // A dialog needs the Kit's `Root`, which a test window has not (the same limit as
            // `workflow_tests.rs`): the dialog step is the native window's, skipped here with its reason.
            _ => None,
        }
    }
}

fn host_run(cx: &mut TestAppContext, corpus: &str) -> Run {
    let (view, cx) = fixture(cx);
    view.update(cx, |view, _| view.viewport.pretend_overlay_supported(true));
    let journey = Journey::load();
    let mut surface = HostSurface { view, cx };
    support::run(&mut surface, &journey, &journey.corpus(corpus))
}

/// A window that sits still asks the core for nothing. Two separate facts, because a sync is not a tick:
///   * the timer's tick (`workflow_tick`) with nothing to apply lays no scene out at all;
///   * a viewport sync of an unchanged state re-renders the picture the monitor shows (exactly one layout of
///     the scene, the monitor's own), but finds the overlay memoised (`gizmo_overlay_key`): it asks the core for no
///     second plan and for no overlay (`computed` stays, `memo_hits` grows).
/// This is the "replan per tick" and "memo off" detector (the time suite cannot see a 250 ms timer that is cheap
/// per call but never idle).
#[gpui_kit::test]
fn an_idle_window_plans_nothing_per_tick_and_the_overlay_stays_memoised(cx: &mut TestAppContext) {
    const TICKS: u64 = 100;
    const SYNCS: u64 = 20;
    for corpus in CORPORA {
        let (view, cx) = fixture(cx);
        view.update(cx, |view, _| view.viewport.pretend_overlay_supported(true));
        let journey = Journey::load();
        {
            let mut surface = HostSurface {
                view: view.clone(),
                cx,
            };
            let run = support::run(&mut surface, &journey, &journey.corpus(corpus));
            assert!(
                run.samples.iter().all(|sample| sample.ok),
                "{corpus}: the journey ran"
            );
        }
        let report = |cx: &mut VisualTestContext| {
            view.read_with(cx, |view, _| view.viewport.overlay_report())
        };
        // The state the journey left, synced once (it may be the first time the host sees it) ...
        view.update(cx, |view, cx| view.viewport_sync(cx));
        cx.run_until_parked();
        let start = (probe::layouts_built(), report(cx));
        assert_eq!(
            start.1["enabled"], true,
            "{corpus}: the overlay is on, so the memo is what is tested"
        );
        // ... a still window: the timer ticks.
        for _ in 0..TICKS {
            view.update(cx, |view, cx| view.workflow_tick(cx));
        }
        cx.run_until_parked();
        assert_eq!(
            probe::layouts_built(),
            start.0,
            "{corpus}: {TICKS} idle ticks laid the scene out"
        );
        // ... and the same state synced again and again.
        for _ in 0..SYNCS {
            view.update(cx, |view, cx| view.viewport_sync(cx));
        }
        cx.run_until_parked();
        let end = (probe::layouts_built(), report(cx));
        assert!(
            end.0 - start.0 <= SYNCS,
            "{corpus}: a sync of an unchanged state laid the scene out more than once ({} for {SYNCS})",
            end.0 - start.0
        );
        assert_eq!(
            end.1["computed"], start.1["computed"],
            "{corpus}: the overlay was asked for again"
        );
        assert!(
            end.1["memo_hits"].as_u64().unwrap() >= start.1["memo_hits"].as_u64().unwrap() + SYNCS,
            "{corpus}: every sync of an unchanged state is a memo hit ({} -> {})",
            start.1["memo_hits"],
            end.1["memo_hits"]
        );
    }
}

/// BL015.22: two attached scenes of ONE bundle file share a `bundleId`, a plan, a resource map and options.
/// The journey's `same.*` steps attach `proof-c` from the file `proof-b` came from and show it; the host must
/// then report `proof-c` as the directed scene. This was a product defect, not a hypothesis: the monitor's
/// dedupe key had no scene id, so `monitor.show_scene proof-c` answered ok while the directed scene stayed
/// `proof-b` (fixed in `ViewportHost::render_scene`: the scene id is part of the key now). This is the
/// journey-level check; `monitor_show_scene_follows_the_request_between_scenes_of_one_bundle_and_of_another`
/// below is the step-by-step regression test of the sequence that reproduced it.
#[gpui_kit::test]
fn showing_a_scene_that_shares_its_bundle_with_the_shown_one_shows_it(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    view.update(cx, |view, _| view.viewport.pretend_overlay_supported(true));
    let journey = Journey::load();
    {
        let mut surface = HostSurface {
            view: view.clone(),
            cx,
        };
        let run = support::run(&mut surface, &journey, &journey.corpus("small"));
        assert!(
            run.steps
                .iter()
                .any(|step| step.id == "same.show" && step.skipped.is_none()),
            "the same-bundle steps must run on the host"
        );
    }
    let shown = view.read_with(cx, |view, _| {
        view.viewport.directed_scene().map(str::to_owned)
    });
    assert_eq!(
        shown.as_deref(),
        Some("proof-c"),
        "`monitor.show_scene` answered ok for a scene of an already shown bundle but the monitor kept another scene"
    );
}

/// BL015.22, the sequence that reproduced it (steps E1-E6 below), now a regression test through
/// `Workstation::run_op`: `monitor.show_scene` leaves the monitor on the scene it was asked for,
/// whichever scenes were shown before and whether or not they come from one bundle file.
/// `proof-b` and `proof-c` are INTACT scenes attached from the same file (same plan, resource map and options at the
/// same playhead); `proof` comes from another bundle. The old rule matched by content, so it failed in both directions
/// (E2: `proof-c` after `proof-b`; E5: `proof-b` after `proof-c`) and worked only after a scene of another bundle (E4);
/// a repeat of one scene (E6) must keep the scene.
#[gpui_kit::test]
fn monitor_show_scene_follows_the_request_between_scenes_of_one_bundle_and_of_another(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    view.update(cx, |view, _| view.viewport.pretend_overlay_supported(true));
    let journey = Journey::load();
    let corpus = journey.corpus("small");
    let root = support::repo_root();
    let abs = |relative: &str| {
        root.join(relative)
            .canonicalize()
            .unwrap()
            .display()
            .to_string()
    };
    let (shared, other) = (abs(&corpus.bundle_b), abs(&corpus.bundle));
    let base = abs(journey.value["base"].as_str().unwrap());
    let mut surface = HostSurface { view, cx };

    fn attach_and_show(
        surface: &mut HostSurface,
        scene: &str,
        bundle: &str,
        base: &str,
    ) -> Option<String> {
        surface.ok(json!({"op": "scene.attach", "id": scene, "path": bundle}));
        surface.ok(json!({"op": "scene.resources.resolve", "scene": scene, "base": base}));
        surface.ok(json!({"op": "monitor.show_scene", "scene": scene}));
        surface.directed_scene()
    }
    fn show(surface: &mut HostSurface, scene: &str) -> Option<String> {
        surface.ok(json!({"op": "monitor.show_scene", "scene": scene}));
        surface.directed_scene()
    }

    surface.ok(json!({"op": "project.create", "name": "bl01522"}));
    let e1 = attach_and_show(&mut surface, "proof-b", &shared, &base);
    assert_eq!(e1.as_deref(), Some("proof-b"), "E1: the first scene");
    let e2 = attach_and_show(&mut surface, "proof-c", &shared, &base);
    assert_eq!(
        e2.as_deref(),
        Some("proof-c"),
        "E2: a second scene of the same file; the monitor kept the first"
    );
    let e3 = attach_and_show(&mut surface, "proof", &other, &base);
    assert_eq!(
        e3.as_deref(),
        Some("proof"),
        "E3: a scene of another bundle"
    );
    let e4 = show(&mut surface, "proof-c");
    assert_eq!(
        e4.as_deref(),
        Some("proof-c"),
        "E4: back to the second scene"
    );
    let e5 = show(&mut surface, "proof-b");
    assert_eq!(
        e5.as_deref(),
        Some("proof-b"),
        "E5: back to the first one; the rule is symmetric"
    );
    let e6 = show(&mut surface, "proof-b");
    assert_eq!(e6.as_deref(), Some("proof-b"), "E6: a repeat of one scene");
}

#[gpui_kit::test]
fn the_host_runs_the_shared_journey_to_the_same_transforms_and_history_as_the_core(
    cx: &mut TestAppContext,
) {
    let expected = json_file(support::EXPECTED);
    for corpus in CORPORA {
        let run = host_run(cx, corpus);
        let got = run.trace();
        let want = expected[corpus].as_object().unwrap();
        for (id, step) in want {
            assert!(
                got.get(id).is_some_and(|host| transforms_equal(host, step)),
                "{corpus} / {id}: the host left a different transform or history than the core's golden trace\nhost {}\ncore {step}",
                got.get(id).unwrap_or(&Value::Null)
            );
        }
        // The host also ran the steps a test window can present (`monitor.show_scene`, the resize); a
        // picture the page presented and a dialog (`Root`) are skipped, with their reason (never a pass).
        let skipped: Vec<&str> = run
            .steps
            .iter()
            .filter(|step| step.skipped.is_some())
            .map(|step| step.id.as_str())
            .collect();
        assert_eq!(
            skipped,
            [
                "open.first_frame",
                "present.dialog.open",
                "present.dialog.close"
            ],
            "{corpus}: only what needs the page (a presented picture) or the Kit root (a dialog) is skipped on the host"
        );
        assert!(
            run.steps
                .iter()
                .any(|step| step.id == "open.show" && step.skipped.is_none())
        );
        assert!(
            run.steps
                .iter()
                .any(|step| step.id == "present.resize.large" && step.skipped.is_none())
        );
    }
}

#[gpui_kit::test]
fn every_host_request_plans_the_scene_at_most_as_often_as_its_contract_says(
    cx: &mut TestAppContext,
) {
    for corpus in CORPORA {
        let run = host_run(cx, corpus);
        for sample in &run.samples {
            let layouts = sample.layouts.unwrap();
            assert!(sample.ok, "{corpus} / {}: {:?}", sample.step, sample.code);
            assert!(
                layouts <= host_budget(&sample.label),
                "{corpus} / {}: the host laid the scene out {layouts} times for `{}` (budget {})",
                sample.step,
                sample.label,
                host_budget(&sample.label)
            );
        }
    }
}

/// The most layouts the host spends per request, as measured on the tree this suite was written
/// against (every corpus, every step; a smaller number never fails). It is the core's contract plus
/// the host's own: the monitor plans the scene it shows for its next picture, so a command that
/// changes the shown scene costs one more layout than in the core, and the pointer press/release
/// one more again. An extra plan per request or per tick raises a count by one and fails here.
fn host_budget(label: &str) -> u64 {
    match label {
        "project.create" => 0,
        "scene.pointer.down" | "scene.pointer.up" | "monitor.show_scene" => 3,
        "scene.gizmo.state"
        | "scene.pointer.move"
        | "scene.pointer.cancel"
        | "scene.gizmo.tool"
        | "history.undo"
        | "history.redo"
        | "scene.select"
        | "scene.attach"
        | "scene.resources.resolve"
        | "seek" => 2,
        other => panic!("the journey runs `{other}` on the host: give it a budget"),
    }
}
