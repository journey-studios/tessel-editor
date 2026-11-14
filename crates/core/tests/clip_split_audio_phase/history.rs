use super::*;

// ---- save, reopen, undo, redo ---------------------------------------------------------------------

#[test]
fn save_reopen_undo_and_redo_keep_the_pcm() {
    let rate = (30_000, 1001);
    let main = Spec::audio(rate, 44_100, true, 3, 24, Frac::new(1, 4));
    let mut state = audio_project(rate, 44_100, true, &["a1"]);
    add_audio(&mut state, "main", "a1", &main, None);
    let before = mix(&state);
    split(&mut state, "main", 14);
    split(&mut state, "main-split-1", 20);
    let after = mix(&state);
    same_bits(&before, &after, "two cuts");

    let path = shared_dir().join("cut.tessel.json");
    persist::save(&mut state, &path).unwrap();
    let reopened = persist::load(&path).unwrap();
    for clip in ["main", "main-split-1", "main-split-1-split-1"] {
        assert_eq!(
            reopened.timeline.clip(clip),
            state.timeline.clip(clip),
            "{clip}"
        );
    }
    same_bits(&before, &mix(&reopened), "reopened");

    run(&mut state, json!({"op":"history.undo"}));
    run(&mut state, json!({"op":"history.undo"}));
    same_bits(&before, &mix(&state), "undone");
    run(&mut state, json!({"op":"history.redo"}));
    run(&mut state, json!({"op":"history.redo"}));
    same_bits(&before, &mix(&state), "redone");
    assert_eq!(state.timeline.clips.len(), 3);
}

// ---- refusals stay atomic --------------------------------------------------------------------------

#[test]
fn a_source_position_that_overflows_is_refused_atomically_with_a_slider_open() {
    use tessel_editor_core::Raw;
    use tessel_editor_core::state::property_gesture::GestureCommand;
    let rate = (30_000, 1001);
    let main = Spec::audio(rate, 44_100, true, 3, 24, Frac::new(1, 4));
    let mut state = audio_project(rate, 44_100, true, &["a1"]);
    add_audio(&mut state, "main", "a1", &main, None);
    state
        .timeline
        .clips
        .iter_mut()
        .find(|clip| clip.id == "main")
        .unwrap()
        .source_start = Ratio::from_int(i64::MAX / 40_000);
    state
        .execute(Command::PropertyGesture(GestureCommand::Begin {
            clip: Some("main".into()),
            property: "volume".into(),
        }))
        .unwrap();
    state
        .execute(Command::PropertyGesture(GestureCommand::Update {
            gesture: None,
            value: Raw::Number(-3.0),
        }))
        .unwrap();
    let before = fingerprint(&state);
    let gesture = state.open_property_gesture();
    let error = fail(
        &mut state,
        json!({"op":"clip.split","clip":"main","frame":14}),
    );
    assert_eq!(error.code, "invalid_time");
    assert_eq!(state.open_property_gesture(), gesture);
    assert_eq!(fingerprint(&state), before);
    assert_eq!(
        state.plan_audio(0, LENGTH).unwrap_err().code,
        "invalid_time"
    );
}

#[test]
fn a_cut_inside_an_audio_fade_is_still_refused_without_changing_the_sound() {
    // BL081.2 stays outside this slice: the ramp is not divided, on any grid.
    let rate = (30_000, 1001);
    let main = Spec::audio(rate, 44_100, true, 3, 24, Frac::new(1, 4));
    let mut state = audio_project(rate, 44_100, true, &["a1"]);
    add_audio(&mut state, "main", "a1", &main, None);
    for (property, value) in [("fade_in", 6), ("fade_out", 6)] {
        run(
            &mut state,
            json!({"op":"clip.property","clip":"main","property":property,"value":value}),
        );
    }
    let before = fingerprint(&state);
    let sound = mix(&state);
    for frame in [5, 24] {
        let error = fail(
            &mut state,
            json!({"op":"clip.split","clip":"main","frame":frame}),
        );
        assert_eq!(error.code, "split_unsupported", "frame {frame}");
        assert_eq!(fingerprint(&state), before);
    }
    same_bits(&sound, &mix(&state), "unchanged");
    state.timeline.selected_clip = state.timeline.clip("main").map(|clip| clip.id);
    state.timeline.playhead = 5;
    assert_eq!(
        state.split_availability(),
        SplitAvailability::Blocked(SplitBlock::FadeCrossesCut)
    );
}
