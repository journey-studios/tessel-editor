//! F038 (BB06 initial), materialization: the write side of
//! `tessel_editor_media::directed_resources`.
//!
//! Proven here: files land at their declared relative paths and nowhere else, a failed or
//! cancelled run leaves the transport root exactly as it was (staging removed, nothing
//! half-written), nothing is ever overwritten or written through a symbolic link, a second run
//! reuses what is already there, and shared files are carried once. Not proven: a writer racing
//! the transport root during publish (a documented residual risk), full disks, Windows.

mod directed_resources_common;

use std::fs;

use directed_resources_common::*;
use serde_json::json;
use tessel_editor_core::jobs::{CancelToken, JobError};
use tessel_editor_core::scene_resources::{ResourceKey, ResourceManifest};
use tessel_editor_media::directed_resources::{MaterializeReport, Options, Outcome, materialize};

fn run(manifest: &ResourceManifest, sandbox: &Sandbox) -> Result<MaterializeReport, JobError> {
    materialize(
        manifest,
        &sandbox.src,
        &sandbox.dst,
        &Options::default(),
        &CancelToken::new(),
        &mut |_| {},
    )
}

#[test]
fn files_land_at_their_declared_paths_and_the_source_is_untouched() {
    let sandbox = Sandbox::new("copy");
    let manifest = manifest(standard_bundle(&sandbox));
    let before = tree(&sandbox.src);
    let report = run(&manifest, &sandbox).unwrap();

    assert_eq!(report.copied(), 2);
    assert_eq!(report.reused(), 0);
    assert!(report.complete);
    assert_eq!(report.items[0].path.as_str(), "assets/backdrop.png");
    assert_eq!(report.items[0].keys, [ResourceKey::asset("backdrop")]);
    assert_eq!(
        report.bytes_copied,
        report.items.iter().map(|i| i.bytes).sum::<u64>()
    );
    // Exactly the two files and their folder: no staging folder left behind.
    assert_eq!(
        tree(&sandbox.dst),
        ["assets", "assets/backdrop.png", "assets/figure.png"]
    );
    assert_eq!(
        fs::read(sandbox.dst.join("assets/figure.png")).unwrap(),
        png(32, 32, 20)
    );
    assert_eq!(tree(&sandbox.src), before);
    assert_eq!(report.to_json()["items"][1]["outcome"], "copied");
}

#[test]
fn a_second_run_reuses_what_is_there_and_needs_no_source() {
    let sandbox = Sandbox::new("reuse");
    let manifest = manifest(standard_bundle(&sandbox));
    run(&manifest, &sandbox).unwrap();
    let before = tree(&sandbox.dst);

    let again = run(&manifest, &sandbox).unwrap();
    assert_eq!((again.copied(), again.reused()), (0, 2));
    assert_eq!(again.bytes_copied, 0);
    assert!(
        again
            .items
            .iter()
            .all(|item| item.outcome == Outcome::Reused)
    );
    assert_eq!(tree(&sandbox.dst), before);

    // The source can be gone: the transport root already holds verified copies.
    fs::remove_dir_all(&sandbox.src).unwrap();
    fs::create_dir(&sandbox.src).unwrap();
    assert_eq!(run(&manifest, &sandbox).unwrap().reused(), 2);
}

#[test]
fn a_partly_filled_root_copies_only_what_is_missing() {
    let sandbox = Sandbox::new("partial");
    let manifest = manifest(standard_bundle(&sandbox));
    write(&sandbox.dst, "assets/backdrop.png", &png(64, 36, 10));
    let mut seen = Vec::new();
    let report = materialize(
        &manifest,
        &sandbox.src,
        &sandbox.dst,
        &Options::default(),
        &CancelToken::new(),
        &mut |progress| seen.push((progress.done, progress.total)),
    )
    .unwrap();
    assert_eq!((report.copied(), report.reused()), (1, 1));
    assert_eq!(report.items[0].outcome, Outcome::Reused);
    assert_eq!(report.items[1].outcome, Outcome::Copied);
    assert_eq!(seen, [(1, 2), (2, 2)]);
}

#[test]
fn the_source_root_may_also_be_the_transport_root() {
    let sandbox = Sandbox::new("same-root");
    let manifest = manifest(standard_bundle(&sandbox));
    let before = tree(&sandbox.src);
    let report = materialize(
        &manifest,
        &sandbox.src,
        &sandbox.src,
        &Options::default(),
        &CancelToken::new(),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.reused(), 2);
    assert_eq!(tree(&sandbox.src), before);
}

#[test]
fn assets_that_share_a_file_are_carried_once_with_both_identities() {
    let sandbox = Sandbox::new("shared");
    let mut document = standard_bundle(&sandbox);
    let hash = sha256(&png(32, 32, 20));
    add_asset(
        &mut document,
        "figure-twin",
        "assets/figure.png",
        &hash,
        None,
        "srgb",
    );
    declare_size(&mut document, "figure-twin", 32, 32);
    add_node(
        &mut document,
        "twin",
        "imagePlane",
        reference("figure-twin", "assets/figure.png", &hash),
    );
    let report = run(&manifest(document), &sandbox).unwrap();
    assert_eq!(report.items.len(), 2);
    assert_eq!(
        report.items[1].keys,
        [
            ResourceKey::asset("figure"),
            ResourceKey::asset("figure-twin")
        ]
    );
}

#[test]
fn splats_and_the_audio_mix_are_carried_too() {
    let sandbox = Sandbox::new("kinds");
    let mut document = standard_bundle(&sandbox);
    let splat = sandbox.source_file("splats/room.spz", &spz());
    add_asset(
        &mut document,
        "room",
        "splats/room.spz",
        &splat,
        Some("spz"),
        "srgb",
    );
    add_node(
        &mut document,
        "room-node",
        "splat",
        reference("room", "splats/room.spz", &splat),
    );
    let mix = sandbox.source_file("audio/mix.wav", b"RIFF-not-really-pcm");
    document["audio"] = json!({
        "mixRelativePath": "audio/mix.wav",
        "editStartSeconds": 0,
        "editEndSecondsExclusive": 4,
        "sourcePcmHashSha256": mix,
    });
    let report = run(&manifest(document), &sandbox).unwrap();
    assert_eq!(report.copied(), 4);
    assert_eq!(
        tree(&sandbox.dst),
        [
            "assets",
            "assets/backdrop.png",
            "assets/figure.png",
            "audio",
            "audio/mix.wav",
            "splats",
            "splats/room.spz",
        ]
    );
    let audio = report
        .items
        .iter()
        .find(|i| i.path.as_str() == "audio/mix.wav")
        .unwrap();
    assert_eq!(audio.keys, [ResourceKey::AudioMix]);
}

#[test]
fn preserved_resources_are_reported_as_skipped_and_the_transport_is_partial() {
    let sandbox = Sandbox::new("skipped");
    let mut document = standard_bundle(&sandbox);
    add_asset(
        &mut document,
        "figure-normal",
        "maps/figure.normal.png",
        &"c".repeat(64),
        None,
        "linear-srgb",
    );
    node(&mut document, "figure")["normalMap"] = json!({
        "assetRef": reference("figure-normal", "maps/figure.normal.png", &"c".repeat(64)),
        "source": "luminance",
    });
    let report = run(&manifest(document), &sandbox).unwrap();
    assert!(!report.complete);
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(report.skipped[0].key, ResourceKey::asset("figure-normal"));
    assert!(
        !tree(&sandbox.dst)
            .iter()
            .any(|path| path.starts_with("maps"))
    );
}

#[test]
fn a_blocked_manifest_writes_nothing() {
    let sandbox = Sandbox::new("blocked");
    let error = run(&manifest(fixture()), &sandbox).unwrap_err();
    assert_eq!(error.code, "manifest_blocked");
    assert_eq!(
        error.details.as_ref().unwrap()["issues"][0]["code"],
        "RESOURCE_HASH_INVALID"
    );
    assert!(tree(&sandbox.dst).is_empty());
}

#[test]
fn source_problems_are_all_reported_and_nothing_is_written() {
    let sandbox = Sandbox::new("source-problems");
    let manifest = manifest(standard_bundle(&sandbox));
    fs::remove_file(sandbox.src.join("assets/figure.png")).unwrap();
    fs::write(sandbox.src.join("assets/backdrop.png"), png(64, 36, 99)).unwrap();
    // A missing file is found while deciding; the altered one only shows when its bytes are
    // read (next test), so only the missing one is reported here, before anything is staged.
    let error = run(&manifest, &sandbox).unwrap_err();
    assert_eq!(error.code, "resources_invalid");
    assert_eq!(problem_codes(&error), ["missing"]);
    assert!(tree(&sandbox.dst).is_empty());
}

#[test]
fn a_file_that_changes_content_is_caught_while_copying_and_leaves_no_trace() {
    let sandbox = Sandbox::new("altered");
    let manifest = manifest(standard_bundle(&sandbox));
    fs::write(sandbox.src.join("assets/figure.png"), png(32, 32, 77)).unwrap();
    let error = run(&manifest, &sandbox).unwrap_err();
    assert_eq!(error.code, "resources_invalid");
    assert_eq!(problem_codes(&error), ["hash_mismatch"]);
    // The first file had already been staged: the staging folder and its bytes are gone.
    assert!(tree(&sandbox.dst).is_empty());
}

#[test]
fn a_different_file_already_at_the_destination_is_a_conflict_and_is_never_overwritten() {
    let sandbox = Sandbox::new("conflict");
    let manifest = manifest(standard_bundle(&sandbox));
    write(&sandbox.dst, "assets/figure.png", b"the user's own file");
    let error = run(&manifest, &sandbox).unwrap_err();
    assert_eq!(problem_codes(&error), ["destination_conflict"]);
    assert_eq!(
        fs::read(sandbox.dst.join("assets/figure.png")).unwrap(),
        b"the user's own file"
    );
    // The other, free path was not published either.
    assert_eq!(tree(&sandbox.dst), ["assets", "assets/figure.png"]);
}

#[test]
fn a_folder_or_a_file_in_the_way_is_a_conflict() {
    let sandbox = Sandbox::new("in-the-way");
    let manifest = manifest(standard_bundle(&sandbox));
    fs::create_dir_all(sandbox.dst.join("assets/figure.png")).unwrap();
    assert_eq!(
        problem_codes(&run(&manifest, &sandbox).unwrap_err()),
        ["destination_conflict"]
    );
    fs::remove_dir_all(sandbox.dst.join("assets")).unwrap();
    write(&sandbox.dst, "assets", b"a file where the folder goes");
    assert_eq!(
        problem_codes(&run(&manifest, &sandbox).unwrap_err()),
        ["destination_conflict", "destination_conflict"]
    );
    assert_eq!(tree(&sandbox.dst), ["assets"]);
}

#[test]
fn an_unusable_transport_root_is_an_error() {
    let sandbox = Sandbox::new("noroot");
    let manifest = manifest(standard_bundle(&sandbox));
    let missing = sandbox.root.join("nowhere");
    let token = CancelToken::new();
    let error = materialize(
        &manifest,
        &sandbox.src,
        &missing,
        &Options::default(),
        &token,
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(error.code, "root_unavailable");
    assert!(
        !missing.exists(),
        "the transport root is never created for the caller"
    );
    let file = sandbox.src.join("assets/figure.png");
    let error = materialize(
        &manifest,
        &sandbox.src,
        &file,
        &Options::default(),
        &token,
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(error.code, "root_not_directory");
}

#[test]
fn limits_are_checked_before_anything_is_written() {
    let sandbox = Sandbox::new("limits");
    let manifest = manifest(standard_bundle(&sandbox));
    let options = Options {
        max_total_bytes: 10,
        ..Options::default()
    };
    let error = materialize(
        &manifest,
        &sandbox.src,
        &sandbox.dst,
        &options,
        &CancelToken::new(),
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(problem_codes(&error), ["too_large"]);
    assert!(tree(&sandbox.dst).is_empty());

    let options = Options {
        max_resource_bytes: 10,
        ..Options::default()
    };
    let error = materialize(
        &manifest,
        &sandbox.src,
        &sandbox.dst,
        &options,
        &CancelToken::new(),
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(problem_codes(&error), ["too_large", "too_large"]);
    assert!(tree(&sandbox.dst).is_empty());
}

#[test]
fn cancelling_before_the_start_changes_nothing() {
    let sandbox = Sandbox::new("cancel-start");
    let manifest = manifest(standard_bundle(&sandbox));
    let token = CancelToken::new();
    token.cancel();
    let error = materialize(
        &manifest,
        &sandbox.src,
        &sandbox.dst,
        &Options::default(),
        &token,
        &mut |_| {},
    )
    .unwrap_err();
    assert!(error.is_cancelled());
    assert!(tree(&sandbox.dst).is_empty());
}

#[test]
fn cancelling_halfway_removes_the_staged_files() {
    let sandbox = Sandbox::new("cancel-mid");
    let manifest = manifest(standard_bundle(&sandbox));
    let token = CancelToken::new();
    let mut stages = 0;
    let error = materialize(
        &manifest,
        &sandbox.src,
        &sandbox.dst,
        &Options::default(),
        &token,
        &mut |_| {
            stages += 1;
            token.cancel(); // after the first file is staged
        },
    )
    .unwrap_err();
    assert!(error.is_cancelled());
    assert_eq!(stages, 1, "the second file was never started");
    assert!(tree(&sandbox.dst).is_empty(), "{:?}", tree(&sandbox.dst));
}

// Unix only: creating a symlink on Windows needs a privilege a plain test run lacks (BL167.1
// records the coverage limit). `SymlinkPolicy` is imported here because only this module uses it.
#[cfg(unix)]
mod containment {
    use std::os::unix::fs::symlink;

    use tessel_editor_media::directed_resources::SymlinkPolicy;

    use super::*;

    fn allow() -> Options {
        Options {
            symlinks: SymlinkPolicy::AllowContained,
            ..Options::default()
        }
    }

    #[test]
    fn a_source_folder_link_to_the_outside_is_never_copied_from() {
        let sandbox = Sandbox::new("src-link");
        let manifest = manifest(standard_bundle(&sandbox));
        fs::rename(sandbox.src.join("assets"), sandbox.outside.join("assets")).unwrap();
        symlink(sandbox.outside.join("assets"), sandbox.src.join("assets")).unwrap();
        for options in [Options::default(), allow()] {
            let error = materialize(
                &manifest,
                &sandbox.src,
                &sandbox.dst,
                &options,
                &CancelToken::new(),
                &mut |_| {},
            )
            .unwrap_err();
            assert_eq!(error.code, "resources_invalid");
            assert!(tree(&sandbox.dst).is_empty());
        }
    }

    #[test]
    fn a_source_file_link_inside_the_root_is_copied_as_a_plain_file_only_when_allowed() {
        let sandbox = Sandbox::new("src-link-inside");
        let manifest = manifest(standard_bundle(&sandbox));
        fs::rename(
            sandbox.src.join("assets/figure.png"),
            sandbox.src.join("real.png"),
        )
        .unwrap();
        symlink(
            sandbox.src.join("real.png"),
            sandbox.src.join("assets/figure.png"),
        )
        .unwrap();
        let error = run(&manifest, &sandbox).unwrap_err();
        assert_eq!(problem_codes(&error), ["symlink_rejected"]);
        assert!(tree(&sandbox.dst).is_empty());

        materialize(
            &manifest,
            &sandbox.src,
            &sandbox.dst,
            &allow(),
            &CancelToken::new(),
            &mut |_| {},
        )
        .unwrap();
        let copy = sandbox.dst.join("assets/figure.png");
        assert!(!fs::symlink_metadata(copy).unwrap().file_type().is_symlink());
    }

    #[test]
    fn a_link_in_the_transport_root_is_never_written_through() {
        let sandbox = Sandbox::new("dst-link");
        let manifest = manifest(standard_bundle(&sandbox));
        symlink(&sandbox.outside, sandbox.dst.join("assets")).unwrap();
        let error = run(&manifest, &sandbox).unwrap_err();
        assert_eq!(
            problem_codes(&error),
            ["destination_conflict", "destination_conflict"]
        );
        assert!(
            tree(&sandbox.outside).is_empty(),
            "nothing reached the outside folder"
        );

        // A link standing at the file itself, pointing at a victim file.
        fs::remove_file(sandbox.dst.join("assets")).unwrap();
        fs::create_dir(sandbox.dst.join("assets")).unwrap();
        let victim = sandbox.outside.join("victim.png");
        fs::write(&victim, b"victim").unwrap();
        symlink(&victim, sandbox.dst.join("assets/figure.png")).unwrap();
        let error = run(&manifest, &sandbox).unwrap_err();
        assert_eq!(problem_codes(&error), ["destination_conflict"]);
        assert_eq!(fs::read(&victim).unwrap(), b"victim");
    }

    #[test]
    fn a_failed_publish_rolls_back_every_file_and_folder_it_created() {
        let mut sandbox = Sandbox::new("rollback");
        let mut document = standard_bundle(&sandbox);
        // Plan order is by path: `a/…` is published first, then `z/…` fails.
        let backdrop = sha256(&png(64, 36, 10));
        let figure = sha256(&png(32, 32, 20));
        sandbox.source_file("a/backdrop.png", &png(64, 36, 10));
        sandbox.source_file("z/figure.png", &png(32, 32, 20));
        restate(&mut document, "backdrop", "a/backdrop.png", &backdrop);
        restate(&mut document, "figure", "z/figure.png", &figure);
        let manifest = manifest(document);

        let locked = sandbox.dst.join("z");
        fs::create_dir(&locked).unwrap();
        sandbox.lock(&locked);
        if fs::write(locked.join("probe"), b"").is_ok() {
            return; // running as a user the permissions do not bind (root): cannot provoke it
        }

        let error = run(&manifest, &sandbox).unwrap_err();
        assert_eq!(error.code, "permission_denied", "{error:?}");
        assert_eq!(
            tree(&sandbox.dst),
            ["z"],
            "`a/` and its file, created by this run, are gone; the user's `z/` stays"
        );
    }
}
