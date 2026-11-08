//! The operations of the media adapters, exposed through the core's [`Extension`] hook.
//!
//! One `MediaHost` per session serves the JSON Lines CLIs (`./scripts/headless`,
//! `./scripts/render`) and the desktop `ui op` channel, so a gesture, a script and a library
//! call reach the same code. Heavy work runs as jobs; operations that change the project
//! prepare their files on the job thread and apply the result to the project in one
//! transaction on the caller's thread (`wait: true`, the default, does both in one call;
//! `wait: false` returns a job ID and `job.commit` applies it when the job succeeded).
//!
//! Operations: `media.import|relink|search|status|prepare|thumbnail|thumbnails|waveform|proxy`,
//! `project.package_preflight|package|verify|restore|import|import_scenes`,
//! `scene.attach.start` (F043: read/validate a bundle on a job, attach it as one undo step),
//! `scene.resources.status|preflight|resolve|adopt|release|invalidate|claims|recover` (F040),
//! `export.preflight|start`, `scene.export[.preflight]` (F042: a directed scene as video, same jobs),
//! `job.list|get|wait|cancel|commit|discard`. Presets (`preset.*`) belong to the core.
//!
//! A finished result nobody applied is discarded with `job.discard {job}` (BL094): a `discard` job
//! takes back the files the result owns, off the caller's thread, and the result keeps owning them
//! (and `prune` keeps its record) until that job ends. An active, applied or already discarded job is
//! refused with a typed error (`job_running`, `job_applied`, `job_discarding`, `job_discarded`,
//! `job_nothing_to_discard`). A window never hashes or deletes on its own thread: a stale result found
//! by [`MediaHost::poll`] and a host dropped with results pending are discarded by such jobs.
//!
//! Anything that reads files, runs a tool or builds a renderer (probe, decode, verify, restore,
//! preflight) is a job; the caller's thread only snapshots the project and later applies the
//! result. A result is applied only if the project it was computed for is still the open one
//! (same ID, editing session and revision for whole-project swaps and scene attach, same content hash per asset for decoded
//! media), so a slow job can never overwrite what the user did meanwhile.
//!
//! Media is prepared automatically: export and the render CLIs decode what they need, and an
//! interactive host schedules a "Prepare media" job (`MediaHost::poll`) for every asset that lacks
//! decoded frames/audio, so nobody has to know the `media.prepare` command.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tessel_editor_core::assets::{
    Asset, Decoded, DecodedFrames, MediaKind, ProxyFormat, ProxyFrames,
};
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::headless::{Extension, open_summary};
use tessel_editor_core::interchange::apply_import;
use tessel_editor_core::jobs::{
    JobContext, JobError, JobId, JobInfo, JobKind, JobRegistry, JobState, ResultState,
};
use tessel_editor_core::preset::{self, Preset};
use tessel_editor_core::preview::{PreviewPolicy, Reason};
use tessel_editor_core::scene_export::{SceneExportSnapshot, SceneExportSpec};
use tessel_editor_core::scene_project::{AdmittedScene, SceneCommand, SceneSource};
use tessel_editor_core::scene_resources::ResourceManifest;
use tessel_editor_core::session::Renderer;
use tessel_editor_core::state::{Command, ProjectStamp};
use tessel_editor_core::thumbnails::Stamp as ThumbStamp;
use tessel_editor_core::{CommandError, EditorState};

use crate::derive::Cache;
use crate::directed_resources::Options as ResourceOptions;
use crate::export::{self, ExportRequest};
use crate::import::{self, ImportMode, ImportRequest, PreparedItem};
use crate::interchange::{self, PreparedProjectImport};
use crate::package::{self, DOCUMENT_NAME, PackageFormat, PackageInputs, PackageOptions};
use crate::scene_encoder::{self, SceneExport, SceneRendererFactory};
use crate::scene_resources::import::{self as scene_import, PreparedSceneImport};
use crate::scene_resources::{
    AdoptionTarget, Invalidation, LoadRequest, PendingAdoption, ResolvedResourceMap, ResourceBase,
    SceneResourceSession,
};
use crate::thumbnails::{self as thumbs, Made};
use crate::tools::Tools;

/// Builds the renderer one export drives. It runs on the export's own thread, so a session that
/// is not `Send` can still be used, and an export never shares GPU state with the live monitor.
pub type RendererFactory = dyn Fn() -> Result<Box<dyn Renderer>, JobError> + Send + Sync;

/// Result a job produced for the project, applied by `job.commit` (or at once with `wait`).
enum Pending {
    Import {
        items: Vec<PreparedItem>,
        folder: Option<String>,
        prepare: bool,
    },
    Relink(Vec<(String, Asset)>),
    ProjectImport(Box<PreparedProjectImport>),
    /// Decoded media for the project `project`; each item applies only while its asset still has
    /// the content hash it was decoded from.
    Decoded {
        project: String,
        items: Vec<DecodedItem>,
    },
    /// Preview proxies for the project `project`; each applies only while its asset still has
    /// the content hash it was derived from (and still draws from the same decoded frames).
    Proxies {
        project: String,
        items: Vec<(String, Option<String>, ProxyFrames)>,
    },
    /// Approved resources of one scene, installed in the session only while the project, the
    /// generation and the scene's bundle revision are still the ones the job was started for.
    SceneResources {
        map: Box<ResolvedResourceMap>,
        session: SceneResourceSession,
    },
    /// Scenes whose files are already in the workspace, attached as one undo step.
    SceneImport(Box<PreparedSceneImport>),
    /// A bundle read, imported and admitted off the caller's thread (BL100), attached as one undo
    /// step only while the open project is still the exact `stamp` it was started for: same ID,
    /// same editing session (BL113a: reopening a document is a new session even with the same ID
    /// and revision 0) and same revision. It owns no files.
    SceneAttach {
        stamp: ProjectStamp,
        id: Option<String>,
        name: Option<String>,
        scene: AdmittedScene,
        /// What loading its resources will need, computed with the scene on the job thread.
        resources: Value,
    },
    /// Files an adoption copied, kept only while the project, its workspace and the scene's
    /// bundle are still the ones the job was started for; otherwise removed (BL089). Changes no
    /// project data. Dropped unapplied (host gone, job never committed) it removes them too,
    /// except what another pending or accepted result relies on (BL096).
    SceneAdopt(Box<PendingAdoption>),
    /// A restored project, already loaded and validated off the caller's thread, that replaces the
    /// open project only if that one is still the exact `stamp` the restore was started for (same
    /// guard as [`Pending::SceneAttach`]).
    Open {
        stamp: ProjectStamp,
        candidate: Box<EditorState>,
        summary: Value,
    },
    /// Thumbnails of splats and scenes drawn for the project (and editing session) `stamp`
    /// names; each applies only while its asset still has the identity it was drawn from.
    Thumbnails {
        stamp: ThumbStamp,
        made: Vec<Made>,
    },
}

impl Pending {
    fn label(&self) -> &'static str {
        match self {
            Self::Import { .. } => "import",
            Self::Relink(_) => "relink",
            Self::ProjectImport(_) => "project-import",
            Self::Decoded { .. } => "decoded-media",
            Self::Proxies { .. } => "preview-proxies",
            Self::SceneResources { .. } => "scene-resources",
            Self::SceneImport(_) => "scene-import",
            Self::SceneAttach { .. } => "scene-attach",
            Self::SceneAdopt(_) => "scene-adopt",
            Self::Open { .. } => "restored-project",
            Self::Thumbnails { .. } => "thumbnails",
        }
    }

    /// Whether dropping this result removes files from disk (hashing each first): the work that
    /// must never run on a window's thread.
    fn owns_files(&self) -> bool {
        matches!(self, Self::SceneImport(_) | Self::SceneAdopt(_))
    }

    /// Gives the result up. A result that owns files takes back exactly those (BL089, BL096,
    /// BL097) and says what it kept; the others only hold memory. The result keeps its claims in
    /// the ledger until this has finished, so another process sees a live holder (BL111). Blocks
    /// for as long as the hashing takes, so it runs on a discard job.
    fn discard(self) -> Value {
        let kind = self.label();
        let cleanup = match self {
            Self::SceneImport(prepared) => Some(prepared.discard().to_json()),
            Self::SceneAdopt(pending) => Some(pending.discard().to_json()),
            other => {
                drop(other);
                None
            }
        };
        json!({ "discarded": kind, "cleanup": cleanup })
    }
}

/// Marks the result of `original` as discarded when the discard job ends, however it ends (a panic
/// while removing included): by then the result has dropped everything it owned.
struct DiscardGuard {
    registry: JobRegistry,
    original: JobId,
}

impl Drop for DiscardGuard {
    fn drop(&mut self) {
        self.registry.finish_discard(&self.original);
    }
}

/// Starts the job that removes `pending` for `original`, which must already be `discarding`.
/// The result moves into the job: it keeps owning its files until the job has taken them back.
fn spawn_discard(registry: &JobRegistry, original: &str, pending: Option<Pending>) -> JobId {
    let guard = DiscardGuard {
        registry: registry.clone(),
        original: original.to_string(),
    };
    let original = original.to_string();
    registry.spawn(
        JobKind::Discard,
        &format!("Discard the result of {original}"),
        move |ctx| {
            let guard = guard;
            ctx.stage("cleanup", 1);
            let mut report = pending.map_or_else(
                || json!({ "discarded": null, "cleanup": null }),
                Pending::discard,
            );
            ctx.advance(1);
            report["action"] = json!("job.discard");
            report["discards"] = json!(original);
            drop(guard);
            Ok(report)
        },
    )
}

/// FFmpeg discovery runs a subprocess, so it happens on a job thread the first time and is shared.
#[derive(Clone, Default)]
struct ToolCell(Arc<Mutex<Option<Tools>>>);

impl ToolCell {
    fn get(&self) -> Result<Tools, JobError> {
        let mut cell = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(tools) = &*cell {
            return Ok(tools.clone());
        }
        let tools = Tools::discover()?;
        *cell = Some(tools.clone());
        Ok(tools)
    }
}

type Slot = Arc<Mutex<Option<Pending>>>;

/// `(asset id, content hash it was decoded from, the decoded media)`: the hash is what lets a
/// result be discarded when the file changed while the job ran.
type DecodedItem = (String, Option<String>, Option<Decoded>);

/// The project an automatic prepare job was started for, and the `(asset, hash)` pairs it covers.
type AutoAssets = (String, Vec<(String, Option<String>)>);

pub struct MediaHost {
    tools: ToolCell,
    registry: JobRegistry,
    renderer: Option<Arc<RendererFactory>>,
    /// Builds the renderer a directed-scene export drives (`scene.export`); `None` answers
    /// `backend_unavailable`.
    scene_renderer: Option<Arc<SceneRendererFactory>>,
    pending: HashMap<JobId, Slot>,
    /// A host that serves a window: no operation blocks its caller (the UI thread), and
    /// finished jobs are applied by [`MediaHost::poll`] instead of `wait`/`job.commit`.
    interactive: bool,
    /// The automatic "Prepare media" job in flight, if any.
    auto_prepare: Option<JobId>,
    /// `(asset, hash)` whose automatic preparation failed: not retried until the file changes,
    /// so a corrupt video does not restart a job every tick.
    auto_failed: Vec<(String, Option<String>)>,
    /// Assets each automatic job was started for, to remember them if it fails.
    auto_assets: HashMap<JobId, AutoAssets>,
    /// The automatic "Build preview proxy" job in flight, if any.
    auto_proxy: Option<JobId>,
    /// The thumbnail job in flight and the project it draws for (BL144). One at a time: it owns
    /// a renderer. A job whose project is no longer the open one is cancelled by `poll`.
    thumb_job: Option<(JobId, ThumbStamp)>,
    /// The project whose automatic thumbnail job the user cancelled: it is not started again by
    /// the tick (that would undo the cancel in a loop); `media.thumbnails` asks for it explicitly,
    /// and another project or another opening of it starts fresh.
    thumb_paused: Option<ThumbStamp>,
    /// `(asset, hash, height, format)` whose proxy failed: not retried until one of them changes.
    proxy_failed: Vec<ProxyKey>,
    proxy_assets: HashMap<JobId, Vec<ProxyKey>>,
    /// Approved scene resources of the open project (BL010). Shared with jobs and, through
    /// [`MediaHost::scene_resources`], with a renderer.
    scene_resources: SceneResourceSession,
    /// Discard jobs [`MediaHost::poll`] already told the window about.
    reported_discards: HashSet<JobId>,
    /// `(original, discard job)` for a result discarded **before** [`MediaHost::poll`] had
    /// reported its job. The slot of such a job leaves `pending` with the discard, so the tick
    /// would never say how it ended: whoever waits for that job (a chain of jobs, a dialog's
    /// report) would wait forever. `poll` reports each once, as a failure `job_discarded`.
    discarded_unreported: Vec<(JobId, JobId)>,
    /// The event of a job whose result `job.commit` applied (or refused) **before**
    /// [`MediaHost::poll`] had seen the job finish (BL094.10). `job.commit` takes the slot out of
    /// `pending`, so the tick would never report the job: a chain or a dialog waiting for it would
    /// wait forever, exactly like a discard before the tick. Interactive hosts only (a host without
    /// a window has no tick); `poll` delivers each once, shaped like its own events.
    committed_unreported: Vec<Value>,
}

type ProxyKey = (String, Option<String>, u32, ProxyFormat);

impl Default for MediaHost {
    fn default() -> Self {
        Self::new()
    }
}

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new("invalid_request", message)
}

fn text<'a>(request: &'a Value, key: &str) -> Result<&'a str, CommandError> {
    request
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("Expected string field `{key}`")))
}

fn optional_text<'a>(request: &'a Value, key: &str) -> Result<Option<&'a str>, CommandError> {
    match request.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or_else(|| invalid(format!("`{key}` must be a string"))),
    }
}

fn flag(request: &Value, key: &str, default: bool) -> bool {
    request.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn strings(request: &Value, key: &str) -> Result<Option<Vec<String>>, CommandError> {
    let Some(value) = request.get(key).filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let list = value
        .as_array()
        .ok_or_else(|| invalid(format!("`{key}` must be an array of strings")))?;
    list.iter()
        .map(|entry| {
            entry
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| invalid(format!("`{key}` must be an array of strings")))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn workspace(state: &EditorState) -> Result<PathBuf, CommandError> {
    state.project.workspace.clone().ok_or_else(|| {
        CommandError::new(
            "no_workspace",
            "The project has no workspace yet: save it first (`project.save`)",
        )
    })
}

fn asset_of(state: &EditorState, id: &str) -> Result<(MediaKind, Asset), CommandError> {
    let item = state
        .library
        .item(id)
        .ok_or_else(|| CommandError::unknown_element("asset", id))?;
    let asset = item.asset.clone().ok_or_else(|| {
        CommandError::new(
            "unsupported_format",
            format!("`{id}` has no file to work on"),
        )
    })?;
    Ok((item.kind, asset))
}

impl MediaHost {
    pub fn new() -> Self {
        Self {
            tools: ToolCell::default(),
            registry: JobRegistry::new(2),
            renderer: None,
            scene_renderer: None,
            thumb_job: None,
            thumb_paused: None,
            pending: HashMap::new(),
            interactive: false,
            auto_prepare: None,
            auto_failed: Vec::new(),
            auto_assets: HashMap::new(),
            auto_proxy: None,
            proxy_failed: Vec::new(),
            proxy_assets: HashMap::new(),
            scene_resources: SceneResourceSession::default(),
            reported_discards: HashSet::new(),
            discarded_unreported: Vec::new(),
            committed_unreported: Vec::new(),
        }
    }

    /// The session that owns the approved resources of the scenes. Hand a clone to a renderer
    /// as its `ResourceResolver`; the host invalidates it when the project changes.
    pub fn scene_resources(&self) -> SceneResourceSession {
        self.scene_resources.clone()
    }

    /// Uses another session (a shared one, or one with a different byte budget).
    pub fn with_scene_resources(mut self, session: SceneResourceSession) -> Self {
        self.scene_resources = session;
        self
    }

    /// Lets `export.start` build a renderer; without one exports answer `backend_unavailable`.
    pub fn with_renderer(mut self, factory: Arc<RendererFactory>) -> Self {
        self.renderer = Some(factory);
        self
    }

    /// Lets `scene.export` build a renderer for directed scenes; without one it answers
    /// `backend_unavailable`. Like `with_renderer`, each export builds its own on its own thread.
    pub fn with_scene_renderer(mut self, factory: Arc<SceneRendererFactory>) -> Self {
        self.scene_renderer = Some(factory);
        self
    }

    /// Uses these tools instead of discovering them: an explicit configuration, and the seam
    /// the tests use to run the host against deliberately slow or failing tool wrappers.
    pub fn with_tools(self, tools: Tools) -> Self {
        *self
            .tools
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(tools);
        self
    }

    /// For the desktop: operations always answer with a job ID at once (`wait` is ignored,
    /// `job.wait` reports the current state) and `poll` commits what finished.
    pub fn interactive(mut self) -> Self {
        self.interactive = true;
        self
    }

    /// Applies the result of every finished job, in the order they finished, each as one
    /// transaction on the caller's thread, and reports what happened to each. A job that failed
    /// or was cancelled changed nothing and is reported once. Call it from the UI thread's tick.
    pub fn poll(&mut self, state: &mut EditorState) -> Vec<Value> {
        self.scene_resources.observe_project(&state.project.id);
        self.cancel_stale_thumbnails(state);
        let finished: Vec<JobId> = self
            .pending
            .keys()
            .filter(|id| {
                self.registry
                    .get(id)
                    .is_some_and(|info| info.state.is_final())
            })
            .cloned()
            .collect();
        let mut events = Vec::new();
        for id in finished {
            let Some(info) = self.registry.get(&id) else {
                continue;
            };
            let slot = self.pending.remove(&id);
            if self.auto_prepare.as_deref() == Some(id.as_str()) {
                self.auto_prepare = None;
            }
            if self.auto_proxy.as_deref() == Some(id.as_str()) {
                self.auto_proxy = None;
            }
            self.release_thumbnail_job(state, &id, info.state == JobState::Cancelled);
            let proxied = self.proxy_assets.remove(&id);
            let automatic = self.auto_assets.remove(&id);
            let outcome = Self::outcome(&info).and_then(|result| {
                let applied = apply(
                    &self.registry.clone(),
                    &id,
                    slot.as_ref().and_then(take),
                    state,
                    false,
                )?;
                Ok((result, applied))
            });
            if let (Err(_), Some((_, assets))) = (&outcome, automatic) {
                // Not retried until the file changes (see `auto_failed`).
                self.auto_failed.extend(assets);
            }
            if let (Err(_), Some(keys)) = (&outcome, proxied) {
                self.proxy_failed.extend(keys);
            }
            events.push(match outcome {
                Ok((result, applied)) => json!({ "job": id, "state": "succeeded", "kind": info.kind.label(), "result": result, "applied": applied }),
                // A job that succeeded but whose result no longer fits the project (`stale_result`)
                // is reported as failed: nothing was applied. `error` is the bare
                // `{code, message}` so a window reads it the same for every kind of failure.
                Err(error) => {
                    let state = if info.state == JobState::Succeeded {
                        "failed"
                    } else {
                        info.state.label()
                    };
                    json!({ "job": id, "state": state, "kind": info.kind.label(), "error": error.to_json()["error"] })
                }
            });
        }
        events.extend(self.discarded_before_report());
        events.extend(std::mem::take(&mut self.committed_unreported));
        events.extend(self.finished_discards());
        self.schedule_prepare(state);
        self.schedule_proxies(state);
        self.schedule_thumbnails(state);
        events
    }

    /// Frees the thumbnail slot if `id` was its job: the job is over, and so is its claim on the one
    /// renderer. A job the user cancelled, for the project that is open, leaves that project's
    /// automatic drawing paused. `poll` calls it when it sees the job finish; `job.commit` and
    /// `job.discard` call it too, because a result taken by a request is never seen by `poll`.
    fn release_thumbnail_job(&mut self, state: &EditorState, id: &str, cancelled: bool) {
        if let Some((_, stamp)) = self.thumb_job.take_if(|(job, _)| job == id)
            && cancelled
            && stamp.is_current(state)
        {
            self.thumb_paused = Some(stamp);
        }
    }

    /// Cancels the thumbnail job of a project that is no longer the open one (another project,
    /// or the same document opened again). The job notices between targets and applies nothing;
    /// even a result that slips through is refused by its stamp.
    fn cancel_stale_thumbnails(&mut self, state: &EditorState) {
        if let Some((job, stamp)) = &self.thumb_job
            && !stamp.is_current(state)
        {
            let _ = self.registry.cancel(job);
        }
    }

    /// Whether a video export is running: it owns a renderer and the GPU, and a thumbnail that
    /// nobody asked for does not compete with it (it starts when the export is over).
    fn export_running(&self) -> bool {
        !self
            .registry
            .matching(|info| info.kind == JobKind::VideoExport && !info.state.is_final())
            .is_empty()
    }

    /// Starts the one thumbnail job the open project needs (splats and scenes with no picture
    /// yet), after the media is prepared: the renderer is the heavy part and runs alone. Each
    /// tick this costs the identities of the splats and scenes (model lookups, no clone); the
    /// project is copied only when a job starts, and then only the assets it draws.
    fn schedule_thumbnails(&mut self, state: &EditorState) {
        if !self.interactive
            || self.thumb_job.is_some()
            || self.auto_prepare.is_some()
            || self.auto_proxy.is_some()
            || state.project.workspace.is_none()
            || self
                .thumb_paused
                .as_ref()
                .is_some_and(|stamp| stamp.is_current(state))
            || self.export_running()
        {
            return;
        }
        let targets = state.thumbnail_targets(false);
        if targets.is_empty() {
            return;
        }
        let _ = self.thumbnail_job(state, targets, "Draw library thumbnails");
    }

    /// The job behind every thumbnail run, explicit or automatic.
    fn thumbnail_job(
        &mut self,
        state: &EditorState,
        targets: Vec<tessel_editor_core::thumbnails::Target>,
        label: &str,
    ) -> Result<(JobId, Slot), CommandError> {
        let workspace = workspace(state)?;
        let stamp = state.thumbnail_stamp();
        let job_stamp = stamp.clone();
        let snapshot = state.thumbnail_snapshot(&targets)?;
        let factory = self.renderer.clone();
        let (id, slot) = self.start(JobKind::Thumbnail, label, move |ctx| {
            let cache = Cache::new(&workspace);
            let made = thumbs::make(&snapshot, &targets, &cache, factory, ctx)?;
            let ready = made
                .iter()
                .filter(|made| matches!(made, Made::Ready(..)))
                .count();
            let report = json!({ "action": "media.thumbnails",
                "assets": made.iter().map(Made::asset).collect::<Vec<_>>(),
                "ready": ready, "failed": made.len() - ready });
            Ok((
                Some(Pending::Thumbnails {
                    stamp: job_stamp,
                    made,
                }),
                report,
            ))
        });
        if self.interactive {
            self.thumb_job = Some((id.clone(), stamp));
        }
        Ok((id, slot))
    }

    /// `media.thumbnails {assets?, retry?}`: draws the thumbnails of splats and scenes that have
    /// none (`assets` limits it, `retry` asks again for the ones that failed). A window answers
    /// with the job and `poll` applies it; a CLI waits and applies it before answering.
    fn thumbnails(
        &mut self,
        state: &mut EditorState,
        request: &Value,
    ) -> Result<Value, CommandError> {
        let wanted = strings(request, "assets")?;
        if let Some(wanted) = &wanted {
            for id in wanted {
                if state.library.item(id).is_none() {
                    return Err(CommandError::unknown_element("asset", id));
                }
            }
        }
        if self.export_running() {
            return Err(CommandError::new(
                "renderer_busy",
                "A video export owns the renderer; retry thumbnails after the export finishes",
            )
            .details(json!({ "retryable": true })));
        }
        if let Some((job, stamp)) = &self.thumb_job {
            if stamp.is_current(state) {
                if wanted.is_some()
                    || flag(request, "retry", false)
                    || self
                        .registry
                        .get(job)
                        .is_some_and(|info| info.cancel_requested)
                {
                    return Err(CommandError::new(
                        "thumbnail_busy",
                        "A thumbnail job is still ending or drawing; retry this request after it finishes",
                    )
                    .details(json!({ "retryable": true, "job": job })));
                }
                return Ok(json!({ "job": job, "state": "running", "reused": true }));
            }
            // The job in flight belongs to another project (or another opening of this one): it
            // is not an answer for this one, and two renderers are not started. It is told to
            // stop; it ends at the next frame boundary, within `FRAME_TIMEOUT` of the renderer
            // answering, and the tick then starts this project's own job.
            let _ = self.registry.cancel(job);
            self.thumb_paused = None;
            return Ok(json!({ "action": "media.thumbnails", "state": "waiting",
                "reused": false, "waiting_for": job, "assets": [],
                "reason": "the thumbnail job of another project is still ending; one renderer at a time",
                "deadline_seconds": thumbs::FRAME_TIMEOUT.as_secs() }));
        }
        self.thumb_paused = None;
        let targets: Vec<_> = state
            .thumbnail_targets(flag(request, "retry", false))
            .into_iter()
            .filter(|target| {
                wanted
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&target.asset))
            })
            .collect();
        if targets.is_empty() {
            return Ok(json!({ "action": "media.thumbnails", "assets": [],
                "reason": "no splat or scene needs a thumbnail" }));
        }
        let (id, slot) = self.thumbnail_job(state, targets, "Draw library thumbnails")?;
        self.finish(state, request, id, slot)
    }

    /// One `failed` event (`job_discarded`) for every job whose result was discarded before the
    /// tick reported the job itself, so a chain or a dialog waiting for that job learns it ended.
    /// The discard job has its own event, reported when it ends.
    fn discarded_before_report(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.discarded_unreported)
            .into_iter()
            .filter_map(|(original, discard)| {
                let info = self.registry.get(&original)?;
                let error = CommandError::new(
                    "job_discarded",
                    format!(
                        "The result of job `{original}` was discarded before it was applied; nothing was changed"
                    ),
                )
                .details(json!({ "discard_job": discard }));
                Some(json!({ "job": original, "state": "failed", "kind": info.kind.label(),
                    "error": error.to_json()["error"] }))
            })
            .collect()
    }

    /// One event for every discard job that ended since the last call, so a window learns that the
    /// files of a discarded result are back without watching the job list.
    fn finished_discards(&mut self) -> Vec<Value> {
        let mut events = Vec::new();
        // Only the discard jobs not reported yet are cloned: this runs on every tick of the
        // window, and a finished job carries its whole result.
        let reported = &self.reported_discards;
        let ended = self.registry.matching(|info| {
            info.kind == JobKind::Discard && info.state.is_final() && !reported.contains(&info.id)
        });
        for info in ended {
            self.reported_discards.insert(info.id.clone());
            let mut event = json!({ "job": info.id, "state": info.state.label(), "kind": "discard",
                "result": info.result });
            if let Some(error) = &info.error {
                event["error"] = error.to_json();
            }
            events.push(event);
        }
        events
    }

    /// Starts one "Build preview proxy" job for the videos the preview policy wants a proxy for
    /// (decoded, taller than the policy's height, no valid proxy yet). It waits for the prepare
    /// job: a proxy is derived from the decoded frames, never from the original file.
    fn schedule_proxies(&mut self, state: &EditorState) {
        if !self.interactive || self.auto_proxy.is_some() || self.auto_prepare.is_some() {
            return;
        }
        let Some(workspace) = state.project.workspace.clone() else {
            return;
        };
        let policy = state.preview_policy;
        let items: Vec<ProxyItem> = proxy_candidates(state, policy, None)
            .into_iter()
            .filter(|(id, asset, _)| {
                let key = (
                    id.clone(),
                    asset.identity.hash.clone(),
                    policy.max_height,
                    policy.format,
                );
                !self.proxy_failed.contains(&key)
            })
            .collect();
        if items.is_empty() {
            return;
        }
        let keys = items
            .iter()
            .map(|(id, asset, _)| {
                (
                    id.clone(),
                    asset.identity.hash.clone(),
                    policy.max_height,
                    policy.format,
                )
            })
            .collect();
        let (id, _) = self.proxy_job(state, workspace, items, policy, "Build preview proxy");
        self.proxy_assets.insert(id.clone(), keys);
        self.auto_proxy = Some(id);
    }

    /// The job behind every proxy build, explicit or automatic.
    fn proxy_job(
        &mut self,
        state: &EditorState,
        workspace: PathBuf,
        items: Vec<ProxyItem>,
        policy: PreviewPolicy,
        label: &str,
    ) -> (JobId, Slot) {
        let tools = self.tools.clone();
        let project = state.project.id.clone();
        self.start(JobKind::Proxy, label, move |ctx| {
            let tools = tools.get()?;
            let cache = Cache::new(&workspace);
            ctx.stage("proxy", items.len() as u64);
            let mut built = Vec::new();
            for (id, asset, frames) in &items {
                ctx.check()?;
                let (proxy, _) = cache.preview_frames(
                    &tools,
                    asset,
                    frames,
                    policy.max_height,
                    policy.format,
                    ctx,
                )?;
                built.push((id.clone(), asset.identity.hash.clone(), proxy));
                ctx.advance(1);
            }
            let ids: Vec<&str> = built.iter().map(|(id, ..)| id.as_str()).collect();
            let report = json!({ "action": "media.preview_proxy", "assets": ids,
                "height": policy.max_height, "format": policy.format.label(),
                "cache_bytes": cache.size() });
            Ok((
                Some(Pending::Proxies {
                    project,
                    items: built,
                }),
                report,
            ))
        })
    }

    /// `media.preview_proxy`: builds the preview proxy the current policy asks for (`assets`
    /// limits it). Never touches the original; export keeps reading it.
    fn preview_proxy(
        &mut self,
        state: &mut EditorState,
        request: &Value,
    ) -> Result<Value, CommandError> {
        let workspace = workspace(state)?;
        let policy = state.preview_policy;
        if policy.mode == tessel_editor_core::preview::PreviewMode::Original {
            return Err(CommandError::new(
                "policy_original",
                "The preview policy is `original`: no proxy is used. Set `preview.policy` mode `auto` first",
            ));
        }
        let wanted = strings(request, "assets")?;
        let items = proxy_candidates(state, policy, wanted.as_deref());
        if items.is_empty() {
            return Ok(json!({ "action": "media.preview_proxy", "assets": [],
                "reason": "nothing needs a proxy (see `preview.policy`)" }));
        }
        let (id, slot) = self.proxy_job(state, workspace, items, policy, "Build preview proxy");
        self.finish(state, request, id, slot)
    }

    /// Starts one "Prepare media" job for every asset that lacks decoded frames or audio. Called
    /// from [`MediaHost::poll`], so a project that was just opened, restored, imported into or
    /// relinked gets its media decoded in the background without anyone asking.
    fn schedule_prepare(&mut self, state: &EditorState) {
        if !self.interactive || self.auto_prepare.is_some() {
            return;
        }
        let Some(workspace) = state.project.workspace.clone() else {
            return;
        };
        let items: Vec<(String, MediaKind, Asset)> = undecoded(state)
            .into_iter()
            .filter(|(id, _, asset)| {
                !self
                    .auto_failed
                    .iter()
                    .any(|(failed, hash)| failed == id && *hash == asset.identity.hash)
            })
            .collect();
        if items.is_empty() {
            return;
        }
        let assets: Vec<(String, Option<String>)> = items
            .iter()
            .map(|(id, _, asset)| (id.clone(), asset.identity.hash.clone()))
            .collect();
        let (id, _) = self.prepare_job(state, workspace, items, "Prepare media");
        self.auto_assets
            .insert(id.clone(), (state.project.id.clone(), assets));
        self.auto_prepare = Some(id);
    }

    /// Shares the job registry used by JSONL and window adapters; it does not apply pending results.
    pub fn registry(&self) -> &JobRegistry {
        &self.registry
    }

    /// Starts `body` as a job. The typed result for the project goes into a slot that
    /// `job.commit` (or `finish`) turns into one transaction.
    fn start(
        &mut self,
        kind: JobKind,
        label: &str,
        body: impl FnOnce(&JobContext) -> Result<(Option<Pending>, Value), JobError> + Send + 'static,
    ) -> (JobId, Slot) {
        let slot: Slot = Arc::default();
        let target = slot.clone();
        let id = self.registry.spawn(kind, label, move |ctx| {
            let (pending, report) = body(ctx)?;
            if pending.is_some() {
                // Not prunable, and discardable, until it is applied or discarded (BL094).
                ctx.hold_result();
            }
            *target
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = pending;
            Ok(report)
        });
        self.pending.insert(id.clone(), slot.clone());
        (id, slot)
    }

    /// Waits for terminal job state within a deadline and refuses jobs that disappear.
    fn wait_final(&self, id: &str, timeout: Duration) -> Result<JobInfo, CommandError> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            // `wait` answers `None` both for an unknown job and for a timeout of its own.
            if let Some(info) = self.registry.wait(id, Duration::from_millis(200)) {
                return Ok(info);
            }
            if self.registry.get(id).is_none() {
                return Err(CommandError::unknown_element("job", id));
            }
            if std::time::Instant::now() >= deadline {
                return Err(CommandError::new(
                    "timeout",
                    format!("Job `{id}` is still running"),
                ));
            }
        }
    }

    /// Either returns the job (`wait: false`) or waits for it and applies its result.
    fn finish(
        &mut self,
        state: &mut EditorState,
        request: &Value,
        id: JobId,
        slot: Slot,
    ) -> Result<Value, CommandError> {
        if self.interactive || !flag(request, "wait", true) {
            let info = self.registry.get(&id).map(|info| info.to_json());
            return Ok(json!({ "job": id, "state": "queued", "info": info }));
        }
        let info = self.wait_final(&id, Duration::from_secs(60 * 60 * 24))?;
        self.pending.remove(&id);
        let result = Self::outcome(&info)?;
        let applied = apply(&self.registry.clone(), &id, take(&slot), state, true)?;
        Ok(merge_applied(result, &id, applied))
    }

    /// Maps terminal results to command replies, preserving stored errors and partial cancellation details.
    fn outcome(info: &JobInfo) -> Result<Value, CommandError> {
        match info.state {
            JobState::Succeeded => Ok(info.result.clone().unwrap_or(Value::Null)),
            JobState::Cancelled => Err(info
                .error
                .clone()
                .unwrap_or_else(JobError::cancelled)
                .into()),
            _ => Err(info.error.clone().map_or_else(
                || CommandError::new("job_failed", "The job failed"),
                CommandError::from,
            )),
        }
    }

    fn handle(
        &mut self,
        state: &mut EditorState,
        op: &str,
        request: &Value,
    ) -> Result<Value, CommandError> {
        match op {
            "media.status" => Ok(import::status(state)),
            "media.import" => self.import(state, request),
            "media.relink" => self.relink(state, request),
            "media.search" => self.search(state, request),
            "media.prepare" => self.prepare(state, request),
            "media.preview_proxy" => self.preview_proxy(state, request),
            "media.thumbnails" => self.thumbnails(state, request),
            "media.thumbnail" | "media.waveform" | "media.proxy" => self.derive(state, op, request),
            "project.package_preflight" => self.package_preflight(state, request),
            "project.package" => self.package(state, request),
            "project.verify" => self.verify(state, request),
            "project.restore" => self.restore(state, request),
            "project.import" => self.project_import(state, request),
            "project.import_scenes" => self.import_scenes(state, request),
            "scene.attach.start" => self.scene_attach(state, request),
            "scene.resources.status" => Ok(self.scene_resources_status(state)),
            "scene.resources.preflight" => self.scene_preflight(state, request),
            "scene.resources.resolve" => self.scene_resolve(state, request),
            "scene.resources.adopt" => self.scene_adopt(state, request),
            "scene.resources.release" => self.scene_release(request),
            "scene.resources.invalidate" => self.scene_invalidate(request),
            "scene.resources.claims" => self.scene_claims(state, request),
            "scene.resources.recover" => self.scene_recover(state, request),
            "export.preflight" => self.export_preflight(state, request),
            "export.start" => self.export_start(state, request),
            "scene.export" => self.scene_export(state, request),
            "scene.export.preflight" => self.scene_export_preflight(state, request),
            "job.list" => Ok(self.registry.to_json()),
            "job.get" => self.job_get(request),
            "job.wait" => self.job_wait(request),
            "job.cancel" => {
                let info = self.registry.cancel(text(request, "job")?)?;
                Ok(json!({ "action": "job.cancel", "info": info.to_json() }))
            }
            "job.commit" => self.job_commit(state, request),
            "job.discard" => self.job_discard(state, request),
            _ => Err(invalid(format!("Unknown media operation `{op}`"))),
        }
    }

    fn import(&mut self, state: &mut EditorState, request: &Value) -> Result<Value, CommandError> {
        let workspace = workspace(state)?;
        let tools = self.tools.clone();
        let paths: Vec<PathBuf> = strings(request, "paths")?
            .ok_or_else(|| invalid("Expected `paths`: an array of file paths"))?
            .into_iter()
            .map(PathBuf::from)
            .collect();
        let mode = match optional_text(request, "mode")? {
            None => ImportMode::Copy,
            Some(label) => ImportMode::parse(label)
                .ok_or_else(|| invalid("`mode` is `copy` or `reference`"))?,
        };
        let folder = optional_text(request, "folder")?.map(str::to_string);
        let capture = tessel_editor_core::ops::capture_choice(request)?;
        let prepare = flag(request, "prepare", true);
        let known = import::known_hashes(state);
        let import_request = ImportRequest {
            paths,
            mode,
            folder: folder.clone(),
            capture,
        };
        let (id, slot) = self.start(JobKind::Import, "Import media", move |ctx| {
            let tools = tools.get()?;
            let items = import::prepare(&tools, &workspace, &import_request, &known, ctx)?;
            let summary =
                json!({ "files": items.iter().map(PreparedItem::to_json).collect::<Vec<_>>() });
            Ok((
                Some(Pending::Import {
                    items,
                    folder,
                    prepare,
                }),
                summary,
            ))
        });
        self.finish(state, request, id, slot)
    }

    fn relink(&mut self, state: &mut EditorState, request: &Value) -> Result<Value, CommandError> {
        let tools = self.tools.clone();
        let asset_id = text(request, "asset")?.to_string();
        let (_, asset) = asset_of(state, &asset_id)?;
        let path = PathBuf::from(text(request, "path")?);
        let accept_changed = flag(request, "accept_changed", false);
        let (id, slot) = self.start(JobKind::Hash, "Relink media", move |ctx| {
            let tools = tools.get()?;
            let (replacement, warnings) =
                import::relink_candidate(&tools, &asset, &path, accept_changed, ctx)?;
            Ok((
                Some(Pending::Relink(vec![(asset_id, replacement)])),
                json!({ "warnings": warnings }),
            ))
        });
        self.finish(state, request, id, slot)
    }

    fn search(&mut self, state: &mut EditorState, request: &Value) -> Result<Value, CommandError> {
        let tools = self.tools.clone();
        let folder = PathBuf::from(text(request, "folder")?);
        let apply = flag(request, "apply", false);
        let snapshot = state.clone();
        let (id, slot) = self.start(JobKind::Hash, "Search media", move |ctx| {
            let tools = tools.get()?;
            let found = import::search_folder(&snapshot, &folder, ctx)?;
            let mut replacements = Vec::new();
            if apply {
                for (asset_id, path) in &found {
                    let asset = snapshot.library.item(asset_id).and_then(|item| item.asset.clone());
                    let Some(asset) = asset else { continue };
                    replacements.push((asset_id.clone(), import::relink_candidate(&tools, &asset, path, false, ctx)?.0));
                }
            }
            let report = json!({ "matches": found.iter().map(|(asset, path)| json!({ "asset": asset, "path": path })).collect::<Vec<_>>() });
            Ok(((!replacements.is_empty()).then_some(Pending::Relink(replacements)), report))
        });
        self.finish(state, request, id, slot)
    }

    /// `media.prepare`: decodes (or revalidates the cache of) the assets that need it. Explicit
    /// callers rarely need it: export, the render CLIs and the window prepare media by themselves.
    fn prepare(&mut self, state: &mut EditorState, request: &Value) -> Result<Value, CommandError> {
        let workspace = workspace(state)?;
        let wanted = strings(request, "assets")?;
        let items: Vec<(String, MediaKind, Asset)> = state
            .library
            .items
            .iter()
            .filter(|item| {
                wanted
                    .as_ref()
                    .is_none_or(|ids| ids.iter().any(|id| id == item.id.as_str()))
            })
            .filter_map(|item| {
                let asset = item.asset.clone()?;
                export::needs_decoding(item.kind, &asset)
                    .then(|| (item.id.as_str().to_string(), item.kind, asset))
            })
            .collect();
        let (id, slot) = self.prepare_job(state, workspace, items, "Prepare media");
        self.finish(state, request, id, slot)
    }

    /// The job behind every preparation, explicit or automatic: decodes `items` into the cache
    /// off the caller's thread and hands the decoded paths back for one transaction.
    fn prepare_job(
        &mut self,
        state: &EditorState,
        workspace: PathBuf,
        items: Vec<(String, MediaKind, Asset)>,
        label: &str,
    ) -> (JobId, Slot) {
        let tools = self.tools.clone();
        let project = state.project.id.clone();
        self.start(JobKind::Proxy, label, move |ctx| {
            let tools = tools.get()?;
            let cache = Cache::new(&workspace);
            let decoded = decode_all(&tools, &cache, &items, ctx)?;
            let ids: Vec<&str> = decoded.iter().map(|(id, ..)| id.as_str()).collect();
            let report =
                json!({ "action": "media.prepare", "assets": ids, "cache_bytes": cache.size() });
            Ok((
                Some(Pending::Decoded {
                    project,
                    items: decoded,
                }),
                report,
            ))
        })
    }

    /// Thumbnails, waveforms and proxies are jobs like everything else that runs a tool.
    fn derive(
        &mut self,
        state: &mut EditorState,
        op: &str,
        request: &Value,
    ) -> Result<Value, CommandError> {
        let tools = self.tools.clone();
        let cache = Cache::new(&workspace(state)?);
        let (kind, asset) = asset_of(state, text(request, "asset")?)?;
        let op = op.to_string();
        let label = op.clone();
        let params = request.clone();
        let (id, slot) = self.start(JobKind::Proxy, &label, move |ctx| {
            let tools = tools.get()?;
            let report = match op.as_str() {
                "media.thumbnail" => {
                    let width = params.get("width").and_then(Value::as_u64).unwrap_or(320) as u32;
                    let at = params.get("at").and_then(Value::as_f64).unwrap_or(0.0);
                    json!({ "path": cache.thumbnail(&tools, &asset, kind, width, at)? })
                }
                "media.waveform" => {
                    let buckets = params.get("buckets").and_then(Value::as_u64).unwrap_or(96) as usize;
                    json!({ "peaks": cache.waveform(&tools, &asset, buckets, ctx)? })
                }
                _ => {
                    let height = params.get("height").and_then(Value::as_u64).unwrap_or(360) as u32;
                    json!({ "path": cache.proxy(&tools, &asset, height, ctx)?, "preview_only": true })
                }
            };
            Ok((None, report))
        });
        self.finish(state, request, id, slot)
    }

    /// Decodes whatever the project still lacks before a render request runs. A CLI waits for it;
    /// a window never blocks, so it schedules the job and answers `media_preparing`.
    fn ensure_prepared(&mut self, state: &mut EditorState, op: &str) -> Result<(), CommandError> {
        let Ok(workspace) = workspace(state) else {
            return Ok(());
        };
        let items = undecoded(state);
        if items.is_empty() {
            return Ok(());
        }
        if self.interactive {
            self.schedule_prepare(state);
            return Err(CommandError::new(
                "media_preparing",
                format!(
                    "`{op}` waits for the media of this project to be prepared; try again in a moment"
                ),
            ));
        }
        let (id, slot) = self.prepare_job(state, workspace, items, "Prepare media");
        let info = self.wait_final(&id, Duration::from_secs(60 * 60 * 24))?;
        self.pending.remove(&id);
        Self::outcome(&info)?;
        apply(&self.registry.clone(), &id, take(&slot), state, true)?;
        Ok(())
    }

    fn package_options(request: &Value) -> Result<PackageOptions, CommandError> {
        let format = match optional_text(request, "format")? {
            None => PackageFormat::Archive,
            Some(label) => PackageFormat::parse(label)
                .ok_or_else(|| invalid("`format` is `archive` or `directory`"))?,
        };
        Ok(PackageOptions {
            format,
            sequences: strings(request, "sequences")?,
            allow_partial: flag(request, "allow_partial", false),
            replace: flag(request, "replace", false),
        })
    }

    fn package_preflight(
        &mut self,
        state: &mut EditorState,
        request: &Value,
    ) -> Result<Value, CommandError> {
        let options = Self::package_options(request)?;
        let inputs = package_inputs(request)?;
        let path = PathBuf::from(text(request, "path")?);
        let snapshot = state.clone();
        let (id, slot) = self.start(JobKind::Preflight, "Package preflight", move |ctx| {
            Ok((
                None,
                package::preflight_with(&snapshot, &options, &inputs, &path, ctx.token())?,
            ))
        });
        self.finish(state, request, id, slot)
    }

    fn package(&mut self, state: &mut EditorState, request: &Value) -> Result<Value, CommandError> {
        let options = Self::package_options(request)?;
        let inputs = package_inputs(request)?;
        let destination = PathBuf::from(text(request, "path")?);
        let snapshot = state.clone();
        let (id, slot) = self.start(JobKind::Package, "Package project", move |ctx| {
            Ok((
                None,
                package::create_with(&snapshot, &destination, &options, &inputs, ctx)?,
            ))
        });
        self.finish(state, request, id, slot)
    }

    fn verify(&mut self, state: &mut EditorState, request: &Value) -> Result<Value, CommandError> {
        let package = PathBuf::from(text(request, "path")?);
        let (id, slot) = self.start(JobKind::Package, "Verify package", move |ctx| {
            Ok((None, package::verify_package(&package, ctx)?))
        });
        self.finish(state, request, id, slot)
    }

    /// Extracts and verifies the package and loads the restored project off the caller's thread;
    /// the swap itself happens at `apply`, and only if the open project did not change meanwhile.
    fn restore(&mut self, state: &mut EditorState, request: &Value) -> Result<Value, CommandError> {
        let package = PathBuf::from(text(request, "path")?);
        let folder = PathBuf::from(text(request, "folder")?);
        let open = flag(request, "open", true);
        let stamp = state.project.stamp();
        let (id, slot) = self.start(JobKind::Restore, "Restore package", move |ctx| {
            let report = package::restore(&package, &folder, ctx)?;
            if !open {
                return Ok((None, report));
            }
            ctx.check()?;
            let document = folder.join(DOCUMENT_NAME);
            let candidate = tessel_editor_core::persist::load(&document)?;
            let summary = open_summary(&candidate, &document);
            Ok((
                Some(Pending::Open {
                    stamp,
                    candidate: Box::new(candidate),
                    summary,
                }),
                report,
            ))
        });
        self.finish(state, request, id, slot)
    }

    fn project_import(
        &mut self,
        state: &mut EditorState,
        request: &Value,
    ) -> Result<Value, CommandError> {
        let workspace = workspace(state)?;
        let source = PathBuf::from(text(request, "source")?);
        let sequences = strings(request, "sequences")?.unwrap_or_default();
        let dry_run = flag(request, "dry_run", false);
        let destination = state.clone();
        let (id, slot) = self.start(JobKind::ProjectImport, "Import from project", move |ctx| {
            let loaded = interchange::load_source(&source, &workspace, ctx)?;
            if dry_run {
                let plan = tessel_editor_core::interchange::plan_import(&destination, &loaded.state, &sequences).map_err(JobError::from);
                loaded.cleanup();
                return Ok((None, json!({ "dry_run": true, "plan": plan?.to_json() })));
            }
            let prepared = interchange::prepare(&destination, loaded.state.clone(), &workspace, &sequences, ctx);
            loaded.cleanup();
            let prepared = prepared?;
            let report = json!({ "plan": prepared.plan.to_json(), "copied": prepared.copied, "reused": prepared.reused });
            Ok((Some(Pending::ProjectImport(Box::new(prepared))), report))
        });
        self.finish(state, request, id, slot)
    }

    /// The scene named by `scene`, its bundle shared (not copied) with the job.
    fn scene_of(
        state: &EditorState,
        request: &Value,
    ) -> Result<(String, Arc<DirectedScene>), CommandError> {
        let id = text(request, "scene")?;
        let attached = state
            .scenes
            .get(id)
            .ok_or_else(|| CommandError::unknown_element("scene", id))?;
        Ok((id.to_string(), attached.scene.clone()))
    }

    /// `base` when the caller names one (absolute), else the managed base of the workspace.
    fn scene_base(
        state: &EditorState,
        request: &Value,
        id: &str,
    ) -> Result<ResourceBase, CommandError> {
        if let Some(root) = optional_text(request, "base")? {
            return Ok(ResourceBase::explicit(root)?);
        }
        Ok(ResourceBase::managed(&workspace(state)?, id)?)
    }

    /// [`Self::scene_base`], except that a scene whose manifest names no files needs no folder
    /// and so no workspace either (BL101): it loads against [`ResourceBase::unbound`]. Anything
    /// else keeps the error that says why there is no base.
    fn scene_base_for(
        state: &EditorState,
        request: &Value,
        id: &str,
        scene: &DirectedScene,
    ) -> Result<ResourceBase, CommandError> {
        match Self::scene_base(state, request, id) {
            Err(error)
                if error.code == "no_workspace" && !crate::scene_resources::needs_files(scene) =>
            {
                Ok(ResourceBase::unbound())
            }
            outcome => outcome,
        }
    }

    /// `scene.attach.start {path | bundle | text, id?, name?, wait?}`: `scene.attach` with the
    /// slow part on a job (BL100). Reading, parsing, importing and the structural and temporal
    /// gates run off the caller's thread (cancellable, `bundle_unreadable`/`invalid_bundle`/...
    /// as the job's typed error); the scene is attached as one undo step by `poll`, `job.commit`
    /// or, for a host that may block, at once. The attach itself is still `scene.attach`'s core
    /// command, so the ID, room and byte-budget rules are not repeated here.
    ///
    /// The result applies only while the open project is exactly the one it was started for (ID,
    /// editing session and revision); otherwise it fails `stale_result` and nothing is attached. The answer carries
    /// `resources`: how many files the scene needs and whether it can load without a folder.
    fn scene_attach(
        &mut self,
        state: &mut EditorState,
        request: &Value,
    ) -> Result<Value, CommandError> {
        let source = tessel_editor_core::ops::scene_source(request)?;
        let id = optional_text(request, "id")?.map(str::to_string);
        let name = optional_text(request, "name")?.map(str::to_string);
        // Refused here, before any file is read, exactly as `scene.attach` would.
        state.scenes.check_attach(id.as_deref(), name.as_deref())?;
        let stamp = state.project.stamp();
        let (job, slot) = self.start(JobKind::Import, "Attach scene", move |ctx| {
            ctx.stage("read", 1);
            let scene = AdmittedScene::prepare(source, ctx.token())?;
            ctx.advance(1);
            let resources = scene_resources_needed(scene.scene());
            let report = json!({ "action": "scene.attach.start",
                "bundle_id": scene.scene().bundle().bundle_id,
                "revision_id": scene.scene().bundle().revision_id,
                "resources": resources });
            Ok((
                Some(Pending::SceneAttach {
                    stamp,
                    id,
                    name,
                    scene,
                    resources,
                }),
                report,
            ))
        });
        self.finish(state, request, job, slot)
    }

    /// `scene.resources.status`: what the session holds and, per attached scene, whether that is
    /// still current for the scene's bundle and where its managed base is.
    fn scene_resources_status(&self, state: &EditorState) -> Value {
        self.scene_resources.observe_project(&state.project.id);
        let mut report = self.scene_resources.status();
        let workspace = state.project.workspace.clone();
        report["attached"] = json!(
            state
                .scenes
                .iter()
                .map(|attached| {
                    let held = self.scene_resources.get(&attached.id);
                    let base = workspace
                        .as_deref()
                        .and_then(|workspace| ResourceBase::managed(workspace, &attached.id).ok());
                    json!({
                        "scene": attached.id,
                        "loaded": held.is_some(),
                        // A map of an older bundle revision is stale even in the same generation:
                        // the renderer must reload, not trust it.
                        "current": held.as_ref().is_some_and(|map| {
                            map.bundle_revision == attached.scene.bundle().revision_id
                        }),
                        "managedBase": base.as_ref().map(ResourceBase::to_json),
                        "managedBaseExists": base.as_ref().is_some_and(ResourceBase::exists),
                    })
                })
                .collect::<Vec<_>>()
        );
        report
    }

    /// `scene.resources.preflight`: read-only check, with every problem listed. Installs nothing.
    fn scene_preflight(
        &mut self,
        state: &mut EditorState,
        request: &Value,
    ) -> Result<Value, CommandError> {
        let (id, scene) = Self::scene_of(state, request)?;
        let base = Self::scene_base_for(state, request, &id, &scene)?;
        let (id_job, slot) = self.start(
            JobKind::Preflight,
            "Scene resources preflight",
            move |ctx| {
                ctx.stage("verify", scene_file_total(&scene));
                let report = crate::scene_resources::check(
                    &scene,
                    &base,
                    &ResourceOptions::default(),
                    ctx.token(),
                    &mut |progress| ctx.progress(progress.done as u64),
                )?;
                Ok((
                    None,
                    json!({ "action": "scene.resources.preflight", "scene": id,
                    "base": base.to_json(), "report": report.to_json() }),
                ))
            },
        );
        self.finish(state, request, id_job, slot)
    }

    /// `scene.resources.resolve`: verifies the files and installs the approved map in the
    /// session, under the generation current when the job started.
    fn scene_resolve(
        &mut self,
        state: &mut EditorState,
        request: &Value,
    ) -> Result<Value, CommandError> {
        let (id, scene) = Self::scene_of(state, request)?;
        let base = Self::scene_base_for(state, request, &id, &scene)?;
        let project = state.project.id.clone();
        let generation = self.scene_resources.observe_project(&project);
        let session = self.scene_resources.clone();
        let (id_job, slot) = self.start(JobKind::Preflight, "Load scene resources", move |ctx| {
            ctx.stage("verify", scene_file_total(&scene));
            let map = crate::scene_resources::load(
                &LoadRequest {
                    scene: &scene,
                    scene_id: &id,
                    project_id: &project,
                    generation,
                    base: &base,
                },
                &ResourceOptions::default(),
                ctx.token(),
                &mut |progress| ctx.progress(progress.done as u64),
            )?;
            let report = json!({ "action": "scene.resources.resolve", "map": map.to_json() });
            Ok((
                Some(Pending::SceneResources {
                    map: Box::new(map),
                    session,
                }),
                report,
            ))
        });
        self.finish(state, request, id_job, slot)
    }

    /// `scene.resources.adopt`: copies the files from `source` (an absolute folder) into the
    /// project's managed base. Also the relink: run it again with another `source`.
    fn scene_adopt(
        &mut self,
        state: &mut EditorState,
        request: &Value,
    ) -> Result<Value, CommandError> {
        let (id, scene) = Self::scene_of(state, request)?;
        let workspace = workspace(state)?;
        let source = ResourceBase::explicit(text(request, "source")?)?;
        let target = AdoptionTarget::of(state, &id);
        let (id_job, slot) = self.start(JobKind::Import, "Adopt scene resources", move |ctx| {
            ctx.stage("copy", scene_file_total(&scene));
            let mut adoption = crate::scene_resources::adopt(
                &scene,
                &id,
                &workspace,
                &source,
                &ResourceOptions::default(),
                ctx.token(),
                &mut |progress| ctx.progress(progress.done as u64),
                &mut |_| {},
            )?;
            let report = json!({ "action": "scene.resources.adopt", "scene": id,
                "base": adoption.base.to_json(), "report": adoption.report.to_json() });
            let pending = PendingAdoption::new(target, &mut adoption);
            Ok((Some(Pending::SceneAdopt(Box::new(pending))), report))
        });
        self.finish(state, request, id_job, slot)
    }

    fn scene_release(&mut self, request: &Value) -> Result<Value, CommandError> {
        let id = text(request, "scene")?;
        let released = self.scene_resources.release(id);
        Ok(json!({ "action": "scene.resources.release", "scene": id, "released": released }))
    }

    /// `scene.resources.invalidate`: everything approved so far is forgotten (a renderer reload,
    /// a lost graphics context, a replaced asset). `reason` is one of `reload`, `context_loss`,
    /// `asset_swap`, `requested`.
    fn scene_invalidate(&mut self, request: &Value) -> Result<Value, CommandError> {
        let reason = match optional_text(request, "reason")? {
            None => Invalidation::Requested,
            Some(label) => Invalidation::parse(label).ok_or_else(|| {
                invalid("`reason` is `reload`, `context_loss`, `asset_swap`, `requested` or `project_switch`")
            })?,
        };
        let generation = self.scene_resources.invalidate(reason);
        Ok(
            json!({ "action": "scene.resources.invalidate", "reason": reason.as_str(),
            "generation": generation.0 }),
        )
    }

    /// `scene.resources.claims`: who holds what in the claims ledger of the project's workspace
    /// (BL111). A job like every operation that touches files, but **not read-only**: opening the
    /// ledger reaps the owners whose lease is gone, and the answer says so (`readOnly: false`,
    /// `effects`, `reaped`). It removes no resource file and hashes nothing. Changes no project
    /// data. A ledger that cannot be read is an answer (`available: false`, `fault`), not a failure.
    fn scene_claims(
        &mut self,
        state: &mut EditorState,
        request: &Value,
    ) -> Result<Value, CommandError> {
        let workspace = workspace(state)?;
        // `Preflight` is the nearest kind: it is the one for jobs that look at files and change
        // no project; the label and the answer carry the exact meaning.
        let (id_job, slot) = self.start(JobKind::Preflight, "Scene resources claims", move |ctx| {
            ctx.stage("ledger", 1);
            ctx.check()?;
            let report = crate::scene_resources::claims_status(&workspace);
            ctx.progress(1);
            Ok((None, report))
        });
        self.finish(state, request, id_job, slot)
    }

    /// `scene.resources.recover`: takes back what a result that no longer exists left claimed in
    /// the project's workspace: orphans only, after the same content proof as any cleanup, under
    /// the lease of each root (BL111). It hashes every orphan file, so it is a cancellable job
    /// with progress in orphan files; `claims_unavailable` when the ledger cannot be used,
    /// with any partial removals reported in details, as for `cancelled`. Changes no
    /// project data. `Hash` is the kind of the job that reads file bytes.
    fn scene_recover(
        &mut self,
        state: &mut EditorState,
        request: &Value,
    ) -> Result<Value, CommandError> {
        let workspace = workspace(state)?;
        let (id_job, slot) = self.start(JobKind::Hash, "Recover scene resources", move |ctx| {
            ctx.stage("recover", 0);
            let recovery = crate::scene_resources::recover_with(
                &workspace,
                ctx.token(),
                &mut |done, total| {
                    if done == 0 {
                        ctx.stage("recover", total as u64);
                    } else {
                        ctx.progress(done as u64);
                    }
                },
            )?;
            for kept in &recovery.report.kept {
                ctx.warn(kept.clone());
            }
            Ok((
                None,
                json!({ "action": "scene.resources.recover",
                "workspace": workspace.display().to_string(),
                "recovery": recovery.to_json() }),
            ))
        });
        self.finish(state, request, id_job, slot)
    }

    /// `project.import_scenes`: carries scenes (their bundle and files) from another project or
    /// package; see `scene_resources::import` for what is guaranteed about IDs, references and paths.
    fn import_scenes(
        &mut self,
        state: &mut EditorState,
        request: &Value,
    ) -> Result<Value, CommandError> {
        let workspace = workspace(state)?;
        let source = PathBuf::from(text(request, "source")?);
        let wanted = strings(request, "scenes")?.unwrap_or_default();
        let bases = scene_bases(request)?;
        let dry_run = flag(request, "dry_run", false);
        let destination = state.clone();
        let (id, slot) = self.start(JobKind::ProjectImport, "Import scenes", move |ctx| {
            let loaded = interchange::load_source(&source, &workspace, ctx)?;
            if dry_run {
                let plan = scene_import::plan(&destination, &loaded.state, &wanted);
                loaded.cleanup();
                let plan = plan?;
                return Ok((None, json!({ "dry_run": true, "scenes": plan.iter().map(|scene| json!({
                    "from": scene.from, "to": scene.to, "renamed": scene.renamed() })).collect::<Vec<_>>() })));
            }
            let prepared = scene_import::prepare(
                &destination,
                &loaded.state,
                &workspace,
                &wanted,
                &bases,
                ctx.token(),
                &mut |progress| ctx.progress(progress.done as u64),
            );
            loaded.cleanup();
            let prepared = prepared?;
            let report = prepared.to_json();
            Ok((Some(Pending::SceneImport(Box::new(prepared))), report))
        });
        self.finish(state, request, id, slot)
    }

    fn export_request(
        state: &EditorState,
        request: &Value,
    ) -> Result<(EditorState, ExportRequest), CommandError> {
        let preset = match request.get("preset") {
            Some(Value::String(id)) => preset::find(id, &state.project.user_presets)?,
            Some(inline @ Value::Object(_)) => Preset::from_json(inline)?,
            _ => match state.project.export_preset.as_deref() {
                Some(id) => preset::find(id, &state.project.user_presets)?,
                None => {
                    return Err(invalid(
                        "Expected `preset`: a preset ID or an inline preset",
                    ));
                }
            },
        };
        let mut snapshot = state.clone();
        if let Some(sequence) = optional_text(request, "sequence")? {
            snapshot.execute(Command::ActivateSequence {
                sequence: sequence.to_string(),
            })?;
        }
        let mut export = ExportRequest::new(preset, text(request, "destination")?);
        export.replace = flag(request, "replace", false);
        if let Some(range) = request.get("range").filter(|range| !range.is_null()) {
            let bounds = range
                .as_array()
                .filter(|list| list.len() == 2)
                .and_then(|list| Some((list[0].as_i64()?, list[1].as_i64()?)));
            export.range = Some(
                bounds.ok_or_else(|| invalid("`range` is `[start, end)` in sequence frames"))?,
            );
        }
        Ok((snapshot, export))
    }

    /// The preflight reads files, runs FFmpeg's capability queries and starts a renderer to ask
    /// its limits: a job. The probe runs on a side thread so cancelling does not wait for a
    /// renderer that is slow to start; the orphan finishes alone and drops its renderer.
    fn export_preflight(
        &mut self,
        state: &mut EditorState,
        request: &Value,
    ) -> Result<Value, CommandError> {
        let tools = self.tools.clone();
        let (snapshot, export) = Self::export_request(state, request)?;
        let factory = self.renderer.clone();
        let (id, slot) = self.start(JobKind::Preflight, "Export preflight", move |ctx| {
            let tools = tools.get()?;
            ctx.stage("renderer", 1);
            let capabilities = factory
                .map(|factory| probe_capabilities(factory, ctx))
                .transpose()?
                .flatten();
            ctx.advance(1);
            ctx.stage("checks", 1);
            ctx.check()?;
            let report = export::preflight(&snapshot, &export, &tools, capabilities.as_ref());
            ctx.advance(1);
            Ok((None, report))
        });
        self.finish(state, request, id, slot)
    }

    fn export_start(
        &mut self,
        state: &mut EditorState,
        request: &Value,
    ) -> Result<Value, CommandError> {
        let tools = self.tools.clone();
        let factory = self.renderer.clone().ok_or_else(|| {
            CommandError::new(
                "backend_unavailable",
                "No renderer is attached: exports need `./scripts/render` or the desktop app",
            )
        })?;
        let (snapshot, export) = Self::export_request(state, request)?;
        let (id, slot) = self.start(JobKind::VideoExport, "Export video", move |ctx| {
            let tools = tools.get()?;
            let mut renderer = factory()?;
            let report = export::run(&snapshot, &export, &tools, renderer.as_mut(), ctx);
            renderer.reset_project();
            Ok((None, report?))
        });
        self.finish(state, request, id, slot)
    }

    /// `scene.export {scene, preset, destination, from?, frames?, replace?, audio?, wait?}`: a
    /// directed scene as an H.264 MP4 (F042). Everything that can be refused without touching a
    /// file is refused here, before a job exists: shape, scene, range, preset, audio, resources.
    /// The rest (FFmpeg, destination, disk) is the job's preflight. The result changes no project
    /// state, so there is nothing to commit.
    fn scene_export(
        &mut self,
        state: &mut EditorState,
        request: &Value,
    ) -> Result<Value, CommandError> {
        let factory = self.scene_renderer.clone().ok_or_else(|| {
            CommandError::new(
                "backend_unavailable",
                "No scene renderer is attached: scene exports need `./scripts/render`",
            )
        })?;
        let export = self.freeze_scene_export(state, request)?;
        let tools = self.tools.clone();
        let (id, slot) = self.start(JobKind::VideoExport, "Export scene", move |ctx| {
            let tools = tools.get()?;
            let mut renderer = factory()?;
            let report = scene_encoder::run(&export, &tools, renderer.as_mut(), ctx);
            renderer.finish();
            Ok((None, report?))
        });
        self.finish(state, request, id, slot)
    }

    /// `scene.export.preflight`: the same request as `scene.export`, answered with what would stop
    /// it (FFmpeg, destination, resources, disk) and the plan summary, without starting a job or
    /// touching the destination. It does not need a renderer, so the renderer's own size limit is
    /// the one thing it cannot check (the job's preflight does).
    fn scene_export_preflight(
        &mut self,
        state: &mut EditorState,
        request: &Value,
    ) -> Result<Value, CommandError> {
        let export = self.freeze_scene_export(state, request)?;
        let tools = self.tools.get()?;
        Ok(scene_encoder::preflight(&export, &tools, None))
    }

    /// The part of `scene.export` that is refused without a job: request shape, scene, range,
    /// preset, audio and the frozen, verified resources.
    fn freeze_scene_export(
        &mut self,
        state: &EditorState,
        request: &Value,
    ) -> Result<SceneExport, CommandError> {
        let spec = SceneExportSpec::from_json(state, request)?;
        let destination = PathBuf::from(text(request, "destination")?);
        let snapshot = SceneExportSnapshot::capture(state, &spec)?;
        self.scene_resources.observe_project(&state.project.id);
        SceneExport::freeze(
            snapshot,
            &self.scene_resources,
            destination,
            flag(request, "replace", false),
        )
    }

    /// Returns observable job state without applying its pending project mutation.
    fn job_get(&mut self, request: &Value) -> Result<Value, CommandError> {
        let id = text(request, "job")?;
        let info = self
            .registry
            .get(id)
            .ok_or_else(|| CommandError::unknown_element("job", id))?;
        Ok(info.to_json())
    }

    /// Waits without committing in headless sessions; interactive callers get a snapshot at once.
    fn job_wait(&mut self, request: &Value) -> Result<Value, CommandError> {
        let id = text(request, "job")?;
        if self.interactive {
            // Waiting would freeze the window; the caller polls `job.get` instead.
            return self.job_get(request);
        }
        let timeout = Duration::from_millis(
            request
                .get("timeout_ms")
                .and_then(Value::as_u64)
                .unwrap_or(60_000),
        );
        let info = self.wait_final(id, timeout)?;
        Ok(info.to_json())
    }

    /// Applies a successful pending result once; terminal errors retain their cleanup details.
    fn job_commit(
        &mut self,
        state: &mut EditorState,
        request: &Value,
    ) -> Result<Value, CommandError> {
        let id = text(request, "job")?;
        let info = self
            .registry
            .get(id)
            .ok_or_else(|| CommandError::unknown_element("job", id))?;
        if !info.state.is_final() {
            return Err(CommandError::new(
                "job_running",
                format!("Job `{id}` has not finished"),
            ));
        }
        match info.result_state {
            ResultState::Discarding => {
                return Err(CommandError::new(
                    "job_discarding",
                    format!("Job `{id}`: its result is being discarded and cannot be applied"),
                ));
            }
            ResultState::Discarded => {
                return Err(CommandError::new(
                    "job_discarded",
                    format!("Job `{id}`: its result was discarded and cannot be applied"),
                ));
            }
            _ => {}
        }
        let result = Self::outcome(&info)?;
        let slot = self.pending.remove(id).ok_or_else(|| {
            CommandError::new(
                "job_finished",
                format!("Job `{id}` has nothing to apply or was already applied"),
            )
        })?;
        self.release_thumbnail_job(state, id, info.state == JobState::Cancelled);
        let outcome = apply(
            &self.registry.clone(),
            id,
            take(&slot),
            state,
            !self.interactive,
        );
        if self.interactive {
            // `poll` never saw this job finish: it reports the end (see the field).
            let kind = info.kind.label();
            self.committed_unreported.push(match &outcome {
                Ok(applied) => json!({ "job": id, "state": "succeeded", "kind": kind,
                    "result": result.clone(), "applied": applied }),
                Err(error) => json!({ "job": id, "state": "failed", "kind": kind,
                    "error": error.to_json()["error"] }),
            });
        }
        Ok(merge_applied(result, id, outcome?))
    }

    /// `job.discard {job, wait?}`: gives up the finished, unapplied result of `job` (BL094).
    ///
    /// The result is claimed atomically (`begin_discard`), so a concurrent commit or a second
    /// discard loses with a typed error, and it moves into a `discard` job that removes its files
    /// off this thread and keeps owning them until it has finished. The answer names that job
    /// (`job`) and the one it discards (`discards`). A host without a window waits for it unless
    /// `wait: false`; an interactive one always answers at once and `poll` reports the end.
    fn job_discard(&mut self, state: &EditorState, request: &Value) -> Result<Value, CommandError> {
        let id = text(request, "job")?;
        let cancelled = self
            .registry
            .get(id)
            .is_some_and(|info| info.state == JobState::Cancelled);
        self.registry.begin_discard(id)?;
        self.release_thumbnail_job(state, id, cancelled);
        let slot = self.pending.remove(id);
        if self.auto_prepare.as_deref() == Some(id) {
            self.auto_prepare = None;
        }
        if self.auto_proxy.as_deref() == Some(id) {
            self.auto_proxy = None;
        }
        self.proxy_assets.remove(id);
        self.auto_assets.remove(id);
        let pending = slot.as_ref().and_then(take);
        let discard = spawn_discard(&self.registry, id, pending);
        if self.interactive && slot.is_some() {
            // `poll` never saw this job finish: it reports the end (see the field).
            self.discarded_unreported
                .push((id.to_string(), discard.clone()));
        }
        if self.interactive || !flag(request, "wait", true) {
            let info = self.registry.get(&discard).map(|info| info.to_json());
            return Ok(
                json!({ "action": "job.discard", "job": discard, "discards": id,
                "state": "discarding", "info": info }),
            );
        }
        let info = self.wait_final(&discard, Duration::from_secs(60 * 60 * 24))?;
        self.reported_discards.insert(discard.clone());
        let mut report = Self::outcome(&info)?;
        report["job"] = json!(discard);
        report["state"] = json!("discarded");
        Ok(report)
    }
}

/// `scene_bases`: `{ "<sceneId>": "<absolute folder>" }`, a folder the caller authorizes for one
/// operation (never stored). Relative folders are refused later, by `ResourceBase::explicit`.
fn scene_bases(
    request: &Value,
) -> Result<std::collections::BTreeMap<String, PathBuf>, CommandError> {
    let Some(value) = request.get("scene_bases").filter(|value| !value.is_null()) else {
        return Ok(Default::default());
    };
    let object = value
        .as_object()
        .ok_or_else(|| invalid("`scene_bases` must be an object of scene ID → folder"))?;
    object
        .iter()
        .map(|(id, root)| {
            root.as_str()
                .map(|root| (id.clone(), PathBuf::from(root)))
                .ok_or_else(|| invalid("`scene_bases` values must be folder paths"))
        })
        .collect()
}

fn package_inputs(request: &Value) -> Result<PackageInputs, CommandError> {
    Ok(PackageInputs {
        scene_bases: scene_bases(request)?,
    })
}

/// What a scene's resources ask of the project, from the core's own manifest: `files` is how many
/// distinct files a load opens (`null` when the manifest is blocked and has no plan) and
/// `needs_folder` is whether loading needs a folder at all (BL101: `false` for data-only scenes).
fn scene_resources_needed(scene: &DirectedScene) -> Value {
    let files = ResourceManifest::from_scene(scene)
        .transport_plan()
        .ok()
        .map(|plan| plan.items.len());
    json!({ "files": files, "needs_folder": crate::scene_resources::needs_files(scene) })
}

/// Files a scene's supported resources need: the job's progress total.
fn scene_file_total(scene: &DirectedScene) -> u64 {
    ResourceManifest::from_scene(scene)
        .transport_plan()
        .map_or(0, |plan| plan.items.len() as u64)
}

/// Assets whose decoded media is missing: container videos and compressed audio that were
/// opened, restored or imported without the runtime-only cache being filled.
fn undecoded(state: &EditorState) -> Vec<(String, MediaKind, Asset)> {
    state
        .library
        .items
        .iter()
        .filter_map(|item| {
            let asset = item.asset.clone()?;
            (asset.decoded.is_none() && export::needs_decoding(item.kind, &asset))
                .then(|| (item.id.as_str().to_string(), item.kind, asset))
        })
        .collect()
}

type ProxyItem = (String, Asset, DecodedFrames);

/// Videos the policy wants a proxy for: decoded, taller than the policy's height, and with no
/// valid proxy (`Missing`) or a stale one. Assets that were never hashed are skipped: the cache
/// is addressed by content hash.
fn proxy_candidates(
    state: &EditorState,
    policy: PreviewPolicy,
    only: Option<&[String]>,
) -> Vec<ProxyItem> {
    state
        .library
        .items
        .iter()
        .filter(|item| only.is_none_or(|ids| ids.iter().any(|id| id == item.id.as_str())))
        .filter_map(|item| {
            let asset = item.asset.as_ref()?;
            asset.identity.hash.as_ref()?;
            let wants = matches!(
                policy.assess(item.kind, Some(asset)),
                Reason::Missing | Reason::Stale
            );
            let frames = asset.decoded.as_ref()?.frames.clone()?;
            wants.then(|| (item.id.as_str().to_string(), asset.clone(), frames))
        })
        .collect()
}

/// Decodes container video and compressed audio into the cache (runtime-only `decoded` paths),
/// as one stage so progress and cancellation cover the whole batch. Each result carries the
/// content hash it was made from, which `apply` checks before storing it.
fn decode_all(
    tools: &Tools,
    cache: &Cache,
    items: &[(String, MediaKind, Asset)],
    ctx: &JobContext,
) -> Result<Vec<DecodedItem>, JobError> {
    let mut decoded = Vec::new();
    ctx.stage("decode", items.len() as u64);
    for (id, kind, asset) in items {
        ctx.check()?;
        decoded.push((
            id.clone(),
            asset.identity.hash.clone(),
            cache.prepare(tools, asset, *kind, ctx)?,
        ));
        ctx.advance(1);
    }
    Ok(decoded)
}

/// Asks the renderer for its limits on a side thread, so the job can still be cancelled while a
/// renderer is slow to start. `None` when the renderer is unavailable: the preflight then
/// reports without its limits instead of failing.
fn probe_capabilities(
    factory: Arc<RendererFactory>,
    ctx: &JobContext,
) -> Result<Option<tessel_editor_core::backend::Capabilities>, JobError> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let capabilities = factory()
            .ok()
            .and_then(|mut renderer| renderer.capabilities().ok());
        let _ = sender.send(capabilities);
    });
    loop {
        ctx.check()?;
        match receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(capabilities) => return Ok(capabilities),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(None),
        }
    }
}

/// Consumes the pending result once so two application paths cannot own the same files.
fn take(slot: &Slot) -> Option<Pending> {
    slot.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
}

/// The single place a job result changes the project: always one transaction (or one command),
/// and only for the project the job was computed for. `decode_inline` is for callers that may
/// block (the CLIs): an import then also decodes its new media before answering; a window leaves
/// that to the automatic preparation job.
///
/// `decode_inline` is also "may block": a window's tick (`false`) must not delete files, so a result
/// that owns files and no longer fits the project goes to a discard job (BL112) instead of being
/// removed here. Either way the job's `result_state` ends `applied` or `discarded`/`discarding`.
fn apply(
    registry: &JobRegistry,
    id: &str,
    pending: Option<Pending>,
    state: &mut EditorState,
    decode_inline: bool,
) -> Result<Option<Value>, CommandError> {
    let Some(pending) = pending else {
        return Ok(None);
    };
    // `took` is what the project did with the result, said by the arm that did it: `Ok` and
    // "applied" are not the same thing. An import can change the project and still fail to decode
    // what it brought (applied, with an error), and a decoded or proxy result that is stale
    // answers `Ok` without changing anything (discarded).
    let mut took = false;
    let outcome = apply_pending(registry, id, pending, state, decode_inline, &mut took);
    // A result that was refused is gone (or already claimed by the discard job it was handed to).
    registry.settle(id, took);
    outcome
}

/// Hands a refused result that owns files to a discard job and reports why it was refused.
///
/// `refusal` is the project's own error when it said no (ID taken, scene limit) or the ledger
/// refused the acceptance (`claims_unavailable`): it keeps its code and details, which gain the
/// discard job that now owns the files. Without one the result is plain `stale_result`.
fn defer_discard(
    registry: &JobRegistry,
    id: &str,
    pending: Pending,
    message: String,
    refusal: Option<CommandError>,
) -> CommandError {
    // The job is final and its result pending: the claim cannot be refused.
    let claimed = registry.begin_discard(id).is_ok();
    let discard = spawn_discard(registry, id, Some(pending));
    let mut error = CommandError::new("stale_result", message);
    let mut details = json!({ "discard_job": discard, "claimed": claimed });
    if let Some(refusal) = refusal {
        error.code = refusal.code;
        if let Some(Value::Object(own)) = refusal.details.map(|own| *own) {
            for (key, value) in own {
                details[key] = value;
            }
        }
    }
    error.details = Some(Box::new(details));
    error
}

/// An error of a step that ran **after** the project took the result: it says so in
/// `details.applied`, so a caller never reads "failed" as "nothing changed".
fn applied_but(mut error: CommandError) -> CommandError {
    let mut details = match error.details.take().map(|details| *details) {
        Some(details @ Value::Object(_)) => details,
        _ => json!({}),
    };
    details["applied"] = json!(true);
    error.details(details)
}

/// Applies `pending` to the project. `took` is set the moment the project has taken the result
/// (so it answers `true` even if a later step of the same arm fails) and stays `false` when
/// nothing of it was used.
fn apply_pending(
    registry: &JobRegistry,
    id: &str,
    pending: Pending,
    state: &mut EditorState,
    decode_inline: bool,
    took: &mut bool,
) -> Result<Option<Value>, CommandError> {
    match pending {
        Pending::Import {
            items,
            folder,
            prepare: decode,
        } => {
            let mut report = import::commit(state, &items, folder.as_deref())?;
            // The assets are in the project from here on, whatever the decoding below does.
            *took = true;
            if decode
                && decode_inline
                && let Ok(workspace) = workspace(state)
            {
                let cache = Cache::new(&workspace);
                let tools = Tools::discover().map_err(|error| applied_but(error.into()))?;
                let items: Vec<(String, MediaKind, Asset)> = report["items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|item| item["id"].as_str())
                    .filter_map(|id| {
                        asset_of(state, id)
                            .ok()
                            .map(|(kind, asset)| (id.to_string(), kind, asset))
                    })
                    .filter(|(_, kind, asset)| export::needs_decoding(*kind, asset))
                    .collect();
                let mut decoded = Vec::new();
                let (_, outcome) = registry.run_inline(JobKind::Proxy, "Prepare media", |ctx| {
                    decoded = decode_all(&tools, &cache, &items, ctx)?;
                    Ok(Value::Null)
                });
                outcome.map_err(|error| applied_but(error.into()))?;
                report["decoded"] = json!(decoded.iter().map(|(id, ..)| id).collect::<Vec<_>>());
                for (id, _, outcome) in decoded {
                    state.set_decoded(&id, outcome);
                }
            }
            Ok(Some(report))
        }
        Pending::Relink(replacements) => {
            let ids: Vec<String> = replacements.iter().map(|(id, _)| id.clone()).collect();
            state.transaction("Relink media", |state| {
                for (asset_id, asset) in replacements {
                    state.execute(Command::ReplaceAsset {
                        asset_id,
                        asset: Box::new(asset),
                    })?;
                }
                Ok(())
            })?;
            *took = true;
            Ok(Some(
                json!({ "action": "media.relink", "assets": ids, "changed": true }),
            ))
        }
        Pending::ProjectImport(prepared) => {
            let report = apply_import(
                state,
                &prepared.source,
                &prepared.plan,
                &prepared.replacements,
            )?;
            *took = true;
            Ok(Some(report))
        }
        Pending::Proxies { project, items } => {
            if state.project.id != project {
                return Ok(Some(
                    json!({ "action": "media.preview_proxy", "stale": true,
                    "reason": "another project is open now", "applied": [] }),
                ));
            }
            let (mut applied, mut stale) = (Vec::new(), Vec::new());
            for (id, hash, proxy) in items {
                let current = state
                    .library
                    .item(&id)
                    .and_then(|item| item.asset.as_ref())
                    .map(|asset| asset.identity.hash.clone());
                // `set_preview_proxy` also refuses a proxy of frames the asset no longer uses.
                if current.as_ref() == Some(&hash) && state.set_preview_proxy(&id, Some(proxy)) {
                    applied.push(id);
                } else {
                    stale.push(id);
                }
            }
            // Everything stale: the answer is `Ok` but the project used none of it.
            *took = !applied.is_empty();
            Ok(Some(
                json!({ "action": "media.preview_proxy", "applied": applied, "stale": stale }),
            ))
        }
        Pending::Thumbnails { stamp, made } => {
            if !stamp.is_current(state) {
                return Ok(Some(json!({ "action": "media.thumbnails", "stale": true,
                    "reason": "another project is open now", "applied": [] })));
            }
            let (mut applied, mut failed, mut stale) = (Vec::new(), Vec::new(), Vec::new());
            for made in made {
                let asset = made.asset().to_string();
                let stored = match made {
                    Made::Ready(id, thumbnail) => state
                        .set_thumbnail(&stamp, &id, thumbnail)
                        .map(|()| applied.push(asset.clone())),
                    Made::Failed(id, failure) => state
                        .set_thumbnail_failure(&stamp, &id, failure)
                        .map(|()| failed.push(asset.clone())),
                };
                if stored.is_err() {
                    stale.push(asset);
                }
            }
            state.prune_thumbnails();
            // Failures are recorded (so they are not retried every tick) but are not pictures.
            *took = !applied.is_empty() || !failed.is_empty();
            Ok(Some(
                json!({ "action": "media.thumbnails", "applied": applied,
                "failed": failed, "stale": stale }),
            ))
        }
        Pending::Decoded { project, items } => {
            if state.project.id != project {
                return Ok(Some(json!({ "action": "media.prepare", "stale": true,
                    "reason": "another project is open now", "applied": [] })));
            }
            let (mut applied, mut stale) = (Vec::new(), Vec::new());
            for (id, hash, decoded) in items {
                let current = state
                    .library
                    .item(&id)
                    .and_then(|item| item.asset.as_ref())
                    .map(|asset| asset.identity.hash.clone());
                if current.as_ref() != Some(&hash) {
                    stale.push(id);
                    continue;
                }
                state.set_decoded(&id, decoded);
                applied.push(id);
            }
            *took = !applied.is_empty();
            Ok(Some(
                json!({ "action": "media.prepare", "applied": applied, "stale": stale }),
            ))
        }
        Pending::SceneResources { map, session } => {
            let current = state
                .scenes
                .get(&map.scene_id)
                .map(|attached| attached.scene.bundle().revision_id.clone());
            if state.project.id != map.project_id
                || current.as_deref() != Some(&map.bundle_revision)
            {
                return Err(CommandError::new(
                    "stale_result",
                    format!(
                        "Scene `{}` or the open project changed while its resources loaded; nothing was installed",
                        map.scene_id
                    ),
                ));
            }
            let installed = session
                .install(*map)
                .map_err(|error| CommandError::new(error.code(), error.to_string()))?;
            *took = true;
            Ok(Some(
                json!({ "action": "scene.resources.resolve", "installed": true,
                "scene": installed.scene_id, "generation": installed.generation.0,
                "resources": installed.entries.len(), "complete": installed.complete,
                "approvedBytes": installed.approved_bytes() }),
            ))
        }
        Pending::SceneAttach {
            stamp,
            id,
            name,
            scene,
            resources,
        } => {
            if !stamp.is_current(&state.project) {
                return Err(CommandError::new(
                    "stale_result",
                    "The open project changed (an edit, or another project or a reopened document took its place) while the scene bundle was being read; nothing was attached (attach it again)",
                ));
            }
            let mut report = state.execute(Command::Scene(SceneCommand::Attach {
                id,
                name,
                source: SceneSource::Admitted(scene),
            }))?;
            report["resources"] = resources;
            *took = true;
            Ok(Some(report))
        }
        Pending::SceneImport(prepared) if decode_inline => {
            let report = scene_import::apply(state, *prepared)?;
            *took = true;
            Ok(Some(report))
        }
        Pending::SceneImport(prepared) => match scene_import::try_apply(state, *prepared) {
            Ok(report) => {
                *took = true;
                Ok(Some(report))
            }
            Err(rejected) => {
                let rejected = *rejected;
                let message = format!(
                    "{}; the files it copied are being removed by a background job",
                    rejected.error.message
                );
                // The refusal keeps its own code (the project said no, or the ledger would not
                // record the acceptance) and the job owns the files until it has removed them.
                Err(defer_discard(
                    registry,
                    id,
                    Pending::SceneImport(Box::new(rejected.prepared)),
                    message,
                    Some(rejected.error),
                ))
            }
        },
        Pending::SceneAdopt(pending) if decode_inline => {
            pending.apply(state)?;
            *took = true;
            Ok(None)
        }
        Pending::SceneAdopt(pending) => match pending.settle(state) {
            Ok(()) => {
                *took = true;
                Ok(None)
            }
            Err(stale) => {
                let stale = *stale;
                // A ledger refusal is not staleness: `claims_unavailable` with the ledger's details,
                // and a message that does not claim the scene changed (it did not).
                let refusal = stale.refusal_error();
                let message = match &refusal {
                    Some(refusal) => format!(
                        "The resources of scene `{}` were not confirmed: {}; the files this adoption copied are being removed by a background job",
                        stale.pending.scene(),
                        refusal.message
                    ),
                    None => format!(
                        "Scene `{}` changed while its resources were adopted ({}); the files this adoption copied are being removed by a background job",
                        stale.pending.scene(),
                        stale.why
                    ),
                };
                Err(defer_discard(
                    registry,
                    id,
                    Pending::SceneAdopt(Box::new(stale.pending)),
                    message,
                    refusal,
                ))
            }
        },
        Pending::Open {
            stamp,
            candidate,
            summary,
        } => {
            if !stamp.is_current(&state.project) {
                return Err(CommandError::new(
                    "stale_result",
                    "The open project changed while the package was restoring; the files were \
                     restored but not opened (open the restored document to use them)",
                ));
            }
            *state = *candidate;
            *took = true;
            Ok(Some(summary))
        }
    }
}

/// Gives a result that owns files to a discard job, **never** dropping it on the caller's thread.
///
/// A job whose slot is already filled but which is not final yet (the body returned, the
/// registry has not marked it finished: a window of microseconds) cannot be claimed by
/// `begin_discard`. The result then waits on a thread of its own for the job to end and is claimed
/// or dropped there, so a caller that must not block (the window's) never hashes or unlinks.
fn hand_over(registry: &JobRegistry, id: &str, pending: Pending) {
    match registry.begin_discard(id) {
        Ok(_) => {
            spawn_discard(registry, id, Some(pending));
        }
        Err(_) => {
            let (registry, id) = (registry.clone(), id.to_string());
            defer_until_final(registry.clone(), id.clone(), pending, move |pending| {
                if registry.begin_discard(&id).is_ok() {
                    spawn_discard(&registry, &id, Some(pending));
                }
                // Refused for good (nothing pending to claim): `pending` is dropped here, on this
                // helper thread, which is where a removal may block.
            });
        }
    }
}

/// Runs `then(payload)` on a thread of its own once job `id` is final (or gone from the registry).
/// `payload` is dropped there too if `then` does not consume it: never on the calling thread.
fn defer_until_final<T: Send + 'static>(
    registry: JobRegistry,
    id: String,
    payload: T,
    then: impl FnOnce(T) + Send + 'static,
) {
    std::thread::spawn(move || {
        while registry.wait(&id, Duration::from_secs(60)).is_none() && registry.get(&id).is_some() {
        }
        then(payload);
    });
}

impl MediaHost {
    /// Blocks until every discard job still queued or running has ended.
    ///
    /// A host without a window is driven by a script that ends with its input: the process must
    /// not leave while a `job.discard {wait:false}` is still taking files back (BL112). A job
    /// waits only for what has to run: discard jobs are never cancelled.
    ///
    /// The discard jobs queue behind the registry's running slots, so they are waited for behind
    /// whatever occupies those slots (`export.start`, a package, an adoption started with
    /// `wait:false`). Nothing else may outlive the process (their results are thrown away), so
    /// everything that is not a discard is cancelled **first** (`cancel_all` skips discards, which
    /// are not cancellable), and the wait is then for the discards and for the cancelled jobs to
    /// give their slots back; it no longer depends on a job that would run for as long as it
    /// likes (N2).
    fn wait_for_discards(&self) {
        let active = self
            .registry
            .matching(|info| info.kind == JobKind::Discard && !info.state.is_final());
        if active.is_empty() {
            return;
        }
        self.registry.cancel_all();
        for info in active {
            while self
                .registry
                .wait(&info.id, Duration::from_secs(60))
                .is_none()
                && self.registry.get(&info.id).is_some()
            {}
        }
    }
}

impl Drop for MediaHost {
    fn drop(&mut self) {
        // A host without a window waits for the discard jobs it started and drops what is left in
        // place, so the files are back before the CLI process exits.
        if !self.interactive {
            self.wait_for_discards();
            return;
        }
        // A window never deletes on its own thread: results that own files go to discard jobs,
        // which finish on their own threads.
        for (id, slot) in std::mem::take(&mut self.pending) {
            let Some(pending) = take(&slot).filter(Pending::owns_files) else {
                continue;
            };
            hand_over(&self.registry, &id, pending);
        }
    }
}

impl Extension for MediaHost {
    fn execute(
        &mut self,
        state: &mut EditorState,
        _renderer: Option<&mut dyn Renderer>,
        request: &Value,
    ) -> Option<Result<Value, CommandError>> {
        let op = request.get("op")?.as_str()?;
        // Whatever swapped the project (open, restore, a window), the next request notices.
        self.scene_resources.observe_project(&state.project.id);
        // A detached scene's resources are not the renderer's to keep; the core still runs it.
        if op == "scene.detach"
            && let Some(scene) = request.get("scene").and_then(Value::as_str)
        {
            self.scene_resources.release(scene);
        }
        // Rendering needs decoded media: prepare it first instead of failing with a hint to run
        // another command. The core still answers the request itself.
        if matches!(op, "render.frame" | "render.audio") {
            return self.ensure_prepared(state, op).err().map(Err);
        }
        let ours = op.starts_with("media.")
            || op.starts_with("job.")
            || op.starts_with("export.")
            || op.starts_with("scene.resources.")
            || op == "scene.export"
            || op == "scene.attach.start"
            || op == "scene.export.preflight"
            || matches!(
                op,
                "project.package_preflight"
                    | "project.package"
                    | "project.verify"
                    | "project.restore"
                    | "project.import"
                    | "project.import_scenes"
            );
        ours.then(|| self.handle(state, op, request))
    }
}

/// The job's own report plus what applying it did. A swapped-in project is also reported as
/// `opened`, the name `project.restore` has always used for it.
fn merge_applied(mut report: Value, id: &str, applied: Option<Value>) -> Value {
    report["job"] = json!(id);
    if let Some(applied) = applied {
        if applied["action"] == "project.open" {
            report["opened"] = applied.clone();
        }
        report["applied"] = applied;
    }
    report
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    /// Reports the thread it was dropped on.
    struct DroppedOn(mpsc::Sender<std::thread::ThreadId>);

    impl Drop for DroppedOn {
        fn drop(&mut self) {
            let _ = self.0.send(std::thread::current().id());
        }
    }

    /// F6 (BL112.2): a result whose job is not final yet cannot be claimed; it must wait on another
    /// thread, and be dropped there, not on the thread that let go of the host.
    #[test]
    fn a_result_of_a_job_that_is_not_final_is_never_dropped_on_the_callers_thread() {
        let registry = JobRegistry::new(2);
        let release = tessel_editor_core::jobs::CancelToken::new();
        let hold = release.clone();
        let job = registry.spawn(JobKind::Proxy, "not final yet", move |_| {
            while !hold.is_cancelled() {
                std::thread::sleep(Duration::from_millis(2));
            }
            Ok(Value::Null)
        });
        while registry.get(&job).unwrap().state != JobState::Running {
            std::thread::sleep(Duration::from_millis(2));
        }
        let (sender, dropped) = mpsc::channel();
        let me = std::thread::current().id();
        defer_until_final(
            registry.clone(),
            job.clone(),
            DroppedOn(sender),
            |_payload| {},
        );
        assert!(
            dropped.recv_timeout(Duration::from_millis(100)).is_err(),
            "nothing is dropped while the job is not final"
        );
        release.cancel();
        let thread = dropped
            .recv_timeout(Duration::from_secs(10))
            .expect("dropped once the job ended");
        assert_ne!(thread, me, "dropped on the helper thread");
    }

    /// The same path through the hand-over itself: refused by `begin_discard` (the job is not
    /// final), so the result goes to the waiting thread instead of the caller's scope.
    #[test]
    fn hand_over_defers_a_result_it_cannot_claim_yet() {
        let registry = JobRegistry::new(2);
        let release = tessel_editor_core::jobs::CancelToken::new();
        let hold = release.clone();
        let job = registry.spawn(JobKind::Proxy, "not final yet", move |ctx| {
            ctx.hold_result();
            while !hold.is_cancelled() {
                std::thread::sleep(Duration::from_millis(2));
            }
            Ok(Value::Null)
        });
        while registry.get(&job).unwrap().state != JobState::Running {
            std::thread::sleep(Duration::from_millis(2));
        }
        // `begin_discard` refuses a running job (`job_running`): this is the case being covered.
        assert_eq!(
            registry.begin_discard(&job).unwrap_err().code,
            "job_running"
        );
        hand_over(&registry, &job, Pending::Relink(Vec::new()));
        assert_eq!(
            registry.get(&job).unwrap().result_state,
            tessel_editor_core::jobs::ResultState::Pending,
            "nothing claimed it while the job ran"
        );
        release.cancel();
        // Once the job is final the waiting thread claims the result and a discard job ends it.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while registry.get(&job).unwrap().result_state
            != tessel_editor_core::jobs::ResultState::Discarded
        {
            assert!(
                std::time::Instant::now() < deadline,
                "the result was never handed to a discard job"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            registry
                .matching(|info| info.kind == JobKind::Discard)
                .len(),
            1,
            "exactly one discard job took the result"
        );
    }
}
