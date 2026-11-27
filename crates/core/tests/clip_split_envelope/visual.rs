use super::*;

#[test]
fn a_visual_cut_in_any_part_of_the_ramps_keeps_every_frame() {
    for rate in [(24, 1), (30_000, 1001)] {
        for (fade_in, fade_out) in VISUAL_FADES {
            // The envelope is counted in frames: one grid is exhaustive, the other samples.
            for cut in (7..65).filter(|cut| rate.1 == 1 || cut % 7 == 0 || *cut < 12) {
                let mut state = visual_project(rate);
                set(&mut state, "t", "fade_in", json!(fade_in));
                set(&mut state, "t", "fade_out", json!(fade_out));
                let before = plans(&mut state, 4..68);
                split(&mut state, "t", cut);
                let after = plans(&mut state, 4..68);
                assert_eq!(
                    first_plan_difference(&before, &after),
                    None,
                    "rate {rate:?}, fades {fade_in}/{fade_out}, cut {cut}"
                );
            }
        }
    }
}

#[test]
fn a_cut_that_loses_the_ramp_of_the_original_clip_is_seen() {
    // Control of the test above: the model without the window (what a half would be if it
    // measured the ramps on its own edges) restarts the envelope at the cut.
    for (fade_in, fade_out, cut) in [(12, 0, 10), (0, 24, 50), (12, 24, 10), (40, 40, 30)] {
        let mut state = visual_project((24, 1));
        set(&mut state, "t", "fade_in", json!(fade_in));
        set(&mut state, "t", "fade_out", json!(fade_out));
        let before = plans(&mut state, 0..LENGTH);
        split(&mut state, "t", cut);
        drop_windows(&mut state);
        let after = plans(&mut state, 0..LENGTH);
        assert!(
            first_plan_difference(&before, &after).is_some(),
            "{fade_in}/{fade_out} at {cut}: dropping the window must change a frame"
        );
    }
}

#[test]
fn loaded_fractional_visual_fades_cut_inside_the_ramp_keep_every_frame() {
    for (fade_in, fade_out) in [
        (1.4, 0.0),
        (10.5, 0.0),
        (0.0, 2.4),
        (0.0, 30.5),
        (7.25, 9.75),
    ] {
        for cut in 7..65 {
            let mut doc = visual_project((24, 1)).to_document(None);
            doc["clip_properties"]["t"]["fade_in"] = json!(fade_in);
            doc["clip_properties"]["t"]["fade_out"] = json!(fade_out);
            let mut state = EditorState::from_document(&doc, None).unwrap();
            assert_eq!(number(&state, "t", "fade_in"), fade_in);
            let before = plans(&mut state, 4..68);
            split(&mut state, "t", cut);
            assert_eq!(
                first_plan_difference(&before, &plans(&mut state, 4..68)),
                None,
                "fades {fade_in}/{fade_out}, cut {cut}"
            );
        }
    }
}

#[test]
fn a_cut_outside_the_ramps_is_what_it_always_was() {
    // The ramps do not reach the right half (in) and the left half (out): they are zeroed and
    // no window is written, so the document of such a cut is the one of the previous model.
    let mut state = visual_project((24, 1));
    set(&mut state, "t", "fade_in", json!(6));
    set(&mut state, "t", "fade_out", json!(12));
    let outcome = split(&mut state, "t", 36);
    assert_eq!(outcome["adjusted"].as_array().unwrap().len(), 2);
    assert_eq!(outcome["fade_window"], json!({"left": null, "right": null}));
    assert_eq!(number(&state, "t", "fade_in"), 6.0);
    assert_eq!(number(&state, "t", "fade_out"), 0.0);
    assert_eq!(number(&state, "t-split-1", "fade_in"), 0.0);
    assert_eq!(number(&state, "t-split-1", "fade_out"), 12.0);
    assert!(window_of(&state, "t").is_empty() && window_of(&state, "t-split-1").is_empty());
    assert!(
        state.to_document(None)["sequences"][0]["clips"]
            .as_array()
            .unwrap()
            .iter()
            .all(|clip| clip.get("fade_window").is_none())
    );
    assert_eq!(state.document_schema_version(), 1);
}

#[test]
fn a_cut_inside_a_ramp_keeps_both_fades_and_writes_the_window() {
    let mut state = visual_project((24, 1));
    set(&mut state, "t", "fade_in", json!(12));
    set(&mut state, "t", "fade_out", json!(24));
    let outcome = split(&mut state, "t", 10);
    // Fade-in: the left half owns the start of the ramp, the right half the rest (4 frames in).
    assert_eq!(number(&state, "t", "fade_in"), 12.0);
    assert_eq!(number(&state, "t-split-1", "fade_in"), 12.0);
    // Fade-out lies wholly in the right half.
    assert_eq!(number(&state, "t", "fade_out"), 0.0);
    assert_eq!(number(&state, "t-split-1", "fade_out"), 24.0);
    assert_eq!(
        outcome["fade_window"],
        json!({"left": null, "right": {"lead": {"frames": 4, "fade": 12.0}}})
    );
    assert_eq!(state.document_schema_version(), 5);
    // The refused-before frames: 11 = last still-ramping frame of the fade-in, 26 = frame whose
    // left half would still fade out. Both are accepted now.
    let mut state = visual_project((24, 1));
    set(&mut state, "t", "fade_in", json!(12));
    set(&mut state, "t", "fade_out", json!(24));
    split(&mut state, "t", 6 + 11);
    split(&mut state, "t-split-1", 6 + 60 - 23);
}

// ---- sound ----------------------------------------------------------------------------------

/// Fade sets of the sound: ramp in, ramp out, both, overlapping (the mixer shortens them), a
/// ramp longer than the half it lands on.

#[test]
fn several_cuts_in_and_between_ramps_keep_the_picture_and_the_sound() {
    let mut visual = visual_project((24, 1));
    set(&mut visual, "t", "fade_in", json!(24));
    set(&mut visual, "t", "fade_out", json!(24));
    let before = plans(&mut visual, 0..LENGTH);
    for (clip, cut) in [
        ("t", 12),
        ("t-split-1", 20),
        ("t", 8),
        ("t-split-1-split-1", 60),
        ("t-split-1", 16),
    ] {
        split(&mut visual, clip, cut);
        assert_eq!(
            first_plan_difference(&before, &plans(&mut visual, 0..LENGTH)),
            None,
            "{clip}@{cut}"
        );
    }
    assert_eq!(visual.timeline.clips.len(), 6);

    let mut audio = audio_project((30_000, 1001), 44_100, true);
    set(&mut audio, "main", "fade_in", json!(20));
    set(&mut audio, "main", "fade_out", json!(20));
    let whole = mix(&audio);
    for (clip, cut) in [
        ("main", 10),
        ("main-split-1", 25),
        ("main", 6),
        ("main-split-1-split-1", 55),
        ("main-split-1", 18),
    ] {
        split(&mut audio, clip, cut);
        same_bits(&whole, &mix(&audio), &format!("{clip}@{cut}"));
    }
}
