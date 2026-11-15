use super::*;

#[test]
fn an_audio_cut_in_any_part_of_the_ramps_mixes_the_same_bits_on_every_grid() {
    let mut mixer = OfflineMixer::default();
    for rate in RATES {
        for (sample_rate, stereo) in [(44_100, true), (48_000, false)] {
            for (fade_in, fade_out) in AUDIO_FADES {
                let mut reference = audio_project(rate, sample_rate, stereo);
                set(&mut reference, "main", "fade_in", json!(fade_in));
                set(&mut reference, "main", "fade_out", json!(fade_out));
                let whole = mix_with(&mut mixer, &reference, 0, LENGTH);
                assert!(whole.peak() > 0.01, "the source is audible");
                for cut in cut_frames(3, 63, fade_in, fade_out) {
                    let label =
                        format!("{rate:?} {sample_rate} {stereo} {fade_in}/{fade_out} cut {cut}");
                    let mut state = reference.clone();
                    split(&mut state, "main", cut);
                    same_bits(&whole, &mix_with(&mut mixer, &state, 0, LENGTH), &label);
                    // Blocks that end just after the cut and start just before it are the
                    // whole's blocks (the export mixes in blocks).
                    for (from, frames) in [(cut - 2, 5), (cut + 1, 30)] {
                        let cut_block = mix_with(&mut mixer, &state, from, frames);
                        let whole_block = mix_with(&mut mixer, &reference, from, frames);
                        same_bits(
                            &whole_block,
                            &cut_block,
                            &format!("{label} block {from}+{frames}"),
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn an_audio_cut_that_loses_the_ramp_of_the_original_clip_is_seen() {
    for (fade_in, fade_out, cut) in [
        (12, 0, 8),
        (0, 12, 57),
        (12, 24, 10),
        (30, 30, 30),
        (44, 0, 40),
    ] {
        let mut reference = audio_project((30_000, 1001), 44_100, true);
        set(&mut reference, "main", "fade_in", json!(fade_in));
        set(&mut reference, "main", "fade_out", json!(fade_out));
        let whole = mix(&reference);
        let mut state = reference.clone();
        split(&mut state, "main", cut);
        same_bits(&whole, &mix(&state), "with the window");
        drop_windows(&mut state);
        assert!(
            first_difference(&whole, &mix(&state)).is_some(),
            "{fade_in}/{fade_out} at {cut}: dropping the window must change the sound"
        );
    }
}

#[test]
fn overlapping_audio_fades_keep_their_triangle_across_any_cut() {
    // 40 + 40 frames on a 60-frame clip: the mixer shortens them to 30 + 30 (a triangle). Both
    // halves keep both fades and the whole window, so the same triangle survives any cut.
    let mut reference = audio_project((24, 1), 48_000, true);
    set(&mut reference, "main", "fade_in", json!(40));
    set(&mut reference, "main", "fade_out", json!(40));
    let whole = mix(&reference);
    for cut in 4..62 {
        let mut state = reference.clone();
        split(&mut state, "main", cut);
        same_bits(&whole, &mix(&state), &format!("cut {cut}"));
        assert_eq!(number(&state, "main", "fade_in"), 40.0);
        assert_eq!(number(&state, "main", "fade_out"), 40.0);
        assert_eq!(number(&state, "main-split-1", "fade_in"), 40.0);
        assert_eq!(number(&state, "main-split-1", "fade_out"), 40.0);
    }
}

#[test]
fn the_sound_of_a_video_never_follows_its_visual_fades_across_a_cut() {
    use tessel_editor_core::assets::{AudioStream, Decoded, DecodedAudio};
    use tessel_editor_core::time::Ratio;
    let mut state = create((30_000, 1001));
    let video = shared_dir().join("fake.mp4");
    fs::write(&video, b"not decoded by the core").unwrap();
    run(
        &mut state,
        json!({"op":"asset.add","kind":"video","id":"vid","name":"Clip","path":video,
            "width":64,"height":36,"rate":24,"duration":4}),
    );
    let item = state
        .library
        .items
        .iter_mut()
        .find(|item| item.id == "vid")
        .unwrap();
    let asset = item.asset.as_mut().unwrap();
    asset.meta.audio_streams = vec![AudioStream {
        index: 0,
        sample_rate: 44_100,
        channels: 2,
        offset: Ratio::new(1, 8).unwrap(),
    }];
    asset.meta.sample_rate = Some(44_100);
    asset.decoded = Some(Decoded {
        frames: None,
        audio: vec![DecodedAudio {
            stream: 0,
            path: ramp(44_100, true).to_string_lossy().into_owned(),
        }],
        proxy: None,
    });
    run(
        &mut state,
        json!({"op":"track.add","id":"v","name":"V","kind":"video"}),
    );
    run(
        &mut state,
        json!({"op":"clip.add","id":"cv","track":"v","asset":"vid","start":3,"duration":60,
            "source_start":{"num":1,"den":4},"properties":{"speed":150}}),
    );
    let without_fades = mix(&state);
    assert!(
        without_fades.peak() > 0.01,
        "the video source contains sound"
    );
    set(&mut state, "cv", "fade_in", json!(12));
    set(&mut state, "cv", "fade_out", json!(24));
    same_bits(
        &without_fades,
        &mix(&state),
        "visual fades never reach the sound",
    );
    for cut in [5, 20, 45, 60] {
        let mut cut_state = state.clone();
        split(&mut cut_state, "cv", cut);
        same_bits(&without_fades, &mix(&cut_state), &format!("cut {cut}"));
    }
    // Their picture does keep the envelope across the same cuts, at 150 % speed.
    let before = plans(&mut state, 0..LENGTH);
    split(&mut state, "cv", 8);
    assert_eq!(
        first_plan_difference(&before, &plans(&mut state, 0..LENGTH)),
        None
    );
}

// ---- repeated cuts, history, files ---------------------------------------------------------
