//! F064 / P4b S1a: the window still refuses `fx.transitions` and still offers three looks.
//!
//! A bundle with a transition reaches the application through the shared handler
//! (`Command::Scene(Attach)`, the one the CLI and the dialogs use); the core refuses it with
//! `FX_TRANSITION_NOT_YET` and the window keeps its scenes, its revision and its history. The scene
//! inspector's "Efeitos" section keeps its three `add:` actions and shows no transition effect. No
//! dialog, control or node is added by this delivery (T32, negative case).
//!
//! The rules themselves are the core's: `crates/core/tests/scene_fx_transition.rs`.
use super::scene_fx_tests::{mounted, node, total_depth};
use super::*;
// The Kit's test macro expands to `#[test]`; the glob import above would resolve it to the Kit macro again.
use core::prelude::v1::test;
use tessel_editor_core::scene_project::{SceneCommand, SceneSource};

const BUNDLE: &str = include_str!(
    "../../../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json"
);

fn with_transition() -> String {
    let mut value: Value = serde_json::from_str(BUNDLE).unwrap();
    value["fx"] = json!({"catalogVersion": 1, "tracks": [], "transitions": [{
        "id": "tr", "effect": "crossDissolve", "fromShotId": "shot-1", "toShotId": "shot-2",
        "durationFrames": 12, "alignment": "center", "handles": "hold",
        "easing": "easeInOutCubic"}]});
    value.to_string()
}

fn attach(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    id: &str,
    text: String,
) -> Result<Value, CommandError> {
    let id = id.to_string();
    let result = view.update(cx, |view, cx| {
        view.execute(
            Command::Scene(SceneCommand::Attach {
                id: Some(id),
                name: None,
                source: SceneSource::Text(text),
            }),
            Source::Cli,
            cx,
        )
    });
    cx.run_until_parked();
    result
}

#[gpui_kit::test]
fn a_bundle_with_a_transition_is_refused_and_the_window_offers_no_transition(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mounted(cx);
    let depth = total_depth(&view, cx);
    let revision = view.read_with(cx, |view, _| view.state.project.revision);
    let before = node(&view, cx, "scene.fx");

    let error = attach(&view, cx, "other", with_transition()).unwrap_err();
    assert_eq!(error.code, "invalid_bundle", "{error:?}");
    let issues = error.details.as_ref().unwrap()["issues"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(
        issues[0]["code"],
        json!("FX_TRANSITION_NOT_YET"),
        "{issues:?}"
    );
    assert_eq!(issues[0]["path"], json!("/fx/transitions"));

    // Nothing was written: same scenes, same revision, same history.
    view.read_with(cx, |view, _| {
        assert_eq!(view.state.scenes.ids(), ["street"]);
        assert_eq!(view.state.project.revision, revision);
    });
    assert_eq!(total_depth(&view, cx), depth);

    // The section is what it was: three `add:` actions, the looks, and no transition anywhere.
    let section = node(&view, cx, "scene.fx");
    assert_eq!(section, before);
    let adds: Vec<&str> = section["actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|action| action["action"].as_str())
        .filter(|name| name.starts_with("add:"))
        .collect();
    assert_eq!(adds, ["add:vignette", "add:fade", "add:filmGrain"]);
    assert!(!section.to_string().contains("crossDissolve"), "{section}");
    assert!(
        !section.to_string().to_lowercase().contains("transition"),
        "{section}"
    );
}

#[gpui_kit::test]
fn a_transition_effect_cannot_be_added_as_a_look_from_the_window(cx: &mut TestAppContext) {
    let (view, cx) = mounted(cx);
    let depth = total_depth(&view, cx);
    let result = view.update(cx, |view, cx| {
        view.scene_invoke("scene.fx", Some("add:crossDissolve"), cx)
    });
    cx.run_until_parked();
    // Either there is no such action (None) or it refuses; it never writes.
    if let Some(result) = result {
        assert!(result.is_err(), "{result:?}");
    }
    assert_eq!(total_depth(&view, cx), depth);
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.state
                .scenes
                .get("street")
                .unwrap()
                .scene
                .bundle()
                .fx
                .clip_count(),
            0
        );
    });
}
