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

use crate::assets::{MediaKind, ProxyFrames};
use crate::backend::{
    AssetDescriptor, AssetStatus, BackendError, BackendEvent, Capabilities,
    FEATURE_SCENE_WARM_DRAW, FEATURE_TEXTURE_PREUPLOAD, FrameRequest, FrameResult, Output,
    ProxyState, RenderBackend, RequestId,
};
use crate::id::Id;
use crate::plan::{PlanRequest, PlanTime, PreviewIntent, RenderPlan};
use crate::preview::{PreviewPolicy, Quality};
use crate::state::EditorState;
use crate::trace::{TraceHandle, ms_since};

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
    /// final-quality frames that replaced a reduced picture (proxy or lighter MSAA) once the
    /// playhead stopped.
    pub proxy_attached: u64,
    pub proxy_failed: u64,
    pub refined: u64,
    /// Loads started ahead of any frame that needs them (see [`ViewportSession::prefetch`]).
    pub prefetched: u64,
    /// Prefetches skipped because the files already prefetched and not yet used fill the budget.
    pub prefetch_skipped: u64,
    /// Heavy assets released, least recently used first, to stay inside the byte budget.
    pub evicted: u64,
    /// Cuts whose splat scenes the runtime was asked to prepare ahead (see [`ViewportSession::warm`]).
    pub warmed: u64,
    /// Cuts whose still images the runtime was asked to upload ahead (see [`ViewportSession::warm_textures`]).
    pub preuploaded: u64,
    /// Loads given up and asked for again after a frame ran out of time (see [`ViewportSession::cancel_timed_out`]).
    pub reloaded: u64,
}

/// Assets below this size are never evicted for memory: releasing them frees nothing worth a reload.
const HEAVY_ASSET_BYTES: u64 = 1 << 20;

/// Bytes of source files the session may hold prefetched and not yet used by a frame. A 66 MB PLY
/// of 1.18 M splats takes ~100-250 MB once decoded, so the budget keeps a burst of clips that
/// start together from loading every scene of the next two seconds at once; what does not fit
/// loads when a frame needs it, as before.
pub const PREFETCH_BUDGET_BYTES: u64 = 256 << 20;

/// How long the monitor must be still before a reduced picture (proxy, or lighter MSAA) is
/// replaced by the final-quality one. WHY not immediately: a scrub that pauses for a tick would
/// pay a final-quality render (the expensive one) for a picture it is about to replace.
pub const REFINE_DELAY: Duration = Duration::from_millis(200);

/// Who asked for a frame; decides coalescing, readback and quality.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    /// `render.frame`, proofs, export: original quality, answers with the data it asked for.
    Explicit,
    /// The monitor following the playhead (playback and scrub): may use proxies and reduced scene
    /// MSAA, and coalesces.
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
    /// Assets the monitor asked the runtime to load ahead of any frame that needs them (see
    /// [`ViewportSession::prefetch`]); the retention release keeps them like a required asset.
    prefetched: HashMap<Id, u64>,
    prefetch_budget: u64,
    /// The cuts (by the assets their plan requires) the runtime was already asked to warm.
    warmed: HashSet<Vec<Id>>,
    /// Same key, for the texture pre-upload (BL157.3): a cut is asked once per pass of the playhead, like its splat warm.
    preuploaded: HashSet<Vec<Id>>,
    /// Whether a runtime that announces `texture.preupload` is sent the cuts' stills (off unless the host turns it on).
    preupload_enabled: bool,
    /// Whether the host's `hello` asked for the warm draw (BL157.3): reported only, the runtime does the work.
    warm_draw_enabled: bool,
    /// Whether a runtime that announces `scene.warm` is sent warms (on unless the host turns it off).
    warm_enabled: bool,
    /// Bytes of source files of every asset loaded or loading, and the request that last needed it.
    asset_bytes: HashMap<Id, u64>,
    last_used: HashMap<Id, u64>,
    retain_bytes: Option<u64>,
    /// Assets below this size are never evicted for memory (see [`HEAVY_ASSET_BYTES`]).
    min_evict_bytes: u64,
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
            prefetched: HashMap::new(),
            prefetch_budget: PREFETCH_BUDGET_BYTES,
            warmed: HashSet::new(),
            preuploaded: HashSet::new(),
            preupload_enabled: false,
            warm_draw_enabled: false,
            warm_enabled: true,
            asset_bytes: HashMap::new(),
            last_used: HashMap::new(),
            retain_bytes: None,
            min_evict_bytes: HEAVY_ASSET_BYTES,
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

    /// Uses `trace` as the recorder instead of a private one, so what the transport and the protocol handler record lands
    /// in the same log as the session's spans.
    pub fn with_trace(mut self, trace: TraceHandle) -> Self {
        self.trace = trace;
        self
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

    /// The sequence frame of the picture on screen (`None` for a source preview or nothing yet).
    pub fn presented_frame(&self) -> Option<i64> {
        self.presented.and_then(|(_, frame)| frame)
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

    /// Whether a reduced picture (a proxy, or a scene drawn with fewer MSAA samples) is on screen
    /// and a final-quality one would replace it.
    pub fn refine_pending(&self) -> bool {
        self.refine.is_some()
    }

    /// Replaces a reduced picture with the final-quality one once the monitor has been still for
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

    /// Bytes of source files the loaded assets may add up to. Past it, a request releases the heavy
    /// assets it does not need, least recently used first (never the ones it needs, nor the ones
    /// prefetched for the next cut). The count limit of [`Self::set_retained_assets`] cannot do
    /// this: sixteen assets can be sixteen scenes of 66 MB, each ~100-250 MB once decoded.
    pub fn set_retained_bytes(&mut self, limit: u64) {
        self.retain_bytes = Some(limit);
    }

    /// Overrides [`PREFETCH_BUDGET_BYTES`] (tests).
    pub fn set_prefetch_budget(&mut self, bytes: u64) {
        self.prefetch_budget = bytes;
    }

    /// The plan being drawn, if a request is in flight.
    pub fn active_plan(&self) -> Option<&RenderPlan> {
        self.active.as_ref().map(|active| &active.request.plan)
    }

    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Starts loading `assets` that are not loaded or loading, without a frame waiting for them.
    ///
    /// WHY: a cut or crossfade into a scene whose splats were never loaded stalls the first frame
    /// that draws it for the whole fetch + decode (≈ 200 ms for a 1.18 M splat PLY, measured in
    /// `docs/viewport-performance-splat-project-investigation.md`). The monitor calls this while
    /// the playhead is moving with the assets the sequence will need a moment ahead, so the load
    /// overlaps the frames before the cut. It is best effort and never an error: a missing or
    /// unreadable asset is reported by the frame that actually needs it, and nothing here
    /// changes the status, the revision or the request in flight. Returns how many loads started.
    pub fn prefetch(&mut self, state: &EditorState, assets: &[Id]) -> usize {
        if self.capabilities.is_none() {
            return 0;
        }
        let mut started = 0;
        for asset in assets {
            if self.assets.contains_key(asset) {
                continue;
            }
            let probing = self.trace.start();
            let Some(item) = state.library.item(asset) else {
                continue;
            };
            let Ok(descriptor) = AssetDescriptor::from_item(item, self.demo_dir.as_deref()) else {
                continue;
            };
            let files = descriptor.files();
            if files
                .iter()
                .any(|file| !std::path::Path::new(file).is_file())
            {
                continue;
            }
            let bytes = files_bytes(&files);
            let fs_ms = ms_since(probing);
            // The first prefetch always fits: the budget limits a burst, not a single big scene.
            let held: u64 = self.prefetched.values().sum();
            if held > 0 && held.saturating_add(bytes) > self.prefetch_budget {
                self.stats.prefetch_skipped += 1;
                continue;
            }
            let sending = self.trace.start();
            let sent = self.backend.load_asset(&descriptor);
            let send_ms = ms_since(sending);
            // BL157.2: where a slow prefetch spent its time on the calling thread, per load: the file checks (stat) or
            // the hand-over of the load to the backend. Only with the trace on; one event per load started, by kind and size
            // (never the asset's name).
            self.trace.event("prefetch_asset", || {
                json!({ "kind": descriptor.kind.label(), "bytes": bytes, "fs_ms": fs_ms, "send_ms": send_ms, "ok": sent.is_ok() })
            });
            if sent.is_err() {
                continue;
            }
            self.assets.insert(*asset, Slot::Loading);
            self.loaded_from.insert(*asset, decoded_key(state, asset));
            self.prefetched.insert(*asset, bytes);
            self.asset_bytes
                .insert(*asset, memory_bytes(descriptor.kind, bytes));
            self.stats.prefetched += 1;
            started += 1;
        }
        started
    }

    /// Asks the runtime to prepare the splat scenes of the cuts in `plans` (the plans at the frames
    /// where clips start a moment ahead of the playhead, see `EditorState::plans_ahead`).
    ///
    /// WHY: the first frame that draws a splat scene waits for the sort of a stage that never held it
    /// (50 to 70 ms for a 300 k splat scene, measured in
    /// `docs/viewport-performance-splat-project-investigation.md`), however early the file loaded:
    /// loading and decoding are not that sort. Once every asset of a cut is loaded the runtime is told
    /// to build the stage and run its first sort now, while the frames before the cut play, so the
    /// frame at the cut is drawn from an order that exists.
    ///
    /// Best effort and never an error. Sent only to a runtime that announced `scene.warm`, only for a
    /// plan with a scene layer whose assets are all loaded, and once per cut until it is passed, its
    /// assets are released or the context is lost. It changes neither the status, the revision nor the
    /// request in flight. Returns how many cuts were sent.
    pub fn warm(&mut self, plans: &[RenderPlan]) -> usize {
        // A cut the playhead passed (or that left the window) is forgotten: a later pass warms it again.
        self.warmed
            .retain(|key| plans.iter().any(|plan| &plan.requires == key));
        self.preuploaded
            .retain(|key| plans.iter().any(|plan| &plan.requires == key));
        let scenes = self.warm_enabled && self.announces("scene.warm");
        let textures = self.preupload_enabled && self.announces(FEATURE_TEXTURE_PREUPLOAD);
        if !scenes && !textures {
            return 0;
        }
        let mut sent = 0;
        for plan in plans {
            // A warm never makes the runtime wait for a file: every asset of the cut is loaded first.
            if plan
                .requires
                .iter()
                .any(|asset| self.assets.get(asset) != Some(&Slot::Ready))
            {
                continue;
            }
            if scenes
                && !self.warmed.contains(&plan.requires)
                && plan
                    .layers
                    .iter()
                    .any(|layer| layer.content.kind() == "scene" && !layer.occluded)
                && self.backend.warm_scene(plan).is_ok()
            {
                self.warmed.insert(plan.requires.clone());
                self.stats.warmed += 1;
                sent += 1;
            }
            if textures
                && !self.preuploaded.contains(&plan.requires)
                && plan
                    .layers
                    .iter()
                    .any(|layer| layer.content.kind() == "image" && !layer.occluded)
                && self.backend.warm_textures(plan).is_ok()
            {
                self.preuploaded.insert(plan.requires.clone());
                self.stats.preuploaded += 1;
                sent += 1;
            }
        }
        sent
    }

    fn announces(&self, feature: &str) -> bool {
        self.capabilities
            .as_ref()
            .is_some_and(|caps| caps.supports(feature))
    }

    /// Opts in to (or out of) the texture pre-upload (BL157.3, a diagnostic experiment, off by default): the
    /// monitor then also sends the cuts a moment ahead whose still images are all loaded, to a runtime that announced
    /// `texture.preupload` (it does only for a `hello` that asked). The runtime uploads those pictures' textures
    /// between frames, within its texture budget; nothing here changes a frame, the status or the revision.
    pub fn set_preupload(&mut self, enabled: bool) {
        self.preupload_enabled = enabled;
        if !enabled {
            self.preuploaded.clear();
        }
    }

    /// Records that the host's `hello` asked for the warm draw (BL157.3, a separate experiment): the runtime does it at the end of
    /// each `frame.warm`, so nothing is sent differently; this only makes the arm visible in `render.status`.
    pub fn set_warm_draw(&mut self, enabled: bool) {
        self.warm_draw_enabled = enabled;
    }

    /// What the pre-upload asked and whether the runtime can do it, for `render.status`.
    pub fn preupload_report(&self) -> serde_json::Value {
        json!({
            "enabled": self.preupload_enabled,
            "supported": self.announces(FEATURE_TEXTURE_PREUPLOAD),
            "sent": self.stats.preuploaded,
        })
    }

    /// Turns the warm off (or on again): the monitor then never sends `frame.warm`, whatever the runtime
    /// announces. The same build with and without it is how its effect is measured
    /// (`scripts/scene-warm-proof.py`); a cut is then drawn exactly as before BL157.
    pub fn set_warm(&mut self, enabled: bool) {
        self.warm_enabled = enabled;
        if !enabled {
            self.warmed.clear();
        }
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
            let mut keep: HashSet<Id> = required.iter().copied().collect();
            keep.extend(self.prefetched.keys().copied());
            self.release_except(&keep);
        }
        // From here on a prefetched asset that this frame needs is an ordinary one.
        self.prefetched.retain(|asset, _| !required.contains(asset));
        // Explicit requests keep today's behaviour byte for byte; only the monitor's own
        // frames may skip the readback, and only when nothing asked for pixels.
        let monitor = origin != Origin::Explicit;
        let inspect =
            !monitor || self.inspect_previews || output.png.is_some() || !probes.is_empty();
        // Only the monitor's own interactive frame (playback, scrub) may be a reduced picture (a
        // proxy, scene MSAA below the final count); anything that asks for pixels or is not that
        // frame (explicit, export, refine) is the original.
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
                    self.asset_bytes.insert(
                        *asset,
                        memory_bytes(descriptor.kind, files_bytes(&descriptor.files())),
                    );
                    self.assets.insert(*asset, Slot::Loading);
                    self.loaded_from.insert(*asset, decoded_key(state, asset));
                    pending.push(*asset);
                }
            }
        }
        for asset in &required {
            self.last_used.insert(*asset, id);
        }
        self.release_over_budget(&required);
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
                self.prefetched.clear();
                self.warmed.clear();
                self.preuploaded.clear();
                let error = BackendError::new("context_lost", "The graphics context was lost");
                SessionEvent::Failed(self.fail(error))
            }
            BackendEvent::ContextRestored => {
                self.needs_redraw = true;
                SessionEvent::Progress
            }
            BackendEvent::Closed(reason) => {
                self.assets.clear();
                self.prefetched.clear();
                self.warmed.clear();
                self.preuploaded.clear();
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
            // The answer of a load a release overtook (the runtime reports `cancelled` for it): a load this session gave
            // up on, or replaced, is not the one now in flight for the asset, whose slot and request it must not touch.
            AssetStatus::Failed(error) if error.code == "cancelled" => SessionEvent::Progress,
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
        // A reduced picture is on screen (a proxy, or a scene with fewer MSAA samples): remember how
        // to replace it with the final-quality one. Only the monitor's own interactive frames can
        // be reduced; an explicit frame and the refine itself are the final quality by definition.
        self.refine = finished.as_ref().and_then(|active| {
            (active.origin == Origin::Interactive && frame.detail.is_reduced()).then(|| Pending {
                plan: active.request.plan.clone(),
                output: active.request.output.clone(),
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

    /// [`cancel`](Self::cancel) for a request that ran out of time. A load it was still waiting for is given up
    /// too: the runtime is told to release the asset (which ends that load) and the session forgets it, so the
    /// next request for the frame sends a new `asset.load` instead of waiting again for one that may never
    /// answer. The runtime's `cancelled` for the load that was given up is ignored (see `on_asset`), so it
    /// cannot be taken for the answer of the new one. Loads that were not part of the request (a prefetch) and
    /// assets that finished loading are left alone.
    pub fn cancel_timed_out(&mut self) {
        let stuck: Vec<Id> = self
            .active
            .as_ref()
            .map(|active| {
                active
                    .required
                    .iter()
                    .filter(|asset| matches!(self.assets.get(*asset), Some(Slot::Loading)))
                    .copied()
                    .collect()
            })
            .unwrap_or_default();
        self.cancel();
        for asset in stuck {
            let _ = self.backend.release_asset(asset);
            self.forget_asset(&asset);
            self.stats.reloaded += 1;
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

    /// Releases the least recently used heavy assets until the bytes of the loaded ones fit the
    /// budget. What `required` names or a prefetch holds for the next cut is never released.
    fn release_over_budget(&mut self, required: &[Id]) {
        let Some(limit) = self.retain_bytes else {
            return;
        };
        let mut total: u64 = self
            .assets
            .keys()
            .map(|id| self.asset_bytes.get(id).copied().unwrap_or(0))
            .sum();
        if total <= limit {
            return;
        }
        let mut candidates: Vec<(u64, Id)> = self
            .assets
            .keys()
            .filter(|id| !required.contains(*id) && !self.prefetched.contains_key(*id))
            .filter(|id| self.asset_bytes.get(*id).copied().unwrap_or(0) >= self.min_evict_bytes)
            .map(|id| (self.last_used.get(id).copied().unwrap_or(0), *id))
            .collect();
        candidates.sort();
        for (_, id) in candidates {
            if total <= limit {
                break;
            }
            total = total.saturating_sub(self.asset_bytes.get(&id).copied().unwrap_or(0));
            let _ = self.backend.release_asset(id);
            self.forget_asset(&id);
            self.stats.evicted += 1;
        }
    }

    fn forget_asset(&mut self, id: &Id) {
        self.warmed.retain(|key| !key.contains(id));
        self.preuploaded.retain(|key| !key.contains(id));
        self.assets.remove(id);
        self.proxies.remove(id);
        self.refused.remove(id);
        self.prefetched.remove(id);
        self.asset_bytes.remove(id);
        self.last_used.remove(id);
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
            self.forget_asset(&id);
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
        self.prefetched.clear();
        self.warmed.clear();
        self.preuploaded.clear();
        self.asset_bytes.clear();
        self.last_used.clear();
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
                "prefetched": self.stats.prefetched, "prefetch_skipped": self.stats.prefetch_skipped, "evicted": self.stats.evicted, "warmed": self.stats.warmed, "reloaded": self.stats.reloaded,
            },
            "preupload": self.preupload_report(),
            "warm_draw": json!({"enabled": self.warm_draw_enabled, "supported": self.announces(FEATURE_SCENE_WARM_DRAW)}),
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
/// Bytes of the source files an asset is loaded from.
fn files_bytes(files: &[String]) -> u64 {
    files
        .iter()
        .filter_map(|file| std::fs::metadata(file).ok())
        .map(|meta| meta.len())
        .sum()
}

/// What a file of this size keeps in memory once loaded, for the byte budget: stills, scenes and
/// splats are decoded whole into the runtime; video and audio stream from disk or from frame
/// files read on demand, under their own cache limits, so their file size says nothing.
fn memory_bytes(kind: MediaKind, file_bytes: u64) -> u64 {
    match kind {
        MediaKind::Video | MediaKind::Audio | MediaKind::Timeline => 0,
        _ => file_bytes,
    }
}

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
