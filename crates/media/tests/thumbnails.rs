//! BL144: library thumbnails of splats and scenes as jobs. A renderer double draws a flat colour
//! per asset at exactly the size it is asked for, so what is proved here is the adapter's
//! contract (one source plan per asset, the cache, failures, cancellation, identity), while
//! `crates/render/tests/thumbnail_frames.rs` proves the pixels of the real runtime.

mod common;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tessel_editor_core::backend::{BackendError, Capabilities, FrameResult, Output};
use tessel_editor_core::content::Camera;
use tessel_editor_core::fixtures::{StandardSet, encode_png, scene_json, write_standard_set};
use tessel_editor_core::plan::{Content, PreviewIntent, RenderPlan};
use tessel_editor_core::session::Renderer;
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::thumbnails::{WIDTH, size_for};
use tessel_editor_core::{CommandError, EditorState, headless};
use tessel_editor_media::derive::png_dimensions;
use tessel_editor_media::host::{MediaHost, RendererFactory};

use common::{temp, tools};

/// One frame the double was asked for.
#[derive(Debug, Clone, PartialEq)]
struct Frame {
    asset: String,
    /// Splats the scene file of that asset draws.
    splats: Vec<String>,
    width: u32,
    height: u32,
    /// The camera of the plan's scene layer.
    camera: Option<Camera>,
    /// `(position, scale)` of every splat object of the scene file, as the file says.
    placed: Vec<([f64; 3], f64)>,
    /// How many probe pixels the frame was asked to report.
    probes: usize,
}

/// What the double saw and how it should misbehave, shared by every renderer the factory builds.
#[derive(Default)]
struct Script {
    /// Every frame asked, in order.
    frames: Mutex<Vec<Frame>>,
    /// Renderers built (each is a Chrome in real life).
    built: AtomicUsize,
    /// Assets whose frame fails.
    broken: Mutex<HashSet<String>>,
    /// Assets whose frame comes back as the background and nothing else (a cloud outside the picture).
    blank: Mutex<HashSet<String>>,
    /// While set, a frame waits for `release` (a render that is still running).
    hold: AtomicBool,
    release: AtomicBool,
}

struct Double(Arc<Script>);

impl Renderer for Double {
    fn render_frame(
        &mut self,
        state: &EditorState,
        plan: RenderPlan,
        output: Output,
        probes: Vec<(u32, u32)>,
        _timeout: Duration,
    ) -> Result<FrameResult, BackendError> {
        let PreviewIntent::Source { asset, .. } = plan.intent else {
            panic!("a thumbnail is a source plan, got {:?}", plan.intent);
        };
        let camera = plan.layers.iter().find_map(|layer| match &layer.content {
            Content::Scene { camera, .. } => Some(*camera),
            _ => None,
        });
        let asset = asset.to_string();
        // What the scene file of that asset puts on screen, read the way the runtime would.
        let item = state
            .library
            .item(&asset)
            .expect("the plan names a library asset");
        let (splats, placed): (Vec<String>, Vec<([f64; 3], f64)>) =
            match &item.asset.as_ref().unwrap().source {
                tessel_editor_core::assets::Source::File { path }
                    if item.kind.label() == "scene" =>
                {
                    let scene: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
                    let objects: Vec<&Value> = scene["objects"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|object| object["type"] == "splat")
                        .collect();
                    let vector = |value: &Value| -> [f64; 3] {
                        match value.as_array() {
                            Some(items) => std::array::from_fn(|i| items[i].as_f64().unwrap()),
                            None => [0.0; 3],
                        }
                    };
                    (
                        objects
                            .iter()
                            .map(|object| object["asset"].as_str().unwrap().to_string())
                            .collect(),
                        objects
                            .iter()
                            .map(|object| {
                                (
                                    vector(&object["position"]),
                                    object["scale"].as_f64().unwrap_or(1.0),
                                )
                            })
                            .collect(),
                    )
                }
                _ => (Vec::new(), Vec::new()),
            };
        self.0.frames.lock().unwrap().push(Frame {
            asset: asset.clone(),
            splats,
            width: output.width,
            height: output.height,
            camera,
            placed,
            probes: probes.len(),
        });
        while self.0.hold.load(Ordering::SeqCst) && !self.0.release.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        if self.0.broken.lock().unwrap().contains(&asset) {
            return Err(BackendError::new(
                "render_failed",
                "the double was told to fail",
            ));
        }
        let blank = self.0.blank.lock().unwrap().contains(&asset);
        let seed = asset
            .bytes()
            .fold(7u8, |sum, byte| sum.wrapping_mul(31).wrapping_add(byte));
        let (width, height) = (output.width as usize, output.height as usize);
        let rgba: Vec<u8> = (0..width * height)
            .flat_map(|_| [seed, seed.wrapping_mul(3), 255 - seed, 255])
            .collect();
        let path = output.png.expect("a thumbnail asks for a PNG");
        fs::write(&path, encode_png(output.width, output.height, &rgba)).unwrap();
        Ok(FrameResult {
            id: 0,
            revision: 0,
            width: output.width,
            height: output.height,
            hash: String::new(),
            // The stand-in's background (10, 13, 26) when told the cloud is outside the picture.
            mean: if blank {
                [10.0, 13.0, 26.0, 255.0]
            } else {
                [0.0; 4]
            },
            probes: if blank {
                vec![[10, 13, 26, 255]; probes.len()]
            } else {
                Vec::new()
            },
            png: Some(path),
            render_ms: 0.0,
            total_ms: 0.0,
            detail: Default::default(),
        })
    }

    fn capabilities(&mut self) -> Result<Capabilities, BackendError> {
        unreachable!("thumbnails do not ask for capabilities")
    }

    fn summary(&self) -> Value {
        json!({})
    }

    fn reset_project(&mut self) {}
}

fn host(script: &Arc<Script>, interactive: bool) -> MediaHost {
    let script = script.clone();
    let factory: Arc<RendererFactory> = Arc::new(move || {
        script.built.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(Double(script.clone())))
    });
    let host = MediaHost::new().with_renderer(factory).with_tools(tools());
    if interactive {
        host.interactive()
    } else {
        host
    }
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

/// A project with a workspace and `paths` imported in that order (hashed, in a workspace of its own).
fn project_of<P: AsRef<Path>>(dir: &Path, name: &str, paths: &[P]) -> EditorState {
    let paths: Vec<&Path> = paths.iter().map(AsRef::as_ref).collect();
    let mut state = EditorState::empty(&ProjectSettings {
        name: name.to_string(),
        workspace: Some(dir.join(name)),
        ..ProjectSettings::default()
    })
    .unwrap();
    let mut importer = MediaHost::new().with_tools(tools());
    let report = ok(
        &mut importer,
        &mut state,
        json!({"op":"media.import","paths":paths}),
    );
    assert_eq!(report["applied"]["imported"], paths.len(), "{report}");
    state
}

/// A project with a workspace, the splat and the scene of the standard set imported (hashed).
fn project(dir: &Path, name: &str, set: &StandardSet) -> EditorState {
    project_of(dir, name, &[&set.splat, &set.scene])
}

/// A `.splat` file with one record per position (the 32-byte layout).
fn write_cloud(path: &Path, points: &[[f32; 3]]) {
    let mut bytes = Vec::new();
    for point in points {
        for value in point.iter().chain(&[0.05f32, 0.05, 0.03]) {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&[200, 120, 60, 255]);
        bytes.extend_from_slice(&[255, 128, 128, 128]);
    }
    fs::write(path, bytes).unwrap();
}

/// The eight corners of a cube and its centre: a cloud whose box is exactly the cube.
fn cube(center: [f32; 3], half: f32) -> Vec<[f32; 3]> {
    let mut points: Vec<[f32; 3]> = (0..8)
        .map(|corner| {
            std::array::from_fn(|axis| {
                center[axis]
                    + if (corner >> axis) & 1 == 1 {
                        half
                    } else {
                        -half
                    }
            })
        })
        .collect();
    points.push(center);
    points
}

/// Pixel of `point` seen from `camera` (a pinhole looking at its target, +Y up), computed here
/// and independent of the code that framed the cloud.
fn project_point(camera: &Camera, point: [f64; 3], width: f64, height: f64) -> Option<(f64, f64)> {
    let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let cross = |a: [f64; 3], b: [f64; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let unit = |a: [f64; 3]| {
        let length = dot(a, a).sqrt();
        [a[0] / length, a[1] / length, a[2] / length]
    };
    let forward = unit(sub(camera.target, camera.position));
    let right = unit(cross(forward, [0.0, 1.0, 0.0]));
    let up = cross(right, forward);
    let offset = sub(point, camera.position);
    let depth = dot(offset, forward);
    if depth <= 0.0 {
        return None;
    }
    let focal = height / 2.0 / (camera.fov / 2.0).to_radians().tan();
    Some((
        width / 2.0 + focal * dot(offset, right) / depth,
        height / 2.0 - focal * dot(offset, up) / depth,
    ))
}

fn id_of(state: &EditorState, kind: &str) -> String {
    state
        .library
        .items
        .iter()
        .find(|item| item.kind.label() == kind)
        .unwrap_or_else(|| panic!("no {kind} in the library"))
        .id
        .to_string()
}

fn png_size(path: &str) -> (u32, u32) {
    let bytes = fs::read(path).unwrap();
    assert_eq!(&bytes[1..4], b"PNG", "{path} is a PNG");
    let word = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    (word(16), word(20))
}

fn poll_until(
    host: &mut MediaHost,
    state: &mut EditorState,
    mut done: impl FnMut(&EditorState) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while !done(state) {
        host.poll(state);
        assert!(Instant::now() < deadline, "the condition was never met");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn setup(name: &str) -> (PathBuf, StandardSet) {
    let dir = temp(name);
    let set = write_standard_set(&dir.join("set")).unwrap();
    (dir, set)
}

#[test]
fn a_run_draws_one_source_frame_per_splat_and_scene_at_the_sequence_aspect() {
    let (dir, set) = setup("explicit");
    let mut state = project(&dir, "film", &set);
    let (splat, scene) = (id_of(&state, "splat"), id_of(&state, "scene"));
    let script = Arc::new(Script::default());
    let mut host = host(&script, false);
    let revision = state.project.revision;

    let report = ok(&mut host, &mut state, json!({"op":"media.thumbnails"}));
    assert_eq!(report["ready"], 2, "{report}");
    assert_eq!(report["applied"]["failed"], json!([]));
    let (width, height) = size_for(state.timeline.width, state.timeline.height);
    // The scene is drawn as itself; the bare splat through a one-object stand-in scene.
    let frames = script.frames.lock().unwrap().clone();
    let drawn: Vec<_> = frames
        .iter()
        .map(|frame| {
            (
                frame.asset.clone(),
                frame.splats.clone(),
                frame.width,
                frame.height,
            )
        })
        .collect();
    assert_eq!(
        drawn,
        vec![
            (
                state.thumbnail_stand_in_id(&splat),
                vec![splat.clone()],
                width,
                height
            ),
            (scene.clone(), vec![splat.clone()], width, height)
        ]
    );
    assert_eq!(
        frames.iter().map(|frame| frame.probes).collect::<Vec<_>>(),
        [25, 0],
        "a bare splat is also asked for the probes that tell an empty picture; a scene is not"
    );
    assert!(
        state
            .library
            .items
            .iter()
            .all(|item| !item.id.as_str().starts_with("thumb-")),
        "the stand-in lives only in the job's copy of the project"
    );
    assert_eq!(
        script.built.load(Ordering::SeqCst),
        1,
        "one renderer for the whole job"
    );

    let (a, b) = (
        state.thumbnail(&splat).unwrap(),
        state.thumbnail(&scene).unwrap(),
    );
    assert_eq!((a.width, a.height), (WIDTH, height));
    assert_eq!(
        png_size(&a.path),
        (WIDTH, height),
        "the file is the size it says"
    );
    assert_ne!(
        fs::read(&a.path).unwrap(),
        fs::read(&b.path).unwrap(),
        "one picture per asset"
    );
    assert!(a.path.contains(".tessel/cache/"), "{}", a.path);
    assert_eq!(
        state.project.revision, revision,
        "a thumbnail is not an edit"
    );
    assert!(!state.project.is_dirty() || state.project.saved_revision.is_none());

    // Nothing left to draw, and an unknown asset is a typed error.
    let again = ok(&mut host, &mut state, json!({"op":"media.thumbnails"}));
    assert_eq!(again["assets"], json!([]));
    let unknown = run(
        &mut host,
        &mut state,
        json!({"op":"media.thumbnails","assets":["ghost"]}),
    );
    assert_eq!(unknown.unwrap_err().code, "unknown_element");
}

#[test]
fn the_cache_serves_a_picture_without_starting_a_renderer() {
    let (dir, set) = setup("cache");
    let mut state = project(&dir, "film", &set);
    let splat = id_of(&state, "splat");
    let script = Arc::new(Script::default());
    let mut host = host(&script, false);
    ok(&mut host, &mut state, json!({"op":"media.thumbnails"}));
    let path = state.thumbnail(&splat).unwrap().path.clone();
    assert_eq!(script.built.load(Ordering::SeqCst), 1);

    // The session forgets its book (another opening of the project); the files are still there.
    state.thumbnails = Default::default();
    let report = ok(&mut host, &mut state, json!({"op":"media.thumbnails"}));
    assert_eq!(report["ready"], 2);
    assert_eq!(
        script.built.load(Ordering::SeqCst),
        1,
        "the cache answered, no new renderer"
    );
    assert_eq!(
        script.frames.lock().unwrap().len(),
        2,
        "no frame was drawn again"
    );
    assert_eq!(state.thumbnail(&splat).unwrap().path, path);

    // A different file under the same asset ID is a different cache entry, drawn afresh. The scene
    // is drawn with that splat, so it is drawn again too, though its own file did not change.
    let scene = id_of(&state, "scene");
    let scene_path = state.thumbnail(&scene).unwrap().path.clone();
    let asset = state.library.items[0].asset.as_mut().unwrap();
    asset.identity.hash = Some("0".repeat(64));
    let report = ok(&mut host, &mut state, json!({"op":"media.thumbnails"}));
    assert_eq!(report["assets"], json!([splat, scene]), "{report}");
    assert_eq!(
        script.frames.lock().unwrap().len(),
        4,
        "the relinked splat and the scene drawn with it, nothing else"
    );
    assert_ne!(state.thumbnail(&splat).unwrap().path, path);
    assert_ne!(
        state.thumbnail(&scene).unwrap().path,
        scene_path,
        "the old picture of the scene stays where it was, and is not the one shown"
    );
    assert!(Path::new(&scene_path).is_file());
}

#[test]
fn a_scene_that_fails_costs_the_others_nothing_and_can_be_retried() {
    let (dir, set) = setup("failure");
    let mut state = project(&dir, "film", &set);
    let (splat, scene) = (id_of(&state, "splat"), id_of(&state, "scene"));
    let script = Arc::new(Script::default());
    script.broken.lock().unwrap().insert(scene.clone());
    let mut host = host(&script, false);

    let report = ok(&mut host, &mut state, json!({"op":"media.thumbnails"}));
    assert_eq!(report["applied"]["applied"], json!([splat]));
    assert_eq!(report["applied"]["failed"], json!([scene]));
    assert!(state.thumbnail(&splat).is_some());
    assert!(
        state.thumbnail(&scene).is_none(),
        "no picture for the broken one"
    );
    let failure = state.thumbnail_report(&scene);
    assert_eq!(failure["state"], "failed");
    assert_eq!(failure["code"], "render_failed");
    assert!(
        failure["message"]
            .as_str()
            .unwrap()
            .contains("told to fail"),
        "{failure}"
    );

    // A failure is not asked again until someone asks to retry.
    let quiet = ok(&mut host, &mut state, json!({"op":"media.thumbnails"}));
    assert_eq!(quiet["assets"], json!([]));
    script.broken.lock().unwrap().clear();
    let retried = ok(
        &mut host,
        &mut state,
        json!({"op":"media.thumbnails","retry":true}),
    );
    assert_eq!(retried["applied"]["applied"], json!([scene]));
    assert_eq!(state.thumbnail_report(&scene)["state"], "ready");
}

#[test]
fn without_a_renderer_the_failure_is_named_not_hidden() {
    let (dir, set) = setup("norenderer");
    let mut state = project(&dir, "film", &set);
    let mut host = MediaHost::new().with_tools(tools());
    let report = ok(&mut host, &mut state, json!({"op":"media.thumbnails"}));
    assert_eq!(report["ready"], 0);
    let splat = id_of(&state, "splat");
    let failure = state.thumbnail_report(&splat);
    assert_eq!(failure["state"], "failed");
    assert_eq!(failure["code"], "backend_unavailable");
}

#[test]
fn a_window_host_draws_what_a_project_lacks_by_itself_and_does_not_loop_on_failures() {
    let (dir, set) = setup("auto");
    let mut state = project(&dir, "film", &set);
    let (splat, scene) = (id_of(&state, "splat"), id_of(&state, "scene"));
    let script = Arc::new(Script::default());
    script.broken.lock().unwrap().insert(scene.clone());
    let mut host = host(&script, true);

    poll_until(&mut host, &mut state, |state| {
        state.thumbnail(&splat).is_some() && state.thumbnail_failure(&scene).is_some()
    });
    let built = script.built.load(Ordering::SeqCst);
    assert_eq!(built, 1);
    for _ in 0..20 {
        host.poll(&mut state);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        script.built.load(Ordering::SeqCst),
        built,
        "a failure is not retried every tick"
    );
    assert_eq!(script.frames.lock().unwrap().len(), 2);
}

#[test]
fn a_project_without_a_workspace_waits_for_one() {
    let (_, set) = setup("noworkspace");
    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    let mut importer = MediaHost::new().with_tools(tools());
    // No workspace: nowhere to keep media or cache, and the import says so.
    assert_eq!(
        run(
            &mut importer,
            &mut state,
            json!({"op":"media.import","paths":[set.splat]})
        )
        .unwrap_err()
        .code,
        "no_workspace"
    );
    let script = Arc::new(Script::default());
    let mut host = host(&script, true);
    for _ in 0..5 {
        host.poll(&mut state);
    }
    assert_eq!(script.built.load(Ordering::SeqCst), 0);
}

#[test]
fn the_job_of_a_project_that_was_replaced_is_cancelled_and_never_applied_to_the_next() {
    let (dir, set) = setup("replaced");
    let mut first = project(&dir, "first", &set);
    let (splat, scene) = (id_of(&first, "splat"), id_of(&first, "scene"));
    let script = Arc::new(Script::default());
    script.hold.store(true, Ordering::SeqCst);
    let mut host = host(&script, true);

    // The first project's job starts and is stuck inside its first frame.
    let deadline = Instant::now() + Duration::from_secs(60);
    while script.frames.lock().unwrap().is_empty() {
        host.poll(&mut first);
        assert!(
            Instant::now() < deadline,
            "the job never reached the renderer"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let running = host
        .registry()
        .list()
        .into_iter()
        .find(|job| job.kind.label() == "thumbnail")
        .unwrap();
    assert!(!running.state.is_final());

    // Another project takes the window's place (same asset IDs, same files: a trap on purpose).
    let mut second = project(&dir, "second", &set);
    assert_eq!(id_of(&second, "splat"), splat);
    let events = host.poll(&mut second);
    assert!(events.is_empty(), "nothing finished yet: {events:?}");
    let cancelling = host.registry().get(&running.id).unwrap();
    assert!(
        cancelling.cancel_requested,
        "the old project's job is asked to stop"
    );

    // Its renderer finally answers; the job stops at the next target and applies nothing.
    script.hold.store(false, Ordering::SeqCst);
    script.release.store(true, Ordering::SeqCst);
    let registry = host.registry().clone();
    poll_until(&mut host, &mut second, |_| {
        registry
            .get(&running.id)
            .is_some_and(|job| job.state.is_final())
    });
    let ended = registry.get(&running.id).unwrap();
    assert_eq!(ended.state.label(), "cancelled", "{ended:?}");

    // Whatever the second project ends up with was drawn for it, by its own job.
    poll_until(&mut host, &mut second, |state| {
        state.thumbnail(&splat).is_some() && state.thumbnail(&scene).is_some()
    });
    assert!(
        first.thumbnails.is_empty(),
        "nothing was ever applied to the project that was replaced"
    );
    let jobs: Vec<_> = host
        .registry()
        .list()
        .into_iter()
        .filter(|job| job.kind.label() == "thumbnail")
        .collect();
    assert_eq!(
        jobs.len(),
        2,
        "one cancelled job and the second project's own"
    );
}

#[test]
fn a_finished_result_of_a_replaced_project_is_refused_when_it_is_committed() {
    let (dir, set) = setup("commit");
    let mut first = project(&dir, "first", &set);
    let splat = id_of(&first, "splat");
    let script = Arc::new(Script::default());
    let mut host = host(&script, false);

    let queued = ok(
        &mut host,
        &mut first,
        json!({"op":"media.thumbnails","wait":false}),
    );
    let job = queued["job"].as_str().unwrap().to_string();
    ok(&mut host, &mut first, json!({"op":"job.wait","job":job}));

    // The same document, opened again in another state: same IDs, hashes and files.
    let mut second = project(&dir, "second", &set);
    let committed = ok(&mut host, &mut second, json!({"op":"job.commit","job":job}));
    assert_eq!(committed["applied"]["stale"], true, "{committed}");
    assert!(second.thumbnails.is_empty());
    assert!(second.thumbnail(&splat).is_none());
}

/// The splat stand-in frames of `state`, keyed by the splat each one draws.
fn stand_in_frame(script: &Script, state: &EditorState, splat: &str) -> Frame {
    let id = state.thumbnail_stand_in_id(splat);
    script
        .frames
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|frame| frame.asset == id)
        .cloned()
        .unwrap_or_else(|| panic!("no frame of the stand-in `{id}` for `{splat}`"))
}

/// Every corner of the cube lands inside the picture as the frame was framed, and the centre
/// lands in the middle.
fn assert_framed(frame: &Frame, center: [f32; 3], half: f32) {
    let (position, scale) = frame.placed[0];
    let camera = frame
        .camera
        .expect("a splat stand-in carries the camera of the framing");
    let (width, height) = (f64::from(frame.width), f64::from(frame.height));
    let middle = project_point(
        &camera,
        std::array::from_fn(|axis| f64::from(center[axis]) * scale + position[axis]),
        width,
        height,
    )
    .expect("the centre is in front of the camera");
    assert!(
        (middle.0 - width / 2.0).abs() < 0.5 && (middle.1 - height / 2.0).abs() < 0.5,
        "the cloud at {center:?} is centred, got {middle:?}"
    );
    for corner in cube(center, half) {
        let (x, y) = project_point(
            &camera,
            std::array::from_fn(|axis| f64::from(corner[axis]) * scale + position[axis]),
            width,
            height,
        )
        .unwrap_or_else(|| panic!("{corner:?} is behind the camera"));
        assert!(
            (0.0..width).contains(&x) && (0.0..height).contains(&y),
            "{corner:?} of the cloud at {center:?} (half {half}) lands at ({x}, {y}) outside {width}x{height}"
        );
    }
    let distance = camera
        .position
        .iter()
        .zip(camera.target)
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f64>()
        .sqrt();
    assert!(
        (0.05..900.0).contains(&distance),
        "the camera stays between the near and far planes whatever the cloud's scale: {distance}"
    );
}

#[test]
fn displaced_and_real_scale_clouds_are_each_framed_from_their_own_file() {
    let (dir, _) = setup("framing");
    let files = dir.join("clouds");
    fs::create_dir_all(&files).unwrap();
    // A displaced cloud, a tiny one (a few millimetres), and one at the scale and distance of a
    // real capture (tens of thousands of units from the origin).
    // The last one has the longest ID an asset can have: `thumb-<id>` would not be an ID.
    let long = format!("long-{}", "x".repeat(59));
    assert_eq!(long.len(), 64);
    let cases: [(&str, [f32; 3], f32); 5] = [
        ("origin", [0.0, 0.0, 0.0], 1.0),
        ("far", [1000.0, -50.0, 300.0], 10.0),
        ("tiny", [0.0, 0.1, 5.0], 0.002),
        ("capture", [-80_000.0, 20_000.0, 90_000.0], 3_000.0),
        (long.as_str(), [12.0, 12.0, -12.0], 2.0),
    ];
    let mut paths = Vec::new();
    for (name, center, half) in cases {
        let path = files.join(format!("{name}.splat"));
        write_cloud(&path, &cube(center, half));
        paths.push(path);
    }
    let paths: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
    let mut state = project_of(&dir, "film", &paths);
    let script = Arc::new(Script::default());
    let mut host = host(&script, false);

    let report = ok(&mut host, &mut state, json!({"op":"media.thumbnails"}));
    assert_eq!(report["ready"], 5, "{report}");
    assert_eq!(
        script.frames.lock().unwrap().len(),
        5,
        "several splats in one session, one frame each"
    );
    for (name, center, half) in cases {
        assert_framed(&stand_in_frame(&script, &state, name), center, half);
        assert_eq!(state.thumbnail_report(name)["state"], "ready", "{name}");
    }
    // The stand-ins are different scenes with different IDs inside one job.
    let ids: HashSet<_> = script
        .frames
        .lock()
        .unwrap()
        .iter()
        .map(|frame| frame.asset.clone())
        .collect();
    assert_eq!(ids.len(), 5);
    assert!(ids.iter().all(|id| id.len() <= 64), "{ids:?}");
}

#[test]
fn a_reused_asset_id_with_another_file_is_framed_again_not_served_from_the_other() {
    let (dir, _) = setup("reused");
    let files = dir.join("clouds");
    fs::create_dir_all(&files).unwrap();
    let script = Arc::new(Script::default());
    let mut host = host(&script, false);

    // The same ID (`shot`), the same workspace, a cloud somewhere else each time.
    let path = files.join("shot.splat");
    write_cloud(&path, &cube([500.0, 0.0, 0.0], 4.0));
    let mut first = project_of(&dir, "film", &[&path]);
    ok(&mut host, &mut first, json!({"op":"media.thumbnails"}));
    let first_path = first.thumbnail("shot").unwrap().path.clone();
    assert_framed(
        &stand_in_frame(&script, &first, "shot"),
        [500.0, 0.0, 0.0],
        4.0,
    );

    write_cloud(&path, &cube([-7.0, 300.0, 40.0], 0.5));
    let mut second = project_of(&dir, "film", &[&path]);
    ok(&mut host, &mut second, json!({"op":"media.thumbnails"}));
    assert_eq!(
        script.frames.lock().unwrap().len(),
        2,
        "the other file is drawn, not looked up by ID"
    );
    assert_framed(
        &stand_in_frame(&script, &second, "shot"),
        [-7.0, 300.0, 40.0],
        0.5,
    );
    assert_ne!(second.thumbnail("shot").unwrap().path, first_path);
}

#[test]
fn a_cloud_that_cannot_be_framed_or_came_out_empty_is_named_never_offered() {
    let (dir, _) = setup("undrawable");
    let files = dir.join("clouds");
    fs::create_dir_all(&files).unwrap();
    // Every splat at one point: there is no extent to frame.
    write_cloud(
        &files.join("dot.splat"),
        &[[3.0, 3.0, 3.0], [3.0, 3.0, 3.0]],
    );
    // A good cloud the renderer returns as the background only (outside the picture).
    write_cloud(&files.join("lost.splat"), &cube([10.0, 10.0, 10.0], 1.0));
    let mut state = project_of(
        &dir,
        "film",
        &[&files.join("dot.splat"), &files.join("lost.splat")],
    );
    let script = Arc::new(Script::default());
    script
        .blank
        .lock()
        .unwrap()
        .insert(state.thumbnail_stand_in_id("lost"));
    let mut host = host(&script, false);

    let report = ok(&mut host, &mut state, json!({"op":"media.thumbnails"}));
    assert_eq!(report["ready"], 0, "{report}");
    let dot = state.thumbnail_report("dot");
    assert_eq!(
        (dot["state"].as_str(), dot["code"].as_str()),
        (Some("failed"), Some("splat_not_drawable")),
        "{dot}"
    );
    let lost = state.thumbnail_report("lost");
    assert_eq!(
        (lost["state"].as_str(), lost["code"].as_str()),
        (Some("failed"), Some("empty_picture")),
        "{lost}"
    );
    assert!(state.thumbnail("dot").is_none() && state.thumbnail("lost").is_none());
    assert_eq!(
        script.frames.lock().unwrap().len(),
        1,
        "the point cloud never reached the renderer; the empty one did"
    );

    // Nothing was cached for the empty one: once the renderer draws it, it is a picture.
    script.blank.lock().unwrap().clear();
    let retried = ok(
        &mut host,
        &mut state,
        json!({"op":"media.thumbnails","retry":true}),
    );
    assert_eq!(retried["applied"]["applied"], json!(["lost"]), "{retried}");
    assert_eq!(state.thumbnail_report("dot")["state"], "failed");
}

#[test]
fn a_scene_with_a_missing_splat_fails_by_name_and_draws_once_the_splat_is_imported() {
    let (dir, set) = setup("dependency");
    let mut state = project(&dir, "film", &set);
    let (splat, scene) = (id_of(&state, "splat"), id_of(&state, "scene"));
    let script = Arc::new(Script::default());
    let mut host = host(&script, false);
    // The splat is gone from the library (removed, or not restored with the project).
    state.library.items.retain(|item| item.id != splat.as_str());

    let report = ok(&mut host, &mut state, json!({"op":"media.thumbnails"}));
    assert_eq!(report["ready"], 0, "{report}");
    let failure = state.thumbnail_report(&scene);
    assert_eq!(failure["code"], "missing_dependency", "{failure}");
    assert!(
        failure["message"].as_str().unwrap().contains(&splat),
        "{failure}"
    );
    assert_eq!(
        script.built.load(Ordering::SeqCst),
        0,
        "no renderer for what cannot be drawn"
    );
    let quiet = ok(&mut host, &mut state, json!({"op":"media.thumbnails"}));
    assert_eq!(
        quiet["assets"],
        json!([]),
        "remembered, not asked for again"
    );

    // The splat is imported: the remembered failure is not the state of the scene any more.
    let mut importer = MediaHost::new().with_tools(tools());
    ok(
        &mut importer,
        &mut state,
        json!({"op":"media.import","paths":[set.splat]}),
    );
    assert_eq!(state.thumbnail_report(&scene)["state"], "pending");
    let drawn = ok(&mut host, &mut state, json!({"op":"media.thumbnails"}));
    assert_eq!(drawn["ready"], 2, "{drawn}");
    assert_eq!(state.thumbnail_report(&scene)["state"], "ready");
}

#[test]
fn a_stand_in_never_takes_the_id_of_an_asset_the_job_holds() {
    let (dir, set) = setup("collision");
    // A splat literally named like the stand-in `cloud` would get, drawn into a scene.
    let taken = EditorState::empty(&ProjectSettings::default())
        .unwrap()
        .thumbnail_stand_in_id("cloud");
    let squatter = dir.join(format!("{taken}.splat"));
    write_cloud(&squatter, &cube([1.0, 2.0, 3.0], 1.0));
    let scene = dir.join("squat.tessel-scene.json");
    fs::write(&scene, scene_json(&taken)).unwrap();
    let mut state = project_of(&dir, "film", &[&set.splat, &squatter, &scene]);
    let script = Arc::new(Script::default());
    let mut host = host(&script, false);

    let report = ok(&mut host, &mut state, json!({"op":"media.thumbnails"}));
    assert_eq!(report["ready"], 3, "{report}");
    let frames = script.frames.lock().unwrap().clone();
    let ids: HashSet<_> = frames.iter().map(|frame| frame.asset.as_str()).collect();
    assert!(ids.contains(format!("{taken}-2").as_str()), "{ids:?}");
    assert_eq!(
        frames.len(),
        3,
        "cloud's stand-in, the squatter's stand-in, the scene"
    );
    assert!(
        state
            .library
            .items
            .iter()
            .any(|item| item.id == taken.as_str())
    );
    assert_eq!(
        state.thumbnail_report("cloud")["state"],
        "ready",
        "the squatter is a splat of the project, untouched"
    );
}

#[test]
fn a_damaged_cached_picture_is_drawn_again_not_trusted() {
    let (dir, set) = setup("integrity");
    let mut state = project(&dir, "film", &set);
    let splat = id_of(&state, "splat");
    let script = Arc::new(Script::default());
    let mut host = host(&script, false);
    ok(&mut host, &mut state, json!({"op":"media.thumbnails"}));
    let picture = PathBuf::from(state.thumbnail(&splat).unwrap().path.clone());
    let good = fs::read(&picture).unwrap();
    let (width, height) = size_for(state.timeline.width, state.timeline.height);
    assert_eq!(png_dimensions(&good), Ok((width, height)));
    let mut frames = script.frames.lock().unwrap().len();

    // The control: an untouched picture is served from the cache without a frame.
    state.thumbnails = Default::default();
    ok(&mut host, &mut state, json!({"op":"media.thumbnails"}));
    assert_eq!(
        script.frames.lock().unwrap().len(),
        frames,
        "intact: not drawn again"
    );

    type Damage<'a> = Box<dyn Fn() + 'a>;
    let damage: Vec<(&str, Damage<'_>)> = vec![
        (
            "truncated",
            Box::new(|| fs::write(&picture, &good[..good.len() / 2]).unwrap()),
        ),
        ("missing", Box::new(|| fs::remove_file(&picture).unwrap())),
        (
            "an altered IHDR",
            Box::new(|| {
                let mut bytes = good.clone();
                bytes[19] ^= 0x01;
                fs::write(&picture, bytes).unwrap()
            }),
        ),
        (
            "an altered pixel chunk",
            Box::new(|| {
                let mut bytes = good.clone();
                let middle = bytes.len() / 2;
                bytes[middle] ^= 0xff;
                fs::write(&picture, bytes).unwrap()
            }),
        ),
        (
            "bytes after IEND",
            Box::new(|| {
                let mut bytes = good.clone();
                bytes.extend_from_slice(b"junk");
                fs::write(&picture, bytes).unwrap()
            }),
        ),
        (
            "a valid PNG of another size",
            Box::new(|| {
                fs::write(&picture, encode_png(8, 8, &[7u8; 8 * 8 * 4])).unwrap();
            }),
        ),
        (
            "an empty file",
            Box::new(|| fs::write(&picture, b"").unwrap()),
        ),
    ];
    for (what, hurt) in damage {
        hurt();
        state.thumbnails = Default::default();
        let report = ok(&mut host, &mut state, json!({"op":"media.thumbnails"}));
        assert!(
            report["applied"]["applied"]
                .as_array()
                .unwrap()
                .contains(&json!(splat)),
            "{what}: {report}"
        );
        frames += 1;
        assert_eq!(
            script.frames.lock().unwrap().len(),
            frames,
            "{what}: exactly the damaged picture was drawn again"
        );
        assert_eq!(
            png_dimensions(&fs::read(&picture).unwrap()),
            Ok((width, height)),
            "{what}: a whole picture is back"
        );
    }
}

#[test]
fn the_png_check_reads_every_chunk_not_only_the_first_twenty_four_bytes() {
    let good = encode_png(6, 4, &[9u8; 6 * 4 * 4]);
    assert_eq!(png_dimensions(&good), Ok((6, 4)));
    assert!(
        png_dimensions(&good[..good.len() - 1])
            .unwrap_err()
            .contains("truncated")
    );
    assert!(png_dimensions(&good[..30]).is_err());
    assert!(png_dimensions(b"not a png at all, but long enough to have a header").is_err());
    let mut flipped = good.clone();
    flipped[good.len() - 20] ^= 0x10;
    assert!(
        png_dimensions(&flipped).is_err(),
        "a bit flipped in the data fails a chunk CRC"
    );
    let mut header = good.clone();
    header[16..20].copy_from_slice(&7u32.to_be_bytes());
    assert!(
        png_dimensions(&header).unwrap_err().contains("IHDR"),
        "the IHDR CRC no longer matches"
    );
}

#[test]
fn another_opening_of_the_same_document_is_another_session_for_a_running_job() {
    let (dir, set) = setup("reopened");
    let mut state = project(&dir, "film", &set);
    let document = dir.join("film").join("film.tessel.json");
    ok(
        &mut MediaHost::new(),
        &mut state,
        json!({"op":"project.save","path":document}),
    );
    let (id, splat) = (state.project.id.clone(), id_of(&state, "splat"));
    let first = state.thumbnail_stamp();
    let script = Arc::new(Script::default());
    script.hold.store(true, Ordering::SeqCst);
    let mut host = host(&script, true);

    // The first opening's job reaches the renderer and waits there.
    let deadline = Instant::now() + Duration::from_secs(60);
    while script.frames.lock().unwrap().is_empty() {
        host.poll(&mut state);
        assert!(
            Instant::now() < deadline,
            "the job never reached the renderer"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let running = host
        .registry()
        .list()
        .into_iter()
        .find(|job| job.kind.label() == "thumbnail")
        .unwrap();
    assert!(!running.cancel_requested);

    // The same document is opened again, in the same window state: same project ID, new session.
    ok(
        &mut host,
        &mut state,
        json!({"op":"project.open","path":document}),
    );
    assert_eq!(state.project.id, id, "the very same document");
    assert!(!first.is_current(&state), "a new opening is a new session");
    host.poll(&mut state);
    assert!(
        host.registry().get(&running.id).unwrap().cancel_requested,
        "the job of the previous opening is told to stop though the project ID is the same"
    );

    // A -> B -> A: another project in between, then the document once more.
    let second_session = state.thumbnail_stamp();
    state = project(&dir, "other", &set);
    host.poll(&mut state);
    ok(
        &mut host,
        &mut state,
        json!({"op":"project.open","path":document}),
    );
    assert_eq!(state.project.id, id);
    assert!(!second_session.is_current(&state));

    // The renderer answers; the old job ends cancelled and its result is applied to nobody. The
    // project that is open gets only what its own job drew.
    script.hold.store(false, Ordering::SeqCst);
    script.release.store(true, Ordering::SeqCst);
    let registry = host.registry().clone();
    poll_until(&mut host, &mut state, |_| {
        registry
            .get(&running.id)
            .is_some_and(|job| job.state.is_final())
    });
    assert_eq!(
        registry.get(&running.id).unwrap().state.label(),
        "cancelled"
    );
    poll_until(&mut host, &mut state, |state| {
        state.thumbnail(&splat).is_some()
    });
    let jobs: Vec<_> = registry
        .list()
        .into_iter()
        .filter(|job| job.kind.label() == "thumbnail")
        .collect();
    assert!(
        jobs.len() >= 2,
        "the old job and the one of the project that is open"
    );
    assert_eq!(
        jobs.iter()
            .filter(|job| job.state.label() == "succeeded")
            .count(),
        1,
        "only the project that is open had a job that completed"
    );
    assert!(
        state.thumbnail(&splat).unwrap().path.contains("/film/"),
        "{:?}",
        state.thumbnail(&splat)
    );
}

#[test]
fn a_result_finished_for_one_opening_is_refused_by_the_next_opening_of_the_same_document() {
    let (dir, set) = setup("reopened-commit");
    let mut state = project(&dir, "film", &set);
    let document = dir.join("film").join("film.tessel.json");
    ok(
        &mut MediaHost::new(),
        &mut state,
        json!({"op":"project.save","path":document}),
    );
    let script = Arc::new(Script::default());
    let mut host = host(&script, false);
    let queued = ok(
        &mut host,
        &mut state,
        json!({"op":"media.thumbnails","wait":false}),
    );
    let job = queued["job"].as_str().unwrap().to_string();
    ok(&mut host, &mut state, json!({"op":"job.wait","job":job}));

    ok(
        &mut host,
        &mut state,
        json!({"op":"project.open","path":document}),
    );
    let committed = ok(&mut host, &mut state, json!({"op":"job.commit","job":job}));
    assert_eq!(committed["applied"]["stale"], true, "{committed}");
    assert!(
        state.thumbnails.is_empty(),
        "nothing of the first opening reached the second"
    );
}

#[test]
fn a_new_project_waits_for_the_old_projects_job_instead_of_reusing_it() {
    let (dir, set) = setup("waiting");
    let mut first = project(&dir, "first", &set);
    let script = Arc::new(Script::default());
    script.hold.store(true, Ordering::SeqCst);
    let mut host = host(&script, true);
    let deadline = Instant::now() + Duration::from_secs(60);
    while script.frames.lock().unwrap().is_empty() {
        host.poll(&mut first);
        assert!(
            Instant::now() < deadline,
            "the job never reached the renderer"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let old = host
        .registry()
        .list()
        .into_iter()
        .find(|job| job.kind.label() == "thumbnail")
        .unwrap();

    // Another project is open and asks for its thumbnails while the old job still holds the renderer.
    let mut second = project(&dir, "second", &set);
    let reply = ok(&mut host, &mut second, json!({"op":"media.thumbnails"}));
    assert_eq!(reply["state"], "waiting", "{reply}");
    assert_eq!(
        reply["reused"], false,
        "never the other project's job: {reply}"
    );
    assert_eq!(reply["waiting_for"], json!(old.id));
    assert!(reply["deadline_seconds"].as_u64().unwrap() >= 1);
    assert!(host.registry().get(&old.id).unwrap().cancel_requested);
    assert!(
        reply.get("job").is_none(),
        "the reply offers no job that is not this project's: {reply}"
    );
    assert_eq!(
        host.registry()
            .list()
            .iter()
            .filter(|job| job.kind.label() == "thumbnail")
            .count(),
        1,
        "no second renderer is started"
    );

    // The old job ends at its frame boundary; the project that is open then gets its own job.
    script.hold.store(false, Ordering::SeqCst);
    script.release.store(true, Ordering::SeqCst);
    poll_until(&mut host, &mut second, |state| {
        state.thumbnail_targets(false).is_empty()
    });
    assert_eq!(
        host.registry().get(&old.id).unwrap().state.label(),
        "cancelled"
    );
    assert!(first.thumbnails.is_empty());
    assert!(second.thumbnail(&id_of(&second, "splat")).is_some());
}

#[test]
fn a_job_the_user_cancelled_is_not_started_again_by_the_next_tick() {
    let (dir, set) = setup("paused");
    let mut state = project(&dir, "film", &set);
    let script = Arc::new(Script::default());
    script.hold.store(true, Ordering::SeqCst);
    let mut host = host(&script, true);
    let deadline = Instant::now() + Duration::from_secs(60);
    while script.frames.lock().unwrap().is_empty() {
        host.poll(&mut state);
        assert!(
            Instant::now() < deadline,
            "the job never reached the renderer"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let job = host
        .registry()
        .list()
        .into_iter()
        .find(|job| job.kind.label() == "thumbnail")
        .unwrap()
        .id;
    ok(&mut host, &mut state, json!({"op":"job.cancel","job":job}));
    let busy = headless::execute_ext(
        &mut state,
        None,
        Some(&mut host),
        &json!({"op":"media.thumbnails"}),
    )
    .unwrap_err();
    assert_eq!(busy.code, "thumbnail_busy");
    script.hold.store(false, Ordering::SeqCst);
    script.release.store(true, Ordering::SeqCst);
    let registry = host.registry().clone();
    poll_until(&mut host, &mut state, |_| {
        registry.get(&job).is_some_and(|info| info.state.is_final())
    });
    for _ in 0..30 {
        host.poll(&mut state);
        std::thread::sleep(Duration::from_millis(10));
    }
    let thumbnails = |host: &MediaHost| {
        host.registry()
            .list()
            .into_iter()
            .filter(|job| job.kind.label() == "thumbnail")
            .count()
    };
    assert_eq!(
        thumbnails(&host),
        1,
        "the tick did not undo the user's cancel"
    );
    assert!(state.thumbnails.is_empty());

    // An invalid request must not resume the user's cancelled job on the next tick.
    let invalid = headless::execute_ext(
        &mut state,
        None,
        Some(&mut host),
        &json!({"op":"media.thumbnails","assets":["unknown"]}),
    )
    .unwrap_err();
    assert_eq!(invalid.code, "unknown_element");
    host.poll(&mut state);
    assert_eq!(thumbnails(&host), 1);

    // Asking for them is a decision, and it goes through.
    let reply = ok(&mut host, &mut state, json!({"op":"media.thumbnails"}));
    assert!(reply["job"].is_string(), "{reply}");
    poll_until(&mut host, &mut state, |state| {
        state.thumbnail_targets(false).is_empty()
    });
    assert_eq!(thumbnails(&host), 2);
}

#[test]
fn the_tick_does_not_draw_thumbnails_while_a_video_export_owns_the_renderer() {
    let (dir, set) = setup("export");
    let mut state = project(&dir, "film", &set);
    let script = Arc::new(Script::default());
    let mut host = host(&script, true);
    // A video export in flight, as the registry sees it (the body is not what matters here).
    let release = Arc::new(AtomicBool::new(false));
    let waiting = release.clone();
    let export = host.registry().spawn(
        tessel_editor_core::jobs::JobKind::VideoExport,
        "export",
        move |ctx| {
            while !waiting.load(Ordering::SeqCst) {
                ctx.check()?;
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(json!({}))
        },
    );
    let explicit = headless::execute_ext(
        &mut state,
        None,
        Some(&mut host),
        &json!({"op":"media.thumbnails"}),
    )
    .unwrap_err();
    assert_eq!(explicit.code, "renderer_busy");
    for _ in 0..30 {
        host.poll(&mut state);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        script.built.load(Ordering::SeqCst),
        0,
        "no renderer was started beside the export"
    );
    assert!(script.frames.lock().unwrap().is_empty());

    release.store(true, Ordering::SeqCst);
    let registry = host.registry().clone();
    poll_until(&mut host, &mut state, |_| {
        registry
            .get(&export)
            .is_some_and(|job| job.state.is_final())
    });
    poll_until(&mut host, &mut state, |state| {
        state.thumbnail_targets(false).is_empty()
    });
    assert_eq!(
        script.frames.lock().unwrap().len(),
        2,
        "after the export, the thumbnails"
    );
}
