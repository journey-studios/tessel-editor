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
    /// `Some(false)` makes every splat scene frame sort and draw again (rollback of the result
    /// cache of still scenes). `None` leaves the runtime's default (on).
    pub result_cache: Option<bool>,
    /// MSAA samples of scene targets, 0, 2 or 4 (see `docs/features/viewport.md`, OPT07): a
    /// visible quality choice, so it is only sent when the host asks. `None` is the runtime's 4.
    pub scene_samples: Option<u32>,
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
    if options.presented {
        message["telemetry"] = json!({ "presented": true });
    }
    if let Some(result_cache) = options.result_cache {
        message["result_cache"] = json!(result_cache);
    }
    if let Some(samples) = options.scene_samples {
        message["scene_samples"] = json!(samples);
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
            Ok(BackendEvent::Asset { id, status })
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
                result_cache: Some(false),
                scene_samples: Some(2),
            },
        );
        assert_eq!(with["retain"], false);
        assert_eq!(with["telemetry"]["presented"], true);
        assert_eq!(
            (
                with["result_cache"].as_bool(),
                with["scene_samples"].as_u64()
            ),
            (Some(false), Some(2))
        );
        assert_eq!(with["host"], "tessel-editor");
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
    }
}
