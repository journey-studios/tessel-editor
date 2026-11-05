//! F036 compatibility guards: a project with directed scenes goes through package and project
//! import without losing a scene and without claiming more than it carries.
//!
//! Scenes travel inside the project document. The files a bundle names by relative path are not
//! resolved, hashed or copied yet (BB05/BB06), so a package of such a project is incomplete and
//! says so (`unsupported` unless a partial package is allowed). Importing sequences never carries
//! scenes (they belong to the project) and the plan warns about it. No FFmpeg is needed.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tessel_editor_core::jobs::{JobError, JobKind, JobRegistry};
use tessel_editor_core::{EditorState, headless, persist};
use tessel_editor_media::interchange::{load_source, prepare};
use tessel_editor_media::package::{self, PackageFormat, PackageOptions};

use common::temp;

fn bundle_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/migration/fixtures/bundles")
        .join(format!("{name}.bundle.json"))
}

fn ok(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

/// An empty project with one scene that names two image files (`assets[].relativePath`).
fn project_with_scene(document: &Path) -> EditorState {
    let mut state = EditorState::demo();
    ok(&mut state, json!({"op":"project.create","name":"Scenes"}));
    ok(
        &mut state,
        json!({"op":"scene.attach","id":"street","path":bundle_path("valid-minimal-continuous")}),
    );
    persist::save(&mut state, document).unwrap();
    state
}

fn options(format: PackageFormat, allow_partial: bool) -> PackageOptions {
    PackageOptions {
        format,
        sequences: None,
        allow_partial,
        replace: false,
    }
}

fn package_job(
    state: &EditorState,
    destination: &Path,
    options: PackageOptions,
) -> Result<Value, JobError> {
    let snapshot = state.clone();
    JobRegistry::new(1)
        .run_inline(JobKind::Package, "package", |ctx| {
            package::create(&snapshot, destination, &options, ctx)
        })
        .1
}

fn restore_job(pack: &Path, destination: &Path) -> Result<Value, JobError> {
    JobRegistry::new(1)
        .run_inline(JobKind::Restore, "restore", |ctx| {
            package::restore(pack, destination, ctx)
        })
        .1
}

#[test]
fn a_package_never_claims_to_be_portable_while_a_scene_names_files_it_does_not_carry() {
    for format in [PackageFormat::Archive, PackageFormat::Directory] {
        let dir = temp(&format!("scene-package-block-{format:?}"));
        let state = project_with_scene(&dir.join("ws/p.tessel.json"));
        let pack = dir.join("out/scenes.tesselpack");

        let plan = package::preflight(&state, &options(format, false), &pack).unwrap();
        assert_eq!(plan["ok"], false);
        assert_eq!(plan["complete"], false);
        assert_eq!(plan["blockers"][0]["code"], "unsupported");
        assert_eq!(plan["missing"][0]["asset"], "scene:street");
        assert_eq!(plan["missing"][0]["state"], "not_packaged");
        assert_eq!(
            plan["missing"][0]["paths"],
            json!(["assets/backdrop.png", "assets/figure.png"])
        );

        let error = package_job(&state, &pack, options(format, false)).unwrap_err();
        assert_eq!(error.code, "unsupported");
        assert!(!pack.exists(), "nothing was written");
        fs::remove_dir_all(dir).ok();
    }
}

#[test]
fn a_partial_package_keeps_the_scenes_in_its_document_and_reports_the_gap() {
    for format in [PackageFormat::Archive, PackageFormat::Directory] {
        let dir = temp(&format!("scene-package-partial-{format:?}"));
        let state = project_with_scene(&dir.join("ws/p.tessel.json"));
        let pack = dir.join("out/scenes.tesselpack");

        let plan = package::preflight(&state, &options(format, true), &pack).unwrap();
        assert_eq!(plan["ok"], true, "{plan}");
        assert_eq!(
            plan["complete"], false,
            "partial is never reported complete"
        );

        let made = package_job(&state, &pack, options(format, true)).unwrap();
        assert_eq!(made["complete"], false);
        assert_eq!(made["missing"][0]["asset"], "scene:street");

        let restored = dir.join("restored");
        let report = restore_job(&pack, &restored).unwrap();
        assert_eq!(report["complete"], false);
        let reopened = persist::load(&restored.join(package::DOCUMENT_NAME)).unwrap();
        assert_eq!(reopened.scenes.ids(), ["street"], "the scene was not lost");
        assert_eq!(
            reopened.scenes.get("street").unwrap().scene.document(),
            state.scenes.get("street").unwrap().scene.document(),
            "the bundle came back as it went in"
        );
        fs::remove_dir_all(dir).ok();
    }
}

#[test]
fn a_project_without_scenes_packages_complete() {
    let dir = temp("scene-package-plain");
    let mut state = project_with_scene(&dir.join("ws/p.tessel.json"));
    // Only data: no images, no audio mix. Nothing is left behind, so nothing is missing.
    let mut bundle: Value =
        serde_json::from_str(&fs::read_to_string(bundle_path("valid-minimal-continuous")).unwrap())
            .unwrap();
    bundle["assets"] = json!([]);
    for node in bundle["sceneDefinitions"][0]["nodes"]
        .as_array_mut()
        .unwrap()
    {
        if node["type"] == "imagePlane" {
            node["type"] = json!("group");
            let fields = node.as_object_mut().unwrap();
            for key in ["assetRef", "widthSceneUnits", "heightSceneUnits"] {
                fields.remove(key);
            }
        }
    }
    ok(&mut state, json!({"op":"scene.detach","scene":"street"}));
    assert!(
        headless::execute(
            &mut state,
            &json!({"op":"scene.attach","id":"data-only","bundle":bundle})
        )
        .is_err()
    );
    assert!(state.scenes.is_empty());
    let pack = dir.join("out/data.tesselpack");
    let made = package_job(&state, &pack, options(PackageFormat::Archive, false)).unwrap();
    assert_eq!(made["complete"], true, "{made}");
    fs::remove_dir_all(dir).ok();
}

#[test]
fn importing_sequences_never_carries_scenes_and_the_plan_says_so() {
    let dir = temp("scene-import");
    let source_document = dir.join("source/source.tessel.json");
    let source = project_with_scene(&source_document);
    let shared_plan =
        tessel_editor_core::interchange::plan_import(&EditorState::demo(), &source, &[]).unwrap();
    assert!(
        shared_plan
            .warnings
            .iter()
            .any(|warning| warning.contains("directed scene"))
    );
    let source_bytes = fs::read(&source_document).unwrap();
    let workspace = dir.join("dest");
    let mut destination = EditorState::demo();
    ok(
        &mut destination,
        json!({"op":"project.create","name":"Dest","dir":workspace}),
    );

    let mut prepared = None;
    let (_, outcome) = JobRegistry::new(1).run_inline(JobKind::ProjectImport, "import", |ctx| {
        let loaded = load_source(&source_document, &workspace, ctx)?;
        let result = prepare(&destination, loaded.state.clone(), &workspace, &[], ctx);
        loaded.cleanup();
        prepared = Some(result?);
        Ok(json!(null))
    });
    outcome.unwrap();
    let prepared = prepared.unwrap();
    assert!(
        prepared
            .plan
            .warnings
            .iter()
            .any(|warning| warning.contains("directed scene")),
        "{:?}",
        prepared.plan.warnings
    );
    assert_eq!(
        prepared.source.scenes.ids(),
        ["street"],
        "the source keeps its scenes"
    );
    assert!(destination.scenes.is_empty());
    assert_eq!(
        fs::read(&source_document).unwrap(),
        source_bytes,
        "the source project was only read"
    );
    fs::remove_dir_all(dir).ok();
}
