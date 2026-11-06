//! Viewport session: the single place that decides which backend answer may be shown.
//!
//! The session owns the request/revision protocol, asset loading and the status the monitor
//! displays. Two entry points ask for a picture:
//!
//! * [`ViewportSession::request_frame`] is an *explicit* request (CLI `render.frame`, proofs,
//!   export). It supersedes whatever is in flight and answers with the data it asked for
//!   (hash, probes, PNG). Answers to an older request (a cancelled render, a response arriving
//!   after the project was replaced) are counted and dropped.
//! * [`ViewportSession::request_preview`] is the *interactive* path of the monitor. While a
//!   preview frame is in flight and only the playhead moved, the new intent does **not**
//!   cancel it: it waits in a single "latest wins" slot and is submitted when the frame in
//!   flight is accepted ([`ViewportSession::dispatch_pending`]). A scrub therefore shows every
//!   frame the backend manages to finish and converges to the last position, instead of
//!   cancelling each render before it can be shown.
//!
//! The desktop monitor and the headless CLI use this same type, only with different backends.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::assets::ProxyFrames;
use crate::backend::{
    AssetDescriptor, AssetStatus, BackendError, BackendEvent, Capabilities, FrameRequest,
    FrameResult, Output, ProxyState, RenderBackend, RequestId,
};
use crate::id::Id;
use crate::plan::{PlanRequest, PlanTime, PreviewIntent, RenderPlan};
use crate::preview::{PreviewPolicy, Quality};
use crate::state::EditorState;
use crate::trace::TraceHandle;

#[derive(Debug, Clone, PartialEq)]
enum Slot {
    Loading,
    Ready,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Idle,
    /// Waiting for the backend to load these assets.
    Loading(Vec<Id>),
    Rendering(RequestId),
    Ready(FrameResult),
    Failed(BackendError),
}

impl Status {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Loading(_) => "loading",
            Self::Rendering(_) => "rendering",
            Self::Ready(_) => "ready",
            Self::Failed(_) => "failed",
        }
    }
}

/// Outcome of processing one backend event.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionEvent {
    /// The status changed (assets loaded, render started, ...).
    Progress,
    Frame(FrameResult),
    Failed(BackendError),
    /// An answer to a superseded request was discarded.
    Stale(RequestId),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stats {
    pub requests: u64,
    pub accepted: u64,
    pub cancelled: u64,
    pub dropped_stale: u64,
    /// Preview intents that replaced the pending slot instead of cancelling the frame in flight.
    pub coalesced: u64,
    /// Proxies the runtime confirmed, rejected (it then draws the original), and the
    /// original-quality frames that replaced a proxy picture once the scrub settled.
    pub proxy_attached: u64,
    pub proxy_failed: u64,
    pub refined: u64,
}

/// How long the monitor must be still before a proxy picture is replaced by the original.
/// WHY not immediately: a scrub that pauses for a tick would pay an original-quality render
/// (the expensive one) for a picture it is about to replace.
pub const REFINE_DELAY: Duration = Duration::from_millis(200);

/// Who asked for a frame; decides coalescing, readback and quality.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    /// `render.frame`, proofs, export: original quality, answers with the data it asked for.
    Explicit,
    /// The monitor following the playhead: may use proxies and coalesces.
    Interactive,
    /// The monitor replacing a proxy picture with the original; superseded by any new intent.
    Refine,
}

/// What [`ViewportSession::request_preview`] did with an intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preview {
    /// A request was started (it superseded the previous one, if any).
    Submitted(RequestId),
    /// A frame is in flight and still valid: the intent waits for it, newest wins.
    Coalesced,
}

struct Active {
    request: FrameRequest,
    required: Vec<Id>,
    /// The proxy each required asset should have attached when the frame is submitted.
    wanted: HashMap<Id, Option<ProxyFrames>>,
    submitted: bool,
    /// Asked by the monitor (scrub or refine), not by an explicit caller. Only
    /// `Origin::Interactive` frames may be followed by a pending intent instead of cancelled.
    interactive: bool,
    origin: Origin,
    /// Session work between asking and having the request ready (zero when tracing is off). It
    /// excludes the wait for assets: that is loading, not preparing.
    prepare: Duration,
}

/// The newest preview intent that arrived while a frame was in flight.
struct Pending {
    plan: RenderPlan,
    output: Output,
}

pub struct ViewportSession<B: RenderBackend> {
    backend: B,
    demo_dir: Option<PathBuf>,
    capabilities: Option<Capabilities>,
    assets: HashMap<Id, Slot>,
    /// The decoded frames (if any) each loaded asset was sent from. A container video that is
    /// decoded after its first draw changes source (native video -> frame cache); without this the
    /// runtime would keep the old one and could never take a preview proxy.
    loaded_from: HashMap<Id, Option<String>>,
    next_request: RequestId,
    revision: u64,
    active: Option<Active>,
    pending: Option<Pending>,
    retain: Option<usize>,
    policy: PreviewPolicy,
    /// What the runtime holds as the proxy of each loaded asset (the descriptor it was sent).
    proxies: HashMap<Id, Value>,
    /// Proxy descriptors the runtime refused: not sent again until they change.
    refused: HashMap<Id, Value>,
    /// Why the runtime last refused a proxy: the picture falls back to the original, and this is
    /// where a person finds out why (`session.proxy_error`).
    proxy_error: Option<(Id, BackendError)>,
    /// The picture on screen came from a proxy; this is what would replace it.
    refine: Option<Pending>,
    last_intent: Option<Instant>,
    /// The request whose picture is on screen and the sequence frame it shows (`None` for a
    /// source preview): what a proof compares with the playhead to see how far behind it is.
    presented: Option<(RequestId, Option<i64>)>,
    /// Interactive frames skip the readback/hash unless this is set (measurement, proofs).
    inspect_previews: bool,
    status: Status,
    /// Set after a lost GL context is restored: the picture is gone and must be requested again.
    pub needs_redraw: bool,
    pub stats: Stats,
    trace: TraceHandle,
}

impl<B: RenderBackend> ViewportSession<B> {
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            demo_dir: None,
            capabilities: None,
            assets: HashMap::new(),
            loaded_from: HashMap::new(),
            next_request: 1,
            revision: 0,
            active: None,
            pending: None,
            retain: None,
            policy: PreviewPolicy::default(),
            proxies: HashMap::new(),
            refused: HashMap::new(),
            proxy_error: None,
            refine: None,
            last_intent: None,
            presented: None,
            inspect_previews: false,
            status: Status::Idle,
            needs_redraw: false,
            stats: Stats::default(),
            trace: TraceHandle::default(),
        }
    }

    /// The opt-in recorder of host-side spans (off until enabled). Clone it to record from other
    /// places; see `trace.rs` for what is and is not measured.
    pub fn trace(&self) -> &TraceHandle {
        &self.trace
    }

    /// Directory with the bundled demonstration images (`coast.png`, `highlands.png`).
    pub fn with_demo_dir(mut self, dir: PathBuf) -> Self {
        self.demo_dir = Some(dir);
        self
    }

    /// Makes interactive frames carry the pixel hash, mean and probes like explicit ones. Off by
    /// default: the readback and hash are measurement, not something the picture needs.
    pub fn set_preview_inspection(&mut self, inspect: bool) {
        self.inspect_previews = inspect;
    }

    /// Which quality interactive frames may use. Takes effect at the next request; the monitor
    /// asks for a new frame when the policy changes (its key includes it).
    pub fn set_preview_policy(&mut self, policy: PreviewPolicy) {
        if self.policy == policy {
            return;
        }
        self.policy = policy;
        self.refine = None;
    }

    pub fn preview_policy(&self) -> PreviewPolicy {
        self.policy
    }

    pub fn inspects_previews(&self) -> bool {
        self.inspect_previews
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn loaded_assets(&self) -> Vec<Id> {
        let mut ids: Vec<Id> = self
            .assets
            .iter()
            .filter(|(_, slot)| **slot == Slot::Ready)
            .map(|(id, _)| *id)
            .collect();
        ids.sort();
        ids
    }

    /// What the runtime reported, without starting or waiting for anything. A host whose
    /// runtime answers on the same thread that asks (a WebView) must use this, never
    /// [`capabilities`](Self::capabilities), which blocks until the handshake completes.
    pub fn known_capabilities(&self) -> Option<&Capabilities> {
        self.capabilities.as_ref()
    }

    pub fn capabilities(&mut self) -> Result<&Capabilities, BackendError> {
        if self.capabilities.is_none() {
            self.capabilities = Some(self.backend.capabilities()?);
        }
        self.capabilities
            .as_ref()
            .ok_or_else(|| BackendError::new("backend_unavailable", "No capabilities"))
    }

    fn fail(&mut self, error: BackendError) -> BackendError {
        self.active = None;
        // A failure is shown, not masked by retrying the newest intent: the next change retries.
        self.pending = None;
        self.status = Status::Failed(error.clone());
        error
    }

    /// Asks for a frame of `plan`. Cancels the request in flight, loads missing assets and
    /// submits as soon as they are ready. Returns the request ID the answer will carry.
    ///
    /// Failures leave the project and the previously shown frame untouched: the status
    /// becomes `Failed` and the caller keeps editing.
    pub fn request_frame(
        &mut self,
        state: &EditorState,
        plan: RenderPlan,
        output: Output,
        probes: Vec<(u32, u32)>,
    ) -> Result<RequestId, BackendError> {
        // An explicit request is the newest intent: nothing queued behind it may run later.
        self.pending = None;
        self.start(state, plan, output, probes, Origin::Explicit)
    }

    /// The monitor's request. Unlike [`request_frame`](Self::request_frame) it does not cancel a
    /// frame that is rendering when the project is unchanged and only the time moved (see
    /// [`Self::can_coalesce`]); the intent is parked in the pending slot, replacing an older one.
    /// Anything else (another project state, another subject or size, a frame waiting for
    /// assets, an explicit request in flight) supersedes exactly like `request_frame`.
    pub fn request_preview(
        &mut self,
        state: &EditorState,
        plan: RenderPlan,
        output: Output,
    ) -> Result<Preview, BackendError> {
        self.last_intent = Some(Instant::now());
        if self.can_coalesce(state, &plan, &output) {
            self.stats.coalesced += 1;
            // The same picture as the one in flight needs no second render; and if the
            // scrub came back to it, an older pending position is obsolete.
            let same_as_active = self
                .active
                .as_ref()
                .is_some_and(|active| active.request.plan == plan);
            self.pending = (!same_as_active).then_some(Pending { plan, output });
            return Ok(Preview::Coalesced);
        }
        self.pending = None;
        self.start(state, plan, output, Vec::new(), Origin::Interactive)
            .map(Preview::Submitted)
    }

    /// Whether the frame in flight may finish and be shown although `plan` is newer.
    ///
    /// WHY a replan instead of a project revision counter: undo can walk the revision back to a
    /// value an earlier frame was drawn at, and a replan answers the real question — "would the
    /// project, as it is now, still produce the plan that is rendering?" — for any edit,
    /// including ones that bypass the revision (asset replaced, a track hidden). Only the
    /// playhead/source time may differ.
    fn can_coalesce(&self, state: &EditorState, plan: &RenderPlan, output: &Output) -> bool {
        let Some(active) = &self.active else {
            return false;
        };
        if active.origin != Origin::Interactive || !active.submitted {
            return false;
        }
        let in_flight = &active.request;
        // A changed quality policy is a different picture: the frame in flight must not stand in
        // for it, or a toggle during a render would leave the old quality on screen.
        if in_flight.quality != self.policy.interactive_quality() {
            return false;
        }
        if in_flight.output != *output || !same_subject(&in_flight.plan.intent, &plan.intent) {
            return false;
        }
        let at = match in_flight.plan.frame {
            Some(frame) => PlanTime::Frame(frame),
            None => PlanTime::Time(in_flight.plan.time),
        };
        let replanned = state.plan(&PlanRequest {
            intent: in_flight.plan.intent.clone(),
            at,
        });
        replanned.is_ok_and(|replanned| replanned == in_flight.plan)
    }

    /// Submits the pending preview intent once nothing is in flight. Call after draining the
    /// backend's events. `None` when there is nothing to start.
    pub fn dispatch_pending(
        &mut self,
        state: &EditorState,
    ) -> Option<Result<RequestId, BackendError>> {
        if self.active.is_some() {
            return None;
        }
        let pending = self.pending.take()?;
        Some(self.start(
            state,
            pending.plan,
            pending.output,
            Vec::new(),
            Origin::Interactive,
        ))
    }

    /// Whether a proxy picture is on screen and an original one would replace it.
    pub fn refine_pending(&self) -> bool {
        self.refine.is_some()
    }

    /// Replaces a proxy picture with the original once the monitor has been still for
    /// [`REFINE_DELAY`]. `None` when there is nothing to refine, something is in flight or the
    /// scrub has not settled yet (the caller asks again when [`refine_pending`](Self::refine_pending)).
    pub fn dispatch_refine(
        &mut self,
        state: &EditorState,
        now: Instant,
    ) -> Option<Result<RequestId, BackendError>> {
        if self.active.is_some() || self.pending.is_some() {
            return None;
        }
        let settled = self
            .last_intent
            .is_none_or(|at| now.saturating_duration_since(at) >= REFINE_DELAY);
        if !settled {
            return None;
        }
        let refine = self.refine.take()?;
        self.stats.refined += 1;
        Some(self.start(
            state,
            refine.plan,
            refine.output,
            Vec::new(),
            Origin::Refine,
        ))
    }

    /// Loaded assets kept between requests; past it, a new request releases the ones its plan
    /// does not need. Applied when a request *starts* (after the previous one is cancelled), so a
    /// frame that is allowed to finish never loses a texture it is drawing.
    pub fn set_retained_assets(&mut self, limit: usize) {
        self.retain = Some(limit);
    }

    /// The plan being drawn, if a request is in flight.
    pub fn active_plan(&self) -> Option<&RenderPlan> {
        self.active.as_ref().map(|active| &active.request.plan)
    }

    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    fn start(
        &mut self,
        state: &EditorState,
        plan: RenderPlan,
        output: Output,
        probes: Vec<(u32, u32)>,
        origin: Origin,
    ) -> Result<RequestId, BackendError> {
        // Any new request makes the picture it replaces irrelevant, including a refine.
        self.refine = None;
        let interactive = origin == Origin::Interactive;
        let prepare_started = self.trace.start();
        let supported = self.capabilities()?.layers.clone();
        if let Some(layer) = plan
            .layers
            .iter()
            .find(|layer| !supported.iter().any(|kind| kind == layer.content.kind()))
        {
            let error = BackendError::new(
                "unsupported_format",
                format!("The backend cannot draw `{}` layers", layer.content.kind()),
            );
            return Err(self.fail(error));
        }
        if let Some(previous) = self.active.take() {
            self.stats.cancelled += 1;
            self.trace.frame_dropped(previous.request.id);
            // Best effort: the runtime may be gone already, and the answer is dropped by ID anyway.
            let _ = self.backend.cancel(previous.request.id);
        }
        self.revision += 1;
        let id = self.next_request;
        self.next_request += 1;
        self.stats.requests += 1;
        let required = plan.requires.clone();
        if let Some(limit) = self.retain
            && self.loaded_assets().len() > limit
        {
            self.release_except(&required.iter().copied().collect());
        }
        // Explicit requests keep today's behaviour byte for byte; only the monitor's own
        // frames may skip the readback, and only when nothing asked for pixels.
        let monitor = origin != Origin::Explicit;
        let inspect =
            !monitor || self.inspect_previews || output.png.is_some() || !probes.is_empty();
        // Only the monitor's own scrub may be a proxy picture; anything that asks for pixels or
        // is not the scrub (explicit, refine) is the original.
        let quality = if interactive && output.png.is_none() && probes.is_empty() {
            self.policy.interactive_quality()
        } else {
            Quality::Original
        };
        let request = FrameRequest {
            id,
            revision: self.revision,
            plan,
            output,
            probes,
            inspect,
            quality,
        };
        let wanted: HashMap<Id, Option<ProxyFrames>> = required
            .iter()
            .map(|asset| {
                let proxy = state.library.item(asset).and_then(|item| {
                    let asset = item.asset.as_ref()?;
                    self.policy.proxy_for(item.kind, asset).cloned()
                });
                (*asset, proxy)
            })
            .collect();
        for asset in &required {
            let changed = matches!(self.assets.get(asset), Some(Slot::Ready))
                && self.loaded_from.get(asset) != Some(&decoded_key(state, asset));
            if changed {
                let _ = self.backend.release_asset(*asset);
                self.assets.remove(asset);
                self.proxies.remove(asset);
                self.refused.remove(asset);
            }
        }
        let mut pending = Vec::new();
        for asset in &required {
            match self.assets.get(asset) {
                Some(Slot::Ready) => {}
                Some(Slot::Loading) => pending.push(*asset),
                None => {
                    let descriptor = state
                        .library
                        .item(asset)
                        .ok_or_else(|| {
                            BackendError::new(
                                "asset_unreadable",
                                format!("`{asset}` is not in the library"),
                            )
                        })
                        .and_then(|item| {
                            AssetDescriptor::from_item(item, self.demo_dir.as_deref())
                        });
                    let descriptor = match descriptor {
                        Ok(descriptor) => descriptor,
                        Err(error) => return Err(self.fail(error)),
                    };
                    // WHY: a missing file is reported here, with its path, instead of as a
                    // generic fetch failure from inside the runtime.
                    if let Some(missing) = descriptor
                        .files()
                        .into_iter()
                        .find(|file| !std::path::Path::new(file).is_file())
                    {
                        let error = BackendError::new(
                            "asset_unreadable",
                            format!("`{asset}`: file `{missing}` does not exist"),
                        );
                        return Err(self.fail(error));
                    }
                    if let Err(error) = self.backend.load_asset(&descriptor) {
                        return Err(self.fail(error));
                    }
                    self.assets.insert(*asset, Slot::Loading);
                    self.loaded_from.insert(*asset, decoded_key(state, asset));
                    pending.push(*asset);
                }
            }
        }
        self.active = Some(Active {
            request,
            required,
            wanted,
            submitted: false,
            interactive: monitor,
            origin,
            prepare: prepare_started.map_or(Duration::ZERO, |started| started.elapsed()),
        });
        if pending.is_empty() {
            self.submit_active()?;
        } else {
            self.status = Status::Loading(pending);
        }
        Ok(id)
    }

    fn submit_active(&mut self) -> Result<(), BackendError> {
        let Some(active) = &mut self.active else {
            return Ok(());
        };
        if active.submitted {
            return Ok(());
        }
        let id = active.request.id;
        active.submitted = true;
        let (interactive, prepare) = (active.interactive, active.prepare);
        let request = active.request.clone();
        let wanted = std::mem::take(&mut active.wanted);
        // Before the frame: the runtime handles messages in order, so the proxy a frame is
        // allowed to use is attached by the time it renders.
        self.sync_proxies(&wanted);
        match self.backend.submit(&request) {
            Ok(()) => {
                self.status = Status::Rendering(id);
                if let Some(at) = self.trace.start() {
                    let spans = self.backend.take_submit_spans().unwrap_or_default();
                    self.trace.frame_submitted(
                        id,
                        request.revision,
                        interactive,
                        prepare,
                        spans,
                        at,
                    );
                }
                Ok(())
            }
            Err(error) => Err(self.fail(error)),
        }
    }

    /// Brings the runtime's proxies in line with `wanted`. Only runs for assets that are
    /// loaded, files that exist and runtimes that announced `asset.proxy`; anything else leaves
    /// the original in use, which is always correct.
    fn sync_proxies(&mut self, wanted: &HashMap<Id, Option<ProxyFrames>>) {
        let supported = self
            .capabilities
            .as_ref()
            .is_some_and(|capabilities| capabilities.supports("asset.proxy"));
        if !supported {
            return;
        }
        for (id, proxy) in wanted {
            let descriptor = proxy.as_ref().and_then(proxy_descriptor);
            if self.proxies.get(id) == descriptor.as_ref() {
                continue;
            }
            if descriptor.is_some() && self.refused.get(id) == descriptor.as_ref() {
                continue;
            }
            if self.backend.set_proxy(*id, descriptor.as_ref()).is_err() {
                continue;
            }
            match descriptor {
                Some(descriptor) => self.proxies.insert(*id, descriptor),
                None => self.proxies.remove(id),
            };
        }
    }

    /// Processes at most one backend event.
    pub fn pump(&mut self, timeout: Duration) -> Option<SessionEvent> {
        let event = self.backend.poll(timeout)?;
        Some(match event {
            BackendEvent::Ready(capabilities) => {
                self.capabilities = Some(*capabilities);
                SessionEvent::Progress
            }
            BackendEvent::Asset { id, status, .. } => self.on_asset(id, status),
            BackendEvent::Proxy { id, state } => {
                match state {
                    ProxyState::Attached => self.stats.proxy_attached += 1,
                    ProxyState::Detached => {}
                    ProxyState::Failed(error) => {
                        self.stats.proxy_failed += 1;
                        self.proxy_error = Some((id, error));
                        if let Some(descriptor) = self.proxies.remove(&id) {
                            self.refused.insert(id, descriptor);
                        }
                    }
                }
                SessionEvent::Progress
            }
            BackendEvent::FrameDone(frame) => {
                let received = self.trace.start();
                let event = self.on_frame(frame);
                match (&event, received) {
                    (SessionEvent::Frame(frame), Some(at)) => {
                        let shown = self.presented.and_then(|(_, shown)| shown);
                        self.trace.frame_done(frame, shown, at, at.elapsed());
                    }
                    (SessionEvent::Stale(id), _) => self.trace.frame_dropped(*id),
                    _ => {}
                }
                event
            }
            BackendEvent::FramePresented {
                id, after_done_ms, ..
            } => {
                self.trace.frame_presented(id, after_done_ms);
                SessionEvent::Progress
            }
            BackendEvent::FrameFailed {
                id,
                revision,
                error,
            } => {
                self.trace.frame_dropped(id);
                if self.is_current(id, revision) {
                    SessionEvent::Failed(self.fail(error))
                } else {
                    self.stats.dropped_stale += 1;
                    SessionEvent::Stale(id)
                }
            }
            BackendEvent::ContextLost => {
                // Textures are gone with the context; every asset must be loaded again.
                self.assets.clear();
                self.proxies.clear();
                self.refused.clear();
                let error = BackendError::new("context_lost", "The graphics context was lost");
                SessionEvent::Failed(self.fail(error))
            }
            BackendEvent::ContextRestored => {
                self.needs_redraw = true;
                SessionEvent::Progress
            }
            BackendEvent::Closed(reason) => {
                self.assets.clear();
                self.proxies.clear();
                self.refused.clear();
                self.capabilities = None;
                let error = BackendError::new("backend_unavailable", reason);
                SessionEvent::Failed(self.fail(error))
            }
        })
    }

    fn is_current(&self, id: RequestId, revision: u64) -> bool {
        self.active.as_ref().is_some_and(|active| {
            active.request.id == id
                && active.request.revision == revision
                && revision == self.revision
        })
    }

    fn on_asset(&mut self, id: Id, status: AssetStatus) -> SessionEvent {
        match status {
            AssetStatus::Loading => SessionEvent::Progress,
            AssetStatus::Ready(_) => {
                self.assets.insert(id, Slot::Ready);
                let all_ready = self.active.as_ref().is_some_and(|active| {
                    active
                        .required
                        .iter()
                        .all(|asset| self.assets.get(asset) == Some(&Slot::Ready))
                });
                if all_ready && let Err(error) = self.submit_active() {
                    return SessionEvent::Failed(error);
                }
                if let Some(active) = &self.active
                    && !all_ready
                {
                    let pending = active
                        .required
                        .iter()
                        .filter(|asset| self.assets.get(*asset) != Some(&Slot::Ready))
                        .copied()
                        .collect();
                    self.status = Status::Loading(pending);
                }
                SessionEvent::Progress
            }
            AssetStatus::Failed(error) => {
                self.assets.remove(&id);
                let needed = self
                    .active
                    .as_ref()
                    .is_some_and(|active| active.required.contains(&id));
                if !needed {
                    return SessionEvent::Progress;
                }
                let wrapped =
                    BackendError::new(error.code.clone(), format!("`{id}`: {}", error.message));
                SessionEvent::Failed(self.fail(wrapped))
            }
        }
    }

    fn on_frame(&mut self, frame: FrameResult) -> SessionEvent {
        if !self.is_current(frame.id, frame.revision) {
            self.stats.dropped_stale += 1;
            return SessionEvent::Stale(frame.id);
        }
        let finished = self.active.take();
        // A proxy picture is on screen: remember how to replace it with the original.
        self.refine = finished.as_ref().and_then(|active| {
            (active.origin == Origin::Interactive && frame.detail.proxy_layers() > 0).then(|| {
                Pending {
                    plan: active.request.plan.clone(),
                    output: active.request.output.clone(),
                }
            })
        });
        self.presented = finished.map(|active| (active.request.id, active.request.plan.frame));
        self.stats.accepted += 1;
        self.needs_redraw = false;
        self.status = Status::Ready(frame.clone());
        SessionEvent::Frame(frame)
    }

    /// Requests a frame and waits for its answer. Stale answers received meanwhile are dropped.
    pub fn render_blocking(
        &mut self,
        state: &EditorState,
        plan: RenderPlan,
        output: Output,
        probes: Vec<(u32, u32)>,
        timeout: Duration,
    ) -> Result<FrameResult, BackendError> {
        let id = self.request_frame(state, plan, output, probes)?;
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                let _ = self.backend.cancel(id);
                self.stats.cancelled += 1;
                let error = BackendError::new(
                    "timeout",
                    format!("No frame within {timeout:?}; the request was cancelled"),
                );
                return Err(self.fail(error));
            }
            match self.pump(remaining.min(Duration::from_millis(250))) {
                Some(SessionEvent::Frame(frame)) if frame.id == id => return Ok(frame),
                Some(SessionEvent::Failed(error)) => return Err(error),
                _ => {}
            }
        }
    }

    /// Cancels the request in flight, if any. Its answer will be dropped as stale.
    pub fn cancel(&mut self) {
        self.pending = None;
        self.refine = None;
        if let Some(active) = self.active.take() {
            self.stats.cancelled += 1;
            self.trace.frame_dropped(active.request.id);
            let _ = self.backend.cancel(active.request.id);
            self.status = Status::Idle;
        }
    }

    /// Releases loaded assets that are not in `keep` (the monitor left them).
    pub fn release_except(&mut self, keep: &HashSet<Id>) {
        let drop: Vec<Id> = self
            .assets
            .keys()
            .filter(|id| !keep.contains(*id))
            .copied()
            .collect();
        for id in drop {
            let _ = self.backend.release_asset(id);
            self.assets.remove(&id);
            self.proxies.remove(&id);
            self.refused.remove(&id);
        }
    }

    /// Forgets every asset: the project was replaced, so IDs may now mean something else.
    pub fn reset_project(&mut self) {
        self.cancel();
        self.release_except(&HashSet::new());
        self.presented = None;
        self.status = Status::Idle;
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), BackendError> {
        self.backend.resize(width, height)
    }

    pub fn shutdown(&mut self) {
        self.cancel();
        self.assets.clear();
        self.proxies.clear();
        self.refused.clear();
        self.capabilities = None;
        self.backend.shutdown();
        self.status = Status::Idle;
    }

    pub fn summary(&self) -> serde_json::Value {
        json!({
            "backend": self.backend.name(),
            "status": self.status.label(),
            "revision": self.revision,
            "loaded_assets": self.loaded_assets(),
            "stats": {
                "requests": self.stats.requests, "accepted": self.stats.accepted,
                "cancelled": self.stats.cancelled, "dropped_stale": self.stats.dropped_stale,
                "coalesced": self.stats.coalesced, "proxy_attached": self.stats.proxy_attached,
                "proxy_failed": self.stats.proxy_failed, "refined": self.stats.refined,
            },
            "preview_policy": self.policy.to_json(), "refine_pending": self.refine.is_some(),
            "proxy_error": self.proxy_error.as_ref().map(|(id, error)| json!({ "asset": id, "error": error.to_json() })),
            "proxies": self.proxies.keys().collect::<Vec<_>>(),
            "pending": self.pending.is_some(),
            "presented": self.presented.map(|(request, frame)| json!({ "request": request, "frame": frame })),
            "inspect_previews": self.inspect_previews,
            "trace": if self.trace.enabled() { self.trace.snapshot(8) } else { json!({ "enabled": false }) },
            "needs_redraw": self.needs_redraw,
            "error": match &self.status { Status::Failed(error) => error.to_json(), _ => serde_json::Value::Null },
            "frame": match &self.status { Status::Ready(frame) => frame.to_json(), _ => serde_json::Value::Null },
        })
    }
}

/// What the runtime is told about a proxy, and the identity under which the session remembers
/// it. `None` when a file of the sequence is missing: the original stays in use.
/// What identifies the frames an asset is drawn from, see `Session::loaded_from`.
fn decoded_key(state: &EditorState, asset: &Id) -> Option<String> {
    let item = state.library.item(asset)?;
    let decoded = item.asset.as_ref()?.decoded.as_ref()?;
    decoded.frames.as_ref().map(|frames| frames.pattern.clone())
}

fn proxy_descriptor(proxy: &ProxyFrames) -> Option<Value> {
    let descriptor = json!({ "pattern": proxy.pattern, "first": proxy.first,
        "count": proxy.count, "width": proxy.width, "height": proxy.height,
        "format": proxy.format.label() });
    let probe = AssetDescriptor {
        id: Id::lit("proxy"),
        kind: crate::assets::MediaKind::Video,
        source: json!({ "type": "frame_sequence", "pattern": proxy.pattern,
            "first": proxy.first, "count": proxy.count }),
        meta: Value::Null,
    };
    probe
        .files()
        .iter()
        .all(|file| std::path::Path::new(file).is_file())
        .then_some(descriptor)
}

/// Same thing on screen: the sequence, or the same asset (at any source time).
fn same_subject(a: &PreviewIntent, b: &PreviewIntent) -> bool {
    match (a, b) {
        (PreviewIntent::Sequence, PreviewIntent::Sequence) => true,
        (PreviewIntent::Source { asset: a, .. }, PreviewIntent::Source { asset: b, .. }) => a == b,
        _ => false,
    }
}

/// Object-safe face of a session, so the headless executor can take any backend.
pub trait Renderer {
    fn render_frame(
        &mut self,
        state: &EditorState,
        plan: RenderPlan,
        output: Output,
        probes: Vec<(u32, u32)>,
        timeout: Duration,
    ) -> Result<FrameResult, BackendError>;
    fn capabilities(&mut self) -> Result<Capabilities, BackendError>;
    fn summary(&self) -> serde_json::Value;
    /// The project was replaced: forget loaded assets.
    fn reset_project(&mut self);
}

impl<B: RenderBackend> Renderer for ViewportSession<B> {
    fn render_frame(
        &mut self,
        state: &EditorState,
        plan: RenderPlan,
        output: Output,
        probes: Vec<(u32, u32)>,
        timeout: Duration,
    ) -> Result<FrameResult, BackendError> {
        self.render_blocking(state, plan, output, probes, timeout)
    }

    fn capabilities(&mut self) -> Result<Capabilities, BackendError> {
        ViewportSession::capabilities(self).cloned()
    }

    fn summary(&self) -> serde_json::Value {
        ViewportSession::summary(self)
    }

    fn reset_project(&mut self) {
        ViewportSession::reset_project(self);
    }
}

#[cfg(test)]
mod tests;
