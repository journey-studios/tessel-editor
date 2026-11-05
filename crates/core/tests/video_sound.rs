//! The audio plan of a video's own sound: stream choice, per-clip mute, trim, speed and the stream's
//! offset on the asset clock, all decided by the core (the adapter only supplies decoded PCM).

use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};
use tessel_editor_core::assets::{AudioStream, Decoded, DecodedAudio};
use tessel_editor_core::audio::{AudioBackend, OfflineMixer, Pcm, encode_wav_pcm16};
use tessel_editor_core::time::Ratio;
use tessel_editor_core::{EditorState, headless};

fn temp(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("tessel-video-sound-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn ok(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

/// A ramp: sample `n` is `n / 100_000`, so any output sample says which source sample it read.
fn ramp_wav(dir: &std::path::Path, seconds: usize) -> PathBuf {
    let samples = (0..48_000 * seconds)
        .map(|n| n as f32 / 100_000.0)
        .collect();
    let path = dir.join("ramp.wav");
    let pcm = Pcm {
        sample_rate: 48_000,
        channels: 1,
        samples,
    };
    fs::write(&path, encode_wav_pcm16(&pcm)).unwrap();
    path
}

/// A project (24 fps, 4 s) with one container-video asset whose sound starts `offset` seconds
/// after its first picture, decoded to a ramp WAV, and a clip of it with `clip` extras.
fn project(dir: &std::path::Path, offset: Ratio, clip: Value) -> EditorState {
    let mut state = EditorState::demo();
    ok(
        &mut state,
        json!({"op":"project.create","name":"S","sequence":"main","rate":24,"width":64,"height":36,"length":96}),
    );
    let video = dir.join("fake.mp4");
    fs::write(&video, b"not decoded by the core").unwrap();
    ok(
        &mut state,
        json!({"op":"asset.add","kind":"video","id":"vid","name":"Clip","path":video,
            "width":64,"height":36,"rate":24,"duration":3}),
    );
    let wav = ramp_wav(dir, 3);
    let item = state
        .library
        .items
        .iter_mut()
        .find(|item| item.id == "vid")
        .unwrap();
    let asset = item.asset.as_mut().unwrap();
    asset.meta.audio_streams = vec![AudioStream {
        index: 0,
        sample_rate: 48_000,
        channels: 1,
        offset,
    }];
    asset.meta.sample_rate = Some(48_000);
    asset.decoded = Some(Decoded {
        frames: None,
        audio: vec![DecodedAudio {
            stream: 0,
            path: wav.to_string_lossy().into_owned(),
        }],
        proxy: None,
    });
    ok(
        &mut state,
        json!({"op":"track.add","id":"v","name":"V","kind":"video"}),
    );
    let mut request =
        json!({"op":"clip.add","id":"c","track":"v","asset":"vid","start":0,"duration":48});
    for (key, value) in clip.as_object().unwrap() {
        request[key] = value.clone();
    }
    ok(&mut state, request);
    state
}

fn mix(state: &EditorState, from: i64, frames: i64) -> Pcm {
    let plan = state.plan_audio(from, frames).unwrap();
    OfflineMixer::default().mix(&plan).unwrap()
}

/// Source sample `n` as the left channel of a mono source panned to the center.
fn centered(n: f64) -> f32 {
    (n / 100_000.0 * std::f64::consts::FRAC_1_SQRT_2) as f32
}

#[test]
fn a_video_clip_plays_its_own_sound_without_an_audio_track() {
    let dir = temp("own");
    let state = project(&dir, Ratio::ZERO, json!({}));
    assert!(
        state
            .timeline
            .tracks
            .iter()
            .all(|track| track.kind != tessel_editor_core::state::timeline::TrackKind::Audio),
        "the project has no audio track at all"
    );
    let plan = state.plan_audio(0, 48).unwrap();
    assert_eq!(plan.segments.len(), 1);
    let segment = &plan.segments[0];
    assert_eq!(
        (segment.source_start, segment.output_start, segment.frames),
        (0, 0, 96_000)
    );
    let sources = state.audio_sources(0, 48).unwrap();
    assert!(sources.len() == 1 && sources[0].embedded);
    let pcm = mix(&state, 0, 48);
    // Output sample 48 000 (1 s) is source sample 48 000 at the same rate.
    assert!((pcm.sample(48_000, 0) - centered(48_000.0)).abs() < 1e-3);
}

#[test]
fn trim_and_timeline_offset_move_the_source_position_exactly() {
    let dir = temp("trim");
    // Clip starts at 0.5 s on the timeline and reads from source 1 s.
    let state = project(
        &dir,
        Ratio::ZERO,
        json!({"start":12,"duration":24,"source_start":1}),
    );
    let plan = state.plan_audio(0, 48).unwrap();
    let segment = &plan.segments[0];
    assert_eq!(
        (segment.output_start, segment.source_start),
        (24_000, 48_000)
    );
    let pcm = mix(&state, 0, 48);
    assert_eq!(pcm.sample(10_000, 0), 0.0, "silence before the clip");
    assert!((pcm.sample(24_000, 0) - centered(48_000.0)).abs() < 1e-3);
    assert!((pcm.sample(30_000, 0) - centered(54_000.0)).abs() < 1e-3);
    // Asking for a later window of the same clip lands on the same source samples.
    let later = state.plan_audio(24, 24).unwrap();
    assert_eq!(later.segments[0].source_start, 48_000 + 24_000);
}

#[test]
fn the_stream_offset_shifts_where_its_samples_sit_on_the_asset_clock() {
    let dir = temp("offset");
    // The stream starts 0.5 s after the first picture: picture time 1 s is stream sample 24 000.
    let state = project(
        &dir,
        Ratio::new(1, 2).unwrap(),
        json!({"start":0,"duration":48,"source_start":1}),
    );
    let plan = state.plan_audio(0, 48).unwrap();
    assert_eq!(plan.segments[0].source_start, 24_000);
    // Before the stream exists the position is negative: silence, not a wrapped sample.
    let early = project(
        &dir,
        Ratio::new(1, 2).unwrap(),
        json!({"start":0,"duration":48}),
    );
    let plan = early.plan_audio(0, 48).unwrap();
    assert_eq!(plan.segments[0].source_start, -24_000);
    let pcm = mix(&early, 0, 48);
    assert_eq!(pcm.sample(1_000, 0), 0.0);
    assert!((pcm.sample(24_000, 0) - centered(0.0)).abs() < 1e-3);
    assert!((pcm.sample(30_000, 0) - centered(6_000.0)).abs() < 1e-3);
}

#[test]
fn speed_makes_the_mix_read_faster_through_the_source() {
    let dir = temp("speed");
    let state = project(
        &dir,
        Ratio::ZERO,
        json!({"duration":24,"properties":{"speed":200}}),
    );
    let plan = state.plan_audio(0, 24).unwrap();
    assert_eq!(plan.segments[0].speed, 2.0);
    let pcm = mix(&state, 0, 24);
    // 0.5 s of timeline = 1 s of source: output sample 24 000 is source sample 48 000.
    assert!((pcm.sample(24_000, 0) - centered(48_000.0)).abs() < 1e-3);
}

#[test]
fn mute_and_levels_are_per_clip_and_hiding_the_track_keeps_the_sound() {
    let dir = temp("mute");
    let mut state = project(
        &dir,
        Ratio::ZERO,
        json!({"properties":{"volume":-6,"pan":-100}}),
    );
    let segment = state.plan_audio(0, 48).unwrap().segments.remove(0);
    assert!((segment.gain - 10f32.powf(-6.0 / 20.0)).abs() < 1e-5);
    assert_eq!(segment.pan, -1.0);
    ok(
        &mut state,
        json!({"op":"set","target":"track:v","field":"hidden","value":true}),
    );
    assert_eq!(
        state.plan_audio(0, 48).unwrap().segments.len(),
        1,
        "a hidden picture is not a mute"
    );
    ok(
        &mut state,
        json!({"op":"clip.property","clip":"c","property":"audio_muted","value":true}),
    );
    assert!(state.plan_audio(0, 48).unwrap().segments.is_empty());
    assert!(state.audio_sources(0, 48).unwrap().is_empty());
}

#[test]
fn a_missing_stream_or_undecoded_sound_is_a_typed_error() {
    let dir = temp("errors");
    let mut state = project(&dir, Ratio::ZERO, json!({"properties":{"audio_stream":1}}));
    let error = state.plan_audio(0, 48).unwrap_err();
    assert_eq!(error.code, "audio_stream_missing");

    ok(
        &mut state,
        json!({"op":"clip.property","clip":"c","property":"audio_stream","value":0}),
    );
    state.set_decoded("vid", None);
    let error = state.plan_audio(0, 48).unwrap_err();
    assert_eq!(error.code, "asset_not_decoded", "{error:?}");
    assert!(
        !error.message.contains("media.prepare"),
        "the message does not send people to an internal command: {}",
        error.message
    );
}

#[test]
fn a_video_without_sound_is_silent_and_a_legacy_asset_still_has_one_stream() {
    let dir = temp("legacy");
    let mut state = project(&dir, Ratio::ZERO, json!({}));
    // Documents saved before streams were recorded only carry sample_rate/channels.
    let asset = state
        .library
        .items
        .iter_mut()
        .find(|i| i.id == "vid")
        .unwrap()
        .asset
        .as_mut()
        .unwrap();
    asset.meta.audio_streams.clear();
    assert_eq!(state.plan_audio(0, 48).unwrap().segments.len(), 1);
    let asset = state
        .library
        .items
        .iter_mut()
        .find(|i| i.id == "vid")
        .unwrap()
        .asset
        .as_mut()
        .unwrap();
    asset.meta.sample_rate = None;
    assert!(
        state.plan_audio(0, 48).unwrap().segments.is_empty(),
        "no sound, no segment, no error"
    );
}
