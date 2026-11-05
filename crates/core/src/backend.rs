//! Contracts for injected render backends.
//!
//! The core never imports GPUI, Wry, Node or the DOM. A backend is anything that can load
//! assets, draw a [`RenderPlan`] and answer asynchronously: the Three.js runtime behind the
//! versioned JSON bridge (see `bridge.rs`), or a fake in tests. Responses carry the request
//! ID and revision so the session can drop answers that were overtaken by a newer seek.

use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

use crate::assets::{Asset, MediaKind, Source, expand_pattern};
use crate::id::Id;
use crate::plan::RenderPlan;
use crate::preview::Quality;
use crate::state::library::MediaItem;
use crate::trace::SubmitSpans;

pub type RequestId = u64;

/// Failure reported by a backend or its transport. `code` values used by the core:
/// `backend_unavailable`, `version_mismatch`, `asset_unreadable`, `decode_failed`,
/// `unsupported_format`, `asset_load_failed`, `render_failed`, `context_lost`, `timeout`,
/// `cancelled`, `protocol_error`.
#[derive(Debug, Clone, PartialEq)]
pub struct BackendError {
    pub code: String,
    pub message: String,
    pub details: Option<Value>,
}

impl BackendError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn to_json(&self) -> Value {
        let mut value = json!({ "code": self.code, "message": self.message });
        if let Some(details) = &self.details {
            value["details"] = details.clone();
        }
        value
    }

    pub fn from_json(value: &Value) -> Self {
        Self {
            code: value["code"]
                .as_str()
                .unwrap_or("render_failed")
                .to_string(),
            message: value["message"]
                .as_str()
                .unwrap_or("The backend reported an error")
                .to_string(),
            details: value.get("details").cloned(),
        }
    }
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for BackendError {}

#[derive(Debug, Clone, PartialEq)]
pub struct Capabilities {
    pub backend: String,
    pub bridge_version: u64,
    pub runtime_version: String,
    /// GPU or software renderer string reported by WebGL, for reproducing a measurement.
    pub renderer: String,
    /// Layer kinds the runtime can draw.
    pub layers: Vec<String>,
    /// Asset formats per kind, e.g. `still: [png]`.
    pub formats: Value,
    pub raw: Value,
}

impl Capabilities {
    pub fn from_json(value: &Value) -> Self {
        let runtime = &value["runtime"];
        Self {
            backend: runtime["name"].as_str().unwrap_or("unknown").to_string(),
            bridge_version: value["v"].as_u64().unwrap_or(0),
            runtime_version: runtime["version"].as_str().unwrap_or_default().to_string(),
            renderer: runtime["renderer"].as_str().unwrap_or_default().to_string(),
            layers: value["capabilities"]["layers"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            formats: value["capabilities"]["formats"].clone(),
            raw: value.clone(),
        }
    }

    /// Whether the runtime announced an optional capability in `capabilities.features`. A
    /// runtime that predates the feature does not list it and is never sent its messages.
    pub fn supports(&self, feature: &str) -> bool {
        self.raw["capabilities"]["features"]
            .as_array()
            .is_some_and(|features| features.iter().any(|item| item.as_str() == Some(feature)))
    }

    pub fn to_json(&self) -> Value {
        json!({
            "backend": self.backend, "bridge_version": self.bridge_version,
            "runtime_version": self.runtime_version, "renderer": self.renderer,
            "layers": self.layers, "formats": self.formats,
        })
    }
}

/// Where a backend finds an asset. Built from a library item; the demo images need the
/// directory that holds the bundled files.
#[derive(Debug, Clone, PartialEq)]
pub struct AssetDescriptor {
    pub id: Id,
    pub kind: MediaKind,
    pub source: Value,
    pub meta: Value,
}

impl AssetDescriptor {
    pub fn from_item(item: &MediaItem, demo_dir: Option<&Path>) -> Result<Self, BackendError> {
        let asset: &Asset = item.asset.as_ref().ok_or_else(|| {
            BackendError::new(
                "asset_unreadable",
                format!("`{}` has no media behind it", item.id),
            )
        })?;
        let decoded_frames = asset
            .decoded
            .as_ref()
            .and_then(|decoded| decoded.frames.as_ref());
        let source = match (&asset.source, decoded_frames) {
            // A decoded container video is drawn from its frame cache, see `Asset::decoded`.
            (Source::File { .. }, Some(frames)) => json!({ "type": "frame_sequence",
                "pattern": absolute(&frames.pattern), "first": frames.first, "count": frames.count }),
            (source, _) => Self::source_json(source, demo_dir)?,
        };
        Ok(Self {
            id: item.id,
            kind: item.kind,
            source,
            meta: asset.to_json(item.kind),
        })
    }

    fn source_json(source: &Source, demo_dir: Option<&Path>) -> Result<Value, BackendError> {
        Ok(match source {
            Source::Demo(image) => {
                let dir = demo_dir.ok_or_else(|| {
                    BackendError::new(
                        "asset_unreadable",
                        "The demonstration images need the demo asset directory",
                    )
                })?;
                json!({ "type": "file", "path": dir.join(image.file_name()).to_string_lossy() })
            }
            Source::File { path } => json!({ "type": "file", "path": absolute(path) }),
            Source::FrameSequence {
                pattern,
                first,
                count,
            } => {
                json!({ "type": "frame_sequence", "pattern": absolute(pattern), "first": first, "count": count })
            }
            Source::Inline => json!({ "type": "inline" }),
        })
    }

    /// Paths of every file the runtime will read, for existence checks before loading.
    pub fn files(&self) -> Vec<String> {
        match self.source["type"].as_str() {
            Some("file") => self.source["path"]
                .as_str()
                .map(String::from)
                .into_iter()
                .collect(),
            Some("frame_sequence") => {
                let pattern = self.source["pattern"].as_str().unwrap_or_default();
                let first = self.source["first"].as_i64().unwrap_or(0);
                [
                    first,
                    first + self.source["count"].as_i64().unwrap_or(1) - 1,
                ]
                .iter()
                .filter_map(|index| expand_pattern(pattern, *index))
                .collect()
            }
            _ => Vec::new(),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({ "id": self.id, "kind": self.kind.label(), "source": self.source, "meta": self.meta })
    }
}

fn absolute(path: &str) -> String {
    std::path::absolute(path).map_or_else(
        |_| path.to_string(),
        |path| path.to_string_lossy().into_owned(),
    )
}

#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    pub width: u32,
    pub height: u32,
    /// Write a PNG here (headless host). `None` keeps pixels on screen only.
    pub png: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FrameRequest {
    pub id: RequestId,
    /// Monotonic seek counter; only the newest revision may be shown.
    pub revision: u64,
    pub plan: RenderPlan,
    pub output: Output,
    /// Pixels to read back, `(x, y)` from the top-left corner.
    pub probes: Vec<(u32, u32)>,
    /// Read the frame back and describe it (hash, mean, probes, PNG). `false` is the
    /// monitor's interactive path: the runtime only draws and waits for the GPU, and the answer
    /// carries an empty `hash` (see [`FrameResult::inspected`]). Never `false` when `probes` or
    /// `output.png` ask for pixels.
    pub inspect: bool,
    /// `Preview` lets the runtime draw video layers from their attached proxy; `Original`
    /// never does. Explicit requests and anything that asks for pixels are always `Original`.
    pub quality: Quality,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FrameResult {
    pub id: RequestId,
    pub revision: u64,
    pub width: u32,
    pub height: u32,
    /// FNV-1a 64 of the RGBA readback, hex.
    pub hash: String,
    pub mean: [f64; 4],
    pub probes: Vec<[u8; 4]>,
    pub png: Option<String>,
    pub render_ms: f64,
    pub total_ms: f64,
    /// Additive runtime telemetry (bridge v1 minor): zero/`None` from a runtime that predates it.
    pub detail: FrameDetail,
}

/// Where the runtime's CPU time of one frame went and what its GPU caches did. CPU time only:
/// nothing here says when the GPU finished (see `docs/features/viewport.md`, "What the numbers mean").
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FrameDetail {
    /// Resolving assets and building what the frame draws with.
    pub prepare_ms: f64,
    /// Recording the draw calls.
    pub draw_ms: f64,
    /// Handing the commands to the GPU process (`gl.flush`); not "the GPU finished".
    pub flush_ms: f64,
    /// Textures, text, materials, rigs and targets created/reused/evicted during this frame,
    /// and what the caches hold now, as the runtime reported it.
    pub cache: Option<Value>,
    /// What the layers were really read from: `{ quality, proxy_layers, original_layers,
    /// result_hits, proxy_error? }`. `None` from a runtime that predates it.
    pub source: Option<Value>,
}

impl FrameDetail {
    /// Layers of this frame that were drawn from a proxy (0 when unknown).
    pub fn proxy_layers(&self) -> u64 {
        self.source
            .as_ref()
            .and_then(|source| source["proxy_layers"].as_u64())
            .unwrap_or(0)
    }
}

impl FrameResult {
    /// Whether the runtime read the frame back. A frame answered without inspection has an empty
    /// `hash`, zero `mean` and no probes: that is "not measured", never "black".
    pub fn inspected(&self) -> bool {
        !self.hash.is_empty()
    }

    pub fn to_json(&self) -> Value {
        json!({
            "request": self.id, "revision": self.revision, "width": self.width, "height": self.height,
            "hash": self.hash, "mean": self.mean, "probes": self.probes, "png": self.png,
            "inspected": self.inspected(), "render_ms": self.render_ms, "total_ms": self.total_ms,
            "prepare_ms": self.detail.prepare_ms, "draw_ms": self.detail.draw_ms,
            "flush_ms": self.detail.flush_ms, "cache": self.detail.cache,
            "source": self.detail.source,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AssetStatus {
    Loading,
    Ready(Value),
    Failed(BackendError),
}

/// Answer to [`RenderBackend::set_proxy`].
#[derive(Debug, Clone, PartialEq)]
pub enum ProxyState {
    Attached,
    Detached,
    Failed(BackendError),
}

#[derive(Debug, Clone, PartialEq)]
pub enum BackendEvent {
    Ready(Box<Capabilities>),
    Asset {
        id: Id,
        status: AssetStatus,
    },
    /// The runtime answered an `asset.proxy` message.
    Proxy {
        id: Id,
        state: ProxyState,
    },
    FrameDone(FrameResult),
    FrameFailed {
        id: RequestId,
        revision: u64,
        error: BackendError,
    },
    /// The page reached a rendering opportunity after `frame.done` (only when the host asked
    /// for it in `hello`). A proxy for "on screen", not a measurement of it.
    FramePresented {
        id: RequestId,
        revision: u64,
        after_done_ms: f64,
    },
    ContextLost,
    ContextRestored,
    Closed(String),
}

/// A render backend. Methods return immediately; results arrive through [`poll`](Self::poll).
pub trait RenderBackend {
    fn name(&self) -> &str;
    /// Starts the backend if needed and returns what it can draw.
    fn capabilities(&mut self) -> Result<Capabilities, BackendError>;
    fn load_asset(&mut self, descriptor: &AssetDescriptor) -> Result<(), BackendError>;
    fn release_asset(&mut self, id: Id) -> Result<(), BackendError>;
    /// Attaches (`Some`, the `{pattern, first, count, ...}` of a [`ProxyFrames`]) or removes
    /// (`None`) the preview proxy of a loaded frame sequence. Backends without proxies ignore it.
    fn set_proxy(&mut self, _id: Id, _proxy: Option<&Value>) -> Result<(), BackendError> {
        Ok(())
    }
    fn submit(&mut self, request: &FrameRequest) -> Result<(), BackendError>;
    fn cancel(&mut self, id: RequestId) -> Result<(), BackendError>;
    fn resize(&mut self, width: u32, height: u32) -> Result<(), BackendError>;
    /// What the last [`submit`](Self::submit) spent building and handing over the request, for
    /// backends that measure it (see `trace.rs`). `None` for the rest.
    fn take_submit_spans(&mut self) -> Option<SubmitSpans> {
        None
    }
    /// Next event, waiting at most `timeout`.
    fn poll(&mut self, timeout: Duration) -> Option<BackendEvent>;
    /// Releases every resource. Idempotent.
    fn shutdown(&mut self);
}

/// Backend used when no runtime is configured: every call fails with `backend_unavailable`
/// and says how to get one. Keeps the headless CLI honest instead of silently skipping render.
#[derive(Debug, Clone)]
pub struct UnavailableBackend {
    pub reason: String,
}

impl UnavailableBackend {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    fn error(&self) -> BackendError {
        BackendError::new("backend_unavailable", self.reason.clone())
    }
}

impl RenderBackend for UnavailableBackend {
    fn name(&self) -> &str {
        "unavailable"
    }
    fn capabilities(&mut self) -> Result<Capabilities, BackendError> {
        Err(self.error())
    }
    fn load_asset(&mut self, _: &AssetDescriptor) -> Result<(), BackendError> {
        Err(self.error())
    }
    fn release_asset(&mut self, _: Id) -> Result<(), BackendError> {
        Ok(())
    }
    fn submit(&mut self, _: &FrameRequest) -> Result<(), BackendError> {
        Err(self.error())
    }
    fn cancel(&mut self, _: RequestId) -> Result<(), BackendError> {
        Ok(())
    }
    fn resize(&mut self, _: u32, _: u32) -> Result<(), BackendError> {
        Err(self.error())
    }
    fn poll(&mut self, _: Duration) -> Option<BackendEvent> {
        None
    }
    fn shutdown(&mut self) {}
}
