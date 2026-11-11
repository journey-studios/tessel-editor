//! F062: the composition in stages, the plan (a `scope` per pass, the capability) and the budget
//! over both stages. The helpers and the corpora are the ones of `scene_fx_shot_scope.rs`.

use super::*;
use tessel_editor_core::scene_plan::{Capability, PlanTimeline, ScenePlan};

// ---- composition in stages ---------------------------------------------------------------------

#[test]
fn the_passes_of_a_shot_are_composed_before_the_output_whatever_the_track_order() {
    let mut state = session_with_layer(layer_in_stages());
    // Hand-written expectations: the order the passes are drawn in, frame by frame. `o1` is on the
    // first track and is drawn last because it is the output's.
    let expected: [(i64, &[&str]); 8] = [
        (0, &["a1", "o1"]),
        (9, &["a1", "o1"]),
        (10, &["a1", "b1", "o1"]),
        (19, &["a1", "b1", "o1"]),
        (20, &["a1", "o1"]),
        (47, &["a1", "o1"]),
        (48, &["a2", "o1"]),
        (95, &["a2", "o1"]),
    ];
    for (at, ids) in expected {
        assert_eq!(order(&mut state, at), ids, "frame {at}");
    }
    let answer = frame(&mut state, 10);
    assert_eq!(answer["stages"], json!({"deck": 2, "output": 1}));
    assert_eq!(answer["count"], 3);
    assert_eq!(
        answer["passes"][0]["scope"],
        json!({"kind":"shot","shotId":"shot-1"})
    );
    assert_eq!(answer["passes"][2]["scope"], json!({"kind":"output"}));
    assert_eq!(
        frame(&mut state, 60)["stages"],
        json!({"deck": 1, "output": 1})
    );
}

#[test]
fn composition_is_a_function_of_the_frame_alone() {
    let mut state = session_with_layer(layer_in_stages());
    let linear: Vec<Vec<String>> = (0..96).map(|at| order(&mut state, at)).collect();
    // Seek in any order, with repeats: the same answer at every frame.
    let mut seeked: Vec<(i64, Vec<String>)> = (0..96)
        .rev()
        .chain([10, 47, 48, 10])
        .map(|at| (at, order(&mut state, at)))
        .collect();
    seeked.sort_by_key(|(at, _)| *at);
    for (at, ids) in seeked {
        assert_eq!(ids, linear[usize::try_from(at).unwrap()], "frame {at}");
    }
}

#[test]
fn hidden_clips_and_tracks_draw_nothing_in_either_stage() {
    let mut state = session_with_layer(layer_in_stages());
    edit(
        &mut state,
        json!([{"op":"set_hidden","clip":"b1","hidden":true}]),
    );
    assert_eq!(order(&mut state, 10), ["a1", "o1"], "a hidden shot clip");
    edit(
        &mut state,
        json!([{"op":"set_hidden","clip":"b1","hidden":false},
               {"op":"set_hidden","clip":"o1","hidden":true}]),
    );
    assert_eq!(order(&mut state, 10), ["a1", "b1"], "a hidden output clip");
    // A hidden track: the document's own flag, written by hand.
    let mut layer = layer_in_stages();
    layer["tracks"][1]["hidden"] = json!(true);
    let mut state = session_with_layer(layer);
    assert_eq!(order(&mut state, 10), ["b1", "o1"]);
    assert_eq!(order(&mut state, 60), ["o1"]);
}

// ---- the plan ------------------------------------------------------------------------------------------

#[test]
fn the_plan_lists_the_passes_by_stage_with_a_scope_each_and_asks_for_the_capability() {
    let mut state = session_with_layer(layer_in_stages());
    let plan = plan_json(&mut state, 10);
    let listed: Vec<(&str, Value)> = plan["fx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pass| (pass["clipId"].as_str().unwrap(), pass["scope"].clone()))
        .collect();
    assert_eq!(
        listed,
        [
            ("a1", json!({"kind":"shot","shotId":"shot-1"})),
            ("b1", json!({"kind":"shot","shotId":"shot-1"})),
            ("o1", json!({"kind":"output"})),
        ]
    );
    let requires = plan["requires"].as_array().unwrap();
    for capability in ["outputFx", "fxShotScope", "fxNoise"] {
        assert!(requires.contains(&json!(capability)), "{capability}");
    }
    // The very passes the executor reports for the frame.
    assert_eq!(
        plan["fx"].as_array().cloned().unwrap(),
        passes(&mut state, 10)
    );
    // The wire round-trips.
    let built = ScenePlan::build(&scene_of(&state), 10).unwrap();
    assert_eq!(ScenePlan::from_json(&built.to_json()).unwrap(), built);
}

#[test]
fn the_capability_is_asked_for_only_by_a_plan_with_a_pass_of_a_shot() {
    let mut state = session_with_layer(json!({
        "catalogVersion": 1,
        "tracks": [
            {"id": "t-out", "label": "Out", "clips": [
                {"id": "o1", "effect": "fade", "startFrame": 0, "durationFrames": 96}]},
            {"id": "t-s", "label": "S", "clips": [
                {"id": "s1", "effect": "vignette", "startFrame": 10, "durationFrames": 10,
                 "scope": {"kind": "shot", "shotId": "shot-1"}}]},
        ],
        "transitions": []
    }));
    let requires = |state: &mut EditorState, at: i64| -> Vec<Value> {
        plan_json(state, at)["requires"].as_array().unwrap().clone()
    };
    assert!(
        !requires(&mut state, 5).contains(&json!("fxShotScope")),
        "before the clip"
    );
    assert!(
        requires(&mut state, 5).contains(&json!("outputFx")),
        "the output draws"
    );
    assert!(
        requires(&mut state, 10).contains(&json!("fxShotScope")),
        "first frame"
    );
    assert!(
        requires(&mut state, 19).contains(&json!("fxShotScope")),
        "last frame"
    );
    assert!(
        !requires(&mut state, 20).contains(&json!("fxShotScope")),
        "the end is exclusive"
    );
    assert!(
        !requires(&mut state, 60).contains(&json!("fxShotScope")),
        "another shot"
    );

    // Output passes alone: scope `output` on the wire, no new key, no new capability.
    let mut output_only = session();
    edit(&mut output_only, json!([add("v1", 8, 24)]));
    let plan = plan_json(&mut output_only, 10);
    assert!(
        !plan["requires"]
            .as_array()
            .unwrap()
            .contains(&json!("fxShotScope"))
    );
    let mut keys: Vec<&str> = plan["fx"][0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "amount",
            "clipId",
            "effect",
            "frameInClip",
            "params",
            "scope",
            "trackId",
            "weight"
        ],
        "the pass of an output clip has the same keys as before F062"
    );
    assert_eq!(plan["fx"][0]["scope"], json!({"kind":"output"}));
}

#[test]
fn a_runtime_without_the_capability_refuses_the_shot_plan_by_name_and_serves_the_others() {
    assert_eq!(
        Capability::ALL.len(),
        13,
        "the 13th capability, `fxShotScope`"
    );
    assert_eq!(Capability::ALL[12], Capability::FxShotScope);
    assert_eq!(Capability::FxShotScope.as_str(), "fxShotScope");
    let state = session_with_layer(layer_in_stages());
    let scene = scene_of(&state);
    let without: Vec<Capability> = Capability::ALL
        .into_iter()
        .filter(|capability| *capability != Capability::FxShotScope)
        .collect();
    let in_shot = ScenePlan::build(&scene, 10).unwrap();
    assert_eq!(
        in_shot.missing_capabilities(&without),
        [Capability::FxShotScope]
    );
    assert!(in_shot.require_capabilities(&without).is_err());
    assert_eq!(in_shot.require_capabilities(&Capability::ALL), Ok(()));
    // A runtime that has the capability but not `outputFx` is told `outputFx` too.
    let neither: Vec<Capability> = without
        .iter()
        .copied()
        .filter(|capability| *capability != Capability::OutputFx)
        .collect();
    let missing = in_shot.missing_capabilities(&neither);
    assert!(missing.contains(&Capability::OutputFx));
    assert!(missing.contains(&Capability::FxShotScope));

    // A scene whose effects are all of the output is served by the runtime that predates F062.
    let mut plain = session();
    edit(&mut plain, json!([add("v1", 8, 24)]));
    let plan = ScenePlan::build(&scene_of(&plain), 10).unwrap();
    assert_eq!(plan.require_capabilities(&without), Ok(()));
}

#[test]
fn the_plan_refuses_what_the_frame_cannot_draw() {
    let mut state = session_with_layer(layer_in_stages());
    let good = plan_json(&mut state, 10);
    assert!(ScenePlan::from_json(&good).is_ok());

    // A pass of another shot than the frame's.
    let mut wire = good.clone();
    wire["fx"][0]["scope"]["shotId"] = json!("shot-2");
    let error = ScenePlan::from_json(&wire).unwrap_err().to_string();
    assert!(error.contains("/fx/0/scope/shotId"), "{error}");

    // The output's pass before a shot's: the stages are in order.
    let mut wire = good.clone();
    let listed = wire["fx"].as_array().unwrap().clone();
    wire["fx"] = json!([listed[2], listed[0], listed[1]]);
    let error = ScenePlan::from_json(&wire).unwrap_err().to_string();
    assert!(error.contains("/fx/1/scope"), "{error}");

    // `requires` that does not say `fxShotScope` while a pass is of a shot.
    let mut wire = good.clone();
    let requires: Vec<Value> = wire["requires"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|capability| *capability != "fxShotScope")
        .cloned()
        .collect();
    wire["requires"] = json!(requires);
    let error = ScenePlan::from_json(&wire).unwrap_err().to_string();
    assert!(error.contains("`/requires`"), "{error}");

    // The shapes of the scope.
    for (scope, at) in [
        (json!({"kind":"shot"}), "/fx/0/scope/shotId"),
        (json!({"kind":"shot","shotId":""}), "/fx/0/scope/shotId"),
        (json!({"kind":"node","nodeId":"n"}), "/fx/0/scope"),
        (json!({"kind":"frame"}), "/fx/0/scope"),
        (json!({"kind":"output","shotId":"shot-1"}), "/fx/0/scope"),
    ] {
        let mut wire = good.clone();
        wire["fx"][0]["scope"] = scope.clone();
        let error = ScenePlan::from_json(&wire).unwrap_err().to_string();
        assert!(error.contains(at), "{scope}: {error}");
    }
}

#[test]
fn the_timeline_gives_the_plan_of_a_build_at_every_frame_and_in_any_order() {
    let state = session_with_layer(layer_in_stages());
    let scene = scene_of(&state);
    let timeline = PlanTimeline::new(&scene);
    // Out of order on purpose: the template of a shot is made by whichever frame comes first, and
    // the boundary frames come at both ends.
    for at in (0..96).rev().chain([47, 48, 0, 47, 48]).chain(0..96) {
        assert_eq!(
            timeline.build(at),
            ScenePlan::build(&scene, at),
            "frame {at}"
        );
    }
    let last_of_first = timeline.build(47).unwrap();
    assert_eq!(last_of_first.frame.shot_id, "shot-1");
    assert_eq!(last_of_first.fx[0].clip_id, "a1");
    let first_of_second = timeline.build(48).unwrap();
    assert_eq!(first_of_second.frame.shot_id, "shot-2");
    assert_eq!(first_of_second.fx[0].clip_id, "a2");
}

// ---- the budget: both stages together --------------------------------------------------------------

#[test]
fn the_pass_budget_sums_the_shot_and_the_output() {
    let ops = |shot: usize, output: usize| -> Value {
        let mut ops = Vec::new();
        for index in 0..shot {
            ops.push(add_scoped(&format!("s{index}"), 10, 10, "shot:shot-1"));
        }
        for index in 0..output {
            ops.push(add(&format!("o{index}"), 10, 10));
        }
        Value::Array(ops)
    };
    // 4 + 4 = 8 passes: at the budget, no warning.
    let mut state = session();
    let answer = edit(&mut state, ops(4, 4));
    assert_eq!(answer["budget"]["peak_passes"], 8);
    assert_eq!(answer["budget"]["over_budget"], false);
    assert_eq!(answer["warnings"], json!([]));
    assert_eq!(frame(&mut state, 15)["count"], 8);

    // 5 + 4 = 9 passes: over it, though neither stage alone is.
    let mut state = session();
    let answer = edit(&mut state, ops(5, 4));
    assert_eq!(answer["budget"]["peak_passes"], 9);
    assert_eq!(answer["budget"]["over_budget"], true);
    assert_eq!(answer["budget"]["first_frame_over"], 10);
    assert_eq!(answer["budget"]["frames_over"], 10);
    assert_eq!(answer["warnings"][0]["code"], "FX_PASS_BUDGET");
    let at_15 = frame(&mut state, 15);
    assert_eq!(
        at_15["count"], 9,
        "the budget's peak is what the frame draws"
    );
    assert_eq!(at_15["stages"], json!({"deck": 5, "output": 4}));
    assert_eq!(at_15["over_budget"], true);
    assert_eq!(frame(&mut state, 9)["count"], 0);
    assert_eq!(frame(&mut state, 20)["count"], 0, "the end is exclusive");
    // A warning is not a refusal: the plan is still served, every pass in it.
    assert_eq!(plan_json(&mut state, 15)["fx"].as_array().unwrap().len(), 9);
}
