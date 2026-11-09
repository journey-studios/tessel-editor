//! The viewport host without a window: a scripted page answers on the bridge, the way the real
//! runtime does, and the tests watch what the host asks for and what it reports.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tessel_editor_core::bridge::PreviewSort;
use tessel_editor_core::content::TextSpec;
use tessel_editor_core::headless::create_project;
use tessel_editor_core::ops::RenderFrame;
use tessel_editor_core::plan::{PlanRequest, PreviewIntent};
use tessel_editor_core::state::timeline::TrackKind;
use tessel_editor_core::state::{Command, ProjectSettings};
use tessel_editor_core::time::Ratio;
use tessel_editor_core::{EditorState, Id, Raw};
use tessel_editor_render::SetupError;
use wry::http::Request;

use super::viewport_host::{HostOptions, PAGE_URL, ViewportHost};

pub(super) fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tessel-vh-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub(super) fn page(tag: &str) -> PathBuf {
    let dist = scratch(tag).join("dist");
    std::fs::create_dir_all(&dist).unwrap();
    std::fs::write(dist.join("index.html"), "<html>").unwrap();
    std::fs::write(dist.join("runtime.js"), "//").unwrap();
    dist
}

fn empty_state() -> EditorState {
    EditorState::empty(&ProjectSettings::default()).unwrap()
}

/// Messages the host queued for the page, decoded back into JSON.
pub(super) fn sent(host: &mut ViewportHost) -> Vec<Value> {
    host.take_outgoing()
        .iter()
        .map(|script| {
            let body = script
                .strip_prefix("window.TesselRuntime.receive(")
                .and_then(|rest| rest.strip_suffix(')'))
                .unwrap_or_else(|| panic!("not a receive call: {script}"));
            serde_json::from_str(body).unwrap()
        })
        .collect()
}

pub(super) fn kinds(messages: &[Value]) -> Vec<&str> {
    messages
        .iter()
        .map(|message| message["type"].as_str().unwrap())
        .collect()
}

pub(super) fn from_page(host: &mut ViewportHost, state: &EditorState, message: Value) {
    let mut message = message;
    message["v"] = json!(1);
    host.on_ipc(&message.to_string(), state);
}

pub(super) fn ready_message() -> Value {
    json!({"type":"ready","runtime":{"name":"tessel-three","version":"0.1.0","renderer":"Test GPU"},
        "capabilities":{"layers":["image","video","text","scene","waveform"],"formats":{}}})
}

pub(super) fn frame_done(id: u64, revision: u64) -> Value {
    json!({"type":"frame.done","id":id,"revision":revision,"width":4,"height":2,"hash":"abcd",
        "mean":[1,2,3,255],"probes":[[9,8,7,255]],"png":null,
        "timings":{"render_ms":1.5,"total_ms":2.5}})
}

/// A 100-frame sequence with one title clip over all of it, so the plan has a layer an edit can
/// change and every playhead position has a distinct plan.
pub(super) fn titled_state(title: &str) -> EditorState {
    let mut state = empty_state();
    state
        .execute(Command::SetSequenceLength { frames: 100 })
        .unwrap();
    state
        .execute(Command::CreateTrack {
            id: Some("v1".into()),
            name: "V1".into(),
            kind: TrackKind::Video,
        })
        .unwrap();
    state
        .execute(Command::AddClip {
            id: Some("t1".into()),
            name: None,
            track: "v1".into(),
            asset: None,
            start: 0,
            duration: 100,
            source_start: None,
            text: Some(TextSpec::new(title, 10.0, "#ffffff").unwrap()),
            camera: None,
            properties: vec![],
        })
        .unwrap();
    state
}

pub(super) fn retitle(state: &mut EditorState, title: &str) {
    state
        .execute(Command::SetClipText {
            clip: "t1".into(),
            text: TextSpec::new(title, 10.0, "#ffffff").unwrap(),
        })
        .unwrap();
}

fn seek(state: &mut EditorState, frame: i64) {
    state
        .execute(Command::SetPlayhead {
            frame: Raw::Number(frame as f64),
        })
        .unwrap();
}

/// The `frame.render` messages among `messages`, as `(id, revision, plan frame)`.
fn renders(messages: &[Value]) -> Vec<(u64, u64, i64)> {
    messages
        .iter()
        .filter(|message| message["type"] == "frame.render")
        .map(|message| {
            (
                message["id"].as_u64().unwrap(),
                message["revision"].as_u64().unwrap(),
                message["plan"]["frame"].as_i64().unwrap(),
            )
        })
        .collect()
}

/// A host whose page has loaded and completed the handshake.
fn ready_host(tag: &str, state: &EditorState) -> ViewportHost {
    let mut host = ViewportHost::new(Ok(page(tag)));
    from_page(&mut host, state, json!({"type":"page.ready"}));
    assert_eq!(kinds(&sent(&mut host)), ["hello"]);
    from_page(&mut host, state, ready_message());
    assert!(host.is_ready());
    host
}

#[test]
fn nothing_is_sent_to_a_page_that_has_not_announced_itself() {
    let state = empty_state();
    let mut host = ViewportHost::new(Ok(page("boot")));
    host.sync(&state);
    assert!(host.take_outgoing().is_empty(), "the page is still loading");
    assert_eq!(host.headline(), "Iniciando visualizador…");
    assert_eq!(host.report()["phase"], "booting");
}

#[test]
fn the_handshake_is_event_driven_and_the_first_frame_follows_it() {
    let state = empty_state();
    let mut host = ViewportHost::new(Ok(page("handshake")));
    from_page(&mut host, &state, json!({"type":"page.ready"}));
    assert_eq!(host.report()["phase"], "handshaking");
    assert_eq!(sent(&mut host)[0]["type"], "hello");

    from_page(&mut host, &state, ready_message());
    // The project changed while the page was loading: the first frame is requested at once.
    let messages = sent(&mut host);
    assert_eq!(kinds(&messages), ["frame.render"]);
    assert_eq!(messages[0]["output"]["width"], 960);
    assert_eq!(host.report()["capabilities"]["renderer"], "Test GPU");
}

#[test]
fn large_sequences_render_at_a_bounded_preview_size() {
    let state = empty_state();
    let host = ready_host("size", &state);
    // The 1920x1080 sequence is shown at 960x540: same aspect, long side bounded.
    assert_eq!(
        host.report()["output"],
        json!({"width": 960, "height": 540})
    );
}

#[test]
fn an_answer_for_a_superseded_request_is_dropped() {
    let mut state = titled_state("Um");
    let mut host = ready_host("stale", &state);
    let first = sent(&mut host);
    let first_id = first[0]["id"].as_u64().unwrap();
    let first_revision = first[0]["revision"].as_u64().unwrap();

    // An edit (not a seek) changes what the frame in flight shows: it is obsolete.
    retitle(&mut state, "Dois");
    host.sync(&state);
    let second = sent(&mut host);
    assert_eq!(kinds(&second), ["frame.cancel", "frame.render"]);
    let second_id = second[1]["id"].as_u64().unwrap();
    assert!(second_id > first_id);

    // The runtime answers the old request last: it must not become the picture.
    from_page(
        &mut host,
        &state,
        frame_done(second_id, second[1]["revision"].as_u64().unwrap()),
    );
    from_page(&mut host, &state, frame_done(first_id, first_revision));
    let report = host.report();
    assert_eq!(report["last_frame"]["request"], second_id);
    assert_eq!(report["session"]["stats"]["dropped_stale"], 1);
    assert_eq!(report["session"]["stats"]["accepted"], 1);
}

#[test]
fn a_state_change_that_does_not_alter_the_picture_does_not_render_again() {
    let state = empty_state();
    let mut host = ready_host("same", &state);
    let requested = sent(&mut host);
    assert_eq!(kinds(&requested), ["frame.render"]);
    from_page(
        &mut host,
        &state,
        frame_done(
            requested[0]["id"].as_u64().unwrap(),
            requested[0]["revision"].as_u64().unwrap(),
        ),
    );
    host.sync(&state);
    host.sync(&state);
    assert!(host.take_outgoing().is_empty());
    assert_eq!(host.report()["session"]["stats"]["requests"], 1);
}

#[test]
fn a_failed_frame_is_reported_and_the_next_change_recovers() {
    let mut state = empty_state();
    let mut host = ready_host("failure", &state);
    let request = sent(&mut host).remove(0);
    from_page(
        &mut host,
        &state,
        json!({"type":"frame.failed","id":request["id"],"revision":request["revision"],
            "error":{"code":"unsupported_format","message":"cannot draw `overlay`"}}),
    );
    assert_eq!(host.headline(), "Falha: unsupported_format");
    assert_eq!(host.failure().unwrap().code, "unsupported_format");
    assert!(
        host.blocking_error().is_none(),
        "a failed frame does not take the viewport down"
    );

    state
        .execute(Command::SetSequenceLength { frames: 100 })
        .unwrap();
    state
        .execute(Command::SetPlayhead {
            frame: Raw::Number(3.0),
        })
        .unwrap();
    host.sync(&state);
    let retry = sent(&mut host);
    assert_eq!(kinds(&retry), ["frame.render"]);
    from_page(
        &mut host,
        &state,
        frame_done(
            retry[0]["id"].as_u64().unwrap(),
            retry[0]["revision"].as_u64().unwrap(),
        ),
    );
    assert!(host.failure().is_none());
    assert_eq!(host.headline(), "Three.js · 4×2 · 2 ms");
}

#[test]
fn a_lost_context_fails_the_frame_and_a_restored_one_redraws() {
    let state = empty_state();
    let mut host = ready_host("context", &state);
    let request = sent(&mut host).remove(0);
    from_page(
        &mut host,
        &state,
        frame_done(
            request["id"].as_u64().unwrap(),
            request["revision"].as_u64().unwrap(),
        ),
    );

    from_page(&mut host, &state, json!({"type":"context.lost"}));
    assert_eq!(host.failure().unwrap().code, "context_lost");
    assert!(
        host.is_ready(),
        "the page is still there, only its context is gone"
    );

    from_page(&mut host, &state, json!({"type":"context.restored"}));
    let redraw = sent(&mut host);
    assert_eq!(
        kinds(&redraw),
        ["frame.render"],
        "the picture must be requested again"
    );
}

#[test]
fn a_missing_bundle_is_unavailable_with_the_fix_and_never_simulated() {
    let state = empty_state();
    let mut host = ViewportHost::new(Err(SetupError::BundleMissing(
        "runtime/dist/index.html".into(),
    )));
    assert!(!host.has_page());
    host.sync(&state);
    assert!(host.take_outgoing().is_empty());
    let error = host.blocking_error().unwrap();
    assert!(error.message.contains("pnpm --dir"), "{}", error.message);
    let frame = RenderFrame {
        plan: PlanRequest::of(&state),
        out: None,
        size: None,
        probes: vec![],
    };
    let refused = host.render_frame(&state, &frame).unwrap_err();
    assert_eq!(refused.code, "backend_unavailable");
    assert!(!host.begin_reload());
}

#[test]
fn an_explicit_frame_answers_when_the_page_has_drawn_it() {
    let state = empty_state();
    let mut host = ready_host("explicit", &state);
    let auto = sent(&mut host).remove(0);
    let frame = RenderFrame {
        plan: PlanRequest::of(&state),
        out: None,
        size: Some((64, 36)),
        probes: vec![(1, 1)],
    };
    let waiter = host.render_frame(&state, &frame).unwrap();
    // The automatic request in flight is cancelled by the explicit one.
    let messages = sent(&mut host);
    assert_eq!(kinds(&messages), ["frame.cancel", "frame.render"]);
    assert_eq!(messages[1]["output"]["width"], 64);
    assert_eq!(messages[1]["probes"], json!([[1, 1]]));
    assert!(
        waiter.try_recv().is_err(),
        "nothing is answered before the page draws"
    );

    let id = messages[1]["id"].as_u64().unwrap();
    from_page(
        &mut host,
        &state,
        frame_done(
            auto["id"].as_u64().unwrap(),
            auto["revision"].as_u64().unwrap(),
        ),
    );
    assert!(
        waiter.try_recv().is_err(),
        "the superseded answer does not resolve it"
    );
    from_page(
        &mut host,
        &state,
        frame_done(id, messages[1]["revision"].as_u64().unwrap()),
    );
    let report = waiter.try_recv().unwrap().unwrap();
    assert_eq!(report["action"], "render.frame");
    assert_eq!(report["frame"]["probes"], json!([[9, 8, 7, 255]]));
}

#[test]
fn a_newer_request_cancels_the_explicit_one_waiting_for_its_frame() {
    let mut state = empty_state();
    let mut host = ready_host("supersede", &state);
    sent(&mut host);
    let frame = RenderFrame {
        plan: PlanRequest::of(&state),
        out: None,
        size: None,
        probes: vec![],
    };
    let waiter = host.render_frame(&state, &frame).unwrap();
    state
        .execute(Command::SetSequenceLength { frames: 50 })
        .unwrap();
    state
        .execute(Command::SetPlayhead {
            frame: Raw::Number(7.0),
        })
        .unwrap();
    host.sync(&state);
    let error = waiter.try_recv().unwrap().unwrap_err();
    assert_eq!(error.code, "cancelled");
}

#[test]
fn a_png_the_page_encodes_is_written_by_the_host() {
    let state = empty_state();
    let mut host = ready_host("png", &state);
    sent(&mut host);
    let target = scratch("png-out").join("frame.png");
    let frame = RenderFrame {
        plan: PlanRequest::of(&state),
        out: Some(target.clone()),
        size: None,
        probes: vec![],
    };
    let waiter = host.render_frame(&state, &frame).unwrap();
    let render = sent(&mut host).pop().unwrap();
    assert_eq!(render["output"]["png"], target.to_string_lossy().as_ref());
    let mut done = frame_done(
        render["id"].as_u64().unwrap(),
        render["revision"].as_u64().unwrap(),
    );
    done["png_data"] = json!("iVBORw0KGgo=");
    from_page(&mut host, &state, done);
    let report = waiter.try_recv().unwrap().unwrap();
    assert_eq!(report["frame"]["png"], target.to_string_lossy().as_ref());
    assert_eq!(&std::fs::read(&target).unwrap()[1..4], b"PNG");
}

#[test]
fn a_png_that_cannot_be_written_fails_the_frame_instead_of_lying() {
    let state = empty_state();
    let mut host = ready_host("png-bad", &state);
    sent(&mut host);
    let directory = scratch("png-bad-out");
    // A directory where the file should be: the write fails.
    std::fs::create_dir_all(directory.join("frame.png")).unwrap();
    let frame = RenderFrame {
        plan: PlanRequest::of(&state),
        out: Some(directory.join("frame.png")),
        size: None,
        probes: vec![],
    };
    let waiter = host.render_frame(&state, &frame).unwrap();
    let render = sent(&mut host).pop().unwrap();
    let mut done = frame_done(
        render["id"].as_u64().unwrap(),
        render["revision"].as_u64().unwrap(),
    );
    done["png_data"] = json!("iVBORw0KGgo=");
    from_page(&mut host, &state, done);
    let error = waiter.try_recv().unwrap().unwrap_err();
    assert_eq!(error.code, "render_failed");
}

#[test]
fn a_closed_runtime_takes_the_viewport_down_and_fails_waiters() {
    let state = empty_state();
    let mut host = ready_host("closed", &state);
    sent(&mut host);
    let frame = RenderFrame {
        plan: PlanRequest::of(&state),
        out: None,
        size: None,
        probes: vec![],
    };
    let waiter = host.render_frame(&state, &frame).unwrap();
    sent(&mut host);
    from_page(
        &mut host,
        &state,
        json!({"type":"closed","reason":"disposed"}),
    );
    assert_eq!(host.report()["phase"], "unavailable");
    assert_eq!(
        waiter.try_recv().unwrap().unwrap_err().code,
        "backend_unavailable"
    );
    host.sync(&state);
    assert!(
        host.take_outgoing().is_empty(),
        "nothing is sent to a closed runtime"
    );

    // Reload brings it back: the page announces itself again and the handshake restarts.
    assert!(host.begin_reload());
    assert_eq!(host.report()["phase"], "booting");
    from_page(&mut host, &state, json!({"type":"page.ready"}));
    assert_eq!(kinds(&sent(&mut host)), ["hello"]);
}

#[test]
fn a_runtime_speaking_another_bridge_version_is_refused() {
    let state = empty_state();
    let mut host = ViewportHost::new(Ok(page("version")));
    from_page(&mut host, &state, json!({"type":"page.ready"}));
    host.take_outgoing();
    host.on_ipc(
        &json!({"v":2,"type":"ready","runtime":{},"capabilities":{}}).to_string(),
        &state,
    );
    assert_eq!(host.report()["phase"], "unavailable");
    // A runtime that cannot be understood is closed, never half-trusted.
    assert!(
        host.blocking_error().unwrap().message.contains("bridge"),
        "{:?}",
        host.blocking_error()
    );
}

#[test]
fn a_page_that_never_starts_times_out_with_the_build_command() {
    let mut host = ViewportHost::new(Ok(page("boot-timeout")));
    assert!(!host.tick(Instant::now()));
    assert!(host.tick(Instant::now() + Duration::from_secs(16)));
    let error = host.blocking_error().unwrap();
    assert_eq!(error.code, "timeout");
    assert!(error.message.contains("pnpm --dir runtime run build"));
}

#[test]
fn a_frame_that_never_comes_is_cancelled_after_the_timeout() {
    let state = empty_state();
    let mut host = ready_host("frame-timeout", &state);
    sent(&mut host);
    assert!(!host.tick(Instant::now()));
    assert!(host.tick(Instant::now() + Duration::from_secs(31)));
    assert_eq!(host.failure().unwrap().code, "timeout");
    assert!(
        host.is_ready(),
        "a slow frame does not take the viewport down"
    );
}

#[test]
fn a_frame_that_timed_out_is_asked_for_again_twice_and_then_left_alone() {
    let mut state = titled_state("Titulo");
    let mut host = ready_host("frame-retry", &state);
    sent(&mut host);
    let timeout = Duration::from_secs(31);
    assert!(host.tick(Instant::now() + timeout));
    assert_eq!(host.failure().unwrap().code, "timeout");
    assert!(
        !host.retry_due(Instant::now()),
        "after a pause, not at once"
    );
    assert!(host.retry_due(Instant::now() + Duration::from_secs(2)));
    assert!(
        !host.retry_due(Instant::now() + Duration::from_secs(2)),
        "one retry per due time"
    );
    // The retry is the sync a change would make: the frame is asked for again.
    host.sync(&state);
    assert_eq!(kinds(&sent(&mut host)).last(), Some(&"frame.render"));
    assert!(host.failure().is_none());

    // Second timeout, second retry (a longer pause); a third one is not made.
    assert!(host.tick(Instant::now() + timeout));
    assert!(!host.retry_due(Instant::now() + Duration::from_secs(2)));
    assert!(host.retry_due(Instant::now() + Duration::from_secs(4)));
    host.sync(&state);
    assert_eq!(kinds(&sent(&mut host)).last(), Some(&"frame.render"));
    assert!(host.tick(Instant::now() + timeout));
    assert!(!host.retry_due(Instant::now() + Duration::from_secs(3600)));
    assert_eq!(
        host.failure().unwrap().code,
        "timeout",
        "the failure stays on the header"
    );

    // A change the user makes starts the retries over (the window says so with `note_change`).
    seek(&mut state, 3);
    host.note_change();
    host.sync(&state);
    assert_eq!(kinds(&sent(&mut host)).last(), Some(&"frame.render"));
    assert!(host.tick(Instant::now() + timeout));
    assert!(host.retry_due(Instant::now() + Duration::from_secs(2)));
}

#[test]
fn a_picture_that_arrives_gives_the_next_failure_all_its_retries() {
    let mut state = titled_state("Titulo");
    let mut host = ready_host("retry-delivered", &state);
    let timeout = Duration::from_secs(31);
    sent(&mut host);
    assert!(host.tick(Instant::now() + timeout));
    assert!(host.retry_due(Instant::now() + Duration::from_secs(2)));
    host.sync(&state);
    let render = sent(&mut host);
    let id = render.last().unwrap()["id"].as_u64().unwrap();
    let revision = render.last().unwrap()["revision"].as_u64().unwrap();
    from_page(&mut host, &state, frame_done(id, revision));
    assert!(host.failure().is_none());

    // Had the count survived the picture, this would be the second retry (3 s), not the first (1 s).
    seek(&mut state, 3);
    host.note_change();
    host.sync(&state);
    assert!(host.tick(Instant::now() + timeout));
    assert!(host.retry_due(Instant::now() + Duration::from_secs(2)));
}

#[test]
fn another_failure_while_a_retry_is_due_takes_the_retry_with_it() {
    let state = titled_state("Titulo");
    let mut host = ready_host("retry-other-code", &state);
    let first = sent(&mut host);
    assert!(host.tick(Instant::now() + Duration::from_secs(31)));
    assert_eq!(host.failure().unwrap().code, "timeout");
    // Before the retry fires, the frame is asked for by a change and the page answers with another error.
    host.note_change();
    host.sync(&state);
    let second = sent(&mut host);
    let (id, revision) = {
        let render = second.last().unwrap();
        (
            render["id"].as_u64().unwrap(),
            render["revision"].as_u64().unwrap(),
        )
    };
    assert_ne!(Some(id), first.last().unwrap()["id"].as_u64());
    from_page(
        &mut host,
        &state,
        json!({"type":"frame.failed","id":id,"revision":revision,
            "error":{"code":"render_failed","message":"boom"}}),
    );
    assert_eq!(host.failure().unwrap().code, "render_failed");
    assert!(
        !host.retry_due(Instant::now() + Duration::from_secs(60)),
        "only a timeout is retried, and the retry that was due belonged to the failure that is gone"
    );
}

#[test]
fn a_script_the_page_refuses_is_reported_not_retried() {
    let state = empty_state();
    let mut host = ready_host("script", &state);
    host.on_script_error("view is gone");
    assert_eq!(host.report()["phase"], "unavailable");
    assert!(
        host.blocking_error()
            .unwrap()
            .message
            .contains("view is gone")
    );
}

#[test]
fn assets_are_loaded_before_the_frame_and_only_those_are_served() {
    let state = EditorState::demo();
    let mut host = ready_host("assets", &state);
    let messages = sent(&mut host);
    let loads: Vec<&Value> = messages
        .iter()
        .filter(|m| m["type"] == "asset.load")
        .collect();
    assert!(
        !loads.is_empty(),
        "the demo's images are loaded: {:?}",
        kinds(&messages)
    );
    assert!(
        !kinds(&messages).contains(&"frame.render"),
        "no frame until the assets are ready"
    );

    let handle = host.protocol_handle();
    let file = loads[0]["asset"]["source"]["path"].as_str().unwrap();
    let encoded = file.replace('/', "%2F").replace(' ', "%20");
    let served = |query: &str| {
        let request = Request::builder()
            .uri(format!("tessel://localhost/asset?p={query}"))
            .body(Vec::new())
            .unwrap();
        handle.respond(&request)
    };
    assert_eq!(served(&encoded).status(), 200);
    assert_eq!(served("%2Fetc%2Fpasswd").status(), 403);
    let index = Request::builder().uri(PAGE_URL).body(Vec::new()).unwrap();
    assert_eq!(handle.respond(&index).status(), 200);

    // Once every asset reports ready, the frame is submitted.
    for load in &loads {
        from_page(
            &mut host,
            &state,
            json!({"type":"asset.state","id":load["asset"]["id"],"state":"ready","meta":{}}),
        );
    }
    assert!(kinds(&sent(&mut host)).contains(&"frame.render"));
}

#[test]
fn replacing_the_project_forgets_loaded_assets_and_forbids_their_files() {
    let mut state = EditorState::demo();
    let mut host = ready_host("replace", &state);
    let loads = sent(&mut host);
    for load in loads.iter().filter(|m| m["type"] == "asset.load") {
        from_page(
            &mut host,
            &state,
            json!({"type":"asset.state","id":load["asset"]["id"],"state":"ready","meta":{}}),
        );
    }
    sent(&mut host);
    create_project(&mut state, None, &ProjectSettings::default()).unwrap();
    host.reset_project();
    host.sync(&state);
    let after = sent(&mut host);
    assert!(
        after.iter().any(|m| m["type"] == "asset.release"),
        "the old project's textures are released: {:?}",
        kinds(&after)
    );
    let handle = host.protocol_handle();
    let request = Request::builder()
        .uri("tessel://localhost/asset?p=%2Fnot%2Fregistered.png")
        .body(Vec::new())
        .unwrap();
    assert_eq!(handle.respond(&request).status(), 403);
}

#[test]
fn source_preview_asks_for_one_layer_apart_from_the_sequence() {
    let mut state = EditorState::demo();
    let mut host = ready_host("source", &state);
    let loads = sent(&mut host);
    for load in loads.iter().filter(|m| m["type"] == "asset.load") {
        from_page(
            &mut host,
            &state,
            json!({"type":"asset.state","id":load["asset"]["id"],"state":"ready","meta":{}}),
        );
    }
    let sequence_frame = sent(&mut host)
        .into_iter()
        .find(|m| m["type"] == "frame.render")
        .unwrap();
    assert!(sequence_frame["plan"]["layers"].as_array().unwrap().len() > 1);

    state
        .execute(Command::SetPreview {
            intent: PreviewIntent::Source {
                asset: Id::new("ao01").unwrap(),
                time: Ratio::ZERO,
            },
        })
        .unwrap();
    host.sync(&state);
    let source_frame = sent(&mut host)
        .into_iter()
        .find(|m| m["type"] == "frame.render")
        .unwrap();
    assert_eq!(source_frame["plan"]["layers"].as_array().unwrap().len(), 1);
}

// ---- scrub: a slow page must keep presenting and converge on the last position ----

/// The page answers the frame it was last asked for, as it does once its render finishes.
fn finish_render(host: &mut ViewportHost, state: &EditorState, render: (u64, u64, i64)) {
    from_page(host, state, frame_done(render.0, render.1));
}

#[test]
fn a_scrub_against_a_slow_page_presents_intermediate_frames_and_ends_on_the_last_position() {
    let mut state = titled_state("Titulo");
    let mut host = ready_host("scrub", &state);
    let mut outgoing = sent(&mut host);
    let mut in_flight = renders(&outgoing).pop().expect("the first frame");
    let mut every_message = outgoing.clone();

    // 90 playhead moves (a 3 s drag at 30 Hz); the page finishes one frame every 6 moves.
    for frame in 1..=90 {
        seek(&mut state, frame);
        host.sync(&state);
        outgoing = sent(&mut host);
        assert!(
            outgoing.is_empty(),
            "move {frame}: the frame in flight is not cancelled nor replaced: {:?}",
            kinds(&outgoing)
        );
        if frame % 6 == 0 {
            finish_render(&mut host, &state, in_flight);
            outgoing = sent(&mut host);
            let next = renders(&outgoing)
                .pop()
                .expect("the newest position is requested as soon as a frame is presented");
            assert_eq!(
                next.2, frame,
                "it is the newest position, never an older one"
            );
            in_flight = next;
            every_message.extend(outgoing);
        }
    }
    let during = host.report()["session"]["stats"].clone();
    assert_eq!(
        during["accepted"], 15,
        "15 frames were presented during the drag"
    );
    assert_eq!(during["cancelled"], 0);
    assert_eq!(during["dropped_stale"], 0);
    assert!(
        !kinds(&every_message).contains(&"frame.cancel"),
        "no cancellation reached the runtime: {:?}",
        kinds(&every_message)
    );
    assert!(
        every_message
            .iter()
            .filter(|message| message["type"] == "frame.render")
            .all(|message| message["inspect"] == json!(false)),
        "the monitor's frames skip the readback"
    );

    // Release on frame 95: the frame in flight finishes, then the last position is drawn.
    seek(&mut state, 95);
    host.sync(&state);
    assert!(sent(&mut host).is_empty());
    finish_render(&mut host, &state, in_flight);
    let last = renders(&sent(&mut host)).pop().expect("the final position");
    assert_eq!(last.2, 95);
    finish_render(&mut host, &state, last);
    assert!(sent(&mut host).is_empty(), "nothing left to catch up on");
    let report = host.report();
    assert_eq!(report["last_frame"]["request"], last.0);
    assert_eq!(report["session"]["stats"]["cancelled"], 0);
    assert_eq!(report["session"]["status"], "ready");
    assert_eq!(report["session"]["pending"], false);
    assert_eq!(report["session"]["stats"]["coalesced"], 91);
    assert_eq!(
        report["session"]["presented"],
        json!({"request": last.0, "frame": 95}),
        "the picture on screen is the playhead the user released on"
    );
    assert!(host.failure().is_none());
}

#[test]
fn an_edit_during_a_scrub_cancels_the_frame_in_flight_and_its_answer_is_never_shown() {
    let mut state = titled_state("Antes");
    let mut host = ready_host("scrub-edit", &state);
    let first = renders(&sent(&mut host)).pop().unwrap();
    seek(&mut state, 10);
    host.sync(&state);
    assert!(sent(&mut host).is_empty());

    retitle(&mut state, "Depois");
    host.sync(&state);
    let messages = sent(&mut host);
    assert_eq!(kinds(&messages), ["frame.cancel", "frame.render"]);
    let second = renders(&messages).pop().unwrap();
    assert_eq!(
        second.2, 10,
        "the edit is drawn at the playhead the user is on"
    );

    finish_render(&mut host, &state, first);
    assert_eq!(host.report()["session"]["stats"]["dropped_stale"], 1);
    assert!(host.report()["last_frame"].is_null());
    finish_render(&mut host, &state, second);
    assert_eq!(host.report()["last_frame"]["request"], second.0);
}

#[test]
fn replacing_the_project_during_a_scrub_forgets_the_pending_position() {
    let mut state = titled_state("Antes");
    let mut host = ready_host("scrub-replace", &state);
    let first = renders(&sent(&mut host)).pop().unwrap();
    seek(&mut state, 40);
    host.sync(&state);
    assert_eq!(host.report()["session"]["pending"], true);

    create_project(&mut state, None, &ProjectSettings::default()).unwrap();
    host.reset_project();
    assert_eq!(host.report()["session"]["pending"], false);
    sent(&mut host);
    // The old project's frame answers late: it belongs to a project that is gone.
    finish_render(&mut host, &state, first);
    assert!(host.report()["last_frame"].is_null());
    assert!(
        renders(&sent(&mut host)).iter().all(|render| render.2 == 0),
        "the new project is drawn at its own playhead, not the old pending position"
    );
}

#[test]
fn reloading_the_page_during_a_scrub_drops_what_was_pending() {
    let mut state = titled_state("Antes");
    let mut host = ready_host("scrub-reload", &state);
    let first = renders(&sent(&mut host)).pop().unwrap();
    seek(&mut state, 40);
    host.sync(&state);
    assert!(host.begin_reload());
    assert_eq!(host.report()["session"]["pending"], false);
    from_page(&mut host, &state, json!({"type":"page.ready"}));
    sent(&mut host);
    from_page(&mut host, &state, ready_message());
    // The new page draws the playhead the user is on, once.
    let fresh = renders(&sent(&mut host));
    assert_eq!(fresh.len(), 1, "{fresh:?}");
    assert_eq!(fresh[0].2, 40);
    // A late answer of the page that was reloaded is not the picture of the new one.
    finish_render(&mut host, &state, first);
    assert!(host.report()["last_frame"].is_null());
    finish_render(&mut host, &state, fresh[0]);
    assert_eq!(host.report()["last_frame"]["request"], fresh[0].0);
}

#[test]
fn a_frame_that_fails_mid_scrub_is_reported_and_the_next_move_recovers() {
    let mut state = titled_state("Titulo");
    let mut host = ready_host("scrub-fail", &state);
    let first = renders(&sent(&mut host)).pop().unwrap();
    seek(&mut state, 20);
    host.sync(&state);
    from_page(
        &mut host,
        &state,
        json!({"type":"frame.failed","id":first.0,"revision":first.1,
            "error":{"code":"render_failed","message":"decode"}}),
    );
    assert_eq!(host.headline(), "Falha: render_failed");
    assert!(
        sent(&mut host).is_empty(),
        "a failure is shown, not hidden by silently retrying the pending position"
    );
    seek(&mut state, 21);
    host.sync(&state);
    let retry = renders(&sent(&mut host))
        .pop()
        .expect("the next move retries");
    assert_eq!(retry.2, 21);
    finish_render(&mut host, &state, retry);
    assert!(host.failure().is_none());
}

#[test]
fn an_explicit_frame_during_a_scrub_keeps_its_data_and_the_scrub_resumes_after_it() {
    let mut state = titled_state("Titulo");
    let mut host = ready_host("scrub-explicit", &state);
    let first = renders(&sent(&mut host)).pop().unwrap();
    seek(&mut state, 30);
    host.sync(&state);
    assert!(sent(&mut host).is_empty());

    let frame = RenderFrame {
        plan: PlanRequest::of(&state),
        out: None,
        size: Some((64, 36)),
        probes: vec![(1, 1)],
    };
    let waiter = host.render_frame(&state, &frame).unwrap();
    let messages = sent(&mut host);
    assert_eq!(kinds(&messages), ["frame.cancel", "frame.render"]);
    assert_eq!(messages[1]["probes"], json!([[1, 1]]));
    assert!(
        messages[1].get("inspect").is_none(),
        "an explicit request is the message it always was"
    );
    let explicit = renders(&messages).pop().unwrap();
    assert_eq!(explicit.2, 30);

    // The scrub moves on while the explicit frame renders: that frame is superseded
    // by the monitor's, exactly as before this scheduler existed.
    seek(&mut state, 31);
    host.sync(&state);
    assert_eq!(waiter.try_recv().unwrap().unwrap_err().code, "cancelled");
    finish_render(&mut host, &state, first);
    finish_render(&mut host, &state, explicit);
    assert_eq!(host.report()["session"]["stats"]["dropped_stale"], 2);
}

#[test]
fn a_seek_while_an_explicit_frame_renders_answers_its_waiter_as_cancelled() {
    let mut state = titled_state("Titulo");
    let mut host = ready_host("scrub-explicit-ok", &state);
    sent(&mut host);
    let frame = RenderFrame {
        plan: PlanRequest::of(&state),
        out: None,
        size: Some((64, 36)),
        probes: vec![(1, 1)],
    };
    let waiter = host.render_frame(&state, &frame).unwrap();
    let explicit = renders(&sent(&mut host)).pop().unwrap();
    // A seek while the explicit frame renders supersedes it (explicit frames are never
    // coalesced into): the waiter is told, not left hanging.
    seek(&mut state, 5);
    host.sync(&state);
    assert_eq!(waiter.try_recv().unwrap().unwrap_err().code, "cancelled");
    let scrub = renders(&sent(&mut host)).pop().unwrap();
    finish_render(&mut host, &state, explicit);
    finish_render(&mut host, &state, scrub);
    assert_eq!(host.report()["last_frame"]["request"], scrub.0);
}

#[test]
fn the_monitor_can_be_asked_to_inspect_its_frames_for_measurement() {
    let state = titled_state("Titulo");
    let mut host = ViewportHost::new(Ok(page("inspect")));
    host.set_inspect_previews(true);
    from_page(&mut host, &state, json!({"type":"page.ready"}));
    sent(&mut host);
    from_page(&mut host, &state, ready_message());
    let messages = sent(&mut host);
    assert_eq!(kinds(&messages), ["frame.render"]);
    assert!(messages[0].get("inspect").is_none(), "{:?}", messages[0]);
    assert_eq!(host.report()["session"]["inspect_previews"], true);
}

#[test]
fn a_slow_page_does_not_hold_the_monitor_forever() {
    let mut state = titled_state("Titulo");
    let mut host = ready_host("scrub-timeout", &state);
    sent(&mut host);
    seek(&mut state, 3);
    host.sync(&state);
    assert!(host.tick(Instant::now() + Duration::from_secs(31)));
    assert_eq!(host.failure().unwrap().code, "timeout");
    assert_eq!(host.report()["session"]["pending"], false);
}

#[test]
fn the_handshake_asks_for_nothing_extra_unless_the_host_opted_in() {
    let state = empty_state();
    let mut plain = ViewportHost::with_options(Ok(page("hello-plain")), HostOptions::default());
    from_page(&mut plain, &state, json!({"type":"page.ready"}));
    assert_eq!(
        sent(&mut plain)[0],
        json!({"v":1,"type":"hello","host":"tessel-editor"}),
        "a runtime or host that predates the options sees the message it always saw"
    );

    let options = HostOptions {
        trace: true,
        retain: Some(false),
        preview_scene_samples: Some(0),
        preview_sort: Some(PreviewSort::Exact),
        ..HostOptions::default()
    };
    let mut host = ViewportHost::with_options(Ok(page("hello-opts")), options);
    from_page(&mut host, &state, json!({"type":"page.ready"}));
    let hello = &sent(&mut host)[0];
    assert_eq!(hello["retain"], false);
    assert_eq!(hello["telemetry"]["presented"], true);
    assert_eq!(hello["preview_scene_samples"], 0);
    assert_eq!(hello["preview_sort"], "exact");
    assert!(
        hello.get("scene_samples").is_none(),
        "only what the host asked for"
    );
}

fn frame_done_with_source(id: u64, revision: u64, source: Value) -> Value {
    let mut done = frame_done(id, revision);
    done["source"] = source;
    done
}

#[test]
fn a_scene_drawn_with_fewer_samples_is_labelled_and_replaced_by_the_final_picture_on_its_own() {
    let state = titled_state("Titulo");
    let mut host = ready_host("msaa-refine", &state);
    let first = sent(&mut host).remove(0);
    assert_eq!(
        first["quality"], "preview",
        "the monitor's own frame is the interactive quality"
    );
    from_page(
        &mut host,
        &state,
        frame_done_with_source(
            first["id"].as_u64().unwrap(),
            first["revision"].as_u64().unwrap(),
            json!({"quality":"preview","proxy_layers":0,"original_layers":0,"result_hits":0,
                "scene_samples":2,"reduced_scene_layers":1}),
        ),
    );
    // The quality is never hidden, and nothing is left for the user to ask.
    assert!(
        host.headline().ends_with(" · MSAA 2"),
        "{}",
        host.headline()
    );
    assert!(host.refine_pending());
    assert!(
        !host.refine(&state, Instant::now()),
        "not before the monitor has been still"
    );
    assert!(host.refine(&state, Instant::now() + Duration::from_millis(400)));
    let refine = sent(&mut host).remove(0);
    assert_eq!(refine["type"], "frame.render");
    assert!(
        refine.get("quality").is_none(),
        "the refine is a frame without `quality`: the final samples, byte for byte an explicit frame's request"
    );
    assert_eq!(refine["plan"], first["plan"], "the same position");
    from_page(
        &mut host,
        &state,
        frame_done_with_source(
            refine["id"].as_u64().unwrap(),
            refine["revision"].as_u64().unwrap(),
            json!({"quality":"original","proxy_layers":0,"original_layers":0,"result_hits":0,
                "scene_samples":4,"reduced_scene_layers":0}),
        ),
    );
    assert!(!host.headline().contains("MSAA"), "{}", host.headline());
    assert!(!host.refine_pending(), "the monitor is not stuck refining");
    assert!(!host.refine(&state, Instant::now() + Duration::from_secs(1)));
    assert_eq!(host.report()["session"]["stats"]["refined"], 1);
}

#[test]
fn a_moving_playhead_keeps_drawing_reduced_frames_and_only_stopping_refines() {
    let mut state = titled_state("Titulo");
    let mut host = ready_host("msaa-play", &state);
    let reduced = |id: u64, revision: u64| {
        frame_done_with_source(
            id,
            revision,
            json!({"quality":"preview","proxy_layers":0,"original_layers":0,"result_hits":0,
                "scene_samples":0,"reduced_scene_layers":1}),
        )
    };
    let first = sent(&mut host).remove(0);
    from_page(
        &mut host,
        &state,
        reduced(
            first["id"].as_u64().unwrap(),
            first["revision"].as_u64().unwrap(),
        ),
    );
    // Playback: every tick moves the playhead; the frame it asks for is reduced and the refine of
    // the previous picture is dropped, so nothing final-quality is drawn while it plays.
    for frame in 1..=5 {
        seek(&mut state, frame);
        host.sync(&state);
        let request = sent(&mut host)
            .into_iter()
            .find(|message| message["type"] == "frame.render")
            .expect("a new position is drawn");
        assert_eq!(request["quality"], "preview");
        from_page(
            &mut host,
            &state,
            reduced(
                request["id"].as_u64().unwrap(),
                request["revision"].as_u64().unwrap(),
            ),
        );
        assert!(
            host.headline().ends_with(" · MSAA 0"),
            "{}",
            host.headline()
        );
    }
    assert_eq!(host.report()["session"]["stats"]["refined"], 0);
    // It stops: one final frame, of the position on screen.
    assert!(host.refine(&state, Instant::now() + Duration::from_millis(400)));
    let refine = sent(&mut host).remove(0);
    assert_eq!(refine["plan"]["frame"], 5);
    assert!(refine.get("quality").is_none());
}

#[test]
fn the_retention_budget_reads_mebibytes_and_ignores_what_it_cannot_use() {
    use super::viewport_host::retained_bytes_from_env as read;
    assert_eq!(read(None), None);
    assert_eq!(read(Some("64")), Some(64 << 20));
    assert_eq!(read(Some(" 8 ")), Some(8 << 20));
    assert_eq!(
        read(Some("0")),
        None,
        "zero would release everything: keep the default"
    );
    assert_eq!(read(Some("lots")), None);
    assert_eq!(read(Some("-4")), None);
    assert_eq!(
        read(Some("18446744073709551615")),
        None,
        "an overflow keeps the default"
    );
}

#[test]
fn an_unreadable_preview_sort_is_the_safe_order_and_says_so() {
    use super::viewport_host::preview_sort_from_env as read;
    assert_eq!(read(None), Ok(None), "unset keeps the runtime's default");
    assert_eq!(read(Some("")), Ok(None));
    assert_eq!(read(Some("  ")), Ok(None));
    assert_eq!(read(Some("latest")), Ok(Some(PreviewSort::Latest)));
    assert_eq!(read(Some(" Exact ")), Ok(Some(PreviewSort::Exact)));
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let invalid = std::ffi::OsStr::from_bytes(b"exact\xff");
        let (sort, _) = read(Some(&invalid.to_string_lossy())).unwrap_err();
        assert_eq!(sort, PreviewSort::Exact);
    }
    // A typo of a rollback must not leave the reused order on: it is `exact`, with the reason to print.
    for typo in ["exacto", "fast", "0", "on"] {
        let (sort, warning) = read(Some(typo)).expect_err(typo);
        assert_eq!(sort, PreviewSort::Exact, "{typo}");
        assert!(
            warning.contains("TESSEL_VIEWPORT_PREVIEW_SORT"),
            "{warning}"
        );
        assert!(warning.contains(typo), "{warning}");
        assert!(warning.contains("`exact`"), "{warning}");
    }
}

#[test]
fn splats_painted_in_an_earlier_cameras_order_are_labelled_and_replaced_by_the_exact_picture() {
    let state = titled_state("Titulo");
    let mut host = ready_host("sort-refine", &state);
    let first = sent(&mut host).remove(0);
    assert_eq!(first["quality"], "preview");
    // Full MSAA, no proxy: the reused order is the only way this picture differs from the final one.
    from_page(
        &mut host,
        &state,
        frame_done_with_source(
            first["id"].as_u64().unwrap(),
            first["revision"].as_u64().unwrap(),
            json!({"quality":"preview","proxy_layers":0,"original_layers":0,"result_hits":0,
                "scene_samples":4,"reduced_scene_layers":0,"stale_sort_layers":1}),
        ),
    );
    assert!(
        host.headline().ends_with(" · sort"),
        "the quality is never hidden: {}",
        host.headline()
    );
    assert!(host.refine_pending());
    assert!(
        !host.refine(&state, Instant::now()),
        "not before the monitor has been still"
    );
    assert!(host.refine(&state, Instant::now() + Duration::from_millis(400)));
    let refine = sent(&mut host).remove(0);
    assert!(
        refine.get("quality").is_none(),
        "the refine is a frame without `quality`: the runtime waits for the sort of its camera"
    );
    assert_eq!(refine["plan"], first["plan"], "the same position");
    from_page(
        &mut host,
        &state,
        frame_done_with_source(
            refine["id"].as_u64().unwrap(),
            refine["revision"].as_u64().unwrap(),
            json!({"quality":"original","proxy_layers":0,"original_layers":0,"result_hits":0,
                "scene_samples":4,"reduced_scene_layers":0,"stale_sort_layers":0}),
        ),
    );
    assert!(!host.headline().contains("sort"), "{}", host.headline());
    assert!(!host.refine_pending());
    assert_eq!(host.report()["session"]["stats"]["refined"], 1);
}

#[test]
fn a_traced_frame_reports_the_host_spans_and_presented_is_not_a_second_frame() {
    let state = titled_state("Titulo");
    let options = HostOptions {
        trace: true,
        retain: None,
        ..HostOptions::default()
    };
    let mut host = ViewportHost::with_options(Ok(page("trace")), options);
    from_page(&mut host, &state, json!({"type":"page.ready"}));
    sent(&mut host);
    from_page(&mut host, &state, ready_message());
    let messages = sent(&mut host);
    let (id, revision, _) = renders(&messages)[0];
    from_page(&mut host, &state, frame_done(id, revision));
    from_page(
        &mut host,
        &state,
        json!({"type":"frame.presented","id":id,"revision":revision,"after_done_ms":7.0}),
    );

    let trace = host.report()["session"]["trace"].clone();
    assert_eq!(trace["frames_total"], 1);
    assert_eq!(trace["recent"][0]["id"], id);
    assert_eq!(trace["recent"][0]["presented_after_done_ms"], 7.0);
    for span in [
        "plan",
        "plan_key",
        "sync",
        "ipc",
        "serialize",
        "dispatch",
        "transit",
    ] {
        assert!(
            trace["spans"][span].is_object(),
            "missing span `{span}`: {trace}"
        );
    }
    assert_eq!(host.report()["session"]["stats"]["accepted"], 1);
    assert!(host.failure().is_none());
}

#[test]
fn spans_are_not_recorded_when_tracing_is_off() {
    let state = titled_state("Titulo");
    let mut host = ready_host("trace-off", &state);
    sent(&mut host);
    assert_eq!(host.report()["session"]["trace"], json!({"enabled": false}));
    assert_eq!(host.trace().snapshot(0)["spans"], json!({}));
}

#[test]
fn a_presentation_report_changes_nothing_the_window_shows_but_a_new_picture_does() {
    let mut state = titled_state("Titulo");
    let mut host = ready_host("ui-signature", &state);
    let (id, revision, _) = renders(&sent(&mut host))[0];
    from_page(&mut host, &state, frame_done(id, revision));
    let settled = host.ui_signature();

    // OPT04: the page saying the picture reached a rendering opportunity is not something GPUI
    // draws, so the window has no reason to repaint for it.
    from_page(
        &mut host,
        &state,
        json!({"type":"frame.presented","id":id,"revision":revision,"after_done_ms":9.0}),
    );
    assert_eq!(host.ui_signature(), settled);

    host.note_ui(false);
    host.note_ui(true);
    assert_eq!(host.report()["ui"], json!({"repaints": 1, "skipped": 1}));

    // A failed frame, on the other hand, changes the header: the window must repaint.
    seek(&mut state, 5);
    host.sync(&state);
    let (next, next_revision, _) = renders(&sent(&mut host))[0];
    from_page(
        &mut host,
        &state,
        json!({"type":"frame.failed","id":next,"revision":next_revision,
               "error":{"code":"render_failed","message":"boom"}}),
    );
    assert_ne!(host.ui_signature(), settled);
}

#[test]
fn changing_the_preview_quality_redraws_the_monitor_and_says_which_quality_it_asked_for() {
    let mut state = titled_state("Titulo");
    let mut host = ready_host("quality", &state);
    let first = sent(&mut host);
    assert_eq!(
        first.iter().find(|m| m["type"] == "frame.render").unwrap()["quality"],
        "preview",
        "the monitor's own frame may use a proxy under the default policy"
    );
    let (id, revision, _) = renders(&first)[0];
    from_page(&mut host, &state, frame_done(id, revision));
    assert!(renders(&sent(&mut host)).is_empty());

    state
        .execute(Command::SetPreviewPolicy {
            mode: Some(tessel_editor_core::preview::PreviewMode::Original),
            max_height: None,
            format: None,
        })
        .unwrap();
    host.sync(&state);
    let messages = sent(&mut host);
    assert_eq!(
        renders(&messages).len(),
        1,
        "same plan, other policy: a new picture"
    );
    let render = messages
        .iter()
        .find(|m| m["type"] == "frame.render")
        .unwrap();
    assert!(
        render.get("quality").is_none(),
        "the original policy asks for the original, the field absent"
    );
    assert_eq!(
        host.report()["session"]["preview_policy"]["mode"],
        "original"
    );
}

/// Directed scene fixtures cross the media verification seam before reaching the WebView.
fn directed_host(
    tag: &str,
) -> (
    ViewportHost,
    EditorState,
    tessel_editor_media::scene_resources::SceneResourceSession,
) {
    use tessel_editor_core::headless::execute_ext;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/directed-scene-proof/corpus")
        .canonicalize()
        .unwrap();
    let mut state = empty_state();
    let mut media = tessel_editor_media::host::MediaHost::new();
    execute_ext(
        &mut state,
        None,
        Some(&mut media),
        &json!({"op":"scene.attach","id":"proof","path":root.join("bundle.json")}),
    )
    .unwrap();
    execute_ext(
        &mut state,
        None,
        Some(&mut media),
        &json!({"op":"scene.resources.resolve","scene":"proof","base":root}),
    )
    .unwrap();
    let mut host = ViewportHost::new(Ok(page(tag)));
    from_page(&mut host, &state, json!({"type":"page.ready"}));
    sent(&mut host);
    let mut ready = ready_message();
    ready["capabilities"]["features"] = json!(["directed_scene"]);
    ready["capabilities"]["directed_scene"]["capabilities"] =
        json!(tessel_editor_core::scene_plan::Capability::ALL.map(|c| c.as_str()));
    from_page(&mut host, &state, ready);
    sent(&mut host);
    (host, state, media.scene_resources())
}

fn confirm_scene_assets(host: &mut ViewportHost, state: &EditorState) -> Vec<Value> {
    let loads = sent(host);
    for message in loads.iter().filter(|m| m["type"] == "asset.load") {
        from_page(
            host,
            state,
            json!({"type":"asset.state","id":message["asset"]["id"],"state":"ready","meta":{}}),
        );
    }
    sent(host)
}

#[test]
fn directed_frames_use_verified_files_and_wait_for_asset_acknowledgements() {
    let (mut host, mut state, resources) = directed_host("directed-ready");
    let waiter = host
        .render_scene(
            &mut state,
            &resources,
            &json!({"scene":"proof","frame":20,"width":320,"height":180}),
            true,
        )
        .unwrap()
        .unwrap();
    let renders = confirm_scene_assets(&mut host, &state);
    let message = renders
        .iter()
        .find(|m| m["type"] == "frame.render")
        .unwrap();
    assert_eq!(message["plan"]["layers"][0]["kind"], "directed_scene");
    assert_eq!(message["plan"]["frame"], 20);
    from_page(
        &mut host,
        &state,
        frame_done(
            message["id"].as_u64().unwrap(),
            message["revision"].as_u64().unwrap(),
        ),
    );
    assert!(waiter.try_recv().unwrap().is_ok());
    assert_eq!(host.report()["directed_scene"], "proof");
}

#[test]
fn directed_interactive_seeks_coalesce_and_explicit_requests_are_cancelled_on_project_reset() {
    let (mut host, mut state, resources) = directed_host("directed-coalesce");
    let waiter = host
        .render_scene(
            &mut state,
            &resources,
            &json!({"scene":"proof","frame":0}),
            true,
        )
        .unwrap()
        .unwrap();
    let first = confirm_scene_assets(&mut host, &state)
        .into_iter()
        .find(|m| m["type"] == "frame.render")
        .unwrap();
    for frame in [11, 20, 36] {
        host.render_scene(
            &mut state,
            &resources,
            &json!({"scene":"proof","frame":frame}),
            false,
        )
        .unwrap();
    }
    assert!(sent(&mut host).is_empty());
    from_page(
        &mut host,
        &state,
        frame_done(
            first["id"].as_u64().unwrap(),
            first["revision"].as_u64().unwrap(),
        ),
    );
    assert!(waiter.try_recv().unwrap().is_ok());
    let next = sent(&mut host)
        .into_iter()
        .find(|m| m["type"] == "frame.render")
        .unwrap();
    assert_eq!(next["plan"]["frame"], 36);
    let waiter = host
        .render_scene(
            &mut state,
            &resources,
            &json!({"scene":"proof","frame":48}),
            true,
        )
        .unwrap()
        .unwrap();
    host.reset_project();
    assert_eq!(waiter.try_recv().unwrap().unwrap_err().code, "cancelled");
    assert_eq!(host.directed_scene(), None);
    from_page(
        &mut host,
        &state,
        frame_done(
            next["id"].as_u64().unwrap(),
            next["revision"].as_u64().unwrap(),
        ),
    );
    assert!(host.report()["last_frame"].is_null());
}

#[test]
fn directed_output_rejection_leaves_the_monitor_and_history_unchanged() {
    let (mut host, mut state, resources) = directed_host("directed-reject");
    let before = state.summary();
    let error = host
        .render_scene(
            &mut state,
            &resources,
            &json!({"scene":"proof","frame":20,"width":0,"height":180}),
            true,
        )
        .unwrap_err();
    assert_eq!(error.code, "invalid_value");
    assert_eq!(state.summary(), before);
    assert_eq!(host.directed_scene(), None);
    assert!(sent(&mut host).is_empty());
}

// ---- the debug meter's feed (F049)

/// A host wired to a meter that is switched on, and the meter.
fn metered(
    tag: &str,
    state: &EditorState,
    on: bool,
) -> (
    ViewportHost,
    std::rc::Rc<std::cell::RefCell<tessel_editor_core::perf::Meter>>,
) {
    let mut host = ViewportHost::new(Ok(page(tag)));
    let meter = std::rc::Rc::new(std::cell::RefCell::new(
        tessel_editor_core::perf::Meter::default(),
    ));
    if on {
        meter.borrow_mut().enable(Duration::ZERO, None, None);
    }
    host.attach_meter(meter.clone(), super::perf_meter::PerfClock::wall());
    from_page(&mut host, state, json!({"type":"page.ready"}));
    let _ = sent(&mut host);
    from_page(&mut host, state, ready_message());
    (host, meter)
}

#[test]
fn the_meter_counts_pictures_the_host_accepted_and_nothing_else() {
    let mut state = titled_state("Um");
    let (mut host, meter) = metered("meter-feed", &state, true);
    let first = sent(&mut host);
    let first_id = first[0]["id"].as_u64().unwrap();
    let first_revision = first[0]["revision"].as_u64().unwrap();
    assert_eq!(
        meter.borrow().samples(),
        (0, 0),
        "a request is not a picture"
    );

    // An edit supersedes the frame in flight; the late answer for the old one is dropped.
    retitle(&mut state, "Dois");
    host.sync(&state);
    let second = sent(&mut host);
    let second_id = second[1]["id"].as_u64().unwrap();
    from_page(
        &mut host,
        &state,
        frame_done(second_id, second[1]["revision"].as_u64().unwrap()),
    );
    assert_eq!(meter.borrow().samples(), (0, 1));
    from_page(&mut host, &state, frame_done(first_id, first_revision));
    assert_eq!(
        meter.borrow().samples(),
        (0, 1),
        "a stale answer was never shown"
    );

    // The page saying the picture reached a rendering opportunity is not a new picture.
    from_page(
        &mut host,
        &state,
        json!({"type":"frame.presented","id":second_id,"revision":second[1]["revision"],
               "after_done_ms":9.0}),
    );
    assert_eq!(meter.borrow().samples(), (0, 1));
}

#[test]
fn the_meter_reads_a_stale_sort_picture_and_its_exact_refine_as_two_pictures_of_one_frame() {
    use tessel_editor_core::perf::{FrameContext, LossReading};
    let state = titled_state("Um");
    let (mut host, meter) = metered("meter-sort-refine", &state, true);
    // Playback is running from frame 0, with nothing known to be on screen yet.
    meter
        .borrow_mut()
        .playback_started(Duration::ZERO, 0, host.presented_frame());
    let first = sent(&mut host).remove(0);
    from_page(
        &mut host,
        &state,
        frame_done_with_source(
            first["id"].as_u64().unwrap(),
            first["revision"].as_u64().unwrap(),
            json!({"quality":"preview","proxy_layers":0,"original_layers":0,"result_hits":0,
                "scene_samples":4,"reduced_scene_layers":0,"stale_sort_layers":1}),
        ),
    );
    assert_eq!(meter.borrow().samples(), (0, 1));
    assert_eq!(host.presented_frame(), Some(0));
    let preview = FrameContext::of(host.last_frame().unwrap());
    assert_eq!(preview.stale_sort_layers, 1);
    assert_eq!(
        preview.quality_label(),
        "preview + sort",
        "the card never shows a reused sort order as the final picture"
    );

    // The refine of the same frame is a second accepted picture: the monitor rate counts it, the
    // dropped-frame count does not (frame 0 was presented once, and nothing was skipped).
    assert!(host.refine(&state, Instant::now() + Duration::from_millis(400)));
    let refine = sent(&mut host).remove(0);
    from_page(
        &mut host,
        &state,
        frame_done_with_source(
            refine["id"].as_u64().unwrap(),
            refine["revision"].as_u64().unwrap(),
            json!({"quality":"original","proxy_layers":0,"original_layers":0,"result_hits":0,
                "scene_samples":4,"reduced_scene_layers":0,"stale_sort_layers":0}),
        ),
    );
    assert_eq!(meter.borrow().samples(), (0, 2), "the refine is a picture");
    let exact = FrameContext::of(host.last_frame().unwrap());
    assert_eq!(exact.quality_label(), "original");
    match meter
        .borrow()
        .readout(Duration::from_secs(1), true, None, None)
        .loss
    {
        LossReading::Live {
            presented_frames,
            lost,
            span_frames,
            ..
        } => assert_eq!(
            (presented_frames, lost, span_frames),
            (1, 0, 1),
            "the refine of a frame is not a second presented frame"
        ),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_stale_answer_after_a_refine_is_not_a_picture_for_the_meter() {
    let state = titled_state("Um");
    let (mut host, meter) = metered("meter-stale-refine", &state, true);
    let first = sent(&mut host).remove(0);
    let (first_id, first_revision) = (
        first["id"].as_u64().unwrap(),
        first["revision"].as_u64().unwrap(),
    );
    from_page(&mut host, &state, frame_done(first_id, first_revision));
    assert_eq!(meter.borrow().samples(), (0, 1));
    // A late copy of the same answer (the request is no longer in flight) is ignored.
    from_page(&mut host, &state, frame_done(first_id, first_revision));
    assert_eq!(
        meter.borrow().samples(),
        (0, 1),
        "a picture the host did not accept is not counted"
    );
}

/// The meter's reading of the dropped-frame count after a playback started at frame 0 with
/// `on_screen` known to be showing (`presented_frames`, `lost`, `span_frames`).
fn loss_after_starting_at_zero(
    meter: &std::rc::Rc<std::cell::RefCell<tessel_editor_core::perf::Meter>>,
    on_screen: Option<i64>,
) -> (u64, u64, u64) {
    use tessel_editor_core::perf::LossReading;
    meter
        .borrow_mut()
        .playback_started(Duration::from_secs(1), 0, on_screen);
    let reading = match meter
        .borrow()
        .readout(Duration::from_secs(2), true, None, None)
        .loss
    {
        LossReading::Live {
            presented_frames,
            lost,
            span_frames,
            ..
        } => (presented_frames, lost, span_frames),
        other => panic!("{other:?}"),
    };
    // A second start of a running playback is ignored: end this one for the next reading.
    meter.borrow_mut().playback_stopped(Duration::from_secs(2));
    reading
}

#[test]
fn replacing_the_project_or_reloading_the_viewport_forgets_the_picture_on_screen() {
    let state = titled_state("Um");
    let (mut host, meter) = metered("meter-forget", &state, true);
    let present = |host: &mut ViewportHost| {
        let (id, revision, _) = renders(&sent(host))[0];
        from_page(host, &state, frame_done(id, revision));
    };
    present(&mut host);
    assert_eq!(host.presented_frame(), Some(0));
    // The control: with frame 0 on screen, a playback from 0 counts it as presented.
    assert_eq!(
        loss_after_starting_at_zero(&meter, host.presented_frame()).0,
        1
    );

    // A new project: the monitor still has the old picture until a new one arrives, but what
    // it showed is no longer known, so the meter must not count frame 0 as presented.
    host.reset_project();
    assert_eq!(
        host.presented_frame(),
        None,
        "the project was replaced: no picture is known to be on screen"
    );
    assert_eq!(
        loss_after_starting_at_zero(&meter, host.presented_frame()).0,
        0
    );
    // The next picture of the new project is known again.
    host.sync(&state);
    present(&mut host);
    assert_eq!(host.presented_frame(), Some(0));

    // The same for a reload of the viewport page.
    assert!(host.begin_reload());
    assert_eq!(
        host.presented_frame(),
        None,
        "the page reloaded: nothing is on screen until its first new picture"
    );
    assert_eq!(
        loss_after_starting_at_zero(&meter, host.presented_frame()).0,
        0
    );
    from_page(&mut host, &state, json!({"type":"page.ready"}));
    let _ = sent(&mut host);
    from_page(&mut host, &state, ready_message());
    present(&mut host);
    assert_eq!(host.presented_frame(), Some(0));
}

#[test]
fn a_hidden_meter_keeps_no_samples_from_the_monitor() {
    let state = titled_state("Um");
    let (mut host, meter) = metered("meter-off", &state, false);
    let (id, revision, _) = renders(&sent(&mut host))[0];
    from_page(&mut host, &state, frame_done(id, revision));
    assert_eq!(host.report()["session"]["stats"]["accepted"], 1);
    assert_eq!(meter.borrow().samples(), (0, 0));
    assert_eq!(meter.borrow().next_wakeup(Duration::from_secs(9)), None);
}

#[test]
fn the_context_rows_come_from_the_last_finished_frame_and_msaa_is_absent_without_data() {
    let state = titled_state("Um");
    let (mut host, _meter) = metered("meter-context", &state, true);
    assert!(host.last_frame().is_none());
    let (id, revision, _) = renders(&sent(&mut host))[0];
    from_page(&mut host, &state, frame_done(id, revision));
    let context = tessel_editor_core::perf::FrameContext::of(host.last_frame().unwrap());
    assert_eq!((context.width, context.height), (4, 2));
    assert_eq!(context.msaa, None);
    assert_eq!(context.reduced_scene_layers, None);
}

// --- F055: the gizmo overlay ------------------------------------------------------------------

fn gizmo_ready_message() -> Value {
    let mut message = ready_message();
    message["capabilities"]["features"] = json!(["overlay.gizmo"]);
    message
}

fn gizmo_host(tag: &str, state: &EditorState) -> ViewportHost {
    let mut host = ViewportHost::new(Ok(page(tag)));
    from_page(&mut host, state, json!({"type":"page.ready"}));
    let _ = sent(&mut host);
    from_page(&mut host, state, gizmo_ready_message());
    assert!(host.overlay_supported());
    let _ = sent(&mut host); // the first frame the handshake asks for
    host
}

fn overlay_payload(origin: f64) -> Value {
    json!({"version":1,"visible":true,"entity":"scene/a/node/n","origin":{"u":origin,"v":0.5},
        "handles":[],"outlines":[],"active":null,"aspect":1.7777,
        "stamp":{"scene":"a","session":"s","revision":1,"frame":0}})
}

#[test]
fn the_overlay_is_never_sent_to_a_page_that_did_not_announce_it() {
    let state = empty_state();
    let mut host = ready_host("overlay-old", &state);
    let _ = sent(&mut host); // the first frame the handshake asks for
    assert!(!host.overlay_supported());
    assert!(!host.overlay_set_enabled(true), "nothing to switch on");
    assert!(!host.overlay_show(&overlay_payload(0.5)));
    assert!(sent(&mut host).is_empty());
    assert_eq!(host.report()["overlay"]["supported"], false);
    assert_eq!(host.report()["overlay"]["enabled"], false);
}

#[test]
fn the_overlay_is_switched_on_once_and_an_unchanged_overlay_is_not_resent() {
    let state = empty_state();
    let mut host = gizmo_host("overlay-on", &state);
    // Nothing is sent before it is switched on.
    assert!(!host.overlay_show(&overlay_payload(0.5)));
    assert!(host.overlay_set_enabled(true));
    assert!(!host.overlay_set_enabled(true), "already on");
    assert!(host.overlay_show(&overlay_payload(0.5)));
    assert!(
        !host.overlay_show(&overlay_payload(0.5)),
        "same JSON: not sent again"
    );
    assert!(host.overlay_show(&overlay_payload(0.6)));
    let messages = sent(&mut host);
    assert_eq!(
        kinds(&messages),
        ["overlay.enable", "overlay.set", "overlay.set"]
    );
    assert_eq!(messages[0]["enabled"], true);
    assert_eq!(messages[0]["v"], 1);
    assert_eq!(messages[2]["overlay"]["origin"]["u"], 0.6);
    assert_eq!(messages[1]["ack"], false, "the ack is opt-in");

    assert!(host.overlay_set_enabled(false));
    assert!(
        !host.overlay_show(&overlay_payload(0.7)),
        "off: nothing is sent"
    );
    assert_eq!(kinds(&sent(&mut host)), ["overlay.enable"]);
    // Back on, the first overlay is sent again even if it equals the last one before.
    assert!(host.overlay_set_enabled(true));
    assert!(host.overlay_show(&overlay_payload(0.6)));
    assert_eq!(host.report()["overlay"]["sets"], 3);
}

#[test]
fn what_the_page_reports_is_counted_and_a_reload_forgets_the_overlay() {
    let state = empty_state();
    let mut host = gizmo_host("overlay-report", &state);
    assert!(host.overlay_set_enabled(true));
    assert!(host.overlay_show(&overlay_payload(0.5)));
    for message in [
        json!({"type":"overlay.pointer","phase":"down","seq":1,"point":{"u":0.5,"v":0.5}}),
        json!({"type":"overlay.pointer","phase":"cancel","seq":2,"reason":"escape"}),
        json!({"type":"overlay.layout","pane":{"width":800,"height":600}}),
        json!({"type":"overlay.drawn","draws":1,"visible":true,"probes":[],
               "stamp":{"scene":"a","session":"s","revision":1,"frame":0}}),
    ] {
        assert!(host.overlay_note(&message), "{message}");
    }
    assert!(
        !host.overlay_note(&json!({"type":"frame.done"})),
        "not an overlay message"
    );
    let report = host.report()["overlay"].clone();
    assert_eq!(report["pointer_events"], 2);
    assert_eq!(report["last_phase"], "cancel");
    assert_eq!(report["layout"]["pane"]["width"], 800);
    assert_eq!(report["drawn"]["draws"], 1);
    assert_eq!(report["stale"], 0);

    // An ack of an overlay that is not the current one (a reload and a new project happened
    // since) is counted and never recorded as what the page shows now.
    assert!(
        host.overlay_show(&json!({
        "version":1,"visible":false,"entity":null,"origin":null,"handles":[],"outlines":[],
        "active":null,"aspect":1.7777,"stamp":{"scene":"a","session":"s","revision":2,"frame":0}}))
    );
    assert!(
        host.overlay_note(&json!({"type":"overlay.drawn","draws":2,"visible":true,
        "probes":[],"stamp":{"scene":"a","session":"s","revision":1,"frame":0}}))
    );
    let stale = host.report()["overlay"].clone();
    assert_eq!(stale["stale"], 1);
    assert_eq!(
        stale["drawn"]["draws"], 1,
        "the old ack did not replace the last good one"
    );

    host.begin_reload();
    let after = host.report()["overlay"].clone();
    assert_eq!(after["enabled"], false);
    assert_eq!(after["pointer_events"], 0);
    assert_eq!(after["supported"], false);
}

/// M2 of the PR #25 review: what the adapter spends on the overlay is gated and counted.
#[test]
fn the_overlay_gates_and_counters_are_the_hosts_and_a_reload_resets_them() {
    let state = empty_state();
    let mut host = gizmo_host("overlay-gates", &state);
    // Off: nothing is enabled, so the adapter has no reason to plan the scene.
    assert!(!host.overlay_enabled());
    assert!(!host.overlay_memo_hit("k"), "off: never a memo hit");
    assert!(!host.overlay_hold(), "off: nothing to clear");
    assert!(host.overlay_set_enabled(true));
    assert!(host.overlay_enabled());
    assert!(!host.overlay_memo_hit("k"), "nothing computed yet");
    host.overlay_computed("k".to_string());
    assert!(host.overlay_memo_hit("k"));
    assert!(
        !host.overlay_memo_hit("other"),
        "another state is not a hit"
    );
    host.overlay_adopted("adopted".to_string());
    assert!(host.overlay_memo_hit("adopted"));
    let report = host.report()["overlay"].clone();
    assert_eq!(
        (
            report["computed"].clone(),
            report["adopted"].clone(),
            report["memo_hits"].clone()
        ),
        (json!(1), json!(1), json!(2))
    );
    // Playback: cleared once, then nothing until a state is computed again.
    let _ = sent(&mut host);
    assert!(host.overlay_hold());
    assert!(!host.overlay_hold(), "held: not cleared again every tick");
    assert_eq!(kinds(&sent(&mut host)), ["overlay.clear"]);
    assert!(
        !host.overlay_memo_hit("adopted"),
        "the page holds nothing now"
    );
    assert_eq!(host.report()["overlay"]["held"], true);
    host.overlay_computed("again".to_string());
    assert_eq!(host.report()["overlay"]["held"], false);
}

/// M4 of the PR #25 review: every page incarnation has its own epoch, which the adapter puts in
/// the stamps it sends, so the queued messages of the page before a reload are never current.
#[test]
fn every_reload_of_the_page_is_a_new_overlay_epoch() {
    let state = empty_state();
    let mut host = gizmo_host("overlay-epoch", &state);
    let first = host.overlay_epoch();
    assert_eq!(host.overlay_epoch(), first, "reading it changes nothing");
    assert!(host.overlay_set_enabled(true));
    assert!(host.overlay_show(&overlay_payload(0.5)));
    assert_eq!(
        host.overlay_epoch(),
        first,
        "overlays of one page share its epoch"
    );
    assert!(host.begin_reload());
    let second = host.overlay_epoch();
    assert_ne!(second, first, "the reloaded page is another incarnation");
    assert_eq!(host.report()["overlay"]["epoch"], second);
    assert!(host.begin_reload());
    assert_ne!(host.overlay_epoch(), second, "and so is the next one");
}
