//! F016: the vignette in a real export. Lib + headless host, no window: real Node + headless
//! Chrome + Three.js frames, real FFmpeg, real ffprobe, over the synthetic corpus of
//! `scripts/directed-scene-proof`.
//!
//! What this proves:
//!
//! - the effect is edited through the JSONL surface (`scene.fx.edit`), the export then carries it
//!   (`PlanTimeline` yields the passes per frame, the runtime draws them): the decoded MP4 frames
//!   match the core's reference of the vignette applied to the decoded baseline export, on a grid
//!   of 54 pixels per frame, within the codec tolerance, with a real difference somewhere;
//! - frames outside the clip are the baseline's, byte-identical hashes;
//! - every export frame hash equals what `scene.render` reports for the same edit frame (same plan,
//!   same renderer path);
//! - an undo of the edit gives the baseline export hashes back; the export itself adds no entry to
//!   the history and no timeline clip.
//!
//! Skipped (with the reason on stderr) without Chrome, the runtime bundle or FFmpeg; set
//! `TESSEL_REQUIRE_RUNTIME=1` to turn the skip into a failure. Run under `with-gpu-lock.py`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::fx::apply_pass_to_srgb;
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::{CommandError, EditorState, headless};
use tessel_editor_media::host::MediaHost;
use tessel_editor_media::tools::Tools;
use tessel_editor_render::RuntimeConfig;
use tessel_editor_render::scene_host::SceneHost;

const WIDTH: usize = 1920;
const HEIGHT: usize = 1080;
/// An H.264 CRF 18 round trip on both sides of the comparison, on top of 8-bit rounding.
const CODEC_TOLERANCE: i32 = 16;

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/directed-scene-proof/corpus")
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

    fn export(&mut self, name: &str, from: i64, frames: i64) -> (PathBuf, Vec<String>) {
        let destination = self.work.join(name);
        let result = self.ok(
            json!({"op":"scene.export","scene":"street","preset":"fullhd",
            "from":from,"frames":frames,"destination":destination}),
        );
        assert_eq!(result["verification"]["ok"], true);
        let hashes = result["frame_hashes"]["list"]
            .as_array()
            .expect("a short export lists every frame hash")
            .iter()
            .map(|hash| hash.as_str().unwrap().to_string())
            .collect();
        (destination, hashes)
    }

    fn rendered_hash(&mut self, frame: i64) -> String {
        self.ok(json!({"op":"scene.render","scene":"street","frame":frame,
            "width":WIDTH,"height":HEIGHT}))["frame"]["hash"]
            .as_str()
            .unwrap()
            .to_string()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.work);
    }
}

fn session(name: &str) -> Option<Session> {
    let skip = |reason: &str| {
        assert!(
            std::env::var("TESSEL_REQUIRE_RUNTIME").as_deref() != Ok("1"),
            "the runtime is required but unavailable: {reason}"
        );
        eprintln!("SKIPPED (runtime unavailable): {reason}");
        None
    };
    let config = match RuntimeConfig::discover() {
        Ok(config) => config,
        Err(setup) => return skip(&setup.message()),
    };
    if let Err(error) = Tools::discover() {
        return skip(&error.message);
    }
    let work = std::env::temp_dir().join(format!("te-fx-{}-{name}", std::process::id()));
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

/// RGB24 frames of `path`, decoded with an explicit BT.709 limited-range matrix.
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

fn grid() -> Vec<(usize, usize)> {
    let mut points = Vec::new();
    for row in 0..6 {
        for column in 0..9 {
            points.push(((column * (WIDTH - 1)) / 8, (row * (HEIGHT - 1)) / 5));
        }
    }
    points
}

fn rgb(frame: &[u8], x: usize, y: usize) -> [i32; 3] {
    let at = (y * WIDTH + x) * 3;
    [
        i32::from(frame[at]),
        i32::from(frame[at + 1]),
        i32::from(frame[at + 2]),
    ]
}

#[test]
fn an_export_carries_the_vignette_and_matches_the_reference_over_the_decoded_baseline() {
    let Some(mut session) = session("vignette") else {
        return;
    };
    // The clip covers edit frames 0..48; the range 44..52 straddles its end, so one export holds
    // frames with and without the effect.
    let (from, frames) = (44, 8);
    let (base_mp4, base_hashes) = session.export("base.mp4", from, frames);
    let history_before = session.ok(json!({"op":"history.status"}));

    let edited = session.ok(json!({"op":"scene.fx.edit","scene":"street","ops":[
        {"op":"add_clip","effect":"vignette","start_frame":0,"duration_frames":48,
         "id":"v1","params":{"intensity":1.0}}]}));
    assert_eq!(edited["changed"], true);
    let (fx_mp4, fx_hashes) = session.export("fx.mp4", from, frames);

    // Inside the clip the pictures differ, outside they are the baseline's.
    for index in 0..frames as usize {
        let edit_frame = from + index as i64;
        if edit_frame < 48 {
            assert_ne!(
                fx_hashes[index], base_hashes[index],
                "edit frame {edit_frame} is inside the clip"
            );
        } else {
            assert_eq!(
                fx_hashes[index], base_hashes[index],
                "edit frame {edit_frame} is outside the clip"
            );
        }
        assert_eq!(
            fx_hashes[index],
            session.rendered_hash(edit_frame),
            "export frame {index} is not what scene.render draws for edit frame {edit_frame}"
        );
    }

    // Pixels through the codec: the reference over the decoded baseline, frame by frame.
    let base = decode_frames(&base_mp4, WIDTH, HEIGHT);
    let shaded = decode_frames(&fx_mp4, WIDTH, HEIGHT);
    assert_eq!(
        (base.len(), shaded.len()),
        (frames as usize, frames as usize)
    );
    let (mut worst, mut strongest_change) = (0, 0);
    for index in 0..frames as usize {
        let edit_frame = from + index as i64;
        let passes = session
            .state
            .scenes
            .get("street")
            .unwrap()
            .scene
            .bundle()
            .fx
            .resolve(edit_frame);
        assert_eq!(
            passes.len(),
            usize::from(edit_frame < 48),
            "frame {edit_frame}"
        );
        for (x, y) in grid() {
            let before = rgb(&base[index], x, y);
            let after = rgb(&shaded[index], x, y);
            let mut want = [
                f64::from(before[0] as u8) / 255.0,
                f64::from(before[1] as u8) / 255.0,
                f64::from(before[2] as u8) / 255.0,
            ];
            for pass in &passes {
                want = apply_pass_to_srgb(pass, want, x, y, WIDTH, HEIGHT).unwrap();
            }
            for channel in 0..3 {
                let expected = (want[channel] * 255.0).round() as i32;
                let delta = (after[channel] - expected).abs();
                worst = worst.max(delta);
                assert!(
                    delta <= CODEC_TOLERANCE,
                    "edit frame {edit_frame} at ({x},{y}): export {after:?}, reference {expected} on channel {channel} (baseline {before:?})"
                );
                strongest_change = strongest_change.max((after[channel] - before[channel]).abs());
            }
        }
    }
    eprintln!(
        "EVIDENCE vignette export: worst delta to the reference {worst}, strongest change from the baseline {strongest_change}"
    );
    assert!(
        strongest_change >= 25,
        "the comparison must discriminate: the effect changed at most {strongest_change}"
    );

    // The export added nothing to the history; undoing the edit restores the baseline export.
    let undone_before = session.ok(json!({"op":"history.status"}));
    assert_ne!(
        history_before, undone_before,
        "the edit is one history entry"
    );
    assert_eq!(
        session.state.timeline.clips.len(),
        0,
        "never a timeline clip"
    );
    session.ok(json!({"op":"history.undo"}));
    let (_, restored) = session.export("base2.mp4", from, frames);
    assert_eq!(restored, base_hashes, "undo restores the baseline pictures");
}
