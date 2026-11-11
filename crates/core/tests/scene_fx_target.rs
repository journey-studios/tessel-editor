//! F061 (G4.1, BL082.35): the target of the vignette handles and the questions about it, through
//! the public library API and the JSON executor (in process: no window, no GPU, no GPUI).
//!
//! - **One target.** Selecting an entity leaves the clip, selecting a clip empties the selection,
//!   and either swap cancels a live drag (no phantom drag survives).
//! - **Staleness.** An undo, a scene change or a detach under a live drag kills it; the target the
//!   project no longer holds is gone where the project changes, before any read.
//! - **Query = press.** `scene.gizmo.hit` names the handle `scene.pointer.down` would hold.

use core::prelude::v1::test;
use serde_json::{Value, json};
use tessel_editor_core::EditorState;

#[path = "scene_fx_handles_support/mod.rs"]
mod support;
use support::*;

#[test]
fn one_target_at_a_time_and_a_swap_leaves_no_phantom_drag() {
    let mut state = session();
    // An entity is selected: choosing a clip empties the selection and says so.
    ok(
        &mut state,
        json!({"op":"scene.select","scene":"street","entities":[FIGURE],"mode":"replace"}),
    );
    let chosen = ok(
        &mut state,
        json!({"op":"scene.fx.select","scene":"street","clip":"v1"}),
    );
    assert_eq!(chosen["selection_cleared"], true);
    let selection = ok(&mut state, json!({"op":"scene.selection"}));
    assert_eq!(selection["count"], 0);
    let overlay = ok(
        &mut state,
        json!({"op":"scene.gizmo.state","scene":"street","frame":FRAME}),
    );
    assert_eq!(overlay["overlay"]["target"], "fx", "{overlay}");
    assert_eq!(
        overlay["overlay"]["reason"], "fx_target",
        "the node gizmo gives way"
    );

    // A live drag of a handle, then selecting the entity: the drag and its gesture are gone.
    let start = handle_at(&handles(&mut state), "radius");
    let pressed = down(&mut state, start);
    assert_eq!(pressed["dragging"], true);
    let swapped = ok(
        &mut state,
        json!({"op":"scene.select","scene":"street","entities":[FIGURE],"mode":"replace"}),
    );
    assert_eq!(swapped["fx_target_dropped"]["clip"], "v1", "{swapped}");
    assert_eq!(swapped["cancelled_drag"], pressed["gesture"]);
    let error = err(
        &mut state,
        json!({"op":"scene.pointer.move","point":{"u":0.5,"v":0.5}}),
    );
    assert_eq!(error.code, "no_drag", "no phantom drag moves anything");
    let released = up(&mut state, None);
    assert_eq!(released["noop"], true, "{released}");
    assert_eq!(
        undo_depth(&state),
        2,
        "attach + the clip; the drag wrote nothing"
    );
    let overlay = ok(
        &mut state,
        json!({"op":"scene.gizmo.state","scene":"street","frame":FRAME}),
    );
    assert!(
        overlay["overlay"].get("target").is_none(),
        "the node overlay is back: {overlay}"
    );
    assert!(overlay["overlay"].get("fx").is_none());

    // And the other way: a live drag of a node handle, then choosing the clip.
    ok(
        &mut state,
        json!({"op":"scene.select","scene":"street","entities":[FIGURE],"mode":"replace"}),
    );
    let layout = ok(
        &mut state,
        json!({"op":"scene.gizmo.state","scene":"street","frame":FRAME}),
    );
    let arrow = layout["overlay"]["handles"]
        .as_array()
        .and_then(|handles| handles.iter().find(|handle| handle["id"] == "x"))
        .map(|handle| handle["to"].clone())
        .unwrap_or_else(|| panic!("the figure has an x arrow: {layout}"));
    let (u, v) = (arrow["u"].as_f64().unwrap(), arrow["v"].as_f64().unwrap());
    let node = down(&mut state, (u, v));
    assert_eq!(node["dragging"], true, "a node drag is open: {node}");
    let chosen = ok(
        &mut state,
        json!({"op":"scene.fx.select","scene":"street","clip":"v1"}),
    );
    assert_eq!(chosen["cancelled_drag"], node["gesture"], "{chosen}");
    let error = err(
        &mut state,
        json!({"op":"scene.pointer.move","point":{"u":0.5,"v":0.5}}),
    );
    assert_eq!(error.code, "no_drag", "no phantom drag of the node either");
    // Clearing the target is an answer, not an error, and a second clear changes nothing.
    let cleared = ok(&mut state, json!({"op":"scene.fx.select","scene":"street"}));
    assert!(cleared["target"].is_null());
    let again = ok(&mut state, json!({"op":"scene.fx.select","scene":"street"}));
    assert_eq!(again["changed"], false);
}

#[test]
fn choosing_another_clip_cancels_the_drag_of_the_first() {
    let mut state = session();
    ok(
        &mut state,
        json!({"op":"scene.fx.edit","scene":"street","ops":[
            {"op":"add_clip","effect":"vignette","start_frame":10,"duration_frames":30,"id":"v2"}]}),
    );
    selected(&mut state);
    let start = handle_at(&handles(&mut state), "radius");
    let pressed = down(&mut state, start);
    mv(&mut state, (start.0 + 0.1, 0.5));
    let chosen = ok(
        &mut state,
        json!({"op":"scene.fx.select","scene":"street","clip":"v2"}),
    );
    assert_eq!(chosen["cancelled_drag"], pressed["gesture"]);
    let error = err(
        &mut state,
        json!({"op":"scene.pointer.move","point":{"u":0.5,"v":0.5}}),
    );
    assert_eq!(error.code, "no_drag");
    let status = ok(&mut state, json!({"op":"scene.fx.gesture.status"}));
    assert_eq!(
        status["gesture"],
        Value::Null,
        "the gesture went with the drag: {status}"
    );
    // Choosing the same clip again while dragging is not a swap.
    let mut state = session();
    selected(&mut state);
    let start = handle_at(&handles(&mut state), "radius");
    down(&mut state, start);
    let same = ok(
        &mut state,
        json!({"op":"scene.fx.select","scene":"street","clip":"v1"}),
    );
    assert_eq!(same["cancelled_drag"], Value::Null);
    assert_eq!(same["changed"], false);
    mv(&mut state, (start.0 + 0.05, 0.5));
}

#[test]
fn a_press_that_picks_something_else_leaves_the_clip() {
    let mut state = session();
    selected(&mut state);
    // A press on empty picture, away from every handle, deselects: the clip goes with it.
    let pressed = down(&mut state, (0.95, 0.05));
    assert_eq!(pressed["dragging"], false);
    assert_eq!(pressed["fx_target_dropped"]["clip"], "v1", "{pressed}");
    assert!(state.scene_interaction.fx_target().is_none());
}

#[test]
fn an_undo_or_an_edit_under_a_live_drag_kills_it_and_it_never_confirms_late() {
    let mut state = session();
    selected(&mut state);
    let start = handle_at(&handles(&mut state), "radius");
    down(&mut state, start);
    mv(&mut state, (start.0 + 0.1, 0.5));
    assert!(!state.gizmo_drag_is_dead());
    ok(&mut state, json!({"op":"history.undo"}));
    assert!(state.gizmo_drag_is_dead(), "the scene moved under the drag");
    let error = err(
        &mut state,
        json!({"op":"scene.pointer.move","point":{"u":0.6,"v":0.5}}),
    );
    assert_eq!(error.code, "gesture_invalidated");
    let depth = undo_depth(&state);
    // The release of a drag whose scene moved is refused by name and confirms nothing.
    let released = err(&mut state, json!({"op":"scene.pointer.up"}));
    assert_eq!(released.code, "gesture_invalidated");
    assert_eq!(undo_depth(&state), depth);
    let cancelled = ok(&mut state, json!({"op":"scene.pointer.cancel"}));
    assert!(cancelled["dragging"] == false);
}

/// The target is dropped where the project changes, so every read below comes with NO overlay or
/// selection read in between: a lazy prune would publish the removed clip as the live target.
#[test]
fn removing_the_clip_or_the_scene_drops_the_target_before_any_read() {
    let node = (json!("node"), json!(""));
    let mut state = session();
    selected(&mut state);
    assert_eq!(published_target(&state), (json!("fx"), json!("v1")));
    ok(
        &mut state,
        json!({"op":"scene.fx.edit","scene":"street","ops":[{"op":"remove_clip","clip":"v1"}]}),
    );
    assert_eq!(published_target(&state), node);
    assert!(state.scene_interaction.fx_target().is_none());
    assert_eq!(
        state.summary()["scene_interaction"]["fx_target"],
        Value::Null
    );

    let mut state = session();
    selected(&mut state);
    let detached = ok(&mut state, json!({"op":"scene.detach","scene":"street"}));
    assert_eq!(detached["fx_target_dropped"]["clip"], "v1", "{detached}");
    assert!(state.scene_interaction.fx_target().is_none());
    assert_eq!(
        state.summary()["scene_interaction"]["fx_target"],
        Value::Null
    );
}

#[test]
fn undoing_the_creation_of_the_clip_drops_the_target_and_redo_does_not_bring_it_back() {
    // The command (`history.undo`) and the library call (`undo()`): neither goes unpruned.
    for by_command in [true, false] {
        let mut state = session();
        selected(&mut state);
        if by_command {
            ok(&mut state, json!({"op":"history.undo"}));
        } else {
            state.undo().unwrap();
        }
        assert_eq!(published_target(&state), (json!("node"), json!("")));
        assert!(state.scene_interaction.fx_target().is_none());
        if by_command {
            ok(&mut state, json!({"op":"history.redo"}));
        } else {
            state.redo().unwrap();
        }
        assert_eq!(handles(&mut state)["clip"], "v1", "the clip is back");
        assert_eq!(published_target(&state), (json!("node"), json!("")));
    }
}

/// `scene.gizmo.hit` ("what is under this point") and `scene.pointer.down` at the same point read
/// the same handle, drawn or greyed, and the query writes nothing.
#[test]
fn the_hit_query_names_what_the_press_at_the_same_point_holds() {
    let hit = |state: &mut EditorState, at: (f64, f64)| {
        ok(
            state,
            json!({"op":"scene.gizmo.hit","scene":"street","frame":FRAME,
                   "point":{"u":at.0,"v":at.1}}),
        )
    };
    let mut state = session();
    selected(&mut state);
    let start = handle_at(&handles(&mut state), "radius");
    let (before, depth) = (document(&state), undo_depth(&state));
    let asked = hit(&mut state, start);
    assert_eq!(asked["hit"]["kind"], "fx_handle", "{asked}");
    assert_eq!(asked["hit"]["handle"], "radius");
    assert_eq!(asked["refused"], Value::Null);
    assert_eq!((document(&state), undo_depth(&state)), (before, depth));
    assert!(state.scene_interaction.fx_target().is_some());
    let pressed = down(&mut state, start);
    assert_eq!(
        pressed["hit"], asked["hit"],
        "the press holds what was named"
    );
    assert_eq!(pressed["dragging"], true);
    ok(&mut state, json!({"op":"scene.pointer.cancel"}));

    // Away from every handle the query says what a press would pick, and no effect handle.
    let away = hit(&mut state, (0.95, 0.05));
    assert_ne!(away["hit"]["kind"], "fx_handle", "{away}");
    assert_eq!(down(&mut state, (0.95, 0.05))["hit"], away["hit"]);

    // A handle with keys is named and refused by the query as by the press.
    let mut keyed = session();
    ok(
        &mut keyed,
        json!({"op":"scene.fx.edit","scene":"street","ops":[
            {"op":"key_param","clip":"v1","param":"radius","frame":0,"value":0.4},
            {"op":"key_param","clip":"v1","param":"radius","frame":20,"value":0.6}]}),
    );
    selected(&mut keyed);
    let rx = handles(&mut keyed)["ellipses"][0]["rx"].as_f64().unwrap();
    let asked = hit(&mut keyed, (0.2 + rx, 0.5));
    assert_eq!(asked["refused"], "animated_param", "{asked}");
    let pressed = down(&mut keyed, (0.2 + rx, 0.5));
    assert_eq!(pressed["hit"], asked["hit"]);
    assert_eq!(pressed["refused"], asked["refused"]);
}
