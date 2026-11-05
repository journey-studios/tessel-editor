//! P07: consolidate, verify and restore projects without access to the original files.

mod common;

use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use tessel_editor_core::assets::Source;
use tessel_editor_core::fixtures::{StandardSet, proof_requests, write_standard_set};
use tessel_editor_core::jobs::{JobKind, JobRegistry};
use tessel_editor_core::plan::{PlanRequest, PlanTime, PreviewIntent};
use tessel_editor_core::{EditorState, headless, persist};
use tessel_editor_media::package::{self, PackageFormat, PackageOptions};
use tessel_editor_media::zip::ZipWriter;

use common::temp;

fn ok(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn proof_state(set: &StandardSet, workspace: &Path) -> EditorState {
    let mut state = EditorState::demo();
    for request in proof_requests(set) {
        ok(&mut state, request);
    }
    let document = workspace.join("proof.tessel.json");
    persist::save(&mut state, &document).unwrap();
    state
}

fn plans(state: &EditorState) -> Vec<Value> {
    (0..state.timeline.length)
        .step_by(5)
        .map(|frame| {
            state
                .plan(&PlanRequest {
                    intent: PreviewIntent::Sequence,
                    at: PlanTime::Frame(frame),
                })
                .unwrap()
                .to_json()
        })
        .collect()
}

fn options(format: PackageFormat) -> PackageOptions {
    PackageOptions {
        format,
        sequences: None,
        allow_partial: false,
        replace: false,
    }
}

fn package_job(
    state: &EditorState,
    dest: &Path,
    options: PackageOptions,
) -> Result<Value, tessel_editor_core::jobs::JobError> {
    let registry = JobRegistry::new(1);
    let snapshot = state.clone();
    registry
        .run_inline(JobKind::Package, "package", |ctx| {
            package::create(&snapshot, dest, &options, ctx)
        })
        .1
}

fn restore_job(pack: &Path, dest: &Path) -> Result<Value, tessel_editor_core::jobs::JobError> {
    let registry = JobRegistry::new(1);
    registry
        .run_inline(JobKind::Restore, "restore", |ctx| {
            package::restore(pack, dest, ctx)
        })
        .1
}

#[test]
fn a_project_packages_restores_elsewhere_and_plays_the_same_without_its_originals() {
    for format in [PackageFormat::Archive, PackageFormat::Directory] {
        let dir = temp(&format!("package-{format:?}"));
        let src = dir.join("src");
        let set = write_standard_set(&src).unwrap();
        let state = proof_state(&set, &dir.join("ws"));
        let expected_plans = plans(&state);
        let pack = dir.join(if format == PackageFormat::Archive {
            "out/proof.tesselpack"
        } else {
            "out/proof-folder"
        });

        let preflight = package::preflight(&state, &options(format), &pack).unwrap();
        assert_eq!(preflight["ok"], true, "{preflight}");
        let report = package_job(&state, &pack, options(format)).unwrap();
        assert_eq!(report["complete"], true);
        assert!(report["files"].as_u64().unwrap() >= 8);
        // No staging leftovers next to the package.
        let leftovers: Vec<_> = fs::read_dir(pack.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(".tessel-staging")
            })
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        // Packaging never overwrites silently.
        assert_eq!(
            package_job(&state, &pack, options(format))
                .unwrap_err()
                .code,
            "destination_exists"
        );

        // The originals become unavailable; the package alone must be enough.
        fs::rename(&src, dir.join("src-gone")).unwrap();
        let restored = dir.join("elsewhere/restored");
        let result = restore_job(&pack, &restored).unwrap();
        assert_eq!(
            (result["verified"].as_bool(), result["complete"].as_bool()),
            (Some(true), Some(true))
        );
        let document = restored.join(package::DOCUMENT_NAME);
        let reopened = persist::load(&document).unwrap();
        assert_eq!(reopened.library.items.len(), state.library.items.len());
        for item in &reopened.library.items {
            let Some(asset) = &item.asset else { continue };
            for file in tessel_editor_core::document::asset_files(asset) {
                assert!(
                    file.starts_with(&restored),
                    "{} is outside the restored folder",
                    file.display()
                );
                assert!(file.is_file(), "{} was not restored", file.display());
            }
        }
        // Same project, same plan (asset paths aside, which differ by design).
        let strip =
            |plans: Vec<Value>| plans.into_iter().map(|p| p.to_string()).collect::<Vec<_>>();
        assert_eq!(strip(plans(&reopened)), strip(expected_plans), "{format:?}");
        // Restoring over an existing folder is refused.
        assert_eq!(
            restore_job(&pack, &restored).unwrap_err().code,
            "destination_exists"
        );
        fs::remove_dir_all(dir).ok();
    }
}

#[test]
fn tampering_missing_files_and_hostile_entries_are_rejected_before_publishing() {
    let dir = temp("package-bad");
    let set = write_standard_set(&dir.join("src")).unwrap();
    let state = proof_state(&set, &dir.join("ws"));
    let pack = dir.join("p");
    package_job(&state, &pack, options(PackageFormat::Directory)).unwrap();

    // A flipped byte in a media file.
    let victim = fs::read_dir(pack.join("media"))
        .unwrap()
        .filter_map(Result::ok)
        .find(|e| e.file_name().to_string_lossy().len() == 64)
        .unwrap()
        .path();
    let file = fs::read_dir(&victim)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut bytes = fs::read(&file).unwrap();
    bytes[0] ^= 0xff;
    fs::write(&file, &bytes).unwrap();
    let error = restore_job(&pack, &dir.join("r1")).unwrap_err();
    assert_eq!(error.code, "integrity_error");
    assert!(!dir.join("r1").exists(), "nothing is published on failure");
    bytes[0] ^= 0xff;
    fs::write(&file, &bytes).unwrap();
    restore_job(&pack, &dir.join("r-ok")).unwrap();

    // A file that the manifest does not list.
    fs::write(pack.join("media").join("sneaky.bin"), b"x").unwrap();
    assert_eq!(
        restore_job(&pack, &dir.join("r2")).unwrap_err().code,
        "integrity_error"
    );
    fs::remove_file(pack.join("media").join("sneaky.bin")).unwrap();

    // A manifest from the future.
    let manifest_path = pack.join(package::MANIFEST_NAME);
    let mut manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["version"] = json!(9);
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert_eq!(
        restore_job(&pack, &dir.join("r3")).unwrap_err().code,
        "unsupported_schema"
    );

    // Zip-slip, absolute names, and case collisions inside an archive never reach the disk.
    for (name, _) in [("../escape.txt", 0), ("/abs.txt", 0), ("a/../../b", 0)] {
        let mut zip = ZipWriter::new(Vec::new());
        assert_eq!(
            zip.add_bytes(name, b"x").unwrap_err().code,
            "invalid_package",
            "{name}"
        );
    }
    // Craft an archive whose manifest names an escaping path: restore must refuse it.
    let evil = dir.join("evil.tesselpack");
    let mut zip = ZipWriter::new(fs::File::create(&evil).unwrap());
    let manifest = json!({ "format": "tessel-package", "version": 1, "files": [
        { "path": "../escape.txt", "size": 1, "sha256": "00" }] });
    zip.add_bytes("manifest.json", &serde_json::to_vec(&manifest).unwrap())
        .unwrap();
    zip.finish().unwrap();
    assert_eq!(
        restore_job(&evil, &dir.join("r4")).unwrap_err().code,
        "invalid_package"
    );
    assert!(!dir.join("escape.txt").exists());
    // Not a package at all.
    let junk = dir.join("junk.tesselpack");
    fs::write(&junk, b"hello").unwrap();
    assert_eq!(
        restore_job(&junk, &dir.join("r5")).unwrap_err().code,
        "invalid_package"
    );
    fs::remove_dir_all(dir).ok();
}

#[test]
fn missing_media_blocks_a_complete_package_and_is_explicit_when_partial() {
    let dir = temp("package-partial");
    let set = write_standard_set(&dir.join("src")).unwrap();
    let mut state = proof_state(&set, &dir.join("ws"));
    state.library.items.iter_mut().for_each(|item| {
        if let Some(asset) = &mut item.asset
            && let Source::File { path } = &mut asset.source
            && path.ends_with("tone-440.wav")
        {
            asset.identity.size = Some(1);
        }
    });
    fs::remove_file(&set.splat).unwrap();
    let preflight =
        package::preflight(&state, &options(PackageFormat::Directory), &dir.join("p")).unwrap();
    assert_eq!(preflight["ok"], false);
    assert_eq!(preflight["blockers"][0]["code"], "missing_media");
    assert_eq!(preflight["missing"].as_array().unwrap().len(), 2);
    let error = package_job(&state, &dir.join("p"), options(PackageFormat::Directory)).unwrap_err();
    assert_eq!(error.code, "preflight_failed");
    assert!(!dir.join("p").exists(), "no partial output is left behind");

    let mut partial = options(PackageFormat::Directory);
    partial.allow_partial = true;
    let report = package_job(&state, &dir.join("p"), partial).unwrap();
    assert_eq!(report["complete"], false);
    let manifest: Value =
        serde_json::from_slice(&fs::read(dir.join("p").join(package::MANIFEST_NAME)).unwrap())
            .unwrap();
    assert_eq!(manifest["complete"], false);
    assert_eq!(manifest["missing"].as_array().unwrap().len(), 2);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn a_package_of_selected_sequences_carries_the_dependency_closure_only() {
    let dir = temp("package-closure");
    let set = write_standard_set(&dir.join("src")).unwrap();
    let mut state = proof_state(&set, &dir.join("ws"));
    ok(
        &mut state,
        json!({"op":"sequence.add","id":"alt","name":"Alt","activate":true}),
    );
    ok(
        &mut state,
        json!({"op":"track.add","id":"v","name":"V","kind":"video"}),
    );
    ok(
        &mut state,
        json!({"op":"clip.add","id":"alt-img","track":"v","asset":"img","start":0,"duration":4}),
    );
    let mut only_alt = options(PackageFormat::Directory);
    only_alt.sequences = Some(vec!["alt".into()]);
    let pack = dir.join("alt-pack");
    let report = package_job(&state, &pack, only_alt).unwrap();
    assert_eq!(
        report["files"], 2,
        "image + document, nothing else: {report}"
    );
    let restored = dir.join("alt-restored");
    restore_job(&pack, &restored).unwrap();
    let reopened = persist::load(&restored.join(package::DOCUMENT_NAME)).unwrap();
    assert_eq!(
        reopened
            .all_sequences()
            .map(|t| t.id.as_str().to_string())
            .collect::<Vec<_>>(),
        ["alt"]
    );
    assert!(reopened.library.item("img").is_some() && reopened.library.item("cloud").is_none());
    // The source project was not touched by packaging a subset.
    assert_eq!(state.all_sequences().count(), 2);
    fs::remove_dir_all(dir).ok();
}
