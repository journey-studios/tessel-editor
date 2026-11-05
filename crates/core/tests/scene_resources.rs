//! F038 (BB06 initial): the resource manifest of a directed-scene bundle, through the public
//! library API only (no filesystem, no GPUI, no media adapter).
//!
//! The bundle is the oracle's synthetic `valid-minimal-continuous` (two PNG planes), extended
//! in memory with the assets, nodes and preserved sections each test needs. Hashes are fake
//! 64-hex strings: the core never reads a file, so only their *shape* matters here; real bytes
//! and real hashes are exercised by `crates/media/tests/directed_resources*.rs`.
//!
//! What this proves: classification (supported / preserved / unreferenced), uses with JSON
//! pointers, transitive closure, lookup by `assetId`, shared files, path collisions, non-portable
//! paths, declared-but-unread sections and determinism. What it does not: that a file exists,
//! matches its hash or can be copied.

use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::{DirectedScene, ImportError};
use tessel_editor_core::scene_resources::{
    CAPABILITIES, CollisionKind, LISTING_TRUNCATED, MANIFEST_VERSION, MAX_LISTED_FILES, PathError,
    REFERENCE_IGNORED, REFERENCE_MISMATCH, REFERENCE_UNRESOLVED, RESOURCE_FORMAT_MISMATCH,
    RESOURCE_HASH_INVALID, RESOURCE_PATH_COLLISION, RESOURCE_PATH_INVALID, ResourceKey,
    ResourceKind, ResourceManifest, ResourceStatus, ResourceUse, Severity, UseRole,
};

fn fixture() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json");
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn hash(seed: char) -> String {
    seed.to_string().repeat(64)
}

fn reference(asset_id: &str, path: &str, hash: &str) -> Value {
    json!({
        "assetId": asset_id,
        "revisionId": "rev-1",
        "contentHash": hash,
        "relativePath": path,
    })
}

fn asset(id: &str, path: &str, hash: &str, format: Option<&str>, color_space: &str) -> Value {
    let mut record = json!({
        "assetId": id,
        "revisionId": "rev-1",
        "contentHash": hash,
        "relativePath": path,
        "widthPx": 8,
        "heightPx": 8,
        "alphaMode": "opaque",
        "colorSpace": color_space,
    });
    if let Some(format) = format {
        record["format"] = json!(format);
    }
    record
}

fn nodes(document: &mut Value) -> &mut Vec<Value> {
    document["sceneDefinitions"][0]["nodes"]
        .as_array_mut()
        .unwrap()
}

fn node<'a>(document: &'a mut Value, id: &str) -> &'a mut Value {
    nodes(document)
        .iter_mut()
        .find(|node| node["id"] == id)
        .unwrap()
}

fn add_asset(document: &mut Value, record: Value) {
    document["assets"].as_array_mut().unwrap().push(record);
}

fn add_node(document: &mut Value, id: &str, kind: &str, asset: Option<Value>) {
    let mut record = json!({
        "id": id,
        "parentId": "root",
        "type": kind,
        "localPosition": {"x": 0, "y": 0, "z": 0},
        "localRotation": {"x": 0, "y": 0, "z": 0, "w": 1},
        "localScale": {"x": 1, "y": 1, "z": 1},
    });
    if let Some(asset) = asset {
        record["assetRef"] = asset;
    }
    if kind == "imagePlane" {
        record["widthSceneUnits"] = json!(1);
        record["heightSceneUnits"] = json!(1);
    }
    nodes(document).push(record);
}

/// Points an asset and the nodes that name it at a new path/hash, keeping both copies equal as
/// the importer requires.
fn restate(document: &mut Value, id: &str, path: &str, hash: &str) {
    for record in document["assets"].as_array_mut().unwrap() {
        if record["assetId"] == id {
            record["relativePath"] = json!(path);
            record["contentHash"] = json!(hash);
        }
    }
    for node in nodes(document) {
        if node["assetRef"]["assetId"] == id {
            node["assetRef"]["relativePath"] = json!(path);
            node["assetRef"]["contentHash"] = json!(hash);
        }
    }
}

/// The fixture with 64-hex hashes, so nothing blocks transport yet.
fn base() -> Value {
    let mut document = fixture();
    restate(&mut document, "backdrop", "assets/backdrop.png", &hash('a'));
    restate(&mut document, "figure", "assets/figure.png", &hash('b'));
    document
}

fn scene(document: Value) -> DirectedScene {
    DirectedScene::import_value(document)
        .unwrap_or_else(|error: ImportError| panic!("the test bundle must import: {error:?}"))
}

fn manifest(document: Value) -> ResourceManifest {
    ResourceManifest::from_scene(&scene(document))
}

fn codes(manifest: &ResourceManifest) -> Vec<&'static str> {
    manifest.issues().iter().map(|issue| issue.code).collect()
}

#[test]
fn image_planes_are_listed_by_asset_id_with_their_origin_path_and_uses() {
    let manifest = manifest(base());
    assert_eq!(manifest.bundle_id(), "oracle-minimal");
    assert_eq!(manifest.revision_id(), "rev-1");
    assert_eq!(manifest.len(), 2);

    let backdrop = manifest.resource("backdrop").unwrap();
    assert_eq!(backdrop.key, ResourceKey::asset("backdrop"));
    assert_eq!(backdrop.asset_id(), Some("backdrop"));
    assert_eq!(backdrop.kind, ResourceKind::Image);
    assert_eq!(backdrop.status, ResourceStatus::Supported);
    assert_eq!(backdrop.relative_path, "assets/backdrop.png");
    assert_eq!(
        backdrop.transport_path.as_ref().unwrap().as_str(),
        "assets/backdrop.png"
    );
    assert_eq!(backdrop.content_hash, Some(hash('a')));
    assert_eq!(
        backdrop.uses,
        vec![ResourceUse {
            role: UseRole::NodeImage,
            scene_id: Some("scene-a".into()),
            node_id: Some("backdrop".into()),
            pointer: "/sceneDefinitions/0/nodes/1/assetRef".into(),
        }]
    );
    assert_eq!(
        manifest.uses_of("figure")[0].pointer,
        "/sceneDefinitions/0/nodes/2/assetRef"
    );
    assert!(manifest.resource("missing").is_none());
    assert!(manifest.uses_of("missing").is_empty());
    assert!(manifest.is_transportable());
    assert!(manifest.is_complete());
    assert!(manifest.audio_mix().is_none());
}

#[test]
fn identity_is_the_asset_id_not_the_path_or_the_hash() {
    let mut document = base();
    // Same file under a new identity, and another file under the old path: ids decide.
    restate(&mut document, "figure", "assets/backdrop.png", &hash('a'));
    let manifest = manifest(document);
    assert!(manifest.resource("figure").is_some());
    assert!(manifest.resource("assets/figure.png").is_none());
    assert!(manifest.resource(&hash('b')).is_none());
    assert_eq!(
        manifest.resource("figure").unwrap().relative_path,
        "assets/backdrop.png"
    );
}

#[test]
fn lookup_keeps_the_asked_order_and_reports_unknown_ids() {
    let manifest = manifest(base());
    let lookup = manifest.lookup(&["figure", "ghost", "backdrop", "figure", "ghost"]);
    let found: Vec<_> = lookup.found.iter().map(|entry| entry.key.label()).collect();
    assert_eq!(found, ["asset:figure", "asset:backdrop"]);
    assert_eq!(lookup.unknown, ["ghost"]);
}

#[test]
fn synthetic_hashes_block_transport_with_one_issue_per_resource() {
    let manifest = manifest(fixture());
    assert!(!manifest.is_transportable());
    let blocking: Vec<_> = manifest.blocking_issues().collect();
    assert_eq!(blocking.len(), 2);
    assert!(
        blocking
            .iter()
            .all(|issue| issue.code == RESOURCE_HASH_INVALID && issue.severity == Severity::Error)
    );
    let refused = manifest.transport_plan().unwrap_err();
    assert_eq!(refused.len(), 2);
}

#[test]
fn uppercase_hashes_never_match_the_origin_so_they_are_refused() {
    let mut document = base();
    restate(
        &mut document,
        "figure",
        "assets/figure.png",
        &"B".repeat(64),
    );
    let manifest = manifest(document);
    assert_eq!(codes(&manifest), [RESOURCE_HASH_INVALID]);
}

#[test]
fn transport_plan_is_ordered_by_path_and_skips_what_is_not_supported() {
    let mut document = base();
    add_asset(
        &mut document,
        asset("spare", "assets/spare.png", &hash('c'), None, "srgb"),
    );
    let manifest = manifest(document);
    let plan = manifest.transport_plan().unwrap();
    let paths: Vec<_> = plan.items.iter().map(|item| item.path.as_str()).collect();
    assert_eq!(paths, ["assets/backdrop.png", "assets/figure.png"]);
    assert_eq!(plan.items[0].expected_hash, hash('a'));
    assert_eq!(
        plan.items[0].resources[0].key,
        ResourceKey::asset("backdrop")
    );
    assert_eq!(
        plan.items[0].resources[0].declared_path,
        "assets/backdrop.png"
    );
    assert_eq!(plan.items[0].resources[0].width_px, Some(64));
    assert_eq!(plan.skipped.len(), 1);
    assert_eq!(plan.skipped[0].key, ResourceKey::asset("spare"));
    assert_eq!(plan.skipped[0].status, ResourceStatus::Unreferenced);
    // An unreferenced asset is neither carried nor an issue.
    assert!(manifest.issues().is_empty());
    assert!(manifest.is_complete());
}

#[test]
fn one_asset_used_by_two_nodes_is_one_resource_with_two_uses() {
    let mut document = base();
    add_node(
        &mut document,
        "figure-copy",
        "imagePlane",
        Some(reference("figure", "assets/figure.png", &hash('b'))),
    );
    let manifest = manifest(document);
    assert_eq!(manifest.len(), 2);
    let uses = manifest.uses_of("figure");
    assert_eq!(uses.len(), 2);
    assert!(
        uses.iter()
            .any(|usage| usage.node_id.as_deref() == Some("figure-copy"))
    );
    assert_eq!(manifest.for_node("scene-a", "figure-copy").len(), 1);
    assert_eq!(manifest.transport_plan().unwrap().items.len(), 2);
}

#[test]
fn assets_that_declare_one_file_are_carried_once_and_keep_their_identity() {
    let mut document = base();
    add_asset(
        &mut document,
        asset("figure-twin", "assets/figure.png", &hash('b'), None, "srgb"),
    );
    add_node(
        &mut document,
        "twin-plane",
        "imagePlane",
        Some(reference("figure-twin", "assets/figure.png", &hash('b'))),
    );
    let manifest = manifest(document);
    assert!(manifest.collisions().is_empty());
    assert!(manifest.is_transportable());
    let plan = manifest.transport_plan().unwrap();
    assert_eq!(plan.items.len(), 2);
    let shared = plan
        .items
        .iter()
        .find(|item| item.path.as_str() == "assets/figure.png")
        .unwrap();
    let ids: Vec<_> = shared.resources.iter().map(|r| r.key.label()).collect();
    assert_eq!(ids, ["asset:figure", "asset:figure-twin"]);
    assert!(
        shared
            .resources
            .iter()
            .all(|r| r.declared_path == "assets/figure.png")
    );
}

#[test]
fn one_path_with_two_contents_is_a_blocking_collision() {
    let mut document = base();
    add_asset(
        &mut document,
        asset("figure-twin", "assets/figure.png", &hash('c'), None, "srgb"),
    );
    add_node(
        &mut document,
        "twin-plane",
        "imagePlane",
        Some(reference("figure-twin", "assets/figure.png", &hash('c'))),
    );
    let manifest = manifest(document);
    assert!(!manifest.is_transportable());
    assert_eq!(manifest.collisions().len(), 1);
    let collision = &manifest.collisions()[0];
    assert_eq!(collision.kind, CollisionKind::DifferentContent);
    assert_eq!(collision.path, "assets/figure.png");
    assert_eq!(
        collision.keys,
        [
            ResourceKey::asset("figure"),
            ResourceKey::asset("figure-twin")
        ]
    );
    assert!(codes(&manifest).contains(&RESOURCE_PATH_COLLISION));
    assert!(manifest.transport_plan().is_err());
}

#[test]
fn paths_that_differ_only_by_case_collide_on_case_insensitive_volumes() {
    let mut document = base();
    restate(&mut document, "figure", "Assets/Backdrop.PNG", &hash('b'));
    let manifest = manifest(document);
    assert_eq!(manifest.collisions().len(), 1);
    assert_eq!(manifest.collisions()[0].kind, CollisionKind::CaseOnly);
    assert_eq!(manifest.collisions()[0].path, "assets/backdrop.png");
    assert!(!manifest.is_transportable());
}

#[test]
fn a_resource_path_cannot_also_be_a_folder_of_another_resource() {
    let mut document = base();
    restate(
        &mut document,
        "figure",
        "assets/backdrop.png/inner.png",
        &hash('b'),
    );
    let manifest = manifest(document);
    let kinds: Vec<_> = manifest.collisions().iter().map(|c| c.kind).collect();
    assert_eq!(kinds, [CollisionKind::FileIsDirectory]);
    assert_eq!(manifest.collisions()[0].path, "assets/backdrop.png");
    assert_eq!(manifest.collisions()[0].keys.len(), 2);
    assert!(!manifest.is_transportable());
}

#[test]
fn paths_the_importer_accepts_but_a_transport_root_must_not_create_are_refused() {
    for declared in [
        "assets/fig:ure.png",
        "assets/figure.png.",
        "assets/CON.png",
        ".tessel-staging-1/figure.png",
    ] {
        let mut document = base();
        restate(&mut document, "figure", declared, &hash('b'));
        let manifest = manifest(document);
        let entry = manifest.resource("figure").unwrap();
        assert_eq!(
            entry.relative_path, declared,
            "the origin path is kept as declared"
        );
        assert!(entry.transport_path.is_none(), "{declared}");
        assert!(entry.path_error.is_some(), "{declared}");
        assert_eq!(codes(&manifest), [RESOURCE_PATH_INVALID], "{declared}");
        assert!(!manifest.is_transportable(), "{declared}");
    }
    let mut document = base();
    restate(&mut document, "figure", "assets/fig:ure.png", &hash('b'));
    let manifest = manifest(document);
    assert_eq!(
        manifest.resource("figure").unwrap().path_error,
        Some(PathError::InvalidCharacter(':'))
    );
}

#[test]
fn backslashes_normalize_for_transport_but_the_declared_path_survives() {
    let mut document = base();
    restate(
        &mut document,
        "figure",
        "assets\\sub\\.\\figure.png",
        &hash('b'),
    );
    let manifest = manifest(document);
    let entry = manifest.resource("figure").unwrap();
    assert_eq!(entry.relative_path, "assets\\sub\\.\\figure.png");
    assert_eq!(
        entry.transport_path.as_ref().unwrap().as_str(),
        "assets/sub/figure.png"
    );
    assert!(manifest.is_transportable());
}

#[test]
fn splat_nodes_are_supported_spz_resources() {
    let mut document = base();
    add_asset(
        &mut document,
        asset("cloud", "splats/cloud.spz", &hash('c'), Some("spz"), "srgb"),
    );
    add_node(
        &mut document,
        "cloud-node",
        "splat",
        Some(reference("cloud", "splats/cloud.spz", &hash('c'))),
    );
    let manifest = manifest(document);
    let cloud = manifest.resource("cloud").unwrap();
    assert_eq!(cloud.kind, ResourceKind::Splat);
    assert_eq!(cloud.status, ResourceStatus::Supported);
    assert_eq!(cloud.uses[0].role, UseRole::NodeSplat);
    assert!(manifest.is_transportable());
}

#[test]
fn a_preserved_map_that_names_a_file_of_the_wrong_format_warns() {
    let mut document = base();
    add_asset(
        &mut document,
        asset(
            "cloud",
            "splats/cloud.spz",
            &hash('c'),
            Some("spz"),
            "linear-srgb",
        ),
    );
    node(&mut document, "figure")["normalMap"] = json!({
        "assetRef": reference("cloud", "splats/cloud.spz", &hash('c')),
        "source": "luminance",
    });
    let manifest = manifest(document);
    let mismatch = manifest
        .issues()
        .iter()
        .find(|issue| issue.code == RESOURCE_FORMAT_MISMATCH)
        .unwrap();
    assert_eq!(mismatch.severity, Severity::Warning);
    assert_eq!(
        mismatch.pointer,
        "/sceneDefinitions/0/nodes/2/normalMap/assetRef"
    );
    assert!(
        manifest.is_transportable(),
        "the map is preserved, not carried"
    );
}

#[test]
fn the_importer_already_refuses_a_plane_that_names_a_splat_file() {
    let mut document = base();
    add_asset(
        &mut document,
        asset("cloud", "splats/cloud.spz", &hash('c'), Some("spz"), "srgb"),
    );
    node(&mut document, "figure")["assetRef"] = reference("cloud", "splats/cloud.spz", &hash('c'));
    assert!(DirectedScene::import_value(document).is_err());
}

#[test]
fn a_group_that_names_an_asset_is_reported_not_lost() {
    let mut document = base();
    node(&mut document, "root")["assetRef"] =
        reference("backdrop", "assets/backdrop.png", &hash('a'));
    let manifest = manifest(document);
    assert!(codes(&manifest).contains(&REFERENCE_IGNORED));
    assert_eq!(
        manifest.uses_of("backdrop").len(),
        1,
        "only the image plane counts as a use"
    );
    assert!(manifest.is_transportable(), "a warning does not block");
    assert!(!manifest.is_complete());
}

/// Two preserved data maps on the figure plane and a depth grid on a splat.
fn with_preserved_sections() -> Value {
    let mut document = base();
    add_asset(
        &mut document,
        asset(
            "figure-normal",
            "maps/figure.normal.png",
            &hash('c'),
            None,
            "linear-srgb",
        ),
    );
    add_asset(
        &mut document,
        asset(
            "figure-surface",
            "maps/figure.surface.png",
            &hash('d'),
            None,
            "linear-srgb",
        ),
    );
    add_asset(
        &mut document,
        asset("cloud", "splats/cloud.spz", &hash('e'), Some("spz"), "srgb"),
    );
    add_asset(
        &mut document,
        asset(
            "cloud-depth",
            "maps/cloud.depth.f32",
            &hash('f'),
            Some("f32"),
            "linear-srgb",
        ),
    );
    add_asset(
        &mut document,
        asset("cloud-photo", "maps/cloud.png", &hash('1'), None, "srgb"),
    );
    node(&mut document, "figure")["normalMap"] = json!({
        "assetRef": reference("figure-normal", "maps/figure.normal.png", &hash('c')),
        "source": "luminance",
    });
    node(&mut document, "figure")["surface"] = json!({
        "assetRef": reference("figure-surface", "maps/figure.surface.png", &hash('d')),
    });
    add_node(
        &mut document,
        "cloud-node",
        "splat",
        Some(reference("cloud", "splats/cloud.spz", &hash('e'))),
    );
    node(&mut document, "cloud-node")["sceneDepth"] = json!({
        "depthAssetRef": reference("cloud-depth", "maps/cloud.depth.f32", &hash('f')),
        "imageAssetRef": reference("cloud-photo", "maps/cloud.png", &hash('1')),
        "cols": 2, "rows": 2, "binPx": 1, "focalPx": 1, "pivotDepth": 1,
    });
    document
}

#[test]
fn preserved_sections_are_declared_and_never_transported() {
    let manifest = manifest(with_preserved_sections());
    for id in [
        "figure-normal",
        "figure-surface",
        "cloud-depth",
        "cloud-photo",
    ] {
        assert_eq!(
            manifest.resource(id).unwrap().status,
            ResourceStatus::Uninterpreted,
            "{id}"
        );
    }
    assert_eq!(
        manifest.resource("figure").unwrap().status,
        ResourceStatus::Supported
    );
    assert_eq!(
        manifest.resource("cloud-depth").unwrap().kind,
        ResourceKind::Grid
    );

    let sections: Vec<_> = manifest
        .uninterpreted()
        .iter()
        .map(|s| (s.name, s.node_id.clone().unwrap(), s.asset_ids.clone()))
        .collect();
    assert_eq!(sections.len(), 3);
    assert!(sections.contains(&(
        "normalMap",
        "figure".to_string(),
        vec!["figure-normal".to_string()]
    )));
    assert!(sections.contains(&(
        "surface",
        "figure".to_string(),
        vec!["figure-surface".to_string()]
    )));
    assert!(sections.contains(&(
        "sceneDepth",
        "cloud-node".to_string(),
        vec!["cloud-depth".to_string(), "cloud-photo".to_string()]
    )));

    // The supported subset is still carried; the rest is listed as skipped, never dropped.
    assert!(manifest.is_transportable());
    assert!(!manifest.is_complete());
    let plan = manifest.transport_plan().unwrap();
    let carried: Vec<_> = plan.items.iter().map(|i| i.path.as_str()).collect();
    assert_eq!(
        carried,
        [
            "assets/backdrop.png",
            "assets/figure.png",
            "splats/cloud.spz"
        ]
    );
    let skipped: Vec<_> = plan.skipped.iter().map(|s| s.key.label()).collect();
    assert_eq!(
        skipped,
        [
            "asset:cloud-depth",
            "asset:cloud-photo",
            "asset:figure-normal",
            "asset:figure-surface"
        ]
    );
    assert!(
        plan.skipped
            .iter()
            .all(|s| s.status == ResourceStatus::Uninterpreted)
    );
}

#[test]
fn closure_follows_the_dependencies_of_supported_resources() {
    let manifest = manifest(with_preserved_sections());
    let keys = |entries: Vec<&tessel_editor_core::scene_resources::ResourceEntry>| -> Vec<String> {
        entries.iter().map(|entry| entry.key.label()).collect()
    };

    assert_eq!(
        keys(manifest.closure("backdrop").unwrap()),
        ["asset:backdrop"]
    );
    assert_eq!(
        keys(manifest.closure("figure").unwrap()),
        [
            "asset:figure",
            "asset:figure-normal",
            "asset:figure-surface"
        ]
    );
    assert_eq!(
        keys(manifest.closure("cloud").unwrap()),
        ["asset:cloud", "asset:cloud-depth", "asset:cloud-photo"]
    );
    assert!(manifest.closure("ghost").is_none());

    let direct: Vec<_> = manifest
        .dependencies_of("figure")
        .iter()
        .map(|d| (d.to.label(), d.role))
        .collect();
    assert_eq!(
        direct,
        [
            ("asset:figure-normal".to_string(), UseRole::NormalMap),
            ("asset:figure-surface".to_string(), UseRole::SurfaceMap)
        ]
    );
    assert_eq!(manifest.dependencies().len(), 4);
    // A map does not need its owner.
    assert!(manifest.dependencies_of("figure-normal").is_empty());

    assert_eq!(manifest.for_scene("scene-a").len(), 7);
    assert!(manifest.for_scene("other-scene").is_empty());
    assert_eq!(
        keys(manifest.for_node("scene-a", "figure")),
        [
            "asset:figure",
            "asset:figure-normal",
            "asset:figure-surface"
        ]
    );
}

#[test]
fn dangling_and_stale_references_in_preserved_sections_warn_without_blocking() {
    let mut document = with_preserved_sections();
    node(&mut document, "figure")["normalMap"]["assetRef"]["contentHash"] = json!(hash('9'));
    node(&mut document, "figure")["relief"] = json!({
        "depthAssetRef": reference("ghost-grid", "maps/ghost.f32", &hash('8')),
        "cols": 2, "rows": 2, "planeDepth": 1, "source": "moge-3",
        "intrinsics": {"fx": 1, "fy": 1, "cx": 0.5, "cy": 0.5},
    });
    let manifest = manifest(document);
    let by_code = |code: &str| manifest.issues().iter().filter(|i| i.code == code).count();
    assert_eq!(by_code(REFERENCE_UNRESOLVED), 1);
    assert_eq!(by_code(REFERENCE_MISMATCH), 1);
    let unresolved = manifest
        .issues()
        .iter()
        .find(|i| i.code == REFERENCE_UNRESOLVED)
        .unwrap();
    assert_eq!(unresolved.severity, Severity::Warning);
    assert_eq!(
        unresolved.pointer,
        "/sceneDefinitions/0/nodes/2/relief/depthAssetRef"
    );
    assert_eq!(unresolved.key, Some(ResourceKey::asset("ghost-grid")));
    let stale = manifest
        .issues()
        .iter()
        .find(|i| i.code == REFERENCE_MISMATCH)
        .unwrap();
    assert_eq!(
        stale.pointer,
        "/sceneDefinitions/0/nodes/2/normalMap/assetRef"
    );
    assert!(stale.message.contains("contentHash"));
    // The section is still listed, even though its file is not declared anywhere.
    assert!(manifest.uninterpreted().iter().any(|s| s.name == "relief"));
    assert!(manifest.resource("ghost-grid").is_none());
    assert!(manifest.is_transportable());
    assert!(!manifest.is_complete());
}

#[test]
fn the_audio_mix_has_its_own_key_and_is_carried() {
    let mut document = base();
    document["audio"] = json!({
        "mixRelativePath": "audio/mix.wav",
        "editStartSeconds": 0,
        "editEndSecondsExclusive": 4,
        "sourcePcmHashSha256": hash('7'),
    });
    let manifest = manifest(document);
    let mix = manifest.audio_mix().unwrap();
    assert_eq!(mix.key, ResourceKey::AudioMix);
    assert_eq!(mix.asset_id(), None);
    assert_eq!(mix.kind, ResourceKind::Audio);
    assert_eq!(mix.status, ResourceStatus::Supported);
    assert_eq!(mix.content_hash, Some(hash('7')));
    assert_eq!(mix.uses[0].pointer, "/audio/mixRelativePath");
    assert!(
        manifest.resource("audio-mix").is_none(),
        "not reachable as an assetId"
    );
    let plan = manifest.transport_plan().unwrap();
    assert!(
        plan.items
            .iter()
            .any(|item| item.path.as_str() == "audio/mix.wav")
    );
    assert_eq!(plan.items.len(), 3);
}

#[test]
fn narration_takes_are_listed_as_files_that_stay_behind() {
    let mut document = base();
    document["narration"] = json!({"tracks": [{
        "id": "voice",
        "clips": [
            {"id": "c1", "audio": {"relativePath": "takes/c1.wav"}},
            {"id": "c2"},
            {"id": "c3", "audio": {"relativePath": "takes/c3.wav"}},
        ],
    }]});
    let manifest = manifest(document);
    let narration = manifest
        .uninterpreted()
        .iter()
        .find(|s| s.name == "narration")
        .unwrap();
    assert_eq!(narration.pointer, "/narration");
    let files: Vec<_> = narration
        .files
        .iter()
        .map(|f| (f.pointer.as_str(), f.relative_path.as_str()))
        .collect();
    assert_eq!(
        files,
        [
            (
                "/narration/tracks/0/clips/0/audio/relativePath",
                "takes/c1.wav"
            ),
            (
                "/narration/tracks/0/clips/2/audio/relativePath",
                "takes/c3.wav"
            ),
        ]
    );
    assert!(!narration.truncated);
    assert!(manifest.is_transportable());
    assert!(!manifest.is_complete());
    assert_eq!(
        manifest.transport_plan().unwrap().items.len(),
        2,
        "takes are not carried"
    );
}

#[test]
fn an_oversized_narration_listing_says_it_is_incomplete() {
    let mut document = base();
    let clips: Vec<Value> = (0..=MAX_LISTED_FILES)
        .map(|index| json!({"id": format!("c{index}"), "audio": {"relativePath": "t.wav"}}))
        .collect();
    document["narration"] = json!({"tracks": [{"id": "voice", "clips": clips}]});
    let manifest = manifest(document);
    let narration = manifest
        .uninterpreted()
        .iter()
        .find(|s| s.name == "narration")
        .unwrap();
    assert!(narration.truncated);
    assert_eq!(narration.files.len(), MAX_LISTED_FILES);
    assert!(codes(&manifest).contains(&LISTING_TRUNCATED));
}

#[test]
fn the_same_bundle_gives_the_same_manifest_whatever_the_asset_order() {
    let first = manifest(with_preserved_sections());
    let again = manifest(with_preserved_sections());
    assert_eq!(first, again);

    let mut shuffled = with_preserved_sections();
    shuffled["assets"].as_array_mut().unwrap().reverse();
    let reordered = manifest(shuffled);
    assert_eq!(first.to_json(), reordered.to_json());
    assert_eq!(first.to_json().to_string(), reordered.to_json().to_string());
}

#[test]
fn json_form_carries_version_identity_and_every_resource() {
    let manifest = manifest(with_preserved_sections());
    let json = manifest.to_json();
    assert_eq!(json["manifestVersion"], MANIFEST_VERSION);
    assert_eq!(json["bundleId"], "oracle-minimal");
    assert_eq!(json["transportable"], true);
    assert_eq!(json["complete"], false);
    let resources = json["resources"].as_array().unwrap();
    assert_eq!(resources.len(), 7);
    let figure = resources.iter().find(|r| r["assetId"] == "figure").unwrap();
    assert_eq!(figure["key"], "asset:figure");
    assert_eq!(figure["status"], "supported");
    assert_eq!(figure["transportPath"], "assets/figure.png");
    assert_eq!(figure["uses"][0]["role"], "nodeImage");
    assert_eq!(json["dependencies"].as_array().unwrap().len(), 4);
    assert_eq!(json["uninterpreted"].as_array().unwrap().len(), 3);
}

#[test]
fn capabilities_declare_what_is_read_and_what_is_not() {
    assert_eq!(CAPABILITIES.manifest_version, MANIFEST_VERSION);
    let kinds: Vec<_> = CAPABILITIES.supported.iter().map(|s| s.kind).collect();
    assert_eq!(
        kinds,
        [
            ResourceKind::Image,
            ResourceKind::Splat,
            ResourceKind::Audio
        ]
    );
    for section in ["surface", "normalMap", "sceneDepth", "relief", "narration"] {
        assert!(
            CAPABILITIES
                .uninterpreted
                .iter()
                .any(|text| text.contains(section)),
            "{section}"
        );
    }
    assert!(!CAPABILITIES.unscanned.is_empty());
}

#[test]
fn building_a_manifest_does_not_touch_the_scene() {
    let imported = scene(with_preserved_sections());
    let before = imported.clone();
    let first = ResourceManifest::from_scene(&imported);
    let second = ResourceManifest::from_scene(&imported);
    assert_eq!(imported, before);
    assert_eq!(first, second);
}
