//! BL082.4.17 (P4b S2-C0b): the representable analytic reference
//! (`dissolve-representable-cases.json`: 7 scenarios, 98 frames, 390 colour probes) read as
//! **state** by the real plan of `main`. The committed JSON is only read (no Python runs, no
//! pixel is drawn or compared): for every frame the plan of the real core, built under the hidden
//! `Accept` policy, must declare the numbers the reference states.
//!
//! In the window the roles, both decks' clocks (raw / sampled / held, independently), offsets,
//! ids, limits, exact `Ratio`s (including 30000/1001), the camera pose, the marker, the lights on
//! the real edit clock and the mix numbers are compared. Outside it the plan is the plain plan of
//! the real primary and the neighbour is compared apart, through the offset evaluator.
//!
//! Mapping and limits. Compared: `primary`/`neighbour`, `instance_wanted`/`camera_wanted` (raw),
//! `instance_frame`/`camera_clip_frame` (sampled), `held_*`, `lights[].effective_intensity`,
//! `marker.world_centre`, `camera` (position and target along the one segment, fixed FOV), `edit_seconds.rational`,
//! `transition.progress`/`amount`/`shaped` (the smoothing is the runtime's own, recomputed here
//! by its definition), `fx_edge`, `k`, `window`, `shots`, `fps`. Diagnostic only, never compared
//! as state: `scene_seconds` (a decimal of the edit time), `probes[]`, `centre_px`,
//! `shot_clip_frame`, `matrix`, `skipped_probes` (pixels, which a later renderer owns).

use core::prelude::v1::test;

use tessel_editor_core::scene_plan::{PlanTimeline, ScenePlan};

#[path = "pilot_dissolve_representable_state/isolation.rs"]
mod isolation;
#[path = "pilot_dissolve_representable_state/reference.rs"]
mod reference;

use reference::*;
// ---- the tests ---------------------------------------------------------------------------------

#[test]
fn the_reference_is_read_whole() {
    let cases = cases();
    assert_eq!(scenarios(&cases).len(), 7);
    let total: usize = scenarios(&cases).iter().map(|s| frames(s).len()).sum();
    assert_eq!(total, 98);
    let probes: usize = scenarios(&cases)
        .iter()
        .flat_map(frames)
        .map(|frame| frame["probes"].as_array().map_or(0, Vec::len))
        .sum();
    assert_eq!(
        probes, 390,
        "the colour probes are diagnostic and not compared"
    );
    let inside: usize = scenarios(&cases)
        .iter()
        .flat_map(frames)
        .filter(|frame| frame["in_window"] == true)
        .count();
    assert_eq!(inside, 7 * 12, "twelve frames of every window");
    for bundle in cases["bundles"].as_object().unwrap().values() {
        for clip in bundle["cameraClips"].as_array().unwrap() {
            assert_eq!(real(&clip["initialFovDeg"]), REFERENCE_FOV_DEG);
        }
    }
}

#[test]
fn every_frame_inside_the_window_matches_the_reference_state_on_both_decks() {
    let cases = cases();
    let mut checked = 0;
    let (mut independent, mut ntsc) = (0, 0);
    for scenario in scenarios(&cases) {
        let world = world(&cases, scenario);
        for frame in frames(scenario) {
            if frame["in_window"] != true {
                continue;
            }
            let plan = plan_of(&world.scene, scenario, frame);
            verify_window(scenario, frame, &plan).unwrap_or_else(|error| panic!("{error}"));
            // The plan is also a plan the three doors read back unchanged.
            plan.validate().unwrap();
            assert_eq!(ScenePlan::from_json(&plan.to_json()).unwrap(), plan);
            assert_eq!(
                ScenePlan::from_json_str(&plan.to_json_string()).unwrap(),
                plan
            );
            let t = plan.transition.as_deref().unwrap();
            independent += usize::from(t.held_instance != t.held_camera);
            ntsc += usize::from(plan.output.fps.numerator == 30_000);
            checked += 1;
        }
    }
    assert_eq!(checked, 84);
    assert!(independent > 0, "a frame holds one clock and not the other");
    assert_eq!(ntsc, 12, "the NTSC scenario is exact, not nominal");
}

#[test]
fn every_frame_outside_the_window_is_the_plain_plan_of_the_real_primary() {
    let cases = cases();
    let mut outside = 0;
    for scenario in scenarios(&cases) {
        let world = world(&cases, scenario);
        for frame in frames(scenario) {
            if frame["in_window"] == true {
                continue;
            }
            let plan = plan_of(&world.scene, scenario, frame);
            let plain = plan_of(&world.plain, scenario, frame);
            verify_outside(scenario, frame, &plan, &plain)
                .unwrap_or_else(|error| panic!("{error}"));
            outside += 1;
        }
    }
    assert_eq!(outside, 98 - 84);
}

/// The offset evaluator answers for the neighbour on every one of the 98 frames, with no plan:
/// kept apart from the plan's own comparison.
#[test]
fn the_offset_evaluator_agrees_with_the_neighbour_of_every_frame() {
    let cases = cases();
    for scenario in scenarios(&cases) {
        let world = world(&cases, scenario);
        let bundle = world.scene.bundle();
        let id = text(&scenario["id"]);
        for frame in frames(scenario) {
            let f = int(&frame["edit_frame"]);
            // Outside the window the reference names no neighbour: it is the deck that is not primary.
            let neighbour = if text(&frame["primary"]) == "A" {
                "B"
            } else {
                "A"
            };
            let row = &frame["decks"][neighbour];
            let at = bundle
                .evaluate_shot_at_offset(deck_row(neighbour).1, f)
                .unwrap_or_else(|error| panic!("{id} f{f}: {error:?}"));
            let mapping = &at.evaluation.mapping;
            for (got, key) in [
                (mapping.instance_frame as i64, "instance_frame"),
                (mapping.camera_clip_frame as i64, "camera_clip_frame"),
            ] {
                assert_eq!(got, int(&row[key]), "{id} f{f} {key}");
            }
            assert_eq!(at.held_instance, flag(&row["held_instance"]), "{id} f{f}");
            assert_eq!(at.held_camera, flag(&row["held_camera"]), "{id} f{f}");
        }
    }
}

#[test]
fn the_answer_does_not_depend_on_the_order_the_frames_are_asked_in() {
    let cases = cases();
    for scenario in scenarios(&cases) {
        let world = world(&cases, scenario);
        let all = frames(scenario);
        let cold: Vec<String> = all
            .iter()
            .map(|frame| plan_of(&world.scene, scenario, frame).to_json_string())
            .collect();
        let check = |timeline: &PlanTimeline, index: usize| {
            let frame = &all[index];
            let plan = timeline.build(int(&frame["edit_frame"])).unwrap();
            assert_eq!(
                plan.to_json_string(),
                cold[index],
                "{} #{index}",
                text(&scenario["id"])
            );
            if frame["in_window"] == true {
                verify_window(scenario, frame, &plan).unwrap_or_else(|error| panic!("{error}"));
            }
        };
        let ascending = PlanTimeline::new(&world.scene);
        (0..all.len()).for_each(|index| check(&ascending, index));
        let reverse = PlanTimeline::new(&world.scene);
        (0..all.len())
            .rev()
            .for_each(|index| check(&reverse, index));
        let repeated = PlanTimeline::new(&world.scene);
        for index in [5, 5, 12, 0, 7, 13, 7, 3, 3, 9, 1, 11, 2, 12, 6, 4, 8, 10] {
            check(&repeated, index);
        }
    }
}
