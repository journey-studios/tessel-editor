//! Shared fixtures of the BL064 plan-performance suites (`scene_plan_performance*`): the synthetic
//! scale scenes and the loader of the 55 + 47 oracle corpus. Included with `#[path]` by each test
//! binary, so a helper one of them does not use is expected (`dead_code` is allowed here only).
//!
//! WHY synthetic and generated: the cost of a plan depends on the *shape* of the scene (nodes,
//! channels, camera segments, edit length), not on its pixels, and no private media may enter the
//! repository. Every scene below is plain JSON built from a `Shape`; assets are declared by
//! metadata only (nothing here opens an image).
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::DirectedScene;

/// The knobs that move the cost of one frame plan. Names are what the benchmark report prints.
#[derive(Debug, Clone, Copy)]
pub struct Shape {
    pub name: &'static str,
    /// Image planes (the nodes a renderer draws), each with its own `assetRef`.
    pub planes: usize,
    /// Group nodes between the root and the planes (the hierarchy the matrices go through).
    pub groups: usize,
    /// Performance channels: cycles on `localPosition`, then on `localRotation`, one per
    /// (plane, property), so no two channels write the same property.
    pub channels: usize,
    /// Edit length, one instance of the same length and `shots` shots tiling it.
    pub edit_frames: u64,
    /// Movements of the main camera track (orbit / dolly alternating).
    pub camera_segments: usize,
    /// Camera layers stacked over the main track (0 or 1 here).
    pub camera_layers: usize,
    pub shots: usize,
}

/// Distinct assets shared among the planes: resources are deduplicated by `assetId`, so the plan
/// carries `min(planes, ASSET_POOL)` of them and `used_by` grows with the planes.
pub const ASSET_POOL: usize = 16;

fn vec3(x: f64, y: f64, z: f64) -> Value {
    json!({ "x": x, "y": y, "z": z })
}

fn identity_rotation() -> Value {
    json!({ "x": 0, "y": 0, "z": 0, "w": 1 })
}

fn asset_ref(index: usize) -> Value {
    json!({
        "assetId": format!("asset-{index}"),
        "revisionId": format!("asset-{index}-r1"),
        "contentHash": format!("hash-{index}"),
        "relativePath": format!("layers/asset-{index}.png"),
    })
}

fn render_profile(fps: &Value) -> Value {
    json!({
        "width": 1920,
        "height": 1080,
        "fps": fps,
        "colorSpace": "srgb-display",
        "alphaMode": "straight",
        "premultiplyOnUpload": false,
        "videoCodec": "h264-yuv420p",
        "threeRevision": "0.187.0",
        "browserChannel": "chrome",
    })
}

fn nodes(shape: &Shape) -> Vec<Value> {
    let mut nodes = vec![json!({
        "id": "root",
        "parentId": null,
        "type": "group",
        "localPosition": vec3(0.0, 0.0, 0.0),
        "localRotation": identity_rotation(),
        "localScale": vec3(1.0, 1.0, 1.0),
    })];
    for group in 0..shape.groups {
        nodes.push(json!({
            "id": format!("g{group}"),
            "parentId": "root",
            "type": "group",
            "localPosition": vec3(group as f64 * 0.5, 0.0, 0.0),
            "localRotation": identity_rotation(),
            "localScale": vec3(1.0, 1.0, 1.0),
        }));
    }
    for plane in 0..shape.planes {
        let parent = if shape.groups == 0 {
            "root".to_string()
        } else {
            format!("g{}", plane % shape.groups)
        };
        nodes.push(json!({
            "id": format!("p{plane}"),
            "parentId": parent,
            "type": "imagePlane",
            "assetRef": asset_ref(plane % ASSET_POOL),
            "localPosition": vec3(plane as f64 * 0.01, 0.0, -(plane as f64) * 0.001),
            "localRotation": identity_rotation(),
            "localScale": vec3(1.0, 1.0, 1.0),
            "widthSceneUnits": 1.0,
            "heightSceneUnits": 1.5,
            // Distinct, non-monotonic render orders keep the paint-order sort honest.
            "renderOrder": ((plane * 7) % 13) as i64,
        }));
    }
    nodes
}

fn channels(shape: &Shape) -> Vec<Value> {
    (0..shape.channels)
        .map(|index| {
            let plane = index % shape.planes.max(1);
            let property = if index < shape.planes {
                "localPosition"
            } else {
                "localRotation"
            };
            json!({
                "kind": "cycle",
                "channelId": format!("c{index}"),
                "nodeId": format!("p{plane}"),
                "property": property,
                "periodFrames": 12 + (index % 7) as u64,
                "amplitude": 0.05,
                "axis": vec3(0.0, 1.0, 0.0),
                "phaseOffsetFrames": (index % 12) as u64,
            })
        })
        .collect()
}

fn camera_segment(index: usize, frames: u64) -> Value {
    let from = if index == 0 { "initial" } else { "previousEnd" };
    if index.is_multiple_of(2) {
        return json!({
            "recipe": "orbit",
            "durationFrames": frames,
            "from": from,
            "yawDeg": 3.0,
            "easing": "linear",
        });
    }
    json!({
        "recipe": "dollyIn",
        "durationFrames": frames,
        "from": from,
        "distanceSceneUnits": 0.001,
        "easing": "easeInOutCubic",
    })
}

fn camera_clip(shape: &Shape) -> Value {
    let segments = shape.camera_segments.max(1);
    // Never shorter than 2 frames (a movement needs two) and the clip never shorter than the edit.
    let each = shape.edit_frames.div_ceil(segments as u64).max(2);
    let main: Vec<Value> = (0..segments).map(|i| camera_segment(i, each)).collect();
    let layers: Vec<Value> = (0..shape.camera_layers)
        .map(|_| {
            json!({ "segments": [{
                "recipe": "orbit",
                "durationFrames": shape.edit_frames.max(2),
                "from": "previousEnd",
                "yawDeg": 1.0,
                "easing": "linear",
            }] })
        })
        .collect();
    json!({
        "id": "clip",
        "initialPosition": vec3(0.0, 2.0, 10.0),
        "initialTarget": vec3(0.0, 1.0, 0.0),
        "initialFovDeg": 50.0,
        "segments": main,
        "layers": layers,
    })
}

fn shots(shape: &Shape) -> Vec<Value> {
    let count = shape.shots.max(1) as u64;
    let each = shape.edit_frames / count;
    (0..count)
        .map(|index| {
            let start = index * each;
            let end = if index + 1 == count {
                shape.edit_frames
            } else {
                start + each
            };
            let mut shot = json!({
                "id": format!("s{index}"),
                "sceneInstanceId": "instance",
                "editRange": { "startFrame": start, "endFrameExclusive": end },
                "instanceStartFrame": start,
                "cameraClipId": "clip",
                "cameraInFrame": start,
                "intent": format!("synthetic shot {index}"),
            });
            if index > 0 {
                shot["transitionInto"] = json!({ "type": "continuous" });
            }
            shot
        })
        .collect()
}

/// The JSON document of a synthetic scene: complete (no domain a plan refuses), 24000/1001 fps,
/// one scene definition, one instance, one camera clip, `shape.shots` shots.
pub fn synthetic_bundle(shape: &Shape) -> Value {
    let fps = json!({ "numerator": 24000, "denominator": 1001 });
    let assets: Vec<Value> = (0..shape.planes.clamp(1, ASSET_POOL))
        .map(|index| {
            json!({
                "assetId": format!("asset-{index}"),
                "revisionId": format!("asset-{index}-r1"),
                "contentHash": format!("hash-{index}"),
                "relativePath": format!("layers/asset-{index}.png"),
                "format": "png",
                "widthPx": 1024,
                "heightPx": 1024,
                "alphaMode": "straight",
                "colorSpace": "srgb",
            })
        })
        .collect();
    json!({
        "schemaVersion": "1.0",
        "bundleId": format!("synthetic-{}", shape.name),
        "revisionId": "rev-1",
        "fps": fps,
        "editDurationFrames": shape.edit_frames,
        "renderProfile": render_profile(&fps),
        "assets": assets,
        "sceneDefinitions": [{
            "id": "scene",
            "schemaVersion": "1.0",
            "rootNodeId": "root",
            "nodes": nodes(shape),
        }],
        "sceneInstances": [{
            "id": "instance",
            "sceneDefinitionId": "scene",
            "durationFrames": shape.edit_frames,
            "performanceTrackIds": if shape.channels == 0 { json!([]) } else { json!(["track"]) },
        }],
        "performanceTracks": if shape.channels == 0 {
            json!([])
        } else {
            json!([{ "id": "track", "instanceId": "instance", "channels": channels(shape) }])
        },
        "cameraClips": [camera_clip(shape)],
        "shots": shots(shape),
        "compositionPasses": [],
    })
}

pub fn synthetic_scene(shape: &Shape) -> DirectedScene {
    DirectedScene::import_value(synthetic_bundle(shape))
        .unwrap_or_else(|error| panic!("synthetic scene {} must import: {error:?}", shape.name))
}

/// The scale ladder the report measures. Small enough to run in a test, large enough that the
/// per-node and per-segment terms show.
pub const SHAPES: [Shape; 6] = [
    Shape {
        name: "small",
        planes: 8,
        groups: 2,
        channels: 4,
        edit_frames: 240,
        camera_segments: 2,
        camera_layers: 0,
        shots: 2,
    },
    Shape {
        name: "medium",
        planes: 100,
        groups: 10,
        channels: 50,
        edit_frames: 1440,
        camera_segments: 12,
        camera_layers: 1,
        shots: 4,
    },
    Shape {
        name: "large",
        planes: 1000,
        groups: 50,
        channels: 500,
        edit_frames: 7200,
        camera_segments: 60,
        camera_layers: 1,
        shots: 6,
    },
    Shape {
        name: "camera-heavy",
        planes: 4,
        groups: 1,
        channels: 2,
        edit_frames: 10_000,
        camera_segments: 2000,
        camera_layers: 1,
        shots: 2,
    },
    Shape {
        name: "channel-heavy",
        planes: 20,
        groups: 4,
        channels: 40,
        edit_frames: 2400,
        camera_segments: 4,
        camera_layers: 0,
        shots: 2,
    },
    Shape {
        name: "wide-flat",
        planes: 4000,
        groups: 0,
        channels: 0,
        edit_frames: 1200,
        camera_segments: 2,
        camera_layers: 0,
        shots: 1,
    },
];

pub fn shape_named(name: &str) -> Option<Shape> {
    SHAPES.iter().copied().find(|shape| shape.name == name)
}

// ---- the 55 + 47 corpus --------------------------------------------------------------------

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `(name, text)` of every bundle of the oracle corpora, valid and invalid: the 55 of
/// `docs/migration/fixtures/bundles` and the 47 of `extended-round2/bundles`, sorted by name so a
/// failure names the same bundle on every machine.
pub fn corpus_texts() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (label, directory) in [
        ("oracle", "docs/migration/fixtures/bundles"),
        (
            "extended-round2",
            "docs/migration/fixtures/extended-round2/bundles",
        ),
    ] {
        let root = repo_root().join(directory);
        let mut files: Vec<PathBuf> = fs::read_dir(&root)
            .unwrap_or_else(|error| panic!("{}: {error}", root.display()))
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .collect();
        files.sort();
        for path in files {
            let name = format!("{label}/{}", path.file_name().unwrap().to_string_lossy());
            out.push((name, fs::read_to_string(&path).unwrap()));
        }
    }
    out
}

/// The corpus bundles the importer accepts (the rest are the oracle's refusals and have no plan).
pub fn corpus_scenes() -> Vec<(String, DirectedScene)> {
    corpus_texts()
        .into_iter()
        .filter_map(|(name, text)| {
            DirectedScene::import_str(&text)
                .ok()
                .map(|scene| (name, scene))
        })
        .collect()
}

// ---- outcomes, digests and orders -----------------------------------------------------------

use tessel_editor_core::scene_plan::{PlanError, ScenePlan};

/// Canonical text of what a planner answered for one frame: the plan's wire JSON, or the error's
/// code, details and message. Two planners agree on a frame exactly when these texts are equal, so
/// Comparison preserves every value and number bit; object keys use canonical order because
/// workspace consumers can enable serde_json/preserve_order through feature unification.
pub fn outcome_text(outcome: &Result<ScenePlan, PlanError>) -> String {
    match outcome {
        Ok(plan) => format!("ok {}", canonical_json(plan.to_json())),
        Err(error) => format!(
            "err {} {} {}",
            error.code(),
            error.details().map(canonical_json).unwrap_or_default(),
            error
        ),
    }
}

fn canonical_json(mut value: Value) -> String {
    value.sort_all_objects();
    value.to_string()
}

/// FNV-1a 64 over a stream of frames. A digest, not a security hash: it is only here to pin the
/// output of the pre-change planner in a small committed file (`scene_plan_performance_golden/`).
pub struct Digest(u64);

impl Digest {
    pub fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    pub fn push(&mut self, text: &str) {
        for byte in text.bytes().chain(std::iter::once(0xff)) {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }

    pub fn finish(&self) -> String {
        format!("{:016x}", self.0)
    }
}

/// Every edit frame of a corpus bundle plus one on each side of the edit, so the range errors are
/// compared too.
pub fn corpus_frames(edit_duration_frames: u64) -> Vec<i64> {
    (-1..=edit_duration_frames as i64).collect()
}

/// At most `limit` frames spread over the edit (all of them when it is short), plus the two range
/// errors around it.
pub fn spread_frames(edit_frames: u64, limit: usize) -> Vec<i64> {
    let count = (limit as u64).min(edit_frames).max(1);
    let mut frames = vec![-1];
    frames.extend((0..count).map(|index| (index * edit_frames / count) as i64));
    frames.push(edit_frames as i64 - 1);
    frames.push(edit_frames as i64);
    frames
}

/// A deterministic shuffle (xorshift), so "out of order" is the same order on every machine.
pub fn shuffled<T: Clone>(items: &[T], seed: u64) -> Vec<T> {
    let mut state = seed | 1;
    let mut out = items.to_vec();
    for index in (1..out.len()).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.swap(index, (state % (index as u64 + 1)) as usize);
    }
    out
}
