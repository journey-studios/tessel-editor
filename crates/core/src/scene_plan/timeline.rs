//! `PlanTimeline`: the plans of many frames of one scene without laying the scene out again for
//! each of them (BL064).
//!
//! `ScenePlan::build_with` pays, for every frame, for work that does not depend on the frame: it
//! rebuilds the node and camera timelines, scans the document for the domains the scene declares
//! and does not evaluate, sorts the draw order, collects the resources, walks the unknown fields
//! and validates the whole plan. Measured on `scripts/scene-plan-performance/` (report:
//! `docs/migration/next-r6-report.md`), the timeline setup is a few percent of a plan except for a
//! clip with thousands of movements, and the rest is the frame-independent assembly. This type
//! does that work once per shot and per instance and answers each frame from it.
//!
//! What it promises, and what keeps the promise:
//!
//! - **Same answer.** `PlanTimeline::build_with(f, options)` returns exactly what
//!   `ScenePlan::build_with(scene, f, options)` returns, the plan or the error, whichever frames
//!   were asked before and in whatever order. The evaluation is the same function
//!   (`Bundle::evaluate_frame_with`) and the frame-independent parts come from the same code
//!   (`Prepared`), so there is no second implementation to drift. `tests/scene_plan_performance.rs`
//!   compares plan and error, value for value and byte for byte of the JSON, over the 55 + 47
//!   corpus bundles and synthetic scenes.
//! - **Same error order.** The frame's error (range, shot, channels, camera, non-finite node) comes
//!   before anything about the document, as in `build_with`; a shared layout that failed to build
//!   fails again, with the same error, for every frame that needs it.
//! - **Bounded memory.** The timeline holds one lazily built layout per instance, per camera clip
//!   and per shot of the bundle, nothing per frame: asking a million frames retains what asking
//!   one did. The layouts borrow the scene, so a `PlanTimeline` cannot outlive an edit of it: a
//!   consumer that swaps its scene (a new generation) builds a new timeline, and the old one keeps
//!   answering for the scene it was made for until it is dropped.
//! - **Shareable.** It is `Sync`: several threads may ask for different frames of the same
//!   timeline. Preparing a layout twice at once is harmless (`OnceLock` keeps one result).
//!
//! The shot is the unit of reuse because it fixes the instance, the camera clip and the scene
//! definition, and everything reused is a function of those (see `Prepared`). The first frame of a
//! shot builds and validates the full plan the later frames are cut from; a later frame only
//! replaces what a frame changes (times, camera, node transforms and matrices), whose numbers the
//! evaluator already guarantees finite (`NonFiniteNode`, `NonFiniteCamera`).

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::OnceLock;

use crate::directed_scene::{
    CameraTimeline, DirectedScene, NodeTimeline, SceneInstance, Shot, TemporalError,
};

use super::build::Prepared;
use super::{PlanError, PlanOptions, ScenePlan};

/// The frame-independent part of a shot's plans. Both stages are kept as results so a failure is
/// reported again, unchanged, to every frame of the shot.
struct ShotLayout<'a> {
    prepared: Prepared<'a>,
    /// The validated plan of the first frame asked for; later frames are cut from it.
    template: OnceLock<Result<ScenePlan, PlanError>>,
}

/// A scene laid out once to plan any number of its frames, in any order. See the module docs.
pub struct PlanTimeline<'a> {
    scene: &'a DirectedScene,
    // Lookups mirror the ones `evaluate_frame` does by id (first match wins, as in `Bundle`); shots
    // are identified by the address of their element in `bundle.shots`, which is exact even if two
    // shots were to share an id.
    instance_slots: HashMap<&'a str, usize>,
    clip_slots: HashMap<&'a str, usize>,
    shot_slots: HashMap<usize, usize>,
    nodes: Vec<OnceLock<Result<NodeTimeline<'a>, TemporalError>>>,
    cameras: Vec<OnceLock<Result<CameraTimeline<'a>, TemporalError>>>,
    shots: Vec<OnceLock<Result<ShotLayout<'a>, PlanError>>>,
}

fn first_slots<'a>(ids: impl Iterator<Item = &'a str>) -> HashMap<&'a str, usize> {
    let mut slots = HashMap::new();
    for (slot, id) in ids.enumerate() {
        slots.entry(id).or_insert(slot);
    }
    slots
}

fn empty_slots<T>(count: usize) -> Vec<OnceLock<T>> {
    (0..count).map(|_| OnceLock::new()).collect()
}

fn address_of(shot: &Shot) -> usize {
    std::ptr::from_ref(shot) as usize
}

impl<'a> PlanTimeline<'a> {
    /// A timeline over `scene`. Cheap: it indexes the bundle's instances, clips and shots and
    /// builds nothing until a frame asks.
    pub fn new(scene: &'a DirectedScene) -> Self {
        let bundle = scene.bundle();
        Self {
            scene,
            instance_slots: first_slots(bundle.instances.iter().map(|item| item.id.as_str())),
            clip_slots: first_slots(bundle.camera_clips.iter().map(|item| item.id.as_str())),
            shot_slots: bundle
                .shots
                .iter()
                .enumerate()
                .map(|(slot, shot)| (address_of(shot), slot))
                .collect(),
            nodes: empty_slots(bundle.instances.len()),
            cameras: empty_slots(bundle.camera_clips.len()),
            shots: empty_slots(bundle.shots.len()),
        }
    }

    /// The scene the timeline plans.
    pub fn scene(&self) -> &'a DirectedScene {
        self.scene
    }

    /// How many layouts (node timelines, camera timelines, shots) are built so far. It never
    /// exceeds `instances + clips + shots` of the bundle, however many frames were asked: what the
    /// timeline retains is a function of the scene, not of the use. Diagnostic; no behaviour
    /// depends on it.
    pub fn prepared_layouts(&self) -> usize {
        let nodes = self
            .nodes
            .iter()
            .filter(|slot| slot.get().is_some())
            .count();
        let cameras = self
            .cameras
            .iter()
            .filter(|slot| slot.get().is_some())
            .count();
        let shots = self
            .shots
            .iter()
            .filter(|slot| slot.get().is_some())
            .count();
        nodes + cameras + shots
    }

    /// `ScenePlan::build` for `edit_frame`: the same plan or error, from the shared layouts.
    pub fn build(&self, edit_frame: i64) -> Result<ScenePlan, PlanError> {
        self.build_with(edit_frame, PlanOptions::default())
    }

    /// `ScenePlan::build_with` for `edit_frame`: the same plan or error, from the shared layouts.
    pub fn build_with(
        &self,
        edit_frame: i64,
        options: PlanOptions,
    ) -> Result<ScenePlan, PlanError> {
        let frame = self.scene.bundle().evaluate_frame_with(
            edit_frame,
            |instance| self.node_timeline(instance),
            |clip_id| self.camera_timeline(clip_id),
        )?;
        let Some(&slot) = self.shot_slots.get(&address_of(frame.mapping.shot)) else {
            // Not reachable for a frame of this bundle; kept total instead of panicking.
            return Err(PlanError::Unresolved {
                kind: "shot",
                id: frame.mapping.shot.id.clone(),
            });
        };
        let layout = self.shots[slot]
            .get_or_init(|| {
                Prepared::new(self.scene, &frame).map(|prepared| ShotLayout {
                    prepared,
                    template: OnceLock::new(),
                })
            })
            .as_ref()
            .map_err(Clone::clone)?;
        layout.prepared.refuse_unsupported(options)?;
        let template = layout
            .template
            .get_or_init(|| layout.prepared.assemble(self.scene, &frame))
            .as_ref()
            .map_err(Clone::clone)?;
        let bundle = self.scene.bundle();
        let plan = template.at_frame(&frame, &bundle.fx, &bundle.shots)?;
        Ok(plan.relit(layout.prepared.lights_at(&frame)))
    }

    fn node_timeline(
        &self,
        instance: &'a SceneInstance,
    ) -> Result<Cow<'_, NodeTimeline<'a>>, TemporalError> {
        let bundle = self.scene.bundle();
        let Some(&slot) = self.instance_slots.get(instance.id.as_str()) else {
            return bundle.node_timeline(&instance.id).map(Cow::Owned);
        };
        match self.nodes[slot].get_or_init(|| bundle.node_timeline(&instance.id)) {
            Ok(timeline) => Ok(Cow::Borrowed(timeline)),
            Err(error) => Err(error.clone()),
        }
    }

    fn camera_timeline(&self, clip_id: &str) -> Result<Cow<'_, CameraTimeline<'a>>, TemporalError> {
        let bundle = self.scene.bundle();
        let Some(&slot) = self.clip_slots.get(clip_id) else {
            return bundle.camera_timeline(clip_id).map(Cow::Owned);
        };
        match self.cameras[slot].get_or_init(|| bundle.camera_timeline(clip_id)) {
            Ok(timeline) => Ok(Cow::Borrowed(timeline)),
            Err(error) => Err(error.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directed_scene::fx::Fx;

    const MINIMAL: &str = include_str!(
        "../../../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json"
    );
    const RECIPES: &str = include_str!(
        "../../../../docs/migration/fixtures/bundles/valid-camera-recipes-layers.bundle.json"
    );
    const SOCKETS: &str = include_str!(
        "../../../../docs/migration/fixtures/bundles/valid-graph-sockets-performance.bundle.json"
    );

    /// The template of one scene must never be cut to the frame of another: a node list that does
    /// not line up is a typed error naming the node, not a plan holding someone else's nodes.
    #[test]
    fn a_frame_of_another_scene_does_not_line_up_with_the_template() {
        let scene = DirectedScene::import_str(MINIMAL).unwrap();
        let other = DirectedScene::import_str(SOCKETS).unwrap();
        let frame = scene.bundle().evaluate_frame(0).unwrap();
        let foreign = other.bundle().evaluate_frame(0).unwrap();
        let template = Prepared::new(&scene, &frame)
            .unwrap()
            .assemble(&scene, &frame)
            .unwrap();

        assert_eq!(
            template.at_frame(&frame, &Fx::default(), &[]).unwrap(),
            template
        );
        let error = template
            .at_frame(&foreign, &Fx::default(), &[])
            .unwrap_err();
        assert!(
            matches!(&error, PlanError::Unresolved { kind: "node", id } if !id.is_empty()),
            "{error:?}"
        );
        assert_eq!(error.code(), "invalid_plan");
    }

    /// BL123: the template of a shot is cut to each frame, and the cut re-checks the splat matrices
    /// of *that* frame: a template made of a frame whose splat was a similarity does not let a frame
    /// whose splat is not one through, and a template never carries the refusal of one frame.
    #[test]
    fn a_frame_cut_from_a_template_is_checked_for_its_own_splat_matrices() {
        let mut raw: serde_json::Value = serde_json::from_str(RECIPES).unwrap();
        raw["assets"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "assetId": "cloud", "revisionId": "cloud-r1", "contentHash": "hash-cloud",
                "relativePath": "splats/cloud.spz", "format": "spz", "widthPx": 1, "heightPx": 1,
                "alphaMode": "opaque", "colorSpace": "srgb",
            }));
        let node = |scale: f64| {
            serde_json::json!({
                "id": "cloud-node", "parentId": "root", "type": "splat",
                "assetRef": { "assetId": "cloud", "revisionId": "cloud-r1", "contentHash": "hash-cloud",
                    "relativePath": "splats/cloud.spz" },
                "localPosition": { "x": 0, "y": 0, "z": 0 },
                "localRotation": { "x": 0, "y": 0, "z": 0, "w": 1 },
                "localScale": { "x": scale, "y": 1, "z": 1 },
            })
        };
        let scene_with = |scale: f64| {
            let mut doc = raw.clone();
            doc["sceneDefinitions"][0]["nodes"]
                .as_array_mut()
                .unwrap()
                .push(node(scale));
            DirectedScene::import_value(doc).unwrap()
        };
        let uniform = scene_with(1.0);
        let stretched = scene_with(2.0);
        let frame = uniform.bundle().evaluate_frame(0).unwrap();
        let stretched_frame = stretched.bundle().evaluate_frame(0).unwrap();
        let template = Prepared::new(&uniform, &frame)
            .unwrap()
            .assemble(&uniform, &frame)
            .unwrap();
        // The template is structurally valid and is not poisoned by the stretched scene's frame.
        let stretched_template = Prepared::new(&stretched, &stretched_frame)
            .unwrap()
            .assemble(&stretched, &stretched_frame)
            .expect("the template does not check the per-frame similarity");
        assert!(template.at_frame(&frame, &Fx::default(), &[]).is_ok());
        let error = template
            .at_frame(&stretched_frame, &Fx::default(), &[])
            .unwrap_err();
        assert!(
            matches!(&error, PlanError::SplatTransform { node_id, .. } if node_id == "cloud-node"),
            "{error:?}"
        );
        assert_eq!(error.code(), "unsupported");
        assert!(
            stretched_template
                .at_frame(&frame, &Fx::default(), &[])
                .is_ok()
        );
    }
}
