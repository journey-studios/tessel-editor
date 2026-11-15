//! T16 (continued): the layers in edit frames that S1 does not move. An op that would have to move
//! one is refused with `SHOTS_LAYER_NOT_YET` (the layer and the JSON pointer of the first item in
//! `details`); it is never accepted with the layer left where it was. Every number is written by
//! hand from the reference's mapping (`map_clip`, `map_point`, `reframe_shot_clip`).

use core::prelude::v1::test;

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::scene_edit::shots::{LOCKED, SHOTS_LAYER_NOT_YET};

use super::support::*;

/// `a` `[0,30)`, `b` `[30,60)` with 20 head frames, `c` `[60,90)`, with `extra` (`fx`,
/// `sceneDefinitions`...) in the oracle's shape.
fn with(extra: Value) -> DirectedScene {
    scene_of(layered(extra))
}

fn layered(extra: Value) -> Value {
    bundle(
        90,
        &[
            sh("a", "ia", 0, 30, 0, 0),
            sh("b", "ib", 30, 60, 20, 20),
            sh("c", "ic", 60, 90, 0, 0),
        ],
        &[("ia", 60, false), ("ib", 60, false), ("ic", 60, false)],
        extra,
    )
}

fn output_clips(clips: &[(i64, i64)]) -> Value {
    let clips: Vec<Value> = clips
        .iter()
        .enumerate()
        .map(|(index, (start, frames))| {
            json!({"id": format!("o{index}"), "startFrame": start, "durationFrames": frames})
        })
        .collect();
    json!({"fx": {"tracks": [{"id": "o", "clips": clips}], "transitions": []}})
}

#[track_caller]
fn layer_refusal(scene: &DirectedScene, value: Value, layer: &str, path: &str) {
    let error = refused(scene, value.clone());
    assert_eq!(error.code, SHOTS_LAYER_NOT_YET, "{value}: {error}");
    assert_eq!(error.index, Some(0), "{value}");
    assert_eq!(detail(&error, "layer"), json!(layer), "{value}");
    assert_eq!(detail(&error, "path"), json!(path), "{value}");
}

#[track_caller]
fn untouched(scene: &DirectedScene, value: Value, field: &str) {
    let out = done(scene, value.clone());
    assert!(out.changed, "{value}");
    assert_eq!(
        serde_json::to_string(&out.candidate.document()[field]).unwrap(),
        serde_json::to_string(&scene.document()[field]).unwrap(),
        "{field} after {value}"
    );
}

// ---- effect clips of the output --------------------------------------------------------------

#[test]
fn an_output_clip_that_starts_at_the_first_seam_or_after_it_is_refused_one_before_it_is_not() {
    // `b` longer by 10 opens a seam at 60: a clip at [60,70) moves by 10, one at [50,60) ends at
    // the seam (`end > at` is false) and stays.
    let grow = json!({"set_shot_duration": {"shot": "b", "frames": 40}});
    layer_refusal(
        &with(output_clips(&[(60, 10)])),
        grow.clone(),
        "fx_clip_output",
        "/fx/tracks/0/clips/0",
    );
    layer_refusal(
        &with(output_clips(&[(50, 8), (75, 10)])),
        grow.clone(),
        "fx_clip_output",
        "/fx/tracks/0/clips/1",
    );
    untouched(&with(output_clips(&[(50, 10)])), grow, "fx");
}

#[test]
fn a_clip_inside_a_removed_stretch_or_cut_by_it_is_refused_with_keep_absolute_as_the_way_out() {
    // `b` shorter by 10 removes [50,60): the clip [52,58) would be removed, [40,55) cut.
    let shrink = json!({"set_shot_duration": {"shot": "b", "frames": 20}});
    layer_refusal(
        &with(output_clips(&[(52, 6)])),
        shrink.clone(),
        "fx_clip_output",
        "/fx/tracks/0/clips/0",
    );
    layer_refusal(
        &with(output_clips(&[(40, 15)])),
        shrink,
        "fx_clip_output",
        "/fx/tracks/0/clips/0",
    );
    let frozen = json!({"set_shot_duration": {"shot": "b", "frames": 20, "keep_absolute": true}});
    untouched(&with(output_clips(&[(40, 3), (52, 6)])), frozen, "fx");
}

#[test]
fn the_first_item_is_the_one_reported_across_tracks() {
    let extra = json!({"fx": {"tracks": [
        {"id": "p", "clips": [{"id": "p0", "startFrame": 0, "durationFrames": 10}]},
        {"id": "q", "clips": [
            {"id": "q0", "startFrame": 5, "durationFrames": 10},
            {"id": "q1", "startFrame": 70, "durationFrames": 10},
        ]},
    ], "transitions": []}});
    layer_refusal(
        &with(extra),
        json!({"set_shot_duration": {"shot": "b", "frames": 40}}),
        "fx_clip_output",
        "/fx/tracks/1/clips/1",
    );
}

// ---- effect clips of a shot ------------------------------------------------------------------

fn shot_clip(start: i64, frames: i64) -> Value {
    json!({"fx": {"tracks": [{"id": "s", "clips": [{
        "id": "s0", "startFrame": start, "durationFrames": frames,
        "scope": {"kind": "shot", "shotId": "b"},
    }]}], "transitions": []}})
}

#[test]
fn a_clip_of_a_shot_follows_the_plan_so_keep_absolute_does_not_help() {
    let scene = with(shot_clip(35, 10));
    // `a` longer by 10 slides `b` to [40,70): the clip has to go with it.
    for keep in [false, true] {
        layer_refusal(
            &scene,
            json!({"set_shot_duration": {"shot": "a", "frames": 40, "keep_absolute": keep}}),
            "fx_clip_shot",
            "/fx/tracks/0/clips/0",
        );
    }
    // The tail of `b` is cut to 38: the clip [35,45) loses its end.
    layer_refusal(
        &scene,
        json!({"set_shot_duration": {"shot": "b", "frames": 8}}),
        "fx_clip_shot",
        "/fx/tracks/0/clips/0",
    );
    // A roll that cuts the head of `b` past the clip's start.
    layer_refusal(
        &scene,
        json!({"move_cut": {"from": "a", "to": "b", "frame": 40}}),
        "fx_clip_shot",
        "/fx/tracks/0/clips/0",
    );
}

#[test]
fn a_clip_of_a_shot_that_neither_slides_nor_loses_frames_stays_where_it_is() {
    let scene = with(shot_clip(35, 10));
    // `b` keeps its start, its picture and the frames the clip uses: only `c` moves.
    untouched(
        &scene,
        json!({"set_shot_duration": {"shot": "b", "frames": 20}}),
        "fx",
    );
    untouched(
        &scene,
        json!({"set_shot_duration": {"shot": "c", "frames": 40}}),
        "fx",
    );
}

// ---- light keys and weatherKeys --------------------------------------------------------------

fn light_keys() -> Value {
    json!({"sceneDefinitions": [{"id": "def-ia", "lighting": {"ambient": 1, "lights": [
        {"id": "lamp", "keys": {
            "intensity": [
                {"easing": "linear", "frame": 5, "value": 1.0},
                {"easing": "linear", "frame": 45, "value": 2.0},
                {"easing": "linear", "frame": 70, "value": 3.0},
            ],
        }},
        {"id": "torch"},
    ]}}]})
}

#[test]
fn a_light_key_the_seam_moves_is_refused_and_keep_absolute_freezes_it() {
    let scene = with(light_keys());
    // The seam is at 60: the key at 70 moves, the ones at 5 and 45 do not.
    layer_refusal(
        &scene,
        json!({"set_shot_duration": {"shot": "b", "frames": 40}}),
        "light_keys",
        "/sceneDefinitions/0/lighting/lights/0/keys/intensity/2",
    );
    untouched(
        &scene,
        json!({"set_shot_duration": {"shot": "b", "frames": 40, "keep_absolute": true}}),
        "sceneDefinitions",
    );
    // A seam after every key: nothing to move.
    untouched(
        &scene,
        json!({"set_shot_duration": {"shot": "c", "frames": 40}}),
        "sceneDefinitions",
    );
}

#[test]
fn a_key_in_a_removed_stretch_would_be_dropped_and_so_it_is_refused() {
    let scene = with(light_keys());
    // `b` shorter by 10 removes [50,60): no key falls in it, the one at 70 moves back.
    layer_refusal(
        &scene,
        json!({"set_shot_duration": {"shot": "b", "frames": 20}}),
        "light_keys",
        "/sceneDefinitions/0/lighting/lights/0/keys/intensity/2",
    );
    // `b` shorter by 20 removes [40,60): the key at 45 is dropped, and is the first one to move.
    layer_refusal(
        &scene,
        json!({"set_shot_duration": {"shot": "b", "frames": 10}}),
        "light_keys",
        "/sceneDefinitions/0/lighting/lights/0/keys/intensity/1",
    );
}

fn weather_keys() -> Value {
    json!({"sceneDefinitions": [{"id": "def-ia", "weather": "snow:0.5", "weatherKeys": [
        {"easing": "linear", "frame": 10, "value": 0.1},
        {"easing": "linear", "frame": 60, "value": 0.9},
    ]}]})
}

#[test]
fn a_weather_key_at_the_seam_or_after_it_is_refused_and_keep_absolute_freezes_it() {
    let scene = with(weather_keys());
    // The seam is at 60 and a key sits exactly on it: it moves (`frame >= at`).
    layer_refusal(
        &scene,
        json!({"set_shot_duration": {"shot": "b", "frames": 40}}),
        "weather_keys",
        "/sceneDefinitions/0/weatherKeys/1",
    );
    untouched(
        &scene,
        json!({"set_shot_duration": {"shot": "b", "frames": 40, "keep_absolute": true}}),
        "sceneDefinitions",
    );
}

// ---- transitions -----------------------------------------------------------------------------

/// A centred 10 frame transition from `b` to `c` that draws 5 frames from each side.
fn centred() -> DirectedScene {
    let mut document = layered(json!({}));
    document["fx"] = json!({"catalogVersion": 1, "tracks": [], "transitions": [{
        "id": "tr1", "effect": "crossDissolve", "fromShotId": "b", "toShotId": "c",
        "durationFrames": 10, "alignment": "center", "handles": "hold", "easing": "easeInOutCubic",
    }]});
    scene_admitting(document)
}

#[test]
fn a_transition_that_stops_fitting_is_the_gates_finding_unless_the_op_asked_to_shrink_it() {
    let scene = centred();
    // `b` of 3 frames cannot hold the 5 frames the transition draws before the cut.
    let plain = refused(
        &scene,
        json!({"set_shot_duration": {"shot": "b", "frames": 3}}),
    );
    finding(&plain, "TRANSITION_TOO_LONG");
    // `shrink_transitions` asked for what S1 cannot do: it is a typed refusal, not the gate's.
    for keep in [false, true] {
        layer_refusal(
            &scene,
            json!({"set_shot_duration": {
                "shot": "b", "frames": 3, "shrink_transitions": true, "keep_absolute": keep,
            }}),
            "transition",
            "/fx/transitions/0",
        );
    }
}

#[test]
fn a_transition_that_still_fits_stays_as_the_reference_leaves_it() {
    let scene = centred();
    for shrink in [false, true] {
        untouched(
            &scene,
            json!({"set_shot_duration": {"shot": "b", "frames": 40, "shrink_transitions": shrink}}),
            "fx",
        );
        untouched(
            &scene,
            json!({"move_cut": {"from": "b", "to": "c", "frame": 50, "shrink_transitions": shrink}}),
            "fx",
        );
    }
}

// ---- order -----------------------------------------------------------------------------------

#[test]
fn the_lock_is_judged_before_the_layers() {
    // `a` and `c` are on the locked `il`, in step; a longer `b` takes `c` out of step, and the
    // output clip at 70 would have to move too: the rule of the plan comes first.
    let document = bundle(
        90,
        &[
            sh("a", "il", 0, 30, 0, 0),
            sh("b", "ib", 30, 60, 0, 0),
            sh("c", "il", 60, 90, 60, 20),
        ],
        &[("il", 120, true), ("ib", 60, false)],
        output_clips(&[(70, 5)]),
    );
    let error = refused(
        &scene_of(document),
        json!({"set_shot_duration": {"shot": "b", "frames": 40}}),
    );
    assert_eq!(error.code, LOCKED, "{error}");
}
