//! L (corpus, P5 S1): the committed oracle `docs/migration/fixtures/shot-trim/cases.json`, read by
//! Rust with no Python and no network, against the planner and the writer of `scene_edit::shots`.
//!
//! The oracle's documents are minimal; `shot_trim_support` completes each one to a render-bundle
//! 1.0 the importer accepts and reads the result back into the minimal form, so the comparison
//! with `expect` looks at the fields the oracle models. Every case is run through the public
//! `parse_shot_ops` and `apply_shots`; a case's documents are imported under
//! `TransitionAdmission::Accept` (the hidden door the P4b tests use), because the public admission
//! refuses `fx.transitions` today.
//!
//! **No silent skip.** S1 moves shots, instances and the edit length; the layers (effect clips,
//! transitions, light and weather keys) are S2's and `explain` is S3's. Each of those cases is
//! named in [`PENDING`] with the reason. The test fails if a pending case starts to reproduce
//! without leaving the list, and if a case outside it does not reproduce. A mismatch is a finding
//! for the coordinator, not a reason to edit the corpus or bend the rule.

use core::prelude::v1::test;

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::fx::TransitionAdmission;
use tessel_editor_core::directed_scene::{DirectedScene, Limits};
use tessel_editor_core::scene_edit::shots::{apply_shots, parse_shot_ops};

mod shot_trim_support;
use shot_trim_support::{project, unproject};

const CASES: &str = include_str!("../../../docs/migration/fixtures/shot-trim/cases.json");

/// Why a case is not reproduced by S1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    /// An effect clip has to move (D09): S2.
    FxClips,
    /// A transition is tied to the cut (D10): S2, and only in L under `Accept`.
    Transitions,
    /// Light keys or `weatherKeys` have to move (D11, D12): S2.
    Keys,
    /// `scene.shots.explain` (D16): S3.
    Explain,
}

/// Cases S1 does not reproduce, with the reason. Filled from the run, checked in both directions.
const PENDING: &[(&str, Pending)] = &[
    ("t11-head-cut-moves-clips", Pending::FxClips),
    ("t11-end-shrink-cuts-tail", Pending::FxClips),
    ("t11-head-earlier-anchor-moves-clips", Pending::FxClips),
    ("t11-roll-follow-slides-clips", Pending::FxClips),
    ("t11-roll-later-cuts-head-clips", Pending::FxClips),
    ("t11-continuation-clips-follow", Pending::FxClips),
    ("t11-later-shot-clips-move-with-plan", Pending::FxClips),
    ("t12-shrink-removes-and-cuts", Pending::FxClips),
    ("t12-grow-stretches-crossing", Pending::FxClips),
    ("t12-keep-absolute-overlap", Pending::FxClips),
    ("t12-head-cut-clip", Pending::FxClips),
    ("t12-roll-follow-chain", Pending::FxClips),
    ("t12-head-earlier-chain", Pending::FxClips),
    ("t13-moves-whole-keys-intact", Pending::FxClips),
    ("t13-head-cut-shifts-and-prunes", Pending::FxClips),
    ("t13-tail-cut-prunes", Pending::FxClips),
    ("t13-stretched-keeps-keys", Pending::FxClips),
    ("t13-fade-reduced", Pending::FxClips),
    ("t13-fade-out-reduced", Pending::FxClips),
    ("t13-fade-gone", Pending::FxClips),
    ("t13-all-keys-pruned", Pending::FxClips),
    ("batch-with-output-clips", Pending::FxClips),
    ("t14-too-long", Pending::Transitions),
    ("t14-shrink-center", Pending::Transitions),
    ("t14-shrink-start", Pending::Transitions),
    ("t14-shrink-end", Pending::Transitions),
    ("t14-too-long-start-incoming", Pending::Transitions),
    ("t14-too-long-roll", Pending::Transitions),
    ("t14-too-long-head", Pending::Transitions),
    ("t14-edit-duration", Pending::Transitions),
    ("t14-two-transitions-shrink", Pending::Transitions),
    ("t15-keys-move-after-point", Pending::Keys),
    ("t15-keys-dropped-in-removed", Pending::Keys),
    ("t15-empty-track-removed", Pending::Keys),
    ("t15-key-at-insertion-point-moves", Pending::Keys),
    ("t15-roll-follow-chain", Pending::Keys),
    ("t16-weather-keys-remapped", Pending::Keys),
    ("t16-weather-keys-dropped", Pending::Keys),
    ("t16-weather-keys-emptied", Pending::Keys),
    ("t19-explain-handles", Pending::Explain),
    ("t19-explain-locked-and-continuous", Pending::Explain),
    ("t19-audio-longer-than-edit", Pending::Explain),
    ("t19-edit-longer-than-audio", Pending::Explain),
    ("t19-audio-within-one-frame", Pending::Explain),
    ("t19-no-audio", Pending::Explain),
    ("t19-window-not-at-zero", Pending::Explain),
];

fn cases() -> Vec<Value> {
    let corpus: Value = serde_json::from_str(CASES).unwrap();
    corpus["cases"].as_array().unwrap().clone()
}

fn admitted(bundle: Value) -> Result<DirectedScene, String> {
    DirectedScene::import_value_admitting(bundle, &Limits::default(), TransitionAdmission::Accept)
        .map_err(|error| {
            let codes: Vec<&str> = error.issues().iter().map(|issue| issue.code).collect();
            format!("the projected document was refused: {codes:?}")
        })
}

/// Runs one `ops` case; `Ok` means Rust gave what the oracle's `expect` says.
fn reproduces(case: &Value) -> Result<(), String> {
    if case["kind"] == "explain" {
        return Err("explain is S3".into());
    }
    let minimal = &case["document"];
    let scene = admitted(project(minimal))?;
    let before = scene.document().clone();
    let ops = parse_shot_ops(&Value::Array(case["ops"].as_array().unwrap().clone()))
        .map_err(|error| format!("the ops did not parse: {error}"))?;
    let result = apply_shots(&scene, &ops);
    assert_eq!(
        *scene.document(),
        before,
        "{}: the input scene changed",
        case["id"]
    );
    let expect = &case["expect"];
    match (result, expect["ok"].as_bool().unwrap()) {
        (Ok(outcome), true) => {
            let found = unproject(outcome.candidate.document(), minimal);
            let notes: Vec<Value> = outcome.notes.iter().map(|note| note.to_json()).collect();
            if found != expect["document"] {
                return Err(format!(
                    "document differs\n  found:    {found}\n  expected: {}",
                    expect["document"]
                ));
            }
            if notes != *expect["notes"].as_array().unwrap() {
                return Err(format!(
                    "notes differ\n  found:    {}\n  expected: {}",
                    json!(notes),
                    expect["notes"]
                ));
            }
            if outcome.changed != expect["changed"].as_bool().unwrap() {
                return Err(format!("changed is {}", outcome.changed));
            }
            Ok(())
        }
        (Err(error), false) => {
            let refusal = &expect["refusal"];
            let code = refusal["code"].as_str().unwrap();
            if error.code != code {
                return Err(format!(
                    "refused with {} (expected {code}): {error}",
                    error.code
                ));
            }
            // The gate attributes a finding to the op by pointer; the oracle has no pointers.
            if code != "CANDIDATE_REJECTED"
                && error.index != refusal["index"].as_u64().map(|i| i as usize)
            {
                return Err(format!(
                    "refused at index {:?}, expected {}",
                    error.index, refusal["index"]
                ));
            }
            let limit = error
                .details()
                .and_then(|details| details.get("limit_frame"));
            if limit != refusal.get("limitFrame") {
                return Err(format!(
                    "limit_frame is {limit:?}, expected {:?}",
                    refusal.get("limitFrame")
                ));
            }
            Ok(())
        }
        (Ok(_), false) => Err(format!(
            "accepted; the oracle refuses with {}",
            expect["refusal"]
        )),
        (Err(error), true) => Err(format!("refused with {}: {error}", error.code)),
    }
}

fn layer_of(case: &Value) -> Vec<Pending> {
    let document = &case["document"];
    let mut found = Vec::new();
    let fx = &document["fx"];
    let has = |list: &Value| list.as_array().is_some_and(|items| !items.is_empty());
    if fx["tracks"]
        .as_array()
        .is_some_and(|tracks| tracks.iter().any(|track| has(&track["clips"])))
    {
        found.push(Pending::FxClips);
    }
    if has(&fx["transitions"]) {
        found.push(Pending::Transitions);
    }
    let keyed = document["sceneDefinitions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|definition| {
            definition.get("weatherKeys").is_some()
                || definition["lighting"]["lights"]
                    .as_array()
                    .is_some_and(|lights| lights.iter().any(|light| light.get("keys").is_some()))
        });
    if keyed {
        found.push(Pending::Keys);
    }
    if case["kind"] == "explain" {
        found.push(Pending::Explain);
    }
    found
}

#[test]
fn the_corpus_is_what_the_reference_wrote() {
    let cases = cases();
    assert_eq!(cases.len(), 167);
    let explain = cases
        .iter()
        .filter(|case| case["kind"] == "explain")
        .count();
    assert_eq!(explain, 7);
    let mut ids: Vec<&str> = cases
        .iter()
        .map(|case| case["id"].as_str().unwrap())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 167, "case ids are unique");
}

#[test]
fn s1_reproduces_every_case_it_does_not_name_pending() {
    let cases = cases();
    let mut reproduced = 0;
    let mut failures = Vec::new();
    let mut report = Vec::new();
    for case in &cases {
        let id = case["id"].as_str().unwrap();
        let pending = PENDING.iter().find(|(name, _)| *name == id);
        let outcome = reproduces(case);
        match (pending, outcome) {
            (None, Ok(())) => reproduced += 1,
            (None, Err(why)) => failures.push(format!("{id}: not reproduced: {why}")),
            (Some((_, reason)), Ok(())) => failures.push(format!(
                "{id}: listed pending ({reason:?}) but it reproduces: take it off the list"
            )),
            (Some((_, reason)), Err(why)) => {
                // A case is pending for the layer it has, not for a reason it was typed with.
                if !layer_of(case).contains(reason) {
                    failures.push(format!(
                        "{id}: pending as {reason:?} but the document has {:?}",
                        layer_of(case)
                    ));
                }
                report.push(format!("{id}: {}", why.lines().next().unwrap_or_default()));
            }
        }
    }
    for line in &report {
        println!("pending {line}");
    }
    println!(
        "reproduced {reproduced} of {}; pending {}",
        cases.len(),
        PENDING.len()
    );
    for name in PENDING.iter().map(|(name, _)| name) {
        assert!(
            cases.iter().any(|case| case["id"] == *name),
            "{name} is on the pending list but not in the corpus"
        );
    }
    assert!(
        failures.is_empty(),
        "{} case(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}
