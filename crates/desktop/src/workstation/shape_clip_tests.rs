//! F067: the shape clip as the live window serves it. The timeline button inserts one, the
//! inspector inputs edit it through the same commands as `set clip_shape:<id> ...`, a refused value
//! puts the model's back and every edit is one undo entry.
use serde_json::Value;
use tessel_editor_core::commands::Parsed;

use super::input_tests::fixture;
use super::*;
use crate::state::timeline::ClipKind;
use core::prelude::v1::test;

fn repaint(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn node(view: &Entity<Workstation>, cx: &mut VisualTestContext, id: &str) -> Option<Value> {
    view.read_with(cx, |view, cx| view.tree_node(id, cx))
}

fn revision(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> u64 {
    view.read_with(cx, |view, _| view.state.project.revision)
}

/// Clicks the toolbar button and returns the id of the clip it selected.
fn add_shape(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> String {
    let at = view.read_with(cx, |view, _| {
        view.geometry
            .get("control:timeline-add-shape")
            .expect("the shape button is laid out in the timeline toolbar")
            .center()
    });
    cx.simulate_click(at, Default::default());
    repaint(cx);
    view.read_with(cx, |view, _| {
        let clip = view
            .state
            .inspector_clip()
            .expect("the new shape is selected");
        assert_eq!(clip.kind, ClipKind::Shape);
        clip.id.to_string()
    })
}

fn model_field(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    id: &str,
    field: &str,
) -> String {
    node(view, cx, &format!("clip_shape:{id}")).expect("the shape has a node")["state"][field]
        .to_string()
}

fn shown(view: &Entity<Workstation>, cx: &mut VisualTestContext, field: &str) -> String {
    view.read_with(cx, |view, cx| {
        view.shape_inputs
            .input_of(field)
            .read(cx)
            .value()
            .to_string()
    })
}

/// What a person does: types into one field, then presses Enter.
fn type_and_confirm(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    field: &str,
    text: &str,
) {
    view.update_in(cx, |view, window, cx| {
        let input = view.shape_inputs.input_of(field);
        input.update(cx, |input, cx| {
            input.set_value(text.to_string(), window, cx)
        });
        input.update(cx, |_, cx| {
            cx.emit(InputEvent::PressEnter {
                secondary: false,
                shift: false,
            });
        });
    });
    repaint(cx);
}

#[gpui_kit::test]
fn the_timeline_button_inserts_one_shape_and_undo_removes_clip_and_track(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let before = view.read_with(cx, |view, _| {
        (
            view.state.timeline.clips.len(),
            view.state.timeline.tracks.len(),
        )
    });
    let id = add_shape(&view, cx);
    let playhead = view.read_with(cx, |view, _| view.state.timeline.playhead);
    view.read_with(cx, |view, _| {
        let clip = view.state.timeline.clip(&id).unwrap();
        assert_eq!(clip.start, playhead);
        assert_eq!(clip.shape.as_ref().unwrap().width, 40.0);
    });
    assert_eq!(model_field(&view, cx, &id, "kind"), "\"rect\"");
    assert_eq!(
        node(&view, cx, "inspector.shape").unwrap()["visible"],
        true,
        "the shape section follows the selection"
    );

    view.update(cx, |view, cx| view.execute(Command::Undo, Source::Cli, cx))
        .unwrap();
    repaint(cx);
    let after = view.read_with(cx, |view, _| {
        (
            view.state.timeline.clips.len(),
            view.state.timeline.tracks.len(),
        )
    });
    assert_eq!(after, before, "one gesture, one undo entry");
}

#[gpui_kit::test]
fn the_inputs_show_the_shape_and_enter_commits_one_undoable_edit(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let id = add_shape(&view, cx);
    assert_eq!(shown(&view, cx, "width"), "40");
    assert_eq!(shown(&view, cx, "fill"), "#4a90e2");

    let start = revision(&view, cx);
    type_and_confirm(&view, cx, "width", "55.5");
    assert_eq!(model_field(&view, cx, &id, "width"), "55.5");
    assert_eq!(shown(&view, cx, "width"), "55.5");
    assert_eq!(revision(&view, cx), start + 1, "one commit, one revision");

    // The same value again is not an edit.
    type_and_confirm(&view, cx, "width", "55.5");
    assert_eq!(revision(&view, cx), start + 1);

    type_and_confirm(&view, cx, "fill", "#112233");
    type_and_confirm(&view, cx, "radius", "30");
    assert_eq!(model_field(&view, cx, &id, "radius"), "30.0");

    view.update(cx, |view, cx| view.execute(Command::Undo, Source::Cli, cx))
        .unwrap();
    repaint(cx);
    assert_eq!(shown(&view, cx, "radius"), "0", "undo reaches the input");
    assert_eq!(shown(&view, cx, "fill"), "#112233");
}

#[gpui_kit::test]
fn a_refused_value_changes_nothing_and_the_input_shows_the_model_again(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let id = add_shape(&view, cx);
    let start = revision(&view, cx);

    type_and_confirm(&view, cx, "fill", "red");
    assert_eq!(model_field(&view, cx, &id, "fill"), "\"#4a90e2\"");
    assert_eq!(shown(&view, cx, "fill"), "#4a90e2");
    type_and_confirm(&view, cx, "width", "500");
    assert_eq!(shown(&view, cx, "width"), "40");
    assert_eq!(revision(&view, cx), start);
    let (error, text) = view.read_with(cx, |view, _| {
        let activity = view.activity.as_ref().unwrap();
        (activity.error, activity.text.clone())
    });
    assert!(error, "the refusal is reported: {text}");
}

#[gpui_kit::test]
fn the_semantic_set_and_the_input_run_the_same_command(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let id = add_shape(&view, cx);
    let Parsed::Run(command) = tessel_editor_core::commands::parse_set(
        &format!("clip_shape:{id}"),
        "height",
        &Raw::Number(12.0),
    )
    .unwrap() else {
        panic!("a shape field is a state command");
    };
    view.update(cx, |view, cx| view.execute(command, Source::Cli, cx))
        .unwrap();
    repaint(cx);
    assert_eq!(
        shown(&view, cx, "height"),
        "12",
        "a script's edit reaches the input"
    );
}

#[gpui_kit::test]
fn a_delayed_input_event_cannot_write_into_a_different_selected_shape(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let id = add_shape(&view, cx);
    view.update(cx, |view, _| view.shape_inputs.forget());
    type_and_confirm(&view, cx, "width", "60");
    assert_eq!(
        model_field(&view, cx, &id, "width"),
        "40.0",
        "inputs not synced to this clip never edit it"
    );
}

/// Every field of the spec has its input in the section, showing the model's value.
#[gpui_kit::test]
fn every_shape_field_has_an_input_that_shows_the_model(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let id = add_shape(&view, cx);
    for field in tessel_editor_core::shape::ShapeSpec::FIELDS {
        let model = node(&view, cx, &format!("clip_shape:{id}")).unwrap()["state"][field].clone();
        let expected = match &model {
            Value::String(text) => text.clone(),
            number => number
                .as_f64()
                .map_or_else(|| number.to_string(), |value| value.to_string()),
        };
        assert_eq!(shown(&view, cx, field), expected, "{field}");
        assert!(
            view.read_with(cx, |view, _| view
                .geometry
                .get(&format!("control:shape-{field}"))
                .is_some()),
            "{field} is laid out in the inspector"
        );
    }
}

/// An ellipse with a radial fill, a contour and a shadow, typed field by field: each commit is its
/// own undo entry and undo walks them back to the rectangle the button inserted.
#[gpui_kit::test]
fn the_style_fields_edit_the_shape_one_undo_entry_each(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let id = add_shape(&view, cx);
    let start = revision(&view, cx);
    let edits = [
        ("kind", "ellipse", "\"ellipse\""),
        ("fill_mode", "radial", "\"radial\""),
        ("fill_to", "#00000000", "\"#00000000\""),
        ("gradient_angle", "90", "90.0"),
        ("stroke_width", "4", "4.0"),
        ("stroke", "#AABBCC", "\"#aabbcc\""),
        ("shadow_blur", "6", "6.0"),
        ("shadow_y", "3", "3.0"),
    ];
    for (field, text, model) in edits {
        type_and_confirm(&view, cx, field, text);
        assert_eq!(model_field(&view, cx, &id, field), model, "{field}");
    }
    assert_eq!(
        revision(&view, cx),
        start + edits.len() as u64,
        "one commit, one revision"
    );

    // The colour is stored normalised and the input shows what the model holds.
    assert_eq!(shown(&view, cx, "stroke"), "#aabbcc");

    // A path is canonicalised: the input shows the canonical spelling.
    type_and_confirm(&view, cx, "kind", "path");
    type_and_confirm(&view, cx, "path", "m0,0 L100,0  50 100z");
    assert_eq!(
        model_field(&view, cx, &id, "path"),
        "\"M 50 0 L 100 100 L 0 100 Z\"",
        "a relative or malformed path is refused and the default stays"
    );

    type_and_confirm(&view, cx, "path", "M 0 0 L 100 0 L 50 100 Z");
    assert_eq!(shown(&view, cx, "path"), "M 0 0 L 100 0 L 50 100 Z");

    for _ in 0..(edits.len() + 2) {
        view.update(cx, |view, cx| view.execute(Command::Undo, Source::Cli, cx))
            .unwrap();
    }
    repaint(cx);
    assert_eq!(shown(&view, cx, "kind"), "rect");
    assert_eq!(shown(&view, cx, "fill_mode"), "solid");
    assert_eq!(shown(&view, cx, "stroke_width"), "0");
}

/// A value the core refuses puts the model's back in the input, for the new fields too.
#[gpui_kit::test]
fn a_refused_style_value_leaves_the_model_and_shows_it_again(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let id = add_shape(&view, cx);
    let start = revision(&view, cx);
    for (field, text) in [
        ("kind", "star"),
        ("fill_mode", "conic"),
        ("stroke", "white"),
        ("stroke_width", "50"),
        ("shadow_x", "-31"),
        ("gradient_angle", "400"),
        ("path", "m 0 0 l 5 5"),
    ] {
        let before = shown(&view, cx, field);
        type_and_confirm(&view, cx, field, text);
        assert_eq!(shown(&view, cx, field), before, "{field} {text}");
    }
    assert_eq!(revision(&view, cx), start);
    assert_eq!(model_field(&view, cx, &id, "kind"), "\"rect\"");
}

/// The semantic `set` of a style field and the input run the same command.
#[gpui_kit::test]
fn the_semantic_set_of_a_style_field_reaches_its_input(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let id = add_shape(&view, cx);
    let Parsed::Run(command) = tessel_editor_core::commands::parse_set(
        &format!("clip_shape:{id}"),
        "fill_mode",
        &Raw::Text("linear".into()),
    )
    .unwrap() else {
        panic!("a shape field is a state command");
    };
    view.update(cx, |view, cx| view.execute(command, Source::Cli, cx))
        .unwrap();
    repaint(cx);
    assert_eq!(shown(&view, cx, "fill_mode"), "linear");
}

/// The rounded mask of an image is a section of the inspector of image and video clips, set with
/// the same property command as every other, and absent from a shape.
#[gpui_kit::test]
fn the_mask_section_belongs_to_image_clips_and_one_edit_is_one_undo_entry(cx: &mut TestAppContext) {
    use crate::state::inspector::{InspectorTab, Value as PropertyValue, property_def};
    let (view, cx) = fixture(cx);
    let run = |view: &Entity<Workstation>, cx: &mut VisualTestContext, command: Command| {
        view.update(cx, |view, cx| view.execute(command, Source::Cli, cx))
    };
    run(
        &view,
        cx,
        Command::SelectClip {
            clip: "basalt-cliffs".into(),
        },
    )
    .unwrap();
    run(
        &view,
        cx,
        Command::SetInspectorTab {
            tab: InspectorTab::Video,
        },
    )
    .unwrap();
    repaint(cx);
    let visible = |view: &Entity<Workstation>, cx: &mut VisualTestContext, id: &str| {
        node(view, cx, id).is_some_and(|node| node["visible"] == true)
    };
    assert!(
        visible(&view, cx, "inspector.mask"),
        "a video clip has the mask section"
    );

    run(
        &view,
        cx,
        Command::SetSectionExpanded {
            section: "mask".into(),
            expanded: true,
        },
    )
    .unwrap();
    repaint(cx);
    assert!(visible(&view, cx, "property:mask_shape"));
    assert!(visible(&view, cx, "property:mask_radius"));

    let depth = |view: &Entity<Workstation>, cx: &mut VisualTestContext| {
        view.read_with(cx, |view, _| {
            view.state.history.to_json()["undo_depth"].as_u64().unwrap()
        })
    };
    let before = depth(&view, cx);
    run(
        &view,
        cx,
        Command::SetProperty {
            property: "mask_shape".into(),
            value: Raw::Text("rounded".into()),
        },
    )
    .unwrap();
    assert_eq!(depth(&view, cx), before + 1);
    let mask = |view: &Entity<Workstation>, cx: &mut VisualTestContext| {
        view.read_with(cx, |view, _| {
            match view
                .state
                .inspector
                .value("basalt-cliffs", property_def("mask_shape").unwrap())
            {
                PropertyValue::Choice(text) => text,
                other => panic!("mask_shape is a choice, got {other:?}"),
            }
        })
    };
    assert_eq!(mask(&view, cx), "rounded");
    run(&view, cx, Command::Undo).unwrap();
    assert_eq!(mask(&view, cx), "none");

    // A shape has no picture to cut.
    add_shape(&view, cx);
    assert!(!visible(&view, cx, "inspector.mask"));
}

/// F072: the reveal is a section of the common inspector (no panel of its own), on video and on
/// shape clips alike, edited through the same property command and undone one entry at a time.
#[gpui_kit::test]
fn the_reveal_section_is_the_common_inspector_on_a_video_and_on_a_shape(cx: &mut TestAppContext) {
    use crate::state::inspector::{InspectorTab, Value as PropertyValue, property_def};
    let (view, cx) = fixture(cx);
    let run = |view: &Entity<Workstation>, cx: &mut VisualTestContext, command: Command| {
        view.update(cx, |view, cx| view.execute(command, Source::Cli, cx))
    };
    run(
        &view,
        cx,
        Command::SelectClip {
            clip: "basalt-cliffs".into(),
        },
    )
    .unwrap();
    run(
        &view,
        cx,
        Command::SetInspectorTab {
            tab: InspectorTab::Video,
        },
    )
    .unwrap();
    run(
        &view,
        cx,
        Command::SetSectionExpanded {
            section: "reveal".into(),
            expanded: true,
        },
    )
    .unwrap();
    repaint(cx);
    let visible = |view: &Entity<Workstation>, cx: &mut VisualTestContext, id: &str| {
        node(view, cx, id).is_some_and(|node| node["visible"] == true)
    };
    assert!(
        visible(&view, cx, "inspector.reveal"),
        "a video clip has it"
    );
    for property in [
        "reveal_mode",
        "reveal_from",
        "reveal_to",
        "reveal_angle",
        "reveal_duration",
        "reveal_offset",
        "reveal_easing",
    ] {
        assert!(
            visible(&view, cx, &format!("property:{property}")),
            "{property}"
        );
    }

    let depth = |view: &Entity<Workstation>, cx: &mut VisualTestContext| {
        view.read_with(cx, |view, _| {
            view.state.history.to_json()["undo_depth"].as_u64().unwrap()
        })
    };
    let mode = |view: &Entity<Workstation>, cx: &mut VisualTestContext| {
        view.read_with(cx, |view, _| {
            match view
                .state
                .inspector
                .value("basalt-cliffs", property_def("reveal_mode").unwrap())
            {
                PropertyValue::Choice(text) => text,
                other => panic!("reveal_mode is a choice, got {other:?}"),
            }
        })
    };
    let before = depth(&view, cx);
    run(
        &view,
        cx,
        Command::SetProperty {
            property: "reveal_mode".into(),
            value: Raw::Text("wipe".into()),
        },
    )
    .unwrap();
    assert_eq!(depth(&view, cx), before + 1, "one edit, one entry");
    assert_eq!(mode(&view, cx), "wipe");
    run(&view, cx, Command::Undo).unwrap();
    assert_eq!(mode(&view, cx), "none");

    // A shape is a visual clip too: the section follows it.
    add_shape(&view, cx);
    repaint(cx);
    assert!(visible(&view, cx, "inspector.reveal"));
}

/// F072: the gradient stops of a shape are one more input of the Forma section, committed like
/// the rest: canonical spelling shown back, a refused text puts the model's value back.
#[gpui_kit::test]
fn the_gradient_stops_are_an_input_of_the_shape_section(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let id = add_shape(&view, cx);
    assert_eq!(model_field(&view, cx, &id, "stops"), "\"\"");
    type_and_confirm(&view, cx, "fill_mode", "linear");
    type_and_confirm(&view, cx, "stops", "0.58,#C6A7FF60  0.92 #c6a7fffa");
    assert_eq!(
        model_field(&view, cx, &id, "stops"),
        "\"0.58 #c6a7ff60 0.92 #c6a7fffa\""
    );
    assert_eq!(shown(&view, cx, "stops"), "0.58 #c6a7ff60 0.92 #c6a7fffa");

    // Out of order is refused and the field shows the model again.
    type_and_confirm(&view, cx, "stops", "0.9 #ffffff 0.5 #000000");
    assert_eq!(shown(&view, cx, "stops"), "0.58 #c6a7ff60 0.92 #c6a7fffa");
    assert_eq!(
        model_field(&view, cx, &id, "stops"),
        "\"0.58 #c6a7ff60 0.92 #c6a7fffa\""
    );
}
