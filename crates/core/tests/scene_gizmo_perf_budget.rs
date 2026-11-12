//! F055 / BL015.21: the TIME half of the gizmo performance suite. Opt-in and controlled: every test
//! that measures is `#[ignore]` on purpose (a wall-clock budget must not make the functional
//! `cargo test` depend on the load of the machine; the four tests at the end only judge reports
//! written by hand, with no clock, and run in the plain `cargo test`) and an ignored test is NOT a
//! green result of this suite:
//! only `scripts/gizmo-perf/run.py` counts, because it takes the performance lock without waiting,
//! builds in release, runs these tests with `--ignored`, checks that every expected test RAN and
//! passed, and writes the report with all the samples.
//!
//! ```text
//! python3 scripts/gizmo-perf/run.py            # lock, release, the tests, the report
//! ```
//!
//! What runs, through the public handlers only (`headless::execute`, the JSON executor the CLI
//! and the window share), on the three synthetic scenes of the shared corpus:
//!
//! - **cold**: a fresh editor, `scene.attach` and the first `scene.gizmo.state` of that scene,
//!   repeated `COLD_RUNS` times; every sample kept, the first included (process-level warmth of the
//!   first run is declared in the report, not hidden by dropping it);
//! - **hot**: after `WARMUP_CYCLES` discarded-from-the-statistics (but kept in the report) cycles,
//!   `HOT_CYCLES` interaction cycles per tool: state, press, `MOVES` moves, release, undo; plus
//!   state reads of a visible overlay per tool, a hidden one (nothing selected) and a refused one.
//!
//! The time is judged against the PLAN, not against a clock constant: the scene's own
//! `ScenePlan::build_with` (what every handler has to pay at least once) is timed in the same
//! process, in a small block right before each WINDOW of requests (one interaction cycle, or
//! `READ_WINDOW` reads), and every request is divided by the reference of its own window. A ratio
//! cancels the speed of the machine at that moment (a thread moved between a fast and a slow core,
//! the load of a neighbour), which a constant or a single reference taken once cannot. Each
//! handler's p50 and p95 ratios are limited by `budgets.json`, which was NOT fixed before the
//! candidate was measured: it comes from calibration runs of this same tree (see
//! `scripts/gizmo-perf/budgets.py`), and its cold margins were widened after a judged run failed
//! (see `docs/features/scene-gizmo-performance-timing.md`), so a pass is no proof that
//! nothing regressed; a handler that does a second plan's worth of work (the mutants of
//! `scripts/gizmo-perf/mutate.py`) leaves the budget on a dense scene. A long-tick count (a sample
//! over a 60 Hz frame) is kept as well. A run whose reference blocks are themselves unstable (a
//! block whose own samples disagree by more than `reference.max_block_spread`, in more than
//! `reference.max_unstable_blocks` of the blocks) is INCONCLUSIVE, not a pass.
//!
//! What it does not measure: CPU time (no portable counter without `unsafe`), allocations (the
//! workspace forbids a counting allocator), the WebView, the GPU or the compositor: those are the
//! window half (`scripts/gizmo-perf/native.py`).

use core::prelude::v1::test;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

use serde_json::{Value, json};
use tessel_editor_core::scene_plan::{PlanOptions, ScenePlan};

#[path = "gizmo_perf_support/mod.rs"]
mod support;
use support::core_surface::CoreSurface;
use support::*;

const BUDGETS: &str = "docs/migration/fixtures/gizmo-perf/budgets.json";
const IGNORED: &str = "performance: run through scripts/gizmo-perf/run.py (lock, release)";
const COLD_RUNS: usize = 60;
const WARMUP_CYCLES: usize = 8;
const HOT_CYCLES: usize = 40;
const READS: usize = 200;
const MOVES: usize = 6;
const REFERENCE_BLOCK: usize = 5;
const READ_WINDOW: usize = 20;

fn release_build() -> bool {
    !cfg!(debug_assertions)
}

fn calibrating() -> bool {
    std::env::var("TESSEL_GIZMO_PERF_CALIBRATE").is_ok_and(|value| value == "1")
}

/// NEGATIVE CONTROLS ONLY (the two `the_budget_detector_*` tests): work added to every timed
/// handler call from the outside, to prove the detector can fail. It is never part of the real
/// measurement and proves nothing about the product's flow.
static INJECT: AtomicU8 = AtomicU8::new(0);
const INJECT_PLAN: u8 = 1;
const INJECT_DELAY: u8 = 2;

fn injected(surface: &CoreSurface, scene: &str, frame: i64) {
    match INJECT.load(Ordering::Relaxed) {
        INJECT_PLAN => {
            let entry = surface.state.scenes.get(scene).unwrap();
            let plan = ScenePlan::build_with(
                &entry.scene,
                frame,
                PlanOptions {
                    allow_partial: true,
                },
            );
            assert!(plan.is_ok());
        }
        INJECT_DELAY => std::thread::sleep(std::time::Duration::from_millis(1)),
        _ => {}
    }
}

fn ok(surface: &mut CoreSurface, request: Value) -> Value {
    let reply = surface.call(&request);
    assert!(reply.ok, "{request}: {}", reply.value);
    reply.value
}

fn timed(surface: &mut CoreSurface, request: Value) -> (u128, Value) {
    let started = Instant::now();
    let reply = surface.call(&request);
    if INJECT.load(Ordering::Relaxed) != 0 && request["op"] != "scene.attach" {
        injected(surface, "bench", 24);
    }
    let ns = started.elapsed().as_nanos();
    assert!(reply.ok, "{request}: {}", reply.value);
    (ns, reply.value)
}

fn attach(surface: &mut CoreSurface, scene: &str, path: &str) -> u128 {
    timed(
        surface,
        json!({"op": "scene.attach", "id": scene, "path": path}),
    )
    .0
}

struct Bench {
    scene: String,
    frame: i64,
    node: String,
    other: String,
}

impl Bench {
    fn state(&self) -> Value {
        json!({"op": "scene.gizmo.state", "scene": self.scene, "frame": self.frame})
    }
    fn select(&self, entity: Option<&str>) -> Value {
        json!({"op": "scene.select", "scene": self.scene, "entities": entity.map(|e| vec![e]).unwrap_or_default()})
    }
}

fn plan_samples(surface: &CoreSurface, scene: &str, frame: i64, n: usize) -> Vec<u128> {
    let entry = surface.state.scenes.get(scene).unwrap();
    (0..n)
        .map(|_| {
            let started = Instant::now();
            let plan = ScenePlan::build_with(
                &entry.scene,
                frame,
                PlanOptions {
                    allow_partial: true,
                },
            );
            let ns = started.elapsed().as_nanos();
            assert!(plan.is_ok());
            ns
        })
        .collect()
}

/// One interaction cycle of a tool: state, press, moves, release, undo. Every request is a sample
/// of its own kind, `tool`-tagged. Returns `(kind, ns)` pairs in order.
fn cycle(
    surface: &mut CoreSurface,
    bench: &Bench,
    tool: &str,
    space: &str,
    handle: &str,
    index: usize,
) -> Vec<(String, u128)> {
    let mut out = Vec::new();
    let (ns, shown) = timed(surface, bench.state());
    out.push((format!("state.{tool}"), ns));
    let entry = shown["overlay"]["handles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["id"] == handle)
        .unwrap_or_else(|| panic!("no `{handle}` in {}", shown["overlay"]))
        .clone();
    let ring: Vec<Value> = entry["points"]
        .as_array()
        .map(|points| {
            points[points.len() / 8..]
                .iter()
                .filter(|p| !p.is_null())
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let start = if ring.is_empty() {
        entry["to"].clone()
    } else {
        ring[0].clone()
    };
    let (ns, down) = timed(
        surface,
        json!({"op": "scene.pointer.down", "scene": bench.scene, "frame": bench.frame, "point": start}),
    );
    assert_eq!(down["dragging"], true, "{down}");
    out.push((format!("down.{tool}"), ns));
    for step in 1..=MOVES {
        let point = if ring.is_empty() {
            // Alternate sides of the press so the candidate stays legal over many cycles.
            let sign = if (index + step).is_multiple_of(2) {
                1.0
            } else {
                -1.0
            };
            json!({"u": start["u"].as_f64().unwrap() + sign * 0.004 * step as f64, "v": start["v"].as_f64().unwrap() + 0.002 * step as f64})
        } else {
            ring[step.min(ring.len() - 1)].clone()
        };
        let (ns, moved) = timed(surface, json!({"op": "scene.pointer.move", "point": point}));
        assert_eq!(moved["applied"], true, "{moved}");
        out.push((format!("move.{tool}"), ns));
    }
    let (ns, up) = timed(surface, json!({"op": "scene.pointer.up"}));
    assert_eq!(up["document_changed"], true, "{up}");
    out.push((format!("up.{tool}"), ns));
    let (ns, _) = timed(surface, json!({"op": "history.undo"}));
    out.push(("undo".to_string(), ns));
    let _ = space;
    out
}

type Pairs = std::collections::BTreeMap<String, Vec<(u128, u128)>>;

fn median_of(values: &[u128]) -> u128 {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    percentile(&sorted, 0.5)
}

/// Every reference block taken, with its own spread, so the run can say how steady it was.
#[derive(Default)]
struct References {
    blocks: Vec<Vec<u128>>,
}

impl References {
    /// A block of plan builds right before a window; its median is the reference of the window.
    fn block(&mut self, surface: &CoreSurface, bench: &Bench) -> u128 {
        let samples = plan_samples(surface, &bench.scene, bench.frame, REFERENCE_BLOCK);
        let reference = median_of(&samples);
        self.blocks.push(samples);
        reference
    }
}

fn push(map: &mut Pairs, kind: &str, ns: u128, reference: u128) {
    map.entry(kind.to_string())
        .or_default()
        .push((ns, reference));
}

fn measure_corpus(journey: &Journey, name: &str) -> Value {
    let corpus = journey.corpus(name);
    let path = repo_root()
        .join(&corpus.bundle)
        .canonicalize()
        .unwrap()
        .display()
        .to_string();
    let bench = Bench {
        scene: "bench".into(),
        frame: journey.frame(),
        node: corpus.node.clone(),
        other: corpus.other.clone(),
    };
    let mut references = References::default();

    // Cold: every sample of every fresh editor, the first run included. The reference is taken on
    // the freshly attached scene, in the same editor.
    let mut cold: Pairs = Default::default();
    for _ in 0..COLD_RUNS {
        let mut surface = CoreSurface::new();
        ok(
            &mut surface,
            json!({"op": "project.create", "name": "cold"}),
        );
        let attach_ns = attach(&mut surface, &bench.scene, &path);
        ok(&mut surface, bench.select(Some(&bench.node)));
        let reference = references.block(&surface, &bench);
        push(&mut cold, "cold.attach", attach_ns, reference);
        // The block above laid the scene out once already; a cold state read must be the FIRST
        // plan of its editor, so build the next editor for it.
        let mut fresh = CoreSurface::new();
        ok(&mut fresh, json!({"op": "project.create", "name": "cold"}));
        attach(&mut fresh, &bench.scene, &path);
        ok(&mut fresh, bench.select(Some(&bench.node)));
        let (ns, _) = timed(&mut fresh, bench.state());
        push(&mut cold, "cold.state", ns, reference);
    }

    // Hot.
    let mut surface = CoreSurface::new();
    ok(&mut surface, json!({"op": "project.create", "name": "hot"}));
    attach(&mut surface, &bench.scene, &path);
    let mut warmup: Pairs = Default::default();
    let mut hot: Pairs = Default::default();
    let tools = [
        ("translate", "world", "x"),
        ("rotate", "local", "y"),
        ("scale", "local", "uniform"),
    ];
    ok(&mut surface, bench.select(Some(&bench.node)));
    for (tool, space, handle) in tools {
        ok(
            &mut surface,
            json!({"op": "scene.gizmo.tool", "tool": tool, "space": space}),
        );
        for index in 0..WARMUP_CYCLES + HOT_CYCLES {
            let reference = references.block(&surface, &bench);
            let target = if index < WARMUP_CYCLES {
                &mut warmup
            } else {
                &mut hot
            };
            for (kind, ns) in cycle(&mut surface, &bench, tool, space, handle, index) {
                push(target, &kind, ns, reference);
            }
        }
        // Reads of the overlay (what the host asks for after every command).
        let mut reference = 0;
        for index in 0..WARMUP_CYCLES + READS {
            if index % READ_WINDOW == 0 {
                reference = references.block(&surface, &bench);
            }
            let (ns, _) = timed(&mut surface, bench.state());
            let target = if index < WARMUP_CYCLES {
                &mut warmup
            } else {
                &mut hot
            };
            push(target, &format!("read.{tool}"), ns, reference);
        }
    }
    ok(&mut surface, bench.select(None));
    let mut reference = 0;
    for index in 0..WARMUP_CYCLES + READS {
        if index % READ_WINDOW == 0 {
            reference = references.block(&surface, &bench);
        }
        let (ns, shown) = timed(&mut surface, bench.state());
        assert_eq!(shown["overlay"]["visible"], false);
        let target = if index < WARMUP_CYCLES {
            &mut warmup
        } else {
            &mut hot
        };
        push(target, "read.hidden", ns, reference);
    }
    ok(&mut surface, bench.select(Some(&bench.other)));
    ok(
        &mut surface,
        json!({"op": "scene.gizmo.tool", "tool": "rotate", "space": "local"}),
    );
    let mut reference = 0;
    for index in 0..WARMUP_CYCLES + READS {
        if index % READ_WINDOW == 0 {
            reference = references.block(&surface, &bench);
        }
        let (ns, shown) = timed(&mut surface, bench.state());
        assert_eq!(
            shown["overlay"]["visible"], false,
            "the refusal is part of the overlay"
        );
        let target = if index < WARMUP_CYCLES {
            &mut warmup
        } else {
            &mut hot
        };
        push(target, "read.refused", ns, reference);
    }

    let all_refs: Vec<u128> = references.blocks.iter().flatten().copied().collect();
    let reference_p50 = median_of(&all_refs);
    let spreads: Vec<f64> = references
        .blocks
        .iter()
        .map(|block| {
            *block.iter().max().unwrap() as f64 / (*block.iter().min().unwrap()).max(1) as f64
        })
        .collect();
    let unstable = spreads
        .iter()
        .filter(|spread| **spread > MAX_BLOCK_SPREAD)
        .count();

    let report_of = |samples: &Pairs| -> Value {
        Value::Object(
            samples
                .iter()
                .map(|(kind, pairs)| {
                    let times: Vec<u128> = pairs.iter().map(|(ns, _)| *ns).collect();
                    let mut ratios: Vec<f64> = pairs
                        .iter()
                        .map(|(ns, r)| *ns as f64 / (*r).max(1) as f64)
                        .collect();
                    ratios.sort_by(f64::total_cmp);
                    let at = |q: f64| {
                        ratios
                            [((q * ratios.len() as f64).ceil() as usize).clamp(1, ratios.len()) - 1]
                    };
                    let mut summary = summary(&times);
                    summary["ratio_p50"] = json!(at(0.5));
                    summary["ratio_p95"] = json!(at(0.95));
                    summary["ratio_p99"] = json!(at(0.99));
                    summary["ratio_max"] = json!(ratios[ratios.len() - 1]);
                    summary["long_ticks"] =
                        json!(times.iter().filter(|ns| **ns > 16_600_000).count());
                    summary["samples_ns"] = json!(times);
                    summary["reference_ns"] =
                        json!(pairs.iter().map(|(_, r)| *r).collect::<Vec<_>>());
                    (kind.clone(), summary)
                })
                .collect(),
        )
    };
    json!({
        "corpus": name,
        "nodes": surface.state.scenes.get(&bench.scene).unwrap().scene.document()["sceneDefinitions"][0]["nodes"].as_array().map(Vec::len),
        "reference": {
            "what": "ScenePlan::build_with(allow_partial) of the attached scene at the journey frame, median of a block right before each window",
            "p50_ns": reference_p50,
            "blocks": references.blocks.len(),
            "unstable_blocks": unstable,
            "unstable_fraction": unstable as f64 / spreads.len() as f64,
            "block_spread_max": spreads.iter().cloned().fold(0.0, f64::max),
            "blocks_ns": references.blocks,
        },
        "cold": report_of(&cold),
        "warmup": report_of(&warmup),
        "hot": report_of(&hot),
    })
}

const MAX_BLOCK_SPREAD: f64 = 1.5;

/// Judges one corpus against the budgets, appending what is over to `failures`. A corpus whose plan reference
/// disagreed with itself is NOT judged: it returns the `INCONCLUSIVE:` note instead (the caller reports a real
/// failure of another corpus first, so an inconclusive corpus never masks a failure).
fn within(report: &Value, budgets: &Value, failures: &mut Vec<String>) -> Option<String> {
    let corpus = report["corpus"].as_str().unwrap();
    let unstable = report["reference"]["unstable_fraction"].as_f64().unwrap();
    let allowed = budgets["reference"]["max_unstable_blocks"]
        .as_f64()
        .unwrap();
    if unstable > allowed {
        return Some(format!(
            "INCONCLUSIVE: {:.0}% of the plan reference blocks of {corpus} disagreed with themselves by more than {MAX_BLOCK_SPREAD}x (limit {:.0}%); the machine was not steady enough to judge",
            unstable * 100.0,
            allowed * 100.0
        ));
    }
    for phase in ["cold", "hot"] {
        for (kind, summary) in report[phase].as_object().unwrap() {
            let Some(limit) = budgets["ratios"][corpus].get(kind) else {
                failures.push(format!("{corpus}/{kind}: measured but has no budget"));
                continue;
            };
            let [p50, p95] = ["ratio_p50", "ratio_p95"].map(|key| summary[key].as_f64().unwrap());
            if p50 > limit["p50_ratio"].as_f64().unwrap() {
                failures.push(format!(
                    "{corpus}/{kind}: p50 is {p50:.2}x the plan (budget {})",
                    limit["p50_ratio"]
                ));
            }
            if p95 > limit["p95_ratio"].as_f64().unwrap() {
                failures.push(format!(
                    "{corpus}/{kind}: p95 is {p95:.2}x the plan (budget {})",
                    limit["p95_ratio"]
                ));
            }
            if let Some(max) = budgets["long_ticks_max"][corpus].as_u64()
                && summary["long_ticks"].as_u64().unwrap() > max
                && phase == "hot"
            {
                failures.push(format!(
                    "{corpus}/{kind}: {} samples over a 60 Hz frame (max {max})",
                    summary["long_ticks"]
                ));
            }
        }
    }
    // The other direction of "measured but has no budget": a budget no sample reached judges nothing, so a handler that
    // stopped being measured would leave the run green. Only a corpus that was judged gets here (an unsteady one has
    // already returned its note, and adds no failure of its own).
    if let Some(kinds) = budgets["ratios"][corpus].as_object() {
        for kind in kinds.keys() {
            if !["cold", "hot"]
                .iter()
                .any(|phase| report[*phase].get(kind).is_some())
            {
                failures.push(format!(
                    "{corpus}/{kind}: has a budget but was not measured"
                ));
            }
        }
    }
    None
}

fn write_report(corpora: &[Value]) {
    if let Ok(path) = std::env::var("TESSEL_GIZMO_PERF_OUT") {
        let report = json!({
            "suite": "gizmo-perf-core",
            "journey_version": Journey::load().version(),
            "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
            "cold_runs": COLD_RUNS, "warmup_cycles": WARMUP_CYCLES, "hot_cycles": HOT_CYCLES,
            "reads": READS, "moves": MOVES, "reference_block": REFERENCE_BLOCK, "read_window": READ_WINDOW,
            "corpora": corpora,
        });
        std::fs::write(path, serde_json::to_string(&report).unwrap()).unwrap();
    }
}

#[test]
#[ignore = "performance: run through scripts/gizmo-perf/run.py (lock, release)"]
fn the_gizmo_handlers_stay_within_their_budget_cold_and_hot_on_every_corpus() {
    let _ = IGNORED;
    assert!(
        release_build() || calibrating(),
        "a time budget is only meaningful in release: run scripts/gizmo-perf/run.py"
    );
    let journey = Journey::load();
    assert_eq!(INJECT.load(Ordering::Relaxed), 0);
    let reports: Vec<Value> = CORPORA
        .iter()
        .map(|name| measure_corpus(&journey, name))
        .collect();
    write_report(&reports);
    for report in &reports {
        println!("GIZMO_PERF_SUMMARY {}", summary_line(report));
    }
    if calibrating() {
        return;
    }
    let budgets = json_file(BUDGETS);
    let mut failures = Vec::new();
    let inconclusive: Vec<String> = reports
        .iter()
        .filter_map(|report| within(report, &budgets, &mut failures))
        .collect();
    // A real failure is reported first and never hidden by an inconclusive corpus.
    assert!(
        failures.is_empty(),
        "over budget:\n{}\n{}",
        failures.join("\n"),
        inconclusive.join("\n")
    );
    assert!(inconclusive.is_empty(), "{}", inconclusive.join("\n"));
}

fn summary_line(report: &Value) -> String {
    let mut rows = Vec::new();
    for phase in ["cold", "hot"] {
        for (kind, s) in report[phase].as_object().unwrap() {
            rows.push(format!(
                "{kind}:p50={:.2}x p95={:.2}x n={}",
                s["ratio_p50"].as_f64().unwrap(),
                s["ratio_p95"].as_f64().unwrap(),
                s["n"]
            ));
        }
    }
    format!(
        "{} ref_p50={}us unstable_blocks={:.0}% | {}",
        report["corpus"],
        report["reference"]["p50_ns"].as_f64().unwrap() / 1000.0,
        report["reference"]["unstable_fraction"].as_f64().unwrap() * 100.0,
        rows.join(" ")
    )
}

fn controlled(inject: u8) -> Vec<String> {
    INJECT.store(inject, Ordering::Relaxed);
    let journey = Journey::load();
    let report = measure_corpus(&journey, "dense");
    INJECT.store(0, Ordering::Relaxed);
    let mut failures = Vec::new();
    if let Some(note) = within(&report, &json_file(BUDGETS), &mut failures) {
        panic!("{note}");
    }
    failures
}

#[test]
#[ignore = "performance: run through scripts/gizmo-perf/run.py (lock, release)"]
fn the_budget_detector_rejects_one_extra_plan_of_work_per_handler_call() {
    assert!(release_build(), "release only");
    if calibrating() {
        return; // no budget exists yet to be tested against
    }
    let failures = controlled(INJECT_PLAN);
    // Negative control: an artificial plan after every call is not the product's flow; it only
    // proves that the budgets above are tight enough to see a plan's worth of extra work.
    for kind in [
        "state.translate",
        "down.translate",
        "read.translate",
        "cold.state",
    ] {
        assert!(
            failures
                .iter()
                .any(|line| line.starts_with(&format!("dense/{kind}:"))),
            "an extra plan per call was not caught for {kind}: {failures:#?}"
        );
    }
}

#[test]
#[ignore = "performance: run through scripts/gizmo-perf/run.py (lock, release)"]
fn the_budget_detector_rejects_a_millisecond_of_delay_per_handler_call() {
    assert!(release_build(), "release only");
    if calibrating() {
        return;
    }
    let failures = controlled(INJECT_DELAY);
    for kind in [
        "state.translate",
        "move.translate",
        "up.scale",
        "read.hidden",
    ] {
        assert!(
            failures
                .iter()
                .any(|line| line.starts_with(&format!("dense/{kind}:"))),
            "a 1 ms delay per call was not caught for {kind}: {failures:#?}"
        );
    }
}

// What `within` judges, on reports written by hand: no clock and no editor, so these four run in the plain
// `cargo test` (the three tests above are the ones that measure, and stay `#[ignore]`).

fn hand_report(corpus: &str, unstable: f64, cold: &[&str], hot: &[&str]) -> Value {
    let summary = json!({"ratio_p50": 1.0, "ratio_p95": 1.0, "long_ticks": 0});
    let phase = |kinds: &[&str]| -> Value {
        Value::Object(
            kinds
                .iter()
                .map(|kind| (kind.to_string(), summary.clone()))
                .collect(),
        )
    };
    json!({
        "corpus": corpus,
        "reference": {"unstable_fraction": unstable},
        "cold": phase(cold),
        "hot": phase(hot),
    })
}

fn hand_budgets(corpus: &str, kinds: &[&str]) -> Value {
    let limit = json!({"p50_ratio": 2.0, "p95_ratio": 3.0});
    let ratios: serde_json::Map<String, Value> = kinds
        .iter()
        .map(|kind| (kind.to_string(), limit.clone()))
        .collect();
    let mut by_corpus = serde_json::Map::new();
    by_corpus.insert(corpus.to_string(), Value::Object(ratios));
    let mut long_ticks = serde_json::Map::new();
    long_ticks.insert(corpus.to_string(), json!(0));
    json!({
        "reference": {"max_unstable_blocks": 0.25},
        "long_ticks_max": long_ticks,
        "ratios": by_corpus,
    })
}

#[test]
fn a_budget_no_sample_reached_fails_the_run_instead_of_judging_nothing() {
    let budgets = hand_budgets("small", &["cold.state", "move.translate", "read.refused"]);
    // `read.refused` stopped being recorded: its budget would have judged nothing and the run stayed green.
    let report = hand_report("small", 0.0, &["cold.state"], &["move.translate"]);
    let mut failures = Vec::new();
    assert_eq!(within(&report, &budgets, &mut failures), None);
    assert_eq!(
        failures,
        ["small/read.refused: has a budget but was not measured"]
    );
}

#[test]
fn a_kind_measured_in_either_phase_satisfies_its_budget() {
    let budgets = hand_budgets("small", &["cold.state", "move.translate"]);
    let report = hand_report("small", 0.0, &["cold.state"], &["move.translate"]);
    let mut failures = Vec::new();
    assert_eq!(within(&report, &budgets, &mut failures), None);
    assert!(failures.is_empty(), "{failures:?}");
}

#[test]
fn a_measured_kind_without_a_budget_still_fails() {
    let budgets = hand_budgets("small", &["move.translate"]);
    let report = hand_report("small", 0.0, &[], &["move.translate", "move.rotate"]);
    let mut failures = Vec::new();
    assert_eq!(within(&report, &budgets, &mut failures), None);
    assert_eq!(failures, ["small/move.rotate: measured but has no budget"]);
}

#[test]
fn an_unsteady_corpus_is_not_judged_and_adds_no_failure_of_its_own() {
    let budgets = hand_budgets("small", &["move.translate", "read.refused"]);
    let report = hand_report("small", 0.5, &[], &["move.translate"]);
    let mut failures = Vec::new();
    let note = within(&report, &budgets, &mut failures).expect("an unsteady corpus is not judged");
    assert!(note.starts_with("INCONCLUSIVE:"), "{note}");
    assert!(failures.is_empty(), "{failures:?}");
}
