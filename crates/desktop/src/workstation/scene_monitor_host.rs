//! Asynchronous WebView adapter for the same SceneSlots/messages as the headless renderer.
use super::*;
use tessel_editor_core::backend::{AssetStatus, BackendEvent};
use tessel_editor_core::bridge::{
    FEATURE_ASSET_IDENTITY, LoadIdentity, decode_event, frame_cancel, stamp_asset_load,
};
use tessel_editor_core::scene_plan::ScenePlan;
use tessel_editor_media::scene_resources::SceneResourceSession;
use tessel_editor_render::FrameOptions;
use tessel_editor_render::scene::{
    SceneFrameMessage, SceneResourceMap, SceneSlots, frame_render_message,
};

const FIRST_ID: u64 = 1 << 48;
struct Intent {
    plan: ScenePlan,
    resources: SceneResourceMap,
    options: FrameOptions,
    reply: Option<Sender<Result<Value, CommandError>>>,
}
struct Active {
    id: u64,
    waiting: Vec<String>,
    message: Value,
    sent: bool,
    reply: Option<Sender<Result<Value, CommandError>>>,
    plan: Value,
    /// The edit frame this request draws, so the report can say which picture is on screen.
    edit_frame: Value,
}

/// The last `asset.load` sent for a slot: the resource it holds and, with `asset.identity`
/// negotiated, the request the runtime has to echo for the answer to count (BL102).
struct SentLoad {
    asset_id: String,
    generation: u64,
    identity: Option<LoadIdentity>,
}

/// What the lifecycle rejected or survived; reported so a native proof can tell "nothing arrived"
/// from "an old answer arrived and was refused" (BL092).
#[derive(Default)]
struct DirectedStats {
    /// `frame.done`/`frame.failed` of a frame that is no longer the active one.
    stale_frames: u64,
    /// `asset.state` of a load that a release/reload already replaced.
    stale_asset_states: u64,
    context_losses: u64,
}

pub(super) struct DirectedViewport {
    pub scene: Option<String>,
    slots: SceneSlots,
    next: u64,
    key: Option<String>,
    desired: Option<Intent>,
    active: Option<Active>,
    suspended: bool,
    /// The last `asset.load` sent for each slot. A slot name is reused for another identity (LRU,
    /// new project), and the page answers loads one at a time, so only this tells the answer of a
    /// replaced load from the answer of the current one.
    loads: HashMap<String, SentLoad>,
    /// Next `asset.request`. Never reset (not even by a reload): a number is used once per host.
    next_request: u64,
    stats: DirectedStats,
    /// `(request id, edit frame)` of the last picture that was drawn and accepted.
    shown: Option<(u64, Value)>,
}
impl Default for DirectedViewport {
    fn default() -> Self {
        Self {
            scene: None,
            slots: SceneSlots::default(),
            next: FIRST_ID,
            key: None,
            desired: None,
            active: None,
            suspended: false,
            loads: HashMap::new(),
            next_request: 1,
            stats: DirectedStats::default(),
            shown: None,
        }
    }
}
impl ViewportHost {
    pub fn scene_error(&mut self, error: &CommandError) {
        self.reset_directed_runtime();
        self.failure = Some(BackendError::new(error.code, error.to_string()));
    }

    pub fn directed_scene(&self) -> Option<&str> {
        self.directed.scene.as_deref()
    }

    pub fn render_scene(
        &mut self,
        state: &mut EditorState,
        resources: &SceneResourceSession,
        request: &Value,
        explicit: bool,
    ) -> Result<Option<FrameWaiter>, CommandError> {
        let (plan, map, mut options) =
            tessel_editor_render::scene_operation::prepare(state, request, resources)?;
        // WHY refuse now instead of queueing: a queued explicit request waits on something that
        // only an event can end (the runtime booting, a context restore), and the monitor's own
        // re-sync after that event replaces the queue, so the caller would never hear back (BL092).
        // The ordinary `render.frame` answers the same way.
        if explicit {
            self.require_directed_runtime()?;
        }
        if !explicit {
            let width = plan.output.width as f64;
            let height = plan.output.height as f64;
            let scale = (960.0 / width.max(height)).min(1.0);
            options.size = Some((
                (width * scale).round().max(1.0) as u32,
                (height * scale).round().max(1.0) as u32,
            ));
            options.inspect = std::env::var(INSPECT_ENV).is_ok_and(|v| v == "1");
        }
        let key = format!("{}|{:?}|{:?}", plan.to_json(), map, options);
        if !explicit && self.directed.key.as_ref() == Some(&key) {
            // Same picture as the last request, but that request may still be parked (the monitor
            // was hidden while the playhead moved): showing it again must start it, or nothing
            // would until an unrelated message from the page.
            self.dispatch_directed();
            return Ok(None);
        }
        if self.directed.scene.is_none() {
            self.session.reset_project();
            self.fail_waiters(&BackendError::new("cancelled", "Directed scene selected"));
            self.last_key = None;
        }
        self.directed.scene = request["scene"].as_str().map(str::to_owned);
        self.directed.key = Some(key);
        let (sender, receiver) = async_channel::bounded(1);
        if let Some(old) = self.directed.desired.take()
            && let Some(reply) = old.reply
        {
            let _ = reply.try_send(Err(CommandError::new(
                "cancelled",
                "A newer scene frame replaced this request",
            )));
        }
        self.directed.desired = Some(Intent {
            plan,
            resources: map,
            options,
            reply: explicit.then_some(sender),
        });
        // Explicit requests supersede; interactive seeks coalesce without starving the active frame.
        if explicit {
            self.cancel_directed_active(&BackendError::new(
                "cancelled",
                "A newer explicit frame was requested",
            ));
        }
        self.dispatch_directed();
        Ok(explicit.then_some(receiver))
    }

    /// An explicit frame needs a runtime that can draw now; the monitor's own frames just wait.
    fn require_directed_runtime(&self) -> Result<(), CommandError> {
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
        if self.directed.suspended {
            return Err(CommandError::new(
                "context_lost",
                "The graphics context is lost; try again once it is restored",
            ));
        }
        Ok(())
    }

    /// Ends the frame in flight, answering its waiter with `reason` (a typed error, never silence).
    fn cancel_directed_active(&mut self, reason: &BackendError) {
        if let Some(active) = self.directed.active.take() {
            self.png_paths.remove(&active.id);
            let _ = self
                .session
                .backend_mut()
                .transport_mut()
                .send(&frame_cancel(active.id));
            if let Some(reply) = active.reply {
                let _ = reply.try_send(Err(backend_error(reason.clone())));
            }
        }
    }

    pub fn clear_directed(&mut self) {
        self.cancel_directed_active(&BackendError::new(
            "cancelled",
            "The scene monitor was closed",
        ));
        if let Some(intent) = self.directed.desired.take()
            && let Some(reply) = intent.reply
        {
            let _ = reply.try_send(Err(CommandError::new(
                "cancelled",
                "The scene monitor was closed",
            )));
        }
        for message in self.directed.slots.release_all() {
            let _ = self.session.backend_mut().transport_mut().send(&message);
        }
        self.directed.loads.clear();
        self.directed.shown = None;
        self.directed.scene = None;
        self.directed.key = None;
        self.request_started = None;
        self.last_key = None;
        self.dirty = true;
    }

    pub(super) fn reset_directed_runtime(&mut self) {
        self.reset_directed_runtime_for(&BackendError::new(
            "cancelled",
            "The scene runtime was reset",
        ));
    }

    /// The same page keeps running but this frame cannot be trusted (timeout, a request the core
    /// refused): the frame in flight is answered with `reason` and every slot is RELEASED in the page.
    /// WHY release instead of forget: the page still holds those pictures, and the next request picks
    /// slot names by LRU, so forgetting them leaks `scene-res-*` entries in the runtime until its
    /// 64-entry budget refuses every load (found by the native proof, BL092).
    pub(super) fn reset_directed_runtime_for(&mut self, reason: &BackendError) {
        self.cancel_directed_active(reason);
        for message in self.directed.slots.release_all() {
            let _ = self.session.backend_mut().transport_mut().send(&message);
        }
        self.forget_directed_state();
    }

    /// The page is gone (reload): nothing it held is left to release, so the book is only forgotten.
    pub(super) fn forget_directed_runtime(&mut self, reason: &BackendError) {
        self.cancel_directed_active(reason);
        self.directed.slots.reset();
        self.forget_directed_state();
    }

    fn forget_directed_state(&mut self) {
        self.directed.loads.clear();
        // The canvas lost its picture with the context/page, so nothing is "shown" any more.
        self.directed.shown = None;
        self.directed.key = None;
        self.directed.suspended = false;
    }

    /// The graphics context died; the page did not. The runtime keeps the decoded pictures (only their
    /// textures die and are rebuilt from the bitmaps), so the book stays valid: forgetting it would
    /// reload every picture into other slots while the old ones stay in the page. The frame in flight
    /// is answered with `reason`; drawing waits for `context.restored`.
    fn suspend_directed(&mut self, reason: &BackendError) {
        self.cancel_directed_active(reason);
        self.directed.shown = None;
        self.directed.key = None;
        self.directed.suspended = true;
    }

    pub(super) fn dispatch_directed(&mut self) {
        // An explicit request (it has a waiter) draws even if the monitor is hidden, like the
        // ordinary `render.frame`; hiding only parks the monitor's own frames.
        let explicit = self
            .directed
            .desired
            .as_ref()
            .is_some_and(|intent| intent.reply.is_some());
        if !self.is_ready()
            || self.directed.suspended
            || (!self.on_screen && !explicit)
            || self.directed.active.is_some()
        {
            return;
        }
        let Some(intent) = self.directed.desired.take() else {
            return;
        };
        let result = self.prepare_directed(intent.plan, intent.resources, &intent.options);
        match result {
            Ok(mut active) => {
                active.reply = intent.reply;
                self.directed.active = Some(active);
                self.request_started = Some(Instant::now());
                self.failure = None;
                self.send_directed_if_loaded();
            }
            Err(error) => {
                if let Some(reply) = intent.reply {
                    let _ = reply.try_send(Err(backend_error(error.clone())));
                }
                self.directed.key = None;
                self.failure = Some(error);
            }
        }
    }

    fn prepare_directed(
        &mut self,
        plan: ScenePlan,
        resources: SceneResourceMap,
        options: &FrameOptions,
    ) -> Result<Active, BackendError> {
        let caps = self
            .session
            .known_capabilities()
            .ok_or_else(|| BackendError::new("backend_unavailable", "Runtime unavailable"))?;
        if !caps.supports("directed_scene") {
            return Err(BackendError::new(
                "unsupported_format",
                "Rebuild the runtime to draw directed scenes",
            ));
        }
        let supported = caps.raw["capabilities"]["directed_scene"]["capabilities"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| {
                v.as_str()
                    .and_then(tessel_editor_core::scene_plan::Capability::parse)
            })
            .collect::<Vec<_>>();
        plan.require_capabilities(&supported)
            .map_err(|e| tessel_editor_render::scene::plan_error(&e))?;
        // Negotiated, not assumed: a runtime that does not list the feature gets no extra field and
        // is judged by the heuristics it always was (`superseded_load`).
        let identified = caps.supports(FEATURE_ASSET_IDENTITY);
        let mut prepared = self.directed.slots.prepare(&plan, &resources)?;
        for message in &prepared.releases {
            self.session.backend_mut().transport_mut().send(message)?;
            if let Some(slot) = message["id"].as_str() {
                self.directed.loads.remove(slot);
            }
        }
        for load in &mut prepared.loads {
            let identity = identified.then(|| {
                let request = self.directed.next_request;
                self.directed.next_request += 1;
                stamp_asset_load(&mut load.message, request)
            });
            self.session
                .backend_mut()
                .transport_mut()
                .send(&load.message)?;
            let scene = &load.message["asset"]["scene"];
            self.directed.loads.insert(
                load.slot.clone(),
                SentLoad {
                    asset_id: scene["asset_id"].as_str().unwrap_or_default().to_owned(),
                    generation: scene["generation"].as_u64().unwrap_or_default(),
                    identity,
                },
            );
        }
        let id = self.directed.next;
        self.directed.next += 1;
        let (width, height) = options.size.unwrap_or((960, 540));
        if let Some(path) = &options.png {
            self.png_paths.insert(id, path.clone());
        }
        self.output_size = Some((width, height));
        Ok(Active {
            id,
            waiting: prepared
                .pending_slots()
                .iter()
                .map(|v| (*v).to_owned())
                .collect(),
            message: frame_render_message(&SceneFrameMessage {
                id,
                revision: id,
                edit_frame: plan.frame.edit_frame,
                width,
                height,
                layer: prepared.layer,
                png: options
                    .png
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned()),
                probes: options.probes.clone(),
                inspect: options.inspect,
            }),
            sent: false,
            reply: None,
            plan: plan.to_json(),
            edit_frame: json!(plan.frame.edit_frame),
        })
    }

    fn send_directed_if_loaded(&mut self) {
        let Some(active) = self.directed.active.as_mut() else {
            return;
        };
        if active.sent
            || active
                .waiting
                .iter()
                .any(|slot| !self.directed.slots.is_ready(slot))
        {
            return;
        }
        active.sent = true;
        if let Err(error) = self
            .session
            .backend_mut()
            .transport_mut()
            .send(&active.message)
        {
            self.finish_directed(Err(error));
        }
    }

    fn finish_directed(&mut self, result: Result<FrameResult, BackendError>) {
        let Some(active) = self.directed.active.take() else {
            return;
        };
        self.request_started = None;
        match result {
            Ok(frame) => {
                if let Some(reply) = active.reply {
                    let _ = reply.try_send(frame_report(&active.plan, &frame));
                }
                self.failure = None;
                self.note_presented(active.edit_frame.as_i64());
                self.directed.shown = Some((active.id, active.edit_frame));
                self.last_frame = Some(frame);
            }
            Err(error) => {
                if let Some(reply) = active.reply {
                    let _ = reply.try_send(Err(backend_error(error.clone())));
                }
                self.directed.key = None;
                self.failure = Some(error);
            }
        }
        self.dispatch_directed();
    }

    /// Scene slot/large frame IDs belong to this adapter; all other IPC stays in the ordinary session.
    pub(super) fn directed_ipc(&mut self, message: &Value) -> bool {
        match message["type"].as_str() {
            Some("context.lost") => {
                self.directed.stats.context_losses += 1;
                let error = BackendError::new(
                    "context_lost",
                    "The graphics context was lost while the frame was drawn",
                );
                self.suspend_directed(&error);
                // A context that is never restored must be visible, not a frozen last picture.
                // The ordinary monitor reports its own loss through the session.
                if self.directed.scene.is_some() {
                    self.failure = Some(error);
                }
            }
            Some("context.restored") => {
                self.directed.suspended = false;
                self.directed.key = None;
            }
            _ => {}
        }
        let slot = message["id"]
            .as_str()
            .filter(|id| id.starts_with("scene-res-"));
        if let Some(slot) = slot {
            if let Ok(BackendEvent::Asset { status, .. }) = decode_event(message) {
                self.directed_asset_state(slot, status, message);
            }
            self.send_directed_if_loaded();
            return true;
        }
        if message["id"].as_u64().is_some_and(|id| id >= FIRST_ID) {
            if self
                .directed
                .active
                .as_ref()
                .is_some_and(|a| message["id"] == a.id)
            {
                match decode_event(message) {
                    Ok(BackendEvent::FrameDone(frame)) => self.finish_directed(Ok(frame)),
                    Ok(BackendEvent::FrameFailed { error, .. }) => self.finish_directed(Err(error)),
                    _ => {}
                }
            } else if matches!(
                message["type"].as_str(),
                Some("frame.done" | "frame.failed")
            ) {
                // A frame this host already cancelled or replaced: its picture is not the
                // current one and must not reach a waiter or the header.
                self.directed.stats.stale_frames += 1;
            }
            return true;
        }
        false
    }

    /// `asset.state` of a scene slot. Only the answer of the load this host sent last for the slot
    /// may change the book: the page answers loads in its own order, so a replaced load can end
    /// (`cancelled`, `resource_stale`) after its successor was sent.
    ///
    /// With `asset.identity` negotiated the answer must repeat the `request` (and scene identity)
    /// of that load: an old `ready` that reaches a slot already given to a newer load, even the
    /// same picture, confirms nothing. Without it the slot name and the failure's details are all
    /// there is, as before.
    fn directed_asset_state(&mut self, slot: &str, status: AssetStatus, message: &Value) {
        let foreign = self
            .directed
            .loads
            .get(slot)
            .and_then(|sent| sent.identity.as_ref())
            .is_some_and(|identity| !identity.answered_by(message));
        match status {
            AssetStatus::Ready(_) => {
                if self.directed.loads.contains_key(slot) && !foreign {
                    self.directed.slots.mark_ready(slot);
                } else {
                    self.directed.stats.stale_asset_states += 1;
                }
            }
            AssetStatus::Failed(error) => {
                if foreign || self.superseded_load(slot, &error) {
                    self.directed.stats.stale_asset_states += 1;
                    return;
                }
                self.directed.loads.remove(slot);
                self.directed.slots.mark_failed(slot);
                self.finish_directed(Err(error));
            }
            _ => {}
        }
    }

    /// Whether a failed load is the end of a load that was already replaced or released.
    fn superseded_load(&self, slot: &str, error: &BackendError) -> bool {
        let Some(sent) = self.directed.loads.get(slot) else {
            return true;
        };
        // The caller checked the echoed request: this is the end of the load that holds the slot
        // now, so what the runtime says is a real answer, `cancelled` included.
        if sent.identity.is_some() {
            return false;
        }
        let (asset_id, generation) = (&sent.asset_id, &sent.generation);
        // The runtime ends a load that a release (or another load of the same id) overtook as `cancelled`.
        if error.code == "cancelled" {
            return true;
        }
        let details = error.details.as_ref();
        let other_asset = details
            .and_then(|d| d["asset_id"].as_str())
            .is_some_and(|id| id != asset_id);
        let other_generation = details
            .and_then(|d| d["generation"].as_u64())
            .is_some_and(|g| g != *generation);
        other_asset || other_generation
    }

    /// Lifecycle facts for `render.status` and the `monitor.viewport` node: what is in flight, what
    /// is parked, which slots the book holds and how many old answers were refused.
    pub(super) fn directed_report(&self) -> Value {
        json!({
            "scene": self.directed.scene,
            "suspended": self.directed.suspended,
            "asset_identity": self
                .session
                .known_capabilities()
                .is_some_and(|caps| caps.supports(FEATURE_ASSET_IDENTITY)),
            "active": self.directed.active.as_ref().map(|active| json!({
                "id": active.id, "sent": active.sent, "waiting": active.waiting,
                "explicit": active.reply.is_some(),
            })),
            "queued": self.directed.desired.as_ref().map(|intent| json!({
                "explicit": intent.reply.is_some(),
            })),
            "slots": {
                "occupied": self.directed.slots.occupied(),
                "generation": self.directed.slots.generation(),
                "tracked_loads": self.directed.loads.len(),
                "identified_loads": self
                    .directed
                    .loads
                    .values()
                    .filter(|sent| sent.identity.is_some())
                    .count(),
            },
            "refused": {
                "stale_frames": self.directed.stats.stale_frames,
                "stale_asset_states": self.directed.stats.stale_asset_states,
            },
            "context_losses": self.directed.stats.context_losses,
            "shown": self.directed.shown.as_ref().map(|(id, frame)| json!({
                "request_id": id, "edit_frame": frame,
            })),
        })
    }
}

#[cfg(test)]
#[path = "scene_lifecycle_tests.rs"]
mod scene_lifecycle_tests;
