//! Shared builders. Every number is written by hand from the oracle's models
//! (`scripts/pilot-dissolve-corpus.py`: `spec_edit`, `corpus_edit`, `tr`); nothing is read back
//! from the code under test.

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::fx::TransitionAdmission;
use tessel_editor_core::directed_scene::{DirectedScene, ImportError, Limits};

/// Two shots over 96 frames joined by nothing (`shot-1` `[0, 48)`, `shot-2` `[48, 96)`).
pub const MINIMAL: &str = include_str!(
    "../../../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json"
);
/// Two shots over 100 frames joined by a cut: `shot-a` `[0, 40)`, `shot-b` `[40, 100)`.
pub const CUT: &str = include_str!(
    "../../../../docs/migration/fixtures/bundles/valid-cut-instance-offsets.bundle.json"
);

pub fn bundle_of(source: &str) -> Value {
    serde_json::from_str(source).unwrap()
}

/// One shot of an edit model.
#[derive(Clone)]
pub struct EditShot {
    pub id: String,
    pub start: u64,
    pub end: u64,
    pub instance: String,
    pub instance_start: u64,
    pub camera: String,
    pub camera_in: u64,
}

/// The oracle's `edit_model`: the numbers a transition is judged against.
#[derive(Clone)]
pub struct Edit {
    pub duration: u64,
    pub shots: Vec<EditShot>,
    pub instances: Vec<(String, u64)>,
    pub cameras: Vec<(String, u64)>,
}

fn shot(
    id: &str,
    start: u64,
    end: u64,
    instance: &str,
    instance_start: u64,
    camera: &str,
    camera_in: u64,
) -> EditShot {
    EditShot {
        id: id.into(),
        start,
        end,
        instance: instance.into(),
        instance_start,
        camera: camera.into(),
        camera_in,
    }
}

impl Edit {
    /// The origin's three 48-frame shots (`fx-test-bundle`, reconstructed by the oracle from the
    /// numbers its specs pin): `s1` `[0,48)`, `s2` `[48,96)`, `s3` `[96,144)`.
    pub fn spec() -> Self {
        Self {
            duration: 144,
            shots: vec![
                shot("s1", 0, 48, "inst1", 20, "cam1", 0),
                shot("s2", 48, 96, "inst2", 0, "cam2", 0),
                shot("s3", 96, 144, "inst3", 20, "cam2", 48),
            ],
            instances: vec![
                ("inst1".into(), 200),
                ("inst2".into(), 200),
                ("inst3".into(), 200),
            ],
            cameras: vec![("cam1".into(), 48), ("cam2".into(), 96)],
        }
    }

    pub fn camera(mut self, id: &str, frames: u64) -> Self {
        self.cameras
            .iter_mut()
            .find(|(name, _)| name == id)
            .unwrap()
            .1 = frames;
        self
    }

    pub fn shot(mut self, id: &str, change: impl FnOnce(&mut EditShot)) -> Self {
        change(self.shots.iter_mut().find(|item| item.id == id).unwrap());
        self
    }

    /// `n` shots of 4 frames, each with its own instance (8 frames) and camera (4 frames): the
    /// smallest edit with `n - 1` cuts, for the count limit.
    pub fn many_shots(n: u64) -> Self {
        let shots = (0..n)
            .map(|i| {
                shot(
                    &format!("s{i}"),
                    i * 4,
                    i * 4 + 4,
                    &format!("i{i}"),
                    0,
                    &format!("c{i}"),
                    0,
                )
            })
            .collect();
        Self {
            duration: n * 4,
            shots,
            instances: (0..n).map(|i| (format!("i{i}"), 8)).collect(),
            cameras: (0..n).map(|i| (format!("c{i}"), 4)).collect(),
        }
    }

    /// The edit as a bundle document (no `fx`): the public corpus bundle's scenes, one instance
    /// per entry, a camera of one `hold` segment per camera, a cut into every shot after the first.
    pub fn bundle(&self) -> Value {
        let mut raw = bundle_of(CUT);
        raw["editDurationFrames"] = json!(self.duration);
        raw["sceneInstances"] = Value::Array(
            self.instances
                .iter()
                .enumerate()
                .map(|(i, (id, frames))| {
                    json!({"id": id, "sceneDefinitionId": if i % 2 == 0 { "scene-a" } else { "scene-b" },
                           "durationFrames": frames, "performanceTrackIds": []})
                })
                .collect(),
        );
        raw["cameraClips"] = Value::Array(
            self.cameras
                .iter()
                .map(|(id, frames)| {
                    json!({"id": id, "initialPosition": {"x":0,"y":1,"z":6},
                           "initialTarget": {"x":0,"y":1,"z":0}, "initialFovDeg": 50,
                           "segments": [{"recipe":"hold","durationFrames":frames,"from":"initial"}]})
                })
                .collect(),
        );
        raw["shots"] = Value::Array(
            self.shots
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    let mut value = json!({
                        "id": item.id, "sceneInstanceId": item.instance,
                        "editRange": {"startFrame": item.start, "endFrameExclusive": item.end},
                        "instanceStartFrame": item.instance_start, "cameraClipId": item.camera,
                        "cameraInFrame": item.camera_in, "intent": format!("synthetic {}", item.id),
                    });
                    if i > 0 {
                        value["transitionInto"] = json!({"type":"cut","reason":"synthetic cut"});
                    }
                    value
                })
                .collect(),
        );
        raw
    }
}

/// The corpus bundle as the repository ships it.
pub fn corpus() -> Value {
    bundle_of(CUT)
}

/// A transition as the oracle's `tr()` writes it: `s1` to `s2`, 12 frames, centred, `hold`.
pub fn tr(overrides: Value) -> Value {
    let mut base = json!({
        "id": "tr", "effect": "crossDissolve", "fromShotId": "s1", "toShotId": "s2",
        "durationFrames": 12, "alignment": "center", "handles": "hold", "easing": "easeInOutCubic",
    });
    for (key, value) in overrides.as_object().unwrap() {
        base[key] = value.clone();
    }
    base
}

/// The bundle with `list` as its transitions (and no tracks).
pub fn with_transitions(mut bundle: Value, list: Vec<Value>) -> Value {
    bundle["fx"] = json!({"catalogVersion": 1, "tracks": [], "transitions": list});
    bundle
}

/// A `splat` node (and the `spz` asset it reads, once) in each of the named scene definitions.
pub fn add_splats(bundle: &mut Value, scenes: &[&str]) {
    let assets = bundle["assets"].as_array_mut().unwrap();
    if !assets.iter().any(|asset| asset["assetId"] == "cloud") {
        assets.push(json!({
            "assetId": "cloud", "revisionId": "cloud-r1", "contentHash": "hash-cloud",
            "relativePath": "splats/cloud.spz", "format": "spz", "widthPx": 1, "heightPx": 1,
            "alphaMode": "opaque", "colorSpace": "srgb",
        }));
    }
    for definition in bundle["sceneDefinitions"].as_array_mut().unwrap() {
        if !scenes.iter().any(|scene| definition["id"] == *scene) {
            continue;
        }
        definition["nodes"].as_array_mut().unwrap().push(json!({
            "id": "cloud-node", "parentId": "root", "type": "splat",
            "assetRef": {"assetId": "cloud", "revisionId": "cloud-r1",
                         "contentHash": "hash-cloud", "relativePath": "splats/cloud.spz"},
            "localPosition": {"x": 0, "y": 0, "z": 0}, "localRotation": {"x": 0, "y": 0, "z": 0, "w": 1},
            "localScale": {"x": 1, "y": 1, "z": 1},
        }));
    }
}

/// The hidden policy that reads transitions (the tests' only door to it).
pub fn admit(value: Value) -> Result<DirectedScene, ImportError> {
    DirectedScene::import_value_admitting(value, &Limits::default(), TransitionAdmission::Accept)
}

pub fn admit_str(text: &str) -> Result<DirectedScene, ImportError> {
    DirectedScene::import_str_admitting(text, &Limits::default(), TransitionAdmission::Accept)
}

pub fn codes(error: &ImportError) -> Vec<&'static str> {
    error.issues().iter().map(|issue| issue.code).collect()
}

/// The codes an admitting import answers with, `[]` for an accepted bundle.
pub fn admit_codes(value: Value) -> Vec<&'static str> {
    match admit(value) {
        Ok(_) => Vec::new(),
        Err(error) => codes(&error),
    }
}

/// The only issue of a refusal.
pub fn only_issue(value: Value) -> (String, String, String) {
    let error = admit(value).expect_err("the bundle should be refused");
    assert_eq!(error.issues().len(), 1, "{:?}", error.issues());
    let issue = &error.issues()[0];
    (
        issue.code.to_string(),
        issue.path.clone(),
        issue.message.clone(),
    )
}

// ---- S1b (BL082.4.3): two-deck evaluation and the plan of a transition window ---------------------

use tessel_editor_core::scene_plan::ScenePlan;

/// The scene of `bundle` through the hidden admitting importer, or the issues it refused with.
pub fn scene_of(bundle: Value) -> DirectedScene {
    admit(bundle).unwrap_or_else(|error| panic!("the bundle was refused: {:?}", error.issues()))
}

/// The oracle's spec edit (`s1`, `s2`, `s3`) with `list` as its transitions.
pub fn spec_scene(list: Vec<Value>) -> DirectedScene {
    scene_of(with_transitions(Edit::spec().bundle(), list))
}

/// The same edit with a layer of effect `tracks` next to the transitions.
pub fn spec_scene_with_tracks(list: Vec<Value>, tracks: Value) -> DirectedScene {
    let mut bundle = with_transitions(Edit::spec().bundle(), list);
    bundle["fx"]["tracks"] = tracks;
    scene_of(bundle)
}

pub fn plan_at(scene: &DirectedScene, frame: i64) -> ScenePlan {
    ScenePlan::build(scene, frame)
        .unwrap_or_else(|error| panic!("plan at frame {frame}: {error:?}"))
}

/// A shot-scope clip of the layer, on its own track.
pub fn shot_clip(
    track: &str,
    clip: &str,
    effect: &str,
    shot: &str,
    start: i64,
    duration: i64,
) -> Value {
    json!({"id": track, "label": track, "clips": [{
        "id": clip, "effect": effect, "startFrame": start, "durationFrames": duration,
        "scope": {"kind": "shot", "shotId": shot}}]})
}

/// An output-scope clip of the layer, on its own track.
pub fn output_clip(track: &str, clip: &str, effect: &str, start: i64, duration: i64) -> Value {
    json!({"id": track, "label": track, "clips": [{
        "id": clip, "effect": effect, "startFrame": start, "durationFrames": duration}]})
}
