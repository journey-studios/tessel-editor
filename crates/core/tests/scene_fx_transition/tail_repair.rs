//! BL082.4.25 (S2-C0a): the repair the diagnostic gives for the OUTGOING camera is enough. A
//! camera that already ends before its shot needs the frames that reach the shot as well as the
//! `after` frames of the window; the old number (`after` alone) left the clip short and the
//! transition still refused. The repair is the exact `cameraExtend`; `camera` keeps its meaning.

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::Issue;
use tessel_editor_core::directed_scene::fx::{Locale, RawProblem, project_problem};

use crate::support::*;

/// `s1` runs 48 frames from `cameraIn 40` of `cam1` (`cam1_frames` long); a centred 12-frame
/// window asks it for 6 frames after its end. `s2` has all it needs before its start.
fn tail(cam1_frames: u64) -> Edit {
    Edit::spec()
        .camera("cam1", cam1_frames)
        .shot("s1", |shot| shot.camera_in = 40)
        .shot("s2", |shot| {
            shot.instance_start = 6;
            shot.camera_in = 6;
        })
}

fn transition(handles: &str) -> Value {
    tr(json!({"handles": handles}))
}

fn refused(edit: &Edit, handles: &str) -> Issue {
    let error = admit(with_transitions(edit.bundle(), vec![transition(handles)]))
        .expect_err("the outgoing tail should lack frames");
    assert_eq!(
        codes(&error),
        ["TRANSITION_NO_HANDLES"],
        "{:?}",
        error.issues()
    );
    error.issues()[0].clone()
}

fn accepted(edit: &Edit) -> bool {
    admit(with_transitions(edit.bundle(), vec![transition("use")])).is_ok()
}

fn details(issue: &Issue) -> &Value {
    issue.details.as_ref().unwrap()
}

fn hint(issue: &Issue) -> String {
    details(issue)["hint"].as_str().unwrap().to_string()
}

#[test]
fn a_camera_that_ends_before_its_shot_asks_for_the_frames_that_reach_it_too() {
    // 60 frames from `cameraIn 40`: the clip ends 28 frames before the shot does, so 6 frames
    // after the shot need 34 more, not 6. `camera` is still the 6 the window asks for.
    let issue = refused(&tail(60), "use");
    assert_eq!(
        details(&issue)["lacks"],
        json!([{"shot": "s1", "side": "after its end", "instance": 0, "camera": 6,
                "cameraExtend": 34}])
    );
    let text = hint(&issue);
    assert!(
        text.contains("extend its camera clip cam1 by 34 frames after its end"),
        "{text}"
    );
    assert!(!text.contains("by 6 frames"), "{text}");
    // The old advice (6 more: 66) is still refused, one frame short of the new one (93) too, and
    // the number now given (94) is enough.
    assert!(!accepted(&tail(66)));
    assert!(!accepted(&tail(93)));
    assert!(accepted(&tail(94)));
    // `refuse` reads the same frames as `use` and says the same number, with its note.
    let refuse = refused(&tail(60), "refuse");
    assert_eq!(details(&refuse)["lacks"], details(&issue)["lacks"]);
    assert!(hint(&refuse).contains("by 34 frames after its end"));
    assert!(hint(&refuse).contains("\"refuse\" asks for the same real frames"));
}

#[test]
fn the_field_exists_only_where_the_repair_is_not_the_camera_the_lack_always_had() {
    // The clip reaches the shot (88 frames) with 2 to spare: 4 of the 6 after it are missing, the
    // same 4 to add. The lack is the bytes it always was.
    let issue = refused(&tail(90), "use");
    assert_eq!(
        details(&issue)["lacks"],
        json!([{"shot": "s1", "side": "after its end", "instance": 0, "camera": 4}])
    );
    assert!(hint(&issue).contains("by 4 frames after its end"));
    assert!(accepted(&tail(94)) && !accepted(&tail(93)));
    // Exactly at the shot's end the same holds: the whole window is missing.
    let issue = refused(&tail(88), "use");
    assert_eq!(details(&issue)["lacks"][0]["camera"], 6);
    assert!(details(&issue)["lacks"][0].get("cameraExtend").is_none());
    // The start side never carries it (its repair is `camera`, or `cameraEnd` for the end).
    let both = Edit::spec()
        .camera("cam1", 60)
        .shot("s1", |shot| shot.camera_in = 40)
        .shot("s2", |shot| shot.camera_in = 2);
    let issue = refused(&both, "use");
    let lacks = details(&issue)["lacks"].as_array().unwrap().clone();
    assert_eq!(lacks.len(), 2, "{lacks:?}");
    assert_eq!(lacks[0]["side"], "after its end");
    assert_eq!(lacks[0]["cameraExtend"], 34);
    assert_eq!(lacks[1]["side"], "before its start");
    assert!(lacks[1].get("cameraExtend").is_none(), "{:?}", lacks[1]);
    assert!(lacks[1]["camera"].as_i64().unwrap() > 0);
}

#[test]
fn the_projection_gives_the_exact_repair_in_the_three_languages() {
    let issue = refused(&tail(60), "use");
    assert_eq!(details(&issue)["args"]["lacks"][0]["cameraExtend"], 34);
    let expected = [
        (Locale::PtBr, "em 34 quadros depois do fim"),
        (Locale::En, "by 34 frames after its end"),
        (Locale::Es, "en 34 fotogramas después de su final"),
    ];
    for (locale, sentence) in expected {
        let result = project_problem(&RawProblem::from_issue(&issue), locale);
        assert!(result.details.get("untranslated").is_none(), "{locale:?}");
        let hint = result.hint.unwrap();
        assert!(hint.contains(sentence), "{locale:?}: {hint}");
        assert!(!hint.contains(['{', '}']), "{locale:?}: {hint}");
        // The message still says what the window asks (6), never the repair.
        assert!(
            result.message.contains('6'),
            "{locale:?}: {}",
            result.message
        );
        assert!(
            !result.message.contains("34"),
            "{locale:?}: {}",
            result.message
        );
    }
}

#[test]
fn an_absent_repair_falls_back_to_the_camera_and_an_invalid_one_keeps_the_original_text() {
    let issue = refused(&tail(60), "use");
    let with = |change: &dyn Fn(&mut serde_json::Map<String, Value>)| {
        let mut issue = issue.clone();
        let entry = issue.details.as_mut().unwrap()["args"]["lacks"][0]
            .as_object_mut()
            .unwrap();
        change(entry);
        issue
    };
    // Absent: a problem from an older producer asks for `camera`, translated.
    let older = with(&|entry| {
        entry.remove("cameraExtend");
    });
    let result = project_problem(&RawProblem::from_issue(&older), Locale::En);
    assert!(result.details.get("untranslated").is_none());
    assert!(result.hint.unwrap().contains("by 6 frames after its end"));
    // Present and not a whole non-negative integer: the importer's own words, said so.
    for value in [json!("34"), json!(-1), json!(1.5), Value::Null, json!(34.0)] {
        let broken = with(&|entry| {
            entry.insert("cameraExtend".to_string(), value.clone());
        });
        for locale in Locale::ALL {
            let result = project_problem(&RawProblem::from_issue(&broken), locale);
            assert_eq!(result.message, issue.message, "{value} in {locale:?}");
            assert_eq!(
                result.hint.as_deref(),
                Some(hint(&issue).as_str()),
                "{value} in {locale:?}"
            );
            assert_eq!(
                result.details["untranslated"], true,
                "{value} in {locale:?}"
            );
        }
    }
}
