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
mod tests {
    use super::*;

    #[test]
    fn decodes_frames_and_rejects_other_versions() {
        let done = json!({"v":1,"type":"frame.done","id":7,"revision":3,"width":4,"height":2,
            "hash":"abcd","mean":[1,2,3,255],"probes":[[1,2,3,4]],"png":null,
            "timings":{"render_ms":1.5,"total_ms":2.5}});
        let BackendEvent::FrameDone(frame) = decode_event(&done).unwrap() else {
            panic!("expected a frame");
        };
        assert_eq!((frame.id, frame.revision, frame.width), (7, 3, 4));
        assert_eq!(frame.probes, vec![[1, 2, 3, 4]]);
        assert_eq!(frame.render_ms, 1.5);
        let wrong = json!({"v":2,"type":"ready"});
        assert_eq!(decode_event(&wrong).unwrap_err().code, "version_mismatch");
        assert_eq!(
            decode_event(&json!({"v":1,"type":"nope"}))
                .unwrap_err()
                .code,
            "protocol_error"
        );
        let failed = json!({"v":1,"type":"frame.failed","id":1,"revision":1,
            "error":{"code":"render_failed","message":"boom"}});
        assert!(matches!(
            decode_event(&failed).unwrap(),
            BackendEvent::FrameFailed { id: 1, .. }
        ));
    }

    #[test]
    fn asset_states_cover_loading_ready_and_failed() {
        let ready = json!({"v":1,"type":"asset.state","id":"a","state":"ready","meta":{"width":2}});
        assert!(matches!(
            decode_event(&ready).unwrap(),
            BackendEvent::Asset {
                status: AssetStatus::Ready(_),
                ..
            }
        ));
        let failed = json!({"v":1,"type":"asset.state","id":"a","state":"failed",
            "error":{"code":"decode_failed","message":"bad png"}});
        let BackendEvent::Asset {
            status: AssetStatus::Failed(error),
            ..
        } = decode_event(&failed).unwrap()
        else {
            panic!("expected a failure");
        };
        assert_eq!(error.code, "decode_failed");
        assert!(
            decode_event(&json!({"v":1,"type":"asset.state","id":"bad id","state":"ready"}))
                .is_err()
        );
    }

    fn scene_load(slot: &str, asset: &str, generation: u64) -> Value {
        json!({"v":1,"type":"asset.load","asset":{"id":slot,"kind":"still",
            "source":{"type":"file","path":"/a.png"},"meta":{},
            "scene":{"asset_id":asset,"revision_id":format!("{asset}-r1"),
                     "content_hash":format!("h-{asset}"),"generation":generation}}})
    }

    fn echo(slot: &str, state: &str, request: u64, asset: &str, generation: u64) -> Value {
        json!({"v":1,"type":"asset.state","id":slot,"state":state,"request":request,
            "scene":{"asset_id":asset,"revision_id":format!("{asset}-r1"),
                     "content_hash":format!("h-{asset}"),"generation":generation}})
    }

    #[test]
    fn a_stamped_load_is_answered_only_by_the_echo_of_its_own_request() {
        let mut old = scene_load("slot", "red", 1);
        let old_identity = stamp_asset_load(&mut old, 7);
        let mut new = scene_load("slot", "green", 1);
        let new_identity = stamp_asset_load(&mut new, 8);
        assert_eq!(old["asset"]["request"], 7);
        assert_eq!(new_identity.request, 8);

        let old_ready = echo("slot", "ready", 7, "red", 1);
        assert!(old_identity.answered_by(&old_ready));
        assert!(
            !new_identity.answered_by(&old_ready),
            "the late answer of the load that held the slot never confirms the load that replaced it"
        );
        assert!(new_identity.answered_by(&echo("slot", "ready", 8, "green", 1)));
    }

    #[test]
    fn the_same_picture_loaded_again_into_a_slot_is_a_different_request() {
        let mut first = scene_load("slot", "red", 1);
        let first_identity = stamp_asset_load(&mut first, 1);
        let mut again = scene_load("slot", "red", 1);
        let again_identity = stamp_asset_load(&mut again, 2);
        let first_ready = echo("slot", "ready", 1, "red", 1);
        assert!(first_identity.answered_by(&first_ready));
        assert!(!again_identity.answered_by(&first_ready));
    }

    #[test]
    fn a_missing_or_partial_echo_answers_nothing_once_the_feature_is_negotiated() {
        let mut load = scene_load("slot", "red", 1);
        let identity = stamp_asset_load(&mut load, 3);
        assert!(!identity.answered_by(&json!({"type":"asset.state","id":"slot","state":"ready"})));
        assert!(
            !identity.answered_by(
                &json!({"type":"asset.state","id":"slot","state":"ready","request":3})
            )
        );
        assert!(!identity.answered_by(&echo("slot", "ready", 3, "red", 2)));
        assert!(!identity.answered_by(&echo("slot", "ready", 3, "blue", 1)));
        let mut changed = echo("slot", "ready", 3, "red", 1);
        changed["scene"]["content_hash"] = json!("another");
        assert!(!identity.answered_by(&changed));
    }

    #[test]
    fn a_load_without_a_scene_identity_is_matched_on_the_request_alone() {
        let mut load = json!({"v":1,"type":"asset.load","asset":{"id":"a","kind":"still",
            "source":{"type":"file","path":"/a.png"},"meta":{}}});
        let identity = stamp_asset_load(&mut load, 5);
        assert_eq!(identity.scene, None);
        assert!(identity.answered_by(&json!({"type":"asset.state","id":"a","request":5})));
        assert!(!identity.answered_by(&json!({"type":"asset.state","id":"a","request":6})));
    }

    #[test]
    fn decode_carries_the_echo_of_an_asset_state_and_nothing_else() {
        // The echo is the identity fields as received; a path or other field never travels with it.
        let mut message = echo("slot", "ready", 9, "red", 1);
        message["meta"] = json!({"width": 2});
        message["source"] = json!({"path": "/secret.png"});
        let BackendEvent::Asset { echo, status, .. } = decode_event(&message).unwrap() else {
            panic!("an asset event");
        };
        assert_eq!(status, AssetStatus::Ready(json!({"width": 2})));
        let echo = echo.expect("the identity was echoed");
        assert_eq!(
            echo,
            json!({"request": 9, "scene": {"asset_id":"red","revision_id":"red-r1",
                "content_hash":"h-red","generation":1}})
        );
        // What was decoded is judged by the same pure rule as the raw message.
        let mut load = scene_load("slot", "red", 1);
        let identity = stamp_asset_load(&mut load, 9);
        assert!(identity.answered_by(&echo));
        assert!(!stamp_asset_load(&mut load, 10).answered_by(&echo));
    }

    #[test]
    fn decode_of_a_runtime_without_the_echo_has_none() {
        for message in [
            json!({"v":1,"type":"asset.state","id":"slot","state":"ready"}),
            json!({"v":1,"type":"asset.state","id":"slot","state":"loading","request":null}),
        ] {
            let BackendEvent::Asset { echo, .. } = decode_event(&message).unwrap() else {
                panic!("an asset event");
            };
            assert_eq!(echo, None, "{message}");
        }
        let request_only = json!({"v":1,"type":"asset.state","id":"slot","state":"failed","request":4,
                "error":{"code":"cancelled","message":"x"}});
        let BackendEvent::Asset { echo, .. } = decode_event(&request_only).unwrap() else {
            panic!("an asset event");
        };
        assert_eq!(echo, Some(json!({"request": 4})));
    }

    #[test]
    fn the_identity_field_is_additive_to_the_plain_load_message() {
        let plain = scene_load("slot", "red", 1);
        let mut stamped = plain.clone();
        stamp_asset_load(&mut stamped, 9);
        let mut without = stamped.clone();
        without["asset"].as_object_mut().unwrap().remove("request");
        assert_eq!(without, plain, "only `asset.request` was added");
        // The decoder of the ordinary session ignores the echo: the same event as without it.
        assert!(matches!(
            decode_event(&echo("slot", "ready", 9, "red", 1)).unwrap(),
            BackendEvent::Asset {
                status: AssetStatus::Ready(_),
                ..
            }
        ));
    }

    #[test]
    fn hello_defaults_stay_byte_identical_and_options_are_additive() {
        assert_eq!(
            hello("tessel-editor"),
            json!({"v":1,"type":"hello","host":"tessel-editor"})
        );
        let with = hello_with(
            "tessel-editor",
            HelloOptions {
                retain: Some(false),
                presented: true,
                timing: true,
                gpu_finish: true,
                result_cache: Some(false),
                scene_samples: Some(2),
                preview_scene_samples: Some(0),
                preview_sort: Some(PreviewSort::Exact),
                preupload_textures: true,
            },
        );
        assert_eq!(with["retain"], false);
        assert_eq!(with["preupload_textures"], true);
        assert_eq!(with["preview_scene_samples"].as_u64(), Some(0));
        assert_eq!(with["preview_sort"], "exact");
        assert_eq!(with["telemetry"]["presented"], true);
        assert_eq!(with["telemetry"]["timing"], true);
        assert_eq!(with["telemetry"]["gpu_finish"], true);
        // The diagnostic arm is only ever asked for with the timing it reports through.
        let finish_alone = hello_with(
            "tessel-editor",
            HelloOptions {
                gpu_finish: true,
                ..HelloOptions::default()
            },
        );
        assert_eq!(finish_alone, hello("tessel-editor"));
        // Each flag alone says only itself, and neither leaves the message as it always was.
        let timing_only = hello_with(
            "tessel-editor",
            HelloOptions {
                timing: true,
                ..HelloOptions::default()
            },
        );
        assert_eq!(timing_only["telemetry"], json!({"timing": true}));
        assert_eq!(
            (
                with["result_cache"].as_bool(),
                with["scene_samples"].as_u64()
            ),
            (Some(false), Some(2))
        );
        assert_eq!(with["host"], "tessel-editor");
        // The preview count is not in the message unless the host asks for it.
        let plain = hello_with("tessel-editor", HelloOptions::default());
        assert!(plain.get("preview_scene_samples").is_none());
        assert!(plain.get("preview_sort").is_none());
        // The texture pre-upload (BL157.3) is not in the message unless the host asks for it, and says only itself.
        assert!(plain.get("preupload_textures").is_none());
        let preupload = hello_with(
            "tessel-editor",
            HelloOptions {
                preupload_textures: true,
                ..HelloOptions::default()
            },
        );
        assert_eq!(
            preupload,
            json!({"v":1,"type":"hello","host":"tessel-editor","preupload_textures":true})
        );
    }

    #[test]
    fn the_preview_sort_names_round_trip_and_anything_else_is_refused() {
        for sort in [PreviewSort::Latest, PreviewSort::Exact] {
            assert_eq!(PreviewSort::parse(sort.as_str()), Some(sort));
        }
        assert_eq!(PreviewSort::parse(" Exact "), Some(PreviewSort::Exact));
        assert_eq!(PreviewSort::parse("fast"), None);
        assert_eq!(PreviewSort::parse(""), None);
    }

    #[test]
    fn frame_done_keeps_additive_telemetry_and_presented_decodes() {
        let done = json!({"v":1,"type":"frame.done","id":1,"revision":1,"width":2,"height":2,
            "hash":null,"mean":null,"probes":[],"png":null,
            "timings":{"render_ms":1.0,"total_ms":2.0,"prepare_ms":0.5,"draw_ms":0.25,"flush_ms":0.125},
            "cache":{"texture_reused":1}});
        let BackendEvent::FrameDone(frame) = decode_event(&done).unwrap() else {
            panic!("expected a frame");
        };
        assert_eq!(
            (
                frame.detail.prepare_ms,
                frame.detail.draw_ms,
                frame.detail.flush_ms
            ),
            (0.5, 0.25, 0.125)
        );
        assert_eq!(frame.detail.cache.unwrap()["texture_reused"], 1);
        // A runtime that predates the telemetry answers with neither field.
        let old = json!({"v":1,"type":"frame.done","id":1,"revision":1,"width":2,"height":2,
            "hash":"ab","timings":{"render_ms":1.0,"total_ms":2.0}});
        let BackendEvent::FrameDone(frame) = decode_event(&old).unwrap() else {
            panic!("expected a frame");
        };
        assert_eq!(frame.detail, FrameDetail::default());
        let presented =
            json!({"v":1,"type":"frame.presented","id":3,"revision":2,"after_done_ms":4.5});
        assert_eq!(
            decode_event(&presented).unwrap(),
            BackendEvent::FramePresented {
                id: 3,
                revision: 2,
                after_done_ms: 4.5
            }
        );
    }

    struct Loopback(Vec<Value>);

    impl Transport for Loopback {
        fn send(&mut self, message: &Value) -> Result<(), BackendError> {
            self.0.push(message.clone());
            Ok(())
        }
        fn recv(&mut self, _: Duration) -> Option<Value> {
            None
        }
        fn close(&mut self) {}
    }

    #[test]
    fn the_bridge_reports_what_a_submit_spent_once() {
        use crate::plan::PlanRequest;
        use crate::state::{EditorState, ProjectSettings};
        let state = EditorState::empty(&ProjectSettings::default()).unwrap();
        let mut backend = BridgeBackend::new("t", Loopback(Vec::new()), Duration::ZERO);
        assert!(backend.take_submit_spans().is_none());
        let request = FrameRequest {
            id: 1,
            revision: 1,
            plan: state.plan(&PlanRequest::of(&state)).unwrap(),
            output: crate::backend::Output {
                width: 2,
                height: 2,
                png: None,
            },
            probes: Vec::new(),
            inspect: false,
            quality: Quality::Original,
        };
        backend.submit(&request).unwrap();
        assert!(backend.take_submit_spans().is_some());
        assert!(backend.take_submit_spans().is_none(), "taken, not repeated");
        assert_eq!(backend.transport_mut().0.len(), 1);
    }

    #[test]
    fn a_warm_is_the_plan_alone_and_the_backend_sends_it_without_a_frame() {
        use crate::plan::PlanRequest;
        use crate::state::{EditorState, ProjectSettings};
        let state = EditorState::empty(&ProjectSettings::default()).unwrap();
        let plan = state.plan(&PlanRequest::of(&state)).unwrap();
        let message = frame_warm(&plan);
        assert_eq!(message["type"], "frame.warm");
        assert_eq!(message["plan"], plan.to_json());
        assert!(
            message.get("id").is_none() && message.get("output").is_none(),
            "no picture is asked for, so nothing to identify or size"
        );
        let mut backend = BridgeBackend::new("t", Loopback(Vec::new()), Duration::ZERO);
        backend.warm_scene(&plan).unwrap();
        assert_eq!(backend.transport_mut().0.len(), 1);
        assert_eq!(backend.transport_mut().0[0]["type"], "frame.warm");
        // The warm of the splat scenes is the message it always was: nothing says `textures` or `scenes`.
        assert!(message.get("textures").is_none() && message.get("scenes").is_none());

        // BL157.3: the same message for the textures only; it does not ask for the splat warm.
        let textures = frame_warm_textures(&plan);
        assert_eq!(textures["type"], "frame.warm");
        assert_eq!(textures["plan"], plan.to_json());
        assert_eq!(
            (textures["textures"].as_bool(), textures["scenes"].as_bool()),
            (Some(true), Some(false))
        );
        backend.warm_textures(&plan).unwrap();
        assert_eq!(backend.transport_mut().0[1], textures);
    }

    #[test]
    fn quality_and_proxy_messages_are_additive_and_decode() {
        use crate::plan::PlanRequest;
        use crate::state::{EditorState, ProjectSettings};
        let state = EditorState::empty(&ProjectSettings::default()).unwrap();
        let mut request = FrameRequest {
            id: 1,
            revision: 1,
            plan: state.plan(&PlanRequest::of(&state)).unwrap(),
            output: crate::backend::Output {
                width: 2,
                height: 2,
                png: None,
            },
            probes: Vec::new(),
            inspect: true,
            quality: Quality::Original,
        };
        assert!(
            frame_render(&request).get("quality").is_none(),
            "an original frame is the message that predates the field"
        );
        request.quality = Quality::Preview;
        assert_eq!(frame_render(&request)["quality"], "preview");

        let id = Id::lit("vid");
        let attach = asset_proxy(
            id,
            Some(&json!({ "pattern": "/p/p%06d.jpg", "first": 0, "count": 3 })),
        );
        assert_eq!(
            (attach["type"].as_str(), attach["proxy"]["count"].as_i64()),
            (Some("asset.proxy"), Some(3))
        );
        assert!(asset_proxy(id, None)["proxy"].is_null());

        let state_of = |state: &str| {
            decode_event(&json!({"v":1,"type":"asset.proxy","id":"vid","state":state,
                "error":{"code":"invalid_proxy","message":"wrong count"}}))
            .unwrap()
        };
        assert!(matches!(
            state_of("attached"),
            BackendEvent::Proxy {
                state: ProxyState::Attached,
                ..
            }
        ));
        assert!(matches!(
            state_of("detached"),
            BackendEvent::Proxy {
                state: ProxyState::Detached,
                ..
            }
        ));
        let BackendEvent::Proxy {
            state: ProxyState::Failed(error),
            ..
        } = state_of("failed")
        else {
            panic!("a failed proxy keeps its error");
        };
        assert_eq!(error.code, "invalid_proxy");
        assert!(
            decode_event(&json!({"v":1,"type":"asset.proxy","id":"vid","state":"odd"})).is_err()
        );

        let done = json!({"v":1,"type":"frame.done","id":1,"revision":1,"width":2,"height":2,
            "timings":{"render_ms":1.0,"total_ms":2.0},
            "source":{"quality":"preview","proxy_layers":2,"original_layers":1}});
        let BackendEvent::FrameDone(frame) = decode_event(&done).unwrap() else {
            panic!("expected a frame");
        };
        assert_eq!(frame.detail.proxy_layers(), 2);
        assert_eq!(frame.to_json()["source"]["original_layers"], 1);
        assert!(
            frame.detail.is_reduced(),
            "a proxy picture is a reduced one"
        );
        assert_eq!(frame.detail.scene_samples(), None);
    }

    #[test]
    fn frame_done_reports_reduced_scene_msaa_and_old_runtimes_report_none() {
        let decode = |source: Value| {
            let done = json!({"v":1,"type":"frame.done","id":1,"revision":1,"width":2,"height":2,
                "timings":{"render_ms":1.0,"total_ms":2.0}, "source": source});
            let BackendEvent::FrameDone(frame) = decode_event(&done).unwrap() else {
                panic!("expected a frame");
            };
            frame.detail
        };
        let reduced = decode(
            json!({"quality":"preview","proxy_layers":0,"original_layers":0,
            "result_hits":0,"scene_samples":2,"reduced_scene_layers":1}),
        );
        assert_eq!(
            (reduced.scene_samples(), reduced.reduced_scene_layers()),
            (Some(2), 1)
        );
        assert!(reduced.is_reduced());
        let final_quality = decode(json!({"quality":"original","proxy_layers":0,
            "scene_samples":4,"reduced_scene_layers":0}));
        assert_eq!(final_quality.scene_samples(), Some(4));
        assert!(!final_quality.is_reduced());
        // A runtime that predates the field, and a frame without a scene layer, are never reduced.
        assert!(!decode(json!({"quality":"preview","proxy_layers":0})).is_reduced());
        assert!(!FrameDetail::default().is_reduced());
    }

    #[test]
    fn a_splat_order_taken_from_an_earlier_camera_makes_the_picture_a_reduced_one() {
        let decode = |source: Value| {
            let done = json!({"v":1,"type":"frame.done","id":1,"revision":1,"width":2,"height":2,
                "timings":{"render_ms":1.0,"total_ms":2.0}, "source": source});
            let BackendEvent::FrameDone(frame) = decode_event(&done).unwrap() else {
                panic!("expected a frame");
            };
            frame.detail
        };
        // Full MSAA, no proxy: the order alone is what the final picture differs in.
        let stale = decode(
            json!({"quality":"preview","proxy_layers":0,"scene_samples":4,
            "reduced_scene_layers":0,"stale_sort_layers":2}),
        );
        assert_eq!(stale.stale_sort_layers(), 2);
        assert!(
            stale.is_reduced(),
            "the monitor refines it when the playhead stops"
        );
        let exact = decode(
            json!({"quality":"preview","proxy_layers":0,"scene_samples":4,
            "reduced_scene_layers":0,"stale_sort_layers":0}),
        );
        assert!(!exact.is_reduced());
        // A runtime that predates the field always waited for its sort: never stale.
        let old = decode(
            json!({"quality":"preview","proxy_layers":0,"scene_samples":4,"reduced_scene_layers":0}),
        );
        assert_eq!(old.stale_sort_layers(), 0);
        assert!(!old.is_reduced());
        assert_eq!(FrameDetail::default().stale_sort_layers(), 0);
    }
}
