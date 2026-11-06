//! F048 (S7, BL025 subset): a Gaussian splat of a directed scene travels like any other approved
//! resource — adopted into the project, packed, restored without the originals, resolved again.
//!
//! Synthetic (procedural PNG/SPZ, the repo's minimal bundle restated with real hashes); no
//! renderer is involved here: what is proven is that the `.spz` file reaches the restored project
//! with the hash the bundle declares, and that the restored scene still plans the splat.

mod directed_resources_common;

use std::fs;
use std::path::{Path, PathBuf};

use directed_resources_common::*;
use serde_json::{Value, json};
use tessel_editor_core::jobs::{CancelToken, JobError, JobKind, JobRegistry};
use tessel_editor_core::scene_plan::{Capability, ScenePlan};
use tessel_editor_core::{EditorState, headless, persist};
use tessel_editor_media::host::MediaHost;
use tessel_editor_media::package::{self, PackageFormat, PackageInputs, PackageOptions};
use tessel_editor_media::scene_resources::SceneResourceSession;

struct World {
    sandbox: Sandbox,
    state: EditorState,
    workspace: PathBuf,
    host: MediaHost,
    cloud_hash: String,
}

fn world(name: &str) -> World {
    let sandbox = Sandbox::new(name);
    let workspace = sandbox.root.join("ws");
    fs::create_dir_all(&workspace).unwrap();
    let mut bundle = standard_bundle(&sandbox);
    let cloud_hash = sandbox.source_file("splats/cloud.spz", &spz());
    add_asset(
        &mut bundle,
        "cloud",
        "splats/cloud.spz",
        &cloud_hash,
        Some("spz"),
        "srgb",
    );
    add_node(
        &mut bundle,
        "cloud-node",
        "splat",
        reference("cloud", "splats/cloud.spz", &cloud_hash),
    );
    let path = sandbox.root.join("bundles/street.bundle.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, serde_json::to_vec_pretty(&bundle).unwrap()).unwrap();

    let mut state = EditorState::demo();
    headless::execute(&mut state, &json!({"op":"project.create","name":"Splats"})).unwrap();
    headless::execute(
        &mut state,
        &json!({"op":"scene.attach","id":"street","path":path}),
    )
    .unwrap();
    persist::save(&mut state, &workspace.join("p.tessel.json")).unwrap();
    World {
        sandbox,
        state,
        workspace,
        host: MediaHost::new().with_scene_resources(SceneResourceSession::default()),
        cloud_hash,
    }
}

impl World {
    fn ok(&mut self, request: Value) -> Value {
        headless::execute_ext(&mut self.state, None, Some(&mut self.host), &request)
            .unwrap_or_else(|error| panic!("{request}: {error:?}"))
    }
}

fn options(format: PackageFormat) -> PackageOptions {
    PackageOptions {
        format,
        sequences: None,
        allow_partial: false,
        replace: false,
    }
}

fn create(
    state: &EditorState,
    destination: &Path,
    format: PackageFormat,
) -> Result<Value, JobError> {
    let snapshot = state.clone();
    JobRegistry::new(1)
        .run_inline(JobKind::Package, "package", |ctx| {
            package::create_with(
                &snapshot,
                destination,
                &options(format),
                &PackageInputs::default(),
                ctx,
            )
        })
        .1
}

fn restore(pack: &Path, destination: &Path) -> Result<Value, JobError> {
    JobRegistry::new(1)
        .run_inline(JobKind::Restore, "restore", |ctx| {
            package::restore(pack, destination, ctx)
        })
        .1
}

#[test]
fn a_splat_is_adopted_packed_and_restored_with_the_hash_the_bundle_declares() {
    for format in [PackageFormat::Directory, PackageFormat::Archive] {
        let mut world = world(&format!("splat-package-{format:?}"));
        let source = world.sandbox.src.clone();
        world.ok(json!({"op":"scene.resources.adopt","scene":"street","source":source}));
        assert!(
            world
                .workspace
                .join("scenes/street/splats/cloud.spz")
                .is_file(),
            "{format:?}: the splat is adopted into the project's own folder"
        );

        let pack = world.sandbox.root.join("out/street.tesselpack");
        let plan = package::preflight_with(
            &world.state,
            &options(format),
            &PackageInputs::default(),
            &pack,
            &CancelToken::new(),
        )
        .unwrap();
        assert_eq!(plan["ok"], true, "{plan}");
        assert_eq!(plan["complete"], true, "{plan}");
        assert_eq!(plan["scene_files"], 3, "two pictures and one splat: {plan}");

        let made = create(&world.state, &pack, format).unwrap();
        assert_eq!(made["complete"], true, "{made}");
        assert_eq!(made["scene_files"], 3, "{made}");

        // Everything the machine had is gone: the originals and the workspace.
        fs::remove_dir_all(&world.sandbox.src).unwrap();
        fs::remove_dir_all(&world.workspace).unwrap();

        let restored = world.sandbox.root.join("restored");
        let report = restore(&pack, &restored).unwrap();
        assert_eq!(report["complete"], true, "{report}");
        assert_eq!(report["unresolved_scenes"], json!([]));

        let bytes = fs::read(restored.join("scenes/street/splats/cloud.spz")).unwrap();
        assert_eq!(
            sha256(&bytes),
            world.cloud_hash,
            "{format:?}: the same bytes"
        );
        assert_eq!(bytes, spz(), "{format:?}");

        // It opens: the restored project resolves the scene alone and still plans the splat.
        let mut reopened = persist::load(&restored.join(package::DOCUMENT_NAME)).unwrap();
        let mut host = MediaHost::new().with_scene_resources(SceneResourceSession::default());
        let resolved = headless::execute_ext(
            &mut reopened,
            None,
            Some(&mut host),
            &json!({"op":"scene.resources.resolve","scene":"street"}),
        )
        .unwrap();
        assert_eq!(resolved["applied"]["complete"], true, "{resolved}");
        let attached = reopened.scenes.get("street").unwrap();
        let plan = ScenePlan::build(&attached.scene, 0).unwrap();
        assert_eq!(plan.splats.len(), 1, "{format:?}");
        assert!(plan.requires.contains(&Capability::Splats));
        assert_eq!(
            plan.resource("cloud").unwrap().content_hash,
            world.cloud_hash
        );
    }
}
