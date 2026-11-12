//! F060: a point light drawn by the real runtime (Node worker + headless Chrome, SwiftShader) over
//! the synthetic corpus of `scripts/directed-scene-proof`, with the plan the core builds for the
//! frame. The pixels are compared with a reference written HERE (decode sRGB, multiply by
//! `1 + colour * intensity * max(0, n.l) * falloff^2`, encode), from the numbers of the plan, never
//! with the runtime's shader or the core's `lambert_falloff`.
//!
//! What it proves: the sky plane (opaque, behind everything) is lit where the reference says, at
//! every probe the other planes do not cover, and a real difference exists (the comparison
//! discriminates); a light parented to a node follows it across frames (the plan's world position
//! is the parent's world translation plus the stored offset, summed here); a hidden light, an
//! intensity of zero and a light behind the picture leave the baseline; the editor's range sphere
//! and markers are not part of the frame (they are not in the plan: `crates/render` never sees
//! them).
//!
//! What it does NOT prove: splats (they are not lit, by design: `docs/features/scene-light.md`),
//! a physical WKWebView, performance.
//!
//! A machine without Chrome or the runtime bundle skips these with the reason on stderr; set
//! `TESSEL_REQUIRE_RUNTIME=1` (CI) to turn the skip into a failure.

mod scene_support;

use std::fs;

use scene_support::{corpus, resources};
use serde_json::{Value, json};
use tessel_editor_core::backend::FrameResult;
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::scene_plan::{Capability, ScenePlan};
use tessel_editor_render::scene::import_gated;
use tessel_editor_render::{
    FrameOptions, HANDSHAKE_TIMEOUT, ProcessTransport, RuntimeConfig, SceneRenderer,
};

const TOLERANCE: f64 = 3.0;

fn renderer() -> Option<SceneRenderer<ProcessTransport>> {
    let config = match RuntimeConfig::discover() {
        Ok(config) => config,
        Err(setup) => return skip(&setup.message()),
    };
    let backend = tessel_editor_core::bridge::BridgeBackend::new(
        "three-worker",
        ProcessTransport::new(config),
        HANDSHAKE_TIMEOUT,
    );
    let mut renderer = SceneRenderer::new(backend);
    match renderer.ensure_ready() {
        Ok(_) => Some(renderer),
        Err(error) if error.code == "backend_unavailable" && !error.message.contains("Rebuild") => {
            skip(&error.message)
        }
        Err(error) => panic!("{error:?}"),
    }
}

fn skip(reason: &str) -> Option<SceneRenderer<ProcessTransport>> {
    assert!(
        std::env::var("TESSEL_REQUIRE_RUNTIME").as_deref() != Ok("1"),
        "the runtime is required but unavailable: {reason}"
    );
    eprintln!("SKIPPED (runtime unavailable): {reason}");
    None
}

/// The corpus scene with `lights` in `lighting` (ambient 1: the art's own light untouched).
fn lit(lights: Value) -> DirectedScene {
    let mut bundle: Value =
        serde_json::from_str(&fs::read_to_string(corpus().join("bundle.json")).unwrap()).unwrap();
    bundle["sceneDefinitions"][0]["lighting"] = json!({ "ambient": 1, "lights": lights });
    import_gated(&bundle.to_string()).unwrap()
}

fn grid(width: u32, height: u32) -> Vec<(u32, u32)> {
    let mut points = Vec::new();
    for row in 0..12 {
        for column in 0..17 {
            points.push(((column * (width - 1)) / 16, (row * (height - 1)) / 11));
        }
    }
    points
}

fn draw(
    renderer: &mut SceneRenderer<ProcessTransport>,
    scene: &DirectedScene,
    edit_frame: i64,
) -> (ScenePlan, FrameResult) {
    let plan = ScenePlan::build(scene, edit_frame).unwrap();
    let points = grid(plan.output.width as u32, plan.output.height as u32);
    let result = renderer
        .render(
            &plan,
            &resources(1, &plan),
            &FrameOptions {
                probes: points,
                ..FrameOptions::default()
            },
        )
        .unwrap();
    (plan, result)
}

// ---- the oracle, written in this file ---------------------------------------------------------

type V3 = [f64; 3];

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn unit(a: V3) -> V3 {
    let length = dot(a, a).sqrt();
    [a[0] / length, a[1] / length, a[2] / length]
}
fn vec3(value: &Value) -> V3 {
    [
        value["x"].as_f64().unwrap(),
        value["y"].as_f64().unwrap(),
        value["z"].as_f64().unwrap(),
    ]
}

fn lin(byte: f64) -> f64 {
    let c = byte / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}
fn enc(linear: f64) -> f64 {
    let x = linear.clamp(0.0, 1.0);
    255.0
        * if x <= 0.0031308 {
            12.92 * x
        } else {
            1.055 * x.powf(1.0 / 2.4) - 0.055
        }
}

/// A node of the plan as the oracle needs it: where its plane is and how big.
struct Plane {
    id: String,
    centre: V3,
    right: V3,
    up: V3,
    normal: V3,
    half: [f64; 2],
}

fn planes(wire: &Value) -> Vec<Plane> {
    wire["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|node| node["kind"] == "imagePlane")
        .map(|node| {
            let m: Vec<f64> = node["worldMatrix"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_f64().unwrap())
                .collect();
            let right = [m[0], m[1], m[2]];
            let up = [m[4], m[5], m[6]];
            Plane {
                id: node["id"].as_str().unwrap().to_string(),
                centre: [m[12], m[13], m[14]],
                right: unit(right),
                up: unit(up),
                normal: unit(cross(right, up)),
                half: [
                    node["widthSceneUnits"].as_f64().unwrap() * dot(right, right).sqrt() / 2.0,
                    node["heightSceneUnits"].as_f64().unwrap() * dot(up, up).sqrt() / 2.0,
                ],
            }
        })
        .collect()
}

struct Camera {
    position: V3,
    forward: V3,
    right: V3,
    up: V3,
    tan_half: f64,
    aspect: f64,
}

fn camera(wire: &Value) -> Camera {
    let position = vec3(&wire["camera"]["position"]);
    let target = vec3(&wire["camera"]["target"]);
    let forward = unit(sub(target, position));
    let right = unit(cross(forward, [0.0, 1.0, 0.0]));
    let up = cross(right, forward);
    Camera {
        position,
        forward,
        right,
        up,
        tan_half: (wire["camera"]["fovDeg"].as_f64().unwrap().to_radians() / 2.0).tan(),
        aspect: wire["output"]["width"].as_f64().unwrap()
            / wire["output"]["height"].as_f64().unwrap(),
    }
}

impl Camera {
    fn ray(&self, x: u32, y: u32, width: u32, height: u32) -> V3 {
        let ndc_x = (f64::from(x) + 0.5) / f64::from(width) * 2.0 - 1.0;
        let ndc_y = 1.0 - (f64::from(y) + 0.5) / f64::from(height) * 2.0;
        let mut direction = self.forward;
        for (axis, component) in direction.iter_mut().enumerate() {
            *component += ndc_x * self.tan_half * self.aspect * self.right[axis]
                + ndc_y * self.tan_half * self.up[axis];
        }
        direction
    }
}

/// Where the ray meets `plane`, and whether inside its rectangle grown by `margin` scene units.
fn hit(camera: &Camera, direction: V3, plane: &Plane, margin: f64) -> Option<(V3, bool)> {
    let denominator = dot(plane.normal, direction);
    if denominator.abs() < 1e-9 {
        return None;
    }
    let t = dot(plane.normal, sub(plane.centre, camera.position)) / denominator;
    if t <= 0.0 {
        return None;
    }
    let point = [
        camera.position[0] + t * direction[0],
        camera.position[1] + t * direction[1],
        camera.position[2] + t * direction[2],
    ];
    let local = sub(point, plane.centre);
    let inside = dot(local, plane.right).abs() <= plane.half[0] + margin
        && dot(local, plane.up).abs() <= plane.half[1] + margin;
    Some((point, inside))
}

struct Lamp {
    position: V3,
    colour: V3,
    intensity: f64,
    radius: f64,
}

fn lamps(wire: &Value) -> Vec<Lamp> {
    wire["lights"]
        .as_array()
        .map(|lights| {
            lights
                .iter()
                .filter(|light| light["hidden"] != true)
                .map(|light| Lamp {
                    position: vec3(&light["position"]),
                    colour: [
                        light["color"]["r"].as_f64().unwrap(),
                        light["color"]["g"].as_f64().unwrap(),
                        light["color"]["b"].as_f64().unwrap(),
                    ],
                    intensity: light["intensity"].as_f64().unwrap(),
                    radius: light["radius"].as_f64().unwrap(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The pixel of `base` (an unlit probe) the reference says the lamps make at `point` of `plane`.
fn lit_pixel(base: [u8; 4], lamps: &[Lamp], point: V3, plane: &Plane, camera: &Camera) -> [f64; 3] {
    // A double-sided plane is lit on the side the camera sees.
    let normal = if dot(plane.normal, sub(camera.position, point)) < 0.0 {
        plane.normal.map(|c| -c)
    } else {
        plane.normal
    };
    let mut extra = [0.0; 3];
    for lamp in lamps {
        let to_lamp = sub(lamp.position, point);
        let distance = dot(to_lamp, to_lamp).sqrt();
        let lambert = (dot(normal, to_lamp) / distance).max(0.0);
        let once = 1.0 / (1.0 + (distance / lamp.radius).powi(2));
        let strength = lambert * once * once * lamp.intensity;
        for (channel, sum) in extra.iter_mut().enumerate() {
            *sum += lamp.colour[channel] * strength;
        }
    }
    [0, 1, 2].map(|channel| enc(lin(f64::from(base[channel])) * (1.0 + extra[channel])))
}

/// Every probe the sky alone shows (no other plane within half a unit of its ray), compared with
/// the reference. Returns how many probes were compared and the worst channel difference.
fn assert_sky_lit(
    wire: &Value,
    baseline: &FrameResult,
    lit_frame: &FrameResult,
    what: &str,
) -> (usize, f64, f64) {
    let (width, height) = (
        wire["output"]["width"].as_u64().unwrap() as u32,
        wire["output"]["height"].as_u64().unwrap() as u32,
    );
    let planes = planes(wire);
    let camera = camera(wire);
    let lamps = lamps(wire);
    let sky = planes.iter().find(|plane| plane.id == "sky").unwrap();
    let points = grid(width, height);
    let (mut compared, mut worst, mut biggest_change) = (0, 0.0_f64, 0.0_f64);
    for ((point, base), read) in points.iter().zip(&baseline.probes).zip(&lit_frame.probes) {
        let direction = camera.ray(point.0, point.1, width, height);
        let covered = planes
            .iter()
            .filter(|plane| plane.id != "sky")
            .any(|plane| hit(&camera, direction, plane, 0.5).is_some_and(|(_, inside)| inside));
        let Some((world, true)) = hit(&camera, direction, sky, -0.2) else {
            continue;
        };
        if covered {
            continue;
        }
        let want = lit_pixel(*base, &lamps, world, sky, &camera);
        compared += 1;
        for channel in 0..3 {
            let delta = (f64::from(read[channel]) - want[channel]).abs();
            worst = worst.max(delta);
            assert!(
                delta <= TOLERANCE,
                "{what}: probe {point:?} channel {channel} read {read:?}, the reference says {:.1} (baseline {base:?})",
                want[channel]
            );
            biggest_change =
                biggest_change.max((f64::from(read[channel]) - f64::from(base[channel])).abs());
        }
    }
    (compared, worst, biggest_change)
}

fn plan_wire(plan: &ScenePlan) -> Value {
    plan.to_json()
}

// ---- the tests --------------------------------------------------------------------------------

fn lamp_json(extra: Value) -> Value {
    let mut light = json!({
        "id": "lamp", "position": {"x": 0.0, "y": 1.0, "z": 2.5},
        "color": {"r": 1.0, "g": 0.8, "b": 0.5}, "intensity": 3.0, "radius": 5.0, "flicker": 0.0,
    });
    for (key, value) in extra.as_object().unwrap() {
        light[key] = value.clone();
    }
    light
}

#[test]
fn the_runtime_lists_point_lights_and_the_plan_asks_for_them_only_when_a_light_is_visible() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let capabilities = renderer.ensure_ready().unwrap().clone();
    let advertised = capabilities.raw["capabilities"]["directed_scene"]["capabilities"].to_string();
    assert!(
        advertised.contains(Capability::PointLights.as_str()),
        "{advertised}"
    );
    let unlit = ScenePlan::build(&lit(json!([])), 0).unwrap();
    assert!(!unlit.requires.contains(&Capability::PointLights));
    let visible = ScenePlan::build(&lit(json!([lamp_json(json!({}))])), 0).unwrap();
    assert!(visible.requires.contains(&Capability::PointLights));
    let hidden = ScenePlan::build(&lit(json!([lamp_json(json!({"hidden": true}))])), 0).unwrap();
    assert!(!hidden.requires.contains(&Capability::PointLights));
    renderer.shutdown();
}

#[test]
fn a_lit_corpus_frame_matches_the_reference_and_the_baseline_cases_stay_the_baseline() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let (baseline_plan, baseline) = draw(&mut renderer, &lit(json!([])), 0);
    assert!(baseline_plan.lights.is_empty());

    // One lamp in front of the sky, near enough to matter: every sky probe against the reference.
    let scene = lit(json!([lamp_json(json!({}))]));
    let (plan, shaded) = draw(&mut renderer, &scene, 0);
    let wire = plan_wire(&plan);
    assert_eq!(wire["lights"].as_array().unwrap().len(), 1);
    let (compared, worst, change) = assert_sky_lit(&wire, &baseline, &shaded, "one lamp");
    assert!(compared >= 20, "only {compared} probes show the sky alone");
    assert!(
        change >= 10.0,
        "the light changes the picture by {change}: the comparison must discriminate"
    );
    assert_ne!(shaded.hash, baseline.hash);

    // Two lamps add; the second changes the picture again, against the same reference.
    let two = lit(json!([
        lamp_json(json!({})),
        lamp_json(
            json!({"id": "cold", "position": {"x": -3.0, "y": 0.0, "z": 2.0},
            "color": {"r": 0.2, "g": 0.4, "b": 1.0}, "intensity": 2.0, "radius": 4.0})
        ),
    ]));
    let (two_plan, two_frame) = draw(&mut renderer, &two, 0);
    let (two_compared, two_worst, _) =
        assert_sky_lit(&plan_wire(&two_plan), &baseline, &two_frame, "two lamps");
    assert!(two_compared >= 20);
    assert_ne!(two_frame.hash, shaded.hash);

    // Intensity 0, a hidden lamp and a lamp behind the picture are the baseline.
    for (what, extra, exact) in [
        ("hidden", json!({"hidden": true, "intensity": 20.0}), true),
        ("intensity zero", json!({"intensity": 0.0}), false),
        (
            "behind the sky",
            json!({"position": {"x": 0.0, "y": 1.0, "z": -9.0}}),
            false,
        ),
    ] {
        let (_, frame) = draw(&mut renderer, &lit(json!([lamp_json(extra)])), 0);
        if exact {
            assert_eq!(frame.hash, baseline.hash, "{what}: the very picture");
        }
        for (read, base) in frame.probes.iter().zip(&baseline.probes) {
            for channel in 0..3 {
                assert!(
                    (i32::from(read[channel]) - i32::from(base[channel])).abs() <= 1,
                    "{what}: {read:?} against {base:?}"
                );
            }
        }
    }
    eprintln!(
        "EVIDENCE point light: {compared} sky probes, worst channel difference {worst:.2} (one lamp) / {two_worst:.2} (two)"
    );
    renderer.shutdown();
}

#[test]
fn a_lamp_parented_to_a_node_follows_it_across_frames_and_the_picture_follows_the_lamp() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let offset = [0.0, 1.5, 2.0];
    let scene = lit(json!([lamp_json(
        json!({"parent": "hero", "position": {"x": offset[0], "y": offset[1], "z": offset[2]}})
    )]));
    let mut positions = Vec::new();
    for frame in [0, 30] {
        let (plan, lit_frame) = draw(&mut renderer, &scene, frame);
        let wire = plan_wire(&plan);
        let hero = wire["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["id"] == "hero")
            .unwrap();
        let world: Vec<f64> = hero["worldMatrix"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_f64().unwrap())
            .collect();
        // The oracle: parent's world translation plus the stored offset, summed here.
        let want = [
            world[12] + offset[0],
            world[13] + offset[1],
            world[14] + offset[2],
        ];
        let got = vec3(&wire["lights"][0]["position"]);
        for axis in 0..3 {
            assert!(
                (got[axis] - want[axis]).abs() < 1e-9,
                "frame {frame}: {got:?} against {want:?}"
            );
        }
        positions.push((got, lit_frame.hash));
    }
    // The hero moves between the two frames (the corpus animates it), so does the lamp and the picture.
    assert_ne!(positions[0].0, positions[1].0, "the lamp followed the node");
    assert_ne!(positions[0].1, positions[1].1);
    renderer.shutdown();
}
