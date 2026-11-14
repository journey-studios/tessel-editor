use super::*;

// ---- chunks and external fades ---------------------------------------------------------------

#[test]
fn chunks_are_the_whole_mix_and_external_fades_keep_their_envelope() {
    for rate in [(30_000, 1001), (24_000, 1001), (25, 1)] {
        let mut main = Spec::audio(rate, 44_100, true, 3, 24, Frac::new(1, 4));
        main.fade_in = 4;
        main.fade_out = 6;
        let cross = Spec::audio(rate, 44_100, true, 9, 20, Frac::new(1, 3));
        let mut state = audio_project(rate, 44_100, true, &["a1", "a2"]);
        add_audio(&mut state, "main", "a1", &main, None);
        add_audio(&mut state, "cross", "a2", &cross, None);
        for (clip, property, value) in [("main", "fade_in", 4), ("main", "fade_out", 6)] {
            run(
                &mut state,
                json!({"op":"clip.property","clip":clip,"property":property,"value":value}),
            );
        }
        let label = format!("{rate:?}");
        let whole = [("main", main), ("cross", cross)];
        let before = check(&state, 0, LENGTH, &whole, &format!("{label} whole"));
        assert!(before.samples.iter().any(|sample| sample.abs() > 0.01));

        // Cuts in the plateau between the ramps (frames 7..=21), then in chunks.
        for cut in [7, 8, 14, 21] {
            let tag = format!("{label} fades cut {cut}");
            split(&mut state, "main", cut);
            let (left, right) = (main.piece(3, cut), main.piece(cut, 27));
            assert_eq!((left.fade_in, left.fade_out), (4, 0));
            assert_eq!((right.fade_in, right.fade_out), (0, 6));
            let halves = [("main", left), ("main-split-1", right), ("cross", cross)];
            let after = check(&state, 0, LENGTH, &halves, &tag);
            same_bits(&before, &after, &tag);

            // Chunks that start and stop inside the clips, off the cut, laid end to end.
            let points = [0, 5, cut, cut + 1, 29, 57, LENGTH];
            let mut points: Vec<i64> = points.into_iter().collect();
            points.sort_unstable();
            points.dedup();
            let mut joined: Vec<f32> = Vec::new();
            for window in points.windows(2) {
                let block = check(
                    &state,
                    window[0],
                    window[1] - window[0],
                    &halves,
                    &format!("{tag} block {window:?}"),
                );
                joined.extend_from_slice(&block.samples);
            }
            assert_eq!(joined.len(), before.samples.len(), "{tag}: chunk total");
            assert!(
                joined
                    .iter()
                    .zip(&before.samples)
                    .all(|(a, b)| a.to_bits() == b.to_bits()),
                "{tag}: chunks differ from the whole mix"
            );
            run(&mut state, json!({"op":"history.undo"}));
        }
    }
}

// ---- mute, unmute and the old guard ------------------------------------------------------------

#[test]
fn mute_and_unmute_are_equivalent_and_no_longer_a_bypass_of_the_cut() {
    let rate = (30_000, 1001);
    let streams = [(44_100, true, Ratio::ZERO)];
    let mut main = Spec::audio(rate, 44_100, true, 5, 24, Frac::new(1, 4));
    main.audio_clip = false;
    let mut state = video_project(rate, &streams);
    add_video(&mut state, "main", &main, 0, 0);
    let before = check(&state, 0, LENGTH, &[("main", main)], "whole");

    run(
        &mut state,
        json!({"op":"clip.property","clip":"main","property":"audio_muted","value":true}),
    );
    assert!(mix(&state).samples.iter().all(|sample| *sample == 0.0));
    state.timeline.selected_clip = state.timeline.clip("main").map(|clip| clip.id);
    state.timeline.playhead = 14;
    assert!(
        matches!(
            state.split_availability(),
            SplitAvailability::Ready { frame: 14, .. }
        ),
        "a muted NTSC clip is cut like any other"
    );
    split(&mut state, "main", 14);
    assert!(mix(&state).samples.iter().all(|sample| *sample == 0.0));
    for clip in ["main", "main-split-1"] {
        run(
            &mut state,
            json!({"op":"clip.property","clip":clip,"property":"audio_muted","value":false}),
        );
    }
    let halves = [
        ("main", main.piece(5, 14)),
        ("main-split-1", main.piece(14, 29)),
    ];
    let after = check(&state, 0, LENGTH, &halves, "unmuted halves");
    same_bits(&before, &after, "unmuted halves");

    // An audio track muted before the cut and unmuted after it.
    let main = Spec::audio(rate, 48_000, false, 3, 24, Frac::new(1, 4));
    let mut state = audio_project(rate, 48_000, false, &["a1"]);
    add_audio(&mut state, "main", "a1", &main, Some(-100));
    let before = mix(&state);
    let mute = |state: &mut EditorState, value| {
        state
            .execute(Command::SetTrackFlag {
                track: "a1".into(),
                flag: TrackFlag::Muted,
                value,
            })
            .unwrap();
    };
    mute(&mut state, true);
    split(&mut state, "main", 11);
    assert!(mix(&state).samples.iter().all(|sample| *sample == 0.0));
    mute(&mut state, false);
    same_bits(&before, &mix(&state), "track unmuted");
}
