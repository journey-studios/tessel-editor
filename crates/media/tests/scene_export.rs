//! F042 (BL008), the encoder half without a browser: the real FFmpeg pipeline, the real job registry,
//! the real core snapshot and frozen resource map, driven by a stand-in for the *frame source only*.
//!
//! The stand-in (`StillRenderer`) writes one real PNG per frame instead of running Three.js. It is a
//! failure-injection seam for what the encoder must do when the renderer misbehaves (a frame fails,
//! a frame has the wrong size, the user cancels, there is no renderer). It proves nothing about
//! pixels: that is `crates/render/tests/scene_export.rs`, with the real runtime. A machine without
//! FFmpeg skips; `TESSEL_REQUIRE_RUNTIME=1` turns the skip into a failure.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tessel_editor_core::jobs::JobError;
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::{CommandError, EditorState, headless};
use tessel_editor_media::host::MediaHost;
use tessel_editor_media::scene_encoder::{
    FrameJob, RenderedFrame, SceneFrameRenderer, SceneRendererFactory,
};
use tessel_editor_media::tools::Tools;

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/directed-scene-proof/corpus")
}

#[derive(Clone, Copy)]
enum Fault {
    None,
    FailAt(u64),
    WrongSize,
    SlowFrames,
}

struct StillRenderer {
    png: PathBuf,
    fault: Fault,
    drawn: Arc<AtomicU64>,
    finished: Arc<AtomicU64>,
}

impl SceneFrameRenderer for StillRenderer {
    fn max_size(&mut self) -> Result<Option<u32>, JobError> {
        Ok(None)
    }

    fn render(&mut self, job: &FrameJob) -> Result<RenderedFrame, JobError> {
        let index = self.drawn.fetch_add(1, Ordering::SeqCst);
        match self.fault {
            Fault::FailAt(at) if at == index => {
                return Err(JobError::new(
                    "render_failed",
                    "the renderer failed on purpose",
                ));
            }
            Fault::SlowFrames => std::thread::sleep(Duration::from_millis(25)),
            _ => {}
        }
        let source = match self.fault {
            Fault::WrongSize => self.png.with_file_name("small.png"),
            _ => self.png.clone(),
        };
        fs::copy(source, job.png)
            .map_err(|error| JobError::new("render_failed", error.to_string()))?;
        Ok(RenderedFrame {
            hash: format!("still-{index}"),
            render_ms: 1.0,
        })
    }

    fn finish(&mut self) {
        self.finished.fetch_add(1, Ordering::SeqCst);
    }
}

struct Lab {
    state: EditorState,
    host: MediaHost,
    work: PathBuf,
    drawn: Arc<AtomicU64>,
    finished: Arc<AtomicU64>,
}

impl Lab {
    fn run(&mut self, request: Value) -> Result<Value, CommandError> {
        headless::execute_ext(&mut self.state, None, Some(&mut self.host), &request)
    }

    fn ok(&mut self, request: Value) -> Value {
        self.run(request.clone())
            .unwrap_or_else(|error| panic!("{request}: {error:?}"))
    }

    fn export(&mut self, name: &str, frames: i64, wait: bool) -> Result<Value, CommandError> {
        let destination = self.work.join(name);
        self.run(
            json!({"op":"scene.export","scene":"street","preset":preset(),
            "from":0,"frames":frames,"destination":destination,"wait":wait}),
        )
    }

    fn leftovers(&self) -> Vec<String> {
        fs::read_dir(&self.work)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".tessel-"))
            .collect()
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.work);
    }
}

/// The corpus canvas (320x180) as a preset: tiny frames, same aspect, no padding.
fn preset() -> Value {
    json!({"id":"tiny","name":"Tiny","width":320,"height":180,"fps":24})
}

fn lab(name: &str, fault: Fault, with_renderer: bool) -> Option<Lab> {
    let tools = match Tools::discover() {
        Ok(tools) => tools,
        Err(error) => {
            assert!(
                std::env::var("TESSEL_REQUIRE_RUNTIME").as_deref() != Ok("1"),
                "FFmpeg is required: {}",
                error.message
            );
            eprintln!("SKIPPED (FFmpeg unavailable): {}", error.message);
            return None;
        }
    };
    let work = std::env::temp_dir().join(format!("te-r1-{}-m-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).unwrap();
    // Real PNGs made by FFmpeg: the frame the stand-in "renders" and a wrongly sized one.
    for (file, size) in [("frame.png", "320x180"), ("small.png", "160x90")] {
        let made = Command::new(&tools.ffmpeg)
            .args(["-v", "error", "-nostdin", "-f", "lavfi", "-i"])
            .arg(format!("testsrc=size={size}:rate=1"))
            .args(["-frames:v", "1"])
            .arg(work.join(file))
            .status()
            .unwrap();
        assert!(made.success());
    }
    let drawn = Arc::new(AtomicU64::new(0));
    let finished = Arc::new(AtomicU64::new(0));
    let mut host = MediaHost::new();
    if with_renderer {
        let png = work.join("frame.png");
        let (counter, done) = (drawn.clone(), finished.clone());
        let factory: Arc<SceneRendererFactory> = Arc::new(move || {
            Ok(Box::new(StillRenderer {
                png: png.clone(),
                fault,
                drawn: counter.clone(),
                finished: done.clone(),
            }))
        });
        host = host.with_scene_renderer(factory);
    }
    let mut lab = Lab {
        state: EditorState::empty(&ProjectSettings::default()).unwrap(),
        host,
        work,
        drawn,
        finished,
    };
    lab.ok(json!({"op":"scene.attach","id":"street","path":corpus().join("bundle.json")}));
    lab.ok(json!({"op":"scene.resources.resolve","scene":"street","base":corpus()}));
    Some(lab)
}

fn probe_frames(path: &Path) -> String {
    let tools = Tools::discover().unwrap();
    let output = Command::new(tools.ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=nb_frames,width,height,codec_name",
        ])
        .args(["-of", "csv=p=0"])
        .arg(path)
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[test]
fn a_host_without_a_scene_renderer_answers_backend_unavailable() {
    let Some(mut lab) = lab("none", Fault::None, false) else {
        return;
    };
    let error = lab.export("nope.mp4", 2, true).unwrap_err();
    assert_eq!(error.code, "backend_unavailable");
    assert!(!lab.work.join("nope.mp4").exists());
    assert!(lab.leftovers().is_empty());
}

#[test]
fn the_pipeline_encodes_every_frame_in_order_and_publishes_one_verified_file() {
    let Some(mut lab) = lab("ok", Fault::None, true) else {
        return;
    };
    let result = lab.export("ok.mp4", 12, true).unwrap();
    assert_eq!(result["frames"], 12);
    assert_eq!(result["verification"]["ok"], true);
    assert_eq!(result["frame_hashes"]["first"], "still-0");
    assert_eq!(result["frame_hashes"]["last"], "still-11");
    assert_eq!(result["frame_hashes"]["list"].as_array().unwrap().len(), 12);
    assert_eq!(probe_frames(&lab.work.join("ok.mp4")), "h264,320,180,12");
    assert_eq!(lab.drawn.load(Ordering::SeqCst), 12);
    assert_eq!(
        lab.finished.load(Ordering::SeqCst),
        1,
        "the renderer is released once"
    );
    assert!(lab.leftovers().is_empty());
}

#[test]
fn a_frame_that_fails_fails_the_job_and_leaves_no_file_and_no_staging() {
    let Some(mut lab) = lab("fail", Fault::FailAt(3), true) else {
        return;
    };
    let error = lab.export("fail.mp4", 12, true).unwrap_err();
    assert_eq!(error.code, "render_failed", "{}", error.message);
    assert!(!lab.work.join("fail.mp4").exists());
    assert!(lab.leftovers().is_empty(), "{:?}", lab.leftovers());
    assert_eq!(
        lab.finished.load(Ordering::SeqCst),
        1,
        "released even on failure"
    );
}

#[test]
fn a_frame_of_the_wrong_size_is_refused_instead_of_being_scaled_silently() {
    let Some(mut lab) = lab("size", Fault::WrongSize, true) else {
        return;
    };
    let error = lab.export("size.mp4", 4, true).unwrap_err();
    assert!(
        error.message.contains("160") && error.message.contains("320"),
        "{}",
        error.message
    );
    assert!(!lab.work.join("size.mp4").exists());
    assert!(lab.leftovers().is_empty(), "{:?}", lab.leftovers());
}

#[test]
fn cancelling_stops_at_a_frame_boundary_and_leaves_nothing_behind() {
    let Some(mut lab) = lab("cancel", Fault::SlowFrames, true) else {
        return;
    };
    let started = lab.export("cancel.mp4", 96, false).unwrap();
    let job = started["job"].as_str().unwrap().to_string();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let info = lab.ok(json!({"op":"job.get","job":job}));
        if info["stage"] == "render" && info["done"].as_u64().unwrap_or(0) >= 3 {
            break;
        }
        assert!(Instant::now() < deadline, "never rendered: {info}");
        std::thread::sleep(Duration::from_millis(10));
    }
    lab.ok(json!({"op":"job.cancel","job":job}));
    let info = lab.ok(json!({"op":"job.wait","job":job,"timeout_ms":60000}));
    assert_eq!(info["state"], "cancelled", "{info}");
    assert!(info["done"].as_u64().unwrap() < 96, "stopped early: {info}");
    assert!(!lab.work.join("cancel.mp4").exists());
    assert!(lab.leftovers().is_empty(), "{:?}", lab.leftovers());
    assert!(lab.drawn.load(Ordering::SeqCst) < 96);
}

#[test]
fn a_refused_request_never_starts_the_renderer() {
    let Some(mut lab) = lab("refused", Fault::None, true) else {
        return;
    };
    let jobs_before = lab.ok(json!({"op":"job.list"}));
    let error = lab.export("refused.mp4", 0, true).unwrap_err();
    assert_eq!(error.code, "invalid_range");
    let error = lab
        .run(
            json!({"op":"scene.export","scene":"street","preset":preset(),
            "from":0,"frames":2,"destination":lab.work.join("x.mp4"),"audio":"maybe"}),
        )
        .unwrap_err();
    assert_eq!(error.code, "invalid_request");
    assert_eq!(lab.ok(json!({"op":"job.list"})), jobs_before);
    assert_eq!(lab.drawn.load(Ordering::SeqCst), 0);
    assert_eq!(
        lab.finished.load(Ordering::SeqCst),
        0,
        "no renderer was even built"
    );
}

#[test]
fn preflight_reports_what_would_block_without_a_job_a_file_or_a_renderer() {
    let Some(mut lab) = lab("preflight", Fault::None, false) else {
        return;
    };
    let jobs_before = lab.ok(json!({"op":"job.list"}));
    let ask = |lab: &mut Lab, name: &str| {
        let destination = lab.work.join(name);
        lab.ok(
            json!({"op":"scene.export.preflight","scene":"street","preset":preset(),
            "from":0,"frames":6,"destination":destination}),
        )
    };

    let clear = ask(&mut lab, "fresh.mp4");
    assert_eq!(clear["ok"], true, "{clear}");
    assert_eq!(clear["summary"]["frames"], 6);
    assert_eq!(clear["summary"]["scene"], "street");

    fs::write(lab.work.join("taken.mp4"), b"x").unwrap();
    let taken = ask(&mut lab, "taken.mp4");
    assert_eq!(taken["ok"], false);
    assert_eq!(taken["blockers"][0]["code"], "destination_exists");

    let wrong = ask(&mut lab, "movie.mov");
    assert_eq!(wrong["blockers"][0]["code"], "invalid_destination");

    assert!(!lab.work.join("fresh.mp4").exists());
    assert_eq!(fs::read(lab.work.join("taken.mp4")).unwrap(), b"x");
    assert_eq!(lab.ok(json!({"op":"job.list"})), jobs_before);
    assert!(lab.leftovers().is_empty());
}
