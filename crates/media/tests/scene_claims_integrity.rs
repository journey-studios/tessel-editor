//! BL111.2 (F040): the two operations that used to ignore a ledger that could not be written,
//! accepting a result and merging two of them, now fail closed.
//!
//! * **Accepting** (`import::apply`, `PendingAdoption::apply`) is the last step of applying a
//!   result and it is fallible: when the ledger cannot say the files are the project's, the
//!   project is exactly as it was (no scene, no undo step, same revision) and the result stays
//!   claimed, never reported as accepted. Silently ignoring that write left the files claimed by
//!   a process that then died, and `recover` removed files an accepted result owned.
//! * **Merging** (`absorb`, one scene's files joining the import) writes the transfer before it
//!   moves anything in memory: a refusal returns the owner intact, so nobody loses ownership.
//!
//! The write failure is provoked the way `scene_claims_failure.rs` does: `claims.json.tmp` is a
//! folder. A bystander adoption keeps the ledger from being emptied (emptying removes the state
//! file instead of replacing it, which needs no temporary file), so the write really happens.
//!
//! Not proven: other kinds of disk failure, Windows, network filesystems, a hostile writer.

mod directed_resources_common;

use std::cell::Cell;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use directed_resources_common::*;
use serde_json::{Value, json};
use tessel_editor_core::jobs::{CancelToken, JobError};
use tessel_editor_core::{EditorState, headless, persist};
use tessel_editor_media::directed_resources::Options;
use tessel_editor_media::scene_resources::import::{self, PreparedSceneImport};
use tessel_editor_media::scene_resources::{
    Adoption, AdoptionTarget, PendingAdoption, ResourceBase, adopt, claims_status, recover,
};

struct World {
    sandbox: Sandbox,
    destination: EditorState,
    source: EditorState,
    workspace: PathBuf,
    bases: BTreeMap<String, PathBuf>,
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

fn world(name: &str) -> World {
    let sandbox = Sandbox::new(&format!("te-integrity-{name}"));
    let workspace = sandbox.root.join("ws");
    fs::create_dir_all(&workspace).unwrap();
    let mut destination = new_project("destination");
    persist::save(&mut destination, &workspace.join("p.tessel.json")).unwrap();
    let mut source = new_project("source");
    let mut bases = BTreeMap::new();
    for (index, id) in ["street", "market"].into_iter().enumerate() {
        let base = sandbox.root.join("authored").join(id);
        attach(
            &mut source,
            &sandbox,
            id,
            &bundle_in(&base, 10 + 40 * index as u8),
        );
        bases.insert(id.to_string(), base);
    }
    World {
        sandbox,
        destination,
        source,
        workspace,
        bases,
    }
}

impl World {
    fn ledger(&self) -> PathBuf {
        self.workspace.join(".tessel-claims")
    }

    fn block_writes(&self) {
        assert!(self.ledger().join("claims.json").is_file());
        fs::create_dir(self.ledger().join("claims.json.tmp")).unwrap();
    }

    fn unblock_writes(&self) {
        fs::remove_dir(self.ledger().join("claims.json.tmp")).unwrap();
    }

    fn prepare(&self, on_published: &mut dyn FnMut(&str)) -> Result<PreparedSceneImport, JobError> {
        self.prepare_scenes(&["street", "market"], on_published)
    }

    fn prepare_scenes(
        &self,
        scenes: &[&str],
        on_published: &mut dyn FnMut(&str),
    ) -> Result<PreparedSceneImport, JobError> {
        let wanted: Vec<String> = scenes.iter().map(|id| id.to_string()).collect();
        import::prepare_observed(
            &self.destination,
            &self.source,
            &self.workspace,
            &wanted,
            &self.bases,
            &CancelToken::default(),
            &mut |_| {},
            &mut |scene, _| on_published(scene),
        )
    }

    /// A pending adoption of another scene that is never settled: it keeps the ledger non-empty.
    fn bystander(&self) -> Adoption {
        let scene = scene(standard_bundle(&self.sandbox));
        adopt(
            &scene,
            "bystander",
            &self.workspace,
            &ResourceBase::explicit(&self.sandbox.src).unwrap(),
            &Options::default(),
            &CancelToken::default(),
            &mut |_| {},
            &mut |_| {},
        )
        .unwrap()
    }

    fn files(&self) -> Vec<PathBuf> {
        ["street", "market"]
            .iter()
            .flat_map(|id| {
                ["backdrop.png", "figure.png"]
                    .iter()
                    .map(move |name| (id, name))
            })
            .map(|(id, name)| self.workspace.join(format!("scenes/{id}/assets/{name}")))
            .collect()
    }
}

fn entries(workspace: &Path) -> Vec<Value> {
    claims_status(workspace)["entries"]
        .as_array()
        .unwrap()
        .clone()
}

fn file_entries(workspace: &Path) -> Vec<Value> {
    entries(workspace)
        .into_iter()
        .filter(|entry| entry["kind"] == "file")
        .collect()
}

// ------------------------------------------------------------------------------ accepting --

#[test]
fn an_import_whose_acceptance_cannot_be_written_attaches_nothing_and_adds_no_undo_step() {
    let mut world = world("import-accept");
    let _bystander = world.bystander();
    let prepared = world.prepare(&mut |_| {}).unwrap();
    let (summary, revision, can_undo) = (
        world.destination.summary(),
        world.destination.project.revision,
        world.destination.history.can_undo(),
    );
    world.block_writes();

    let error = import::apply(&mut world.destination, prepared)
        .expect_err("an acceptance nobody could write down must not apply the import");

    assert_eq!(error.code, "claims_unavailable", "{error:?}");
    let details = error.details.as_ref().expect("it explains itself");
    assert_eq!(details["phase"], "accept", "{error:?}");
    assert!(error.message.contains("no scene was attached"), "{error:?}");
    // The project is exactly as it was: no scene, no undo entry, no new revision.
    assert!(world.destination.scenes.is_empty());
    assert_eq!(world.destination.summary(), summary);
    assert_eq!(world.destination.project.revision, revision);
    assert_eq!(world.destination.history.can_undo(), can_undo);
    // The copied files were not lost nor reported as accepted: the ledger could not record
    // leaving them either, so they are kept, still claimed, and the error says so.
    assert!(world.files().iter().all(|file| file.is_file()));
    assert!(!details["cleanup"]["kept"].as_array().unwrap().is_empty());
    assert!(error.message.contains("left in place"), "{error:?}");
    world.unblock_writes();
    let held = file_entries(&world.workspace);
    assert!(held.len() >= 4 + 2, "still claimed: {held:?}");
    assert!(
        held.iter().all(|entry| entry["orphan"] == false),
        "held by this live process, not orphans: {held:?}"
    );
    // Nothing is taken from a live holder by a recovery.
    let recovery = recover(&world.workspace, &CancelToken::default()).unwrap();
    assert_eq!(recovery.report.removed_files, 0, "{recovery:?}");
    assert!(world.files().iter().all(|file| file.is_file()));
}

#[test]
fn an_import_whose_acceptance_is_written_still_attaches_and_keeps_its_files() {
    let mut world = world("import-ok");
    let _bystander = world.bystander();
    let prepared = world.prepare(&mut |_| {}).unwrap();
    let result = import::apply(&mut world.destination, prepared).unwrap();
    assert_eq!(result["changed"], true);
    assert_eq!(world.destination.scenes.len(), 2);
    assert!(world.destination.history.can_undo());
    assert!(world.files().iter().all(|file| file.is_file()));
    // Accepted: those claims are gone from the ledger, only the bystander's remain.
    let remaining = file_entries(&world.workspace);
    assert_eq!(remaining.len(), 2, "{remaining:?}");
    assert!(
        remaining
            .iter()
            .all(|entry| entry["path"].as_str().unwrap().contains("bystander"))
    );
}

fn pending_adoption(world: &mut World) -> (PendingAdoption, PathBuf) {
    let bundle = standard_bundle(&world.sandbox);
    attach(&mut world.destination, &world.sandbox, "lot", &bundle);
    let target = AdoptionTarget::of(&world.destination, "lot");
    let scene = world.destination.scenes.get("lot").unwrap().scene.clone();
    let mut adoption = adopt(
        &scene,
        "lot",
        &world.workspace,
        &ResourceBase::explicit(&world.sandbox.src).unwrap(),
        &Options::default(),
        &CancelToken::default(),
        &mut |_| {},
        &mut |_| {},
    )
    .unwrap();
    let file = world.workspace.join("scenes/lot/assets/backdrop.png");
    (PendingAdoption::new(target, &mut adoption), file)
}

#[test]
fn an_adoption_whose_acceptance_cannot_be_written_is_refused_and_keeps_its_files() {
    let mut world = world("adopt-accept");
    let _bystander = world.bystander();
    let (pending, backdrop) = pending_adoption(&mut world);
    let summary = world.destination.summary();
    world.block_writes();

    let error = pending
        .apply(&world.destination)
        .expect_err("the files are not the project's until the ledger says so");

    assert_eq!(error.code, "claims_unavailable", "{error:?}");
    assert_eq!(
        error.details.as_ref().unwrap()["phase"],
        "accept",
        "{error:?}"
    );
    assert_eq!(world.destination.summary(), summary);
    assert!(backdrop.is_file(), "kept, never silently removed");
    world.unblock_writes();
    assert!(
        file_entries(&world.workspace)
            .iter()
            .any(|entry| entry["path"]
                .as_str()
                .unwrap()
                .ends_with("lot/assets/backdrop.png")),
        "and still claimed"
    );
}

#[test]
fn a_settle_the_ledger_refuses_returns_the_adoption_still_owning_and_claiming_its_files() {
    // The seam with `job.discard` (BL094/BL112): the window's tick does not remove, so a refused
    // acceptance must hand the owner back instead of discarding or dropping it silently.
    let mut world = world("settle-accept");
    let _bystander = world.bystander();
    let (pending, backdrop) = pending_adoption(&mut world);
    let summary = world.destination.summary();
    world.block_writes();

    let stale = *pending
        .settle(&world.destination)
        .expect_err("not accepted: the ledger could not say the files are the project's");

    assert!(stale.refusal.is_some(), "a ledger refusal, not staleness");
    let error = stale.refusal_error().expect("project-facing error");
    assert_eq!(error.code, "claims_unavailable", "{error:?}");
    assert_eq!(error.details.as_ref().unwrap()["phase"], "accept");
    assert_eq!(world.destination.summary(), summary);
    assert!(
        backdrop.is_file(),
        "nothing was removed on the caller's thread"
    );
    world.unblock_writes();
    let claimed = |world: &World| {
        file_entries(&world.workspace)
            .iter()
            .filter(|entry| entry["path"].as_str().unwrap().contains("/lot/"))
            .count()
    };
    assert_eq!(claimed(&world), 2, "still claimed by the returned owner");

    // Off the UI thread the owner removes what only it created.
    let report = stale.pending.discard();
    assert_eq!(report.removed_files, 2, "{report:?}");
    assert!(!backdrop.exists());
    assert_eq!(claimed(&world), 0);
}

#[test]
fn a_settle_gives_up_inside_the_tick_budget_when_the_ledger_lock_stays_held() {
    // BL112.3: `settle` runs on the window's tick, whose ceiling is 100 ms. The bounded wait of
    // the ledger lock (1.5 s) belongs to callers that may block; the tick waits a fixed 20 ms and
    // refuses (the holder here is a descriptor that never lets go, like a stopped process).
    let mut world = world("settle-busy");
    let _bystander = world.bystander();
    let (pending, backdrop) = pending_adoption(&mut world);
    let summary = world.destination.summary();
    // Another open file description holding the lock is what a stopped process does.
    let stuck = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(world.ledger().join("ledger.lock"))
        .unwrap();
    stuck.lock().unwrap();

    let started = std::time::Instant::now();
    let stale = *pending
        .settle(&world.destination)
        .expect_err("the ledger is busy: the acceptance cannot be recorded");
    let waited = started.elapsed();
    eprintln!(
        "BUDGET {}",
        json!({"measure": "settle-holder-stuck-same-process",
            "settleMs": waited.as_secs_f64() * 1000.0, "ceilingMs": 100})
    );

    // Held to the UI's ceiling, not to the 1.5 s a waiting caller would take.
    assert!(
        waited < std::time::Duration::from_millis(100),
        "the tick waited beyond its ceiling for the lock: {waited:?}"
    );
    let error = stale
        .refusal_error()
        .expect("a ledger refusal, not staleness");
    assert_eq!(error.code, "claims_unavailable", "{error:?}");
    assert_eq!(error.details.as_ref().unwrap()["phase"], "accept");
    assert!(
        error.details.as_ref().unwrap()["why"]
            .as_str()
            .is_some_and(|why| why.contains("gave up after") && !why.contains("another process")),
        "{error:?}"
    );
    assert_eq!(world.destination.summary(), summary);
    assert!(backdrop.is_file(), "nothing was removed on the tick");

    // Once the holder lets go, the owner that came back is still claimed and removes its files.
    stuck.unlock().unwrap();
    drop(stuck);
    let report = stale.pending.discard();
    assert_eq!(report.removed_files, 2, "{report:?}");
    assert!(!backdrop.exists());
}

#[test]
fn a_settle_rides_out_a_ledger_lock_another_job_of_this_process_holds_for_a_moment() {
    // N1: a discard or an adoption of the other slot holds `ledger.lock` for microseconds, and
    // `flock` conflicts between descriptors of the same process. The tick used to look once, so a
    // valid result was refused, its copy removed and "another process" blamed.
    let mut world = world("settle-moment");
    let _bystander = world.bystander();
    let (pending, backdrop) = pending_adoption(&mut world);
    let lock_path = world.ledger().join("ledger.lock");
    let ticking = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (held_tx, held_rx) = std::sync::mpsc::channel();
    let other_job = {
        let ticking = ticking.clone();
        std::thread::spawn(move || {
            let file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(lock_path)
                .unwrap();
            file.lock().unwrap();
            held_tx.send(()).unwrap();
            // Held until the tick has started, and 2 ms more.
            while !ticking.load(std::sync::atomic::Ordering::SeqCst) {
                std::thread::yield_now();
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
            file.unlock().unwrap();
        })
    };
    held_rx.recv().unwrap();
    ticking.store(true, std::sync::atomic::Ordering::SeqCst);
    let looked = std::time::Instant::now();
    let settled = pending.settle(&world.destination);
    eprintln!(
        "BUDGET {}",
        json!({"measure": "settle-holder-2ms-same-process",
            "settleMs": looked.elapsed().as_secs_f64() * 1000.0, "ceilingMs": 100})
    );
    other_job.join().unwrap();
    assert!(
        settled.is_ok(),
        "a holder that lets go within the budget does not cost the result: {:?}",
        settled.err().map(|stale| stale.refusal_error())
    );
    assert!(backdrop.is_file(), "the accepted copy stays");
    assert!(
        file_entries(&world.workspace)
            .iter()
            .all(|entry| !entry["path"].as_str().unwrap().contains("/lot/")),
        "and nobody is left to recover it: the acceptance was recorded"
    );
}

#[test]
fn an_acceptance_that_succeeds_makes_the_files_nobodys_to_recover() {
    let mut world = world("adopt-ok");
    let _bystander = world.bystander();
    let (pending, backdrop) = pending_adoption(&mut world);
    pending.apply(&world.destination).unwrap();
    assert!(backdrop.is_file());
    assert!(
        file_entries(&world.workspace)
            .iter()
            .all(|entry| !entry["path"].as_str().unwrap().contains("/lot/"))
    );
}

// --------------------------------------------------------------------------------- merging --

#[test]
fn a_merge_the_ledger_cannot_write_fails_and_nobody_loses_ownership() {
    let world = world("absorb");
    let _bystander = world.bystander();
    let blocked = Cell::new(false);
    let seen = Cell::new(0);
    // Break the ledger once the first scene's last file is published: the claims of those files
    // are already written, and the next write is the transfer to the import's record.
    // Only one scene, so the merge is the very last thing `prepare` does.
    let error = world
        .prepare_scenes(&["street"], &mut |scene| {
            if scene == "street" {
                seen.set(seen.get() + 1);
                if seen.get() == 2 && !blocked.replace(true) {
                    world.block_writes();
                }
            }
        })
        .expect_err("a merge nobody could write down must not succeed");

    assert!(blocked.get(), "the failure was provoked");
    assert_eq!(error.code, "claims_unavailable", "{error:?}");
    assert_eq!(
        error.details.as_ref().unwrap()["phase"],
        "after_changes",
        "{error:?}"
    );
    // Every copied file of the first scene is still there and still claimed by the holder that
    // made it: not lost, not reported removed.
    let street: Vec<PathBuf> = world
        .files()
        .into_iter()
        .filter(|file| file.to_string_lossy().contains("/street/"))
        .collect();
    assert!(street.iter().all(|file| file.is_file()));
    world.unblock_writes();
    let held: Vec<Value> = file_entries(&world.workspace)
        .into_iter()
        .filter(|entry| entry["path"].as_str().unwrap().contains("/street/"))
        .collect();
    assert_eq!(held.len(), 2, "{held:?}");
    assert!(
        held.iter().all(
            |entry| entry["holders"].as_array().unwrap().len() == 1 && entry["orphan"] == false
        ),
        "{held:?}"
    );
}

#[test]
fn a_merge_that_is_written_hands_everything_to_the_import() {
    let world = world("absorb-ok");
    let prepared = world.prepare(&mut |_| {}).unwrap();
    assert_eq!(prepared.owned().created().files().count(), 4);
    let report = prepared.discard();
    assert!(report.is_clean(), "{report:?}");
    assert_eq!(report.removed_files, 4);
    assert!(!world.ledger().exists());
}

// ------------------------------------------------------------------------ damaged ledger --

#[test]
fn a_ledger_without_its_lock_refuses_every_operation_and_removes_no_file() {
    let mut world = world("dangling");
    let (pending, _backdrop) = pending_adoption(&mut world);
    let before = file_entries(&world.workspace).len();
    assert!(before >= 2);
    fs::remove_file(world.ledger().join("ledger.lock")).unwrap();

    let status = claims_status(&world.workspace);
    assert_eq!(status["available"], false, "{status}");
    let error = recover(&world.workspace, &CancelToken::default()).unwrap_err();
    assert_eq!(error.code, "claims_unavailable", "{error:?}");

    // Not accepted, and a discard cannot vouch for anything: the files stay.
    let error = pending.apply(&world.destination).unwrap_err();
    assert_eq!(error.code, "claims_unavailable", "{error:?}");
    assert!(
        world
            .workspace
            .join("scenes/lot/assets/backdrop.png")
            .is_file()
    );
    assert!(
        world.ledger().join("claims.json").is_file(),
        "what the ledger lists is left for a person to look at"
    );
}

#[test]
fn a_second_scene_merge_failure_reports_files_kept_from_both_scenes() {
    let world = world("second-absorb");
    let _bystander = world.bystander();
    let seen = Cell::new(0);
    let error = world
        .prepare(&mut |scene| {
            if scene == "market" {
                seen.set(seen.get() + 1);
                if seen.get() == 2 {
                    world.block_writes();
                }
            }
        })
        .unwrap_err();
    assert_eq!(seen.get(), 2);
    assert_eq!(error.code, "claims_unavailable");
    let kept = error.details.as_ref().unwrap()["cleanup"]["kept"]
        .as_array()
        .unwrap();
    for file in world.files() {
        assert!(file.exists());
        assert!(
            kept.iter()
                .any(|entry| entry.as_str().unwrap().contains(file.to_str().unwrap())),
            "missing {} from {kept:?}",
            file.display()
        );
    }
    world.unblock_writes();
}
