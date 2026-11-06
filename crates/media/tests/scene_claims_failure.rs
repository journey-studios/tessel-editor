//! BL111 (F040): a claims ledger that stops being writable **after** an adoption has started
//! copying.
//!
//! `scene_claims_processes.rs` proves the ledger is refused *before* anything is copied (an
//! unreadable ledger). Here the ledger opens and passes the first write, and then a later write
//! fails (`claims.json.tmp` is a folder, so the atomic replace cannot even start). That is a
//! different case: the copy has already happened, so the answer must not claim "nothing was
//! changed", and the file that was copied but could not be claimed must not be left behind as a
//! file nobody owns (a discard would keep it as "an accepted result's", `recover` would never
//! find it).
//!
//! What is proven:
//!
//! * the adoption fails with `claims_unavailable` instead of succeeding without a claim;
//! * the copy that could not be claimed is taken back at once, under the root lease;
//! * the ledger on disk never lists a file that was not claimed, and keeps listing what it did
//!   claim, so a later discard or `recover` can still decide about it;
//! * a file another pending result holds is never removed by the refused adoption, and stays
//!   removable by its own holder afterwards;
//! * a cancellation that races with the failure still cancels and still leaves no copy.
//!
//! Not proven: a disk that fails in some other way (full, read-only), Windows, network
//! filesystems, a hostile writer.

mod directed_resources_common;

use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};

use directed_resources_common::*;
use serde_json::Value;
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::jobs::{CancelToken, JobError};
use tessel_editor_media::directed_resources::Options;
use tessel_editor_media::scene_resources::{
    Adoption, ResourceBase, adopt, claims_status, recover, tracked_under,
};

struct World {
    sandbox: Sandbox,
    scene: DirectedScene,
    workspace: PathBuf,
}

fn world(name: &str) -> World {
    let sandbox = Sandbox::new(&format!("te-claimfail-{name}"));
    let workspace = sandbox.root.join("ws");
    fs::create_dir_all(&workspace).unwrap();
    let scene = scene(standard_bundle(&sandbox));
    World {
        sandbox,
        scene,
        workspace,
    }
}

impl World {
    fn ledger(&self) -> PathBuf {
        self.workspace.join(".tessel-claims")
    }

    /// What makes the next write of the ledger fail: its temporary file is a folder.
    fn block_writes(&self) {
        assert!(
            self.ledger().join("claims.json").is_file(),
            "the ledger was opened and written once before it broke"
        );
        fs::create_dir(self.ledger().join("claims.json.tmp")).unwrap();
    }

    fn unblock_writes(&self) {
        fs::remove_dir(self.ledger().join("claims.json.tmp")).unwrap();
    }

    fn backdrop(&self) -> PathBuf {
        self.workspace.join("scenes/street/assets/backdrop.png")
    }

    fn figure(&self) -> PathBuf {
        self.workspace.join("scenes/street/assets/figure.png")
    }

    /// Adopts with a hook that runs once progress is reported: the preflight (folders and their
    /// claims) is done and nothing was published yet.
    fn adopt_breaking_after_preflight(&self, token: &CancelToken) -> Result<Adoption, JobError> {
        let broken = Cell::new(false);
        self.adopt_with(token, &mut |_| {
            if !broken.replace(true) {
                self.block_writes();
            }
        })
    }

    fn adopt_with(
        &self,
        token: &CancelToken,
        on_progress: &mut dyn FnMut(tessel_editor_media::directed_resources::Progress),
    ) -> Result<Adoption, JobError> {
        adopt(
            &self.scene,
            "street",
            &self.workspace,
            &ResourceBase::explicit(&self.sandbox.src).unwrap(),
            &Options::default(),
            token,
            on_progress,
            &mut |_| {},
        )
    }

    fn adopt(&self) -> Adoption {
        self.adopt_with(&CancelToken::default(), &mut |_| {})
            .unwrap()
    }
}

/// The entries the ledger on disk lists, as `(key, kind)`.
fn listed(workspace: &Path) -> Vec<(String, String)> {
    let text = fs::read_to_string(workspace.join(".tessel-claims/claims.json")).unwrap();
    let value: Value = serde_json::from_str(&text).unwrap();
    value["entries"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(key, entry)| (key.clone(), entry["kind"].as_str().unwrap().to_string()))
        .collect()
}

fn listed_files(workspace: &Path) -> Vec<String> {
    listed(workspace)
        .into_iter()
        .filter(|(_, kind)| kind == "file")
        .map(|(key, _)| key)
        .collect()
}

fn details(error: &JobError) -> &Value {
    error.details.as_ref().expect("the refusal explains itself")
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_claim_that_cannot_be_written_after_the_copy_refuses_the_adoption() {
    let world = world("refused");
    let error = world
        .adopt_breaking_after_preflight(&CancelToken::default())
        .expect_err("an adoption whose claims cannot be recorded must not succeed");

    assert_eq!(error.code, "claims_unavailable", "{error:?}");
    assert!(
        !error.message.contains("nothing was changed"),
        "the copy had begun, so the message must not say it had not: {}",
        error.message
    );
    assert_eq!(details(&error)["phase"], "after_changes", "{error:?}");
    assert!(
        error.message.contains("taken back"),
        "it says what happened to the copies: {}",
        error.message
    );

    // No copy of the scene is left lying around unowned.
    assert!(!world.backdrop().exists(), "{:?}", tree(&world.workspace));
    assert!(!world.figure().exists(), "{:?}", tree(&world.workspace));
    // The ledger never claimed a file that was not kept.
    assert_eq!(listed_files(&world.workspace), Vec::<String>::new());
    // What could not be tidied (the folders: the ledger cannot record leaving them) is listed,
    // and stays claimed instead of being forgotten.
    assert!(
        !strings(&details(&error)["cleanup"]["kept"]).is_empty(),
        "{error:?}"
    );
}

#[test]
fn a_file_taken_back_cannot_be_found_by_the_recovery_nor_kept_by_a_discard() {
    let world = world("recover");
    world
        .adopt_breaking_after_preflight(&CancelToken::default())
        .unwrap_err();
    world.unblock_writes();

    let recovery = recover(&world.workspace, &CancelToken::default()).unwrap();
    assert_eq!(
        recovery.report.removed_files, 0,
        "nothing was left to recover"
    );
    assert!(!world.backdrop().exists());
    assert!(!world.figure().exists());
    // The workspace now looks only at what really remains: claimed folders of a holder that is
    // this very process (alive), none of them a file.
    assert_eq!(listed_files(&world.workspace), Vec::<String>::new());
}

#[test]
fn a_failure_after_some_files_were_claimed_keeps_only_the_claimed_ones() {
    let world = world("midway");
    let published = Cell::new(0);
    let first = std::cell::RefCell::new(String::new());
    let error = adopt(
        &world.scene,
        "street",
        &world.workspace,
        &ResourceBase::explicit(&world.sandbox.src).unwrap(),
        &Options::default(),
        &CancelToken::default(),
        &mut |_| {},
        &mut |item| {
            // The first file is published and claimed; the ledger breaks before the second.
            if published.replace(published.get() + 1) == 0 {
                *first.borrow_mut() = item.path.as_str().to_string();
                world.block_writes();
            }
        },
    )
    .expect_err("the second claim cannot be written");

    assert_eq!(error.code, "claims_unavailable", "{error:?}");
    let first = first.into_inner();
    let (claimed, unclaimed) = if first.ends_with("backdrop.png") {
        (world.backdrop(), world.figure())
    } else {
        (world.figure(), world.backdrop())
    };
    assert!(!unclaimed.exists(), "the copy nobody could claim is gone");
    // The one that was claimed stays, and the ledger still lists it: with the ledger broken a
    // discard cannot record leaving it, so it is kept (and said so) for a later recovery.
    assert!(claimed.exists());
    let files = listed_files(&world.workspace);
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(files[0].ends_with(claimed.file_name().unwrap().to_str().unwrap()));
    assert!(
        strings(&details(&error)["cleanup"]["kept"])
            .iter()
            .any(|line| line.contains("cannot vouch")),
        "{error:?}"
    );
}

#[test]
fn a_refused_adoption_never_removes_what_another_pending_result_relies_on() {
    let world = world("live-holder");
    let mut holder = world.adopt();
    let mut pending = holder.provisional();
    assert_eq!(listed_files(&world.workspace).len(), 2);

    // A second adoption reuses both files; its claims cannot be written.
    let error = world
        .adopt_breaking_after_preflight(&CancelToken::default())
        .expect_err("the reuse cannot be claimed");
    assert_eq!(error.code, "claims_unavailable", "{error:?}");

    assert!(world.backdrop().is_file(), "the live holder's file stays");
    assert!(world.figure().is_file(), "the live holder's file stays");
    assert_eq!(
        listed_files(&world.workspace).len(),
        2,
        "and its claims are still in the ledger"
    );

    // The holder still decides: once the ledger works again, accepting keeps both files.
    world.unblock_writes();
    pending.accept().unwrap();
    assert!(world.backdrop().is_file());
    assert!(world.figure().is_file());
    assert_eq!(
        tracked_under(&world.workspace),
        0,
        "accepted: nobody's to remove"
    );
    assert!(!world.ledger().exists(), "{:?}", tree(&world.workspace));
    drop(pending);
}

#[test]
fn a_cancellation_that_races_the_failure_still_cancels_and_leaves_no_copy() {
    let world = world("cancel");
    let token = CancelToken::default();
    let published = Cell::new(false);
    let broken = Cell::new(false);
    let error = adopt(
        &world.scene,
        "street",
        &world.workspace,
        &ResourceBase::explicit(&world.sandbox.src).unwrap(),
        &Options::default(),
        &token,
        &mut |_| {
            if !broken.replace(true) {
                world.block_writes();
            }
        },
        &mut |_| {
            if !published.replace(true) {
                token.cancel();
            }
        },
    )
    .expect_err("cancelled");

    assert!(error.is_cancelled(), "{error:?}");
    assert!(!world.backdrop().exists() && !world.figure().exists());
    assert_eq!(listed_files(&world.workspace), Vec::<String>::new());
    let leftovers = error
        .details
        .as_ref()
        .and_then(|details| details.get("rollbackLeftovers"));
    assert!(
        leftovers.is_none(),
        "a copy already taken back is not a leftover: {leftovers:?}"
    );
}

#[test]
fn a_healthy_adoption_is_unchanged_by_the_failure_path() {
    let world = world("healthy");
    let mut adoption = world.adopt();
    assert_eq!(adoption.report.copied(), 2);
    assert_eq!(listed_files(&world.workspace).len(), 2);
    let report = adoption.provisional().discard();
    assert!(report.is_clean(), "{report:?}");
    assert_eq!(report.removed_files, 2);
    assert_eq!(tracked_under(&world.workspace), 0);
    assert!(!world.ledger().exists(), "{:?}", tree(&world.workspace));
    assert_eq!(claims_status(&world.workspace)["available"], true);
}
