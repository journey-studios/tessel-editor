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
//! named in [`PENDING`] with the reason **and the symptom S1 shows today** ([`Symptom`]):
//!
//! * `Layer(layer)`: refused with `SHOTS_LAYER_NOT_YET`, whose `details.layer` is pinned (S1 refuses
//!   an op that would have to move a layer it does not move);
//! * `Gate(findings)`: refused with `CANDIDATE_REJECTED` carrying those importer findings;
//! * `LayersOnly`: accepted, and the part S1 controls (shots, instances, `editDurationFrames`, the
//!   rest of the document) equals the oracle's once `fx` and `sceneDefinitions` are removed from
//!   both sides. No case is in this state today: the layer guard refuses instead of leaving a
//!   layer behind;
//! * `Explain`: `scene.shots.explain` is S3's.
//!
//! The test fails if a pending case starts to reproduce without leaving the list, if its symptom
//! changes, and if a case outside the list does not reproduce. A mismatch is a finding for the
//! coordinator, not a reason to edit the corpus or bend the rule.

use core::prelude::v1::test;

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::fx::TransitionAdmission;
use tessel_editor_core::directed_scene::{DirectedScene, Limits};
use tessel_editor_core::scene_edit::EditError;
use tessel_editor_core::scene_edit::shots::{
    SHOTS_LAYER_NOT_YET, ShotsOutcome, apply_shots, parse_shot_ops,
};

mod shot_trim_support;
use shot_trim_support::{project, unproject};

const CASES: &str = include_str!("../../../docs/migration/fixtures/shot-trim/cases.json");

/// Why a case is not reproduced by S1: who delivers what it needs.
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

/// What S1 does with a pending case today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Symptom {
    /// `SHOTS_LAYER_NOT_YET` for this layer (`fx_clip_output`, `fx_clip_shot`, `transition`,
    /// `light_keys`, `weather_keys`), with a JSON pointer in `details.path`.
    Layer(&'static str),
    /// `CANDIDATE_REJECTED` with at least these importer findings.
    Gate(&'static [&'static str]),
    /// Accepted; only the layers differ from the oracle's document.
    #[allow(dead_code)]
    LayersOnly,
    /// An `explain` case: not an op.
    Explain,
}

use Pending::{Explain as ExplainOf, FxClips, Keys, Transitions};
use Symptom::{Explain, Gate, Layer};

/// Cases S1 does not reproduce, with the reason and the symptom. Checked in both directions.
const PENDING: &[(&str, Pending, Symptom)] = &[
    ("t11-head-cut-moves-clips", FxClips, Layer("fx_clip_shot")),
    ("t11-end-shrink-cuts-tail", FxClips, Layer("fx_clip_shot")),
    (
        "t11-head-earlier-anchor-moves-clips",
        FxClips,
        Layer("fx_clip_shot"),
    ),
    (
        "t11-roll-follow-slides-clips",
        FxClips,
        Layer("fx_clip_shot"),
    ),
    (
        "t11-roll-later-cuts-head-clips",
        FxClips,
        Layer("fx_clip_shot"),
    ),
    (
        "t11-continuation-clips-follow",
        FxClips,
        Layer("fx_clip_shot"),
    ),
    (
        "t11-later-shot-clips-move-with-plan",
        FxClips,
        Layer("fx_clip_shot"),
    ),
    (
        "t12-shrink-removes-and-cuts",
        FxClips,
        Layer("fx_clip_output"),
    ),
    (
        "t12-grow-stretches-crossing",
        FxClips,
        Layer("fx_clip_output"),
    ),
    ("t12-keep-absolute-overlap", FxClips, Layer("fx_clip_shot")),
    ("t12-head-cut-clip", FxClips, Layer("fx_clip_output")),
    ("t12-roll-follow-chain", FxClips, Layer("fx_clip_output")),
    ("t12-head-earlier-chain", FxClips, Layer("fx_clip_output")),
    (
        "t13-moves-whole-keys-intact",
        FxClips,
        Layer("fx_clip_output"),
    ),
    (
        "t13-head-cut-shifts-and-prunes",
        FxClips,
        Layer("fx_clip_output"),
    ),
    ("t13-tail-cut-prunes", FxClips, Layer("fx_clip_output")),
    ("t13-stretched-keeps-keys", FxClips, Layer("fx_clip_output")),
    ("t13-fade-reduced", FxClips, Layer("fx_clip_output")),
    ("t13-fade-out-reduced", FxClips, Layer("fx_clip_output")),
    ("t13-fade-gone", FxClips, Layer("fx_clip_output")),
    ("t13-all-keys-pruned", FxClips, Layer("fx_clip_output")),
    ("batch-with-output-clips", FxClips, Layer("fx_clip_output")),
    ("t14-too-long", Transitions, Gate(&["TRANSITION_TOO_LONG"])),
    ("t14-shrink-center", Transitions, Layer("transition")),
    ("t14-shrink-start", Transitions, Layer("transition")),
    ("t14-shrink-end", Transitions, Layer("transition")),
    (
        "t14-too-long-start-incoming",
        Transitions,
        Gate(&["TRANSITION_OUT_OF_RANGE"]),
    ),
    (
        "t14-too-long-roll",
        Transitions,
        Gate(&["TRANSITION_TOO_LONG"]),
    ),
    (
        "t14-too-long-head",
        Transitions,
        Gate(&["TRANSITION_TOO_LONG"]),
    ),
    (
        "t14-edit-duration",
        Transitions,
        Gate(&["TRANSITION_OUT_OF_RANGE"]),
    ),
    (
        "t14-two-transitions-shrink",
        Transitions,
        Layer("transition"),
    ),
    ("t15-keys-move-after-point", Keys, Layer("light_keys")),
    ("t15-keys-dropped-in-removed", Keys, Layer("light_keys")),
    ("t15-empty-track-removed", Keys, Layer("light_keys")),
    (
        "t15-key-at-insertion-point-moves",
        Keys,
        Layer("light_keys"),
    ),
    ("t15-roll-follow-chain", Keys, Layer("light_keys")),
    ("t16-weather-keys-remapped", Keys, Layer("weather_keys")),
    ("t16-weather-keys-dropped", Keys, Layer("weather_keys")),
    ("t16-weather-keys-emptied", Keys, Layer("weather_keys")),
    ("t19-explain-handles", ExplainOf, Explain),
    ("t19-explain-locked-and-continuous", ExplainOf, Explain),
    ("t19-audio-longer-than-edit", ExplainOf, Explain),
    ("t19-edit-longer-than-audio", ExplainOf, Explain),
    ("t19-audio-within-one-frame", ExplainOf, Explain),
    ("t19-no-audio", ExplainOf, Explain),
    ("t19-window-not-at-zero", ExplainOf, Explain),
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

/// What Rust did with a case.
enum Observed {
    /// An `explain` case: no op to run.
    Explain,
    Refused(EditError),
    /// The outcome (boxed: a scene is large) and the candidate read back into the oracle's minimal form.
    Accepted(Box<ShotsOutcome>, Value),
}

fn observe(case: &Value) -> Result<Observed, String> {
    if case["kind"] == "explain" {
        return Ok(Observed::Explain);
    }
    let minimal = &case["document"];
    let scene = admitted(project(minimal))?;
    let ops = parse_shot_ops(&Value::Array(case["ops"].as_array().unwrap().clone()))
        .map_err(|error| format!("the ops did not parse: {error}"))?;
    Ok(match apply_shots(&scene, &ops) {
        Ok(outcome) => {
            let found = unproject(outcome.candidate.document(), minimal);
            Observed::Accepted(Box::new(outcome), found)
        }
        Err(error) => Observed::Refused(error),
    })
}

/// The codes of the findings of a `CANDIDATE_REJECTED`.
fn finding_codes(error: &EditError) -> Vec<&'static str> {
    error
        .issues()
        .iter()
        .map(|found| found.issue.code)
        .collect()
}

/// `Ok` means Rust gave what the oracle's `expect` says.
fn reproduces(case: &Value, observed: &Observed) -> Result<(), String> {
    if matches!(observed, Observed::Explain) {
        return Err("explain is S3".into());
    }
    let expect = &case["expect"];
    match (observed, expect["ok"].as_bool().unwrap()) {
        (Observed::Explain, _) => unreachable!("handled above"),
        (Observed::Accepted(outcome, found), true) => {
            let notes: Vec<Value> = outcome.notes.iter().map(|note| note.to_json()).collect();
            if *found != expect["document"] {
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
        (Observed::Refused(error), false) => {
            let refusal = &expect["refusal"];
            let code = refusal["code"].as_str().unwrap();
            if error.code != code {
                return Err(format!(
                    "refused with {} (expected {code}): {error}",
                    error.code
                ));
            }
            if code == "CANDIDATE_REJECTED" {
                // The gate attributes a finding to the op by pointer, so the oracle has no index;
                // the importer names its findings with the same codes the oracle models.
                let found = finding_codes(error);
                for finding in refusal["findings"].as_array().unwrap() {
                    let wanted = finding["code"].as_str().unwrap();
                    if !found.contains(&wanted) {
                        return Err(format!(
                            "the gate did not find {wanted} (it found {found:?})"
                        ));
                    }
                }
            } else if error.index != refusal["index"].as_u64().map(|i| i as usize) {
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
        (Observed::Accepted(..), false) => Err(format!(
            "accepted; the oracle refuses with {}",
            expect["refusal"]
        )),
        (Observed::Refused(error), true) => Err(format!("refused with {}: {error}", error.code)),
    }
}

/// The document without what S2 owns (`fx` and `sceneDefinitions`): what S1 controls.
fn without_layers(document: &Value) -> Value {
    let mut out = document.clone();
    let object = out.as_object_mut().unwrap();
    object.remove("fx");
    object.remove("sceneDefinitions");
    out
}

/// Checks that a pending case shows the symptom the list pins.
fn symptom_holds(case: &Value, observed: &Observed, symptom: Symptom) -> Result<(), String> {
    match (symptom, observed) {
        (Symptom::Explain, Observed::Explain) => Ok(()),
        (Symptom::Layer(layer), Observed::Refused(error)) => {
            if error.code != SHOTS_LAYER_NOT_YET {
                return Err(format!(
                    "pinned as a refusal of the layer `{layer}`, refused with {}: {error}",
                    error.code
                ));
            }
            let details = error.details().ok_or("the refusal has no details")?;
            if details["layer"] != layer {
                return Err(format!("the layer is {}, pinned {layer}", details["layer"]));
            }
            if !details["path"].as_str().is_some_and(|p| p.starts_with('/')) {
                return Err(format!(
                    "the path is not a JSON pointer: {}",
                    details["path"]
                ));
            }
            Ok(())
        }
        (Symptom::Gate(wanted), Observed::Refused(error)) => {
            if error.code != "CANDIDATE_REJECTED" {
                return Err(format!(
                    "pinned as a gate refusal, refused with {}: {error}",
                    error.code
                ));
            }
            let found = finding_codes(error);
            match wanted.iter().find(|code| !found.contains(code)) {
                Some(missing) => Err(format!("the gate did not find {missing}: {found:?}")),
                None => Ok(()),
            }
        }
        (Symptom::LayersOnly, Observed::Accepted(_, found)) => {
            let expected = &case["expect"]["document"];
            if without_layers(found) != without_layers(expected) {
                return Err(format!(
                    "the part S1 controls differs\n  found:    {}\n  expected: {}",
                    without_layers(found),
                    without_layers(expected)
                ));
            }
            if found == expected {
                return Err("the layers do not differ".into());
            }
            Ok(())
        }
        (symptom, Observed::Accepted(..)) => {
            Err(format!("pinned as {symptom:?}, but the op was accepted"))
        }
        (symptom, Observed::Refused(error)) => Err(format!(
            "pinned as {symptom:?}, refused with {}: {error}",
            error.code
        )),
        (symptom, Observed::Explain) => Err(format!("pinned as {symptom:?}, but it is `explain`")),
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
    let mut symptoms: Vec<(String, usize)> = Vec::new();
    for case in &cases {
        let id = case["id"].as_str().unwrap();
        let pending = PENDING.iter().find(|(name, ..)| *name == id);
        let observed = match observe(case) {
            Ok(observed) => observed,
            Err(why) => {
                failures.push(format!("{id}: could not run: {why}"));
                continue;
            }
        };
        let outcome = reproduces(case, &observed);
        match (pending, outcome) {
            (None, Ok(())) => reproduced += 1,
            (None, Err(why)) => failures.push(format!("{id}: not reproduced: {why}")),
            (Some((_, reason, _)), Ok(())) => failures.push(format!(
                "{id}: listed pending ({reason:?}) but it reproduces: take it off the list"
            )),
            (Some((_, reason, symptom)), Err(why)) => {
                // A case is pending for the layer it has, not for a reason it was typed with.
                if !layer_of(case).contains(reason) {
                    failures.push(format!(
                        "{id}: pending as {reason:?} but the document has {:?}",
                        layer_of(case)
                    ));
                }
                // And it shows the symptom the list pins, not some other failure.
                if let Err(drift) = symptom_holds(case, &observed, *symptom) {
                    failures.push(format!("{id}: the symptom changed: {drift}"));
                }
                let name = format!("{symptom:?}");
                match symptoms.iter_mut().find(|(known, _)| *known == name) {
                    Some((_, count)) => *count += 1,
                    None => symptoms.push((name, 1)),
                }
                report.push(format!("{id}: {}", why.lines().next().unwrap_or_default()));
            }
        }
    }
    for line in &report {
        println!("pending {line}");
    }
    for (symptom, count) in &symptoms {
        println!("symptom {symptom}: {count}");
    }
    println!(
        "reproduced {reproduced} of {}; pending {}",
        cases.len(),
        PENDING.len()
    );
    for name in PENDING.iter().map(|(name, ..)| name) {
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
