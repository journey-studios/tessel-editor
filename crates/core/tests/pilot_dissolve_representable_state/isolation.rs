//! What the comparison must refuse: a falsified copy of the real plan for every wrong clock, flag,
//! role, rate and lamp clock; the static state of a held clock; the camera length with layers.

use core::prelude::v1::test;

use std::collections::BTreeMap;

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::fx::DeckSide;
use tessel_editor_core::directed_scene::lighting::PointLight;
use tessel_editor_core::scene_plan::ScenePlan;
use tessel_editor_core::time::Ratio;

use crate::reference::*;
/// How a hostile implementation would differ, as a falsified copy of the real plan. Each is asked
/// of the same comparison, which must refuse it.
fn falsified(
    scenario: &Value,
    frame: &Value,
    plan: &ScenePlan,
    lights: &[Vec<PointLight>; 2],
) -> Vec<(&'static str, ScenePlan)> {
    let t = plan.transition.as_deref().unwrap();
    let f = int(&frame["edit_frame"]);
    let neighbour = text(&frame["neighbour"]);
    let row = &frame["decks"][neighbour];
    let side = t.primary.other();
    let clocks = t.clocks(side).unwrap();
    let mut found: Vec<(&'static str, ScenePlan)> = Vec::new();
    let mut push = |kind: &'static str, edit: &dyn Fn(&mut ScenePlan)| {
        let mut copy = plan.clone();
        edit(&mut copy);
        found.push((kind, copy));
    };
    push("primary at the cut", &|p| {
        let t = p.transition.as_deref_mut().unwrap();
        t.primary = t.primary.other();
    });
    push("held instance flag", &|p| {
        let t = p.transition.as_deref_mut().unwrap();
        t.held_instance = !t.held_instance;
    });
    push("held camera flag", &|p| {
        let t = p.transition.as_deref_mut().unwrap();
        t.held_camera = !t.held_camera;
    });
    for (kind, primary, axis) in [
        ("primary camera target x", true, 0),
        ("primary camera target y", true, 1),
        ("primary camera target z", true, 2),
        ("neighbour camera target x", false, 0),
        ("neighbour camera target y", false, 1),
        ("neighbour camera target z", false, 2),
    ] {
        push(kind, &|p| {
            let camera = if primary {
                &mut p.camera
            } else {
                &mut p.transition.as_deref_mut().unwrap().other.camera
            };
            match axis {
                0 => camera.target.x += 1.0,
                1 => camera.target.y += 1.0,
                _ => camera.target.z += 1.0,
            }
        });
    }
    push("primary camera fov", &|p| p.camera.fov_deg += 1.0);
    push("neighbour camera fov", &|p| {
        p.transition.as_deref_mut().unwrap().other.camera.fov_deg += 1.0;
    });
    for (clock, key, held) in [
        (clocks.instance, "instance_frame", "held_instance"),
        (clocks.camera, "camera_clip_frame", "held_camera"),
    ] {
        let instance = key == "instance_frame";
        // Held at the last real frame or at the first one (FLG): the flag the rule never sets.
        let (length, wanted) = if instance {
            (
                t.timing.deck(side).instance_duration_frames as i64,
                int(&row["instance_wanted"]),
            )
        } else {
            (
                t.timing.deck(side).camera_clip_duration_frames as i64,
                int(&row["camera_wanted"]),
            )
        };
        if !flag(&row[held]) && (wanted == length - 1 || wanted == 0) {
            push(
                if instance {
                    "flagged edge instance"
                } else {
                    "flagged edge camera"
                },
                &|p| {
                    let t = p.transition.as_deref_mut().unwrap();
                    if instance {
                        t.held_instance = true;
                    } else {
                        t.held_camera = true;
                    }
                },
            );
        }
        // The tail with no clamp: the raw frame where the clip has none.
        if clock.raw > clock.sampled as i64 {
            let raw = clock.raw as u64;
            push(
                if instance {
                    "unclamped instance"
                } else {
                    "unclamped camera"
                },
                &|p| {
                    let other = &mut p.transition.as_deref_mut().unwrap().other;
                    if instance {
                        other.frame.instance_frame = raw;
                    } else {
                        other.frame.camera_clip_frame = raw;
                    }
                },
            );
        }
    }
    // The clocks swapped, and the neighbour on the primary's clocks (SWP, PRI).
    if clocks.instance.sampled != clocks.camera.sampled {
        let (instance, camera) = (clocks.camera.sampled, clocks.instance.sampled);
        push("swapped clocks", &|p| {
            let other = &mut p.transition.as_deref_mut().unwrap().other;
            other.frame.instance_frame = instance;
            other.frame.camera_clip_frame = camera;
        });
    }
    if int(&frame["decks"][text(&frame["primary"])]["instance_wanted"])
        != int(&row["instance_frame"])
    {
        let (instance, camera) = (plan.frame.instance_frame, plan.frame.camera_clip_frame);
        push("primary clocks on the neighbour", &|p| {
            let other = &mut p.transition.as_deref_mut().unwrap().other;
            other.frame.instance_frame = instance;
            other.frame.camera_clip_frame = camera;
        });
    }
    // The nominal frame rate (30 for 30000/1001): the exact edit time of the plan is the reference.
    let nominal =
        (int(&scenario["fps"][0]) + int(&scenario["fps"][1]) / 2) / int(&scenario["fps"][1]);
    if nominal * int(&scenario["fps"][1]) != int(&scenario["fps"][0]) {
        let wrong = Ratio::new(f, nominal).unwrap();
        push("nominal frame rate", &|p| p.frame.edit_time = wrong);
    }
    // The neighbour's lamp read on another clock: the instance's, or another frame's.
    let index = if side == deck_side_of(t, "shot-a") {
        0
    } else {
        1
    };
    for (kind, seconds_frame) in [
        ("lamp on the instance clock", clocks.instance.sampled as i64),
        ("lamp on another frame", f + 7),
    ] {
        for (position, light) in lights[index].iter().enumerate() {
            let seconds = time_of(scenario, seconds_frame).to_f64();
            let edit = if kind == "lamp on another frame" {
                seconds_frame
            } else {
                f
            };
            let wrong = light.at(edit, seconds, None).intensity;
            if (wrong - t.other.lights[position].intensity).abs() > 1e-6 {
                push(kind, &|p| {
                    p.transition.as_deref_mut().unwrap().other.lights[position].intensity = wrong;
                });
            }
        }
    }
    found
}

fn deck_side_of(t: &tessel_editor_core::scene_plan::PlanTransition, shot: &str) -> DeckSide {
    if t.from_shot_id == shot {
        DeckSide::From
    } else {
        DeckSide::To
    }
}

#[test]
fn every_wrong_clock_flag_role_or_rate_is_refused_by_the_comparison() {
    let cases = cases();
    let mut refused: BTreeMap<&'static str, usize> = BTreeMap::new();
    for scenario in scenarios(&cases) {
        let world = world(&cases, scenario);
        for frame in frames(scenario) {
            if frame["in_window"] != true {
                continue;
            }
            let plan = plan_of(&world.scene, scenario, frame);
            verify_window(scenario, frame, &plan).unwrap();
            for (kind, copy) in falsified(scenario, frame, &plan, &world.lights) {
                assert!(
                    verify_window(scenario, frame, &copy).is_err(),
                    "{} f{}: `{kind}` went unnoticed",
                    text(&scenario["id"]),
                    int(&frame["edit_frame"])
                );
                *refused.entry(kind).or_default() += 1;
            }
        }
    }
    for kind in [
        "primary at the cut",
        "held instance flag",
        "held camera flag",
        "primary camera target x",
        "primary camera target y",
        "primary camera target z",
        "neighbour camera target x",
        "neighbour camera target y",
        "neighbour camera target z",
        "primary camera fov",
        "neighbour camera fov",
        // No neighbour camera of the reference sits on its first or last real frame (the instance
        // does, once): the camera's border is proven by scene_fx_transition::timing.
        "flagged edge instance",
        "unclamped instance",
        "unclamped camera",
        "swapped clocks",
        "primary clocks on the neighbour",
        "nominal frame rate",
        "lamp on the instance clock",
        "lamp on another frame",
    ] {
        assert!(
            refused.get(kind).copied().unwrap_or(0) > 0,
            "no frame exercised `{kind}`: {refused:?}"
        );
    }
    // The NTSC scenario is where nominal 30 and the exact 30000/1001 part ways: every window frame.
    assert_eq!(refused["nominal frame rate"], 12, "{refused:?}");
    assert_eq!(refused["primary at the cut"], 84);
}

/// A held deck is a static state: the instance holds the same nodes, the camera the same pose,
/// while the other clock keeps running (UNH: the instance never runs past its last frame, the
/// camera follows the truth).
#[test]
fn a_held_clock_is_a_static_state_and_the_other_clock_keeps_running() {
    let cases = cases();
    let (mut instance_static, mut camera_static, mut camera_runs, mut static_while_running) =
        (0, 0, 0, 0);
    for scenario in scenarios(&cases) {
        let world = world(&cases, scenario);
        let plans: Vec<ScenePlan> = frames(scenario)
            .iter()
            .filter(|frame| frame["in_window"] == true)
            .map(|frame| plan_of(&world.scene, scenario, frame))
            .collect();
        for pair in plans.windows(2) {
            let (p, q) = (
                pair[0].transition.as_deref().unwrap(),
                pair[1].transition.as_deref().unwrap(),
            );
            if p.other.frame.shot_id != q.other.frame.shot_id {
                continue; // the neighbour changes at the cut
            }
            let id = text(&scenario["id"]);
            let f = pair[1].frame.edit_frame;
            let both_instance = p.held_instance && q.held_instance;
            let both_camera = p.held_camera && q.held_camera;
            if both_instance {
                assert_eq!(
                    p.other.frame.instance_frame, q.other.frame.instance_frame,
                    "{id} f{f}"
                );
                assert_eq!(
                    p.other.nodes, q.other.nodes,
                    "{id} f{f}: a held instance is the same nodes"
                );
                instance_static += 1;
            }
            if both_camera {
                assert_eq!(
                    p.other.camera, q.other.camera,
                    "{id} f{f}: a held camera is the same pose"
                );
                assert_eq!(
                    p.other.frame.camera_clip_frame,
                    q.other.frame.camera_clip_frame
                );
                camera_static += 1;
            }
            let running = !p.held_camera
                && !q.held_camera
                && q.other.frame.camera_clip_frame != p.other.frame.camera_clip_frame
                && q.other.camera.position != p.other.camera.position;
            camera_runs += usize::from(running);
            static_while_running += usize::from(both_instance && running);
        }
    }
    assert!(
        instance_static > 0 && camera_static > 0,
        "{instance_static} {camera_static}"
    );
    assert!(camera_runs > 0, "a camera that is not held moves");
    assert!(
        static_while_running > 0,
        "a held instance sits still while its camera runs (UNH)"
    );
}

/// The camera clip is as long as its longest track: a layer past the main track makes the frame
/// real where a main-track-only length would hold it (`rep-a`: raw 405 on a 403-frame main track).
#[test]
fn the_camera_length_counts_the_layers_and_not_only_the_main_track() {
    let cases = cases();
    let scenario = scenarios(&cases)
        .iter()
        .find(|scenario| scenario["id"] == "rep-a-tail-both-held-light")
        .unwrap();
    let frame = frames(scenario)
        .iter()
        .find(|frame| int(&frame["edit_frame"]) == 286)
        .unwrap();
    let mut bundle = cases["bundles"][text(&scenario["id"])].clone();
    let clip = bundle["cameraClips"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|clip| clip["id"] == "cam-a")
        .unwrap();
    assert_eq!(clip["segments"].as_array().unwrap().len(), 1);
    clip["layers"] =
        json!([{"segments": [{"recipe": "hold", "durationFrames": 410, "from": "previousEnd"}]}]);
    let plan = ScenePlan::build(&admit(bundle), 286).unwrap();
    let t = plan.transition.as_deref().unwrap();
    // The reference's own plan: 403 frames, raw 405, held on 402.
    let real_plan =
        ScenePlan::build(&admit(cases["bundles"][text(&scenario["id"])].clone()), 286).unwrap();
    verify_window(scenario, frame, &real_plan).unwrap();
    assert_eq!(
        real_plan
            .transition
            .as_deref()
            .unwrap()
            .timing
            .from
            .camera_clip_duration_frames,
        403
    );
    // Layered: 410 frames, so 405 is real, not held, and the instance is held as before.
    assert_eq!(t.timing.from.camera_clip_duration_frames, 410);
    assert_eq!(t.other.frame.camera_clip_frame, 405);
    assert!(!t.held_camera && t.held_instance);
    assert!(
        verify_window(scenario, frame, &plan).is_err(),
        "a main-track-only reading is told apart from the layered clip"
    );
    plan.validate().unwrap();
    assert_eq!(
        ScenePlan::from_json_str(&plan.to_json_string()).unwrap(),
        plan
    );
}
