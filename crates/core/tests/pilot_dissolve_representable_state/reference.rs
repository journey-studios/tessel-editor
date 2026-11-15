//! The comparison of the representable reference with the real plan: the numbers the reference
//! states (`dissolve-representable-cases.json`, read as data) against a `ScenePlan`, one frame at
//! a time. A mismatch is a `String`, so a falsified plan can be asked the same question.

use std::fmt::Debug;

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::fx::{DeckSide, TransitionAdmission};
use tessel_editor_core::directed_scene::lighting::{Lighting, PointLight};
use tessel_editor_core::directed_scene::{DirectedScene, Limits};
use tessel_editor_core::scene_plan::ScenePlan;
use tessel_editor_core::time::Ratio;
pub const CASES: &str =
    include_str!("../../../../docs/migration/fixtures/pilot-fx/dissolve-representable-cases.json");
/// Every committed kit uses this initial FOV and has only hold/truck segments (no FOV change).
/// The fixture reader pins this precondition against every bundle camera clip.
pub const REFERENCE_FOV_DEG: f64 = 50.0;

/// Fixture side, shot, instance and camera clip of each deck.
pub const DECKS: [(&str, &str, &str, &str); 2] = [
    ("A", "shot-a", "inst-a", "cam-a"),
    ("B", "shot-b", "inst-b", "cam-b"),
];

pub type Check = Result<(), String>;

pub fn cases() -> Value {
    serde_json::from_str(CASES).expect("the committed cases are JSON")
}

pub fn admit(value: Value) -> DirectedScene {
    DirectedScene::import_value_admitting(value, &Limits::default(), TransitionAdmission::Accept)
        .unwrap_or_else(|error| {
            let codes: Vec<_> = error.issues().iter().map(|issue| issue.code).collect();
            panic!("the bundle was refused: {codes:?}")
        })
}

pub fn text(value: &Value) -> &str {
    value.as_str().expect("a string")
}

pub fn int(value: &Value) -> i64 {
    value.as_i64().expect("an integer")
}

pub fn real(value: &Value) -> f64 {
    value.as_f64().expect("a number")
}

pub fn flag(value: &Value) -> bool {
    value.as_bool().expect("a boolean")
}

pub fn triple(value: &Value) -> [f64; 3] {
    let parts: Vec<f64> = text(value)
        .split(',')
        .map(|part| part.parse().expect("a number"))
        .collect();
    [parts[0], parts[1], parts[2]]
}

pub fn scenarios(cases: &Value) -> &Vec<Value> {
    cases["scenarios"].as_array().expect("scenarios")
}

pub fn frames(scenario: &Value) -> &Vec<Value> {
    scenario["frames"].as_array().expect("frames")
}

pub fn deck_row(side: &str) -> &'static (&'static str, &'static str, &'static str, &'static str) {
    DECKS.iter().find(|deck| deck.0 == side).unwrap()
}

pub fn eq<T: PartialEq + Debug>(got: T, want: T, what: impl AsRef<str>) -> Check {
    if got == want {
        Ok(())
    } else {
        Err(format!("{}: {got:?} vs {want:?}", what.as_ref()))
    }
}

pub fn close(got: f64, want: f64, tolerance: f64, what: impl AsRef<str>) -> Check {
    if (got - want).abs() <= tolerance {
        Ok(())
    } else {
        Err(format!(
            "{}: {got} vs {want} (tolerance {tolerance})",
            what.as_ref()
        ))
    }
}

/// The exact time of `frame` at the scenario's frame rate, from its own numbers.
pub fn time_of(scenario: &Value, frame: i64) -> Ratio {
    let (numerator, denominator) = (int(&scenario["fps"][0]), int(&scenario["fps"][1]));
    Ratio::new(frame * denominator, numerator).unwrap()
}

pub fn rational(value: Ratio) -> String {
    format!("{}/{}", value.num(), value.den())
}

/// The lights of each scene definition of the scenario, as the document reads them.
pub fn lighting(cases: &Value, scenario: &Value) -> [Vec<PointLight>; 2] {
    let id = text(&scenario["id"]);
    [0, 1].map(|index| {
        let definition = &cases["bundles"][id]["sceneDefinitions"][index];
        Lighting::read(definition, &format!("/sceneDefinitions/{index}"))
            .unwrap_or_else(|refusals| panic!("{id}: {refusals:?}"))
            .unwrap_or_default()
            .lights
    })
}

/// A scenario's scene, the same scene with no transition (the plain reference of the outside) and
/// the lights of its two scene definitions.
pub struct World {
    pub scene: DirectedScene,
    pub plain: DirectedScene,
    pub lights: [Vec<PointLight>; 2],
}

pub fn world(cases: &Value, scenario: &Value) -> World {
    let id = text(&scenario["id"]);
    let mut without = cases["bundles"][id].clone();
    without["fx"]["transitions"] = json!([]);
    World {
        scene: admit(cases["bundles"][id].clone()),
        plain: admit(without),
        lights: lighting(cases, scenario),
    }
}

pub fn plan_of(scene: &DirectedScene, scenario: &Value, frame: &Value) -> ScenePlan {
    let f = int(&frame["edit_frame"]);
    ScenePlan::build(scene, f).unwrap_or_else(|error| {
        panic!("{} f{f}: {error:?}", text(&scenario["id"]));
    })
}

// ---- the comparison --------------------------------------------------------------------------

/// Everything the reference states about one frame inside the window, against the plan.
pub fn verify_window(scenario: &Value, frame: &Value, plan: &ScenePlan) -> Check {
    let id = text(&scenario["id"]);
    let f = int(&frame["edit_frame"]);
    let at = |what: &str| format!("{id} f{f} {what}");
    let t = plan
        .transition
        .as_deref()
        .ok_or_else(|| at("a window frame has a transition"))?;
    let expected = &scenario["transition"];
    let (primary, neighbour) = (text(&frame["primary"]), text(&frame["neighbour"]));
    let deck_side = |shot: &str| -> Result<DeckSide, String> {
        if t.from_shot_id == shot {
            Ok(DeckSide::From)
        } else if t.to_shot_id == shot {
            Ok(DeckSide::To)
        } else {
            Err(at("a deck of the transition"))
        }
    };

    // Roles, window and the mix.
    eq(
        plan.frame.shot_id.as_str(),
        deck_row(primary).1,
        at("primary shot"),
    )?;
    eq(
        t.other.frame.shot_id.as_str(),
        deck_row(neighbour).1,
        at("neighbour shot"),
    )?;
    eq(
        t.primary,
        deck_side(deck_row(primary).1)?,
        at("primary role"),
    )?;
    eq(
        deck_side(deck_row(neighbour).1)?,
        t.primary.other(),
        at("neighbour role"),
    )?;
    eq(
        t.alignment.name(),
        text(&expected["alignment"]),
        at("alignment"),
    )?;
    eq(
        t.duration_frames,
        int(&expected["duration"]),
        at("duration"),
    )?;
    eq(
        t.start_frame,
        int(&expected["window"]["start"]),
        at("window start"),
    )?;
    eq(t.frame_in_window, int(&frame["k"]), at("k"))?;
    eq(t.timing.cut() as i64, int(&scenario["cut"]), at("the cut"))?;
    eq(
        t.timing.edit_duration_frames as i64,
        int(&scenario["edit_duration_frames"]),
        at("edit length"),
    )?;
    close(
        t.progress,
        real(&frame["transition"]["progress"]),
        2e-9,
        at("progress"),
    )?;
    close(
        t.amount,
        real(&frame["transition"]["amount"]),
        2e-9,
        at("amount"),
    )?;
    // The runtime's smoothing of the eased progress, by its definition.
    let smooth = t.progress * t.progress * (3.0 - 2.0 * t.progress);
    close(
        smooth,
        real(&frame["transition"]["shaped"]),
        2e-9,
        at("shaped"),
    )?;
    close(
        smooth,
        real(&frame["transition"]["weight"]),
        2e-9,
        at("weight"),
    )?;

    // The real edit clock and the frame rate, exactly.
    let edit_time = time_of(scenario, f);
    eq(
        rational(edit_time),
        text(&frame["edit_seconds"]["rational"]).to_string(),
        at("edit seconds"),
    )?;
    eq(plan.frame.edit_time, edit_time, at("primary edit time"))?;
    eq(
        t.other.frame.edit_time,
        edit_time,
        at("neighbour edit time"),
    )?;
    for deck in [plan, &t.other] {
        eq(
            (
                deck.output.fps.numerator as i64,
                deck.output.fps.denominator as i64,
            ),
            (int(&scenario["fps"][0]), int(&scenario["fps"][1])),
            at("frame rate"),
        )?;
    }

    for (side, shot, instance, camera) in DECKS {
        let s = deck_side(shot)?;
        let row = &frame["decks"][side];
        let numbers = &scenario["shots"][side];
        let timing = t.timing.deck(s);
        let label = |what: &str| at(&format!("{side} {what}"));
        // Offsets, limits and lengths of the shot.
        for (got, key, what) in [
            (timing.shot_start_frame, "start", "shot start"),
            (timing.shot_end_frame_exclusive, "end", "shot end"),
            (timing.instance_start_frame, "inst_start", "instance start"),
            (
                timing.instance_duration_frames,
                "inst_frames",
                "instance frames",
            ),
            (timing.camera_in_frame, "cam_in", "camera in"),
            (
                timing.camera_clip_duration_frames,
                "cam_frames",
                "camera frames",
            ),
        ] {
            eq(got as i64, int(&numbers[key]), label(what))?;
        }
        // Raw, sampled and held, each clock on its own.
        let clocks = t.clocks(s).map_err(|error| format!("{error:?}"))?;
        for (clock, name, sampled, wanted, held) in [
            (
                clocks.instance,
                "instance",
                timing.sampled_instance_frame,
                "instance_wanted",
                "held_instance",
            ),
            (
                clocks.camera,
                "camera",
                timing.sampled_camera_clip_frame,
                "camera_wanted",
                "held_camera",
            ),
        ] {
            let sample = if name == "instance" {
                "instance_frame"
            } else {
                "camera_clip_frame"
            };
            eq(clock.raw, int(&row[wanted]), label(&format!("{name} raw")))?;
            eq(
                clock.sampled as i64,
                int(&row[sample]),
                label(&format!("{name} sampled")),
            )?;
            eq(
                sampled as i64,
                int(&row[sample]),
                label(&format!("{name} declared sample")),
            )?;
            eq(clock.held, flag(&row[held]), label(&format!("{name} held")))?;
        }
        // The deck itself: the primary keeps its raw frames, the neighbour its samples.
        let is_primary = side == primary;
        let deck = if is_primary { plan } else { &t.other };
        let (instance_key, camera_key) = if is_primary {
            ("instance_wanted", "camera_wanted")
        } else {
            ("instance_frame", "camera_clip_frame")
        };
        eq(
            deck.source.scene_instance_id.as_str(),
            instance,
            label("instance id"),
        )?;
        eq(
            deck.frame.camera_clip_id.as_str(),
            camera,
            label("camera clip id"),
        )?;
        eq(
            deck.frame.instance_frame as i64,
            int(&row[instance_key]),
            label("instance frame"),
        )?;
        eq(
            deck.frame.camera_clip_frame as i64,
            int(&row[camera_key]),
            label("camera frame"),
        )?;
        eq(
            deck.frame.instance_time,
            time_of(scenario, int(&row[instance_key])),
            label("instance time"),
        )?;
        eq(
            deck.frame.camera_clip_time,
            time_of(scenario, int(&row[camera_key])),
            label("camera time"),
        )?;
        eq(
            flag(&row["fx_edge"]),
            !(timing.shot_start_frame as i64..timing.shot_end_frame_exclusive as i64).contains(&f),
            label("fx edge"),
        )?;
        if !is_primary {
            eq(
                t.held_instance,
                flag(&row["held_instance"]),
                label("neighbour held instance"),
            )?;
            eq(
                t.held_camera,
                flag(&row["held_camera"]),
                label("neighbour held camera"),
            )?;
        }
        // The camera pose along the clip's one segment (held frames sit on the clip's pose).
        let clip = &scenario["decks"][side]["camera"];
        let segment = &clip["segments"][0];
        let length = int(&segment["frames"]);
        let travelled = if text(&segment["recipe"]) == "hold" {
            0.0
        } else {
            real(&segment["distance"]) * int(&row["camera_clip_frame"]).min(length - 1) as f64
                / (length - 1).max(1) as f64
        };
        let position = &clip["position"];
        close(
            deck.camera.position.x,
            real(&position[0]) + travelled,
            1e-9,
            label("camera x"),
        )?;
        close(
            deck.camera.position.y,
            real(&position[1]),
            1e-9,
            label("camera y"),
        )?;
        close(
            deck.camera.position.z,
            real(&position[2]),
            1e-9,
            label("camera z"),
        )?;
        // A truck translates position and target together; a hold leaves both at the input pose.
        let target = &clip["target"];
        for (got, want, axis) in [
            (deck.camera.target.x, real(&target[0]) + travelled, "x"),
            (deck.camera.target.y, real(&target[1]), "y"),
            (deck.camera.target.z, real(&target[2]), "z"),
        ] {
            close(got, want, 1e-9, label(&format!("camera target {axis}")))?;
        }
        close(
            deck.camera.fov_deg,
            REFERENCE_FOV_DEG,
            1e-9,
            label("camera fov"),
        )?;
        // The marker of the moving kits sits where the neighbour or primary instance frame puts it.
        if text(&scenario["kit"]) != "lit" {
            let marker = deck
                .nodes
                .iter()
                .find(|node| node.id == "marker")
                .ok_or_else(|| label("the marker node"))?;
            let want = triple(&row["marker"]["world_centre"]);
            for (axis, index) in [(0, 12), (1, 13), (2, 14)] {
                close(
                    marker.world_matrix.0[index],
                    want[axis],
                    1e-9,
                    label("marker"),
                )?;
            }
        }
        // The lamps on the real edit clock (the flicker of a held deck keeps running).
        let rows = row["lights"].as_array().unwrap();
        eq(deck.lights.len(), rows.len(), label("lamps"))?;
        for (light, wanted) in deck.lights.iter().zip(rows) {
            eq(light.id.as_str(), text(&wanted["id"]), label("lamp id"))?;
            close(
                light.intensity,
                real(&wanted["effective_intensity"]),
                2e-9,
                label("lamp"),
            )?;
        }
    }
    Ok(())
}

/// The frame outside the window: the plain plan of the real primary, and nothing else.
pub fn verify_outside(
    scenario: &Value,
    frame: &Value,
    plan: &ScenePlan,
    plain: &ScenePlan,
) -> Check {
    let id = text(&scenario["id"]);
    let f = int(&frame["edit_frame"]);
    let at = |what: &str| format!("{id} f{f} {what}");
    if plan.transition.is_some() {
        return Err(at("an outside frame has no transition"));
    }
    eq(plan, plain, at("the plan without the transition"))?;
    eq(
        plan.to_json_string(),
        plain.to_json_string(),
        at("its bytes"),
    )?;
    let primary = text(&frame["primary"]);
    let row = &frame["decks"][primary];
    eq(
        plan.frame.shot_id.as_str(),
        deck_row(primary).1,
        at("the real primary"),
    )?;
    eq(
        plan.frame.instance_frame as i64,
        int(&row["instance_frame"]),
        at("instance frame"),
    )?;
    eq(
        plan.frame.camera_clip_frame as i64,
        int(&row["camera_clip_frame"]),
        at("camera frame"),
    )?;
    eq(
        false,
        flag(&row["held_instance"]) || flag(&row["held_camera"]),
        at("a primary is never held"),
    )?;
    eq(plan.frame.edit_time, time_of(scenario, f), at("edit time"))
}
