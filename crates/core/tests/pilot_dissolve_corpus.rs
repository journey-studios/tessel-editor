//! L (corpus, S1b): the committed oracle `docs/migration/fixtures/pilot-fx/dissolve-cases.json`,
//! read by Rust with no Python and no network, against the two-deck evaluation and the plan of a
//! transition window (F064, P4b S1b, BL082.4.3). S1a's rows (document, windows, explain, ids,
//! limits of the document) are in `scene_fx_transition/corpus.rs`; this file consumes the rest:
//!
//! * `deck_frames`, `fps`, `analytic_probes` (12 scenes, 116 frames): the clocks of both decks, the
//!   held flags, the real-frame light and the progress/amount of every probed frame;
//! * `progress` tables and extremes, `pixels` (reference calculation of the mix), `tracer_bullet`;
//! * `shot_clip_frames`: which clip of the neighbour deck is drawn, at which frame;
//! * `resource_union`: the union of both decks' resources.
//!
//! Nothing is changed or regenerated here. What this proves is the plan the core hands a runtime
//! (clocks, flags, `progress`, `amount`, lights, passes, resources); it renders nothing, so the
//! pixel rows are a REFERENCE CALCULATION of the oracle's own formula fed with the plan's numbers,
//! not output of a runtime or a GPU. `centre_px`, `rect_px` and the pixel probes of the analytic
//! scenes need a renderer and are left to S2 (`GOLDEN_NOT_COMPARABLE`).
//!
//! `support` is shared with `scene_fx_transition`; the helpers one of the two files does not use
//! are not dead code.
#![allow(dead_code)]

use core::prelude::v1::test;

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::directed_scene::fx::{FxEasing, FxValue};
use tessel_editor_core::scene_plan::{DeckRole, ScenePlan};

#[path = "scene_fx_transition/support.rs"]
mod support;

use support::*;

const CASES: &str = include_str!("../../../docs/migration/fixtures/pilot-fx/dissolve-cases.json");

fn cases() -> Value {
    serde_json::from_str(CASES).unwrap()
}

fn int(value: &Value) -> i64 {
    value
        .as_i64()
        .unwrap_or_else(|| panic!("{value} is not an integer"))
}

fn real(value: &Value) -> f64 {
    value
        .as_f64()
        .unwrap_or_else(|| panic!("{value} is not a number"))
}

fn near(found: f64, expected: f64, tolerance: f64, what: &str) {
    assert!(
        (found - expected).abs() <= tolerance,
        "{what}: {found:.17} vs {expected:.17}"
    );
}

fn fraction(text: &str) -> (i64, i64) {
    let (num, den) = text.split_once('/').unwrap();
    (num.parse().unwrap(), den.parse().unwrap())
}

// ---- deck frames (T06, T10) ---------------------------------------------------------------------

/// One oracle row against `evaluate_shot_at_offset`: a frame the edit does not have is refused.
fn check_row(scene: &DirectedScene, shot: &str, row: &Value) {
    let bundle = scene.bundle();
    let frame = int(&row["edit_frame"]);
    if !(0..bundle.edit_duration_frames as i64).contains(&frame) {
        assert!(
            bundle.evaluate_shot_at_offset(shot, frame).is_err(),
            "{shot} at {frame} is outside the edit"
        );
        return;
    }
    let found = bundle.evaluate_shot_at_offset(shot, frame).unwrap();
    let mapping = &found.evaluation.mapping;
    assert_eq!(
        (
            mapping.instance_frame as i64,
            mapping.camera_clip_frame as i64,
            found.held_instance,
            found.held_camera
        ),
        (
            int(&row["instance_frame"]),
            int(&row["camera_clip_frame"]),
            row["held_instance"] == true,
            row["held_camera"] == true
        ),
        "{shot} at {frame}: {row}"
    );
}

#[test]
fn deck_frame_rows_agree_with_the_oracle() {
    let cases = cases();
    let rows = &cases["deck_frames"];
    let scene = admit(corpus()).unwrap();
    let corpus_rows = &rows["corpus_valid_cut_instance_offsets"];
    let mut compared = 0;
    for (side, shot) in [("from", "shot-a"), ("to", "shot-b")] {
        for row in corpus_rows[side]["rows"].as_array().unwrap() {
            check_row(&scene, shot, row);
            compared += 1;
        }
    }
    assert!(compared >= 20, "{compared} rows");
    // The reach the oracle states for both shots.
    for (side, shot) in [("from", "shot-a"), ("to", "shot-b")] {
        let reach = scene
            .bundle()
            .shot_reach(scene.bundle().shot(shot).unwrap())
            .unwrap();
        let expected = &corpus_rows[side];
        // Only what the oracle states (the `from` side may state no head).
        if !expected["head_instance"].is_null() {
            assert_eq!(
                reach.head_instance,
                int(&expected["head_instance"]),
                "{shot}"
            );
            assert_eq!(reach.head_camera, int(&expected["head_camera"]), "{shot}");
        }
    }
    // An instance longer than its camera runs out of one first: the clocks are clamped apart.
    check_row(&scene, "shot-a", &rows["joint_clamp_is_wrong"]["row"]);
    // The origin's spec vectors, on the edit the oracle reconstructs from them.
    let spec = scene_of(Edit::spec().bundle());
    let vectors = rows["origin_spec_vectors"]["rows"].as_array().unwrap();
    assert!(!vectors.is_empty());
    for vector in vectors {
        let shot = vector["shot"].as_str().unwrap();
        let mut row = vector["row"].clone();
        row["edit_frame"] = vector["frame"].clone();
        check_row(&spec, shot, &row);
        // What the origin's spec literally says, as it names its keys.
        let found = spec
            .bundle()
            .evaluate_shot_at_offset(shot, int(&vector["frame"]))
            .unwrap();
        let expected = &vector["spec"];
        assert_eq!(
            found.evaluation.mapping.instance_frame as i64,
            int(&expected["instanceFrame"])
        );
        assert_eq!(found.held_camera, expected["heldCamera"] == true);
    }
}

fn with_fps(mut bundle: Value, fps: (i64, i64)) -> Value {
    let value = json!({"numerator": fps.0, "denominator": fps.1});
    bundle["fps"] = value.clone();
    bundle["renderProfile"]["fps"] = value;
    bundle
}

#[test]
fn the_fps_rows_agree_with_the_oracle() {
    let cases = cases();
    let rows = cases["fps"]["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 4);
    for row in rows {
        let fps = (int(&row["fps"][0]), int(&row["fps"][1]));
        let scene = scene_of(with_transitions(
            with_fps(Edit::spec().bundle(), fps),
            vec![tr(json!({}))],
        ));
        let at = |frame: i64| {
            scene
                .bundle()
                .evaluate_shot_at_offset("s2", frame)
                .unwrap()
                .evaluation
                .mapping
        };
        let held = at(44);
        let edit = held.edit_time;
        assert_eq!(
            (edit.num(), edit.den()),
            fraction(row["edit_clock_seconds_frame_44"].as_str().unwrap()),
            "{fps:?}"
        );
        let instance = held.instance_time;
        assert_eq!(
            (instance.num(), instance.den()),
            fraction(row["held_instance_clock_seconds"].as_str().unwrap()),
            "the held instance clock does not run: {fps:?}"
        );
        let start = at(int(&row["window_center_12_cut_48"]["start"])).edit_time;
        assert_eq!(
            (start.num(), start.den()),
            fraction(row["window_start_seconds"].as_str().unwrap())
        );
        let progress = plan_at(&scene, 45).transition.unwrap().progress;
        near(progress, real(&row["progress_k3_d12_cubic"]), 1e-15, "k3");
    }
}

// ---- the analytic scenes: both decks, frame by frame (T06, T10, T27) ---------------------------

fn lamp_json(lamp: &Value) -> Value {
    let triple =
        |key: &str| -> Vec<f64> { lamp[key].as_array().unwrap().iter().map(real).collect() };
    let (colour, position) = (triple("colour"), triple("position"));
    json!({
        "id": lamp["id"], "intensity": lamp["intensity"], "radius": lamp["radius"],
        "flicker": lamp["flicker"],
        "position": {"x": position[0], "y": position[1], "z": position[2]},
        "color": {"r": colour[0], "g": colour[1], "b": colour[2]},
    })
}

/// The scenario as a bundle: its shots, instances and camera clips of the stated lengths (cameras
/// are `hold` clips; the truck recipes only move pixels, which this file does not compare), one
/// lamp per deck of the `lit` and `both` kits, and the transition the scenario names.
fn scenario_bundle(cases: &Value, scenario: &Value, cover_the_shots: bool) -> Value {
    let shot = |side: &str, id: &str| {
        let s = &scenario["shots"][side];
        EditShot {
            id: id.into(),
            start: s["start"].as_u64().unwrap(),
            end: s["end"].as_u64().unwrap(),
            instance: s["instance"].as_str().unwrap().into(),
            instance_start: s["instance_start"].as_u64().unwrap(),
            camera: s["camera"].as_str().unwrap().into(),
            camera_in: s["camera_in"].as_u64().unwrap(),
        }
    };
    let frames = |key: &str| -> Vec<(String, u64)> {
        ["A", "B"]
            .iter()
            .map(|side| {
                let name = scenario["shots"][side][if key == "instance_frames" {
                    "instance"
                } else {
                    "camera"
                }]
                .as_str()
                .unwrap();
                let mut length = scenario[key][name].as_u64().unwrap();
                if cover_the_shots && key == "instance_frames" {
                    let s = &scenario["shots"][side];
                    let (start, end) = (s["start"].as_u64().unwrap(), s["end"].as_u64().unwrap());
                    length = length.max(s["instance_start"].as_u64().unwrap() + end - start);
                }
                (name.to_string(), length)
            })
            .collect()
    };
    let edit = Edit {
        duration: scenario["edit_duration_frames"].as_u64().unwrap(),
        shots: vec![shot("A", "A"), shot("B", "B")],
        instances: frames("instance_frames"),
        cameras: frames("camera_clip_frames"),
    };
    let fps = (int(&scenario["fps"][0]), int(&scenario["fps"][1]));
    let mut bundle = with_fps(edit.bundle(), fps);
    let transition = &scenario["transition"];
    bundle = with_transitions(
        bundle,
        vec![tr(json!({
            "id": "tr", "fromShotId": "A", "toShotId": "B",
            "durationFrames": transition["duration"], "alignment": transition["alignment"],
            "easing": transition["easing"],
            "params": {"curve": transition["curve"], "intensity": transition["intensity"]},
        }))],
    );
    let kit = &cases["analytic_probes"]["kits"][scenario["kit"].as_str().unwrap()];
    for (deck, scene_id) in [("A", "scene-a"), ("B", "scene-b")] {
        let lamps = kit["decks"][deck]["lights"].as_array().unwrap();
        if lamps.is_empty() {
            continue;
        }
        let definition = bundle["sceneDefinitions"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|definition| definition["id"] == scene_id)
            .unwrap();
        definition["lighting"] =
            json!({"ambient": 1, "lights": lamps.iter().map(lamp_json).collect::<Vec<_>>()});
    }
    bundle
}

/// The scenario's scene, and whether the instance lengths had to be lengthened to cover the shot.
///
/// KNOWN DIVERGENCE: three `lit-*-held` scenes give shot A a 12-frame instance under a 30-frame
/// shot, which the origin's frame mapping tolerates (it holds) and this importer refuses
/// (`SHOT_EXCEEDS_INSTANCE_DURATION`, S1a's contract). Those scenes are rebuilt with an instance
/// that covers the shot, and then only what does not depend on the instance length is compared:
/// the real-frame light, `progress`, `amount`, the window and the neighbour's clocks of the shot
/// that was not changed. The refusal itself is asserted.
fn scenario_scene(cases: &Value, scenario: &Value) -> (DirectedScene, bool) {
    match admit(scenario_bundle(cases, scenario, false)) {
        Ok(scene) => (scene, false),
        Err(error) => {
            assert_eq!(
                codes(&error),
                ["SHOT_EXCEEDS_INSTANCE_DURATION"],
                "{}",
                scenario["id"]
            );
            (scene_of(scenario_bundle(cases, scenario, true)), true)
        }
    }
}

fn deck_matches(
    scene: &DirectedScene,
    shot: &str,
    deck: &ScenePlan,
    frame: i64,
    state: &Value,
    what: &str,
    skip_instance_clocks: bool,
) {
    assert_eq!(deck.frame.shot_id, shot, "{what}");
    assert_eq!(deck.frame.edit_frame as i64, frame, "{what}");
    // The scene clock is the real edit frame at the rational fps, held or not.
    near(
        deck.frame.edit_time.to_f64(),
        real(&state["scene_seconds"]),
        1e-9,
        what,
    );
    let held = scene.bundle().evaluate_shot_at_offset(shot, frame).unwrap();
    let source = scene.bundle().shot(shot).unwrap();
    if skip_instance_clocks
        && source.start_frame <= frame as u64
        && (frame as u64) < source.end_frame_exclusive
    {
        // The legacy primary evaluator keeps the raw camera counter even past a short clip.
        // Its pose still holds; the offset evaluator reports the clamped counter. Do not call
        // those two representations equal or claim the adapted primary is an origin deck.
        assert_eq!(
            deck.frame.camera_clip_frame,
            source.camera_in_frame + frame as u64 - source.start_frame,
            "{what}: legacy primary camera counter"
        );
    } else {
        assert_eq!(
            deck.frame.camera_clip_frame as i64,
            int(&state["camera_clip_frame"]),
            "{what}: camera"
        );
    }
    assert_eq!(
        held.evaluation.mapping.camera_clip_frame as i64,
        int(&state["camera_clip_frame"]),
        "{what}: offset camera"
    );
    assert_eq!(
        held.held_camera,
        state["held_camera"] == true,
        "{what}: held camera"
    );
    if !skip_instance_clocks {
        assert_eq!(
            deck.frame.instance_frame as i64,
            int(&state["instance_frame"]),
            "{what}: instance"
        );
        assert_eq!(
            held.held_instance,
            state["held_instance"] == true,
            "{what}: held instance"
        );
    }
}

fn lights_match(deck: &ScenePlan, expected: &Value, what: &str) {
    let expected = expected.as_array().unwrap();
    assert_eq!(deck.lights.len(), expected.len(), "{what}: lamps");
    for (light, expected) in deck.lights.iter().zip(expected) {
        assert_eq!(light.id, expected["id"].as_str().unwrap(), "{what}");
        // The oracle states 9 decimals of `intensity * flicker level`.
        near(
            light.intensity,
            real(&expected["effective_intensity"]),
            2e-9,
            &format!("{what}: {}", light.id),
        );
        assert!((0.0..=20.0).contains(&light.intensity));
        assert!(light.radius > 0.0);
        for channel in [light.color.r, light.color.g, light.color.b] {
            assert!((0.0..=1.0).contains(&channel));
        }
    }
    assert!(deck.lights.len() <= 4);
}

#[test]
fn the_analytic_scenes_agree_with_the_oracle_frame_by_frame() {
    let cases = cases();
    let scenarios = cases["analytic_probes"]["scenarios"].as_array().unwrap();
    assert_eq!(scenarios.len(), 12);
    let (mut probed, mut in_window, mut held_neighbour, mut lit) = (0, 0, 0, 0);
    let mut lengthened_scenes = Vec::new();
    for scenario in scenarios {
        let id = scenario["id"].as_str().unwrap();
        let (scene, lengthened) = scenario_scene(&cases, scenario);
        if lengthened {
            lengthened_scenes.push(id);
        }
        for frame in scenario["frames"].as_array().unwrap() {
            probed += 1;
            let at = int(&frame["edit_frame"]);
            let what = format!("{id} @ {at}");
            let primary = frame["primary"].as_str().unwrap();
            let plan = plan_at(&scene, at);
            deck_matches(
                &scene,
                primary,
                &plan,
                at,
                &frame["states"][primary],
                &what,
                lengthened && primary == "A",
            );
            lights_match(&plan, &frame["lights"][primary], &format!("{what} primary"));
            lit += usize::from(!plan.lights.is_empty());
            if frame["in_window"] != true {
                assert!(
                    plan.transition.is_none(),
                    "{what}: a dry cut is the primary deck alone"
                );
                assert!(plan.to_json().get("transition").is_none());
                continue;
            }
            in_window += 1;
            let neighbour = frame["neighbour"].as_str().unwrap();
            let t = plan
                .transition
                .as_deref()
                .unwrap_or_else(|| panic!("{what}: no window"));
            assert_eq!(
                t.primary.name(),
                if primary == "A" { "from" } else { "to" },
                "{what}"
            );
            deck_matches(
                &scene,
                neighbour,
                &t.other,
                at,
                &frame["states"][neighbour],
                &what,
                lengthened && neighbour == "A",
            );
            lights_match(
                &t.other,
                &frame["lights"][neighbour],
                &format!("{what} neighbour"),
            );
            lit += usize::from(!t.other.lights.is_empty());
            held_neighbour += usize::from(t.held_instance || t.held_camera);
            assert_eq!(t.frame_in_window, int(&frame["k"]), "{what}");
            // The oracle states 9 decimals.
            near(
                t.progress,
                real(&frame["transition"]["progress"]),
                1e-9,
                &what,
            );
            near(t.amount, real(&frame["transition"]["amount"]), 1e-9, &what);
            // The weight is the runtime's, never stored in the plan.
            assert!(plan.to_json()["transition"].get("weight").is_none());
        }
    }
    assert_eq!(probed, 116, "the oracle's frames");
    assert_eq!(
        lengthened_scenes,
        ["lit-center-held", "lit-end-held", "lit-start-held"],
        "the scenes whose shot A the importer would not accept as stated"
    );
    assert!(
        in_window >= 90 && held_neighbour >= 40 && lit >= 40,
        "{in_window} {held_neighbour} {lit}"
    );
}

// ---- progress, amount and the reference mix (T07, T16) -------------------------------------------

#[test]
fn progress_tables_and_extremes_agree_with_the_oracle() {
    let cases = cases();
    for table in cases["progress"]["tables"].as_array().unwrap() {
        let duration = int(&table["duration"]);
        let easing = table["easing"].as_str().unwrap();
        // Through the window of the spec edit's s1/s2 cut (centred, so the frames are `k`).
        let scene = spec_scene(vec![tr(
            json!({"durationFrames": duration, "easing": easing}),
        )]);
        let start = 48 - duration / 2;
        for row in table["frames"].as_array().unwrap() {
            let k = int(&row["k"]);
            let found = plan_at(&scene, start + k).transition.unwrap();
            assert_eq!(found.frame_in_window, k);
            near(
                found.progress,
                real(&row["progress"]),
                1e-15,
                &format!("{duration}/{easing}/{k}"),
            );
        }
    }
    // The extremes go through the same curve; the windows are too long for the spec edit.
    for row in cases["progress"]["extremes"].as_array().unwrap() {
        let (duration, k) = (real(&row["duration"]), real(&row["k"]));
        for name in ["linear", "easeInOutCubic", "easeOutCubic"] {
            let expected = &row[name];
            let found = FxEasing::parse(name).unwrap().apply((k + 0.5) / duration);
            assert!(found.is_finite() && (0.0..=1.0).contains(&found), "{row}");
            assert_eq!(expected["finite"], true);
            near(
                found,
                real(&expected["progress"]),
                1e-15,
                &format!("{name} {row}"),
            );
        }
    }
    for row in cases["progress"]["symmetry"].as_array().unwrap() {
        let duration = int(&row["duration"]);
        let easing = FxEasing::parse(row["easing"].as_str().unwrap()).unwrap();
        let progress = |k: i64| easing.apply((k as f64 + 0.5) / duration as f64);
        let worst = (0..duration)
            .map(|k| (progress(k) + progress(duration - 1 - k) - 1.0).abs())
            .fold(0.0, f64::max);
        assert!(worst <= 1e-12, "{row}: {worst}");
    }
}

/// The oracle's 8-bit sRGB <-> linear transfer, half-up rounding on encode.
fn decode(code: f64) -> f64 {
    let c = code / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn encode(linear: f64) -> f64 {
    let l = linear.clamp(0.0, 1.0);
    255.0
        * if l <= 0.003_130_8 {
            12.92 * l
        } else {
            1.055 * l.powf(1.0 / 2.4) - 0.055
        }
}

/// `out = mix(mix(a, b, progress), mix(a, b, shaped), amount)`, `shaped = progress` for `linear`
/// and `smoothstep(progress)` for `smooth`, in linear light: the oracle's `formula.pixel`, fed with
/// the numbers of the PLAN. A reference calculation, not a runtime.
fn mix_pixel(
    a: [f64; 3],
    b: [f64; 3],
    curve: &str,
    progress: f64,
    amount: f64,
) -> ([u8; 3], [f64; 3]) {
    let shaped = if curve == "smooth" {
        progress * progress * (3.0 - 2.0 * progress)
    } else {
        progress
    };
    let mut codes = [0; 3];
    let mut unrounded = [0.0; 3];
    for channel in 0..3 {
        let (a, b) = (decode(a[channel]), decode(b[channel]));
        let plain = a + (b - a) * progress;
        let curved = a + (b - a) * shaped;
        let out = encode(plain + (curved - plain) * amount);
        unrounded[channel] = out;
        codes[channel] = (out + 0.5).floor() as u8;
    }
    (codes, unrounded)
}

fn triple(value: &Value) -> [f64; 3] {
    let list = value.as_array().unwrap();
    [real(&list[0]), real(&list[1]), real(&list[2])]
}

/// A window of `duration` frames on the spec edit's s1/s2 cut, as the plan of its frame `k`.
fn window_frame(duration: i64, easing: &str, curve: &str, intensity: f64, k: i64) -> ScenePlan {
    let scene = spec_scene(vec![tr(json!({
        "durationFrames": duration, "easing": easing,
        "params": {"curve": curve, "intensity": intensity},
    }))]);
    plan_at(&scene, 48 - duration / 2 + k)
}

fn curve_of(plan: &ScenePlan) -> String {
    let t = plan.transition.as_deref().unwrap();
    match t.params.iter().find(|(name, _)| name == "curve").unwrap().1 {
        FxValue::Choice(curve) => curve.to_string(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn intensity_rows_give_the_clamped_amount_and_the_same_progress() {
    let cases = cases();
    for row in cases["progress"]["intensity"].as_array().unwrap() {
        let plan = window_frame(
            12,
            "easeInOutCubic",
            row["curve"].as_str().unwrap(),
            real(&row["intensity"]),
            int(&row["k"]),
        );
        let t = plan.transition.as_deref().unwrap();
        near(t.progress, real(&row["progress"]), 1e-15, &format!("{row}"));
        near(
            t.amount,
            real(&row["intensity"]).clamp(0.0, 1.0),
            0.0,
            &format!("{row}"),
        );
        assert_eq!(
            row["weight_is_zero"], false,
            "intensity 0 is not an identity of the effect"
        );
    }
}

#[test]
fn the_pixel_rows_agree_with_the_oracle_through_the_plans_numbers() {
    let cases = cases();
    let pixels = &cases["pixels"];
    for case in pixels["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let plan = window_frame(
            int(&case["duration"]),
            case["easing"].as_str().unwrap(),
            case["curve"].as_str().unwrap(),
            real(&case["intensity"]),
            int(&case["frame_in_window"]),
        );
        let t = plan.transition.as_deref().unwrap();
        near(t.progress, real(&case["progress"]), 1e-15, id);
        let (codes, unrounded) = mix_pixel(
            triple(&case["deck_a"]),
            triple(&case["deck_b"]),
            &curve_of(&plan),
            t.progress,
            t.amount,
        );
        let expected = &case["expected"];
        assert_eq!(
            codes.map(i64::from),
            [0, 1, 2].map(|i| int(&expected["rgb8"][i])),
            "{id}"
        );
        for channel in 0..3 {
            near(
                unrounded[channel],
                real(&expected["unrounded_255"][channel]),
                1e-5,
                id,
            );
        }
    }
    // The grid: six deck pairs, each probed at several (intensity, k).
    let grid = &pixels["grid"];
    let mut probed = 0;
    for probe in grid["probes"].as_array().unwrap() {
        let pair = &grid["decks"][int(&probe["pair"]) as usize];
        let plan = window_frame(
            int(&grid["duration"]),
            grid["easing"].as_str().unwrap(),
            grid["curve"].as_str().unwrap(),
            real(&probe["intensity"]),
            int(&probe["k"]),
        );
        let t = plan.transition.as_deref().unwrap();
        let (codes, _) = mix_pixel(
            triple(&pair["a"]),
            triple(&pair["b"]),
            &curve_of(&plan),
            t.progress,
            t.amount,
        );
        assert_eq!(
            codes.map(i64::from),
            [0, 1, 2].map(|i| int(&probe["rgb8"][i])),
            "{probe}"
        );
        probed += 1;
    }
    assert!(probed > 20, "{probed}");
}

#[test]
fn the_tracer_bullet_agrees_with_the_oracle() {
    let cases = cases();
    let bullet = &cases["tracer_bullet"];
    let scene = admit(with_transitions(
        corpus(),
        vec![tr(json!({
            "id": "tr-1", "fromShotId": "shot-a", "toShotId": "shot-b", "durationFrames": 12,
            "params": {"curve": "smooth", "intensity": 1},
        }))],
    ))
    .unwrap();
    let at = int(&bullet["edit_frame"]);
    let plan = plan_at(&scene, at);
    let t = plan.transition.as_deref().unwrap();
    assert_eq!(plan.frame.shot_id, "shot-a");
    assert_eq!(t.primary.name(), bullet["primary"].as_str().unwrap());
    assert_eq!(
        t.other.frame.shot_id,
        bullet["other_shot"].as_str().unwrap()
    );
    assert_eq!(t.frame_in_window, int(&bullet["frame_in_window"]));
    near(t.progress, real(&bullet["progress"]), 1e-9, "progress");
    // The neighbour at the real edit frame, with both clocks held (the oracle's row).
    let row = &bullet["other_deck_frame"];
    assert_eq!(t.other.frame.edit_frame as i64, int(&row["edit_frame"]));
    assert_eq!(
        t.other.frame.instance_frame as i64,
        int(&row["instance_frame"])
    );
    assert_eq!(
        t.other.frame.camera_clip_frame as i64,
        int(&row["camera_clip_frame"])
    );
    assert_eq!(
        (t.held_instance, t.held_camera),
        (row["held_instance"] == true, row["held_camera"] == true)
    );
    let window = &bullet["window"];
    assert_eq!(t.start_frame, int(&window["start"]));
    // The oracle states the shader weight, `mix(progress, smoothstep(progress), amount)`, which the
    // plan does NOT store: it keeps `progress` and `amount` and the runtime derives the weight.
    let smooth = t.progress * t.progress * (3.0 - 2.0 * t.progress);
    near(
        t.progress + (smooth - t.progress) * t.amount,
        real(&bullet["weight_not_stored_in_the_plan"]),
        1e-9,
        "weight",
    );
    assert!(plan.to_json()["transition"].get("weight").is_none());
    let constant = &bullet["pixel_constant_decks"];
    let (codes, unrounded) = mix_pixel(
        triple(&constant["deck_a"]),
        triple(&constant["deck_b"]),
        "smooth",
        t.progress,
        t.amount,
    );
    assert_eq!(
        codes.map(i64::from),
        [0, 1, 2].map(|i| int(&constant["expected"]["rgb8"][i]))
    );
    for channel in 0..3 {
        near(
            unrounded[channel],
            real(&constant["expected"]["unrounded_255"][channel]),
            1e-5,
            "tracer",
        );
    }
}

// ---- the neighbour's clips (T11) -------------------------------------------------------------------

#[test]
fn shot_clip_rows_agree_with_the_oracle() {
    let cases = cases();
    let mut compared = 0;
    let mut skipped = Vec::new();
    for row in cases["shot_clip_frames"]["rows"].as_array().unwrap() {
        let name = row["name"].as_str().unwrap();
        let (start, duration) = (int(&row["clip_start"]), int(&row["clip_duration"]));
        let (shot_start, shot_end) = (int(&row["shot_start"]), int(&row["shot_end"]));
        let at = int(&row["edit_frame"]);
        // The importer refuses a `shot` clip that leaves its shot (F062), so the oracle's rows with
        // a clip starting before the shot are an origin case with no destination document.
        if start < shot_start || start + duration > shot_end {
            skipped.push(name.to_string());
            continue;
        }
        assert_eq!(
            (shot_start, shot_end),
            (48, 96),
            "{name}: the spec edit's s2"
        );
        let tracks = json!([shot_clip("t", "c", "vignette", "s2", start, duration)]);
        // A window around the cut `s2` is neighbour of: s1/s2 (cut 48) for the head and frames
        // inside the shot, s2/s3 (cut 96) for the tail. The longest window is the whole of both.
        let (from, to, length) = if at < 48 {
            ("s1", "s2", 96)
        } else {
            ("s2", "s3", 96)
        };
        let scene = spec_scene_with_tracks(
            vec![tr(
                json!({"fromShotId": from, "toShotId": to, "durationFrames": length}),
            )],
            tracks,
        );
        let plan = plan_at(&scene, at);
        let passes: Vec<_> = if (48..96).contains(&at) {
            // The frame is inside s2: it is the primary deck, no border involved.
            plan.fx.iter().filter(|pass| pass.clip_id == "c").collect()
        } else {
            plan.transition
                .as_deref()
                .unwrap()
                .other
                .fx
                .iter()
                .filter(|pass| pass.clip_id == "c")
                .collect()
        };
        match row["frame"].as_i64() {
            None => assert!(passes.is_empty(), "{name}: the clip must not enter"),
            Some(frame) => {
                assert_eq!(passes.len(), 1, "{name}");
                assert_eq!(passes[0].frame_in_clip, frame - start, "{name}");
            }
        }
        compared += 1;
    }
    assert!(compared >= 9, "{compared} compared, skipped {skipped:?}");
    assert_eq!(skipped, ["covers-and-reaches-before-the-shot"]);
}

// ---- the resources of two decks (T09) -------------------------------------------------------------

fn union_json(plan: &ScenePlan) -> Value {
    let mut map = serde_json::Map::new();
    for (asset, resource) in plan.resource_union().unwrap() {
        map.insert(
            asset,
            json!({
                "contentHash": resource.content_hash, "relativePath": resource.relative_path,
                "revisionId": resource.revision_id,
                "used_by": resource.used_by.iter().map(|user| json!({
                    "deck": user.deck.name(), "node_id": user.node_id})).collect::<Vec<_>>(),
            }),
        );
    }
    Value::Object(map)
}

#[test]
fn the_resource_union_agrees_with_the_oracle() {
    let cases = cases();
    let oracle = &cases["resource_union"];
    let scene = admit(with_transitions(
        corpus(),
        vec![tr(
            json!({"id": "tr-1", "fromShotId": "shot-a", "toShotId": "shot-b"}),
        )],
    ))
    .unwrap();
    let plan = plan_at(&scene, 37);
    // Each deck keeps its own list, by the oracle's `decks`.
    for (deck, resources) in [
        (&plan, &oracle["decks"]["primary"]["resources"]),
        (
            &plan.transition.as_deref().unwrap().other,
            &oracle["decks"]["other"]["resources"],
        ),
    ] {
        for (asset, expected) in resources.as_object().unwrap() {
            let found = deck.resource(asset).unwrap_or_else(|| panic!("{asset}"));
            assert_eq!(
                found.content_hash,
                expected["contentHash"].as_str().unwrap()
            );
            assert_eq!(
                found.relative_path,
                expected["relativePath"].as_str().unwrap()
            );
            assert_eq!(found.revision_id, expected["revisionId"].as_str().unwrap());
            assert_eq!(found.used_by, [expected["used_by"][0].as_str().unwrap()]);
        }
    }
    assert_eq!(
        oracle["colliding_node_ids_between_decks"],
        json!(["backdrop", "figure"])
    );
    assert_eq!(
        union_json(&plan),
        oracle["union"]["value"],
        "node ids collide: a node is (deck, id)"
    );
    assert!(oracle["order"].as_str().is_some());
    assert_eq!(
        oracle["requires_added_by_a_transition"],
        json!(["fxTransition"])
    );
    assert!(plan.requires.iter().any(|c| c.as_str() == "fxTransition"));

    // An asset only the neighbour uses: shot-b's scene gets a `sky` plane.
    let mut bundle = with_transitions(
        corpus(),
        vec![tr(
            json!({"id": "tr-1", "fromShotId": "shot-a", "toShotId": "shot-b"}),
        )],
    );
    bundle["assets"].as_array_mut().unwrap().push(json!({
        "assetId": "sky", "revisionId": "rev-1", "contentHash": "synthetic-sha256-sky",
        "relativePath": "assets/sky.png", "widthPx": 8, "heightPx": 8,
        "alphaMode": "opaque", "colorSpace": "srgb",
    }));
    let scene_b = bundle["sceneDefinitions"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|d| d["id"] == "scene-b")
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
    let plan = plan_at(&scene_of(bundle), 37);
    assert!(plan.resource("sky").is_none());
    assert_eq!(
        union_json(&plan),
        oracle["only_the_neighbour_uses_an_asset"]["outcome"]["value"]
    );
    let sky = plan.resource_union().unwrap();
    assert_eq!(sky["sky"].used_by[0].deck, DeckRole::Other);

    // The same assetId with other content is no union: invalid, in the neighbour's list.
    let mut conflicting = plan_at(&scene, 37);
    conflicting.transition.as_mut().unwrap().other.resources[1].content_hash = "other".into();
    let error = conflicting.resource_union().unwrap_err();
    assert!(
        format!("{error:?}").contains("/transition/other/resources"),
        "{error:?}"
    );
    assert_eq!(oracle["conflict"]["outcome"]["ok"], false);
}

#[test]
fn the_plan_ceilings_are_the_oracles_and_its_rows_are_all_exercised() {
    let cases = cases();
    let ceilings = &cases["limits"]["ceilings"];
    assert_eq!(
        int(&ceilings["MAX_TRANSITION_PLAN_BYTES"]) as usize,
        tessel_editor_core::scene_plan::MAX_TRANSITION_PLAN_BYTES
    );
    assert_eq!(
        int(&ceilings["MAX_TRANSITION_PLAN_ITEMS"]) as usize,
        tessel_editor_core::scene_plan::MAX_TRANSITION_PLAN_ITEMS
    );
    assert_eq!(int(&ceilings["TRANSITION_PLAN_MAX_DEPTH"]), 1);
    // The seven rows are the ones `scene_fx_transition/limits.rs` runs, by name.
    let names: Vec<&str> = cases["limits"]["plan_rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "bytes-at-the-limit",
            "bytes-one-above",
            "items-at-the-limit",
            "items-one-above",
            "depth-1-is-the-only-depth",
            "depth-2-hostile-nesting",
            "depth-bomb-refused-without-recursing"
        ]
    );
}
