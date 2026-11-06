//! The operations of the media adapters, exposed through the core's [`Extension`] hook.
//!
//! One `MediaHost` per session serves the JSON Lines CLIs (`./scripts/headless`,
//! `./scripts/render`) and the desktop `ui op` channel, so a gesture, a script and a library
//! call reach the same code. Heavy work runs as jobs; operations that change the project
//! prepare their files on the job thread and apply the result to the project in one
//! transaction on the caller's thread (`wait: true`, the default, does both in one call;
//! `wait: false` returns a job ID and `job.commit` applies it when the job succeeded).
//!
//! Operations: `media.import|relink|search|status|prepare|thumbnail|waveform|proxy`,
//! `project.package_preflight|package|verify|restore|import|import_scenes`,
//! `scene.resources.status|preflight|resolve|adopt|release|invalidate|claims|recover` (F040),
//! `export.preflight|start`, `scene.export[.preflight]` (F042: a directed scene as video, same jobs),
//! `job.list|get|wait|cancel|commit`. Presets (`preset.*`) belong to the core.
//!
//! Anything that reads files, runs a tool or builds a renderer (probe, decode, verify, restore,
//! preflight) is a job; the caller's thread only snapshots the project and later applies the
//! result. A result is applied only if the project it was computed for is still the open one
//! (same ID, same revision for whole-project swaps, same content hash per asset for decoded
//! media), so a slow job can never overwrite what the user did meanwhile.
//!
//! Media is prepared automatically: export and the render CLIs decode what they need, and an
//! interactive host schedules a "Prepare media" job (`MediaHost::poll`) for every asset that lacks
//! decoded frames/audio, so nobody has to know the `media.prepare` command.

use std::collections::HashMap;
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
    JobContext, JobError, JobId, JobInfo, JobKind, JobRegistry, JobState,
};
use tessel_editor_core::preset::{self, Preset};
use tessel_editor_core::preview::{PreviewPolicy, Reason};
use tessel_editor_core::scene_export::{SceneExportSnapshot, SceneExportSpec};
use tessel_editor_core::scene_resources::ResourceManifest;
use tessel_editor_core::session::Renderer;
use tessel_editor_core::state::Command;
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
    /// Files an adoption copied, kept only while the project, its workspace and the scene's
    /// bundle are still the ones the job was started for; otherwise removed (BL089). Changes no
    /// project data. Dropped unapplied (host gone, job never committed) it removes them too,
    /// except what another pending or accepted result relies on (BL096).
    SceneAdopt(Box<PendingAdoption>),
    /// A restored project, already loaded and validated off the caller's thread, that replaces the
    /// open project only if that one is still at `(project, revision)`.
    Open {
        project: String,
        revision: u64,
        candidate: Box<EditorState>,
        summary: Value,
    },
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
    /// `(asset, hash, height, format)` whose proxy failed: not retried until one of them changes.
    proxy_failed: Vec<ProxyKey>,
    proxy_assets: HashMap<JobId, Vec<ProxyKey>>,
    /// Approved scene resources of the open project (BL010). Shared with jobs and, through
    /// [`MediaHost::scene_resources`], with a renderer.
    scene_resources: SceneResourceSession,
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
            pending: HashMap::new(),
            interactive: false,
            auto_prepare: None,
            auto_failed: Vec::new(),
            auto_assets: HashMap::new(),
            auto_proxy: None,
            proxy_failed: Vec::new(),
            proxy_assets: HashMap::new(),
            scene_resources: SceneResourceSession::default(),
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
            let proxied = self.proxy_assets.remove(&id);
            let automatic = self.auto_assets.remove(&id);
            let outcome = Self::outcome(&info).and_then(|result| {
                let applied = apply(
                    &self.registry.clone(),
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
        self.schedule_prepare(state);
        self.schedule_proxies(state);
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
            *target
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = pending;
            Ok(report)
        });
        self.pending.insert(id.clone(), slot.clone());
        (id, slot)
    }

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
        let applied = apply(&self.registry.clone(), take(&slot), state, true)?;
        Ok(merge_applied(result, &id, applied))
    }

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
            "media.thumbnail" | "media.waveform" | "media.proxy" => self.derive(state, op, request),
            "project.package_preflight" => self.package_preflight(state, request),
            "project.package" => self.package(state, request),
            "project.verify" => self.verify(state, request),
            "project.restore" => self.restore(state, request),
            "project.import" => self.project_import(state, request),
            "project.import_scenes" => self.import_scenes(state, request),
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
        let prepare = flag(request, "prepare", true);
        let known = import::known_hashes(state);
        let import_request = ImportRequest {
            paths,
            mode,
            folder: folder.clone(),
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
        apply(&self.registry.clone(), take(&slot), state, true)?;
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
        let (project, revision) = (state.project.id.clone(), state.project.revision);
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
                    project,
                    revision,
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
        let base = Self::scene_base(state, request, &id)?;
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
        let base = Self::scene_base(state, request, &id)?;
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

    fn job_get(&mut self, request: &Value) -> Result<Value, CommandError> {
        let id = text(request, "job")?;
        let info = self
            .registry
            .get(id)
            .ok_or_else(|| CommandError::unknown_element("job", id))?;
        Ok(info.to_json())
    }

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
        let result = Self::outcome(&info)?;
        let slot = self.pending.remove(id).ok_or_else(|| {
            CommandError::new(
                "job_finished",
                format!("Job `{id}` has nothing to apply or was already applied"),
            )
        })?;
        let applied = apply(
            &self.registry.clone(),
            take(&slot),
            state,
            !self.interactive,
        )?;
        Ok(merge_applied(result, id, applied))
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

fn take(slot: &Slot) -> Option<Pending> {
    slot.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
}

/// The single place a job result changes the project: always one transaction (or one command),
/// and only for the project the job was computed for. `decode_inline` is for callers that may
/// block (the CLIs): an import then also decodes its new media before answering; a window leaves
/// that to the automatic preparation job.
fn apply(
    registry: &JobRegistry,
    pending: Option<Pending>,
    state: &mut EditorState,
    decode_inline: bool,
) -> Result<Option<Value>, CommandError> {
    let Some(pending) = pending else {
        return Ok(None);
    };
    match pending {
        Pending::Import {
            items,
            folder,
            prepare: decode,
        } => {
            let mut report = import::commit(state, &items, folder.as_deref())?;
            if decode
                && decode_inline
                && let Ok(workspace) = workspace(state)
            {
                let cache = Cache::new(&workspace);
                let tools = Tools::discover()?;
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
                outcome.map_err(CommandError::from)?;
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
            Ok(Some(
                json!({ "action": "media.preview_proxy", "applied": applied, "stale": stale }),
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
            Ok(Some(
                json!({ "action": "scene.resources.resolve", "installed": true,
                "scene": installed.scene_id, "generation": installed.generation.0,
                "resources": installed.entries.len(), "complete": installed.complete,
                "approvedBytes": installed.approved_bytes() }),
            ))
        }
        Pending::SceneImport(prepared) => Ok(Some(scene_import::apply(state, *prepared)?)),
        Pending::SceneAdopt(pending) => {
            pending.apply(state)?;
            Ok(None)
        }
        Pending::Open {
            project,
            revision,
            candidate,
            summary,
        } => {
            if state.project.id != project || state.project.revision != revision {
                return Err(CommandError::new(
                    "stale_result",
                    "The open project changed while the package was restoring; the files were \
                     restored but not opened (open the restored document to use them)",
                ));
            }
            *state = *candidate;
            Ok(Some(summary))
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
