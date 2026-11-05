//! A headless renderer for directed-scene plans: the [`ScenePlan`] the core evaluates, drawn by
//! the Three.js runtime behind any bridge transport (BL007/BL063).
//!
//! It owns no domain logic. The core decides the frame ([`ScenePlan::build`]); an adapter decides
//! which approved local file holds each resource ([`SceneResourceMap`]); this type only moves them
//! to the runtime in the order the runtime needs (`asset.release`, `asset.load`, wait for the
//! pictures, `frame.render`), waits for the answer, and survives what the bridge can do to it:
//! a seek that supersedes a running frame, a lost graphics context, a runtime that went away.
//!
//! The same messages drive the desktop monitor: [`SceneSlots::prepare`] and
//! [`frame_render_message`] are the seam, this type is their synchronous host.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use tessel_editor_core::backend::{
    AssetStatus, BackendError, BackendEvent, Capabilities, FrameResult, RenderBackend, RequestId,
};
use tessel_editor_core::bridge::{BridgeBackend, Transport};
use tessel_editor_core::scene_plan::{Capability, ScenePlan};

use crate::scene::{
    DIRECTED_FEATURE, SceneFrameMessage, SceneResourceMap, SceneSlots, frame_render_message,
    plan_error,
};

/// How long the renderer waits for one answer (a picture decoding, a frame drawing).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// A frame that fails with `context_lost` is sent again once the context is back, this many times.
pub const CONTEXT_RETRIES: usize = 2;

/// What one frame asks for besides the plan.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameOptions {
    /// Canvas size. `None` is the plan's output size.
    pub size: Option<(u32, u32)>,
    /// Where the runtime writes the PNG (the Node worker does; a WebView host ignores it).
    pub png: Option<PathBuf>,
    /// Pixels (x, y from the top left) to read back and report.
    pub probes: Vec<(u32, u32)>,
    /// Read the picture back for a hash and the mean. `false` is the cheap interactive path.
    pub inspect: bool,
}

impl Default for FrameOptions {
    fn default() -> Self {
        Self {
            size: None,
            png: None,
            probes: Vec::new(),
            inspect: true,
        }
    }
}

/// A frame the runtime has been asked to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubmittedFrame {
    pub id: RequestId,
    pub revision: u64,
}

pub struct SceneRenderer<T: Transport> {
    backend: BridgeBackend<T>,
    slots: SceneSlots,
    capabilities: Option<Capabilities>,
    /// What the runtime says a `directed_scene` layer can carry (from `ready`).
    supported: Vec<Capability>,
    next_id: RequestId,
    next_revision: u64,
    /// Answers that arrived while the renderer waited for something else, by frame id.
    finished: BTreeMap<RequestId, Result<FrameResult, BackendError>>,
    /// Loads the runtime refused, by slot. Taken by the wait that needs the slot.
    load_failures: BTreeMap<String, BackendError>,
    context_lost: bool,
    timeout: Duration,
}

impl<T: Transport> SceneRenderer<T> {
    pub fn new(backend: BridgeBackend<T>) -> Self {
        Self::with_slots(backend, SceneSlots::default())
    }

    pub fn with_slots(backend: BridgeBackend<T>, slots: SceneSlots) -> Self {
        Self {
            backend,
            slots,
            capabilities: None,
            supported: Vec::new(),
            next_id: 1,
            next_revision: 1,
            finished: BTreeMap::new(),
            load_failures: BTreeMap::new(),
            context_lost: false,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn slots(&self) -> &SceneSlots {
        &self.slots
    }

    pub fn capabilities(&self) -> Option<&Capabilities> {
        self.capabilities.as_ref()
    }

    /// Starts the handshake if it has not happened and checks the runtime can draw scenes.
    /// A runtime built before this feature (no `directed_scene` in `features`) is refused here
    /// with the command that rebuilds it, rather than with a protocol error on the first frame.
    pub fn ensure_ready(&mut self) -> Result<&Capabilities, BackendError> {
        if self.capabilities.is_none() {
            let capabilities = self.backend.capabilities()?;
            if !capabilities.supports(DIRECTED_FEATURE) {
                return Err(BackendError::new(
                    "backend_unavailable",
                    "The runtime does not draw directed scenes. Rebuild it: pnpm --dir runtime run build",
                ));
            }
            self.supported = capabilities.raw["capabilities"]["directed_scene"]["capabilities"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|name| name.as_str().and_then(Capability::parse))
                .collect();
            self.capabilities = Some(capabilities);
        }
        self.capabilities
            .as_ref()
            .ok_or_else(|| BackendError::new("backend_unavailable", "The runtime is not ready"))
    }

    /// Releases, loads and waits for the pictures `plan` needs, then asks for the frame. Returns
    /// as soon as the runtime has the request; [`wait`](Self::wait) collects the answer. A later
    /// `submit` supersedes this frame in the runtime (latest frame wins), and `wait` then reports
    /// `cancelled` for it.
    pub fn submit(
        &mut self,
        plan: &ScenePlan,
        resources: &SceneResourceMap,
        options: &FrameOptions,
    ) -> Result<SubmittedFrame, BackendError> {
        self.ensure_ready()?;
        plan.require_capabilities(&self.supported)
            .map_err(|failure| plan_error(&failure))?;
        let prepared = self.slots.prepare(plan, resources)?;
        for release in &prepared.releases {
            self.backend.transport_mut().send(release)?;
        }
        for load in &prepared.loads {
            self.backend.transport_mut().send(&load.message)?;
        }
        let pending: Vec<String> = prepared
            .pending_slots()
            .into_iter()
            .map(str::to_string)
            .collect();
        self.wait_for_pictures(&pending)?;

        let (width, height) = options
            .size
            .unwrap_or((plan.output.width as u32, plan.output.height as u32));
        let frame = SubmittedFrame {
            id: self.next_id,
            revision: self.next_revision,
        };
        self.next_id += 1;
        self.next_revision += 1;
        self.backend
            .transport_mut()
            .send(&frame_render_message(&SceneFrameMessage {
                id: frame.id,
                revision: frame.revision,
                edit_frame: plan.frame.edit_frame,
                width,
                height,
                layer: prepared.layer,
                png: options
                    .png
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned()),
                probes: options.probes.clone(),
                inspect: options.inspect,
            }))?;
        Ok(frame)
    }

    /// The runtime's answer to `frame`: the frame, or the error it failed with (`cancelled` when a
    /// later frame took its place, `context_lost` when the graphics context went away).
    pub fn wait(&mut self, frame: SubmittedFrame) -> Result<FrameResult, BackendError> {
        let deadline = Instant::now() + self.timeout;
        loop {
            if let Some(answer) = self.finished.remove(&frame.id) {
                return answer;
            }
            self.pump(deadline, "the frame")?;
        }
    }

    /// Tells the runtime to drop `frame` if it has not started. Its `wait` then answers `cancelled`.
    pub fn cancel(&mut self, frame: SubmittedFrame) -> Result<(), BackendError> {
        self.backend.cancel(frame.id)
    }

    /// `submit` + `wait`, sending the frame again when the context was lost mid-way. A frame that
    /// was cancelled, or any other failure, is returned as it is: retrying would hide it.
    pub fn render(
        &mut self,
        plan: &ScenePlan,
        resources: &SceneResourceMap,
        options: &FrameOptions,
    ) -> Result<FrameResult, BackendError> {
        let mut attempt = 0;
        loop {
            let frame = self.submit(plan, resources, options)?;
            match self.wait(frame) {
                Err(error) if error.code == "context_lost" && attempt < CONTEXT_RETRIES => {
                    attempt += 1;
                    self.wait_for_context()?;
                }
                answer => return answer,
            }
        }
    }

    /// The project closed: releases every picture and forgets them.
    pub fn release_all(&mut self) -> Result<(), BackendError> {
        for release in self.slots.release_all() {
            self.backend.transport_mut().send(&release)?;
        }
        Ok(())
    }

    pub fn shutdown(&mut self) {
        self.backend.shutdown();
        self.slots.reset();
        self.capabilities = None;
        self.supported.clear();
        self.finished.clear();
        self.load_failures.clear();
        self.context_lost = false;
    }

    fn wait_for_pictures(&mut self, slots: &[String]) -> Result<(), BackendError> {
        let deadline = Instant::now() + self.timeout;
        loop {
            if let Some(failure) = slots
                .iter()
                .find_map(|slot| self.load_failures.remove(slot))
            {
                return Err(failure);
            }
            if slots.iter().all(|slot| self.slots.is_ready(slot)) {
                return Ok(());
            }
            self.pump(deadline, "the pictures")?;
        }
    }

    fn wait_for_context(&mut self) -> Result<(), BackendError> {
        let deadline = Instant::now() + self.timeout;
        while self.context_lost {
            self.pump(deadline, "the graphics context")?;
        }
        Ok(())
    }

    /// Reads one runtime message and applies it to the book.
    fn pump(&mut self, deadline: Instant, waiting_for: &str) -> Result<(), BackendError> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let Some(event) = self.backend.poll(remaining) else {
            return Err(BackendError::new(
                "timeout",
                format!(
                    "The runtime did not answer in {:?} while waiting for {waiting_for}",
                    self.timeout
                ),
            ));
        };
        self.absorb(event)
    }

    fn absorb(&mut self, event: BackendEvent) -> Result<(), BackendError> {
        match event {
            BackendEvent::Asset {
                id,
                status: AssetStatus::Ready(_),
            } => self.slots.mark_ready(id.as_str()),
            BackendEvent::Asset {
                id,
                status: AssetStatus::Failed(error),
            } => {
                self.slots.mark_failed(id.as_str());
                self.load_failures.insert(id.as_str().to_string(), error);
            }
            BackendEvent::FrameDone(result) => {
                self.finished.insert(result.id, Ok(result));
            }
            BackendEvent::FrameFailed { id, error, .. } => {
                self.finished.insert(id, Err(error));
            }
            BackendEvent::ContextLost => self.context_lost = true,
            BackendEvent::ContextRestored => self.context_lost = false,
            BackendEvent::Closed(reason) => {
                // Everything the runtime held is gone with it; the next request loads again.
                self.slots.reset();
                self.capabilities = None;
                self.supported.clear();
                self.context_lost = false;
                return Err(BackendError::new("backend_unavailable", reason));
            }
            BackendEvent::Asset { .. }
            | BackendEvent::Proxy { .. }
            | BackendEvent::FramePresented { .. }
            | BackendEvent::Ready(_) => {}
        }
        Ok(())
    }
}
