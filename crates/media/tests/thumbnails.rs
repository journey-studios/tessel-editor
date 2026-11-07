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
use tessel_editor_core::fixtures::{StandardSet, encode_png, write_standard_set};
use tessel_editor_core::plan::{PreviewIntent, RenderPlan};
use tessel_editor_core::session::Renderer;
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::thumbnails::{WIDTH, size_for};
use tessel_editor_core::{CommandError, EditorState, headless};
use tessel_editor_media::host::{MediaHost, RendererFactory};

use common::{temp, tools};

/// One frame the double was asked for.
type Frame = (String, Vec<String>, u32, u32);

/// What the double saw and how it should misbehave, shared by every renderer the factory builds.
#[derive(Default)]
struct Script {
    /// `(asset, splats its scene file draws, width, height)` of every frame asked, in order.
    frames: Mutex<Vec<Frame>>,
    /// Renderers built (each is a Chrome in real life).
    built: AtomicUsize,
    /// Assets whose frame fails.
    broken: Mutex<HashSet<String>>,
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
        _probes: Vec<(u32, u32)>,
        _timeout: Duration,
    ) -> Result<FrameResult, BackendError> {
        let PreviewIntent::Source { asset, .. } = plan.intent else {
            panic!("a thumbnail is a source plan, got {:?}", plan.intent);
        };
        let asset = asset.to_string();
        // What the scene file of that asset puts on screen, read the way the runtime would.
        let item = state
            .library
            .item(&asset)
            .expect("the plan names a library asset");
        let splats = match &item.asset.as_ref().unwrap().source {
            tessel_editor_core::assets::Source::File { path } if item.kind.label() == "scene" => {
                let scene: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
                scene["objects"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|object| object["type"] == "splat")
                    .map(|object| object["asset"].as_str().unwrap().to_string())
                    .collect()
            }
            _ => Vec::new(),
        };
        self.0
            .frames
            .lock()
            .unwrap()
            .push((asset.clone(), splats, output.width, output.height));
        while self.0.hold.load(Ordering::SeqCst) && !self.0.release.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        if self.0.broken.lock().unwrap().contains(&asset) {
            return Err(BackendError::new(
                "render_failed",
                "the double was told to fail",
            ));
        }
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
            mean: [0.0; 4],
            probes: Vec::new(),
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

/// A project with a workspace, the splat and the scene of the standard set imported (hashed).
fn project(dir: &Path, name: &str, set: &StandardSet) -> EditorState {
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
        json!({"op":"media.import","paths":[set.splat, set.scene]}),
    );
    assert_eq!(report["applied"]["imported"], 2, "{report}");
    state
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
    assert_eq!(
        frames,
        vec![
            (format!("thumb-{splat}"), vec![splat.clone()], width, height),
            (scene.clone(), vec![splat.clone()], width, height)
        ]
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

    // A different file under the same asset ID is a different cache entry, drawn afresh.
    let asset = state.library.items[0].asset.as_mut().unwrap();
    asset.identity.hash = Some("0".repeat(64));
    let report = ok(&mut host, &mut state, json!({"op":"media.thumbnails"}));
    assert_eq!(report["assets"], json!([splat]), "{report}");
    assert_eq!(
        script.frames.lock().unwrap().len(),
        3,
        "only the changed asset was drawn"
    );
    assert_ne!(state.thumbnail(&splat).unwrap().path, path);
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
