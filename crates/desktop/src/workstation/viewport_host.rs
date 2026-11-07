//! The persistent viewport: one WebView running the Three.js runtime, driven through the
//! same bridge and session the headless CLI uses.
//!
//! `ViewportHost` holds no GPUI type. It turns editor state into frame requests, decodes what
//! the page answers and keeps the status the monitor and the automation tree report; the
//! workstation owns the `WebView` entity and only moves strings between it and this type
//! ([`ViewportHost::take_outgoing`] / [`ViewportHost::on_ipc`]). That keeps the lifecycle rules
//! (boot, handshake, stale answers, timeouts, context loss) testable without a window.
//!
//! WHY nothing here blocks: the page answers on the same thread that asks, so any wait for a
//! reply would deadlock. The handshake is therefore driven by events, and the blocking
//! `ViewportSession::capabilities()` is never called (see `known_capabilities`).

#[path = "scene_monitor_host.rs"]
mod directed;

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use async_channel::{Receiver, Sender};
use serde_json::{Value, json};
use tessel_editor_core::CommandError;
use tessel_editor_core::backend::{BackendError, FrameDetail, FrameResult, Output};
use tessel_editor_core::bridge::{BridgeBackend, HelloOptions, Transport};
use tessel_editor_core::headless::{backend_error, frame_geometry, frame_report, plan_summary};
use tessel_editor_core::ops::RenderFrame;
use tessel_editor_core::plan::{PlanRequest, RenderPlan};
use tessel_editor_core::session::{Preview, SessionEvent, Status, ViewportSession};
use tessel_editor_core::state::EditorState;
use tessel_editor_core::trace::TraceHandle;
use tessel_editor_render::SetupError;
use tessel_editor_render::host::{AssetAccess, Served, serve, write_png};
use wry::http::{Request, Response};

/// Scheme and host the page is served from. A custom scheme keeps the page same-origin with
/// its assets and unreachable from any web page.
pub const PAGE_URL: &str = "tessel://localhost/index.html";
pub const SCHEME: &str = "tessel";

/// The page has this long to load `runtime.js` and announce itself.
const BOOT_TIMEOUT: Duration = Duration::from_secs(15);
/// WebGL context creation and the `ready` answer.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
/// Loading assets and drawing one frame.
const FRAME_TIMEOUT: Duration = Duration::from_secs(30);
/// Longest side of the picture the monitor renders itself; the CSS fit scales it to the pane.
const PREVIEW_LONG_SIDE: u32 = 960;
/// Loaded assets kept between frames before the ones the plan no longer needs are released.
const RETAINED_ASSETS: usize = 16;
/// Set to `1` to make the monitor's own frames carry hash/mean like explicit ones (measurement
/// and pixel proofs); off by default because the readback is not needed to show a picture.
pub const INSPECT_ENV: &str = "TESSEL_VIEWPORT_INSPECT";
/// Set to `1` to time the host side of every preview frame (plan, serialize, dispatch, transit,
/// accept, script evaluation, asset serving) and to ask the runtime for `frame.presented`.
/// Reported under `trace` in `render.status` and in the `monitor.viewport` node. Off by default.
pub const TRACE_ENV: &str = "TESSEL_VIEWPORT_TRACE";
/// Set to `0` to run the runtime's per-frame create/dispose path instead of GPU retention, to
/// compare the two in the same build or to roll back without a new release.
pub const RETAIN_ENV: &str = "TESSEL_VIEWPORT_RETAIN";
/// Set to `0` to make every splat scene frame sort and draw again (rollback of the result cache).
pub const RESULT_CACHE_ENV: &str = "TESSEL_VIEWPORT_RESULT_CACHE";
/// `0`, `2` or `4` MSAA samples for scene targets. Unset keeps the runtime's 4 (OPT07 measured
/// fewer samples as a real gain on large splat scenes but a visible edge-quality trade).
pub const SCENE_SAMPLES_ENV: &str = "TESSEL_VIEWPORT_SCENE_SAMPLES";
/// `0`, `2` or `4` MSAA samples of scene layers in the monitor's interactive frames (playback and
/// scrub, BL129); the picture is refined to the final count when the playhead stops. Unset keeps the
/// runtime's default; `4` turns the reduction off (A/B and rollback). Explicit frames and export
/// never read it: they draw at the final count.
pub const PREVIEW_SCENE_SAMPLES_ENV: &str = "TESSEL_VIEWPORT_PREVIEW_SCENE_SAMPLES";

/// Opt-ins read once when the host is created.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HostOptions {
    pub trace: bool,
    /// `Some(false)` disables the runtime's GPU retention; `None` keeps its default.
    pub retain: Option<bool>,
    pub result_cache: Option<bool>,
    pub scene_samples: Option<u32>,
    pub preview_scene_samples: Option<u32>,
}

/// A MSAA sample count the runtime accepts, from an environment value.
fn samples_from_env(name: &str) -> Option<u32> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|samples| [0, 2, 4].contains(samples))
}

impl HostOptions {
    pub fn from_env() -> Self {
        Self {
            trace: std::env::var(TRACE_ENV).is_ok_and(|value| value == "1"),
            retain: std::env::var(RETAIN_ENV)
                .is_ok_and(|value| value == "0")
                .then_some(false),
            result_cache: std::env::var(RESULT_CACHE_ENV)
                .is_ok_and(|value| value == "0")
                .then_some(false),
            scene_samples: samples_from_env(SCENE_SAMPLES_ENV),
            preview_scene_samples: samples_from_env(PREVIEW_SCENE_SAMPLES_ENV),
        }
    }
}

pub type FrameWaiter = Receiver<Result<Value, CommandError>>;

#[derive(Debug, Clone, PartialEq)]
enum Phase {
    /// No page to load (the bundle is missing) or the runtime closed.
    Unavailable(BackendError),
    Booting(Instant),
    Handshaking(Instant),
    Ready,
}

/// The bridge carried by the WebView: messages to the page are queued as scripts, messages
/// from the page arrive through [`ViewportHost::on_ipc`].
pub struct WebTransport {
    outbox: Rc<RefCell<VecDeque<String>>>,
    inbox: Rc<RefCell<VecDeque<Value>>>,
    access: Rc<RefCell<AssetAccess>>,
}

/// What the monitor's header adds when the picture is not the final quality: a proxy copy of a
/// video, and/or a scene drawn with fewer MSAA samples (the refine replaces both once the playhead
/// stops). Empty for a final-quality picture.
fn reduced_label(detail: &FrameDetail) -> String {
    let mut label = String::new();
    if detail.proxy_layers() > 0 {
        label.push_str(" · proxy");
    }
    if detail.reduced_scene_layers() > 0 {
        match detail.scene_samples() {
            Some(samples) => label.push_str(&format!(" · MSAA {samples}")),
            None => label.push_str(" · MSAA"),
        }
    }
    label
}

/// JSON is almost JavaScript: only these two separators are legal in JSON strings and not
/// in a JS source text.
fn script_literal(message: &Value) -> String {
    message
        .to_string()
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

impl Transport for WebTransport {
    fn send(&mut self, message: &Value) -> Result<(), BackendError> {
        if message["type"] == "asset.load" {
            // The page may only fetch what a load message named.
            self.access.borrow_mut().register(&message["asset"]);
        }
        if message["type"] == "asset.proxy" && !message["proxy"].is_null() {
            // A proxy is read like a frame sequence, and only after the host attached it.
            self.access.borrow_mut().register(&json!({
                "source": { "type": "frame_sequence", "pattern": message["proxy"]["pattern"] }
            }));
        }
        self.outbox.borrow_mut().push_back(format!(
            "window.TesselRuntime.receive({})",
            script_literal(message)
        ));
        Ok(())
    }

    fn recv(&mut self, _timeout: Duration) -> Option<Value> {
        self.inbox.borrow_mut().pop_front()
    }

    fn close(&mut self) {
        self.inbox.borrow_mut().clear();
    }
}

struct Waiter {
    request: u64,
    reply: Sender<Result<Value, CommandError>>,
    summary: Value,
}

pub struct ViewportHost {
    directed: directed::DirectedViewport,
    dist: Option<PathBuf>,
    session: ViewportSession<BridgeBackend<WebTransport>>,
    outbox: Rc<RefCell<VecDeque<String>>>,
    inbox: Rc<RefCell<VecDeque<Value>>>,
    access: Rc<RefCell<AssetAccess>>,
    phase: Phase,
    /// The last plan asked for, so a command that changes nothing the picture shows (selection,
    /// zoom, dock edits) does not render again.
    last_key: Option<String>,
    /// A change happened while the runtime was not ready; render once it is.
    dirty: bool,
    png_paths: HashMap<u64, PathBuf>,
    waiters: Vec<Waiter>,
    request_started: Option<Instant>,
    /// The last frame request that failed; cleared by the next one that works.
    failure: Option<BackendError>,
    last_frame: Option<FrameResult>,
    output_size: Option<(u32, u32)>,
    /// Whether the monitor is on screen; reported, and used to skip work while hidden.
    pub on_screen: bool,
    /// Messages from the page that changed what the monitor shows (and so repainted the window)
    /// and those that did not (`frame.presented`, asset and proxy acknowledgements, a frame that
    /// took the same rounded milliseconds). OPT04: the window only repaints for the former.
    ui_repaints: u64,
    ui_skipped: u64,
}

impl ViewportHost {
    pub fn new(page: Result<PathBuf, SetupError>) -> Self {
        Self::with_options(page, HostOptions::from_env())
    }

    pub fn with_options(page: Result<PathBuf, SetupError>, options: HostOptions) -> Self {
        let outbox = Rc::new(RefCell::new(VecDeque::new()));
        let inbox = Rc::new(RefCell::new(VecDeque::new()));
        let access = Rc::new(RefCell::new(AssetAccess::default()));
        let transport = WebTransport {
            outbox: outbox.clone(),
            inbox: inbox.clone(),
            access: access.clone(),
        };
        let backend = BridgeBackend::new("three-webview", transport, HANDSHAKE_TIMEOUT)
            .with_options(HelloOptions {
                retain: options.retain,
                presented: options.trace,
                result_cache: options.result_cache,
                scene_samples: options.scene_samples,
                preview_scene_samples: options.preview_scene_samples,
            });
        let mut session =
            ViewportSession::new(backend).with_demo_dir(tessel_editor_render::demo_dir());
        session.set_retained_assets(RETAINED_ASSETS);
        session.trace().set_enabled(options.trace);
        let (dist, phase) = match page {
            Ok(dist) => (Some(dist), Phase::Booting(Instant::now())),
            Err(error) => (
                None,
                Phase::Unavailable(BackendError::new("backend_unavailable", error.message())),
            ),
        };
        let mut host = Self {
            dist,
            session,
            outbox,
            inbox,
            access,
            phase,
            last_key: None,
            dirty: true,
            png_paths: HashMap::new(),
            waiters: Vec::new(),
            request_started: None,
            failure: None,
            last_frame: None,
            output_size: None,
            on_screen: true,
            directed: directed::DirectedViewport::default(),
            ui_repaints: 0,
            ui_skipped: 0,
        };
        host.set_inspect_previews(std::env::var(INSPECT_ENV).is_ok_and(|value| value == "1"));
        host
    }

    /// The recorder of host-side spans (see [`TRACE_ENV`]); the window records its own into it.
    pub fn trace(&self) -> TraceHandle {
        self.session.trace().clone()
    }

    /// Makes the monitor's frames carry hash/mean like explicit ones (see [`INSPECT_ENV`]).
    pub fn set_inspect_previews(&mut self, inspect: bool) {
        self.session.set_preview_inspection(inspect);
    }

    /// Whether a WebView should exist at all.
    pub fn has_page(&self) -> bool {
        self.dist.is_some()
    }

    pub fn is_ready(&self) -> bool {
        self.phase == Phase::Ready
    }

    /// What the protocol handler needs, detached from the host so its closure is `'static`.
    pub fn protocol_handle(&self) -> ProtocolHandle {
        ProtocolHandle {
            dist: self.dist.clone(),
            access: self.access.clone(),
            trace: self.trace(),
        }
    }

    /// Scripts to run in the page, in order.
    pub fn take_outgoing(&mut self) -> Vec<String> {
        self.outbox.borrow_mut().drain(..).collect()
    }

    /// The page could not run a script (the view is gone or still navigating). The runtime
    /// cannot be trusted to have seen the message, so the viewport reports it and stops.
    pub fn on_script_error(&mut self, message: &str) {
        self.set_unavailable(BackendError::new(
            "backend_unavailable",
            format!("The viewport page rejected a message: {message}"),
        ));
    }

    /// Reloads the page: a closed or broken runtime comes back without restarting the app.
    /// Returns whether a page exists to reload.
    pub fn begin_reload(&mut self) -> bool {
        let Some(_) = &self.dist else {
            return false;
        };
        self.forget_directed_runtime(&BackendError::new("cancelled", "The viewport is reloading"));
        self.session.reset_project();
        self.access.borrow_mut().clear();
        self.inbox.borrow_mut().clear();
        self.outbox.borrow_mut().clear();
        self.png_paths.clear();
        self.fail_waiters(&BackendError::new("cancelled", "The viewport is reloading"));
        self.phase = Phase::Booting(Instant::now());
        self.last_key = None;
        self.dirty = true;
        self.last_frame = None;
        true
    }

    /// The project was replaced: IDs may now mean something else.
    pub fn reset_project(&mut self) {
        self.clear_directed();
        self.fail_waiters(&BackendError::new("cancelled", "The project was replaced"));
        self.session.reset_project();
        self.access.borrow_mut().clear();
        self.last_key = None;
        self.last_frame = None;
        self.dirty = true;
    }

    /// Closes the runtime before the window goes away.
    pub fn shutdown(&mut self) {
        self.clear_directed();
        self.fail_waiters(&BackendError::new("cancelled", "The viewport is closing"));
        if self.phase == Phase::Ready || matches!(self.phase, Phase::Handshaking(_)) {
            self.session.shutdown();
        }
        self.take_outgoing();
    }

    fn set_unavailable(&mut self, error: BackendError) {
        self.fail_waiters(&error);
        self.phase = Phase::Unavailable(error);
    }

    fn fail_waiters(&mut self, error: &BackendError) {
        for waiter in self.waiters.drain(..) {
            let _ = waiter.reply.try_send(Err(backend_error(error.clone())));
        }
    }

    /// Output size for the monitor's own picture: the sequence's aspect at a bounded size.
    fn preview_size(plan: &RenderPlan) -> (u32, u32) {
        let (width, height) = (plan.width.max(1), plan.height.max(1));
        let long = width.max(height);
        if long <= PREVIEW_LONG_SIDE {
            return (width, height);
        }
        let scale = f64::from(PREVIEW_LONG_SIDE) / f64::from(long);
        (
            ((f64::from(width) * scale).round() as u32).max(1),
            ((f64::from(height) * scale).round() as u32).max(1),
        )
    }

    /// Follows the editor state: asks for a new picture if what the monitor shows changed.
    pub fn sync(&mut self, state: &EditorState) {
        if self.directed_scene().is_some() {
            self.dispatch_directed();
            return;
        }
        if !self.is_ready() {
            self.dirty = true;
            return;
        }
        let trace = self.session.trace().clone();
        let sync_started = trace.start();
        self.session.set_preview_policy(state.preview_policy);
        let planned = trace.start();
        let plan = state.plan(&PlanRequest::of(state));
        if let Some(started) = planned {
            trace.note_plan(started.elapsed());
        }
        let plan = match plan {
            Ok(plan) => plan,
            Err(error) => {
                // A plan the core refuses (e.g. a preview of an asset that is gone) is shown,
                // not retried: the next state change plans again.
                self.last_key = None;
                self.report_failure(BackendError::new("render_failed", error.to_string()));
                return;
            }
        };
        let size = Self::preview_size(&plan);
        let keyed = trace.start();
        // The policy is part of what the picture is made of: changing it redraws the monitor.
        let key = format!(
            "{}|{}x{}|{}",
            plan.to_json(),
            size.0,
            size.1,
            state.preview_policy.to_json()
        );
        trace.span("plan_key", keyed);
        let busy = matches!(
            self.session.status(),
            Status::Ready(_) | Status::Rendering(_) | Status::Loading(_)
        );
        if !self.dirty && busy && self.last_key.as_deref() == Some(key.as_str()) {
            trace.span("sync_unchanged", sync_started);
            return;
        }
        self.dirty = false;
        self.last_key = Some(key);
        self.output_size = Some(size);
        let output = Output {
            width: size.0,
            height: size.1,
            png: None,
        };
        self.request_preview(state, plan, output);
        trace.span("sync", sync_started);
    }

    /// The monitor's own frame. A frame already rendering is left to finish when only the
    /// playhead moved (the session parks the newest intent), so a scrub keeps presenting.
    /// Anything that supersedes it answers the explicit waiter, if any, as cancelled.
    fn request_preview(&mut self, state: &EditorState, plan: RenderPlan, output: Output) {
        match self.session.request_preview(state, plan, output) {
            // Only an interactive frame is ever coalesced into, and explicit requests (the ones
            // with a waiter) are never interactive: nothing to answer here.
            Ok(Preview::Coalesced) => {}
            Ok(Preview::Submitted(_)) => {
                self.request_started = Some(Instant::now());
                self.failure = None;
                self.png_paths.clear();
                Self::cancel_waiters(std::mem::take(&mut self.waiters));
            }
            Err(error) => {
                Self::cancel_waiters(std::mem::take(&mut self.waiters));
                self.report_failure(error);
            }
        }
    }

    /// Superseded explicit requests are answered as cancelled, not left hanging.
    fn cancel_waiters(waiters: Vec<Waiter>) {
        for old in waiters {
            let error = BackendError::new("cancelled", "A newer frame request replaced this one");
            let _ = old.reply.try_send(Err(backend_error(error)));
        }
    }

    fn report_failure(&mut self, error: BackendError) {
        self.session.cancel();
        self.request_started = None;
        self.last_frame = None;
        self.png_paths.clear();
        self.fail_waiters(&error);
        self.failure = Some(error);
    }

    fn request(
        &mut self,
        state: &EditorState,
        plan: RenderPlan,
        output: Output,
        probes: Vec<(u32, u32)>,
        waiter: Option<(Sender<Result<Value, CommandError>>, Value)>,
    ) -> Result<u64, BackendError> {
        let png = output.png.clone();
        // The request in flight is superseded by this one; its waiter must not wait forever.
        Self::cancel_waiters(std::mem::take(&mut self.waiters));
        match self.session.request_frame(state, plan, output, probes) {
            Ok(id) => {
                self.request_started = Some(Instant::now());
                self.failure = None;
                if let Some(path) = png {
                    self.png_paths.insert(id, PathBuf::from(path));
                }
                if let Some((reply, summary)) = waiter {
                    self.waiters.push(Waiter {
                        request: id,
                        reply,
                        summary,
                    });
                }
                Ok(id)
            }
            Err(error) => {
                self.report_failure(error.clone());
                Err(error)
            }
        }
    }

    /// An explicit `render.frame`: the answer arrives on the returned channel, after the
    /// monitor has actually drawn it.
    pub fn render_frame(
        &mut self,
        state: &EditorState,
        frame: &RenderFrame,
    ) -> Result<FrameWaiter, CommandError> {
        let plan = state.plan(&frame.plan)?;
        match &self.phase {
            Phase::Ready => {}
            Phase::Unavailable(error) => return Err(backend_error(error.clone())),
            Phase::Booting(_) | Phase::Handshaking(_) => {
                return Err(CommandError::new(
                    "backend_unavailable",
                    "The viewport is still starting; try again in a moment",
                ));
            }
        }
        let (width, height) = frame_geometry(&plan, frame)?;
        let summary = plan_summary(&plan);
        let output = Output {
            width,
            height,
            png: frame.out.as_ref().map(|path| {
                std::path::absolute(path)
                    .unwrap_or_else(|_| path.clone())
                    .to_string_lossy()
                    .into_owned()
            }),
        };
        let (reply, receiver) = async_channel::bounded(1);
        self.dirty = false;
        // The next `sync` must re-evaluate against what is on the canvas now.
        self.last_key = None;
        self.output_size = Some((width, height));
        self.request(
            state,
            plan,
            output,
            frame.probes.clone(),
            Some((reply, summary)),
        )
        .map_err(backend_error)?;
        Ok(receiver)
    }

    /// One message from the page.
    pub fn on_ipc(&mut self, raw: &str, state: &EditorState) {
        let trace = self.session.trace().clone();
        let started = trace.start();
        self.handle_ipc(raw, state);
        trace.span("ipc", started);
    }

    fn handle_ipc(&mut self, raw: &str, state: &EditorState) {
        let Ok(mut message) = serde_json::from_str::<Value>(raw) else {
            // A page that speaks something else is not our runtime.
            self.set_unavailable(BackendError::new(
                "protocol_error",
                "The viewport page sent a message that is not JSON",
            ));
            return;
        };
        // WHY: after a reload the old document may still have messages queued; until the new page
        // announces itself, everything else belongs to the page that is gone (its frames, asset
        // states and `ready`) and must not touch the new session.
        if matches!(self.phase, Phase::Booting(_)) && message["type"] != "page.ready" {
            return;
        }
        match message["type"].as_str() {
            Some("page.ready") => {
                if matches!(self.phase, Phase::Booting(_)) {
                    self.phase = Phase::Handshaking(Instant::now());
                    let backend = self.session.backend_mut();
                    let message = backend.hello_message("tessel-editor");
                    let _ = backend.transport_mut().send(&message);
                }
                return;
            }
            Some("frame.done") => self.attach_png(&mut message),
            _ => {}
        }
        if self.directed_ipc(&message) {
            return;
        }
        self.inbox.borrow_mut().push_back(message);
        self.drain(state);
    }

    /// The page wrote the PNG as base64; this side owns the filesystem.
    fn attach_png(&mut self, message: &mut Value) {
        let Some(data) = message
            .get("png_data")
            .and_then(Value::as_str)
            .map(String::from)
        else {
            return;
        };
        message
            .as_object_mut()
            .map(|object| object.remove("png_data"));
        let Some(id) = message["id"].as_u64() else {
            return;
        };
        let Some(path) = self.png_paths.remove(&id) else {
            return;
        };
        match write_png(&path, &data) {
            Ok(()) => message["png"] = json!(path.to_string_lossy()),
            Err(error) => {
                *message = json!({
                    "v": 1, "type": "frame.failed", "id": id, "revision": message["revision"],
                    "error": { "code": "render_failed", "message": error },
                });
            }
        }
    }

    /// Processes every message already received.
    pub fn drain(&mut self, state: &EditorState) {
        while let Some(event) = self.session.pump(Duration::ZERO) {
            self.on_event(event);
        }
        self.dispatch_pending(state);
        self.dispatch_directed();
        if matches!(self.phase, Phase::Handshaking(_))
            && self.session.known_capabilities().is_some()
        {
            self.phase = Phase::Ready;
            self.dirty = true;
        }
        if self.session.needs_redraw {
            // The context came back without its picture (and without its textures).
            self.session.needs_redraw = false;
            self.dirty = true;
        }
        if self.dirty && self.is_ready() {
            self.sync(state);
        }
    }

    /// The frame in flight was accepted: start the newest intent that waited for it.
    fn dispatch_pending(&mut self, state: &EditorState) {
        if !self.is_ready() {
            return;
        }
        match self.session.dispatch_pending(state) {
            Some(Ok(_)) => {
                self.request_started = Some(Instant::now());
                self.failure = None;
            }
            Some(Err(error)) => self.report_failure(error),
            None => {}
        }
    }

    /// What the monitor's header and error area show right now: when a message from the page
    /// leaves this unchanged, nothing on the GPUI side needs to repaint (the picture itself is the
    /// WebView's canvas, which the page draws).
    pub fn ui_signature(&self) -> String {
        let phase = format!("{:?}", self.phase);
        let failure = self.failure.as_ref().map(|error| error.code.as_str());
        format!("{}|{phase}|{failure:?}", self.headline())
    }

    /// Counts one message from the page as repainting or not.
    pub fn note_ui(&mut self, repainted: bool) {
        if repainted {
            self.ui_repaints += 1;
        } else {
            self.ui_skipped += 1;
        }
    }

    /// Whether the picture on screen is a proxy one that the original would replace.
    pub fn refine_pending(&self) -> bool {
        self.session.refine_pending()
    }

    /// Replaces a proxy picture with the original once the scrub settled. `true` when a request
    /// started (the caller flushes the scripts).
    pub fn refine(&mut self, state: &EditorState, now: Instant) -> bool {
        if !self.is_ready() {
            return false;
        }
        match self.session.dispatch_refine(state, now) {
            Some(Ok(_)) => {
                self.request_started = Some(now);
                self.failure = None;
                true
            }
            Some(Err(error)) => {
                self.reset_directed_runtime();
                self.report_failure(error);
                true
            }
            None => false,
        }
    }

    fn on_event(&mut self, event: SessionEvent) {
        match event {
            SessionEvent::Progress | SessionEvent::Stale(_) => {}
            SessionEvent::Frame(frame) => {
                self.request_started = None;
                self.failure = None;
                self.png_paths.remove(&frame.id);
                if let Some(index) = self
                    .waiters
                    .iter()
                    .position(|waiter| waiter.request == frame.id)
                {
                    let waiter = self.waiters.remove(index);
                    let _ = waiter.reply.try_send(frame_report(&waiter.summary, &frame));
                }
                self.last_frame = Some(frame);
            }
            SessionEvent::Failed(error) => {
                self.request_started = None;
                self.png_paths.clear();
                if error.code == "backend_unavailable"
                    || error.code == "protocol_error"
                    || error.code == "version_mismatch"
                {
                    self.set_unavailable(error);
                    return;
                }
                self.fail_waiters(&error);
                self.failure = Some(error);
            }
        }
    }

    /// Time-based failures; returns whether anything the UI shows changed.
    pub fn tick(&mut self, now: Instant) -> bool {
        match self.phase.clone() {
            Phase::Booting(since) if now.duration_since(since) > BOOT_TIMEOUT => {
                self.set_unavailable(BackendError::new(
                    "timeout",
                    "The viewport page did not start. Rebuild it with: pnpm --dir runtime run build",
                ));
                true
            }
            Phase::Handshaking(since) if now.duration_since(since) > HANDSHAKE_TIMEOUT => {
                self.set_unavailable(BackendError::new(
                    "timeout",
                    "The viewport runtime did not answer the handshake (WebGL may be unavailable)",
                ));
                true
            }
            Phase::Ready => {
                let Some(started) = self.request_started else {
                    return false;
                };
                if now.duration_since(started) <= FRAME_TIMEOUT {
                    return false;
                }
                let error = BackendError::new(
                    "timeout",
                    format!("No frame within {FRAME_TIMEOUT:?}; the request was cancelled"),
                );
                self.reset_directed_runtime_for(&error);
                self.report_failure(error);
                true
            }
            _ => false,
        }
    }

    /// Status for the monitor header and for `ui get monitor.viewport`.
    pub fn headline(&self) -> String {
        match &self.phase {
            Phase::Unavailable(_) => "Visualizador indisponível".to_string(),
            Phase::Booting(_) | Phase::Handshaking(_) => "Iniciando visualizador…".to_string(),
            Phase::Ready => {
                if let Some(error) = &self.failure {
                    return format!("Falha: {}", error.code);
                }
                match self.session.status() {
                    Status::Loading(_) => "Carregando mídia…".to_string(),
                    Status::Rendering(_) => "Renderizando…".to_string(),
                    _ => match &self.last_frame {
                        Some(frame) => format!(
                            "Three.js · {}×{} · {:.0} ms{}",
                            frame.width,
                            frame.height,
                            frame.render_ms,
                            // Never hide that the picture is a preview copy.
                            reduced_label(&frame.detail)
                        ),
                        None => "Aguardando quadro".to_string(),
                    },
                }
            }
        }
    }

    /// The error to show in place of the picture, if the viewport cannot show one.
    pub fn blocking_error(&self) -> Option<&BackendError> {
        match &self.phase {
            Phase::Unavailable(error) => Some(error),
            _ => None,
        }
    }

    pub fn failure(&self) -> Option<&BackendError> {
        self.failure.as_ref()
    }

    pub fn report(&self) -> Value {
        self.report_with_frames(8)
    }

    /// `report` with the last `frames` traced frames. The automation tree is polled and keeps the
    /// short one; `render.status` is asked for once by a proof and reads the long one.
    pub fn report_with_frames(&self, frames: usize) -> Value {
        let phase = match &self.phase {
            Phase::Unavailable(_) => "unavailable",
            Phase::Booting(_) => "booting",
            Phase::Handshaking(_) => "handshaking",
            Phase::Ready => "ready",
        };
        let capabilities = self.session.known_capabilities().map(|caps| caps.to_json());
        let error = self
            .blocking_error()
            .or(self.failure.as_ref())
            .map(BackendError::to_json);
        let mut summary = self.session.summary();
        summary["error"] = error.unwrap_or(Value::Null);
        if self.session.trace().enabled() {
            summary["trace"] = self.session.trace().snapshot(frames);
        }
        json!({
            "directed_scene": self.directed_scene(),
            "directed": self.directed_report(),
            "phase": phase,
            "headline": self.headline(),
            "on_screen": self.on_screen,
            "page": self.dist.as_ref().map(|path| path.to_string_lossy().into_owned()),
            "output": self.output_size.map(|(width, height)| json!({ "width": width, "height": height })),
            "capabilities": capabilities,
            "session": summary,
            "ui": json!({ "repaints": self.ui_repaints, "skipped": self.ui_skipped }),
            "last_frame": self.last_frame.as_ref().map(FrameResult::to_json),
        })
    }
}

/// Serves `tessel://localhost/...` for the WebView.
pub struct ProtocolHandle {
    dist: Option<PathBuf>,
    access: Rc<RefCell<AssetAccess>>,
    trace: TraceHandle,
}

impl ProtocolHandle {
    pub fn respond(&self, request: &Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
        let started = self.trace.start();
        let response = build_response(self.dist.as_deref(), &self.access.borrow(), request);
        self.trace.span("serve", started);
        response
    }
}

fn build_response(
    dist: Option<&Path>,
    access: &AssetAccess,
    request: &Request<Vec<u8>>,
) -> Response<Cow<'static, [u8]>> {
    let served = match dist {
        Some(dist) => serve(
            dist,
            access,
            request.uri().path(),
            request.uri().query().unwrap_or_default(),
            request
                .headers()
                .get("range")
                .and_then(|value| value.to_str().ok()),
        ),
        None => Served {
            status: 404,
            content_type: "text/plain; charset=utf-8",
            body: b"the runtime bundle is not built".to_vec(),
            content_range: None,
        },
    };
    let mut response = Response::builder()
        .status(served.status)
        .header("Content-Type", served.content_type)
        .header("Accept-Ranges", "bytes")
        .header("Cache-Control", "no-store");
    if let Some(range) = &served.content_range {
        response = response.header("Content-Range", range);
    }
    response
        .body(Cow::Owned(served.body))
        .unwrap_or_else(|_| Response::new(Cow::Borrowed(&[][..])))
}
