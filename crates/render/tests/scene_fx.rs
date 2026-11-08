//! F016: the vignette drawn by the real runtime (Node worker + headless Chrome, SwiftShader),
//! compared with the core's own reference of the pixel math (`directed_scene::fx::reference`),
//! which is written independently of the runtime's GLSL. The scene is the synthetic corpus of
//! `scripts/directed-scene-proof`; the effect is added by the core's atomic edit, not by hand, and
//! the plan the runtime draws is the one the core builds for the frame.
//!
//! What it proves: no effect and intensity 0 draw the very pixels of the baseline; intensity above
//! 0 changes the picture exactly where the reference says (a grid of probes, every one compared,
//! not only the corners) and nowhere near the centre; moving the centre moves the dark region; a
//! frame is the same picture however the playhead got there; a runtime asked for an effect it
//! does not draw is refused by name.
//!
//! A machine without Chrome or the runtime bundle skips these with the reason on stderr; set
//! `TESSEL_REQUIRE_RUNTIME=1` (CI) to turn the skip into a failure.

mod scene_support;

use scene_support::{plan, resources, scene};
use serde_json::json;
use tessel_editor_core::backend::FrameResult;
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::directed_scene::fx::apply_pass_to_srgb;
use tessel_editor_core::scene_edit::{FxOp, apply_fx};
use tessel_editor_core::scene_plan::{Capability, ScenePlan};
use tessel_editor_render::{
    FrameOptions, HANDSHAKE_TIMEOUT, ProcessTransport, RuntimeConfig, SceneRenderer,
};

/// 8-bit rounding in the GPU target and in the sRGB curves: a channel may move a unit or two.
const TOLERANCE: i32 = 3;

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

/// The corpus scene with a vignette over frames 0..48 (`params` are the vignette's own).
fn with_vignette(base: &DirectedScene, params: serde_json::Value) -> DirectedScene {
    let ops = [FxOp::from_json(&json!({
        "op": "add_clip", "effect": "vignette", "start_frame": 0, "duration_frames": 48,
        "id": "v1", "params": params,
    }))
    .unwrap()];
    apply_fx(base, &ops).unwrap().candidate
}

fn grid(width: u32, height: u32) -> Vec<(u32, u32)> {
    let mut points = Vec::new();
    for row in 0..6 {
        for column in 0..9 {
            points.push(((column * (width - 1)) / 8, (row * (height - 1)) / 5));
        }
    }
    points
}

fn draw(
    renderer: &mut SceneRenderer<ProcessTransport>,
    scene: &DirectedScene,
    edit_frame: i64,
) -> (ScenePlan, FrameResult) {
    let plan = plan(scene, edit_frame);
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

/// What the reference says each probe of `shaded` should be, from the baseline probes.
fn assert_against_reference(
    plan: &ScenePlan,
    baseline: &FrameResult,
    shaded: &FrameResult,
    what: &str,
) -> i32 {
    let (width, height) = (plan.output.width as usize, plan.output.height as usize);
    let points = grid(width as u32, height as u32);
    let mut worst = 0;
    for ((point, base), read) in points.iter().zip(&baseline.probes).zip(&shaded.probes) {
        let mut want = [f64::from(base[0]), f64::from(base[1]), f64::from(base[2])];
        want.iter_mut().for_each(|channel| *channel /= 255.0);
        for pass in &plan.fx {
            want = apply_pass_to_srgb(
                pass,
                want,
                point.0 as usize,
                point.1 as usize,
                width,
                height,
            )
            .unwrap();
        }
        for channel in 0..3 {
            let expected = (want[channel] * 255.0).round() as i32;
            let delta = (i32::from(read[channel]) - expected).abs();
            worst = worst.max(delta);
            assert!(
                delta <= TOLERANCE,
                "{what}: probe {point:?} channel {channel} read {read:?}, the reference says {expected} (baseline {base:?})"
            );
        }
    }
    worst
}

#[test]
fn the_runtime_advertises_output_fx() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let capabilities = renderer.ensure_ready().unwrap().clone();
    let advertised = capabilities.raw["capabilities"]["directed_scene"]["capabilities"].to_string();
    assert!(
        advertised.contains(Capability::OutputFx.as_str()),
        "{advertised}"
    );
    renderer.shutdown();
}

#[test]
fn the_vignette_matches_the_reference_and_zero_intensity_is_the_baseline() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let base = scene("bundle.json");
    let (baseline_plan, baseline) = draw(&mut renderer, &base, 12);
    assert!(baseline_plan.fx.is_empty());
    assert!(
        !baseline_plan.requires.contains(&Capability::OutputFx),
        "a scene without effects does not ask for the capability"
    );

    // Intensity 0: the clip exists, the plan carries its pass at amount 0, the pixels do not move.
    let zero = with_vignette(&base, json!({ "intensity": 0.0 }));
    let (zero_plan, zeroed) = draw(&mut renderer, &zero, 12);
    assert_eq!(zero_plan.fx.len(), 1);
    assert_eq!(zero_plan.fx[0].amount, 0.0);
    assert_eq!(
        zeroed.probes, baseline.probes,
        "intensity 0 is the baseline"
    );

    // Outside the clip (frame 60 is past it) the same scene is the baseline again.
    let (_, outside) = draw(&mut renderer, &zero, 60);
    let (_, outside_base) = draw(&mut renderer, &base, 60);
    assert_eq!(outside.probes, outside_base.probes);

    // Full intensity: every probe of the grid against the reference, the centre untouched, the
    // corners darker, and a real difference somewhere (the comparison discriminates).
    let strong = with_vignette(&base, json!({ "intensity": 1.0 }));
    let (plan, shaded) = draw(&mut renderer, &strong, 12);
    assert_eq!(plan.fx[0].amount, 1.0);
    assert!(plan.requires.contains(&Capability::OutputFx));
    let worst = assert_against_reference(&plan, &baseline, &shaded, "default centre");
    assert_ne!(shaded.probes, baseline.probes);
    let points = grid(320, 180);
    let at = |result: &FrameResult, x: u32, y: u32| {
        let slot = points.iter().position(|point| *point == (x, y)).unwrap();
        result.probes[slot]
    };
    let corner = at(&shaded, 0, 0);
    let corner_base = at(&baseline, 0, 0);
    assert!(
        i32::from(corner[0]) + 20 < i32::from(corner_base[0]) || corner_base[0] < 30,
        "the corner darkens: {corner:?} against {corner_base:?}"
    );
    let (mx, my) = (4 * 319 / 8, 2 * 179 / 5);
    let middle_changed = (0..3)
        .map(|c| (i32::from(at(&shaded, mx, my)[c]) - i32::from(at(&baseline, mx, my)[c])).abs())
        .max()
        .unwrap();
    assert!(
        middle_changed <= TOLERANCE,
        "the middle stays: {middle_changed}"
    );

    // A discriminating second picture: the centre at the bottom right moves the dark region.
    let moved = with_vignette(
        &base,
        json!({ "intensity": 1.0, "center": { "x": 1.0, "y": 1.0 } }),
    );
    let (moved_plan, moved_result) = draw(&mut renderer, &moved, 12);
    let moved_worst =
        assert_against_reference(&moved_plan, &baseline, &moved_result, "moved centre");
    assert_ne!(moved_result.probes, shaded.probes);

    // Same edit frame, same picture, however the playhead got there.
    let (_, again) = draw(&mut renderer, &strong, 12);
    assert_eq!(again.hash, shaded.hash);
    let (_, elsewhere) = draw(&mut renderer, &strong, 40);
    let (_, back) = draw(&mut renderer, &strong, 12);
    assert_eq!(back.hash, shaded.hash, "after a seek away and back");
    assert_ne!(elsewhere.hash, shaded.hash);
    eprintln!(
        "EVIDENCE vignette: worst channel difference to the reference {worst} / {moved_worst}"
    );
    renderer.shutdown();
}

#[test]
fn a_fade_scales_the_effect_and_a_runtime_without_the_capability_is_refused() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let base = scene("bundle.json");
    let (_, baseline) = draw(&mut renderer, &base, 12);
    let faded = {
        let ops = [
            FxOp::from_json(&json!({
                "op": "add_clip", "effect": "vignette", "start_frame": 10, "duration_frames": 20,
                "id": "v1", "params": { "intensity": 1.0 },
            }))
            .unwrap(),
            FxOp::from_json(&json!({
                "op": "set_field", "clip": "v1", "field": "fade_in_frames", "value": 10,
            }))
            .unwrap(),
        ];
        apply_fx(&base, &ops).unwrap().candidate
    };
    // Frame 12 is the third frame of the clip: weight (2 + 0.5) / 10.
    let (plan, shaded) = draw(&mut renderer, &faded, 12);
    assert!((plan.fx[0].weight - 0.25).abs() < 1e-12);
    assert!((plan.fx[0].amount - 0.25).abs() < 1e-12);
    assert_against_reference(&plan, &baseline, &shaded, "fade");

    // The plan says it needs `outputFx`; a runtime that lacks it refuses by name (the core's
    // capability check, which the renderer applies before sending anything).
    let without: Vec<Capability> = Capability::ALL
        .into_iter()
        .filter(|capability| *capability != Capability::OutputFx)
        .collect();
    assert_eq!(plan.missing_capabilities(&without), [Capability::OutputFx]);
    renderer.shutdown();
}
