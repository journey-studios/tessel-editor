//! Transition diagnostics have the same order with and without serde_json/preserve_order.

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::fx::TransitionAdmission;
use tessel_editor_core::directed_scene::{DirectedScene, Limits};

use super::support::{Edit, admit, tr, with_transitions};

#[test]
fn parameter_diagnostics_are_lexical_in_value_and_text_imports() {
    for authored in [
        r#"{"zzz":1,"intensity":true,"curve":2}"#,
        r#"{"curve":2,"zzz":1,"intensity":true}"#,
        r#"{"intensity":true,"curve":2,"zzz":1}"#,
    ] {
        let params: Value = serde_json::from_str(authored).unwrap();
        let bundle = with_transitions(Edit::spec().bundle(), vec![tr(json!({"params": params}))]);
        let from_value = admit(bundle.clone()).err().expect("invalid parameters");
        let from_text = DirectedScene::import_str_admitting(
            &serde_json::to_string(&bundle).unwrap(),
            &Limits::default(),
            TransitionAdmission::Accept,
        )
        .err()
        .expect("invalid parameters");
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
