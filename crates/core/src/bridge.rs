//! Versioned Rust ↔ JS bridge.
//!
//! One JSON object per message, always with `"v": BRIDGE_VERSION` and a `"type"`. The same
//! messages travel over a Node worker's stdio (headless) and over the WebView IPC
//! (desktop). Only transport differs: [`Transport`] moves JSON values, [`BridgeBackend`]
//! turns them into the [`RenderBackend`] contract. Rules for what to draw stay in the plan.
//!
//! Compatibility rule (see `AGENTS.md`, contracts between services): a message or field may
//! be added in a minor revision; changing the meaning of an existing one bumps `BRIDGE_VERSION`
//! and both ends refuse the mismatch with `version_mismatch` instead of guessing.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::backend::{
    AssetDescriptor, AssetStatus, BackendError, BackendEvent, Capabilities, FrameDetail,
    FrameRequest, FrameResult, ProxyState, RenderBackend, RequestId,
};
use crate::id::Id;
use crate::plan::RenderPlan;
use crate::preview::Quality;
use crate::trace::SubmitSpans;

pub const BRIDGE_VERSION: u64 = 1;

/// Optional behaviours a host asks for in `hello` (additive fields of bridge v1). A runtime that
/// predates them ignores the fields; one that has them defaults to what is asked here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HelloOptions {
    /// `Some(false)` turns the runtime's GPU retention off (the per-frame create/dispose path),
    /// for A/B measurement and as a quick rollback. `None` leaves the runtime's default (on).
    pub retain: Option<bool>,
    /// Ask for a `frame.presented` message after each `frame.done`. Off by default: a host that
    /// does not know the message rejects it (see [`decode_event`]).
    pub presented: bool,
    /// Ask the page to measure itself (BL157.2): how long each frame waited in the page's own queue, the lag of its event
    /// loop and how long it spent handing the answer over, as durations on the page clock added to `frame.done`. Off by
    /// default; the same cost on whichever arm of an experiment turns it on.
    pub timing: bool,
    /// DIAGNOSTIC arm (BL157.2), only with `timing`: the page calls `gl.finish()` after each draw and reports how long it
    /// waited as `timings.gpu_finish_ms`. It changes the cost (the wait moves from the browser's presentation into the
    /// frame), so it is never on in a product run; it exists to say whether a stall after a draw is the GPU still working.
    pub gpu_finish: bool,
    /// `Some(false)` makes every splat scene frame sort and draw again (rollback of the result
    /// cache of still scenes). `None` leaves the runtime's default (on).
    pub result_cache: Option<bool>,
    /// MSAA samples of scene targets, 0, 2 or 4 (see `docs/features/viewport.md`, OPT07): a
    /// visible quality choice, so it is only sent when the host asks. `None` is the runtime's 4.
    pub scene_samples: Option<u32>,
    /// MSAA samples of scene layers in `preview` frames (the monitor while the playhead moves,
    /// BL129): 0, 2 or 4, where 4 turns the reduction off. `None` is the runtime's default (2);
    /// it never raises the count above `scene_samples`.
    pub preview_scene_samples: Option<u32>,
    /// How a splat scene of a `preview` frame gets its draw order (BL130): `"latest"` paints with the
    /// order of the last finished sort and sorts for the camera behind the frame, `"exact"` waits for
    /// the sort of the frame's own camera. `None` is the runtime's default (`latest`).
    pub preview_sort: Option<PreviewSort>,
    /// DIAGNOSTIC (BL157.3), off by default: lets the runtime upload the textures of still images ahead of the frame that
    /// draws them when a `frame.warm` with `textures: true` asks (`FEATURE_TEXTURE_PREUPLOAD`). Sent only when on, so a
    /// default `hello` is byte for byte the one it always was.
    pub preupload_textures: bool,
    /// DIAGNOSTIC (BL157.3, a separate experiment from `preupload_textures`), off by default: after the warm sort of a splat
    /// scene the runtime draws it once into a scratch target (`FEATURE_SCENE_WARM_DRAW`). Sent only when on.
    pub warm_draw: bool,
}

/// See [`HelloOptions::preview_sort`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewSort {
    Latest,
    Exact,
}

impl PreviewSort {
    pub fn as_str(self) -> &'static str {
        match self {
            PreviewSort::Latest => "latest",
            PreviewSort::Exact => "exact",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "latest" => Some(PreviewSort::Latest),
            "exact" => Some(PreviewSort::Exact),
            _ => None,
        }
    }
}

/// Messages sent to the runtime.
pub fn hello(host: &str) -> Value {
    hello_with(host, HelloOptions::default())
}

/// `hello` with options. With the defaults the message is byte-for-byte the one it always was.
pub fn hello_with(host: &str, options: HelloOptions) -> Value {
    let mut message = json!({ "v": BRIDGE_VERSION, "type": "hello", "host": host });
    if let Some(retain) = options.retain {
        message["retain"] = json!(retain);
    }
    if options.presented || options.timing {
        let mut telemetry = serde_json::Map::new();
        if options.presented {
            telemetry.insert("presented".to_owned(), json!(true));
        }
        if options.timing {
            telemetry.insert("timing".to_owned(), json!(true));
            if options.gpu_finish {
                telemetry.insert("gpu_finish".to_owned(), json!(true));
            }
        }
        message["telemetry"] = Value::Object(telemetry);
    }
    if let Some(result_cache) = options.result_cache {
        message["result_cache"] = json!(result_cache);
    }
    if let Some(samples) = options.scene_samples {
        message["scene_samples"] = json!(samples);
    }
    if let Some(samples) = options.preview_scene_samples {
        message["preview_scene_samples"] = json!(samples);
    }
    if let Some(sort) = options.preview_sort {
        message["preview_sort"] = json!(sort.as_str());
    }
    if options.preupload_textures {
        message["preupload_textures"] = json!(true);
    }
    if options.warm_draw {
        message["warm_draw"] = json!(true);
    }
    message
}

pub fn asset_load(descriptor: &AssetDescriptor) -> Value {
    json!({ "v": BRIDGE_VERSION, "type": "asset.load", "asset": descriptor.to_json() })
}

/// `capabilities.features` entry of a runtime whose `asset.state` repeats the identity of the
/// `asset.load` it answers (additive field `request`, and `scene` for a directed-scene resource).
/// A host sends the identity only to a runtime that lists it, and trusts the echo only then.
pub const FEATURE_ASSET_IDENTITY: &str = "asset.identity";

/// Resource identity of a directed-scene load (`asset.scene`), as the host sent it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneLoadIdentity {
    pub asset_id: String,
    pub revision_id: String,
    pub content_hash: String,
    pub generation: u64,
}

/// What a host asked in one `asset.load`, kept to recognise the answer of exactly that request.
///
/// A slot name is reused (release, LRU, project switch) and the page answers loads in its own
/// order, so an old `ready` can arrive after the slot was given to a newer load. `request` is one
/// number per message sent and never reused by a host, so even the same picture loaded twice into
/// one slot cannot be confused (BL102).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadIdentity {
    pub request: u64,
    pub scene: Option<SceneLoadIdentity>,
}

impl LoadIdentity {
    /// Whether an `asset.state` message is the answer of this request: the `request` it echoes is
    /// this one, and a scene identity, if the load had one, is echoed whole and equal. A message
    /// without the echo does not answer anything: a runtime that announced the feature repeats it.
    pub fn answered_by(&self, state: &Value) -> bool {
        if state["request"].as_u64() != Some(self.request) {
            return false;
        }
        let Some(scene) = &self.scene else {
            return true;
        };
        let echoed = &state["scene"];
        echoed["asset_id"].as_str() == Some(scene.asset_id.as_str())
            && echoed["revision_id"].as_str() == Some(scene.revision_id.as_str())
            && echoed["content_hash"].as_str() == Some(scene.content_hash.as_str())
            && echoed["generation"].as_u64() == Some(scene.generation)
    }
}

/// Stamps `request` on an `asset.load` message (field `asset.request`) and returns the identity
/// the answer has to repeat. Only for a runtime that lists [`FEATURE_ASSET_IDENTITY`]: an older
/// one would ignore the field, but the host would then wait for an echo that never comes.
pub fn stamp_asset_load(message: &mut Value, request: u64) -> LoadIdentity {
    message["asset"]["request"] = json!(request);
    let scene = &message["asset"]["scene"];
    let identity = (|| {
        Some(SceneLoadIdentity {
            asset_id: scene["asset_id"].as_str()?.to_owned(),
            revision_id: scene["revision_id"].as_str()?.to_owned(),
            content_hash: scene["content_hash"].as_str()?.to_owned(),
            generation: scene["generation"].as_u64()?,
        })
    })();
    LoadIdentity {
        request,
        scene: identity,
    }
}

pub fn asset_release(id: Id) -> Value {
    json!({ "v": BRIDGE_VERSION, "type": "asset.release", "id": id })
}

/// Attaches (`Some`) or removes (`None`) the preview proxy of a loaded frame sequence.
pub fn asset_proxy(id: Id, proxy: Option<&Value>) -> Value {
    json!({ "v": BRIDGE_VERSION, "type": "asset.proxy", "id": id, "proxy": proxy })
}

pub fn frame_render(request: &FrameRequest) -> Value {
    let mut message = json!({
        "v": BRIDGE_VERSION, "type": "frame.render", "id": request.id, "revision": request.revision,
        "plan": request.plan.to_json(),
        "output": { "width": request.output.width, "height": request.output.height, "png": request.output.png },
        "probes": request.probes,
    });
    // Additive field (bridge v1 minor): absent means "inspect", which is what every runtime and
    // host that predates it does, so the message of an explicit request is unchanged.
    if !request.inspect {
        message["inspect"] = json!(false);
    }
    // Same rule for `quality`: absent means "original", so explicit requests are unchanged.
    if request.quality == Quality::Preview {
        message["quality"] = json!(request.quality.label());
    }
    message
}

/// `scene.warm` (additive): the plan of a frame that is not drawn now, whose splat scenes the runtime
/// prepares. No picture and no answer.
pub fn frame_warm(plan: &RenderPlan) -> Value {
    json!({ "v": BRIDGE_VERSION, "type": "frame.warm", "plan": plan.to_json() })
}

/// `texture.preupload` (additive, BL157.3): the same `frame.warm`, for the textures of the plan's still images only (`scenes: false`
/// keeps the runtime from preparing its splat scenes, which a host that turned the scene warm off did not ask for).
pub fn frame_warm_textures(plan: &RenderPlan) -> Value {
    json!({ "v": BRIDGE_VERSION, "type": "frame.warm", "scenes": false, "textures": true, "plan": plan.to_json() })
}

pub fn frame_cancel(id: RequestId) -> Value {
    json!({ "v": BRIDGE_VERSION, "type": "frame.cancel", "id": id })
}

pub fn resize(width: u32, height: u32) -> Value {
    json!({ "v": BRIDGE_VERSION, "type": "viewport.resize", "width": width, "height": height })
}

pub fn dispose() -> Value {
    json!({ "v": BRIDGE_VERSION, "type": "dispose" })
}

fn protocol(message: impl Into<String>) -> BackendError {
    BackendError::new("protocol_error", message)
}

/// Decodes one runtime message. A wrong version is an error event, never silently accepted.
pub fn decode_event(message: &Value) -> Result<BackendEvent, BackendError> {
    let version = message["v"].as_u64();
    if version != Some(BRIDGE_VERSION) {
        return Err(BackendError::new(
            "version_mismatch",
            format!("Runtime speaks bridge v{version:?}, host speaks v{BRIDGE_VERSION}"),
        ));
    }
    let kind = message["type"]
        .as_str()
        .ok_or_else(|| protocol("Message without `type`"))?;
    match kind {
        "ready" => Ok(BackendEvent::Ready(Box::new(Capabilities::from_json(
            message,
        )))),
        "asset.state" => {
            let id = message["id"]
                .as_str()
                .and_then(|raw| Id::new(raw).ok())
                .ok_or_else(|| protocol("asset.state without a valid `id`"))?;
            let status = match message["state"].as_str() {
                Some("loading") => AssetStatus::Loading,
                Some("ready") => AssetStatus::Ready(message["meta"].clone()),
                Some("failed") => AssetStatus::Failed(BackendError::from_json(&message["error"])),
                other => return Err(protocol(format!("Unknown asset state {other:?}"))),
            };
            // Additive (BL102b): only the identity fields, so a message without them decodes as before.
            let mut echo = serde_json::Map::new();
            for field in ["request", "scene"] {
                if !message[field].is_null() {
                    echo.insert(field.to_owned(), message[field].clone());
                }
            }
            Ok(BackendEvent::Asset {
                id,
                status,
                echo: (!echo.is_empty()).then_some(Value::Object(echo)),
            })
        }
        "asset.proxy" => {
            let id = message["id"]
                .as_str()
                .and_then(|raw| Id::new(raw).ok())
                .ok_or_else(|| protocol("asset.proxy without a valid `id`"))?;
            let state = match message["state"].as_str() {
                Some("attached") => ProxyState::Attached,
                Some("detached") => ProxyState::Detached,
                Some("failed") => ProxyState::Failed(BackendError::from_json(&message["error"])),
                other => return Err(protocol(format!("Unknown proxy state {other:?}"))),
            };
            Ok(BackendEvent::Proxy { id, state })
        }
        "frame.done" => {
            let probes = message["probes"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .map(|item| {
                            let mut pixel = [0u8; 4];
                            for (slot, value) in
                                pixel.iter_mut().zip(item.as_array().into_iter().flatten())
                            {
                                *slot = value.as_u64().unwrap_or(0) as u8;
                            }
                            pixel
                        })
                        .collect()
                })
                .unwrap_or_default();
            let mut mean = [0.0; 4];
            for (slot, value) in mean
                .iter_mut()
                .zip(message["mean"].as_array().into_iter().flatten())
            {
                *slot = value.as_f64().unwrap_or(0.0);
            }
            Ok(BackendEvent::FrameDone(FrameResult {
                id: message["id"]
                    .as_u64()
                    .ok_or_else(|| protocol("frame.done without `id`"))?,
                revision: message["revision"].as_u64().unwrap_or(0),
                width: message["width"].as_u64().unwrap_or(0) as u32,
                height: message["height"].as_u64().unwrap_or(0) as u32,
                hash: message["hash"].as_str().unwrap_or_default().to_string(),
                mean,
                probes,
                png: message["png"].as_str().map(String::from),
                render_ms: message["timings"]["render_ms"].as_f64().unwrap_or(0.0),
                total_ms: message["timings"]["total_ms"].as_f64().unwrap_or(0.0),
                detail: FrameDetail {
                    prepare_ms: message["timings"]["prepare_ms"].as_f64().unwrap_or(0.0),
                    draw_ms: message["timings"]["draw_ms"].as_f64().unwrap_or(0.0),
                    flush_ms: message["timings"]["flush_ms"].as_f64().unwrap_or(0.0),
                    cache: message
                        .get("cache")
                        .filter(|cache| !cache.is_null())
                        .cloned(),
                    source: message
                        .get("source")
                        .filter(|source| !source.is_null())
                        .cloned(),
                },
            }))
        }
        "frame.failed" => Ok(BackendEvent::FrameFailed {
            id: message["id"]
                .as_u64()
                .ok_or_else(|| protocol("frame.failed without `id`"))?,
            revision: message["revision"].as_u64().unwrap_or(0),
            error: BackendError::from_json(&message["error"]),
        }),
        "frame.presented" => Ok(BackendEvent::FramePresented {
            id: message["id"]
                .as_u64()
                .ok_or_else(|| protocol("frame.presented without `id`"))?,
            revision: message["revision"].as_u64().unwrap_or(0),
            after_done_ms: message["after_done_ms"].as_f64().unwrap_or(0.0),
        }),
        "context.lost" => Ok(BackendEvent::ContextLost),
        "context.restored" => Ok(BackendEvent::ContextRestored),
        "closed" => Ok(BackendEvent::Closed(
            message["reason"].as_str().unwrap_or("closed").to_string(),
        )),
        "error" => Err(BackendError::from_json(&message["error"])),
        other => Err(protocol(format!("Unknown message type `{other}`"))),
    }
}

/// Moves JSON messages between the core and one runtime instance.
pub trait Transport {
    fn send(&mut self, message: &Value) -> Result<(), BackendError>;
    /// Next message from the runtime, waiting at most `timeout`. `None` on timeout or close.
    fn recv(&mut self, timeout: Duration) -> Option<Value>;
    fn close(&mut self);
}

/// [`RenderBackend`] over any [`Transport`].
pub struct BridgeBackend<T: Transport> {
    name: String,
    transport: T,
    capabilities: Option<Capabilities>,
    events: VecDeque<BackendEvent>,
    handshake_timeout: Duration,
    options: HelloOptions,
    last_submit: Option<SubmitSpans>,
}

impl<T: Transport> BridgeBackend<T> {
    pub fn new(name: impl Into<String>, transport: T, handshake_timeout: Duration) -> Self {
        Self {
            name: name.into(),
            transport,
            capabilities: None,
            events: VecDeque::new(),
            handshake_timeout,
            options: HelloOptions::default(),
            last_submit: None,
        }
    }

    /// Options sent with the handshake. Must be set before it starts.
    pub fn with_options(mut self, options: HelloOptions) -> Self {
        self.options = options;
        self
    }

    /// The `hello` this backend sends, for hosts that drive the handshake themselves (a WebView
    /// answers on the thread that asks, so it cannot block in [`RenderBackend::capabilities`]).
    pub fn hello_message(&self, host: &str) -> Value {
        hello_with(host, self.options)
    }

    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    fn translate(&mut self, message: &Value) {
        match decode_event(message) {
            Ok(event) => self.events.push_back(event),
            // A protocol or version error is reported as a closed backend: continuing
            // against a runtime we cannot understand would produce wrong pixels.
            Err(error) => self
                .events
                .push_back(BackendEvent::Closed(error.to_string())),
        }
    }
}

impl<T: Transport> RenderBackend for BridgeBackend<T> {
    fn name(&self) -> &str {
        &self.name
    }

    fn capabilities(&mut self) -> Result<Capabilities, BackendError> {
        if let Some(capabilities) = &self.capabilities {
            return Ok(capabilities.clone());
        }
        self.transport.send(&hello_with("host", self.options))?;
        let deadline = Instant::now() + self.handshake_timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let Some(message) = self.transport.recv(remaining) else {
                return Err(BackendError::new(
                    "timeout",
                    format!(
                        "The runtime did not answer the handshake in {:?}",
                        self.handshake_timeout
                    ),
                ));
            };
            match decode_event(&message) {
                Ok(BackendEvent::Ready(capabilities)) => {
                    self.capabilities = Some((*capabilities).clone());
                    return Ok(*capabilities);
                }
                Ok(BackendEvent::Closed(reason)) => {
                    return Err(BackendError::new("backend_unavailable", reason));
                }
                Ok(other) => self.events.push_back(other),
                Err(error) => return Err(error),
            }
        }
    }

    fn load_asset(&mut self, descriptor: &AssetDescriptor) -> Result<(), BackendError> {
        self.transport.send(&asset_load(descriptor))
    }

    fn release_asset(&mut self, id: Id) -> Result<(), BackendError> {
        self.transport.send(&asset_release(id))
    }

    fn set_proxy(&mut self, id: Id, proxy: Option<&Value>) -> Result<(), BackendError> {
        self.transport.send(&asset_proxy(id, proxy))
    }

    fn warm_scene(&mut self, plan: &RenderPlan) -> Result<(), BackendError> {
        self.transport.send(&frame_warm(plan))
    }

    fn warm_textures(&mut self, plan: &RenderPlan) -> Result<(), BackendError> {
        self.transport.send(&frame_warm_textures(plan))
    }

    fn submit(&mut self, request: &FrameRequest) -> Result<(), BackendError> {
        let started = Instant::now();
        let message = frame_render(request);
        let serialized = Instant::now();
        let sent = self.transport.send(&message);
        self.last_submit = Some(SubmitSpans {
            serialize: serialized - started,
            dispatch: serialized.elapsed(),
        });
        sent
    }

    fn take_submit_spans(&mut self) -> Option<SubmitSpans> {
        self.last_submit.take()
    }

    fn cancel(&mut self, id: RequestId) -> Result<(), BackendError> {
        self.transport.send(&frame_cancel(id))
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<(), BackendError> {
        self.transport.send(&resize(width, height))
    }

    fn poll(&mut self, timeout: Duration) -> Option<BackendEvent> {
        if let Some(event) = self.events.pop_front() {
            return Some(event);
        }
        let message = self.transport.recv(timeout)?;
        self.translate(&message);
        self.events.pop_front()
    }

    fn shutdown(&mut self) {
        // Best effort: the process may already be gone.
        let _ = self.transport.send(&dispose());
        self.transport.close();
        self.capabilities = None;
        self.events.clear();
    }
}

/// Kinds of layers a runtime must be able to draw for the plan formats the core produces.
pub const PLAN_LAYER_KINDS: [&str; 5] = ["image", "video", "text", "scene", "waveform"];

#[cfg(test)]
mod tests;
