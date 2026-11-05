//! BL064 — the planner's output, pinned. `ScenePlan::build_with` is shared by every consumer, and
//! BL064 refactored its internals (`Prepared`) to share them with `PlanTimeline`; this suite proves
//! the refactor changed nothing a consumer can see.
//!
//! The digests in `scene_plan_performance_golden/digests.json` were produced by **this very file
//! run on the unmodified main (914c830)**, before any change of BL064, once per profile:
//!
//! ```text
//! SCENE_PLAN_GOLDEN_WRITE=1 ./scripts/cargo test           -p tessel-editor-core --test scene_plan_performance_golden
//! SCENE_PLAN_GOLDEN_WRITE=1 ./scripts/cargo test --release -p tessel-editor-core --test scene_plan_performance_golden
//! ```
//!
//! WHY one set per profile: the unmodified main already answers differently in `debug` and
//! `release` in the last bit of some trigonometry-derived numbers (measured: 56 floats over 168
//! plans of two bundles differ, the worst by 8.9e-16 absolute, six orders below the 1e-10
//! tolerance the oracle fixed). A single digest would fail one profile for a reason that predates
//! this work; two digests pin each profile exactly.
//!
//! Each digest covers, for one scene, the full outcome (plan JSON or error code/details/message) of
//! every frame asked, in both modes (`allow_partial` false and true). The scenes are the 55 + 47
//! oracle corpus bundles that import and the synthetic scale scenes (no private media). A digest is
//! Object keys are sorted before hashing so GPUI feature unification enabling
//! serde_json/preserve_order does not alter the gate. Numbers and values remain exact.
//! FNV-1a 64 of the concatenated outcomes: it detects any difference, it does not say where; use
//! `scene_plan_performance.rs` (plan against plan, frame by frame) to localize one.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::scene_plan::{PlanOptions, ScenePlan};

#[path = "scene_plan_performance_support/mod.rs"]
mod support;

use support::{Digest, SHAPES, corpus_frames, corpus_scenes, outcome_text, spread_frames};

const PROFILE: &str = if cfg!(debug_assertions) {
    "debug"
} else {
    "release"
};

const GOLDEN: &str = "tests/scene_plan_performance_golden/digests.json";

/// Frames of a synthetic scene that the digest covers: bounded so the suite stays fast, spread so
/// shot changes are inside.
const SYNTHETIC_FRAME_LIMIT: usize = 200;

fn digest_of(scene: &DirectedScene, frames: &[i64]) -> String {
    let mut digest = Digest::new();
    for partial in [false, true] {
        let options = PlanOptions {
            allow_partial: partial,
        };
        for frame in frames {
            digest.push(&outcome_text(&ScenePlan::build_with(
                scene, *frame, options,
            )));
        }
    }
    digest.finish()
}

fn current_digests() -> BTreeMap<String, String> {
    let mut digests = BTreeMap::new();
    for (name, scene) in corpus_scenes() {
        let frames = corpus_frames(scene.bundle().edit_duration_frames);
        digests.insert(name, digest_of(&scene, &frames));
    }
    for shape in SHAPES {
        // The widest scenes are covered by the equivalence suite and the benchmark; here only the
        // ones whose full-JSON digest is cheap enough for a debug build.
        if shape.planes > 200 || shape.camera_segments > 100 {
            continue;
        }
        let scene = support::synthetic_scene(&shape);
        let frames = spread_frames(shape.edit_frames, SYNTHETIC_FRAME_LIMIT);
        digests.insert(
            format!("synthetic/{}", shape.name),
            digest_of(&scene, &frames),
        );
    }
    digests
}

#[test]
fn planner_output_is_the_one_pinned_before_the_refactor() {
    let digests = current_digests();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(GOLDEN);
    if std::env::var_os("SCENE_PLAN_GOLDEN_WRITE").is_some() {
        // Merge: writing one profile keeps the other's digests.
        let mut body: Value = fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_else(|| json!({}));
        body["pinnedAt"] = json!("914c830 (main, before BL064)");
        body["digest"] = json!("fnv1a-64 of the outcomes of every frame in both modes");
        body[PROFILE] = json!(digests);
        fs::write(&path, serde_json::to_string_pretty(&body).unwrap() + "\n").unwrap();
        return;
    }
    let pinned: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let pinned: BTreeMap<String, String> = pinned[PROFILE]
        .as_object()
        .unwrap_or_else(|| panic!("no digests pinned for the {PROFILE} profile"))
        .iter()
        .map(|(name, value)| (name.clone(), value.as_str().unwrap().to_string()))
        .collect();
    assert_eq!(
        digests.keys().collect::<Vec<_>>(),
        pinned.keys().collect::<Vec<_>>(),
        "the set of pinned scenes changed"
    );
    let changed: Vec<&String> = digests
        .iter()
        .filter(|(name, digest)| pinned[*name] != **digest)
        .map(|(name, _)| name)
        .collect();
    assert!(
        changed.is_empty(),
        "planner output changed for: {changed:?}"
    );
}
