//! P10/P11: video export with audio, presets, preflight, cancellation and failures.
//!
//! The frames come from a deterministic renderer double that paints a flat color chosen by the
//! sequence frame of the plan. That keeps these tests about what export owns (time mapping,
//! geometry, color, audio sync, container, cleanup) and fast; `export_real_chrome.rs` runs the
//! same pipeline against the real Three.js runtime.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::{Value, json};
use tessel_editor_core::fixtures::{proof_requests, video_frame_color, write_standard_set};
use tessel_editor_core::jobs::{JobError, JobKind, JobRegistry, JobState};
use tessel_editor_core::preset::Preset;
use tessel_editor_core::session::Renderer;
use tessel_editor_core::{EditorState, headless};
use tessel_editor_media::export::{ExportRequest, preflight, run, schedule, verify};
use tessel_editor_media::tools::Tools;

use common::mock::{Mock, Settings};
use common::{temp, tools};

fn project(dir: &Path) -> (EditorState, PathBuf) {
    let set = write_standard_set(&dir.join("fx")).unwrap();
    let mut state = EditorState::demo();
    for request in proof_requests(&set) {
        headless::execute(&mut state, &request).unwrap();
    }
    (state, set.tone)
}

fn custom(id: &str, width: u32, height: u32, fps: Option<Value>) -> Preset {
    Preset::from_json(
        &json!({ "id": id, "name": id, "width": width, "height": height, "fps": fps }),
    )
    .unwrap()
}

fn export(
    state: &EditorState,
    request: &ExportRequest,
    renderer: &mut Mock,
) -> Result<Value, JobError> {
    let registry = JobRegistry::new(1);
    let tools = tools();
    let (_, outcome) = registry.run_inline(JobKind::VideoExport, "export", |ctx| {
        run(state, request, &tools, renderer, ctx)
    });
    outcome
}

/// Decoded RGB of output frame `index` through an explicit BT.709 limited-range conversion, so
/// the check does not depend on FFmpeg honouring the stream tags.
fn rgb_frame(tools: &Tools, file: &Path, index: u64, width: u32, height: u32) -> Vec<u8> {
    let filter = format!(
        "select=eq(n\\,{index}),scale=in_color_matrix=bt709:in_range=tv:out_range=pc,format=rgb24"
    );
    let output = Command::new(&tools.ffmpeg)
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(file)
        .args(["-vf", &filter, "-frames:v", "1", "-f", "rawvideo", "-"])
        .output()
        .unwrap();
    assert_eq!(
        output.stdout.len(),
        (width * height * 3) as usize,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn pixel(rgb: &[u8], width: u32, x: u32, y: u32) -> [i32; 3] {
    let at = ((y * width + x) * 3) as usize;
    [
        i32::from(rgb[at]),
        i32::from(rgb[at + 1]),
        i32::from(rgb[at + 2]),
    ]
}

fn close(actual: [i32; 3], expected: [u8; 3], tolerance: i32) -> bool {
    (0..3).all(|channel| (actual[channel] - i32::from(expected[channel])).abs() <= tolerance)
}

fn decoded_audio(tools: &Tools, file: &Path, dir: &Path) -> tessel_editor_core::audio::Pcm {
    let wav = dir.join("decoded.wav");
    common::ffmpeg(
        tools,
        &[
            "-i",
            file.to_str().unwrap(),
            "-vn",
            "-c:a",
            "pcm_s16le",
            wav.to_str().unwrap(),
        ],
    );
    tessel_editor_core::audio::decode_wav(&fs::read(wav).unwrap()).unwrap()
}

fn leftovers(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(".tessel-staging"))
        .collect()
}

#[test]
fn exports_video_and_audio_that_match_the_plan() {
    let dir = temp("export-basic");
    let (state, _) = project(&dir);
    let tools = tools();
    let target = dir.join("out/film.mp4");
    let request = ExportRequest::new(custom("t640", 640, 360, None), &target);
    let mut renderer = Mock::new(Settings::default());
    let before = state.to_document(None);

    let report = export(&state, &request, &mut renderer).unwrap();

    assert_eq!(report["frames"], 48);
    assert_eq!(report["verification"]["ok"], true, "{report}");
    assert_eq!(report["audio"]["samples"], 96_000);
    assert_eq!(
        state.to_document(None),
        before,
        "export works on a snapshot"
    );
    assert!(leftovers(&dir.join("out")).is_empty());
    assert_eq!(
        *renderer.seen.lock().unwrap(),
        (0..48).collect::<Vec<i64>>()
    );

    // Independent proof from the file itself: first, middle and last frame carry the color of
    // the sequence frame that was asked for, at the right place.
    for index in [0u64, 17, 47] {
        let rgb = rgb_frame(&tools, &target, index, 640, 360);
        let center = pixel(&rgb, 640, 320, 180);
        assert!(
            close(center, video_frame_color(index as i64), 4),
            "frame {index}: {center:?}"
        );
    }
    let pcm = decoded_audio(&tools, &target, &dir);
    assert!(
        (pcm.frames() as f64 - 96_000.0).abs() < 2200.0,
        "{}",
        pcm.frames()
    );
    let expected_rms = report["audio"]["rms"].as_f64().unwrap();
    assert!(
        (f64::from(pcm.rms()) - expected_rms).abs() < expected_rms * 0.1,
        "{} vs {expected_rms}",
        pcm.rms()
    );
    assert!(expected_rms > 0.05, "the tone is audible in the mix");
}

#[test]
fn frame_rate_override_picks_the_sequence_frame_showing_at_each_instant() {
    let dir = temp("export-fps");
    let (state, _) = project(&dir);
    for (fps, den, frames) in [
        (json!(30), 1u64, 60usize),
        (json!({ "num": 30000, "den": 1001 }), 1001, 60),
    ] {
        let num = if den == 1 { 30 } else { 30_000 };
        let target = dir.join(format!("fps-{num}.mp4"));
        let request = ExportRequest::new(custom("t320", 320, 180, Some(fps)), &target);
        let mut renderer = Mock::new(Settings::default());
        let report = export(&state, &request, &mut renderer).unwrap();
        assert_eq!(report["frames"], frames);
        assert_eq!(report["verification"]["ok"], true, "{report}");
        let seen = renderer.seen.lock().unwrap().clone();
        let expected: Vec<i64> = (0..frames as u64)
            .map(|k| (k * 24 * den / num) as i64)
            .collect();
        assert_eq!(seen, expected, "{num}/{den}");
        assert!(
            report["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|w| w["code"] == "frame_rate_conversion")
        );
        // The audio is as long as the video, to the sample.
        let samples = (frames as u64 * den * 48_000 + num / 2) / num;
        assert_eq!(report["audio"]["samples"].as_u64().unwrap(), samples);
    }
}

#[test]
fn builtin_presets_render_at_the_real_size_with_bars_and_correct_colors() {
    let dir = temp("export-presets");
    let (state, _) = project(&dir);
    let tools = tools();
    let cases = [
        ("fullhd", 1920u32, 1080u32, (960u32, 540u32), None),
        ("vertical", 1080, 1920, (540, 960), Some((540u32, 20u32))),
        ("square", 1080, 1080, (540, 540), Some((540, 20))),
    ];
    for (id, width, height, center, bar) in cases {
        let preset = tessel_editor_core::preset::find(id, &[]).unwrap();
        let target = dir.join(format!("{id}.mp4"));
        let mut request = ExportRequest::new(preset, &target);
        request.range = Some((10, 22));
        let mut renderer = Mock::new(Settings::default());
        let report = export(&state, &request, &mut renderer).unwrap();
        assert_eq!(report["verification"]["ok"], true, "{id}: {report}");
        assert_eq!(
            report["output"],
            json!({ "width": width, "height": height })
        );
        let rgb = rgb_frame(&tools, &target, 3, width, height);
        let middle = pixel(&rgb, width, center.0, center.1);
        assert!(close(middle, video_frame_color(13), 4), "{id}: {middle:?}");
        if let Some((x, y)) = bar {
            let bars = pixel(&rgb, width, x, y);
            assert!(
                close(bars, [0, 0, 0], 3),
                "{id}: letterbox bar is black, got {bars:?}"
            );
        }
        // The renderer was asked for the real aspect-correct size, never the final size upscaled.
        let probe = report["render"].clone();
        assert!(probe["width"].as_u64().unwrap() <= u64::from(width));
        assert!(probe["height"].as_u64().unwrap() <= u64::from(height));
    }
}

#[test]
fn uhd4k_exports_a_short_range_at_3840x2160() {
    let dir = temp("export-4k");
    let (state, _) = project(&dir);
    let target = dir.join("uhd.mp4");
    let mut request = ExportRequest::new(
        tessel_editor_core::preset::find("uhd4k", &[]).unwrap(),
        &target,
    );
    request.range = Some((0, 12));
    let report = export(&state, &request, &mut Mock::new(Settings::default())).unwrap();
    assert_eq!(report["verification"]["ok"], true, "{report}");
    assert_eq!(report["render"], json!({ "width": 3840, "height": 2160 }));
    assert_eq!(report["frames"], 12);
}

#[test]
fn preflight_names_every_blocker_before_any_work() {
    let dir = temp("export-preflight");
    let (state, tone) = project(&dir);
    let tools = tools();
    let target = dir.join("film.mp4");
    let preset = custom("t640", 640, 360, None);
    let renderer_caps = Mock::new(Settings {
        max_size: Some(1024),
        ..Settings::default()
    })
    .capabilities()
    .unwrap();

    let ok = preflight(
        &state,
        &ExportRequest::new(preset.clone(), &target),
        &tools,
        Some(&renderer_caps),
    );
    assert_eq!(ok["ok"], true, "{ok}");
    assert_eq!(ok["summary"]["frames"], 48);
    assert_eq!(ok["summary"]["audio"]["enabled"], true);
    assert!(!target.exists(), "preflight writes nothing");

    let too_large = preflight(
        &state,
        &ExportRequest::new(
            tessel_editor_core::preset::find("uhd4k", &[]).unwrap(),
            &target,
        ),
        &tools,
        Some(&renderer_caps),
    );
    assert!(
        too_large["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["code"] == "render_too_large"),
        "{too_large}"
    );

    let wrong_extension = preflight(
        &state,
        &ExportRequest::new(preset.clone(), dir.join("film.mov")),
        &tools,
        None,
    );
    assert!(
        wrong_extension["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["code"] == "invalid_destination")
    );

    let mut bad_range = ExportRequest::new(preset.clone(), &target);
    bad_range.range = Some((10, 400));
    let bad_range = preflight(&state, &bad_range, &tools, None);
    assert_eq!(bad_range["blockers"][0]["code"], "invalid_range");

    fs::write(&target, b"existing").unwrap();
    let exists = preflight(
        &state,
        &ExportRequest::new(preset.clone(), &target),
        &tools,
        None,
    );
    assert!(
        exists["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["code"] == "destination_exists")
    );
    let mut replace = ExportRequest::new(preset.clone(), &target);
    replace.replace = true;
    assert_eq!(preflight(&state, &replace, &tools, None)["ok"], true);
    fs::remove_file(&target).unwrap();

    // The tone is the user's original: moving it away blocks the export and names the asset.
    let hidden = tone.with_extension("moved");
    fs::rename(&tone, &hidden).unwrap();
    let missing = preflight(&state, &ExportRequest::new(preset, &target), &tools, None);
    let blocker = missing["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["code"] == "asset_missing")
        .unwrap()
        .clone();
    assert_eq!(blocker["asset"], "tone");
    let error = export(
        &state,
        &ExportRequest::new(custom("t640", 640, 360, None), &target),
        &mut Mock::new(Settings::default()),
    )
    .unwrap_err();
    assert_eq!(error.code, "preflight_failed");
    assert!(!target.exists());
    fs::rename(&hidden, &tone).unwrap();
}

#[test]
fn a_renderer_failure_leaves_no_file_and_no_staging() {
    let dir = temp("export-fail");
    let (state, _) = project(&dir);
    let target = dir.join("out/film.mp4");
    let request = ExportRequest::new(custom("t320", 320, 180, None), &target);
    let mut renderer = Mock::new(Settings {
        fail_at: Some(10),
        ..Settings::default()
    });
    let error = export(&state, &request, &mut renderer).unwrap_err();
    assert_eq!(error.code, "render_failed", "{error:?}");
    assert!(error.message.contains("Frame 10"), "{}", error.message);
    assert!(!target.exists());
    assert!(leftovers(&dir.join("out")).is_empty());
}

#[test]
fn replacing_swaps_the_file_in_one_step_and_refusing_keeps_the_original() {
    let dir = temp("export-replace");
    let (state, _) = project(&dir);
    let target = dir.join("film.mp4");
    fs::write(&target, b"previous export").unwrap();
    let mut request = ExportRequest::new(custom("t320", 320, 180, None), &target);
    request.range = Some((0, 12));
    let error = export(&state, &request, &mut Mock::new(Settings::default())).unwrap_err();
    assert_eq!(error.code, "destination_exists");
    assert_eq!(fs::read(&target).unwrap(), b"previous export");
    request.replace = true;
    let report = export(&state, &request, &mut Mock::new(Settings::default())).unwrap();
    assert_eq!(report["verification"]["ok"], true);
    assert!(fs::metadata(&target).unwrap().len() > 1000);
}

#[test]
fn verification_rejects_a_file_that_is_not_what_was_asked() {
    let dir = temp("export-verify");
    let (state, _) = project(&dir);
    let tools = tools();
    let target = dir.join("film.mp4");
    let mut request = ExportRequest::new(custom("t320", 320, 180, None), &target);
    request.range = Some((0, 12));
    export(&state, &request, &mut Mock::new(Settings::default())).unwrap();

    let mut other = ExportRequest::new(custom("t640", 640, 360, None), &target);
    other.range = Some((0, 24));
    let wanted = schedule(&state, &other).unwrap();
    let error = verify(&tools, &target, &other, &wanted, true).unwrap_err();
    assert_eq!(error.code, "verification_failed");
    let failed: Vec<&str> = error.details.as_ref().unwrap()["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|check| check["ok"] == false)
        .filter_map(|check| check["name"].as_str())
        .collect();
    assert!(
        failed.contains(&"size") && failed.contains(&"frame_count"),
        "{failed:?}"
    );

    let not_media = dir.join("note.mp4");
    fs::write(&not_media, b"just text").unwrap();
    assert_eq!(
        verify(&tools, &not_media, &other, &wanted, true)
            .unwrap_err()
            .code,
        "verification_failed"
    );
}

#[test]
fn cancelling_stops_the_encoder_and_removes_everything_it_wrote() {
    let dir = temp("export-cancel");
    let (state, _) = project(&dir);
    let tools = tools();
    let target = dir.join("out/film.mp4");
    let request = ExportRequest::new(custom("t320", 320, 180, None), &target);
    let renderer = Mock::new(Settings {
        delay: Duration::from_millis(40),
        ..Settings::default()
    });
    let rendered = renderer.rendered.clone();
    let registry = JobRegistry::new(1);
    let id = registry.spawn(JobKind::VideoExport, "export", move |ctx| {
        let mut renderer = renderer;
        run(&state, &request, &tools, &mut renderer, ctx)
    });
    while registry
        .get(&id)
        .is_some_and(|info| !info.state.is_final() && (info.stage != "render" || info.done < 5))
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    registry.cancel(&id).unwrap();
    let info = registry.wait(&id, Duration::from_secs(20)).unwrap();
    assert_eq!(info.state, JobState::Cancelled, "{info:?}");
    assert!(
        rendered.load(Ordering::SeqCst) < 48,
        "rendering stopped early"
    );
    assert!(!target.exists());
    assert!(leftovers(&dir.join("out")).is_empty());
}

/// A real full volume: an HFS+ image of 40 MB takes an incompressible export that the estimate
/// cannot foresee, so the failure comes from the filesystem in the middle of the encode.
#[test]
fn a_full_disk_ends_the_export_with_disk_full_and_leaves_the_volume_clean() {
    if !cfg!(target_os = "macos") {
        eprintln!("skipped: needs hdiutil");
        return;
    }
    let dir = temp("export-disk");
    let (state, _) = project(&dir);
    let image = dir.join("small.dmg");
    let mount = dir.join("mnt");
    fs::create_dir_all(&mount).unwrap();
    let created = Command::new("hdiutil")
        .args([
            "create",
            "-size",
            "40m",
            "-fs",
            "HFS+",
            "-volname",
            "tessel-full",
            "-ov",
        ])
        .arg(&image)
        .output()
        .unwrap();
    if !created.status.success() {
        eprintln!(
            "skipped: hdiutil create failed: {}",
            String::from_utf8_lossy(&created.stderr)
        );
        return;
    }
    let attached = Command::new("hdiutil")
        .args(["attach", "-nobrowse", "-mountpoint"])
        .arg(&mount)
        .arg(&image)
        .output()
        .unwrap();
    assert!(
        attached.status.success(),
        "{}",
        String::from_utf8_lossy(&attached.stderr)
    );
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let target = mount.join("film.mp4");
        // CRF 1 on noise: the estimate (which scales with CRF) stays far below what is written.
        let lossless = Preset::from_json(&json!({ "id": "full", "name": "full", "width": 1280, "height": 720, "video": { "crf": 1 } })).unwrap();
        let mut request = ExportRequest::new(lossless, &target);
        request.range = Some((0, 48));
        let mut renderer = Mock::new(Settings {
            noise: true,
            ..Settings::default()
        });
        let error = export(&state, &request, &mut renderer).unwrap_err();
        assert_eq!(error.code, "disk_full", "{error:?}");
        assert!(!target.exists());
        assert!(
            leftovers(&mount).is_empty(),
            "staging was removed: {:?}",
            leftovers(&mount)
        );
    }));
    let _ = Command::new("hdiutil")
        .args(["detach", "-force"])
        .arg(&mount)
        .output();
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
