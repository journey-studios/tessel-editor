//! File, import, package and export flows of the window, and the jobs behind them.
//!
//! Every button, dialog and automation command here ends in a JSON operation run through the same
//! [`MediaHost`] the CLIs use, so the window has no second rule set. The host is *interactive*:
//! nothing blocks the UI thread, an operation answers with a job and [`Workstation::workflow_tick`]
//! applies what finished (one transaction per job). A dialog is a thin form over
//! [`super::flows`]; its fields, report and buttons are also nodes of the automation tree
//! (`flow.*` opens, `dialog.*` fills, submits or cancels), so tests never click coordinates.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::component::button::Button;
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::searchable_list::SearchableVec;
use gpui_kit::component::select::{Select, SelectState};
use gpui_kit::component::{ActiveTheme, Sizable, WindowExt as _, h_flex, v_flex};
use gpui_kit::*;
use serde_json::{Value, json};
use tessel_editor_core::headless::{Extension, run_project_op};
use tessel_editor_core::jobs::{JobError, JobInfo, ResultState};
use tessel_editor_core::ops::ProjectOp;
use tessel_editor_core::session::Renderer;
use tessel_editor_core::{Command, CommandError};
use tessel_editor_media::host::MediaHost;
use tessel_editor_render::{RuntimeConfig, session};

use super::flows::{self, FieldSpec, FlowKind, Picker};
use super::scene_import::SceneImport;
use super::{Activity, Source, Workstation};

/// How often finished jobs are applied and autosave is considered.
const TICK: Duration = Duration::from_millis(250);
/// A dirty project is autosaved at most this often; a crash loses at most this much work.
const AUTOSAVE_EVERY: Duration = Duration::from_secs(20);

type ChoiceState = Entity<SelectState<SearchableVec<String>>>;

struct FlowField {
    spec: FieldSpec,
    input: Option<Entity<InputState>>,
    choice: Option<ChoiceState>,
    /// What the choice offers (the spec's options plus the project's own presets).
    options: Vec<String>,
}

/// The dialog that is open, if any. Its entities live here, not in the dialog builder, which may
/// run again whenever the dialog layer is rebuilt.
pub struct FlowDialog {
    kind: FlowKind,
    fields: Vec<FlowField>,
    /// Preflight or recovery listing the user can refresh.
    report: Option<String>,
    error: Option<String>,
    /// The job that is computing `report` (preflight and verification run off the UI thread);
    /// `workflow_tick` fills the report when it finishes. A newer request replaces it, so an
    /// older answer is never shown for fields the user has since changed.
    report_job: Option<String>,
}

pub struct Workflow {
    host: MediaHost,
    pub(super) flow: Option<FlowDialog>,
    /// The jobs dialog has no state of its own; this only tells the monitor to step aside.
    jobs_open: bool,
    /// Newest first; what the status bar and `jobs` show about jobs that already finished.
    history: Vec<Value>,
    last_autosave: Instant,
    autosaved_revision: Option<u64>,
    /// The last scene attach/load chain (F043, `scene_import.rs`); its jobs finish on the tick.
    pub(super) scene_import: Option<SceneImport>,
    /// Discard job -> the refusal that sent a result to it (`details.discard_job` of a failed
    /// event). The discard's own event must not hide why the result was refused (F5).
    refusals: HashMap<String, String>,
}

impl Workflow {
    /// What the status bar says about `event`, given the refusals seen so far (F5).
    ///
    /// A result the project refused (`stale_result`, `claims_unavailable`) names the discard job
    /// that now owns its files in `error.details.discard_job`; the refusal is remembered. When
    /// that discard job ends, its line must not replace the reason: a clean discard keeps saying
    /// why the result was refused, and one that left something behind says both.
    pub(super) fn remember_refusal(
        &mut self,
        event: &Value,
        text: String,
        error: bool,
    ) -> (String, bool) {
        // `job_discarded` is the answer to an explicit `job.discard`, not a refusal.
        if let Some(discard) = event["error"]["details"]["discard_job"].as_str()
            && event["kind"] != "discard"
            && event["error"]["code"] != "job_discarded"
        {
            self.refusals.insert(discard.to_string(), text.clone());
            return (text, error);
        }
        if event["kind"] != "discard" {
            return (text, error);
        }
        let Some(refusal) = event["job"]
            .as_str()
            .and_then(|job| self.refusals.remove(job))
        else {
            return (text, error);
        };
        if error {
            (format!("{refusal}; {text}"), true)
        } else {
            (refusal, true)
        }
    }

    pub fn new() -> Self {
        // WHY a factory: an export drives its own headless renderer on its own thread, so it
        // never shares GPU state or frame timing with the live monitor's WebView.
        let renderer: Arc<tessel_editor_media::host::RendererFactory> = Arc::new(|| {
            let config = RuntimeConfig::discover()
                .map_err(|setup| JobError::new("backend_unavailable", setup.message()))?;
            let renderer: Box<dyn Renderer> = Box::new(session(&config));
            Ok(renderer)
        });
        let scene_renderer: Arc<tessel_editor_media::scene_encoder::SceneRendererFactory> =
            Arc::new(|| {
                let config = RuntimeConfig::discover()
                    .map_err(|setup| JobError::new("backend_unavailable", setup.message()))?;
                Ok(
                    Box::new(tessel_editor_render::scene_export::ProcessSceneRenderer::new(config))
                        as Box<dyn tessel_editor_media::scene_encoder::SceneFrameRenderer>,
                )
            });
        Self {
            host: MediaHost::new()
                .with_renderer(renderer)
                .with_scene_renderer(scene_renderer)
                .interactive(),
            flow: None,
            jobs_open: false,
            history: Vec::new(),
            last_autosave: Instant::now(),
            autosaved_revision: None,
            scene_import: None,
            refusals: HashMap::new(),
        }
    }

    /// A native WebView paints above every GPUI element, so any open dialog must hide it.
    pub(super) fn scene_resources(
        &self,
    ) -> tessel_editor_media::scene_resources::SceneResourceSession {
        self.host.scene_resources()
    }

    pub fn has_dialog(&self) -> bool {
        self.flow.is_some() || self.jobs_open
    }

    fn active_jobs(&self) -> Vec<JobInfo> {
        self.host
            .registry()
            .list()
            .into_iter()
            .filter(|job| !job.state.is_final())
            .collect()
    }

    /// "Exportação 42%" style line for the status bar; `None` while idle.
    pub fn jobs_line(&self) -> Option<String> {
        let active = self.active_jobs();
        let first = active.first()?;
        let progress = first
            .progress()
            .map_or(String::new(), |p| format!(" {:.0}%", p * 100.0));
        let more = if active.len() > 1 {
            format!(" (+{})", active.len() - 1)
        } else {
            String::new()
        };
        Some(format!("{}{progress}{more}", first.label))
    }
}

impl Workstation {
    // ---- operations outside the core vocabulary (media.*, export.*, job.*, project.package…) ----

    /// Runs `request` on the media host when it is one of its operations.
    pub(super) fn run_media_op(
        &mut self,
        request: &Value,
        cx: &mut Context<Self>,
    ) -> Option<Result<Value, CommandError>> {
        let op = request["op"].as_str().unwrap_or_default().to_string();
        let result =
            self.workflow
                .host
                .execute(&mut self.state, None::<&mut dyn Renderer>, request)?;
        if result.is_ok() {
            // A restore that replaces the project is applied later, by `workflow_tick`, which
            // resets the viewport then; here only the immediate edits are followed.
            self.viewport_sync(cx);
        }
        self.note_result(&op, &result, Source::Cli, cx);
        Some(result)
    }

    fn note_result(
        &mut self,
        op: &str,
        result: &Result<Value, CommandError>,
        source: Source,
        cx: &mut Context<Self>,
    ) {
        self.activity = Some(match result {
            Ok(_) => Activity {
                source,
                text: op.to_string(),
                error: false,
            },
            Err(error) => Activity {
                source,
                text: error.to_string(),
                error: true,
            },
        });
        cx.notify();
    }

    /// Applies finished jobs, reports them in the status bar and autosaves a dirty project.
    pub(super) fn workflow_tick(&mut self, cx: &mut Context<Self>) {
        let events = self.workflow.host.poll(&mut self.state);
        if !events.is_empty() {
            // A job that opened another project (restore) invalidates what the viewport loaded.
            let replaced = events
                .iter()
                .any(|event| event["applied"]["action"] == "project.open");
            if replaced {
                self.viewport_reset_project(cx);
            } else {
                self.viewport_sync(cx);
            }
            for event in events {
                self.flow_receive_report(&event);
                let (text, error) = flows::describe_job_event(&event);
                let (text, error) = self.workflow.remember_refusal(&event, text, error);
                self.activity = Some(Activity {
                    source: Source::Cli,
                    text,
                    error,
                });
                // After the generic line above: a chain that finished says its own, more specific one.
                self.scene_import_receive(&event, cx);
                self.workflow.history.insert(0, event);
                self.workflow.history.truncate(20);
            }
        }
        let revision = self.state.project.revision;
        let due = self.workflow.last_autosave.elapsed() >= AUTOSAVE_EVERY
            && self.state.project.is_dirty()
            && self.workflow.autosaved_revision != Some(revision);
        if due {
            self.workflow.last_autosave = Instant::now();
            // A project without a workspace has nowhere to write recovery data: that is not an
            // error worth showing every 20 seconds, and an explicit save creates the folder.
            if run_project_op(&mut self.state, None, ProjectOp::Autosave).is_ok() {
                self.workflow.autosaved_revision = Some(revision);
            }
        }
        cx.notify();
    }

    pub(super) fn spawn_workflow_tick(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                if this.update(cx, |this, cx| this.workflow_tick(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    // ---- gestures that need no dialog ----

    pub fn undo(&mut self, cx: &mut Context<Self>) -> Result<Value, CommandError> {
        self.execute(Command::Undo, Source::Gesture, cx)
    }

    pub fn redo(&mut self, cx: &mut Context<Self>) -> Result<Value, CommandError> {
        self.execute(Command::Redo, Source::Gesture, cx)
    }

    /// Saves to the project's document; a project that was never saved asks where first.
    pub fn save_project(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        if self.state.project.document.is_none() {
            return self.open_flow(FlowKind::SaveAs, window, cx);
        }
        let result = run_project_op(&mut self.state, None, ProjectOp::Save { path: None });
        self.note_result("project.save", &result, Source::Gesture, cx);
        result
    }

    pub fn jobs_report(&self) -> Value {
        json!({
            "active": self.workflow.active_jobs().iter().map(JobInfo::to_json).collect::<Vec<_>>(),
            "recent": self.workflow.history,
            "all": self.workflow.host.registry().to_json()["jobs"],
        })
    }

    // ---- dialogs ----

    pub fn open_flow(
        &mut self,
        kind: FlowKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        let user_presets: Vec<String> = self
            .state
            .project
            .user_presets
            .iter()
            .filter_map(|preset| preset["id"].as_str().map(str::to_string))
            .collect();
        let selected = self.state.project.export_preset.clone();
        let default_name = self.state.project_name.replace(['/', ':'], "-");
        let fields = kind
            .fields()
            .iter()
            .map(|spec| {
                if spec.options.is_empty() {
                    let input =
                        cx.new(|cx| InputState::new(window, cx).placeholder(spec.placeholder));
                    return FlowField {
                        spec: *spec,
                        input: Some(input),
                        choice: None,
                        options: Vec::new(),
                    };
                }
                let mut options: Vec<String> = spec
                    .options
                    .iter()
                    .map(|option| option.to_string())
                    .collect();
                if spec.name == "preset" {
                    options.extend(user_presets.iter().cloned());
                }
                let initial = selected
                    .as_ref()
                    .filter(|_| spec.name == "preset")
                    .and_then(|id| options.iter().position(|option| option == id))
                    .unwrap_or(0);
                let items = options.clone();
                let choice = cx.new(|cx| {
                    SelectState::new(
                        SearchableVec::new(items),
                        Some(gpui_kit::component::IndexPath::new(initial)),
                        window,
                        cx,
                    )
                });
                FlowField {
                    spec: *spec,
                    input: None,
                    choice: Some(choice),
                    options,
                }
            })
            .collect();
        self.workflow.flow = Some(FlowDialog {
            kind,
            fields,
            report: None,
            error: None,
            report_job: None,
        });
        // Suggested destinations, so the common case is "open the dialog, press the button".
        let base = self
            .state
            .project
            .workspace
            .clone()
            .unwrap_or_else(std::env::temp_dir);
        match kind {
            FlowKind::Export => self.flow_prefill(
                "destination",
                &base.join(format!("{default_name}.mp4")).to_string_lossy(),
                window,
                cx,
            ),
            FlowKind::Package => self.flow_prefill(
                "path",
                &base
                    .join(format!("{default_name}.tesselpack"))
                    .to_string_lossy(),
                window,
                cx,
            ),
            FlowKind::SaveAs => self.flow_prefill(
                "path",
                &base
                    .join(format!("{default_name}.tessel.json"))
                    .to_string_lossy(),
                window,
                cx,
            ),
            // The scene a resources dialog most likely means: the one the inspector follows.
            FlowKind::ExportScene => {
                if let Some(scene) = self.scene_active_id() {
                    self.flow_prefill("scene", &scene, window, cx);
                    self.flow_prefill(
                        "destination",
                        &base.join(format!("{scene}.mp4")).to_string_lossy(),
                        window,
                        cx,
                    );
                }
            }
            FlowKind::SceneResources => {
                if let Some(scene) = self.scene_active_id() {
                    self.flow_prefill("scene", &scene, window, cx);
                }
            }
            _ => {}
        }
        if let Some(document) = self.state.project.document.clone()
            && matches!(kind, FlowKind::Recover)
        {
            self.flow_prefill("path", &document.to_string_lossy(), window, cx);
        }

        let this = cx.entity();
        let view = cx.new(|cx| FlowView::new(this.clone(), cx));
        let title = kind.title();
        let submit = kind.submit_label();
        window.open_dialog(cx, move |dialog, _, _| {
            let on_ok = this.clone();
            let on_close = this.clone();
            dialog
                .title(title)
                .w(px(560.))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(submit)
                        .cancel_text("Cancelar")
                        .show_cancel(true),
                )
                .child(view.clone())
                .on_ok(move |_, window, cx| {
                    on_ok.update(cx, |workstation, cx| {
                        workstation.flow_submit(window, cx).is_ok()
                    })
                })
                .on_close(move |_, _, cx| {
                    on_close.update(cx, |workstation, cx| {
                        workstation.workflow.flow = None;
                        cx.notify();
                    });
                })
        });
        self.flow_verify(window, cx);
        cx.notify();
        Ok(json!({ "action": "flow.open", "flow": kind.id(), "changed": true }))
    }

    fn flow_prefill(
        &mut self,
        name: &str,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = self.flow_set(name, value, window, cx);
    }

    fn flow_values(&self, cx: &App) -> Vec<(&'static str, String)> {
        let Some(flow) = &self.workflow.flow else {
            return Vec::new();
        };
        flow.fields
            .iter()
            .map(|field| {
                let value = match (&field.input, &field.choice) {
                    (Some(input), _) => input.read(cx).value().to_string(),
                    (_, Some(choice)) => choice
                        .read(cx)
                        .selected_value()
                        .cloned()
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                (field.spec.name, value)
            })
            .collect()
    }

    pub fn flow_set(
        &mut self,
        name: &str,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        let flow = self
            .workflow
            .flow
            .as_ref()
            .ok_or_else(|| CommandError::new("no_dialog", "No dialog is open"))?;
        let field = flow
            .fields
            .iter()
            .find(|field| field.spec.name == name)
            .ok_or_else(|| CommandError::unknown_element("dialog field", name))?;
        if let Some(input) = &field.input {
            input.update(cx, |input, cx| {
                input.set_value(value.to_string(), window, cx)
            });
        } else if let Some(choice) = &field.choice
            && !field.spec.options.is_empty()
            && !choice
                .read(cx)
                .selected_value()
                .is_some_and(|current| current == value)
        {
            let known = choice.update(cx, |select, cx| {
                select.set_selected_value(&value.to_string(), window, cx);
                select
                    .selected_value()
                    .is_some_and(|current| current == value)
            });
            if !known {
                return Err(CommandError::new(
                    "invalid_value",
                    format!("`{value}` is not an option of `{name}`"),
                ));
            }
        }
        if let Some(flow) = &mut self.workflow.flow {
            flow.error = None;
        }
        cx.notify();
        Ok(json!({ "target": format!("dialog.field.{name}"), "value": value, "changed": true }))
    }

    /// Fills the dialog's report (preflight / recovery listing) from the same read-only
    /// operations the CLI offers.
    pub fn flow_verify(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(kind) = self.workflow.flow.as_ref().map(|flow| flow.kind) else {
            return;
        };
        let Some(request) = flows::report_request(kind, &self.flow_values(cx)) else {
            return;
        };
        self.flow_cancel_report();
        let outcome = self.run_media_op(&request, cx).or_else(|| {
            let op = tessel_editor_core::ops::parse_op(&self.state, &request)
                .ok()
                .flatten()?;
            match op {
                tessel_editor_core::ops::Op::Project(project) => {
                    Some(run_project_op(&mut self.state, None, project))
                }
                _ => None,
            }
        });
        if let Some(flow) = &mut self.workflow.flow {
            match outcome {
                // The media host answers with a job: the report arrives through the tick.
                Some(Ok(answer))
                    if answer["job"].is_string() && answer.get("blockers").is_none() =>
                {
                    flow.report_job = answer["job"].as_str().map(str::to_string);
                    flow.report = Some(flows::VERIFYING.to_string());
                    flow.error = None;
                }
                Some(Ok(report)) => {
                    flow.report_job = None;
                    flow.report = Some(flows::describe_report(&report));
                    flow.error = None;
                }
                Some(Err(error)) => {
                    flow.report_job = None;
                    flow.error = Some(error.message);
                }
                None => {}
            }
        }
        cx.notify();
    }

    /// Stops the report job of the open dialog, if one is still running.
    fn flow_cancel_report(&mut self) {
        let Some(id) = self
            .workflow
            .flow
            .as_mut()
            .and_then(|flow| flow.report_job.take())
        else {
            return;
        };
        // Already finished is fine: its event no longer matches any dialog report.
        let _ = self.workflow.host.registry().cancel(&id);
    }

    /// Shows the answer of the dialog's report job when `event` is that job finishing.
    fn flow_receive_report(&mut self, event: &Value) {
        let Some(flow) = &mut self.workflow.flow else {
            return;
        };
        if flow.report_job.as_deref() != event["job"].as_str() {
            return;
        }
        flow.report_job = None;
        if event["state"] == "succeeded" {
            flow.report = Some(flows::describe_report(&event["result"]));
            flow.error = None;
            return;
        }
        flow.report = None;
        flow.error = Some(
            event["error"]["message"]
                .as_str()
                .unwrap_or("A verificação falhou")
                .to_string(),
        );
    }

    /// Sends the dialog's operation. `Ok` closes the dialog; an error stays in it, in words.
    pub fn flow_submit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        let kind = self
            .workflow
            .flow
            .as_ref()
            .map(|flow| flow.kind)
            .ok_or_else(|| CommandError::new("no_dialog", "No dialog is open"))?;
        let values = self.flow_values(cx);
        let outcome = if kind.is_scene_import() {
            // A chain of jobs rather than one operation: see `scene_import.rs`.
            self.scene_import_submit(kind, &values, cx)
        } else {
            flows::build_request(kind, &values)
                .and_then(|request| self.run_flow_request(&request, window, cx))
        };
        match &outcome {
            Ok(_) => {
                self.workflow.flow = None;
                self.focus_root(window, cx);
                window.refresh();
            }
            Err(error) => {
                if let Some(flow) = &mut self.workflow.flow {
                    flow.error = Some(error.message.clone());
                }
            }
        }
        self.activity = Some(match &outcome {
            Ok(_) => Activity {
                source: Source::Gesture,
                text: kind.title().to_string(),
                error: false,
            },
            Err(error) => Activity {
                source: Source::Gesture,
                text: error.to_string(),
                error: true,
            },
        });
        cx.notify();
        outcome
    }

    fn run_flow_request(
        &mut self,
        request: &Value,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        // `run_op` tries the media host first, then the core's vocabulary.
        match self.run_op(request, cx)? {
            super::OpReply::Done(value) => Ok(value),
            super::OpReply::Pending(_) => Ok(Value::Null),
        }
    }

    pub fn close_flow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.flow_cancel_report();
        self.workflow.flow = None;
        window.close_dialog(cx);
        // Escape and the keyboard shortcuts only reach the workstation while it has focus.
        self.focus_root(window, cx);
        cx.notify();
    }

    /// The native picker next to a field: it only fills the field; the operation still runs
    /// through the dialog's button, so a picked path is validated like a typed one.
    fn choose_path(
        &mut self,
        name: &'static str,
        picker: Picker,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let options = |files: bool, directories: bool, multiple: bool| PathPromptOptions {
            files,
            directories,
            multiple,
            prompt: None,
        };
        let fill = |this: &mut Workstation,
                    paths: Vec<std::path::PathBuf>,
                    window: &mut Window,
                    cx: &mut Context<Workstation>| {
            let text = paths
                .iter()
                .map(|path| path.to_string_lossy())
                .collect::<Vec<_>>()
                .join(";");
            let _ = this.flow_set(name, &text, window, cx);
            this.flow_verify(window, cx);
        };
        match picker {
            Picker::None => {}
            Picker::File | Picker::Files | Picker::Directory => {
                let receiver = cx.prompt_for_paths(match picker {
                    Picker::Directory => options(false, true, false),
                    _ => options(true, false, picker == Picker::Files),
                });
                cx.spawn_in(window, async move |this, cx| {
                    let Some(paths) = receiver.await.ok().and_then(Result::ok).flatten() else {
                        return;
                    };
                    let _ = this.update_in(cx, |this, window, cx| fill(this, paths, window, cx));
                })
                .detach();
            }
            Picker::NewFile(suggested) => {
                let base = self
                    .state
                    .project
                    .workspace
                    .clone()
                    .unwrap_or_else(std::env::temp_dir);
                let receiver = cx.prompt_for_new_path(&base, Some(suggested));
                cx.spawn_in(window, async move |this, cx| {
                    let Some(path) = receiver.await.ok().and_then(Result::ok).flatten() else {
                        return;
                    };
                    let _ =
                        this.update_in(cx, |this, window, cx| fill(this, vec![path], window, cx));
                })
                .detach();
            }
        }
    }

    // ---- jobs dialog ----

    pub fn open_jobs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        let this = cx.entity();
        let view = cx.new(|cx| JobsView::new(this.clone(), cx));
        self.workflow.jobs_open = true;
        let closing = this.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let closing = closing.clone();
            dialog
                .title("Tarefas")
                .w(px(560.))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Fechar")
                        .show_cancel(false),
                )
                .child(view.clone())
                .on_close(move |_, window, cx| {
                    // The monitor's WebView comes back with the next render.
                    closing.update(cx, |workstation, cx| {
                        workstation.workflow.jobs_open = false;
                        cx.notify();
                    });
                    window.refresh();
                })
        });
        Ok(json!({ "action": "flow.jobs", "changed": true }))
    }

    pub fn cancel_job(&mut self, id: &str, cx: &mut Context<Self>) -> Result<Value, CommandError> {
        let info = self.workflow.host.registry().cancel(id)?;
        cx.notify();
        Ok(json!({ "action": "job.cancel", "info": info.to_json() }))
    }

    // ---- automation: flows, dialog fields, jobs ----

    /// Window-only nodes: the flows, the open dialog with its fields and buttons, and the jobs.
    pub fn workflow_nodes(&self, cx: &App) -> Vec<Value> {
        let mut nodes = flows::nodes();
        nodes.push(json!({
            "id": "flow.jobs", "kind": "button", "label": "Tarefas", "parent": "region:header",
            "children": [], "enabled": true, "visible": true, "fields": [],
            "actions": [{ "action": "open", "label": "Tarefas" }], "source": "workflow.rs",
        }));
        nodes.push(json!({
            "id": "jobs", "kind": "list", "label": "Tarefas em segundo plano", "parent": "region:status",
            "children": [], "enabled": true, "visible": true, "fields": [], "actions": [],
            "state": self.jobs_report(),
        }));
        nodes.extend(self.scene_import_nodes());
        let Some(flow) = &self.workflow.flow else {
            return nodes;
        };
        let values = self.flow_values(cx);
        let mut children: Vec<String> = Vec::new();
        for field in &flow.fields {
            let id = format!("dialog.field.{}", field.spec.name);
            children.push(id.clone());
            let value = values
                .iter()
                .find(|(name, _)| *name == field.spec.name)
                .map(|(_, value)| value.clone())
                .unwrap_or_default();
            let mut entry = json!({ "field": "value", "label": field.spec.label, "value": value });
            if field.choice.is_some() {
                entry["options"] = json!(field.options);
            }
            nodes.push(json!({
                "id": id, "kind": "input", "label": field.spec.label, "parent": "dialog",
                "children": [], "enabled": true, "visible": true, "fields": [entry], "actions": [],
                "value": value,
            }));
        }
        for (id, label) in [
            ("dialog.submit", flow.kind.submit_label()),
            ("dialog.cancel", "Cancelar"),
        ]
        .into_iter()
        .chain(
            flow.kind
                .has_report()
                .then_some(("dialog.verify", "Verificar")),
        ) {
            children.push(id.to_string());
            nodes.push(json!({
                "id": id, "kind": "button", "label": label, "parent": "dialog", "children": [],
                "enabled": true, "visible": true, "fields": [],
                "actions": [{ "action": "invoke", "label": label }],
            }));
        }
        nodes.push(json!({
            "id": "dialog", "kind": "dialog", "label": flow.kind.title(), "parent": "region:header",
            "children": children, "enabled": true, "visible": true, "fields": [], "actions": [],
            "state": { "flow": flow.kind.id(), "report": flow.report, "error": flow.error, "verifying": flow.report_job.is_some() },
        }));
        nodes
    }

    /// `invoke` on a flow, a dialog button, an undo/redo/save/jobs header button. `None` when the
    /// target is not one of these, so the caller falls through to the core's tree.
    pub fn workflow_invoke(
        &mut self,
        target: &str,
        action: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Result<Value, CommandError>> {
        let flow = target.strip_prefix("flow.").and_then(|id| match id {
            "save" => None,
            "jobs" => None,
            other => FlowKind::parse(other),
        });
        let outcome = match (target, flow) {
            (_, Some(kind)) => self.open_flow(kind, window, cx),
            ("flow.save" | "node:19", _) => self.save_project(window, cx),
            ("flow.jobs" | "node:21", _) => self.open_jobs(window, cx),
            ("monitor.play", _) => self.playback_toggle(cx),
            ("node:9", _) => self.undo(cx),
            ("node:11", _) => self.redo(cx),
            ("node:14", _) => self.open_flow(FlowKind::Import, window, cx),
            ("node:16", _) => {
                let kind = if self.scene_ui.mode == super::scene_ui::InspectorMode::Scene {
                    FlowKind::ExportScene
                } else {
                    FlowKind::Export
                };
                self.open_flow(kind, window, cx)
            }
            ("node:18", _) => self.open_flow(FlowKind::Open, window, cx),
            ("node:20", _) => self.open_flow(FlowKind::Package, window, cx),
            ("node:26", _) => self.open_flow(FlowKind::AttachScene, window, cx),
            ("dialog.submit", _) => self
                .flow_submit(window, cx)
                .inspect(|_| window.close_dialog(cx)),
            ("dialog.cancel", _) => {
                self.close_flow(window, cx);
                Ok(json!({ "action": "dialog.cancel", "changed": true }))
            }
            ("dialog.verify", _) => {
                self.flow_verify(window, cx);
                Ok(
                    json!({ "action": "dialog.verify", "report": self.workflow.flow.as_ref().and_then(|flow| flow.report.clone()),
                    "error": self.workflow.flow.as_ref().and_then(|flow| flow.error.clone()) }),
                )
            }
            _ => return None,
        };
        let _ = action;
        Some(outcome)
    }

    pub fn workflow_set(
        &mut self,
        target: &str,
        field: &str,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Result<Value, CommandError>> {
        let name = target.strip_prefix("dialog.field.")?;
        if field != "value" {
            return Some(Err(CommandError::new(
                "unknown_field",
                format!("`{target}` has no settable field `{field}`"),
            )));
        }
        Some(self.flow_set(name, value, window, cx))
    }
}

// ---- dialog bodies ----

struct FlowView {
    workstation: Entity<Workstation>,
}

impl FlowView {
    fn new(workstation: Entity<Workstation>, cx: &mut Context<Self>) -> Self {
        // The report and error change while the dialog is open; follow the workstation.
        cx.observe(&workstation, |_, _, cx| cx.notify()).detach();
        Self { workstation }
    }
}

impl Render for FlowView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors;
        let workstation = self.workstation.clone();
        let Some(flow) = &workstation.read(cx).workflow.flow else {
            return div().into_any_element();
        };
        let rows = flow.fields.iter().map(|field| {
            let name = field.spec.name;
            let picker = field.spec.picker;
            let control = match (&field.input, &field.choice) {
                (Some(input), _) => Input::new(input).into_any_element(),
                (_, Some(choice)) => Select::new(choice).into_any_element(),
                _ => div().into_any_element(),
            };
            let this = workstation.clone();
            let choose = (picker != Picker::None).then(|| {
                Button::new(format!("dialog-choose-{name}"))
                    .small()
                    .label("Escolher…")
                    .accessibility_label(format!("Escolher {}", field.spec.label))
                    .on_click(move |_, window, cx| {
                        this.update(cx, |workstation, cx| {
                            workstation.choose_path(name, picker, window, cx)
                        });
                    })
            });
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors.muted_foreground)
                        .child(field.spec.label),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().flex_1().child(control))
                        .children(choose),
                )
        });
        let verify = flow.kind.has_report().then(|| {
            let this = workstation.clone();
            Button::new("dialog-verify")
                .small()
                .label("Verificar")
                .accessibility_label("Verificar")
                .on_click(move |_, window, cx| {
                    this.update(cx, |workstation, cx| workstation.flow_verify(window, cx));
                })
        });
        let report = flow.report.clone().map(|text| {
            div()
                .text_size(px(11.))
                .text_color(colors.muted_foreground)
                .child(text)
        });
        let error = flow.error.clone().map(|text| {
            div()
                .text_size(px(11.))
                .text_color(colors.danger)
                .child(text)
        });
        v_flex()
            .gap_3()
            .children(rows)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .children(verify)
                    .children(report),
            )
            .children(error)
            .into_any_element()
    }
}

struct JobsView {
    workstation: Entity<Workstation>,
}

impl JobsView {
    fn new(workstation: Entity<Workstation>, cx: &mut Context<Self>) -> Self {
        // Progress moves while the dialog is open; the workflow tick notifies the workstation.
        cx.observe(&workstation, |_, _, cx| cx.notify()).detach();
        Self { workstation }
    }
}

impl Render for JobsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors;
        let jobs = self.workstation.read(cx).workflow.host.registry().list();
        if jobs.is_empty() {
            return div()
                .text_size(px(12.))
                .text_color(colors.muted_foreground)
                .child("Nenhuma tarefa nesta sessão")
                .into_any_element();
        }
        let rows = jobs.into_iter().rev().take(12).map(|job| {
            let id = job.id.clone();
            let running = !job.state.is_final() && job.kind.is_cancellable();
            let this = self.workstation.clone();
            let cancel = running.then(|| {
                Button::new(format!("job-cancel-{id}"))
                    .small()
                    .label("Cancelar")
                    .accessibility_label(format!("Cancelar {}", job.label))
                    .on_click(move |_, _, cx| {
                        let id = id.clone();
                        this.update(cx, |workstation, cx| {
                            let _ = workstation.cancel_job(&id, cx);
                        });
                    })
            });
            let mut detail = match (&job.error, job.state.label()) {
                (Some(error), _) => error.message.clone(),
                (None, state) if job.stage.is_empty() => state.to_string(),
                (None, state) => format!("{state} · {}", job.stage),
            };
            // A result being discarded still owns its files until the discard job ends.
            if job.result_state == ResultState::Discarding {
                detail.push_str(" · descartando o resultado");
            }
            v_flex()
                .gap_1()
                .child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .child(div().text_size(px(12.)).child(job.label.clone()))
                        .children(cancel),
                )
                .child(
                    Progress::new(format!("job-progress-{}", job.id))
                        .value(job.progress().unwrap_or(0.0) as f32 * 100.0),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(colors.muted_foreground)
                        .child(detail),
                )
        });
        v_flex().gap_3().children(rows).into_any_element()
    }
}
