//! L (library): the document model of `fx.transitions` read through the hidden admitting importer
//! (T01, T02, T03doc, T05, T28doc). Public entries refuse (`admission.rs`); here transitions are
//! read, validated and kept.
//!
//! Every expectation is written by hand from the oracle (`dissolve-cases.json`) or from the
//! arithmetic of the window, never read back from the code under test.

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::fx::{
    self, Alignment, FxEasing, FxValue, Handles, TRANSITION_NO_HANDLES, TRANSITION_OUT_OF_RANGE,
};
use tessel_editor_core::directed_scene::{DirectedScene, ImportError};

use crate::support::*;

fn spec_with(list: Vec<Value>) -> Value {
    with_transitions(Edit::spec().bundle(), list)
}

fn issues_of(value: Value) -> Vec<(String, String)> {
    let error = admit(value).expect_err("the bundle should be refused");
    error
        .issues()
        .iter()
        .map(|issue| (issue.code.to_string(), issue.path.clone()))
        .collect()
}

#[test]
fn a_valid_transition_is_admitted_and_round_trips_byte_for_byte() {
    let transition = tr(json!({
        "params": {"curve": "linear", "intensity": 0.5},
        "seed": 4294967295u64, "hidden": true, "x-note": "keep me",
    }));
    let document = spec_with(vec![transition]);
    let scene = admit(document.clone()).unwrap();

    let typed = &scene.bundle().fx.transitions[0];
    assert_eq!(typed.id, "tr");
    assert_eq!(typed.effect.id, "crossDissolve");
    assert_eq!(typed.effect.family, "transition");
    assert_eq!(
        (typed.from_shot_id.as_str(), typed.to_shot_id.as_str()),
        ("s1", "s2")
    );
    assert_eq!(typed.duration_frames, 12);
    assert_eq!(typed.alignment, Alignment::Center);
    assert_eq!(typed.handles, Handles::Hold);
    assert_eq!(typed.easing, FxEasing::EaseInOutCubic);
    assert_eq!(
        typed.params,
        [FxValue::Choice("linear"), FxValue::Number(0.5)]
    );
    assert_eq!(typed.param("curve"), Some(FxValue::Choice("linear")));
    assert_eq!(typed.seed, Some(u32::MAX));
    assert!(typed.hidden);

    // The document is the canonical data: same value, same bytes, unknown field included.
    assert_eq!(scene.document(), &document);
    assert_eq!(scene.to_json_string(), document.to_string());
    assert!(
        scene
            .notices()
            .iter()
            .any(|notice| notice.code == "UNKNOWN_FIELD"
                && notice.path == "/fx/transitions/0/x-note")
    );

    // Idempotent: what comes out comes back identical, through the compact and the pretty text.
    for text in [scene.to_json_string(), scene.to_json_pretty()] {
        let again = admit_str(&text).unwrap();
        assert_eq!(again.document(), scene.document());
        assert_eq!(again.to_json_string(), scene.to_json_string());
        assert_eq!(again.bundle(), scene.bundle());
    }
}

#[test]
fn defaults_are_filled_in_but_never_written_to_the_document() {
    let document = spec_with(vec![tr(json!({}))]);
    let scene = admit(document.clone()).unwrap();
    let typed = &scene.bundle().fx.transitions[0];
    assert_eq!(
        typed.params,
        [FxValue::Choice("smooth"), FxValue::Number(1.0)]
    );
    assert_eq!(typed.seed, None);
    assert!(!typed.hidden);
    assert_eq!(scene.document(), &document, "asking never writes");
}

#[test]
fn a_scene_without_transitions_is_bit_identical_to_before() {
    for source in [MINIMAL, CUT] {
        let plain = DirectedScene::import_str(source).unwrap();
        let admitting = admit_str(source).unwrap();
        assert_eq!(plain.bundle(), admitting.bundle());
        assert_eq!(plain.bundle().fx, fx::Fx::default());
        assert!(plain.bundle().fx.transitions.is_empty());
        assert_eq!(
            plain.bundle().fx.to_json(),
            json!({"catalog_version": 1, "tracks": []})
        );
        assert!(plain.transition_warnings().is_empty());
    }
    // An empty list is the same layer as no list at all, for every reader.
    let mut layer = bundle_of(MINIMAL);
    layer["fx"] = json!({"catalogVersion": 1, "tracks": [], "transitions": []});
    let public = DirectedScene::import_value(layer.clone()).unwrap();
    assert_eq!(public.bundle().fx, fx::Fx::default());
    assert_eq!(admit(layer).unwrap().bundle(), public.bundle());
}

#[test]
fn the_reports_of_the_layer_never_mention_a_transition() {
    let with = admit(spec_with(vec![tr(json!({}))])).unwrap();
    let without = admit(Edit::spec().bundle()).unwrap();
    assert_eq!(with.bundle().fx.to_json(), without.bundle().fx.to_json());
}

#[test]
fn the_public_catalog_stays_the_three_looks() {
    let ids: Vec<&str> = fx::catalog().iter().map(|spec| spec.id).collect();
    assert_eq!(ids, ["vignette", "fade", "filmGrain"]);
    assert!(fx::effect("crossDissolve").is_none());
    assert_eq!(fx::catalog_json()["effects"].as_array().unwrap().len(), 3);
    assert_eq!(fx::catalog_json()["catalog_version"], 1);
    assert_eq!(fx::FX_CATALOG_VERSION, 1);
    // The transition table is separate, with its own family and a choice parameter.
    let table: Vec<&str> = fx::transition_catalog()
        .iter()
        .map(|spec| spec.id)
        .collect();
    assert_eq!(table, ["crossDissolve"]);
    let spec = fx::transition_effect("crossDissolve").unwrap();
    assert_eq!(spec.family, "transition");
    assert!(fx::transition_effect("vignette").is_none());
    assert_eq!(spec.params.len(), 2);
    assert_eq!(spec.to_json()["params"][0]["kind"], "choice");
    assert_eq!(
        spec.to_json()["params"][0]["options"],
        json!(["linear", "smooth"])
    );
    assert_eq!(spec.to_json()["params"][0]["default"], "smooth");
}

#[test]
fn integers_are_strict_where_the_document_says_integer() {
    // A float, a boolean, a string, null and an array are not an integer; nothing is coerced.
    for (value, code) in [
        (json!(12.0), "INVALID_VALUE"),
        (json!(12.5), "INVALID_VALUE"),
        (json!(1e1), "INVALID_VALUE"),
        (json!(true), "INVALID_TYPE"),
        (json!("12"), "INVALID_TYPE"),
        (json!(null), "INVALID_TYPE"),
        (json!([12]), "INVALID_TYPE"),
        (json!(9007199254740992u64), "INVALID_VALUE"),
        (json!(9223372036854775807i64), "INVALID_VALUE"),
        (json!(9223372036854775808u64), "INVALID_VALUE"),
    ] {
        let (found, path, _) = only_issue(spec_with(vec![tr(json!({"durationFrames": value}))]));
        assert_eq!(
            (found.as_str(), path.as_str()),
            (code, "/fx/transitions/0/durationFrames"),
            "{value}"
        );
    }
    // The same through the text a file holds: `12.0` written out is still a float.
    let text = spec_with(vec![tr(json!({"durationFrames": 12.0}))]).to_string();
    assert!(text.contains("\"durationFrames\":12.0"), "{text}");
    let error = admit_str(&text).err().unwrap();
    assert_eq!(codes(&error), ["INVALID_VALUE"]);
    // And a real integer is fine.
    assert!(admit(spec_with(vec![tr(json!({"durationFrames": 12}))])).is_ok());
}

#[test]
fn seeds_keep_their_exact_uint32_value_and_never_coerce() {
    for seed in [0u64, 1, 99, 2_147_483_648, 4_294_967_295] {
        let document = spec_with(vec![tr(json!({"seed": seed}))]);
        let scene = admit(document.clone()).unwrap();
        assert_eq!(
            scene.bundle().fx.transitions[0].seed,
            u32::try_from(seed).ok()
        );
        assert_eq!(scene.document(), &document, "{seed} kept as written");
    }
    for (seed, code) in [
        (json!(4294967296u64), "INVALID_VALUE"),
        (json!(-1), "INVALID_VALUE"),
        (json!(1.0), "INVALID_VALUE"),
        (json!(1.5), "INVALID_VALUE"),
        (json!(true), "INVALID_TYPE"),
        (json!("5"), "INVALID_TYPE"),
        (json!(null), "INVALID_TYPE"),
    ] {
        let (found, path, _) = only_issue(spec_with(vec![tr(json!({"seed": seed}))]));
        assert_eq!(
            (found.as_str(), path.as_str()),
            (code, "/fx/transitions/0/seed"),
            "{seed}"
        );
    }
}

#[test]
fn hidden_false_is_a_structural_refusal_for_transitions_and_for_looks_alike() {
    // Absent and `true` are the two writable spellings of a transition.
    assert!(
        !admit(spec_with(vec![tr(json!({}))]))
            .unwrap()
            .bundle()
            .fx
            .transitions[0]
            .hidden
    );
    assert!(
        admit(spec_with(vec![tr(json!({"hidden": true}))]))
            .unwrap()
            .bundle()
            .fx
            .transitions[0]
            .hidden
    );
    let (code, path, _) = only_issue(spec_with(vec![tr(json!({"hidden": false}))]));
    assert_eq!(
        (code.as_str(), path.as_str()),
        ("INVALID_VALUE", "/fx/transitions/0/hidden")
    );
    let (code, _, _) = only_issue(spec_with(vec![tr(json!({"hidden": "yes"}))]));
    assert_eq!(code, "INVALID_TYPE");

    // Looks keep the behavior they have today: `hidden: false` on a clip is refused the same way.
    let mut layered = Edit::spec().bundle();
    layered["fx"] = json!({"catalogVersion": 1, "transitions": [], "tracks": [{
        "id": "t1", "label": "Looks",
        "clips": [{"id": "v1", "effect": "vignette", "startFrame": 0, "durationFrames": 10, "hidden": false}],
    }]});
    let error = DirectedScene::import_value(layered).err().unwrap();
    assert_eq!(codes(&error), ["INVALID_VALUE"]);
    assert_eq!(error.issues()[0].path, "/fx/tracks/0/clips/0/hidden");
}

#[test]
fn keys_on_a_transition_are_refused_even_when_empty_and_looks_keep_theirs() {
    for keys in [
        json!({}),
        json!({"intensity": []}),
        json!({"intensity": [{"frame": 0, "value": 1}]}),
    ] {
        let (code, path, _) = only_issue(spec_with(vec![tr(json!({"keys": keys}))]));
        assert_eq!(
            (code.as_str(), path.as_str()),
            (fx::FX_TRANSITION_KEYS_NOT_YET, "/fx/transitions/0/keys"),
            "{keys}"
        );
    }
    // A look clip with an empty `keys` object is accepted as it always was.
    let mut layered = Edit::spec().bundle();
    layered["fx"] = json!({"catalogVersion": 1, "transitions": [], "tracks": [{
        "id": "t1", "label": "Looks",
        "clips": [{"id": "v1", "effect": "vignette", "startFrame": 0, "durationFrames": 10, "keys": {}}],
    }]});
    assert!(DirectedScene::import_value(layered).is_ok());
}

#[test]
fn a_newer_catalog_version_is_refused_with_transitions_too() {
    let mut document = spec_with(vec![tr(json!({}))]);
    document["fx"]["catalogVersion"] = json!(2);
    assert_eq!(
        issues_of(document),
        [(
            "CATALOG_TOO_NEW".to_string(),
            "/fx/catalogVersion".to_string()
        )]
    );
}

#[test]
fn ids_share_one_space_with_tracks_and_clips_and_keep_the_reserved_words() {
    // Against a track, a clip, and another transition.
    let track = json!({"id": "taken-track", "label": "T", "clips": [
        {"id": "taken-clip", "effect": "vignette", "startFrame": 0, "durationFrames": 5}]});
    for taken in ["taken-track", "taken-clip"] {
        let mut document = spec_with(vec![tr(json!({"id": taken}))]);
        document["fx"]["tracks"] = json!([track]);
        assert_eq!(
            issues_of(document),
            [(
                "DUPLICATE_ID".to_string(),
                "/fx/transitions/0/id".to_string()
            )],
            "{taken}"
        );
    }
    let twice = spec_with(vec![
        tr(json!({"id": "a"})),
        tr(json!({"id": "a", "fromShotId": "s2", "toShotId": "s3"})),
    ]);
    assert_eq!(
        issues_of(twice),
        [(
            "DUPLICATE_ID".to_string(),
            "/fx/transitions/1/id".to_string()
        )]
    );
    // Case-sensitive for duplicates (the pilot's rule), case-insensitive for the reserved words.
    assert!(
        admit(spec_with(vec![
            tr(json!({"id": "A"})),
            tr(json!({"id": "a", "fromShotId": "s2", "toShotId": "s3", "durationFrames": 4}))
        ]))
        .is_ok()
    );
    for reserved in [
        "track",
        "TRACK",
        "Fx",
        "transition",
        "Transition",
        "fx",
        "TRANSITION",
    ] {
        let (code, path, _) = only_issue(spec_with(vec![tr(json!({"id": reserved}))]));
        assert_eq!(
            (code.as_str(), path.as_str()),
            ("RESERVED_ID", "/fx/transitions/0/id"),
            "{reserved}"
        );
    }
    let long = "x".repeat(65);
    for bad in [
        "",
        "-x",
        "_x",
        "tr 1",
        "tr\n",
        "tr.1",
        "tr/1",
        "tr-é",
        long.as_str(),
    ] {
        let (code, _, _) = only_issue(spec_with(vec![tr(json!({"id": bad}))]));
        assert_eq!(code, "INVALID_VALUE", "{bad:?}");
    }
    for good in ["tr-1", "A", "a_b-9", "0x", "TR-UPPER", &"x".repeat(64)] {
        assert!(
            admit(spec_with(vec![tr(json!({"id": good}))])).is_ok(),
            "{good}"
        );
    }
}

#[test]
fn the_count_is_judged_before_any_entry_is_read() {
    // Three entries for two cuts, all of them malformed: only the limit is reported, so none was read.
    let hostile = vec![json!({}), json!({"id": 7}), json!("nope")];
    let (code, path, message) = only_issue(spec_with(hostile));
    assert_eq!(
        (code.as_str(), path.as_str()),
        (fx::FX_LIMIT, "/fx/transitions")
    );
    assert!(
        message.contains("3 transitions") && message.contains("2 cut"),
        "{message}"
    );
    // One cut, one transition: the limit holds exactly. One shot has no cut at all.
    let single = Edit::many_shots(1).bundle();
    let (code, path, _) = only_issue(with_transitions(single.clone(), vec![tr(json!({}))]));
    assert_eq!(
        (code.as_str(), path.as_str()),
        (fx::FX_LIMIT, "/fx/transitions")
    );
    assert!(admit(with_transitions(single, vec![])).is_ok());
    assert!(
        admit(spec_with(vec![
            tr(json!({})),
            tr(json!({"id": "b", "fromShotId": "s2", "toShotId": "s3", "durationFrames": 4}))
        ]))
        .is_ok()
    );
}

#[test]
fn the_oracle_limit_rows_hold_for_every_shot_count() {
    for shots in [2u64, 5, 64] {
        let cuts = shots - 1;
        let edit = Edit::many_shots(shots);
        let list = |count: u64| -> Vec<Value> {
            (0..count)
                .map(|i| {
                    tr(json!({"id": format!("t{i}"), "fromShotId": format!("s{i}"),
                              "toShotId": format!("s{}", i + 1), "durationFrames": 1}))
                })
                .collect()
        };
        assert!(
            admit(with_transitions(edit.bundle(), list(cuts))).is_ok(),
            "{shots} shots, {cuts}"
        );
        let mut over = list(cuts);
        over.push(tr(json!({"id": "extra"})));
        let (code, _, _) = only_issue(with_transitions(edit.bundle(), over));
        assert_eq!(code, fx::FX_LIMIT, "{shots} shots, {}", cuts + 1);
    }
}

#[test]
fn a_family_mix_up_is_not_an_unknown_effect() {
    // A look where a transition goes.
    let issues = admit(spec_with(vec![tr(json!({"effect": "vignette"}))]))
        .err()
        .unwrap();
    assert_eq!(codes(&issues), [fx::WRONG_FAMILY]);
    let issues = admit(spec_with(vec![tr(json!({"effect": "nope"}))]))
        .err()
        .unwrap();
    assert_eq!(codes(&issues), ["UNKNOWN_EFFECT"]);
    // A transition where a look goes.
    let mut document = Edit::spec().bundle();
    document["fx"] = json!({"catalogVersion": 1, "transitions": [], "tracks": [{
        "id": "t1", "label": "Looks",
        "clips": [{"id": "v1", "effect": "crossDissolve", "startFrame": 0, "durationFrames": 10}],
    }]});
    let error = admit(document.clone()).err().unwrap();
    assert_eq!(codes(&error), [fx::WRONG_FAMILY]);
    assert_eq!(error.issues()[0].path, "/fx/tracks/0/clips/0/effect");
    // Public readers never grew the family: the same document is the unknown effect it was.
    let error = DirectedScene::import_value(document).err().unwrap();
    assert_eq!(codes(&error), ["UNKNOWN_EFFECT"]);
}

#[test]
fn params_are_typed_and_ranged_with_their_own_codes_and_paths() {
    for (params, code, name) in [
        (json!({"speed": 1}), "UNKNOWN_PARAM", "speed"),
        (json!({"curve": 1}), "PARAM_TYPE", "curve"),
        (json!({"intensity": true}), "PARAM_TYPE", "intensity"),
        (json!({"intensity": "1"}), "PARAM_TYPE", "intensity"),
        (json!({"curve": "wavy"}), "PARAM_OUT_OF_RANGE", "curve"),
        (json!({"curve": "Smooth"}), "PARAM_OUT_OF_RANGE", "curve"),
        (json!({"intensity": 1.5}), "PARAM_OUT_OF_RANGE", "intensity"),
        (
            json!({"intensity": -0.25}),
            "PARAM_OUT_OF_RANGE",
            "intensity",
        ),
    ] {
        let error = admit(spec_with(vec![tr(json!({"params": params}))]))
            .err()
            .unwrap();
        assert_eq!(codes(&error), [code], "{params}");
        assert_eq!(
            error.issues()[0].path,
            format!("/fx/transitions/0/params/{name}")
        );
    }
    // `params` itself must be an object.
    let (code, path, _) = only_issue(spec_with(vec![tr(json!({"params": [1]}))]));
    assert_eq!(
        (code.as_str(), path.as_str()),
        ("INVALID_TYPE", "/fx/transitions/0/params")
    );
    // Both edges of the range are inside.
    assert!(
        admit(spec_with(vec![tr(
            json!({"params": {"intensity": 0, "curve": "linear"}})
        )]))
        .is_ok()
    );
    assert!(admit(spec_with(vec![tr(json!({"params": {"intensity": 1}}))])).is_ok());
}

#[test]
fn easing_and_enums_use_the_existing_vocabulary() {
    let error = admit(spec_with(vec![tr(json!({"easing": "bounce"}))]))
        .err()
        .unwrap();
    assert_eq!(codes(&error), ["UNKNOWN_KEYFRAME_EASING"]);
    assert_eq!(error.issues()[0].path, "/fx/transitions/0/easing");
    for easing in ["linear", "easeInOutCubic", "easeOutCubic"] {
        assert!(
            admit(spec_with(vec![tr(json!({"easing": easing}))])).is_ok(),
            "{easing}"
        );
    }
    let error = admit(spec_with(vec![tr(json!({"alignment": "middle"}))]))
        .err()
        .unwrap();
    assert_eq!(codes(&error), ["INVALID_VALUE"]);
    let error = admit(spec_with(vec![tr(json!({"handles": "freeze"}))]))
        .err()
        .unwrap();
    assert_eq!(codes(&error), ["INVALID_VALUE"]);
    // Missing required fields are structural and each is named.
    for field in [
        "id",
        "effect",
        "fromShotId",
        "toShotId",
        "durationFrames",
        "alignment",
        "handles",
        "easing",
    ] {
        let mut transition = tr(json!({}));
        transition.as_object_mut().unwrap().remove(field);
        let error = admit(spec_with(vec![transition])).err().unwrap();
        assert_eq!(codes(&error), ["MISSING_FIELD"], "{field}");
        assert!(error.issues()[0].message.contains(field), "{field}");
    }
}

#[test]
fn the_window_and_the_cut_decide_what_a_transition_may_ask() {
    // Valid: centre 12 / 5 / 1, start 12, end 12 on the cut at frame 48.
    for (duration, alignment) in [
        (12, "center"),
        (5, "center"),
        (1, "center"),
        (12, "start"),
        (12, "end"),
    ] {
        let document = spec_with(vec![tr(
            json!({"durationFrames": duration, "alignment": alignment}),
        )]);
        assert!(admit(document).is_ok(), "{duration} {alignment}");
    }
    // The edges 0 and `edit` are valid: centre 96 at cut 48 is [0, 96); at cut 96 it is [48, 144).
    assert!(admit(spec_with(vec![tr(json!({"durationFrames": 96}))])).is_ok());
    assert!(
        admit(spec_with(vec![tr(
            json!({"durationFrames": 96, "fromShotId": "s2", "toShotId": "s3"})
        )]))
        .is_ok()
    );
    // One frame beyond what two shots hold: at cut 48 the window [0, 97) is still inside the edit,
    // so only the shots object; at cut 96 it is [48, 145) and leaves the edit as well.
    let codes_of =
        |duration: i64| admit_codes(spec_with(vec![tr(json!({"durationFrames": duration}))]));
    assert_eq!(codes_of(97), [fx::TRANSITION_TOO_LONG]);
    let later = tr(json!({"durationFrames": 97, "fromShotId": "s2", "toShotId": "s3"}));
    assert_eq!(
        admit_codes(spec_with(vec![later])),
        [TRANSITION_OUT_OF_RANGE, fx::TRANSITION_TOO_LONG]
    );
    // A duration below one or above the edit is refused before any window is computed: only the
    // range code, and on the field that is wrong.
    for duration in [0, -3, 145, 9_007_199_254_740_991i64] {
        let (code, path, _) = only_issue(spec_with(vec![tr(json!({"durationFrames": duration}))]));
        assert_eq!(
            (code.as_str(), path.as_str()),
            (TRANSITION_OUT_OF_RANGE, "/fx/transitions/0/durationFrames"),
            "{duration}"
        );
    }
    assert_eq!(
        codes_of(144),
        [TRANSITION_OUT_OF_RANGE, fx::TRANSITION_TOO_LONG]
    );
}

#[test]
fn the_order_of_refusals_is_the_contract() {
    // Family, easing, then the shot: each transition reports in the order of the plan.
    let all = tr(json!({"effect": "vignette", "easing": "bounce", "fromShotId": "ghost"}));
    assert_eq!(
        admit_codes(spec_with(vec![all])),
        [fx::WRONG_FAMILY, "UNKNOWN_KEYFRAME_EASING", "UNKNOWN_SHOT"]
    );
    // Easing before adjacency.
    let skipping = tr(json!({"easing": "bounce", "toShotId": "s3"}));
    assert_eq!(
        admit_codes(spec_with(vec![skipping])),
        ["UNKNOWN_KEYFRAME_EASING", fx::TRANSITION_NOT_AT_CUT]
    );
    // Both shots missing: one issue each, from before to.
    let ghosts = tr(json!({"fromShotId": "ghost", "toShotId": "phantom"}));
    let error = admit(spec_with(vec![ghosts])).err().unwrap();
    let paths: Vec<&str> = error
        .issues()
        .iter()
        .map(|issue| issue.path.as_str())
        .collect();
    assert_eq!(
        paths,
        ["/fx/transitions/0/fromShotId", "/fx/transitions/0/toShotId"]
    );
    // A duplicate on a cut is followed by the overlap it causes.
    let pair = vec![tr(json!({"id": "a"})), tr(json!({"id": "b"}))];
    assert_eq!(
        admit_codes(spec_with(pair)),
        [fx::TRANSITION_DUPLICATE, fx::TRANSITION_OVERLAP]
    );
    // Structure comes first of all: a bad type hides everything the bundle would say.
    let broken = tr(json!({"durationFrames": "12", "effect": "nope", "toShotId": "ghost"}));
    assert_eq!(admit_codes(spec_with(vec![broken])), ["INVALID_TYPE"]);
}

#[test]
fn a_transition_must_sit_on_a_cut() {
    for (from, to) in [("s1", "s3"), ("s2", "s1"), ("s3", "s1"), ("s1", "s1")] {
        let codes = admit_codes(spec_with(vec![tr(
            json!({"fromShotId": from, "toShotId": to}),
        )]));
        assert_eq!(codes, [fx::TRANSITION_NOT_AT_CUT], "{from} -> {to}");
    }
    let error = admit(spec_with(vec![tr(
        json!({"fromShotId": "s3", "toShotId": "s1"}),
    )]))
    .err()
    .unwrap();
    let hint = error.issues()[0].details.as_ref().unwrap()["hint"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(hint.contains("this is the last shot"), "{hint}");
    let error = admit(spec_with(vec![tr(json!({"toShotId": "s3"}))]))
        .err()
        .unwrap();
    let hint = error.issues()[0].details.as_ref().unwrap()["hint"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(hint.contains("after it comes s2"), "{hint}");
}

#[test]
fn windows_that_touch_do_not_overlap_and_windows_that_share_a_frame_do() {
    let second = |extra: Value| {
        let mut base = json!({"id": "b", "fromShotId": "s2", "toShotId": "s3"});
        for (key, value) in extra.as_object().unwrap() {
            base[key] = value.clone();
        }
        tr(base)
    };
    // `a` is [48, 96) (start, 48 frames); `b` starts exactly at 96: touching.
    let a = tr(json!({"id": "a", "alignment": "start", "durationFrames": 48}));
    assert!(
        admit(spec_with(vec![
            a.clone(),
            second(json!({"alignment": "start", "durationFrames": 12}))
        ]))
        .is_ok()
    );
    // `b` centred 12 is [90, 102): it shares frames 90..96 with `a`.
    let codes = admit_codes(spec_with(vec![
        a.clone(),
        second(json!({"durationFrames": 12})),
    ]));
    assert_eq!(codes, [fx::TRANSITION_OVERLAP]);
    // `b` ending 12 is [84, 96) and `a` [48, 96): overlap, reported on the later window.
    let error = admit(spec_with(vec![
        a,
        second(json!({"alignment": "end", "durationFrames": 12})),
    ]))
    .err()
    .unwrap();
    assert_eq!(codes_of_error(&error), [fx::TRANSITION_OVERLAP]);
    // Document order does not matter: the windows are ordered by start.
    let reversed = vec![
        second(json!({"durationFrames": 12})),
        tr(json!({"id": "a", "alignment": "start", "durationFrames": 48})),
    ];
    assert_eq!(admit_codes(spec_with(reversed)), [fx::TRANSITION_OVERLAP]);
}

fn codes_of_error(error: &ImportError) -> Vec<&'static str> {
    codes(error)
}

#[test]
fn handles_use_and_refuse_need_real_frames_and_hold_never_does() {
    let spec = || Edit::spec().camera("cam1", 96);
    // `hold` freezes: the same window that lacks frames is fine.
    let hold = with_transitions(spec().bundle(), vec![tr(json!({"handles": "hold"}))]);
    assert!(admit(hold).is_ok());
    // Both the instance and the camera of the incoming shot lack the 6 frames before it.
    let both =
        |handles: &str| with_transitions(spec().bundle(), vec![tr(json!({"handles": handles}))]);
    let lacks = |error: &ImportError| error.issues()[0].details.as_ref().unwrap()["lacks"].clone();
    let error = admit(both("use")).err().unwrap();
    assert_eq!(codes(&error), [TRANSITION_NO_HANDLES]);
    assert_eq!(
        lacks(&error),
        json!([{"shot": "s2", "side": "before its start", "instance": 6, "camera": 6}])
    );
    // `refuse` asks for the same frames, and says so in its hint.
    let error = admit(both("refuse")).err().unwrap();
    assert_eq!(codes(&error), [TRANSITION_NO_HANDLES]);
    let hint = error.issues()[0].details.as_ref().unwrap()["hint"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        hint.ends_with("only \"hold\" accepts frozen ones."),
        "{hint}"
    );
    assert!(error.issues()[0].message.contains("(handles: refuse)"));

    // Only the camera lacks: the instance has its own head.
    let camera_only = spec().shot("s2", |shot| shot.instance_start = 10);
    let error = admit(with_transitions(
        camera_only.bundle(),
        vec![tr(json!({"handles": "use"}))],
    ))
    .err()
    .unwrap();
    assert_eq!(
        lacks(&error),
        json!([{"shot": "s2", "side": "before its start", "instance": 0, "camera": 6}])
    );
    // Only the instance lacks: the camera starts 10 frames into its clip.
    let instance_only = spec().shot("s2", |shot| shot.camera_in = 10);
    let error = admit(with_transitions(
        instance_only.bundle(),
        vec![tr(json!({"handles": "use"}))],
    ))
    .err()
    .unwrap();
    assert_eq!(
        lacks(&error),
        json!([{"shot": "s2", "side": "before its start", "instance": 6, "camera": 0}])
    );
    // Exactly the frames needed is enough; one short is refused with the one frame it lacks.
    let exact = spec().shot("s2", |shot| {
        shot.instance_start = 6;
        shot.camera_in = 6;
    });
    assert!(
        admit(with_transitions(
            exact.bundle(),
            vec![tr(json!({"handles": "use"}))]
        ))
        .is_ok()
    );
    let short = spec().shot("s2", |shot| {
        shot.instance_start = 6;
        shot.camera_in = 5;
    });
    let error = admit(with_transitions(
        short.bundle(),
        vec![tr(json!({"handles": "use"}))],
    ))
    .err()
    .unwrap();
    assert_eq!(
        lacks(&error),
        json!([{"shot": "s2", "side": "before its start", "instance": 0, "camera": 1}])
    );
    // The outgoing shot is judged on its tail: its camera ends with the shot (48 of 48).
    let outgoing = Edit::spec().shot("s2", |shot| {
        shot.instance_start = 6;
        shot.camera_in = 6;
    });
    let error = admit(with_transitions(
        outgoing.bundle(),
        vec![tr(json!({"handles": "use"}))],
    ))
    .err()
    .unwrap();
    assert_eq!(
        lacks(&error),
        json!([{"shot": "s1", "side": "after its end", "instance": 0, "camera": 6}])
    );
}

#[test]
fn an_odd_window_puts_its_extra_frame_after_the_cut() {
    // Centre 5 at cut 48 is [46, 51): 2 frames before the cut, 3 from it (floor, not ceiling).
    // The two halves are what each shot is asked for, so the lacks name them.
    let edit = Edit::spec().camera("cam1", 96);
    let bundle = with_transitions(
        edit.bundle(),
        vec![tr(json!({"durationFrames": 5, "handles": "use"}))],
    );
    let error = admit(bundle).err().unwrap();
    let lacks = &error.issues()[0].details.as_ref().unwrap()["lacks"];
    assert_eq!(
        lacks,
        &json!([{"shot": "s2", "side": "before its start", "instance": 2, "camera": 2}]),
        "2 frames before the cut"
    );
    // The outgoing side asks for the 3 frames after the cut when its tail is short.
    let tail = Edit::spec().shot("s2", |shot| {
        shot.instance_start = 6;
        shot.camera_in = 6;
    });
    let bundle = with_transitions(
        tail.bundle(),
        vec![tr(json!({"durationFrames": 5, "handles": "use"}))],
    );
    let error = admit(bundle).err().unwrap();
    let lacks = &error.issues()[0].details.as_ref().unwrap()["lacks"];
    assert_eq!(
        lacks,
        &json!([{"shot": "s1", "side": "after its end", "instance": 0, "camera": 3}]),
        "3 frames from the cut"
    );
}

#[test]
fn too_long_is_judged_inside_the_two_shots() {
    // 100 centred at cut 48 asks 50 + 50 of two shots of 48.
    let error = admit(spec_with(vec![tr(json!({"durationFrames": 100}))]))
        .err()
        .unwrap();
    assert_eq!(
        codes(&error),
        [TRANSITION_OUT_OF_RANGE, fx::TRANSITION_TOO_LONG]
    );
    let issue = &error.issues()[1];
    assert_eq!(
        issue.message,
        "Transition tr needs 50 frames inside s1 (48) and 50 inside s2 (48)"
    );
    assert_eq!(
        issue.details.as_ref().unwrap()["hint"],
        "With alignment center the longest these two shots hold is 96 frames."
    );
    // Within the shots but 1 frame too long only on one side: start 49 into a 48-frame shot.
    let codes = admit_codes(spec_with(vec![tr(
        json!({"alignment": "start", "durationFrames": 49}),
    )]));
    assert_eq!(codes, [fx::TRANSITION_TOO_LONG]);
    assert!(
        admit(spec_with(vec![tr(
            json!({"alignment": "start", "durationFrames": 48})
        )]))
        .is_ok()
    );
    assert!(
        admit(spec_with(vec![tr(
            json!({"alignment": "end", "durationFrames": 48})
        )]))
        .is_ok()
    );
    // `end` 49 is [-1, 48): outside the edit and longer than the outgoing shot.
    let codes = admit_codes(spec_with(vec![tr(
        json!({"alignment": "end", "durationFrames": 49}),
    )]));
    assert_eq!(codes, [TRANSITION_OUT_OF_RANGE, fx::TRANSITION_TOO_LONG]);
}

/// `s1` `[0, 48)`, `s2` `[48, 58)` (only 10 frames), `s3` `[58, 96)`: the shot between the two
/// cuts is the short one, so each cut has a long side and a short side on opposite ends.
fn short_middle() -> Edit {
    let mut edit = Edit::spec()
        .shot("s2", |shot| shot.end = 58)
        .shot("s3", |shot| {
            shot.start = 58;
            shot.end = 96;
        });
    edit.duration = 96;
    edit
}

#[test]
fn too_long_is_judged_per_side_so_swapping_before_and_after_is_caught() {
    const FITS: &[&str] = &[];
    const LONG: &[&str] = &[fx::TRANSITION_TOO_LONG];
    // The cut `s1 → s2` is at 48 (long shot before, 10 frames after); `s2 → s3` is at 58 (10
    // frames before, long shot after). By hand: `start` asks `D` of the shot after the cut,
    // `end` asks `D` of the shot before it, `center` asks ⌊D/2⌋ before and ⌈D/2⌉ after.
    let cases: [(&str, &str, &str, u64, &[&str]); 15] = [
        // short shot AFTER the cut
        ("s1", "s2", "start", 10, FITS),
        ("s1", "s2", "start", 11, LONG),
        ("s1", "s2", "start", 20, LONG),
        ("s1", "s2", "end", 20, FITS), // 20 before the cut, 0 after: fine
        ("s1", "s2", "center", 19, FITS), // 9 before, 10 after
        ("s1", "s2", "center", 20, FITS), // 10 before, 10 after
        ("s1", "s2", "center", 21, LONG), // 10 before, 11 after > 10
        // short shot BEFORE the cut
        ("s2", "s3", "end", 10, FITS),
        ("s2", "s3", "end", 11, LONG),
        ("s2", "s3", "end", 12, LONG),
        ("s2", "s3", "start", 20, FITS), // 0 before the cut, 20 after: fine
        ("s2", "s3", "start", 38, FITS),
        ("s2", "s3", "center", 21, FITS), // 10 before, 11 after
        ("s2", "s3", "center", 22, LONG), // 11 before > 10, 11 after
        ("s2", "s3", "center", 1, FITS),  // 0 before, 1 after
    ];
    for (from, to, alignment, duration, expected) in cases {
        let bundle = with_transitions(
            short_middle().bundle(),
            vec![tr(json!({"fromShotId": from, "toShotId": to,
                           "alignment": alignment, "durationFrames": duration}))],
        );
        assert_eq!(
            admit_codes(bundle),
            expected,
            "{from} → {to}, {alignment}, {duration} frames"
        );
    }

    // The numbers named in the refusal are the two halves, in the right shots.
    let one = |from: &str, to: &str, alignment: &str, duration: u64| {
        let bundle = with_transitions(
            short_middle().bundle(),
            vec![tr(json!({"fromShotId": from, "toShotId": to,
                           "alignment": alignment, "durationFrames": duration}))],
        );
        let error = admit(bundle).expect_err("too long");
        let issue = &error.issues()[0];
        (
            issue.message.clone(),
            issue.details.as_ref().unwrap()["hint"]
                .as_str()
                .unwrap()
                .to_string(),
        )
    };
    assert_eq!(
        one("s1", "s2", "start", 20),
        (
            "Transition tr needs 0 frames inside s1 (48) and 20 inside s2 (10)".to_string(),
            "With alignment start the longest these two shots hold is 10 frames.".to_string()
        )
    );
    assert_eq!(
        one("s2", "s3", "end", 11),
        (
            "Transition tr needs 11 frames inside s2 (10) and 0 inside s3 (38)".to_string(),
            "With alignment end the longest these two shots hold is 10 frames.".to_string()
        )
    );
    assert_eq!(
        one("s1", "s2", "center", 21),
        (
            "Transition tr needs 10 frames inside s1 (48) and 11 inside s2 (10)".to_string(),
            "With alignment center the longest these two shots hold is 20 frames.".to_string()
        )
    );
    assert_eq!(
        one("s2", "s3", "center", 22),
        (
            "Transition tr needs 11 frames inside s2 (10) and 11 inside s3 (38)".to_string(),
            "With alignment center the longest these two shots hold is 21 frames.".to_string()
        )
    );
}

#[test]
fn windows_are_judged_in_start_order_whatever_the_document_order() {
    // Four shots of 4 frames: cuts at 4, 8 and 12. `a`, `b` and `c` sit on them in that order.
    let edit = Edit::many_shots(4);
    let on_cut = |id: &str, from: u32, alignment: &str, duration: u64| {
        tr(
            json!({"id": id, "fromShotId": format!("s{from}"), "toShotId": format!("s{}", from + 1),
                  "alignment": alignment, "durationFrames": duration, "handles": "hold"}),
        )
    };
    let list = |entries: [&Value; 3], order: [usize; 3]| -> Vec<Value> {
        order.iter().map(|&index| entries[index].clone()).collect()
    };
    let orders = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];

    // Disjoint windows `[4, 6)`, `[8, 10)`, `[12, 14)`, and ones that only touch: `[4, 8)`,
    // `[8, 10)`. No order of the document is an overlap, and the document order is kept.
    for first in [2, 4] {
        let a = on_cut("a", 0, "start", first);
        let b = on_cut("b", 1, "start", 2);
        let c = on_cut("c", 2, "start", 2);
        for order in orders {
            let ids = ["a", "b", "c"];
            let bundle = with_transitions(edit.bundle(), list([&a, &b, &c], order));
            let scene = admit(bundle).unwrap_or_else(|error| {
                panic!("order {order:?}, a lasts {first}: {:?}", codes(&error))
            });
            let kept: Vec<&str> = scene
                .bundle()
                .fx
                .transitions
                .iter()
                .map(|transition| transition.id.as_str())
                .collect();
            let wanted: Vec<&str> = order.iter().map(|&index| ids[index]).collect();
            assert_eq!(kept, wanted, "the document order is the order kept");
        }
    }

    // `a` is `[4, 7)` and `b` (centred, 4) is `[6, 10)`: they share frame 6, in every order of
    // the document and with `c` anywhere between. One refusal, on the window that starts later,
    // at that window's own place in the document.
    let a = on_cut("a", 0, "start", 3);
    let b = on_cut("b", 1, "center", 4);
    let c = on_cut("c", 2, "start", 2);
    for order in orders {
        let error = admit(with_transitions(edit.bundle(), list([&a, &b, &c], order)))
            .err()
            .unwrap_or_else(|| panic!("order {order:?} must overlap"));
        assert_eq!(codes(&error), [fx::TRANSITION_OVERLAP], "order {order:?}");
        let issue = &error.issues()[0];
        let place = order.iter().position(|&index| index == 1).unwrap();
        assert_eq!(
            issue.path,
            format!("/fx/transitions/{place}"),
            "reported on `b`, which starts later: order {order:?}"
        );
        assert_eq!(
            issue.message, "Transitions a (frames 4 to 7) and b (from frame 6) overlap",
            "order {order:?}"
        );
    }
}

#[test]
fn a_legacy_dissolve_is_still_refused_by_the_importer_before_any_transition() {
    // The oracle's `spec+legacy-dissolve` case reports TRANSITION_DUPLICATE; this importer refuses
    // the legacy dissolve itself, earlier and for everyone (DISSOLVE_UNSUPPORTED).
    let mut bundle = Edit::spec().bundle();
    bundle["shots"][1]["transitionInto"] = json!({"type": "dissolve", "durationFrames": 12});
    let error = admit(with_transitions(bundle, vec![tr(json!({}))]))
        .err()
        .unwrap();
    assert!(
        codes(&error).contains(&"DISSOLVE_UNSUPPORTED"),
        "{:?}",
        error.issues()
    );
    assert!(!codes(&error).contains(&fx::TRANSITION_DUPLICATE));
}

#[test]
fn warnings_never_refuse_and_name_the_transition() {
    // A continuous junction: the transition is admitted, with a warning.
    let mut continuous = Edit::spec().bundle();
    continuous["shots"][1]["transitionInto"] = json!({"type": "continuous"});
    let scene = admit(with_transitions(continuous.clone(), vec![tr(json!({}))])).unwrap();
    let warnings = scene.transition_warnings();
    assert_eq!(warnings.len(), 1);
    assert_eq!(
        (warnings[0].code, warnings[0].path.as_str()),
        (fx::TRANSITION_ON_CONTINUOUS, "/fx/transitions/0")
    );
    // A hidden transition draws nothing and warns of nothing; a cut does not warn.
    let hidden = admit(with_transitions(
        continuous,
        vec![tr(json!({"hidden": true}))],
    ))
    .unwrap();
    assert!(hidden.transition_warnings().is_empty());
    assert!(
        admit(spec_with(vec![tr(json!({}))]))
            .unwrap()
            .transition_warnings()
            .is_empty()
    );
}

#[test]
fn two_splat_scenes_warn_of_the_cost_and_one_does_not() {
    let mut both = Edit::spec().bundle();
    add_splats(&mut both, &["scene-a", "scene-b"]);
    let scene = admit(with_transitions(both, vec![tr(json!({}))])).unwrap();
    let warnings = scene.transition_warnings();
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].code, fx::TRANSITION_SPLAT_COST);
    assert_eq!(warnings[0].code, "TRANSITION_SPLAT_COST");

    // Only the outgoing side has a splat: no cost warning.
    let mut one = Edit::spec().bundle();
    add_splats(&mut one, &["scene-a"]);
    let scene = admit(with_transitions(one, vec![tr(json!({}))])).unwrap();
    assert!(scene.transition_warnings().is_empty());
}
