//! The projector of the shot-trim corpus (F065, P5 S1): minimal oracle documents to render-bundle
//! 1.0 that the importer accepts, and the way back.
//!
//! The oracle (`scripts/shot-trim-reference.py`) models only what the shot ops read: shots,
//! instances, camera clips (a length), fx clips and transitions, light and weather keys, and a few
//! fields it never touches. [`project`] completes a minimal document with what the importer needs
//! (version, a scene definition per instance, camera segments, assets, render profile) **without
//! changing any field the oracle models**; [`unproject`] reads the bundle back into the minimal
//! form, field by field, so a comparison with the oracle's `expect` looks at the same things.
//!
//! The completed parts are written by hand here from the oracle's own models; nothing is read
//! back from the code under test.

#![allow(dead_code)]

use serde_json::{Map, Value, json};

/// The public corpus bundle the projector starts from (two scene definitions with an image plane
/// each, two assets).
pub const BASE: &str = include_str!(
    "../../../../docs/migration/fixtures/bundles/valid-cut-instance-offsets.bundle.json"
);

/// The effect every projected fx clip uses, and the numeric parameter each oracle key name maps to
/// (`filmGrain` has `amount` 0..1, `size` 0.5..4 and `luma` 0..1).
const EFFECT: &str = "filmGrain";
const PARAMS: [(&str, &str); 3] = [("amount", "amount"), ("x", "size"), ("tint", "luma")];

fn param_of(oracle: &str) -> &'static str {
    PARAMS
        .iter()
        .find(|(name, _)| *name == oracle)
        .unwrap_or_else(|| panic!("the projector has no parameter for the oracle key `{oracle}`"))
        .1
}

fn oracle_of(param: &str) -> &'static str {
    PARAMS
        .iter()
        .find(|(_, name)| *name == param)
        .unwrap_or_else(|| panic!("no oracle key for the parameter `{param}`"))
        .0
}

/// The oracle's minimal document as a bundle.
pub fn project(minimal: &Value) -> Value {
    let mut bundle: Value = serde_json::from_str(BASE).unwrap();
    bundle["editDurationFrames"] = minimal["editDurationFrames"].clone();
    bundle["fps"] = minimal["fps"].clone();
    bundle["renderProfile"]["fps"] = minimal["fps"].clone();

    let template = bundle["sceneDefinitions"][0].clone();
    let mut definitions: Vec<Value> = Vec::new();
    let mut wanted: Vec<String> = Vec::new();
    for instance in minimal["sceneInstances"].as_array().unwrap() {
        let id = instance["sceneDefinitionId"].as_str().unwrap().to_string();
        if !wanted.contains(&id) {
            wanted.push(id);
        }
    }
    for definition in minimal["sceneDefinitions"].as_array().unwrap() {
        let id = definition["id"].as_str().unwrap().to_string();
        if !wanted.contains(&id) {
            wanted.push(id);
        }
    }
    for id in &wanted {
        let mut definition = template.clone();
        definition["id"] = json!(id);
        if let Some(extra) = minimal["sceneDefinitions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|candidate| candidate["id"] == *id)
        {
            for (key, value) in extra.as_object().unwrap() {
                definition[key] = value.clone();
            }
        }
        definitions.push(definition);
    }
    bundle["sceneDefinitions"] = Value::Array(definitions);

    bundle["sceneInstances"] = Value::Array(
        minimal["sceneInstances"]
            .as_array()
            .unwrap()
            .iter()
            .map(|instance| {
                let mut out = json!({
                    "id": instance["id"], "sceneDefinitionId": instance["sceneDefinitionId"],
                    "durationFrames": instance["durationFrames"], "performanceTrackIds": [],
                });
                if instance.get("editLocked") == Some(&Value::Bool(true)) {
                    out["editLocked"] = json!(true);
                }
                out
            })
            .collect(),
    );
    bundle["cameraClips"] = Value::Array(
        minimal["cameraClips"]
            .as_array()
            .unwrap()
            .iter()
            .map(|clip| {
                json!({"id": clip["id"], "initialPosition": {"x": 0, "y": 1, "z": 6},
                       "initialTarget": {"x": 0, "y": 1, "z": 0}, "initialFovDeg": 50,
                       "segments": [{"recipe": "hold", "durationFrames": clip["durationFrames"],
                                     "from": "initial"}]})
            })
            .collect(),
    );
    bundle["shots"] = minimal["shots"].clone();
    if let Some(fx) = minimal.get("fx") {
        bundle["fx"] = project_fx(fx);
    }
    for key in ["narration", "outline"] {
        if let Some(value) = minimal.get(key) {
            bundle[key] = value.clone();
        }
    }
    if let Some(audio) = minimal.get("audio") {
        bundle["audio"] = json!({
            "mixRelativePath": "audio/mix.wav", "sourcePcmHashSha256": "synthetic-sha256-mix",
            "editStartSeconds": audio["editStartSeconds"],
            "editEndSecondsExclusive": audio["editEndSecondsExclusive"],
        });
    }
    bundle
}

/// `fx` is a reserved id of the importer and the oracle names a track that: tracks are prefixed
/// on the way in and unprefixed on the way out.
fn track_id(oracle: &str) -> String {
    format!("track-{oracle}")
}

fn untrack_id(bundle: &str) -> String {
    bundle.strip_prefix("track-").unwrap_or(bundle).to_string()
}

fn project_fx(fx: &Value) -> Value {
    let tracks: Vec<Value> = fx["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|track| {
            let clips: Vec<Value> = track["clips"]
                .as_array()
                .unwrap()
                .iter()
                .map(|clip| {
                    let mut out = json!({
                        "id": clip["id"], "effect": EFFECT,
                        "startFrame": clip["startFrame"], "durationFrames": clip["durationFrames"],
                    });
                    for key in ["scope", "fadeInFrames", "fadeOutFrames"] {
                        if let Some(value) = clip.get(key) {
                            out[key] = value.clone();
                        }
                    }
                    if let Some(keys) = clip.get("keys") {
                        let mut mapped = Map::new();
                        for (name, list) in keys.as_object().unwrap() {
                            mapped.insert(param_of(name).to_string(), list.clone());
                        }
                        out["keys"] = Value::Object(mapped);
                    }
                    out
                })
                .collect();
            json!({"id": track_id(track["id"].as_str().unwrap()), "label": track["id"], "clips": clips})
        })
        .collect();
    let transitions: Vec<Value> = fx["transitions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|transition| {
            json!({
                "id": transition["id"], "effect": "crossDissolve",
                "fromShotId": transition["fromShotId"], "toShotId": transition["toShotId"],
                "durationFrames": transition["durationFrames"],
                "alignment": transition["alignment"],
                "handles": transition.get("handles").cloned().unwrap_or(json!("hold")),
                "easing": "easeInOutCubic",
            })
        })
        .collect();
    json!({"catalogVersion": 1, "tracks": tracks, "transitions": transitions})
}

fn copy_keys(from: &Value, keys: &[&str]) -> Value {
    let mut out = Map::new();
    for key in keys {
        if let Some(value) = from.get(*key) {
            out.insert((*key).to_string(), value.clone());
        }
    }
    Value::Object(out)
}

/// What the bundle says, in the oracle's minimal form. `original` is the minimal document that was
/// projected: it says which optional fields (`sceneDefinitions` keys, `narration`, `audio`...) the
/// form carries and what each instance called its definition.
pub fn unproject(bundle: &Value, original: &Value) -> Value {
    let shots: Vec<Value> = bundle["shots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|shot| {
            copy_keys(
                shot,
                &[
                    "id",
                    "sceneInstanceId",
                    "editRange",
                    "instanceStartFrame",
                    "cameraClipId",
                    "cameraInFrame",
                    "intent",
                    "transitionInto",
                ],
            )
        })
        .collect();
    let instances: Vec<Value> = bundle["sceneInstances"]
        .as_array()
        .unwrap()
        .iter()
        .map(|instance| {
            copy_keys(
                instance,
                &["id", "sceneDefinitionId", "durationFrames", "editLocked"],
            )
        })
        .collect();
    let clips: Vec<Value> = bundle["cameraClips"]
        .as_array()
        .unwrap()
        .iter()
        .map(|clip| {
            let frames: u64 = clip["segments"]
                .as_array()
                .unwrap()
                .iter()
                .map(|segment| segment["durationFrames"].as_u64().unwrap())
                .sum();
            json!({"id": clip["id"], "durationFrames": frames})
        })
        .collect();
    let definitions: Vec<Value> = original["sceneDefinitions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|wanted| {
            let found = bundle["sceneDefinitions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|definition| definition["id"] == wanted["id"])
                .expect("the definition is still in the bundle");
            let mut out = Map::new();
            out.insert("id".into(), wanted["id"].clone());
            for key in wanted
                .as_object()
                .unwrap()
                .keys()
                .filter(|key| *key != "id")
            {
                if let Some(value) = found.get(key) {
                    out.insert(key.clone(), value.clone());
                }
            }
            Value::Object(out)
        })
        .collect();
    let mut out = json!({
        "editDurationFrames": bundle["editDurationFrames"], "fps": bundle["fps"],
        "shots": shots, "sceneInstances": instances, "cameraClips": clips,
        "sceneDefinitions": definitions, "fx": unproject_fx(bundle.get("fx")),
    });
    for key in ["narration", "outline"] {
        if original.get(key).is_some() {
            out[key] = bundle[key].clone();
        }
    }
    if original.get("audio").is_some() {
        out["audio"] = copy_keys(
            &bundle["audio"],
            &["editStartSeconds", "editEndSecondsExclusive"],
        );
    }
    out
}

fn unproject_fx(fx: Option<&Value>) -> Value {
    let Some(fx) = fx else {
        return json!({"tracks": [], "transitions": []});
    };
    let empty = Vec::new();
    let tracks: Vec<Value> = fx["tracks"]
        .as_array()
        .unwrap_or(&empty)
        .iter()
        .map(|track| {
            let clips: Vec<Value> = track["clips"]
                .as_array()
                .unwrap()
                .iter()
                .map(|clip| {
                    let mut out = copy_keys(
                        clip,
                        &[
                            "id",
                            "startFrame",
                            "durationFrames",
                            "scope",
                            "fadeInFrames",
                            "fadeOutFrames",
                        ],
                    );
                    if let Some(keys) = clip.get("keys") {
                        let mut mapped = Map::new();
                        for (name, list) in keys.as_object().unwrap() {
                            mapped.insert(oracle_of(name).to_string(), list.clone());
                        }
                        out["keys"] = Value::Object(mapped);
                    }
                    out
                })
                .collect();
            json!({"id": untrack_id(track["id"].as_str().unwrap()), "clips": clips})
        })
        .collect();
    let transitions: Vec<Value> = fx["transitions"]
        .as_array()
        .unwrap_or(&empty)
        .iter()
        .map(|transition| {
            copy_keys(
                transition,
                &[
                    "id",
                    "fromShotId",
                    "toShotId",
                    "durationFrames",
                    "alignment",
                    "handles",
                ],
            )
        })
        .collect();
    json!({"tracks": tracks, "transitions": transitions})
}
