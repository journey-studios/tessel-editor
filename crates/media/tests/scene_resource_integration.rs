//! F040 (BL010/BL011): directed-scene resources inside the editor, through the same surfaces a
//! window, a CLI or a library caller uses: `scene.resources.*` and `project.import_scenes` on
//! the `MediaHost`, `package::{preflight_with, create_with, restore}` and `persist`.
//!
//! Everything is synthetic (procedural PNGs, the repo's oracle bundle restated with their real
//! hashes), so no private media and no FFmpeg are needed. No renderer exists in this crate, so
//! what is proven is dependencies and files: which paths are approved, with which hash and
//! generation, what is written where, what is left unchanged on failure. No pixels.

mod directed_resources_common;

use std::fs;
use std::path::{Path, PathBuf};

use directed_resources_common::*;
use serde_json::{Value, json};
use tessel_editor_core::jobs::{CancelToken, JobError, JobKind, JobRegistry};
use tessel_editor_core::scene_resources::{ResourceKey, ResourceKind};
use tessel_editor_core::{CommandError, EditorState, headless, persist};
use tessel_editor_media::directed_resources::Options;
use tessel_editor_media::host::MediaHost;
use tessel_editor_media::package::{self, PackageFormat, PackageInputs, PackageOptions};
use tessel_editor_media::scene_resources::{
    ResolveError, ResourceBase, ResourceResolver, SceneResourceSession, adopt,
};

/// A workspace with the project saved in it and one scene (`street`) attached from a bundle
/// whose files live in `sandbox.src`.
struct World {
    sandbox: Sandbox,
    state: EditorState,
    host: MediaHost,
    session: SceneResourceSession,
    workspace: PathBuf,
}

fn attach_bundle(sandbox: &Sandbox, name: &str, document: &Value) -> PathBuf {
    let path = sandbox
        .root
        .join("bundles")
        .join(format!("{name}.bundle.json"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, serde_json::to_vec_pretty(document).unwrap()).unwrap();
    path
}

fn project(sandbox: &Sandbox, workspace: &Path, scene_id: &str, bundle: &Value) -> EditorState {
    let mut state = EditorState::demo();
    headless::execute(&mut state, &json!({"op":"project.create","name":"Scenes"})).unwrap();
    let path = attach_bundle(sandbox, scene_id, bundle);
    headless::execute(
        &mut state,
        &json!({"op":"scene.attach","id":scene_id,"path":path}),
    )
    .unwrap();
    persist::save(&mut state, &workspace.join("p.tessel.json")).unwrap();
    state
}

fn world(name: &str) -> World {
    let sandbox = Sandbox::new(name);
    let workspace = sandbox.root.join("ws");
    fs::create_dir_all(&workspace).unwrap();
    let bundle = standard_bundle(&sandbox);
    let state = project(&sandbox, &workspace, "street", &bundle);
    let session = SceneResourceSession::default();
    let host = MediaHost::new().with_scene_resources(session.clone());
    World {
        sandbox,
        state,
        host,
        session,
        workspace,
    }
}

impl World {
    fn run(&mut self, request: Value) -> Result<Value, CommandError> {
        headless::execute_ext(&mut self.state, None, Some(&mut self.host), &request)
    }

    fn ok(&mut self, request: Value) -> Value {
        self.run(request.clone())
            .unwrap_or_else(|error| panic!("{request}: {error:?}"))
    }

    fn adopt_from_source(&mut self) -> Value {
        let source = self.sandbox.src.clone();
        self.ok(json!({"op":"scene.resources.adopt","scene":"street","source":source}))
    }

    fn managed(&self) -> PathBuf {
        self.workspace.join("scenes/street")
    }

    fn bundle_hash(&self, asset: &str) -> String {
        let attached = self.state.scenes.get("street").unwrap();
        let manifest =
            tessel_editor_core::scene_resources::ResourceManifest::from_scene(&attached.scene);
        manifest
            .transport_plan()
            .unwrap()
            .items
            .iter()
            .find(|item| {
                item.resources
                    .iter()
                    .any(|resource| resource.key == ResourceKey::asset(asset))
            })
            .unwrap()
            .expected_hash
            .clone()
    }
}

/// The code a failure carries. A host job reports `job_failed` and keeps the job's own code in
/// `details.job_code`; a request refused before any job ran carries its code directly.
fn code(error: &CommandError) -> String {
    error
        .details
        .as_ref()
        .and_then(|details| details["job_code"].as_str())
        .unwrap_or(error.code)
        .to_string()
}

fn package_options(format: PackageFormat, allow_partial: bool) -> PackageOptions {
    PackageOptions {
        format,
        sequences: None,
        allow_partial,
        replace: false,
    }
}

fn create(
    state: &EditorState,
    destination: &Path,
    options: PackageOptions,
    inputs: &PackageInputs,
) -> Result<Value, JobError> {
    let snapshot = state.clone();
    JobRegistry::new(1)
        .run_inline(JobKind::Package, "package", |ctx| {
            package::create_with(&snapshot, destination, &options, inputs, ctx)
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

// ---------------------------------------------------------------- load, resolve, session --

#[test]
fn nothing_is_approved_until_a_scene_is_loaded_and_loading_reports_what_is_missing() {
    let mut world = world("not-loaded");
    let error = world
        .session
        .resolve("street", &ResourceKey::asset("backdrop"));
    assert_eq!(error.unwrap_err().code(), "scene_resources_not_loaded");

    // No managed base yet: the files were never brought into the workspace.
    let error = world
        .run(json!({"op":"scene.resources.resolve","scene":"street"}))
        .unwrap_err();
    assert_eq!(code(&error), "root_unavailable", "{error:?}");
    assert!(world.session.loaded_scenes().is_empty());
    assert!(
        !world.workspace.join("scenes").exists(),
        "a failed load creates nothing"
    );
}

#[test]
fn preflight_lists_every_problem_and_changes_nothing() {
    let mut world = world("preflight");
    fs::remove_file(world.sandbox.src.join("assets/figure.png")).unwrap();
    let source = world.sandbox.src.clone();
    let before = tree(&world.sandbox.root);
    let report = world.ok(json!({"op":"scene.resources.preflight","scene":"street","base":source}));
    assert_eq!(report["report"]["ok"], false, "{report}");
    assert!(report.to_string().contains("figure.png"));
    assert_eq!(tree(&world.sandbox.root), before, "preflight is read-only");
    assert!(world.session.loaded_scenes().is_empty());
}

#[test]
fn a_relative_base_is_refused_instead_of_meaning_the_working_directory() {
    let mut world = world("relative-base");
    let error = world
        .run(json!({"op":"scene.resources.preflight","scene":"street","base":"assets"}))
        .unwrap_err();
    assert_eq!(code(&error), "base_not_absolute");
}

#[test]
fn adopt_copies_into_the_managed_base_and_resolve_approves_path_hash_and_generation() {
    let mut world = world("adopt-resolve");
    let adopted = world.adopt_from_source();
    assert_eq!(adopted["base"]["kind"], "managed", "{adopted}");
    assert_eq!(
        tree(&world.managed()),
        ["assets", "assets/backdrop.png", "assets/figure.png"]
    );
    // The source is not the project's: deleting it changes nothing from here on.
    fs::remove_dir_all(&world.sandbox.src).unwrap();

    let resolved = world.ok(json!({"op":"scene.resources.resolve","scene":"street"}));
    assert_eq!(resolved["applied"]["installed"], true, "{resolved}");
    assert_eq!(resolved["applied"]["complete"], true);
    let generation = world.session.generation();
    assert_eq!(resolved["applied"]["generation"], generation.0);

    let approved = world
        .session
        .resolve("street", &ResourceKey::asset("backdrop"))
        .unwrap();
    assert_eq!(approved.generation, generation);
    assert_eq!(approved.content_hash, world.bundle_hash("backdrop"));
    assert!(
        approved
            .path
            .starts_with(fs::canonicalize(world.managed()).unwrap())
    );
    assert_eq!(
        sha256(&fs::read(&approved.path).unwrap()),
        approved.content_hash
    );
    assert_eq!(approved.kind, ResourceKind::Image);
    assert!(
        approved.cache_key.len() >= 32,
        "a scoped, content-addressed key"
    );

    // The same lookup by the bundle's own asset ID: what a renderer holds.
    let by_asset = world.session.resolve_asset("street", "figure").unwrap();
    assert_eq!(by_asset.content_hash, world.bundle_hash("figure"));
    assert_ne!(by_asset.cache_key, approved.cache_key);
    assert_eq!(
        world
            .session
            .resolve_asset("street", "nope")
            .unwrap_err()
            .code(),
        "resource_unknown"
    );
}

#[test]
fn adopting_again_reuses_and_adopting_a_broken_source_changes_nothing() {
    let mut world = world("adopt-reuse");
    world.adopt_from_source();
    let again = world.adopt_from_source();
    assert!(again.to_string().contains("reused"), "{again}");
    assert_eq!(again["report"]["bytesCopied"], 0, "{again}");

    // A second scene folder, from a source that is wrong: nothing is created.
    let broken = world.sandbox.outside.join("broken");
    fs::create_dir_all(broken.join("assets")).unwrap();
    fs::write(
        broken.join("assets/backdrop.png"),
        b"not what the bundle says",
    )
    .unwrap();
    fs::write(broken.join("assets/figure.png"), png(32, 32, 20)).unwrap();
    fs::remove_dir_all(world.managed()).unwrap();
    let before = tree(&world.workspace);
    let error = world
        .run(json!({"op":"scene.resources.adopt","scene":"street","source":broken}))
        .unwrap_err();
    assert_eq!(code(&error), "resources_invalid", "{error:?}");
    assert_eq!(tree(&world.workspace), before, "no folder, no stray file");
}

#[test]
fn relinking_is_adopting_from_the_new_location() {
    let mut world = world("relink");
    // The first adoption points at files that moved away; the user relinks to the new folder.
    let moved = world.sandbox.root.join("moved");
    fs::rename(&world.sandbox.src, &moved).unwrap();
    let error = world
        .run(json!({"op":"scene.resources.adopt","scene":"street","source":world.sandbox.src}))
        .unwrap_err();
    assert_eq!(code(&error), "root_unavailable", "{error:?}");
    world.ok(json!({"op":"scene.resources.adopt","scene":"street","source":moved}));
    world.ok(json!({"op":"scene.resources.resolve","scene":"street"}));
    assert_eq!(world.session.loaded_scenes(), ["street"]);
}

#[test]
fn cancelling_an_adoption_leaves_no_folder_behind_and_it_can_be_run_again() {
    let world = world("adopt-cancel");
    let scene = world.state.scenes.get("street").unwrap().scene.clone();
    let source = ResourceBase::explicit(&world.sandbox.src).unwrap();
    let token = CancelToken::new();
    let error = adopt(
        &scene,
        "street",
        &world.workspace,
        &source,
        &Options::default(),
        &token,
        &mut |_| {},
        &mut |_| token.cancel(),
    )
    .unwrap_err();
    assert_eq!(error.code, "cancelled");
    assert!(
        !world.workspace.join("scenes").exists(),
        "the folders this run created are gone: {:?}",
        tree(&world.workspace)
    );
    adopt(
        &scene,
        "street",
        &world.workspace,
        &source,
        &Options::default(),
        &CancelToken::new(),
        &mut |_| {},
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(tree(&world.managed()).len(), 3);
}

#[test]
fn a_map_from_an_old_generation_is_never_installed() {
    let mut world = world("stale");
    world.adopt_from_source();
    let before = world.session.generation();
    let scene = world.state.scenes.get("street").unwrap().scene.clone();
    let base = ResourceBase::managed(&world.workspace, "street").unwrap();
    let map = tessel_editor_media::scene_resources::load(
        &tessel_editor_media::scene_resources::LoadRequest {
            scene: &scene,
            scene_id: "street",
            project_id: &world.state.project.id,
            generation: before,
            base: &base,
        },
        &Options::default(),
        &CancelToken::new(),
        &mut |_| {},
    )
    .unwrap();
    // The renderer lost its context while the job was running.
    let invalidated = world.ok(json!({"op":"scene.resources.invalidate","reason":"context_loss"}));
    assert!(invalidated["generation"].as_u64().unwrap() > before.0);
    let error = world.session.install(map).unwrap_err();
    assert!(matches!(error, ResolveError::Stale { .. }), "{error:?}");
    assert!(world.session.loaded_scenes().is_empty());
}

#[test]
fn invalidate_release_detach_and_a_new_project_each_drop_what_was_approved() {
    let mut world = world("invalidate");
    world.adopt_from_source();
    let load = |world: &mut World| {
        world.ok(json!({"op":"scene.resources.resolve","scene":"street"}));
        assert_eq!(world.session.loaded_scenes(), ["street"]);
    };

    load(&mut world);
    let status = world.ok(json!({"op":"scene.resources.status"}));
    assert_eq!(status["attached"][0]["scene"], "street");
    assert_eq!(status["attached"][0]["current"], true, "{status}");
    world.ok(json!({"op":"scene.resources.invalidate","reason":"reload"}));
    assert!(world.session.loaded_scenes().is_empty());

    load(&mut world);
    assert_eq!(
        world.ok(json!({"op":"scene.resources.release","scene":"street"}))["released"],
        true
    );
    assert!(world.session.loaded_scenes().is_empty());

    load(&mut world);
    world.ok(json!({"op":"scene.detach","scene":"street"}));
    assert!(
        world.session.loaded_scenes().is_empty(),
        "detaching releases its resources"
    );

    // Opening another project (here: creating one) starts a new generation.
    let mut world = self::world("invalidate-project");
    world.adopt_from_source();
    load(&mut world);
    let generation = world.session.generation();
    world.ok(json!({"op":"project.create","name":"Other"}));
    world.ok(json!({"op":"scene.resources.status"}));
    assert!(world.session.loaded_scenes().is_empty());
    assert!(world.session.generation() > generation);
}

#[test]
fn a_byte_budget_refuses_a_scene_without_installing_part_of_it() {
    let mut world = world("budget");
    world.adopt_from_source();
    let session = SceneResourceSession::with_budget(16);
    world.host = MediaHost::new().with_scene_resources(session.clone());
    let error = world
        .run(json!({"op":"scene.resources.resolve","scene":"street"}))
        .unwrap_err();
    assert_eq!(code(&error), "resource_budget_exceeded", "{error:?}");
    assert!(session.loaded_scenes().is_empty());
    assert_eq!(session.held_bytes(), 0);
}

#[test]
fn a_changed_file_is_caught_at_load_not_at_draw_time() {
    let mut world = world("changed");
    world.adopt_from_source();
    fs::write(world.managed().join("assets/figure.png"), png(32, 32, 99)).unwrap();
    let error = world
        .run(json!({"op":"scene.resources.resolve","scene":"street"}))
        .unwrap_err();
    assert_eq!(code(&error), "resources_invalid", "{error:?}");
    assert!(world.session.loaded_scenes().is_empty());
}

// ------------------------------------------------------------------ save, reopen, move --

#[test]
fn the_base_survives_save_reopen_and_moving_the_whole_workspace() {
    let mut world = world("reopen");
    world.adopt_from_source();
    persist::save(&mut world.state, &world.workspace.join("p.tessel.json")).unwrap();

    let moved = world.sandbox.root.join("elsewhere/renamed-workspace");
    fs::create_dir_all(moved.parent().unwrap()).unwrap();
    fs::rename(&world.workspace, &moved).unwrap();
    fs::remove_dir_all(&world.sandbox.src).unwrap();

    let mut reopened = persist::load(&moved.join("p.tessel.json")).unwrap();
    let session = SceneResourceSession::default();
    let mut host = MediaHost::new().with_scene_resources(session.clone());
    let request = json!({"op":"scene.resources.resolve","scene":"street"});
    let result = headless::execute_ext(&mut reopened, None, Some(&mut host), &request).unwrap();
    assert_eq!(result["applied"]["installed"], true, "{result}");
    let approved = session.resolve_asset("street", "backdrop").unwrap();
    assert!(approved.path.starts_with(fs::canonicalize(&moved).unwrap()));
}

// ----------------------------------------------------------------------- package / restore --

#[test]
fn a_package_carries_the_scene_files_and_restores_without_the_originals() {
    for format in [PackageFormat::Directory, PackageFormat::Archive] {
        let mut world = world(&format!("package-{format:?}"));
        world.adopt_from_source();
        let pack = world.sandbox.root.join("out/street.tesselpack");

        let plan = package::preflight_with(
            &world.state,
            &package_options(format, false),
            &PackageInputs::default(),
            &pack,
            &CancelToken::new(),
        )
        .unwrap();
        assert_eq!(plan["ok"], true, "{plan}");
        assert_eq!(plan["complete"], true);
        assert_eq!(plan["scene_files"], 2);

        let made = create(
            &world.state,
            &pack,
            package_options(format, false),
            &PackageInputs::default(),
        )
        .unwrap();
        assert_eq!(made["complete"], true, "{made}");
        assert_eq!(made["scene_files"], 2, "{made}");

        // Everything the machine had is gone: the originals and the workspace.
        fs::remove_dir_all(&world.sandbox.src).unwrap();
        fs::remove_dir_all(&world.workspace).unwrap();

        let restored = world.sandbox.root.join("restored");
        let report = restore(&pack, &restored).unwrap();
        assert_eq!(report["complete"], true, "{report}");
        assert_eq!(report["unresolved_scenes"], json!([]));

        // The hash is the bundle's, not merely "whatever was copied".
        for (asset, file) in [("backdrop", "backdrop.png"), ("figure", "figure.png")] {
            let bytes = fs::read(restored.join("scenes/street/assets").join(file)).unwrap();
            assert_eq!(
                sha256(&bytes),
                world.bundle_hash(asset),
                "{format:?} {asset}"
            );
        }

        // And it opens: the restored project resolves its scene with no access to the origin.
        let mut reopened = persist::load(&restored.join(package::DOCUMENT_NAME)).unwrap();
        let session = SceneResourceSession::default();
        let mut host = MediaHost::new().with_scene_resources(session.clone());
        let resolved = headless::execute_ext(
            &mut reopened,
            None,
            Some(&mut host),
            &json!({"op":"scene.resources.resolve","scene":"street"}),
        )
        .unwrap();
        assert_eq!(resolved["applied"]["complete"], true, "{resolved}");
        assert_eq!(session.loaded_scenes(), ["street"]);
    }
}

#[test]
fn a_package_can_carry_a_scene_straight_from_a_folder_the_caller_authorizes() {
    let mut world = world("package-explicit-base");
    let pack = world.sandbox.root.join("out/direct.tesselpack");
    // No adoption: without an authorized folder the scene cannot be carried and says so.
    let refused = create(
        &world.state,
        &pack,
        package_options(PackageFormat::Directory, false),
        &PackageInputs::default(),
    )
    .unwrap_err();
    assert_eq!(refused.code, "unsupported");
    assert!(!pack.exists());

    let inputs = PackageInputs {
        scene_bases: [("street".to_string(), world.sandbox.src.clone())].into(),
    };
    let made = create(
        &world.state,
        &pack,
        package_options(PackageFormat::Directory, false),
        &inputs,
    )
    .unwrap();
    assert_eq!(made["complete"], true, "{made}");
    // The request surface takes the same thing.
    let preflight = world.ok(json!({"op":"project.package_preflight","path":pack,
        "format":"directory","scene_bases":{"street":world.sandbox.src}}));
    assert_eq!(preflight["complete"], true, "{preflight}");
}

#[test]
fn a_missing_scene_file_stops_the_package_and_a_partial_one_says_which() {
    let world = world("package-missing");
    fs::remove_file(world.sandbox.src.join("assets/figure.png")).unwrap();
    let inputs = PackageInputs {
        scene_bases: [("street".to_string(), world.sandbox.src.clone())].into(),
    };
    let pack = world.sandbox.root.join("out/missing.tesselpack");

    let error = create(
        &world.state,
        &pack,
        package_options(PackageFormat::Directory, false),
        &inputs,
    )
    .unwrap_err();
    assert_eq!(error.code, "resources_invalid", "{error:?}");
    assert!(!pack.exists(), "nothing half-written");
    assert_eq!(tree(&world.sandbox.root.join("out")), Vec::<String>::new());

    let plan = package::preflight_with(
        &world.state,
        &package_options(PackageFormat::Directory, false),
        &inputs,
        &pack,
        &CancelToken::new(),
    )
    .unwrap();
    assert_eq!(plan["ok"], false);
    assert_eq!(plan["blockers"][0]["code"], "missing_media", "{plan}");

    let partial = create(
        &world.state,
        &pack,
        package_options(PackageFormat::Directory, true),
        &inputs,
    )
    .unwrap();
    assert_eq!(partial["complete"], false);
    assert_eq!(partial["missing"][0]["asset"], "scene:street");
    assert_ne!(partial["missing"][0]["state"], "not_packaged");
    assert!(
        !pack.join("scenes").exists(),
        "a scene that cannot be verified is not half-carried"
    );
}

#[test]
fn a_package_missing_a_scene_file_is_refused_on_restore() {
    let mut world = world("restore-tampered");
    world.adopt_from_source();
    let pack = world.sandbox.root.join("out/tampered.tesselpack");
    create(
        &world.state,
        &pack,
        package_options(PackageFormat::Directory, false),
        &PackageInputs::default(),
    )
    .unwrap();
    fs::remove_file(pack.join("scenes/street/assets/figure.png")).unwrap();

    let error = restore(&pack, &world.sandbox.root.join("restored")).unwrap_err();
    assert!(
        ["integrity_error", "invalid_package"].contains(&error.code.as_str()),
        "{error:?}"
    );
    assert!(!world.sandbox.root.join("restored").exists());
}

// ------------------------------------------------------------------------ import_scenes --

/// A second project with its own scene `street`: the same relative paths, other pixels.
fn other_project(name: &str) -> (Sandbox, PathBuf) {
    let sandbox = Sandbox::new(name);
    let workspace = sandbox.root.join("ws");
    fs::create_dir_all(&workspace).unwrap();
    let mut bundle = fixture();
    let backdrop = sandbox.source_file("assets/backdrop.png", &png(64, 36, 111));
    let figure = sandbox.source_file("assets/figure.png", &png(32, 32, 222));
    restate(&mut bundle, "backdrop", "assets/backdrop.png", &backdrop);
    restate(&mut bundle, "figure", "assets/figure.png", &figure);
    project(&sandbox, &workspace, "street", &bundle);
    (sandbox, workspace.join("p.tessel.json"))
}

#[test]
fn imported_scenes_keep_their_files_apart_when_ids_and_paths_collide() {
    let mut world = world("import-collision");
    world.adopt_from_source();
    let (other, document) = other_project("import-collision-source");

    // Dry run: the plan names the new ID and touches nothing.
    let before = tree(&world.workspace);
    let plan = world.ok(json!({"op":"project.import_scenes","source":document,"dry_run":true}));
    assert_eq!(plan["scenes"][0]["from"], "street");
    assert_eq!(plan["scenes"][0]["to"], "street-2", "{plan}");
    assert_eq!(plan["scenes"][0]["renamed"], true);
    assert_eq!(tree(&world.workspace), before);
    assert_eq!(world.state.scenes.ids(), ["street"]);

    let result = world.ok(json!({"op":"project.import_scenes","source":document,
        "scene_bases":{"street":other.src}}));
    assert_eq!(world.state.scenes.ids(), ["street", "street-2"], "{result}");

    // Same relative path, two scenes, two different files.
    let mine = fs::read(world.workspace.join("scenes/street/assets/backdrop.png")).unwrap();
    let theirs = fs::read(world.workspace.join("scenes/street-2/assets/backdrop.png")).unwrap();
    assert_eq!(theirs, png(64, 36, 111));
    assert_ne!(mine, theirs);

    // Both resolve, each to its own bundle's hashes.
    for id in ["street", "street-2"] {
        world.ok(json!({"op":"scene.resources.resolve","scene":id}));
    }
    let first = world.session.resolve_asset("street", "backdrop").unwrap();
    let second = world.session.resolve_asset("street-2", "backdrop").unwrap();
    assert_ne!(first.content_hash, second.content_hash);
    assert_ne!(first.path, second.path);
    assert_eq!(sha256(&theirs), second.content_hash);

    // One undo step takes the attached scenes back out of the project.
    world.ok(json!({"op":"history.undo"}));
    assert_eq!(world.state.scenes.ids(), ["street"]);
}

#[test]
fn an_import_whose_files_are_broken_changes_neither_the_project_nor_the_disk() {
    let mut world = world("import-broken");
    let (other, document) = other_project("import-broken-source");
    fs::remove_file(other.src.join("assets/figure.png")).unwrap();
    let before = tree(&world.workspace);
    let error = world
        .run(json!({"op":"project.import_scenes","source":document,
            "scene_bases":{"street":other.src}}))
        .unwrap_err();
    assert_eq!(code(&error), "resources_invalid", "{error:?}");
    assert_eq!(tree(&world.workspace), before, "no scenes/ folder either");
    assert_eq!(world.state.scenes.ids(), ["street"]);
}
