//! Shared fixtures of the `directed_resources*` tests: a throwaway source and transport root,
//! tiny procedural PNG/SPZ files (no private images) and the oracle's synthetic bundle restated
//! with the real SHA-256 of those files.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::fixtures::{encode_png, splat_spz};
use tessel_editor_core::jobs::JobError;
use tessel_editor_core::scene_resources::ResourceManifest;
use tessel_editor_media::fsx::hex;

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// `src/` holds the bundle's files, `dst/` is an empty transport root, `outside/` is a folder
/// next to both that no resource may ever reach.
pub struct Sandbox {
    pub root: PathBuf,
    pub src: PathBuf,
    pub dst: PathBuf,
    pub outside: PathBuf,
    locked: Vec<PathBuf>,
}

impl Sandbox {
    pub fn new(name: &str) -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!(
            "tessel-dres-{name}-{}-{unique}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let sandbox = Self {
            src: root.join("src"),
            dst: root.join("dst"),
            outside: root.join("outside"),
            root,
            locked: Vec::new(),
        };
        for dir in [&sandbox.src, &sandbox.dst, &sandbox.outside] {
            fs::create_dir_all(dir).unwrap();
        }
        sandbox
    }

    /// Writes `bytes` at `relative` under the source root and returns its SHA-256.
    pub fn source_file(&self, relative: &str, bytes: &[u8]) -> String {
        write(&self.src, relative, bytes)
    }

    /// Makes a directory read-only; `Drop` restores it so the sandbox can be removed.
    #[cfg(unix)]
    pub fn lock(&mut self, dir: &Path) {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o555)).unwrap();
        self.locked.push(dir.to_path_buf());
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        #[cfg(unix)]
        for dir in &self.locked {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o755));
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

pub fn write(root: &Path, relative: &str, bytes: &[u8]) -> String {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
    sha256(bytes)
}

pub fn png(width: u32, height: u32, shade: u8) -> Vec<u8> {
    let pixels = vec![shade; usize::try_from(width * height * 4).unwrap()];
    encode_png(width, height, &pixels)
}

pub fn spz() -> Vec<u8> {
    splat_spz(8)
}

pub fn fixture() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json");
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn nodes(document: &mut Value) -> &mut Vec<Value> {
    document["sceneDefinitions"][0]["nodes"]
        .as_array_mut()
        .unwrap()
}

pub fn node<'a>(document: &'a mut Value, id: &str) -> &'a mut Value {
    nodes(document)
        .iter_mut()
        .find(|node| node["id"] == id)
        .unwrap()
}

pub fn reference(asset_id: &str, path: &str, hash: &str) -> Value {
    json!({
        "assetId": asset_id,
        "revisionId": "rev-1",
        "contentHash": hash,
        "relativePath": path,
    })
}

/// Points an asset, and the nodes that name it, at a path and hash (the importer requires the
/// two copies to agree). Optionally sets the declared size.
pub fn restate(document: &mut Value, id: &str, path: &str, hash: &str) {
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

pub fn declare_size(document: &mut Value, id: &str, width: u32, height: u32) {
    for record in document["assets"].as_array_mut().unwrap() {
        if record["assetId"] == id {
            record["widthPx"] = json!(width);
            record["heightPx"] = json!(height);
        }
    }
}

pub fn add_asset(
    document: &mut Value,
    id: &str,
    path: &str,
    hash: &str,
    format: Option<&str>,
    color_space: &str,
) {
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
    document["assets"].as_array_mut().unwrap().push(record);
}

pub fn add_node(document: &mut Value, id: &str, kind: &str, asset: Value) {
    let mut record = json!({
        "id": id,
        "parentId": "root",
        "type": kind,
        "assetRef": asset,
        "localPosition": {"x": 0, "y": 0, "z": 0},
        "localRotation": {"x": 0, "y": 0, "z": 0, "w": 1},
        "localScale": {"x": 1, "y": 1, "z": 1},
    });
    if kind == "imagePlane" {
        record["widthSceneUnits"] = json!(1);
        record["heightSceneUnits"] = json!(1);
    }
    nodes(document).push(record);
}

/// Writes `assets/backdrop.png` (64x36) and `assets/figure.png` (32x32) under the source root
/// and returns the bundle that declares them with their real hashes and sizes.
pub fn standard_bundle(sandbox: &Sandbox) -> Value {
    let mut document = fixture();
    let backdrop = sandbox.source_file("assets/backdrop.png", &png(64, 36, 10));
    let figure = sandbox.source_file("assets/figure.png", &png(32, 32, 20));
    restate(&mut document, "backdrop", "assets/backdrop.png", &backdrop);
    restate(&mut document, "figure", "assets/figure.png", &figure);
    document
}

pub fn scene(document: Value) -> DirectedScene {
    DirectedScene::import_value(document)
        .unwrap_or_else(|error| panic!("the test bundle must import: {error:?}"))
}

pub fn manifest(document: Value) -> ResourceManifest {
    ResourceManifest::from_scene(&scene(document))
}

/// Every file and folder under `dir`, relative and sorted: what a transport root holds.
pub fn tree(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let relative = path.strip_prefix(base).unwrap().display().to_string();
            out.push(relative);
            if path.is_dir() && !path.is_symlink() {
                walk(base, &path, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// The `code` of every problem a `resources_invalid` error carries.
pub fn problem_codes(error: &JobError) -> Vec<String> {
    error.details.as_ref().unwrap()["problems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|problem| problem["code"].as_str().unwrap().to_string())
        .collect()
}
