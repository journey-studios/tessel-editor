//! F042 (BL008): the real directed-scene export, end to end, over the synthetic corpus of
//! `scripts/directed-scene-proof` (drawn from scratch, expectations computed independently by
//! `model.py`): real Node + headless Chrome + Three.js frames, real FFmpeg, real ffprobe.
//!
//! What this proves (lib + headless host, no window):
//!
//! - a short 1080p and a short UHD 4K export publish a verified H.264/BT.709 MP4 with the exact
//!   frame count, rate and size, no audio stream for a scene without audio;
//! - the frames inside the MP4 hold the pixels the independent oracle expects (probe points of
//!   `expected.json`, scaled to the output size) within an H.264 tolerance;
//! - the runtime hashes of the export frames equal, frame by frame, those of `scene.render` for the
//!   same scene frame and size (same plan, same resources, same renderer path), including across
//!   the shot cut and with a rate conversion (24 → 30 repeats frames, never blends);
//! - an export reads a frozen scene and frozen resources: invalidating the approved map while it
//!   runs does not change or break it;
//! - invalid requests (unknown scene, bad range, declared audio, missing or stale resources,
//!   existing destination, wrong extension) answer typed errors, leave no file or staging folder
//!   and change nothing in the project; a cancel leaves no partial file; a scene never lands on the
//!   timeline.
//!
//! What it does not prove: the desktop UI/socket (`R2`/coordinator), what the audio sounds like (measured
//! in `crates/media/tests/scene_export_audio.rs`), the independent re-derivation of expectations by R10, a real GPU (SwiftShader by default; run
//! under `with-gpu-lock.py`), pixels against the origin's own runtime.
//!
//! A machine without Chrome, the runtime bundle or FFmpeg skips with the reason on stderr; set
//! `TESSEL_REQUIRE_RUNTIME=1` to turn the skip into a failure.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::{CommandError, EditorState, headless};
use tessel_editor_media::host::MediaHost;
use tessel_editor_media::tools::Tools;
use tessel_editor_render::RuntimeConfig;
use tessel_editor_render::scene_host::SceneHost;

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/directed-scene-proof/corpus")
}

fn expected() -> Value {
    serde_json::from_str(&fs::read_to_string(corpus().join("expected.json")).unwrap()).unwrap()
}

/// Max channel difference an H.264 CRF 18 4:2:0 round trip may add to a flat block.
const CODEC_TOLERANCE: i32 = 14;

fn skip(reason: &str) -> Option<Session> {
    assert!(
        std::env::var("TESSEL_REQUIRE_RUNTIME").as_deref() != Ok("1"),
        "the runtime is required but unavailable: {reason}"
    );
    eprintln!("SKIPPED (runtime unavailable): {reason}");
    None
}

struct Session {
    state: EditorState,
    host: SceneHost,
    work: PathBuf,
}

impl Session {
    fn run(&mut self, request: Value) -> Result<Value, CommandError> {
        headless::execute_ext(&mut self.state, None, Some(&mut self.host), &request)
    }

    fn ok(&mut self, request: Value) -> Value {
        self.run(request.clone())
            .unwrap_or_else(|error| panic!("{request}: {error:?}"))
    }

    fn err(&mut self, request: Value) -> CommandError {
        match self.run(request.clone()) {
            Ok(answer) => panic!("{request} should have failed, got {answer}"),
            Err(error) => error,
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.work.join(name)
    }

    /// Files left in the work folder besides what the test created on purpose.
    fn leftovers(&self) -> Vec<String> {
        fs::read_dir(&self.work)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".tessel-"))
            .collect()
    }

    fn export(&mut self, name: &str, preset: Value, from: i64, frames: i64) -> Value {
        let destination = self.path(name);
        self.ok(json!({"op":"scene.export","scene":"street","preset":preset,
            "from":from,"frames":frames,"destination":destination}))
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.work);
    }
}

/// A session with the corpus attached and its files approved by the media host.
fn session(name: &str) -> Option<Session> {
    let config = match RuntimeConfig::discover() {
        Ok(config) => config,
        Err(setup) => return skip(&setup.message()),
    };
    if let Err(error) = Tools::discover() {
        return skip(&error.message);
    }
    let work = std::env::temp_dir().join(format!("te-r1-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).unwrap();
    let mut session = Session {
        state: EditorState::empty(&ProjectSettings::default()).unwrap(),
        host: SceneHost::new(MediaHost::new(), config),
        work,
    };
    session.ok(json!({"op":"scene.attach","id":"street","path":corpus().join("bundle.json")}));
    session.ok(json!({"op":"scene.resources.resolve","scene":"street","base":corpus()}));
    Some(session)
}

fn ffprobe(path: &Path) -> Value {
    let output = Command::new(Tools::discover().unwrap().ffprobe)
        .args([
            "-v",
            "error",
            "-show_format",
            "-show_streams",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .unwrap();
    serde_json::from_slice(&output.stdout).unwrap()
}

fn video_stream(probe: &Value) -> Value {
    probe["streams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|stream| stream["codec_type"] == "video")
        .cloned()
        .unwrap()
}

/// Decodes `path` to RGB24 frames with an explicit BT.709 limited-range matrix, so the test does
/// not depend on what FFmpeg would guess from the stream.
fn decode_frames(path: &Path, width: usize, height: usize) -> Vec<Vec<u8>> {
    let output = Command::new(Tools::discover().unwrap().ffmpeg)
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args([
            "-vf",
            "scale=in_color_matrix=bt709:in_range=tv:out_range=pc:flags=accurate_rnd+full_chroma_int,format=rgb24",
            "-f",
            "rawvideo",
            "-",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
        .stdout
        .chunks_exact(width * height * 3)
        .map(<[u8]>::to_vec)
        .collect()
}

fn pixel(frame: &[u8], width: usize, x: usize, y: usize) -> [i32; 3] {
    let at = (y * width + x) * 3;
    [
        i32::from(frame[at]),
        i32::from(frame[at + 1]),
        i32::from(frame[at + 2]),
    ]
}

/// The expected probes of `edit_frame` at `scale` times the corpus's 320x180, against `frame`.
/// Returns the worst channel difference, for the evidence line.
fn assert_oracle_pixels(
    frame: &[u8],
    width: usize,
    edit_frame: i64,
    scale: usize,
    what: &str,
) -> i32 {
    let expected = expected();
    let entry = expected["frames"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["editFrame"] == edit_frame)
        .unwrap_or_else(|| panic!("no expected frame {edit_frame}"));
    let mut worst = 0;
    for probe in entry["pixelProbes"].as_array().unwrap() {
        let (x, y) = (
            probe["x"].as_u64().unwrap() as usize * scale + scale / 2,
            probe["y"].as_u64().unwrap() as usize * scale + scale / 2,
        );
        let got = pixel(frame, width, x, y);
        for channel in 0..3 {
            let want = probe["color"][channel].as_i64().unwrap() as i32;
            worst = worst.max((got[channel] - want).abs());
            assert!(
                (got[channel] - want).abs() <= CODEC_TOLERANCE,
                "{what}: edit frame {edit_frame}, probe `{}` at ({x},{y}) is {got:?}, the oracle says {:?}",
                probe["layer"],
                probe["color"]
            );
        }
    }
    worst
}

/// Mean and max absolute channel difference between two pictures of the same size.
fn difference(left: &[u8], right: &[u8]) -> (f64, i32) {
    assert_eq!(left.len(), right.len());
    let (mut sum, mut worst) = (0u64, 0i32);
    for (a, b) in left.iter().zip(right) {
        let delta = (i32::from(*a) - i32::from(*b)).abs();
        sum += delta as u64;
        worst = worst.max(delta);
    }
    (sum as f64 / left.len() as f64, worst)
}

fn png_to_rgb(path: &Path) -> Vec<u8> {
    let output = Command::new(Tools::discover().unwrap().ffmpeg)
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .unwrap();
    assert!(output.status.success());
    output.stdout
}

fn assert_stream(probe: &Value, width: u64, height: u64, frames: u64, rate: &str) {
    let video = video_stream(probe);
    assert_eq!(video["codec_name"], "h264");
    assert_eq!(
        (video["width"].as_u64(), video["height"].as_u64()),
        (Some(width), Some(height))
    );
    assert_eq!(video["pix_fmt"], "yuv420p");
    assert_eq!(video["color_space"], "bt709");
    assert_eq!(video["color_range"], "tv");
    assert_eq!(video["r_frame_rate"], rate);
    assert_eq!(video["nb_frames"], frames.to_string());
    assert!(
        probe["streams"]
            .as_array()
            .unwrap()
            .iter()
            .all(|stream| stream["codec_type"] != "audio"),
        "a scene without audio has no audio stream"
    );
}

/// The `hash` `scene.render` reports for a scene frame at `width`x`height`.
fn rendered_hash(session: &mut Session, frame: i64, width: u32, height: u32) -> String {
    session.ok(json!({"op":"scene.render","scene":"street","frame":frame,
        "width":width,"height":height}))["frame"]["hash"]
        .as_str()
        .unwrap()
        .to_string()
}

fn listed(result: &Value) -> Vec<String> {
    result["frame_hashes"]["list"]
        .as_array()
        .expect("a short export lists every frame hash")
        .iter()
        .map(|hash| hash.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_short_1080p_export_is_a_verified_mp4_with_the_oracle_pixels_and_scene_render_hashes() {
    let Some(mut session) = session("fullhd") else {
        return;
    };
    let clips_before = session.state.timeline.clips.len();
    let history_before = session.ok(json!({"op":"history.status"}));

    let started = Instant::now();
    // Edit frames 8..16 cross no cut but include 11 and 12, which the oracle describes.
    let result = session.export("hd.mp4", json!("fullhd"), 8, 8);
    let elapsed = started.elapsed();
    eprintln!(
        "1080p: 8 frames in {elapsed:?} ({:.1} ms/frame mean, slowest {:.0} ms), {} bytes",
        result["metrics"]["render_ms_mean"].as_f64().unwrap(),
        result["metrics"]["render_ms_slowest"].as_f64().unwrap(),
        result["bytes"]
    );

    assert_eq!(result["action"], "scene.export");
    assert_eq!(result["frames"], 8);
    assert_eq!(result["verification"]["ok"], true);
    assert_eq!(result["range"], json!({"from":8,"frames":8,"to":16}));
    assert_eq!(result["audio"]["included"], false);
    // The report names the bytes the frames came from: the corpus hashes of `expected.json`.
    let resources = result["resources"].as_array().unwrap();
    assert!(!resources.is_empty());
    for resource in resources {
        let asset = resource["asset"].as_str().unwrap();
        assert_eq!(
            resource["sha256"],
            expected()["assets"][asset]["sha256"],
            "hash of `{asset}`"
        );
    }
    let destination = session.path("hd.mp4");
    let probe = ffprobe(&destination);
    assert_stream(&probe, 1920, 1080, 8, "24/1");
    let seconds = video_stream(&probe)["duration"]
        .as_str()
        .unwrap()
        .parse::<f64>()
        .unwrap();
    assert!((seconds - 8.0 / 24.0).abs() < 0.001, "{seconds}");

    // Pixels: the independent oracle, through the codec.
    let frames = decode_frames(&destination, 1920, 1080);
    assert_eq!(frames.len(), 8);
    let worst = assert_oracle_pixels(&frames[3], 1920, 11, 6, "1080p export").max(
        assert_oracle_pixels(&frames[4], 1920, 12, 6, "1080p export"),
    );
    eprintln!("1080p: worst channel delta against the oracle probes: {worst}");

    // Hashes: the same runtime path as `scene.render`, frame by frame.
    let hashes = listed(&result);
    for (index, hash) in hashes.iter().enumerate() {
        assert_eq!(
            hash,
            &rendered_hash(&mut session, 8 + index as i64, 1920, 1080),
            "export frame {index} is not what scene.render draws for edit frame {}",
            8 + index
        );
    }

    // And the picture a viewer sees equals the picture scene.render writes, up to the codec.
    let still = session.path("still.png");
    session.ok(
        json!({"op":"scene.render","scene":"street","frame":11,"width":1920,
        "height":1080,"out":still}),
    );
    let (mean, max) = difference(&frames[3], &png_to_rgb(&still));
    eprintln!("1080p: decoded export frame vs scene.render PNG: mean {mean:.3}, max {max}");
    assert!(mean < 1.5, "mean difference {mean}");

    assert_eq!(
        session.state.timeline.clips.len(),
        clips_before,
        "never a timeline clip"
    );
    assert_eq!(
        session.ok(json!({"op":"history.status"})),
        history_before,
        "no undo entry"
    );
    assert!(session.leftovers().is_empty(), "{:?}", session.leftovers());
}

#[test]
fn a_short_uhd_4k_export_is_a_verified_mp4_with_the_oracle_pixels() {
    let Some(mut session) = session("uhd") else {
        return;
    };
    let started = Instant::now();
    // Edit frames 11 and 12 are consecutive frames the oracle describes.
    let result = session.export("uhd.mp4", json!("uhd4k"), 11, 2);
    eprintln!(
        "UHD 4K: 2 frames in {:?} ({:.0} ms/frame mean), {} bytes",
        started.elapsed(),
        result["metrics"]["render_ms_mean"].as_f64().unwrap(),
        result["bytes"]
    );
    assert_eq!(result["verification"]["ok"], true);
    assert_eq!(result["render"], json!({"width":3840,"height":2160}));
    let destination = session.path("uhd.mp4");
    assert_stream(&ffprobe(&destination), 3840, 2160, 2, "24/1");

    let frames = decode_frames(&destination, 3840, 2160);
    assert_eq!(frames.len(), 2);
    let worst = assert_oracle_pixels(&frames[0], 3840, 11, 12, "UHD export")
        .max(assert_oracle_pixels(&frames[1], 3840, 12, 12, "UHD export"));
    eprintln!("UHD 4K: worst channel delta against the oracle probes: {worst}");
    for (index, hash) in listed(&result).iter().enumerate() {
        assert_eq!(
            hash,
            &rendered_hash(&mut session, 11 + index as i64, 3840, 2160),
            "UHD export frame {index} differs from scene.render"
        );
    }
}

#[test]
fn a_rate_conversion_repeats_scene_frames_and_still_matches_scene_render() {
    let Some(mut session) = session("rate") else {
        return;
    };
    // 24 edit frames at 24 fps through a 30 fps preset: 30 output frames, edit = floor(i*24/30),
    // starting inside the first shot and crossing the cut at 48 when from = 40.
    let thirty = json!({"id":"p30","name":"30 fps","width":640,"height":360,"fps":30});
    let result = session.export("rate.mp4", thirty, 40, 24);
    assert_eq!(result["frames"], 30);
    assert_eq!(result["verification"]["ok"], true);
    assert_eq!(result["warnings"][0]["code"], "frame_rate_conversion");
    assert_stream(&ffprobe(&session.path("rate.mp4")), 640, 360, 30, "30/1");

    let hashes = listed(&result);
    assert_eq!(hashes.len(), 30);
    for (index, hash) in hashes.iter().enumerate() {
        let edit_frame = 40 + (index as i64 * 24) / 30;
        assert_eq!(
            hash,
            &rendered_hash(&mut session, edit_frame, 640, 360),
            "output frame {index} must show edit frame {edit_frame}"
        );
    }
    assert_eq!(hashes[0], hashes[1], "frame 0 is repeated, never blended");
}

#[test]
fn an_export_keeps_its_frozen_resources_when_the_session_is_invalidated() {
    let Some(mut session) = session("frozen") else {
        return;
    };
    let destination = session.path("frozen.mp4");
    let started = session.ok(
        json!({"op":"scene.export","scene":"street","preset":"fullhd",
        "from":0,"frames":16,"destination":destination,"wait":false}),
    );
    let job = started["job"].as_str().unwrap().to_string();
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let info = session.ok(json!({"op":"job.get","job":job}));
        if info["stage"] == "render" && info["done"].as_u64().unwrap_or(0) >= 1 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the export never started rendering: {info}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // Everything approved so far is forgotten: a renderer that read the live session would fail.
    session.ok(json!({"op":"scene.resources.invalidate","reason":"requested"}));
    let info = session.ok(json!({"op":"job.wait","job":job,"timeout_ms":120000}));
    assert_eq!(info["state"], "succeeded", "{info}");
    assert_eq!(info["result"]["verification"]["ok"], true);
    assert_stream(&ffprobe(&destination), 1920, 1080, 16, "24/1");
}

#[test]
fn invalid_requests_are_refused_before_any_job_and_leave_nothing_behind() {
    let Some(mut session) = session("invalid") else {
        return;
    };
    let destination = session.path("never.mp4");
    let base = json!({"op":"scene.export","scene":"street","preset":"fullhd","from":0,"frames":2,
        "destination":destination});
    let describe_before = session.ok(json!({"op":"scene.describe","scene":"street"}));
    let jobs_before = session.ok(json!({"op":"job.list"}));

    let with = |patch: Value| {
        let mut request = base.clone();
        for (key, value) in patch.as_object().unwrap() {
            request[key] = value.clone();
        }
        request
    };
    for (patch, code) in [
        (json!({"scene":"ghost"}), "unknown_element"),
        (json!({"from":-1}), "invalid_range"),
        (json!({"from":96}), "invalid_range"),
        (json!({"frames":0}), "invalid_range"),
        (json!({"from":95,"frames":2}), "invalid_range"),
        (json!({"preset":"nope"}), "unknown_element"),
        (
            json!({"preset":{"id":"odd","name":"odd","width":641,"height":360}}),
            "invalid_value",
        ),
        (json!({"frames":1.5}), "invalid_request"),
    ] {
        let error = session.err(with(patch.clone()));
        assert_eq!(error.code, code, "{patch}: {}", error.message);
    }
    assert!(!destination.exists());
    assert!(session.leftovers().is_empty());
    assert_eq!(
        session.ok(json!({"op":"job.list"})),
        jobs_before,
        "a refused request never creates a job"
    );
    assert_eq!(
        session.ok(json!({"op":"scene.describe","scene":"street"})),
        describe_before
    );

    // Resources not approved for the scene (never resolved / released): refused up front.
    session.ok(json!({"op":"scene.resources.release","scene":"street"}));
    let unapproved = session.err(base.clone());
    assert_eq!(
        unapproved.code, "scene_resources_not_loaded",
        "{}",
        unapproved.message
    );
    assert!(session.leftovers().is_empty());
    assert_eq!(session.ok(json!({"op":"job.list"})), jobs_before);
}

#[test]
fn a_destination_that_exists_or_is_not_an_mp4_fails_the_job_without_touching_anything() {
    let Some(mut session) = session("destination") else {
        return;
    };
    let existing = session.path("keep.mp4");
    fs::write(&existing, b"precious").unwrap();
    // The dialog's question: the same request, answered without a job (headless host, real runtime).
    let asked = session.ok(
        json!({"op":"scene.export.preflight","scene":"street","preset":"fullhd",
        "from":0,"frames":2,"destination":existing}),
    );
    assert_eq!(asked["ok"], false);
    assert_eq!(asked["blockers"][0]["code"], "destination_exists");
    assert_eq!(fs::read(&existing).unwrap(), b"precious");
    let error = session.err(
        json!({"op":"scene.export","scene":"street","preset":"fullhd",
        "from":0,"frames":2,"destination":existing}),
    );
    assert_eq!(error.code, "destination_exists", "{}", error.message);
    assert_eq!(
        fs::read(&existing).unwrap(),
        b"precious",
        "nothing was overwritten"
    );

    let wrong = session.path("movie.mov");
    let error = session.err(
        json!({"op":"scene.export","scene":"street","preset":"fullhd",
        "from":0,"frames":2,"destination":wrong}),
    );
    assert_eq!(error.code, "preflight_failed", "{}", error.message);
    assert!(error.message.contains(".mp4"), "{}", error.message);
    assert!(!wrong.exists());
    assert!(session.leftovers().is_empty(), "{:?}", session.leftovers());

    // `replace: true` swaps the old file in one rename, and only after verification.
    let result = session.ok(
        json!({"op":"scene.export","scene":"street","preset":"fullhd",
        "from":0,"frames":2,"destination":existing,"replace":true}),
    );
    assert_eq!(result["verification"]["ok"], true);
    assert_stream(&ffprobe(&existing), 1920, 1080, 2, "24/1");
}

#[test]
fn declared_audio_is_mixed_video_only_is_explicit_and_unevaluated_domains_are_refused() {
    let Some(mut session) = session("audio") else {
        return;
    };
    // The declared mix is a real WAV next to a copy of the corpus images, so the resources of the
    // scene resolve and the refusal below is about audio support, not about a missing file.
    let base = session.path("sound-base");
    copy_corpus_images(&base);
    fs::create_dir_all(base.join("audio")).unwrap();
    let made = Command::new(Tools::discover().unwrap().ffmpeg)
        .args([
            "-v",
            "error",
            "-nostdin",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=4",
        ])
        .arg(base.join("audio/mix.wav"))
        .status()
        .unwrap();
    assert!(made.success());
    let sum = Command::new("shasum")
        .args(["-a", "256"])
        .arg(base.join("audio/mix.wav"))
        .output()
        .unwrap();
    let hash = String::from_utf8_lossy(&sum.stdout)
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    let mut raw = raw_without_audio();
    raw["audio"] = json!({"mixRelativePath":"audio/mix.wav","editStartSeconds":0.0,
        "editEndSecondsExclusive":4.0,"sourcePcmHashSha256":hash});
    session.ok(json!({"op":"scene.attach","id":"sound","bundle":raw}));
    session.ok(json!({"op":"scene.resources.resolve","scene":"sound","base":base}));
    // The declared mix is in the file by default: a real AAC stream the length of the picture.
    let mixed = session.path("sound-mixed.mp4");
    let result = session.ok(
        json!({"op":"scene.export","scene":"sound","preset":"fullhd",
        "from":0,"frames":2,"destination":mixed}),
    );
    assert_eq!(result["audio"]["included"], true, "{result}");
    assert_eq!(result["verification"]["ok"], true, "{result}");
    let probe = ffprobe(&mixed);
    assert!(
        probe["streams"]
            .as_array()
            .unwrap()
            .iter()
            .any(|stream| stream["codec_type"] == "audio" && stream["codec_name"] == "aac"),
        "{probe}"
    );
    let destination = session.path("sound.mp4");

    let result = session.ok(
        json!({"op":"scene.export","scene":"sound","preset":"fullhd",
        "from":0,"frames":2,"audio":false,"destination":destination}),
    );
    assert_eq!(result["audio"]["included"], false);
    assert_eq!(result["audio"]["declared"], json!(["audio"]));
    assert!(
        result["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning["code"] == "audio_omitted"),
        "{result}"
    );
    assert_stream(&ffprobe(&destination), 1920, 1080, 2, "24/1");

    let mut lit = raw_without_audio();
    lit["sceneDefinitions"][0]["lighting"] = json!({"ambient":{"intensity":0.5}});
    session.ok(json!({"op":"scene.attach","id":"lit","bundle":lit}));
    session.ok(json!({"op":"scene.resources.resolve","scene":"lit","base":corpus()}));
    let error = session.err(json!({"op":"scene.export","scene":"lit","preset":"fullhd",
        "from":0,"frames":2,"destination":session.path("lit.mp4")}));
    assert_eq!(error.code, "unsupported", "{}", error.message);
    assert!(!session.path("lit.mp4").exists());
    assert!(session.leftovers().is_empty());
}

fn copy_corpus_images(base: &Path) {
    for entry in ["assets/sky.png", "assets/mid.png", "assets/hero.png"] {
        let target = base.join(entry);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(corpus().join(entry), target).unwrap();
    }
}

fn raw_without_audio() -> Value {
    serde_json::from_str(&fs::read_to_string(corpus().join("bundle.json")).unwrap()).unwrap()
}

#[test]
fn a_resource_that_vanished_since_it_was_approved_blocks_the_export_in_preflight() {
    let Some(mut session) = session("vanished") else {
        return;
    };
    // The approved files live in the session's own copy, so deleting one touches no fixture.
    let origin = session.path("origin");
    copy_corpus_images(&origin);
    session.ok(json!({"op":"scene.attach","id":"copy",
        "path":corpus().join("bundle.json")}));
    session.ok(json!({"op":"scene.resources.resolve","scene":"copy","base":origin}));
    fs::remove_file(origin.join("assets/hero.png")).unwrap();
    let destination = session.path("vanished.mp4");
    let error = session.err(json!({"op":"scene.export","scene":"copy","preset":"fullhd",
        "from":0,"frames":2,"destination":destination}));
    assert_eq!(error.code, "preflight_failed", "{}", error.message);
    let blockers = error.details.as_ref().unwrap()["blockers"].to_string();
    assert!(blockers.contains("resource_"), "{blockers}");
    assert!(!destination.exists());
    assert!(session.leftovers().is_empty(), "{:?}", session.leftovers());
}

#[test]
fn a_same_size_swap_after_the_map_was_approved_is_refused_by_hash_not_by_size() {
    let Some(mut session) = session("swapped") else {
        return;
    };
    let origin = session.path("origin");
    copy_corpus_images(&origin);
    session.ok(json!({"op":"scene.attach","id":"copy",
        "path":corpus().join("bundle.json")}));
    session.ok(json!({"op":"scene.resources.resolve","scene":"copy","base":origin}));
    // Same length, other bytes: presence and size (the preflight stat) cannot tell.
    let hero = origin.join("assets/hero.png");
    let size = fs::metadata(&hero).unwrap().len() as usize;
    fs::write(&hero, vec![0x5A; size]).unwrap();
    let destination = session.path("swapped.mp4");
    let error = session.err(json!({"op":"scene.export","scene":"copy","preset":"fullhd",
        "from":0,"frames":2,"destination":destination}));
    assert_eq!(error.code, "resource_changed", "{}", error.message);
    assert_eq!(error.details.as_ref().unwrap()["resource"], "asset:hero");
    assert!(!destination.exists());
    assert!(session.leftovers().is_empty(), "{:?}", session.leftovers());
}

#[test]
fn cancelling_an_export_leaves_no_partial_file_and_no_staging_folder() {
    let Some(mut session) = session("cancel") else {
        return;
    };
    let destination = session.path("cancelled.mp4");
    let started = session.ok(
        json!({"op":"scene.export","scene":"street","preset":"fullhd",
        "from":0,"frames":96,"destination":destination,"wait":false}),
    );
    let job = started["job"].as_str().unwrap().to_string();
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let info = session.ok(json!({"op":"job.get","job":job}));
        if info["stage"] == "render" && info["done"].as_u64().unwrap_or(0) >= 2 {
            assert!(
                info["total"] == 96 && info["progress"].as_f64().unwrap() < 1.0,
                "progress is reported per frame: {info}"
            );
            break;
        }
        assert!(Instant::now() < deadline, "never started rendering: {info}");
        std::thread::sleep(Duration::from_millis(20));
    }
    session.ok(json!({"op":"job.cancel","job":job}));
    let info = session.ok(json!({"op":"job.wait","job":job,"timeout_ms":120000}));
    assert_eq!(info["state"], "cancelled", "{info}");
    assert!(!destination.exists(), "no partial file at the destination");
    assert!(session.leftovers().is_empty(), "{:?}", session.leftovers());
}

#[test]
fn the_headless_binary_exports_a_scene_from_a_jsonl_session() {
    if RuntimeConfig::discover().is_err() || Tools::discover().is_err() {
        assert!(std::env::var("TESSEL_REQUIRE_RUNTIME").as_deref() != Ok("1"));
        eprintln!("SKIPPED (runtime or FFmpeg unavailable)");
        return;
    }
    let work = std::env::temp_dir().join(format!("te-r1-{}-cli", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).unwrap();
    let destination = work.join("cli.mp4");
    let lines = [
        json!({"op":"scene.attach","id":"street","path":corpus().join("bundle.json")}),
        json!({"op":"scene.export","scene":"street","preset":"fullhd","from":0,"frames":4,
            "destination":destination,"resources_root":corpus()}),
        json!({"op":"scene.export","scene":"street","preset":"fullhd","from":0,"frames":4,
            "destination":destination}),
    ];
    let mut child = Command::new(env!("CARGO_BIN_EXE_tessel-render"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        for line in &lines {
            writeln!(stdin, "{line}").unwrap();
        }
    }
    let output = child.wait_with_output().unwrap();
    let answers: Vec<Value> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(answers.len(), 3);
    assert_eq!(answers[1]["ok"], true, "{}", answers[1]);
    assert_eq!(answers[1]["result"]["verification"]["ok"], true);
    assert_eq!(
        answers[2]["ok"], false,
        "the second export finds the destination taken"
    );
    assert_eq!(answers[2]["error"]["code"], "destination_exists");
    assert_eq!(output.status.code(), Some(1), "a failed line exits 1");
    assert_stream(&ffprobe(&destination), 1920, 1080, 4, "24/1");
    let _ = fs::remove_dir_all(&work);
}

/// F042A: the same headless binary, a bundle that declares a WAV mix, real frames from the runtime.
/// The mix is 440 Hz for 2 s then 880 Hz; frames 36..60 are scene seconds 1.5–2.5, so the file must
/// carry half a second of each, measured on the decoded AAC. Nothing is heard: the samples are measured.
#[test]
fn the_headless_binary_mixes_the_declared_wav_at_the_requested_range() {
    if RuntimeConfig::discover().is_err() || Tools::discover().is_err() {
        assert!(std::env::var("TESSEL_REQUIRE_RUNTIME").as_deref() != Ok("1"));
        eprintln!("SKIPPED (runtime or FFmpeg unavailable)");
        return;
    }
    let tools = Tools::discover().unwrap();
    let work = std::env::temp_dir().join(format!("te-s9-{}-cli-audio", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).unwrap();
    let base = work.join("base");
    copy_corpus_images(&base);
    fs::create_dir_all(base.join("audio")).unwrap();
    // 4 s at 48 kHz mono: 440 Hz, then 880 Hz after 2 s, rendered by FFmpeg from two sine sources.
    let made = Command::new(&tools.ffmpeg)
        .args(["-v", "error", "-nostdin", "-f", "lavfi", "-i"])
        .arg("sine=frequency=440:duration=2:sample_rate=48000")
        .args(["-f", "lavfi", "-i"])
        .arg("sine=frequency=880:duration=2:sample_rate=48000")
        .args([
            "-filter_complex",
            "[0:a][1:a]concat=n=2:v=0:a=1",
            "-c:a",
            "pcm_s16le",
        ])
        .arg(base.join("audio/mix.wav"))
        .status()
        .unwrap();
    assert!(made.success());
    let hash = Command::new("shasum")
        .args(["-a", "256"])
        .arg(base.join("audio/mix.wav"))
        .output()
        .unwrap();
    let hash = String::from_utf8_lossy(&hash.stdout)
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    let mut raw = raw_without_audio();
    raw["audio"] = json!({"mixRelativePath":"audio/mix.wav","editStartSeconds":0.0,
        "editEndSecondsExclusive":4.0,"sourcePcmHashSha256":hash});
    let destination = work.join("cli-audio.mp4");
    let lines = [
        json!({"op":"scene.attach","id":"sound","bundle":raw}),
        json!({"op":"scene.export","scene":"sound","preset":"fullhd","from":36,"frames":24,
            "destination":destination,"resources_root":base}),
    ];
    let mut child = Command::new(env!("CARGO_BIN_EXE_tessel-render"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        for line in &lines {
            writeln!(stdin, "{line}").unwrap();
        }
    }
    let output = child.wait_with_output().unwrap();
    let answers: Vec<Value> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(answers.len(), 2);
    assert_eq!(answers[1]["ok"], true, "{}", answers[1]);
    let result = &answers[1]["result"];
    assert_eq!(result["verification"]["ok"], true, "{result}");
    assert_eq!(result["audio"]["included"], true);
    assert_eq!(result["audio"]["mix"]["samples"], 48_000);
    assert_eq!(output.status.code(), Some(0));
    let probe = ffprobe(&destination);
    let video = probe["streams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|stream| stream["codec_type"] == "video")
        .expect("a video stream");
    assert_eq!(
        (video["width"].as_u64(), video["height"].as_u64()),
        (Some(1920), Some(1080))
    );
    assert_eq!(video["nb_frames"], "24");
    assert_eq!(video["r_frame_rate"], "24/1");

    let decoded = Command::new(&tools.ffmpeg)
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(&destination)
        .args([
            "-map", "0:a:0", "-f", "s16le", "-ac", "1", "-ar", "48000", "-",
        ])
        .output()
        .unwrap();
    assert!(decoded.status.success());
    let samples: Vec<f64> = decoded
        .stdout
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| f64::from(i16::from_le_bytes(*pair)) / 32768.0)
        .collect();
    assert!(
        (samples.len() as i64 - 48_000).abs() <= 2_048,
        "{}",
        samples.len()
    );
    // Goertzel power of `hz` over the second `from..to` of the decoded stream.
    let power = |from: f64, to: f64, hz: f64| {
        let slice = &samples[(from * 48_000.0) as usize..(to * 48_000.0) as usize];
        let coeff = 2.0 * (2.0 * std::f64::consts::PI * hz / 48_000.0).cos();
        let (mut s1, mut s2) = (0.0f64, 0.0f64);
        for sample in slice {
            let s0 = sample + coeff * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        (s1 * s1 + s2 * s2 - coeff * s1 * s2) / (slice.len() as f64).powi(2)
    };
    assert!(power(0.05, 0.40, 440.0) > 50.0 * power(0.05, 0.40, 880.0));
    assert!(power(0.60, 0.95, 880.0) > 50.0 * power(0.60, 0.95, 440.0));
    let _ = fs::remove_dir_all(&work);
}
