//! F038 (BB06 initial), resolution and preflight: the read-only side of
//! `tessel_editor_media::directed_resources`, with small synthetic files (no private images, no
//! FFmpeg, no window).
//!
//! Proven here: every problem is reported (missing, hash, size of a PNG, wrong format, folder
//! instead of file, size limits), shared files are read once, preserved resources are listed and
//! not required, and a symbolic link cannot take a read outside the source root. The write side
//! is `directed_resources_transport.rs`. Not proven: pixel decoding or alpha (a documented limit)
//! and the integration with the package flow (next slice).

mod directed_resources_common;

use std::fs;

use directed_resources_common::*;
use serde_json::json;
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::jobs::CancelToken;
use tessel_editor_core::scene_resources::{ResourceKey, ResourceStatus};
use tessel_editor_media::directed_resources::{
    Options, PreflightReport, ProblemKind, SymlinkPolicy, preflight,
};

fn run(
    manifest: &tessel_editor_core::scene_resources::ResourceManifest,
    sandbox: &Sandbox,
) -> PreflightReport {
    run_with(manifest, sandbox, &Options::default())
}

fn run_with(
    manifest: &tessel_editor_core::scene_resources::ResourceManifest,
    sandbox: &Sandbox,
    options: &Options,
) -> PreflightReport {
    preflight(
        manifest,
        &sandbox.src,
        options,
        &CancelToken::new(),
        &mut |_| {},
    )
    .unwrap()
}

fn kinds(report: &PreflightReport) -> Vec<(String, ProblemKind)> {
    report
        .problems
        .iter()
        .map(|problem| (problem.path.clone(), problem.kind))
        .collect()
}

#[test]
fn a_complete_source_passes_and_reports_sizes() {
    let sandbox = Sandbox::new("pass");
    let manifest = manifest(standard_bundle(&sandbox));
    let report = run(&manifest, &sandbox);
    assert!(report.ok(), "{:?}", report.problems);
    let paths: Vec<_> = report.checked.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(paths, ["assets/backdrop.png", "assets/figure.png"]);
    assert_eq!(report.checked[0].keys, [ResourceKey::asset("backdrop")]);
    let expected = fs::metadata(sandbox.src.join("assets/backdrop.png"))
        .unwrap()
        .len()
        + fs::metadata(sandbox.src.join("assets/figure.png"))
            .unwrap()
            .len();
    assert_eq!(report.total_bytes, expected);
    assert!(report.skipped.is_empty());
    assert_eq!(report.to_json()["ok"], true);
}

#[test]
fn every_problem_is_reported_not_only_the_first() {
    let sandbox = Sandbox::new("problems");
    let manifest = manifest(standard_bundle(&sandbox));
    // Same length, other bytes: only the hash can tell. And the other file is gone.
    fs::write(sandbox.src.join("assets/backdrop.png"), png(64, 36, 99)).unwrap();
    fs::remove_file(sandbox.src.join("assets/figure.png")).unwrap();
    let report = run(&manifest, &sandbox);
    assert!(!report.ok());
    assert_eq!(
        kinds(&report),
        [
            ("assets/backdrop.png".to_string(), ProblemKind::HashMismatch),
            ("assets/figure.png".to_string(), ProblemKind::Missing),
        ]
    );
    assert_eq!(report.problems[0].keys, [ResourceKey::asset("backdrop")]);
    assert!(report.problems[0].detail.contains("expected sha256"));
    assert!(report.checked.is_empty());
    assert_eq!(report.to_json()["problems"][1]["code"], "missing");
}

#[test]
fn a_png_of_another_size_than_declared_is_refused() {
    let sandbox = Sandbox::new("dimensions");
    let mut document = standard_bundle(&sandbox);
    let wrong = sandbox.source_file("assets/figure.png", &png(16, 16, 20));
    restate(&mut document, "figure", "assets/figure.png", &wrong);
    let report = run(&manifest(document), &sandbox);
    assert_eq!(
        kinds(&report),
        [(
            "assets/figure.png".to_string(),
            ProblemKind::DimensionMismatch
        )]
    );
    assert!(report.problems[0].detail.contains("16x16"));
}

#[test]
fn bytes_that_are_not_the_declared_format_are_refused_even_with_a_matching_hash() {
    let sandbox = Sandbox::new("format");
    let mut document = standard_bundle(&sandbox);
    let text = sandbox.source_file(
        "assets/figure.png",
        b"definitely not a png, but long enough",
    );
    restate(&mut document, "figure", "assets/figure.png", &text);

    // A splat node whose file is a PNG, and one whose file is a real SPZ.
    let fake = sandbox.source_file("splats/fake.spz", &png(8, 8, 1));
    let real = sandbox.source_file("splats/real.spz", &spz());
    add_asset(
        &mut document,
        "fake",
        "splats/fake.spz",
        &fake,
        Some("spz"),
        "srgb",
    );
    add_asset(
        &mut document,
        "real",
        "splats/real.spz",
        &real,
        Some("spz"),
        "srgb",
    );
    add_node(
        &mut document,
        "fake-node",
        "splat",
        reference("fake", "splats/fake.spz", &fake),
    );
    add_node(
        &mut document,
        "real-node",
        "splat",
        reference("real", "splats/real.spz", &real),
    );

    let report = run(&manifest(document), &sandbox);
    assert_eq!(
        kinds(&report),
        [
            ("assets/figure.png".to_string(), ProblemKind::InvalidContent),
            ("splats/fake.spz".to_string(), ProblemKind::InvalidContent),
        ]
    );
    let checked: Vec<_> = report.checked.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(checked, ["assets/backdrop.png", "splats/real.spz"]);
}

#[test]
fn the_audio_mix_is_checked_by_hash() {
    let sandbox = Sandbox::new("audio");
    let mut document = standard_bundle(&sandbox);
    let mix = sandbox.source_file("audio/mix.wav", b"RIFF-not-really-pcm");
    document["audio"] = json!({
        "mixRelativePath": "audio/mix.wav",
        "editStartSeconds": 0,
        "editEndSecondsExclusive": 4,
        "sourcePcmHashSha256": mix,
    });
    let manifest = manifest(document);
    assert!(run(&manifest, &sandbox).ok());
    fs::write(sandbox.src.join("audio/mix.wav"), b"changed").unwrap();
    let report = run(&manifest, &sandbox);
    assert_eq!(
        kinds(&report),
        [("audio/mix.wav".to_string(), ProblemKind::HashMismatch)]
    );
    assert_eq!(report.problems[0].keys, [ResourceKey::AudioMix]);
}

#[test]
fn assets_that_share_a_file_are_read_once_and_keep_their_identity() {
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
    let report = run(&manifest(document), &sandbox);
    assert!(report.ok(), "{:?}", report.problems);
    assert_eq!(report.checked.len(), 2);
    let shared = report
        .checked
        .iter()
        .find(|c| c.path.as_str() == "assets/figure.png")
        .unwrap();
    assert_eq!(
        shared.keys,
        [
            ResourceKey::asset("figure"),
            ResourceKey::asset("figure-twin")
        ]
    );
}

#[test]
fn preserved_resources_are_listed_and_their_files_are_not_required() {
    let sandbox = Sandbox::new("preserved");
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
    let manifest = manifest(document);
    assert_eq!(
        manifest.resource("figure-normal").unwrap().status,
        ResourceStatus::Uninterpreted
    );
    let report = run(&manifest, &sandbox);
    assert!(report.ok(), "the map file does not exist and is not needed");
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(report.skipped[0].key, ResourceKey::asset("figure-normal"));
    assert_eq!(report.skipped[0].declared_path, "maps/figure.normal.png");
}

#[test]
fn a_folder_where_a_file_belongs_is_not_a_regular_file() {
    let sandbox = Sandbox::new("folder");
    let manifest = manifest(standard_bundle(&sandbox));
    fs::remove_file(sandbox.src.join("assets/figure.png")).unwrap();
    fs::create_dir(sandbox.src.join("assets/figure.png")).unwrap();
    // And a file whose bytes are not the declared ones.
    fs::remove_file(sandbox.src.join("assets/backdrop.png")).unwrap();
    fs::write(sandbox.src.join("assets/backdrop.png"), b"x").unwrap();
    let report = run(&manifest, &sandbox);
    assert_eq!(
        kinds(&report),
        [
            ("assets/backdrop.png".to_string(), ProblemKind::HashMismatch),
            ("assets/figure.png".to_string(), ProblemKind::NotRegularFile),
        ]
    );
}

#[test]
fn size_limits_apply_per_file_and_to_the_total() {
    let sandbox = Sandbox::new("limits");
    let manifest = manifest(standard_bundle(&sandbox));
    let tiny = Options {
        max_resource_bytes: 10,
        ..Options::default()
    };
    let report = run_with(&manifest, &sandbox, &tiny);
    assert_eq!(report.problems.len(), 2);
    assert!(
        report
            .problems
            .iter()
            .all(|p| p.kind == ProblemKind::TooLarge)
    );

    let total = Options {
        max_total_bytes: 10,
        ..Options::default()
    };
    let report = run_with(&manifest, &sandbox, &total);
    assert_eq!(report.problems.len(), 1);
    assert_eq!(report.problems[0].kind, ProblemKind::TooLarge);
    assert_eq!(
        report.problems[0].path, "",
        "a problem of the whole set has no path"
    );
    assert_eq!(report.checked.len(), 2, "each file was fine on its own");
}

#[test]
fn paths_that_climb_or_are_absolute_never_reach_the_manifest() {
    for declared in [
        "../outside/secret.png",
        "assets/../../outside/secret.png",
        "/etc/hosts",
    ] {
        let sandbox = Sandbox::new("traversal");
        let mut document = standard_bundle(&sandbox);
        let hash = sandbox.source_file("assets/figure.png", &png(32, 32, 20));
        restate(&mut document, "figure", declared, &hash);
        assert!(
            DirectedScene::import_value(document).is_err(),
            "{declared} must be refused by the importer"
        );
    }
}

#[test]
fn a_path_the_importer_accepts_but_a_root_cannot_hold_blocks_before_any_read() {
    let sandbox = Sandbox::new("portable");
    let mut document = standard_bundle(&sandbox);
    let hash = sandbox.source_file("assets/fig:ure.png", &png(32, 32, 20));
    restate(&mut document, "figure", "assets/fig:ure.png", &hash);
    let report = run(&manifest(document), &sandbox);
    assert!(!report.ok());
    assert!(report.checked.is_empty() && report.problems.is_empty());
    assert_eq!(report.manifest_issues.len(), 1);
    assert_eq!(report.manifest_issues[0].code, "RESOURCE_PATH_INVALID");
}

#[test]
fn a_synthetic_hash_blocks_preflight_with_the_manifest_issues() {
    let sandbox = Sandbox::new("synthetic");
    let report = run(&manifest(fixture()), &sandbox);
    assert!(!report.ok());
    assert_eq!(report.manifest_issues.len(), 2);
    assert_eq!(
        report.to_json()["manifestIssues"][0]["code"],
        "RESOURCE_HASH_INVALID"
    );
}

#[test]
fn preflight_observes_cancellation_and_reports_progress() {
    let sandbox = Sandbox::new("cancel");
    let manifest = manifest(standard_bundle(&sandbox));
    let token = CancelToken::new();
    token.cancel();
    let error = preflight(
        &manifest,
        &sandbox.src,
        &Options::default(),
        &token,
        &mut |_| {},
    )
    .unwrap_err();
    assert!(error.is_cancelled());

    let mut steps = Vec::new();
    preflight(
        &manifest,
        &sandbox.src,
        &Options::default(),
        &CancelToken::new(),
        &mut |progress| steps.push((progress.done, progress.total)),
    )
    .unwrap();
    assert_eq!(steps, [(1, 2), (2, 2)]);
}

#[test]
fn an_unusable_source_root_is_an_error_not_a_report() {
    let sandbox = Sandbox::new("noroot");
    let manifest = manifest(standard_bundle(&sandbox));
    let missing = sandbox.root.join("nowhere");
    let error = preflight(
        &manifest,
        &missing,
        &Options::default(),
        &CancelToken::new(),
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(error.code, "root_unavailable");
    let file = sandbox.src.join("assets/figure.png");
    let error = preflight(
        &manifest,
        &file,
        &Options::default(),
        &CancelToken::new(),
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(error.code, "root_not_directory");
}

#[cfg(unix)]
mod symlinks {
    use std::os::unix::fs::symlink;

    use super::*;

    fn allow() -> Options {
        Options {
            symlinks: SymlinkPolicy::AllowContained,
            ..Options::default()
        }
    }

    #[test]
    fn a_link_inside_the_root_is_refused_by_default_and_followed_when_allowed() {
        let sandbox = Sandbox::new("link-inside");
        let manifest = manifest(standard_bundle(&sandbox));
        // figure.png becomes a link to a file that lives inside the root.
        fs::rename(
            sandbox.src.join("assets/figure.png"),
            sandbox.src.join("real-figure.png"),
        )
        .unwrap();
        symlink(
            sandbox.src.join("real-figure.png"),
            sandbox.src.join("assets/figure.png"),
        )
        .unwrap();
        assert_eq!(
            kinds(&run(&manifest, &sandbox)),
            [(
                "assets/figure.png".to_string(),
                ProblemKind::SymlinkRejected
            )]
        );
        assert!(run_with(&manifest, &sandbox, &allow()).ok());
    }

    #[test]
    fn a_folder_link_to_the_outside_never_lets_a_read_leave_the_root() {
        let sandbox = Sandbox::new("link-folder");
        let manifest = manifest(standard_bundle(&sandbox));
        // `assets` is a link to a folder outside the root that holds identical, valid files, so
        // only containment (not content) can refuse them.
        fs::rename(sandbox.src.join("assets"), sandbox.outside.join("assets")).unwrap();
        symlink(sandbox.outside.join("assets"), sandbox.src.join("assets")).unwrap();

        let refused = run(&manifest, &sandbox);
        assert_eq!(
            kinds(&refused),
            [
                (
                    "assets/backdrop.png".to_string(),
                    ProblemKind::SymlinkRejected
                ),
                (
                    "assets/figure.png".to_string(),
                    ProblemKind::SymlinkRejected
                ),
            ]
        );
        let contained = run_with(&manifest, &sandbox, &allow());
        assert_eq!(
            kinds(&contained),
            [
                ("assets/backdrop.png".to_string(), ProblemKind::EscapesRoot),
                ("assets/figure.png".to_string(), ProblemKind::EscapesRoot),
            ]
        );
    }

    #[test]
    fn a_file_link_to_a_secret_outside_is_an_escape() {
        let sandbox = Sandbox::new("link-secret");
        let manifest = manifest(standard_bundle(&sandbox));
        fs::write(sandbox.outside.join("secret.png"), png(32, 32, 20)).unwrap();
        fs::remove_file(sandbox.src.join("assets/figure.png")).unwrap();
        symlink(
            sandbox.outside.join("secret.png"),
            sandbox.src.join("assets/figure.png"),
        )
        .unwrap();
        assert_eq!(
            kinds(&run_with(&manifest, &sandbox, &allow())),
            [("assets/figure.png".to_string(), ProblemKind::EscapesRoot)]
        );
    }

    #[test]
    fn a_dangling_link_is_missing_not_a_crash() {
        let sandbox = Sandbox::new("link-dangling");
        let manifest = manifest(standard_bundle(&sandbox));
        fs::remove_file(sandbox.src.join("assets/figure.png")).unwrap();
        symlink(
            sandbox.root.join("gone.png"),
            sandbox.src.join("assets/figure.png"),
        )
        .unwrap();
        assert_eq!(
            kinds(&run_with(&manifest, &sandbox, &allow())),
            [("assets/figure.png".to_string(), ProblemKind::Missing)]
        );
    }

    #[test]
    fn the_source_root_itself_may_be_reached_through_a_link() {
        let sandbox = Sandbox::new("link-root");
        let manifest = manifest(standard_bundle(&sandbox));
        let alias = sandbox.root.join("alias");
        symlink(&sandbox.src, &alias).unwrap();
        let report = preflight(
            &manifest,
            &alias,
            &Options::default(),
            &CancelToken::new(),
            &mut |_| {},
        )
        .unwrap();
        assert!(report.ok(), "{:?}", report.problems);
    }
}
