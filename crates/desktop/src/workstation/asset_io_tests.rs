//! BL157.1: the protocol handler answers the same bytes either way, answers every request exactly once, and the
//! asynchronous way leaves the window's thread free while a read is held (the synchronous way does not).
//!
//! The control that holds a read uses a named pipe: opening it for reading blocks until something opens it for writing,
//! which gives the test a deterministic "the file is slow" with no sleep standing in for a disk.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tessel_editor_core::trace::TraceHandle;
use tessel_editor_render::asset_io::{AssetPool, DEFAULT_QUEUE, DEFAULT_WORKERS, Reader};
use tessel_editor_render::host::{AssetAccess, MAX_RANGE_BODY};
use wry::http::Request;

use super::viewport_host_tests::{answer, page, scratch};
use super::viewport_protocol::{AssetIoMode, ProtocolHandle};

fn handle(
    reader: Reader,
    dist: &Path,
    access: &Rc<RefCell<AssetAccess>>,
    trace: &TraceHandle,
) -> ProtocolHandle {
    ProtocolHandle::new(
        Some(dist.to_path_buf()),
        access.clone(),
        trace.clone(),
        reader,
        Arc::new(AtomicU64::new(0)),
    )
}

fn asset_request(file: &Path, range: Option<&str>) -> Request<Vec<u8>> {
    let encoded = file
        .to_string_lossy()
        .replace('/', "%2F")
        .replace(' ', "%20");
    let mut builder = Request::builder().uri(format!("tessel://localhost/asset?p={encoded}"));
    if let Some(range) = range {
        builder = builder.header("range", range);
    }
    builder.body(Vec::new()).unwrap()
}

fn register(access: &Rc<RefCell<AssetAccess>>, file: &Path) {
    access
        .borrow_mut()
        .register(&json!({"source": {"type": "file", "path": file}}));
}

/// The fields of the events of `kind` the trace holds now.
fn events_of(trace: &TraceHandle, kind: &str) -> Vec<Value> {
    trace.events_since(0, 1000)["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["kind"] == kind)
        .map(|event| event["fields"].clone())
        .collect()
}

/// The fields of the events of `kind`, once the trace holds at least `count` of them: for an event a worker records.
///
/// A worker answers first and records `asset_io` after, so whoever has the answer (even whoever joined the window's thread) is
/// not ordered after that record, and a read right then finds none whenever the worker was descheduled between the two. Only
/// the pool's `completed` counter comes after it. The wait is bounded, and a miss fails with what the trace did hold. An event
/// that the reading thread recorded itself (an inline read, a refusal answered inside `handle`) is there when `handle`
/// returns: `events_of` is enough for those.
fn events_after(trace: &TraceHandle, kind: &str, count: usize) -> Vec<Value> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let events = events_of(trace, kind);
        if events.len() >= count {
            return events;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {count} `{kind}` event(s); the trace holds {}",
            trace.events_since(0, 1000)["events"]
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn a_request_and_the_load_message_of_the_same_file_share_one_label_and_no_path() {
    use super::viewport_host::{HostOptions, ViewportHost};
    use super::viewport_host_tests::{from_page, ready_message, sent};
    use tessel_editor_core::EditorState;

    // The load comes from the host as it does in the app: the demo's files are named by real `asset.load` messages, and the
    // label of each is whatever `WebTransport::send` wrote, not something this test computes.
    let state = EditorState::demo();
    let options = HostOptions {
        trace: true,
        ..HostOptions::default()
    };
    let mut host = ViewportHost::with_options(Ok(page("io-label")), options);
    from_page(&mut host, &state, json!({"type":"page.ready"}));
    sent(&mut host);
    from_page(&mut host, &state, ready_message());
    let loads: Vec<_> = sent(&mut host)
        .into_iter()
        .filter(|message| message["type"] == "asset.load")
        .collect();
    let at = loads
        .iter()
        .position(|load| load["asset"]["source"]["path"].is_string())
        .expect("the demo loads a file");
    let file = loads[at]["asset"]["source"]["path"].as_str().unwrap();
    let request = Request::builder()
        .uri(format!(
            "tessel://localhost/asset?p={}",
            file.replace('/', "%2F").replace(' ', "%20")
        ))
        .body(Vec::new())
        .unwrap();
    assert_eq!(answer(&host.protocol_handle(), &request).status(), 200);

    let events = host.trace().events_since(0, 1_000);
    let of_kind = |kind: &str| -> Vec<serde_json::Value> {
        events["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["kind"] == kind)
            .map(|event| event["fields"].clone())
            .collect()
    };
    let (load_events, callbacks) = (of_kind("asset_load_sent"), of_kind("asset_cb"));
    assert_eq!(load_events.len(), loads.len(), "one event per load message");
    assert_eq!(callbacks.len(), 1);
    assert_eq!(
        load_events[at]["path"], callbacks[0]["asset"],
        "the load and the request of the same file name it the same way"
    );
    assert!(
        !events.to_string().contains(file),
        "the path itself is never written"
    );
}

#[test]
fn a_load_is_labelled_by_the_path_of_its_file_the_pattern_of_its_sequence_or_nothing() {
    use super::viewport_protocol::load_fields;

    let file = load_fields(&json!({"type":"asset.load","asset":{
        "id":"a1","kind":"still","source":{"type":"file","path":"/p/one.png"}}}));
    let sequence = load_fields(&json!({"type":"asset.load","asset":{
        "id":"a2","kind":"video","source":{"type":"frame_sequence","pattern":"/p/f_%04d.png"}}}));
    let generated = load_fields(&json!({"type":"asset.load","asset":{
        "id":"a3","kind":"text","source":{"type":"text"}}}));
    assert_eq!(file["kind"], "still");
    assert!(file["path"].is_string() && file.get("pattern").is_none());
    assert!(sequence["pattern"].is_string() && sequence.get("path").is_none());
    assert!(generated.get("path").is_none() && generated.get("pattern").is_none());
    for fields in [&file, &sequence, &generated] {
        let text = fields.to_string();
        assert!(
            !text.contains("one.png") && !text.contains("f_%04d"),
            "{text}"
        );
    }
    assert_ne!(
        file["asset"], sequence["asset"],
        "assets are told apart by their id"
    );
}

#[test]
fn a_label_is_the_value_it_has_always_been_so_recorded_traces_keep_their_correlation() {
    use super::viewport_protocol::trace_label;
    assert_eq!(trace_label('a', "p=/x/clip.mp4"), "a44a5eced");
    assert_eq!(trace_label('k', "k"), "k8601fd8a");
    assert_eq!(trace_label('k', ""), "k84222325");
}

#[test]
fn both_modes_answer_the_same_status_headers_and_bytes() {
    let dist = page("io-same");
    let dir = scratch("io-same-files");
    let body: Vec<u8> = (0..(MAX_RANGE_BODY + 777))
        .map(|index| (index % 253) as u8)
        .collect();
    std::fs::write(dir.join("cloud.ply"), &body).unwrap();
    std::fs::write(dir.join("small.png"), b"0123456789").unwrap();
    std::fs::write(dir.join("empty.png"), b"").unwrap();
    std::fs::write(dir.join("private.txt"), b"never").unwrap();
    let access = Rc::new(RefCell::new(AssetAccess::default()));
    for name in ["cloud.ply", "small.png", "empty.png", "missing.png"] {
        register(&access, &dir.join(name));
    }
    let trace = TraceHandle::default();
    let sync = handle(Reader::Inline, &dist, &access, &trace);
    let pool = AssetPool::new(DEFAULT_WORKERS, DEFAULT_QUEUE);
    let asynchronous = handle(Reader::Pooled(pool), &dist, &access, &trace);
    let requests: Vec<(&str, Option<&str>)> = vec![
        ("cloud.ply", None),
        ("cloud.ply", Some("bytes=0-99")),
        ("cloud.ply", Some("bytes=100-")),
        ("cloud.ply", Some("bytes=-64")),
        ("cloud.ply", Some("bytes=-999999999")),
        ("cloud.ply", Some("bytes=999999999-")),
        ("cloud.ply", Some("bytes=9-3")),
        ("small.png", None),
        ("small.png", Some("bytes=0-0")),
        ("empty.png", None),
        ("empty.png", Some("bytes=0-9")),
        ("missing.png", None),
        ("private.txt", None),
    ];
    for (name, range) in requests {
        let request = asset_request(&dir.join(name), range);
        let (left, right) = (answer(&sync, &request), answer(&asynchronous, &request));
        assert_eq!(left.status(), right.status(), "{name} {range:?}: status");
        assert_eq!(left.headers(), right.headers(), "{name} {range:?}: headers");
        assert_eq!(left.body(), right.body(), "{name} {range:?}: bytes");
    }
    // The statuses are the real ones, not two copies of the same failure.
    let status = |name: &str, range: Option<&str>| {
        answer(&asynchronous, &asset_request(&dir.join(name), range)).status()
    };
    assert_eq!(status("cloud.ply", None), 200);
    assert_eq!(status("cloud.ply", Some("bytes=0-99")), 206);
    assert_eq!(status("cloud.ply", Some("bytes=9-3")), 416);
    assert_eq!(status("missing.png", None), 404);
    assert_eq!(
        status("private.txt", None),
        403,
        "authorization is unchanged: not registered, not served"
    );
    // The page itself is served the same way.
    let index = Request::builder()
        .uri("tessel://localhost/index.html")
        .body(Vec::new())
        .unwrap();
    assert_eq!(
        answer(&asynchronous, &index).body(),
        answer(&sync, &index).body()
    );
}

#[test]
fn the_asset_io_event_has_the_same_keys_whichever_way_the_file_is_read() {
    let dist = page("io-keys");
    let dir = scratch("io-keys-files");
    let file = dir.join("clip.png");
    std::fs::write(&file, b"0123456789").unwrap();
    let access = Rc::new(RefCell::new(AssetAccess::default()));
    register(&access, &file);
    let pool = AssetPool::new(DEFAULT_WORKERS, DEFAULT_QUEUE);
    let keys = |reader: Reader| {
        let trace = TraceHandle::default();
        trace.set_enabled(true);
        let handle = handle(reader, &dist, &access, &trace);
        assert_eq!(answer(&handle, &asset_request(&file, None)).status(), 200);
        // A pooled read records its event on the worker after it answered; an inline one before `answer` returned.
        let read = events_after(&trace, "asset_io", 1).remove(0);
        let mut names: Vec<String> = read.as_object().unwrap().keys().cloned().collect();
        names.sort();
        (names, read)
    };
    let (inline, inline_event) = keys(Reader::Inline);
    let (pooled, pooled_event) = keys(Reader::Pooled(pool));
    assert_eq!(inline, pooled, "one event shape, one builder");
    for key in [
        "respond_ms",
        "queue_wait_ms",
        "depth_at_submit",
        "stale",
        "worker",
    ] {
        assert!(inline.iter().any(|name| name == key), "{key} in {inline:?}");
    }
    for event in [&inline_event, &pooled_event] {
        assert!(event["respond_ms"].is_number(), "{event}");
    }
    assert_eq!(inline_event["mode"], "sync");
    assert_eq!(inline_event["worker"], serde_json::Value::Null);
    assert_eq!(inline_event["queue_wait_ms"], 0.0);
    assert_eq!(inline_event["depth_at_submit"], 0);
    assert_eq!(inline_event["stale"], false);
    assert_eq!(pooled_event["mode"], "async");
}

/// A pool that cannot take a job (here, one that never got a worker) is answered on the thread of the callback, like a full
/// queue, and the `asset_io` event of that answer says so: the refusal does not run on a worker, whatever refused it.
#[test]
fn a_closed_pool_is_answered_503_on_the_callbacks_thread_and_its_event_says_so() {
    let dist = page("io-closed");
    let dir = scratch("io-closed-files");
    let file = dir.join("clip.png");
    std::fs::write(&file, b"0123456789").unwrap();
    let access = Rc::new(RefCell::new(AssetAccess::default()));
    register(&access, &file);
    let trace = TraceHandle::default();
    trace.set_enabled(true);
    let handle = handle(Reader::Pooled(AssetPool::closed()), &dist, &access, &trace);
    assert_eq!(answer(&handle, &asset_request(&file, None)).status(), 503);

    // Both events are this thread's own, written before `handle` returned: no worker to wait for.
    let callbacks = events_of(&trace, "asset_cb");
    assert_eq!(callbacks.len(), 1, "{callbacks:?}");
    assert_eq!(callbacks[0]["outcome"], "closed");
    assert_eq!(callbacks[0]["on_ui_thread"], true);
    let reads = events_of(&trace, "asset_io");
    assert_eq!(reads.len(), 1, "{reads:?}");
    assert_eq!(reads[0]["status"], 503);
    assert_eq!(reads[0]["on_ui_thread"], true);
    assert_eq!(reads[0]["worker"], Value::Null);
    assert_eq!(reads[0]["req"], callbacks[0]["req"]);
}

#[test]
fn the_report_names_the_mode_in_effect_and_the_page_spans_reach_the_trace_without_names() {
    use super::viewport_host::{HostOptions, ViewportHost};
    use super::viewport_host_tests::{from_page, ready_message, sent};
    use tessel_editor_core::EditorState;

    let state = EditorState::demo();
    let options = HostOptions {
        trace: true,
        asset_io: AssetIoMode::Async,
        ..HostOptions::default()
    };
    let mut host = ViewportHost::with_options(Ok(page("io-report")), options);
    from_page(&mut host, &state, json!({"type":"page.ready"}));
    sent(&mut host);
    from_page(&mut host, &state, ready_message());
    let report = host.report();
    assert_eq!(report["asset_io"]["mode"], "async");
    assert_eq!(report["asset_io"]["workers"], 2);
    assert_eq!(report["asset_io"]["queue"], 16);
    assert_eq!(report["asset_io"]["pool"]["saturated"], 0);
    let default = ViewportHost::with_options(Ok(page("io-report-default")), HostOptions::default());
    assert_eq!(
        default.report()["asset_io"]["mode"],
        "sync",
        "the default is the synchronous path"
    );
    assert!(
        default.report()["asset_io"]["pool"].is_null(),
        "no workers for the default"
    );

    from_page(
        &mut host,
        &state,
        json!({"type":"asset.state","id":"my-private-cloud","state":"ready","meta":{},
            "timings":{"load_ms":12.5,"fetch_ms":4.0,"decode_ms":8.0}}),
    );
    let mut done = super::viewport_host_tests::frame_done(9, 1);
    done["runtime_spans"] = json!([{"k":"warm","ms":3.0,"queued_ms":1.0,"started":1}]);
    from_page(&mut host, &state, done);
    let events = host.trace().events_since(0, 100);
    let text = events.to_string();
    assert!(
        !text.contains("my-private-cloud"),
        "an asset's name is not written to the trace: {text}"
    );
    let kinds: Vec<&str> = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["kind"].as_str().unwrap())
        .collect();
    assert!(
        kinds.contains(&"asset_state") && kinds.contains(&"runtime_span"),
        "{kinds:?}"
    );
    let state_event = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["kind"] == "asset_state")
        .unwrap();
    assert_eq!(state_event["fields"]["timings"]["decode_ms"], 8.0);
}

#[test]
fn the_page_account_of_a_frame_and_the_wait_of_its_message_reach_the_trace() {
    use super::viewport_host::{HostOptions, ViewportHost};
    use super::viewport_host_tests::{frame_done, from_page, ready_message, sent};
    use tessel_editor_core::EditorState;

    let state = EditorState::demo();
    let options = HostOptions {
        trace: true,
        ..HostOptions::default()
    };
    let mut host = ViewportHost::with_options(Ok(page("io-page-account")), options);
    from_page(&mut host, &state, json!({"type":"page.ready"}));
    sent(&mut host);
    from_page(&mut host, &state, ready_message());

    // A frame that comes without the account (a page that was not asked, or an older one) leaves no such event.
    from_page(&mut host, &state, frame_done(8, 1));
    let mut done = frame_done(9, 1);
    done["page"] = json!({
        "queue_ms": 31.5, "held_ms": 33.0, "ahead": {"frames": 0, "warms": 1, "loads": 1},
        "post_prev_ms": 0.2, "loop": {"n": 10, "max_ms": 28.0, "over": 1, "beat_ms": 4}
    });
    done["runtime_spans"] = json!([{"k":"loop_stall","ms":28.0,"ago_ms":12.0}]);
    host.on_ipc_waited(
        &done.to_string(),
        &state,
        Some(std::time::Duration::from_millis(7)),
    );

    let events = host.trace().events_since(0, 100);
    let pages: Vec<&serde_json::Value> = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["kind"] == "frame_page")
        .collect();
    assert_eq!(pages.len(), 1, "only the frame that carried the account");
    let fields = &pages[0]["fields"];
    assert_eq!(fields["id"], 9);
    assert_eq!(fields["page"]["queue_ms"], 31.5);
    assert_eq!(fields["page"]["ahead"]["warms"], 1);
    assert_eq!(fields["ipc_wait_ms"], 7.0);
    let stall = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["kind"] == "runtime_span" && event["fields"]["k"] == "loop_stall")
        .expect("the stall span is kept with the duration back from its hand-over");
    assert_eq!(stall["fields"]["ago_ms"], 12.0);
    let spans = host.trace().snapshot(1)["spans"].clone();
    assert!(
        spans["ipc_wait"]["max"].as_f64().unwrap() >= 7.0,
        "the wait is also a span with percentiles: {spans}"
    );
}

/// BL157.3: an `on` arm of the two first-use experiments is judged by what the PAGE says it did (`preupload` / `warm_draw` spans), and
/// the host is the only road those take to the trace the A/B harness reads. `runtime/test/fixtures/activation-spans.json` is the one
/// list of what the page can say: the runtime suite makes the real page say every item, `scripts/first-use-ab.spec.py` has the harness
/// judge an arm by it, and here every item must come out of the host as a `runtime_span` with `k`, `outcome` and `ms` untouched, in
/// order. Otherwise a working arm could be judged "never happened" (or the reverse) by a trace that lost its evidence on the way.
#[test]
fn every_outcome_the_page_can_report_for_the_two_experiments_reaches_the_trace_intact() {
    use super::viewport_host::{HostOptions, ViewportHost};
    use super::viewport_host_tests::{frame_done, from_page, ready_message, sent};
    use tessel_editor_core::EditorState;

    let contract: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../runtime/test/fixtures/activation-spans.json"
    )))
    .unwrap();
    // What the page sends with one frame: every outcome of both experiments, each with a duration of its own.
    let mut said: Vec<serde_json::Value> = Vec::new();
    for (k, spec) in contract["spans"].as_object().unwrap() {
        let mut outcomes = vec![spec["worked"].clone()];
        outcomes.extend(spec["not_worked"].as_array().unwrap().iter().cloned());
        for (n, outcome) in outcomes.into_iter().enumerate() {
            said.push(json!({"k": k, "outcome": outcome, "ms": 1.5 + n as f64}));
        }
    }
    assert!(
        said.len() >= 14,
        "both experiments list their outcomes: {said:?}"
    );

    let state = EditorState::demo();
    let options = HostOptions {
        trace: true,
        ..HostOptions::default()
    };
    let mut host = ViewportHost::with_options(Ok(page("io-activation-spans")), options);
    from_page(&mut host, &state, json!({"type":"page.ready"}));
    sent(&mut host);
    from_page(&mut host, &state, ready_message());
    let mut done = frame_done(9, 1);
    done["runtime_spans"] = json!(said);
    from_page(&mut host, &state, done);

    let events = host.trace().events_since(0, 500);
    let heard: Vec<&serde_json::Value> = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["kind"] == "runtime_span")
        .map(|event| &event["fields"])
        .collect();
    assert_eq!(
        heard.len(),
        said.len(),
        "one trace event per span the page sent"
    );
    for (heard, said) in heard.iter().zip(&said) {
        for key in ["k", "outcome", "ms"] {
            assert_eq!(
                heard[key], said[key],
                "`{key}` of {said} came out as {heard}"
            );
        }
    }
}

/// The controls that hold a read with a named pipe (Unix only).
#[cfg(unix)]
mod held_read {
    use super::*;
    use std::path::PathBuf;
    use std::sync::mpsc;

    use super::super::viewport_host::{HostOptions, ViewportHost};

    fn fifo(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        path
    }

    /// Lets every reader blocked in `open` proceed; the returned file keeps the pipe "written" until it is dropped.
    fn release(path: &Path) -> std::fs::File {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap()
    }

    enum Ui {
        Request(Box<Request<Vec<u8>>>, mpsc::Sender<u16>),
        Pulse(mpsc::Sender<()>),
        Stop,
    }

    /// A thread that plays the window's: it owns the (non-`Send`) handle, takes requests the way the WebView's callback does,
    /// and answers a pulse whenever it is free. The handle is made on it, so it is the thread the trace calls the window's.
    fn window_thread(
        reader: Reader,
        dist: PathBuf,
        held: PathBuf,
        trace: TraceHandle,
    ) -> (mpsc::Sender<Ui>, std::thread::JoinHandle<()>) {
        let (sender, receiver) = mpsc::channel::<Ui>();
        let join = std::thread::Builder::new()
            .name("window-sim".into())
            .spawn(move || {
                let access = Rc::new(RefCell::new(AssetAccess::default()));
                register(&access, &held);
                let handle = handle(reader, &dist, &access, &trace);
                while let Ok(message) = receiver.recv() {
                    match message {
                        Ui::Request(request, answered) => handle.handle(
                            &request,
                            Box::new(move |response| {
                                let _ = answered.send(response.status().as_u16());
                            }),
                        ),
                        Ui::Pulse(ack) => {
                            let _ = ack.send(());
                        }
                        Ui::Stop => break,
                    }
                }
            })
            .unwrap();
        (sender, join)
    }

    fn wait_until(what: &str, mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !ready() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// The control of the experiment: a read is held (the file does not answer), a pulse is sent to the window's thread, and
    /// the result says whether the pulse came back before the read was released. Proves the mechanism, not that a disk is
    /// what delays the real project.
    fn pulse_while_a_read_is_held(mode: AssetIoMode, tag: &str) -> (bool, TraceHandle) {
        let dist = page(tag);
        let dir = scratch(&format!("{tag}-files"));
        let held = fifo(&dir, "slow.bin");
        let trace = TraceHandle::default();
        trace.set_enabled(true);
        let reader = mode.reader();
        let pool = reader.pool().cloned();
        let (ui, join) = window_thread(reader, dist, held.clone(), trace.clone());
        let (answered, answers) = mpsc::channel();
        ui.send(Ui::Request(Box::new(asset_request(&held, None)), answered))
            .unwrap();
        if let Some(pool) = &pool {
            // The read really started and is being held before the pulse is sent.
            wait_until("the worker to hold the read", || {
                pool.stats().in_flight() == 1
            });
        }
        let (ack, pulses) = mpsc::channel();
        ui.send(Ui::Pulse(ack)).unwrap();
        // Asynchronous: the window answers at once, with the read still held. Synchronous: it cannot until the read ends, so
        // the bounded wait below ends with nothing (it can only miss by the window being busy, which is the point).
        let before_release = pulses
            .recv_timeout(if mode == AssetIoMode::Async {
                Duration::from_secs(20)
            } else {
                Duration::from_millis(300)
            })
            .is_ok();
        if let Some(pool) = &pool {
            assert_eq!(
                pool.stats().completed(),
                0,
                "the read was still held when the pulse returned"
            );
        }
        assert!(
            answers.try_recv().is_err(),
            "the held request is not answered before its file is"
        );
        let writer = release(&held);
        assert_eq!(answers.recv_timeout(Duration::from_secs(20)).unwrap(), 200);
        assert!(
            answers.recv_timeout(Duration::from_millis(50)).is_err(),
            "answered exactly once"
        );
        if !before_release {
            // The pulse that waited behind the held read comes back once the window is free again.
            assert!(pulses.recv_timeout(Duration::from_secs(20)).is_ok());
        }
        drop(writer);
        ui.send(Ui::Stop).unwrap();
        join.join().unwrap();
        (before_release, trace)
    }

    #[test]
    fn async_keeps_the_window_responsive_while_a_read_is_held() {
        let (responsive, trace) =
            pulse_while_a_read_is_held(AssetIoMode::Async, "io-async-control");
        assert!(
            responsive,
            "the window answered its pulse before the file was read"
        );
        // The trace says where each part ran: the callback on the window's thread, the read on a worker. The callback's event
        // is the window thread's own (it was joined); the read's is the worker's, recorded after its answer.
        let callbacks = events_of(&trace, "asset_cb");
        assert_eq!(callbacks.len(), 1);
        assert_eq!(callbacks[0]["on_ui_thread"], true);
        assert_eq!(callbacks[0]["thread"], "window-sim");
        assert_eq!(callbacks[0]["outcome"], "queued");
        let reads = events_after(&trace, "asset_io", 1);
        assert_eq!(reads.len(), 1);
        assert_eq!(reads[0]["on_ui_thread"], false);
        assert!(
            reads[0]["thread"]
                .as_str()
                .unwrap()
                .starts_with("tessel-asset-io-")
        );
        assert_eq!(
            reads[0]["req"], callbacks[0]["req"],
            "one id correlates the two"
        );
        assert_eq!(reads[0]["stale"], false);
    }

    #[test]
    fn sync_blocks_the_window_while_the_same_read_is_held() {
        // The negative control: the same held read, the other mode. If this ever passed the pulse, the control would not be
        // measuring what the experiment changes.
        let (responsive, trace) = pulse_while_a_read_is_held(AssetIoMode::Sync, "io-sync-control");
        assert!(
            !responsive,
            "the window cannot answer while it is the one reading"
        );
        // The window's own thread recorded it, and that thread was joined before this: no worker to wait for.
        let reads = events_of(&trace, "asset_io");
        assert_eq!(reads.len(), 1);
        assert_eq!(
            reads[0]["on_ui_thread"], true,
            "the read ran on the window's thread"
        );
        assert_eq!(reads[0]["mode"], "sync");
    }

    #[test]
    fn a_full_queue_is_answered_503_once_counted_and_never_dropped() {
        let dist = page("io-full");
        let dir = scratch("io-full-files");
        let held = fifo(&dir, "slow.bin");
        let access = Rc::new(RefCell::new(AssetAccess::default()));
        register(&access, &held);
        let trace = TraceHandle::default();
        trace.set_enabled(true);
        let pool = AssetPool::new(1, 1);
        let asynchronous = handle(Reader::Pooled(pool.clone()), &dist, &access, &trace);
        let (tx, rx) = mpsc::channel();
        let send = |tag: &'static str| {
            let tx = tx.clone();
            asynchronous.handle(
                &asset_request(&held, None),
                Box::new(move |response| {
                    let _ = tx.send((tag, response.status().as_u16()));
                }),
            );
        };
        send("running");
        wait_until("the worker to hold the read", || {
            pool.stats().in_flight() == 1
        });
        send("queued");
        // The third does not fit: answered at once, by the callback, with an explicit status.
        send("refused");
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            ("refused", 503)
        );
        assert_eq!(pool.stats().saturated(), 1);
        let callbacks = events_of(&trace, "asset_cb");
        assert_eq!(callbacks.last().unwrap()["outcome"], "saturated");
        // The refusal is answered on the thread of the callback, and its `asset_io` event says so (it ran there, not on a
        // worker): the same event that carries the thread name carries the matching flag. `handle` wrote it before it
        // returned, on this very thread, and the two held reads have not answered: no worker to wait for.
        let reads = events_of(&trace, "asset_io");
        let refused: Vec<&Value> = reads.iter().filter(|read| read["status"] == 503).collect();
        assert_eq!(refused.len(), 1, "{reads:?}");
        assert_eq!(refused[0]["on_ui_thread"], true);
        assert_eq!(refused[0]["worker"], Value::Null);
        assert_eq!(refused[0]["req"], callbacks.last().unwrap()["req"]);
        let writer = release(&held);
        let mut rest = vec![
            rx.recv_timeout(Duration::from_secs(20)).unwrap(),
            rx.recv_timeout(Duration::from_secs(20)).unwrap(),
        ];
        drop(writer);
        rest.sort();
        assert_eq!(rest, vec![("queued", 200), ("running", 200)]);
        assert!(
            rx.recv_timeout(Duration::from_millis(50)).is_err(),
            "every request answered once"
        );
    }

    #[test]
    fn a_project_change_answers_waiting_and_running_reads_gone() {
        let dist = page("io-stale");
        let dir = scratch("io-stale-files");
        let held = fifo(&dir, "slow.bin");
        let access = Rc::new(RefCell::new(AssetAccess::default()));
        register(&access, &held);
        let trace = TraceHandle::default();
        let pool = AssetPool::new(1, 4);
        let asynchronous = handle(Reader::Pooled(pool.clone()), &dist, &access, &trace);
        let (tx, rx) = mpsc::channel();
        let send = |tag: &'static str| {
            let tx = tx.clone();
            asynchronous.handle(
                &asset_request(&held, None),
                Box::new(move |response| {
                    let _ = tx.send((tag, response.status().as_u16()));
                }),
            );
        };
        send("running");
        wait_until("the worker to hold the read", || {
            pool.stats().in_flight() == 1
        });
        send("waiting");
        // What `reset_project` and a reload do: nothing the old session asked for may be applied to the new one.
        pool.invalidate();
        let writer = release(&held);
        let mut answers = vec![
            rx.recv_timeout(Duration::from_secs(20)).unwrap(),
            rx.recv_timeout(Duration::from_secs(20)).unwrap(),
        ];
        drop(writer);
        answers.sort();
        assert_eq!(answers, vec![("running", 410), ("waiting", 410)]);
        assert_eq!(pool.stats().stale(), 2);
    }

    /// The pool's own `invalidate` is not what the editor calls: the host does, from `reset_project` and `begin_reload`.
    /// Dropping those calls would leave every other test green, so each is exercised here with a read in flight and one
    /// waiting behind it, through the protocol handle the host hands to the WebView.
    fn host_change_answers_reads_gone(tag: &str, change: impl FnOnce(&mut ViewportHost)) {
        let mut host = ViewportHost::with_options(
            Ok(page(tag)),
            HostOptions {
                asset_io: AssetIoMode::Async,
                ..HostOptions::default()
            },
        );
        let dir = scratch(&format!("{tag}-files"));
        let held = fifo(&dir, "slow.bin");
        host.allow_file(&held);
        let handle = host.protocol_handle();
        let (tx, rx) = mpsc::channel();
        let send = |tag: &'static str| {
            let tx = tx.clone();
            handle.handle(
                &asset_request(&held, None),
                Box::new(move |response| {
                    let _ = tx.send((tag, response.status().as_u16()));
                }),
            );
        };
        send("running");
        let report = || host.report()["asset_io"]["pool"]["in_flight"].as_u64();
        wait_until("a worker to hold the read", || report() == Some(1));
        send("waiting");
        change(&mut host);
        let writer = release(&held);
        let mut answers = vec![
            rx.recv_timeout(Duration::from_secs(20)).unwrap(),
            rx.recv_timeout(Duration::from_secs(20)).unwrap(),
        ];
        drop(writer);
        answers.sort();
        assert_eq!(answers, vec![("running", 410), ("waiting", 410)]);
        assert_eq!(host.report()["asset_io"]["pool"]["stale"], 2);
    }

    #[test]
    fn replacing_the_project_never_refuses_the_page_its_own_bundle() {
        let mut host = ViewportHost::with_options(
            Ok(page("io-bundle")),
            HostOptions {
                asset_io: AssetIoMode::Async,
                ..HostOptions::default()
            },
        );
        let handle = host.protocol_handle();
        host.reset_project();
        for uri in [
            "tessel://localhost/runtime.js",
            "tessel://localhost/index.html",
        ] {
            let (tx, rx) = mpsc::channel();
            handle.handle(
                &Request::builder().uri(uri).body(Vec::new()).unwrap(),
                Box::new(move |response| {
                    let _ = tx.send(response.status().as_u16());
                }),
            );
            assert_eq!(
                rx.try_recv(),
                Ok(200),
                "{uri} is answered on the calling thread, after the change as before it"
            );
        }
        assert_eq!(
            host.report()["asset_io"]["pool"]["submitted"],
            0,
            "the bundle never went through the pool"
        );
    }

    #[test]
    fn replacing_the_project_through_the_host_answers_reads_gone() {
        host_change_answers_reads_gone("io-host-reset", |host| host.reset_project());
    }

    #[test]
    fn reloading_the_page_through_the_host_answers_reads_gone() {
        host_change_answers_reads_gone("io-host-reload", |host| {
            assert!(host.begin_reload());
        });
    }
}
