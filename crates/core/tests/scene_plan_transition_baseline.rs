//! P4b S1b (BL082.4.3), M0: the plans of documents WITHOUT a transition, pinned before S1b.
//!
//! `scene_plan_transition_baseline/no-transition-plans.json` was produced by this file's generator
//! run on the UNMODIFIED base (`origin/main` e3c20cb). Windows profiles were pinned before S1b;
//! Mac profiles were captured later on that same old source, never on the candidate. It is not
//! two calls of the new planner compared to each other: it is bytes and digests of what the
//! planner answered when it knew nothing of transitions. The same file then guards the planner
//! with the transition code in place (an absent transition is an absent key: same bytes).
//!
//! Per scene and per frame (`-1` to `duration`, so the range errors are pinned too), in both modes
//! (`allow_partial` false and true), the canonical outcome text (object keys sorted, every number
//! exact) is recorded as `[bytes, fnv1a64]`. One set per build profile, like the BL064 golden: the
//! trig-derived floats differ in the last bit between debug and release.
//!
//! Temporary generators were removed after each capture on the unmodified base; this file only
//! reads the pinned bytes. The file's `pinnedAt` names the commit.
//!
//! Scenes: the four valid bundles of the oracle corpus (they import through the PUBLIC door) and
//! the two-shot minimal bundle with shot-scope and output effects but no transition.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::scene_plan::{PlanOptions, ScenePlan};

#[path = "scene_plan_performance_support/mod.rs"]
mod support;

use support::{Digest, outcome_text};

const PROFILE: &str = match (cfg!(windows), cfg!(debug_assertions)) {
    (false, true) => "debug",
    (false, false) => "release",
    (true, true) => "windows-debug",
    (true, false) => "windows-release",
};

const PINNED: &str = "tests/scene_plan_transition_baseline/no-transition-plans.json";

const VALID: [&str; 4] = [
    "valid-camera-recipes-layers",
    "valid-cut-instance-offsets",
    "valid-graph-sockets-performance",
    "valid-minimal-continuous",
];

/// The shot-scope + output effects of `scene_fx_shot_scope.rs::layer_in_stages`, over the minimal
/// bundle's `shot-1` / `shot-2`: a document with effects and no transition.
fn minimal_with_effects() -> Value {
    let path = support::repo_root()
        .join("docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json");
    let mut bundle: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    bundle["fx"] = json!({
        "catalogVersion": 1,
        "tracks": [
            {"id": "t-out", "label": "Out", "clips": [
                {"id": "o1", "effect": "fade", "startFrame": 0, "durationFrames": 96}]},
            {"id": "t-a", "label": "A", "clips": [
                {"id": "a1", "effect": "vignette", "startFrame": 0, "durationFrames": 48,
                 "scope": {"kind": "shot", "shotId": "shot-1"}},
                {"id": "a2", "effect": "vignette", "startFrame": 48, "durationFrames": 48,
                 "scope": {"kind": "shot", "shotId": "shot-2"}}]},
            {"id": "t-b", "label": "B", "clips": [
                {"id": "b1", "effect": "filmGrain", "startFrame": 10, "durationFrames": 10,
                 "scope": {"kind": "shot", "shotId": "shot-1"}}]},
        ],
        "transitions": [],
    });
    bundle
}

fn scenes() -> Vec<(String, DirectedScene)> {
    let mut scenes = Vec::new();
    for name in VALID {
        let path = support::repo_root()
            .join("docs/migration/fixtures/bundles")
            .join(format!("{name}.bundle.json"));
        let text = fs::read_to_string(path).unwrap();
        scenes.push((
            name.to_string(),
            DirectedScene::import_str(&text).unwrap_or_else(|error| panic!("{name}: {error:?}")),
        ));
    }
    scenes.push((
        "minimal-with-effects".to_string(),
        DirectedScene::import_value(minimal_with_effects()).expect("minimal with effects"),
    ));
    scenes
}

fn record(scene: &DirectedScene) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    let duration = scene.bundle().edit_duration_frames as i64;
    for (mode, partial) in [("strict", false), ("partial", true)] {
        let options = PlanOptions {
            allow_partial: partial,
        };
        for frame in -1..=duration {
            let text = outcome_text(&ScenePlan::build_with(scene, frame, options));
            let mut digest = Digest::new();
            digest.push(&text);
            out.insert(
                format!("{mode}:{frame}"),
                json!([text.len(), digest.finish()]),
            );
        }
    }
    out
}

#[test]
fn plans_without_a_transition_are_the_bytes_pinned_on_the_unmodified_base() {
    let current: BTreeMap<String, BTreeMap<String, Value>> = scenes()
        .iter()
        .map(|(name, scene)| (name.clone(), record(scene)))
        .collect();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(PINNED);
    let pinned: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let pinned = pinned[PROFILE]
        .as_object()
        .unwrap_or_else(|| panic!("no baseline pinned for the {PROFILE} profile"));
    assert_eq!(
        current.keys().collect::<Vec<_>>(),
        pinned.keys().collect::<Vec<_>>(),
        "the set of pinned scenes changed"
    );
    for (name, frames) in &current {
        let pinned_frames = pinned[name].as_object().unwrap();
        assert_eq!(frames.len(), pinned_frames.len(), "{name}: frames");
        let changed: Vec<&String> = frames
            .iter()
            .filter(|(key, entry)| pinned_frames[*key] != **entry)
            .map(|(key, _)| key)
            .collect();
        assert!(
            changed.is_empty(),
            "{name}: the plan of a document without transition changed at {changed:?}"
        );
    }
}
