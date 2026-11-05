//! BL089 / BL096 through the real `MediaHost` (F040): the result of `scene.resources.adopt` is a
//! pending adoption. `job.commit` (or `poll`, or `wait:true`) keeps its files only while the
//! project, its workspace and the scene's bundle are the ones the job was started for, and
//! removes them otherwise; a host that goes away with an uncommitted result removes them too,
//! except what another result relies on.
//!
//! Every case uses the real host and `adopt` with synthetic PNGs; jobs are awaited with
//! `job.wait`/`poll` on the job's end state, never on a delay. Unproven: a window, Windows, a
//! second process, a hostile writer.

mod directed_resources_common;

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use directed_resources_common::*;
use serde_json::{Value, json};
use tessel_editor_core::{CommandError, EditorState, headless, persist};
use tessel_editor_media::host::MediaHost;
use tessel_editor_media::scene_resources::tracked_under;

struct World {
    sandbox: Sandbox,
    state: EditorState,
    host: Option<MediaHost>,
    workspace: PathBuf,
}

fn world(name: &str) -> World {
    world_with(name, MediaHost::new())
}

fn world_with(name: &str, host: MediaHost) -> World {
    let sandbox = Sandbox::new(&format!("te-host-{name}"));
    let workspace = sandbox.root.join("ws");
    fs::create_dir_all(&workspace).unwrap();
    let mut state = EditorState::demo();
    headless::execute(&mut state, &json!({"op":"project.create","name":"adopt"})).unwrap();
    persist::save(&mut state, &workspace.join("p.tessel.json")).unwrap();
    let bundle = standard_bundle(&sandbox);
    let mut world = World {
        sandbox,
        state,
        host: Some(host),
        workspace,
    };
    world.attach(&bundle);
    // Saved with the scene: a copy of this folder opened later is the same project and scene.
    let path = world.workspace.join("p.tessel.json");
    persist::save(&mut world.state, &path).unwrap();
    world
}

impl World {
    fn run(&mut self, request: Value) -> Result<Value, CommandError> {
        let host = self.host.as_mut().expect("the host is still alive");
        headless::execute_ext(&mut self.state, None, Some(host), &request)
    }

    fn attach(&mut self, bundle: &Value) {
        let path = self.sandbox.root.join("bundles/street.bundle.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, serde_json::to_vec_pretty(bundle).unwrap()).unwrap();
        self.run(json!({"op":"scene.attach","id":"street","path":path}))
            .unwrap();
    }

    /// Starts `scene.resources.adopt` as a job and waits for it to finish (not to be applied).
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

    fn commit(&mut self, job: &str) -> Result<Value, CommandError> {
        self.run(json!({"op":"job.commit","job":job}))
    }

    fn backdrop(&self) -> PathBuf {
        self.workspace.join("scenes/street/assets/backdrop.png")
    }

    fn figure(&self) -> PathBuf {
        self.workspace.join("scenes/street/assets/figure.png")
    }

    fn assert_files_kept(&self) {
        for path in [self.backdrop(), self.figure()] {
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

#[test]
fn an_adopt_that_applies_at_once_keeps_its_files_and_answers_as_before() {
    let mut world = world("sync");
    let source = world.sandbox.src.clone();
    let result = world
        .run(json!({"op":"scene.resources.adopt","scene":"street","source":source}))
        .unwrap();
    assert_eq!(result["action"], "scene.resources.adopt", "{result}");
    assert_eq!(result["scene"], "street");
    assert!(result["base"].is_object(), "{result}");
    assert_eq!(result["report"]["items"].as_array().unwrap().len(), 2);
    assert!(
        result.get("applied").is_none(),
        "an adoption changes no project data: {result}"
    );
    world.assert_files_kept();
    assert_eq!(
        tracked_under(&world.workspace),
        0,
        "accepted: nobody holds it"
    );
}

#[test]
fn a_job_committed_in_the_same_project_keeps_its_files() {
    let mut world = world("commit");
    let job = world.adopt_job();
    world.assert_files_kept();
    let committed = world.commit(&job).unwrap();
    assert_eq!(committed["action"], "scene.resources.adopt", "{committed}");
    world.assert_files_kept();

    // Accepted for good: the host going away takes nothing back.
    drop(world.host.take());
    world.assert_files_kept();
    assert_eq!(tracked_under(&world.workspace), 0);
}

#[test]
fn a_commit_after_a_project_switch_is_stale_and_takes_the_copied_files_back() {
    let mut world = world("switch");
    let job = world.adopt_job();
    world
        .run(json!({"op":"project.create","name":"next"}))
        .unwrap();

    let error = world.commit(&job).unwrap_err();
    assert_eq!(error.code, "stale_result", "{error:?}");
    assert!(error.message.contains("another project"), "{error:?}");
    world.assert_nothing_copied();
}

#[test]
fn a_commit_after_the_same_project_was_opened_from_a_copy_is_stale() {
    let mut world = world("copy");
    let job = world.adopt_job();
    let copy = world.sandbox.root.join("copy");
    fs::create_dir_all(&copy).unwrap();
    fs::copy(
        world.workspace.join("p.tessel.json"),
        copy.join("p.tessel.json"),
    )
    .unwrap();
    world
        .run(json!({"op":"project.open","path":copy.join("p.tessel.json")}))
        .unwrap();
    assert_eq!(
        world.state.project.workspace.as_deref(),
        Some(copy.as_path())
    );

    let error = world.commit(&job).unwrap_err();
    assert_eq!(error.code, "stale_result", "{error:?}");
    assert!(error.message.contains("another workspace"), "{error:?}");
    world.assert_nothing_copied();
    assert!(
        !copy.join("scenes").exists(),
        "nothing was copied for the other workspace either"
    );
}

#[test]
fn a_commit_after_the_scene_was_detached_is_stale() {
    let mut world = world("detach");
    let job = world.adopt_job();
    world
        .run(json!({"op":"scene.detach","scene":"street"}))
        .unwrap();

    let error = world.commit(&job).unwrap_err();
    assert_eq!(error.code, "stale_result", "{error:?}");
    assert!(error.message.contains("scene"), "{error:?}");
    world.assert_nothing_copied();
}

#[test]
fn a_commit_after_the_scenes_bundle_was_replaced_is_stale() {
    let mut world = world("revision");
    let job = world.adopt_job();
    world
        .run(json!({"op":"scene.detach","scene":"street"}))
        .unwrap();
    let mut other = standard_bundle(&world.sandbox);
    other["revisionId"] = json!("rev-2");
    world.attach(&other);

    let error = world.commit(&job).unwrap_err();
    assert_eq!(error.code, "stale_result", "{error:?}");
    world.assert_nothing_copied();
}

#[test]
fn a_host_dropped_with_an_uncommitted_adopt_takes_the_copied_files_back() {
    let mut world = world("dropped");
    world.adopt_job();
    world.assert_files_kept();

    drop(world.host.take());
    world.assert_nothing_copied();
}

#[test]
fn an_adopt_job_that_ran_into_an_existing_folder_leaves_what_was_there() {
    let mut world = world("existing");
    let root = world.workspace.join("scenes/street");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("notes.txt"), b"mine").unwrap();
    let before = tree(&world.workspace);
    world.adopt_job();

    drop(world.host.take());
    assert_eq!(tree(&world.workspace), before);
    assert_eq!(fs::read(root.join("notes.txt")).unwrap(), b"mine");
}

// ------------------------------------------------------------------------------- BL096 --

#[test]
fn two_uncommitted_jobs_for_one_scene_keep_the_files_while_one_of_them_is_committed() {
    for committed_first in [true, false] {
        let mut world = world(&format!("two-jobs-{committed_first}"));
        let first = world.adopt_job();
        let second = world.adopt_job();

        // The job that is not committed is dropped with the host; the other one was accepted.
        let (accepted, _left) = if committed_first {
            (first, second)
        } else {
            (second, first)
        };
        world.commit(&accepted).unwrap();
        world.assert_files_kept();
        drop(world.host.take());
        world.assert_files_kept();
        assert_eq!(tracked_under(&world.workspace), 0);
    }
}

#[test]
fn two_uncommitted_jobs_for_one_scene_take_everything_back_together() {
    let mut world = world("two-jobs-dropped");
    world.adopt_job();
    world.adopt_job();
    world.assert_files_kept();

    drop(world.host.take());
    world.assert_nothing_copied();
}

#[test]
fn a_stale_job_does_not_remove_what_a_committed_one_reused() {
    let mut world = world("stale-and-committed");
    let first = world.adopt_job();
    let second = world.adopt_job();

    // The first job finishes against a scene that changed: it is discarded. The second one
    // reused its files and is committed afterwards, against the scene as it is now.
    world
        .run(json!({"op":"scene.detach","scene":"street"}))
        .unwrap();
    let error = world.commit(&first).unwrap_err();
    assert_eq!(error.code, "stale_result", "{error:?}");
    assert!(
        error.message.contains("0 file(s)"),
        "it removed none: the second job relies on them: {error:?}"
    );
    world.assert_files_kept();

    // Re-attached as it was: the second job's target is the same revision again.
    let bundle = standard_bundle(&world.sandbox);
    world.attach(&bundle);
    world.commit(&second).unwrap();
    world.assert_files_kept();
    drop(world.host.take());
    world.assert_files_kept();
}

// ------------------------------------------------------------------------------- poll --

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

#[test]
fn a_window_applies_a_finished_adopt_on_its_next_poll() {
    let mut world = world_with("poll-ok", MediaHost::new().interactive());
    let job = interactive_adopt(&mut world);
    let event = poll_until_reported(&mut world, &job);
    assert_eq!(event["state"], "succeeded", "{event}");
    world.assert_files_kept();
    drop(world.host.take());
    world.assert_files_kept();
}

#[test]
fn a_window_reports_an_adopt_the_project_outgrew_as_failed_and_removes_its_files() {
    let mut world = world_with("poll-stale", MediaHost::new().interactive());
    let job = interactive_adopt(&mut world);
    // The project is replaced before the window's next tick sees the finished job.
    world
        .run(json!({"op":"project.create","name":"next"}))
        .unwrap();

    let event = poll_until_reported(&mut world, &job);
    assert_eq!(event["state"], "failed", "{event}");
    assert_eq!(event["error"]["code"], "stale_result", "{event}");
    // The window's tick does not delete: a discard job (BL094/BL112) takes the files back and the
    // window hears about it with a later event.
    let discard = event["error"]["details"]["discard_job"]
        .as_str()
        .unwrap_or_else(|| panic!("the refusal names its discard job: {event}"))
        .to_string();
    let ended = poll_until_reported(&mut world, &discard);
    assert_eq!(ended["kind"], "discard", "{ended}");
    assert_eq!(ended["state"], "succeeded", "{ended}");
    world.assert_nothing_copied();
}
