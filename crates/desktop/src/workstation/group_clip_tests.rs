//! F069: the group clip as the live window serves it. The timeline button inserts one with its
//! definition, the inspector inputs edit it through the same commands as `set group_def:<id> ...`,
//! a refused value puts the model's back and every edit is one undo entry.
use serde_json::Value;

use super::input_tests::fixture;
use super::*;
use crate::state::timeline::ClipKind;
use core::prelude::v1::test;

fn repaint(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn revision(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> u64 {
    view.read_with(cx, |view, _| view.state.project.revision)
}

/// Clicks the toolbar button and returns `(clip id, group id)` of what it selected.
fn add_group(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> (String, String) {
    let at = view.read_with(cx, |view, _| {
        view.geometry
            .get("control:timeline-add-group")
            .expect("the group button is laid out in the timeline toolbar")
            .center()
    });
    cx.simulate_click(at, Default::default());
    repaint(cx);
    view.read_with(cx, |view, _| {
        let clip = view
            .state
            .inspector_clip()
            .expect("the new group clip is selected");
        assert_eq!(clip.kind, ClipKind::Group);
        let group = clip.group.as_ref().expect("a group clip has an instance");
        (clip.id.to_string(), group.def.to_string())
    })
}

fn input_text(view: &Entity<Workstation>, cx: &mut VisualTestContext, part: &str) -> String {
    view.read_with(cx, |view, cx| {
        view.group_inputs
            .input_for(part)
            .unwrap_or_else(|| panic!("no group input matches {part}"))
            .read(cx)
            .value()
            .to_string()
    })
}

/// What a person does: types into one field, then presses Enter.
fn type_and_confirm(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    part: &str,
    text: &str,
) {
    view.update_in(cx, |view, window, cx| {
        let input = view
            .group_inputs
            .input_for(part)
            .unwrap_or_else(|| panic!("no group input matches {part}"));
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

fn undo(view: &Entity<Workstation>, cx: &mut VisualTestContext) {
    view.update(cx, |view, cx| view.execute(Command::Undo, Source::Cli, cx))
        .unwrap();
    repaint(cx);
}

fn def_node(view: &Entity<Workstation>, cx: &mut VisualTestContext, group: &str) -> Value {
    view.read_with(cx, |view, cx| {
        view.tree_node(&format!("group_def:{group}"), cx)
    })
    .expect("the group definition has a node")
}

#[gpui_kit::test]
fn the_timeline_button_inserts_a_group_with_its_definition_and_undo_removes_both(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    let before = view.read_with(cx, |view, _| {
        (
            view.state.timeline.clips.len(),
            view.state.timeline.tracks.len(),
            view.state.groups.defs.len(),
        )
    });
    let (clip, group) = add_group(&view, cx);
    view.read_with(cx, |view, _| {
        assert!(view.state.groups.def(&group).is_some());
        assert!(view.state.timeline.clip(&clip).is_some());
    });
    assert_eq!(
        view.read_with(cx, |view, cx| view.tree_node("inspector.group", cx))
            .unwrap()["visible"],
        true,
        "the group section follows the selection"
    );
    assert_eq!(def_node(&view, cx, &group)["state"]["stagger_ms"], 200);

    undo(&view, cx);
    let after = view.read_with(cx, |view, _| {
        (
            view.state.timeline.clips.len(),
            view.state.timeline.tracks.len(),
            view.state.groups.defs.len(),
        )
    });
    assert_eq!(after, before, "one gesture, one undo entry");
}

#[gpui_kit::test]
fn the_inputs_show_the_group_and_enter_commits_one_undoable_edit(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let (_, group) = add_group(&view, cx);
    assert_eq!(input_text(&view, cx, "stagger_ms"), "200");

    let start = revision(&view, cx);
    type_and_confirm(&view, cx, "stagger_ms", "350");
    assert_eq!(def_node(&view, cx, &group)["state"]["stagger_ms"], 350);
    assert_eq!(input_text(&view, cx, "stagger_ms"), "350");
    assert_eq!(revision(&view, cx), start + 1, "one commit, one revision");

    // The same value again is not an edit.
    type_and_confirm(&view, cx, "stagger_ms", "350");
    assert_eq!(revision(&view, cx), start + 1);

    undo(&view, cx);
    assert_eq!(
        input_text(&view, cx, "stagger_ms"),
        "200",
        "undo reaches the input"
    );
}

#[gpui_kit::test]
fn a_refused_value_changes_nothing_and_the_input_shows_the_model_again(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let (_, group) = add_group(&view, cx);
    let start = revision(&view, cx);

    type_and_confirm(&view, cx, "stagger_ms", "999999");
    assert_eq!(input_text(&view, cx, "stagger_ms"), "200");
    assert_eq!(def_node(&view, cx, &group)["state"]["stagger_ms"], 200);
    assert_eq!(revision(&view, cx), start);
    let (error, text) = view.read_with(cx, |view, _| {
        let activity = view.activity.as_ref().unwrap();
        (activity.error, activity.text.clone())
    });
    assert!(error, "the refusal is reported: {text}");
}

#[gpui_kit::test]
fn a_clip_overrides_one_text_without_touching_the_definition(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let (clip, group) = add_group(&view, cx);
    assert_eq!(input_text(&view, cx, "Text { clip"), "Título");

    type_and_confirm(&view, cx, "Text { clip", "Etapa 1");
    view.read_with(cx, |view, _| {
        let instance = view
            .state
            .timeline
            .clip(&clip)
            .unwrap()
            .group
            .clone()
            .unwrap();
        assert!(
            instance
                .texts
                .iter()
                .any(|(child, text)| child.as_str() == "title" && text == "Etapa 1")
        );
        assert!(view.state.groups.def(&group).is_some());
    });
    assert_eq!(input_text(&view, cx, "Text { clip"), "Etapa 1");

    undo(&view, cx);
    assert_eq!(input_text(&view, cx, "Text { clip"), "Título");
}

#[gpui_kit::test]
fn a_delayed_input_event_cannot_write_into_a_different_selected_group(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let (_, group) = add_group(&view, cx);
    view.update(cx, |view, _| view.group_inputs.forget());
    // Not synced to this clip: the commit still goes through the model's validation and, being
    // the same text the model holds, changes nothing.
    let start = revision(&view, cx);
    type_and_confirm(&view, cx, "stagger_ms", "200");
    assert_eq!(def_node(&view, cx, &group)["state"]["stagger_ms"], 200);
    assert_eq!(
        revision(&view, cx),
        start,
        "an unchanged value is not an edit"
    );
}

/// Clicks the "Adicionar chat" button and returns `(clip id, group id)` of what it selected.
fn add_chat(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> (String, String) {
    let at = view.read_with(cx, |view, _| {
        view.geometry
            .get("control:timeline-add-chat")
            .expect("the chat button is laid out in the timeline toolbar")
            .center()
    });
    cx.simulate_click(at, Default::default());
    repaint(cx);
    view.read_with(cx, |view, _| {
        let clip = view
            .state
            .inspector_clip()
            .expect("the chat clip is selected");
        assert_eq!(clip.kind, ClipKind::Group);
        (
            clip.id.to_string(),
            clip.group.as_ref().unwrap().def.to_string(),
        )
    })
}

fn child_node(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    group: &str,
    child: &str,
) -> Value {
    view.read_with(cx, |view, cx| {
        view.tree_node(&format!("group_child:{group}/{child}"), cx)
    })
    .expect("the chat child has a node")
}

#[gpui_kit::test]
fn the_chat_button_inserts_the_preset_and_one_undo_removes_it(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    assert_eq!(
        view.read_with(cx, |view, cx| view.tree_node("timeline.add_chat", cx))
            .unwrap()["label"],
        "Adicionar chat"
    );
    let before = view.read_with(cx, |view, _| {
        (
            view.state.timeline.clips.len(),
            view.state.timeline.tracks.len(),
        )
    });
    let (clip, group) = add_chat(&view, cx);
    view.read_with(cx, |view, _| {
        let def = view
            .state
            .groups
            .def(&group)
            .expect("the chat group exists");
        assert_eq!(def.children.len(), 6);
        assert!(view.state.timeline.clip(&clip).is_some());
    });
    undo(&view, cx);
    view.read_with(cx, |view, _| {
        assert!(view.state.groups.is_empty());
        assert_eq!(
            (
                view.state.timeline.clips.len(),
                view.state.timeline.tracks.len()
            ),
            before,
            "one gesture, one undo entry"
        );
    });
}

#[gpui_kit::test]
fn the_reveal_and_the_schedule_are_editable_in_the_inspector_and_undo_reaches_the_inputs(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    let (_, group) = add_chat(&view, cx);
    let answer = r#"child: "answer", field: "#;
    assert_eq!(
        input_text(&view, cx, &format!("{answer}\"reveal\"")),
        "lines"
    );
    assert_eq!(
        input_text(&view, cx, &format!("{answer}\"step_ms\"")),
        "450"
    );
    assert_eq!(
        input_text(&view, cx, &format!("{answer}\"after\"")),
        "question"
    );
    assert_eq!(
        input_text(&view, cx, &format!("{answer}\"delay_ms\"")),
        "300"
    );

    let start = revision(&view, cx);
    type_and_confirm(&view, cx, &format!("{answer}\"reveal\""), "type");
    assert_eq!(
        child_node(&view, cx, &group, "answer")["state"]["reveal"],
        "type"
    );
    assert_eq!(revision(&view, cx), start + 1, "one commit, one revision");
    assert_eq!(
        input_text(&view, cx, &format!("{answer}\"step_ms\"")),
        "40",
        "a new mode starts at its own step"
    );

    type_and_confirm(&view, cx, &format!("{answer}\"delay_ms\""), "0");
    assert_eq!(
        child_node(&view, cx, &group, "answer")["state"]["delay_ms"],
        0
    );

    undo(&view, cx);
    undo(&view, cx);
    assert_eq!(
        input_text(&view, cx, &format!("{answer}\"reveal\"")),
        "lines"
    );
    assert_eq!(
        input_text(&view, cx, &format!("{answer}\"step_ms\"")),
        "450"
    );
    assert_eq!(
        input_text(&view, cx, &format!("{answer}\"delay_ms\"")),
        "300"
    );
}

#[gpui_kit::test]
fn an_after_that_points_forward_is_refused_and_the_input_shows_the_model_again(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    let (_, group) = add_chat(&view, cx);
    let question = r#"child: "question", field: "after""#;
    let start = revision(&view, cx);
    type_and_confirm(&view, cx, question, "answer");
    assert_eq!(input_text(&view, cx, question), "");
    assert_eq!(
        child_node(&view, cx, &group, "question")["state"]["after"],
        ""
    );
    assert_eq!(revision(&view, cx), start, "a refusal is not an edit");
    let (error, text) = view.read_with(cx, |view, _| {
        let activity = view.activity.as_ref().unwrap();
        (activity.error, activity.text.clone())
    });
    assert!(error, "the refusal is reported: {text}");
}
