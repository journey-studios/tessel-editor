//! F055 / BL015.21: the STRUCTURAL half of the gizmo performance suite, deterministic and part of
//! the default `cargo test` (no clock is read to pass or fail anything here).
//!
//! It runs the shared journey (`docs/migration/fixtures/gizmo-perf/journey.json`, the same script
//! the GPUI host and the real window run) through the real handlers on three synthetic scenes,
//! and asserts what a handler is allowed to COST in work, not in time:
//!
//! - the corpus, the journey and the generator are the ones the manifest hashes (a number measured
//!   on another corpus is not this suite's number);
//! - the RESULT: the stored transform and the history depths after every step are the golden
//!   trace's (so a faster handler that edits the wrong thing fails here), and the commit, cancel,
//!   undo and redo equalities the driver asserts on every run hold;
//! - the plans: each request lays the scene out at most the contracted number of times, the same
//!   on a 5-node scene and on a 617-node one (a per-node cost is the BL015.12 risk), a long drag
//!   costs one layout per move, undo/redo/tool/select cost none;
//! - the memo key the host gates its replanning on is free to read (no layout), stable while
//!   nothing changed and different after every change that moves the picture or the handles.
//!
//! What it cannot say: how long anything takes (`scene_gizmo_perf_budget.rs`, opt-in, release,
//! under the performance lock) or what a window presents (`scripts/gizmo-perf/native.py`).

use core::prelude::v1::test;
use serde_json::{Value, json};
use tessel_editor_core::scene_plan::probe;

#[path = "gizmo_perf_support/mod.rs"]
mod support;
use support::core_surface::CoreSurface;
use support::*;

/// What one request of each kind may lay out. The numbers are the code's design, measured once
/// and read back from it: a state read plans the scene it draws (1); a press plans it to find
/// what is under the pointer and again for the candidate it opens (2); a move, a release and a
/// cancel each plan the candidate once (1); everything that is not a read of the picture plans
/// nothing. An optimization may lower a number (the test is an upper bound); a regression that
/// plans more fails, on every corpus.
fn layout_budget(label: &str) -> u64 {
    match label {
        "scene.gizmo.state"
        | "scene.pointer.move"
        | "scene.pointer.up"
        | "scene.pointer.cancel" => 1,
        "scene.pointer.down" => 2,
        "scene.gizmo.tool" | "scene.select" | "scene.attach" | "project.create"
        | "history.undo" | "history.redo" | "seek" => 0,
        other => panic!("the journey runs `{other}`: give it a layout budget"),
    }
}

#[test]
fn the_corpus_the_journey_and_the_generator_match_their_manifest() {
    let manifest = json_file(MANIFEST);
    assert_eq!(manifest["corpusVersion"], 1);
    assert_eq!(manifest["private_media"], false);
    for (name, entry) in manifest["files"].as_object().unwrap() {
        let relative = format!("docs/migration/fixtures/gizmo-perf/{name}");
        assert_eq!(
            sha256_of(&relative),
            entry["sha256"].as_str().unwrap(),
            "{name} is not the generated bundle the manifest hashes: run scripts/gizmo-perf/corpus.py"
        );
    }
    assert_eq!(
        sha256_of(JOURNEY),
        manifest["journey"]["sha256"].as_str().unwrap(),
        "the journey changed after the manifest was written"
    );
    assert_eq!(
        sha256_of("scripts/directed-scene-proof/corpus/bundle.json"),
        manifest["source"]["sha256"].as_str().unwrap()
    );
    assert_eq!(
        sha256_of("scripts/gizmo-perf/corpus.py"),
        manifest["generator"]["sha256"].as_str().unwrap()
    );
    let journey = Journey::load();
    assert_eq!(journey.version(), 1);
    let nodes = &manifest["nodes"];
    assert_eq!(nodes["small"], 6);
    assert!(nodes["dense.bundle.json"].as_u64().unwrap() >= 600);
    assert!(nodes["deep.bundle.json"].as_u64().unwrap() >= 36);
}

fn runs() -> Vec<Run> {
    let journey = Journey::load();
    CORPORA
        .iter()
        .map(|name| run(&mut CoreSurface::new(), &journey, &journey.corpus(name)))
        .collect()
}

#[test]
fn the_journey_leaves_the_transform_and_the_history_the_golden_trace_says() {
    let runs = runs();
    let observed: serde_json::Map<String, Value> = runs
        .iter()
        .map(|run| (run.corpus.clone(), run.trace()))
        .collect();
    if std::env::var("TESSEL_GIZMO_PERF_BLESS").is_ok_and(|value| value == "1") {
        let text = serde_json::to_string_pretty(&Value::Object(observed)).unwrap() + "\n";
        std::fs::write(repo_root().join(EXPECTED), text).unwrap();
        return;
    }
    let expected = json_file(EXPECTED);
    for run in &runs {
        let want = &expected[&run.corpus];
        let got = &run.trace();
        assert_eq!(
            got.as_object().unwrap().keys().collect::<Vec<_>>(),
            want.as_object().unwrap().keys().collect::<Vec<_>>(),
            "{}: the steps that ran are not the golden trace's",
            run.corpus
        );
        for (id, step) in want.as_object().unwrap() {
            assert!(
                transforms_equal(&got[id], step),
                "{} / {id}: transform or history differs from the golden trace\n got {}\nwant {step}",
                run.corpus,
                got[id]
            );
        }
    }
}

#[test]
fn a_core_run_skips_only_what_needs_a_window_and_says_why() {
    for run in runs() {
        let skipped: Vec<&StepRecord> = run.steps.iter().filter(|s| s.skipped.is_some()).collect();
        assert!(!skipped.is_empty());
        for step in &skipped {
            assert!(
                step.class == "presentation" || step.class == "presentation_only",
                "{}: `{}` is a document step and was skipped: {:?}",
                run.corpus,
                step.id,
                step.skipped
            );
        }
        let ran = run.steps.iter().filter(|s| s.skipped.is_none()).count();
        let documents = run.steps.iter().filter(|s| s.class == "document").count();
        assert_eq!(
            ran, documents,
            "{}: exactly the document steps run on the core",
            run.corpus
        );
        assert!(documents >= 28);
        // Not one of the skipped steps left a sample: nothing was "measured" by not running.
        for step in skipped {
            assert_eq!(run.samples_of(&step.id).count(), 0, "{}", step.id);
        }
    }
}

#[test]
fn every_request_plans_the_scene_at_most_as_often_as_its_contract_says_on_every_corpus() {
    for run in runs() {
        assert!(
            run.samples.len() >= 80,
            "{}: {} samples",
            run.corpus,
            run.samples.len()
        );
        for sample in &run.samples {
            let layouts = sample.layouts.expect("the core counts its layouts");
            assert!(
                layouts <= layout_budget(&sample.label),
                "{} / {}: `{}` laid the scene out {layouts} times (budget {})",
                run.corpus,
                sample.step,
                sample.label,
                layout_budget(&sample.label)
            );
            assert!(
                sample.ok,
                "{} / {}: {:?}",
                run.corpus, sample.step, sample.code
            );
        }
    }
}

#[test]
fn the_layouts_of_a_request_do_not_depend_on_the_size_of_the_scene() {
    let runs = runs();
    let small = &runs[0];
    for other in &runs[1..] {
        assert_eq!(
            small.samples.len(),
            other.samples.len(),
            "same journey, same requests"
        );
        for (a, b) in small.samples.iter().zip(&other.samples) {
            assert_eq!((&a.step, &a.label), (&b.step, &b.label));
            assert_eq!(
                a.layouts, b.layouts,
                "{} / {}: {} laid out {:?} times on `small` and {:?} on `{}`",
                a.step, a.label, a.label, a.layouts, b.layouts, other.corpus
            );
        }
    }
}

#[test]
fn a_long_drag_costs_one_layout_per_move_and_nothing_else_grows() {
    let journey = Journey::load();
    let corpus = journey.corpus("dense");
    let mut surface = CoreSurface::new();
    let scene = "long";
    for request in [
        json!({"op":"project.create","name":"long"}),
        json!({"op":"scene.attach","id":scene,"path":repo_root().join(&corpus.bundle).canonicalize().unwrap()}),
        json!({"op":"scene.select","scene":scene,"entities":[corpus.node]}),
        json!({"op":"scene.gizmo.tool","tool":"translate","space":"world"}),
    ] {
        assert!(surface.call(&request).ok, "{request}");
    }
    let frame = journey.frame();
    let shown = surface.call(&json!({"op":"scene.gizmo.state","scene":scene,"frame":frame}));
    let tip = shown.value["overlay"]["handles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|handle| handle["id"] == "x")
        .unwrap()["to"]
        .clone();
    let pressed = probe::layouts_built();
    let down =
        surface.call(&json!({"op":"scene.pointer.down","scene":scene,"frame":frame,"point":tip}));
    assert!(down.ok && down.value["dragging"] == true, "{}", down.value);
    let after_press = probe::layouts_built();
    assert_eq!(after_press - pressed, 2);
    const MOVES: u64 = 60;
    for index in 1..=MOVES {
        let before = probe::layouts_built();
        let point = json!({"u": tip["u"].as_f64().unwrap() + 0.0005 * index as f64, "v": tip["v"]});
        let moved = surface.call(&json!({"op":"scene.pointer.move","point":point}));
        assert!(
            moved.ok && moved.value["applied"] == true,
            "{index}: {}",
            moved.value
        );
        assert_eq!(
            probe::layouts_built() - before,
            1,
            "move {index} planned more than once"
        );
    }
    let before = probe::layouts_built();
    assert!(surface.call(&json!({"op":"scene.pointer.up"})).ok);
    assert_eq!(probe::layouts_built() - before, 1);
    assert_eq!(
        probe::layouts_built() - after_press,
        MOVES + 1,
        "the whole gesture after the press"
    );
}

#[test]
fn the_overlay_memo_key_is_free_stable_and_changes_with_everything_that_moves_the_picture() {
    let journey = Journey::load();
    let corpus = journey.corpus("dense");
    let mut surface = CoreSurface::new();
    let scene = "memo";
    let path = repo_root().join(&corpus.bundle).canonicalize().unwrap();
    for request in [
        json!({"op":"project.create","name":"memo"}),
        json!({"op":"scene.attach","id":scene,"path":path}),
        json!({"op":"scene.select","scene":scene,"entities":[corpus.node]}),
    ] {
        assert!(surface.call(&request).ok, "{request}");
    }
    let frame = journey.frame();
    let key = |surface: &CoreSurface, frame: i64| surface.state.gizmo_overlay_key(scene, frame);
    let base = probe::layouts_built();
    let first = key(&surface, frame);
    for _ in 0..1000 {
        assert_eq!(
            key(&surface, frame),
            first,
            "an unchanged state must keep its key"
        );
        let _ = surface.state.gizmo_stamp(scene, frame);
        let _ = surface.state.gizmo_selection(scene);
        let _ = surface.state.gizmo_drag_is_dead();
    }
    assert_eq!(
        probe::layouts_built(),
        base,
        "reading the memo key (what the host does on every sync and tick) must plan nothing"
    );
    let mut seen = vec![first.clone()];
    let mut changed = |what: &str, now: String| {
        assert!(
            !seen.contains(&now),
            "{what} did not change the memo key: the host would show a stale overlay"
        );
        seen.push(now);
    };
    changed("another frame", key(&surface, frame + 1));
    assert!(
        surface
            .call(&json!({"op":"scene.gizmo.tool","tool":"rotate","space":"local"}))
            .ok
    );
    changed("the tool", key(&surface, frame));
    assert!(
        surface
            .call(&json!({"op":"scene.gizmo.tool","tool":"translate","space":"world"}))
            .ok
    );
    assert_eq!(
        key(&surface, frame),
        first,
        "the same tool and space is the same key"
    );
    assert!(surface.call(&json!({"op":"scene.translate","scene":scene,"entity":corpus.node,"frame":frame,"delta":{"x":0.1,"y":0.0,"z":0.0}})).ok);
    changed("an edit of the document", key(&surface, frame));
    assert!(surface.call(&json!({"op":"history.undo"})).ok);
    changed("an undo", key(&surface, frame));
    assert!(
        surface
            .call(&json!({"op":"scene.select","scene":scene,"entities":[corpus.other]}))
            .ok
    );
    changed("another selection", key(&surface, frame));
}
