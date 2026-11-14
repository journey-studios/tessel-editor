//! Transition diagnostics have the same order with and without serde_json/preserve_order.
//!
//! Only the profile that preserves insertion order can tell the sorted reader from an unsorted
//! one: without it every `Value` object is already sorted and the test passes either way. The
//! isolated core (`cargo test -p tessel-editor-core`) is that blind profile; the workspace build
//! and `--features serde_json/preserve_order` are the discriminating ones. Set
//! `TESSEL_EXPECT_PRESERVE_ORDER=1` (or `0`) and the first test fails when the profile is not the
//! one the run claims, so a log cannot present the blind profile as proof.

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::fx::TransitionAdmission;
use tessel_editor_core::directed_scene::{DirectedScene, Limits};

use super::support::{Edit, admit, tr, with_transitions};

/// Whether this build of `serde_json` keeps the authored key order.
fn preserves_insertion_order() -> bool {
    let value: Value = serde_json::from_str(r#"{"b":1,"a":2}"#).unwrap();
    value.as_object().unwrap().keys().next().unwrap() == "b"
}

#[test]
fn the_profile_under_test_is_the_one_the_run_claims() {
    let Ok(claimed) = std::env::var("TESSEL_EXPECT_PRESERVE_ORDER") else {
        return;
    };
    assert_eq!(
        preserves_insertion_order(),
        claimed == "1",
        "TESSEL_EXPECT_PRESERVE_ORDER={claimed}"
    );
}

#[test]
fn parameter_diagnostics_are_lexical_in_value_and_text_imports() {
    for authored in [
        r#"{"zzz":1,"intensity":true,"curve":2}"#,
        r#"{"curve":2,"zzz":1,"intensity":true}"#,
        r#"{"intensity":true,"curve":2,"zzz":1}"#,
    ] {
        let params: Value = serde_json::from_str(authored).unwrap();
        let bundle = with_transitions(Edit::spec().bundle(), vec![tr(json!({"params": params}))]);
        let from_value = admit(bundle.clone()).expect_err("invalid parameters");
        let from_text = DirectedScene::import_str_admitting(
            &serde_json::to_string(&bundle).unwrap(),
            &Limits::default(),
            TransitionAdmission::Accept,
        )
        .expect_err("invalid parameters");
        for error in [from_value, from_text] {
            let issues: Vec<_> = error
                .issues()
                .iter()
                .map(|issue| (issue.code, issue.path.as_str()))
                .collect();
            assert_eq!(
                issues,
                [
                    ("PARAM_TYPE", "/fx/transitions/0/params/curve"),
                    ("PARAM_TYPE", "/fx/transitions/0/params/intensity"),
                    ("UNKNOWN_PARAM", "/fx/transitions/0/params/zzz"),
                ],
                "authored parameters: {authored}"
            );
        }
    }
}
