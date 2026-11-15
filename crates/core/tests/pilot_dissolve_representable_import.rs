//! BL082.4.17: the representable analytic scenarios (`dissolve-representable-cases.json`) are
//! VALID destination documents. This file proves **importability and bounds** in the real core of
//! `main` and nothing else: the two-deck evaluation, the hold of a deck outside its own shot and
//! every pixel belong to later consumers (S1b, S2). It reads the committed JSON and runs no
//! Python; the public transition gate stays closed (`FX_TRANSITION_NOT_YET`).

use core::prelude::v1::test;

use serde_json::Value;
use tessel_editor_core::directed_scene::fx::{TransitionAdmission, transition_window};
use tessel_editor_core::directed_scene::lighting::Lighting;
use tessel_editor_core::directed_scene::{DirectedScene, ImportError, Limits};
use tessel_editor_core::time::Ratio;

const CASES: &str =
    include_str!("../../../docs/migration/fixtures/pilot-fx/dissolve-representable-cases.json");

const DECKS: [(&str, &str, &str, &str); 2] = [
    ("A", "shot-a", "inst-a", "cam-a"),
    ("B", "shot-b", "inst-b", "cam-b"),
];

fn cases() -> Value {
    serde_json::from_str(CASES).expect("the committed cases are JSON")
}

fn admit(value: Value) -> Result<DirectedScene, ImportError> {
    DirectedScene::import_value_admitting(value, &Limits::default(), TransitionAdmission::Accept)
}

fn codes(error: &ImportError) -> Vec<&'static str> {
    error.issues().iter().map(|issue| issue.code).collect()
}

fn text(value: &Value) -> &str {
    value.as_str().expect("a string")
}

fn int(value: &Value) -> i64 {
    value.as_i64().expect("an integer")
}

fn real(value: &Value) -> f64 {
    value.as_f64().expect("a number")
}

fn near(got: f64, want: f64, tolerance: f64, what: &str) {
    assert!(
        (got - want).abs() <= tolerance,
        "{what}: {got} vs {want} (tolerance {tolerance})"
    );
}

fn triple(value: &Value) -> [f64; 3] {
    let parts: Vec<f64> = text(value)
        .split(',')
        .map(|part| part.parse().expect("a number"))
        .collect();
    [parts[0], parts[1], parts[2]]
}

fn scenarios(cases: &Value) -> &Vec<Value> {
    cases["scenarios"].as_array().expect("scenarios")
}

fn scene_of(cases: &Value, scenario: &Value) -> DirectedScene {
    let id = text(&scenario["id"]);
    admit(cases["bundles"][id].clone()).unwrap_or_else(|error| panic!("{id}: {:?}", codes(&error)))
}

fn rational(value: Ratio) -> String {
    format!("{}/{}", value.num(), value.den())
}

#[test]
fn every_bundle_imports_and_the_public_gate_stays_closed() {
    let cases = cases();
    assert_eq!(scenarios(&cases).len(), 7);
    for scenario in scenarios(&cases) {
        let id = text(&scenario["id"]);
        let scene = scene_of(&cases, scenario);
        assert_eq!(scene.bundle().fx.transitions.len(), 1, "{id}");
        let transition = &scene.bundle().fx.transitions[0];
        let expected = &scenario["transition"];
        assert_eq!(
            transition.alignment.name(),
            text(&expected["alignment"]),
            "{id}"
        );
        assert_eq!(
            transition.duration_frames,
            int(&expected["duration"]),
            "{id}"
        );
        let cut = scene
            .bundle()
            .shot(&transition.to_shot_id)
            .unwrap()
            .start_frame as i64;
        let window = transition_window(
            cut,
            transition.duration_frames,
            transition.alignment,
            scene.bundle().edit_duration_frames as i64,
        )
        .unwrap();
        assert_eq!(window.start, int(&expected["window"]["start"]), "{id}");
        assert_eq!(
            window.end,
            int(&expected["window"]["end_exclusive"]),
            "{id}"
        );
        let public = DirectedScene::import_value(cases["bundles"][id].clone())
            .expect_err("the public import refuses a transition");
        assert!(codes(&public).contains(&"FX_TRANSITION_NOT_YET"), "{id}");
        assert!(
            !codes(&public).contains(&"SHOT_EXCEEDS_INSTANCE_DURATION"),
            "{id}: the refusal must be the gate, not the shape"
        );
    }
}

#[test]
fn the_old_instance_shorter_than_its_shot_is_still_refused() {
    let cases = cases();
    let scenario = scenarios(&cases)
        .iter()
        .find(|scenario| scenario["id"] == "rep-b-tail-instance-held-camera-runs")
        .expect("scenario b");
    let mut bundle = cases["bundles"][text(&scenario["id"])].clone();
    // The S0 `lit-*-held` shape: a 12-frame instance under a 30-frame shot.
    bundle["sceneInstances"][0]["durationFrames"] = 12.into();
    let error = admit(bundle).expect_err("the importer is not weakened");
    assert!(
        codes(&error).contains(&"SHOT_EXCEEDS_INSTANCE_DURATION"),
        "{:?}",
        codes(&error)
    );
}

#[test]
fn bounds_and_held_flags_follow_from_the_imported_numbers() {
    let cases = cases();
    for scenario in scenarios(&cases) {
        let id = text(&scenario["id"]);
        let scene = scene_of(&cases, scenario);
        let bundle = scene.bundle();
        let mut held_somewhere = false;
        for (side, shot_id, _, _) in DECKS {
            let shot = bundle.shot(shot_id).unwrap();
            let instance = bundle.instance(&shot.scene_instance_id).unwrap();
            let length = shot.duration_frames();
            let camera_frames = bundle.camera_clip_frames(&shot.camera_clip_id).unwrap();
            assert!(
                shot.instance_start_frame + length <= instance.duration_frames,
                "{id} {side}: the instance covers its shot"
            );
            assert!(
                shot.camera_in_frame + length <= camera_frames,
                "{id} {side}: the camera covers its shot (BL082.4.23)"
            );
            assert_eq!(
                int(&scenario["shots"][side]["inst_frames"]),
                instance.duration_frames as i64,
                "{id} {side}"
            );
            assert_eq!(
                int(&scenario["shots"][side]["cam_frames"]),
                camera_frames as i64,
                "{id} {side}"
            );
            for frame in scenario["frames"].as_array().unwrap() {
                let f = int(&frame["edit_frame"]);
                let deck = &frame["decks"][side];
                let delta = f - shot.start_frame as i64;
                for (clock, start, frames, got, held) in [
                    (
                        "instance",
                        shot.instance_start_frame as i64,
                        instance.duration_frames as i64,
                        int(&deck["instance_frame"]),
                        deck["held_instance"].as_bool().unwrap(),
                    ),
                    (
                        "camera",
                        shot.camera_in_frame as i64,
                        camera_frames as i64,
                        int(&deck["camera_clip_frame"]),
                        deck["held_camera"].as_bool().unwrap(),
                    ),
                ] {
                    let wanted = start + delta;
                    let clamped = wanted.clamp(0, frames - 1);
                    assert_eq!(
                        int(&deck[format!("{clock}_wanted")]),
                        wanted,
                        "{id} {side} f{f}"
                    );
                    assert_eq!(got, clamped, "{id} {side} {clock} f{f}");
                    assert_eq!(held, wanted != clamped, "{id} {side} {clock} f{f}: held");
                    if (0..frames).contains(&wanted) {
                        assert!(
                            !held,
                            "{id} {side} {clock} f{f}: a real frame is never held"
                        );
                    }
                    if shot.holds_frame(f as u64) {
                        assert!(!held && wanted == got, "{id} {side} f{f}: inside its shot");
                    }
                    held_somewhere |= held;
                }
                assert_eq!(
                    deck["fx_edge"].as_bool().unwrap(),
                    !shot.holds_frame(f as u64),
                    "{id} {side} f{f}"
                );
            }
        }
        assert_eq!(
            held_somewhere,
            text(&scenario["transition"]["handles"]) == "hold",
            "{id}: `hold` handles exactly when a clock is held"
        );
    }
}

#[test]
fn the_core_agrees_on_mapping_time_camera_and_marker() {
    let cases = cases();
    for scenario in scenarios(&cases) {
        let id = text(&scenario["id"]);
        let scene = scene_of(&cases, scenario);
        let bundle = scene.bundle();
        let moving = text(&scenario["kit"]) != "lit";
        for frame in scenario["frames"].as_array().unwrap() {
            let f = int(&frame["edit_frame"]);
            let time = bundle.edit_time_of_frame(f as u64).unwrap();
            assert_eq!(
                rational(time),
                text(&frame["edit_seconds"]["rational"]),
                "{id} f{f}"
            );
            near(
                time.to_f64(),
                real(&frame["edit_seconds"]["decimal"]),
                1e-9,
                id,
            );
            for (side, shot_id, instance_id, camera_id) in DECKS {
                let deck = &frame["decks"][side];
                near(time.to_f64(), real(&deck["scene_seconds"]), 1e-9, id);
                if bundle.shot(shot_id).unwrap().holds_frame(f as u64) {
                    let mapping = bundle.resolve_frame_mapping(f).unwrap();
                    assert_eq!(mapping.shot.id, shot_id, "{id} f{f}");
                    assert_eq!(
                        mapping.instance_frame as i64,
                        int(&deck["instance_frame"]),
                        "{id} {side} f{f}"
                    );
                    assert_eq!(
                        mapping.camera_clip_frame as i64,
                        int(&deck["camera_clip_frame"]),
                        "{id} {side} f{f}"
                    );
                    assert_eq!(rational(mapping.edit_time), rational(time), "{id} f{f}");
                }
                let clip = &scenario["decks"][side]["camera"];
                let segment = &clip["segments"][0];
                let frames = int(&segment["frames"]);
                let clip_frame = int(&deck["camera_clip_frame"]);
                let pose = bundle.camera_at_clip_frame(camera_id, clip_frame).unwrap();
                let travelled = if text(&segment["recipe"]) == "hold" {
                    0.0
                } else {
                    real(&segment["distance"]) * clip_frame.min(frames - 1) as f64
                        / (frames - 1).max(1) as f64
                };
                let initial = &clip["position"];
                let label = format!("{id} {side} f{f} camera");
                near(pose.position.x, real(&initial[0]) + travelled, 1e-9, &label);
                near(pose.position.y, real(&initial[1]), 1e-9, &label);
                near(pose.position.z, real(&initial[2]), 1e-9, &label);
                if moving {
                    let nodes = bundle
                        .evaluate_instance_nodes(instance_id, int(&deck["instance_frame"]) as u64)
                        .unwrap();
                    let marker = nodes.node("marker").expect("the marker node");
                    let want = triple(&deck["marker"]["world_centre"]);
                    let label = format!("{id} {side} f{f} marker");
                    near(marker.local_position.x, want[0], 1e-9, &label);
                    near(marker.local_position.y, want[1], 1e-9, &label);
                    near(marker.local_position.z, want[2], 1e-9, &label);
                }
            }
        }
    }
}

#[test]
fn lights_are_in_limits_and_flicker_on_the_real_edit_clock() {
    let cases = cases();
    for scenario in scenarios(&cases) {
        let id = text(&scenario["id"]);
        let scene = scene_of(&cases, scenario);
        let bundle = scene.bundle();
        let fps = &scenario["fps"];
        let (numerator, denominator) = (int(&fps[0]), int(&fps[1]));
        let mut instance_clock_gap = 0;
        for (index, (side, _, _, _)) in DECKS.iter().enumerate() {
            let pointer = format!("/sceneDefinitions/{index}");
            let definition = &cases["bundles"][id]["sceneDefinitions"][index];
            let lighting = Lighting::read(definition, &pointer)
                .unwrap_or_else(|refusals| panic!("{id} {side}: {refusals:?}"))
                .unwrap_or_default();
            assert!(lighting.lights.len() <= 4, "{id} {side}");
            for light in &lighting.lights {
                assert!((0.0..=20.0).contains(&light.intensity), "{id} {side}");
                assert!(light.radius > 0.0, "{id} {side}");
                assert!((0.0..=1.0).contains(&light.flicker), "{id} {side}");
                for channel in [light.color.r, light.color.g, light.color.b] {
                    assert!((0.0..=1.0).contains(&channel), "{id} {side}");
                }
            }
            for frame in scenario["frames"].as_array().unwrap() {
                let f = int(&frame["edit_frame"]);
                let seconds = bundle.edit_time_of_frame(f as u64).unwrap().to_f64();
                let rows = frame["decks"][*side]["lights"].as_array().unwrap();
                assert_eq!(rows.len(), lighting.lights.len(), "{id} {side} f{f}");
                for (row, light) in rows.iter().zip(&lighting.lights) {
                    assert_eq!(text(&row["id"]), light.id, "{id} {side}");
                    let state = light.at(f, seconds, None);
                    // The oracle states 9 decimals of `intensity * flicker level`.
                    near(state.intensity, real(&row["effective_intensity"]), 2e-9, id);
                    // On the INSTANCE clock the same lamp would flicker elsewhere: the real clock
                    // is what a held deck must keep (D17).
                    let held = frame["decks"][*side]["held_instance"].as_bool().unwrap();
                    if held && frame["in_window"] == true {
                        let instance_seconds = Ratio::new(
                            int(&frame["decks"][*side]["instance_frame"]) * denominator,
                            numerator,
                        )
                        .unwrap()
                        .to_f64();
                        let frozen = light.at(f, instance_seconds, None).intensity;
                        if (frozen - state.intensity).abs() > 1.0 {
                            instance_clock_gap += 1;
                        }
                    }
                }
            }
        }
        let claims_instance_clock = scenario["must_reject"]
            .as_array()
            .unwrap()
            .iter()
            .any(|claim| claim == "LIN");
        if claims_instance_clock {
            assert!(
                instance_clock_gap >= 3,
                "{id}: the core's own flicker separates the instance clock from the real one on {instance_clock_gap} held frames"
            );
        }
    }
}
