//! BL019 / front P: resource flow of the directed scene over the synthetic corpus, with the
//! lib-level pieces that exist today (plan × manifest × `directed_resources`), and the package
//! round trip that does not exist yet.
//!
//! What this proves (lib, in process, no Chrome, no GPU, no window):
//!
//! - the corpus has real hashes, so the manifest is transportable and the plan's resource
//!   identity (assetId, revision, hash, path) equals the manifest's, frame after frame;
//! - `preflight` + `materialize` carry the three PNGs to a transport root, the origin folder is
//!   then deleted, and every resource of every frame is found **by assetId from the transport
//!   root alone**, byte-identical to what the origin held;
//! - a swapped revision of one assetId lands beside the first under its own path, and an
//!   in-place swap (same assetId, same path, other content) is refused as `destination_conflict`
//!   instead of overwriting: the assetId is never the key of the content.
//!
//! What it does not prove: a real package or restore (the ignored test below), pixels, the
//! renderer's resource cache, context loss, the UI, any pointer. Surfaces other than the lib are
//! `scripts/directed-scene-proof/proof.py`.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::jobs::CancelToken;
use tessel_editor_core::scene_plan::ScenePlan;
use tessel_editor_core::scene_resources::ResourceManifest;
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::{EditorState, headless};
use tessel_editor_media::directed_resources::{Options, materialize, preflight};
use tessel_editor_media::host::MediaHost;

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/directed-scene-proof/corpus")
}

fn expected() -> Value {
    serde_json::from_str(&fs::read_to_string(corpus().join("expected.json")).unwrap()).unwrap()
}

/// A scratch directory of this test, removed on drop; nothing outside it is touched.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "tessel-scene-flow-res-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Copies the corpus (bundle + images, nothing else) into `target` so the "origin" of a test is
/// its own throwaway folder and can be deleted.
fn copy_origin(target: &Path) {
    for entry in [
        "bundle.json",
        "bundle.swap.json",
        "assets/sky.png",
        "assets/mid.png",
        "assets/hero.png",
        "assets-rev2/hero.png",
    ] {
        let to = target.join(entry);
        fs::create_dir_all(to.parent().unwrap()).unwrap();
        fs::copy(corpus().join(entry), to).unwrap();
    }
}

fn scene_of(path: &Path) -> DirectedScene {
    DirectedScene::import_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn carry(
    manifest: &ResourceManifest,
    source: &Path,
    destination: &Path,
) -> Result<usize, tessel_editor_core::jobs::JobError> {
    // `materialize` requires an existing transport root (the approved destination); it never
    // creates one, so the test plays the user who picked the folder.
    fs::create_dir_all(destination).unwrap();
    let token = CancelToken::new();
    let options = Options::default();
    let checked = preflight(manifest, source, &options, &token, &mut |_| {})?;
    assert!(
        checked.ok(),
        "preflight: {:?} {:?}",
        checked.problems,
        checked.manifest_issues
    );
    materialize(manifest, source, destination, &options, &token, &mut |_| {})
        .map(|report| report.copied())
}

fn png_size(bytes: &[u8]) -> (u32, u32) {
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "not a PNG");
    (
        u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
        u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
    )
}

#[test]
fn plan_and_manifest_agree_on_the_identity_of_every_resource_in_every_frame() {
    let expected = expected();
    let scene = scene_of(&corpus().join("bundle.json"));
    let manifest = ResourceManifest::from_scene(&scene);
    assert!(
        manifest.is_transportable(),
        "{:?}",
        manifest.blocking_issues().collect::<Vec<_>>()
    );
    assert!(
        manifest.is_complete(),
        "the corpus names only PNGs: nothing may be skipped"
    );
    for frame in expected["seekOrder"].as_array().unwrap() {
        let plan = ScenePlan::build(&scene, frame.as_i64().unwrap()).unwrap();
        let lookup = manifest.lookup(&plan.required_asset_ids());
        assert!(
            lookup.unknown.is_empty(),
            "frame {frame}: {:?}",
            lookup.unknown
        );
        for resource in &plan.resources {
            let entry = manifest.resource(&resource.asset_id).unwrap();
            assert_eq!(
                entry.content_hash.as_deref(),
                Some(resource.content_hash.as_str())
            );
            assert_eq!(
                entry.revision_id.as_deref(),
                Some(resource.revision_id.as_str())
            );
            assert_eq!(entry.relative_path, resource.relative_path);
            assert_eq!(entry.width_px, Some(resource.width_px));
        }
    }
}

#[test]
fn resources_are_found_by_asset_id_from_the_transport_root_after_the_origin_is_gone() {
    let expected = expected();
    let scratch = Scratch::new("no-origin");
    let (origin, transport) = (scratch.path("origin"), scratch.path("transport"));
    copy_origin(&origin);
    let scene = scene_of(&origin.join("bundle.json"));
    let manifest = ResourceManifest::from_scene(&scene);
    let held: Vec<(String, Vec<u8>)> = ["sky", "mid", "hero"]
        .iter()
        .map(|id| {
            let file = expected["assets"][*id]["file"].as_str().unwrap();
            (id.to_string(), fs::read(origin.join(file)).unwrap())
        })
        .collect();

    assert_eq!(carry(&manifest, &origin, &transport).unwrap(), 3);
    fs::remove_dir_all(&origin).unwrap(); // from here the origin does not exist
    assert!(!origin.exists());

    for frame in [0_i64, 47, 48, 95] {
        let plan = ScenePlan::build(&scene, frame).unwrap();
        for resource in &plan.resources {
            let path = transport.join(&resource.relative_path);
            let bytes =
                fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            let (_, original) = held
                .iter()
                .find(|(id, _)| *id == resource.asset_id)
                .unwrap();
            assert_eq!(
                &bytes, original,
                "{} differs from the origin's bytes",
                resource.asset_id
            );
            assert_eq!(
                png_size(&bytes),
                (resource.width_px as u32, resource.height_px as u32)
            );
        }
    }
}

#[test]
fn a_swapped_revision_lands_beside_the_first_and_an_in_place_swap_is_refused() {
    let expected = expected();
    let scratch = Scratch::new("swap");
    let (origin, transport) = (scratch.path("origin"), scratch.path("transport"));
    copy_origin(&origin);
    let first = ResourceManifest::from_scene(&scene_of(&origin.join("bundle.json")));
    let swapped = ResourceManifest::from_scene(&scene_of(&origin.join("bundle.swap.json")));
    assert_eq!(carry(&first, &origin, &transport).unwrap(), 3);

    // Revision 2 of `hero` has its own path: one new file, the rest reused, the old one kept.
    assert_eq!(carry(&swapped, &origin, &transport).unwrap(), 1);
    let old = fs::read(transport.join("assets/hero.png")).unwrap();
    let new = fs::read(transport.join("assets-rev2/hero.png")).unwrap();
    assert_ne!(old, new);
    assert_eq!(
        swapped.resource("hero").unwrap().content_hash.as_deref(),
        expected["swapAsset"]["sha256"].as_str()
    );

    // Same assetId, same path, other content: never an overwrite.
    let in_place = scratch.path("in-place");
    copy_origin(&in_place);
    fs::copy(
        in_place.join("assets-rev2/hero.png"),
        in_place.join("assets/hero.png"),
    )
    .unwrap();
    let mut bundle: Value =
        serde_json::from_str(&fs::read_to_string(in_place.join("bundle.swap.json")).unwrap())
            .unwrap();
    bundle["assets"][2]["relativePath"] = json!("assets/hero.png");
    for node in bundle["sceneDefinitions"][0]["nodes"]
        .as_array_mut()
        .unwrap()
    {
        if node["assetRef"]["assetId"] == "hero" {
            node["assetRef"]["relativePath"] = json!("assets/hero.png");
        }
    }
    let conflicting =
        ResourceManifest::from_scene(&DirectedScene::import_str(&bundle.to_string()).unwrap());
    let before = fs::read(transport.join("assets/hero.png")).unwrap();
    let error = carry(&conflicting, &in_place, &transport).unwrap_err();
    // The job fails as a whole (`resources_invalid`); the conflict is the named problem inside it.
    assert_eq!(error.code, "resources_invalid", "{error:?}");
    let problems = error
        .details
        .as_ref()
        .and_then(|details| details["problems"].as_array())
        .unwrap();
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert_eq!(problems[0]["code"], "destination_conflict");
    assert_eq!(problems[0]["path"], "assets/hero.png");
    assert_eq!(
        fs::read(transport.join("assets/hero.png")).unwrap(),
        before,
        "the file was overwritten"
    );
}

/// The package carries adopted scene resources and restores without the origin.
#[test]
fn the_package_carries_the_scene_resources_and_restores_without_the_origin() {
    let scratch = Scratch::new("package");
    let origin = scratch.path("origin");
    copy_origin(&origin);
    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    let mut media = MediaHost::new();
    let mut send = |state: &mut EditorState, request: Value| {
        headless::execute_ext(state, None, Some(&mut media), &request)
    };
    send(
        &mut state,
        json!({"op":"project.create","name":"pkg","dir":scratch.path("ws")}),
    )
    .unwrap();
    send(
        &mut state,
        json!({"op":"scene.attach","id":"proof","path":origin.join("bundle.json")}),
    )
    .unwrap();
    send(
        &mut state,
        json!({"op":"project.save","path":scratch.path("ws/pkg.tessel.json")}),
    )
    .unwrap();
    send(
        &mut state,
        json!({"op":"scene.resources.adopt","scene":"proof","source":origin}),
    )
    .unwrap();
    let zip = scratch.path("out.zip");
    let report = send(
        &mut state,
        json!({"op":"project.package_preflight","path":zip}),
    )
    .unwrap();
    assert!(
        report["complete"] == true && report["ok"] == true,
        "BLOCKED BL011: package preflight is incomplete: missing={} blockers={}",
        report["missing"],
        report["blockers"]
    );
    send(&mut state, json!({"op":"project.package","path":zip})).unwrap();
    fs::remove_dir_all(&origin).unwrap();
    let restored = scratch.path("restored");
    send(
        &mut state,
        json!({"op":"project.restore","path":zip,"folder":restored}),
    )
    .unwrap();
    let expected = expected();
    for id in ["sky", "mid", "hero"] {
        let file = expected["assets"][id]["file"].as_str().unwrap();
        let found = walk(&restored).into_iter().any(|path| path.ends_with(file));
        assert!(found, "restored root lacks {id} ({file})");
    }
}

fn walk(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                found.push(path);
            }
        }
    }
    found
}
