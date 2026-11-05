//! Shared by the scene renderer tests: the synthetic corpus of `scripts/directed-scene-proof`
//! (drawn from scratch, no private media) and the helpers that turn it into plans and resources.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::scene_plan::ScenePlan;
use tessel_editor_render::SceneResourceMap;
use tessel_editor_render::scene::import_gated;

pub fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/directed-scene-proof/corpus")
}

pub fn expected() -> Value {
    serde_json::from_str(&fs::read_to_string(corpus().join("expected.json")).unwrap()).unwrap()
}

/// `bundle.json` (hero rev-1) or `bundle.swap.json` (hero rev-2), through the renderer's gates.
pub fn scene(file: &str) -> DirectedScene {
    import_gated(&fs::read_to_string(corpus().join(file)).unwrap()).unwrap()
}

pub fn plan(scene: &DirectedScene, frame: i64) -> ScenePlan {
    ScenePlan::build(scene, frame).unwrap()
}

/// The resources of `plan` under the corpus folder, as project generation `generation`.
pub fn resources(generation: u64, plan: &ScenePlan) -> SceneResourceMap {
    SceneResourceMap::from_root(generation, plan, &corpus()).unwrap()
}
