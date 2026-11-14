//! M3 and M4 (T08, T09, T28 plan): `ScenePlan.transition`, its wire form, the resources of two
//! decks and the capability. Expected numbers are written by hand (see `stages.rs`).

use core::prelude::v1::test;

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::directed_scene::fx::DeckSide;
use tessel_editor_core::scene_plan::{Capability, DeckRole, PlanError, PlanTimeline, ScenePlan};

use crate::support::*;

fn window_scene() -> DirectedScene {
    spec_scene(vec![tr(json!({}))])
}

/// A plan of the spec edit inside the window, as the tree a runtime would be handed.
fn window_json() -> Value {
    plan_at(&window_scene(), 44).to_json()
}

fn pointer_of(error: PlanError) -> String {
    match error {
        PlanError::Invalid { pointer, .. } => pointer,
        other => panic!("expected an invalid plan, got {other:?}"),
    }
}

fn refused(value: &Value) -> String {
    pointer_of(ScenePlan::from_json(value).expect_err("the plan should be refused"))
}

/// `value` with `pointer` set; the last member of an object is added when it is not there yet.
fn with(mut value: Value, pointer: &str, new: Value) -> Value {
    if let Some(slot) = value.pointer_mut(pointer) {
        *slot = new;
        return value;
    }
    let (parent, key) = pointer.rsplit_once('/').expect("a pointer");
    value
        .pointer_mut(parent)
        .and_then(Value::as_object_mut)
        .unwrap_or_else(|| panic!("no object at {parent}"))
        .insert(key.to_string(), new);
    value
}

#[test]
fn a_frame_outside_the_window_is_the_plan_it_always_was_byte_for_byte() {
    let with_transition = window_scene();
    let without = spec_scene(vec![]);
    for frame in [0, 20, 41, 54, 55, 100, 143] {
        let plan = plan_at(&with_transition, frame);
        assert!(plan.transition.is_none(), "{frame}");
        assert!(plan.to_json().get("transition").is_none(), "{frame}");
        assert_eq!(
            plan.to_json_string(),
            plan_at(&without, frame).to_json_string(),
            "frame {frame}"
        );
        assert!(!plan.requires.contains(&Capability::FxTransition));
    }
    // A hidden transition is not in the document's effect: even inside its window nothing changes.
    let hidden = spec_scene(vec![tr(json!({"hidden": true}))]);
    for frame in [42, 44, 48, 50, 53] {
        assert_eq!(
            plan_at(&hidden, frame).to_json_string(),
            plan_at(&without, frame).to_json_string(),
            "hidden, frame {frame}"
        );
    }
}

#[test]
fn a_window_frame_carries_the_neighbour_the_mix_and_the_capability() {
    let scene = window_scene();
    // Frame 44, before the cut: the primary is s1 (inst1 from 20, cam1), the neighbour s2 in its
    // head, both clocks held at zero. k = 2: t = 2.5 / 12, progress = 4 t^3.
    let plan = plan_at(&scene, 44);
    assert_eq!(plan.frame.shot_id, "s1");
    assert_eq!(
        (plan.frame.instance_frame, plan.frame.camera_clip_frame),
        (64, 44)
    );
    let t = plan.transition.as_deref().expect("a window frame");
    assert_eq!((t.id.as_str(), t.effect.as_str()), ("tr", "crossDissolve"));
    assert_eq!(
        (t.from_shot_id.as_str(), t.to_shot_id.as_str()),
        ("s1", "s2")
    );
    assert_eq!(t.primary, DeckSide::From);
    assert_eq!(
        (t.start_frame, t.duration_frames, t.frame_in_window),
        (42, 12, 2)
    );
    assert_eq!(t.easing.name(), "easeInOutCubic");
    assert!((t.progress - 4.0 * (2.5_f64 / 12.0).powi(3)).abs() < 1e-15);
    assert_eq!(t.amount, 1.0);
    assert_eq!(
        t.params
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["curve", "intensity"]
    );
    assert!(t.held_instance && t.held_camera);
    assert_eq!(t.other.frame.shot_id, "s2");
    assert_eq!(
        (
            t.other.frame.instance_frame,
            t.other.frame.camera_clip_frame
        ),
        (0, 0)
    );
    // The scene clock of BOTH decks is the real edit frame 44 = 11/6 s.
    for deck in [&plan, &t.other] {
        assert_eq!(deck.frame.edit_frame, 44);
        assert_eq!(
            (deck.frame.edit_time.num(), deck.frame.edit_time.den()),
            (11, 6)
        );
    }
    assert!(t.other.transition.is_none(), "depth 1");
    assert!(plan.requires.contains(&Capability::FxTransition));
    assert!(!t.other.requires.contains(&Capability::FxTransition));
    assert!(plan.is_complete());

    // Frame 50, after the cut: the primary is s2, the neighbour s1 in its tail, the camera held at
    // its last frame (cam1 has 48 frames), the instance still running (20 + 50).
    let plan = plan_at(&scene, 50);
    assert_eq!(plan.frame.shot_id, "s2");
    let t = plan.transition.as_deref().unwrap();
    assert_eq!((t.primary, t.frame_in_window), (DeckSide::To, 8));
    assert_eq!(
        (
            t.other.frame.instance_frame,
            t.other.frame.camera_clip_frame
        ),
        (70, 47)
    );
    assert!(!t.held_instance && t.held_camera);
}

#[test]
fn the_wire_form_has_the_documented_keys_round_trips_and_is_deterministic() {
    let plan = plan_at(&window_scene(), 44);
    let value = plan.to_json();
    let transition = value["transition"].as_object().expect("the key is present");
    let mut keys: Vec<&str> = transition.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "amount",
            "durationFrames",
            "easing",
            "effect",
            "frameInWindow",
            "fromShotId",
            "heldCamera",
            "heldInstance",
            "id",
            "other",
            "params",
            "primary",
            "progress",
            "startFrame",
            "toShotId"
        ]
    );
    // An ordered array of pairs, never an object.
    assert_eq!(
        value["transition"]["params"],
        json!([["curve", "smooth"], ["intensity", 1.0]])
    );
    assert_eq!(value["transition"]["primary"], "from");
    assert_eq!(value["transition"]["other"]["frame"]["shotId"], "s2");
    assert!(value["transition"]["other"].get("transition").is_none());
    // Lossless in both doors, and the text is stable.
    let text = plan.to_json_string();
    assert_eq!(text, plan.to_json_string());
    assert_eq!(ScenePlan::from_json_str(&text).unwrap(), plan);
    assert_eq!(ScenePlan::from_json(&value).unwrap(), plan);
    let again = ScenePlan::from_json_str(&text).unwrap().to_json_string();
    assert_eq!(again, text);
    // The pretty form reads back to the same plan.
    assert_eq!(
        ScenePlan::from_json_str(&plan.to_json_pretty()).unwrap(),
        plan
    );
}

#[test]
fn the_planned_frames_do_not_depend_on_the_order_they_are_asked_in() {
    let scene = spec_scene(vec![
        tr(json!({})),
        tr(json!({"id": "tr-2", "fromShotId": "s2", "toShotId": "s3", "alignment": "start"})),
    ]);
    let whole: Vec<String> = (0..144)
        .map(|frame| plan_at(&scene, frame).to_json_string())
        .collect();
    assert!(
        whole
            .iter()
            .filter(|text| text.contains("\"transition\""))
            .count()
            >= 24
    );
    let timeline = PlanTimeline::new(&scene);
    // Reverse, then a strided subinterval, through the timeline: the same bytes as a cold build.
    for frame in (0..144).rev() {
        assert_eq!(
            timeline.build(frame).unwrap().to_json_string(),
            whole[frame as usize],
            "timeline, frame {frame}"
        );
    }
    for frame in (38..110).step_by(7).chain([54, 42, 44, 44, 96, 95]) {
        assert_eq!(
            plan_at(&scene, frame).to_json_string(),
            whole[frame as usize],
            "frame {frame}"
        );
    }
}

#[test]
fn each_deck_keeps_its_own_resources_and_the_union_is_derived_with_qualified_users() {
    let plan = plan_at(&window_scene(), 44);
    let t = plan.transition.as_deref().unwrap();
    // scene-a and scene-b both use `backdrop` and `figure`, under the same node ids.
    let ids = |deck: &ScenePlan| -> Vec<String> {
        deck.resources
            .iter()
            .map(|resource| resource.asset_id.clone())
            .collect()
    };
    assert_eq!(ids(&plan), ["backdrop", "figure"]);
    assert_eq!(ids(&t.other), ["backdrop", "figure"]);
    let union = plan.resource_union().unwrap();
    assert_eq!(
        union.keys().map(String::as_str).collect::<Vec<_>>(),
        ["backdrop", "figure"]
    );
    for (asset, node) in [("backdrop", "backdrop"), ("figure", "figure")] {
        let users: Vec<(DeckRole, &str)> = union[asset]
            .used_by
            .iter()
            .map(|user| (user.deck, user.node_id.as_str()))
            .collect();
        assert_eq!(
            users,
            [(DeckRole::Primary, node), (DeckRole::Other, node)],
            "{asset}"
        );
    }
    assert_eq!(plan.decks().len(), 2);
    assert_eq!(plan_at(&window_scene(), 20).decks().len(), 1);
    // The wire form lists each deck's own resources, not the union.
    let value = plan.to_json();
    assert_eq!(value["resources"].as_array().unwrap().len(), 2);
    assert_eq!(
        value["transition"]["other"]["resources"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(value.get("resourceUnion").is_none());
}

fn scene_with_a_neighbour_only_asset() -> DirectedScene {
    let mut bundle = with_transitions(Edit::spec().bundle(), vec![tr(json!({}))]);
    bundle["assets"].as_array_mut().unwrap().push(json!({
        "assetId": "sky", "revisionId": "rev-1", "contentHash": "synthetic-sha256-sky",
        "relativePath": "assets/sky.png", "widthPx": 8, "heightPx": 8,
        "alphaMode": "opaque", "colorSpace": "srgb",
    }));
    // inst2 plays scene-b: the incoming shot s2 is the neighbour of the head frames.
    let scene_b = bundle["sceneDefinitions"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|definition| definition["id"] == "scene-b")
        .unwrap();
    scene_b["nodes"].as_array_mut().unwrap().push(json!({
        "id": "sky-plane", "parentId": "root", "type": "imagePlane",
        "assetRef": {"assetId": "sky", "revisionId": "rev-1",
                     "contentHash": "synthetic-sha256-sky", "relativePath": "assets/sky.png"},
        "widthSceneUnits": 8, "heightSceneUnits": 4.5, "renderOrder": 5,
        "localPosition": {"x": 0, "y": 0, "z": -3},
        "localRotation": {"x": 0, "y": 0, "z": 0, "w": 1},
        "localScale": {"x": 1, "y": 1, "z": 1},
    }));
    scene_of(bundle)
}

#[test]
fn an_asset_only_the_neighbour_uses_is_in_the_union_and_not_in_the_primary_list() {
    let scene = scene_with_a_neighbour_only_asset();
    let plan = plan_at(&scene, 44);
    assert_eq!(plan.frame.shot_id, "s1");
    assert!(
        plan.resource("sky").is_none(),
        "the primary deck does not use it"
    );
    let t = plan.transition.as_deref().unwrap();
    assert!(t.other.resource("sky").is_some());
    let union = plan.resource_union().unwrap();
    assert_eq!(
        union.keys().map(String::as_str).collect::<Vec<_>>(),
        ["backdrop", "figure", "sky"]
    );
    let sky = &union["sky"];
    assert_eq!(sky.used_by.len(), 1);
    assert_eq!(
        (sky.used_by[0].deck, sky.used_by[0].node_id.as_str()),
        (DeckRole::Other, "sky-plane")
    );
    // After the cut the roles swap: the primary is s2 and uses it, the neighbour does not.
    let after = plan_at(&scene, 50);
    assert!(after.resource("sky").is_some());
    assert!(
        after
            .transition
            .as_deref()
            .unwrap()
            .other
            .resource("sky")
            .is_none()
    );
    assert_eq!(
        after.resource_union().unwrap()["sky"].used_by[0].deck,
        DeckRole::Primary
    );
    // The plan survives its own wire form with the extra asset on one side only.
    assert_eq!(
        ScenePlan::from_json_str(&plan.to_json_string()).unwrap(),
        plan
    );
}

#[test]
fn the_same_asset_id_with_other_content_in_the_neighbour_is_invalid_there() {
    let value = window_json();
    for (field, replacement) in [
        ("contentHash", json!("other-hash")),
        ("revisionId", json!("rev-2")),
        ("relativePath", json!("assets/other.png")),
    ] {
        let tampered = with(
            value.clone(),
            &format!("/transition/other/resources/0/{field}"),
            replacement,
        );
        let pointer = refused(&tampered);
        assert!(
            pointer.starts_with("/transition/other/resources"),
            "{field}: {pointer}"
        );
    }
    // A hand-built plan is judged by the same rule.
    let mut plan = plan_at(&window_scene(), 44);
    plan.transition.as_mut().unwrap().other.resources[1].content_hash = "different".into();
    assert!(plan.resource_union().is_err());
}

/// The lamp lives only in scene-b: the head frames of the incoming shot need `pointLights` only
/// through the neighbour, and the plan must say so.
#[test]
fn the_requirement_is_the_union_of_both_decks_plus_the_transition() {
    let mut bundle = with_transitions(Edit::spec().bundle(), vec![tr(json!({}))]);
    let scene_b = bundle["sceneDefinitions"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|definition| definition["id"] == "scene-b")
        .unwrap();
    scene_b["lighting"] = json!({"ambient": 1, "lights": [{
        "id": "lamp", "position": {"x": 0, "y": 1, "z": 2},
        "color": {"r": 1, "g": 1, "b": 1}, "intensity": 2, "radius": 5, "flicker": 0}]});
    let scene = scene_of(bundle);
    let plan = plan_at(&scene, 44);
    assert!(plan.lights.is_empty(), "the primary (scene-a) has no lamp");
    let t = plan.transition.as_deref().unwrap();
    assert_eq!(t.other.lights.len(), 1);
    assert!(t.other.requires.contains(&Capability::PointLights));
    // The top level asks for it too, in the one canonical order, with `fxTransition` last.
    assert!(plan.requires.contains(&Capability::PointLights));
    let mut sorted = plan.requires.clone();
    sorted.sort();
    assert_eq!(plan.requires, sorted);
    assert_eq!(plan.requires.last(), Some(&Capability::FxTransition));
    // A runtime that lacks `fxTransition` (every other capability present) refuses the plan by name.
    let without: Vec<Capability> = Capability::ALL
        .into_iter()
        .filter(|capability| *capability != Capability::FxTransition)
        .collect();
    assert_eq!(
        plan.missing_capabilities(&without),
        [Capability::FxTransition]
    );
    assert!(plan.require_capabilities(&without).is_err());
    assert_eq!(plan.require_capabilities(&Capability::ALL), Ok(()));
    // One that has it but not the lamp's capability is refused for the lamp.
    let no_lights: Vec<Capability> = Capability::ALL
        .into_iter()
        .filter(|capability| *capability != Capability::PointLights)
        .collect();
    assert_eq!(
        plan.missing_capabilities(&no_lights),
        [Capability::PointLights]
    );
    // A requirement that omits what the neighbour needs is a lie the reader refuses.
    let value = plan.to_json();
    let shortened: Vec<Value> = value["requires"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|name| *name != "pointLights")
        .cloned()
        .collect();
    assert_eq!(
        refused(&with(value.clone(), "/requires", json!(shortened))),
        "/requires"
    );
    let no_transition: Vec<Value> = value["requires"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|name| *name != "fxTransition")
        .cloned()
        .collect();
    assert_eq!(
        refused(&with(value, "/requires", json!(no_transition))),
        "/requires"
    );
}

#[test]
fn fx_transition_is_the_fourteenth_capability_and_the_catalog_did_not_move() {
    assert_eq!(Capability::ALL.len(), 14);
    assert_eq!(Capability::ALL[13], Capability::FxTransition);
    assert_eq!(Capability::FxTransition.as_str(), "fxTransition");
    assert_eq!(
        Capability::parse("fxTransition"),
        Some(Capability::FxTransition)
    );
    assert_eq!(tessel_editor_core::scene_plan::PLAN_VERSION, 1);
    assert_eq!(
        tessel_editor_core::directed_scene::fx::FX_CATALOG_VERSION,
        1
    );
    assert_eq!(tessel_editor_core::scene_plan::Domain::ALL.len(), 13);
}

/// T28 (plan): every field of `transition` corrupted gives an invalid plan with a pointer into
/// `/transition`, never a panic and never a quiet acceptance.
#[test]
fn every_field_of_the_transition_refuses_the_wrong_type_and_the_missing_value() {
    let value = window_json();
    let fields: Vec<String> = value["transition"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert_eq!(fields.len(), 15);
    let strings = [
        json!(null),
        json!(1),
        json!(true),
        json!([]),
        json!({}),
        json!(""),
    ];
    let numbers = [
        json!(null),
        json!("x"),
        json!(true),
        json!([]),
        json!({}),
        json!(-1),
        json!(1.5),
        json!(1e300),
        json!(u64::MAX),
    ];
    let bools = [json!(null), json!(1), json!("x"), json!([]), json!({})];
    let params = [
        json!(null),
        json!("x"),
        json!(1),
        json!({}),
        json!([]),
        json!([["curve", "smooth"]]),
        json!([["intensity", 1.0], ["curve", "smooth"]]),
        json!([["curve", "bogus"], ["intensity", 1.0]]),
        json!([["curve", "smooth"], ["intensity", "x"]]),
        json!([["curve", "smooth"], ["intensity", 0.5]]),
        json!([["curve", "smooth", 1], ["intensity", 1.0]]),
        json!([["curve", "smooth"], ["intensity", 1.0], ["extra", 1]]),
    ];
    let others = [json!(null), json!(1), json!("x"), json!([]), json!({})];
    let mut tried = 0;
    for field in &fields {
        let candidates: &[Value] = match field.as_str() {
            "id" | "effect" | "fromShotId" | "toShotId" | "primary" | "easing" => &strings,
            "startFrame" | "durationFrames" | "frameInWindow" | "progress" | "amount" => &numbers,
            "heldInstance" | "heldCamera" => &bools,
            "params" => &params,
            "other" => &others,
            other => panic!("a field this test does not know: {other}"),
        };
        for candidate in candidates {
            let pointer = refused(&with(
                value.clone(),
                &format!("/transition/{field}"),
                candidate.clone(),
            ));
            assert!(
                pointer.starts_with("/transition"),
                "{field} = {candidate}: {pointer}"
            );
            tried += 1;
        }
        // The field removed.
        let mut missing = value.clone();
        missing["transition"].as_object_mut().unwrap().remove(field);
        assert!(
            refused(&missing).starts_with("/transition"),
            "missing {field}"
        );
        tried += 1;
    }
    assert!(tried > 100);
    // A key the plan does not define.
    let mut extra = value.clone();
    extra["transition"]["extra"] = json!(1);
    assert!(refused(&extra).starts_with("/transition"));
    // The transition itself, as the wrong type.
    for candidate in [json!(null), json!(1), json!("x"), json!([]), json!(true)] {
        assert!(refused(&with(value.clone(), "/transition", candidate)).starts_with("/transition"));
    }
}

#[test]
fn the_relations_between_the_decks_and_the_mix_are_checked_with_their_own_pointer() {
    let value = window_json();
    let cases: [(&str, Value, &str); 14] = [
        ("/transition/progress", json!(0.5), "/transition/progress"),
        ("/transition/amount", json!(0.5), "/transition/amount"),
        (
            "/transition/frameInWindow",
            json!(3),
            "/transition/frameInWindow",
        ),
        (
            "/transition/frameInWindow",
            json!(12),
            "/transition/frameInWindow",
        ),
        (
            "/transition/durationFrames",
            json!(0),
            "/transition/durationFrames",
        ),
        (
            "/transition/startFrame",
            json!(-42),
            "/transition/startFrame",
        ),
        ("/transition/primary", json!("to"), "/transition/primary"),
        ("/transition/toShotId", json!("s1"), "/transition/toShotId"),
        ("/transition/effect", json!("wipe"), "/transition/effect"),
        ("/transition/easing", json!("bounce"), "/transition/easing"),
        (
            "/transition/other/frame/shotId",
            json!("s3"),
            "/transition/other/frame/shotId",
        ),
        (
            "/transition/other/frame/editFrame",
            json!(45),
            "/transition/other/frame/editFrame",
        ),
        (
            "/transition/other/output/width",
            json!(1),
            "/transition/other/output",
        ),
        (
            "/transition/other/source/bundleId",
            json!("elsewhere"),
            "/transition/other/source",
        ),
    ];
    for (pointer, replacement, expected) in cases {
        let tampered = with(value.clone(), pointer, replacement.clone());
        assert_eq!(refused(&tampered), expected, "{pointer} = {replacement}");
    }
    // The neighbour may not carry an output pass (they act once, in the primary deck).
    let scene = spec_scene_with_tracks(
        vec![tr(json!({}))],
        json!([output_clip("t3", "fade-out", "fade", 0, 144)]),
    );
    let mut tampered = plan_at(&scene, 44).to_json();
    let pass = tampered["fx"][0].clone();
    tampered["transition"]["other"]["fx"] = json!([pass]);
    // The neighbour's own requirement follows its passes, so only the relation is left to refuse.
    let requires: Vec<Value> = tampered["requires"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|name| *name != "fxTransition")
        .cloned()
        .collect();
    tampered["transition"]["other"]["requires"] = json!(requires);
    assert_eq!(refused(&tampered), "/transition/other/fx/0/scope");
    // A held head must sit on the first frame.
    let held = with(
        value.clone(),
        "/transition/other/frame/instanceFrame",
        json!(3),
    );
    assert_eq!(refused(&held), "/transition/heldInstance");
}

#[test]
fn a_plan_never_nests_and_a_neighbour_with_a_transition_is_refused_before_it_is_read() {
    let value = window_json();
    // The neighbour carries a whole transition (depth 2).
    let nested = with(
        value.clone(),
        "/transition/other/transition",
        value["transition"].clone(),
    );
    assert_eq!(refused(&nested), "/transition/other/transition");
    // And one that is garbage: still refused at the same place, without reading it.
    for garbage in [json!(null), json!(1), json!("x"), json!({"other": {}})] {
        let nested = with(value.clone(), "/transition/other/transition", garbage);
        assert_eq!(refused(&nested), "/transition/other/transition");
    }
    // A hand-built chain is refused by the validator without recursing into it.
    let mut plan = plan_at(&window_scene(), 44);
    let inner = plan.clone();
    plan.transition.as_mut().unwrap().other.transition = inner.transition;
    assert!(plan.resource_union().is_ok());
    // The validator refuses it on its own, without going through the reader.
    assert_eq!(
        pointer_of(plan.validate().unwrap_err()),
        "/transition/other/transition"
    );
    let wire = plan.to_json();
    assert_eq!(refused(&wire), "/transition/other/transition");
}

#[test]
fn a_plan_in_an_edit_with_two_windows_is_one_window_at_a_time() {
    let scene = spec_scene(vec![
        tr(json!({})),
        tr(json!({"id": "tr-2", "fromShotId": "s2", "toShotId": "s3"})),
    ]);
    for (frame, id) in [(45, "tr"), (49, "tr"), (93, "tr-2"), (98, "tr-2")] {
        let plan = plan_at(&scene, frame);
        assert_eq!(plan.transition.as_deref().unwrap().id, id, "{frame}");
    }
    // s2 sits between the two: its middle frames are plain.
    for frame in [60, 80, 89] {
        assert!(plan_at(&scene, frame).transition.is_none(), "{frame}");
    }
}
