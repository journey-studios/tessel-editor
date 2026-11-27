//! BL081.2.2: editing one fade of a cut sound redefines only that ramp.
//!
//! The reported bug: a 24 fps, 48 kHz sound `3..63` with `fade_out` 30 and `fade_in` 0, cut at
//! 40, gave its right half a window of 37 frames that only kept the length `resolve_fades`
//! normalizes against. Editing the half's `fade_in` to 5 dropped that window, the mixer cut the
//! ramp it did not edit from 30 to 20 frames, and the end of the sound changed. The checks
//! below look at the plan (the ramp's length and where it ends) and at the PCM **outside the
//! ramp that was edited**, bit for bit; the controls run the previous model's outcome (the
//! window dropped) and must fail the same checks.
use super::ramp_support::*;
use super::*;

#[test]
fn the_reported_right_half_keeps_its_fade_out_when_the_fade_in_is_edited() {
    let reference = whole((24, 1), 48_000, true, 0, 30);
    let original = mix(&reference);
    let ref_seg = segment(&reference, "main");
    let mut state = cut(&reference, "main", 40);
    // The cut itself: the right half carries a ramp that reaches before it.
    let right = segment(&state, "main-split-1");
    assert_eq!(
        (right.fade_out, right.out_anchor()),
        (30 * 2000, ref_seg.out_anchor())
    );
    set(&mut state, "main-split-1", "fade_in", json!(5));

    let right = segment(&state, "main-split-1");
    assert_eq!(right.fade_out, ref_seg.fade_out, "30 frames, as the whole");
    assert_eq!(
        right.out_anchor(),
        ref_seg.out_anchor(),
        "ends where it ended"
    );
    assert_eq!(
        right.fade_in,
        5 * 2000,
        "the edit is measured from the half's edge"
    );
    assert_eq!(right.in_anchor(), 40 * 2000);
    let edited = mix(&state);
    let (cut_at, end) = (at((24, 1), 40), at((24, 1), LENGTH));
    assert_same(&original, &edited, 0, cut_at, "the left half");
    let new_end = (40 + 5) * 2000;
    assert_same(&original, &edited, new_end, end, "the end of the sound");
    assert_eq!(
        edited.sample(cut_at, 0),
        0.0,
        "the new fade-in starts from silence"
    );
    assert!(
        differs(&original, &edited, cut_at, new_end),
        "the new ramp is heard"
    );

    // Control: the previous model (the window dropped, both ramps on the half) shortens the
    // ramp that was not edited, and the same checks see it.
    let old = without_windows(&state);
    let old_right = segment(&old, "main-split-1");
    // The previous model's arithmetic, written out: its window was gone after the edit, so the
    // 5 and the 30 frames were shortened in proportion to fit the 23-frame half.
    let previous = tessel_editor_core::audio::resolve_fades(5 * 2000, 30 * 2000, 23 * 2000);
    assert_eq!(previous, (6571, 39429), "about 20 frames, not 30");
    assert_eq!(old_right.fade_out, previous.1);
    assert_ne!(old_right.fade_out, ref_seg.fade_out);
    assert!(
        differs(&original, &mix(&old), new_end, end),
        "the end of the sound moved"
    );
}

#[test]
fn the_symmetric_left_half_keeps_its_fade_in_when_the_fade_out_is_edited() {
    let reference = whole((24, 1), 48_000, true, 30, 0);
    let original = mix(&reference);
    let ref_seg = segment(&reference, "main");
    let mut state = cut(&reference, "main", 20);
    set(&mut state, "main", "fade_out", json!(5));

    let left = segment(&state, "main");
    assert_eq!(left.fade_in, ref_seg.fade_in, "30 frames, as the whole");
    assert_eq!(
        left.in_anchor(),
        ref_seg.in_anchor(),
        "begins where it began"
    );
    assert_eq!(left.fade_out, 5 * 2000);
    assert_eq!(left.out_anchor(), 20 * 2000);
    let edited = mix(&state);
    let (cut_at, end) = (at((24, 1), 20), at((24, 1), LENGTH));
    let ramp = (20 - 5) * 2000;
    assert_same(&original, &edited, 0, ramp, "the start of the sound");
    assert_same(&original, &edited, cut_at, end, "the right half");
    assert!(
        differs(&original, &edited, ramp, cut_at),
        "the new ramp is heard"
    );
    let last = cut_at - 1;
    assert!(
        edited.sample(last, 0).abs() < original.sample(last, 0).abs() / 1000.0,
        "the sound fades to the end of the half"
    );

    let old = without_windows(&state);
    let old_left = segment(&old, "main");
    // 30 + 5 frames on the 17-frame left half: the previous model shortened the fade-in.
    let previous = tessel_editor_core::audio::resolve_fades(30 * 2000, 5 * 2000, 17 * 2000);
    assert_eq!(old_left.fade_in, previous.0);
    assert_ne!(
        old_left.fade_in, ref_seg.fade_in,
        "the previous model changed it"
    );
    assert!(
        differs(&original, &mix(&old), 0, ramp),
        "and the sound with it"
    );
}

/// Edits the fade-in of the right half to `k` and checks everything but the ramp it edited.
fn check_right_edit(
    reference: &EditorState,
    cut_at: i64,
    k: i64,
    rate: (u32, u32),
    mixer: &mut OfflineMixer,
    label: &str,
) -> bool {
    let original = mix_with(mixer, reference, 0, LENGTH);
    let ref_seg = segment(reference, "main");
    let mut state = cut(reference, "main", cut_at);
    set(&mut state, "main-split-1", "fade_in", json!(k));
    let right = segment(&state, "main-split-1");
    assert_eq!(right.fade_out, ref_seg.fade_out, "{label}: fade_out");
    assert_eq!(
        right.out_anchor(),
        ref_seg.out_anchor(),
        "{label}: where it ends"
    );
    assert_eq!(
        right.in_anchor(),
        at(rate, cut_at) as u64,
        "{label}: the new fade-in"
    );
    let edited = mix_with(mixer, &state, 0, LENGTH);
    let (cut_s, end) = (at(rate, cut_at), at(rate, LENGTH));
    assert_same(&original, &edited, 0, cut_s, &format!("{label}: left half"));
    // The old fade-in of the whole (if any) is handed over with the edit: compare beyond both.
    let from = (right.in_end().max(ref_seg.in_end()) as usize).min(end);
    assert_same(
        &original,
        &edited,
        from,
        end,
        &format!("{label}: beyond the ramps"),
    );
    // The edit is the ramp it says, from the half's edge: silent first, then gain `at / len`.
    assert_eq!(edited.sample(cut_s, 0), 0.0, "{label}");
    for n in (cut_s..right.in_end() as usize).step_by(7) {
        let Some(ratio) = gain_ratio(&edited, &original, n) else {
            continue;
        };
        let expected = ref_seg.fade_in as f32 + (n - cut_s) as f32 / right.fade_in as f32;
        let expected = expected - ref_seg.fade_in as f32;
        // The whole has no fade-in here when `ref_seg.fade_in == 0`: the ratio is the ramp.
        if ref_seg.fade_in == 0 {
            assert!(
                (ratio - expected).abs() < 2e-4,
                "{label}: sample {n}: {ratio} vs {expected}"
            );
        }
    }
    // Control: the window dropped (the previous model) changes the sound the edit left alone.
    differs(
        &original,
        &mix_with(mixer, &without_windows(&state), 0, LENGTH),
        from,
        end,
    )
}

#[test]
fn editing_the_fade_in_of_a_right_half_never_moves_its_fade_out() {
    let mut mixer = OfflineMixer::default();
    let mut seen_by_the_control = 0;
    for rate in RATES {
        for (sample_rate, stereo) in CONFIGS {
            for fade_out in [30, 12] {
                let reference = whole(rate, sample_rate, stereo, 0, fade_out);
                for cut_at in [20, 33, 34, 40, 50, 57] {
                    for k in [1, 5, 23, 48] {
                        let label = format!(
                            "{rate:?} {sample_rate} {stereo} out {fade_out} cut {cut_at} in {k}"
                        );
                        seen_by_the_control += usize::from(check_right_edit(
                            &reference, cut_at, k, rate, &mut mixer, &label,
                        ));
                    }
                }
            }
        }
    }
    assert!(
        seen_by_the_control > 100,
        "the previous model must fail this check in many of the cases ({seen_by_the_control})"
    );
}

#[test]
fn editing_the_fade_out_of_a_left_half_never_moves_its_fade_in() {
    let mut mixer = OfflineMixer::default();
    let mut seen_by_the_control = 0;
    for rate in RATES {
        for (sample_rate, stereo) in CONFIGS {
            for fade_in in [30, 12] {
                let reference = whole(rate, sample_rate, stereo, fade_in, 0);
                let original = mix_with(&mut mixer, &reference, 0, LENGTH);
                let ref_seg = segment(&reference, "main");
                for cut_at in [10, 17, 20, 26, 32, 40] {
                    for k in [1, 5, 23, 48] {
                        let label = format!(
                            "{rate:?} {sample_rate} {stereo} in {fade_in} cut {cut_at} out {k}"
                        );
                        let mut state = cut(&reference, "main", cut_at);
                        set(&mut state, "main", "fade_out", json!(k));
                        let left = segment(&state, "main");
                        assert_eq!(left.fade_in, ref_seg.fade_in, "{label}: fade_in");
                        assert_eq!(
                            left.in_anchor(),
                            ref_seg.in_anchor(),
                            "{label}: where it begins"
                        );
                        assert_eq!(left.out_anchor(), at(rate, cut_at) as u64, "{label}");
                        let edited = mix_with(&mut mixer, &state, 0, LENGTH);
                        let (cut_s, end) = (at(rate, cut_at), at(rate, LENGTH));
                        let ramp = left.out_start() as usize;
                        assert_same(
                            &original,
                            &edited,
                            0,
                            ramp,
                            &format!("{label}: before the ramp"),
                        );
                        assert_same(
                            &original,
                            &edited,
                            cut_s,
                            end,
                            &format!("{label}: right half"),
                        );
                        // The ramp ends the half: `remaining / len`, from the half's edge.
                        for n in (ramp..cut_s).step_by(5) {
                            if let Some(ratio) = gain_ratio(&edited, &original, n) {
                                let expected = (cut_s - n) as f32 / left.fade_out as f32;
                                assert!(
                                    (ratio - expected).abs() < 2e-4,
                                    "{label}: {n}: {ratio} vs {expected}"
                                );
                            }
                        }
                        let control = mix_with(&mut mixer, &without_windows(&state), 0, LENGTH);
                        seen_by_the_control += usize::from(differs(&original, &control, 0, ramp));
                    }
                }
            }
        }
    }
    assert!(
        seen_by_the_control > 100,
        "the control sees it ({seen_by_the_control})"
    );
}

#[test]
fn overlapping_originals_keep_the_ramp_that_was_not_edited_as_the_whole_shortened_it() {
    // 40 + 40 on 60 frames: the whole mixes 30 + 30. Whatever half is edited, the other ramp
    // stays one of 30 frames, in its place, even where the half alone would shorten it more.
    let mut mixer = OfflineMixer::default();
    let mut seen_by_the_control = 0;
    for rate in RATES {
        for (sample_rate, stereo) in CONFIGS {
            let reference = whole(rate, sample_rate, stereo, 40, 40);
            let original = mix_with(&mut mixer, &reference, 0, LENGTH);
            let ref_seg = segment(&reference, "main");
            assert_eq!(
                (ref_seg.fade_in as f64 / ref_seg.fade_out as f64).round(),
                1.0
            );
            for cut_at in [10, 25, 33, 40, 50, 58] {
                let label = format!("{rate:?} {sample_rate} {stereo} cut {cut_at}");
                let end = at(rate, LENGTH);
                // The right half edits its fade-in: the fade-out is the whole's.
                let mut right = cut(&reference, "main", cut_at);
                set(&mut right, "main-split-1", "fade_in", json!(5));
                assert_eq!(number(&right, "main-split-1", "fade_out"), 40.0, "{label}");
                let plan = segment(&right, "main-split-1");
                assert_eq!(
                    (plan.fade_out, plan.out_anchor()),
                    (ref_seg.fade_out, ref_seg.out_anchor()),
                    "{label}"
                );
                let edited = mix_with(&mut mixer, &right, 0, LENGTH);
                assert_same(
                    &original,
                    &edited,
                    0,
                    at(rate, cut_at),
                    &format!("{label}: left"),
                );
                let from = (plan.in_end().max(ref_seg.in_end()) as usize).min(end);
                assert_same(&original, &edited, from, end, &format!("{label}: right"));
                let control = mix_with(&mut mixer, &without_windows(&right), 0, LENGTH);
                seen_by_the_control += usize::from(differs(&original, &control, from, end));

                // The left half edits its fade-out: the fade-in is the whole's.
                let mut left = cut(&reference, "main", cut_at);
                set(&mut left, "main", "fade_out", json!(5));
                assert_eq!(number(&left, "main", "fade_in"), 40.0, "{label}");
                let plan = segment(&left, "main");
                assert_eq!(
                    (plan.fade_in, plan.in_anchor()),
                    (ref_seg.fade_in, ref_seg.in_anchor()),
                    "{label}"
                );
                let edited = mix_with(&mut mixer, &left, 0, LENGTH);
                let to = (plan.out_start().min(ref_seg.out_start())) as usize;
                assert_same(&original, &edited, 0, to, &format!("{label}: left"));
                assert_same(
                    &original,
                    &edited,
                    at(rate, cut_at),
                    end,
                    &format!("{label}: right"),
                );
                let control = mix_with(&mut mixer, &without_windows(&left), 0, LENGTH);
                seen_by_the_control += usize::from(differs(&original, &control, 0, to));
            }
        }
    }
    assert!(
        seen_by_the_control > 20,
        "the control sees it ({seen_by_the_control})"
    );
}

#[test]
fn the_one_edit_never_zeroes_the_other_side_to_get_around_the_coupling() {
    let reference = whole((24, 1), 48_000, true, 0, 30);
    let mut state = cut(&reference, "main", 40);
    let left = window_of(&state, "main");
    set(&mut state, "main-split-1", "fade_in", json!(5));
    assert_eq!(
        number(&state, "main-split-1", "fade_out"),
        30.0,
        "the property stays"
    );
    assert_eq!(
        number(&state, "main", "fade_out"),
        30.0,
        "the left half keeps it"
    );
    assert_eq!(
        window_of(&state, "main"),
        left,
        "the left half is untouched"
    );
    let window = window_of(&state, "main-split-1");
    assert!(window.tail.is_some(), "the inherited ramp is still pinned");
    assert!(window.lead.is_none(), "nothing of the edited one is left");
}

#[test]
fn a_whole_clip_and_a_plain_half_keep_the_legacy_ramps_bit_for_bit() {
    use tessel_editor_core::audio::resolve_fades;
    for rate in RATES {
        for (fade_in, fade_out) in [
            (12, 0),
            (0, 12),
            (12, 24),
            (30, 30),
            (40, 40),
            (48, 30),
            (2, 3),
        ] {
            let state = whole(rate, 44_100, true, fade_in, fade_out);
            let seg = segment(&state, "main");
            // The legacy rule, written out: resolve both on the clip, on the absolute clock.
            let (first, last) = (at(rate, 3) as i64, at(rate, 63) as i64);
            let ramp_in = at(rate, 3 + fade_in) as i64 - first;
            let ramp_out = last - at(rate, 63 - fade_out) as i64;
            let (expected_in, expected_out) =
                resolve_fades(ramp_in as u64, ramp_out as u64, (last - first) as u64);
            let label = format!("{rate:?} {fade_in}/{fade_out}");
            assert_eq!(
                (seg.fade_in, seg.fade_out),
                (expected_in, expected_out),
                "{label}"
            );
            assert_eq!(
                (seg.clip_offset, seg.clip_frames),
                (0, (last - first) as u64),
                "{label}"
            );
        }
    }
    // A half that no fade the sliders allow can reach into the other ramp needs no window and
    // is the whole, bit for bit: a 92-frame clip, 12 frames of fade-in, cut at 65.
    for rate in RATES {
        let mut reference = whole(rate, 48_000, true, 12, 0);
        reference
            .execute(Command::TrimClip {
                clip: "main".into(),
                edge: TrimEdge::End,
                frame: 95,
            })
            .unwrap();
        let state = cut(&reference, "main", 65);
        assert!(
            window_of(&state, "main").is_empty(),
            "{rate:?}: no window needed"
        );
        assert!(window_of(&state, "main-split-1").is_empty());
        assert_same(
            &mix(&reference),
            &mix(&state),
            0,
            at(rate, LENGTH),
            &format!("{rate:?}"),
        );
    }
}
