//! BL094 / BL112 through the real `MediaHost` (F025, F040): `job.discard` gives up the finished,
//! unapplied result of a job from a background `discard` job. The result keeps owning its files
//! (and the registry keeps its record) until that job has taken them back; an active, applied or
//! already discarded job is refused with a typed error; a window never deletes on its own thread.
//!
//! Every case uses the real host, real adoptions of synthetic PNGs and the job registry. Waits are
//! on job end states; "the job is still discarding" is held open deterministically by occupying
//! the registry's running slots (the discard job queues behind them), never by a delay. Unproven:
//! a native window tick, release-build timing, Windows, a second process, a hostile writer.

mod directed_resources_common;

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use directed_resources_common::*;
use serde_json::{Value, json};
use tessel_editor_core::jobs::{CancelToken, JobKind, JobRegistry, JobState, ResultState};
use tessel_editor_core::{CommandError, EditorState, headless, persist};
use tessel_editor_media::host::MediaHost;
use tessel_editor_media::scene_resources::tracked_under;

struct World {
    sandbox: Sandbox,
    state: EditorState,
    host: Option<MediaHost>,
    workspace: PathBuf,
}

fn world(name: &str, host: MediaHost) -> World {
    let sandbox = Sandbox::new(&format!("te-discard-{name}"));
    let workspace = sandbox.root.join("ws");
    fs::create_dir_all(&workspace).unwrap();
    let mut state = EditorState::demo();
    headless::execute(&mut state, &json!({"op":"project.create","name":"discard"})).unwrap();
    persist::save(&mut state, &workspace.join("p.tessel.json")).unwrap();
    let bundle = standard_bundle(&sandbox);
    let mut world = World {
        sandbox,
        state,
        host: Some(host),
        workspace,
    };
    let path = world.sandbox.root.join("bundles/street.bundle.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, serde_json::to_vec_pretty(&bundle).unwrap()).unwrap();
    world
        .run(json!({"op":"scene.attach","id":"street","path":path}))
        .unwrap();
    let document = world.workspace.join("p.tessel.json");
    persist::save(&mut world.state, &document).unwrap();
    world
}

impl World {
    fn host(&mut self) -> &mut MediaHost {
        self.host.as_mut().expect("the host is still alive")
    }

    fn run(&mut self, request: Value) -> Result<Value, CommandError> {
        let host = self.host.as_mut().expect("the host is still alive");
        headless::execute_ext(&mut self.state, None, Some(host), &request)
    }

    fn registry(&mut self) -> JobRegistry {
        self.host().registry().clone()
    }

    /// `scene.resources.adopt` as a job that finished and waits to be applied or discarded.
    fn adopt_job(&mut self) -> String {
        let source = self.sandbox.src.clone();
        let started = self
            .run(
                json!({"op":"scene.resources.adopt","scene":"street","source":source,"wait":false}),
            )
            .unwrap();
        let job = started["job"].as_str().unwrap().to_string();
        let waited = self.run(json!({"op":"job.wait","job":job})).unwrap();
        assert_eq!(waited["state"], "succeeded", "{waited}");
        job
    }

    fn state_of(&mut self, job: &str) -> String {
        let info = self.run(json!({"op":"job.get","job":job})).unwrap();
        info["result_state"].as_str().unwrap().to_string()
    }

    fn files(&self) -> [PathBuf; 2] {
        [
            self.workspace.join("scenes/street/assets/backdrop.png"),
            self.workspace.join("scenes/street/assets/figure.png"),
        ]
    }

    fn assert_files_kept(&self) {
        for path in self.files() {
            assert!(path.is_file(), "`{}` must still be there", path.display());
        }
    }

    fn assert_nothing_copied(&self) {
        assert!(
            !self.workspace.join("scenes").exists(),
            "no `scenes/` left: {:?}",
            tree(&self.workspace)
        );
        assert_eq!(tracked_under(&self.workspace), 0);
    }
}

/// Fills every running slot of `registry` with jobs that end when the token is cancelled, so any
/// job spawned afterwards stays `queued`.
struct Blockers {
    release: CancelToken,
    jobs: Vec<String>,
    registry: JobRegistry,
}

impl Blockers {
    fn hold(registry: &JobRegistry) -> Self {
        let release = CancelToken::new();
        let jobs: Vec<String> = (0..2)
            .map(|_| {
                let hold = release.clone();
                registry.spawn(JobKind::Proxy, "blocker", move |_| {
                    while !hold.is_cancelled() {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Ok(json!(null))
                })
            })
            .collect();
        for job in &jobs {
            let deadline = Instant::now() + Duration::from_secs(10);
            while registry.get(job).unwrap().state != JobState::Running {
                assert!(Instant::now() < deadline, "a blocker never started");
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        Self {
            release,
            jobs,
            registry: registry.clone(),
        }
    }

    fn release(self) {
        self.release.cancel();
        for job in &self.jobs {
            self.registry.wait(job, Duration::from_secs(10)).unwrap();
        }
    }
}

fn wait_end(world: &mut World, job: &str) -> Value {
    world
        .run(json!({"op":"job.wait","job":job,"timeout_ms":30000}))
        .unwrap()
}

// ---------------------------------------------------------------------------- the happy path --

#[test]
fn a_finished_unapplied_adopt_is_discarded_by_a_job_that_is_observable() {
    let mut world = world("observable", MediaHost::new());
    let job = world.adopt_job();
    world.assert_files_kept();
    assert_eq!(world.state_of(&job), "pending");

    let started = world
        .run(json!({"op":"job.discard","job":job,"wait":false}))
        .unwrap();
    assert_eq!(started["action"], "job.discard", "{started}");
    assert_eq!(started["discards"], job.as_str());
    assert_eq!(started["state"], "discarding");
    let discard = started["job"].as_str().unwrap().to_string();
    assert_ne!(discard, job, "the discard is a job of its own");

    let ended = wait_end(&mut world, &discard);
    assert_eq!(ended["kind"], "discard", "{ended}");
    assert_eq!(ended["state"], "succeeded", "{ended}");
    assert_eq!(ended["result"]["discards"], job.as_str());
    assert_eq!(ended["result"]["discarded"], "scene-adopt");
    assert_eq!(ended["result"]["cleanup"]["removedFiles"], 2, "{ended}");
    assert_eq!(ended["result"]["cleanup"]["kept"], json!([]));

    assert_eq!(world.state_of(&job), "discarded");
    world.assert_nothing_copied();
    let listed = world.run(json!({"op":"job.list"})).unwrap();
    let kinds: Vec<&str> = listed["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|info| info["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"discard"), "{listed}");
}

#[test]
fn a_host_without_a_window_waits_for_the_discard_unless_told_not_to() {
    let mut world = world("waits", MediaHost::new());
    let job = world.adopt_job();

    let answer = world.run(json!({"op":"job.discard","job":job})).unwrap();
    assert_eq!(answer["state"], "discarded", "{answer}");
    assert_eq!(answer["discards"], job.as_str());
    assert_eq!(answer["cleanup"]["removedFiles"], 2, "{answer}");
    assert_eq!(world.state_of(&job), "discarded");
    world.assert_nothing_copied();
}

#[test]
fn a_discarded_result_cannot_be_committed_and_nothing_changes_in_the_project() {
    let mut world = world("commit-after", MediaHost::new());
    let job = world.adopt_job();
    let revision = world.state.project.revision;
    world.run(json!({"op":"job.discard","job":job})).unwrap();

    let error = world.run(json!({"op":"job.commit","job":job})).unwrap_err();
    assert_eq!(error.code, "job_discarded", "{error:?}");
    assert_eq!(world.state.project.revision, revision);
    world.assert_nothing_copied();
}

// ----------------------------------------------------------------------------------- refusals --

#[test]
fn an_applied_result_is_refused_and_keeps_its_files() {
    let mut world = world("applied", MediaHost::new());
    let job = world.adopt_job();
    world.run(json!({"op":"job.commit","job":job})).unwrap();
    assert_eq!(world.state_of(&job), "applied");

    let error = world
        .run(json!({"op":"job.discard","job":job}))
        .unwrap_err();
    assert_eq!(error.code, "job_applied", "{error:?}");
    world.assert_files_kept();
    assert_eq!(world.state_of(&job), "applied");
    let kinds = world.registry().list();
    assert!(
        kinds.iter().all(|info| info.kind != JobKind::Discard),
        "a refusal starts no job"
    );
}

#[test]
fn an_active_job_is_refused_until_it_ends() {
    let mut world = world("active", MediaHost::new());
    let registry = world.registry();
    let blockers = Blockers::hold(&registry);
    let running = blockers.jobs[0].clone();

    let error = world
        .run(json!({"op":"job.discard","job":running}))
        .unwrap_err();
    assert_eq!(error.code, "job_running", "{error:?}");
    assert_eq!(registry.get(&running).unwrap().state, JobState::Running);
    blockers.release();
}

#[test]
fn a_job_that_made_no_result_or_failed_has_nothing_to_discard() {
    let mut world = world("nothing", MediaHost::new());
    // A preflight reports and changes nothing.
    let source = world.sandbox.src.clone();
    let started = world
        .run(json!({"op":"scene.resources.preflight","scene":"street","base":source,"wait":false}))
        .unwrap();
    let preflight = started["job"].as_str().unwrap().to_string();
    wait_end(&mut world, &preflight);
    let error = world
        .run(json!({"op":"job.discard","job":preflight}))
        .unwrap_err();
    assert_eq!(error.code, "job_nothing_to_discard", "{error:?}");

    // A failed adoption (no such source) left nothing either.
    let missing = world.sandbox.root.join("missing");
    let started = world
        .run(json!({"op":"scene.resources.adopt","scene":"street","source":missing,"wait":false}))
        .unwrap();
    let failed = started["job"].as_str().unwrap().to_string();
    let ended = wait_end(&mut world, &failed);
    assert_eq!(ended["state"], "failed", "{ended}");
    assert_eq!(ended["result_state"], "none");
    let error = world
        .run(json!({"op":"job.discard","job":failed}))
        .unwrap_err();
    assert_eq!(error.code, "job_nothing_to_discard", "{error:?}");
    world.assert_nothing_copied();
}

#[test]
fn an_unknown_job_is_refused_by_name() {
    let mut world = world("unknown", MediaHost::new());
    let error = world
        .run(json!({"op":"job.discard","job":"job-404"}))
        .unwrap_err();
    assert_eq!(error.code, "unknown_element", "{error:?}");
    let error = world.run(json!({"op":"job.discard"})).unwrap_err();
    assert_eq!(error.code, "invalid_request", "{error:?}");
}

// ----------------------------------------------------------------------------------- ownership --

#[test]
fn the_result_keeps_its_files_and_its_record_until_the_discard_job_has_finished() {
    let mut world = world("ownership", MediaHost::new());
    let job = world.adopt_job();
    let registry = world.registry();
    let blockers = Blockers::hold(&registry);

    let started = world
        .run(json!({"op":"job.discard","job":job,"wait":false}))
        .unwrap();
    let discard = started["job"].as_str().unwrap().to_string();
    // The discard job waits for a slot: it has not started, so nothing was taken back.
    assert_eq!(registry.get(&discard).unwrap().state, JobState::Queued);
    assert_eq!(world.state_of(&job), "discarding");
    world.assert_files_kept();
    assert!(
        tracked_under(&world.workspace) > 0,
        "the files are still claimed by their result"
    );

    // Everything that could take the result from the discard says no, typed.
    let error = world.run(json!({"op":"job.commit","job":job})).unwrap_err();
    assert_eq!(error.code, "job_discarding", "{error:?}");
    let error = world
        .run(json!({"op":"job.discard","job":job}))
        .unwrap_err();
    assert_eq!(error.code, "job_discarding", "{error:?}");
    let error = world
        .run(json!({"op":"job.cancel","job":discard}))
        .unwrap_err();
    assert_eq!(error.code, "job_not_cancellable", "{error:?}");
    assert!(!registry.get(&discard).unwrap().cancel_requested);

    // `prune` does not forget a record that still owns files, nor the job that is removing them.
    registry.prune(0);
    assert!(registry.get(&job).is_some(), "the result's record stays");
    assert!(registry.get(&discard).is_some());
    world.assert_files_kept();

    blockers.release();
    let ended = wait_end(&mut world, &discard);
    assert_eq!(ended["state"], "succeeded", "{ended}");
    assert_eq!(world.state_of(&job), "discarded");
    world.assert_nothing_copied();

    // Now nothing is owed to anyone: the records can go.
    registry.prune(0);
    assert!(registry.get(&job).is_none());
    assert!(registry.get(&discard).is_none());
}

#[test]
fn prune_never_drops_a_finished_result_nobody_has_applied_or_discarded() {
    let mut world = world("prune", MediaHost::new());
    let job = world.adopt_job();
    let registry = world.registry();
    registry.prune(0);
    assert_eq!(
        registry.get(&job).map(|info| info.result_state),
        Some(ResultState::Pending),
        "pruned while pending"
    );

    // Still committable after the prune, and then it may be forgotten.
    let committed = world.run(json!({"op":"job.commit","job":job})).unwrap();
    assert_eq!(committed["action"], "scene.resources.adopt", "{committed}");
    world.assert_files_kept();
    registry.prune(0);
    assert!(registry.get(&job).is_none());
}

#[test]
fn discarding_one_of_two_results_never_removes_what_the_other_relies_on() {
    let mut world = world("shared", MediaHost::new());
    let first = world.adopt_job();
    let second = world.adopt_job();

    world.run(json!({"op":"job.discard","job":first})).unwrap();
    world.assert_files_kept();
    let committed = world.run(json!({"op":"job.commit","job":second})).unwrap();
    assert_eq!(committed["action"], "scene.resources.adopt", "{committed}");
    world.assert_files_kept();
    assert_eq!(tracked_under(&world.workspace), 0);
}

#[test]
fn a_file_the_user_rewrote_is_kept_and_reported_by_the_discard() {
    let mut world = world("rewritten", MediaHost::new());
    let job = world.adopt_job();
    let [backdrop, figure] = world.files();
    let length = fs::metadata(&backdrop).unwrap().len();
    // Same inode, same length, other content: not the file that was published.
    let mut edited = fs::read(&backdrop).unwrap();
    let last = edited.len() - 1;
    edited[last] ^= 0xff;
    fs::write(&backdrop, &edited).unwrap();
    assert_eq!(fs::metadata(&backdrop).unwrap().len(), length);

    let answer = world.run(json!({"op":"job.discard","job":job})).unwrap();
    assert_eq!(answer["cleanup"]["removedFiles"], 1, "{answer}");
    // The edited file stays, and so do the folders that still hold it.
    let kept = answer["cleanup"]["kept"].as_array().unwrap();
    assert!(
        kept.iter()
            .any(|why| why.as_str().unwrap().contains("rewritten in place")),
        "{answer}"
    );
    assert_eq!(kept.len(), 4, "{answer}");
    assert_eq!(fs::read(&backdrop).unwrap(), edited, "the edit survives");
    assert!(!figure.exists());
    assert_eq!(world.state_of(&job), "discarded");
}

// -------------------------------------------------------------------------------------- window --

fn poll_until_reported(world: &mut World, job: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let host = world.host.as_mut().unwrap();
        let events = host.poll(&mut world.state);
        if let Some(event) = events.into_iter().find(|event| event["job"] == job) {
            return event;
        }
        assert!(Instant::now() < deadline, "the job was never reported");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn interactive_adopt(world: &mut World) -> String {
    let source = world.sandbox.src.clone();
    let started = world
        .run(json!({"op":"scene.resources.adopt","scene":"street","source":source}))
        .unwrap();
    assert_eq!(started["state"], "queued", "{started}");
    started["job"].as_str().unwrap().to_string()
}

fn wait_final(registry: &JobRegistry, job: &str) {
    registry
        .wait(job, Duration::from_secs(30))
        .unwrap_or_else(|| panic!("`{job}` never ended"));
}

#[test]
fn a_window_never_deletes_on_its_tick_a_stale_result_goes_to_a_discard_job() {
    let mut world = world("tick", MediaHost::new().interactive());
    let job = interactive_adopt(&mut world);
    let registry = world.registry();
    wait_final(&registry, &job);
    // The project is replaced before the next tick sees the finished job; every running slot is
    // busy, so the discard job cannot start while the tick returns.
    world
        .run(json!({"op":"project.create","name":"next"}))
        .unwrap();
    let blockers = Blockers::hold(&registry);

    let event = poll_until_reported(&mut world, &job);
    assert_eq!(event["state"], "failed", "{event}");
    assert_eq!(event["error"]["code"], "stale_result", "{event}");
    let discard = event["error"]["details"]["discard_job"]
        .as_str()
        .unwrap()
        .to_string();
    world.assert_files_kept();
    assert!(
        tracked_under(&world.workspace) > 0,
        "still owned by the result"
    );
    assert_eq!(
        registry.get(&job).unwrap().result_state,
        ResultState::Discarding
    );
    assert_eq!(registry.get(&discard).unwrap().state, JobState::Queued);

    blockers.release();
    let ended = poll_until_reported(&mut world, &discard);
    assert_eq!(ended["kind"], "discard", "{ended}");
    assert_eq!(ended["state"], "succeeded", "{ended}");
    assert_eq!(ended["result"]["cleanup"]["removedFiles"], 2, "{ended}");
    assert_eq!(
        registry.get(&job).unwrap().result_state,
        ResultState::Discarded
    );
    world.assert_nothing_copied();
    // Each event is delivered once.
    let again = world.host.as_mut().unwrap().poll(&mut world.state);
    assert!(again.iter().all(|event| event["job"] != discard.as_str()));
}

#[test]
fn a_window_discards_a_finished_result_on_request_without_waiting() {
    let mut world = world("window-discard", MediaHost::new().interactive());
    let source = world.sandbox.src.clone();
    // Not polled yet: the result waits in its slot while the discard claims it.
    let started = world
        .run(json!({"op":"scene.resources.adopt","scene":"street","source":source}))
        .unwrap();
    let job = started["job"].as_str().unwrap().to_string();
    let registry = world.registry();
    wait_final(&registry, &job);

    let answer = world.run(json!({"op":"job.discard","job":job})).unwrap();
    assert_eq!(answer["state"], "discarding", "{answer}");
    let discard = answer["job"].as_str().unwrap().to_string();
    let ended = poll_until_reported(&mut world, &discard);
    assert_eq!(ended["state"], "succeeded", "{ended}");
    world.assert_nothing_copied();
    // The tick that follows has nothing to apply for the discarded job.
    let later = world.host.as_mut().unwrap().poll(&mut world.state);
    assert!(later.iter().all(|event| event["job"] != job.as_str()));
    assert_eq!(
        registry.get(&job).unwrap().result_state,
        ResultState::Discarded
    );
}

#[test]
fn a_window_host_that_goes_away_hands_pending_files_to_a_discard_job() {
    let mut world = world("window-drop", MediaHost::new().interactive());
    let job = interactive_adopt(&mut world);
    let registry = world.registry();
    wait_final(&registry, &job);
    world.assert_files_kept();

    drop(world.host.take());
    // The drop returned without hashing: the cleanup runs on a job of the surviving registry.
    let discards: Vec<String> = registry
        .list()
        .into_iter()
        .filter(|info| info.kind == JobKind::Discard)
        .map(|info| info.id)
        .collect();
    assert_eq!(discards.len(), 1, "one discard job for one pending result");
    wait_final(&registry, &discards[0]);
    assert_eq!(
        registry.get(&job).unwrap().result_state,
        ResultState::Discarded
    );
    world.assert_nothing_copied();
}

// ------------------------------------------------------------------------------ measurement --

/// Measurement with a gate on what matters (BL112): while a result holding one large file is
/// discarded, the caller (the window's thread) is back in a small fraction of the time the removal
/// takes, which is spent on the discard job. Run on purpose, debug or `--release`, with
/// `-- --ignored --nocapture`; it writes about 2 x 256 MiB under the temp folder.
#[test]
#[ignore = "measurement: writes and hashes a 256 MiB file"]
fn measure_the_caller_is_not_blocked_while_a_large_result_is_discarded() {
    const EXTRA: usize = 256 << 20;
    let mut world = world("cost", MediaHost::new().interactive());
    let mut document = fixture();
    // A valid PNG followed by padding: the transport only reads the header.
    let mut big = png(64, 36, 10);
    big.resize(big.len() + EXTRA, 0);
    let backdrop = world.sandbox.source_file("assets/backdrop.png", &big);
    let figure = world
        .sandbox
        .source_file("assets/figure.png", &png(32, 32, 20));
    restate(&mut document, "backdrop", "assets/backdrop.png", &backdrop);
    restate(&mut document, "figure", "assets/figure.png", &figure);
    let path = world.sandbox.root.join("bundles/big.bundle.json");
    fs::write(&path, serde_json::to_vec_pretty(&document).unwrap()).unwrap();
    world
        .run(json!({"op":"scene.detach","scene":"street"}))
        .unwrap();
    world
        .run(json!({"op":"scene.attach","id":"street","path":path}))
        .unwrap();

    let job = interactive_adopt(&mut world);
    let registry = world.registry();
    wait_final(&registry, &job);
    world.assert_files_kept();

    let started = Instant::now();
    let answer = world.run(json!({"op":"job.discard","job":job})).unwrap();
    let caller = started.elapsed();
    let discard = answer["job"].as_str().unwrap().to_string();
    let mut polled = Duration::ZERO;
    let ended = loop {
        let tick = Instant::now();
        let events = world.host.as_mut().unwrap().poll(&mut world.state);
        polled = polled.max(tick.elapsed());
        if let Some(event) = events.into_iter().find(|event| event["job"] == discard) {
            break event;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let total = started.elapsed();
    println!(
        "discarding {} MiB: job.discard returned in {caller:?}, slowest poll {polled:?}, the discard job ended after {total:?} ({})",
        EXTRA >> 20,
        ended["result"]["cleanup"]
    );
    assert_eq!(ended["state"], "succeeded", "{ended}");
    assert_eq!(ended["result"]["cleanup"]["removedFiles"], 2, "{ended}");
    assert!(
        caller < total / 10 && polled < total / 10,
        "the caller must not pay for the removal: {caller:?} / {polled:?} of {total:?}"
    );
    world.assert_nothing_copied();
}
