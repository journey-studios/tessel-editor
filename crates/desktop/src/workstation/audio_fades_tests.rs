//! BL142 (F051): the fades of an audio clip in the real inspector, driven by real GPUI input.
//!
//! The Kit slider of "Entrada" (`fade_in`) is dragged with the window's pointer path, and the
//! semantic tree is read the way `./scripts/ui tree` reads it. The audio rules (samples, trim,
//! overlap) are tested without a window in `crates/core/tests/audio_fades.rs`.
use super::input_tests::fixture;
use super::*;
// The Kit's test macro expands to `#[test]`; the glob import above would resolve it to the Kit macro again.
use core::prelude::v1::test;

use crate::state::inspector::{InspectorTab, Value as PropertyValue, property_def};

fn run(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    command: Command,
) -> Result<Value, CommandError> {
    view.update(cx, |view, cx| view.execute(command, Source::Cli, cx))
}

fn select(view: &Entity<Workstation>, cx: &mut VisualTestContext, clip: &str, tab: InspectorTab) {
    run(
        view,
        cx,
        Command::SelectClip {
            clip: clip.to_string(),
        },
    )
    .unwrap();
    run(view, cx, Command::SetInspectorTab { tab }).unwrap();
    // Paint the new inspector so the geometry probes are the ones of this clip and tab.
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn fade_in(view: &Entity<Workstation>, cx: &mut VisualTestContext, clip: &str) -> f64 {
    view.read_with(cx, |view, _| {
        let property = property_def("fade_in").unwrap();
        match view.state.inspector.value(clip, property) {
            PropertyValue::Number(number) => number,
            other => panic!("fade_in is a number, got {other:?}"),
        }
    })
}

fn undo_depth(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> u64 {
    view.read_with(cx, |view, _| {
        view.state.history.to_json()["undo_depth"].as_u64().unwrap()
    })
}

fn has_control(view: &Entity<Workstation>, cx: &mut VisualTestContext, id: &str) -> bool {
    view.read_with(cx, |view, _| {
        view.geometry.get(&format!("control:{id}")).is_some()
    })
}

/// Whether the semantic tree lists `id` as visible in the live window.
fn is_visible(view: &Entity<Workstation>, cx: &mut VisualTestContext, id: &str) -> bool {
    view.read_with(cx, |view, cx| {
        view.tree_node(id, cx)
            .is_some_and(|node| node["visible"] == true)
    })
}

#[gpui_kit::test]
fn an_audio_clip_shows_its_fades_in_the_audio_tab_and_a_drag_is_one_undo_entry(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    select(&view, cx, "score-clip", InspectorTab::Audio);
    assert!(has_control(&view, cx, "fade_in"), "Entrada is on screen");
    assert!(has_control(&view, cx, "fade_out"), "Saída is on screen");
    assert!(
        has_control(&view, cx, "volume"),
        "the levels are still there"
    );
    assert!(is_visible(&view, cx, "property:fade_in"));
    assert!(is_visible(&view, cx, "inspector.fades"));
    assert!(
        !is_visible(&view, cx, "property:zoom"),
        "visual properties are not"
    );
    assert_eq!(fade_in(&view, cx, "score-clip"), 0.0);

    let bounds = view.read_with(cx, |view, _| view.geometry.get("control:fade_in").unwrap());
    let at = |fraction: f32| {
        point(
            bounds.left() + bounds.size.width * fraction,
            bounds.center().y,
        )
    };
    cx.simulate_mouse_move(at(0.5), None, Default::default());
    cx.simulate_mouse_down(at(0.5), MouseButton::Left, Default::default());
    for fraction in [0.55, 0.65, 0.75] {
        cx.simulate_mouse_move(at(fraction), MouseButton::Left, Default::default());
        cx.simulate_mouse_move(at(fraction), MouseButton::Left, Default::default());
    }
    cx.simulate_mouse_up(at(0.75), MouseButton::Left, Default::default());
    let value = fade_in(&view, cx, "score-clip");
    assert!(
        (10.0..=36.0).contains(&value),
        "the drag set {value} frames"
    );
    assert_eq!(undo_depth(&view, cx), 1, "one drag, one undo entry");

    run(&view, cx, Command::Undo).unwrap();
    assert_eq!(fade_in(&view, cx, "score-clip"), 0.0);
    run(&view, cx, Command::Redo).unwrap();
    assert_eq!(fade_in(&view, cx, "score-clip"), value);
}

#[gpui_kit::test]
fn the_semantic_tree_lists_each_fade_once_under_the_section_of_the_clip_kind(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    let parent_of = |view: &Entity<Workstation>, cx: &mut VisualTestContext, id: &str| {
        view.read_with(cx, |view, cx| {
            let nodes = view.tree_nodes(cx);
            let found: Vec<&Value> = nodes.iter().filter(|node| node["id"] == id).collect();
            assert_eq!(found.len(), 1, "{id} is listed exactly once");
            (
                found[0]["parent"].as_str().unwrap().to_string(),
                found[0]["state"]["value"].clone(),
                found[0]["visible"].as_bool().unwrap(),
            )
        })
    };
    select(&view, cx, "score-clip", InspectorTab::Audio);
    let (parent, value, visible) = parent_of(&view, cx, "property:fade_in");
    assert_eq!(parent, "inspector.fades");
    assert_eq!(value, json!(0.0));
    assert!(visible);

    select(&view, cx, "basalt-cliffs", InspectorTab::Video);
    let (parent, _, visible) = parent_of(&view, cx, "property:fade_in");
    assert_eq!(parent, "inspector.timing");
    assert!(visible, "the visual fades stay where they were");
    assert!(has_control(&view, cx, "fade_in"));

    // A video clip's Audio tab has its levels, never a fade of its sound.
    select(&view, cx, "basalt-cliffs", InspectorTab::Audio);
    assert!(!is_visible(&view, cx, "property:fade_in"));
    assert!(!is_visible(&view, cx, "inspector.fades"));
    assert!(is_visible(&view, cx, "property:volume"));
}
