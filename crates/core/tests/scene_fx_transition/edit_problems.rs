//! S3a (BL082.4.24, plan §9.4): the camera-lack diagnostic repairs what it says, and every problem
//! of a transition reads in the three languages from one table.
//!
//! The repair tests apply the hint to the edit and read it again through the importer; the
//! localization tests walk the rules of a transition (importer issues and op errors) and check the
//! table against what they really say.

use std::collections::BTreeSet;

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::fx::{
    self, Locale, RawProblem, TransitionProblem, project_issues, project_problem, templates,
    translated_keys,
};
use tessel_editor_core::directed_scene::{DirectedScene, Issue};
use tessel_editor_core::scene_edit::{CANDIDATE_REJECTED, apply_fx, parse_fx_batch};

use crate::support::*;

// ---- E12 (BL082.4.24): the repair is enough --------------------------------------------------

/// `s2` needs 6 real frames before its start (`instanceStartFrame` 6, a centred 12-frame window);
/// its camera is `cam2` of `cam2_frames`, entered at `camera_in`.
fn head(camera_in: u64, cam2_frames: u64) -> Edit {
    Edit::spec()
        .camera("cam1", 60)
        .camera("cam2", cam2_frames)
        .shot("s2", |shot| {
            shot.instance_start = 6;
            shot.camera_in = camera_in;
        })
}

fn refused(edit: &Edit, handles: &str) -> Issue {
    let error = admit(with_transitions(
        edit.bundle(),
        vec![tr(json!({"handles": handles}))],
    ))
    .expect_err("the incoming head should lack frames");
    assert_eq!(
        codes(&error),
        ["TRANSITION_NO_HANDLES"],
        "{:?}",
        error.issues()
    );
    error.issues()[0].clone()
}

fn accepted(edit: &Edit) -> bool {
    admit(with_transitions(
        edit.bundle(),
        vec![tr(json!({"handles": "use"}))],
    ))
    .is_ok()
}

fn lacks(issue: &Issue) -> Value {
    issue.details.as_ref().unwrap()["lacks"].clone()
}

fn hint(issue: &Issue) -> String {
    issue.details.as_ref().unwrap()["hint"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn a_camera_entered_past_its_end_asks_for_the_frames_that_put_it_back_inside() {
    // cameraInFrame 60 over a 48-frame clip: 12 frames past the end, nothing before the start.
    let issue = refused(&head(60, 48), "use");
    assert_eq!(
        lacks(&issue),
        json!([{"shot": "s2", "side": "before its start", "instance": 0, "camera": 0,
                "cameraEnd": 12, "cameraIn": 60, "cameraFrames": 48}])
    );
    assert!(
        hint(&issue).contains("extend its camera clip cam2 by 12 frames at its end"),
        "{}",
        hint(&issue)
    );
    assert!(hint(&issue).contains("cameraInFrame 60 lies inside it"));
    // The old advice (6 more frames, the window's own length) is not enough: still refused.
    assert!(!accepted(&head(60, 54)));
    // The advice now given is enough.
    assert!(accepted(&head(60, 60)));
}

#[test]
fn a_missing_or_invalid_count_preserves_the_original_diagnostic() {
    for key in ["instance", "camera"] {
        for value in [None, Some(json!("0")), Some(json!(-1))] {
            let mut issue = refused(&head(3, 48), "use");
            let entry = issue.details.as_mut().unwrap()["args"]["lacks"][0]
                .as_object_mut()
                .unwrap();
            match &value {
                Some(value) => {
                    entry.insert(key.to_string(), value.clone());
                }
                None => {
                    entry.remove(key);
                }
            }
            for locale in Locale::ALL {
                let result = project_problem(&RawProblem::from_issue(&issue), locale);
                assert_eq!(result.message, issue.message);
                assert_eq!(result.hint.as_deref(), Some(hint(&issue).as_str()));
                assert_eq!(result.details["untranslated"], true);
            }
        }
    }
}

#[test]
fn a_duplicate_cut_hint_names_the_editors_real_operation() {
    let error = admit(two_transitions(json!({"id":"a"}), json!({"id":"b"}))).unwrap_err();
    let issue = error
        .issues()
        .iter()
        .find(|i| i.code == fx::TRANSITION_DUPLICATE)
        .unwrap();
    for locale in Locale::ALL {
        let result = project_problem(&RawProblem::from_issue(issue), locale);
        let text = result.hint.unwrap();
        assert!(text.contains("set_transition"));
        assert!(!text.contains("setTransition"));
    }
}

#[test]
fn one_frame_past_the_end_asks_for_one_frame() {
    let issue = refused(&head(49, 48), "use");
    assert_eq!(lacks(&issue)[0]["cameraEnd"], 1);
    assert!(hint(&issue).contains("by 1 frames at its end"));
    assert!(accepted(&head(49, 49)));
    // Exactly at the exclusive end is valid, as before.
    assert!(accepted(&head(48, 48)));
}

#[test]
fn a_camera_entered_too_early_still_asks_only_for_frames_before_its_start() {
    // cameraInFrame 3 (< 6): 3 frames short at the start, and 3 <= 48 so nothing at the end.
    let issue = refused(&head(3, 48), "use");
    assert_eq!(
        lacks(&issue),
        json!([{"shot": "s2", "side": "before its start", "instance": 0, "camera": 3}])
    );
    assert!(hint(&issue).contains("by 3 frames before its start"));
    assert!(!hint(&issue).contains("at its end"));
    // Giving the frames before the start (camera 3 frames longer, entered at 6) is valid.
    assert!(accepted(&head(6, 51)));
}

#[test]
fn both_faults_are_reported_and_each_repair_is_needed() {
    // cameraInFrame 4 over a 3-frame clip, 6 needed: 2 short at the start, 1 past the end.
    let issue = refused(&head(4, 3), "use");
    assert_eq!(
        lacks(&issue),
        json!([{"shot": "s2", "side": "before its start", "instance": 0, "camera": 2,
                "cameraEnd": 1, "cameraIn": 4, "cameraFrames": 3}])
    );
    let text = hint(&issue);
    assert!(text.contains("by 2 frames before its start"), "{text}");
    assert!(text.contains("by 1 frames at its end"), "{text}");
    // Neither repair alone is enough, both together are (the clip then has 6 frames, entered at 6).
    assert!(!accepted(&head(4, 4)), "only the end");
    assert!(!accepted(&head(6, 5)), "only the start");
    assert!(accepted(&head(6, 6)), "both");
}

#[test]
fn refuse_reads_the_same_frames_as_use_and_keeps_its_note() {
    let issue = refused(&head(60, 48), "refuse");
    assert_eq!(lacks(&issue)[0]["cameraEnd"], 12);
    assert!(hint(&issue).contains("\"refuse\" asks for the same real frames as \"use\""));
}

#[test]
fn a_window_with_no_frames_before_the_cut_does_not_need_a_head() {
    // A start-aligned window has nothing before the cut: the incoming shot's head is never read,
    // so a camera entered past its end is no fault.
    let edit = head(60, 48);
    let scene = admit(with_transitions(
        edit.bundle(),
        vec![tr(json!({"handles": "use", "alignment": "start"}))],
    ));
    assert!(
        scene.is_ok(),
        "{:?}",
        scene.err().map(|e| e.issues().to_vec())
    );
}

#[test]
fn the_three_languages_say_the_same_numbers_and_never_show_a_marker() {
    let issue = refused(&head(4, 3), "use");
    let english = project_problem(&RawProblem::from_issue(&issue), Locale::En);
    assert_eq!(
        english.message, issue.message,
        "English is the importer's text"
    );
    assert_eq!(english.hint.as_deref(), Some(hint(&issue).as_str()));
    let mut seen = Vec::new();
    for locale in Locale::ALL {
        let found = project_problem(&RawProblem::from_issue(&issue), locale);
        let text = format!("{} | {}", found.message, found.hint.clone().unwrap());
        assert!(
            !text.contains('{') && !text.contains('}'),
            "{locale:?}: {text}"
        );
        assert!(found.details.get("untranslated").is_none());
        assert_eq!(found.transition_id.as_deref(), Some("tr"));
        assert_eq!(found.shot_id.as_deref(), Some("s2"));
        // The numbers of the case, whichever the language.
        for number in ["2", "1", "4", "3"] {
            assert!(text.contains(number), "{locale:?} lacks {number}: {text}");
        }
        seen.push(text);
    }
    assert_ne!(seen[0], seen[1]);
    assert_ne!(seen[1], seen[2]);
    assert_ne!(seen[0], seen[2]);
}

// ---- projection: the table against the rules ------------------------------------------------------

/// A problem as a rule produced it: what the projection receives.
struct Found {
    code: String,
    path: Option<String>,
    message: String,
    details: Option<Value>,
}

impl Found {
    fn raw(&self) -> RawProblem<'_> {
        RawProblem {
            code: &self.code,
            path: self.path.as_deref(),
            message: &self.message,
            details: self.details.as_ref(),
        }
    }

    fn key(&self) -> String {
        let kind = self
            .details
            .as_ref()
            .and_then(|details| details["args"]["kind"].as_str());
        match kind {
            Some(kind) => format!("{}:{kind}", self.code),
            None => self.code.clone(),
        }
    }
}

fn from_issues(issues: &[Issue]) -> Vec<Found> {
    issues
        .iter()
        .map(|issue| Found {
            code: issue.code.to_string(),
            path: Some(issue.path.clone()),
            message: issue.message.clone(),
            details: issue.details.clone(),
        })
        .collect()
}

fn doc(over: Value) -> Value {
    with_transitions(Edit::spec().bundle(), vec![tr(over)])
}

/// The importer's findings on one hand-made bundle (the admitting policy).
fn import(bundle: Value) -> Vec<Found> {
    match admit(bundle) {
        Ok(_) => Vec::new(),
        Err(error) => from_issues(error.issues()),
    }
}

/// The problems of an op-level refusal or of the candidate a batch was refused for.
fn through_edit(scene: &DirectedScene, ops: Value) -> Vec<Found> {
    let error = apply_fx(scene, &parse_fx_batch(&ops).unwrap()).unwrap_err();
    if error.code == CANDIDATE_REJECTED {
        return from_issues(
            &error
                .issues()
                .iter()
                .map(|found| found.issue.clone())
                .collect::<Vec<_>>(),
        );
    }
    vec![Found {
        code: error.code.to_string(),
        path: error.path.clone(),
        message: error.message.clone(),
        details: error.details().cloned(),
    }]
}

fn two_transitions(first: Value, second: Value) -> Value {
    with_transitions(Edit::spec().bundle(), vec![tr(first), tr(second)])
}

/// Every rule a transition can break, in one list: the importer's issues first, then the ones only
/// an op raises.
fn scenarios() -> Vec<Found> {
    let mut all = Vec::new();
    // The public importer, which still refuses (policy Refuse).
    let public = tessel_editor_core::directed_scene::DirectedScene::import_value(doc(json!({})))
        .err()
        .unwrap();
    all.extend(from_issues(public.issues()));
    for over in [
        json!({"keys": {}}),
        json!({"effect": "vignette"}),
        json!({"effect": "wipe"}),
        json!({"params": {"grain": 1}}),
        json!({"params": {"intensity": "x"}}),
        json!({"params": {"intensity": 5}}),
        json!({"params": {"curve": "cubic"}}),
        json!({"easing": "bounce"}),
        json!({"alignment": "middle"}),
        json!({"handles": "keep"}),
        json!({"toShotId": "zz"}),
        json!({"fromShotId": "s1", "toShotId": "s3"}),
        json!({"fromShotId": "s3", "toShotId": "s1"}),
        json!({"durationFrames": 0}),
        json!({"durationFrames": 200}),
        json!({"durationFrames": 100}),
        json!({"durationFrames": 60, "alignment": "end"}),
        json!({"durationFrames": "x"}),
        json!({"id": "fx"}),
        json!({"id": 3}),
        json!({"hidden": false}),
    ] {
        all.extend(import(doc(over)));
    }
    let mut missing = doc(json!({}));
    missing["fx"]["transitions"][0]
        .as_object_mut()
        .unwrap()
        .remove("easing");
    all.extend(import(missing));
    all.extend(import(two_transitions(
        json!({"id": "a"}),
        json!({"id": "b"}),
    )));
    all.extend(import(two_transitions(
        json!({"id": "a"}),
        json!({"id": "a", "fromShotId": "s2", "toShotId": "s3"}),
    )));
    all.extend(import(two_transitions(
        json!({"id": "a", "alignment": "start", "durationFrames": 48}),
        json!({"id": "b", "fromShotId": "s2", "toShotId": "s3"}),
    )));
    all.extend(import(with_transitions(
        head(60, 48).bundle(),
        vec![tr(json!({"handles": "use"}))],
    )));
    // A scene with more transitions than the edit has cuts.
    let many = Edit::many_shots(3);
    let crowd: Vec<Value> = (0..3)
        .map(|i| {
            tr(
                json!({"id": format!("t{i}"), "fromShotId": "s0", "toShotId": "s1",
                      "durationFrames": 2}),
            )
        })
        .collect();
    all.extend(import(with_transitions(many.bundle(), crowd)));

    // A gesture inside the window of a transition (the predicate of `window_guard`).
    let guarded = spec_scene(vec![tr(json!({}))]);
    let bundle = guarded.bundle();
    let refusal = bundle
        .fx
        .overlay_refusal(
            &bundle.shots,
            bundle.edit_duration_frames,
            50,
            fx::OverlayTarget::Gizmo,
        )
        .unwrap();
    let english = refusal.problem(Locale::En);
    all.push(Found {
        code: refusal.code().to_string(),
        path: None,
        message: english.message.clone(),
        details: Some(json!({
            "transitionId": "tr", "hint": english.hint,
            "args": {"transitionId": "tr", "start": 42, "end": 54, "frame": 50},
        })),
    });

    // What only an op raises.
    let scene = spec_scene(vec![tr(json!({}))]);
    all.extend(through_edit(
        &scene,
        json!([{"op": "remove_transition", "id": "ghost"}]),
    ));
    all.extend(through_edit(
        &spec_scene(vec![]),
        json!([{"op": "remove_transition", "id": "ghost"}]),
    ));
    all.extend(through_edit(
        &scene,
        json!([{"op": "set_transition", "fromShotId": "s1", "toShotId": "s2",
                "effect": "crossDissolve", "id": "other"}]),
    ));
    for (effect, extra) in [
        ("vignette", json!({})),
        ("wipe", json!({})),
        ("crossDissolve", json!({"easing": "bounce"})),
        ("crossDissolve", json!({"params": {"grain": 1}})),
        ("crossDissolve", json!({"params": {"intensity": "x"}})),
        ("crossDissolve", json!({"params": {"intensity": 5}})),
        ("crossDissolve", json!({"toShotId": "zz"})),
    ] {
        let mut op = json!({"op": "set_transition", "fromShotId": "s1", "toShotId": "s2",
                            "effect": effect});
        for (key, value) in extra.as_object().unwrap() {
            op[key] = value.clone();
        }
        all.extend(through_edit(&spec_scene(vec![]), json!([op])));
    }
    all
}

/// The keys of the table that name a sentence of a problem (the smaller phrases are not).
fn sentence_keys() -> BTreeSet<String> {
    translated_keys()
        .into_iter()
        .filter(|key| !key.contains('.') && *key != "and" && *key != "refuse_note")
        .map(str::to_string)
        .collect()
}

/// Keys no valid scene can reach: a window of zero frames needs a shot of no frames, and the window\n/// arithmetic only overflows for a duration the edit's own length has already refused.
const DEFENSIVE: [&str; 2] = [
    "TRANSITION_TOO_LONG:no_room",
    "TRANSITION_OUT_OF_RANGE:overflow",
];

fn markers(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let close = after.find('}').expect("a marker closes");
        found.insert(after[..close].to_string());
        rest = &after[close + 1..];
    }
    found
}

#[test]
fn every_sentence_of_the_table_is_reached_by_a_rule() {
    let reached: BTreeSet<String> = scenarios().iter().map(Found::key).collect();
    for key in sentence_keys() {
        if DEFENSIVE.contains(&key.as_str()) {
            continue;
        }
        assert!(
            reached.contains(&key),
            "no scenario reaches `{key}`; reached: {reached:?}"
        );
    }
    // And the other way: nothing a rule says is outside the table.
    let known = sentence_keys();
    for key in &reached {
        assert!(
            known.contains(key),
            "`{key}` is said by a rule but is not in the table"
        );
    }
}

#[test]
fn a_problem_reads_in_three_languages_without_a_hole_and_english_is_the_importers_text() {
    let generic = [
        "INVALID_TYPE",
        "MISSING_FIELD",
        "INVALID_VALUE",
        "DUPLICATE_ID",
        "RESERVED_ID",
        "FX_LIMIT",
    ];
    for found in scenarios() {
        let projected: Vec<TransitionProblem> = Locale::ALL
            .iter()
            .map(|locale| project_problem(&found.raw(), *locale))
            .collect();
        for (locale, problem) in Locale::ALL.iter().zip(&projected) {
            let label = format!(
                "{} {locale:?}: {} <- {:?} {:?}",
                found.key(),
                problem.message,
                found.message,
                found.details
            );
            assert_eq!(problem.code, found.code);
            assert!(!problem.message.is_empty(), "{label}");
            assert!(!problem.message.contains(['{', '}']), "{label}");
            let hint = problem.hint.clone().unwrap_or_default();
            assert!(!hint.contains(['{', '}']), "{label}: {hint}");
            assert!(problem.details.get("untranslated").is_none(), "{label}");
            assert_eq!(problem.details["path"].as_str(), found.path.as_deref());
            assert_eq!(problem.to_json()["code"], found.code);
        }
        // English reads as the importer does (the generic structural sentences point at a path and
        // keep the importer's reason in the details instead).
        let english = &projected[1];
        let importer_hint = found
            .details
            .as_ref()
            .and_then(|details| details["hint"].as_str())
            .map(str::to_string);
        let has_args = found
            .details
            .as_ref()
            .is_some_and(|details| details.get("args").is_some());
        if generic.contains(&found.code.as_str()) && !has_args {
            assert_eq!(english.details["reason"], found.message, "{}", found.key());
        } else {
            assert_eq!(english.message, found.message, "{}", found.key());
            if importer_hint.is_some() {
                assert_eq!(english.hint, importer_hint, "{}", found.key());
            }
        }
        // The other languages are translated, not the English left in place.
        assert_ne!(
            projected[0].message,
            projected[1].message,
            "{}",
            found.key()
        );
        assert_ne!(
            projected[2].message,
            projected[1].message,
            "{}",
            found.key()
        );
        if english.hint.is_some() {
            assert!(
                projected[0].hint.is_some(),
                "{}: pt-br hint missing",
                found.key()
            );
            assert!(
                projected[2].hint.is_some(),
                "{}: es hint missing",
                found.key()
            );
            // A short imperative such as "Use linear" is valid in all three languages.
            // This full sentence must instead distinguish the localizations (R1 mutant).
            if found.code == fx::TRANSITION_ID_MISMATCH {
                assert_ne!(
                    projected[0].hint,
                    english.hint,
                    "{}: pt-br hint",
                    found.key()
                );
                assert_ne!(projected[2].hint, english.hint, "{}: es hint", found.key());
            }
        }
        // The transition and the shot are the same whichever the language.
        assert_eq!(projected[0].transition_id, projected[1].transition_id);
        assert_eq!(projected[2].shot_id, projected[1].shot_id);
    }
}

#[test]
fn the_three_templates_of_a_sentence_name_the_same_markers() {
    let all: Vec<&str> = translated_keys();
    assert!(
        all.len() > 30,
        "the table is walked, not skipped: {}",
        all.len()
    );
    for key in all {
        let (message, hint) = templates(key).unwrap();
        let [pt, en, es] = [message.pt_br, message.en, message.es].map(markers);
        assert_eq!(pt, en, "{key}: message markers pt-br vs en");
        assert_eq!(es, en, "{key}: message markers es vs en");
        if let Some(hint) = hint {
            let [pt, en, es] = [hint.pt_br, hint.en, hint.es].map(markers);
            assert_eq!(pt, en, "{key}: hint markers pt-br vs en");
            assert_eq!(es, en, "{key}: hint markers es vs en");
        }
        assert_ne!(message.pt_br, message.en, "{key}: pt-br is translated");
    }
}

#[test]
fn the_defensive_sentences_read_in_three_languages_too() {
    let no_room = json!({"args": {"kind": "no_room", "from": "s1", "to": "s2", "fromLength": 0,
                                  "toLength": 0, "alignment": "start"},
                         "transitionId": "tr-1"});
    let overflow = json!({"args": {"kind": "overflow", "duration": 9, "cut": 48},
                          "transitionId": "tr-1"});
    for (code, details) in [
        (fx::TRANSITION_TOO_LONG, &no_room),
        (fx::TRANSITION_OUT_OF_RANGE, &overflow),
    ] {
        for locale in Locale::ALL {
            let problem = project_problem(
                &RawProblem {
                    code,
                    path: None,
                    message: "",
                    details: Some(details),
                },
                locale,
            );
            assert!(
                problem.details.get("untranslated").is_none(),
                "{code} {locale:?}"
            );
            assert!(!problem.message.contains('{'), "{code} {locale:?}");
            assert!(problem.hint.is_some());
        }
    }
}

#[test]
fn a_code_outside_the_table_comes_back_in_the_original_text_and_says_so() {
    let details = json!({"hint": "the original hint", "transitionId": "t"});
    for locale in Locale::ALL {
        let problem = project_problem(
            &RawProblem {
                code: "SOMETHING_NEW",
                path: Some("/fx/transitions/0"),
                message: "the original text",
                details: Some(&details),
            },
            locale,
        );
        assert_eq!(problem.message, "the original text");
        assert_eq!(problem.hint.as_deref(), Some("the original hint"));
        assert_eq!(problem.details["untranslated"], true);
        assert_eq!(problem.transition_id.as_deref(), Some("t"));
    }
    // A known code without the numbers its sentence needs is untranslated, never half-filled.
    let problem = project_problem(
        &RawProblem {
            code: fx::UNKNOWN_SHOT,
            path: None,
            message: "original",
            details: None,
        },
        Locale::Es,
    );
    assert_eq!(problem.message, "original");
    assert_eq!(problem.details["untranslated"], true);
}

#[test]
fn the_projection_of_a_whole_refusal_keeps_the_order_of_the_importer() {
    let error = admit(two_transitions(
        json!({"id": "a", "durationFrames": 0}),
        json!({"id": "b", "fromShotId": "s2", "toShotId": "s3", "easing": "bounce"}),
    ))
    .unwrap_err();
    let problems = project_issues(error.issues(), Locale::PtBr);
    let originals: Vec<&str> = error.issues().iter().map(|issue| issue.code).collect();
    let projected: Vec<&str> = problems
        .iter()
        .map(|problem| problem.code.as_str())
        .collect();
    assert_eq!(projected, originals);
    assert_eq!(problems.len(), 2);
}

#[test]
fn an_edit_error_carries_the_problems_in_english_next_to_the_issues() {
    let scene = spec_scene(vec![tr(json!({}))]);
    // A candidate the importer refuses.
    let error = apply_fx(
        &scene,
        &parse_fx_batch(&json!([{"op": "resize_transition", "id": "tr", "durationFrames": 500}]))
            .unwrap(),
    )
    .unwrap_err();
    let errors = error.details().unwrap()["errors"].as_array().unwrap();
    assert_eq!(errors.len(), error.issues().len());
    assert_eq!(errors[0]["code"], error.issues()[0].issue.code);
    assert_eq!(errors[0]["transitionId"], "tr");
    assert_eq!(errors[0]["message"], error.issues()[0].issue.message);
    assert_eq!(errors[0]["details"]["ops"], json!([0]));
    // An op that failed by itself.
    let error = apply_fx(
        &scene,
        &parse_fx_batch(&json!([{"op": "remove_transition", "id": "ghost"}])).unwrap(),
    )
    .unwrap_err();
    let errors = error.details().unwrap()["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0]["code"], fx::UNKNOWN_TRANSITION);
    assert_eq!(errors[0]["message"], error.message);
    assert_eq!(errors[0]["hint"], error.details().unwrap()["hint"]);
    // A refusal of looks only is exactly what it was: no `errors`.
    let error = apply_fx(
        &scene,
        &parse_fx_batch(&json!([{"op": "remove_clip", "clip": "ghost"}])).unwrap(),
    )
    .unwrap_err();
    assert!(
        error
            .details()
            .is_none_or(|details| details.get("errors").is_none())
    );
    // The failing op is a look even though a successful transition op precedes it.
    let ops = json!([
        {"op":"set_transition_hidden","id":"tr","hidden":true},
        {"op":"remove_clip","clip":"ghost"}
    ]);
    let error = apply_fx(&scene, &parse_fx_batch(&ops).unwrap()).unwrap_err();
    assert_eq!(error.index, Some(1));
    assert!(
        error
            .details()
            .is_none_or(|details| details.get("errors").is_none())
    );
}
