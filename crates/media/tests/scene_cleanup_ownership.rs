//! BL096 / BL097 (F040): who owns a copied file when two results share it, and what proves a file
//! is still the one an operation published.
//!
//! * **BL096.** Two adoptions into the same scene folder: the second finds the first's files
//!   already there with the expected hash and reuses them. Discarding the first must not remove
//!   what the second (accepted or still pending) relies on, in any order of accept/discard.
//! * **BL097.** A file rewritten in place keeps its inode and, when the writer writes as many
//!   bytes as before, its length: only the content tells it apart from what was published.
//!
//! Every case drives the real `adopt` and its owners with synthetic PNGs; nothing depends on
//! timing. Unproven, on purpose: Windows, a hostile writer (the lease and the claims are
//! cooperative), another *process* sharing the folder (claims live in one process), a window.

mod directed_resources_common;

use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use directed_resources_common::*;
use serde_json::json;
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::jobs::CancelToken;
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::{EditorState, headless};
use tessel_editor_media::directed_resources::Options;
use tessel_editor_media::scene_resources::{
    Adoption, AdoptionTarget, PendingAdoption, ResourceBase, adopt, tracked_under,
};

fn world(name: &str) -> (Sandbox, DirectedScene, PathBuf) {
    let sandbox = Sandbox::new(&format!("te-own-{name}"));
    let workspace = sandbox.root.join("ws");
    fs::create_dir_all(&workspace).unwrap();
    let scene = scene(standard_bundle(&sandbox));
    (sandbox, scene, workspace)
}

fn adopt_into(scene: &DirectedScene, workspace: &Path, source: &Path) -> Adoption {
    adopt(
        scene,
        "street",
        workspace,
        &ResourceBase::explicit(source).unwrap(),
        &Options::default(),
        &CancelToken::default(),
        &mut |_| {},
        &mut |_| {},
    )
    .unwrap()
}

fn backdrop(workspace: &Path) -> PathBuf {
    workspace.join("scenes/street/assets/backdrop.png")
}

fn figure(workspace: &Path) -> PathBuf {
    workspace.join("scenes/street/assets/figure.png")
}

/// Both files and both folders of `scenes/street` are exactly as the adoption left them.
fn assert_intact(workspace: &Path, sandbox: &Sandbox) {
    assert_eq!(
        fs::read(backdrop(workspace)).unwrap(),
        fs::read(sandbox.src.join("assets/backdrop.png")).unwrap()
    );
    assert_eq!(
        fs::read(figure(workspace)).unwrap(),
        fs::read(sandbox.src.join("assets/figure.png")).unwrap()
    );
}

/// Every result ended (accepted or discarded): the registry holds nothing for this workspace.
fn assert_no_claims(workspace: &Path) {
    assert_eq!(tracked_under(workspace), 0, "no claim outlives its results");
}

/// An adoption that reuses what another one copied: its files are not its own.
fn assert_reuse_only(second: &Adoption) {
    assert_eq!(second.report.copied(), 0, "{:?}", second.report);
    assert_eq!(second.report.reused(), 2, "{:?}", second.report);
}

// ------------------------------------------------------------------------------- BL096 --

#[test]
fn discarding_the_first_adoption_keeps_the_files_the_second_one_reused_and_accepted() {
    let (sandbox, scene, workspace) = world("first-discarded");
    let mut first = adopt_into(&scene, &workspace, &sandbox.src);
    let mut second = adopt_into(&scene, &workspace, &sandbox.src);
    assert_reuse_only(&second);

    // The second one has not been accepted yet: its files must survive the first one's discard.
    first.provisional().discard();
    assert_intact(&workspace, &sandbox);

    second.provisional().accept().unwrap();
    assert_intact(&workspace, &sandbox);
    assert_no_claims(&workspace);
}

#[test]
fn accepting_the_second_adoption_first_and_discarding_the_first_after_keeps_the_files() {
    let (sandbox, scene, workspace) = world("second-accepted-first");
    let mut first = adopt_into(&scene, &workspace, &sandbox.src);
    let mut second = adopt_into(&scene, &workspace, &sandbox.src);
    assert_reuse_only(&second);

    second.provisional().accept().unwrap();
    let report = first.provisional().discard();
    assert_intact(&workspace, &sandbox);
    assert!(
        workspace.join("scenes/street/assets").is_dir(),
        "{report:?}"
    );
}

#[test]
fn accepting_the_first_adoption_keeps_the_files_when_the_second_one_that_reused_them_is_discarded()
{
    let (sandbox, scene, workspace) = world("first-accepted");
    let mut first = adopt_into(&scene, &workspace, &sandbox.src);
    let mut second = adopt_into(&scene, &workspace, &sandbox.src);
    assert_reuse_only(&second);

    first.provisional().accept().unwrap();
    second.provisional().discard();
    assert_intact(&workspace, &sandbox);
}

#[test]
fn discarding_the_second_adoption_and_then_accepting_the_first_keeps_the_files() {
    let (sandbox, scene, workspace) = world("second-discarded-first-accepted");
    let mut first = adopt_into(&scene, &workspace, &sandbox.src);
    let mut second = adopt_into(&scene, &workspace, &sandbox.src);

    second.provisional().discard();
    assert_intact(&workspace, &sandbox);
    first.provisional().accept().unwrap();
    assert_intact(&workspace, &sandbox);
}

#[test]
fn dropping_an_adoption_an_accepted_one_had_reused_leaves_the_accepted_files() {
    // The first result was accepted long ago (its owner is gone); a second adoption reuses the
    // files and is dropped unapplied. The files were never the second one's.
    let (sandbox, scene, workspace) = world("accepted-then-dropped");
    let mut first = adopt_into(&scene, &workspace, &sandbox.src);
    first.provisional().accept().unwrap();
    drop(first);

    let mut second = adopt_into(&scene, &workspace, &sandbox.src);
    assert_reuse_only(&second);
    drop(second.provisional());
    assert_intact(&workspace, &sandbox);
}

#[test]
fn when_every_holder_discards_the_files_and_folders_are_removed_in_either_order() {
    for first_goes_first in [true, false] {
        let (sandbox, scene, workspace) = world(&format!("all-discarded-{first_goes_first}"));
        let before = tree(&workspace);
        let mut first = adopt_into(&scene, &workspace, &sandbox.src);
        let mut second = adopt_into(&scene, &workspace, &sandbox.src);
        assert_reuse_only(&second);

        if first_goes_first {
            first.provisional().discard();
            assert_intact(&workspace, &sandbox);
            second.provisional().discard();
        } else {
            second.provisional().discard();
            assert_intact(&workspace, &sandbox);
            first.provisional().discard();
        }
        assert_eq!(
            tree(&workspace),
            before,
            "the last holder takes everything back (first first: {first_goes_first})"
        );
        assert_no_claims(&workspace);
    }
}

#[test]
fn three_holders_keep_the_files_until_the_last_one_leaves_or_one_accepts() {
    let (sandbox, scene, workspace) = world("three-holders");
    let before = tree(&workspace);
    let mut first = adopt_into(&scene, &workspace, &sandbox.src);
    let mut second = adopt_into(&scene, &workspace, &sandbox.src);
    let mut third = adopt_into(&scene, &workspace, &sandbox.src);

    first.provisional().discard();
    second.provisional().discard();
    assert_intact(&workspace, &sandbox);
    third.provisional().discard();
    assert_eq!(tree(&workspace), before);

    // Same trio, but the last one standing is accepted.
    let mut first = adopt_into(&scene, &workspace, &sandbox.src);
    let mut second = adopt_into(&scene, &workspace, &sandbox.src);
    let mut third = adopt_into(&scene, &workspace, &sandbox.src);
    second.provisional().discard();
    third.provisional().accept().unwrap();
    first.provisional().discard();
    assert_intact(&workspace, &sandbox);
    assert_no_claims(&workspace);
}

#[test]
fn a_root_that_was_there_before_stays_whoever_leaves_first() {
    for first_goes_first in [true, false] {
        let (sandbox, scene, workspace) = world(&format!("preexisting-{first_goes_first}"));
        let root = workspace.join("scenes/street");
        fs::create_dir_all(root.join("assets")).unwrap();
        fs::write(root.join("assets/notes.txt"), b"mine").unwrap();
        fs::write(root.join("keep.txt"), b"mine too").unwrap();
        let before = tree(&workspace);
        let mut first = adopt_into(&scene, &workspace, &sandbox.src);
        let mut second = adopt_into(&scene, &workspace, &sandbox.src);
        assert_reuse_only(&second);
        assert_eq!(
            first.created.dirs().count(),
            0,
            "the folders were not made here"
        );

        if first_goes_first {
            first.provisional().discard();
            assert_intact(&workspace, &sandbox);
            second.provisional().discard();
        } else {
            second.provisional().discard();
            assert_intact(&workspace, &sandbox);
            first.provisional().discard();
        }
        assert_eq!(tree(&workspace), before);
        assert_eq!(fs::read(root.join("assets/notes.txt")).unwrap(), b"mine");
        assert_eq!(fs::read(root.join("keep.txt")).unwrap(), b"mine too");
    }
}

#[test]
fn a_root_that_was_there_before_keeps_the_accepted_files_when_the_first_one_is_discarded() {
    let (sandbox, scene, workspace) = world("preexisting-accepted");
    let root = workspace.join("scenes/street");
    fs::create_dir_all(root.join("assets")).unwrap();
    fs::write(root.join("assets/notes.txt"), b"mine").unwrap();
    let mut first = adopt_into(&scene, &workspace, &sandbox.src);
    let mut second = adopt_into(&scene, &workspace, &sandbox.src);

    second.provisional().accept().unwrap();
    first.provisional().discard();
    assert_intact(&workspace, &sandbox);
    assert_eq!(fs::read(root.join("assets/notes.txt")).unwrap(), b"mine");
}

#[test]
fn the_same_workspace_spelled_differently_is_still_one_owner_of_each_file() {
    let (sandbox, scene, workspace) = world("spelling");
    // `ws/../ws` is the same folder through another path: the claims must meet.
    let spelled = workspace.join("..").join("ws");
    let mut first = adopt_into(&scene, &workspace, &sandbox.src);
    let mut second = adopt_into(&scene, &spelled, &sandbox.src);
    assert_reuse_only(&second);

    first.provisional().discard();
    assert_intact(&workspace, &sandbox);
    second.provisional().accept().unwrap();
    assert_intact(&workspace, &sandbox);
}

#[test]
fn an_adoption_into_a_different_scene_does_not_share_anything() {
    let (sandbox, scene, workspace) = world("other-scene");
    let mut first = adopt_into(&scene, &workspace, &sandbox.src);
    let mut other = adopt(
        &scene,
        "market",
        &workspace,
        &ResourceBase::explicit(&sandbox.src).unwrap(),
        &Options::default(),
        &CancelToken::default(),
        &mut |_| {},
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(other.report.copied(), 2, "another folder: its own copies");

    first.provisional().discard();
    assert!(!workspace.join("scenes/street").exists());
    assert!(
        workspace
            .join("scenes/market/assets/backdrop.png")
            .is_file()
    );
    other.provisional().discard();
    assert!(!workspace.join("scenes").exists());
}

#[test]
fn the_report_says_what_was_left_to_another_result_and_is_still_clean() {
    let (sandbox, scene, workspace) = world("report");
    let mut first = adopt_into(&scene, &workspace, &sandbox.src);
    let mut second = adopt_into(&scene, &workspace, &sandbox.src);
    assert_eq!(second.created.relied_on().count(), 5, "2 files + 3 folders");

    let report = first.provisional().discard();
    assert_eq!(report.removed_files, 0, "{report:?}");
    assert_eq!(report.removed_dirs, 0, "{report:?}");
    assert!(report.is_clean(), "nothing was left over: {report:?}");
    assert_eq!(report.shared.len(), 5, "{report:?}");
    assert!(report.to_json()["shared"].is_array());

    let last = second.provisional().discard();
    assert_eq!((last.removed_files, last.removed_dirs), (2, 3), "{last:?}");
    assert!(last.shared.is_empty(), "{last:?}");
}

#[test]
fn a_second_adoption_that_added_a_file_to_the_first_ones_folder_holds_the_folder_too() {
    let (sandbox, scene, workspace) = world("added-file");
    let before = tree(&workspace);
    let mut first = adopt_into(&scene, &workspace, &sandbox.src);
    // The first file is lost from the folder (deleted by hand): the second adoption brings it
    // back into a folder the first one made.
    fs::remove_file(figure(&workspace)).unwrap();
    let mut second = adopt_into(&scene, &workspace, &sandbox.src);
    assert_eq!(second.report.copied(), 1, "{:?}", second.report);
    assert_eq!(second.report.reused(), 1);

    first.provisional().discard();
    assert_intact(&workspace, &sandbox);
    second.provisional().discard();
    assert_eq!(
        tree(&workspace),
        before,
        "the folders go with the last holder"
    );
    assert_no_claims(&workspace);
}

#[test]
fn a_second_adoption_that_fails_before_publishing_holds_nothing() {
    let (sandbox, scene, workspace) = world("second-cancelled");
    let before = tree(&workspace);
    let mut first = adopt_into(&scene, &workspace, &sandbox.src);

    let token = CancelToken::default();
    token.cancel();
    let error = adopt(
        &scene,
        "street",
        &workspace,
        &ResourceBase::explicit(&sandbox.src).unwrap(),
        &Options::default(),
        &token,
        &mut |_| {},
        &mut |_| {},
    )
    .unwrap_err();
    assert!(error.is_cancelled(), "{error:?}");
    assert_intact(&workspace, &sandbox);

    // Had the failed call claimed the files, this discard would leave them behind.
    first.provisional().discard();
    assert_eq!(tree(&workspace), before);
    assert_no_claims(&workspace);
}

#[test]
fn results_sharing_a_folder_across_threads_never_lose_an_accepted_file() {
    // Real concurrency: whatever the interleaving, a result that was accepted keeps its files,
    // however the others discard around it.
    for round in 0..6 {
        let (sandbox, scene, workspace) = world(&format!("threads-{round}"));
        let accepter = round % 4;
        let barrier = std::sync::Barrier::new(4);
        std::thread::scope(|scope| {
            for index in 0..4 {
                let (scene, workspace, source, barrier) =
                    (&scene, &workspace, &sandbox.src, &barrier);
                scope.spawn(move || {
                    let mut adoption = loop {
                        let attempt = adopt(
                            scene,
                            "street",
                            workspace,
                            &ResourceBase::explicit(source).unwrap(),
                            &Options::default(),
                            &CancelToken::default(),
                            &mut |_| {},
                            &mut |_| {},
                        );
                        match attempt {
                            Ok(adoption) => break adoption,
                            // Another adoption holds the root right now: try again.
                            Err(error) if error.code == "root_busy" => {
                                std::thread::sleep(std::time::Duration::from_millis(1));
                            }
                            Err(error) => panic!("{error:?}"),
                        }
                    };
                    barrier.wait();
                    if index == accepter {
                        adoption.provisional().accept().unwrap();
                    } else {
                        adoption.provisional().discard();
                    }
                });
            }
        });
        assert_intact(&workspace, &sandbox);
        assert_no_claims(&workspace);
    }
}

// ------------------------------------------------------------------------------- BL097 --

/// Overwrites the first bytes of `path` without truncating or replacing it: same inode, same
/// length.
fn rewrite_in_place(path: &Path, bytes: &[u8]) {
    let mut file = OpenOptions::new().write(true).open(path).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
}

#[cfg(unix)]
fn identity(path: &Path) -> (u64, u64, u64) {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::metadata(path).unwrap();
    (metadata.dev(), metadata.ino(), metadata.len())
}

#[test]
fn a_file_rewritten_in_place_with_the_same_length_is_not_removed() {
    let (sandbox, scene, workspace) = world("in-place");
    let mut adoption = adopt_into(&scene, &workspace, &sandbox.src);
    let path = figure(&workspace);
    let original = fs::read(&path).unwrap();
    #[cfg(unix)]
    let before = identity(&path);

    // Same number of bytes, same inode: what the stamp of length/device/inode cannot see.
    let edited: Vec<u8> = original.iter().map(|byte| byte ^ 0xFF).collect();
    rewrite_in_place(&path, &edited);
    assert_eq!(fs::metadata(&path).unwrap().len(), original.len() as u64);
    #[cfg(unix)]
    assert_eq!(identity(&path), before, "the rewrite kept inode and length");

    let report = adoption.provisional().discard();
    assert_eq!(fs::read(&path).unwrap(), edited, "the edit is preserved");
    assert!(
        report.kept.iter().any(|entry| entry.contains("figure.png")),
        "the report says why: {report:?}"
    );
    assert!(
        !backdrop(&workspace).exists(),
        "the untouched file is still ours and goes: {report:?}"
    );
}

#[test]
fn a_rewrite_that_puts_the_modification_time_back_is_still_seen() {
    let (sandbox, scene, workspace) = world("in-place-mtime");
    let mut adoption = adopt_into(&scene, &workspace, &sandbox.src);
    let path = figure(&workspace);
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let original = fs::read(&path).unwrap();

    let edited: Vec<u8> = original.iter().map(|byte| byte.wrapping_add(1)).collect();
    rewrite_in_place(&path, &edited);
    // A tool that preserves timestamps: the content is the only thing left to compare.
    let file = OpenOptions::new().write(true).open(&path).unwrap();
    file.set_modified(modified).unwrap();
    drop(file);

    let report = adoption.provisional().discard();
    assert_eq!(fs::read(&path).unwrap(), edited, "{report:?}");
}

#[test]
fn a_one_byte_edit_is_enough_to_keep_the_file() {
    let (sandbox, scene, workspace) = world("in-place-one-byte");
    let mut adoption = adopt_into(&scene, &workspace, &sandbox.src);
    let path = backdrop(&workspace);
    let mut edited = fs::read(&path).unwrap();
    let last = edited.len() - 1;
    edited[last] ^= 0x01;
    rewrite_in_place(&path, &edited);
    // `SeekFrom::Start(0)` rewrote everything; only the last byte differs from the original.

    let report = adoption.provisional().discard();
    assert_eq!(fs::read(&path).unwrap(), edited, "{report:?}");
}

#[test]
fn an_appended_file_is_kept() {
    let (sandbox, scene, workspace) = world("appended");
    let mut adoption = adopt_into(&scene, &workspace, &sandbox.src);
    let path = figure(&workspace);
    let mut file = OpenOptions::new().append(true).open(&path).unwrap();
    file.write_all(b"trailer").unwrap();
    drop(file);

    let report = adoption.provisional().discard();
    assert!(path.is_file(), "{report:?}");
}

#[test]
fn a_rewrite_with_the_same_bytes_is_indistinguishable_and_is_removed() {
    // The limit, written down as a test: nothing the user wrote exists to be lost.
    let (sandbox, scene, workspace) = world("in-place-same-bytes");
    let mut adoption = adopt_into(&scene, &workspace, &sandbox.src);
    let path = figure(&workspace);
    let same = fs::read(&path).unwrap();
    rewrite_in_place(&path, &same);

    let report = adoption.provisional().discard();
    assert!(!path.exists(), "{report:?}");
}

#[cfg(unix)]
#[test]
fn a_file_that_cannot_be_read_is_kept_instead_of_trusted() {
    use std::os::unix::fs::PermissionsExt;
    let (sandbox, scene, workspace) = world("unreadable");
    let mut adoption = adopt_into(&scene, &workspace, &sandbox.src);
    let path = figure(&workspace);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::File::open(&path).is_ok() {
        // Running as a user that ignores permissions (root): nothing to prove here.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        return;
    }

    let report = adoption.provisional().discard();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(path.is_file(), "fail closed: {report:?}");
    assert!(
        report.kept.iter().any(|entry| entry.contains("figure.png")),
        "{report:?}"
    );
}

// ------------------------------------------------------------------------------- BL095 --

mod unwinding {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use super::*;
    use tessel_editor_media::directed_resources::{RootLease, materialize_observed};
    use tessel_editor_media::scene_resources::tracked_under;

    #[test]
    fn a_panic_while_publishing_leaves_the_transport_root_as_it_was() {
        let sandbox = Sandbox::new("te-own-unwind-materialize");
        let manifest = manifest(standard_bundle(&sandbox));
        fs::write(sandbox.dst.join("mine.txt"), b"user").unwrap();
        let before = tree(&sandbox.dst);

        // The first file is already visible when the observer panics.
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            materialize_observed(
                &manifest,
                &sandbox.src,
                &sandbox.dst,
                &Options::default(),
                &CancelToken::default(),
                &mut |_| {},
                &mut |published| {
                    if published.done == 1 {
                        panic!("observer failed");
                    }
                },
            )
        }));
        assert!(outcome.is_err(), "the panic must propagate");
        assert_eq!(
            tree(&sandbox.dst),
            before,
            "published files and folders are taken back"
        );
        drop(RootLease::acquire(&sandbox.dst).expect("the lease is released while unwinding"));
        assert_eq!(fs::read(sandbox.dst.join("mine.txt")).unwrap(), b"user");
    }

    #[test]
    fn a_panic_inside_an_adoption_takes_back_what_it_copied_and_holds_nothing() {
        let (sandbox, scene, workspace) = world("unwind-adopt");
        let before = tree(&workspace);

        let outcome = catch_unwind(AssertUnwindSafe(|| {
            adopt(
                &scene,
                "street",
                &workspace,
                &ResourceBase::explicit(&sandbox.src).unwrap(),
                &Options::default(),
                &CancelToken::default(),
                &mut |_| {},
                &mut |published| {
                    if published.done == 1 {
                        panic!("observer failed");
                    }
                },
            )
        }));
        assert!(outcome.is_err());
        assert_eq!(tree(&workspace), before);
        assert_eq!(
            tracked_under(&workspace),
            0,
            "no claim outlives the unwinding"
        );
    }
}

// ------------------------------------------------------------------------------- cost --

/// Measurement, not a gate: how long discarding a result takes when it holds one large file, the
/// cost the content check adds (see the `cleanup` module docs). Run on purpose with
/// `-- --ignored --nocapture`.
#[test]
#[ignore = "measurement: writes and hashes a 256 MiB file"]
fn measure_discarding_a_result_with_a_large_file() {
    use std::time::Instant;
    const EXTRA: usize = 256 << 20;
    let sandbox = Sandbox::new("te-own-cost");
    let workspace = sandbox.root.join("ws");
    fs::create_dir_all(&workspace).unwrap();
    let mut document = fixture();
    // A valid PNG followed by padding: the transport only reads the header.
    let mut big = png(64, 36, 10);
    big.resize(big.len() + EXTRA, 0);
    let backdrop = sandbox.source_file("assets/backdrop.png", &big);
    let figure = sandbox.source_file("assets/figure.png", &png(32, 32, 20));
    restate(&mut document, "backdrop", "assets/backdrop.png", &backdrop);
    restate(&mut document, "figure", "assets/figure.png", &figure);
    let scene = scene(document);
    let mut adoption = adopt_into(&scene, &workspace, &sandbox.src);

    let started = Instant::now();
    let report = adoption.provisional().discard();
    let elapsed = started.elapsed();
    println!(
        "discarding {:.0} MiB took {elapsed:?} ({:.0} MiB/s): {report:?}",
        EXTRA as f64 / f64::from(1 << 20),
        EXTRA as f64 / f64::from(1 << 20) / elapsed.as_secs_f64()
    );
    assert_eq!(report.removed_files, 2, "{report:?}");
}

// ------------------------------------------------------- BL094/BL112 seam, for BL111 --
//
// What a persistent, cross-process owner of the claims (BL111) has to keep true for the job that
// discards a refused result: `settle` answers without reading or removing a file, a refusal hands
// the adoption back **still owning** its copies (and their claims), and only `discard` takes them.

fn settled_world(name: &str) -> (Sandbox, PathBuf, PendingAdoption, EditorState) {
    let (sandbox, scene, workspace) = world(name);
    let bundle = sandbox.root.join("bundle.json");
    fs::write(
        &bundle,
        serde_json::to_vec(&standard_bundle(&sandbox)).unwrap(),
    )
    .unwrap();
    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    state.project.workspace = Some(workspace.clone());
    headless::execute(
        &mut state,
        &json!({"op":"scene.attach","id":"street","path":bundle}),
    )
    .unwrap();
    let target = AdoptionTarget::of(&state, "street");
    let mut adoption = adopt_into(&scene, &workspace, &sandbox.src);
    let pending = PendingAdoption::new(target, &mut adoption);
    (sandbox, workspace, pending, state)
}

#[test]
fn settle_accepts_without_touching_a_file_and_the_claims_end_with_it() {
    let (sandbox, workspace, pending, state) = settled_world("settle-ok");
    assert!(
        tracked_under(&workspace) > 0,
        "pending: the result is the owner"
    );
    pending
        .settle(&state)
        .expect("same project, workspace and scene");
    assert_intact(&workspace, &sandbox);
    assert_no_claims(&workspace);
}

#[test]
fn a_refused_settle_returns_the_adoption_still_owning_its_copies_until_it_is_discarded() {
    let (sandbox, workspace, pending, mut state) = settled_world("settle-stale");
    state.project.id = "another-project".to_string();

    let stale = *pending.settle(&state).expect_err("another project is open");
    assert_eq!(stale.why, "another project is open now");
    assert_eq!(stale.pending.scene(), "street");
    // The refusal removed nothing and released nothing: that is the job's work, off the UI thread.
    assert_intact(&workspace, &sandbox);
    assert!(
        tracked_under(&workspace) > 0,
        "the files keep their owner while they wait for the discard"
    );

    let report = stale.pending.discard();
    assert!(report.is_clean(), "{report:?}");
    assert_eq!(report.removed_files, 2, "{report:?}");
    assert!(!workspace.join("scenes").exists());
    assert_no_claims(&workspace);
}

#[test]
fn apply_is_settle_plus_the_removal_with_the_same_refusal() {
    let (_sandbox, workspace, pending, mut state) = settled_world("apply-stale");
    state.project.id = "another-project".to_string();
    let error = pending.apply(&state).unwrap_err();
    assert_eq!(error.code, "stale_result", "{error:?}");
    assert!(error.message.contains("2 file(s)"), "{}", error.message);
    assert!(!workspace.join("scenes").exists(), "apply removes inline");
    assert_no_claims(&workspace);
}
