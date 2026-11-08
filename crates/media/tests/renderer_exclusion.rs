//! BL144.10: one renderer at a time between video exports (normal and directed), their preflight
//! and the library thumbnails (the window's tick and the explicit `media.thumbnails`).
//!
//! The renderers are doubles that **count themselves**: every one built registers in an arena
//! (`live`, `peak`, frames in flight), so "never two renderers" is a measured number and not an
//! absence of errors. A hold flag parks a frame at a known point, which turns every ordering
//! below into a fact instead of a race. What the real runtime draws is `crates/render/tests`.
//!
//! What is proved: the two orders of start (thumbnail first, export first), explicit and
//! automatic requests, the preflight, the directed export, project switches, a renderer that
//! fails to start or to draw, cancelling and the user's pause before/during/after an export, and
//! that a refusal is not a queue. `the_counter_sees_two_renderers_when_nothing_coordinates_them`
//! proves the counter would notice an overlap (the check is not vacuous).

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tessel_editor_core::backend::{BackendError, Capabilities, FrameResult, Output};
use tessel_editor_core::fixtures::{StandardSet, encode_png, write_standard_set};
use tessel_editor_core::jobs::JobError;
use tessel_editor_core::plan::{PreviewIntent, RenderPlan};
use tessel_editor_core::session::Renderer;
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::{CommandError, EditorState, headless};
use tessel_editor_media::host::{MediaHost, RendererFactory};
use tessel_editor_media::scene_encoder::{
    FrameJob, RenderedFrame, SceneFrameRenderer, SceneRendererFactory,
};

use common::mock::{Mock, Settings};
use common::{temp, tools};

/// What every renderer the factories build shares: how many are alive, how many frames are being
/// drawn, and the switches that park or break them.
#[derive(Default)]
struct Arena {
    live: AtomicUsize,
    peak: AtomicUsize,
    built: AtomicUsize,
    in_frame: AtomicUsize,
    peak_in_frame: AtomicUsize,
    thumbnail_frames: AtomicUsize,
    export_frames: AtomicUsize,
    /// A thumbnail frame waits while set.
    hold_thumbnail: AtomicBool,
    /// A frame of a normal export waits while set.
    hold_export: AtomicBool,
    /// A frame of a directed export waits while set.
    hold_scene: AtomicBool,
    /// The renderer's capability query (the preflight probe) waits while set.
    hold_probe: AtomicBool,
    /// Renderers that have started a capability query and not ended it.
    probing: AtomicUsize,
    /// The next normal export renderer fails on its first frame.
    fail_export_frames: AtomicBool,
    /// The factory cannot start a renderer.
    fail_build: AtomicBool,
}

impl Arena {
    fn live(&self) -> usize {
        self.live.load(Ordering::SeqCst)
    }

    fn peak(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }

    fn in_frame(&self) -> usize {
        self.in_frame.load(Ordering::SeqCst)
    }
}

/// A renderer that is alive: counted on build, uncounted when dropped.
struct Live(Arc<Arena>);

impl Live {
    fn new(arena: &Arc<Arena>) -> Self {
        let now = arena.live.fetch_add(1, Ordering::SeqCst) + 1;
        arena.peak.fetch_max(now, Ordering::SeqCst);
        arena.built.fetch_add(1, Ordering::SeqCst);
        Self(arena.clone())
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        self.0.live.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A frame being drawn.
struct Drawing(Arc<Arena>);

impl Drawing {
    fn new(arena: &Arc<Arena>) -> Self {
        let now = arena.in_frame.fetch_add(1, Ordering::SeqCst) + 1;
        arena.peak_in_frame.fetch_max(now, Ordering::SeqCst);
        Self(arena.clone())
    }
}

impl Drop for Drawing {
    fn drop(&mut self) {
        self.0.in_frame.fetch_sub(1, Ordering::SeqCst);
    }
}

fn park_while(flag: &AtomicBool) {
    while flag.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The renderer of thumbnails, normal exports and the preflight probe: a source plan is a
/// thumbnail (a flat picture), anything else is an export frame (the shared mock).
struct GateRenderer {
    arena: Arc<Arena>,
    mock: Mock,
    _live: Live,
}

impl GateRenderer {
    fn new(arena: &Arc<Arena>) -> Self {
        Self {
            mock: Mock::new(Settings {
                fail_at: arena.fail_export_frames.load(Ordering::SeqCst).then_some(0),
                ..Settings::default()
            }),
            _live: Live::new(arena),
            arena: arena.clone(),
        }
    }
}

impl Renderer for GateRenderer {
    fn render_frame(
        &mut self,
        state: &EditorState,
        plan: RenderPlan,
        output: Output,
        probes: Vec<(u32, u32)>,
        timeout: Duration,
    ) -> Result<FrameResult, BackendError> {
        let _drawing = Drawing::new(&self.arena);
        if !matches!(plan.intent, PreviewIntent::Source { .. }) {
            self.arena.export_frames.fetch_add(1, Ordering::SeqCst);
            park_while(&self.arena.hold_export);
            return self.mock.render_frame(state, plan, output, probes, timeout);
        }
        self.arena.thumbnail_frames.fetch_add(1, Ordering::SeqCst);
        park_while(&self.arena.hold_thumbnail);
        let (width, height) = (output.width as usize, output.height as usize);
        let rgba: Vec<u8> = (0..width * height)
            .flat_map(|_| [40u8, 90, 160, 255])
            .collect();
        let path = output.png.expect("a thumbnail asks for a PNG");
        fs::write(&path, encode_png(output.width, output.height, &rgba))
            .map_err(|error| BackendError::new("io_error", error.to_string()))?;
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
        self.arena.probing.fetch_add(1, Ordering::SeqCst);
        park_while(&self.arena.hold_probe);
        let capabilities = self.mock.capabilities();
        self.arena.probing.fetch_sub(1, Ordering::SeqCst);
        capabilities
    }

    fn summary(&self) -> Value {
        self.mock.summary()
    }

    fn reset_project(&mut self) {
        self.mock.reset_project();
    }
}

/// The renderer of a directed-scene export: copies one real picture per frame.
struct GateScene {
    arena: Arc<Arena>,
    png: PathBuf,
    _live: Live,
}

impl SceneFrameRenderer for GateScene {
    fn max_size(&mut self) -> Result<Option<u32>, JobError> {
        Ok(None)
    }

    fn render(&mut self, job: &FrameJob) -> Result<RenderedFrame, JobError> {
        let _drawing = Drawing::new(&self.arena);
        park_while(&self.arena.hold_scene);
        fs::copy(&self.png, job.png)
            .map_err(|error| JobError::new("render_failed", error.to_string()))?;
        Ok(RenderedFrame {
            hash: "still".into(),
            render_ms: 1.0,
        })
    }

    fn finish(&mut self) {}
}

/// A window host (interactive: its `poll` is the tick) whose renderers are the doubles. `hold` is
/// how long a refused export reserves the renderer (`None`: the default).
fn window(arena: &Arc<Arena>, hold: Option<Duration>) -> MediaHost {
    let shared = arena.clone();
    let factory: Arc<RendererFactory> = Arc::new(move || {
        if shared.fail_build.load(Ordering::SeqCst) {
            return Err(JobError::new(
                "backend_unavailable",
                "the renderer could not start",
            ));
        }
        Ok(Box::new(GateRenderer::new(&shared)))
    });
    let host = MediaHost::new()
        .with_renderer(factory)
        .with_tools(tools())
        .interactive();
    match hold {
        Some(hold) => host.with_export_priority(hold),
        None => host,
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
    let paths = [&set.splat, &set.scene];
    let report = ok(
        &mut importer,
        &mut state,
        json!({"op":"media.import","paths":paths}),
    );
    assert_eq!(report["applied"]["imported"], 2, "{report}");
    state
}

fn setup(name: &str) -> (PathBuf, StandardSet) {
    let dir = temp(name);
    let set = write_standard_set(&dir.join("set")).unwrap();
    (dir, set)
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

fn tiny() -> Value {
    json!({"id":"tiny","name":"Tiny","width":320,"height":180,"fps":24})
}

fn export_request(dir: &Path, name: &str) -> Value {
    json!({"op":"export.start","preset":tiny(),"destination":dir.join(name),"range":[0,4]})
}

fn scene_export_request(dir: &Path, name: &str) -> Value {
    json!({"op":"scene.export","scene":"street","preset":tiny(),"from":0,"frames":2,
        "destination":dir.join(name)})
}

/// What the window's tick does, until `done`.
fn poll_until(
    host: &mut MediaHost,
    state: &mut EditorState,
    what: &str,
    done: impl FnMut(&MediaHost, &EditorState) -> bool,
) {
    poll_within(host, state, what, Duration::from_secs(60), done);
}

/// Like [`poll_until`], with a limit that is part of what is being proved.
fn poll_within(
    host: &mut MediaHost,
    state: &mut EditorState,
    what: &str,
    limit: Duration,
    mut done: impl FnMut(&MediaHost, &EditorState) -> bool,
) {
    let deadline = Instant::now() + limit;
    while !done(host, state) {
        host.poll(state);
        assert!(Instant::now() < deadline, "never happened: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn tick(host: &mut MediaHost, state: &mut EditorState, times: usize) {
    for _ in 0..times {
        host.poll(state);
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn count(host: &MediaHost, kind: &str) -> usize {
    host.registry()
        .list()
        .iter()
        .filter(|job| job.kind.label() == kind)
        .count()
}

fn only_job(host: &MediaHost, kind: &str) -> String {
    let jobs: Vec<_> = host
        .registry()
        .list()
        .into_iter()
        .filter(|job| job.kind.label() == kind)
        .collect();
    assert_eq!(jobs.len(), 1, "expected exactly one {kind} job");
    jobs[0].id.clone()
}

fn label(host: &MediaHost, job: &str) -> Option<&'static str> {
    host.registry().get(job).map(|info| info.state.label())
}

fn job_of(reply: &Value) -> String {
    reply["job"]
        .as_str()
        .unwrap_or_else(|| panic!("no job in {reply}"))
        .to_string()
}

fn details(error: &CommandError) -> &Value {
    error.details.as_deref().expect("a refusal carries details")
}

/// A thumbnail frame parked in the renderer: the tick started the job and it reached the frame.
fn thumbnail_frame_in_flight(
    arena: &Arc<Arena>,
    dir: &Path,
    set: &StandardSet,
    hold: Option<Duration>,
) -> (EditorState, MediaHost) {
    let mut state = project(dir, "film", set);
    arena.hold_thumbnail.store(true, Ordering::SeqCst);
    let mut host = window(arena, hold);
    poll_until(
        &mut host,
        &mut state,
        "a thumbnail frame in flight",
        |_, _| arena.in_frame() == 1,
    );
    (state, host)
}

/// An export frame parked in the renderer.
fn export_frame_in_flight(
    arena: &Arc<Arena>,
    dir: &Path,
    set: &StandardSet,
    hold: Option<Duration>,
) -> (EditorState, MediaHost, String) {
    let mut state = project(dir, "film", set);
    arena.hold_export.store(true, Ordering::SeqCst);
    let mut host = window(arena, hold);
    let started = ok(&mut host, &mut state, export_request(dir, "held.mp4"));
    let job = job_of(&started);
    poll_until(
        &mut host,
        &mut state,
        "an export frame in flight",
        |_, _| arena.in_frame() == 1,
    );
    (state, host, job)
}

#[test]
fn the_counter_sees_two_renderers_when_nothing_coordinates_them() {
    // Two factories calls with no policy between them: the measure reports the overlap.
    let arena = Arc::new(Arena::default());
    let first = GateRenderer::new(&arena);
    let second = GateRenderer::new(&arena);
    assert_eq!(arena.live(), 2);
    assert_eq!(arena.peak(), 2, "an overlap is visible in the peak");
    drop((first, second));
    assert_eq!(arena.live(), 0);
    assert_eq!(arena.peak(), 2);
}

#[test]
fn an_export_asked_behind_a_thumbnail_frame_is_refused_the_frame_ends_and_the_retry_wins() {
    let (dir, set) = setup("export-behind-thumbnail");
    let arena = Arc::new(Arena::default());
    let (mut state, mut host) = thumbnail_frame_in_flight(&arena, &dir, &set, None);
    let thumbnail = only_job(&host, "thumbnail");
    assert_eq!(arena.live(), 1);

    // Priority for the export: it is told to wait, the thumbnail job is asked to stop, and no
    // second renderer starts. The request is refused, not remembered.
    let refused = run(&mut host, &mut state, export_request(&dir, "a.mp4")).unwrap_err();
    assert_eq!(refused.code, "renderer_busy");
    assert_eq!(details(&refused)["retryable"], true);
    assert_eq!(details(&refused)["queued"], false);
    assert_eq!(details(&refused)["job"], json!(thumbnail));
    assert_eq!(details(&refused)["stopping"], true);
    assert!(host.registry().get(&thumbnail).unwrap().cancel_requested);
    assert_eq!(count(&host, "video-export"), 0, "refused, not accepted");
    assert_eq!(arena.live(), 1, "no second renderer");
    assert_eq!(arena.in_frame(), 1, "the frame in flight is not cut short");

    // The renderer stays reserved for the export: the tick draws nothing new and an explicit
    // thumbnail request is refused too.
    tick(&mut host, &mut state, 20);
    assert_eq!(count(&host, "thumbnail"), 1);
    let explicit = run(&mut host, &mut state, json!({"op":"media.thumbnails"})).unwrap_err();
    assert_eq!(explicit.code, "renderer_busy");
    assert_eq!(details(&explicit)["reserved_for_export"], true);

    // The frame finishes first; the renderer is gone before the export may start.
    arena.hold_thumbnail.store(false, Ordering::SeqCst);
    poll_until(
        &mut host,
        &mut state,
        "the thumbnail job to end",
        |host, _| label(host, &thumbnail) == Some("cancelled"),
    );
    assert_eq!(arena.live(), 0);
    tick(&mut host, &mut state, 20);
    assert_eq!(count(&host, "thumbnail"), 1, "still reserved for the retry");

    let started = ok(&mut host, &mut state, export_request(&dir, "a.mp4"));
    let export = job_of(&started);
    poll_until(&mut host, &mut state, "the export", |host, _| {
        label(host, &export) == Some("succeeded")
    });
    assert!(arena.export_frames.load(Ordering::SeqCst) >= 4);
    assert_eq!(arena.peak(), 1, "never two renderers");
    assert_eq!(arena.peak_in_frame.load(Ordering::SeqCst), 1);

    // A stop for an export is not the user's pause: the drawing resumes after the export.
    let (splat, scene) = (id_of(&state, "splat"), id_of(&state, "scene"));
    poll_until(
        &mut host,
        &mut state,
        "the thumbnails after the export",
        |_, state| state.thumbnail(&splat).is_some() && state.thumbnail(&scene).is_some(),
    );
    assert_eq!(arena.peak(), 1);
}

#[test]
fn a_thumbnail_asked_while_an_export_runs_is_refused_by_the_tick_and_by_the_caller() {
    let (dir, set) = setup("thumbnail-behind-export");
    let arena = Arc::new(Arena::default());
    let (mut state, mut host, export) = export_frame_in_flight(&arena, &dir, &set, None);
    assert_eq!(arena.live(), 1);

    tick(&mut host, &mut state, 30);
    assert_eq!(count(&host, "thumbnail"), 0, "the tick starts nothing");
    assert_eq!(arena.thumbnail_frames.load(Ordering::SeqCst), 0);

    let busy = run(&mut host, &mut state, json!({"op":"media.thumbnails"})).unwrap_err();
    assert_eq!(busy.code, "renderer_busy");
    assert_eq!(details(&busy)["retryable"], true);
    assert_eq!(details(&busy)["job"], json!(export));
    assert!(
        details(&busy).get("stopping").is_none(),
        "an export is never preempted"
    );
    assert!(!host.registry().get(&export).unwrap().cancel_requested);

    // A second export is refused too: one renderer, and nothing is queued behind the first.
    let second = run(&mut host, &mut state, export_request(&dir, "second.mp4")).unwrap_err();
    assert_eq!(second.code, "renderer_busy");
    assert_eq!(count(&host, "video-export"), 1);
    assert_eq!(arena.live(), 1);

    arena.hold_export.store(false, Ordering::SeqCst);
    poll_until(&mut host, &mut state, "the export", |host, _| {
        label(host, &export) == Some("succeeded")
    });
    let (splat, scene) = (id_of(&state, "splat"), id_of(&state, "scene"));
    poll_until(
        &mut host,
        &mut state,
        "the thumbnails after the export",
        |_, state| state.thumbnail(&splat).is_some() && state.thumbnail(&scene).is_some(),
    );
    assert_eq!(arena.peak(), 1, "never two renderers");
}

#[test]
fn the_preflight_that_probes_the_renderer_asks_for_it_like_an_export() {
    let (dir, set) = setup("preflight");
    let probe = json!({"op":"export.preflight","preset":tiny(),"destination":dir.join("p.mp4"),
        "range":[0,4]});

    // Behind a thumbnail frame: refused, the job is asked to stop, the renderer is reserved.
    let arena = Arc::new(Arena::default());
    let (mut state, mut host) = thumbnail_frame_in_flight(&arena, &dir, &set, None);
    let thumbnail = only_job(&host, "thumbnail");
    let refused = run(&mut host, &mut state, probe.clone()).unwrap_err();
    assert_eq!(refused.code, "renderer_busy");
    assert_eq!(details(&refused)["stopping"], true);
    assert_eq!(count(&host, "preflight"), 0, "refused, not accepted");
    arena.hold_thumbnail.store(false, Ordering::SeqCst);
    poll_until(
        &mut host,
        &mut state,
        "the thumbnail job to end",
        |host, _| label(host, &thumbnail) == Some("cancelled"),
    );
    let started = ok(&mut host, &mut state, probe.clone());
    let preflight = job_of(&started);
    poll_until(&mut host, &mut state, "the preflight", |host, _| {
        label(host, &preflight) == Some("succeeded")
    });
    assert_eq!(arena.peak(), 1);

    // Behind an export frame: refused, and nothing is stopped.
    let (dir, set) = setup("preflight-behind-export");
    let arena = Arc::new(Arena::default());
    let (mut state, mut host, export) = export_frame_in_flight(&arena, &dir, &set, None);
    let busy = run(&mut host, &mut state, probe).unwrap_err();
    assert_eq!(busy.code, "renderer_busy");
    assert_eq!(details(&busy)["job"], json!(export));
    assert_eq!(count(&host, "preflight"), 0);
    arena.hold_export.store(false, Ordering::SeqCst);
    poll_until(&mut host, &mut state, "the export", |host, _| {
        label(host, &export) == Some("succeeded")
    });
    assert_eq!(arena.peak(), 1);
}

#[test]
fn a_refusal_is_not_a_queue_nothing_runs_until_the_caller_asks_again() {
    // No reservation: the tick may take the renderer back at once, and the export that was
    // refused never appears by itself.
    let (dir, set) = setup("no-queue");
    let arena = Arc::new(Arena::default());
    let (mut state, mut host) = thumbnail_frame_in_flight(&arena, &dir, &set, Some(Duration::ZERO));
    let thumbnail = only_job(&host, "thumbnail");
    let refused = run(&mut host, &mut state, export_request(&dir, "lost.mp4")).unwrap_err();
    assert_eq!(refused.code, "renderer_busy");
    arena.hold_thumbnail.store(false, Ordering::SeqCst);
    poll_until(
        &mut host,
        &mut state,
        "the stopped job to end",
        |host, _| label(host, &thumbnail) == Some("cancelled"),
    );
    let (splat, scene) = (id_of(&state, "splat"), id_of(&state, "scene"));
    poll_until(
        &mut host,
        &mut state,
        "the thumbnails by the tick",
        |_, state| state.thumbnail(&splat).is_some() && state.thumbnail(&scene).is_some(),
    );
    tick(&mut host, &mut state, 30);
    assert_eq!(
        count(&host, "video-export"),
        0,
        "the refused export is gone"
    );
    assert!(!dir.join("lost.mp4").exists());
    assert_eq!(
        count(&host, "thumbnail"),
        2,
        "a stop for an export is not a pause: the tick drew them again"
    );
    assert_eq!(arena.peak(), 1);
}

#[test]
fn the_users_pause_survives_an_export_that_finishes_is_cancelled_or_fails() {
    for outcome in ["finishes", "is cancelled", "fails"] {
        let (dir, set) = setup(&format!("pause-{}", outcome.replace(' ', "-")));
        let arena = Arc::new(Arena::default());
        let (mut state, mut host) = thumbnail_frame_in_flight(&arena, &dir, &set, None);
        let thumbnail = only_job(&host, "thumbnail");

        // The user cancels the drawing: that project's automatic drawing is paused.
        ok(
            &mut host,
            &mut state,
            json!({"op":"job.cancel","job":thumbnail}),
        );
        arena.hold_thumbnail.store(false, Ordering::SeqCst);
        poll_until(
            &mut host,
            &mut state,
            "the cancelled job to end",
            |host, _| label(host, &thumbnail) == Some("cancelled"),
        );
        tick(&mut host, &mut state, 20);
        assert_eq!(
            count(&host, "thumbnail"),
            1,
            "{outcome}: paused before the export"
        );

        // The export is granted (a paused drawing owns no renderer) and ends its own way.
        match outcome {
            "fails" => arena.fail_export_frames.store(true, Ordering::SeqCst),
            "is cancelled" => arena.hold_export.store(true, Ordering::SeqCst),
            _ => {}
        }
        let started = ok(&mut host, &mut state, export_request(&dir, "p.mp4"));
        let export = job_of(&started);
        if outcome == "is cancelled" {
            poll_until(
                &mut host,
                &mut state,
                "an export frame in flight",
                |_, _| arena.in_frame() == 1,
            );
            ok(
                &mut host,
                &mut state,
                json!({"op":"job.cancel","job":export}),
            );
            arena.hold_export.store(false, Ordering::SeqCst);
        }
        let expected = match outcome {
            "finishes" => "succeeded",
            "is cancelled" => "cancelled",
            _ => "failed",
        };
        poll_until(&mut host, &mut state, "the export to end", |host, _| {
            label(host, &export) == Some(expected)
        });
        assert_eq!(
            arena.live(),
            0,
            "{outcome}: the export released the renderer"
        );

        // The pause is still the user's: the export did not undo it, whichever way it ended.
        tick(&mut host, &mut state, 30);
        assert_eq!(
            count(&host, "thumbnail"),
            1,
            "{outcome}: the pause survived"
        );
        assert!(state.thumbnails.is_empty(), "{outcome}");

        // Asking explicitly is a decision and resumes it.
        let resumed = ok(&mut host, &mut state, json!({"op":"media.thumbnails"}));
        assert!(resumed["job"].is_string(), "{outcome}: {resumed}");
        let (splat, scene) = (id_of(&state, "splat"), id_of(&state, "scene"));
        poll_until(&mut host, &mut state, "the resumed drawing", |_, state| {
            state.thumbnail(&splat).is_some() && state.thumbnail(&scene).is_some()
        });
        assert_eq!(arena.peak(), 1, "{outcome}");
    }
}

#[test]
fn a_cancel_by_the_user_wins_over_the_stop_asked_for_an_export_in_either_order() {
    // No reservation, so only the pause can keep the tick from drawing again.
    let hold = Some(Duration::ZERO);

    // The user cancelled first; the export is asked while the frame is still ending.
    let (dir, set) = setup("cancel-then-export");
    let arena = Arc::new(Arena::default());
    let (mut state, mut host) = thumbnail_frame_in_flight(&arena, &dir, &set, hold);
    let thumbnail = only_job(&host, "thumbnail");
    ok(
        &mut host,
        &mut state,
        json!({"op":"job.cancel","job":thumbnail}),
    );
    let refused = run(&mut host, &mut state, export_request(&dir, "a.mp4")).unwrap_err();
    assert_eq!(refused.code, "renderer_busy");
    arena.hold_thumbnail.store(false, Ordering::SeqCst);
    poll_until(&mut host, &mut state, "the job to end", |host, _| {
        label(host, &thumbnail) == Some("cancelled")
    });
    let export = job_of(&ok(&mut host, &mut state, export_request(&dir, "a.mp4")));
    poll_until(&mut host, &mut state, "the export", |host, _| {
        label(host, &export) == Some("succeeded")
    });
    tick(&mut host, &mut state, 30);
    assert_eq!(count(&host, "thumbnail"), 1, "the user's pause held");

    // The host stopped it for the export; the user cancelled it before it ended.
    let (dir, set) = setup("export-then-cancel");
    let arena = Arc::new(Arena::default());
    let (mut state, mut host) = thumbnail_frame_in_flight(&arena, &dir, &set, hold);
    let thumbnail = only_job(&host, "thumbnail");
    let refused = run(&mut host, &mut state, export_request(&dir, "a.mp4")).unwrap_err();
    assert_eq!(refused.code, "renderer_busy");
    ok(
        &mut host,
        &mut state,
        json!({"op":"job.cancel","job":thumbnail}),
    );
    arena.hold_thumbnail.store(false, Ordering::SeqCst);
    poll_until(&mut host, &mut state, "the job to end", |host, _| {
        label(host, &thumbnail) == Some("cancelled")
    });
    let export = job_of(&ok(&mut host, &mut state, export_request(&dir, "a.mp4")));
    poll_until(&mut host, &mut state, "the export", |host, _| {
        label(host, &export) == Some("succeeded")
    });
    tick(&mut host, &mut state, 30);
    assert_eq!(count(&host, "thumbnail"), 1, "the user's cancel won");
    assert_eq!(arena.peak(), 1);
}

#[test]
fn an_export_does_not_wait_for_a_project_that_is_no_longer_open_and_the_new_one_draws_after() {
    let (dir, set) = setup("switch");
    let arena = Arc::new(Arena::default());
    let (first, mut host) = thumbnail_frame_in_flight(&arena, &dir, &set, None);
    let stale = only_job(&host, "thumbnail");

    // Another project is open now: the tick cancels the old job (not the user's cancel), but the
    // frame in flight still owns the renderer, so the export is refused rather than overlapping.
    let mut second = project(&dir, "second", &set);
    host.poll(&mut second);
    assert!(host.registry().get(&stale).unwrap().cancel_requested);
    let refused = run(&mut host, &mut second, export_request(&dir, "s.mp4")).unwrap_err();
    assert_eq!(refused.code, "renderer_busy");
    assert_eq!(arena.live(), 1);
    assert_eq!(count(&host, "video-export"), 0);

    arena.hold_thumbnail.store(false, Ordering::SeqCst);
    poll_until(&mut host, &mut second, "the old job to end", |host, _| {
        label(host, &stale) == Some("cancelled")
    });
    let export = job_of(&ok(&mut host, &mut second, export_request(&dir, "s.mp4")));
    poll_until(&mut host, &mut second, "the export", |host, _| {
        label(host, &export) == Some("succeeded")
    });

    // The new project was never paused by the old project's end, and nothing crossed over.
    let (splat, scene) = (id_of(&second, "splat"), id_of(&second, "scene"));
    poll_until(
        &mut host,
        &mut second,
        "the new project's thumbnails",
        |_, state| state.thumbnail(&splat).is_some() && state.thumbnail(&scene).is_some(),
    );
    assert!(
        first.thumbnails.is_empty(),
        "nothing of the old project's job landed"
    );
    assert_eq!(arena.peak(), 1);
}

#[test]
fn a_renderer_that_fails_to_start_or_draw_gives_the_renderer_back() {
    let (dir, set) = setup("renderer-errors");
    let arena = Arc::new(Arena::default());
    let mut state = project(&dir, "film", &set);
    let mut host = window(&arena, None);
    // Nothing is drawn yet: the export is the first to ask, and its renderer cannot start.
    arena.fail_build.store(true, Ordering::SeqCst);
    let export = job_of(&ok(&mut host, &mut state, export_request(&dir, "f.mp4")));
    poll_until(&mut host, &mut state, "the export to fail", |host, _| {
        label(host, &export) == Some("failed")
    });
    assert_eq!(arena.live(), 0);

    // The failed export owns nothing: the tick is granted the renderer and its job runs (and
    // fails by itself, for the same reason: this is the renderer failing, not a busy refusal).
    poll_until(&mut host, &mut state, "the tick to draw", |host, _| {
        host.registry()
            .list()
            .iter()
            .any(|job| job.kind.label() == "thumbnail" && job.state.is_final())
    });
    assert_eq!(arena.live(), 0);

    // The renderer comes back: a failed frame of an export is the same story.
    arena.fail_build.store(false, Ordering::SeqCst);
    arena.fail_export_frames.store(true, Ordering::SeqCst);
    let export = job_of(&ok(&mut host, &mut state, export_request(&dir, "g.mp4")));
    poll_until(&mut host, &mut state, "the export to fail", |host, _| {
        label(host, &export) == Some("failed")
    });
    assert_eq!(arena.live(), 0);
    arena.fail_export_frames.store(false, Ordering::SeqCst);
    let retry = job_of(&ok(&mut host, &mut state, export_request(&dir, "g.mp4")));
    poll_until(&mut host, &mut state, "the retried export", |host, _| {
        label(host, &retry) == Some("succeeded")
    });
    assert_eq!(arena.peak(), 1);
}

#[test]
fn a_finished_thumbnail_job_nobody_polled_yet_does_not_hold_the_renderer() {
    for action in ["job.commit", "job.discard"] {
        let (dir, set) = setup(&format!("finished-{action}"));
        let arena = Arc::new(Arena::default());
        let mut state = project(&dir, "film", &set);
        let mut host = window(&arena, None);
        // The window's request starts the job; no tick runs between this and its end.
        let job = job_of(&ok(&mut host, &mut state, json!({"op":"media.thumbnails"})));
        let deadline = Instant::now() + Duration::from_secs(60);
        while !host
            .registry()
            .get(&job)
            .is_some_and(|info| info.state.is_final())
        {
            assert!(Instant::now() < deadline, "{action}: the job never ended");
            std::thread::sleep(Duration::from_millis(10));
        }

        // Over means over: the export is granted at once, with the job still unreleased...
        let export = job_of(&ok(&mut host, &mut state, export_request(&dir, "r.mp4")));
        // ...and releasing it by a request, before the tick saw it, does not disturb the export.
        ok(&mut host, &mut state, json!({"op": action, "job": job}));
        poll_until(&mut host, &mut state, "the export", |host, _| {
            label(host, &export) == Some("succeeded")
        });
        assert_eq!(arena.peak(), 1, "{action}");
    }
}

/// A project that also holds the directed scene `street` (its resources approved), so a directed
/// export can run beside the library's thumbnails.
fn directed(dir: &Path, set: &StandardSet, arena: &Arc<Arena>) -> (EditorState, MediaHost) {
    let mut state = project(dir, "film", set);
    let corpus =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/directed-scene-proof/corpus");
    let mut loader = MediaHost::new().with_tools(tools());
    ok(
        &mut loader,
        &mut state,
        json!({"op":"scene.attach","id":"street","path":corpus.join("bundle.json")}),
    );
    ok(
        &mut loader,
        &mut state,
        json!({"op":"scene.resources.resolve","scene":"street","base":corpus}),
    );
    let png = dir.join("scene-frame.png");
    let rgba: Vec<u8> = (0..320 * 180).flat_map(|_| [10u8, 120, 60, 255]).collect();
    fs::write(&png, encode_png(320, 180, &rgba)).unwrap();
    let shared = arena.clone();
    let factory: Arc<SceneRendererFactory> = Arc::new(move || {
        Ok(Box::new(GateScene {
            arena: shared.clone(),
            png: png.clone(),
            _live: Live::new(&shared),
        }))
    });
    let host = window(arena, None)
        .with_scene_resources(loader.scene_resources())
        .with_scene_renderer(factory);
    (state, host)
}

#[test]
fn a_directed_export_owns_the_renderer_against_thumbnails_and_yields_nothing() {
    let (dir, set) = setup("directed-first");
    let arena = Arc::new(Arena::default());
    let (mut state, mut host) = directed(&dir, &set, &arena);
    arena.hold_scene.store(true, Ordering::SeqCst);
    let export = job_of(&ok(
        &mut host,
        &mut state,
        scene_export_request(&dir, "d.mp4"),
    ));
    poll_until(
        &mut host,
        &mut state,
        "a directed frame in flight",
        |_, _| arena.in_frame() == 1,
    );

    tick(&mut host, &mut state, 30);
    assert_eq!(count(&host, "thumbnail"), 0, "the tick starts nothing");
    let busy = run(&mut host, &mut state, json!({"op":"media.thumbnails"})).unwrap_err();
    assert_eq!(busy.code, "renderer_busy");
    assert_eq!(details(&busy)["job"], json!(export));
    let second = run(&mut host, &mut state, scene_export_request(&dir, "d2.mp4")).unwrap_err();
    assert_eq!(second.code, "renderer_busy");
    assert_eq!(arena.live(), 1);

    arena.hold_scene.store(false, Ordering::SeqCst);
    poll_until(&mut host, &mut state, "the directed export", |host, _| {
        label(host, &export) == Some("succeeded")
    });
    let (splat, scene) = (id_of(&state, "splat"), id_of(&state, "scene"));
    poll_until(
        &mut host,
        &mut state,
        "the thumbnails after it",
        |_, state| state.thumbnail(&splat).is_some() && state.thumbnail(&scene).is_some(),
    );
    assert_eq!(arena.peak(), 1);
}

#[test]
fn a_directed_export_asked_behind_a_thumbnail_frame_is_refused_and_retried() {
    let (dir, set) = setup("directed-second");
    let arena = Arc::new(Arena::default());
    let (mut state, mut host) = directed(&dir, &set, &arena);
    arena.hold_thumbnail.store(true, Ordering::SeqCst);
    poll_until(
        &mut host,
        &mut state,
        "a thumbnail frame in flight",
        |_, _| arena.in_frame() == 1,
    );
    let thumbnail = only_job(&host, "thumbnail");

    let refused = run(&mut host, &mut state, scene_export_request(&dir, "d.mp4")).unwrap_err();
    assert_eq!(refused.code, "renderer_busy");
    assert_eq!(details(&refused)["stopping"], true);
    assert_eq!(count(&host, "video-export"), 0);
    assert_eq!(arena.live(), 1);

    arena.hold_thumbnail.store(false, Ordering::SeqCst);
    poll_until(
        &mut host,
        &mut state,
        "the thumbnail job to end",
        |host, _| label(host, &thumbnail) == Some("cancelled"),
    );
    assert_eq!(arena.live(), 0);
    let export = job_of(&ok(
        &mut host,
        &mut state,
        scene_export_request(&dir, "d.mp4"),
    ));
    poll_until(&mut host, &mut state, "the directed export", |host, _| {
        label(host, &export) == Some("succeeded")
    });
    assert_eq!(arena.peak(), 1);
}

fn preflight_request(dir: &Path, name: &str) -> Value {
    json!({"op":"export.preflight","preset":tiny(),"destination":dir.join(name),"range":[0,4]})
}

/// The explicit retry a caller makes after `renderer_busy`: asks again until the renderer is its
/// own, never longer than a minute. Every answer other than "busy" ends the loop.
fn ask_until_granted(host: &mut MediaHost, state: &mut EditorState, request: Value) -> Value {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match run(host, state, request.clone()) {
            Ok(reply) => return reply,
            Err(error) if error.code == "renderer_busy" => {
                assert!(Instant::now() < deadline, "the renderer never came free");
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("{request}: {error:?}"),
        }
    }
}

/// A window host whose first thing is a renderer probe parked inside its capability query.
fn probe_in_flight(
    arena: &Arc<Arena>,
    dir: &Path,
    set: &StandardSet,
    hold: Option<Duration>,
) -> (EditorState, MediaHost, String) {
    let mut state = project(dir, "film", set);
    arena.hold_probe.store(true, Ordering::SeqCst);
    // The tick has nothing to draw yet: the preflight is the first to ask for the renderer.
    let mut host = window(arena, hold);
    let job = job_of(&ok(&mut host, &mut state, preflight_request(dir, "p.mp4")));
    let deadline = Instant::now() + Duration::from_secs(60);
    while arena.probing.load(Ordering::SeqCst) != 1 {
        assert!(
            Instant::now() < deadline,
            "the probe never reached the renderer"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    (state, host, job)
}

#[test]
fn a_cancelled_preflight_keeps_the_renderer_until_its_probe_lets_it_go() {
    // BL144.18: the probe asks the renderer for its limits on a side thread, so cancelling the job
    // ends the job at once while the renderer is still alive. The registry says `cancelled`; the
    // renderer is not free until the thread drops it. Counting the registry alone would start a
    // second renderer here (peak 2): the counter below is what proves it does not.
    let (dir, set) = setup("cancelled-probe");
    let arena = Arc::new(Arena::default());
    let (mut state, mut host, preflight) = probe_in_flight(&arena, &dir, &set, None);
    assert_eq!(arena.live(), 1);

    ok(
        &mut host,
        &mut state,
        json!({"op":"job.cancel","job":preflight}),
    );
    poll_until(
        &mut host,
        &mut state,
        "the preflight job to end",
        |host, _| label(host, &preflight) == Some("cancelled"),
    );
    assert_eq!(arena.live(), 1, "the job is over, its renderer is not");

    // Every request for a renderer is refused while the probe still has it, and none is queued.
    let export = run(&mut host, &mut state, export_request(&dir, "e.mp4")).unwrap_err();
    assert_eq!(export.code, "renderer_busy");
    assert_eq!(details(&export)["retryable"], true);
    assert_eq!(details(&export)["queued"], false);
    assert_eq!(details(&export)["job"], json!(preflight));
    let again = run(&mut host, &mut state, preflight_request(&dir, "p2.mp4")).unwrap_err();
    assert_eq!(again.code, "renderer_busy");
    let thumbnails = run(&mut host, &mut state, json!({"op":"media.thumbnails"})).unwrap_err();
    assert_eq!(thumbnails.code, "renderer_busy");
    tick(&mut host, &mut state, 20);
    assert_eq!(
        count(&host, "thumbnail"),
        0,
        "the tick draws nothing behind a probe"
    );
    assert_eq!(count(&host, "video-export"), 0);
    assert_eq!(arena.live(), 1);
    assert_eq!(arena.peak(), 1, "never two renderers");

    // The probe lets go; only then is the renderer anyone's.
    arena.hold_probe.store(false, Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_secs(60);
    while arena.live() != 0 {
        assert!(
            Instant::now() < deadline,
            "the probe never dropped its renderer"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let started = ask_until_granted(&mut host, &mut state, export_request(&dir, "e.mp4"));
    let export = job_of(&started);
    poll_until(&mut host, &mut state, "the export", |host, _| {
        label(host, &export) == Some("succeeded")
    });
    assert_eq!(
        arena.peak(),
        1,
        "the export built its renderer after the probe's was gone"
    );
}

#[test]
fn a_new_preflight_right_after_cancelling_the_previous_one_is_refused_once_then_granted() {
    // The export dialog cancels its previous report and asks again at once (BL144.19). The first
    // ask is a clear, retryable refusal that also stops the old probe; the retry succeeds.
    let (dir, set) = setup("preflight-after-cancel");
    let arena = Arc::new(Arena::default());
    let (mut state, mut host, first) = probe_in_flight(&arena, &dir, &set, None);

    host.registry().cancel(&first).unwrap();
    let refused = run(&mut host, &mut state, preflight_request(&dir, "p2.mp4")).unwrap_err();
    assert_eq!(refused.code, "renderer_busy");
    assert_eq!(details(&refused)["retryable"], true);
    assert_eq!(details(&refused)["queued"], false);
    assert_eq!(details(&refused)["stopping"], true);
    assert_eq!(count(&host, "preflight"), 1, "refused, not accepted");
    assert_eq!(arena.peak(), 1);

    arena.hold_probe.store(false, Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_secs(60);
    while arena.live() != 0 {
        assert!(Instant::now() < deadline, "the old probe never ended");
        std::thread::sleep(Duration::from_millis(5));
    }
    let second = job_of(&ask_until_granted(
        &mut host,
        &mut state,
        preflight_request(&dir, "p2.mp4"),
    ));
    poll_until(&mut host, &mut state, "the second preflight", |host, _| {
        label(host, &second) == Some("succeeded")
    });
    assert_eq!(arena.peak(), 1);
}

#[test]
fn an_export_asked_while_the_dialogs_preflight_still_probes_stops_it_and_is_retried() {
    // `Exportar` pressed while the dialog still says "Verificando…": the verification is read-only
    // and out of date, so the export stops it. The export itself is not accepted yet.
    let (dir, set) = setup("export-behind-probe");
    let arena = Arc::new(Arena::default());
    let (mut state, mut host, preflight) = probe_in_flight(&arena, &dir, &set, None);

    let refused = run(&mut host, &mut state, export_request(&dir, "e.mp4")).unwrap_err();
    assert_eq!(refused.code, "renderer_busy");
    assert_eq!(details(&refused)["stopping"], true);
    assert_eq!(details(&refused)["job"], json!(preflight));
    assert!(host.registry().get(&preflight).unwrap().cancel_requested);
    assert_eq!(count(&host, "video-export"), 0);
    assert_eq!(arena.live(), 1);

    // Still reserved for the export: the tick does not give the renderer to thumbnails.
    arena.hold_probe.store(false, Ordering::SeqCst);
    poll_until(&mut host, &mut state, "the probe to end", |_, _| {
        arena.live() == 0
    });
    tick(&mut host, &mut state, 20);
    assert_eq!(count(&host, "thumbnail"), 0, "reserved for the retry");

    let export = job_of(&ask_until_granted(
        &mut host,
        &mut state,
        export_request(&dir, "e.mp4"),
    ));
    poll_until(&mut host, &mut state, "the export", |host, _| {
        label(host, &export) == Some("succeeded")
    });
    assert_eq!(arena.peak(), 1);
}

#[test]
fn the_users_pause_survives_a_preflight_that_finishes_is_cancelled_or_cannot_start() {
    for outcome in ["finishes", "is cancelled", "cannot start a renderer"] {
        let (dir, set) = setup(&format!("pause-preflight-{}", outcome.replace(' ', "-")));
        let arena = Arc::new(Arena::default());
        let (mut state, mut host) = thumbnail_frame_in_flight(&arena, &dir, &set, None);
        let thumbnail = only_job(&host, "thumbnail");
        ok(
            &mut host,
            &mut state,
            json!({"op":"job.cancel","job":thumbnail}),
        );
        arena.hold_thumbnail.store(false, Ordering::SeqCst);
        poll_until(&mut host, &mut state, "the job to end", |host, _| {
            label(host, &thumbnail) == Some("cancelled")
        });
        tick(&mut host, &mut state, 20);
        assert_eq!(
            count(&host, "thumbnail"),
            1,
            "{outcome}: paused before the preflight"
        );

        match outcome {
            "is cancelled" => arena.hold_probe.store(true, Ordering::SeqCst),
            "cannot start a renderer" => arena.fail_build.store(true, Ordering::SeqCst),
            _ => {}
        }
        let preflight = job_of(&ok(&mut host, &mut state, preflight_request(&dir, "p.mp4")));
        if outcome == "is cancelled" {
            let deadline = Instant::now() + Duration::from_secs(60);
            while arena.probing.load(Ordering::SeqCst) != 1 {
                assert!(Instant::now() < deadline, "the probe never started");
                std::thread::sleep(Duration::from_millis(5));
            }
            // The dialog's way of cancelling: the registry, not `job.cancel`.
            host.registry().cancel(&preflight).unwrap();
            arena.hold_probe.store(false, Ordering::SeqCst);
        }
        let expected = if outcome == "is cancelled" {
            "cancelled"
        } else {
            "succeeded"
        };
        poll_until(&mut host, &mut state, "the preflight to end", |host, _| {
            label(host, &preflight) == Some(expected)
        });
        poll_until(&mut host, &mut state, "the renderer to be free", |_, _| {
            arena.live() == 0
        });
        arena.fail_build.store(false, Ordering::SeqCst);

        // The pause is still the user's: the preflight did not undo it, whichever way it ended.
        tick(&mut host, &mut state, 30);
        assert_eq!(
            count(&host, "thumbnail"),
            1,
            "{outcome}: the pause survived"
        );
        assert!(state.thumbnails.is_empty(), "{outcome}");
        let resumed = ok(&mut host, &mut state, json!({"op":"media.thumbnails"}));
        assert!(resumed["job"].is_string(), "{outcome}: {resumed}");
        let (splat, scene) = (id_of(&state, "splat"), id_of(&state, "scene"));
        poll_until(&mut host, &mut state, "the resumed drawing", |_, state| {
            state.thumbnail(&splat).is_some() && state.thumbnail(&scene).is_some()
        });
        assert_eq!(arena.peak(), 1, "{outcome}");
    }
}

#[test]
fn a_cancel_that_does_not_come_through_the_host_still_wins_over_the_stop_for_an_export() {
    // The jobs dialog cancels through the registry, not through `job.cancel`. The host must not
    // need to hear about it: the job's own count of requests tells the user's cancel from its own.
    let hold = Some(Duration::ZERO);
    let (dir, set) = setup("registry-cancel");
    let arena = Arc::new(Arena::default());
    let (mut state, mut host) = thumbnail_frame_in_flight(&arena, &dir, &set, hold);
    let thumbnail = only_job(&host, "thumbnail");
    let refused = run(&mut host, &mut state, export_request(&dir, "a.mp4")).unwrap_err();
    assert_eq!(refused.code, "renderer_busy");
    // A second refusal asks nobody to stop a second time: the count is the user's alone.
    let refused = run(&mut host, &mut state, export_request(&dir, "a.mp4")).unwrap_err();
    assert_eq!(refused.code, "renderer_busy");
    assert_eq!(host.registry().get(&thumbnail).unwrap().cancel_requests, 1);

    host.registry().cancel(&thumbnail).unwrap();
    arena.hold_thumbnail.store(false, Ordering::SeqCst);
    poll_until(&mut host, &mut state, "the job to end", |host, _| {
        label(host, &thumbnail) == Some("cancelled")
    });
    tick(&mut host, &mut state, 30);
    assert_eq!(
        count(&host, "thumbnail"),
        1,
        "the user's cancel paused the drawing"
    );
    assert_eq!(arena.peak(), 1);
}

#[test]
fn the_user_cancelling_while_an_export_forces_the_stop_always_leaves_the_pause() {
    // The user's cancel and the export's forced stop race on two threads, many times over. Whoever
    // wins the registry, the user's request is counted: the pause must hold in every round.
    for round in 0..12 {
        let (dir, set) = setup(&format!("race-{round}"));
        let arena = Arc::new(Arena::default());
        let (mut state, mut host) =
            thumbnail_frame_in_flight(&arena, &dir, &set, Some(Duration::ZERO));
        let thumbnail = only_job(&host, "thumbnail");
        let registry = host.registry().clone();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let user = {
            let (barrier, thumbnail) = (barrier.clone(), thumbnail.clone());
            std::thread::spawn(move || {
                barrier.wait();
                for _ in 0..(round % 4) {
                    std::thread::yield_now();
                }
                let _ = registry.cancel(&thumbnail);
            })
        };
        barrier.wait();
        let _ = run(&mut host, &mut state, export_request(&dir, "a.mp4"));
        user.join().unwrap();
        arena.hold_thumbnail.store(false, Ordering::SeqCst);
        poll_until(&mut host, &mut state, "the job to end", |host, _| {
            label(host, &thumbnail) == Some("cancelled")
        });
        tick(&mut host, &mut state, 20);
        assert!(
            host.registry().get(&thumbnail).unwrap().cancel_requests >= 1,
            "round {round}: the user's request was counted"
        );
        assert_eq!(
            count(&host, "thumbnail"),
            1,
            "round {round}: the pause held"
        );
        assert_eq!(arena.peak(), 1, "round {round}");
    }
}

#[test]
fn the_reservation_for_a_refused_export_expires_and_the_tick_draws_again() {
    let (dir, set) = setup("reservation-expiry");
    let arena = Arc::new(Arena::default());
    let hold = Duration::from_millis(1500);
    let (mut state, mut host) = thumbnail_frame_in_flight(&arena, &dir, &set, Some(hold));
    let thumbnail = only_job(&host, "thumbnail");
    let refused = run(&mut host, &mut state, export_request(&dir, "a.mp4")).unwrap_err();
    assert_eq!(refused.code, "renderer_busy");
    arena.hold_thumbnail.store(false, Ordering::SeqCst);
    poll_until(
        &mut host,
        &mut state,
        "the stopped job to end",
        |host, _| label(host, &thumbnail) == Some("cancelled"),
    );
    tick(&mut host, &mut state, 10);
    assert_eq!(
        count(&host, "thumbnail"),
        1,
        "inside the reservation nothing is drawn"
    );

    // Nobody asked again: the reservation runs out and the drawing comes back by itself. The
    // export that was refused does not.
    std::thread::sleep(hold + Duration::from_millis(300));
    let (splat, scene) = (id_of(&state, "splat"), id_of(&state, "scene"));
    poll_until(
        &mut host,
        &mut state,
        "the drawing after the reservation",
        |_, state| state.thumbnail(&splat).is_some() && state.thumbnail(&scene).is_some(),
    );
    assert_eq!(count(&host, "video-export"), 0);
    assert_eq!(arena.peak(), 1);
}

#[test]
fn a_reservation_made_for_one_project_never_delays_another_project() {
    let (dir, set) = setup("reservation-switch");
    let arena = Arc::new(Arena::default());
    // The default 30 s: only the project's stamp can end it early.
    let (first, mut host) = thumbnail_frame_in_flight(&arena, &dir, &set, None);
    let mut first = first;
    let stale = only_job(&host, "thumbnail");
    let refused = run(&mut host, &mut first, export_request(&dir, "a.mp4")).unwrap_err();
    assert_eq!(refused.code, "renderer_busy");
    arena.hold_thumbnail.store(false, Ordering::SeqCst);

    let mut second = project(&dir, "second", &set);
    poll_until(
        &mut host,
        &mut second,
        "the stopped job to end",
        |host, _| label(host, &stale) == Some("cancelled"),
    );
    let (splat, scene) = (id_of(&second, "splat"), id_of(&second, "scene"));
    // Far inside the 30 s of the reservation: it was made for the other project, not this one.
    poll_within(
        &mut host,
        &mut second,
        "the other project's thumbnails, with no export in sight",
        Duration::from_secs(10),
        |_, state| state.thumbnail(&splat).is_some() && state.thumbnail(&scene).is_some(),
    );
    assert_eq!(count(&host, "video-export"), 0);
    assert_eq!(arena.peak(), 1);
}

#[test]
fn a_request_that_would_fail_anyway_stops_nothing_and_reserves_nothing() {
    let (dir, set) = setup("invalid-requests");
    let arena = Arc::new(Arena::default());
    let (mut state, mut host) = thumbnail_frame_in_flight(&arena, &dir, &set, None);
    let thumbnail = only_job(&host, "thumbnail");
    for bad in [
        json!({"op":"export.start","preset":tiny()}),
        json!({"op":"export.start","preset":"no-such-preset","destination":dir.join("x.mp4")}),
        json!({"op":"export.start","preset":tiny(),"destination":dir.join("x.mp4"),"range":[3]}),
        json!({"op":"export.preflight","preset":tiny()}),
        json!({"op":"media.thumbnails","assets":["no-such-asset"]}),
    ] {
        let error = run(&mut host, &mut state, bad.clone()).unwrap_err();
        assert_ne!(error.code, "renderer_busy", "{bad}: {error:?}");
        let info = host.registry().get(&thumbnail).unwrap();
        assert_eq!(
            info.cancel_requests, 0,
            "{bad}: the thumbnail job was not stopped"
        );
        assert!(!info.cancel_requested, "{bad}");
    }
    // Not reserved either: the explicit thumbnail request still reaches its own family's answer
    // (a job already runs: reuse), never `renderer_busy`.
    let reply = ok(&mut host, &mut state, json!({"op":"media.thumbnails"}));
    assert_eq!(reply["reused"], true, "{reply}");

    arena.hold_thumbnail.store(false, Ordering::SeqCst);
    poll_until(&mut host, &mut state, "the job to end", |host, _| {
        label(host, &thumbnail) == Some("succeeded") || label(host, &thumbnail).is_none()
    });
    assert_eq!(arena.peak(), 1);
}
