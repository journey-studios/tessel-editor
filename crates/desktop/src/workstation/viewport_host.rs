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
#[path = "viewport_overlay.rs"]
mod overlay_host;
use overlay_host::OverlayHost;

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use async_channel::{Receiver, Sender};
use serde_json::{Value, json};
use tessel_editor_core::CommandError;
use tessel_editor_core::backend::{BackendError, FrameDetail, FrameResult, Output};
use tessel_editor_core::bridge::{BridgeBackend, HelloOptions, PreviewSort, Transport};
use tessel_editor_core::headless::{backend_error, frame_geometry, frame_report, plan_summary};
use tessel_editor_core::ops::RenderFrame;
use tessel_editor_core::perf::{FrameContext, Meter};
use tessel_editor_core::perf_history::{Context, EventKind, Image, SessionKind};
use tessel_editor_core::plan::{PlanRequest, RenderPlan};
use tessel_editor_core::preview::ProxyCoverage;
use tessel_editor_core::session::{Preview, SessionEvent, Status, ViewportSession};
use tessel_editor_core::state::EditorState;
use tessel_editor_core::trace::TraceHandle;
use tessel_editor_render::SetupError;
use tessel_editor_render::asset_io::{
    AssetPool, DEFAULT_QUEUE, DEFAULT_WORKERS, Outcome, ReadJob, Rejection,
};
use tessel_editor_render::host::{
    AssetAccess, Resolved, Served, query_param, read_file, resolve, write_png,
};
use wry::http::{Request, Response};

use super::perf_meter::PerfClock;

/// Scheme and host the page is served from. A custom scheme keeps the page same-origin with
/// its assets and unreachable from any web page.
#[cfg(not(windows))]
pub const PAGE_URL: &str = "tessel://localhost/index.html";
/// WebView2 has no custom schemes: `wry` serves the protocol from the reserved
/// `http://<scheme>.localhost/` host. The host handler only looks at the path.
#[cfg(windows)]
pub const PAGE_URL: &str = "http://tessel.localhost/index.html";
/// What every URL the viewport page may navigate to starts with.
#[cfg(not(windows))]
pub const ORIGIN: &str = "tessel://";
#[cfg(windows)]
pub const ORIGIN: &str = "http://tessel.localhost/";
pub const SCHEME: &str = "tessel";

/// The page has this long to load `runtime.js` and announce itself.
const BOOT_TIMEOUT: Duration = Duration::from_secs(15);
/// WebGL context creation and the `ready` answer.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
/// Loading assets and drawing one frame.
const FRAME_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the monitor waits before it asks again for a frame that failed with a `timeout` (a slow
/// machine, a media file that was still opening), one entry per retry. After the last one the failure
/// stays on the header until the next change, as for any other failure (BL158).
const RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(3)];
/// Longest side of the picture the monitor renders itself; the CSS fit scales it to the pane.
const PREVIEW_LONG_SIDE: u32 = 960;
/// Loaded assets kept between frames before the ones the plan no longer needs are released.
const RETAINED_ASSETS: usize = 16;
/// Source-file bytes the monitor keeps loaded: three 66 MB splat scenes (the one on screen, the one
/// fading out or in, and one more to scrub back to). Past it the least recently used heavy asset is
/// released, because each scene costs ~100-250 MB in the WebView once decoded.
const RETAINED_ASSET_BYTES: u64 = 256 << 20;
/// Source-file MiB the monitor keeps loaded before it releases the least recently used heavy asset
/// (default 256; see [`RETAINED_ASSET_BYTES`]). A smaller value makes a project of a few scenes
/// exercise the release in a real window; `0` or anything unreadable keeps the default.
pub const RETAIN_MIB_ENV: &str = "TESSEL_VIEWPORT_RETAIN_MIB";
/// Set to `1` to make the monitor's own frames carry hash/mean like explicit ones (measurement
/// and pixel proofs); off by default because the readback is not needed to show a picture.
pub const INSPECT_ENV: &str = "TESSEL_VIEWPORT_INSPECT";
/// Set to `1` to time the host side of every preview frame (plan, serialize, dispatch, transit,
/// accept, script evaluation, asset serving) and to ask the runtime for `frame.presented`.
/// Reported under `trace` in `render.status` and in the `monitor.viewport` node. Off by default.
pub const TRACE_ENV: &str = "TESSEL_VIEWPORT_TRACE";

/// `1` (with the trace on) makes the page wait for the GPU after each draw and report the wait: a DIAGNOSTIC arm of the BL157.2
/// experiments, never a default (it moves a wait the browser would have made at presentation into the frame).
pub const GPU_FINISH_ENV: &str = "TESSEL_VIEWPORT_PROBE_GPU_FINISH";
/// Set to `0` to run the runtime's per-frame create/dispose path instead of GPU retention, to
/// compare the two in the same build or to roll back without a new release.
pub const RETAIN_ENV: &str = "TESSEL_VIEWPORT_RETAIN";
/// Set to `0` to make every splat scene frame sort and draw again (rollback of the result cache).
pub const RESULT_CACHE_ENV: &str = "TESSEL_VIEWPORT_RESULT_CACHE";
/// `TESSEL_VIEWPORT_WARM=0` stops the monitor from sending `frame.warm` (BL157), whatever the runtime announces: the
/// same build with and without the warm is how its effect is measured. Anything else leaves it on.
pub const WARM_ENV: &str = "TESSEL_VIEWPORT_WARM";
/// `TESSEL_VIEWPORT_PREUPLOAD=1` (BL157.3, a DIAGNOSTIC experiment, off by default): the runtime uploads the textures of the still
/// images of the cuts a moment ahead before the frame that draws them, between frames and inside its texture budget. The same build
/// with and without it is how the effect is measured; it is not a product setting and not reachable from the interface. The report
/// (`render.status` -> `session.preupload`) says whether it is on, whether the runtime announced it and how many cuts were sent.
pub const PREUPLOAD_ENV: &str = "TESSEL_VIEWPORT_PREUPLOAD";
/// `0`, `2` or `4` MSAA samples for scene targets. Unset keeps the runtime's 4 (OPT07 measured
/// fewer samples as a real gain on large splat scenes but a visible edge-quality trade).
pub const SCENE_SAMPLES_ENV: &str = "TESSEL_VIEWPORT_SCENE_SAMPLES";
/// `sync` (the default) or `async`: whether the protocol handler of the WebView reads the project's files on the thread that
/// received the request or hands the read to the bounded worker pool of `tessel_editor_render::asset_io` (BL157.1). A
/// diagnostic of one experiment, read once when the host is created: the same build answers both ways, so the two can be
/// measured against each other. Not a product setting and not reachable from the interface. A value that is neither
/// name is a typo: it is warned about on stderr and the default is used (the report names the mode in effect).
pub const ASSET_IO_ENV: &str = "TESSEL_VIEWPORT_ASSET_IO";
/// How far ahead of the playhead playback loads the assets of clips that are about to start.
pub const PREFETCH_SECONDS: i64 = 2;
/// `0`, `2` or `4` MSAA samples of scene layers in the monitor's interactive frames (playback and
/// scrub, BL129); the picture is refined to the final count when the playhead stops. Unset keeps the
/// runtime's default; `4` turns the reduction off (A/B and rollback). Explicit frames and export
/// never read it: they draw at the final count.
pub const PREVIEW_SCENE_SAMPLES_ENV: &str = "TESSEL_VIEWPORT_PREVIEW_SCENE_SAMPLES";
/// `latest` or `exact`: how a splat scene of the monitor's interactive frames (playback and scrub,
/// BL130) gets its draw order. `latest` paints with the order of the last finished sort and sorts for
/// the camera behind the frame; the picture is replaced by the exact one when the playhead stops.
/// `exact` waits for the sort of every frame (A/B and rollback). Unset (or empty) keeps the runtime's
/// default (`latest`); any other value is a typo and is read as `exact`, the safe order, with a warning
/// on stderr: a mistyped rollback must never leave the reused order on. This only orders splats: the
/// MSAA of a preview frame (`TESSEL_VIEWPORT_PREVIEW_SCENE_SAMPLES`) and the proxies of a video stay as
/// they are, and the monitor policy `monitor.preview.quality = original` is what draws an interactive
/// frame at the original quality. Explicit frames and export never read it: they wait for their own sort.
pub const PREVIEW_SORT_ENV: &str = "TESSEL_VIEWPORT_PREVIEW_SORT";

/// Who reads the files the page asks for (see [`ASSET_IO_ENV`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AssetIoMode {
    /// On the thread that received the request (the behaviour before BL157.1).
    #[default]
    Sync,
    /// On a bounded pool of workers; the callback returns without waiting for the read.
    Async,
}

impl AssetIoMode {
    pub fn as_str(self) -> &'static str {
        match self {
            AssetIoMode::Sync => "sync",
            AssetIoMode::Async => "async",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "sync" => Some(AssetIoMode::Sync),
            "async" => Some(AssetIoMode::Async),
            _ => None,
        }
    }
}

/// The mode an environment value asks for: `Ok(None)` when unset or empty, `Err(warning)` for a value that is neither
/// name (the default is used).
pub(super) fn asset_io_from_env(value: Option<&str>) -> Result<Option<AssetIoMode>, String> {
    match value.map(str::trim) {
        None | Some("") => Ok(None),
        Some(text) => AssetIoMode::parse(text).map(Some).ok_or_else(|| {
            format!("{ASSET_IO_ENV}={text:?} is not `sync` or `async`: using `sync`")
        }),
    }
}

/// Opt-ins read once when the host is created.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HostOptions {
    pub trace: bool,
    /// See [`GPU_FINISH_ENV`]; only has an effect with `trace`.
    pub gpu_finish: bool,
    /// `Some(false)` disables the runtime's GPU retention; `None` keeps its default.
    pub retain: Option<bool>,
    pub result_cache: Option<bool>,
    pub scene_samples: Option<u32>,
    pub preview_scene_samples: Option<u32>,
    pub preview_sort: Option<PreviewSort>,
    /// Bytes of heavy assets the session keeps loaded; `None` is [`RETAINED_ASSET_BYTES`].
    pub retained_bytes: Option<u64>,
    /// `Some(false)` never sends `frame.warm`; `None` keeps the default (on, for a runtime that announces it).
    pub warm: Option<bool>,
    /// See [`PREUPLOAD_ENV`].
    pub preupload: bool,
    /// Who reads the page's files (BL157.1); `None` is [`AssetIoMode::Sync`].
    pub asset_io: Option<AssetIoMode>,
}

/// A MSAA sample count the runtime accepts, from an environment value.
fn samples_from_env(name: &str) -> Option<u32> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|samples| [0, 2, 4].contains(samples))
}

/// The preview sort an environment value asks for: `Ok(None)` when it is unset or empty, and `Err(warning)`
/// with the order that is used instead (`exact`) when it is neither name.
pub(super) fn preview_sort_from_env(
    value: Option<&str>,
) -> Result<Option<PreviewSort>, (PreviewSort, String)> {
    match value.map(str::trim) {
        None | Some("") => Ok(None),
        Some(text) => match PreviewSort::parse(text) {
            Some(sort) => Ok(Some(sort)),
            None => Err((
                PreviewSort::Exact,
                format!(
                    "{PREVIEW_SORT_ENV}={text:?} is not `latest` or `exact`: using `exact` (every interactive frame waits for its own sort)"
                ),
            )),
        },
    }
}

/// The retention budget an environment value (MiB) asks for; `None` for unset, zero or unreadable.
pub(super) fn retained_bytes_from_env(value: Option<&str>) -> Option<u64> {
    value
        .and_then(|text| text.trim().parse::<u64>().ok())
        .filter(|mib| *mib > 0)
        .and_then(|mib| mib.checked_mul(1 << 20))
}

impl HostOptions {
    pub fn from_env() -> Self {
        Self {
            trace: std::env::var(TRACE_ENV).is_ok_and(|value| value == "1"),
            gpu_finish: std::env::var(GPU_FINISH_ENV).is_ok_and(|value| value == "1"),
            retain: std::env::var(RETAIN_ENV)
                .is_ok_and(|value| value == "0")
                .then_some(false),
            result_cache: std::env::var(RESULT_CACHE_ENV)
                .is_ok_and(|value| value == "0")
                .then_some(false),
            warm: std::env::var(WARM_ENV)
                .is_ok_and(|value| value == "0")
                .then_some(false),
            preupload: std::env::var(PREUPLOAD_ENV).is_ok_and(|value| value == "1"),
            asset_io: match asset_io_from_env(std::env::var(ASSET_IO_ENV).ok().as_deref()) {
                Ok(mode) => mode,
                Err(warning) => {
                    eprintln!("tessel-editor: {warning}");
                    None
                }
            },
            scene_samples: samples_from_env(SCENE_SAMPLES_ENV),
            retained_bytes: retained_bytes_from_env(std::env::var(RETAIN_MIB_ENV).ok().as_deref()),
            preview_scene_samples: samples_from_env(PREVIEW_SCENE_SAMPLES_ENV),
            preview_sort: match preview_sort_from_env(
                std::env::var_os(PREVIEW_SORT_ENV)
                    .as_deref()
                    .map(std::ffi::OsStr::to_string_lossy)
                    .as_deref(),
            ) {
                Ok(sort) => sort,
                Err((sort, warning)) => {
                    eprintln!("tessel-editor: {warning}");
                    Some(sort)
                }
            },
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
    trace: TraceHandle,
}

/// A short, stable, non-reversible label for something private (an asset's path or name) so a trace can correlate the
/// protocol request, the `asset.load` and the runtime's own spans without writing the name down. FNV-1a, 32 bits shown.
pub fn trace_label(prefix: char, text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{prefix}{:08x}", hash as u32)
}

/// What the monitor's header adds when the picture is not the final quality: a proxy copy of a
/// video, a scene drawn with fewer MSAA samples, and/or splats painted in the order of an earlier
/// camera (the refine replaces all of them once the playhead stops). Empty for a final-quality picture.
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
    if detail.stale_sort_layers() > 0 {
        label.push_str(" · sort");
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
            self.trace.event("asset_load_sent", || {
                let source = &message["asset"]["source"];
                let location = source["path"]
                    .as_str()
                    .or_else(|| source["pattern"].as_str())
                    .unwrap_or_default();
                json!({
                    "asset": trace_label('k', message["asset"]["id"].as_str().unwrap_or_default()),
                    "kind": message["asset"]["kind"],
                    "path": trace_label('a', &format!("p={}", location)),
                })
            });
        }
        if message["type"] == "frame.warm" {
            self.trace.event(
                "warm_sent",
                || json!({ "layers": message["plan"]["layers"].as_array().map_or(0, Vec::len) }),
            );
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

/// Where presented pictures are reported for the debug meter (F049), and the clock it uses.
struct MeterLink {
    meter: Rc<RefCell<Meter>>,
    clock: PerfClock,
}

struct Waiter {
    request: u64,
    reply: Sender<Result<Value, CommandError>>,
    summary: Value,
}

pub struct ViewportHost {
    directed: directed::DirectedViewport,
    /// F055: the gizmo overlay the page draws over the picture.
    overlay: OverlayHost,
    dist: Option<PathBuf>,
    session: ViewportSession<BridgeBackend<WebTransport>>,
    outbox: Rc<RefCell<VecDeque<String>>>,
    inbox: Rc<RefCell<VecDeque<Value>>>,
    access: Rc<RefCell<AssetAccess>>,
    /// Who reads the page's files, and the pool that does when it is not the receiving thread (BL157.1).
    asset_io: AssetIo,
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
    /// Retries already made for the failure on show, and when the next one is due (BL158).
    retries: usize,
    retry_due: Option<Instant>,
    last_frame: Option<FrameResult>,
    output_size: Option<(u32, u32)>,
    /// Proxy coverage of the sequence's videos as of the last sync: while some are pending the
    /// monitor draws them from full-size frames, which is much slower, and says so.
    proxy_coverage: ProxyCoverage,
    /// Whether the monitor is on screen; reported, and used to skip work while hidden.
    pub on_screen: bool,
    /// Messages from the page that changed what the monitor shows (and so repainted the window)
    /// and those that did not (`frame.presented`, asset and proxy acknowledgements, a frame that
    /// took the same rounded milliseconds). OPT04: the window only repaints for the former.
    ui_repaints: u64,
    ui_skipped: u64,
    /// The debug meter, when the workstation attached one. It records only while it is enabled.
    meter: Option<MeterLink>,
    /// Sequence frame of the newest accepted picture.
    presented: Option<i64>,
    /// The rate the monitor aims at: the sequence's for a sequence preview, the directed
    /// scene's for a scene. `None` for a source preview or before the first request, so the
    /// history never assumes one (and never 60: this is not the interface's target).
    target_fps: Option<f64>,
    /// How many times the session under the history's measurements changed (a scene, the project,
    /// the page). A request is stamped with it when the host admits it, and its answer is recorded
    /// only while the stamp is still the current one: an answer that comes back after the edge
    /// belongs to the segment it was asked in, which is over (BL145.6). Counted whether or not a
    /// recording runs, so a recording that starts later never mistakes an old request for its own.
    history_epoch: u64,
}

/// The protocol handler's way of reading, fixed when the host is created.
struct AssetIo {
    mode: AssetIoMode,
    /// Present only for [`AssetIoMode::Async`]; nothing is spawned for the default.
    pool: Option<AssetPool>,
    /// Requests the handler received, whichever way it answered them.
    requests: Arc<AtomicU64>,
}

impl AssetIo {
    fn new(mode: AssetIoMode) -> Self {
        Self {
            mode,
            pool: (mode == AssetIoMode::Async)
                .then(|| AssetPool::new(DEFAULT_WORKERS, DEFAULT_QUEUE)),
            requests: Arc::default(),
        }
    }

    /// A project or page change: reads of the old session answer `410` instead of being applied.
    fn invalidate(&self) {
        if let Some(pool) = &self.pool {
            pool.invalidate();
        }
    }

    fn report(&self) -> Value {
        json!({
            "mode": self.mode.as_str(),
            "workers": self.pool.as_ref().map(AssetPool::workers),
            "queue": self.pool.as_ref().map(AssetPool::queue_capacity),
            "requests": self.requests.load(Ordering::Relaxed),
            "pool": self.pool.as_ref().map(|pool| pool.stats().to_json()),
        })
    }
}

impl ViewportHost {
    pub fn new(page: Result<PathBuf, SetupError>) -> Self {
        Self::with_options(page, HostOptions::from_env())
    }

    pub fn with_options(page: Result<PathBuf, SetupError>, options: HostOptions) -> Self {
        let outbox = Rc::new(RefCell::new(VecDeque::new()));
        let inbox = Rc::new(RefCell::new(VecDeque::new()));
        let access = Rc::new(RefCell::new(AssetAccess::default()));
        // One recorder for the session, the transport and the protocol handler: their events share a log and a clock.
        let trace = TraceHandle::default();
        let transport = WebTransport {
            outbox: outbox.clone(),
            inbox: inbox.clone(),
            access: access.clone(),
            trace: trace.clone(),
        };
        let backend = BridgeBackend::new("three-webview", transport, HANDSHAKE_TIMEOUT)
            .with_options(HelloOptions {
                retain: options.retain,
                presented: options.trace,
                timing: options.trace,
                gpu_finish: options.trace && options.gpu_finish,
                result_cache: options.result_cache,
                scene_samples: options.scene_samples,
                preview_scene_samples: options.preview_scene_samples,
                preview_sort: options.preview_sort,
                preupload_textures: options.preupload,
            });
        let mut session = ViewportSession::new(backend)
            .with_trace(trace)
            .with_demo_dir(tessel_editor_render::demo_dir());
        session.set_retained_assets(RETAINED_ASSETS);
        session.set_retained_bytes(options.retained_bytes.unwrap_or(RETAINED_ASSET_BYTES));
        session.set_warm(options.warm != Some(false));
        session.set_preupload(options.preupload);
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
            asset_io: AssetIo::new(options.asset_io.unwrap_or_default()),
            phase,
            last_key: None,
            dirty: true,
            png_paths: HashMap::new(),
            waiters: Vec::new(),
            request_started: None,
            failure: None,
            retries: 0,
            retry_due: None,
            last_frame: None,
            output_size: None,
            proxy_coverage: ProxyCoverage::default(),
            on_screen: true,
            directed: directed::DirectedViewport::default(),
            overlay: OverlayHost::from_env(),
            ui_repaints: 0,
            ui_skipped: 0,
            meter: None,
            presented: None,
            target_fps: None,
            history_epoch: 0,
        };
        host.set_inspect_previews(std::env::var(INSPECT_ENV).is_ok_and(|value| value == "1"));
        host
    }

    /// Lets the debug meter hear every picture the monitor accepts, on the meter's clock.
    pub(super) fn attach_meter(&mut self, meter: Rc<RefCell<Meter>>, clock: PerfClock) {
        self.meter = Some(MeterLink { meter, clock });
    }

    /// Tests only: one accepted picture with its sequence frame and no request. A hidden meter
    /// returns at once inside `monitor_presented`: nothing is kept. The frame is remembered
    /// whatever the meter does (one `Option<i64>`, no sample): the meter needs to know what the
    /// monitor shows at the instant it is switched on or playback starts. Real pictures arrive
    /// through [`Self::note_image`].
    #[cfg(test)]
    pub(super) fn note_presented(&mut self, frame: Option<i64>) {
        self.presented = frame;
        if let Some(link) = &self.meter {
            link.meter
                .borrow_mut()
                .monitor_presented(link.clock.elapsed(), frame);
        }
    }

    /// One accepted picture of the three.js runtime, with what the host knows about it: its
    /// request (latency), `render_ms` (CPU) and the texture estimate go to the history.
    pub(super) fn note_image(&mut self, frame: Option<i64>, result: &FrameResult) {
        self.presented = frame;
        if let Some(link) = &self.meter
            && link.meter.borrow().enabled()
        {
            let context = FrameContext::of(result);
            let image = Image {
                request: Some(result.id),
                frame,
                render_ms: Some(result.render_ms),
                texture_bytes: context.texture_bytes,
            };
            let at = link.clock.elapsed();
            let mut meter = link.meter.borrow_mut();
            meter.monitor_image(at, frame, &image);
            meter.picture_context(
                at,
                Context {
                    width: context.width,
                    height: context.height,
                    quality: context.quality,
                    proxy_layers: context.proxy_layers,
                    stale_sort_layers: context.stale_sort_layers,
                    reduced_scene_layers: context.reduced_scene_layers,
                    monitor_on_screen: Some(self.on_screen),
                    sequence_fps: self.target_fps,
                    ..Context::default()
                },
            );
        }
    }

    /// The session the history is measuring now; a request admitted under another one is over.
    pub(super) fn history_epoch(&self) -> u64 {
        self.history_epoch
    }

    /// [`Self::note_image`] for a picture whose request was admitted under `epoch`. A picture of
    /// a session that has since changed (the user moved to another scene while its frame was in
    /// flight) is still what the canvas shows, so the monitor remembers it, but it is **not** a
    /// frame, an interval, a latency, a texture reading or a frame format of the segment that is
    /// open now: the history counts it as a stale answer and nothing else.
    pub(super) fn note_image_of(&mut self, epoch: u64, frame: Option<i64>, result: &FrameResult) {
        if epoch == self.history_epoch {
            self.note_image(frame, result);
            return;
        }
        self.presented = frame;
        if let Some(link) = &self.meter {
            link.meter
                .borrow_mut()
                .monitor_stale_image(link.clock.elapsed(), frame, result.id);
        }
    }

    /// The answer to request `id` came back after its session ended: counted, never measured.
    pub(super) fn note_stale(&self, id: u64) {
        if let Some(link) = &self.meter {
            link.meter.borrow_mut().response_stale(id);
        }
    }

    /// The host admitted request `id` (history latency starts here).
    pub(super) fn note_admitted(&self, id: u64) {
        if let Some(link) = &self.meter {
            link.meter
                .borrow_mut()
                .request_admitted(id, link.clock.elapsed());
        }
    }

    /// Request `id` (when there is one) ended in failure `code`: the history stops waiting for
    /// its picture and records the failure (a `context_lost` becomes the context event, with the
    /// request it interrupted). Nothing is built unless the history records.
    pub(super) fn note_failed(&self, id: Option<u64>, code: &str) {
        if let Some(link) = &self.meter {
            link.meter
                .borrow_mut()
                .request_failed(link.clock.elapsed(), id, code);
        }
    }

    pub(super) fn note_session(&mut self, kind: SessionKind) {
        self.history_epoch = self.history_epoch.wrapping_add(1);
        if let Some(link) = &self.meter {
            link.meter
                .borrow_mut()
                .session_changed(link.clock.elapsed(), kind);
        }
    }

    /// A fact the history's event line should show (a resource load, a context restore). Nothing
    /// is built unless the history records: the recorder refuses it first.
    pub(super) fn note_event(&self, kind: EventKind, detail: Option<&str>) {
        if let Some(link) = &self.meter {
            link.meter
                .borrow_mut()
                .history_event(link.clock.elapsed(), kind, detail);
        }
    }

    /// The monitor stops showing a directed scene for the sequence (an ordinary preview was
    /// chosen): that is a scene change, one segment, and then the scene's state is dropped.
    pub fn leave_directed(&mut self) {
        if self.directed_scene().is_some() {
            self.note_session(SessionKind::SceneChanged);
            self.target_fps = None;
        }
        self.clear_directed();
    }

    /// The sequence frame of the newest picture the monitor accepted (`None`: a source preview,
    /// nothing yet, or the project was replaced or the viewport reloaded since).
    pub fn presented_frame(&self) -> Option<i64> {
        self.presented
    }

    /// The last picture the runtime finished, for the meter's context rows.
    pub fn last_frame(&self) -> Option<&FrameResult> {
        self.last_frame.as_ref()
    }

    /// Whether the runtime can produce pictures at all (the meter says "indisponível" otherwise).
    pub fn monitor_available(&self) -> bool {
        self.phase == Phase::Ready
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
        ProtocolHandle::new(
            self.dist.clone(),
            self.access.clone(),
            self.trace(),
            self.asset_io.mode,
            self.asset_io.pool.clone(),
            self.asset_io.requests.clone(),
        )
    }

    /// Lets the page read `file`, as an `asset.load` of it would (tests that drive the protocol handle of a host).
    #[cfg(test)]
    pub(super) fn allow_file(&self, file: &std::path::Path) {
        self.access
            .borrow_mut()
            .register(&json!({"source": {"type": "file", "path": file}}));
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
        self.overlay_forget();
        self.session.reset_project();
        self.access.borrow_mut().clear();
        self.asset_io.invalidate();
        self.inbox.borrow_mut().clear();
        self.outbox.borrow_mut().clear();
        self.png_paths.clear();
        self.fail_waiters(&BackendError::new("cancelled", "The viewport is reloading"));
        self.phase = Phase::Booting(Instant::now());
        self.last_key = None;
        self.dirty = true;
        self.last_frame = None;
        // The reloaded page shows nothing until its first new picture.
        self.presented = None;
        self.target_fps = None;
        self.note_session(SessionKind::ViewportReloaded);
        true
    }

    /// The project was replaced: IDs may now mean something else.
    pub fn reset_project(&mut self) {
        self.clear_directed();
        self.fail_waiters(&BackendError::new("cancelled", "The project was replaced"));
        self.session.reset_project();
        self.access.borrow_mut().clear();
        self.asset_io.invalidate();
        self.last_key = None;
        self.last_frame = None;
        // Frame IDs may now mean something else: what the monitor showed is not known anymore.
        self.presented = None;
        self.target_fps = None;
        self.note_session(SessionKind::ProjectReplaced);
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
        self.overlay_forget();
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

    /// A change the user made asks for the next picture: the retries of an earlier failure start over. The window
    /// calls it for every change (and for none of the retries the monitor schedules itself, see
    /// [`retry_due`](Self::retry_due)), whichever way the picture is then asked for (an ordinary preview or a
    /// directed scene), so the count lives in one place and a path that never reaches `sync` cannot leave it stale.
    pub(super) fn note_change(&mut self) {
        self.retries = 0;
    }

    /// The failure on show goes away (a new request was admitted, or a picture arrived): so does the retry
    /// that was due for it, which belonged to that failure and to no other.
    fn clear_failure(&mut self) {
        self.failure = None;
        self.retry_due = None;
    }

    /// A picture was delivered: the next failure starts with all its retries.
    fn delivered(&mut self) {
        self.clear_failure();
        self.retries = 0;
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
        self.proxy_coverage = state.preview_policy.coverage(state);
        let planned = trace.start();
        let plan = state.plan(&PlanRequest::of(state));
        let plan_ms = planned.map(|started| started.elapsed().as_secs_f64() * 1000.0);
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
            trace.event("sync", || {
                json!({ "outcome": "unchanged", "frame": plan.frame, "plan_ms": plan_ms,
                    "total_ms": sync_started.map(|at| at.elapsed().as_secs_f64() * 1000.0) })
            });
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
        let frame = plan.frame;
        self.request_preview(state, plan, output);
        trace.span("sync", sync_started);
        trace.event("sync", || {
            json!({ "outcome": "requested", "frame": frame, "plan_ms": plan_ms,
                "total_ms": sync_started.map(|at| at.elapsed().as_secs_f64() * 1000.0) })
        });
    }

    /// While the playhead moves: starts loading what clips about to start will need, so a cut or
    /// crossfade does not stall its first frame on the fetch + decode of a large splat (see
    /// [`ViewportSession::prefetch`]). Looks [`PREFETCH_SECONDS`] ahead. Best effort, never an error;
    /// does nothing for a directed scene, a source preview or a runtime that is not ready.
    /// Returns how many loads started.
    pub fn prefetch_ahead(&mut self, state: &EditorState) -> usize {
        if self.directed_scene().is_some() || !self.is_ready() {
            return 0;
        }
        let trace = self.session.trace().clone();
        let frames = state
            .timeline
            .rate
            .nominal_fps()
            .saturating_mul(PREFETCH_SECONDS);
        let entered = trace.start();
        let planned = trace.start();
        let plans = state.plans_ahead(frames);
        let plans_ahead_ms = planned.map(|at| at.elapsed().as_secs_f64() * 1000.0);
        trace.span("plans_ahead", planned);
        let mut assets = Vec::new();
        for asset in plans.iter().flat_map(|plan| plan.requires.iter()) {
            if !assets.contains(asset) {
                assets.push(*asset);
            }
        }
        let loading = trace.start();
        let started = if assets.is_empty() {
            0
        } else {
            self.session.prefetch(state, &assets)
        };
        let prefetch_ms = loading.map(|at| at.elapsed().as_secs_f64() * 1000.0);
        // What was loaded on an earlier tick gets its splat scenes' first sort now, not at the cut (BL157). Also
        // with nothing ahead: the cuts that left the window are forgotten, so a return to one warms it again.
        let warming = trace.start();
        let warmed = self.session.warm(&plans);
        trace.event("prefetch", || {
            json!({
                "plans": plans.len(), "assets": assets.len(), "loads_started": started, "warms_started": warmed,
                "plans_ahead_ms": plans_ahead_ms, "prefetch_ms": prefetch_ms,
                "warm_ms": warming.map(|at| at.elapsed().as_secs_f64() * 1000.0),
                "total_ms": entered.map(|at| at.elapsed().as_secs_f64() * 1000.0),
            })
        });
        started + warmed
    }

    /// The monitor's own frame. A frame already rendering is left to finish when only the
    /// playhead moved (the session parks the newest intent), so a scrub keeps presenting.
    /// Anything that supersedes it answers the explicit waiter, if any, as cancelled.
    fn request_preview(&mut self, state: &EditorState, plan: RenderPlan, output: Output) {
        // Only a sequence preview has a sequence frame, and so a rate the monitor aims at.
        self.target_fps = plan.frame.is_some().then(|| plan.rate.as_f64());
        match self.session.request_preview(state, plan, output) {
            // Only an interactive frame is ever coalesced into, and explicit requests (the ones
            // with a waiter) are never interactive: nothing to answer here.
            Ok(Preview::Coalesced) => {}
            Ok(Preview::Submitted(id)) => {
                self.note_admitted(id);
                self.request_started = Some(Instant::now());
                self.clear_failure();
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
        // A frame timeout and a request the core refused never come back as a session `Failed`
        // event: this is where the history learns about them (the code is kept only when plain).
        // The same failure repeated with no request admitted in between (a plan the core
        // refuses is asked again on every state change) is one event, not one per sync: the
        // recorder folds it, because only it knows whether the first one was recorded.
        if let Some(link) = self.meter.as_ref() {
            link.meter
                .borrow_mut()
                .request_failed(link.clock.elapsed(), None, &error.code);
        }
        // The first timeout is waited out again (a load that is slow is not a load that is stuck); at the second a
        // load still pending is given up and asked for anew, so the retry cannot wait on an answer that never comes.
        if error.code == "timeout" && self.retries >= 1 {
            self.session.cancel_timed_out();
        } else {
            self.session.cancel();
        }
        self.request_started = None;
        self.last_frame = None;
        self.png_paths.clear();
        self.fail_waiters(&error);
        self.show_failure(error);
    }

    /// Keeps `error` on show and, for a `timeout`, schedules the retry (BL158): the slow open or frame
    /// that timed out usually works the second time, and nothing else would ever ask again.
    fn show_failure(&mut self, error: BackendError) {
        self.retry_due = (error.code == "timeout")
            .then(|| RETRY_DELAYS.get(self.retries))
            .flatten()
            .map(|delay| Instant::now() + *delay);
        self.failure = Some(error);
    }

    /// Whether the failure on show is due its retry. Consumes the due time and counts the retry: the caller then
    /// asks for the frame again by the path a change takes, without [`note_change`](Self::note_change).
    pub fn retry_due(&mut self, now: Instant) -> bool {
        if self.failure.is_none() || !self.retry_due.is_some_and(|due| now >= due) {
            return false;
        }
        self.retry_due = None;
        self.retries += 1;
        // The plan is the one that failed, so only `dirty` keeps `sync` from taking it for "unchanged".
        self.dirty = true;
        true
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
                self.note_admitted(id);
                self.request_started = Some(Instant::now());
                self.clear_failure();
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
        // An explicit request is the user asking again: an earlier failure's retries start over.
        self.note_change();
        self.dirty = false;
        // The next `sync` must re-evaluate against what is on the canvas now.
        self.last_key = None;
        self.output_size = Some((width, height));
        // The picture of an explicit request is aimed at this plan's rate, not at whatever the
        // previous preview left in the context.
        self.target_fps = plan.frame.is_some().then(|| plan.rate.as_f64());
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
    #[cfg(test)]
    pub fn on_ipc(&mut self, raw: &str, state: &EditorState) {
        self.on_ipc_waited(raw, state, None);
    }

    /// One message from the page that waited `waited` between the WebView handing it over and the window's task taking
    /// it (BL157.2; `None` when nobody measured, trace off).
    pub fn on_ipc_waited(&mut self, raw: &str, state: &EditorState, waited: Option<Duration>) {
        let trace = self.session.trace().clone();
        let started = trace.start();
        if let Some(waited) = waited {
            trace.record("ipc_wait", waited);
        }
        self.handle_ipc(raw, state, waited);
        trace.span("ipc", started);
    }

    fn handle_ipc(&mut self, raw: &str, state: &EditorState, waited: Option<Duration>) {
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
        self.trace_incoming(&message, waited);
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

    /// Records what the page says about its own work, with the host's arrival stamp. The page's durations are on its
    /// own clock and stay durations: nothing here subtracts a page timestamp from a host one.
    fn trace_incoming(&self, message: &Value, waited: Option<Duration>) {
        let trace = self.session.trace();
        if !trace.enabled() {
            return;
        }
        match message["type"].as_str() {
            Some("asset.state") => trace.event("asset_state", || {
                json!({
                    "asset": trace_label('k', message["id"].as_str().unwrap_or_default()),
                    "state": message["state"], "request": message["request"],
                    "timings": message["timings"],
                })
            }),
            Some("frame.done") => {
                // BL157.2: the page's own account of this frame (durations on its clock) next to how long the message
                // waited for the window's task. Only when the page was asked for it (`telemetry.timing`).
                if message["page"].is_object() {
                    trace.event("frame_page", || {
                        json!({
                            "id": message["id"],
                            "page": message["page"],
                            "runtime_total_ms": message["timings"]["total_ms"],
                            "gpu_finish_ms": message["timings"]["gpu_finish_ms"],
                            "ipc_wait_ms": waited.map(|waited| round3(waited.as_secs_f64() * 1000.0)),
                        })
                    });
                }
                if let Some(spans) = message["runtime_spans"].as_array() {
                    for span in spans {
                        trace.event("runtime_span", || {
                            let mut span = span.clone();
                            if let Some(id) = span["asset"].as_str() {
                                span["asset"] = json!(trace_label('k', id));
                            }
                            span
                        });
                    }
                }
            }
            _ => {}
        }
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
            self.note_event(EventKind::ContextRestored, None);
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
            Some(Ok(id)) => {
                self.note_admitted(id);
                self.request_started = Some(Instant::now());
                self.clear_failure();
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
            Some(Ok(id)) => {
                self.note_admitted(id);
                self.request_started = Some(now);
                self.clear_failure();
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
            SessionEvent::Progress => {}
            SessionEvent::Stale(id) => {
                if let Some(link) = &self.meter {
                    link.meter.borrow_mut().response_stale(id);
                }
            }
            SessionEvent::Frame(frame) => {
                self.request_started = None;
                self.delivered();
                self.png_paths.remove(&frame.id);
                if let Some(index) = self
                    .waiters
                    .iter()
                    .position(|waiter| waiter.request == frame.id)
                {
                    let waiter = self.waiters.remove(index);
                    let _ = waiter.reply.try_send(frame_report(&waiter.summary, &frame));
                }
                let presented = self.session.presented_frame();
                self.note_image(presented, &frame);
                self.last_frame = Some(frame);
            }
            SessionEvent::Failed(error) => {
                if let Some(link) = &self.meter {
                    link.meter
                        .borrow_mut()
                        .request_failed(link.clock.elapsed(), None, &error.code);
                }
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
                self.show_failure(error);
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
                        Some(frame) => {
                            format!(
                                "Three.js · {}×{} · {:.0} ms{}",
                                frame.width,
                                frame.height,
                                frame.render_ms,
                                // Never hide that the picture is a preview copy.
                                reduced_label(&frame.detail)
                            ) + &self.proxy_note()
                        }
                        None => "Aguardando quadro".to_string(),
                    },
                }
            }
        }
    }

    /// Why playback may be slower than the picture's cost suggests: videos of the sequence whose
    /// preview proxy is still being built are drawn from their full-size frames.
    fn proxy_note(&self) -> String {
        match self.proxy_coverage.pending {
            0 => String::new(),
            pending => format!(" · gerando proxy ({pending})"),
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
            "overlay": self.overlay_report(),
            "phase": phase,
            "headline": self.headline(),
            "on_screen": self.on_screen,
            "page": self.dist.as_ref().map(|path| path.to_string_lossy().into_owned()),
            "output": self.output_size.map(|(width, height)| json!({ "width": width, "height": height })),
            "capabilities": capabilities,
            "session": summary,
            "ui": json!({ "repaints": self.ui_repaints, "skipped": self.ui_skipped }),
            "asset_io": self.asset_io.report(),
            "proxy_coverage": self.proxy_coverage.to_json(),
            "last_frame": self.last_frame.as_ref().map(FrameResult::to_json),
        })
    }
}

/// How a request is answered: the WebView's responder, or anything that takes the response once.
pub type Respond = Box<dyn FnOnce(Response<Cow<'static, [u8]>>) + Send>;

/// Serves `tessel://localhost/...` for the WebView.
pub struct ProtocolHandle {
    dist: Option<PathBuf>,
    access: Rc<RefCell<AssetAccess>>,
    trace: TraceHandle,
    mode: AssetIoMode,
    pool: Option<AssetPool>,
    requests: Arc<AtomicU64>,
    /// The thread the handle was made on: the window's. A request received anywhere else is traced as such.
    ui_thread: std::thread::ThreadId,
}

/// The facts of one request that the trace carries from the callback to the completion.
struct RequestFacts {
    id: u64,
    mode: AssetIoMode,
    label: String,
    range: Option<String>,
    /// Host clock when the callback began.
    received_ms: f64,
}

impl ProtocolHandle {
    pub(crate) fn new(
        dist: Option<PathBuf>,
        access: Rc<RefCell<AssetAccess>>,
        trace: TraceHandle,
        mode: AssetIoMode,
        pool: Option<AssetPool>,
        requests: Arc<AtomicU64>,
    ) -> Self {
        // `Async` without a pool cannot read anywhere else: it answers on this thread instead of failing.
        let mode = if pool.is_some() {
            mode
        } else {
            AssetIoMode::Sync
        };
        Self {
            dist,
            access,
            trace,
            mode,
            pool,
            requests,
            ui_thread: std::thread::current().id(),
        }
    }

    /// Answers the request through `respond`: in `sync` mode before it returns; in `async` mode `respond` is called later
    /// by a worker, and this returns as soon as the request is authorized and queued. Every request is answered exactly
    /// once: a full queue is answered here with `503` (and counted), never dropped.
    pub fn handle(&self, request: &Request<Vec<u8>>, respond: Respond) {
        let entered = self.trace.start();
        let id = self.requests.fetch_add(1, Ordering::Relaxed) + 1;
        let uri = request.uri();
        let (path, query) = (uri.path(), uri.query().unwrap_or_default());
        let range = request
            .headers()
            .get("range")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let facts = RequestFacts {
            id,
            mode: self.mode,
            label: if self.trace.enabled() {
                if path == "/asset" {
                    // The same label the load message gets: of the decoded path, not of the encoded query.
                    let location = query_param(query, "p").unwrap_or_default();
                    trace_label('a', &format!("p={location}"))
                } else {
                    path.to_owned()
                }
            } else {
                String::new()
            },
            range: range.clone(),
            received_ms: self.trace.now_ms(),
        };
        let resolved = match &self.dist {
            Some(dist) => resolve(dist, &self.access.borrow(), path, query, range.as_deref()),
            None => Resolved::Answer(not_built()),
        };
        let mut outcome = "answered";
        match resolved {
            Resolved::Answer(served) => respond(to_response(served)),
            Resolved::File { file, range } => match (&self.pool, self.mode) {
                // Only project assets go to the pool: the page's own bundle (`/`, `/index.html`, `/runtime.js`) is read
                // here, as it always was, so replacing the project (which makes queued reads answer `410`) can never
                // refuse the runtime its own code.
                (Some(pool), AssetIoMode::Async) if path == "/asset" => {
                    outcome = self.submit(pool, &facts, ReadJob { file, range }, respond);
                }
                _ => {
                    let started = Instant::now();
                    let served = read_file(&file, range.as_deref());
                    let read = started.elapsed();
                    self.note_io(&facts, &served, read, None);
                    respond(to_response(served));
                }
            },
        }
        self.trace.span("serve", entered);
        self.trace.event("asset_cb", || {
            let thread = std::thread::current();
            json!({
                "req": facts.id, "mode": facts.mode.as_str(), "asset": facts.label,
                "range": facts.range, "outcome": outcome,
                "on_ui_thread": thread.id() == self.ui_thread,
                "thread": thread.name().unwrap_or("unnamed"),
                "recv_ms": round3(facts.received_ms),
                "callback_ms": round3(self.trace.now_ms() - facts.received_ms),
            })
        });
    }

    /// Hands the read to the pool. Returns what happened to the request, for the trace.
    fn submit(
        &self,
        pool: &AssetPool,
        facts: &RequestFacts,
        job: ReadJob,
        respond: Respond,
    ) -> &'static str {
        let trace = self.trace.clone();
        let id = facts.id;
        let label = facts.label.clone();
        let range = facts.range.clone();
        let received_ms = facts.received_ms;
        let done = Box::new(move |served: Served, outcome: Outcome| {
            let status = served.status;
            let bytes = served.body.len();
            let responding = Instant::now();
            respond(to_response(served));
            let respond_ms = responding.elapsed().as_secs_f64() * 1000.0;
            trace.event("asset_io", || {
                json!({
                    "req": id, "mode": "async", "asset": label, "range": range,
                    "status": status, "bytes": bytes,
                    "worker": outcome.worker,
                    "thread": std::thread::current().name().unwrap_or("unnamed"),
                    "on_ui_thread": false,
                    "recv_ms": round3(received_ms),
                    "queue_wait_ms": round3(outcome.queue_wait.as_secs_f64() * 1000.0),
                    "read_ms": round3(outcome.read.as_secs_f64() * 1000.0),
                    "respond_ms": round3(respond_ms),
                    "depth_at_submit": outcome.depth_at_submit,
                    "stale": outcome.stale,
                })
            });
        });
        match pool.submit(job, done) {
            Ok(()) => "queued",
            Err((rejection, done)) => {
                // Explicit and countable, never silent: the run that sees it is invalid, not fast.
                let (status, text, outcome) = match rejection {
                    Rejection::Saturated => (503, "the asset read queue is full", "saturated"),
                    Rejection::Closed => (503, "the asset read pool is closed", "closed"),
                };
                done(
                    Served {
                        status,
                        content_type: "text/plain; charset=utf-8",
                        body: text.as_bytes().to_vec(),
                        content_range: None,
                    },
                    Outcome {
                        worker: None,
                        queue_wait: Duration::ZERO,
                        read: Duration::ZERO,
                        stale: false,
                        depth_at_submit: pool.stats().queued(),
                    },
                );
                outcome
            }
        }
    }

    /// The `asset_io` event of a read that ran on the calling thread.
    fn note_io(
        &self,
        facts: &RequestFacts,
        served: &Served,
        read: Duration,
        worker: Option<usize>,
    ) {
        self.trace.event("asset_io", || {
            let thread = std::thread::current();
            json!({
                "req": facts.id, "mode": facts.mode.as_str(), "asset": facts.label,
                "range": facts.range, "status": served.status, "bytes": served.body.len(),
                "worker": worker, "thread": thread.name().unwrap_or("unnamed"),
                "on_ui_thread": thread.id() == self.ui_thread,
                "recv_ms": round3(facts.received_ms),
                "queue_wait_ms": 0.0,
                "read_ms": round3(read.as_secs_f64() * 1000.0),
                "depth_at_submit": 0, "stale": false,
            })
        });
    }
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

fn not_built() -> Served {
    Served {
        status: 404,
        content_type: "text/plain; charset=utf-8",
        body: b"the runtime bundle is not built".to_vec(),
        content_range: None,
    }
}

/// The HTTP response of what was served: the same status, type and range headers whichever thread built it.
pub fn to_response(served: Served) -> Response<Cow<'static, [u8]>> {
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
