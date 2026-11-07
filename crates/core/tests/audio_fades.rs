//! BL142 (F051): `fade_in` / `fade_out` of an audio clip, through the public library API and
//! the JSON executor (no window). The signal is a constant 0.5 on the left channel (mono source,
//! pan -100), so every output sample says exactly which gain the mixer applied to it.
//!
//! 24 fps over 48 kHz is 2000 samples per frame: positions are exact, never rounded.
use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};
use tessel_editor_core::audio::{AudioBackend, OfflineMixer, Pcm, encode_wav_pcm16, resolve_fades};
use tessel_editor_core::{EditorState, headless};

const SAMPLES_PER_FRAME: usize = 2000;
const LEVEL: f32 = 0.5;

fn ok(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn err(state: &mut EditorState, request: Value) -> &'static str {
    headless::execute(state, &request)
        .expect_err(&request.to_string())
        .code
}

fn dc_wav(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tessel-fades-{tag}-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("dc.wav");
    let pcm = Pcm {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![LEVEL; 48_000 * 6],
    };
    fs::write(&path, encode_wav_pcm16(&pcm)).unwrap();
    path
}

/// 24 fps, 120 frames; an audio clip `c` at frame 12 lasting 48 frames, panned hard left.
fn project(tag: &str) -> EditorState {
    let mut state = EditorState::demo();
    ok(
        &mut state,
        json!({"op":"project.create","name":"Fades","sequence":"main","rate":24,
            "width":64,"height":36,"length":120}),
    );
    ok(
        &mut state,
        json!({"op":"asset.add","kind":"audio","id":"dc","name":"DC","path":dc_wav(tag)}),
    );
    ok(
        &mut state,
        json!({"op":"track.add","id":"a","name":"Audio","kind":"audio"}),
    );
    ok(
        &mut state,
        json!({"op":"clip.add","id":"c","track":"a","asset":"dc","start":12,"duration":48,
            "properties":{"pan":-100}}),
    );
    state
}

fn fade(state: &mut EditorState, property: &str, frames: i64) -> Value {
    ok(
        state,
        json!({"op":"clip.property","clip":"c","property":property,"value":frames}),
    )
}

fn mix(state: &EditorState, from: i64, frames: i64) -> Pcm {
    let plan = state.plan_audio(from, frames).unwrap();
    OfflineMixer::default().mix(&plan).unwrap()
}

fn left(pcm: &Pcm, at: usize) -> f32 {
    pcm.sample(at, 0)
}

fn near(actual: f32, expected: f32) -> bool {
    (actual - expected).abs() < 2e-4
}

fn rms(pcm: &Pcm, from: usize, to: usize) -> f64 {
    let sum: f64 = (from..to).map(|at| f64::from(left(pcm, at)).powi(2)).sum();
    (sum / (to - from) as f64).sqrt()
}

#[test]
fn a_fade_in_and_out_shape_the_clip_with_exact_samples() {
    let mut state = project("shape");
    fade(&mut state, "fade_in", 12);
    fade(&mut state, "fade_out", 24);
    let pcm = mix(&state, 0, 120);
    let clip = 12 * SAMPLES_PER_FRAME;
    assert_eq!(left(&pcm, clip - 1), 0.0, "silence before the clip");
    assert_eq!(left(&pcm, clip), 0.0, "the fade starts from silence");
    // Half way up the 12-frame (24000 sample) ramp.
    assert!(near(left(&pcm, clip + 12_000), LEVEL * 0.5));
    // The ramp is done at its last sample and the plateau is untouched.
    assert!(near(left(&pcm, clip + 24_000), LEVEL));
    assert!(near(left(&pcm, clip + 40_000), LEVEL));
    // The 24-frame (48000 sample) fade-out starts 48000 samples before the end.
    assert!(near(left(&pcm, clip + 96_000 - 48_000), LEVEL));
    assert!(near(left(&pcm, clip + 96_000 - 24_000), LEVEL * 0.5));
    assert!(near(left(&pcm, clip + 95_999), LEVEL / 48_000.0));
    assert_eq!(left(&pcm, clip + 96_000), 0.0, "silence after the clip");
    // The right channel stays empty with the pan hard left.
    assert_eq!(pcm.sample(clip + 40_000, 1), 0.0);
}

#[test]
fn rms_follows_the_ramp_and_a_clip_without_fades_is_untouched() {
    let mut state = project("rms");
    let plain = mix(&state, 0, 120);
    let clip = 12 * SAMPLES_PER_FRAME;
    // The whole clip at a constant level: RMS equals the level.
    assert!((rms(&plain, clip, clip + 96_000) - f64::from(LEVEL)).abs() < 1e-4);
    fade(&mut state, "fade_in", 12);
    let faded = mix(&state, 0, 120);
    // A linear 0..1 ramp has RMS 1/sqrt(3) of the level.
    let ramp = rms(&faded, clip, clip + 24_000);
    assert!(
        (ramp - f64::from(LEVEL) / 3f64.sqrt()).abs() < 2e-3,
        "ramp rms {ramp}"
    );
    // Outside the ramp nothing moved, sample for sample.
    for at in (clip + 24_000..clip + 96_000).step_by(997) {
        assert_eq!(left(&faded, at), left(&plain, at), "sample {at}");
    }
    // Fades set back to zero give back the exact original mix.
    fade(&mut state, "fade_in", 0);
    assert_eq!(mix(&state, 0, 120).samples, plain.samples);
    let segment = &state.plan_audio(0, 120).unwrap().segments[0];
    assert_eq!((segment.fade_in, segment.fade_out), (0, 0));
}

#[test]
fn a_mix_in_chunks_is_the_mix_in_one_piece_even_when_a_fade_crosses_the_cuts() {
    let mut state = project("chunks");
    fade(&mut state, "fade_in", 20);
    fade(&mut state, "fade_out", 30);
    let whole = mix(&state, 0, 120);
    // Chunks of 7 frames: every boundary falls inside a fade or the plateau at some point.
    let mut from = 0;
    while from < 120 {
        let frames = 7.min(120 - from);
        let chunk = mix(&state, from, frames);
        let base = from as usize * SAMPLES_PER_FRAME;
        for index in 0..chunk.frames() {
            assert!(
                (left(&chunk, index) - left(&whole, base + index)).abs() < 1e-6,
                "chunk at frame {from}, sample {index}"
            );
        }
        from += frames;
    }
}

#[test]
fn trimming_and_moving_the_clip_move_its_fades_with_its_edges() {
    let mut state = project("edges");
    fade(&mut state, "fade_in", 12);
    fade(&mut state, "fade_out", 12);
    // Trim the head by 6 frames: the clip now starts at frame 18 and the ramp starts there.
    ok(
        &mut state,
        json!({"op":"set","target":"trim:c:start","field":"frame","value":18}),
    );
    let pcm = mix(&state, 0, 120);
    let start = 18 * SAMPLES_PER_FRAME;
    assert_eq!(left(&pcm, start - 1), 0.0);
    assert_eq!(left(&pcm, start), 0.0);
    assert!(near(left(&pcm, start + 12_000), LEVEL * 0.5));
    assert!(near(left(&pcm, start + 24_000), LEVEL));
    // Trim the tail to frame 48: the ramp down now ends at the new last sample.
    ok(
        &mut state,
        json!({"op":"set","target":"trim:c:end","field":"frame","value":48}),
    );
    let pcm = mix(&state, 0, 120);
    let end = 48 * SAMPLES_PER_FRAME;
    assert!(near(left(&pcm, end - 24_000 - 1), LEVEL));
    assert!(near(left(&pcm, end - 12_000), LEVEL * 0.5));
    assert!(near(left(&pcm, end - 1), LEVEL / 24_000.0));
    assert_eq!(left(&pcm, end), 0.0);
    // Moving the clip by 10 frames carries both fades along.
    ok(
        &mut state,
        json!({"op":"move","target":"clip:c","args":{"frame":28}}),
    );
    let pcm = mix(&state, 0, 120);
    let start = 28 * SAMPLES_PER_FRAME;
    assert_eq!(left(&pcm, start), 0.0);
    assert!(near(left(&pcm, start + 12_000), LEVEL * 0.5));
    assert!(near(
        left(&pcm, 58 * SAMPLES_PER_FRAME - 12_000),
        LEVEL * 0.5
    ));
}

#[test]
fn fades_that_overlap_are_shortened_to_meet_at_full_gain() {
    // The pure rule.
    assert_eq!(resolve_fades(10, 10, 100), (10, 10));
    assert_eq!(resolve_fades(60, 60, 100), (50, 50));
    assert_eq!(resolve_fades(30, 90, 60), (15, 45));
    assert_eq!(resolve_fades(500, 0, 100), (100, 100 - 100));
    assert_eq!(resolve_fades(0, 500, 100), (0, 100));
    assert_eq!(resolve_fades(0, 0, 0), (0, 0));
    // Through the plan: 40 + 40 frames over a 48-frame clip become 24 + 24.
    let mut state = project("overlap");
    fade(&mut state, "fade_in", 40);
    fade(&mut state, "fade_out", 40);
    let segment = &state.plan_audio(0, 120).unwrap().segments[0];
    assert_eq!((segment.fade_in, segment.fade_out), (48_000, 48_000));
    let pcm = mix(&state, 0, 120);
    let clip = 12 * SAMPLES_PER_FRAME;
    // A triangle: it peaks at the middle and never multiplies into a dip.
    assert!(near(left(&pcm, clip + 48_000), LEVEL));
    assert!(near(left(&pcm, clip + 24_000), LEVEL * 0.5));
    assert!(near(left(&pcm, clip + 72_000), LEVEL * 0.5));
    let peak = (clip..clip + 96_000)
        .map(|at| left(&pcm, at))
        .fold(0.0f32, f32::max);
    assert!(near(peak, LEVEL), "peak {peak}");
}

#[test]
fn the_slider_range_is_the_extreme_and_bad_values_change_nothing() {
    let mut state = project("range");
    let answer = fade(&mut state, "fade_in", 500);
    assert_eq!(answer["value"], 48.0, "{answer}");
    assert_eq!(answer["clamped"], true);
    assert_eq!(fade(&mut state, "fade_in", -3)["value"], 0.0);
    let before = state.plan_audio(0, 120).unwrap().to_json();
    let last_edit = state.history.undo_label().map(str::to_string);
    assert_eq!(
        err(
            &mut state,
            json!({"op":"clip.property","clip":"c","property":"fade_out","value":"soon"})
        ),
        "invalid_value"
    );
    // The properties that are only visual still do not apply to a clip that is only sound.
    for property in ["zoom", "speed", "opacity"] {
        assert_eq!(
            err(
                &mut state,
                json!({"op":"clip.property","clip":"c","property":property,"value":50})
            ),
            "not_applicable",
            "{property}"
        );
    }
    assert_eq!(state.plan_audio(0, 120).unwrap().to_json(), before);
    assert_eq!(
        state.history.undo_label().map(str::to_string),
        last_edit,
        "rejections record nothing"
    );
}

#[test]
fn a_fade_is_one_undoable_edit_and_survives_save_and_reopen() {
    let mut state = project("undo");
    fade(&mut state, "fade_out", 24);
    let with = state.plan_audio(0, 120).unwrap().to_json();
    ok(&mut state, json!({"op":"history.undo"}));
    let segment = &state.plan_audio(0, 120).unwrap().segments[0];
    assert_eq!(segment.fade_out, 0);
    ok(&mut state, json!({"op":"history.redo"}));
    assert_eq!(state.plan_audio(0, 120).unwrap().to_json(), with);

    let dir = std::env::temp_dir().join(format!("tessel-fades-save-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("fades.tessel.json");
    ok(
        &mut state,
        json!({"op":"project.save","path":file.to_string_lossy()}),
    );
    let mut reopened = EditorState::demo();
    ok(
        &mut reopened,
        json!({"op":"project.open","path":file.to_string_lossy()}),
    );
    let segment = &reopened.plan_audio(0, 120).unwrap().segments[0];
    assert_eq!((segment.fade_in, segment.fade_out), (0, 48_000));
    assert_eq!(mix(&reopened, 0, 120).samples, mix(&state, 0, 120).samples);
}
