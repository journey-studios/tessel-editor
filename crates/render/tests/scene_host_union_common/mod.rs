//! Shared fixture of the S2-H tests (BL082.4.4, slice H): a two-deck plan whose decks do NOT draw
//! the same files, real tiny PNG/SPZ files with real SHA-256, and a media session that really
//! approved them.
//!
//! The scene is the repository's public `valid-cut-instance-offsets` bundle (`shot-a` `[0, 40)`,
//! `shot-b` `[40, 100)`, scenes `scene-a`/`scene-b`, both with `backdrop` and `figure` under the
//! SAME node ids) restated with the real hashes of the generated files, plus a transition between
//! the shots and, optionally, `sky` (a node of `scene-b` only) and `cloud` (an SPZ splat read by a
//! node called `cloud-node` in each scene named by the caller).
//!
//! The transition is read through the hidden admitting importer, **only here, in tests**: the
//! public importer still refuses a transition (`FX_TRANSITION_NOT_YET`), and no `src` file of any
//! crate names the admission. Frames 36 and 42 sit in the 12-frame window centred on the cut at
//! 40: at 36 the primary deck is `scene-a` and `scene-b` is the neighbour, at 42 the roles swap.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::fx::TransitionAdmission;
use tessel_editor_core::directed_scene::{DirectedScene, Limits};
use tessel_editor_core::fixtures::{encode_png, splat_spz};
use tessel_editor_core::jobs::CancelToken;
use tessel_editor_core::scene_plan::ScenePlan;
use tessel_editor_core::scene_resources::ResourceKey;
use tessel_editor_media::directed_resources::Options;
use tessel_editor_media::fsx::hash_file;
use tessel_editor_media::scene_resources::{
    ApprovedResource, Generation, LoadRequest, ResolveError, ResourceBase, ResourceResolver,
    SceneResourceSession, load,
};

/// Inside the window, before the cut: primary `scene-a`, neighbour `scene-b`.
pub const BEFORE_CUT: i64 = 36;
/// Inside the window, after the cut: primary `scene-b`, neighbour `scene-a`.
pub const AFTER_CUT: i64 = 42;
/// Outside every window: a plan with no transition.
pub const PLAIN: i64 = 10;
/// The id of the scene as the PROJECT attached it. Deliberately not `scene-a`, `scene-b`, `inst-a`
/// or `inst-b`: the approval key is this, never a source id.
pub const PROJECT_SCENE: &str = "street";
pub const PROJECT: &str = "project-s2h";

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A throwaway folder of this test, removed on drop; nothing outside it is touched.
pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("tessel-s2h-{name}-{}-{unique}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// What the bundle of a test contains beyond `backdrop` and `figure`.
#[derive(Clone, Default)]
pub struct Content {
    /// `sky`, an 8x8 PNG read by `sky-plane` in `scene-b` only.
    pub sky: bool,
    /// `cloud`, an SPZ of this many splats read by `cloud-node` in each scene listed.
    pub cloud: Option<(usize, Vec<&'static str>)>,
}

impl Content {
    pub fn sky() -> Self {
        Self {
            sky: true,
            cloud: None,
        }
    }

    pub fn with_cloud(mut self, count: usize, scenes: &[&'static str]) -> Self {
        self.cloud = Some((count, scenes.to_vec()));
        self
    }
}

pub struct Fixture {
    /// The folder the bundle's files live in (an explicit, absolute base).
    pub root: PathBuf,
    pub scene: DirectedScene,
    _scratch: Scratch,
}

fn bundle_text() -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/migration/fixtures/bundles/valid-cut-instance-offsets.bundle.json"),
    )
    .unwrap()
}

pub fn write(root: &Path, relative: &str, bytes: &[u8]) -> String {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, bytes).unwrap();
    hash_of(&path)
}

pub fn hash_of(path: &Path) -> String {
    hash_file(path, &CancelToken::new(), |_| {}).unwrap().0
}

fn restate(bundle: &mut Value, id: &str, path: &str, hash: &str) {
    for asset in bundle["assets"].as_array_mut().unwrap() {
        if asset["assetId"] == id {
            asset["relativePath"] = json!(path);
            asset["contentHash"] = json!(hash);
        }
    }
    for definition in bundle["sceneDefinitions"].as_array_mut().unwrap() {
        for node in definition["nodes"].as_array_mut().unwrap() {
            if node["assetRef"]["assetId"] == id {
                node["assetRef"]["relativePath"] = json!(path);
                node["assetRef"]["contentHash"] = json!(hash);
            }
        }
    }
}

fn add_asset(bundle: &mut Value, id: &str, path: &str, hash: &str, spz: bool) {
    let mut asset = json!({
        "assetId": id, "revisionId": "rev-1", "contentHash": hash, "relativePath": path,
        "widthPx": if spz { 1 } else { 8 }, "heightPx": if spz { 1 } else { 8 },
        "alphaMode": "opaque", "colorSpace": "srgb",
    });
    if spz {
        asset["format"] = json!("spz");
    }
    bundle["assets"].as_array_mut().unwrap().push(asset);
}

fn add_node(
    bundle: &mut Value,
    scene: &str,
    id: &str,
    kind: &str,
    asset: &str,
    path: &str,
    hash: &str,
) {
    let mut node = json!({
        "id": id, "parentId": "root", "type": kind,
        "assetRef": {"assetId": asset, "revisionId": "rev-1", "contentHash": hash, "relativePath": path},
        "localPosition": {"x": 0, "y": 0, "z": 0},
        "localRotation": {"x": 0, "y": 0, "z": 0, "w": 1},
        "localScale": {"x": 1, "y": 1, "z": 1},
    });
    if kind == "imagePlane" {
        node["widthSceneUnits"] = json!(1);
        node["heightSceneUnits"] = json!(1);
    }
    bundle["sceneDefinitions"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|definition| definition["id"] == scene)
        .unwrap()["nodes"]
        .as_array_mut()
        .unwrap()
        .push(node);
}

/// `shot-a` into `shot-b`, 12 frames, centred on the cut at 40.
pub fn transition() -> Value {
    json!({
        "id": "tr", "effect": "crossDissolve", "fromShotId": "shot-a", "toShotId": "shot-b",
        "durationFrames": 12, "alignment": "center", "handles": "hold", "easing": "easeInOutCubic",
    })
}

pub fn admit(bundle: Value) -> DirectedScene {
    DirectedScene::import_value_admitting(bundle, &Limits::default(), TransitionAdmission::Accept)
        .unwrap_or_else(|error| panic!("the fixture was refused: {:?}", error.issues()))
}

/// The bundle JSON (transition included) and the files it names, written under `scratch`.
pub fn bundle_and_files(root: &Path, options: &Content) -> Value {
    let mut bundle: Value = serde_json::from_str(&bundle_text()).unwrap();
    let backdrop = write(
        root,
        "assets/backdrop.png",
        &encode_png(64, 36, &vec![10; 64 * 36 * 4]),
    );
    let figure = write(
        root,
        "assets/figure.png",
        &encode_png(32, 32, &vec![20; 32 * 32 * 4]),
    );
    restate(&mut bundle, "backdrop", "assets/backdrop.png", &backdrop);
    restate(&mut bundle, "figure", "assets/figure.png", &figure);
    if options.sky {
        let sky = write(
            root,
            "neighbour/sky.png",
            &encode_png(8, 8, &vec![30; 8 * 8 * 4]),
        );
        add_asset(&mut bundle, "sky", "neighbour/sky.png", &sky, false);
        add_node(
            &mut bundle,
            "scene-b",
            "sky-plane",
            "imagePlane",
            "sky",
            "neighbour/sky.png",
            &sky,
        );
    }
    if let Some((count, scenes)) = &options.cloud {
        let cloud = write(root, "splats/cloud.spz", &splat_spz(*count));
        add_asset(&mut bundle, "cloud", "splats/cloud.spz", &cloud, true);
        for scene in scenes {
            add_node(
                &mut bundle,
                scene,
                "cloud-node",
                "splat",
                "cloud",
                "splats/cloud.spz",
                &cloud,
            );
        }
    }
    bundle["fx"] = json!({"catalogVersion": 1, "tracks": [], "transitions": [transition()]});
    bundle
}

impl Fixture {
    pub fn new(name: &str, options: &Content) -> Self {
        let scratch = Scratch::new(name);
        let root = scratch.0.join("src");
        fs::create_dir_all(&root).unwrap();
        let scene = admit(bundle_and_files(&root, options));
        Self {
            root,
            scene,
            _scratch: scratch,
        }
    }

    pub fn plan(&self, frame: i64) -> ScenePlan {
        ScenePlan::build(&self.scene, frame)
            .unwrap_or_else(|error| panic!("plan at frame {frame}: {error:?}"))
    }

    pub fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    /// A media session that REALLY approved this bundle's files (hash, header, containment) for
    /// the project scene `PROJECT_SCENE`.
    pub fn approve(&self) -> SceneResourceSession {
        approve(&self.scene, &self.root)
    }
}

pub fn approve(scene: &DirectedScene, root: &Path) -> SceneResourceSession {
    let session = SceneResourceSession::default();
    session.observe_project(PROJECT);
    let base = ResourceBase::explicit(root).unwrap();
    let map = load(
        &LoadRequest {
            scene,
            scene_id: PROJECT_SCENE,
            project_id: PROJECT,
            generation: session.generation(),
            base: &base,
        },
        &Options::default(),
        &CancelToken::new(),
        &mut |_| {},
    )
    .unwrap_or_else(|error| panic!("media did not approve the fixture: {error:?}"));
    session.install(map).unwrap();
    session
}

/// A resolver that records every question and answers from a real session (or another resolver).
pub struct Spy<'a> {
    pub inner: &'a dyn ResourceResolver,
    pub calls: Mutex<Vec<(String, String)>>,
}

impl<'a> Spy<'a> {
    pub fn new(inner: &'a dyn ResourceResolver) -> Self {
        Self {
            inner,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn calls(&self) -> Vec<(String, String)> {
        self.calls.lock().unwrap().clone()
    }
}

impl ResourceResolver for Spy<'_> {
    fn resolve(&self, scene_id: &str, key: &ResourceKey) -> Result<ApprovedResource, ResolveError> {
        self.calls
            .lock()
            .unwrap()
            .push((scene_id.to_string(), key.label()));
        self.inner.resolve(scene_id, key)
    }
}

/// The resolver of a session that holds no map: every question would be `scene_resources_not_loaded`.
pub fn empty_session() -> SceneResourceSession {
    let session = SceneResourceSession::default();
    session.observe_project(PROJECT);
    session
}

pub fn generation(session: &SceneResourceSession) -> Generation {
    session.generation()
}

/// `details.resource` of an error, as the contract writes it.
pub fn context(asset: &str, uses: &[(&str, &str)]) -> Value {
    json!({
        "asset_id": asset,
        "used_by": uses.iter().map(|(deck, node)| json!({"deck": deck, "node_id": node})).collect::<Vec<_>>(),
    })
}
