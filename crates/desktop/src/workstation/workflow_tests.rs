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
    let made = std::process::Command::new("ffmpeg")
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
        view.update(cx, |view, cx| view.playback_tick(cx));
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
