//! Lifecycle of the directed-scene monitor host (BL092) without a window: a scripted page answers on
//! the bridge, and the tests watch that every explicit request is answered exactly once, that old
//! answers never reach the current frame, and that the book of slots is rebuilt after the runtime
//! lost what it held.
//!
//! These are host-level tests with a scripted page. They do not replace the native proof
//! (`scripts/scene-lifecycle-proof.py`), which uses the real WKWebView and real `WEBGL_lose_context`.
//! The helpers repeat the small ones of `viewport_host_tests.rs` because that module is private.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use core::prelude::v1::test;
use serde_json::{Value, json};
use tessel_editor_core::headless::execute_ext;
use tessel_editor_core::scene_plan::Capability;
use tessel_editor_core::{EditorState, state::ProjectSettings};
use tessel_editor_media::host::MediaHost;
use tessel_editor_media::scene_resources::SceneResourceSession;

use super::super::ViewportHost;

fn page(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tessel-sl-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let dist = dir.join("dist");
    std::fs::create_dir_all(&dist).unwrap();
    std::fs::write(dist.join("index.html"), "<html>").unwrap();
    std::fs::write(dist.join("runtime.js"), "//").unwrap();
    dist
}

fn sent(host: &mut ViewportHost) -> Vec<Value> {
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

fn from_page(host: &mut ViewportHost, state: &EditorState, message: Value) {
    let mut message = message;
    message["v"] = json!(1);
    host.on_ipc(&message.to_string(), state);
}

fn ready_message() -> Value {
    json!({"type":"ready","runtime":{"name":"tessel-three","version":"0.1.0","renderer":"Test GPU"},
        "capabilities":{"layers":["image","video","text","scene","waveform"],"formats":{},
            "features":["directed_scene"],
            "directed_scene":{"capabilities":Capability::ALL.map(|c| c.as_str())}}})
}

fn frame_done(id: u64, revision: u64) -> Value {
    json!({"type":"frame.done","id":id,"revision":revision,"width":4,"height":2,"hash":"abcd",
        "mean":[1,2,3,255],"probes":[[9,8,7,255]],"png":null,
        "timings":{"render_ms":1.5,"total_ms":2.5}})
}

fn asset_ready(slot: &str) -> Value {
    json!({"type":"asset.state","id":slot,"state":"ready","meta":{}})
}

fn asset_failed(slot: &str, code: &str, details: Value) -> Value {
    json!({"type":"asset.state","id":slot,"state":"failed",
        "error":{"code":code,"message":format!("{code} (scripted)"),"details":details}})
}

/// A host whose page completed the handshake, and the verified resources of the corpus scene.
fn directed_host(tag: &str) -> (ViewportHost, EditorState, SceneResourceSession) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/directed-scene-proof/corpus")
        .canonicalize()
        .unwrap();
    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    let mut media = MediaHost::new();
    for request in [
        json!({"op":"scene.attach","id":"proof","path":root.join("bundle.json")}),
        json!({"op":"scene.resources.resolve","scene":"proof","base":root}),
    ] {
        execute_ext(&mut state, None, Some(&mut media), &request).unwrap();
    }
    let mut host = ViewportHost::new(Ok(page(tag)));
    start_page(&mut host, &state);
    (host, state, media.scene_resources())
}

/// `page.ready` then the handshake: what a freshly loaded page does.
fn start_page(host: &mut ViewportHost, state: &EditorState) {
    from_page(host, state, json!({"type":"page.ready"}));
    sent(host);
    from_page(host, state, ready_message());
    sent(host);
    assert!(host.is_ready());
}

fn frame_request(frame: i64) -> Value {
    json!({"scene":"proof","frame":frame,"width":320,"height":180})
}

/// Acknowledges every `asset.load` among `messages` and returns what the host sent next.
fn confirm_loads(host: &mut ViewportHost, state: &EditorState, messages: &[Value]) -> Vec<Value> {
    for load in messages.iter().filter(|m| m["type"] == "asset.load") {
        from_page(
            host,
            state,
            asset_ready(load["asset"]["id"].as_str().unwrap()),
        );
    }
    sent(host)
}

/// The `frame.render` the host sends once its slots are loaded: either already among `messages`
/// (everything was loaded) or after the page acknowledges the loads among them.
fn render_after_loads(host: &mut ViewportHost, state: &EditorState, messages: &[Value]) -> Value {
    if messages.iter().any(|m| m["type"] == "frame.render") {
        return frame_render(messages);
    }
    frame_render(&confirm_loads(host, state, messages))
}

fn frame_render(messages: &[Value]) -> Value {
    messages
        .iter()
        .find(|m| m["type"] == "frame.render")
        .unwrap_or_else(|| panic!("no frame.render in {messages:?}"))
        .clone()
}

fn complete(host: &mut ViewportHost, state: &EditorState, render: &Value) {
    from_page(
        host,
        state,
        frame_done(
            render["id"].as_u64().unwrap(),
            render["revision"].as_u64().unwrap(),
        ),
    );
}

#[test]
fn an_explicit_frame_still_draws_and_answers_while_the_monitor_is_hidden() {
    let (mut host, mut state, resources) = directed_host("hidden-explicit");
    host.on_screen = false;
    let waiter = host
        .render_scene(&mut state, &resources, &frame_request(20), true)
        .unwrap()
        .unwrap();
    let loads = sent(&mut host);
    assert!(
        loads.iter().any(|m| m["type"] == "asset.load"),
        "a hidden monitor must not park a request somebody waits for"
    );
    let render = frame_render(&confirm_loads(&mut host, &state, &loads));
    complete(&mut host, &state, &render);
    assert!(waiter.try_recv().unwrap().is_ok());
}

#[test]
fn hiding_the_monitor_with_a_frame_in_flight_still_answers_the_waiter() {
    let (mut host, mut state, resources) = directed_host("hide-in-flight");
    let waiter = host
        .render_scene(&mut state, &resources, &frame_request(20), true)
        .unwrap()
        .unwrap();
    let loads = sent(&mut host);
    let render = frame_render(&confirm_loads(&mut host, &state, &loads));
    host.on_screen = false;
    complete(&mut host, &state, &render);
    assert!(waiter.try_recv().unwrap().is_ok());
    assert_eq!(host.report()["directed"]["active"], Value::Null);
}

#[test]
fn a_seek_while_hidden_draws_when_the_monitor_is_shown_again_even_if_the_request_is_the_same() {
    let (mut host, mut state, resources) = directed_host("hidden-seek");
    host.render_scene(&mut state, &resources, &frame_request(0), false)
        .unwrap();
    let loads = sent(&mut host);
    let first = render_after_loads(&mut host, &state, &loads);
    complete(&mut host, &state, &first);

    host.on_screen = false;
    host.render_scene(&mut state, &resources, &frame_request(36), false)
        .unwrap();
    assert!(
        sent(&mut host).is_empty(),
        "nothing is drawn for a hidden monitor"
    );
    assert_eq!(host.report()["directed"]["queued"]["explicit"], false);

    // The workstation shows the monitor and syncs with the very same request: it must start it.
    host.on_screen = true;
    host.render_scene(&mut state, &resources, &frame_request(36), false)
        .unwrap();
    let messages = sent(&mut host);
    assert!(
        !messages.is_empty(),
        "showing the monitor starts the parked frame"
    );
    let render = render_after_loads(&mut host, &state, &messages);
    assert_eq!(render["plan"]["frame"], 36);
}

#[test]
fn an_explicit_frame_before_the_runtime_is_ready_is_refused_not_parked() {
    let (mut host, mut state, resources) = directed_host("explicit-booting");
    assert!(host.begin_reload());
    let before = host.report()["directed"].clone();
    let error = host
        .render_scene(&mut state, &resources, &frame_request(20), true)
        .unwrap_err();
    assert_eq!(error.code, "backend_unavailable");
    assert_eq!(host.report()["directed"]["queued"], Value::Null);
    assert_eq!(host.report()["directed"]["scene"], before["scene"]);
}

#[test]
fn an_explicit_frame_while_the_context_is_lost_is_refused_and_the_restore_redraws_the_monitor() {
    let (mut host, mut state, resources) = directed_host("lost-explicit");
    host.render_scene(&mut state, &resources, &frame_request(0), false)
        .unwrap();
    let loads = sent(&mut host);
    let first = frame_render(&confirm_loads(&mut host, &state, &loads));
    complete(&mut host, &state, &first);

    from_page(&mut host, &state, json!({"type":"context.lost"}));
    assert_eq!(host.report()["directed"]["suspended"], true);
    assert_eq!(host.report()["directed"]["context_losses"], 1);
    assert_eq!(
        host.report()["headline"],
        "Falha: context_lost",
        "a context that never comes back must be visible"
    );
    let error = host
        .render_scene(&mut state, &resources, &frame_request(20), true)
        .unwrap_err();
    assert_eq!(error.code, "context_lost");

    // The monitor's own request waits for the restore instead of being refused.
    host.render_scene(&mut state, &resources, &frame_request(20), false)
        .unwrap();
    assert!(sent(&mut host).is_empty());
    from_page(&mut host, &state, json!({"type":"context.restored"}));
    host.render_scene(&mut state, &resources, &frame_request(20), false)
        .unwrap();
    let messages = sent(&mut host);
    assert!(
        !messages.iter().any(|m| m["type"] == "asset.load"),
        "the page kept the decoded pictures through the loss: nothing is loaded again"
    );
    let render = render_after_loads(&mut host, &state, &messages);
    assert_eq!(render["plan"]["frame"], 20);
    complete(&mut host, &state, &render);
    assert!(
        !host.report()["headline"]
            .as_str()
            .unwrap()
            .starts_with("Falha")
    );
}

#[test]
fn a_context_lost_with_an_explicit_frame_in_flight_answers_context_lost_and_refuses_its_late_answer()
 {
    let (mut host, mut state, resources) = directed_host("lost-in-flight");
    let waiter = host
        .render_scene(&mut state, &resources, &frame_request(20), true)
        .unwrap()
        .unwrap();
    let loads = sent(&mut host);
    let old = frame_render(&confirm_loads(&mut host, &state, &loads));

    from_page(&mut host, &state, json!({"type":"context.lost"}));
    assert_eq!(waiter.try_recv().unwrap().unwrap_err().code, "context_lost");
    assert!(
        sent(&mut host).iter().any(|m| m["type"] == "frame.cancel"),
        "the frame in flight is cancelled in the page"
    );

    // The page finishes the frame it drew on the dead context: the answer is old news.
    from_page(
        &mut host,
        &state,
        json!({"type":"frame.failed","id":old["id"],"revision":old["revision"],
            "error":{"code":"context_lost","message":"lost"}}),
    );
    complete(&mut host, &state, &old);
    assert_eq!(host.report()["directed"]["refused"]["stale_frames"], 2);
    assert!(host.report()["last_frame"].is_null());
}

#[test]
fn a_reload_with_an_explicit_frame_in_flight_answers_it_and_drops_what_the_old_page_still_says() {
    let (mut host, mut state, resources) = directed_host("reload-in-flight");
    let waiter = host
        .render_scene(&mut state, &resources, &frame_request(20), true)
        .unwrap()
        .unwrap();
    let loads = sent(&mut host);
    let old = frame_render(&confirm_loads(&mut host, &state, &loads));

    assert!(host.begin_reload());
    let error = waiter.try_recv().unwrap().unwrap_err();
    assert_eq!(error.code, "cancelled");
    assert!(error.to_string().contains("reloading"), "{error}");

    // Queued messages of the old document: no frame, no asset acknowledgement reaches the new session.
    complete(&mut host, &state, &old);
    from_page(&mut host, &state, asset_ready("scene-res-0"));
    from_page(&mut host, &state, ready_message());
    assert!(
        !host.is_ready(),
        "only the new page's own page.ready starts a handshake"
    );
    assert!(host.report()["last_frame"].is_null());

    start_page(&mut host, &state);
    let waiter = host
        .render_scene(&mut state, &resources, &frame_request(20), true)
        .unwrap()
        .unwrap();
    let loads = sent(&mut host);
    assert!(
        loads.iter().any(|m| m["type"] == "asset.load"),
        "a reloaded page holds nothing, so every slot loads again"
    );
    let render = frame_render(&confirm_loads(&mut host, &state, &loads));
    assert!(render["id"].as_u64().unwrap() > old["id"].as_u64().unwrap());
    complete(&mut host, &state, &render);
    assert!(waiter.try_recv().unwrap().is_ok());
}

#[test]
fn a_load_that_a_release_overtook_does_not_fail_the_frame_that_replaced_it() {
    let (mut host, mut state, resources) = directed_host("superseded-load");
    host.render_scene(&mut state, &resources, &frame_request(20), false)
        .unwrap();
    let old_loads = sent(&mut host);
    let slot = old_loads
        .iter()
        .find(|m| m["type"] == "asset.load")
        .map(|m| m["asset"]["id"].as_str().unwrap().to_owned())
        .unwrap();

    // The project is replaced and the new one needs the same slot name before the page answered.
    host.reset_project();
    let released = sent(&mut host);
    assert!(
        released
            .iter()
            .any(|m| m["type"] == "asset.release" && m["id"] == slot.as_str())
    );
    let waiter = host
        .render_scene(&mut state, &resources, &frame_request(20), true)
        .unwrap()
        .unwrap();
    let loads = sent(&mut host);
    assert!(loads.iter().any(|m| m["type"] == "asset.load"));

    // The runtime ends the replaced load as cancelled, after the new project asked for its own
    // loads. Whatever slot it names, it is not the answer of the frame that waits now.
    from_page(
        &mut host,
        &state,
        asset_failed(&slot, "cancelled", Value::Null),
    );
    assert!(
        waiter.try_recv().is_err(),
        "the new frame is still waiting for its slots"
    );
    assert_eq!(
        host.report()["directed"]["refused"]["stale_asset_states"],
        1
    );
    let render = frame_render(&confirm_loads(&mut host, &state, &loads));
    complete(&mut host, &state, &render);
    assert!(waiter.try_recv().unwrap().is_ok());
}

#[test]
fn a_stale_generation_error_of_an_old_load_is_refused_but_a_real_failure_still_fails_the_frame() {
    let (mut host, mut state, resources) = directed_host("real-failure");
    let waiter = host
        .render_scene(&mut state, &resources, &frame_request(20), true)
        .unwrap()
        .unwrap();
    let loads = sent(&mut host);
    let failing = loads.iter().find(|m| m["type"] == "asset.load").unwrap();
    let slot = failing["asset"]["id"].as_str().unwrap().to_owned();
    let asset_id = failing["asset"]["scene"]["asset_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let tracked = host.report()["directed"]["slots"]["tracked_loads"]
        .as_u64()
        .unwrap();
    assert!(tracked >= 1);

    from_page(
        &mut host,
        &state,
        asset_failed(
            &slot,
            "resource_stale",
            json!({"asset_id":"another-asset","generation":0}),
        ),
    );
    assert!(waiter.try_recv().is_err());
    assert_eq!(
        host.report()["directed"]["refused"]["stale_asset_states"],
        1
    );

    from_page(
        &mut host,
        &state,
        asset_failed(&slot, "asset_unreadable", Value::Null),
    );
    assert_eq!(
        waiter.try_recv().unwrap().unwrap_err().code,
        "asset_unreadable"
    );
    assert_eq!(
        host.report()["directed"]["slots"]["tracked_loads"]
            .as_u64()
            .unwrap(),
        tracked - 1,
        "a failed slot is forgotten so the next request loads it again"
    );

    let waiter = host
        .render_scene(&mut state, &resources, &frame_request(20), true)
        .unwrap()
        .unwrap();
    let again = sent(&mut host);
    assert!(
        again
            .iter()
            .any(|m| m["type"] == "asset.load"
                && m["asset"]["scene"]["asset_id"] == asset_id.as_str()),
        "the failed resource is loaded again, in whatever slot is free"
    );
    // The slots the first request loaded were never acknowledged either; the page answers them all.
    let pending = [loads.clone(), again].concat();
    let render = frame_render(&confirm_loads(&mut host, &state, &pending));
    complete(&mut host, &state, &render);
    assert!(waiter.try_recv().unwrap().is_ok());
}

#[test]
fn a_frame_that_never_comes_answers_the_explicit_waiter_with_the_timeout() {
    let (mut host, mut state, resources) = directed_host("timeout");
    let waiter = host
        .render_scene(&mut state, &resources, &frame_request(20), true)
        .unwrap()
        .unwrap();
    let loads = sent(&mut host);
    let render = frame_render(&confirm_loads(&mut host, &state, &loads));
    assert!(host.tick(Instant::now() + Duration::from_secs(31)));
    assert_eq!(waiter.try_recv().unwrap().unwrap_err().code, "timeout");
    assert_eq!(host.report()["directed"]["active"], Value::Null);

    complete(&mut host, &state, &render);
    assert!(
        host.report()["last_frame"].is_null(),
        "a frame the host gave up on never shows"
    );
}

#[test]
fn the_project_switch_with_a_frame_in_flight_releases_the_slots_and_refuses_the_old_picture() {
    let (mut host, mut state, resources) = directed_host("project-switch");
    let waiter = host
        .render_scene(&mut state, &resources, &frame_request(20), true)
        .unwrap()
        .unwrap();
    let loads = sent(&mut host);
    let old = frame_render(&confirm_loads(&mut host, &state, &loads));
    assert!(
        host.report()["directed"]["slots"]["occupied"]
            .as_u64()
            .unwrap()
            >= 1
    );

    host.reset_project();
    assert_eq!(waiter.try_recv().unwrap().unwrap_err().code, "cancelled");
    let released = sent(&mut host);
    assert!(released.iter().any(|m| m["type"] == "asset.release"));
    assert_eq!(host.report()["directed"]["slots"]["occupied"], 0);
    assert_eq!(host.report()["directed"]["scene"], Value::Null);

    complete(&mut host, &state, &old);
    assert_eq!(host.report()["directed"]["refused"]["stale_frames"], 1);
    assert!(host.report()["last_frame"].is_null());
}

#[test]
fn repeated_context_losses_neither_reload_nor_leak_slots() {
    let (mut host, mut state, resources) = directed_host("loss-cycles");
    host.render_scene(&mut state, &resources, &frame_request(0), false)
        .unwrap();
    let loads = sent(&mut host);
    let first_loads = loads.iter().filter(|m| m["type"] == "asset.load").count();
    assert!(first_loads >= 1);
    let render = render_after_loads(&mut host, &state, &loads);
    complete(&mut host, &state, &render);
    let occupied = host.report()["directed"]["slots"]["occupied"].clone();

    for frame in [11, 20, 36, 48, 60, 95] {
        from_page(&mut host, &state, json!({"type":"context.lost"}));
        from_page(&mut host, &state, json!({"type":"context.restored"}));
        host.render_scene(&mut state, &resources, &frame_request(frame), false)
            .unwrap();
        let messages = sent(&mut host);
        assert!(
            !messages
                .iter()
                .any(|m| m["type"] == "asset.load" || m["type"] == "asset.release"),
            "frame {frame}: a lost context moves no picture in or out of the page"
        );
        let render = render_after_loads(&mut host, &state, &messages);
        assert_eq!(render["plan"]["frame"], frame);
        complete(&mut host, &state, &render);
    }
    assert_eq!(host.report()["directed"]["slots"]["occupied"], occupied);
    assert_eq!(host.report()["directed"]["context_losses"], 6);
}

#[test]
fn a_timeout_releases_the_pictures_the_page_still_holds_but_a_reload_only_forgets_them() {
    let (mut host, mut state, resources) = directed_host("release-vs-forget");
    host.render_scene(&mut state, &resources, &frame_request(20), false)
        .unwrap();
    let loads = sent(&mut host);
    let held = loads.iter().filter(|m| m["type"] == "asset.load").count();
    let render = render_after_loads(&mut host, &state, &loads);

    assert!(host.tick(Instant::now() + Duration::from_secs(31)));
    let messages = sent(&mut host);
    let releases = messages
        .iter()
        .filter(|m| m["type"] == "asset.release")
        .count();
    assert_eq!(
        releases, held,
        "the page still holds them, so each one is released instead of leaking"
    );
    assert_eq!(host.report()["directed"]["slots"]["occupied"], 0);
    complete(&mut host, &state, &render);

    // A reload leaves nothing in the page to release: the book is forgotten and no message is sent.
    host.render_scene(&mut state, &resources, &frame_request(20), false)
        .unwrap();
    let again = sent(&mut host);
    assert_eq!(
        again.iter().filter(|m| m["type"] == "asset.load").count(),
        held
    );
    assert!(host.begin_reload());
    assert!(sent(&mut host).is_empty());
    assert_eq!(host.report()["directed"]["slots"]["occupied"], 0);
}
