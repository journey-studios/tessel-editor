//! BL100 and BL101 through the real `MediaHost`.
//!
//! BL100: `scene.attach.start` reads, parses and validates the bundle on a job thread and the
//! core's `scene.attach` attaches the admitted scene when the result is applied, as one undo
//! step, only while the project is still the one (ID and revision) the job was started for.
//! BL101: a scene whose manifest names no files resolves without a folder (or a workspace);
//! a scene that names files keeps the full preflight.
//!
//! Real host, real files, real jobs. Jobs are awaited on their end state (`job.wait`/`poll`),
//! never on a delay. Unproven here: the window that polls the host (see
//! `crates/desktop/src/workstation/scene_import_tests.rs` and `scripts/scene-import-proof.py`).

mod directed_resources_common;

use core::prelude::v1::test;

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use directed_resources_common::*;
use serde_json::{Value, json};
use tessel_editor_core::jobs::JobKind;
use tessel_editor_core::{CommandError, EditorState, headless, persist};
use tessel_editor_media::host::MediaHost;

struct World {
    sandbox: Sandbox,
    state: EditorState,
    host: MediaHost,
    workspace: PathBuf,
}

fn world(name: &str, host: MediaHost) -> World {
    let sandbox = Sandbox::new(&format!("te-attach-{name}"));
    let workspace = sandbox.root.join("ws");
    fs::create_dir_all(&workspace).unwrap();
    let mut state = EditorState::demo();
    headless::execute(&mut state, &json!({"op":"project.create","name":"attach"})).unwrap();
    persist::save(&mut state, &workspace.join("p.tessel.json")).unwrap();
    World {
        sandbox,
        state,
        host,
        workspace,
    }
}

impl World {
    fn run(&mut self, request: Value) -> Result<Value, CommandError> {
        headless::execute_ext(&mut self.state, None, Some(&mut self.host), &request)
    }

    fn ok(&mut self, request: Value) -> Value {
        self.run(request.clone())
            .unwrap_or_else(|error| panic!("{request}: {error:?}"))
    }

    fn write(&self, name: &str, bundle: &Value) -> PathBuf {
        let path = self.sandbox.root.join("bundles").join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, serde_json::to_vec_pretty(bundle).unwrap()).unwrap();
        path
    }

    /// The scene with two real PNGs under `sandbox.src`.
    fn with_files(&self) -> PathBuf {
        self.write("files.bundle.json", &standard_bundle(&self.sandbox))
    }

    fn data_only(&self) -> PathBuf {
        self.write("data.bundle.json", &data_only_bundle())
    }

    fn undo_depth(&mut self) -> u64 {
        self.ok(json!({"op":"history.status"}))["history"]["undo_depth"]
            .as_u64()
            .unwrap()
    }

    fn scene_ids(&self) -> Vec<String> {
        self.state
            .scenes
            .ids()
            .iter()
            .map(|id| id.to_string())
            .collect()
    }

    fn jobs(&self) -> usize {
        self.host.registry().list().len()
    }
}

/// A valid scene that needs no file: its assets are declared but nothing uses them (the format
/// requires at least one declared asset), so the manifest has nothing to transport.
fn data_only_bundle() -> Value {
    let mut document = fixture();
    document["sceneDefinitions"][0]["nodes"]
        .as_array_mut()
        .unwrap()
        .retain(|node| node["type"] == "group");
    document
}

fn poll_until(host: &mut MediaHost, state: &mut EditorState, job: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        for event in host.poll(state) {
            if event["job"] == job {
                return event;
            }
        }
        assert!(Instant::now() < deadline, "job {job} never finished");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn job_code(error: &CommandError) -> String {
    error
        .details
        .as_ref()
        .and_then(|details| details["job_code"].as_str())
        .unwrap_or(error.code)
        .to_string()
}

// ---- BL100: attach as a job ----

#[test]
fn the_attach_job_attaches_as_one_undo_step_and_says_what_loading_needs() {
    let mut world = world("headless", MediaHost::new());
    let path = world.with_files();
    let revision = world.state.project.revision;

    let answer =
        world.ok(json!({"op":"scene.attach.start","path":path,"id":"street","name":"Rua"}));
    assert!(answer["job"].is_string(), "{answer}");
    let applied = &answer["applied"];
    assert_eq!(applied["action"], "scene.attach", "{answer}");
    assert_eq!(applied["scene"], "street");
    assert_eq!(applied["name"], "Rua");
    assert_eq!(applied["changed"], true);
    // The job's own report, and what loading will ask of the project (from the core's manifest).
    assert_eq!(
        answer["resources"],
        json!({"files": 2, "needs_folder": true})
    );
    assert_eq!(world.scene_ids(), ["street"]);
    assert_eq!(world.undo_depth(), 1, "one undo entry");
    assert_eq!(world.state.project.revision, revision + 1);

    world.ok(json!({"op":"history.undo"}));
    assert!(world.scene_ids().is_empty(), "undo removes the attach");
}

#[test]
fn the_job_attaches_the_same_scene_as_the_synchronous_command() {
    let mut job = world("parity-job", MediaHost::new());
    let path = job.with_files();
    let by_job = job.ok(json!({"op":"scene.attach.start","path":path,"id":"street"}));
    let mut sync = world("parity-sync", MediaHost::new());
    let path = sync.with_files();
    let direct = sync.ok(json!({"op":"scene.attach","path":path,"id":"street"}));

    let mut applied = by_job["applied"].clone();
    applied.as_object_mut().unwrap().remove("resources");
    assert_eq!(applied, direct, "the same attach answer");
    assert_eq!(
        job.state.scenes.get("street").unwrap().scene,
        sync.state.scenes.get("street").unwrap().scene,
        "the same scene document"
    );
}

#[test]
fn inline_and_text_sources_are_read_like_scene_attach() {
    let mut world = world("sources", MediaHost::new());
    let bundle = data_only_bundle();
    world.ok(json!({"op":"scene.attach.start","bundle":bundle,"id":"inline"}));
    world.ok(json!({"op":"scene.attach.start","text":bundle.to_string(),"id":"text"}));
    let neither = world
        .run(json!({"op":"scene.attach.start","id":"x"}))
        .unwrap_err();
    assert_eq!(neither.code, "invalid_request", "{neither:?}");
    assert_eq!(world.scene_ids(), ["inline", "text"]);
}

#[test]
fn every_refusal_is_typed_and_leaves_project_history_and_jobs_alone() {
    let mut world = world("refusals", MediaHost::new());
    let good = world.data_only();
    world.ok(json!({"op":"scene.attach.start","path":good,"id":"taken"}));
    let (revision, depth, jobs) = (
        world.state.project.revision,
        world.undo_depth(),
        world.jobs(),
    );

    let wrong = world.sandbox.root.join("wrong.json");
    fs::write(&wrong, r#"{"bundleVersion": 99}"#).unwrap();
    let not_json = world.sandbox.root.join("broken.json");
    fs::write(&not_json, "{ not json").unwrap();
    let huge = world.sandbox.root.join("huge.json");
    fs::File::create(&huge)
        .unwrap()
        .set_len(33 * 1024 * 1024)
        .unwrap();
    let missing = world.sandbox.root.join("nowhere.json");

    // Refused by the job: the same codes `scene.attach` answers.
    for (path, code) in [
        (wrong, "invalid_bundle"),
        (not_json, "invalid_json"),
        (huge, "bundle_too_large"),
        (missing, "bundle_unreadable"),
    ] {
        let error = world
            .run(json!({"op":"scene.attach.start","path":path,"id":"fresh"}))
            .unwrap_err();
        assert_eq!(error.code, code, "{}: {error:?}", path.display());
    }
    // Refused before any job exists: ID, room and name, as the command would.
    for (request, code) in [
        (
            json!({"op":"scene.attach.start","path":good,"id":"taken"}),
            "duplicate_id",
        ),
        (
            json!({"op":"scene.attach.start","path":good,"id":"bad id!"}),
            "invalid_id",
        ),
        (
            json!({"op":"scene.attach.start","path":good,"id":"fresh","name":"  "}),
            "invalid_value",
        ),
    ] {
        let error = world.run(request.clone()).unwrap_err();
        assert_eq!(error.code, code, "{request}: {error:?}");
    }

    assert_eq!(world.scene_ids(), ["taken"]);
    assert_eq!(world.state.project.revision, revision);
    assert_eq!(world.undo_depth(), depth);
    assert_eq!(
        world.jobs(),
        jobs + 4,
        "only the four reads became jobs; the early refusals started none"
    );
}

#[test]
fn an_interactive_host_answers_with_a_job_and_the_project_changes_only_when_polled() {
    let mut world = world("interactive", MediaHost::new().interactive());
    let path = world.with_files();
    let revision = world.state.project.revision;

    let answer = world.ok(json!({"op":"scene.attach.start","path":path,"id":"street"}));
    let job = answer["job"].as_str().unwrap().to_string();
    assert_eq!(answer["state"], "queued", "{answer}");
    assert!(answer.get("applied").is_none(), "nothing applied yet");
    assert_eq!(world.state.project.revision, revision);
    assert!(world.scene_ids().is_empty());

    let event = poll_until(&mut world.host, &mut world.state, &job);
    assert_eq!(event["state"], "succeeded", "{event}");
    assert_eq!(event["applied"]["scene"], "street");
    assert_eq!(event["applied"]["resources"]["needs_folder"], true);
    assert_eq!(world.scene_ids(), ["street"]);
    assert_eq!(world.undo_depth(), 1);
    assert!(
        world.host.poll(&mut world.state).is_empty(),
        "reported once"
    );
}

#[test]
fn a_failed_read_is_reported_once_with_its_typed_code_and_changes_nothing() {
    let mut world = world("failed-event", MediaHost::new().interactive());
    let missing = world.sandbox.root.join("nowhere.json");
    let revision = world.state.project.revision;
    let answer = world.ok(json!({"op":"scene.attach.start","path":missing}));
    let job = answer["job"].as_str().unwrap().to_string();
    let event = poll_until(&mut world.host, &mut world.state, &job);
    assert_eq!(event["state"], "failed", "{event}");
    assert_eq!(event["error"]["code"], "bundle_unreadable", "{event}");
    assert_eq!(world.state.project.revision, revision);
    assert!(world.scene_ids().is_empty());
}

#[test]
fn a_result_for_a_project_that_was_edited_meanwhile_is_not_applied() {
    let mut world = world("stale-edit", MediaHost::new());
    let path = world.data_only();
    let started = world.ok(json!({"op":"scene.attach.start","path":path,"id":"late","wait":false}));
    let job = started["job"].as_str().unwrap().to_string();
    world.ok(json!({"op":"job.wait","job":job}));
    // The project moves on before the result is applied.
    world.ok(json!({"op":"scene.attach","path":path,"id":"first"}));
    let (revision, depth) = (world.state.project.revision, world.undo_depth());

    let error = world.run(json!({"op":"job.commit","job":job})).unwrap_err();
    assert_eq!(error.code, "stale_result", "{error:?}");
    assert_eq!(
        world.scene_ids(),
        ["first"],
        "the late candidate was dropped"
    );
    assert_eq!(world.state.project.revision, revision);
    assert_eq!(world.undo_depth(), depth);
}

#[test]
fn a_result_for_another_project_is_not_applied() {
    let mut world = world("stale-project", MediaHost::new().interactive());
    let path = world.data_only();
    let answer = world.ok(json!({"op":"scene.attach.start","path":path,"id":"street"}));
    let job = answer["job"].as_str().unwrap().to_string();
    // A different project is open by the time the tick applies it.
    world.ok(json!({"op":"project.create","name":"other"}));
    let (project, revision) = (world.state.project.id.clone(), world.state.project.revision);

    let event = poll_until(&mut world.host, &mut world.state, &job);
    assert_eq!(event["state"], "failed", "{event}");
    assert_eq!(event["error"]["code"], "stale_result", "{event}");
    assert!(world.scene_ids().is_empty());
    assert_eq!(
        (world.state.project.id.clone(), world.state.project.revision),
        (project, revision)
    );
}

// ---- BL113a: the guard knows the editing session, not only (ID, revision) ----

impl World {
    fn document(&self) -> PathBuf {
        self.workspace.join("p.tessel.json")
    }

    /// `(id, revision)`: what the guard used to compare, and what a reopen leaves unchanged.
    fn identity(&self) -> (String, u64) {
        (self.state.project.id.clone(), self.state.project.revision)
    }

    fn session(&self) -> u64 {
        self.state.project.session.number()
    }

    /// Fills both job slots so the next job waits queued: the test then decides, without any
    /// delay, what happens to the project before that job starts reading. Release with the flag.
    fn hold_the_slots(&self) -> Arc<AtomicBool> {
        let release = Arc::new(AtomicBool::new(false));
        for _ in 0..2 {
            let release = release.clone();
            self.host
                .registry()
                .spawn(JobKind::Hash, "blocker", move |ctx| {
                    while !release.load(Ordering::SeqCst) {
                        ctx.check()?;
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Ok(Value::Null)
                });
        }
        release
    }

    fn assert_untouched(&mut self, scenes: &[&str], identity: &(String, u64), depth: u64) {
        assert_eq!(self.scene_ids(), scenes);
        assert_eq!(&self.identity(), identity);
        assert_eq!(self.undo_depth(), depth, "no undo entry was recorded");
    }
}

#[test]
fn a_result_started_before_the_document_was_reopened_is_stale() {
    let mut world = world("reopen-headless", MediaHost::new());
    let path = world.data_only();
    let started = world.ok(json!({"op":"scene.attach.start","path":path,"id":"late","wait":false}));
    let job = started["job"].as_str().unwrap().to_string();
    // The job is finished (a barrier, not a delay) and its result is held, not applied.
    world.ok(json!({"op":"job.wait","job":job}));
    let (before_identity, before_session) = (world.identity(), world.session());

    // The project is closed and its own document opened again: same ID, revision back to 0.
    world.ok(json!({"op":"project.open","path":world.document()}));
    assert_eq!(
        world.identity(),
        before_identity,
        "the reopened project has the ID and revision the job saw: only the session tells them apart"
    );
    assert_ne!(world.session(), before_session);
    let identity = world.identity();

    let error = world.run(json!({"op":"job.commit","job":job})).unwrap_err();
    assert_eq!(error.code, "stale_result", "{error:?}");
    world.assert_untouched(&[], &identity, 0);
    let undo = world.run(json!({"op":"history.undo"})).unwrap_err();
    assert_eq!(
        undo.code, "nothing_to_undo",
        "nothing was recorded: {undo:?}"
    );
}

#[test]
fn an_interactive_poll_refuses_a_job_that_read_after_the_document_was_reopened() {
    let mut world = world("reopen-interactive", MediaHost::new().interactive());
    let path = world.data_only();
    let release = world.hold_the_slots();
    let answer = world.ok(json!({"op":"scene.attach.start","path":path,"id":"late"}));
    let job = answer["job"].as_str().unwrap().to_string();
    assert_eq!(answer["state"], "queued", "{answer}");
    let before = world.identity();

    world.ok(json!({"op":"project.open","path":world.document()}));
    assert_eq!(world.identity(), before);
    release.store(true, Ordering::SeqCst);

    let event = poll_until(&mut world.host, &mut world.state, &job);
    assert_eq!(event["state"], "failed", "{event}");
    assert_eq!(event["error"]["code"], "stale_result", "{event}");
    assert!(
        world.host.poll(&mut world.state).is_empty(),
        "reported once"
    );
    world.assert_untouched(&[], &before, 0);
}

#[test]
fn a_project_with_the_same_id_and_revision_from_another_session_does_not_receive_the_result() {
    let mut world = world("other-session", MediaHost::new());
    let path = world.data_only();
    let started = world.ok(json!({"op":"scene.attach.start","path":path,"id":"late","wait":false}));
    let job = started["job"].as_str().unwrap().to_string();
    world.ok(json!({"op":"job.wait","job":job}));

    // Another session's copy of the same document takes the project's place without going
    // through `project.open` (what a window does when it swaps its state).
    let other = persist::load(&world.document()).unwrap();
    assert_eq!(
        (other.project.id.clone(), other.project.revision),
        world.identity()
    );
    world.state = other;
    let identity = world.identity();

    let error = world.run(json!({"op":"job.commit","job":job})).unwrap_err();
    assert_eq!(error.code, "stale_result", "{error:?}");
    world.assert_untouched(&[], &identity, 0);
}

#[test]
fn the_normal_flow_survives_saving_queries_and_undo_cycles_without_reopening() {
    let mut world = world("same-session", MediaHost::new());
    let path = world.data_only();
    // History moved before the job started; the session is the same one throughout.
    world.ok(json!({"op":"scene.attach","path":path,"id":"first"}));
    world.ok(json!({"op":"history.undo"}));
    world.ok(json!({"op":"history.redo"}));
    let session = world.session();

    let started =
        world.ok(json!({"op":"scene.attach.start","path":path,"id":"second","wait":false}));
    let job = started["job"].as_str().unwrap().to_string();
    world.ok(json!({"op":"job.wait","job":job}));
    // Saving is not an edit; neither is anything that only reads, nor the clones the host and the
    // package plan take of the project.
    world.ok(json!({"op":"project.save","path":world.document()}));
    world.ok(json!({"op":"scene.list"}));
    world.ok(json!({"op":"scene.resources.status"}));
    world.ok(json!({"op":"history.status"}));
    let pack = world.sandbox.root.join("out/pack.tesselpack");
    world.ok(json!({"op":"project.package_preflight","path":pack}));
    assert_eq!(world.session(), session);

    let committed = world.ok(json!({"op":"job.commit","job":job}));
    assert_eq!(committed["applied"]["scene"], "second", "{committed}");
    assert_eq!(world.scene_ids(), ["first", "second"]);
    assert_eq!(world.undo_depth(), 2, "one undo entry for the job");
}

#[test]
fn an_interactive_attach_applies_across_a_save_in_the_same_session() {
    let mut world = world("save-interactive", MediaHost::new().interactive());
    let path = world.data_only();
    let release = world.hold_the_slots();
    let answer = world.ok(json!({"op":"scene.attach.start","path":path,"id":"street"}));
    let job = answer["job"].as_str().unwrap().to_string();
    world.ok(json!({"op":"project.save","path":world.document()}));
    release.store(true, Ordering::SeqCst);
    let event = poll_until(&mut world.host, &mut world.state, &job);
    assert_eq!(event["state"], "succeeded", "{event}");
    assert_eq!(world.scene_ids(), ["street"]);
    assert_eq!(world.undo_depth(), 1);
}

#[test]
fn a_cancelled_attach_job_attaches_nothing() {
    let mut world = world("cancel", MediaHost::new());
    // Both job slots busy: the attach waits its turn, queued, so the cancel is deterministic.
    let release = Arc::new(AtomicBool::new(false));
    for _ in 0..2 {
        let release = release.clone();
        world
            .host
            .registry()
            .spawn(JobKind::Hash, "blocker", move |ctx| {
                while !release.load(Ordering::SeqCst) {
                    ctx.check()?;
                    std::thread::sleep(Duration::from_millis(5));
                }
                Ok(Value::Null)
            });
    }
    let path = world.data_only();
    let revision = world.state.project.revision;
    let started =
        world.ok(json!({"op":"scene.attach.start","path":path,"id":"street","wait":false}));
    let job = started["job"].as_str().unwrap().to_string();
    world.ok(json!({"op":"job.cancel","job":job}));
    let waited = world.ok(json!({"op":"job.wait","job":job}));
    release.store(true, Ordering::SeqCst);
    assert_eq!(waited["state"], "cancelled", "{waited}");

    let error = world.run(json!({"op":"job.commit","job":job})).unwrap_err();
    assert_eq!(error.code, "cancelled", "{error:?}");
    assert!(world.scene_ids().is_empty());
    assert_eq!(world.state.project.revision, revision);
}

#[test]
fn the_job_never_blocks_on_a_large_bundle_in_the_callers_thread() {
    // An interactive host (the window's) answers before the file is even read: the call returns
    // while the job is queued or running, and only `poll` ever touches the project.
    let mut world = world("large", MediaHost::new().interactive());
    let mut bundle = data_only_bundle();
    // Valid padding the importer must still parse: a big, ignored extension section.
    bundle["x-padding"] = json!("0".repeat(8 * 1024 * 1024));
    let path = world.write("large.bundle.json", &bundle);
    let started = Instant::now();
    let answer = world.ok(json!({"op":"scene.attach.start","path":path,"id":"big"}));
    let returned = started.elapsed();
    let job = answer["job"].as_str().unwrap().to_string();
    assert!(
        world.scene_ids().is_empty(),
        "nothing is attached when the call returns: {answer}"
    );
    let event = poll_until(&mut world.host, &mut world.state, &job);
    assert_eq!(event["state"], "succeeded", "{event}");
    assert_eq!(world.scene_ids(), ["big"]);
    assert!(
        returned < Duration::from_millis(500),
        "the caller waited {returned:?}"
    );
}

// ---- BL101: a scene with no files needs no folder ----

#[test]
fn a_scene_with_no_files_resolves_without_a_folder() {
    let mut world = world("empty-resolve", MediaHost::new());
    let path = world.data_only();
    let attached = world.ok(json!({"op":"scene.attach.start","path":path,"id":"data"}));
    assert_eq!(
        attached["resources"],
        json!({"files": 0, "needs_folder": false})
    );
    assert!(
        !world.workspace.join("scenes").exists(),
        "there is no resource folder, and none is needed"
    );

    let preflight = world.ok(json!({"op":"scene.resources.preflight","scene":"data"}));
    assert_eq!(preflight["report"]["ok"], true, "{preflight}");
    let resolved = world.ok(json!({"op":"scene.resources.resolve","scene":"data"}));
    assert_eq!(resolved["applied"]["installed"], true, "{resolved}");
    assert_eq!(resolved["applied"]["resources"], 0);
    assert!(world.host.scene_resources().get("data").is_some());
    assert!(
        !world.workspace.join("scenes").exists(),
        "loading created no folder either"
    );
    let status = world.ok(json!({"op":"scene.resources.status"}));
    assert_eq!(status["attached"][0]["loaded"], true, "{status}");
    assert_eq!(status["attached"][0]["current"], true);
}

#[test]
fn a_scene_with_no_files_resolves_in_a_project_that_was_never_saved() {
    let mut host = MediaHost::new();
    let mut state = EditorState::demo();
    headless::execute(&mut state, &json!({"op":"project.create","name":"unsaved"})).unwrap();
    assert!(state.project.workspace.is_none());
    let bundle = data_only_bundle();
    let run = |state: &mut EditorState, host: &mut MediaHost, request: Value| {
        headless::execute_ext(state, None, Some(host), &request)
    };
    run(
        &mut state,
        &mut host,
        json!({"op":"scene.attach.start","bundle":bundle,"id":"data"}),
    )
    .unwrap();
    let resolved = run(
        &mut state,
        &mut host,
        json!({"op":"scene.resources.resolve","scene":"data"}),
    )
    .unwrap();
    assert_eq!(resolved["applied"]["installed"], true, "{resolved}");
}

#[test]
fn a_scene_that_names_files_still_needs_a_folder_and_the_full_preflight() {
    let mut world = world("files-keep-preflight", MediaHost::new());
    let path = world.with_files();
    world.ok(json!({"op":"scene.attach.start","path":path,"id":"street"}));

    // No managed folder yet: the load says so instead of approving an empty map.
    let error = world
        .run(json!({"op":"scene.resources.resolve","scene":"street"}))
        .unwrap_err();
    assert_eq!(job_code(&error), "root_unavailable", "{error:?}");
    assert!(world.host.scene_resources().get("street").is_none());

    // A folder with the files missing: every problem is listed, nothing is approved.
    fs::create_dir_all(world.workspace.join("scenes/street")).unwrap();
    let error = world
        .run(json!({"op":"scene.resources.resolve","scene":"street"}))
        .unwrap_err();
    assert_eq!(job_code(&error), "resources_invalid", "{error:?}");
    let problems = error.details.as_ref().unwrap()["problems"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(problems, 2, "{error:?}");
    assert!(world.host.scene_resources().get("street").is_none());

    // The authored folder, adopted, makes the same load succeed.
    let source = world.sandbox.src.clone();
    world.ok(json!({"op":"scene.resources.adopt","scene":"street","source":source}));
    let resolved = world.ok(json!({"op":"scene.resources.resolve","scene":"street"}));
    assert_eq!(resolved["applied"]["installed"], true, "{resolved}");
    assert_eq!(resolved["applied"]["resources"], 2);
}

#[test]
fn without_a_workspace_a_scene_with_files_keeps_the_no_workspace_refusal() {
    let mut host = MediaHost::new();
    let mut state = EditorState::demo();
    headless::execute(&mut state, &json!({"op":"project.create","name":"unsaved"})).unwrap();
    let sandbox = Sandbox::new("te-attach-unsaved-files");
    let bundle = standard_bundle(&sandbox);
    headless::execute_ext(
        &mut state,
        None,
        Some(&mut host),
        &json!({"op":"scene.attach.start","bundle":bundle,"id":"street"}),
    )
    .unwrap();
    let error = headless::execute_ext(
        &mut state,
        None,
        Some(&mut host),
        &json!({"op":"scene.resources.resolve","scene":"street"}),
    )
    .unwrap_err();
    assert_eq!(error.code, "no_workspace", "{error:?}");
}

#[test]
fn a_relative_explicit_base_is_still_refused_for_a_scene_with_no_files() {
    // Only a missing workspace is waived for a data-only scene; a caller's bad base is a bug.
    let mut world = world("empty-bad-base", MediaHost::new());
    let path = world.data_only();
    world.ok(json!({"op":"scene.attach.start","path":path,"id":"data"}));
    let error = world
        .run(json!({"op":"scene.resources.resolve","scene":"data","base":"relative/dir"}))
        .unwrap_err();
    assert_eq!(job_code(&error), "base_not_absolute", "{error:?}");
}

#[test]
fn a_finished_attach_can_be_discarded_without_attaching_or_removing_its_bundle() {
    let mut world = world("discard-attach", MediaHost::new());
    let path = world.data_only();
    let revision = world.state.project.revision;
    let depth = world.undo_depth();
    let started = world.ok(json!({"op":"scene.attach.start","path":path,"id":"late","wait":false}));
    let job = started["job"].as_str().unwrap().to_string();
    world.ok(json!({"op":"job.wait","job":job}));
    let discarded = world.ok(json!({"op":"job.discard","job":job}));
    assert_eq!(discarded["discarded"], "scene-attach");
    assert!(path.is_file());
    assert!(world.state.scenes.is_empty());
    assert_eq!(world.state.project.revision, revision);
    assert_eq!(world.undo_depth(), depth);
    assert_eq!(
        world
            .run(json!({"op":"job.commit","job":job}))
            .unwrap_err()
            .code,
        "job_discarded"
    );
}
