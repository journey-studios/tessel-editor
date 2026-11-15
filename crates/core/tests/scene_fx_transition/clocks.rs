//! M1 (T06, T10): the neighbour deck's clocks. `evaluate_shot_at_offset` asks a NAMED shot for a
//! frame outside its range. The numbers are written by hand from the origin's clamp formulas (plan
//! 6.1, D14, D17), never read back from the code under test; the oracle's own rows are consumed in
//! `pilot_dissolve_corpus.rs`.

use core::prelude::v1::test;

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::{DirectedScene, TemporalError};

use crate::support::*;

fn clocks(scene: &DirectedScene, shot: &str, frame: i64) -> (u64, u64, bool, bool) {
    let found = scene
        .bundle()
        .evaluate_shot_at_offset(shot, frame)
        .unwrap_or_else(|error| panic!("{shot} at {frame}: {error:?}"));
    (
        found.evaluation.mapping.instance_frame,
        found.evaluation.mapping.camera_clip_frame,
        found.held_instance,
        found.held_camera,
    )
}

fn spec() -> DirectedScene {
    spec_scene(vec![tr(json!({}))])
}

fn with_fps(mut bundle: Value, numerator: u32, denominator: u32) -> Value {
    let fps = json!({"numerator": numerator, "denominator": denominator});
    bundle["fps"] = fps.clone();
    bundle["renderProfile"]["fps"] = fps;
    bundle
}

#[test]
fn a_frame_inside_the_shot_is_the_strict_frame_and_holds_nothing() {
    let scene = spec();
    // s2 [48,96), instance 0, camera 0: delta 2 is instance 2 and camera 2 (origin spec vector).
    assert_eq!(clocks(&scene, "s2", 50), (2, 2, false, false));
    // Its first and last frames.
    assert_eq!(clocks(&scene, "s2", 48), (0, 0, false, false));
    assert_eq!(clocks(&scene, "s2", 95), (47, 47, false, false));
    // The strict evaluator agrees on every frame of the shot.
    let bundle = scene.bundle();
    for frame in 48..96 {
        let strict = bundle.evaluate_frame(frame).unwrap().mapping;
        let offset = bundle.evaluate_shot_at_offset("s2", frame).unwrap();
        assert_eq!(
            (strict.instance_frame, strict.camera_clip_frame),
            (
                offset.evaluation.mapping.instance_frame,
                offset.evaluation.mapping.camera_clip_frame
            ),
            "frame {frame}"
        );
        assert!(!offset.held_instance && !offset.held_camera, "{frame}");
    }
}

#[test]
fn the_head_of_the_incoming_shot_holds_both_clocks_at_zero_and_says_so() {
    let scene = spec();
    // delta -4 and -48: before the first frame of both clips.
    assert_eq!(clocks(&scene, "s2", 44), (0, 0, true, true));
    assert_eq!(clocks(&scene, "s2", 0), (0, 0, true, true));
    // The last held frame of the head, then the first real one.
    assert_eq!(clocks(&scene, "s2", 47), (0, 0, true, true));
    assert_eq!(clocks(&scene, "s2", 48), (0, 0, false, false));
}

#[test]
fn the_tail_of_the_outgoing_shot_holds_each_clock_on_its_own_length() {
    let scene = spec();
    // s1 [0,48): instance 20 + 50 = 70 of 200 (running), camera 50 of 48 frames (held at 47).
    assert_eq!(clocks(&scene, "s1", 50), (70, 47, false, true));
    // At 48 the camera has run out by one frame and the instance has not: separate flags.
    assert_eq!(clocks(&scene, "s1", 48), (68, 47, false, true));
    // The last real frame holds nothing.
    assert_eq!(clocks(&scene, "s1", 47), (67, 47, false, false));
}

#[test]
fn a_held_flag_is_true_only_when_the_clamp_changed_the_frame() {
    // shot-a [0,40): instance 60 frames from 10, camera 40 frames from 5.
    let scene = admit(corpus()).unwrap();
    // delta 35: the instance runs (45), the camera would be 40 and is held at 39.
    assert_eq!(clocks(&scene, "shot-a", 35), (45, 39, false, true));
    // delta 34: the camera is exactly its last frame: reaching it is not holding it.
    assert_eq!(clocks(&scene, "shot-a", 34), (44, 39, false, false));
}

#[test]
fn the_two_clocks_are_clamped_apart_not_together() {
    // shot-b [40,100): instance 60 frames from 0, camera 30 frames from 0. At delta 35 the camera
    // has run out and the instance has not: a joint clamp would pull the instance back to 29.
    let scene = admit(corpus()).unwrap();
    assert_eq!(clocks(&scene, "shot-b", 75), (35, 29, false, true));
    // And in the head of shot-b the instance holds zero while shot-a's camera still runs.
    assert_eq!(clocks(&scene, "shot-b", 38), (0, 0, true, true));
    assert_eq!(clocks(&scene, "shot-a", 38), (48, 39, false, true));
}

#[test]
fn the_scene_clock_is_the_real_edit_frame_even_while_both_clocks_hold() {
    let scene = spec();
    let held = scene.bundle().evaluate_shot_at_offset("s2", 44).unwrap();
    let mapping = held.evaluation.mapping;
    assert_eq!((mapping.instance_frame, mapping.camera_clip_frame), (0, 0));
    // Frame 44 at 24 fps is 44/24 = 11/6 s, not the held 0.
    assert_eq!(
        (
            mapping.edit_frame,
            mapping.edit_time.num(),
            mapping.edit_time.den()
        ),
        (44, 11, 6)
    );
    assert_eq!(mapping.instance_time.num(), 0);
    assert_eq!(mapping.camera_clip_time.num(), 0);
}

#[test]
fn the_clocks_hold_for_every_fps_and_the_edit_clock_is_rational() {
    // (fps, edit time of frame 44, edit time of frame 42) as reduced fractions: 44 * den / num.
    let rows = [
        ((24, 1), (11, 6), (7, 4)),
        ((25, 1), (44, 25), (42, 25)),
        ((60, 1), (11, 15), (7, 10)),
        ((30000, 1001), (11011, 7500), (7007, 5000)),
    ];
    for ((numerator, denominator), at_44, at_42) in rows {
        let scene = scene_of(with_transitions(
            with_fps(Edit::spec().bundle(), numerator, denominator),
            vec![tr(json!({}))],
        ));
        let at = |frame| {
            let mapping = scene
                .bundle()
                .evaluate_shot_at_offset("s2", frame)
                .unwrap()
                .evaluation
                .mapping;
            (mapping.edit_time.num(), mapping.edit_time.den())
        };
        assert_eq!(at(44), at_44, "{numerator}/{denominator}");
        assert_eq!(at(42), at_42, "{numerator}/{denominator}");
        // The held clocks do not depend on the fps.
        assert_eq!(clocks(&scene, "s2", 44), (0, 0, true, true));
    }
}

#[test]
fn an_unknown_shot_or_a_frame_outside_the_edit_is_refused_never_clamped() {
    let scene = spec();
    let bundle = scene.bundle();
    assert!(matches!(
        bundle.evaluate_shot_at_offset("nope", 10),
        Err(TemporalError::Unresolved { .. })
    ));
    for frame in [-1, 144, i64::MAX, i64::MIN] {
        assert!(
            matches!(
                bundle.evaluate_shot_at_offset("s2", frame),
                Err(TemporalError::EditFrameOutOfRange { .. })
            ),
            "{frame}"
        );
    }
}

#[test]
fn the_reach_of_a_shot_never_goes_negative() {
    let scene = admit(corpus()).unwrap();
    let bundle = scene.bundle();
    let reach = |id: &str| bundle.shot_reach(bundle.shot(id).unwrap()).unwrap();
    // shot-a: head 10 (instanceStart) / 5 (cameraIn); tail 60 - 10 - 40 = 10, camera 40 - 5 - 40 < 0.
    let a = reach("shot-a");
    assert_eq!((a.head_instance, a.tail_instance), (10, 10));
    assert_eq!((a.head_camera, a.tail_camera), (5, 0));
    // shot-b: instance 60 from 0 for 60 frames, camera 30 from 0.
    let b = reach("shot-b");
    assert_eq!((b.head_instance, b.tail_instance), (0, 0));
    assert_eq!((b.head_camera, b.tail_camera), (0, 0));
}

/// The deck is a pure function of the frame: any order, any repetition, and one asked from a
/// subinterval gives what the whole sweep gave (no state between frames).
#[test]
fn evaluation_is_pure_and_order_free() {
    let scene = spec();
    let bundle = scene.bundle();
    let frames: Vec<i64> = (0..144).collect();
    let forward: Vec<_> = frames
        .iter()
        .map(|&frame| bundle.evaluate_shot_at_offset("s2", frame).unwrap())
        .collect();
    // Reverse, then a fixed pseudo-random order (an LCG: no dependency, deterministic).
    let mut order: Vec<usize> = (0..frames.len()).collect();
    order.reverse();
    let mut seed = 0x2545_f491_4f6c_dd1d_u64;
    let mut shuffled = order.clone();
    for at in (1..shuffled.len()).rev() {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        shuffled.swap(at, (seed >> 33) as usize % (at + 1));
    }
    for sequence in [order, shuffled] {
        for index in sequence {
            let again = bundle.evaluate_shot_at_offset("s2", frames[index]).unwrap();
            assert_eq!(again, forward[index], "frame {}", frames[index]);
        }
    }
    // A repeated frame is the same value.
    let twice = bundle.evaluate_shot_at_offset("s2", 44).unwrap();
    assert_eq!(twice, bundle.evaluate_shot_at_offset("s2", 44).unwrap());
}
