//! BL141: the title text input of the inspector and the `clip_text:` node, as the live window
//! serves them. Typing is one commit (Enter or leaving the field) through the shared command, a
//! refused text puts the model's text back, and decomposed accents are stored composed.
use serde_json::Value;
use tessel_editor_core::commands::Parsed;

use super::input_tests::fixture;
use super::*;
use core::prelude::v1::test;

fn node(view: &Entity<Workstation>, cx: &mut VisualTestContext, id: &str) -> Option<Value> {
    view.read_with(cx, |view, cx| view.tree_node(id, cx))
}

fn select_title(view: &Entity<Workstation>, cx: &mut VisualTestContext) {
    let parsed = view
        .read_with(cx, |view, cx| {
            let layout = view.layout_snapshot(cx);
            tessel_editor_core::commands::parse_invoke(
                &view.state,
                &layout,
                "clip:title-north",
                "select",
            )
        })
        .unwrap();
    let Parsed::Run(command) = parsed else {
        panic!("selecting a clip is a state command");
    };
    view.update(cx, |view, cx| view.execute(command, Source::Cli, cx))
        .unwrap();
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn model_text(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |view, _| {
        view.state
            .timeline
            .clip("title-north")
            .unwrap()
            .text
            .as_ref()
            .unwrap()
            .content
            .clone()
    })
}

fn shown(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |view, cx| view.title_text.read(cx).value().to_string())
}

/// What a person does: types into the field, then presses Enter.
fn type_and_confirm(view: &Entity<Workstation>, cx: &mut VisualTestContext, text: &str) {
    view.update_in(cx, |view, window, cx| {
        view.title_text.update(cx, |input, cx| {
            input.set_value(text.to_string(), window, cx)
        });
        view.title_text.update(cx, |_, cx| {
            cx.emit(InputEvent::PressEnter {
                secondary: false,
                shift: false,
            });
        });
    });
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_text_section_and_input_follow_the_selected_title(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    select_title(&view, cx);
    let section = node(&view, cx, "inspector.text").expect("the text section exists");
    assert_eq!(section["visible"], true);
    let field = node(&view, cx, "clip_text:title-north").expect("the title text has a node");
    assert_eq!(field["state"]["content"], "NORTH");
    assert_eq!(
        shown(&view, cx),
        "NORTH",
        "the input shows the model's text"
    );

    // A clip that is not a title has no text node and no section.
    let parsed = view
        .read_with(cx, |view, cx| {
            let layout = view.layout_snapshot(cx);
            tessel_editor_core::commands::parse_invoke(
                &view.state,
                &layout,
                "clip:first-light",
                "select",
            )
        })
        .unwrap();
    let Parsed::Run(command) = parsed else {
        panic!("selecting a clip is a state command");
    };
    view.update(cx, |view, cx| view.execute(command, Source::Cli, cx))
        .unwrap();
    cx.run_until_parked();
    assert!(node(&view, cx, "clip_text:title-north").is_none());
    assert_eq!(node(&view, cx, "inspector.text").unwrap()["visible"], false);
}

#[gpui_kit::test]
fn enter_commits_one_undoable_edit_with_the_text_composed(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    select_title(&view, cx);
    let revision = view.read_with(cx, |view, _| view.state.project.revision);

    type_and_confirm(&view, cx, "Ac\u{0327}a\u{0303}o do Pa\u{0301}ssaro");
    assert_eq!(model_text(&view, cx), "Ação do Pássaro");
    assert_eq!(
        shown(&view, cx),
        "Ação do Pássaro",
        "the input shows it composed"
    );
    assert_eq!(
        view.read_with(cx, |view, _| view.state.project.revision),
        revision + 1,
        "one commit, one revision"
    );

    // The same text again is not an edit.
    type_and_confirm(&view, cx, "Ação do Pássaro");
    assert_eq!(
        view.read_with(cx, |view, _| view.state.project.revision),
        revision + 1
    );

    view.update(cx, |view, cx| view.execute(Command::Undo, Source::Cli, cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(model_text(&view, cx), "NORTH");
    assert_eq!(shown(&view, cx), "NORTH", "undo reaches the input");
}

#[gpui_kit::test]
fn a_refused_text_changes_nothing_and_the_input_shows_the_model_again(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    select_title(&view, cx);
    type_and_confirm(&view, cx, "AÇÃO");
    let revision = view.read_with(cx, |view, _| view.state.project.revision);

    type_and_confirm(&view, cx, "preço €5");
    assert_eq!(model_text(&view, cx), "AÇÃO");
    assert_eq!(
        shown(&view, cx),
        "AÇÃO",
        "the refused text is not left in the field"
    );
    assert_eq!(
        view.read_with(cx, |view, _| view.state.project.revision),
        revision
    );
    let (error, text) = view.read_with(cx, |view, _| {
        let activity = view.activity.as_ref().unwrap();
        (activity.error, activity.text.clone())
    });
    assert!(error, "the refusal is reported: {text}");
}

#[gpui_kit::test]
fn the_semantic_set_and_the_input_run_the_same_command(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    select_title(&view, cx);
    let Parsed::Run(command) = tessel_editor_core::commands::parse_set(
        "clip_text:title-north",
        "content",
        &Raw::Text("Fe\u{0301}rias".into()),
    )
    .unwrap() else {
        panic!("a title text field is a state command");
    };
    view.update(cx, |view, cx| view.execute(command, Source::Cli, cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(model_text(&view, cx), "Férias");
    assert_eq!(
        shown(&view, cx),
        "Férias",
        "a script's edit reaches the input"
    );
}

#[gpui_kit::test]
fn one_click_changes_the_title_preset_and_undo_restores_it(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    select_title(&view, cx);
    let animation = |cx: &mut VisualTestContext| {
        node(&view, cx, "clip_text:title-north").unwrap()["state"]["animation"].clone()
    };
    assert_eq!(animation(cx), "none");
    let at = view.read_with(cx, |view, _| {
        view.geometry
            .get("control:title-animation-rise")
            .expect("the rise preset is laid out in the title inspector")
            .center()
    });
    cx.simulate_click(at, Default::default());
    cx.run_until_parked();
    assert_eq!(animation(cx), "rise");

    let Parsed::Run(command) = tessel_editor_core::commands::parse_set(
        "clip_text:title-north",
        "animation",
        &Raw::Text("pop".into()),
    )
    .unwrap() else {
        panic!("the animation field must use the shared state command");
    };
    view.update(cx, |view, cx| view.execute(command, Source::Cli, cx))
        .unwrap();
    assert_eq!(animation(cx), "pop");
    view.update(cx, |view, cx| view.execute(Command::Undo, Source::Cli, cx))
        .unwrap();
    assert_eq!(animation(cx), "rise");
}

#[gpui_kit::test]
fn the_title_input_keeps_explicit_line_breaks(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    select_title(&view, cx);
    type_and_confirm(&view, cx, "Uma história\nem dois atos");
    assert_eq!(model_text(&view, cx), "Uma história\nem dois atos");
    assert_eq!(shown(&view, cx), "Uma história\nem dois atos");
}

#[gpui_kit::test]
fn the_timeline_button_inserts_and_selects_a_new_title(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let at = view.read_with(cx, |view, _| {
        view.geometry
            .get("control:timeline-add-title")
            .expect("the title button is laid out in the timeline toolbar")
            .center()
    });
    cx.simulate_click(at, Default::default());
    cx.run_until_parked();
    let title = view.read_with(cx, |view, _| {
        let clip = view.state.inspector_clip().expect("new title selected");
        assert_eq!(clip.kind, crate::state::timeline::ClipKind::Title);
        assert_eq!(clip.start, view.state.timeline.playhead);
        assert_eq!(clip.text.as_ref().unwrap().font.as_str(), "sans");
        clip.id.to_string()
    });
    assert!(node(&view, cx, &format!("clip_text:{title}")).is_some());
    type_and_confirm(&view, cx, "O próximo capítulo");
    let field = node(&view, cx, &format!("clip_text:{title}")).unwrap();
    assert_eq!(field["state"]["content"], "O próximo capítulo");
}

#[gpui_kit::test]
fn a_delayed_input_event_cannot_write_into_a_different_selected_title(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    select_title(&view, cx);
    let before = model_text(&view, cx);
    view.update(cx, |view, _| {
        view.title_text_shown = Some(("previous-title".into(), "Old title".into()));
    });
    type_and_confirm(&view, cx, "Texto atrasado");
    assert_eq!(model_text(&view, cx), before);
}

#[gpui_kit::test]
fn an_equivalent_decomposed_commit_resynchronizes_the_input_without_an_edit(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    select_title(&view, cx);
    type_and_confirm(&view, cx, "Ação");
    let revision = view.read_with(cx, |view, _| view.state.project.revision);
    type_and_confirm(&view, cx, "Ac\u{0327}a\u{0303}o");
    assert_eq!(shown(&view, cx), "Ação");
    assert_eq!(model_text(&view, cx), "Ação");
    assert_eq!(
        view.read_with(cx, |view, _| view.state.project.revision),
        revision
    );
}
