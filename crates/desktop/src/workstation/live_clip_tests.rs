//! F071: the player and caption clips as the live window serves them. The timeline buttons insert
//! one, the inspector inputs edit it through the same commands as `set clip_player:<id> ...` and
//! `set subtitle_cue:<id>/<n> ...`, a refused value puts the model's back and every edit is one
//! undo entry.
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

/// Clicks a toolbar button and returns the id of the clip it selected, of `kind`.
fn add(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    probe: &str,
    kind: ClipKind,
) -> String {
    let at = view.read_with(cx, |view, _| {
        view.geometry
            .get(probe)
            .unwrap_or_else(|| panic!("{probe} is laid out in the timeline toolbar"))
            .center()
    });
    cx.simulate_click(at, Default::default());
    repaint(cx);
    view.read_with(cx, |view, _| {
        let clip = view
            .state
            .inspector_clip()
            .expect("the new clip is selected");
        assert_eq!(clip.kind, kind);
        clip.id.to_string()
    })
}

fn confirm(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    input: Entity<InputState>,
    text: &str,
) {
    view.update_in(cx, |_, window, cx| {
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

fn shown(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    input: Entity<InputState>,
) -> String {
    view.read_with(cx, |_, cx| input.read(cx).value().to_string())
}

fn player_of(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    id: &str,
) -> tessel_editor_core::player::PlayerSpec {
    view.read_with(cx, |view, _| {
        view.state
            .timeline
            .clip(id)
            .unwrap()
            .player
            .clone()
            .unwrap()
    })
}

fn subtitles_of(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    id: &str,
) -> tessel_editor_core::subtitles::SubtitleSpec {
    view.read_with(cx, |view, _| view.state.subtitles_of(id).unwrap().1)
}

fn undo(view: &Entity<Workstation>, cx: &mut VisualTestContext) {
    view.update(cx, |view, cx| view.execute(Command::Undo, Source::Cli, cx))
        .unwrap();
    repaint(cx);
}

#[gpui_kit::test]
fn the_player_button_inserts_one_player_and_undo_removes_clip_and_track(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let before = view.read_with(cx, |view, _| {
        (
            view.state.timeline.clips.len(),
            view.state.timeline.tracks.len(),
        )
    });
    let id = add(&view, cx, "control:timeline-add-player", ClipKind::Player);
    assert_eq!(player_of(&view, cx, &id).bars, 48);
    undo(&view, cx);
    let after = view.read_with(cx, |view, _| {
        (
            view.state.timeline.clips.len(),
            view.state.timeline.tracks.len(),
        )
    });
    assert_eq!(after, before, "one gesture, one undo entry");
}

#[gpui_kit::test]
fn the_player_inputs_show_the_model_and_enter_commits_one_undoable_edit(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let id = add(&view, cx, "control:timeline-add-player", ClipKind::Player);
    let input = |view: &Entity<Workstation>, cx: &mut VisualTestContext, field: &str| {
        view.read_with(cx, |view, _| view.live_inputs.player_input(field))
    };
    let bars = input(&view, cx, "bars");
    assert_eq!(shown(&view, cx, bars.clone()), "48");

    let start = revision(&view, cx);
    confirm(&view, cx, bars.clone(), "24");
    assert_eq!(player_of(&view, cx, &id).bars, 24);
    assert_eq!(revision(&view, cx), start + 1, "one commit, one revision");

    confirm(&view, cx, bars.clone(), "24");
    assert_eq!(
        revision(&view, cx),
        start + 1,
        "the same value is not an edit"
    );

    let fill = input(&view, cx, "fill_color");
    confirm(&view, cx, fill, "#112233");
    assert_eq!(player_of(&view, cx, &id).fill_color, "#112233");

    undo(&view, cx);
    assert_eq!(player_of(&view, cx, &id).fill_color, "#4a90e2");
    let fill = input(&view, cx, "fill_color");
    assert_eq!(shown(&view, cx, fill), "#4a90e2", "undo reaches the input");
}

#[gpui_kit::test]
fn a_refused_player_value_changes_nothing_and_is_reported(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let id = add(&view, cx, "control:timeline-add-player", ClipKind::Player);
    let start = revision(&view, cx);
    let bars = view.read_with(cx, |view, _| view.live_inputs.player_input("bars"));
    confirm(&view, cx, bars.clone(), "500");
    assert_eq!(player_of(&view, cx, &id).bars, 48);
    assert_eq!(shown(&view, cx, bars), "48", "the model's value is back");
    assert_eq!(revision(&view, cx), start);
    let (error, text) = view.read_with(cx, |view, _| {
        let activity = view.activity.as_ref().unwrap();
        (activity.error, activity.text.clone())
    });
    assert!(error, "the refusal is reported: {text}");

    let audio = view.read_with(cx, |view, _| view.live_inputs.player_input("audio"));
    // A new player follows the audio clip of the sequence: a link to anything else is refused.
    let linked = player_of(&view, cx, &id).audio;
    assert!(linked.is_some(), "the fixture has an audio clip to follow");
    confirm(&view, cx, audio.clone(), "not-an-audio-clip");
    assert_eq!(
        player_of(&view, cx, &id).audio,
        linked,
        "a link must be an audio clip"
    );
    confirm(&view, cx, audio, "");
    assert_eq!(
        player_of(&view, cx, &id).audio,
        None,
        "empty text unlinks the player"
    );
}

#[gpui_kit::test]
fn a_delayed_input_event_cannot_write_into_a_different_selected_player(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let id = add(&view, cx, "control:timeline-add-player", ClipKind::Player);
    view.update(cx, |view, _| view.live_inputs.forget());
    let bars = view.read_with(cx, |view, _| view.live_inputs.player_input("bars"));
    confirm(&view, cx, bars, "12");
    assert_eq!(
        player_of(&view, cx, &id).bars,
        48,
        "inputs not synced to this clip never edit it"
    );
}

#[gpui_kit::test]
fn the_subtitles_button_inserts_one_track_with_a_starter_cue(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let id = add(
        &view,
        cx,
        "control:timeline-add-subtitles",
        ClipKind::Subtitles,
    );
    assert_eq!(subtitles_of(&view, cx, &id).cues.len(), 1);
    let text = view.read_with(cx, |view, _| view.live_inputs.cue_input("text"));
    assert_eq!(shown(&view, cx, text), "Sua legenda");
    undo(&view, cx);
    view.read_with(cx, |view, _| {
        assert!(view.state.timeline.clip(&id).is_none())
    });
}

#[gpui_kit::test]
fn the_cue_and_style_inputs_commit_through_the_shared_commands(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let id = add(
        &view,
        cx,
        "control:timeline-add-subtitles",
        ClipKind::Subtitles,
    );

    let text = view.read_with(cx, |view, _| view.live_inputs.cue_input("text"));
    confirm(&view, cx, text, "Olá, mundo");
    assert_eq!(subtitles_of(&view, cx, &id).cues[0].text, "Olá, mundo");

    let end = view.read_with(cx, |view, _| view.live_inputs.cue_input("end_ms"));
    confirm(&view, cx, end.clone(), "3500");
    assert_eq!(subtitles_of(&view, cx, &id).cues[0].end_ms, 3500);
    // A cue that ends before it starts is refused and the input shows the model again.
    confirm(&view, cx, end.clone(), "-5");
    assert_eq!(shown(&view, cx, end), "3500");

    let burn = view.read_with(cx, |view, _| view.live_inputs.style_input("burn"));
    assert_eq!(shown(&view, cx, burn.clone()), "true");
    confirm(&view, cx, burn, "false");
    assert!(!subtitles_of(&view, cx, &id).burn);

    let size = view.read_with(cx, |view, _| view.live_inputs.style_input("size"));
    confirm(&view, cx, size, "6");
    assert_eq!(subtitles_of(&view, cx, &id).size, 6.0);

    undo(&view, cx);
    assert_eq!(subtitles_of(&view, cx, &id).size, 4.6);
    assert!(
        !subtitles_of(&view, cx, &id).burn,
        "one undo undoes one edit"
    );
}

#[gpui_kit::test]
fn the_cue_buttons_add_at_the_playhead_step_and_remove(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let id = add(
        &view,
        cx,
        "control:timeline-add-subtitles",
        ClipKind::Subtitles,
    );
    view.update(cx, |view, cx| {
        view.dispatch(
            Command::Live(crate::state::live_edit::LiveCommand::SubtitleCueAdd {
                clip: id.clone(),
                start_ms: 5000,
                end_ms: 6000,
                text: "Segunda".to_string(),
            }),
            cx,
        )
    });
    repaint(cx);
    assert_eq!(subtitles_of(&view, cx, &id).cues.len(), 2);

    // "Next" moves the editor to the second cue; it shows that cue's text.
    view.update(cx, |view, cx| view.show_cue(1, cx));
    repaint(cx);
    let text = view.read_with(cx, |view, _| view.live_inputs.cue_input("text"));
    assert_eq!(shown(&view, cx, text.clone()), "Segunda");
    confirm(&view, cx, text, "Segunda legenda");
    assert_eq!(subtitles_of(&view, cx, &id).cues[1].text, "Segunda legenda");

    view.update(cx, |view, cx| view.remove_cue(cx));
    repaint(cx);
    let spec = subtitles_of(&view, cx, &id);
    assert_eq!(spec.cues.len(), 1);
    let text = view.read_with(cx, |view, _| view.live_inputs.cue_input("text"));
    assert_eq!(
        shown(&view, cx, text),
        "Sua legenda",
        "the editor falls back to the last cue"
    );
}

#[gpui_kit::test]
fn the_import_field_reads_an_srt_file_and_reports_a_missing_one(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let id = add(
        &view,
        cx,
        "control:timeline-add-subtitles",
        ClipKind::Subtitles,
    );
    let dir = std::env::temp_dir().join(format!("tessel-live-srt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("falas.srt");
    std::fs::write(
        &file,
        "1\n00:00:00,500 --> 00:00:01,500\nPrimeira\n\n2\n00:00:02,000 --> 00:00:03,000\nSegunda\n",
    )
    .unwrap();

    let input = view.read_with(cx, |view, _| view.live_inputs.srt_input());
    confirm(&view, cx, input.clone(), &file.display().to_string());
    let spec = subtitles_of(&view, cx, &id);
    assert_eq!(
        spec.cues
            .iter()
            .map(|cue| cue.text.as_str())
            .collect::<Vec<_>>(),
        ["Primeira", "Segunda"]
    );

    let start = revision(&view, cx);
    confirm(
        &view,
        cx,
        input,
        &dir.join("nao-existe.srt").display().to_string(),
    );
    assert_eq!(revision(&view, cx), start, "a missing file changes nothing");
    let error = view.read_with(cx, |view, _| view.activity.as_ref().unwrap().error);
    assert!(error, "the missing file is reported");
    let _ = std::fs::remove_dir_all(dir);
}
