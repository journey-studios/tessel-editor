//! F061 (G4.1, BL082.35): the 2D handles of the vignette, through the public library API and the
//! JSON executor (in process: no window, no GPU, no Chrome, no GPUI, no pointer device), plus one
//! test with the real `tessel-headless` process.
//!
//! Expectations are computed **here**, never with the code under test: the position of each
//! handle is judged by `vignette_shade`, the reference of the pixel math (shade 0 on the ellipse
//! of the radius, 1 on the one of radius + softness); a drag is judged against `scene.fx.edit`
//! with the same value, bit for bit; the value a pointer gives is a closed formula written below.
//!
//! - **Geometry.** 16:9, 1:1 and 9:16, roundness 0, 0.6 and 1.
//! - **Drag.** The same document as `scene.fx.edit`; one "Edit effects" entry per gesture, none
//!   for a no-op or a drag that came back; cancel leaves the document as it was.
//! - **Keys.** A parameter with keys has its handle off (`animated_param`): the press is refused
//!   by name, nothing is written, and the other handles keep working.
//! - **Off picture / out of frame.** Not drawn, not pressable.
//! - **Not here**: the target, its staleness and `scene.gizmo.hit` are in `scene_fx_target.rs`;
//!   what a runtime draws is in `runtime/test/`; the window and a physical pointer are not covered.

use std::io::Write;
use std::process::{Command as Process, Stdio};

use core::prelude::v1::test;
use serde_json::{Value, json};
use tessel_editor_core::EditorState;
use tessel_editor_core::directed_scene::fx::{self, FxPass};

#[path = "scene_fx_handles_support/mod.rs"]
mod support;
use support::*;

fn pass_of(state: &EditorState, frame: i64) -> FxPass {
    let scene = &state.scenes.get("street").unwrap().scene;
    scene.bundle().fx.resolve(frame).remove(0)
}

fn smoothstep_at(state: &EditorState, at: (f64, f64), size: (u64, u64)) -> f64 {
    fx::vignette_shade(
        &pass_of(state, FRAME),
        at.0,
        at.1,
        size.0 as f64,
        size.1 as f64,
    )
    .unwrap()
}

/// The distance of the picture position `at` from `center`, written from the pixel math of the
/// reference (not from the code under test).
fn distance(at: (f64, f64), center: (f64, f64), size: (u64, u64), roundness: f64) -> f64 {
    let stretch = 1.0 + (size.0 as f64 / size.1 as f64 - 1.0) * roundness;
    let (dx, dy) = (2.0 * (at.0 - center.0) * stretch, 2.0 * (at.1 - center.1));
    dx.hypot(dy) / stretch.hypot(1.0).max(1.0)
}

fn to_grid(value: f64, step: f64, min: f64) -> f64 {
    ((value - min) / step).round() * step + min
}

#[test]
fn the_handles_sit_on_the_ellipses_the_reference_shade_draws() {
    for size in [(320_u64, 180_u64), (256, 256), (180, 320)] {
        for roundness in [0.0, 0.6, 1.0] {
            let mut state = session_with(size.0, size.1, roundness);
            let answer = handles(&mut state);
            assert_eq!(answer["visible"], true, "{answer}");
            let radius = handle_at(&answer, "radius");
            let softness = handle_at(&answer, "softness");
            let centre = handle_at(&answer, "center");
            assert_eq!(centre, (0.2, 0.5), "the centre handle is the centre");
            let start = smoothstep_at(&state, radius, size);
            let end = smoothstep_at(&state, softness, size);
            assert!(
                start.abs() < 1e-9,
                "{size:?} ρ={roundness}: shade at the radius handle is {start}"
            );
            assert!(
                (1.0 - end).abs() < 1e-9,
                "{size:?} ρ={roundness}: shade at the softness handle is {end}"
            );
            assert!(
                softness.0 > radius.0,
                "the falloff ends beyond where it starts"
            );
            assert_eq!(
                radius.1, 0.5,
                "both edge handles are on the horizontal axis through the centre"
            );
        }
    }
}

#[test]
fn the_ellipses_are_the_level_lines_of_the_distance() {
    let mut state = session_with(320, 180, 1.0);
    let answer = handles(&mut state);
    let ellipses = answer["ellipses"].as_array().unwrap();
    assert_eq!(ellipses.len(), 2);
    for (ellipse, level) in ellipses.iter().zip([0.4, 0.6]) {
        let (rx, ry) = (
            ellipse["rx"].as_f64().unwrap(),
            ellipse["ry"].as_f64().unwrap(),
        );
        let centre = (0.2, 0.5);
        for angle in [0.0_f64, 0.7, 1.9, 3.3, 5.0] {
            let at = (centre.0 + rx * angle.cos(), centre.1 + ry * angle.sin());
            let found = distance(at, centre, (320, 180), 1.0);
            assert!(
                (found - level).abs() < 1e-9,
                "{ellipse}: the point at {angle} rad is at distance {found}, not {level}"
            );
        }
    }
}

#[test]
fn a_drag_leaves_the_document_scene_fx_edit_leaves_for_the_same_value() {
    let size = (320, 180);
    let cases = [
        ("radius", (0.62, 0.5)),
        ("softness", (0.9, 0.5)),
        ("center", (0.33, 0.71)),
    ];
    for (param, target) in cases {
        let mut dragged = session();
        selected(&mut dragged);
        let start = handle_at(&handles(&mut dragged), param);
        let pressed = down(&mut dragged, start);
        assert_eq!(pressed["dragging"], true, "{pressed}");
        assert_eq!(pressed["hit"]["handle"], param);
        mv(&mut dragged, (target.0 * 0.5 + start.0 * 0.5, target.1));
        let moved = mv(&mut dragged, target);
        assert_eq!(moved["applied"], true, "{moved}");
        assert_eq!(moved["document_changed"], false, "a move writes nothing");
        let released = up(&mut dragged, Some(target));
        assert_eq!(released["document_changed"], true, "{released}");
        let value = released["value"].clone();

        let expected = match param {
            "radius" => json!(to_grid(distance(target, (0.2, 0.5), size, 0.6), 0.01, 0.0)),
            "softness" => json!(to_grid(
                distance(target, (0.2, 0.5), size, 0.6) - 0.4,
                0.01,
                0.05
            )),
            _ => json!({"x": to_grid(target.0, 0.01, 0.0), "y": to_grid(target.1, 0.01, 0.0)}),
        };
        let close = |a: &Value, b: &Value| match (a.as_f64(), b.as_f64()) {
            (Some(a), Some(b)) => (a - b).abs() < 1e-9,
            _ => {
                a["x"]
                    .as_f64()
                    .zip(b["x"].as_f64())
                    .is_some_and(|(a, b)| (a - b).abs() < 1e-9)
                    && a["y"]
                        .as_f64()
                        .zip(b["y"].as_f64())
                        .is_some_and(|(a, b)| (a - b).abs() < 1e-9)
            }
        };
        assert!(
            close(&value, &expected),
            "{param}: wrote {value}, expected {expected}"
        );

        let mut edited = session();
        ok(
            &mut edited,
            json!({"op":"scene.fx.edit","scene":"street","ops":[
                {"op":"set_param","clip":"v1","param":param,"value":value}]}),
        );
        assert_eq!(
            serde_json::to_string(&document(&dragged)).unwrap(),
            serde_json::to_string(&document(&edited)).unwrap(),
            "{param}: the drag and scene.fx.edit must leave the same document"
        );
    }
}

#[test]
fn one_gesture_is_one_undo_entry_and_nothing_else_is() {
    let mut state = session();
    selected(&mut state);
    let before = document(&state);
    let depth = undo_depth(&state);
    let start = handle_at(&handles(&mut state), "radius");

    down(&mut state, start);
    for step in 1..=6 {
        mv(&mut state, (start.0 + 0.02 * f64::from(step), 0.5));
    }
    assert_eq!(document(&state), before, "no move reaches the document");
    assert_eq!(undo_depth(&state), depth, "no move is an entry");
    let released = up(&mut state, None);
    assert_eq!(released["changed"], true, "{released}");
    assert_eq!(undo_depth(&state), depth + 1, "one gesture, one entry");
    assert_eq!(state.history.to_json()["undo_label"], "Edit effects");
    let after = document(&state);
    assert_ne!(after, before);

    ok(&mut state, json!({"op":"history.undo"}));
    assert_eq!(document(&state), before, "undo restores the value");
    ok(&mut state, json!({"op":"history.redo"}));
    assert_eq!(document(&state), after, "redo brings it back");

    // A press and release on the same spot, and a drag that returns to where it began, are no-ops.
    let depth = undo_depth(&state);
    let start = handle_at(&handles(&mut state), "softness");
    down(&mut state, start);
    let released = up(&mut state, None);
    assert_eq!(released["changed"], false, "{released}");
    down(&mut state, start);
    mv(&mut state, (start.0 + 0.1, start.1));
    mv(&mut state, start);
    let released = up(&mut state, Some(start));
    assert_eq!(released["changed"], false, "{released}");
    assert_eq!(undo_depth(&state), depth, "a no-op leaves no entry");
}

/// The value of a handle drag is absolute (the position of the pointer becomes the value), so the
/// pointer has to MOVE from the press for a position to count: a click, however it lands, is not
/// an edit. `center.x` is authored off the control's grid (`0.1234`, the grid is `0.01`), so a
/// value taken from the press point itself would be `0.12`: an edit nobody dragged.
fn session_with_an_off_grid_center() -> EditorState {
    let mut state = session();
    ok(
        &mut state,
        json!({"op":"scene.fx.edit","scene":"street","ops":[
            {"op":"set_param","clip":"v1","param":"center","value":{"x":0.1234,"y":0.5}}]}),
    );
    selected(&mut state);
    state
}

#[test]
fn a_press_and_release_on_the_same_point_writes_nothing() {
    let mut state = session_with_an_off_grid_center();
    let (before, depth) = (document(&state), undo_depth(&state));
    let start = handle_at(&handles(&mut state), "center");
    assert_eq!(
        start,
        (0.1234, 0.5),
        "the handle sits on the authored value"
    );

    // The release alone carries the point: "move there, then commit" must not move anywhere.
    let pressed = down(&mut state, start);
    assert_eq!(pressed["dragging"], true, "{pressed}");
    let released = up(&mut state, Some(start));
    assert_eq!(released["changed"], false, "{released}");
    assert_eq!(released["document_changed"], false, "{released}");
    assert_eq!(released["noop"], true, "{released}");
    assert_eq!(
        (document(&state), undo_depth(&state)),
        (before.clone(), depth)
    );

    // A move to the same point says so and does not count as the first position of the drag...
    down(&mut state, start);
    let still = mv(&mut state, start);
    assert_eq!(still["applied"], false, "{still}");
    assert_eq!(still["reason"], "not_moved", "{still}");
    // ...so a release without a point still has no candidate to commit...
    let released = up(&mut state, None);
    assert_eq!(released["noop"], true, "{released}");
    assert_eq!(
        (document(&state), undo_depth(&state)),
        (before.clone(), depth)
    );

    // ...and the drag is untouched: the next position away from the press is applied as usual.
    down(&mut state, start);
    mv(&mut state, start);
    let away = mv(&mut state, (0.6, 0.5));
    assert_eq!(away["applied"], true, "{away}");
    assert_eq!(away["value"], json!({"x": 0.6, "y": 0.5}), "{away}");
    ok(&mut state, json!({"op":"scene.pointer.cancel"}));
    assert_eq!((document(&state), undo_depth(&state)), (before, depth));
}

#[test]
fn a_press_a_little_off_the_handle_and_released_there_writes_nothing() {
    let mut state = session();
    selected(&mut state);
    let (before, depth) = (document(&state), undo_depth(&state));
    let handle = handle_at(&handles(&mut state), "radius");
    // 0.012 of the width is 0.021 of the height at 16:9: inside the hit radius (0.025), and far
    // enough that the value under the pointer (`radius` ~0.42) is not the one the clip holds (0.4).
    let press = (handle.0 + 0.012, handle.1);
    let pressed = down(&mut state, press);
    assert_eq!(pressed["hit"]["handle"], "radius", "{pressed}");
    assert_eq!(pressed["dragging"], true, "{pressed}");
    let released = up(&mut state, Some(press));
    assert_eq!(released["document_changed"], false, "{released}");
    assert_eq!((document(&state), undo_depth(&state)), (before, depth));
}

#[test]
fn a_drag_away_writes_the_value_of_the_point_it_ends_on_even_if_it_came_back() {
    // Away: the value of the last point, as the move said.
    let mut state = session_with_an_off_grid_center();
    let depth = undo_depth(&state);
    let start = handle_at(&handles(&mut state), "center");
    down(&mut state, start);
    let moved = mv(&mut state, (0.5, 0.25));
    assert_eq!(moved["applied"], true, "{moved}");
    let released = up(&mut state, Some((0.5, 0.25)));
    assert_eq!(released["document_changed"], true, "{released}");
    assert_eq!(released["value"], moved["value"]);
    assert_eq!(released["value"], json!({"x": 0.5, "y": 0.25}));
    assert_eq!(undo_depth(&state), depth + 1);

    // Back: once the pointer has left the press point, a position at the press point is a
    // position like any other (the user dragged), so the value of THAT point is written: `0.12`,
    // the grid value of the authored `0.1234`.
    let mut state = session_with_an_off_grid_center();
    let depth = undo_depth(&state);
    down(&mut state, start);
    mv(&mut state, (0.5, 0.25));
    let back = mv(&mut state, start);
    assert_eq!(back["applied"], true, "{back}");
    assert_eq!(back["value"], json!({"x": 0.12, "y": 0.5}), "{back}");
    let released = up(&mut state, Some(start));
    assert_eq!(released["document_changed"], true, "{released}");
    assert_eq!(undo_depth(&state), depth + 1, "one gesture, one entry");
    let center = &document(&state)["fx"];
    assert!(
        center.to_string().contains("\"x\":0.12"),
        "the clip holds the grid value: {center}"
    );
}

#[test]
fn a_cancelled_drag_leaves_the_document_and_history_as_they_were() {
    let mut state = session();
    selected(&mut state);
    let (before, depth) = (document(&state), undo_depth(&state));
    let start = handle_at(&handles(&mut state), "center");
    down(&mut state, start);
    mv(&mut state, (0.5, 0.5));
    let cancelled = ok(
        &mut state,
        json!({"op":"scene.pointer.cancel","reason":"escape"}),
    );
    assert_eq!(cancelled["dragging"], false);
    assert_eq!((document(&state), undo_depth(&state)), (before, depth));
    let error = err(
        &mut state,
        json!({"op":"scene.pointer.move","point":{"u":0.4,"v":0.4}}),
    );
    assert_eq!(error.code, "no_drag", "a cancelled drag never revives");
    // The slot is free again: a new press begins a new gesture.
    let again = down(&mut state, start);
    assert_eq!(again["dragging"], true);
}

#[test]
fn the_drag_is_clamped_to_the_ranges_of_the_catalog() {
    let mut state = session();
    selected(&mut state);
    let start = handle_at(&handles(&mut state), "radius");
    down(&mut state, start);
    let far = mv(&mut state, (7.0, 0.5));
    assert_eq!(
        far["value"].as_f64(),
        Some(1.5),
        "radius tops out at 1.5: {far}"
    );
    up(&mut state, None);

    let start = handle_at(&handles(&mut state), "center");
    down(&mut state, start);
    let corner = mv(&mut state, (-3.0, 7.0));
    assert_eq!(corner["value"], json!({"x": 0.0, "y": 1.0}), "{corner}");
    up(&mut state, None);

    let mut state = session();
    selected(&mut state);
    let start = handle_at(&handles(&mut state), "softness");
    down(&mut state, start);
    let inside = mv(&mut state, (0.2, 0.5));
    assert_eq!(
        inside["value"].as_f64(),
        Some(0.05),
        "softness bottoms out at 0.05: {inside}"
    );
}

#[test]
fn a_parameter_with_keys_has_its_handle_off_and_a_press_on_it_is_refused() {
    let mut state = session();
    ok(
        &mut state,
        json!({"op":"scene.fx.edit","scene":"street","ops":[
            {"op":"key_param","clip":"v1","param":"radius","frame":0,"value":0.4},
            {"op":"key_param","clip":"v1","param":"radius","frame":20,"value":0.6}]}),
    );
    selected(&mut state);
    let answer = handles(&mut state);
    assert_eq!(
        disabled(&answer, "radius").as_deref(),
        Some("animated_param")
    );
    assert!(
        answer["handles"]
            .as_array()
            .unwrap()
            .iter()
            .all(|h| h["id"] != "radius")
    );
    assert_eq!(disabled(&answer, "softness"), None, "the others stay on");

    // Where the keyed radius puts its (drawn, greyed) handle: the press is refused by name.
    let (before, depth) = (document(&state), undo_depth(&state));
    let at = ok(
        &mut state,
        json!({"op":"scene.fx.handles","scene":"street","clip":"v1","frame":FRAME}),
    );
    assert!(at["ellipses"][0]["rx"].as_f64().unwrap() > 0.0);
    let ellipse = &at["ellipses"][0];
    let spot = (0.2 + ellipse["rx"].as_f64().unwrap(), 0.5);
    let refused = down(&mut state, spot);
    assert_eq!(refused["refused"], "animated_param", "{refused}");
    assert_eq!(refused["dragging"], false);
    assert_eq!(refused["gesture"], Value::Null, "no gesture opened");
    assert_eq!((document(&state), undo_depth(&state)), (before, depth));
    let status = ok(&mut state, json!({"op":"scene.fx.gesture.status"}));
    assert_eq!(status["gesture"], Value::Null, "{status}");

    // The softness handle still drags, and the keys are untouched by it: the document is the one
    // `scene.fx.edit` leaves for the same value (a key written, or any other road, would differ).
    let keys_of = |state: &mut EditorState| {
        let list = ok(state, json!({"op":"scene.fx.list","scene":"street"}));
        list["tracks"][0]["clips"][0]["keys"].clone()
    };
    let keys = keys_of(&mut state);
    assert!(!keys.is_null(), "the clip lists its keys: {keys}");
    let start = handle_at(&answer, "softness");
    down(&mut state, start);
    mv(&mut state, (start.0 + 0.05, start.1));
    let released = up(&mut state, None);
    assert_eq!(released["changed"], true, "{released}");
    assert_eq!(keys_of(&mut state), keys, "the keys are as they were");
    let mut edited = session();
    ok(
        &mut edited,
        json!({"op":"scene.fx.edit","scene":"street","ops":[
            {"op":"key_param","clip":"v1","param":"radius","frame":0,"value":0.4},
            {"op":"key_param","clip":"v1","param":"radius","frame":20,"value":0.6},
            {"op":"set_param","clip":"v1","param":"softness","value":released["value"]}]}),
    );
    assert_eq!(
        document(&state),
        document(&edited),
        "same road as scene.fx.edit"
    );
}

#[test]
fn a_handle_off_the_picture_is_not_drawn_and_does_not_capture_the_pointer() {
    let mut state = session();
    ok(
        &mut state,
        json!({"op":"scene.fx.edit","scene":"street","ops":[
            {"op":"set_param","clip":"v1","param":"radius","value":1.5},
            {"op":"set_param","clip":"v1","param":"center","value":{"x":0.5,"y":0.5}}]}),
    );
    selected(&mut state);
    let answer = handles(&mut state);
    assert_eq!(
        disabled(&answer, "radius").as_deref(),
        Some("handle_off_picture"),
        "{answer}"
    );
    assert_eq!(
        disabled(&answer, "softness").as_deref(),
        Some("handle_off_picture")
    );
    let ids: Vec<&str> = answer["handles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|handle| handle["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["center"], "only the centre is on the picture");
    let (before, depth) = (document(&state), undo_depth(&state));
    // Near the picture's edge, where the radius handle would be pulled to: nothing to hold.
    let pressed = down(&mut state, (0.99, 0.5));
    assert_eq!(pressed["dragging"], false, "{pressed}");
    assert_eq!(pressed["hit"]["kind"], "none");
    assert_eq!((document(&state), undo_depth(&state)), (before, depth));
}

#[test]
fn a_clip_that_does_not_cover_the_frame_has_no_handles() {
    let mut state = session();
    selected(&mut state);
    let early = ok(
        &mut state,
        json!({"op":"scene.fx.handles","scene":"street","clip":"v1","frame":5}),
    );
    assert_eq!(early["visible"], false);
    assert_eq!(early["reason"], "clip_not_in_frame");
    assert_eq!(early["handles"], json!([]));
    assert_eq!(early["ellipses"], json!([]));
    // Press where the handle is at a frame the clip covers: at frame 5 it is not there.
    let spot = handle_at(&handles(&mut state), "radius");
    let pressed = ok(
        &mut state,
        json!({"op":"scene.pointer.down","scene":"street","frame":5,
               "point":{"u":spot.0,"v":spot.1}}),
    );
    assert_eq!(pressed["dragging"], false, "{pressed}");
    let late = ok(
        &mut state,
        json!({"op":"scene.fx.handles","scene":"street","clip":"v1","frame":40}),
    );
    assert_eq!(
        late["reason"], "clip_not_in_frame",
        "the end of the clip is exclusive"
    );
}

#[test]
fn only_the_vignette_has_handles() {
    let mut state = session();
    ok(
        &mut state,
        json!({"op":"scene.fx.edit","scene":"street","ops":[
            {"op":"add_clip","effect":"fade","start_frame":0,"duration_frames":10,"id":"f1"}]}),
    );
    let error = err(
        &mut state,
        json!({"op":"scene.fx.select","scene":"street","clip":"f1"}),
    );
    assert_eq!(error.code, "fx_has_no_handles");
    let unknown = err(
        &mut state,
        json!({"op":"scene.fx.select","scene":"street","clip":"nope"}),
    );
    assert_eq!(unknown.code, "unknown_element");
    assert!(
        state.scene_interaction.fx_target().is_none(),
        "a refusal selects nothing"
    );
}

/// F062 x F061 (BL082.64): a vignette laid on a shot has no handles, and every door that asks the
/// core says it with the same typed refusal (`scope_not_supported`, the clip as target, the scope in
/// `details`): the select, the read with and without a probe, and the hit query and the press, which
/// answer by the absence of an effect handle because no target can exist for such a clip. A target
/// that turns into a shot clip is dropped by the edit that does it, before any read.
#[test]
fn a_vignette_on_a_shot_has_no_handles_whichever_door_asks() {
    let shot_scope = json!({"op":"scene.fx.edit","scene":"street","ops":[
        {"op":"set_scope","clip":"v1","scope":"shot:shot-1"}]});
    let output_scope = json!({"op":"scene.fx.edit","scene":"street","ops":[
        {"op":"set_scope","clip":"v1","scope":"output"}]});
    let mut state = session();
    selected(&mut state);
    assert_eq!(published_target(&state), (json!("fx"), json!("v1")));

    // The edit that lays the target clip on a shot drops it: no read in between.
    let depth = undo_depth(&state);
    ok(&mut state, shot_scope.clone());
    assert_eq!(undo_depth(&state), depth + 1, "the scope edit is one entry");
    assert_eq!(published_target(&state), (json!("node"), json!("")));
    assert!(state.scene_interaction.fx_target().is_none());

    // The doors: the typed refusal, the same everywhere, and nothing written.
    let (before, depth) = (document(&state), undo_depth(&state));
    let refused = [
        json!({"op":"scene.fx.select","scene":"street","clip":"v1"}),
        json!({"op":"scene.fx.handles","scene":"street","clip":"v1","frame":FRAME}),
        json!({"op":"scene.fx.handles","scene":"street","clip":"v1","frame":FRAME,
               "probe":{"handle":"radius","point":{"u":0.7,"v":0.5}}}),
    ];
    for request in refused {
        let error = err(&mut state, request.clone());
        assert_eq!(error.code, "scope_not_supported", "{request}");
        assert_eq!(error.target.as_deref(), Some("v1"), "{request}");
        assert_eq!(
            error.details.as_deref(),
            Some(&json!({"clip": "v1", "scope": "shot:shot-1"})),
            "{request}"
        );
    }
    assert!(state.scene_interaction.fx_target().is_none());
    let at = (0.5, 0.5);
    let asked = ok(
        &mut state,
        json!({"op":"scene.gizmo.hit","scene":"street","frame":FRAME,
               "point":{"u":at.0,"v":at.1}}),
    );
    assert_ne!(asked["hit"]["kind"], "fx_handle", "{asked}");
    assert_ne!(down(&mut state, at)["hit"]["kind"], "fx_handle");
    assert_eq!((document(&state), undo_depth(&state)), (before, depth));

    // Back on the output the same doors answer, and a drag open when the scope changes is dead.
    ok(&mut state, output_scope);
    selected(&mut state);
    let start = handle_at(&handles(&mut state), "radius");
    assert_eq!(down(&mut state, start)["dragging"], true);
    ok(&mut state, shot_scope);
    assert!(state.gizmo_drag_is_dead(), "the edit kills the open drag");
    assert!(state.scene_interaction.fx_target().is_none());
}

/// The target is dropped by `redo` too, not only by the edit: `restore` prunes as `apply` does, so a clip
/// that history lays back on a shot stops being the target at that step, before any read. (The undo of a
/// scope edit lays the clip back on the output, where selecting it is allowed again.)
#[test]
fn a_redo_that_lays_the_target_clip_on_a_shot_drops_the_target() {
    let mut state = session();
    ok(
        &mut state,
        json!({"op":"scene.fx.edit","scene":"street","ops":[
            {"op":"set_scope","clip":"v1","scope":"shot:shot-1"}]}),
    );
    ok(&mut state, json!({"op":"history.undo"}));
    selected(&mut state);
    assert_eq!(published_target(&state), (json!("fx"), json!("v1")));

    ok(&mut state, json!({"op":"history.redo"}));
    assert_eq!(published_target(&state), (json!("node"), json!("")));
    assert!(state.scene_interaction.fx_target().is_none());
    // And the clip is a shot clip for every door again.
    let error = err(
        &mut state,
        json!({"op":"scene.fx.select","scene":"street","clip":"v1"}),
    );
    assert_eq!(error.code, "scope_not_supported");
}

/// The same refusal through the real `tessel-headless` process (the surface a script uses without a
/// window): the select and the read answer `scope_not_supported`, the document is untouched.
#[test]
fn the_headless_executable_refuses_the_handles_of_a_shot_clip() {
    let mut process = Headless::start();
    process.ask(json!({"op":"scene.attach","id":"street","bundle":bundle(320, 180)}));
    let added = process.ask(json!({"op":"scene.fx.edit","scene":"street","ops":[
        {"op":"add_clip","effect":"vignette","start_frame":10,"duration_frames":30,"id":"v1",
         "scope":"shot:shot-1"}]}));
    assert_eq!(added["ok"], true, "{added}");
    let depth = |answer: &Value| answer["result"]["history"]["undo_depth"].as_u64().unwrap();
    let base = depth(&process.ask(json!({"op":"history.status"})));
    for request in [
        json!({"op":"scene.fx.select","scene":"street","clip":"v1"}),
        json!({"op":"scene.fx.handles","scene":"street","clip":"v1","frame":FRAME}),
    ] {
        let answer = process.ask(request.clone());
        assert_eq!(answer["ok"], false, "{request}: {answer}");
        assert_eq!(answer["error"]["code"], "scope_not_supported", "{answer}");
        assert_eq!(answer["error"]["target"], "v1", "{answer}");
        assert_eq!(
            answer["error"]["details"]["scope"], "shot:shot-1",
            "{answer}"
        );
    }
    assert_eq!(depth(&process.ask(json!({"op":"history.status"}))), base);
    drop(process.stdin);
    // The executable ends with 1 when a request of the session was refused.
    assert_eq!(process.child.wait().unwrap().code(), Some(1));
}

/// The page picks «the nearest, a tie to the first of the list», so the order of the lists IS the
/// rule: the outer handles first, in the read, in the overlay, and among the disabled ones.
#[test]
fn the_handles_are_listed_in_priority_order_the_outer_ones_first() {
    let ids = |answer: &Value, key: &str| -> Vec<String> {
        answer[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["id"].as_str().unwrap().to_string())
            .collect()
    };
    let mut state = session();
    selected(&mut state);
    let read = handles(&mut state);
    assert_eq!(ids(&read, "handles"), ["softness", "radius", "center"]);
    let overlay = ok(
        &mut state,
        json!({"op":"scene.gizmo.state","scene":"street","frame":FRAME}),
    );
    assert_eq!(
        ids(&overlay["overlay"]["fx"], "handles"),
        ["softness", "radius", "center"]
    );
    ok(
        &mut state,
        json!({"op":"scene.fx.edit","scene":"street","ops":[
            {"op":"key_param","clip":"v1","param":"radius","frame":0,"value":0.4},
            {"op":"key_param","clip":"v1","param":"radius","frame":20,"value":0.6},
            {"op":"key_param","clip":"v1","param":"softness","frame":0,"value":0.2},
            {"op":"key_param","clip":"v1","param":"softness","frame":20,"value":0.3}]}),
    );
    let keyed = handles(&mut state);
    assert_eq!(ids(&keyed, "handles"), ["center"]);
    assert_eq!(ids(&keyed, "disabled_handles"), ["softness", "radius"]);
}

#[test]
fn the_read_and_its_probe_write_nothing_and_judge_by_a_dry_run() {
    let mut state = session();
    selected(&mut state);
    let (before, depth, revision) = (document(&state), undo_depth(&state), state.project.revision);
    let answer = ok(
        &mut state,
        json!({"op":"scene.fx.handles","scene":"street","frame":FRAME,
               "probe":{"handle":"radius","point":{"u":0.7,"v":0.5}}}),
    );
    let probe = &answer["probe"];
    assert_eq!(probe["ok"], true, "{probe}");
    assert_eq!(probe["would_change"], true);
    let expected = to_grid(distance((0.7, 0.5), (0.2, 0.5), (320, 180), 0.6), 0.01, 0.0);
    assert!((probe["value"].as_f64().unwrap() - expected).abs() < 1e-9);
    assert_eq!(answer["selected"], true);
    assert_eq!(
        (document(&state), undo_depth(&state), state.project.revision),
        (before, depth, revision)
    );

    // A probe on the handle where it already is says it would change nothing.
    let at = handle_at(&answer, "center");
    let same = ok(
        &mut state,
        json!({"op":"scene.fx.handles","scene":"street","frame":FRAME,
               "probe":{"handle":"center","point":{"u":at.0,"v":at.1}}}),
    );
    assert_eq!(same["probe"]["would_change"], false, "{}", same["probe"]);

    // Without a selected clip the read needs one; with the field it does not.
    ok(&mut state, json!({"op":"scene.fx.select","scene":"street"}));
    let missing = err(
        &mut state,
        json!({"op":"scene.fx.handles","scene":"street","frame":FRAME}),
    );
    assert_eq!(missing.code, "invalid_request");
    let named = ok(
        &mut state,
        json!({"op":"scene.fx.handles","scene":"street","clip":"v1","frame":FRAME}),
    );
    assert_eq!(named["selected"], false);
    let bad = err(
        &mut state,
        json!({"op":"scene.fx.handles","scene":"street","clip":"v1","frame":FRAME,
               "probe":{"handle":"roundness","point":{"u":0.5,"v":0.5}}}),
    );
    assert_eq!(bad.code, "invalid_request");
}

#[test]
fn the_overlay_and_the_stamp_carry_the_clip_and_nothing_else_changes_without_it() {
    let mut state = session();
    let plain = state.gizmo_stamp("street", FRAME);
    assert!(
        plain.get("fx_clip").is_none(),
        "no target, no new field: {plain}"
    );
    let key = state.gizmo_overlay_key("street", FRAME);
    selected(&mut state);
    let stamp = state.gizmo_stamp("street", FRAME);
    assert_eq!(stamp["fx_clip"], "v1");
    assert_ne!(
        key,
        state.gizmo_overlay_key("street", FRAME),
        "the memo key follows the target"
    );
    let overlay = ok(
        &mut state,
        json!({"op":"scene.gizmo.state","scene":"street","frame":FRAME}),
    );
    assert_eq!(overlay["overlay"]["stamp"], stamp);
    assert_eq!(
        overlay["overlay"]["fx"]["metrics"]["unit"],
        "picture_height"
    );
    assert_eq!(overlay["overlay"]["fx"]["clip"], "v1");
    let summary = state.summary();
    assert_eq!(summary["scene_interaction"]["fx_target"]["clip"], "v1");

    // While dragging, the overlay lays the handles on the candidate and names the active one.
    let start = handle_at(&handles(&mut state), "radius");
    down(&mut state, start);
    let moved = mv(&mut state, (start.0 + 0.1, 0.5));
    assert_eq!(moved["overlay"]["fx"]["active"], "radius");
    let drawn = handle_at(&moved["overlay"]["fx"], "radius");
    assert!(drawn.0 > start.0, "the handle follows the candidate");
    let document_handle = handle_at(&handles(&mut state), "radius");
    assert_eq!(document_handle, start, "the document did not move");
}

#[test]
fn capabilities_list_the_new_operations() {
    let mut state = session();
    let answer = ok(&mut state, json!({"op":"scene.capabilities"}));
    let commands = answer["commands"].as_array().unwrap();
    for name in ["scene.fx.select", "scene.fx.handles"] {
        assert!(commands.contains(&json!(name)), "{name}");
    }
}

/// A real `tessel-headless` process, one request at a time.
struct Headless {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    stdout: std::io::BufReader<std::process::ChildStdout>,
}

impl Headless {
    fn start() -> Self {
        let mut child = Process::new(env!("CARGO_BIN_EXE_tessel-headless"))
            .arg("--empty")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = std::io::BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
        }
    }

    fn ask(&mut self, request: Value) -> Value {
        use std::io::BufRead;
        writeln!(self.stdin, "{request}").unwrap();
        self.stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }
}

#[test]
fn the_headless_executable_drags_a_handle_and_commits_one_entry() {
    let mut process = Headless::start();
    process.ask(json!({"op":"scene.attach","id":"street","bundle":bundle(320, 180)}));
    process.ask(json!({"op":"scene.fx.edit","scene":"street","ops":[
        {"op":"add_clip","effect":"vignette","start_frame":10,"duration_frames":30,"id":"v1",
         "params":{"radius":0.4,"softness":0.2,"roundness":0.6,"center":{"x":0.2,"y":0.5}}}]}));
    let depth = |answer: &Value| answer["result"]["history"]["undo_depth"].as_u64().unwrap();
    let base = depth(&process.ask(json!({"op":"history.status"})));

    let chosen = process.ask(json!({"op":"scene.fx.select","scene":"street","clip":"v1"}));
    assert_eq!(chosen["result"]["target"]["clip"], "v1", "{chosen}");
    let read =
        process.ask(json!({"op":"scene.fx.handles","scene":"street","clip":"v1","frame":FRAME}));
    let start = handle_at(&read["result"], "radius");
    let pressed = process.ask(
        json!({"op":"scene.pointer.down","scene":"street","frame":FRAME,
        "point":{"u":start.0,"v":start.1}}),
    );
    assert_eq!(pressed["result"]["dragging"], true, "{pressed}");
    let moved = process.ask(json!({"op":"scene.pointer.move","point":{"u":0.75,"v":0.5}}));
    assert_eq!(moved["result"]["applied"], true, "{moved}");
    assert_eq!(
        depth(&process.ask(json!({"op":"history.status"}))),
        base,
        "a move is not an entry"
    );
    let released = process.ask(json!({"op":"scene.pointer.up"}));
    assert_eq!(released["result"]["document_changed"], true, "{released}");
    assert_eq!(
        depth(&process.ask(json!({"op":"history.status"}))),
        base + 1
    );
    let listed = process.ask(json!({"op":"scene.fx.list","scene":"street"}));
    let radius = listed["result"]["tracks"][0]["clips"][0]["params"]["radius"]
        .as_f64()
        .unwrap();
    let expected = to_grid(
        distance((0.75, 0.5), (0.2, 0.5), (320, 180), 0.6),
        0.01,
        0.0,
    );
    assert!((radius - expected).abs() < 1e-9, "{radius} vs {expected}");

    drop(process.stdin);
    let status = process.child.wait().unwrap();
    assert_eq!(status.code(), Some(0), "a clean session exits 0");
}
