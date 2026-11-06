//! F044 (S7, BL025 subset): a Gaussian splat in a directed scene, drawn by the real runtime
//! (Node worker + headless Chrome, SwiftShader + Spark) with no window.
//!
//! The oracle is independent of the renderer: the position a splat must take in the picture is
//! projected here from the plan's own camera and node matrix with a plain pinhole model, and the
//! picture is compared with the same scene *without* the splat (what changed is the splat).
//!
//! Proved: the splat lands where the plan's matrix and camera put it, at two different frames
//! (the camera moves); the same frame is the same picture however the playhead got there; a file
//! that is not an SPZ is refused before it reaches Spark; a headless export carries the same
//! splat frames as `scene.render` and verifies as an MP4.
//!
//! Not proved here: a real GPU's pixels (SwiftShader), lights, weather or rigs (refused). A splat
//! among image planes by depth (the plan's `afterLayers`) and the stored colour (BL019-F1) are
//! proved in `runtime/test/directed-scene-splats.test.mjs` and, through this same host, by the
//! independent S10 harness (`splat_render`, `splat_export`, `full_av`).
//!
//! A machine without Chrome or the runtime bundle skips these with the reason on stderr; set
//! `TESSEL_REQUIRE_RUNTIME=1` to turn the skip into a failure. Run them under the GPU lock.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tessel_editor_core::backend::FrameResult;
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::fixtures::splat_spz;
use tessel_editor_core::jobs::CancelToken;
use tessel_editor_core::scene_plan::{Capability, ScenePlan};
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::{CommandError, EditorState, headless};
use tessel_editor_media::fsx::hash_file;
use tessel_editor_media::host::MediaHost;
use tessel_editor_media::tools::Tools;
use tessel_editor_render::scene::import_gated;
use tessel_editor_render::scene_host::SceneHost;
use tessel_editor_render::{
    FrameOptions, HANDSHAKE_TIMEOUT, ProcessTransport, RuntimeConfig, SceneRenderer,
    SceneResourceMap,
};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
/// Grid step of the pixel survey: a changed region is found to within this many pixels.
const STEP: u32 = 4;
/// A channel moving more than this between the two pictures is the splat, not codec noise.
const CHANGED: i32 = 24;
/// Where the cloud sits in scene space: right of every plane of the corpus, in front of the camera.
const CLOUD_AT: [f64; 3] = [3.0, 1.2, -1.0];

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/directed-scene-proof/corpus")
}

fn skip(reason: &str) {
    assert!(
        std::env::var("TESSEL_REQUIRE_RUNTIME").as_deref() != Ok("1"),
        "the runtime is required but unavailable: {reason}"
    );
    eprintln!("SKIPPED (runtime unavailable): {reason}");
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("te-s7-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn sha256(path: &Path) -> String {
    hash_file(path, &CancelToken::new(), |_| {}).unwrap().0
}

/// The corpus scene without its full-frame sky (an opaque backdrop would cover everything
/// behind it), with or without a splat. Writes every file under `root`; returns the bundle path.
fn write_bundle(root: &Path, with_splat: bool) -> PathBuf {
    let mut bundle: Value =
        serde_json::from_str(&fs::read_to_string(corpus().join("bundle.json")).unwrap()).unwrap();
    for name in ["hero", "mid"] {
        let target = root.join(format!("assets/{name}.png"));
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(corpus().join(format!("assets/{name}.png")), target).unwrap();
    }
    bundle["assets"]
        .as_array_mut()
        .unwrap()
        .retain(|asset| asset["assetId"] != "sky");
    let nodes = bundle["sceneDefinitions"][0]["nodes"]
        .as_array_mut()
        .unwrap();
    nodes.retain(|node| node["id"] != "sky");
    if with_splat {
        let file = root.join("splats/cloud.spz");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, splat_spz(1500)).unwrap();
        let hash = sha256(&file);
        bundle["assets"].as_array_mut().unwrap().push(json!({
            "assetId": "cloud", "revisionId": "rev-1", "contentHash": hash,
            "relativePath": "splats/cloud.spz", "format": "spz",
            "widthPx": 1, "heightPx": 1, "alphaMode": "opaque", "colorSpace": "srgb",
        }));
        bundle["sceneDefinitions"][0]["nodes"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "id": "cloud-node", "parentId": "root", "type": "splat",
                "assetRef": {
                    "assetId": "cloud", "revisionId": "rev-1", "contentHash": hash,
                    "relativePath": "splats/cloud.spz",
                },
                "localPosition": { "x": CLOUD_AT[0], "y": CLOUD_AT[1], "z": CLOUD_AT[2] },
                "localRotation": { "x": 0, "y": 0, "z": 0, "w": 1 },
                "localScale": { "x": 1, "y": 1, "z": 1 },
            }));
    }
    let path = root.join("bundle.json");
    fs::write(&path, serde_json::to_vec_pretty(&bundle).unwrap()).unwrap();
    path
}

fn scene_at(path: &Path) -> DirectedScene {
    import_gated(&fs::read_to_string(path).unwrap()).unwrap()
}

fn renderer() -> Option<SceneRenderer<ProcessTransport>> {
    let config = match RuntimeConfig::discover() {
        Ok(config) => config,
        Err(setup) => {
            skip(&setup.message());
            return None;
        }
    };
    let backend = tessel_editor_core::bridge::BridgeBackend::new(
        "three-worker",
        ProcessTransport::new(config),
        HANDSHAKE_TIMEOUT,
    );
    let mut renderer = SceneRenderer::new(backend);
    match renderer.ensure_ready() {
        Ok(_) => Some(renderer),
        Err(error) if error.code == "backend_unavailable" && !error.message.contains("Rebuild") => {
            skip(&error.message);
            None
        }
        Err(error) => panic!("{error:?}"),
    }
}

fn grid() -> Vec<(u32, u32)> {
    (0..HEIGHT / STEP)
        .flat_map(|row| (0..WIDTH / STEP).map(move |column| (column * STEP, row * STEP)))
        .collect()
}

fn frame(
    renderer: &mut SceneRenderer<ProcessTransport>,
    root: &Path,
    scene: &DirectedScene,
    edit_frame: i64,
    generation: u64,
) -> (ScenePlan, FrameResult) {
    let plan = ScenePlan::build(scene, edit_frame).unwrap();
    let map = SceneResourceMap::from_root(generation, &plan, root).unwrap();
    let result = renderer
        .render(
            &plan,
            &map,
            &FrameOptions {
                probes: grid(),
                ..FrameOptions::default()
            },
        )
        .unwrap_or_else(|error| panic!("frame {edit_frame}: {error:?}"));
    (plan, result)
}

/// Pixel of a scene-space point under the plan's camera: a pinhole looking from `position` at
/// `target`, +Y up, vertical `fov_deg`, on the plan's output aspect. Returns `(x, y, scale)`
/// where `scale` is the pixels one scene unit covers at the point's depth.
fn project(plan: &ScenePlan, point: [f64; 3]) -> (f64, f64, f64) {
    let camera = &plan.camera;
    let eye = [camera.position.x, camera.position.y, camera.position.z];
    let aim = [camera.target.x, camera.target.y, camera.target.z];
    let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let cross = |a: [f64; 3], b: [f64; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let unit = |a: [f64; 3]| {
        let length = dot(a, a).sqrt();
        [a[0] / length, a[1] / length, a[2] / length]
    };
    let forward = unit(sub(aim, eye));
    let right = unit(cross(forward, [0.0, 1.0, 0.0]));
    let up = cross(right, forward);
    let offset = sub(point, eye);
    let depth = dot(offset, forward);
    let half_height = (camera.fov_deg.to_radians() / 2.0).tan() * depth;
    let half_width = half_height * f64::from(WIDTH) / f64::from(HEIGHT);
    let x = 0.5 + dot(offset, right) / (2.0 * half_width);
    let y = 0.5 - dot(offset, up) / (2.0 * half_height);
    (
        x * f64::from(WIDTH),
        y * f64::from(HEIGHT),
        f64::from(HEIGHT) / (2.0 * half_height),
    )
}

/// Grid points whose picture changed between `without` and `with`, as `(x, y)` pixels.
fn changed(without: &FrameResult, with: &FrameResult) -> Vec<(f64, f64)> {
    grid()
        .into_iter()
        .zip(without.probes.iter().zip(&with.probes))
        .filter(|(_, (a, b))| (0..3).any(|c| (i32::from(a[c]) - i32::from(b[c])).abs() > CHANGED))
        .map(|((x, y), _)| (f64::from(x), f64::from(y)))
        .collect()
}

#[test]
fn a_splat_lands_where_the_plan_matrix_and_camera_put_it_at_two_frames() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let with_root = scratch("with");
    let plain_root = scratch("plain");
    let with = scene_at(&write_bundle(&with_root, true));
    let plain = scene_at(&write_bundle(&plain_root, false));

    // Frame 0 and 24: the camera dollies in, so the same node moves and grows in the picture.
    let mut centers = Vec::new();
    for edit_frame in [0, 24] {
        let (plan, drawn) = frame(&mut renderer, &with_root, &with, edit_frame, 1);
        let (plain_plan, undrawn) = frame(&mut renderer, &plain_root, &plain, edit_frame, 1);
        assert!(plan.requires.contains(&Capability::Splats));
        assert!(plain_plan.splats.is_empty());
        assert_ne!(
            drawn.hash, undrawn.hash,
            "frame {edit_frame}: the splat is drawn"
        );

        let node = plan.node("cloud-node").unwrap();
        let at = &node.world_matrix.0[12..15];
        assert_eq!(at, &CLOUD_AT[..], "the matrix is the evaluator's own");
        let (px, py, scale) = project(&plan, CLOUD_AT);
        let points = changed(&undrawn, &drawn);
        assert!(
            points.len() > 8,
            "frame {edit_frame}: too few changed points ({})",
            points.len()
        );
        let min_x = points.iter().map(|p| p.0).fold(f64::MAX, f64::min);
        let max_x = points.iter().map(|p| p.0).fold(f64::MIN, f64::max);
        let min_y = points.iter().map(|p| p.1).fold(f64::MAX, f64::min);
        let max_y = points.iter().map(|p| p.1).fold(f64::MIN, f64::max);
        let (cx, cy) = ((min_x + max_x) / 2.0, (min_y + max_y) / 2.0);
        let half = ((max_x - min_x) + (max_y - min_y)) / 4.0;
        eprintln!(
            "frame {edit_frame}: oracle ({px:.1}, {py:.1}) px, {scale:.1} px/unit; drawn centre ({cx:.1}, {cy:.1}), half-extent {half:.1}"
        );
        // The unit-radius cloud: centre within a few grid steps, extent within a factor of two.
        assert!((cx - px).abs() <= 3.0 * f64::from(STEP), "x: {cx} vs {px}");
        assert!((cy - py).abs() <= 3.0 * f64::from(STEP), "y: {cy} vs {py}");
        assert!(
            half > 0.5 * scale && half < 2.0 * scale,
            "extent {half} vs {scale} px/unit"
        );
        centers.push((cx, scale));
    }
    assert!(
        centers[1].0 != centers[0].0 && centers[1].1 > centers[0].1,
        "the camera moved: the splat moved and grew ({centers:?})"
    );
    renderer.release_all().unwrap();
    renderer.shutdown();
    let _ = fs::remove_dir_all(with_root);
    let _ = fs::remove_dir_all(plain_root);
}

#[test]
fn the_same_frame_is_the_same_picture_however_the_playhead_got_there() {
    let (Some(mut warm), Some(mut cold)) = (renderer(), renderer()) else {
        return;
    };
    let root = scratch("seek");
    let scene = scene_at(&write_bundle(&root, true));
    let mut hashes = std::collections::BTreeMap::new();
    for edit_frame in [24, 0, 60, 24, 0, 60, 47] {
        let (_, result) = frame(&mut warm, &root, &scene, edit_frame, 1);
        let previous = hashes.insert(edit_frame, result.hash.clone());
        assert!(
            previous.is_none_or(|hash| hash == result.hash),
            "frame {edit_frame} changed between visits"
        );
    }
    let (_, fresh) = frame(&mut cold, &root, &scene, 24, 1);
    assert_eq!(
        fresh.hash, hashes[&24],
        "a cold renderer draws what a warm one does"
    );
    assert!(
        hashes
            .values()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            >= 3
    );
    warm.shutdown();
    cold.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_file_that_is_not_an_spz_is_refused_before_spark_sees_it_and_the_renderer_keeps_drawing() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let root = scratch("not-spz");
    let scene = scene_at(&write_bundle(&root, true));
    let plan = ScenePlan::build(&scene, 0).unwrap();
    // The bytes of a picture where the plan promised a splat.
    fs::copy(
        corpus().join("assets/hero.png"),
        root.join("splats/cloud.spz"),
    )
    .unwrap();
    let map = SceneResourceMap::from_root(1, &plan, &root).unwrap();
    let error = renderer
        .render(&plan, &map, &FrameOptions::default())
        .unwrap_err();
    assert_eq!(error.code, "resource_mismatch", "{error:?}");
    // Nothing was loaded for it: the next honest frame draws.
    fs::write(root.join("splats/cloud.spz"), splat_spz(1500)).unwrap();
    let (_, drawn) = frame(&mut renderer, &root, &scene, 0, 1);
    assert_eq!((drawn.width, drawn.height), (WIDTH, HEIGHT));
    renderer.shutdown();
    let _ = fs::remove_dir_all(root);
}

// ------------------------------------------------------------------------ export --

struct Session {
    state: EditorState,
    host: SceneHost,
    work: PathBuf,
}

impl Session {
    fn run(&mut self, request: Value) -> Result<Value, CommandError> {
        headless::execute_ext(&mut self.state, None, Some(&mut self.host), &request)
    }

    fn ok(&mut self, request: Value) -> Value {
        self.run(request.clone())
            .unwrap_or_else(|error| panic!("{request}: {error:?}"))
    }

    fn hash(&mut self, scene: &str, frame: i64) -> String {
        self.ok(json!({"op":"scene.render","scene":scene,"frame":frame,
            "width":WIDTH,"height":HEIGHT}))["frame"]["hash"]
            .as_str()
            .unwrap()
            .to_string()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.work);
    }
}

#[test]
fn an_export_carries_the_same_splat_frames_as_scene_render_and_verifies_as_an_mp4() {
    let config = match RuntimeConfig::discover() {
        Ok(config) => config,
        Err(setup) => return skip(&setup.message()),
    };
    if let Err(error) = Tools::discover() {
        return skip(&error.message);
    }
    let work = scratch("export");
    let with_root = work.join("with");
    let plain_root = work.join("plain");
    let with_bundle = write_bundle(&with_root, true);
    let plain_bundle = write_bundle(&plain_root, false);
    let mut session = Session {
        state: EditorState::empty(&ProjectSettings::default()).unwrap(),
        host: SceneHost::new(MediaHost::new(), config),
        work,
    };
    session.ok(json!({"op":"scene.attach","id":"cloud","path":with_bundle}));
    session.ok(json!({"op":"scene.resources.resolve","scene":"cloud","base":with_root}));
    session.ok(json!({"op":"scene.attach","id":"plain","path":plain_bundle}));
    session.ok(json!({"op":"scene.resources.resolve","scene":"plain","base":plain_root}));

    let destination = session.work.join("cloud.mp4");
    let preset = json!({"id":"s7","name":"S7","width":WIDTH,"height":HEIGHT,"fps":24});
    let result = session.ok(json!({"op":"scene.export","scene":"cloud","preset":preset,
        "from":20,"frames":4,"destination":destination}));
    assert_eq!(result["frames"], 4);
    assert_eq!(result["verification"]["ok"], true, "{result}");
    assert!(destination.is_file());

    // The report names the file the frames came from, by the hash the bundle declares.
    let declared = sha256(&with_root.join("splats/cloud.spz"));
    let cloud = result["resources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|resource| resource["asset"] == "cloud")
        .unwrap_or_else(|| panic!("the export lists the splat: {}", result["resources"]));
    assert_eq!(cloud["sha256"], declared);

    // Frame by frame the export holds what scene.render draws, and it is not the splat-less picture.
    let listed: Vec<String> = result["frame_hashes"]["list"]
        .as_array()
        .expect("a short export lists every frame hash")
        .iter()
        .map(|hash| hash.as_str().unwrap().to_string())
        .collect();
    assert_eq!(listed.len(), 4);
    for (index, hash) in listed.iter().enumerate() {
        let edit_frame = 20 + index as i64;
        assert_eq!(
            hash,
            &session.hash("cloud", edit_frame),
            "edit frame {edit_frame}"
        );
        assert_ne!(
            hash,
            &session.hash("plain", edit_frame),
            "edit frame {edit_frame}"
        );
    }
}
