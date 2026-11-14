use super::*;

// ---- audio clips: every grid, source rate, channel layout and cut ---------------------------

#[test]
fn every_cut_keeps_the_phase_on_every_grid_for_audio_clips() {
    let mut cases = 0;
    for rate in RATES {
        for sr in [44_100, 48_000] {
            for stereo in [false, true] {
                let pan = if stereo { None } else { Some(-100) };
                let main = Spec::audio(rate, sr, stereo, 3, 24, Frac::new(1, 4));
                // Clips on other tracks that end before the cut, cross it, sit inside the left
                // half or start after it, none aligned with it (different phases).
                let pre = Spec::audio(rate, sr, stereo, 0, 9, Frac::int(0));
                let cross = Spec::audio(rate, sr, stereo, 9, 20, Frac::new(1, 3));
                let post = Spec::audio(rate, sr, stereo, 20, 10, Frac::new(2, 7));
                let mut state = audio_project(rate, sr, stereo, &["a1", "a2", "a3"]);
                add_audio(&mut state, "main", "a1", &main, pan);
                add_audio(&mut state, "pre", "a2", &pre, pan);
                add_audio(&mut state, "cross", "a3", &cross, pan);
                add_audio(&mut state, "post", "a2", &post, pan);
                let label = format!("{rate:?} {sr} Hz stereo={stereo}");
                let whole = [
                    ("main", main),
                    ("pre", pre),
                    ("cross", cross),
                    ("post", post),
                ];
                let before = check(&state, 0, LENGTH, &whole, &format!("{label} whole"));
                for cut in [4, 11, 14, 26] {
                    let tag = format!("{label} cut {cut}");
                    split(&mut state, "main", cut);
                    let left = main.piece(3, cut);
                    let right = main.piece(cut, 27);
                    assert_eq!(ss_of(&state, "main"), left.ss, "{tag}");
                    assert_eq!(ss_of(&state, "main-split-1"), right.ss, "{tag}");
                    let halves = [
                        ("main", left),
                        ("main-split-1", right),
                        ("pre", pre),
                        ("cross", cross),
                        ("post", post),
                    ];
                    let after = check(&state, 0, LENGTH, &halves, &tag);
                    same_bits(&before, &after, &tag);
                    run(&mut state, json!({"op":"history.undo"}));
                    same_bits(&before, &mix(&state), &format!("{tag} undone"));
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 6 * 2 * 2 * 4);
}

#[test]
fn the_special_grids_that_the_old_guard_refused_are_cut_at_every_frame() {
    // NTSC cut on multiples of 5 (a whole number of output samples per five frames) and 44.1 kHz at
    // 24 fps (1837.5 source samples per frame): the old guard refused both, in any layout.
    let cases: [((u32, u32), u32, Vec<i64>); 3] = [
        ((30_000, 1001), 48_000, vec![5, 10, 15, 20, 25, 30, 35]),
        ((24_000, 1001), 44_100, vec![5, 10, 15, 20, 25, 30, 35]),
        ((24, 1), 44_100, (1..40).collect()),
    ];
    for (rate, sr, cuts) in cases {
        let main = Spec::audio(rate, sr, true, 0, 40, Frac::int(0));
        let mut state = audio_project(rate, sr, true, &["a1"]);
        add_audio(&mut state, "main", "a1", &main, None);
        let before = check(&state, 0, LENGTH, &[("main", main)], "whole");
        for cut in cuts {
            let tag = format!("{rate:?} {sr} Hz cut {cut}");
            split(&mut state, "main", cut);
            let halves = [
                ("main", main.piece(0, cut)),
                ("main-split-1", main.piece(cut, 40)),
            ];
            let after = check(&state, 0, LENGTH, &halves, &tag);
            same_bits(&before, &after, &tag);
            run(&mut state, json!({"op":"history.undo"}));
        }
    }
}

// ---- the sound of a video: speed, offset and several streams ---------------------------------

#[test]
fn video_sound_keeps_the_phase_with_speed_offset_and_two_streams() {
    let off_a = Ratio::new(7, 1000).unwrap();
    let off_b = Ratio::new(1, 50).unwrap();
    let mut cases = 0;
    for rate in [(24_000, 1001), (30_000, 1001), (25, 1)] {
        for speed in [50, 100, 150, 200] {
            for first_offset in [Ratio::ZERO, off_a] {
                // Stream 0: 44.1 kHz stereo; stream 1: 48 kHz mono, 20 ms late.
                let streams = [(44_100, true, first_offset), (48_000, false, off_b)];
                for stream in [0u32, 1] {
                    let (sr, stereo, offset) = streams[stream as usize];
                    let mut main = Spec::audio(rate, sr, stereo, 5, 24, Frac::new(1, 4));
                    (main.speed, main.offset, main.audio_clip) = (speed, frac(offset), false);
                    let mut state = video_project(rate, &streams);
                    add_video(
                        &mut state,
                        "main",
                        &main,
                        stream,
                        if stereo { 0 } else { -100 },
                    );
                    let label =
                        format!("{rate:?} speed {speed} offset {first_offset} stream {stream}");
                    let before = check(&state, 0, LENGTH, &[("main", main)], &label);
                    for cut in [6, 17, 28] {
                        let tag = format!("{label} cut {cut}");
                        split(&mut state, "main", cut);
                        let (left, right) = (main.piece(5, cut), main.piece(cut, 29));
                        assert_eq!(ss_of(&state, "main-split-1"), right.ss, "{tag}");
                        let after = check(
                            &state,
                            0,
                            LENGTH,
                            &[("main", left), ("main-split-1", right)],
                            &tag,
                        );
                        same_bits(&before, &after, &tag);
                        run(&mut state, json!({"op":"history.undo"}));
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 3 * 4 * 2 * 2 * 3);
}
