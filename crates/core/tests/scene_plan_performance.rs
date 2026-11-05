//! BL064 — `PlanTimeline` returns, for every frame and in every order, exactly what
//! `ScenePlan::build_with` returns: the same plan (value and wire JSON byte for byte) or the same
//! error. Public library API only: no GPUI, window, runtime, socket, CLI or IO beyond fixtures.
//!
//! What is proved here, and what is not:
//!
//! - **Equivalence** on the 55 + 47 oracle corpus bundles that import (`docs/migration/fixtures`,
//!   valid and refused-by-a-gate alike), in both modes, over every frame plus one on each side of
//!   the edit; and on synthetic scale scenes without private media. The suite asserts it saw both
//!   plans and refusals, so a corpus that quietly turned into only errors would fail.
//! - **Order independence and reuse:** one timeline answers forward, backward, shuffled and
//!   repeated frames, and each answer equals a fresh `build_with`.
//! - **Generation switches:** timelines of two generations of a scene (an edit between them) live
//!   at once and each keeps answering for its own scene.
//! - **Errors are shared:** a layout that cannot be built fails the same way for every frame that
//!   needs it, without stopping the frames that do not.
//! - **Threads:** a timeline asked from several threads gives the sequential answers.
//! - **Not proved:** speed (that is the benchmark, `scripts/scene-plan-performance/`), the pre-change
//!   output (that is `scene_plan_performance_golden.rs`) or any renderer.

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::scene_plan::{PlanError, PlanOptions, PlanTimeline, ScenePlan};

#[path = "scene_plan_performance_support/mod.rs"]
mod support;

use support::{
    SHAPES, Shape, corpus_frames, corpus_scenes, outcome_text, shape_named, shuffled,
    spread_frames, synthetic_bundle, synthetic_scene,
};

const BOTH_MODES: [PlanOptions; 2] = [
    PlanOptions {
        allow_partial: false,
    },
    PlanOptions {
        allow_partial: true,
    },
];

fn direct(scene: &DirectedScene, frame: i64, options: PlanOptions) -> Result<ScenePlan, PlanError> {
    ScenePlan::build_with(scene, frame, options)
}

/// Asserts the two planners agree on `frame`, as values and as canonical text. Returns whether it
/// was a plan.
fn assert_same(
    label: &str,
    scene: &DirectedScene,
    timeline: &PlanTimeline<'_>,
    frame: i64,
    options: PlanOptions,
) -> bool {
    let expected = direct(scene, frame, options);
    let actual = timeline.build_with(frame, options);
    assert_eq!(
        actual, expected,
        "{label}: frame {frame}, partial {}",
        options.allow_partial
    );
    assert_eq!(
        outcome_text(&actual),
        outcome_text(&expected),
        "{label}: frame {frame} text"
    );
    expected.is_ok()
}

#[test]
fn a_timeline_can_be_shared_between_threads() {
    fn assert_sync<T: Sync + Send>() {}
    assert_sync::<PlanTimeline<'static>>();
}

#[test]
fn corpus_answers_match_in_forward_backward_shuffled_and_repeated_order() {
    let scenes = corpus_scenes();
    assert!(
        scenes.len() >= 40,
        "the corpus lost its scenes: {}",
        scenes.len()
    );
    let (mut plans, mut refusals) = (0_usize, 0_usize);
    for (name, scene) in &scenes {
        let frames = corpus_frames(scene.bundle().edit_duration_frames);
        let timeline = PlanTimeline::new(scene);
        let backward: Vec<i64> = frames.iter().rev().copied().collect();
        let mixed = shuffled(&frames, 0x5eed);
        // A frame asked twice in a row and then again after others: nothing may remember it.
        let repeated: Vec<i64> = frames
            .iter()
            .flat_map(|frame| [*frame, *frame])
            .chain(frames.iter().copied())
            .collect();
        for order in [&frames, &backward, &mixed, &repeated] {
            for options in BOTH_MODES {
                for frame in order {
                    if assert_same(name, scene, &timeline, *frame, options) {
                        plans += 1;
                    } else {
                        refusals += 1;
                    }
                }
            }
        }
    }
    // Both outcomes must be present, or the comparison above would prove less than it says.
    assert!(plans > 1_000, "only {plans} plans compared");
    assert!(refusals > 1_000, "only {refusals} refusals compared");
}

#[test]
fn a_fresh_timeline_per_frame_matches_too() {
    // The cold path (first frame of a shot builds the template) is the one a cache bug would hide
    // in, so every frame is also asked of a timeline that has never seen another.
    for (name, scene) in corpus_scenes() {
        for frame in corpus_frames(scene.bundle().edit_duration_frames) {
            for options in BOTH_MODES {
                assert_same(&name, &scene, &PlanTimeline::new(&scene), frame, options);
            }
        }
    }
}

#[test]
fn synthetic_scenes_match_across_shots_and_out_of_order() {
    for shape in SHAPES {
        // The widest scenes are measured by the benchmark; the equivalence runs on the shapes a
        // debug build can compare in full.
        if shape.planes > 200 || shape.camera_segments > 100 {
            continue;
        }
        let scene = synthetic_scene(&shape);
        let timeline = PlanTimeline::new(&scene);
        let frames = spread_frames(shape.edit_frames, 120);
        let mixed = shuffled(&frames, 0xc0ffee);
        for order in [&frames, &mixed] {
            for options in BOTH_MODES {
                for frame in order {
                    assert_same(shape.name, &scene, &timeline, *frame, options);
                }
            }
        }
    }
}

#[test]
fn a_scene_with_thousands_of_camera_movements_matches() {
    // The shape where rebuilding the camera layout dominated the cost: few frames, full comparison.
    let shape = shape_named("camera-heavy").unwrap();
    let scene = synthetic_scene(&shape);
    let timeline = PlanTimeline::new(&scene);
    for frame in spread_frames(shape.edit_frames, 12) {
        assert_same(shape.name, &scene, &timeline, frame, PlanOptions::default());
    }
}

// ---- generations ----------------------------------------------------------------------------

fn edited(shape: &Shape, edit: impl FnOnce(&mut Value)) -> DirectedScene {
    let mut document = synthetic_bundle(shape);
    edit(&mut document);
    DirectedScene::import_value(document).unwrap()
}

#[test]
fn timelines_of_two_generations_of_a_scene_answer_for_their_own_scene() {
    let shape = shape_named("small").unwrap();
    let first = synthetic_scene(&shape);
    // Generation two: a node moved, a plane added and a shot lengthened by one more camera
    // movement: different transforms, different node list, different clip.
    let second = edited(&shape, |document| {
        let nodes = document["sceneDefinitions"][0]["nodes"]
            .as_array_mut()
            .unwrap();
        nodes[3]["localPosition"] = json!({ "x": 9.0, "y": 0.0, "z": 0.0 });
        let mut plane = nodes[3].clone();
        plane["id"] = json!("added-plane");
        nodes.push(plane);
        document["cameraClips"][0]["initialFovDeg"] = json!(35.0);
    });
    let old_timeline = PlanTimeline::new(&first);
    let new_timeline = PlanTimeline::new(&second);
    let mut differing = 0;
    for frame in spread_frames(shape.edit_frames, 40) {
        // Interleaved on purpose: neither timeline may serve the other generation's answer.
        let a = assert_same(
            "first",
            &first,
            &old_timeline,
            frame,
            PlanOptions::default(),
        );
        let b = assert_same(
            "second",
            &second,
            &new_timeline,
            frame,
            PlanOptions::default(),
        );
        assert_eq!(a, b, "both generations are in range or both are not");
        if a && old_timeline.build(frame) != new_timeline.build(frame) {
            differing += 1;
        }
    }
    assert!(
        differing > 0,
        "the edit must change the plans, or the test proves nothing"
    );
    // The older generation still answers, unchanged, after the newer one was used.
    assert_same(
        "first again",
        &first,
        &old_timeline,
        0,
        PlanOptions::default(),
    );
}

// ---- errors ---------------------------------------------------------------------------------

/// Two instances in two shots; the second one has two channels writing the same property, which
/// the importer accepts and the evaluator refuses (`PerformanceWriterConflict`) when a frame of it
/// is evaluated.
fn one_broken_instance() -> DirectedScene {
    let shape = Shape {
        name: "two-instances",
        planes: 3,
        groups: 1,
        channels: 2,
        edit_frames: 20,
        camera_segments: 2,
        camera_layers: 0,
        shots: 1,
    };
    let mut document = synthetic_bundle(&shape);
    let broken_channels = json!([
        { "kind": "static", "channelId": "a", "nodeId": "p0", "property": "localScale",
          "value": { "x": 2.0, "y": 2.0, "z": 2.0 } },
        { "kind": "static", "channelId": "b", "nodeId": "p0", "property": "localScale",
          "value": { "x": 3.0, "y": 3.0, "z": 3.0 } },
    ]);
    document["sceneInstances"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": "broken", "sceneDefinitionId": "scene", "durationFrames": 20,
            "performanceTrackIds": ["broken-track"],
        }));
    document["performanceTracks"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": "broken-track", "instanceId": "broken", "channels": broken_channels,
        }));
    document["editDurationFrames"] = json!(40);
    document["shots"][0]["editRange"] = json!({ "startFrame": 0, "endFrameExclusive": 20 });
    document["shots"].as_array_mut().unwrap().push(json!({
        "id": "s-broken", "sceneInstanceId": "broken",
        "editRange": { "startFrame": 20, "endFrameExclusive": 40 },
        "instanceStartFrame": 0, "cameraClipId": "clip", "cameraInFrame": 0,
        "transitionInto": { "type": "cut", "reason": "new scene" },
        "intent": "a shot whose instance cannot be evaluated",
    }));
    DirectedScene::import_value(document).unwrap_or_else(|error| panic!("{error:?}"))
}

#[test]
fn a_layout_that_fails_fails_the_same_way_for_every_frame_that_needs_it() {
    let scene = one_broken_instance();
    let timeline = PlanTimeline::new(&scene);
    // The healthy shot is planned before, between and after the broken one.
    let order: Vec<i64> = (0..40).chain((0..40).rev()).collect();
    let (mut plans, mut refusals) = (0, 0);
    for frame in order {
        if assert_same("broken", &scene, &timeline, frame, PlanOptions::default()) {
            plans += 1;
        } else {
            refusals += 1;
            assert!(frame >= 20, "the healthy shot must not be refused: {frame}");
        }
    }
    assert_eq!(plans, 40, "20 frames twice");
    assert_eq!(refusals, 40);
    let error = timeline.build(25).unwrap_err();
    assert_eq!(error.code(), "invalid_animation");
}

#[test]
fn the_frame_error_still_comes_before_the_declared_domains() {
    // A scene that both declares weather (a domain the plan refuses) and is asked an impossible
    // frame: the frame's error wins, in both planners, as in `build_with`.
    let shape = shape_named("small").unwrap();
    let scene = edited(&shape, |document| {
        document["sceneDefinitions"][0]["weather"] = json!("rain");
    });
    let timeline = PlanTimeline::new(&scene);
    for frame in [-1, shape.edit_frames as i64, i64::MAX, i64::MIN] {
        for options in BOTH_MODES {
            assert!(!assert_same("range", &scene, &timeline, frame, options));
        }
    }
    // In range: refused when complete, planned when partial, in both planners and in either order.
    for options in [BOTH_MODES[1], BOTH_MODES[0], BOTH_MODES[1]] {
        assert_same("weather", &scene, &timeline, 3, options);
    }
    assert!(matches!(
        timeline.build(3),
        Err(PlanError::Unsupported { .. })
    ));
    assert!(!timeline.build_with(3, BOTH_MODES[1]).unwrap().is_complete());
}

// ---- threads and memory ---------------------------------------------------------------------

#[test]
fn threads_asking_different_frames_get_the_sequential_answers() {
    let shape = shape_named("medium").unwrap();
    let scene = synthetic_scene(&shape);
    let frames = spread_frames(shape.edit_frames, 64);
    let expected: Vec<String> = frames
        .iter()
        .map(|frame| outcome_text(&direct(&scene, *frame, PlanOptions::default())))
        .collect();
    let timeline = PlanTimeline::new(&scene);
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..4)
            .map(|worker| {
                let (timeline, frames, expected) = (&timeline, &frames, &expected);
                scope.spawn(move || {
                    // Each worker walks the same frames in its own order, so shots are prepared
                    // by whichever thread gets there first.
                    for (frame, want) in
                        shuffled(&frames.iter().zip(expected).collect::<Vec<_>>(), worker + 1)
                    {
                        assert_eq!(&outcome_text(&timeline.build(*frame)), want);
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
    });
}

#[test]
fn what_a_timeline_keeps_is_bounded_by_the_bundle_not_by_the_frames_asked() {
    let shape = shape_named("small").unwrap();
    let scene = synthetic_scene(&shape);
    let timeline = PlanTimeline::new(&scene);
    assert_eq!(
        timeline.prepared_layouts(),
        0,
        "nothing is built before a frame asks"
    );
    for frame in 0..shape.edit_frames as i64 {
        timeline.build(frame).unwrap();
    }
    let after_one_pass = timeline.prepared_layouts();
    for frame in (0..shape.edit_frames as i64).rev() {
        timeline.build(frame).unwrap();
    }
    assert_eq!(
        timeline.prepared_layouts(),
        after_one_pass,
        "a second pass retains nothing more"
    );
    let bundle = scene.bundle();
    assert_eq!(
        after_one_pass,
        bundle.instances.len() + bundle.camera_clips.len() + bundle.shots.len(),
        "one layout per instance, clip and shot, and no more"
    );
}
