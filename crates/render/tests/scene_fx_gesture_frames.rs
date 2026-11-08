//! F016 / BL082.6 / BL082.17: the pixels of an effect gesture, drawn by the real runtime (Node
//! worker + headless Chrome, SwiftShader) from the plans the CORE hands out.
//!
//! The scene is edited through `headless::execute` (the same handlers as the window and the JSONL
//! session). What it proves: the candidate of a gesture is drawn BEFORE the commit, and the
//! document's plan stays the old picture meanwhile; the picture of the commit is the picture of the
//! preview; cancel returns the picture of the document; one undo returns the pixels before the
//! drag and redo the pixels after it; every drawn picture agrees with the core's reference of the
//! pixel math (written independently of the runtime's GLSL) on a grid of probes.
//!
//! A machine without Chrome or the runtime bundle skips with the reason on stderr; set
//! `TESSEL_REQUIRE_RUNTIME=1` (CI) to turn the skip into a failure.

mod scene_support;

use scene_support::resources;
use serde_json::{Value, json};
use tessel_editor_core::backend::FrameResult;
use tessel_editor_core::directed_scene::fx::apply_pass_to_srgb;
use tessel_editor_core::scene_plan::ScenePlan;
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::{EditorState, headless};
use tessel_editor_render::{
    FrameOptions, HANDSHAKE_TIMEOUT, ProcessTransport, RuntimeConfig, SceneRenderer,
};

const TOLERANCE: i32 = 3;
const FRAME: i64 = 12;

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

fn run(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
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

/// The plan the core serves for the frame, optionally from a gesture's candidate, drawn.
fn draw(
    renderer: &mut SceneRenderer<ProcessTransport>,
    state: &mut EditorState,
    gesture: Option<&Value>,
) -> (ScenePlan, FrameResult) {
    let mut request = json!({"op":"scene.plan","scene":"street","frame":FRAME});
    if let Some(gesture) = gesture {
        request["gesture"] = gesture.clone();
    }
    let answer = run(state, request);
    let plan = ScenePlan::from_json(&answer["plan"]).unwrap();
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

/// Every probe of `shaded` against the reference applied to the baseline probe.
fn against_reference(plan: &ScenePlan, baseline: &FrameResult, shaded: &FrameResult, what: &str) {
    let (width, height) = (plan.output.width as usize, plan.output.height as usize);
    let points = grid(width as u32, height as u32);
    for ((point, base), read) in points.iter().zip(&baseline.probes).zip(&shaded.probes) {
        let mut want = [0, 1, 2].map(|c| f64::from(base[c]) / 255.0);
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
            assert!(
                (i32::from(read[channel]) - expected).abs() <= TOLERANCE,
                "{what}: probe {point:?} channel {channel} read {read:?}, the reference says {expected}"
            );
        }
    }
}

fn depth(state: &EditorState) -> u64 {
    state.history.to_json()["undo_depth"].as_u64().unwrap()
}

#[test]
fn the_candidate_is_drawn_before_the_commit_and_undo_redo_restore_the_pixels() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let bundle = std::fs::read_to_string(scene_support::corpus().join("bundle.json")).unwrap();
    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    run(
        &mut state,
        json!({"op":"scene.attach","id":"street","bundle":serde_json::from_str::<Value>(&bundle).unwrap()}),
    );

    let (baseline_plan, baseline) = draw(&mut renderer, &mut state, None);
    assert!(baseline_plan.fx.is_empty());

    // The committed document: a vignette at 0.2.
    run(
        &mut state,
        json!({"op":"scene.fx.edit","scene":"street","ops":[{"op":"add_clip","effect":"vignette",
            "start_frame":0,"duration_frames":48,"id":"v1","params":{"intensity":0.2}}]}),
    );
    let (document_plan, document) = draw(&mut renderer, &mut state, None);
    against_reference(&document_plan, &baseline, &document, "document at 0.2");
    assert_ne!(document.probes, baseline.probes);
    let depth_before = depth(&state);

    // A drag to 1.0: the candidate is drawn, the document is not touched.
    let began = run(
        &mut state,
        json!({"op":"scene.fx.gesture.begin","scene":"street","clip":"v1"}),
    );
    let id = began["gesture"].clone();
    run(
        &mut state,
        json!({"op":"scene.fx.gesture.update","gesture":id,
            "ops":[{"op":"set_param","clip":"v1","param":"intensity","value":1.0}]}),
    );
    let (preview_plan, preview) = draw(&mut renderer, &mut state, Some(&id));
    against_reference(&preview_plan, &baseline, &preview, "preview at 1.0");
    assert_ne!(
        preview.probes, document.probes,
        "the drag is visible before the commit"
    );
    let (_, during) = draw(&mut renderer, &mut state, None);
    assert_eq!(
        during.hash, document.hash,
        "the document still draws the old picture"
    );
    assert_eq!(depth(&state), depth_before, "a drag records nothing");

    // Cancel: the document's picture, nothing recorded, the gesture is gone.
    run(
        &mut state,
        json!({"op":"scene.fx.gesture.cancel","gesture":id}),
    );
    let (_, cancelled) = draw(&mut renderer, &mut state, None);
    assert_eq!(cancelled.hash, document.hash);
    assert_eq!(depth(&state), depth_before);

    // The same drag, committed: the picture of the commit is the picture of the preview.
    let began = run(
        &mut state,
        json!({"op":"scene.fx.gesture.begin","scene":"street","clip":"v1"}),
    );
    let id = began["gesture"].clone();
    run(
        &mut state,
        json!({"op":"scene.fx.gesture.update","gesture":id,
            "ops":[{"op":"set_param","clip":"v1","param":"intensity","value":1.0}]}),
    );
    run(
        &mut state,
        json!({"op":"scene.fx.gesture.commit","gesture":id}),
    );
    assert_eq!(
        depth(&state),
        depth_before + 1,
        "one undo entry for the whole drag"
    );
    let (_, committed) = draw(&mut renderer, &mut state, None);
    assert_eq!(
        committed.hash, preview.hash,
        "commit draws what the preview drew"
    );

    // Undo restores the pixels before the drag, redo the pixels after it.
    run(&mut state, json!({"op":"history.undo"}));
    let (_, undone) = draw(&mut renderer, &mut state, None);
    assert_eq!(undone.hash, document.hash, "undo restores the pixels");
    run(&mut state, json!({"op":"history.redo"}));
    let (_, redone) = draw(&mut renderer, &mut state, None);
    assert_eq!(redone.hash, preview.hash, "redo restores the pixels");

    // Back past the insertion: the baseline picture, byte for byte.
    run(&mut state, json!({"op":"history.undo"}));
    run(&mut state, json!({"op":"history.undo"}));
    let (plan, back) = draw(&mut renderer, &mut state, None);
    assert!(plan.fx.is_empty());
    assert_eq!(back.hash, baseline.hash, "no effect is the baseline");
    eprintln!(
        "EVIDENCE fx gesture frames: preview/commit/undo/redo hashes agree; baseline restored"
    );
    renderer.shutdown();
}
