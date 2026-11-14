//! M2 (T07, T11, T12, T13): one edit frame in stages. `Fx::resolve_frame` keeps the primary deck,
//! the transition and the output apart. Expected values are written by hand from plan 5.3 / 6.2
//! (a reference calculation, not the runtime nor the GPU).

use core::prelude::v1::test;

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::directed_scene::fx::{DeckSide, FxFrame};

use crate::support::*;

const EDIT: u64 = 144;

fn frame_of(scene: &DirectedScene, frame: i64) -> FxFrame<'_> {
    let bundle = scene.bundle();
    bundle.fx.resolve_frame(&bundle.shots, EDIT, frame)
}

fn near(found: f64, expected: f64, what: &str) {
    assert!(
        (found - expected).abs() <= 1e-15,
        "{what}: {found:.17} vs {expected:.17}"
    );
}

#[test]
fn the_primary_is_the_shot_under_the_needle_on_both_sides_of_the_cut() {
    let scene = spec_scene(vec![tr(json!({}))]);
    // Window [42, 54), cut at 48.
    for frame in 42..48 {
        let stages = frame_of(&scene, frame);
        let window = stages.transition.unwrap_or_else(|| panic!("{frame}"));
        assert_eq!(window.primary, DeckSide::From, "{frame}");
        assert_eq!(window.primary_shot_id, "s1");
        assert_eq!(window.other_shot_id, "s2");
    }
    for frame in 48..54 {
        let stages = frame_of(&scene, frame);
        let window = stages.transition.unwrap_or_else(|| panic!("{frame}"));
        assert_eq!(window.primary, DeckSide::To, "{frame}");
        assert_eq!(window.primary_shot_id, "s2");
        assert_eq!(window.other_shot_id, "s1");
    }
    for frame in [0, 41, 54, 143] {
        assert!(frame_of(&scene, frame).transition.is_none(), "{frame}");
    }
}

#[test]
fn progress_is_half_frame_centred_and_eased_by_the_documented_curve() {
    let scene = spec_scene(vec![tr(json!({}))]);
    // easeInOutCubic, D = 12, k = frame - 42: t = (k + 0.5) / 12; 4t^3 below 1/2.
    let cubic = |k: i64| frame_of(&scene, 42 + k).transition.unwrap().progress;
    near(cubic(0), 4.0 * (0.5_f64 / 12.0).powi(3), "k0");
    near(cubic(3), 0.099_247_685_185_185_22, "k3 (plan 6.2)");
    near(cubic(5), 0.385_127_314_814_814_77, "k5 (plan 6.2)");
    for k in 0..12 {
        // Symmetric about the middle, strictly inside (0, 1): the first and last frames are NOT
        // a pure deck.
        near(cubic(k) + cubic(11 - k), 1.0, "symmetry");
        assert!(0.0 < cubic(k) && cubic(k) < 1.0, "k{k}");
    }
    assert!(cubic(0) > 0.0 && cubic(11) < 1.0);
    // linear and easeOutCubic, by the formula.
    for (easing, f) in [
        ("linear", (|t: f64| t) as fn(f64) -> f64),
        ("easeOutCubic", |t| 1.0 - (1.0 - t).powi(3)),
    ] {
        let scene = spec_scene(vec![tr(json!({"easing": easing}))]);
        for k in 0..12 {
            let found = frame_of(&scene, 42 + k).transition.unwrap().progress;
            near(found, f((k as f64 + 0.5) / 12.0), easing);
        }
    }
}

#[test]
fn one_frame_odd_and_aligned_windows_have_the_right_frames_and_progress() {
    // D = 1: one frame at the cut, t = 1/2.
    let one = spec_scene(vec![tr(json!({"durationFrames": 1}))]);
    assert!(frame_of(&one, 47).transition.is_none());
    let only = frame_of(&one, 48).transition.unwrap();
    near(only.progress, 0.5, "D1 cubic");
    assert_eq!((only.start_frame, only.frame_in_window), (48, 0));
    assert!(frame_of(&one, 49).transition.is_none());
    let out = spec_scene(vec![tr(
        json!({"durationFrames": 1, "easing": "easeOutCubic"}),
    )]);
    near(
        frame_of(&out, 48).transition.unwrap().progress,
        0.875,
        "D1 out",
    );

    // Odd D = 13, centre: floor(13 / 2) = 6 before the cut, 7 from it: [42, 55).
    let odd = spec_scene(vec![tr(json!({"durationFrames": 13}))]);
    assert!(frame_of(&odd, 41).transition.is_none());
    assert_eq!(frame_of(&odd, 42).transition.unwrap().frame_in_window, 0);
    assert_eq!(frame_of(&odd, 54).transition.unwrap().frame_in_window, 12);
    assert!(frame_of(&odd, 55).transition.is_none());
    // The middle frame of an odd window is exactly half.
    near(
        frame_of(&odd, 48).transition.unwrap().progress,
        0.5,
        "odd middle",
    );

    // start: [48, 60), every frame is the incoming shot's. end: [36, 48), every frame outgoing.
    let start = spec_scene(vec![tr(json!({"alignment": "start"}))]);
    assert!(frame_of(&start, 47).transition.is_none());
    for frame in 48..60 {
        let window = frame_of(&start, frame).transition.unwrap();
        assert_eq!(window.primary, DeckSide::To, "{frame}");
        assert_eq!(window.start_frame, 48);
    }
    assert!(frame_of(&start, 60).transition.is_none());
    let end = spec_scene(vec![tr(json!({"alignment": "end"}))]);
    assert!(frame_of(&end, 35).transition.is_none());
    for frame in 36..48 {
        let window = frame_of(&end, frame).transition.unwrap();
        assert_eq!(window.primary, DeckSide::From, "{frame}");
    }
    assert!(frame_of(&end, 48).transition.is_none());
}

#[test]
fn amount_is_the_clamped_intensity_and_not_a_weight_of_the_decks() {
    for (intensity, expected) in [(0.0, 0.0), (0.25, 0.25), (0.5, 0.5), (1.0, 1.0)] {
        let scene = spec_scene(vec![tr(
            json!({"params": {"curve": "smooth", "intensity": intensity}}),
        )]);
        let window = frame_of(&scene, 45).transition.unwrap();
        near(window.amount, expected, "amount");
        // progress does not depend on it (intensity 0 is not an identity of the effect).
        near(window.progress, 0.099_247_685_185_185_22, "progress");
    }
    // The default is a full mix.
    let plain = spec_scene(vec![tr(json!({}))]);
    near(
        frame_of(&plain, 45).transition.unwrap().amount,
        1.0,
        "default",
    );
}

fn layer(tracks: Vec<Value>) -> Value {
    Value::Array(tracks)
}

#[test]
fn the_neighbour_deck_holds_its_border_and_the_output_acts_once() {
    let scene = spec_scene_with_tracks(
        vec![tr(json!({}))],
        layer(vec![
            shot_clip("t1", "c1", "vignette", "s1", 0, 48),
            shot_clip("t2", "c2", "vignette", "s2", 48, 48),
            output_clip("t3", "fade-out", "fade", 0, 144),
        ]),
    );
    let bundle = scene.bundle();
    // Before the cut (frame 44): primary s1, neighbour s2 in its head.
    let head = frame_of(&scene, 44);
    assert_eq!(
        head.primary_passes
            .iter()
            .map(|p| p.clip_id.as_str())
            .collect::<Vec<_>>(),
        ["c1"]
    );
    assert_eq!(head.primary_passes[0].frame_in_clip, 44);
    let neighbour = &head.transition.as_ref().unwrap().other_passes;
    assert_eq!(neighbour.len(), 1);
    assert_eq!(neighbour[0].clip_id, "c2");
    // Held at the border: the clip's first frame, with the envelope the plain frame 48 has.
    assert_eq!(neighbour[0].frame_in_clip, 0);
    let at_border = bundle.fx.resolve(48);
    let border = at_border.iter().find(|pass| pass.clip_id == "c2").unwrap();
    assert_eq!(neighbour[0].weight, border.weight);
    assert_eq!(neighbour[0].params, border.params);
    // The output pass is the frame's own, once, after the blend.
    assert_eq!(
        head.output_passes
            .iter()
            .map(|p| p.clip_id.as_str())
            .collect::<Vec<_>>(),
        ["fade-out"]
    );
    assert_eq!(head.output_passes[0].frame_in_clip, 44);
    assert!(neighbour.iter().all(|pass| pass.clip_id != "fade-out"));
    // After the cut (frame 50): primary s2, neighbour s1 in its tail, held at its last frame 47.
    let tail = frame_of(&scene, 50);
    let neighbour = &tail.transition.as_ref().unwrap().other_passes;
    assert_eq!(neighbour.len(), 1);
    assert_eq!(neighbour[0].clip_id, "c1");
    assert_eq!(neighbour[0].frame_in_clip, 47);
    let at_border = bundle.fx.resolve(47);
    let border = at_border.iter().find(|pass| pass.clip_id == "c1").unwrap();
    assert_eq!(neighbour[0].weight, border.weight);
    // The flat list is what the frame without a transition would give, and the count is each
    // pass once: primary 1, other 1, blend 1, output 1.
    assert_eq!(tail.flat_passes(), bundle.fx.resolve(50));
    assert_eq!(tail.pass_count(), 4);
    assert_eq!(head.pass_count(), 4);
}

#[test]
fn a_clip_in_the_middle_of_the_neighbour_shot_does_not_enter_the_head_or_the_tail() {
    let scene = spec_scene_with_tracks(
        vec![tr(json!({}))],
        layer(vec![
            shot_clip("t1", "mid-a", "vignette", "s1", 20, 10),
            shot_clip("t2", "mid-b", "vignette", "s2", 60, 10),
            shot_clip("t3", "edge-b", "vignette", "s2", 48, 12),
            shot_clip("t4", "edge-a", "vignette", "s1", 36, 12),
        ]),
    );
    let head = frame_of(&scene, 44).transition.unwrap();
    let ids: Vec<_> = head
        .other_passes
        .iter()
        .map(|p| p.clip_id.as_str())
        .collect();
    assert_eq!(
        ids,
        ["edge-b"],
        "only the clip that touches s2's first frame"
    );
    let tail = frame_of(&scene, 50).transition.unwrap();
    let ids: Vec<_> = tail
        .other_passes
        .iter()
        .map(|p| p.clip_id.as_str())
        .collect();
    assert_eq!(
        ids,
        ["edge-a"],
        "only the clip that touches s1's last frame"
    );
}

#[test]
fn a_hidden_transition_a_hidden_clip_and_a_hidden_track_draw_nothing_extra() {
    let hidden = spec_scene(vec![tr(json!({"hidden": true}))]);
    for frame in [42, 44, 48, 50] {
        assert!(frame_of(&hidden, frame).transition.is_none(), "{frame}");
    }
    let mut track = shot_clip("t2", "c2", "vignette", "s2", 48, 48);
    track["clips"][0]["hidden"] = json!(true);
    let scene = spec_scene_with_tracks(vec![tr(json!({}))], layer(vec![track]));
    assert!(
        frame_of(&scene, 44)
            .transition
            .unwrap()
            .other_passes
            .is_empty()
    );
}

#[test]
fn the_noise_seed_of_the_neighbour_follows_the_real_frame_while_its_clip_holds() {
    let scene = spec_scene_with_tracks(
        vec![tr(json!({}))],
        layer(vec![shot_clip("t2", "grain", "filmGrain", "s2", 48, 48)]),
    );
    let seed_at = |frame| {
        frame_of(&scene, frame).transition.unwrap().other_passes[0]
            .seed
            .expect("a grain pass has a seed")
    };
    let held = scene.bundle().fx.resolve(48);
    let held_seed = held
        .iter()
        .find(|pass| pass.clip_id == "grain")
        .unwrap()
        .seed
        .unwrap();
    // The clip holds frame 0 across the head...
    for frame in 42..48 {
        let pass = &frame_of(&scene, frame).transition.unwrap().other_passes[0];
        assert_eq!(pass.frame_in_clip, 0, "{frame}");
    }
    // ...but its seed keeps moving, so the grain does not freeze with the deck.
    let seeds: Vec<u32> = (42..48).map(seed_at).collect();
    for pair in seeds.windows(2) {
        assert_ne!(pair[0], pair[1], "{seeds:?}");
    }
    assert!(
        seeds.iter().all(|seed| *seed != held_seed),
        "{seeds:?} vs {held_seed}"
    );
}

#[test]
fn a_frame_without_a_window_is_exactly_the_flat_list_and_a_window_flat_list_is_the_same() {
    let scene = spec_scene_with_tracks(
        vec![tr(json!({}))],
        layer(vec![
            shot_clip("t1", "c1", "vignette", "s1", 0, 48),
            shot_clip("t2", "c2", "vignette", "s2", 48, 48),
            output_clip("t3", "fade-out", "fade", 0, 144),
        ]),
    );
    let bundle = scene.bundle();
    for frame in 0..144 {
        let stages = frame_of(&scene, frame);
        assert_eq!(
            stages.flat_passes(),
            bundle.fx.resolve(frame),
            "frame {frame}"
        );
        assert_eq!(
            stages.transition.is_some(),
            (42..54).contains(&frame),
            "{frame}"
        );
    }
}

/// T03: the sweep over edges and the sum over frames agree, and the sweep does not depend on
/// the length of the edit.
#[test]
fn the_budget_counts_each_deck_the_blend_and_the_output_once_and_stays_a_sweep() {
    let tracks = layer(vec![
        shot_clip("t1", "c1", "vignette", "s1", 0, 48),
        shot_clip("t2", "c2", "vignette", "s2", 48, 48),
        output_clip("t3", "fade-out", "fade", 0, 144),
    ]);
    let scene = spec_scene_with_tracks(vec![tr(json!({}))], tracks.clone());
    let bundle = scene.bundle();
    let by_frames = (0..144)
        .map(|frame| frame_of(&scene, frame).pass_count())
        .max()
        .unwrap();
    assert_eq!(by_frames, 4, "primary 1 + neighbour 1 + blend 1 + output 1");
    let budget = bundle.fx.pass_budget_in(&bundle.shots, EDIT);
    assert_eq!(budget.peak, by_frames);
    // Without the transition the same layer draws two passes at most.
    let plain = spec_scene_with_tracks(vec![], tracks);
    let plain_bundle = plain.bundle();
    assert_eq!(
        plain_bundle
            .fx
            .pass_budget_in(&plain_bundle.shots, EDIT)
            .peak,
        2
    );
    assert_eq!(plain_bundle.fx.pass_budget(EDIT).peak, 2);
    // A hidden transition adds nothing.
    let hidden = spec_scene_with_tracks(
        vec![tr(json!({"hidden": true}))],
        layer(vec![shot_clip("t1", "c1", "vignette", "s1", 0, 48)]),
    );
    let hidden_bundle = hidden.bundle();
    assert_eq!(
        hidden_bundle
            .fx
            .pass_budget_in(&hidden_bundle.shots, EDIT)
            .peak,
        1
    );
    // A huge edit length is a sweep over a handful of edges: it returns at once with a peak.
    let huge = bundle.fx.pass_budget_in(&bundle.shots, 4_000_000_000);
    assert_eq!(huge.peak, by_frames);
}

/// The budget is a warning, never a refusal: more than the budget still plans.
#[test]
fn passes_over_the_budget_still_build_a_plan() {
    use tessel_editor_core::directed_scene::fx::FX_PASS_BUDGET;
    let mut tracks = Vec::new();
    for index in 0..=FX_PASS_BUDGET {
        tracks.push(output_clip(
            &format!("t{index}"),
            &format!("c{index}"),
            "vignette",
            0,
            144,
        ));
    }
    let scene = spec_scene_with_tracks(vec![tr(json!({}))], layer(tracks));
    let bundle = scene.bundle();
    let budget = bundle.fx.pass_budget_in(&bundle.shots, EDIT);
    assert!(budget.peak > FX_PASS_BUDGET);
    assert!(budget.first_frame_over.is_some());
    let plan = plan_at(&scene, 45);
    assert_eq!(plan.fx.len(), FX_PASS_BUDGET + 1);
    assert!(plan.transition.is_some());
}
