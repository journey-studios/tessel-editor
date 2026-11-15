//! L (corpus): the committed oracle `docs/migration/fixtures/pilot-fx/dissolve-cases.json` (S0),
//! read here by Rust with no Python. The oracle's numbers are the expectation; nothing below is
//! derived from the importer, and none of the fixture's rows or tolerances is changed or added to.
//!
//! Where the destination knowingly differs from the oracle the case is named and asserted on its
//! own (`known_divergences`), never skipped silently.

use std::collections::BTreeSet;

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::ImportError;
use tessel_editor_core::directed_scene::fx::{
    self, Alignment, available_frames, default_transition_duration, max_transition_frames,
    transition_window,
};

use crate::support::*;

const CASES: &str =
    include_str!("../../../../docs/migration/fixtures/pilot-fx/dissolve-cases.json");

fn cases() -> Value {
    serde_json::from_str(CASES).unwrap()
}

fn alignment(name: &str) -> Alignment {
    Alignment::parse(name).unwrap_or_else(|| panic!("alignment {name}"))
}

fn issues_json(error: &ImportError) -> Vec<Value> {
    error.issues().iter().map(|issue| issue.to_json()).collect()
}

/// The oracle's issue and the importer's must agree on what a person reads and what an adapter
/// acts on: code, message, hint, the transition and the lacking frames.
fn assert_issue_matches(name: &str, found: &Value, expected: &Value) {
    assert_eq!(found["code"], expected["code"], "{name}: code");
    assert_eq!(found["message"], expected["message"], "{name}: message");
    let details = &found["details"];
    assert_eq!(
        details["transitionId"], expected["transitionId"],
        "{name}: transitionId"
    );
    // The pinned pilot used camelCase. Only this hint's operation name differs in the editor;
    // the immutable oracle remains unchanged and every other character/issue still compares.
    let expected_hint = if expected["code"] == "TRANSITION_DUPLICATE" {
        assert_eq!(
            expected["hint"],
            "One transition per cut: setTransition replaces the one that is there."
        );
        Some(json!(
            "One transition per cut: set_transition replaces the one that is there."
        ))
    } else {
        expected.get("hint").cloned()
    };
    assert_eq!(details.get("hint"), expected_hint.as_ref(), "{name}: hint");
    assert_eq!(details.get("lacks"), expected.get("lacks"), "{name}: lacks");
}

#[test]
fn the_oracle_is_the_committed_file_of_the_expected_shape() {
    let cases = cases();
    assert_eq!(cases["schema"], 1);
    assert_eq!(cases["explain"].as_array().unwrap().len(), 32);
    assert_eq!(cases["handles"].as_array().unwrap().len(), 12);
    assert_eq!(cases["windows"]["bounds"].as_array().unwrap().len(), 15);
    assert_eq!(
        cases["windows"]["max_transition_frames"]
            .as_array()
            .unwrap()
            .len(),
        15
    );
    assert_eq!(cases["windows"]["raw"].as_array().unwrap().len(), 30);
}

#[test]
fn explain_cases_agree_with_the_oracle() {
    let cases = cases();
    let mut seen = BTreeSet::new();
    for case in cases["explain"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let list = case["transitions"].as_array().unwrap().clone();
        let bundle = match case["edit"].as_str().unwrap() {
            "spec" => with_transitions(Edit::spec().bundle(), list),
            "corpus" => with_transitions(corpus(), list),
            // The oracle reports TRANSITION_DUPLICATE next to a legacy dissolve; this importer
            // refuses the legacy dissolve earlier (DISSOLVE_UNSUPPORTED). Asserted separately in
            // `known_divergences` and in `document.rs`.
            "spec+legacy-dissolve" => continue,
            other => panic!("{name}: edit {other}"),
        };
        let expected_codes: Vec<&str> = case["codes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|code| code.as_str().unwrap())
            .collect();
        match admit(bundle) {
            Ok(scene) => {
                assert!(
                    expected_codes.is_empty(),
                    "{name}: accepted but the oracle refuses"
                );
                assert_eq!(
                    scene.bundle().fx.transitions.len(),
                    case["transitions"].as_array().unwrap().len(),
                    "{name}"
                );
            }
            Err(error) => {
                assert_eq!(codes(&error), expected_codes, "{name}");
                let found = issues_json(&error);
                let expected = case["issues"].as_array().unwrap();
                assert_eq!(found.len(), expected.len(), "{name}: {found:?}");
                for (found, expected) in found.iter().zip(expected) {
                    assert_issue_matches(name, found, expected);
                }
            }
        }
        seen.insert(name.to_string());
    }
    assert_eq!(
        seen.len(),
        31,
        "every case but the legacy-dissolve one was compared"
    );
}

/// The 12 handle scenarios, each on the edit the oracle's scenario builds.
fn handle_edit(name: &str) -> Value {
    let spec = Edit::spec;
    match name {
        n if n.starts_with("corpus-") => corpus(),
        "variant-only-the-camera-is-short" => {
            // shot-b's instance starts at 10 (a head of its own); the instance grows by the same
            // 10 so it still covers the shot, as the importer requires.
            let mut bundle = corpus();
            bundle["shots"][1]["instanceStartFrame"] = json!(10);
            bundle["sceneInstances"][1]["durationFrames"] = json!(70);
            bundle
        }
        "origin-spec-s1-s2-use" | "origin-spec-s2-s3-use-has-the-frames" => spec().bundle(),
        "origin-spec-s2-s3-camera-cut-to-50" => spec().camera("cam2", 50).bundle(),
        "origin-spec-s1-s2-cam1-96-both-lack-the-head" => spec().camera("cam1", 96).bundle(),
        "origin-spec-s2-camera-in-10-instance-only" => spec()
            .camera("cam1", 96)
            .shot("s2", |shot| shot.camera_in = 10)
            .bundle(),
        "origin-spec-s3-camera-in-4-end-12" => {
            spec().shot("s3", |shot| shot.camera_in = 4).bundle()
        }
        other => panic!("no edit for {other}"),
    }
}

#[test]
fn handle_scenarios_agree_with_the_oracle() {
    let cases = cases();
    for case in cases["handles"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let bundle = with_transitions(handle_edit(name), vec![case["transition"].clone()]);
        let expected = case["issues"].as_array().unwrap();
        let window = &case["window"];
        match admit(bundle) {
            Ok(scene) => assert!(
                expected.is_empty(),
                "{name}: accepted but the oracle refuses {scene:?}"
            ),
            Err(error) => {
                let found = issues_json(&error);
                assert_eq!(found.len(), expected.len(), "{name}: {found:?}");
                for (found, expected) in found.iter().zip(expected) {
                    assert_issue_matches(name, found, expected);
                }
            }
        }
        // The window the oracle names is the one the typed arithmetic computes.
        let transition = &case["transition"];
        let edit_duration = 1_000_000;
        let cut = match name {
            n if n.starts_with("corpus-") || n.starts_with("variant-") => 40,
            n if n.contains("-s2-s3") || n.contains("s3-camera") => 96,
            _ => 48,
        };
        let computed = transition_window(
            cut,
            transition["durationFrames"].as_i64().unwrap(),
            alignment(transition["alignment"].as_str().unwrap()),
            edit_duration,
        )
        .unwrap();
        assert_eq!(
            json!({"start": computed.start, "end": computed.end,
                   "before": computed.before, "after": computed.after}),
            *window,
            "{name}"
        );
    }
}

#[test]
fn window_bounds_agree_with_the_oracle() {
    let cases = cases();
    for row in cases["windows"]["bounds"].as_array().unwrap() {
        let (cut, duration, edit) = (
            row["cut"].as_i64().unwrap(),
            row["duration"].as_i64().unwrap(),
            row["edit_duration"].as_i64().unwrap(),
        );
        let result = transition_window(
            cut,
            duration,
            alignment(row["alignment"].as_str().unwrap()),
            edit,
        );
        let outcome = &row["outcome"];
        let label = format!("{row}");
        if outcome["ok"] == true {
            let window = result.unwrap_or_else(|error| panic!("{label}: {error:?}"));
            assert_eq!(
                json!({"start": window.start, "end": window.end,
                       "before": window.before, "after": window.after}),
                outcome["value"],
                "{label}"
            );
        } else {
            assert!(result.is_err(), "{label}");
            assert_eq!(outcome["code"], fx::TRANSITION_OUT_OF_RANGE, "{label}");
        }
    }
}

#[test]
fn raw_windows_and_the_longest_transition_agree_with_the_oracle() {
    let cases = cases();
    for row in cases["windows"]["raw"].as_array().unwrap() {
        let window = transition_window(
            row["cut"].as_i64().unwrap(),
            row["duration"].as_i64().unwrap(),
            alignment(row["alignment"].as_str().unwrap()),
            1_000_000,
        )
        .unwrap();
        assert_eq!(
            json!({"start": window.start, "end": window.end,
                   "before": window.before, "after": window.after}),
            row["window"],
            "{row}"
        );
    }
    for row in cases["windows"]["max_transition_frames"]
        .as_array()
        .unwrap()
    {
        let found = max_transition_frames(
            alignment(row["alignment"].as_str().unwrap()),
            row["from_frames"].as_i64().unwrap(),
            row["to_frames"].as_i64().unwrap(),
        );
        assert_eq!(found, row["max"].as_i64().unwrap(), "{row}");
    }
}

#[test]
fn strict_inputs_agree_with_the_oracle() {
    let cases = cases();
    let strict = &cases["strict_inputs"];
    for row in strict["availability"].as_array().unwrap() {
        let found = available_frames(
            row["total"].as_i64().unwrap(),
            row["start"].as_i64().unwrap(),
            row["length"].as_i64().unwrap(),
        );
        assert_eq!(found, row["available"].as_i64().unwrap(), "{row}");
    }
    // A duration that is not a strict integer is refused on its own field, and only there.
    for row in strict["duration"].as_array().unwrap() {
        let value = row["value"].clone();
        let text = with_transitions(
            Edit::spec().bundle(),
            vec![tr(json!({"durationFrames": value.clone()}))],
        )
        .to_string();
        let error = admit_str(&text)
            .err()
            .unwrap_or_else(|| panic!("{row} was accepted"));
        assert_eq!(error.issues().len(), 1, "{row}: {:?}", error.issues());
        assert_eq!(
            error.issues()[0].path,
            "/fx/transitions/0/durationFrames",
            "{row}"
        );
    }
    // Seeds: preserved in range, refused outside it or when not an integer.
    for row in strict["seeds"].as_array().unwrap() {
        let value = row["value"].clone();
        let document = with_transitions(
            Edit::spec().bundle(),
            vec![tr(json!({"seed": value.clone()}))],
        );
        let text = document.to_string();
        match (admit_str(&text), row["outcome"]["ok"] == true) {
            (Ok(scene), true) => {
                assert_eq!(
                    scene.bundle().fx.transitions[0].seed,
                    Some(row["outcome"]["value"].as_u64().unwrap() as u32),
                    "{row}"
                );
                assert_eq!(
                    scene.document()["fx"]["transitions"][0]["seed"],
                    value,
                    "{row}"
                );
            }
            (Err(error), false) => {
                assert_eq!(error.issues().len(), 1, "{row}: {:?}", error.issues());
                assert_eq!(error.issues()[0].path, "/fx/transitions/0/seed", "{row}");
            }
            (found, _) => panic!("{row}: importer {:?}", found.map(|_| "accepted")),
        }
    }
    // A window that leaves the edit is refused, never clamped. The three oracle rows are windows
    // of an edit of 100 frames; the destination states them on the cut of its own corpus edit.
    for (alignment_name, duration, cut, edit) in [
        ("end", 12, 11, 100),
        ("center", 12, 95, 100),
        ("start", 1, 100, 100),
    ] {
        let result = transition_window(cut, duration, alignment(alignment_name), edit);
        assert!(result.is_err(), "{alignment_name} {duration} at {cut}");
    }
    assert_eq!(
        strict["invalid_window_is_refused"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    // Extreme cuts never wrap or saturate into a valid window.
    assert!(transition_window(i64::MAX, 2, Alignment::Start, 100).is_err());
    assert!(transition_window(i64::MIN, 2, Alignment::Center, 100).is_err());
    assert!(transition_window(i64::MIN + 1, 100, Alignment::End, 100).is_err());
    assert!(transition_window(i64::MAX - 1, 100, Alignment::Start, 100).is_err());
}

#[test]
fn id_rows_agree_with_the_oracle() {
    let cases = cases();
    let ids = &cases["ids"];
    let verdict = |id: &str| -> Option<&'static str> { fx::id_problem(id).map(|(code, _)| code) };
    for row in ids["valid"].as_array().unwrap() {
        let id = row["id"].as_str().unwrap();
        assert_eq!(verdict(id), None, "{id:?}");
        let transition = tr(json!({"id": id}));
        assert!(
            admit(with_transitions(Edit::spec().bundle(), vec![transition])).is_ok(),
            "{id:?}"
        );
    }
    for row in ids["invalid"].as_array().unwrap() {
        let id = row["id"].as_str().unwrap();
        let expected_problem = row["problems"].as_array().unwrap();
        if expected_problem.is_empty() {
            // The oracle's own sample of a name that matches (a letter and a dash).
            assert_eq!(verdict(id), None, "{id:?}");
            continue;
        }
        assert_eq!(verdict(id), Some("INVALID_VALUE"), "{id:?}");
        let (code, path, _) = only_issue(with_transitions(
            Edit::spec().bundle(),
            vec![tr(json!({"id": id}))],
        ));
        assert_eq!(
            (code.as_str(), path.as_str()),
            ("INVALID_VALUE", "/fx/transitions/0/id"),
            "{id:?}"
        );
    }
    for row in ids["reserved"].as_array().unwrap() {
        let id = row["id"].as_str().unwrap();
        assert_eq!(verdict(id), Some(fx::RESERVED_ID), "{id:?}");
    }
    // Duplicates: the oracle's `taken` set is a track and a transition.
    let track = json!({"id": "track-1", "label": "T", "clips": []});
    for row in ids["duplicate"].as_array().unwrap() {
        let id = row["id"].as_str().unwrap();
        let mut document = with_transitions(
            Edit::spec().bundle(),
            vec![
                tr(json!({"id": "a"})),
                tr(json!({"id": id, "fromShotId": "s2", "toShotId": "s3", "durationFrames": 4})),
            ],
        );
        document["fx"]["tracks"] = json!([track]);
        let expected = !row["problems"].as_array().unwrap().is_empty();
        let refused = admit(document)
            .err()
            .map(|error| codes(&error).contains(&fx::DUPLICATE_ID));
        // `a` against itself is the first entry's own id: the second entry is the one refused.
        assert_eq!(refused.unwrap_or(false), expected, "{id}");
    }
}

#[test]
fn limits_agree_with_the_oracle() {
    let cases = cases();
    let limits = &cases["limits"];
    for row in limits["fx_limit"].as_array().unwrap() {
        let (shots, transitions) = (
            row["shots"].as_u64().unwrap(),
            row["transitions"].as_u64().unwrap(),
        );
        // An edit of 0 shots has no document; the importer refuses that earlier and for its own
        // reason, so the oracle's two shots=0 rows are stated through the cut arithmetic only.
        if shots == 0 {
            assert_eq!(row["outcome"]["max"], 0);
            continue;
        }
        let edit = Edit::many_shots(shots);
        let list: Vec<Value> = (0..transitions)
            .map(|i| {
                tr(json!({"id": format!("t{i}"), "fromShotId": format!("s{i}"),
                          "toShotId": format!("s{}", i + 1), "durationFrames": 1}))
            })
            .collect();
        let result = admit(with_transitions(edit.bundle(), list));
        if row["outcome"]["verdict"] == "accepted" {
            result.unwrap_or_else(|error| panic!("{row}: {:?}", error.issues()));
        } else {
            let error = result.err().unwrap_or_else(|| panic!("{row} accepted"));
            assert_eq!(codes(&error), [fx::FX_LIMIT], "{row}");
        }
    }
    for row in limits["default_duration"].as_array().unwrap() {
        let found = default_transition_duration(
            24,
            alignment(row["alignment"].as_str().unwrap()),
            row["from_frames"].as_i64().unwrap(),
            row["to_frames"].as_i64().unwrap(),
        );
        let outcome = &row["omitted_duration"];
        assert_eq!(found.is_some(), outcome["ok"] == true, "{row}");
        if let Some(found) = found {
            assert_eq!(found, outcome["value"].as_i64().unwrap(), "{row}");
        }
        assert_eq!(
            max_transition_frames(
                alignment(row["alignment"].as_str().unwrap()),
                row["from_frames"].as_i64().unwrap(),
                row["to_frames"].as_i64().unwrap()
            ),
            row["max"].as_i64().unwrap(),
            "{row}"
        );
    }
    assert_eq!(
        fx::transition_effect("crossDissolve")
            .unwrap()
            .typical_duration_frames,
        24
    );
    for row in limits["explicit_duration"].as_array().unwrap() {
        let max = max_transition_frames(
            alignment(row["alignment"].as_str().unwrap()),
            row["from_frames"].as_i64().unwrap(),
            row["to_frames"].as_i64().unwrap(),
        );
        assert_eq!(
            row["duration"].as_i64().unwrap() <= max,
            row["fits"] == true,
            "{row}"
        );
    }
}

#[test]
fn keys_hidden_and_seed_rows_agree_with_the_oracle() {
    let cases = cases();
    let rows = &cases["keys_hidden_seed"];
    // hidden: true or absent are written; `false` is a structural refusal (the origin's literal).
    for case in rows["hidden"]["cases"].as_array().unwrap() {
        let hidden = &case["hidden"];
        let transition = if hidden.is_null() {
            tr(json!({}))
        } else {
            tr(json!({"hidden": hidden.clone()}))
        };
        let result = admit(with_transitions(Edit::spec().bundle(), vec![transition]));
        match case["origin"].as_str().unwrap() {
            "accepted" => {
                let scene = result.unwrap();
                let drawn = !scene.bundle().fx.transitions[0].hidden;
                assert_eq!(Some(drawn), case["draws"].as_bool(), "{case}");
            }
            _ => assert_eq!(codes(&result.err().unwrap()), ["INVALID_VALUE"], "{case}"),
        }
    }
    // keys: refused, with the code of the plan, and the oracle's divergence name is KEYS_REFUSED.
    assert_eq!(rows["keys"]["divergence"], "KEYS_REFUSED");
    assert_eq!(
        admit_codes(with_transitions(
            Edit::spec().bundle(),
            vec![tr(json!({"keys": {}}))]
        )),
        [fx::FX_TRANSITION_KEYS_NOT_YET]
    );
    // seed: kept in the document, consumed by nothing (the effect draws no noise).
    assert_eq!(rows["seed"]["preserved"], true);
    assert_eq!(rows["seed"]["consumed_by_crossDissolve"], false);
    assert_eq!(
        fx::transition_effect("crossDissolve").unwrap().reseed,
        fx::Reseed::Never
    );
}

#[test]
fn known_divergences() {
    // 1. Legacy dissolve: refused earlier, by the importer, for every caller.
    let cases = cases();
    let case = cases["explain"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["edit"] == "spec+legacy-dissolve")
        .unwrap();
    assert_eq!(case["codes"], json!(["TRANSITION_DUPLICATE"]));
    let mut bundle = Edit::spec().bundle();
    bundle["shots"][1]["transitionInto"] = json!({"type": "dissolve", "durationFrames": 12});
    let error = admit(with_transitions(
        bundle,
        case["transitions"].as_array().unwrap().clone(),
    ))
    .err()
    .unwrap();
    assert!(codes(&error).contains(&"DISSOLVE_UNSUPPORTED"));

    // 2. Reserved and duplicate: one refusal (RESERVED_ID), where the oracle lists both. The
    //    reserved check runs first and the importer does not look at a duplicate of a bad ID.
    let both = &cases["ids"]["reserved_and_duplicate"];
    assert_eq!(both["problems"], json!(["RESERVED_ID", "DUPLICATE_ID"]));
    let mut document = with_transitions(Edit::spec().bundle(), vec![tr(json!({"id": "fx"}))]);
    document["fx"]["tracks"] = json!([{"id": "fx", "label": "T", "clips": []}]);
    let error = admit(document).err().unwrap();
    assert_eq!(codes(&error).first(), Some(&fx::RESERVED_ID));
}
