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
use std::time::Instant;

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::fx::fnv1a64;
use tessel_editor_core::trace::{TraceHandle, ms, round3};
use tessel_editor_render::asset_io::{
    AssetPool, DEFAULT_QUEUE, DEFAULT_WORKERS, Dispatch, Done, Outcome, Reader,
};
use tessel_editor_render::host::{AssetAccess, Resolved, Served, query_param, resolve};
use wry::http::{Request, Response};

use super::env_mode::EnvMode;

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

impl EnvMode for AssetIoMode {
    const VAR: &'static str = ASSET_IO_ENV;
    const NAMES: &'static [(&'static str, Self)] = &[("sync", Self::Sync), ("async", Self::Async)];
}

impl AssetIoMode {
    /// The reader this mode selects; the pool of `async` is created here, once, and nothing is spawned for `sync`.
    pub(super) fn reader(self) -> Reader {
        match self {
            AssetIoMode::Sync => Reader::Inline,
            AssetIoMode::Async => Reader::Pooled(AssetPool::new(DEFAULT_WORKERS, DEFAULT_QUEUE)),
        }
    }
}

/// A short, stable, non-reversible label for something private (an asset's path or name) so a trace can correlate the
/// protocol request and a file-based `asset.load` without writing the name down. A frame-sequence
/// pattern label does not identify an expanded frame request (F063/BL157.3). FNV-1a, 32 bits shown.
pub fn trace_label(prefix: char, text: &str) -> String {
    format!("{prefix}{:08x}", fnv1a64(text.as_bytes()) as u32)
}

/// The protocol handler's way of reading, fixed when the host is created.
pub(super) struct AssetIo {
    pub(super) reader: Reader,
    /// Requests the handler received, whichever way it answered them.
    pub(super) requests: Arc<AtomicU64>,
}

impl AssetIo {
    pub(super) fn new(mode: AssetIoMode) -> Self {
        Self {
            reader: mode.reader(),
            requests: Arc::default(),
        }
    }

    pub(super) fn report(&self) -> Value {
        let pool = self.reader.pool();
        json!({
            "mode": self.reader.name(),
            "workers": pool.map(AssetPool::workers),
            "queue": pool.map(AssetPool::queue_capacity),
            "requests": self.requests.load(Ordering::Relaxed),
            "pool": pool.map(|pool| pool.stats().to_json()),
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
    reader: Reader,
    requests: Arc<AtomicU64>,
    /// The thread the handle was made on: the window's. A request received anywhere else is traced as such.
    ui_thread: std::thread::ThreadId,
}

/// The facts of one request that the trace carries from the callback to the completion.
struct RequestFacts {
    id: u64,
    mode: &'static str,
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
        reader: Reader,
        requests: Arc<AtomicU64>,
    ) -> Self {
        Self {
            dist,
            access,
            trace,
            reader,
            requests,
            ui_thread: std::thread::current().id(),
        }
    }

    /// Answers the request through `respond`: with an inline reader before it returns; with a pooled one `respond` is
    /// called later by a worker, and this returns as soon as the request is authorized and queued. Every request is
    /// answered exactly once: a full queue is answered here with `503` (and counted), never dropped.
    pub fn handle(&self, request: &Request<Vec<u8>>, respond: Respond) {
        let entered = self.trace.start();
        let uri = request.uri();
        let (path, query) = (uri.path(), uri.query().unwrap_or_default());
        let range = request
            .headers()
            .get("range")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let facts = Arc::new(RequestFacts {
            id: self.requests.fetch_add(1, Ordering::Relaxed) + 1,
            mode: self.reader.name(),
            label: self.label(path, query),
            range: range.clone(),
            received_ms: self.trace.now_ms(),
        });
        let resolved = match &self.dist {
            Some(dist) => resolve(dist, &self.access.borrow(), path, query, range.as_deref()),
            None => Resolved::Answer(not_built()),
        };
        let dispatch = match resolved {
            Resolved::Answer(served) => {
                respond(to_response(served));
                Dispatch::Answered
            }
            // The page's own code is always read here, as it always was: replacing the project (which makes queued reads
            // answer `410`) can never refuse the runtime its own code.
            Resolved::Bundle(job) => Reader::Inline.read(job, self.completion(&facts, respond)),
            Resolved::Asset(job) => self.reader.read(job, self.completion(&facts, respond)),
        };
        self.trace.span("serve", entered);
        self.trace.event("asset_cb", || {
            let thread = std::thread::current();
            json!({
                "req": facts.id, "mode": facts.mode, "asset": facts.label,
                "range": facts.range, "outcome": dispatch.as_str(),
                "on_ui_thread": thread.id() == self.ui_thread,
                "thread": thread.name().unwrap_or("unnamed"),
                "recv_ms": round3(facts.received_ms),
                "callback_ms": round3(self.trace.now_ms() - facts.received_ms),
            })
        });
    }

    /// What the trace calls the request: for a file of the project the label its `asset.load` gets too (the decoded path,
    /// so the two correlate; a sequence load names a pattern, not this concrete frame), else the path asked for.
    fn label(&self, path: &str, query: &str) -> String {
        if !self.trace.enabled() {
            String::new()
        } else if path == "/asset" {
            let location = query_param(query, "p").unwrap_or_default();
            trace_label('a', &format!("p={location}"))
        } else {
            path.to_owned()
        }
    }

    /// What answers the request once its file is read, wherever that happens: the response, then the one `asset_io` event of
    /// the request. The thread it records is the one the closure runs on (a worker's, or this one for an inline read and for
    /// the refusal of a full queue).
    fn completion(&self, facts: &Arc<RequestFacts>, respond: Respond) -> Done {
        let (trace, ui_thread, facts) = (self.trace.clone(), self.ui_thread, facts.clone());
        Box::new(move |served: Served, outcome: Outcome| {
            let (status, bytes) = (served.status, served.body.len());
            let responding = Instant::now();
            respond(to_response(served));
            let respond_ms = ms(responding.elapsed());
            trace.event("asset_io", || {
                let thread = std::thread::current();
                json!({
                    "req": facts.id, "mode": facts.mode, "asset": facts.label, "range": facts.range,
                    "status": status, "bytes": bytes,
                    "worker": outcome.worker,
                    "thread": thread.name().unwrap_or("unnamed"),
                    "on_ui_thread": thread.id() == ui_thread,
                    "recv_ms": round3(facts.received_ms),
                    "queue_wait_ms": round3(ms(outcome.queue_wait)),
                    "read_ms": round3(ms(outcome.read)),
                    "respond_ms": round3(respond_ms),
                    "depth_at_submit": outcome.depth_at_submit,
                    "stale": outcome.stale,
                })
            });
        })
    }
}

fn not_built() -> Served {
    Served::text(404, "the runtime bundle is not built")
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
