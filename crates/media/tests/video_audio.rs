//! The sound a video carries is mixed by the core's audio plan, with no second WAV asset: stream
//! choice, trim, speed, stream offset and per-clip mute, proved on the exported file's samples.
//! Also the variable-frame-rate policy: frames are conformed by timestamp, never by index.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use serde_json::{Value, json};
use tessel_editor_core::audio::{Pcm, decode_wav};
use tessel_editor_core::{CommandError, EditorState, headless};
use tessel_editor_media::host::{MediaHost, RendererFactory};

use common::mock::{Mock, Settings};
use common::{ffmpeg, temp, tools};

fn host() -> MediaHost {
    let factory: Arc<RendererFactory> = Arc::new(|| Ok(Box::new(Mock::new(Settings::default()))));
    MediaHost::new().with_renderer(factory)
}

fn run(
    host: &mut MediaHost,
    state: &mut EditorState,
    request: Value,
) -> Result<Value, CommandError> {
    headless::execute_ext(state, None, Some(host), &request)
}

fn ok(host: &mut MediaHost, state: &mut EditorState, request: Value) -> Value {
    run(host, state, request.clone()).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

/// 2 s at 24 fps whose own audio is silent for the first second and a 440 Hz tone for the
/// second. `audio_delay` shifts the audio stream against the picture (container timestamps).
fn silent_then_tone(dir: &Path, name: &str, audio_delay: f64) -> PathBuf {
    let path = dir.join(name);
    let delay = audio_delay.to_string();
    let mut args = vec![
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=320x180:rate=24:duration=2",
    ];
    if audio_delay > 0.0 {
        args.extend(["-itsoffset", &delay]);
    }
    args.extend([
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=2:sample_rate=48000",
        "-af",
        "volume='if(lt(t,1),0,1)':eval=frame",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        path.to_str().unwrap(),
    ]);
    ffmpeg(&tools(), &args);
    path
}

/// A project with the video imported (copied: one managed asset, no extra audio asset) and one
/// clip of it on a video track.
fn project_with(dir: &Path, video: &Path, clip: Value) -> (MediaHost, EditorState) {
    let mut host = host();
    let mut state = EditorState::demo();
    let workspace = dir.join("project");
    ok(
        &mut host,
        &mut state,
        json!({"op":"project.create","name":"Sound","sequence":"main","rate":24,
            "width":320,"height":180,"length":48,"dir":workspace}),
    );
    ok(
        &mut host,
        &mut state,
        json!({"op":"project.save","path":workspace.join("sound.tessel.json")}),
    );
    let report = ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[video]}),
    );
    let id = report["applied"]["items"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    ok(
        &mut host,
        &mut state,
        json!({"op":"track.add","id":"v1","name":"Video","kind":"video"}),
    );
    let mut request = json!({"op":"clip.add","id":"c","track":"v1","asset":id});
    for (key, value) in clip.as_object().unwrap() {
        request[key] = value.clone();
    }
    ok(&mut host, &mut state, request);
    (host, state)
}

fn export(
    host: &mut MediaHost,
    state: &mut EditorState,
    dir: &Path,
    name: &str,
) -> (Value, PathBuf) {
    let destination = dir.join(name);
    let report = ok(
        host,
        state,
        json!({"op":"export.start","preset":{"id":"t","name":"t","width":320,"height":180},
            "destination":destination}),
    );
    (report, destination)
}

fn exported_audio(file: &Path, dir: &Path) -> Pcm {
    let wav = dir.join(format!(
        "{}.wav",
        file.file_stem().unwrap().to_string_lossy()
    ));
    ffmpeg(
        &tools(),
        &[
            "-i",
            file.to_str().unwrap(),
            "-vn",
            "-c:a",
            "pcm_s16le",
            wav.to_str().unwrap(),
        ],
    );
    decode_wav(&fs::read(wav).unwrap()).unwrap()
}

/// RMS of every 10 ms window of the left channel.
fn envelope(pcm: &Pcm) -> Vec<f32> {
    let window = pcm.sample_rate as usize / 100;
    (0..pcm.frames() / window)
        .map(|index| {
            let sum: f32 = (0..window)
                .map(|n| pcm.sample(index * window + n, 0).powi(2))
                .sum();
            (sum / window as f32).sqrt()
        })
        .collect()
}

/// Seconds at which the envelope first rises above, and last falls below, `threshold`.
fn sound_span(envelope: &[f32], threshold: f32) -> Option<(f64, f64)> {
    let first = envelope.iter().position(|value| *value > threshold)?;
    let last = envelope.iter().rposition(|value| *value > threshold)?;
    Some((first as f64 / 100.0, (last + 1) as f64 / 100.0))
}

fn zero_crossings(pcm: &Pcm, from_second: f64, to_second: f64) -> usize {
    let rate = f64::from(pcm.sample_rate);
    let (from, to) = ((from_second * rate) as usize, (to_second * rate) as usize);
    (from + 1..to)
        .filter(|&n| pcm.sample(n - 1, 0).is_sign_negative() != pcm.sample(n, 0).is_sign_negative())
        .count()
}

#[test]
fn a_video_with_its_own_sound_exports_that_sound_with_trim_and_offset() {
    let dir = temp("video-sound");
    let video = silent_then_tone(&dir, "clip.mp4", 0.0);
    // Timeline 0.5 s .. 1.5 s plays source 1 s .. 2 s: only the tone, shifted by half a second.
    let (mut host, mut state) = project_with(
        &dir,
        &video,
        json!({"start":12,"duration":24,"source_start":1}),
    );
    assert_eq!(
        state
            .library
            .items
            .iter()
            .filter(|item| item.asset.is_some())
            .count(),
        1,
        "the sound is not a second, separate asset"
    );

    let preflight = ok(
        &mut host,
        &mut state,
        json!({"op":"export.preflight","preset":{"id":"t","name":"t","width":320,"height":180},
            "destination":dir.join("preflight.mp4")}),
    );
    assert_eq!(preflight["ok"], true, "{preflight}");
    assert_eq!(
        preflight["summary"]["audio"]["enabled"], true,
        "{preflight}"
    );
    assert_eq!(
        preflight["summary"]["audio"]["sources"][0]["embedded"],
        true
    );
    assert!(
        !preflight["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning["code"] == "video_audio_not_mixed"),
        "the old refusal is gone: {preflight}"
    );

    let (report, file) = export(&mut host, &mut state, &dir, "out.mp4");
    assert_eq!(report["verification"]["ok"], true, "{report}");
    assert_eq!(report["audio"]["samples"], 96_000);
    let pcm = exported_audio(&file, &dir);
    let (start, end) = sound_span(&envelope(&pcm), 0.02).expect("the exported file has sound");
    // AAC priming and window quantization cost a few tens of milliseconds, not a frame of drift.
    assert!(
        (start - 0.5).abs() < 0.06,
        "tone starts at {start} s, expected 0.5 s"
    );
    assert!(
        (end - 1.5).abs() < 0.06,
        "tone ends at {end} s, expected 1.5 s"
    );
    let crossings = zero_crossings(&pcm, 0.6, 1.4);
    // 440 Hz → 880 sign changes per second over 0.8 s.
    assert!(
        (crossings as f64 - 704.0).abs() < 12.0,
        "the exported tone is 440 Hz: {crossings} crossings in 0.8 s"
    );
}

#[test]
fn speed_scales_the_source_position_of_the_sound() {
    let dir = temp("video-speed");
    let video = silent_then_tone(&dir, "clip.mp4", 0.0);
    // 1 s of timeline at 200 % reads source 0 s .. 2 s: the tone (source 1 s) starts at 0.5 s.
    let (mut host, mut state) = project_with(
        &dir,
        &video,
        json!({"start":0,"duration":24,"properties":{"speed":200}}),
    );
    let (report, file) = export(&mut host, &mut state, &dir, "out.mp4");
    assert_eq!(report["verification"]["ok"], true, "{report}");
    let (start, _) = sound_span(&envelope(&exported_audio(&file, &dir)), 0.02).unwrap();
    assert!(
        (start - 0.5).abs() < 0.06,
        "tone starts at {start} s, expected 0.5 s"
    );
}

#[test]
fn a_stream_that_starts_after_the_picture_is_placed_by_its_timestamp() {
    let dir = temp("video-offset");
    // The audio stream begins 0.5 s after the first picture and its tone starts at container time
    // 1 s (the filter clock includes the input offset). Samples 0.. of the decoded WAV are the
    // stream's own, so without the offset the tone would land at 0.5 s of the timeline.
    let video = silent_then_tone(&dir, "late.mp4", 0.5);
    let (mut host, mut state) = project_with(&dir, &video, json!({"start":0,"duration":48}));
    let offset = state
        .library
        .items
        .iter()
        .find_map(|item| {
            item.asset
                .as_ref()
                .and_then(|asset| asset.meta.audio_stream(0))
        })
        .expect("the probe recorded the stream")
        .offset
        .to_f64();
    assert!(
        (offset - 0.5).abs() < 0.03,
        "recorded stream offset {offset}"
    );
    let (report, file) = export(&mut host, &mut state, &dir, "out.mp4");
    assert_eq!(report["verification"]["ok"], true, "{report}");
    let (start, _) = sound_span(&envelope(&exported_audio(&file, &dir)), 0.02).unwrap();
    assert!(
        (start - 1.0).abs() < 0.07,
        "tone starts at {start} s, expected 1.0 s (0.5 s would mean the offset was ignored)"
    );
}

#[test]
fn a_muted_video_clip_adds_no_audio_stream() {
    let dir = temp("video-muted");
    let video = silent_then_tone(&dir, "clip.mp4", 0.0);
    let (mut host, mut state) = project_with(
        &dir,
        &video,
        json!({"start":0,"duration":48,"properties":{"audio_muted":true}}),
    );
    let preflight = ok(
        &mut host,
        &mut state,
        json!({"op":"export.preflight","preset":{"id":"t","name":"t","width":320,"height":180},
            "destination":dir.join("p.mp4")}),
    );
    assert!(
        preflight["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning["code"] == "no_audio_sources"),
        "{preflight}"
    );
    let (report, file) = export(&mut host, &mut state, &dir, "out.mp4");
    assert_eq!(report["verification"]["ok"], true, "{report}");
    assert!(report["audio"].is_null(), "{report}");
    let probe = Command::new(&tools().ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type",
            "-of",
            "csv=p=0",
        ])
        .arg(&file)
        .output()
        .unwrap();
    assert!(
        !String::from_utf8_lossy(&probe.stdout).contains("audio"),
        "muted: no audio stream in the file"
    );
}

#[test]
fn a_stream_the_file_does_not_have_is_an_error_not_silence() {
    let dir = temp("video-stream");
    let video = silent_then_tone(&dir, "clip.mp4", 0.0);
    let (mut host, mut state) = project_with(
        &dir,
        &video,
        json!({"start":0,"duration":48,"properties":{"audio_stream":1}}),
    );
    let preflight = ok(
        &mut host,
        &mut state,
        json!({"op":"export.preflight","preset":{"id":"t","name":"t","width":320,"height":180},
            "destination":dir.join("p.mp4")}),
    );
    assert_eq!(preflight["ok"], false);
    assert_eq!(
        preflight["blockers"][0]["code"], "audio_stream_missing",
        "{preflight}"
    );
    assert!(!dir.join("p.mp4").exists());
}

// ---- variable frame rate ----

/// 2 s of a brightness ramp (luma = 100 × seconds, as a flat gray) at 24 fps from which the
/// frames 10..=20 are removed with their timestamps kept: a variable-rate file whose frame at
/// 0.75 s is the one from 0.375 s.
fn variable_rate_video(dir: &Path) -> PathBuf {
    let path = dir.join("vfr.mp4");
    ffmpeg(
        &tools(),
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=black:size=64x36:rate=24:duration=2,geq=lum='min(255,100*T)':cb=128:cr=128",
            "-vf",
            "select='not(between(n,10,20))'",
            "-fps_mode",
            "vfr",
            "-c:v",
            "libx264",
            "-qp",
            "0",
            "-pix_fmt",
            "yuv420p",
            path.to_str().unwrap(),
        ],
    );
    path
}

#[test]
fn variable_frame_rate_is_conformed_by_timestamp_and_reported() {
    let dir = temp("vfr");
    let video = variable_rate_video(&dir);
    let mut host = host();
    let mut state = EditorState::demo();
    let workspace = dir.join("project");
    ok(
        &mut host,
        &mut state,
        json!({"op":"project.create","name":"Vfr","sequence":"main","rate":24,
            "width":64,"height":36,"length":48,"dir":workspace}),
    );
    ok(
        &mut host,
        &mut state,
        json!({"op":"project.save","path":workspace.join("vfr.tessel.json")}),
    );
    let report = ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[video]}),
    );
    let item = &report["applied"]["items"][0];
    assert!(
        item["warnings"].to_string().contains("variable_frame_rate"),
        "import reports the policy instead of staying silent: {report}"
    );
    let asset_id = item["id"].as_str().unwrap();
    let asset = state.library.item(asset_id).unwrap().asset.clone().unwrap();
    assert!(asset.meta.vfr_conformed);
    assert_eq!(
        asset.meta.rate.map(|rate| (rate.num(), rate.den())),
        Some((24, 1))
    );
    let frames = asset
        .decoded
        .as_ref()
        .and_then(|d| d.frames.as_ref())
        .expect("decoded on import");
    assert_eq!(
        frames.count, 48,
        "a constant grid over the whole 2 s, gap included"
    );
    let pattern = |index: u32| {
        tessel_editor_core::assets::expand_pattern(&frames.pattern, i64::from(index)).unwrap()
    };
    let luma = |index: u32| {
        let png = pattern(index);
        let output = Command::new(&tools().ffmpeg)
            .args([
                "-v",
                "error",
                "-nostdin",
                "-i",
                &png,
                "-vf",
                "format=gray",
                "-f",
                "rawvideo",
                "-",
            ])
            .output()
            .unwrap();
        i32::from(output.stdout[output.stdout.len() / 2])
    };
    // The source luma is 100 × seconds in limited-range Y; the decoded PNGs are full-range RGB.
    let full_range = |y: f64| ((y - 16.0) * 255.0 / 219.0) as i32;
    // By index, frame 18 would be the 8th source frame (0.333 s). By timestamp the gap
    // 0.417 s..0.875 s keeps showing the last frame before it (0.375 s), and the frame at
    // 1.5 s shows the 1.5 s picture.
    let hold = luma(18);
    assert!(
        (hold - full_range(37.5)).abs() <= 6,
        "frame inside the gap holds the 0.375 s picture, got {hold}"
    );
    let late = luma(36);
    assert!(
        (late - full_range(150.0)).abs() <= 8,
        "frame at 1.5 s shows the 1.5 s picture, got {late}"
    );

    // The preflight keeps saying so: never a silent timing change.
    ok(
        &mut host,
        &mut state,
        json!({"op":"track.add","id":"v1","name":"Video","kind":"video"}),
    );
    ok(
        &mut host,
        &mut state,
        json!({"op":"clip.add","id":"c","track":"v1","asset":asset_id,"start":0,"duration":48}),
    );
    let preflight = ok(
        &mut host,
        &mut state,
        json!({"op":"export.preflight","preset":{"id":"t","name":"t","width":64,"height":36},
            "destination":dir.join("p.mp4")}),
    );
    assert!(
        preflight["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning["code"] == "variable_frame_rate"),
        "{preflight}"
    );
}

#[test]
fn a_constant_rate_file_is_not_flagged() {
    let dir = temp("cfr");
    let video = common::video_with_audio(&tools(), &dir, "cfr.mp4");
    let mut host = host();
    let mut state = EditorState::demo();
    let workspace = dir.join("project");
    ok(
        &mut host,
        &mut state,
        json!({"op":"project.create","name":"Cfr","sequence":"main","rate":24,
            "width":64,"height":36,"length":48,"dir":workspace}),
    );
    ok(
        &mut host,
        &mut state,
        json!({"op":"project.save","path":workspace.join("cfr.tessel.json")}),
    );
    let report = ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[video]}),
    );
    assert!(
        !report.to_string().contains("variable_frame_rate"),
        "{report}"
    );
}
