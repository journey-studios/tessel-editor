//! BL089 (F040): an adoption or an import whose result is cancelled, failed, late or discarded
//! removes the files it created and nothing else.
//!
//! Every case drives the real `adopt` / `import::{prepare_observed, apply}` / `MediaHost` APIs
//! with synthetic PNGs. Cancellation and races come from the callbacks the APIs already expose
//! (`on_progress`, `on_published`), never from timing, so each run is deterministic. What stays
//! unproven: Windows, a hostile writer (the lease is cooperative), a full disk, a window.

mod directed_resources_common;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use directed_resources_common::*;
use serde_json::{Value, json};
use tessel_editor_core::jobs::{CancelToken, JobError};
use tessel_editor_core::{CommandError, EditorState, headless, persist};
use tessel_editor_media::directed_resources::{Options, Progress, RootLease};
use tessel_editor_media::host::MediaHost;
use tessel_editor_media::scene_resources::import::{self, PreparedSceneImport};
use tessel_editor_media::scene_resources::{
    AdoptionTarget, PendingAdoption, ResourceBase, adopt, tracked_under,
};

/// Names a throwaway tree so a leftover can be told from another suite's (`te-r5-` prefix).
fn sandbox(name: &str) -> Sandbox {
    Sandbox::new(&format!("te-r5-{name}"))
}

fn attach(state: &mut EditorState, sandbox: &Sandbox, id: &str, bundle: &Value) {
    let path = sandbox
        .root
        .join("bundles")
        .join(format!("{id}.bundle.json"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, serde_json::to_vec_pretty(bundle).unwrap()).unwrap();
    headless::execute(state, &json!({"op":"scene.attach","id":id,"path":path})).unwrap();
}

fn new_project(name: &str) -> EditorState {
    let mut state = EditorState::demo();
    headless::execute(&mut state, &json!({"op":"project.create","name":name})).unwrap();
    state
}

/// A bundle whose two images live in `base/assets/`, with `shade` making the bytes unique.
fn bundle_in(base: &Path, shade: u8) -> Value {
    let mut bundle = fixture();
    let backdrop = write(base, "assets/backdrop.png", &png(64, 36, shade));
    let figure = write(
        base,
        "assets/figure.png",
        &png(32, 32, shade.wrapping_add(1)),
    );
    restate(&mut bundle, "backdrop", "assets/backdrop.png", &backdrop);
    restate(&mut bundle, "figure", "assets/figure.png", &figure);
    bundle
}

/// A destination project saved in `ws/` and a source project with the scenes `street` and
/// `market`, whose files sit in two different folders the import is authorized to read.
struct Fixture {
    sandbox: Sandbox,
    destination: EditorState,
    source: EditorState,
    workspace: PathBuf,
    bases: BTreeMap<String, PathBuf>,
}

fn import_fixture(name: &str) -> Fixture {
    let sandbox = sandbox(name);
    let workspace = sandbox.root.join("ws");
    fs::create_dir_all(&workspace).unwrap();
    let mut destination = new_project("destination");
    persist::save(&mut destination, &workspace.join("p.tessel.json")).unwrap();
    let mut source = new_project("source");
    let mut bases = BTreeMap::new();
    for (index, id) in ["street", "market"].into_iter().enumerate() {
        let base = sandbox.root.join("authored").join(id);
        let bundle = bundle_in(&base, 10 + 40 * index as u8);
        attach(&mut source, &sandbox, id, &bundle);
        bases.insert(id.to_string(), base);
    }
    Fixture {
        sandbox,
        destination,
        source,
        workspace,
        bases,
    }
}

impl Fixture {
    fn wanted() -> Vec<String> {
        vec!["street".to_string(), "market".to_string()]
    }

    fn prepare(
        &self,
        token: &CancelToken,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<PreparedSceneImport, JobError> {
        self.prepare_observed(token, on_progress, &mut |_, _| {})
    }

    fn prepare_observed(
        &self,
        token: &CancelToken,
        on_progress: &mut dyn FnMut(Progress),
        on_published: &mut dyn FnMut(&str, tessel_editor_media::directed_resources::Published<'_>),
    ) -> Result<PreparedSceneImport, JobError> {
        import::prepare_observed(
            &self.destination,
            &self.source,
            &self.workspace,
            &Self::wanted(),
            &self.bases,
            token,
            on_progress,
            on_published,
        )
    }

    fn prepared(&self) -> PreparedSceneImport {
        self.prepare(&CancelToken::default(), &mut |_| {}).unwrap()
    }

    fn scenes(&self) -> PathBuf {
        self.workspace.join("scenes")
    }
}

// --------------------------------------------------------------------------- adopt --

fn adoption_world(
    name: &str,
) -> (
    Sandbox,
    tessel_editor_core::directed_scene::DirectedScene,
    PathBuf,
) {
    let sandbox = sandbox(name);
    let workspace = sandbox.root.join("ws");
    fs::create_dir_all(&workspace).unwrap();
    let bundle = standard_bundle(&sandbox);
    (sandbox, scene(bundle), workspace)
}

fn run_adopt(
    scene: &tessel_editor_core::directed_scene::DirectedScene,
    workspace: &Path,
    source: &Path,
    token: &CancelToken,
    on_published: &mut dyn FnMut(tessel_editor_media::directed_resources::Published<'_>),
) -> Result<tessel_editor_media::scene_resources::Adoption, JobError> {
    adopt(
        scene,
        "street",
        workspace,
        &ResourceBase::explicit(source).unwrap(),
        &Options::default(),
        token,
        &mut |_| {},
        on_published,
    )
}

#[test]
fn an_adoption_records_the_files_and_folders_it_made_and_nothing_on_a_reuse() {
    let (sandbox, scene, workspace) = adoption_world("adopt-record");
    let first = run_adopt(
        &scene,
        &workspace,
        &sandbox.src,
        &CancelToken::default(),
        &mut |_| {},
    )
    .unwrap();
    let names = |paths: Vec<&Path>| -> Vec<String> {
        paths
            .iter()
            .map(|path| path.strip_prefix(&workspace).unwrap().display().to_string())
            .collect()
    };
    assert_eq!(
        names(first.created.dirs().collect()),
        ["scenes", "scenes/street", "scenes/street/assets"]
    );
    let mut files = names(first.created.files().collect());
    files.sort();
    assert_eq!(
        files,
        [
            "scenes/street/assets/backdrop.png",
            "scenes/street/assets/figure.png"
        ]
    );

    // Run again: every file is reused, so there is nothing of this call's to take back.
    let again = run_adopt(
        &scene,
        &workspace,
        &sandbox.src,
        &CancelToken::default(),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(again.report.reused(), 2);
    assert!(again.created.is_empty(), "{:?}", again.created);
}

#[test]
fn discarding_an_adoption_into_an_existing_folder_leaves_what_was_already_there() {
    let (sandbox, scene, workspace) = adoption_world("adopt-existing-folder");
    let assets = workspace.join("scenes/street/assets");
    fs::create_dir_all(&assets).unwrap();
    fs::write(assets.join("notes.txt"), b"mine").unwrap();
    let before = tree(&workspace);

    let mut adoption = run_adopt(
        &scene,
        &workspace,
        &sandbox.src,
        &CancelToken::default(),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(
        adoption.created.dirs().count(),
        0,
        "the folders were not made by this call"
    );
    let report = adoption.provisional().discard();
    assert_eq!(report.removed_files, 2, "{report:?}");
    assert_eq!(report.removed_dirs, 0);
    assert!(report.is_clean(), "{report:?}");
    assert_eq!(tree(&workspace), before);
    assert_eq!(fs::read(assets.join("notes.txt")).unwrap(), b"mine");
}

#[test]
fn a_cancelled_adoption_removes_only_its_own_files_and_folders() {
    let (sandbox, scene, workspace) = adoption_world("adopt-cancel-own");
    // The workspace already holds another scene's folder and a file of the user's in this one.
    fs::create_dir_all(workspace.join("scenes/other")).unwrap();
    fs::write(workspace.join("scenes/other/keep.txt"), b"other scene").unwrap();
    fs::create_dir_all(workspace.join("scenes/street")).unwrap();
    fs::write(workspace.join("scenes/street/notes.txt"), b"mine").unwrap();
    let before = tree(&workspace);

    let token = CancelToken::default();
    let cancel = token.clone();
    let error = run_adopt(&scene, &workspace, &sandbox.src, &token, &mut |_| {
        cancel.cancel()
    })
    .unwrap_err();
    assert!(error.is_cancelled(), "{error:?}");
    assert_eq!(tree(&workspace), before);
    assert_eq!(
        fs::read(workspace.join("scenes/street/notes.txt")).unwrap(),
        b"mine"
    );
}

#[test]
fn a_provisional_adoption_removes_on_drop_and_keeps_once_released() {
    let (sandbox, scene, workspace) = adoption_world("adopt-provisional");
    let before = tree(&workspace);

    let mut adoption = run_adopt(
        &scene,
        &workspace,
        &sandbox.src,
        &CancelToken::default(),
        &mut |_| {},
    )
    .unwrap();
    assert_ne!(tree(&workspace), before);
    drop(adoption.provisional());
    assert_eq!(
        tree(&workspace),
        before,
        "dropping the owner takes the files back"
    );

    let mut adoption = run_adopt(
        &scene,
        &workspace,
        &sandbox.src,
        &CancelToken::default(),
        &mut |_| {},
    )
    .unwrap();
    let mut guard = adoption.provisional();
    let accepted = guard.release();
    drop(guard);
    assert_eq!(accepted.files().count(), 2);
    assert!(
        workspace
            .join("scenes/street/assets/backdrop.png")
            .is_file(),
        "a released result is the project's"
    );
}

/// A project with the scene `street` attached and an adoption of its files already done, as the
/// job of `scene.resources.adopt` leaves it before the result is applied.
fn pending_adoption(name: &str) -> (Sandbox, EditorState, PathBuf, PendingAdoption) {
    let sandbox = sandbox(name);
    let workspace = sandbox.root.join("ws");
    fs::create_dir_all(&workspace).unwrap();
    let mut state = new_project("adopt");
    // The host only adopts into a project that has a workspace (`no_workspace` otherwise).
    persist::save(&mut state, &workspace.join("p.tessel.json")).unwrap();
    let bundle = standard_bundle(&sandbox);
    attach(&mut state, &sandbox, "street", &bundle);
    let target = AdoptionTarget::of(&state, "street");
    let scene = state.scenes.get("street").unwrap().scene.clone();
    let mut adoption = run_adopt(
        &scene,
        &workspace,
        &sandbox.src,
        &CancelToken::default(),
        &mut |_| {},
    )
    .unwrap();
    let pending = PendingAdoption::new(target, &mut adoption);
    (sandbox, state, workspace, pending)
}

#[test]
fn an_adoption_applied_to_the_project_it_was_started_for_keeps_its_files() {
    let (_sandbox, state, workspace, pending) = pending_adoption("pending-accepted");
    pending.apply(&state).unwrap();
    assert!(
        workspace
            .join("scenes/street/assets/backdrop.png")
            .is_file()
    );
    assert!(workspace.join("scenes/street/assets/figure.png").is_file());
}

#[test]
fn an_adoption_that_finishes_after_a_project_switch_removes_what_it_copied() {
    let (_sandbox, mut state, workspace, pending) = pending_adoption("pending-switch");
    headless::execute(&mut state, &json!({"op":"project.create","name":"next"})).unwrap();
    let error = pending.apply(&state).unwrap_err();
    assert_eq!(error.code, "stale_result", "{error:?}");
    assert!(error.message.contains("2 file(s)"), "{error:?}");
    assert!(!workspace.join("scenes").exists());
}

#[test]
fn an_adoption_that_finishes_after_the_same_project_moved_to_another_workspace_removes_it() {
    let (sandbox, mut state, workspace, pending) = pending_adoption("pending-workspace");
    // The same project ID, opened from a copy of its folder: the scene's managed base derives
    // from that other workspace, where these files are not.
    let elsewhere = sandbox.root.join("copy");
    fs::create_dir_all(&elsewhere).unwrap();
    state.project.workspace = Some(elsewhere);
    let error = pending.apply(&state).unwrap_err();
    assert_eq!(error.code, "stale_result", "{error:?}");
    assert!(error.message.contains("another workspace"), "{error:?}");
    assert!(!workspace.join("scenes").exists());
}

#[test]
fn the_same_workspace_spelled_another_way_is_not_a_change() {
    let (_sandbox, mut state, workspace, pending) = pending_adoption("pending-spelling");
    state.project.workspace = Some(workspace.join("..").join("ws"));
    pending.apply(&state).unwrap();
    assert!(workspace.join("scenes/street/assets/figure.png").is_file());
}

#[test]
fn an_adoption_whose_scene_was_detached_meanwhile_removes_what_it_copied() {
    let (_sandbox, mut state, workspace, pending) = pending_adoption("pending-detach");
    headless::execute(&mut state, &json!({"op":"scene.detach","scene":"street"})).unwrap();
    let error = pending.apply(&state).unwrap_err();
    assert_eq!(error.code, "stale_result", "{error:?}");
    assert!(!workspace.join("scenes").exists());
}

#[test]
fn an_adoption_whose_bundle_revision_changed_meanwhile_removes_what_it_copied() {
    let (sandbox, mut state, workspace, pending) = pending_adoption("pending-revision");
    headless::execute(&mut state, &json!({"op":"scene.detach","scene":"street"})).unwrap();
    let mut other = standard_bundle(&sandbox);
    other["revisionId"] = json!("rev-2");
    attach(&mut state, &sandbox, "street", &other);
    let error = pending.apply(&state).unwrap_err();
    assert_eq!(error.code, "stale_result", "{error:?}");
    assert!(!workspace.join("scenes").exists());
}

#[test]
fn an_adoption_result_that_is_never_applied_removes_what_it_copied() {
    let (_sandbox, _state, workspace, pending) = pending_adoption("pending-dropped");
    assert!(workspace.join("scenes/street").is_dir());
    drop(pending);
    assert!(!workspace.join("scenes").exists());
}

// --------------------------------------------------------------------------- import --

#[test]
fn an_import_cancelled_while_copying_its_second_scene_takes_back_the_first() {
    let fixture = import_fixture("cancel-second");
    let before = tree(&fixture.workspace);
    let token = CancelToken::default();
    let cancel = token.clone();
    let mut calls = 0;
    // Calls 1 and 2 stage `street`'s files; call 3 is the first file of `market`.
    let error = fixture
        .prepare(&token, &mut |_| {
            calls += 1;
            if calls == 3 {
                cancel.cancel();
            }
        })
        .unwrap_err();
    assert!(error.is_cancelled(), "{error:?}");
    assert_eq!(tree(&fixture.workspace), before, "no scenes/ folder either");
}

#[test]
fn cancelling_right_after_the_last_file_is_published_is_still_a_cancellation() {
    let fixture = import_fixture("cancel-after-last");
    let before = tree(&fixture.workspace);
    let token = CancelToken::default();
    let cancel = token.clone();
    let error = fixture
        .prepare_observed(&token, &mut |_| {}, &mut |scene, published| {
            if scene == "market" && published.done == published.total {
                cancel.cancel();
            }
        })
        .unwrap_err();
    assert!(
        error.is_cancelled(),
        "otherwise the job ends as succeeded and its files are applied: {error:?}"
    );
    assert_eq!(tree(&fixture.workspace), before);
}

#[test]
fn a_failure_in_the_second_scene_takes_back_the_first() {
    let fixture = import_fixture("fail-second");
    fs::remove_file(fixture.bases["market"].join("assets/figure.png")).unwrap();
    let before = tree(&fixture.workspace);
    let error = fixture
        .prepare(&CancelToken::default(), &mut |_| {})
        .unwrap_err();
    assert_eq!(error.code, "resources_invalid", "{error:?}");
    assert_eq!(tree(&fixture.workspace), before);
}

#[test]
fn a_prepared_import_that_is_dropped_unapplied_removes_its_files() {
    let fixture = import_fixture("drop-unapplied");
    let before = tree(&fixture.workspace);
    let prepared = fixture.prepared();
    assert_eq!(prepared.owned().created().files().count(), 4);
    assert_ne!(tree(&fixture.workspace), before);
    // A job slot dropped (job cancelled late, host dropped, a panic unwinding) ends here.
    drop(prepared);
    assert_eq!(tree(&fixture.workspace), before);
}

#[test]
fn applying_keeps_the_files_for_the_project_and_undo_does_not_touch_them() {
    let mut fixture = import_fixture("apply-keeps");
    let prepared = fixture.prepared();
    let result = import::apply(&mut fixture.destination, prepared).unwrap();
    assert_eq!(result["changed"], true, "{result}");
    let mut ids = fixture.destination.scenes.ids();
    ids.sort();
    assert_eq!(ids, ["market", "street"]);
    // The prepared value is gone, and with it its destructor: the files stay.
    let backdrop = fixture.scenes().join("street/assets/backdrop.png");
    assert!(backdrop.is_file());
    assert!(fixture.scenes().join("market/assets/figure.png").is_file());

    headless::execute(&mut fixture.destination, &json!({"op":"history.undo"})).unwrap();
    assert!(fixture.destination.scenes.is_empty());
    assert!(
        backdrop.is_file(),
        "undo is a project edit; files are not rolled back"
    );
}

#[test]
fn an_import_prepared_for_another_project_is_discarded_by_apply() {
    let mut fixture = import_fixture("stale-project");
    let before = tree(&fixture.workspace);
    let prepared = fixture.prepared();
    fixture.destination.project.id = "another-project".to_string();
    let error = import::apply(&mut fixture.destination, prepared).unwrap_err();
    assert_eq!(error.code, "stale_result");
    assert!(fixture.destination.scenes.is_empty());
    assert_eq!(tree(&fixture.workspace), before);
}

#[test]
fn the_same_project_opened_from_another_workspace_makes_the_import_stale() {
    let mut fixture = import_fixture("stale-workspace");
    let before = tree(&fixture.workspace);
    let prepared = fixture.prepared();
    // A copy of the project folder is the same project ID in another place: the copied files
    // would not be where the scenes' managed base is derived from.
    let elsewhere = fixture.sandbox.root.join("copy");
    fs::create_dir_all(&elsewhere).unwrap();
    fixture.destination.project.workspace = Some(elsewhere);
    let error = import::apply(&mut fixture.destination, prepared).unwrap_err();
    assert_eq!(error.code, "stale_result", "{error:?}");
    assert!(error.message.contains("workspace"), "{error:?}");
    assert!(fixture.destination.scenes.is_empty());
    assert_eq!(tree(&fixture.workspace), before);
}

#[test]
fn an_attach_the_project_refuses_late_removes_the_files_and_keeps_the_users_scene() {
    let mut fixture = import_fixture("late-reject");
    let before = tree(&fixture.workspace);
    let prepared = fixture.prepared();
    // While the files were copied the user attached a scene that took one of the new IDs.
    let own = bundle_in(&fixture.sandbox.root.join("own"), 200);
    attach(&mut fixture.destination, &fixture.sandbox, "market", &own);
    let summary = fixture.destination.summary();

    let error: CommandError = import::apply(&mut fixture.destination, prepared).unwrap_err();
    assert!(!error.code.is_empty());
    assert_eq!(fixture.destination.scenes.ids(), ["market"]);
    assert_eq!(fixture.destination.summary(), summary, "project untouched");
    assert_eq!(
        tree(&fixture.workspace),
        before,
        "the copied files are gone"
    );
}

#[test]
fn discarding_leaves_a_destination_that_was_already_there_and_the_files_it_reused() {
    let fixture = import_fixture("reuse");
    let market_files = fixture.scenes().join("market");
    let token = CancelToken::default();
    let mut first = true;
    // While `street` is staged another writer fills the folder `market` was planned for: one file
    // of the right content (reusable) and one of the user's.
    let prepared = fixture
        .prepare(&token, &mut |_| {
            if !std::mem::take(&mut first) {
                return;
            }
            // `market`'s own backdrop: shade 10 + 40, as `fixture` authored it.
            write(&market_files, "assets/backdrop.png", &png(64, 36, 50));
            fs::write(market_files.join("notes.txt"), b"mine").unwrap();
        })
        .unwrap();
    let market = prepared
        .scenes
        .iter()
        .find(|scene| scene.planned.to == "market")
        .unwrap();
    assert_eq!(market.reused, 1, "{market:?}");
    assert!(market.copied > 0, "the figure was copied: {market:?}");

    let report = prepared.discard();
    assert_eq!(
        report.removed_files, 3,
        "`street`'s two files and `market`'s figure, not the reused backdrop: {report:?}"
    );
    assert!(
        market_files.join("assets/backdrop.png").is_file(),
        "a reused file stays"
    );
    assert!(!market_files.join("assets/figure.png").exists());
    assert_eq!(fs::read(market_files.join("notes.txt")).unwrap(), b"mine");
    assert!(!fixture.scenes().join("street").exists());
    assert!(
        fixture.scenes().is_dir(),
        "`scenes/` is shared now, so it stays"
    );
}

#[test]
fn a_file_replaced_after_it_was_copied_is_not_removed() {
    let fixture = import_fixture("replaced");
    let prepared = fixture.prepared();
    let figure = fixture.scenes().join("street/assets/figure.png");
    // Another program saves over the file: a new file at the same path.
    fs::remove_file(&figure).unwrap();
    fs::write(&figure, b"edited elsewhere").unwrap();

    let report = prepared.discard();
    assert_eq!(fs::read(&figure).unwrap(), b"edited elsewhere");
    assert!(
        report.kept.iter().any(|entry| entry.contains("figure.png")),
        "the report says why: {report:?}"
    );
    assert_eq!(report.removed_files, 3);
}

#[test]
fn a_root_another_operation_holds_keeps_its_files_instead_of_forcing_them() {
    let fixture = import_fixture("busy-root");
    let prepared = fixture.prepared();
    let root = fixture.scenes().join("street");
    let lease = RootLease::acquire(&root).unwrap();

    let report = prepared.discard();
    assert_eq!(report.busy_roots, 1, "{report:?}");
    assert!(
        root.join("assets/backdrop.png").is_file(),
        "not removed under another lease"
    );
    assert!(
        !fixture.scenes().join("market").exists(),
        "the idle root was cleaned"
    );
    drop(lease);
}

// ----------------------------------------------------------------------------- host --

fn host_world(name: &str) -> (Fixture, EditorState, MediaHost, PathBuf) {
    let fixture = import_fixture(name);
    let mut state = new_project("host");
    let workspace = fixture.workspace.clone();
    persist::save(&mut state, &workspace.join("host.tessel.json")).unwrap();
    // The source is a saved project too: `project.import_scenes` reads a document.
    let mut source = fixture.source.clone();
    let document = fixture.sandbox.root.join("source/source.tessel.json");
    fs::create_dir_all(document.parent().unwrap()).unwrap();
    persist::save(&mut source, &document).unwrap();
    (fixture, state, MediaHost::new(), document)
}

fn run(
    state: &mut EditorState,
    host: &mut MediaHost,
    request: Value,
) -> Result<Value, CommandError> {
    headless::execute_ext(state, None, Some(host), &request)
}

fn start_import(
    fixture: &Fixture,
    state: &mut EditorState,
    host: &mut MediaHost,
    document: &Path,
) -> Value {
    let started = run(
        state,
        host,
        json!({"op":"project.import_scenes","source":document,
            "scene_bases":fixture.bases,"wait":false}),
    )
    .unwrap();
    let job = started["job"].clone();
    let waited = run(state, host, json!({"op":"job.wait","job":job})).unwrap();
    assert_eq!(waited["state"], "succeeded", "{waited}");
    job
}

#[test]
fn an_import_job_nobody_commits_is_cleaned_when_the_host_goes_away() {
    let (fixture, mut state, mut host, document) = host_world("host-dropped");
    let before = tree(&fixture.workspace);
    start_import(&fixture, &mut state, &mut host, &document);
    assert!(
        fixture.scenes().join("street").is_dir(),
        "prepared on the job thread"
    );
    drop(host);
    assert_eq!(tree(&fixture.workspace), before);
    assert!(state.scenes.is_empty());
}

#[test]
fn committing_into_the_same_project_opened_from_another_workspace_is_stale() {
    let (fixture, mut state, mut host, document) = host_world("host-other-workspace");
    let job = start_import(&fixture, &mut state, &mut host, &document);
    let staged = fixture.scenes().join("street");
    assert!(staged.is_dir());

    // The same project (same ID) is opened from a copy of its folder.
    let copy = fixture.sandbox.root.join("copy");
    fs::create_dir_all(&copy).unwrap();
    fs::copy(
        fixture.workspace.join("host.tessel.json"),
        copy.join("host.tessel.json"),
    )
    .unwrap();
    run(
        &mut state,
        &mut host,
        json!({"op":"project.open","path":copy.join("host.tessel.json")}),
    )
    .unwrap();
    assert_eq!(state.project.workspace.as_deref(), Some(copy.as_path()));

    let error = run(&mut state, &mut host, json!({"op":"job.commit","job":job})).unwrap_err();
    assert_eq!(error.code, "stale_result", "{error:?}");
    assert!(state.scenes.is_empty());
    assert!(
        !staged.exists(),
        "the files that were copied for the old workspace are removed"
    );
    assert!(!fixture.scenes().join("market").exists());
}

// ---------------------------------------------------------------------------- BL096 --

/// An adoption of `street` from the folder the fixture authored it in, into the workspace's
/// managed base: the files `prepared()` already copied for the same scene ID.
fn adopt_street(fixture: &Fixture) -> tessel_editor_media::scene_resources::Adoption {
    let scene = fixture.source.scenes.get("street").unwrap().scene.clone();
    adopt(
        &scene,
        "street",
        &fixture.workspace,
        &ResourceBase::explicit(&fixture.bases["street"]).unwrap(),
        &Options::default(),
        &CancelToken::default(),
        &mut |_| {},
        &mut |_| {},
    )
    .unwrap()
}

#[test]
fn discarding_a_prepared_import_keeps_the_files_an_accepted_adoption_reused() {
    let fixture = import_fixture("share-import-then-adopt");
    let before = tree(&fixture.workspace);
    let prepared = fixture.prepared();
    let mut adoption = adopt_street(&fixture);
    assert_eq!(adoption.report.reused(), 2, "{:?}", adoption.report);

    let report = prepared.discard();
    assert_eq!(
        report.removed_files, 2,
        "only `market`'s files go: {report:?}"
    );
    assert!(
        fixture
            .scenes()
            .join("street/assets/backdrop.png")
            .is_file()
    );
    assert!(fixture.scenes().join("street/assets/figure.png").is_file());
    assert!(!fixture.scenes().join("market").exists());

    drop(adoption.provisional().release());
    assert!(fixture.scenes().join("street/assets/figure.png").is_file());
    assert_eq!(tracked_under(&fixture.workspace), 0);
    assert_ne!(tree(&fixture.workspace), before);
}

#[test]
fn an_accepted_import_keeps_its_files_when_an_adoption_that_reused_them_is_dropped() {
    let mut fixture = import_fixture("share-adopt-then-import");
    let prepared = fixture.prepared();
    let mut adoption = adopt_street(&fixture);

    import::apply(&mut fixture.destination, prepared).unwrap();
    adoption.provisional().discard();
    assert!(
        fixture
            .scenes()
            .join("street/assets/backdrop.png")
            .is_file()
    );
    assert!(fixture.scenes().join("market/assets/figure.png").is_file());
    assert_eq!(tracked_under(&fixture.workspace), 0);
}

#[test]
fn everything_goes_when_the_import_and_the_adoption_that_shared_its_files_both_leave() {
    for import_first in [true, false] {
        let fixture = import_fixture(&format!("share-all-leave-{import_first}"));
        let before = tree(&fixture.workspace);
        let prepared = fixture.prepared();
        let mut adoption = adopt_street(&fixture);
        if import_first {
            prepared.discard();
            adoption.provisional().discard();
        } else {
            adoption.provisional().discard();
            prepared.discard();
        }
        assert_eq!(
            tree(&fixture.workspace),
            before,
            "import first: {import_first}"
        );
        assert_eq!(tracked_under(&fixture.workspace), 0);
    }
}

#[test]
fn a_second_import_planned_while_the_first_is_pending_gets_its_own_folders() {
    let fixture = import_fixture("two-imports");
    let first = fixture.prepared();
    let second = fixture.prepared();
    let renamed: Vec<_> = second
        .scenes
        .iter()
        .map(|s| s.planned.to.as_str())
        .collect();
    assert_eq!(
        renamed,
        ["street-2", "market-2"],
        "the plan sees the first one's folders"
    );

    first.discard();
    assert!(
        fixture
            .scenes()
            .join("street-2/assets/backdrop.png")
            .is_file()
    );
    assert!(!fixture.scenes().join("street").exists());
    second.discard();
    assert!(!fixture.scenes().exists());
    assert_eq!(tracked_under(&fixture.workspace), 0);
}

#[test]
fn a_dropped_prepared_import_leaves_no_claim_behind() {
    let fixture = import_fixture("no-claims");
    drop(fixture.prepared());
    assert_eq!(tracked_under(&fixture.workspace), 0);
    let mut accepted = import_fixture("no-claims-accepted");
    let prepared = accepted.prepared();
    assert!(tracked_under(&accepted.workspace) > 0, "pending: held");
    import::apply(&mut accepted.destination, prepared).unwrap();
    assert_eq!(
        tracked_under(&accepted.workspace),
        0,
        "accepted: nobody's to remove"
    );
}
