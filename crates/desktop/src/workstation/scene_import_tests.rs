//! F043 — the scene dialogs' chain, proven against the real media host and the synthetic corpus
//! (`scripts/directed-scene-proof/corpus`: drawn from scratch, no private media).
//!
//! Nothing here mocks processing: `scene.attach` is the core's, adopt/resolve are real jobs that
//! hash and copy real PNGs, and the window applies them on its own tick. Opening the dialog itself
//! needs the Kit `Root`, which a test window lacks, so the dialog screens (fields, the native
//! picker, WebView hiding) are proven by `scripts/scene-import-proof.py` in a real window; here the
//! flows' `submit` and the chain behind it are exercised exactly as the dialog's button calls them.
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::*;
use crate::workstation::OpReply;
use crate::workstation::flows::{self, FlowKind};
use crate::workstation::input_tests::fixture;
use core::prelude::v1::test;
use gpui_kit::*;
use serde_json::{Value, json};
use tessel_editor_core::CommandError;

fn plan(source: Option<&str>, load: bool) -> ImportPlan {
    ImportPlan {
        scene: "street".into(),
        source: source.map(str::to_string),
        load,
        show: true,
    }
}

fn values<'a>(pairs: &[(&'a str, &str)]) -> Vec<(&'a str, String)> {
    pairs
        .iter()
        .map(|(name, value)| (*name, value.to_string()))
        .collect()
}

fn succeeded(job: &str, extra: Value) -> Value {
    let mut event = json!({ "job": job, "state": "succeeded", "kind": "import" });
    for (key, value) in extra.as_object().cloned().unwrap_or_default() {
        event[key] = value;
    }
    event
}

// ---- the dialogs send what a script sends (no window) ----

#[test]
fn the_attach_dialog_starts_the_attach_job_with_only_the_fields_that_were_filled() {
    let full = flows::build_request(
        FlowKind::AttachScene,
        &values(&[
            ("bundle", " /b/render-bundle.json "),
            ("id", "street"),
            ("name", "Rua"),
            ("source", "/b"),
            ("show", "yes"),
        ]),
    )
    .unwrap();
    assert_eq!(
        full,
        json!({ "op": "scene.attach.start", "path": "/b/render-bundle.json", "id": "street", "name": "Rua" })
    );
    let bare = flows::build_request(
        FlowKind::AttachScene,
        &values(&[("bundle", "/b/x.json"), ("id", " "), ("name", "")]),
    )
    .unwrap();
    assert_eq!(
        bare,
        json!({ "op": "scene.attach.start", "path": "/b/x.json" })
    );
    let error =
        flows::build_request(FlowKind::AttachScene, &values(&[("bundle", "  ")])).unwrap_err();
    assert_eq!(error.code, "missing_argument");
    assert!(error.message.contains("Render-bundle"), "{}", error.message);
}

#[test]
fn the_resources_dialog_starts_with_adopt_when_it_has_a_folder_and_with_load_otherwise() {
    let adopt = flows::build_request(
        FlowKind::SceneResources,
        &values(&[("scene", "street"), ("source", "/authored")]),
    )
    .unwrap();
    assert_eq!(
        adopt,
        json!({ "op": "scene.resources.adopt", "scene": "street", "source": "/authored" })
    );
    let load =
        flows::build_request(FlowKind::SceneResources, &values(&[("scene", "street")])).unwrap();
    assert_eq!(
        load,
        json!({ "op": "scene.resources.resolve", "scene": "street" })
    );
    assert_eq!(
        flows::build_request(FlowKind::SceneResources, &values(&[("scene", "")]))
            .unwrap_err()
            .code,
        "missing_argument"
    );
}

#[test]
fn the_plan_loads_only_when_asked_and_shows_unless_told_not_to() {
    let attach = |pairs: &[(&str, &str)]| {
        flows::import_plan(FlowKind::AttachScene, &values(pairs), "street")
    };
    assert!(
        !attach(&[("source", "")]).load,
        "attach alone loads nothing"
    );
    assert!(attach(&[("source", "/b")]).load);
    assert!(attach(&[]).show, "showing is the default");
    assert!(!attach(&[("show", "no")]).show);
    let reload = flows::import_plan(FlowKind::SceneResources, &values(&[]), "street");
    assert!(reload.load && reload.source.is_none(), "{reload:?}");
}

#[test]
fn every_scene_flow_is_a_flow_node_and_a_scene_import() {
    for kind in [FlowKind::AttachScene, FlowKind::SceneResources] {
        assert!(kind.is_scene_import());
        assert!(
            flows::nodes()
                .iter()
                .any(|node| node["id"] == format!("flow.{}", kind.id())),
            "{kind:?}"
        );
    }
    assert!(!FlowKind::Import.is_scene_import());
}

// ---- the chain's state, without a window ----

#[test]
fn a_chain_adopts_then_loads_and_ignores_other_jobs() {
    let mut chain = SceneImport::new(plan(Some("/authored"), true), true);
    assert_eq!(
        chain.request()["op"],
        "scene.resources.adopt",
        "a folder starts with a copy"
    );
    chain.bind("job-1".into());
    assert_eq!(
        chain.receive(&succeeded("job-9", json!({}))),
        Received::Ignored,
        "an import of media is not ours"
    );
    assert!(chain.is_running());
    assert_eq!(
        chain.receive(&succeeded(
            "job-1",
            json!({ "result": { "report": { "ok": true } } })
        )),
        Received::Next
    );
    assert_eq!(chain.request()["op"], "scene.resources.resolve");
    chain.bind("job-2".into());
    assert_eq!(
        chain.receive(&succeeded(
            "job-2",
            json!({ "applied": { "installed": true } })
        )),
        Received::Loaded
    );
    let report = chain.to_json();
    assert_eq!(report["state"], "loaded");
    assert_eq!(report["done"], json!(["attach", "adopt", "resolve"]));
    assert_eq!(report["adopted"], json!({ "ok": true }));
    assert_eq!(report["error"], Value::Null);
}

#[test]
fn a_failed_job_stops_the_chain_with_the_hosts_own_code() {
    let mut chain = SceneImport::new(plan(Some("/authored"), true), true);
    chain.bind("job-1".into());
    // The command vocabulary has no name for this job error, so the host keeps it in the details.
    let failed = json!({ "job": "job-1", "state": "failed", "kind": "import",
        "error": { "code": "job_failed", "message": "1 resource problem(s); nothing was published",
            "details": { "job_code": "resources_invalid", "problems": [
                { "path": "assets/hero.png", "code": "missing", "detail": "no such file" }] } } });
    assert_eq!(chain.receive(&failed), Received::Failed);
    let report = chain.to_json();
    assert_eq!(report["state"], "failed");
    assert_eq!(report["error"]["step"], "adopt");
    assert_eq!(report["error"]["code"], "resources_invalid");
    let message = report["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("assets/hero.png") && message.contains("no such file"),
        "{message}"
    );
    assert_eq!(report["error"]["details"]["problems"][0]["code"], "missing");
    assert_eq!(report["done"], json!(["attach"]));
    // Settled: a late duplicate of the event changes nothing.
    assert_eq!(chain.receive(&failed), Received::Ignored);
    assert!(!chain.is_running());
}

#[test]
fn a_cancelled_job_is_a_failure_and_never_a_load() {
    let mut chain = SceneImport::new(plan(None, true), false);
    assert_eq!(chain.request()["op"], "scene.resources.resolve");
    chain.bind("job-1".into());
    let cancelled = json!({ "job": "job-1", "state": "cancelled", "kind": "preflight" });
    assert_eq!(chain.receive(&cancelled), Received::Failed);
    assert_eq!(chain.to_json()["error"]["code"], "cancelled");
    assert!(!chain.is_loaded());
}

#[test]
fn a_load_that_installed_nothing_is_not_reported_as_loaded() {
    let mut chain = SceneImport::new(plan(None, true), false);
    chain.bind("job-1".into());
    assert_eq!(
        chain.receive(&succeeded("job-1", json!({ "applied": null }))),
        Received::Failed
    );
    assert_eq!(chain.to_json()["error"]["code"], "not_installed");
}

#[test]
fn a_step_that_cannot_start_is_recorded_on_the_step_it_was() {
    let mut chain = SceneImport::new(plan(Some("relative"), true), true);
    chain.fail(&CommandError::new(
        "base_not_absolute",
        "use an absolute path",
    ));
    let report = chain.to_json();
    assert_eq!(report["state"], "failed");
    assert_eq!(report["error"]["step"], "adopt");
    assert_eq!(report["error"]["code"], "base_not_absolute");
}

fn attached_event(job: &str, scene: &str, needs_folder: Option<bool>) -> Value {
    let mut applied = json!({ "action": "scene.attach", "scene": scene, "changed": true });
    if let Some(needs_folder) = needs_folder {
        applied["resources"] = json!({ "files": 2, "needs_folder": needs_folder });
    }
    succeeded(job, json!({ "applied": applied }))
}

fn attaching(source: Option<&str>) -> SceneImport {
    let mut plan = plan(source, source.is_some());
    plan.scene = String::new();
    let mut chain = SceneImport::attaching(
        plan,
        json!({ "op": "scene.attach.start", "path": "/b.json" }),
    );
    chain.bind("job-1".into());
    chain
}

#[test]
fn an_attach_chain_starts_with_the_attach_job_and_learns_the_scene_id_from_its_answer() {
    let mut chain = attaching(Some("/authored"));
    assert_eq!(chain.request()["op"], "scene.attach.start");
    assert!(chain.is_running());
    assert_eq!(chain.to_json()["scene"], Value::Null, "not attached yet");
    assert_eq!(chain.to_json()["attached"], Value::Null);
    assert_eq!(
        chain.receive(&attached_event("job-9", "other", Some(true))),
        Received::Ignored
    );
    assert_eq!(
        chain.receive(&attached_event("job-1", "scene-1", Some(true))),
        Received::Next
    );
    assert_eq!(chain.scene(), "scene-1", "the core's generated ID");
    assert_eq!(chain.request()["op"], "scene.resources.adopt");
    assert_eq!(chain.request()["scene"], "scene-1");
    assert_eq!(chain.to_json()["done"], json!(["attach"]));
    assert_eq!(chain.to_json()["attached"]["resources"]["files"], 2);
}

#[test]
fn an_attach_without_a_folder_stops_attached_when_files_are_needed_and_loads_when_none_are() {
    let mut needs = attaching(None);
    assert_eq!(
        needs.receive(&attached_event("job-1", "street", Some(true))),
        Received::Attached
    );
    assert_eq!(needs.to_json()["state"], "attached");
    assert!(!needs.is_running());

    // A host that does not say keeps the old behavior: attach alone.
    let mut silent = attaching(None);
    assert_eq!(
        silent.receive(&attached_event("job-1", "street", None)),
        Received::Attached
    );

    // BL101: nothing to load from a folder, so loading is implied and goes straight to resolve.
    let mut data = attaching(None);
    assert_eq!(
        data.receive(&attached_event("job-1", "street", Some(false))),
        Received::Next
    );
    assert_eq!(
        data.request(),
        json!({ "op": "scene.resources.resolve", "scene": "street" })
    );
    assert_eq!(data.to_json()["load"], true);
    data.bind("job-2".into());
    assert_eq!(
        data.receive(&succeeded(
            "job-2",
            json!({ "applied": { "installed": true } })
        )),
        Received::Loaded
    );
    assert_eq!(data.to_json()["done"], json!(["attach", "resolve"]));
}

#[test]
fn a_refused_or_cancelled_attach_job_fails_on_the_attach_step_with_its_typed_code() {
    let mut refused = attaching(Some("/authored"));
    let event = json!({ "job": "job-1", "state": "failed", "kind": "import",
        "error": { "code": "invalid_bundle", "message": "INVALID_VALUE: needs at least 1 item(s)",
            "details": { "issues": [{ "path": "/assets" }] } } });
    assert_eq!(refused.receive(&event), Received::Failed);
    let report = refused.to_json();
    assert_eq!(report["state"], "failed");
    assert_eq!(report["error"]["step"], "attach");
    assert_eq!(report["error"]["code"], "invalid_bundle");
    assert_eq!(report["done"], json!([]), "nothing was attached");
    assert_eq!(report["scene"], Value::Null);

    let mut stale = attaching(None);
    let event = json!({ "job": "job-1", "state": "failed", "kind": "import",
        "error": { "code": "stale_result", "message": "The open project changed" } });
    assert_eq!(stale.receive(&event), Received::Failed);
    assert_eq!(stale.to_json()["error"]["code"], "stale_result");

    let mut cancelled = attaching(None);
    let event = json!({ "job": "job-1", "state": "cancelled", "kind": "import" });
    assert_eq!(cancelled.receive(&event), Received::Failed);
    assert_eq!(cancelled.to_json()["error"]["code"], "cancelled");
    assert_eq!(cancelled.to_json()["error"]["step"], "attach");

    let mut nameless = attaching(None);
    let event = succeeded("job-1", json!({ "applied": { "action": "scene.attach" } }));
    assert_eq!(nameless.receive(&event), Received::Failed);
    assert_eq!(nameless.to_json()["error"]["code"], "internal_error");
}

// ---- the window, the real host, real files ----

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/directed-scene-proof/corpus")
}

/// A private copy of the corpus bundle and its three PNGs: the "authoring folder" the dialog
/// points at, which the tests may then break or alter without touching the repository.
struct Authoring {
    root: PathBuf,
}

impl Authoring {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("te-r2-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("authored/assets")).unwrap();
        fs::copy(
            corpus().join("bundle.json"),
            root.join("authored/bundle.json"),
        )
        .unwrap();
        for file in ["sky.png", "mid.png", "hero.png"] {
            fs::copy(
                corpus().join("assets").join(file),
                root.join("authored/assets").join(file),
            )
            .unwrap();
        }
        Self { root }
    }

    fn folder(&self) -> PathBuf {
        self.root.join("authored")
    }

    fn bundle(&self) -> String {
        self.folder().join("bundle.json").to_string_lossy().into()
    }

    fn workspace(&self) -> PathBuf {
        self.root.join("workspace")
    }
}

impl Drop for Authoring {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn project(view: &Entity<Workstation>, cx: &mut VisualTestContext, authoring: &Authoring) {
    let reply = view.update(cx, |view, cx| {
        view.run_op(
            &json!({"op":"project.create","name":"Scenes","dir":authoring.workspace(),
                "width":160,"height":90,"length":48}),
            cx,
        )
    });
    assert!(reply.is_ok(), "{:?}", reply.err());
    cx.run_until_parked();
}

fn submit(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    kind: FlowKind,
    pairs: &[(&'static str, &str)],
) -> Result<Value, CommandError> {
    let values = values(pairs);
    view.update(cx, |view, cx| view.scene_import_submit(kind, &values, cx))
}

fn attach(authoring: &Authoring, extra: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
    let mut pairs = vec![("bundle", authoring.bundle()), ("id", "street".into())];
    pairs.extend(extra.iter().map(|(name, value)| (*name, value.to_string())));
    pairs
}

fn run_attach(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    pairs: Vec<(&'static str, String)>,
) -> Result<Value, CommandError> {
    let borrowed: Vec<(&'static str, &str)> = pairs
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    submit(view, cx, FlowKind::AttachScene, &borrowed)
}

fn import_state(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Value {
    view.read_with(cx, |view, cx| view.tree_node("scene_import", cx))
        .expect("the scene_import node is always in the tree")["state"]
        .clone()
}

/// Ticks the window the way its timer does until the chain stops running.
fn settle(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        view.update(cx, |view, cx| view.workflow_tick(cx));
        cx.run_until_parked();
        let state = import_state(view, cx);
        if state["state"] != "running" {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "the chain never settled: {state}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn scene_ids(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |view, _| {
        view.state
            .scenes
            .ids()
            .iter()
            .map(|id| id.to_string())
            .collect()
    })
}

fn undo_depth(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> u64 {
    let status = view
        .update(cx, |view, cx| {
            view.run_op(&json!({"op":"history.status"}), cx)
        })
        .expect("history.status");
    let OpReply::Done(status) = status else {
        panic!("history.status answers at once");
    };
    status["history"]["undo_depth"]
        .as_u64()
        .expect("history.undo_depth")
}

fn installed(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |view, _| {
        view.workflow.scene_resources().get("street").is_some()
    })
}

#[gpui_kit::test]
fn a_valid_file_attaches_adopts_loads_selects_and_asks_the_monitor(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let authoring = Authoring::new("valid");
    project(&view, cx, &authoring);

    let started = Instant::now();
    let answer = run_attach(
        &view,
        cx,
        attach(
            &authoring,
            &[
                ("source", &authoring.folder().to_string_lossy()),
                ("show", "yes"),
            ],
        ),
    )
    .unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the UI thread only attached and started a job: {answer}"
    );
    // The UI thread only started the attach job: nothing is read or attached until the tick.
    assert!(scene_ids(&view, cx).is_empty(), "{answer}");
    assert_eq!(undo_depth(&view, cx), 0);
    assert_eq!(answer["changed"], false);
    let running = import_state(&view, cx);
    assert_eq!(running["state"], "running", "{running}");
    assert_eq!(running["step"], "attach", "{running}");
    assert!(running["job"].is_string());
    assert!(
        !installed(&view, cx),
        "nothing is approved before the jobs end"
    );

    let done = settle(&view, cx);
    // Read before anything else runs an operation (that would replace the status line).
    let activity = view.read_with(cx, |view, _| {
        view.activity.as_ref().map(|a| (a.text.clone(), a.error))
    });
    assert_eq!(done["state"], "loaded", "{done}");
    assert_eq!(done["error"], Value::Null, "{done}");
    let steps: Vec<&str> = done["done"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(
        &steps[..4],
        ["attach", "adopt", "resolve", "select"],
        "{done}"
    );

    // The core attached the admitted scene when the job was applied: one undo entry.
    assert_eq!(scene_ids(&view, cx), vec!["street".to_string()]);
    assert_eq!(undo_depth(&view, cx), 1);
    // Real files in the project, verified and loaded under this project's generation.
    for file in ["sky.png", "mid.png", "hero.png"] {
        assert!(
            authoring
                .workspace()
                .join("scenes/street/assets")
                .join(file)
                .is_file(),
            "{file} was not copied into the project"
        );
    }
    assert!(installed(&view, cx));
    view.read_with(cx, |view, _| {
        let map = view.workflow.scene_resources().get("street").unwrap();
        assert_eq!(map.project_id, view.state.project.id);
        assert!(map.complete);
        assert_eq!(view.scene_active_id().as_deref(), Some("street"));
        assert_eq!(
            view.scene_ui.mode,
            super::super::scene_ui::InspectorMode::Scene
        );
        assert_eq!(
            view.scene_monitor(),
            super::super::scene_monitor::SceneMonitor::Unavailable {
                reason: "O monitor está iniciando ou indisponível"
            },
            "only the test window's missing WebView is what keeps the monitor from being available"
        );
    });
    // Every job is in the shared job history, in order, like any other job.
    let kinds: Vec<String> = view.read_with(cx, |view, _| {
        view.jobs_report()["recent"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .map(|event| event["result"]["action"].as_str().unwrap_or("").to_string())
            .collect()
    });
    assert_eq!(
        kinds,
        [
            "scene.attach.start",
            "scene.resources.adopt",
            "scene.resources.resolve"
        ]
    );
    // The monitor was asked: it either took the request or said why not, never silence.
    let shown = done["done"].as_array().unwrap().contains(&json!("show"));
    assert!(
        shown || done["presentation_error"]["step"] == "show",
        "{done}"
    );
    assert!(
        activity
            .as_ref()
            .is_some_and(|(text, _)| text.contains("street")),
        "{activity:?}"
    );
}

#[gpui_kit::test]
fn an_invalid_bundle_is_refused_by_the_attach_job_before_anything_changes(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let authoring = Authoring::new("invalid");
    project(&view, cx, &authoring);
    let revision = view.read_with(cx, |view, _| view.state.project.revision);

    // Valid JSON that is not a render-bundle, and text that is not JSON.
    let broken = authoring.folder().join("broken.json");
    let missing = authoring.folder().join("nowhere.json");
    fs::write(&missing, "").unwrap();
    fs::remove_file(&missing).unwrap();
    let cases = [
        (r#"{"bundleVersion": 99}"#, broken.clone(), "invalid_bundle"),
        ("{ not json", broken.clone(), "invalid_json"),
        ("", missing, "bundle_unreadable"),
    ];
    for (content, path, expected) in cases {
        if !content.is_empty() {
            fs::write(&path, content).unwrap();
        }
        // The dialog closes: the bundle is read by a job, so its refusal is the node's error.
        run_attach(
            &view,
            cx,
            vec![
                ("bundle", path.to_string_lossy().into()),
                ("id", "street".into()),
                ("source", authoring.folder().to_string_lossy().into()),
            ],
        )
        .expect("a readable request starts the job");
        let failed = settle(&view, cx);
        assert_eq!(failed["state"], "failed", "{content}: {failed}");
        assert_eq!(failed["error"]["step"], "attach", "{failed}");
        assert_eq!(failed["error"]["code"], expected, "{failed}");
        assert_eq!(failed["scene"], Value::Null, "{failed}");
    }

    // A request the host refuses before it becomes a job stays in the dialog.
    let error = run_attach(
        &view,
        cx,
        vec![
            ("bundle", broken.to_string_lossy().into()),
            ("id", "bad id!".into()),
        ],
    )
    .unwrap_err();
    assert_eq!(error.code, "invalid_id", "{error:?}");

    assert!(scene_ids(&view, cx).is_empty(), "no scene was attached");
    assert_eq!(undo_depth(&view, cx), 0, "no history entry");
    assert_eq!(
        view.read_with(cx, |view, _| view.state.project.revision),
        revision
    );
    assert!(
        !authoring.workspace().join("scenes").exists(),
        "no resource folder was created"
    );
    assert!(!installed(&view, cx));
    // The failed chain is over: the next one is not busy.
    run_attach(&view, cx, attach(&authoring, &[])).unwrap();
    assert_eq!(settle(&view, cx)["state"], "attached");
}

#[gpui_kit::test]
fn a_missing_resource_leaves_the_scene_attached_unloaded_and_the_workspace_clean(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    let authoring = Authoring::new("missing");
    project(&view, cx, &authoring);
    fs::remove_file(authoring.folder().join("assets/hero.png")).unwrap();

    run_attach(
        &view,
        cx,
        attach(
            &authoring,
            &[("source", &authoring.folder().to_string_lossy())],
        ),
    )
    .expect("the bundle itself is valid: the dialog closes and the failure is on the node");
    let failed = settle(&view, cx);
    assert_eq!(failed["state"], "failed", "{failed}");
    assert_eq!(failed["error"]["step"], "adopt");
    assert_eq!(failed["error"]["code"], "resources_invalid");
    assert!(
        failed["error"]["message"]
            .as_str()
            .unwrap()
            .contains("hero"),
        "the host's own explanation reaches the window: {failed}"
    );
    // The attach is kept (it is a valid scene); everything the failing step touched is gone.
    assert_eq!(scene_ids(&view, cx), vec!["street".to_string()]);
    assert!(!installed(&view, cx));
    assert!(!authoring.workspace().join("scenes/street").exists());
    let activity = view.read_with(cx, |view, _| {
        view.activity.as_ref().map(|a| (a.text.clone(), a.error))
    });
    assert!(
        activity
            .as_ref()
            .is_some_and(|(text, error)| *error && text.contains("adopt")),
        "{activity:?}"
    );

    // Repairing the folder and asking again through the second dialog completes the chain.
    fs::copy(
        corpus().join("assets/hero.png"),
        authoring.folder().join("assets/hero.png"),
    )
    .unwrap();
    submit(
        &view,
        cx,
        FlowKind::SceneResources,
        &[
            ("scene", "street"),
            ("source", &authoring.folder().to_string_lossy()),
        ],
    )
    .unwrap();
    let done = settle(&view, cx);
    assert_eq!(done["state"], "loaded", "{done}");
    assert!(installed(&view, cx));
    assert_eq!(scene_ids(&view, cx).len(), 1, "no second attach");
}

#[gpui_kit::test]
fn a_source_file_that_changed_since_the_bundle_was_made_is_refused_by_hash(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    let authoring = Authoring::new("altered-source");
    project(&view, cx, &authoring);
    let sky = authoring.folder().join("assets/sky.png");
    let mut bytes = fs::read(&sky).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xFF;
    fs::write(&sky, bytes).unwrap();

    run_attach(
        &view,
        cx,
        attach(
            &authoring,
            &[("source", &authoring.folder().to_string_lossy())],
        ),
    )
    .unwrap();
    let failed = settle(&view, cx);
    assert_eq!(failed["state"], "failed", "{failed}");
    assert_eq!(failed["error"]["step"], "adopt");
    assert_eq!(failed["error"]["code"], "resources_invalid", "{failed}");
    assert!(!installed(&view, cx));
    assert!(
        !authoring.workspace().join("scenes/street").exists(),
        "an altered file is never copied into the project"
    );
}

#[gpui_kit::test]
fn a_managed_file_altered_after_adoption_fails_the_reload_without_approving_it(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    let authoring = Authoring::new("altered-managed");
    project(&view, cx, &authoring);
    run_attach(
        &view,
        cx,
        attach(
            &authoring,
            &[("source", &authoring.folder().to_string_lossy())],
        ),
    )
    .unwrap();
    assert_eq!(settle(&view, cx)["state"], "loaded");

    // A new session of the same project: what was approved is forgotten, the files stay.
    view.update(cx, |view, cx| {
        view.run_op(
            &json!({"op":"scene.resources.invalidate","reason":"reload"}),
            cx,
        )
    })
    .unwrap();
    assert!(!installed(&view, cx));
    let managed = authoring.workspace().join("scenes/street/assets/mid.png");
    let mut bytes = fs::read(&managed).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xFF;
    fs::write(&managed, bytes).unwrap();

    let started = submit(&view, cx, FlowKind::SceneResources, &[("scene", "street")]).unwrap();
    assert_eq!(
        started["changed"], false,
        "starting a job has not changed the project"
    );
    let failed = settle(&view, cx);
    assert_eq!(failed["state"], "failed", "{failed}");
    assert_eq!(failed["error"]["step"], "resolve");
    assert_eq!(failed["error"]["code"], "resources_invalid", "{failed}");
    assert!(!installed(&view, cx), "the altered file was never approved");
}

#[gpui_kit::test]
fn attaching_without_a_folder_selects_the_scene_and_loads_nothing(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let authoring = Authoring::new("attach-only");
    project(&view, cx, &authoring);
    run_attach(&view, cx, attach(&authoring, &[])).unwrap();
    let state = settle(&view, cx);
    assert_eq!(state["state"], "attached", "{state}");
    assert_eq!(state["job"], Value::Null);
    assert_eq!(
        state["attached"]["resources"],
        json!({ "files": 3, "needs_folder": true }),
        "the host says this scene needs its files: nothing is loaded without a folder"
    );
    assert!(!installed(&view, cx));
    view.read_with(cx, |view, _| {
        assert_eq!(view.scene_active_id().as_deref(), Some("street"));
        assert!(view.workflow.scene_resources().get("street").is_none());
    });
    let activity = view.read_with(cx, |view, _| view.activity.as_ref().map(|a| a.text.clone()));
    assert!(activity.is_some_and(|text| text.contains("recursos não carregados")));
}

/// A scene that needs no file (BL101): its assets are declared and nothing uses them, because
/// the format requires one declared asset at least.
fn data_only_bundle(authoring: &Authoring) -> String {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json");
    let mut document: Value = serde_json::from_str(&fs::read_to_string(source).unwrap()).unwrap();
    document["sceneDefinitions"][0]["nodes"]
        .as_array_mut()
        .unwrap()
        .retain(|node| node["type"] == "group");
    let path = authoring.folder().join("data.bundle.json");
    fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    path.to_string_lossy().into()
}

fn attach_data_only(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    bundle: String,
) -> Value {
    run_attach(
        view,
        cx,
        vec![
            ("bundle", bundle),
            ("id", "data".into()),
            ("show", "yes".into()),
        ],
    )
    .unwrap();
    settle(view, cx)
}

#[gpui_kit::test]
fn a_scene_that_needs_no_files_loads_without_a_folder(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let authoring = Authoring::new("data-only");
    project(&view, cx, &authoring);
    let bundle = data_only_bundle(&authoring);

    let done = attach_data_only(&view, cx, bundle);
    assert_eq!(done["state"], "loaded", "{done}");
    assert_eq!(done["error"], Value::Null, "{done}");
    let steps: Vec<&str> = done["done"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(&steps[..3], ["attach", "resolve", "select"], "{done}");
    assert_eq!(
        done["attached"]["resources"],
        json!({ "files": 0, "needs_folder": false })
    );
    view.read_with(cx, |view, _| {
        assert!(view.workflow.scene_resources().get("data").is_some());
        assert_eq!(view.scene_active_id().as_deref(), Some("data"));
    });
    assert!(
        !authoring.workspace().join("scenes").exists(),
        "no folder was needed, none was created"
    );
}

#[gpui_kit::test]
fn a_scene_that_needs_no_files_loads_even_in_a_project_that_was_never_saved(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    let authoring = Authoring::new("data-only-unsaved");
    assert!(
        view.read_with(cx, |view, _| view.state.project.workspace.is_none()),
        "the fixture project was never saved"
    );
    let bundle = data_only_bundle(&authoring);
    let done = attach_data_only(&view, cx, bundle);
    assert_eq!(done["state"], "loaded", "{done}");
    assert!(view.read_with(cx, |view, _| {
        view.workflow.scene_resources().get("data").is_some()
    }));
}

#[gpui_kit::test]
fn a_second_chain_waits_for_the_running_one_and_a_wrong_scene_changes_nothing(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    let authoring = Authoring::new("busy");
    project(&view, cx, &authoring);
    run_attach(
        &view,
        cx,
        attach(
            &authoring,
            &[("source", &authoring.folder().to_string_lossy())],
        ),
    )
    .unwrap();
    let busy = submit(&view, cx, FlowKind::SceneResources, &[("scene", "street")]).unwrap_err();
    assert_eq!(busy.code, "scene_import_busy");
    let busy_attach = run_attach(&view, cx, attach(&authoring, &[])).unwrap_err();
    assert_eq!(busy_attach.code, "scene_import_busy");
    assert_eq!(settle(&view, cx)["state"], "loaded");
    assert_eq!(
        scene_ids(&view, cx).len(),
        1,
        "a refused attach attached nothing"
    );

    let depth = undo_depth(&view, cx);
    let unknown = submit(&view, cx, FlowKind::SceneResources, &[("scene", "ghost")]).unwrap_err();
    assert_eq!(unknown.code, "unknown_element", "{unknown:?}");
    assert_eq!(undo_depth(&view, cx), depth);
    assert_eq!(
        import_state(&view, cx)["state"],
        "loaded",
        "the finished chain is untouched"
    );
}

#[gpui_kit::test]
fn without_a_workspace_the_attach_is_kept_and_the_load_says_why_it_could_not_start(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    let authoring = Authoring::new("no-workspace");
    let workspace = view.read_with(cx, |view, _| view.state.project.workspace.clone());
    assert!(workspace.is_none(), "the fixture project was never saved");

    run_attach(
        &view,
        cx,
        attach(
            &authoring,
            &[("source", &authoring.folder().to_string_lossy())],
        ),
    )
    .expect("the attach job starts; the load's refusal is on the node");
    let state = settle(&view, cx);
    assert_eq!(scene_ids(&view, cx), vec!["street".to_string()]);
    assert_eq!(state["state"], "failed", "{state}");
    assert_eq!(state["error"]["step"], "adopt");
    assert_eq!(state["error"]["code"], "no_workspace", "{state}");
    assert_eq!(state["job"], Value::Null);

    // The same refusal from the second dialog is an error of the dialog: nothing was attached.
    let error = submit(
        &view,
        cx,
        FlowKind::SceneResources,
        &[
            ("scene", "street"),
            ("source", &authoring.folder().to_string_lossy()),
        ],
    )
    .unwrap_err();
    assert_eq!(error.code, "no_workspace");
}

#[gpui_kit::test]
fn the_scene_flows_are_in_the_automation_tree_and_the_header(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    for id in ["flow.attach_scene", "flow.scene_resources"] {
        let node = view
            .read_with(cx, |view, cx| view.tree_node(id, cx))
            .unwrap_or_else(|| panic!("{id} is not in the tree"));
        assert_eq!(node["enabled"], true, "{id}");
        assert_eq!(node["actions"][0]["action"], "open", "{id}");
    }
    // The header button is the same gesture as `flow.attach_scene` (`workflow_invoke`).
    let button = crate::header::node("node:26").expect("the header declares the button");
    assert_eq!(button["label"], "Anexar cena");
    assert_eq!(button["kind"], "button");
    assert_eq!(import_state(&view, cx), json!({ "state": "idle" }));
}

#[test]
fn an_attach_failure_without_an_id_has_a_readable_scene_label() {
    for id in ["", "street"] {
        let mut request = plan(None, true);
        request.scene = id.to_string();
        let mut chain = SceneImport::attaching(request, json!({"op":"scene.attach.start"}));
        chain.fail(&CommandError::new("invalid_bundle", "invalid bundle"));
        let summary = chain.summary();
        assert!(!summary.contains("«»"), "{summary}");
        assert!(summary.contains("invalid bundle"), "{summary}");
        if !id.is_empty() {
            assert!(summary.contains("«street»"), "{summary}");
        }
    }
}

/// F3 (BL094.6): a `job.discard` that reaches the attach job before the window's tick used to take
/// its slot away, so the tick never reported the job: the chain stayed `running` for good and
/// `scene_import_busy` refused every new attach until the window was restarted.
#[gpui_kit::test]
fn discarding_the_attach_job_before_the_tick_ends_the_chain_instead_of_leaving_it_running(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    let authoring = Authoring::new("discard-attach");
    project(&view, cx, &authoring);
    let answer = run_attach(
        &view,
        cx,
        attach(
            &authoring,
            &[("source", &authoring.folder().to_string_lossy())],
        ),
    )
    .unwrap();
    let job = answer["import"]["job"]
        .as_str()
        .expect("the chain waits for the attach job")
        .to_string();

    // The attach job ends in the background; no tick has run, so nothing reported it.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let report = view.read_with(cx, |view, _| view.jobs_report());
        let info = report["all"]
            .as_array()
            .and_then(|jobs| jobs.iter().find(|info| info["id"] == job.as_str()))
            .cloned()
            .expect("the attach job is in the registry");
        if matches!(
            info["state"].as_str(),
            Some("succeeded" | "failed" | "cancelled")
        ) {
            assert_eq!(info["result_state"], "pending", "{info}");
            break;
        }
        assert!(Instant::now() < deadline, "the attach job never ended");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(import_state(&view, cx)["state"], "running");

    // The discard arrives first.
    let discarded = view
        .update(cx, |view, cx| {
            view.run_op(&json!({"op":"job.discard","job":job}), cx)
        })
        .expect("job.discard is accepted");
    let OpReply::Done(discarded) = discarded else {
        panic!("job.discard answers at once");
    };
    assert_eq!(discarded["state"], "discarding", "{discarded}");

    // The tick reports the job that was discarded, and the chain ends instead of waiting for it.
    let state = settle(&view, cx);
    assert_eq!(state["state"], "failed", "{state}");
    assert_eq!(state["error"]["step"], "attach", "{state}");
    assert_eq!(state["error"]["code"], "job_discarded", "{state}");
    assert!(scene_ids(&view, cx).is_empty(), "nothing was attached");
    assert_eq!(undo_depth(&view, cx), 0);
    let report = view.read_with(cx, |view, _| view.jobs_report());
    assert!(
        report["recent"].as_array().unwrap().iter().any(|event| {
            event["job"] == job.as_str() && event["error"]["code"] == "job_discarded"
        }),
        "the event of the discarded job is in the history: {report}"
    );

    // The window is not stuck: a new attach is accepted and runs to the end.
    run_attach(
        &view,
        cx,
        attach(
            &authoring,
            &[("source", &authoring.folder().to_string_lossy())],
        ),
    )
    .expect("the chain is no longer busy");
    assert_eq!(settle(&view, cx)["state"], "loaded");
    assert_eq!(scene_ids(&view, cx).len(), 1);
}
