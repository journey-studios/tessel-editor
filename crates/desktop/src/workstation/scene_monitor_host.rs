//! Asynchronous WebView adapter for the same SceneSlots/messages as the headless renderer.
use super::*;
use tessel_editor_core::backend::{AssetStatus, BackendEvent};
use tessel_editor_core::bridge::{decode_event, frame_cancel};
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
}

pub(super) struct DirectedViewport {
    pub scene: Option<String>,
    slots: SceneSlots,
    next: u64,
    key: Option<String>,
    desired: Option<Intent>,
    active: Option<Active>,
    suspended: bool,
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
            self.cancel_directed_active("A newer explicit frame was requested");
        }
        self.dispatch_directed();
        Ok(explicit.then_some(receiver))
    }

    fn cancel_directed_active(&mut self, reason: &str) {
        if let Some(active) = self.directed.active.take() {
            self.png_paths.remove(&active.id);
            let _ = self
                .session
                .backend_mut()
                .transport_mut()
                .send(&frame_cancel(active.id));
            if let Some(reply) = active.reply {
                let _ = reply.try_send(Err(CommandError::new("cancelled", reason)));
            }
        }
    }

    pub fn clear_directed(&mut self) {
        self.cancel_directed_active("The scene monitor was closed");
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
        self.directed.scene = None;
        self.directed.key = None;
        self.request_started = None;
        self.last_key = None;
        self.dirty = true;
    }

    pub(super) fn reset_directed_runtime(&mut self) {
        self.cancel_directed_active("The scene runtime was reset");
        self.directed.slots.reset();
        self.directed.key = None;
        self.directed.suspended = false;
    }

    pub(super) fn dispatch_directed(&mut self) {
        if !self.is_ready()
            || self.directed.suspended
            || !self.on_screen
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
        let prepared = self.directed.slots.prepare(&plan, &resources)?;
        for message in &prepared.releases {
            self.session.backend_mut().transport_mut().send(message)?;
        }
        for load in &prepared.loads {
            self.session
                .backend_mut()
                .transport_mut()
                .send(&load.message)?;
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
                self.reset_directed_runtime();
                self.directed.suspended = true;
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
                match status {
                    AssetStatus::Ready(_) => self.directed.slots.mark_ready(slot),
                    AssetStatus::Failed(error) => {
                        self.directed.slots.mark_failed(slot);
                        self.finish_directed(Err(error));
                    }
                    _ => {}
                }
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
            }
            return true;
        }
        false
    }
}
