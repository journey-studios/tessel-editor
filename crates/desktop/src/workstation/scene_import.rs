//! F043 — attach a directed scene from the window and bring its resources to the monitor.
//!
//! A dialog (`FlowKind::AttachScene` / `FlowKind::SceneResources`) collects strings and nothing
//! else: the picked file is only a path, and the chain it starts is the one a script sends —
//! `scene.attach.start` (media host job: read, parse and validate the bundle off the UI thread;
//! the core's `scene.attach` then attaches the admitted scene as one undo step), then
//! `scene.resources.adopt` and `scene.resources.resolve` (more jobs), then the presentation
//! (`scene_choose`, `monitor.show_scene`). No rule lives here: the bundle is validated by the
//! importer inside the attach job, the files by the host's preflight/hash/containment, and nothing
//! in this module reads, parses, hashes, copies or decodes. The UI thread only starts jobs and, on
//! the workflow tick, reacts to the ones that finished (BL100).
//!
//! [`SceneImport`] is the pure state of that chain (no GPUI, no window): it says which request
//! comes next and what a finished job means. [`Workstation`]'s methods below run it and expose it
//! as the `scene_import` automation node, so the dialog, the socket and the tests read one state.

use serde_json::{Value, json};
use tessel_editor_core::CommandError;

use super::flows::{self, FlowKind};
use super::scene_ui::InspectorMode;
use super::{Activity, Source, Workstation};

/// What a scene dialog asks for once its own operation is done.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportPlan {
    pub scene: String,
    /// Folder the resources were authored in. `Some`: copy them into the project (adopt), then load.
    /// `None` with `load`: load what the project already holds in the scene's managed folder.
    pub source: Option<String>,
    /// `false`: attach and select only; loading is the other dialog's job.
    pub load: bool,
    pub show: bool,
}

/// The first job of the chain: adopt when there is a folder to copy from, else load.
pub fn first_request(plan: &ImportPlan) -> Value {
    step_request(plan, first_step(plan))
}

fn first_step(plan: &ImportPlan) -> Step {
    if plan.source.is_some() {
        Step::Adopt
    } else {
        Step::Resolve
    }
}

fn step_request(plan: &ImportPlan, step: Step) -> Value {
    match step {
        // Never asked of a plan: the attach request is the dialog's own (`SceneImport::request`).
        Step::Attach => Value::Null,
        Step::Adopt => {
            json!({ "op": "scene.resources.adopt", "scene": plan.scene, "source": plan.source })
        }
        Step::Resolve => json!({ "op": "scene.resources.resolve", "scene": plan.scene }),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// `scene.attach.start`: the bundle is read, validated and attached by the host.
    Attach,
    Adopt,
    Resolve,
}

impl Step {
    fn label(self) -> &'static str {
        match self {
            Self::Attach => "attach",
            Self::Adopt => "adopt",
            Self::Resolve => "resolve",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    /// `attach`, `adopt`, `resolve`, `select` or `show`: where the chain stopped.
    pub step: &'static str,
    pub code: String,
    pub message: String,
    /// The host's own details (`problems` per file for `resources_invalid`), kept verbatim.
    pub details: Option<Value>,
}

impl Failure {
    fn of(step: &'static str, error: &CommandError) -> Self {
        Self {
            step,
            code: error.code.to_string(),
            message: error.message.clone(),
            details: error.details.as_deref().cloned(),
        }
    }

    /// From the `{code, message, details}` the host puts in a failed job's event.
    ///
    /// WHY `details.job_code` wins: the command error vocabulary maps every job failure it has no
    /// name for to `job_failed` and keeps the job's own code (`resources_invalid`, `root_busy`…) in
    /// the details; showing `job_failed` would hide the one thing a person can act on. The message
    /// of a refused resource set only counts the problems, so the first one is appended.
    fn of_event(step: &'static str, event: &Value) -> Self {
        let error = &event["error"];
        let details = Some(error["details"].clone()).filter(|details| !details.is_null());
        let code = error["details"]["job_code"]
            .as_str()
            .or_else(|| error["code"].as_str())
            .or_else(|| event["state"].as_str())
            .unwrap_or("job_failed");
        let message = error["message"].as_str().unwrap_or("A tarefa não terminou");
        let first = &error["details"]["problems"][0];
        let message = match (first["path"].as_str(), first["detail"].as_str()) {
            (Some(path), Some(detail)) => format!("{message}: {path} — {detail}"),
            _ => message.to_string(),
        };
        Self {
            step,
            code: code.to_string(),
            message,
            details,
        }
    }

    fn to_json(&self) -> Value {
        json!({ "step": self.step, "code": self.code, "message": self.message, "details": self.details })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Running,
    /// Attached, nothing asked to load: the scene is edited but the monitor has no resources.
    Attached,
    /// The approved resource map is installed in the session.
    Loaded,
    /// A step failed. Whatever an earlier step committed stays (the scene is attached; adopted
    /// files are in the project); nothing after the failing step ran.
    Failed(Failure),
}

impl State {
    fn label(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Attached => "attached",
            Self::Loaded => "loaded",
            Self::Failed(_) => "failed",
        }
    }
}

/// What a job event meant for the chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Received {
    /// Not the job this chain is waiting for.
    Ignored,
    /// The step succeeded and [`SceneImport::request`] is the next one to start.
    Next,
    /// The scene was attached and nothing more was asked or needed: select it, load nothing.
    Attached,
    Loaded,
    Failed,
}

#[derive(Clone, Debug)]
pub struct SceneImport {
    plan: ImportPlan,
    /// The dialog's `scene.attach.start` while the chain still has to attach the scene.
    attach: Option<Value>,
    step: Step,
    job: Option<String>,
    state: State,
    /// Steps that completed, in order (`attach` first when this chain attached the scene).
    done: Vec<&'static str>,
    adopted: Option<Value>,
    /// What the core answered when the host attached the scene (ID, counts, `resources`).
    attached: Option<Value>,
    /// Presentation (select/show) is separate from loading: the resources can be fine while the
    /// monitor refuses, and that must not read as "the load failed".
    presentation: Option<Failure>,
}

impl SceneImport {
    pub fn new(plan: ImportPlan, attached: bool) -> Self {
        let state = if plan.load {
            State::Running
        } else {
            State::Attached
        };
        Self {
            step: first_step(&plan),
            plan,
            attach: None,
            job: None,
            state,
            done: if attached { vec!["attach"] } else { Vec::new() },
            adopted: None,
            attached: None,
            presentation: None,
        }
    }

    /// A chain that starts by attaching the scene: `request` is `scene.attach.start`. The scene's
    /// ID is the one the dialog typed, or the one the core gives when the job is applied.
    pub fn attaching(plan: ImportPlan, request: Value) -> Self {
        Self {
            step: Step::Attach,
            attach: Some(request),
            state: State::Running,
            ..Self::new(plan, false)
        }
    }

    pub fn scene(&self) -> &str {
        &self.plan.scene
    }

    pub fn is_running(&self) -> bool {
        self.state == State::Running
    }

    pub fn is_loaded(&self) -> bool {
        self.state == State::Loaded
    }

    pub fn wants_show(&self) -> bool {
        self.plan.show
    }

    /// The request of the step the chain is on.
    pub fn request(&self) -> Value {
        match (&self.step, &self.attach) {
            (Step::Attach, Some(request)) => request.clone(),
            _ => step_request(&self.plan, self.step),
        }
    }

    pub fn bind(&mut self, job: String) {
        self.job = Some(job);
    }

    /// A step that could not even start (an operation refused before it became a job).
    pub fn fail(&mut self, error: &CommandError) {
        self.job = None;
        self.state = State::Failed(Failure::of(self.step.label(), error));
    }

    pub fn mark(&mut self, step: &'static str) {
        self.done.push(step);
    }

    pub fn present_failed(&mut self, step: &'static str, error: &CommandError) {
        self.presentation = Some(Failure::of(step, error));
    }

    /// Reads one finished job. Only the job the chain is waiting for counts; the window's other
    /// jobs (imports, exports) pass through untouched.
    pub fn receive(&mut self, event: &Value) -> Received {
        let Some(job) = self.job.as_deref() else {
            return Received::Ignored;
        };
        if self.state != State::Running || event["job"].as_str() != Some(job) {
            return Received::Ignored;
        }
        self.job = None;
        if event["state"] != "succeeded" {
            self.state = State::Failed(Failure::of_event(self.step.label(), event));
            return Received::Failed;
        }
        match self.step {
            Step::Attach => self.receive_attached(event),
            Step::Adopt => {
                self.done.push("adopt");
                self.adopted = Some(event["result"]["report"].clone());
                self.step = Step::Resolve;
                Received::Next
            }
            Step::Resolve => {
                // The host installs the map in the same transaction that reports the job; an event
                // without it would mean nothing was installed, which must not read as loaded.
                if event["applied"]["installed"] != true {
                    self.state = State::Failed(Failure {
                        step: Step::Resolve.label(),
                        code: "not_installed".into(),
                        message: "Os recursos foram verificados, mas não instalados".into(),
                        details: None,
                    });
                    return Received::Failed;
                }
                self.done.push("resolve");
                self.state = State::Loaded;
                Received::Loaded
            }
        }
    }

    /// The attach job finished and was applied: the core's answer names the scene. A scene that
    /// names no files can load without a folder (`resources.needs_folder == false`, BL101), so
    /// the load is implied even though the dialog gave none; an old host that does not say keeps
    /// the dialog's own request (attach alone).
    fn receive_attached(&mut self, event: &Value) -> Received {
        let applied = &event["applied"];
        let Some(scene) = applied["scene"].as_str() else {
            self.state = State::Failed(Failure {
                step: Step::Attach.label(),
                code: "internal_error".into(),
                message: "A cena foi lida, mas o host não informou o ID anexado".into(),
                details: None,
            });
            return Received::Failed;
        };
        self.plan.scene = scene.to_string();
        self.attach = None;
        self.attached = Some(applied.clone());
        self.done.push("attach");
        if !self.plan.load && applied["resources"]["needs_folder"] != false {
            self.state = State::Attached;
            return Received::Attached;
        }
        self.plan.load = true;
        self.step = first_step(&self.plan);
        Received::Next
    }

    pub fn to_json(&self) -> Value {
        json!({
            // Until the attach job is applied there is no scene, whatever ID was typed.
            "scene": Some(&self.plan.scene).filter(|scene| !scene.is_empty() && self.attach.is_none()),
            "attached": self.attached,
            "state": self.state.label(),
            "step": self.step.label(),
            "job": self.job,
            "source": self.plan.source,
            "load": self.plan.load,
            "show": self.plan.show,
            "done": self.done,
            "adopted": self.adopted,
            "error": match &self.state {
                State::Failed(failure) => failure.to_json(),
                _ => Value::Null,
            },
            "presentation_error": self.presentation.as_ref().map(Failure::to_json),
        })
    }

    /// One line for the status bar once the chain stopped.
    fn summary(&self) -> String {
        let scene = &self.plan.scene;
        let label = if scene.is_empty() {
            "Cena".to_string()
        } else {
            format!("Cena «{scene}»")
        };
        match &self.state {
            State::Running if self.step == Step::Attach => "Anexando cena…".to_string(),
            State::Running => format!("{label}: carregando recursos…"),
            State::Attached => format!("{label} anexada; recursos não carregados"),
            State::Failed(failure) => {
                format!("{label}: falha em {}: {}", failure.step, failure.message)
            }
            State::Loaded => match (&self.presentation, self.done.contains(&"show")) {
                (Some(failure), _) => {
                    format!("{label} carregada; {}: {}", failure.step, failure.message)
                }
                (None, true) => format!("{label} carregada e exibida no monitor"),
                (None, false) => format!("{label} carregada"),
            },
        }
    }
}

impl Workstation {
    /// The dialog's submit for the scene flows. An error leaves the dialog open and the project
    /// exactly as it was: a missing field is refused before any operation and a request the host
    /// refuses before it becomes a job (a malformed ID) comes back here. The bundle itself is
    /// read and validated by a job (BL100), so an unreadable or invalid bundle is no longer an
    /// answer of the submit: the dialog closes and the failure is the `scene_import` node's
    /// `error` (step `attach`, typed code), with nothing attached. After the scene is attached the
    /// chain keeps that failure semantics: loading can fail without undoing the attach.
    pub(super) fn scene_import_submit(
        &mut self,
        kind: FlowKind,
        values: &[(&'static str, String)],
        cx: &mut gpui_kit::Context<Self>,
    ) -> Result<Value, CommandError> {
        // Built first so an empty required field is refused with its label before anything runs.
        let request = flows::build_request(kind, values)?;
        if self
            .workflow
            .scene_import
            .as_ref()
            .is_some_and(SceneImport::is_running)
        {
            return Err(CommandError::new(
                "scene_import_busy",
                "Aguarde o carregamento da cena anterior terminar",
            ));
        }
        if kind == FlowKind::SceneResources {
            let plan = flows::import_plan(kind, values, &flows::field_value(values, "scene"));
            self.scene_import_begin(SceneImport::new(plan, false), false, cx)?;
            return Ok(self.scene_import_answer(false));
        }
        let typed_id = flows::field_value(values, "id");
        let plan = flows::import_plan(kind, values, &typed_id);
        self.scene_import_begin(SceneImport::attaching(plan, request), false, cx)?;
        Ok(self.scene_import_answer(false))
    }

    /// `changed` is whether the project changed by the time of the answer: a scene dialog that
    /// only starts jobs changed nothing yet; the node says what happens next.
    fn scene_import_answer(&self, changed: bool) -> Value {
        json!({
            "action": "scene.import", "changed": changed,
            "import": self.workflow.scene_import.as_ref().map(SceneImport::to_json),
        })
    }

    /// Starts `import`'s first step. `attached`: the scene is already attached, so a step that
    /// cannot start is recorded on the node instead of returned (the attach already happened and
    /// must not be repeated); otherwise it is returned and nothing was started.
    pub(super) fn scene_import_begin(
        &mut self,
        mut import: SceneImport,
        attached: bool,
        cx: &mut gpui_kit::Context<Self>,
    ) -> Result<(), CommandError> {
        if !import.is_running() {
            self.scene_import_present(&mut import, cx);
            self.scene_import_settle(import, cx);
            return Ok(());
        }
        match self.scene_import_start(&import.request(), cx) {
            Ok(job) => import.bind(job),
            Err(error) if !attached => return Err(error),
            Err(error) => import.fail(&error),
        }
        if import.is_running() {
            self.workflow.scene_import = Some(import);
            cx.notify();
        } else {
            self.scene_import_settle(import, cx);
        }
        Ok(())
    }

    /// Runs a step's request on the media host; the host answers with a job at once (it is
    /// interactive), so the UI thread never waits for hashing or copying.
    fn scene_import_start(
        &mut self,
        request: &Value,
        cx: &mut gpui_kit::Context<Self>,
    ) -> Result<String, CommandError> {
        let answer = self.run_media_op(request, cx).ok_or_else(|| {
            CommandError::new(
                "unknown_operation",
                "The media host does not know this scene operation",
            )
        })??;
        answer["job"].as_str().map(str::to_string).ok_or_else(|| {
            CommandError::new("internal_error", "A scene job answered without a job id")
        })
    }

    /// Called by the workflow tick for every finished job.
    pub(super) fn scene_import_receive(&mut self, event: &Value, cx: &mut gpui_kit::Context<Self>) {
        let Some(mut import) = self.workflow.scene_import.take() else {
            return;
        };
        let received = import.receive(event);
        match received {
            Received::Ignored => {}
            Received::Next => match self.scene_import_start(&import.request(), cx) {
                Ok(job) => import.bind(job),
                Err(error) => import.fail(&error),
            },
            Received::Loaded | Received::Attached => self.scene_import_present(&mut import, cx),
            Received::Failed => {}
        }
        if received == Received::Ignored || import.is_running() {
            self.workflow.scene_import = Some(import);
            return;
        }
        self.scene_import_settle(import, cx);
    }

    /// Selects the scene in the inspector and, when loaded and asked for, shows it in the monitor.
    /// WHY a separate presentation error: the load already succeeded, so a refusal from the
    /// monitor (runtime still booting, a gate of `scene.plan`) is reported without un-loading.
    fn scene_import_present(&mut self, import: &mut SceneImport, cx: &mut gpui_kit::Context<Self>) {
        let scene = import.scene().to_string();
        match self.scene_choose(&scene, cx) {
            Ok(_) => import.mark("select"),
            Err(error) => {
                import.present_failed("select", &error);
                return;
            }
        }
        self.scene_set_mode(InspectorMode::Scene, cx);
        if !(import.is_loaded() && import.wants_show()) {
            return;
        }
        match self.show_directed_scene(&scene, cx) {
            Ok(_) => import.mark("show"),
            Err(error) => import.present_failed("show", &error),
        }
    }

    /// The chain stopped: keep its final state on the node and say it in the status bar.
    fn scene_import_settle(&mut self, import: SceneImport, cx: &mut gpui_kit::Context<Self>) {
        self.activity = Some(Activity {
            source: Source::Gesture,
            text: import.summary(),
            error: matches!(import.state, State::Failed(_)) || import.presentation.is_some(),
        });
        self.workflow.scene_import = Some(import);
        cx.notify();
    }

    /// The `scene_import` node: always present, `state.state` is `idle` before any chain.
    pub(super) fn scene_import_nodes(&self) -> Vec<Value> {
        let state = self
            .workflow
            .scene_import
            .as_ref()
            .map_or_else(|| json!({ "state": "idle" }), SceneImport::to_json);
        vec![json!({
            "id": "scene_import", "kind": "status", "label": "Anexar cena",
            "parent": "region:status", "children": [], "enabled": true, "visible": true,
            "fields": [], "actions": [], "state": state,
        })]
    }
}

#[cfg(test)]
#[path = "scene_import_tests.rs"]
mod tests;
