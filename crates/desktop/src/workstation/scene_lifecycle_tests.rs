//! Lifecycle of the directed-scene monitor host (BL092) without a window: a scripted page answers on
//! the bridge, and the tests watch that every explicit request is answered exactly once, that old
//! answers never reach the current frame, and that the book of slots is rebuilt after the runtime
//! lost what it held.
//!
//! These are host-level tests with a scripted page. They do not replace the native proof
//! (`scripts/scene-lifecycle-proof.py`), which uses the real WKWebView and real `WEBGL_lose_context`.
//! The helpers repeat the small ones of `viewport_host_tests.rs` because that module is private.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use core::prelude::v1::test;
use serde_json::{Value, json};
use tessel_editor_core::headless::execute_ext;
use tessel_editor_core::perf::Meter;
use tessel_editor_core::perf_history::{EventKind, Metric};
use tessel_editor_core::scene_plan::Capability;
use tessel_editor_core::{EditorState, state::ProjectSettings};
use tessel_editor_media::host::MediaHost;
use tessel_editor_media::scene_resources::SceneResourceSession;
use tessel_editor_render::scene::SceneSlots;

use super::super::super::perf_meter::PerfClock;
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
    host_with(tag, ready_message())
}

/// The handshake of a runtime that lists `asset.identity` and echoes the request of every load.
fn identified_ready_message() -> Value {
    let mut ready = ready_message();
    ready["capabilities"]["features"] = json!(["directed_scene", "asset.identity"]);
    ready
}

fn identified_host(tag: &str) -> (ViewportHost, EditorState, SceneResourceSession) {
    host_with(tag, identified_ready_message())
}

fn host_with(tag: &str, ready: Value) -> (ViewportHost, EditorState, SceneResourceSession) {
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
    start_page_with(&mut host, &state, ready);
    (host, state, media.scene_resources())
}

/// `page.ready` then the handshake: what a freshly loaded page does.
fn start_page(host: &mut ViewportHost, state: &EditorState) {
    start_page_with(host, state, ready_message());
}

fn start_page_with(host: &mut ViewportHost, state: &EditorState, ready: Value) {
    from_page(host, state, json!({"type":"page.ready"}));
    sent(host);
    from_page(host, state, ready);
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

/// The `asset.state` a runtime that echoes identity sends for `load`: the request and scene it names.
fn echoed(load: &Value, state: &str) -> Value {
    let mut message = json!({"type":"asset.state","id":load["asset"]["id"],"state":state,
        "request":load["asset"]["request"],"scene":load["asset"]["scene"]});
    if state == "ready" {
        message["meta"] = json!({});
    }
    message
}

fn loads_of(messages: &[Value]) -> Vec<Value> {
    messages
        .iter()
        .filter(|m| m["type"] == "asset.load")
        .cloned()
        .collect()
}

/// BL102: the page answered the load of a slot late, after the slot was released and given to a
/// newer load. The old `ready` carries the old request, so it cannot unblock the new frame.
#[test]
fn an_old_ready_of_a_reused_slot_never_confirms_the_load_that_replaced_it() {
    let (mut host, mut state, resources) = identified_host("identity-reused-slot");
    // Exactly the three slots the scene needs: a replaced project has to reuse the same names.
    host.directed.slots = SceneSlots::new(3, 512 * 1024 * 1024);
    assert_eq!(host.report()["directed"]["asset_identity"], true);
    host.render_scene(&mut state, &resources, &frame_request(20), false)
        .unwrap();
    let old_loads = loads_of(&sent(&mut host));
    assert!(!old_loads.is_empty());
    assert!(
        old_loads.iter().all(|m| m["asset"]["request"].is_u64()),
        "every load is stamped once the feature is negotiated"
    );

    // The project is replaced before the page answered: the same slot names are loaded again.
    host.reset_project();
    sent(&mut host);
    let waiter = host
        .render_scene(&mut state, &resources, &frame_request(20), true)
        .unwrap()
        .unwrap();
    let new_loads = loads_of(&sent(&mut host));
    assert_eq!(new_loads.len(), old_loads.len());
    for old in &old_loads {
        let new = new_loads
            .iter()
            .find(|new| new["asset"]["id"] == old["asset"]["id"])
            .expect("the slot is loaded again");
        assert_ne!(
            old["asset"]["request"], new["asset"]["request"],
            "a request number is never reused, not even for the same picture in the same slot"
        );
    }

    // The old answers arrive now, in the page's own order.
    for old in &old_loads {
        from_page(&mut host, &state, echoed(old, "ready"));
    }
    assert!(
        sent(&mut host).iter().all(|m| m["type"] != "frame.render"),
        "the frame is not sent against pictures nobody confirmed"
    );
    assert_eq!(
        host.report()["directed"]["refused"]["stale_asset_states"],
        old_loads.len() as u64
    );
    assert!(waiter.try_recv().is_err());

    for new in &new_loads {
        from_page(&mut host, &state, echoed(new, "ready"));
    }
    let render = frame_render(&sent(&mut host));
    complete(&mut host, &state, &render);
    assert!(waiter.try_recv().unwrap().is_ok());
}

#[test]
fn without_the_feature_the_load_is_unchanged_and_the_slot_name_is_all_there_is() {
    let (mut host, mut state, resources) = directed_host("identity-legacy-runtime");
    assert_eq!(host.report()["directed"]["asset_identity"], false);
    host.render_scene(&mut state, &resources, &frame_request(20), false)
        .unwrap();
    let loads = loads_of(&sent(&mut host));
    assert!(!loads.is_empty());
    assert!(
        loads.iter().all(|m| m["asset"].get("request").is_none()),
        "an older runtime is never sent the field"
    );
    assert_eq!(host.report()["directed"]["slots"]["identified_loads"], 0);
    let render = frame_render(&confirm_loads(&mut host, &state, &loads));
    assert_eq!(render["type"], "frame.render");
}

#[test]
fn once_negotiated_an_answer_without_the_echo_confirms_nothing() {
    let (mut host, mut state, resources) = identified_host("identity-no-echo");
    host.render_scene(&mut state, &resources, &frame_request(20), false)
        .unwrap();
    let loads = loads_of(&sent(&mut host));
    for load in &loads {
        from_page(
            &mut host,
            &state,
            asset_ready(load["asset"]["id"].as_str().unwrap()),
        );
    }
    assert!(sent(&mut host).iter().all(|m| m["type"] != "frame.render"));
    assert_eq!(
        host.report()["directed"]["refused"]["stale_asset_states"],
        loads.len() as u64
    );
    // The echoed answers still work: nothing was forgotten by refusing the bare ones.
    for load in &loads {
        from_page(&mut host, &state, echoed(load, "ready"));
    }
    assert_eq!(frame_render(&sent(&mut host))["type"], "frame.render");
}

#[test]
fn the_echo_tells_the_failure_of_a_replaced_load_from_the_failure_of_the_current_one() {
    let (mut host, mut state, resources) = identified_host("identity-failures");
    host.render_scene(&mut state, &resources, &frame_request(20), false)
        .unwrap();
    let old = loads_of(&sent(&mut host));
    host.reset_project();
    sent(&mut host);
    let waiter = host
        .render_scene(&mut state, &resources, &frame_request(20), true)
        .unwrap()
        .unwrap();
    let new = loads_of(&sent(&mut host));

    // The old load ends with a real-looking error, but it is not the load that holds the slot.
    let mut stale = echoed(&old[0], "failed");
    stale["error"] = json!({"code":"asset_unreadable","message":"old"});
    from_page(&mut host, &state, stale);
    assert!(
        waiter.try_recv().is_err(),
        "the answer of a replaced load does not fail the frame that replaced it"
    );
    assert_eq!(
        host.report()["directed"]["refused"]["stale_asset_states"],
        1
    );

    let mut current = echoed(&new[0], "failed");
    current["error"] = json!({"code":"asset_unreadable","message":"the real one"});
    from_page(&mut host, &state, current);
    assert_eq!(
        waiter.try_recv().unwrap().unwrap_err().code,
        "asset_unreadable"
    );
}

// ---- the history hears the directed monitor's context (F049 / BL145.4)

/// The history records into a meter on the wall clock; the host feeds it as the window does.
fn recording(host: &mut ViewportHost) -> (Rc<RefCell<Meter>>, PerfClock) {
    let meter = Rc::new(RefCell::new(Meter::default()));
    let clock = PerfClock::wall();
    meter
        .borrow_mut()
        .start_recording(clock.elapsed(), None, None);
    host.attach_meter(meter.clone(), clock.clone());
    (meter, clock)
}

/// The context events of the history, in order, with the request each one names.
fn context_line(meter: &Rc<RefCell<Meter>>, clock: &PerfClock) -> Vec<(EventKind, Option<u64>)> {
    meter
        .borrow()
        .recorder()
        .snapshot(clock.elapsed())
        .unwrap()
        .events
        .iter()
        .filter(|event| {
            matches!(
                event.kind,
                EventKind::ContextLost | EventKind::ContextRestored | EventKind::RequestFailed
            )
        })
        .map(|event| (event.kind, event.request))
        .collect()
}

#[test]
fn a_context_lost_and_restored_while_the_directed_monitor_is_idle_reach_the_history_once() {
    let (mut host, mut state, resources) = directed_host("history-idle");
    let (meter, clock) = recording(&mut host);
    host.render_scene(&mut state, &resources, &frame_request(0), false)
        .unwrap();
    let loads = sent(&mut host);
    let first = frame_render(&confirm_loads(&mut host, &state, &loads));
    complete(&mut host, &state, &first);
    assert_eq!(host.report()["directed"]["active"], Value::Null, "idle");
    assert!(context_line(&meter, &clock).is_empty());

    // Nothing is in flight and nothing asks for a render: the page's notice is all there is.
    from_page(&mut host, &state, json!({"type":"context.lost"}));
    // The page repeating itself while the context is down is the same loss.
    from_page(&mut host, &state, json!({"type":"context.lost"}));
    from_page(&mut host, &state, json!({"type":"context.restored"}));
    from_page(&mut host, &state, json!({"type":"context.restored"}));
    assert_eq!(
        context_line(&meter, &clock),
        vec![
            (EventKind::ContextLost, None),
            (EventKind::ContextRestored, None)
        ],
        "one loss and one restore, none of them a failed request, neither naming a request"
    );

    // A second loss later is a second event.
    from_page(&mut host, &state, json!({"type":"context.lost"}));
    assert_eq!(context_line(&meter, &clock).len(), 3);
}

#[test]
fn a_context_lost_with_a_directed_frame_in_flight_is_one_event_that_names_its_request() {
    let (mut host, mut state, resources) = directed_host("history-in-flight");
    let (meter, clock) = recording(&mut host);
    let waiter = host
        .render_scene(&mut state, &resources, &frame_request(20), true)
        .unwrap()
        .unwrap();
    let loads = sent(&mut host);
    let old = frame_render(&confirm_loads(&mut host, &state, &loads));
    let in_flight = old["id"].as_u64().unwrap();

    from_page(&mut host, &state, json!({"type":"context.lost"}));
    assert_eq!(waiter.try_recv().unwrap().unwrap_err().code, "context_lost");
    // The frame the page then reports as failed is the same loss, and an old news item.
    from_page(
        &mut host,
        &state,
        json!({"type":"frame.failed","id":old["id"],"revision":old["revision"],
            "error":{"code":"context_lost","message":"lost"}}),
    );
    from_page(&mut host, &state, json!({"type":"context.restored"}));
    assert_eq!(
        context_line(&meter, &clock),
        vec![
            (EventKind::ContextLost, Some(in_flight)),
            (EventKind::ContextRestored, None)
        ]
    );
    assert_eq!(
        meter.borrow().recorder().pending_requests(),
        0,
        "the interrupted request is not waited for any more"
    );
}

#[test]
fn a_host_with_no_recording_hears_nothing_of_a_directed_context_loss() {
    let (mut host, state, _resources) = directed_host("history-off");
    let meter = Rc::new(RefCell::new(Meter::default()));
    host.attach_meter(meter.clone(), PerfClock::wall());
    from_page(&mut host, &state, json!({"type":"context.lost"}));
    from_page(&mut host, &state, json!({"type":"context.restored"}));
    assert!(!meter.borrow().enabled());
    assert!(meter.borrow().recorder().snapshot(Duration::ZERO).is_none());
    assert_eq!(meter.borrow().recorder().events_held(), 0);
}

// ---- a frame asked under a scene that was left is not a measurement of the next one (BL145.6)
//
// The monitor does not cancel the frame in flight when the user moves to another scene (only an
// explicit request does): the old frame is answered to whoever asked, and it used to be recorded
// as an image, an interval, a latency, a texture and a frame format of the NEW segment, because
// the scene change opens the segment before the old answer comes back.

/// The corpus scene attached twice (`proof` and `second`), both with their verified resources, on a
/// page that completed the handshake.
fn two_scene_host(tag: &str) -> (ViewportHost, EditorState, SceneResourceSession) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/directed-scene-proof/corpus")
        .canonicalize()
        .unwrap();
    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    let mut media = MediaHost::new();
    for scene in ["proof", "second"] {
        for request in [
            json!({"op":"scene.attach","id":scene,"path":root.join("bundle.json")}),
            json!({"op":"scene.resources.resolve","scene":scene,"base":root}),
        ] {
            execute_ext(&mut state, None, Some(&mut media), &request).unwrap();
        }
    }
    let mut host = ViewportHost::new(Ok(page(tag)));
    start_page_with(&mut host, &state, ready_message());
    (host, state, media.scene_resources())
}

fn scene_frame(scene: &str, frame: i64) -> Value {
    json!({"scene":scene,"frame":frame,"width":320,"height":180})
}

/// How many samples of `metric` the open window holds (0 when it holds none).
fn samples(meter: &Rc<RefCell<Meter>>, clock: &PerfClock, metric: Metric) -> u64 {
    meter
        .borrow()
        .recorder()
        .snapshot(clock.elapsed())
        .and_then(|snapshot| snapshot.summary(metric))
        .map_or(0, |summary| summary.count)
}

fn stale_answers(meter: &Rc<RefCell<Meter>>) -> u64 {
    meter.borrow().recorder().counters().stale_responses
}

/// Every event of `kinds`, in order, as `(kind, segment)`.
fn events_of(
    meter: &Rc<RefCell<Meter>>,
    clock: &PerfClock,
    kinds: &[EventKind],
) -> Vec<(EventKind, u32)> {
    meter
        .borrow()
        .recorder()
        .snapshot(clock.elapsed())
        .unwrap()
        .events
        .iter()
        .filter(|event| kinds.contains(&event.kind))
        .map(|event| (event.kind, event.segment))
        .collect()
}

/// A frame of `proof` drawn and answered inside its own segment (the control), then a second one
/// (`explicit` or not) left in flight. Returns its `frame.render`, its waiter when explicit.
fn proof_frame_left_in_flight(
    host: &mut ViewportHost,
    state: &mut EditorState,
    resources: &SceneResourceSession,
    explicit: bool,
) -> (
    Value,
    Option<async_channel::Receiver<Result<Value, tessel_editor_core::CommandError>>>,
) {
    host.render_scene(state, resources, &scene_frame("proof", 0), false)
        .unwrap();
    let loads = sent(host);
    let first = render_after_loads(host, state, &loads);
    complete(host, state, &first);
    let waiter = host
        .render_scene(state, resources, &scene_frame("proof", 24), explicit)
        .unwrap();
    let old = frame_render(&sent(host));
    (old, waiter)
}

#[test]
fn an_old_frame_that_comes_back_after_a_scene_switch_is_no_measurement_of_the_new_segment() {
    let (mut host, mut state, resources) = two_scene_host("history-stale-image");
    let (meter, clock) = recording(&mut host);
    meter.borrow_mut().enable(clock.elapsed(), None, None);
    let (old, waiter) = proof_frame_left_in_flight(&mut host, &mut state, &resources, true);
    let waiter = waiter.unwrap();
    // The control: the first frame was answered inside its own segment and was recorded.
    for metric in [Metric::MonitorFrames, Metric::RenderMs, Metric::LatencyMs] {
        assert_eq!(
            samples(&meter, &clock, metric),
            1,
            "{metric:?} of the control frame"
        );
    }
    assert_eq!(meter.borrow().samples().1, 1);
    assert_eq!(stale_answers(&meter), 0);
    let segment_of_the_old_scene = meter.borrow().recorder().segment();

    // The user moves to another scene: the segment changes, the old frame is still in flight.
    host.render_scene(&mut state, &resources, &scene_frame("second", 0), false)
        .unwrap();
    assert_eq!(
        meter.borrow().recorder().segment(),
        segment_of_the_old_scene + 1,
        "the premise: the scene change opened a segment before the old answer"
    );
    assert!(
        sent(&mut host).is_empty(),
        "nothing goes out behind the frame in flight"
    );

    // The old answer arrives. Whoever asked for it is answered, and the new segment hears none of it.
    complete(&mut host, &state, &old);
    assert_eq!(
        meter.borrow().samples().1,
        2,
        "the old picture was still displayed"
    );
    assert!(
        waiter.try_recv().unwrap().is_ok(),
        "the caller keeps its answer"
    );
    assert_eq!(
        samples(&meter, &clock, Metric::MonitorFrames),
        1,
        "no frame of the new segment"
    );
    assert_eq!(
        samples(&meter, &clock, Metric::RenderMs),
        1,
        "no CPU time of the new segment"
    );
    assert_eq!(
        samples(&meter, &clock, Metric::LatencyMs),
        1,
        "no latency of the new segment"
    );
    assert_eq!(
        samples(&meter, &clock, Metric::MonitorIntervalMs),
        0,
        "no interval anchor"
    );
    assert_eq!(
        stale_answers(&meter),
        1,
        "counted as a stale answer, and only that"
    );
    assert_eq!(
        meter.borrow().recorder().pending_requests(),
        1,
        "only the request of the new scene (admitted as the old one ended) is waited for"
    );

    // The new scene's own frame is measured normally, with the latency of its own request.
    let loads = sent(&mut host);
    let new = render_after_loads(&mut host, &state, &loads);
    complete(&mut host, &state, &new);
    assert_eq!(meter.borrow().samples().1, 3);
    assert_eq!(samples(&meter, &clock, Metric::MonitorFrames), 2);
    assert_eq!(samples(&meter, &clock, Metric::LatencyMs), 2);
    assert_eq!(stale_answers(&meter), 1);
    assert_eq!(meter.borrow().recorder().pending_requests(), 0);
}

#[test]
fn an_old_frame_that_fails_after_a_scene_switch_is_no_event_of_the_new_segment() {
    let (mut host, mut state, resources) = two_scene_host("history-stale-failure");
    let (meter, clock) = recording(&mut host);
    let (old, waiter) = proof_frame_left_in_flight(&mut host, &mut state, &resources, true);
    let waiter = waiter.unwrap();
    host.render_scene(&mut state, &resources, &scene_frame("second", 0), false)
        .unwrap();

    from_page(
        &mut host,
        &state,
        json!({"type":"frame.failed","id":old["id"],"revision":old["revision"],
            "error":{"code":"render_failed","message":"the old scene failed"}}),
    );
    assert_eq!(
        waiter.try_recv().unwrap().unwrap_err().code,
        "render_failed",
        "the caller is told, whatever the history does"
    );
    assert!(
        events_of(&meter, &clock, &[EventKind::RequestFailed]).is_empty(),
        "the failure of a frame of the scene that was left is not an event of the next segment"
    );
    assert_eq!(stale_answers(&meter), 1);

    // A failure of the new scene's own frame IS news in its segment.
    let loads = sent(&mut host);
    let new = render_after_loads(&mut host, &state, &loads);
    from_page(
        &mut host,
        &state,
        json!({"type":"frame.failed","id":new["id"],"revision":new["revision"],
            "error":{"code":"render_failed","message":"the new scene failed"}}),
    );
    let segment = meter.borrow().recorder().segment();
    assert_eq!(
        events_of(&meter, &clock, &[EventKind::RequestFailed]),
        vec![(EventKind::RequestFailed, segment)]
    );
}

#[test]
fn a_cancelled_frame_released_after_the_new_scene_is_stale_and_never_measured() {
    let (mut host, mut state, resources) = two_scene_host("history-out-of-order");
    let (meter, clock) = recording(&mut host);
    let (old, waiter) = proof_frame_left_in_flight(&mut host, &mut state, &resources, true);
    let old_waiter = waiter.unwrap();

    // An explicit frame of another scene supersedes it: the old one is cancelled and answered.
    let new_waiter = host
        .render_scene(&mut state, &resources, &scene_frame("second", 0), true)
        .unwrap()
        .unwrap();
    assert_eq!(
        old_waiter.try_recv().unwrap().unwrap_err().code,
        "cancelled"
    );
    let loads = sent(&mut host);
    let new = render_after_loads(&mut host, &state, &loads);
    complete(&mut host, &state, &new);
    assert!(new_waiter.try_recv().unwrap().is_ok());
    assert_eq!(
        samples(&meter, &clock, Metric::MonitorFrames),
        2,
        "the control and the new frame"
    );

    // The page releases the cancelled frame LAST (out of order): the host refuses it.
    complete(&mut host, &state, &old);
    assert_eq!(
        host.report()["directed"]["refused"]["stale_frames"],
        json!(1)
    );
    assert_eq!(
        samples(&meter, &clock, Metric::MonitorFrames),
        2,
        "still two"
    );
    assert_eq!(samples(&meter, &clock, Metric::LatencyMs), 2);
    assert_eq!(samples(&meter, &clock, Metric::RenderMs), 2);
    assert!(events_of(&meter, &clock, &[EventKind::RequestFailed]).is_empty());
}

#[test]
fn a_load_of_the_scene_that_was_left_ends_in_no_segment_that_never_saw_it_start() {
    let (mut host, mut state, resources) = two_scene_host("history-stale-load");
    let (meter, clock) = recording(&mut host);
    let finished = [EventKind::ResourceLoadFinished];
    let load_events = [
        EventKind::ResourceLoadStarted,
        EventKind::ResourceLoadFinished,
    ];

    // The control: loads that start and end inside one segment show both ends there.
    host.render_scene(&mut state, &resources, &scene_frame("proof", 0), false)
        .unwrap();
    let loads = sent(&mut host);
    let started = loads.iter().filter(|m| m["type"] == "asset.load").count();
    assert!(started > 0, "the premise: the first frame waits for loads");
    let first = render_after_loads(&mut host, &state, &loads);
    let segment = meter.borrow().recorder().segment();
    assert_eq!(
        events_of(&meter, &clock, &load_events),
        [
            vec![(EventKind::ResourceLoadStarted, segment); started],
            vec![(EventKind::ResourceLoadFinished, segment); started],
        ]
        .concat()
    );
    complete(&mut host, &state, &first);
    let ended_before = events_of(&meter, &clock, &finished).len();

    // The pictures are dropped (the monitor was closed), so the next frame loads them again.
    host.clear_directed();
    host.render_scene(&mut state, &resources, &scene_frame("proof", 24), false)
        .unwrap();
    let old_loads = sent(&mut host);
    assert_eq!(
        old_loads
            .iter()
            .filter(|m| m["type"] == "asset.load")
            .count(),
        started,
        "the premise: they are loaded again"
    );
    // The user moves on before the page answered; then the page answers the old loads. (A frame
    // other than 24: two scenes of one bundle plan the same picture at the same frame, and the
    // host would take a repeated request for the same scene.)
    host.render_scene(&mut state, &resources, &scene_frame("second", 12), false)
        .unwrap();
    let render = render_after_loads(&mut host, &state, &old_loads);
    assert_eq!(
        events_of(&meter, &clock, &finished).len(),
        ended_before,
        "the end of a load of the scene that was left is not in the segment that never saw it start"
    );

    // The book is updated all the same: the old frame draws, and the next scene's frame is
    // drawn from the pictures that are ready, and measured in its own segment.
    complete(&mut host, &state, &render);
    let next_messages = sent(&mut host);
    let next = render_after_loads(&mut host, &state, &next_messages);
    complete(&mut host, &state, &next);
    assert_eq!(samples(&meter, &clock, Metric::MonitorFrames), 2);
    assert_eq!(
        host.report()["directed"]["slots"]["occupied"],
        json!(started)
    );
}
