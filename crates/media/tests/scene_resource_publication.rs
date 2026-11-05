//! BL079 / BL080: what a transport root looks like while and after a scene's files are
//! published into it.
//!
//! Proven here, through the public media API with synthetic files: cancelling *between two
//! published files* rolls back exactly what that run created and nothing else (a file the
//! caller already had in the root survives, the lease file is gone); a second operation on the
//! same root is refused with `root_busy` and writes nothing; the root can be used again right
//! after either; the lease is released by a failure. Not proven: a hostile writer swapping a
//! parent folder for a link between two path operations (needs directory-anchored I/O).

mod directed_resources_common;

use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

use directed_resources_common::*;
use tessel_editor_core::jobs::CancelToken;
use tessel_editor_media::directed_resources::{
    LEASE_FILE, Options, RootLease, materialize, materialize_leased, materialize_observed,
};

#[test]
fn cancelling_after_the_first_published_file_removes_only_what_the_run_created() {
    let sandbox = Sandbox::new("cancel-publish");
    let manifest = manifest(standard_bundle(&sandbox));
    // A file the destination already had: it is not ours to touch.
    fs::create_dir_all(sandbox.dst.join("assets")).unwrap();
    fs::write(sandbox.dst.join("assets/keep.txt"), b"mine").unwrap();
    let before = tree(&sandbox.dst);

    let token = CancelToken::new();
    let published = AtomicUsize::new(0);
    let error = materialize_observed(
        &manifest,
        &sandbox.src,
        &sandbox.dst,
        &Options::default(),
        &token,
        &mut |_| {},
        &mut |file| {
            published.fetch_add(1, Ordering::SeqCst);
            // The first file is already visible in the root when the caller cancels.
            assert_eq!((file.done, file.total), (1, 2));
            assert!(sandbox.dst.join(file.path.as_str()).exists());
            token.cancel();
        },
    )
    .unwrap_err();

    assert_eq!(error.code, "cancelled");
    assert_eq!(
        published.load(Ordering::SeqCst),
        1,
        "the next file never started"
    );
    assert_eq!(
        tree(&sandbox.dst),
        before,
        "own files rolled back, the others kept"
    );
    assert_eq!(
        fs::read(sandbox.dst.join("assets/keep.txt")).unwrap(),
        b"mine"
    );
}

#[test]
fn the_root_is_reusable_right_after_a_cancelled_publication() {
    let sandbox = Sandbox::new("cancel-reuse");
    let manifest = manifest(standard_bundle(&sandbox));
    let token = CancelToken::new();
    let error = materialize_observed(
        &manifest,
        &sandbox.src,
        &sandbox.dst,
        &Options::default(),
        &token,
        &mut |_| {},
        &mut |_| token.cancel(),
    )
    .unwrap_err();
    assert_eq!(error.code, "cancelled");
    assert!(tree(&sandbox.dst).is_empty(), "{:?}", tree(&sandbox.dst));

    let report = materialize(
        &manifest,
        &sandbox.src,
        &sandbox.dst,
        &Options::default(),
        &CancelToken::new(),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.copied(), 2);
    assert_eq!(
        tree(&sandbox.dst),
        ["assets", "assets/backdrop.png", "assets/figure.png"]
    );
}

#[test]
fn a_second_operation_on_a_leased_root_is_refused_and_writes_nothing() {
    let sandbox = Sandbox::new("busy");
    let manifest = manifest(standard_bundle(&sandbox));
    let lease = RootLease::acquire(&sandbox.dst).unwrap();
    let held = tree(&sandbox.dst);
    assert_eq!(
        held,
        [LEASE_FILE],
        "the lease is the only thing in the root"
    );

    let error = materialize(
        &manifest,
        &sandbox.src,
        &sandbox.dst,
        &Options::default(),
        &CancelToken::new(),
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(error.code, "root_busy");
    assert_eq!(
        error.details.as_ref().unwrap()["holder"],
        std::process::id(),
        "the error names the process that holds the root"
    );
    assert_eq!(tree(&sandbox.dst), held, "the refused run wrote nothing");

    // The holder itself may keep working under the lease it has.
    let report = materialize_leased(
        &manifest,
        &sandbox.src,
        &lease,
        &Options::default(),
        &CancelToken::new(),
        &mut |_| {},
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.copied(), 2);

    drop(lease);
    assert_eq!(
        tree(&sandbox.dst),
        ["assets", "assets/backdrop.png", "assets/figure.png"],
        "releasing the lease leaves no lock file behind"
    );
}

#[test]
fn a_failed_run_releases_the_lease() {
    let sandbox = Sandbox::new("lease-after-failure");
    let mut document = standard_bundle(&sandbox);
    // The declared hash is wrong: the run fails before anything is published.
    restate(
        &mut document,
        "backdrop",
        "assets/backdrop.png",
        &"0".repeat(64),
    );
    let broken = manifest(document);
    let error = materialize(
        &broken,
        &sandbox.src,
        &sandbox.dst,
        &Options::default(),
        &CancelToken::new(),
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(error.code, "resources_invalid");
    assert!(tree(&sandbox.dst).is_empty());
    RootLease::acquire(&sandbox.dst).expect("a failure must not leave the root leased");
}
