//! What answers the WebView's requests for the runtime page and the project's files (the `tessel://` protocol), and what
//! the trace says about each one.
//!
//! [`ProtocolHandle`] is detached from [`super::viewport_host::ViewportHost`] so the handler the WebView keeps is `'static`;
//! the host owns the [`AssetIo`] it is made from and tells it when a project or page change makes reads of the old session stale.

use std::borrow::Cow;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tessel_editor_core::trace::TraceHandle;
use tessel_editor_render::asset_io::{
    AssetPool, DEFAULT_QUEUE, DEFAULT_WORKERS, Outcome, ReadJob, Rejection,
};
use tessel_editor_render::host::{AssetAccess, Resolved, Served, query_param, read_file, resolve};
use wry::http::{Request, Response};

/// `sync` (the default) or `async`: whether the protocol handler of the WebView reads the project's files on the thread that
/// received the request or hands the read to the bounded worker pool of `tessel_editor_render::asset_io` (BL157.1). A
/// diagnostic of one experiment, read once when the host is created: the same build answers both ways, so the two can be
/// measured against each other. Not a product setting and not reachable from the interface. A value that is neither
/// name is a typo: it is warned about on stderr and the default is used (the report names the mode in effect).
pub const ASSET_IO_ENV: &str = "TESSEL_VIEWPORT_ASSET_IO";

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

/// A short, stable, non-reversible label for something private (an asset's path or name) so a trace can correlate the
/// protocol request and a file-based `asset.load` without writing the name down. A frame-sequence
/// pattern label does not identify an expanded frame request (F063/BL157.3). FNV-1a, 32 bits shown.
pub fn trace_label(prefix: char, text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{prefix}{:08x}", hash as u32)
}

/// The protocol handler's way of reading, fixed when the host is created.
pub(super) struct AssetIo {
    pub(super) mode: AssetIoMode,
    /// Present only for [`AssetIoMode::Async`]; nothing is spawned for the default.
    pub(super) pool: Option<AssetPool>,
    /// Requests the handler received, whichever way it answered them.
    pub(super) requests: Arc<AtomicU64>,
}

impl AssetIo {
    pub(super) fn new(mode: AssetIoMode) -> Self {
        Self {
            mode,
            pool: (mode == AssetIoMode::Async)
                .then(|| AssetPool::new(DEFAULT_WORKERS, DEFAULT_QUEUE)),
            requests: Arc::default(),
        }
    }

    /// A project or page change: reads of the old session answer `410` instead of being applied.
    pub(super) fn invalidate(&self) {
        if let Some(pool) = &self.pool {
            pool.invalidate();
        }
    }

    pub(super) fn report(&self) -> Value {
        json!({
            "mode": self.mode.as_str(),
            "workers": self.pool.as_ref().map(AssetPool::workers),
            "queue": self.pool.as_ref().map(AssetPool::queue_capacity),
            "requests": self.requests.load(Ordering::Relaxed),
            "pool": self.pool.as_ref().map(|pool| pool.stats().to_json()),
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
                    // A file load gets this decoded-path label too; a sequence load names a pattern, not this concrete frame.
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

pub(super) fn round3(value: f64) -> f64 {
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
