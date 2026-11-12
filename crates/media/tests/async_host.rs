//! The interactive host (what the desktop runs) never blocks its caller: media work is a job,
//! `poll` applies each finished job as one transaction, and a result that no longer fits the
//! project is discarded. Proved against deliberately slow and failing tool wrappers, using only
//! the semantic operations a window, a CLI or a library caller would send.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tessel_editor_core::jobs::ResultState;
use tessel_editor_core::{CommandError, EditorState, headless};
use tessel_editor_media::host::{MediaHost, RendererFactory};
use tessel_editor_media::tools::Tools;

use common::mock::{Mock, Settings};
use common::{temp, tools, video_with_audio};

/// ffmpeg/ffprobe that wait `delay` seconds, fail while `flag` exists, and otherwise are the real ones.
fn wrapped_tools(dir: &Path, delay: f64, flag: &Path) -> Tools {
    let real = tools();
    let wrap = |name: &str, target: &Path| -> PathBuf {
        let path = dir.join(name);
        fs::write(
            &path,
            format!(
                "#!/bin/sh\nsleep {delay}\nif [ -e '{}' ]; then echo 'forced failure' >&2; exit 1; fi\nexec '{}' \"$@\"\n",
                flag.display(),
                target.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    };
    Tools {
        ffmpeg: wrap("slow-ffmpeg", &real.ffmpeg),
        ffprobe: wrap("slow-ffprobe", &real.ffprobe),
        ..real
    }
}

fn interactive(tools: Tools) -> MediaHost {
    let factory: Arc<RendererFactory> = Arc::new(|| Ok(Box::new(Mock::new(Settings::default()))));
    MediaHost::new()
        .with_renderer(factory)
        .with_tools(tools)
        .interactive()
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

fn new_project(host: &mut MediaHost, state: &mut EditorState, dir: &Path, name: &str) {
    ok(
        host,
        state,
        json!({"op":"project.create","name":name,"sequence":"main","rate":24,
            "width":320,"height":180,"length":48,"dir":dir.join(name)}),
    );
}

/// What the window's tick does: poll until `job` reports, keeping every event seen.
fn poll_until(host: &mut MediaHost, state: &mut EditorState, job: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        for event in host.poll(state) {
            if event["job"] == job {
                return event;
            }
        }
        assert!(Instant::now() < deadline, "job {job} never reported");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn poll_for(
    host: &mut MediaHost,
    state: &mut EditorState,
    mut done: impl FnMut(&EditorState) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while !done(state) {
        host.poll(state);
        assert!(Instant::now() < deadline, "the condition was never met");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn files_under(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

fn jobs_of_kind(host: &MediaHost, kind: &str) -> usize {
    host.registry().to_json()["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|job| job["kind"] == kind)
        .count()
}

/// The ids of the jobs of `kind`, in the order they were started.
fn job_ids_of_kind(host: &MediaHost, kind: &str) -> Vec<String> {
    host.registry().to_json()["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|job| job["kind"] == kind)
        .map(|job| job["id"].as_str().unwrap().to_string())
        .collect()
}

fn decoded(state: &EditorState) -> bool {
    state
        .library
        .items
        .iter()
        .filter_map(|item| item.asset.as_ref())
        .all(|asset| asset.decoded.is_some())
        && !state.library.items.is_empty()
}

#[test]
fn a_slow_import_never_blocks_the_caller_and_is_applied_once_on_poll() {
    let dir = temp("async-import");
    let video = video_with_audio(&tools(), &dir, "clip.mp4");
    let mut host = interactive(wrapped_tools(&dir, 0.6, &dir.join("no-such-flag")));
    let mut state = EditorState::demo();
    new_project(&mut host, &mut state, &dir, "slow");

    let started = Instant::now();
    let answer = ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[video],"wait":true}),
    );
    assert!(
        started.elapsed() < Duration::from_millis(400),
        "the call returned a job, not the result, after {:?}",
        started.elapsed()
    );
    let job = answer["job"].as_str().expect("a job id").to_string();
    assert!(
        state.library.items.is_empty(),
        "nothing applies before poll"
    );

    // Meanwhile the caller keeps editing the very same state.
    let revision = state.project.revision;
    ok(
        &mut host,
        &mut state,
        json!({"op":"track.add","id":"during","name":"During","kind":"video"}),
    );
    assert!(state.project.revision > revision);
    // In flight, not necessarily started: the worker thread may not have been scheduled yet when
    // the machine is busy (a `queued` job is just as unfinished as a `running` one).
    assert!(
        !host.registry().get(&job).unwrap().state.is_final(),
        "the 0.6 s import cannot be over while the caller was only editing"
    );

    let event = poll_until(&mut host, &mut state, &job);
    assert_eq!(event["state"], "succeeded", "{event}");
    assert_eq!(state.library.items.len(), 1);
    assert!(
        state
            .timeline
            .tracks
            .iter()
            .any(|track| track.id == "during"),
        "the edit made while the job ran is not lost to a divergent clone"
    );

    // No manual `media.prepare`: the host schedules it and applies decoded media on a later poll.
    poll_for(&mut host, &mut state, decoded);
    assert_eq!(jobs_of_kind(&host, "proxy"), 1, "prepared once, in one job");
}

#[test]
fn cancelling_a_running_job_changes_nothing_and_leaves_no_files() {
    let dir = temp("async-cancel");
    let video = video_with_audio(&tools(), &dir, "clip.mp4");
    let mut host = interactive(wrapped_tools(&dir, 0.8, &dir.join("no-such-flag")));
    let mut state = EditorState::demo();
    new_project(&mut host, &mut state, &dir, "cancelled");
    let before = files_under(&dir.join("cancelled"));
    let revision = state.project.revision;

    let answer = ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[video]}),
    );
    let job = answer["job"].as_str().unwrap().to_string();
    std::thread::sleep(Duration::from_millis(150));
    ok(&mut host, &mut state, json!({"op":"job.cancel","job":job}));

    let event = poll_until(&mut host, &mut state, &job);
    assert_eq!(event["state"], "cancelled", "{event}");
    assert!(state.library.items.is_empty());
    assert_eq!(
        state.project.revision, revision,
        "a cancelled job is not an edit"
    );
    assert_eq!(files_under(&dir.join("cancelled")), before, "no leftovers");
    // A cancelled job is reported once.
    assert!(host.poll(&mut state).is_empty());
}

#[test]
fn a_prepared_result_for_a_replaced_project_is_discarded() {
    let dir = temp("async-stale-decoded");
    let video = video_with_audio(&tools(), &dir, "clip.mp4");
    let mut host = interactive(tools());
    let mut state = EditorState::demo();
    new_project(&mut host, &mut state, &dir, "first");
    let answer = ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[video]}),
    );
    let import = answer["job"].as_str().unwrap().to_string();
    // This poll applies the import and starts the automatic prepare job.
    assert_eq!(
        poll_until(&mut host, &mut state, &import)["state"],
        "succeeded"
    );
    assert_eq!(jobs_of_kind(&host, "proxy"), 1);

    // Another project takes the window before the prepare result is applied.
    new_project(&mut host, &mut state, &dir, "second");
    let prepare = host.registry().to_json()["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|job| job["kind"] == "proxy")
        .and_then(|job| job["id"].as_str())
        .unwrap()
        .to_string();
    let event = poll_until(&mut host, &mut state, &prepare);
    assert_eq!(event["applied"]["stale"], true, "{event}");
    assert!(
        state.library.items.is_empty(),
        "nothing leaked into the new project"
    );
    // F4: the answer was `Ok`, but the project used none of it: it is not "applied".
    assert_eq!(
        host.registry().get(&prepare).unwrap().result_state,
        ResultState::Discarded
    );
    assert_eq!(
        host.registry().get(&import).unwrap().result_state,
        ResultState::Applied,
        "the import before it was applied"
    );
}

/// BL094.11: `job.commit` on the automatic prepare job before the tick applied the result but left
/// the host believing the job was still in flight. Every scheduler waits for that marker, so the
/// window prepared nothing else (and drew no thumbnail) until it was restarted. Here the next
/// import is prepared on its own after the commit, through the same tick the window runs.
#[test]
fn committing_the_automatic_prepare_job_before_the_tick_does_not_stall_the_next_one() {
    let dir = temp("async-commit-automatic");
    let first = video_with_audio(&tools(), &dir, "first.mp4");
    // Another file, not the same bytes again: a copy would be one asset, not a second one to prepare.
    let second = dir.join("second.mp4");
    common::ffmpeg(
        &tools(),
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=192x108:rate=24:duration=1",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=880:duration=1:sample_rate=48000",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
            second.to_str().unwrap(),
        ],
    );
    let mut host = interactive(tools());
    let mut state = EditorState::demo();
    new_project(&mut host, &mut state, &dir, "commit-automatic");
    let answer = ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[first]}),
    );
    let import = answer["job"].as_str().unwrap().to_string();
    // This poll applies the import and starts the automatic prepare job.
    assert_eq!(
        poll_until(&mut host, &mut state, &import)["state"],
        "succeeded"
    );
    let prepare = job_ids_of_kind(&host, "proxy")
        .pop()
        .expect("the automatic prepare job");

    // The job ends in the background and no tick has run since, so nothing reported it. (An
    // interactive host never waits for a job: `job.get` is how a caller sees it end.)
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let info = ok(&mut host, &mut state, json!({"op":"job.get","job":prepare}));
        if !matches!(info["state"].as_str(), Some("queued" | "running")) {
            assert_eq!(info["state"], "succeeded", "{info}");
            assert_eq!(info["result_state"], "pending", "{info}");
            break;
        }
        assert!(Instant::now() < deadline, "the prepare job never ended");
        std::thread::sleep(Duration::from_millis(20));
    }
    // The commit arrives first and applies the decoded media.
    ok(
        &mut host,
        &mut state,
        json!({"op":"job.commit","job":prepare}),
    );
    assert_eq!(
        host.registry().get(&prepare).unwrap().result_state,
        ResultState::Applied
    );
    assert!(decoded(&state), "the commit applied the decoded media");

    // The next tick reports the committed job once (BL094.10)...
    let reported = host.poll(&mut state);
    assert_eq!(
        reported
            .iter()
            .filter(|event| event["job"] == prepare)
            .count(),
        1,
        "{reported:?}"
    );

    // ...and the host is free to prepare what comes next: the second import starts its own
    // automatic job in the very tick that applies it.
    let answer = ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[second]}),
    );
    let import = answer["job"].as_str().unwrap().to_string();
    assert_eq!(
        poll_until(&mut host, &mut state, &import)["state"],
        "succeeded"
    );
    let next = job_ids_of_kind(&host, "proxy")
        .into_iter()
        .find(|id| *id != prepare)
        .expect("the second import started its own automatic prepare job");
    assert_eq!(
        poll_until(&mut host, &mut state, &next)["state"],
        "succeeded"
    );
    assert!(decoded(&state), "both assets are prepared");
    assert_eq!(state.library.items.len(), 2);
}

#[test]
fn a_restore_that_lost_the_race_with_an_edit_is_discarded_not_merged() {
    let dir = temp("async-stale-open");
    let video = video_with_audio(&tools(), &dir, "clip.mp4");
    let mut host = interactive(tools());
    let mut state = EditorState::demo();
    new_project(&mut host, &mut state, &dir, "origin");
    let imported = ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[video]}),
    );
    let job = imported["job"].as_str().unwrap().to_string();
    poll_until(&mut host, &mut state, &job);
    poll_for(&mut host, &mut state, decoded);
    ok(
        &mut host,
        &mut state,
        json!({"op":"project.save","path":dir.join("origin/origin.tessel.json")}),
    );

    let archive = dir.join("share/pack.tesselpack");
    let packaged = ok(
        &mut host,
        &mut state,
        json!({"op":"project.package","path":archive,"format":"archive"}),
    );
    let job = packaged["job"].as_str().unwrap().to_string();
    assert_eq!(
        poll_until(&mut host, &mut state, &job)["state"],
        "succeeded"
    );

    let mut elsewhere = EditorState::demo();
    let library_before = elsewhere.library.items.len();
    let restore = ok(
        &mut host,
        &mut elsewhere,
        json!({"op":"project.restore","path":archive,"folder":dir.join("restored")}),
    );
    let restore = restore["job"].as_str().unwrap().to_string();
    // The user edits the project that is open while the package unpacks.
    ok(
        &mut host,
        &mut elsewhere,
        json!({"op":"track.add","id":"mine","name":"Mine","kind":"video"}),
    );
    let event = poll_until(&mut host, &mut elsewhere, &restore);
    assert_eq!(event["state"], "failed", "{event}");
    assert_eq!(event["error"]["code"], "stale_result", "{event}");
    assert!(
        elsewhere
            .timeline
            .tracks
            .iter()
            .any(|track| track.id == "mine"),
        "the user's edit stays"
    );
    assert_eq!(
        elsewhere.library.items.len(),
        library_before,
        "nothing of the package was opened"
    );
}

/// BL113a for restore: the same guard as the scene attach job. The destination project is
/// closed and its document reopened while the package unpacks; the reopened one has the same ID
/// and revision 0, and is still not the project the restore was started for.
#[test]
fn a_restore_started_before_the_document_was_reopened_is_discarded() {
    let dir = temp("async-stale-reopen");
    let video = video_with_audio(&tools(), &dir, "clip.mp4");
    let mut host = interactive(tools());
    let mut state = EditorState::demo();
    new_project(&mut host, &mut state, &dir, "origin");
    let imported = ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[video]}),
    );
    let job = imported["job"].as_str().unwrap().to_string();
    poll_until(&mut host, &mut state, &job);
    poll_for(&mut host, &mut state, decoded);
    ok(
        &mut host,
        &mut state,
        json!({"op":"project.save","path":dir.join("origin/origin.tessel.json")}),
    );
    let archive = dir.join("share/pack.tesselpack");
    let packaged = ok(
        &mut host,
        &mut state,
        json!({"op":"project.package","path":archive,"format":"archive"}),
    );
    let job = packaged["job"].as_str().unwrap().to_string();
    assert_eq!(
        poll_until(&mut host, &mut state, &job)["state"],
        "succeeded"
    );

    let mut destination = EditorState::demo();
    new_project(&mut host, &mut destination, &dir, "destination");
    let document = dir.join("destination/destination.tessel.json");
    ok(
        &mut host,
        &mut destination,
        json!({"op":"project.save","path":document}),
    );
    let started = destination.project.stamp();
    let restore = ok(
        &mut host,
        &mut destination,
        json!({"op":"project.restore","path":archive,"folder":dir.join("restored")}),
    );
    let restore = restore["job"].as_str().unwrap().to_string();
    // The destination is closed and its document opened again, before the restore is applied.
    ok(
        &mut host,
        &mut destination,
        json!({"op":"project.open","path":document}),
    );
    assert_eq!(destination.project.revision, 0, "the revision restarted");
    assert!(
        !started.is_current(&destination.project),
        "same ID and revision, other session"
    );
    let reopened = destination.project.session.number();

    let event = poll_until(&mut host, &mut destination, &restore);
    assert_eq!(event["state"], "failed", "{event}");
    assert_eq!(event["error"]["code"], "stale_result", "{event}");
    assert_eq!(
        destination.project.session.number(),
        reopened,
        "the reopened project was not replaced by the restored one"
    );
    assert!(
        destination.library.items.is_empty(),
        "nothing of the package was opened"
    );
}

#[test]
fn a_decode_that_fails_is_reported_once_and_not_retried_every_tick() {
    let dir = temp("async-failing");
    let video = video_with_audio(&tools(), &dir, "clip.mp4");
    let flag = dir.join("fail-now");
    let mut host = interactive(wrapped_tools(&dir, 0.0, &flag));
    let mut state = EditorState::demo();
    new_project(&mut host, &mut state, &dir, "failing");
    let answer = ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[video]}),
    );
    let import = answer["job"].as_str().unwrap().to_string();
    // The import job is done; from now on every tool run fails, so the automatic prepare does.
    let deadline = Instant::now() + Duration::from_secs(60);
    while !host.registry().get(&import).unwrap().state.is_final() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    fs::write(&flag, b"x").unwrap();
    assert_eq!(
        poll_until(&mut host, &mut state, &import)["state"],
        "succeeded"
    );
    let prepare = host.registry().to_json()["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|job| job["kind"] == "proxy")
        .and_then(|job| job["id"].as_str())
        .expect("an automatic prepare job")
        .to_string();
    let event = poll_until(&mut host, &mut state, &prepare);
    assert_eq!(event["state"], "failed", "{event}");
    assert!(
        !event["error"]["message"].as_str().unwrap_or("").is_empty(),
        "{event}"
    );

    for _ in 0..10 {
        host.poll(&mut state);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        jobs_of_kind(&host, "proxy"),
        1,
        "no retry loop on an unchanged file"
    );
    assert!(!decoded(&state));
}

#[test]
fn a_slow_renderer_probe_in_preflight_is_a_cancellable_job() {
    let dir = temp("async-preflight");
    let video = video_with_audio(&tools(), &dir, "clip.mp4");
    // A renderer that is slow to start for as long as the test says (BL139). The mock's `delay`
    // used to stand for that, but the probe only asks for capabilities and never waited for it:
    // the cancel raced a job that took a few milliseconds and, on a busy machine, lost. Here
    // `starting` says the probe is parked inside the factory, `go` lets it through and `returned`
    // says it has left; the bound only keeps a regression from hanging the suite.
    let starting = Arc::new(AtomicBool::new(false));
    let go = Arc::new(AtomicBool::new(false));
    let returned = Arc::new(AtomicBool::new(false));
    let factory: Arc<RendererFactory> = {
        let (starting, go, returned) = (starting.clone(), go.clone(), returned.clone());
        Arc::new(move || {
            starting.store(true, Ordering::SeqCst);
            let give_up = Instant::now() + Duration::from_secs(60);
            while !go.load(Ordering::SeqCst) && Instant::now() < give_up {
                std::thread::sleep(Duration::from_millis(2));
            }
            returned.store(true, Ordering::SeqCst);
            Ok(Box::new(Mock::new(Settings::default())))
        })
    };
    let mut host = MediaHost::new()
        .with_renderer(factory)
        .with_tools(tools())
        .interactive();
    let mut state = EditorState::demo();
    new_project(&mut host, &mut state, &dir, "preflight");
    let imported = ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[video]}),
    );
    poll_until(&mut host, &mut state, imported["job"].as_str().unwrap());
    poll_for(&mut host, &mut state, decoded);

    let started = Instant::now();
    let answer = ok(
        &mut host,
        &mut state,
        json!({"op":"export.preflight","preset":"fullhd","destination":dir.join("out.mp4")}),
    );
    // Parked renderer: a call that waited for it would sit here for the whole bound above.
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "the call returned a job, not the report, after {:?}",
        started.elapsed()
    );
    let job = answer["job"].as_str().unwrap().to_string();
    // The job cannot end before it is cancelled: wait until the probe is inside the renderer.
    let deadline = Instant::now() + Duration::from_secs(60);
    while !starting.load(Ordering::SeqCst) {
        assert!(
            Instant::now() < deadline,
            "the probe never asked for a renderer"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    ok(&mut host, &mut state, json!({"op":"job.cancel","job":job}));
    // It ends as cancelled while the renderer is still parked: cancelling does not wait for it.
    // (`poll_until` would also wait out the factory's own 60 s bound, so the state alone cannot
    // tell a cancel that did not wait from one that did.)
    let event = poll_until(&mut host, &mut state, &job);
    assert_eq!(event["state"], "cancelled", "{event}");
    assert!(
        !returned.load(Ordering::SeqCst),
        "the job ended only after the factory returned: the cancel waited for the probe: {event}"
    );

    // Let the orphaned probe go: it finishes alone and drops its renderer. Asked before that, the
    // host answers that the renderer is busy; asked again and left alone, the same preflight reports.
    go.store(true, Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_secs(60);
    let again = loop {
        let request =
            json!({"op":"export.preflight","preset":"fullhd","destination":dir.join("out.mp4")});
        match run(&mut host, &mut state, request.clone()) {
            Ok(answer) => break answer,
            Err(error) if error.code == "renderer_busy" => {
                assert!(
                    Instant::now() < deadline,
                    "the orphaned probe never let go of its renderer"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("{request}: {error:?}"),
        }
    };
    let event = poll_until(&mut host, &mut state, again["job"].as_str().unwrap());
    assert_eq!(event["state"], "succeeded", "{event}");
    assert_eq!(event["result"]["ok"], true, "{event}");
}
