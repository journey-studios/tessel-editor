//! The dialogs, project persistence and playback entry points as the live window serves them.
//! Opening a dialog needs the kit's `Root`, which a test window does not have, so the dialog
//! screens themselves are proven by `scripts/live-flow-proof.py` against the real window; here
//! the shared rules behind them are pinned down: what the tree offers, what an operation
//! refuses, and that saving changes the persistence state the status bar reads.
use std::time::Duration;

use serde_json::Value;

use super::input_tests::fixture;
use super::*;
use crate::workstation::OpReply;
use core::prelude::v1::test;

fn op(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    request: Value,
) -> Result<Value, CommandError> {
    let reply = view.update(cx, |view, cx| view.run_op(&request, cx));
    cx.run_until_parked();
    match reply? {
        OpReply::Done(value) => Ok(value),
        OpReply::Pending(_) => panic!("{request} should not wait for a frame"),
    }
}

/// The `ffmpeg` the app itself would run (override, `PATH`, install places), not the bare name:
/// on Windows the bare name needs a `PATH` that was refreshed after the install. When nothing is
/// found the bare name stays, so the proof is skipped as before.
fn ffmpeg() -> std::path::PathBuf {
    use tessel_editor_media::tools::locate::{System, Tool, locate};
    locate(&System, Tool::Ffmpeg)
        .map(|found| found.path)
        .unwrap_or_else(|| "ffmpeg".into())
}

fn create_project(view: &Entity<Workstation>, cx: &mut VisualTestContext) {
    op(
        view,
        cx,
        json!({"op":"project.create","name":"Flow","width":160,"height":90,"length":48}),
    )
    .unwrap();
}

#[gpui_kit::test]
fn the_flow_buttons_are_in_the_tree_and_enabled(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    for id in [
        "flow.open",
        "flow.save_as",
        "flow.import",
        "flow.package",
        "flow.export",
        "flow.jobs",
        "monitor.play",
    ] {
        let node = view
            .read_with(cx, |view, cx| view.tree_node(id, cx))
            .unwrap_or_else(|| panic!("{id} is not in the tree"));
        assert_eq!(node["enabled"], true, "{id}");
    }
}

#[gpui_kit::test]
fn a_dialog_field_cannot_be_set_without_a_dialog(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let error = cx
        .update(|window, cx| {
            view.update(cx, |view, cx| {
                view.workflow_set("dialog.field.path", "value", "/x", window, cx)
            })
        })
        .expect("the workflow owns dialog.field.*")
        .unwrap_err();
    assert_eq!(error.code, "no_dialog");
    assert!(!view.read_with(cx, |view, _| view.workflow.has_dialog()));
}

#[gpui_kit::test]
fn playback_refuses_a_start_outside_the_sequence_and_stops_when_idle(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    create_project(&view, cx);
    let error = op(&view, cx, json!({"op":"playback.start","from":48})).unwrap_err();
    assert_eq!(error.code, "out_of_bounds");
    let status = op(&view, cx, json!({"op":"playback.status"})).unwrap();
    assert_eq!(status["state"], "stopped");
    let stopped = op(&view, cx, json!({"op":"playback.stop"})).unwrap();
    assert_eq!(stopped["report"]["state"], "stopped");
    assert!(!view.read_with(cx, |view, _| view.playback.is_playing()));
}

#[gpui_kit::test]
fn saving_clears_the_dirty_state_the_status_bar_shows(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    create_project(&view, cx);
    let directory = std::env::temp_dir().join(format!("tessel-workflow-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("flow.tessel.json");
    op(
        &view,
        cx,
        json!({"op":"project.save","path": path.to_string_lossy()}),
    )
    .unwrap();
    let persistence = |view: &Entity<Workstation>, cx: &mut VisualTestContext| {
        view.read_with(cx, |view, _| view.state.summary()["persistence"].clone())
    };
    assert_eq!(persistence(&view, cx)["dirty"], false);
    op(
        &view,
        cx,
        json!({"op":"track.add","id":"late","name":"Late","kind":"video"}),
    )
    .unwrap();
    assert_eq!(persistence(&view, cx)["dirty"], true);
    std::fs::remove_dir_all(&directory).unwrap();
}

#[gpui_kit::test]
fn the_visual_sequence_selector_follows_the_model_and_switches_without_losing_work(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    create_project(&view, cx);
    op(
        &view,
        cx,
        json!({"op":"sequence.add","id":"vertical","name":"Vertical","width":180,"height":320,
            "rate":30,"length":60,"activate":false}),
    )
    .unwrap();
    cx.run_until_parked();
    let (main_id, tracks) = view.read_with(cx, |view, _| {
        (
            view.state.timeline.id.to_string(),
            view.state.timeline.tracks.len(),
        )
    });
    let listed = view.read_with(cx, |view, _| view.sequence_choices.clone());
    assert_eq!(listed.len(), 2, "{listed:?}");
    let label_of = |id: &str| {
        listed
            .iter()
            .find(|(candidate, _)| candidate == id)
            .map(|(_, label)| label.clone())
            .unwrap()
    };
    let shown = view.read_with(cx, |view, cx| {
        view.sequence_select.read(cx).selected_value().cloned()
    });
    assert_eq!(
        shown,
        Some(label_of(&main_id)),
        "the selector shows the active sequence"
    );

    // Picking the other one in the control is the same command `sequence.activate` sends.
    let picked = label_of("vertical");
    view.update(cx, |view, cx| {
        view.sequence_select
            .update(cx, |_, cx| cx.emit(SelectEvent::Confirm(Some(picked))));
    });
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert_eq!(view.state.timeline.id, "vertical");
        assert_eq!(
            (view.state.timeline.width, view.state.timeline.height),
            (180, 320)
        );
        assert_eq!(view.state.sequence(&main_id).unwrap().tracks.len(), tracks);
    });

    // The CLI and the control agree: a semantic switch moves the control too.
    let tessel_editor_core::commands::Parsed::Run(command) =
        tessel_editor_core::commands::parse_set(
            "timeline.sequence",
            "value",
            &Raw::Text(main_id.clone()),
        )
        .unwrap()
    else {
        panic!("selecting a sequence is a state command");
    };
    view.update(cx, |view, cx| view.execute(command, Source::Cli, cx))
        .unwrap();
    cx.run_until_parked();
    let shown = view.read_with(cx, |view, cx| {
        view.sequence_select.read(cx).selected_value().cloned()
    });
    assert_eq!(shown, Some(label_of(&main_id)));
    assert_eq!(
        view.read_with(cx, |view, _| view.state.timeline.tracks.len()),
        tracks
    );
}

#[gpui_kit::test]
fn an_import_is_a_job_the_window_keeps_editing_through_and_applies_on_its_tick(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    let scratch = ClaimsScratch::new("import");
    let dir = &scratch.0;
    let picture = dir.join("still.jpg");
    let made = std::process::Command::new(ffmpeg())
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=64x36:rate=1",
            "-frames:v",
            "1",
        ])
        .arg(&picture)
        .status();
    if !made.is_ok_and(|status| status.success()) {
        // The window only needs FFmpeg for this proof; the media crate's tests require it hard.
        eprintln!("ffmpeg is not on PATH: skipping the window import proof");
        return;
    }
    op(
        &view,
        cx,
        json!({"op":"project.create","name":"Window","dir":dir.join("project"),"width":160,"height":90,"length":48}),
    )
    .unwrap();
    let before = view.read_with(cx, |view, _| view.state.library.items.len());

    let started = std::time::Instant::now();
    let answer = op(&view, cx, json!({"op":"media.import","paths":[picture]})).unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the UI thread only started a job: {answer}"
    );
    let job = answer["job"].as_str().expect("a job id").to_string();

    // The same window keeps taking semantic edits while the job runs.
    op(
        &view,
        cx,
        json!({"op":"track.add","id":"meanwhile","name":"Meanwhile","kind":"video"}),
    )
    .unwrap();

    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        view.update(cx, |view, cx| view.workflow_tick(cx));
        let done = view.read_with(cx, |view, _| {
            view.state.library.items.len() > before
                && view.jobs_report()["recent"]
                    .as_array()
                    .is_some_and(|recent| recent.iter().any(|event| event["job"] == job.as_str()))
        });
        if done {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the tick never applied the import"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    view.read_with(cx, |view, _| {
        assert!(
            view.state
                .timeline
                .tracks
                .iter()
                .any(|track| track.id == "meanwhile")
        );
        assert_eq!(view.jobs_report()["recent"][0]["job"], job.as_str());
    });
}

/// Ticks the window until the job `id` appears among the finished events the status bar reads.
fn tick_until_reported(view: &Entity<Workstation>, cx: &mut VisualTestContext, id: &str) -> Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        view.update(cx, |view, cx| view.workflow_tick(cx));
        let event = view.read_with(cx, |view, _| {
            view.jobs_report()["recent"]
                .as_array()
                .and_then(|recent| recent.iter().find(|event| event["job"] == id).cloned())
        });
        if let Some(event) = event {
            return event;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the tick never reported {id}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// The id of the automatic prepare job (the registry names its kind `proxy`) that is not one of
/// `known`, if the window has started one.
fn prepare_job_besides(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    known: &[&str],
) -> Option<String> {
    view.read_with(cx, |view, _| {
        view.jobs_report()["all"].as_array().and_then(|jobs| {
            jobs.iter()
                .filter(|job| job["kind"] == "proxy")
                .filter_map(|job| job["id"].as_str())
                .find(|id| !known.contains(id))
                .map(str::to_string)
        })
    })
}

/// BL094.11: `job.commit` on the automatic prepare job before the tick took its result but left
/// the host believing the job was still in flight, and every scheduler waits for that marker: the
/// window prepared nothing else, and drew no thumbnail, until it was restarted. Through the
/// window's own tick, the next import is prepared on its own after the commit.
#[gpui_kit::test]
fn committing_the_automatic_prepare_job_before_the_tick_leaves_the_window_free_to_prepare_more(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    let scratch = ClaimsScratch::new("commit-automatic");
    let dir = &scratch.0;
    // Two different files with picture and sound, so that the second is one more asset to prepare.
    let video = |name: &str, size: &str, tone: &str| -> Option<std::path::PathBuf> {
        let path = dir.join(name);
        std::process::Command::new(ffmpeg())
            .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
            .arg(format!("testsrc2=size={size}:rate=24:duration=1"))
            .args(["-f", "lavfi", "-i"])
            .arg(format!(
                "sine=frequency={tone}:duration=1:sample_rate=48000"
            ))
            .args([
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-shortest",
            ])
            .arg(&path)
            .status()
            .ok()
            .filter(|status| status.success())
            .map(|_| path)
    };
    let (Some(first), Some(second)) = (
        video("first.mp4", "160x90", "440"),
        video("second.mp4", "192x108", "880"),
    ) else {
        // The window only needs FFmpeg for this proof; the media crate's tests require it hard.
        eprintln!("ffmpeg is not on PATH: skipping the window automatic-job proof");
        return;
    };
    op(
        &view,
        cx,
        json!({"op":"project.create","name":"Auto","dir":dir.join("project"),"width":160,"height":90,"length":48}),
    )
    .unwrap();

    let answer = op(&view, cx, json!({"op":"media.import","paths":[first]})).unwrap();
    let import = answer["job"].as_str().expect("a job id").to_string();
    // The tick that applies the import also starts the automatic prepare job.
    tick_until_reported(&view, cx, &import);
    let prepare = prepare_job_besides(&view, cx, &[]).expect("the automatic prepare job");

    // The job ends in the background; no tick has run since, so nothing reported it.
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        let info = view.read_with(cx, |view, _| {
            view.jobs_report()["all"]
                .as_array()
                .and_then(|jobs| {
                    jobs.iter()
                        .find(|job| job["id"] == prepare.as_str())
                        .cloned()
                })
                .expect("the job is in the registry")
        });
        if !matches!(info["state"].as_str(), Some("queued" | "running")) {
            assert_eq!(info["state"], "succeeded", "{info}");
            assert_eq!(info["result_state"], "pending", "{info}");
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the prepare job never ended"
        );
        std::thread::sleep(Duration::from_millis(25));
    }

    // The commit arrives first and applies the decoded media; the tick still reports the job.
    op(&view, cx, json!({"op":"job.commit","job":prepare})).unwrap();
    let event = tick_until_reported(&view, cx, &prepare);
    assert_eq!(event["state"], "succeeded", "{event}");

    // The window is not stuck: the next import is prepared on its own, by the same tick.
    let answer = op(&view, cx, json!({"op":"media.import","paths":[second]})).unwrap();
    let import = answer["job"].as_str().expect("a job id").to_string();
    tick_until_reported(&view, cx, &import);
    let next = prepare_job_besides(&view, cx, &[prepare.as_str()])
        .expect("the second import started its own automatic prepare job");
    let event = tick_until_reported(&view, cx, &next);
    assert_eq!(event["state"], "succeeded", "{event}");
    view.read_with(cx, |view, _| {
        let assets: Vec<_> = view
            .state
            .library
            .items
            .iter()
            .filter_map(|item| item.asset.as_ref())
            .collect();
        assert_eq!(assets.len(), 2);
        assert!(
            assets.iter().all(|asset| asset.decoded.is_some()),
            "both assets are prepared"
        );
    });
}

#[gpui_kit::test]
fn the_claims_operations_are_jobs_with_typed_refusals_and_the_window_reports_them(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    // No workspace yet: both refuse before any job exists, and nothing is created.
    let jobs_before = view.read_with(cx, |view, _| view.jobs_report()["all"].clone());
    for name in ["scene.resources.claims", "scene.resources.recover"] {
        let error = op(&view, cx, json!({ "op": name })).unwrap_err();
        assert_eq!(error.code, "no_workspace", "{name}");
    }
    assert_eq!(
        view.read_with(cx, |view, _| view.jobs_report()["all"].clone()),
        jobs_before,
        "a refusal starts no job"
    );

    let scratch = ClaimsScratch::new("claims");
    let dir = &scratch.0;
    op(
        &view,
        cx,
        json!({"op":"project.create","name":"Claims","dir":dir.join("project"),"width":160,"height":90,"length":48}),
    )
    .unwrap();
    let workspace = dir.join("project");

    // The UI thread only starts the jobs; the tick reports them; the answer says what it did.
    let started = op(&view, cx, json!({"op":"scene.resources.claims"})).unwrap();
    let job = started["job"].as_str().expect("a job id").to_string();
    let event = tick_until_reported(&view, cx, &job);
    assert_eq!(
        event["result"]["action"], "scene.resources.claims",
        "{event}"
    );
    assert_eq!(
        event["result"]["readOnly"], false,
        "claims reaps: not read-only"
    );
    assert_eq!(event["result"]["orphans"], 0);

    let started = op(&view, cx, json!({"op":"scene.resources.recover"})).unwrap();
    let job = started["job"].as_str().expect("a job id").to_string();
    let event = tick_until_reported(&view, cx, &job);
    assert_eq!(event["result"]["recovery"]["orphans"], 0, "{event}");

    // A ledger that cannot be read: the job fails with the typed code and the window says so.
    std::fs::create_dir_all(workspace.join(".tessel-claims")).unwrap();
    let ledger = workspace.join(".tessel-claims/claims.json");
    std::fs::write(&ledger, b"not a ledger").unwrap();
    std::fs::write(workspace.join(".tessel-claims/ledger.lock"), b"").unwrap();
    let started = op(&view, cx, json!({"op":"scene.resources.recover"})).unwrap();
    let job = started["job"].as_str().expect("a job id").to_string();
    let event = tick_until_reported(&view, cx, &job);
    // The window's events carry the command-level error: the job's own code is `details.job_code`.
    assert_eq!(event["state"], "failed", "{event}");
    assert_eq!(
        event["error"]["details"]["job_code"], "claims_unavailable",
        "{event}"
    );
    assert_eq!(
        std::fs::read(&ledger).unwrap(),
        b"not a ledger",
        "nothing repaired"
    );
}

/// Ticks the window's playback clock the way its timer does, until `done` or the time runs out.
fn tick_playback(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    mut done: impl FnMut(&Value) -> bool,
) -> Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        view.update(cx, |view, cx| {
            view.playback_tick(cx, std::time::Instant::now(), Duration::ZERO)
        });
        let report = view.read_with(cx, |view, _| view.playback.report());
        if done(&report) || std::time::Instant::now() > deadline {
            return report;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[gpui_kit::test]
fn playback_follows_the_audio_clock_or_says_why_it_cannot(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    create_project(&view, cx);
    let started = op(&view, cx, json!({"op":"playback.start","from":0})).unwrap();
    assert_eq!(started["state"], "preparing");
    cx.run_until_parked();
    let report = tick_playback(&view, cx, |report| report["clock"] != "starting");
    assert_eq!(report["state"], "playing", "{report}");
    match report["clock"].as_str().unwrap() {
        // A machine with an output device: the sample clock drives the playhead.
        "audio_device" => {
            // The device's own rate, whatever the machine's default output is (44.1/48/96 kHz, or
            // 24 kHz for a Bluetooth headset): the test must not depend on which one is plugged in.
            let rate = report["audio"]["output"]["sample_rate"].as_u64();
            assert!(rate.is_some_and(|rate| rate >= 8_000), "{report}");
            assert!(report["audio"]["latency_ms"].as_f64().unwrap() >= 0.0);
            assert_eq!(
                report["audio"]["heard"], false,
                "nothing here claims to be heard"
            );
        }
        // No device (CI): an explicit error, never a silent pretend.
        "wall" => assert_eq!(
            report["audio"]["error"]["code"], "audio_output_unavailable",
            "{report}"
        ),
        other => panic!("unexpected clock {other}: {report}"),
    }

    // Pause holds the playhead; seek moves picture and sound, even while paused.
    op(&view, cx, json!({"op":"playback.pause"})).unwrap();
    let held = tick_playback(&view, cx, |_| true)["frame"].clone();
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(
        tick_playback(&view, cx, |_| true)["frame"],
        held,
        "paused is held"
    );
    let sought = op(&view, cx, json!({"op":"playback.seek","frame":30})).unwrap();
    assert_eq!(sought["report"]["state"], "paused");
    assert_eq!(
        view.read_with(cx, |view, _| view.state.timeline.playhead),
        30
    );
    let error = op(&view, cx, json!({"op":"playback.seek","frame":4800})).unwrap_err();
    assert_eq!(error.code, "out_of_bounds");

    // Resume continues from the seek target, never from the old position.
    op(&view, cx, json!({"op":"playback.resume"})).unwrap();
    let moved = tick_playback(&view, cx, |report| {
        report["frame"].as_i64().unwrap_or(0) > 30
    });
    assert!(moved["frame"].as_i64().unwrap() > 30, "{moved}");
    assert_eq!(moved["state"], "playing");

    let stopped = op(&view, cx, json!({"op":"playback.stop"})).unwrap();
    assert_eq!(stopped["report"]["state"], "stopped");
    let idle = op(&view, cx, json!({"op":"playback.pause"})).unwrap_err();
    assert_eq!(idle.code, "not_playing");
}

/// Removes this test's project even when an assertion unwinds.
struct ClaimsScratch(std::path::PathBuf);

impl ClaimsScratch {
    fn new(kind: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("tessel-window-{kind}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for ClaimsScratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// F5: the status line keeps the reason a result was refused when the discard job that took it
/// ends, and says both when that discard left something behind.
#[test]
fn a_discard_never_hides_why_its_result_was_refused() {
    let mut workflow = workflow::Workflow::new();
    let refused = json!({"job": "job-3", "state": "failed", "kind": "scene-adopt",
        "error": {"code": "claims_unavailable", "message": "the ledger said no",
            "details": {"discard_job": "job-9", "phase": "accept"}}});
    let (text, error) = flows::describe_job_event(&refused);
    assert_eq!(
        workflow.remember_refusal(&refused, text.clone(), error),
        (text.clone(), true),
        "the refusal itself reads as it always did"
    );

    let clean = json!({"job": "job-9", "state": "succeeded", "kind": "discard",
        "result": {"cleanup": {"removedFiles": 2, "kept": [], "busyRoots": 0}}});
    let (line, line_error) = flows::describe_job_event(&clean);
    assert_eq!(line, "Resultado descartado");
    assert_eq!(
        workflow.remember_refusal(&clean, line, line_error),
        (text.clone(), true),
        "a clean discard keeps saying why the result was refused"
    );

    // Remembered once: the same discard cannot hide a later message again.
    let (line, line_error) = flows::describe_job_event(&clean);
    assert_eq!(
        workflow.remember_refusal(&clean, line.clone(), line_error),
        (line, false)
    );

    // A discard that kept files says both.
    workflow.remember_refusal(&refused, text.clone(), true);
    let kept = json!({"job": "job-9", "state": "succeeded", "kind": "discard",
        "result": {"cleanup": {"kept": ["`/w/a.png`: its content changed"], "busyRoots": 0}}});
    let (line, line_error) = flows::describe_job_event(&kept);
    let (both, both_error) = workflow.remember_refusal(&kept, line.clone(), line_error);
    assert!(both_error);
    assert!(both.starts_with(&text) && both.ends_with(&line), "{both}");
}

/// An explicit `job.discard` is not a refusal: its `job_discarded` event is not remembered, so the
/// discard that follows keeps its own line.
#[test]
fn an_explicit_discard_is_not_remembered_as_a_refusal() {
    let mut workflow = workflow::Workflow::new();
    let discarded = json!({"job": "job-3", "state": "failed", "kind": "scene-attach",
        "error": {"code": "job_discarded", "message": "gone",
            "details": {"discard_job": "job-9"}}});
    let (text, error) = flows::describe_job_event(&discarded);
    workflow.remember_refusal(&discarded, text, error);
    let clean = json!({"job": "job-9", "state": "succeeded", "kind": "discard",
        "result": {"cleanup": {"removedFiles": 0, "kept": [], "busyRoots": 0}}});
    let (line, line_error) = flows::describe_job_event(&clean);
    assert_eq!(
        workflow.remember_refusal(&clean, line.clone(), line_error),
        (line, false)
    );
}

#[gpui_kit::test]
fn an_idle_tick_does_not_repaint_the_window_but_a_changed_jobs_line_does(cx: &mut TestAppContext) {
    use std::cell::Cell;
    use std::rc::Rc;

    let (view, cx) = fixture(cx);
    let repaints = Rc::new(Cell::new(0usize));
    let counter = repaints.clone();
    cx.update(|_, cx| {
        cx.observe(&view, move |_, _| counter.set(counter.get() + 1))
            .detach();
    });

    // The tick runs every 250 ms: with nothing finished, nothing running and nothing to autosave
    // it must not ask for a frame (it used to, so an idle window painted four times a second).
    for _ in 0..4 {
        view.update(cx, |view, cx| view.workflow_tick(cx));
    }
    cx.run_until_parked();
    assert_eq!(repaints.get(), 0, "an idle tick repainted the window");

    // A job appearing changes the status bar's jobs line: that one tick repaints, the next
    // (same line) does not.
    let (release, wait) = std::sync::mpsc::channel();
    view.update(cx, |view, _| {
        view.workflow.spawn_blocked_job("Hashing", wait);
    });
    view.update(cx, |view, cx| view.workflow_tick(cx));
    cx.run_until_parked();
    assert_eq!(
        repaints.get(),
        1,
        "the jobs line changed and nothing repainted"
    );
    view.update(cx, |view, cx| view.workflow_tick(cx));
    cx.run_until_parked();
    assert_eq!(repaints.get(), 1, "an unchanged jobs line repainted again");

    // The job ending is reported (an event) and repaints once more.
    release.send(()).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while repaints.get() < 2 {
        view.update(cx, |view, cx| view.workflow_tick(cx));
        cx.run_until_parked();
        assert!(
            std::time::Instant::now() < deadline,
            "the finished job never repainted"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}
