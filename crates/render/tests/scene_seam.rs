//! The host side of the scene renderer without a browser: resource identity, the book of runtime
//! slots, the bridge messages, and the renderer's behaviour against a scripted runtime (message
//! order, superseded frames, context loss, a runtime that goes away). What the pixels look like is
//! `scene_frames.rs`; what the runtime does with these messages is `runtime/test/directed-scene*`.

mod scene_support;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

use scene_support::{corpus, expected, plan, resources, scene};
use serde_json::{Value, json};
use tessel_editor_core::backend::BackendError;
use tessel_editor_core::bridge::{BridgeBackend, Transport};
use tessel_editor_core::scene_plan::Capability;
use tessel_editor_render::scene::{DIRECTED_LAYER_KIND, SceneFrameMessage, frame_render_message};
use tessel_editor_render::{
    FrameOptions, ResolvedResource, SceneRenderer, SceneResourceMap, SceneSlots,
};

fn code(result: Result<impl std::fmt::Debug, BackendError>) -> String {
    result.unwrap_err().code
}

// ---- resource identity ------------------------------------------------------------------------

#[test]
fn the_map_resolves_every_resource_of_the_plan_under_an_approved_root() {
    let scene = scene("bundle.json");
    let plan = plan(&scene, 0);
    let map = resources(7, &plan);
    assert_eq!((map.generation(), map.len()), (7, 3));
    for resource in &plan.resources {
        let entry = map.get(&resource.asset_id).unwrap();
        assert_eq!(entry.revision_id, resource.revision_id);
        assert_eq!(entry.content_hash, resource.content_hash);
        assert!(
            entry.path.is_absolute() && entry.path.is_file(),
            "{entry:?}"
        );
    }
}

#[test]
fn a_relative_path_and_a_missing_root_are_refused() {
    let mut map = SceneResourceMap::new(1);
    let error = map
        .insert(
            "sky",
            ResolvedResource {
                revision_id: "rev-1".into(),
                content_hash: "h".into(),
                path: "assets/sky.png".into(),
            },
        )
        .unwrap_err();
    assert_eq!(error.code, "resource_path_invalid");
    let plan = plan(&scene("bundle.json"), 0);
    let missing = SceneResourceMap::from_root(1, &plan, &corpus().join("nowhere"));
    assert_eq!(code(missing), "resource_unresolved");
}

#[cfg(unix)]
#[test]
fn a_link_that_leaves_the_root_is_refused() {
    let plan = plan(&scene("bundle.json"), 0);
    let root = std::env::temp_dir().join(format!("tessel-scene-seam-link-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("assets")).unwrap();
    // `sky` points outside the root; the other two are real copies.
    for name in ["mid.png", "hero.png"] {
        std::fs::copy(
            corpus().join("assets").join(name),
            root.join("assets").join(name),
        )
        .unwrap();
    }
    std::os::unix::fs::symlink(corpus().join("assets/sky.png"), root.join("assets/sky.png"))
        .unwrap();
    let result = SceneResourceMap::from_root(1, &plan, &root);
    let _ = std::fs::remove_dir_all(&root);
    let error = result.unwrap_err();
    assert_eq!(error.code, "resource_path_invalid");
    assert_eq!(error.details.unwrap()["asset_id"], "sky");
}

#[test]
fn a_map_in_json_round_trips_and_names_what_is_wrong() {
    let plan = plan(&scene("bundle.json"), 0);
    let map = resources(3, &plan);
    let mut text = json!({ "generation": 3, "resources": {} });
    for resource in &plan.resources {
        let entry = map.get(&resource.asset_id).unwrap();
        text["resources"][&resource.asset_id] = json!({
            "revisionId": entry.revision_id, "contentHash": entry.content_hash, "path": entry.path,
        });
    }
    assert_eq!(SceneResourceMap::from_json(&text).unwrap(), map);
    assert_eq!(
        code(SceneResourceMap::from_json(&json!({"resources": {}}))),
        "invalid_request"
    );
    let mut broken = text.clone();
    broken["resources"]["sky"]["contentHash"] = json!(5);
    let error = SceneResourceMap::from_json(&broken).unwrap_err();
    assert!(
        error.message.contains("sky") && error.message.contains("contentHash"),
        "{error:?}"
    );
}

// ---- the book of slots ------------------------------------------------------------------------

fn slot_names(prepared: &tessel_editor_render::scene::Prepared) -> Vec<String> {
    prepared
        .loads
        .iter()
        .map(|load| load.slot.clone())
        .collect()
}

fn confirm(slots: &mut SceneSlots, prepared: &tessel_editor_render::scene::Prepared) {
    for slot in prepared.pending_slots() {
        slots.mark_ready(slot);
    }
}

#[test]
fn a_second_frame_with_the_same_resources_loads_nothing() {
    let scene = scene("bundle.json");
    let mut slots = SceneSlots::default();
    let first_plan = plan(&scene, 0);
    let map = resources(1, &first_plan);
    let first = slots.prepare(&first_plan, &map).unwrap();
    assert_eq!(first.loads.len(), 3);
    assert!(first.releases.is_empty());
    // Until the runtime confirms them, a repeated request waits for them instead of loading twice.
    let again = slots.prepare(&first_plan, &map).unwrap();
    assert!(again.loads.is_empty());
    assert_eq!(again.waiting.len(), 3);
    confirm(&mut slots, &first);
    for frame in expected()["seekOrder"].as_array().unwrap() {
        let later = slots
            .prepare(&plan(&scene, frame.as_i64().unwrap()), &map)
            .unwrap();
        assert!(
            later.loads.is_empty() && later.waiting.is_empty(),
            "frame {frame}: {later:?}"
        );
        assert_eq!(later.layer["resources"].as_array().unwrap().len(), 3);
    }
    assert_eq!(slots.occupied(), 3);
}

#[test]
fn the_layer_carries_the_plan_and_where_each_resource_was_loaded() {
    let scene = scene("bundle.json");
    let plan = plan(&scene, 12);
    let mut slots = SceneSlots::default();
    let prepared = slots.prepare(&plan, &resources(4, &plan)).unwrap();
    let layer = &prepared.layer;
    assert_eq!(layer["kind"], DIRECTED_LAYER_KIND);
    assert_eq!(layer["scene_plan"], plan.to_json());
    for reference in layer["resources"].as_array().unwrap() {
        let asset_id = reference["asset_id"].as_str().unwrap();
        let resource = plan.resource(asset_id).unwrap();
        assert_eq!(reference["generation"], 4);
        assert_eq!(reference["revision_id"], resource.revision_id.as_str());
        assert_eq!(reference["content_hash"], resource.content_hash.as_str());
        let slot = reference["slot"].as_str().unwrap();
        assert!(slot.starts_with("scene-res-"), "{slot}");
        assert_eq!(
            slots.slot_of(4, asset_id, &resource.revision_id, &resource.content_hash),
            Some(slot)
        );
    }
    // Each load names the same slot and the identity the runtime binds it to.
    for load in &prepared.loads {
        let asset = &load.message["asset"];
        assert_eq!(load.message["type"], "asset.load");
        assert_eq!(
            (asset["id"].as_str(), asset["kind"].as_str()),
            (Some(load.slot.as_str()), Some("still"))
        );
        assert_eq!(asset["scene"]["generation"], 4);
        assert!(asset["source"]["path"].as_str().unwrap().ends_with(".png"));
    }
}

#[test]
fn the_same_asset_id_with_another_revision_gets_another_slot() {
    let first_scene = scene("bundle.json");
    let swap_scene = scene("bundle.swap.json");
    let (first_plan, swap_plan) = (plan(&first_scene, 0), plan(&swap_scene, 0));
    let mut slots = SceneSlots::default();
    let first = slots
        .prepare(&first_plan, &resources(1, &first_plan))
        .unwrap();
    confirm(&mut slots, &first);
    // Same generation (an in-place edit of one picture): the other pictures are reused and only
    // hero is loaded again, beside its old revision, never over it by asset id.
    let swapped = slots
        .prepare(&swap_plan, &resources(1, &swap_plan))
        .unwrap();
    assert_eq!(swapped.loads.len(), 1);
    let (old, new) = (
        first_plan.resource("hero").unwrap(),
        swap_plan.resource("hero").unwrap(),
    );
    assert_ne!(old.content_hash, new.content_hash);
    let old_slot = slots
        .slot_of(1, "hero", &old.revision_id, &old.content_hash)
        .map(String::from);
    let new_slot = slots
        .slot_of(1, "hero", &new.revision_id, &new.content_hash)
        .map(String::from);
    assert!(
        old_slot.is_some() && new_slot.is_some() && old_slot != new_slot,
        "{old_slot:?} {new_slot:?}"
    );
    assert_eq!(slot_names(&swapped), vec![new_slot.unwrap()]);
}

#[test]
fn a_newer_generation_releases_everything_of_the_older_one() {
    let scene = scene("bundle.json");
    let plan = plan(&scene, 0);
    let mut slots = SceneSlots::default();
    let first = slots.prepare(&plan, &resources(1, &plan)).unwrap();
    confirm(&mut slots, &first);
    let next = slots.prepare(&plan, &resources(2, &plan)).unwrap();
    assert_eq!(
        (next.releases.len(), next.loads.len(), slots.generation()),
        (3, 3, 2)
    );
    for release in &next.releases {
        assert_eq!(release["type"], "asset.release");
    }
    assert_eq!(
        slots.occupied(),
        3,
        "the old pictures are gone, not kept beside the new ones"
    );
    // The old generation does not come back.
    let stale = slots.prepare(&plan, &resources(1, &plan));
    assert_eq!(code(stale), "resource_stale");
}

#[test]
fn a_refused_plan_changes_nothing() {
    let scene = scene("bundle.json");
    let plan = plan(&scene, 0);
    let mut slots = SceneSlots::default();
    let first = slots.prepare(&plan, &resources(1, &plan)).unwrap();
    confirm(&mut slots, &first);
    let before = slots.occupied();

    // No entry for one asset.
    let mut missing = SceneResourceMap::new(2);
    for resource in plan
        .resources
        .iter()
        .filter(|resource| resource.asset_id != "mid")
    {
        let entry = resources(2, &plan).get(&resource.asset_id).unwrap().clone();
        missing.insert(resource.asset_id.clone(), entry).unwrap();
    }
    let error = slots.prepare(&plan, &missing).unwrap_err();
    assert_eq!(
        (
            error.code.as_str(),
            error.details.unwrap()["asset_id"].clone()
        ),
        ("resource_unresolved", json!("mid"))
    );

    // A file vouched for another revision than the plan requires.
    let mut wrong = resources(2, &plan);
    let hero = wrong.get("hero").unwrap().clone();
    wrong
        .insert(
            "hero",
            ResolvedResource {
                content_hash: "not-the-hash".into(),
                ..hero
            },
        )
        .unwrap();
    assert_eq!(code(slots.prepare(&plan, &wrong)), "resource_mismatch");

    // A file that is not there.
    let mut gone = resources(2, &plan);
    let sky = gone.get("sky").unwrap().clone();
    gone.insert(
        "sky",
        ResolvedResource {
            path: corpus().join("assets/no-such.png"),
            ..sky
        },
    )
    .unwrap();
    assert_eq!(code(slots.prepare(&plan, &gone)), "resource_unresolved");

    // Nothing of the three attempts reached the book: generation and slots are as they were, so
    // a correct request of the same generation still finds its pictures.
    assert_eq!((slots.occupied(), slots.generation()), (before, 1));
    let ok = slots.prepare(&plan, &resources(1, &plan)).unwrap();
    assert!(ok.loads.is_empty() && ok.releases.is_empty());
}

#[test]
fn a_partial_plan_is_refused_before_any_resource_is_looked_at() {
    let scene = scene("bundle.json");
    let mut plan = plan(&scene, 0);
    plan.unsupported
        .push(tessel_editor_core::scene_plan::UnsupportedDomain {
            domain: tessel_editor_core::scene_plan::Domain::ALL[0],
            subject: None,
            reason: None,
        });
    let mut slots = SceneSlots::default();
    // An empty map: if the resources were looked at first the error would be `resource_unresolved`.
    let error = slots.prepare(&plan, &SceneResourceMap::new(1)).unwrap_err();
    assert_eq!(error.code, "unsupported");
    assert_eq!(error.details.unwrap()["unsupported"], "scene_plan_domains");
    assert_eq!(slots.occupied(), 0);
}

#[test]
fn the_slots_stay_bounded_and_evict_what_the_frame_does_not_need() {
    let scene = scene("bundle.json");
    let swap_scene = scene_support::scene("bundle.swap.json");
    let (first_plan, swap_plan) = (plan(&scene, 0), plan(&swap_scene, 0));

    // Two slots cannot hold the three pictures of one frame: a typed budget error.
    let mut small = SceneSlots::new(2, u64::MAX);
    assert_eq!(
        code(small.prepare(&first_plan, &resources(1, &first_plan))),
        "resource_budget"
    );
    assert_eq!(small.occupied(), 0);

    // Three slots hold one frame; a swapped hero needs a fourth picture, so the oldest one the
    // frame no longer needs (the old hero) is released and its slot reused.
    let mut slots = SceneSlots::new(3, u64::MAX);
    let first = slots
        .prepare(&first_plan, &resources(1, &first_plan))
        .unwrap();
    confirm(&mut slots, &first);
    let swapped = slots
        .prepare(&swap_plan, &resources(1, &swap_plan))
        .unwrap();
    assert_eq!(
        (
            swapped.loads.len(),
            swapped.releases.len(),
            slots.occupied()
        ),
        (1, 1, 3)
    );
    let old = first_plan.resource("hero").unwrap();
    assert_eq!(
        slots.slot_of(1, "hero", &old.revision_id, &old.content_hash),
        None
    );

    // The byte budget counts decoded RGBA (64*36 + 48*48 + 32*32 pixels, 4 bytes each).
    let needed = (64 * 36 + 48 * 48 + 32 * 32) * 4;
    let mut tight = SceneSlots::new(8, needed - 1);
    assert_eq!(
        code(tight.prepare(&first_plan, &resources(1, &first_plan))),
        "resource_budget"
    );
    let mut exact = SceneSlots::new(8, needed);
    exact
        .prepare(&first_plan, &resources(1, &first_plan))
        .unwrap();
    // With only that budget the swapped hero (32*32*4 more bytes) displaces the old one.
    let displaced = exact
        .prepare(&swap_plan, &resources(1, &swap_plan))
        .unwrap();
    assert_eq!(displaced.releases.len(), 1);
}

#[test]
fn a_failed_load_is_forgotten_so_the_next_request_loads_it_again() {
    let scene = scene("bundle.json");
    let plan = plan(&scene, 0);
    let map = resources(1, &plan);
    let mut slots = SceneSlots::default();
    let first = slots.prepare(&plan, &map).unwrap();
    let failed = first.loads[0].slot.clone();
    slots.mark_failed(&failed);
    for load in first.loads.iter().skip(1) {
        slots.mark_ready(&load.slot);
    }
    let retry = slots.prepare(&plan, &map).unwrap();
    assert_eq!(retry.loads.len(), 1);
    // `release_all` empties the book and says which slots the runtime has to drop.
    slots.mark_ready(&retry.loads[0].slot);
    assert_eq!(slots.release_all().len(), 3);
    assert_eq!(slots.occupied(), 0);
    // `reset` is the runtime having gone: nothing to release, but the generation stays.
    slots.prepare(&plan, &map).unwrap();
    slots.reset();
    assert_eq!((slots.occupied(), slots.generation()), (0, 1));
}

#[test]
fn slot_ids_never_grow_with_the_number_of_projects_opened() {
    // WHY: bridge ids become interned `Id`s in the core and are never freed.
    let scene = scene("bundle.json");
    let plan = plan(&scene, 0);
    let mut slots = SceneSlots::default();
    let mut seen = std::collections::BTreeSet::new();
    for generation in 1..=200 {
        let prepared = slots.prepare(&plan, &resources(generation, &plan)).unwrap();
        seen.extend(slot_names(&prepared));
    }
    assert!(seen.len() <= 32, "{} distinct slot ids", seen.len());
}

// ---- messages ---------------------------------------------------------------------------------

#[test]
fn the_frame_message_is_the_ordinary_frame_render_with_one_directed_layer() {
    let scene = scene("bundle.json");
    let plan = plan(&scene, 12);
    let mut slots = SceneSlots::default();
    let prepared = slots.prepare(&plan, &resources(1, &plan)).unwrap();
    let message = frame_render_message(&SceneFrameMessage {
        id: 9,
        revision: 4,
        edit_frame: 12,
        width: 320,
        height: 180,
        layer: prepared.layer.clone(),
        png: Some("/tmp/x.png".into()),
        probes: vec![(1, 2)],
        inspect: true,
    });
    assert_eq!(
        (message["v"].clone(), message["type"].clone()),
        (json!(1), json!("frame.render"))
    );
    assert_eq!(
        (message["id"].clone(), message["revision"].clone()),
        (json!(9), json!(4))
    );
    assert_eq!(message["plan"]["canvas"]["width"], 320);
    assert_eq!(message["plan"]["layers"], json!([prepared.layer]));
    assert_eq!(
        message["output"],
        json!({"width": 320, "height": 180, "png": "/tmp/x.png"})
    );
    assert_eq!(message["probes"], json!([[1, 2]]));
    assert!(
        message.get("inspect").is_none(),
        "`inspect` is only sent when it is off"
    );
    let cheap = frame_render_message(&SceneFrameMessage {
        id: 1,
        revision: 1,
        edit_frame: 0,
        width: 4,
        height: 4,
        layer: json!({}),
        png: None,
        probes: vec![],
        inspect: false,
    });
    assert_eq!(cheap["inspect"], false);
}

// ---- the renderer against a scripted runtime -----------------------------------------------------

type Sent = Rc<RefCell<Vec<Value>>>;
type Responder = Box<dyn FnMut(&Value) -> Vec<Value>>;

/// A runtime that answers every message with `respond(message) -> replies`, in order.
struct Scripted {
    sent: Sent,
    queue: VecDeque<Value>,
    respond: Responder,
}

impl Transport for Scripted {
    fn send(&mut self, message: &Value) -> Result<(), BackendError> {
        self.sent.borrow_mut().push(message.clone());
        self.queue.extend((self.respond)(message));
        Ok(())
    }

    fn recv(&mut self, _: Duration) -> Option<Value> {
        self.queue.pop_front()
    }

    fn close(&mut self) {}
}

fn ready(features: &[&str]) -> Value {
    let capabilities: Vec<&str> = Capability::ALL
        .iter()
        .map(|capability| capability.as_str())
        .collect();
    json!({
        "v": 1, "type": "ready", "runtime": {"name": "scripted", "version": "0", "renderer": "none"},
        "capabilities": {
            "layers": ["directed_scene"], "formats": {}, "features": features,
            "directed_scene": {"layer_kind": "directed_scene", "capabilities": capabilities},
        },
    })
}

fn done(id: u64) -> Value {
    json!({"v": 1, "type": "frame.done", "id": id, "revision": id, "width": 320, "height": 180,
        "hash": "abcd", "mean": [1, 2, 3, 255], "probes": [], "png": null,
        "timings": {"render_ms": 1.0, "total_ms": 2.0}})
}

fn failed(id: u64, error_code: &str) -> Value {
    json!({"v": 1, "type": "frame.failed", "id": id, "revision": id,
        "error": {"code": error_code, "message": error_code}})
}

fn well_behaved(message: &Value) -> Vec<Value> {
    match message["type"].as_str() {
        Some("hello") => vec![ready(&["directed_scene"])],
        Some("asset.load") => vec![
            json!({"v": 1, "type": "asset.state", "id": message["asset"]["id"], "state": "ready", "meta": {}}),
        ],
        Some("frame.render") => vec![done(message["id"].as_u64().unwrap())],
        _ => vec![],
    }
}

fn renderer(
    respond: impl FnMut(&Value) -> Vec<Value> + 'static,
) -> (SceneRenderer<Scripted>, Sent) {
    renderer_with_slots(SceneSlots::default(), respond)
}

fn renderer_with_slots(
    slots: SceneSlots,
    respond: impl FnMut(&Value) -> Vec<Value> + 'static,
) -> (SceneRenderer<Scripted>, Sent) {
    let sent: Sent = Rc::default();
    let transport = Scripted {
        sent: Rc::clone(&sent),
        queue: VecDeque::new(),
        respond: Box::new(respond),
    };
    let backend = BridgeBackend::new("scripted", transport, Duration::from_secs(1));
    (
        SceneRenderer::with_slots(backend, slots).with_timeout(Duration::from_secs(1)),
        sent,
    )
}

fn types(sent: &Sent) -> Vec<String> {
    sent.borrow()
        .iter()
        .map(|message| message["type"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn the_renderer_loads_waits_and_then_draws_once_per_picture_set() {
    let scene = scene("bundle.json");
    let (mut renderer, sent) = renderer(well_behaved);
    let first_plan = plan(&scene, 0);
    let map = resources(1, &first_plan);
    let first = renderer
        .render(&first_plan, &map, &FrameOptions::default())
        .unwrap();
    assert_eq!(first.hash, "abcd");
    assert_eq!(
        types(&sent),
        [
            "hello",
            "asset.load",
            "asset.load",
            "asset.load",
            "frame.render"
        ]
    );
    // The frame is requested with the plan's own canvas size, after the loads.
    let render = sent.borrow().last().unwrap().clone();
    assert_eq!(
        (
            render["output"]["width"].clone(),
            render["output"]["height"].clone()
        ),
        (json!(320), json!(180))
    );
    // The second frame finds the pictures already there.
    renderer
        .render(&plan(&scene, 48), &map, &FrameOptions::default())
        .unwrap();
    assert_eq!(sent.borrow().len(), 6);
    assert_eq!(
        types(&sent).last().map(String::as_str),
        Some("frame.render")
    );
    // Frame ids grow, so the runtime's latest-frame rule is never confused by a repeat.
    let ids: Vec<u64> = sent
        .borrow()
        .iter()
        .filter(|m| m["type"] == "frame.render")
        .map(|m| m["id"].as_u64().unwrap())
        .collect();
    assert_eq!(ids, vec![1, 2]);
    renderer.release_all().unwrap();
    assert_eq!(
        types(&sent)
            .iter()
            .filter(|kind| *kind == "asset.release")
            .count(),
        3
    );
}

#[test]
fn a_runtime_without_the_feature_or_a_capability_is_refused_before_a_frame() {
    let scene = scene("bundle.json");
    let first_plan = plan(&scene, 0);
    let map = resources(1, &first_plan);

    let (mut old, sent) = renderer(|message| match message["type"].as_str() {
        Some("hello") => vec![ready(&[])],
        _ => vec![],
    });
    let error = old
        .render(&first_plan, &map, &FrameOptions::default())
        .unwrap_err();
    assert_eq!(error.code, "backend_unavailable");
    assert!(
        error.message.contains("pnpm --dir runtime run build"),
        "{error:?}"
    );
    assert_eq!(types(&sent), ["hello"], "no asset or frame was sent to it");

    // A runtime that lists the feature but not every capability the plan requires.
    let (mut narrow, sent) = renderer(|message| match message["type"].as_str() {
        Some("hello") => {
            let mut ready = ready(&["directed_scene"]);
            ready["capabilities"]["directed_scene"]["capabilities"] = json!(["camera"]);
            vec![ready]
        }
        _ => vec![],
    });
    let error = narrow
        .render(&first_plan, &map, &FrameOptions::default())
        .unwrap_err();
    assert_eq!(error.code, "unsupported");
    assert_eq!(
        error.details.unwrap()["unsupported"],
        "scene_plan_capabilities"
    );
    assert_eq!(types(&sent), ["hello"]);
}

/// F060: a runtime that lists every capability but `pointLights` is refused before any asset or
/// frame is sent when a light is visible, and is served when the only light is hidden (the plan
/// then does not require it) or there is none: negotiated like `outputFx`.
#[test]
fn a_runtime_without_point_lights_is_refused_only_for_a_visible_light() {
    let lit = |hidden: bool| {
        let mut bundle: Value =
            serde_json::from_str(&std::fs::read_to_string(corpus().join("bundle.json")).unwrap())
                .unwrap();
        bundle["sceneDefinitions"][0]["lighting"] = json!({"ambient": 1, "lights": [{
            "id": "lamp", "position": {"x": 0.0, "y": 1.0, "z": 2.5},
            "color": {"r": 1.0, "g": 1.0, "b": 1.0}, "intensity": 2.0, "radius": 4.0,
            "flicker": 0.0, "hidden": hidden,
        }]});
        tessel_editor_render::scene::import_gated(&bundle.to_string()).unwrap()
    };
    let without_point_lights = |message: &Value| match message["type"].as_str() {
        Some("hello") => {
            let mut ready = ready(&["directed_scene"]);
            ready["capabilities"]["directed_scene"]["capabilities"] = json!(
                Capability::ALL
                    .iter()
                    .filter(|capability| **capability != Capability::PointLights)
                    .map(|capability| capability.as_str())
                    .collect::<Vec<_>>()
            );
            vec![ready]
        }
        _ => well_behaved(message),
    };

    let visible = plan(&lit(false), 0);
    assert!(visible.requires.contains(&Capability::PointLights));
    let (mut runtime, sent) = renderer(without_point_lights);
    let error = runtime
        .render(&visible, &resources(1, &visible), &FrameOptions::default())
        .unwrap_err();
    assert_eq!(error.code, "unsupported");
    let details = error.details.unwrap();
    assert_eq!(details["unsupported"], "scene_plan_capabilities");
    assert!(details.to_string().contains("pointLights"), "{details}");
    assert_eq!(types(&sent), ["hello"], "nothing else was sent to it");

    // Hidden: the plan does not ask for the capability, so the same runtime is not refused for it.
    let hidden = plan(&lit(true), 0);
    assert!(!hidden.requires.contains(&Capability::PointLights));
    let (mut runtime, sent) = renderer(without_point_lights);
    runtime
        .render(&hidden, &resources(1, &hidden), &FrameOptions::default())
        .expect("a hidden light does not get the plan refused");
    assert_eq!(
        types(&sent).last().map(String::as_str),
        Some("frame.render")
    );
}

#[test]
fn a_picture_the_runtime_refuses_is_reported_and_loaded_again_next_time() {
    let scene = scene("bundle.json");
    let first_plan = plan(&scene, 0);
    let map = resources(1, &first_plan);
    let refuse_once = Rc::new(RefCell::new(true));
    let flag = Rc::clone(&refuse_once);
    let (mut renderer, sent) = renderer(move |message| {
        if message["type"] == "asset.load"
            && message["asset"]["scene"]["asset_id"] == "mid"
            && flag.replace(false)
        {
            return vec![
                json!({"v": 1, "type": "asset.state", "id": message["asset"]["id"], "state": "failed",
                "error": {"code": "resource_mismatch", "message": "other hash"}}),
            ];
        }
        well_behaved(message)
    });
    let error = renderer
        .render(&first_plan, &map, &FrameOptions::default())
        .unwrap_err();
    assert_eq!(error.code, "resource_mismatch");
    assert!(
        !types(&sent).contains(&"frame.render".to_string()),
        "no frame was drawn without its picture"
    );
    renderer
        .render(&first_plan, &map, &FrameOptions::default())
        .unwrap();
    let loads = types(&sent)
        .iter()
        .filter(|kind| *kind == "asset.load")
        .count();
    assert_eq!(
        loads, 4,
        "three pictures, then the refused one again and nothing else"
    );
}

#[test]
fn a_frame_lost_with_the_context_is_sent_again_once_the_context_is_back() {
    let scene = scene("bundle.json");
    let first_plan = plan(&scene, 0);
    let map = resources(1, &first_plan);
    let mut frames = 0;
    let (mut renderer, sent) = renderer(move |message| {
        if message["type"] != "frame.render" {
            return well_behaved(message);
        }
        frames += 1;
        let id = message["id"].as_u64().unwrap();
        match frames {
            1 => vec![
                json!({"v": 1, "type": "context.lost"}),
                failed(id, "context_lost"),
                json!({"v": 1, "type": "context.restored"}),
            ],
            _ => vec![done(id)],
        }
    });
    let result = renderer
        .render(&first_plan, &map, &FrameOptions::default())
        .unwrap();
    assert_eq!(result.id, 2, "the answer is the second request's");
    // The pictures survive the context (the runtime keeps the decoded images): only the frame is resent.
    assert_eq!(
        types(&sent),
        [
            "hello",
            "asset.load",
            "asset.load",
            "asset.load",
            "frame.render",
            "frame.render"
        ]
    );
}

#[test]
fn a_context_that_keeps_getting_lost_is_given_up_on_and_a_cancelled_frame_is_never_retried() {
    let scene = scene("bundle.json");
    let first_plan = plan(&scene, 0);
    let map = resources(1, &first_plan);
    let (mut always_lost, sent) = renderer(|message| {
        if message["type"] != "frame.render" {
            return well_behaved(message);
        }
        let id = message["id"].as_u64().unwrap();
        vec![
            json!({"v": 1, "type": "context.lost"}),
            failed(id, "context_lost"),
            json!({"v": 1, "type": "context.restored"}),
        ]
    });
    assert_eq!(
        code(always_lost.render(&first_plan, &map, &FrameOptions::default())),
        "context_lost"
    );
    assert_eq!(
        types(&sent)
            .iter()
            .filter(|kind| *kind == "frame.render")
            .count(),
        3,
        "one try and two retries"
    );

    let (mut superseded, sent) = renderer(|message| {
        if message["type"] != "frame.render" {
            return well_behaved(message);
        }
        vec![failed(message["id"].as_u64().unwrap(), "cancelled")]
    });
    assert_eq!(
        code(superseded.render(&first_plan, &map, &FrameOptions::default())),
        "cancelled"
    );
    assert_eq!(
        types(&sent)
            .iter()
            .filter(|kind| *kind == "frame.render")
            .count(),
        1
    );
}

#[test]
fn submitted_frames_are_answered_by_id_in_any_order_and_a_cancel_reaches_the_runtime() {
    let scene = scene("bundle.json");
    let first_plan = plan(&scene, 0);
    let map = resources(1, &first_plan);
    // The runtime answers a burst the way the real one does: the newest frame wins.
    let mut pending: Vec<u64> = Vec::new();
    let (mut renderer, sent) = renderer(move |message| match message["type"].as_str() {
        Some("frame.render") => {
            let id = message["id"].as_u64().unwrap();
            let superseded: Vec<Value> = pending
                .drain(..)
                .map(|old| failed(old, "cancelled"))
                .collect();
            pending.push(id);
            let mut replies = superseded;
            replies.push(done(id));
            replies
        }
        _ => well_behaved(message),
    });
    let frames: Vec<_> = [0, 12, 48]
        .iter()
        .map(|frame| {
            renderer
                .submit(&plan(&scene, *frame), &map, &FrameOptions::default())
                .unwrap()
        })
        .collect();
    // Wait for the newest first: the older answers stay until asked for.
    assert!(renderer.wait(frames[2]).is_ok());
    assert_eq!(code(renderer.wait(frames[0])), "cancelled");
    assert_eq!(code(renderer.wait(frames[1])), "cancelled");
    renderer.cancel(frames[2]).unwrap();
    let cancel = sent.borrow().last().unwrap().clone();
    assert_eq!(
        (cancel["type"].clone(), cancel["id"].clone()),
        (json!("frame.cancel"), json!(frames[2].id))
    );
}

#[test]
fn a_runtime_that_goes_away_empties_the_book_and_says_so() {
    let scene = scene("bundle.json");
    let first_plan = plan(&scene, 0);
    let map = resources(1, &first_plan);
    let (mut renderer, _) = renderer(|message| match message["type"].as_str() {
        Some("frame.render") => {
            vec![json!({"v": 1, "type": "closed", "reason": "The page crashed"})]
        }
        _ => well_behaved(message),
    });
    let error = renderer
        .render(&first_plan, &map, &FrameOptions::default())
        .unwrap_err();
    assert_eq!(error.code, "backend_unavailable");
    assert!(error.message.contains("The page crashed"), "{error:?}");
    assert_eq!(
        renderer.slots().occupied(),
        0,
        "what the runtime held is gone with it"
    );
}

#[test]
fn a_silent_runtime_times_out_with_what_it_was_asked() {
    let scene = scene("bundle.json");
    let first_plan = plan(&scene, 0);
    let map = resources(1, &first_plan);
    let (mut renderer, _) = renderer(|message| match message["type"].as_str() {
        Some("hello") => vec![ready(&["directed_scene"])],
        _ => vec![],
    });
    let error = renderer
        .render(&first_plan, &map, &FrameOptions::default())
        .unwrap_err();
    assert_eq!(error.code, "timeout");
    assert!(error.message.contains("pictures"), "{error:?}");
}

// ---- load identity (BL102b) -----------------------------------------------------------------

/// What a runtime that negotiated `asset.identity` answers to `load`: the `ready` of exactly that
/// request, with the request and the scene identity repeated and nothing else.
fn echoed_ready(load: &Value) -> Value {
    let mut state = json!({"v": 1, "type": "asset.state", "id": load["asset"]["id"],
        "state": "ready", "meta": {}});
    state["request"] = load["asset"]["request"].clone();
    state["scene"] = load["asset"]["scene"].clone();
    state
}

fn identified(message: &Value) -> Vec<Value> {
    match message["type"].as_str() {
        Some("hello") => vec![ready(&["directed_scene", "asset.identity"])],
        Some("asset.load") => vec![echoed_ready(message)],
        Some("frame.render") => vec![done(message["id"].as_u64().unwrap())],
        _ => vec![],
    }
}

fn loads(sent: &Sent) -> Vec<Value> {
    sent.borrow()
        .iter()
        .filter(|message| message["type"] == "asset.load")
        .cloned()
        .collect()
}

#[test]
fn a_negotiated_runtime_is_sent_one_new_request_per_load_and_its_echo_unlocks_the_frame() {
    let scene = scene("bundle.json");
    let first_plan = plan(&scene, 0);
    let map = resources(1, &first_plan);
    let (mut renderer, sent) = renderer(identified);
    renderer
        .render(&first_plan, &map, &FrameOptions::default())
        .unwrap();
    assert!(renderer.asset_identity());
    assert_eq!(renderer.refused_asset_states(), 0);
    let requests: Vec<u64> = loads(&sent)
        .iter()
        .map(|load| load["asset"]["request"].as_u64().unwrap())
        .collect();
    assert_eq!(requests, vec![1, 2, 3], "one number per load, never reused");
    // The frame was only asked for after the three echoes.
    assert_eq!(
        types(&sent).last().map(String::as_str),
        Some("frame.render")
    );
}

#[test]
fn a_runtime_that_did_not_list_the_feature_is_sent_nothing_new_and_judged_by_slot_alone() {
    let scene = scene("bundle.json");
    let first_plan = plan(&scene, 0);
    let map = resources(1, &first_plan);
    // The old runtime answers with the slot only: no echo, and that still unlocks the frame.
    let (mut renderer, sent) = renderer(well_behaved);
    renderer
        .render(&first_plan, &map, &FrameOptions::default())
        .unwrap();
    assert!(!renderer.asset_identity());
    assert_eq!(renderer.refused_asset_states(), 0);
    for load in loads(&sent) {
        assert!(load["asset"].get("request").is_none(), "{load}");
    }
}

#[test]
fn a_late_ready_of_an_earlier_load_in_the_same_slot_never_confirms_the_new_one() {
    let scene = scene("bundle.json");
    let first_plan = plan(&scene, 0);
    let first_map = resources(1, &first_plan);
    // Generation 1 is never answered (the page is busy), then the project is reopened as
    // generation 2: the slots are given to new loads and only then do the old answers arrive.
    let held: Rc<RefCell<Vec<Value>>> = Rc::default();
    let stale_held = Rc::clone(&held);
    let deliver_new = Rc::new(RefCell::new(false));
    let new_answers = Rc::clone(&deliver_new);
    // Exactly as many slots as the plan has pictures: the new loads land in the very slots of the
    // old ones, which is the only way the name of a slot says nothing about which load it answers.
    let (mut renderer, sent) = renderer_with_slots(SceneSlots::new(3, u64::MAX), move |message| {
        match message["type"].as_str() {
            Some("asset.load") if message["asset"]["scene"]["generation"] == 1 => {
                stale_held.borrow_mut().push(echoed_ready(message));
                vec![]
            }
            Some("asset.load") => {
                // The old, correctly echoed `ready`s come first; the answer to this very load, only
                // when the test lets it.
                let mut replies: Vec<Value> = stale_held.borrow_mut().drain(..).collect();
                if *new_answers.borrow() {
                    replies.push(echoed_ready(message));
                }
                replies
            }
            _ => identified(message),
        }
    });
    renderer = renderer.with_timeout(Duration::from_millis(200));
    let error = renderer
        .render(&first_plan, &first_map, &FrameOptions::default())
        .unwrap_err();
    assert_eq!(error.code, "timeout");

    let second_map = resources(2, &first_plan);
    let error = renderer
        .render(&first_plan, &second_map, &FrameOptions::default())
        .unwrap_err();
    assert_eq!(error.code, "timeout", "no answer to a new load, no frame");
    assert!(
        !types(&sent).contains(&"frame.render".to_string()),
        "a late ready of the replaced loads released a frame with the wrong pictures"
    );
    assert!(renderer.refused_asset_states() >= 1);
    let refused = renderer.refused_asset_states();

    // The same situation where the new loads are answered: the frame is drawn, once.
    *deliver_new.borrow_mut() = true;
    let third_map = resources(3, &first_plan);
    renderer
        .render(&first_plan, &third_map, &FrameOptions::default())
        .unwrap();
    assert_eq!(renderer.refused_asset_states(), refused);
    assert_eq!(
        types(&sent)
            .iter()
            .filter(|kind| *kind == "frame.render")
            .count(),
        1
    );
}

#[test]
fn an_echo_of_another_request_or_picture_or_none_at_all_is_refused_when_negotiated() {
    let scene = scene("bundle.json");
    let first_plan = plan(&scene, 0);
    let map = resources(1, &first_plan);
    let wrong: [fn(&mut Value); 4] = [
        |state| state["request"] = json!(999),
        |state| state["scene"]["content_hash"] = json!("other"),
        |state| state["scene"]["generation"] = json!(77),
        |state| {
            state.as_object_mut().unwrap().remove("request");
            state.as_object_mut().unwrap().remove("scene");
        },
    ];
    for (index, corrupt) in wrong.into_iter().enumerate() {
        let (mut renderer, sent) = renderer(move |message| {
            let mut replies = identified(message);
            if message["type"] == "asset.load" {
                corrupt(&mut replies[0]);
            }
            replies
        });
        renderer = renderer.with_timeout(Duration::from_millis(100));
        let error = renderer
            .render(&first_plan, &map, &FrameOptions::default())
            .unwrap_err();
        assert_eq!(error.code, "timeout", "case {index}");
        assert_eq!(renderer.refused_asset_states(), 3, "case {index}");
        assert!(
            !types(&sent).contains(&"frame.render".to_string()),
            "case {index}"
        );
    }
}

#[test]
fn a_failure_is_real_only_with_the_right_echo_and_a_stale_cancelled_does_not_fail_the_new_load() {
    let scene = scene("bundle.json");
    let first_plan = plan(&scene, 0);
    let map = resources(1, &first_plan);
    let (mut renderer, _) = renderer(|message| {
        let mut replies = identified(message);
        if message["type"] == "asset.load" && message["asset"]["scene"]["asset_id"] == "mid" {
            // The `cancelled` of an earlier load of the slot, then the real failure of this one.
            let mut stale = echoed_ready(message);
            stale["state"] = json!("failed");
            stale["request"] = json!(message["asset"]["request"].as_u64().unwrap() + 100);
            stale["error"] = json!({"code": "cancelled", "message": "superseded"});
            let mut real = echoed_ready(message);
            real["state"] = json!("failed");
            real["error"] = json!({"code": "resource_mismatch", "message": "other hash"});
            replies = vec![stale, real];
        }
        replies
    });
    let error = renderer
        .render(&first_plan, &map, &FrameOptions::default())
        .unwrap_err();
    assert_eq!(
        error.code, "resource_mismatch",
        "the stale one is not the answer"
    );
    assert_eq!(renderer.refused_asset_states(), 1);
}

#[test]
fn a_restarted_runtime_keeps_counting_requests_so_an_old_process_cannot_match_a_new_load() {
    let scene = scene("bundle.json");
    let first_plan = plan(&scene, 0);
    let map = resources(1, &first_plan);
    let (mut renderer, sent) = renderer(identified);
    renderer
        .render(&first_plan, &map, &FrameOptions::default())
        .unwrap();
    renderer.shutdown();
    renderer
        .render(&first_plan, &map, &FrameOptions::default())
        .unwrap();
    let requests: Vec<u64> = loads(&sent)
        .iter()
        .map(|load| load["asset"]["request"].as_u64().unwrap())
        .collect();
    assert_eq!(requests, vec![1, 2, 3, 4, 5, 6]);
}
