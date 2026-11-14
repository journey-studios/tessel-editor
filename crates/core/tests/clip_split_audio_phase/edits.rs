use super::*;

// ---- no stale anchor: repeated cuts, moves, trims and speed changes --------------------------

#[test]
fn repeated_cuts_and_later_edits_leave_no_stale_anchor() {
    let rate = (30_000, 1001);
    let main = Spec::audio(rate, 44_100, true, 3, 24, Frac::new(1, 4));
    let mut state = audio_project(rate, 44_100, true, &["a1"]);
    add_audio(&mut state, "main", "a1", &main, None);
    let before = mix(&state);

    // Three cuts, each on the newest right half and one inside an older half.
    split(&mut state, "main", 8);
    split(&mut state, "main-split-1", 14);
    split(&mut state, "main-split-1-split-1", 20);
    split(&mut state, "main-split-1", 11);
    let pieces = [
        ("main", main.piece(3, 8)),
        ("main-split-1", main.piece(8, 11)),
        ("main-split-1-split-2", main.piece(11, 14)),
        ("main-split-1-split-1", main.piece(14, 20)),
        ("main-split-1-split-1-split-1", main.piece(20, 27)),
    ];
    for (id, spec) in &pieces {
        assert_eq!(ss_of(&state, id), spec.ss, "{id}");
    }
    let after = check(&state, 0, LENGTH, &pieces, "four cuts");
    same_bits(&before, &after, "four cuts");

    // Move the last piece 13 frames later: its source stays anchored to its own start.
    let (last, last_spec) = pieces[4];
    state
        .execute(Command::MoveClip {
            clip: last.into(),
            track: None,
            frame: Some(40),
            snap: Some(false),
        })
        .unwrap();
    let mut moved = last_spec;
    moved.start = 40;
    let edited = [pieces[0], pieces[1], pieces[2], pieces[3], (last, moved)];
    assert_eq!(
        ss_of(&state, last),
        last_spec.ss,
        "a move keeps source_start"
    );
    check(&state, 0, LENGTH, &edited, "after a move");

    // Trim the head of a middle piece by two frames: the source advances by exactly that.
    // (The timeline's snap would pull the edge to a neighbouring clip's edge.)
    state.timeline.snap = false;
    state
        .execute(Command::TrimClip {
            clip: "main-split-1-split-2".into(),
            edge: TrimEdge::Start,
            frame: 13,
        })
        .unwrap();
    let trimmed = pieces[2].1.piece(13, 14);
    assert_eq!(ss_of(&state, "main-split-1-split-2"), trimmed.ss);
    let edited = [
        pieces[0],
        pieces[1],
        ("main-split-1-split-2", trimmed),
        pieces[3],
        (last, moved),
    ];
    check(&state, 0, LENGTH, &edited, "after a trim");

    // Undo everything: the original clip, bit for bit.
    for _ in 0..6 {
        run(&mut state, json!({"op":"history.undo"}));
    }
    assert_eq!(state.timeline.clips.len(), 1);
    same_bits(&before, &mix(&state), "all undone");
}

#[test]
fn a_speed_change_after_the_cut_reads_from_the_halfs_own_anchor() {
    let rate = (24_000, 1001);
    let streams = [(44_100, true, Ratio::ZERO)];
    let mut main = Spec::audio(rate, 44_100, true, 5, 24, Frac::new(1, 4));
    main.audio_clip = false;
    let mut state = video_project(rate, &streams);
    add_video(&mut state, "main", &main, 0, 0);
    split(&mut state, "main", 17);
    run(
        &mut state,
        json!({"op":"clip.property","clip":"main-split-1","property":"speed","value":150}),
    );
    let (left, mut right) = (main.piece(5, 17), main.piece(17, 29));
    right.speed = 150;
    check(
        &state,
        0,
        LENGTH,
        &[("main", left), ("main-split-1", right)],
        "speed changed after the cut",
    );
}
