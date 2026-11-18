//! F073: the keys section of the inspector as the live window serves it: the tree offers it for
//! the clips a key can animate, the buttons and `set` run the shared command handler, and the
//! plan follows the keys.
use serde_json::Value;
use tessel_editor_core::commands::Parsed;

use super::input_tests::fixture;
use super::*;
use core::prelude::v1::test;

fn node(view: &Entity<Workstation>, cx: &mut VisualTestContext, id: &str) -> Option<Value> {
    view.read_with(cx, |view, cx| view.tree_node(id, cx))
}

fn run(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    parsed: Result<Parsed, CommandError>,
) -> Result<Value, CommandError> {
    let Parsed::Run(command) = parsed? else {
        panic!("a clip key control is a state command");
    };
    let outcome = view.update(cx, |view, cx| view.execute(command, Source::Cli, cx));
    cx.run_until_parked();
    outcome
}

fn invoke(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    target: &str,
    action: &str,
) -> Result<Value, CommandError> {
    let parsed = view.read_with(cx, |view, cx| {
        let layout = view.layout_snapshot(cx);
        tessel_editor_core::commands::parse_invoke(&view.state, &layout, target, action)
    });
    run(view, cx, parsed)
}

fn set(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    target: &str,
    field: &str,
    value: Raw,
) -> Result<Value, CommandError> {
    run(
        view,
        cx,
        tessel_editor_core::commands::parse_set(target, field, &value),
    )
}

fn keys_of(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |view, _| {
        view.state
            .timeline
            .clip("first-light")
            .unwrap()
            .keys
            .iter()
            .map(|track| track.keys.len())
            .sum()
    })
}

fn scale_at(view: &Entity<Workstation>, cx: &mut VisualTestContext, frame: i64) -> f64 {
    view.read_with(cx, |view, _| {
        let clip = view.state.timeline.clip("first-light").unwrap();
        view.state
            .clip_keyed_value(clip, tessel_editor_core::group::Prop::Scale, frame)
    })
}

#[gpui_kit::test]
fn the_keys_section_lists_its_controls_for_the_selected_clip(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    invoke(&view, cx, "clip:first-light", "select").unwrap();
    let section = node(&view, cx, "inspector.keys").unwrap();
    assert_eq!(section["kind"], "accordion");
    assert_eq!(section["visible"], true);
    let capture = node(&view, cx, "clip_keys.capture:zoom").unwrap();
    assert_eq!(capture["parent"], "inspector.keys");
    assert_eq!(
        node(&view, cx, "clip_keys.clear:zoom").unwrap()["enabled"],
        false,
        "nothing to clear yet, and the node says why"
    );
    assert!(node(&view, cx, "clip_keys.clear:zoom").unwrap()["disabled_reason"].is_string());
    // The loop switch is an ordinary inspector property of the timing section.
    assert!(node(&view, cx, "property:loop").is_some());
}

#[gpui_kit::test]
fn capture_edit_and_remove_run_through_the_shared_handler(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    invoke(&view, cx, "clip:first-light", "select").unwrap();
    let (start, end) = view.read_with(cx, |view, _| {
        let clip = view.state.timeline.clip("first-light").unwrap();
        (clip.start, clip.end())
    });

    set(
        &view,
        cx,
        "timeline.playhead",
        "frame",
        Raw::Number((end + 50) as f64),
    )
    .unwrap();
    assert_eq!(
        node(&view, cx, "clip_keys.capture:zoom").unwrap()["enabled"],
        false,
        "the playhead is outside the clip"
    );
    assert!(invoke(&view, cx, "clip_keys.capture:zoom", "invoke").is_err());

    set(
        &view,
        cx,
        "timeline.playhead",
        "frame",
        Raw::Number(start as f64),
    )
    .unwrap();
    invoke(&view, cx, "clip_keys.capture:zoom", "invoke").unwrap();
    set(
        &view,
        cx,
        "timeline.playhead",
        "frame",
        Raw::Number((start + 24) as f64),
    )
    .unwrap();
    let before = view.read_with(cx, |view, _| view.state.project.revision);
    invoke(&view, cx, "clip_keys.capture:zoom", "invoke").unwrap();
    assert_eq!(keys_of(&view, cx), 2);
    assert!(view.read_with(cx, |view, _| view.state.project.revision) > before);

    let key = node(&view, cx, "clip_key:first-light:zoom:1").unwrap();
    assert_eq!(key["kind"], "clip-key");
    assert_eq!(key["state"]["property"], "zoom");
    assert_eq!(key["fields"].as_array().unwrap().len(), 3);

    set(
        &view,
        cx,
        "clip_key:first-light:zoom:0",
        "value",
        Raw::Number(100.0),
    )
    .unwrap();
    set(
        &view,
        cx,
        "clip_key:first-light:zoom:1",
        "value",
        Raw::Number(200.0),
    )
    .unwrap();
    set(
        &view,
        cx,
        "clip_key:first-light:zoom:1",
        "easing",
        Raw::Text("smoothStep".into()),
    )
    .unwrap();
    assert!(
        set(
            &view,
            cx,
            "clip_key:first-light:zoom:1",
            "value",
            Raw::Number(5000.0)
        )
        .is_err(),
        "out-of-range values are refused"
    );
    assert_eq!(scale_at(&view, cx, start), 100.0);
    assert_eq!(scale_at(&view, cx, start + 24), 200.0);

    // The undo entry is one edit; the keys go and come back with it.
    view.update(cx, |view, cx| view.dispatch(Command::Undo, cx));
    cx.run_until_parked();
    assert_eq!(
        keys_of(&view, cx),
        2,
        "the last edit was a field, not a key"
    );
    invoke(&view, cx, "clip_key:first-light:zoom:1", "remove").unwrap();
    invoke(&view, cx, "clip_keys.clear:zoom", "invoke").unwrap();
    assert_eq!(keys_of(&view, cx), 0);
    assert!(invoke(&view, cx, "clip_key:first-light:zoom:0", "remove").is_err());
}
